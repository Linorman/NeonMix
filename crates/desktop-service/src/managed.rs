//! Each lifecycle task owns Child until exit. Request timeout never drops Child.
use crate::{
    ProcessStatus, Result, StopResult,
    daemon::{audio_command, drain},
};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{io::AsyncWriteExt, process::Child, sync::watch};

#[derive(Clone)]
pub(crate) struct Owner {
    stop: watch::Sender<u64>,
    result: watch::Receiver<Option<Result<StopResult>>>,
}
pub(crate) struct Managed {
    pub(crate) child: Option<Owner>,
    status: Arc<Mutex<ProcessStatus>>,
}
impl Default for Managed {
    fn default() -> Self {
        Self {
            child: None,
            status: Arc::new(Mutex::new(ProcessStatus::default())),
        }
    }
}
impl Managed {
    pub(crate) fn status(&self) -> ProcessStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .observed_snapshot()
    }
    pub(crate) fn reap(&mut self) {
        if !self.status().running {
            self.child = None;
        }
    }
    pub(crate) fn start(&mut self, executable: &Path, mut args: Vec<String>) -> Result<()> {
        self.reap();
        if self.child.is_some() {
            return Err("process_already_running".into());
        }
        let ready_event = match args.first().map(String::as_str) {
            Some("serve") => Some("hub_started"),
            Some("send") => Some("sender_started"),
            _ => None,
        };
        let managed = args
            .first()
            .is_some_and(|a| matches!(a.as_str(), "serve" | "send"));
        let cleanup = args
            .windows(2)
            .find(|a| a[0] == "--config")
            .and_then(|a| Path::new(&a[1]).parent())
            .map(|p| p.join("airplay"));
        if managed {
            args.push("--managed-control-stdin".into());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let mut command = {
            let guardian = guardian_path(executable)?;
            let mut command = audio_command(
                &guardian,
                vec!["--child".into(), executable.to_string_lossy().into_owned()],
            );
            if managed {
                command.arg("--pipe-child");
            }
            // Closing the private writer is the guardian's parent-loss signal.
            // Do not SIGKILL the supervisor when a failed runtime drops Child;
            // it must survive long enough to reap the actual media tree.
            command
                .arg("--")
                .args(args)
                .stdin(Stdio::piped())
                .kill_on_drop(false);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mut command = {
            let mut command = audio_command(executable, args);
            if managed {
                command.stdin(Stdio::piped());
            }
            command
        };
        #[cfg(windows)]
        let (mut child, job) = neonmix_lifecycle::windows_job::spawn(&mut command)
            .map_err(|_| "runtime_unavailable".to_string())?;
        #[cfg(not(windows))]
        let mut child = command
            .spawn()
            .map_err(|_| "runtime_unavailable".to_string())?;
        self.status = Arc::new(Mutex::new(ProcessStatus {
            running: true,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            guardian_version: 1,
            owner_pid: child.id(),
            pid: child.id(),
            ..Default::default()
        }));
        let stdout = child.stdout.take().ok_or("missing child stdout")?;
        let stderr = child.stderr.take().ok_or("missing child stderr")?;
        let mut output = tokio::spawn(drain(stdout, self.status.clone(), false, ready_event));
        let mut errors = tokio::spawn(drain(stderr, self.status.clone(), true, None));
        let mut input = child.stdin.take();
        let (stop, requested) = watch::channel(0u64);
        let (completed, result) = watch::channel(None);
        let status = self.status.clone();
        tokio::spawn(async move {
            let mut requested = requested;
            let mut natural_exit = tokio::select! {
                exit = child.wait() => Some(exit),
                _ = requested.changed() => None, // Owner drop also requests stop.
            };
            let requested_stop = natural_exit.is_none();
            {
                let mut current = status.lock().unwrap_or_else(|p| p.into_inner());
                current.ready = false;
                current.stopping = true;
                if !requested_stop && current.fault.is_none() {
                    current.set_failure("process_exited");
                }
            }
            loop {
                let started = Instant::now();
                let natural = natural_exit.take();
                let result = finish(
                    &mut child,
                    &mut input,
                    natural.as_ref(),
                    managed || cfg!(any(target_os = "macos", target_os = "linux")),
                    cleanup.as_deref(),
                    &status,
                    #[cfg(windows)]
                    &job,
                )
                .await;
                if let Ok(mut result) = result {
                    // Keep draining until pipes close. A leaked descendant must
                    // not retain stdout forever; the job emptiness is checked first.
                    let _ = tokio::join!(
                        tokio::time::timeout(Duration::from_millis(250), &mut output),
                        tokio::time::timeout(Duration::from_millis(250), &mut errors)
                    );
                    let worker_forced = status
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .last_event
                        .as_ref()
                        .is_some_and(|e| e["forced"].as_bool() == Some(true));
                    result.forced |= worker_forced;
                    result.graceful &= !worker_forced;
                    let final_status = status.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    if final_status.guardian_version == 1 {
                        if let Some(guardian) = final_status.guardian_result {
                            result.forced |= guardian.forced;
                            result.graceful &= !guardian.forced;
                            result.cleanup_complete &=
                                guardian.version == 1 && guardian.cleanup_complete;
                            result.exit_code = guardian.child_exit_code;
                        } else {
                            result.cleanup_complete = false;
                            result.graceful = false;
                        }
                    }
                    result.elapsed_ms = started.elapsed().as_millis() as u64;
                    let mut state = status.lock().unwrap_or_else(|p| p.into_inner());
                    state.running = false;
                    state.ready = false;
                    state.stopping = false;
                    state.pid = None;
                    state.owner_pid = None;
                    if !result.cleanup_complete {
                        state.set_failure("runtime_cleanup_incomplete");
                    } else if !requested_stop && state.fault.is_none() {
                        state.set_failure("process_exited");
                    }
                    state.stop_result = Some(result.clone());
                    completed.send_replace(Some(Ok(result)));
                    break;
                } else {
                    let error = result.unwrap_err();
                    status
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .set_failure(&error);
                    completed.send_replace(Some(Err(error)));
                    // Retain Child and job on failed exit verification. A later
                    // Stop retries; a cancelled IPC does not cancel this owner.
                    if requested.changed().await.is_err() {
                        break;
                    }
                    completed.send_replace(None);
                }
            }
            output.abort();
            errors.abort();
        });
        self.child = Some(Owner { stop, result });
        Ok(())
    }
    pub(crate) async fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // Observe Child exit as well as stdout, including ready followed by
            // immediate exit. Readiness belongs to this status Arc, so readers
            // from an earlier process can never revive a replacement.
            tokio::time::sleep(Duration::from_millis(20)).await;
            let current = self.status();
            let failure = current.fault.as_ref().map(|fault| {
                serde_json::to_string(fault).unwrap_or_else(|_| "generic_failure".into())
            });
            if !current.running || current.stopping || failure.is_some() {
                let error = failure.unwrap_or_else(|| "process_exited".into());
                self.stop().await?;
                self.status
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set_failure(&error);
                return Err(error);
            }
            if current.ready {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                self.stop().await?;
                self.status
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set_failure("background_timeout");
                return Err("background_timeout".into());
            }
        }
    }
    pub(crate) fn signal(&self) -> ManagedSignal {
        ManagedSignal {
            owner: self.child.clone(),
            status: self.status.clone(),
        }
    }
    pub(crate) async fn stop(&mut self) -> Result<()> {
        self.signal().stop().await?;
        self.reap();
        Ok(())
    }
}

/// Cloning this control handle never transfers Child ownership. A disconnected
/// request drops only its subscription; the lifecycle task still reaps Child.
#[derive(Clone)]
pub(crate) struct ManagedSignal {
    owner: Option<Owner>,
    status: Arc<Mutex<ProcessStatus>>,
}
impl ManagedSignal {
    pub(crate) fn status(&self) -> ProcessStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .observed_snapshot()
    }
    pub(crate) async fn stop(&self) -> Result<StopResult> {
        let mut result = self.owner.as_ref().map(|owner| owner.result.clone());
        let retry;
        {
            let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
            if !status.running {
                let stopped = status.stop_result.clone().unwrap_or(StopResult {
                    graceful: true,
                    forced: false,
                    elapsed_ms: 0,
                    exit_code: None,
                    cleanup_complete: true,
                });
                return if stopped.cleanup_complete {
                    Ok(stopped)
                } else {
                    Err("runtime_cleanup_incomplete".into())
                };
            }
            let owner = self.owner.as_ref().ok_or("lifecycle_owner_lost")?;
            retry = result
                .as_mut()
                .and_then(|r| r.borrow_and_update().clone())
                .is_some_and(|r| r.is_err());
            if !status.stopping || retry {
                status.stopping = true;
                status.ready = false;
                owner.stop.send_modify(|n| *n = n.saturating_add(1));
            }
        }
        let mut result = result.ok_or("lifecycle_owner_lost")?;
        if retry {
            result.changed().await.map_err(|_| "lifecycle_owner_lost")?;
        }
        loop {
            let latest = result.borrow_and_update().clone();
            if let Some(latest) = latest {
                let stopped = latest?;
                return if stopped.cleanup_complete {
                    Ok(stopped)
                } else {
                    Err("runtime_cleanup_incomplete".into())
                };
            }
            result.changed().await.map_err(|_| "lifecycle_owner_lost")?;
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn guardian_path(executable: &Path) -> Result<PathBuf> {
    let sibling = executable
        .parent()
        .ok_or("runtime_unavailable")?
        .join("neonmix-guardian");
    if sibling.is_file() {
        return Ok(sibling);
    }
    #[cfg(test)]
    {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("runtime_unavailable")?
            .parent()
            .ok_or("runtime_unavailable")?
            .to_path_buf();
        let candidate = project.join("target/debug/neonmix-guardian");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("runtime_unavailable".into())
}

async fn finish(
    child: &mut Child,
    input: &mut Option<tokio::process::ChildStdin>,
    natural: Option<&std::io::Result<std::process::ExitStatus>>,
    managed: bool,
    cleanup: Option<&Path>,
    status: &Arc<Mutex<ProcessStatus>>,
    #[cfg(windows)] job: &neonmix_lifecycle::windows_job::Job,
) -> Result<StopResult> {
    // The Unix guardian has a 5 s grace and 2 s group verification budget.
    // Retain it long enough to enforce that budget after the parent is gone.
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(if cfg!(any(target_os = "macos", target_os = "linux")) {
            9
        } else {
            5
        });
    let mut forced = false;
    let exit = if let Some(exit) = natural {
        *exit.as_ref().map_err(|_| "stop_failed".to_string())?
    } else {
        if managed {
            if let Some(mut input) = input.take() {
                let _ = tokio::time::timeout(
                    Duration::from_millis(250),
                    input.write_all(neonmix_lifecycle::STOP_LINE),
                )
                .await;
                // Drop also supplies EOF if the write failed or child lost its owner.
            }
        } else {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                terminate(pid)?;
            }
        }
        match tokio::time::timeout_at(deadline, child.wait()).await {
            Ok(exit) => exit.map_err(|_| "stop_failed".to_string())?,
            Err(_) => {
                forced = true;
                #[cfg(windows)]
                job.terminate().map_err(|_| "stop_failed".to_string())?;
                #[cfg(not(windows))]
                child.start_kill().map_err(|_| "stop_failed".to_string())?;
                tokio::time::timeout(Duration::from_secs(2), child.wait())
                    .await
                    .map_err(|_| "stop_incomplete".to_string())?
                    .map_err(|_| "stop_failed".to_string())?
            }
        }
    };
    #[cfg(windows)]
    {
        // Job accounting may lag a signalled process handle briefly. Give
        // already exiting descendants a bounded drain before forced cleanup.
        let settled = tokio::time::Instant::now() + Duration::from_millis(250);
        while !job.empty().map_err(|_| "stop_failed".to_string())?
            && tokio::time::Instant::now() < settled
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if !job.empty().map_err(|_| "stop_failed".to_string())? {
            forced = true;
            job.terminate().map_err(|_| "stop_failed".to_string())?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                if job.empty().map_err(|_| "stop_failed".to_string())? {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("stop_incomplete".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    let worker_forced = status
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .last_event
        .as_ref()
        .is_some_and(|e| e["forced"].as_bool() == Some(true));
    forced |= worker_forced;
    let cleanup_complete = cleanup.is_none_or(runtime_files_absent);
    Ok(StopResult {
        graceful: !forced && exit.success() && cleanup_complete,
        forced,
        elapsed_ms: 0,
        exit_code: exit.code(),
        cleanup_complete,
    })
}
fn runtime_files_absent(root: &Path) -> bool {
    if !root.exists() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| uuid::Uuid::parse_str(n).is_ok())
        {
            dirs.push(entry.path());
        }
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            if entry.file_name().to_str().is_some_and(|n| {
                n.strip_prefix("runtime-key-").is_some_and(|id| {
                    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
                })
            }) {
                return false;
            }
        }
    }
    true
}
#[cfg(unix)]
#[allow(unsafe_code)]
fn terminate(pid: u32) -> Result<()> {
    // SAFETY: pid comes from our owned live Child, never a request argument.
    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0
        && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return Err("stop_failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let path =
            std::env::var_os("NEONMIX_LIFECYCLE_PROBE").expect("set project-local probe path");
        PathBuf::from(path)
    }
    #[tokio::test]
    #[ignore = "requires the project-local neonmix-lifecycle-probe fixture"]
    async fn cancelled_stop_completes_and_fifty_restarts_are_graceful() {
        let mut process = Managed::default();
        for _ in 0..50 {
            process.start(&fixture(), vec!["serve".into()]).unwrap();
            let cancellation =
                tokio::time::timeout(Duration::from_millis(20), process.stop()).await;
            assert!(cancellation.is_err());
            assert!(process.start(&fixture(), vec!["serve".into()]).is_err());
            // The actor continues even when no stop request is awaiting it.
            tokio::time::sleep(Duration::from_millis(220)).await;
            process.stop().await.unwrap();
            let status = process.status();
            assert!(!status.running);
            let result = status.stop_result.unwrap();
            assert!(
                result.graceful && !result.forced && result.cleanup_complete,
                "{result:?}"
            );
            assert_eq!(result.exit_code, Some(0));
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "requires the project-local neonmix-lifecycle-probe fixture"]
    fn failed_runtime_drop_keeps_guardian_alive_until_its_hung_media_exits() {
        let executable = fixture();
        let (owner, media) = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let ids = runtime.block_on(async {
                let mut process = Managed::default();
                process
                    .start(&executable, vec!["serve".into(), "--ignore-stop".into()])
                    .unwrap();
                let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
                loop {
                    let status = process.status();
                    if status.ready && status.pid != status.owner_pid {
                        break (status.owner_pid.unwrap(), status.pid.unwrap());
                    }
                    assert!(tokio::time::Instant::now() < deadline);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                // process/Runtime teardown closes the pipe while cancelling
                // its actor. No surviving Rust task can enforce this timeout.
            });
            drop(runtime);
            ids
        })
        .join()
        .unwrap();
        let before = Instant::now();
        #[allow(unsafe_code)]
        let alive = |pid: u32| {
            // SAFETY: these are the two fresh processes returned by this test.
            unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
        };
        let mut owner_reaped = false;
        while before.elapsed() < Duration::from_secs(8) {
            if !owner_reaped {
                let mut status = 0;
                #[allow(unsafe_code)]
                // SAFETY: this is the direct child of this test process. The
                // cancelled runtime cannot reap its orphan without a driver;
                // observing/reaping it must not rely on another concurrent test.
                let result =
                    unsafe { libc::waitpid(owner as libc::pid_t, &mut status, libc::WNOHANG) };
                owner_reaped = result == owner as libc::pid_t
                    || (result < 0
                        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD));
            }
            if owner_reaped && !alive(media) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let retained = (!owner_reaped, alive(media));
        if retained.0 || retained.1 {
            #[allow(unsafe_code)]
            // SAFETY: cleanup only this fixture's still-live allocated PIDs.
            unsafe {
                if retained.1 {
                    libc::kill(media as libc::pid_t, libc::SIGKILL);
                }
                if retained.0 {
                    libc::kill(owner as libc::pid_t, libc::SIGKILL);
                }
            }
        }
        assert_eq!(
            retained,
            (false, false),
            "runtime teardown killed supervisor before media cleanup"
        );
    }
    #[tokio::test]
    #[ignore = "requires the project-local neonmix-lifecycle-probe fixture"]
    async fn hung_process_is_forced_with_a_bounded_wait() {
        let mut process = Managed::default();
        process
            .start(&fixture(), vec!["serve".into(), "--ignore-stop".into()])
            .unwrap();
        let before = Instant::now();
        process.stop().await.unwrap();
        assert!(before.elapsed() < Duration::from_secs(8));
        let result = process.status().stop_result.unwrap();
        assert!(!result.graceful && result.forced && result.cleanup_complete);
    }
}

#[cfg(all(test, unix))]
mod ready_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[tokio::test]
    async fn five_real_child_startup_cases_use_latched_readiness_and_confirmed_exit() {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let directory = project
            .join(".local/tmp")
            .join(format!("ready-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let fixture = Fixture(directory);
        for (name, script, expected) in [
            ("immediate-exit", "exit 1", false),
            ("never-ready", "IFS= read -r stop; exit 0", false),
            (
                "late-ready",
                "sleep .2; printf '{\"event\":\"hub_started\"}\n'; IFS= read -r stop; exit 0",
                false,
            ),
            (
                "ready-then-log",
                "printf '{\"event\":\"hub_started\"}\n{\"event\":\"hub_stats\"}\n'; IFS= read -r stop; exit 0",
                true,
            ),
            (
                "ready-then-exit",
                "printf '{\"event\":\"hub_started\"}\n'; exit 1",
                false,
            ),
        ] {
            let executable = fixture.0.join(name);
            std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut managed = Managed::default();
            managed.start(&executable, vec!["serve".into()]).unwrap();
            let pid = managed.status().pid.unwrap();
            let start = Instant::now();
            let result = managed
                .wait_ready(if expected {
                    Duration::from_secs(5)
                } else {
                    Duration::from_millis(100)
                })
                .await;
            assert_eq!(
                result.is_ok(),
                expected,
                "{name}: {result:?}, {:?}",
                managed.status()
            );
            if expected {
                let status = managed.status();
                assert!(status.ready);
                assert_eq!(
                    status.last_event.unwrap()["event"],
                    "hub_stats",
                    "ordinary log must not overwrite ready"
                );
                assert!(start.elapsed() < Duration::from_secs(5));
                managed.stop().await.unwrap();
            } else {
                assert!(
                    !managed.status().ready,
                    "late reader revived failed startup"
                );
                assert!(!managed.status().running, "failed start leaked a child");
            }
            assert!(!managed.status().ready);
            #[allow(unsafe_code)]
            // SAFETY: zero-signal checks only this fixture's exited child PID.
            unsafe {
                assert_eq!(
                    libc::kill(pid as libc::pid_t, 0),
                    -1,
                    "{name}: child was not reaped"
                );
            }
        }
    }
}
