//! Bounded field intentions. The queue owns identities; a selected UI row does
//! not own a request. Only the in-flight envelope can be retried for Unknown.
use super::{Message, Request, Write};
use neonmix_airplay_adapter::control::AirplayActionV2 as Airplay;
use neonmix_control::Operation;
use std::{collections::VecDeque, path::PathBuf, time::Instant};
use uuid::Uuid;

pub const CAPACITY: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextKey {
    pub hub_id: Uuid,
    pub credential_id: Uuid,
    pub runtime_epoch: Uuid,
    pub context_epoch: u64,
}
#[derive(Clone, Debug)]
pub struct Route {
    pub credential: PathBuf,
    pub hub: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetKey {
    Master,
    Native {
        stream_id: u64,
        session_id: Uuid,
        stream_epoch: u64,
        device_id: Uuid,
    },
    Airplay {
        source_id: String,
        session_id: u64,
        stream_epoch: u64,
        receiver_id: String,
    },
    Device(Uuid),
    Source(String),
    Receiver(String),
    AirplayRoom,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Gain,
    Mute,
    Solo,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Queued,
    InFlight,
    Acknowledged,
    Conflict,
    Failed,
    Unknown,
    Cancelled,
}

#[derive(Clone)]
pub struct Intent {
    pub id: Uuid,
    pub context: ContextKey,
    pub route: Route,
    pub target: TargetKey,
    pub write: Write,
    pub label: Option<Message>,
    pub guard: Option<Write>,
    pub frozen: Option<neonmix_desktop_service::IntentCommand>,
    pub created_at: Instant,
    pub phase: Phase,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ApplicationState {
    Pending,
    Stalled,
    Unknown,
}
pub struct ApplicationWatch {
    pub intent: Intent,
    pub sequence: u64,
    pub state: ApplicationState,
}
pub struct Flight {
    pub intent: Intent,
    pub request: Request,
    pub inverse: Option<Write>,
    pub reconciling: bool,
}
#[derive(Default)]
pub struct IntentScheduler {
    pub current: Option<ContextKey>,
    pub queued: VecDeque<Intent>,
    pub flight: Option<Flight>,
    pub history: VecDeque<Intent>,
    pub unknown: VecDeque<Flight>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    Capacity,
    ContextEnded,
    TargetEnding,
    InvalidPatch,
}

impl IntentScheduler {
    pub fn select(&mut self, context: Option<ContextKey>) -> bool {
        if self.current == context {
            return false;
        }
        self.current = context;
        while let Some(intent) = self.queued.pop_front() {
            self.record(intent, Phase::Cancelled);
        }
        // An in-flight operation may already be committed. Keep its original
        // envelope and record the result in its original context.
        true
    }
    pub fn submit(&mut self, proposal: Intent) -> Result<(), Rejected> {
        let Intent {
            id,
            context,
            route,
            target,
            write,
            label,
            guard,
            frozen,
            created_at: now,
            ..
        } = proposal;

        if self.current.as_ref() != Some(&context) {
            return Err(Rejected::ContextEnded);
        }
        let writes = fields(write);
        if writes.is_empty() || (frozen.is_some() && writes.len() != 1) {
            return Err(Rejected::InvalidPatch);
        }
        if writes.len() == 1
            && terminal(&writes[0])
            && self
                .queued
                .iter()
                .chain(self.flight.iter().map(|flight| &flight.intent))
                .chain(self.unknown.iter().map(|flight| &flight.intent))
                .any(|intent| {
                    intent.context == context
                        && intent.target == target
                        && intent.write == writes[0]
                })
        {
            return Ok(());
        }
        if writes.iter().any(|write| field(write).is_some()) && self.ending(&context, &target) {
            return Err(Rejected::TargetEnding);
        }
        let needed = writes
            .iter()
            .filter(|write| {
                !self.queued.iter().any(|pending| {
                    pending.context == context
                        && pending.target == target
                        && field(write).is_some()
                        && field(&pending.write) == field(write)
                })
            })
            .count();
        let removed = if writes.iter().any(terminal) {
            self.queued
                .iter()
                .filter(|pending| {
                    pending.context == context
                        && field(&pending.write).is_some()
                        && writes
                            .iter()
                            .any(|write| terminates(write, &target, &pending.target))
                })
                .count()
        } else {
            0
        };
        if self.queued.len() - removed + needed > CAPACITY {
            return Err(Rejected::Capacity);
        }
        if removed != 0 {
            let mut keep = VecDeque::new();
            while let Some(pending) = self.queued.pop_front() {
                if pending.context == context
                    && field(&pending.write).is_some()
                    && writes
                        .iter()
                        .any(|write| terminates(write, &target, &pending.target))
                {
                    self.record(pending, Phase::Cancelled);
                } else {
                    keep.push_back(pending);
                }
            }
            self.queued = keep;
        }
        for (index, write) in writes.into_iter().enumerate() {
            if let Some(pending) = self.queued.iter_mut().find(|pending| {
                pending.context == context
                    && pending.target == target
                    && field(&write).is_some()
                    && field(&pending.write) == field(&write)
            }) {
                // Retain the original slot/order; fader drags cannot starve C.
                pending.write = write;
                pending.label = label.clone();
                pending.guard = guard.clone();
            } else {
                // Repeated terminal clicks share one queued operation.
                if terminal(&write)
                    && self.queued.iter().any(|pending| {
                        pending.context == context
                            && pending.target == target
                            && pending.write == write
                    })
                {
                    continue;
                }
                self.queued.push_back(Intent {
                    id: if index == 0 { id } else { Uuid::new_v4() },
                    context: context.clone(),
                    route: route.clone(),
                    target: target.clone(),
                    write,
                    label: label.clone(),
                    guard: guard.clone(),
                    frozen: frozen.clone(),
                    created_at: now,
                    phase: Phase::Queued,
                });
            }
        }
        Ok(())
    }
    pub fn take(&mut self) -> Option<Intent> {
        if self.flight.is_some() {
            return None;
        }
        self.queued.pop_front()
    }
    pub fn start(&mut self, mut intent: Intent, request: Request, inverse: Option<Write>) {
        assert!(self.flight.is_none());
        intent.phase = Phase::InFlight;
        self.flight = Some(Flight {
            intent,
            request,
            inverse,
            reconciling: false,
        });
    }
    pub fn complete(&mut self, phase: Phase) -> Option<Flight> {
        let mut flight = self.flight.take()?;
        flight.intent.phase = phase;
        self.record(flight.intent.clone(), phase);
        if phase == Phase::Unknown {
            if self.unknown.len() == CAPACITY {
                self.unknown.pop_front();
            }
            self.unknown.push_back(Flight {
                intent: flight.intent.clone(),
                request: flight.request.clone(),
                inverse: flight.inverse.clone(),
                reconciling: flight.reconciling,
            });
        }
        Some(flight)
    }
    pub fn record(&mut self, mut intent: Intent, phase: Phase) {
        intent.phase = phase;
        if self.history.len() == CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(intent);
    }
    pub fn status(&self, target: &TargetKey, selected_field: Option<Field>) -> Option<Phase> {
        let matches = |intent: &Intent| {
            Some(&intent.context) == self.current.as_ref()
                && &intent.target == target
                && selected_field.is_none_or(|f| field(&intent.write) == Some(f))
        };
        self.queued
            .iter()
            .rev()
            .find(|intent| matches(intent))
            .map(|i| i.phase)
            .or_else(|| {
                self.flight
                    .as_ref()
                    .filter(|flight| matches(&flight.intent))
                    .map(|_| Phase::InFlight)
            })
            .or_else(|| {
                self.history
                    .iter()
                    .rev()
                    .find(|intent| matches(intent))
                    .map(|i| i.phase)
            })
    }
    fn ending(&self, context: &ContextKey, target: &TargetKey) -> bool {
        self.queued
            .iter()
            .chain(self.flight.iter().map(|flight| &flight.intent))
            .chain(self.unknown.iter().map(|flight| &flight.intent))
            .any(|intent| {
                &intent.context == context && terminates(&intent.write, &intent.target, target)
            })
    }
}

pub fn field(write: &Write) -> Option<Field> {
    let (gain, mute, solo) = match write {
        Write::Control(Operation::OutputMix { gain_db, muted }) => {
            (gain_db.is_some(), muted.is_some(), false)
        }
        Write::Control(Operation::StreamMix {
            gain_db,
            muted,
            solo,
            ..
        }) => (gain_db.is_some(), muted.is_some(), solo.is_some()),
        Write::Airplay(Airplay::PatchMixSource {
            gain_db,
            muted,
            solo,
            ..
        }) => (gain_db.is_some(), muted.is_some(), solo.is_some()),
        _ => return None,
    };
    match (gain, mute, solo) {
        (true, false, false) => Some(Field::Gain),
        (false, true, false) => Some(Field::Mute),
        (false, false, true) => Some(Field::Solo),
        _ => None,
    }
}
fn fields(write: Write) -> Vec<Write> {
    let values = match &write {
        Write::Control(Operation::OutputMix { gain_db, muted }) => (*gain_db, *muted, None),
        Write::Control(Operation::StreamMix {
            gain_db,
            muted,
            solo,
            ..
        })
        | Write::Airplay(Airplay::PatchMixSource {
            gain_db,
            muted,
            solo,
            ..
        }) => (*gain_db, *muted, *solo),
        _ => return vec![write],
    };
    [
        (values.0, None, None),
        (None, values.1, None),
        (None, None, values.2),
    ]
    .into_iter()
    .filter(|(gain, mute, solo)| gain.is_some() || mute.is_some() || solo.is_some())
    .map(|(gain, mute, solo)| replace_fields(&write, gain, mute, solo))
    .collect()
}
pub fn replace_fields(
    write: &Write,
    gain: Option<f32>,
    mute: Option<bool>,
    solo: Option<bool>,
) -> Write {
    match write {
        Write::Control(Operation::OutputMix { .. }) => Write::Control(Operation::OutputMix {
            gain_db: gain,
            muted: mute,
        }),
        Write::Control(Operation::StreamMix { stream_id, .. }) => {
            Write::Control(Operation::StreamMix {
                stream_id: *stream_id,
                gain_db: gain,
                muted: mute,
                solo,
            })
        }
        Write::Airplay(Airplay::PatchMixSource {
            source_id,
            session_id,
            ..
        }) => Write::Airplay(Airplay::PatchMixSource {
            source_id: source_id.clone(),
            session_id: *session_id,
            gain_db: gain,
            muted: mute,
            solo,
        }),
        _ => write.clone(),
    }
}
fn terminal(write: &Write) -> bool {
    matches!(
        write,
        Write::Control(
            Operation::Stop { .. } | Operation::Disconnect { .. } | Operation::Revoke { .. }
        ) | Write::Airplay(
            Airplay::DisconnectSource { .. }
                | Airplay::RevokeSource { .. }
                | Airplay::Disable
                | Airplay::DisableReceiver { .. }
        )
    )
}
fn terminates(write: &Write, owner: &TargetKey, target: &TargetKey) -> bool {
    if !terminal(write) {
        return false;
    }
    if owner == target {
        return true;
    }
    match (write, owner, target) {
        (
            Write::Control(Operation::Disconnect { .. } | Operation::Revoke { .. }),
            TargetKey::Device(id),
            TargetKey::Native { device_id, .. },
        ) => id == device_id,
        (
            Write::Airplay(Airplay::RevokeSource { .. }),
            TargetKey::Source(id),
            TargetKey::Airplay { source_id, .. },
        ) => id == source_id,
        (
            Write::Airplay(Airplay::DisableReceiver { .. }),
            TargetKey::Receiver(id),
            TargetKey::Airplay { receiver_id, .. },
        ) => id == receiver_id,
        (Write::Airplay(Airplay::Disable), _, TargetKey::Airplay { .. }) => true,
        _ => false,
    }
}
