#![allow(unsafe_code)]
use std::{
    fs::{self, DirBuilder},
    io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

pub type Stream = UnixStream;

pub struct Listener {
    socket: UnixListener,
    path: PathBuf,
    directory: PathBuf,
    socket_inode: u64,
    directory_inode: u64,
}
fn uid() -> u32 {
    // SAFETY: getuid has no pointer arguments or failure mode.
    unsafe { libc::getuid() }
}
fn refused(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}
fn check_parent(root: &Path) -> io::Result<()> {
    if !root.is_absolute() {
        return Err(refused("local media directory must be absolute"));
    }
    for ancestor in root.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(refused("local media path cannot traverse symlinks"));
        }
    }
    let metadata = fs::symlink_metadata(root)?;
    if metadata.uid() != uid() || metadata.mode() & 0o022 != 0 {
        return Err(refused(
            "local media parent must be owned by current UID without group/other write",
        ));
    }
    Ok(())
}
impl Listener {
    pub fn bind(root: &Path, nonce: &str) -> io::Result<Self> {
        super::validate_nonce(nonce)?;
        check_parent(root)?;
        let directory = root.join(format!("am-{nonce}"));
        let path = directory.join("s");
        // sockaddr_un.sun_path includes its trailing NUL (104 on macOS, 108
        // on Linux). Fail explicitly; never fall back to a network listener.
        #[cfg(target_os = "macos")]
        const MAX_PATH: usize = 104;
        #[cfg(not(target_os = "macos"))]
        const MAX_PATH: usize = 108;
        if path.as_os_str().as_bytes().len() >= MAX_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local media socket path too long",
            ));
        }
        DirBuilder::new().mode(0o700).create(&directory)?;
        let result = (|| {
            check_parent(&directory)?;
            let directory_metadata = fs::symlink_metadata(&directory)?;
            if directory_metadata.mode() & 0o777 != 0o700 {
                return Err(refused("local media directory must have mode 0700"));
            }
            let socket = UnixListener::bind(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_socket()
                || metadata.uid() != uid()
                || metadata.mode() & 0o777 != 0o600
            {
                return Err(refused("local media socket ownership or mode invalid"));
            }
            socket.set_nonblocking(true)?;
            Ok(Self {
                socket,
                path: path.clone(),
                directory: directory.clone(),
                socket_inode: metadata.ino(),
                directory_inode: directory_metadata.ino(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_dir(&directory);
        }
        result
    }
    pub fn endpoint(&self) -> io::Result<String> {
        self.path
            .to_str()
            .map(|path| format!("unix:{path}"))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "local media path must be UTF-8",
                )
            })
    }
    /// Nonblocking accept: rejection drops the connected descriptor immediately.
    pub fn accept(&mut self, expected_pid: u32) -> io::Result<Stream> {
        let (stream, _) = self.socket.accept()?;
        let (peer_uid, peer_pid) = peer(&stream)?;
        if peer_uid != uid() || peer_pid != expected_pid {
            return Err(refused("local media peer UID/PID mismatch"));
        }
        Ok(stream)
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        // Only unlink objects created by this listener, never replacements.
        if fs::symlink_metadata(&self.path).is_ok_and(|m| {
            m.ino() == self.socket_inode && m.uid() == uid() && m.file_type().is_socket()
        }) {
            let _ = fs::remove_file(&self.path);
        }
        if fs::symlink_metadata(&self.directory).is_ok_and(|m| {
            m.ino() == self.directory_inode
                && m.uid() == uid()
                && m.is_dir()
                && !m.file_type().is_symlink()
        }) {
            let _ = fs::remove_dir(&self.directory);
        }
    }
}
#[cfg(target_os = "macos")]
fn peer(stream: &Stream) -> io::Result<(u32, u32)> {
    let mut user = 0;
    let mut group = 0;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of_val(&pid) as libc::socklen_t;
    // SAFETY: descriptor is live; each output points to its exact writable type.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) } != 0
        // SAFETY: LOCAL_PEERPID writes a pid_t into the buffer of the stated size.
        || unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut libc::pid_t).cast(), &mut length) } != 0
        || length as usize != std::mem::size_of_val(&pid) || pid <= 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((user, pid as u32))
}
#[cfg(target_os = "linux")]
fn peer(stream: &Stream) -> io::Result<(u32, u32)> {
    // SAFETY: ucred contains only integer fields, so all-zero bits are valid.
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
    // SAFETY: SO_PEERCRED writes into a live, correctly sized ucred buffer.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&credentials)
        || credentials.pid <= 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((credentials.uid, credentials.pid as u32))
}
