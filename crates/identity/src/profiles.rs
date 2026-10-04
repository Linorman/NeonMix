//! Closed product schemas. Metadata never loads secrets or contacts a vault.
use crate::{
    Result, files,
    store::{FileCredentialStore, SecretKind},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

// A profile may contain a certificate and bounded trust lists, but no secret.
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
pub const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStore {
    File,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    Admin,
    Member,
}
impl ProfileKind {
    pub fn secret_kind(self) -> SecretKind {
        match self {
            Self::Admin => SecretKind::AdminToken,
            Self::Member => SecretKind::MemberToken,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenProfile {
    pub version: u16,
    pub credential_store: CredentialStore,
    pub profile_kind: ProfileKind,
    pub hub_id: Uuid,
    pub certificate: String,
    pub secret_ref: String,
    pub request_id: Uuid,
    pub name: String,
    pub pending: bool,
    pub invitation_id: Option<Uuid>,
    pub device_id: Option<Uuid>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubProfile {
    pub version: u16,
    pub credential_store: CredentialStore,
    pub room_name: String,
    pub state_path: PathBuf,
    pub output: String,
    pub certificate: String,
    pub private_key_ref: String,
    pub admin_token_ref: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    LowLatency,
    Synchronized,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirPlayProfile<M = PlaybackMode> {
    pub version: u16,
    pub credential_store: CredentialStore,
    pub key_reference: Option<String>,
    pub known_keys: BTreeSet<String>,
    pub blocked_keys: BTreeSet<String>,
    pub playback_allowed: bool,
    #[serde(default)]
    pub playback_mode: M,
}
impl<M: Default> Default for AirPlayProfile<M> {
    fn default() -> Self {
        Self {
            version: 1,
            credential_store: CredentialStore::File,
            key_reference: None,
            known_keys: Default::default(),
            blocked_keys: Default::default(),
            playback_allowed: true,
            playback_mode: M::default(),
        }
    }
}
pub enum ProductProfile {
    Hub(HubProfile),
    Token(TokenProfile),
    AirPlay(AirPlayProfile),
    AirPlayV2(crate::airplay_profile::ReceiverProfile),
}
fn reference(value: &str) -> Result<()> {
    if Uuid::parse_str(value)
        .ok()
        .is_none_or(|id| id.to_string() != value)
    {
        return Err("credential_corrupt".into());
    }
    Ok(())
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
pub fn parse_metadata(bytes: &[u8]) -> Result<ProductProfile> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("credential_corrupt".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "credential_corrupt")?;
    let version = value.get("version").and_then(serde_json::Value::as_u64);
    if value.get("receivers").is_some() {
        return Ok(ProductProfile::AirPlayV2(crate::airplay_profile::parse(
            bytes,
        )?));
    }
    let hub = value.get("private_key_ref").is_some();
    let token = value.get("secret_ref").is_some();
    let airplay = value.get("key_reference").is_some();
    if (hub || token) && version == Some(1) && value.get("credential_store").is_none() {
        return Err("migration_required: run neonmix-credential-migrate offline".into());
    }
    if airplay && version.is_none() {
        return Err("migration_required: run neonmix-credential-migrate offline".into());
    }
    if version != Some(if airplay { 1 } else { 2 }) {
        return Err("credential_version_unsupported".into());
    }
    if hub {
        let profile: HubProfile =
            serde_json::from_value(value).map_err(|_| "credential_corrupt")?;
        reference(&profile.private_key_ref)?;
        reference(&profile.admin_token_ref)?;
        if !text(&profile.room_name)
            || profile.output.is_empty()
            || profile.certificate.len() > 8192
            || profile.state_path.as_os_str().is_empty()
            || profile
                .state_path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("credential_corrupt".into());
        }
        Ok(ProductProfile::Hub(profile))
    } else if token {
        let profile: TokenProfile =
            serde_json::from_value(value).map_err(|_| "credential_corrupt")?;
        reference(&profile.secret_ref)?;
        if !text(&profile.name)
            || profile.certificate.is_empty()
            || profile.certificate.len() > 8192
            || (profile.pending
                && (profile.profile_kind != ProfileKind::Member || profile.invitation_id.is_none()))
            || (!profile.pending && profile.device_id.is_none())
        {
            return Err("credential_corrupt".into());
        }
        Ok(ProductProfile::Token(profile))
    } else if airplay {
        let profile: AirPlayProfile =
            serde_json::from_value(value).map_err(|_| "credential_corrupt")?;
        reference(
            profile
                .key_reference
                .as_deref()
                .ok_or("credential_missing")?,
        )?;
        if profile.known_keys.len() + profile.blocked_keys.len() > 64
            || profile
                .known_keys
                .iter()
                .chain(&profile.blocked_keys)
                // The worker returns encoded public keys (currently Base64). Keep
                // these protocol identifiers verbatim; they are not SHA256 hex IDs.
                .any(|k| k.is_empty() || k.len() > 128 || k.chars().any(char::is_control))
        {
            return Err("credential_corrupt".into());
        }
        Ok(ProductProfile::AirPlay(profile))
    } else {
        Err("credential_corrupt".into())
    }
}
pub fn load_profile_metadata(path: &Path) -> Result<ProductProfile> {
    let bytes = files::read_private(path, MAX_PROFILE_BYTES).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "credential_missing",
        std::io::ErrorKind::PermissionDenied => "credential_permission_denied",
        std::io::ErrorKind::InvalidData => "credential_corrupt",
        _ => "credential_io_failed",
    })?;
    parse_metadata(&bytes)
}
pub fn token(path: &Path) -> Result<TokenProfile> {
    match load_profile_metadata(path)? {
        ProductProfile::Token(p) => Ok(p),
        _ => Err("credential_kind_mismatch".into()),
    }
}
pub fn hub(path: &Path) -> Result<HubProfile> {
    match load_profile_metadata(path)? {
        ProductProfile::Hub(p) => Ok(p),
        _ => Err("credential_kind_mismatch".into()),
    }
}
pub fn state_path(path: &Path, profile: &HubProfile) -> Result<PathBuf> {
    if profile
        .state_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("credential_corrupt".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let target = parent.join(&profile.state_path);
    // Every existing component must stay inside the profile's directory.
    let canonical = target
        .canonicalize()
        .map_err(|_| "setup_incomplete: state file missing")?;
    if !canonical.starts_with(&parent) || canonical != target {
        return Err("credential_permission_denied".into());
    }
    Ok(target)
}
pub fn profile_lock_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.lock",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}
/// Local deletion protection is shared by CLI and daemon; it grants no remote role.
pub fn check_forget(path: &Path, selected: &TokenProfile, admins: &[PathBuf]) -> Result<()> {
    if selected.profile_kind != ProfileKind::Member {
        return Err("administrator_credential_protected".into());
    }
    let store = FileCredentialStore::for_profile(path)?;
    for admin_path in admins.iter().filter(|p| p.exists()) {
        let admin = token(admin_path)?;
        if admin.profile_kind != ProfileKind::Admin {
            return Err("credential_corrupt".into());
        }
        if (store.identity() == FileCredentialStore::for_profile(admin_path)?.identity()
            && selected.secret_ref == admin.secret_ref)
            || (selected.hub_id == admin.hub_id
                && selected.device_id.is_some()
                && selected.device_id == admin.device_id)
        {
            return Err("administrator_credential_protected".into());
        }
    }
    Ok(())
}

pub fn operation_lock(path: &Path) -> Result<files::Guard> {
    files::lock(path).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        match e.kind() {
            std::io::ErrorKind::WouldBlock => "credential_store_busy".into(),
            std::io::ErrorKind::PermissionDenied => "credential_permission_denied".into(),
            _ => "credential_io_failed".into(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_preserves_worker_encoded_public_keys() {
        let known = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
        let blocked = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";
        let encoded = serde_json::to_vec(&serde_json::json!({"version":1,"credential_store":"file","key_reference":Uuid::new_v4(),"known_keys":[known],"blocked_keys":[blocked],"playback_allowed":true,"playback_mode":"low_latency"})).unwrap();
        let ProductProfile::AirPlay(profile) = parse_metadata(&encoded).unwrap() else {
            panic!("expected receiver metadata");
        };
        assert!(profile.known_keys.contains(known));
        assert!(profile.blocked_keys.contains(blocked));
        for key in ["".to_owned(), "x".repeat(129), "invalid\nkey".to_owned()] {
            let mut profile = profile.clone();
            profile.known_keys = BTreeSet::from([key]);
            assert!(parse_metadata(&serde_json::to_vec(&profile).unwrap()).is_err());
        }
    }
}
