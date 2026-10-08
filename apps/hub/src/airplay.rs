//! Independent audio worker. The Hub owns its lifetime, admission and selected output.
use super::*;
#[path = "airplay_api.rs"]
pub(super) mod api;
use super::admission::Owner;
use neonmix_airplay_adapter::{
    Context, Ingress,
    control::{AirplayAction, AirplayCommand, PlaybackMode},
};
use neonmix_airplay_ipc::{
    HEADER_BYTES, PcmPacket,
    local::{Listener as MediaListener, Stream as MediaStream},
};
use neonmix_identity::airplay_profile::{self as profile, ReceiverProfile};
use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        atomic::AtomicU64,
        mpsc::{self, Receiver as ChannelReceiver, SyncSender},
    },
    thread::JoinHandle,
};

const MAX_CLIENTS: usize = 64;
const PAIRING_WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Serialize)]
struct Status {
    revision: u64,
    enabled: bool,
    ready: bool,
    discovery_state: String,
    visibility_reason: String,
    active: bool,
    playback_allowed: bool,
    source_id: Option<String>,
    source_name: Option<String>,
    source_revoked: bool,
    playback_mode: PlaybackMode,
    media_resets: u64,
    last_media_reset: Option<String>,
    format: Option<serde_json::Value>,
    failure_stage: Option<String>,
    pairing_window_open: bool,
    pairing_window_remaining_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pairing_pin: Option<String>,
    mix: neonmix_control::Mix,
    error: Option<String>,
    received_blocks: u64,
    rejected_blocks: u64,
    released_blocks: u64,
    queued_blocks: usize,
    max_loop_gap_ns: u64,
    max_control_ns: u64,
    max_pcm_ns: u64,
    ingress: neonmix_airplay_adapter::IngressStats,
    protocol_events: std::collections::VecDeque<serde_json::Value>,
    protocol_events_dropped: u64,
    admission_denials: std::collections::BTreeMap<&'static str, u64>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            revision: 1,
            enabled: false,
            ready: false,
            discovery_state: "hidden".into(),
            visibility_reason: "disabled".into(),
            active: false,
            playback_allowed: true,
            source_id: None,
            source_name: None,
            source_revoked: false,
            playback_mode: PlaybackMode::default(),
            media_resets: 0,
            last_media_reset: None,
            format: None,
            failure_stage: None,
            pairing_window_open: false,
            pairing_window_remaining_seconds: 0,
            pairing_pin: None,
            mix: Default::default(),
            error: None,
            received_blocks: 0,
            rejected_blocks: 0,
            released_blocks: 0,
            queued_blocks: 0,
            max_loop_gap_ns: 0,
            max_control_ns: 0,
            max_pcm_ns: 0,
            ingress: Default::default(),
            protocol_events: Default::default(),
            protocol_events_dropped: 0,
            admission_denials: Default::default(),
        }
    }
}
type Saved = neonmix_identity::profiles::AirPlayProfile<PlaybackMode>;
#[derive(Clone)]
enum Action {
    PlaybackMode(PlaybackMode),
    PairWindow {
        trust_generation: u64,
        pin: String,
        deadline: Instant,
        attempts: u32,
    },
    Stop,
    Disconnect(Option<(Owner, Context)>),
    Reset(Option<(Owner, Context)>),
    Allow,
    Revoke(Option<(Owner, Context)>),
}
pub(super) struct EndpointState {
    receiver_id: Uuid,
    receiver_uuid: Uuid,
    name: String,
    generation: u64,
    lane: Option<usize>,
    session_owner: Option<(Owner, Context)>,
    trust_generation: u64,
    pairing_attempts: u32,
    pairing_in_progress: bool,
    status: Status,
    pairing_deadline: Option<Instant>,
    directory: PathBuf,
    listen: SocketAddr,
    commands: Option<SyncSender<Action>>,
    gate: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    stop_deadline: Arc<Mutex<Option<Instant>>>,
    epoch: Arc<AtomicU64>,
    pub(super) join: Option<JoinHandle<()>>,
}
impl EndpointState {
    pub(super) fn new(directory: PathBuf, listen: SocketAddr) -> Self {
        Self {
            receiver_id: Uuid::new_v4(),
            receiver_uuid: Uuid::nil(),
            name: String::new(),
            generation: 0,
            lane: None,
            session_owner: None,
            trust_generation: 1,
            pairing_attempts: 0,
            pairing_in_progress: false,
            status: Default::default(),
            pairing_deadline: None,
            directory,
            listen,
            commands: None,
            gate: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            stop_deadline: Arc::new(Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(1)),
            join: None,
        }
    }
    fn display_status_at(&self, now: Instant) -> Status {
        let mut status = self.status.clone();
        let remaining = self
            .pairing_deadline
            .filter(|_| {
                status.enabled
                    && status.ready
                    && status.playback_allowed
                    && (self.pairing_attempts < 5 || self.pairing_in_progress)
            })
            .and_then(|deadline| deadline.checked_duration_since(now))
            .filter(|duration| !duration.is_zero());
        status.pairing_window_open = remaining.is_some();
        status.pairing_window_remaining_seconds = remaining
            .map(|duration| duration.as_secs() + u64::from(duration.subsec_nanos() > 0))
            .unwrap_or(0);
        if !status.pairing_window_open {
            status.pairing_pin = None;
        }
        status
    }
    pub(super) fn diagnostic(&self) -> serde_json::Value {
        let mut status = self.display_status_at(Instant::now());
        status.pairing_pin = None;
        status.source_name = None;
        status.source_id = None;
        serde_json::to_value(status).unwrap_or_default()
    }
    pub(super) fn stop(&mut self) {
        if let Ok(mut deadline) = self.stop_deadline.lock() {
            deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
        }
        self.shutdown.store(true, Release);
        self.gate.store(false, Release);
        self.status.active = false;
        self.status.enabled = false;
        self.status.ready = false;
        self.status.pairing_pin = None;

        if let Some(command) = &self.commands {
            let _ = command.try_send(Action::Stop);
        }
    }
    pub(super) fn output_epoch_changed(&mut self) {
        if self.status.active || self.session_owner.is_some() {
            self.gate.store(false, Release);
            self.epoch.fetch_add(1, Relaxed);
            self.status.active = false;
            self.status.revision += 1;
            if let Some(tx) = &self.commands
                && tx
                    .try_send(Action::Reset(self.session_owner.clone()))
                    .is_err()
            {
                self.shutdown.store(true, Release);
            }
        }
    }
    pub(super) fn disconnect(&mut self) {
        if self.commands.is_none() {
            return;
        }
        self.gate.store(false, Release);
        self.epoch.fetch_add(1, Relaxed);
        self.status.active = false;
        self.status.playback_allowed = false;
        self.status.revision = self.status.revision.saturating_add(1);
        if let Some(command) = &self.commands
            && command
                .try_send(Action::Disconnect(self.session_owner.clone()))
                .is_err()
        {
            self.shutdown.store(true, Release);
        }
    }
}
struct PairingLease {
    generation: u64,
    trust_generation: u64,
    connection: u64,
    request: u64,
    deadline: Instant,
}
pub(super) struct State {
    stopping: bool,
    pub(super) worker_forced: Arc<AtomicBool>,
    pub(super) cleanup_failures: Arc<AtomicU64>,
    pub(super) shutdown_failures: Arc<AtomicU64>,
    receivers: Vec<EndpointState>,
    pairing_leases: std::collections::BTreeMap<Uuid, PairingLease>,
    multi_receiver: bool,
    pub(super) mixer_dirty: bool,
    configuration_pending: bool,
    recovery: Option<api::Recovery>,
    profile_recovery: Option<api::ProfileRecovery>,
    profile: Arc<Mutex<Option<ReceiverProfile>>>,
    revision: u64,
    published_business: Option<(u64, serde_json::Value)>,
    sequence_exhausted: bool,
    receipts: std::collections::VecDeque<(String, String, serde_json::Value)>,
    pub(super) output_latency_ns: Arc<AtomicU64>,
}
impl State {
    pub(super) fn new(directory: PathBuf, listen: SocketAddr) -> Self {
        Self {
            stopping: false,
            worker_forced: Arc::new(AtomicBool::new(false)),
            cleanup_failures: Arc::new(AtomicU64::new(0)),
            shutdown_failures: Arc::new(AtomicU64::new(0)),
            receivers: vec![EndpointState::new(directory, listen)],
            pairing_leases: Default::default(),
            multi_receiver: false,
            mixer_dirty: false,
            configuration_pending: false,
            recovery: None,
            profile_recovery: None,
            profile: Arc::new(Mutex::new(None)),
            revision: 1,
            published_business: None,
            sequence_exhausted: false,
            receipts: Default::default(),
            output_latency_ns: Arc::new(AtomicU64::new(0)),
        }
    }
    fn advance_event(&mut self) {
        if let Some(sequence) = self.revision.checked_add(1) {
            self.revision = sequence;
        } else {
            self.sequence_exhausted = true;
        }
    }
    pub(super) fn stop(&mut self) {
        if !self.stopping {
            self.worker_forced.store(false, Release);
            self.stopping = true;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        for receiver in &mut self.receivers {
            if let Ok(mut stored) = receiver.stop_deadline.lock() {
                stored.get_or_insert(deadline);
            }
            receiver.stop();
        }
    }
    pub(super) fn output_epoch_changed(&mut self) {
        for receiver in &mut self.receivers {
            receiver.output_epoch_changed();
        }
    }
    pub(super) fn take_threads(&mut self) -> Vec<JoinHandle<()>> {
        self.receivers
            .iter_mut()
            .filter_map(|r| r.join.take())
            .collect()
    }
    pub(super) fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({"revision":self.revision,"multi_receiver":self.multi_receiver,
            "receivers":self.receivers.iter().map(EndpointState::diagnostic).collect::<Vec<_>>()})
    }
}

pub(super) async fn snapshot(
    axum::extract::State(shared): axum::extract::State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let receiver = 0;
    let profiles = shared
        .lock()
        .map_err(|_| ControlError::Busy)?
        .airplay
        .profile
        .clone();
    let profiles = profiles.lock().map_err(|_| ControlError::Busy)?;
    let engine = shared.lock().map_err(|_| ControlError::Busy)?;
    let principal = authenticate(&engine, &headers)?;
    if engine.airplay.multi_receiver {
        return Err(ApiError(ControlError::UpgradeRequired));
    }
    let mut status = engine.airplay.receivers[receiver].display_status_at(Instant::now());
    status.revision = engine.airplay.revision;
    if let Some(source) = profiles.as_ref().and_then(|p| {
        p.sources
            .iter()
            .find(|s| Some(&s.source_id) == status.source_id.as_ref())
    }) {
        status.playback_allowed = !source.blocked && !source.revoked;
        status.source_revoked = source.revoked;
    }
    if engine
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_none_or(|d| d.role != neonmix_control::Role::Admin)
    {
        status.pairing_pin = None;
    }
    Ok(Json(
        serde_json::to_value(status).map_err(|_| ControlError::Busy)?,
    ))
}
pub(super) async fn command(
    axum::extract::State(shared): axum::extract::State<Shared>,
    headers: HeaderMap,
    Json(command): Json<AirplayCommand>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    use neonmix_airplay_adapter::control::{AirplayActionV2 as V2, AirplayCommandV2};
    {
        let e = shared.lock().map_err(|_| ControlError::Busy)?;
        let principal = authenticate(&e, &headers)?;
        if e.authority
            .current()
            .devices
            .get(&principal.device_id())
            .is_none_or(|d| d.role != neonmix_control::Role::Admin)
        {
            return Err(ControlError::PermissionDenied.into());
        }
        if e.airplay.multi_receiver {
            return Err(ControlError::UpgradeRequired.into());
        }
        if e.airplay.configuration_pending {
            return Err(ControlError::Busy.into());
        }
    }
    ensure_profile(&shared)?;
    let v2 = {
        let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
        let receiver = &e.airplay.receivers[0];
        if e.airplay.revision != command.expected_revision {
            return Err(ControlError::RevisionConflict.into());
        }
        // The old receiver-default setting remains available without a source.
        if let AirplayAction::PlaybackMode { mode } = command.operation {
            if !e.resources.control.has_capacity() {
                return Err(ControlError::Busy.into());
            }
            apply_action(&mut e, &shared, 0, AirplayAction::PlaybackMode { mode })?;
            e.airplay.receivers[0].status.revision += 1;
            e.airplay.advance_event();
            let mut status = e.airplay.receivers[0].display_status_at(Instant::now());
            status.revision = e.airplay.revision;
            return Ok(Json(
                serde_json::to_value(status).map_err(|_| ControlError::Busy)?,
            ));
        }
        let source = receiver.status.source_id.clone();
        let session = e
            .resources
            .admissions
            .claims
            .get(&receiver.receiver_id)
            .map(|c| c.context.session_id);
        let operation = match command.operation {
            AirplayAction::Enable => V2::Enable,
            AirplayAction::Disable => V2::Disable,
            AirplayAction::Disconnect => V2::DisconnectSource {
                source_id: source.ok_or(ControlError::NotFound)?,
                session_id: session.ok_or(ControlError::NotFound)?,
            },
            AirplayAction::Revoke => V2::RevokeSource {
                source_id: source.ok_or(ControlError::NotFound)?,
                session_id: session,
            },
            AirplayAction::Allow => V2::AllowSource {
                source_id: source.ok_or(ControlError::NotFound)?,
            },
            AirplayAction::Mix {
                gain_db,
                muted,
                solo,
            } => V2::MixSource {
                source_id: source.ok_or(ControlError::NotFound)?,
                session_id: session.ok_or(ControlError::NotFound)?,
                gain_db,
                muted,
                solo,
            },
            AirplayAction::PlaybackMode { .. } => unreachable!(),
        };
        AirplayCommandV2 {
            command_version: 2,
            expected_config_revision: None,
            expected_event_sequence: None,
            runtime_epoch: None,
            credential_id: None,
            command_id: Uuid::new_v4().to_string(),
            expected_revision: Some(e.airplay.revision),
            operation,
        }
    };
    let _ = api::command(
        axum::extract::State(shared.clone()),
        headers.clone(),
        Json(v2),
    )
    .await
    .map_err(|error| {
        ApiError(match error.0 {
            "stale_revision" => ControlError::RevisionConflict,
            "session_changed" | "source_unknown" => ControlError::NotFound,
            "pairing_revoked" => ControlError::PlaybackBlocked,
            "room_capacity_full" => ControlError::QuotaExceeded,
            "receiver_busy" => ControlError::AlreadyActive,
            "permission_denied" => ControlError::PermissionDenied,
            _ => ControlError::Busy,
        })
    })?;
    snapshot(axum::extract::State(shared), headers).await
}
fn apply_action(
    engine: &mut Engine,
    shared: &Shared,
    receiver: usize,
    operation: AirplayAction,
) -> std::result::Result<(), ApiError> {
    if matches!(
        operation,
        AirplayAction::Disable | AirplayAction::Disconnect | AirplayAction::Revoke
    ) && let Some(lane) = engine.airplay.receivers[receiver].lane
    {
        engine.resources.control.revoke_lane(lane);
    }
    match operation {
        AirplayAction::PlaybackMode { mode } => {
            if engine.airplay.receivers[receiver].status.active {
                return Err(ControlError::AlreadyActive.into());
            }
            let tx = engine.airplay.receivers[receiver]
                .commands
                .as_ref()
                .ok_or(ControlError::PlaybackBlocked)?;
            if !engine.airplay.receivers[receiver].status.ready {
                return Err(ControlError::Busy.into());
            }
            tx.try_send(Action::PlaybackMode(mode))
                .map_err(|_| ControlError::Busy)?;
            engine.airplay.receivers[receiver].status.playback_mode = mode;
        }
        AirplayAction::Enable => {
            if engine.stopping || engine.airplay.stopping || engine.airplay.configuration_pending {
                return Err(ControlError::Busy.into());
            }
            if engine.airplay.receivers[receiver].commands.is_some() {
                return Err(ControlError::AlreadyActive.into());
            }
            if !engine.authority.current().output.available {
                return Err(ControlError::PlaybackBlocked.into());
            }
            engine.airplay.receivers[receiver].generation += 1;
            engine.airplay.receivers[receiver]
                .shutdown
                .store(false, Release);
            if let Ok(mut deadline) = engine.airplay.receivers[receiver].stop_deadline.lock() {
                *deadline = None;
            }
            let (tx, rx) = mpsc::sync_channel(8);
            let worker_shared = shared.clone();
            engine.airplay.receivers[receiver].status.enabled = true;
            engine.airplay.receivers[receiver].status.error = None;
            engine.airplay.receivers[receiver].status.failure_stage = None;
            engine.airplay.receivers[receiver]
                .status
                .protocol_events
                .clear();
            engine.airplay.receivers[receiver]
                .status
                .protocol_events_dropped = 0;
            engine.airplay.receivers[receiver].status.format = None;
            engine.airplay.receivers[receiver].commands = Some(tx);
            let handle = std::thread::spawn(move || run(worker_shared, receiver, rx));
            engine.airplay.receivers[receiver].join = Some(handle);
        }
        AirplayAction::Disable => {
            engine.airplay.receivers[receiver].stop();
            engine.resources.airplay_mix[receiver] = None;
        }
        AirplayAction::Disconnect => {
            engine.airplay.receivers[receiver].disconnect();
            engine.resources.airplay_mix[receiver] = None;
        }
        AirplayAction::Allow => {
            if let Some(tx) = &engine.airplay.receivers[receiver].commands {
                tx.try_send(Action::Allow).map_err(|_| ControlError::Busy)?;
            }
            engine.airplay.receivers[receiver].status.playback_allowed = true;
        }
        AirplayAction::Revoke => {
            engine.airplay.receivers[receiver]
                .gate
                .store(false, Release);
            engine.airplay.receivers[receiver]
                .epoch
                .fetch_add(1, Relaxed);
            engine.airplay.receivers[receiver].status.active = false;
            engine.airplay.receivers[receiver].status.playback_allowed = false;
            engine.airplay.receivers[receiver].status.source_revoked = true;
            engine.resources.airplay_mix[receiver] = None;
            if let Some(tx) = &engine.airplay.receivers[receiver].commands
                && tx
                    .try_send(Action::Revoke(
                        engine.airplay.receivers[receiver].session_owner.clone(),
                    ))
                    .is_err()
            {
                engine.airplay.receivers[receiver]
                    .shutdown
                    .store(true, Release);
                return Err(ControlError::Busy.into());
            }
        }
        AirplayAction::Mix {
            gain_db,
            muted,
            solo,
        } => {
            if !gain_db.is_finite() || !(-96.0..=12.0).contains(&gain_db) {
                return Err(ControlError::InvalidArgument.into());
            }
            engine.airplay.receivers[receiver].status.mix = neonmix_control::Mix {
                gain_db,
                muted,
                solo,
            };
            if let Some((_, mix)) = &mut engine.resources.airplay_mix[receiver] {
                mix.gain_db = gain_db;
                mix.muted = muted;
                mix.solo = solo;
            }
        }
    }
    Ok(())
}
fn update(shared: &Shared, apply: impl FnOnce(&mut Engine)) {
    if let Ok(mut engine) = shared.lock() {
        let before = engine
            .airplay
            .receivers
            .iter()
            .map(|r| r.status.revision)
            .collect::<Vec<_>>();
        apply(&mut engine);
        if engine
            .airplay
            .receivers
            .iter()
            .map(|r| r.status.revision)
            .ne(before)
        {
            engine.airplay.advance_event();
        }
    }
}
fn send(tx: &SyncSender<serde_json::Value>, value: serde_json::Value) -> crate::Result<()> {
    tx.try_send(value)
        .map_err(|_| "airplay_control_busy".into())
}
fn save_receiver(
    path: &std::path::Path,
    profiles: &Arc<Mutex<Option<ReceiverProfile>>>,
    receiver_id: Uuid,
    saved: &Saved,
    shared: &Shared,
) -> crate::Result<()> {
    let mut guard = profiles.lock().map_err(|_| "airplay_profile_busy")?;
    if shared
        .lock()
        .map_err(|_| "airplay_state_busy")?
        .airplay
        .configuration_pending
    {
        return Err("profile_durability_unconfirmed".into());
    }
    let mut next = guard.as_ref().ok_or("airplay_profile_missing")?.clone();
    let receiver = next
        .receivers
        .iter_mut()
        .find(|r| r.receiver_id == receiver_id)
        .ok_or("receiver_unknown")?;
    receiver.default_playback_mode =
        serde_json::from_value(serde_json::to_value(saved.playback_mode)?)?;
    api::persist_background(
        shared,
        path,
        guard.as_ref().unwrap(),
        &mut next,
        profile::save,
    )?;
    *guard = Some(next);
    Ok(())
}
fn trusted(profile: &ReceiverProfile, receiver: Uuid, source: &str) -> bool {
    profile.playback_allowed
        && profile
            .sources
            .iter()
            .any(|s| s.source_id == source && !s.blocked && !s.revoked)
        && profile
            .bindings
            .iter()
            .any(|b| b.receiver_id == receiver && b.source_id == source && !b.revoked)
}
// Admission shares the durable profile with status reads and file transactions.
// A busy mutex is not an authorization decision. Retry on the media-owner loop,
// leaving time for the reply before the worker's 500 ms admission deadline.
fn event_lock<'a, T>(
    state: &'a Mutex<T>,
    kind: &str,
    received_at: Instant,
    now: Instant,
) -> std::result::Result<Option<std::sync::MutexGuard<'a, T>>, &'static str> {
    if kind == "admit_request"
        && now.saturating_duration_since(received_at) >= Duration::from_millis(400)
    {
        return Err("admission_timeout");
    }
    match state.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(std::sync::TryLockError::WouldBlock) => Ok(None),
        Err(_) => Err("airplay_profile_busy"),
    }
}

fn deny_admission(
    denials: &mut std::collections::BTreeMap<&'static str, u64>,
    commands: &SyncSender<serde_json::Value>,
    generation: u64,
    event: &serde_json::Value,
    reason: &'static str,
) -> crate::Result<()> {
    let connection = event["connection_id"]
        .as_u64()
        .ok_or("admit_connection_id")?;
    let request = event["request_id"].as_u64().ok_or("admit_request_id")?;
    send(
        commands,
        serde_json::json!({"type":"admit","worker_generation":generation,
        "connection_id":connection,"request_id":request,"allowed":false}),
    )?;
    admission_denied(denials, reason);
    Ok(())
}

fn admission_denied(
    denials: &mut std::collections::BTreeMap<&'static str, u64>,
    reason: &'static str,
) {
    let count = denials.entry(reason).or_default();
    *count = count.saturating_add(1);
}
fn release_claim(engine: &mut Engine, receiver: usize, producer: &mut Option<BlockProducer>) {
    let endpoint = &mut engine.airplay.receivers[receiver];
    endpoint.gate.store(false, Release);
    if let Some(lane) = endpoint.lane {
        engine.resources.control.revoke_lane(lane);
    }
    if let Some(p) = producer.take() {
        p.invalidate();
        if let Some(lane) = endpoint.lane.take() {
            engine.resources.lanes[lane].producer = Some(p);
        }
    }
    if let Some(claim) = engine
        .resources
        .admissions
        .claims
        .get(&endpoint.receiver_id)
        .cloned()
    {
        engine.resources.admissions.release(&claim.owner);
    }
    engine.resources.airplay_mix[receiver] = None;
    engine.airplay.mixer_dirty = true;
    endpoint.status.active = false;
    endpoint.session_owner = None;
    endpoint.status.revision += 1;
    endpoint.status.mix.solo = false;
}
/// Restore capacity before accepting native Start commands after a Hub restart.
/// Legacy single-receiver metadata is migrated only on explicit receiver use.
pub(super) fn load_existing(shared: &Shared) -> crate::Result<()> {
    let path = shared
        .lock()
        .map_err(|_| "airplay_state_busy")?
        .airplay
        .receivers[0]
        .directory
        .join("receiver.json");
    if !path.try_exists()? {
        return Ok(());
    }
    match neonmix_identity::profiles::load_profile_metadata(&path)? {
        neonmix_identity::profiles::ProductProfile::AirPlayV2(_) => {
            ensure_profile(shared).map_err(|_| "airplay_profile_unavailable".into())
        }
        neonmix_identity::profiles::ProductProfile::AirPlay(_) => Ok(()),
        _ => Err("airplay_profile_kind".into()),
    }
}
fn ensure_profile(shared: &Shared) -> std::result::Result<(), ApiError> {
    let (profiles, path, hub, name, listen) = {
        let e = shared.lock().map_err(|_| ControlError::Busy)?;
        (
            e.airplay.profile.clone(),
            e.airplay.receivers[0].directory.join("receiver.json"),
            e.authority.current().hub_id,
            speaker_name(&e.room_name),
            e.airplay.receivers[0].listen,
        )
    };
    let mut guard = profiles.lock().map_err(|_| ControlError::Busy)?;
    if guard.is_some() {
        return Ok(());
    }
    neonmix_identity::files::private_dir(path.parent().ok_or(ControlError::Busy)?)
        .map_err(|_| ControlError::Busy)?;
    let loaded = if path.exists() {
        profile::migrate_v1(&path, hub, &name)
    } else {
        profile::create(&path, hub, &name)
    }
    .map_err(|_| ControlError::Busy)?;
    let mut engine = shared.lock().map_err(|_| ControlError::Busy)?;
    engine.airplay.receivers = loaded
        .receivers
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let dir = if i == 0 {
                path.parent().unwrap().to_owned()
            } else {
                path.parent().unwrap().join(r.receiver_id.to_string())
            };
            let mut endpoint = EndpointState::new(dir, listen);
            endpoint.receiver_id = r.receiver_id;
            endpoint.receiver_uuid = r.receiver_uuid;
            endpoint.name = r.name.clone();
            endpoint.status.playback_allowed = loaded.playback_allowed;
            endpoint.status.playback_mode =
                serde_json::from_value(serde_json::to_value(r.default_playback_mode).unwrap())
                    .unwrap();
            endpoint
        })
        .collect();
    engine.airplay.multi_receiver = loaded.multi_receiver;
    engine.resources.multi_receiver = loaded.multi_receiver;
    *guard = Some(loaded);
    Ok(())
}
// Discovery has a separate owner: unregister confirmation must never stall PCM.
struct DiscoveryTask {
    desired: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl DiscoveryTask {
    fn new(
        shared: Shared,
        receiver: usize,
        mut publisher: neonmix_identity::speaker::SpeakerPublisher,
    ) -> Self {
        let desired = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let target = desired.clone();
        let stopped = stop.clone();
        let join = std::thread::spawn(move || {
            let mut applied = u64::MAX;
            let mut failures = 0;
            let mut retry = Instant::now();
            while !stopped.load(Acquire) {
                let generation = target.load(Acquire);
                let visible = generation & 1 == 1;
                if generation != applied && Instant::now() >= retry {
                    update(&shared, |e| {
                        e.airplay.receivers[receiver].status.discovery_state =
                            if visible { "publishing" } else { "withdrawing" }.into();
                    });
                    let result = if visible {
                        publisher.publish()
                    } else {
                        publisher.withdraw()
                    };
                    if target.load(Acquire) == generation {
                        update(&shared, |e| {
                            let status = &mut e.airplay.receivers[receiver].status;
                            status.discovery_state = if result.is_err() {
                                "error"
                            } else if visible {
                                "published"
                            } else {
                                "hidden"
                            }
                            .into();
                            status.visibility_reason = if status.active {
                                "occupied"
                            } else if visible {
                                "available"
                            } else {
                                "capacity_or_output"
                            }
                            .into();
                        });
                    }
                    if result.is_ok() {
                        applied = generation;
                        failures = 0;
                    } else {
                        failures += 1;
                        retry = Instant::now() + Duration::from_secs(1);
                        if failures >= 3 {
                            applied = generation;
                            failures = 0;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let withdrawn = publisher.withdraw().is_ok();
            update(&shared, |e| {
                let status = &mut e.airplay.receivers[receiver].status;
                status.discovery_state = if withdrawn { "hidden" } else { "error" }.into();
                status.visibility_reason = "disabled_or_worker_stopped".into();
            });
        });
        Self {
            desired,
            stop,
            join: Some(join),
        }
    }
    fn set(&self, visible: bool) {
        let current = self.desired.load(Relaxed);
        if current & 1 != u64::from(visible) {
            self.desired
                .store(((current >> 1) + 1) * 2 + u64::from(visible), Release);
        }
    }
}
impl Drop for DiscoveryTask {
    fn drop(&mut self) {
        self.stop.store(true, Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
struct ChildGuard(Child, bool);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
struct FileGuard {
    path: Option<PathBuf>,
    failures: Arc<AtomicU64>,
}
impl FileGuard {
    fn cleanup(&mut self) -> bool {
        if let Some(path) = self.path.take() {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    self.failures.fetch_add(1, Relaxed);
                    return false;
                }
            }
        }
        true
    }
}
impl Drop for FileGuard {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn run(shared: Shared, receiver: usize, actions: ChannelReceiver<Action>) {
    let mut producer = None;
    let result = run_worker(&shared, receiver, &mut producer, actions);
    update(&shared, |engine| {
        engine.airplay.receivers[receiver]
            .gate
            .store(false, Release);
        let receiver_id = engine.airplay.receivers[receiver].receiver_id;
        engine.airplay.pairing_leases.remove(&receiver_id);
        engine.airplay.receivers[receiver].pairing_in_progress = false;
        engine.airplay.receivers[receiver].commands = None;
        engine.airplay.receivers[receiver].status.enabled = false;
        engine.airplay.receivers[receiver].status.ready = false;
        engine.airplay.receivers[receiver].status.active = false;
        engine.airplay.receivers[receiver].status.pairing_pin = None;
        if let Err(error) = &result {
            if engine.stopping {
                engine.airplay.shutdown_failures.fetch_add(1, Relaxed);
            }
            engine.airplay.receivers[receiver].status.error =
                Some(public_worker_error(&error.to_string()).into());
            engine.airplay.receivers[receiver].status.failure_stage = result
                .as_ref()
                .err()
                .map(|error| failure_stage(&error.to_string()).to_string());
        }
        engine.airplay.receivers[receiver].status.revision += 1;
        engine.resources.airplay_mix[receiver] = None;
        release_claim(engine, receiver, &mut producer);
        let state = engine.authority.snapshot();
        let _ = engine.resources.prepare(&state, None);
    });
}
fn run_worker(
    shared: &Shared,
    receiver: usize,
    producer: &mut Option<BlockProducer>,
    actions: ChannelReceiver<Action>,
) -> crate::Result<()> {
    let (
        directory,
        listen,
        hub,
        name,
        origin,
        gate,
        epoch,
        output_latency,
        shutdown,
        receiver_id,
        generation,
        profiles,
        state_path,
    ) = {
        let e = shared.lock().map_err(|_| "airplay_state_busy")?;
        (
            e.airplay.receivers[receiver].directory.clone(),
            e.airplay.receivers[receiver].listen,
            e.airplay.receivers[receiver].receiver_uuid,
            e.airplay.receivers[receiver].name.clone(),
            e.resources.origin,
            e.airplay.receivers[receiver].gate.clone(),
            e.airplay.receivers[receiver].epoch.clone(),
            e.airplay.output_latency_ns.clone(),
            e.airplay.receivers[receiver].shutdown.clone(),
            e.airplay.receivers[receiver].receiver_id,
            e.airplay.receivers[receiver].generation,
            e.airplay.profile.clone(),
            e.airplay.receivers[0].directory.join("receiver.json"),
        )
    };
    neonmix_identity::files::private_dir(&directory)?;
    // The Hub state owner lock is held. Clean only our UUID runtime filenames.
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .and_then(|n| n.strip_prefix("runtime-key-"))
            .is_some_and(|n| {
                Uuid::parse_str(n)
                    .ok()
                    .is_some_and(|id| id.to_string() == n)
            })
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    let mut saved: Saved = {
        let profiles = profiles.lock().map_err(|_| "airplay_profile_busy")?;
        serde_json::from_value(serde_json::to_value(profile::export_v1(
            profiles.as_ref().ok_or("airplay_profile_missing")?,
            receiver_id,
        )?)?)?
    };
    let store = neonmix_identity::store::FileCredentialStore::for_profile(&state_path)?;
    update(shared, |e| {
        e.airplay.receivers[receiver].status.playback_mode = saved.playback_mode
    });
    if saved.known_keys.len() + saved.blocked_keys.len() > MAX_CLIENTS {
        return Err("airplay_trust_capacity".into());
    }
    let keyfile = directory.join(format!("runtime-key-{}", Uuid::new_v4()));
    let (cleanup_failures, worker_forced, stop_deadline) = {
        let e = shared.lock().map_err(|_| "airplay_state_busy")?;
        (
            e.airplay.cleanup_failures.clone(),
            e.airplay.worker_forced.clone(),
            e.airplay.receivers[receiver].stop_deadline.clone(),
        )
    };
    let mut key_guard = FileGuard {
        path: Some(keyfile.clone()),
        failures: cleanup_failures,
    };
    let reference = saved.key_reference.as_ref().ok_or("credential_missing")?;
    let key = store
        .get(
            reference,
            neonmix_identity::store::SecretKind::AirplayReceiverKey,
        )?
        .expose()
        .to_owned();
    let parsed_key = rcgen::KeyPair::from_pem(&key).map_err(|_| "credential_corrupt")?;
    if parsed_key.algorithm() != &rcgen::PKCS_ED25519 {
        return Err("credential_kind_mismatch".into());
    }
    neonmix_identity::files::write_new(&keyfile, key.as_bytes())?;
    let media_root = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| directory.clone());
    let media_root = if media_root.is_absolute() {
        media_root
    } else {
        std::env::current_dir()?.join(media_root)
    };
    let nonce = Uuid::new_v4().simple().to_string();
    let mut listener = MediaListener::bind(&media_root, &nonce[..16])?;
    let token = neonmix_identity::secret();
    let context = Context {
        session_id: ((Uuid::new_v4().as_u128() as u64) & ((1u64 << 53) - 1)).max(1),
        stream_id: ((Uuid::new_v4().as_u128() as u64) & ((1u64 << 53) - 1)).max(1),
        stream_epoch: epoch.load(Relaxed),
        format_epoch: 1,
        mapping_id: 1,
    };
    let pin = format!("{:04}", Uuid::new_v4().as_u128() % 10000);
    let sibling = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "neonmix-airplay-worker.exe"
    } else {
        "neonmix-airplay-worker"
    });
    let runtime = std::env::var_os("NEONMIX_AIRPLAY_RUNTIME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            sibling
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("airplay")
        });
    // A package may keep the worker and its own DLL set under the runtime
    // directory so they cannot collide with the Hub's libraries.
    let private = runtime
        .join("bin")
        .join(sibling.file_name().unwrap_or_default());
    let executable = if private.is_file() { private } else { sibling };
    let mut process = std::process::Command::new(executable);
    process
        .env("GST_PLUGIN_SYSTEM_PATH_1_0", runtime.join("plugins"))
        .env("GST_PLUGIN_PATH_1_0", "")
        .env("GST_PLUGIN_PATH", "")
        .env("GST_REGISTRY", directory.join("gstreamer-registry.bin"));
    let mut child = ChildGuard(
        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
        true,
    );
    let mut input = child.0.stdin.take().ok_or("worker_stdin")?;
    let output = child.0.stdout.take().ok_or("worker_stdout")?;
    let (pairing_deadline, pairing_attempts, mut trust_generation) = {
        let mut e = shared.lock().map_err(|_| "airplay_state_busy")?;
        let receiver = &mut e.airplay.receivers[receiver];
        let deadline = *receiver
            .pairing_deadline
            .get_or_insert_with(|| Instant::now() + PAIRING_WINDOW);
        (
            deadline,
            receiver.pairing_attempts,
            receiver.trust_generation,
        )
    };
    let pairing_remaining_ms = pairing_deadline
        .saturating_duration_since(Instant::now())
        .as_millis() as u64;
    let startup = serde_json::json!({"control_version":2,"session_control_version":1,"pcm_version":2,"worker_generation":generation,"trust_generation":trust_generation,"pairing_remaining_ms":pairing_remaining_ms,"pairing_attempts":pairing_attempts,"media_address":listener.endpoint()?,"ipc_token":token,
        "device_id":neonmix_identity::speaker::receiver_id(hub),"receiver_uuid":hub,"keyfile":keyfile,"name":name,"pin":pin,"rtsp_port":0,
        "session_id":context.session_id,"stream_id":context.stream_id,"stream_epoch":context.stream_epoch,
        "format_epoch":context.format_epoch,"mapping_id":context.mapping_id,
        "protocol_trace":std::env::var("NEONMIX_AIRPLAY_TRACE").is_ok_and(|v|v=="1"),
        "known_client_keys":saved.known_keys,"blocked_client_keys":saved.blocked_keys,"pairing_allowed":saved.playback_allowed});
    // Start the UI deadline before the worker receives configuration. It may
    // hide a code slightly early, but can never extend protocol authorization.

    neonmix_lifecycle::prepare_writer(&input)?;
    let control_stop = Arc::new(AtomicBool::new(false));
    write_worker_control(&mut input, &startup, &control_stop)?;
    let (command_tx, command_rx) = mpsc::sync_channel::<serde_json::Value>(8);
    let writer_stop = control_stop.clone();
    let writer = std::thread::spawn(move || {
        loop {
            if writer_stop.load(Acquire) {
                let line = b"{\"type\":\"stop\"}\n";
                let _ = neonmix_lifecycle::write_bounded(
                    &mut input,
                    line,
                    &AtomicBool::new(false),
                    Duration::from_millis(250),
                );
                break;
            }
            match command_rx.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => {
                    if write_worker_control(&mut input, &command, &writer_stop).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // Closing stdin requests stop if a partial write or full queue blocked it.
    });
    let (event_tx, event_rx) = mpsc::sync_channel::<(serde_json::Value, Instant)>(16);
    let reader_events = event_tx.clone();
    let trace_dropped = Arc::new(AtomicU64::new(0));
    let reader_dropped = trace_dropped.clone();
    let terminal_failure = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let reader_failure = terminal_failure.clone();
    let reader_stop = control_stop.clone();
    let reader = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(output);
        loop {
            let value = match neonmix_airplay_ipc::read_control::<serde_json::Value>(&mut reader) {
                Ok(Some(value)) => value,
                Ok(None) => break,
                Err(_) => {
                    reader_failure.store(5, Release);
                    break;
                }
            };
            if value["type"].as_str() == Some("fatal") {
                reader_failure.store(fatal_code(value["message"].as_str().unwrap_or("")), Release);
            }
            if reader_stop.load(Acquire) {
                continue;
            }
            if let Err(error) = reader_events.try_send((value, Instant::now())) {
                if let mpsc::TrySendError::Full((value, _)) = error
                    && matches!(
                        value["type"].as_str(),
                        Some("protocol" | "protocol_detail" | "protocol_transport")
                    )
                {
                    reader_dropped.fetch_add(1, Relaxed);
                    continue;
                }
                reader_failure.store(6, Release);
                // Keep stdout drained until child exit even on event backpressure.
                continue;
            }
        }
    });
    let (pcm_tx, pcm_rx) = mpsc::sync_channel::<PcmPacket>(8);
    let mut media_thread = None;
    let media_failure = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let media_stop = Arc::new(AtomicBool::new(false));
    let mut ingress = Ingress::new(origin, context);
    ingress.set_playback_mode(saved.playback_mode);
    let mut publisher: Option<DiscoveryTask> = None;
    let mut receiver_ready = false;
    let mut pending_pair_window: Option<(u64, String, Instant)> = None;

    let mut admitted = false;
    let mut owner: Option<Owner> = None;
    let mut closing_owner: Option<(Owner, Context, Instant)> = None;
    let mut awaiting_grant = false;
    let mut grant_deadline: Option<Instant> = None;
    let mut deferred_event: Option<(serde_json::Value, Instant)> = None;
    let mut admission_denials = std::collections::BTreeMap::new();
    let mut admit_epoch = 0;
    let started = Instant::now();
    let mut previous_loop = started;
    let mut max_loop_gap_ns = 0;
    let mut max_control_ns = 0;
    let mut max_pcm_ns = 0;
    let mut active_context = context;
    let (mut received_blocks, mut rejected_blocks, mut released_blocks) = {
        let e = shared.lock().map_err(|_| "airplay_state_busy")?;
        (
            e.airplay.receivers[receiver].status.received_blocks,
            e.airplay.receivers[receiver].status.rejected_blocks,
            e.airplay.receivers[receiver].status.released_blocks,
        )
    };
    let result = (|| -> crate::Result<()> {
        loop {
            let loop_started = Instant::now();
            max_loop_gap_ns = max_loop_gap_ns.max(
                loop_started
                    .duration_since(previous_loop)
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64,
            );
            previous_loop = loop_started;
            let media_finished = media_thread
                .as_ref()
                .is_some_and(|t: &JoinHandle<()>| t.is_finished());
            let child_exit = child.0.try_wait()?;
            if writer.is_finished()
                || reader.is_finished()
                || media_finished
                || child_exit.is_some()
            {
                // stdout and PCM are independent pipes. Preserve a terminal
                // worker reason before classifying the accompanying EOF.
                if terminal_failure.load(Acquire) == 0 {
                    std::thread::sleep(Duration::from_millis(2));
                }
                let code = terminal_failure.load(Acquire);
                // Closing a rejected/timed-out reader also makes worker send
                // fail. Preserve the specific Hub cause instead of its EPIPE.
                if code == 0
                    && let Some(reason) = media_failure_reason(media_failure.load(Acquire))
                {
                    return Err(reason.into());
                }
                if code != 0 {
                    return Err(terminal_failure_reason(code, media_failure.load(Acquire)).into());
                }
                if child_exit.is_some() {
                    return Err("worker_exit".into());
                }
                if reader.is_finished() {
                    return Err("worker_control_reader_closed".into());
                }
                if writer.is_finished() {
                    return Err("worker_control_writer_closed".into());
                }
                return Err("worker_media_closed".into());
            }
            if media_thread.is_none() {
                if let Ok(stream) = listener.accept(child.0.id()) {
                    let stop = media_stop.clone();
                    let expected = token.clone();
                    let tx = pcm_tx.clone();
                    let failure = media_failure.clone();
                    let draining = control_stop.clone();
                    media_thread = Some(std::thread::spawn(move || {
                        if let Err(error) = read_media(stream, &expected, tx, &stop, &draining) {
                            let message = error.to_string();
                            failure.store(
                                if message == "media_consumer_slow" {
                                    1
                                } else if message == "media_length"
                                    || message == "media_authentication"
                                {
                                    2
                                } else if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                                    matches!(
                                        e.kind(),
                                        std::io::ErrorKind::TimedOut
                                            | std::io::ErrorKind::WouldBlock
                                    )
                                }) {
                                    4
                                } else {
                                    3
                                },
                                Release,
                            );
                        }
                    }));
                } else if started.elapsed() > Duration::from_secs(10) {
                    return Err("worker_media_timeout".into());
                }
            }
            if !receiver_ready && started.elapsed() > Duration::from_secs(10) {
                return Err("worker_ready_timeout".into());
            }
            if pending_pair_window
                .as_ref()
                .is_some_and(|(_, _, d)| Instant::now() >= *d)
            {
                update(shared, |e| {
                    e.airplay.receivers[receiver].status.error =
                        Some("pairing_window_timeout".into());
                    e.airplay.receivers[receiver].status.pairing_pin = None;
                });
                pending_pair_window = None;
            }
            if grant_deadline.is_some_and(|d| Instant::now() >= d) {
                return Err("worker_grant_timeout".into());
            }
            if closing_owner
                .as_ref()
                .is_some_and(|(_, _, deadline)| Instant::now() >= *deadline)
            {
                return Err("worker_close_timeout".into());
            }
            let expired = shared.try_lock().is_ok_and(|e| {
                e.resources
                    .admissions
                    .claims
                    .get(&receiver_id)
                    .is_some_and(|c| !c.active && Instant::now() >= c.deadline)
            });
            if expired {
                if let Some(o) = owner.as_ref() {
                    send(&command_tx, targeted("disconnect", o, active_context))?;
                    closing_owner = Some((
                        o.clone(),
                        active_context,
                        Instant::now() + Duration::from_secs(5),
                    ));
                }
                gate.store(false, Release);
                if let Some(producer) = producer.as_ref() {
                    producer.invalidate();
                }
                update(shared, |e| release_claim(e, receiver, producer));
                owner = None;
                admitted = false;
                awaiting_grant = false;
                grant_deadline = None;
                ingress.clear();
            }
            if let Some(publisher) = publisher.as_mut()
                && let Ok(e) = shared.try_lock()
            {
                let visible = {
                    let native = e
                        .authority
                        .current()
                        .sessions
                        .values()
                        .filter(|s| s.status.active())
                        .count()
                        + usize::from(e.resources.native_reservation.is_some());
                    let claim = e.resources.admissions.claims.get(&receiver_id);
                    e.airplay.receivers[receiver].status.enabled
                        && closing_owner.is_none()
                        && e.authority.current().output.available
                        && claim.is_none_or(|c| !c.active)
                        && (claim.is_some()
                            || (e
                                .resources
                                .lanes
                                .iter()
                                .any(|lane| lane.session.is_none() && lane.producer.is_some())
                                && if e.resources.multi_receiver {
                                    native + e.resources.admissions.claims.len() < 4
                                } else {
                                    native <= 1 && e.resources.admissions.claims.is_empty()
                                }))
                };
                publisher.set(visible);
            }
            if let Ok(profiles) = profiles.try_lock()
                && let Some(profile) = profiles.as_ref()
                && let Ok(e) = shared.try_lock()
            {
                let (known, blocked) = profile.trust_for(receiver_id)?;
                let next_generation = e.airplay.receivers[receiver].trust_generation;
                if next_generation != trust_generation {
                    saved.known_keys = known;
                    saved.blocked_keys = blocked;
                    trust_generation = next_generation;
                    send(
                        &command_tx,
                        serde_json::json!({"type":"trust_update","worker_generation":generation,"trust_generation":trust_generation,
                        "known_client_keys":saved.known_keys,"blocked_client_keys":saved.blocked_keys,"pairing_allowed":true}),
                    )?;
                }
            }
            while let Ok(action) = actions.try_recv() {
                match action {
                    Action::PairWindow {
                        trust_generation: new_generation,
                        pin,
                        deadline,
                        attempts,
                    } => {
                        if owner.is_some() {
                            continue;
                        }
                        let guard = profiles.lock().map_err(|_| "airplay_profile_busy")?;
                        let (known, blocked) = guard
                            .as_ref()
                            .ok_or("airplay_profile_missing")?
                            .trust_for(receiver_id)?;
                        if new_generation > trust_generation {
                            saved.known_keys = known;
                            saved.blocked_keys = blocked;
                            trust_generation = new_generation;
                            send(
                                &command_tx,
                                serde_json::json!({"type":"trust_update","worker_generation":generation,"trust_generation":trust_generation,
                                "known_client_keys":saved.known_keys,"blocked_client_keys":saved.blocked_keys,"pairing_allowed":true}),
                            )?;
                        }
                        send(
                            &command_tx,
                            serde_json::json!({"type":"pairing_window","worker_generation":generation,"trust_generation":new_generation,
                            "pin":pin,"remaining_ms":deadline.saturating_duration_since(Instant::now()).as_millis() as u64,"attempts":attempts}),
                        )?;
                        pending_pair_window =
                            Some((new_generation, pin, Instant::now() + Duration::from_secs(5)));
                    }
                    Action::PlaybackMode(mode) => {
                        saved.playback_mode = mode;
                        save_receiver(&state_path, &profiles, receiver_id, &saved, shared)?;
                        ingress.set_playback_mode(mode);
                    }
                    Action::Stop => {
                        send(&command_tx, serde_json::json!({"type":"stop"}))?;
                        return Ok(());
                    }
                    Action::Disconnect(ref target)
                    | Action::Revoke(ref target)
                    | Action::Reset(ref target) => {
                        if target
                            .as_ref()
                            .is_none_or(|(expected, _)| owner.as_ref() != Some(expected))
                        {
                            continue;
                        }
                        cancel_owner_claim(shared, receiver, producer, &gate);
                        ingress.clear();
                        admitted = false;
                        awaiting_grant = false;
                        grant_deadline = None;
                        saved.playback_allowed = true;
                        update(shared, |e| {
                            e.airplay.receivers[receiver].status.playback_allowed = true
                        });
                        send(
                            &command_tx,
                            owner.as_ref().map(|o|targeted(if matches!(action,Action::Revoke(_)){"revoke"}else{"disconnect"},o,active_context)).unwrap_or_else(||serde_json::json!({"type":"allow","worker_generation":generation})),
                        )?;
                        if let Some(o) = owner.as_ref() {
                            closing_owner = Some((
                                o.clone(),
                                active_context,
                                Instant::now() + Duration::from_secs(5),
                            ));
                        }
                        owner = None;
                    }
                    Action::Allow => {
                        saved.playback_allowed = true;
                        save_receiver(&state_path, &profiles, receiver_id, &saved, shared)?;
                        send(
                            &command_tx,
                            serde_json::json!({"type":"allow","worker_generation":generation}),
                        )?;
                    }
                }
            }
            // A mode selection acknowledged before Disable must be saved even
            // when the independent stop flag overtakes this thread's queue.
            if shutdown.load(Acquire) {
                return Ok(());
            }
            for _ in 0..16 {
                let (event, received_at) = if let Some(event) = deferred_event.take() {
                    event
                } else {
                    let Ok(event) = event_rx.try_recv() else {
                        break;
                    };
                    event
                };
                let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if !matches!(
                    event_type,
                    "protocol" | "protocol_detail" | "protocol_transport" | "fatal"
                ) && event["worker_generation"].as_u64() != Some(generation)
                {
                    continue;
                }
                if event_type == "session_ended" {
                    if let Some((closing, context, _)) = closing_owner.as_ref()
                        && terminal_session_event(&event, Some(closing), *context, false)
                    {
                        closing_owner = None;
                        continue;
                    }
                    if !terminal_session_event(
                        &event,
                        owner.as_ref(),
                        active_context,
                        gate.load(Acquire),
                    ) {
                        continue;
                    }
                }
                if matches!(event_type, "flush" | "volume" | "format")
                    && !owner
                        .as_ref()
                        .is_some_and(|o| event_matches(&event, o, Some(active_context)))
                {
                    continue;
                }
                if matches!(event_type, "session_started" | "grant_applied")
                    && !current_session_event(
                        &event,
                        owner.as_ref(),
                        active_context,
                        awaiting_grant,
                        gate.load(Acquire),
                    )
                {
                    continue;
                }
                // A profile transaction may wait on disk. Media scheduling and
                // reservation deadlines must continue while its trust lock is busy.
                let event_profile = if matches!(
                    event_type,
                    "admit_request" | "session_started" | "grant_applied"
                ) {
                    match event_lock(&profiles, event_type, received_at, Instant::now()) {
                        Ok(Some(guard)) => Some(guard),
                        Err("admission_timeout") => {
                            deny_admission(
                                &mut admission_denials,
                                &command_tx,
                                generation,
                                &event,
                                "admission_timeout",
                            )?;
                            continue;
                        }
                        Ok(None) => {
                            deferred_event = Some((event, received_at));
                            break;
                        }
                        Err(_) => return Err("airplay_profile_busy".into()),
                    }
                } else {
                    None
                };
                match event.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "ready" => {
                        if event["pcm_version"].as_u64() != Some(2)
                            || event["control_version"].as_u64() != Some(2)
                            || event["worker_generation"].as_u64() != Some(generation)
                        {
                            return Err("worker_control_version".into());
                        }
                        if event["session_control_version"].as_u64() != Some(1)
                            || event["identity_loader_version"].as_u64() != Some(1)
                            || event["stop_version"].as_u64() != Some(1)
                        {
                            return Err("upgrade_required".into());
                        }
                        receiver_ready = true;
                        let port = event
                            .get("port")
                            .and_then(|v| v.as_u64())
                            .filter(|p| *p > 0 && *p <= 65535)
                            .ok_or("worker_port")? as u16;
                        let public = event
                            .get("public_key")
                            .and_then(|v| v.as_str())
                            .ok_or("worker_public_key")?;
                        let feature_bits = event
                            .get("features")
                            .and_then(|v| v.as_u64())
                            .filter(|bits| *bits == 0x481C5A00)
                            .ok_or("worker_features")?;
                        let features =
                            format!("0x{:X},0x{:X}", feature_bits as u32, feature_bits >> 32);
                        let mut address = listen;
                        address.set_port(port);
                        publisher = Some(DiscoveryTask::new(
                            shared.clone(),
                            receiver,
                            neonmix_identity::speaker::SpeakerPublisher::prepare(
                                hub, &name, address, public, &features,
                            )?,
                        ));
                        update(shared, |e| {
                            e.airplay.receivers[receiver].pairing_deadline = Some(pairing_deadline);
                            e.airplay.receivers[receiver].status.ready = true;
                            e.airplay.receivers[receiver].status.playback_allowed =
                                saved.playback_allowed;
                            e.airplay.receivers[receiver].status.pairing_pin = Some(pin.clone());
                            e.airplay.receivers[receiver].status.revision += 1;
                        });
                    }
                    "pairing_request" => {
                        let trust = event["trust_generation"]
                            .as_u64()
                            .ok_or("pairing_provenance")?;
                        let connection = event["connection_id"]
                            .as_u64()
                            .ok_or("pairing_provenance")?;
                        let request = event["pairing_request_id"]
                            .as_u64()
                            .ok_or("pairing_provenance")?;
                        let (allowed, attempts) = {
                            let mut e = shared.lock().map_err(|_| "airplay_state_busy")?;
                            let now = Instant::now();
                            e.airplay.pairing_leases.retain(|_, p| p.deadline > now);
                            let r = &e.airplay.receivers[receiver];
                            let allowed = !e.airplay.configuration_pending
                                && r.status.enabled
                                && r.status.ready
                                && r.trust_generation == trust
                                && r.pairing_attempts < 5
                                && r.pairing_deadline.is_some_and(|d| now < d)
                                && !e.resources.admissions.claims.contains_key(&receiver_id)
                                && !e.airplay.pairing_leases.contains_key(&receiver_id)
                                && e.airplay.pairing_leases.len() < 4;
                            if allowed {
                                let r = &mut e.airplay.receivers[receiver];
                                r.pairing_attempts += 1;
                                r.pairing_in_progress = true;
                                let deadline = r
                                    .pairing_deadline
                                    .unwrap()
                                    .min(now + Duration::from_secs(60));
                                e.airplay.pairing_leases.insert(
                                    receiver_id,
                                    PairingLease {
                                        generation,
                                        trust_generation: trust,
                                        connection,
                                        request,
                                        deadline,
                                    },
                                );
                            }
                            (allowed, e.airplay.receivers[receiver].pairing_attempts)
                        };
                        send(
                            &command_tx,
                            serde_json::json!({"type":"pairing_admit","worker_generation":generation,"trust_generation":trust,
                            "connection_id":connection,"pairing_request_id":request,"allowed":allowed,"attempts":attempts}),
                        )?;
                    }
                    "pairing_ended" => {
                        update(shared, |e| {
                            if e.airplay
                                .pairing_leases
                                .get(&receiver_id)
                                .is_some_and(|p| pairing_matches(p, &event))
                            {
                                e.airplay.pairing_leases.remove(&receiver_id);
                                e.airplay.receivers[receiver].pairing_in_progress = false;
                            }
                        });
                    }
                    "pairing_window_applied" => {
                        if let Some((trust, new_pin, _)) = pending_pair_window.as_ref()
                            && event["trust_generation"].as_u64() == Some(*trust)
                        {
                            update(shared, |e| {
                                e.airplay.receivers[receiver].status.pairing_pin =
                                    Some(new_pin.clone());
                                e.airplay.receivers[receiver].status.revision += 1;
                            });
                            pending_pair_window = None;
                        }
                    }
                    "pairing_attempt" => {
                        // Observation only. Hub PairingRequest is the sole attempt owner;
                        // delayed worker observations cannot refill a new PIN window.
                        if event["attempts"].as_u64().is_none_or(|n| n > 6) {
                            return Err("pairing_attempt_invalid".into());
                        }
                    }
                    "pairing_pin" => {} // Hub owns the fixed PIN and exposes it only after ready/window-applied.
                    "admit_request" => {
                        let public = event
                            .get("client_public_key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let valid = closing_owner.is_none()
                            && !public.is_empty()
                            && public.len() <= 128
                            && !saved.blocked_keys.contains(public)
                            && saved.playback_allowed
                            && (saved.known_keys.contains(public)
                                || saved.known_keys.len() + saved.blocked_keys.len() < MAX_CLIENTS);
                        let request_id = event["request_id"].as_u64().ok_or("admit_request_id")?;
                        let connection = event["connection_id"]
                            .as_u64()
                            .ok_or("admit_connection_id")?;
                        let source = profile::source_id(public)?;
                        let candidate = Owner {
                            receiver: receiver_id,
                            generation,
                            connection,
                            request: request_id,
                            source,
                        };
                        let mut denial = if admitted {
                            "receiver_busy"
                        } else {
                            "source_policy"
                        };
                        let allowed = if valid && !admitted {
                            let profile_guard =
                                event_profile.as_ref().ok_or("airplay_profile_busy")?;
                            let current_profile =
                                profile_guard.as_ref().ok_or("airplay_profile_missing")?;
                            let trust_valid =
                                trusted(current_profile, receiver_id, &candidate.source);
                            let mut e =
                                match event_lock(shared, event_type, received_at, Instant::now()) {
                                    Ok(Some(engine)) => engine,
                                    Ok(None) => {
                                        deferred_event = Some((event, received_at));
                                        break;
                                    }
                                    Err("admission_timeout") => {
                                        deny_admission(
                                            &mut admission_denials,
                                            &command_tx,
                                            generation,
                                            &event,
                                            "admission_timeout",
                                        )?;
                                        continue;
                                    }
                                    Err(_) => return Err("airplay_state_busy".into()),
                                };
                            let policy_denial = if e.airplay.configuration_pending {
                                Some("profile_durability_unconfirmed")
                            } else if event["trust_generation"].as_u64()
                                != Some(e.airplay.receivers[receiver].trust_generation)
                            {
                                Some("trust_changed")
                            } else if !trust_valid {
                                Some("source_policy")
                            } else if !e.airplay.receivers[receiver].status.enabled
                                || !e.airplay.receivers[receiver].status.playback_allowed
                            {
                                Some("receiver_disabled")
                            } else if !e.authority.current().output.available {
                                Some("output_unavailable")
                            } else {
                                None
                            };
                            if let Some(reason) = policy_denial {
                                denial = reason;
                                false
                            } else {
                                let native = e
                                    .authority
                                    .current()
                                    .sessions
                                    .values()
                                    .filter(|s| s.status.active())
                                    .count()
                                    + usize::from(e.resources.native_reservation.is_some());
                                let mut streams = e
                                    .authority
                                    .current()
                                    .streams
                                    .keys()
                                    .copied()
                                    .collect::<Vec<_>>();
                                if let Some((_, _, _, stream)) = e.resources.native_reservation {
                                    streams.push(stream);
                                }
                                let free = e
                                    .resources
                                    .lanes
                                    .iter()
                                    .position(|l| l.session.is_none() && l.producer.is_some());
                                let multi = e.resources.multi_receiver;
                                match e.resources.admissions.reserve(
                                    candidate.clone(),
                                    native,
                                    multi,
                                    free,
                                    &streams,
                                    Instant::now(),
                                ) {
                                    Ok(claim) => {
                                        *producer = e.resources.lanes[claim.lane].producer.take();
                                        if let Some(producer) = producer.as_mut() {
                                            producer
                                                .bind_next()
                                                .map_err(|_| "binding_generation_exhausted")?;
                                        }
                                        e.airplay.receivers[receiver].lane = Some(claim.lane);
                                        active_context = claim.context;
                                        owner = Some(candidate.clone());
                                        e.airplay.receivers[receiver].session_owner =
                                            Some((candidate.clone(), active_context));
                                        true
                                    }
                                    Err(reason) => {
                                        denial = reason;
                                        false
                                    }
                                }
                            }
                        } else {
                            false
                        };
                        if allowed {
                            admitted = true;
                            admit_epoch = epoch.load(Relaxed);
                        } else {
                            admission_denied(&mut admission_denials, denial);
                        }
                        send(
                            &command_tx,
                            serde_json::json!({"type":"admit","worker_generation":generation,"connection_id":connection,"request_id":request_id,"allowed":allowed}),
                        )?;
                    }
                    "registered" => {
                        let public = event
                            .get("client_public_key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !public.is_empty()
                            && public.len() <= 128
                            && !saved.blocked_keys.contains(public)
                            && saved.playback_allowed
                            && saved.known_keys.len() + saved.blocked_keys.len() < MAX_CLIENTS
                            && shared.lock().is_ok_and(|e| {
                                e.airplay.receivers[receiver].status.enabled
                                    && e.airplay.receivers[receiver].status.playback_allowed
                            })
                        {
                            let mut profile_guard =
                                profiles.lock().map_err(|_| "airplay_profile_busy")?;
                            if shared
                                .lock()
                                .map_err(|_| "airplay_state_busy")?
                                .airplay
                                .configuration_pending
                            {
                                continue;
                            }
                            let current_generation = shared
                                .lock()
                                .map_err(|_| "airplay_state_busy")?
                                .airplay
                                .receivers[receiver]
                                .trust_generation;
                            if event["trust_generation"].as_u64() != Some(current_generation) {
                                continue;
                            }
                            if !shared.lock().is_ok_and(|e| {
                                e.airplay.pairing_leases.get(&receiver_id).is_some_and(|p| {
                                    pairing_matches(p, &event) && p.deadline > Instant::now()
                                })
                            }) {
                                continue;
                            }
                            let mut next = profile_guard
                                .as_ref()
                                .ok_or("airplay_profile_missing")?
                                .clone();
                            let name = event
                                .get("name")
                                .and_then(|v| v.as_str())
                                .filter(|n| !n.trim().is_empty())
                                .map(|n| bounded_name(n, 128));
                            if next.register_source(receiver_id, public, name).is_ok() {
                                api::persist_background(
                                    shared,
                                    &state_path,
                                    profile_guard.as_ref().unwrap(),
                                    &mut next,
                                    profile::save,
                                )?;
                                *profile_guard = Some(next);
                                saved.known_keys.insert(public.into());
                                update(shared, |e| e.airplay.advance_event());
                            }
                        }
                    }
                    "session_started" => {
                        let Some(current_owner) = owner.as_ref() else {
                            continue;
                        };
                        if awaiting_grant
                            || gate.load(Acquire)
                            || !event_matches(&event, current_owner, None)
                            || event["session_id"].as_u64() != Some(0)
                            || event["stream_epoch"].as_u64() != Some(0)
                        {
                            continue;
                        }
                        let still_allowed = {
                            let profiles = event_profile.as_ref().ok_or("airplay_profile_busy")?;
                            let trust_valid = profiles
                                .as_ref()
                                .is_some_and(|p| trusted(p, receiver_id, &current_owner.source));
                            let e = shared.lock().map_err(|_| "airplay_state_busy")?;
                            trust_valid
                                && e.airplay.receivers[receiver].status.enabled
                                && e.airplay.receivers[receiver].status.playback_allowed
                                && e.authority.current().output.available
                                && epoch.load(Relaxed) == admit_epoch
                                && e.resources.admissions.claims.get(&receiver_id).is_some_and(
                                    |c| c.owner == *current_owner && Instant::now() < c.deadline,
                                )
                        };
                        if !admitted || !still_allowed {
                            send(
                                &command_tx,
                                targeted("disconnect", current_owner, active_context),
                            )?;
                            update(shared, |e| release_claim(e, receiver, producer));
                            closing_owner = Some((
                                current_owner.clone(),
                                active_context,
                                Instant::now() + Duration::from_secs(5),
                            ));
                            gate.store(false, Release);
                            owner = None;
                            admitted = false;
                            awaiting_grant = false;
                            grant_deadline = None;
                            continue;
                        }
                        active_context.stream_epoch = epoch.fetch_add(1, Relaxed) + 1;
                        update(shared, |e| {
                            e.airplay.receivers[receiver].session_owner =
                                Some((current_owner.clone(), active_context))
                        });
                        // A new connection gets fresh per-session counters/state;
                        // flush below resets only its epoch and retains its counters.
                        ingress = Ingress::new(origin, active_context);
                        received_blocks = 0;
                        rejected_blocks = 0;
                        released_blocks = 0;
                        awaiting_grant = true;
                        grant_deadline = Some(Instant::now() + Duration::from_secs(5));
                        send(&command_tx, grant(active_context, current_owner))?;
                        let source_preferences = {
                            let profiles = event_profile.as_ref().ok_or("airplay_profile_busy")?;
                            profiles
                                .as_ref()
                                .and_then(|p| {
                                    p.sources
                                        .iter()
                                        .find(|s| s.source_id == current_owner.source)
                                })
                                .cloned()
                        };
                        let actual_mode = source_preferences
                            .as_ref()
                            .and_then(|s| s.playback_mode)
                            .map(|m| {
                                serde_json::from_value(serde_json::to_value(m).unwrap()).unwrap()
                            })
                            .unwrap_or(saved.playback_mode);
                        ingress.set_playback_mode(actual_mode);
                        update(shared, |e| {
                            e.airplay.receivers[receiver].status.source_revoked = false;
                            e.airplay.receivers[receiver].status.media_resets = 0;
                            e.airplay.receivers[receiver].status.last_media_reset = None;
                            e.airplay.receivers[receiver].status.format = None;
                            e.airplay.receivers[receiver].status.error = None;
                            e.airplay.receivers[receiver].status.source_id =
                                Some(current_owner.source.clone());
                            e.airplay.receivers[receiver].status.source_name = event
                                .get("name")
                                .and_then(|v| v.as_str())
                                .map(|s| s.chars().take(128).collect());
                            e.airplay.receivers[receiver].status.pairing_pin = None;
                            e.airplay.receivers[receiver].status.mix = source_preferences
                                .as_ref()
                                .map(|s| neonmix_control::Mix {
                                    gain_db: s.gain_db,
                                    muted: s.muted,
                                    solo: false,
                                })
                                .unwrap_or_default();
                            e.airplay.receivers[receiver].status.playback_mode = actual_mode;
                            e.airplay.receivers[receiver].status.revision += 1;
                        });
                    }
                    "grant_applied" => {
                        let Some(current_owner) = owner.as_ref() else {
                            continue;
                        };
                        if !awaiting_grant
                            || !event_matches(&event, current_owner, Some(active_context))
                            || event["stream_id"].as_u64() != Some(active_context.stream_id)
                            || event["format_epoch"].as_u64() != Some(active_context.format_epoch)
                            || event["mapping_id"].as_u64() != Some(active_context.mapping_id)
                        {
                            continue;
                        }
                        let profiles = event_profile.as_ref().ok_or("airplay_profile_busy")?;
                        let trust_valid = profiles
                            .as_ref()
                            .is_some_and(|p| trusted(p, receiver_id, &current_owner.source));
                        let mut e = shared.lock().map_err(|_| "airplay_state_busy")?;
                        if !trust_valid
                            || epoch.load(Relaxed) != active_context.stream_epoch
                            || !e.airplay.receivers[receiver].status.playback_allowed
                            || !e.authority.current().output.available
                            || !e.airplay.receivers[receiver].status.enabled
                        {
                            continue;
                        }
                        if let Ok(claim) =
                            e.resources.admissions.commit(current_owner, Instant::now())
                        {
                            if let Some(current) =
                                e.resources.admissions.claims.get_mut(&receiver_id)
                            {
                                current.context = active_context;
                            }
                            let mix = e.airplay.receivers[receiver].status.mix;
                            e.resources.airplay_mix[receiver] = Some((
                                claim.lane,
                                LaneMix {
                                    stream_id: active_context.stream_id,
                                    epoch: active_context.stream_epoch,
                                    binding_generation: producer
                                        .as_ref()
                                        .map_or(1, BlockProducer::binding_generation),
                                    playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
                                    gain_db: mix.gain_db,
                                    muted: mix.muted,
                                    solo: mix.solo,
                                },
                            ));
                            let state = e.authority.snapshot();
                            if e.resources.prepare(&state, None).is_ok() {
                                e.airplay.receivers[receiver].status.active = true;
                                gate.store(true, Release);
                                awaiting_grant = false;
                                grant_deadline = None;
                                e.airplay.advance_event();
                            } else {
                                release_claim(&mut e, receiver, producer);
                                send(
                                    &command_tx,
                                    targeted("disconnect", current_owner, active_context),
                                )?;
                                closing_owner = Some((
                                    current_owner.clone(),
                                    active_context,
                                    Instant::now() + Duration::from_secs(5),
                                ));
                                gate.store(false, Release);
                                owner = None;
                                admitted = false;
                                awaiting_grant = false;
                                grant_deadline = None;
                            }
                        }
                    }
                    "flush" => {
                        let parsed: neonmix_airplay_ipc::WorkerEvent =
                            serde_json::from_value(event)?;
                        let neonmix_airplay_ipc::WorkerEvent::Flush { reason, .. } = parsed else {
                            return Err("worker_media_reset_type".into());
                        };
                        let reason = match reason {
                            neonmix_airplay_ipc::MediaResetReason::Protocol => "protocol",
                            neonmix_airplay_ipc::MediaResetReason::StreamSetup => "stream_setup",
                            neonmix_airplay_ipc::MediaResetReason::TimestampJump => {
                                "timestamp_jump"
                            }
                        };
                        if !admitted || !gate.load(Acquire) {
                            continue;
                        }
                        gate.store(false, Release);
                        if let Some(p) = producer.as_mut() {
                            p.invalidate();
                        }
                        ingress.clear();
                        active_context.stream_epoch = epoch.fetch_add(1, Relaxed) + 1;
                        update(shared, |e| {
                            e.airplay.receivers[receiver].session_owner =
                                owner.clone().map(|o| (o, active_context))
                        });
                        ingress.reset(active_context);
                        awaiting_grant = true;
                        grant_deadline = Some(Instant::now() + Duration::from_secs(5));
                        send(
                            &command_tx,
                            grant(active_context, owner.as_ref().ok_or("session_missing")?),
                        )?;
                        update(shared, |e| {
                            e.airplay.receivers[receiver].status.media_resets = e.airplay.receivers
                                [receiver]
                                .status
                                .media_resets
                                .saturating_add(1);
                            e.airplay.receivers[receiver].status.last_media_reset =
                                Some(reason.into());
                            if let Some((_, mix)) = &mut e.resources.airplay_mix[receiver] {
                                mix.epoch = active_context.stream_epoch;
                            }
                        });
                    }
                    "session_ended" => {
                        awaiting_grant = false;
                        grant_deadline = None;
                        update(shared, |e| release_claim(e, receiver, producer));
                        owner = None;
                        if let Some(p) = producer.as_mut() {
                            p.invalidate();
                        }
                        ingress.clear();
                        admitted = false;
                        gate.store(false, Release);
                        update(shared, |e| {
                            e.airplay.receivers[receiver].status.active = false;
                            e.airplay.receivers[receiver].status.revision += 1;
                            e.resources.airplay_mix[receiver] = None;
                            let state = e.authority.snapshot();
                            let _ = e.resources.prepare(&state, None);
                        });
                    }
                    "protocol_transport" => {
                        let parsed: neonmix_airplay_ipc::WorkerEvent =
                            serde_json::from_value(event)?;
                        let neonmix_airplay_ipc::WorkerEvent::ProtocolTransport(ref transport) =
                            parsed
                        else {
                            return Err("protocol_transport_type".into());
                        };
                        transport.validate()?;
                        let safe = serde_json::to_value(parsed)?;
                        update(shared, |e| {
                            if e.airplay.receivers[receiver].status.protocol_events.len() == 32 {
                                e.airplay.receivers[receiver]
                                    .status
                                    .protocol_events
                                    .pop_front();
                            }
                            e.airplay.receivers[receiver]
                                .status
                                .protocol_events
                                .push_back(safe);
                        });
                    }
                    "protocol_detail" => {
                        let stage = event
                            .get("stage")
                            .and_then(|v| v.as_str())
                            .filter(|s| *s == "srp_proof")
                            .ok_or("protocol_detail_stage")?;
                        let reason = event
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .filter(|s| {
                                [
                                    "received",
                                    "invalid_length",
                                    "invalid_phase",
                                    "proof_rejected",
                                    "verified",
                                ]
                                .contains(s)
                            })
                            .ok_or("protocol_detail_reason")?;
                        let a_bytes = event
                            .get("a_bytes")
                            .and_then(|v| v.as_u64())
                            .filter(|v| *v <= 16384)
                            .ok_or("protocol_detail_length")?;
                        let proof_bytes = event
                            .get("proof_bytes")
                            .and_then(|v| v.as_u64())
                            .filter(|v| *v <= 16384)
                            .ok_or("protocol_detail_length")?;
                        update(shared, |e| {
                            if e.airplay.receivers[receiver].status.protocol_events.len() == 32 {
                                e.airplay.receivers[receiver]
                                    .status
                                    .protocol_events
                                    .pop_front();
                            }
                            e.airplay.receivers[receiver].status.protocol_events.push_back(serde_json::json!({"stage":stage,"reason":reason,"a_bytes":a_bytes,"proof_bytes":proof_bytes,"at_worker_ms":started.elapsed().as_millis() as u64}));
                        });
                    }
                    "protocol" => {
                        let method = event
                            .get("method")
                            .and_then(|v| v.as_str())
                            .filter(|s| {
                                s.len() <= 16
                                    && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
                            })
                            .ok_or("protocol_method")?;
                        let route = event
                            .get("route")
                            .and_then(|v| v.as_str())
                            .filter(|s| {
                                s.len() <= 32
                                    && s.bytes()
                                        .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
                            })
                            .ok_or("protocol_route")?;
                        let status = event
                            .get("status")
                            .and_then(|v| v.as_u64())
                            .filter(|s| (100..=599).contains(s))
                            .ok_or("protocol_status")?;
                        update(shared, |e| {
                            if e.airplay.receivers[receiver].status.protocol_events.len() == 32 {
                                e.airplay.receivers[receiver]
                                    .status
                                    .protocol_events
                                    .pop_front();
                            }
                            e.airplay.receivers[receiver].status.protocol_events.push_back(
                                serde_json::json!({"method":method,"route":route,"status":status,
                                    "at_worker_ms":started.elapsed().as_millis() as u64}),
                            );
                        });
                    }
                    "format" => {
                        let codec = event
                            .get("codec")
                            .and_then(|v| v.as_str())
                            .filter(|s| ["pcm_s16", "alac", "aac_lc", "aac_eld"].contains(s))
                            .ok_or("worker_codec")?;
                        let rate = event
                            .get("source_rate")
                            .and_then(|v| v.as_u64())
                            .filter(|v| *v == 44100)
                            .ok_or("worker_rate")?;
                        let frames = event
                            .get("source_frame_count")
                            .and_then(|v| v.as_u64())
                            .filter(|v| (1..=8192).contains(v))
                            .ok_or("worker_frames")?;
                        update(shared, |e| {
                            e.airplay.receivers[receiver].status.format = Some(
                                serde_json::json!({"codec":codec,"source_rate":rate,"source_frame_count":frames}),
                            )
                        });
                    }
                    "volume" => {} // gain is carried per PCM block and applied exactly once by Ingress.
                    "fatal" => {
                        let message = event.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        return Err(terminal_failure_reason(
                            fatal_code(message),
                            media_failure.load(Acquire),
                        )
                        .into());
                    }
                    _ => return Err("worker_event_invalid".into()),
                }
            }
            // The epoch translation is chosen by the first packet. Observe
            // device backlog before accepting it, not after draining PCM.
            max_control_ns = max_control_ns
                .max(loop_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
            let pcm_started = Instant::now();
            ingress.set_output_latency_ns(output_latency.load(Relaxed));
            for _ in 0..8 {
                let Ok(packet) = pcm_rx.try_recv() else {
                    break;
                };
                let accepted = if gate.load(Acquire) || awaiting_grant {
                    match ingress.push(packet) {
                        Ok(()) => true,
                        Err(
                            neonmix_airplay_adapter::IngressError::ClockReset
                            | neonmix_airplay_adapter::IngressError::Full,
                        ) => {
                            gate.store(false, Release);
                            if let Some(p) = producer.as_mut() {
                                p.invalidate();
                            }
                            ingress.clear();
                            admitted = false;
                            if let Some(o) = owner.as_ref() {
                                send(&command_tx, targeted("disconnect", o, active_context))?;
                                closing_owner = Some((
                                    o.clone(),
                                    active_context,
                                    Instant::now() + Duration::from_secs(5),
                                ));
                            }
                            update(shared, |e| {
                                e.airplay.receivers[receiver].status.error =
                                    Some("timing_invalidated".into());
                                e.airplay.receivers[receiver].status.active = false;
                                e.resources.airplay_mix[receiver] = None;
                                let state = e.authority.snapshot();
                                let _ = e.resources.prepare(&state, None);
                            });
                            false
                        }
                        Err(_) => false,
                    }
                } else {
                    false
                };
                received_blocks += 1;
                if !accepted {
                    rejected_blocks += 1;
                }
            }
            if producer.is_none() {
                ingress.release_storage();
            }
            if gate.load(Acquire) {
                let released = producer
                    .as_mut()
                    .map(|p| ingress.release_due(Instant::now(), p))
                    .unwrap_or(0);
                released_blocks += released as u64;
            } else if !awaiting_grant {
                ingress.clear();
            }
            let stats = ingress.stats();
            max_pcm_ns =
                max_pcm_ns.max(pcm_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
            // Audio scheduling never waits for native session preparation or
            // diagnostic readers holding the authoritative control mutex.
            if let Ok(mut engine) = shared.try_lock() {
                let now = Instant::now();
                engine
                    .airplay
                    .pairing_leases
                    .retain(|_, p| p.deadline > now);
                let pairing_live = engine.airplay.pairing_leases.contains_key(&receiver_id);
                engine.airplay.receivers[receiver].pairing_in_progress = pairing_live;
                if engine.airplay.mixer_dirty {
                    let state = engine.authority.snapshot();
                    if engine.resources.prepare(&state, None).is_ok() {
                        engine.airplay.mixer_dirty = false;
                    }
                }
                let status = &mut engine.airplay.receivers[receiver].status;
                // Publish diagnostics only when the Engine is available; rejection
                // accounting must not block the same loop that just deferred it.
                for (reason, count) in std::mem::take(&mut admission_denials) {
                    let total = status.admission_denials.entry(reason).or_default();
                    *total = total.saturating_add(count);
                }
                status.received_blocks = received_blocks;
                status.rejected_blocks = rejected_blocks;
                status.released_blocks = released_blocks;
                status.queued_blocks = stats.pending_frames.div_ceil(480);
                status.max_loop_gap_ns = max_loop_gap_ns;
                status.max_control_ns = max_control_ns;
                status.max_pcm_ns = max_pcm_ns;
                status.ingress = stats;
                status.protocol_events_dropped = trace_dropped.load(Relaxed);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    })();
    gate.store(false, Release);
    if let Some(p) = producer.as_mut() {
        p.invalidate();
    }
    drop(publisher);
    control_stop.store(true, Release);
    drop(command_tx);
    let deadline = stop_deadline
        .lock()
        .ok()
        .and_then(|d| *d)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(2));
    let mut forced = false;
    let exit = loop {
        match child.0.try_wait() {
            Ok(Some(exit)) => {
                child.1 = false;
                break Ok(exit);
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                forced = true;
                let killed = child.0.kill();
                if killed.is_err() {
                    break Err("worker_kill_failed");
                }
                match child.0.wait() {
                    Ok(exit) => {
                        child.1 = false;
                        break Ok(exit);
                    }
                    Err(_) => break Err("worker_wait_failed"),
                }
            }
            Err(_) => break Err("worker_wait_failed"),
        }
    };
    media_stop.store(true, Release);
    let _ = writer.join();
    let _ = reader.join();
    if let Some(thread) = media_thread {
        let _ = thread.join();
    }
    let cleanup_complete = key_guard.cleanup();
    if forced {
        worker_forced.store(true, Release);
    }
    crate::emit(
        serde_json::json!({"event":"airplay_worker_stopped","forced":forced,"exit_code":exit.as_ref().ok().and_then(|e|e.code()),"cleanup_complete":cleanup_complete}),
    )?;
    if !cleanup_complete {
        return Err("worker_key_cleanup_failed".into());
    }
    let exit =
        exit.map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    if result.is_ok() && !forced && !exit.success() {
        return Err("worker_shutdown_failed".into());
    }
    result
}
fn write_worker_control(
    input: &mut std::process::ChildStdin,
    value: &serde_json::Value,
    cancel: &AtomicBool,
) -> crate::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() + 1 > neonmix_airplay_ipc::MAX_CONTROL_BYTES {
        return Err("worker_control_length".into());
    }
    bytes.push(b'\n');
    neonmix_lifecycle::write_bounded(input, &bytes, cancel, Duration::from_millis(250))?;
    Ok(())
}
fn read_media(
    mut stream: MediaStream,
    token: &str,
    tx: SyncSender<PcmPacket>,
    stop: &AtomicBool,
    draining: &AtomicBool,
) -> crate::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    let mut auth = vec![0u8; token.len() + 1];
    read_media_exact(&mut stream, &mut auth, stop)?;
    if auth[..token.len()] != *token.as_bytes() || auth[token.len()] != b'\n' {
        return Err("media_authentication".into());
    }
    let mut frame = vec![0u8; HEADER_BYTES + 3840];
    while !stop.load(Acquire) {
        match stream.read(&mut frame[..1]) {
            Ok(0) => return Err("worker_media_eof".into()),
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        read_media_exact(&mut stream, &mut frame[1..HEADER_BYTES], stop)?;
        let length = u32::from_le_bytes(frame[8..12].try_into()?) as usize;
        if length > 3840 {
            return Err("media_length".into());
        }
        read_media_exact(
            &mut stream,
            &mut frame[HEADER_BYTES..HEADER_BYTES + length],
            stop,
        )?;
        let packet = PcmPacket::decode(&frame[..HEADER_BYTES + length])?;
        queue_media_packet(&tx, packet, draining)?;
    }
    Ok(())
}

fn read_media_exact(
    stream: &mut impl Read,
    mut bytes: &mut [u8],
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(250);
    while !bytes.is_empty() {
        if stop.load(Acquire) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        if Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let count = stream.read(bytes)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes = &mut bytes[count..];
    }
    Ok(())
}

fn queue_media_packet(
    tx: &SyncSender<PcmPacket>,
    mut packet: PcmPacket,
    stop: &AtomicBool,
) -> crate::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        if stop.load(Acquire) {
            return Ok(());
        }
        match tx.try_send(packet) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("media_consumer_closed".into()),
            Err(mpsc::TrySendError::Full(pending)) => {
                packet = pending;
                if Instant::now() >= deadline {
                    return Err("media_consumer_slow".into());
                }
                // Bounded backpressure lives only on the media reader. The
                // independent control pipe and stop flag remain responsive.
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

fn pairing_matches(lease: &PairingLease, event: &serde_json::Value) -> bool {
    event["worker_generation"].as_u64() == Some(lease.generation)
        && event["trust_generation"].as_u64() == Some(lease.trust_generation)
        && event["connection_id"].as_u64() == Some(lease.connection)
        && event["pairing_request_id"].as_u64() == Some(lease.request)
}
fn event_matches(event: &serde_json::Value, owner: &Owner, context: Option<Context>) -> bool {
    event["worker_generation"].as_u64() == Some(owner.generation)
        && event["connection_id"].as_u64() == Some(owner.connection)
        && event["request_id"].as_u64() == Some(owner.request)
        && context.is_none_or(|c| {
            event["session_id"].as_u64() == Some(c.session_id)
                && event["stream_epoch"].as_u64() == Some(c.stream_epoch)
        })
}
fn current_session_event(
    event: &serde_json::Value,
    owner: Option<&Owner>,
    context: Context,
    awaiting_grant: bool,
    gate_open: bool,
) -> bool {
    owner.is_some_and(|owner| match event["type"].as_str() {
        Some("session_started") => {
            !awaiting_grant
                && !gate_open
                && event_matches(event, owner, None)
                && event["session_id"].as_u64() == Some(0)
                && event["stream_epoch"].as_u64() == Some(0)
        }
        Some("grant_applied") => {
            awaiting_grant
                && event_matches(event, owner, Some(context))
                && event["stream_id"].as_u64() == Some(context.stream_id)
                && event["format_epoch"].as_u64() == Some(context.format_epoch)
                && event["mapping_id"].as_u64() == Some(context.mapping_id)
        }
        _ => false,
    })
}
fn targeted(kind: &str, owner: &Owner, context: Context) -> serde_json::Value {
    serde_json::json!({"type":kind,"worker_generation":owner.generation,"connection_id":owner.connection,"request_id":owner.request,"session_id":context.session_id,"stream_epoch":context.stream_epoch})
}
fn terminal_session_event(
    event: &serde_json::Value,
    owner: Option<&Owner>,
    context: Context,
    gate_open: bool,
) -> bool {
    owner.is_some_and(|owner| {
        if !event_matches(event, owner, None) {
            return false;
        }
        if gate_open {
            return event_matches(event, owner, Some(context));
        }
        let session = event["session_id"].as_u64();
        let epoch = event["stream_epoch"].as_u64();
        (session == Some(0) && epoch == Some(0))
            || (session == Some(context.session_id)
                && epoch.is_some_and(|epoch| epoch > 0 && epoch <= context.stream_epoch))
    })
}
fn cancel_owner_claim(
    shared: &Shared,
    receiver: usize,
    producer: &mut Option<BlockProducer>,
    gate: &AtomicBool,
) {
    gate.store(false, Release);
    if let Some(producer) = producer.as_ref() {
        producer.invalidate();
    }
    update(shared, |engine| release_claim(engine, receiver, producer));
}
fn grant(context: Context, owner: &Owner) -> serde_json::Value {
    serde_json::json!({"type":"grant","worker_generation":owner.generation,"connection_id":owner.connection,"request_id":owner.request,"session_id":context.session_id,"stream_id":context.stream_id,"stream_epoch":context.stream_epoch,"format_epoch":context.format_epoch,"mapping_id":context.mapping_id})
}
fn fatal_code(message: &str) -> u8 {
    match message {
        "identity_path_encoding" | "identity_path_invalid" => return 10,
        "identity_not_found" => return 11,
        "identity_read_failed" => return 12,
        "identity_permission_denied" => return 13,
        "identity_format_invalid" => return 14,
        _ => {}
    }
    if message == "decoder compressed queue limit exceeded" {
        7
    } else if message == "PCM queue limit exceeded" {
        8
    } else if message == "decoder metadata limit exceeded" {
        9
    } else if message.contains("queue limit") {
        1
    } else if message.contains("decoder") || message.contains("audio") || message.contains("gst") {
        2
    } else if message.contains("media IPC") {
        3
    } else {
        4
    }
}
fn fatal_reason(code: u8) -> &'static str {
    match code {
        1 => "worker_decoder_queue",
        2 => "worker_decoder_failure",
        3 => "worker_media_transport",
        4 => "worker_protocol_failure",
        5 => "worker_control_invalid",
        6 => "worker_control_overflow",
        7 => "worker_decoder_input_queue",
        8 => "worker_pcm_output_queue",
        9 => "worker_decoder_metadata_queue",
        10 => "identity_path_invalid",
        11 => "credential_missing",
        12 => "credential_io_failed",
        13 => "credential_permission_denied",
        14 => "credential_corrupt",
        _ => "worker_protocol_failure",
    }
}
fn media_failure_reason(code: u8) -> Option<&'static str> {
    match code {
        1 => Some("worker_media_backpressure"),
        2 => Some("worker_media_bad_packet"),
        4 => Some("worker_media_frame_timeout"),
        // EOF/IO alone may be a consequence of a worker crash or decoder error.
        _ => None,
    }
}
fn terminal_failure_reason(terminal: u8, media: u8) -> &'static str {
    if terminal == 3
        && let Some(reason) = media_failure_reason(media)
    {
        return reason;
    }
    fatal_reason(terminal)
}
fn public_worker_error(message: &str) -> &'static str {
    match message {
        "upgrade_required" => "upgrade_required",
        "identity_path_invalid" => "invalid_path",
        "credential_missing" => "credential_missing",
        "credential_io_failed" => "credential_io_failed",
        "credential_permission_denied" => "credential_permission_denied",
        "credential_corrupt" => "credential_corrupt",
        "worker_key_cleanup_failed" => "runtime_cleanup_incomplete",
        "worker_shutdown_failed" | "worker_kill_failed" | "worker_wait_failed" => "stop_failed",
        _ => "worker_unavailable",
    }
}
fn failure_stage(message: &str) -> &'static str {
    match message {
        "worker_media_backpressure" => "media_backpressure",
        "worker_media_bad_packet" => "media_validation",
        "worker_media_closed" => "media_closed",
        "worker_media_frame_timeout" => "media_frame_timeout",
        "worker_media_transport" => "media_transport",
        "worker_control_invalid" => "control_invalid",
        "worker_control_overflow" => "control_overflow",
        "worker_control_reader_closed" => "control_reader_closed",
        "worker_control_writer_closed" => "control_writer_closed",
        "worker_decoder_queue" => "decoder_queue",
        "worker_decoder_input_queue" => "decoder_input_queue",
        "worker_pcm_output_queue" => "pcm_output_queue",
        "worker_decoder_metadata_queue" => "decoder_metadata_queue",
        "worker_decoder_failure" => "decoder",
        "worker_protocol_failure" => "protocol",
        "worker_exit" => "worker_exit",
        "upgrade_required" => "worker_capabilities",
        "identity_path_invalid" => "identity_path",
        "credential_missing" => "identity_missing",
        "credential_io_failed" => "identity_read",
        "credential_permission_denied" => "identity_permission",
        "credential_corrupt" => "identity_format",
        _ => "startup_or_native_runtime",
    }
}

fn bounded_name(value: &str, limit: usize) -> String {
    let mut result = String::new();
    for c in value.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > limit {
            break;
        }
        result.push(c);
    }
    result
}
fn speaker_name(room: &str) -> String {
    let mut name = String::from("NeonMix — ");
    for c in room.chars() {
        if name.len() + c.len_utf8() > 50 {
            break;
        }
        name.push(c);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_queue_failures_keep_the_failed_stage_without_raw_messages() {
        for (message, stage) in [
            (
                "decoder compressed queue limit exceeded",
                "decoder_input_queue",
            ),
            ("PCM queue limit exceeded", "pcm_output_queue"),
            ("decoder metadata limit exceeded", "decoder_metadata_queue"),
        ] {
            let code = fatal_code(message);
            assert_eq!(failure_stage(terminal_failure_reason(code, 0)), stage);
            assert_eq!(failure_stage(terminal_failure_reason(code, 1)), stage);
        }
    }

    #[test]
    fn worker_transport_failure_preserves_specific_reader_cause_without_masking_decoder() {
        for (code, reason) in [
            (1, "worker_media_backpressure"),
            (2, "worker_media_bad_packet"),
            (4, "worker_media_frame_timeout"),
        ] {
            assert_eq!(terminal_failure_reason(3, code), reason);
            for terminal in [1, 2, 4, 5, 6] {
                assert_eq!(
                    terminal_failure_reason(terminal, code),
                    fatal_reason(terminal)
                );
            }
        }
        for media in [0, 3, 255] {
            assert_eq!(terminal_failure_reason(3, media), "worker_media_transport");
        }
    }
    #[test]
    fn admission_retries_profile_contention_without_blocking_media_or_extending_deadline() {
        let profiles = Mutex::new(None::<ReceiverProfile>);
        let received = Instant::now();
        let guard = profiles.lock().unwrap();
        // A reader or durable transaction may own the mutex. The media loop
        // must regain control, retaining the request for a later iteration.
        assert!(
            event_lock(&profiles, "admit_request", received, received)
                .unwrap()
                .is_none()
        );
        assert!(
            event_lock(
                &profiles,
                "admit_request",
                received,
                received + Duration::from_millis(399)
            )
            .unwrap()
            .is_none()
        );
        drop(guard);
        assert!(
            event_lock(
                &profiles,
                "admit_request",
                received,
                received + Duration::from_millis(399)
            )
            .unwrap()
            .is_some()
        );
        // Expired requests cannot take capacity even if the lock is now free.
        assert_eq!(
            event_lock(
                &profiles,
                "admit_request",
                received,
                received + Duration::from_millis(400)
            )
            .err(),
            Some("admission_timeout")
        );
        // The Engine lock uses the same original deadline after profile access.
        let engine = Mutex::new(());
        let held = engine.lock().unwrap();
        assert!(
            event_lock(
                &engine,
                "admit_request",
                received,
                received + Duration::from_millis(399)
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            event_lock(
                &engine,
                "admit_request",
                received,
                received + Duration::from_millis(500)
            )
            .err(),
            Some("admission_timeout")
        );
        drop(held);
        assert_eq!(
            event_lock(
                &engine,
                "admit_request",
                received,
                received + Duration::from_millis(501)
            )
            .err(),
            Some("admission_timeout")
        );
    }
    #[test]
    fn deferred_grant_is_discarded_after_disconnect_before_reacquiring_profile() {
        let owner = Owner {
            receiver: Uuid::new_v4(),
            generation: 1,
            connection: 2,
            request: 3,
            source: "fixture".into(),
        };
        let context = pcm_fixture().header;
        let context = Context {
            session_id: context.session_id,
            stream_id: context.stream_id,
            stream_epoch: context.stream_epoch,
            format_epoch: context.format_epoch,
            mapping_id: context.mapping_id,
        };
        let mut event = grant(context, &owner);
        event["type"] = serde_json::json!("grant_applied");
        assert!(current_session_event(
            &event,
            Some(&owner),
            context,
            true,
            false
        ));
        assert!(!current_session_event(&event, None, context, false, false));
        let mut successor = owner.clone();
        successor.connection += 1;
        assert!(!current_session_event(
            &event,
            Some(&successor),
            context,
            true,
            false
        ));
    }
    #[test]
    fn pregrant_terminal_event_requires_exact_owner_and_active_zero_context_is_rejected() {
        let owner = Owner {
            receiver: Uuid::new_v4(),
            generation: 7,
            connection: 2,
            request: 3,
            source: "fixture".into(),
        };
        let context = Context {
            session_id: 4,
            stream_id: 5,
            stream_epoch: 6,
            format_epoch: 7,
            mapping_id: 8,
        };
        let mut ended = targeted("session_ended", &owner, context);
        assert_eq!(ended["request_id"], 3);
        assert!(terminal_session_event(&ended, Some(&owner), context, true));
        ended["session_id"] = serde_json::json!(0);
        ended["stream_epoch"] = serde_json::json!(0);
        assert!(terminal_session_event(&ended, Some(&owner), context, false));
        assert!(!terminal_session_event(&ended, Some(&owner), context, true));
        ended["request_id"] = serde_json::json!(2);
        assert!(!terminal_session_event(
            &ended,
            Some(&owner),
            context,
            false
        ));
        ended["request_id"] = serde_json::json!(3);
        ended["connection_id"] = serde_json::json!(1);
        assert!(!terminal_session_event(
            &ended,
            Some(&owner),
            context,
            false
        ));
        assert!(!terminal_session_event(&ended, None, context, false));
    }
    #[test]
    fn cancelling_a_real_claim_returns_lane_and_fences_late_start_grant_without_affecting_other_claim()
     {
        for active in [false, true] {
            let (shared, _, _, _mixer) = super::super::transaction_tests::shared_fixture();
            let (owner, context, gate, mut producer, other) = {
                let mut engine = shared.lock().unwrap();
                let receiver = engine.airplay.receivers[0].receiver_id;
                let owner = Owner {
                    receiver,
                    generation: 7,
                    connection: 2,
                    request: 3,
                    source: "cancelled".into(),
                };
                let claim = engine
                    .resources
                    .admissions
                    .reserve(owner.clone(), 0, true, Some(0), &[], Instant::now())
                    .unwrap();
                if active {
                    engine
                        .resources
                        .admissions
                        .commit(&owner, Instant::now())
                        .unwrap();
                }
                let other = Owner {
                    receiver: Uuid::new_v4(),
                    generation: 8,
                    connection: 9,
                    request: 10,
                    source: "survivor".into(),
                };
                engine
                    .resources
                    .admissions
                    .reserve(other.clone(), 0, true, Some(1), &[], Instant::now())
                    .unwrap();
                let endpoint = &mut engine.airplay.receivers[0];
                endpoint.lane = Some(0);
                endpoint.session_owner = Some((owner.clone(), claim.context));
                endpoint.status.active = active;
                let gate = endpoint.gate.clone();
                gate.store(active, Release);
                let producer = engine.resources.lanes[0].producer.take();
                (owner, claim.context, gate, producer, other)
            };
            let barrier = std::sync::Barrier::new(2);
            std::thread::scope(|scope| {
                let delayed = scope.spawn(|| {
                    barrier.wait();
                    barrier.wait();
                    let mut event = grant(context, &owner);
                    event["type"] = serde_json::json!("grant_applied");
                    assert!(!current_session_event(&event, None, context, false, false));
                    event["type"] = serde_json::json!("session_started");
                    event["session_id"] = serde_json::json!(0);
                    event["stream_epoch"] = serde_json::json!(0);
                    assert!(!current_session_event(&event, None, context, false, false));
                });
                barrier.wait();
                cancel_owner_claim(&shared, 0, &mut producer, &gate);
                barrier.wait();
                delayed.join().unwrap();
            });
            assert!(!gate.load(Acquire));
            assert!(producer.is_none());
            let engine = shared.lock().unwrap();
            assert!(
                !engine
                    .resources
                    .admissions
                    .claims
                    .contains_key(&owner.receiver)
            );
            assert!(
                engine
                    .resources
                    .admissions
                    .claims
                    .contains_key(&other.receiver)
            );
            assert!(engine.resources.lanes[0].producer.is_some());
            assert!(engine.resources.airplay_mix[0].is_none());
            assert!(engine.airplay.receivers[0].session_owner.is_none());
            assert!(!engine.airplay.receivers[0].status.active);
        }
    }
    #[test]
    fn existing_receiver_profiles_default_to_low_latency_and_keep_trust() {
        let saved: Saved = serde_json::from_value(serde_json::json!({
            "version": 1, "credential_store": "file", "key_reference": null, "known_keys": ["client"],
            "blocked_keys": [], "playback_allowed": true
        }))
        .unwrap();
        assert_eq!(saved.playback_mode, PlaybackMode::LowLatency);
        assert!(saved.known_keys.contains("client"));
        let restored: Saved = serde_json::from_value(
            serde_json::to_value(Saved {
                playback_mode: PlaybackMode::Synchronized,
                ..saved
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(restored.playback_mode, PlaybackMode::Synchronized);
    }
    #[test]
    fn expired_or_disabled_pairing_window_never_returns_a_stale_pin() {
        let mut state = EndpointState::new(
            PathBuf::from(".local/tmp/pin-window-test"),
            "127.0.0.1:0".parse().unwrap(),
        );
        let now = Instant::now();
        state.status.enabled = true;
        state.status.ready = true;
        state.status.pairing_pin = Some("1234".into());
        state.pairing_deadline = Some(now + Duration::from_secs(5));
        let live = state.display_status_at(now);
        assert!(live.pairing_window_open);
        assert_eq!(live.pairing_window_remaining_seconds, 5);
        assert_eq!(live.pairing_pin.as_deref(), Some("1234"));
        let expired = state.display_status_at(now + Duration::from_secs(5));
        assert!(!expired.pairing_window_open);
        assert_eq!(expired.pairing_window_remaining_seconds, 0);
        assert!(expired.pairing_pin.is_none());
        state.status.playback_allowed = false;
        assert!(state.display_status_at(now).pairing_pin.is_none());
        state.status.playback_allowed = true;
        state.stop();
        assert!(state.display_status_at(now).pairing_pin.is_none());
    }
    #[test]
    fn fifth_pairing_challenge_can_finish_but_retry_does_not_renew_attempt_budget() {
        let mut state = EndpointState::new(
            PathBuf::from(".local/tmp/pair-budget-test"),
            "127.0.0.1:0".parse().unwrap(),
        );
        let now = Instant::now();
        state.status.enabled = true;
        state.status.ready = true;
        state.status.pairing_pin = Some("1234".into());
        state.pairing_deadline = Some(now + Duration::from_secs(600));
        state.pairing_attempts = 5;
        state.pairing_in_progress = true;
        assert!(state.display_status_at(now).pairing_window_open);
        state.pairing_in_progress = false;
        assert!(state.display_status_at(now).pairing_pin.is_none());
        state.stop();
        state.status.enabled = true;
        state.status.ready = true;
        assert_eq!(state.pairing_attempts, 5);
        assert_eq!(state.pairing_deadline, Some(now + Duration::from_secs(600)));
        assert!(!state.display_status_at(now).pairing_window_open);
    }
    fn pcm_fixture() -> PcmPacket {
        PcmPacket {
            header: neonmix_airplay_ipc::PcmHeader {
                flags: 0,
                session_id: 1,
                stream_id: 2,
                stream_epoch: 1,
                format_epoch: 1,
                sequence: 1,
                normalized_sample_position: 0,
                source_sample_position: 0,
                presentation_time_ns: 1,
                mapping_id: 1,
                uncertainty_ns: 0,
                source_rate: 44100,
                frame_count: 1,
                protocol_gain: 1.0,
                gain_applied: false,
            },
            samples: vec![0.0; 2],
        }
    }
    #[test]
    fn bounded_media_backpressure_preserves_burst_packets_and_stop_remains_independent() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.try_send(pcm_fixture()).unwrap();
        let consumer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(35));
            assert_eq!(rx.recv().unwrap().header.sequence, 1);
            assert_eq!(rx.recv().unwrap().header.sequence, 2);
        });
        let mut packet = pcm_fixture();
        packet.header.sequence = 2;
        queue_media_packet(&tx, packet, &AtomicBool::new(false)).unwrap();
        consumer.join().unwrap();
        let (tx, _stalled) = mpsc::sync_channel(1);
        tx.try_send(pcm_fixture()).unwrap();
        let start = Instant::now();
        queue_media_packet(&tx, pcm_fixture(), &AtomicBool::new(true)).unwrap();
        assert!(start.elapsed() < Duration::from_millis(50));
    }
    #[test]
    fn stop_and_disconnect_fail_closed_even_when_control_consumer_stalls() {
        let mut state = EndpointState::new(
            PathBuf::from(".local/tmp/airplay-control-test"),
            "127.0.0.1:0".parse().unwrap(),
        );
        let (tx, _stalled) = mpsc::sync_channel(1);
        tx.try_send(Action::Allow)
            .unwrap_or_else(|_| panic!("initial control capacity"));
        state.commands = Some(tx);
        state.gate.store(true, Release);
        state.status.enabled = true;
        state.status.active = true;
        state.disconnect();
        assert!(!state.gate.load(Acquire));
        assert!(state.shutdown.load(Acquire));
        assert!(!state.status.playback_allowed);
        state.stop();
        assert!(!state.status.enabled);
        assert!(state.shutdown.load(Acquire));
    }
    #[test]
    fn idle_coreaudio_epoch_change_keeps_pin_window_but_active_media_is_revoked() {
        let mut state = EndpointState::new(
            PathBuf::from(".local/tmp/airplay-epoch-test"),
            "127.0.0.1:0".parse().unwrap(),
        );
        let (tx, rx) = mpsc::sync_channel(8);
        state.commands = Some(tx);
        state.status.enabled = true;
        state.output_epoch_changed();
        assert!(state.status.playback_allowed);
        assert!(rx.try_recv().is_err());
        state.status.active = true;
        state.gate.store(true, Release);
        let before = state.status.revision;
        state.output_epoch_changed();
        assert!(!state.status.active);
        assert!(!state.gate.load(Acquire));
        assert!(state.status.playback_allowed);
        assert!(state.status.revision > before);
        assert!(matches!(rx.try_recv(), Ok(Action::Reset(_))));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn media_transport_rejects_authenticated_truncation_and_oversized_frame() {
        for bytes in [vec![b'a'; 3], {
            let mut bytes = vec![b'a'; 64];
            bytes.push(b'\n');
            let mut header = vec![0u8; HEADER_BYTES];
            header[8..12].copy_from_slice(&3841u32.to_le_bytes());
            bytes.extend(header);
            bytes
        }] {
            let (stream, mut sender) = std::os::unix::net::UnixStream::pair().unwrap();
            let worker = std::thread::spawn(move || {
                std::io::Write::write_all(&mut sender, &bytes).unwrap();
            });
            let (tx, _) = mpsc::sync_channel(1);
            assert!(
                read_media(
                    stream,
                    &"a".repeat(64),
                    tx,
                    &AtomicBool::new(false),
                    &AtomicBool::new(false)
                )
                .is_err()
            );
            worker.join().unwrap();
        }
    }
    #[test]
    fn full_grant_preserves_all_hub_identity_fields() {
        let context = Context {
            session_id: 19,
            stream_id: 23,
            stream_epoch: 41,
            format_epoch: 43,
            mapping_id: 47,
        };
        let decoded: neonmix_airplay_ipc::HubCommand = serde_json::from_value(grant(
            context,
            &Owner {
                receiver: Uuid::new_v4(),
                generation: 1,
                connection: 2,
                request: 3,
                source: "test".into(),
            },
        ))
        .unwrap();
        let encoded = serde_json::to_value(decoded).unwrap();
        assert_eq!(encoded["session_id"], 19);
        assert_eq!(encoded["stream_id"], 23);
        assert_eq!(encoded["stream_epoch"], 41);
        assert_eq!(encoded["format_epoch"], 43);
        assert_eq!(encoded["mapping_id"], 47);
    }
}
