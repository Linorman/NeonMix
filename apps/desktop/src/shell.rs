//! Window frame: icon sidebar with identity and process controls, a top bar
//! that always answers "what is the room doing now", the page body, a status
//! strip with 还原, the confirmation modal and the command palette.
use super::*;
use crate::widgets::{Kind, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

/// Wide windows keep line length and meter span readable.
const CONTENT_MAX_WIDTH: f32 = 1180.0;

impl Desktop {
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        self.process();
        if self.message != self.shown_message {
            self.shown_message = self.message.clone();
            self.message_since = Instant::now();
        }
        self.shortcuts(ctx);
        let narrow = ctx.screen_rect().width() < 820.0;
        let short = ctx.screen_rect().height() < 560.0;
        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(if narrow { 164.0 } else { 212.0 })
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::symmetric(10, if short { 10 } else { 16 })),
            )
            .show(ctx, |ui| self.sidebar(ui, short));
        egui::TopBottomPanel::bottom("status")
            .exact_height(36.0)
            .frame(
                egui::Frame::new()
                    .fill(self.status_fill())
                    .inner_margin(Margin::symmetric(20, 6)),
            )
            .show(ctx, |ui| self.status_strip(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin {
                left: 24,
                right: 8,
                top: if short { 10 } else { 14 },
                bottom: 0,
            }))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .inner_margin(Margin {
                        left: 0,
                        right: 16,
                        top: 0,
                        bottom: 10,
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                        self.top_bar(ui)
                    });
                // Settle from partial opacity: a page switch never blanks the
                // window for a frame, it only eases the new content in.
                let fade = 0.55
                    + 0.45 * animation::fade_in(ctx, self.page_since.elapsed(), animation::PAGE);
                egui::ScrollArea::vertical()
                    .id_salt(self.page.title())
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(Margin {
                                left: 0,
                                right: 16,
                                top: 2,
                                bottom: 24,
                            })
                            .show(ui, |ui| {
                                ui.set_opacity(fade);
                                ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                                ui.spacing_mut().item_spacing.y = 14.0;
                                // No page-wide disable while an action runs: egui
                                // would repaint every widget faded for the whole
                                // round trip. The acting button shows progress and
                                // `request` refuses duplicates.
                                if self.preview && self.preview_airplay_only {
                                    self.airplay_panel(ui);
                                } else {
                                    match self.page {
                                        Page::Hub => self.hub_page(ui),
                                        Page::Sender => self.sender_page(ui),
                                        Page::Mixer => self.mixer_page(ui),
                                        Page::Devices => self.devices_page(ui),
                                        Page::Diagnostics => self.diagnostics_page(ui),
                                    }
                                }
                            });
                    });
            });
        self.confirm_modal(ctx);
        self.palette_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    pub(crate) fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            self.page_since = Instant::now();
            // Mixer meters poll faster; refresh right away instead of after 1 s.
            if page == Page::Mixer {
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
        }
    }

    /// Window-level keys. Single-letter mixer keys live in the mixer page and
    /// never fire while a text field has focus.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm.is_some() || self.palette.is_some() {
            return;
        }
        let command = egui::Modifiers::COMMAND;
        if ctx.input_mut(|i| i.consume_key(command, egui::Key::K)) {
            self.open_palette();
            return;
        }
        for (i, key) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
        ]
        .into_iter()
        .enumerate()
        {
            if ctx.input_mut(|input| input.consume_key(command, key)) {
                self.navigate(Page::ALL[i].0);
            }
        }
        if !ctx.wants_keyboard_input()
            && self.undo_available()
            && ctx.input_mut(|i| i.consume_key(command, egui::Key::Z))
        {
            self.restore_last();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, short: bool) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            for (i, h) in [7.0, 15.0, 11.0, 5.0].into_iter().enumerate() {
                let x = rect.left() + 1.5 + i as f32 * 4.5;
                ui.painter().line_segment(
                    [
                        egui::pos2(x, rect.center().y - h / 2.0),
                        egui::pos2(x, rect.center().y + h / 2.0),
                    ],
                    Stroke::new(2.5, theme::ACCENT),
                );
            }
            ui.label(
                RichText::new("NeonMix")
                    .font(theme::heading(17.0))
                    .color(theme::TEXT),
            );
        });
        ui.add_space(if short { 10.0 } else { 18.0 });

        let item_height = if short { 32.0 } else { 36.0 };
        let item_width = ui.available_width();
        let mut selected_top = None;
        let first_top = ui.cursor().top();
        for (i, (page, title)) in Page::ALL.into_iter().enumerate() {
            let selected = self.page == page;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(item_width, item_height), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, title)
            });
            let hover = ui.ctx().animate_bool_with_time(
                response.id.with("hover"),
                response.hovered() || response.has_focus(),
                animation::FAST,
            );
            let active = ui.ctx().animate_bool_with_time(
                response.id.with("active"),
                selected,
                animation::PAGE,
            );
            let fill = animation::lerp_color(
                animation::lerp_color(
                    egui::Color32::TRANSPARENT,
                    theme::HOVER.gamma_multiply(0.6),
                    hover,
                ),
                theme::ACCENT.gamma_multiply(0.13),
                active,
            );
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), fill);
            if response.has_focus() {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    CornerRadius::same(theme::CONTROL_RADIUS),
                    Stroke::new(1.5, theme::ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            let ink = animation::lerp_color(theme::TEXT_3, theme::ACCENT, active);
            icons::paint(
                painter,
                icons::square(egui::pos2(rect.left() + 22.0, rect.center().y), 16.0),
                page.icon(),
                animation::lerp_color(ink, theme::TEXT_2, hover * (1.0 - active)),
            );
            painter.text(
                egui::pos2(rect.left() + 40.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                if selected {
                    theme::heading(theme::BODY)
                } else {
                    egui::FontId::proportional(theme::BODY)
                },
                animation::lerp_color(theme::TEXT_2, theme::TEXT, active.max(hover * 0.6)),
            );
            if !short && item_width > 180.0 {
                painter.text(
                    egui::pos2(rect.right() - 10.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    widgets::command_hint(&(i + 1).to_string()),
                    egui::FontId::monospace(10.5),
                    theme::TEXT_3.gamma_multiply(0.5 + 0.5 * hover),
                );
            }
            if selected {
                selected_top = Some(rect.top());
            }
            if self.focus_after_modal && self.confirm.is_none() && selected {
                response.request_focus();
                self.focus_after_modal = false;
            }
            if response.clicked() {
                self.navigate(page);
            }
            ui.add_space(2.0);
        }
        // Indicator slides between items instead of jumping.
        if let Some(top) = selected_top {
            let y = ui.ctx().animate_value_with_time(
                ui.id().with("nav-indicator"),
                top - first_top,
                0.18,
            );
            let left = ui.min_rect().left();
            let bar = egui::Rect::from_min_size(
                egui::pos2(left, first_top + y + 9.0),
                egui::vec2(3.0, item_height - 18.0),
            );
            ui.painter()
                .rect_filled(bar, CornerRadius::same(2), theme::ACCENT);
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let width = ui.available_width();
            let full = |ui: &mut egui::Ui, text: &str, kind: Kind, busy: bool| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, theme::CONTROL_HEIGHT),
                    Layout::top_down_justified(Align::Min),
                    |ui| widgets::button_busy(ui, true, busy, text, kind),
                )
                .inner
            };
            let quit = full(ui, "退出后台", Kind::Quiet, false);
            if quit.clicked() {
                self.confirmation(
                    "退出后台".into(),
                    "停止本实例的共享、发送和全部音频连接。关闭窗口不会停止音频；退出后台会。"
                        .into(),
                    Request::Shutdown,
                    quit.id,
                );
            }
            if full(ui, "隐藏窗口", Kind::Secondary, false).clicked() {
                if self.tray.is_some() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                } else {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            if self.status.as_ref().is_some_and(|s| s.sender.running)
                && full(ui, "停止发送", Kind::Danger, self.pending_stop).clicked()
            {
                self.stop_sender();
            }
            ui.add_space(6.0);
            self.identity(ui, width, short);
        });
    }

    /// Identity switcher at the foot of the sidebar: who this window acts as.
    fn identity(&mut self, ui: &mut egui::Ui, width: f32, short: bool) {
        let previous = self.credential.clone();
        let (role, tone) = match self.role() {
            Some(Role::Admin) => ("管理员", Tone::Accent),
            Some(Role::Controller) => ("房间控制者", Tone::Accent),
            Some(Role::Member) => ("成员", Tone::Neutral),
            None => ("身份未验证", Tone::Warning),
        };
        let selected = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .map_or("未配对".to_owned(), profile_name);
        // Fixed height: inside the bottom-up stack an unsized child would
        // overlap the buttons below it.
        // Short windows drop the caption row; the combo keeps its name.
        ui.allocate_ui_with_layout(
            egui::vec2(width, if short { 32.0 } else { 60.0 }),
            Layout::top_down(Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let label = (!short).then(|| {
                    ui.horizontal(|ui| {
                        let label = widgets::caption(ui, "控制身份");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            widgets::pill(ui, role, tone);
                        });
                        label
                    })
                    .inner
                });
                ui.spacing_mut().interact_size.y = 30.0;
                ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
                let response = egui::ComboBox::from_id_salt("credential")
                    .width(width - 4.0)
                    .selected_text(RichText::new(selected).size(theme::SMALL + 1.0))
                    .show_ui(ui, |ui| {
                        if let Some(status) = &self.status {
                            for p in status.profiles.iter().filter(|p| !p.pending) {
                                widgets::select_value(
                                    ui,
                                    &mut self.credential,
                                    p.credential.clone(),
                                    profile_name(p),
                                );
                            }
                        }
                    })
                    .response
                    .on_hover_text(format!("当前身份：{role}"));
                let response = match label {
                    Some(label) => response.labelled_by(label.id),
                    None => response,
                };
                widgets::label_combo(&response, "控制身份");
            },
        );
        if previous != self.credential {
            self.snapshot = None;
            self.diagnostics = None;
            self.fresh = None;
            self.synced = None;
            self.last_poll = Instant::now() - Duration::from_secs(5);
        }
    }

    /// Room at a glance on every page: title, room state, a live master
    /// meter (except on the mixer, which shows it large) and ⌘K.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let title = self.page.title();
        let status = self.status.as_ref();
        let room = self
            .remote_room
            .clone()
            .or_else(|| {
                status
                    .and_then(|s| s.hub_settings.as_ref())
                    .map(|h| h.name.clone())
            })
            .unwrap_or_else(|| "未连接房间".into());
        let (room_state, room_tone) = if status.is_some_and(|s| s.hub.running) {
            ("共享中", Tone::Success)
        } else if self.snapshot.is_some() && !self.writable() {
            ("状态已过期", Tone::Warning)
        } else if self.snapshot.is_some() {
            ("已连接", Tone::Success)
        } else {
            ("未连接", Tone::Neutral)
        };
        let streams =
            self.snapshot.as_ref().map_or(0, |s| s.streams.len()) + self.airplay_sessions().len();
        let known_room =
            self.remote_room.is_some() || status.is_some_and(|s| s.hub_settings.is_some());
        let chip = if !known_room {
            room
        } else if streams > 0 {
            format!("{room} · {room_state} · {streams} 路输入")
        } else {
            format!("{room} · {room_state}")
        };
        let left = |ui: &mut egui::Ui| {
            ui.label(
                RichText::new(title)
                    .font(theme::heading(theme::TITLE))
                    .color(theme::TEXT),
            );
            ui.add_space(6.0);
            let _ = widgets::pill(ui, &chip, room_tone)
                .on_hover_text("当前控制身份所连接的房间及其状态");
        };
        if ui.available_width() >= 640.0 {
            ui.horizontal(|ui| {
                left(ui);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.top_bar_actions(ui)
                });
            });
        } else {
            ui.horizontal_wrapped(left);
            ui.horizontal_wrapped(|ui| {
                // Left-to-right here: present the same items in reading order.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.top_bar_actions(ui)
                });
            });
        }
        if !self.cjk {
            ui.add_space(6.0);
            widgets::pill(
                ui,
                "中文字体未找到，请配置 NEONMIX_CJK_FONT 后重新打开。",
                Tone::Danger,
            );
        }
    }

    /// Right-to-left: ⌘K first (rightmost), then the master glance.
    fn top_bar_actions(&mut self, ui: &mut egui::Ui) {
        let palette = widgets::button(
            ui,
            &format!("快速操作  {}", widgets::command_hint("K")),
            Kind::Secondary,
        )
        .on_hover_text("搜索并执行操作、跳转页面、控制通道");
        if palette.clicked() {
            self.open_palette();
        }
        if self.page == Page::Mixer {
            return;
        }
        let Some(state) = self.snapshot.clone() else {
            return;
        };
        ui.add_space(10.0);
        if self.controls_room() {
            let muted = state.output.muted;
            if widgets::toggle(
                ui,
                self.writable(),
                muted,
                if muted {
                    "取消总静音"
                } else {
                    "总静音"
                },
                Tone::Warning,
            )
            .clicked()
            {
                self.master_mute(!muted);
            }
        }
        ui.label(
            RichText::new(format!("{} dB", widgets::gain_text(state.output.gain_db)))
                .monospace()
                .size(theme::MONO)
                .color(theme::TEXT_2),
        );
        let meter = self
            .diagnostics
            .as_ref()
            .and_then(|v| v.pointer("/meters/output"));
        let (peak, rms) = (
            meter.and_then(|v| v["peak"].as_f64()),
            meter.and_then(|v| v["rms"].as_f64()),
        );
        let level = widgets::level(ui, "topbar-master", egui::vec2(96.0, 6.0), peak, rms)
            .interact(egui::Sense::click())
            .on_hover_text("房间总输出电平（限幅后）· 点击打开 Mixer");
        if level.clicked() {
            self.navigate(Page::Mixer);
        }
    }

    pub(crate) fn master_mute(&mut self, on: bool) {
        self.mix_change(
            if on { "总静音" } else { "取消总静音" }.into(),
            Write::Control(Operation::OutputMix {
                gain_db: None,
                muted: Some(on),
            }),
            Write::Control(Operation::OutputMix {
                gain_db: None,
                muted: Some(!on),
            }),
        );
    }

    /// Steady fill; errors keep a quiet red tint instead of a flash.
    fn status_fill(&self) -> egui::Color32 {
        if self.error {
            animation::lerp_color(theme::SIDEBAR, theme::DANGER, 0.08)
        } else {
            theme::SIDEBAR
        }
    }

    fn status_strip(&mut self, ui: &mut egui::Ui) {
        // New text eases in; nothing else in the strip moves or blinks.
        let appear = 0.35 + 0.65 * animation::fade_in(ui.ctx(), self.message_since.elapsed(), 0.22);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let status = self.status.as_ref();
            if status.is_some_and(|s| s.sender.running) {
                widgets::dot(ui, "发送中", Tone::Success);
            }
            match status.map(|s| s.hub.running) {
                Some(true) => widgets::dot(ui, "Hub 共享中", Tone::Success),
                Some(false) => widgets::dot(ui, "Hub 未共享", Tone::Neutral),
                None => widgets::dot(ui, "Hub 未知", Tone::Neutral),
            };
            if self.preview {
                widgets::dot(ui, "预览", Tone::Accent);
            } else if self.online {
                widgets::dot(ui, "后台在线", Tone::Success);
            } else {
                widgets::dot(ui, "后台离线", Tone::Warning);
            }
            // 还原: the last mixer change, for a few seconds.
            if self.undo_available() {
                ui.ctx().request_repaint_after(Duration::from_millis(500));
                if widgets::small_button(ui, true, &format!("还原  {}", widgets::command_hint("Z")))
                    .on_hover_text("恢复上一次混音调整前的值")
                    .clicked()
                {
                    self.restore_last();
                }
                if let Some(undo) = &self.undo {
                    ui.label(
                        RichText::new(format!("已调整：{}", undo.label))
                            .size(theme::SMALL)
                            .color(theme::TEXT_2),
                    );
                }
            }
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().max(0.0), 22.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Fixed slot so the message never jumps when work starts.
                    let (slot, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    if self.busy && !self.polling || self.pending_stop {
                        egui::Spinner::new()
                            .size(12.0)
                            .color(theme::ACCENT)
                            .paint_at(ui, slot);
                    } else if self.error {
                        ui.painter()
                            .circle_filled(slot.center(), 3.5, theme::DANGER);
                    }
                    let color = if self.error {
                        theme::DANGER
                    } else {
                        theme::TEXT_2
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.message)
                                .size(theme::SMALL + 0.5)
                                .color(color.gamma_multiply(appear)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&self.message);
                },
            );
        });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(mut confirmation) = self.confirm.take() else {
            return;
        };
        let mut keep = true;
        let mut execute = false;
        let busy = self.action_busy();
        let response = egui::Modal::new(egui::Id::new("confirm"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                    .inner_margin(Margin::same(22))
                    .corner_radius(CornerRadius::same(14)),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 64.0).min(420.0));
                ui.label(
                    RichText::new(&confirmation.label)
                        .font(theme::heading(17.0))
                        .color(theme::TEXT),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(&confirmation.consequence).color(theme::TEXT_2));
                ui.add_space(16.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let action = confirmation.action_label().to_owned();
                    if widgets::button_enabled(ui, !busy, &action, Kind::Danger).clicked() {
                        execute = true;
                        keep = false;
                    }
                    let cancel = widgets::button(ui, "取消", Kind::Secondary);
                    if confirmation.focus {
                        cancel.request_focus();
                        confirmation.focus = false;
                    }
                    if cancel.clicked() {
                        keep = false;
                    }
                });
            });
        if response.should_close() {
            keep = false;
        }
        if keep {
            self.confirm = Some(confirmation);
        } else {
            self.focus_after_modal = true;
            if execute {
                self.request(confirmation.request);
            }
        }
    }
}

pub(crate) fn profile_name(p: &neonmix_desktop_service::ProfileInfo) -> String {
    if p.role == Some(Role::Admin) && p.credential == std::path::Path::new("hub/admin.json") {
        "本地管理员".into()
    } else {
        p.name.clone().unwrap_or_else(|| "已配对设备".into())
    }
}
