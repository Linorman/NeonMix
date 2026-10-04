//! Targeted administrator API. File transactions are serialized separately from audio state.
use super::*;
use neonmix_airplay_adapter::control::{AirplayActionV2 as Op, AirplayCommandV2};
#[derive(Debug)]
pub(crate) struct Error(pub(super) &'static str);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0 {
            "unauthenticated" => StatusCode::UNAUTHORIZED,
            "permission_denied" => StatusCode::FORBIDDEN,
            "stale_revision" | "session_changed" | "receiver_busy" | "room_capacity_full"
            | "command_id_reused" => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        (status, Json(serde_json::json!({"error":self.0}))).into_response()
    }
}
impl From<ApiError> for Error {
    fn from(value: ApiError) -> Self {
        Self(match value.0 {
            ControlError::Unauthenticated => "unauthenticated",
            ControlError::PermissionDenied => "permission_denied",
            ControlError::RevisionConflict => "stale_revision",
            ControlError::QuotaExceeded => "room_capacity_full",
            ControlError::AlreadyActive => "receiver_busy",
            ControlError::PlaybackBlocked => "output_unavailable",
            _ => "busy",
        })
    }
}
fn role(shared: &Shared, headers: &HeaderMap, write: bool) -> std::result::Result<bool, Error> {
    let e = shared.lock().map_err(|_| Error("busy"))?;
    let principal = authenticate(&e, headers)?;
    let admin = e
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_some_and(|d| d.role == neonmix_control::Role::Admin);
    if write && !admin {
        return Err(Error("permission_denied"));
    }
    Ok(admin)
}
fn view(e: &Engine, p: &ReceiverProfile, admin: bool) -> serde_json::Value {
    let receivers = e
        .airplay
        .receivers
        .iter()
        .map(|r| {
            let mut status = r.display_status_at(Instant::now());
            if !admin {
                status.pairing_pin = None;
            }
            let source_id = status.source_id.clone();
            let mut value = serde_json::to_value(status).unwrap();
            if let Some(alias) = source_id
                .as_ref()
                .and_then(|id| p.sources.iter().find(|s| s.source_id == *id))
                .and_then(|s| s.alias.as_ref())
            {
                value["source_name"] = serde_json::json!(alias);
            }
            value["receiver_id"] = serde_json::json!(r.receiver_id);
            value["name"] = serde_json::json!(r.name);
            value["configured_enabled"] = serde_json::json!(
                p.receivers
                    .iter()
                    .find(|saved| saved.receiver_id == r.receiver_id)
                    .is_some_and(|saved| saved.enabled)
            );
            value
        })
        .collect::<Vec<_>>();
    let sources=p.sources.iter().map(|s|serde_json::json!({"source_id":s.source_id,"alias":s.alias,"last_name":s.last_name,
        "blocked":s.blocked,"revoked":s.revoked,"gain_db":s.gain_db,"muted":s.muted,"playback_mode":s.playback_mode})).collect::<Vec<_>>();
    let sessions = e
        .airplay
        .receivers
        .iter()
        .filter(|r| r.status.active)
        .filter_map(|r| {
            let claim = e.resources.admissions.claims.get(&r.receiver_id)?;
            let mut v = serde_json::to_value(&r.status).ok()?;
            v.as_object_mut()?.remove("pairing_pin");
            v["receiver_id"] = serde_json::json!(r.receiver_id);
            v["source_id"] = serde_json::json!(claim.owner.source);
            if let Some(alias) = p
                .sources
                .iter()
                .find(|s| s.source_id == claim.owner.source)
                .and_then(|s| s.alias.as_ref())
            {
                v["source_name"] = serde_json::json!(alias);
            }
            v["session_id"] = serde_json::json!(claim.context.session_id);
            v["stream_id"] = serde_json::json!(claim.context.stream_id);
            v["stream_epoch"] = serde_json::json!(claim.context.stream_epoch);
            v["format_epoch"] = serde_json::json!(claim.context.format_epoch);
            v["mapping_id"] = serde_json::json!(claim.context.mapping_id);
            v["lane"] = serde_json::json!(claim.lane);
            Some(v)
        })
        .collect::<Vec<_>>();
    let native = e
        .authority
        .current()
        .sessions
        .values()
        .filter(|s| s.status.active())
        .count();
    let active = native
        + e.resources
            .admissions
            .claims
            .values()
            .filter(|c| c.active)
            .count();
    let native_reserved = usize::from(e.resources.native_reservation.is_some());
    let reserved = native_reserved
        + e.resources
            .admissions
            .claims
            .values()
            .filter(|c| !c.active)
            .count();
    let limit = if e.airplay.multi_receiver {
        4
    } else if e.resources.admissions.claims.is_empty() {
        LANES
    } else {
        2
    };
    serde_json::json!({"revision":e.airplay.revision,"enabled":e.airplay.receivers.iter().any(|r|r.status.enabled),
        "multi_receiver":e.airplay.multi_receiver,"receivers":receivers,"sources":sources,"sessions":sessions,
        "capacity":{"limit":limit,"active":active,"reserved":reserved,"available":limit.saturating_sub(active+reserved)}})
}
pub(crate) async fn snapshot(
    axum::extract::State(shared): axum::extract::State<Shared>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, Error> {
    snapshot_with(&shared, &headers, || {})
}
fn snapshot_with(
    shared: &Shared,
    headers: &HeaderMap,
    after_authorize: impl FnOnce(),
) -> std::result::Result<Json<serde_json::Value>, Error> {
    role(shared, headers, false)?;
    after_authorize();
    let profiles = shared
        .lock()
        .map_err(|_| Error("busy"))?
        .airplay
        .profile
        .clone();
    let guard = profiles.lock().map_err(|_| Error("busy"))?;
    let e = shared.lock().map_err(|_| Error("busy"))?;
    // Trust-file contention must not preserve an administrator's old PIN access.
    let principal = authenticate(&e, headers)?;
    let admin = e
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_some_and(|d| d.role == neonmix_control::Role::Admin);
    if let Some(profile) = guard.as_ref() {
        return Ok(Json(view(&e, profile, admin)));
    }
    // Reading an uninitialized room must not create credentials or migrate files.
    // The first real receiver uses the durable Hub UUID when an admin enables it.
    let mut receiver =
        serde_json::to_value(e.airplay.receivers[0].display_status_at(Instant::now())).unwrap();
    receiver["receiver_id"] = serde_json::json!(e.authority.current().hub_id);
    receiver["name"] = serde_json::json!(speaker_name(&e.room_name));
    receiver["configured_enabled"] = serde_json::json!(true);
    receiver.as_object_mut().unwrap().remove("pairing_pin");
    let active = e
        .authority
        .current()
        .sessions
        .values()
        .filter(|s| s.status.active())
        .count();
    let reserved = usize::from(e.resources.native_reservation.is_some());
    Ok(Json(
        serde_json::json!({"revision":e.airplay.revision,"enabled":false,"multi_receiver":false,
        "receivers":[receiver],"sources":[],"sessions":[],
        "capacity":{"limit":LANES,"active":active,"reserved":reserved,"available":LANES.saturating_sub(active+reserved)}}),
    ))
}
fn receiver_index(e: &Engine, id: &str) -> std::result::Result<usize, Error> {
    let id = Uuid::parse_str(id).map_err(|_| Error("receiver_unknown"))?;
    e.airplay
        .receivers
        .iter()
        .position(|r| r.receiver_id == id)
        .ok_or(Error("receiver_unknown"))
}
fn session_index(
    e: &Engine,
    source: &str,
    session: Option<u64>,
) -> std::result::Result<Option<usize>, Error> {
    let found = e.airplay.receivers.iter().position(|r| {
        e.resources
            .admissions
            .claims
            .get(&r.receiver_id)
            .is_some_and(|c| c.owner.source == source)
    });
    match (found, session) {
        (Some(i), Some(id))
            if e.resources.admissions.claims[&e.airplay.receivers[i].receiver_id]
                .context
                .session_id
                == id =>
        {
            Ok(Some(i))
        }
        (None, None) => Ok(None),
        _ => Err(Error("session_changed")),
    }
}
pub(crate) async fn command(
    axum::extract::State(shared): axum::extract::State<Shared>,
    headers: HeaderMap,
    Json(command): Json<AirplayCommandV2>,
) -> std::result::Result<Json<serde_json::Value>, Error> {
    execute(&shared, &headers, command, persist_change)
}
fn execute(
    shared: &Shared,
    headers: &HeaderMap,
    command: AirplayCommandV2,
    persist: impl FnOnce(
        &std::path::Path,
        &ReceiverProfile,
        ReceiverProfile,
        Option<(usize, bool)>,
    ) -> std::result::Result<Persisted, Error>,
) -> std::result::Result<Json<serde_json::Value>, Error> {
    role(shared, headers, true)?;
    ensure_profile(shared)?;
    let id = Uuid::parse_str(&command.command_id).map_err(|_| Error("invalid_command_id"))?;
    if id.to_string() != command.command_id {
        return Err(Error("invalid_command_id"));
    }
    let payload = serde_json::to_string(&command).map_err(|_| Error("invalid_command"))?;
    let profiles = shared
        .lock()
        .map_err(|_| Error("busy"))?
        .airplay
        .profile
        .clone();
    // Consistent lock order is profile -> Engine; no disk I/O holds Engine.
    let mut guard = profiles.lock().map_err(|_| Error("busy"))?;
    let mut next = guard.as_ref().ok_or(Error("profile_unavailable"))?.clone();
    let mut e = shared.lock().map_err(|_| Error("busy"))?;
    let principal = authenticate(&e, headers)?;
    if e.authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_none_or(|d| d.role != neonmix_control::Role::Admin)
    {
        return Err(Error("permission_denied"));
    }
    if let Some((_, old, result)) = e
        .airplay
        .receipts
        .iter()
        .find(|(id, _, _)| id == &command.command_id)
    {
        return if old == &payload {
            Ok(Json(result.clone()))
        } else {
            Err(Error("command_id_reused"))
        };
    }
    if command.expected_revision != e.airplay.revision {
        return Err(Error("stale_revision"));
    }
    if !e.resources.control.has_capacity() {
        return Err(Error("busy"));
    }
    let path = e.airplay.receivers[0].directory.join("receiver.json");
    let mut actions = Vec::new();
    let mut save = false;
    let mut configure = None;
    let mut pairing_window = None;
    let running = e.airplay.receivers.iter().any(|r| r.status.enabled);
    match &command.operation {
        Op::Configure {
            receiver_count,
            multi_receiver,
        } => {
            if !(1..=4).contains(receiver_count) || (!multi_receiver && *receiver_count != 1) {
                return Err(Error("invalid_receiver_count"));
            }
            if !*multi_receiver && !e.resources.admissions.claims.is_empty() {
                return Err(Error("receiver_busy"));
            }
            for receiver in e.airplay.receivers.iter().skip(*receiver_count) {
                if e.resources
                    .admissions
                    .claims
                    .contains_key(&receiver.receiver_id)
                    || e.airplay.pairing_leases.contains_key(&receiver.receiver_id)
                {
                    return Err(Error("receiver_busy"));
                }
            }
            let native = e
                .authority
                .current()
                .sessions
                .values()
                .filter(|s| s.status.active())
                .count();
            if *multi_receiver
                && native
                    + usize::from(e.resources.native_reservation.is_some())
                    + e.resources.admissions.claims.len()
                    > 4
            {
                return Err(Error("room_capacity_full"));
            }
            configure = Some((*receiver_count, *multi_receiver));
            save = true;
        }
        Op::Enable => {
            if !e.authority.current().output.available {
                return Err(Error("output_unavailable"));
            }
            for (i, r) in next.receivers.iter().enumerate().filter(|(_, r)| r.enabled) {
                if e.airplay.receivers[i].commands.is_none() {
                    actions.push((i, AirplayAction::Enable));
                }
                let _ = r;
            }
        }
        Op::Disable => {
            for i in 0..e.airplay.receivers.len() {
                actions.push((i, AirplayAction::Disable));
            }
        }
        Op::EnableReceiver { receiver_id } => {
            let i = receiver_index(&e, receiver_id)?;
            if !next.multi_receiver && i != 0 {
                return Err(Error("receiver_disabled"));
            }
            next.receivers[i].enabled = true;
            save = true;
            if e.airplay.receivers[i].commands.is_none() {
                actions.push((i, AirplayAction::Enable));
            }
        }
        Op::DisableReceiver { receiver_id } => {
            let i = receiver_index(&e, receiver_id)?;
            next.receivers[i].enabled = false;
            save = true;
            actions.push((i, AirplayAction::Disable));
        }
        Op::PairReceiver { receiver_id } => {
            let i = receiver_index(&e, receiver_id)?;
            if !next.receivers[i].enabled || (!next.multi_receiver && i != 0) {
                return Err(Error("receiver_disabled"));
            }
            if e.resources
                .admissions
                .claims
                .contains_key(&e.airplay.receivers[i].receiver_id)
            {
                return Err(Error("receiver_busy"));
            }
            pairing_window = Some(i);
            if e.airplay.receivers[i].commands.is_none() {
                actions.push((i, AirplayAction::Enable));
            }
        }
        Op::MixSource {
            source_id,
            session_id,
            gain_db,
            muted,
            solo,
        } => {
            if !gain_db.is_finite() || !(-96.0..=12.0).contains(gain_db) {
                return Err(Error("invalid_gain"));
            }
            let i =
                session_index(&e, source_id, Some(*session_id))?.ok_or(Error("session_changed"))?;
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            source.gain_db = *gain_db;
            source.muted = *muted;
            save = true;
            actions.push((
                i,
                AirplayAction::Mix {
                    gain_db: *gain_db,
                    muted: *muted,
                    solo: *solo,
                },
            ));
        }
        Op::DisconnectSource {
            source_id,
            session_id,
        } => {
            let i =
                session_index(&e, source_id, Some(*session_id))?.ok_or(Error("session_changed"))?;
            next.sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?
                .blocked = true;
            save = true;
            actions.push((i, AirplayAction::Disconnect));
        }
        Op::RevokeSource {
            source_id,
            session_id,
        } => {
            let active = session_index(&e, source_id, *session_id)?;
            next.revoke_source(source_id)
                .map_err(|_| Error("source_unknown"))?;
            save = true;
            if let Some(i) = active {
                actions.push((i, AirplayAction::Revoke));
            }
        }
        Op::AllowSource { source_id } => {
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            if source.revoked {
                return Err(Error("pairing_revoked"));
            }
            source.blocked = false;
            save = true;
        }
        Op::RepairSource {
            source_id,
            receiver_id,
        } => {
            let i = receiver_index(&e, receiver_id)?;
            session_index(&e, source_id, None)?;
            if !next.receivers[i].enabled || (!next.multi_receiver && i != 0) {
                return Err(Error("receiver_disabled"));
            }
            if e.resources
                .admissions
                .claims
                .contains_key(&e.airplay.receivers[i].receiver_id)
            {
                return Err(Error("receiver_busy"));
            }
            pairing_window = Some(i);
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            source.revoked = false;
            source.blocked = false;
            // Old trust is discarded; a new explicit PIN binding is required.
            next.bindings.retain(|b| b.source_id != *source_id);
            save = true;
            if e.airplay.receivers[i].commands.is_none() {
                actions.push((i, AirplayAction::Enable));
            }
        }
        Op::RenameReceiver { receiver_id, name } => {
            let i = receiver_index(&e, receiver_id)?;
            if e.airplay.receivers[i].commands.is_some()
                || e.resources
                    .admissions
                    .claims
                    .contains_key(&e.airplay.receivers[i].receiver_id)
            {
                return Err(Error("receiver_busy"));
            }
            if name.trim().is_empty()
                || name.len() > 50
                || name.chars().any(char::is_control)
                || next
                    .receivers
                    .iter()
                    .enumerate()
                    .any(|(j, r)| j != i && r.name == *name)
            {
                return Err(Error("invalid_receiver_name"));
            }
            next.receivers[i].name = name.clone();
            save = true;
        }
        Op::AliasSource { source_id, alias } => {
            if alias.as_ref().is_some_and(|name| {
                name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control)
            }) {
                return Err(Error("invalid_source_alias"));
            }
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            source.alias = alias.clone();
            save = true;
        }
        Op::PlaybackMode { source_id, mode } => {
            session_index(&e, source_id, None)?;
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            source.playback_mode =
                Some(serde_json::from_value(serde_json::to_value(mode).unwrap()).unwrap());
            save = true;
        }
    }
    // Validate every enable before any endpoint is changed. Volatile commands
    // never release Engine, so a batch cannot partially start on stale state.
    for (i, action) in &actions {
        if matches!(action, AirplayAction::Enable) {
            if e.airplay.receivers[*i].commands.is_some() {
                return Err(Error("receiver_busy"));
            }
            if !e.authority.current().output.available {
                return Err(Error("output_unavailable"));
            }
        }
    }
    if !save {
        if let Some(i) = pairing_window {
            open_pairing_window(&mut e, i)?;
        }
        for (i, action) in actions {
            apply_action(&mut e, shared, i, action)?;
        }
        return Ok(Json(finish(
            &mut e,
            &next,
            &command.command_id,
            payload,
            "applied",
            None,
        )));
    }
    // Capture the exact owner before releasing Engine. A session ending during
    // disk I/O does not undo the durable source preference/revoke; it only makes
    // its volatile audio action unnecessary. Never retarget a replacement.
    let targets = actions
        .iter()
        .map(|(i, _)| {
            (
                *i,
                e.resources
                    .admissions
                    .claims
                    .get(&e.airplay.receivers[*i].receiver_id)
                    .map(|c| (c.owner.clone(), c.context.session_id)),
            )
        })
        .collect::<Vec<_>>();
    let original = guard.as_ref().unwrap().clone();
    if matches!(command.operation, Op::RenameReceiver { .. })
        || actions
            .iter()
            .any(|(_, action)| matches!(action, AirplayAction::Enable))
    {
        e.airplay.configuration_pending = true;
    }
    if let Some((_, multi)) = configure {
        e.airplay.configuration_pending = true;
        e.resources.pending_multi_capacity = e.resources.multi_receiver || multi;
    }
    drop(e);
    let persisted = persist(&path, &original, next, configure);
    let mut e = shared.lock().map_err(|_| Error("busy"))?;
    e.airplay.configuration_pending = false;
    e.resources.pending_multi_capacity = false;
    let persisted = persisted?;
    let next = persisted.profile;
    *guard = Some(next.clone());
    if configure.is_some() {
        let listen = e.airplay.receivers[0].listen;
        for r in next.receivers.iter().skip(e.airplay.receivers.len()) {
            let mut endpoint = EndpointState::new(
                path.parent().unwrap().join(r.receiver_id.to_string()),
                listen,
            );
            endpoint.receiver_id = r.receiver_id;
            endpoint.receiver_uuid = r.receiver_uuid;
            endpoint.name = r.name.clone();
            e.airplay.receivers.push(endpoint);
        }
        e.airplay.multi_receiver = next.multi_receiver;
        e.resources.multi_receiver = next.multi_receiver;
        // Keep retained workers and sessions intact. Global-off configuration
        // allocates identities only; live configuration starts newly enabled idle entries.
        for (i, saved) in next.receivers.iter().enumerate() {
            if !saved.enabled && e.airplay.receivers[i].commands.is_some() {
                actions.push((i, AirplayAction::Disable));
            } else if saved.enabled && running && e.airplay.receivers[i].commands.is_none() {
                actions.push((i, AirplayAction::Enable));
            }
        }
    }
    for receiver in &mut e.airplay.receivers {
        if let Some(saved) = next
            .receivers
            .iter()
            .find(|r| r.receiver_id == receiver.receiver_id)
        {
            receiver.name.clone_from(&saved.name);
        }
    }
    if matches!(
        command.operation,
        Op::RevokeSource { .. }
            | Op::DisconnectSource { .. }
            | Op::AllowSource { .. }
            | Op::RepairSource { .. }
    ) {
        for receiver in &mut e.airplay.receivers {
            receiver.trust_generation += 1;
        }
    }
    let mut effect_error = persisted.warning;
    if let Some(i) = pairing_window
        && let Err(error) = open_pairing_window(&mut e, i)
    {
        e.airplay.receivers[i].stop();
        effect_error = Some(error.0);
    }
    let mut ended = false;
    for (i, action) in actions {
        let expected = targets
            .iter()
            .find(|(index, _)| *index == i)
            .and_then(|(_, target)| target.clone());
        let current = e
            .resources
            .admissions
            .claims
            .get(&e.airplay.receivers[i].receiver_id)
            .map(|c| (c.owner.clone(), c.context.session_id));
        if matches!(
            action,
            AirplayAction::Mix { .. } | AirplayAction::Disconnect | AirplayAction::Revoke
        ) && expected != current
        {
            ended = true;
            continue;
        }
        if let Err(error) = apply_action(&mut e, shared, i, action) {
            // Durable denial is authoritative even if the worker's queue is full.
            // Stop this endpoint rather than reporting the saved operation as
            // unexecuted or leaving its active gate open.
            e.airplay.receivers[i].stop();
            e.resources.airplay_mix[i] = None;
            effect_error = Some(Error::from(error).0);
        }
    }
    let outcome = if !persisted.complete {
        "configuration_partial"
    } else if ended {
        "saved_session_ended"
    } else {
        "applied"
    };
    Ok(Json(finish(
        &mut e,
        &next,
        &command.command_id,
        payload,
        outcome,
        effect_error,
    )))
}

fn open_pairing_window(e: &mut Engine, index: usize) -> std::result::Result<(), Error> {
    let receiver = &mut e.airplay.receivers[index];
    if e.resources
        .admissions
        .claims
        .contains_key(&receiver.receiver_id)
    {
        return Err(Error("receiver_busy"));
    }
    let generation = receiver
        .trust_generation
        .checked_add(1)
        .ok_or(Error("busy"))?;
    let deadline = Instant::now() + PAIRING_WINDOW;
    let pin = format!("{:04}", Uuid::new_v4().as_u128() % 10000);
    if let Some(sender) = receiver.commands.as_ref() {
        sender
            .try_send(Action::PairWindow {
                trust_generation: generation,
                pin,
                deadline,
                attempts: 0,
            })
            .map_err(|_| Error("busy"))?;
    }
    receiver.trust_generation = generation;
    receiver.pairing_attempts = 0;
    receiver.pairing_in_progress = false;
    receiver.pairing_deadline = Some(deadline);
    receiver.status.pairing_pin = None;
    receiver.status.revision += 1;
    e.airplay.pairing_leases.remove(&receiver.receiver_id);
    Ok(())
}

fn finish(
    e: &mut Engine,
    profile: &ReceiverProfile,
    command_id: &str,
    payload: String,
    outcome: &str,
    warning: Option<&str>,
) -> serde_json::Value {
    e.airplay.revision += 1;
    let state = e.authority.snapshot();
    // A full bounded Mixer queue can be transient. Durable commands still have
    // a successful receipt; the worker owner retries the newest desired mix.
    e.airplay.mixer_dirty = e.resources.prepare(&state, None).is_err();
    let mut result = view(e, profile, true);
    result["outcome"] = serde_json::json!(outcome);
    result["media_pending"] = serde_json::json!(e.airplay.mixer_dirty);
    if let Some(warning) = warning {
        result["warning"] = serde_json::json!(warning);
    }
    if e.airplay.receipts.len() == 128 {
        e.airplay.receipts.pop_front();
    }
    let mut receipt = result.clone();
    if let Some(receivers) = receipt["receivers"].as_array_mut() {
        for receiver in receivers {
            receiver.as_object_mut().unwrap().remove("pairing_pin");
        }
    }
    e.airplay
        .receipts
        .push_back((command_id.to_owned(), payload, receipt));
    result
}

struct Persisted {
    profile: ReceiverProfile,
    complete: bool,
    warning: Option<&'static str>,
}
fn same_profile(a: &ReceiverProfile, b: &ReceiverProfile) -> bool {
    serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
}
fn persist_change(
    path: &std::path::Path,
    original: &ReceiverProfile,
    next: ReceiverProfile,
    configure: Option<(usize, bool)>,
) -> std::result::Result<Persisted, Error> {
    persist_with(
        path,
        original,
        next,
        configure,
        |path, next, name| profile::add_receiver(path, next, name, false).map(|_| ()),
        profile::save,
    )
}
fn persist_with(
    path: &std::path::Path,
    original: &ReceiverProfile,
    mut next: ReceiverProfile,
    configure: Option<(usize, bool)>,
    mut add: impl FnMut(&std::path::Path, &mut ReceiverProfile, &str) -> neonmix_identity::Result<()>,
    save: impl FnOnce(&std::path::Path, &ReceiverProfile) -> neonmix_identity::Result<()>,
) -> std::result::Result<Persisted, Error> {
    let mut final_save_attempted = false;
    let result = (|| -> neonmix_identity::Result<()> {
        if let Some((count, multi)) = configure {
            // add_receiver publishes one identity at a time. Keep each staged
            // entry disabled under the OLD capacity contract until all exist.
            while next.receivers.len() < count {
                let name = format!(
                    "{} · {}",
                    next.receivers[0].name.chars().take(10).collect::<String>(),
                    next.receivers.len() + 1
                );
                add(path, &mut next, &name)?;
            }
            next.multi_receiver = multi;
            for (i, receiver) in next.receivers.iter_mut().enumerate() {
                receiver.enabled = i < count;
            }
        }
        final_save_attempted = true;
        save(path, &next)
    })();
    if result.is_ok() {
        return Ok(Persisted {
            profile: next,
            complete: true,
            warning: None,
        });
    }
    // Atomic rename can succeed before a directory fsync error. Read back to
    // distinguish a committed command from an unchanged file and to recover
    // harmless disabled identities if multi-step preparation partially saved.
    let actual = profile::load(path).map_err(|_| Error("profile_write_failed"))?;
    if final_save_attempted && same_profile(&actual, &next) {
        Ok(Persisted {
            profile: actual,
            complete: true,
            warning: Some("profile_durability_unconfirmed"),
        })
    } else if configure.is_some() && !same_profile(&actual, original) {
        Ok(Persisted {
            profile: actual,
            complete: false,
            warning: Some("profile_write_failed"),
        })
    } else {
        Err(Error("profile_write_failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        shared: Shared,
        headers: HeaderMap,
        directory: PathBuf,
        profile: ReceiverProfile,
        source: String,
        _mixer: Mixer,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/tmp")
                .join(format!("airplay-api-{}", Uuid::new_v4()));
            neonmix_identity::files::private_dir(&directory).unwrap();
            let token = neonmix_identity::secret();
            let authority = Authority::new("test-output".into(), "admin".into(), &token).unwrap();
            let mut profile = profile::create(
                &directory.join("receiver.json"),
                authority.current().hub_id,
                "API test",
            )
            .unwrap();
            let source = profile
                .register_source(
                    profile.receivers[0].receiver_id,
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                    None,
                )
                .unwrap();
            profile::save(&directory.join("receiver.json"), &profile).unwrap();
            let origin = Instant::now();
            let (mixer, control, inputs, stats) = Mixer::new(origin).unwrap();
            let (pem, certificate, _) = crate::certificate().unwrap();
            let shared = Arc::new(Mutex::new(Engine {
                pairings: Default::default(),
                pair_window: Instant::now(),
                pair_attempts: 0,
                room_name: "API test".into(),
                certificate,
                event_slots: Arc::new(tokio::sync::Semaphore::new(8)),
                output_stats: Default::default(),
                authority,
                resources: Resources {
                    lanes: inputs
                        .into_iter()
                        .map(|producer| super::super::super::Lane {
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
                    airplay_mix: [None; 4],
                    admissions: Default::default(),
                    multi_receiver: false,
                    pending_multi_capacity: false,
                    native_reservation: None,
                    retiring: Vec::new(),
                },
                airplay: super::super::State::new(
                    directory.clone(),
                    "127.0.0.1:0".parse().unwrap(),
                ),
                errors: Vec::new(),
                state_path: None,
            }));
            ensure_profile(&shared).map_err(Error::from).unwrap();
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
            Self {
                shared,
                headers,
                directory,
                profile,
                source,
                _mixer: mixer,
            }
        }
        fn command(&self, operation: Op) -> AirplayCommandV2 {
            AirplayCommandV2 {
                command_id: Uuid::new_v4().to_string(),
                expected_revision: self.shared.lock().unwrap().airplay.revision,
                operation,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn committed_revoke_has_receipt_when_session_ends_and_mixer_queue_fills_during_save() {
        let f = Fixture::new();
        let owner = Owner {
            receiver: f.profile.receivers[0].receiver_id,
            generation: 1,
            connection: 7,
            request: 9,
            source: f.source.clone(),
        };
        let claim = f
            .shared
            .lock()
            .unwrap()
            .resources
            .admissions
            .reserve(owner.clone(), 0, false, Some(0), &[], Instant::now())
            .unwrap();
        let command = f.command(Op::RevokeSource {
            source_id: f.source.clone(),
            session_id: Some(claim.context.session_id),
        });
        let Json(result) = execute(
            &f.shared,
            &f.headers,
            command.clone(),
            |path, original, next, config| {
                // This callback runs with profile locked and Engine unlocked.
                let mut e = f.shared.try_lock().expect("disk I/O must not hold Engine");
                e.resources.admissions.release(&owner);
                while e.resources.control.has_capacity() {
                    e.resources.control.apply(MixerConfig::default()).unwrap();
                }
                drop(e);
                persist_change(path, original, next, config)
            },
        )
        .unwrap();
        assert_eq!(result["outcome"], "saved_session_ended");
        assert_eq!(result["media_pending"], true);
        assert!(
            profile::load(&f.directory.join("receiver.json"))
                .unwrap()
                .sources[0]
                .revoked
        );
        assert_eq!(f.shared.lock().unwrap().airplay.receipts.len(), 1);
        let Json(replayed) = execute(&f.shared, &f.headers, command.clone(), |_, _, _, _| {
            panic!("duplicate must not write")
        })
        .unwrap();
        assert_eq!(result, replayed);
        let mut conflicting = command;
        conflicting.operation = Op::AllowSource {
            source_id: f.source.clone(),
        };
        assert_eq!(
            execute(&f.shared, &f.headers, conflicting, persist_change)
                .unwrap_err()
                .0,
            "command_id_reused"
        );
    }

    #[test]
    fn configuration_reserves_four_source_capacity_before_disk_write_and_clears_on_failure() {
        let f = Fixture::new();
        let command = f.command(Op::Configure {
            receiver_count: 2,
            multi_receiver: true,
        });
        let error = execute(&f.shared, &f.headers, command, |_, _, _, _| {
            let e = f.shared.try_lock().unwrap();
            assert!(e.airplay.configuration_pending);
            assert!(e.resources.pending_multi_capacity);
            assert!(!e.resources.admissions.native_capacity(
                5,
                e.resources.multi_receiver || e.resources.pending_multi_capacity
            ));
            Err(Error("profile_write_failed"))
        })
        .unwrap_err();
        assert_eq!(error.0, "profile_write_failed");
        let e = f.shared.lock().unwrap();
        assert!(!e.airplay.configuration_pending);
        assert!(!e.resources.pending_multi_capacity);
        assert!(!e.resources.multi_receiver);
        assert!(e.airplay.receipts.is_empty());
    }

    #[test]
    fn partial_identity_preparation_retains_disabled_recoverable_entries() {
        let f = Fixture::new();
        let mut attempts = 0;
        let command = f.command(Op::Configure {
            receiver_count: 3,
            multi_receiver: true,
        });
        let Json(result) = execute(
            &f.shared,
            &f.headers,
            command.clone(),
            |path, original, next, config| {
                persist_with(
                    path,
                    original,
                    next,
                    config,
                    |path, next, name| {
                        attempts += 1;
                        if attempts == 2 {
                            return Err("injected credential failure".into());
                        }
                        profile::add_receiver(path, next, name, false).map(|_| ())
                    },
                    profile::save,
                )
            },
        )
        .unwrap();
        assert_eq!(result["outcome"], "configuration_partial");
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        actual.validate().unwrap();
        assert_eq!(actual.receivers.len(), 2);
        assert!(!actual.receivers[1].enabled);
        assert!(!actual.multi_receiver);
        assert_eq!(f.shared.lock().unwrap().airplay.receivers.len(), 2);
        let Json(again) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(again, result);
        // A fresh command reuses the saved identity and completes the request.
        let original_id = actual.receivers[1].receiver_id;
        let command = f.command(Op::Configure {
            receiver_count: 3,
            multi_receiver: true,
        });
        let Json(result) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(result["outcome"], "applied");
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(actual.receivers[1].receiver_id, original_id);
        assert_eq!(actual.receivers.len(), 3);
        assert!(actual.multi_receiver && actual.receivers.iter().all(|r| r.enabled));
    }

    #[test]
    fn rename_success_followed_by_sync_error_is_acknowledged_as_committed() {
        let f = Fixture::new();
        let command = f.command(Op::RevokeSource {
            source_id: f.source.clone(),
            session_id: None,
        });
        let Json(result) = execute(
            &f.shared,
            &f.headers,
            command,
            |path, original, next, config| {
                persist_with(
                    path,
                    original,
                    next,
                    config,
                    |_, _, _| unreachable!(),
                    |path, profile| {
                        profile::save(path, profile)?;
                        Err("injected directory fsync failure".into())
                    },
                )
            },
        )
        .unwrap();
        assert_eq!(result["outcome"], "applied");
        assert_eq!(result["warning"], "profile_durability_unconfirmed");
        assert_eq!(f.shared.lock().unwrap().airplay.receipts.len(), 1);
        assert!(
            profile::load(&f.directory.join("receiver.json"))
                .unwrap()
                .sources[0]
                .revoked
        );
    }

    #[test]
    fn explicit_pair_window_rotates_running_idle_receiver_and_replay_does_not_reopen() {
        let f = Fixture::new();
        let (sender, receiver) = mpsc::sync_channel(4);
        let generation = {
            let mut e = f.shared.lock().unwrap();
            let entry = &mut e.airplay.receivers[0];
            entry.commands = Some(sender);
            entry.status.enabled = true;
            entry.status.ready = true;
            entry.pairing_attempts = 5;
            entry.pairing_deadline = Some(Instant::now() - Duration::from_secs(1));
            entry.status.pairing_pin = Some("old-pin".into());
            entry.trust_generation
        };
        let command = f.command(Op::PairReceiver {
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
        });
        let Json(result) = execute(&f.shared, &f.headers, command.clone(), |_, _, _, _| {
            panic!("pair window is not persistent")
        })
        .unwrap();
        match receiver.try_recv().unwrap() {
            Action::PairWindow {
                trust_generation,
                pin,
                deadline,
                attempts,
            } => {
                assert_eq!(trust_generation, generation + 1);
                assert_eq!(pin.len(), 4);
                assert!(pin.bytes().all(|b| b.is_ascii_digit()));
                assert_eq!(attempts, 0);
                assert!(deadline > Instant::now());
            }
            _ => panic!("expected scoped PairWindow"),
        }
        assert!(
            f.shared.lock().unwrap().airplay.receivers[0]
                .status
                .pairing_pin
                .is_none()
        );
        assert_eq!(result["receivers"][0]["configured_enabled"], true);
        let Json(again) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(result, again);
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn failed_pair_window_queue_does_not_reset_attempts_or_generation() {
        let f = Fixture::new();
        let (sender, _receiver) = mpsc::sync_channel(1);
        sender.try_send(Action::Allow).unwrap();
        let generation = {
            let mut e = f.shared.lock().unwrap();
            let r = &mut e.airplay.receivers[0];
            r.commands = Some(sender);
            r.pairing_attempts = 5;
            r.trust_generation
        };
        let command = f.command(Op::PairReceiver {
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command, persist_change)
                .unwrap_err()
                .0,
            "busy"
        );
        let e = f.shared.lock().unwrap();
        assert_eq!(e.airplay.receivers[0].pairing_attempts, 5);
        assert_eq!(e.airplay.receivers[0].trust_generation, generation);
        assert!(e.airplay.receipts.is_empty());
    }
    #[test]
    fn repair_running_idle_receiver_discards_old_binding_and_opens_explicit_window() {
        let f = Fixture::new();
        let revoke = f.command(Op::RevokeSource {
            source_id: f.source.clone(),
            session_id: None,
        });
        let _ = execute(&f.shared, &f.headers, revoke, persist_change).unwrap();
        let (sender, receiver) = mpsc::sync_channel(4);
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].commands = Some(sender);
            e.airplay.receivers[0].pairing_attempts = 5;
        }
        let repair = f.command(Op::RepairSource {
            source_id: f.source.clone(),
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
        });
        let _ = execute(&f.shared, &f.headers, repair, persist_change).unwrap();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Action::PairWindow { attempts: 0, .. }
        ));
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert!(!actual.sources[0].revoked && !actual.sources[0].blocked);
        assert!(actual.bindings.is_empty());
    }
    #[test]
    fn individual_disable_is_persistent_and_global_disable_preserves_configuration() {
        let f = Fixture::new();
        let command = f.command(Op::DisableReceiver {
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
        });
        let Json(result) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(result["receivers"][0]["configured_enabled"], false);
        assert!(
            !profile::load(&f.directory.join("receiver.json"))
                .unwrap()
                .receivers[0]
                .enabled
        );
        let f = Fixture::new();
        let command = f.command(Op::Disable);
        let _ = execute(&f.shared, &f.headers, command, |_, _, _, _| {
            panic!("global off must not overwrite receiver configuration")
        })
        .unwrap();
        assert!(
            profile::load(&f.directory.join("receiver.json"))
                .unwrap()
                .receivers[0]
                .enabled
        );
    }
    #[test]
    fn metadata_edits_keep_receiver_identity_and_redact_cached_pairing_pins() {
        let f = Fixture::new();
        let rename = f.command(Op::RenameReceiver {
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
            name: "新入口".into(),
        });
        let Json(result) = execute(&f.shared, &f.headers, rename, persist_change).unwrap();
        assert_eq!(result["receivers"][0]["name"], "新入口");
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(
            actual.receivers[0].receiver_uuid,
            f.profile.receivers[0].receiver_uuid
        );
        assert_eq!(
            actual.receivers[0].key_reference,
            f.profile.receivers[0].key_reference
        );
        {
            let mut e = f.shared.lock().unwrap();
            let r = &mut e.airplay.receivers[0];
            r.status.enabled = true;
            r.status.ready = true;
            r.status.source_id = Some(f.source.clone());
            r.status.pairing_pin = Some("1234".into());
            r.pairing_deadline = Some(Instant::now() + PAIRING_WINDOW);
        }
        let alias = f.command(Op::AliasSource {
            source_id: f.source.clone(),
            alias: Some("客厅手机".into()),
        });
        let Json(result) = execute(&f.shared, &f.headers, alias.clone(), persist_change).unwrap();
        assert_eq!(result["receivers"][0]["source_name"], "客厅手机");
        assert_eq!(result["receivers"][0]["pairing_pin"], "1234");
        f.shared.lock().unwrap().airplay.receivers[0].pairing_deadline =
            Some(Instant::now() - Duration::from_secs(1));
        let Json(replayed) = execute(&f.shared, &f.headers, alias, persist_change).unwrap();
        assert!(replayed["receivers"][0]["pairing_pin"].is_null());
        let e = f.shared.lock().unwrap();
        assert!(
            e.airplay
                .receipts
                .iter()
                .all(|(_, _, v)| v["receivers"][0]["pairing_pin"].is_null())
        );
    }
    #[test]
    fn command_ids_require_canonical_uuid_spelling_before_deduplication() {
        let f = Fixture::new();
        let mut command = f.command(Op::Disable);
        command.command_id = Uuid::new_v4().simple().to_string();
        assert_eq!(
            execute(&f.shared, &f.headers, command, persist_change)
                .unwrap_err()
                .0,
            "invalid_command_id"
        );
        assert!(f.shared.lock().unwrap().airplay.receipts.is_empty());
    }
    #[test]
    fn live_configuration_stops_only_removed_idle_entries_and_preserves_retained_claim() {
        let f = Fixture::new();
        let command = f.command(Op::Configure {
            receiver_count: 3,
            multi_receiver: true,
        });
        let _ = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        let mut receivers = Vec::new();
        let owner = {
            let mut e = f.shared.lock().unwrap();
            for r in &mut e.airplay.receivers {
                let (sender, receiver) = mpsc::sync_channel(4);
                r.commands = Some(sender);
                r.status.enabled = true;
                receivers.push(receiver);
            }
            let owner = Owner {
                receiver: e.airplay.receivers[0].receiver_id,
                generation: 1,
                connection: 1,
                request: 1,
                source: f.source.clone(),
            };
            e.resources
                .admissions
                .reserve(owner.clone(), 0, true, Some(0), &[], Instant::now())
                .unwrap();
            e.airplay.receivers[0].gate.store(true, Release);
            owner
        };
        let command = f.command(Op::Configure {
            receiver_count: 2,
            multi_receiver: true,
        });
        let Json(result) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(result["outcome"], "applied");
        assert!(matches!(receivers[2].try_recv().unwrap(), Action::Stop));
        assert!(receivers[0].try_recv().is_err() && receivers[1].try_recv().is_err());
        {
            let e = f.shared.lock().unwrap();
            assert!(e.resources.admissions.claims.contains_key(&owner.receiver));
            assert!(e.airplay.receivers[0].gate.load(Acquire));
            assert!(e.airplay.receivers[0].status.enabled);
            assert!(!e.airplay.receivers[2].status.enabled);
        }
        let command = f.command(Op::Configure {
            receiver_count: 1,
            multi_receiver: false,
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command, persist_change)
                .unwrap_err()
                .0,
            "receiver_busy"
        );
        let command = f.command(Op::Configure {
            receiver_count: 1,
            multi_receiver: true,
        });
        // An independent pairing lease on the removed second entry also blocks shrink.
        {
            let mut e = f.shared.lock().unwrap();
            let id = e.airplay.receivers[1].receiver_id;
            e.airplay.pairing_leases.insert(
                id,
                PairingLease {
                    generation: 1,
                    trust_generation: 1,
                    connection: 2,
                    request: 2,
                    deadline: Instant::now() + PAIRING_WINDOW,
                },
            );
        }
        assert_eq!(
            execute(&f.shared, &f.headers, command, persist_change)
                .unwrap_err()
                .0,
            "receiver_busy"
        );
    }
    #[test]
    fn snapshot_rechecks_role_after_waiting_for_profile_transaction() {
        let fixture = Fixture::new();
        let result = snapshot_with(&fixture.shared, &fixture.headers, || {
            let mut e = fixture.shared.lock().unwrap();
            let target = authenticate(&e, &fixture.headers)
                .unwrap_or_else(|_| panic!("fixture authentication"))
                .device_id();
            e.authority
                .add_device(
                    "other admin".into(),
                    neonmix_control::Role::Admin,
                    &neonmix_identity::secret(),
                )
                .unwrap();
            let mut state = e.authority.persistent();
            state.devices.get_mut(&target).unwrap().revoked = true;
            e.authority = Authority::restore(state).unwrap();
        });
        assert!(matches!(result, Err(Error("unauthenticated"))));
    }
    #[tokio::test]
    async fn member_snapshot_of_uninitialized_airplay_is_read_only() {
        let f = Fixture::new();
        let untouched = f.directory.join("uninitialized-airplay");
        let token = neonmix_identity::secret();
        let hub = {
            let mut e = f.shared.lock().unwrap();
            e.authority
                .add_device("member".into(), neonmix_control::Role::Member, &token)
                .unwrap();
            e.airplay = super::super::State::new(untouched.clone(), "127.0.0.1:0".parse().unwrap());
            e.authority.current().hub_id
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        let Json(result) = snapshot(axum::extract::State(f.shared.clone()), headers)
            .await
            .unwrap();
        assert_eq!(result["receivers"][0]["receiver_id"], hub.to_string());
        assert_eq!(result["receivers"][0]["configured_enabled"], true);
        assert_eq!(result["sources"], serde_json::json!([]));
        assert!(!untouched.exists());
    }
    #[test]
    fn stale_session_is_rejected_before_any_durable_change() {
        let f = Fixture::new();
        let command = f.command(Op::DisconnectSource {
            source_id: f.source.clone(),
            session_id: 777,
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "stale command must not write"
            ))
            .unwrap_err()
            .0,
            "session_changed"
        );
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert!(!actual.sources[0].blocked && !actual.sources[0].revoked);
    }
}
