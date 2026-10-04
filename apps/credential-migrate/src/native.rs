//! The blocking native API is isolated in a killable helper with private pipes.
use neonmix_credential_migrate::{Error, LegacyReader, Result};
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const LIMIT: usize = 64 * 1024;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    reference: String,
    parent_id: u32,
    #[serde(default)]
    operation: Operation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
}
#[derive(Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    #[default]
    Read,
    CreateFixture,
    DeleteFixture,
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
pub struct NativeReader {
    pub timeout: Duration,
    pub cancelled: Arc<AtomicBool>,
}
impl LegacyReader for NativeReader {
    fn read(&mut self, reference: &str) -> Result<String> {
        let executable = std::env::current_exe().map_err(|_| Error("migration_helper_failed"))?;
        let mut command = Command::new(executable);
        command.arg("--native-read-helper");
        self.read_command(command, reference)
    }
}
impl NativeReader {
    fn read_command(&self, mut command: Command, reference: &str) -> Result<String> {
        let mut child = OwnedChild(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| Error("migration_helper_failed"))?,
        );
        let input = child
            .0
            .stdin
            .take()
            .ok_or(Error("migration_helper_failed"))?;
        let mut input = input;
        let request = serde_json::to_vec(&Request {
            reference: reference.to_owned(),
            parent_id: std::process::id(),
            operation: Operation::Read,
            value: None,
        })
        .map_err(|_| Error("migration_helper_failed"))?;
        input
            .write_all(&request)
            .map_err(|_| Error("migration_helper_failed"))?;
        drop(input);
        let output = child
            .0
            .stdout
            .take()
            .ok_or(Error("migration_helper_failed"))?;
        // Drain concurrently so a valid value larger than the pipe buffer cannot
        // deadlock while the parent is enforcing the helper's deadline.
        let read = thread::spawn(move || {
            let mut bytes = Vec::new();
            output
                .take((LIMIT + 2) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let started = Instant::now();
        let completed = loop {
            if self.cancelled.load(Ordering::Acquire) {
                break Err(Error("migration_cancelled"));
            }
            if started.elapsed() >= self.timeout {
                break Err(Error("migration_native_timeout"));
            }
            match child.0.try_wait() {
                Ok(Some(status)) if status.success() => break Ok(()),
                Ok(Some(_)) => break Err(Error("migration_native_read_failed")),
                Err(_) => break Err(Error("migration_helper_failed")),
                Ok(None) => thread::sleep(Duration::from_millis(10)),
            }
        };
        if completed.is_err() {
            let _ = child.0.kill();
            let _ = child.0.wait();
        }
        let bytes = read
            .join()
            .map_err(|_| Error("migration_helper_failed"))?
            .map_err(|_| Error("migration_helper_failed"))?;
        completed?;
        if bytes == [2] {
            return Err(Error("migration_native_missing"));
        }
        if bytes.len() < 2 || bytes.len() > LIMIT + 1 || bytes[0] != 1 {
            return Err(Error("migration_native_read_failed"));
        }
        String::from_utf8(bytes[1..].to_vec()).map_err(|_| Error("migration_native_read_failed"))
    }
}

pub fn helper(fixture: bool) -> bool {
    let result = (|| -> std::result::Result<(), ()> {
        let mut input = Vec::new();
        std::io::stdin()
            .take((LIMIT + 1025) as u64)
            .read_to_end(&mut input)
            .map_err(|_| ())?;
        if input.len() > LIMIT + 1024 {
            return Err(());
        }
        let request: Request = serde_json::from_slice(&input).map_err(|_| ())?;
        if !fixture && (request.operation != Operation::Read || request.value.is_some()) {
            return Err(());
        }
        #[cfg(unix)]
        {
            // Include the race where the parent dies before this helper starts.
            let parent = request.parent_id;
            if nix::unistd::getppid().as_raw() as u32 != parent {
                return Err(());
            }
            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(50));
                    if nix::unistd::getppid().as_raw() as u32 != parent {
                        std::process::exit(1);
                    }
                }
            });
        }
        let reference = request.reference;
        let id = uuid::Uuid::parse_str(&reference).map_err(|_| ())?;
        if id.to_string() != reference {
            return Err(());
        }
        let entry = keyring::Entry::new("com.neonmix.identity.v1", &reference).map_err(|_| ())?;
        if request.operation != Operation::Read {
            let expected = request.value.ok_or(())?;
            if expected.is_empty() || expected.len() > LIMIT {
                return Err(());
            }
            match request.operation {
                Operation::CreateFixture => {
                    if !matches!(entry.get_password(), Err(keyring::Error::NoEntry)) {
                        return Err(());
                    }
                    if entry.set_password(&expected).is_err() {
                        let _ = entry.delete_credential();
                        return Err(());
                    }
                    if entry.get_password().ok().as_deref() != Some(&expected) {
                        let _ = entry.delete_credential();
                        return Err(());
                    }
                }
                Operation::DeleteFixture => match entry.get_password() {
                    Ok(actual) if actual == expected => {
                        entry.delete_credential().map_err(|_| ())?
                    }
                    Err(keyring::Error::NoEntry) => {}
                    _ => return Err(()),
                },
                Operation::Read => unreachable!(),
            }
            return std::io::stdout().lock().write_all(&[3]).map_err(|_| ());
        }
        let secret = match entry.get_password() {
            Ok(secret) => secret,
            Err(keyring::Error::NoEntry) => {
                return std::io::stdout().lock().write_all(&[2]).map_err(|_| ());
            }
            Err(_) => return Err(()),
        };
        if secret.is_empty() || secret.len() > LIMIT {
            return Err(());
        }
        let mut output = std::io::stdout().lock();
        output
            .write_all(&[1])
            .and_then(|_| output.write_all(secret.as_bytes()))
            .and_then(|_| output.flush())
            .map_err(|_| ())
    })();
    result.is_ok()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn stalled_mock_helper_is_killed_and_reaped_on_timeout_and_cancellation() {
        for cancellation in [false, true] {
            let reader = NativeReader {
                timeout: Duration::from_millis(40),
                cancelled: Arc::new(AtomicBool::new(cancellation)),
            };
            let mut command = Command::new("/bin/sleep");
            command.arg("10");
            let start = Instant::now();
            let error = reader
                .read_command(command, &uuid::Uuid::new_v4().to_string())
                .unwrap_err();
            assert_eq!(
                error.0,
                if cancellation {
                    "migration_cancelled"
                } else {
                    "migration_native_timeout"
                }
            );
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }
    #[test]
    fn large_mock_helper_response_is_drained_without_deadlock() {
        let reader = NativeReader {
            timeout: Duration::from_secs(5),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        let mut command = Command::new("python3");
        command.args([
            "-c",
            "import sys,json; r=json.load(sys.stdin)['reference'].encode(); sys.stdout.buffer.write(bytes([1])+r*1400)",
        ]);
        let value = reader
            .read_command(command, &uuid::Uuid::new_v4().to_string())
            .unwrap();
        assert_eq!(value.len(), 36 * 1400);
    }
}
