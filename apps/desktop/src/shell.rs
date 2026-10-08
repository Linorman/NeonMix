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
        localization::install(ctx, self.localization.renderer.clone());
        self.process();
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
                    .inner_margin(Margin::symmetric(20, 2)),
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
                // Ambient light that names the room state; it only moves
                // when that state changes.
                let ambient = if self.error {
                    theme::DANGER.gamma_multiply(0.05)
                } else if self.status.as_ref().is_some_and(|s| s.hub.running)
                    || self.snapshot.is_some()
                {
                    theme::ACCENT.gamma_multiply(0.045)
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
                        bottom: 10,
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
                                ui.spacing_mut().item_spacing.y = 14.0;
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
        if !ctx.wants_keyboard_input()
            && self.undo_available()
            && ctx.input_mut(|i| i.consume_key(command, egui::Key::Z))
        {
            self.restore_last();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, short: bool) {
        let shell_quit = crate::localization::renderer(ui.ctx()).render(&Message::ShellQuit);
        let shell_hide = crate::localization::renderer(ui.ctx()).render(&Message::ShellHide);
        let shell_stop_sending =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellStopSending);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
            // The mark is the room's output: bars follow the master RMS
            // through meter ballistics and rest in the logo shape in silence.
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
                ui.painter().line_segment(
                    [
                        egui::pos2(x, rect.center().y - h / 2.0),
                        egui::pos2(x, rect.center().y + h / 2.0),
                    ],
                    Stroke::new(2.5, theme::ACCENT),
                );
            }
            if level.is_some_and(|l| l > 0.05) {
                crate::fx::glow(
                    ui.painter(),
                    rect.center(),
                    14.0,
                    theme::ACCENT.gamma_multiply(0.15),
                );
            }
            ui.label(
                RichText::new("NeonMix")
                    .font(theme::heading(17.0))
                    .color(theme::TEXT),
            );
        });
        ui.add_space(if short { 8.0 } else { 18.0 });

        let item_height = if short { 30.0 } else { 36.0 };
        let item_width = ui.available_width();
        let mut selected_top = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        let first_top = ui.cursor().top();
        for (i, (page, title)) in Page::ALL.into_iter().enumerate() {
            let title = self.tr(&title);
            let selected = self.page == page;
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(item_width, item_height), egui::Sense::click());
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
                &title,
                if selected {
                    theme::heading(theme::BODY)
                } else {
                    egui::FontId::proportional(theme::BODY)
                },
                animation::lerp_color(theme::TEXT_2, theme::TEXT, active.max(hover * 0.6)),
            );
            if !short && item_width > 180.0 && i < 6 {
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
            ui.add_space(if short { 2.0 } else { 6.0 });
        }
        // Indicator slides between items instead of jumping.
        if let Some(top) = selected_top {
            let y = animation::ease_to(
                ui.ctx(),
                ui.id().with("nav-indicator"),
                top - first_top,
                0.22,
            );
            let left = ui.min_rect().left();
            let bar = egui::Rect::from_min_size(
                egui::pos2(left, first_top + y + 9.0),
                egui::vec2(3.0, item_height - 18.0),
            );
            crate::fx::glow(
                ui.painter(),
                bar.center(),
                10.0,
                theme::ACCENT.gamma_multiply(0.35),
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
            // Hide and quit share one row so the destructive pair never
            // outweighs navigation; narrow sidebars use compact buttons.
            let compact = width < 180.0;
            let half = (width - 6.0) / 2.0;
            let (hide, quit) = ui
                .allocate_ui_with_layout(
                    egui::vec2(width, theme::CONTROL_HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let cell = |ui: &mut egui::Ui, text: &str, kind: Kind| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(half, theme::CONTROL_HEIGHT),
                                Layout::top_down_justified(Align::Min),
                                |ui| {
                                    if compact {
                                        widgets::button_compact(ui, text, kind)
                                    } else {
                                        widgets::button(ui, text, kind)
                                    }
                                },
                            )
                            .inner
                        };
                        (
                            cell(ui, &self.tr(&Message::ShellHideShort), Kind::Secondary),
                            cell(ui, &self.tr(&Message::ShellQuitShort), Kind::Quiet),
                        )
                    },
                )
                .inner;
            hide.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &shell_hide)
            });
            quit.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &shell_quit)
            });
            let hide = hide.on_hover_text(&shell_hide);
            let quit = quit.on_hover_text(&shell_quit);
            if hide.clicked() {
                self.hide_window(ui.ctx());
            }
            if quit.clicked() {
                self.confirmation(
                    Message::ShellQuit,
                    Message::ShellQuitConsequence,
                    Request::Shutdown,
                    quit.id,
                );
            }
            if self.status.as_ref().is_some_and(|s| s.sender.running)
                && full(
                    ui,
                    shell_stop_sending.as_str(),
                    Kind::Quiet,
                    self.pending_stop,
                )
                .clicked()
            {
                self.stop_sender();
            }
            ui.add_space(6.0);
            self.identity(ui, width, short);
        });
    }

    fn hide_window(&self, ctx: &egui::Context) {
        if self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    /// Identity switcher at the foot of the sidebar: who this window acts as.
    fn identity(&mut self, ui: &mut egui::Ui, width: f32, short: bool) {
        let shell_role_admin =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoleAdmin);
        let shell_role_controller =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoleController);
        let shell_role_member =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoleMember);
        let shell_unverified =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellUnverified);
        let shell_unpaired =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellUnpaired);
        let shell_identity =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellIdentity);
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
            (Some(Role::Admin), _) => (shell_role_admin.as_str(), Tone::Accent),
            (Some(Role::Controller), _) => (shell_role_controller.as_str(), Tone::Accent),
            (Some(Role::Member), _) => (shell_role_member.as_str(), Tone::Neutral),
            (None, Some(Role::Admin)) => (shell_role_admin.as_str(), Tone::Neutral),
            (None, Some(Role::Controller)) => (shell_role_controller.as_str(), Tone::Neutral),
            (None, Some(Role::Member)) => (shell_role_member.as_str(), Tone::Neutral),
            (None, None) => (shell_unverified.as_str(), Tone::Warning),
        };
        let selected = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .map_or(shell_unpaired.as_str().to_owned(), |p| {
                profile_name(p, &self.localization.renderer)
            });
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
                        let label = widgets::caption(ui, shell_identity.as_str());
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
                    .truncate()
                    .selected_text(RichText::new(selected).size(theme::SMALL + 1.0))
                    .show_ui(ui, |ui| {
                        if let Some(status) = &self.status {
                            for p in status.profiles.iter().filter(|p| !p.pending) {
                                widgets::select_value(
                                    ui,
                                    &mut self.credential,
                                    p.credential.clone(),
                                    profile_name(p, &self.localization.renderer),
                                );
                            }
                        }
                    })
                    .response
                    .on_hover_text(self.tr(&Message::ShellCurrentIdentity { role: role.into() }));
                let response = match label {
                    Some(label) => response.labelled_by(label.id),
                    None => response,
                };
                widgets::label_combo(&response, shell_identity.as_str());
            },
        );
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

    /// Room at a glance on every page: title, room state, a live master
    /// meter (except on the mixer, which shows it large) and ⌘K.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let shell_no_room = crate::localization::renderer(ui.ctx()).render(&Message::ShellNoRoom);
        let shell_sharing = crate::localization::renderer(ui.ctx()).render(&Message::ShellSharing);
        let shell_not_sharing =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellNotSharing);
        let shell_stale = crate::localization::renderer(ui.ctx()).render(&Message::ShellStale);
        let shell_room_connected =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoomConnected);
        let shell_room_disconnected =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoomDisconnected);
        let shell_room_tooltip =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRoomTooltip);
        let title = self.tr(&self.page.title());
        let status = self.status.as_ref();
        let room = self
            .remote_room
            .clone()
            .or_else(|| {
                status
                    .and_then(|s| s.hub_settings.as_ref())
                    .map(|h| h.name.clone())
            })
            .unwrap_or_else(|| shell_no_room.as_str().into());
        let local_admin = self.credential == std::path::Path::new("hub/admin.json");
        let (room_state, room_tone) = if status.is_some_and(|s| s.hub.running) {
            (shell_sharing.as_str(), Tone::Success)
        } else if local_admin && status.is_some_and(|s| s.hub_settings.is_some()) {
            (shell_not_sharing.as_str(), Tone::Neutral)
        } else if self.snapshot.is_some() && !self.writable() {
            (shell_stale.as_str(), Tone::Warning)
        } else if self.snapshot.is_some() {
            (shell_room_connected.as_str(), Tone::Success)
        } else {
            (shell_room_disconnected.as_str(), Tone::Neutral)
        };
        let streams =
            self.snapshot.as_ref().map_or(0, |s| s.streams.len()) + self.airplay_sessions().len();
        let known_room =
            self.remote_room.is_some() || status.is_some_and(|s| s.hub_settings.is_some());
        let chip = if !known_room {
            room
        } else if streams > 0 {
            self.tr(&Message::ShellRoomSummary {
                room: room.clone(),
                state: room_state.into(),
                count: streams as u64,
            })
        } else {
            format!("{room} · {room_state}")
        };
        let hub_id = self.snapshot.as_ref().map(|s| s.hub_id);
        let left = |ui: &mut egui::Ui| {
            ui.label(
                RichText::new(&title)
                    .font(theme::heading(theme::TITLE))
                    .color(theme::TEXT),
            );
            ui.add_space(6.0);
            if let Some(id) = hub_id {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                crate::emblem::paint(
                    ui.painter(),
                    rect.center(),
                    9.0,
                    crate::emblem::Emblem::from_id(id),
                    1.0,
                );
            }
            // The chip shares this row with the title and emblem. Bounding
            // it by the whole row lets a long name push actions off-window.
            let width = ui.available_width();
            let _ = widgets::pill_sized(ui, &chip, room_tone, width)
                .on_hover_text(format!("{chip}\n{shell_room_tooltip}"));
        };
        let actions_width = self.top_bar_actions_width(ui);
        let title_width = ui.fonts(|f| {
            f.layout_no_wrap(title.clone(), theme::heading(theme::TITLE), theme::TEXT)
                .size()
                .x
        });
        // Reserve actions first, using the actual localized labels instead
        // of a fixed breakpoint that only fits one language.
        if ui.available_width() >= actions_width + title_width + 190.0 {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(actions_width, theme::CONTROL_HEIGHT),
                        Layout::right_to_left(Align::Center),
                        |ui| self.top_bar_actions(ui),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        left,
                    );
                })
            });
        } else {
            ui.horizontal(left);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                self.top_bar_actions(ui);
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

    fn top_bar_actions_width(&self, ui: &egui::Ui) -> f32 {
        let button_width = |text: String| {
            ui.fonts(|f| {
                f.layout_no_wrap(text, egui::FontId::proportional(theme::BODY), theme::TEXT)
                    .size()
                    .x
            }) + 2.0 * ui.spacing().button_padding.x
        };
        let mut width = button_width(self.tr(&Message::ShellQuickActions {
            shortcut: widgets::command_hint("K"),
        }))
        .max(theme::CONTROL_HEIGHT);
        if self.page != Page::Mixer && self.snapshot.is_some() {
            width += 10.0 + 76.0 + 96.0 + 4.0 * ui.spacing().item_spacing.x;
            if self.controls_room() {
                width += button_width(self.tr(&Message::ShellMasterMute))
                    .max(button_width(self.tr(&Message::ShellMasterUnmute)))
                    .max(theme::CONTROL_HEIGHT)
                    + ui.spacing().item_spacing.x;
            }
        }
        width.ceil()
    }

    /// Right-to-left: ⌘K first (rightmost), then the master glance.
    fn top_bar_actions(&mut self, ui: &mut egui::Ui) {
        let shell_quick_tooltip =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellQuickTooltip);
        let shell_master_unmute =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellMasterUnmute);
        let shell_master_mute =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellMasterMute);
        let shell_output_tooltip =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellOutputTooltip);
        let palette = widgets::button(
            ui,
            &self.tr(&Message::ShellQuickActions {
                shortcut: widgets::command_hint("K"),
            }),
            Kind::Secondary,
        )
        .on_hover_text(shell_quick_tooltip.as_str());
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
                    shell_master_unmute.as_str()
                } else {
                    shell_master_mute.as_str()
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
        let meter = self.meters_current().and_then(|v| v.get("output"));
        let (peak, rms) = (
            meter.and_then(|v| v["peak"].as_f64()),
            meter.and_then(|v| v["rms"].as_f64()),
        );
        // In a wrapped row the meter may use the remaining width; forcing
        // 96 px would create a mostly empty extra header row in English.
        let meter_width = if ui.layout().main_wrap() {
            let remaining = ui.available_size_before_wrap().x;
            if remaining >= 24.0 {
                remaining.min(96.0)
            } else {
                96.0
            }
        } else {
            96.0
        };
        let level = widgets::level(ui, "topbar-master", egui::vec2(meter_width, 6.0), peak, rms)
            .interact(egui::Sense::click())
            .on_hover_text(shell_output_tooltip.as_str());
        if level.clicked() {
            self.navigate(Page::Mixer);
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
            animation::lerp_color(theme::SIDEBAR, theme::DANGER, 0.08)
        } else {
            theme::SIDEBAR
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
        let shell_restore_tooltip =
            crate::localization::renderer(ui.ctx()).render(&Message::ShellRestoreTooltip);
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
            if self.undo_available() {
                ui.ctx().request_repaint_after(Duration::from_millis(500));
                let restore = widgets::small_button(
                    ui,
                    true,
                    &self.tr(&Message::ShellRestoreButton {
                        shortcut: widgets::command_hint("Z"),
                    }),
                )
                .on_hover_text(shell_restore_tooltip.as_str());
                if let Some(undo) = &self.undo {
                    // The 8 s restore window, draining under the button.
                    let left = 1.0 - undo.at.elapsed().as_secs_f32() / 8.0;
                    let r = restore.rect;
                    let line = egui::Rect::from_min_size(
                        egui::pos2(r.left() + 4.0, r.bottom() + 1.0),
                        egui::vec2((r.width() - 8.0) * left.clamp(0.0, 1.0), 2.0),
                    );
                    ui.painter().rect_filled(
                        line,
                        CornerRadius::same(1),
                        theme::ACCENT.gamma_multiply(0.8),
                    );
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                if restore.clicked() {
                    self.restore_last();
                }
                if let Some(undo) = &self.undo {
                    let label = self.tr(&Message::ShellAdjusted {
                        change: self.tr(&undo.label),
                    });
                    let width = ((ui.available_width() - ui.spacing().item_spacing.x) * 0.5)
                        .clamp(1.0, 280.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 22.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&label)
                                        .size(theme::SMALL)
                                        .color(theme::TEXT_2),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&label);
                        },
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
                                Stroke::new(1.0, theme::ACCENT),
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
                        theme::DANGER
                    } else {
                        theme::TEXT_2
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

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let common_cancel = crate::localization::renderer(ctx).render(&Message::CommonCancel);
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
                    RichText::new(self.tr(&confirmation.label))
                        .font(theme::heading(17.0))
                        .color(theme::TEXT),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(self.tr(&confirmation.consequence)).color(theme::TEXT_2));
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
