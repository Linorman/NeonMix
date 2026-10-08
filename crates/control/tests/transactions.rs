use neonmix_control::*;
use uuid::Uuid;
const TOKEN: &str = "transaction-admin-credential-32-bytes-minimum";
fn command(a: &Authority, gain: f32) -> Command {
    Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        runtime_epoch: None,
        credential_id: None,
        request_id: Uuid::new_v4(),
        expected_revision: Some(a.current().revision),
        operation: Operation::OutputMix {
            gain_db: Some(gain),
            muted: None,
        },
    }
}
#[test]
fn staged_command_excludes_durable_writers_and_preserves_live_output_loss() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let command = command(&a, -3.);
    let prepared = prepared(&mut a, principal, command.clone());
    assert_eq!(a.current().output.gain_db, -12.);
    assert_eq!(prepared.snapshot().output.gain_db, -3.);
    assert!(matches!(
        a.execute(principal, command.clone(), |_| panic!(
            "frozen durable callback"
        )),
        Err(ControlError::Busy)
    ));
    a.set_output_available(false).unwrap();
    assert!(
        !a.current().output.available,
        "new admission must immediately see hardware loss"
    );
    assert_eq!(prepared.persistent().output.gain_db, -3.);
    let receipt = a.commit_transaction(prepared).unwrap();
    assert_eq!(a.current().output.gain_db, -3.);
    assert!(!a.current().output.available);
    let duplicate = a
        .execute(principal, command, |_| {
            panic!("idempotent replay must not save")
        })
        .unwrap();
    assert_eq!(duplicate.revision, receipt.revision);
    let events = a.events_after(receipt.revision - 2).unwrap();
    assert_eq!(
        events.len(),
        2,
        "health publishes before commit and remains in history"
    );
    assert!(!events[0].output.as_ref().unwrap().available);
    assert_eq!(events[1].output.as_ref().unwrap().gain_db, -3.);
}
#[test]
fn failed_preparation_aborts_only_the_candidate_and_keeps_health_updates() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let new_command = command(&a, -3.);
    let prepared = prepared(&mut a, principal, new_command);
    a.set_output_available(false).unwrap();
    assert_eq!(a.abort_transaction(Uuid::new_v4()), Err(ControlError::Busy));
    a.abort_transaction(prepared.token()).unwrap();
    assert_eq!(a.current().output.gain_db, -12.);
    assert!(!a.current().output.available);
    assert!(matches!(
        a.commit_transaction(prepared),
        Err(ControlError::Busy)
    ));
    a.execute(principal, command(&a, -6.), |_| Ok(())).unwrap();
    assert_eq!(a.current().output.gain_db, -6.);
}

fn prepared(a: &mut Authority, principal: Principal, command: Command) -> PreparedCommand {
    match a.prepare_transaction(principal, command).unwrap() {
        Preparation::Prepared(prepared) => prepared,
        Preparation::Replay(_) => panic!("expected new transaction"),
    }
}

#[test]
fn completed_replay_precedes_unrelated_pending_transaction_and_checks_fingerprint() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let original = command(&a, -3.);
    let receipt = a.execute(principal, original.clone(), |_| Ok(())).unwrap();
    let next = command(&a, -6.);
    let pending = prepared(&mut a, principal, next);
    match a.prepare_transaction(principal, original.clone()).unwrap() {
        Preparation::Replay(replayed) => assert_eq!(replayed.revision, receipt.revision),
        Preparation::Prepared(_) => panic!("completed request must not become a candidate"),
    }
    let mut reused = original.clone();
    reused.operation = Operation::OutputMix {
        gain_db: Some(-9.),
        muted: None,
    };
    assert!(matches!(
        a.prepare_transaction(principal, reused),
        Err(ControlError::IdempotencyConflict)
    ));
    assert_eq!(
        a.execute(principal, original, |_| panic!("replay has no prepare"))
            .unwrap()
            .revision,
        receipt.revision
    );
    a.commit_transaction(pending).unwrap();
    assert_eq!(a.current().output.gain_db, -6.);
}

#[test]
fn cached_receipt_does_not_bypass_current_principal_validation() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let token = "second-admin-credential-with-32-bytes-minimum";
    let device = a.add_device("second".into(), Role::Admin, token).unwrap();
    let principal = a.authenticate(token).unwrap();
    let original = command(&a, -3.);
    a.execute(principal, original.clone(), |_| Ok(())).unwrap();
    let admin = a.authenticate(TOKEN).unwrap();
    a.execute(
        admin,
        Command {
            control_version: 1,
            expected_config_revision: None,
            expected_event_sequence: None,
            runtime_epoch: None,
            credential_id: None,
            request_id: Uuid::new_v4(),
            expected_revision: Some(a.current().revision),
            operation: Operation::Revoke { device_id: device },
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(matches!(
        a.prepare_transaction(principal, original),
        Err(ControlError::Unauthenticated)
    ));
}

#[test]
fn runtime_epoch_and_requesting_identity_are_checked_before_replay_or_side_effects() {
    let mut authority = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = authority.authenticate(TOKEN).unwrap();
    let original = Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        request_id: Uuid::new_v4(),
        runtime_epoch: Some(authority.current().runtime_epoch),
        credential_id: Some(principal.device_id()),
        expected_revision: Some(authority.current().revision),
        operation: Operation::OutputMix {
            gain_db: Some(-3.),
            muted: None,
        },
    };
    authority
        .execute(principal, original.clone(), |_| Ok(()))
        .unwrap();
    let saved = authority.persistent();
    let mut restored = Authority::restore(saved).unwrap();
    assert_ne!(
        restored.current().runtime_epoch,
        authority.current().runtime_epoch
    );
    let mut stale = original.clone();
    stale.expected_revision = Some(restored.current().revision);
    assert!(matches!(
        restored.prepare_transaction(restored.authenticate(TOKEN).unwrap(), stale),
        Err(ControlError::SnapshotRequired)
    ));
    let mut swapped = original;
    swapped.credential_id = Some(Uuid::new_v4());
    assert!(matches!(
        authority.prepare_transaction(principal, swapped),
        Err(ControlError::Unauthenticated)
    ));
    let before = restored.snapshot();
    let mut old_event = authority.events_after(0).unwrap().pop().unwrap();
    old_event.revision = before.revision + 1;
    let mut replica = before.clone();
    assert_eq!(
        replica.apply_event(old_event),
        Err(ControlError::SnapshotRequired)
    );
    assert_eq!(replica.revision, before.revision);
}

fn assert_replica(a: &Authority, replica: &mut Snapshot) {
    for event in a.events_after(replica.revision).unwrap() {
        replica.apply_event(event).unwrap();
    }
    assert_eq!(
        serde_json::to_value(&*replica).unwrap(),
        serde_json::to_value(a.snapshot()).unwrap()
    );
}
#[test]
fn health_false_get_true_then_commit_or_abort_preserves_each_public_cursor() {
    for commit in [false, true] {
        let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
        let principal = a.authenticate(TOKEN).unwrap();
        let mut before = a.snapshot();
        let intent = command(&a, -3.);
        let candidate = prepared(&mut a, principal, intent.clone());
        let first = before.revision;
        a.set_output_available(false).unwrap();
        assert_eq!(a.current().revision, first + 1);
        assert_replica(&a, &mut before);
        let mut intermediate_get = a.snapshot();
        a.set_output_available(true).unwrap();
        assert_eq!(a.current().revision, first + 2);
        assert_replica(&a, &mut before);
        if commit {
            let receipt = a.commit_transaction(candidate).unwrap();
            assert_eq!(receipt.revision, first + 3);
            assert_eq!(
                a.execute(principal, intent, |_| panic!("no replay write"))
                    .unwrap()
                    .revision,
                receipt.revision
            );
        } else {
            let latest = a.snapshot();
            a.abort_transaction(candidate.token()).unwrap();
            assert_eq!(
                serde_json::to_value(latest).unwrap(),
                serde_json::to_value(a.snapshot()).unwrap()
            );
        }
        assert_replica(&a, &mut before);
        assert_replica(&a, &mut intermediate_get);
        assert_eq!(a.current().output.gain_db, if commit { -3. } else { -12. });
        assert!(a.current().output.available);
        assert!(!a.transaction_pending());
    }
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
fn op(a: &Authority, operation: Operation) -> Command {
    Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        runtime_epoch: Some(a.current().runtime_epoch),
        credential_id: None,
        request_id: Uuid::new_v4(),
        expected_revision: Some(a.current().revision),
        operation,
    }
}
#[test]
fn session_ending_during_saved_mix_is_not_resurrected_but_preferences_survive() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let start = op(&a, Operation::Start { offer: offer() });
    let receipt = a.execute(principal, start, |_| Ok(())).unwrap();
    let session = receipt.session_id.unwrap();
    let stream = receipt.stream_id.unwrap();
    let intent = op(
        &a,
        Operation::StreamMix {
            stream_id: stream,
            gain_db: Some(-9.),
            muted: Some(true),
            solo: Some(true),
        },
    );
    let candidate = prepared(&mut a, principal, intent);
    let mut replica = a.snapshot();
    a.set_session_status(session, SessionStatus::NetworkInterrupted)
        .unwrap();
    assert_replica(&a, &mut replica);
    assert!(!a.current().streams.contains_key(&stream));
    a.commit_transaction(candidate).unwrap();
    assert_replica(&a, &mut replica);
    assert_eq!(
        a.current().sessions[&session].status,
        SessionStatus::NetworkInterrupted
    );
    assert!(!a.current().streams.contains_key(&stream));
    let start = op(&a, Operation::Start { offer: offer() });
    let next = a.execute(principal, start, |_| Ok(())).unwrap();
    let mix = a.current().streams[&next.stream_id.unwrap()].mix;
    assert_eq!(mix.gain_db, -9.);
    assert!(mix.muted);
    assert!(
        !mix.solo,
        "old transient Solo must not be applied to the new session"
    );
}
#[test]
fn unrelated_session_health_during_registration_is_retained_without_deferred_replay() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let start = op(&a, Operation::Start { offer: offer() });
    let session = a
        .execute(principal, start, |_| Ok(()))
        .unwrap()
        .session_id
        .unwrap();
    let register = op(
        &a,
        Operation::RegisterDevice {
            name: "new".into(),
            role: Role::Member,
            token_sha256: "b".repeat(64),
        },
    );
    let candidate = prepared(&mut a, principal, register);
    let mut replica = a.snapshot();
    for status in [
        SessionStatus::Playing,
        SessionStatus::NetworkDegraded,
        SessionStatus::NetworkInterrupted,
    ] {
        a.set_session_status(session, status).unwrap();
        assert_replica(&a, &mut replica);
    }
    a.commit_transaction(candidate).unwrap();
    assert_replica(&a, &mut replica);
    assert_eq!(
        a.current().sessions[&session].status,
        SessionStatus::NetworkInterrupted
    );
    assert!(a.current().streams.is_empty());
}

#[test]
fn pending_health_keeps_bounded_event_history_and_requires_snapshot_after_eviction() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let intent = command(&a, -3.);
    let candidate = prepared(&mut a, principal, intent);
    let mut stale = a.snapshot();
    let stale_bytes = serde_json::to_value(&stale).unwrap();
    for index in 0..EVENT_CAPACITY + 2 {
        a.set_output_available(index % 2 != 0).unwrap();
    }
    assert!(matches!(
        a.events_after(stale.revision),
        Err(ControlError::SnapshotRequired)
    ));
    let earliest = a
        .events_after(a.current().revision - EVENT_CAPACITY as u64)
        .unwrap()
        .remove(0);
    assert_eq!(
        stale.apply_event(earliest),
        Err(ControlError::SnapshotRequired)
    );
    assert_eq!(serde_json::to_value(stale).unwrap(), stale_bytes);
    let mut fresh = a.snapshot();
    a.commit_transaction(candidate).unwrap();
    assert_replica(&a, &mut fresh);
    assert_eq!(fresh.output.gain_db, -3.);
    assert!(fresh.output.available);
}
#[test]
fn pending_health_reserves_final_sequence_for_commit_without_unversioned_mutation() {
    let a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let mut saved = a.persistent();
    saved.revision = u64::MAX - 2;
    let mut a = Authority::restore(saved).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let intent = command(&a, -3.);
    let candidate = prepared(&mut a, principal, intent);
    let before = serde_json::to_value(a.snapshot()).unwrap();
    assert_eq!(
        a.set_output_available(false),
        Err(ControlError::QuotaExceeded)
    );
    assert_eq!(serde_json::to_value(a.snapshot()).unwrap(), before);
    a.commit_transaction(candidate).unwrap();
    assert_eq!(a.current().revision, u64::MAX);
    assert_eq!(a.current().output.gain_db, -3.);
    assert!(a.current().output.available);
}
