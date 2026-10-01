//! Win32 is isolated here; file data is never written before ACL-capable
//! creation succeeds. The protected DACL grants file access only to its owner.
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, GetVolumeInformationByHandleW,
    },
    System::SystemServices::FILE_PERSISTENT_ACLS,
};

pub(super) fn create(path: &Path) -> io::Result<File> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "private identity path must name a regular file",
        )
    })?;
    // Alternate data streams inherit an existing base file's ACL; CREATE_NEW
    // on a stream cannot establish this function's private-file guarantee.
    if name
        .encode_wide()
        .any(|unit| unit == 0 || unit == u16::from(b':'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "private identity path cannot contain NUL or an alternate data stream",
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // std canonicalization supplies the extended Win32 prefix for long paths.
    let absolute = std::fs::canonicalize(parent)?.join(name);
    let filename: Vec<u16> = absolute.as_os_str().encode_wide().chain([0]).collect();
    // OW is OWNER_RIGHTS, and P disables inherited ACEs. The OS assigns the
    // file's owner from the creating token; no user-supplied SID is accepted.
    let sddl: Vec<u16> = "D:P(A;;FA;;;OW)".encode_utf16().chain([0]).collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: NUL-terminated SDDL and writable output live through the call.
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
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: pathname and converted descriptor are valid for this call.
    // CREATE_NEW prevents ignoring the requested ACL on an existing file.
    let handle = unsafe {
        CreateFileW(
            filename.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    let failure = (handle == INVALID_HANDLE_VALUE).then(io::Error::last_os_error);
    // SAFETY: the descriptor was allocated by the conversion API with LocalAlloc.
    unsafe {
        LocalFree(descriptor);
    }
    if let Some(error) = failure {
        return Err(error);
    }
    // SAFETY: handle is valid and exclusively transferred to File, which closes it.
    let file = unsafe { File::from_raw_handle(handle) };
    let mut flags = 0;
    // SAFETY: the live file handle and flags output are valid; optional outputs are NULL.
    let result = unsafe {
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
    };
    let failure = if result == 0 {
        Some(io::Error::last_os_error())
    } else if flags & FILE_PERSISTENT_ACLS == 0 {
        Some(io::Error::new(
            io::ErrorKind::Unsupported,
            "private identity files require a filesystem with persistent ACLs",
        ))
    } else {
        None
    };
    if let Some(error) = failure {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(file)
}
