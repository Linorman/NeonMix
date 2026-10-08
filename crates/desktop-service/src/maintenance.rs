//! User-mode installer stop client. Every process is pinned by handle and checked
//! against this installation's exact executable path and the original user SID.
#![allow(unsafe_code)]
use crate::{Request, Result, transport};
use std::{
    mem::{size_of, zeroed},
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_OBJECT_0},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, RegisterWindowMessageW,
    },
};
struct Process {
    handle: HANDLE,
    pid: u32,
    kind: usize,
}
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: exclusively owned live process handle.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
struct Snapshot(HANDLE);
impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: exclusively owned ToolHelp snapshot.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct WindowMessage {
    pid: u32,
    message: u32,
}
unsafe extern "system" fn close_window(window: HWND, data: LPARAM) -> i32 {
    // SAFETY: EnumWindows receives a pointer to a live stack structure.
    let target = unsafe { &*(data as *const WindowMessage) };
    let mut pid = 0;
    // SAFETY: window came from EnumWindows; pid output remains valid.
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if pid == target.pid {
        // SAFETY: fixed message has no pointer payload. It requests UI exit only.
        unsafe {
            PostMessageW(window, target.message, 0, 0);
        }
    }
    1
}
pub fn stop_installation(state: &Path, root: &Path) -> Result<()> {
    let root = root
        .canonicalize()
        .map_err(|_| "安装目录不可用".to_string())?;
    // Reject occupied shared directories before any recursive installer removal.
    let allowed = [
        "bin",
        "plugins",
        "airplay",
        "licenses",
        "NeonMix.exe",
        "Uninstall.exe",
        "neonmix-installation.json",
        "network-instance.ini",
        "network-journal.ini",
    ];
    for entry in std::fs::read_dir(&root).map_err(|_| "network_installation_invalid".to_string())? {
        let entry = entry.map_err(|_| "network_installation_invalid".to_string())?;
        if !allowed.iter().any(|name| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        }) {
            return Err("network_installation_invalid".into());
        }
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|_| "network_installation_invalid".to_string())?;
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("network_installation_invalid".into());
        }
    }
    let expected: Vec<_> = [
        "bin/neonmix-background.exe",
        "bin/neonmix-hub.exe",
        "airplay/bin/neonmix-airplay-worker.exe",
        "bin/neonmix-desktop.exe",
    ]
    .iter()
    .map(|p| {
        root.join(p)
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase()
    })
    .collect();
    let user = transport::current_sid()?;
    // SAFETY: no pointer arguments, snapshot is immediately owned.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err("无法枚举安装实例进程".into());
    }
    let snapshot = Snapshot(handle);
    // SAFETY: zeroed entry with documented required size.
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut processes = Vec::new();
    // SAFETY: live snapshot and matching sized structure.
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while found != 0 {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
        if entry.th32ProcessID != std::process::id()
            && expected.iter().any(|e| e.ends_with(&format!("\\{name}")))
        {
            // SAFETY: query/synchronize grants no termination authority.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    entry.th32ProcessID,
                )
            };
            if !handle.is_null() {
                let mut process = Process {
                    handle,
                    pid: entry.th32ProcessID,
                    kind: usize::MAX,
                };
                let mut path = vec![0u16; 32768];
                let mut count = path.len() as u32;
                // SAFETY: pinned process and correctly sized UTF-16 output.
                if unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut count) }
                    != 0
                    && transport::process_sid(handle)? == user
                {
                    let path = String::from_utf16_lossy(&path[..count as usize])
                        .trim_start_matches(r"\\?\")
                        .to_lowercase();
                    if let Some(kind) = expected.iter().position(|e| *e == path) {
                        process.kind = kind;
                        processes.push(process);
                    }
                }
            }
        }
        // SAFETY: snapshot remains held during enumeration.
        found = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    let backgrounds: Vec<_> = processes.iter().filter(|p| p.kind == 0).collect();
    if backgrounds.len() > 1 {
        return Err("安装实例有多个后台，无法确认归属".into());
    }
    if let Some(background) = backgrounds.first() {
        let status = transport::request_bound(state, &Request::Status, Some(background.pid))?;
        if !status.ok || status.data["managed_control_version"] != 1 {
            return Err("旧后台不支持协作式安装维护；请先手动退出该实例".into());
        }
        let reply = transport::request_bound(state, &Request::Shutdown, Some(background.pid))?;
        if !reply.ok {
            return Err(reply.error.unwrap_or_else(|| "后台停止未完成".into()));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    for process in processes.iter().filter(|p| p.kind < 3) {
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        // SAFETY: waiting on the pinned process object, not a possibly recycled PID.
        if unsafe { WaitForSingleObject(process.handle, remaining) } != WAIT_OBJECT_0 {
            return Err("安装目录仍有音频进程；请停止该实例或手动 CLI 后重试".into());
        }
    }
    let name: Vec<_> = "NeonMix.Maintenance.Exit.v1"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: fixed NUL-terminated message identifier.
    let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    if message == 0 {
        return Err("安装维护消息不可用".into());
    }
    for process in processes.iter().filter(|p| p.kind == 3) {
        let target = WindowMessage {
            pid: process.pid,
            message,
        };
        // SAFETY: callback is synchronous; target remains valid until it returns.
        unsafe {
            EnumWindows(
                Some(close_window),
                (&target as *const WindowMessage) as LPARAM,
            );
        }
        // SAFETY: owned process handle; ordinary background loss sends no message.
        if unsafe { WaitForSingleObject(process.handle, 2000) } != WAIT_OBJECT_0 {
            return Err("桌面尚未退出；请关闭该安装实例后重试".into());
        }
    }
    Ok(())
}
