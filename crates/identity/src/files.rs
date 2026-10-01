//! Private creation and atomic replacement on the three supported platforms.
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "files_windows.rs"]
mod platform;

#[cfg(unix)]
fn create(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}
#[cfg(windows)]
fn create(path: &Path) -> io::Result<File> {
    platform::create(path)
}

/// Never replace a caller's existing file, even when initial writing fails.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create(path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
/// The temporary file is an exclusively owned, private sibling. A stale file
/// from a previous process cannot block retry or receive newly written data.
pub fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_file_name(format!(".neonmix-{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temporary, bytes)?;
    let result = std::fs::rename(&temporary, path);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let path = root
                .join(".local/tmp")
                .join(format!("identity-files-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn private_replace_is_exclusive_and_removes_an_old_insecure_file_mode() {
        let directory = Directory::new();
        let path = directory.0.join("credential.json");
        write_new(&path, b"original").unwrap();
        assert_eq!(
            write_new(&path, b"overwrite").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        replace(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
    #[test]
    fn failed_replace_preserves_destination_and_cleans_its_own_temporary_file() {
        let directory = Directory::new();
        let destination = directory.0.join("existing-directory");
        std::fs::create_dir(&destination).unwrap();
        assert!(replace(&destination, b"sensitive").is_err());
        assert!(destination.is_dir());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}
