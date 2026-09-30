use neonmix_output_binding::{Error, Provider, Store};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};
use uuid::Uuid;

struct Fixture {
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self {
            directory: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/tmp")
                .join(format!("e06-binding-{}", Uuid::new_v4())),
        }
    }
    fn store(&self) -> Store {
        Store::new(&self.directory)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn reopen_rename_and_repeated_add_preserve_identity() {
    let f = Fixture::new();
    let store = f.store();
    let hub = Uuid::new_v4();
    let first = store
        .add(
            hub,
            Provider::Blackhole,
            "coreaudio:BlackHole2ch_UID".into(),
            "NeonMix — 客厅".into(),
        )
        .unwrap();
    let reopened = f.store().load().unwrap();
    assert_eq!(reopened, first);
    let renamed = store
        .rename(first.revision, "NeonMix — 书房".into())
        .unwrap();
    assert_eq!(renamed.output_id, first.output_id);
    assert_eq!(renamed.hub_id, hub);
    assert_eq!(renamed.device_id, first.device_id);
    assert!(renamed.permits(&first));
    assert_eq!(
        store
            .add(
                hub,
                Provider::Blackhole,
                first.device_id.clone(),
                "another name".into()
            )
            .unwrap(),
        renamed
    );
    assert!(matches!(
        store.add(
            Uuid::new_v4(),
            Provider::Blackhole,
            first.device_id,
            "wrong Hub".into()
        ),
        Err(Error::AlreadyBound)
    ));
}

#[test]
fn disable_enable_and_recreation_cannot_reauthorize_an_old_sender() {
    let f = Fixture::new();
    let store = f.store();
    let original = store
        .add(
            Uuid::new_v4(),
            Provider::Blackhole,
            "coreaudio:BlackHole2ch_UID".into(),
            "Room".into(),
        )
        .unwrap();
    let disabled = store.set_enabled(original.revision, false).unwrap();
    assert!(!disabled.permits(&original));
    let enabled = store.set_enabled(disabled.revision, true).unwrap();
    assert!(!enabled.permits(&original));
    assert!(enabled.permits(&enabled));
    assert!(matches!(
        store.rename(original.revision, "stale write".into()),
        Err(Error::Conflict(_))
    ));
    store.remove(enabled.revision).unwrap();
    let replacement = store
        .add(
            original.hub_id,
            original.provider,
            original.device_id.clone(),
            "Replacement".into(),
        )
        .unwrap();
    assert_ne!(replacement.output_id, original.output_id);
    assert!(!replacement.permits(&original));
}

#[test]
fn concurrent_writers_use_revision_conflicts_and_never_publish_partial_json() {
    let f = Fixture::new();
    let store = f.store();
    let initial = store
        .add(
            Uuid::new_v4(),
            Provider::Blackhole,
            "coreaudio:BlackHole2ch_UID".into(),
            "Room".into(),
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|index| {
            let store = store.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                store.rename(1, format!("Room {index}"))
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(Error::Conflict(2))))
            .count(),
        1
    );
    let final_value = store.load().unwrap();
    assert_eq!(final_value.output_id, initial.output_id);
    assert_eq!(final_value.revision, 2);
    assert!(!fs::read_dir(&f.directory).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")
    }));
}

#[test]
fn corrupt_or_oversized_records_fail_closed_and_are_not_replaced() {
    let f = Fixture::new();
    let store = f.store();
    store
        .add(
            Uuid::new_v4(),
            Provider::Blackhole,
            "coreaudio:BlackHole2ch_UID".into(),
            "Room".into(),
        )
        .unwrap();
    fs::write(f.directory.join("binding.json"), vec![b' '; 20000]).unwrap();
    assert!(store.load().is_err());
    assert!(
        store
            .add(
                Uuid::new_v4(),
                Provider::Blackhole,
                "coreaudio:BlackHole2ch_UID".into(),
                "New".into()
            )
            .is_err()
    );
    assert_eq!(
        fs::metadata(f.directory.join("binding.json"))
            .unwrap()
            .len(),
        20000
    );
}

#[cfg(unix)]
#[test]
fn losing_private_permissions_or_replacing_the_record_with_a_symlink_fails_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let store = fixture.store();
    store
        .add(
            Uuid::new_v4(),
            Provider::Blackhole,
            "coreaudio:BlackHole2ch_UID".into(),
            "Room".into(),
        )
        .unwrap();
    let path = fixture.directory.join("binding.json");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load().is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&path, fixture.directory.join("original.json")).unwrap();
    symlink("original.json", &path).unwrap();
    assert!(store.load().is_err());
}
