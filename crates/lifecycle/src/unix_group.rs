//! An unreaped direct child pins its process-group ID, including after exit.
//! Signals only target that owned group; no name lookup or disk PID adoption.
use std::{
    io,
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus},
};

pub struct OwnedGroup {
    child: Child,
    pid: libc::pid_t,
    reaped: bool,
}
impl OwnedGroup {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        command.process_group(0);
        let child = command.spawn()?;
        let pid = libc::pid_t::try_from(child.id())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok(Self {
            child,
            pid,
            reaped: false,
        })
    }
    pub fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.child.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.child.stderr.take()
    }
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }
    /// Observe without reaping: WNOWAIT reserves the PID while descendants
    /// are checked/killed. try_wait/wait before group cleanup would allow reuse.
    pub fn exited(&self) -> io::Result<bool> {
        // SAFETY: zeroed siginfo is valid output storage; the PID belongs to
        // this unreaped Child. WNOHANG/WNOWAIT never consumes its wait status.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: info is writable; the identifier is our live/unreaped child.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(error)
            };
        }
        #[cfg(target_os = "macos")]
        let observed = info.si_pid;
        #[cfg(target_os = "linux")]
        // SAFETY: waitid initializes the SIGCHLD fields of this siginfo.
        let observed = unsafe { info.si_pid() };
        Ok(observed == self.pid)
    }
    pub fn signal(&self, signal: libc::c_int) -> io::Result<()> {
        if self.reaped {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        // SAFETY: group was created for this Child before user code; retaining
        // its unreaped leader prevents another process from reusing that ID.
        if unsafe { libc::kill(-self.pid, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn has_descendants(&self) -> io::Result<bool> {
        group_has_others(self.pid)
    }
    pub fn reap(&mut self) -> io::Result<ExitStatus> {
        if !self.exited()? || self.has_descendants()? {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let exit = self.child.wait()?;
        self.reaped = true;
        Ok(exit)
    }
}
impl Drop for OwnedGroup {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.signal(libc::SIGKILL);
            // Drop must not wait forever for an uninterruptible kernel task.
            // Reaping is explicit on the verified normal path above.
            if self.exited().unwrap_or(false) {
                let _ = self.child.wait();
            }
        }
    }
}
#[cfg(target_os = "macos")]
fn group_has_others(pid: libc::pid_t) -> io::Result<bool> {
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_listpids(
            kind: u32,
            group: u32,
            buffer: *mut libc::c_void,
            bytes: libc::c_int,
        ) -> libc::c_int;
    }
    let mut members = [0 as libc::pid_t; 4096];
    // SAFETY: the kernel writes only within this supplied PID buffer.
    let bytes = unsafe {
        proc_listpids(
            2, // PROC_PGRP_ONLY, declared in macOS sys/proc_info.h.
            pid as u32,
            members.as_mut_ptr().cast(),
            std::mem::size_of_val(&members) as libc::c_int,
        )
    };
    if bytes < 0 {
        return Err(io::Error::last_os_error());
    }
    let count = bytes as usize / std::mem::size_of::<libc::pid_t>();
    if count >= members.len() {
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    Ok(members[..count]
        .iter()
        .any(|member| *member > 0 && *member != pid))
}
#[cfg(target_os = "linux")]
fn group_has_others(pid: libc::pid_t) -> io::Result<bool> {
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(other) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if other == pid {
            continue;
        }
        let text = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(text) => text,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some((_, fields)) = text.rsplit_once(')')
            && fields
                .split_whitespace()
                .nth(2)
                .and_then(|group| group.parse::<libc::pid_t>().ok())
                == Some(pid)
        {
            return Ok(true);
        }
    }
    Ok(false)
}
