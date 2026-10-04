//! Authoritative single-room model. Authentication is supplied by the TLS server,
//! never by a client-provided role. No audio or network work occurs here.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use uuid::Uuid;

pub const MAX_STREAMS: usize = 16;
pub const EVENT_CAPACITY: usize = 256;
pub const PROTOCOL_VERSION: u16 = 1;
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
    pub hub_id: Uuid,
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
        if event.hub_id != self.hub_id || self.revision.checked_add(1) != Some(event.revision) {
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
    pub request_id: Uuid,
    pub expected_revision: u64,
    pub operation: Operation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub revision: u64,
    pub session_id: Option<Uuid>,
    pub stream_id: Option<u64>,
    pub device_id: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
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
    original_available: bool,
    changes: Vec<DeferredChange>,
}
#[derive(Clone)]
enum DeferredChange {
    Session(Uuid, SessionStatus),
    Output(bool),
}
pub struct PreparedCommand {
    token: Uuid,
    next: Box<Authority>,
    receipt: Receipt,
}
impl PreparedCommand {
    pub fn token(&self) -> Uuid {
        self.token
    }
    pub fn snapshot(&self) -> &Snapshot {
        self.next.current()
    }
    pub fn persistent(&self) -> PersistentState {
        self.next.persistent()
    }
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
}
/// No media keys, RTP counters, buffers, PCM or live sessions are persisted.
#[derive(Serialize, Deserialize)]
pub struct PersistentState {
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
                hub_id: Uuid::new_v4(),
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
        let id = Uuid::new_v4();
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
                hub_id: saved.hub_id,
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
        if self.pending.is_some() {
            return Err(ControlError::Busy);
        }
        let live = self
            .state
            .devices
            .get(&principal.device)
            .ok_or(ControlError::Unauthenticated)?;
        if live.revoked || live.role != principal.role {
            return Err(ControlError::Unauthenticated);
        }
        if let Some((_, previous, receipt)) = self
            .receipts
            .iter()
            .find(|(id, c, _)| *id == principal.device && c.request_id == command.request_id)
        {
            return if previous == &command {
                Ok(receipt.clone())
            } else {
                Err(ControlError::IdempotencyConflict)
            };
        }
        if command.expected_revision != self.state.revision {
            return Err(ControlError::RevisionConflict);
        }
        let mut next = self.state.clone();
        let mut receipt = Receipt {
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
        for stream in next.streams.values() {
            preferences.insert(
                stream.device_id,
                Mix {
                    solo: false,
                    ..stream.mix
                },
            );
        }
        let saved = PersistentState {
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
    /// Other durable mutations return Busy; health updates are retained.
    pub fn prepare_transaction(
        &mut self,
        principal: Principal,
        command: Command,
    ) -> Result<PreparedCommand, ControlError> {
        if self.pending.is_some() {
            return Err(ControlError::Busy);
        }
        let mut next = self.clone();
        let receipt = next.execute(principal, command, |_| Ok(()))?;
        let token = Uuid::new_v4();
        self.pending = Some(PendingTransaction {
            token,
            original_available: self.state.output.available,
            changes: Vec::new(),
        });
        Ok(PreparedCommand {
            token,
            next: Box::new(next),
            receipt,
        })
    }
    pub fn commit_transaction(
        &mut self,
        prepared: PreparedCommand,
    ) -> Result<Receipt, ControlError> {
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.token != prepared.token)
        {
            return Err(ControlError::Busy);
        }
        let pending = self.pending.take().unwrap();
        *self = *prepared.next;
        self.replay_health(pending);
        Ok(prepared.receipt)
    }
    pub fn abort_transaction(&mut self, token: Uuid) -> Result<(), ControlError> {
        if self.pending.as_ref().is_none_or(|p| p.token != token) {
            return Err(ControlError::Busy);
        }
        let pending = self.pending.take().unwrap();
        self.state.output.available = pending.original_available;
        self.replay_health(pending);
        Ok(())
    }
    fn replay_health(&mut self, pending: PendingTransaction) {
        for change in pending.changes {
            match change {
                DeferredChange::Session(id, status) => {
                    let _ = self.set_session_status(id, status);
                }
                DeferredChange::Output(available) => {
                    let _ = self.set_output_available(available);
                }
            }
        }
    }
    pub fn set_session_status(
        &mut self,
        id: Uuid,
        status: SessionStatus,
    ) -> Result<(), ControlError> {
        if let Some(pending) = self.pending.as_mut() {
            if !self.state.sessions.contains_key(&id) {
                return Err(ControlError::NotFound);
            }
            pending
                .changes
                .retain(|c| !matches!(c, DeferredChange::Session(existing, _) if *existing == id));
            pending.changes.push(DeferredChange::Session(id, status));
            return Ok(());
        }
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
        if let Some(pending) = self.pending.as_mut() {
            // Availability is a live safety observation even while durable state
            // is frozen; callers must reject new admission immediately.
            self.state.output.available = available;
            pending
                .changes
                .retain(|c| !matches!(c, DeferredChange::Output(_)));
            pending.changes.push(DeferredChange::Output(available));
            return Ok(());
        }
        if self.state.output.available == available {
            return Ok(());
        }
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
        self.record_event();
        Ok(())
    }
    fn record_event(&mut self) {
        if self.events.len() == EVENT_CAPACITY {
            self.events.pop_front();
        }
        let previous = self.last_published.as_ref();
        self.events.push_back(Event {
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
