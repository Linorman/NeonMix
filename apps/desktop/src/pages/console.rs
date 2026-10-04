//! Mixer console: one vertical strip per input, the room master pinned on
//! the right, detail drawers below, then the last minute of levels. Same
//! controls and rules as the row layout, arranged like a desk.
use super::mixer::{MASTER_W, STRIP_GAP, STRIP_W, disclosure};
use super::*;
use crate::emblem::{self, Emblem};
use crate::lanes::Lane;
use crate::widgets::{FaderSize, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

const TRAVEL: f32 = 200.0;

impl Desktop {
    pub(super) fn console(
        &mut self,
        ui: &mut egui::Ui,
        state: &Snapshot,
        lanes: &[Lane],
        focus: Option<u64>,
    ) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = STRIP_GAP;
            for lane in lanes {
                ui.push_id(lane.key, |ui| self.strip(ui, lane, focus == Some(lane.key)));
            }
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                self.master_strip(ui, state)
            });
        });
        let open: Vec<&Lane> = lanes
            .iter()
            .filter(|l| self.lane_details.contains(&l.key))
            .collect();
        for lane in open {
            widgets::surface(
                ui,
                Some(lane.source_color()),
                Margin::symmetric(16, 12),
                |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("「{}」详情", lane.name))
                                .font(theme::heading(theme::SECTION))
                                .color(theme::TEXT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::small_button(ui, true, "收起").clicked() {
                                self.lane_details.remove(&lane.key);
                            }
                        });
                    });
                    self.lane_detail_panel(ui, lane);
                },
            );
        }
        let sessions = self.airplay_sessions();
        let flag = |s: &Value, k: &str| {
            s.pointer(&format!("/mix/{k}")).and_then(Value::as_bool) == Some(true)
        };
        let muted = state.streams.values().filter(|s| s.mix.muted).count()
            + sessions.iter().filter(|s| flag(s, "muted")).count();
        let solo = state.streams.values().filter(|s| s.mix.solo).count()
            + sessions.iter().filter(|s| flag(s, "solo")).count();
        widgets::note(
            ui,
            format!(
                "活动 {} 路 · 静音 {muted} · Solo {solo} · 版本 {}",
                lanes.len(),
                state.revision
            ),
        );
    }

    fn strip(&mut self, ui: &mut egui::Ui, lane: &Lane, focus_fader: bool) {
        let ctx = ui.ctx().clone();
        let id = ui.id();
        let selected = self.selected_lane == Some(lane.key);
        let open = self.lane_details.contains(&lane.key);
        let dim = ctx.animate_bool_with_time(id.with("dim"), !lane.audible, 0.2);
        let sel = ctx.animate_bool_with_time(id.with("sel"), selected, 0.14);
        let draft = self
            .gain_drafts
            .get(&lane.key)
            .copied()
            .unwrap_or(lane.gain);
        let writable = self.writable();
        let mut picked = false;
        let mut commit = None;
        let mut toggles = (None, None);
        let mut toggle_details = false;
        let glow = if lane.solo {
            Some(theme::SOLO)
        } else {
            Some(lane.source_color())
        };
        let shown = widgets::surface(ui, glow, Margin::symmetric(12, 14), |ui| {
            let inner = STRIP_W - 24.0;
            ui.set_width(inner);
            ui.set_opacity(1.0 - 0.32 * dim);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.vertical_centered(|ui| {
                let (rect, avatar) =
                    ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
                let color = lane.source_color();
                let painter = ui.painter();
                painter.circle_filled(rect.center(), 17.0, color.gamma_multiply(0.16));
                if lane.is_airplay() {
                    icons::paint(
                        painter,
                        icons::square(rect.center(), 17.0),
                        icons::Icon::AirPlay,
                        color,
                    );
                } else {
                    let initial: String = lane
                        .name
                        .trim()
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().collect())
                        .unwrap_or_else(|| "?".into());
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        initial,
                        theme::heading(14.0),
                        color,
                    );
                }
                if avatar.clicked() {
                    picked = true;
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(&lane.name)
                            .font(theme::heading(13.5))
                            .color(theme::TEXT),
                    )
                    .truncate(),
                )
                .on_hover_text(&lane.name);
                ui.add(
                    egui::Label::new(
                        RichText::new(match lane.role {
                            Some(role) => format!("{role} · {}", lane.status),
                            None => lane.status.into(),
                        })
                        .size(theme::SMALL)
                        .color(lane.tone.color()),
                    )
                    .truncate(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_space(((inner - 10.0 - 10.0 - 30.0) / 2.0).max(0.0));
                    let (meter, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, TRAVEL), egui::Sense::hover());
                    widgets::paint_level_vertical(ui, id.with("meter"), meter, lane.peak, lane.rms);
                    if lane.muted {
                        crate::fx::hatch(
                            ui.painter(),
                            meter,
                            theme::WARNING.gamma_multiply(0.45),
                            5.0,
                        );
                    }
                    ui.add_enabled_ui(writable && lane.can_mix, |ui| {
                        let (c, response) = self.gain_control(
                            ui,
                            lane.key,
                            lane.gain,
                            &format!("「{}」音量", lane.name),
                            FaderSize::Strip(TRAVEL),
                        );
                        if focus_fader {
                            response.request_focus();
                        }
                        if response.has_focus() || response.dragged() {
                            picked = true;
                        }
                        commit = c;
                    });
                });
                let shown = animation::ease_to(&ctx, id.with("readout"), draft, 0.12);
                ui.label(
                    RichText::new(widgets::gain_text(shown))
                        .monospace()
                        .size(15.0)
                        .color(if draft > 0.0 {
                            theme::WARNING
                        } else {
                            theme::TEXT
                        }),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let widths = if lane.can_solo { 98.0 } else { 45.0 };
                    ui.add_space(((inner - widths) / 2.0).max(0.0));
                    let mute = widgets::toggle_small(
                        ui,
                        writable && lane.can_mix,
                        lane.muted,
                        "静音",
                        Tone::Warning,
                    );
                    mute.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            lane.can_mix,
                            lane.muted,
                            format!("静音「{}」", lane.name),
                        )
                    });
                    if mute.clicked() {
                        toggles.0 = Some(!lane.muted);
                    }
                    if lane.can_solo {
                        let solo =
                            widgets::toggle_small(ui, writable, lane.solo, "Solo", Tone::Solo);
                        solo.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Button,
                                true,
                                lane.solo,
                                format!("Solo「{}」", lane.name),
                            )
                        });
                        if solo.clicked() {
                            toggles.1 = Some(!lane.solo);
                        }
                    }
                });
                if disclosure(ui, open, &lane.name).clicked() {
                    toggle_details = true;
                }
            });
        });
        let rect = shown.response.rect;
        let cap = if lane.solo {
            theme::SOLO
        } else {
            lane.source_color()
        };
        let cap = animation::color(&ctx, id.with("cap"), cap);
        let painter = ui.painter();
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(18.0, 1.0),
            egui::vec2(rect.width() - 36.0, 3.0),
        );
        crate::fx::glow(painter, bar.center(), 18.0, cap.gamma_multiply(0.25));
        painter.rect_filled(bar, CornerRadius::same(2), cap);
        if sel > 0.0 {
            painter.rect_stroke(
                rect,
                CornerRadius::same(theme::RADIUS),
                Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.7 * sel)),
                egui::StrokeKind::Inside,
            );
        }
        if picked || commit.is_some() || toggles.0.is_some() || toggles.1.is_some() {
            self.selected_lane = Some(lane.key);
        }
        if toggle_details {
            if open {
                self.lane_details.remove(&lane.key);
            } else {
                self.lane_details.insert(lane.key);
            }
        }
        if let Some(gain) = commit {
            self.lane_gain(lane, gain);
        }
        if toggles.0.is_some() || toggles.1.is_some() {
            self.lane_toggle(lane.key, toggles.0, toggles.1);
        }
    }

    fn master_strip(&mut self, ui: &mut egui::Ui, state: &Snapshot) {
        let meters = self.diagnostics.as_ref().and_then(|v| v.pointer("/meters"));
        let peak = meters
            .and_then(|m| m.pointer("/output/peak"))
            .and_then(Value::as_f64);
        let rms = meters
            .and_then(|m| m.pointer("/output/rms"))
            .and_then(Value::as_f64);
        let limiter = meters
            .and_then(|m| m.get("limiter_gain"))
            .and_then(Value::as_f64);
        let (status, tone) = if !state.output.available {
            ("输出丢失", Tone::Warning)
        } else if state.output.muted {
            ("总静音中", Tone::Warning)
        } else {
            ("正在输出", Tone::Success)
        };
        let can = self.controls_room() && self.writable();
        let current = state.output.gain_db;
        let draft = self.gain_drafts.get(&0).copied().unwrap_or(current);
        let output_name = self.output_name(state);
        let id = ui.id().with("master-strip");
        let mut commit = None;
        let shown = widgets::surface(ui, Some(tone.color()), Margin::symmetric(14, 14), |ui| {
            let inner = MASTER_W - 28.0;
            ui.set_width(inner);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.vertical_centered(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                emblem::paint(
                    ui.painter(),
                    rect.center(),
                    17.0,
                    Emblem::from_id(state.hub_id),
                    if state.output.available { 1.0 } else { 0.0 },
                );
                ui.label(
                    RichText::new("房间总控")
                        .font(theme::heading(14.0))
                        .color(theme::TEXT),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(&output_name)
                            .size(theme::SMALL)
                            .color(theme::TEXT_3),
                    )
                    .truncate(),
                );
                widgets::pill(ui, status, tone);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // Scale labels | meter | limiter reduction | fader.
                    let used = 26.0 + 14.0 + 4.0 + 6.0 + 14.0 + 30.0;
                    ui.add_space(((inner - used) / 2.0).max(0.0));
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(26.0 + 14.0, TRAVEL), egui::Sense::hover());
                    let meter = egui::Rect::from_min_max(
                        egui::pos2(rect.right() - 14.0, rect.top()),
                        rect.max,
                    );
                    widgets::paint_level_vertical(ui, id.with("meter"), meter, peak, rms);
                    widgets::paint_scale_vertical(ui.painter(), meter);
                    let (gr, gr_response) =
                        ui.allocate_exact_size(egui::vec2(6.0, TRAVEL), egui::Sense::hover());
                    widgets::paint_reduction(ui.painter(), gr, limiter);
                    gr_response.on_hover_text("限幅器增益衰减（自上而下，满刻度 12 dB）");
                    ui.add_space(10.0);
                    ui.add_enabled_ui(can, |ui| {
                        let (c, _) =
                            self.gain_control(ui, 0, current, "总音量", FaderSize::Strip(TRAVEL));
                        commit = c;
                    });
                });
                let shown = animation::ease_to(ui.ctx(), id.with("readout"), draft, 0.12);
                ui.label(
                    RichText::new(format!("{} dB", widgets::gain_text(shown)))
                        .monospace()
                        .size(24.0)
                        .color(if draft > 0.0 {
                            theme::WARNING
                        } else {
                            theme::TEXT
                        }),
                );
                ui.add_enabled_ui(can, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(((inner - 132.0) / 2.0).max(0.0));
                        if let Some(db) = widgets::segmented(
                            ui,
                            true,
                            &[("−12", -12.0), ("−6", -6.0), ("0", 0.0)],
                            current,
                        ) {
                            commit = Some(db);
                        }
                    });
                });
                if self.controls_room() {
                    let muted = state.output.muted;
                    if widgets::toggle_small(
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
                } else {
                    widgets::note(ui, "总控需要控制者或管理员");
                }
                if let Some(g) = limiter.filter(|g| *g < 0.999 && *g > 0.0) {
                    ui.label(
                        RichText::new(format!("限幅 {} dB", widgets::db_text(g)))
                            .size(theme::SMALL)
                            .color(theme::WARNING),
                    );
                }
            });
        });
        let rect = shown.response.rect;
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(18.0, 1.0),
            egui::vec2(rect.width() - 36.0, 3.0),
        );
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), theme::SRC_HUB);
        if let Some(gain) = commit {
            self.master_gain(current, gain);
        }
    }

    /// Last minute of each lane's RMS, recorded from real polls.
    pub(super) fn level_ribbon(&mut self, ui: &mut egui::Ui, lanes: &[Lane]) {
        if lanes.is_empty() {
            return;
        }
        widgets::surface(ui, None, Margin::symmetric(16, 12), |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("最近 60 秒")
                        .font(theme::heading(theme::SECTION))
                        .color(theme::TEXT),
                );
                widgets::note(ui, "每路 RMS · 斜线为静音或因 Solo 静音 · 仅保存在本窗口");
            });
            let window = self.level_history.window();
            for lane in lanes {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(132.0, 22.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&lane.name)
                                        .size(theme::SMALL)
                                        .color(theme::TEXT_2),
                                )
                                .truncate(),
                            );
                        },
                    );
                    let rect = crate::history::row_rect(ui, 22.0);
                    match self.level_history.get(&lane.key).filter(|r| r.len() >= 2) {
                        Some(samples) => crate::history::paint_levels(
                            ui.painter(),
                            rect,
                            samples,
                            window,
                            lane.source_color(),
                        ),
                        None => {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(4),
                                theme::METER_TRACK,
                            );
                            ui.painter().text(
                                rect.left_center() + egui::vec2(10.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                "正在记录…",
                                egui::FontId::proportional(theme::SMALL),
                                theme::TEXT_3,
                            );
                        }
                    }
                });
            }
        });
    }
}
