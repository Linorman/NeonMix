use crate::intent::resolve_airplay_patch;
use crate::manager::{Kind, Manager, StartTicket};
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
    process::Command,
    sync::{Mutex, Notify, Semaphore},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);
const LOG_LINE_LIMIT: usize = 16_384;
/// Up to 10 s for `hub serve` to bind or fail (first launch scans GStreamer).
const MEDIA_START_TIMEOUT: Duration = Duration::from_secs(10);

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
        .ok_or_else(|| "discovery_incomplete".into())
}

pub(crate) fn audio_command(executable: &Path, args: Vec<String>) -> Command {
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
use crate::managed::Managed;
fn classify_error(message: &str) -> String {
    // CLI libraries sometimes wrap a machine code in `Error: "..."`. Only
    // recognized lexical tokens are retained; surrounding private data is lost.
    if let Some(fault) = Fault::from_machine_code(message).or_else(|| {
        message
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .find_map(Fault::from_machine_code)
    }) {
        return fault.code.as_str().into();
    }
    let m = message.to_lowercase();
    let code =
        if m.contains("macos microphone access") || m.contains("audio capture permission denied") {
            FaultCode::CapturePermissionDenied
        } else if m.contains("unsupported audio format") {
            FaultCode::UnsupportedAudioFormat
        } else if m.contains("address already in use")
            || m.contains("os error 48)")
            || m.contains("os error 98)")
            || m.contains("os error 10048)")
        {
            FaultCode::HubPortInUse
        } else if m.contains("revok")
            || m.contains("unauthoriz")
            || m.contains("permission")
            || m.contains("rejected")
        {
            FaultCode::PermissionDenied
        } else if m.contains("capture") {
            FaultCode::CaptureUnavailable
        } else if m.contains("output") || m.contains("device") {
            FaultCode::OutputUnavailable
        } else if m.contains("timeout") {
            FaultCode::BackgroundTimeout
        } else if m.contains("connect") || m.contains("offline") || m.contains("network") {
            FaultCode::ConnectionUnavailable
        } else {
            FaultCode::GenericFailure
        };
    code.as_str().into()
}
pub(crate) async fn drain(
    reader: impl AsyncRead + Unpin,
    status: Arc<StdMutex<ProcessStatus>>,
    stderr: bool,
    ready_event: Option<&'static str>,
) {
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
            record_process_line(&line, &status, stderr, ready_event);
        }
        line.clear();
        oversized = false;
    }
    if !oversized && !line.is_empty() {
        record_process_line(&line, &status, stderr, ready_event);
    }
}
fn record_process_line(
    line: &[u8],
    status: &Arc<StdMutex<ProcessStatus>>,
    stderr: bool,
    ready_event: Option<&str>,
) {
    let mut current = status.lock().unwrap_or_else(|p| p.into_inner());
    if stderr {
        let error = classify_error(&String::from_utf8_lossy(line));
        if error != "generic_failure" || current.fault.is_none() {
            current.set_failure(&error);
        }
    } else if let Ok(mut event) = serde_json::from_slice::<Value>(line) {
        if current.guardian_version == 1
            && event["event"] == "managed_child_spawned"
            && event["guardian_version"] == 1
            && let Some(pid) = event["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid > 0)
        {
            current.pid = Some(pid);
            if event["ready_version"] != 1 {
                current.set_failure("ipc_incompatible_version");
            }
            // Owner protocol records are private process metadata, not a new
            // ordinary log that can overwrite the child's latest business event.
            return;
        }
        if current.guardian_version == 1
            && event["event"] == "managed_child_ready"
            && event["guardian_version"] == 1
        {
            if ready_event.is_some_and(|expected| event["kind"] == expected)
                && current.pid.map(u64::from) == event["pid"].as_u64()
                && current.running
                && !current.stopping
                && current.fault.is_none()
            {
                current.ready = true;
            }
            return;
        }
        if current.guardian_version == 1
            && event["event"] == "managed_child_exited"
            && event["guardian_version"] == 1
        {
            current.ready = false;
            if !current.stopping {
                current.set_failure("process_exited");
            }
            current.stopping = true;
            return;
        }
        if current.guardian_version == 1 && event["event"] == "guardian_stopped" {
            current.guardian_result = serde_json::from_value(event["result"].clone()).ok();
            return;
        }
        if current.guardian_version == 0
            && ready_event.is_some_and(|expected| event["event"] == expected)
            && current.running
            && !current.stopping
            && current.fault.is_none()
        {
            current.ready = true;
        }
        if let Some(fault) = event
            .get("fault")
            .cloned()
            .and_then(|value| serde_json::from_value::<Fault>(value).ok())
        {
            current.set_failure(
                &serde_json::to_string(&fault).unwrap_or_else(|_| "generic_failure".into()),
            );
        } else if let Some(fault) = event
            .get("error")
            .and_then(Value::as_str)
            .and_then(Fault::from_machine_code)
        {
            current.set_failure(fault.code.as_str());
        }
        if event["event"] == "sender_started" && current.running && !current.stopping {
            current.sender_target = serde_json::from_value(event["sender_target"].clone())
                .ok()
                .filter(|target: &crate::SenderTarget| !target.hub_id.is_nil());
        }
        redact(&mut event, false);
        if event
            .get("event")
            .and_then(Value::as_str)
            .is_some_and(|e| e.ends_with("stats"))
        {
            current.metrics = Some(event.clone());
            current.metrics_sequence = current.metrics_sequence.saturating_add(1);
            current.metrics_observed_at = Some(std::time::Instant::now());
        }
        current.last_event = Some(event);
    }
}

/// Export diagnostics never contains names, device/host identifiers, routes,
/// credential refs, certificates or arbitrary error text. Functional snapshots
/// retain their identifiers so revision-checked controls remain possible.
pub fn redact(value: &mut Value, diagnostic: bool) {
    redact_at(value, diagnostic, 0);
}
// 0: ordinary data, 1: meters, 2: lanes array, 3: one meter lane.
fn redact_at(value: &mut Value, diagnostic: bool, scope: u8) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = key.to_lowercase();
                let meter_session = scope == 3
                    && key == "session_id"
                    && (value.is_null()
                        || value.as_u64().is_some()
                        || value.as_str().is_some_and(|id| Uuid::parse_str(id).is_ok()));
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
                            || (key.ends_with("id") && key != "stream_id" && !meter_session)
                            || key.contains("address")
                            || key.contains("endpoint")
                            || key == "listen"
                            || (key == "hub" && value.is_string())
                            || key == "output_binding"
                            || key == "path"))
                {
                    *value = Value::String("[redacted]".into());
                } else {
                    let next = if key == "meters" {
                        1
                    } else if scope == 1 && key == "lanes" {
                        2
                    } else {
                        0
                    };
                    redact_at(value, diagnostic, next);
                }
            }
        }
        Value::Array(array) => {
            for value in array {
                redact_at(value, diagnostic, if scope == 2 { 3 } else { 0 });
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
                    "network",
                    "configuration_code",
                    "effective_policy_code",
                    "network_ready",
                    "active_profiles",
                    "firewall_enabled",
                    "rules_match",
                    "matching_block_rule",
                    "network_categories",
                    "network_categories_known",
                    "uac_cancelled",
                    "stopping",
                    "stop_result",
                    "graceful",
                    "forced",
                    "elapsed_ms",
                    "exit_code",
                    "cleanup_complete",
                    "cleanup_failures",
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
                    // Per-cause AirPlay ingress counters: without them an
                    // exported report cannot tell which check dropped audio.
                    "accepted_packets",
                    "released_packets",
                    "identity_rejections",
                    "malformed_packets",
                    "future_packets",
                    "overflow_packets",
                    "clock_resets",
                    "timeline_rejections",
                    "coordinate_rejections",
                    "pts_rejections",
                    "source_gap_frames",
                    "last_normalized_sample_position",
                    "last_uncertainty_ns",
                    "max_control_ns",
                    "max_pcm_ns",
                    "queued_blocks",
                    "rejected_blocks",
                    "received_blocks",
                    "released_blocks",
                    "pending_frames",
                    "pending_bytes",
                    "available",
                    "sample_age_ms",
                    "metrics_age_ms",
                    "metrics_sequence",
                    "command_fault",
                    "operation_failed",
                    "media_application",
                    "desired_config_sequence",
                    "applied_config_sequence",
                    "stalled",
                    "pending_ms",
                    "queue_rejections",
                    "persistence",
                    "pending",
                    "durable",
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
                    "ready",
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
                    "rendered_pcm_frames_by_lane",
                    "render_state_by_lane",
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
    lifecycle: Manager,
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
    async fn stop_all(&mut self) -> Result<()> {
        #[cfg(not(target_os = "linux"))]
        let (sender, hub) = tokio::join!(self.sender.stop(), self.hub.stop());
        #[cfg(target_os = "linux")]
        let (sender, hub, virtual_output) = tokio::join!(
            self.sender.stop(),
            self.hub.stop(),
            self.virtual_output_owner.stop()
        );
        let mut failures = Vec::new();
        for result in [sender, hub] {
            if let Err(error) = result {
                failures.push(error);
            }
        }
        #[cfg(target_os = "linux")]
        if let Err(error) = virtual_output {
            failures.push(error);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    #[cfg(target_os = "linux")]
    async fn ensure_virtual_output_owner(&mut self, binding_directory: &Path) -> Result<()> {
        let output_id = neonmix_output_binding::Store::new(binding_directory)
            .load()
            .map_err(|_| "invalid_output_binding")?
            .output_id;
        let instance = self.lifecycle.instance_generation();
        self.virtual_output_owner.reap();
        if self.virtual_output_owner.child.is_none() {
            if let Some(owner) = crate::linux_owner::inspect_owner()? {
                owner.matches(instance, output_id).map_err(str::to_owned)?;
                // A process handle is required; never adopt an unowned media process.
                return Err("lifecycle_owner_lost".into());
            }
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
                    "--instance-generation".into(),
                    instance.to_string(),
                ],
            )?;
        }
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                self.virtual_output_owner.reap();
                if self.virtual_output_owner.child.is_none() {
                    return Err("output_unavailable".into());
                }
                if let Some(owner) = crate::linux_owner::inspect_owner()?
                    && owner.matches(instance, output_id).map_err(str::to_owned)?
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| "output_unavailable".to_string())?
    }

    fn path(&self, relative: &Path) -> Result<PathBuf> {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("invalid_path".into());
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
                if path
                    .file_name()?
                    .to_string_lossy()
                    .starts_with(".neonmix-intent-")
                {
                    return None;
                }
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
        let status = ServiceStatus {
            lifecycle: None,
            intent_version: crate::INTENT_VERSION,
            managed_control_version: 1,
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
        };
        self.lifecycle.publish(
            status.clone(),
            self.hub.signal(),
            self.sender.signal(),
            #[cfg(target_os = "linux")]
            self.virtual_output_owner.signal(),
        );
        status
    }
    async fn run(&self, executable: &Path, args: Vec<String>) -> Result<Value> {
        let preserve_airplay_pin = args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "airplay" | "airplay-v2"));
        let mut child = audio_command(executable, args)
            .spawn()
            .map_err(|_| "runtime_unavailable".to_string())?;
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
            Ok(result) => result.map_err(|_| "invalid_backend_response".to_string())?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                output.abort();
                errors.abort();
                return Err("background_timeout".into());
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
            return Err("ipc_message_too_large".into());
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
            // Valid structured fault takes priority over the old machine error
            // and stderr, and accepts only public parameters from the closed schema.
            let events: Vec<Value> = bytes
                .split(|b| *b == b'\n')
                .filter_map(|line| serde_json::from_slice(line).ok())
                .collect();
            if let Some(fault) = events.iter().find_map(|event| {
                event
                    .get("fault")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Fault>(v).ok())
            }) {
                return Err(
                    serde_json::to_string(&fault).unwrap_or_else(|_| "generic_failure".into())
                );
            }
            if let Some(fault) = events.iter().find_map(|event| {
                event
                    .get("error")
                    .and_then(Value::as_str)
                    .and_then(Fault::from_machine_code)
            }) {
                return Err(fault.code.as_str().into());
            }
            return Err(classify_error(&String::from_utf8_lossy(&errors)));
        }
        let mut events: Vec<Value> = bytes
            .split(|b| *b == b'\n')
            .filter(|b| !b.is_empty())
            .map(|line| {
                serde_json::from_slice(line).map_err(|_| "invalid_backend_response".to_string())
            })
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
    async fn network_inspection(&self) -> Value {
        #[cfg(windows)]
        {
            let helper = self.hub_bin.with_file_name("neonmix-network-helper.exe");
            if helper.is_file()
                && let Ok(value) = self
                    .run(
                        &helper,
                        vec!["inspect".into(), "--port".into(), "7443".into()],
                    )
                    .await
                && value["configuration_code"].is_number()
                && value["effective_policy_code"].is_number()
            {
                return value;
            }
        }
        json!({"configuration_code":4,"effective_policy_code":4,"network_ready":false})
    }
    async fn hub_command(&self, args: Vec<String>) -> Result<Value> {
        self.run(&self.hub_bin, args).await
    }
    fn endpoint(args: &mut Vec<String>, hub: Option<String>) -> Result<()> {
        if let Some(hub) = hub {
            if !hub.starts_with("https://") || hub.len() > 512 || hub.chars().any(char::is_control)
            {
                return Err("invalid_argument".into());
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
            return Err("invalid_argument".into());
        }
        Ok(())
    }
    async fn execute(
        &mut self,
        request: Request,
        start_ticket: Option<StartTicket>,
    ) -> Result<Value> {
        if (self.shutting_down || self.lifecycle.is_shutdown())
            && !matches!(&request, Request::Status | Request::Shutdown)
        {
            return Err("background_shutting_down".into());
        }
        self.hub.reap();
        self.sender.reap();
        let diagnostics_request = matches!(&request, Request::Diagnostics { .. });
        match request {
            Request::LifecycleStart { .. } | Request::LifecycleStop { .. } => {
                Err("ipc_invalid_request".into())
            }
            Request::Status => self.lifecycle.status(),
            Request::LifecycleOperation {
                operation_id,
                instance_generation,
            } => self.lifecycle.lookup(operation_id, instance_generation),
            Request::Devices => self.run(&self.audio_bin, vec!["devices".into()]).await,
            Request::TestTone { output } => {
                if output.is_empty() || output.len() > 512 {
                    return Err("output_unavailable".into());
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
                    return Err("hub_requires_sender_stop".into());
                }
                let directory = self.path(Path::new("hub"))?;
                let profile = directory.join("server.json");
                if std::fs::symlink_metadata(&profile).is_ok() {
                    // An old profile must show migration guidance even when the UI
                    // cannot populate its settings from the current schema.
                    neonmix_identity::profiles::hub(&profile).map_err(profile_error)?;
                    return Err("hub_setup_exists".into());
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
                    return Err("hub_requires_sender_stop".into());
                }
                let profile = self.path(Path::new("hub/server.json"))?;
                neonmix_identity::hub_settings::recover(&profile, validate_settings_state)
                    .map_err(transaction_error)?;
                neonmix_identity::profiles::hub(&profile).map_err(profile_error)?;
                let ticket = start_ticket.ok_or("request_interrupted")?;
                self.lifecycle.install(ticket, || {
                    self.hub.start(
                        &self.hub_bin,
                        vec![
                            "serve".into(),
                            "--config".into(),
                            profile.to_string_lossy().into_owned(),
                        ],
                    )?;
                    Ok(self.hub.signal())
                })?;
                self.status();
                self.hub.wait_ready(MEDIA_START_TIMEOUT).await?;
                Ok(serde_json::to_value(self.status()).map_err(|e| e.to_string())?)
            }
            Request::HubStop => {
                self.hub.stop().await?;
                Ok(json!({"event":"hub_stopped","stop_result":self.hub.status().stop_result}))
            }
            Request::HubSettings { settings } => {
                Self::validate_settings(&settings)?;
                if self.hub.child.is_some() {
                    return Err("hub_settings_require_stop".into());
                }
                let config = self.path(Path::new("hub/server.json"))?;
                neonmix_identity::hub_settings::update(
                    &config,
                    &settings.output,
                    &settings.name,
                    validate_settings_state,
                )
                .map_err(transaction_error)?;
                Ok(json!(settings))
            }
            Request::Discover { seconds } => {
                if !(1..=10).contains(&seconds) {
                    return Err("invalid_argument".into());
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
                    return Err("invalid_argument".into());
                }
                let credential = self.credential(&credential)?;
                let out = self.path(&out)?;
                if out.exists() {
                    return Err("invalid_path".into());
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
                    json!(std::fs::read_to_string(out).map_err(|_| "credential_io_failed")?);
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
                    return Err("invalid_argument".into());
                }
                let invite = self.path(&invite)?;
                let credential = self.path(&credential)?;
                self.parent(&credential)?;
                let invitation: Value = serde_json::from_slice(
                    &std::fs::read(&invite).map_err(|_| "credential_io_failed")?,
                )
                .map_err(|_| "invalid_argument")?;
                if let Ok(local) = std::fs::read(self.directory.join("hub/admin.json")) {
                    let local: Value =
                        serde_json::from_slice(&local).map_err(|_| "credential_corrupt")?;
                    if local.get("hub_id") == invitation.get("hub_id") {
                        return Err("self_connection_forbidden".into());
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
                    return Err("invalid_argument".into());
                }
                let relative =
                    PathBuf::from("commands").join(format!("invite-{}.json", Uuid::new_v4()));
                let temporary = self.path(&relative)?;
                neonmix_identity::files::write_new(&temporary, invitation.as_bytes())
                    .map_err(|e| e.to_string())?;
                let _temporary = TemporaryGuard(temporary.clone());
                let result = Box::pin(self.execute(
                    Request::Pair {
                        invite: relative,
                        credential: "profiles/sender.json".into(),
                        name,
                        hub,
                    },
                    None,
                ))
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
                            return Err("invalid_argument".into());
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
                        expected_output_id,
                        name,
                    } => args.extend([
                        "rename".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                        "--expected-output-id".into(),
                        expected_output_id.to_string(),
                        "--name".into(),
                        name,
                    ]),
                    OutputAction::Enable {
                        expected_revision,
                        expected_output_id,
                    } => args.extend([
                        "enable".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                        "--expected-output-id".into(),
                        expected_output_id.to_string(),
                    ]),
                    OutputAction::Disable {
                        expected_revision,
                        expected_output_id,
                    } => args.extend([
                        "disable".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                        "--expected-output-id".into(),
                        expected_output_id.to_string(),
                    ]),
                    OutputAction::Remove {
                        expected_revision,
                        expected_output_id,
                    } => args.extend([
                        "remove".into(),
                        "--expected-revision".into(),
                        expected_revision.to_string(),
                        "--expected-output-id".into(),
                        expected_output_id.to_string(),
                    ]),
                }
                args.extend([
                    "--directory".into(),
                    directory.to_string_lossy().into_owned(),
                ]);
                let mut binding = self.hub_command(args).await?;
                if sync_name && binding.get("provider").and_then(Value::as_str) == Some("neonmix") {
                    let id = binding["output_id"]
                        .as_str()
                        .and_then(|value| uuid::Uuid::parse_str(value).ok())
                        .filter(|id| !id.is_nil())
                        .ok_or("invalid_backend_response")?;
                    let revision = binding["revision"]
                        .as_u64()
                        .ok_or("invalid_backend_response")?;
                    match self
                        .hub_command(vec![
                            "output".into(),
                            "sync-name".into(),
                            "--expected-output-id".into(),
                            id.to_string(),
                            "--expected-revision".into(),
                            revision.to_string(),
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
                    return Err("administrator_credential_protected".into());
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
                        .ok_or("invalid_output_binding")?;
                    let id = binding
                        .get("output_id")
                        .and_then(Value::as_str)
                        .and_then(|value| uuid::Uuid::parse_str(value).ok())
                        .filter(|id| !id.is_nil())
                        .ok_or("invalid_output_binding")?;
                    self.hub_command(vec![
                        "output".into(),
                        "disable".into(),
                        "--directory".into(),
                        directory.to_string_lossy().into_owned(),
                        "--expected-revision".into(),
                        revision.to_string(),
                        "--expected-output-id".into(),
                        id.to_string(),
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
                    if let Some(object) = data.as_object_mut() {
                        object.entry("sample_age_ms").or_insert(json!(0));
                    }
                    let mut local = json!({"hub":self.hub.status(),"sender":self.sender.status(),"events":self.history,"command_fault":self.command_fault.lock().unwrap_or_else(|p|p.into_inner()).clone()});
                    local["network"] = self.network_inspection().await;
                    redact(&mut local, true);
                    data = json!({"remote":data,"local":local,"latency_scope":"network/buffer/output components; no end-to-end measurement"});
                }
                Ok(data)
            }
            Request::ExportDiagnostics { credential, hub } => {
                let diagnostics = match Box::pin(
                    self.execute(Request::Diagnostics { credential, hub }, None),
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
                .map_err(|_| "diagnostics_save_failed")?;
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
            Request::ResolveAirplayPatch {
                credential,
                hub,
                hub_id,
                credential_id,
                runtime_epoch,
                stream_epoch,
                command,
                guard,
            } => {
                let path = self.credential(&credential)?;
                let profile =
                    neonmix_identity::profiles::token(Path::new(&path)).map_err(profile_error)?;
                if profile.hub_id != hub_id
                    || profile.device_id != Some(credential_id)
                    || profile.pending
                {
                    return Err("unauthenticated".into());
                }
                let id = Uuid::parse_str(&command.command_id).map_err(|_| "invalid_command_id")?;
                let frozen = Path::new(&path)
                    .parent()
                    .ok_or("invalid_path")?
                    .join(format!(".neonmix-intent-{id}.json"));
                neonmix_identity::files::write_new(
                    &frozen,
                    &serde_json::to_vec(&profile).map_err(|_| "invalid_backend_response")?,
                )
                .map_err(|_| "credential_write_failed")?;
                let _credential_guard = TemporaryGuard(frozen.clone());
                let mut args = vec![
                    "airplay-v2".into(),
                    "--credential".into(),
                    frozen.to_string_lossy().into_owned(),
                ];
                Self::endpoint(&mut args, hub)?;
                let view = self.hub_command(args).await?;
                resolve_airplay_patch(
                    &view,
                    runtime_epoch,
                    stream_epoch,
                    credential_id,
                    command,
                    guard.as_deref(),
                )
            }
            Request::Intent {
                credential,
                hub,
                hub_id,
                credential_id,
                command,
            } => {
                let credential_path = self.credential(&credential)?;
                let profile = neonmix_identity::profiles::token(Path::new(&credential_path))
                    .map_err(profile_error)?;
                if profile.hub_id != hub_id
                    || profile.device_id != Some(credential_id)
                    || profile.pending
                {
                    return Err("unauthenticated".into());
                }
                let (action, id, bytes) = match command {
                    IntentCommand::Native(command) => {
                        if matches!(
                            command.operation,
                            Operation::RegisterDevice { .. } | Operation::Start { .. }
                        ) {
                            return Err("permission_denied".into());
                        }
                        if command.runtime_epoch.is_none_or(|epoch| epoch.is_nil())
                            || command.credential_id != Some(credential_id)
                        {
                            return Err("upgrade_required".into());
                        }
                        ("control", command.request_id, serde_json::to_vec(&command))
                    }
                    IntentCommand::Airplay(command) => {
                        if command.runtime_epoch.as_ref().is_none_or(|epoch| {
                            Uuid::parse_str(epoch).map_or(true, |epoch| epoch.is_nil())
                        }) || command.credential_id.as_deref()
                            != Some(credential_id.to_string().as_str())
                        {
                            return Err("upgrade_required".into());
                        }
                        let id = Uuid::parse_str(&command.command_id)
                            .map_err(|_| "invalid_command_id")?;
                        ("airplay-v2", id, serde_json::to_vec(&command))
                    }
                };
                // Freeze metadata next to its immutable secret reference. A
                // replacement at the UI's credential path cannot retarget CLI.
                let frozen = Path::new(&credential_path)
                    .parent()
                    .ok_or("invalid_path")?
                    .join(format!(".neonmix-intent-{id}.json"));
                neonmix_identity::files::write_new(
                    &frozen,
                    &serde_json::to_vec(&profile).map_err(|_| "invalid_backend_response")?,
                )
                .map_err(|_| "credential_write_failed")?;
                let _credential_guard = TemporaryGuard(frozen.clone());
                let temporary = self
                    .directory
                    .join("commands")
                    .join(format!("intent-{id}.json"));
                neonmix_identity::files::write_new(
                    &temporary,
                    &bytes.map_err(|_| "invalid_command")?,
                )
                .map_err(|_| "credential_write_failed")?;
                let _command_guard = TemporaryGuard(temporary.clone());
                let mut args = vec![
                    action.into(),
                    "--credential".into(),
                    frozen.to_string_lossy().into_owned(),
                    "--command".into(),
                    temporary.to_string_lossy().into_owned(),
                ];
                Self::endpoint(&mut args, hub)?;
                self.hub_command(args).await
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
                    return Err("permission_denied".into());
                }
                let body = neonmix_control::Command {
                    control_version: 1,
                    expected_config_revision: None,
                    expected_event_sequence: None,
                    runtime_epoch: None,
                    credential_id: None,
                    request_id: Uuid::new_v4(),
                    expected_revision: Some(expected_revision),
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
                    return Err("sender_requires_hub_stop".into());
                }
                let credential = self.credential(&options.credential)?;
                let selected: Value =
                    serde_json::from_slice(&std::fs::read(&credential).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                if let Ok(local) = std::fs::read(self.directory.join("hub/admin.json")) {
                    let local: Value = serde_json::from_slice(&local).map_err(|e| e.to_string())?;
                    if local.get("hub_id") == selected.get("hub_id") {
                        return Err("self_connection_forbidden".into());
                    }
                }
                let binding = self.path(&options.output_binding)?;
                #[cfg(target_os = "linux")]
                {
                    let saved: Value = serde_json::from_slice(
                        &std::fs::read(binding.join("binding.json"))
                            .map_err(|_| "output_binding_required")?,
                    )
                    .map_err(|_| "invalid_output_binding")?;
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
                    "--until-stopped".into(),
                ];
                #[cfg(target_os = "linux")]
                args.extend([
                    "--instance-generation".into(),
                    self.lifecycle.instance_generation().to_string(),
                ]);
                Self::endpoint(&mut args, options.hub.clone())?;
                let ticket = start_ticket.ok_or("request_interrupted")?;
                self.lifecycle.install(ticket, || {
                    self.sender.start(&self.hub_bin, args)?;
                    Ok(self.sender.signal())
                })?;
                self.sender_options = Some(options);
                self.status();
                if let Err(error) = self.sender.wait_ready(MEDIA_START_TIMEOUT).await {
                    self.sender_options = None;
                    return Err(error);
                }
                Ok(serde_json::to_value(self.status()).map_err(|e| e.to_string())?)
            }
            Request::SenderStop => {
                self.sender.stop().await?;
                self.sender_options = None;
                Ok(
                    json!({"event":"sender_user_stopped","automatic_restart":false,"stop_result":self.sender.status().stop_result}),
                )
            }
            Request::Shutdown => {
                self.shutting_down = true;
                self.stop_all().await?;
                Ok(
                    json!({"event":"background_stopped","hub":self.hub.status().stop_result,"sender":self.sender.status().stop_result}),
                )
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
    neonmix_identity::hub_settings::recover(
        &directory.join("hub/server.json"),
        validate_settings_state,
    )
    .map_err(transaction_error)?;
    let mut listener = crate::transport::Listener::bind(&directory)?;
    let mut lifecycle_listener = crate::transport::Listener::bind_endpoint(&directory, true)?;
    let lifecycle = Manager::new();
    let commands = directory.join("commands");
    crate::transport::prepare(&commands)?;
    let mut runtime = Runtime {
        lifecycle: lifecycle.clone(),
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
    };
    runtime.status();
    let state = Arc::new(Mutex::new(runtime));
    let (shutdown, mut requested) = tokio::sync::mpsc::channel(1);
    let slots = Arc::new(Semaphore::new(8));
    let cancellation = Arc::new(ReadCancellation::default());
    let lifecycle_slots = Arc::new(Semaphore::new(8));
    let mut lifecycle_connections = tokio::task::JoinSet::new();
    let mut connections = tokio::task::JoinSet::new();
    let terminated = termination_signal();
    tokio::pin!(terminated);
    loop {
        tokio::select! {
            _=requested.recv()=>break,
            _=&mut terminated=>break,
            _=tokio::signal::ctrl_c()=>break,
            _=lifecycle_connections.join_next(),if !lifecycle_connections.is_empty()=>{},
            _=connections.join_next(),if !connections.is_empty()=>{},
            accepted=lifecycle_listener.accept()=>{
                let stream=accepted?;
                if !crate::transport::same_user(&stream){continue;}
                let Ok(permit)=lifecycle_slots.clone().try_acquire_owned() else{continue;};
                let state=state.clone();let shutdown=shutdown.clone();let cancellation=cancellation.clone();let lifecycle=lifecycle.clone();
                lifecycle_connections.spawn(async move {let _permit=permit;let _=connection(stream,state,shutdown,cancellation,lifecycle,true).await;});
            }
            accepted=listener.accept()=>{
                let stream=accepted?;
                if !crate::transport::same_user(&stream){continue;}
                let Ok(permit)=slots.clone().try_acquire_owned() else{continue;};
                let state=state.clone();let shutdown=shutdown.clone();let cancellation=cancellation.clone();let lifecycle=lifecycle.clone();
                connections.spawn(async move {let _permit=permit;let _=connection(stream,state,shutdown,cancellation,lifecycle,false).await;});
            }
        }
    }
    drop(lifecycle_listener);
    drop(listener);
    // Completed stop replies get a short drain; unfinished metadata clients
    // cannot retain a named-pipe instance after this owner releases its lock.
    // Stop operations themselves belong to Manager and survive client closure.
    if tokio::time::timeout(Duration::from_millis(250), async {
        while lifecycle_connections.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        lifecycle_connections.abort_all();
        while lifecycle_connections.join_next().await.is_some() {}
    }
    // Signal-owned children can exit while an unrelated remote transaction is
    // still holding Runtime. Background exit never waits for that lock.
    let accepted = lifecycle.stop(Kind::Shutdown, shutdown)?;
    let id = serde_json::from_value(accepted["operation_id"].clone())
        .map_err(|_| "invalid_backend_response")?;
    let instance = serde_json::from_value(accepted["instance_generation"].clone())
        .map_err(|_| "invalid_backend_response")?;
    loop {
        let operation = lifecycle.lookup(id, instance)?;
        if operation["state"] == "completed" {
            // Release ordinary IPC streams, Runtime and kill-on-drop command
            // children before relinquishing the daemon lock. Detached handlers
            // otherwise survive serve(), retaining named pipes and old owners.
            connections.abort_all();
            while connections.join_next().await.is_some() {}
            return if operation["ok"] == true {
                Ok(())
            } else {
                Err("runtime_cleanup_incomplete".into())
            };
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn connection(
    mut stream: crate::transport::Stream,
    state: Arc<Mutex<Runtime>>,
    shutdown: tokio::sync::mpsc::Sender<()>,
    cancellation: Arc<ReadCancellation>,
    lifecycle: Manager,
    lifecycle_only: bool,
) -> Result<()> {
    let envelope: Result<(Envelope, Option<StartTicket>)> =
        tokio::time::timeout(Duration::from_secs(3), async {
            let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
            if len == 0
                || len
                    > if lifecycle_only {
                        1024
                    } else {
                        MAX_MESSAGE_BYTES
                    }
            {
                return Err("ipc_message_too_large".into());
            }
            let mut bytes = vec![0; len];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|e| e.to_string())?;
            let raw: Value =
                serde_json::from_slice(&bytes).map_err(|_| "ipc_invalid_request".to_string())?;
            let mut envelope: Envelope = serde_json::from_value(raw.clone())
                .map_err(|_| "ipc_invalid_request".to_string())?;
            let canonical =
                serde_json::to_value(&envelope).map_err(|_| "ipc_invalid_request".to_string())?;
            if !known_shape(&raw, &canonical) {
                return Err("ipc_invalid_request".into());
            }
            let bound_start = if let Request::LifecycleStart {
                instance_generation,
                expected_stop_generation,
                request,
            } = &envelope.request
            {
                let kind = match request.as_ref() {
                    Request::HubStart => Kind::Hub,
                    Request::SenderStart { .. } => Kind::Sender,
                    _ => return Err("ipc_invalid_request".into()),
                };
                Some(lifecycle.bound_ticket(
                    kind,
                    *instance_generation,
                    *expected_stop_generation,
                )?)
            } else {
                None
            };
            if let Request::LifecycleStop {
                instance_generation,
                request,
            } = &envelope.request
            {
                if !matches!(
                    request.as_ref(),
                    Request::HubStop | Request::SenderStop | Request::Shutdown
                ) {
                    return Err("ipc_invalid_request".into());
                }
                if !lifecycle.matches_instance(*instance_generation) {
                    return Err("request_interrupted".into());
                }
            }
            if let Request::LifecycleStart { request, .. }
            | Request::LifecycleStop { request, .. } = envelope.request
            {
                envelope.request = *request;
            }
            Ok((envelope, bound_start))
        })
        .await
        .map_err(|_| "background_timeout")?;
    let mut is_shutdown = false;
    let reply = match envelope {
        Ok((envelope, _))
            if envelope.version == PROTOCOL_VERSION
                && crate::manager::is_lifecycle(&envelope.request) =>
        {
            let result = if let Some(kind) = crate::manager::kind(&envelope.request) {
                cancellation.epoch.fetch_add(1, Ordering::AcqRel);
                cancellation.notify.notify_waiters();
                lifecycle.stop(kind, shutdown.clone())
            } else {
                match envelope.request {
                    Request::Status => lifecycle.status(),
                    Request::LifecycleOperation {
                        operation_id,
                        instance_generation,
                    } => lifecycle.lookup(operation_id, instance_generation),
                    _ => unreachable!(),
                }
            };
            match result {
                Ok(data) => Reply::success(data),
                Err(error) => Reply::failure(error),
            }
        }
        Ok(_) if lifecycle_only => Reply::failure("ipc_invalid_request"),
        Ok((envelope, bound_start)) if envelope.version == PROTOCOL_VERSION => {
            is_shutdown = matches!(envelope.request, Request::Shutdown);
            let start_ticket = bound_start.or_else(|| match &envelope.request {
                Request::HubStart => lifecycle.ticket(Kind::Hub).ok(),
                Request::SenderStart { .. } => lifecycle.ticket(Kind::Sender).ok(),
                _ => None,
            });
            let read_only = matches!(
                &envelope.request,
                Request::Devices
                    | Request::ResolveAirplayPatch { .. }
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
            match tokio::time::timeout(Duration::from_secs(5), state.lock()).await {
                Err(_) => Reply::failure("background_busy"),
                Ok(mut runtime) => {
                    let interrupted = cancellation.notify.notified();
                    tokio::pin!(interrupted);
                    interrupted.as_mut().enable();
                    let result = if read_only && cancellation.epoch.load(Ordering::Acquire) != epoch
                    {
                        Err("request_interrupted".into())
                    } else {
                        tokio::select! {
                            _=&mut interrupted,if read_only=>Err("request_interrupted".into()),
                            result=tokio::time::timeout(Duration::from_secs(35), runtime.execute(envelope.request,start_ticket))=>result.unwrap_or_else(|_|Err("background_timeout".into())),
                        }
                    };
                    runtime.status();
                    match result {
                        Ok(data) => Reply::success(data),
                        Err(error) => {
                            if runtime.history.len() == 32 {
                                runtime.history.pop_front();
                            }
                            runtime.history.push_back(
                                json!({"operation":event,"failure":classify_error(&error)}),
                            );
                            Reply::failure(error)
                        }
                    }
                }
            }
        }
        Ok(_) => Reply::failure("ipc_incompatible_version"),
        Err(error) => Reply::failure(error),
    };
    let mut body = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE_BYTES {
        body = serde_json::to_vec(&Reply::failure("ipc_message_too_large"))
            .map_err(|e| e.to_string())?;
    }
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        stream.write_u32(body.len() as u32).await?;
        stream.write_all(&body).await?;
        stream.flush().await
    })
    .await;
    // Exiting is honored even if a UI crashes before reading the reply.
    if is_shutdown && reply.ok {
        let _ = shutdown.send(()).await;
    }
    result
        .map_err(|_| "IPC write timeout")?
        .map_err(|e| e.to_string())
}

fn validate_settings_state(bytes: &[u8]) -> Result<()> {
    neonmix_control::Authority::restore(
        serde_json::from_slice(bytes).map_err(|_| "credential_corrupt")?,
    )
    .map(|_| ())
    .map_err(|_| "credential_corrupt".into())
}
fn transaction_error(error: neonmix_identity::hub_settings::Error) -> String {
    use neonmix_identity::hub_settings::Outcome;
    match error.outcome {
        Outcome::NotCommitted => error.code,
        Outcome::RecoveryRequired => "configuration_recovery_required",
        Outcome::DurabilityUnconfirmed => "profile_durability_unconfirmed",
    }
    .into()
}
fn profile_error(error: impl std::fmt::Display) -> String {
    Fault::from_machine_code(&error.to_string())
        .map_or(FaultCode::CredentialCorrupt, |fault| fault.code)
        .as_str()
        .into()
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn diagnostic_meter_session_scope_survives_ipc_but_never_the_export_whitelist() {
        let session = Uuid::new_v4();
        let mut data = json!({"session_id":session,"device_id":Uuid::new_v4(),"meters":{"lanes":[
            {"stream_id":9,"session_id":session,"stream_epoch":1,"peak":0.1,"rms":0.05},
            {"stream_id":10,"session_id":null}, {"stream_id":11,"session_id":"private-user-name"}
        ]}});
        redact(&mut data, true);
        assert_eq!(data["session_id"], "[redacted]");
        assert_eq!(
            data["meters"]["lanes"][0]["session_id"],
            session.to_string()
        );
        assert!(data["meters"]["lanes"][1]["session_id"].is_null());
        assert_eq!(data["meters"]["lanes"][2]["session_id"], "[redacted]");
        let safe = export_whitelist(&data);
        assert!(safe["meters"]["lanes"][0].get("session_id").is_none());
        assert!(!safe.to_string().contains(&session.to_string()));
    }
    #[tokio::test]
    async fn persistent_fault_keeps_semantics_through_eof_and_unexpected_exit_noise() {
        let status = Arc::new(StdMutex::new(ProcessStatus::default()));
        drain(
            &b"{\"fault\":{\"code\":\"hub_port_in_use\",\"params\":{\"port\":9000}}}"[..],
            status.clone(),
            false,
            None,
        )
        .await;
        drain(
            &b"SECRET_TOKEN arbitrary failure"[..],
            status.clone(),
            true,
            None,
        )
        .await;
        let state = status.lock().unwrap();
        let fault = state.fault.as_ref().unwrap();
        assert_eq!(fault.code, FaultCode::HubPortInUse);
        assert_eq!(fault.params.port, Some(9000));
        assert!(
            !serde_json::to_string(&*state)
                .unwrap()
                .contains("SECRET_TOKEN")
        );
    }
    #[test]
    fn sender_target_is_latched_before_redaction_and_stats_cannot_change_it() {
        let hub_id = Uuid::new_v4();
        let device_id = Uuid::new_v4();
        let status = Arc::new(StdMutex::new(ProcessStatus {
            running: true,
            ..Default::default()
        }));
        let started = json!({"event":"sender_started","sender_target":{"hub_id":hub_id,"device_id":device_id,"room_name":"目标 A"}});
        record_process_line(
            &serde_json::to_vec(&started).unwrap(),
            &status,
            false,
            Some("sender_started"),
        );
        let stats = json!({"event":"sender_stats","sender_target":{"hub_id":Uuid::new_v4(),"room_name":"错误目标"},"capture_stats":{"frames":480,"errors":0}});
        record_process_line(
            &serde_json::to_vec(&stats).unwrap(),
            &status,
            false,
            Some("sender_started"),
        );
        let process = status.lock().unwrap();
        assert_eq!(process.sender_target.as_ref().unwrap().hub_id, hub_id);
        assert_eq!(
            process.sender_target.as_ref().unwrap().device_id,
            Some(device_id)
        );
        let view = process.observed_snapshot();
        assert!(view.metrics_age_ms.is_some());
        let value = json!({"local":{"sender":view},"remote":{"available":false,"sample_age_ms":123},"address":"secret-host","credential":"secret-path","new_unapproved_metric":999});
        let safe = export_whitelist(&value);
        assert_eq!(safe["remote"]["available"], false);
        assert_eq!(safe["remote"]["sample_age_ms"], 123);
        assert!(safe["local"]["sender"]["metrics_age_ms"].is_number());
        assert!(safe["local"]["sender"].get("sender_target").is_none());
        assert!(safe.get("new_unapproved_metric").is_none());
        for private in [
            "目标 A",
            "错误目标",
            "secret-host",
            "secret-path",
            &hub_id.to_string(),
            &device_id.to_string(),
        ] {
            assert!(!safe.to_string().contains(private));
        }
    }
    #[test]
    fn capture_permission_is_not_room_permission() {
        for message in [
            "audio capture permission denied: macOS Microphone access is not yet granted.",
            "audio capture permission denied: macOS Microphone access is denied.",
        ] {
            assert_eq!(classify_error(message), "capture_permission_denied");
        }
        assert_eq!(classify_error("permission_denied"), "permission_denied");
        assert_eq!(
            classify_error("credential_permission_denied"),
            "credential_permission_denied"
        );
    }

    #[test]
    fn unsupported_device_format_has_a_specific_redacted_error() {
        let error =
            classify_error("unsupported audio format: sample rate 384000; private_device_name");
        assert_eq!(error, "unsupported_audio_format");
        assert!(!error.contains("private_device_name"));
        assert!(!error.contains("384000"));
    }
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
