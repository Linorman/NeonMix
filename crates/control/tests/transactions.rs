use neonmix_control::*;
use uuid::Uuid;
const TOKEN: &str = "transaction-admin-credential-32-bytes-minimum";
fn command(a: &Authority, gain: f32) -> Command {
    Command {
        request_id: Uuid::new_v4(),
        expected_revision: a.current().revision,
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
    let prepared = a.prepare_transaction(principal, command.clone()).unwrap();
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
    let events = a.events_after(receipt.revision).unwrap();
    assert!(
        !events.is_empty(),
        "deferred health must still publish an event"
    );
}
#[test]
fn failed_preparation_aborts_only_the_candidate_and_keeps_health_updates() {
    let mut a = Authority::new("test".into(), "admin".into(), TOKEN).unwrap();
    let principal = a.authenticate(TOKEN).unwrap();
    let prepared = a.prepare_transaction(principal, command(&a, -3.)).unwrap();
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
