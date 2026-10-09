//! Targeted administrator API. File transactions are serialized separately from audio state.
use super::*;
use neonmix_airplay_adapter::control::{AirplayActionV2 as Op, AirplayCommandV2};
#[derive(Debug)]
pub(crate) struct Error(pub(super) &'static str);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self.0 {
            "profile_durability_unconfirmed" | "configuration_recovery_required" | "busy" => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            "upgrade_required" => StatusCode::UPGRADE_REQUIRED,
            "unauthenticated" => StatusCode::UNAUTHORIZED,
            "permission_denied" => StatusCode::FORBIDDEN,
            "stale_revision" | "session_changed" | "receiver_busy" | "room_capacity_full"
            | "command_id_reused" | "snapshot_required" => StatusCode::CONFLICT,
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
fn view(e: &mut Engine, p: &ReceiverProfile, admin: bool) -> serde_json::Value {
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
    let active = e
        .resources
        .admissions
        .claims
        .values()
        .filter(|c| c.active)
        .count();
    let reserved = e
        .resources
        .admissions
        .claims
        .values()
        .filter(|c| !c.active)
        .count();
    // Configured AirPlay entries determine capacity, even while reception is off.
    let limit = p.receivers.iter().filter(|r| r.enabled).count();
    let confirmed = e
        .airplay
        .recovery
        .as_ref()
        .map(|r| &r.original)
        .unwrap_or(p);
    let config_revision = confirmed.config_revision;
    let configuration = serde_json::json!({"multi_receiver":confirmed.multi_receiver,
        "receivers":confirmed.receivers.iter().map(|r|serde_json::json!({"receiver_id":r.receiver_id,"name":r.name,"enabled":r.enabled,"default_playback_mode":r.default_playback_mode})).collect::<Vec<_>>(),
        "sources":confirmed.sources.iter().map(|s|serde_json::json!({"source_id":s.source_id,"alias":s.alias,"last_name":s.last_name,"blocked":s.blocked,"revoked":s.revoked,"gain_db":s.gain_db,"muted":s.muted,"playback_mode":s.playback_mode})).collect::<Vec<_>>()});
    let receiver_state = receivers
        .iter()
        .enumerate()
        .map(|(index, r)| {
            let mut value = serde_json::Map::new();
            for key in [
                "receiver_id",
                "name",
                "configured_enabled",
                "enabled",
                "ready",
                "discovery_state",
                "visibility_reason",
                "active",
                "playback_allowed",
                "source_id",
                "source_name",
                "source_revoked",
                "playback_mode",
                "failure_stage",
                "error",
                "mix",
            ] {
                if let Some(field) = r.get(key) {
                    value.insert(key.to_owned(), field.clone());
                }
            }
            value.insert(
                "configured_enabled".into(),
                serde_json::json!(
                    confirmed
                        .receivers
                        .iter()
                        .find(|saved| saved.receiver_id == e.airplay.receivers[index].receiver_id)
                        .is_some_and(|saved| saved.enabled)
                ),
            );
            // Pairing counters/remaining time/PIN and PCM statistics are observations,
            // not part of this role-independent versioned business state.
            value.insert(
                "pairing_window_open".into(),
                serde_json::json!(e.airplay.receivers[index].status.pairing_window_open),
            );
            serde_json::Value::Object(value)
        })
        .collect::<Vec<_>>();
    let session_state = sessions
        .iter()
        .map(|s| {
            let mut value = serde_json::Map::new();
            for key in [
                "receiver_id",
                "source_id",
                "source_name",
                "session_id",
                "stream_id",
                "stream_epoch",
                "format_epoch",
                "mapping_id",
                "lane",
                "active",
                "playback_allowed",
                "mix",
                "playback_mode",
            ] {
                if let Some(field) = s.get(key) {
                    value.insert(key.to_owned(), field.clone());
                }
            }
            serde_json::Value::Object(value)
        })
        .collect::<Vec<_>>();
    let state = serde_json::json!({"configuration":configuration,"receivers":receiver_state,"sessions":session_state,
        "effective_sources":sources,"persistence_pending":e.airplay.configuration_pending});
    publish_business(e, &state);
    serde_json::json!({"command_version":3,"revision":e.airplay.revision,"event_sequence":if e.airplay.sequence_exhausted {None} else {Some(e.airplay.revision)},"snapshot_required":e.airplay.sequence_exhausted,"config_revision":config_revision,
        "runtime_epoch":e.authority.current().runtime_epoch,"capabilities":["patch_mix_source","runtime_epoch","version_domains"],"state":state,
        "native_versions":{"config_revision":e.authority.current().config_revision,"event_sequence":e.authority.current().event_sequence},
        "media_application":e.resources.application(e.authority.current().runtime_epoch,Instant::now()),
        "enabled":e.airplay.receivers.iter().any(|r|r.status.enabled),"persistence_pending":e.airplay.configuration_pending,
        "pending_command_id":if admin { e.airplay.recovery.as_ref().map(|r|r.command.command_id.as_str()).or_else(||e.airplay.profile_recovery.as_ref().map(|r|r.request_id.as_str())) } else { None },
        "multi_receiver":e.airplay.multi_receiver,"receivers":receivers,"sources":sources,"sessions":sessions,
        "capacity":{"limit":limit,"active":active,"reserved":reserved,"available":limit.saturating_sub(active+reserved)}})
}

fn publish_business(e: &mut Engine, state: &serde_json::Value) {
    if e.airplay
        .published_business
        .as_ref()
        .is_some_and(|(sequence, old)| *sequence == e.airplay.revision && old != state)
    {
        e.airplay.advance_event();
    }
    e.airplay.published_business = Some((e.airplay.revision, state.clone()));
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
    let mut e = shared.lock().map_err(|_| Error("busy"))?;
    // Trust-file contention must not preserve an administrator's old PIN access.
    let principal = authenticate(&e, headers)?;
    let admin = e
        .authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_some_and(|d| d.role == neonmix_control::Role::Admin);
    if let Some(profile) = guard.as_ref() {
        return Ok(Json(view(&mut e, profile, admin)));
    }
    // Reading an uninitialized room must not create credentials or migrate files.
    // The first real receiver uses the durable Hub UUID when an admin enables it.
    let mut receiver =
        serde_json::to_value(e.airplay.receivers[0].display_status_at(Instant::now())).unwrap();
    receiver["receiver_id"] = serde_json::json!(e.authority.current().hub_id);
    receiver["name"] = serde_json::json!(speaker_name(&e.room_name));
    receiver["configured_enabled"] = serde_json::json!(true);
    receiver.as_object_mut().unwrap().remove("pairing_pin");
    Ok(Json(
        serde_json::json!({"revision":e.airplay.revision,"enabled":false,"multi_receiver":false,
        "receivers":[receiver],"sources":[],"sessions":[],
        "capacity":{"limit":1,"active":0,"reserved":0,"available":1}}),
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
    command.validate_version().map_err(Error)?;
    let id = Uuid::parse_str(&command.command_id).map_err(|_| Error("invalid_command_id"))?;
    if id.to_string() != command.command_id {
        return Err(Error("invalid_command_id"));
    }
    let payload = serde_json::to_string(&command).map_err(|_| Error("invalid_command"))?;
    {
        let e = shared.lock().map_err(|_| Error("busy"))?;
        let principal = authenticate(&e, headers)?;
        if command
            .credential_id
            .as_ref()
            .is_some_and(|id| *id != principal.device_id().to_string())
        {
            return Err(Error("unauthenticated"));
        }
        if command
            .runtime_epoch
            .as_ref()
            .is_some_and(|epoch| *epoch != e.authority.current().runtime_epoch.to_string())
        {
            return Err(Error("snapshot_required"));
        }
        if let Some((_, old, response)) = e
            .airplay
            .receipts
            .iter()
            .find(|(id, _, _)| id == &command.command_id)
        {
            return if old == &payload {
                Ok(Json(response.clone()))
            } else {
                Err(Error("command_id_reused"))
            };
        }
    }
    ensure_profile(shared)?;

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
    if command
        .credential_id
        .as_ref()
        .is_some_and(|id| *id != principal.device_id().to_string())
    {
        return Err(Error("unauthenticated"));
    }
    if command
        .runtime_epoch
        .as_ref()
        .is_some_and(|epoch| *epoch != e.authority.current().runtime_epoch.to_string())
    {
        return Err(Error("snapshot_required"));
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
    if let Some(recovery) = e.airplay.recovery.as_ref() {
        if recovery.command.command_id == command.command_id && recovery.payload != payload {
            return Err(Error("command_id_reused"));
        }
        if recovery.command.command_id != command.command_id {
            return Err(Error("profile_durability_unconfirmed"));
        }
        let recovery = e.airplay.recovery.take().unwrap();
        drop(e);
        let persisted = persist(
            &recovery.path,
            &recovery.original,
            recovery.next.clone(),
            if recovery.staged {
                None
            } else {
                recovery.configure
            },
        );
        let mut e = shared.lock().map_err(|_| Error("busy"))?;
        return settle(&mut e, shared, &mut guard, recovery, persisted, true);
    }
    if e.airplay.configuration_pending {
        return Err(Error("profile_durability_unconfirmed"));
    }
    let _ = view(&mut e, &next, true);
    if e.airplay.sequence_exhausted {
        return Err(Error("snapshot_required"));
    }
    if command.command_version == 3 {
        if command
            .expected_config_revision
            .is_some_and(|r| r != next.config_revision)
            || command
                .expected_event_sequence
                .is_some_and(|s| s != e.airplay.revision)
        {
            return Err(Error("stale_revision"));
        }
    } else if command.expected_revision != Some(e.airplay.revision) {
        return Err(Error("stale_revision"));
    }
    if !e.resources.control.has_capacity()
        && !matches!(
            command.operation,
            Op::Disable
                | Op::DisableReceiver { .. }
                | Op::DisconnectSource { .. }
                | Op::RevokeSource { .. }
        )
    {
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
        Op::PatchMixSource {
            source_id,
            session_id,
            gain_db,
            muted,
            solo,
        } => {
            if gain_db.is_none() && muted.is_none() && solo.is_none() {
                return Err(Error("empty_patch"));
            }
            if gain_db.is_some_and(|g| !g.is_finite() || !(-96.0..=12.0).contains(&g)) {
                return Err(Error("invalid_gain"));
            }
            let i =
                session_index(&e, source_id, Some(*session_id))?.ok_or(Error("session_changed"))?;
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.source_id == *source_id)
                .ok_or(Error("source_unknown"))?;
            // Complete unspecified fields from this locked authoritative
            // source/session, never from a client-provided stale triple.
            if let Some(gain) = gain_db {
                source.gain_db = *gain;
            }
            if let Some(mute) = muted {
                source.muted = *mute;
            }
            save = gain_db.is_some() || muted.is_some();
            actions.push((
                i,
                AirplayAction::Mix {
                    gain_db: source.gain_db,
                    muted: source.muted,
                    solo: solo.unwrap_or(e.airplay.receivers[i].status.mix.solo),
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
    if configure.is_some() {
        e.airplay.configuration_pending = true;
    }
    let recovery = Recovery {
        path,
        original,
        next,
        configure,
        staged: false,
        restrictions_applied: false,
        actions,
        targets,
        pairing_window,
        running,
        command,
        payload,
    };
    drop(e);
    let persisted = persist(
        &recovery.path,
        &recovery.original,
        recovery.next.clone(),
        configure,
    );
    let mut e = shared.lock().map_err(|_| Error("busy"))?;
    settle(&mut e, shared, &mut guard, recovery, persisted, false)
}

pub(super) struct Recovery {
    path: PathBuf,
    original: ReceiverProfile,
    next: ReceiverProfile,
    configure: Option<(usize, bool)>,
    staged: bool,
    restrictions_applied: bool,
    actions: Vec<(usize, AirplayAction)>,
    targets: Vec<(usize, Option<(Owner, u64)>)>,
    pairing_window: Option<usize>,
    running: bool,
    command: AirplayCommandV2,
    payload: String,
}

pub(super) struct ProfileRecovery {
    pub(super) request_id: String,
    path: PathBuf,
    next: ReceiverProfile,
}
/// Caller holds the profile lock. The worker never adopts an unconfirmed grant.
pub(super) fn persist_background(
    shared: &Shared,
    path: &std::path::Path,
    original: &ReceiverProfile,
    next: &mut ReceiverProfile,
    save: impl FnOnce(&std::path::Path, &ReceiverProfile) -> neonmix_identity::Result<()>,
) -> crate::Result<()> {
    original.prepare_change(next)?;
    {
        let mut e = shared.lock().map_err(|_| "airplay_state_busy")?;
        if e.airplay.configuration_pending {
            return Err("profile_durability_unconfirmed".into());
        }
        e.airplay.configuration_pending = true;
    }
    let result = save(path, next);
    let mut e = shared.lock().map_err(|_| "airplay_state_busy")?;
    if result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<neonmix_identity::files::PublicationError>())
        .is_some_and(|error| {
            error.publication != neonmix_identity::files::Publication::NotPublished
        })
    {
        e.airplay.profile_recovery = Some(ProfileRecovery {
            request_id: Uuid::new_v4().to_string(),
            path: path.to_owned(),
            next: next.clone(),
        });
    } else {
        e.airplay.configuration_pending = false;
    }
    result
}

pub(in crate::server) fn pending_request(e: &Engine) -> Option<Uuid> {
    e.airplay
        .recovery
        .as_ref()
        .map(|r| r.command.command_id.as_str())
        .or_else(|| {
            e.airplay
                .profile_recovery
                .as_ref()
                .map(|r| r.request_id.as_str())
        })
        .and_then(|id| Uuid::parse_str(id).ok())
}

pub(in crate::server) fn pending(e: &Engine) -> bool {
    e.airplay.configuration_pending
}

/// Operator reconciliation uses the exact frozen profile and effects. The
/// current admin may recover a request from an expired/revoked original caller.
pub(in crate::server) fn recover(
    shared: &Shared,
    headers: &HeaderMap,
    request: Uuid,
    epoch: Uuid,
) -> std::result::Result<Option<serde_json::Value>, ApiError> {
    let profiles = shared
        .lock()
        .map_err(|_| ControlError::Busy)?
        .airplay
        .profile
        .clone();
    let mut guard = profiles.lock().map_err(|_| ControlError::Busy)?;
    let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
    let principal = authenticate(&e, headers)?;
    if e.authority.current().runtime_epoch != epoch {
        return Err(ControlError::SnapshotRequired.into());
    }
    if e.authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_none_or(|d| d.role != neonmix_control::Role::Admin)
    {
        return Err(ControlError::PermissionDenied.into());
    }
    if let Some((_, _, result)) = e
        .airplay
        .receipts
        .iter()
        .find(|(id, _, _)| id == &request.to_string())
    {
        return Ok(Some(result.clone()));
    }
    if pending_request(&e) != Some(request) {
        return Ok(None);
    }
    if let Some(recovery) = e.airplay.recovery.take() {
        drop(e);
        let persisted = persist_change(
            &recovery.path,
            &recovery.original,
            recovery.next.clone(),
            if recovery.staged {
                None
            } else {
                recovery.configure
            },
        );
        let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
        let Json(response) = settle(&mut e, shared, &mut guard, recovery, persisted, true)
            .map_err(|_| ControlError::DurabilityUnconfirmed)?;
        e.cache_recovery(request, response.clone());
        return Ok(Some(response));
    }
    let recovery = e.airplay.profile_recovery.take().unwrap();
    drop(e);
    let saved = profile::save(&recovery.path, &recovery.next);
    let mut e = shared.lock().map_err(|_| ControlError::Busy)?;
    if saved.is_err() {
        e.airplay.profile_recovery = Some(recovery);
        return Err(ControlError::DurabilityUnconfirmed.into());
    }
    *guard = Some(recovery.next.clone());
    e.airplay.configuration_pending = false;
    for receiver in &mut e.airplay.receivers {
        receiver.trust_generation += 1;
    }
    e.airplay.advance_event();
    let mut response = view(&mut e, &recovery.next, true);
    response["outcome"] = serde_json::json!("saved_worker_stopped");
    e.cache_recovery(request, response.clone());
    Ok(Some(response))
}

fn settle(
    e: &mut Engine,
    shared: &Shared,
    guard: &mut Option<ReceiverProfile>,
    mut recovery: Recovery,
    persisted: std::result::Result<Persisted, Error>,
    retry: bool,
) -> std::result::Result<Json<serde_json::Value>, Error> {
    let persisted = match persisted {
        Ok(persisted) => persisted,
        Err(error) if !retry && error.0 != "configuration_recovery_required" => {
            e.airplay.configuration_pending = false;
            return Err(error);
        }
        Err(_) => {
            e.airplay.configuration_pending = true;
            e.airplay.recovery = Some(recovery);
            return Err(Error("profile_durability_unconfirmed"));
        }
    };
    if persisted.warning.is_some() || !persisted.complete {
        recovery.next = persisted.profile;
        if persisted.complete {
            recovery.staged = true;
        }
        // The effective profile is an intersection: publication can deny, never grant.
        let mut effective = recovery.original.clone();
        effective.playback_allowed &= recovery.next.playback_allowed;
        for receiver in &mut effective.receivers {
            receiver.enabled &= recovery
                .next
                .receivers
                .iter()
                .find(|r| r.receiver_id == receiver.receiver_id)
                .is_some_and(|r| r.enabled);
        }
        for source in &mut effective.sources {
            if let Some(candidate) = recovery
                .next
                .sources
                .iter()
                .find(|s| s.source_id == source.source_id)
            {
                source.blocked |= candidate.blocked;
                source.revoked |= candidate.revoked;
            }
        }
        for binding in &mut effective.bindings {
            binding.revoked |= recovery
                .next
                .bindings
                .iter()
                .find(|b| b.receiver_id == binding.receiver_id && b.source_id == binding.source_id)
                .is_none_or(|b| b.revoked);
        }
        *guard = Some(effective);
        e.airplay.configuration_pending = true;
        if !recovery.restrictions_applied {
            e.airplay.advance_event();
            for receiver in &mut e.airplay.receivers {
                receiver.trust_generation += 1;
            }
            recovery.restrictions_applied = true;
        }
        for (index, action) in &recovery.actions {
            if !matches!(
                action,
                AirplayAction::Disable | AirplayAction::Disconnect | AirplayAction::Revoke
            ) {
                continue;
            }
            let expected = recovery
                .targets
                .iter()
                .find(|(i, _)| i == index)
                .and_then(|(_, t)| t.as_ref());
            let current = e
                .resources
                .admissions
                .claims
                .get(&e.airplay.receivers[*index].receiver_id)
                .map(|c| (&c.owner, c.context.session_id));
            if !matches!(action, AirplayAction::Disable)
                && expected.map(|(o, id)| (o, *id)) != current
            {
                continue;
            }
            if let Some(lane) = e.airplay.receivers[*index].lane {
                e.resources.control.revoke_lane(lane);
            }
            if apply_action(e, shared, *index, action.clone()).is_err() {
                e.airplay.receivers[*index].stop();
            }
        }
        // These restrictions are already applied; reconciliation cannot repeat them
        // against a new owner or double-increment its epoch.
        recovery.actions.retain(|(_, a)| {
            !matches!(
                a,
                AirplayAction::Disable | AirplayAction::Disconnect | AirplayAction::Revoke
            )
        });
        e.airplay.recovery = Some(recovery);
        return Err(Error("profile_durability_unconfirmed"));
    }
    e.airplay.configuration_pending = false;
    let Recovery {
        path,
        configure,
        mut actions,
        targets,
        pairing_window,
        running,
        command,
        payload,
        ..
    } = recovery;
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
    let mut effect_error = None;
    if let Some(i) = pairing_window
        && let Err(error) = open_pairing_window(e, i)
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
        if let Err(error) = apply_action(e, shared, i, action) {
            // Durable denial is authoritative even if the worker's queue is full.
            // Stop this endpoint rather than reporting the saved operation as
            // unexecuted or leaving its active gate open.
            e.airplay.receivers[i].stop();
            e.resources.airplay_mix[i] = None;
            effect_error = Some(Error::from(error).0);
        }
    }
    let outcome = if ended {
        "saved_session_ended"
    } else {
        "applied"
    };
    Ok(Json(finish(
        e,
        &next,
        &command.command_id,
        payload,
        outcome,
        effect_error,
    )))
}

fn open_pairing_window(e: &mut Engine, index: usize) -> std::result::Result<(), Error> {
    if e.stopping || e.airplay.stopping {
        return Err(Error("busy"));
    }
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
    e.airplay.advance_event();
    let state = e.authority.snapshot();
    // A full bounded Mixer queue can be transient. Durable commands still have
    // a successful receipt; the worker owner retries the newest desired mix.
    e.airplay.mixer_dirty = e.resources.prepare(&state, None).is_err();
    let mut result = view(e, profile, true);
    result["outcome"] = serde_json::json!(outcome);
    let application = e
        .resources
        .application(e.authority.current().runtime_epoch, Instant::now());
    result["media_pending"] = serde_json::json!(application.pending);
    result["media_application"] = serde_json::json!(application);
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
            let identities = count.saturating_sub(next.receivers.len()) as u64;
            let policy_change = identities > 0
                || next.multi_receiver != multi
                || next
                    .receivers
                    .iter()
                    .enumerate()
                    .any(|(index, r)| r.enabled != (index < count));
            next.config_revision
                .checked_add(identities + u64::from(policy_change))
                .ok_or("configuration_revision_exhausted")?;
            // add_receiver publishes one identity at a time. Keep each staged
            // entry disabled under the OLD capacity contract until all exist.
            while next.receivers.len() < count {
                // Keep the room in the name: "NeonMix — " alone is ten
                // characters, so a ten-character prefix dropped every room.
                let suffix = format!(" · {}", next.receivers.len() + 1);
                let mut name = String::new();
                for c in next.receivers[0].name.chars() {
                    if name.len() + c.len_utf8() + suffix.len() > 50 {
                        break;
                    }
                    name.push(c);
                }
                name.push_str(&suffix);
                add(path, &mut next, &name)?;
            }
            let staged = next.clone();
            next.multi_receiver = multi;
            for (i, receiver) in next.receivers.iter_mut().enumerate() {
                receiver.enabled = i < count;
            }
            staged.prepare_change(&mut next)?;
        }
        original.prepare_change(&mut next)?;
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
    let error = result.unwrap_err();
    if final_save_attempted
        && let Some(publication) = error.downcast_ref::<neonmix_identity::files::PublicationError>()
    {
        if publication.publication != neonmix_identity::files::Publication::NotPublished {
            return Ok(Persisted {
                profile: next,
                complete: true,
                warning: Some("profile_durability_unconfirmed"),
            });
        }
        if configure.is_none() || next.receivers.len() == original.receivers.len() {
            return Err(Error("profile_write_failed"));
        }
    }
    // Reading equal bytes only confirms visibility. Keep the candidate pending;
    // no success receipt or grant is possible without a new successful sync.
    let actual = profile::load(path).map_err(|_| Error("profile_write_failed"))?;
    if final_save_attempted && same_profile(&actual, &next) {
        Ok(Persisted {
            profile: next,
            complete: true,
            warning: Some("profile_durability_unconfirmed"),
        })
    } else if configure.is_some()
        && (!same_profile(&actual, original)
            || profile::operation_pending(path)
                .map_err(|_| Error("configuration_recovery_required"))?)
    {
        Ok(Persisted {
            profile: actual,
            complete: false,
            warning: Some("configuration_recovery_required"),
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
                stopping: false,
                pairings: Default::default(),
                pair_window: Instant::now(),
                pair_attempts: 0,
                room_name: "API test".into(),
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
                        .map(|producer| super::super::super::Lane {
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
            let Json(snapshot) = snapshot_with(&self.shared, &self.headers, || {}).unwrap();
            AirplayCommandV2 {
                command_version: 2,
                expected_config_revision: None,
                expected_event_sequence: None,
                runtime_epoch: None,
                credential_id: None,
                command_id: Uuid::new_v4().to_string(),
                expected_revision: Some(snapshot["revision"].as_u64().unwrap()),
                operation,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn reserve_source(f: &Fixture) -> u64 {
        let owner = Owner {
            receiver: f.profile.receivers[0].receiver_id,
            generation: 1,
            connection: 7,
            request: 9,
            source: f.source.clone(),
        };
        f.shared
            .lock()
            .unwrap()
            .resources
            .admissions
            .reserve(owner, 0, false, Some(0), &[], Instant::now())
            .unwrap()
            .context
            .session_id
    }

    fn v3(f: &Fixture, operation: Op) -> AirplayCommandV2 {
        let Json(snapshot) = snapshot_with(&f.shared, &f.headers, || {}).unwrap();
        let e = f.shared.lock().unwrap();
        let principal = authenticate(&e, &f.headers).map_err(|e| e.0).unwrap();
        AirplayCommandV2::bound(
            Uuid::new_v4().to_string(),
            e.authority.current().runtime_epoch.to_string(),
            principal.device_id().to_string(),
            snapshot["config_revision"].as_u64().unwrap(),
            snapshot["event_sequence"].as_u64().unwrap(),
            operation,
        )
    }
    fn read(f: &Fixture) -> serde_json::Value {
        snapshot_with(&f.shared, &f.headers, || {}).unwrap().0
    }
    #[test]
    fn capacity_counts_configured_airplay_entries_and_claims_only() {
        let f = Fixture::new();
        let _ = execute(
            &f.shared,
            &f.headers,
            f.command(Op::Configure {
                receiver_count: 2,
                multi_receiver: true,
            }),
            persist_change,
        )
        .unwrap();
        f.shared.lock().unwrap().resources.native_reservation =
            Some((Uuid::new_v4(), 8, Uuid::new_v4(), 900));
        assert_eq!(
            read(&f)["capacity"],
            serde_json::json!({"limit":2,"active":0,"reserved":0,"available":2})
        );
        reserve_source(&f);
        assert_eq!(
            read(&f)["capacity"],
            serde_json::json!({"limit":2,"active":0,"reserved":1,"available":1})
        );
        {
            let mut e = f.shared.lock().unwrap();
            let owner = e
                .resources
                .admissions
                .claims
                .values()
                .next()
                .unwrap()
                .owner
                .clone();
            e.resources
                .admissions
                .commit(&owner, Instant::now())
                .unwrap();
        }
        assert_eq!(
            read(&f)["capacity"],
            serde_json::json!({"limit":2,"active":1,"reserved":0,"available":1})
        );
    }
    #[test]
    fn terminal_airplay_actions_cut_actual_timed_fifo_with_full_queue_and_preserve_native_lane() {
        use neonmix_core::{AudioBlock, AudioFormat, Discontinuity, signal::StereoSource};
        for action in 0..3 {
            let mut f = Fixture::new();
            let session = reserve_source(&f);
            let (stream, epoch, origin, native_stream) = {
                let mut e = f.shared.lock().unwrap();
                let principal = authenticate(&e, &f.headers).map_err(|e| e.0).unwrap();
                let command = neonmix_control::Command::bound(
                    e.authority.current(),
                    principal.device_id(),
                    neonmix_control::Operation::Start {
                        offer: neonmix_control::MediaOffer {
                            version: 1,
                            codec: "opus".into(),
                            rate: 48_000,
                            channels: 2,
                            packet_frames: 480,
                            payload_type: 96,
                            ssrc: 19,
                            stream_epoch: 1,
                            udp_port: 34001,
                            certificate_sha256: "a".repeat(64),
                        },
                    },
                );
                let receipt = e.authority.execute(principal, command, |_| Ok(())).unwrap();
                let native = receipt.stream_id.unwrap();
                e.resources.lanes[1].session = receipt.session_id;
                e.resources.lanes[1].binding_generation = e.resources.lanes[1]
                    .producer
                    .as_mut()
                    .unwrap()
                    .bind_next()
                    .unwrap();
                let claim = e
                    .resources
                    .admissions
                    .claims
                    .get_mut(&f.profile.receivers[0].receiver_id)
                    .unwrap();
                claim.active = true;
                let (stream, epoch) = (claim.context.stream_id, claim.context.stream_epoch);
                let generation = e.resources.lanes[0]
                    .producer
                    .as_mut()
                    .unwrap()
                    .bind_next()
                    .unwrap();
                e.airplay.receivers[0].lane = Some(0);
                e.airplay.receivers[0].gate.store(true, Release);
                e.airplay.receivers[0].status.active = true;
                let (tx, _rx) = mpsc::sync_channel(4);
                e.airplay.receivers[0].commands = Some(tx);
                e.resources.airplay_mix[0] = Some((
                    0,
                    LaneMix {
                        stream_id: stream,
                        epoch,
                        binding_generation: generation,
                        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
                        ..Default::default()
                    },
                ));
                let output = neonmix_control::Command::bound(
                    e.authority.current(),
                    principal.device_id(),
                    neonmix_control::Operation::OutputMix {
                        gain_db: Some(0.),
                        muted: None,
                    },
                );
                e.authority.execute(principal, output, |_| Ok(())).unwrap();
                let state = e.authority.snapshot();
                e.resources.prepare(&state, None).unwrap();
                for index in 0..8 {
                    for (lane, id, pts, value) in [
                        (0, stream, Some(1_000_000_000 + index * 10_000_000), 0.6),
                        (1, native, None, 0.05),
                    ] {
                        let mut block = AudioBlock::empty(id, AudioFormat::INTERNAL);
                        block.header.stream_epoch = epoch;
                        block.header.source_sample_position = index * 480;
                        block.header.presentation_time_ns = pts;
                        block.header.arrival_ns = u64::MAX;
                        block.header.frame_count = 480;
                        block.header.discontinuity_flags = Discontinuity::NONE;
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
                (stream, epoch, e.resources.origin, native)
            };
            f._mixer
                .set_presentation_time(origin + Duration::from_secs(1));
            let mut warm = [[0.; 2]; 1440];
            f._mixer.render_block(&mut warm);
            assert!(warm[1200][0] > 0.6);
            {
                let mut e = f.shared.lock().unwrap();
                let state = e.authority.snapshot();
                while e.resources.control.has_capacity() {
                    e.resources.prepare(&state, None).unwrap();
                }
            }
            let command = v3(
                &f,
                match action {
                    0 => Op::RevokeSource {
                        source_id: f.source.clone(),
                        session_id: Some(session),
                    },
                    1 => Op::DisconnectSource {
                        source_id: f.source.clone(),
                        session_id: session,
                    },
                    _ => Op::DisableReceiver {
                        receiver_id: f.profile.receivers[0].receiver_id.to_string(),
                    },
                },
            );
            let Json(saved) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
            assert!(saved["media_pending"].as_bool().unwrap());
            assert!(
                saved["media_application"]["desired_config_sequence"]
                    .as_u64()
                    .unwrap()
                    > saved["media_application"]["applied_config_sequence"]
                        .as_u64()
                        .unwrap()
            );
            assert!(
                !f.shared.lock().unwrap().airplay.receivers[0]
                    .gate
                    .load(Acquire)
            );
            let mut closed = [[0.; 2]; 480];
            f._mixer.render_block(&mut closed);
            assert!(
                closed.iter().all(|frame| frame[0] < 0.055),
                "AirPlay FIFO leaked after limiting command"
            );
            assert!(
                closed.iter().any(|frame| frame[0] > 0.045),
                "independent native lane was silenced"
            );
            {
                let mut e = f.shared.lock().unwrap();
                let state = e.authority.snapshot();
                e.resources.prepare(&state, None).unwrap();
                let generation = e.resources.lanes[0]
                    .producer
                    .as_mut()
                    .unwrap()
                    .bind_next()
                    .unwrap();
                e.resources.airplay_mix[0] = Some((
                    0,
                    LaneMix {
                        stream_id: stream,
                        epoch,
                        binding_generation: generation,
                        playback_kind: neonmix_core::mixer::PlaybackKind::Timed,
                        ..Default::default()
                    },
                ));
                let state = e.authority.snapshot();
                e.resources.prepare(&state, None).unwrap();
                let mut block = AudioBlock::empty(stream, AudioFormat::INTERNAL);
                block.header.stream_epoch = epoch;
                block.header.source_sample_position = 0;
                block.header.presentation_time_ns = Some(2_000_000_000);
                block.header.arrival_ns = u64::MAX;
                block.header.frame_count = 480;
                block.pcm.fill([0.2; 2]);
                assert!(e.resources.lanes[0].producer.as_mut().unwrap().push(block));
                assert!(e.authority.current().streams.contains_key(&native_stream));
            }
            f._mixer
                .set_presentation_time(origin + Duration::from_secs(2));
            f._mixer.render_block(&mut closed);
            assert!(
                closed[479][0] > 0.19 && closed[479][0] < 0.26,
                "new lease replayed the old amplitude"
            );
        }
    }

    #[test]
    fn v3_health_and_pcm_observations_do_not_advance_config_but_business_cursor_changes() {
        let f = Fixture::new();
        let initial = read(&f);
        let command = v3(
            &f,
            Op::AliasSource {
                source_id: f.source.clone(),
                alias: Some("new alias".into()),
            },
        );
        assert!(command.expected_event_sequence.is_none());
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].status.received_blocks += 17;
        }
        let samples = read(&f);
        assert_eq!(samples["event_sequence"], initial["event_sequence"]);
        assert_eq!(samples["state"], initial["state"]);
        assert_ne!(samples["receivers"], initial["receivers"]);
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].status.discovery_state = "publishing".into();
        }
        let health = read(&f);
        assert_eq!(health["config_revision"], initial["config_revision"]);
        assert!(
            health["event_sequence"].as_u64().unwrap()
                > initial["event_sequence"].as_u64().unwrap()
        );
        let Json(saved) = execute(&f.shared, &f.headers, command.clone(), persist_change).unwrap();
        assert_eq!(
            saved["config_revision"].as_u64().unwrap(),
            initial["config_revision"].as_u64().unwrap() + 1
        );
        let Json(replay) = execute(&f.shared, &f.headers, command, |_, _, _, _| {
            panic!("completed v3 replay must not persist")
        })
        .unwrap();
        assert_eq!(replay, saved);
    }
    #[test]
    fn v3_runtime_cas_is_not_replaced_by_configuration_revision() {
        let f = Fixture::new();
        let session = reserve_source(&f);
        let command = v3(
            &f,
            Op::PatchMixSource {
                source_id: f.source.clone(),
                session_id: session,
                gain_db: Some(-9.),
                muted: None,
                solo: None,
            },
        );
        assert!(
            command.expected_config_revision.is_some() && command.expected_event_sequence.is_some()
        );
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].status.ready = true;
        }
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "stale runtime cannot write"
            ))
            .unwrap_err()
            .0,
            "stale_revision"
        );
        let solo = v3(
            &f,
            Op::PatchMixSource {
                source_id: f.source.clone(),
                session_id: session,
                gain_db: None,
                muted: None,
                solo: Some(true),
            },
        );
        assert!(solo.expected_config_revision.is_none() && solo.expected_event_sequence.is_some());
        let base = read(&f);
        let Json(saved) = execute(&f.shared, &f.headers, solo, |_, _, _, _| {
            panic!("solo is volatile")
        })
        .unwrap();
        assert_eq!(saved["config_revision"], base["config_revision"]);
    }
    #[test]
    fn v3_unknown_keeps_confirmed_configuration_and_one_frozen_candidate_version() {
        let f = Fixture::new();
        let initial = read(&f);
        let command = v3(
            &f,
            Op::AliasSource {
                source_id: f.source.clone(),
                alias: Some("pending alias".into()),
            },
        );
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        let pending = read(&f);
        let visible = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(pending["config_revision"], initial["config_revision"]);
        assert_eq!(
            pending["state"]["configuration"],
            initial["state"]["configuration"]
        );
        assert_eq!(
            visible.config_revision,
            initial["config_revision"].as_u64().unwrap() + 1
        );
        assert!(
            pending["event_sequence"].as_u64().unwrap()
                > initial["event_sequence"].as_u64().unwrap()
        );
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        assert_eq!(
            profile::load(&f.directory.join("receiver.json"))
                .unwrap()
                .config_revision,
            visible.config_revision
        );
        let Json(saved) = execute(&f.shared, &f.headers, command.clone(), persist_change).unwrap();
        assert_eq!(saved["config_revision"], visible.config_revision);
        assert_eq!(
            saved["state"]["configuration"]["sources"][0]["alias"],
            "pending alias"
        );
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "no second transaction"
            ))
            .unwrap()
            .0,
            saved
        );
    }
    #[test]
    fn v3_old_runtime_and_mixed_or_missing_conditions_are_rejected_before_disk_work() {
        let f = Fixture::new();
        let original = v3(
            &f,
            Op::AliasSource {
                source_id: f.source.clone(),
                alias: Some("new".into()),
            },
        );
        for index in 0..6 {
            let mut invalid = original.clone();
            let expected = match index {
                0 => {
                    invalid.expected_revision = Some(1);
                    "upgrade_required"
                }
                1 => {
                    invalid.expected_config_revision = None;
                    "upgrade_required"
                }
                2 => {
                    invalid.runtime_epoch = None;
                    "upgrade_required"
                }
                3 => {
                    invalid.command_version = 2;
                    invalid.expected_revision = Some(1);
                    "upgrade_required"
                }
                4 => {
                    invalid.runtime_epoch = Some(Uuid::new_v4().to_string());
                    "snapshot_required"
                }
                _ => {
                    invalid.command_version = 99;
                    "incompatible_version"
                }
            };
            assert_eq!(
                execute(&f.shared, &f.headers, invalid, |_, _, _, _| panic!(
                    "invalid context saved"
                ))
                .unwrap_err()
                .0,
                expected
            );
        }
        assert!(f.shared.lock().unwrap().airplay.receipts.is_empty());
    }
    #[test]
    fn exhausted_business_sequence_never_wraps_or_advertises_an_unchanged_valid_cursor() {
        let f = Fixture::new();
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.revision = u64::MAX;
            e.airplay.published_business = None;
        }
        let before = read(&f);
        assert_eq!(before["event_sequence"], u64::MAX);
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].status.discovery_state = "changed".into();
        }
        let after = read(&f);
        assert!(after["event_sequence"].is_null());
        assert_eq!(after["snapshot_required"], true);
        let command = f.command(Op::AliasSource {
            source_id: f.source.clone(),
            alias: Some("no write".into()),
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "exhausted sequence cannot write"
            ))
            .unwrap_err()
            .0,
            "snapshot_required"
        );
    }

    #[test]
    fn v3_config_versions_survive_profile_reload_without_changing_identity() {
        let f = Fixture::new();
        let id = f.profile.receivers[0].receiver_uuid;
        let reference = f.profile.receivers[0].key_reference.clone();
        let command = v3(
            &f,
            Op::AliasSource {
                source_id: f.source.clone(),
                alias: Some("durable".into()),
            },
        );
        let Json(saved) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(
            actual.config_revision,
            saved["config_revision"].as_u64().unwrap()
        );
        assert_eq!(actual.receivers[0].receiver_uuid, id);
        assert_eq!(actual.receivers[0].key_reference, reference);
    }

    #[test]
    fn source_field_patch_preserves_other_client_mute_and_session_solo() {
        let f = Fixture::new();
        let session = reserve_source(&f);
        for (gain_db, muted, solo) in [
            (None, Some(true), None),
            (None, None, Some(true)),
            (Some(-9.), None, None),
        ] {
            let command = f.command(Op::PatchMixSource {
                source_id: f.source.clone(),
                session_id: session,
                gain_db,
                muted,
                solo,
            });
            let Json(result) =
                execute(&f.shared, &f.headers, command.clone(), persist_change).unwrap();
            assert_eq!(result["capabilities"][0], "patch_mix_source");
            let Json(replay) = execute(&f.shared, &f.headers, command, |_, _, _, _| {
                panic!("no replay save")
            })
            .unwrap();
            assert_eq!(result, replay);
        }
        let e = f.shared.lock().unwrap();
        let mix = e.airplay.receivers[0].status.mix;
        assert_eq!(mix.gain_db, -9.);
        assert!(mix.muted && mix.solo);
        drop(e);
        let saved = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(saved.sources[0].gain_db, -9.);
        assert!(saved.sources[0].muted);
    }

    #[test]
    fn source_patch_rejects_empty_invalid_and_replaced_session_before_save() {
        let f = Fixture::new();
        let session = reserve_source(&f);
        for (gain_db, session_id, expected) in [
            (None, session, "empty_patch"),
            (Some(f32::NAN), session, "invalid_gain"),
            (Some(13.), session, "invalid_gain"),
            (Some(-97.), session, "invalid_gain"),
            (Some(-9.), session + 1, "session_changed"),
        ] {
            let command = f.command(Op::PatchMixSource {
                source_id: f.source.clone(),
                session_id,
                gain_db,
                muted: None,
                solo: None,
            });
            assert_eq!(
                execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                    "invalid patch must not save"
                ))
                .unwrap_err()
                .0,
                expected
            );
        }
        assert!(f.shared.lock().unwrap().airplay.receipts.is_empty());
    }

    #[test]
    fn solo_only_source_patch_has_no_persistent_side_effect() {
        let f = Fixture::new();
        let session_id = reserve_source(&f);
        let before = std::fs::read(f.directory.join("receiver.json")).unwrap();
        let _ = execute(
            &f.shared,
            &f.headers,
            f.command(Op::PatchMixSource {
                source_id: f.source.clone(),
                session_id,
                gain_db: None,
                muted: None,
                solo: Some(true),
            }),
            |_, _, _, _| panic!("session solo must not persist"),
        )
        .unwrap();
        assert!(
            f.shared.lock().unwrap().airplay.receivers[0]
                .status
                .mix
                .solo
        );
        assert_eq!(
            std::fs::read(f.directory.join("receiver.json")).unwrap(),
            before
        );
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
    fn configuration_does_not_reduce_native_capacity_during_disk_write() {
        let f = Fixture::new();
        let command = f.command(Op::Configure {
            receiver_count: 2,
            multi_receiver: true,
        });
        let error = execute(&f.shared, &f.headers, command, |_, _, _, _| {
            let e = f.shared.try_lock().unwrap();
            assert!(e.airplay.configuration_pending);
            assert!(e.resources.admissions.native_capacity(5));
            Err(Error("profile_write_failed"))
        })
        .unwrap_err();
        assert_eq!(error.0, "profile_write_failed");
        let e = f.shared.lock().unwrap();
        assert!(!e.airplay.configuration_pending);
        assert!(!e.resources.multi_receiver);
        assert!(e.airplay.receipts.is_empty());
    }

    #[test]
    fn partial_identity_preparation_reconciles_original_request_with_same_identities() {
        let f = Fixture::new();
        let mut attempts = 0;
        let command = f.command(Op::Configure {
            receiver_count: 3,
            multi_receiver: true,
        });
        let error = execute(
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
        .unwrap_err();
        assert_eq!(error.0, "profile_durability_unconfirmed");
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(actual.receivers.len(), 2);
        assert!(!actual.receivers[1].enabled && !actual.multi_receiver);
        let e = f.shared.lock().unwrap();
        assert_eq!(e.airplay.receivers.len(), 1);
        assert!(e.airplay.receipts.is_empty() && e.airplay.configuration_pending);
        drop(e);
        let original_id = actual.receivers[1].receiver_id;
        let Json(result) = execute(&f.shared, &f.headers, command.clone(), persist_change).unwrap();
        assert_eq!(result["outcome"], "applied");
        let actual = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(actual.receivers[1].receiver_id, original_id);
        assert_eq!(actual.receivers.len(), 3);
        assert_eq!(
            actual.receivers[2].name,
            format!("{} · 3", actual.receivers[0].name)
        );
        assert!(actual.multi_receiver && actual.receivers.iter().all(|r| r.enabled));
        let e = f.shared.lock().unwrap();
        assert_eq!(e.airplay.receivers.len(), 3);
        assert!(!e.airplay.configuration_pending);
        assert_eq!(e.airplay.receipts.len(), 1);
        drop(e);
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "no save on replay"
            ))
            .unwrap()
            .0,
            result
        );
    }

    fn fail_published_sync(
        path: &std::path::Path,
        next: &ReceiverProfile,
    ) -> neonmix_identity::Result<()> {
        neonmix_identity::files::replace_with(
            path,
            &serde_json::to_vec_pretty(next).unwrap(),
            &mut |stage| {
                if stage == neonmix_identity::files::ReplaceStage::SyncPublishedDirectory {
                    Err(std::io::Error::other("injected directory sync failure"))
                } else {
                    Ok(())
                }
            },
        )?;
        Ok(())
    }
    fn unknown_save(
        path: &std::path::Path,
        original: &ReceiverProfile,
        next: ReceiverProfile,
        config: Option<(usize, bool)>,
    ) -> std::result::Result<Persisted, Error> {
        persist_with(
            path,
            original,
            next,
            config,
            |path, next, name| profile::add_receiver(path, next, name, false).map(|_| ()),
            fail_published_sync,
        )
    }

    #[test]
    fn published_revoke_retains_denial_with_full_mixer_queue_and_one_business_commit() {
        let f = Fixture::new();
        let session = reserve_source(&f);
        let command = f.command(Op::RevokeSource {
            source_id: f.source.clone(),
            session_id: Some(session),
        });
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut e = f.shared.lock().unwrap();
            e.airplay.receivers[0].commands = Some(tx);
            e.airplay.receivers[0].lane = Some(0);
            e.airplay.receivers[0].gate.store(true, Release);
            while e.resources.control.has_capacity() {
                e.resources.control.apply(MixerConfig::default()).unwrap();
            }
        }
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        let epoch = {
            let e = f.shared.lock().unwrap();
            assert!(!e.airplay.receivers[0].gate.load(Acquire));
            assert!(e.airplay.profile.lock().unwrap().as_ref().unwrap().sources[0].revoked);
            assert!(e.airplay.receipts.is_empty());
            e.airplay.receivers[0].epoch.load(Relaxed)
        };
        assert!(matches!(rx.try_recv(), Ok(Action::Revoke(_))));
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), |_, _, _, _| Err(
                Error("profile_write_failed")
            ))
            .unwrap_err()
            .0,
            "profile_durability_unconfirmed"
        );
        assert_eq!(
            f.shared.lock().unwrap().airplay.receivers[0]
                .epoch
                .load(Relaxed),
            epoch
        );
        assert!(rx.try_recv().is_err());
        let Json(result) = execute(&f.shared, &f.headers, command.clone(), persist_change).unwrap();
        assert_eq!(result["outcome"], "applied");
        assert!(rx.try_recv().is_err());
        assert_eq!(f.shared.lock().unwrap().airplay.receipts.len(), 1);
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "no second commit"
            ))
            .unwrap()
            .0,
            result
        );
    }

    #[test]
    fn unknown_allow_or_repair_never_grants_or_opens_pairing_and_freezes_payload() {
        for repair in [false, true] {
            let f = Fixture::new();
            let (tx, _rx) = mpsc::sync_channel(4);
            if repair {
                f.shared.lock().unwrap().airplay.receivers[0].commands = Some(tx);
            }
            {
                let profiles = f.shared.lock().unwrap().airplay.profile.clone();
                let mut guard = profiles.lock().unwrap();
                let p = guard.as_mut().unwrap();
                p.sources[0].blocked = true;
                p.sources[0].revoked = repair;
                profile::save(&f.directory.join("receiver.json"), p).unwrap();
            }
            let op = if repair {
                Op::RepairSource {
                    source_id: f.source.clone(),
                    receiver_id: f.profile.receivers[0].receiver_id.to_string(),
                }
            } else {
                Op::AllowSource {
                    source_id: f.source.clone(),
                }
            };
            let command = f.command(op);
            assert_eq!(
                execute(&f.shared, &f.headers, command.clone(), unknown_save)
                    .unwrap_err()
                    .0,
                "profile_durability_unconfirmed"
            );
            {
                let e = f.shared.lock().unwrap();
                assert!(!trusted(
                    e.airplay.profile.lock().unwrap().as_ref().unwrap(),
                    f.profile.receivers[0].receiver_id,
                    &f.source
                ));
                assert!(e.airplay.receivers[0].pairing_deadline.is_none());
                assert_eq!(e.airplay.receivers[0].commands.is_some(), repair);
            }
            let new = f.command(Op::AliasSource {
                source_id: f.source.clone(),
                alias: Some("another".into()),
            });
            assert_eq!(
                execute(&f.shared, &f.headers, new, |_, _, _, _| panic!(
                    "conflicting save"
                ))
                .unwrap_err()
                .0,
                "profile_durability_unconfirmed"
            );
            let mut conflict = command.clone();
            conflict.operation = Op::AllowSource {
                source_id: "different".into(),
            };
            assert_eq!(
                execute(&f.shared, &f.headers, conflict, |_, _, _, _| panic!(
                    "changed ID"
                ))
                .unwrap_err()
                .0,
                "command_id_reused"
            );
            // Persisted candidate is the original complete request, without repeating preparation.
            let Json(result) = execute(
                &f.shared,
                &f.headers,
                command.clone(),
                |path, original, next, config| {
                    assert!(config.is_none());
                    assert!(!next.sources[0].blocked && !next.sources[0].revoked);
                    assert_eq!(next.bindings.is_empty(), repair);
                    persist_change(path, original, next, config)
                },
            )
            .unwrap();
            assert_eq!(result["outcome"], "applied");
            let mut e = f.shared.lock().unwrap();
            if repair {
                assert!(e.airplay.receivers[0].pairing_deadline.is_some());
                e.airplay.receivers[0].stop();
            }
        }
    }

    #[test]
    fn unknown_configuration_retry_never_allocates_second_receiver_identity() {
        let f = Fixture::new();
        let command = f.command(Op::Configure {
            receiver_count: 3,
            multi_receiver: true,
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        let visible = profile::load(&f.directory.join("receiver.json")).unwrap();
        assert_eq!(visible.receivers.len(), 3);
        assert_eq!(f.shared.lock().unwrap().airplay.receivers.len(), 1);
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        assert!(same_profile(
            &visible,
            &profile::load(&f.directory.join("receiver.json")).unwrap()
        ));
        let Json(result) = execute(&f.shared, &f.headers, command, persist_change).unwrap();
        assert_eq!(result["outcome"], "applied");
        assert!(same_profile(
            &visible,
            &profile::load(&f.directory.join("receiver.json")).unwrap()
        ));
        assert_eq!(f.shared.lock().unwrap().airplay.receivers.len(), 3);
    }

    #[test]
    fn prepublication_error_is_definite_even_when_candidate_equals_visible_bytes() {
        let f = Fixture::new();
        let command = f.command(Op::AliasSource {
            source_id: f.source.clone(),
            alias: None,
        });
        let error = execute(
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
                    |path, next| {
                        neonmix_identity::files::replace_with(
                            path,
                            &serde_json::to_vec_pretty(next).unwrap(),
                            &mut |stage| {
                                if stage == neonmix_identity::files::ReplaceStage::Publish {
                                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                                } else {
                                    Ok(())
                                }
                            },
                        )?;
                        Ok(())
                    },
                )
            },
        )
        .unwrap_err();
        assert_eq!(error.0, "profile_write_failed");
        let e = f.shared.lock().unwrap();
        assert!(!e.airplay.configuration_pending && e.airplay.recovery.is_none());
        assert!(e.airplay.receipts.is_empty());
        assert!(same_profile(
            &f.profile,
            &profile::load(&f.directory.join("receiver.json")).unwrap()
        ));
    }

    #[tokio::test]
    async fn background_registration_unknown_retains_candidate_and_admin_epoch_recovery() {
        let f = Fixture::new();
        let profiles = f.shared.lock().unwrap().airplay.profile.clone();
        let (next, new_source, path, request, epoch) = {
            let guard = profiles.lock().unwrap();
            let mut next = guard.as_ref().unwrap().clone();
            let new_source = next
                .register_source(
                    next.receivers[0].receiver_id,
                    "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
                    Some("new device".into()),
                )
                .unwrap();
            let path = f.directory.join("receiver.json");
            let error = persist_background(
                &f.shared,
                &path,
                guard.as_ref().unwrap(),
                &mut next,
                fail_published_sync,
            )
            .unwrap_err();
            assert!(
                error
                    .downcast_ref::<neonmix_identity::files::PublicationError>()
                    .is_some()
            );
            assert!(!trusted(
                guard.as_ref().unwrap(),
                next.receivers[0].receiver_id,
                &new_source
            ));
            let (request, epoch) = {
                let e = f.shared.lock().unwrap();
                assert!(pending(&e));
                (
                    pending_request(&e).unwrap(),
                    e.authority.current().runtime_epoch,
                )
            };
            (next, new_source, path, request, epoch)
        };
        assert_eq!(
            recover(&f.shared, &f.headers, request, Uuid::new_v4())
                .unwrap_err()
                .0,
            ControlError::SnapshotRequired
        );
        assert_eq!(
            recover(&f.shared, &HeaderMap::new(), request, epoch)
                .unwrap_err()
                .0,
            ControlError::Unauthenticated
        );
        let command = f.command(Op::AllowSource {
            source_id: f.source.clone(),
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command, |_, _, _, _| panic!(
                "no conflicting writer"
            ))
            .unwrap_err()
            .0,
            "profile_durability_unconfirmed"
        );
        let Json(result) = super::super::super::recover_persistence(
            axum::extract::State(f.shared.clone()),
            f.headers.clone(),
            Json(super::super::super::RecoveryRequest {
                request_id: request,
                runtime_epoch: epoch,
            }),
        )
        .await
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(result["outcome"], "saved_worker_stopped");
        assert!(trusted(
            profiles.lock().unwrap().as_ref().unwrap(),
            next.receivers[0].receiver_id,
            &new_source
        ));
        assert!(same_profile(&next, &profile::load(&path).unwrap()));
        assert!(!pending(&f.shared.lock().unwrap()));
        let Json(replay) = super::super::super::recover_persistence(
            axum::extract::State(f.shared.clone()),
            f.headers.clone(),
            Json(super::super::super::RecoveryRequest {
                request_id: request,
                runtime_epoch: epoch,
            }),
        )
        .await
        .map_err(|error| error.0)
        .unwrap();
        assert_eq!(replay, result);
    }

    #[test]
    fn operator_reconciliation_does_not_restart_stopping_airplay_owner() {
        let f = Fixture::new();
        let profiles = f.shared.lock().unwrap().airplay.profile.clone();
        {
            let mut guard = profiles.lock().unwrap();
            guard.as_mut().unwrap().receivers[0].enabled = false;
            profile::save(&f.directory.join("receiver.json"), guard.as_ref().unwrap()).unwrap();
        }
        let command = f.command(Op::EnableReceiver {
            receiver_id: f.profile.receivers[0].receiver_id.to_string(),
        });
        assert_eq!(
            execute(&f.shared, &f.headers, command.clone(), unknown_save)
                .unwrap_err()
                .0,
            "profile_durability_unconfirmed"
        );
        let epoch = {
            let mut e = f.shared.lock().unwrap();
            e.airplay.stopping = true;
            e.authority.current().runtime_epoch
        };
        let result = recover(
            &f.shared,
            &f.headers,
            Uuid::parse_str(&command.command_id).unwrap(),
            epoch,
        )
        .map_err(|error| error.0)
        .unwrap()
        .unwrap();
        assert_eq!(result["warning"], "busy");
        let e = f.shared.lock().unwrap();
        assert!(!e.airplay.receivers[0].status.enabled);
        assert!(e.airplay.receivers[0].commands.is_none() && e.airplay.receivers[0].join.is_none());
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
