//! Bounded, user-private desktop IPC. Audio ownership belongs to the background,
//! never the UI. Commands are a closed typed set, not shell commands.
use neonmix_control::Operation;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod guardian;
mod intent;
mod manager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const INTENT_VERSION: u16 = 1;
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
        expected_output_id: Uuid,
        name: String,
    },
    Enable {
        expected_revision: u64,
        expected_output_id: Uuid,
    },
    Disable {
        expected_revision: u64,
        expected_output_id: Uuid,
    },
    Remove {
        expected_revision: u64,
        expected_output_id: Uuid,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    ResolveAirplayPatch {
        credential: PathBuf,
        hub: Option<String>,
        hub_id: Uuid,
        credential_id: Uuid,
        runtime_epoch: Uuid,
        stream_epoch: u64,
        command: neonmix_airplay_adapter::control::AirplayCommandV2,
        guard: Option<Box<neonmix_airplay_adapter::control::AirplayActionV2>>,
    },
    /// Immutable user intent. Identity and command IDs are never assigned
    /// from whatever credential/room happens to be selected at execution.
    Intent {
        credential: PathBuf,
        hub: Option<String>,
        hub_id: Uuid,
        credential_id: Uuid,
        command: IntentCommand,
    },
    /// A UI freezes this instance and stop generation when creating Start.
    LifecycleStart {
        instance_generation: Uuid,
        expected_stop_generation: u64,
        request: Box<Request>,
    },
    LifecycleStop {
        instance_generation: Uuid,
        request: Box<Request>,
    },
    Status,
    LifecycleOperation {
        operation_id: Uuid,
        instance_generation: Uuid,
    },
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
    #[serde(default, deserialize_with = "fault::optional_fault")]
    pub fault: Option<Fault>,
}
impl Reply {
    pub fn success(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
            error: None,
            fault: None,
        }
    }
    pub fn failure(error: impl Into<String>) -> Self {
        let error = error.into();
        let fault =
            Fault::from_error(&error).unwrap_or_else(|| Fault::new(FaultCode::GenericFailure));
        Self {
            ok: false,
            data: Value::Null,
            error: Some(legacy_error(&error)),
            fault: Some(fault),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopResult {
    pub graceful: bool,
    pub forced: bool,
    pub elapsed_ms: u64,
    pub exit_code: Option<i32>,
    pub cleanup_complete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianResult {
    pub version: u16,
    pub forced: bool,
    pub cleanup_complete: bool,
    pub child_exit_code: Option<i32>,
    #[serde(default)]
    pub runtime_keys_observed: u32,
    #[serde(default)]
    pub cleanup_failures: Vec<String>,
    #[serde(default)]
    pub error_code: Option<i32>,
    #[serde(default)]
    pub failure_stage: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessStatus {
    /// Authenticated target latched by this Sender Child, independent of the UI.
    #[serde(default)]
    pub sender_target: Option<SenderTarget>,
    #[serde(default)]
    pub guardian_version: u16,
    #[serde(default)]
    pub guardian_result: Option<GuardianResult>,
    #[serde(default)]
    pub owner_pid: Option<u32>,
    pub running: bool,
    /// Latched business readiness for this Child; ordinary logs cannot clear it.
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub stopping: bool,
    #[serde(default)]
    pub stop_result: Option<StopResult>,
    pub pid: Option<u32>,
    pub error: Option<String>,
    #[serde(default, deserialize_with = "fault::optional_fault")]
    pub fault: Option<Fault>,
    pub last_event: Option<Value>,
    pub metrics: Option<Value>,
    #[serde(default)]
    pub metrics_sequence: u64,
    #[serde(default)]
    pub metrics_age_ms: Option<u64>,
    #[serde(skip)]
    pub metrics_observed_at: Option<std::time::Instant>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderTarget {
    pub hub_id: Uuid,
    pub device_id: Option<Uuid>,
    pub room_name: Option<String>,
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
pub struct LifecycleView {
    pub version: u16,
    pub instance_generation: Uuid,
    pub hub_stop_generation: u64,
    pub sender_stop_generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    #[serde(default)]
    pub lifecycle: Option<LifecycleView>,
    #[serde(default)]
    pub intent_version: u16,
    #[serde(default)]
    pub managed_control_version: u16,
    pub version: u16,
    pub pid: u32,
    pub hub: ProcessStatus,
    pub sender: ProcessStatus,
    pub hub_settings: Option<HubSettings>,
    pub profiles: Vec<ProfileInfo>,
    pub sender_options: Option<SenderOptions>,
    pub output_binding: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "domain",
    content = "command",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum IntentCommand {
    Native(neonmix_control::Command),
    Airplay(neonmix_airplay_adapter::control::AirplayCommandV2),
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
            let reply = transport::request(&self.state_dir, request)?;
            if crate::manager::kind(request).is_none()
                || !reply.ok
                || !matches!(reply.data["state"].as_str(), Some("accepted" | "completed"))
            {
                return Ok(reply);
            }
            let operation_id = serde_json::from_value(reply.data["operation_id"].clone())
                .map_err(|_| "invalid_backend_response")?;
            let instance_generation =
                serde_json::from_value(reply.data["instance_generation"].clone())
                    .map_err(|_| "invalid_backend_response")?;
            let started = std::time::Instant::now();
            let mut observed = reply;
            loop {
                if observed.data["state"] != "completed" {
                    observed = transport::request(
                        &self.state_dir,
                        &Request::LifecycleOperation {
                            operation_id,
                            instance_generation,
                        },
                    )?;
                }
                if !observed.ok {
                    return Ok(observed);
                }
                if observed.data["operation_id"] != serde_json::json!(operation_id)
                    || observed.data["instance_generation"]
                        != serde_json::json!(instance_generation)
                {
                    return Err("invalid_backend_response".into());
                }
                if observed.data["state"] == "completed" {
                    if observed.data["ok"] == true {
                        let mut data = observed.data["data"].clone();
                        data["operation_id"] = serde_json::json!(operation_id);
                        data["instance_generation"] = serde_json::json!(instance_generation);
                        return Ok(Reply::success(data));
                    }
                    return Ok(Reply::failure(
                        serde_json::to_string(&observed.data["fault"])
                            .map_err(|_| "invalid_backend_response")?,
                    ));
                }
                if started.elapsed() >= IPC_TIMEOUT {
                    return Err("background_timeout".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
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
            .map_err(|_| "runtime_unavailable".to_string())?;
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
                return Err("runtime_unavailable".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("background_timeout".into())
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

mod managed;

#[cfg(windows)]
pub mod maintenance;

mod fault;
pub use fault::{Fault, FaultCode, FaultParams};

impl ProcessStatus {
    pub(crate) fn observed_snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.metrics_age_ms = self
            .metrics_observed_at
            .map(|at| at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        snapshot
    }

    pub(crate) fn set_failure(&mut self, error: &str) {
        self.ready = false;
        self.fault =
            Some(Fault::from_error(error).unwrap_or_else(|| Fault::new(FaultCode::GenericFailure)));
        self.error = Some(legacy_error(error));
    }
}

// Preserve the old desktop's migration guidance while stable codes travel in
// fault. Existing public control codes stay machine codes in legacy error.
fn legacy_error(error: &str) -> String {
    let fault = Fault::from_error(error);
    let port = fault
        .as_ref()
        .and_then(|fault| fault.params.port)
        .unwrap_or(7443);
    match fault.map(|fault| fault.code) {
        Some(FaultCode::GenericFailure) => "操作失败。请检查诊断后重试。".into(),
        Some(FaultCode::HubPortInUse) => {
            format!("Hub 端口 {port} 已被占用。请关闭占用该端口的程序后重试。")
        }
        Some(FaultCode::MigrationRequired) => {
            "旧配对资料需要离线迁移。请运行 neonmix-credential-migrate。".into()
        }
        Some(FaultCode::CredentialMissing) => "凭证文件缺失。请恢复完整资料目录或重新配对。".into(),
        Some(FaultCode::CredentialPermissionDenied) => {
            "无法访问凭证文件。请检查资料目录的权限。".into()
        }
        Some(FaultCode::CredentialStoreBusy) => "凭证正在使用。请稍后重试。".into(),
        Some(FaultCode::AdministratorCredentialProtected) => {
            "Hub 管理凭证受保护，不能从配对入口删除。".into()
        }
        Some(FaultCode::CredentialIoFailed) => "无法读写凭证文件。请检查目录和磁盘。".into(),
        Some(FaultCode::CredentialCorrupt) => "配对资料损坏。请恢复完整备份或重新配对。".into(),
        Some(FaultCode::CredentialVersionUnsupported) => {
            "配对资料版本不受支持。请检查 NeonMix 版本和迁移说明。".into()
        }
        Some(FaultCode::CredentialKindMismatch) => {
            "凭证类型与操作不符。请选择正确的配对资料。".into()
        }
        Some(FaultCode::CredentialReferenceInvalid) => "凭证引用无效。请恢复完整资料目录。".into(),
        Some(FaultCode::CredentialAlreadyExists) => {
            "凭证已经存在。请使用已有资料或选择新的目录。".into()
        }
        Some(FaultCode::CredentialProfileMissing) => {
            "配对资料缺失。请恢复完整资料目录或重新配对。".into()
        }
        Some(FaultCode::SetupIncomplete) => {
            "Hub 资料不完整。请恢复完整目录，不要覆盖已有身份。".into()
        }
        Some(FaultCode::UnsupportedAudioFormat) => {
            "所选音频设备格式不受支持。请检查采样率、声道和缓冲设置。".into()
        }
        Some(FaultCode::CaptureUnavailable) => "音频采集不可用。请检查虚拟输出和系统权限。".into(),
        Some(FaultCode::ConnectionUnavailable) => "连接不可用。请检查 Hub 地址和网络诊断。".into(),
        Some(FaultCode::BackgroundTimeout) => "后台请求超时。请检查后台状态后重试。".into(),
        Some(FaultCode::BackgroundBusy) => "后台忙碌。请稍后重试。".into(),
        Some(FaultCode::RequestInterrupted) => "查询已被停止操作中断。需要时请重新查询。".into(),
        Some(FaultCode::BackgroundShuttingDown) => "后台正在退出。请等待退出完成。".into(),
        Some(FaultCode::RuntimeUnavailable) => {
            "运行时程序不可用。请检查同目录安装文件或完成构建。".into()
        }
        Some(FaultCode::ProcessAlreadyRunning) => {
            "音频进程仍在运行或停止中。请等待停止完成。".into()
        }
        Some(FaultCode::ProcessExited) => "音频进程已退出。请检查诊断并手动重新启动。".into(),
        Some(FaultCode::StopFailed) => "无法确认音频进程已停止。请检查诊断后重试停止。".into(),
        Some(FaultCode::StopIncomplete) => "音频进程尚未完全停止。请稍后重试停止。".into(),
        Some(FaultCode::RuntimeCleanupIncomplete) => {
            "音频已停止，但运行密钥清理未完成。请检查目录权限后重试停止。".into()
        }
        Some(FaultCode::LifecycleOwnerLost) => {
            "音频生命周期任务不可用。请检查后台状态和诊断。".into()
        }
        Some(FaultCode::HubSetupExists) => {
            "已有 Hub 资料。请修改现有设置，保留已建立的身份。".into()
        }
        Some(FaultCode::HubSettingsRequireStop) => "修改 Hub 设置前请停止播放。".into(),
        Some(FaultCode::SenderRequiresHubStop) => "本实例正在播放。开始发送前请停止 Hub。".into(),
        Some(FaultCode::HubRequiresSenderStop) => {
            "本实例正在发送。配置或启动 Hub 前请停止发送。".into()
        }
        Some(FaultCode::SelfConnectionForbidden) => {
            "不能连接本实例的 Hub。请使用独立的 Sender 主机。".into()
        }
        Some(FaultCode::OutputBindingRequired) => "尚未添加输出绑定。请先添加输出绑定。".into(),
        Some(FaultCode::InvalidOutputBinding) => "输出绑定无效。请检查绑定或重新添加。".into(),
        Some(FaultCode::DiscoveryIncomplete) => "未取得完整发现结果。请检查网络后重新发现。".into(),
        Some(FaultCode::InvalidBackendResponse) => {
            "后台返回了无效结果。请检查版本和诊断后重试。".into()
        }
        Some(FaultCode::IpcInvalidRequest) => "后台请求无效。请检查桌面与后台版本。".into(),
        Some(FaultCode::IpcIncompatibleVersion) => {
            "桌面与后台协议版本不兼容。请使用同一完整安装包。".into()
        }
        Some(FaultCode::IpcMessageTooLarge) => {
            "操作结果超过通信上限。请减少操作范围或检查诊断。".into()
        }
        Some(FaultCode::InvalidPath) => "资料路径无效。请使用后台资料目录内的相对路径。".into(),
        Some(FaultCode::DiagnosticsSaveFailed) => {
            "无法保存诊断。请检查资料目录权限和可用磁盘空间。".into()
        }
        Some(FaultCode::NetworkApiFailed) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkIdentityUnavailable) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkIdentityInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkTokenUnavailable) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkModuleUnavailable) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkPackagePathInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkHashUnavailable) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkPackageReadFailed) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRecordInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRecordWriteFailed) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkInstanceInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkInstallationInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkPackageHashMismatch) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRequesterUnavailable) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRequesterInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRecordMissing) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRuleOwnerMismatch) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRuleReadbackFailed) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkRuleRemoveFailed) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkOperationRequired) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkOperationInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkArgumentInvalid) => {
            "网络配置失败。请检查安装文件和网络诊断，然后重新运行网络修复。".into()
        }
        Some(FaultCode::NetworkUacCancelled) => {
            "网络配置授权已取消。需要配置时请重试并批准系统授权。".into()
        }
        Some(code) => code.as_str().into(),
        None => "后台操作失败；请检查运行时和配置".into(),
    }
}

fn client_io_error(error: std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "background_timeout",
        ErrorKind::PermissionDenied => "permission_denied",
        _ => "connection_unavailable",
    }
    .into()
}
