//! Mixer: the live console. Master first (largest numbers on the page), then
//! one row per input with level, fader and latching Mute/Solo; details fold
//! out per row. Rows that are not audible recede.
use super::*;
use crate::lanes::Lane;
use crate::widgets::{FaderSize, Kind, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

impl Desktop {
    pub(crate) fn mixer_page(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.snapshot.clone() else {
            self.mixer_empty(ui);
            return;
        };
        let lanes = self.lanes(&state);
        let console = console_fits(ui.available_width(), lanes.len());
        if !console {
            self.master_hero(ui, &state);
        }
        let wide = ui.available_width() >= 620.0;
        widgets::section(ui, "输入通道", |ui| {
            widgets::pill(ui, &format!("{} 路", lanes.len()), Tone::Neutral);
            if wide && !lanes.is_empty() {
                // Keys follow the fader direction of the current layout.
                let (pick, gain) = if console {
                    ("← →", "↑ ↓")
                } else {
                    ("↑ ↓", "← →")
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    widgets::note(ui, "Solo");
                    widgets::kbd(ui, "S");
                    widgets::note(ui, "静音");
                    widgets::kbd(ui, "M");
                    widgets::note(ui, "音量");
                    widgets::kbd(ui, gain);
                    widgets::note(ui, "选择通道");
                    widgets::kbd(ui, pick);
                });
            }
        });
        if lanes.is_empty() {
            widgets::surface(ui, None, Margin::same(16), |ui| {
                ui.set_min_width(ui.available_width());
                widgets::empty(
                    ui,
                    "暂无输入",
                    "Sender 配对并开始发送，或 AirPlay 来源连接后，通道会显示在这里。",
                );
            });
        } else if console {
            let focus = self.mixer_keys(ui, &lanes, true);
            self.console(ui, &state, &lanes, focus);
        } else {
            let focus = self.mixer_keys(ui, &lanes, false);
            ui.spacing_mut().item_spacing.y = 8.0;
            for lane in &lanes {
                ui.push_id(lane.key, |ui| {
                    self.lane_row(ui, lane, focus == Some(lane.key), wide)
                });
            }
            ui.spacing_mut().item_spacing.y = 14.0;
        }
        self.level_ribbon(ui, &lanes);
        ui.label(
            RichText::new("电平与推子说明")
                .size(theme::SMALL)
                .color(theme::TEXT_3)
                .underline(),
        )
        .on_hover_text(
            "电平为最近 50 ms 双声道窗口：每路在 Mute/Solo 与增益之后、总控之前测量，总控在限幅之后。实心为 RMS，浅色为峰值，横线/竖线为峰值保持。推子按住拖动（Shift 微调），双击回到 0 dB；聚焦后或按住 Option 可用滚轮调节。",
        );
    }

    fn mixer_empty(&mut self, ui: &mut egui::Ui) {
        widgets::surface(ui, None, Margin::same(24), |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(rect.center(), 20.0, theme::ACCENT.gamma_multiply(0.12));
                    icons::paint(
                        ui.painter(),
                        icons::square(rect.center(), 20.0),
                        icons::Icon::Mixer,
                        theme::ACCENT,
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("尚未连接房间")
                            .font(theme::heading(17.0))
                            .color(theme::TEXT),
                    );
                    widgets::note(
                        ui,
                        "在 Hub 设置中创建并开始共享房间，或在 Sender 页面配对一个房间，然后在这里调音。",
                    );
                    ui.add_space(6.0);
                });
            ui.horizontal(|ui| {
                let pad = ((ui.available_width() - 260.0) / 2.0).max(0.0);
                ui.add_space(pad);
                if widgets::button(ui, "前往 Hub 设置", Kind::Primary).clicked() {
                    self.navigate(Page::Hub);
                }
                if widgets::button(ui, "前往 Sender", Kind::Secondary).clicked() {
                    self.navigate(Page::Sender);
                }
            });
        });
        if self.airplay.is_some() {
            self.airplay_panel(ui);
        }
    }

    /// ↑/↓ choose a lane (and move focus to its fader, where ←/→ adjust),
    /// M/S latch Mute/Solo. Ignored while typing or when a dialog is open.
    /// Lane selection keys: ↑/↓ for rows, ←/→ for console strips (where
    /// ↑/↓ belong to the focused vertical fader).
    pub(crate) fn mixer_keys(
        &mut self,
        ui: &egui::Ui,
        lanes: &[Lane],
        console: bool,
    ) -> Option<u64> {
        let ctx = ui.ctx().clone();
        let mut focus_to = None;
        if ctx.wants_keyboard_input() || self.confirm.is_some() || self.palette.is_some() {
            return None;
        }
        let index = self
            .selected_lane
            .and_then(|k| lanes.iter().position(|l| l.key == k));
        let none = egui::Modifiers::NONE;
        let (next_key, prev_key) = if console {
            (egui::Key::ArrowRight, egui::Key::ArrowLeft)
        } else {
            (egui::Key::ArrowDown, egui::Key::ArrowUp)
        };
        let (down, up, m, s) = ctx.input_mut(|i| {
            (
                i.consume_key(none, next_key),
                i.consume_key(none, prev_key),
                i.consume_key(none, egui::Key::M),
                i.consume_key(none, egui::Key::S),
            )
        });
        if down || up {
            let next = match (index, down) {
                (None, _) => 0,
                (Some(i), true) => (i + 1).min(lanes.len() - 1),
                (Some(i), false) => i.saturating_sub(1),
            };
            self.selected_lane = Some(lanes[next].key);
            focus_to = Some(lanes[next].key);
        }
        if let Some(lane) = self
            .selected_lane
            .and_then(|k| lanes.iter().find(|l| l.key == k))
        {
            if m {
                self.lane_toggle(lane.key, Some(!lane.muted), None);
            }
            if s {
                self.lane_toggle(lane.key, None, Some(!lane.solo));
            }
        }
        focus_to
    }

    fn master_hero(&mut self, ui: &mut egui::Ui, state: &Snapshot) {
        let output_name = self
            .devices
            .iter()
            .find(|d| d.id == state.output.id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| state.output.id.clone());
        let limiter = self
            .diagnostics
            .as_ref()
            .and_then(|v| v.pointer("/meters/limiter_gain"))
            .and_then(Value::as_f64);
        let (pill, tone) = if !state.output.available {
            ("输出丢失 · 等待设备恢复", Tone::Warning)
        } else if state.output.muted {
            ("总静音中", Tone::Warning)
        } else {
            ("正在输出", Tone::Success)
        };
        let meter = self
            .diagnostics
            .as_ref()
            .and_then(|v| v.pointer("/meters/output"));
        let (peak, rms) = (
            meter.and_then(|v| v["peak"].as_f64()),
            meter.and_then(|v| v["rms"].as_f64()),
        );
        let can = self.controls_room();
        widgets::card_ex(
            ui,
            "房间总控",
            Some(&output_name),
            Some(tone.color()),
            |ui| {
                widgets::pill(ui, pill, tone);
                if let Some(g) = limiter.filter(|g| *g < 0.999) {
                    widgets::pill(
                        ui,
                        &format!("限幅 {} dB", widgets::db_text(g)),
                        Tone::Warning,
                    )
                    .on_hover_text("限幅器正在压低输出峰值");
                }
            },
            |ui| {
                widgets::meter_sized(ui, "master", peak, rms, 14.0);
                let wide = ui.available_width() >= 560.0;
                let current = state.output.gain_db;
                let shown = self.gain_drafts.get(&0).copied().unwrap_or(current);
                let readout = |ui: &mut egui::Ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let caption = widgets::caption(ui, "总音量");
                        ui.label(
                            RichText::new(format!("{} dB", widgets::gain_text(shown)))
                                .monospace()
                                .size(theme::DISPLAY)
                                .color(if shown > 0.0 {
                                    theme::WARNING
                                } else {
                                    theme::TEXT
                                }),
                        );
                        caption
                    })
                    .inner
                };
                let mut committed = None;
                ui.add_enabled_ui(self.writable() && can, |ui| {
                    if wide {
                        ui.horizontal(|ui| {
                            let caption = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(150.0, 52.0),
                                    Layout::top_down(Align::Min),
                                    readout,
                                )
                                .inner;
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 52.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.add_space(12.0);
                                    let (commit, response) = self.gain_control(
                                        ui,
                                        0,
                                        current,
                                        "总音量",
                                        FaderSize::Hero,
                                    );
                                    response.labelled_by(caption.id);
                                    committed = commit;
                                },
                            );
                        });
                    } else {
                        let caption = readout(ui);
                        let (commit, response) =
                            self.gain_control(ui, 0, current, "总音量", FaderSize::Hero);
                        response.labelled_by(caption.id);
                        committed = commit;
                    }
                    ui.horizontal_wrapped(|ui| {
                        let muted = state.output.muted;
                        if widgets::toggle(
                            ui,
                            true,
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
                        ui.add_space(6.0);
                        if let Some(db) = widgets::segmented(
                            ui,
                            true,
                            &[("−12 dB", -12.0), ("−6 dB", -6.0), ("0 dB", 0.0)],
                            current,
                        ) {
                            committed = Some(db);
                        }
                        if wide {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let sessions = self.airplay_sessions();
                                let muted = state.streams.values().filter(|s| s.mix.muted).count()
                                    + sessions
                                        .iter()
                                        .filter(|s| {
                                            s.pointer("/mix/muted").and_then(Value::as_bool)
                                                == Some(true)
                                        })
                                        .count();
                                let solo = state.streams.values().filter(|s| s.mix.solo).count()
                                    + sessions
                                        .iter()
                                        .filter(|s| {
                                            s.pointer("/mix/solo").and_then(Value::as_bool)
                                                == Some(true)
                                        })
                                        .count();
                                widgets::note(
                                    ui,
                                    format!(
                                        "活动 {} 路 · 静音 {muted} · Solo {solo} · 版本 {}",
                                        state.streams.len() + sessions.len(),
                                        state.revision
                                    ),
                                );
                            });
                        }
                    });
                });
                if let Some(gain) = committed {
                    self.master_gain(current, gain);
                }
                if !can {
                    widgets::note(ui, "总控需要房间控制者或管理员身份；当前只能查看。");
                }
            },
        );
    }

    fn lane_row(&mut self, ui: &mut egui::Ui, lane: &Lane, focus_fader: bool, wide: bool) {
        let selected = self.selected_lane == Some(lane.key);
        let open = self.lane_details.contains(&lane.key);
        let ctx = ui.ctx().clone();
        let id = ui.id();
        let sel = ctx.animate_bool_with_time(id.with("sel"), selected, 0.14);
        let dim = ctx.animate_bool_with_time(id.with("dim"), !lane.audible, 0.2);
        let frame = egui::Frame::new()
            .fill(animation::lerp_color(theme::SURFACE, theme::RAISED, sel))
            .stroke(Stroke::new(
                1.0,
                animation::lerp_color(theme::BORDER, theme::ACCENT.gamma_multiply(0.55), sel),
            ))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin {
                left: 16,
                right: 12,
                top: 10,
                bottom: 10,
            })
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                // Not audible: recede, but stay fully operable.
                ui.set_opacity(1.0 - 0.32 * dim);
                let mut picked = false;
                let mut gain_commit = None;
                let mut toggles = (None, None);
                let mut toggle_details = false;
                let draft = self
                    .gain_drafts
                    .get(&lane.key)
                    .copied()
                    .unwrap_or(lane.gain);
                let name_block = |ui: &mut egui::Ui, width: f32| {
                    let r = ui
                        .allocate_ui_with_layout(
                            egui::vec2(width, 40.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                let color = if lane.airplay_target.is_some() {
                                    theme::ACCENT
                                } else {
                                    lane.tone.color()
                                };
                                if lane.airplay_target.is_some() {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(30.0, 30.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        15.0,
                                        color.gamma_multiply(0.18),
                                    );
                                    icons::paint(
                                        ui.painter(),
                                        icons::square(rect.center(), 16.0),
                                        icons::Icon::AirPlay,
                                        color,
                                    );
                                } else {
                                    widgets::avatar(ui, &lane.name, color, 30.0);
                                }
                                ui.vertical(|ui| {
                                    // Long names truncate instead of running
                                    // into the level lane.
                                    ui.set_max_width(width - 42.0);
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&lane.name)
                                                .font(theme::heading(14.5))
                                                .color(theme::TEXT),
                                        )
                                        .truncate(),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        if let Some(role) = lane.role {
                                            ui.label(
                                                RichText::new(format!("{role} ·"))
                                                    .size(theme::SMALL)
                                                    .color(theme::TEXT_3),
                                            );
                                        }
                                        ui.label(
                                            RichText::new(lane.status)
                                                .size(theme::SMALL)
                                                .color(lane.tone.color()),
                                        );
                                    });
                                });
                            },
                        )
                        .response;
                    ui.interact(r.rect, ui.id().with("pick"), egui::Sense::click())
                };
                let readout = |ui: &mut egui::Ui| {
                    ui.add_sized(
                        egui::vec2(66.0, 24.0),
                        egui::Label::new(
                            RichText::new(widgets::gain_text(draft))
                                .monospace()
                                .size(15.0)
                                .color(if draft > 0.0 {
                                    theme::WARNING
                                } else {
                                    theme::TEXT
                                }),
                        ),
                    );
                };
                // Mute / Solo / details; returns what the user pressed.
                let controls = |ui: &mut egui::Ui, details: bool| {
                    let mut out = (None, None, false);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mute = widgets::toggle(ui, lane.can_mix, lane.muted, "静音", Tone::Warning);
                    mute.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            lane.can_mix,
                            lane.muted,
                            format!("静音「{}」", lane.name),
                        )
                    });
                    if mute.clicked() {
                        out.0 = Some(!lane.muted);
                    }
                    if lane.can_solo {
                        let solo = widgets::toggle(ui, true, lane.solo, "Solo", Tone::Solo);
                        solo.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Button,
                                true,
                                lane.solo,
                                format!("Solo「{}」", lane.name),
                            )
                        });
                        if solo.clicked() {
                            out.1 = Some(!lane.solo);
                        }
                    }
                    if details && disclosure(ui, open, &lane.name).clicked() {
                        out.2 = true;
                    }
                    out
                };
                ui.add_enabled_ui(self.writable(), |ui| {
                    if wide {
                        ui.horizontal(|ui| {
                            if name_block(ui, 196.0).clicked() {
                                picked = true;
                            }
                            let right = 66.0 + 6.0 + 62.0 + 6.0 + 62.0 + 6.0 + 30.0 + 16.0;
                            let mid = (ui.available_width() - right).max(120.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(mid, 40.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.spacing_mut().item_spacing.y = 4.0;
                                    ui.add_space(4.0);
                                    widgets::level(
                                        ui,
                                        lane.key,
                                        egui::vec2(mid, 6.0),
                                        lane.peak,
                                        lane.rms,
                                    );
                                    ui.add_enabled_ui(lane.can_mix, |ui| {
                                        let (commit, response) = self.gain_control(
                                            ui,
                                            lane.key,
                                            lane.gain,
                                            &format!("「{}」音量", lane.name),
                                            FaderSize::Row,
                                        );
                                        if focus_fader {
                                            response.request_focus();
                                        }
                                        if response.has_focus() || response.dragged() {
                                            picked = true;
                                        }
                                        gain_commit = commit;
                                    });
                                },
                            );
                            readout(ui);
                            let (m, s, d) = controls(ui, true);
                            toggles = (m, s);
                            toggle_details = d;
                        });
                    } else {
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - 36.0;
                            if name_block(ui, w).clicked() {
                                picked = true;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if disclosure(ui, open, &lane.name).clicked() {
                                    toggle_details = true;
                                }
                            });
                        });
                        widgets::level(
                            ui,
                            lane.key,
                            egui::vec2(ui.available_width(), 6.0),
                            lane.peak,
                            lane.rms,
                        );
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - 74.0;
                            ui.allocate_ui_with_layout(
                                egui::vec2(w, 24.0),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.add_enabled_ui(lane.can_mix, |ui| {
                                        let (commit, response) = self.gain_control(
                                            ui,
                                            lane.key,
                                            lane.gain,
                                            &format!("「{}」音量", lane.name),
                                            FaderSize::Row,
                                        );
                                        if focus_fader {
                                            response.request_focus();
                                        }
                                        if response.has_focus() || response.dragged() {
                                            picked = true;
                                        }
                                        gain_commit = commit;
                                    });
                                },
                            );
                            readout(ui);
                        });
                        ui.horizontal(|ui| {
                            let (m, s, _) = controls(ui, false);
                            toggles = (m, s);
                        });
                    }
                });
                if open {
                    ui.add_space(8.0);
                    self.lane_detail_panel(ui, lane);
                }
                if picked || gain_commit.is_some() || toggles.0.is_some() || toggles.1.is_some() {
                    self.selected_lane = Some(lane.key);
                }
                if toggle_details {
                    if open {
                        self.lane_details.remove(&lane.key);
                    } else {
                        self.lane_details.insert(lane.key);
                    }
                }
                if let Some(gain) = gain_commit {
                    self.lane_gain(lane, gain);
                }
                if toggles.0.is_some() || toggles.1.is_some() {
                    self.lane_toggle(lane.key, toggles.0, toggles.1);
                }
            });
        let rect = frame.response.rect;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 12.0),
            egui::vec2(3.0, (rect.height() - 24.0).max(8.0)),
        );
        let color = animation::color(&ctx, id.with("rail"), lane.tone.color());
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }

    pub(crate) fn lane_detail_panel(&mut self, ui: &mut egui::Ui, lane: &Lane) {
        use neonmix_airplay_adapter::control::AirplayActionV2 as AirplayAction;
        widgets::inset(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                widgets::metric(
                    ui,
                    "Mixer 队列估计",
                    &lane
                        .queue_ms
                        .map_or("未取得".into(), |ms| format!("{ms:.1} ms")),
                    None,
                );
                widgets::metric(
                    ui,
                    "时钟漂移",
                    &lane
                        .drift_ppm
                        .map_or("未取得".into(), |p| format!("{p:+.1} ppm")),
                    None,
                );
                widgets::metric(ui, "媒体网络", lane.network.0, lane.network.1);
                if lane.airplay_target.is_none() {
                    widgets::metric(ui, "通道", &format!("…{}", short_id(lane.key)), None);
                }
            });
            if let Some((source_id, session_id)) = &lane.airplay_target {
                let mode = self
                    .airplay_sessions()
                    .into_iter()
                    .find(|s| s["session_id"].as_u64() == Some(*session_id))
                    .map_or("低延迟", |s| {
                        if s["playback_mode"].as_str() == Some("synchronized") {
                            "音画同步"
                        } else {
                            "低延迟"
                        }
                    });
                widgets::note(ui, format!("播放方式：{mode}（断开后在设备管理中切换）"));
                if self.admin() {
                    ui.horizontal_wrapped(|ui| {
                        if widgets::button(ui, "断开 AirPlay 来源", Kind::Secondary).clicked() {
                            self.airplay_operation(AirplayAction::DisconnectSource {
                                source_id: source_id.clone(),
                                session_id: *session_id,
                            });
                        }
                        let revoke = widgets::button(ui, "撤销 AirPlay 配对", Kind::Quiet);
                        if revoke.clicked()
                            && let Some(request) =
                                self.airplay_request(AirplayAction::RevokeSource {
                                    source_id: source_id.clone(),
                                    session_id: Some(*session_id),
                                })
                        {
                            self.confirmation(
                                format!("撤销 {} 的配对", lane.name),
                                "结束该设备音频，并撤销它在所有入口的配对。".into(),
                                request,
                                revoke.id,
                            );
                        }
                    });
                }
            } else if self.admin()
                && let Some(device_id) = lane.device_id
            {
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, "断开设备", Kind::Quiet)
                        .on_hover_text(
                            "结束该设备的会话；需要在设备管理中重新允许后，它才能再次发送",
                        )
                        .clicked()
                    {
                        self.operation(Operation::Disconnect { device_id });
                    }
                    if widgets::button(ui, "在设备管理中查看", Kind::Secondary).clicked() {
                        self.search = lane.name.clone();
                        self.navigate(Page::Devices);
                    }
                });
            }
        });
    }
}

/// Chevron button that folds a row's details in and out.
pub(super) fn disclosure(ui: &mut egui::Ui, open: bool, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            true,
            open,
            format!("「{name}」详情"),
        )
    });
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("open"), open, 0.16);
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::CONTROL_RADIUS),
            theme::HOVER,
        );
    }
    icons::chevron(
        ui.painter(),
        icons::square(rect.center(), 16.0),
        t,
        theme::TEXT_2,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::CONTROL_RADIUS),
            Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("详情")
}

/// Strip width, master strip width and gap of the console layout.
pub(super) const STRIP_W: f32 = 128.0;
pub(super) const MASTER_W: f32 = 188.0;
pub(super) const STRIP_GAP: f32 = 10.0;

/// The console needs every strip side by side plus the master strip; the
/// row layout takes over otherwise (narrow windows, no lanes, many lanes).
pub(super) fn console_fits(width: f32, lanes: usize) -> bool {
    (1..=6).contains(&lanes) && width >= lanes as f32 * (STRIP_W + STRIP_GAP) + MASTER_W
}

#[cfg(test)]
mod multi_airplay_tests {
    use super::*;
    #[test]
    fn meters_and_mix_follow_stream_ids_when_receiver_order_changes() {
        let authority =
            neonmix_control::Authority::new("speaker".into(), "admin".into(), &"a".repeat(64))
                .unwrap();
        let state = authority.snapshot();
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.airplay = Some(serde_json::json!({"sessions":[
            {"source_id":"source-b","source_name":"iPad","session_id":22,"stream_id":202,"mix":{"gain_db":-9.0,"muted":false,"solo":false}},
            {"source_id":"source-a","source_name":"iPhone","session_id":11,"stream_id":101,"mix":{"gain_db":-3.0,"muted":false,"solo":true}}
        ]}));
        app.diagnostics = Some(
            serde_json::json!({"lane_stream_ids":[101,202],"queues":[480,960],"meters":{"lanes":[{"stream_id":101,"peak":0.25,"rms":0.1},{"stream_id":202,"peak":0.5,"rms":0.2}]}}),
        );
        let lanes = app.lanes(&state);
        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0].key, 202);
        assert_eq!(lanes[0].peak, Some(0.5));
        assert_eq!(lanes[0].queue_ms, Some(20.0));
        assert_eq!(lanes[0].status, "因 Solo 静音");
        assert_eq!(lanes[1].airplay_target, Some(("source-a".into(), 11)));
        assert_eq!(lanes[1].gain, -3.0);
        assert_eq!(lanes[1].peak, Some(0.25));
    }
}
