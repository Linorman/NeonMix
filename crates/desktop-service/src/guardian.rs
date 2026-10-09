//! Unix parent-death supervisor. The ordinary daemon owns only its stdin
//! writer; this process owns media and survives daemon SIGKILL long enough to
//! apply the same Stop protocol, then upgrade to killing its isolated group.
use neonmix_lifecycle::{StopSignal, unix_group::OwnedGroup};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[allow(unsafe_code)]
fn prepare_output() -> io::Result<()> {
    // SAFETY: install before launching any threads; a closed parent log pipe
    // is an expected owner-loss condition, not a reason to kill the supervisor.
    if unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) } == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    for descriptor in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: process-owned standard descriptors; only status flags change.
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the descriptor is live and the flags preserve other settings.
        if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
fn write_output(bytes: &[u8], stderr: bool) -> io::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_millis(250);
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let result = if stderr {
            io::stderr().write(remaining)
        } else {
            io::stdout().write(remaining)
        };
        match result {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => remaining = &remaining[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                if std::time::Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
struct RuntimeKey {
    directory: neonmix_identity::files::PrivateDirectory,
    name: String,
    dev: u64,
    ino: u64,
}
struct Cleanup {
    root: std::path::PathBuf,
    owner_lock: Option<std::path::PathBuf>,
    keys: Vec<RuntimeKey>,
    snapshot_valid: bool,
}
impl Cleanup {
    fn from_args(args: &[OsString]) -> Option<Self> {
        if args.first().is_none_or(|arg| arg != "serve") {
            return None;
        }
        let profile = args
            .windows(2)
            .find(|pair| pair[0] == "--config")
            .map(|pair| std::path::PathBuf::from(&pair[1]))?;
        let parent = profile.parent()?;
        let owner_lock = neonmix_identity::profiles::hub(&profile)
            .ok()
            .and_then(|metadata| neonmix_identity::profiles::state_path(&profile, &metadata).ok())
            .map(|state| state.with_extension("lock"));
        Some(Self {
            root: parent.join("airplay"),
            owner_lock,
            keys: Vec::new(),
            snapshot_valid: true,
        })
    }
    fn capture(&mut self) {
        match self.snapshot() {
            Ok(keys) => {
                self.keys = keys;
                self.snapshot_valid = true;
            }
            Err(_) => self.snapshot_valid = false,
        }
    }
    fn snapshot(&self) -> io::Result<Vec<RuntimeKey>> {
        use std::os::unix::fs::MetadataExt;
        if !self.root.try_exists()? {
            return Ok(Vec::new());
        }
        let _root = neonmix_identity::files::PrivateDirectory::open(&self.root)?;
        let mut directories = vec![self.root.clone()];
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_name().to_str().is_some_and(canonical_uuid) {
                if directories.len() == 5 {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
                directories.push(entry.path());
            }
        }
        let mut keys = Vec::new();
        for path in directories {
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                let Some(name) = entry
                    .file_name()
                    .to_str()
                    .filter(|name| {
                        name.strip_prefix("runtime-key-")
                            .is_some_and(canonical_uuid)
                    })
                    .map(str::to_owned)
                else {
                    continue;
                };
                if keys.len() == 128 {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
                let directory = neonmix_identity::files::PrivateDirectory::open(&path)?;
                let metadata = directory.metadata(&name)?;
                keys.push(RuntimeKey {
                    directory,
                    name,
                    dev: metadata.dev(),
                    ino: metadata.ino(),
                });
            }
        }
        Ok(keys)
    }
    fn finish(self) -> Vec<String> {
        use std::os::unix::fs::MetadataExt;
        if !self.snapshot_valid {
            return vec!["runtime_key_snapshot_failed".into()];
        }
        if self.keys.is_empty() {
            return Vec::new();
        }
        let Some(lock) = self.owner_lock else {
            return vec!["runtime_key_scope_unavailable".into()];
        };
        let Ok(_owner) = neonmix_identity::profiles::operation_lock(&lock) else {
            return vec!["owner_lock_busy".into()];
        };
        let mut failures = std::collections::BTreeSet::new();
        for key in self.keys {
            match key.directory.metadata(&key.name) {
                Ok(metadata) if metadata.dev() == key.dev && metadata.ino() == key.ino => {
                    if key.directory.remove(&key.name).is_err() {
                        failures.insert("runtime_key_remove_failed");
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                _ => {
                    failures.insert("runtime_key_changed");
                }
            }
        }
        failures.into_iter().map(str::to_owned).collect()
    }
}
fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn emit(value: serde_json::Value) {
    let _ = write_output(format!("{value}\n").as_bytes(), false);
}
fn pump(
    mut source: impl Read + Send + 'static,
    stderr: bool,
    ready: Arc<AtomicBool>,
    expected_ready: Option<&'static str>,
) -> std::thread::JoinHandle<io::Result<()>> {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        let mut line = Vec::with_capacity(512);
        let mut oversized = false;
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            if let Some(expected) = expected_ready {
                for byte in &buffer[..count] {
                    if *byte == b'\n' {
                        if !oversized
                            && serde_json::from_slice::<serde_json::Value>(&line)
                                .is_ok_and(|event| event["event"] == expected)
                        {
                            ready.store(true, Ordering::Release);
                        }
                        line.clear();
                        oversized = false;
                    } else if line.len() < 16_384 {
                        line.push(*byte);
                    } else {
                        oversized = true;
                    }
                }
            }
            let result = write_output(&buffer[..count], stderr);
            // Keep draining after parent EOF to prevent a full log pipe from
            // becoming a media shutdown dependency.
            if let Err(error) = result
                && error.kind() != io::ErrorKind::BrokenPipe
            {
                return Err(error);
            }
        }
    })
}
/// A pipe can remain open even after the owned process group is empty (for
/// example an inherited descriptor). Logs must not hold the guardian alive or
/// prevent its final cleanup receipt. The binary exits after run returns.
async fn finish_pumps(
    output: std::thread::JoinHandle<io::Result<()>>,
    errors: std::thread::JoinHandle<io::Result<()>>,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
    while !(output.is_finished() && errors.is_finished()) && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for reader in [output, errors] {
        if reader.is_finished() {
            let _ = reader.join();
        }
    }
}

pub async fn run(executable: &Path, args: &[OsString], pipe_child: bool) -> io::Result<i32> {
    prepare_output()?;
    let stop = StopSignal::new(true)?;
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(if pipe_child {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut owned = OwnedGroup::spawn(&mut command)?;
    let mut input = owned.take_stdin();
    emit(
        serde_json::json!({"event":"managed_child_spawned","guardian_version":1,"ready_version":1,"pid":owned.pid()}),
    );
    let ready = Arc::new(AtomicBool::new(false));
    let expected_ready = match args.first().and_then(|arg| arg.to_str()) {
        Some("serve") => Some("hub_started"),
        Some("send") => Some("sender_started"),
        _ => None,
    };
    let output = pump(
        owned
            .take_stdout()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?,
        false,
        ready.clone(),
        expected_ready,
    );
    let errors = pump(
        owned
            .take_stderr()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?,
        true,
        ready.clone(),
        None,
    );
    let stopped = stop.wait();
    tokio::pin!(stopped);
    let mut deadline = None;
    let mut forced = false;
    let mut cleanup = Cleanup::from_args(args);
    let mut next_snapshot = tokio::time::Instant::now();
    let mut readiness_observed = None;
    let mut readiness_confirmed = false;
    let mut stage = "observe_child";
    let result:io::Result<_>=async {
        loop {
            stage="observe_child";
            if owned.exited()? {break;}
            if ready.load(Ordering::Acquire)&&!readiness_confirmed&&deadline.is_none() {
                let observed=*readiness_observed.get_or_insert_with(tokio::time::Instant::now);
                // Business readiness must survive a short startup observation,
                // and this owner checks the actual unreaped Child again first.
                // A line immediately followed by exit is never a Ready proof.
                if observed.elapsed()>=Duration::from_millis(50) {
                    emit(serde_json::json!({"event":"managed_child_ready","guardian_version":1,"pid":owned.pid(),"kind":expected_ready}));
                    readiness_confirmed=true;
                }
            }
            if tokio::time::Instant::now()>=next_snapshot {
                if let Some(cleanup)=cleanup.as_mut() {cleanup.capture();}
                next_snapshot=tokio::time::Instant::now()+Duration::from_millis(100);
            }
            if deadline.is_none() {
                tokio::select! {
                    result=&mut stopped=>{
                        result?;
                        deadline=Some(tokio::time::Instant::now()+Duration::from_secs(5));
                        if let Some(mut pipe)=input.take() {
                            neonmix_lifecycle::prepare_writer(&pipe)?;
                            let _=neonmix_lifecycle::write_bounded(&mut pipe,neonmix_lifecycle::STOP_LINE,&AtomicBool::new(false),Duration::from_millis(250));
                        }else{owned.signal(libc::SIGTERM)?;}
                    }
                    _=tokio::time::sleep(Duration::from_millis(20))=>{}
                }
            }else{
                if deadline.is_some_and(|deadline|tokio::time::Instant::now()>=deadline) {
                    if let Some(cleanup)=cleanup.as_mut() {cleanup.capture();}
                    stage="force_group";
                    owned.signal(libc::SIGKILL)?;forced=true;break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        emit(serde_json::json!({"event":"managed_child_exited","guardian_version":1,"pid":owned.pid()}));
        // A media leader exiting unexpectedly also loses its descendants.
        stage="verify_group";
        if !forced&&owned.has_descendants()? {
            stage="force_descendants";
            match owned.signal(libc::SIGKILL) {
                Ok(())=>forced=true,
                // Darwin can reject signalling an all-zombie group. Keep the
                // leader pinned and verify disappearance instead of declaring
                // failure or treating EPERM as proof that resources are gone.
                Err(error) if error.raw_os_error()==Some(libc::EPERM)=>{},
                Err(error)=>return Err(error),
            }
        }
        let settle=tokio::time::Instant::now()+Duration::from_secs(2);
        stage="reap_child";
        loop {
            match owned.reap() {
                Ok(exit)=>return Ok(exit),
                Err(error) if matches!(error.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::Interrupted)=> {
                    if tokio::time::Instant::now()>=settle {return Err(io::ErrorKind::TimedOut.into());}
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error)=>return Err(error),
            }
        }
    }.await;
    if result.is_err() {
        let _ = owned.signal(libc::SIGKILL);
    }
    // On error a remaining process could retain stdout. Do not join its reader;
    // terminating the supervisor ends those readers after the group kill.
    if result.is_ok() {
        finish_pumps(output, errors).await;
    }
    let runtime_keys_observed = cleanup.as_ref().map_or(0, |scope| scope.keys.len() as u32);
    let mut cleanup_failures = if result.is_ok() {
        cleanup.map_or_else(Vec::new, Cleanup::finish)
    } else {
        vec!["process_tree_incomplete".into()]
    };
    if cleanup_failures.len() > 128 {
        cleanup_failures.truncate(128);
    }
    let cleanup_complete = result.is_ok() && cleanup_failures.is_empty();
    let receipt = serde_json::json!({"event":"guardian_stopped","result":crate::GuardianResult {version:1,forced,cleanup_complete,child_exit_code:result.as_ref().ok().and_then(|exit|exit.code()),runtime_keys_observed,cleanup_failures,error_code:result.as_ref().err().and_then(|error|error.raw_os_error()),failure_stage:result.is_err().then(||stage.to_owned())}});
    #[cfg(debug_assertions)]
    if let Some(directory) = std::env::var_os("NEONMIX_GUARDIAN_AUDIT_DIR") {
        // Private, opt-in fixture evidence when SIGKILL removed the IPC reader.
        // No config, arguments or key contents are recorded.
        if let Ok(directory) =
            neonmix_identity::files::PrivateDirectory::open(Path::new(&directory))
        {
            let _ = directory.write_new(
                &format!("guardian-result-{}.json", uuid::Uuid::new_v4()),
                receipt.to_string().as_bytes(),
            );
        }
    }
    emit(receipt);
    match result {
        Ok(exit) => Ok(exit.code().unwrap_or(1)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use neonmix_identity::{files, profiles};
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Fixture {
        fn new() -> Self {
            let project = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let directory = project
                .join(".local/tmp")
                .join(format!("guardian-回收-{}", uuid::Uuid::new_v4()));
            files::private_dir(&directory).unwrap();
            files::private_dir(&directory.join("airplay")).unwrap();
            let profile = profiles::HubProfile {
                version: 2,
                credential_store: profiles::CredentialStore::File,
                room_name: "Fixture".into(),
                state_path: "state.json".into(),
                output: "fixture".into(),
                certificate: "public fixture".into(),
                private_key_ref: uuid::Uuid::new_v4().to_string(),
                admin_token_ref: uuid::Uuid::new_v4().to_string(),
            };
            files::write_new(
                &directory.join("server.json"),
                &serde_json::to_vec(&profile).unwrap(),
            )
            .unwrap();
            files::write_new(&directory.join("state.json"), b"{}").unwrap();
            Self(directory)
        }
        fn key(&self) -> std::path::PathBuf {
            let key = self
                .0
                .join("airplay")
                .join(format!("runtime-key-{}", uuid::Uuid::new_v4()));
            files::write_new(&key, b"synthetic temporary key").unwrap();
            key
        }
        fn capture(&self) -> Cleanup {
            let args = vec![
                "serve".into(),
                "--config".into(),
                self.0.join("server.json").into_os_string(),
            ];
            let mut cleanup = Cleanup::from_args(&args).unwrap();
            cleanup.capture();
            cleanup
        }
    }
    #[tokio::test]
    async fn inherited_log_writer_cannot_hold_final_cleanup_forever() {
        let (reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let output = pump(reader, false, Arc::new(AtomicBool::new(false)), None);
        let errors = std::thread::spawn(|| Ok(()));
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(2), finish_pumps(output, errors))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(writer);
    }
    #[test]
    fn cleanup_keeps_persistent_identity_and_removes_only_captured_temporary_keys() {
        let fixture = Fixture::new();
        let key = fixture.key();
        let persistent = fixture.0.join("airplay/receiver.json");
        files::write_new(&persistent, b"persistent identity fixture").unwrap();
        let cleanup = fixture.capture();
        assert_eq!(cleanup.keys.len(), 1);
        assert!(cleanup.finish().is_empty());
        assert!(!key.exists());
        assert_eq!(
            files::read_private(&persistent, 128).unwrap(),
            b"persistent identity fixture"
        );
        assert!(fixture.0.join("server.json").exists());
    }
    #[test]
    fn another_owner_lock_prevents_cleanup_and_file_replacement_does_not_get_deleted() {
        let fixture = Fixture::new();
        let key = fixture.key();
        let cleanup = fixture.capture();
        let owner = profiles::operation_lock(&fixture.0.join("state.lock")).unwrap();
        assert_eq!(cleanup.finish(), ["owner_lock_busy"]);
        assert!(key.exists());
        drop(owner);
        let cleanup = fixture.capture();
        files::replace(&key, b"new owner's object").unwrap();
        assert_eq!(cleanup.finish(), ["runtime_key_changed"]);
        assert_eq!(
            files::read_private(&key, 128).unwrap(),
            b"new owner's object"
        );
    }
    #[test]
    fn receiver_directory_symlink_fails_closed_without_deleting_other_directory_keys() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let outside_key = outside.key();
        std::os::unix::fs::symlink(
            outside.0.join("airplay"),
            fixture
                .0
                .join("airplay")
                .join(uuid::Uuid::new_v4().to_string()),
        )
        .unwrap();
        let cleanup = fixture.capture();
        assert!(!cleanup.snapshot_valid);
        assert_eq!(cleanup.finish(), ["runtime_key_snapshot_failed"]);
        assert!(outside_key.exists());
    }
}
