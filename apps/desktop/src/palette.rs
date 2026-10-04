//! 快速操作 (⌘K): one searchable list of navigation, room, sending and mixer
//! actions built from the current authoritative state. Only actions the
//! current identity may perform are listed; dangerous ones still confirm.
use super::*;
use crate::widgets::Tone;
use egui::{Align2, CornerRadius, Margin, Stroke};

#[derive(Default)]
pub struct Palette {
    query: String,
    selected: usize,
    /// IME preedit is active: Enter belongs to the input method, not to us.
    composing: bool,
    focused: bool,
}

#[derive(Clone)]
enum Cmd {
    Go(Page),
    HubStart,
    HubStop,
    SenderStart,
    SenderStop,
    MasterMute(bool),
    LaneMute(u64, bool),
    LaneSolo(u64, bool),
    Restore,
    Invite,
    Discover,
    Export,
    Hide,
    Quit,
}

struct Entry {
    group: &'static str,
    title: String,
    keywords: &'static str,
    hint: Option<String>,
    tone: Tone,
    cmd: Cmd,
}

impl Desktop {
    pub(crate) fn open_palette(&mut self) {
        if self.confirm.is_none() {
            self.palette = Some(Palette::default());
            self.palette_since = Instant::now();
        }
    }

    fn palette_entries(&self) -> Vec<Entry> {
        let mut list = Vec::new();
        let mut push = |group, title: String, keywords, hint, tone, cmd| {
            list.push(Entry {
                group,
                title,
                keywords,
                hint,
                tone,
                cmd,
            })
        };
        for (i, (page, title)) in Page::ALL.into_iter().enumerate() {
            let hint = widgets::command_hint(&(i + 1).to_string());
            push(
                "前往",
                format!("前往 {title}"),
                "go page",
                Some(hint),
                Tone::Neutral,
                Cmd::Go(page),
            );
        }
        if self.undo_available()
            && let Some(undo) = &self.undo
        {
            push(
                "混音",
                format!("还原：{}", undo.label),
                "undo restore",
                Some(widgets::command_hint("Z")),
                Tone::Accent,
                Cmd::Restore,
            );
        }
        let status = self.status.as_ref();
        let configured = status.is_some_and(|s| s.hub_settings.is_some());
        let sharing = status.is_some_and(|s| s.hub.running);
        if configured && !sharing {
            push(
                "房间",
                "开始共享".into(),
                "hub share start",
                None,
                Tone::Success,
                Cmd::HubStart,
            );
        }
        if sharing {
            push(
                "房间",
                "停止共享".into(),
                "hub share stop",
                None,
                Tone::Danger,
                Cmd::HubStop,
            );
        }
        let sending = status.is_some_and(|s| s.sender.running);
        let binding_on = self
            .binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        if !sending && binding_on && self.sender_allowed() {
            push(
                "发送",
                "开始发送".into(),
                "send start",
                None,
                Tone::Success,
                Cmd::SenderStart,
            );
        }
        if sending {
            push(
                "发送",
                "停止发送".into(),
                "send stop",
                None,
                Tone::Danger,
                Cmd::SenderStop,
            );
        }
        if let Some(state) = &self.snapshot
            && self.writable()
        {
            if self.controls_room() {
                let muted = state.output.muted;
                push(
                    "混音",
                    if muted {
                        "取消总静音"
                    } else {
                        "总静音"
                    }
                    .into(),
                    "master mute",
                    None,
                    Tone::Warning,
                    Cmd::MasterMute(!muted),
                );
            }
            let mine = self
                .status
                .as_ref()
                .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
                .and_then(|p| p.device_id);
            for stream in state.streams.values() {
                let name = state
                    .devices
                    .get(&stream.device_id)
                    .map_or("未知设备", |d| d.name.as_str());
                if self.controls_room() || mine == Some(stream.device_id) {
                    let m = stream.mix.muted;
                    push(
                        "混音",
                        format!("{}「{name}」", if m { "取消静音" } else { "静音" }),
                        "mute channel",
                        None,
                        Tone::Warning,
                        Cmd::LaneMute(stream.id, !m),
                    );
                }
                if self.controls_room() {
                    let s = stream.mix.solo;
                    push(
                        "混音",
                        format!("{}「{name}」", if s { "取消 Solo" } else { "Solo" }),
                        "solo channel",
                        None,
                        Tone::Accent,
                        Cmd::LaneSolo(stream.id, !s),
                    );
                }
            }
            if self.admin() {
                push(
                    "房间",
                    "创建一次性邀请".into(),
                    "invite pair",
                    None,
                    Tone::Accent,
                    Cmd::Invite,
                );
            }
        }
        push(
            "发送",
            "发现局域网房间".into(),
            "discover scan room",
            None,
            Tone::Neutral,
            Cmd::Discover,
        );
        if self.diagnostics.is_some() {
            push(
                "诊断",
                "导出脱敏诊断".into(),
                "export diagnostics",
                None,
                Tone::Neutral,
                Cmd::Export,
            );
        }
        push(
            "窗口",
            "隐藏窗口".into(),
            "hide window close",
            cfg!(target_os = "macos").then(|| widgets::command_hint("W")),
            Tone::Neutral,
            Cmd::Hide,
        );
        push(
            "窗口",
            "退出后台…".into(),
            "quit exit shutdown",
            cfg!(target_os = "macos").then(|| widgets::command_hint("Q")),
            Tone::Danger,
            Cmd::Quit,
        );
        list
    }

    pub(crate) fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut palette) = self.palette.take() else {
            return;
        };
        // Track IME composition so Enter that commits a candidate is not
        // taken as "run the highlighted action".
        let mut committed_text = false;
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit(text)) => {
                        palette.composing = !text.is_empty()
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_))
                    | egui::Event::Ime(egui::ImeEvent::Disabled) => {
                        palette.composing = false;
                        committed_text = true;
                    }
                    _ => {}
                }
            }
        });
        let needle = palette.query.trim().to_lowercase();
        let entries: Vec<Entry> = self
            .palette_entries()
            .into_iter()
            .filter(|e| {
                needle.is_empty()
                    || e.title.to_lowercase().contains(&needle)
                    || e.keywords.contains(&needle)
                    || e.group.contains(&needle)
            })
            .collect();
        if !palette.composing {
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    palette.selected += 1;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    palette.selected = palette.selected.saturating_sub(1);
                }
            });
        }
        palette.selected = palette.selected.min(entries.len().saturating_sub(1));
        // While composing, Enter belongs to the input method: swallow it so
        // the search field keeps focus and receives the committed text.
        if palette.composing {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        let enter = !palette.composing
            && !committed_text
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut run = enter
            .then(|| entries.get(palette.selected).map(|e| e.cmd.clone()))
            .flatten();
        let width = (ctx.screen_rect().width() - 48.0).min(520.0);
        // Opens with a short fade and settle from slightly above.
        let appear = animation::fade_in(ctx, self.palette_since.elapsed(), 0.14);
        let top = (ctx.screen_rect().height() * 0.1).min(64.0) - 8.0 * (1.0 - appear);
        // Reserve search, footer and frame space inside the minimum window.
        let list_height = (ctx.screen_rect().height() - top - 120.0).clamp(80.0, 340.0);
        let response = egui::Modal::new(egui::Id::new("palette"))
            .area(
                egui::Modal::default_area(egui::Id::new("palette-area"))
                    .anchor(Align2::CENTER_TOP, egui::vec2(0.0, top)),
            )
            .backdrop_color(egui::Color32::from_black_alpha(130))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                    .inner_margin(Margin::same(10))
                    .corner_radius(CornerRadius::same(14)),
            )
            .show(ctx, |ui| {
                ui.set_opacity(0.4 + 0.6 * appear);
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let (icon, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 30.0), egui::Sense::hover());
                    icons::paint(
                        ui.painter(),
                        icons::square(icon.center(), 16.0),
                        icons::Icon::Search,
                        theme::TEXT_3,
                    );
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut palette.query)
                            .hint_text("搜索操作、页面或通道…")
                            .frame(false)
                            .font(egui::FontId::proportional(16.0))
                            .desired_width(ui.available_width()),
                    );
                    if !palette.focused {
                        edit.request_focus();
                        palette.focused = true;
                    }
                    if edit.changed() {
                        palette.selected = 0;
                    }
                });
                ui.separator();
                if entries.is_empty() {
                    ui.add_space(10.0);
                    widgets::note(ui, "没有匹配的操作。");
                    ui.add_space(6.0);
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut last_group = "";
                        for (i, entry) in entries.iter().enumerate() {
                            if entry.group != last_group {
                                last_group = entry.group;
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(entry.group).size(11.0).color(theme::TEXT_3),
                                );
                            }
                            let selected = i == palette.selected;
                            let (rect, row) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 32.0),
                                egui::Sense::click(),
                            );
                            row.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    true,
                                    selected,
                                    &entry.title,
                                )
                            });
                            if row.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                            {
                                palette.selected = i;
                            }
                            if selected {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(8),
                                    theme::ACCENT.gamma_multiply(0.16),
                                );
                                ui.scroll_to_rect(rect, None);
                            }
                            ui.painter().circle_filled(
                                egui::pos2(rect.left() + 12.0, rect.center().y),
                                3.0,
                                entry.tone.color(),
                            );
                            ui.painter().text(
                                egui::pos2(rect.left() + 24.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                &entry.title,
                                egui::FontId::proportional(theme::BODY),
                                if selected { theme::TEXT } else { theme::TEXT_2 },
                            );
                            if let Some(hint) = &entry.hint {
                                ui.painter().text(
                                    egui::pos2(rect.right() - 10.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    hint,
                                    egui::FontId::monospace(11.0),
                                    theme::TEXT_3,
                                );
                            }
                            if row.clicked() {
                                run = Some(entry.cmd.clone());
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    widgets::kbd(ui, "↑↓");
                    widgets::note(ui, "选择");
                    widgets::kbd(ui, "↩");
                    widgets::note(ui, "执行");
                    widgets::kbd(ui, "esc");
                    widgets::note(ui, "关闭");
                });
            });
        if let Some(cmd) = run {
            self.run_command(ctx, cmd);
            return;
        }
        if !response.should_close() {
            self.palette = Some(palette);
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Go(page) => self.navigate(page),
            Cmd::HubStart => self.request(Request::HubStart),
            Cmd::HubStop => self.request(Request::HubStop),
            Cmd::SenderStart => self.request(Request::SenderStart {
                options: SenderOptions {
                    credential: self.credential.clone(),
                    hub: self.hub(),
                    output_binding: PathBuf::from("output"),
                },
            }),
            Cmd::SenderStop => self.stop_sender(),
            Cmd::MasterMute(on) => self.mix_change(
                if on { "总静音" } else { "取消总静音" }.into(),
                Write::Control(Operation::OutputMix {
                    gain_db: None,
                    muted: Some(on),
                }),
                Write::Control(Operation::OutputMix {
                    gain_db: None,
                    muted: Some(!on),
                }),
            ),
            Cmd::LaneMute(id, on) => self.lane_toggle(id, Some(on), None),
            Cmd::LaneSolo(id, on) => self.lane_toggle(id, None, Some(on)),
            Cmd::Restore => self.restore_last(),
            Cmd::Invite => self.request(Request::Invite {
                credential: self.credential.clone(),
                hub: self.hub(),
                out: PathBuf::from(format!("invitations/{}.json", uuid::Uuid::new_v4())),
                seconds: 120,
            }),
            Cmd::Discover => {
                self.navigate(Page::Sender);
                self.request(Request::Discover { seconds: 3 });
            }
            Cmd::Export => self.request(Request::ExportDiagnostics {
                credential: self.credential.clone(),
                hub: self.hub(),
            }),
            Cmd::Hide => {
                if self.tray.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            Cmd::Quit => self.confirmation(
                "退出后台".into(),
                "停止本实例的共享、发送和全部音频连接。关闭窗口不会停止音频；退出后台会。".into(),
                Request::Shutdown,
                egui::Id::new("palette-quit"),
            ),
        }
    }
}
