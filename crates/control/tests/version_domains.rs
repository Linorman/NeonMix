use neonmix_control::*;
use uuid::Uuid;
const TOKEN: &str = "version-domain-admin-credential-32-bytes-minimum";
fn model() -> (Authority, Principal) {
    let a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    (a, principal)
}
fn offer() -> MediaOffer {
    MediaOffer {
        version: 1,
        codec: "opus".into(),
        rate: 48_000,
        channels: 2,
        packet_frames: 480,
        payload_type: 96,
        ssrc: 1,
        stream_epoch: 1,
        udp_port: 5000,
        certificate_sha256: "a".repeat(64),
    }
}
fn output() -> Operation {
    Operation::OutputMix {
        gain_db: Some(-3.),
        muted: None,
    }
}
fn prepared(a: &mut Authority, p: Principal, c: Command) -> PreparedCommand {
    match a.prepare_transaction(p, c).unwrap() {
        Preparation::Prepared(candidate) => candidate,
        Preparation::Replay(_) => panic!("new command required"),
    }
}
#[test]
fn health_and_runtime_do_not_advance_config_and_config_cas_ignores_unrelated_health() {
    let (mut a, p) = model();
    let base = a.snapshot();
    let config = Command::bound(&base, p.device_id(), output());
    assert_eq!(config.control_version, CONTROL_VERSION);
    assert_eq!(config.expected_revision, None);
    assert_eq!(config.expected_config_revision, Some(base.config_revision));
    assert_eq!(config.expected_event_sequence, None);
    let start = Command::bound(&base, p.device_id(), Operation::Start { offer: offer() });
    let receipt = a.execute(p, start, |_| Ok(())).unwrap();
    assert_eq!(a.current().config_revision, base.config_revision);
    for available in [false, true] {
        a.set_output_available(available).unwrap();
    }
    a.set_session_status(receipt.session_id.unwrap(), SessionStatus::Playing)
        .unwrap();
    assert_eq!(a.current().config_revision, base.config_revision);
    assert_eq!(a.current().event_sequence, base.event_sequence + 4);
    let receipt = a.execute(p, config.clone(), |_| Ok(())).unwrap();
    assert_eq!(receipt.config_revision, base.config_revision + 1);
    assert_eq!(receipt.event_sequence, base.event_sequence + 5);
    assert_eq!(a.persistent().config_revision, receipt.config_revision);
    let replay = a
        .execute(p, config, |_| panic!("completed config replay cannot save"))
        .unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
}
#[test]
fn runtime_cas_remains_required_even_when_config_has_not_changed() {
    let (mut a, p) = model();
    let start = Command::bound(
        a.current(),
        p.device_id(),
        Operation::Start { offer: offer() },
    );
    let receipt = a.execute(p, start, |_| Ok(())).unwrap();
    let stop = Command::bound(
        a.current(),
        p.device_id(),
        Operation::Stop {
            session_id: receipt.session_id.unwrap(),
        },
    );
    let config = a.current().config_revision;
    a.set_session_status(receipt.session_id.unwrap(), SessionStatus::Playing)
        .unwrap();
    assert_eq!(a.current().config_revision, config);
    assert!(matches!(
        a.prepare_transaction(p, stop),
        Err(ControlError::RevisionConflict)
    ));
    assert_eq!(
        a.current().sessions[&receipt.session_id.unwrap()].status,
        SessionStatus::Playing
    );
}
#[test]
fn stream_preferences_use_both_conditions_and_solo_uses_only_runtime() {
    let (mut a, p) = model();
    let start = Command::bound(
        a.current(),
        p.device_id(),
        Operation::Start { offer: offer() },
    );
    let stream = a.execute(p, start, |_| Ok(())).unwrap().stream_id.unwrap();
    let config = a.current().config_revision;
    let mix = Command::bound(
        a.current(),
        p.device_id(),
        Operation::StreamMix {
            stream_id: stream,
            gain_db: Some(-9.),
            muted: None,
            solo: Some(true),
        },
    );
    assert!(mix.expected_config_revision.is_some() && mix.expected_event_sequence.is_some());
    a.execute(p, mix, |_| Ok(())).unwrap();
    assert_eq!(a.current().config_revision, config + 1);
    let solo = Command::bound(
        a.current(),
        p.device_id(),
        Operation::StreamMix {
            stream_id: stream,
            gain_db: None,
            muted: None,
            solo: Some(false),
        },
    );
    assert!(solo.expected_config_revision.is_none() && solo.expected_event_sequence.is_some());
    a.execute(p, solo, |_| Ok(())).unwrap();
    assert_eq!(a.current().config_revision, config + 1);
}
#[test]
fn protocols_cannot_mix_or_omit_cas_context() {
    let (mut a, p) = model();
    let valid = Command::bound(a.current(), p.device_id(), output());
    for change in 0..6 {
        let mut command = valid.clone();
        match change {
            0 => command.expected_revision = Some(a.current().revision),
            1 => command.runtime_epoch = None,
            2 => command.credential_id = None,
            3 => command.expected_config_revision = None,
            4 => command.control_version = 1,
            _ => command.control_version = 99,
        }
        let expected = if change == 5 {
            ControlError::IncompatibleVersion
        } else {
            ControlError::UpgradeRequired
        };
        assert!(matches!(a.prepare_transaction(p,command),Err(error) if error == expected));
    }
    let encoded = serde_json::to_value(&valid).unwrap();
    assert!(encoded.get("expected_revision").is_none());
    assert_eq!(encoded["control_version"], 2);
    let legacy: Command = serde_json::from_value(serde_json::json!({"request_id":Uuid::new_v4(),
        "expected_revision":a.current().revision,"operation":{"type":"output_mix","gain_db":-6.}}))
    .unwrap();
    assert_eq!(legacy.control_version, 1);
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("control_version")
            .is_none()
    );
    a.execute(p, legacy, |_| Ok(())).unwrap();
}
#[test]
fn staged_config_version_stays_fixed_while_commit_receipt_uses_latest_event_sequence() {
    let (mut a, p) = model();
    let command = Command::bound(a.current(), p.device_id(), output());
    let candidate = prepared(&mut a, p, command.clone());
    let saved = candidate.persistent();
    let base = a.snapshot();
    a.set_output_available(false).unwrap();
    a.set_output_available(true).unwrap();
    assert_eq!(
        candidate.persistent().config_revision,
        saved.config_revision
    );
    assert_eq!(a.current().config_revision, base.config_revision);
    let receipt = a.commit_transaction(candidate).unwrap();
    assert_eq!(receipt.config_revision, saved.config_revision);
    assert_eq!(receipt.event_sequence, base.event_sequence + 3);
    assert_eq!(
        a.execute(p, command, |_| panic!("no replay effects"))
            .unwrap()
            .event_sequence,
        receipt.event_sequence
    );
}
#[test]
fn restored_runtime_rejects_old_epoch_even_if_config_version_matches() {
    let (a, p) = model();
    let old = Command::bound(a.current(), p.device_id(), output());
    let epoch = a.current().runtime_epoch;
    let mut restored = Authority::restore(a.persistent()).unwrap();
    assert_eq!(
        restored.current().config_revision,
        a.current().config_revision
    );
    assert_ne!(restored.current().runtime_epoch, epoch);
    assert!(matches!(
        restored.events_after_in_runtime(epoch, a.current().event_sequence),
        Err(ControlError::SnapshotRequired)
    ));
    assert!(matches!(
        restored.prepare_transaction(restored.authenticate(TOKEN).unwrap(), old),
        Err(ControlError::SnapshotRequired)
    ));
}
#[test]
fn replica_rejects_mixed_sequence_protocol_or_impossible_config_jump_atomically() {
    let (mut a, p) = model();
    let initial = a.snapshot();
    let command = Command::bound(a.current(), p.device_id(), output());
    a.execute(p, command, |_| Ok(())).unwrap();
    let event = a.events_after(initial.revision).unwrap().remove(0);
    for change in 0..4 {
        let mut replica = initial.clone();
        let mut invalid = event.clone();
        match change {
            0 => invalid.event_sequence += 1,
            1 => invalid.control_version = 1,
            2 => invalid.config_revision += 2,
            _ => invalid.runtime_epoch = Uuid::new_v4(),
        }
        assert_eq!(
            replica.apply_event(invalid),
            Err(ControlError::SnapshotRequired)
        );
        assert_eq!(
            serde_json::to_value(replica).unwrap(),
            serde_json::to_value(&initial).unwrap()
        );
    }
    let mut replica = initial;
    replica.apply_event(event).unwrap();
    assert_eq!(
        serde_json::to_value(replica).unwrap(),
        serde_json::to_value(a.snapshot()).unwrap()
    );
}
#[test]
fn old_state_import_preserves_identity_and_config_counter_exhaustion_does_not_freeze_runtime() {
    let (mut a, _) = model();
    for available in [false, true] {
        a.set_output_available(available).unwrap();
    }
    let mut old = serde_json::to_value(a.persistent()).unwrap();
    old.as_object_mut().unwrap().remove("config_revision");
    let imported = Authority::restore(serde_json::from_value(old).unwrap()).unwrap();
    assert_eq!(imported.current().hub_id, a.current().hub_id);
    assert_eq!(imported.current().config_revision, a.current().revision);
    let mut saved = a.persistent();
    saved.config_revision = u64::MAX;
    let mut exhausted = Authority::restore(saved).unwrap();
    let p = exhausted.authenticate(TOKEN).unwrap();
    let command = Command::bound(exhausted.current(), p.device_id(), output());
    assert!(matches!(
        exhausted.prepare_transaction(p, command),
        Err(ControlError::QuotaExceeded)
    ));
    let start = Command::bound(
        exhausted.current(),
        p.device_id(),
        Operation::Start { offer: offer() },
    );
    exhausted.execute(p, start, |_| Ok(())).unwrap();
    assert_eq!(exhausted.current().config_revision, u64::MAX);
}

#[test]
fn cache_eviction_ended_start_and_other_identity_cannot_reprepare_old_media() {
    let (mut a, p) = model();
    let second = "another-admin-token-with-at-least-32-bytes";
    let other = a.add_device("other".into(), Role::Admin, second).unwrap();
    let original = Command::bound(
        a.current(),
        p.device_id(),
        Operation::Start { offer: offer() },
    );
    let first = a.execute(p, original.clone(), |_| Ok(())).unwrap();
    a.set_session_status(first.session_id.unwrap(), SessionStatus::UserStopped)
        .unwrap();
    assert_eq!(
        a.execute(p, original.clone(), |_| panic!(
            "ended Start replay prepared media"
        ))
        .unwrap()
        .session_id,
        first.session_id
    );
    let principal = a.authenticate(second).unwrap();
    assert_eq!(principal.device_id(), other);
    assert!(matches!(
        a.prepare_transaction(principal, original.clone()),
        Err(ControlError::Unauthenticated)
    ));
    for index in 0..128 {
        let c = Command::bound(
            a.current(),
            p.device_id(),
            Operation::OutputMix {
                gain_db: Some(if index % 2 == 0 { -3. } else { -6. }),
                muted: None,
            },
        );
        a.execute(p, c, |_| Ok(())).unwrap();
    }
    assert!(matches!(
        a.prepare_transaction(p, original),
        Err(ControlError::RevisionConflict)
    ));
    assert_eq!(a.current().sessions.len(), 1);
    assert!(a.current().streams.is_empty());
}
#[test]
fn revoked_principal_is_rejected_even_if_replayed_protocol_shape_is_invalid() {
    let (mut a, p) = model();
    let token = "revoked-admin-token-with-at-least-32-bytes";
    let id = a.add_device("revoked".into(), Role::Admin, token).unwrap();
    let old = a.authenticate(token).unwrap();
    let mut original = Command::bound(a.current(), id, output());
    a.execute(old, original.clone(), |_| Ok(())).unwrap();
    let revoke = Command::bound(
        a.current(),
        p.device_id(),
        Operation::Revoke { device_id: id },
    );
    a.execute(p, revoke, |_| Ok(())).unwrap();
    original.control_version = 99;
    assert!(matches!(
        a.prepare_transaction(old, original),
        Err(ControlError::Unauthenticated)
    ));
}
