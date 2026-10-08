//! Authoritative single-room model. Authentication is supplied by the TLS server,
//! never by a client-provided role. No audio or network work occurs here.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use uuid::Uuid;

pub const MAX_STREAMS: usize = 16;
pub const EVENT_CAPACITY: usize = 256;
pub const PROTOCOL_VERSION: u16 = 1;
pub const CONTROL_VERSION: u16 = 2;
fn legacy_control_version() -> u16 {
    1
}
fn is_legacy_control_version(version: &u16) -> bool {
    *version == 1
}
pub const MEDIA_TTL_SECONDS: u32 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Controller,
    Admin,
}
#[derive(Debug, Clone, Copy)]
pub struct Principal {
    device: Uuid,
    role: Role,
}
impl Principal {
    pub fn device_id(self) -> Uuid {
        self.device
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: Uuid,
    pub name: String,
    pub role: Role,
    pub revoked: bool,
    pub playback_allowed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Buffering,
    Playing,
    NetworkDegraded,
    UserStopped,
    AdminDisconnected,
    NetworkInterrupted,
    Revoked,
    OutputLost,
}
impl SessionStatus {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Buffering | Self::Playing | Self::NetworkDegraded | Self::OutputLost
        )
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaOffer {
    pub version: u16,
    pub codec: String,
    pub rate: u32,
    pub channels: u16,
    pub packet_frames: u16,
    pub payload_type: u8,
    pub ssrc: u32,
    pub stream_epoch: u64,
    pub udp_port: u16,
    /// SHA-256 of the DER certificate, lower-case hex; supplied over authenticated TLS.
    pub certificate_sha256: String,
}
impl MediaOffer {
    pub fn validate(&self) -> Result<(), ControlError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ControlError::IncompatibleVersion);
        }
        if self.codec != "opus"
            || self.rate != 48_000
            || self.channels != 2
            || self.packet_frames != 480
            || self.payload_type != 96
            || self.ssrc == 0
            || self.stream_epoch == 0
            || self.udp_port == 0
            || self.certificate_sha256.len() != 64
            || !self
                .certificate_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ControlError::InvalidArgument);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub created_revision: u64,
    pub media_ttl_seconds: u32,
    pub id: Uuid,
    pub device_id: Uuid,
    pub stream_id: u64,
    /// Unique for every negotiation. Never reuse a DTLS connection or its SRTP keys.
    pub media_context: Uuid,
    pub offer: MediaOffer,
    pub status: SessionStatus,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mix {
    pub gain_db: f32,
    pub muted: bool,
    pub solo: bool,
}
impl Default for Mix {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            muted: false,
            solo: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    pub id: u64,
    pub device_id: Uuid,
    pub session_id: Uuid,
    pub mix: Mix,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub id: String,
    pub available: bool,
    pub gain_db: f32,
    pub muted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default = "legacy_control_version")]
    pub control_version: u16,
    #[serde(default)]
    pub config_revision: u64,
    #[serde(default)]
    pub event_sequence: u64,
    pub hub_id: Uuid,
    /// One Hub process lifetime; absent on legacy snapshots.
    #[serde(default)]
    pub runtime_epoch: Uuid,
    pub revision: u64,
    pub bus_id: String,
    pub devices: BTreeMap<Uuid, Device>,
    pub sessions: BTreeMap<Uuid, Session>,
    pub streams: BTreeMap<u64, Stream>,
    pub output: Output,
}
impl Snapshot {
    /// Apply an authenticated event atomically. Never show a partial state when
    /// a connection loses an event or switches Hub identity.
    pub fn apply_event(&mut self, event: Event) -> Result<(), ControlError> {
        if !matches!(self.control_version, 1 | CONTROL_VERSION)
            || (self.control_version == CONTROL_VERSION && self.runtime_epoch.is_nil())
            || self.control_version != event.control_version
            || (self.control_version == CONTROL_VERSION
                && (self.event_sequence != self.revision
                    || event.event_sequence != event.revision
                    || self.event_sequence.checked_add(1) != Some(event.event_sequence)
                    || event.config_revision < self.config_revision
                    || event.config_revision > self.config_revision.saturating_add(1)))
            || event.runtime_epoch != self.runtime_epoch
            || event.hub_id != self.hub_id
            || self.revision.checked_add(1) != Some(event.revision)
        {
            return Err(ControlError::SnapshotRequired);
        }
        let mut next = self.clone();
        for device in event.devices {
            next.devices.insert(device.id, device);
        }
        for session in event.sessions {
            next.sessions.insert(session.id, session);
        }
        for stream in event.streams {
            next.streams.insert(stream.id, stream);
        }
        for id in event.removed_streams {
            next.streams.remove(&id);
        }
        for id in event.removed_sessions {
            next.sessions.remove(&id);
        }
        if let Some(output) = event.output {
            next.output = output;
        }
        if next.devices.len() > 64 || next.sessions.len() > 256 || next.streams.len() > MAX_STREAMS
        {
            return Err(ControlError::QuotaExceeded);
        }
        next.revision = event.revision;
        next.config_revision = event.config_revision;
        next.event_sequence = event.event_sequence;
        *self = next;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    RegisterDevice {
        name: String,
        role: Role,
        token_sha256: String,
    },
    Start {
        offer: MediaOffer,
    },
    Stop {
        session_id: Uuid,
    },
    StreamMix {
        stream_id: u64,
        gain_db: Option<f32>,
        muted: Option<bool>,
        solo: Option<bool>,
    },
    OutputMix {
        gain_db: Option<f32>,
        muted: Option<bool>,
    },
    Disconnect {
        device_id: Uuid,
    },
    AllowPlayback {
        device_id: Uuid,
    },
    Revoke {
        device_id: Uuid,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    #[serde(
        default = "legacy_control_version",
        skip_serializing_if = "is_legacy_control_version"
    )]
    pub control_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_config_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_event_sequence: Option<u64>,
    pub request_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_epoch: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    pub operation: Operation,
}
impl Operation {
    fn config_condition(&self) -> bool {
        match self {
            Self::Start { .. } | Self::Stop { .. } => false,
            Self::StreamMix { gain_db, muted, .. } => gain_db.is_some() || muted.is_some(),
            _ => true,
        }
    }
    fn runtime_condition(&self) -> bool {
        matches!(
            self,
            Self::Start { .. }
                | Self::Stop { .. }
                | Self::StreamMix { .. }
                | Self::Disconnect { .. }
                | Self::Revoke { .. }
        )
    }
}
impl Command {
    pub fn bound(state: &Snapshot, credential: Uuid, operation: Operation) -> Self {
        let current = state.control_version == CONTROL_VERSION;
        Self {
            control_version: state.control_version,
            expected_config_revision: if current && operation.config_condition() {
                Some(state.config_revision)
            } else {
                None
            },
            expected_event_sequence: if current && operation.runtime_condition() {
                Some(state.event_sequence)
            } else {
                None
            },
            request_id: Uuid::new_v4(),
            runtime_epoch: Some(state.runtime_epoch),
            credential_id: Some(credential),
            expected_revision: if current { None } else { Some(state.revision) },
            operation,
        }
    }
    fn validate_version(&self) -> Result<(), ControlError> {
        match self.control_version {
            1 if self.expected_revision.is_some()
                && self.expected_config_revision.is_none()
                && self.expected_event_sequence.is_none() =>
            {
                Ok(())
            }
            CONTROL_VERSION
                if self.expected_revision.is_none()
                    && self.runtime_epoch.is_some_and(|e| !e.is_nil())
                    && self.credential_id.is_some_and(|id| !id.is_nil())
                    && (!self.operation.config_condition()
                        || self.expected_config_revision.is_some())
                    && (!self.operation.runtime_condition()
                        || self.expected_event_sequence.is_some()) =>
            {
                Ok(())
            }
            CONTROL_VERSION => Err(ControlError::UpgradeRequired),
            1 => Err(ControlError::UpgradeRequired),
            _ => Err(ControlError::IncompatibleVersion),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    #[serde(default)]
    pub config_revision: u64,
    #[serde(default)]
    pub event_sequence: u64,
    pub revision: u64,
    pub session_id: Option<Uuid>,
    pub stream_id: Option<u64>,
    pub device_id: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default = "legacy_control_version")]
    pub control_version: u16,
    #[serde(default)]
    pub config_revision: u64,
    #[serde(default)]
    pub event_sequence: u64,
    #[serde(default)]
    pub runtime_epoch: Uuid,
    pub revision: u64,
    pub hub_id: Uuid,
    pub devices: Vec<Device>,
    pub sessions: Vec<Session>,
    pub streams: Vec<Stream>,
    pub removed_streams: Vec<u64>,
    pub removed_sessions: Vec<Uuid>,
    pub output: Option<Output>,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("revision_conflict")]
    RevisionConflict,
    #[error("invalid_argument")]
    InvalidArgument,
    #[error("upgrade_required")]
    UpgradeRequired,
    #[error("incompatible_version")]
    IncompatibleVersion,
    #[error("not_found")]
    NotFound,
    #[error("playback_blocked")]
    PlaybackBlocked,
    #[error("quota_exceeded")]
    QuotaExceeded,
    #[error("already_active")]
    AlreadyActive,
    #[error("idempotency_conflict")]
    IdempotencyConflict,
    #[error("snapshot_required")]
    SnapshotRequired,
    #[error("busy")]
    Busy,
    #[error("profile_durability_unconfirmed")]
    DurabilityUnconfirmed,
}

pub fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
#[derive(Clone)]
pub struct Authority {
    last_published: Option<Snapshot>,
    state: Snapshot,
    credentials: BTreeMap<[u8; 32], Uuid>,
    events: VecDeque<Event>,
    receipts: VecDeque<(Uuid, Command, Receipt)>,
    preferences: BTreeMap<Uuid, Mix>,
    pending: Option<PendingTransaction>,
}
#[derive(Clone)]
struct PendingTransaction {
    token: Uuid,
}
pub struct PreparedCommand {
    token: Uuid,
    changes: Box<PreparedChanges>,
    receipt: Receipt,
}
struct PreparedChanges {
    base: Snapshot,
    candidate: Snapshot,
    credentials: BTreeMap<[u8; 32], Uuid>,
    preferences: BTreeMap<Uuid, Mix>,
    principal: Uuid,
    command: Command,
}
/// A completed request never becomes a new transaction. Owners must return
/// the stored response before reserving resources or calling external code.
pub enum Preparation {
    Replay(Receipt),
    Prepared(PreparedCommand),
}
impl PreparedCommand {
    pub fn token(&self) -> Uuid {
        self.token
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.changes.candidate
    }
    pub fn persistent(&self) -> PersistentState {
        PersistentState {
            config_revision: self.changes.candidate.config_revision,
            version: PROTOCOL_VERSION,
            hub_id: self.changes.candidate.hub_id,
            revision: self.changes.candidate.revision,
            devices: self.changes.candidate.devices.clone(),
            credentials: self
                .changes
                .credentials
                .iter()
                .map(|(hash, id)| (*hash, *id))
                .collect(),
            preferences: self.changes.preferences.clone(),
            output: Output {
                available: true,
                ..self.changes.candidate.output.clone()
            },
        }
    }
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
}
/// No media keys, RTP counters, buffers, PCM or live sessions are persisted.
#[derive(Serialize, Deserialize)]
pub struct PersistentState {
    #[serde(default)]
    pub config_revision: u64,
    pub version: u16,
    pub hub_id: Uuid,
    pub revision: u64,
    pub devices: BTreeMap<Uuid, Device>,
    pub credentials: Vec<([u8; 32], Uuid)>,
    pub preferences: BTreeMap<Uuid, Mix>,
    pub output: Output,
}
impl Authority {
    pub fn new(
        output_id: String,
        admin_name: String,
        admin_token: &str,
    ) -> Result<Self, ControlError> {
        if admin_token.len() < 32 || output_id.is_empty() {
            return Err(ControlError::InvalidArgument);
        }
        let mut this = Self {
            last_published: None,
            state: Snapshot {
                control_version: CONTROL_VERSION,
                config_revision: 0,
                event_sequence: 0,
                hub_id: Uuid::new_v4(),
                runtime_epoch: Uuid::new_v4(),
                revision: 0,
                bus_id: "main".into(),
                devices: BTreeMap::new(),
                sessions: BTreeMap::new(),
                streams: BTreeMap::new(),
                output: Output {
                    id: output_id,
                    available: true,
                    gain_db: -12.0,
                    muted: false,
                },
            },
            credentials: BTreeMap::new(),
            events: VecDeque::with_capacity(EVENT_CAPACITY),
            receipts: VecDeque::with_capacity(128),
            preferences: BTreeMap::new(),
            pending: None,
        };
        this.add_device(admin_name, Role::Admin, admin_token)?;
        Ok(this)
    }
    /// Provisioned by an administrator on the server. Initial trust/discovery UI is E05.
    pub fn add_device(
        &mut self,
        name: String,
        role: Role,
        token: &str,
    ) -> Result<Uuid, ControlError> {
        if self.pending.is_some() {
            return Err(ControlError::Busy);
        }
        if name.is_empty() || name.len() > 128 || token.len() < 32 {
            return Err(ControlError::InvalidArgument);
        }
        if self.state.devices.len() >= 64 {
            return Err(ControlError::QuotaExceeded);
        }
        let digest = token_digest(token);
        if self.credentials.contains_key(&digest) {
            return Err(ControlError::InvalidArgument);
        }
        let config_revision = self
            .state
            .config_revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        self.state
            .revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        let id = Uuid::new_v4();
        self.state.config_revision = config_revision;
        self.state.devices.insert(
            id,
            Device {
                id,
                name,
                role,
                revoked: false,
                playback_allowed: true,
            },
        );
        self.credentials.insert(digest, id);
        self.publish()?;
        Ok(id)
    }
    pub fn authenticate(&self, token: &str) -> Result<Principal, ControlError> {
        let id = self
            .credentials
            .get(&token_digest(token))
            .ok_or(ControlError::Unauthenticated)?;
        let device = self
            .state
            .devices
            .get(id)
            .ok_or(ControlError::Unauthenticated)?;
        if device.revoked {
            return Err(ControlError::Unauthenticated);
        }
        Ok(Principal {
            device: *id,
            role: device.role,
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.clone()
    }
    pub fn current(&self) -> &Snapshot {
        &self.state
    }
    pub fn persistent(&self) -> PersistentState {
        PersistentState {
            config_revision: self.state.config_revision,
            version: PROTOCOL_VERSION,
            hub_id: self.state.hub_id,
            revision: self.state.revision,
            devices: self.state.devices.clone(),
            credentials: self
                .credentials
                .iter()
                .map(|(digest, id)| (*digest, *id))
                .collect(),
            preferences: self.preferences.clone(),
            output: Output {
                available: true,
                ..self.state.output.clone()
            },
        }
    }
    pub fn restore(saved: PersistentState) -> Result<Self, ControlError> {
        if saved.version != PROTOCOL_VERSION {
            return Err(ControlError::IncompatibleVersion);
        }
        if saved.devices.len() > 64
            || saved.credentials.len() > 64
            || saved.preferences.len() > 64
            || !saved
                .devices
                .values()
                .any(|d| d.role == Role::Admin && !d.revoked)
            || saved
                .credentials
                .iter()
                .any(|(_, id)| !saved.devices.contains_key(id))
            || saved
                .devices
                .iter()
                .any(|(id, d)| *id != d.id || d.name.is_empty() || d.name.len() > 128)
            || saved
                .preferences
                .iter()
                .any(|(id, m)| !saved.devices.contains_key(id) || validate_gain(m.gain_db).is_err())
        {
            return Err(ControlError::InvalidArgument);
        }
        validate_gain(saved.output.gain_db)?;
        let mut this = Self {
            last_published: None,
            state: Snapshot {
                control_version: CONTROL_VERSION,
                config_revision: if saved.config_revision == 0 {
                    saved.revision
                } else {
                    saved.config_revision
                },
                event_sequence: saved.revision,
                hub_id: saved.hub_id,
                runtime_epoch: Uuid::new_v4(),
                revision: saved.revision,
                bus_id: "main".into(),
                devices: saved.devices,
                sessions: BTreeMap::new(),
                streams: BTreeMap::new(),
                output: saved.output,
            },
            credentials: saved.credentials.into_iter().collect(),
            events: VecDeque::with_capacity(EVENT_CAPACITY),
            receipts: VecDeque::with_capacity(128),
            preferences: saved.preferences,
            pending: None,
        };
        this.publish()?;
        Ok(this)
    }
    pub fn events_after_in_runtime(
        &self,
        epoch: Uuid,
        sequence: u64,
    ) -> Result<Vec<Event>, ControlError> {
        if epoch.is_nil() || epoch != self.state.runtime_epoch {
            return Err(ControlError::SnapshotRequired);
        }
        self.events_after(sequence)
    }
    pub fn events_after(&self, revision: u64) -> Result<Vec<Event>, ControlError> {
        if revision > self.state.revision
            || self
                .events
                .front()
                .is_some_and(|e| revision < e.revision.saturating_sub(1))
        {
            return Err(ControlError::SnapshotRequired);
        }
        Ok(self
            .events
            .iter()
            .filter(|e| e.revision > revision)
            .cloned()
            .collect())
    }
    /// Caller prepares the new DSP/media state before commit. If the bounded
    /// command queue is full, return Busy and keep the old authoritative state.
    pub fn execute(
        &mut self,
        principal: Principal,
        command: Command,
        prepare: impl FnOnce(&Snapshot) -> Result<(), ControlError>,
    ) -> Result<Receipt, ControlError> {
        self.execute_durable(principal, command, |state, _| prepare(state))
    }
    pub fn execute_durable(
        &mut self,
        principal: Principal,
        command: Command,
        prepare: impl FnOnce(&Snapshot, &PersistentState) -> Result<(), ControlError>,
    ) -> Result<Receipt, ControlError> {
        if let Some(receipt) = self.replay(principal, &command)? {
            return Ok(receipt);
        }
        if self.pending.is_some() {
            return Err(ControlError::Busy);
        }
        self.execute_new(principal, command, prepare)
    }
    fn replay(
        &self,
        principal: Principal,
        command: &Command,
    ) -> Result<Option<Receipt>, ControlError> {
        let live = self
            .state
            .devices
            .get(&principal.device)
            .ok_or(ControlError::Unauthenticated)?;
        if live.revoked || live.role != principal.role {
            return Err(ControlError::Unauthenticated);
        }
        command.validate_version()?;
        if command
            .credential_id
            .is_some_and(|id| id != principal.device_id())
        {
            return Err(ControlError::Unauthenticated);
        }
        if command
            .runtime_epoch
            .is_some_and(|epoch| epoch != self.state.runtime_epoch)
        {
            return Err(ControlError::SnapshotRequired);
        }
        if let Some((_, previous, receipt)) = self
            .receipts
            .iter()
            .find(|(id, c, _)| *id == principal.device && c.request_id == command.request_id)
        {
            return if previous == command {
                Ok(Some(receipt.clone()))
            } else {
                Err(ControlError::IdempotencyConflict)
            };
        }
        Ok(None)
    }
    fn execute_new(
        &mut self,
        principal: Principal,
        command: Command,
        prepare: impl FnOnce(&Snapshot, &PersistentState) -> Result<(), ControlError>,
    ) -> Result<Receipt, ControlError> {
        let live = self
            .state
            .devices
            .get(&principal.device)
            .ok_or(ControlError::Unauthenticated)?;
        if command.control_version == CONTROL_VERSION {
            if command
                .expected_config_revision
                .is_some_and(|r| r != self.state.config_revision)
                || command
                    .expected_event_sequence
                    .is_some_and(|s| s != self.state.event_sequence)
            {
                return Err(ControlError::RevisionConflict);
            }
        } else if command.expected_revision != Some(self.state.revision) {
            return Err(ControlError::RevisionConflict);
        }
        let mut next = self.state.clone();
        let mut receipt = Receipt {
            config_revision: self.state.config_revision,
            event_sequence: self
                .state
                .revision
                .checked_add(1)
                .ok_or(ControlError::QuotaExceeded)?,
            revision: self
                .state
                .revision
                .checked_add(1)
                .ok_or(ControlError::QuotaExceeded)?,
            session_id: None,
            stream_id: None,
            device_id: None,
        };
        match &command.operation {
            Operation::RegisterDevice {
                name,
                role,
                token_sha256,
            } => {
                if principal.role != Role::Admin {
                    return Err(ControlError::PermissionDenied);
                }
                if name.is_empty()
                    || name.len() > 128
                    || token_sha256.len() != 64
                    || !token_sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(ControlError::InvalidArgument);
                }
                if next.devices.len() >= 64 {
                    return Err(ControlError::QuotaExceeded);
                }
                let digest = parse_digest(token_sha256)?;
                if self.credentials.contains_key(&digest) {
                    return Err(ControlError::InvalidArgument);
                }
                let id = Uuid::new_v4();
                next.devices.insert(
                    id,
                    Device {
                        id,
                        name: name.clone(),
                        role: *role,
                        revoked: false,
                        playback_allowed: true,
                    },
                );
                receipt.device_id = Some(id);
            }
            Operation::Start { offer } => {
                offer.validate()?;
                if !live.playback_allowed {
                    return Err(ControlError::PlaybackBlocked);
                }
                if next
                    .sessions
                    .values()
                    .any(|s| s.device_id == principal.device && s.status.active())
                {
                    return Err(ControlError::AlreadyActive);
                }
                if next.streams.len() >= MAX_STREAMS {
                    return Err(ControlError::QuotaExceeded);
                }
                if next.sessions.len() >= 256 {
                    let old = next
                        .sessions
                        .values()
                        .filter(|s| !s.status.active())
                        .min_by_key(|s| s.created_revision)
                        .map(|s| s.id)
                        .ok_or(ControlError::QuotaExceeded)?;
                    next.sessions.remove(&old);
                }
                let id = Uuid::new_v4();
                let mut stream_id = Uuid::new_v4().as_u128() as u64 & ((1u64 << 53) - 1);
                while stream_id == 0 || next.streams.contains_key(&stream_id) {
                    stream_id = Uuid::new_v4().as_u128() as u64 & ((1u64 << 53) - 1);
                }
                next.sessions.insert(
                    id,
                    Session {
                        created_revision: receipt.revision,
                        media_ttl_seconds: MEDIA_TTL_SECONDS,
                        id,
                        device_id: principal.device,
                        stream_id,
                        media_context: Uuid::new_v4(),
                        offer: offer.clone(),
                        status: SessionStatus::Buffering,
                    },
                );
                next.streams.insert(
                    stream_id,
                    Stream {
                        id: stream_id,
                        device_id: principal.device,
                        session_id: id,
                        mix: self
                            .preferences
                            .get(&principal.device)
                            .copied()
                            .unwrap_or_default(),
                    },
                );
                receipt.session_id = Some(id);
                receipt.stream_id = Some(stream_id);
            }
            Operation::Stop { session_id } => {
                let s = next
                    .sessions
                    .get_mut(session_id)
                    .ok_or(ControlError::NotFound)?;
                if s.device_id != principal.device {
                    return Err(ControlError::PermissionDenied);
                }
                if s.status.active() {
                    s.status = SessionStatus::UserStopped;
                    next.streams.remove(&s.stream_id);
                }
            }
            Operation::StreamMix {
                stream_id,
                gain_db,
                muted,
                solo,
            } => {
                let s = next
                    .streams
                    .get_mut(stream_id)
                    .ok_or(ControlError::NotFound)?;
                if principal.role == Role::Member
                    && (s.device_id != principal.device || solo.is_some())
                {
                    return Err(ControlError::PermissionDenied);
                }
                if let Some(g) = gain_db {
                    validate_gain(*g)?;
                    s.mix.gain_db = *g;
                }
                if let Some(m) = muted {
                    s.mix.muted = *m;
                }
                if let Some(solo) = solo {
                    s.mix.solo = *solo;
                }
            }
            Operation::OutputMix { gain_db, muted } => {
                if principal.role == Role::Member {
                    return Err(ControlError::PermissionDenied);
                }
                if let Some(g) = gain_db {
                    validate_gain(*g)?;
                    next.output.gain_db = *g;
                }
                if let Some(m) = muted {
                    next.output.muted = *m;
                }
            }
            Operation::Disconnect { device_id }
            | Operation::Revoke { device_id }
            | Operation::AllowPlayback { device_id } => {
                if principal.role != Role::Admin {
                    return Err(ControlError::PermissionDenied);
                }
                let last_admin = next
                    .devices
                    .values()
                    .filter(|d| d.role == Role::Admin && !d.revoked)
                    .count()
                    == 1;
                let device = next
                    .devices
                    .get_mut(device_id)
                    .ok_or(ControlError::NotFound)?;
                if matches!(command.operation, Operation::Revoke { .. })
                    && device.role == Role::Admin
                    && !device.revoked
                    && last_admin
                {
                    return Err(ControlError::InvalidArgument);
                }
                match command.operation {
                    Operation::AllowPlayback { .. } => {
                        if device.revoked {
                            return Err(ControlError::PlaybackBlocked);
                        }
                        device.playback_allowed = true;
                    }
                    _ => {
                        device.playback_allowed = false;
                        let revoke = matches!(command.operation, Operation::Revoke { .. });
                        device.revoked |= revoke;
                        for session in next
                            .sessions
                            .values_mut()
                            .filter(|s| s.device_id == *device_id && s.status.active())
                        {
                            session.status = if revoke {
                                SessionStatus::Revoked
                            } else {
                                SessionStatus::AdminDisconnected
                            };
                            next.streams.remove(&session.stream_id);
                        }
                    }
                }
            }
        }
        next.revision = receipt.revision;
        let mut credentials = self.credentials.clone();
        if let Operation::RegisterDevice { token_sha256, .. } = &command.operation {
            credentials.insert(
                parse_digest(token_sha256)?,
                receipt.device_id.ok_or(ControlError::InvalidArgument)?,
            );
        }
        let mut preferences = self.preferences.clone();
        if let Operation::StreamMix {
            stream_id,
            gain_db,
            muted,
            ..
        } = &command.operation
            && (gain_db.is_some() || muted.is_some())
        {
            let stream = &next.streams[stream_id];
            preferences.insert(
                stream.device_id,
                Mix {
                    solo: false,
                    ..stream.mix
                },
            );
        }
        let config_changed = next.devices != self.state.devices
            || credentials != self.credentials
            || preferences != self.preferences
            || next.output.id != self.state.output.id
            || next.output.gain_db != self.state.output.gain_db
            || next.output.muted != self.state.output.muted;
        next.config_revision = if config_changed {
            self.state
                .config_revision
                .checked_add(1)
                .ok_or(ControlError::QuotaExceeded)?
        } else {
            self.state.config_revision
        };
        next.event_sequence = next.revision;
        receipt.config_revision = next.config_revision;
        let saved = PersistentState {
            config_revision: next.config_revision,
            version: PROTOCOL_VERSION,
            hub_id: next.hub_id,
            revision: next.revision,
            devices: next.devices.clone(),
            credentials: credentials.iter().map(|(hash, id)| (*hash, *id)).collect(),
            preferences: preferences.clone(),
            output: Output {
                available: true,
                ..next.output.clone()
            },
        };
        prepare(&next, &saved)?;
        self.state = next;
        self.credentials = credentials;
        self.preferences = preferences;
        self.record_event();
        if self.receipts.len() == 128 {
            self.receipts.pop_front();
        }
        self.receipts
            .push_back((principal.device, command, receipt.clone()));
        Ok(receipt)
    }
    /// Stage a durable command while holding only the short control mutex.
    /// The owner must commit or abort the returned token after external work.
    /// Other durable mutations return Busy; live health still publishes immediately.
    pub fn transaction_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn prepare_transaction(
        &mut self,
        principal: Principal,
        command: Command,
    ) -> Result<Preparation, ControlError> {
        if let Some(receipt) = self.replay(principal, &command)? {
            return Ok(Preparation::Replay(receipt));
        }
        if self.pending.is_some() {
            return Err(ControlError::Busy);
        }
        let mut next = self.clone();
        let receipt = next.execute(principal, command.clone(), |_| Ok(()))?;
        let token = Uuid::new_v4();
        self.pending = Some(PendingTransaction { token });
        Ok(Preparation::Prepared(PreparedCommand {
            token,
            changes: Box::new(PreparedChanges {
                base: self.state.clone(),
                candidate: next.state,
                credentials: next.credentials,
                preferences: next.preferences,
                principal: principal.device_id(),
                command,
            }),
            receipt,
        }))
    }
    pub fn commit_transaction(
        &mut self,
        prepared: PreparedCommand,
    ) -> Result<Receipt, ControlError> {
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.token != prepared.token)
            || prepared.changes.base.hub_id != self.state.hub_id
            || prepared.changes.base.runtime_epoch != self.state.runtime_epoch
        {
            return Err(ControlError::Busy);
        }
        let revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        let PreparedCommand {
            changes,
            mut receipt,
            ..
        } = prepared;
        let PreparedChanges {
            base,
            candidate,
            credentials,
            preferences,
            principal,
            command,
        } = *changes;
        // Merge only this transaction's changes into the latest runtime. Health
        // events and terminal sessions published during fsync are never restored
        // from the preparation snapshot.
        self.state.config_revision = candidate.config_revision;
        self.state.devices = candidate.devices;
        self.credentials = credentials;
        self.preferences = preferences;
        self.state.output.id = candidate.output.id;
        self.state.output.gain_db = candidate.output.gain_db;
        self.state.output.muted = candidate.output.muted;
        for id in base.sessions.keys() {
            if !candidate.sessions.contains_key(id) {
                self.state.sessions.remove(id);
            }
        }
        for (id, mut session) in candidate.sessions {
            match base.sessions.get(&id) {
                None => {
                    session.created_revision = revision;
                    if !self.state.output.available {
                        session.status = SessionStatus::OutputLost;
                    }
                    self.state.sessions.insert(id, session);
                }
                Some(previous) if previous != &session => {
                    if let Some(current) = self.state.sessions.get_mut(&id)
                        && current.status.active()
                    {
                        current.status = session.status;
                    }
                }
                _ => {}
            }
        }
        for id in base.streams.keys() {
            if !candidate.streams.contains_key(id) {
                self.state.streams.remove(id);
            }
        }
        for (id, stream) in candidate.streams {
            if base.streams.get(&id) != Some(&stream)
                && self
                    .state
                    .sessions
                    .get(&stream.session_id)
                    .is_some_and(|s| s.status.active())
            {
                self.state.streams.insert(id, stream);
            }
        }
        self.pending = None;
        self.state.revision = revision;
        self.state.event_sequence = revision;
        self.record_event();
        receipt.revision = revision;
        receipt.event_sequence = revision;
        receipt.config_revision = self.state.config_revision;
        if self.receipts.len() == 128 {
            self.receipts.pop_front();
        }
        self.receipts
            .push_back((principal, command, receipt.clone()));
        Ok(receipt)
    }
    pub fn abort_transaction(&mut self, token: Uuid) -> Result<(), ControlError> {
        if self.pending.as_ref().is_none_or(|p| p.token != token) {
            return Err(ControlError::Busy);
        }
        self.pending = None;
        Ok(())
    }
    fn check_health_sequence(&self) -> Result<(), ControlError> {
        // Keep one event slot in the numeric sequence reserved for a prepared
        // command. This bound is about u64 exhaustion, not the event ring size.
        self.state
            .revision
            .checked_add(if self.pending.is_some() { 2 } else { 1 })
            .ok_or(ControlError::QuotaExceeded)
            .map(|_| ())
    }
    pub fn set_session_status(
        &mut self,
        id: Uuid,
        status: SessionStatus,
    ) -> Result<(), ControlError> {
        let sequence_capacity = self.check_health_sequence();
        let s = self
            .state
            .sessions
            .get_mut(&id)
            .ok_or(ControlError::NotFound)?;
        if !s.status.active() {
            return Err(ControlError::PlaybackBlocked);
        }
        if s.status == status {
            return Ok(());
        }
        sequence_capacity?;
        self.state
            .revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        s.status = status;
        if !status.active() {
            self.state.streams.remove(&s.stream_id);
        }
        self.publish()
    }
    pub fn set_output_available(&mut self, available: bool) -> Result<(), ControlError> {
        if self.state.output.available == available {
            return Ok(());
        }
        self.check_health_sequence()?;
        self.state
            .revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        self.state.output.available = available;
        for s in self
            .state
            .sessions
            .values_mut()
            .filter(|s| s.status.active())
        {
            s.status = if available {
                SessionStatus::Buffering
            } else {
                SessionStatus::OutputLost
            };
        }
        self.publish()
    }
    fn publish(&mut self) -> Result<(), ControlError> {
        self.state.revision = self
            .state
            .revision
            .checked_add(1)
            .ok_or(ControlError::QuotaExceeded)?;
        self.state.event_sequence = self.state.revision;
        self.record_event();
        Ok(())
    }
    fn record_event(&mut self) {
        if self.events.len() == EVENT_CAPACITY {
            self.events.pop_front();
        }
        let previous = self.last_published.as_ref();
        self.events.push_back(Event {
            control_version: CONTROL_VERSION,
            config_revision: self.state.config_revision,
            event_sequence: self.state.event_sequence,
            runtime_epoch: self.state.runtime_epoch,
            revision: self.state.revision,
            hub_id: self.state.hub_id,
            devices: self
                .state
                .devices
                .iter()
                .filter(|(id, d)| previous.and_then(|p| p.devices.get(id)) != Some(*d))
                .map(|(_, d)| d.clone())
                .collect(),
            sessions: self
                .state
                .sessions
                .iter()
                .filter(|(id, s)| previous.and_then(|p| p.sessions.get(id)) != Some(*s))
                .map(|(_, s)| s.clone())
                .collect(),
            streams: self
                .state
                .streams
                .iter()
                .filter(|(id, s)| previous.and_then(|p| p.streams.get(id)) != Some(*s))
                .map(|(_, s)| s.clone())
                .collect(),
            removed_streams: previous.map_or_else(Vec::new, |p| {
                p.streams
                    .keys()
                    .filter(|id| !self.state.streams.contains_key(id))
                    .copied()
                    .collect()
            }),
            removed_sessions: previous.map_or_else(Vec::new, |p| {
                p.sessions
                    .keys()
                    .filter(|id| !self.state.sessions.contains_key(id))
                    .copied()
                    .collect()
            }),
            output: if previous.is_none_or(|p| p.output != self.state.output) {
                Some(self.state.output.clone())
            } else {
                None
            },
        });
        self.last_published = Some(self.state.clone());
    }
}
fn validate_gain(g: f32) -> Result<(), ControlError> {
    if g.is_finite() && (-96.0..=12.0).contains(&g) {
        Ok(())
    } else {
        Err(ControlError::InvalidArgument)
    }
}
fn parse_digest(hex: &str) -> Result<[u8; 32], ControlError> {
    let mut digest = [0u8; 32];
    for (i, bytes) in hex.as_bytes().chunks_exact(2).enumerate() {
        if i >= 32 {
            return Err(ControlError::InvalidArgument);
        }
        digest[i] = u8::from_str_radix(
            std::str::from_utf8(bytes).map_err(|_| ControlError::InvalidArgument)?,
            16,
        )
        .map_err(|_| ControlError::InvalidArgument)?;
    }
    Ok(digest)
}
