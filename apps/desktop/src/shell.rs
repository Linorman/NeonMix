//! Window frame: a sidebar that names the room, groups the pages and holds
//! identity and process controls; a top bar with the page title, quick
//! actions and the room master; the page body; a status strip; the 还原
//! toast, the confirmation modal and the command palette.
use super::*;
use crate::fx;
use crate::widgets::{Kind, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

/// Wide windows keep line length and meter span readable.
const CONTENT_MAX_WIDTH: f32 = 1180.0;
/// Sidebar width in wide and narrow (< 820 px) windows.
pub(crate) const SIDEBAR_WIDE: f32 = 232.0;
pub(crate) const SIDEBAR_NARROW: f32 = 176.0;

pub(crate) fn sidebar_width(window_width: f32) -> f32 {
    if window_width < 820.0 {
        SIDEBAR_NARROW
    } else {
        SIDEBAR_WIDE
    }
}

/// Navigation groups: listen to the room, connect devices, look after the app.
const NAV_GROUPS: [(Message, &[Page]); 3] = [
    (Message::ShellGroupMonitor, &[Page::Live, Page::Mixer]),
    (
        Message::ShellGroupConnect,
        &[Page::Hub, Page::Sender, Page::Devices],
    ),
    (Message::ShellGroupSystem, &[Page::Diagnostics, Page::About]),
];

impl Desktop {
    pub(crate) fn show(&mut self, ctx: &egui::Context) {
        theme::sync(ctx);
        localization::install(ctx, self.localization.renderer.clone());
        self.process();
        if self.close_if_exiting(ctx) {
            return;
        }
        self.track_events();
        if self.shown_message.as_ref() != Some(&self.message) {
            // A new error replacing an error (polls alternate between causes
            // while the room is unreachable) updates in place; fading it in
            // again every second read as the strip blinking.
            if !(self.error && self.shown_error) {
                self.message_since = Instant::now();
            }
            self.shown_message = Some(self.message.clone());
            self.shown_error = self.error;
        }
        self.shortcuts(ctx);
        let short = ctx.screen_rect().height() < 560.0;
        let sidebar = sidebar_width(ctx.screen_rect().width());
        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(sidebar)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::sidebar())
                    .inner_margin(Margin::symmetric(12, if short { 10 } else { 16 })),
            )
            .show(ctx, |ui| {
                // Hairline where the sidebar meets the page.
                let r = ui.max_rect().expand2(egui::vec2(12.0, 16.0));
                ui.painter().line_segment(
                    [r.right_top(), r.right_bottom()],
                    Stroke::new(1.0, theme::border()),
                );
                self.sidebar(ui, short)
            });
        egui::TopBottomPanel::bottom("status")
            .exact_height(36.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(self.status_fill())
                    .inner_margin(Margin::symmetric(20, 2)),
            )
            .show(ctx, |ui| self.status_strip(ui));
        let page = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg()).inner_margin(Margin {
                left: 28,
                right: 8,
                top: if short { 10 } else { 16 },
                bottom: 0,
            }))
            .show(ctx, |ui| {
                // Ambient light that names the room state; it only moves
                // when that state changes.
                let ambient = if self.error {
                    theme::danger().gamma_multiply(0.05)
                } else if self.status.as_ref().is_some_and(|s| s.hub.running)
                    || self.snapshot.is_some()
                {
                    theme::accent().gamma_multiply(0.04)
                } else {
                    egui::Color32::TRANSPARENT
                };
                let ambient = animation::color(ctx, egui::Id::new("ambient"), ambient);
                if ambient.a() > 0 {
                    let r = ui.max_rect();
                    crate::fx::glow(
                        ui.painter(),
                        r.left_top() + egui::vec2(220.0, 40.0),
                        360.0,
                        ambient,
                    );
                }
                egui::Frame::new()
                    .inner_margin(Margin {
                        left: 0,
                        right: 16,
                        top: 0,
                        bottom: if short { 8 } else { 14 },
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                        self.top_bar(ui)
                    });
                // Navigation replaces the content at full opacity and its final
                // position. Fading/sliding the whole page reads as a flash on
                // every navigation click, even when all controls stay enabled.
                egui::ScrollArea::vertical()
                    .id_salt(self.page)
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
                                ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                                ui.spacing_mut().item_spacing.y = 16.0;
                                // No page-wide disable while an action runs: egui
                                // would repaint every widget faded for the whole
                                // round trip. The acting button shows progress and
                                // `request` refuses duplicates.
                                if self.preview && self.preview_airplay_only {
                                    self.airplay_panel(ui);
                                } else {
                                    match self.page {
                                        Page::Live => self.live_page(ui),
                                        Page::Hub => self.hub_page(ui),
                                        Page::Sender => self.sender_page(ui),
                                        Page::Mixer => self.mixer_page(ui),
                                        Page::Devices => self.devices_page(ui),
                                        Page::Diagnostics => self.diagnostics_page(ui),
                                        Page::About => self.about_page(ui),
                                    }
                                }
                            });
                    });
            });
        self.undo_toast(ctx, page.response.rect);
        self.confirm_modal(ctx);
        self.palette_ui(ctx);
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    pub(crate) fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            if page == Page::About && !self.preview {
                self.localization.detect(
                    &self.preferences.value.language,
                    &localization::NativeLocaleProvider,
                );
            }
            // Mixer meters poll faster; refresh right away instead of after 1 s.
            if page == Page::Mixer {
                self.last_poll = Instant::now() - Duration::from_secs(5);
            }
        }
    }

    /// Window-level keys. Single-letter mixer keys live in the mixer page and
    /// never fire while a text field has focus.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm.is_some() || self.palette.is_some() || self.localization.composing {
            return;
        }
        let command = egui::Modifiers::COMMAND;
        if !self.localization.composing && ctx.input_mut(|i| i.consume_key(command, egui::Key::K)) {
            self.open_palette();
            return;
        }
        for (i, key) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
        ]
        .into_iter()
        .enumerate()
        {
            if ctx.input_mut(|input| input.consume_key(command, key)) {
                self.navigate(Page::ALL[i].0);
            }
        }
        if ctx.input_mut(|input| input.consume_key(command, egui::Key::Comma)) {
            self.navigate(Page::About);
        }
        if !ctx.wants_keyboard_input()
            && self.undo_available()
            && ctx.input_mut(|i| i.consume_key(command, egui::Key::Z))
        {
            self.restore_last();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, short: bool) {
        let width = ui.available_width();
        self.brand(ui);
        ui.add_space(if short { 6.0 } else { 12.0 });
        self.room_card(ui, short);
        ui.add_space(if short { 2.0 } else { 4.0 });

        let item_height = if short { 28.0 } else { 34.0 };
        ui.spacing_mut().item_spacing.y = 0.0;
        let pill_slot = ui.painter().add(egui::Shape::Noop);
        let first_top = ui.cursor().top();
        let mut selected_rect = None;
        for (group, pages) in NAV_GROUPS {
            if short {
                ui.add_space(5.0);
            } else {
                ui.add_space(12.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
                ui.painter().text(
                    egui::pos2(rect.left() + 10.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    self.tr(&group),
                    theme::heading(theme::GROUP),
                    theme::text_3().gamma_multiply(0.85),
                );
                ui.add_space(4.0);
            }
            for &page in pages {
                let index = Page::ALL.iter().position(|p| p.0 == page).unwrap_or(0);
                let title = self.tr(&page.title());
                let selected = self.page == page;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, item_height), egui::Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &title)
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
                let painter = ui.painter();
                if hover > 0.0 && !selected {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(theme::CONTROL_RADIUS),
                        theme::text().gamma_multiply(0.05 * hover),
                    );
                }
                if response.has_focus() {
                    painter.rect_stroke(
                        rect.shrink(1.0),
                        CornerRadius::same(theme::CONTROL_RADIUS),
                        Stroke::new(1.5, theme::accent()),
                        egui::StrokeKind::Inside,
                    );
                }
                let ink = animation::lerp_color(theme::text_3(), theme::accent(), active);
                icons::paint(
                    painter,
                    icons::square(egui::pos2(rect.left() + 20.0, rect.center().y), 16.0),
                    page.icon(),
                    animation::lerp_color(ink, theme::text_2(), hover * (1.0 - active)),
                );
                painter.text(
                    egui::pos2(rect.left() + 38.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &title,
                    if selected {
                        theme::heading(theme::BODY)
                    } else {
                        egui::FontId::proportional(theme::BODY)
                    },
                    animation::lerp_color(theme::text_2(), theme::text(), active.max(hover * 0.6)),
                );
                let hint = match index {
                    0..=5 => Some(widgets::command_hint(&(index + 1).to_string())),
                    _ => Some(widgets::command_hint(",")),
                };
                if let Some(hint) = hint.filter(|_| !short && width > 190.0) {
                    painter.text(
                        egui::pos2(rect.right() - 10.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        hint,
                        egui::FontId::monospace(10.5),
                        theme::text_3().gamma_multiply(0.45 + 0.55 * hover.max(active)),
                    );
                }
                if selected {
                    selected_rect = Some(rect);
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
        }
        // The selected item rises out of the sidebar; the cap slides between
        // items instead of jumping.
        if let Some(rect) = selected_rect {
            let y = animation::ease_to(
                ui.ctx(),
                ui.id().with("nav-indicator"),
                rect.top() - first_top,
                0.22,
            );
            let pill =
                egui::Rect::from_min_size(egui::pos2(rect.left(), first_top + y), rect.size());
            ui.painter().set(
                pill_slot,
                egui::Shape::Vec(fx::elevated(
                    ui.ctx(),
                    pill,
                    theme::CONTROL_RADIUS,
                    theme::raised(),
                    fx::Level::Control,
                    None,
                )),
            );
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            self.identity(ui, width, short);
            if self.status.as_ref().is_some_and(|s| s.sender.running) {
                self.sending_card(ui, width, short);
            }
        });
    }

    /// Product mark: four bars that follow the master RMS through meter
    /// ballistics and rest in the logo shape in silence.
    fn brand(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            let rms = self
                .meters_current()
                .and_then(|v| v.pointer("/output/rms"))
                .and_then(Value::as_f64)
                .filter(|_| self.snapshot.as_ref().is_some_and(|s| s.output.available));
            let level = rms.and_then(widgets::to_db).map(|db| {
                let (db, _) = animation::meter(ui.ctx(), egui::Id::new("brand-meter"), db);
                ((db - widgets::METER_FLOOR_DB) / -widgets::METER_FLOOR_DB).clamp(0.0, 1.0)
            });
            let shape = [7.0, 15.0, 11.0, 5.0];
            let live = [0.55, 1.0, 0.8, 0.45];
            for i in 0..4 {
                let h = match level {
                    Some(l) if l > 0.05 => (4.0 + 14.0 * l * live[i]).min(18.0),
                    _ => shape[i],
                };
                let x = rect.left() + 1.5 + i as f32 * 4.5;
                let color = if i < 2 {
                    theme::src_native()
                } else {
                    theme::src_airplay()
                };
                ui.painter().line_segment(
                    [
                        egui::pos2(x, rect.center().y - h / 2.0),
                        egui::pos2(x, rect.center().y + h / 2.0),
                    ],
                    Stroke::new(2.5, color),
                );
            }
            if level.is_some_and(|l| l > 0.05) {
                fx::glow(
                    ui.painter(),
                    rect.center(),
                    14.0,
                    theme::accent().gamma_multiply(0.15),
                );
            }
            ui.label(
                RichText::new("NeonMix")
                    .font(theme::heading(16.0))
                    .color(theme::text()),
            );
        });
    }

    /// The room this window looks at: name, state and live input count.
    fn room_glance(&self) -> (String, String, Tone, Option<uuid::Uuid>) {
        let status = self.status.as_ref();
        let known_room =
            self.remote_room.is_some() || status.is_some_and(|s| s.hub_settings.is_some());
        let room = self
            .remote_room
            .clone()
            .or_else(|| {
                status
                    .and_then(|s| s.hub_settings.as_ref())
                    .map(|h| h.name.clone())
            })
            .unwrap_or_else(|| self.tr(&Message::ShellNoRoom));
        let local_admin = self.credential == std::path::Path::new("hub/admin.json");
        let (state, tone) = if status.is_some_and(|s| s.hub.running) {
            (self.tr(&Message::ShellSharing), Tone::Success)
        } else if local_admin && status.is_some_and(|s| s.hub_settings.is_some()) {
            (self.tr(&Message::ShellNotSharing), Tone::Neutral)
        } else if self.snapshot.is_some() && !self.writable() {
            (self.tr(&Message::ShellStale), Tone::Warning)
        } else if self.snapshot.is_some() {
            (self.tr(&Message::ShellRoomConnected), Tone::Success)
        } else {
            (self.tr(&Message::ShellRoomDisconnected), Tone::Neutral)
        };
        let streams =
            self.snapshot.as_ref().map_or(0, |s| s.streams.len()) + self.airplay_sessions().len();
        let state = if known_room && streams > 0 {
            self.tr(&Message::ShellRoomState {
                state,
                count: streams as u64,
            })
        } else {
            state
        };
        (room, state, tone, self.snapshot.as_ref().map(|s| s.hub_id))
    }

    /// Room card at the top of the sidebar: emblem, name and state on every
    /// page. Opens the room settings.
    fn room_card(&mut self, ui: &mut egui::Ui, short: bool) {
        let (room, state, tone, hub_id) = self.room_glance();
        let tooltip = self.tr(&Message::ShellRoomTooltip);
        let open = self.tr(&Message::ShellOpenRoom);
        let width = ui.available_width();
        let height = if short { 40.0 } else { 52.0 };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{room} · {state}"))
        });
        let hover = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            animation::FAST,
        );
        let base = animation::lerp_color(theme::surface(), theme::raised(), hover);
        ui.painter().extend(fx::elevated(
            ui.ctx(),
            rect,
            12,
            base,
            fx::Level::Card,
            None,
        ));
        let painter = ui.painter();
        let size = if short { 26.0 } else { 32.0 };
        let center = egui::pos2(rect.left() + 10.0 + size / 2.0, rect.center().y);
        match hub_id {
            Some(id) => {
                crate::emblem::paint(
                    painter,
                    center,
                    size / 2.0,
                    crate::emblem::Emblem::from_id(id),
                    1.0,
                );
            }
            None => {
                fx::dashed(
                    painter,
                    &(0..=40)
                        .map(|i| {
                            center
                                + egui::Vec2::angled(i as f32 / 40.0 * std::f32::consts::TAU)
                                    * (size / 2.0 - 1.0)
                        })
                        .collect::<Vec<_>>(),
                    Stroke::new(1.2, theme::text_3()),
                    3.0,
                    3.0,
                );
            }
        }
        let left = center.x + size / 2.0 + 10.0;
        let max = (rect.right() - 10.0 - left).max(1.0);
        let line = |text: &str, font: egui::FontId, color: egui::Color32| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max);
            job.wrap.max_rows = 1;
            painter.layout_job(job)
        };
        let name = line(&room, theme::heading(13.5), theme::text());
        if short {
            painter.galley(
                egui::pos2(left, rect.center().y - name.size().y / 2.0),
                name,
                theme::text(),
            );
        } else {
            let detail = line(
                &state,
                egui::FontId::proportional(11.5),
                if tone == Tone::Neutral {
                    theme::text_3()
                } else {
                    tone.color()
                },
            );
            let top = rect.center().y - (name.size().y + detail.size().y + 2.0) / 2.0;
            painter.galley(egui::pos2(left, top), name.clone(), theme::text());
            let y = top + name.size().y + 2.0;
            painter.circle_filled(
                egui::pos2(left + 3.0, y + detail.size().y / 2.0),
                3.0,
                tone.color(),
            );
            painter.galley(egui::pos2(left + 10.0, y), detail, theme::text_3());
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        widgets::focus_ring(ui, &response);
        let response = response.on_hover_text(format!("{room}\n{state}\n{tooltip}\n{open}"));
        if response.clicked() {
            self.navigate(Page::Hub);
        }
    }

    /// Shown only while this device sends: what is happening and the one
    /// way to stop it.
    fn sending_card(&mut self, ui: &mut egui::Ui, width: f32, short: bool) {
        let stop = self.tr(&Message::ShellStopSending);
        let sending = self.tr(&Message::ShellSendingNow);
        let height = if short { 34.0 } else { 40.0 };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter().extend(fx::elevated(
            ui.ctx(),
            rect,
            11,
            theme::surface(),
            fx::Level::Card,
            None,
        ));
        let dot = egui::pos2(rect.left() + 14.0, rect.center().y);
        fx::glow(
            ui.painter(),
            dot,
            6.0,
            theme::src_native().gamma_multiply(0.45),
        );
        ui.painter().circle_filled(dot, 3.5, theme::src_native());
        let button = height - 12.0;
        let max = rect.width() - 26.0 - button - 16.0;
        let mut job = egui::text::LayoutJob::simple(
            sending,
            egui::FontId::proportional(theme::SMALL + 0.5),
            theme::text(),
            max.max(1.0),
        );
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(
            egui::pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text(),
        );
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 6.0 - button / 2.0, rect.center().y),
            egui::vec2(button, button),
        );
        let clicked = ui
            .scope_builder(egui::UiBuilder::new().max_rect(slot), |ui| {
                widgets::icon_button(
                    ui,
                    icons::Icon::Stop,
                    &stop,
                    Kind::Quiet,
                    button,
                    self.pending_stop,
                )
                .clicked()
            })
            .inner;
        if clicked {
            self.stop_sender();
        }
    }

    fn hide_window(&self, ctx: &egui::Context) {
        if self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    /// Who this window acts as, at the foot of the sidebar. The row opens a
    /// menu to switch identity, hide the window or quit background audio.
    fn identity(&mut self, ui: &mut egui::Ui, width: f32, short: bool) {
        let shell_identity = self.tr(&Message::ShellIdentity);
        let shell_hide = self.tr(&Message::ShellHide);
        let shell_quit = self.tr(&Message::ShellQuit);
        let previous = self.credential.clone();
        // Display only: without live room state, show the role saved with
        // this identity (neutral) instead of warning "unverified" while a
        // local room is simply not sharing. Permissions still use `role()`.
        let saved = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .and_then(|p| p.role);
        let (role, tone) = match (self.role(), saved) {
            (Some(Role::Admin), _) => (self.tr(&Message::ShellRoleAdmin), Tone::Accent),
            (Some(Role::Controller), _) => (self.tr(&Message::ShellRoleController), Tone::Accent),
            (Some(Role::Member), _) => (self.tr(&Message::ShellRoleMember), Tone::Neutral),
            (None, Some(Role::Admin)) => (self.tr(&Message::ShellRoleAdmin), Tone::Neutral),
            (None, Some(Role::Controller)) => {
                (self.tr(&Message::ShellRoleController), Tone::Neutral)
            }
            (None, Some(Role::Member)) => (self.tr(&Message::ShellRoleMember), Tone::Neutral),
            (None, None) => (self.tr(&Message::ShellUnverified), Tone::Warning),
        };
        let selected = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .map_or(self.tr(&Message::ShellUnpaired), |p| {
                profile_name(p, &self.localization.renderer)
            });
        let height = if short { 34.0 } else { 42.0 };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &shell_identity)
        });
        let popup = egui::Id::new("identity-menu");
        let open = ui.memory(|m| m.is_popup_open(popup));
        let hover = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered() || open,
            animation::FAST,
        );
        let painter = ui.painter();
        if hover > 0.0 {
            painter.rect_filled(
                rect,
                CornerRadius::same(theme::CONTROL_RADIUS + 1),
                theme::text().gamma_multiply(0.05 * hover),
            );
        }
        let avatar = egui::pos2(rect.left() + 8.0 + 14.0, rect.center().y);
        painter.circle_filled(avatar, 14.0, theme::text().gamma_multiply(0.09));
        painter.text(
            avatar,
            egui::Align2::CENTER_CENTER,
            selected
                .trim()
                .chars()
                .next()
                .map(|c| c.to_uppercase().collect::<String>())
                .unwrap_or_else(|| "?".into()),
            theme::heading(12.0),
            theme::text_2(),
        );
        let left = avatar.x + 22.0;
        let max = (rect.right() - 30.0 - left).max(1.0);
        let line = |text: &str, font: egui::FontId, color: egui::Color32| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max);
            job.wrap.max_rows = 1;
            painter.layout_job(job)
        };
        let name = line(&selected, egui::FontId::proportional(13.0), theme::text());
        if short {
            painter.galley(
                egui::pos2(left, rect.center().y - name.size().y / 2.0),
                name,
                theme::text(),
            );
        } else {
            let detail = line(
                &role,
                egui::FontId::proportional(11.0),
                if tone == Tone::Warning {
                    theme::warning()
                } else {
                    theme::text_3()
                },
            );
            let top = rect.center().y - (name.size().y + detail.size().y + 1.0) / 2.0;
            painter.galley(egui::pos2(left, top), name.clone(), theme::text());
            painter.galley(
                egui::pos2(left, top + name.size().y + 1.0),
                detail,
                theme::text_3(),
            );
        }
        icons::paint(
            painter,
            icons::square(egui::pos2(rect.right() - 16.0, rect.center().y), 14.0),
            icons::Icon::More,
            theme::text_3(),
        );
        widgets::focus_ring(ui, &response);
        let response =
            response.on_hover_text(self.tr(&Message::ShellCurrentIdentity { role: role.clone() }));
        if response.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup));
        }
        let mut hide = false;
        let mut quit = false;
        egui::popup::popup_above_or_below_widget(
            ui,
            popup,
            &response,
            egui::AboveOrBelow::Above,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(width.max(200.0));
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    RichText::new(self.tr(&Message::ShellSwitchIdentity))
                        .size(theme::GROUP)
                        .color(theme::text_3()),
                );
                if let Some(status) = &self.status {
                    for p in status.profiles.iter().filter(|p| !p.pending) {
                        let name = profile_name(p, &self.localization.renderer);
                        let current = p.credential == self.credential;
                        if widgets::menu_item(ui, None, &name, current, false).clicked() {
                            self.credential = p.credential.clone();
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);
                if widgets::menu_item(ui, Some(icons::Icon::Hide), &shell_hide, false, false)
                    .clicked()
                {
                    hide = true;
                }
                if widgets::menu_item(ui, Some(icons::Icon::Power), &shell_quit, false, true)
                    .clicked()
                {
                    quit = true;
                }
            },
        );
        if hide || quit {
            ui.memory_mut(|m| m.close_popup());
        }
        if hide {
            self.hide_window(ui.ctx());
        }
        if quit {
            self.confirmation(
                Message::ShellQuit,
                Message::ShellQuitConsequence,
                Request::Shutdown,
                response.id,
            );
        }
        if previous != self.credential {
            self.snapshot = None;
            self.diagnostics = None;
            self.events.clear();
            self.level_history.clear();
            self.metric_history.clear();
            self.fresh = None;
            self.synced = None;
            self.last_poll = Instant::now() - Duration::from_secs(5);
        }
    }

    /// Page title on the left; quick actions and, except on the mixer which
    /// shows it large, the room master on the right.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let title = self.tr(&self.page.title());
        let left = |ui: &mut egui::Ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(&title)
                        .font(theme::heading(theme::TITLE))
                        .color(theme::text()),
                )
                .truncate(),
            );
        };
        let actions_width = self.top_bar_actions_width(ui, false);
        let title_width = ui.fonts(|f| {
            f.layout_no_wrap(title.clone(), theme::heading(theme::TITLE), theme::text())
                .size()
                .x
        });
        // Reserve actions first, using the actual localized labels instead
        // of a fixed breakpoint that only fits one language.
        let compact = ui.available_width() < actions_width + title_width + 24.0;
        let actions_width = self.top_bar_actions_width(ui, compact);
        if ui.available_width() >= actions_width + title_width.min(120.0) + 16.0 {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(actions_width, theme::CONTROL_HEIGHT + 2.0),
                        Layout::right_to_left(Align::Center),
                        |ui| self.top_bar_actions(ui, compact),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT + 2.0),
                        Layout::left_to_right(Align::Center),
                        left,
                    );
                })
            });
        } else {
            ui.horizontal(left);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                self.top_bar_actions(ui, true);
            });
        }
        if !self.cjk {
            ui.add_space(6.0);
            widgets::error_text(
                ui,
                neonmix_i18n::Localizer::new(neonmix_i18n::ResolvedLocale::En)
                    .render(&Message::ShellFontMissing),
            );
        }
    }

    fn master_mute_label(&self) -> String {
        let muted = self.snapshot.as_ref().is_some_and(|s| s.output.muted);
        self.tr(if muted {
            &Message::ShellMasterUnmute
        } else {
            &Message::ShellMasterMute
        })
    }

    fn search_width(compact: bool) -> f32 {
        if compact { 36.0 } else { 232.0 }
    }

    /// Inner width of the master group, or `None` where it is not shown.
    fn master_group_width(&self, ui: &egui::Ui) -> Option<f32> {
        if self.page == Page::Mixer || self.snapshot.is_none() {
            return None;
        }
        let gap = ui.spacing().item_spacing.x;
        let text = |t: String, font: egui::FontId| {
            ui.fonts(|f| f.layout_no_wrap(t, font, theme::text()).size().x)
        };
        let gain = text("−12.0 dB".into(), egui::FontId::monospace(theme::MONO));
        let mut width = 16.0 + gap + 56.0 + gap + gain;
        if self.controls_room() {
            width +=
                gap + text(
                    self.tr(&Message::ShellMasterMute),
                    egui::FontId::proportional(theme::SMALL + 0.5),
                )
                .max(text(
                    self.tr(&Message::ShellMasterUnmute),
                    egui::FontId::proportional(theme::SMALL + 0.5),
                )) + 20.0;
        }
        Some(width.ceil())
    }

    fn top_bar_actions_width(&self, ui: &egui::Ui, compact: bool) -> f32 {
        let gap = ui.spacing().item_spacing.x;
        let mut width = Self::search_width(compact);
        if let Some(group) = self.master_group_width(ui) {
            width += gap + group + 13.0;
        }
        width.ceil()
    }

    /// Search field, then the master group: right-to-left in the bar (so the
    /// master group sits rightmost), left-to-right when the bar wraps.
    fn top_bar_actions(&mut self, ui: &mut egui::Ui, compact: bool) {
        if ui.layout().prefer_right_to_left() {
            self.top_bar_master(ui);
            self.top_bar_search(ui, compact);
        } else {
            self.top_bar_search(ui, compact);
            self.top_bar_master(ui);
        }
    }

    fn top_bar_search(&mut self, ui: &mut egui::Ui, compact: bool) {
        let quick = self.tr(&Message::ShellQuickActions {
            shortcut: widgets::command_hint("K"),
        });
        let quick_tooltip = self.tr(&Message::ShellQuickTooltip);
        let search = widgets::search_field(
            ui,
            Self::search_width(compact),
            &self.tr(&Message::ShellSearchPlaceholder),
            &widgets::command_hint("K"),
            &quick,
        )
        .on_hover_text(&quick_tooltip);
        if search.clicked() {
            self.open_palette();
        }
    }

    fn top_bar_master(&mut self, ui: &mut egui::Ui) {
        let output_tooltip = self.tr(&Message::ShellOutputTooltip);
        if let Some(inner) = self.master_group_width(ui)
            && let Some(state) = self.snapshot.clone()
        {
            let slot = ui.painter().add(egui::Shape::Noop);
            let group = egui::Frame::new()
                .inner_margin(Margin {
                    left: 10,
                    right: 3,
                    top: 3,
                    bottom: 3,
                })
                .show(ui, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(inner, theme::COMPACT_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            let (icon, _) = ui
                                .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            icons::paint(ui.painter(), icon, icons::Icon::Speaker, theme::text_2());
                            let meter = self.meters_current().and_then(|v| v.get("output"));
                            let (peak, rms) = (
                                meter.and_then(|v| v["peak"].as_f64()),
                                meter.and_then(|v| v["rms"].as_f64()),
                            );
                            let level = widgets::level(
                                ui,
                                "topbar-master",
                                egui::vec2(56.0, 6.0),
                                peak,
                                rms,
                            )
                            .interact(egui::Sense::click())
                            .on_hover_text(&output_tooltip);
                            if level.clicked() {
                                self.navigate(Page::Mixer);
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{} dB",
                                    widgets::gain_text(state.output.gain_db)
                                ))
                                .monospace()
                                .size(theme::MONO)
                                .color(theme::text()),
                            );
                            if self.controls_room() {
                                let muted = state.output.muted;
                                if widgets::toggle_small(
                                    ui,
                                    self.writable(),
                                    muted,
                                    &self.master_mute_label(),
                                    Tone::Warning,
                                )
                                .clicked()
                                {
                                    self.master_mute(!muted);
                                }
                            }
                        },
                    );
                });
            ui.painter().set(
                slot,
                egui::Shape::Vec(fx::elevated(
                    ui.ctx(),
                    group.response.rect,
                    theme::CONTROL_RADIUS + 1,
                    theme::raised(),
                    fx::Level::Control,
                    None,
                )),
            );
        }
    }

    pub(crate) fn master_gain(&mut self, current: f32, gain: f32) {
        self.mix_change(
            Message::ShellMasterGain {
                previous: widgets::gain_text(current),
                next: widgets::gain_text(gain),
            },
            Write::Control(Operation::OutputMix {
                gain_db: Some(gain),
                muted: None,
            }),
            Write::Control(Operation::OutputMix {
                gain_db: Some(current),
                muted: None,
            }),
        );
    }

    pub(crate) fn master_mute(&mut self, on: bool) {
        self.mix_change(
            if on {
                Message::ShellMasterMute
            } else {
                Message::ShellMasterUnmute
            },
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
            animation::lerp_color(theme::sidebar(), theme::danger(), 0.08)
        } else {
            theme::sidebar()
        }
    }

    fn status_strip(&mut self, ui: &mut egui::Ui) {
        let shell_sending = crate::localization::renderer(ui.ctx()).render(&Message::ShellSending);
        let shell_hub_sharing =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellHubSharing);
        let shell_hub_stopped =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellHubStopped);
        let shell_hub_unknown =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellHubUnknown);
        let shell_preview_badge =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellPreviewBadge);
        let shell_online = crate::localization::renderer(ui.ctx()).render(&Message::ShellOnline);
        let shell_offline = crate::localization::renderer(ui.ctx()).render(&Message::ShellOffline);
        // New text eases in; nothing else in the strip moves or blinks.
        let appear = 0.35 + 0.65 * animation::fade_in(ui.ctx(), self.message_since.elapsed(), 0.22);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let compact = ui.available_width() < 600.0;
            ui.spacing_mut().item_spacing.x = if compact { 8.0 } else { 14.0 };
            // Secondary process states retain their complete accessible name
            // and tooltip when the strip needs room for feedback and Undo.
            let process_dot = |ui: &mut egui::Ui, text: &str, tone: Tone| {
                let response = widgets::dot(ui, if compact { "" } else { text }, tone);
                response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
                response.on_hover_text(text)
            };
            let status = self.status.as_ref();
            if status.is_some_and(|s| s.sender.running) {
                process_dot(ui, shell_sending.as_str(), Tone::Success);
            }
            match status.map(|s| s.hub.running) {
                Some(true) => widgets::dot(ui, shell_hub_sharing.as_str(), Tone::Success),
                Some(false) => widgets::dot(ui, shell_hub_stopped.as_str(), Tone::Neutral),
                None => widgets::dot(ui, shell_hub_unknown.as_str(), Tone::Neutral),
            };
            if self.preview {
                process_dot(ui, shell_preview_badge.as_str(), Tone::Accent);
            } else if self.online {
                process_dot(ui, shell_online.as_str(), Tone::Success);
            } else {
                process_dot(ui, shell_offline.as_str(), Tone::Warning);
            }
            // 还原: the last mixer change, for a few seconds.
            if self
                .intents
                .unknown
                .iter()
                .any(|flight| self.intents.current.as_ref() == Some(&flight.intent.context))
                && widgets::small_button(ui, self.ready(), &self.tr(&Message::IntentReconcile))
                    .clicked()
            {
                self.reconcile_intent();
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
                            .color(theme::accent())
                            .paint_at(ui, slot);
                    } else if self.error {
                        ui.painter()
                            .circle_filled(slot.center(), 3.5, theme::danger());
                        let code = self
                            .message
                            .id()
                            .strip_prefix("fault-")
                            .map(|id| id.replace('-', "_"))
                            .unwrap_or_else(|| "ui_error".into());
                        let label = self.tr(&Message::ShellCopyErrorCode { code: code.clone() });
                        let response = ui.interact(
                            slot,
                            ui.id().with("copy-error-code"),
                            egui::Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label)
                        });
                        if response.has_focus() {
                            ui.painter().rect_stroke(
                                slot.expand(2.0),
                                CornerRadius::same(3),
                                Stroke::new(1.0, theme::accent()),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if response.clicked() {
                            ui.ctx().copy_text(code);
                            self.copied_at = Some(Instant::now());
                        }
                        let copied = self
                            .copied_at
                            .is_some_and(|at| at.elapsed() < Duration::from_millis(1500));
                        response
                            .on_hover_text(if copied {
                                self.tr(&Message::CommonCopied)
                            } else {
                                label
                            })
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                    }
                    let color = if self.error {
                        theme::danger()
                    } else {
                        theme::text_2()
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.tr(&self.message))
                                .size(theme::SMALL + 0.5)
                                .color(color.gamma_multiply(appear)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(self.tr(&self.message));
                },
            );
        });
    }

    /// 还原: the last mixer change, floating above the page for the 8 s
    /// restore window, with the time left draining along its bottom edge.
    fn undo_toast(&mut self, ctx: &egui::Context, page: egui::Rect) {
        if !self.undo_available() {
            return;
        }
        let Some(undo) = &self.undo else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(100));
        let left = (1.0 - undo.at.elapsed().as_secs_f32() / 8.0).clamp(0.0, 1.0);
        let label = self.tr(&Message::ShellAdjusted {
            change: self.tr(&undo.label),
        });
        let restore_text = self.tr(&Message::ShellRestoreButton {
            shortcut: widgets::command_hint("Z"),
        });
        let restore_tooltip = self.tr(&Message::ShellRestoreTooltip);
        // Measured up front so the toast has its final place on its first
        // frame (an `Area` would spend that frame sizing itself).
        let measure = |text: &str, size: f32| {
            ctx.fonts(|f| {
                f.layout_no_wrap(
                    text.to_owned(),
                    egui::FontId::proportional(size),
                    theme::text(),
                )
                .size()
                .x
            })
        };
        let button_width = measure(&restore_text, theme::SMALL + 0.5) + 20.0;
        let max_width = (page.width() - 48.0).clamp(200.0, 560.0);
        let chrome = 12.0 + 18.0 + 10.0 + 10.0 + button_width + 8.0;
        let text_width = measure(&label, theme::SMALL + 0.5)
            .min(max_width - chrome)
            .max(40.0);
        let size = egui::vec2(chrome + text_width, 42.0);
        let appear = animation::fade_in(ctx, undo.at.elapsed(), 0.16);
        let rect = egui::Rect::from_center_size(
            egui::pos2(
                page.center().x,
                page.bottom() - 20.0 - size.y / 2.0 + 6.0 * (1.0 - appear),
            ),
            size,
        );
        let mut restore = false;
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("undo-toast"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("undo-toast"),
                ))
                .max_rect(rect),
        );
        let shadow = egui::Shadow {
            offset: [0, 14],
            blur: 36,
            spread: 0,
            color: theme::shadow(0.7),
        };
        ui.painter()
            .add(shadow.as_shape(rect, CornerRadius::same(12)));
        ui.painter().rect(
            rect,
            CornerRadius::same(12),
            theme::overlay(),
            Stroke::new(1.0, theme::border_strong()),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(egui::vec2(12.0, 7.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    inner.min,
                    egui::pos2(rect.right() - 8.0, inner.max.y),
                ))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    icon.center(),
                    9.0,
                    theme::success().gamma_multiply(0.16),
                );
                icons::paint(
                    ui.painter(),
                    icons::square(icon.center(), 11.0),
                    icons::Icon::Check,
                    theme::success(),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, 22.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&label)
                                    .size(theme::SMALL + 0.5)
                                    .color(theme::text()),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&label);
                    },
                );
                if widgets::small_button(ui, true, &restore_text)
                    .on_hover_text(&restore_tooltip)
                    .clicked()
                {
                    restore = true;
                }
            },
        );
        let line = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 2.5),
            egui::vec2((rect.width() - 24.0) * left, 1.5),
        );
        ui.painter().rect_filled(
            line,
            CornerRadius::same(1),
            theme::text_3().gamma_multiply(0.8),
        );
        if restore {
            self.restore_last();
        }
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let common_cancel = crate::localization::renderer(ctx).render(&Message::CommonCancel);
        let Some(mut confirmation) = self.confirm.take() else {
            return;
        };
        let mut keep = true;
        let mut execute = false;
        let busy = self.action_busy();
        let response = egui::Modal::new(egui::Id::new("confirm"))
            .backdrop_color(theme::shadow(0.45))
            .frame(overlay_frame(ctx).inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 64.0).min(420.0));
                ui.label(
                    RichText::new(self.tr(&confirmation.label))
                        .font(theme::heading(17.0))
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(self.tr(&confirmation.consequence)).color(theme::text_2()));
                ui.add_space(16.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let action = self.tr(&confirmation.action_label());
                    if widgets::button_enabled(ui, !busy, &action, Kind::Danger).clicked() {
                        execute = true;
                        keep = false;
                    }
                    let cancel = widgets::button(ui, common_cancel.as_str(), Kind::Secondary);
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
                if let Some(intent) = confirmation.intent {
                    self.enqueue_bound(intent);
                } else {
                    self.request(confirmation.request);
                }
            }
        }
    }
}

/// Frame of the highest layer: dialogs and the command palette.
pub(crate) fn overlay_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.style())
        .fill(theme::overlay())
        .stroke(Stroke::new(1.0, theme::border_strong()))
        .corner_radius(CornerRadius::same(14))
        .shadow(egui::Shadow {
            offset: [0, 24],
            blur: 64,
            spread: 0,
            color: theme::shadow(0.8),
        })
}

pub(crate) fn profile_name(
    p: &neonmix_desktop_service::ProfileInfo,
    localizer: &neonmix_i18n::Localizer,
) -> String {
    let shell_local_admin = localizer.render(&Message::ShellLocalAdmin);
    let shell_paired_device = localizer.render(&Message::ShellPairedDevice);
    if p.role == Some(Role::Admin) && p.credential == std::path::Path::new("hub/admin.json") {
        shell_local_admin.as_str().into()
    } else {
        p.name
            .clone()
            .unwrap_or_else(|| shell_paired_device.as_str().into())
    }
}
