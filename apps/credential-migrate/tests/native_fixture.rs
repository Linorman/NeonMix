//! Explicit opt-in only. These native entries belong solely to this generated fixture.
use neonmix_credential_migrate::{Error, Layout, LegacyReader, NoHooks, Options, Result, migrate};
use neonmix_identity::{
    files,
    store::{FileCredentialStore, SecretKind},
};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn action(operation: &str, reference: &str, value: Option<&str>) -> Result<String> {
    let mut child = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_neonmix-credential-migrate"))
            .arg(if operation == "read" {
                "--native-read-helper"
            } else {
                "--native-fixture-helper"
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error("native_fixture_helper_failed"))?,
    );
    let request = serde_json::to_vec(&json!({"reference":reference,"parent_id":std::process::id(),"operation":operation,"value":value})).map_err(|_| Error("native_fixture_helper_failed"))?;
    let mut input = child
        .0
        .stdin
        .take()
        .ok_or(Error("native_fixture_helper_failed"))?;
    input
        .write_all(&request)
        .map_err(|_| Error("native_fixture_helper_failed"))?;
    drop(input);
    let output = child
        .0
        .stdout
        .take()
        .ok_or(Error("native_fixture_helper_failed"))?;
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(64 * 1024 + 2)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    let outcome = loop {
        if start.elapsed() >= Duration::from_secs(30) {
            break Err(Error("native_fixture_timeout"));
        }
        match child.0.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) | Err(_) => break Err(Error("native_fixture_helper_failed")),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    };
    if outcome.is_err() {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    let bytes = drain
        .join()
        .map_err(|_| Error("native_fixture_helper_failed"))?
        .map_err(|_| Error("native_fixture_helper_failed"))?;
    outcome?;
    if bytes == [2] {
        return Err(Error("migration_native_missing"));
    }
    if operation != "read" && bytes == [3] {
        return Ok(String::new());
    }
    if operation != "read" || bytes.first() != Some(&1) || bytes.len() > 64 * 1024 + 1 {
        return Err(Error("native_fixture_helper_failed"));
    }
    String::from_utf8(bytes[1..].to_vec()).map_err(|_| Error("native_fixture_helper_failed"))
}
struct Native;
impl LegacyReader for Native {
    fn read(&mut self, reference: &str) -> Result<String> {
        action("read", reference, None)
    }
}
struct Fixture {
    root: PathBuf,
    entries: Vec<(String, String)>,
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
            .join(format!("credential-migration-native-{}", Uuid::new_v4()));
        fs::create_dir_all(root.parent().unwrap()).unwrap();
        files::private_dir(&root).unwrap();
        Self {
            root,
            entries: Vec::new(),
        }
    }
    fn put(&mut self, value: String) -> String {
        let reference = Uuid::new_v4().to_string();
        self.entries.push((reference.clone(), value.clone()));
        // Persist cleanup ownership before the helper can create a native entry.
        files::replace(
            &self.root.join("native-fixture-owner.json"),
            &serde_json::to_vec(&self.entries).unwrap(),
        )
        .unwrap();
        assert!(
            action("create_fixture", &reference, Some(&value)).is_ok(),
            "native fixture creation failed"
        );
        reference
    }
    fn cleanup(&mut self) -> bool {
        let mut success = true;
        for (reference, value) in &self.entries {
            success &= action("delete_fixture", reference, Some(value)).is_ok();
        }
        if success {
            self.entries.clear();
            let _ = fs::remove_dir_all(&self.root);
        }
        success
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.entries.is_empty() && !self.cleanup() {
            eprintln!(
                "native_fixture_cleanup_incomplete; private ownership journal retained inside project .local/tmp"
            );
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
#[ignore = "explicit native vault acceptance only; ordinary tests and --all-features never access native credentials"]
fn native_fixture_migration() {
    let mut fixture = Fixture::new();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let token = neonmix_identity::secret();
    let authority = neonmix_control::Authority::new(
        "coreaudio:native-fixture".into(),
        "native fixture admin".into(),
        &token,
    )
    .unwrap();
    let tls_ref = fixture.put(cert.signing_key.serialize_pem());
    let admin_ref = fixture.put(token.clone());
    let airplay_ref = fixture.put(
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .unwrap()
            .serialize_pem(),
    );
    let source = fixture.root.join("source");
    files::private_dir(&source).unwrap();
    files::private_dir(&source.join("airplay")).unwrap();
    let write = |relative: &str, value: serde_json::Value| {
        files::write_new(
            &source.join(relative),
            &serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap()
    };
    write(
        "state.json",
        serde_json::to_value(authority.persistent()).unwrap(),
    );
    write(
        "server.json",
        json!({"version":1,"room_name":"Native fixture","state_path":source.join("state.json"),"output":"coreaudio:native-fixture","certificate":cert.cert.pem(),"private_key_ref":tls_ref,"admin_token_ref":admin_ref}),
    );
    write(
        "admin.json",
        json!({"version":1,"hub_id":authority.current().hub_id,"certificate":cert.cert.pem(),"secret_ref":admin_ref,"request_id":Uuid::new_v4(),"name":"native fixture admin","pending":false,"invitation_id":null,"device_id":authority.authenticate(&token).unwrap().device_id()}),
    );
    write(
        "airplay/receiver.json",
        json!({"key_reference":airplay_ref,"known_keys":[],"blocked_keys":[],"playback_allowed":true}),
    );
    let destination = fixture.root.join("destination");
    let report = migrate(
        &Options {
            layout: Layout::Hub,
            source: source.clone(),
            destination: destination.clone(),
            profile_kind: None,
        },
        &mut Native,
        &mut NoHooks,
    )
    .unwrap();
    assert!(report.identity_preserved && report.source_unchanged);
    assert!(
        fs::read(source.join("state.json")).unwrap()
            == fs::read(destination.join("state.json")).unwrap()
    );
    let store = FileCredentialStore::for_profile(&destination.join("admin.json")).unwrap();
    assert!(
        store
            .get(&admin_ref, SecretKind::AdminToken)
            .unwrap()
            .expose()
            == token
    );
    for (reference, expected) in &fixture.entries {
        assert!(
            action("read", reference, None).unwrap() == *expected,
            "native fixture value changed"
        );
    }
    let references: Vec<_> = fixture.entries.iter().map(|(r, _)| r.clone()).collect();
    assert!(
        fixture.cleanup(),
        "native fixture cleanup failed; ownership journal retained"
    );
    for reference in references {
        assert!(
            matches!(
                action("read", &reference, None),
                Err(Error("migration_native_missing"))
            ),
            "native fixture entry remains"
        );
    }
}
