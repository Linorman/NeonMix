mod animation;
mod icons;
mod pages;
mod palette;
mod shell;
#[cfg(feature = "screenshot")]
mod shot;
mod theme;
mod tray;
mod widgets;
use clap::Parser;
use eframe::egui::{self, RichText};
use neonmix_control::{Operation, Role, SessionStatus, Snapshot};
use neonmix_core::DeviceInfo;
use neonmix_desktop_service::{
    Client, HubSettings, OutputAction, Request, SenderOptions, ServiceStatus,
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant},
};

/// Version shown by `--version` and the 关于 page: package version plus the
/// commit it was built from.
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("NEONMIX_GIT_HASH"),
    ")"
);

#[derive(Parser)]
#[command(version = VERSION)]
struct Args {
    #[arg(long, default_value = ".local/desktop")]
    state_dir: PathBuf,
    #[arg(long, default_value_t = 1100.0)]
    width: f32,
    #[arg(long, default_value_t = 760.0)]
    height: f32,
    /// Load recorded public state for preview verification; requires preview-page.
    #[arg(long, requires = "preview_page")]
    preview_data: Option<PathBuf>,
    /// Open a page without connecting to a background (visual verification only).
    #[arg(long, value_parser = ["hub", "sender", "mixer", "devices", "diagnostics", "about", "airplay"])]
    preview_page: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Hash, Debug)]
enum Page {
    Hub,
    Sender,
    Mixer,
    Devices,
    Diagnostics,
    About,
}
impl Page {
    fn icon(self) -> icons::Icon {
        match self {
            Self::Hub => icons::Icon::Room,
            Self::Sender => icons::Icon::Sender,
            Self::Mixer => icons::Icon::Mixer,
            Self::Devices => icons::Icon::Devices,
            Self::Diagnostics => icons::Icon::Pulse,
            Self::About => icons::Icon::Info,
        }
    }
    const ALL: [(Self, &'static str); 6] = [
        (Self::Hub, "Hub 设置"),
        (Self::Sender, "Sender"),
        (Self::Mixer, "Mixer"),
        (Self::Devices, "设备管理"),
        (Self::Diagnostics, "诊断"),
        (Self::About, "关于"),
    ];
    fn title(self) -> &'static str {
        Self::ALL.iter().find(|p| p.0 == self).unwrap().1
    }
}
struct PollData {
    credential: PathBuf,
    hub: Option<String>,
    room_name: Option<String>,
    status: ServiceStatus,
    snapshot: Option<Snapshot>,
    diagnostics: Option<Value>,
    airplay: Option<Value>,
    error: Option<String>,
}
enum Work {
    Boot,
    Poll {
        credential: PathBuf,
        hub: Option<String>,
    },
    Action(Request),
}
enum Outcome {
    Boot,
    Poll(Box<PollData>),
    Action(Request, Value),
}
fn call(client: &Client, request: &Request) -> Result<Value, String> {
    let reply = client.request(request)?;
    if reply.ok {
        Ok(reply.data)
    } else {
        Err(reply.error.unwrap_or_else(|| "后台操作失败".into()))
    }
}
fn worker(
    client: Client,
    ctx: egui::Context,
) -> (SyncSender<Work>, Receiver<Result<Outcome, String>>) {
    let (tx, jobs) = mpsc::sync_channel(1);
    let (done, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        while let Ok(job) = jobs.recv() {
            let result = match job {
                Work::Boot => client.ensure_background().map(|()| Outcome::Boot),
                Work::Action(request) => {
                    call(&client, &request).map(|data| Outcome::Action(request, data))
                }
                Work::Poll { credential, hub } => (|| {
                    let status =
                        serde_json::from_value::<ServiceStatus>(call(&client, &Request::Status)?)
                            .map_err(|e| e.to_string())?;
                    let mut data = PollData {
                        credential: credential.clone(),
                        hub: hub.clone(),
                        room_name: None,
                        status,
                        snapshot: None,
                        diagnostics: None,
                        airplay: None,
                        error: None,
                    };
                    if data
                        .status
                        .profiles
                        .iter()
                        .any(|p| p.credential == credential && !p.pending)
                    {
                        match call(
                            &client,
                            &Request::Snapshot {
                                credential: credential.clone(),
                                hub: hub.clone(),
                            },
                        ) {
                            Ok(value) => {
                                data.room_name = value
                                    .pointer("/viewer/room_name")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned);
                                match serde_json::from_value(value) {
                                    Ok(s) => data.snapshot = Some(s),
                                    Err(e) => data.error = Some(e.to_string()),
                                }
                            }
                            Err(e) => data.error = Some(e),
                        }
                        if data.snapshot.is_some() {
                            data.airplay = call(
                                &client,
                                &Request::AirplayV2 {
                                    credential: credential.clone(),
                                    hub: hub.clone(),
                                    command: None,
                                },
                            )
                            .ok();
                            match call(
                                &client,
                                &Request::Diagnostics {
                                    credential: credential.clone(),
                                    hub: hub.clone(),
                                },
                            ) {
                                Ok(v) => data.diagnostics = Some(v),
                                Err(e) => data.error = Some(e),
                            }
                        }
                    }
                    Ok(Outcome::Poll(Box::new(data)))
                })(),
            };
            if done.send(result).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, rx)
}
/// A write the user made while the previous one was still in flight. Only
/// the latest intent is kept and it is sent against the next fresh revision,
/// so rapid input neither conflicts nor greys out the controls.
#[derive(Clone)]
enum Write {
    Control(Operation),
    Airplay(neonmix_airplay_adapter::control::AirplayActionV2),
}
/// The last mixer change, offered as 还原 (restore) for a few seconds. Named
/// apart from 撤销 (revoke a pairing) so the two can never be confused.
struct Undo {
    label: String,
    inverse: Write,
    at: Instant,
}
struct Confirmation {
    label: String,
    consequence: String,
    request: Request,
    focus: bool,
    _origin: egui::Id,
}
impl Confirmation {
    fn action_label(&self) -> &str {
        match &self.request {
            Request::Shutdown => "退出后台",
            Request::Control {
                operation: Operation::Revoke { .. },
                ..
            } => "撤销配对",
            Request::ForgetCredential { .. } => "删除本机配对",
            Request::Output {
                action: OutputAction::Remove { .. },
                ..
            } => "删除输出绑定",
            _ => &self.label,
        }
    }
}
struct Desktop {
    page: Page,
    page_since: Instant,
    shown_message: String,
    message_since: Instant,
    client: Client,
    worker: Option<SyncSender<Work>>,
    results: Option<Receiver<Result<Outcome, String>>>,
    busy: bool,
    inflight_start: bool,
    inflight_stop: bool,
    stop_after_start: bool,
    polling: bool,
    pending_action: Option<Request>,
    pending_stop: bool,
    urgent_action: Option<Receiver<Result<Value, String>>>,
    urgent_is_shutdown: bool,
    repaint: egui::Context,
    online: bool,
    message: String,
    error: bool,
    cjk: bool,
    status: Option<ServiceStatus>,
    snapshot: Option<Snapshot>,
    diagnostics: Option<Value>,
    airplay: Option<Value>,
    fresh: Option<Instant>,
    /// Last authoritative snapshot; unlike `fresh` our own writes do not clear
    /// it, so controls stay enabled (and do not flash) while a write settles.
    synced: Option<Instant>,
    queued_write: Option<Write>,
    /// Which action is in flight, so only its button shows progress.
    inflight: Option<&'static str>,
    poll_error: bool,
    copied_at: Option<Instant>,
    last_poll: Instant,
    devices: Vec<DeviceInfo>,
    room: String,
    remote_room: Option<String>,
    output: String,
    sender_name: String,
    invitation: String,
    show_invitation: bool,
    show_airplay_pin: bool,
    airplay_name_drafts: std::collections::BTreeMap<String, String>,
    issued_invitation: String,
    invite_id: Option<uuid::Uuid>,
    invite_expiry: Option<u64>,
    credential: PathBuf,
    hub_address: String,
    candidates: Vec<Value>,
    binding: Option<Value>,
    binding_name: String,
    provider: String,
    search: String,
    confirm: Option<Confirmation>,
    focus_after_modal: bool,
    exiting: bool,
    preview: bool,
    preview_airplay_only: bool,
    gain_drafts: std::collections::BTreeMap<u64, f32>,
    /// Mixer lane under keyboard control, keyed by the actual stream id.
    selected_lane: Option<u64>,
    lane_details: std::collections::BTreeSet<u64>,
    /// User choices for collapsible panels; absent keys follow data defaults.
    panels: std::collections::HashMap<&'static str, bool>,
    device_filter: usize,
    /// Panel to bring into view on the next frame (stepper / tile clicks).
    scroll_to: Option<&'static str>,
    palette: Option<palette::Palette>,
    undo: Option<Undo>,
    tray: Option<tray::Tray>,
    tray_attempted: bool,
    #[cfg(feature = "screenshot")]
    shot: Option<shot::Shot>,
}
impl Desktop {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let cjk = theme::install(&cc.egui_ctx);
        let mut app = Self::empty(
            Client::new(args.state_dir),
            cjk,
            args.preview_page.is_some(),
        );
        app.repaint = cc.egui_ctx.clone();
        if let Some(page) = args.preview_page {
            app.preview_airplay_only = page == "airplay";
            app.page = match page.as_str() {
                "sender" => Page::Sender,
                "mixer" => Page::Mixer,
                "devices" => Page::Devices,
                "diagnostics" => Page::Diagnostics,
                "about" => Page::About,
                _ => Page::Hub,
            };
        } else {
            let (tx, rx) = worker(app.client.clone(), cc.egui_ctx.clone());
            app.worker = Some(tx);
            app.results = Some(rx);
            app.queue(Work::Boot);
        }
        #[cfg(feature = "screenshot")]
        if app.preview {
            app.shot = shot::Shot::from_env();
            if let Ok(key) = std::env::var("NEONMIX_SCREENSHOT_BUSY") {
                app.inflight = ["discover", "hub-start", "pair", "invite", "sender-start"]
                    .into_iter()
                    .find(|k| *k == key);
            }
            if std::env::var_os("NEONMIX_SCREENSHOT_PALETTE").is_some() {
                app.palette = Some(Default::default());
            }
            if std::env::var_os("NEONMIX_SCREENSHOT_UNDO").is_some() {
                app.selected_lane = app
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.streams.keys().next().copied());
                app.undo = Some(Undo {
                    label: "「E07 合成 B」音量 0.0 → +3.5 dB".into(),
                    inverse: Write::Control(Operation::OutputMix {
                        gain_db: None,
                        muted: None,
                    }),
                    at: Instant::now() + Duration::from_secs(30),
                });
            }
            if std::env::var_os("NEONMIX_SCREENSHOT_CONFIRM").is_some() {
                app.confirmation(
                    "退出后台".into(),
                    "停止本实例的共享、发送和全部音频连接。关闭窗口不会停止音频；退出后台会。"
                        .into(),
                    Request::Shutdown,
                    egui::Id::new("preview-confirm"),
                );
            }
        }
        if app.preview {
            app.message = "视觉验证模式".into();
            if let Some(path) = args.preview_data {
                match std::fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                {
                    Some(data) => {
                        app.devices =
                            serde_json::from_value(data["devices"].clone()).unwrap_or_default();
                        app.snapshot = serde_json::from_value(data["snapshot"].clone()).ok();
                        app.status = serde_json::from_value(data["status"].clone()).ok();
                        if let Some(settings) =
                            app.status.as_ref().and_then(|s| s.hub_settings.as_ref())
                        {
                            app.room = settings.name.clone();
                            app.output = settings.output.clone();
                        }
                        app.remote_room = data["snapshot"]
                            .pointer("/viewer/room_name")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        app.diagnostics = data.get("diagnostics").cloned();
                        app.airplay = data.get("airplay").cloned();
                        #[cfg(feature = "screenshot")]
                        {
                            app.airplay_name_drafts =
                                serde_json::from_value(data["preview_name_drafts"].clone())
                                    .unwrap_or_default();
                            if let Some(error) = data["preview_error"].as_str() {
                                app.error = true;
                                app.message = error.into();
                            }
                        }
                        app.binding = app.status.as_ref().and_then(|s| s.output_binding.clone());
                        app.fresh = Some(Instant::now());
                        app.synced = app.fresh;
                    }
                    None => {
                        app.error = true;
                        app.message = "无法读取预览记录".into();
                    }
                }
            }
        }
        app
    }
    fn empty(client: Client, cjk: bool, preview: bool) -> Self {
        Self {
            page: Page::Hub,
            page_since: Instant::now() - Duration::from_secs(5),
            shown_message: String::new(),
            message_since: Instant::now() - Duration::from_secs(5),
            client,
            worker: None,
            results: None,
            busy: false,
            inflight_start: false,
            inflight_stop: false,
            stop_after_start: false,
            polling: false,
            pending_action: None,
            pending_stop: false,
            urgent_action: None,
            urgent_is_shutdown: false,
            repaint: egui::Context::default(),
            online: false,
            message: "正在连接后台…".into(),
            error: false,
            cjk,
            status: None,
            snapshot: None,
            diagnostics: None,
            airplay: None,
            fresh: None,
            synced: None,
            queued_write: None,
            inflight: None,
            poll_error: false,
            copied_at: None,
            last_poll: Instant::now() - Duration::from_secs(5),
            devices: vec![],
            room: "客厅".into(),
            remote_room: None,
            output: String::new(),
            sender_name: "我的电脑".into(),
            invitation: String::new(),
            show_invitation: false,
            show_airplay_pin: false,
            airplay_name_drafts: Default::default(),
            issued_invitation: String::new(),
            invite_id: None,
            invite_expiry: None,
            credential: PathBuf::from("hub/admin.json"),
            hub_address: String::new(),
            candidates: vec![],
            binding: None,
            binding_name: "NeonMix — 客厅".into(),
            provider: "neonmix".into(),
            search: String::new(),
            confirm: None,
            focus_after_modal: false,
            exiting: false,
            preview,
            preview_airplay_only: false,
            gain_drafts: std::collections::BTreeMap::new(),
            selected_lane: None,
            lane_details: Default::default(),
            panels: Default::default(),
            device_filter: 0,
            scroll_to: None,
            palette: None,
            undo: None,
            tray: None,
            tray_attempted: false,
            #[cfg(feature = "screenshot")]
            shot: None,
        }
    }
    fn queue(&mut self, work: Work) {
        if self.busy || self.preview {
            return;
        }
        let polling = matches!(&work, Work::Poll { .. });
        let starting = matches!(&work, Work::Action(Request::SenderStart { .. }));
        let stopping = matches!(&work, Work::Action(Request::SenderStop));
        let key = match &work {
            Work::Action(request) => Some(request_key(request)),
            _ => None,
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|tx| tx.try_send(work).is_ok())
        {
            self.busy = true;
            self.polling = polling;
            self.inflight_start = starting;
            self.inflight_stop = stopping;
            self.inflight = key;
        }
    }
    /// The action a button represents is queued or running.
    fn pending(&self, key: &str) -> bool {
        self.inflight == Some(key)
            || self
                .pending_action
                .as_ref()
                .is_some_and(|r| request_key(r) == key)
    }
    fn action_busy(&self) -> bool {
        (self.busy && !self.polling) || self.pending_action.is_some() || self.pending_stop
    }
    fn request(&mut self, request: Request) {
        if self.busy && matches!(&request, Request::Shutdown) {
            self.begin_urgent(request);
            return;
        }
        if self.preview {
            return;
        }
        if self.busy {
            if !self.polling || self.pending_action.is_some() {
                return;
            }
            self.pending_action = Some(request);
        } else {
            self.queue(Work::Action(request));
        }
        self.message = "正在处理…".into();
        self.error = false;
    }
    fn stop_sender(&mut self) {
        if self.preview || self.pending_stop {
            return;
        }
        if self.inflight_start {
            self.pending_stop = true;
            self.stop_after_start = true;
            self.message = "开始操作结束后立即停止发送…".into();
            return;
        }
        if matches!(self.pending_action, Some(Request::SenderStart { .. })) {
            self.pending_action = None;
        }
        if self.busy {
            self.begin_urgent(Request::SenderStop);
        } else {
            self.request(Request::SenderStop);
        }
    }
    fn begin_urgent(&mut self, request: Request) {
        if self.preview || self.pending_stop {
            return;
        }
        self.pending_stop = true;
        self.urgent_is_shutdown = matches!(&request, Request::Shutdown);
        self.message = if self.urgent_is_shutdown {
            "正在退出后台…"
        } else {
            "正在停止发送…"
        }
        .into();
        let client = self.client.clone();
        let repaint = self.repaint.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.urgent_action = Some(rx);
        std::thread::spawn(move || {
            let result = call(&client, &request);
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }
    fn manual_hub(&self) -> Option<String> {
        (!self.hub_address.trim().is_empty()).then(|| self.hub_address.trim().to_string())
    }
    fn hub(&self) -> Option<String> {
        self.manual_hub().or_else(|| {
            (self.credential.as_path() == std::path::Path::new("hub/admin.json")
                && self
                    .status
                    .as_ref()
                    .is_some_and(|s| s.hub_settings.is_some()))
            .then(|| "https://localhost:7443".into())
        })
    }
    fn poll(&mut self) {
        self.last_poll = Instant::now();
        self.queue(Work::Poll {
            credential: self.credential.clone(),
            hub: self.hub(),
        });
    }
    fn process(&mut self) {
        if let Some(result) = self
            .urgent_action
            .as_ref()
            .and_then(|rx| rx.try_recv().ok())
        {
            self.urgent_action = None;
            self.pending_stop = false;
            match result {
                Ok(_) => {
                    if self.urgent_is_shutdown {
                        self.exiting = true;
                    }
                    self.urgent_is_shutdown = false;
                    self.message = "发送已停止".into();
                    self.error = false;
                    if let Some(status) = &mut self.status {
                        status.sender.running = false;
                        status.sender.pid = None;
                    }
                }
                Err(error) => {
                    self.message = user_error(error);
                    self.error = true;
                }
            }
        }
        let outcome = self.results.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(outcome) = outcome {
            let completed_start = self.inflight_start;
            let completed_stop = self.inflight_stop;
            let was_poll = self.polling;
            self.inflight = None;
            self.inflight_start = false;
            self.inflight_stop = false;
            if completed_stop {
                self.pending_stop = false;
            }
            self.busy = false;
            self.polling = false;
            self.last_poll = Instant::now();
            match outcome {
                Err(e) => {
                    self.message = user_error(e);
                    self.error = true;
                    self.fresh = None;
                    // A failed poll keeps the last readings on screen (marked
                    // stale by age) instead of blanking meters and cards.
                    if was_poll {
                        self.poll_error = true;
                    } else {
                        self.gain_drafts.clear();
                        self.queued_write = None;
                    }
                }
                Ok(Outcome::Boot) => {
                    self.online = true;
                    self.message = "后台已连接".into();
                    self.request(Request::Devices);
                }
                Ok(Outcome::Poll(data)) => {
                    self.online = true;
                    if self.status.is_none()
                        && let Some(settings) = &data.status.hub_settings
                    {
                        self.room = settings.name.clone();
                        self.output = settings.output.clone();
                    }
                    if !data
                        .status
                        .profiles
                        .iter()
                        .any(|p| p.credential == self.credential)
                        && let Some(profile) = data.status.profiles.iter().find(|p| !p.pending)
                    {
                        self.credential = profile.credential.clone();
                    }
                    if self.status.is_none()
                        && let Some(name) = data
                            .status
                            .output_binding
                            .as_ref()
                            .and_then(|b| b["display_name"].as_str())
                    {
                        self.binding_name = name.into();
                    }
                    if data.credential == self.credential && data.hub == self.hub() {
                        self.remote_room = data.room_name;
                    }
                    self.binding = data.status.output_binding.clone();
                    self.status = Some(data.status);
                    if let Some(snapshot) = data
                        .snapshot
                        .filter(|_| data.credential == self.credential && data.hub == self.hub())
                    {
                        self.snapshot = Some(snapshot);
                        self.airplay = data.airplay;
                        self.fresh = Some(Instant::now());
                        self.synced = self.fresh;
                        if self.poll_error && data.error.is_none() {
                            self.poll_error = false;
                            self.error = false;
                            self.message = "已重新取得房间状态".into();
                        }
                        self.diagnostics = data
                            .diagnostics
                            .map(|v| v.get("remote").cloned().unwrap_or(v));
                    } else {
                        self.fresh = None;
                        self.synced = None;
                        self.airplay = None;
                        self.diagnostics = None;
                    }
                    if let Some(e) = data.error {
                        self.message = user_error(e);
                        self.error = true;
                        self.fresh = None;
                    }
                }
                Ok(Outcome::Action(request, data)) => {
                    // Mixer and AirPlay writes show their result in the control
                    // itself; a status line per click is noise.
                    if !matches!(request, Request::Control { .. } | Request::AirplayV2 { .. }) {
                        self.message = "操作已完成".into();
                    }
                    self.error = false;
                    self.last_poll = Instant::now() - Duration::from_secs(5);
                    match request {
                        Request::AirplayV2 { command, .. } => {
                            if let Some(command) = command {
                                self.airplay_name_saved(&command.operation);
                            }
                            match data["outcome"].as_str() {
                                Some("configuration_partial") => {
                                    self.message =
                                        "部分入口身份已保存，但配置尚未完成；请检查入口后重试。"
                                            .into();
                                    self.error = true;
                                }
                                Some("saved_session_ended") => {
                                    self.message =
                                        "设备设置已保存；原会话已结束，未操作新的连接。".into()
                                }
                                _ => {}
                            }
                            if let Some(warning) = data["warning"].as_str() {
                                self.message = if warning == "profile_durability_unconfirmed" {
                                    "配置已写入，但磁盘同步未确认；请检查存储状态。"
                                } else {
                                    "设置已保存，但入口未完成更新；请检查诊断和入口状态。"
                                }
                                .into();
                                self.error = true;
                            } else if data["media_pending"].as_bool() == Some(true) {
                                self.message = "设置已保存，正在同步到音频引擎。".into();
                            }
                            self.airplay = Some(data);
                        }
                        Request::Devices => match serde_json::from_value(data) {
                            Ok(devices) => self.devices = devices,
                            Err(e) => {
                                self.error = true;
                                self.message = format!("设备列表格式错误：{e}");
                            }
                        },
                        Request::Discover { .. } => {
                            self.candidates = data
                                .get("candidates")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default();
                        }
                        Request::Invite { .. } => {
                            self.issued_invitation = data
                                .get("invitation")
                                .map(|v| {
                                    if let Some(t) = v.as_str() {
                                        t.to_owned()
                                    } else {
                                        v.to_string()
                                    }
                                })
                                .unwrap_or_default();
                            self.invite_id = data
                                .get("invitation_id")
                                .and_then(Value::as_str)
                                .and_then(|s| s.parse().ok());
                            self.invite_expiry =
                                data.get("expires_unix_seconds").and_then(Value::as_u64);
                        }
                        Request::CancelInvite { .. } => {
                            self.issued_invitation.clear();
                            self.invite_id = None;
                        }
                        Request::Output {
                            action: OutputAction::Remove { .. },
                            ..
                        } => self.binding = None,
                        Request::Output { .. } => self.binding = Some(data),
                        Request::Shutdown => self.exiting = true,
                        Request::ExportDiagnostics { .. } => {
                            self.message = "脱敏诊断已导出到 diagnostics-redacted.json".into()
                        }
                        Request::ForgetCredential { .. } => {
                            self.snapshot = None;
                            self.fresh = None;
                            self.synced = None;
                            self.invitation.clear();
                            self.message =
                                "本机配对已删除，输出已禁用；重新配对后手动启用或重新添加输出。"
                                    .into();
                        }
                        Request::PairText { .. } => {
                            self.credential = PathBuf::from("profiles/sender.json");
                            self.invitation.clear();
                            self.snapshot = None;
                            self.synced = None;
                        }
                        _ => {}
                    }
                }
            }
            if completed_start && self.stop_after_start {
                self.stop_after_start = false;
                self.pending_stop = false;
                self.request(Request::SenderStop);
                self.pending_stop = true;
            }
        }
        if !self.busy
            && !self.pending_stop
            && let Some(request) = self.pending_action.take()
        {
            self.request(request);
        }
        if self.ready()
            && let Some(write) = self.queued_write.take()
        {
            self.dispatch(write);
        }
        if self.online
            && !self.busy
            && !self.pending_stop
            && self.last_poll.elapsed() > self.poll_interval()
        {
            self.poll();
        }
    }
    /// Meters need a faster cadence than settings; other pages poll at 1 Hz.
    fn poll_interval(&self) -> Duration {
        if self.page == Page::Mixer {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        }
    }
    fn ready(&self) -> bool {
        self.fresh
            .is_some_and(|t| t.elapsed() < Duration::from_secs(4))
            && !self.action_busy()
    }
    fn role(&self) -> Option<Role> {
        let p = self
            .status
            .as_ref()?
            .profiles
            .iter()
            .find(|p| p.credential == self.credential)?;
        let id = p.device_id?;
        self.snapshot
            .as_ref()?
            .devices
            .get(&id)
            .filter(|d| !d.revoked)
            .map(|d| d.role)
    }
    /// Controls are interactive while the last authoritative state is recent.
    /// Writes themselves are gated by `ready()` and queued meanwhile.
    fn writable(&self) -> bool {
        self.synced
            .is_some_and(|t| t.elapsed() < Duration::from_secs(4))
            && !self.pending_stop
    }
    fn sender_allowed(&self) -> bool {
        if !self.writable() || self.credential.as_path() == std::path::Path::new("hub/admin.json") {
            return false;
        }
        let Some(id) = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .and_then(|p| p.device_id)
        else {
            return false;
        };
        self.snapshot
            .as_ref()
            .and_then(|s| s.devices.get(&id))
            .is_some_and(|d| !d.revoked && d.playback_allowed)
    }
    fn controls_room(&self) -> bool {
        matches!(self.role(), Some(Role::Admin | Role::Controller))
    }
    fn admin(&self) -> bool {
        self.role() == Some(Role::Admin)
    }
    fn operation(&mut self, operation: Operation) {
        self.write(Write::Control(operation));
    }
    fn write(&mut self, write: Write) {
        if self.ready() {
            self.dispatch(write);
        } else if self.writable() {
            self.queued_write = Some(write);
        }
    }
    fn dispatch(&mut self, write: Write) {
        let request = match write {
            Write::Control(operation) => self.snapshot.as_ref().map(|state| Request::Control {
                credential: self.credential.clone(),
                hub: self.hub(),
                expected_revision: state.revision,
                operation,
            }),
            Write::Airplay(operation) => self.airplay_request(operation),
        };
        if let Some(request) = request {
            self.request(request);
            // Our own write makes the snapshot stale until the next poll.
            self.fresh = None;
        }
    }
    fn panel_open(&self, key: &'static str, default: bool) -> bool {
        self.panels.get(key).copied().unwrap_or(default)
    }
    fn toggle_panel(&mut self, key: &'static str, open: bool) {
        self.panels.insert(key, !open);
    }
    /// A mixer change the user can restore with one click or ⌘Z.
    fn mix_change(&mut self, label: String, write: Write, inverse: Write) {
        if !self.writable() {
            return;
        }
        self.write(write);
        self.undo = Some(Undo {
            label,
            inverse,
            at: Instant::now(),
        });
    }
    fn undo_available(&self) -> bool {
        self.undo
            .as_ref()
            .is_some_and(|u| u.at.elapsed() < Duration::from_secs(8))
    }
    fn restore_last(&mut self) {
        if !self.undo_available() {
            return;
        }
        if let Some(undo) = self.undo.take() {
            self.write(undo.inverse);
            self.message = format!("已还原：{}", undo.label);
            self.error = false;
        }
    }
    fn confirmation(
        &mut self,
        label: String,
        consequence: String,
        request: Request,
        origin: egui::Id,
    ) {
        self.confirm = Some(Confirmation {
            label,
            consequence,
            request,
            focus: true,
            _origin: origin,
        });
    }
}
fn request_key(request: &Request) -> &'static str {
    match request {
        Request::HubSetup { .. } | Request::HubSettings { .. } => "hub-settings",
        Request::HubStart => "hub-start",
        Request::HubStop => "hub-stop",
        Request::TestTone { .. } => "test-tone",
        Request::Discover { .. } => "discover",
        Request::PairText { .. } => "pair",
        Request::Invite { .. } => "invite",
        Request::SenderStart { .. } => "sender-start",
        Request::Output { .. } => "output",
        Request::ExportDiagnostics { .. } => "export",
        Request::Devices => "devices",
        _ => "other",
    }
}
fn user_error(error: String) -> String {
    match error.as_str() {
        "revision_conflict" | "stale_revision" => {
            "房间状态已被其他操作更新；正在刷新，请核对后重试。".into()
        }
        "permission_denied" => "当前身份无权执行此操作；请刷新设备权限。".into(),
        "unauthenticated" => "配对凭证不可用；检查是否已被撤销，必要时重新配对。".into(),
        "invalid_argument" => "参数无效；请检查字段和所选设备。".into(),
        "not_found" => "目标设备或通道已不存在；请刷新状态。".into(),
        "quota_exceeded" | "room_capacity_full" => "房间已达到设备或通道上限。".into(),
        "receiver_busy" => "这个入口已被占用；请选择空闲入口。".into(),
        "source_already_active" => "此来源已在房间播放；先断开旧连接再更换入口。".into(),
        "source_blocked" => "此来源已被禁止播放，请由管理员重新允许。".into(),
        "pairing_revoked" => "此来源的配对已撤销，需要管理员重新开放配对。".into(),
        "session_changed" => "此来源的会话已变化；正在刷新，请核对后重试。".into(),
        "worker_unavailable" => "此入口的接收引擎不可用，请检查诊断后重新开启。".into(),
        "output_unavailable" => "实体输出不可用，请检查声卡连接。".into(),
        "upgrade_required" => "此房间使用多路 AirPlay，请更新桌面客户端。".into(),
        _ => error,
    }
}
fn virtual_device(device: &DeviceInfo) -> bool {
    device.id.contains("com.neonmix.")
        || device.id.to_ascii_lowercase().contains("blackhole")
        || device.id.contains("neonmix.sink")
        || device.id.contains("NEONMIX")
}
fn status_text(status: Option<SessionStatus>) -> &'static str {
    match status {
        None => "无输入",
        Some(SessionStatus::Buffering) => "缓冲中",
        Some(SessionStatus::Playing) => "播放中",
        Some(SessionStatus::NetworkDegraded) => "网络降级",
        Some(SessionStatus::UserStopped) => "用户已停止",
        Some(SessionStatus::AdminDisconnected) => "管理员已断开",
        Some(SessionStatus::NetworkInterrupted) => "连接中断 · 需重新发送",
        Some(SessionStatus::Revoked) => "配对已撤销",
        Some(SessionStatus::OutputLost) => "输出丢失",
    }
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.tray_attempted && !self.preview {
            self.tray_attempted = true;
            match tray::Tray::new(ctx.clone()) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => {
                    self.message = format!("托盘不可用：{e}；关闭窗口会最小化，后台继续运行。");
                    self.error = true;
                }
            }
        }
        if let Some(tray) = &self.tray {
            for action in tray.actions() {
                match action {
                    tray::Action::Show => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    tray::Action::Hide => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }
                    tray::Action::Stop => self.stop_sender(),
                    tray::Action::Quit => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        self.confirmation(
                            "退出后台".into(),
                            "停止本实例的共享、发送和全部音频连接。".into(),
                            Request::Shutdown,
                            ctx.memory(|m| m.focused())
                                .unwrap_or_else(|| egui::Id::new("tray-quit")),
                        );
                    }
                }
            }
        }
        if self.exiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.preview {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.tray.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        self.show(ctx);
        #[cfg(feature = "screenshot")]
        if let Some(shot) = &mut self.shot {
            shot.update(ctx);
        }
    }
}
fn main() -> eframe::Result {
    let args = Args::parse();
    let size = [args.width.max(600.0), args.height.max(440.0)];
    eframe::run_native(
        "NeonMix",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(size)
                .with_min_inner_size([600.0, 440.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(Desktop::new(cc, args)))),
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn themed() -> egui::Context {
        let ctx = egui::Context::default();
        theme::install_style(&ctx);
        theme::fallback_fonts(&ctx);
        ctx
    }

    #[test]
    fn named_fields_publish_accessible_labels_without_exposing_password_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for secret in [false, true] {
            let mut value = "private-test-value".to_owned();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    widgets::field(ui, "受测字段", &mut value, secret);
                });
            });
            let tree = output.platform_output.accesskit_update.unwrap();
            let field = tree
                .nodes
                .iter()
                .map(|(_, node)| node)
                .find(|node| {
                    matches!(
                        node.role(),
                        egui::accesskit::Role::TextInput | egui::accesskit::Role::PasswordInput
                    )
                })
                .expect("field absent from accessibility tree");
            assert!(!field.labelled_by().is_empty());
            assert!(
                field
                    .labelled_by()
                    .iter()
                    .all(|label| tree.nodes.iter().any(|(id, _)| id == label))
            );
            if secret {
                assert_ne!(field.value(), Some(value.as_str()));
            }
        }
    }

    #[test]
    fn pages_handle_missing_loading_failed_and_stale_states() {
        let ctx = themed();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        for (page, _) in Page::ALL {
            app.page = page;
            for (busy, error) in [(false, false), (true, false), (false, true)] {
                app.busy = busy;
                app.error = error;
                app.message = "设备不可用，请刷新".into();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 440.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.show(ctx),
                );
            }
        }
        assert!(!app.ready());
        assert!(!app.admin());
        assert!(!app.controls_room());
    }
    #[test]
    fn airplay_source_is_visible_searchable_and_admin_controls_follow_room_permissions() {
        let mut authority =
            neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                .unwrap();
        let admin = authority.snapshot().devices.values().next().unwrap().id;
        let member = authority
            .add_device("Member".into(), Role::Member, &"b".repeat(64))
            .unwrap();
        let snapshot = authority.snapshot();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.snapshot = Some(snapshot.clone());
        app.fresh = Some(Instant::now());
        app.airplay = Some(
            serde_json::json!({"revision":7,"enabled":true,"multi_receiver":true,
            "receivers":[{"receiver_id":"receiver-1","name":"Input 1","ready":true,"active":true}],
            "sources":[{"source_id":"source-digest","last_name":"Studio iPhone","blocked":false,"revoked":false}],
            "sessions":[{"receiver_id":"receiver-1","source_id":"source-digest","source_name":"Studio iPhone","session_id":77,"stream_id":88}],
            "capacity":{"limit":4,"active":1}}),
        );
        app.status = Some(ServiceStatus {
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![neonmix_desktop_service::ProfileInfo {
                credential: app.credential.clone(),
                device_id: Some(admin),
                hub_id: Some(snapshot.hub_id),
                name: Some("admin".into()),
                role: Some(Role::Admin),
                pending: false,
            }],
            sender_options: None,
            output_binding: None,
        });
        for (query, visible) in [
            ("", true),
            ("iphone", true),
            ("AIRPLAY", true),
            ("source-digest", true),
            ("no-match", false),
        ] {
            app.search = query.into();
            let ctx = themed();
            ctx.enable_accesskit();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.devices_page(ui));
            });
            let tree = output.platform_output.accesskit_update.unwrap();
            let has_name = tree
                .nodes
                .iter()
                .any(|(_, node)| node.value() == Some("Studio iPhone"));
            assert_eq!(has_name, visible, "query {query}");
            if visible {
                assert!(
                    tree.nodes
                        .iter()
                        .any(|(_, node)| node.label() == Some("断开 AirPlay 来源"))
                );
            }
        }
        assert!(
            app.airplay_request(
                neonmix_airplay_adapter::control::AirplayActionV2::DisconnectSource {
                    source_id: "source-digest".into(),
                    session_id: 76
                }
            )
            .is_none()
        );
        assert!(
            app.airplay_request(
                neonmix_airplay_adapter::control::AirplayActionV2::DisconnectSource {
                    source_id: "different-source".into(),
                    session_id: 77
                }
            )
            .is_none()
        );
        app.status.as_mut().unwrap().profiles[0].device_id = Some(member);
        app.search.clear();
        let ctx = themed();
        ctx.enable_accesskit();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.devices_page(ui));
        });
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.value() == Some("Studio iPhone"))
        );
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("断开 AirPlay 来源"))
        );
        assert!(
            app.airplay_request(
                neonmix_airplay_adapter::control::AirplayActionV2::DisconnectSource {
                    source_id: "source-digest".into(),
                    session_id: 77
                }
            )
            .is_none()
        );
    }
    #[test]
    fn commands_use_authoritative_revision_and_duplicate_submission_is_blocked() {
        let mut authority =
            neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                .unwrap();
        let admin = authority.snapshot().devices.values().next().unwrap().id;
        let member = authority
            .add_device("Member".into(), Role::Member, &"b".repeat(64))
            .unwrap();
        let snapshot = authority.snapshot();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.snapshot = Some(snapshot.clone());
        app.fresh = Some(Instant::now());
        app.status = Some(ServiceStatus {
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![neonmix_desktop_service::ProfileInfo {
                credential: app.credential.clone(),
                device_id: Some(member),
                hub_id: Some(snapshot.hub_id),
                name: Some("Member".into()),
                role: Some(Role::Member),
                pending: false,
            }],
            sender_options: None,
            output_binding: None,
        });
        assert!(!app.admin());
        assert!(!app.controls_room());
        app.status.as_mut().unwrap().profiles[0].device_id = Some(admin);
        assert!(app.admin());
        assert!(app.controls_room());
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        app.operation(Operation::OutputMix {
            gain_db: Some(-18.0),
            muted: None,
        });
        assert!(app.busy);
        assert!(!app.ready());
        match rx.try_recv().unwrap() {
            Work::Action(Request::Control {
                expected_revision,
                operation,
                ..
            }) => {
                assert_eq!(expected_revision, snapshot.revision);
                assert_eq!(
                    operation,
                    Operation::OutputMix {
                        gain_db: Some(-18.0),
                        muted: None
                    }
                );
            }
            _ => panic!("unexpected work"),
        }
        app.request(Request::HubStart);
        assert!(rx.try_recv().is_err());
        app.snapshot
            .as_mut()
            .unwrap()
            .devices
            .get_mut(&admin)
            .unwrap()
            .revoked = true;
        assert!(!app.admin());
        assert!(!app.controls_room());
    }
    #[test]
    fn chinese_composition_preserves_draft_without_enter_submission() {
        let ctx = egui::Context::default();
        let mut value = String::new();
        for events in [
            vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                egui::Event::Ime(egui::ImeEvent::Preedit("客厅".into())),
            ],
            vec![egui::Event::Ime(egui::ImeEvent::Commit("客厅".into()))],
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id("房间名称");
                        ui.memory_mut(|m| m.request_focus(id));
                        widgets::field(ui, "房间名称", &mut value, false);
                    });
                },
            );
        }
        assert_eq!(value, "客厅");
    }
    #[test]
    fn polling_does_not_lock_forms_and_stop_has_priority_over_pending_start() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        assert_eq!(app.manual_hub(), None);
        assert_eq!(app.hub(), None);
        app.busy = true;
        app.polling = true;
        assert!(!app.action_busy());
        app.request(Request::SenderStart {
            options: SenderOptions {
                credential: "profiles/sender.json".into(),
                hub: None,
                output_binding: "output".into(),
            },
        });
        assert!(app.action_busy());
        assert!(app.pending_action.is_some());
        app.stop_sender();
        assert!(app.pending_stop);
        assert!(app.pending_action.is_none());
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        assert!(app.urgent_action.is_some());
        let (urgent_done, urgent_result) = mpsc::sync_channel(1);
        app.urgent_action = Some(urgent_result);
        let (done, result) = mpsc::sync_channel(1);
        app.results = Some(result);
        done.send(Err("连接中断".into())).unwrap();
        urgent_done.send(Ok(Value::Null)).unwrap();
        app.process();
        assert!(app.last_poll.elapsed() < Duration::from_secs(1));
        assert!(rx.try_recv().is_err());
        assert!(!app.pending_stop);
    }
    #[test]
    fn stop_after_an_inflight_start_runs_last_and_keeps_the_stop_pending_until_ack() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        let start = Request::SenderStart {
            options: SenderOptions {
                credential: "profiles/sender.json".into(),
                hub: None,
                output_binding: "output".into(),
            },
        };
        app.request(start.clone());
        assert!(matches!(
            rx.try_recv().unwrap(),
            Work::Action(Request::SenderStart { .. })
        ));
        app.stop_sender();
        assert!(app.stop_after_start);
        assert!(app.pending_stop);
        assert!(app.urgent_action.is_none());
        let (done, result) = mpsc::sync_channel(1);
        app.results = Some(result);
        done.send(Ok(Outcome::Action(start, Value::Null))).unwrap();
        app.process();
        assert!(matches!(
            rx.try_recv().unwrap(),
            Work::Action(Request::SenderStop)
        ));
        assert!(app.pending_stop);
        assert!(!app.stop_after_start);
        done.send(Ok(Outcome::Action(Request::SenderStop, Value::Null)))
            .unwrap();
        app.process();
        assert!(!app.pending_stop);
        assert!(!app.busy);
    }
    #[test]
    fn cancel_modal_keeps_accesskit_focus_in_the_published_tree() {
        let ctx = themed();
        ctx.enable_accesskit();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        app.confirmation(
            "退出后台".into(),
            "停止本实例音频".into(),
            Request::Shutdown,
            egui::Id::new("nonexistent-origin"),
        );
        for events in [
            vec![],
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![],
        ] {
            let output = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            );
            let tree = output.platform_output.accesskit_update.unwrap();
            assert!(tree.nodes.iter().any(|(id, _)| *id == tree.focus));
        }
        assert!(app.confirm.is_none());
        assert!(!app.focus_after_modal);
    }

    /// User-reported flicker: an in-flight write used to disable (fade) the
    /// whole page and grey every control until the next poll. Controls must
    /// stay enabled, and a second write made meanwhile is kept and sent
    /// against the next authoritative revision instead of being dropped.
    #[test]
    fn inflight_write_keeps_controls_enabled_and_sends_latest_intent_later() {
        let authority =
            neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                .unwrap();
        let mut snapshot = authority.snapshot();
        let admin = snapshot.devices.values().next().unwrap().id;
        let status = ServiceStatus {
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![neonmix_desktop_service::ProfileInfo {
                credential: "hub/admin.json".into(),
                device_id: Some(admin),
                hub_id: Some(snapshot.hub_id),
                name: Some("admin".into()),
                role: Some(Role::Admin),
                pending: false,
            }],
            sender_options: None,
            output_binding: None,
        };
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.page = Page::Mixer;
        app.online = true;
        app.snapshot = Some(snapshot.clone());
        app.status = Some(status.clone());
        app.fresh = Some(Instant::now());
        app.synced = app.fresh;
        app.last_poll = Instant::now();
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        let (done, results) = mpsc::sync_channel(1);
        app.results = Some(results);

        app.operation(Operation::OutputMix {
            gain_db: None,
            muted: Some(true),
        });
        assert!(matches!(
            rx.try_recv(),
            Ok(Work::Action(Request::Control { .. }))
        ));
        assert!(!app.ready() && app.writable());

        let ctx = themed();
        ctx.enable_accesskit();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let mute = tree
            .nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.label() == Some("总静音"))
            .expect("master mute absent");
        assert!(!mute.is_disabled(), "controls must not grey out mid-write");

        app.operation(Operation::OutputMix {
            gain_db: Some(-6.0),
            muted: None,
        });
        assert!(rx.try_recv().is_err(), "second write must wait, not race");
        assert!(app.queued_write.is_some());

        done.send(Ok(Outcome::Action(Request::Status, Value::Null)))
            .unwrap();
        app.process();
        assert!(matches!(rx.try_recv(), Ok(Work::Poll { .. })));
        snapshot.revision += 1;
        done.send(Ok(Outcome::Poll(Box::new(PollData {
            credential: "hub/admin.json".into(),
            hub: None,
            room_name: None,
            status,
            snapshot: Some(snapshot.clone()),
            diagnostics: None,
            airplay: None,
            error: None,
        }))))
        .unwrap();
        app.process();
        match rx.try_recv() {
            Ok(Work::Action(Request::Control {
                expected_revision,
                operation,
                ..
            })) => {
                assert_eq!(expected_revision, snapshot.revision);
                assert_eq!(
                    operation,
                    Operation::OutputMix {
                        gain_db: Some(-6.0),
                        muted: None
                    }
                );
            }
            _ => panic!("queued write was not sent"),
        }
    }

    fn pointer(pos: egui::Pos2, pressed: Option<bool>, time: f64) -> egui::RawInput {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        egui::RawInput {
            events,
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 100.0),
            )),
            ..Default::default()
        }
    }

    /// A live room must not jump level because someone clicked the track;
    /// only a drag moves the fader. Double-click is the explicit reset.
    #[test]
    fn fader_click_never_jumps_gain_and_double_click_resets_to_unity() {
        let ctx = themed();
        let mut gain = -30.0_f32;
        let frame = |input: egui::RawInput, gain: &mut f32| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    widgets::fader(ui, "test", gain, "受测音量", widgets::FaderSize::Row);
                });
            });
        };
        let far = egui::pos2(380.0, 20.0);
        frame(pointer(far, None, 0.0), &mut gain);
        frame(pointer(far, Some(true), 0.1), &mut gain);
        frame(pointer(far, Some(false), 0.15), &mut gain);
        assert_eq!(gain, -30.0, "a click on the track must not move the fader");
        frame(pointer(far, Some(true), 0.2), &mut gain);
        frame(pointer(far, Some(false), 0.25), &mut gain);
        frame(pointer(far, None, 0.3), &mut gain);
        assert_eq!(gain, 0.0, "double-click returns to 0 dB");
    }

    /// Enter that confirms an IME candidate belongs to the input method; it
    /// must not run the highlighted palette action.
    #[test]
    fn palette_enter_during_ime_composition_does_not_run_an_action() {
        let ctx = themed();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        app.page = Page::Sender;
        app.open_palette();
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let run = |app: &mut Desktop, events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            );
        };
        run(&mut app, vec![]);
        run(
            &mut app,
            vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                egui::Event::Ime(egui::ImeEvent::Preedit("qian".into())),
                enter.clone(),
            ],
        );
        assert!(app.palette.is_some(), "palette stays open while composing");
        assert_eq!(app.page, Page::Sender);
        run(
            &mut app,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("前往".into()))],
        );
        run(&mut app, vec![enter]);
        assert!(
            app.palette.is_none(),
            "plain Enter runs the highlighted action"
        );
        assert_eq!(app.page, Page::Hub);
    }

    /// 还原 sends the inverse of the last mixer change, against the next
    /// authoritative revision rather than the one the change was made on.
    #[test]
    fn restore_sends_the_inverse_change_against_a_fresh_revision() {
        let authority =
            neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                .unwrap();
        let mut snapshot = authority.snapshot();
        let admin = snapshot.devices.values().next().unwrap().id;
        let status = ServiceStatus {
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![neonmix_desktop_service::ProfileInfo {
                credential: "hub/admin.json".into(),
                device_id: Some(admin),
                hub_id: Some(snapshot.hub_id),
                name: Some("admin".into()),
                role: Some(Role::Admin),
                pending: false,
            }],
            sender_options: None,
            output_binding: None,
        };
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.online = true;
        app.snapshot = Some(snapshot.clone());
        app.status = Some(status.clone());
        app.fresh = Some(Instant::now());
        app.synced = app.fresh;
        app.last_poll = Instant::now();
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        let (done, results) = mpsc::sync_channel(1);
        app.results = Some(results);

        app.master_mute(true);
        assert!(matches!(
            rx.try_recv(),
            Ok(Work::Action(Request::Control { .. }))
        ));
        assert!(app.undo_available());
        app.restore_last();
        assert!(
            rx.try_recv().is_err(),
            "restore waits for the fresh revision"
        );
        done.send(Ok(Outcome::Action(Request::Status, Value::Null)))
            .unwrap();
        app.process();
        assert!(matches!(rx.try_recv(), Ok(Work::Poll { .. })));
        snapshot.revision += 1;
        snapshot.output.muted = true;
        done.send(Ok(Outcome::Poll(Box::new(PollData {
            credential: "hub/admin.json".into(),
            hub: None,
            room_name: None,
            status,
            snapshot: Some(snapshot.clone()),
            diagnostics: None,
            airplay: None,
            error: None,
        }))))
        .unwrap();
        app.process();
        match rx.try_recv() {
            Ok(Work::Action(Request::Control {
                expected_revision,
                operation,
                ..
            })) => {
                assert_eq!(expected_revision, snapshot.revision);
                assert_eq!(
                    operation,
                    Operation::OutputMix {
                        gain_db: None,
                        muted: Some(false)
                    }
                );
            }
            _ => panic!("restore was not sent"),
        }
    }
    #[test]
    fn long_error_at_minimum_width_keeps_page_content_on_screen() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        app.page = Page::Devices;
        app.error = true;
        app.message =
            "状态已被其他操作更新，请核对后重试；未提交的设备名称与别名草稿仍会保留。".repeat(3);
        let ctx = themed();
        ctx.enable_accesskit();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 440.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        let header = tree
            .nodes
            .iter()
            .find(|(_, node)| node.value() == Some("设备管理"))
            .expect("page title must remain visible");
        let bounds = header.1.bounds().expect("page title bounds");
        assert!(
            bounds.y0 >= 0.0 && bounds.y1 < 440.0,
            "status message displaced page title"
        );
    }
}
