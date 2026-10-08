//! Private parent-owned stop pipes. No LAN endpoint or arbitrary command.
use serde::Deserialize;
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::watch;

mod run_limit;
pub use run_limit::{RunEndReason, RunLimit};

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(unsafe_code)]
pub mod unix_group;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows_job;

pub const STOP_LINE: &[u8] = b"{\"version\":1,\"type\":\"stop\"}\n";
pub const MAX_CONTROL_BYTES: usize = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopCommand {
    version: u16,
    #[serde(rename = "type")]
    kind: String,
}
pub fn valid_stop(line: &[u8]) -> bool {
    line.len() <= MAX_CONTROL_BYTES
        && serde_json::from_slice::<StopCommand>(line)
            .is_ok_and(|c| c.version == 1 && c.kind == "stop")
}

/// The reader only reads ready pipe data, with a 20 ms polling deadline.
/// Drop cancels and joins it; no Tokio blocking-stdin task survives runtime exit.
pub struct StopSignal {
    #[cfg(not(unix))]
    managed: bool,
    stopped: watch::Receiver<bool>,
    cancel: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    _idle: Option<watch::Sender<bool>>,
}
impl StopSignal {
    pub fn new(managed: bool) -> io::Result<Self> {
        let (notify, stopped) = watch::channel(false);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut idle = None;
        let reader = if managed {
            platform::prepare_input()?;
            let cancelled = cancel.clone();
            Some(
                std::thread::Builder::new()
                    .name("managed-stop".into())
                    .spawn(move || {
                        let mut line = Vec::with_capacity(MAX_CONTROL_BYTES);
                        while !cancelled.load(Ordering::Acquire) {
                            match platform::read_byte() {
                                Ok(Some(b'\n')) => {
                                    if !valid_stop(&line) {
                                        eprintln!("managed_control_invalid");
                                    }
                                    break;
                                }
                                Ok(Some(byte)) if line.len() < MAX_CONTROL_BYTES => line.push(byte),
                                Ok(None) => continue,
                                _ => break, // EOF, overlong line or IO failure loses the owner.
                            }
                        }
                        notify.send_replace(true);
                    })?,
            )
        } else {
            // Keep the channel open without creating a reader for manual CLI.
            idle = Some(notify);
            None
        };
        Ok(Self {
            #[cfg(not(unix))]
            managed,
            stopped,
            cancel,
            reader,
            _idle: idle,
        })
    }
    pub async fn wait(&self) -> io::Result<()> {
        let mut stopped = self.stopped.clone();
        let private = async {
            while !*stopped.borrow_and_update() {
                if stopped.changed().await.is_err() {
                    break;
                }
            }
            Ok(())
        };
        #[cfg(unix)]
        {
            let mut terminated =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                result = private => result,
                result = tokio::signal::ctrl_c() => result,
                _ = terminated.recv() => Ok(()),
            }
        }
        #[cfg(not(unix))]
        {
            // CREATE_NO_WINDOW owners have no console. Registering a console
            // handler can fail immediately; the private pipe is their signal.
            if self.managed {
                private.await
            } else {
                tokio::signal::ctrl_c().await
            }
        }
    }
}
impl Drop for StopSignal {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Configure a child control pipe for immediate writes. Call before transferring
/// it to a dedicated writer, including before the startup configuration write.
pub fn prepare_writer(pipe: &std::process::ChildStdin) -> io::Result<()> {
    platform::prepare_writer(pipe)
}
pub fn write_bounded(
    pipe: &mut std::process::ChildStdin,
    mut bytes: &[u8],
    cancel: &AtomicBool,
    budget: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + budget;
    while !bytes.is_empty() {
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "control_write_timeout",
            ));
        }
        match pipe.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if platform::retry_write(&e) => std::thread::sleep(Duration::from_millis(2)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod platform {
    use super::*;
    use std::os::fd::AsRawFd;
    pub fn prepare_input() -> io::Result<()> {
        Ok(())
    }
    pub fn read_byte() -> io::Result<Option<u8>> {
        let mut fd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll receives one live descriptor; reads are limited to one byte.
        let result = unsafe { libc::poll(&mut fd, 1, 20) };
        if result == 0 {
            return Ok(None);
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let mut byte = 0;
        // SAFETY: stdin remains valid during the joined reader's lifetime.
        if unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) } != 1 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(Some(byte))
    }
    pub fn prepare_writer(pipe: &std::process::ChildStdin) -> io::Result<()> {
        // SAFETY: pipe owns this descriptor; only its status flags are changed.
        let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: valid descriptor and flags; no pointer arguments.
        let changed =
            unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if changed < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn retry_write(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        )
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::*;
    use std::{os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_NO_DATA, HANDLE_FLAG_INHERIT, SetHandleInformation},
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType, ReadFile},
        System::{
            Console::{GetStdHandle, STD_INPUT_HANDLE},
            Pipes::{PIPE_NOWAIT, PIPE_READMODE_BYTE, PeekNamedPipe, SetNamedPipeHandleState},
        },
    };
    pub fn prepare_input() -> io::Result<()> {
        // SAFETY: standard handle belongs to this process; do not close it here.
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        // SAFETY: type query uses the current standard handle.
        let kind = unsafe { GetFileType(handle) };
        // SAFETY: prevent this process's owned stdin handle from being inherited.
        let private = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        if kind != FILE_TYPE_PIPE || private == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "managed_stdin_requires_pipe",
            ));
        }
        Ok(())
    }
    pub fn read_byte() -> io::Result<Option<u8>> {
        let mut available = 0;
        // SAFETY: reader joins before the standard input handle is released.
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        // SAFETY: no payload is copied by Peek; available is a live output.
        if unsafe {
            PeekNamedPipe(
                handle,
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
        if available == 0 {
            std::thread::sleep(Duration::from_millis(20));
            return Ok(None);
        }
        let mut byte = 0u8;
        let mut count = 0;
        // SAFETY: only this reader consumes stdin; one byte is known ready.
        if unsafe { ReadFile(handle, &mut byte, 1, &mut count, ptr::null_mut()) } == 0 || count != 1
        {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(Some(byte))
    }
    pub fn prepare_writer(pipe: &std::process::ChildStdin) -> io::Result<()> {
        let mode = PIPE_NOWAIT | PIPE_READMODE_BYTE;
        // SAFETY: owned anonymous pipe supports immediate byte-mode writes.
        if unsafe { SetNamedPipeHandleState(pipe.as_raw_handle(), &mode, ptr::null(), ptr::null()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn retry_write(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
        ) || error.raw_os_error() == Some(ERROR_NO_DATA as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_stop_protocol_rejects_versions_fields_commands_and_garbage() {
        assert!(valid_stop(STOP_LINE));
        for line in [
            br#"{"version":2,"type":"stop"}"#.as_slice(),
            br#"{"version":1,"type":"start"}"#,
            br#"{"version":1,"type":"stop","command":"shell"}"#,
            br#"{}"#,
            b"not json",
        ] {
            assert!(!valid_stop(line));
        }
        assert!(!valid_stop(&vec![b' '; MAX_CONTROL_BYTES + 1]));
    }
}
