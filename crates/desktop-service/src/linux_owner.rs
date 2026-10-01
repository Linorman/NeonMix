//! E06 uses a kernel-owned abstract socket to reserve one virtual node per UID,
//! regardless of which private state directory started its independent owner.
#![cfg(target_os = "linux")]
use crate::Result;
use std::os::{
    fd::AsRawFd,
    linux::net::SocketAddrExt,
    unix::net::{SocketAddr, UnixStream},
};

pub fn current_user_owner_exists() -> Result<bool> {
    let uid = crate::transport::uid();
    if uid == 0 {
        return Err("Linux 虚拟输出需要已登录的普通 PipeWire 用户".into());
    }
    let address =
        SocketAddr::from_abstract_name(format!("neonmix.virtual-output.owner.{uid}").as_bytes())
            .map_err(|e| e.to_string())?;
    let stream = match UnixStream::connect_addr(&address) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ECONNREFUSED | libc::ENOENT)
            ) =>
        {
            return Ok(false);
        }
        Err(_) => return Err("无法检查当前用户的虚拟输出 owner".into()),
    };
    if !same_user(&stream, uid) {
        return Err("虚拟输出 owner 不属于当前用户".into());
    }
    // No name is sent: E06 rejects an empty request without changing the node.
    drop(stream);
    Ok(true)
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
