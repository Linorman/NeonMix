//! Offline copy migration. Native reads are an injected boundary, never a product backend.
use neonmix_control::{Authority, PersistentState, Role};
use neonmix_identity::{
    files,
    store::{FileCredentialStore, SecretKind},
};
use neonmix_output_binding::OutputBinding;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const PROFILE_LIMIT: usize = 1024 * 1024;
const STATE_LIMIT: usize = 8 * 1024 * 1024;
const MARKER: &str = ".migration-owner.json";

#[derive(Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum Layout {
    Desktop,
    Hub,
    Profiles,
}
#[derive(Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum ProfileKind {
    Admin,
    Member,
}
impl ProfileKind {
    fn label(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
    fn secret(self) -> SecretKind {
        match self {
            Self::Admin => SecretKind::AdminToken,
            Self::Member => SecretKind::MemberToken,
        }
    }
}
pub struct Options {
    pub layout: Layout,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub profile_kind: Option<ProfileKind>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error(pub &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|_| Error("migration_io_failed"))
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| Error("migration_invalid_source"))
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|_| Error("migration_invalid_source"))
}
/// Reader errors must be stable codes, with no native error text or secret value.
pub trait LegacyReader {
    fn read(&mut self, reference: &str) -> Result<String>;
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    NativeRead,
    SecretWrite,
    ProfileWrite,
    Validate,
    Publish,
}
pub trait Hooks {
    fn checkpoint(&mut self, _: Phase) -> Result<()> {
        Ok(())
    }
}
pub struct NoHooks;
impl Hooks for NoHooks {}
#[derive(Serialize)]
pub struct Report {
    pub event: &'static str,
    pub operation: Uuid,
    pub destination: PathBuf,
    pub profiles: usize,
    pub secrets: usize,
    pub identity_preserved: bool,
    pub source_unchanged: bool,
    pub native_entries_retained: bool,
    pub offline_required: bool,
    pub rollback: &'static str,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OldHub {
    version: u16,
    room_name: String,
    state_path: PathBuf,
    output: String,
    certificate: String,
    private_key_ref: String,
    admin_token_ref: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OldToken {
    version: u16,
    hub_id: Uuid,
    certificate: String,
    secret_ref: String,
    request_id: Uuid,
    name: String,
    pending: bool,
    invitation_id: Option<Uuid>,
    device_id: Option<Uuid>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OldReceiver {
    key_reference: Option<String>,
    known_keys: BTreeSet<String>,
    blocked_keys: BTreeSet<String>,
    playback_allowed: bool,
    #[serde(default = "default_mode")]
    playback_mode: String,
}
fn default_mode() -> String {
    "low_latency".into()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    version: u16,
    operation: Uuid,
    destination: String,
}
struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn regular_directory(path: &Path) -> Result<()> {
    let meta = io(fs::symlink_metadata(path))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Error("migration_unsafe_path"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error("migration_unsafe_path"));
        }
    }
    Ok(())
}
fn source_paths(options: &Options) -> Result<(PathBuf, PathBuf)> {
    regular_directory(&options.source)?;
    let source = io(options.source.canonicalize())?;
    let name = options
        .destination
        .file_name()
        .ok_or(Error("migration_unsafe_path"))?;
    let parent = options
        .destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    regular_directory(parent)?;
    let destination = io(parent.canonicalize())?.join(name);
    if destination == source || destination.starts_with(&source) || source.starts_with(&destination)
    {
        return Err(Error("migration_overlapping_paths"));
    }
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(Error("migration_destination_exists"));
    }
    Ok((source, destination))
}
fn ephemeral(relative: &Path) -> bool {
    let name = relative.file_name().and_then(|v| v.to_str()).unwrap_or("");
    // Manual-session binaries and the receiver's known registry are runtime
    // assets, rebuilt/copied separately when switching to the migrated state.
    matches!(relative.to_str(), Some("bin" | "hub/airplay/gstreamer-registry.bin" | "airplay/gstreamer-registry.bin")) || relative.components().any(
        |c| matches!(c, Component::Normal(v) if v == "commands" || v == "invitations" || v == "logs" || v == "cache"),
    ) || name.ends_with(".lock")
        || name.ends_with(".pid")
        || name.ends_with(".sock")
        || name.ends_with(".log")
        || name.starts_with("runtime-key-")
        || name.starts_with("invite-")
        || name.starts_with(".neonmix-") && name.ends_with(".tmp")
        || name == "diagnostics-redacted.json"
}
fn allowed_directory(layout: Layout, relative: &Path, state: Option<&Path>) -> bool {
    if state.is_some_and(|s| s.starts_with(relative)) {
        return true;
    }
    match layout {
        Layout::Desktop => matches!(
            relative.to_str(),
            Some("hub" | "hub/airplay" | "profiles" | "output" | "outputs" | "outputs/main")
        ),
        Layout::Hub => relative == Path::new("airplay"),
        Layout::Profiles => false,
    }
}
fn profile_file(layout: Layout, relative: &Path) -> bool {
    let in_profiles = match layout {
        Layout::Desktop => relative.parent() == Some(Path::new("profiles")),
        Layout::Profiles => relative.parent() == Some(Path::new("")),
        Layout::Hub => false,
    };
    in_profiles && relative.extension().is_some_and(|e| e == "json")
}
fn persistent_file(layout: Layout, relative: &Path, state: Option<&Path>) -> bool {
    if state == Some(relative) || profile_file(layout, relative) {
        return true;
    }
    match layout {
        Layout::Desktop => matches!(
            relative.to_str(),
            Some(
                "hub/server.json"
                    | "hub/admin.json"
                    | "hub/airplay/receiver.json"
                    | "output/binding.json"
                    | "outputs/main/binding.json"
            )
        ),
        Layout::Hub => matches!(
            relative.to_str(),
            Some("server.json" | "admin.json" | "airplay/receiver.json")
        ),
        Layout::Profiles => false,
    }
}
fn snapshot(
    source: &Path,
    layout: Layout,
    state: Option<&Path>,
) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn walk(
        source: &Path,
        relative: &Path,
        layout: Layout,
        state: Option<&Path>,
        out: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<()> {
        for entry in io(fs::read_dir(source.join(relative)))? {
            let entry = io(entry)?;
            let path = relative.join(entry.file_name());
            if ephemeral(&path) {
                continue;
            }
            let meta = io(fs::symlink_metadata(entry.path()))?;
            if meta.file_type().is_symlink() {
                return Err(Error("migration_unsafe_path"));
            }
            if meta.is_dir() && allowed_directory(layout, &path, state) {
                regular_directory(&entry.path())?;
                walk(source, &path, layout, state, out)?;
            } else if meta.is_file() && persistent_file(layout, &path, state) {
                out.insert(path, io(files::read_private(&entry.path(), STATE_LIMIT))?);
            } else {
                return Err(Error("migration_unsupported_file"));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(source, Path::new(""), layout, state, &mut out)?;
    Ok(out)
}
fn mkdir_tree(stage: &Path, relative: &Path) -> Result<()> {
    let mut directory = stage.to_owned();
    for part in relative.components() {
        if let Component::Normal(part) = part {
            directory.push(part);
            io(files::private_dir(&directory))?;
        } else {
            return Err(Error("migration_unsafe_path"));
        }
    }
    Ok(())
}
fn canonical_reference(reference: &str) -> Result<()> {
    if Uuid::parse_str(reference)
        .ok()
        .is_none_or(|id| id.is_nil() || id.to_string() != reference)
    {
        return Err(Error("migration_invalid_reference"));
    }
    Ok(())
}
fn validate_token(profile: &OldToken) -> Result<()> {
    canonical_reference(&profile.secret_ref)?;
    if profile.version != 1
        || profile.hub_id.is_nil()
        || profile.request_id.is_nil()
        || profile.name.trim().is_empty()
        || profile.name.len() > 128
        || profile.name.chars().any(char::is_control)
        || profile.pending && profile.invitation_id.is_none()
        || !profile.pending && profile.device_id.is_none()
    {
        return Err(Error("migration_invalid_profile"));
    }
    neonmix_identity::trust::tls(&profile.certificate)
        .map_err(|_| Error("migration_invalid_certificate"))?;
    Ok(())
}
fn validate_tls(certificate: &str, key: &str) -> Result<()> {
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(certificate))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| Error("migration_invalid_certificate"))?;
    if certs.len() != 1 {
        return Err(Error("migration_invalid_certificate"));
    }
    let key = rustls_pemfile::private_key(&mut std::io::Cursor::new(key))
        .map_err(|_| Error("migration_invalid_tls_key"))?
        .ok_or(Error("migration_invalid_tls_key"))?;
    let provider = rustls::crypto::ring::default_provider();
    rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| Error("migration_invalid_tls_key"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|_| Error("migration_tls_key_mismatch"))?;
    Ok(())
}
fn validate_airplay(key: &str) -> Result<()> {
    let pair = rcgen::KeyPair::from_pem(key).map_err(|_| Error("migration_invalid_airplay_key"))?;
    if pair.algorithm() != &rcgen::PKCS_ED25519 {
        return Err(Error("migration_invalid_airplay_key"));
    }
    Ok(())
}
fn authority(bytes: &[u8]) -> Result<Authority> {
    let value: Value = parse(bytes)?;
    let fields = [
        "version",
        "hub_id",
        "revision",
        "devices",
        "credentials",
        "preferences",
        "output",
    ];
    if value
        .as_object()
        .is_none_or(|o| o.keys().any(|k| !fields.contains(&k.as_str())))
    {
        return Err(Error("migration_invalid_state"));
    }
    Authority::restore(parse::<PersistentState>(bytes)?)
        .map_err(|_| Error("migration_invalid_state"))
}
fn json_with_format<T: Serialize>(
    old: &T,
    version: u16,
    kind: Option<ProfileKind>,
) -> Result<Vec<u8>> {
    let mut value = serde_json::to_value(old).map_err(|_| Error("migration_invalid_profile"))?;
    value["version"] = json!(version);
    value["credential_store"] = json!("file");
    if let Some(kind) = kind {
        value["profile_kind"] = json!(kind.label());
    }
    encode(&value)
}
fn write_profile(
    stage: &Path,
    relative: &Path,
    bytes: &[u8],
    hooks: &mut impl Hooks,
) -> Result<()> {
    mkdir_tree(stage, relative.parent().unwrap_or(Path::new("")))?;
    hooks.checkpoint(Phase::ProfileWrite)?;
    io(files::write_new(&stage.join(relative), bytes))
}

pub fn migrate<R: LegacyReader, H: Hooks>(
    options: &Options,
    reader: &mut R,
    hooks: &mut H,
) -> Result<Report> {
    if options.layout == Layout::Profiles && options.profile_kind.is_none()
        || options.layout != Layout::Profiles && options.profile_kind.is_some()
    {
        return Err(Error("migration_profile_kind_required"));
    }
    let (source, destination) = source_paths(options)?;
    // These stable locks coordinate current background/Hub/binding owners. Old CLI
    // pairing writers do not honor them: migration must still be run offline.
    let mut locks = Vec::new();
    if options.layout == Layout::Desktop {
        locks.push(
            files::lock(&source.join("background.lock"))
                .map_err(|_| Error("migration_source_busy"))?,
        );
        #[cfg(unix)]
        if std::os::unix::net::UnixStream::connect(source.join("ipc.sock")).is_ok() {
            return Err(Error("migration_source_busy"));
        }
    }
    let hub_dir = match options.layout {
        Layout::Desktop => Some(PathBuf::from("hub")),
        Layout::Hub => Some(PathBuf::new()),
        Layout::Profiles => None,
    };
    let mut old_hub = None;
    let mut hub_snapshot = None;
    let mut state_relative = None;
    if let Some(dir) = &hub_dir {
        let profile_path = source.join(dir).join("server.json");
        if profile_path.exists() {
            let bytes = io(files::read_private(&profile_path, PROFILE_LIMIT))?;
            let hub: OldHub = parse(&bytes)?;
            hub_snapshot = Some(bytes);
            if hub.version != 1
                || hub.output.is_empty()
                || hub.room_name.trim().is_empty()
                || hub.room_name.len() > 128
            {
                return Err(Error("migration_invalid_profile"));
            }
            canonical_reference(&hub.private_key_ref)?;
            canonical_reference(&hub.admin_token_ref)?;
            let state_path = if hub.state_path.is_absolute() {
                hub.state_path.clone()
            } else {
                source.join(dir).join(&hub.state_path)
            };
            let canonical = io(state_path.canonicalize())?;
            if !canonical.starts_with(&source) {
                return Err(Error("migration_external_state"));
            }
            // Every component is inspected without following links by snapshot.
            let relative = canonical
                .strip_prefix(&source)
                .map_err(|_| Error("migration_external_state"))?
                .to_owned();
            locks.push(
                files::lock(&state_path.with_extension("lock"))
                    .map_err(|_| Error("migration_source_busy"))?,
            );
            state_relative = Some(relative);
            old_hub = Some(hub);
        } else if options.layout == Layout::Hub {
            return Err(Error("migration_missing_hub"));
        }
    }
    if options.layout == Layout::Desktop {
        for directory in ["output", "outputs/main"] {
            if source.join(directory).is_dir() {
                locks.push(
                    files::lock(&source.join(directory).join("binding.lock"))
                        .map_err(|_| Error("migration_source_busy"))?,
                );
            }
        }
    }
    let initial = snapshot(&source, options.layout, state_relative.as_deref())?;
    if let (Some(dir), Some(bytes)) = (&hub_dir, &hub_snapshot)
        && initial.get(&dir.join("server.json")) != Some(bytes)
    {
        return Err(Error("migration_source_changed"));
    }
    if initial.is_empty() {
        return Err(Error("migration_empty_source"));
    }
    if let (Some(hub), Some(dir)) = (&old_hub, &hub_dir) {
        let admin: OldToken = parse(
            initial
                .get(&dir.join("admin.json"))
                .ok_or(Error("migration_missing_admin"))?,
        )?;
        if admin.secret_ref != hub.admin_token_ref
            || admin.certificate != hub.certificate
            || admin.pending
        {
            return Err(Error("migration_admin_mismatch"));
        }
    }
    for relative in initial.keys().filter(|p| profile_file(options.layout, p)) {
        locks.push(
            files::lock(&neonmix_identity::profiles::profile_lock_path(
                &source.join(relative),
            ))
            .map_err(|_| Error("migration_source_busy"))?,
        );
    }
    if initial != snapshot(&source, options.layout, state_relative.as_deref())? {
        return Err(Error("migration_source_changed"));
    }
    let operation = Uuid::new_v4();
    let stage = Stage(
        destination
            .parent()
            .ok_or(Error("migration_unsafe_path"))?
            .join(format!(".neonmix-migration-{operation}.staging")),
    );
    io(files::private_dir(&stage.0))?;
    io(files::write_new(
        &stage.0.join(MARKER),
        &encode(&Owner {
            version: 1,
            operation,
            destination: destination
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(Error("migration_unsafe_path"))?
                .into(),
        })?,
    ))?;
    let mut secrets: BTreeMap<String, String> = BTreeMap::new();
    let mut written: BTreeMap<(PathBuf, String), SecretKind> = BTreeMap::new();
    let mut store_secret =
        |relative: &Path, reference: &str, kind: SecretKind, hooks: &mut H| -> Result<()> {
            canonical_reference(reference)?;
            let store_key = (
                relative.parent().unwrap_or(Path::new("")).to_owned(),
                reference.to_owned(),
            );
            if let Some(previous) = written.get(&store_key) {
                if *previous != kind {
                    return Err(Error("migration_reference_kind_conflict"));
                }
                return Ok(());
            }
            if !secrets.contains_key(reference) {
                hooks.checkpoint(Phase::NativeRead)?;
                let secret = reader.read(reference)?;
                if secret.is_empty() || secret.len() > 64 * 1024 {
                    return Err(Error("migration_invalid_secret"));
                }
                secrets.insert(reference.to_owned(), secret);
            }
            mkdir_tree(&stage.0, relative.parent().unwrap_or(Path::new("")))?;
            let store = FileCredentialStore::for_profile(&stage.0.join(relative))
                .map_err(|_| Error("migration_credential_io_failed"))?;
            hooks.checkpoint(Phase::SecretWrite)?;
            store
                .put_reference(reference, kind, &secrets[reference])
                .map_err(|_| Error("migration_credential_io_failed"))?;
            let readback = store
                .get(reference, kind)
                .map_err(|_| Error("migration_credential_validation_failed"))?;
            if readback.expose() != secrets[reference] {
                return Err(Error("migration_credential_validation_failed"));
            }
            written.insert(store_key, kind);
            Ok(())
        };
    let mut profiles = 0;
    let saved_authority = if let Some(relative) = &state_relative {
        Some(authority(
            initial
                .get(relative)
                .ok_or(Error("migration_invalid_state"))?,
        )?)
    } else {
        None
    };
    let mut admin_device = None;
    if let (Some(hub), Some(dir)) = (&mut old_hub, &hub_dir) {
        let server = dir.join("server.json");
        store_secret(&server, &hub.private_key_ref, SecretKind::HubTlsKey, hooks)?;
        store_secret(&server, &hub.admin_token_ref, SecretKind::AdminToken, hooks)?;
        let store = FileCredentialStore::for_profile(&stage.0.join(&server))
            .map_err(|_| Error("migration_credential_io_failed"))?;
        validate_tls(
            &hub.certificate,
            store
                .get(&hub.private_key_ref, SecretKind::HubTlsKey)
                .map_err(|_| Error("migration_credential_validation_failed"))?
                .expose(),
        )?;
        let token = store
            .get(&hub.admin_token_ref, SecretKind::AdminToken)
            .map_err(|_| Error("migration_credential_validation_failed"))?;
        let auth = saved_authority
            .as_ref()
            .ok_or(Error("migration_invalid_state"))?;
        if hub.output != auth.current().output.id {
            return Err(Error("migration_output_mismatch"));
        }
        let principal = auth
            .authenticate(token.expose())
            .map_err(|_| Error("migration_admin_mismatch"))?;
        let device = auth
            .current()
            .devices
            .get(&principal.device_id())
            .ok_or(Error("migration_admin_mismatch"))?;
        if device.role != Role::Admin {
            return Err(Error("migration_admin_mismatch"));
        }
        admin_device = Some(principal.device_id());
        hub.state_path = "state.json".into();
        write_profile(&stage.0, &server, &json_with_format(hub, 2, None)?, hooks)?;
        write_profile(
            &stage.0,
            &dir.join("state.json"),
            initial.get(state_relative.as_ref().unwrap()).unwrap(),
            hooks,
        )?;
        profiles += 1;
    }
    for (relative, bytes) in &initial {
        if state_relative.as_ref() == Some(relative)
            || hub_dir
                .as_ref()
                .is_some_and(|d| relative == &d.join("server.json"))
        {
            continue;
        }
        if options.layout == Layout::Desktop
            && matches!(
                relative.to_str(),
                Some("output/binding.json" | "outputs/main/binding.json")
            )
        {
            let binding: OutputBinding = parse(bytes)?;
            binding
                .validate()
                .map_err(|_| Error("migration_invalid_binding"))?;
            write_profile(&stage.0, relative, bytes, hooks)?;
        } else if hub_dir
            .as_ref()
            .is_some_and(|dir| relative == &dir.join("airplay/receiver.json"))
        {
            let receiver: OldReceiver = parse(bytes)?;
            if !matches!(
                receiver.playback_mode.as_str(),
                "low_latency" | "synchronized"
            ) {
                return Err(Error("migration_invalid_airplay_profile"));
            }
            let reference = receiver
                .key_reference
                .as_ref()
                .ok_or(Error("migration_missing_airplay_key"))?;
            store_secret(relative, reference, SecretKind::AirplayReceiverKey, hooks)?;
            let store = FileCredentialStore::for_profile(&stage.0.join(relative))
                .map_err(|_| Error("migration_credential_io_failed"))?;
            validate_airplay(
                store
                    .get(reference, SecretKind::AirplayReceiverKey)
                    .map_err(|_| Error("migration_credential_validation_failed"))?
                    .expose(),
            )?;
            write_profile(
                &stage.0,
                relative,
                &json_with_format(&receiver, 1, None)?,
                hooks,
            )?;
            profiles += 1;
        } else {
            let token: OldToken = parse(bytes)?;
            validate_token(&token)?;
            let kind = if let Some(hub) = &old_hub {
                let known_admin = saved_authority.as_ref().is_some_and(|auth| {
                    auth.current().hub_id == token.hub_id
                        && token
                            .device_id
                            .and_then(|id| auth.current().devices.get(&id))
                            .is_some_and(|device| device.role == Role::Admin)
                });
                if token.secret_ref == hub.admin_token_ref
                    || token.device_id == admin_device
                    || known_admin
                {
                    ProfileKind::Admin
                } else {
                    ProfileKind::Member
                }
            } else {
                options.profile_kind.unwrap_or(ProfileKind::Member)
            };
            if hub_dir
                .as_ref()
                .is_some_and(|d| relative == &d.join("admin.json"))
                && kind != ProfileKind::Admin
            {
                return Err(Error("migration_admin_mismatch"));
            }
            store_secret(relative, &token.secret_ref, kind.secret(), hooks)?;
            if kind == ProfileKind::Admin
                && let Some(auth) = &saved_authority
            {
                let store = FileCredentialStore::for_profile(&stage.0.join(relative))
                    .map_err(|_| Error("migration_credential_io_failed"))?;
                let secret = store
                    .get(&token.secret_ref, SecretKind::AdminToken)
                    .map_err(|_| Error("migration_credential_validation_failed"))?;
                let principal = auth
                    .authenticate(secret.expose())
                    .map_err(|_| Error("migration_admin_mismatch"))?;
                if auth.current().hub_id != token.hub_id
                    || token.device_id != Some(principal.device_id())
                    || auth.current().devices[&principal.device_id()].role != Role::Admin
                    || old_hub
                        .as_ref()
                        .is_some_and(|h| h.certificate != token.certificate)
                {
                    return Err(Error("migration_admin_mismatch"));
                }
            }
            write_profile(
                &stage.0,
                relative,
                &json_with_format(&token, 2, Some(kind))?,
                hooks,
            )?;
            profiles += 1;
        }
    }
    hooks.checkpoint(Phase::Validate)?;
    validate_destination(
        &stage.0,
        &initial,
        state_relative.as_deref(),
        hub_dir.as_deref(),
    )?;
    if initial != snapshot(&source, options.layout, state_relative.as_deref())? {
        return Err(Error("migration_source_changed"));
    }
    hooks.checkpoint(Phase::Publish)?;
    if initial != snapshot(&source, options.layout, state_relative.as_deref())? {
        return Err(Error("migration_source_changed"));
    }
    // Keep the non-secret ownership marker through publication. A process exit
    // immediately before rename must still leave an explicitly cleanable stage.
    files::publish_directory(&stage.0, &destination).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error("migration_destination_exists")
        } else {
            Error("migration_publish_failed")
        }
    })?;
    drop(locks);
    Ok(Report {
        event: "credential_migration_complete",
        operation,
        destination,
        profiles,
        secrets: written.len(),
        identity_preserved: true,
        source_unchanged: true,
        native_entries_retained: true,
        offline_required: true,
        rollback: "Stop the new instance before reverting. Revert to the source only before any new pairing, revocation or settings write; otherwise preserve and repair the destination.",
    })
}

fn validate_destination(
    stage: &Path,
    initial: &BTreeMap<PathBuf, Vec<u8>>,
    source_state: Option<&Path>,
    hub_dir: Option<&Path>,
) -> Result<()> {
    for relative in initial.keys() {
        if source_state == Some(relative) {
            continue;
        }
        let path = stage.join(relative);
        let bytes = io(files::read_private(&path, STATE_LIMIT))?;
        if matches!(
            relative.to_str(),
            Some("output/binding.json" | "outputs/main/binding.json")
        ) {
            parse::<OutputBinding>(&bytes)?
                .validate()
                .map_err(|_| Error("migration_invalid_binding"))?;
            if bytes != initial[relative] {
                return Err(Error("migration_identity_changed"));
            }
        } else {
            neonmix_identity::profiles::load_profile_metadata(&path)
                .map_err(|_| Error("migration_invalid_destination"))?;
            let value: Value = parse(&bytes)?;
            let store = FileCredentialStore::for_profile(&path)
                .map_err(|_| Error("migration_credential_io_failed"))?;
            for (field, kind) in [
                ("private_key_ref", SecretKind::HubTlsKey),
                ("admin_token_ref", SecretKind::AdminToken),
                ("key_reference", SecretKind::AirplayReceiverKey),
            ] {
                if let Some(reference) = value.get(field).and_then(Value::as_str) {
                    store
                        .get(reference, kind)
                        .map_err(|_| Error("migration_credential_validation_failed"))?;
                }
            }
            if let Some(reference) = value.get("secret_ref").and_then(Value::as_str) {
                let kind = if value["profile_kind"] == "admin" {
                    SecretKind::AdminToken
                } else {
                    SecretKind::MemberToken
                };
                store
                    .get(reference, kind)
                    .map_err(|_| Error("migration_credential_validation_failed"))?;
            }
        }
    }
    if let (Some(state), Some(dir)) = (source_state, hub_dir) {
        let bytes = io(files::read_private(
            &stage.join(dir).join("state.json"),
            STATE_LIMIT,
        ))?;
        authority(&bytes)?;
        if bytes != initial[state] {
            return Err(Error("migration_identity_changed"));
        }
    }
    Ok(())
}

/// Crash remnants require an explicit, owned staging path; never scan/delete siblings.
pub fn cleanup_staging(path: &Path) -> Result<()> {
    regular_directory(path)?;
    io(files::validate_private_dir(path))?;
    let owner: Owner = parse(&io(files::read_private(&path.join(MARKER), 4096))?)?;
    let expected = format!(".neonmix-migration-{}.staging", owner.operation);
    if owner.version != 1
        || path.file_name().and_then(|n| n.to_str()) != Some(&expected)
        || Path::new(&owner.destination).components().count() != 1
    {
        return Err(Error("migration_not_owned_staging"));
    }
    io(fs::remove_dir_all(path))
}
