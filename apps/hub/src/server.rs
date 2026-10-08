#[path = "admission.rs"]
mod admission;
#[path = "airplay.rs"]
mod airplay;
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
    Authority, Command, ControlError, Preparation, Principal, Session, SessionStatus, Snapshot,
};
use neonmix_core::{
    mixer::{LANES, LaneMix, METER_WINDOW_FRAMES, Mixer, MixerConfig, MixerControl, MixerStats},
    queue::BlockProducer,
};
use neonmix_media::{ReceiveStats, Receiver, certificate_fingerprint};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
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
    binding_generation: u64,
}
struct Resources {
    lanes: Vec<Lane>,
    control: MixerControl,
    stats: Arc<MixerStats>,
    origin: Instant,
    pem: String,
    output_epoch: u64,
    airplay_mix: [Option<(usize, LaneMix)>; 4],
    admissions: admission::Admissions,
    multi_receiver: bool,
    pending_multi_capacity: bool,
    native_reservation: Option<(Uuid, usize, Uuid, u64)>,
    retiring: Vec<(usize, crate::media_worker::MediaWorker)>,
}
const APPLICATION_STALL_AFTER: Duration = Duration::from_secs(2);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaApplication {
    pub runtime_epoch: Uuid,
    pub desired_config_sequence: u64,
    pub applied_config_sequence: u64,
    pub pending: bool,
    pub stalled: bool,
    pub pending_ms: Option<u64>,
    pub queue_rejections: u64,
}
impl Resources {
    fn application(&self, epoch: Uuid, now: Instant) -> MediaApplication {
        let progress = self.stats.config_progress();
        let elapsed = self.control.pending_for(now);
        MediaApplication {
            runtime_epoch: epoch,
            desired_config_sequence: progress.desired_config_sequence,
            applied_config_sequence: progress.applied_config_sequence,
            pending: progress.pending(),
            stalled: elapsed.is_some_and(|duration| duration >= APPLICATION_STALL_AFTER),
            pending_ms: elapsed
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64),
            queue_rejections: progress.queue_rejections,
        }
    }

    fn revoke_terminals(&mut self, state: &Snapshot) {
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            if lane.session.is_some_and(|id| {
                state
                    .sessions
                    .get(&id)
                    .is_none_or(|session| !session.status.active())
            }) {
                self.control.revoke_lane(index);
                if let Some(producer) = lane.producer.as_ref() {
                    producer.revoke_binding();
                }
                if let Some(media) = lane.media.take() {
                    media.revoke();
                    self.retiring.push((index, media));
                }
                lane.session = None;
            }
        }
    }
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
        if remote.is_some() && !self.control.has_capacity() {
            return Err(ControlError::Busy);
        }
        // Native IDs are randomly allocated independently of AirPlay. Reject a
        // collision before preparing resources or committing authority/disk.
        if self.admissions.claims.values().any(|claim| {
            state.streams.contains_key(&claim.context.stream_id)
                || state.streams.contains_key(&claim.context.session_id)
        }) {
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
        if let Some((index, _, _, _)) = &added
            && let Some(producer) = self.lanes[*index].producer.as_mut()
        {
            self.lanes[*index].binding_generation =
                producer.bind_next().map_err(|_| ControlError::Busy)?;
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
                    binding_generation: self.lanes[i].binding_generation,
                    playback_kind: neonmix_core::mixer::PlaybackKind::NativeAdaptive,
                    gain_db: mix.gain_db,
                    muted: mix.muted,
                    solo: mix.solo,
                };
            }
        }
        let native = state
            .sessions
            .values()
            .filter(|s| s.status.active())
            .count()
            + usize::from(self.native_reservation.is_some());
        if !self
            .admissions
            .native_capacity(native, self.multi_receiver || self.pending_multi_capacity)
        {
            return Err(ControlError::QuotaExceeded);
        }
        for (lane, mix) in self.airplay_mix.iter().flatten() {
            config.lanes[*lane] = *mix;
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
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            if lane
                .session
                .is_some_and(|id| state.sessions.get(&id).is_none_or(|s| !s.status.active()))
            {
                if let Some(media) = lane.media.take() {
                    media.revoke();
                    self.retiring.push((index, media));
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
    pub(super) stopping: bool,
    pub(super) pairings: neonmix_identity::pairing::Book<Principal>,
    pub(super) pair_window: Instant,
    pub(super) pair_attempts: u32,
    pub(super) room_name: String,
    pub(super) certificate: String,
    event_slots: Arc<tokio::sync::Semaphore>,
    output_stats: Arc<neonmix_core::stats::AudioStats>,
    pub(super) authority: Authority,
    pub(super) native_responses: VecDeque<(Uuid, Uuid, StartResponse)>,
    pub(super) native_recovery: Option<NativeRecovery>,
    pub(super) persistence_denials: BTreeSet<Uuid>,
    pub(super) pairing_recovery: Option<crate::pairing_api::PairingRecovery>,
    pub(super) recovery_results: VecDeque<(Uuid, serde_json::Value)>,
    resources: Resources,
    airplay: airplay::State,
    errors: Vec<String>,
    pub(super) state_path: Option<std::path::PathBuf>,
}
impl Engine {
    pub(super) fn cache_recovery(&mut self, id: Uuid, response: serde_json::Value) {
        if self.recovery_results.len() == 32 {
            self.recovery_results.pop_front();
        }
        self.recovery_results.push_back((id, response));
    }
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
    persist_bytes_with(path, &bytes, &mut |_| Ok(()))
}
fn persist_bytes_with(
    path: &std::path::Path,
    bytes: &[u8],
    hook: &mut impl FnMut(neonmix_identity::files::ReplaceStage) -> std::io::Result<()>,
) -> std::result::Result<(), ControlError> {
    match neonmix_identity::files::replace_with(path, bytes, hook) {
        Ok(neonmix_identity::files::Publication::Durable) => Ok(()),
        Ok(_) => Err(ControlError::DurabilityUnconfirmed),
        Err(failure)
            if failure.publication == neonmix_identity::files::Publication::NotPublished =>
        {
            Err(ControlError::Busy)
        }
        Err(_) => Err(ControlError::DurabilityUnconfirmed),
    }
}
pub(super) type Shared = Arc<Mutex<Engine>>;
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StartResponse {
    #[serde(default)]
    pub hub_id: Option<Uuid>,
    pub receipt: neonmix_control::Receipt,
    #[serde(default)]
    pub media_pending: bool,
    #[serde(default)]
    pub media_application: Option<MediaApplication>,
    pub session: Option<Session>,
    pub media_port: Option<u16>,
    pub hub_certificate_sha256: String,
}
pub(super) struct ApiError(pub(super) ControlError);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ControlError::UpgradeRequired => StatusCode::UPGRADE_REQUIRED,
            ControlError::Unauthenticated => StatusCode::UNAUTHORIZED,
            ControlError::PermissionDenied | ControlError::PlaybackBlocked => StatusCode::FORBIDDEN,
            ControlError::RevisionConflict
            | ControlError::IdempotencyConflict
            | ControlError::AlreadyActive => StatusCode::CONFLICT,
            ControlError::NotFound => StatusCode::NOT_FOUND,
            ControlError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
            ControlError::Busy | ControlError::DurabilityUnconfirmed => {
                StatusCode::SERVICE_UNAVAILABLE
            }
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
    if engine.stopping {
        return Err(ControlError::Busy.into());
    }

    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(ControlError::Unauthenticated)?;
    let principal = engine.authority.authenticate(token)?;
    if engine.persistence_denials.contains(&principal.device_id()) {
        return Err(ControlError::Unauthenticated.into());
    }
    Ok(principal)
}
async fn me(
    State(shared): State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    let principal = authenticate(&engine, &headers)?;
    let admin = engine
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_some_and(|device| device.role == neonmix_control::Role::Admin);
    let pending = if admin {
        engine
            .native_recovery
            .as_ref()
            .map(|recovery| recovery.command.request_id)
            .or_else(|| {
                engine
                    .pairing_recovery
                    .as_ref()
                    .map(|recovery| recovery.request.request_id)
            })
            .or_else(|| airplay::api::pending_request(&engine))
    } else {
        None
    };
    Ok(Json(
        serde_json::json!({"hub_id":engine.authority.current().hub_id,"room_name":engine.room_name,"device_id":principal.device_id(),"role":engine.authority.current().devices.get(&principal.device_id()).map(|device|device.role),"persistence_pending":engine.authority.transaction_pending() || airplay::api::pending(&engine),"pending_request_id":pending}),
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
    execute_native(
        &shared,
        &headers,
        command,
        remote,
        crate::media_worker::MediaWorker::prepare,
        persist_saved,
    )
}
pub(super) struct NativeRecovery {
    principal_id: Uuid,
    command: Command,
    prepared: neonmix_control::PreparedCommand,
    setup: Option<NativeSetup>,
    media: Option<crate::media_worker::PreparedWorker>,
    fingerprint: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRequest {
    request_id: Uuid,
    runtime_epoch: Uuid,
}
async fn recover_persistence(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(request): Json<RecoveryRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    {
        let engine = shared.lock().map_err(|_| ControlError::Busy)?;
        let principal = authenticate(&engine, &headers)?;
        if engine.authority.current().runtime_epoch != request.runtime_epoch {
            return Err(ControlError::SnapshotRequired.into());
        }
        if engine
            .authority
            .current()
            .devices
            .get(&principal.device_id())
            .is_none_or(|device| device.role != neonmix_control::Role::Admin)
        {
            return Err(ControlError::PermissionDenied.into());
        }
    }
    let transaction = crate::pairing_api::PAIRING_TRANSACTION.lock().await;
    tokio::task::spawn_blocking(move || {
        let _transaction = transaction;
        {
            let engine = shared.lock().map_err(|_| ControlError::Busy)?;
            let principal = authenticate(&engine, &headers)?;
            if engine.authority.current().runtime_epoch != request.runtime_epoch {
                return Err(ControlError::SnapshotRequired.into());
            }
            if engine
                .authority
                .current()
                .devices
                .get(&principal.device_id())
                .is_none_or(|device| device.role != neonmix_control::Role::Admin)
            {
                return Err(ControlError::PermissionDenied.into());
            }
        }
        let native = {
            let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
            if let Some((_, response)) = engine
                .recovery_results
                .iter()
                .find(|(id, _)| *id == request.request_id)
            {
                return Ok(Json(response.clone()));
            }
            if let Some(recovery) = engine.native_recovery.as_ref() {
                if recovery.command.request_id == request.request_id {
                    engine.native_recovery.take()
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(recovery) = native {
            let path = shared
                .lock()
                .map_err(|_| ControlError::Busy)?
                .state_path
                .clone();
            if persist_saved(path.as_ref(), &recovery.prepared.persistent()).is_err() {
                shared
                    .lock()
                    .map_err(|_| ControlError::Busy)?
                    .native_recovery = Some(recovery);
                return Err(ControlError::DurabilityUnconfirmed.into());
            }
            let Json(response) = finish_native(
                &shared,
                recovery.principal_id,
                recovery.command.request_id,
                recovery.prepared,
                recovery.setup,
                recovery.media,
                recovery.fingerprint,
            )?;
            let response = serde_json::to_value(response).map_err(|_| ControlError::Busy)?;
            shared
                .lock()
                .map_err(|_| ControlError::Busy)?
                .cache_recovery(request.request_id, response.clone());
            return Ok(Json(response));
        }
        if let Some(done) =
            airplay::api::recover(&shared, &headers, request.request_id, request.runtime_epoch)?
        {
            return Ok(Json(done));
        }
        let done = crate::pairing_api::recover_registration(&shared, request.request_id).map_err(
            |error| match error {
                crate::pairing_api::Error::Control(control) => ApiError(control),
                _ => ApiError(ControlError::Busy),
            },
        )?;
        let done = serde_json::to_value(done).map_err(|_| ControlError::Busy)?;
        shared
            .lock()
            .map_err(|_| ControlError::Busy)?
            .cache_recovery(request.request_id, done.clone());
        Ok(Json(done))
    })
    .await
    .map_err(|_| ControlError::Busy)?
}
struct NativeSetup {
    lane: usize,
    session: Session,
    producer: Option<BlockProducer>,
}
fn execute_native(
    shared: &Shared,
    headers: &HeaderMap,
    command: Command,
    remote: Connection,
    prepare_media: impl FnOnce(
        Receiver,
        UdpSocket,
        usize,
        Arc<MixerStats>,
    ) -> std::io::Result<crate::media_worker::PreparedWorker>,
    save: impl FnOnce(
        Option<&std::path::PathBuf>,
        &neonmix_control::PersistentState,
    ) -> std::result::Result<(), ControlError>,
) -> std::result::Result<Json<StartResponse>, ApiError> {
    let request_id = command.request_id;
    let frozen_command = command.clone();
    let recovery = {
        let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
        let principal = authenticate(&engine, headers)?;
        if let Some(recovery) = engine.native_recovery.as_ref() {
            if recovery.command != command {
                if recovery.command.request_id == command.request_id {
                    return Err(ControlError::IdempotencyConflict.into());
                }
                // Completed exact replay must still precede an unrelated
                // pending durability candidate, preserving the P01 contract.
                match engine
                    .authority
                    .prepare_transaction(principal, command.clone())
                {
                    Ok(Preparation::Replay(_)) => {
                        let response = engine
                            .native_responses
                            .iter()
                            .find(|(device, request, _)| {
                                *device == principal.device_id() && *request == command.request_id
                            })
                            .map(|(_, _, response)| response.clone())
                            .ok_or(ControlError::SnapshotRequired)?;
                        return Ok(Json(response));
                    }
                    Err(ControlError::Busy) => {
                        return Err(ControlError::DurabilityUnconfirmed.into());
                    }
                    Err(error) => return Err(error.into()),
                    Ok(Preparation::Prepared(unexpected)) => {
                        let _ = engine.authority.abort_transaction(unexpected.token());
                        return Err(ControlError::Busy.into());
                    }
                }
            }
            let admin = engine
                .authority
                .current()
                .devices
                .get(&principal.device_id())
                .is_some_and(|device| device.role == neonmix_control::Role::Admin);
            if recovery.principal_id != principal.device_id() && !admin {
                return Err(ControlError::PermissionDenied.into());
            }
            engine.native_recovery.take()
        } else {
            None
        }
    };
    if let Some(recovery) = recovery {
        let path = shared
            .lock()
            .map_err(|_| ControlError::Busy)?
            .state_path
            .clone();
        if save(path.as_ref(), &recovery.prepared.persistent()).is_err() {
            shared
                .lock()
                .map_err(|_| ControlError::Busy)?
                .native_recovery = Some(recovery);
            return Err(ControlError::DurabilityUnconfirmed.into());
        }
        return finish_native(
            shared,
            recovery.principal_id,
            recovery.command.request_id,
            recovery.prepared,
            recovery.setup,
            recovery.media,
            recovery.fingerprint,
        );
    }
    let (principal_id, prepared, mut setup, path, pem, origin, stats) = {
        let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
        let principal = authenticate(&e, headers)?;
        let prepared = match e.authority.prepare_transaction(principal, command)? {
            Preparation::Replay(_) => {
                let response = e
                    .native_responses
                    .iter()
                    .find(|(device, request, _)| {
                        *device == principal.device_id() && *request == request_id
                    })
                    .map(|(_, _, response)| response.clone())
                    .ok_or(ControlError::SnapshotRequired)?;
                return Ok(Json(response));
            }
            Preparation::Prepared(prepared) => prepared,
        };
        let token = prepared.token();
        let reserve = (|| -> std::result::Result<Option<NativeSetup>, ControlError> {
            let state = prepared.snapshot();
            let count = state
                .sessions
                .values()
                .filter(|s| s.status.active())
                .count();
            if !e.resources.admissions.native_capacity(
                count,
                e.resources.multi_receiver || e.resources.pending_multi_capacity,
            ) {
                return Err(ControlError::QuotaExceeded);
            }
            if e.resources.admissions.claims.values().any(|claim| {
                state.streams.contains_key(&claim.context.stream_id)
                    || state.streams.contains_key(&claim.context.session_id)
            }) {
                return Err(ControlError::Busy);
            }
            let added = state
                .sessions
                .values()
                .find(|s| {
                    s.status.active() && !e.resources.lanes.iter().any(|l| l.session == Some(s.id))
                })
                .cloned();
            if let Some(session) = added {
                if !e.resources.control.has_capacity() {
                    return Err(ControlError::Busy);
                }
                let lane = e
                    .resources
                    .lanes
                    .iter()
                    .position(|l| l.session.is_none() && l.producer.is_some())
                    .ok_or(ControlError::QuotaExceeded)?;
                let mut producer = e.resources.lanes[lane].producer.take();
                if let Some(producer) = producer.as_mut() {
                    e.resources.lanes[lane].binding_generation =
                        producer.bind_next().map_err(|_| ControlError::Busy)?;
                }
                e.resources.native_reservation = Some((token, lane, session.id, session.stream_id));
                Ok(Some(NativeSetup {
                    lane,
                    session,
                    producer,
                }))
            } else {
                Ok(None)
            }
        })();
        let setup = match reserve {
            Ok(setup) => setup,
            Err(error) => {
                let _ = e.authority.abort_transaction(token);
                return Err(error.into());
            }
        };
        (
            principal.device_id(),
            prepared,
            setup,
            e.state_path.clone(),
            e.resources.pem.clone(),
            e.resources.origin,
            e.resources.stats.clone(),
        )
    };
    let token = prepared.token();
    // Socket, certificates, DTLS/decoder initialization and the waiting media
    // thread are prepared without Engine. Reserved producer/lane cannot be reused.
    let preparation = (|| -> std::result::Result<_, ControlError> {
        let fingerprint = certificate_fingerprint(&pem).map_err(|_| ControlError::Busy)?;
        let worker = if let Some(setup) = setup.as_ref() {
            let socket = remote
                .media_socket(setup.session.offer.udp_port)
                .map_err(|_| ControlError::Busy)?;
            let receiver = Receiver::new(setup.session.clone(), &pem, origin)
                .map_err(|_| ControlError::Busy)?;
            Some(
                prepare_media(receiver, socket, setup.lane, stats)
                    .map_err(|_| ControlError::Busy)?,
            )
        } else {
            None
        };
        Ok((worker, fingerprint))
    })();
    let (media, fingerprint) = match preparation {
        Ok(value) => value,
        Err(error) => {
            abort_native(shared, token, &mut setup);
            return Err(error.into());
        }
    };
    // A route loss observed during slow preparation is a real cancellation.
    if setup.is_some()
        && !shared
            .lock()
            .map_err(|_| ControlError::Busy)?
            .authority
            .current()
            .output
            .available
    {
        drop(media);
        abort_native(shared, token, &mut setup);
        return Err(ControlError::PlaybackBlocked.into());
    }
    if let Err(error) = save(path.as_ref(), &prepared.persistent()) {
        if error == ControlError::DurabilityUnconfirmed {
            let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
            engine.resources.revoke_terminals(prepared.snapshot());
            engine.persistence_denials = prepared
                .snapshot()
                .devices
                .values()
                .filter(|device| device.revoked)
                .map(|device| device.id)
                .collect();
            engine.native_recovery = Some(NativeRecovery {
                principal_id,
                command: frozen_command,
                prepared,
                setup,
                media,
                fingerprint,
            });
            return Err(error.into());
        }
        drop(media);
        abort_native(shared, token, &mut setup);
        return Err(error.into());
    }
    finish_native(
        shared,
        principal_id,
        request_id,
        prepared,
        setup,
        media,
        fingerprint,
    )
}
fn finish_native(
    shared: &Shared,
    principal_id: Uuid,
    request_id: Uuid,
    prepared: neonmix_control::PreparedCommand,
    setup: Option<NativeSetup>,
    media: Option<crate::media_worker::PreparedWorker>,
    fingerprint: String,
) -> std::result::Result<Json<StartResponse>, ApiError> {
    let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
    let receipt = e.authority.commit_transaction(prepared)?;
    e.persistence_denials.clear();
    e.resources.native_reservation = None;
    if let (Some(mut setup), Some(media)) = (setup, media) {
        let allowed = !e.stopping
            && e.authority
                .current()
                .sessions
                .get(&setup.session.id)
                .is_some_and(|session| session.status.active());
        if allowed {
            let lane = &mut e.resources.lanes[setup.lane];
            lane.media = Some(media.activate(setup.producer.take().expect("reserved producer")));
            lane.session = Some(setup.session.id);
        } else {
            drop(media);
            e.resources.lanes[setup.lane].producer = setup.producer.take();
            let unavailable = !e.authority.current().output.available;
            let _ = e.authority.set_session_status(
                setup.session.id,
                if unavailable {
                    SessionStatus::OutputLost
                } else {
                    SessionStatus::AdminDisconnected
                },
            );
        }
    }
    let state = e.authority.snapshot();
    e.resources.revoke_terminals(&state);
    // Persistence is already committed: transient Mixer saturation is retried
    // from the current desired state and must not turn into a false failure.
    e.airplay.mixer_dirty = e.resources.prepare(&state, None).is_err();
    let session = receipt
        .session_id
        .and_then(|id| state.sessions.get(&id))
        .cloned();
    let port = receipt
        .session_id
        .and_then(|id| e.resources.lanes.iter().find(|l| l.session == Some(id)))
        .and_then(|l| l.media.as_ref())
        .map(|m| m.port);
    let application = e.resources.application(state.runtime_epoch, Instant::now());
    let response = StartResponse {
        media_pending: application.pending,
        media_application: Some(application),
        hub_id: Some(state.hub_id),
        receipt,
        session,
        media_port: port,
        hub_certificate_sha256: fingerprint,
    };
    if e.native_responses.len() == 128 {
        e.native_responses.pop_front();
    }
    // Stored under the commit lock, before a reply can be lost. Replay never
    // rebuilds ports/session data from a later state or revives ended media.
    e.native_responses
        .push_back((principal_id, request_id, response.clone()));
    Ok(Json(response))
}
fn abort_native(shared: &Shared, token: Uuid, setup: &mut Option<NativeSetup>) {
    if let Ok(mut e) = shared.lock() {
        if e.resources.native_reservation.is_some_and(|r| r.0 == token) {
            if let Some(setup) = setup.as_mut() {
                e.resources.lanes[setup.lane].producer = setup.producer.take();
            }
            e.resources.native_reservation = None;
        }
        let _ = e.authority.abort_transaction(token);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: u64,
    #[serde(default)]
    control_version: Option<u16>,
    #[serde(default)]
    runtime_epoch: Option<Uuid>,
}
impl Cursor {
    fn read(
        &self,
        authority: &Authority,
    ) -> std::result::Result<Vec<neonmix_control::Event>, ControlError> {
        if self.control_version != Some(neonmix_control::CONTROL_VERSION) {
            return Err(ControlError::UpgradeRequired);
        }
        authority.events_after_in_runtime(
            self.runtime_epoch.ok_or(ControlError::SnapshotRequired)?,
            self.after,
        )
    }
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
        cursor.read(&engine.authority)?;
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
                        Ok(engine) => authenticate(&engine, &headers).and_then(|_| engine.authority.events_after_in_runtime(cursor.runtime_epoch.unwrap(),revision).map_err(ApiError)),
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
    let mut lane_stream_ids: Vec<_> = engine
        .resources
        .lanes
        .iter()
        .map(|l| {
            l.session
                .and_then(|id| engine.authority.current().sessions.get(&id))
                .map(|s| s.stream_id)
        })
        .collect();
    for (lane, mix) in engine.resources.airplay_mix.iter().flatten() {
        lane_stream_ids[*lane] = Some(mix.stream_id);
    }
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
        .enumerate()
        .map(|(index, (expected, meter))| {
            let measured = meter.snapshot();
            let stream_id = expected.unwrap_or(0);
            // Control state can advance before the output callback applies it.
            // Never attach the previous occupant's samples to a new stream.
            let native = engine.resources.lanes[index]
                .session
                .and_then(|id| engine.authority.current().sessions.get(&id));
            let airplay = engine
                .resources
                .airplay_mix
                .iter()
                .flatten()
                .find(|(lane, _)| *lane == index)
                .map(|(_, mix)| mix);
            let epoch = native
                .map(|s| s.offer.stream_epoch)
                .or_else(|| airplay.map(|m| m.epoch));
            let generation = airplay
                .map_or(engine.resources.lanes[index].binding_generation, |m| {
                    m.binding_generation
                });
            let valid = measured.observed
                && output_available
                && stream_id != 0
                && measured.stream_id == stream_id
                && Some(measured.stream_epoch) == epoch
                && measured.binding_generation == generation
                && measured.output_epoch == engine.resources.output_epoch;
            serde_json::json!({
                "stream_id": stream_id,
                "stream_epoch": epoch,
                "session_id": native.map(|s| s.id),
                "available": valid,
                "peak": valid.then_some(measured.peak),
                "rms": valid.then_some(measured.rms),
            })
        })
        .collect();
    let output_meter = stats.output_meter.snapshot();
    let meter_sample_age_ms = output_meter
        .observed
        .then(|| engine.resources.origin.elapsed().as_nanos())
        .and_then(|now| now.checked_sub(u128::from(output_meter.sampled_at_ns)))
        .map(|age| (age / 1_000_000).min(u128::from(u64::MAX)) as u64);
    let output_meter_valid = output_meter.observed
        && output_available
        && output_meter.output_epoch == engine.resources.output_epoch;
    let meters = serde_json::json!({
        "sample_age_ms": meter_sample_age_ms,
        "window_frames": METER_WINDOW_FRAMES,
        "sample_rate": 48000,
        "lanes": lane_meters,
        "output": {
            "available": output_meter_valid,
            "peak": output_meter_valid.then_some(output_meter.peak),
            "rms": output_meter_valid.then_some(output_meter.rms),
        },
        "limiter_gain": output_meter_valid.then(|| f32::from_bits(stats.limiter_gain_bits.load(Relaxed) as u32)),
    });
    Ok(Json(
        serde_json::json!({"available":true,"sample_age_ms":0,"runtime_epoch":engine.authority.current().runtime_epoch,"media_application":engine.resources.application(engine.authority.current().runtime_epoch,Instant::now()),"airplay":engine.airplay.diagnostic(),"timed_late_frames":stats.timed_late_frames.load(Relaxed),"timed_late_frames_by_lane":stats.timed_late_frames_by_lane.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_timed_late_stream_id":stats.last_timed_late_stream_id.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_timed_late_epoch":stats.last_timed_late_epoch.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_timed_late_output_frame":stats.last_timed_late_output_frame.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_timed_late_target_ns":stats.last_timed_late_target_ns.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_timed_late_presentation_ns":stats.last_timed_late_presentation_ns.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"timed_drift_ppm":stats.timed_drift_ppm_milli.iter().map(|a|a.load(Relaxed) as i64 as f64 /1000.0).collect::<Vec<_>>(),"timed_phase_error_ns":stats.timed_phase_error_ns.iter().map(|a|a.load(Relaxed) as i64).collect::<Vec<_>>(),"meters":meters,"receivers":receivers,"media_workers":media_workers,"lane_stream_ids":lane_stream_ids,"packet_budget_drops":budget_drops,"pcm_queue_age_max_ns":stats.pcm_queue_age_max_ns.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"fifo_frames":stats.fifo_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"sinc_delay_frames":stats.sinc_delay_frames.load(Relaxed),"output_stats":engine.output_stats.snapshot(),"output_frames":stats.output_frames.load(Relaxed),"limited_frames":stats.limited_frames.load(Relaxed),"underrun_frames":stats.underrun_frames.load(Relaxed),"last_underrun_output_frame":stats.last_underrun_output_frame.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_fifo_frames":stats.last_underrun_fifo_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_needed_frames":stats.last_underrun_needed_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"last_underrun_source_end":stats.last_underrun_source_end.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"underrun_frames_by_lane":stats.underrun_frames_by_lane.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"rendered_pcm_frames_by_lane":stats.rendered_pcm_frames_by_lane.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"render_state_by_lane":stats.render_state_by_lane.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"filtered_queues":stats.filtered_queue_frames.iter().map(|a|a.load(Relaxed)).collect::<Vec<_>>(),"queues":stats.queue_frames.iter().map(|a| a.load(Relaxed)).collect::<Vec<_>>(),"drift_ppm":stats.drift_ppm_milli.iter().map(|a| a.load(Relaxed) as i64 as f64 / 1000.0).collect::<Vec<_>>(),"persistence":{"pending":engine.authority.transaction_pending() || airplay::api::pending(&engine),"durable":!(engine.authority.transaction_pending() || airplay::api::pending(&engine))},"errors":engine.errors}),
    ))
}
fn worker(shared: Shared, stopped: Arc<AtomicBool>) {
    while !stopped.load(Acquire) {
        if let Ok(mut engine) = shared.lock() {
            if engine.airplay.mixer_dirty {
                let state = engine.authority.snapshot();
                if engine.resources.prepare(&state, None).is_ok() {
                    engine.airplay.mixer_dirty = false;
                }
            }
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
        let retiring = shared
            .lock()
            .ok()
            .map(|mut e| std::mem::take(&mut e.resources.retiring))
            .unwrap_or_default();
        for (index, media) in retiring {
            let producer = media.close();
            if let Ok(mut e) = shared.lock() {
                e.resources.lanes[index].producer = producer;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub async fn serve(
    config: ServerConfig,
    listen: SocketAddr,
    profile_directory: std::path::PathBuf,
    stop: &neonmix_lifecycle::StopSignal,
) -> Result<()> {
    if profile_directory.ancestors().any(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".neonmix-migration-") && n.ends_with(".staging"))
    }) {
        return Err("migration_incomplete: staging is not a runnable state directory".into());
    }
    // One owner per persistent Hub. The lock file stays at a stable inode while
    // the data file is atomically replaced on successful control commits.
    let owner_path = if let Some(path) = &config.state_path {
        path.with_extension("lock")
    } else {
        neonmix_identity::files::private_dir(&profile_directory)?;
        profile_directory.join("hub-owner.lock")
    };
    let _state_lock = neonmix_identity::profiles::operation_lock(&owner_path)?;
    neonmix_media::runtime_probe()?;
    let admin = config
        .devices
        .first()
        .filter(|d| d.role == neonmix_control::Role::Admin)
        .ok_or("first provisioned device must be admin")?;
    let mut authority = if let Some(path) = config.state_path.as_ref().filter(|p| p.exists()) {
        let saved: neonmix_control::PersistentState = if config.require_existing_state {
            serde_json::from_slice(&neonmix_identity::files::read_private(
                path,
                neonmix_identity::profiles::MAX_STATE_BYTES,
            )?)
            .map_err(|_| "credential_corrupt")?
        } else {
            crate::read(path)?
        };
        if saved.output.id != config.output {
            return Err("persisted output differs from explicitly selected device".into());
        }
        Authority::restore(saved)?
    } else {
        if config.require_existing_state {
            return Err("setup_incomplete: state file missing".into());
        }
        let mut authority =
            Authority::new(config.output.clone(), admin.name.clone(), &admin.token)?;
        for device in &config.devices[1..] {
            authority.add_device(device.name.clone(), device.role, &device.token)?;
        }
        authority
    };
    if config.require_existing_state {
        let principal = authority.authenticate(&admin.token)?;
        let profile = neonmix_identity::profiles::token(&profile_directory.join("admin.json"))?;
        if authority
            .current()
            .devices
            .get(&principal.device_id())
            .is_none_or(|d| d.role != neonmix_control::Role::Admin)
            || profile.hub_id != authority.current().hub_id
            || profile.device_id != Some(principal.device_id())
            || profile.certificate != config.certificate
        {
            return Err("credential_kind_mismatch".into());
        }
    }
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
                // PipeWire and WASAPI negotiate the device's supported period.
                period_frames: if cfg!(any(target_os = "linux", target_os = "windows")) {
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
            binding_generation: 1,
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
        stopping: false,
        pairings: Default::default(),
        pair_window: Instant::now(),
        pair_attempts: 0,
        room_name: room_name.clone(),
        certificate: config.certificate.clone(),
        event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
        output_stats: output.as_ref().map(|o| o.stats.clone()).unwrap_or_default(),
        authority,
        native_responses: Default::default(),
        native_recovery: None,
        persistence_denials: Default::default(),
        pairing_recovery: None,
        recovery_results: Default::default(),
        resources: Resources {
            lanes,
            control,
            stats,
            origin,
            pem: config.pem,
            output_epoch: 0,
            airplay_mix: [None; 4],
            admissions: Default::default(),
            multi_receiver: false,
            pending_multi_capacity: false,
            native_reservation: None,
            retiring: Vec::new(),
        },
        airplay: airplay::State::new(profile_directory.join("airplay"), listen),
        errors: Vec::new(),
        state_path: config.state_path,
    }));
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_shared = shared.clone();
    let worker_stop = stopped.clone();
    airplay::load_existing(&shared)?;
    let app = Router::new()
        .route("/v1/identity", get(crate::pairing_api::identity))
        .route("/v1/pairing/invitations", post(crate::pairing_api::open))
        .route("/v1/pairing/cancel", post(crate::pairing_api::cancel))
        .route("/v1/pairing/complete", post(crate::pairing_api::complete))
        .route("/v1/airplay", get(airplay::snapshot).post(airplay::command))
        .route(
            "/v2/airplay",
            get(airplay::api::snapshot).post(airplay::api::command),
        )
        .route("/v1/me", get(me))
        .route("/v1/hub", get(snapshot))
        .route("/v1/devices", get(devices))
        .route("/v1/streams", get(streams))
        .route("/v1/outputs", get(outputs))
        .route("/v1/commands", post(command))
        .route("/v1/persistence/recover", post(recover_persistence))
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
    let mut publisher = Some(neonmix_identity::discovery::Publisher::new(
        hub_id,
        &room_name,
        advertised,
        &discovery_fingerprint,
    )?);
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
    let stop_signal = stop.wait();
    tokio::pin!(stop_signal);
    let mut stop_requested = false;
    let mut retry_output = Instant::now();
    let result = loop {
        tokio::select! {
            result = &mut server => break result.map_err(Into::into),
            _ = &mut stop_signal, if !stop_requested => {
                stop_requested = true;
                emit(serde_json::json!({"event":"shutdown_started"}))?;
                if let Ok(mut engine)=shared.lock() {
                    engine.stopping=true;
                    engine.airplay.stop();
                    engine.resources.airplay_mix=[None;4];
                    let _=engine.authority.set_output_available(false);
                    let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);
                }
                drop(publisher.take());
                shutdown.graceful_shutdown(Some(Duration::from_secs(1)));
            },
            _ = monitor.tick(), if !stop_requested => {
                if let Some(position)=output.as_ref().and_then(|o|o.latest_position())&& let Ok(engine)=shared.lock() {engine.airplay.output_latency_ns.store(position.device_timestamp_ns.saturating_sub(position.clock_timestamp_ns),Relaxed);}
                if let Some(position)=output.as_ref().and_then(|o|o.latest_position())&& let Ok(mut engine)=shared.lock()&& engine.resources.output_epoch!=position.epoch {engine.resources.output_epoch=position.epoch;engine.airplay.output_epoch_changed();engine.resources.airplay_mix=[None; 4];let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}
                if output.as_ref().is_some_and(|o|o.stats.snapshot().errors>0) {
                    if let Some(o)=output.take() {o.control.stop();drop(o);}
                    if let Ok(mut engine)=shared.lock() {let _=engine.authority.set_output_available(false);engine.airplay.output_epoch_changed();engine.resources.airplay_mix=[None; 4];let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}
                    emit(serde_json::json!({"event":"output_lost","device":output_id}))?;
                    retry_output=Instant::now()+Duration::from_secs(1);
                }
                if output.is_none()&& let Ok(mut mixer)=returned.try_recv() {
                    mixer.discard_backlog();
                    if Instant::now()>=retry_output {
                        retry_output=Instant::now()+Duration::from_secs(1);
                        let source=crate::recoverable::RecoverableMixer {mixer:Some(mixer),returned:returned_to.clone()};
                        if let Ok(running)=crate::backend().and_then(|b|b.open_output(&output_id,neonmix_io::OpenOptions {sample_rate:None,period_frames:if cfg!(any(target_os = "linux", target_os = "windows")) {None} else {Some(256)}},source))
                            && running.play().is_ok() {if let Ok(mut engine)=shared.lock() {engine.output_stats=running.stats.clone();let _=engine.authority.set_output_available(true);let state=engine.authority.snapshot();let _=engine.resources.prepare(&state,None);}output=Some(running);emit(serde_json::json!({"event":"output_reopened","device":output_id}))?;}
                    } else {let _=returned_to.try_send(mixer);}
                }
            }
        }
    };
    let airplay_thread = shared.lock().ok().map(|mut engine| {
        engine.airplay.stop();
        engine.airplay.take_threads()
    });
    if let Some(threads) = airplay_thread {
        for thread in threads {
            let _ = thread.join();
        }
    }
    stopped.store(true, Release);
    let _ = worker.join();
    drop(output);
    let (forced, cleanup_failures, shutdown_failures) = shared
        .lock()
        .map(|e| {
            (
                e.airplay.worker_forced.load(Acquire),
                e.airplay.cleanup_failures.load(Relaxed),
                e.airplay.shutdown_failures.load(Relaxed),
            )
        })
        .unwrap_or((true, 1, 1));
    emit(
        serde_json::json!({"event":"shutdown_complete","forced":forced,"cleanup_complete":cleanup_failures==0,"cleanup_failures":cleanup_failures,"shutdown_failures":shutdown_failures}),
    )?;
    if shutdown_failures > 0 {
        return Err("stop_failed".into());
    }

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
            control_version: neonmix_control::CONTROL_VERSION,
            config_revision: 1,
            event_sequence: 0,
            runtime_epoch: Uuid::new_v4(),
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
                    binding_generation: 1,
                })
                .collect(),
            control,
            stats,
            origin,
            pem,
            output_epoch: 1,
            airplay_mix: [None; 4],
            admissions: Default::default(),
            multi_receiver: false,
            pending_multi_capacity: false,
            native_reservation: None,
            retiring: Vec::new(),
        };
        (resources, snapshot, mixer)
    }
    pub(super) fn shared_fixture() -> (Shared, HeaderMap, Command, Mixer) {
        let (resources, sample, mixer) = fixture();
        let token = neonmix_identity::secret();
        let authority = Authority::new("test-output".into(), "admin".into(), &token).unwrap();
        let command = Command {
            control_version: 1,
            expected_config_revision: None,
            expected_event_sequence: None,
            runtime_epoch: None,
            credential_id: None,
            request_id: Uuid::new_v4(),
            expected_revision: Some(authority.current().revision),
            operation: neonmix_control::Operation::Start {
                offer: sample.sessions.values().next().unwrap().offer.clone(),
            },
        };
        let shared = Arc::new(Mutex::new(Engine {
            stopping: false,
            pairings: Default::default(),
            pair_window: Instant::now(),
            pair_attempts: 0,
            room_name: "native-tx".into(),
            certificate: String::new(),
            event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
            output_stats: Default::default(),
            authority,
            native_responses: Default::default(),
            native_recovery: None,
            persistence_denials: Default::default(),
            pairing_recovery: None,
            recovery_results: Default::default(),
            resources,
            airplay: airplay::State::new(
                std::path::PathBuf::from("unused-native-fixture"),
                "127.0.0.1:0".parse().unwrap(),
            ),
            errors: Vec::new(),
            state_path: None,
        }));
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        (shared, headers, command, mixer)
    }
    #[test]
    fn native_stop_and_unknown_revoke_close_buffered_pcm_before_full_config_queue_without_harming_timed_lane()
     {
        use neonmix_core::{AudioBlock, AudioFormat, signal::StereoSource};
        for unknown in [false, true] {
            let (shared, headers, mut start, mut mixer) = shared_fixture();
            let (session, stream, epoch, origin, principal_id) = {
                let mut e = shared.lock().unwrap();
                let principal = authenticate(&e, &headers).map_err(|e| e.0).unwrap();
                if unknown {
                    e.authority
                        .add_device(
                            "spare admin".into(),
                            neonmix_control::Role::Admin,
                            "spare-admin-token-with-at-least-32-bytes",
                        )
                        .unwrap();
                    start.expected_revision = Some(e.authority.current().revision);
                }
                let receipt = e.authority.execute(principal, start, |_| Ok(())).unwrap();
                let id = receipt.session_id.unwrap();
                let stream = receipt.stream_id.unwrap();
                e.resources.lanes[0].session = Some(id);
                e.resources.lanes[0].binding_generation = e.resources.lanes[0]
                    .producer
                    .as_mut()
                    .unwrap()
                    .bind_next()
                    .unwrap();
                let generation = e.resources.lanes[1]
                    .producer
                    .as_mut()
                    .unwrap()
                    .bind_next()
                    .unwrap();
                e.resources.airplay_mix[0] = Some((
                    1,
                    LaneMix {
                        stream_id: 71,
                        epoch: 1,
                        binding_generation: generation,
                        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
                        ..Default::default()
                    },
                ));
                let command = Command::bound(
                    e.authority.current(),
                    principal.device_id(),
                    neonmix_control::Operation::OutputMix {
                        gain_db: Some(0.),
                        muted: None,
                    },
                );
                e.authority.execute(principal, command, |_| Ok(())).unwrap();
                let state = e.authority.snapshot();
                e.resources.prepare(&state, None).unwrap();
                for index in 0..8 {
                    for (lane, key, value, pts) in [
                        (0, stream, 0.7, None),
                        (1, 71, 0.1, Some(1_000_000_000 + index * 10_000_000)),
                    ] {
                        let mut block = AudioBlock::empty(key, AudioFormat::INTERNAL);
                        block.header.discontinuity_flags = neonmix_core::Discontinuity::NONE;
                        block.header.stream_epoch = 1;
                        block.header.frame_count = 480;
                        block.header.arrival_ns = u64::MAX;
                        block.header.source_sample_position = index * 480;
                        block.header.presentation_time_ns = pts;
                        block.pcm.fill([value; 2]);
                        assert!(
                            e.resources.lanes[lane]
                                .producer
                                .as_mut()
                                .unwrap()
                                .push(block)
                        );
                    }
                }
                (id, stream, 1, e.resources.origin, principal.device_id())
            };
            mixer.set_presentation_time(origin + Duration::from_secs(1));
            let mut warm = [[0.; 2]; 1440];
            mixer.render_block(&mut warm);
            assert!(warm[1200][0] > 0.75);
            let command = {
                let mut e = shared.lock().unwrap();
                let state = e.authority.snapshot();
                while e.resources.control.has_capacity() {
                    e.resources.prepare(&state, None).unwrap();
                }
                Command::bound(
                    e.authority.current(),
                    principal_id,
                    if unknown {
                        neonmix_control::Operation::Revoke {
                            device_id: principal_id,
                        }
                    } else {
                        neonmix_control::Operation::Stop {
                            session_id: session,
                        }
                    },
                )
            };
            let result = execute_native(
                &shared,
                &headers,
                command,
                remote(),
                |_, _, _, _| panic!("limiting command cannot prepare a new SDK"),
                |_, _| {
                    if unknown {
                        Err(ControlError::DurabilityUnconfirmed)
                    } else {
                        Ok(())
                    }
                },
            );
            if unknown {
                assert!(matches!(
                    result,
                    Err(ApiError(ControlError::DurabilityUnconfirmed))
                ));
            } else {
                assert!(result.map_err(|e| e.0).unwrap().0.media_pending);
            }
            let mut output = [[0.; 2]; 480];
            mixer.render_block(&mut output);
            assert!(
                output.iter().all(|frame| frame[0] < 0.11),
                "native FIFO survived limiting gate"
            );
            assert!(
                output.iter().any(|frame| frame[0] > 0.09),
                "independent timed source stopped"
            );
            let e = shared.lock().unwrap();
            assert!(e.resources.lanes[0].session.is_none());
            assert_eq!(e.resources.airplay_mix[0].unwrap().1.stream_id, 71);
            assert_eq!(
                e.authority.current().sessions[&session].offer.stream_epoch,
                epoch
            );
            let _ = stream;
        }
    }
    #[test]
    fn pending_application_exposes_elapsed_stall_and_clears_only_after_actual_callback() {
        let (shared, _, _, mut mixer) = shared_fixture();
        let mut e = shared.lock().unwrap();
        let state = e.authority.snapshot();
        e.resources.prepare(&state, None).unwrap();
        let application = e
            .resources
            .application(state.runtime_epoch, Instant::now() + Duration::from_secs(3));
        assert!(application.pending && application.stalled);
        assert_eq!(application.applied_config_sequence, 0);
        mixer.discard_backlog();
        assert!(
            e.resources
                .application(state.runtime_epoch, Instant::now())
                .pending
        );
        mixer.render_block(&mut [[0.; 2]; 1]);
        let done = e.resources.application(state.runtime_epoch, Instant::now());
        assert!(!done.pending && !done.stalled);
        assert_eq!(done.desired_config_sequence, done.applied_config_sequence);
    }

    #[test]
    fn unknown_start_keeps_the_exact_candidate_and_replay_never_prepares_another_worker() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let mut prepared_count = 0;
        let first = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            |receiver, socket, lane, stats| {
                prepared_count += 1;
                crate::media_worker::MediaWorker::prepare(receiver, socket, lane, stats)
            },
            |_, _| Err(ControlError::DurabilityUnconfirmed),
        );
        assert!(matches!(
            first,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let expected_session = {
            let engine = shared.lock().unwrap();
            assert!(engine.authority.current().sessions.is_empty());
            assert!(
                engine
                    .resources
                    .lanes
                    .iter()
                    .all(|lane| lane.media.is_none())
            );
            let pending = engine.native_recovery.as_ref().unwrap();
            assert_eq!(pending.command, command);
            pending.prepared.receipt().session_id.unwrap()
        };
        let failed_retry = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            |_, _, _, _| panic!("replay prepared another SDK worker"),
            |_, _| Err(ControlError::Busy),
        );
        assert!(matches!(
            failed_retry,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let mut changed = command.clone();
        changed.request_id = Uuid::new_v4();
        assert!(matches!(
            execute_native(
                &shared,
                &headers,
                changed,
                remote(),
                |_, _, _, _| panic!("other request created a worker"),
                |_, _| panic!("other request wrote persistence")
            ),
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let Json(recovered) = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            |_, _, _, _| panic!("recovery regenerated media"),
            |_, _| Ok(()),
        )
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(recovered.receipt.session_id, Some(expected_session));
        assert!(recovered.media_port.is_some());
        assert_eq!(prepared_count, 1);
        assert!(shared.lock().unwrap().native_recovery.is_none());
        let Json(replayed) = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |_, _, _, _| panic!("completed replay prepared media"),
            |_, _| panic!("completed replay wrote persistence"),
        )
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(
            serde_json::to_value(recovered).unwrap(),
            serde_json::to_value(replayed).unwrap()
        );
    }
    #[test]
    fn published_revoke_keeps_restriction_and_closes_gate_with_a_full_config_queue() {
        let (shared, admin_headers, mut start, _mixer) = shared_fixture();
        let member_token = "b".repeat(64);
        let member = {
            let mut engine = shared.lock().unwrap();
            let id = engine
                .authority
                .add_device(
                    "Member".into(),
                    neonmix_control::Role::Member,
                    &member_token,
                )
                .unwrap();
            start.expected_revision = Some(engine.authority.current().revision);
            id
        };
        let mut member_headers = HeaderMap::new();
        member_headers.insert(
            "authorization",
            format!("Bearer {member_token}").parse().unwrap(),
        );
        let Json(active) = execute_native(
            &shared,
            &member_headers,
            start,
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| Ok(()),
        )
        .map_err(|error| error.0)
        .unwrap();
        let revoke = {
            let mut engine = shared.lock().unwrap();
            while engine.resources.control.has_capacity() {
                engine
                    .resources
                    .control
                    .apply(MixerConfig::default())
                    .unwrap();
            }
            Command {
                control_version: 1,
                expected_config_revision: None,
                expected_event_sequence: None,
                runtime_epoch: None,
                credential_id: None,
                request_id: Uuid::new_v4(),
                expected_revision: Some(engine.authority.current().revision),
                operation: neonmix_control::Operation::Revoke { device_id: member },
            }
        };
        let unknown = execute_native(
            &shared,
            &admin_headers,
            revoke.clone(),
            remote(),
            |_, _, _, _| panic!("revoke prepared media"),
            |_, _| Err(ControlError::DurabilityUnconfirmed),
        );
        assert!(matches!(
            unknown,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        {
            let engine = shared.lock().unwrap();
            assert!(authenticate(&engine, &member_headers).is_err());
            assert!(
                engine
                    .resources
                    .lanes
                    .iter()
                    .all(|lane| lane.media.is_none())
            );
            assert!(engine.resources.native_reservation.is_none());
            assert!(engine.native_recovery.is_some());
        }
        let retry = execute_native(
            &shared,
            &admin_headers,
            revoke.clone(),
            remote(),
            |_, _, _, _| panic!(),
            |_, _| Err(ControlError::Busy),
        );
        assert!(matches!(
            retry,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        assert!(authenticate(&shared.lock().unwrap(), &member_headers).is_err());
        let _ = execute_native(
            &shared,
            &admin_headers,
            revoke,
            remote(),
            |_, _, _, _| panic!(),
            |_, _| Ok(()),
        )
        .map_err(|error| error.0)
        .unwrap();
        let engine = shared.lock().unwrap();
        assert!(engine.authority.current().devices[&member].revoked);
        assert_eq!(
            engine.authority.current().sessions[&active.receipt.session_id.unwrap()].status,
            SessionStatus::Revoked
        );
        assert!(engine.airplay.mixer_dirty);
    }
    #[test]
    fn real_published_equal_bytes_with_sync_error_are_not_turned_into_saved_success() {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let directory = project
            .join(".local/tmp")
            .join(format!("persist-phase-{}", Uuid::new_v4()));
        neonmix_identity::files::private_dir(&directory).unwrap();
        let path = directory.join("state.json");
        neonmix_identity::files::write_new(&path, b"same bytes").unwrap();
        let failed = persist_bytes_with(&path, b"same bytes", &mut |stage| {
            if stage == neonmix_identity::files::ReplaceStage::SyncPublishedDirectory {
                Err(std::io::ErrorKind::Other.into())
            } else {
                Ok(())
            }
        });
        assert_eq!(failed, Err(ControlError::DurabilityUnconfirmed));
        assert_eq!(
            neonmix_identity::files::read_private(&path, 64).unwrap(),
            b"same bytes"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn recovery_endpoint_requires_admin_and_runtime_and_preserves_original_response() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let error = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| Err(ControlError::DurabilityUnconfirmed),
        );
        assert!(matches!(
            error,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let epoch = shared.lock().unwrap().authority.current().runtime_epoch;
        let wrong = recover_persistence(
            State(shared.clone()),
            headers.clone(),
            Json(RecoveryRequest {
                request_id: command.request_id,
                runtime_epoch: Uuid::new_v4(),
            }),
        )
        .await;
        assert!(matches!(
            wrong,
            Err(ApiError(ControlError::SnapshotRequired))
        ));
        let member_token = "b".repeat(64);
        let mut member_headers = HeaderMap::new();
        member_headers.insert(
            "authorization",
            format!("Bearer {member_token}").parse().unwrap(),
        );
        // The fixture's pending Start freezes durable mutations. A fresh,
        // unrecognized identity cannot turn a query into an authorization grant.
        let unauthorized = recover_persistence(
            State(shared.clone()),
            member_headers,
            Json(RecoveryRequest {
                request_id: command.request_id,
                runtime_epoch: epoch,
            }),
        )
        .await;
        assert!(matches!(
            unauthorized,
            Err(ApiError(ControlError::Unauthenticated))
        ));
        let Json(recovered) = recover_persistence(
            State(shared.clone()),
            headers.clone(),
            Json(RecoveryRequest {
                request_id: command.request_id,
                runtime_epoch: epoch,
            }),
        )
        .await
        .map_err(|error| error.0)
        .unwrap();
        assert!(!recovered["receipt"]["session_id"].is_null());
        let Json(again) = recover_persistence(
            State(shared.clone()),
            headers.clone(),
            Json(RecoveryRequest {
                request_id: command.request_id,
                runtime_epoch: epoch,
            }),
        )
        .await
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(
            again, recovered,
            "lost recovery reply started another transaction"
        );
        let Json(replayed) = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |_, _, _, _| panic!("operator recovery duplicated SDK work"),
            |_, _| panic!("operator recovery replay wrote state"),
        )
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(recovered, serde_json::to_value(replayed).unwrap());
    }
    #[test]
    fn completed_exact_replay_is_not_blocked_by_a_different_unknown_transaction() {
        let (shared, headers, start, _mixer) = shared_fixture();
        let Json(original) = execute_native(
            &shared,
            &headers,
            start.clone(),
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| Ok(()),
        )
        .map_err(|error| error.0)
        .unwrap();
        let pending = {
            let engine = shared.lock().unwrap();
            Command {
                control_version: 1,
                expected_config_revision: None,
                expected_event_sequence: None,
                runtime_epoch: None,
                credential_id: None,
                request_id: Uuid::new_v4(),
                expected_revision: Some(engine.authority.current().revision),
                operation: neonmix_control::Operation::OutputMix {
                    gain_db: Some(-18.),
                    muted: None,
                },
            }
        };
        assert!(matches!(
            execute_native(
                &shared,
                &headers,
                pending,
                remote(),
                |_, _, _, _| panic!(),
                |_, _| Err(ControlError::DurabilityUnconfirmed)
            ),
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let Json(replayed) = execute_native(
            &shared,
            &headers,
            start,
            remote(),
            |_, _, _, _| panic!("completed replay prepared media during pending persistence"),
            |_, _| panic!("completed replay saved during another transaction"),
        )
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(
            serde_json::to_value(original).unwrap(),
            serde_json::to_value(replayed).unwrap()
        );
    }
    fn remote() -> Connection {
        Connection {
            remote: "127.0.0.1:54321".parse().unwrap(),
            local: "127.0.0.1:0".parse().unwrap(),
        }
    }
    #[test]
    fn native_http_replay_returns_original_response_without_external_work() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let Json(first) = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| Ok(()),
        )
        .map_err(|e| e.0)
        .unwrap();
        let revision = shared.lock().unwrap().authority.current().revision;
        let Json(replayed) = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |_, _, _, _| panic!("Replay must not create media"),
            |_, _| panic!("Replay must not persist"),
        )
        .map_err(|e| e.0)
        .unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(replayed).unwrap()
        );
        let e = shared.lock().unwrap();
        assert_eq!(e.authority.current().revision, revision);
        assert!(e.resources.native_reservation.is_none());
    }
    #[test]
    fn native_http_replay_keeps_original_media_parameters_after_session_ends_and_during_pending() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let Json(first) = execute_native(
            &shared,
            &headers,
            command.clone(),
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| Ok(()),
        )
        .map_err(|e| e.0)
        .unwrap();
        let pending = {
            let mut e = shared.lock().unwrap();
            e.authority
                .set_session_status(
                    first.receipt.session_id.unwrap(),
                    SessionStatus::AdminDisconnected,
                )
                .unwrap();
            let principal = authenticate(&e, &headers).map_err(|error| error.0).unwrap();
            let revision = e.authority.current().revision;
            match e
                .authority
                .prepare_transaction(
                    principal,
                    Command {
                        control_version: 1,
                        expected_config_revision: None,
                        expected_event_sequence: None,
                        runtime_epoch: None,
                        credential_id: None,
                        request_id: Uuid::new_v4(),
                        expected_revision: Some(revision),
                        operation: neonmix_control::Operation::OutputMix {
                            gain_db: Some(-3.),
                            muted: None,
                        },
                    },
                )
                .unwrap()
            {
                Preparation::Prepared(pending) => pending,
                Preparation::Replay(_) => panic!("new request cannot replay"),
            }
        };
        let Json(replayed) = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |_, _, _, _| panic!("no replay media"),
            |_, _| panic!("no replay storage"),
        )
        .map_err(|e| e.0)
        .unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(replayed).unwrap()
        );
        let mut e = shared.lock().unwrap();
        assert_eq!(
            e.authority.current().sessions[&first.receipt.session_id.unwrap()].status,
            SessionStatus::AdminDisconnected
        );
        assert_eq!(e.authority.current().sessions.len(), 1);
        assert!(e.resources.native_reservation.is_none());
        e.authority.abort_transaction(pending.token()).unwrap();
    }
    #[test]
    fn native_start_releases_engine_during_preparation_and_persistence() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let Json(response) = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |receiver, socket, lane, stats| {
                let e = shared
                    .try_lock()
                    .expect("native SDK preparation must not hold Engine");
                let (_, reserved_lane, _, _) = e.resources.native_reservation.unwrap();
                assert_eq!(reserved_lane, lane);
                assert!(e.resources.lanes[lane].producer.is_none());
                drop(e);
                crate::media_worker::MediaWorker::prepare(receiver, socket, lane, stats)
            },
            |_, _| {
                let mut e = shared
                    .try_lock()
                    .expect("persistent write must not hold Engine");
                assert!(e.resources.native_reservation.is_some());
                // Health must remain observable while the durable transaction is staged.
                e.authority.set_output_available(false).unwrap();
                assert!(!e.authority.current().output.available);
                Ok(())
            },
        )
        .map_err(|error| error.0)
        .unwrap();
        let e = shared.lock().unwrap();
        assert!(e.resources.native_reservation.is_none());
        assert!(!e.authority.current().output.available);
        assert_eq!(
            e.authority.current().sessions[&response.receipt.session_id.unwrap()].status,
            SessionStatus::OutputLost
        );
        assert!(response.media_port.is_some());
    }
    #[test]
    fn native_disk_barrier_health_get_and_event_replica_converge_for_commit_and_abort() {
        for commit in [false, true] {
            let (shared, headers, command, _mixer) = shared_fixture();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let Json(mut replica) = runtime
                .block_on(snapshot(State(shared.clone()), headers.clone()))
                .map_err(|e| e.0)
                .unwrap();
            let first = replica.revision;
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let writer = scope.spawn(|| {
                    execute_native(
                        &shared,
                        &headers,
                        command,
                        remote(),
                        crate::media_worker::MediaWorker::prepare,
                        |_, _| {
                            // The actual persistence seam remains pending while GET and
                            // health events run through their product owners.
                            for _ in 0..2 {
                                barrier.wait();
                                barrier.wait();
                            }
                            if commit {
                                Ok(())
                            } else {
                                Err(ControlError::Busy)
                            }
                        },
                    )
                });
                for (index, available) in [false, true].into_iter().enumerate() {
                    barrier.wait();
                    {
                        let mut e = shared.try_lock().expect("fsync cannot hold Engine");
                        assert!(e.authority.transaction_pending());
                        e.authority.set_output_available(available).unwrap();
                    }
                    let Json(get) = runtime
                        .block_on(snapshot(State(shared.clone()), headers.clone()))
                        .map_err(|e| e.0)
                        .unwrap();
                    assert_eq!(get.revision, first + index as u64 + 1);
                    assert_eq!(get.output.available, available);
                    for event in shared
                        .lock()
                        .unwrap()
                        .authority
                        .events_after(replica.revision)
                        .unwrap()
                    {
                        replica.apply_event(event).unwrap();
                    }
                    assert_eq!(
                        serde_json::to_value(&replica).unwrap(),
                        serde_json::to_value(&get).unwrap()
                    );
                    barrier.wait();
                }
                let result = writer.join().unwrap();
                if commit {
                    assert_eq!(
                        result.map_err(|e| e.0).unwrap().0.receipt.revision,
                        first + 3
                    );
                } else {
                    assert!(matches!(result, Err(ApiError(ControlError::Busy))));
                }
            });
            let e = shared.lock().unwrap();
            for event in e.authority.events_after(replica.revision).unwrap() {
                replica.apply_event(event).unwrap();
            }
            assert_eq!(
                serde_json::to_value(replica).unwrap(),
                serde_json::to_value(e.authority.snapshot()).unwrap()
            );
            assert!(e.authority.current().output.available);
            assert!(!e.authority.transaction_pending());
            assert_eq!(e.authority.current().sessions.len(), usize::from(commit));
        }
    }

    #[test]
    fn native_external_preparation_failure_returns_the_reserved_lane_without_writing() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let result = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            |_, _, _, _| {
                assert!(shared.try_lock().is_ok());
                Err(std::io::Error::other("injected SDK failure"))
            },
            |_, _| panic!("failed preparation must not write"),
        );
        assert!(matches!(result, Err(ApiError(ControlError::Busy))));
        let mut e = shared.lock().unwrap();
        assert!(e.authority.current().sessions.is_empty());
        assert!(e.resources.native_reservation.is_none());
        untouched(&mut e.resources);
    }
    #[test]
    fn in_flight_native_start_owns_the_last_multi_source_slot() {
        let (shared, headers, command, _mixer) = shared_fixture();
        {
            let mut e = shared.lock().unwrap();
            e.resources.multi_receiver = true;
            for index in 0..3 {
                let owner = admission::Owner {
                    receiver: Uuid::new_v4(),
                    generation: 1,
                    connection: 1,
                    request: 1,
                    source: format!("existing-{index}"),
                };
                e.resources
                    .admissions
                    .reserve(owner, 0, true, Some(index + 2), &[], Instant::now())
                    .unwrap();
            }
        }
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let native = scope.spawn(|| {
                execute_native(
                    &shared,
                    &headers,
                    command,
                    remote(),
                    |_, _, _, _| {
                        barrier.wait();
                        barrier.wait();
                        Err(std::io::Error::other("release fixture reservation"))
                    },
                    |_, _| panic!("fixture deliberately aborts before disk"),
                )
            });
            barrier.wait();
            {
                let mut e = shared
                    .try_lock()
                    .expect("in-flight native preparation cannot hold Engine");
                let pending = usize::from(e.resources.native_reservation.is_some());
                assert_eq!(pending, 1);
                let owner = admission::Owner {
                    receiver: Uuid::new_v4(),
                    generation: 1,
                    connection: 2,
                    request: 2,
                    source: "racing-airplay".into(),
                };
                assert_eq!(
                    e.resources
                        .admissions
                        .reserve(owner, pending, true, Some(8), &[], Instant::now())
                        .err(),
                    Some("room_capacity_full")
                );
            }
            barrier.wait();
            assert!(native.join().unwrap().is_err());
        });
        let e = shared.lock().unwrap();
        assert!(e.resources.native_reservation.is_none());
        assert_eq!(e.resources.admissions.claims.len(), 3);
    }
    #[test]
    fn native_failed_disk_write_aborts_candidate_and_returns_lane() {
        let (shared, headers, command, _mixer) = shared_fixture();
        let result = execute_native(
            &shared,
            &headers,
            command,
            remote(),
            crate::media_worker::MediaWorker::prepare,
            |_, _| {
                assert!(shared.try_lock().is_ok());
                Err(ControlError::Busy)
            },
        );
        assert!(matches!(result, Err(ApiError(ControlError::Busy))));
        let mut e = shared.lock().unwrap();
        assert!(e.authority.current().sessions.is_empty());
        assert!(e.resources.native_reservation.is_none());
        untouched(&mut e.resources);
    }
    #[test]
    fn native_ids_cannot_collide_with_reserved_airplay_context_before_prepare() {
        let (mut resources, snapshot, _mixer) = fixture();
        let owner = admission::Owner {
            receiver: Uuid::new_v4(),
            generation: 1,
            connection: 1,
            request: 1,
            source: "source".into(),
        };
        let claim = resources
            .admissions
            .reserve(owner, 0, true, Some(2), &[], Instant::now())
            .unwrap();
        assert_eq!(claim.context.session_id, 1);
        let result = resources.prepare_with_factory(
            &snapshot,
            Some(remote()),
            || panic!("collision must not save"),
            |_, _, _, _| panic!("collision must not prepare media"),
        );
        assert_eq!(result, Err(ControlError::Busy));
        untouched(&mut resources);
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
        let mut rollback_began = None;
        assert_eq!(
            resources.prepare_with_commit(
                &snapshot,
                Some(Connection {
                    remote: "127.0.0.1:54321".parse().unwrap(),
                    local: "127.0.0.1:0".parse().unwrap()
                }),
                || {
                    // Native SDK/plugin preparation happens before disk commit;
                    // only the failed commit's rollback has the five-second bound.
                    rollback_began = Some(Instant::now());
                    Err(ControlError::Busy)
                }
            ),
            Err(ControlError::Busy)
        );
        let rollback_began = rollback_began.expect("disk commit stage was not reached");
        eprintln!(
            "native preparation: {:?}; prepared rollback: {:?}",
            rollback_began.duration_since(began),
            rollback_began.elapsed()
        );
        assert!(
            rollback_began.elapsed() < Duration::from_secs(5),
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
    #[test]
    fn subscription_cursor_requires_protocol_epoch_and_rejects_restart_between_get_and_subscribe() {
        let token = "subscription-restart-credential-32-bytes-minimum";
        let a = Authority::new("test".into(), "admin".into(), token).unwrap();
        let get = a.snapshot();
        let cursor = Cursor {
            after: get.event_sequence,
            control_version: Some(neonmix_control::CONTROL_VERSION),
            runtime_epoch: Some(get.runtime_epoch),
        };
        assert!(cursor.read(&a).unwrap().is_empty());
        let restarted = Authority::restore(a.persistent()).unwrap();
        assert_ne!(restarted.current().runtime_epoch, get.runtime_epoch);
        assert!(matches!(
            cursor.read(&restarted),
            Err(ControlError::SnapshotRequired)
        ));
        let old = Cursor {
            after: get.revision,
            control_version: None,
            runtime_epoch: None,
        };
        assert!(matches!(old.read(&a), Err(ControlError::UpgradeRequired)));
        let no_epoch = Cursor {
            after: get.event_sequence,
            control_version: Some(neonmix_control::CONTROL_VERSION),
            runtime_epoch: None,
        };
        assert!(matches!(
            no_epoch.read(&a),
            Err(ControlError::SnapshotRequired)
        ));
        let ahead = Cursor {
            after: get.event_sequence + 1,
            control_version: Some(neonmix_control::CONTROL_VERSION),
            runtime_epoch: Some(get.runtime_epoch),
        };
        assert!(matches!(
            ahead.read(&a),
            Err(ControlError::SnapshotRequired)
        ));
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
                stopping: false,
                pairings: Default::default(),
                pair_window: Instant::now(),
                pair_attempts: 0,
                room_name: "test".into(),
                certificate,
                event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
                output_stats: Default::default(),
                authority,
                native_responses: Default::default(),
                native_recovery: None,
                persistence_denials: Default::default(),
                pairing_recovery: None,
                recovery_results: Default::default(),
                resources: Resources {
                    lanes: inputs
                        .into_iter()
                        .map(|producer| Lane {
                            producer: Some(producer),
                            media: None,
                            session: None,
                            binding_generation: 1,
                        })
                        .collect(),
                    control,
                    stats,
                    origin: Instant::now(),
                    pem,
                    output_epoch: 1,
                    airplay_mix: [None; 4],
                    admissions: Default::default(),
                    multi_receiver: false,
                    pending_multi_capacity: false,
                    native_reservation: None,
                    retiring: Vec::new(),
                },
                airplay: airplay::State::new(
                    std::path::PathBuf::from(".local/tmp/airplay-tests"),
                    "127.0.0.1:0".parse().unwrap(),
                ),
                errors: Vec::new(),
                state_path: None,
            })),
            issuer,
            member,
            mixer,
        )
    }
    #[tokio::test]
    async fn unknown_registration_keeps_one_device_candidate_and_freezes_grant_mutations() {
        let (shared, issuer, _member_token, _mixer) = shared();
        let admin_token = neonmix_identity::secret();
        shared
            .lock()
            .unwrap()
            .authority
            .add_device(
                "recovery admin".into(),
                neonmix_control::Role::Admin,
                &admin_token,
            )
            .unwrap();
        let (invitation, secret) = shared
            .lock()
            .unwrap()
            .pairings
            .open(issuer, 30, Instant::now())
            .unwrap();
        let token = neonmix_identity::secret();
        let request = Request {
            invitation_id: invitation,
            request_id: Uuid::new_v4(),
            name: "new member".into(),
            token_sha256: neonmix_identity::digest(&token),
        };
        let first = crate::pairing_api::complete_with(
            shared.clone(),
            headers(&secret),
            request.clone(),
            |_, _| Err(ControlError::DurabilityUnconfirmed),
        )
        .await;
        assert!(matches!(
            first,
            Err(crate::pairing_api::Error::Control(
                ControlError::DurabilityUnconfirmed
            ))
        ));
        let expected = {
            let engine = shared.lock().unwrap();
            assert!(engine.authority.authenticate(&token).is_err());
            engine
                .pairing_recovery
                .as_ref()
                .unwrap()
                .prepared
                .receipt()
                .device_id
                .unwrap()
        };
        let cancel = crate::pairing_api::cancel(
            State(shared.clone()),
            headers(&admin_token),
            Json(crate::pairing_api::Cancel {
                invitation_id: invitation,
            }),
        )
        .await;
        assert!(matches!(
            cancel,
            Err(ApiError(ControlError::DurabilityUnconfirmed))
        ));
        let second = crate::pairing_api::complete_with(
            shared.clone(),
            headers(&secret),
            request.clone(),
            |_, _| Err(ControlError::Busy),
        )
        .await;
        assert!(matches!(
            second,
            Err(crate::pairing_api::Error::Control(
                ControlError::DurabilityUnconfirmed
            ))
        ));
        assert!(
            shared
                .lock()
                .unwrap()
                .authority
                .authenticate(&token)
                .is_err()
        );
        let Json(done) = crate::pairing_api::complete_with(
            shared.clone(),
            headers(&secret),
            request.clone(),
            |_, _| Ok(()),
        )
        .await
        .map_err(|_| "recovery failed")
        .unwrap();
        assert_eq!(done.device_id, expected);
        let Json(replayed) =
            crate::pairing_api::complete_with(shared.clone(), headers(&secret), request, |_, _| {
                panic!("completed pairing replay saved again")
            })
            .await
            .map_err(|_| "replay failed")
            .unwrap();
        assert_eq!(
            serde_json::to_value(done).unwrap(),
            serde_json::to_value(replayed).unwrap()
        );
        assert!(
            shared
                .lock()
                .unwrap()
                .authority
                .authenticate(&token)
                .is_ok()
        );
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
                        control_version: 1,
                        expected_config_revision: None,
                        expected_event_sequence: None,
                        runtime_epoch: None,
                        credential_id: None,
                        request_id: Uuid::new_v4(),
                        expected_revision: Some(revision),
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
                        control_version: 1,
                        expected_config_revision: None,
                        expected_event_sequence: None,
                        runtime_epoch: None,
                        credential_id: None,
                        request_id: Uuid::new_v4(),
                        expected_revision: Some(revision),
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
    #[tokio::test]
    async fn pairing_save_releases_engine_and_keeps_book_until_concurrent_retry_and_cancel() {
        let (shared, issuer, _, _mixer) = shared();
        let admin_token = neonmix_identity::secret();
        let token = neonmix_identity::secret();
        let (id, grant) = {
            let mut e = shared.lock().unwrap();
            e.authority
                .add_device(
                    "cancel admin".into(),
                    neonmix_control::Role::Admin,
                    &admin_token,
                )
                .unwrap();
            e.pairings.open(issuer, 30, Instant::now()).unwrap()
        };
        let request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "concurrent-paired".into(),
            token_sha256: neonmix_identity::digest(&token),
        };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let writing = shared.clone();
        let first = tokio::spawn(crate::pairing_api::complete_with(
            shared.clone(),
            headers(&grant),
            request.clone(),
            move |path, saved| {
                let e = writing
                    .try_lock()
                    .expect("pairing disk write must not hold Engine");
                assert!(
                    !e.authority
                        .current()
                        .devices
                        .values()
                        .any(|d| d.name == "concurrent-paired")
                );
                drop(e);
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                persist_saved(path, saved)
            },
        ));
        started_rx.await.unwrap();
        {
            let mut e = shared.try_lock().unwrap();
            let revision = e.authority.current().revision;
            assert_eq!(
                e.authority
                    .execute(
                        issuer,
                        Command {
                            control_version: 1,
                            expected_config_revision: None,
                            expected_event_sequence: None,
                            runtime_epoch: None,
                            credential_id: None,
                            request_id: Uuid::new_v4(),
                            expected_revision: Some(revision),
                            operation: neonmix_control::Operation::Revoke {
                                device_id: issuer.device_id()
                            }
                        },
                        |_| panic!("frozen issuer change must not persist")
                    )
                    .unwrap_err(),
                ControlError::Busy
            );
        }
        let mut repeated = Box::pin(crate::pairing_api::complete(
            State(shared.clone()),
            headers(&grant),
            Json(request.clone()),
        ));
        // Enqueue the exact retry before cancellation without relying on task scheduling.
        assert!(matches!(
            futures_util::poll!(repeated.as_mut()),
            std::task::Poll::Pending
        ));
        let cancelled = tokio::spawn(crate::pairing_api::cancel(
            State(shared.clone()),
            headers(&admin_token),
            Json(crate::pairing_api::Cancel { invitation_id: id }),
        ));
        tokio::task::yield_now().await;
        assert!(
            !cancelled.is_finished(),
            "Book changes must wait outside Engine until finish"
        );
        release_tx.send(()).unwrap();
        let Json(first) = first
            .await
            .unwrap()
            .unwrap_or_else(|_| panic!("initial redemption failed"));
        let Json(repeated) = repeated
            .await
            .unwrap_or_else(|_| panic!("concurrent exact retry failed"));
        assert_eq!(first.device_id, repeated.device_id);
        assert_eq!(first.revision, repeated.revision);
        assert!(cancelled.await.unwrap().is_ok());
        let e = shared.lock().unwrap();
        assert_eq!(
            e.authority.authenticate(&token).unwrap().device_id(),
            first.device_id
        );
        assert!(matches!(
            e.pairings.check(&grant, &request, Instant::now()),
            Err(neonmix_identity::pairing::Error::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn cancelled_http_redemption_still_finishes_book_and_durable_authority() {
        let (shared, issuer, _, _mixer) = shared();
        let token = neonmix_identity::secret();
        let (id, grant) = shared
            .lock()
            .unwrap()
            .pairings
            .open(issuer, 30, Instant::now())
            .unwrap();
        let request = Request {
            invitation_id: id,
            request_id: Uuid::new_v4(),
            name: "cancelled-http".into(),
            token_sha256: neonmix_identity::digest(&token),
        };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn(crate::pairing_api::complete_with(
            shared.clone(),
            headers(&grant),
            request.clone(),
            move |path, saved| {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                persist_saved(path, saved)
            },
        ));
        started_rx.await.unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        release_tx.send(()).unwrap();
        // Retry waits for the uncancellable commit owner, then gets its receipt.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            crate::pairing_api::complete(
                State(shared.clone()),
                headers(&grant),
                Json(request.clone()),
            ),
        )
        .await
        .unwrap();
        let Json(done) =
            result.unwrap_or_else(|_| panic!("cancelled caller abandoned its transaction"));
        let e = shared.lock().unwrap();
        assert_eq!(
            e.authority.authenticate(&token).unwrap().device_id(),
            done.device_id
        );
        assert!(matches!(
            e.pairings.check(&grant, &request, Instant::now()),
            Ok(Checked::Repeated(_))
        ));
    }
}
