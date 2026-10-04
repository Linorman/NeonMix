//! Product pairing orchestration; laboratory provisioning remains explicit.
use crate::{Credential, Provisioned, Result, ServerConfig, emit, read, write_secret};
use futures_util::{StreamExt, stream::FuturesUnordered};
use neonmix_control::{Authority, Role, Snapshot};
pub use neonmix_identity::profiles::TokenProfile as Profile;
use neonmix_identity::{
    discovery, files,
    pairing::{Completion, Invitation, Request},
    profiles::{self, CredentialStore, HubProfile, ProfileKind},
    store::{FileCredentialStore, SecretKind},
};
use std::collections::BTreeSet;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
fn laboratory(path: &Path) -> Result<serde_json::Value> {
    let bytes = files::read_private(path, profiles::MAX_PROFILE_BYTES)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "credential_corrupt")?;
    if [
        "version",
        "credential_store",
        "profile_kind",
        "secret_ref",
        "private_key_ref",
    ]
    .iter()
    .any(|k| value.get(k).is_some())
    {
        // Preserve migration/version errors; never reinterpret formal formats.
        profiles::parse_metadata(&bytes)?;
        return Err("credential_kind_mismatch".into());
    }
    Ok(value)
}
pub fn credential(path: &Path) -> Result<Credential> {
    match profiles::token(path) {
        Ok(profile) => {
            if profile.pending {
                return Err("pairing incomplete; resume the pair command".into());
            }
            let store = FileCredentialStore::for_profile(path)?;
            Ok(Credential {
                route: None,
                hub_id: Some(profile.hub_id),
                token: store
                    .get(&profile.secret_ref, profile.profile_kind.secret_kind())?
                    .expose()
                    .to_owned(),
                certificate: profile.certificate,
            })
        }
        Err(error) => {
            let value = laboratory(path).map_err(|_| error)?;
            Ok(serde_json::from_value(value).map_err(|_| "credential_corrupt")?)
        }
    }
}
pub fn config(path: &Path) -> Result<ServerConfig> {
    let profile = match profiles::hub(path) {
        Ok(p) => p,
        Err(error) => {
            return Ok(serde_json::from_value(laboratory(path).map_err(|_| error)?)
                .map_err(|_| "credential_corrupt")?);
        }
    };
    let store = FileCredentialStore::for_profile(path)?;
    let private_key = store
        .get(&profile.private_key_ref, SecretKind::HubTlsKey)?
        .expose()
        .to_owned();
    let token = store
        .get(&profile.admin_token_ref, SecretKind::AdminToken)?
        .expose()
        .to_owned();
    let state_path = profiles::state_path(path, &profile)?;
    let saved: neonmix_control::PersistentState = serde_json::from_slice(&files::read_private(
        &state_path,
        neonmix_identity::profiles::MAX_STATE_BYTES,
    )?)
    .map_err(|_| "credential_corrupt")?;
    let authority = Authority::restore(saved)?;
    let principal = authority.authenticate(&token)?;
    if authority
        .current()
        .devices
        .get(&principal.device_id())
        .is_none_or(|d| d.role != Role::Admin)
        || authority.current().output.id != profile.output
    {
        return Err("credential_kind_mismatch".into());
    }
    let admin_path = path.with_file_name("admin.json");
    let admin = profiles::token(&admin_path)?;
    if admin.profile_kind != ProfileKind::Admin
        || admin.secret_ref != profile.admin_token_ref
        || admin.hub_id != authority.current().hub_id
        || admin.device_id != Some(principal.device_id())
        || admin.certificate != profile.certificate
    {
        return Err("credential_corrupt".into());
    }
    Ok(ServerConfig {
        state_path: Some(state_path),
        require_existing_state: true,
        output: profile.output,
        pem: format!("{}{}", profile.certificate, private_key),
        certificate: profile.certificate,
        private_key,
        devices: vec![Provisioned {
            name: "admin".into(),
            role: Role::Admin,
            token,
        }],
        room_name: Some(profile.room_name),
    })
}
pub fn setup(directory: &Path, output: String, name: String) -> Result<()> {
    if output.is_empty()
        || name.trim().is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
    {
        return Err("invalid output or room name".into());
    }
    if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    files::private_dir(directory)?;
    let _setup_lock = profiles::operation_lock(&directory.join("setup.lock"))?;
    if std::fs::read_dir(directory)?
        .filter_map(|e| e.ok())
        .any(|e| e.file_name() != "setup.lock")
    {
        return Err(
            "setup_incomplete: directory already contains data; inspect before explicit cleanup"
                .into(),
        );
    }
    let (_, certificate, private_key) = crate::certificate()?;
    let token = neonmix_identity::secret();
    let authority = Authority::new(output.clone(), "admin".into(), &token)?;
    let store = FileCredentialStore::for_profile(&directory.join("server.json"))?;
    let key_ref = match store.put(SecretKind::HubTlsKey, &private_key) {
        Ok(reference) => reference,
        Err(error) => {
            let _ = std::fs::remove_dir(store.identity());
            return Err(error);
        }
    };
    let token_ref = match store.put(SecretKind::AdminToken, &token) {
        Ok(r) => r,
        Err(e) => {
            let _ = store.remove(&key_ref, SecretKind::HubTlsKey);
            let _ = std::fs::remove_dir(store.identity());
            let _ = files::sync_parent(&store.identity());
            return Err(e);
        }
    };
    #[cfg(test)]
    setup_checkpoint("secrets");
    let mut created = Vec::new();
    let result: Result<()> = (|| {
        for (filename, bytes) in [
            (
                "state.json",
                serde_json::to_vec_pretty(&authority.persistent())?,
            ),
            (
                "admin.json",
                serde_json::to_vec_pretty(&Profile {
                    version: 2,
                    credential_store: CredentialStore::File,
                    profile_kind: ProfileKind::Admin,
                    hub_id: authority.current().hub_id,
                    certificate: certificate.clone(),
                    secret_ref: token_ref.clone(),
                    request_id: Uuid::new_v4(),
                    name: "admin".into(),
                    pending: false,
                    invitation_id: None,
                    device_id: Some(authority.authenticate(&token)?.device_id()),
                })?,
            ),
            (
                "server.json",
                serde_json::to_vec_pretty(&HubProfile {
                    version: 2,
                    credential_store: CredentialStore::File,
                    room_name: name.clone(),
                    state_path: PathBuf::from("state.json"),
                    output,
                    certificate,
                    private_key_ref: key_ref.clone(),
                    admin_token_ref: token_ref.clone(),
                })?,
            ),
        ] {
            let path = directory.join(filename);
            write_secret(&path, &bytes)?;
            created.push(path);
            #[cfg(test)]
            setup_checkpoint(filename);
        }
        Ok(())
    })();
    if result.is_err() {
        for f in created {
            let _ = std::fs::remove_file(f);
        }
        let _ = store.remove(&key_ref, SecretKind::HubTlsKey);
        let _ = store.remove(&token_ref, SecretKind::AdminToken);
        let _ = std::fs::remove_dir(directory.join(".credentials"));
    }
    result?;
    emit(
        serde_json::json!({"event":"hub_configured","hub_id":authority.current().hub_id,"room_name":name,"directory":directory.canonicalize()?,"credential_store":"file"}),
    )
}
pub fn check_hub(c: &Credential, hub_id: Uuid) -> Result<()> {
    if c.hub_id.is_some_and(|id| id != hub_id) {
        return Err("authenticated Hub UUID differs from paired identity".into());
    }
    Ok(())
}
fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
pub fn validate_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err("Hub address must be an HTTPS origin".into());
    }
    Ok(())
}
async fn verify_endpoint(c: &Credential, url: &str) -> Result<()> {
    validate_url(url)?;
    let response: serde_json::Value = crate::sender::client(c)?
        .get(format!("{}/v1/identity", url.trim_end_matches('/')))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id: Uuid = serde_json::from_value(
        response
            .get("hub_id")
            .cloned()
            .ok_or("missing Hub identity")?,
    )?;
    check_hub(c, id)?;
    if response.get("version").and_then(|v| v.as_u64()) != Some(1) {
        return Err("incompatible Hub protocol".into());
    }
    Ok(())
}
pub async fn endpoint(c: &mut Credential, manual: Option<String>) -> Result<String> {
    let manual = manual.or_else(|| c.hub_id.is_none().then(|| "https://localhost:7443".into()));
    if let Some(url) = manual {
        c.route = None;
        let url = url.trim_end_matches('/').to_string();
        verify_endpoint(c, &url).await?;
        return Ok(url);
    }
    let id = c.hub_id.ok_or("laboratory credentials require --hub")?;
    let (publish, mut discovered) = tokio::sync::mpsc::channel(8);
    let search = tokio::task::spawn_blocking(move || {
        discovery::browse(3, |event, candidate| {
            if publish.is_closed() {
                return Err("discovery selection completed".into());
            }
            if event == "resolved" && candidate.hub_id == id {
                let _ = publish.try_send(candidate.clone());
            }
            Ok(())
        })
    });
    let mut checks = FuturesUnordered::new();
    let mut seen = BTreeSet::new();
    let mut finished = false;
    // Try independent interfaces as they resolve, so a slow address or the
    // collection window cannot delay a healthy, already authenticated endpoint.
    while !finished || !checks.is_empty() {
        tokio::select! {
            candidate=discovered.recv(),if !finished=>{
                let Some(candidate)=candidate else {finished=true;continue;};
                for route in candidate.endpoints {
                    if checks.len()>=32 || seen.len()>=128 {break;}
                    if !seen.insert(route.clone()) {continue;}
                    let mut probe=c.clone();probe.route=Some(route.clone());
                    checks.push(async move {
                        let checked=tokio::time::timeout(std::time::Duration::from_secs(2),verify_endpoint(&probe,&route.url)).await;
                        matches!(checked,Ok(Ok(()))).then_some(route)
                    });
                }
            },
            checked=checks.next(),if !checks.is_empty()=>{
                if let Some(Some(route))=checked {
                    let url=route.url.clone();c.route=Some(route);return Ok(url);
                }
            },
        }
    }
    search.await??;
    c.route = None;
    Err("paired Hub is offline, unreachable or its identity changed".into())
}
pub async fn invite(path: &Path, hub: Option<String>, out: &Path, seconds: u32) -> Result<()> {
    if out.exists() {
        return Err("invitation output already exists".into());
    }
    let mut c = credential(path)?;
    let hub = endpoint(&mut c, hub).await?;
    let invitation: Invitation = crate::sender::client(&c)?
        .post(format!("{hub}/v1/pairing/invitations"))
        .bearer_auth(&c.token)
        .json(&serde_json::json!({"ttl_seconds":seconds}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    check_hub(&c, invitation.hub_id)?;
    if let Err(error) = write_secret(out, &serde_json::to_vec_pretty(&invitation)?) {
        let _ = cancel(path, Some(hub), invitation.invitation_id).await;
        return Err(error);
    }
    emit(
        serde_json::json!({"event":"invitation_created","invitation_id":invitation.invitation_id,"hub_id":invitation.hub_id,"expires_unix_seconds":invitation.expires_unix_seconds,"file":out}),
    )
}
pub async fn cancel(path: &Path, hub: Option<String>, id: Uuid) -> Result<()> {
    let mut c = credential(path)?;
    let hub = endpoint(&mut c, hub).await?;
    crate::sender::client(&c)?
        .post(format!("{hub}/v1/pairing/cancel"))
        .bearer_auth(&c.token)
        .json(&serde_json::json!({"invitation_id":id}))
        .send()
        .await?
        .error_for_status()?;
    emit(serde_json::json!({"event":"invitation_cancelled","invitation_id":id}))
}
fn commit_profile(path: &Path, profile: &Profile) -> Result<()> {
    Ok(neonmix_identity::files::replace(
        path,
        &serde_json::to_vec_pretty(profile)?,
    )?)
}

pub async fn pair(invite: &Path, path: &Path, name: String, manual: Option<String>) -> Result<()> {
    let _profile_lock = profiles::operation_lock(&profiles::profile_lock_path(path))?;
    let store = FileCredentialStore::for_profile(path)?;
    let invitation: Invitation = read(invite)?;
    if invitation.version != 1
        || invitation.secret.len() != 64
        || invitation.certificate.len() > 8192
    {
        return Err("invalid invitation".into());
    }
    // Validate the pin before any credential-store mutation.
    neonmix_identity::trust::tls(&invitation.certificate)?;
    let mut profile = if path.exists() {
        let previous = profiles::token(path)?;
        if previous.version != 2
            || previous.profile_kind != ProfileKind::Member
            || previous.hub_id != invitation.hub_id
            || previous.certificate != invitation.certificate
            || previous.name != name
            || (previous.pending && previous.invitation_id != Some(invitation.invitation_id))
        {
            return Err("credential path belongs to another pairing; use a new path".into());
        }
        previous
    } else {
        if invitation.expires_unix_seconds <= now()? {
            return Err("invitation expired".into());
        }
        Request {
            invitation_id: invitation.invitation_id,
            request_id: Uuid::new_v4(),
            name: name.clone(),
            token_sha256: neonmix_identity::digest(&neonmix_identity::secret()),
        }
        .validate()?;
        let reference = store.put(SecretKind::MemberToken, &neonmix_identity::secret())?;
        let profile = Profile {
            version: 2,
            credential_store: CredentialStore::File,
            profile_kind: ProfileKind::Member,
            hub_id: invitation.hub_id,
            certificate: invitation.certificate.clone(),
            secret_ref: reference.clone(),
            request_id: Uuid::new_v4(),
            name,
            pending: true,
            invitation_id: Some(invitation.invitation_id),
            device_id: None,
        };
        if let Err(e) = write_secret(path, &serde_json::to_vec_pretty(&profile)?) {
            let _ = store.remove(&reference, SecretKind::MemberToken);
            return Err(e);
        }
        profile
    };
    let mut c = Credential {
        route: None,
        hub_id: Some(profile.hub_id),
        certificate: profile.certificate.clone(),
        token: store
            .get(&profile.secret_ref, SecretKind::MemberToken)?
            .expose()
            .to_owned(),
    };
    let hub = endpoint(&mut c, manual).await?;
    let client = crate::sender::client(&c)?;
    // Durable server commit may outlive a lost response or Hub restart.
    let recovery = client
        .get(format!("{hub}/v1/hub"))
        .bearer_auth(&c.token)
        .send()
        .await?;
    if recovery.status().is_success() {
        let snapshot: Snapshot = recovery.json().await?;
        check_hub(&c, snapshot.hub_id)?;
    } else {
        if recovery.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Err("pairing recovery request failed".into());
        }
        if !profile.pending {
            return Err("credential revoked; create a new invitation and credential path".into());
        }
        let request = Request {
            invitation_id: invitation.invitation_id,
            request_id: profile.request_id,
            name: profile.name.clone(),
            token_sha256: neonmix_identity::digest(&c.token),
        };
        let response = client
            .post(format!("{hub}/v1/pairing/complete"))
            .bearer_auth(&invitation.secret)
            .json(&request)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("pairing rejected: {}", response.status()).into());
        }
        let completion: Completion = response.json().await?;
        check_hub(&c, completion.hub_id)?;
        profile.device_id = Some(completion.device_id);
    }
    let me: serde_json::Value = client
        .get(format!("{hub}/v1/me"))
        .bearer_auth(&c.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    check_hub(&c, serde_json::from_value(me["hub_id"].clone())?)?;
    profile.device_id = Some(serde_json::from_value(me["device_id"].clone())?);
    profile.pending = false;
    commit_profile(path, &profile)?;
    emit(
        serde_json::json!({"event":"paired","hub_id":profile.hub_id,"device_id":profile.device_id,"credential":path,"credential_store":"file"}),
    )
}
pub fn forget(path: &Path) -> Result<()> {
    let _lock = profiles::operation_lock(&profiles::profile_lock_path(path))?;
    let profile = profiles::token(path)?;
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut admins = vec![parent.join("admin.json")];
    if let Some(root) = parent.parent() {
        admins.push(root.join("hub/admin.json"));
    }
    profiles::check_forget(path, &profile, &admins)?;
    FileCredentialStore::for_profile(path)?.remove(&profile.secret_ref, SecretKind::MemberToken)?;
    std::fs::remove_file(path)?;
    files::sync_parent(path)?;
    emit(serde_json::json!({"event":"local_credential_forgotten","hub_id":profile.hub_id}))
}
pub fn store_probe(directory: &Path) -> Result<()> {
    files::private_dir(directory)?;
    let fixture = directory.join(format!("credential-probe-{}", Uuid::new_v4()));
    files::private_dir(&fixture)?;
    let result: Result<()> = (|| {
        let store = FileCredentialStore::for_profile(&fixture.join("profile.json"))?;
        let secret = neonmix_identity::secret();
        let reference = store.put(SecretKind::MemberToken, &secret)?;
        if store.get(&reference, SecretKind::MemberToken)?.expose() != secret {
            return Err("credential_corrupt".into());
        }
        store.remove(&reference, SecretKind::MemberToken)?;
        store.remove(&reference, SecretKind::MemberToken)?;
        Ok(())
    })();
    std::fs::remove_dir_all(fixture)?;
    result?;
    emit(serde_json::json!({"event":"credential_store_verified", "credential_store":"file"}))
}

#[cfg(test)]
fn setup_checkpoint(phase: &str) {
    if std::env::var("NEONMIX_TEST_SETUP_PHASE").ok().as_deref() == Some(phase) {
        std::process::exit(77);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let directory = root
                .join(".local/tmp")
                .join(format!("hub-credentials-{}", Uuid::new_v4()));
            files::private_dir(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn setup_crash_helper() {
        if let Some(directory) = std::env::var_os("NEONMIX_TEST_SETUP_DIRECTORY") {
            setup(
                Path::new(&directory),
                "fixture-output".into(),
                "room".into(),
            )
            .unwrap();
        }
    }
    #[test]
    fn setup_force_exit_preserves_incomplete_identity_and_refuses_retry() {
        for phase in ["secrets", "state.json", "admin.json", "server.json"] {
            let fixture = Fixture::new();
            let directory = fixture.0.join("hub");
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "identity::tests::setup_crash_helper"])
                .env("NEONMIX_TEST_SETUP_DIRECTORY", &directory)
                .env("NEONMIX_TEST_SETUP_PHASE", phase)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(77));
            let entries_before: Vec<_> = std::fs::read_dir(directory.join(".credentials"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(entries_before.len(), 2);
            assert!(
                setup(&directory, "fixture-output".into(), "room".into())
                    .unwrap_err()
                    .to_string()
                    .contains("setup_incomplete")
            );
            assert_eq!(
                std::fs::read_dir(directory.join(".credentials"))
                    .unwrap()
                    .count(),
                2
            );
            assert_eq!(
                directory.join("server.json").exists(),
                phase == "server.json"
            );
            if phase == "server.json" {
                assert!(config(&directory.join("server.json")).is_ok());
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn failed_pending_commit_preserves_secret_request_and_recovery_metadata() {
        let fixture = Fixture::new();
        let path = fixture.0.join("member.json");
        let store = FileCredentialStore::for_profile(&path).unwrap();
        let secret = neonmix_identity::secret();
        let reference = store.put(SecretKind::MemberToken, &secret).unwrap();
        let original = Profile {
            version: 2,
            credential_store: CredentialStore::File,
            profile_kind: ProfileKind::Member,
            hub_id: Uuid::new_v4(),
            certificate: "fixture-public".into(),
            secret_ref: reference.clone(),
            request_id: Uuid::new_v4(),
            name: "member".into(),
            pending: true,
            invitation_id: Some(Uuid::new_v4()),
            device_id: None,
        };
        write_secret(&path, &serde_json::to_vec(&original).unwrap()).unwrap();
        let mut completed = original.clone();
        completed.pending = false;
        completed.device_id = Some(Uuid::new_v4());
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o500)).unwrap();
        let failure = commit_profile(&path, &completed);
        std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failure.is_err());
        let retained = profiles::token(&path).unwrap();
        assert!(retained.pending);
        assert_eq!(retained.request_id, original.request_id);
        assert_eq!(retained.invitation_id, original.invitation_id);
        assert_eq!(retained.secret_ref, reference);
        assert!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap()
                .expose()
                == secret,
            "pending secret changed"
        );
    }
    #[test]
    fn setup_move_missing_state_and_missing_secret_do_not_rebuild_identity() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        setup(&source, "fixture-output".into(), "room".into()).unwrap();
        let before = profiles::token(&source.join("admin.json")).unwrap();
        let token = credential(&source.join("admin.json")).unwrap().token;
        assert!(setup(&source, "fixture-output".into(), "room".into()).is_err());
        let destination = fixture.0.join("moved");
        std::fs::rename(&source, &destination).unwrap();
        let loaded = config(&destination.join("server.json")).unwrap();
        assert_eq!(
            loaded.state_path,
            Some(destination.join("state.json").canonicalize().unwrap())
        );
        assert!(loaded.devices[0].token == token, "moved credential differs");
        let moved = profiles::token(&destination.join("admin.json")).unwrap();
        assert_eq!(moved.hub_id, before.hub_id);
        let key = profiles::hub(&destination.join("server.json"))
            .unwrap()
            .private_key_ref;
        std::fs::remove_file(destination.join("state.json")).unwrap();
        assert!(config(&destination.join("server.json")).is_err());
        assert!(!destination.join("state.json").exists());
        assert_eq!(
            profiles::token(&destination.join("admin.json"))
                .unwrap()
                .hub_id,
            before.hub_id
        );
        FileCredentialStore::for_profile(&destination.join("server.json"))
            .unwrap()
            .remove(&key, SecretKind::HubTlsKey)
            .unwrap();
        assert!(config(&destination.join("server.json")).is_err());
    }
    #[test]
    fn forget_is_resumable_and_protects_admin_kind_and_device_aliases() {
        let fixture = Fixture::new();
        let hub = fixture.0.join("hub");
        setup(&hub, "fixture-output".into(), "room".into()).unwrap();
        let admin_path = hub.join("admin.json");
        let admin = profiles::token(&admin_path).unwrap();
        let store = FileCredentialStore::for_profile(&admin_path).unwrap();
        store
            .remove(&admin.secret_ref, SecretKind::AdminToken)
            .unwrap();
        assert!(forget(&admin_path).is_err());
        assert!(admin_path.exists());
        let mut alias = admin.clone();
        alias.profile_kind = ProfileKind::Member;
        let alias_path = hub.join("alias.json");
        write_secret(&alias_path, &serde_json::to_vec(&alias).unwrap()).unwrap();
        assert!(forget(&alias_path).is_err());
        assert!(alias_path.exists());
        let sender_path = hub.join("member.json");
        let mut member = alias;
        member.device_id = Some(Uuid::new_v4());
        member.secret_ref = store
            .put(SecretKind::MemberToken, &neonmix_identity::secret())
            .unwrap();
        write_secret(&sender_path, &serde_json::to_vec(&member).unwrap()).unwrap();
        // Simulate termination after secret deletion and before profile deletion.
        store
            .remove(&member.secret_ref, SecretKind::MemberToken)
            .unwrap();
        forget(&sender_path).unwrap();
        assert!(!sender_path.exists());
        assert!(admin_path.exists());
    }
    #[test]
    fn legacy_and_mixed_formats_never_enter_laboratory_branch() {
        let fixture = Fixture::new();
        let path = fixture.0.join("profile.json");
        for value in [
            serde_json::json!({"version":1,"secret_ref":Uuid::new_v4()}),
            serde_json::json!({"version":99,"token":"must-not-load", "certificate":"invalid"}),
            serde_json::json!({"token":"must-not-load", "certificate":"invalid", "secret_ref":Uuid::new_v4()}),
        ] {
            files::replace(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(credential(&path).is_err());
        }
        files::replace(
            &path,
            &serde_json::to_vec(&Credential {
                route: None,
                hub_id: None,
                token: neonmix_identity::secret(),
                certificate: "lab".into(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(credential(&path).unwrap().hub_id.is_none());
        let _held = profiles::operation_lock(&profiles::profile_lock_path(&path)).unwrap();
        assert!(
            forget(&path)
                .unwrap_err()
                .to_string()
                .contains("credential_store_busy")
        );
    }
}
