//! Suspended spawn + private non-inheritable Job Object, before child code runs.
use std::{
    io,
    mem::{size_of, zeroed},
    ptr,
};
use tokio::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

// Store the uniquely owned handle as usize so the owner can move into a Tokio
// task. It is never copied, exported or inherited by the process tree.
pub struct Job(usize);
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: Job exclusively owns this live native handle.
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}
impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null security attributes make the unnamed handle non-inheritable.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle as usize);
        // SAFETY: zero is the documented initial value for the limit structure.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: pointer/length match the live limit structure and information class.
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: terminates only processes assigned to our private job.
        if unsafe { TerminateJobObject(self.0 as HANDLE, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn empty(&self) -> io::Result<bool> {
        // SAFETY: query fills a correctly sized accounting structure.
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        // SAFETY: live owned handle and matching output pointer/length.
        if unsafe {
            QueryInformationJobObject(
                self.0 as HANDLE,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses == 0)
    }
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: temporary owns the snapshot or thread handle exclusively.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn resume_primary(pid: u32) -> io::Result<()> {
    // Rust's Child exposes the process handle, but not its initial thread.
    // CREATE_SUSPENDED prevents child code/secondary threads from running.
    // SAFETY: snapshot has no pointer inputs and is owned below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Handle(snapshot);
    // SAFETY: zeroed THREADENTRY32 with its required size.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    let mut threads = Vec::new();
    // SAFETY: live snapshot and sized output structure.
    let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == pid {
            threads.push(entry.th32ThreadID);
        }
        // SAFETY: snapshot remains alive until enumeration ends.
        found = unsafe { Thread32Next(snapshot.0, &mut entry) };
    }
    if threads.len() != 1 {
        return Err(io::Error::other("managed_primary_thread_invalid"));
    }
    // SAFETY: thread belongs to the suspended Child whose handle is still held.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, threads[0]) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    let thread = Handle(thread);
    // SAFETY: assignment completed before the thread can run any child code.
    if unsafe { ResumeThread(thread.0) } != 1 {
        return Err(io::Error::other("managed_resume_failed"));
    }
    Ok(())
}
pub fn spawn(command: &mut Command) -> io::Result<(Child, Job)> {
    let job = Job::new()?;
    command
        .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let result = (|| {
        let handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("managed_process_handle_missing"))?;
        // SAFETY: valid owned child/job handles. No breakaway limits are enabled.
        if unsafe { AssignProcessToJobObject(job.0 as HANDLE, handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        resume_primary(
            child
                .id()
                .ok_or_else(|| io::Error::other("managed_pid_missing"))?,
        )
    })();
    if let Err(error) = result {
        // Child cannot run on failure; Tokio's reaper handles its wait on drop.
        let _ = child.start_kill();
        return Err(error);
    }
    Ok((child, job))
}
