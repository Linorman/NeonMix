mod animation;
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

#[derive(Parser)]
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
    #[arg(long, value_parser = ["hub", "sender", "mixer", "devices", "diagnostics"])]
    preview_page: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Hash)]
enum Page {
    Hub,
    Sender,
    Mixer,
    Devices,
    Diagnostics,
}
impl Page {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Hub, "Hub 设置"),
        (Self::Sender, "Sender"),
        (Self::Mixer, "Mixer"),
        (Self::Devices, "设备管理"),
        (Self::Diagnostics, "诊断"),
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
                            match call(&client, &Request::Diagnostics { credential, hub }) {
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
    prev_page: Option<Page>,
    page_transition_start: Option<Instant>,
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
    fresh: Option<Instant>,
    last_poll: Instant,
    devices: Vec<DeviceInfo>,
    room: String,
    remote_room: Option<String>,
    output: String,
    sender_name: String,
    invitation: String,
    show_invitation: bool,
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
    gain_drafts: std::collections::BTreeMap<u64, f32>,
    tray: Option<tray::Tray>,
    tray_attempted: bool,
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
            app.page = match page.as_str() {
                "sender" => Page::Sender,
                "mixer" => Page::Mixer,
                "devices" => Page::Devices,
                "diagnostics" => Page::Diagnostics,
                _ => Page::Hub,
            };
        } else {
            let (tx, rx) = worker(app.client.clone(), cc.egui_ctx.clone());
            app.worker = Some(tx);
            app.results = Some(rx);
            app.queue(Work::Boot);
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
                        app.binding = app.status.as_ref().and_then(|s| s.output_binding.clone());
                        app.fresh = Some(Instant::now());
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
            prev_page: None,
            page_transition_start: None,
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
            fresh: None,
            last_poll: Instant::now() - Duration::from_secs(5),
            devices: vec![],
            room: "客厅".into(),
            remote_room: None,
            output: String::new(),
            sender_name: "我的电脑".into(),
            invitation: String::new(),
            show_invitation: false,
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
            gain_drafts: std::collections::BTreeMap::new(),
            tray: None,
            tray_attempted: false,
        }
    }
    fn queue(&mut self, work: Work) {
        if self.busy || self.preview {
            return;
        }
        let polling = matches!(&work, Work::Poll { .. });
        let starting = matches!(&work, Work::Action(Request::SenderStart { .. }));
        let stopping = matches!(&work, Work::Action(Request::SenderStop));
        if self
            .worker
            .as_ref()
            .is_some_and(|tx| tx.try_send(work).is_ok())
        {
            self.busy = true;
            self.polling = polling;
            self.inflight_start = starting;
            self.inflight_stop = stopping;
        }
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
                    self.diagnostics = None;
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
                        self.fresh = Some(Instant::now());
                        self.diagnostics = data
                            .diagnostics
                            .map(|v| v.get("remote").cloned().unwrap_or(v));
                    } else {
                        self.fresh = None;
                        self.diagnostics = None;
                    }
                    if let Some(e) = data.error {
                        self.message = user_error(e);
                        self.error = true;
                        self.fresh = None;
                    }
                }
                Ok(Outcome::Action(request, data)) => {
                    self.message = "操作已完成".into();
                    self.error = false;
                    self.last_poll = Instant::now() - Duration::from_secs(5);
                    match request {
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
                            self.invitation.clear();
                            self.message =
                                "本机配对已删除，输出已禁用；重新配对后手动启用或重新添加输出。"
                                    .into();
                        }
                        Request::PairText { .. } => {
                            self.credential = PathBuf::from("profiles/sender.json");
                            self.invitation.clear();
                            self.snapshot = None;
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
        if self.online
            && !self.busy
            && !self.pending_stop
            && self.last_poll.elapsed() > Duration::from_secs(1)
        {
            self.poll();
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
    fn sender_allowed(&self) -> bool {
        if !self.ready() || self.credential.as_path() == std::path::Path::new("hub/admin.json") {
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
        if let Some(state) = &self.snapshot {
            self.request(Request::Control {
                credential: self.credential.clone(),
                hub: self.hub(),
                expected_revision: state.revision,
                operation,
            });
            self.fresh = None;
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
    fn show(&mut self, ctx: &egui::Context) {
        self.process();

        // 顶部导航栏
        egui::TopBottomPanel::top("navigation")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE_2)
                    .inner_margin(egui::Margin::symmetric(24, 16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("NeonMix")
                            .size(22.0)
                            .strong()
                            .color(theme::TEXT_PRIMARY),
                    );
                    ui.add_space(24.0);

                    for (page, title) in Page::ALL {
                        let is_current = self.page == page;
                        let tab_id = ui.id().with(page);

                        // 标签页激活动画
                        let active_anim =
                            animation::animate_bool(ui, tab_id.with("active"), is_current);

                        // 悬停动画
                        let hovered = ui.ctx().memory(|mem| {
                            mem.data
                                .get_temp::<bool>(tab_id.with("hovered"))
                                .unwrap_or(false)
                        });
                        let hover_anim = animation::animate_bool(
                            ui,
                            tab_id.with("hover"),
                            hovered && !is_current,
                        );

                        // 组合背景色：激活状态优先
                        let bg_color = if is_current {
                            animation::lerp_color(
                                egui::Color32::TRANSPARENT,
                                theme::ACCENT.gamma_multiply(0.15),
                                active_anim,
                            )
                        } else {
                            animation::lerp_color(
                                egui::Color32::TRANSPARENT,
                                theme::SURFACE_3,
                                hover_anim,
                            )
                        };

                        let text_color = animation::lerp_color(
                            theme::TEXT_SECONDARY,
                            theme::ACCENT,
                            active_anim,
                        );

                        let button =
                            egui::Button::new(RichText::new(title).size(14.0).color(text_color))
                                .fill(bg_color)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(egui::CornerRadius::same(8));

                        let response = ui.add(button);

                        // 更新悬停状态
                        ui.ctx().memory_mut(|mem| {
                            mem.data
                                .insert_temp(tab_id.with("hovered"), response.hovered());
                        });

                        if self.focus_after_modal && self.confirm.is_none() && self.page == page {
                            response.request_focus();
                            self.focus_after_modal = false;
                        }
                        if response.clicked() {
                            if self.page != page {
                                animation::reset_page_transition(ctx, page);
                                self.prev_page = Some(self.page);
                                self.page = page;
                                self.page_transition_start = Some(Instant::now());
                            }
                        }
                    }

                    // 右侧状态信息
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Hub 状态
                        if self.status.as_ref().is_some_and(|s| s.hub.running) {
                            if let Some(snapshot) = &self.snapshot {
                                let active_streams = snapshot.streams.len();
                                let total_devices = snapshot.devices.len();

                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{}设备 · {}通道",
                                        total_devices, active_streams
                                    ))
                                    .size(13.0)
                                    .color(theme::TEXT_MUTED),
                                );
                            }
                            ui.add_space(8.0);
                            widgets::status_indicator(ui, true, "Hub");
                        }

                        // Sender 状态
                        if self.status.as_ref().is_some_and(|s| s.sender.running) {
                            ui.add_space(8.0);
                            widgets::status_indicator(ui, true, "发送");
                        }

                        // 连接状态
                        ui.add_space(8.0);
                        if self.online {
                            widgets::status_indicator(ui, true, "在线");
                        } else {
                            widgets::status_indicator(ui, false, "离线");
                        }
                    });
                });
            });
        // 底部状态栏
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE_2)
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.busy {
                        ui.spinner();
                        ui.add_space(8.0);
                    }
                    ui.label(
                        RichText::new(&self.message)
                            .size(13.0)
                            .color(if self.error {
                                theme::DANGER
                            } else {
                                theme::TEXT_SECONDARY
                            }),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = ui.add_enabled(
                            !self.action_busy(),
                            egui::Button::new(RichText::new("退出后台").size(13.0))
                                .fill(theme::DANGER.gamma_multiply(0.3))
                                .stroke(egui::Stroke::new(1.0, theme::DANGER))
                                .corner_radius(egui::CornerRadius::same(255))
                                .min_size(egui::vec2(0.0, 32.0)),
                        );
                        if response.clicked() {
                            self.confirmation(
                                "退出后台".into(),
                                "停止本实例的共享、发送和全部音频连接。".into(),
                                Request::Shutdown,
                                response.id,
                            );
                        }

                        ui.add_space(8.0);

                        if self.status.as_ref().is_some_and(|s| s.sender.running) {
                            if widgets::secondary_button(ui, "停止发送").clicked() {
                                self.stop_sender();
                            }
                            ui.add_space(8.0);
                        }

                        if widgets::secondary_button(ui, "隐藏窗口").clicked() {
                            if self.tray.is_some() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                            } else {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                    });
                });
            });
        // 主内容区域
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE_1)
                    .inner_margin(egui::Margin::symmetric(24, 20)),
            )
            .show(ctx, |ui| {
                // 页面过渡动画
                let transition = animation::page_transition(ui, self.page);

                egui::ScrollArea::vertical()
                    .id_salt(self.page.title())
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 应用淡入效果
                        let _alpha = (transition * 255.0) as u8;
                        ui.style_mut().visuals.widgets.inactive.fg_stroke.color = ui
                            .style()
                            .visuals
                            .widgets
                            .inactive
                            .fg_stroke
                            .color
                            .linear_multiply(transition);
                        ui.style_mut().visuals.widgets.active.fg_stroke.color = ui
                            .style()
                            .visuals
                            .widgets
                            .active
                            .fg_stroke
                            .color
                            .linear_multiply(transition);

                        // 垂直偏移动画（从上方滑入）
                        let offset = (1.0 - transition) * 20.0;
                        ui.add_space(offset + 8.0);

                        ui.label(
                            RichText::new(self.page.title())
                                .size(28.0)
                                .strong()
                                .color(theme::TEXT_PRIMARY.linear_multiply(transition)),
                        );
                        ui.add_space(16.0);

                        if !self.cjk {
                            widgets::card(ui, "cjk-warning", |ui| {
                                ui.colored_label(
                                    theme::WARNING,
                                    "中文字体未找到，请配置 NEONMIX_CJK_FONT 后重新打开。",
                                );
                            });
                        }
                        if self.preview {
                            widgets::card(ui, "preview-mode", |ui| {
                                ui.colored_label(
                                    theme::INFO,
                                    "视觉验证模式 · 不启动后台或播放音频",
                                );
                            });
                        }

                        ui.add_enabled_ui(!self.action_busy(), |ui| {
                            // 计算页面切换动画进度
                            let fade_alpha = if let Some(start) = self.page_transition_start {
                                let elapsed = start.elapsed().as_secs_f32();
                                let duration = 0.2; // 200ms 过渡时间
                                if elapsed < duration {
                                    ctx.request_repaint();
                                    (elapsed / duration).min(1.0)
                                } else {
                                    self.page_transition_start = None;
                                    1.0
                                }
                            } else {
                                1.0
                            };

                            // 应用淡入淡出效果
                            ui.scope(|ui| {
                                ui.set_opacity(fade_alpha);
                                match self.page {
                                    Page::Hub => self.hub_page(ui),
                                    Page::Sender => self.sender_page(ui),
                                    Page::Mixer => self.mixer_page(ui),
                                    Page::Devices => self.devices_page(ui),
                                    Page::Diagnostics => self.diagnostics_page(ui),
                                }
                            });
                        });
                    });
            });
        // 确认对话框
        if let Some(mut confirmation) = self.confirm.take() {
            let mut keep = true;
            let mut execute = false;
            let response = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
                ui.set_max_width((ctx.screen_rect().width() - 64.0).min(480.0));

                ui.add_space(8.0);
                ui.label(
                    RichText::new(&confirmation.label)
                        .size(20.0)
                        .strong()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.add_space(12.0);

                ui.label(
                    RichText::new(&confirmation.consequence)
                        .size(14.0)
                        .color(theme::TEXT_SECONDARY),
                );
                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    if widgets::secondary_button(ui, "取消").clicked() {
                        keep = false;
                    }
                    if confirmation.focus {
                        ui.memory_mut(|m| m.request_focus(ui.next_auto_id()));
                        confirmation.focus = false;
                    }

                    ui.add_space(8.0);
                    if ui
                        .add_enabled(
                            !self.action_busy(),
                            egui::Button::new(
                                RichText::new(confirmation.action_label())
                                    .color(theme::SURFACE_0)
                                    .size(15.0),
                            )
                            .fill(theme::DANGER)
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(255))
                            .min_size(egui::vec2(0.0, 36.0)),
                        )
                        .clicked()
                    {
                        execute = true;
                        keep = false;
                    }
                });
                ui.add_space(8.0);
            });
            if response.should_close() {
                keep = false;
            }
            if keep {
                self.confirm = Some(confirmation)
            } else {
                self.focus_after_modal = true;
                if execute {
                    self.request(confirmation.request);
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }
    fn hub_page(&mut self, ui: &mut egui::Ui) {
        widgets::note(ui, "配置房间、选择输出设备、管理配对邀请");
        ui.add_space(16.0);

        widgets::card(ui, "hub-settings", |ui| {
            ui.label(
                RichText::new("房间设置")
                    .size(18.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.add_space(12.0);

            widgets::field(ui, "房间名称", &mut self.room, false);
            ui.add_space(8.0);

            // 输出设备选择
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("实体输出设备")
                        .color(theme::TEXT_SECONDARY)
                        .size(13.0),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let label = ui.label("");
                    let response = egui::ComboBox::from_id_salt("output")
                        .width(ui.available_width().min(480.0) - 120.0)
                        .selected_text(
                            self.devices
                                .iter()
                                .find(|d| d.id == self.output)
                                .map_or("请选择设备", |d| d.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for device in &self.devices {
                                if device.output.is_some() && !virtual_device(device) {
                                    widgets::select_value(
                                        ui,
                                        &mut self.output,
                                        device.id.clone(),
                                        &device.name,
                                    );
                                }
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    widgets::label_combo(&response, "实体输出");

                    if widgets::secondary_button(ui, "刷新设备").clicked() {
                        self.request(Request::Devices);
                    }
                });
            });

            if !self.output.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&self.output)
                        .monospace()
                        .size(11.5)
                        .color(theme::TEXT_MUTED),
                );
            }

            ui.add_space(16.0);
            widgets::divider(ui);
            ui.add_space(8.0);

            // 操作按钮
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.output.is_empty(),
                        egui::Button::new(RichText::new("播放测试音").size(15.0))
                            .fill(theme::SURFACE_3)
                            .stroke(egui::Stroke::new(1.0, theme::SURFACE_4))
                            .corner_radius(egui::CornerRadius::same(255))
                            .min_size(egui::vec2(0.0, 36.0)),
                    )
                    .on_hover_text("−36 dBFS / 2 秒低音量测试")
                    .clicked()
                {
                    self.request(Request::TestTone {
                        output: self.output.clone(),
                    });
                }

                let configured = self
                    .status
                    .as_ref()
                    .is_some_and(|s| s.hub_settings.is_some());
                let running = self.status.as_ref().is_some_and(|s| s.hub.running);

                if ui
                    .add_enabled(
                        !running && !self.room.trim().is_empty() && !self.output.is_empty(),
                        egui::Button::new(
                            RichText::new(if configured {
                                "保存设置"
                            } else {
                                "创建房间"
                            })
                            .size(15.0),
                        )
                        .fill(theme::SURFACE_3)
                        .stroke(egui::Stroke::new(1.0, theme::SURFACE_4))
                        .corner_radius(egui::CornerRadius::same(255))
                        .min_size(egui::vec2(0.0, 36.0)),
                    )
                    .clicked()
                {
                    let settings = HubSettings {
                        name: self.room.trim().into(),
                        output: self.output.clone(),
                    };
                    self.request(if configured {
                        Request::HubSettings { settings }
                    } else {
                        Request::HubSetup { settings }
                    });
                }

                if !running && configured {
                    if widgets::primary_button(ui, "开始共享").clicked() {
                        self.request(Request::HubStart);
                    }
                }
                if running {
                    if widgets::danger_button(ui, "停止共享").clicked() {
                        self.request(Request::HubStop);
                    }
                }
            });

            ui.add_space(8.0);
            widgets::note(
                ui,
                "输出按设备身份绑定，修改设置前需停止共享。共享仅面向局域网。",
            );

            // 状态显示
            if let Some(status) = &self.status {
                ui.add_space(12.0);
                if status.hub.running {
                    widgets::status_badge(ui, "Hub 正在共享", widgets::BadgeStatus::Success);
                } else {
                    widgets::status_badge(ui, "Hub 已停止", widgets::BadgeStatus::Neutral);
                }
                if let Some(e) = &status.hub.error {
                    ui.add_space(8.0);
                    ui.colored_label(theme::DANGER, e);
                }
            }
        });

        self.profile_picker(ui);

        widgets::card(ui, "pairing", |ui| {
            ui.label(
                RichText::new("配对邀请")
                    .size(18.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.add_space(8.0);
            widgets::note(
                ui,
                "邀请允许一台设备加入房间，有效期 120 秒。将邀请私下交给目标设备。",
            );
            ui.add_space(12.0);

            if widgets::primary_button(ui, "创建一次性邀请")
                .on_disabled_hover_text("请选择已连接的本地管理员身份")
                .clicked()
                && self.ready()
                && self.admin()
            {
                self.request(Request::Invite {
                    credential: self.credential.clone(),
                    hub: self.hub(),
                    out: PathBuf::from(format!("invitations/{}.json", uuid::Uuid::new_v4())),
                    seconds: 120,
                });
            }

            if !self.issued_invitation.is_empty() {
                ui.add_space(16.0);
                ui.checkbox(&mut self.show_invitation, "显示邀请内容");
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.issued_invitation)
                        .password(!self.show_invitation)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width())
                        .interactive(false),
                );

                if let Some(expiry) = self.invite_expiry {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    ui.add_space(4.0);
                    widgets::note(ui, format!("剩余 {} 秒", expiry.saturating_sub(now)));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::secondary_button(ui, "复制邀请").clicked() {
                        ui.ctx().copy_text(self.issued_invitation.clone());
                    }
                    if let Some(invitation_id) = self.invite_id {
                        if widgets::secondary_button(ui, "取消邀请").clicked() {
                            self.request(Request::CancelInvite {
                                credential: self.credential.clone(),
                                hub: self.hub(),
                                invitation_id,
                            });
                        }
                    }
                });
            }
        });

        // 已配对设备列表
        if let Some(state) = self.snapshot.clone() {
            widgets::card(ui, "paired-devices", |ui| {
                ui.label(
                    RichText::new("已配对设备")
                        .size(18.0)
                        .strong()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.add_space(12.0);

                if state.devices.is_empty() {
                    widgets::empty_state(
                        ui,
                        "📱",
                        "暂无设备",
                        "创建邀请后，设备配对成功将显示在此处。",
                    );
                } else {
                    egui::Grid::new("device_grid")
                        .num_columns(1)
                        .spacing([0.0, 8.0])
                        .show(ui, |ui| {
                            for device in state.devices.values().take(5) {
                                // 设备信息卡片
                                egui::Frame::new()
                                    .fill(theme::SURFACE_3)
                                    .corner_radius(egui::CornerRadius::same(6))
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            // 左侧：状态和名称
                                            let is_active = state
                                                .streams
                                                .values()
                                                .any(|s| s.device_id == device.id);
                                            let (status_text, status_color) = if device.revoked {
                                                ("已撤销", theme::DANGER)
                                            } else if !device.playback_allowed {
                                                ("已断开", theme::WARNING)
                                            } else if is_active {
                                                ("活动中", theme::SUCCESS)
                                            } else {
                                                ("空闲", theme::TEXT_MUTED)
                                            };

                                            // 状态指示器
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(10.0, 10.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().circle_filled(
                                                rect.center(),
                                                5.0,
                                                status_color,
                                            );

                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 4.0;

                                                ui.label(
                                                    RichText::new(&device.name)
                                                        .size(15.0)
                                                        .strong()
                                                        .color(theme::TEXT_PRIMARY),
                                                );

                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 8.0;

                                                    ui.label(
                                                        RichText::new(status_text)
                                                            .size(12.0)
                                                            .color(status_color),
                                                    );

                                                    ui.label(
                                                        RichText::new("·")
                                                            .size(12.0)
                                                            .color(theme::TEXT_MUTED),
                                                    );

                                                    ui.label(
                                                        RichText::new(match device.role {
                                                            Role::Admin => "管理员",
                                                            Role::Controller => "控制者",
                                                            Role::Member => "成员",
                                                        })
                                                        .size(12.0)
                                                        .color(theme::TEXT_MUTED),
                                                    );
                                                });
                                            });

                                            // 右侧：活动通道数
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if is_active {
                                                        let stream_count = state
                                                            .streams
                                                            .values()
                                                            .filter(|s| s.device_id == device.id)
                                                            .count();
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{} 通道",
                                                                stream_count
                                                            ))
                                                            .size(13.0)
                                                            .color(theme::ACCENT),
                                                        );
                                                    }
                                                },
                                            );
                                        });
                                    });
                                ui.end_row();
                            }
                        });

                    if state.devices.len() > 5 {
                        ui.add_space(8.0);
                        widgets::note(
                            ui,
                            format!(
                                "还有 {} 台设备，前往设备页面查看全部",
                                state.devices.len() - 5
                            ),
                        );
                    }
                }
            });
        }
    }
    fn profile_picker(&mut self, ui: &mut egui::Ui) {
        let previous = self.credential.clone();
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("控制身份");
            let response = egui::ComboBox::from_id_salt("credential")
                .selected_text(
                    self.status
                        .as_ref()
                        .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
                        .map_or("未配对", |p| {
                            if p.role == Some(Role::Admin) {
                                "本地管理员"
                            } else {
                                p.name.as_deref().unwrap_or("已配对设备")
                            }
                        }),
                )
                .show_ui(ui, |ui| {
                    if let Some(status) = &self.status {
                        for p in &status.profiles {
                            if !p.pending {
                                widgets::select_value(
                                    ui,
                                    &mut self.credential,
                                    p.credential.clone(),
                                    p.name.as_deref().unwrap_or("已配对设备"),
                                );
                            }
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            widgets::label_combo(&response, "控制身份");
            ui.label(match self.role() {
                Some(Role::Admin) => "管理员",
                Some(Role::Controller) => "房间控制者",
                Some(Role::Member) => "成员",
                None => "身份未验证",
            });
        });
        if previous != self.credential {
            self.snapshot = None;
            self.diagnostics = None;
            self.fresh = None;
            self.last_poll = Instant::now() - Duration::from_secs(5);
        }
        egui::CollapsingHeader::new("连接地址（故障排查）").show(ui, |ui| {
            let before = self.hub_address.clone();
            widgets::field(ui, "HTTPS 地址（可留空）", &mut self.hub_address, false);
            if before != self.hub_address {
                self.fresh = None;
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
            widgets::note(
                ui,
                "留空使用自动发现；任何地址都必须匹配已配对的证书和房间身份。",
            );
        });
        if self.snapshot.is_some() && !self.ready() {
            ui.colored_label(
                theme::DANGER,
                "状态正在刷新或已过期；取得最新状态后才能修改。",
            );
        }
    }
    fn sender_page(&mut self, ui: &mut egui::Ui) {
        widgets::note(
            ui,
            "发送范围：系统选择到此虚拟输出的所有应用声音。停止发送始终可从底部或托盘执行。",
        );
        if let Some(device) = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .and_then(|p| p.device_id)
            .and_then(|id| self.snapshot.as_ref()?.devices.get(&id))
        {
            if device.revoked {
                ui.colored_label(
                    theme::DANGER,
                    "配对已撤销；删除本机配对后，使用新邀请重新配对。",
                );
            } else if !device.playback_allowed {
                ui.colored_label(
                    theme::DANGER,
                    "管理员已断开；需要 Hub 重新允许播放，然后手动开始发送。",
                );
            }
        }
        if let Some(room) = &self.remote_room {
            ui.label(format!("目标房间：{room}"));
        }
        widgets::card(ui, "discover", |ui| {
            if ui.button("发现局域网房间").clicked() {
                self.request(Request::Discover { seconds: 3 });
            }
            if self.candidates.is_empty() {
                ui.label("尚未发现房间。确保 Hub 正在共享且位于同一局域网，然后重新发现。");
            }
            for candidate in &self.candidates {
                ui.label(
                    candidate
                        .get("room_name")
                        .or_else(|| candidate.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("未命名房间"),
                );
                widgets::note(
                    ui,
                    format!(
                        "Hub {} · 发现身份尚未信任",
                        candidate.get("hub_id").unwrap_or(&Value::Null)
                    ),
                );
            }
        });
        widgets::card(ui, "pair", |ui| {
            widgets::field(ui, "设备名称", &mut self.sender_name, false);
            ui.label("粘贴 Hub 给出的一次性邀请");
            ui.add(
                egui::TextEdit::singleline(&mut self.invitation)
                    .password(!self.show_invitation)
                    .desired_width(ui.available_width()),
            );
            ui.checkbox(&mut self.show_invitation, "显示邀请");
            if ui
                .add_enabled(
                    !self.invitation.trim().is_empty() && !self.sender_name.trim().is_empty(),
                    egui::Button::new("配对房间"),
                )
                .clicked()
            {
                self.request(Request::PairText {
                    invitation: self.invitation.trim().into(),
                    name: self.sender_name.trim().into(),
                    hub: self.manual_hub(),
                });
            }
        });
        if self.status.as_ref().is_some_and(|s| {
            s.profiles
                .iter()
                .any(|p| p.credential == std::path::Path::new("profiles/sender.json"))
        }) {
            let forget = ui.button("删除本机配对…");
            if forget.clicked() {
                self.confirmation("删除本机配对".into(),"停止发送并禁用本地输出，删除本机的配对凭证；需要新邀请才能重新配对。Hub 中的设备记录保留。".into(),Request::ForgetCredential{credential:"profiles/sender.json".into()},forget.id);
            }
        }
        self.profile_picker(ui);
        widgets::card(ui, "binding", |ui| {
            widgets::field(ui, "虚拟输出名称", &mut self.binding_name, false);
            ui.label("虚拟输出提供者");
            let response = egui::ComboBox::from_id_salt("provider")
                .selected_text(if self.provider == "blackhole" {
                    "BlackHole（外部提供者）"
                } else {
                    "NeonMix 虚拟输出"
                })
                .show_ui(ui, |ui| {
                    widgets::select_value(
                        ui,
                        &mut self.provider,
                        "neonmix".into(),
                        "NeonMix 虚拟输出",
                    );
                    if cfg!(target_os = "macos") {
                        widgets::select_value(
                            ui,
                            &mut self.provider,
                            "blackhole".into(),
                            "BlackHole（外部提供者）",
                        );
                    }
                });
            widgets::label_combo(&response.response, "虚拟输出提供者");
            if self.binding.is_none() {
                if ui
                    .add_enabled(
                        self.ready()
                            && self.role().is_some()
                            && self.credential.as_path() != std::path::Path::new("hub/admin.json")
                            && !self.binding_name.trim().is_empty(),
                        egui::Button::new("添加虚拟输出"),
                    )
                    .clicked()
                {
                    self.request(Request::Output {
                        directory: PathBuf::from("output"),
                        action: OutputAction::Add {
                            credential: self.credential.clone(),
                            hub: self.hub(),
                            name: self.binding_name.clone(),
                            provider: self.provider.clone(),
                            device: None,
                        },
                    });
                }
                if ui.button("读取已添加输出").clicked() {
                    self.request(Request::Output {
                        directory: PathBuf::from("output"),
                        action: OutputAction::Show,
                    });
                }
                widgets::note(
                    ui,
                    "需要已配对的 Sender 身份及已安装的虚拟设备。缺少驱动时请按 E06 安装说明处理。",
                );
            }
            if let Some(binding) = self.binding.clone() {
                let revision = binding.get("revision").and_then(Value::as_u64).unwrap_or(0);
                let enabled = binding
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                ui.label(format!(
                    "{} · {}",
                    binding
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or("虚拟输出"),
                    if enabled { "已启用" } else { "已禁用" }
                ));
                widgets::note(
                    ui,
                    format!(
                        "设备 {} · 房间 {}",
                        binding.get("device_id").unwrap_or(&Value::Null),
                        binding.get("hub_id").unwrap_or(&Value::Null)
                    ),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("保存输出名称").clicked() {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: OutputAction::Rename {
                                expected_revision: revision,
                                name: self.binding_name.clone(),
                            },
                        });
                    }
                    if ui
                        .button(if enabled {
                            "禁用输出"
                        } else {
                            "启用输出"
                        })
                        .clicked()
                    {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: if enabled {
                                OutputAction::Disable {
                                    expected_revision: revision,
                                }
                            } else {
                                OutputAction::Enable {
                                    expected_revision: revision,
                                }
                            },
                        });
                    }
                    let remove = ui.button("删除绑定…");
                    if remove.clicked() {
                        self.confirmation(
                            "删除输出绑定".into(),
                            "停止此绑定的发送，保留系统音频驱动；需要重新添加后才能发送。".into(),
                            Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Remove {
                                    expected_revision: revision,
                                },
                            },
                            remove.id,
                        );
                    }
                });
                widgets::note(
                    ui,
                    "发送范围：系统选到此虚拟输出的所有应用声音。请在系统声音设置中主动选择该设备。",
                );
                let running = self.status.as_ref().is_some_and(|s| s.sender.running);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            enabled && !running && self.sender_allowed(),
                            egui::Button::new("开始发送"),
                        )
                        .clicked()
                    {
                        self.request(Request::SenderStart {
                            options: SenderOptions {
                                credential: self.credential.clone(),
                                hub: self.hub(),
                                output_binding: PathBuf::from("output"),
                            },
                        });
                    }
                    if ui
                        .add_enabled(running, egui::Button::new("停止发送"))
                        .clicked()
                    {
                        self.stop_sender();
                    }
                });
            }
            if let Some(status) = &self.status {
                ui.label(if status.sender.running {
                    if status
                        .sender
                        .metrics
                        .as_ref()
                        .and_then(|m| m["control_connected"].as_bool())
                        == Some(false)
                    {
                        "控制重连中 · 音频进程运行"
                    } else if status
                        .sender
                        .metrics
                        .as_ref()
                        .and_then(|m| m["encoded_media_packets"].as_u64())
                        .unwrap_or(0)
                        > 0
                    {
                        "Sender 正在发送"
                    } else {
                        "Sender 连接 / 准备采集中"
                    }
                } else {
                    "Sender 已停止"
                });
                if let Some(e) = &status.sender.error {
                    ui.colored_label(theme::DANGER, e);
                }
                if let Some(metrics) = &status.sender.metrics {
                    widgets::note(
                        ui,
                        format!(
                            "采集帧：{}",
                            metrics
                                .pointer("/capture_stats/frames")
                                .unwrap_or(&Value::Null)
                        ),
                    );
                }
            }
        });
    }
    fn mixer_page(&mut self, ui: &mut egui::Ui) {
        self.profile_picker(ui);
        let Some(state) = self.snapshot.clone() else {
            widgets::empty_state(
                ui,
                "🎚️",
                "尚未连接房间",
                "创建并开始共享，或在 Sender 页面配对房间后查看混音状态。",
            );
            return;
        };

        widgets::note(
            ui,
            format!("房间 {} · 状态版本 {}", state.hub_id, state.revision),
        );
        ui.add_space(16.0);

        // 房间总控
        widgets::card(ui, "master", |ui| {
            ui.label(
                RichText::new("房间总控")
                    .size(18.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(&state.output.id)
                    .size(13.0)
                    .monospace()
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(8.0);

            if state.output.available {
                widgets::status_badge(ui, "输出可用", widgets::BadgeStatus::Success);
            } else {
                widgets::status_badge(ui, "输出丢失 · 等待设备恢复", widgets::BadgeStatus::Warning);
            }

            ui.add_space(12.0);

            let meter = self
                .diagnostics
                .as_ref()
                .and_then(|v| v.pointer("/meters/output"));
            widgets::meter(
                ui,
                meter.and_then(|v| v["peak"].as_f64()),
                meter.and_then(|v| v["rms"].as_f64()),
            );

            let limiter = self
                .diagnostics
                .as_ref()
                .and_then(|v| v.pointer("/meters/limiter_gain"))
                .and_then(Value::as_f64);
            ui.add_space(4.0);
            widgets::note(
                ui,
                limiter.map_or_else(
                    || "限幅数据未取得".into(),
                    |g| format!("限幅增益：{:.1} dB", 20.0 * g.max(1e-6).log10()),
                ),
            );

            // 添加房间统计信息
            ui.add_space(8.0);
            let active_count = state.streams.len();
            let total_devices = state.devices.len();
            let muted_count = state.streams.values().filter(|s| s.mix.muted).count();
            let solo_count = state.streams.values().filter(|s| s.mix.solo).count();

            let mut stats = vec![
                ("活动通道", format!("{}", active_count)),
                ("设备数", format!("{}", total_devices)),
            ];
            if muted_count > 0 {
                stats.push(("静音", format!("{}", muted_count)));
            }
            if solo_count > 0 {
                stats.push(("Solo", format!("{}", solo_count)));
            }
            widgets::stat_row(ui, &stats);

            ui.add_space(16.0);
            widgets::divider(ui);
            ui.add_space(8.0);

            ui.add_enabled_ui(self.ready() && self.controls_room(), |ui| {
                if let Some(gain) =
                    gain_input(ui, &mut self.gain_drafts, 0, state.output.gain_db, "总音量")
                {
                    self.operation(Operation::OutputMix {
                        gain_db: Some(gain),
                        muted: None,
                    });
                }

                ui.add_space(12.0);
                if let Some(db) = widgets::volume_presets(ui, state.output.gain_db) {
                    self.operation(Operation::OutputMix {
                        gain_db: Some(db),
                        muted: None,
                    });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if state.output.muted {
                        if widgets::primary_button(ui, "取消总静音").clicked() {
                            self.operation(Operation::OutputMix {
                                gain_db: None,
                                muted: Some(false),
                            });
                        }
                    } else {
                        if widgets::secondary_button(ui, "总静音").clicked() {
                            self.operation(Operation::OutputMix {
                                gain_db: None,
                                muted: Some(true),
                            });
                        }
                    }
                });
            });
        });

        // 输入通道
        if state.streams.is_empty() {
            widgets::empty_state(
                ui,
                "🔇",
                "暂无输入",
                "Sender 配对并开始发送后，通道将显示在此处。",
            );
        } else {
            ui.label(
                RichText::new("输入通道")
                    .size(20.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.add_space(12.0);
        }

        let my_device = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .and_then(|p| p.device_id);

        // 将streams收集到Vec中以便按列分配
        let mut streams_vec: Vec<_> = state.streams.values().collect();
        streams_vec.sort_by_key(|s| s.id);

        let stream_count = streams_vec.len();
        if stream_count > 0 {
            let grid_width = ui.available_width();
            let columns = ((grid_width + 16.0) / (360.0 + 16.0)).floor().max(1.0) as usize;

            widgets::responsive_grid(ui, "streams_grid", 360.0, |column_ui, col_idx| {
                let available_width = column_ui.available_width();

                // 为每一列分配streams
                for (idx, stream) in streams_vec.iter().enumerate() {
                    if idx % columns != col_idx {
                        continue;
                    }

                    let stream = *stream;
                    let card_id = format!("stream_{}", stream.id);

                    column_ui.push_id(card_id.clone(), |ui| {
                        ui.set_width(available_width);

                        widgets::card(ui, &card_id, |ui| {
                            let device = state.devices.get(&stream.device_id);
                            let name = device.map_or("未知设备", |d| d.name.as_str());

                            // 设备信息卡片
                            if let Some(device) = device {
                                let role_str = match device.role {
                                    Role::Admin => "Admin",
                                    Role::Controller => "Controller",
                                    Role::Member => "Member",
                                };
                                widgets::device_card(
                                    ui,
                                    name,
                                    role_str,
                                    !device.revoked && device.playback_allowed,
                                    true,
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(format!("通道 {}", stream.id))
                                        .size(13.0)
                                        .color(theme::TEXT_MUTED),
                                );
                            } else {
                                // 降级显示：设备信息不可用
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(name)
                                            .size(18.0)
                                            .strong()
                                            .color(theme::TEXT_PRIMARY),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(format!("通道 {}", stream.id))
                                                    .size(13.0)
                                                    .color(theme::TEXT_MUTED)
                                                    .monospace(),
                                            );
                                        },
                                    );
                                });
                            }

                            ui.add_space(12.0);

                            // 状态徽章
                            let status = state.sessions.get(&stream.session_id).map(|s| s.status);
                            let status_text = if !state.output.available {
                                "输出丢失"
                            } else if stream.mix.muted {
                                "静音"
                            } else if state.streams.values().any(|s| s.mix.solo) && !stream.mix.solo
                            {
                                "因其他通道 Solo 静音"
                            } else {
                                status_text(status)
                            };

                            let badge_status = match status_text {
                                "播放中" => widgets::BadgeStatus::Success,
                                "缓冲中" => widgets::BadgeStatus::Info,
                                "网络降级" => widgets::BadgeStatus::Warning,
                                _ => widgets::BadgeStatus::Neutral,
                            };
                            widgets::status_badge(ui, status_text, badge_status);

                            ui.add_space(16.0);

                            // 电平表
                            let meter = self
                                .diagnostics
                                .as_ref()
                                .and_then(|v| v.pointer("/meters/lanes"))
                                .and_then(Value::as_array)
                                .and_then(|lanes| {
                                    lanes
                                        .iter()
                                        .find(|v| v["stream_id"].as_u64() == Some(stream.id))
                                });
                            widgets::meter(
                                ui,
                                meter.and_then(|v| v["peak"].as_f64()),
                                meter.and_then(|v| v["rms"].as_f64()),
                            );

                            // 显示网络质量和延迟信息
                            ui.add_space(8.0);
                            if let Some(session) = state.sessions.get(&stream.session_id) {
                                let mut stats = Vec::new();

                                // 延迟估计
                                if let Some(diag) = &self.diagnostics {
                                    if let Some(queues) =
                                        diag.get("queues").and_then(Value::as_array)
                                    {
                                        if let Some(queue_frames) =
                                            queues.get(stream.id as usize).and_then(Value::as_f64)
                                        {
                                            let latency_ms = queue_frames / 48.0;
                                            stats.push(("缓冲", format!("{:.1} ms", latency_ms)));
                                        }
                                    }
                                }

                                // 网络状态
                                let net_status = match session.status {
                                    SessionStatus::Playing => "良好",
                                    SessionStatus::Buffering => "缓冲",
                                    SessionStatus::NetworkDegraded => "降级",
                                    _ => "连接中",
                                };
                                stats.push(("网络", net_status.to_string()));

                                if !stats.is_empty() {
                                    widgets::stat_row(ui, &stats);
                                }
                            }

                            ui.add_space(12.0);
                            widgets::divider(ui);
                            ui.add_space(12.0);

                            ui.add_enabled_ui(
                                self.ready()
                                    && (self.controls_room()
                                        || my_device == Some(stream.device_id)),
                                |ui| {
                                    if let Some(gain) = gain_input(
                                        ui,
                                        &mut self.gain_drafts,
                                        stream.id,
                                        stream.mix.gain_db,
                                        "输入音量",
                                    ) {
                                        self.operation(Operation::StreamMix {
                                            stream_id: stream.id,
                                            gain_db: Some(gain),
                                            muted: None,
                                            solo: None,
                                        });
                                    }

                                    ui.add_space(12.0);
                                    if let Some(db) =
                                        widgets::volume_presets(ui, stream.mix.gain_db)
                                    {
                                        self.operation(Operation::StreamMix {
                                            stream_id: stream.id,
                                            gain_db: Some(db),
                                            muted: None,
                                            solo: None,
                                        });
                                    }

                                    ui.add_space(8.0);
                                    ui.horizontal_wrapped(|ui| {
                                        if stream.mix.muted {
                                            if widgets::primary_button(ui, "取消 Mute").clicked()
                                            {
                                                self.operation(Operation::StreamMix {
                                                    stream_id: stream.id,
                                                    gain_db: None,
                                                    muted: Some(false),
                                                    solo: None,
                                                });
                                            }
                                        } else {
                                            if widgets::secondary_button(ui, "Mute").clicked() {
                                                self.operation(Operation::StreamMix {
                                                    stream_id: stream.id,
                                                    gain_db: None,
                                                    muted: Some(true),
                                                    solo: None,
                                                });
                                            }
                                        }

                                        if self.controls_room() {
                                            if stream.mix.solo {
                                                if widgets::primary_button(ui, "取消 Solo")
                                                    .clicked()
                                                {
                                                    self.operation(Operation::StreamMix {
                                                        stream_id: stream.id,
                                                        gain_db: None,
                                                        muted: None,
                                                        solo: Some(false),
                                                    });
                                                }
                                            } else {
                                                if widgets::secondary_button(ui, "Solo").clicked() {
                                                    self.operation(Operation::StreamMix {
                                                        stream_id: stream.id,
                                                        gain_db: None,
                                                        muted: None,
                                                        solo: Some(true),
                                                    });
                                                }
                                            }
                                        }

                                        if self.admin() {
                                            if widgets::danger_button(ui, "断开设备").clicked()
                                            {
                                                self.operation(Operation::Disconnect {
                                                    device_id: stream.device_id,
                                                });
                                            }
                                        }
                                    });
                                },
                            );
                        });
                    });
                }
            });
        }

        widgets::note(
            ui,
            "电平：最新 50 ms 双声道窗口；每路在 Mute/Solo 和增益之后、总控之前，总控在限幅之后。数据随诊断刷新。",
        );
    }
    fn devices_page(&mut self, ui: &mut egui::Ui) {
        self.profile_picker(ui);
        ui.horizontal(|ui| {
            widgets::field(ui, "搜索设备", &mut self.search, false);
            if !self.search.is_empty() && ui.button("清除").clicked() {
                self.search.clear();
            }
        });
        if let Some(state) = self.snapshot.clone() {
            let mut count = 0;
            for device in state
                .devices
                .values()
                .filter(|d| {
                    self.search.is_empty()
                        || d.name.contains(&self.search)
                        || d.id.to_string().contains(&self.search)
                })
                .cloned()
                .collect::<Vec<_>>()
            {
                count += 1;
                widgets::card(ui, device.id, |ui| {
                    // 使用device_card组件显示设备信息
                    let role_str = match device.role {
                        Role::Admin => "Admin",
                        Role::Controller => "Controller",
                        Role::Member => "Member",
                    };
                    widgets::device_card(
                        ui,
                        &device.name,
                        role_str,
                        !device.revoked && device.playback_allowed,
                        !device.revoked,
                    );

                    ui.add_space(12.0);

                    // 显示设备详细信息
                    let mut device_stats = Vec::new();
                    device_stats.push(("设备ID", device.id.to_string()));

                    // 查找该设备的活动流
                    let active_streams = state
                        .streams
                        .values()
                        .filter(|s| s.device_id == device.id)
                        .count();
                    if active_streams > 0 {
                        device_stats.push(("活动流", format!("{}", active_streams)));
                    }

                    // 权限状态
                    let permission = if device.revoked {
                        "已撤销"
                    } else if device.playback_allowed {
                        "播放已授权"
                    } else {
                        "播放已禁止"
                    };
                    device_stats.push(("权限", permission.to_string()));

                    widgets::stat_row(ui, &device_stats);

                    ui.add_space(12.0);
                    widgets::divider(ui);
                    ui.add_space(8.0);

                    ui.add_enabled_ui(self.ready() && self.admin() && !device.revoked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if !device.playback_allowed && ui.button("重新允许播放").clicked()
                            {
                                self.operation(Operation::AllowPlayback {
                                    device_id: device.id,
                                });
                            }
                            if device.playback_allowed
                                && device.role != Role::Admin
                                && ui.button("断开设备").clicked()
                            {
                                self.operation(Operation::Disconnect {
                                    device_id: device.id,
                                });
                            }
                            let revoke = ui.add_enabled(
                                device.role != Role::Admin,
                                egui::Button::new("撤销配对…"),
                            );
                            if revoke.clicked() {
                                self.confirmation(
                                    format!("撤销 {} 的配对", device.name),
                                    "立即结束该设备的媒体和控制连接；旧凭证将失效，需要重新配对。"
                                        .into(),
                                    Request::Control {
                                        credential: self.credential.clone(),
                                        hub: self.hub(),
                                        expected_revision: state.revision,
                                        operation: Operation::Revoke {
                                            device_id: device.id,
                                        },
                                    },
                                    revoke.id,
                                );
                            }
                        });
                    });
                });
            }
            if count == 0 {
                if self.search.is_empty() {
                    widgets::empty_state(ui, "📱", "暂无设备", "Sender 配对后，设备将显示在此处。");
                } else {
                    widgets::empty_state(
                        ui,
                        "🔍",
                        "无匹配结果",
                        "未找到匹配的设备，尝试清除搜索条件。",
                    );
                }
            }
        } else {
            widgets::empty_state(ui, "🔌", "未连接房间", "请先连接到 Hub 以查看设备列表。");
        }
        egui::CollapsingHeader::new("本机音频设备身份").show(ui, |ui| {
            if ui.button("刷新设备").clicked() {
                self.request(Request::Devices);
            }
            for device in &self.devices {
                ui.label(&device.name);
                widgets::note(ui, &device.id);
            }
            if self.devices.is_empty() {
                ui.label("未取得本机音频设备。");
            }
        });
    }
    fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        self.profile_picker(ui);
        widgets::card(ui, "health", |ui| {
            ui.heading("连接与进程");
            if let Some(status) = &self.status {
                ui.label(format!(
                    "本地后台 PID {} · Hub {} · Sender {}",
                    status.pid,
                    if status.hub.running {
                        "运行"
                    } else {
                        "停止"
                    },
                    if status.sender.running {
                        "运行"
                    } else {
                        "停止"
                    }
                ));
            }
            ui.label(match self.fresh {
                Some(t) => format!("房间状态距上次更新 {:.1} 秒", t.elapsed().as_secs_f32()),
                None => "未取得有效房间状态；请检查共享状态、发现或配对身份。".into(),
            });
        });
        if let Some(diagnostics) = self.diagnostics.clone() {
            widgets::card(ui, "output-stats", |ui| {
                ui.heading("播放与输出");
                for (label, pointer) in [
                    ("播放帧（累计）", "/output_frames"),
                    ("输出错误（累计）", "/output_stats/errors"),
                    (
                        "输出回调超预算（累计）",
                        "/output_stats/callback_over_budget",
                    ),
                    ("欠载帧（累计）", "/underrun_frames"),
                    ("限幅帧（累计）", "/limited_frames"),
                ] {
                    ui.label(format!(
                        "{label}：{}",
                        diagnostics
                            .pointer(pointer)
                            .map_or("未取得".into(), |v| v.to_string())
                    ));
                }
                if let Some(errors) = diagnostics.get("errors") {
                    ui.label(format!("输出故障：{errors}"));
                }
            });
            widgets::card(ui, "buffer", |ui| {
                ui.heading("缓冲与媒体网络");
                let ids = diagnostics.get("lane_stream_ids").and_then(Value::as_array);
                if let Some(queues) = diagnostics.get("queues").and_then(Value::as_array) {
                    for (index, q) in queues.iter().enumerate() {
                        if ids
                            .and_then(|i| i.get(index))
                            .is_some_and(|id| !id.is_null())
                        {
                            ui.label(format!(
                                "通道 {} · Mixer 队列估计 {:.1} ms",
                                ids.unwrap()[index],
                                q.as_f64().unwrap_or(0.0) / 48.0
                            ));
                        }
                    }
                }
                widgets::note(
                    ui,
                    "估计口径：队列帧 / 48 kHz；不含设备、网络与模拟端，不是端到端声音延迟。",
                );
                if let Some(receivers) = diagnostics.get("receivers").and_then(Value::as_array) {
                    for (i, r) in receivers.iter().enumerate() {
                        ui.label(format!("媒体接收器 {}", i + 1));
                        for (label, key) in [
                            ("丢包", "lost_packets"),
                            ("迟到包", "late_packets"),
                            ("PLC 样本", "plc_samples"),
                            ("PCM 缺口", "pcm_timing_gap_count"),
                            ("PCM 队列丢弃", "queue_drops"),
                        ] {
                            ui.label(format!(
                                "{label}（累计）：{}",
                                r.get(key).map_or("未取得".into(), |v| v.to_string())
                            ));
                        }
                    }
                }
            });
            if ui.button("导出脱敏诊断").clicked() {
                self.request(Request::ExportDiagnostics {
                    credential: self.credential.clone(),
                    hub: self.hub(),
                });
            }
            widgets::note(
                ui,
                format!(
                    "导出到 {}/diagnostics-redacted.json；仅保留诊断白名单数值和状态。",
                    self.client.state_dir().display()
                ),
            );
        } else {
            ui.label("诊断未取得。控制断线、未配对与输出设备故障需要分别处理。");
        }
        if let Some(status) = &self.status {
            widgets::card(ui, "capture", |ui| {
                ui.heading("Sender 采集");
                if let Some(metrics) = &status.sender.metrics {
                    for (label, pointer) in [
                        ("采集帧", "/capture_stats/frames"),
                        ("静音帧", "/capture_stats/silent_frames"),
                        ("无数据间隔", "/capture_stats/no_data_intervals"),
                        ("采集错误", "/capture_stats/errors"),
                        ("发送包", "/sent_packets"),
                        ("发包队列丢弃", "/queue_drops"),
                    ] {
                        ui.label(format!(
                            "{label}（累计）：{}",
                            metrics
                                .pointer(pointer)
                                .map_or("未取得".into(), |v| v.to_string())
                        ));
                    }
                    widgets::meter(
                        ui,
                        metrics
                            .pointer("/capture_stats/peak")
                            .and_then(Value::as_f64),
                        None,
                    );
                    widgets::note(
                        ui,
                        "采集 Peak 为本次会话累计最大值，RMS 未取得；Mixer 页面显示近期窗口。",
                    );
                } else {
                    ui.label("当前没有采集统计。开始发送后刷新。");
                }
                if let Some(error) = &status.sender.error {
                    ui.colored_label(theme::DANGER, error);
                }
            });
        }
    }
}
fn user_error(error: String) -> String {
    match error.as_str() {
        "revision_conflict" => "房间状态已被其他操作更新；正在刷新，请核对后重试。".into(),
        "permission_denied" => "当前身份无权执行此操作；请刷新设备权限。".into(),
        "unauthenticated" => "配对凭证不可用；检查是否已被撤销，必要时重新配对。".into(),
        "invalid_argument" => "参数无效；请检查字段和所选设备。".into(),
        "not_found" => "目标设备或通道已不存在；请刷新状态。".into(),
        "quota_exceeded" => "房间已达到设备或通道上限。".into(),
        _ => error,
    }
}
fn gain_input(
    ui: &mut egui::Ui,
    drafts: &mut std::collections::BTreeMap<u64, f32>,
    id: u64,
    current: f32,
    label: &str,
) -> Option<f32> {
    let mut result = None;
    let mut gain = drafts.get(&id).copied().unwrap_or(current);

    // 音量预设按钮
    if let Some(preset_gain) = widgets::volume_presets(ui, gain) {
        gain = preset_gain;
        result = Some(gain);
        drafts.remove(&id);
    }

    ui.add_space(8.0);

    // 音量滑块
    let response = widgets::gain_slider(ui, &mut gain, label);
    if response.changed() {
        drafts.insert(id, gain);
    }
    if response.drag_stopped() || response.changed() && !response.dragged() {
        drafts.remove(&id);
        result = Some(gain);
    }

    result
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
    #[test]
    fn pages_handle_missing_loading_failed_and_stale_states() {
        let ctx = egui::Context::default();
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
        let ctx = egui::Context::default();
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
}
