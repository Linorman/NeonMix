use crate::{Credential, Result, emit, server::StartResponse};
use futures_util::FutureExt;
use neonmix_control::{Command, MediaOffer, Operation, Snapshot};
use neonmix_core::{
    MAX_BLOCK_FRAMES,
    signal::{SignalKind, StereoSource, TestSignal},
};
use neonmix_media::Sender;
use std::{
    collections::VecDeque,
    net::UdpSocket,
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_PENDING_MEDIA: usize = 8;

pub struct SendOptions {
    pub capture: Option<String>,
    pub virtual_output: bool,
    pub provider: crate::virtual_output::Provider,
    pub frequency: f64,
    pub output_binding: Option<std::path::PathBuf>,
}

pub fn client(credential: &Credential) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .use_preconfigured_tls(neonmix_identity::trust::tls(&credential.certificate)?)
        .timeout(Duration::from_secs(5));
    if let Some(route) = &credential.route {
        let url = reqwest::Url::parse(&route.url)?;
        builder = builder.resolve(
            url.host_str().ok_or("missing discovery host")?,
            route.address,
        );
    }
    Ok(builder.build()?)
}
pub async fn run(
    credential: Credential,
    hub: &str,
    seconds: u32,
    options: SendOptions,
) -> Result<()> {
    let SendOptions {
        capture,
        virtual_output,
        provider,
        frequency,
        output_binding,
    } = options;
    let store = output_binding.map(neonmix_output_binding::Store::new);
    let bound = store
        .as_ref()
        .map(neonmix_output_binding::Store::load)
        .transpose()?;
    if bound.as_ref().is_some_and(|binding| !binding.enabled) {
        return Err("local output is disabled; explicitly enable it before starting Sender".into());
    }
    let provider = bound.as_ref().map_or(provider, |binding| binding.provider);
    if !(1..=86400).contains(&seconds) || !hub.starts_with("https://") {
        return Err("seconds must be 1..86400 and Hub must use HTTPS".into());
    }
    let _activity = crate::qos::AudioActivity::begin();
    let client = client(&credential)?;
    // Use the endpoint that actually passed TLS, including its address family.
    let response = client
        .get(format!("{hub}/v1/hub"))
        .bearer_auth(&credential.token)
        .send()
        .await?
        .error_for_status()?;
    let remote = response
        .remote_addr()
        .ok_or("authenticated Hub address missing")?;
    let snapshot: Snapshot = response.json().await?;
    crate::identity::check_hub(&credential, snapshot.hub_id)?;
    if bound
        .as_ref()
        .is_some_and(|binding| binding.hub_id != snapshot.hub_id)
    {
        return Err(
            "authenticated Hub identity does not match the persisted output binding".into(),
        );
    }
    let binding_guard = match (&store, &bound) {
        (Some(store), Some(original)) => {
            Some(crate::binding::Guard::new(store.clone(), original.clone())?)
        }
        _ => None,
    };
    let socket = UdpSocket::bind(if remote.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })?;
    socket.set_nonblocking(true)?;
    let (pem, cert, _) = crate::certificate()?;
    let ssrc = (Uuid::new_v4().as_u128() as u32).max(1);
    // Device ownership is independent of this network session. On Linux a local
    // `neonmix-audio virtual-output` owner keeps the sink present across Sender stops.
    let capture = if let Some(binding) = &bound {
        let selected = crate::virtual_output::resolve_id(
            provider,
            &binding.device_id,
            crate::backend()?.devices()?,
        )?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if binding.provider == crate::virtual_output::Provider::Neonmix {
            crate::binding::sync_native_name(binding)?;
        }
        emit(
            serde_json::json!({"event":"virtual_output_selected","binding":selected,"output_binding":binding}),
        )?;
        Some(binding.device_id.clone())
    } else if virtual_output {
        {
            let mut found = None;
            for _ in 0..20 {
                found = crate::virtual_output::resolve(provider, crate::backend()?.devices()?).ok();
                if found.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let binding = found
                .ok_or("selected virtual output is unavailable or has no stereo capture side")?;
            let id = binding.device.id.clone();
            emit(serde_json::json!({"event":"virtual_output_selected","binding":binding}))?;
            Some(id)
        }
    } else {
        capture
    };
    let capture_id = capture.clone();
    #[cfg(target_os = "macos")]
    let capture_kind = neonmix_io::CaptureKind::VirtualDevice;
    #[cfg(not(target_os = "macos"))]
    let capture_kind = neonmix_io::CaptureKind::Loopback;
    let mut capture = capture
        .map(|id| {
            crate::backend()?.open_capture(
                &id,
                capture_kind,
                neonmix_io::OpenOptions {
                    sample_rate: None,
                    period_frames: if cfg!(target_os = "linux") {
                        None
                    } else {
                        Some(480)
                    },
                },
                1,
            )
        })
        .transpose()?;
    if let Some(capture) = &capture {
        capture.play()?;
    }
    if remote.ip().is_loopback()
        && capture_id
            .as_ref()
            .is_some_and(|id| *id == snapshot.output.id)
    {
        return Err("local capture and Hub output must use different devices".into());
    }
    let mut request = Command {
        request_id: Uuid::new_v4(),
        expected_revision: snapshot.revision,
        operation: Operation::Start {
            offer: MediaOffer {
                version: 1,
                codec: "opus".into(),
                rate: 48000,
                channels: 2,
                packet_frames: 480,
                payload_type: 96,
                ssrc,
                stream_epoch: 1,
                udp_port: socket.local_addr()?.port(),
                certificate_sha256: neonmix_media::certificate_fingerprint(&cert)?,
            },
        },
    };
    let mut attempts = 0;
    let response: StartResponse = loop {
        let response = client
            .post(format!("{hub}/v1/sessions"))
            .bearer_auth(&credential.token)
            .json(&request)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::CONFLICT && attempts < 3 {
            let error: serde_json::Value = response.json().await?;
            if error["error"] != "revision_conflict" {
                return Err("session already active or idempotency conflict".into());
            }
            let snapshot: Snapshot = client
                .get(format!("{hub}/v1/hub"))
                .bearer_auth(&credential.token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            crate::identity::check_hub(&credential, snapshot.hub_id)?;
            if bound
                .as_ref()
                .is_some_and(|binding| binding.hub_id != snapshot.hub_id)
            {
                return Err("Hub identity changed during session negotiation".into());
            }
            request.expected_revision = snapshot.revision;
            attempts += 1;
            continue;
        }
        break response.error_for_status()?.json().await?;
    };
    if let Some(id) = credential.hub_id
        && response.hub_id != Some(id)
    {
        if let Some(session) = &response.session {
            stop_session(&client, &credential, hub, session.id).await;
        }
        return Err("paired Hub identity changed during session negotiation".into());
    }
    let session = response.session.ok_or("missing accepted session")?;
    if bound
        .as_ref()
        .is_some_and(|binding| response.hub_id != Some(binding.hub_id))
    {
        stop_session(&client, &credential, hub, session.id).await;
        return Err("session negotiation did not confirm the bound Hub identity".into());
    }
    socket.connect({
        let mut address = remote;
        address.set_port(response.media_port.ok_or("missing media port")?);
        address
    })?;
    let mut sender = Sender::new(&session, &pem, &response.hub_certificate_sha256)?;
    let mut media_wait = crate::media_wait::MediaWait::new(socket)?;
    let wake = media_wait.waker();
    sender.notify_ready(std::sync::Arc::new(move || {
        let _ = wake.wake();
    }));
    let subscription = crate::control_client::subscribe(credential.clone(), hub.to_owned())?;
    let mut framer = capture
        .as_ref()
        .map(|c| crate::framer::CaptureFramer::new(c.info.format))
        .transpose()?;
    emit(
        serde_json::json!({"event":"sender_started","session_id":session.id,"stream_id":session.stream_id,"virtual_output_provider":(virtual_output||bound.is_some()).then_some(provider),"output_binding":bound,"capture":capture.as_ref().map(|c| &c.info),"capture_conversion_delay_frames":framer.as_ref().map(|f|f.delay_frames())}),
    )?;
    let mut media_log = crate::media_log::MediaLog::new()?;
    let start = Instant::now();
    let mut next_audio = start;
    let mut next_source_push = start;
    let mut next_packet = start;
    let mut next_feedback = start + Duration::from_secs(1);
    let mut next_status = start + Duration::from_secs(1);
    let mut signal = TestSignal::new(SignalKind::Sine, 48000, frequency, -36.0)?;
    let mut frames = [[0.0; 2]; MAX_BLOCK_FRAMES];
    let mut bytes = [0u8; 4097];
    let mut pending = VecDeque::with_capacity(MAX_PENDING_MEDIA);
    let mut max_pending = 0usize;
    let mut dropped = 0u64;
    let mut sent = 0u64;
    let mut sent_bytes = 0u64;
    let mut generated_audio_blocks = 0u64;
    let skipped_source_periods = 0u64;
    let mut max_source_backlog_ns = 0u64;
    let mut encoded_media_packets = 0u64;
    let mut max_loop_gap_ns = 0u64;
    let mut last_loop_gap_ns = 0u64;
    let mut last_source_push_ns = 0u64;
    let mut backlog_at_loop_start_ns = 0u64;
    let mut last_loop = Instant::now();
    let media_scheduling = neonmix_media::ThreadPriority::enter();
    let mut timing = crate::pump_timing::Timing::new();
    let shutdown = stop_signal();
    tokio::pin!(shutdown);
    // TLS/WSS replication runs on Tokio workers. The paced media pump stays
    // on this QoS-classified thread, woken by sockets/native samples and the
    // next bounded source/pacing deadline, independently of Tokio timers.
    let result: Result<()> = (|| {
        while start.elapsed() < Duration::from_secs(u64::from(seconds)) {
            let now = Instant::now();
            last_loop_gap_ns = now
                .duration_since(last_loop)
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64;
            max_loop_gap_ns = max_loop_gap_ns.max(last_loop_gap_ns);
            backlog_at_loop_start_ns = now
                .saturating_duration_since(next_audio)
                .as_nanos()
                .min(u128::from(u64::MAX)) as u64;
            last_source_push_ns = 0;
            last_loop = now;
            let mut receive_exhausted = true;
            for _ in 0..32 {
                match media_wait.recv(&mut bytes) {
                    Ok(n) => {
                        let _ = sender.ingest(&bytes[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        receive_exhausted = false;
                        break;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            sender.check()?;
            if let Some(guard) = &binding_guard
                && !guard.permits()
            {
                media_log.publish(
                    serde_json::json!({"event":"output_binding_revoked","reason":guard.reason()}),
                );
                return Err(
                    "local output permission changed; automatic restart is disabled".into(),
                );
            }
            timing.finish("receive");
            if let Some(capture) = &mut capture {
                let snapshot = capture.stats.snapshot();
                if snapshot.errors > 0 {
                    media_log.publish(
                        serde_json::json!({"event":"capture_fault","info":capture.info,"stats":snapshot,"recovery":"explicit_restart_with_fresh_session"}),
                    );
                    return Err(
                        "capture unavailable; restart explicitly after device recovery".into(),
                    );
                }
                for _ in 0..32 {
                    let Some(block) = capture.pop() else {
                        break;
                    };
                    if let Err(error) = framer
                        .as_mut()
                        .ok_or("capture framer missing")?
                        .push(&block, &mut sender)
                    {
                        media_log.publish(
                            serde_json::json!({"event":"capture_fault","info":capture.info,"stats":capture.stats.snapshot(),"reason":error.to_string(),"recovery":"explicit_restart_with_fresh_session"}),
                        );
                        return Err(error);
                    }
                }
            } else if Instant::now() >= next_audio.max(next_source_push) {
                for frame in &mut frames {
                    *frame = signal.next_frame();
                }
                let push_started = Instant::now();
                let pushed = sender.push_frames(&frames);
                last_source_push_ns =
                    push_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
                timing.finish("source");
                pushed?;
                generated_audio_blocks += 1;
                next_audio += Duration::from_millis(10);
                next_source_push = Instant::now() + Duration::from_millis(1);
                // Keep the synthetic source sample clock continuous after a short
                // scheduling stall. Emit at most one block per loop and cap the
                // backlog at 80 ms; never silently compress elapsed source time.
                let backlog = Instant::now().saturating_duration_since(next_audio);
                max_source_backlog_ns =
                    max_source_backlog_ns.max(backlog.as_nanos().min(u128::from(u64::MAX)) as u64);
                if backlog > Duration::from_millis(80) {
                    return Err(format!(
                        "synthetic source deadline exceeded ({} ms); fresh media context required",
                        backlog.as_millis()
                    )
                    .into());
                }
            }
            timing.finish("source");
            for packet in sender.outgoing() {
                if packet.first().is_some_and(|b| (20..=63).contains(b)) {
                    let _ = media_wait.send(&packet);
                } else {
                    encoded_media_packets += 1;
                    if pending.len() == MAX_PENDING_MEDIA {
                        pending.pop_front();
                        dropped += 1;
                    }
                    pending.push_back(packet);
                    max_pending = max_pending.max(pending.len());
                }
            }
            if Instant::now() >= next_packet {
                if let Some(packet) = pending.pop_front() {
                    match media_wait.send(&packet) {
                        Ok(n) => {
                            sent += 1;
                            sent_bytes += n as u64;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => dropped += 1,
                        Err(e) => return Err(e.into()),
                    }
                }
                // Opus/GStreamer can deliver several encoded packets together.
                // Drain that bounded burst over milliseconds instead of keeping
                // it a whole frame behind the source indefinitely.
                next_packet = Instant::now()
                    + if pending.is_empty() {
                        Duration::from_millis(10)
                    } else {
                        Duration::from_millis(1)
                    };
            }
            timing.finish("outgoing");
            if Instant::now() >= next_feedback {
                sender.poll_feedback();
                next_feedback += Duration::from_secs(1);
            }
            timing.finish("feedback");
            if Instant::now() >= next_status {
                let view = subscription.view.borrow().clone();
                if bound.as_ref().is_some_and(|binding| {
                    view.state
                        .as_ref()
                        .is_some_and(|state| state.hub_id != binding.hub_id)
                }) {
                    return Err("control snapshot no longer belongs to the bound Hub".into());
                }
                if view.rejected
                    || view.state.as_ref().is_some_and(|snapshot| {
                        snapshot
                            .sessions
                            .get(&session.id)
                            .is_none_or(|s| !s.status.active())
                    })
                {
                    return Err("session stopped by Hub; automatic reconnect is disabled".into());
                }
                media_log.publish(
                    serde_json::json!({"event":"sender_stats","receive_throttles":media_wait.receive_throttles,"pump_timing":timing.snapshot,"diagnostic_drops":media_log.dropped,"policy":sender.policy,"sent_packets":sent,"queue_drops":dropped,"max_pending":max_pending,"sent_bytes":sent_bytes,"generated_audio_blocks":generated_audio_blocks,"skipped_source_periods":skipped_source_periods,"max_source_backlog_ns":max_source_backlog_ns,"encoded_media_packets":encoded_media_packets,"audio_queue_dropped":sender.audio_queue_dropped(),"audio_queue_buffers":sender.audio_queue_buffers(),"max_loop_gap_ns":max_loop_gap_ns,"media_scheduling":media_scheduling.snapshot(),"native_scheduling":sender.native_scheduling(),"encoder_bitrate":sender.encoder_bitrate(),"encoder_dtx":sender.encoder_dtx(),"feedback_reports":sender.feedback_reports(),"last_feedback":sender.last_feedback(),"capture_stats":capture.as_ref().map(|c|c.stats.snapshot()),"control_connected":view.connected,"control_snapshots":view.snapshots,"control_subscriptions":view.subscriptions,"control_events":view.applied_events,"control_revision":view.state.as_ref().map(|s|s.revision)}),
                );
                next_status += Duration::from_secs(1);
            }
            if let Some(stop) = shutdown.as_mut().now_or_never() {
                stop?;
                media_log.publish(serde_json::json!({"event":"sender_stop_requested"}));
                break;
            }
            timing.finish("report");
            let now = Instant::now();
            let mut wait = next_status
                .min(next_feedback)
                .saturating_duration_since(now);
            wait = wait.min(if capture.is_some() {
                Duration::from_millis(1)
            } else {
                next_audio
                    .max(next_source_push)
                    .saturating_duration_since(now)
            });
            if !pending.is_empty() {
                wait = wait.min(next_packet.saturating_duration_since(now));
            }
            if receive_exhausted {
                // Retry unread edge-triggered readiness on a short deadline;
                // native sample notifications can wake the pump sooner.
                wait = Duration::from_millis(1);
            }
            media_wait.wait(wait)?;
            timing.finish_wait(wait);
        }
        Ok(())
    })();
    sender.gate().revoke();
    if let Err(error) = &result {
        // The periodic log may be one second old (or its bounded queue full).
        // Record the actual failure counters after stopping media authorization.
        eprintln!(
            "{}",
            serde_json::json!({
                "event":"sender_failure", "reason":error.to_string(),
                "receive_throttles":media_wait.receive_throttles,
                "media_scheduling":media_scheduling.snapshot(),
                "pump_timing":timing.snapshot,
                "elapsed_ns":start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
                "max_loop_gap_ns":max_loop_gap_ns, "max_source_backlog_ns":max_source_backlog_ns,
            "last_loop_gap_ns":last_loop_gap_ns, "last_source_push_ns":last_source_push_ns,
            "backlog_at_loop_start_ns":backlog_at_loop_start_ns,
            "current_loop_elapsed_ns":last_loop.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
                "generated_audio_blocks":generated_audio_blocks, "sent_packets":sent,
                "encoded_media_packets":encoded_media_packets, "queue_drops":dropped,
                "audio_queue_dropped":sender.audio_queue_dropped(),
                "native_scheduling":sender.native_scheduling()
            })
        );
    }
    drop(sender);
    if let Some(capture) = &capture {
        capture.control.stop();
        emit(
            serde_json::json!({"event":"capture_closed","info":capture.info,"stats":capture.stats.snapshot()}),
        )?;
    }
    drop(capture);
    // A user stop never starts a new session. A stale revision is refreshed once
    // for this explicit stop; mix edits never retry a conflicting value blindly.
    if let Ok(response) = client
        .get(format!("{hub}/v1/hub"))
        .bearer_auth(&credential.token)
        .send()
        .await
        && let Ok(snapshot) = response.json::<Snapshot>().await
    {
        let stop = Command {
            request_id: Uuid::new_v4(),
            expected_revision: snapshot.revision,
            operation: Operation::Stop {
                session_id: session.id,
            },
        };
        let _ = client
            .post(format!("{hub}/v1/commands"))
            .bearer_auth(&credential.token)
            .json(&stop)
            .send()
            .await;
    }
    emit(serde_json::json!({"event":"sender_stopped","sent_packets":sent,"queue_drops":dropped}))?;
    result
}

async fn stop_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

async fn stop_session(
    client: &reqwest::Client,
    credential: &Credential,
    hub: &str,
    session_id: Uuid,
) {
    if let Ok(response) = client
        .get(format!("{hub}/v1/hub"))
        .bearer_auth(&credential.token)
        .send()
        .await
        && let Ok(snapshot) = response.json::<Snapshot>().await
    {
        let command = Command {
            request_id: Uuid::new_v4(),
            expected_revision: snapshot.revision,
            operation: Operation::Stop { session_id },
        };
        let _ = client
            .post(format!("{hub}/v1/commands"))
            .bearer_auth(&credential.token)
            .json(&command)
            .send()
            .await;
    }
}
