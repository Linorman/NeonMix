use neonmix_control::{Authority, Role};
use neonmix_credential_migrate::{
    Error, Hooks, Layout, LegacyReader, NoHooks, Options, Phase, ProfileKind, cleanup_staging,
    migrate,
};
use neonmix_identity::{
    files,
    store::{FileCredentialStore, SecretKind},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    destination: PathBuf,
    values: BTreeMap<String, String>,
}
impl Fixture {
    fn new() -> Self {
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let root = project
            .join(".local/tmp")
            .join(format!("credential-migration-{}", Uuid::new_v4()));
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        files::private_dir(&root).unwrap();
        let source = root.join("source");
        files::private_dir(&source).unwrap();
        Self {
            destination: root.join("destination"),
            root,
            source,
            values: BTreeMap::new(),
        }
    }
    fn options(&self, layout: Layout) -> Options {
        Options {
            source: self.source.clone(),
            destination: self.destination.clone(),
            layout,
            profile_kind: if layout == Layout::Profiles {
                Some(ProfileKind::Member)
            } else {
                None
            },
        }
    }
    fn write(&self, relative: &str, value: &Value) {
        let path = self.source.join(relative);
        let mut directory = self.source.clone();
        for part in Path::new(relative).parent().unwrap().components() {
            directory.push(part);
            files::private_dir(&directory).unwrap();
        }
        files::write_new(&path, &serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
    fn put(&mut self, value: String) -> String {
        let reference = Uuid::new_v4().to_string();
        self.values.insert(reference.clone(), value);
        reference
    }
    fn profiles(&mut self, pending: bool) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let reference = self.put(neonmix_identity::secret());
        self.write("sender.json", &json!({"version":1,"hub_id":Uuid::new_v4(),"certificate":cert.cert.pem(),"secret_ref":reference,"request_id":Uuid::new_v4(),"name":"sender","pending":pending,"invitation_id": if pending { Some(Uuid::new_v4()) } else { None },"device_id":if pending { None } else { Some(Uuid::new_v4()) }}));
    }
    fn desktop(&mut self) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let admin = neonmix_identity::secret();
        let member = neonmix_identity::secret();
        let mut authority =
            Authority::new("coreaudio:fixture".into(), "admin".into(), &admin).unwrap();
        let admin_id = authority.authenticate(&admin).unwrap().device_id();
        let member_id = authority
            .add_device("revoked-member".into(), Role::Member, &member)
            .unwrap();
        let mut saved = serde_json::to_value(authority.persistent()).unwrap();
        saved["devices"][member_id.to_string()]["revoked"] = json!(true);
        saved["revision"] = json!(91);
        let tls_reference = self.put(cert.signing_key.serialize_pem());
        let admin_reference = self.put(admin);
        let member_reference = self.put(member);
        self.write("hub/state.json", &saved);
        self.write("hub/server.json", &json!({"version":1,"room_name":"Room","state_path":self.source.join("hub/state.json"),"output":"coreaudio:fixture","certificate":cert.cert.pem(),"private_key_ref":tls_reference,"admin_token_ref":admin_reference}));
        let token = |reference: &str, device: Option<Uuid>, pending: bool| json!({"version":1,"hub_id":authority.current().hub_id,"certificate":cert.cert.pem(),"secret_ref":reference,"request_id":Uuid::new_v4(),"name":"not-an-admin-name","pending":pending,"invitation_id":if pending { Some(Uuid::new_v4()) } else {None},"device_id":device});
        self.write(
            "hub/admin.json",
            &token(&admin_reference, Some(admin_id), false),
        );
        self.write(
            "profiles/admin-alias.json",
            &token(&admin_reference, Some(admin_id), false),
        );
        self.write(
            "profiles/revoked.json",
            &token(&member_reference, Some(member_id), false),
        );
        self.write(
            "profiles/shared-member.json",
            &token(&member_reference, Some(member_id), false),
        );
        let pending_reference = self.put(neonmix_identity::secret());
        self.write(
            "profiles/pending.json",
            &token(&pending_reference, None, true),
        );
        let airplay = self.put(
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
                .unwrap()
                .serialize_pem(),
        );
        self.write("hub/airplay/receiver.json", &json!({"key_reference":airplay,"known_keys":["AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="],"blocked_keys":["AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI="],"playback_allowed":false,"playback_mode":"synchronized"}));
        self.write("outputs/main/binding.json", &json!({"schema_version":1,"revision":8,"output_id":Uuid::new_v4(),"hub_id":authority.current().hub_id,"provider":"neonmix","device_id":"coreaudio:fixture","display_name":"Fixture output","enabled":false,"authorization_epoch":17}));
        files::private_dir(&self.source.join("bin")).unwrap();
        files::write_new(
            &self.source.join("bin/neonmix-hub"),
            b"old executable fixture",
        )
        .unwrap();
        files::write_new(
            &self.source.join("hub/airplay/gstreamer-registry.bin"),
            b"runtime registry fixture",
        )
        .unwrap();
        files::write_new(
            &self.source.join("hub/airplay/runtime-key-old"),
            b"excluded ephemeral fixture",
        )
        .unwrap();
        files::private_dir(&self.source.join("commands")).unwrap();
        files::write_new(
            &self.source.join("commands/invite-fixture.json"),
            b"excluded invitation",
        )
        .unwrap();
        files::private_dir(&self.source.join("invitations")).unwrap();
        files::write_new(
            &self.source.join("invitations/invite.json"),
            b"excluded invitation",
        )
        .unwrap();
        for name in ["old.pid", "old.lock", "old.log", "stale.sock"] {
            files::write_new(&self.source.join(name), b"excluded runtime").unwrap();
        }
    }
    fn reader(&self) -> Mock {
        Mock {
            values: self.values.clone(),
            reads: 0,
            fail: None,
        }
    }
    fn assert_clean(&self) {
        assert!(!self.destination.exists());
        assert!(fs::read_dir(&self.root).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".staging")
        }));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Mock {
    values: BTreeMap<String, String>,
    reads: usize,
    fail: Option<Error>,
}
impl LegacyReader for Mock {
    fn read(&mut self, reference: &str) -> neonmix_credential_migrate::Result<String> {
        self.reads += 1;
        if let Some(error) = self.fail {
            return Err(error);
        }
        self.values
            .get(reference)
            .cloned()
            .ok_or(Error("migration_native_missing"))
    }
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&files::read_private(path, 1024 * 1024).unwrap()).unwrap()
}

#[test]
fn complete_desktop_preserves_identity_revocation_pending_aliases_and_binding() {
    let mut fixture = Fixture::new();
    fixture.desktop();
    let mut reader = fixture.reader();
    let report = migrate(&fixture.options(Layout::Desktop), &mut reader, &mut NoHooks).unwrap();
    assert!(report.identity_preserved && report.source_unchanged && report.native_entries_retained);
    assert_eq!(reader.reads, fixture.values.len());
    for relative in ["hub/state.json", "outputs/main/binding.json"] {
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            fs::read(fixture.destination.join(relative)).unwrap()
        );
    }
    for relative in [
        "hub/admin.json",
        "profiles/admin-alias.json",
        "profiles/revoked.json",
        "profiles/shared-member.json",
        "profiles/pending.json",
        "hub/airplay/receiver.json",
    ] {
        let old = read(&fixture.source.join(relative));
        let new = read(&fixture.destination.join(relative));
        for (field, value) in old.as_object().unwrap() {
            if field != "version" {
                assert_eq!(&new[field], value, "public field {field} changed");
            }
        }
    }
    assert_eq!(
        read(&fixture.destination.join("hub/server.json"))["state_path"],
        "state.json"
    );
    assert_eq!(
        read(&fixture.destination.join("profiles/admin-alias.json"))["profile_kind"],
        "admin"
    );
    assert_eq!(
        read(&fixture.destination.join("profiles/revoked.json"))["profile_kind"],
        "member"
    );
    let alias = read(&fixture.destination.join("profiles/admin-alias.json"));
    let store =
        FileCredentialStore::for_profile(&fixture.destination.join("profiles/admin-alias.json"))
            .unwrap();
    assert!(
        store
            .get(
                alias["secret_ref"].as_str().unwrap(),
                SecretKind::MemberToken
            )
            .is_err()
    );
    assert!(!fixture.destination.join("commands").exists());
    assert!(!fixture.destination.join("invitations").exists());
    assert!(!fixture.destination.join("bin").exists());
    assert!(
        !fixture
            .destination
            .join("hub/airplay/gstreamer-registry.bin")
            .exists()
    );
    for name in ["old.pid", "old.lock", "old.log", "stale.sock"] {
        assert!(!fixture.destination.join(name).exists());
    }
    assert!(
        !fixture
            .destination
            .join("hub/airplay/runtime-key-old")
            .exists()
    );
    assert_eq!(
        migrate(&fixture.options(Layout::Desktop), &mut reader, &mut NoHooks)
            .err()
            .unwrap()
            .0,
        "migration_destination_exists"
    );
}
#[test]
fn hub_layout_and_receiver_default_mode_migrate() {
    let mut fixture = Fixture::new();
    fixture.desktop();
    let mut receiver = read(&fixture.source.join("hub/airplay/receiver.json"));
    receiver.as_object_mut().unwrap().remove("playback_mode");
    files::replace(
        &fixture.source.join("hub/airplay/receiver.json"),
        &serde_json::to_vec(&receiver).unwrap(),
    )
    .unwrap();
    let mut options = fixture.options(Layout::Hub);
    options.source = fixture.source.join("hub");
    migrate(&options, &mut fixture.reader(), &mut NoHooks).unwrap();
    assert_eq!(
        read(&fixture.destination.join("airplay/receiver.json"))["playback_mode"],
        "low_latency"
    );
}
#[test]
fn independent_sender_requires_explicit_kind_and_preserves_pending() {
    let mut fixture = Fixture::new();
    fixture.profiles(true);
    let mut options = fixture.options(Layout::Profiles);
    options.profile_kind = None;
    assert_eq!(
        migrate(&options, &mut fixture.reader(), &mut NoHooks)
            .err()
            .unwrap()
            .0,
        "migration_profile_kind_required"
    );
    options.profile_kind = Some(ProfileKind::Member);
    migrate(&options, &mut fixture.reader(), &mut NoHooks).unwrap();
    let new = read(&fixture.destination.join("sender.json"));
    let old = read(&fixture.source.join("sender.json"));
    assert_eq!(new["request_id"], old["request_id"]);
    assert_eq!(new["invitation_id"], old["invitation_id"]);
    assert_eq!(new["pending"], true);
}
#[test]
fn sender_filenames_do_not_select_hub_receiver_or_binding_schema() {
    for name in ["receiver.json", "binding.json", "server.json"] {
        let mut fixture = Fixture::new();
        fixture.profiles(false);
        fs::rename(
            fixture.source.join("sender.json"),
            fixture.source.join(name),
        )
        .unwrap();
        migrate(
            &fixture.options(Layout::Profiles),
            &mut fixture.reader(),
            &mut NoHooks,
        )
        .unwrap();
        assert_eq!(
            read(&fixture.destination.join(name))["profile_kind"],
            "member"
        );
    }
}
#[test]
fn standalone_admin_and_hub_without_airplay_migrate() {
    let mut fixture = Fixture::new();
    fixture.profiles(false);
    let mut options = fixture.options(Layout::Profiles);
    options.profile_kind = Some(ProfileKind::Admin);
    migrate(&options, &mut fixture.reader(), &mut NoHooks).unwrap();
    assert_eq!(
        read(&fixture.destination.join("sender.json"))["profile_kind"],
        "admin"
    );
    let mut fixture = Fixture::new();
    fixture.desktop();
    fs::remove_dir_all(fixture.source.join("hub/airplay")).unwrap();
    let mut options = fixture.options(Layout::Hub);
    options.source = fixture.source.join("hub");
    migrate(&options, &mut fixture.reader(), &mut NoHooks).unwrap();
    assert!(!fixture.destination.join("airplay").exists());
}
#[test]
fn hub_requires_its_shared_admin_profile_and_matching_output() {
    for fault in ["missing-admin", "wrong-admin-ref", "wrong-output"] {
        let mut fixture = Fixture::new();
        fixture.desktop();
        match fault {
            "missing-admin" => fs::remove_file(fixture.source.join("hub/admin.json")).unwrap(),
            "wrong-admin-ref" => {
                let mut admin = read(&fixture.source.join("hub/admin.json"));
                admin["secret_ref"] = json!(Uuid::new_v4().to_string());
                files::replace(
                    &fixture.source.join("hub/admin.json"),
                    &serde_json::to_vec(&admin).unwrap(),
                )
                .unwrap();
            }
            "wrong-output" => {
                let mut server = read(&fixture.source.join("hub/server.json"));
                server["output"] = json!("coreaudio:wrong");
                files::replace(
                    &fixture.source.join("hub/server.json"),
                    &serde_json::to_vec(&server).unwrap(),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            migrate(
                &fixture.options(Layout::Desktop),
                &mut fixture.reader(),
                &mut NoHooks
            )
            .is_err()
        );
        fixture.assert_clean();
    }
}
struct Fail(Phase);
impl Hooks for Fail {
    fn checkpoint(&mut self, phase: Phase) -> neonmix_credential_migrate::Result<()> {
        if phase == self.0 {
            Err(Error("migration_injected_failure"))
        } else {
            Ok(())
        }
    }
}
#[test]
fn every_failure_boundary_preserves_source_and_removes_own_stage() {
    for phase in [
        Phase::NativeRead,
        Phase::SecretWrite,
        Phase::ProfileWrite,
        Phase::Validate,
        Phase::Publish,
    ] {
        let mut fixture = Fixture::new();
        fixture.profiles(true);
        let before = fs::read(fixture.source.join("sender.json")).unwrap();
        assert!(
            migrate(
                &fixture.options(Layout::Profiles),
                &mut fixture.reader(),
                &mut Fail(phase)
            )
            .is_err()
        );
        assert_eq!(
            fs::read(fixture.source.join("sender.json")).unwrap(),
            before
        );
        fixture.assert_clean();
    }
}
#[test]
fn missing_and_timed_out_native_values_preserve_source_and_do_not_publish() {
    for error in [
        Error("migration_native_missing"),
        Error("migration_native_timeout"),
    ] {
        let mut fixture = Fixture::new();
        fixture.profiles(false);
        let mut reader = fixture.reader();
        reader.fail = Some(error);
        assert_eq!(
            migrate(
                &fixture.options(Layout::Profiles),
                &mut reader,
                &mut NoHooks
            )
            .err()
            .unwrap(),
            error
        );
        fixture.assert_clean();
    }
}
struct Mutation {
    source: PathBuf,
    destination: PathBuf,
    target_race: bool,
}
impl Hooks for Mutation {
    fn checkpoint(&mut self, phase: Phase) -> neonmix_credential_migrate::Result<()> {
        if phase == Phase::Publish {
            if self.target_race {
                fs::create_dir(&self.destination).unwrap();
                files::write_new(&self.destination.join("owned-by-other"), b"winner").unwrap();
            } else {
                let mut value = read(&self.source.join("sender.json"));
                value["name"] = json!("concurrent-change");
                files::replace(
                    &self.source.join("sender.json"),
                    &serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
            }
        }
        Ok(())
    }
}
#[test]
fn source_changes_and_target_races_are_detected_before_publication() {
    for target_race in [false, true] {
        let mut fixture = Fixture::new();
        fixture.profiles(false);
        let mut hooks = Mutation {
            source: fixture.source.clone(),
            destination: fixture.destination.clone(),
            target_race,
        };
        let error = migrate(
            &fixture.options(Layout::Profiles),
            &mut fixture.reader(),
            &mut hooks,
        )
        .err()
        .unwrap();
        if target_race {
            assert_eq!(error.0, "migration_destination_exists");
            assert_eq!(
                fs::read(fixture.destination.join("owned-by-other")).unwrap(),
                b"winner"
            );
        } else {
            assert_eq!(error.0, "migration_source_changed");
            fixture.assert_clean();
        }
    }
}
#[test]
fn rejects_unsupported_files_external_state_bad_tls_admin_and_airplay_keys() {
    for fault in ["unknown", "backup", "external", "tls", "admin", "airplay"] {
        let mut fixture = Fixture::new();
        fixture.desktop();
        match fault {
            "unknown" => files::write_new(&fixture.source.join("unknown.json"), b"{}").unwrap(),
            "backup" => files::write_new(
                &fixture.source.join("state.json.bak"),
                b"unsupported backup",
            )
            .unwrap(),
            "external" => {
                let mut server = read(&fixture.source.join("hub/server.json"));
                files::write_new(
                    &fixture.root.join("external-state.json"),
                    &fs::read(fixture.source.join("hub/state.json")).unwrap(),
                )
                .unwrap();
                server["state_path"] = json!(fixture.root.join("external-state.json"));
                files::replace(
                    &fixture.source.join("hub/server.json"),
                    &serde_json::to_vec(&server).unwrap(),
                )
                .unwrap();
            }
            "tls" => {
                let reference = read(&fixture.source.join("hub/server.json"))["private_key_ref"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                fixture.values.insert(
                    reference,
                    rcgen::KeyPair::generate().unwrap().serialize_pem(),
                );
            }
            "admin" => {
                let reference = read(&fixture.source.join("hub/server.json"))["admin_token_ref"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                fixture.values.insert(reference, neonmix_identity::secret());
            }
            "airplay" => {
                let reference =
                    read(&fixture.source.join("hub/airplay/receiver.json"))["key_reference"]
                        .as_str()
                        .unwrap()
                        .to_owned();
                fixture.values.insert(
                    reference,
                    rcgen::KeyPair::generate().unwrap().serialize_pem(),
                );
            }
            _ => unreachable!(),
        }
        assert!(
            migrate(
                &fixture.options(Layout::Desktop),
                &mut fixture.reader(),
                &mut NoHooks
            )
            .is_err(),
            "accepted {fault}"
        );
        fixture.assert_clean();
    }
}
#[test]
fn overlapping_paths_and_live_locks_are_rejected() {
    let mut fixture = Fixture::new();
    fixture.desktop();
    let mut options = fixture.options(Layout::Desktop);
    options.destination = fixture.source.join("nested");
    assert_eq!(
        migrate(&options, &mut fixture.reader(), &mut NoHooks)
            .err()
            .unwrap()
            .0,
        "migration_overlapping_paths"
    );
    let _guard = files::lock(&fixture.source.join("background.lock")).unwrap();
    assert_eq!(
        migrate(
            &fixture.options(Layout::Desktop),
            &mut fixture.reader(),
            &mut NoHooks
        )
        .err()
        .unwrap()
        .0,
        "migration_source_busy"
    );
    fixture.assert_clean();
}
#[cfg(unix)]
#[test]
fn broad_permissions_and_symlinks_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for link in [false, true] {
        let mut fixture = Fixture::new();
        fixture.profiles(false);
        if link {
            fs::rename(
                fixture.source.join("sender.json"),
                fixture.root.join("original.json"),
            )
            .unwrap();
            symlink(
                fixture.root.join("original.json"),
                fixture.source.join("sender.json"),
            )
            .unwrap();
        } else {
            fs::set_permissions(
                fixture.source.join("sender.json"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        assert!(
            migrate(
                &fixture.options(Layout::Profiles),
                &mut fixture.reader(),
                &mut NoHooks
            )
            .is_err()
        );
        fixture.assert_clean();
    }
}
#[cfg(unix)]
#[test]
fn active_ipc_socket_rejects_offline_migration() {
    let mut fixture = Fixture::new();
    // macOS sockaddr_un is shorter than the ordinary descriptive fixture path.
    let short = fixture
        .root
        .with_file_name(format!("cm-{}", Uuid::new_v4()));
    fs::rename(&fixture.root, &short).unwrap();
    fixture.root = short;
    fixture.source = fixture.root.join("source");
    fixture.destination = fixture.root.join("destination");
    fixture.desktop();
    let _listener =
        std::os::unix::net::UnixListener::bind(fixture.source.join("ipc.sock")).unwrap();
    assert_eq!(
        migrate(
            &fixture.options(Layout::Desktop),
            &mut fixture.reader(),
            &mut NoHooks
        )
        .err()
        .unwrap()
        .0,
        "migration_source_busy"
    );
    fixture.assert_clean();
}
#[test]
fn cleanup_refuses_unowned_directories() {
    let fixture = Fixture::new();
    assert!(cleanup_staging(&fixture.source).is_err());
    assert!(fixture.source.exists());
}
struct Crash(usize);
impl Hooks for Crash {
    fn checkpoint(&mut self, phase: Phase) -> neonmix_credential_migrate::Result<()> {
        let index = match phase {
            Phase::NativeRead => 0,
            Phase::SecretWrite => 1,
            Phase::ProfileWrite => 2,
            Phase::Validate => 3,
            Phase::Publish => 4,
        };
        if index == self.0 {
            std::process::exit(77);
        }
        Ok(())
    }
}
#[test]
fn crash_at_boundary_child() {
    let Ok(root) = std::env::var("NEONMIX_MIGRATION_CRASH_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    let values: BTreeMap<String, String> = serde_json::from_slice(
        &files::read_private(&root.join("mock-values.json"), 1024 * 1024).unwrap(),
    )
    .unwrap();
    let phase: usize = std::env::var("NEONMIX_MIGRATION_CRASH_PHASE")
        .unwrap()
        .parse()
        .unwrap();
    let options = Options {
        source: root.join("source"),
        destination: root.join("destination"),
        layout: Layout::Profiles,
        profile_kind: Some(ProfileKind::Member),
    };
    let _ = migrate(
        &options,
        &mut Mock {
            values,
            reads: 0,
            fail: None,
        },
        &mut Crash(phase),
    );
    panic!("crash checkpoint not reached");
}
#[test]
fn forced_process_exit_leaves_only_explicitly_recoverable_staging() {
    for phase in 0..5 {
        let mut fixture = Fixture::new();
        fixture.profiles(true);
        let before = fs::read(fixture.source.join("sender.json")).unwrap();
        files::write_new(
            &fixture.root.join("mock-values.json"),
            &serde_json::to_vec(&fixture.values).unwrap(),
        )
        .unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_at_boundary_child", "--nocapture"])
            .env("NEONMIX_MIGRATION_CRASH_FIXTURE", &fixture.root)
            .env("NEONMIX_MIGRATION_CRASH_PHASE", phase.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77));
        assert!(!fixture.destination.exists());
        assert_eq!(
            fs::read(fixture.source.join("sender.json")).unwrap(),
            before
        );
        let stages: Vec<_> = fs::read_dir(&fixture.root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "staging"))
            .collect();
        assert_eq!(stages.len(), 1);
        cleanup_staging(&stages[0]).unwrap();
        fixture.assert_clean();
        migrate(
            &fixture.options(Layout::Profiles),
            &mut fixture.reader(),
            &mut NoHooks,
        )
        .unwrap();
    }
}
