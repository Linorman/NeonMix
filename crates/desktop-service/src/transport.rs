use crate::{Envelope, IPC_TIMEOUT, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, Reply, Request, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::Path,
};

pub fn prepare(directory: &Path) -> Result<()> {
    if directory.exists() {
        check_directory(directory)?;
        return Ok(());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    check_directory(directory)
}
pub fn check_directory(directory: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err(
            "IPC state directory must be a user-owned real directory with mode 0700".into(),
        );
    }
    // Reject symlinks in all existing ancestors, not merely the final component.
    let absolute = if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(directory)
    };
    for ancestor in absolute.ancestors() {
        if std::fs::symlink_metadata(ancestor)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("IPC state path may not traverse symlinks".into());
        }
    }
    Ok(())
}
#[allow(unsafe_code)]
pub fn uid() -> u32 {
    // SAFETY: geteuid takes no pointers and cannot fail.
    unsafe { libc::geteuid() }
}
pub fn lock(directory: &Path) -> Result<File> {
    let path = directory.join("background.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0 {
        return Err("insecure background lock".into());
    }
    acquire_lock(&file)?;
    Ok(file)
}
#[allow(unsafe_code)]
fn acquire_lock(file: &File) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the live File owns the valid descriptor for the entire flock call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("background already running".into());
    }
    Ok(())
}
#[allow(unsafe_code)]
pub fn same_user(stream: &tokio::net::UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "macos")]
    {
        let mut peer_uid = 0;
        let mut peer_gid = 0;
        // SAFETY: descriptor is live; getpeereid writes to valid uid/gid storage.
        unsafe {
            libc::getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) == 0
                && peer_uid == libc::geteuid()
        }
    }
    #[cfg(target_os = "linux")]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: cred and len are valid output buffers of the specified size.
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            ) == 0
                && cred.uid == libc::geteuid()
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = stream;
        false
    }
}
pub fn request(directory: &Path, request: &Request) -> Result<Reply> {
    check_directory(directory)?;
    let path = directory.join(if crate::manager::is_lifecycle(request) {
        "lifecycle.sock"
    } else {
        "ipc.sock"
    });
    let metadata = std::fs::symlink_metadata(&path).map_err(crate::client_io_error)?;
    if !metadata.file_type().is_socket() || metadata.uid() != uid() || metadata.mode() & 0o077 != 0
    {
        return Err("permission_denied".into());
    }
    let bytes = serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        request: request.clone(),
    })
    .map_err(|_| "ipc_invalid_request".to_string())?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("ipc_message_too_large".into());
    }
    let mut stream = UnixStream::connect(path).map_err(crate::client_io_error)?;
    stream
        .set_read_timeout(Some(IPC_TIMEOUT))
        .map_err(crate::client_io_error)?;
    stream
        .set_write_timeout(Some(IPC_TIMEOUT))
        .map_err(crate::client_io_error)?;
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .map_err(crate::client_io_error)?;
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .map_err(crate::client_io_error)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_MESSAGE_BYTES {
        return Err("ipc_message_too_large".into());
    }
    let mut body = vec![0; len];
    stream
        .read_exact(&mut body)
        .map_err(crate::client_io_error)?;
    serde_json::from_slice(&body).map_err(|_| "invalid_backend_response".into())
}

pub type Stream = tokio::net::UnixStream;
pub struct Listener {
    listener: tokio::net::UnixListener,
    path: std::path::PathBuf,
}
impl Listener {
    pub fn bind(directory: &Path) -> Result<Self> {
        Self::bind_endpoint(directory, false)
    }
    pub(crate) fn bind_endpoint(directory: &Path, lifecycle: bool) -> Result<Self> {
        let path = directory.join(if lifecycle {
            "lifecycle.sock"
        } else {
            "ipc.sock"
        });
        if path.exists() {
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !metadata.file_type().is_socket() || metadata.uid() != uid() {
                return Err("拒绝覆盖非本人 socket".into());
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let listener = tokio::net::UnixListener::bind(&path).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        Ok(Self { listener, path })
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        self.listener
            .accept()
            .await
            .map(|(stream, _)| stream)
            .map_err(|e| e.to_string())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub fn protect_directory(directory: &Path) -> Result<()> {
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())
}
pub fn check_owned(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || metadata.uid() != uid() {
        return Err("拒绝符号链接或非当前用户的配置路径".into());
    }
    Ok(())
}
