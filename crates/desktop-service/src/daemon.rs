use crate::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{
    collections::VecDeque,
    path::Component,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, Notify, Semaphore},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);
const LOG_LINE_LIMIT: usize = 16_384;
/// Up to 10 s for `hub serve` to bind or fail (first launch scans GStreamer).
const HUB_START_POLLS: u32 = 200;

fn discovery_result(value: Value) -> Result<Value> {
    let completion = match value {
        Value::Array(events) => events
            .into_iter()
            .rev()
            .find(|event| event["event"] == "discovery_complete"),
        event if event["event"] == "discovery_complete" => Some(event),
        _ => None,
    };
    completion
        .filter(|event| event["candidates"].is_array())
        .ok_or_else(|| "发现结果缺少完成事件或房间列表".into())
}

fn audio_command(executable: &Path, args: Vec<String>) -> Command {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}
#[derive(Default)]
struct ReadCancellation {
    epoch: AtomicU64,
    notify: Notify,
}
struct Managed {
    child: Option<Child>,
    status: Arc<StdMutex<ProcessStatus>>,
}
impl Default for Managed {
    fn default() -> Self {
        Self {
            child: None,
            status: Arc::new(StdMutex::new(ProcessStatus::default())),
        }
    }
}
impl Managed {
    fn status(&self) -> ProcessStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    fn reap(&mut self) {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(exit)) => {
                    let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
                    status.running = false;
                    status.pid = None;
                    if !exit.success() && status.error.is_none() {
                        status.error = Some("音频进程意外退出；请检查诊断并手动重新启动".into());
                    }
                    self.child = None;
                }
                Err(_) => {
                    let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
                    status.error = Some("无法读取音频进程状态".into());
                }
                Ok(None) => {}
            }
        }
    }
    fn start(&mut self, executable: &Path, args: Vec<String>) -> Result<()> {
        self.reap();
        if self.child.is_some() {
            return Err("进程已经运行".into());
        }
        let mut child = audio_command(executable, args)
            .spawn()
            .map_err(|_| "无法启动同目录音频程序；请先完成构建".to_string())?;
        self.status = Arc::new(StdMutex::new(ProcessStatus {
            running: true,
            pid: child.id(),
            ..ProcessStatus::default()
        }));
        let output = child.stdout.take().ok_or("missing child stdout")?;
        let errors = child.stderr.take().ok_or("missing child stderr")?;
        tokio::spawn(drain(output, self.status.clone(), false));
        tokio::spawn(drain(errors, self.status.clone(), true));
        self.child = Some(child);
        Ok(())
    }
    async fn stop(&mut self) -> Result<()> {
        self.reap();
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                terminate(pid)?;
            }
            #[cfg(windows)]
            child.start_kill().map_err(|e| e.to_string())?;
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                child.kill().await.map_err(|e| e.to_string())?;
                child.wait().await.map_err(|e| e.to_string())?;
            }
        }
        let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
        status.running = false;
        status.pid = None;
        Ok(())
    }
}
#[allow(unsafe_code)]
#[cfg(unix)]
fn terminate(pid: u32) -> Result<()> {
    // SAFETY: pid originates from our live owned Child. No external PID is accepted.
    if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error.to_string());
        }
    }
    Ok(())
}
fn classify_error(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("credential_")
        || m.contains("migration_required")
        || m.contains("administrator_credential_protected")
    {
        return profile_error(message);
    }
    if m.contains("setup_incomplete") {
        return "Hub 资料不完整，请恢复完整目录；不能重新初始化覆盖".into();
    }
    // EADDRINUSE on macOS/Linux, WSAEADDRINUSE on Windows (localized text).
    if m.contains("address already in use")
        || m.contains("os error 48)")
        || m.contains("os error 98)")
        || m.contains("os error 10048)")
    {
        return "Hub 端口 7443 已被占用；请先退出其他 NeonMix Hub 或占用该端口的程序".into();
    }
    if m.contains("revok")
        || m.contains("unauthoriz")
        || m.contains("permission")
        || m.contains("rejected")
    {
        "权限或配对凭证不可用；检查设备授权"
    } else if m.contains("capture") {
        "采集故障；检查虚拟输出和系统权限"
    } else if m.contains("output") || m.contains("device") {
        "输出设备故障；检查所选设备和输出绑定"
    } else if m.contains("connect")
        || m.contains("offline")
        || m.contains("network")
        || m.contains("timeout")
    {
        "连接不可用；检查 Hub 地址、发现和网络诊断"
    } else {
        "后台操作失败；请检查运行时和配置"
    }
    .into()
}
async fn drain(reader: impl AsyncRead + Unpin, status: Arc<StdMutex<ProcessStatus>>, stderr: bool) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(512);
    let mut oversized = false;
    while let Ok(byte) = reader.read_u8().await {
        if byte != b'\n' {
            if line.len() < LOG_LINE_LIMIT {
                line.push(byte)
            } else {
                oversized = true;
            }
            continue;
        }
        if !oversized && !line.is_empty() {
            let mut current = status.lock().unwrap_or_else(|p| p.into_inner());
            if stderr {
                current.error = Some(classify_error(&String::from_utf8_lossy(&line)));
            } else if let Ok(mut event) = serde_json::from_slice::<Value>(&line) {
                redact(&mut event, false);
                if event
                    .get("event")
                    .and_then(Value::as_str)
                    .is_some_and(|e| e.ends_with("stats"))
                {
                    current.metrics = Some(event.clone());
                }
                current.last_event = Some(event);
            }
        }
        line.clear();
        oversized = false;
    }
}
/// Export diagnostics never contains names, device/host identifiers, routes,
/// credential refs, certificates or arbitrary error text. Functional snapshots
/// retain their identifiers so revision-checked controls remain possible.
pub fn redact(value: &mut Value, diagnostic: bool) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_lowercase();
                let numeric_phase_diagnostic = key == "timed_phase_error_ns"
                    && value.as_array().is_some_and(|items| {
                        items.len() == 16 && items.iter().all(Value::is_number)
                    });
                if key.contains("token")
                    || key.contains("secret")
                    || key.contains("certificate")
                    || key.contains("private_key")
                    || key.contains("pairing_pin")
                    || key.contains("client_public_key")
                    || matches!(
                        key.as_str(),
                        "public_key" | "known_keys" | "blocked_keys" | "key_reference"
                    )
                    || key == "pem"
                    || key == "credential"
                    || key == "file"
                    || key == "directory"
                    || key == "reason"
                    || (key.contains("error")
                        && !value.is_number()
                        && !value.is_null()
                        && !value.is_boolean()
                        && !numeric_phase_diagnostic)
                    || (diagnostic
                        && (key == "alias"
                            || key.contains("name")
                            || (key.ends_with("id") && key != "stream_id")
                            || key.contains("address")
                            || key.contains("endpoint")
                            || key == "listen"
                            || (key == "hub" && value.is_string())
                            || key == "output_binding"
                            || key == "path"))
                {
                    *value = Value::String("[redacted]".into());
                } else {
                    redact(value, diagnostic);
                }
            }
        }
        Value::Array(array) => {
            for value in array {
                redact(value, diagnostic)
            }
        }
        _ => {}
    }
}

fn redact_control_reply(event: &mut Value, preserve_airplay_pin: bool) {
    let pin = event
        .get("pairing_pin")
        .cloned()
        .filter(|_| preserve_airplay_pin);
    let receiver_pins: Vec<_> = if preserve_airplay_pin {
        event
            .get("receivers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| Some((r.get("receiver_id")?.clone(), r.get("pairing_pin")?.clone())))
            .collect()
    } else {
        Vec::new()
    };
    redact(event, false);
    if let Some(receivers) = event.get_mut("receivers").and_then(Value::as_array_mut) {
        for receiver in receivers {
            if let Some((_, pin)) = receiver_pins
                .iter()
                .find(|(id, _)| receiver.get("receiver_id") == Some(id))
            {
                receiver["pairing_pin"] = pin.clone();
            }
        }
    }
    // Only the authenticated AirPlay control response carries an administrator PIN.
    // It never enters process logs, persisted UI state or diagnostic history.
    if let Some(pin) = pin
        && let Some(object) = event.as_object_mut()
    {
        object.insert("pairing_pin".into(), pin);
    }
}

/// Only numeric/boolean telemetry under known structural keys is exported.
/// Unknown keys, arbitrary strings, names, routes and protocol credentials are
/// excluded even if future runtimes add them to diagnostics.
pub fn export_whitelist(value: &Value) -> Value {
    fn project(value: &Value) -> Option<Value> {
        match value {
            Value::Number(_) | Value::Bool(_) | Value::Null => Some(value.clone()),
            Value::Array(array) => Some(Value::Array(
                array
                    .iter()
                    .map(|value| project(value).unwrap_or(Value::Null))
                    .collect(),
            )),
            Value::Object(map) => {
                const KEYS: &[&str] = &[
                    "airplay",
                    "sessions",
                    "capacity",
                    "limit",
                    "active",
                    "reserved",
                    "media_resets",
                    "ingress",
                    "latency_advance_ns",
                    "protocol_lead_ns",
                    "playout_lead_ns",
                    "output_latency_ns",
                    "late_packets",
                    "rejected_blocks",
                    "received_blocks",
                    "released_blocks",
                    "pending_frames",
                    "pending_bytes",
                    "available",
                    "command_fault",
                    "operation_failed",
                    "telemetry",
                    "stats",
                    "clock",
                    "info",
                    "internal_frames",
                    "native_internal_frames",
                    "clock_timestamp_ns",
                    "device_timestamp_ns",
                    "actual_period_frames",
                    "requested_period_frames",
                    "resampler_delay_frames",
                    "stream_epoch",
                    "format",
                    "reset",
                    "epoch",
                    "epoch_exhausted",
                    "callback_frames",
                    "remote",
                    "local",
                    "hub",
                    "sender",
                    "running",
                    "metrics",
                    "meters",
                    "window_frames",
                    "sample_rate",
                    "lanes",
                    "stream_id",
                    "peak",
                    "rms",
                    "output",
                    "limiter_gain",
                    "receivers",
                    "media_workers",
                    "lane_stream_ids",
                    "packet_budget_drops",
                    "pcm_queue_age_max_ns",
                    "fifo_frames",
                    "sinc_delay_frames",
                    "output_stats",
                    "output_frames",
                    "limited_frames",
                    "underrun_frames",
                    "last_underrun_output_frame",
                    "last_underrun_fifo_frames",
                    "last_underrun_needed_frames",
                    "last_underrun_source_end",
                    "underrun_frames_by_lane",
                    "filtered_queues",
                    "queues",
                    "drift_ppm",
                    "scheduling",
                    "pump_timing",
                    "receive_throttles",
                    "max_loop_gap_ns",
                    "max_pump_ns",
                    "max_report_ns",
                    "callbacks",
                    "frames",
                    "silent_frames",
                    "stale_frames",
                    "dropped_frames",
                    "errors",
                    "last_error",
                    "period_min",
                    "period_max",
                    "callback_max_ns",
                    "callback_over_budget",
                    "discontinuities",
                    "clock_epoch",
                    "last_arrival_ns",
                    "last_device_ns",
                    "native_device_frame_position",
                    "native_internal_frames",
                    "native_stream_frames",
                    "no_data_intervals",
                    "presented_frames",
                    "submitted_frames",
                    "sent_packets",
                    "sent_bytes",
                    "queue_drops",
                    "max_pending",
                    "generated_audio_blocks",
                    "skipped_source_periods",
                    "max_source_backlog_ns",
                    "encoded_media_packets",
                    "audio_queue_dropped",
                    "audio_queue_buffers",
                    "encoder_bitrate",
                    "encoder_dtx",
                    "feedback_reports",
                    "capture_stats",
                    "control_connected",
                    "control_snapshots",
                    "control_subscriptions",
                    "control_events",
                    "control_revision",
                    "diagnostic_drops",
                ];
                let projected = map
                    .iter()
                    .filter(|(key, _)| KEYS.contains(&key.as_str()))
                    .filter_map(|(key, value)| project(value).map(|value| (key.clone(), value)))
                    .collect();
                Some(Value::Object(projected))
            }
            _ => None,
        }
    }
    project(value).unwrap_or_else(|| json!({}))
}
struct Runtime {
    directory: PathBuf,
    hub_bin: PathBuf,
    audio_bin: PathBuf,
    hub: Managed,
    sender: Managed,
    #[cfg(target_os = "linux")]
    virtual_output_owner: Managed,
    sender_options: Option<SenderOptions>,
    history: VecDeque<Value>,
    viewer: Option<Value>,
    shutting_down: bool,
    command_fault: Arc<StdMutex<Option<Value>>>,
}
impl Runtime {
    #[cfg(target_os = "linux")]
    async fn ensure_virtual_output_owner(&mut self, binding_directory: &Path) -> Result<()> {
        self.virtual_output_owner.reap();
        if self.virtual_output_owner.child.is_none()
            && !crate::linux_owner::current_user_owner_exists()?
        {
            let directory = self.path(Path::new("virtual-output-owner"))?;
            crate::transport::prepare(&directory)?;
            self.virtual_output_owner.start(
                &self.audio_bin,
                vec![
                    "virtual-output".into(),
                    "--state-directory".into(),
                    directory.to_string_lossy().into_owned(),
                    "--output-binding".into(),
                    binding_directory.to_string_lossy().into_owned(),
                ],
            )?;
        }
        let ready = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                self.virtual_output_owner.reap();
                if self
                    .hub_command(vec![
                        "virtual-output".into(),
                        "--provider".into(),
                        "neonmix".into(),
                    ])
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
                if self.virtual_output_owner.child.is_none()
                    && !crate::linux_owner::current_user_owner_exists()?
                {
                    return Err("Linux 虚拟输出 owner 已退出；检查 PipeWire 用户会话后重试".into());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        ready.map_err(|_| "Linux 虚拟输出尚未就绪；检查 PipeWire 用户会话后重试".to_string())?
    }
    fn path(&self, relative: &Path) -> Result<PathBuf> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("路径必须位于后台 state_dir 内，使用相对路径".into());
        }
        let path = self.directory.join(relative);
        for parent in path.ancestors().take_while(|p| *p != self.directory) {
            if parent.exists() || std::fs::symlink_metadata(parent).is_ok() {
                crate::transport::check_owned(parent)?;
            }
        }
        Ok(path)
    }
    fn parent(&self, path: &Path) -> Result<()> {
        let parent = path.parent().ok_or("missing parent")?;
        #[cfg(windows)]
        crate::transport::prepare(parent)?;
        #[cfg(unix)]
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        for directory in parent.ancestors().take_while(|p| *p != self.directory) {
            crate::transport::protect_directory(directory)?;
        }
        Ok(())
    }
    fn credential(&self, path: &Path) -> Result<String> {
        let path = self.path(path)?;
        neonmix_identity::profiles::token(&path).map_err(profile_error)?;
        Ok(path.to_string_lossy().into_owned())
    }
    fn settings(&self) -> Option<HubSettings> {
        let profile =
            neonmix_identity::profiles::hub(&self.directory.join("hub/server.json")).ok()?;
        Some(HubSettings {
            name: profile.room_name,
            output: profile.output,
        })
    }
    fn profiles(&self) -> Vec<ProfileInfo> {
        let mut paths = Vec::new();
        if self.directory.join("hub/admin.json").is_file() {
            paths.push(self.directory.join("hub/admin.json"));
        }
        if let Ok(entries) = std::fs::read_dir(self.directory.join("profiles")) {
            paths.extend(
                entries
                    .take(64)
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.extension().is_some_and(|e| e == "json")),
            );
        }
        paths
            .into_iter()
            .filter_map(|path| {
                let relative = path.strip_prefix(&self.directory).ok()?;
                self.path(relative).ok()?;
                let profile = neonmix_identity::profiles::token(&path).ok()?;
                let role = if relative == Path::new("hub/admin.json")
                    && profile.profile_kind == neonmix_identity::profiles::ProfileKind::Admin
                {
                    Some(neonmix_control::Role::Admin)
                } else {
                    self.viewer
                        .as_ref()
                        .filter(|v| {
                            v.get("device_id")
                                .and_then(Value::as_str)
                                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                                == profile.device_id
                        })
                        .and_then(|v| v.get("role"))
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                };
                Some(ProfileInfo {
                    credential: relative.into(),
                    device_id: profile.device_id,
                    hub_id: Some(profile.hub_id),
                    name: Some(profile.name),
                    role,
                    pending: profile.pending,
                })
            })
            .collect()
    }
    fn status(&mut self) -> ServiceStatus {
        self.hub.reap();
        self.sender.reap();
        #[cfg(target_os = "linux")]
        self.virtual_output_owner.reap();
        ServiceStatus {
            version: PROTOCOL_VERSION,
            pid: std::process::id(),
            hub: self.hub.status(),
            sender: self.sender.status(),
            hub_settings: self.settings(),
            profiles: self.profiles(),
            sender_options: self.sender_options.clone(),
            output_binding: ["output/binding.json", "outputs/main/binding.json"]
                .iter()
                .find_map(|relative| {
                    self.path(Path::new(relative))
                        .ok()
                        .and_then(|path| std::fs::read(path).ok())
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                }),
        }
    }
    async fn run(&self, executable: &Path, args: Vec<String>) -> Result<Value> {
        let preserve_airplay_pin = args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "airplay" | "airplay-v2"));
        let mut child = audio_command(executable, args)
            .spawn()
            .map_err(|_| "无法启动同目录工具；请先完成构建".to_string())?;
        let stdout = child.stdout.take().ok_or("missing stdout")?;
        let stderr = child.stderr.take().ok_or("missing stderr")?;
        let output = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_MESSAGE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        });
        let errors = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr
                .take((LOG_LINE_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        });
        let exit = match tokio::time::timeout(COMMAND_TIMEOUT, child.wait()).await {
            Ok(result) => result.map_err(|_| "无法读取操作结果".to_string())?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                output.abort();
                errors.abort();
                return Err("后台操作超时，已终止该请求".into());
            }
        };
        let bytes = output
            .await
            .map_err(|_| "output task failed")?
            .map_err(|_| "output read failed")?;
        let errors = errors
            .await
            .map_err(|_| "error task failed")?
            .map_err(|_| "error read failed")?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err("操作输出超过 IPC 上限".into());
        }
        if !exit.success() {
            let telemetry = bytes
                .split(|b| *b == b'\n')
                .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
                .find(|value| {
                    value
                        .get("event")
                        .and_then(Value::as_str)
                        .is_some_and(|event| {
                            matches!(
                                event,
                                "output_fault"
                                    | "capture_fault"
                                    | "output_complete"
                                    | "sender_failure"
                            )
                        })
                });
            if let Some(telemetry) = telemetry {
                let safe = export_whitelist(&telemetry);
                *self.command_fault.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(json!({"operation_failed":true,"telemetry":safe}));
            }
            // Structured control errors have a finite vocabulary; retain conflicts.
            if let Some(error) = bytes
                .split(|b| *b == b'\n')
                .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
                .find_map(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
                && [
                    "revision_conflict",
                    "permission_denied",
                    "unauthenticated",
                    "invalid_argument",
                    "not_found",
                    "quota_exceeded",
                    "receiver_busy",
                    "room_capacity_full",
                    "source_already_active",
                    "source_blocked",
                    "pairing_revoked",
                    "stale_revision",
                    "session_changed",
                    "output_unavailable",
                    "worker_unavailable",
                    "upgrade_required",
                ]
                .contains(&error.as_str())
            {
                return Err(error);
            }
            return Err(classify_error(&String::from_utf8_lossy(&errors)));
        }
        let mut events: Vec<Value> = bytes
            .split(|b| *b == b'\n')
            .filter(|b| !b.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(|_| "操作返回无效 JSON".to_string()))
            .collect::<Result<_>>()?;
        for event in &mut events {
            redact_control_reply(event, preserve_airplay_pin);
        }
        Ok(if events.len() == 1 {
            events.remove(0)
        } else {
            Value::Array(events)
        })
    }
    async fn hub_command(&self, args: Vec<String>) -> Result<Value> {
        self.run(&self.hub_bin, args).await
    }
    fn endpoint(args: &mut Vec<String>, hub: Option<String>) -> Result<()> {
        if let Some(hub) = hub {
            if !hub.starts_with("https://") || hub.len() > 512 || hub.chars().any(char::is_control)
            {
                return Err("Hub 必须为 HTTPS 地址".into());
            }
            args.extend(["--hub".into(), hub]);
        }
        Ok(())
    }
    fn validate_settings(settings: &HubSettings) -> Result<()> {
        if settings.name.trim().is_empty()
            || settings.name.len() > 128
            || settings.name.chars().any(char::is_control)
            || settings.output.is_empty()
            || settings.output.len() > 512
            || settings.output.chars().any(char::is_control)
        {
            return Err("房间名称或输出设备无效".into());
        }
        Ok(())
    }
    async fn execute(&mut self, request: Request) -> Result<Value> {
        if self.shutting_down && !matches!(&request, Request::Status | Request::Shutdown) {
            return Err("后台正在退出".into());
        }
        self.hub.reap();
        self.sender.reap();
        let diagnostics_request = matches!(&request, Request::Diagnostics { .. });
        match request {
            Request::Status => Ok(serde_json::to_value(self.status()).map_err(|e| e.to_string())?),
            Request::Devices => self.run(&self.audio_bin, vec!["devices".into()]).await,
            Request::TestTone { output } => {
                if output.is_empty() || output.len() > 512 {
                    return Err("请选择实体输出设备".into());
                }
                self.run(
                    &self.audio_bin,
                    vec![
                        "play".into(),
                        "--device".into(),
                        output,
                        "--seconds".into(),
                        "2".into(),
                        "--gain-db".into(),
                        "-36".into(),
                    ],
                )
                .await
            }
            Request::HubSetup { settings } => {
                Self::validate_settings(&settings)?;
                if self.sender.child.is_some() {
                    return Err("发送运行中，无法配置同机 Hub".into());
                }
                let directory = self.path(Path::new("hub"))?;
                let profile = directory.join("server.json");
                if std::fs::symlink_metadata(&profile).is_ok() {
                    // An old profile must show migration guidance even when the UI
                    // cannot populate its settings from the current schema.
                    neonmix_identity::profiles::hub(&profile).map_err(profile_error)?;
                    return Err("已有 Hub 资料，请修改设置；不能再次初始化".into());
                }
                self.parent(&profile)?;
                self.hub_command(vec![
                    "setup".into(),
                    "--directory".into(),
                    directory.to_string_lossy().into_owned(),
                    "--output".into(),
                    settings.output,
                    "--name".into(),
                    settings.name,
                ])
                .await
            }
            Request::HubStart => {
                if self.sender.child.is_some() {
                    return Err("本实例正在发送；请先停止发送".into());
                }
                let profile = self.path(Path::new("hub/server.json"))?;
                neonmix_identity::profiles::hub(&profile).map_err(profile_error)?;
                self.hub.start(
                    &self.hub_bin,
                    vec![
                        "serve".into(),
                        "--config".into(),
                        profile.to_string_lossy().into_owned(),
                    ],
                )?;
                // Report a startup failure (busy port, missing runtime) as the
                // result of this request rather than as a later status change.
                for _ in 0..HUB_START_POLLS {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    self.hub.reap();
                    if !self.hub.status().running {
                        // Let the stderr reader classify the final line.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        return Err(self
                            .hub
                            .status()
                            .error
                            .unwrap_or_else(|| "Hub 启动后立即退出；请检查运行时和配置".into()));
                    }
                    if self
                        .hub
                        .status()
                        .last_event
                        .is_some_and(|e| e["event"] == "hub_started")
                    {
                        break;
                    }
                }
                Ok(serde_json::to_value(self.status()).map_err(|e| e.to_string())?)
            }
            Request::HubStop => {
                self.hub.stop().await?;
                Ok(json!({"event":"hub_stopped"}))
            }
            Request::HubSettings { settings } => {
                Self::validate_settings(&settings)?;
                if self.hub.child.is_some() {
                    return Err("修改 Hub 设置前请停止播放".into());
                }
                let config = self.path(Path::new("hub/server.json"))?;
                let _profile_lock = neonmix_identity::profiles::operation_lock(
                    &neonmix_identity::profiles::profile_lock_path(&config),
                )
                .map_err(profile_error)?;
                let mut profile =
                    neonmix_identity::profiles::hub(&config).map_err(profile_error)?;
                let state = neonmix_identity::profiles::state_path(&config, &profile)
                    .map_err(profile_error)?;
                let _state_lock =
                    neonmix_identity::profiles::operation_lock(&state.with_extension("lock"))
                        .map_err(profile_error)?;
                let original = neonmix_identity::files::read_private(
                    &state,
                    neonmix_identity::profiles::MAX_STATE_BYTES,
                )
                .map_err(|_| "无法读取 Hub 私有状态")?;
                let mut persistent: neonmix_control::PersistentState =
                    serde_json::from_slice(&original).map_err(|_| "Hub 状态损坏")?;
                neonmix_control::Authority::restore(
                    serde_json::from_slice(&original).map_err(|_| "Hub 状态损坏")?,
                )
                .map_err(|_| "Hub 状态损坏")?;
                persistent.output.id = settings.output.clone();
                persistent.revision = persistent
                    .revision
                    .checked_add(1)
                    .ok_or("Hub revision 已到上限")?;
                profile.output = settings.output.clone();
                profile.room_name = settings.name.clone();
                neonmix_identity::files::replace(
                    &state,
                    &serde_json::to_vec_pretty(&persistent).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                if let Err(error) = neonmix_identity::files::replace(
                    &config,
                    &serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?,
                ) {
                    let _ = neonmix_identity::files::replace(&state, &original);
                    return Err(error.to_string());
                }
                Ok(json!(settings))
            }
            Request::Discover { seconds } => {
                if !(1..=10).contains(&seconds) {
                    return Err("发现窗口必须为 1..10 秒".into());
                }
                let events = self
                    .hub_command(vec![
                        "discover".into(),
                        "--seconds".into(),
                        seconds.to_string(),
                    ])
                    .await?;
                discovery_result(events)
            }
            Request::Invite {
                credential,
                hub,
                out,
                seconds,
            } => {
                if !(30..=300).contains(&seconds) {
                    return Err("邀请有效期必须为 30..300 秒".into());
                }
                let credential = self.credential(&credential)?;
                let out = self.path(&out)?;
                if out.exists() {
                    return Err("邀请输出已经存在，请选择新路径".into());
                }
                self.parent(&out)?;
                let _temporary = TemporaryGuard(out.clone());
                let mut args = vec![
                    "invite".into(),
                    "--credential".into(),
                    credential,
                    "--out".into(),
                    out.to_string_lossy().into_owned(),
                    "--seconds".into(),
                    seconds.to_string(),
                ];
                Self::endpoint(&mut args, hub)?;
                let mut response = self.hub_command(args).await?;
                response["invitation"] =
                    json!(std::fs::read_to_string(out).map_err(|_| "无法读取刚创建的邀请")?);
                Ok(response)
            }
            Request::CancelInvite {
                credential,
                hub,
                invitation_id,
            } => {
                let mut args = vec![
                    "cancel-invite".into(),
                    "--credential".into(),
                    self.credential(&credential)?,
                    "--invitation-id".into(),
                    invitation_id.to_string(),
                ];
                Self::endpoint(&mut args, hub)?;
                self.hub_command(args).await
            }
            Request::Pair {
                invite,
                credential,
                name,
                hub,
            } => {
                if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control)
                {
                    return Err("设备名称无效".into());
                }
                let invite = self.path(&invite)?;
                let credential = self.path(&credential)?;
                self.parent(&credential)?;
                let invitation: Value = serde_json::from_slice(
                    &std::fs::read(&invite).map_err(|_| "无法读取邀请文件")?,
                )
                .map_err(|_| "邀请文件无效")?;
                if let Ok(local) = std::fs::read(self.directory.join("hub/admin.json")) {
                    let local: Value =
                        serde_json::from_slice(&local).map_err(|_| "本地 Hub 身份无效")?;
                    if local.get("hub_id") == invitation.get("hub_id") {
                        return Err("首版拒绝与本实例 Hub 自连接；请使用独立 Sender 主机".into());
                    }
                }
                let mut args = vec![
                    "pair".into(),
                    "--invite".into(),
                    invite.to_string_lossy().into_owned(),
                    "--credential".into(),
                    credential.to_string_lossy().into_owned(),
                    "--name".into(),
                    name,
                ];
                Self::endpoint(&mut args, hub)?;
                self.hub_command(args).await
            }
            Request::PairText {
                invitation,
                name,
                hub,
            } => {
                if invitation.len() > 16_384 {
                    return Err("邀请文本过长".into());
                }
                let relative =
                    PathBuf::from("commands").join(format!("invite-{}.json", Uuid::new_v4()));
                let temporary = self.path(&relative)?;
                neonmix_identity::files::write_new(&temporary, invitation.as_bytes())
                    .map_err(|e| e.to_string())?;
                let _temporary = TemporaryGuard(temporary.clone());
                let result = Box::pin(self.execute(Request::Pair {
                    invite: relative,
                    credential: "profiles/sender.json".into(),
                    name,
                    hub,
                }))
                .await;
                let _ = std::fs::remove_file(temporary);
                result
            }
            Request::Output { directory, action } => {
                let directory = self.path(&directory)?;
                self.parent(&directory.join("binding.json"))?;
                let sync_name = matches!(&action,OutputAction::Add{provider,..} if provider=="neonmix")
                    || matches!(&action, OutputAction::Rename { .. });
                let mut args = vec!["output".into()];
                match action {
                    OutputAction::Add {
                        credential,
                        hub,
                        name,
                        provider,
                        device,
                    } => {
                        if !["neonmix", "blackhole"].contains(&provider.as_str()) {
                            return Err("不支持的虚拟输出 provider".into());
                        }
                        #[cfg(target_os = "linux")]
                        if provider == "neonmix" {
                            self.ensure_virtual_output_owner(&directory).await?;
                        }
                        args.extend([
                            "add".into(),
                            "--credential".into(),
                            self.credential(&credential)?,
                            "--name".into(),
                            name,
                            "--provider".into(),
                            provider,
                        ]);
                        Self::endpoint(&mut args, hub)?;
                        if let Some(device) = device {
                            args.extend(["--device".into(), device]);
                        }
                    }
                    OutputAction::Show => args.push("show".into()),
                    OutputAction::Rename {
                        expected_revision,
                        name,
                    } => args.extend([
                        "rename".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                        "--name".into(),
                        name,
                    ]),
                    OutputAction::Enable { expected_revision } => args.extend([
                        "enable".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                    ]),
                    OutputAction::Disable { expected_revision } => args.extend([
                        "disable".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                    ]),
                    OutputAction::Remove { expected_revision } => args.extend([
                        "remove".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                    ]),
                }
                args.extend([
                    "--directory".into(),
                    directory.to_string_lossy().into_owned(),
                ]);
                let mut binding = self.hub_command(args).await?;
                if sync_name && binding.get("provider").and_then(Value::as_str) == Some("neonmix") {
                    match self
                        .hub_command(vec![
                            "output".into(),
                            "sync-name".into(),
                            "--directory".into(),
                            directory.to_string_lossy().into_owned(),
                        ])
                        .await
                    {
                        Ok(_) => binding["native_name_synced"] = json!(true),
                        Err(error) => {
                            binding["native_name_synced"] = json!(false);
                            binding["native_name_error"] = json!(error);
                        }
                    }
                }
                Ok(binding)
            }
            Request::ForgetCredential { credential } => {
                if credential.components().next()
                    != Some(Component::Normal(std::ffi::OsStr::new("profiles")))
                {
                    return Err("只能删除 Sender 配对，Hub 管理身份不能从此入口删除".into());
                }
                let path = self.credential(&credential)?;
                let selected =
                    neonmix_identity::profiles::token(Path::new(&path)).map_err(profile_error)?;
                neonmix_identity::profiles::check_forget(
                    Path::new(&path),
                    &selected,
                    &[self.directory.join("hub/admin.json")],
                )
                .map_err(profile_error)?;
                self.sender.stop().await?;
                self.sender_options = None;
                let mut disabled = false;
                for relative in ["output", "outputs/main"] {
                    let directory = self.path(Path::new(relative))?;
                    if !directory.join("binding.json").exists() {
                        continue;
                    }
                    let binding: Value = serde_json::from_slice(
                        &std::fs::read(directory.join("binding.json"))
                            .map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    let revision = binding
                        .get("revision")
                        .and_then(Value::as_u64)
                        .ok_or("输出绑定缺少 revision")?;
                    self.hub_command(vec![
                        "output".into(),
                        "disable".into(),
                        "--directory".into(),
                        directory.to_string_lossy().into_owned(),
                        "--expected-revision".into(),
                        revision.to_string(),
                    ])
                    .await?;
                    disabled = true;
                }
                self.hub_command(vec!["forget".into(), "--credential".into(), path])
                    .await?;
                self.viewer = None;
                Ok(
                    json!({"event":"local_credential_forgotten","output_binding_disabled":disabled,"output_authorization_required":true}),
                )
            }
            Request::Snapshot { credential, hub } | Request::Diagnostics { credential, hub } => {
                let diagnostics = diagnostics_request;
                let mut args = vec![
                    if diagnostics {
                        "diagnostics"
                    } else {
                        "snapshot"
                    }
                    .into(),
                    "--credential".into(),
                    self.credential(&credential)?,
                ];
                Self::endpoint(&mut args, hub.clone())?;
                let mut data = match self.hub_command(args).await {
                    Ok(data) => data,
                    Err(error) if diagnostics => {
                        json!({"available":false,"fault_category":classify_error(&error)})
                    }
                    Err(error) => return Err(error),
                };
                if !diagnostics {
                    let mut args = vec![
                        "me".into(),
                        "--credential".into(),
                        self.credential(&credential)?,
                    ];
                    Self::endpoint(&mut args, hub)?;
                    match self.hub_command(args).await {
                        Ok(viewer) => {
                            self.viewer = Some(viewer.clone());
                            data["viewer"] = viewer;
                        }
                        Err(_) => {
                            self.viewer = None;
                            data["viewer"] = Value::Null;
                        }
                    }
                }
                if diagnostics {
                    redact(&mut data, true);
                    let mut local = json!({"hub":self.hub.status(),"sender":self.sender.status(),"events":self.history,"command_fault":self.command_fault.lock().unwrap_or_else(|p|p.into_inner()).clone()});
                    redact(&mut local, true);
                    data = json!({"remote":data,"local":local,"latency_scope":"network/buffer/output components; no end-to-end measurement"});
                }
                Ok(data)
            }
            Request::ExportDiagnostics { credential, hub } => {
                let diagnostics = match Box::pin(
                    self.execute(Request::Diagnostics { credential, hub }),
                )
                .await
                {
                    Ok(data) => data,
                    Err(_) => {
                        json!({"remote":{"available":false},"local":{"hub":self.hub.status(),"sender":self.sender.status(),"command_fault":self.command_fault.lock().unwrap_or_else(|p|p.into_inner()).clone()}})
                    }
                };
                let safe = export_whitelist(&diagnostics);
                let path = self.directory.join("diagnostics-redacted.json");
                neonmix_identity::files::replace(
                    &path,
                    &serde_json::to_vec_pretty(&safe).map_err(|e| e.to_string())?,
                )
                .map_err(|_| "无法保存诊断导出")?;
                Ok(json!({"file":"diagnostics-redacted.json","diagnostics":safe}))
            }
            Request::Airplay {
                credential,
                hub,
                command,
            } => {
                let credential = self.credential(&credential)?;
                let mut args = vec!["airplay".into(), "--credential".into(), credential];
                Self::endpoint(&mut args, hub)?;
                let mut temporary = None;
                if let Some(command) = command {
                    let path = self
                        .directory
                        .join("commands")
                        .join(format!("airplay-{}.json", Uuid::new_v4()));
                    neonmix_identity::files::write_new(
                        &path,
                        &serde_json::to_vec(&command).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    args.extend(["--command".into(), path.to_string_lossy().into_owned()]);
                    temporary = Some(TemporaryGuard(path));
                }
                let result = self.hub_command(args).await;
                drop(temporary);
                result
            }
            Request::AirplayV2 {
                credential,
                hub,
                command,
            } => {
                let credential = self.credential(&credential)?;
                let mut args = vec!["airplay-v2".into(), "--credential".into(), credential];
                Self::endpoint(&mut args, hub)?;
                let mut temporary = None;
                if let Some(command) = command {
                    let path = self
                        .directory
                        .join("commands")
                        .join(format!("airplay-{}.json", Uuid::new_v4()));
                    neonmix_identity::files::write_new(
                        &path,
                        &serde_json::to_vec(&command).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    args.extend(["--command".into(), path.to_string_lossy().into_owned()]);
                    temporary = Some(TemporaryGuard(path));
                }
                let result = self.hub_command(args).await;
                drop(temporary);
                result
            }
            Request::Control {
                credential,
                hub,
                expected_revision,
                operation,
            } => {
                if matches!(
                    operation,
                    Operation::RegisterDevice { .. } | Operation::Start { .. }
                ) {
                    return Err("桌面不允许直接注册凭证或媒体会话".into());
                }
                let body = neonmix_control::Command {
                    request_id: Uuid::new_v4(),
                    expected_revision,
                    operation,
                };
                let temporary = self
                    .directory
                    .join("commands")
                    .join(format!("{}.json", body.request_id));
                neonmix_identity::files::write_new(
                    &temporary,
                    &serde_json::to_vec(&body).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let _temporary = TemporaryGuard(temporary.clone());
                let credential = match self.credential(&credential) {
                    Ok(c) => c,
                    Err(error) => {
                        let _ = std::fs::remove_file(&temporary);
                        return Err(error);
                    }
                };
                let mut args = vec![
                    "control".into(),
                    "--credential".into(),
                    credential,
                    "--command".into(),
                    temporary.to_string_lossy().into_owned(),
                ];
                let result = match Self::endpoint(&mut args, hub) {
                    Ok(()) => self.hub_command(args).await,
                    Err(e) => Err(e),
                };
                let _ = std::fs::remove_file(temporary);
                result
            }
            Request::SenderStart { options } => {
                if self.hub.child.is_some() {
                    return Err("本实例正在播放；请先停止 Hub".into());
                }
                let credential = self.credential(&options.credential)?;
                let selected: Value =
                    serde_json::from_slice(&std::fs::read(&credential).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                if let Ok(local) = std::fs::read(self.directory.join("hub/admin.json")) {
                    let local: Value = serde_json::from_slice(&local).map_err(|e| e.to_string())?;
                    if local.get("hub_id") == selected.get("hub_id") {
                        return Err("首版拒绝向本实例 Hub 发送".into());
                    }
                }
                let binding = self.path(&options.output_binding)?;
                #[cfg(target_os = "linux")]
                {
                    let saved: Value = serde_json::from_slice(
                        &std::fs::read(binding.join("binding.json"))
                            .map_err(|_| "请先添加输出绑定")?,
                    )
                    .map_err(|_| "输出绑定无效")?;
                    if saved.get("provider").and_then(Value::as_str) == Some("neonmix") {
                        self.ensure_virtual_output_owner(&binding).await?;
                    }
                }
                let mut args = vec![
                    "send".into(),
                    "--credential".into(),
                    credential,
                    "--output-binding".into(),
                    binding.to_string_lossy().into_owned(),
                    "--seconds".into(),
                    "86400".into(),
                ];
                Self::endpoint(&mut args, options.hub.clone())?;
                self.sender.start(&self.hub_bin, args)?;
                self.sender_options = Some(options);
                tokio::time::sleep(Duration::from_millis(250)).await;
                self.sender.reap();
                Ok(serde_json::to_value(self.status()).map_err(|e| e.to_string())?)
            }
            Request::SenderStop => {
                self.sender.stop().await?;
                self.sender_options = None;
                Ok(json!({"event":"sender_user_stopped","automatic_restart":false}))
            }
            Request::Shutdown => {
                self.shutting_down = true;
                self.sender.stop().await?;
                self.hub.stop().await?;
                #[cfg(target_os = "linux")]
                self.virtual_output_owner.stop().await?;
                Ok(json!({"event":"background_stopped"}))
            }
        }
    }
}

struct TemporaryGuard(PathBuf);
impl Drop for TemporaryGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn known_shape(input: &Value, canonical: &Value) -> bool {
    match (input, canonical) {
        (Value::Object(input), Value::Object(canonical)) => input.iter().all(|(key, value)| {
            canonical
                .get(key)
                .is_some_and(|expected| known_shape(value, expected))
        }),
        (Value::Array(input), Value::Array(canonical)) => input
            .iter()
            .zip(canonical)
            .all(|(value, expected)| known_shape(value, expected)),
        _ => true,
    }
}
pub async fn serve(directory: PathBuf) -> Result<()> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let bin = executable.parent().ok_or("missing executable directory")?;
    serve_with_binaries(
        directory,
        bin.join(if cfg!(windows) {
            "neonmix-hub.exe"
        } else {
            "neonmix-hub"
        }),
        bin.join(if cfg!(windows) {
            "neonmix-audio.exe"
        } else {
            "neonmix-audio"
        }),
    )
    .await
}
async fn termination_signal() {
    #[cfg(unix)]
    if let Ok(mut signal) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        signal.recv().await;
        return;
    }
    std::future::pending::<()>().await;
}
/// Explicit binaries are an internal test seam; the production CLI always uses
/// same-directory packaged binaries and accepts no executable over IPC.
pub async fn serve_with_binaries(
    directory: PathBuf,
    hub_bin: PathBuf,
    audio_bin: PathBuf,
) -> Result<()> {
    crate::transport::prepare(&directory)?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let _lock = crate::transport::lock(&directory)?;
    let mut listener = crate::transport::Listener::bind(&directory)?;
    let commands = directory.join("commands");
    crate::transport::prepare(&commands)?;
    let state = Arc::new(Mutex::new(Runtime {
        directory,
        hub_bin,
        audio_bin,
        hub: Managed::default(),
        sender: Managed::default(),
        #[cfg(target_os = "linux")]
        virtual_output_owner: Managed::default(),
        sender_options: None,
        history: VecDeque::new(),
        viewer: None,
        shutting_down: false,
        command_fault: Arc::new(StdMutex::new(None)),
    }));
    let (shutdown, mut requested) = tokio::sync::mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(8));
    let cancellation = Arc::new(ReadCancellation::default());
    let terminated = termination_signal();
    tokio::pin!(terminated);
    loop {
        tokio::select! {
            _=requested.recv()=>break,
            _=&mut terminated=>break,
            _=tokio::signal::ctrl_c()=>break,
            accepted=listener.accept()=>{
                let stream=accepted?;
                if !crate::transport::same_user(&stream){continue;}
                let Ok(permit)=slots.clone().try_acquire_owned() else{continue;};
                let state=state.clone();let shutdown=shutdown.clone();let cancellation=cancellation.clone();
                tokio::spawn(async move {let _permit=permit;let _=connection(stream,state,shutdown,cancellation).await;});
            }
        }
    }
    let mut runtime = state.lock().await;
    runtime.sender.stop().await?;
    runtime.hub.stop().await?;
    #[cfg(target_os = "linux")]
    runtime.virtual_output_owner.stop().await?;
    Ok(())
}
async fn connection(
    mut stream: crate::transport::Stream,
    state: Arc<Mutex<Runtime>>,
    shutdown: tokio::sync::mpsc::Sender<()>,
    cancellation: Arc<ReadCancellation>,
) -> Result<()> {
    let envelope: Result<Envelope> = tokio::time::timeout(Duration::from_secs(3), async {
        let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
        if len == 0 || len > MAX_MESSAGE_BYTES {
            return Err("IPC request too large".into());
        }
        let mut bytes = vec![0; len];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        let raw: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid IPC request".to_string())?;
        let envelope: Envelope =
            serde_json::from_value(raw.clone()).map_err(|_| "invalid IPC request".to_string())?;
        let canonical =
            serde_json::to_value(&envelope).map_err(|_| "invalid IPC request".to_string())?;
        if !known_shape(&raw, &canonical) {
            return Err("unknown IPC request field".into());
        }
        Ok(envelope)
    })
    .await
    .map_err(|_| "IPC request read timeout")?;
    let mut is_shutdown = false;
    let reply = match envelope {
        Ok(envelope) if envelope.version == PROTOCOL_VERSION => {
            is_shutdown = matches!(envelope.request, Request::Shutdown);
            let read_only = matches!(
                &envelope.request,
                Request::Devices
                    | Request::Discover { .. }
                    | Request::Snapshot { .. }
                    | Request::Diagnostics { .. }
                    | Request::Airplay { command: None, .. }
                    | Request::AirplayV2 { command: None, .. }
                    | Request::ExportDiagnostics { .. }
            );
            let epoch = cancellation.epoch.load(Ordering::Acquire);
            if matches!(
                &envelope.request,
                Request::SenderStop
                    | Request::HubStop
                    | Request::Shutdown
                    | Request::ForgetCredential { .. }
            ) {
                cancellation.epoch.fetch_add(1, Ordering::AcqRel);
                cancellation.notify.notify_waiters();
            }
            let event = serde_json::to_value(&envelope.request)
                .ok()
                .and_then(|v| v.get("type").cloned())
                .unwrap_or(Value::Null);
            let mut runtime = tokio::time::timeout(Duration::from_secs(5), state.lock())
                .await
                .map_err(|_| "后台忙碌，请稍后重试")?;
            let interrupted = cancellation.notify.notified();
            tokio::pin!(interrupted);
            interrupted.as_mut().enable();
            let result = if read_only && cancellation.epoch.load(Ordering::Acquire) != epoch {
                Err("查询已被停止入口中断".into())
            } else {
                tokio::select! {
                    _=&mut interrupted,if read_only=>Err("查询已被停止入口中断".into()),
                    result=tokio::time::timeout(Duration::from_secs(35), runtime.execute(envelope.request))=>result.unwrap_or_else(|_|Err("后台请求超时".into())),
                }
            };
            match result {
                Ok(data) => Reply::success(data),
                Err(error) => {
                    if runtime.history.len() == 32 {
                        runtime.history.pop_front();
                    }
                    runtime
                        .history
                        .push_back(json!({"operation":event,"failure":classify_error(&error)}));
                    Reply::failure(error)
                }
            }
        }
        Ok(_) => Reply::failure("incompatible IPC version"),
        Err(error) => Reply::failure(error),
    };
    let mut body = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE_BYTES {
        body = serde_json::to_vec(&Reply::failure("IPC reply too large"))
            .map_err(|e| e.to_string())?;
    }
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        stream.write_u32(body.len() as u32).await?;
        stream.write_all(&body).await?;
        stream.flush().await
    })
    .await;
    // Exiting is honored even if a UI crashes before reading the reply.
    if is_shutdown {
        let _ = shutdown.send(()).await;
    }
    result
        .map_err(|_| "IPC write timeout")?
        .map_err(|e| e.to_string())
}

fn profile_error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    if message.contains("migration_required") {
        "旧配对资料需要离线迁移，请运行 neonmix-credential-migrate".into()
    } else if message.contains("credential_missing") {
        "凭证文件缺失，请恢复完整资料目录".into()
    } else if message.contains("credential_permission_denied") {
        "凭证文件权限不正确或无法访问".into()
    } else if message.contains("credential_store_busy") {
        "凭证正在使用，请稍后重试".into()
    } else if message.contains("administrator_credential_protected") {
        "不能删除 Hub 管理凭证".into()
    } else if message.contains("credential_io_failed") {
        "无法读写凭证文件，请检查目录和磁盘".into()
    } else {
        "配对资料格式无效或版本不受支持".into()
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn receiver_pins_only_survive_authenticated_control_replies() {
        let original = json!({"receivers":[{"receiver_id":"a","pairing_pin":"1111"},{"receiver_id":"b","pairing_pin":"2222"}],"history":[{"pairing_pin":"3333"}],"secret_ref":"private"});
        let mut control = original.clone();
        redact_control_reply(&mut control, true);
        assert_eq!(control["receivers"][0]["pairing_pin"], "1111");
        assert_eq!(control["receivers"][1]["pairing_pin"], "2222");
        assert_eq!(control["history"][0]["pairing_pin"], "[redacted]");
        assert_eq!(control["secret_ref"], "[redacted]");
        let mut other = original;
        redact_control_reply(&mut other, false);
        assert_eq!(other["receivers"][0]["pairing_pin"], "[redacted]");
        assert!(!export_whitelist(&control).to_string().contains("1111"));
    }

    #[test]
    fn discovery_returns_the_final_candidate_snapshot_instead_of_transient_events() {
        let candidate = json!({"hub_id":"test-hub", "room_name":"客厅"});
        let completion = json!({"event":"discovery_complete", "candidates":[candidate]});
        let events = json!([
            {"event":"resolved", "candidate":candidate},
            {"event":"resolved", "candidate":candidate},
            completion,
        ]);
        assert_eq!(discovery_result(events).unwrap(), completion);
    }

    #[test]
    fn discovery_empty_completion_clears_removed_candidates_and_supports_single_line_output() {
        let completion = json!({"event":"discovery_complete", "candidates":[]});
        assert_eq!(discovery_result(completion.clone()).unwrap(), completion);
        assert_eq!(
            discovery_result(json!([
                {"event":"resolved", "candidate":{"hub_id":"removed-hub"}},
                completion,
            ]))
            .unwrap(),
            completion,
        );
    }

    #[test]
    fn discovery_incomplete_or_malformed_output_is_an_error() {
        for value in [
            Value::Null,
            json!([]),
            json!([{"event":"resolved", "candidate":{}}]),
            json!({"event":"discovery_complete", "candidates":"invalid"}),
        ] {
            assert!(discovery_result(value).is_err());
        }
    }
}
