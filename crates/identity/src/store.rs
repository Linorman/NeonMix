//! Profile-local credential JSON. Only this module serializes secret values.
use crate::{Result, files};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

pub const MAX_SECRET_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    HubTlsKey,
    AdminToken,
    MemberToken,
    AirplayReceiverKey,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential_missing")]
    Missing,
    #[error("credential_corrupt")]
    Corrupt,
    #[error("credential_version_unsupported")]
    VersionUnsupported,
    #[error("credential_permission_denied")]
    PermissionDenied,
    #[error("credential_kind_mismatch")]
    KindMismatch,
    #[error("credential_store_busy")]
    StoreBusy,
    #[error("credential_io_failed")]
    IoFailed,
    #[error("credential_reference_invalid")]
    ReferenceInvalid,
}
fn error(error: io::Error) -> CredentialError {
    match error.kind() {
        io::ErrorKind::NotFound => CredentialError::Missing,
        io::ErrorKind::PermissionDenied => CredentialError::PermissionDenied,
        io::ErrorKind::InvalidData => CredentialError::Corrupt,
        io::ErrorKind::WouldBlock => CredentialError::StoreBusy,
        _ => CredentialError::IoFailed,
    }
}
/// The only accepted filename identity is a UUID, normalized to hyphenated form.
pub fn normalize_reference(value: &str) -> Result<String> {
    Ok(uuid::Uuid::parse_str(value)
        .map_err(|_| CredentialError::ReferenceInvalid)?
        .to_string())
}
/// Deliberately has no Serialize or plaintext Debug implementation.
pub struct SecretValue(String);
impl SecretValue {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    version: u32,
    kind: SecretKind,
    value: String,
}

#[derive(Debug, Clone)]
pub struct FileCredentialStore {
    directory: PathBuf,
}
impl FileCredentialStore {
    /// Resolves the existing profile parent, never creates a credential directory.
    /// CWD is captured here, so subsequent changes cannot redirect the store.
    pub fn for_profile(profile: &Path) -> Result<Self> {
        let parent = profile
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = std::fs::canonicalize(parent).map_err(error)?;
        Ok(Self {
            directory: parent.join(".credentials"),
        })
    }
    pub fn identity(&self) -> PathBuf {
        self.directory.clone()
    }
    fn path(&self, reference: &str) -> Result<PathBuf> {
        Ok(self
            .directory
            .join(format!("{}.json", normalize_reference(reference)?)))
    }
    pub fn put(&self, kind: SecretKind, value: &str) -> Result<String> {
        let reference = uuid::Uuid::new_v4().to_string();
        self.put_reference(&reference, kind, value)?;
        Ok(reference)
    }
    /// Explicit preserved UUID for offline migration; never overwrites an entry.
    pub fn put_reference(&self, reference: &str, kind: SecretKind, value: &str) -> Result<()> {
        let path = self.path(reference)?;
        if value.is_empty() {
            return Err(CredentialError::Corrupt.into());
        }
        let entry = Entry {
            version: 1,
            kind,
            value: value.to_owned(),
        };
        let bytes = serde_json::to_vec(&entry).map_err(|_| CredentialError::Corrupt)?;
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::Corrupt.into());
        }
        files::private_dir(&self.directory).map_err(error)?;
        let directory = files::PrivateDirectory::open(&self.directory).map_err(error)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CredentialError::ReferenceInvalid)?;
        directory.write_new(name, &bytes).map_err(error)?;
        Ok(())
    }
    fn decode(bytes: &[u8], expected_kind: SecretKind) -> Result<SecretValue> {
        let entry: Entry = serde_json::from_slice(bytes).map_err(|_| CredentialError::Corrupt)?;
        if entry.version != 1 {
            return Err(CredentialError::VersionUnsupported.into());
        }
        if entry.kind != expected_kind {
            return Err(CredentialError::KindMismatch.into());
        }
        if entry.value.is_empty() {
            return Err(CredentialError::Corrupt.into());
        }
        Ok(SecretValue(entry.value))
    }
    pub fn get(&self, reference: &str, expected_kind: SecretKind) -> Result<SecretValue> {
        let name = format!("{}.json", normalize_reference(reference)?);
        let bytes =
            files::read_private_in(&self.directory, &name, MAX_SECRET_BYTES).map_err(error)?;
        Self::decode(&bytes, expected_kind)
    }

    /// Missing entries are idempotent; present unsafe or mismatched entries fail.
    pub fn remove(&self, reference: &str, expected_kind: SecretKind) -> Result<()> {
        let name = format!("{}.json", normalize_reference(reference)?);
        let directory = match files::PrivateDirectory::open(&self.directory) {
            Ok(directory) => directory,
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(failure) => return Err(error(failure).into()),
        };
        let bytes = match directory.read(&name, MAX_SECRET_BYTES) {
            Ok(bytes) => bytes,
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(failure) => return Err(error(failure).into()),
        };
        Self::decode(&bytes, expected_kind)?;
        match directory.remove(&name) {
            Ok(()) => {}
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => {}
            Err(failure) => return Err(error(failure).into()),
        }
        Ok(())
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
            let path = root
                .join(".local/tmp")
                .join(format!("credential-store-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> FileCredentialStore {
            FileCredentialStore::for_profile(&self.0.join("member.json")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn round_trip_exclusive_and_idempotent() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let value = crate::secret();
        let reference = store.put(SecretKind::MemberToken, &value).unwrap();
        assert!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap()
                .expose()
                == value,
            "stored credential value changed"
        );
        assert!(
            store
                .put_reference(&reference, SecretKind::MemberToken, "replacement")
                .is_err()
        );
        assert!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap()
                .expose()
                == value,
            "stored credential value changed"
        );
        assert_eq!(
            store
                .get(&reference, SecretKind::AdminToken)
                .unwrap_err()
                .to_string(),
            "credential_kind_mismatch"
        );
        assert!(store.remove(&reference, SecretKind::AdminToken).is_err());
        store.remove(&reference, SecretKind::MemberToken).unwrap();
        store.remove(&reference, SecretKind::MemberToken).unwrap();
        assert_eq!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap_err()
                .to_string(),
            "credential_missing"
        );
    }
    #[test]
    fn complete_store_identity_isolates_same_uuid_and_reads_do_not_create() {
        let first = Fixture::new();
        let second = Fixture::new();
        let a = first.store();
        let b = second.store();
        let reference = uuid::Uuid::new_v4().to_string();
        assert!(a.get(&reference, SecretKind::MemberToken).is_err());
        assert!(!a.identity().exists());
        a.remove(&reference, SecretKind::MemberToken).unwrap();
        assert!(!a.identity().exists());
        a.put_reference(&reference, SecretKind::MemberToken, &crate::secret())
            .unwrap();
        b.put_reference(&reference, SecretKind::MemberToken, &crate::secret())
            .unwrap();
        assert_ne!(a.identity(), b.identity());
        assert!(
            a.get(&reference, SecretKind::MemberToken).unwrap().expose()
                != b.get(&reference, SecretKind::MemberToken).unwrap().expose(),
            "independent stores shared credential content"
        );
        for invalid in ["../member", "id:stream", "", "a/b", "a\\b"] {
            assert!(a.get(invalid, SecretKind::MemberToken).is_err());
        }
    }
    #[test]
    fn cwd_child_helper() {
        let Some(profile) = std::env::var_os("NEONMIX_STORE_CWD_CHILD") else {
            return;
        };
        let store = FileCredentialStore::for_profile(Path::new(&profile)).unwrap();
        let reference = std::env::var("NEONMIX_STORE_CWD_REFERENCE").unwrap();
        std::env::set_current_dir(store.identity().parent().unwrap().parent().unwrap()).unwrap();
        let value = store.get(&reference, SecretKind::MemberToken).unwrap();
        assert!(
            crate::digest(value.expose()) == std::env::var("NEONMIX_STORE_CWD_DIGEST").unwrap(),
            "working directory change redirected credential lookup"
        );
    }
    #[test]
    fn store_resolution_survives_unrelated_working_directory() {
        let first = Fixture::new();
        let second = Fixture::new();
        let store = first.store();
        let value = crate::secret();
        let reference = store.put(SecretKind::MemberToken, &value).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "store::tests::cwd_child_helper"])
            .current_dir(&second.0)
            .env("NEONMIX_STORE_CWD_CHILD", first.0.join("member.json"))
            .env("NEONMIX_STORE_CWD_REFERENCE", reference)
            .env("NEONMIX_STORE_CWD_DIGEST", crate::digest(&value))
            .output()
            .unwrap();
        assert!(output.status.success(), "CWD child failed");
    }
    #[test]
    fn rejects_corruption_version_unknown_fields_and_oversize_without_leaking() {
        let fixture = Fixture::new();
        let store = fixture.store();
        assert!(
            store.put(SecretKind::MemberToken, "").is_err(),
            "empty credential accepted"
        );
        let reference = store
            .put(SecretKind::MemberToken, &crate::secret())
            .unwrap();
        let path = store.path(&reference).unwrap();
        for (bytes, expected) in [
            (
                br#"{"version":1,"kind":"member_token","value":""}"#.to_vec(),
                "credential_corrupt",
            ),
            (b"not JSON".to_vec(), "credential_corrupt"),
            (
                br#"{"version":2,"kind":"member_token","value":"redacted"}"#.to_vec(),
                "credential_version_unsupported",
            ),
            (
                br#"{"version":1,"kind":"member_token","value":"redacted","extra":1}"#.to_vec(),
                "credential_corrupt",
            ),
            (vec![b'x'; MAX_SECRET_BYTES + 1], "credential_corrupt"),
        ] {
            files::replace(&path, &bytes).unwrap();
            assert_eq!(
                store
                    .get(&reference, SecretKind::MemberToken)
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
        assert_eq!(
            format!("{:?}", SecretValue(crate::secret())),
            "SecretValue([REDACTED])"
        );
    }
    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)]
    fn rejects_links_nonfiles_and_wide_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        let store = fixture.store();
        let reference = store
            .put(SecretKind::MemberToken, &crate::secret())
            .unwrap();
        let path = store.path(&reference).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap_err()
                .to_string(),
            "credential_permission_denied"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(store.identity(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            store
                .get(&reference, SecretKind::MemberToken)
                .unwrap_err()
                .to_string(),
            "credential_permission_denied"
        );
        std::fs::set_permissions(store.identity(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let backup = fixture.0.join("private-original");
        std::fs::rename(&path, &backup).unwrap();
        symlink(&backup, &path).unwrap();
        assert!(store.get(&reference, SecretKind::MemberToken).is_err());
        assert!(store.remove(&reference, SecretKind::MemberToken).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.get(&reference, SecretKind::MemberToken).is_err());
        std::fs::remove_dir(&path).unwrap();
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: NUL-terminated path is valid and remains live through mkfifo.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(store.get(&reference, SecretKind::MemberToken).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(store.identity()).unwrap();
        symlink(&fixture.0, store.identity()).unwrap();
        assert!(store.get(&reference, SecretKind::MemberToken).is_err());
        assert!(
            store
                .put(SecretKind::MemberToken, &crate::secret())
                .is_err()
        );
    }
}
