//! E06 uses a kernel-owned abstract socket to reserve one virtual node per UID,
//! regardless of which private state directory started its independent owner.
#![cfg(target_os = "linux")]
use crate::owner::{INSPECT_OWNER, OWNER_MESSAGE_LIMIT, OwnerIdentity};
use std::io::{Read, Write};
use std::os::{
    fd::AsRawFd,
    linux::net::SocketAddrExt,
    unix::net::{SocketAddr, UnixStream},
};
use std::time::Duration;

pub fn inspect_owner() -> Result<Option<OwnerIdentity>, String> {
    // SAFETY: getuid has no pointers and only reads the calling process identity.
    let uid = unsafe { libc::getuid() };
    if uid == 0 {
        return Err("output_unavailable".into());
    }
    let address =
        SocketAddr::from_abstract_name(format!("neonmix.virtual-output.owner.{uid}").as_bytes())
            .map_err(|e| e.to_string())?;
    let mut stream = match UnixStream::connect_addr(&address) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ECONNREFUSED | libc::ENOENT)
            ) =>
        {
            return Ok(None);
        }
        Err(_) => return Err("output_unavailable".into()),
    };
    if !same_user(&stream, uid) {
        return Err("permission_denied".into());
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| "output_unavailable")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| "output_unavailable")?;
    stream
        .write_all(INSPECT_OWNER)
        .map_err(|_| "output_unavailable")?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|_| "output_unavailable")?;
    let mut response = Vec::new();
    stream
        .take((OWNER_MESSAGE_LIMIT + 1) as u64)
        .read_to_end(&mut response)
        .map_err(|_| "output_unavailable")?;
    if response.len() > OWNER_MESSAGE_LIMIT {
        return Err("upgrade_required".into());
    }
    serde_json::from_slice(&response)
        .map(Some)
        .map_err(|_| "upgrade_required".into())
}
#[allow(unsafe_code)]
fn same_user(stream: &UnixStream, uid: u32) -> bool {
    let mut peer = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: stream owns a connected descriptor; peer/len are valid fixed-size
    // output buffers and the kernel supplies the peer credentials.
    unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut len,
        ) == 0
            && len as usize == std::mem::size_of::<libc::ucred>()
            && peer.uid == uid
    }
}
