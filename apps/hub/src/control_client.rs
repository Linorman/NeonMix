//! Verified control replication runs independently of the media pump.
use crate::{Credential, Result};
use futures_util::{SinkExt, StreamExt};
use neonmix_control::{Event, Snapshot};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{Message, client::IntoClientRequest},
};

#[derive(Clone, Default)]
pub struct View {
    pub state: Option<Arc<Snapshot>>,
    pub connected: bool,
    pub snapshots: u64,
    pub subscriptions: u64,
    pub applied_events: u64,
    pub rejected: bool,
}
pub struct Subscription {
    pub view: watch::Receiver<View>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub fn subscribe(credential: Credential, hub: String) -> Result<Subscription> {
    let http = crate::sender::client(&credential)?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut std::io::Cursor::new(&credential.certificate)) {
        roots.add(cert?)?;
    }
    let tls = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = Connector::Rustls(Arc::new(tls));
    let (publish, view) = watch::channel(View::default());
    let task = tokio::spawn(async move {
        let mut current = View::default();
        let mut backoff = Duration::from_millis(250);
        loop {
            // Every connection attempt begins with a snapshot; a retained old
            // revision never bypasses resynchronization after network failure.
            let attempt:Result<()>=async {
                let response=http.get(format!("{hub}/v1/hub")).bearer_auth(&credential.token).send().await?;
                if matches!(response.status().as_u16(),401|403) {current.rejected=true;return Err("control authorization revoked".into());}
                let mut state:Snapshot=response.error_for_status()?.json().await?;
                current.snapshots+=1;current.state=Some(Arc::new(state.clone()));current.connected=false;publish.send_replace(current.clone());
                let mut url=reqwest::Url::parse(&hub)?;url.set_scheme("wss").map_err(|_|"invalid WSS scheme")?;url.set_path("/v1/events");url.set_query(Some(&format!("after={}",state.revision)));
                let mut request=url.as_str().into_client_request()?;
                request.headers_mut().insert("authorization",format!("Bearer {}",credential.token).parse()?);
                let config=tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(262144)).max_frame_size(Some(262144));
                let (mut socket,_)=tokio::time::timeout(Duration::from_secs(5),connect_async_tls_with_config(request,Some(config),true,Some(connector.clone()))).await??;
                current.connected=true;current.subscriptions+=1;publish.send_replace(current.clone());backoff=Duration::from_millis(250);
                let mut heartbeat=tokio::time::interval(Duration::from_secs(2));let mut observed=tokio::time::Instant::now();
                loop {tokio::select! {
                    _=heartbeat.tick()=>{
                        if observed.elapsed()>Duration::from_secs(6) {return Err("control heartbeat timed out".into());}
                        socket.send(Message::Ping(Vec::new().into())).await?;
                    }
                    message=socket.next()=>{
                        let message=message.ok_or("control subscription closed")??;observed=tokio::time::Instant::now();
                        match message {
                            Message::Text(text)=>{
                                let value:serde_json::Value=serde_json::from_str(&text)?;
                                if let Some(error)=value.get("error").and_then(|v|v.as_str()) {if matches!(error,"unauthenticated"|"permission_denied") {current.rejected=true;}return Err("control event requires resynchronization".into());}
                                state.apply_event(serde_json::from_value::<Event>(value)?)?;
                                current.applied_events+=1;current.state=Some(Arc::new(state.clone()));publish.send_replace(current.clone());
                            }
                            Message::Close(_)=>return Err("control subscription closed".into()),
                            Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await?,
                            Message::Pong(_)=>{},
                            _=>return Err("unexpected control frame".into()),
                        }
                    }
                }}
            }.await;
            current.connected = false;
            publish.send_replace(current.clone());
            if current.rejected || publish.is_closed() {
                return;
            }
            let _ = attempt;
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }
    });
    Ok(Subscription { view, task })
}
