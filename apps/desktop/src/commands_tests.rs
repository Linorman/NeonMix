use super::*;
use crate::intent::{Field, Phase, TargetKey};
use neonmix_control::{Authority, Command, MediaOffer};
use neonmix_desktop_service::IntentCommand;

const ADMIN: &str = "desktop-intent-admin-token-at-least-32-bytes";
use neonmix_airplay_adapter::control::{AirplayActionV2 as Airplay, AirplayCommandV2};
struct Fixture {
    app: Desktop,
    authority: Authority,
    work: Receiver<Work>,
    results: SyncSender<Result<Outcome, UiError>>,
    streams: Vec<u64>,
}
impl Fixture {
    fn new() -> Self {
        let mut authority = Authority::new("test-output".into(), "admin".into(), ADMIN).unwrap();
        let admin = authority.authenticate(ADMIN).unwrap().device_id();
        let mut streams = Vec::new();
        for index in 0..3 {
            let token = format!("member-{index}-token-with-at-least-32-bytes");
            authority
                .add_device(format!("member-{index}"), Role::Member, &token)
                .unwrap();
            let command = Command {
                control_version: 1,
                expected_config_revision: None,
                expected_event_sequence: None,
                request_id: uuid::Uuid::new_v4(),
                runtime_epoch: Some(authority.current().runtime_epoch),
                credential_id: None,
                expected_revision: Some(authority.current().revision),
                operation: Operation::Start {
                    offer: MediaOffer {
                        version: 1,
                        codec: "opus".into(),
                        rate: 48000,
                        channels: 2,
                        packet_frames: 480,
                        payload_type: 96,
                        ssrc: index + 1,
                        stream_epoch: 1,
                        udp_port: 32000 + index as u16,
                        certificate_sha256: "a".repeat(64),
                    },
                },
            };
            let receipt = authority
                .execute(authority.authenticate(&token).unwrap(), command, |_| Ok(()))
                .unwrap();
            streams.push(receipt.stream_id.unwrap());
        }
        let mut app = Desktop::empty(Client::new(".local/test-intent-desktop"), true, false);
        app.snapshot = Some(authority.snapshot());
        app.status = Some(ServiceStatus {
            lifecycle: None,
            intent_version: neonmix_desktop_service::INTENT_VERSION,
            managed_control_version: 1,
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![neonmix_desktop_service::ProfileInfo {
                credential: app.credential.clone(),
                device_id: Some(admin),
                hub_id: Some(authority.current().hub_id),
                name: Some("admin".into()),
                role: Some(Role::Admin),
                pending: false,
            }],
            sender_options: None,
            output_binding: None,
        });
        app.fresh = Some(Instant::now());
        app.synced = app.fresh;
        app.online = false;
        app.sync_command_context();
        let (send, work) = mpsc::sync_channel(128);
        app.worker = Some(send);
        let (results, receive) = mpsc::sync_channel(128);
        app.results = Some(receive);
        Self {
            app,
            authority,
            work,
            results,
            streams,
        }
    }
    fn next(&self) -> Request {
        let Work::Action(request) = self.work.try_recv().expect("no dispatched intent") else {
            panic!("unexpected poll");
        };
        request
    }
    fn execute(&mut self, request: &Request) {
        let Request::Intent {
            hub_id,
            credential_id,
            command: IntentCommand::Native(command),
            ..
        } = request
        else {
            panic!("not a bound native intent");
        };
        assert_eq!(*hub_id, self.authority.current().hub_id);
        assert_eq!(
            *credential_id,
            self.authority.authenticate(ADMIN).unwrap().device_id()
        );
        assert_eq!(
            command.runtime_epoch,
            Some(self.authority.current().runtime_epoch)
        );
        self.authority
            .execute(
                self.authority.authenticate(ADMIN).unwrap(),
                command.clone(),
                |_| Ok(()),
            )
            .unwrap();
    }
    fn acknowledge(&mut self, request: Request) {
        self.results
            .send(Ok(Outcome::Action(Box::new(request), Value::Null)))
            .unwrap();
        self.app.process();
    }
    fn refresh(&mut self) {
        self.app.snapshot = Some(self.authority.snapshot());
        self.app.fresh = Some(Instant::now());
        self.app.synced = self.app.fresh;
        self.app.process();
    }
    fn mix(&mut self, stream: u64, gain: Option<f32>, mute: Option<bool>, solo: Option<bool>) {
        self.app.operation(Operation::StreamMix {
            stream_id: stream,
            gain_db: gain,
            muted: mute,
            solo,
        });
    }
}

#[test]
fn delayed_first_write_preserves_b_gain_c_mute_b_solo_and_fair_order() {
    let mut f = Fixture::new();
    f.app.master_mute(true);
    let first = f.next();
    let first_bytes = serde_json::to_vec(&first).unwrap();
    let b = f.streams[1];
    let c = f.streams[2];
    f.mix(b, Some(-3.), None, None);
    f.mix(c, None, Some(true), None);
    f.mix(b, None, None, Some(true));
    for gain in [-6., -9., -12.] {
        f.mix(b, Some(gain), None, None);
    }
    assert_eq!(f.app.intents.queued.len(), 3);
    assert_eq!(
        serde_json::to_vec(&f.app.intents.flight.as_ref().unwrap().request).unwrap(),
        first_bytes
    );
    f.execute(&first);
    f.acknowledge(first);
    for expected in [Field::Gain, Field::Mute, Field::Solo] {
        f.refresh();
        let request = f.next();
        assert_eq!(
            crate::intent::field(&f.app.intents.flight.as_ref().unwrap().intent.write),
            Some(expected)
        );
        f.execute(&request);
        f.acknowledge(request);
    }
    assert_eq!(f.authority.current().streams[&b].mix.gain_db, -12.);
    assert!(f.authority.current().streams[&b].mix.solo);
    assert!(f.authority.current().streams[&c].mix.muted);
    assert!(f.app.intents.queued.is_empty());
    assert!(
        f.app
            .intents
            .history
            .iter()
            .all(|intent| intent.phase == Phase::Acknowledged)
    );
}

#[test]
fn context_change_cancels_queued_fields_and_does_not_retarget_inflight_reply_or_undo() {
    for identity_only in [false, true] {
        let mut f = Fixture::new();
        f.app.master_mute(true);
        let original = f.next();
        f.mix(f.streams[1], Some(-6.), None, None);
        let old = f.app.intents.current.clone().unwrap();
        let mut next = f.authority.snapshot();
        let profile = &mut f.app.status.as_mut().unwrap().profiles[0];
        if identity_only {
            profile.device_id = Some(next.streams[&f.streams[1]].device_id);
        } else {
            next.hub_id = uuid::Uuid::new_v4();
            next.runtime_epoch = uuid::Uuid::new_v4();
            profile.hub_id = Some(next.hub_id);
        }
        f.app.snapshot = Some(next.clone());
        f.app.sync_command_context();
        assert_ne!(f.app.intents.current.as_ref(), Some(&old));
        assert!(f.app.intents.queued.is_empty());
        assert!(f.app.undo.is_none());
        let message = f.app.message.clone();
        f.execute(&original);
        f.acknowledge(original);
        assert_eq!(f.app.snapshot.as_ref().unwrap().hub_id, next.hub_id);
        assert_eq!(
            f.app.snapshot.as_ref().unwrap().output.muted,
            next.output.muted
        );
        assert_eq!(f.app.message, message);
        assert!(f.app.undo.is_none());
        assert_eq!(f.app.intents.history.back().unwrap().context, old);
    }
}

#[test]
fn unknown_reconciles_the_same_envelope_and_only_acknowledgement_creates_undo() {
    let mut f = Fixture::new();
    f.app.master_mute(true);
    let request = f.next();
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(f.app.undo.is_none());
    // Commit succeeded, but the IPC reply was lost.
    f.execute(&request);
    f.results
        .send(Err(UiError::from("background_timeout")))
        .unwrap();
    f.app.process();
    assert!(f.app.undo.is_none());
    assert_eq!(f.app.intents.unknown.len(), 1);
    f.refresh();
    f.app.reconcile_intent();
    let replay = f.next();
    assert_eq!(serde_json::to_vec(&replay).unwrap(), bytes);
    f.execute(&replay);
    f.acknowledge(replay);
    f.refresh();
    assert!(f.app.undo_available());
    f.app.restore_last();
    let inverse = f.next();
    f.execute(&inverse);
    f.acknowledge(inverse);
    assert!(!f.authority.current().output.muted);
}

#[test]
fn undo_rechecks_field_after_a_wait_and_preserves_other_fields() {
    let mut f = Fixture::new();
    f.app.master_gain(-12., -6.);
    let gain = f.next();
    f.execute(&gain);
    f.acknowledge(gain);
    f.refresh();
    assert!(f.app.undo_available());
    // Queue Undo while another ordinary write is in flight.
    f.mix(f.streams[0], None, Some(true), None);
    let pending = f.next();
    f.app.restore_last();
    assert_eq!(f.app.intents.queued.len(), 1);
    f.execute(&pending);
    f.acknowledge(pending);
    let command = Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        request_id: uuid::Uuid::new_v4(),
        runtime_epoch: None,
        credential_id: None,
        expected_revision: Some(f.authority.current().revision),
        operation: Operation::OutputMix {
            gain_db: Some(-3.),
            muted: Some(true),
        },
    };
    f.authority
        .execute(
            f.authority.authenticate(ADMIN).unwrap(),
            command,
            |_| Ok(()),
        )
        .unwrap();
    f.refresh();
    assert!(
        f.work.try_recv().is_err(),
        "Undo must not overwrite another client's gain"
    );
    assert_eq!(f.authority.current().output.gain_db, -3.);
    assert!(f.authority.current().output.muted);
    assert_eq!(f.app.intents.history.back().unwrap().phase, Phase::Conflict);
}

#[test]
fn terminal_target_is_a_barrier_and_capacity_never_evicts_another_channel() {
    let mut f = Fixture::new();
    f.app.master_mute(true);
    let first = f.next();
    let b = f.streams[1];
    f.mix(b, Some(-3.), None, None);
    f.mix(b, None, Some(true), None);
    let device = f.authority.current().streams[&b].device_id;
    f.app.operation(Operation::Disconnect { device_id: device });
    f.app.operation(Operation::Disconnect { device_id: device });
    assert_eq!(f.app.intents.queued.len(), 1);
    f.mix(b, Some(-6.), None, None);
    assert_eq!(f.app.intents.queued.len(), 1);
    for _ in 0..127 {
        f.app
            .operation(Operation::AllowPlayback { device_id: device });
    }
    assert_eq!(f.app.intents.queued.len(), crate::intent::CAPACITY);
    f.app
        .operation(Operation::AllowPlayback { device_id: device });
    assert_eq!(f.app.intents.queued.len(), crate::intent::CAPACITY);
    assert_eq!(f.app.message, Message::IntentQueueFull);
    f.execute(&first);
    f.acknowledge(first);
    f.refresh();
    let stop = f.next();
    f.execute(&stop);
    assert!(!f.authority.current().streams.contains_key(&b));
    assert!(
        matches!(f.app.intents.flight.as_ref().unwrap().intent.target, TargetKey::Device(id) if id == device)
    );
}

fn airplay(f: &mut Fixture, patch: bool) {
    f.app.airplay = Some(
        serde_json::json!({"runtime_epoch":f.authority.current().runtime_epoch,
        "capabilities":if patch { vec!["runtime_epoch", "patch_mix_source"] } else { vec!["runtime_epoch"] },
        "revision":7,"sources":[{"source_id":"source","gain_db":-3.,"muted":false}],
        "sessions":[{"source_id":"source","source_name":"phone","session_id":77,"stream_id":299,
            "stream_epoch":4,"receiver_id":"00000000-0000-4000-8000-000000000001",
            "mix":{"gain_db":-3.,"muted":false,"solo":false}}]}),
    );
}
#[test]
fn airplay_mute_then_gain_and_solo_then_gain_emit_independent_patches() {
    let mut f = Fixture::new();
    airplay(&mut f, true);
    f.app.airplay_operation(Airplay::PatchMixSource {
        source_id: "source".into(),
        session_id: 77,
        gain_db: None,
        muted: Some(true),
        solo: None,
    });
    let first = f.next();
    f.app.airplay_operation(Airplay::PatchMixSource {
        source_id: "source".into(),
        session_id: 77,
        gain_db: Some(-9.),
        muted: None,
        solo: None,
    });
    f.app.airplay_operation(Airplay::PatchMixSource {
        source_id: "source".into(),
        session_id: 77,
        gain_db: None,
        muted: None,
        solo: Some(true),
    });
    let mut state = f.app.airplay.clone().unwrap();
    state["revision"] = serde_json::json!(8);
    state["sessions"][0]["mix"]["muted"] = serde_json::json!(true);
    f.results
        .send(Ok(Outcome::Action(Box::new(first), state.clone())))
        .unwrap();
    f.app.process();
    f.refresh();
    let gain = f.next();
    let Request::Intent {
        command: IntentCommand::Airplay(command),
        ..
    } = &gain
    else {
        panic!()
    };
    assert!(matches!(
        command.operation,
        Airplay::PatchMixSource {
            gain_db: Some(-9.),
            muted: None,
            solo: None,
            ..
        }
    ));
    state["revision"] = serde_json::json!(9);
    state["sessions"][0]["mix"]["gain_db"] = serde_json::json!(-9.);
    f.results
        .send(Ok(Outcome::Action(Box::new(gain), state.clone())))
        .unwrap();
    f.app.process();
    f.refresh();
    let solo = f.next();
    let Request::Intent {
        command: IntentCommand::Airplay(command),
        ..
    } = solo
    else {
        panic!()
    };
    assert!(matches!(
        command.operation,
        Airplay::PatchMixSource {
            gain_db: None,
            muted: None,
            solo: Some(true),
            ..
        }
    ));
}
#[test]
fn legacy_resolution_freezes_one_fresh_payload_and_is_cancelled_on_context_change() {
    for switch in [false, true] {
        let mut f = Fixture::new();
        airplay(&mut f, false);
        f.app.airplay_operation(Airplay::PatchMixSource {
            source_id: "source".into(),
            session_id: 77,
            gain_db: Some(-9.),
            muted: None,
            solo: None,
        });
        let prepare = f.next();
        let Request::ResolveAirplayPatch {
            command: pending, ..
        } = &prepare
        else {
            panic!("missing read-only resolver");
        };
        let resolved = AirplayCommandV2 {
            command_version: 2,
            expected_config_revision: None,
            expected_event_sequence: None,
            command_id: pending.command_id.clone(),
            runtime_epoch: pending.runtime_epoch.clone(),
            credential_id: pending.credential_id.clone(),
            expected_revision: Some(11),
            operation: Airplay::MixSource {
                source_id: "source".into(),
                session_id: 77,
                gain_db: -9.,
                muted: true,
                solo: true,
            },
        };
        if switch {
            f.app.status.as_mut().unwrap().profiles[0].device_id =
                Some(f.authority.current().streams[&f.streams[0]].device_id);
            f.app.sync_command_context();
        }
        f.results
            .send(Ok(Outcome::Action(
                Box::new(prepare),
                serde_json::json!({"command":resolved,
            "before":{"gain_db":-6.,"muted":true,"solo":true}}),
            )))
            .unwrap();
        f.app.process();
        if switch {
            assert!(f.work.try_recv().is_err());
            assert!(f.app.intents.flight.is_none());
            continue;
        }
        let mutation = f.next();
        let bytes = serde_json::to_vec(&mutation).unwrap();
        let Request::Intent {
            command: IntentCommand::Airplay(command),
            ..
        } = &mutation
        else {
            panic!()
        };
        assert_eq!(command.expected_revision, Some(11));
        assert!(matches!(
            command.operation,
            Airplay::MixSource {
                gain_db: -9.,
                muted: true,
                solo: true,
                ..
            }
        ));
        f.results
            .send(Err(UiError::from("background_timeout")))
            .unwrap();
        f.app.process();
        f.refresh();
        f.app.reconcile_intent();
        assert_eq!(
            serde_json::to_vec(&f.next()).unwrap(),
            bytes,
            "reconciliation must not resolve a new triple"
        );
    }
}
#[test]
fn undo_is_invalidated_in_acknowledgement_and_restore_windows_and_on_session_reuse() {
    for window in [0, 1, 2, 3] {
        let mut f = Fixture::new();
        let key = f.streams[0];
        f.app.lane_gain(
            &f.app
                .lanes(f.authority.current())
                .into_iter()
                .find(|l| l.key == key)
                .unwrap(),
            -6.,
        );
        let request = f.next();
        if window >= 1 {
            f.execute(&request);
            f.acknowledge(request.clone());
        }
        if window >= 2 {
            f.refresh();
            assert!(f.app.undo_available());
        }
        if window == 3 {
            f.app.busy = true;
            f.app.polling = true;
            f.app.restore_last();
            assert_eq!(f.app.intents.queued.len(), 1);
        }
        let mut snapshot = f.authority.snapshot();
        snapshot.runtime_epoch = uuid::Uuid::new_v4();
        f.app.snapshot = Some(snapshot);
        f.app.sync_command_context();
        assert!(f.app.undo.is_none());
        assert!(f.app.intents.queued.is_empty());
        if window == 0 {
            f.execute(&request);
            f.acknowledge(request);
            assert!(f.app.undo.is_none());
        }
    }
    let mut f = Fixture::new();
    f.app.master_mute(true);
    let request = f.next();
    let key = f.streams[1];
    f.mix(key, Some(-6.), None, None);
    let stream = f
        .app
        .snapshot
        .as_mut()
        .unwrap()
        .streams
        .get_mut(&key)
        .unwrap();
    stream.session_id = uuid::Uuid::new_v4();
    f.execute(&request);
    f.acknowledge(request);
    f.app.fresh = Some(Instant::now());
    f.app.synced = f.app.fresh;
    f.app.process();
    assert!(f.work.try_recv().is_err());
    assert_eq!(
        f.app.intents.history.back().unwrap().phase,
        Phase::Cancelled
    );
}

#[test]
fn acknowledgement_does_not_erase_a_newer_drag_draft_and_removed_session_clears_its_draft() {
    let mut f = Fixture::new();
    let key = f.streams[0];
    f.app.gain_drafts.insert(key, -3.);
    f.app.capture_gain_draft(key);
    f.mix(key, Some(-3.), None, None);
    let first = f.next();
    f.app.gain_drafts.insert(key, -6.);
    f.app.capture_gain_draft(key);
    f.execute(&first);
    f.acknowledge(first);
    assert_eq!(f.app.gain_drafts.get(&key), Some(&-6.));
    let device_id = f.authority.current().streams[&key].device_id;
    let command = Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        request_id: uuid::Uuid::new_v4(),
        runtime_epoch: None,
        credential_id: None,
        expected_revision: Some(f.authority.current().revision),
        operation: Operation::Disconnect { device_id },
    };
    f.authority
        .execute(
            f.authority.authenticate(ADMIN).unwrap(),
            command,
            |_| Ok(()),
        )
        .unwrap();
    f.refresh();
    assert!(!f.app.gain_drafts.contains_key(&key));
}

#[test]
fn airplay_v3_confirmation_retains_original_versions_and_command_id_after_refresh() {
    let mut f = Fixture::new();
    f.app.airplay = Some(
        serde_json::json!({"command_version":3,"config_revision":7,"event_sequence":19,"revision":19,
        "sources":[{"source_id":"source","blocked":false,"revoked":false,"gain_db":0.,"muted":false}],"sessions":[]}),
    );
    let request = f
        .app
        .airplay_request(Airplay::RevokeSource {
            source_id: "source".into(),
            session_id: None,
        })
        .unwrap();
    let Request::AirplayV2 {
        command: Some(original),
        ..
    } = &request
    else {
        panic!()
    };
    assert_eq!(original.command_version, 3);
    assert_eq!(original.expected_config_revision, Some(7));
    assert_eq!(original.expected_event_sequence, Some(19));
    let frozen = serde_json::to_value(original).unwrap();
    let intent = f.app.confirmation_intent(&request).unwrap();
    let snapshot = f.app.airplay.as_mut().unwrap();
    snapshot["config_revision"] = serde_json::json!(8);
    snapshot["event_sequence"] = serde_json::json!(20);
    snapshot["revision"] = serde_json::json!(20);
    f.app.enqueue_bound(intent);
    let Request::Intent {
        command: IntentCommand::Airplay(command),
        ..
    } = f.next()
    else {
        panic!()
    };
    assert_eq!(
        serde_json::to_value(command).unwrap(),
        frozen,
        "confirmation cannot rebase a discrete trust mutation"
    );
}
#[test]
fn airplay_v3_regular_mix_uses_config_and_runtime_conditions_while_solo_uses_runtime_only() {
    let mut f = Fixture::new();
    f.app.airplay = Some(
        serde_json::json!({"command_version":3,"config_revision":4,"event_sequence":8,"revision":8,
        "capabilities":["patch_mix_source","runtime_epoch","version_domains"],
        "sources":[{"source_id":"source","gain_db":0.,"muted":false}],
        "sessions":[{"source_id":"source","session_id":7,"stream_epoch":3,"mix":{"gain_db":0.,"muted":false,"solo":false}}]}),
    );
    let context = f.app.intents.current.as_ref().unwrap();
    let gain = f
        .app
        .bind_airplay(
            uuid::Uuid::new_v4(),
            context,
            Airplay::PatchMixSource {
                source_id: "source".into(),
                session_id: 7,
                gain_db: Some(-3.),
                muted: None,
                solo: None,
            },
        )
        .unwrap();
    assert_eq!(gain.command_version, 3);
    assert_eq!(gain.expected_config_revision, Some(4));
    assert_eq!(gain.expected_event_sequence, Some(8));
    assert_eq!(gain.expected_revision, None);
    let solo = f
        .app
        .bind_airplay(
            uuid::Uuid::new_v4(),
            context,
            Airplay::PatchMixSource {
                source_id: "source".into(),
                session_id: 7,
                gain_db: None,
                muted: None,
                solo: Some(true),
            },
        )
        .unwrap();
    assert_eq!(solo.expected_config_revision, None);
    assert_eq!(solo.expected_event_sequence, Some(8));
}

#[test]
fn saved_field_waits_for_scoped_callback_ack_and_exposes_stall_or_unknown_without_resending() {
    let mut f = Fixture::new();
    f.app.operation(Operation::OutputMix {
        gain_db: Some(-6.),
        muted: None,
    });
    let request = f.next();
    let Request::Intent {
        command: IntentCommand::Native(command),
        ..
    } = &request
    else {
        panic!()
    };
    let receipt = f
        .authority
        .execute(
            f.authority.authenticate(ADMIN).unwrap(),
            command.clone(),
            |_| Ok(()),
        )
        .unwrap();
    let epoch = f.authority.current().runtime_epoch;
    f.results.send(Ok(Outcome::Action(Box::new(request),serde_json::json!({"receipt":receipt,"media_pending":true,
        "media_application":{"runtime_epoch":epoch,"desired_config_sequence":12,"applied_config_sequence":11,"pending":true,"stalled":false}})))).unwrap();
    f.app.process();
    assert_eq!(f.app.command_status(0), Some(Message::ShellMediaPending));
    f.app.snapshot = Some(f.authority.snapshot());
    let mut progress = serde_json::json!({"runtime_epoch":epoch,"desired_config_sequence":12,"applied_config_sequence":11,"pending":true,"stalled":true});
    f.app.refresh_applications(Some(&progress));
    assert_eq!(f.app.command_status(0), Some(Message::ShellMediaStalled));
    f.app.diagnostics = Some(
        serde_json::json!({"media_application":{"runtime_epoch":epoch,"applied_config_sequence":99}}),
    );
    f.app.refresh_applications(None);
    assert_eq!(f.app.command_status(0), Some(Message::ShellMediaUnknown));
    assert!(
        f.work.try_recv().is_err(),
        "application observation cannot resend a saved mutation"
    );
    progress["applied_config_sequence"] = serde_json::json!(12);
    f.app.refresh_applications(Some(&progress));
    assert!(f.app.applications.is_empty());
    assert_eq!(f.app.message, Message::ShellMediaApplied);
}
#[test]
fn application_ack_from_another_runtime_cannot_settle_saved_field_and_room_switch_clears_watch() {
    let mut f = Fixture::new();
    f.app.operation(Operation::OutputMix {
        gain_db: Some(-3.),
        muted: None,
    });
    let request = f.next();
    let Request::Intent {
        command: IntentCommand::Native(command),
        ..
    } = &request
    else {
        panic!()
    };
    let receipt = f
        .authority
        .execute(
            f.authority.authenticate(ADMIN).unwrap(),
            command.clone(),
            |_| Ok(()),
        )
        .unwrap();
    let epoch = f.authority.current().runtime_epoch;
    f.results.send(Ok(Outcome::Action(Box::new(request),serde_json::json!({"receipt":receipt,"media_pending":true,
        "media_application":{"runtime_epoch":epoch,"desired_config_sequence":4,"applied_config_sequence":3,"pending":true,"stalled":false}})))).unwrap();
    f.app.process();
    f.app.snapshot = Some(f.authority.snapshot());
    f.app.refresh_applications(Some(
        &serde_json::json!({"runtime_epoch":uuid::Uuid::new_v4(),"applied_config_sequence":99}),
    ));
    assert_eq!(f.app.command_status(0), Some(Message::ShellMediaUnknown));
    f.app.snapshot.as_mut().unwrap().runtime_epoch = uuid::Uuid::new_v4();
    f.app.sync_command_context();
    assert!(f.app.applications.is_empty());
}
