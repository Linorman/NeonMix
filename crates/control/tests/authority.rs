use neonmix_control::*;
use uuid::Uuid;
const ADMIN: &str = "administrator-credential-32-bytes-minimum";
const MEMBER: &str = "member-credential-32-bytes-minimum";
const OTHER: &str = "another-member-credential-32-bytes-minimum";
fn model() -> Authority {
    let mut a = Authority::new("coreaudio:test".into(), "admin".into(), ADMIN).unwrap();
    a.add_device("A".into(), Role::Member, MEMBER).unwrap();
    a.add_device("B".into(), Role::Member, OTHER).unwrap();
    a
}
fn offer() -> MediaOffer {
    MediaOffer {
        version: 1,
        codec: "opus".into(),
        rate: 48000,
        channels: 2,
        packet_frames: 480,
        payload_type: 96,
        ssrc: 1,
        stream_epoch: 1,
        udp_port: 5000,
        certificate_sha256: "a".repeat(64),
    }
}
fn command(a: &Authority, operation: Operation) -> Command {
    Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        runtime_epoch: None,
        credential_id: None,
        request_id: Uuid::new_v4(),
        expected_revision: Some(a.snapshot().revision),
        operation,
    }
}
fn execute(a: &mut Authority, token: &str, op: Operation) -> Result<Receipt, ControlError> {
    let c = command(a, op);
    a.execute(a.authenticate(token)?, c, |_| Ok(()))
}
#[test]
fn member_cannot_solo_or_change_others_and_master() {
    let mut a = model();
    let stream = execute(&mut a, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .stream_id
        .unwrap();
    let other = execute(&mut a, OTHER, Operation::Start { offer: offer() })
        .unwrap()
        .stream_id
        .unwrap();
    for op in [
        Operation::StreamMix {
            stream_id: stream,
            gain_db: None,
            muted: None,
            solo: Some(true),
        },
        Operation::StreamMix {
            stream_id: other,
            gain_db: Some(-6.0),
            muted: None,
            solo: None,
        },
        Operation::OutputMix {
            gain_db: Some(-6.0),
            muted: None,
        },
    ] {
        let rev = a.snapshot().revision;
        assert_eq!(
            execute(&mut a, MEMBER, op).unwrap_err(),
            ControlError::PermissionDenied
        );
        assert_eq!(a.snapshot().revision, rev);
    }
    execute(
        &mut a,
        MEMBER,
        Operation::StreamMix {
            stream_id: stream,
            gain_db: Some(-6.0),
            muted: Some(true),
            solo: None,
        },
    )
    .unwrap();
    assert_eq!(a.snapshot().streams[&other].mix.gain_db, 0.0);
}
#[test]
fn revision_idempotency_and_full_dsp_queue_do_not_publish_success() {
    let mut a = model();
    let p = a.authenticate(ADMIN).unwrap();
    let c = command(
        &a,
        Operation::OutputMix {
            gain_db: Some(-6.0),
            muted: None,
        },
    );
    let rev = a.snapshot().revision;
    assert_eq!(
        a.execute(p, c.clone(), |_| Err(ControlError::Busy))
            .unwrap_err(),
        ControlError::Busy
    );
    assert_eq!(a.snapshot().revision, rev);
    let receipt = a.execute(p, c.clone(), |_| Ok(())).unwrap();
    let duplicate = a
        .execute(p, c.clone(), |_| panic!("idempotent retry touched DSP"))
        .unwrap();
    assert_eq!(receipt.revision, duplicate.revision);
    let mut changed = c.clone();
    changed.operation = Operation::OutputMix {
        gain_db: None,
        muted: Some(true),
    };
    assert_eq!(
        a.execute(p, changed, |_| Ok(())).unwrap_err(),
        ControlError::IdempotencyConflict
    );
    let mut stale = c;
    stale.request_id = Uuid::new_v4();
    assert_eq!(
        a.execute(p, stale, |_| Ok(())).unwrap_err(),
        ControlError::RevisionConflict
    );
}
#[test]
fn disconnect_allow_and_revoke_have_distinct_reconnect_rules() {
    let mut a = model();
    let member = a.authenticate(MEMBER).unwrap();
    let id = member.device_id();
    let first = execute(&mut a, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .session_id
        .unwrap();
    execute(&mut a, ADMIN, Operation::Disconnect { device_id: id }).unwrap();
    assert_eq!(
        a.snapshot().sessions[&first].status,
        SessionStatus::AdminDisconnected
    );
    assert_eq!(
        execute(&mut a, MEMBER, Operation::Start { offer: offer() }).unwrap_err(),
        ControlError::PlaybackBlocked
    );
    execute(&mut a, ADMIN, Operation::AllowPlayback { device_id: id }).unwrap();
    let second = execute(&mut a, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .session_id
        .unwrap();
    assert_ne!(
        a.snapshot().sessions[&first].media_context,
        a.snapshot().sessions[&second].media_context
    );
    execute(&mut a, ADMIN, Operation::Revoke { device_id: id }).unwrap();
    assert_eq!(
        a.authenticate(MEMBER).unwrap_err(),
        ControlError::Unauthenticated
    );
    let c = command(&a, Operation::Start { offer: offer() });
    assert_eq!(
        a.execute(member, c, |_| Ok(())).unwrap_err(),
        ControlError::Unauthenticated
    );
    assert_eq!(
        a.snapshot().sessions[&second].status,
        SessionStatus::Revoked
    );
}
#[test]
fn snapshot_event_cursors_and_version_negotiation() {
    let mut a = model();
    let snapshot = a.snapshot();
    for _ in 0..EVENT_CAPACITY + 1 {
        execute(
            &mut a,
            ADMIN,
            Operation::OutputMix {
                gain_db: None,
                muted: Some(false),
            },
        )
        .unwrap();
    }
    assert_eq!(
        a.events_after(snapshot.revision).unwrap_err(),
        ControlError::SnapshotRequired
    );
    let s = a.snapshot();
    assert!(a.events_after(s.revision).unwrap().is_empty());
    execute(
        &mut a,
        ADMIN,
        Operation::OutputMix {
            gain_db: None,
            muted: Some(true),
        },
    )
    .unwrap();
    assert_eq!(
        a.events_after(s.revision).unwrap()[0].revision,
        s.revision + 1
    );
    let mut incompatible = offer();
    incompatible.version = 2;
    assert_eq!(
        execute(
            &mut a,
            MEMBER,
            Operation::Start {
                offer: incompatible
            }
        )
        .unwrap_err(),
        ControlError::IncompatibleVersion
    );
}
#[test]
fn stopped_sender_is_not_reactivated_by_status_updates() {
    let mut a = model();
    let id = execute(&mut a, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .session_id
        .unwrap();
    execute(&mut a, MEMBER, Operation::Stop { session_id: id }).unwrap();
    assert_eq!(
        a.set_session_status(id, SessionStatus::Playing)
            .unwrap_err(),
        ControlError::PlaybackBlocked
    );
    assert!(a.snapshot().streams.is_empty());
}

#[test]
fn exhausted_revision_rejects_runtime_changes_without_unversioned_mutation() {
    let mut saved = model().persistent();
    saved.revision = u64::MAX - 2;
    let mut authority = Authority::restore(saved).unwrap();
    let id = execute(&mut authority, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .session_id
        .unwrap();
    let before = serde_json::to_value(authority.snapshot()).unwrap();
    assert_eq!(authority.snapshot().revision, u64::MAX);
    for status in [SessionStatus::Playing, SessionStatus::NetworkInterrupted] {
        assert_eq!(
            authority.set_session_status(id, status),
            Err(ControlError::QuotaExceeded)
        );
        assert_eq!(serde_json::to_value(authority.snapshot()).unwrap(), before);
    }
    assert_eq!(
        authority.set_output_available(false),
        Err(ControlError::QuotaExceeded)
    );
    assert_eq!(serde_json::to_value(authority.snapshot()).unwrap(), before);
    assert_eq!(authority.events_after(u64::MAX - 1).unwrap().len(), 1);
    assert!(
        authority
            .set_session_status(id, SessionStatus::Buffering)
            .is_ok()
    );
    assert!(authority.set_output_available(true).is_ok());
}
#[test]
fn wire_commands_roundtrip_and_reject_self_reported_role() {
    let a = model();
    let c = command(&a, Operation::Start { offer: offer() });
    let value = serde_json::to_value(&c).unwrap();
    assert_eq!(serde_json::from_value::<Command>(value.clone()).unwrap(), c);
    let mut malicious = value;
    malicious["role"] = serde_json::json!("admin");
    assert!(serde_json::from_value::<Command>(malicious).is_err());
}
#[test]
fn provision_and_restore_preserve_trust_revocation_and_mix_but_clear_media() {
    let mut a = model();
    let member = a.authenticate(MEMBER).unwrap().device_id();
    let receipt = execute(&mut a, MEMBER, Operation::Start { offer: offer() }).unwrap();
    execute(
        &mut a,
        ADMIN,
        Operation::StreamMix {
            stream_id: receipt.stream_id.unwrap(),
            gain_db: Some(-9.0),
            muted: Some(true),
            solo: Some(true),
        },
    )
    .unwrap();
    let digest: String = token_digest("new-credential-with-at-least-32-bytes")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let registration = Operation::RegisterDevice {
        name: "new".into(),
        role: Role::Member,
        token_sha256: digest,
    };
    assert_eq!(
        execute(&mut a, MEMBER, registration.clone()).unwrap_err(),
        ControlError::PermissionDenied
    );
    let registered = execute(&mut a, ADMIN, registration)
        .unwrap()
        .device_id
        .unwrap();
    assert_eq!(
        a.authenticate("new-credential-with-at-least-32-bytes")
            .unwrap()
            .device_id(),
        registered
    );
    execute(&mut a, ADMIN, Operation::Disconnect { device_id: member }).unwrap();
    let bytes = serde_json::to_vec(&a.persistent()).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(MEMBER));
    let mut restored = Authority::restore(serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(restored.snapshot().hub_id, a.snapshot().hub_id);
    assert!(restored.snapshot().sessions.is_empty());
    assert_eq!(
        execute(&mut restored, MEMBER, Operation::Start { offer: offer() }).unwrap_err(),
        ControlError::PlaybackBlocked
    );
    execute(
        &mut restored,
        ADMIN,
        Operation::AllowPlayback { device_id: member },
    )
    .unwrap();
    let id = execute(&mut restored, MEMBER, Operation::Start { offer: offer() })
        .unwrap()
        .stream_id
        .unwrap();
    let mix = restored.snapshot().streams[&id].mix;
    assert_eq!(mix.gain_db, -9.0);
    assert!(mix.muted);
    assert!(!mix.solo);
}
#[test]
fn durable_failure_does_not_commit_and_event_deltas_rebuild_snapshot() {
    let mut a = model();
    let p = a.authenticate(ADMIN).unwrap();
    let before = a.snapshot();
    let c = command(
        &a,
        Operation::OutputMix {
            gain_db: Some(-9.0),
            muted: None,
        },
    );
    assert_eq!(
        a.execute_durable(p, c.clone(), |next, saved| {
            assert_eq!(next.revision, saved.revision);
            assert_eq!(saved.output.gain_db, -9.0);
            Err(ControlError::Busy)
        })
        .unwrap_err(),
        ControlError::Busy
    );
    assert_eq!(a.snapshot().revision, before.revision);
    assert_eq!(a.snapshot().output.gain_db, before.output.gain_db);
    a.execute(p, c, |_| Ok(())).unwrap();
    let stream = execute(&mut a, MEMBER, Operation::Start { offer: offer() }).unwrap();
    execute(
        &mut a,
        MEMBER,
        Operation::StreamMix {
            stream_id: stream.stream_id.unwrap(),
            gain_db: Some(-6.0),
            muted: Some(true),
            solo: None,
        },
    )
    .unwrap();
    execute(
        &mut a,
        MEMBER,
        Operation::Stop {
            session_id: stream.session_id.unwrap(),
        },
    )
    .unwrap();
    let mut rebuilt = before;
    for event in a.events_after(rebuilt.revision).unwrap() {
        rebuilt.apply_event(event).unwrap();
    }
    assert_eq!(
        serde_json::to_value(rebuilt).unwrap(),
        serde_json::to_value(a.snapshot()).unwrap()
    );
}
#[test]
fn terminal_session_history_is_bounded_without_blocking_new_sessions() {
    let mut a = model();
    for _ in 0..270 {
        let receipt = execute(&mut a, MEMBER, Operation::Start { offer: offer() }).unwrap();
        assert!(receipt.stream_id.unwrap() < (1u64 << 53));
        execute(
            &mut a,
            MEMBER,
            Operation::Stop {
                session_id: receipt.session_id.unwrap(),
            },
        )
        .unwrap();
    }
    assert_eq!(a.snapshot().sessions.len(), 256);
    assert!(a.snapshot().streams.is_empty());
    let admin = a.authenticate(ADMIN).unwrap().device_id();
    assert_eq!(
        execute(&mut a, ADMIN, Operation::Revoke { device_id: admin }).unwrap_err(),
        ControlError::InvalidArgument
    );
}
#[test]
fn replica_rejects_gaps_and_foreign_hubs_without_partial_updates() {
    let mut authority = model();
    let mut replica = authority.snapshot();
    let before = serde_json::to_value(&replica).unwrap();
    execute(
        &mut authority,
        ADMIN,
        Operation::OutputMix {
            gain_db: Some(-6.0),
            muted: None,
        },
    )
    .unwrap();
    let event = authority
        .events_after(replica.revision)
        .unwrap()
        .pop()
        .unwrap();
    let mut gap = event.clone();
    gap.revision += 1;
    assert_eq!(
        replica.apply_event(gap).unwrap_err(),
        ControlError::SnapshotRequired
    );
    assert_eq!(serde_json::to_value(&replica).unwrap(), before);
    let mut foreign = event.clone();
    foreign.hub_id = Uuid::new_v4();
    assert_eq!(
        replica.apply_event(foreign).unwrap_err(),
        ControlError::SnapshotRequired
    );
    assert_eq!(serde_json::to_value(&replica).unwrap(), before);
    replica.apply_event(event).unwrap();
    assert_eq!(replica.output.gain_db, -6.0);
}
