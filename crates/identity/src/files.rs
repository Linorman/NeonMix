//! Private creation, bounded handle validation, durable replacement and OS locks.
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

#[cfg(windows)]
#[allow(unsafe_code)]
#[path = "files_windows.rs"]
mod platform;

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "private_file_permission_denied",
    )
}
#[cfg(unix)]
#[allow(unsafe_code)]
fn check(file: &File, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no pointers or preconditions.
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(denied());
    }
    Ok(())
}
#[cfg(unix)]
fn open_private(path: &Path, directory: bool) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let flags = libc::O_NOFOLLOW | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                denied()
            } else {
                error
            }
        })?;
    check(&file, directory)?;
    Ok(file)
}
#[cfg(windows)]
fn open_private(path: &Path, directory: bool) -> io::Result<File> {
    platform::open_private(path, directory)
}
#[cfg(unix)]
fn create(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}
#[cfg(windows)]
fn create(path: &Path) -> io::Result<File> {
    platform::create(path)
}

/// Validates an existing directory through a non-following opened handle.
pub fn validate_private_dir(path: &Path) -> io::Result<()> {
    open_private(path, true).map(drop)
}
/// An opened private directory pins every subsequent operation to that object.
/// Unix uses openat/unlinkat; Windows denies directory delete-sharing while held.
pub struct PrivateDirectory {
    #[cfg(unix)]
    _file: File,
    #[cfg(windows)]
    directory: platform::PinnedDirectory,
}
impl PrivateDirectory {
    pub fn open(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                _file: open_private(path, true)?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                directory: platform::pin_directory(path)?,
            })
        }
    }
    pub fn read(&self, name: &str, limit: usize) -> io::Result<Vec<u8>> {
        validate_name(name)?;
        #[cfg(unix)]
        let file = self.open_file(name, false)?;
        #[cfg(windows)]
        let file = platform::open_private(&self.directory.path.join(name), false)?;
        read_bounded(file, limit)
    }
    /// Exclusive create, durable write and readback all use the same directory
    /// and file handles, including cleanup after our own failed write.
    pub fn write_new(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        use std::io::{Seek, SeekFrom};
        validate_name(name)?;
        #[cfg(unix)]
        let mut file = self.open_file(name, true)?;
        #[cfg(windows)]
        let mut file = platform::create(&self.directory.path.join(name))?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
            let mut readback = Vec::new();
            (&mut file)
                .take(bytes.len() as u64 + 1)
                .read_to_end(&mut readback)?;
            if readback != bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "private_file_readback_failed",
                ));
            }
            self.sync()
        })();
        drop(file);
        if result.is_err() {
            let _ = self.remove(name);
        }
        result
    }
    pub fn remove(&self, name: &str) -> io::Result<()> {
        validate_name(name)?;
        #[cfg(unix)]
        self.unlink(name)?;
        #[cfg(windows)]
        std::fs::remove_file(self.directory.path.join(name))?;
        self.sync()
    }
    fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        self._file.sync_all()?;
        Ok(())
    }
    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn open_file(&self, name: &str, create: bool) -> io::Result<File> {
        use std::{
            ffi::CString,
            os::fd::{AsRawFd, FromRawFd},
        };
        let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let flags = libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if create {
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
        // SAFETY: directory fd and pathname remain live; successful fd is owned.
        let descriptor =
            unsafe { libc::openat(self._file.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(libc::ELOOP) {
                denied()
            } else {
                error
            });
        }
        // SAFETY: openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(descriptor) };
        check(&file, false)?;
        Ok(file)
    }
    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn unlink(&self, name: &str) -> io::Result<()> {
        use std::{ffi::CString, os::fd::AsRawFd};
        let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: directory fd and C pathname remain live through unlinkat.
        if unsafe { libc::unlinkat(self._file.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(())
}
fn read_bounded(file: File, limit: usize) -> io::Result<Vec<u8>> {
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private_file_too_large",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private_file_too_large",
        ));
    }
    Ok(bytes)
}
/// Bounded read anchored in a validated private directory handle.
pub fn read_private_in(directory: &Path, name: &str, limit: usize) -> io::Result<Vec<u8>> {
    PrivateDirectory::open(directory)?.read(name, limit)
}
/// Creates one private directory, or validates the existing one without chmod.
pub fn private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => sync_parent(path)?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    #[cfg(windows)]
    platform::private_dir(path)?;
    validate_private_dir(path)
}
/// Validates type, ownership, permissions and size before reading, then applies
/// the limit again while reading so concurrent growth cannot exceed it.
pub fn read_private(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    read_bounded(open_private(path, false)?, limit)
}
/// Never replaces a caller's existing file, even when initial writing fails.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create(path)?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .and_then(|_| sync_parent(path));
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
/// A random private sibling is fully synced before replacement. Failed rename
/// leaves the destination intact and cleans only this invocation's temporary.
pub fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_file_name(format!(".neonmix-{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temporary, bytes)?;
    #[cfg(unix)]
    let result = std::fs::rename(&temporary, path);
    #[cfg(windows)]
    let result = platform::replace(&temporary, path);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    sync_parent(path)
}
/// Syncs the containing directory after entry creation/removal/rename on Unix.
pub fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?
    .sync_all()?;
    #[cfg(windows)]
    let _ = path; // Win32 replacement uses MOVEFILE_WRITE_THROUGH.
    Ok(())
}
/// Stable lock inode: callers must never delete or replace its pathname.
/// Dropping the guard or terminating the process releases the OS lock.
pub struct Guard {
    _file: File,
}
impl Drop for Guard {
    fn drop(&mut self) {
        // Explicit unlocking avoids keeping the lock alive through a transient
        // descriptor duplicate inherited by a concurrently spawning process.
        #[cfg(unix)]
        unix_unlock(&self._file);
        #[cfg(windows)]
        platform::unlock(&self._file);
    }
}
#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_unlock(file: &File) {
    use std::os::fd::AsRawFd;
    // SAFETY: the guard's owned descriptor remains live until Drop completes.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

pub fn lock(path: &Path) -> io::Result<Guard> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                if error.raw_os_error() == Some(libc::ELOOP) {
                    denied()
                } else {
                    error
                }
            })?;
        check(&file, false)?;
        unix_lock(&file)?;
        Ok(Guard { _file: file })
    }
    #[cfg(windows)]
    {
        Ok(Guard {
            _file: platform::lock(path)?,
        })
    }
}
#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_lock(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: flock receives the live borrowed file descriptor; no pointers.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
/// Publishes a same-filesystem staging directory without replacing any target.
pub fn publish_directory(staging: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    unix_publish(staging, destination)?;
    #[cfg(windows)]
    platform::publish_directory(staging, destination)?;
    sync_parent(destination)
}
#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_publish(staging: &Path, destination: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(staging.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let target = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    #[cfg(target_os = "macos")]
    // SAFETY: both C path strings are valid and live through the syscall.
    let status = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    // SAFETY: both C path strings are valid and live through the syscall.
    let status = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
        assert_eq!(read_private(&path, 64).unwrap(), b"replacement");
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
    #[test]
    fn directory_publish_does_not_replace_existing_target() {
        let directory = Directory::new();
        let source = directory.0.join("staging");
        let target = directory.0.join("target");
        private_dir(&source).unwrap();
        private_dir(&target).unwrap();
        assert!(publish_directory(&source, &target).is_err());
        assert!(source.is_dir());
        std::fs::remove_dir(&target).unwrap();
        publish_directory(&source, &target).unwrap();
        assert!(!source.exists());
        assert!(target.is_dir());
    }
    #[test]
    fn stable_lock_is_nonblocking_and_reusable() {
        let directory = Directory::new();
        let path = directory.0.join("operation.lock");
        let first = lock(&path).unwrap();
        assert_eq!(lock(&path).err().unwrap().kind(), io::ErrorKind::WouldBlock);
        // A concurrent process spawn briefly duplicates the open file
        // description before exec closes it. Keep a duplicate to reproduce
        // that case deterministically and require Drop to unlock explicitly.
        let _inherited_descriptor = first._file.try_clone().unwrap();
        drop(first);
        let _second = lock(&path).unwrap();
        assert!(path.exists());
    }
    #[cfg(unix)]
    #[test]
    fn private_directory_handle_survives_path_replaced_by_symlink() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let original = directory.0.join("credentials");
        let foreign = directory.0.join("foreign");
        private_dir(&original).unwrap();
        private_dir(&foreign).unwrap();
        let pinned = PrivateDirectory::open(&original).unwrap();
        pinned.write_new("value.json", b"original").unwrap();
        let moved = directory.0.join("moved");
        std::fs::rename(&original, &moved).unwrap();
        symlink(&foreign, &original).unwrap();
        assert_eq!(pinned.read("value.json", 64).unwrap(), b"original");
        pinned.write_new("second.json", b"pinned").unwrap();
        assert!(moved.join("second.json").is_file());
        assert!(!foreign.join("second.json").exists());
        pinned.remove("value.json").unwrap();
        assert!(!moved.join("value.json").exists());
        assert!(PrivateDirectory::open(&original).is_err());
    }
    #[test]
    fn lock_child_helper() {
        let Some(directory) = std::env::var_os("NEONMIX_LOCK_TEST_CHILD") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let _guard = lock(&directory.join("owner.lock")).unwrap();
        write_new(&directory.join("ready"), b"ready").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    #[test]
    fn crashed_owner_releases_stable_lock() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let directory = Directory::new();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "files::tests::lock_child_helper", "--nocapture"])
            .env("NEONMIX_LOCK_TEST_CHILD", &directory.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.0.join("ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = directory.0.join("ready").exists();
        let conflict = lock(&directory.0.join("owner.lock"));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(ready, "lock child did not become ready");
        assert_eq!(conflict.err().unwrap().kind(), io::ErrorKind::WouldBlock);
        let _guard = lock(&directory.0.join("owner.lock")).unwrap();
    }
}
