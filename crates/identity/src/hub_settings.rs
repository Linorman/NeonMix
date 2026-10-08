//! Recoverable state/profile configuration commit. The private journal never
//! enters diagnostics; only these fixed, identity-checked targets are writable.
use crate::{
    files::{self, Publication, ReplaceStage},
    profiles::{self, HubProfile, ProductProfile},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;
const SCHEMA: u16 = 1;
const MAX_JOURNAL_BYTES: usize =
    4 * (profiles::MAX_STATE_BYTES + profiles::MAX_PROFILE_BYTES) + 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    CommitDecided,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Journal,
    State,
    Profile,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Write(Target, ReplaceStage),
    Prepared,
    CommitDecided,
    StatePublished,
    ProfilePublished,
    Verified,
    RemoveJournal,
    JournalRemoved,
    SyncRemoval,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NotCommitted,
    RecoveryRequired,
    DurabilityUnconfirmed,
}
#[derive(Debug)]
pub struct Error {
    pub outcome: Outcome,
    pub transaction_id: Option<Uuid>,
    pub code: &'static str,
    pub source: Option<io::Error>,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|error| error as _)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub transaction_id: Option<Uuid>,
    pub revision: u64,
    pub changed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u16,
    transaction_id: Uuid,
    phase: Phase,
    profile_name: String,
    state_relative: PathBuf,
    before_profile: String,
    before_state: String,
    after_profile: String,
    after_state: String,
    hashes: [String; 4],
    old_revision: u64,
    new_revision: u64,
}
fn error(code: &'static str) -> Error {
    Error {
        outcome: Outcome::NotCommitted,
        transaction_id: None,
        code,
        source: None,
    }
}
fn io_error(source: io::Error) -> Error {
    Error {
        source: Some(source),
        ..error("credential_io_failed")
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn parse_profile(bytes: &[u8]) -> Result<HubProfile, Error> {
    match profiles::parse_metadata(bytes).map_err(|_| error("credential_corrupt"))? {
        ProductProfile::Hub(profile) => Ok(profile),
        _ => Err(error("credential_kind_mismatch")),
    }
}
fn parse_state(bytes: &[u8]) -> Result<serde_json::Value, Error> {
    serde_json::from_slice(bytes).map_err(|_| error("credential_corrupt"))
}
fn checked_revision(state: &serde_json::Value) -> Result<u64, Error> {
    state["revision"]
        .as_u64()
        .ok_or_else(|| error("credential_corrupt"))
}
fn profile_name(path: &Path) -> Result<String, Error> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| error("credential_corrupt"))
}
pub fn journal_path(profile: &Path) -> Result<PathBuf, Error> {
    Ok(profile.with_file_name(format!(
        "{}.neonmix-transaction.json",
        profile_name(profile)?
    )))
}
fn parent(profile: &Path) -> Result<PathBuf, Error> {
    let directory = profile
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    files::validate_private_dir(directory).map_err(io_error)?;
    directory.canonicalize().map_err(io_error)
}
fn state_target(profile: &Path, metadata: &HubProfile) -> Result<PathBuf, Error> {
    profiles::state_path(profile, metadata).map_err(|_| error("credential_corrupt"))
}
fn bytes(journal: &Journal) -> [&[u8]; 4] {
    [
        journal.before_profile.as_bytes(),
        journal.before_state.as_bytes(),
        journal.after_profile.as_bytes(),
        journal.after_state.as_bytes(),
    ]
}
fn validate(
    journal: &Journal,
    profile: &Path,
    validate_state: &impl Fn(&[u8]) -> Result<(), String>,
) -> Result<HubProfile, Error> {
    if journal.schema != SCHEMA {
        return Err(error("credential_version_unsupported"));
    }
    if journal.transaction_id.is_nil()
        || journal.profile_name != profile_name(profile)?
        || journal.state_relative.as_os_str().is_empty()
        || journal
            .state_relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(error("credential_corrupt"));
    }
    for (index, content) in bytes(journal).into_iter().enumerate() {
        if content.len()
            > if index % 2 == 0 {
                profiles::MAX_PROFILE_BYTES
            } else {
                profiles::MAX_STATE_BYTES
            }
            || hash(content) != journal.hashes[index]
        {
            return Err(error("credential_corrupt"));
        }
    }
    let before = parse_profile(journal.before_profile.as_bytes())?;
    let after = parse_profile(journal.after_profile.as_bytes())?;
    if before.state_path != journal.state_relative {
        return Err(error("credential_corrupt"));
    }
    let old = parse_state(journal.before_state.as_bytes())?;
    let new = parse_state(journal.after_state.as_bytes())?;
    validate_state(journal.before_state.as_bytes()).map_err(|_| error("credential_corrupt"))?;
    validate_state(journal.after_state.as_bytes()).map_err(|_| error("credential_corrupt"))?;
    if checked_revision(&old)? != journal.old_revision
        || checked_revision(&new)? != journal.new_revision
        || journal.old_revision.checked_add(1) != Some(journal.new_revision)
        || old["output"]["id"] != before.output
        || new["output"]["id"] != after.output
    {
        return Err(error("credential_corrupt"));
    }
    let mut expected = parse_state(journal.before_state.as_bytes())?;
    expected["revision"] = serde_json::json!(journal.new_revision);
    if let Some(config) = old.get("config_revision") {
        let config = config.as_u64().ok_or_else(|| error("credential_corrupt"))?;
        expected["config_revision"] = serde_json::json!(
            config
                .checked_add(1)
                .ok_or_else(|| error("quota_exceeded"))?
        );
    }
    expected["output"]["id"] = serde_json::json!(after.output);
    let mut expected_profile =
        serde_json::to_value(&before).map_err(|_| error("credential_corrupt"))?;
    expected_profile["output"] = serde_json::json!(after.output);
    expected_profile["room_name"] = serde_json::json!(after.room_name);
    if expected != new
        || expected_profile
            != serde_json::to_value(&after).map_err(|_| error("credential_corrupt"))?
    {
        return Err(error("credential_corrupt"));
    }
    Ok(before)
}
fn prefix(profile: &Path) -> Result<String, Error> {
    Ok(format!(
        ".neonmix-config-{}-",
        &hash(profile_name(profile)?.as_bytes())[..16]
    ))
}
fn has_temps(profile: &Path) -> Result<bool, Error> {
    let root = profile.parent().unwrap_or(Path::new("."));
    let prefix = prefix(profile)?;
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(error)),
    };
    for entry in entries {
        if entry
            .map_err(io_error)?
            .file_name()
            .to_str()
            .is_some_and(|name| temporary_name(name, &prefix))
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn temporary_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_suffix(".tmp"))
        .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok_and(|id| id.to_string() == suffix))
}
fn cleanup_temps(profile: &Path, state: &Path) -> Result<(), Error> {
    let prefix = prefix(profile)?;
    let mut directories = vec![profile.parent().unwrap_or(Path::new(".")).to_path_buf()];
    let state_parent = state.parent().ok_or_else(|| error("credential_corrupt"))?;
    if state_parent != directories[0] {
        directories.push(state_parent.to_path_buf());
    }
    for path in directories {
        let directory = files::PrivateDirectory::open(&path).map_err(io_error)?;
        for entry in std::fs::read_dir(&path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let Some(name) = entry
                .file_name()
                .to_str()
                .filter(|name| temporary_name(name, &prefix))
                .map(str::to_owned)
            else {
                continue;
            };
            // Reject symlinks and non-private objects before unlinking; this
            // namespace belongs only to this profile/state locked coordinator.
            directory.metadata(&name).map_err(io_error)?;
            directory.remove(&name).map_err(io_error)?;
        }
    }
    Ok(())
}
fn write(
    target: Target,
    path: &Path,
    content: &[u8],
    scope: &Path,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<(), Error> {
    let temporary = format!("{}{}.tmp", prefix(scope)?, Uuid::new_v4());
    files::replace_scoped_with(path, content, &temporary, &mut |stage| {
        hook(Stage::Write(target, stage))
    })
    .map(|_| ())
    .map_err(|failure| Error {
        outcome: if failure.publication == Publication::NotPublished {
            Outcome::NotCommitted
        } else {
            Outcome::DurabilityUnconfirmed
        },
        source: Some(failure.source),
        code: if failure.publication == Publication::NotPublished {
            "credential_io_failed"
        } else {
            "profile_durability_unconfirmed"
        },
        transaction_id: None,
    })
}
fn checkpoint(stage: Stage, hook: &mut impl FnMut(Stage) -> io::Result<()>) -> Result<(), Error> {
    hook(stage).map_err(io_error)
}
fn finish(
    journal: &Journal,
    profile: &Path,
    state: &Path,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<(), Error> {
    let current_profile =
        files::read_private(profile, profiles::MAX_PROFILE_BYTES).map_err(io_error)?;
    let current_state = files::read_private(state, profiles::MAX_STATE_BYTES).map_err(io_error)?;
    if hash(&current_profile) != journal.hashes[2] || hash(&current_state) != journal.hashes[3] {
        return Err(error("credential_corrupt"));
    }
    checkpoint(Stage::Verified, hook)?;
    checkpoint(Stage::RemoveJournal, hook)?;
    let path = journal_path(profile)?;
    std::fs::remove_file(&path).map_err(io_error)?;
    checkpoint(Stage::JournalRemoved, hook)?;
    checkpoint(Stage::SyncRemoval, hook)?;
    files::sync_parent(&path).map_err(io_error)
}
fn forward(
    journal: &Journal,
    profile: &Path,
    state: &Path,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<(), Error> {
    for (path, limit, old, new) in [
        (profile, profiles::MAX_PROFILE_BYTES, 0, 2),
        (state, profiles::MAX_STATE_BYTES, 1, 3),
    ] {
        let current = files::read_private(path, limit).map_err(io_error)?;
        let digest = hash(&current);
        if digest != journal.hashes[old] && digest != journal.hashes[new] {
            return Err(error("configuration_transaction_conflict"));
        }
    }
    // Re-sync a recovered decision before publishing either target. Visibility
    // of the decision is not itself proof of its durable publication.
    let encoded = serde_json::to_vec(journal).map_err(|_| error("credential_corrupt"))?;
    write(
        Target::Journal,
        &journal_path(profile)?,
        &encoded,
        profile,
        hook,
    )?;
    write(
        Target::State,
        state,
        journal.after_state.as_bytes(),
        profile,
        hook,
    )?;
    checkpoint(Stage::StatePublished, hook)?;
    write(
        Target::Profile,
        profile,
        journal.after_profile.as_bytes(),
        profile,
        hook,
    )?;
    checkpoint(Stage::ProfilePublished, hook)?;
    finish(journal, profile, state, hook)
}
/// Recovery checks journal schema/checksums/identities before strict cross-file
/// configuration loading. No journal means no profile/state locking side effect.
pub fn recover(
    profile: &Path,
    validate_state: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<Option<Receipt>, Error> {
    recover_with(profile, &validate_state, &mut |_| Ok(()))
}
#[doc(hidden)]
pub fn recover_with(
    profile: &Path,
    validate_state: &impl Fn(&[u8]) -> Result<(), String>,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<Option<Receipt>, Error> {
    let path = journal_path(profile)?;
    if !path.try_exists().map_err(io_error)? && !has_temps(profile)? {
        return Ok(None);
    }
    let _profile_lock = profiles::operation_lock(&profiles::profile_lock_path(profile))
        .map_err(|_| error("credential_store_busy"))?;
    recover_locked(profile, validate_state, hook)
}
fn recover_locked(
    profile: &Path,
    validate_state: &impl Fn(&[u8]) -> Result<(), String>,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<Option<Receipt>, Error> {
    parent(profile)?;
    let path = journal_path(profile)?;
    if !path.try_exists().map_err(io_error)? {
        if has_temps(profile)? {
            let metadata = profiles::hub(profile).map_err(|_| error("credential_corrupt"))?;
            let state = state_target(profile, &metadata)?;
            let _state_lock = profiles::operation_lock(&state.with_extension("lock"))
                .map_err(|_| error("credential_store_busy"))?;
            cleanup_temps(profile, &state)?;
        }
        return Ok(None);
    }
    let journal: Journal =
        serde_json::from_slice(&files::read_private(&path, MAX_JOURNAL_BYTES).map_err(io_error)?)
            .map_err(|_| error("credential_corrupt"))?;
    let before = validate(&journal, profile, validate_state)?;
    let state = state_target(profile, &before)?;
    let _state_lock = profiles::operation_lock(&state.with_extension("lock"))
        .map_err(|_| error("credential_store_busy"))?;
    let result = if journal.phase == Phase::Prepared {
        if hash(&files::read_private(profile, profiles::MAX_PROFILE_BYTES).map_err(io_error)?)
            != journal.hashes[0]
            || hash(&files::read_private(&state, profiles::MAX_STATE_BYTES).map_err(io_error)?)
                != journal.hashes[1]
        {
            Err(error("configuration_transaction_conflict"))
        } else {
            std::fs::remove_file(&path)
                .map_err(io_error)
                .and_then(|_| files::sync_parent(&path).map_err(io_error))
        }
    } else {
        forward(&journal, profile, &state, hook)
    };
    result.map_err(|mut error| {
        error.transaction_id = Some(journal.transaction_id);
        error.outcome = Outcome::RecoveryRequired;
        error
    })?;
    cleanup_temps(profile, &state)?;
    Ok(Some(Receipt {
        transaction_id: Some(journal.transaction_id),
        revision: if journal.phase == Phase::Prepared {
            journal.old_revision
        } else {
            journal.new_revision
        },
        changed: journal.phase == Phase::CommitDecided,
    }))
}
pub fn update(
    profile: &Path,
    output: &str,
    name: &str,
    validate_state: impl Fn(&[u8]) -> Result<(), String>,
) -> Result<Receipt, Error> {
    update_with(profile, output, name, &validate_state, &mut |_| Ok(()))
}
#[doc(hidden)]
pub fn update_with(
    profile: &Path,
    output: &str,
    name: &str,
    validate_state: &impl Fn(&[u8]) -> Result<(), String>,
    hook: &mut impl FnMut(Stage) -> io::Result<()>,
) -> Result<Receipt, Error> {
    parent(profile)?;
    let _profile_lock = profiles::operation_lock(&profiles::profile_lock_path(profile))
        .map_err(|_| error("credential_store_busy"))?;
    let recovered = recover_locked(profile, validate_state, hook)?;
    let old_profile_bytes =
        files::read_private(profile, profiles::MAX_PROFILE_BYTES).map_err(io_error)?;
    let mut metadata = parse_profile(&old_profile_bytes)?;
    let state = state_target(profile, &metadata)?;
    let _state_lock = profiles::operation_lock(&state.with_extension("lock"))
        .map_err(|_| error("credential_store_busy"))?;
    let old_state_bytes =
        files::read_private(&state, profiles::MAX_STATE_BYTES).map_err(io_error)?;
    validate_state(&old_state_bytes).map_err(|_| error("credential_corrupt"))?;
    let mut saved = parse_state(&old_state_bytes)?;
    if saved["output"]["id"] != metadata.output {
        return Err(error("credential_corrupt"));
    }
    let old_revision = checked_revision(&saved)?;
    if metadata.output == output && metadata.room_name == name {
        // Confirm current contents, not a stale candidate, when retrying a
        // removal whose directory durability was previously unconfirmed.
        write(Target::State, &state, &old_state_bytes, profile, hook)?;
        write(Target::Profile, profile, &old_profile_bytes, profile, hook)?;
        return Ok(Receipt {
            transaction_id: recovered.and_then(|receipt| receipt.transaction_id),
            revision: old_revision,
            changed: false,
        });
    }
    let new_revision = old_revision
        .checked_add(1)
        .ok_or_else(|| error("quota_exceeded"))?;
    metadata.output = output.into();
    metadata.room_name = name.into();
    saved["output"]["id"] = serde_json::json!(output);
    saved["revision"] = serde_json::json!(new_revision);
    if let Some(config) = saved.get("config_revision") {
        let config = config.as_u64().ok_or_else(|| error("credential_corrupt"))?;
        saved["config_revision"] = serde_json::json!(
            config
                .checked_add(1)
                .ok_or_else(|| error("quota_exceeded"))?
        );
    }
    let after_profile =
        serde_json::to_string_pretty(&metadata).map_err(|_| error("credential_corrupt"))?;
    let after_state =
        serde_json::to_string_pretty(&saved).map_err(|_| error("credential_corrupt"))?;
    let mut journal = Journal {
        schema: SCHEMA,
        transaction_id: Uuid::new_v4(),
        phase: Phase::Prepared,
        profile_name: profile_name(profile)?,
        state_relative: metadata.state_path.clone(),
        before_profile: String::from_utf8(old_profile_bytes)
            .map_err(|_| error("credential_corrupt"))?,
        before_state: String::from_utf8(old_state_bytes)
            .map_err(|_| error("credential_corrupt"))?,
        after_profile,
        after_state,
        hashes: std::array::from_fn(|_| String::new()),
        old_revision,
        new_revision,
    };
    journal.hashes = bytes(&journal).map(hash);
    validate(&journal, profile, validate_state)?;
    let mut committed = false;
    let result = (|| {
        let path = journal_path(profile)?;
        write(
            Target::Journal,
            &path,
            &serde_json::to_vec(&journal).map_err(|_| error("credential_corrupt"))?,
            profile,
            hook,
        )?;
        checkpoint(Stage::Prepared, hook)?;
        journal.phase = Phase::CommitDecided;
        write(
            Target::Journal,
            &path,
            &serde_json::to_vec(&journal).map_err(|_| error("credential_corrupt"))?,
            profile,
            hook,
        )?;
        committed = true;
        checkpoint(Stage::CommitDecided, hook)?;
        forward(&journal, profile, &state, hook)
    })();
    result.map_err(|mut failure| {
        failure.transaction_id = Some(journal.transaction_id);
        if committed {
            failure.outcome = Outcome::DurabilityUnconfirmed;
            failure.code = "profile_durability_unconfirmed";
        } else if journal.phase == Phase::CommitDecided
            && journal_path(profile).is_ok_and(|path| path.exists())
        {
            // The decision write may have failed before rename. Recovery reads
            // the actual retained Prepared/CommitDecided record; never rollback.
            failure.outcome = if failure.outcome == Outcome::DurabilityUnconfirmed {
                Outcome::DurabilityUnconfirmed
            } else {
                Outcome::RecoveryRequired
            };
        }
        failure
    })?;
    Ok(Receipt {
        transaction_id: Some(journal.transaction_id),
        revision: new_revision,
        changed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn validate_state(bytes: &[u8]) -> Result<(), String> {
        neonmix_control::Authority::restore(
            serde_json::from_slice(bytes).map_err(|_| "invalid state")?,
        )
        .map(|_| ())
        .map_err(|_| "invalid authority".into())
    }
    impl Fixture {
        fn new() -> Self {
            let project = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let directory = project
                .join(".local/tmp")
                .join(format!("事务-{}", Uuid::new_v4()));
            files::private_dir(&directory).unwrap();
            let authority = neonmix_control::Authority::new(
                "old-output".into(),
                "Admin".into(),
                &"a".repeat(64),
            )
            .unwrap();
            files::write_new(
                &directory.join("state.json"),
                &serde_json::to_vec_pretty(&authority.persistent()).unwrap(),
            )
            .unwrap();
            let profile = HubProfile {
                version: 2,
                credential_store: profiles::CredentialStore::File,
                room_name: "Old room".into(),
                state_path: "state.json".into(),
                output: "old-output".into(),
                certificate: "public fixture".into(),
                private_key_ref: Uuid::new_v4().to_string(),
                admin_token_ref: Uuid::new_v4().to_string(),
            };
            files::write_new(
                &directory.join("server.json"),
                &serde_json::to_vec(&profile).unwrap(),
            )
            .unwrap();
            Self(directory)
        }
        fn profile(&self) -> PathBuf {
            self.0.join("server.json")
        }
        fn current(&self) -> (HubProfile, serde_json::Value) {
            (
                parse_profile(
                    &files::read_private(&self.profile(), profiles::MAX_PROFILE_BYTES).unwrap(),
                )
                .unwrap(),
                parse_state(
                    &files::read_private(&self.0.join("state.json"), profiles::MAX_STATE_BYTES)
                        .unwrap(),
                )
                .unwrap(),
            )
        }
        fn prepared(&self) {
            let failed = update_with(
                &self.profile(),
                "new-output",
                "New room",
                &validate_state,
                &mut |stage| {
                    if stage == Stage::Prepared {
                        Err(io::ErrorKind::Other.into())
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(failed.is_err());
        }
    }
    #[test]
    fn all_publication_and_journal_boundaries_recover_the_whole_old_or_new_configuration() {
        let mut cases = vec![
            (Stage::Prepared, 1, false),
            (Stage::CommitDecided, 1, true),
            (Stage::StatePublished, 1, true),
            (Stage::ProfilePublished, 1, true),
            (Stage::Verified, 1, true),
            (Stage::RemoveJournal, 1, true),
            (Stage::JournalRemoved, 1, true),
            (Stage::SyncRemoval, 1, true),
        ];
        for point in [
            ReplaceStage::CreateTemporary,
            ReplaceStage::WriteTemporary,
            ReplaceStage::SyncTemporary,
            ReplaceStage::SyncPreparedDirectory,
            ReplaceStage::Publish,
            ReplaceStage::Published,
            ReplaceStage::SyncPublishedDirectory,
        ] {
            cases.push((Stage::Write(Target::Journal, point), 1, false));
            cases.push((
                Stage::Write(Target::Journal, point),
                2,
                matches!(
                    point,
                    ReplaceStage::Published | ReplaceStage::SyncPublishedDirectory
                ),
            ));
            cases.push((Stage::Write(Target::State, point), 1, true));
            cases.push((Stage::Write(Target::Profile, point), 1, true));
        }
        for (target, ordinal, committed) in cases {
            let fixture = Fixture::new();
            let (old_profile, old_state) = fixture.current();
            let mut count = 0;
            let failure = update_with(
                &fixture.profile(),
                "new-output",
                "New room",
                &validate_state,
                &mut |stage| {
                    if stage == target {
                        count += 1;
                        if count == ordinal {
                            return Err(io::Error::from_raw_os_error(28));
                        }
                    }
                    Ok(())
                },
            )
            .unwrap_err();
            assert_eq!(failure.source.as_ref().unwrap().raw_os_error(), Some(28));
            let _ = recover(&fixture.profile(), validate_state).unwrap();
            let (profile, state) = fixture.current();
            assert_eq!(
                profile.output,
                if committed {
                    "new-output"
                } else {
                    "old-output"
                },
                "{target:?}/{ordinal}"
            );
            assert_eq!(state["output"]["id"], profile.output);
            assert_eq!(
                profile.room_name,
                if committed { "New room" } else { "Old room" }
            );
            assert_eq!(profile.private_key_ref, old_profile.private_key_ref);
            assert_eq!(profile.admin_token_ref, old_profile.admin_token_ref);
            assert_eq!(profile.certificate, old_profile.certificate);
            assert_eq!(state["hub_id"], old_state["hub_id"]);
            assert_eq!(state["devices"], old_state["devices"]);
            assert_eq!(state["credentials"], old_state["credentials"]);
            assert_eq!(
                checked_revision(&state).unwrap(),
                checked_revision(&old_state).unwrap() + u64::from(committed)
            );
            assert!(!journal_path(&fixture.profile()).unwrap().exists());
            if committed {
                let retried =
                    update(&fixture.profile(), "new-output", "New room", validate_state).unwrap();
                assert!(!retried.changed);
                assert_eq!(
                    retried.revision,
                    checked_revision(&state).unwrap(),
                    "retry incremented a second business revision"
                );
            }
        }
    }
    #[test]
    fn corrupt_unknown_schema_changed_identity_and_newer_configuration_fail_closed() {
        for variant in 0..4 {
            let fixture = Fixture::new();
            fixture.prepared();
            let path = journal_path(&fixture.profile()).unwrap();
            let mut journal: serde_json::Value =
                serde_json::from_slice(&files::read_private(&path, MAX_JOURNAL_BYTES).unwrap())
                    .unwrap();
            if variant == 0 {
                journal["hashes"][2] = serde_json::json!("bad hash");
            }
            if variant == 1 {
                journal["schema"] = serde_json::json!(2);
            }
            if variant == 2 {
                let mut after: serde_json::Value =
                    serde_json::from_str(journal["after_profile"].as_str().unwrap()).unwrap();
                after["private_key_ref"] = serde_json::json!(Uuid::new_v4());
                let content = serde_json::to_string(&after).unwrap();
                journal["hashes"][2] = serde_json::json!(hash(content.as_bytes()));
                journal["after_profile"] = serde_json::json!(content);
            }
            files::replace(&path, &serde_json::to_vec(&journal).unwrap()).unwrap();
            if variant == 3 {
                let (mut profile, _) = fixture.current();
                profile.room_name = "Other owner's update".into();
                files::replace(&fixture.profile(), &serde_json::to_vec(&profile).unwrap()).unwrap();
            }
            let before =
                files::read_private(&fixture.profile(), profiles::MAX_PROFILE_BYTES).unwrap();
            assert!(recover(&fixture.profile(), validate_state).is_err());
            assert_eq!(
                files::read_private(&fixture.profile(), profiles::MAX_PROFILE_BYTES).unwrap(),
                before
            );
            assert!(path.exists());
        }
    }
    #[test]
    fn published_directory_failure_returns_unconfirmed_and_keeps_the_same_transaction_for_recovery()
    {
        let fixture = Fixture::new();
        let failed = update_with(
            &fixture.profile(),
            "new-output",
            "New room",
            &validate_state,
            &mut |stage| {
                if stage == Stage::Write(Target::Profile, ReplaceStage::SyncPublishedDirectory) {
                    Err(io::ErrorKind::Other.into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(failed.outcome, Outcome::DurabilityUnconfirmed);
        let recovered = recover(&fixture.profile(), validate_state)
            .unwrap()
            .unwrap();
        assert_eq!(recovered.transaction_id, failed.transaction_id);
        assert_eq!(recovered.revision, 2);
    }
}
