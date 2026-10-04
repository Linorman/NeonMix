//! One-instance, byte-mode NamedPipe. PIPE_NOWAIT keeps accept/read independent
//! of worker shutdown; the reader polls at most its configured timeout.
#![allow(unsafe_code)]
use std::{
    ffi::c_void,
    io::{self, Read},
    path::Path,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{PIPE_ACCESS_INBOUND, ReadFile},
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS, PeekNamedPipe,
        },
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};
struct Handle(HANDLE);
// SAFETY: ownership is unique, handles are used sequentially by listener or reader.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns a valid handle and closes it once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: the owned pointer was allocated by a Win32 LocalAlloc API.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn sid(process: HANDLE) -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: process is live and token output is writable.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut size = 0;
    // SAFETY: first query only obtains the required buffer size.
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65_536 {
        return Err(io::Error::other("invalid user token size"));
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: usize buffer is TOKEN_USER aligned and at least size bytes long.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut output = ptr::null_mut();
    // SAFETY: successful query populated TOKEN_USER and its live SID.
    if unsafe {
        ConvertSidToStringSidW(
            (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid,
            &mut output,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let _text = Local(output.cast());
    let mut length = 0;
    // SAFETY: conversion returns a NUL-terminated live UTF-16 string.
    unsafe {
        while *output.add(length) != 0 {
            length += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            output, length,
        )))
    }
}
fn current_sid() -> io::Result<String> {
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle.
    sid(unsafe { GetCurrentProcess() })
}
pub struct Listener {
    endpoint: String,
    pipe: Option<Handle>,
    user: String,
}
pub struct Stream {
    pipe: Handle,
    timeout: Duration,
}
impl Listener {
    pub fn bind(_root: &Path, nonce: &str) -> io::Result<Self> {
        super::validate_nonce(nonce)?;
        let user = current_sid()?;
        let endpoint = format!(r"\\.\pipe\NeonMix.Airplay.v1.{user}.{nonce}");
        let name: Vec<u16> = endpoint.encode_utf16().chain([0]).collect();
        let sddl: Vec<u16> = format!("O:{user}D:P(A;;FA;;;{user})")
            .encode_utf16()
            .chain([0])
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: input is NUL terminated; descriptor output is writable.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let descriptor = Local(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        // SAFETY: descriptor/name remain live through creation. First-instance
        // flag prevents pre-created pipe substitution; D:P disables inheritance.
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | 0x0008_0000,
                PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                crate::MAX_PACKET_BYTES as u32,
                250,
                &attributes,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            endpoint,
            pipe: Some(Handle(pipe)),
            user,
        })
    }
    pub fn endpoint(&self) -> io::Result<String> {
        Ok(self.endpoint.clone())
    }
    pub fn accept(&mut self, expected_pid: u32) -> io::Result<Stream> {
        let pipe = self
            .pipe
            .as_ref()
            .ok_or_else(|| io::Error::other("local media listener already accepted"))?;
        // SAFETY: PIPE_NOWAIT makes this call return immediately.
        if unsafe { ConnectNamedPipe(pipe.0, ptr::null_mut()) } == 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_PIPE_CONNECTED) => {}
                Some(ERROR_PIPE_LISTENING) => return Err(io::ErrorKind::WouldBlock.into()),
                _ => return Err(error),
            }
        }
        let mut pid = 0;
        // SAFETY: kernel returns the connected client's PID into writable output.
        let verified = unsafe { GetNamedPipeClientProcessId(pipe.0, &mut pid) } != 0
            && pid == expected_pid
            && {
                // SAFETY: PID is supplied by the kernel; rights permit only token query.
                let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                !process.is_null() && sid(Handle(process).0).is_ok_and(|peer| peer == self.user)
            };
        if !verified {
            // SAFETY: listener owns the connected pipe and keeps it for a retry.
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "local media peer SID/PID mismatch",
            ));
        }
        Ok(Stream {
            pipe: self.pipe.take().expect("verified pipe exists"),
            timeout: Duration::from_millis(250),
        })
    }
}
impl Stream {
    pub fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        // Transport always enforces 250 ms. Disabling the bound is unsupported.
        if _timeout != Some(self.timeout) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "local media timeout must be 250 ms",
            ));
        }
        Ok(())
    }
}
impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            let mut available = 0;
            // SAFETY: live pipe handle; only available byte count is requested.
            if unsafe {
                PeekNamedPipe(
                    self.pipe.0,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut available,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if available != 0 {
                let mut count = 0;
                // SAFETY: count is bounded by both ready data and destination slice.
                if unsafe {
                    ReadFile(
                        self.pipe.0,
                        bytes.as_mut_ptr(),
                        available.min(bytes.len() as u32),
                        &mut count,
                        ptr::null_mut(),
                    )
                } == 0
                {
                    return Err(io::Error::last_os_error());
                }
                return Ok(count as usize);
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
