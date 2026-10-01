//! Product pairing orchestration; laboratory provisioning remains explicit.
use crate::{Credential, Provisioned, Result, ServerConfig, emit, read, write_secret};
use futures_util::{StreamExt, stream::FuturesUnordered};
use neonmix_control::{Authority, Role, Snapshot};
use neonmix_identity::{
    discovery,
    pairing::{Completion, Invitation, Request},
    vault,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u16,
    pub hub_id: Uuid,
    pub certificate: String,
    pub secret_ref: String,
    pub request_id: Uuid,
    pub name: String,
    pub pending: bool,
    pub invitation_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HubProfile {
    version: u16,
    room_name: String,
    state_path: PathBuf,
    output: String,
    certificate: String,
    private_key_ref: String,
    admin_token_ref: String,
}
pub fn credential(path: &Path) -> Result<Credential> {
    let value: serde_json::Value = read(path)?;
    if value.get("secret_ref").is_none() {
        return Ok(serde_json::from_value(value)?);
    }
    let profile: Profile = serde_json::from_value(value)?;
    if profile.version != 1 || profile.pending {
        return Err("pairing incomplete; resume the pair command".into());
    }
    Ok(Credential {
        route: None,
        hub_id: Some(profile.hub_id),
        token: vault::get(&profile.secret_ref)?,
        certificate: profile.certificate,
    })
}
pub fn config(path: &Path) -> Result<ServerConfig> {
    let value: serde_json::Value = read(path)?;
    if value.get("private_key_ref").is_none() {
        return Ok(serde_json::from_value(value)?);
    }
    let profile: HubProfile = serde_json::from_value(value)?;
    if profile.version != 1 {
        return Err("incompatible Hub profile version".into());
    }
    let private_key = vault::get(&profile.private_key_ref)?;
    Ok(ServerConfig {
        state_path: Some(profile.state_path),
        output: profile.output,
        pem: format!("{}{}", profile.certificate, private_key),
        certificate: profile.certificate,
        private_key,
        devices: vec![Provisioned {
            name: "admin".into(),
            role: Role::Admin,
            token: vault::get(&profile.admin_token_ref)?,
        }],
        room_name: Some(profile.room_name),
    })
}
struct SetupLock(PathBuf);
impl Drop for SetupLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn setup(directory: &Path, output: String, name: String) -> Result<()> {
    if output.is_empty()
        || name.trim().is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
    {
        return Err("invalid output or room name".into());
    }
    std::fs::create_dir_all(directory)?;
    if std::fs::read_dir(directory)?.next().is_some() {
        return Err("setup requires an empty directory".into());
    }
    let state_path = directory.canonicalize()?.join("state.json");
    let lock_path = directory.join("setup.lock");
    write_secret(&lock_path, b"")?;
    let _setup_lock = SetupLock(lock_path);
    let (_, certificate, private_key) = crate::certificate()?;
    let token = neonmix_identity::secret();
    let authority = Authority::new(output.clone(), "admin".into(), &token)?;
    let key_ref = vault::put(&private_key)?;
    let token_ref = match vault::put(&token) {
        Ok(r) => r,
        Err(e) => {
            let _ = vault::remove(&key_ref);
            return Err(e);
        }
    };
    let files = [
        directory.join("state.json"),
        directory.join("server.json"),
        directory.join("admin.json"),
    ];
    let mut created = Vec::new();
    let result: Result<()> = (|| {
        write_secret(
            &files[0],
            &serde_json::to_vec_pretty(&authority.persistent())?,
        )?;
        created.push(files[0].clone());
        write_secret(
            &files[1],
            &serde_json::to_vec_pretty(&HubProfile {
                version: 1,
                room_name: name.clone(),
                state_path,
                output,
                certificate: certificate.clone(),
                private_key_ref: key_ref.clone(),
                admin_token_ref: token_ref.clone(),
            })?,
        )?;
        created.push(files[1].clone());
        write_secret(
            &files[2],
            &serde_json::to_vec_pretty(&Profile {
                version: 1,
                hub_id: authority.current().hub_id,
                certificate,
                secret_ref: token_ref.clone(),
                request_id: Uuid::new_v4(),
                name: "admin".into(),
                pending: false,
                invitation_id: None,
                device_id: Some(authority.authenticate(&token)?.device_id()),
            })?,
        )?;
        created.push(files[2].clone());
        Ok(())
    })();
    if result.is_err() {
        let _ = vault::remove(&key_ref);
        let _ = vault::remove(&token_ref);
        for f in created {
            let _ = std::fs::remove_file(f);
        }
    }
    result?;
    emit(
        serde_json::json!({"event":"hub_configured","hub_id":authority.current().hub_id,"room_name":name,"directory":directory.canonicalize()?,"credential_store":"platform"}),
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
        let previous: Profile = read(path)?;
        if previous.version != 1
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
        let reference = vault::put(&neonmix_identity::secret())?;
        let profile = Profile {
            version: 1,
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
            let _ = vault::remove(&reference);
            return Err(e);
        }
        profile
    };
    let mut c = Credential {
        route: None,
        hub_id: Some(profile.hub_id),
        certificate: profile.certificate.clone(),
        token: vault::get(&profile.secret_ref)?,
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
            return Err(format!("pairing rejected: {}", response.text().await?).into());
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
        serde_json::json!({"event":"paired","hub_id":profile.hub_id,"device_id":profile.device_id,"credential":path,"credential_store":"platform"}),
    )
}
pub fn forget(path: &Path) -> Result<()> {
    let profile: Profile = read(path)?;
    vault::remove(&profile.secret_ref)?;
    std::fs::remove_file(path)?;
    emit(serde_json::json!({"event":"local_credential_forgotten","hub_id":profile.hub_id}))
}
