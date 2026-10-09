mod animation;
mod capture_permission;
mod commands;
#[cfg(test)]
mod commands_tests;
mod emblem;
mod events;
mod flow;
mod fx;
mod history;
#[cfg(test)]
mod i18n_tests;
mod icons;
mod intent;
mod localization;
mod measurement;
mod preferences;
use neonmix_i18n::{LanguagePreference, Message};
mod lanes;
mod pages;
mod palette;
mod shell;
#[cfg(feature = "screenshot")]
mod shot;
mod theme;
mod tray;
mod viz;
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
    #[arg(long, requires="preview_page", value_parser=["auto","zh-CN","en"])]
    preview_language: Option<String>,
    #[arg(long, requires = "preview_page")]
    preview_system_locale: Vec<String>,
    /// Appearance for preview verification; previews default to dark so
    /// captures do not depend on the OS appearance.
    #[arg(long, requires = "preview_page", value_parser = ["system", "dark", "light"])]
    preview_theme: Option<String>,
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
    #[arg(long, value_parser = ["live", "hub", "sender", "mixer", "devices", "diagnostics", "about", "airplay"])]
    preview_page: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Hash, Debug)]
enum Page {
    Live,
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
            Self::Live => icons::Icon::Flow,
            Self::Hub => icons::Icon::Room,
            Self::Sender => icons::Icon::Sender,
            Self::Mixer => icons::Icon::Mixer,
            Self::Devices => icons::Icon::Devices,
            Self::Diagnostics => icons::Icon::Pulse,
            Self::About => icons::Icon::Settings,
        }
    }
    const ALL: [(Self, Message); 7] = [
        (Self::Live, Message::ShellPageLive),
        (Self::Mixer, Message::ShellPageMixer),
        (Self::Hub, Message::ShellPageHub),
        (Self::Sender, Message::ShellPageSender),
        (Self::Devices, Message::ShellPageDevices),
        (Self::Diagnostics, Message::ShellPageDiagnostics),
        (Self::About, Message::ShellPageAbout),
    ];
    fn title(self) -> Message {
        Self::ALL.iter().find(|p| p.0 == self).unwrap().1.clone()
    }
}
/// Consecutive partial polls before the failure is shown.
const PARTIAL_REPORT: u32 = 3;
#[derive(Default, Clone, Copy)]
struct PollTimes {
    status: Option<Instant>,
    snapshot: Option<Instant>,
    diagnostics: Option<Instant>,
    airplay: Option<Instant>,
}
struct PollData {
    received: PollTimes,
    credential: PathBuf,
    hub: Option<String>,
    room_name: Option<String>,
    status: ServiceStatus,
    snapshot: Option<Snapshot>,
    diagnostics: Option<Value>,
    airplay: Option<Value>,
    /// The AirPlay or diagnostics read failed this time (the snapshot did
    /// not); the window keeps the previous readings instead of blanking.
    partial: bool,
    error: Option<UiError>,
}
#[allow(
    clippy::large_enum_variant,
    reason = "The one-slot UI queue keeps the complete frozen IPC envelope inline."
)]
enum Work {
    Boot,
    Poll {
        credential: PathBuf,
        hub: Option<String>,
    },
    Action(Request),
}
#[derive(Clone, Copy)]
enum LocalStop {
    Hub,
    Sender,
    Shutdown,
}
enum Outcome {
    Boot,
    Poll(Box<PollData>),
    Action(Box<Request>, Value),
}
fn call(client: &Client, request: &Request) -> Result<Value, UiError> {
    let reply = client.request(request).map_err(UiError::from)?;
    if reply.ok {
        let stop = match request {
            Request::LifecycleStop { request, .. } => request.as_ref(),
            other => other,
        };
        if matches!(
            stop,
            Request::HubStop | Request::SenderStop | Request::Shutdown
        ) {
            let identity = reply.data["instance_generation"]
                .as_str()
                .and_then(|v| uuid::Uuid::parse_str(v).ok());
            let operation = reply.data["operation_id"]
                .as_str()
                .and_then(|v| uuid::Uuid::parse_str(v).ok());
            let targets: &[&str] = match stop {
                Request::HubStop => &["hub"],
                Request::SenderStop => &["sender"],
                _ => &["hub", "sender"],
            };
            if identity.is_none()
                || matches!(request, Request::LifecycleStop { instance_generation, .. }
                    if identity != Some(*instance_generation))
                || operation.is_none()
                || targets
                    .iter()
                    .any(|key| reply.data[*key]["cleanup_complete"] != true)
            {
                return Err("invalid_backend_response".into());
            }
        }
        Ok(reply.data)
    } else {
        Err(UiError {
            fault: reply.fault.or_else(|| {
                reply
                    .error
                    .as_deref()
                    .and_then(neonmix_desktop_service::Fault::from_machine_code)
            }),
        })
    }
}
fn worker(
    client: Client,
    ctx: egui::Context,
) -> (SyncSender<Work>, Receiver<Result<Outcome, UiError>>) {
    let (tx, jobs) = mpsc::sync_channel(1);
    let (done, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        while let Ok(job) = jobs.recv() {
            let result = match job {
                Work::Boot => client
                    .ensure_background()
                    .map(|()| Outcome::Boot)
                    .map_err(UiError::from),
                Work::Action(request) => {
                    let result = capture_permission::before_action(&request)
                        .map_err(UiError::from)
                        .and_then(|()| call(&client, &request));
                    let original = match request {
                        Request::LifecycleStart { request, .. }
                        | Request::LifecycleStop { request, .. } => *request,
                        other => other,
                    };
                    result.map(|data| Outcome::Action(Box::new(original), data))
                }
                Work::Poll { credential, hub } => {
                    read_poll(credential, hub, |request| call(&client, &request))
                        .map(|data| Outcome::Poll(Box::new(data)))
                }
            };
            if done.send(result).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    (tx, rx)
}
/// Read optional observations before the authoritative control snapshot. A
/// slow diagnostics/helper process must not age the snapshot in the worker
/// queue before the UI can use it. Each observation retains its own timestamp.
fn read_poll(
    credential: PathBuf,
    hub: Option<String>,
    mut read: impl FnMut(Request) -> Result<Value, UiError>,
) -> Result<PollData, UiError> {
    let status = serde_json::from_value::<ServiceStatus>(read(Request::Status)?)
        .map_err(|_| UiError::from("invalid_backend_response"))?;
    let mut data = PollData {
        received: PollTimes {
            status: Some(Instant::now()),
            ..Default::default()
        },
        credential: credential.clone(),
        hub: hub.clone(),
        room_name: None,
        status,
        snapshot: None,
        diagnostics: None,
        airplay: None,
        partial: false,
        error: None,
    };
    if !fetches_room(&credential, &data.status) {
        return Ok(data);
    }
    match read(Request::AirplayV2 {
        credential: credential.clone(),
        hub: hub.clone(),
        command: None,
    }) {
        Ok(value) => {
            data.airplay = Some(value);
            data.received.airplay = Some(Instant::now());
        }
        Err(_) => data.partial = true,
    }
    match read(Request::Diagnostics {
        credential: credential.clone(),
        hub: hub.clone(),
    }) {
        Ok(value) => {
            data.diagnostics = Some(value);
            data.received.diagnostics = Some(Instant::now());
        }
        Err(error) => {
            data.partial = true;
            data.error = Some(error);
        }
    }
    match read(Request::Snapshot { credential, hub }) {
        Ok(value) => {
            data.room_name = value
                .pointer("/viewer/room_name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match serde_json::from_value(value) {
                Ok(snapshot) => {
                    data.snapshot = Some(snapshot);
                    data.received.snapshot = Some(Instant::now());
                }
                Err(_) => {
                    data.partial = false;
                    data.error = Some("invalid_backend_response".into());
                }
            }
        }
        Err(error) => {
            data.partial = false;
            data.error = Some(error);
        }
    }
    if let Some(snapshot) = &data.snapshot {
        let epoch = snapshot.runtime_epoch.to_string();
        for observation in [&mut data.airplay, &mut data.diagnostics] {
            let replaced = observation.as_ref().is_some_and(|value| {
                value
                    .get("remote")
                    .unwrap_or(value)
                    .get("runtime_epoch")
                    .is_some_and(|value| value.as_str() != Some(epoch.as_str()))
            });
            if replaced {
                *observation = None;
                data.partial = true;
            }
        }
    }
    Ok(data)
}

/// A write the user made while the previous one was still in flight. Only
/// the latest intent is kept and it is sent against the next fresh revision,
/// so rapid input neither conflicts nor greys out the controls.
#[derive(Clone, PartialEq)]
enum Write {
    Control(Operation),
    Airplay(neonmix_airplay_adapter::control::AirplayActionV2),
}
/// The last mixer change, offered as 还原 (restore) for a few seconds. Named
/// apart from 撤销 (revoke a pairing) so the two can never be confused.
struct Undo {
    label: Message,
    inverse: Write,
    after: Write,
    context: intent::ContextKey,
    target: intent::TargetKey,
    request_id: uuid::Uuid,
    at: Instant,
}
struct Confirmation {
    label: Message,
    consequence: Message,
    request: Request,
    intent: Option<intent::Intent>,
    focus: bool,
    _origin: egui::Id,
}
impl Confirmation {
    fn action_label(&self) -> Message {
        match &self.request {
            Request::Shutdown => Message::ShellQuit,
            Request::Control {
                operation: Operation::Revoke { .. },
                ..
            } => Message::ShellRevoke,
            Request::ForgetCredential { .. } => Message::ShellForget,
            Request::Output {
                action: OutputAction::Remove { .. },
                ..
            } => Message::ShellRemoveOutput,
            _ => self.label.clone(),
        }
    }
}
struct Desktop {
    preferences: preferences::Preferences,
    localization: localization::Localization,
    page: Page,
    shown_message: Option<Message>,
    /// Whether the message on screen is an error (repeated errors swap text
    /// without re-running the fade-in).
    shown_error: bool,
    message_since: Instant,
    client: Client,
    worker: Option<SyncSender<Work>>,
    results: Option<Receiver<Result<Outcome, UiError>>>,
    busy: bool,
    inflight_start: bool,
    inflight_stop: bool,
    polling: bool,
    pending_action: Option<Request>,
    pending_stop: bool,
    urgent_action: Option<Receiver<Result<Value, UiError>>>,
    urgent_target: LocalStop,
    repaint: egui::Context,
    online: bool,
    message: Message,
    error: bool,
    cjk: bool,
    status: Option<ServiceStatus>,
    snapshot: Option<Snapshot>,
    diagnostics: Option<Value>,
    airplay: Option<Value>,
    diagnostics_clock: measurement::ObservationClock,
    airplay_clock: measurement::ObservationClock,
    sender_clock: measurement::ObservationClock,
    diagnostic_counters: measurement::CounterHistory,
    sender_counters: measurement::CounterHistory,
    fresh: Option<Instant>,
    /// Last authoritative snapshot; unlike `fresh` our own writes do not clear
    /// it, so controls stay enabled (and do not flash) while a write settles.
    synced: Option<Instant>,
    intents: intent::IntentScheduler,
    applications: std::collections::VecDeque<intent::ApplicationWatch>,
    context_epoch: u64,
    context_route: Option<(PathBuf, Option<String>)>,
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
    gain_draft_owners: std::collections::BTreeMap<u64, (intent::ContextKey, intent::TargetKey)>,
    /// Mixer lane under keyboard control, keyed by the actual stream id.
    selected_lane: Option<u64>,
    lane_details: std::collections::BTreeSet<u64>,
    /// User choices for collapsible panels; absent keys follow data defaults.
    panels: std::collections::HashMap<&'static str, bool>,
    device_filter: usize,
    /// Panel to bring into view on the next frame (stepper / tile clicks).
    scroll_to: Option<&'static str>,
    palette: Option<palette::Palette>,
    palette_since: Instant,
    undo: Option<Undo>,
    tray: Option<tray::Tray>,
    tray_attempted: bool,
    native_window: tray::NativeWindow,
    events: std::collections::VecDeque<events::RoomEvent>,
    marks: Option<events::Marks>,
    /// When a lane first joined while this window watched (connect animation).
    joined: std::collections::BTreeMap<u64, Instant>,
    /// Consecutive polls whose AirPlay or diagnostics read failed.
    partial_polls: u32,
    /// Last minute of per-lane RMS (dBFS) for the Mixer ribbon.
    level_history: history::Series<u64>,
    /// Last two minutes of room metrics for diagnostics sparklines.
    metric_history: history::Series<&'static str>,
    marks_key: Option<(uuid::Uuid, u64, Option<u64>, usize)>,
    #[cfg(feature = "screenshot")]
    shot: Option<shot::Shot>,
}
impl Desktop {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let preview = args.preview_page.is_some();
        let client = Client::new(args.state_dir);
        let mut preferences = if preview {
            preferences::Preferences::memory()
        } else {
            preferences::Preferences::load(Some(client.state_dir()))
        };
        if let Some(language) = args.preview_language.as_deref() {
            preferences.value.language = if language == "auto" {
                LanguagePreference::Auto
            } else {
                LanguagePreference::Explicit(language.into())
            };
        }
        if preview {
            preferences.value.theme = match args.preview_theme.as_deref() {
                Some("system") => preferences::ThemeChoice::System,
                Some("light") => preferences::ThemeChoice::Light,
                _ => preferences::ThemeChoice::Dark,
            };
        }
        let candidates = if preview {
            Some(args.preview_system_locale)
        } else {
            localization::SystemLocaleProvider::candidates(&localization::NativeLocaleProvider).ok()
        };
        let localization = localization::Localization::new(&preferences.value.language, candidates);
        let cjk = theme::install(&cc.egui_ctx);
        cc.egui_ctx.set_theme(preferences.value.theme.preference());
        let mut app = Self::empty(client, cjk, preview);
        app.preferences = preferences;
        app.localization = localization;
        localization::install(&cc.egui_ctx, app.localization.renderer.clone());
        app.room = app.tr(&Message::ShellDefaultRoom);
        app.sender_name = app.tr(&Message::ShellDefaultSender);
        app.binding_name = format!("NeonMix — {}", app.room);
        if app.preferences.read_failed {
            app.message = Message::PreferencesReadFailed;
            app.error = true;
        }
        app.repaint = cc.egui_ctx.clone();
        app.native_window = tray::NativeWindow::of(cc);
        animation::set_reduce_motion(
            std::env::var("NEONMIX_REDUCE_MOTION").is_ok_and(|v| v == "1")
                || app.preferences.value.reduce_motion,
        );
        if let Some(page) = args.preview_page {
            app.preview_airplay_only = page == "airplay";
            app.page = match page.as_str() {
                "live" => Page::Live,
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
        // Preview data first: screenshot states (还原) build on its snapshot.
        if app.preview {
            app.message = Message::ShellPreview;
            if let Some(path) = args.preview_data {
                match std::fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                {
                    Some(data) => app.load_preview(&data),
                    None => {
                        app.error = true;
                        app.message = Message::ShellPreviewReadFailed;
                    }
                }
            }
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
                app.sync_command_context();
                if let Some(context) = app.intents.current.clone() {
                    app.undo = Some(Undo {
                        context,
                        target: intent::TargetKey::Master,
                        request_id: uuid::Uuid::new_v4(),
                        after: Write::Control(Operation::OutputMix {
                            gain_db: Some(app.snapshot.as_ref().unwrap().output.gain_db),
                            muted: None,
                        }),
                        label: Message::ShellMasterGain {
                            previous: "0.0".into(),
                            next: "+3.5".into(),
                        },
                        inverse: Write::Control(Operation::OutputMix {
                            gain_db: None,
                            muted: None,
                        }),
                        at: Instant::now() + Duration::from_secs(30),
                    });
                    // 还原 is offered only for an acknowledged write.
                    let undo = app.undo.as_ref().unwrap();
                    app.intents.history.push_back(intent::Intent {
                        id: undo.request_id,
                        context: undo.context.clone(),
                        route: intent::Route {
                            credential: app.credential.clone(),
                            hub: app.hub(),
                        },
                        target: undo.target.clone(),
                        write: undo.after.clone(),
                        label: Some(undo.label.clone()),
                        guard: None,
                        frozen: None,
                        created_at: Instant::now(),
                        phase: intent::Phase::Acknowledged,
                    });
                }
            }
            if std::env::var_os("NEONMIX_SCREENSHOT_CONFIRM").is_some() {
                app.confirmation(
                    Message::ShellQuit,
                    Message::ShellQuitConsequence,
                    Request::Shutdown,
                    egui::Id::new("preview-confirm"),
                );
            }
        }
        app
    }
    pub(crate) fn set_theme_choice(
        &mut self,
        ctx: &egui::Context,
        choice: preferences::ThemeChoice,
    ) {
        ctx.set_theme(choice.preference());
        self.preferences.theme(choice);
        if self.preferences.save().is_err() {
            self.message = Message::PreferencesSaveFailed;
            self.error = true;
        }
    }
    pub(crate) fn set_reduce_motion(&mut self, on: bool) {
        animation::set_reduce_motion(on);
        self.preferences.motion(on);
        if self.preferences.save().is_err() {
            self.message = Message::PreferencesSaveFailed;
            self.error = true;
        }
    }

    /// Recorded public state for preview builds and tests.
    fn load_preview(&mut self, data: &Value) {
        self.devices = serde_json::from_value(data["devices"].clone()).unwrap_or_default();
        self.snapshot = serde_json::from_value(data["snapshot"].clone()).ok();
        self.status = serde_json::from_value(data["status"].clone()).ok();
        if let Some(settings) = self.status.as_ref().and_then(|s| s.hub_settings.as_ref()) {
            self.room = settings.name.clone();
            self.output = settings.output.clone();
        }
        self.remote_room = data["snapshot"]
            .pointer("/viewer/room_name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.diagnostics = data.get("diagnostics").cloned();
        self.airplay = data.get("airplay").cloned();
        let epoch = self.snapshot.as_ref().map(|s| s.runtime_epoch);
        let now = Instant::now();
        if let Some(value) = &self.diagnostics {
            self.diagnostics_clock.success(value, epoch, now);
        }
        if let Some(value) = &self.airplay {
            self.airplay_clock.success(value, epoch, now);
        }
        if let Some(value) = self.status.as_ref().and_then(|s| s.sender.metrics.as_ref()) {
            self.sender_clock.success(value, None, now);
        }
        #[cfg(feature = "screenshot")]
        {
            self.airplay_name_drafts =
                serde_json::from_value(data["preview_name_drafts"].clone()).unwrap_or_default();
            if let Some(error) = data["preview_error"].as_str() {
                self.error = true;
                self.message = user_error(UiError::from(error.to_owned()));
            }
        }
        self.binding = self.status.as_ref().and_then(|s| s.output_binding.clone());
        self.fresh = Some(Instant::now());
        self.synced = self.fresh;
        #[cfg(feature = "screenshot")]
        if let Some(state) = data["preview_application"].as_str() {
            self.sync_command_context();
            if let (Some(context), Some(snapshot)) =
                (self.intents.current.clone(), self.snapshot.as_ref())
            {
                let status = match state {
                    "stalled" => intent::ApplicationState::Stalled,
                    "unknown" => intent::ApplicationState::Unknown,
                    _ => intent::ApplicationState::Pending,
                };
                let intent = intent::Intent {
                    id: uuid::Uuid::new_v4(),
                    context,
                    route: intent::Route {
                        credential: self.credential.clone(),
                        hub: self.hub(),
                    },
                    target: intent::TargetKey::Master,
                    write: Write::Control(Operation::OutputMix {
                        gain_db: Some(snapshot.output.gain_db),
                        muted: None,
                    }),
                    label: None,
                    guard: None,
                    frozen: None,
                    created_at: Instant::now(),
                    phase: intent::Phase::Acknowledged,
                };
                self.applications.push_back(intent::ApplicationWatch {
                    intent,
                    sequence: 2,
                    state: status,
                });
                self.message = match status {
                    intent::ApplicationState::Stalled => Message::ShellMediaStalled,
                    intent::ApplicationState::Unknown => Message::ShellMediaUnknown,
                    _ => Message::ShellMediaPending,
                };
                self.error = status != intent::ApplicationState::Pending;
            }
        }
    }
    fn empty(client: Client, cjk: bool, preview: bool) -> Self {
        Self {
            preferences: preferences::Preferences::memory(),
            localization: localization::Localization::new(
                &LanguagePreference::Auto,
                Some(vec!["zh-CN".into()]),
            ),
            page: Page::Live,
            shown_message: None,
            shown_error: false,
            message_since: Instant::now() - Duration::from_secs(5),
            client,
            worker: None,
            results: None,
            busy: false,
            inflight_start: false,
            inflight_stop: false,
            polling: false,
            pending_action: None,
            pending_stop: false,
            urgent_action: None,
            urgent_target: LocalStop::Sender,
            repaint: egui::Context::default(),
            online: false,
            message: Message::ShellConnecting,
            error: false,
            cjk,
            status: None,
            snapshot: None,
            diagnostics: None,
            airplay: None,
            diagnostics_clock: Default::default(),
            airplay_clock: Default::default(),
            sender_clock: Default::default(),
            diagnostic_counters: Default::default(),
            sender_counters: Default::default(),
            fresh: None,
            synced: None,
            intents: Default::default(),
            applications: Default::default(),
            context_epoch: 0,
            context_route: None,
            inflight: None,
            poll_error: false,
            copied_at: None,
            last_poll: Instant::now() - Duration::from_secs(5),
            devices: vec![],
            room: String::new(),
            remote_room: None,
            output: String::new(),
            sender_name: String::new(),
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
            binding_name: String::new(),
            provider: "neonmix".into(),
            search: String::new(),
            confirm: None,
            focus_after_modal: false,
            exiting: false,
            preview,
            preview_airplay_only: false,
            gain_drafts: std::collections::BTreeMap::new(),
            gain_draft_owners: Default::default(),
            selected_lane: None,
            lane_details: Default::default(),
            panels: Default::default(),
            device_filter: 0,
            scroll_to: None,
            palette: None,
            palette_since: Instant::now() - Duration::from_secs(5),
            undo: None,
            tray: None,
            tray_attempted: false,
            native_window: Default::default(),
            events: Default::default(),
            marks: None,
            marks_key: None,
            joined: Default::default(),
            partial_polls: 0,
            level_history: history::Series::new(Duration::from_secs(60)),
            metric_history: history::Series::new(Duration::from_secs(120)),
            #[cfg(feature = "screenshot")]
            shot: None,
        }
    }
    fn queue(&mut self, work: Work) {
        if self.busy || self.preview {
            return;
        }
        let polling = matches!(&work, Work::Poll { .. });
        let starting = matches!(&work, Work::Action(Request::SenderStart { .. }))
            || matches!(&work,Work::Action(Request::LifecycleStart {request,..}) if matches!(request.as_ref(),Request::SenderStart {..}));
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
        if self.exiting || (self.pending_stop && matches!(self.urgent_target, LocalStop::Shutdown))
        {
            return;
        }
        // A stop cancels a start which has not reached the worker yet. The
        // frozen generation is still required for starts already in flight.
        if let Some(queued) = &self.pending_action {
            let cancel = match &request {
                Request::HubStop => request_key(queued) == "hub-start",
                Request::SenderStop => request_key(queued) == "sender-start",
                Request::Shutdown => true,
                _ => false,
            };
            if cancel {
                self.pending_action = None;
            }
        }
        match &request {
            Request::Control { operation, .. } => {
                self.write(Write::Control(operation.clone()));
                return;
            }
            Request::AirplayV2 {
                command: Some(command),
                ..
            } => {
                self.write(Write::Airplay(command.operation.clone()));
                return;
            }
            _ => {}
        }
        if matches!(
            &request,
            Request::Shutdown | Request::HubStop | Request::SenderStop
        ) {
            self.begin_urgent(request);
            return;
        }
        if self.preview {
            return;
        }
        let request = match self.freeze_lifecycle(request) {
            Ok(request) => request,
            Err(error) => {
                self.message = user_error(error);
                self.error = true;
                return;
            }
        };
        if self.busy {
            if self.pending(request_key(&request)) {
                return;
            }
            if self.pending_action.is_some() {
                self.message = Message::IntentQueueFull;
                self.error = true;
                return;
            }
            // Keep one frozen follow-up while any worker operation runs, not
            // just a poll. Otherwise enabled buttons silently lose clicks
            // during the slower platform startup/device enumeration calls.
            self.pending_action = Some(request);
        } else {
            self.queue(Work::Action(request));
        }
        self.message = Message::ShellProcessing;
        self.error = false;
    }
    fn stop_sender(&mut self) {
        if self.preview || self.pending_stop {
            return;
        }
        if matches!(&self.pending_action, Some(Request::SenderStart { .. }))
            || matches!(&self.pending_action,Some(Request::LifecycleStart {request,..}) if matches!(request.as_ref(),Request::SenderStart {..}))
        {
            self.pending_action = None;
        }
        if self.busy {
            self.begin_urgent(Request::SenderStop);
        } else {
            self.request(Request::SenderStop);
        }
    }
    fn begin_urgent(&mut self, request: Request) {
        // Quit supersedes a local Hub/Sender stop. Its owner continues even
        // when we replace the UI subscription; only the quit reply may close UI.
        if self.preview
            || (self.pending_stop
                && (!matches!(request, Request::Shutdown)
                    || matches!(self.urgent_target, LocalStop::Shutdown)))
        {
            return;
        }
        let target = match &request {
            Request::Shutdown => LocalStop::Shutdown,
            Request::HubStop => LocalStop::Hub,
            _ => LocalStop::Sender,
        };
        let request = match self.freeze_lifecycle(request) {
            Ok(request) => request,
            Err(error) => {
                self.message = stop_error(error);
                self.error = true;
                return;
            }
        };
        self.pending_stop = true;
        self.urgent_target = target;
        self.error = false;
        self.message = if matches!(self.urgent_target, LocalStop::Shutdown) {
            Message::ShellQuitting
        } else {
            Message::ShellStopping
        };
        let client = self.client.clone();
        let repaint = self.repaint.clone();
        let window = self.native_window;
        let quitting = matches!(target, LocalStop::Shutdown);
        let (tx, rx) = mpsc::sync_channel(1);
        self.urgent_action = Some(rx);
        std::thread::spawn(move || {
            let result = call(&client, &request);
            if tx.send(result).is_ok() {
                // A user may hide/minimize while waiting. Windows does not
                // render hidden windows: wake it to consume success or retry.
                if quitting {
                    window.show();
                }
                repaint.request_repaint();
            }
        });
    }
    fn freeze_lifecycle(&self, request: Request) -> Result<Request, UiError> {
        if !matches!(
            request,
            Request::HubStart
                | Request::SenderStart { .. }
                | Request::HubStop
                | Request::SenderStop
                | Request::Shutdown
        ) {
            return Ok(request);
        }
        let status = self
            .status
            .as_ref()
            .ok_or_else(|| UiError::from("background_timeout"))?;
        let view = status
            .lifecycle
            .as_ref()
            .filter(|view| view.version == 1)
            .ok_or_else(|| UiError::from("ipc_incompatible_version"))?;
        if matches!(request, Request::HubStart | Request::SenderStart { .. }) {
            let expected_stop_generation = if matches!(request, Request::HubStart) {
                view.hub_stop_generation
            } else {
                view.sender_stop_generation
            };
            Ok(Request::LifecycleStart {
                instance_generation: view.instance_generation,
                expected_stop_generation,
                request: Box::new(request),
            })
        } else {
            Ok(Request::LifecycleStop {
                instance_generation: view.instance_generation,
                request: Box::new(request),
            })
        }
    }
    fn close_if_exiting(&self, ctx: &egui::Context) -> bool {
        if self.exiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
        }
        self.exiting
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
        self.sync_command_context();
        let outcome = self.results.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(outcome) = outcome {
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
            if !self.consume_intent_result(&outcome) {
                match outcome {
                    Err(e) => {
                        if was_poll {
                            self.diagnostics_clock.failed();
                            self.airplay_clock.failed();
                            self.sender_clock.failed();
                        }
                        self.message = user_error(e);
                        self.error = true;
                        self.fresh = None;
                        // A failed poll keeps the last readings on screen (marked
                        // stale by age) instead of blanking meters and cards.
                        if was_poll {
                            self.poll_error = true;
                        } else {
                            self.gain_drafts.clear();
                            // Other objects/fields remain queued after a failed write.
                        }
                    }
                    Ok(Outcome::Boot) => {
                        self.online = true;
                        self.message = Message::ShellConnected;
                        self.request(Request::Devices);
                    }
                    Ok(Outcome::Poll(data)) => {
                        let sender_replaced = self.status.as_ref().is_some_and(|old| {
                            old.sender.pid != data.status.sender.pid
                                || old.sender.owner_pid != data.status.sender.owner_pid
                                || old.lifecycle.as_ref().map(|s| s.instance_generation)
                                    != data
                                        .status
                                        .lifecycle
                                        .as_ref()
                                        .map(|s| s.instance_generation)
                        });
                        if sender_replaced {
                            self.sender_counters.clear();
                            self.sender_clock.clear();
                        }
                        if let Some(metrics) = data.status.sender.metrics.as_ref()
                            && let Some(age) = data.status.sender.metrics_age_ms
                        {
                            let new_metrics = sender_replaced
                                || self.status.as_ref().is_none_or(|old| {
                                    old.sender.metrics_sequence
                                        != data.status.sender.metrics_sequence
                                });
                            if new_metrics {
                                self.sender_counters.update(metrics);
                            }
                            self.sender_clock.success_with_age(
                                metrics,
                                None,
                                data.received.status.unwrap_or_else(Instant::now),
                                Duration::from_millis(age),
                            );
                        } else {
                            self.sender_clock.failed();
                        }
                        let application = data
                            .diagnostics
                            .as_ref()
                            .and_then(|v| v.get("remote").unwrap_or(v).get("media_application"))
                            .or_else(|| {
                                data.airplay
                                    .as_ref()
                                    .and_then(|v| v.get("media_application"))
                            })
                            .cloned();
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
                        if data.credential == self.credential
                            && data.hub == self.hub()
                            && (data.snapshot.is_some()
                                || !fetches_room(&data.credential, &data.status))
                        {
                            self.remote_room = data.room_name;
                        }
                        self.binding = data.status.output_binding.clone();
                        let outdated = self
                            .status
                            .as_ref()
                            .and_then(|current| current.lifecycle.as_ref())
                            .zip(data.status.lifecycle.as_ref())
                            .is_some_and(|(current, incoming)| {
                                current.instance_generation == incoming.instance_generation
                                    && (incoming.hub_stop_generation < current.hub_stop_generation
                                        || incoming.sender_stop_generation
                                            < current.sender_stop_generation)
                            });
                        if !outdated {
                            self.status = Some(data.status);
                        }
                        if let Some(snapshot) = data.snapshot.filter(|_| {
                            !outdated
                                && data.credential == self.credential
                                && data.hub == self.hub()
                        }) {
                            // A single failed AirPlay/diagnostics read keeps the
                            // last readings of the same room: blanking them made
                            // AirPlay lanes, meters and the graph vanish for one
                            // poll and come back on the next.
                            let same_room = self.snapshot.as_ref().is_some_and(|s| {
                                s.hub_id == snapshot.hub_id
                                    && s.runtime_epoch == snapshot.runtime_epoch
                            });

                            self.partial_polls = if data.partial {
                                self.partial_polls + 1
                            } else {
                                0
                            };
                            let now = Instant::now();
                            let epoch = Some(snapshot.runtime_epoch);
                            if !same_room {
                                self.diagnostic_counters.clear();
                                self.diagnostics_clock.clear();
                                self.airplay_clock.clear();
                            }
                            match data
                                .diagnostics
                                .as_ref()
                                .map(|v| v.get("remote").unwrap_or(v))
                            {
                                Some(value) => self.diagnostics_clock.success(
                                    value,
                                    epoch,
                                    data.received.diagnostics.unwrap_or(now),
                                ),
                                None => self.diagnostics_clock.failed(),
                            }
                            match data.airplay.as_ref() {
                                Some(value) => self.airplay_clock.success(
                                    value,
                                    epoch,
                                    data.received.airplay.unwrap_or(now),
                                ),
                                None => self.airplay_clock.failed(),
                            }
                            if let Some(value) = data
                                .diagnostics
                                .as_ref()
                                .map(|v| v.get("remote").unwrap_or(v))
                                && value["available"].as_bool() != Some(false)
                            {
                                self.diagnostic_counters.update(value);
                            }
                            self.snapshot = Some(snapshot);
                            if data
                                .airplay
                                .as_ref()
                                .is_some_and(|v| v["available"].as_bool() != Some(false))
                            {
                                self.airplay = data.airplay;
                            } else if !same_room {
                                self.airplay = None;
                            }
                            self.fresh = Some(data.received.snapshot.unwrap_or(now));
                            self.synced = self.fresh;
                            if self.poll_error {
                                self.poll_error = false;
                                if data.error.is_none() {
                                    self.error = false;
                                    self.message = Message::ShellRecovered;
                                }
                            }
                            if data.diagnostics.as_ref().is_some_and(|v| {
                                v.get("remote").unwrap_or(v)["available"].as_bool() != Some(false)
                            }) {
                                self.diagnostics = data
                                    .diagnostics
                                    .map(|v| v.get("remote").cloned().unwrap_or(v));
                            } else if !same_room {
                                self.diagnostics = None;
                            }
                            self.record_history();
                            self.sync_command_context();
                            self.refresh_applications(application.as_ref());
                        } else {
                            self.fresh = None;
                            self.poll_error = data.error.is_some();
                            // A transient refresh failure is not a new room or
                            // revoked permission. Keep the last presentation;
                            // sending still requires a new authoritative read.
                            let transient =
                                data.error.as_ref().is_some_and(UiError::transient_read);
                            let same_route =
                                data.credential == self.credential && data.hub == self.hub();
                            let still_running = self
                                .status
                                .as_ref()
                                .is_some_and(|s| fetches_room(&self.credential, s));
                            if outdated || !same_route || !still_running || !transient {
                                self.synced = None;
                                self.airplay = None;
                                self.diagnostics = None;
                            }
                            self.airplay_clock.failed();
                            self.diagnostics_clock.failed();
                        }
                        // Partial reads are reported once they persist; a lone
                        // miss would only flash the status strip red.
                        if let Some(e) = data
                            .error
                            .filter(|_| !data.partial || self.partial_polls >= PARTIAL_REPORT)
                        {
                            self.message = user_error(e);
                            self.error = true;
                            // Optional telemetry errors do not invalidate the
                            // independently successful control snapshot.
                        }
                    }
                    Ok(Outcome::Action(request, data)) => {
                        let request = *request;
                        // Mixer and AirPlay writes show their result in the control
                        // itself; a status line per click is noise.
                        if !matches!(request, Request::Control { .. } | Request::AirplayV2 { .. }) {
                            self.message = Message::ShellCompleted;
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
                                        self.message = Message::ShellPartialConfiguration;
                                        self.error = true;
                                    }
                                    Some("saved_session_ended") => {
                                        self.message = Message::ShellSessionEnded
                                    }
                                    _ => {}
                                }
                                if let Some(warning) = data["warning"].as_str() {
                                    self.message = if warning == "profile_durability_unconfirmed" {
                                        Message::ShellDurability
                                    } else {
                                        Message::ShellEntryPending
                                    };
                                    self.error = true;
                                } else if data["media_pending"].as_bool() == Some(true) {
                                    self.message = Message::ShellMediaPending;
                                }
                                self.airplay = Some(data);
                            }
                            Request::Devices => match serde_json::from_value(data) {
                                Ok(devices) => self.devices = devices,
                                Err(_e) => {
                                    self.error = true;
                                    self.message = Message::FaultInvalidBackendResponse;
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
                                self.message = Message::ShellExported
                            }
                            Request::ForgetCredential { .. } => {
                                self.snapshot = None;
                                self.fresh = None;
                                self.synced = None;
                                self.invitation.clear();
                                self.message = Message::ShellForgotPairing;
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
            }
        }
        if let Some(result) = self
            .urgent_action
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("lifecycle_owner_lost".into())),
            })
        {
            self.urgent_action = None;
            self.pending_stop = false;
            match result {
                Ok(data) => {
                    if matches!(self.urgent_target, LocalStop::Shutdown) {
                        self.exiting = true;
                        self.pending_action = None;
                        return;
                    }
                    self.message = if matches!(self.urgent_target, LocalStop::Hub) {
                        Message::ShellSharingStopped
                    } else {
                        Message::ShellStopped
                    };
                    self.error = false;
                    self.last_poll = Instant::now() - Duration::from_secs(5);
                    if let Some(status) = &mut self.status {
                        let process = match self.urgent_target {
                            LocalStop::Hub => &mut status.hub,
                            _ => &mut status.sender,
                        };
                        process.running = false;
                        process.ready = false;
                        process.pid = None;
                        if let Ok(view) = serde_json::from_value(data["lifecycle"].clone()) {
                            status.lifecycle = Some(view);
                        }
                    }
                }
                Err(error) => {
                    self.message = stop_error(error);
                    self.error = true;
                }
            }
        }
        if self.pending_stop {
            self.message = if matches!(self.urgent_target, LocalStop::Shutdown) {
                Message::ShellQuitting
            } else {
                Message::ShellStopping
            };
            self.error = false;
            return;
        }
        if !self.busy
            && !self.pending_stop
            && let Some(request) = self.pending_action.take()
        {
            self.request(request);
        }
        self.sync_command_context();
        self.dispatch_intents();
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
        self.synced.is_some_and(|t| {
            let age = t.elapsed();
            age < Duration::from_secs(4)
                    // Keep accepting bound intentions while a bounded request
                    // is in flight. `ready`/dispatch still require <4s data;
                    // this grace never authorizes a stale write.
                    || (self.busy && !self.poll_error && age < neonmix_desktop_service::IPC_TIMEOUT)
        }) && !self.pending_stop
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
        self.enqueue_write(write, None);
    }
    fn panel_open(&self, key: &'static str, default: bool) -> bool {
        self.panels.get(key).copied().unwrap_or(default)
    }
    fn toggle_panel(&mut self, key: &'static str, open: bool) {
        self.panels.insert(key, !open);
    }
    /// A mixer change the user can restore with one click or ⌘Z.
    fn mix_change(&mut self, label: Message, write: Write, _inverse: Write) {
        self.enqueue_write(write, Some(label));
    }
    fn undo_available(&self) -> bool {
        self.undo_matches()
    }
    fn restore_last(&mut self) {
        self.restore_confirmed();
    }
    fn confirmation(
        &mut self,
        label: Message,
        consequence: Message,
        request: Request,
        origin: egui::Id,
    ) {
        let intent = self.confirmation_intent(&request);
        if intent.is_none()
            && matches!(
                &request,
                Request::Control { .. }
                    | Request::AirplayV2 {
                        command: Some(_),
                        ..
                    }
            )
        {
            self.message = Message::IntentTargetEnded;
            self.error = true;
            return;
        }
        self.confirm = Some(Confirmation {
            intent,
            label,
            consequence,
            request,
            focus: true,
            _origin: origin,
        });
    }
}
/// Whether a poll should ask for room state as `credential`. The local
/// admin identity can only reach this machine's Hub; while it is not sharing
/// there is nothing to ask, and asking showed a permanent red connection
/// error that alternated between causes every second.
fn fetches_room(credential: &std::path::Path, status: &ServiceStatus) -> bool {
    let local_hub_stopped =
        credential == std::path::Path::new("hub/admin.json") && !status.hub.running;
    !local_hub_stopped
        && status
            .profiles
            .iter()
            .any(|p| p.credential == credential && !p.pending)
}

fn request_key(request: &Request) -> &'static str {
    match request {
        Request::LifecycleStart { request, .. } | Request::LifecycleStop { request, .. } => {
            request_key(request)
        }
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
#[derive(Debug, Clone)]
struct UiError {
    fault: Option<neonmix_desktop_service::Fault>,
}
impl UiError {
    fn transient_read(&self) -> bool {
        use neonmix_desktop_service::FaultCode;
        self.fault.as_ref().is_some_and(|fault| {
            matches!(
                fault.code,
                FaultCode::ConnectionUnavailable
                    | FaultCode::BackgroundTimeout
                    | FaultCode::BackgroundBusy
                    | FaultCode::RequestInterrupted
                    | FaultCode::CredentialStoreBusy
            )
        })
    }
}
impl From<&str> for UiError {
    fn from(error: &str) -> Self {
        Self::from(error.to_owned())
    }
}
impl From<String> for UiError {
    fn from(error: String) -> Self {
        Self {
            fault: neonmix_desktop_service::Fault::from_machine_code(&error),
        }
    }
}
fn stop_error(error: UiError) -> Message {
    use neonmix_desktop_service::FaultCode;
    if error.fault.as_ref().is_none_or(|fault| {
        matches!(
            fault.code,
            FaultCode::ConnectionUnavailable
                | FaultCode::BackgroundTimeout
                | FaultCode::BackgroundBusy
                | FaultCode::LifecycleOwnerLost
                | FaultCode::InvalidBackendResponse
                | FaultCode::RequestInterrupted
        )
    }) {
        Message::ShellStopUnconfirmed
    } else {
        user_error(error)
    }
}

fn user_error(error: UiError) -> Message {
    use neonmix_desktop_service::FaultCode;
    let Some(fault) = error.fault else {
        return Message::ErrorGeneric;
    };
    match fault.code {
        FaultCode::GenericFailure => Message::FaultGenericFailure,
        FaultCode::RevisionConflict => Message::FaultRevisionConflict,
        FaultCode::PermissionDenied => Message::FaultPermissionDenied,
        FaultCode::Unauthenticated => Message::FaultUnauthenticated,
        FaultCode::InvalidArgument => Message::FaultInvalidArgument,
        FaultCode::NotFound => Message::FaultNotFound,
        FaultCode::QuotaExceeded => Message::FaultQuotaExceeded,
        FaultCode::ReceiverBusy => Message::FaultReceiverBusy,
        FaultCode::RoomCapacityFull => Message::FaultRoomCapacityFull,
        FaultCode::SourceAlreadyActive => Message::FaultSourceAlreadyActive,
        FaultCode::SourceBlocked => Message::FaultSourceBlocked,
        FaultCode::PairingRevoked => Message::FaultPairingRevoked,
        FaultCode::StaleRevision => Message::FaultStaleRevision,
        FaultCode::SessionChanged => Message::FaultSessionChanged,
        FaultCode::OutputUnavailable => Message::FaultOutputUnavailable,
        FaultCode::OutputObjectReplaced => Message::FaultOutputObjectReplaced,
        FaultCode::WorkerUnavailable => Message::FaultWorkerUnavailable,
        FaultCode::UpgradeRequired => Message::FaultUpgradeRequired,
        FaultCode::HubPortInUse => Message::FaultHubPortInUse {
            port: u64::from(fault.params.port.unwrap_or(7443)),
        },
        FaultCode::MigrationRequired => Message::FaultMigrationRequired,
        FaultCode::CredentialMissing => Message::FaultCredentialMissing,
        FaultCode::CredentialPermissionDenied => Message::FaultCredentialPermissionDenied,
        FaultCode::CredentialStoreBusy => Message::FaultCredentialStoreBusy,
        FaultCode::AdministratorCredentialProtected => {
            Message::FaultAdministratorCredentialProtected
        }
        FaultCode::CredentialIoFailed => Message::FaultCredentialIoFailed,
        FaultCode::ProfileDurabilityUnconfirmed => Message::FaultProfileDurabilityUnconfirmed,
        FaultCode::ConfigurationRecoveryRequired => Message::FaultConfigurationRecoveryRequired,
        FaultCode::CredentialCorrupt => Message::FaultCredentialCorrupt,
        FaultCode::CredentialVersionUnsupported => Message::FaultCredentialVersionUnsupported,
        FaultCode::CredentialKindMismatch => Message::FaultCredentialKindMismatch,
        FaultCode::CredentialReferenceInvalid => Message::FaultCredentialReferenceInvalid,
        FaultCode::CredentialAlreadyExists => Message::FaultCredentialAlreadyExists,
        FaultCode::CredentialProfileMissing => Message::FaultCredentialProfileMissing,
        FaultCode::SetupIncomplete => Message::FaultSetupIncomplete,
        FaultCode::UnsupportedAudioFormat => Message::FaultUnsupportedAudioFormat,
        FaultCode::CaptureUnavailable => Message::FaultCaptureUnavailable,
        FaultCode::CapturePermissionDenied => Message::FaultCapturePermissionDenied,
        FaultCode::CapturePermissionPending => Message::FaultCapturePermissionPending,
        FaultCode::ConnectionUnavailable => Message::FaultConnectionUnavailable,
        FaultCode::BackgroundTimeout => Message::FaultBackgroundTimeout,
        FaultCode::BackgroundBusy => Message::FaultBackgroundBusy,
        FaultCode::RequestInterrupted => Message::FaultRequestInterrupted,
        FaultCode::BackgroundShuttingDown => Message::FaultBackgroundShuttingDown,
        FaultCode::RuntimeUnavailable => Message::FaultRuntimeUnavailable,
        FaultCode::ProcessAlreadyRunning => Message::FaultProcessAlreadyRunning,
        FaultCode::ProcessExited => Message::FaultProcessExited,
        FaultCode::StopFailed => Message::FaultStopFailed,
        FaultCode::StopIncomplete => Message::FaultStopIncomplete,
        FaultCode::RuntimeCleanupIncomplete => Message::FaultRuntimeCleanupIncomplete,
        FaultCode::LifecycleOwnerLost => Message::FaultLifecycleOwnerLost,
        FaultCode::HubSetupExists => Message::FaultHubSetupExists,
        FaultCode::HubSettingsRequireStop => Message::FaultHubSettingsRequireStop,
        FaultCode::SenderRequiresHubStop => Message::FaultSenderRequiresHubStop,
        FaultCode::HubRequiresSenderStop => Message::FaultHubRequiresSenderStop,
        FaultCode::SelfConnectionForbidden => Message::FaultSelfConnectionForbidden,
        FaultCode::ResourceOwnedByOtherInstance => Message::FaultResourceOwnedByOtherInstance,
        FaultCode::LocalFeedbackLoop => Message::FaultLocalFeedbackLoop,
        FaultCode::LocalFeedbackCheckUnavailable => Message::FaultLocalFeedbackCheckUnavailable,
        FaultCode::OutputBindingRequired => Message::FaultOutputBindingRequired,
        FaultCode::InvalidOutputBinding => Message::FaultInvalidOutputBinding,
        FaultCode::DiscoveryIncomplete => Message::FaultDiscoveryIncomplete,
        FaultCode::InvalidBackendResponse => Message::FaultInvalidBackendResponse,
        FaultCode::IpcInvalidRequest => Message::FaultIpcInvalidRequest,
        FaultCode::IpcIncompatibleVersion => Message::FaultIpcIncompatibleVersion,
        FaultCode::IpcMessageTooLarge => Message::FaultIpcMessageTooLarge,
        FaultCode::InvalidPath => Message::FaultInvalidPath,
        FaultCode::DiagnosticsSaveFailed => Message::FaultDiagnosticsSaveFailed,
        FaultCode::NetworkApiFailed => Message::FaultNetworkApiFailed,
        FaultCode::NetworkIdentityUnavailable => Message::FaultNetworkIdentityUnavailable,
        FaultCode::NetworkIdentityInvalid => Message::FaultNetworkIdentityInvalid,
        FaultCode::NetworkTokenUnavailable => Message::FaultNetworkTokenUnavailable,
        FaultCode::NetworkModuleUnavailable => Message::FaultNetworkModuleUnavailable,
        FaultCode::NetworkPackagePathInvalid => Message::FaultNetworkPackagePathInvalid,
        FaultCode::NetworkHashUnavailable => Message::FaultNetworkHashUnavailable,
        FaultCode::NetworkPackageReadFailed => Message::FaultNetworkPackageReadFailed,
        FaultCode::NetworkRecordInvalid => Message::FaultNetworkRecordInvalid,
        FaultCode::NetworkRecordWriteFailed => Message::FaultNetworkRecordWriteFailed,
        FaultCode::NetworkInstanceInvalid => Message::FaultNetworkInstanceInvalid,
        FaultCode::NetworkInstallationInvalid => Message::FaultNetworkInstallationInvalid,
        FaultCode::NetworkPackageHashMismatch => Message::FaultNetworkPackageHashMismatch,
        FaultCode::NetworkRequesterUnavailable => Message::FaultNetworkRequesterUnavailable,
        FaultCode::NetworkRequesterInvalid => Message::FaultNetworkRequesterInvalid,
        FaultCode::NetworkRecordMissing => Message::FaultNetworkRecordMissing,
        FaultCode::NetworkRuleOwnerMismatch => Message::FaultNetworkRuleOwnerMismatch,
        FaultCode::NetworkRuleReadbackFailed => Message::FaultNetworkRuleReadbackFailed,
        FaultCode::NetworkRuleRemoveFailed => Message::FaultNetworkRuleRemoveFailed,
        FaultCode::NetworkOperationRequired => Message::FaultNetworkOperationRequired,
        FaultCode::NetworkOperationInvalid => Message::FaultNetworkOperationInvalid,
        FaultCode::NetworkArgumentInvalid => Message::FaultNetworkArgumentInvalid,
        FaultCode::NetworkUacCancelled => Message::FaultNetworkUacCancelled,
        FaultCode::IncompatibleVersion => Message::FaultIncompatibleVersion,
        FaultCode::PlaybackBlocked => Message::FaultPlaybackBlocked,
        FaultCode::AlreadyActive => Message::FaultAlreadyActive,
        FaultCode::IdempotencyConflict => Message::FaultIdempotencyConflict,
        FaultCode::SnapshotRequired => Message::FaultSnapshotRequired,
        FaultCode::Busy => Message::FaultBusy,
        FaultCode::CommandIdReused => Message::FaultCommandIdReused,
        FaultCode::InvalidCommand => Message::FaultInvalidCommand,
        FaultCode::InvalidCommandId => Message::FaultInvalidCommandId,
        FaultCode::InvalidGain => Message::FaultInvalidGain,
        FaultCode::InvalidReceiverCount => Message::FaultInvalidReceiverCount,
        FaultCode::InvalidReceiverName => Message::FaultInvalidReceiverName,
        FaultCode::InvalidSourceAlias => Message::FaultInvalidSourceAlias,
        FaultCode::ProfileUnavailable => Message::FaultProfileUnavailable,
        FaultCode::ProfileWriteFailed => Message::FaultProfileWriteFailed,
        FaultCode::ReceiverDisabled => Message::FaultReceiverDisabled,
        FaultCode::ReceiverUnknown => Message::FaultReceiverUnknown,
        FaultCode::SourceUnknown => Message::FaultSourceUnknown,
    }
}
fn virtual_device(device: &DeviceInfo) -> bool {
    device.id.contains("com.neonmix.")
        || device.id.to_ascii_lowercase().contains("blackhole")
        || device.id.contains("neonmix.sink")
        || device.id.contains("NEONMIX")
}
fn status_text(status: Option<SessionStatus>) -> Message {
    match status {
        None => Message::SessionNoInput,
        Some(SessionStatus::Buffering) => Message::SessionBuffering,
        Some(SessionStatus::Playing) => Message::SessionPlaying,
        Some(SessionStatus::NetworkDegraded) => Message::SessionNetworkDegraded,
        Some(SessionStatus::UserStopped) => Message::SessionUserStopped,
        Some(SessionStatus::AdminDisconnected) => Message::SessionAdminDisconnected,
        Some(SessionStatus::NetworkInterrupted) => Message::SessionNetworkInterrupted,
        Some(SessionStatus::Revoked) => Message::SessionRevoked,
        Some(SessionStatus::OutputLost) => Message::SessionOutputLost,
    }
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self
            .localization
            .frame(ctx, &self.preferences.value.language, self.preview)
            && let Some(tray) = &mut self.tray
        {
            let _ = tray.update_locale(&self.localization.renderer);
        }
        let preview_tray = {
            #[cfg(feature = "screenshot")]
            {
                self.preview && std::env::var_os("NEONMIX_SCREENSHOT_TRAY").is_some()
            }
            #[cfg(not(feature = "screenshot"))]
            {
                false
            }
        };
        if !self.tray_attempted && (!self.preview || preview_tray) {
            self.tray_attempted = true;
            match tray::Tray::new(ctx.clone(), self.native_window, &self.localization.renderer) {
                Ok(tray) => self.tray = Some(tray),
                Err(_e) => {
                    self.message = Message::ShellTrayUnavailable;
                    self.error = true;
                }
            }
        }
        if let Some(tray) = &self.tray {
            for action in tray.actions() {
                match action {
                    tray::Action::Show => {
                        if !self.preview {
                            self.localization.detect(
                                &self.preferences.value.language,
                                &localization::NativeLocaleProvider,
                            );
                        }
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
                            Message::ShellQuit,
                            Message::ShellQuitConsequence,
                            Request::Shutdown,
                            ctx.memory(|m| m.focused())
                                .unwrap_or_else(|| egui::Id::new("tray-quit")),
                        );
                    }
                }
            }
        }
        if tray::maintenance_exit_requested() {
            self.exiting = true;
        }
        if self.close_if_exiting(ctx) {
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
    #[cfg(windows)]
    let _instance = if args.preview_page.is_some() {
        None
    } else {
        match tray::single_instance(&args.state_dir) {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    };
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
    fn mixer_keyboard_handles_empty_removed_and_stale_selection_in_both_layouts() {
        for console in [false, true] {
            let ctx = themed();
            let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
            app.selected_lane = Some(99);
            let next = if console {
                egui::Key::ArrowRight
            } else {
                egui::Key::ArrowDown
            };
            let previous = if console {
                egui::Key::ArrowLeft
            } else {
                egui::Key::ArrowUp
            };
            for key in [next, previous, egui::Key::M, egui::Key::S] {
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            assert_eq!(app.mixer_keys(ui, &[], console), None);
                        });
                    },
                );
                assert_eq!(app.selected_lane, None);
                assert!(app.intents.queued.is_empty());
                assert!(!app.busy);
            }
            let authority =
                neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                    .unwrap();
            let mut snapshot = authority.snapshot();
            snapshot.streams.insert(
                7,
                neonmix_control::Stream {
                    id: 7,
                    session_id: uuid::Uuid::new_v4(),
                    device_id: *snapshot.devices.keys().next().unwrap(),
                    mix: Default::default(),
                },
            );
            app.snapshot = Some(snapshot.clone());
            let lanes = app.lanes(&snapshot);
            for (key, selected) in [(egui::Key::M, None), (egui::Key::S, None), (next, Some(7))] {
                app.selected_lane = Some(99);
                let _ = ctx.run(
                    egui::RawInput {
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.mixer_keys(ui, &lanes, console);
                        });
                    },
                );
                assert_eq!(app.selected_lane, selected);
                assert!(app.intents.queued.is_empty());
            }
            // Removing the last lane invalidates selection even without a key.
            app.snapshot = Some(authority.snapshot());
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.mixer_page(ui);
                });
            });
            assert_eq!(app.selected_lane, None);
            // A focused text field owns the keyboard; no lane navigation.
            let mut text = String::new();
            for events in [
                vec![],
                vec![egui::Event::Key {
                    key: next,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                let _ = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let id = egui::Id::new("keyboard-owner");
                            ui.add(egui::TextEdit::singleline(&mut text).id(id));
                            ui.memory_mut(|m| m.request_focus(id));
                            assert_eq!(app.mixer_keys(ui, &lanes, console), None);
                        });
                    },
                );
                assert_eq!(app.selected_lane, None);
            }
        }
    }

    #[test]
    fn named_fields_publish_accessible_labels_without_exposing_password_text() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for secret in [false, true] {
            let mut value = "private-test-value".to_owned();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    widgets::field(ui, "test-field", "受测字段", &mut value, secret);
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
                app.message = Message::FaultOutputUnavailable;
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
            lifecycle: None,
            intent_version: neonmix_desktop_service::INTENT_VERSION,
            managed_control_version: 1,
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
            lifecycle: None,
            intent_version: neonmix_desktop_service::INTENT_VERSION,
            managed_control_version: 1,
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
            Work::Action(Request::Intent {
                command:
                    neonmix_desktop_service::IntentCommand::Native(neonmix_control::Command {
                        expected_revision,
                        expected_config_revision,
                        operation,
                        ..
                    }),
                ..
            }) => {
                assert_eq!(expected_revision, None);
                assert_eq!(expected_config_revision, Some(snapshot.config_revision));
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
                        let id = ui.make_persistent_id(("field", "room-name"));
                        ui.memory_mut(|m| m.request_focus(id));
                        widgets::field(ui, "room-name", "房间名称", &mut value, false);
                    });
                },
            );
        }
        assert_eq!(value, "客厅");
    }
    fn lifecycle_fixture() -> ServiceStatus {
        ServiceStatus {
            lifecycle: Some(neonmix_desktop_service::LifecycleView {
                version: 1,
                instance_generation: uuid::Uuid::new_v4(),
                hub_stop_generation: 0,
                sender_stop_generation: 0,
            }),
            intent_version: 1,
            managed_control_version: 1,
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: Vec::new(),
            sender_options: None,
            output_binding: None,
        }
    }
    #[test]
    fn polling_does_not_lock_forms_and_stop_has_priority_over_pending_start() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.status = Some(lifecycle_fixture());
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
        assert_eq!(app.message, Message::ShellStopped);
        assert!(
            !app.error,
            "an older poll error overwrote the confirmed stop"
        );
        assert!(rx.try_recv().is_err());
        assert!(!app.pending_stop);
    }
    #[test]
    fn slow_start_keeps_one_followup_and_reports_queue_capacity() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.status = Some(lifecycle_fixture());
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        let (done, result) = mpsc::sync_channel(1);
        app.results = Some(result);
        app.request(Request::HubStart);
        let Work::Action(start) = rx.try_recv().unwrap() else {
            panic!("start missing")
        };
        app.request(Request::Devices);
        assert!(matches!(app.pending_action, Some(Request::Devices)));
        // Repeated clicks cannot replace or duplicate the accepted operation.
        app.request(Request::Devices);
        assert!(matches!(app.pending_action, Some(Request::Devices)));
        app.request(Request::HubSettings {
            settings: HubSettings {
                name: "Changed".into(),
                output: "output".into(),
            },
        });
        assert_eq!(app.message, Message::IntentQueueFull);
        assert!(matches!(app.pending_action, Some(Request::Devices)));
        done.send(Ok(Outcome::Action(Box::new(start), Value::Null)))
            .unwrap();
        app.process();
        assert!(matches!(rx.try_recv(), Ok(Work::Action(Request::Devices))));
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn stopping_share_cancels_a_start_waiting_behind_a_poll() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.status = Some(lifecycle_fixture());
        app.busy = true;
        app.polling = true;
        app.request(Request::HubStart);
        assert!(app.pending("hub-start"));
        // Simulate an existing urgent stop so this test needs no real IPC.
        app.pending_stop = true;
        app.request(Request::HubStop);
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn starting_share_switch_remains_clickable_for_stop() {
        let ctx = themed();
        ctx.enable_accesskit();
        let mut app = fixture("full");
        app.page = Page::Hub;
        app.status.as_mut().unwrap().hub.running = false;
        app.inflight = Some("hub-start");
        app.busy = true;
        let mut rect = None;
        for _ in 0..3 {
            let output = frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
            let tree = output.platform_output.accesskit_update.unwrap();
            rect = tree.nodes.iter().find_map(|(_, node)| {
                (node.label() == Some(app.tr(&Message::HubStopSharing).as_str()))
                    .then(|| node.bounds())
                    .flatten()
            });
        }
        let rect = rect.expect("starting share must expose the stop action");
        let pos = egui::pos2(
            ((rect.x0 + rect.x1) / 2.0) as f32,
            ((rect.y0 + rect.y1) / 2.0) as f32,
        );
        app.preview = false;
        app.status.as_mut().unwrap().lifecycle = lifecycle_fixture().lifecycle;
        // No real IPC: a stop should cancel the queued start even when a
        // previous stop is already in flight.
        app.pending_action = Some(Request::HubStart);
        app.pending_stop = true;
        app.urgent_target = LocalStop::Sender;
        for pressed in [true, false] {
            let mut input = pointer(pos, Some(pressed), if pressed { 1.0 } else { 1.1 });
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 760.0),
            ));
            let _ = ctx.run(input, |ctx| app.show(ctx));
        }
        assert!(
            app.pending_action.is_none(),
            "the switch must dispatch HubStop while starting"
        );
    }
    #[test]
    fn stopping_an_inflight_start_uses_the_independent_path_and_keeps_unknown_results_visible() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.status = Some(lifecycle_fixture());
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        app.request(Request::SenderStart {
            options: SenderOptions {
                credential: "profiles/sender.json".into(),
                hub: None,
                output_binding: "output".into(),
            },
        });
        let Work::Action(Request::LifecycleStart {
            request,
            instance_generation,
            expected_stop_generation,
        }) = rx.try_recv().unwrap()
        else {
            panic!("start was not frozen")
        };
        assert!(matches!(request.as_ref(), Request::SenderStart { .. }));
        assert_eq!(
            instance_generation,
            app.status
                .as_ref()
                .unwrap()
                .lifecycle
                .as_ref()
                .unwrap()
                .instance_generation
        );
        assert_eq!(expected_stop_generation, 0);
        app.stop_sender();
        assert!(app.pending_stop);
        assert!(app.urgent_action.is_some());
        assert!(
            rx.try_recv().is_err(),
            "stop waited behind the startup worker"
        );
        let (done, result) = mpsc::sync_channel(1);
        app.urgent_action = Some(result);
        done.send(Err("background_timeout".into())).unwrap();
        app.process();
        assert!(!app.pending_stop);
        assert!(app.error);
        assert_eq!(app.message, Message::ShellStopUnconfirmed);
        assert!(!app.exiting);
    }
    #[cfg(unix)]
    #[test]
    fn quit_closes_idle_busy_and_locally_stopping_ui_after_real_ipc_confirmation() {
        use neonmix_desktop_service::{Envelope, Reply};
        use std::io::{Read, Write};
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        for (busy, stopping) in [(false, false), (true, false), (false, true), (true, true)] {
            let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap();
            let directory = project
                .join(".local/tmp")
                .join(format!("quit-{}", &uuid::Uuid::new_v4().to_string()[..8]));
            neonmix_desktop_service::transport::prepare(&directory).unwrap();
            let socket = directory.join("lifecycle.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let status = lifecycle_fixture();
            let instance = status.lifecycle.as_ref().unwrap().instance_generation;
            let server = std::thread::spawn(move || {
                let operation = uuid::Uuid::new_v4();
                for query in [false, true] {
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(
                                    Instant::now() < deadline,
                                    "quit never reached lifecycle IPC"
                                );
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut size = [0; 4];
                    stream.read_exact(&mut size).unwrap();
                    let mut body = vec![0; u32::from_be_bytes(size) as usize];
                    stream.read_exact(&mut body).unwrap();
                    let envelope: Envelope = serde_json::from_slice(&body).unwrap();
                    if query {
                        assert!(matches!(envelope.request, Request::LifecycleOperation {
                            operation_id, instance_generation
                        } if operation_id == operation && instance_generation == instance));
                    } else {
                        assert!(matches!(envelope.request, Request::LifecycleStop {
                            instance_generation, request
                        } if instance_generation == instance && matches!(*request, Request::Shutdown)));
                    }
                    let reply = Reply::success(serde_json::json!({
                        "operation_id": operation, "instance_generation": instance,
                        "state": if query { "completed" } else { "accepted" }, "ok": true,
                        "data": {"hub": {"cleanup_complete": true}, "sender": {"cleanup_complete": true}}
                    }));
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    stream
                        .write_all(&(bytes.len() as u32).to_be_bytes())
                        .unwrap();
                    stream.write_all(&bytes).unwrap();
                }
            });
            let mut app = Desktop::empty(Client::new(&directory), true, false);
            app.status = Some(status);
            app.busy = busy;
            app.pending_stop = stopping;
            app.urgent_target = LocalStop::Sender;
            let (old_done, old_result) = mpsc::sync_channel(1);
            if stopping {
                app.urgent_action = Some(old_result);
            }
            let (tx, jobs) = mpsc::sync_channel(1);
            app.worker = Some(tx);
            app.pending_action = Some(Request::HubStart);
            app.request(Request::Shutdown);
            assert!(app.pending_stop);
            assert!(matches!(app.urgent_target, LocalStop::Shutdown));
            assert!(app.pending_action.is_none());
            assert!(jobs.try_recv().is_err(), "quit entered the ordinary queue");
            if stopping {
                assert!(
                    old_done.send(Ok(Value::Null)).is_err(),
                    "old stop still owns UI completion"
                );
            }
            app.request(Request::HubStart);
            assert!(jobs.try_recv().is_err(), "start was dispatched during quit");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !app.exiting && Instant::now() < deadline {
                let ctx = themed();
                let output = ctx.run(egui::RawInput::default(), |ctx| app.show(ctx));
                if app.exiting {
                    assert!(
                        output.viewport_output[&egui::ViewportId::ROOT]
                            .commands
                            .iter()
                            .any(|command| matches!(command, egui::ViewportCommand::Close))
                    );
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            server.join().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(app.exiting, "confirmed quit did not close the UI");
            assert!(!app.pending_stop);
        }
    }
    #[test]
    fn lost_stop_worker_clears_pending_and_preserves_retry_after_an_ordinary_reply() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.status = Some(lifecycle_fixture());
        app.pending_stop = true;
        app.urgent_target = LocalStop::Shutdown;
        let (done, result) = mpsc::sync_channel(1);
        app.urgent_action = Some(result);
        drop(done);
        let (done, result) = mpsc::sync_channel(1);
        app.results = Some(result);
        done.send(Ok(Outcome::Action(Box::new(Request::Devices), Value::Null)))
            .unwrap();
        app.process();
        assert!(!app.pending_stop);
        assert!(app.urgent_action.is_none());
        assert!(!app.exiting);
        assert!(app.error);
        assert_eq!(app.message, Message::ShellStopUnconfirmed);
    }
    #[test]
    fn two_failed_ipc_calls_do_not_prove_that_background_audio_stopped() {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let client = Client::new(
            project
                .join(".local/tmp")
                .join(format!("missing-stop-{}", uuid::Uuid::new_v4())),
        );
        for request in [Request::SenderStop, Request::HubStop, Request::Shutdown] {
            assert!(call(&client, &request).is_err());
        }
    }
    #[test]
    fn cancel_modal_keeps_accesskit_focus_in_the_published_tree() {
        let ctx = themed();
        ctx.enable_accesskit();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        app.confirmation(
            Message::ShellQuit,
            Message::ShellQuitConsequence,
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
            lifecycle: None,
            intent_version: neonmix_desktop_service::INTENT_VERSION,
            managed_control_version: 1,
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
        let Work::Action(first_request) = rx.try_recv().unwrap() else {
            panic!("expected intent");
        };
        assert!(matches!(first_request, Request::Intent { .. }));
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
        assert!(!app.intents.queued.is_empty());

        done.send(Ok(Outcome::Action(Box::new(first_request), Value::Null)))
            .unwrap();
        app.process();
        assert!(matches!(rx.try_recv(), Ok(Work::Poll { .. })));
        snapshot.revision += 1;
        done.send(Ok(Outcome::Poll(Box::new(PollData {
            received: Default::default(),
            partial: false,
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
            Ok(Work::Action(Request::Intent {
                command:
                    neonmix_desktop_service::IntentCommand::Native(neonmix_control::Command {
                        expected_revision,
                        expected_config_revision,
                        operation,
                        ..
                    }),
                ..
            })) => {
                assert_eq!(expected_revision, None);
                assert_eq!(expected_config_revision, Some(snapshot.config_revision));
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
        // Horizontal row fader and vertical console fader share one core.
        for (size, far) in [
            (widgets::FaderSize::Row, egui::pos2(380.0, 20.0)),
            (widgets::FaderSize::Strip(80.0), egui::pos2(22.0, 84.0)),
        ] {
            let ctx = themed();
            let mut gain = -30.0_f32;
            let frame = |input: egui::RawInput, gain: &mut f32| {
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        widgets::fader(ui, "test", gain, "受测音量", size);
                    });
                });
            };
            frame(pointer(far, None, 0.0), &mut gain);
            frame(pointer(far, Some(true), 0.1), &mut gain);
            frame(pointer(far, Some(false), 0.15), &mut gain);
            assert_eq!(gain, -30.0, "a click on the track must not move the fader");
            frame(pointer(far, Some(true), 0.2), &mut gain);
            frame(pointer(far, Some(false), 0.25), &mut gain);
            frame(pointer(far, None, 0.3), &mut gain);
            assert_eq!(gain, 0.0, "double-click returns to 0 dB");
        }
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
        // The first "前往" entry is the first page.
        assert_eq!(app.page, Page::ALL[0].0);
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
            lifecycle: None,
            intent_version: neonmix_desktop_service::INTENT_VERSION,
            managed_control_version: 1,
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
        let Work::Action(first_request) = rx.try_recv().unwrap() else {
            panic!("expected intent");
        };
        assert!(matches!(first_request, Request::Intent { .. }));
        assert!(!app.undo_available(), "Undo requires acknowledgement");
        assert!(
            rx.try_recv().is_err(),
            "restore waits for the fresh revision"
        );
        done.send(Ok(Outcome::Action(Box::new(first_request), Value::Null)))
            .unwrap();
        app.process();
        assert!(matches!(rx.try_recv(), Ok(Work::Poll { .. })));
        snapshot.revision += 1;
        snapshot.output.muted = true;
        done.send(Ok(Outcome::Poll(Box::new(PollData {
            received: Default::default(),
            partial: false,
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
        assert!(app.undo_available());
        app.restore_last();
        match rx.try_recv() {
            Ok(Work::Action(Request::Intent {
                command:
                    neonmix_desktop_service::IntentCommand::Native(neonmix_control::Command {
                        expected_revision,
                        expected_config_revision,
                        operation,
                        ..
                    }),
                ..
            })) => {
                assert_eq!(expected_revision, None);
                assert_eq!(expected_config_revision, Some(snapshot.config_revision));
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
        app.message = Message::ShellAdjusted {
            change: "状态已被其他操作更新，请核对后重试；未提交的设备名称与别名草稿仍会保留。"
                .repeat(3),
        };
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

    fn fixture(name: &str) -> Desktop {
        let path = format!(
            "{}/../../docs/evidence/ui-rebuild-20261004/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, true);
        app.load_preview(&data);
        app
    }

    fn frame(ctx: &egui::Context, app: &mut Desktop, size: egui::Vec2) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }

    #[test]
    fn navigation_renders_body_text_at_full_opacity_without_sliding() {
        fn body_label(shape: &egui::Shape) -> Option<(egui::Pos2, u8)> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == "房间名称" => {
                    let alpha = text
                        .galley
                        .rows
                        .iter()
                        .flat_map(|row| &row.visuals.mesh.vertices)
                        .map(|vertex| vertex.color.a())
                        .max()
                        .unwrap();
                    Some((text.pos, alpha))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(body_label),
                _ => None,
            }
        }
        let ctx = themed();
        let mut app = fixture("full");
        app.navigate(Page::Hub);
        // Let egui determine the panel geometry, then revisit the same page
        // exactly as a navigation click would. Its first frame must be final.
        for _ in 0..3 {
            frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
        }
        let settled = frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
        let expected = settled
            .shapes
            .iter()
            .find_map(|s| body_label(&s.shape))
            .unwrap();
        app.navigate(Page::Live);
        frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
        app.navigate(Page::Hub);
        let first = frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
        let actual = first
            .shapes
            .iter()
            .find_map(|s| body_label(&s.shape))
            .unwrap();
        assert_eq!(actual.1, 255, "navigation dimmed the entire page body");
        assert_eq!(actual.0, expected.0, "navigation displaced the page body");
    }

    #[test]
    fn english_header_and_long_room_badge_keep_about_content_inside_the_central_panel() {
        let ctx = themed();
        ctx.enable_accesskit();
        let mut app = fixture("full");
        app.page = Page::About;
        std::sync::Arc::make_mut(&mut app.localization.renderer)
            .set_locale(neonmix_i18n::ResolvedLocale::En);
        app.localization.locale = neonmix_i18n::ResolvedLocale::En;
        for long in [false, true] {
            if long {
                app.remote_room = Some("中文房间 { $name } ".repeat(40));
            }
            let output = frame(&ctx, &mut app, egui::vec2(600.0, 440.0));
            let tree = output.platform_output.accesskit_update.unwrap();
            let brands: Vec<_> = tree
                .nodes
                .iter()
                .filter_map(|(_, n)| (n.value() == Some("NeonMix")).then(|| n.bounds()).flatten())
                .collect();
            assert!(
                brands.iter().any(|b| b.x0 >= 188.0 && b.x1 <= 600.0),
                "central title escaped viewport: {brands:?}"
            );
            assert!(app.localization.renderer.diagnostics().is_empty());
        }
    }

    #[test]
    fn live_graph_names_every_node_for_assistive_tech() {
        for size in [egui::vec2(1100.0, 760.0), egui::vec2(600.0, 440.0)] {
            let mut app = fixture("full");
            assert_eq!(app.page, Page::Live);
            let ctx = themed();
            ctx.enable_accesskit();
            let output = frame(&ctx, &mut app, size);
            let tree = output.platform_output.accesskit_update.unwrap();
            let labels: Vec<String> = tree
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    node.label()
                        .map(|s| s.replace(['\u{2068}', '\u{2069}'], ""))
                })
                .collect();
            for needle in [
                "E07 真实采集 A，原生 Sender，成员 · 已静音",
                "客厅 iPhone，AirPlay，AirPlay · 因 Solo 静音",
                "房间「E07 双路测试」",
                "另有 1 台已配对设备未在发送",
            ] {
                assert!(
                    labels.iter().any(|l| l.starts_with(needle)),
                    "{size:?}: missing {needle} in {labels:?}"
                );
            }
        }
    }

    #[test]
    fn live_graph_moves_only_while_signal_flows() {
        let delay = |app: &mut Desktop| {
            let ctx = themed();
            frame(&ctx, app, egui::vec2(1100.0, 760.0));
            // Settle egui's own first passes and the status message fade-in.
            app.message_since = Instant::now() - Duration::from_secs(5);
            for _ in 0..3 {
                frame(&ctx, app, egui::vec2(1100.0, 760.0));
            }
            let delay = frame(&ctx, app, egui::vec2(1100.0, 760.0)).viewport_output
                [&egui::ViewportId::ROOT]
                .repaint_delay;
            (delay, ctx.repaint_causes())
        };
        let (d, why) = delay(&mut fixture("playing"));
        assert!(d <= animation::LIVE_FRAME, "{d:?} {why:?}");
        let mut silent = fixture("playing");
        silent.diagnostics = None;
        for mut app in [silent, fixture("empty")] {
            let (d, why) = delay(&mut app);
            assert!(d > animation::LIVE_FRAME, "{d:?} {why:?}");
        }
        animation::set_reduce_motion(true);
        let (d, why) = delay(&mut fixture("playing"));
        animation::set_reduce_motion(false);
        assert!(d > animation::LIVE_FRAME, "{d:?} {why:?}");
    }

    #[test]
    fn mixer_arrows_follow_the_fader_direction_of_the_layout() {
        let press = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        // Console (wide): ←/→ pick strips. Rows (narrow): ↑/↓ pick rows.
        for (size, pick, other) in [
            (
                egui::vec2(1100.0, 760.0),
                egui::Key::ArrowRight,
                egui::Key::ArrowDown,
            ),
            (
                egui::vec2(600.0, 440.0),
                egui::Key::ArrowDown,
                egui::Key::ArrowRight,
            ),
        ] {
            let mut app = fixture("full");
            app.page = Page::Mixer;
            let ctx = themed();
            frame(&ctx, &mut app, size);
            let run = |app: &mut Desktop, key| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events: vec![press(key)],
                        ..Default::default()
                    },
                    |ctx| app.show(ctx),
                );
            };
            run(&mut app, other);
            assert_eq!(app.selected_lane, None, "{size:?}: {other:?} must not pick");
            run(&mut app, pick);
            assert_eq!(app.selected_lane, Some(1158294319324071), "{size:?}");
        }
    }

    #[test]
    fn sidebar_fits_seven_pages_and_actions_in_the_minimum_window() {
        let mut app = fixture("full");
        let ctx = themed();
        ctx.enable_accesskit();
        let output = frame(&ctx, &mut app, egui::vec2(600.0, 440.0));
        let tree = output.platform_output.accesskit_update.unwrap();
        let bounds = |label: &str| {
            tree.nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .and_then(|(_, n)| n.bounds())
                .unwrap_or_else(|| panic!("{label} missing"))
        };
        let last_page = bounds("设置");
        let identity = bounds("控制身份");
        assert!(
            last_page.y1 <= identity.y0,
            "navigation {last_page:?} overlaps identity {identity:?}"
        );
        assert!(identity.y1 <= 440.0, "identity row {identity:?} off screen");
        if let Some(stop) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("停止发送"))
            .and_then(|(_, n)| n.bounds())
        {
            assert!(
                last_page.y1 <= stop.y0 && stop.y1 <= identity.y0,
                "stop sending {stop:?} overlaps navigation or identity"
            );
        }
    }

    #[test]
    fn one_failed_airplay_or_diagnostics_read_keeps_the_room_on_screen() {
        let mut app = fixture("full");
        let (done, results) = mpsc::sync_channel(1);
        app.results = Some(results);
        let lanes = |app: &Desktop| app.lanes(app.snapshot.as_ref().unwrap()).len();
        assert_eq!(lanes(&app), 4);
        let partial = |app: &Desktop| PollData {
            received: Default::default(),
            partial: true,
            credential: app.credential.clone(),
            hub: app.hub(),
            room_name: None,
            status: app.status.clone().unwrap(),
            snapshot: app.snapshot.clone(),
            diagnostics: None,
            airplay: None,
            error: Some("connection reset".into()),
        };
        for poll in 1..=PARTIAL_REPORT {
            done.send(Ok(Outcome::Poll(Box::new(partial(&app)))))
                .unwrap();
            app.process();
            assert_eq!(lanes(&app), 4, "poll {poll}: AirPlay lanes must not vanish");
            assert!(app.diagnostics.is_some(), "poll {poll}: meters must stay");
            assert!(app.ready(), "poll {poll}: diagnostics cannot gate control");
            assert_eq!(
                app.error,
                poll >= PARTIAL_REPORT,
                "poll {poll}: only a persistent failure turns the strip red"
            );
        }
    }

    #[test]
    fn fresh_native_get_cannot_refresh_failed_diagnostic_clock_or_feed_old_meters_into_history() {
        let authority = neonmix_control::Authority::new(
            "test".into(),
            "admin".into(),
            "freshness-admin-token-with-at-least-32-bytes",
        )
        .unwrap();
        let mut app = Desktop::empty(
            Client::new(".local/test-independent-observations"),
            true,
            false,
        );
        app.snapshot = Some(authority.snapshot());
        app.status = Some(ServiceStatus {
            lifecycle: None,
            intent_version: 2,
            managed_control_version: 1,
            version: 1,
            pid: 1,
            hub: Default::default(),
            sender: Default::default(),
            hub_settings: None,
            profiles: vec![],
            sender_options: None,
            output_binding: None,
        });
        let value = serde_json::json!({"available":true,"output_stats":{"errors":0,"callback_over_budget":0},"underrun_frames":0,"output_frames":480,
            "receivers":[],"lane_stream_ids":[],"queues":[],"meters":{"lanes":[]}});
        let old = Instant::now() - Duration::from_secs(1);
        app.diagnostics_clock
            .success(&value, Some(authority.current().runtime_epoch), old);
        app.diagnostics = Some(value.clone());
        let (tx, rx) = mpsc::sync_channel(4);
        app.results = Some(rx);
        for _ in 0..2 {
            app.busy = true;
            app.polling = true;
            tx.send(Ok(Outcome::Poll(Box::new(PollData {
                received: PollTimes {
                    snapshot: Some(Instant::now()),
                    ..Default::default()
                },
                credential: app.credential.clone(),
                hub: app.hub(),
                room_name: None,
                status: app.status.clone().unwrap(),
                snapshot: Some(authority.snapshot()),
                diagnostics: None,
                airplay: None,
                partial: true,
                error: None,
            }))))
            .unwrap();
            app.process();
            assert_eq!(app.diagnostics_clock.last_received(), Some(old));
            assert!(app.diagnostics_current().is_none());
            assert_eq!(app.diagnostics.as_ref(), Some(&value));
            assert!(
                app.synced
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
            );
        }
        assert!(
            app.metric_history
                .get(&"output")
                .unwrap()
                .iter()
                .all(|sample| sample.value.is_none())
        );
        app.busy = true;
        app.polling = true;
        tx.send(Ok(Outcome::Poll(Box::new(PollData {
            received: Default::default(),
            credential: app.credential.clone(),
            hub: app.hub(),
            room_name: None,
            status: app.status.clone().unwrap(),
            snapshot: Some(authority.snapshot()),
            diagnostics: Some(serde_json::json!({"available":false})),
            airplay: None,
            partial: false,
            error: None,
        }))))
        .unwrap();
        app.process();
        assert_eq!(app.diagnostics.as_ref(), Some(&value));
        assert_eq!(app.diagnostics_clock.last_received(), Some(old));
        assert!(app.diagnostics_current().is_none());
    }

    #[test]
    fn slow_refresh_keeps_controls_interactive_without_sending_stale_writes() {
        let mut app = fixture("full");
        app.snapshot.as_mut().unwrap().runtime_epoch = uuid::Uuid::new_v4();
        app.snapshot.as_mut().unwrap().control_version = neonmix_control::CONTROL_VERSION;
        app.status.as_mut().unwrap().intent_version = neonmix_desktop_service::INTENT_VERSION;
        app.preview = false;
        app.online = false;
        let (tx, rx) = mpsc::sync_channel(1);
        app.worker = Some(tx);
        app.fresh = Some(Instant::now() - Duration::from_secs(5));
        app.synced = app.fresh;
        app.busy = true;
        app.polling = true;
        app.last_poll = Instant::now();
        for (page, _) in Page::ALL {
            app.page = page;
            let ctx = themed();
            ctx.enable_accesskit();
            let output = frame(&ctx, &mut app, egui::vec2(1100.0, 760.0));
            assert!(app.writable(), "{page:?}: refresh must not grey controls");
            assert!(!app.ready());
            if page != Page::Mixer {
                let label = app.tr(&Message::ShellMasterMute);
                let tree = output.platform_output.accesskit_update.unwrap();
                let control = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label.as_str()))
                    .unwrap();
                assert!(!control.1.is_disabled(), "{page:?}: master mute faded");
            }
        }
        app.operation(Operation::OutputMix {
            gain_db: None,
            muted: Some(true),
        });
        assert!(!app.intents.queued.is_empty());
        assert!(
            rx.try_recv().is_err(),
            "stale authority must never send a command"
        );
        app.synced = Some(Instant::now() - Duration::from_secs(46));
        assert!(
            !app.writable(),
            "a hung request has a bounded interaction grace"
        );
        let (done, results) = mpsc::sync_channel(1);
        app.results = Some(results);
        done.send(Ok(Outcome::Poll(Box::new(PollData {
            received: PollTimes {
                snapshot: Some(Instant::now()),
                ..Default::default()
            },
            credential: app.credential.clone(),
            hub: app.hub(),
            room_name: app.remote_room.clone(),
            status: app.status.clone().unwrap(),
            snapshot: app.snapshot.clone(),
            diagnostics: None,
            airplay: None,
            partial: true,
            error: None,
        }))))
        .unwrap();
        app.process();
        assert!(
            matches!(rx.try_recv(), Ok(Work::Action(Request::Intent { .. }))),
            "fresh authority must dispatch the intention retained during refresh"
        );
    }

    #[test]
    fn transient_snapshot_failure_preserves_recent_controls_and_cannot_toggle_on_each_retry() {
        let mut app = fixture("full");
        let (done, results) = mpsc::sync_channel(1);
        app.results = Some(results);
        let old_name = app.remote_room.clone();
        let failed = PollData {
            received: Default::default(),
            partial: false,
            credential: app.credential.clone(),
            hub: app.hub(),
            room_name: None,
            status: app.status.clone().unwrap(),
            snapshot: None,
            diagnostics: None,
            airplay: None,
            error: Some("background_busy".into()),
        };
        done.send(Ok(Outcome::Poll(Box::new(failed)))).unwrap();
        app.process();
        assert!(
            app.writable(),
            "a transient read is not permission revocation"
        );
        assert!(!app.ready());
        assert_eq!(app.remote_room, old_name);
        assert_eq!(app.lanes(app.snapshot.as_ref().unwrap()).len(), 4);
        assert!(app.diagnostics.is_some());
        assert!(app.diagnostics_current().is_none());
        app.synced = Some(Instant::now() - Duration::from_secs(5));
        for busy in [false, true, false, true] {
            app.busy = busy;
            app.polling = busy;
            assert!(
                !app.writable(),
                "retry alone cannot re-enable a failed connection"
            );
        }
    }

    #[test]
    fn polling_publishes_control_after_optional_reads_and_rejects_previous_runtime_observations() {
        for telemetry_fails in [false, true] {
            let mut app = fixture("full");
            let mut calls = Vec::new();
            let data = read_poll(app.credential.clone(), app.hub(), |request| match request {
                Request::Status => {
                    calls.push("status");
                    Ok(serde_json::to_value(app.status.as_ref().unwrap()).unwrap())
                }
                Request::AirplayV2 { .. } => {
                    calls.push("airplay");
                    Ok(serde_json::json!({"runtime_epoch": uuid::Uuid::new_v4()}))
                }
                Request::Diagnostics { .. } => {
                    calls.push("diagnostics");
                    if telemetry_fails {
                        Err("background_timeout".into())
                    } else {
                        Ok(serde_json::json!({"remote":{"runtime_epoch":uuid::Uuid::new_v4()}}))
                    }
                }
                Request::Snapshot { .. } => {
                    calls.push("snapshot");
                    Ok(serde_json::to_value(app.snapshot.as_ref().unwrap()).unwrap())
                }
                _ => panic!("unexpected read"),
            })
            .unwrap();
            assert_eq!(calls, ["status", "airplay", "diagnostics", "snapshot"]);
            assert!(data.airplay.is_none());
            assert!(data.diagnostics.is_none());
            assert!(data.received.snapshot >= data.received.airplay);
            let (tx, rx) = mpsc::sync_channel(1);
            app.results = Some(rx);
            app.partial_polls = PARTIAL_REPORT;
            tx.send(Ok(Outcome::Poll(Box::new(data)))).unwrap();
            app.process();
            assert!(
                app.ready(),
                "optional failure must not invalidate authoritative control"
            );
        }
    }

    #[test]
    fn permission_loss_and_stopped_room_disable_controls_without_refresh_grace() {
        for error in [
            Some("permission_denied"),
            Some("unauthenticated"),
            Some("pairing_revoked"),
            None,
        ] {
            let mut app = fixture("full");
            let (tx, rx) = mpsc::sync_channel(1);
            app.results = Some(rx);
            let mut status = app.status.clone().unwrap();
            if error.is_none() {
                status.hub.running = false;
            }
            tx.send(Ok(Outcome::Poll(Box::new(PollData {
                received: Default::default(),
                credential: app.credential.clone(),
                hub: app.hub(),
                room_name: None,
                status,
                snapshot: None,
                airplay: None,
                diagnostics: None,
                partial: false,
                error: error.map(UiError::from),
            }))))
            .unwrap();
            app.process();
            app.busy = true;
            app.polling = true;
            assert!(
                !app.writable(),
                "{error:?}: disabled until new authority is obtained"
            );
            assert!(!app.ready());
        }
    }

    #[test]
    fn stopped_local_hub_is_not_polled_for_room_state() {
        let mut status = fixture("full").status.unwrap();
        let admin = std::path::Path::new("hub/admin.json");
        assert!(fetches_room(admin, &status));
        status.hub.running = false;
        assert!(!fetches_room(admin, &status));
        assert!(fetches_room(
            std::path::Path::new("profiles/b.json"),
            &status
        ));
    }
}
