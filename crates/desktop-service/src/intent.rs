//! Field patches for a peer that supports runtime identity and full MixSource,
//! but not PatchMixSource. This resolver is read-only; the resolved revision and
//! full payload are frozen before the mutation is sent or reconciled.
use crate::Result;
use neonmix_airplay_adapter::control::{AirplayActionV2 as Action, AirplayCommandV2};
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) fn resolve_airplay_patch(
    view: &Value,
    runtime_epoch: Uuid,
    stream_epoch: u64,
    credential_id: Uuid,
    mut command: AirplayCommandV2,
    guard: Option<&Action>,
) -> Result<Value> {
    if runtime_epoch.is_nil()
        || view["runtime_epoch"].as_str() != Some(runtime_epoch.to_string().as_str())
        || !view["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == "runtime_epoch"))
    {
        return Err("upgrade_required".into());
    }
    let Action::PatchMixSource {
        source_id,
        session_id,
        gain_db,
        muted,
        solo,
    } = &command.operation
    else {
        return Err("invalid_command".into());
    };
    if gain_db.is_none() && muted.is_none() && solo.is_none() {
        return Err("invalid_command".into());
    }
    if gain_db.is_some_and(|gain| !gain.is_finite() || !(-96.0..=12.0).contains(&gain)) {
        return Err("invalid_gain".into());
    }
    let session = view["sessions"]
        .as_array()
        .and_then(|sessions| {
            sessions.iter().find(|s| {
                s["source_id"].as_str() == Some(source_id)
                    && s["session_id"].as_u64() == Some(*session_id)
                    && s["stream_epoch"].as_u64() == Some(stream_epoch)
            })
        })
        .ok_or("session_changed")?;
    let before = session.get("mix").ok_or("invalid_backend_response")?;
    let old_gain = before["gain_db"]
        .as_f64()
        .ok_or("invalid_backend_response")? as f32;
    if !old_gain.is_finite() || !(-96.0..=12.0).contains(&old_gain) {
        return Err("invalid_backend_response".into());
    }
    let old_mute = before["muted"]
        .as_bool()
        .ok_or("invalid_backend_response")?;
    let old_solo = before["solo"].as_bool().ok_or("invalid_backend_response")?;
    if let Some(Action::PatchMixSource {
        source_id: id,
        session_id: sid,
        gain_db: gain,
        muted: mute,
        solo,
        ..
    }) = guard
        && (id != source_id
            || sid != session_id
            || gain.is_some_and(|g| g != old_gain)
            || mute.is_some_and(|m| m != old_mute)
            || solo.is_some_and(|s| s != old_solo))
    {
        return Err("stale_revision".into());
    }
    if command.command_version != 2 {
        return Err("upgrade_required".into());
    }
    command.expected_revision = Some(
        view["revision"]
            .as_u64()
            .ok_or("invalid_backend_response")?,
    );
    command.runtime_epoch = Some(runtime_epoch.to_string());
    command.credential_id = Some(credential_id.to_string());
    command.operation = Action::MixSource {
        source_id: source_id.clone(),
        session_id: *session_id,
        gain_db: gain_db.unwrap_or(old_gain),
        muted: muted.unwrap_or(old_mute),
        solo: solo.unwrap_or(old_solo),
    };
    Ok(json!({"command":command,"before":before}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Uuid, Uuid, AirplayCommandV2) {
        let runtime = Uuid::new_v4();
        let identity = Uuid::new_v4();
        let view = json!({"runtime_epoch":runtime,"capabilities":["runtime_epoch"],"revision":19,
            "sessions":[{"source_id":"source","session_id":7,"stream_epoch":4,
                "mix":{"gain_db":-3.,"muted":true,"solo":true}}]});
        let command = AirplayCommandV2 {
            command_version: 2,
            expected_config_revision: None,
            expected_event_sequence: None,
            command_id: Uuid::new_v4().to_string(),
            runtime_epoch: Some(runtime.to_string()),
            credential_id: Some(identity.to_string()),
            expected_revision: Some(2),
            operation: Action::PatchMixSource {
                source_id: "source".into(),
                session_id: 7,
                gain_db: Some(-9.),
                muted: None,
                solo: None,
            },
        };
        (view, runtime, identity, command)
    }
    #[test]
    fn legacy_patch_uses_one_fresh_revision_and_preserves_unmodified_fields() {
        let (view, runtime, identity, command) = fixture();
        let id = command.command_id.clone();
        let result = resolve_airplay_patch(&view, runtime, 4, identity, command, None).unwrap();
        assert_eq!(result["command"]["command_id"], id);
        assert_eq!(result["command"]["expected_revision"], 19);
        assert_eq!(result["command"]["operation"]["gain_db"], -9.);
        assert_eq!(result["command"]["operation"]["muted"], true);
        assert_eq!(result["command"]["operation"]["solo"], true);
        assert_eq!(result["before"]["gain_db"], -3.);
    }
    #[test]
    fn legacy_patch_requires_runtime_session_and_unchanged_undo_field() {
        let (view, runtime, identity, command) = fixture();
        assert_eq!(
            resolve_airplay_patch(&view, Uuid::new_v4(), 4, identity, command.clone(), None)
                .unwrap_err(),
            "upgrade_required"
        );
        assert_eq!(
            resolve_airplay_patch(&view, runtime, 5, identity, command.clone(), None).unwrap_err(),
            "session_changed"
        );
        let guard = Action::PatchMixSource {
            source_id: "source".into(),
            session_id: 7,
            gain_db: Some(-6.),
            muted: None,
            solo: None,
        };
        assert_eq!(
            resolve_airplay_patch(&view, runtime, 4, identity, command.clone(), Some(&guard))
                .unwrap_err(),
            "stale_revision"
        );
        let mut missing = view;
        missing["capabilities"] = json!([]);
        assert_eq!(
            resolve_airplay_patch(&missing, runtime, 4, identity, command, None).unwrap_err(),
            "upgrade_required"
        );
    }
}
