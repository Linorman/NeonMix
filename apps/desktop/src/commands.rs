//! One command boundary for rows, console, live view, palette and Undo.
use super::*;
use crate::intent::{ContextKey, Field, Intent, Phase, Rejected, Route, TargetKey};
use neonmix_airplay_adapter::control::AirplayActionV2 as Airplay;
use neonmix_desktop_service::IntentCommand;

impl Desktop {
    pub(crate) fn consume_intent_result(&mut self, result: &Result<Outcome, UiError>) -> bool {
        let Some(flight) = self.intents.flight.as_ref() else {
            return false;
        };
        let current = self.intents.current.as_ref() == Some(&flight.intent.context);
        let (phase, ended) = match result {
            Err(error) => {
                use neonmix_desktop_service::FaultCode;
                let phase = match error.fault.as_ref().map(|fault| fault.code) {
                    Some(FaultCode::RevisionConflict | FaultCode::StaleRevision) => Phase::Conflict,
                    Some(
                        FaultCode::Unauthenticated
                        | FaultCode::PermissionDenied
                        | FaultCode::InvalidArgument
                        | FaultCode::InvalidGain
                        | FaultCode::NotFound
                        | FaultCode::UpgradeRequired
                        | FaultCode::SessionChanged
                        | FaultCode::SnapshotRequired
                        | FaultCode::QuotaExceeded,
                    ) => Phase::Failed,
                    _ => Phase::Unknown,
                };
                let phase = if matches!(flight.request, Request::ResolveAirplayPatch { .. }) {
                    Phase::Failed
                } else {
                    phase
                };
                let phase = if flight.reconciling && phase != Phase::Unknown {
                    Phase::Unknown
                } else {
                    phase
                };
                if current {
                    self.message = match phase {
                        Phase::Conflict => Message::IntentConflict,
                        Phase::Unknown => Message::IntentUnknown,
                        _ => user_error(error.clone()),
                    };
                    self.error = true;
                    self.fresh = None;
                }
                (phase, false)
            }
            Ok(Outcome::Action(request, data))
                if matches!(request.as_ref(), Request::ResolveAirplayPatch { .. }) =>
            {
                if !current {
                    self.intents.complete(Phase::Cancelled);
                    return true;
                }
                let Ok(command) = serde_json::from_value::<
                    neonmix_airplay_adapter::control::AirplayCommandV2,
                >(data["command"].clone()) else {
                    self.intents.complete(Phase::Failed);
                    self.message = Message::IntentFailed;
                    self.error = true;
                    return true;
                };
                if command.command_id != flight.intent.id.to_string()
                    || command.runtime_epoch.as_deref()
                        != Some(flight.intent.context.runtime_epoch.to_string().as_str())
                    || command.credential_id.as_deref()
                        != Some(flight.intent.context.credential_id.to_string().as_str())
                {
                    self.intents.complete(Phase::Failed);
                    self.message = Message::IntentFailed;
                    self.error = true;
                    return true;
                }
                let mut inverse = self.inverse_for(&flight.intent);
                if let Some(old) = &inverse {
                    inverse = match crate::intent::field(old) {
                        Some(Field::Gain) => data["before"]["gain_db"].as_f64().map(|gain| {
                            crate::intent::replace_fields(old, Some(gain as f32), None, None)
                        }),
                        Some(Field::Mute) => data["before"]["muted"]
                            .as_bool()
                            .map(|mute| crate::intent::replace_fields(old, None, Some(mute), None)),
                        Some(Field::Solo) => data["before"]["solo"]
                            .as_bool()
                            .map(|solo| crate::intent::replace_fields(old, None, None, Some(solo))),
                        _ => None,
                    };
                }
                let request = Request::Intent {
                    credential: flight.intent.route.credential.clone(),
                    hub: flight.intent.route.hub.clone(),
                    hub_id: flight.intent.context.hub_id,
                    credential_id: flight.intent.context.credential_id,
                    command: IntentCommand::Airplay(command),
                };
                self.queue(Work::Action(request.clone()));
                if self.busy {
                    let flight = self.intents.flight.as_mut().unwrap();
                    flight.request = request;
                    flight.inverse = inverse;
                } else {
                    self.intents.complete(Phase::Failed);
                    self.message = Message::IntentFailed;
                    self.error = true;
                }
                return true;
            }
            Ok(Outcome::Action(request, data))
                if matches!(request.as_ref(), Request::Intent { .. }) =>
            {
                let Request::Intent {
                    command,
                    hub_id,
                    credential_id,
                    ..
                } = request.as_ref()
                else {
                    unreachable!()
                };
                let id = match command {
                    IntentCommand::Native(command) => command.request_id,
                    IntentCommand::Airplay(command) => {
                        uuid::Uuid::parse_str(&command.command_id).unwrap_or_default()
                    }
                };
                if id != flight.intent.id
                    || *hub_id != flight.intent.context.hub_id
                    || *credential_id != flight.intent.context.credential_id
                {
                    if current {
                        self.message = Message::IntentUnknown;
                        self.error = true;
                    }
                    (Phase::Unknown, false)
                } else {
                    let ended = data["outcome"].as_str() == Some("saved_session_ended");
                    let uncertain = data["warning"].is_string()
                        || data["outcome"].as_str() == Some("configuration_partial");
                    if current {
                        if let IntentCommand::Airplay(command) = command {
                            self.airplay_name_saved(&command.operation);
                            self.airplay = Some(data.clone());
                        }
                        self.message = if uncertain {
                            Message::IntentUnknown
                        } else if ended {
                            Message::IntentTargetEnded
                        } else if data["media_pending"].as_bool() == Some(true) {
                            Message::ShellMediaPending
                        } else {
                            Message::IntentAcknowledged
                        };
                        self.error = uncertain;
                        self.fresh = None;
                    }
                    (
                        if uncertain {
                            Phase::Unknown
                        } else {
                            Phase::Acknowledged
                        },
                        ended,
                    )
                }
            }
            Ok(_) => {
                if current {
                    self.message = Message::IntentUnknown;
                    self.error = true;
                    self.fresh = None;
                }
                (Phase::Unknown, false)
            }
        };
        if current
            && phase == Phase::Acknowledged
            && !ended
            && let Ok(Outcome::Action(_, data)) = result
            && let Some(flight) = self.intents.flight.as_ref()
            && crate::intent::field(&flight.intent.write).is_some()
            && data
                .pointer("/media_application/pending")
                .and_then(Value::as_bool)
                == Some(true)
            && let Some(sequence) = data
                .pointer("/media_application/desired_config_sequence")
                .and_then(Value::as_u64)
        {
            let watch = crate::intent::ApplicationWatch {
                intent: flight.intent.clone(),
                sequence,
                state: if data
                    .pointer("/media_application/stalled")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    crate::intent::ApplicationState::Stalled
                } else {
                    crate::intent::ApplicationState::Pending
                },
            };
            self.applications
                .retain(|old| old.intent.target != watch.intent.target);
            if self.applications.len() == crate::intent::CAPACITY {
                self.applications.pop_front();
            }
            self.applications.push_back(watch);
        }
        self.finish_intent(phase, ended);
        if current {
            self.last_poll = Instant::now() - Duration::from_secs(5);
        }
        true
    }
    pub(crate) fn sync_command_context(&mut self) {
        let route = (self.credential.clone(), self.hub());
        let route_changed = self.context_route.as_ref().is_some_and(|old| old != &route);
        if route_changed {
            self.snapshot = None;
            self.airplay = None;
            self.fresh = None;
            self.synced = None;
        }
        self.context_route = Some(route);
        let identity =
            self.snapshot.as_ref().and_then(|snapshot| {
                let profile =
                    self.status.as_ref()?.profiles.iter().find(|profile| {
                        profile.credential == self.credential && !profile.pending
                    })?;
                if profile.hub_id != Some(snapshot.hub_id) {
                    return None;
                }
                let identity = profile.device_id?;
                if snapshot
                    .devices
                    .get(&identity)
                    .is_none_or(|device| device.revoked)
                {
                    return None;
                }
                Some((snapshot.hub_id, identity, snapshot.runtime_epoch))
            });
        let same = match (identity, self.intents.current.as_ref()) {
            (Some((hub, credential, runtime)), Some(old)) => {
                old.hub_id == hub && old.credential_id == credential && old.runtime_epoch == runtime
            }
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.context_epoch = self.context_epoch.saturating_add(1);
            let context = identity.map(|(hub_id, credential_id, runtime_epoch)| ContextKey {
                hub_id,
                credential_id,
                runtime_epoch,
                context_epoch: self.context_epoch,
            });
            let had_context = self.intents.current.is_some();
            self.intents.select(context);
            self.applications.clear();
            if had_context || route_changed || identity.is_none() {
                self.undo = None;
                self.gain_drafts.clear();
                self.gain_draft_owners.clear();
                self.selected_lane = None;
            }
            if had_context {
                self.message = Message::IntentContextChanged;
            }
        }
        let stale: Vec<_> = self
            .gain_draft_owners
            .iter()
            .filter_map(|(key, (context, target))| {
                (self.intents.current.as_ref() != Some(context)
                    || self.mix_values(target).is_none())
                .then_some(*key)
            })
            .collect();
        for key in stale {
            self.gain_drafts.remove(&key);
            self.gain_draft_owners.remove(&key);
        }
    }
    pub(crate) fn target_for(&self, write: &Write) -> Option<TargetKey> {
        match write {
            Write::Control(Operation::OutputMix { .. }) => Some(TargetKey::Master),
            Write::Control(Operation::StreamMix { stream_id, .. }) => {
                let stream = self.snapshot.as_ref()?.streams.get(stream_id)?;
                let session = self.snapshot.as_ref()?.sessions.get(&stream.session_id)?;
                if !session.status.active() {
                    return None;
                }
                Some(TargetKey::Native {
                    stream_id: *stream_id,
                    session_id: session.id,
                    stream_epoch: session.offer.stream_epoch,
                    device_id: stream.device_id,
                })
            }
            Write::Control(Operation::Stop { session_id }) => {
                let session = self.snapshot.as_ref()?.sessions.get(session_id)?;
                Some(TargetKey::Native {
                    stream_id: session.stream_id,
                    session_id: *session_id,
                    stream_epoch: session.offer.stream_epoch,
                    device_id: session.device_id,
                })
            }
            Write::Control(
                Operation::Disconnect { device_id }
                | Operation::AllowPlayback { device_id }
                | Operation::Revoke { device_id },
            ) => Some(TargetKey::Device(*device_id)),
            Write::Control(_) => None,
            Write::Airplay(action) => match action {
                Airplay::MixSource {
                    source_id,
                    session_id,
                    ..
                }
                | Airplay::PatchMixSource {
                    source_id,
                    session_id,
                    ..
                }
                | Airplay::DisconnectSource {
                    source_id,
                    session_id,
                }
                | Airplay::RevokeSource {
                    source_id,
                    session_id: Some(session_id),
                } => {
                    let sessions = self.airplay_sessions();
                    let session = sessions.iter().find(|s| {
                        s["source_id"].as_str() == Some(source_id)
                            && s["session_id"].as_u64() == Some(*session_id)
                    })?;
                    Some(TargetKey::Airplay {
                        source_id: source_id.clone(),
                        session_id: *session_id,
                        stream_epoch: session["stream_epoch"].as_u64()?,
                        receiver_id: session["receiver_id"].as_str()?.into(),
                    })
                }
                Airplay::RevokeSource { source_id, .. }
                | Airplay::AllowSource { source_id }
                | Airplay::RepairSource { source_id, .. }
                | Airplay::AliasSource { source_id, .. }
                | Airplay::PlaybackMode { source_id, .. } => {
                    Some(TargetKey::Source(source_id.clone()))
                }
                Airplay::EnableReceiver { receiver_id }
                | Airplay::DisableReceiver { receiver_id }
                | Airplay::PairReceiver { receiver_id }
                | Airplay::RenameReceiver { receiver_id, .. } => {
                    Some(TargetKey::Receiver(receiver_id.clone()))
                }
                _ => Some(TargetKey::AirplayRoom),
            },
        }
    }
    fn proposal(&self, write: Write, label: Option<Message>) -> Option<Intent> {
        let context = self.intents.current.clone()?;
        Some(Intent {
            id: uuid::Uuid::new_v4(),
            context,
            route: Route {
                credential: self.credential.clone(),
                hub: self.hub(),
            },
            target: self.target_for(&write)?,
            write,
            label,
            guard: None,
            frozen: None,
            created_at: Instant::now(),
            phase: Phase::Queued,
        })
    }
    pub(crate) fn enqueue_bound(&mut self, intent: Intent) {
        let rejected = intent.clone();
        let result = self.intents.submit(intent);
        if let Err(error) = result {
            self.intents.record(
                rejected,
                if error == Rejected::ContextEnded || error == Rejected::TargetEnding {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                },
            );
            self.message = match error {
                Rejected::Capacity => Message::IntentQueueFull,
                Rejected::ContextEnded => Message::IntentContextChanged,
                Rejected::TargetEnding | Rejected::InvalidPatch => Message::IntentTargetEnded,
            };
            self.error = true;
        }
        self.dispatch_intents();
    }
    pub(crate) fn enqueue_write(&mut self, write: Write, label: Option<Message>) {
        self.sync_command_context();
        if self.preview || !(self.ready() || self.writable()) {
            return;
        }
        if self
            .status
            .as_ref()
            .is_none_or(|s| s.intent_version != neonmix_desktop_service::INTENT_VERSION)
            || self
                .intents
                .current
                .as_ref()
                .is_none_or(|c| c.runtime_epoch.is_nil())
        {
            self.message = Message::FaultUpgradeRequired;
            self.error = true;
            return;
        }
        if let Some(intent) = self.proposal(write, label) {
            self.enqueue_bound(intent);
        } else {
            self.message = Message::IntentTargetEnded;
            self.error = true;
        }
    }
    fn mix_values(&self, target: &TargetKey) -> Option<(f32, bool, bool)> {
        match target {
            TargetKey::Master => {
                let o = &self.snapshot.as_ref()?.output;
                Some((o.gain_db, o.muted, false))
            }
            TargetKey::Native {
                stream_id,
                session_id,
                stream_epoch,
                ..
            } => {
                let stream = self.snapshot.as_ref()?.streams.get(stream_id)?;
                if stream.session_id != *session_id
                    || self
                        .snapshot
                        .as_ref()?
                        .sessions
                        .get(session_id)?
                        .offer
                        .stream_epoch
                        != *stream_epoch
                {
                    return None;
                }
                Some((stream.mix.gain_db, stream.mix.muted, stream.mix.solo))
            }
            TargetKey::Airplay {
                source_id,
                session_id,
                stream_epoch,
                ..
            } => {
                let sessions = self.airplay_sessions();
                let s = sessions.iter().find(|s| {
                    s["source_id"].as_str() == Some(source_id)
                        && s["session_id"].as_u64() == Some(*session_id)
                        && s["stream_epoch"].as_u64() == Some(*stream_epoch)
                })?;
                Some((
                    s.pointer("/mix/gain_db")?.as_f64()? as f32,
                    s.pointer("/mix/muted")?.as_bool()?,
                    s.pointer("/mix/solo")?.as_bool()?,
                ))
            }
            _ => None,
        }
    }
    fn inverse_for(&self, intent: &Intent) -> Option<Write> {
        let (gain, mute, solo) = self.mix_values(&intent.target)?;
        Some(match crate::intent::field(&intent.write)? {
            Field::Gain => crate::intent::replace_fields(&intent.write, Some(gain), None, None),
            Field::Mute => crate::intent::replace_fields(&intent.write, None, Some(mute), None),
            Field::Solo => crate::intent::replace_fields(&intent.write, None, None, Some(solo)),
        })
    }
    fn guard_matches(&self, intent: &Intent) -> bool {
        let Some(guard) = &intent.guard else {
            return true;
        };
        let Some((gain, mute, solo)) = self.mix_values(&intent.target) else {
            return false;
        };
        let current = match crate::intent::field(guard) {
            Some(Field::Gain) => crate::intent::replace_fields(guard, Some(gain), None, None),
            Some(Field::Mute) => crate::intent::replace_fields(guard, None, Some(mute), None),
            Some(Field::Solo) => crate::intent::replace_fields(guard, None, None, Some(solo)),
            _ => return false,
        };
        current == *guard
    }
    pub(crate) fn dispatch_intents(&mut self) {
        if self.busy || !self.ready() || self.preview {
            return;
        }
        // Every scan consumes one of the bounded 128 queued slots.
        for _ in 0..crate::intent::CAPACITY {
            let Some(mut intent) = self.intents.take() else {
                return;
            };
            if self.intents.current.as_ref() != Some(&intent.context)
                || self.target_for(&intent.write).as_ref() != Some(&intent.target)
            {
                self.remove_intent_draft(&intent);
                self.intents.record(intent, Phase::Cancelled);
                self.message = Message::IntentTargetEnded;
                continue;
            }
            let inverse = self.inverse_for(&intent);
            if !self.guard_matches(&intent) {
                self.intents.record(intent, Phase::Conflict);
                self.message = Message::IntentUndoConflict;
                self.error = true;
                continue;
            }
            if let (Some(label), Some(before)) = (&mut intent.label, &inverse) {
                let previous = match before {
                    Write::Control(
                        Operation::OutputMix {
                            gain_db: Some(g), ..
                        }
                        | Operation::StreamMix {
                            gain_db: Some(g), ..
                        },
                    )
                    | Write::Airplay(Airplay::PatchMixSource {
                        gain_db: Some(g), ..
                    }) => Some(widgets::gain_text(*g)),
                    _ => None,
                };
                if let Some(previous) = previous {
                    match label {
                        Message::ShellMasterGain { previous: old, .. } => *old = previous,
                        Message::LaneGainChanged { before, .. } => *before = previous,
                        _ => {}
                    }
                }
            }
            let command = if let Some(command) = intent.frozen.clone() {
                command
            } else {
                match &intent.write {
                    Write::Control(operation) => {
                        let mut command = neonmix_control::Command::bound(
                            self.snapshot.as_ref().unwrap(),
                            intent.context.credential_id,
                            operation.clone(),
                        );
                        command.request_id = intent.id;
                        IntentCommand::Native(command)
                    }
                    Write::Airplay(operation) => {
                        if matches!(operation, Airplay::PatchMixSource { .. })
                            && !self
                                .airplay
                                .as_ref()
                                .and_then(|a| a["capabilities"].as_array())
                                .is_some_and(|caps| caps.iter().any(|c| c == "patch_mix_source"))
                        {
                            let TargetKey::Airplay { stream_epoch, .. } = &intent.target else {
                                unreachable!()
                            };
                            let request = Request::ResolveAirplayPatch {
                                credential: intent.route.credential.clone(),
                                hub: intent.route.hub.clone(),
                                hub_id: intent.context.hub_id,
                                credential_id: intent.context.credential_id,
                                runtime_epoch: intent.context.runtime_epoch,
                                stream_epoch: *stream_epoch,
                                command: neonmix_airplay_adapter::control::AirplayCommandV2 {
                                    command_version: 2,
                                    expected_config_revision: None,
                                    expected_event_sequence: None,
                                    command_id: intent.id.to_string(),
                                    runtime_epoch: Some(intent.context.runtime_epoch.to_string()),
                                    credential_id: Some(intent.context.credential_id.to_string()),
                                    expected_revision: Some(0),
                                    operation: operation.clone(),
                                },
                                guard: intent.guard.as_ref().and_then(|write| {
                                    if let Write::Airplay(action) = write {
                                        Some(Box::new(action.clone()))
                                    } else {
                                        None
                                    }
                                }),
                            };
                            self.queue(Work::Action(request.clone()));
                            if self.busy {
                                self.intents.start(intent, request, inverse);
                                self.fresh = None;
                            } else {
                                self.intents.record(intent, Phase::Failed);
                                self.message = Message::IntentFailed;
                                self.error = true;
                            }
                            return;
                        }
                        let Some(command) =
                            self.bind_airplay(intent.id, &intent.context, operation.clone())
                        else {
                            self.intents.record(intent, Phase::Failed);
                            self.message = Message::IntentFailed;
                            self.error = true;
                            continue;
                        };
                        IntentCommand::Airplay(command)
                    }
                }
            };
            let request = Request::Intent {
                credential: intent.route.credential.clone(),
                hub: intent.route.hub.clone(),
                hub_id: intent.context.hub_id,
                credential_id: intent.context.credential_id,
                command,
            };
            // Freeze the actual envelope only after the single worker accepts it.
            self.queue(Work::Action(request.clone()));
            if self.busy {
                self.intents.start(intent, request, inverse);
                self.fresh = None;
            } else {
                self.intents.record(intent, Phase::Failed);
                self.message = Message::IntentFailed;
                self.error = true;
            }
            return;
        }
    }
    pub(crate) fn remove_intent_draft(&mut self, intent: &Intent) {
        if crate::intent::field(&intent.write) != Some(Field::Gain) {
            return;
        }
        let key = match &intent.target {
            TargetKey::Master => 0,
            TargetKey::Native { stream_id, .. } => *stream_id,
            TargetKey::Airplay {
                source_id,
                session_id,
                ..
            } => self
                .airplay_sessions()
                .iter()
                .find(|s| {
                    s["source_id"].as_str() == Some(source_id)
                        && s["session_id"].as_u64() == Some(*session_id)
                })
                .and_then(|s| s["stream_id"].as_u64())
                .unwrap_or(0),
            _ => return,
        };
        if self
            .gain_draft_owners
            .get(&key)
            .is_some_and(|owner| owner != &(intent.context.clone(), intent.target.clone()))
        {
            return;
        }
        let sent_gain = match &intent.write {
            Write::Control(
                Operation::OutputMix { gain_db, .. } | Operation::StreamMix { gain_db, .. },
            )
            | Write::Airplay(Airplay::PatchMixSource { gain_db, .. }) => *gain_db,
            _ => None,
        };
        if self
            .gain_drafts
            .get(&key)
            .zip(sent_gain)
            .is_some_and(|(draft, sent)| (*draft - sent).abs() > 0.01)
        {
            return;
        }
        if !self.intents.queued.iter().any(|i| {
            i.context == intent.context
                && i.target == intent.target
                && crate::intent::field(&i.write) == Some(Field::Gain)
        }) {
            self.gain_drafts.remove(&key);
            self.gain_draft_owners.remove(&key);
        }
    }
    pub(crate) fn finish_intent(&mut self, phase: Phase, ended: bool) -> bool {
        let Some(flight) = self.intents.complete(phase) else {
            return true;
        };
        if self.intents.current.as_ref() != Some(&flight.intent.context) {
            return false;
        }
        if phase == Phase::Acknowledged {
            self.remove_intent_draft(&flight.intent);
            if !ended && let (Some(label), Some(inverse)) = (flight.intent.label, flight.inverse) {
                self.undo = Some(Undo {
                    label,
                    inverse,
                    after: flight.intent.write,
                    context: flight.intent.context,
                    target: flight.intent.target,
                    request_id: flight.intent.id,
                    at: Instant::now(),
                });
            }
        }
        true
    }
    pub(crate) fn undo_matches(&self) -> bool {
        let Some(undo) = &self.undo else {
            return false;
        };
        if undo.at.elapsed() >= Duration::from_secs(8)
            || Some(&undo.context) != self.intents.current.as_ref()
            || self.target_for(&undo.after).as_ref() != Some(&undo.target)
        {
            return false;
        }
        if !self
            .intents
            .history
            .iter()
            .any(|intent| intent.id == undo.request_id && intent.phase == Phase::Acknowledged)
        {
            return false;
        }
        let Some((gain, mute, solo)) = self.mix_values(&undo.target) else {
            return false;
        };
        let value = match crate::intent::field(&undo.after) {
            Some(Field::Gain) => crate::intent::replace_fields(&undo.after, Some(gain), None, None),
            Some(Field::Mute) => crate::intent::replace_fields(&undo.after, None, Some(mute), None),
            Some(Field::Solo) => crate::intent::replace_fields(&undo.after, None, None, Some(solo)),
            _ => return false,
        };
        value == undo.after
    }
    pub(crate) fn restore_confirmed(&mut self) {
        self.sync_command_context();
        if !self.undo_matches() {
            self.message = Message::IntentUndoConflict;
            self.error = true;
            self.undo = None;
            return;
        }
        if let Some(undo) = self.undo.take()
            && let Some(mut intent) = self.proposal(undo.inverse, None)
        {
            intent.guard = Some(undo.after);
            self.enqueue_bound(intent);
        }
    }
    pub(crate) fn bind_airplay(
        &self,
        id: uuid::Uuid,
        context: &ContextKey,
        operation: Airplay,
    ) -> Option<neonmix_airplay_adapter::control::AirplayCommandV2> {
        let state = self.airplay.as_ref()?;
        if state["command_version"].as_u64() == Some(3) {
            Some(neonmix_airplay_adapter::control::AirplayCommandV2::bound(
                id.to_string(),
                context.runtime_epoch.to_string(),
                context.credential_id.to_string(),
                state["config_revision"].as_u64()?,
                state["event_sequence"].as_u64()?,
                operation,
            ))
        } else {
            Some(neonmix_airplay_adapter::control::AirplayCommandV2 {
                command_version: 2,
                expected_config_revision: None,
                expected_event_sequence: None,
                command_id: id.to_string(),
                runtime_epoch: Some(context.runtime_epoch.to_string()),
                credential_id: Some(context.credential_id.to_string()),
                expected_revision: Some(state["revision"].as_u64()?),
                operation,
            })
        }
    }
    pub(crate) fn confirmation_intent(&mut self, request: &Request) -> Option<Intent> {
        self.sync_command_context();
        let write = match request {
            Request::Control { operation, .. } => Write::Control(operation.clone()),
            Request::AirplayV2 {
                command: Some(command),
                ..
            } => Write::Airplay(command.operation.clone()),
            _ => return None,
        };
        let mut intent = self.proposal(write, None)?;
        intent.frozen = Some(match request {
            Request::Control {
                expected_revision,
                operation,
                ..
            } => {
                let state = self.snapshot.as_ref()?;
                if *expected_revision != state.revision {
                    return None;
                }
                let mut command = neonmix_control::Command::bound(
                    state,
                    intent.context.credential_id,
                    operation.clone(),
                );
                command.request_id = intent.id;
                IntentCommand::Native(command)
            }
            Request::AirplayV2 {
                command: Some(command),
                ..
            } => {
                intent.id = uuid::Uuid::parse_str(&command.command_id).ok()?;
                IntentCommand::Airplay(command.clone())
            }
            _ => return None,
        });
        Some(intent)
    }
    pub(crate) fn reconcile_intent(&mut self) {
        self.sync_command_context();
        if self.busy || !self.ready() || self.intents.flight.is_some() {
            return;
        }
        let Some(index) = self
            .intents
            .unknown
            .iter()
            .position(|flight| self.intents.current.as_ref() == Some(&flight.intent.context))
        else {
            return;
        };
        let mut flight = self.intents.unknown.remove(index).unwrap();
        self.queue(Work::Action(flight.request.clone()));
        if self.busy {
            flight.reconciling = true;
            flight.intent.phase = Phase::InFlight;
            self.intents.flight = Some(flight);
            self.fresh = None;
        } else {
            self.intents.unknown.push_front(flight);
        }
    }
    pub(crate) fn refresh_applications(&mut self, progress: Option<&Value>) {
        let mut remaining = std::collections::VecDeque::new();
        let mut notice = None;
        while let Some(mut watch) = self.applications.pop_front() {
            if self.intents.current.as_ref() != Some(&watch.intent.context) {
                continue;
            }
            let mut expected = watch.intent.clone();
            expected.guard = Some(watch.intent.write.clone());
            if !self.guard_matches(&expected) {
                notice = Some(if self.mix_values(&expected.target).is_none() {
                    Message::IntentTargetEnded
                } else {
                    Message::ShellMediaSuperseded
                });
                continue;
            }
            let scoped = progress.filter(|p| {
                p["runtime_epoch"].as_str()
                    == Some(watch.intent.context.runtime_epoch.to_string().as_str())
            });
            let next = if let Some(value) = scoped {
                if value["applied_config_sequence"]
                    .as_u64()
                    .is_some_and(|seq| seq >= watch.sequence)
                {
                    notice = Some(Message::ShellMediaApplied);
                    continue;
                }
                if value["stalled"].as_bool() == Some(true) {
                    crate::intent::ApplicationState::Stalled
                } else {
                    crate::intent::ApplicationState::Pending
                }
            } else {
                crate::intent::ApplicationState::Unknown
            };
            if next != watch.state {
                notice = Some(match next {
                    crate::intent::ApplicationState::Pending => Message::ShellMediaPending,
                    crate::intent::ApplicationState::Stalled => Message::ShellMediaStalled,
                    crate::intent::ApplicationState::Unknown => Message::ShellMediaUnknown,
                });
            }
            watch.state = next;
            remaining.push_back(watch);
        }
        self.applications = remaining;
        if let Some(message) = notice {
            self.message = message;
            self.error = matches!(
                self.message,
                Message::ShellMediaStalled | Message::ShellMediaUnknown
            );
        }
    }
    pub(crate) fn command_status(&self, key: u64) -> Option<Message> {
        let target = if key == 0 {
            TargetKey::Master
        } else {
            let native = Write::Control(Operation::StreamMix {
                stream_id: key,
                gain_db: Some(0.),
                muted: None,
                solo: None,
            });
            self.target_for(&native).or_else(|| {
                let sessions = self.airplay_sessions();
                let s = sessions
                    .iter()
                    .find(|s| s["stream_id"].as_u64() == Some(key))?;
                self.target_for(&Write::Airplay(Airplay::PatchMixSource {
                    source_id: s["source_id"].as_str()?.into(),
                    session_id: s["session_id"].as_u64()?,
                    gain_db: Some(0.),
                    muted: None,
                    solo: None,
                }))
            })?
        };
        if matches!(
            self.intents.status(&target, None),
            None | Some(Phase::Acknowledged)
        ) && let Some(watch) = self.applications.iter().rev().find(|watch| {
            watch.intent.target == target
                && self.intents.current.as_ref() == Some(&watch.intent.context)
        }) {
            return Some(match watch.state {
                crate::intent::ApplicationState::Pending => Message::ShellMediaPending,
                crate::intent::ApplicationState::Stalled => Message::ShellMediaStalled,
                crate::intent::ApplicationState::Unknown => Message::ShellMediaUnknown,
            });
        }
        self.intents.status(&target, None).map(|phase| match phase {
            Phase::Queued => Message::IntentQueued,
            Phase::InFlight => Message::IntentInFlight,
            Phase::Acknowledged => Message::IntentAcknowledged,
            Phase::Conflict => Message::IntentConflict,
            Phase::Failed => Message::IntentFailed,
            Phase::Unknown => Message::IntentUnknown,
            Phase::Cancelled => Message::IntentCancelled,
        })
    }
    pub(crate) fn capture_gain_draft(&mut self, key: u64) {
        let Some(context) = self.intents.current.clone() else {
            return;
        };
        let target = if key == 0 {
            Some(TargetKey::Master)
        } else {
            self.target_for(&Write::Control(Operation::StreamMix {
                stream_id: key,
                gain_db: Some(0.),
                muted: None,
                solo: None,
            }))
            .or_else(|| {
                let sessions = self.airplay_sessions();
                let s = sessions
                    .iter()
                    .find(|s| s["stream_id"].as_u64() == Some(key))?;
                self.target_for(&Write::Airplay(Airplay::PatchMixSource {
                    source_id: s["source_id"].as_str()?.into(),
                    session_id: s["session_id"].as_u64()?,
                    gain_db: Some(0.),
                    muted: None,
                    solo: None,
                }))
            })
        };
        if let Some(target) = target {
            self.gain_draft_owners.insert(key, (context, target));
        }
    }
    pub(crate) fn command_note(&self, ui: &mut egui::Ui, key: u64) {
        let message = self.command_status(key);
        let failed = matches!(
            message,
            Some(
                Message::IntentFailed
                    | Message::IntentConflict
                    | Message::IntentUnknown
                    | Message::ShellMediaStalled
                    | Message::ShellMediaUnknown
            )
        );
        let text = message.map(|message| self.tr(&message)).unwrap_or_default();
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 16.),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&text).size(11.).color(if failed {
                        theme::WARNING
                    } else {
                        theme::TEXT_3
                    }))
                    .truncate(),
                )
                .on_hover_text(text);
            },
        );
    }
}
