//! Bounded, user-private desktop IPC. Audio ownership belongs to the background,
//! never the UI. Commands are a closed typed set, not shell commands.
use neonmix_control::Operation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 262_144;
pub const IPC_TIMEOUT: Duration = Duration::from_secs(45);
pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubSettings {
    pub name: String,
    pub output: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SenderOptions {
    pub credential: PathBuf,
    pub hub: Option<String>,
    /// A persisted binding directory relative to the private state directory.
    pub output_binding: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputAction {
    Add {
        credential: PathBuf,
        hub: Option<String>,
        name: String,
        provider: String,
        device: Option<String>,
    },
    Show,
    Rename {
        expected_revision: u64,
        name: String,
    },
    Enable {
        expected_revision: u64,
    },
    Disable {
        expected_revision: u64,
    },
    Remove {
        expected_revision: u64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status,
    Devices,
    TestTone {
        output: String,
    },
    HubSetup {
        settings: HubSettings,
    },
    HubStart,
    HubStop,
    HubSettings {
        settings: HubSettings,
    },
    Discover {
        seconds: u32,
    },
    Invite {
        credential: PathBuf,
        hub: Option<String>,
        out: PathBuf,
        seconds: u32,
    },
    CancelInvite {
        credential: PathBuf,
        hub: Option<String>,
        invitation_id: Uuid,
    },
    Pair {
        invite: PathBuf,
        credential: PathBuf,
        name: String,
        hub: Option<String>,
    },
    PairText {
        invitation: String,
        name: String,
        hub: Option<String>,
    },
    ForgetCredential {
        credential: PathBuf,
    },
    Output {
        directory: PathBuf,
        action: OutputAction,
    },
    Snapshot {
        credential: PathBuf,
        hub: Option<String>,
    },
    Control {
        credential: PathBuf,
        hub: Option<String>,
        expected_revision: u64,
        operation: Operation,
    },
    Airplay {
        credential: PathBuf,
        hub: Option<String>,
        command: Option<neonmix_airplay_adapter::control::AirplayCommand>,
    },
    AirplayV2 {
        credential: PathBuf,
        hub: Option<String>,
        command: Option<neonmix_airplay_adapter::control::AirplayCommandV2>,
    },
    Diagnostics {
        credential: PathBuf,
        hub: Option<String>,
    },
    ExportDiagnostics {
        credential: PathBuf,
        hub: Option<String>,
    },
    SenderStart {
        options: SenderOptions,
    },
    SenderStop,
    Shutdown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub request: Request,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub data: Value,
    pub error: Option<String>,
}
impl Reply {
    pub fn success(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
            error: None,
        }
    }
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: Value::Null,
            error: Some(error.into()),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub error: Option<String>,
    pub last_event: Option<Value>,
    pub metrics: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileInfo {
    pub credential: PathBuf,
    pub device_id: Option<Uuid>,
    pub hub_id: Option<Uuid>,
    pub name: Option<String>,
    pub role: Option<neonmix_control::Role>,
    pub pending: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub version: u16,
    pub pid: u32,
    pub hub: ProcessStatus,
    pub sender: ProcessStatus,
    pub hub_settings: Option<HubSettings>,
    pub profiles: Vec<ProfileInfo>,
    pub sender_options: Option<SenderOptions>,
    pub output_binding: Option<Value>,
}

#[derive(Clone)]
pub struct Client {
    state_dir: PathBuf,
}
impl Client {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }
    pub fn request(&self, request: &Request) -> Result<Reply> {
        #[cfg(any(unix, windows))]
        {
            transport::request(&self.state_dir, request)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = request;
            Err("desktop IPC is not implemented on this platform".into())
        }
    }
    pub fn ensure_background(&self) -> Result<()> {
        if self.request(&Request::Status).is_ok() {
            return Ok(());
        }
        #[cfg(any(unix, windows))]
        {
            transport::prepare(&self.state_dir)?;
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let background =
            exe.parent()
                .ok_or("missing executable directory")?
                .join(if cfg!(windows) {
                    "neonmix-background.exe"
                } else {
                    "neonmix-background"
                });
        let mut command = std::process::Command::new(background);
        command
            .arg("--state-dir")
            .arg(&self.state_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动后台，请构建 neonmix-background：{e}"))?;
        for _ in 0..100 {
            if self.request(&Request::Status).is_ok() {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(());
            }
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                // A concurrent UI can win the exclusive daemon lock.
                if self.request(&Request::Status).is_ok() {
                    return Ok(());
                }
                return Err("后台未能启动；检查目录权限、socket 路径长度和同目录二进制".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("后台启动超时".into())
    }
}
#[cfg(any(unix, windows))]
pub mod daemon;
#[cfg(target_os = "linux")]
mod linux_owner;
#[cfg(unix)]
pub mod transport;
#[cfg(windows)]
#[path = "transport_windows.rs"]
pub mod transport;
