//! Win32 private current-user objects. Validation uses the opened object handle.
use std::{
    ffi::c_void,
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_LOCK_VIOLATION, GENERIC_READ, GENERIC_WRITE,
        INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl,
        GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser, WinCreatorOwnerRightsSid,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        GetVolumeInformationByHandleW, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        LockFileEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING,
        UnlockFileEx,
    },
    System::{
        IO::OVERLAPPED,
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, FILE_PERSISTENT_ACLS},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn denied() -> io::Error {
    super::denied()
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    if path.as_os_str().encode_wide().any(|unit| unit == 0)
        || path.components().any(|component| matches!(component, Component::Normal(name) if name.encode_wide().any(|unit| unit == u16::from(b':')))) {
        return Err(denied());
    }
    Ok(path.as_os_str().encode_wide().chain([0]).collect())
}
/// Directory and every ancestor deny delete-sharing, so later pathname opens
/// cannot be redirected by renaming an ancestor or substituting a junction.
pub(super) struct PinnedDirectory {
    _file: File,
    _ancestors: Vec<File>,
    pub(super) path: PathBuf,
}
fn pin_ancestors(path: &Path) -> io::Result<Vec<File>> {
    let mut ancestors = Vec::new();
    for ancestor in path
        .ancestors()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let name = wide(ancestor)?;
        // SAFETY: pathname is NUL-terminated; returned handle is owned by File.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW returned a new valid owned handle.
        let file = unsafe { File::from_raw_handle(handle) };
        let metadata = file.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(denied());
        }
        ancestors.push(file);
    }
    Ok(ancestors)
}
pub(super) fn pin_directory(path: &Path) -> io::Result<PinnedDirectory> {
    let path = std::path::absolute(path)?;
    let ancestors = pin_ancestors(&path)?;
    let file = open(&path, true, false, false)?;
    Ok(PinnedDirectory {
        _file: file,
        _ancestors: ancestors,
        path,
    })
}

fn current_user() -> io::Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut size = 0;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65_536 {
        // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
        unsafe {
            CloseHandle(token);
        }
        return Err(denied());
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    };
    let failure = (result == 0).then(io::Error::last_os_error);
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    unsafe {
        CloseHandle(token);
    }
    if let Some(failure) = failure {
        return Err(failure);
    }
    Ok(buffer)
}
fn descriptor() -> io::Result<Local> {
    let user = current_user()?;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let user = unsafe { &*user.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = ptr::null_mut();
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _sid_buffer = Local(sid.cast());
    let mut len = 0;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    unsafe {
        while *sid.add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid, len) });
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
        .encode_utf16()
        .chain([0])
        .collect();
    let mut output = ptr::null_mut();
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut output,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Local(output))
}
fn check(file: &File, directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(denied());
    }
    let handle = file.as_raw_handle();
    let mut flags = 0;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe {
        GetVolumeInformationByHandleW(
            handle,
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(denied());
    }
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _descriptor = Local(descriptor);
    let user = current_user()?;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let user = unsafe { &*user.as_ptr().cast::<TOKEN_USER>() };
    let mut control = 0;
    let mut revision = 0;
    if owner.is_null()
        || acl.is_null()
        // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
        || unsafe { EqualSid(owner, user.User.Sid) } == 0
        // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
        || unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return Err(denied());
    }
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    for index in 0..unsafe { (*acl).AceCount } {
        let mut entry = ptr::null_mut();
        // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
        if unsafe { GetAce(acl, u32::from(index), &mut entry) } == 0 {
            return Err(denied());
        }
        // SAFETY: GetAce returned a live ACE pointer containing an ACE_HEADER.
        let header = unsafe { &*entry.cast::<ACE_HEADER>() };
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err(denied());
        }
        // SAFETY: header confirms the allow ACE type and minimum structure size.
        let entry = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
        // Legacy profiles used OWNER_RIGHTS; because ownership was already
        // verified above, that well-known SID grants only this same user.
        let sid = (&raw const entry.SidStart).cast_mut().cast();
        // SAFETY: ACE SID and token SID remain live inside their API buffers.
        if unsafe { EqualSid(sid, user.User.Sid) } == 0
            // SAFETY: sid points to the live SID payload of this allow ACE.
            && unsafe { IsWellKnownSid(sid, WinCreatorOwnerRightsSid) } == 0
        {
            return Err(denied());
        }
    }
    Ok(())
}
fn open(path: &Path, directory: bool, write: bool, share_delete: bool) -> io::Result<File> {
    let absolute = std::path::absolute(path)?;
    let _ancestors = pin_ancestors(&absolute)?;
    let name = wide(&absolute)?;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | if write { GENERIC_WRITE } else { 0 },
            FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 },
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let file = unsafe { File::from_raw_handle(handle) };
    check(&file, directory)?;
    Ok(file)
}
pub(super) fn open_private(path: &Path, directory: bool) -> io::Result<File> {
    open(path, directory, false, true)
}
pub(super) fn create(path: &Path) -> io::Result<File> {
    create_with_share(path, true)
}
fn create_with_share(path: &Path, share_delete: bool) -> io::Result<File> {
    let absolute = std::path::absolute(path)?;
    let _ancestors = pin_ancestors(&absolute)?;
    let name = wide(&absolute)?;
    let descriptor = descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 },
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let file = unsafe { File::from_raw_handle(handle) };
    if let Err(error) = check(&file, false) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(file)
}
pub(super) fn private_dir(path: &Path) -> io::Result<()> {
    let absolute = std::path::absolute(path)?;
    let _ancestors = pin_ancestors(&absolute)?;
    let name = wide(&absolute)?;
    let descriptor = descriptor()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(error);
        }
    }
    open_private(path, true).map(drop)
}
pub(super) fn replace(source: &Path, target: &Path) -> io::Result<()> {
    let source = std::path::absolute(source)?;
    let target = std::path::absolute(target)?;
    let _source_ancestors = pin_ancestors(&source)?;
    let _target_ancestors = pin_ancestors(&target)?;
    let source = wide(&source)?;
    let target = wide(&target)?;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub(super) fn publish_directory(source: &Path, target: &Path) -> io::Result<()> {
    let source = std::path::absolute(source)?;
    let target = std::path::absolute(target)?;
    let _source_ancestors = pin_ancestors(&source)?;
    let _target_ancestors = pin_ancestors(&target)?;
    let source = wide(&source)?;
    let target = wide(&target)?;
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub(super) fn lock(path: &Path) -> io::Result<File> {
    let file = match create_with_share(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open(path, false, true, false)?
        }
        Err(error) => return Err(error),
    };
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    let mut operation: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: Win32 pointers reference live aligned buffers or owned handles; outputs remain valid for the call.
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut operation,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        return Err(error);
    }
    Ok(file)
}

pub(super) fn unlock(file: &File) {
    // SAFETY: zeroed OVERLAPPED specifies the same byte range acquired by lock.
    let mut operation: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: the borrowed handle and OVERLAPPED remain live throughout unlocking.
    let _ = unsafe { UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut operation) };
}
