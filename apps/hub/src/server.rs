use crate::connection::{AddressAcceptor, Connection};
use crate::{Result, ServerConfig, emit};
use axum::{
    Json, Router,
    extract::{Query, State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use neonmix_control::{
    Authority, Command, ControlError, Principal, Session, SessionStatus, Snapshot,
};
use neonmix_core::{
    mixer::{LANES, LaneMix, METER_WINDOW_FRAMES, Mixer, MixerConfig, MixerControl, MixerStats},
    queue::BlockProducer,
};
use neonmix_media::{ReceiveStats, Receiver, certificate_fingerprint};
use serde::{Deserialize, Serialize};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{
            AtomicBool,
            Ordering::{Acquire, Relaxed, Release},
        },
    },
    time::{Duration, Instant},
};
use uuid::Uuid;
struct Lane {
    producer: Option<BlockProducer>,
    media: Option<crate::media_worker::MediaWorker>,
    session: Option<Uuid>,
}
struct Resources {
    lanes: Vec<Lane>,
    control: MixerControl,
    stats: Arc<MixerStats>,
    origin: Instant,
    pem: String,
    output_epoch: u64,
}
impl Resources {
    fn prepare(
        &mut self,
        state: &Snapshot,
        remote: Option<Connection>,
    ) -> std::result::Result<(), ControlError> {
        self.prepare_with_commit(state, remote, || Ok(()))
    }
    fn prepare_with_commit(
        &mut self,
        state: &Snapshot,
        remote: Option<Connection>,
        before_commit: impl FnOnce() -> std::result::Result<(), ControlError>,
    ) -> std::result::Result<(), ControlError> {
        self.prepare_with_factory(
            state,
            remote,
            before_commit,
            crate::media_worker::MediaWorker::prepare,
        )
    }
    fn prepare_with_factory(
        &mut self,
        state: &Snapshot,
        remote: Option<Connection>,
        before_commit: impl FnOnce() -> std::result::Result<(), ControlError>,
        prepare_media: impl FnOnce(
            Receiver,
            UdpSocket,
            usize,
            Arc<MixerStats>,
        ) -> std::io::Result<crate::media_worker::PreparedWorker>,
    ) -> std::result::Result<(), ControlError> {
        if !self.control.has_capacity() {
            return Err(ControlError::Busy);
        }
        let mut added = None;
        for s in state.sessions.values().filter(|s| s.status.active()) {
            if self.lanes.iter().any(|l| l.session == Some(s.id)) {
                continue;
            }
            let lane = self
                .lanes
                .iter()
                .position(|l| l.session.is_none() && l.producer.is_some())
                .ok_or(ControlError::QuotaExceeded)?;
            let socket = remote
                .ok_or(ControlError::InvalidArgument)?
                .media_socket(s.offer.udp_port)
                .map_err(|_| ControlError::Busy)?;
            let receiver =
                Receiver::new(s.clone(), &self.pem, self.origin).map_err(|_| ControlError::Busy)?;
            added = Some((lane, s.id, socket, receiver));
        }
        let mut config = MixerConfig {
            master_db: state.output.gain_db,
            muted: state.output.muted || !state.output.available,
            output_epoch: self.output_epoch,
            ..MixerConfig::default()
        };
        for (i, lane) in self.lanes.iter().enumerate() {
            let session = added
                .as_ref()
                .filter(|a| a.0 == i)
                .map(|a| a.1)
                .or(lane.session);
            if let Some(s) = session
                .and_then(|id| state.sessions.get(&id))
                .filter(|s| s.status.active())
            {
                let mix = state
                    .streams
                    .get(&s.stream_id)
                    .ok_or(ControlError::NotFound)?
                    .mix;
                config.lanes[i] = LaneMix {
                    stream_id: s.stream_id,
                    epoch: s.offer.stream_epoch,
                    gain_db: mix.gain_db,
                    muted: mix.muted,
                    solo: mix.solo,
                };
            }
        }
        // Every fallible native/thread preparation precedes disk and audio
        // commit. No producer is moved until the transaction has succeeded.
        let staged = match added {
            Some((lane, id, socket, receiver)) => Some((
                lane,
                id,
                prepare_media(receiver, socket, lane, self.stats.clone())
                    .map_err(|_| ControlError::Busy)?,
            )),
            None => None,
        };
        before_commit()?;
        self.control.apply(config).map_err(|_| ControlError::Busy)?;
        for lane in &mut self.lanes {
            if lane
                .session
                .is_some_and(|id| state.sessions.get(&id).is_none_or(|s| !s.status.active()))
            {
                if let Some(media) = lane.media.take() {
                    lane.producer = media.close();
                }
                lane.session = None;
            }
        }
        if let Some((i, id, prepared)) = staged {
            let lane = &mut self.lanes[i];
            let producer = lane.producer.take().ok_or(ControlError::Busy)?;
            lane.media = Some(prepared.activate(producer));
            lane.session = Some(id);
        }
        Ok(())
    }
}
pub(super) struct Engine {
    pub(super) pairings: neonmix_identity::pairing::Book<Principal>,
    pub(super) pair_window: Instant,
    pub(super) pair_attempts: u32,
    pub(super) room_name: String,
    pub(super) certificate: String,
    event_slots: Arc<tokio::sync::Semaphore>,
    output_stats: Arc<neonmix_core::stats::AudioStats>,
    pub(super) authority: Authority,
    resources: Resources,
    errors: Vec<String>,
    pub(super) state_path: Option<std::path::PathBuf>,
}
fn persist(engine: &Engine) -> std::result::Result<(), ControlError> {
    persist_saved(engine.state_path.as_ref(), &engine.authority.persistent())
}
pub(super) fn persist_saved(
    path: Option<&std::path::PathBuf>,
    saved: &neonmix_control::PersistentState,
) -> std::result::Result<(), ControlError> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = serde_json::to_vec_pretty(saved).map_err(|_| ControlError::Busy)?;
    neonmix_identity::files::replace(path, &bytes).map_err(|_| ControlError::Busy)
}
pub(super) type Shared = Arc<Mutex<Engine>>;
#[derive(Serialize, Deserialize, Debug)]
pub struct StartResponse {
    #[serde(default)]
    pub hub_id: Option<Uuid>,
    pub receipt: neonmix_control::Receipt,
    pub session: Option<Session>,
    pub media_port: Option<u16>,
    pub hub_certificate_sha256: String,
}
pub(super) struct ApiError(pub(super) ControlError);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ControlError::Unauthenticated => StatusCode::UNAUTHORIZED,
            ControlError::PermissionDenied | ControlError::PlaybackBlocked => StatusCode::FORBIDDEN,
            ControlError::RevisionConflict
            | ControlError::IdempotencyConflict
            | ControlError::AlreadyActive => StatusCode::CONFLICT,
            ControlError::NotFound => StatusCode::NOT_FOUND,
            ControlError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
            ControlError::Busy => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        };
        (
            status,
            Json(serde_json::json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
impl From<ControlError> for ApiError {
    fn from(value: ControlError) -> Self {
        Self(value)
    }
}
pub(super) fn authenticate(
    engine: &Engine,
    headers: &HeaderMap,
) -> std::result::Result<Principal, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(ControlError::Unauthenticated)?;
    Ok(engine.authority.authenticate(token)?)
}
async fn me(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    let principal = authenticate(&engine, &headers)?;
    Ok(Json(
        serde_json::json!({"hub_id":engine.authority.current().hub_id,"room_name":engine.room_name,"device_id":principal.device_id(),"role":engine.authority.current().devices.get(&principal.device_id()).map(|device|device.role)}),
    ))
}
async fn snapshot(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<Snapshot>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    authenticate(&engine, &headers)?;
    Ok(Json(engine.authority.snapshot()))
}
async fn devices(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    authenticate(&engine, &headers)?;
    Ok(Json(serde_json::json!(engine.authority.snapshot().devices)))
}
async fn streams(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    authenticate(&engine, &headers)?;
    Ok(Json(serde_json::json!(engine.authority.snapshot().streams)))
}
async fn outputs(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    authenticate(&engine, &headers)?;
    Ok(Json(serde_json::json!([engine
        .authority
        .snapshot()
        .output])))
}
async fn command(
    State(shared): State<Shared>,
    axum::Extension(remote): axum::Extension<Connection>,
    headers: HeaderMap,
    Json(command): Json<Command>,
) -> std::result::Result<Json<StartResponse>, ApiError> {
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    let principal = authenticate(&engine, &headers)?;
    let path = engine.state_path.clone();
    let Engine {
        authority,
        resources,
        ..
    } = &mut *engine;
    let receipt = authority.execute_durable(principal, command, |state, saved| {
        resources.prepare_with_commit(state, Some(remote), || persist_saved(path.as_ref(), saved))
    })?;
    let state = authority.snapshot();
    let session = receipt
        .session_id
        .and_then(|id| state.sessions.get(&id))
        .cloned();
    let port = receipt
        .session_id
        .and_then(|id| resources.lanes.iter().find(|l| l.session == Some(id)))
        .and_then(|l| l.media.as_ref())
        .map(|media| media.port);
    let response = StartResponse {
        hub_id: Some(state.hub_id),
        receipt,
        session,
        media_port: port,
        hub_certificate_sha256: certificate_fingerprint(&resources.pem)
            .map_err(|_| ControlError::Busy)?,
    };
    Ok(Json(response))
}
#[derive(Deserialize)]
struct Cursor {
    after: u64,
}
async fn events(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
    ws: WebSocketUpgrade,
) -> std::result::Result<Response, ApiError> {
    let permit = {
        let engine = shared.lock().map_err(|_| ControlError::Busy)?;
        authenticate(&engine, &headers)?;
        engine.authority.events_after(cursor.after)?;
        engine
            .event_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ControlError::QuotaExceeded)?
    };
    Ok(ws.max_message_size(16384).max_frame_size(16384).max_write_buffer_size(262144).on_upgrade(move |mut socket| async move {
        let _permit = permit;
        let mut revision = cursor.after;
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let batch = match shared.lock() {
                        Ok(engine) => authenticate(&engine, &headers).and_then(|_| engine.authority.events_after(revision).map_err(ApiError)),
                        Err(_) => Err(ApiError(ControlError::Busy)),
                    };
                    match batch {
                        Ok(events) => for event in events {
                            // A backlog is a snapshot of past events, not an
                            // authorization lease for its entire replay time.
                            let authorized = match shared.lock() {
                                Ok(engine) => authenticate(&engine, &headers).is_ok(),
                                Err(_) => false,
                            };
                            if !authorized {
                                close_subscription(&mut socket, ControlError::Unauthenticated).await;
                                return;
                            }
                            let Ok(text) = serde_json::to_string(&event) else { return; };
                            if !send_control_message(&mut socket, Message::Text(text.into())).await { return; }
                            revision = event.revision;
                        },
                        Err(error) => {
                            close_subscription(&mut socket, error.0).await;
                            return;
                        }
                    }
                }
                message = socket.recv() => if !matches!(message, Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_)))) { return; }
            }
        }
    }))
}

async fn send_control_message<S: futures_util::Sink<Message> + Unpin>(
    socket: &mut S,
    message: Message,
) -> bool {
    use futures_util::SinkExt;
    matches!(
        tokio::time::timeout(Duration::from_secs(2), socket.send(message)).await,
        Ok(Ok(()))
    )
}

async fn close_subscription<S: futures_util::Sink<Message> + Unpin>(
    socket: &mut S,
    error: ControlError,
) {
    let message = Message::Text(
        serde_json::json!({"error":error.to_string()})
            .to_string()
            .into(),
    );
    if send_control_message(socket, message).await {
        let _ = send_control_message(socket, Message::Close(None)).await;
    }
}

async fn diagnostics(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    authenticate(&engine, &headers)?;
    let receivers: Vec<ReceiveStats> = engine
        .resources
        .lanes
        .iter()
        .filter_map(|l| l.media.as_ref().and_then(|m| m.latest()).map(|r| r.receive))
        .collect();
    let media_workers: Vec<_> = engine.resources.lanes.iter().filter_map(|l| {
        let report=l.media.as_ref()?.latest()?;
        Some(serde_json::json!({"scheduling":report.scheduling,"pump_timing":report.timing,"receive_throttles":report.receive_throttles,"max_loop_gap_ns":report.max_loop_gap_ns,"max_pump_ns":report.max_pump_ns,"max_report_ns":report.max_report_ns}))
    }).collect();
    let lane_stream_ids: Vec<_> = engine
        .resources
        .lanes
        .iter()
        .map(|l| {
            l.session
                .and_then(|id| engine.authority.current().sessions.get(&id))
                .map(|s| s.stream_id)
        })
        .collect();
    let budget_drops: Vec<_> = engine
        .resources
        .lanes
        .iter()
        .map(|l| {
            l.media
                .as_ref()
                .and_then(|m| m.latest())
                .map_or(0, |r| r.budget_drops)
        })
        .collect();
    let stats = &engine.resources.stats;
    let output_available = engine.authority.current().output.available;
    let lane_meters: Vec<_> = lane_stream_ids
        .iter()
        .zip(&stats.lane_meters)
        .map(|(expected, meter)| {
            let measured = meter.snapshot();
            let stream_id = expected.unwrap_or(0);
            // Control state can advance before the output callback applies it.
            // Never attach the previous occupant's samples to a new stream.
            let valid = output_available && stream_id != 0 && measured.stream_id == stream_id;
            serde_json::json!({
                "stream_id": stream_id,
                "peak": if valid { measured.peak } else { 0.0 },
                "rms": if valid { measured.rms } else { 0.0 },
            })
        })
        .collect();
    let output_meter = stats.output_meter.snapshot();
    let meters = serde_json::json!({
        "window_frames": METER_WINDOW_FRAMES,
        "sample_rate": 48000,
        "lanes": lane_meters,
        "output": {
            "peak": if output_available { output_meter.peak } else { 0.0 },
            "rms": if output_available { output_meter.rms } else { 0.0 },
        },
        "limiter_gain": if output_available {
            f32::from_bits(stats.limiter_gain_bits.load(Relaxed) as u32)
        } else { 1.0 },
    });
    Ok(Json(
        serde_json::json!({"meters":meters,"receivers":receivers,"media_workers":media_workers,"lane_stream_ids":lane_stream_ids,"packet_budget_drops":budget_drops,"pcm_queue_age_max_ns":stats.pcm_queue_age_max_ns.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"fifo_frames":stats.fifo_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"sinc_delay_frames":stats.sinc_delay_frames.load(Relaxed),"output_stats":engine.output_stats.snapshot(),"output_frames":stats.output_frames.load(Relaxed),"limited_frames":stats.limited_frames.load(Relaxed),"underrun_frames":stats.underrun_frames.load(Relaxed),"last_underrun_output_frame":stats.last_underrun_output_frame.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_fifo_frames":stats.last_underrun_fifo_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_needed_frames":stats.last_underrun_needed_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_source_end":stats.last_underrun_source_end.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"underrun_frames_by_lane":stats.underrun_frames_by_lane.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"filtered_queues":stats.filtered_queue_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"queues":stats.queue_frames.iter().map(|a| a.load(Relaxed)).collect::<Vec<_>>(),"drift_ppm":stats.drift_ppm_milli.iter().map(|a| a.load(Relaxed) as i64 as f64 / 1000.0).collect::<Vec<_>>(),"errors":engine.errors}),
    ))
}
fn worker(shared: Shared, stopped: Arc<AtomicBool>) {
    while !stopped.load(Acquire) {
        if let Ok(mut engine) = shared.lock() {
            let available = engine.authority.current().output.available;
            let transitions: Vec<_> = engine
                .resources
                .lanes
                .iter()
                .filter_map(|lane| Some((lane.session?, lane.media.as_ref()?.latest()?)))
                .collect();
            for (id, report) in transitions {
                let status = if available || !report.status.active() {
                    report.status
                } else {
                    SessionStatus::OutputLost
                };
                if let Some(error) = report.error
                    && engine
                        .authority
                        .current()
                        .sessions
                        .get(&id)
                        .is_some_and(|s| s.status.active())
                {
                    if engine.errors.len() >= 32 {
                        engine.errors.remove(0);
                    }
                    engine.errors.push(error);
                }
                let _ = engine.authority.set_session_status(id, status);
            }
            if engine.resources.lanes.iter().any(|l| {
                l.session.is_some_and(|id| {
                    engine
                        .authority
                        .current()
                        .sessions
                        .get(&id)
                        .is_none_or(|s| !s.status.active())
                })
            }) {
                let state = engine.authority.snapshot();
                let _ = engine.resources.prepare(&state, None);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub async fn serve(config: ServerConfig, listen: SocketAddr) -> Result<()> {
    // One owner per persistent Hub. The lock file stays at a stable inode while
    // the data file is atomically replaced on successful control commits.
    let _state_lock = if let Some(path) = &config.state_path {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path.with_extension("lock"))?;
        file.try_lock()
            .map_err(|_| "Hub state is already owned by another process")?;
        Some(file)
    } else {
        None
    };
    neonmix_media::runtime_probe()?;
    let admin = config
        .devices
        .first()
        .filter(|d| d.role == neonmix_control::Role::Admin)
        .ok_or("first provisioned device must be admin")?;
    let mut authority = if let Some(path) = config.state_path.as_ref().filter(|p| p.exists()) {
        let saved: neonmix_control::PersistentState = crate::read(path)?;
        if saved.output.id != config.output {
            return Err("persisted output differs from explicitly selected device".into());
        }
        Authority::restore(saved)?
    } else {
        let mut authority =
            Authority::new(config.output.clone(), admin.name.clone(), &admin.token)?;
        for device in &config.devices[1..] {
            authority.add_device(device.name.clone(), device.role, &device.token)?;
        }
        authority
    };
    let origin = Instant::now();
    let (mixer, control, inputs, stats) = Mixer::new(origin)?;
    let output_id = config.output.clone();
    let (returned_to, returned) = std::sync::mpsc::sync_channel(1);
    let source = crate::recoverable::RecoverableMixer {
        mixer: Some(mixer),
        returned: returned_to.clone(),
    };
    let running = crate::backend().and_then(|backend| {
        backend.open_output(
            &config.output,
            neonmix_io::OpenOptions {
                sample_rate: None,
                period_frames: if cfg!(target_os = "linux") {
                    None
                } else {
                    Some(256)
                },
            },
            source,
        )
    });
    let mut output = running.ok().filter(|running| running.play().is_ok());
    authority.set_output_available(output.is_some())?;
    let lanes = inputs
        .into_iter()
        .map(|producer| Lane {
            producer: Some(producer),
            media: None,
            session: None,
        })
        .collect();
    let room_name = config
        .room_name
        .clone()
        .unwrap_or_else(|| "NeonMix Lab".into());
    let hub_id = authority.current().hub_id;
    let discovery_fingerprint = certificate_fingerprint(&config.certificate)?;
    let discovery_listen = listen;
    let shared = Arc::new(Mutex::new(Engine {
        pairings: Default::default(),
        pair_window: Instant::now(),
        pair_attempts: 0,
        room_name: room_name.clone(),
        certificate: config.certificate.clone(),
        event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
        output_stats: output.as_ref().map(|o| o.stats.clone()).unwrap_or_default(),
        authority,
        resources: Resources {
            lanes,
            control,
            stats,
            origin,
            pem: config.pem,
            output_epoch: 0,
        },
        errors: Vec::new(),
        state_path: config.state_path,
    }));
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_shared = shared.clone();
    let worker_stop = stopped.clone();
    let app = Router::new()
        .route("/v1/identity", get(crate::pairing_api::identity))
        .route("/v1/pairing/invitations", post(crate::pairing_api::open))
        .route("/v1/pairing/cancel", post(crate::pairing_api::cancel))
        .route("/v1/pairing/complete", post(crate::pairing_api::complete))
        .route("/v1/me", get(me))
        .route("/v1/hub", get(snapshot))
        .route("/v1/devices", get(devices))
        .route("/v1/streams", get(streams))
        .route("/v1/outputs", get(outputs))
        .route("/v1/commands", post(command))
        .route("/v1/sessions", post(command))
        .route("/v1/events", get(events))
        .route("/v1/diagnostics", get(diagnostics))
        .layer(axum::extract::DefaultBodyLimit::max(16384))
        .with_state(shared.clone());
    persist(&*shared.lock().map_err(|_| "state lock failed")?)?;
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(config.certificate))
        .collect::<std::io::Result<Vec<_>>>()?;
    let key = rustls_pemfile::private_key(&mut std::io::Cursor::new(config.private_key))?
        .ok_or("TLS private key missing")?;
    let mut tls_config =
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let tls = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_config));
    let listener = std::net::TcpListener::bind(listen)?;
    listener.set_nonblocking(true)?;
    let actual_listen = listener.local_addr()?;
    let mut advertised = discovery_listen;
    advertised.set_port(actual_listen.port());
    let _publisher = neonmix_identity::discovery::Publisher::new(
        hub_id,
        &room_name,
        advertised,
        &discovery_fingerprint,
    )?;
    let handle = axum_server::Handle::new();
    let shutdown = handle.clone();
    emit(
        serde_json::json!({"event":"hub_started","listen":actual_listen,"hub_id":hub_id,"room_name":room_name,"output":output.as_ref().map(|o|&o.info),"lanes":LANES}),
    )?;
    let worker = std::thread::spawn(move || worker(worker_shared, worker_stop));
    let acceptor = axum_server::tls_rustls::RustlsAcceptor::new(tls).acceptor(AddressAcceptor);
    let server = axum_server::from_tcp(listener)
        .acceptor(acceptor)
        .handle(handle)
        .serve(app.into_make_service());
    tokio::pin!(server);
    let mut monitor = tokio::time::interval(Duration::from_millis(100));
    let stop_signal = async {
        #[cfg(unix)]
        {
            let mut terminated =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                result = tokio::signal::ctrl_c() => result,
                _ = terminated.recv() => Ok(()),
            }
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await
    };
    tokio::pin!(stop_signal);
    let mut stop_requested = false;
    let mut retry_output = Instant::now();
    let result = loop {
        tokio::select! {
            result = &mut server => break result.map_err(Into::into),
            _ = &mut stop_signal, if !stop_requested => { stop_requested = true; shutdown.graceful_shutdown(Some(Duration::from_secs(1))); },
            _ = monitor.tick() => {
                if let Some(position)=output.as_ref().and_then(|o|o.latest_position())&& let Ok(mut engine)=shared.lock()&& engine.resources.output_epoch!=position.epoch {engine.resources.output_epoch=position.epoch;let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}
                if output.as_ref().is_some_and(|o|o.stats.snapshot().errors>0) {
                    if let Some(o)=output.take() {o.control.stop();drop(o);}
                    if let Ok(mut engine)=shared.lock() {let _=engine.authority.set_output_available(false);let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}
                    emit(serde_json::json!({"event":"output_lost","device":output_id}))?;
                    retry_output=Instant::now()+Duration::from_secs(1);
                }
                if output.is_none()&& let Ok(mut mixer)=returned.try_recv() {
                    mixer.discard_backlog();
                    if Instant::now()>=retry_output {
                        retry_output=Instant::now()+Duration::from_secs(1);
                        let source=crate::recoverable::RecoverableMixer {mixer:Some(mixer),returned:returned_to.clone()};
                        if let Ok(running)=crate::backend().and_then(|b|b.open_output(&output_id,neonmix_io::OpenOptions {sample_rate:None,period_frames:if cfg!(target_os = "linux") {None} else {Some(256)}},source))
                            && running.play().is_ok() {if let Ok(mut engine)=shared.lock() {engine.output_stats=running.stats.clone();let _=engine.authority.set_output_available(true);let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}output=Some(running);emit(serde_json::json!({"event":"output_reopened","device":output_id}))?;}
                    } else {let _=returned_to.try_send(mixer);}
                }
            }
        }
    };
    stopped.store(true, Release);
    let _ = worker.join();
    drop(output);
    result
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use neonmix_control::{MediaOffer, Mix, Output, Stream};
    use std::collections::BTreeMap;

    fn fixture() -> (Resources, Snapshot, Mixer) {
        let (pem, _, _) = crate::certificate().unwrap();
        let (_, source_cert, _) = crate::certificate().unwrap();
        let origin = Instant::now();
        let (mixer, control, producers, stats) = Mixer::new(origin).unwrap();
        let id = Uuid::new_v4();
        let device_id = Uuid::new_v4();
        let session = Session {
            created_revision: 1,
            media_ttl_seconds: neonmix_control::MEDIA_TTL_SECONDS,
            id,
            device_id,
            stream_id: 1,
            media_context: Uuid::new_v4(),
            status: SessionStatus::Buffering,
            offer: MediaOffer {
                version: 1,
                codec: "opus".into(),
                rate: 48000,
                channels: 2,
                packet_frames: 480,
                payload_type: 96,
                ssrc: 1,
                stream_epoch: 1,
                udp_port: 54321,
                certificate_sha256: certificate_fingerprint(&source_cert).unwrap(),
            },
        };
        let snapshot = Snapshot {
            hub_id: Uuid::new_v4(),
            revision: 1,
            bus_id: "main".into(),
            devices: BTreeMap::new(),
            sessions: [(id, session)].into(),
            streams: [(
                1,
                Stream {
                    id: 1,
                    device_id,
                    session_id: id,
                    mix: Mix::default(),
                },
            )]
            .into(),
            output: Output {
                id: "test-output".into(),
                available: true,
                gain_db: -12.0,
                muted: false,
            },
        };
        let resources = Resources {
            lanes: producers
                .into_iter()
                .map(|producer| Lane {
                    producer: Some(producer),
                    media: None,
                    session: None,
                })
                .collect(),
            control,
            stats,
            origin,
            pem,
            output_epoch: 1,
        };
        (resources, snapshot, mixer)
    }
    fn untouched(resources: &mut Resources) {
        assert!(
            resources
                .lanes
                .iter()
                .all(|l| l.producer.is_some() && l.media.is_none() && l.session.is_none())
        );
        // All eight command slots remain free: no audible config was committed.
        for _ in 0..8 {
            resources.control.apply(MixerConfig::default()).unwrap();
        }
        assert!(!resources.control.has_capacity());
    }
    #[test]
    fn native_thread_preparation_failure_precedes_disk_and_audio_commit() {
        let (mut resources, snapshot, _mixer) = fixture();
        let mut persisted = false;
        let result = resources.prepare_with_factory(
            &snapshot,
            Some(Connection {
                remote: "127.0.0.1:54321".parse().unwrap(),
                local: "127.0.0.1:0".parse().unwrap(),
            }),
            || {
                persisted = true;
                Ok(())
            },
            |_, _, _, _| Err(std::io::Error::other("injected thread creation failure")),
        );
        assert_eq!(result, Err(ControlError::Busy));
        assert!(!persisted);
        untouched(&mut resources);
    }
    #[test]
    fn disk_failure_closes_prepared_thread_and_retains_producer() {
        let (mut resources, snapshot, _mixer) = fixture();
        let began = Instant::now();
        assert_eq!(
            resources.prepare_with_commit(
                &snapshot,
                Some(Connection {
                    remote: "127.0.0.1:54321".parse().unwrap(),
                    local: "127.0.0.1:0".parse().unwrap()
                }),
                || Err(ControlError::Busy)
            ),
            Err(ControlError::Busy)
        );
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "prepared rollback blocked"
        );
        untouched(&mut resources);
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    struct Blocked;
    impl futures_util::Sink<Message> for Blocked {
        type Error = std::convert::Infallible;
        fn poll_ready(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Pending
        }
        fn start_send(self: Pin<&mut Self>, _: Message) -> std::result::Result<(), Self::Error> {
            unreachable!("blocked writer never becomes ready")
        }
        fn poll_flush(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Pending
        }
        fn poll_close(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Pending
        }
    }
    #[tokio::test]
    async fn revoked_slow_subscription_releases_its_slot_after_error_send_deadline() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = slots.clone().try_acquire_owned().unwrap();
        let started = Instant::now();
        let closing = async {
            let _permit = permit;
            close_subscription(&mut Blocked, ControlError::Unauthenticated).await;
        };
        tokio::time::timeout(Duration::from_secs(3), closing)
            .await
            .expect("revoked slow subscriber kept its slot indefinitely");
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert!(slots.try_acquire().is_ok());
    }
}

#[cfg(test)]
mod pairing_tests {
    use super::*;
    use neonmix_identity::pairing::{Checked, Request};
    fn shared() -> (Shared, Principal, String, Mixer) {
        let token = neonmix_identity::secret();
        let mut authority = Authority::new("test-output".into(), "admin".into(), &token).unwrap();
        let member = neonmix_identity::secret();
        authority
            .add_device("member".into(), neonmix_control::Role::Member, &member)
            .unwrap();
        let issuer = authority.authenticate(&token).unwrap();
        let (mixer, control, inputs, stats) = Mixer::new(Instant::now()).unwrap();
        let (pem, certificate, _) = crate::certificate().unwrap();
        (
            Arc::new(Mutex::new(Engine {
                pairings: Default::default(),
                pair_window: Instant::now(),
                pair_attempts: 0,
                room_name: "test".into(),
                certificate,
                event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
                output_stats: Default::default(),
                authority,
                resources: Resources {
                    lanes: inputs
                        .into_iter()
                        .map(|producer| Lane {
                            producer: Some(producer),
                            media: None,
                            session: None,
                        })
                        .collect(),
                    control,
                    stats,
                    origin: Instant::now(),
                    pem,
                    output_epoch: 1,
                },
                errors: Vec::new(),
                state_path: None,
            })),
            issuer,
            member,
            mixer,
        )
    }
    fn headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        headers
    }
    async fn completion(
        shared: Shared,
        secret: &str,
        request: Request,
    ) -> (StatusCode, serde_json::Value) {
        let response = crate::pairing_api::complete(State(shared), headers(secret), Json(request))
            .await
            .into_response();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    #[tokio::test]
    async fn durable_registration_retry_revocation_and_member_permissions() {
        let (shared, issuer, member, _mixer) = shared();
        let secret = neonmix_identity::secret();
        let (id, grant) = shared
            .lock()
            .unwrap()
            .pairings
            .open(issuer, 30, Instant::now())
            .unwrap();
        let request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "paired".into(),
            token_sha256: neonmix_identity::digest(&secret),
        };
        assert_eq!(
            completion(shared.clone(), "wrong", request.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (status, done) = completion(shared.clone(), &grant, request.clone()).await;
        assert_eq!(status, StatusCode::OK);
        let (_, again) = completion(shared.clone(), &grant, request.clone()).await;
        assert_eq!(done, again);
        let id: Uuid = serde_json::from_value(done["device_id"].clone()).unwrap();
        let mut changed = request.clone();
        changed.request_id = Uuid::new_v4();
        assert_eq!(
            completion(shared.clone(), &grant, changed).await.0,
            StatusCode::CONFLICT
        );
        {
            let mut engine = shared.lock().unwrap();
            let principal = engine.authority.authenticate(&secret).unwrap();
            assert_eq!(
                engine.authority.current().devices[&principal.device_id()].role,
                neonmix_control::Role::Member
            );
            let revision = engine.authority.current().revision;
            engine
                .authority
                .execute(
                    issuer,
                    Command {
                        request_id: Uuid::new_v4(),
                        expected_revision: revision,
                        operation: neonmix_control::Operation::Revoke { device_id: id },
                    },
                    |_| Ok(()),
                )
                .unwrap();
            assert!(engine.authority.authenticate(&secret).is_err());
        }
        assert_eq!(
            completion(shared.clone(), &grant, request).await.0,
            StatusCode::UNAUTHORIZED
        );
        let result = crate::pairing_api::cancel(
            State(shared),
            headers(&member),
            Json(crate::pairing_api::Cancel {
                invitation_id: Uuid::new_v4(),
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(ApiError(ControlError::PermissionDenied))
        ));
    }
    #[tokio::test]
    async fn persistence_failure_leaves_grant_retryable_and_identity_unregistered() {
        let (shared, issuer, _, _mixer) = shared();
        let now = Instant::now();
        let token = neonmix_identity::secret();
        let (id, grant) = shared
            .lock()
            .unwrap()
            .pairings
            .open(issuer, 30, now)
            .unwrap();
        let request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "disk-fail".into(),
            token_sha256: neonmix_identity::digest(&token),
        };
        shared.lock().unwrap().state_path = Some(std::path::PathBuf::from(
            ".local/tmp/nonexistent-pairing-parent/state.json",
        ));
        assert_eq!(
            completion(shared.clone(), &grant, request.clone()).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        {
            let mut engine = shared.lock().unwrap();
            assert!(engine.authority.authenticate(&token).is_err());
            assert!(matches!(
                engine.pairings.check(&grant, &request, now),
                Ok(Checked::New(_))
            ));
            engine.state_path = None;
        }
        assert_eq!(completion(shared, &grant, request).await.0, StatusCode::OK);
    }
    #[tokio::test]
    async fn administrator_revocation_invalidates_unredeemed_grants_and_attempts_are_bounded() {
        let (shared, issuer, _, _mixer) = shared();
        let (id, grant) = shared
            .lock()
            .unwrap()
            .pairings
            .open(issuer, 30, Instant::now())
            .unwrap();
        let request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "paired".into(),
            token_sha256: neonmix_identity::digest(&neonmix_identity::secret()),
        };
        {
            let mut engine = shared.lock().unwrap();
            engine
                .authority
                .add_device(
                    "other admin".into(),
                    neonmix_control::Role::Admin,
                    &neonmix_identity::secret(),
                )
                .unwrap();
            let revision = engine.authority.current().revision;
            engine
                .authority
                .execute(
                    issuer,
                    Command {
                        request_id: Uuid::new_v4(),
                        expected_revision: revision,
                        operation: neonmix_control::Operation::Revoke {
                            device_id: issuer.device_id(),
                        },
                    },
                    |_| Ok(()),
                )
                .unwrap();
        }
        assert_eq!(
            completion(shared.clone(), &grant, request.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        shared.lock().unwrap().pair_attempts = 32;
        assert_eq!(
            completion(shared, &grant, request).await.0,
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
