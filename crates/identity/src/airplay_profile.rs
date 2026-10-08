//! Durable multi-receiver identities and trust. Callers hold the receiver owner
//! lock and stop reception before migration or rollback; mutations use atomic files.
use crate::{
    Result, files,
    profiles::{AirPlayProfile, CredentialStore, MAX_PROFILE_BYTES, PlaybackMode},
    store::{FileCredentialStore, SecretKind},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};
use uuid::Uuid;

pub const MAX_RECEIVERS: usize = 4;
pub const MAX_SOURCES: usize = 64;
pub const MAX_BINDINGS: usize = 256;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverEndpoint {
    pub receiver_id: Uuid,
    pub receiver_uuid: Uuid,
    pub device_id: String,
    pub name: String,
    pub key_reference: String,
    pub enabled: bool,
    pub default_playback_mode: PlaybackMode,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSource {
    pub source_id: String,
    pub public_key: String,
    pub alias: Option<String>,
    pub last_name: Option<String>,
    pub blocked: bool,
    pub revoked: bool,
    pub gain_db: f32,
    pub muted: bool,
    pub playback_mode: Option<PlaybackMode>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingBinding {
    pub receiver_id: Uuid,
    pub source_id: String,
    pub revoked: bool,
}
fn initial_config_revision() -> u64 {
    1
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiverProfile {
    #[serde(default = "initial_config_revision")]
    pub config_revision: u64,
    pub version: u16,
    pub credential_store: CredentialStore,
    pub multi_receiver: bool,
    pub capacity: usize,
    pub playback_allowed: bool,
    pub receivers: Vec<ReceiverEndpoint>,
    pub sources: Vec<ExternalSource>,
    pub bindings: Vec<PairingBinding>,
}

/// Reject alternate encodings so worker trust lists and fingerprints agree.
pub fn source_id(public_key: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(public_key)
        .map_err(|_| "airplay_invalid_public_key")?;
    if bytes.len() != 32 || STANDARD.encode(&bytes) != public_key {
        return Err("airplay_invalid_public_key".into());
    }
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}
fn name_valid(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
impl ReceiverProfile {
    /// Assign one persistent version to a new candidate; repeated sync of an
    /// already assigned candidate never allocates another configuration.
    pub fn prepare_change(&self, next: &mut Self) -> Result<()> {
        next.validate()?;
        if next.config_revision < self.config_revision {
            return Err("profile_revision_conflict".into());
        }
        let mut old = serde_json::to_value(self)?;
        let mut new = serde_json::to_value(&*next)?;
        old.as_object_mut().unwrap().remove("config_revision");
        new.as_object_mut().unwrap().remove("config_revision");
        if old != new && next.config_revision == self.config_revision {
            next.config_revision = self
                .config_revision
                .checked_add(1)
                .ok_or("configuration_revision_exhausted")?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.config_revision == 0 {
            return Err("credential_corrupt".into());
        }
        if self.version != 2 {
            return Err("credential_version_unsupported".into());
        }
        if self.receivers.is_empty()
            || self.receivers.len() > MAX_RECEIVERS
            || self.sources.len() > MAX_SOURCES
            || self.bindings.len() > MAX_BINDINGS
            || !(1..=4).contains(&self.capacity)
        {
            return Err("credential_corrupt".into());
        }
        let mut ids = BTreeSet::new();
        let mut uuids = BTreeSet::new();
        let mut devices = BTreeSet::new();
        let mut references = BTreeSet::new();
        let mut names = BTreeSet::new();
        for r in &self.receivers {
            if r.receiver_id.is_nil()
                || r.receiver_uuid.is_nil()
                || !ids.insert(r.receiver_id)
                || !uuids.insert(r.receiver_uuid)
                || !devices.insert(&r.device_id)
                || !references.insert(&r.key_reference)
                || !names.insert(&r.name)
                || !name_valid(&r.name, 50)
                || crate::store::normalize_reference(&r.key_reference)? != r.key_reference
                || r.device_id != crate::speaker::receiver_id(r.receiver_uuid)
            {
                return Err("credential_corrupt".into());
            }
        }
        let mut sources = BTreeSet::new();
        for s in &self.sources {
            if source_id(&s.public_key)? != s.source_id
                || !sources.insert(&s.source_id)
                || !s.gain_db.is_finite()
                || !(-96.0..=12.0).contains(&s.gain_db)
                || s.alias.as_ref().is_some_and(|n| !name_valid(n, 128))
                || s.last_name.as_ref().is_some_and(|n| !name_valid(n, 128))
            {
                return Err("credential_corrupt".into());
            }
        }
        let mut bindings = BTreeSet::new();
        for b in &self.bindings {
            if !ids.contains(&b.receiver_id)
                || !sources.contains(&b.source_id)
                || !bindings.insert((b.receiver_id, &b.source_id))
            {
                return Err("credential_corrupt".into());
            }
        }
        Ok(())
    }
    /// A revoked source cannot be silently restored by a late registration.
    pub fn register_source(
        &mut self,
        receiver_id: Uuid,
        public_key: &str,
        name: Option<String>,
    ) -> Result<String> {
        if !self.receivers.iter().any(|r| r.receiver_id == receiver_id) {
            return Err("receiver_unknown".into());
        }
        if name.as_ref().is_some_and(|n| !name_valid(n, 128)) {
            return Err("airplay_invalid_name".into());
        }
        let id = source_id(public_key)?;
        let existing = self.sources.iter().position(|s| s.source_id == id);
        if existing.is_none() && self.sources.len() >= MAX_SOURCES {
            return Err("airplay_trust_capacity".into());
        }
        if existing.is_some_and(|i| self.sources[i].revoked || self.sources[i].blocked) {
            return Err("pairing_revoked".into());
        }
        let binding = self
            .bindings
            .iter()
            .find(|b| b.receiver_id == receiver_id && b.source_id == id);
        if binding.is_some_and(|b| b.revoked) {
            return Err("pairing_revoked".into());
        }
        if binding.is_none() && self.bindings.len() >= MAX_BINDINGS {
            return Err("airplay_trust_capacity".into());
        }
        let needs_binding = binding.is_none();
        if let Some(i) = existing {
            if name.is_some() {
                self.sources[i].last_name = name;
            }
        } else {
            self.sources.push(ExternalSource {
                source_id: id.clone(),
                public_key: public_key.into(),
                alias: None,
                last_name: name,
                blocked: false,
                revoked: false,
                gain_db: 0.0,
                muted: false,
                playback_mode: None,
            });
        }
        if needs_binding {
            self.bindings.push(PairingBinding {
                receiver_id,
                source_id: id.clone(),
                revoked: false,
            });
        }
        Ok(id)
    }
    pub fn revoke_source(&mut self, source_id: &str) -> Result<()> {
        let source = self
            .sources
            .iter_mut()
            .find(|s| s.source_id == source_id)
            .ok_or("source_unknown")?;
        source.revoked = true;
        for binding in self
            .bindings
            .iter_mut()
            .filter(|b| b.source_id == source_id)
        {
            binding.revoked = true;
        }
        Ok(())
    }
    pub fn trust_for(&self, receiver_id: Uuid) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
        if !self.receivers.iter().any(|r| r.receiver_id == receiver_id) {
            return Err("receiver_unknown".into());
        }
        let mut known = BTreeSet::new();
        let mut blocked = BTreeSet::new();
        for source in &self.sources {
            let binding = self
                .bindings
                .iter()
                .find(|b| b.receiver_id == receiver_id && b.source_id == source.source_id);
            if source.blocked || source.revoked || binding.is_some_and(|b| b.revoked) {
                blocked.insert(source.public_key.clone());
            } else if binding.is_some() {
                known.insert(source.public_key.clone());
            }
        }
        Ok((known, blocked))
    }
}
pub fn parse(bytes: &[u8]) -> Result<ReceiverProfile> {
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("credential_corrupt".into());
    }
    let profile: ReceiverProfile =
        serde_json::from_slice(bytes).map_err(|_| "credential_corrupt")?;
    profile.validate()?;
    Ok(profile)
}
pub fn load(path: &Path) -> Result<ReceiverProfile> {
    parse(&files::read_private(path, MAX_PROFILE_BYTES)?)
}
/// True while identity creation has a recoverable private journal.
pub fn operation_pending(path: &Path) -> Result<bool> {
    Ok(read_pending(path)?.is_some())
}
pub fn save(path: &Path, profile: &ReceiverProfile) -> Result<()> {
    if read_pending(path)?.is_some() {
        return Err("airplay_profile_operation_pending".into());
    }
    write_profile(path, profile)
}
fn write_profile(path: &Path, profile: &ReceiverProfile) -> Result<()> {
    profile.validate()?;
    let bytes = serde_json::to_vec_pretty(profile)?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("credential_corrupt".into());
    }
    files::replace_reported(path, &bytes)?;
    Ok(())
}
fn validate_secret(store: &FileCredentialStore, reference: &str) -> Result<()> {
    let secret = store.get(reference, SecretKind::AirplayReceiverKey)?;
    let key = rcgen::KeyPair::from_pem(secret.expose()).map_err(|_| "credential_corrupt")?;
    if key.algorithm() != &rcgen::PKCS_ED25519 {
        return Err("credential_kind_mismatch".into());
    }
    Ok(())
}
/// Caller supplies the exact legacy Speaker UUID and announcement name, which
/// were not part of the v1 file. Migration creates no new active receivers.
pub fn migrate_v1(path: &Path, legacy_uuid: Uuid, legacy_name: &str) -> Result<ReceiverProfile> {
    if let Some(profile) = recover_pending(path)? {
        return Ok(profile);
    }
    let old = match crate::profiles::load_profile_metadata(path)? {
        crate::profiles::ProductProfile::AirPlay(old) => old,
        crate::profiles::ProductProfile::AirPlayV2(profile) => {
            validate_secrets(path, &profile)?;
            return Ok(profile);
        }
        _ => return Err("credential_kind_mismatch".into()),
    };
    let key_reference = old.key_reference.clone().ok_or("credential_missing")?;
    let store = FileCredentialStore::for_profile(path)?;
    validate_secret(&store, &key_reference)?;
    let receiver_id = legacy_uuid;
    let mut profile = ReceiverProfile {
        config_revision: 1,
        version: 2,
        credential_store: CredentialStore::File,
        multi_receiver: false,
        capacity: 4,
        playback_allowed: true,
        receivers: vec![ReceiverEndpoint {
            receiver_id,
            receiver_uuid: legacy_uuid,
            device_id: crate::speaker::receiver_id(legacy_uuid),
            name: legacy_name.into(),
            key_reference,
            enabled: true,
            default_playback_mode: old.playback_mode,
        }],
        sources: vec![],
        bindings: vec![],
    };
    for key in old.known_keys.union(&old.blocked_keys) {
        let id = profile.register_source(receiver_id, key, None)?;
        if old.blocked_keys.contains(key) {
            profile.revoke_source(&id)?;
        }
        if !old.playback_allowed {
            profile
                .sources
                .iter_mut()
                .find(|source| source.source_id == id)
                .unwrap()
                .blocked = true;
        }
    }
    save(path, &profile)?;
    Ok(profile)
}
pub fn validate_secrets(path: &Path, profile: &ReceiverProfile) -> Result<()> {
    profile.validate()?;
    let store = FileCredentialStore::for_profile(path)?;
    for r in &profile.receivers {
        validate_secret(&store, &r.key_reference)?;
    }
    Ok(())
}

const MAX_JOURNAL_BYTES: usize = MAX_PROFILE_BYTES * 2 + 4096;
/// No secret content: the journal commits the intended identities and reference
/// before their secret can exist. The owner/profile lock serializes operations.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingReceiver {
    version: u16,
    previous: Option<ReceiverProfile>,
    next: ReceiverProfile,
    receiver_id: Uuid,
}
fn pending_path(path: &Path) -> std::path::PathBuf {
    path.with_file_name(format!(
        "{}.pending",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}
fn same_profile(a: &ReceiverProfile, b: &ReceiverProfile) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}
impl PendingReceiver {
    fn receiver(&self) -> Result<&ReceiverEndpoint> {
        if !matches!(self.version, 1 | 2) {
            return Err("credential_version_unsupported".into());
        }
        self.next.validate()?;
        let receiver = self
            .next
            .receivers
            .last()
            .filter(|r| r.receiver_id == self.receiver_id)
            .ok_or("credential_corrupt")?;
        if let Some(previous) = &self.previous {
            previous.validate()?;
            let mut expected = previous.clone();
            expected.receivers.push(receiver.clone());
            if self.version == 2 {
                previous.prepare_change(&mut expected)?;
            }
            if !same_profile(&expected, &self.next) {
                return Err("credential_corrupt".into());
            }
        } else if self.next.receivers.len() != 1
            || !self.next.sources.is_empty()
            || !self.next.bindings.is_empty()
        {
            return Err("credential_corrupt".into());
        }
        Ok(receiver)
    }
}
fn read_pending(path: &Path) -> Result<Option<PendingReceiver>> {
    let bytes = match files::read_private(&pending_path(path), MAX_JOURNAL_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let pending: PendingReceiver =
        serde_json::from_slice(&bytes).map_err(|_| "credential_corrupt")?;
    pending.receiver()?;
    Ok(Some(pending))
}
fn publish_pending(path: &Path, pending: &PendingReceiver) -> Result<()> {
    pending.receiver()?;
    if read_pending(path)?.is_some() {
        return Err("airplay_profile_operation_pending".into());
    }
    // Atomic journal publication: a torn write never becomes a valid pending
    // operation. The new key is not generated or written before this returns.
    files::replace(&pending_path(path), &serde_json::to_vec_pretty(pending)?)?;
    checkpoint(path, "journal");
    Ok(())
}
fn existing(path: &Path) -> Result<Option<ReceiverProfile>> {
    match files::read_private(path, MAX_PROFILE_BYTES) {
        Ok(bytes) => Ok(Some(parse(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
/// Explicit recovery under the same owner/profile lock as create/add. Ordinary
/// metadata reads remain read-only. Never removes credentials, including after
/// a rename/fsync uncertainty; conflicting state fails closed for inspection.
pub fn recover_pending(path: &Path) -> Result<Option<ReceiverProfile>> {
    let Some(pending) = read_pending(path)? else {
        return Ok(None);
    };
    let receiver = pending.receiver()?;
    let current = existing(path)?;
    let committed = current
        .as_ref()
        .is_some_and(|p| same_profile(p, &pending.next));
    let unchanged = match (&current, &pending.previous) {
        (None, None) => true,
        (Some(current), Some(previous)) => same_profile(current, previous),
        _ => false,
    };
    if !committed && !unchanged {
        return Err("airplay_profile_journal_conflict".into());
    }
    let store = FileCredentialStore::for_profile(path)?;
    if !committed {
        if let Some(previous) = &pending.previous {
            validate_secrets(path, previous)?;
        }
        match store.get(&receiver.key_reference, SecretKind::AirplayReceiverKey) {
            Ok(_) => validate_secret(&store, &receiver.key_reference)?,
            Err(error) if error.to_string() == "credential_missing" => {
                let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?.serialize_pem();
                store.put_reference(
                    &receiver.key_reference,
                    SecretKind::AirplayReceiverKey,
                    &key,
                )?;
                validate_secret(&store, &receiver.key_reference)?;
            }
            Err(error) => return Err(error),
        }
        checkpoint(path, "secret");
        // The caller's owner/profile lock protects the verified preimage.
        // Atomic replacement also protects first creation from a torn profile.
        write_profile(path, &pending.next)?;
        checkpoint(path, "profile");
    }
    // A published identity with a missing/corrupt key is never regenerated.
    validate_secrets(path, &pending.next)?;
    let actual = load(path)?;
    if !same_profile(&actual, &pending.next) {
        return Err("airplay_profile_journal_conflict".into());
    }
    std::fs::remove_file(pending_path(path))?;
    files::sync_parent(path)?;
    Ok(Some(actual))
}
#[cfg(not(test))]
fn checkpoint(_: &Path, _: &str) {}
#[cfg(test)]
fn checkpoint(path: &Path, phase: &str) {
    if std::env::var("NEONMIX_AIRPLAY_CRASH_PHASE").ok().as_deref() == Some(phase) {
        files::write_new(&path.with_extension("crash-ready"), phase.as_bytes()).unwrap();
        loop {
            std::thread::park();
        }
    }
}
/// Exclusive first creation. A matching pending creation is resumed with its
/// journaled identity/reference; a different request never replaces it.
pub fn create(path: &Path, receiver_uuid: Uuid, name: &str) -> Result<ReceiverProfile> {
    if let Some(pending) = read_pending(path)? {
        let receiver = pending.receiver()?;
        if pending.previous.is_some()
            || receiver.receiver_uuid != receiver_uuid
            || receiver.name != name
        {
            return Err("airplay_profile_operation_pending".into());
        }
        return recover_pending(path)?.ok_or_else(|| "airplay_profile_operation_pending".into());
    }
    if std::fs::symlink_metadata(path).is_ok() {
        return Err("credential_already_exists".into());
    }
    if receiver_uuid.is_nil() || !name_valid(name, 50) {
        return Err("airplay_invalid_identity".into());
    }
    let store = FileCredentialStore::for_profile(path)?;
    match std::fs::symlink_metadata(store.identity()) {
        Ok(_) => {
            files::validate_private_dir(&store.identity())?;
            if std::fs::read_dir(store.identity())?
                .next()
                .transpose()?
                .is_some()
            {
                return Err("credential_profile_missing".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let reference = Uuid::new_v4().to_string();
    let profile = ReceiverProfile {
        config_revision: 1,
        version: 2,
        credential_store: CredentialStore::File,
        multi_receiver: false,
        capacity: 4,
        playback_allowed: true,
        receivers: vec![ReceiverEndpoint {
            receiver_id: receiver_uuid,
            receiver_uuid,
            device_id: crate::speaker::receiver_id(receiver_uuid),
            name: name.into(),
            key_reference: reference,
            enabled: true,
            default_playback_mode: PlaybackMode::LowLatency,
        }],
        sources: vec![],
        bindings: vec![],
    };
    let pending = PendingReceiver {
        version: 2,
        previous: None,
        next: profile,
        receiver_id: receiver_uuid,
    };
    publish_pending(path, &pending)?;
    recover_pending(path)?.ok_or_else(|| "airplay_profile_operation_pending".into())
}
/// Creates an independently random identity. Failures retain a private journal
/// for retry/restart, never delete a possibly-published secret, and never expose
/// a different identity on retry. Caller retains the owner/profile lock.
pub fn add_receiver(
    path: &Path,
    profile: &mut ReceiverProfile,
    name: &str,
    enabled: bool,
) -> Result<Uuid> {
    if let Some(pending) = read_pending(path)? {
        let receiver = pending.receiver()?;
        if pending.previous.is_none() || receiver.name != name || receiver.enabled != enabled {
            return Err("airplay_profile_operation_pending".into());
        }
        let id = receiver.receiver_id;
        *profile = recover_pending(path)?.ok_or("airplay_profile_operation_pending")?;
        return Ok(id);
    }
    profile.validate()?;
    if existing(path)?
        .as_ref()
        .is_none_or(|actual| !same_profile(actual, profile))
    {
        return Err("airplay_profile_changed".into());
    }
    if profile.receivers.len() >= MAX_RECEIVERS {
        return Err("airplay_receiver_capacity".into());
    }
    if !name_valid(name, 50) || profile.receivers.iter().any(|r| r.name == name) {
        return Err("airplay_invalid_name".into());
    }
    let receiver_uuid = loop {
        let id = Uuid::new_v4();
        if !profile.receivers.iter().any(|r| {
            r.receiver_uuid == id
                || r.receiver_id == id
                || r.device_id == crate::speaker::receiver_id(id)
        }) {
            break id;
        }
    };
    let mut next = profile.clone();
    next.receivers.push(ReceiverEndpoint {
        receiver_id: receiver_uuid,
        receiver_uuid,
        device_id: crate::speaker::receiver_id(receiver_uuid),
        name: name.into(),
        key_reference: Uuid::new_v4().to_string(),
        enabled,
        default_playback_mode: PlaybackMode::LowLatency,
    });
    profile.prepare_change(&mut next)?;
    let pending = PendingReceiver {
        version: 2,
        previous: Some(profile.clone()),
        next,
        receiver_id: receiver_uuid,
    };
    publish_pending(path, &pending)?;
    *profile = recover_pending(path)?.ok_or("airplay_profile_operation_pending")?;
    Ok(receiver_uuid)
}
/// Export current trust, including every global revoke. The v2 original must
/// remain private and untouched; callers write this to a separate destination.
pub fn export_v1(profile: &ReceiverProfile, receiver_id: Uuid) -> Result<AirPlayProfile> {
    profile.validate()?;
    let receiver = profile
        .receivers
        .iter()
        .find(|r| r.receiver_id == receiver_id)
        .ok_or("receiver_unknown")?;
    let (known_keys, blocked_keys) = profile.trust_for(receiver_id)?;
    Ok(AirPlayProfile {
        version: 1,
        credential_store: CredentialStore::File,
        key_reference: Some(receiver.key_reference.clone()),
        known_keys,
        blocked_keys,
        playback_allowed: profile.playback_allowed,
        playback_mode: receiver.default_playback_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/tmp")
                .join(format!("airplay-profile-{}", Uuid::new_v4()));
            files::private_dir(&root).unwrap();
            Self(root)
        }
        fn path(&self) -> std::path::PathBuf {
            self.0.join("receiver.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn public(value: u8) -> String {
        STANDARD.encode([value; 32])
    }
    #[test]
    fn imported_profile_and_identity_staging_keep_monotonic_config_without_repeated_sync_bumps() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let original = create(&fixture.path(), id, "Receiver").unwrap();
        let mut old = serde_json::to_value(&original).unwrap();
        old.as_object_mut().unwrap().remove("config_revision");
        let imported = parse(&serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(imported.config_revision, 1);
        assert_eq!(
            imported.receivers[0].key_reference,
            original.receivers[0].key_reference
        );
        let mut staged = original.clone();
        add_receiver(&fixture.path(), &mut staged, "Input 2", false).unwrap();
        assert_eq!(staged.config_revision, 2);
        let mut next = staged.clone();
        next.multi_receiver = true;
        next.receivers[1].enabled = true;
        staged.prepare_change(&mut next).unwrap();
        assert_eq!(next.config_revision, 3);
        staged.prepare_change(&mut next).unwrap();
        assert_eq!(next.config_revision, 3);
        save(&fixture.path(), &next).unwrap();
        assert_eq!(load(&fixture.path()).unwrap().config_revision, 3);
        let mut exhausted = next.clone();
        exhausted.config_revision = u64::MAX;
        let mut changed = exhausted.clone();
        changed.receivers[0].name = "changed".into();
        assert!(exhausted.prepare_change(&mut changed).is_err());
        assert_eq!(changed.config_revision, u64::MAX);
    }

    #[test]
    fn migration_preserves_identity_trust_and_rollback_uses_current_revocations() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let mut profile = create(&fixture.path(), id, "Original name").unwrap();
        let original_ref = profile.receivers[0].key_reference.clone();
        profile.register_source(id, &public(1), None).unwrap();
        profile.register_source(id, &public(2), None).unwrap();
        profile
            .revoke_source(&source_id(&public(2)).unwrap())
            .unwrap();
        let mut old = export_v1(&profile, id).unwrap();
        old.playback_mode = PlaybackMode::Synchronized;
        files::replace(&fixture.path(), &serde_json::to_vec(&old).unwrap()).unwrap();
        let mut migrated = migrate_v1(&fixture.path(), id, "Original name").unwrap();
        assert_eq!(migrated.receivers[0].receiver_uuid, id);
        assert_eq!(migrated.receivers[0].key_reference, original_ref);
        assert_eq!(migrated.receivers[0].name, "Original name");
        assert_eq!(
            migrated.receivers[0].default_playback_mode,
            PlaybackMode::Synchronized
        );
        assert!(!migrated.multi_receiver);
        let other = add_receiver(&fixture.path(), &mut migrated, "Input 2", false).unwrap();
        assert!(!migrated.receivers[1].enabled);
        let store = FileCredentialStore::for_profile(&fixture.path()).unwrap();
        assert_ne!(
            store
                .get(&original_ref, SecretKind::AirplayReceiverKey)
                .unwrap()
                .expose(),
            store
                .get(
                    &migrated.receivers[1].key_reference,
                    SecretKind::AirplayReceiverKey
                )
                .unwrap()
                .expose()
        );
        assert!(migrated.trust_for(other).unwrap().0.is_empty());
        assert!(migrated.register_source(other, &public(2), None).is_err());
        migrated.register_source(other, &public(3), None).unwrap();
        migrated
            .revoke_source(&source_id(&public(3)).unwrap())
            .unwrap();
        let rollback = export_v1(&migrated, id).unwrap();
        assert!(rollback.blocked_keys.contains(&public(3)));
        assert!(rollback.known_keys.contains(&public(1)));
    }
    #[test]
    fn corrupt_missing_secret_unknown_version_and_duplicate_identity_never_reset() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let profile = create(&fixture.path(), id, "Receiver").unwrap();
        assert!(create(&fixture.path(), id, "Replacement").is_err());
        let mut duplicate = profile.clone();
        duplicate.receivers.push(profile.receivers[0].clone());
        assert!(save(&fixture.path(), &duplicate).is_err());
        assert_eq!(load(&fixture.path()).unwrap().receivers.len(), 1);
        let old = export_v1(&profile, id).unwrap();
        let bytes = serde_json::to_vec(&old).unwrap();
        files::replace(&fixture.path(), &bytes).unwrap();
        FileCredentialStore::for_profile(&fixture.path())
            .unwrap()
            .remove(
                &profile.receivers[0].key_reference,
                SecretKind::AirplayReceiverKey,
            )
            .unwrap();
        assert!(migrate_v1(&fixture.path(), id, "Receiver").is_err());
        assert_eq!(
            files::read_private(&fixture.path(), MAX_PROFILE_BYTES).unwrap(),
            bytes
        );
        let mut unknown = profile.clone();
        unknown.version = 3;
        assert!(parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
        assert!(source_id("not a public key").is_err());
    }
    #[test]
    fn capacity_counts_revocations_and_preserves_independent_preferences() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let mut profile = create(&fixture.path(), id, "Receiver").unwrap();
        for value in 0..64 {
            profile.register_source(id, &public(value), None).unwrap();
        }
        let first = source_id(&public(0)).unwrap();
        profile.revoke_source(&first).unwrap();
        assert!(profile.register_source(id, &public(64), None).is_err());
        profile.sources[1].gain_db = -6.0;
        profile.sources[1].muted = true;
        profile.sources[1].playback_mode = Some(PlaybackMode::Synchronized);
        save(&fixture.path(), &profile).unwrap();
        let loaded = load(&fixture.path()).unwrap();
        assert!(loaded.sources[0].revoked);
        assert_eq!(loaded.sources[1].gain_db, -6.0);
        assert_eq!(loaded.sources[2].gain_db, 0.0);
    }
    #[test]
    fn legacy_global_denial_becomes_recoverable_per_source_denial() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let mut profile = create(&fixture.path(), id, "Receiver").unwrap();
        let known = profile.register_source(id, &public(1), None).unwrap();
        let revoked = profile.register_source(id, &public(2), None).unwrap();
        profile.revoke_source(&revoked).unwrap();
        let mut old = export_v1(&profile, id).unwrap();
        old.playback_allowed = false;
        files::replace(&fixture.path(), &serde_json::to_vec(&old).unwrap()).unwrap();
        let mut migrated = migrate_v1(&fixture.path(), id, "Receiver").unwrap();
        assert!(migrated.playback_allowed);
        assert!(migrated.sources.iter().all(|s| s.blocked));
        assert!(migrated.trust_for(id).unwrap().0.is_empty());
        migrated
            .sources
            .iter_mut()
            .find(|s| s.source_id == known)
            .unwrap()
            .blocked = false;
        assert!(migrated.trust_for(id).unwrap().0.contains(&public(1)));
        assert!(
            migrated
                .sources
                .iter()
                .find(|s| s.source_id == revoked)
                .unwrap()
                .revoked
        );
        assert!(migrated.trust_for(id).unwrap().1.contains(&public(2)));
    }
    #[test]
    fn crash_child_helper() {
        let Some(path) = std::env::var_os("NEONMIX_AIRPLAY_CRASH_PATH") else {
            return;
        };
        let path = Path::new(&path);
        let id = Uuid::parse_str(&std::env::var("NEONMIX_AIRPLAY_CRASH_UUID").unwrap()).unwrap();
        if std::env::var("NEONMIX_AIRPLAY_CRASH_OPERATION").unwrap() == "create" {
            create(path, id, "Receiver").unwrap();
        } else {
            let mut profile = load(path).unwrap();
            add_receiver(path, &mut profile, "Input 2", false).unwrap();
        }
    }
    fn killed_operation(fixture: &Fixture, id: Uuid, operation: &str, phase: &str) {
        use std::process::{Command, Stdio};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "airplay_profile::tests::crash_child_helper"])
            .env("NEONMIX_AIRPLAY_CRASH_PATH", fixture.path())
            .env("NEONMIX_AIRPLAY_CRASH_UUID", id.to_string())
            .env("NEONMIX_AIRPLAY_CRASH_OPERATION", operation)
            .env("NEONMIX_AIRPLAY_CRASH_PHASE", phase)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !fixture.path().with_extension("crash-ready").exists()
            && std::time::Instant::now() < deadline
        {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before crash point: {status}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let ready = fixture.path().with_extension("crash-ready").exists();
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(ready, "child never reached crash checkpoint");
    }
    #[test]
    fn killed_create_and_add_recover_the_same_identity_without_orphan_secrets() {
        for operation in ["create", "add"] {
            for phase in ["journal", "secret", "profile"] {
                let fixture = Fixture::new();
                let id = Uuid::new_v4();
                let before = if operation == "add" {
                    Some(create(&fixture.path(), id, "Receiver").unwrap())
                } else {
                    None
                };
                killed_operation(&fixture, id, operation, phase);
                let pending = read_pending(&fixture.path()).unwrap().unwrap();
                let receiver = pending.receiver().unwrap().clone();
                let store = FileCredentialStore::for_profile(&fixture.path()).unwrap();
                let secret_before = store
                    .get(&receiver.key_reference, SecretKind::AirplayReceiverKey)
                    .ok()
                    .map(|s| crate::digest(s.expose()));
                let recovered = if operation == "create" {
                    create(&fixture.path(), id, "Receiver").unwrap()
                } else if phase == "secret" {
                    // A fresh Hub startup also finishes the journal while stopped.
                    migrate_v1(&fixture.path(), id, "Receiver").unwrap()
                } else {
                    let mut original = before.unwrap();
                    assert_eq!(
                        add_receiver(&fixture.path(), &mut original, "Input 2", false).unwrap(),
                        receiver.receiver_id
                    );
                    original
                };
                assert!(same_profile(&recovered, &pending.next));
                assert_eq!(
                    recovered.receivers.last().unwrap().key_reference,
                    receiver.key_reference
                );
                if let Some(digest) = secret_before {
                    assert_eq!(
                        crate::digest(
                            store
                                .get(&receiver.key_reference, SecretKind::AirplayReceiverKey)
                                .unwrap()
                                .expose()
                        ),
                        digest
                    );
                }
                assert!(!pending_path(&fixture.path()).exists());
                assert_eq!(
                    std::fs::read_dir(store.identity()).unwrap().count(),
                    recovered.receivers.len()
                );
                assert!(recover_pending(&fixture.path()).unwrap().is_none());
            }
        }
    }
    #[test]
    fn recovery_rejects_conflicts_and_never_replaces_a_published_missing_secret() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let original = create(&fixture.path(), id, "Receiver").unwrap();
        killed_operation(&fixture, id, "add", "secret");
        let pending = read_pending(&fixture.path()).unwrap().unwrap();
        assert!(save(&fixture.path(), &original).is_err());
        let mut changed = original.clone();
        changed.playback_allowed = false;
        write_profile(&fixture.path(), &changed).unwrap();
        assert_eq!(
            recover_pending(&fixture.path()).err().unwrap().to_string(),
            "airplay_profile_journal_conflict"
        );
        assert!(pending_path(&fixture.path()).exists());
        write_profile(&fixture.path(), &pending.next).unwrap();
        let store = FileCredentialStore::for_profile(&fixture.path()).unwrap();
        let reference = &pending.receiver().unwrap().key_reference;
        store
            .remove(reference, SecretKind::AirplayReceiverKey)
            .unwrap();
        assert!(recover_pending(&fixture.path()).is_err());
        assert!(
            store
                .get(reference, SecretKind::AirplayReceiverKey)
                .is_err()
        );
        assert!(pending_path(&fixture.path()).exists());
    }
    #[test]
    fn missing_profile_with_existing_credentials_never_creates_a_new_identity() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4();
        let profile = create(&fixture.path(), id, "Receiver").unwrap();
        let store = FileCredentialStore::for_profile(&fixture.path()).unwrap();
        let reference = &profile.receivers[0].key_reference;
        let digest = crate::digest(
            store
                .get(reference, SecretKind::AirplayReceiverKey)
                .unwrap()
                .expose(),
        );
        std::fs::remove_file(fixture.path()).unwrap();
        assert_eq!(
            create(&fixture.path(), Uuid::new_v4(), "New receiver")
                .err()
                .unwrap()
                .to_string(),
            "credential_profile_missing"
        );
        assert!(!fixture.path().exists());
        assert!(!pending_path(&fixture.path()).exists());
        assert_eq!(std::fs::read_dir(store.identity()).unwrap().count(), 1);
        assert_eq!(
            crate::digest(
                store
                    .get(reference, SecretKind::AirplayReceiverKey)
                    .unwrap()
                    .expose()
            ),
            digest
        );
    }
}
