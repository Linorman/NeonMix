//! Local NamedPipe transport with a protected current-user SID DACL. Native
//! calls live at this boundary; IPC never accepts executable names or shell text.
#![allow(unsafe_code)]
use crate::{Envelope, IPC_TIMEOUT, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, Reply, Request, Result};
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    fs::File,
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::Path,
    ptr,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        ACCESS_ALLOWED_ACE,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorDacl, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_OPEN_REPARSE_POINT, OPEN_ALWAYS,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        SystemServices::ACCESS_ALLOWED_ACE_TYPE,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle exclusively owns a valid Win32 handle created in this module.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: conversion/security APIs allocate these buffers with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn sid_string(sid: *mut c_void) -> Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: callers supply a SID inside a live token/security buffer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err("无法读取用户 SID".into());
    }
    let _buffer = Local(text.cast());
    let mut len = 0;
    // SAFETY: conversion returns a NUL-terminated UTF-16 buffer; each read is within it.
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            text, len,
        )))
    }
}
pub(crate) fn process_sid(process: HANDLE) -> Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: process is a live borrowed handle; token output is writable.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err("无法读取进程用户".into());
    }
    let token = Handle(token);
    let mut required = 0;
    // SAFETY: zero-length query requests the required size through a valid output.
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required == 0 || required > 65_536 {
        return Err("用户令牌大小无效".into());
    }
    let mut buffer = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: usize buffer provides TOKEN_USER alignment and at least required bytes.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err("无法读取用户令牌".into());
    }
    // SAFETY: successful TokenUser query initialized a properly aligned TOKEN_USER.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_string(user.User.Sid)
}
pub(crate) fn current_sid() -> Result<String> {
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle, not owned/closed here.
    process_sid(unsafe { GetCurrentProcess() })
}
fn peer_sid(pid: u32) -> Result<String> {
    // SAFETY: PID comes only from the kernel's connected named-pipe peer query.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err("无法验证 IPC 对端用户".into());
    }
    process_sid(Handle(process).0)
}
fn descriptor() -> Result<Local> {
    let sid = current_sid()?;
    // Elevated tokens can default ownership to Administrators. Newly created
    // per-user objects must explicitly name the authenticated user's SID.
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: SDDL is NUL-terminated and descriptor output lives through the call.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err("无法建立私有 IPC ACL".into());
    }
    Ok(Local(descriptor))
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}
pub fn check_owned(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("配置路径不能包含 Windows reparse point".into());
    }
    let name = wide(path);
    let mut owner = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: name is NUL-terminated and all requested output pointers are valid.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err("无法验证配置文件所有者".into());
    }
    let _descriptor = Local(descriptor);
    if sid_string(owner)? != current_sid()? {
        return Err("配置文件必须属于当前用户".into());
    }
    Ok(())
}
pub fn protect_directory(path: &Path) -> Result<()> {
    check_owned(path)?;
    let descriptor = descriptor()?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    // SAFETY: descriptor is live and ACL/present/defaulted outputs are writable.
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
        || present == 0
        || acl.is_null()
    {
        return Err("私有目录 ACL 无效".into());
    }
    let name = wide(path);
    // SAFETY: ACL lives in descriptor; pathname is valid; only DACL is updated.
    if unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        )
    } != 0
    {
        return Err("无法保护后台目录".into());
    }
    Ok(())
}
pub fn check_directory(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err("后台状态目录不存在".into());
    }
    check_owned(path)?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    for ancestor in absolute.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("后台路径不能经过 reparse point".into());
        }
    }
    let name = wide(path);
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: pathname and ACL/descriptor outputs are valid for the call.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 || acl.is_null() {
        return Err("后台目录必须具有私有 DACL".into());
    }
    let _descriptor = Local(descriptor);
    let sid = current_sid()?;
    // SAFETY: ACL is inside the live returned descriptor; AceCount bounds GetAce.
    let count = unsafe { (*acl).AceCount };
    if count == 0 {
        return Err("后台目录 DACL 为空".into());
    }
    for index in 0..u32::from(count) {
        let mut ace = ptr::null_mut();
        // SAFETY: index is below AceCount and ace output is writable.
        if unsafe { GetAce(acl, index, &mut ace) } == 0 {
            return Err("无法检查目录 ACL".into());
        }
        // SAFETY: GetAce returns a live ACE_HEADER; we check its type before casting.
        if u32::from(unsafe { (*ace.cast::<windows_sys::Win32::Security::ACE_HEADER>()).AceType })
            != ACCESS_ALLOWED_ACE_TYPE
        {
            return Err("后台目录 ACL 含未允许的 ACE 类型".into());
        }
        // SAFETY: ACCESS_ALLOWED_ACE's SidStart begins the variable-length SID.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let ace_sid = (&allowed.SidStart as *const u32).cast_mut().cast();
        if sid_string(ace_sid)? != sid {
            return Err("后台目录允许其他用户访问".into());
        }
    }
    Ok(())
}
pub fn prepare(path: &Path) -> Result<()> {
    if path.exists() {
        if !path.is_dir() {
            return Err("后台状态路径必须是目录".into());
        }
        check_owned(path)?;
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.exists() {
            prepare(parent)?;
        }
        let descriptor = descriptor()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let name = wide(path);
        // SAFETY: pathname and descriptor live through creation. The explicit
        // owner/DACL apply atomically, before any IPC or credential is written.
        if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0
            && std::io::Error::last_os_error().raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32)
        {
            return Err("无法创建当前用户的私有目录".into());
        }
    }
    protect_directory(path)?;
    check_directory(path)
}
pub fn lock(directory: &Path) -> Result<File> {
    let path = directory.join("background.lock");
    if path.exists() {
        check_owned(&path)?;
    }
    let descriptor = descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let name = wide(&path);
    // SAFETY: descriptor/path remain live. OPEN_ALWAYS never truncates an
    // existing lock, and sharing=0 preserves exclusive background ownership.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &attributes,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("后台已运行或锁文件不可用".into());
    }
    // SAFETY: this valid handle is transferred exactly once to File for closing.
    let file = unsafe { File::from_raw_handle(handle) };
    if file
        .metadata()
        .map_err(|e| e.to_string())?
        .file_attributes()
        & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err("后台锁不能是 reparse point".into());
    }
    Ok(file)
}
fn pipe_name(directory: &Path, lifecycle: bool) -> Result<String> {
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let hash = Sha256::digest(directory.to_string_lossy().to_lowercase().as_bytes());
    Ok(format!(
        r"\\.\pipe\NeonMix.v1.{}.{:x}{}",
        current_sid()?,
        hash,
        if lifecycle { ".lifecycle" } else { "" }
    ))
}
fn create_pipe(name: &str, first: bool) -> Result<NamedPipeServer> {
    let descriptor = descriptor()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true)
        .max_instances(10)
        .in_buffer_size(MAX_MESSAGE_BYTES as u32)
        .out_buffer_size(MAX_MESSAGE_BYTES as u32);
    // SAFETY: attributes and descriptor remain valid through CreateNamedPipeW.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
    .map_err(|error| {
        format!(
            "ipc_pipe_create_failed:{}",
            error.raw_os_error().unwrap_or(-1)
        )
    })
}
pub type Stream = NamedPipeServer;
pub struct Listener {
    name: String,
    next: Option<NamedPipeServer>,
}
impl Listener {
    pub fn bind(directory: &Path) -> Result<Self> {
        Self::bind_endpoint(directory, false)
    }
    pub(crate) fn bind_endpoint(directory: &Path, lifecycle: bool) -> Result<Self> {
        let name = pipe_name(directory, lifecycle)?;
        let next = create_pipe(&name, true)?;
        Ok(Self {
            name,
            next: Some(next),
        })
    }
    pub async fn accept(&mut self) -> Result<Stream> {
        let current = self.next.as_mut().ok_or("missing pipe listener")?;
        current.connect().await.map_err(|_| "管道连接失败")?;
        let next = create_pipe(&self.name, false)?;
        Ok(self.next.replace(next).expect("existing named pipe"))
    }
}
pub fn same_user(stream: &NamedPipeServer) -> bool {
    let mut pid = 0;
    // SAFETY: stream owns the connected pipe handle and pid output is valid.
    (unsafe { GetNamedPipeClientProcessId(stream.as_raw_handle().cast(), &mut pid) }) != 0
        && matches!((peer_sid(pid),current_sid()),(Ok(peer),Ok(current))if peer==current)
}
pub fn request(directory: &Path, request: &Request) -> Result<Reply> {
    request_bound(directory, request, None)
}
pub fn request_bound(
    directory: &Path,
    request: &Request,
    expected_pid: Option<u32>,
) -> Result<Reply> {
    check_directory(directory)?;
    let name = pipe_name(directory, crate::manager::is_lifecycle(request))?;
    let bytes = serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        request: request.clone(),
    })
    .map_err(|_| "ipc_invalid_request".to_string())?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("ipc_message_too_large".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(crate::client_io_error)?;
    runtime.block_on(async {
        tokio::time::timeout(IPC_TIMEOUT, async {
            let mut stream = loop {
                match ClientOptions::new().open(&name) {
                    Ok(stream) => break stream,
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(Duration::from_millis(20)).await
                    }
                    Err(_) => return Err("connection_unavailable".into()),
                }
            };
            let mut pid = 0;
            // SAFETY: stream owns the live pipe; server PID output is valid.
            if unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle().cast(), &mut pid) } == 0
                || expected_pid.is_some_and(|expected| pid != expected)
                || !matches!((peer_sid(pid),current_sid()),(Ok(peer),Ok(current))if peer==current)
            {
                return Err("permission_denied".into());
            }
            stream
                .write_u32(bytes.len() as u32)
                .await
                .map_err(crate::client_io_error)?;
            stream
                .write_all(&bytes)
                .await
                .map_err(crate::client_io_error)?;
            let size = stream.read_u32().await.map_err(crate::client_io_error)? as usize;
            if size > MAX_MESSAGE_BYTES {
                return Err("ipc_message_too_large".into());
            }
            let mut body = vec![0; size];
            stream
                .read_exact(&mut body)
                .await
                .map_err(crate::client_io_error)?;
            serde_json::from_slice(&body).map_err(|_| "invalid_backend_response".into())
        })
        .await
        .map_err(|_| "background_timeout")?
    })
}
