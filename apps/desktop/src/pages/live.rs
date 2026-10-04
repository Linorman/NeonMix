//! 现场: the room as a signal graph. Sources flow into the room core and out
//! of the physical output; the inspector beside it adjusts whichever node
//! is selected, and 房间动态 lists what changed while this window watched.
use super::*;
use crate::flow::{self, Edge};
use crate::lanes::Lane;
use crate::widgets::{FaderSize, Kind, Tone};
use egui::{Align, Layout, Margin};

const INSPECTOR_WIDTH: f32 = 300.0;
const SIDE_BY_SIDE: f32 = 900.0;

fn edge(lane: &Lane, any_solo: bool) -> Edge {
    match lane.session {
        Some(
            SessionStatus::NetworkInterrupted
            | SessionStatus::Revoked
            | SessionStatus::AdminDisconnected
            | SessionStatus::UserStopped
            | SessionStatus::OutputLost,
        ) => Edge::Broken,
        _ if lane.muted => Edge::Muted,
        _ if any_solo && !lane.solo => Edge::SoloedOut,
        Some(SessionStatus::Playing) => Edge::Live,
        Some(SessionStatus::NetworkDegraded) => Edge::Degraded,
        Some(SessionStatus::Buffering) => Edge::Buffering,
        None => Edge::Unknown,
    }
}

impl Desktop {
    fn room_label(&self) -> String {
        self.remote_room
            .clone()
            .or_else(|| {
                self.status
                    .as_ref()
                    .and_then(|s| s.hub_settings.as_ref())
                    .map(|h| h.name.clone())
            })
            .unwrap_or_else(|| "房间".into())
    }

    pub(crate) fn output_name(&self, state: &Snapshot) -> String {
        self.devices
            .iter()
            .find(|d| d.id == state.output.id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| state.output.id.clone())
    }

    fn flow_model(&self, state: &Snapshot, lanes: &[Lane]) -> flow::Model {
        let any_solo = lanes.iter().any(|l| l.solo);
        let meters = self.diagnostics.as_ref().and_then(|v| v.pointer("/meters"));
        let sources = lanes
            .iter()
            .map(|lane| {
                let detail = match lane.role {
                    Some(role) => format!("{role} · {}", lane.status),
                    None => lane.status.to_owned(),
                };
                let appear = self
                    .joined
                    .get(&lane.key)
                    .map_or(1.0, |at| (at.elapsed().as_secs_f32() / 0.6).min(1.0));
                if appear < 1.0 && !animation::reduce_motion() {
                    self.repaint.request_repaint();
                }
                flow::Source {
                    appear: if animation::reduce_motion() {
                        1.0
                    } else {
                        appear
                    },
                    key: lane.key,
                    name: lane.name.clone(),
                    detail,
                    detail_color: lane.tone.color(),
                    airplay: lane.is_airplay(),
                    mine: lane.mine,
                    solo: lane.solo,
                    edge: edge(lane, any_solo),
                    gain_db: self
                        .gain_drafts
                        .get(&lane.key)
                        .copied()
                        .unwrap_or(lane.gain),
                    rms: lane.rms,
                    facts: vec![
                        (
                            "类型",
                            if lane.is_airplay() {
                                "AirPlay".into()
                            } else {
                                "原生 Sender".into()
                            },
                        ),
                        ("状态", lane.status.into()),
                        ("音量", format!("{} dB", widgets::gain_text(lane.gain))),
                        (
                            "Mixer 队列估计",
                            lane.queue_ms
                                .map_or("未取得".into(), |ms| format!("{ms:.1} ms")),
                        ),
                        (
                            "时钟漂移",
                            lane.drift_ppm
                                .map_or("未取得".into(), |p| format!("{p:+.1} ppm")),
                        ),
                        ("媒体网络", lane.network.0.into()),
                    ],
                }
            })
            .collect();
        let local_admin: Vec<_> = self
            .status
            .iter()
            .flat_map(|s| &s.profiles)
            .filter(|p| p.credential == std::path::Path::new("hub/admin.json"))
            .filter_map(|p| p.device_id)
            .collect();
        let offline = state
            .devices
            .values()
            .filter(|d| !d.revoked && !local_admin.contains(&d.id))
            .filter(|d| !state.streams.values().any(|s| s.device_id == d.id))
            .count();
        let sharing = self.status.as_ref().is_some_and(|s| s.hub.running);
        let (hub_state, hub_tone) = if !state.output.available {
            ("输出丢失 · 等待设备恢复", Tone::Warning)
        } else if !self.writable() {
            ("状态已过期", Tone::Warning)
        } else if sharing {
            ("共享中", Tone::Success)
        } else {
            ("已连接", Tone::Success)
        };
        let limiter = meters
            .and_then(|m| m.get("limiter_gain"))
            .and_then(Value::as_f64);
        let hub_state = match limiter.filter(|g| *g < 0.999 && *g > 0.0) {
            Some(g) => format!("{hub_state} · 限幅 {} dB", widgets::db_text(g)),
            None => hub_state.to_owned(),
        };
        flow::Model {
            sources,
            offline,
            hub: flow::Hub {
                name: self.room_label(),
                id: Some(state.hub_id),
                state: hub_state,
                state_color: hub_tone.color(),
                active: state.output.available,
                rms: meters
                    .and_then(|m| m.pointer("/output/rms"))
                    .and_then(Value::as_f64),
                limiter,
            },
            output: Some(flow::Output {
                name: self.output_name(state),
                gain_db: self
                    .gain_drafts
                    .get(&0)
                    .copied()
                    .unwrap_or(state.output.gain_db),
                muted: state.output.muted,
                available: state.output.available,
            }),
            selected: self.selected_lane,
        }
    }

    pub(crate) fn live_page(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.snapshot.clone() else {
            self.live_empty(ui);
            return;
        };
        let lanes = self.lanes(&state);
        if self
            .selected_lane
            .is_some_and(|k| !lanes.iter().any(|l| l.key == k))
        {
            self.selected_lane = None;
        }
        self.mixer_keys(ui, &lanes, false);
        let model = self.flow_model(&state, &lanes);
        let mut action = None;
        if ui.available_width() >= SIDE_BY_SIDE {
            ui.horizontal_top(|ui| {
                let flow_width = ui.available_width() - INSPECTOR_WIDTH - 14.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(flow_width, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| action = self.flow_card(ui, &model, lanes.len()),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(INSPECTOR_WIDTH, 0.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 14.0;
                        self.inspector(ui, &state, &lanes);
                    },
                );
            });
        } else {
            action = self.flow_card(ui, &model, lanes.len());
            self.inspector(ui, &state, &lanes);
        }
        match action {
            Some(flow::Action::Select(key)) => {
                self.selected_lane = (self.selected_lane != Some(key)).then_some(key);
            }
            Some(flow::Action::OpenMixer) => self.navigate(Page::Mixer),
            Some(flow::Action::OpenDevices) => self.navigate(Page::Devices),
            None => {}
        }
        self.events_card(ui);
    }

    fn flow_card(
        &mut self,
        ui: &mut egui::Ui,
        model: &flow::Model,
        count: usize,
    ) -> Option<flow::Action> {
        widgets::surface(ui, None, Margin::symmetric(18, 14), |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("信号汇流")
                        .font(theme::heading(theme::SECTION))
                        .color(theme::TEXT),
                );
                widgets::pill(
                    ui,
                    &if count == 0 {
                        "暂无输入".to_owned()
                    } else {
                        format!("{count} 路输入")
                    },
                    Tone::Neutral,
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    legend(ui, theme::SRC_AIRPLAY, "AirPlay");
                    legend(ui, theme::SRC_NATIVE, "原生 Sender");
                });
            });
            let action = flow::show(ui, model);
            if count == 0 {
                widgets::note(
                    ui,
                    "Sender 配对并开始发送，或 AirPlay 设备连接后，来源会出现在左侧并接入房间。",
                );
            }
            action
        })
        .inner
    }

    fn live_empty(&mut self, ui: &mut egui::Ui) {
        let model = flow::Model {
            sources: vec![],
            offline: 0,
            hub: flow::Hub {
                name: "尚未连接房间".into(),
                id: None,
                state: "创建一个房间，或加入局域网里的房间".into(),
                state_color: theme::TEXT_3,
                active: false,
                rms: None,
                limiter: None,
            },
            output: None,
            selected: None,
        };
        widgets::surface(ui, None, Margin::symmetric(18, 14), |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new("信号汇流")
                    .font(theme::heading(theme::SECTION))
                    .color(theme::TEXT),
            );
            flow::show(ui, &model);
            ui.vertical_centered(|ui| {
                widgets::note(
                    ui,
                    "NeonMix 把局域网里多台设备的声音汇入一个房间，混音后从一个实体输出播放。",
                );
            });
            ui.horizontal(|ui| {
                let pad = ((ui.available_width() - 300.0) / 2.0).max(0.0);
                ui.add_space(pad);
                if widgets::button(ui, "创建房间", Kind::Primary)
                    .on_hover_text("在 Hub 设置中让这台电脑成为房间")
                    .clicked()
                {
                    self.navigate(Page::Hub);
                }
                if widgets::button(ui, "加入房间", Kind::Secondary)
                    .on_hover_text("在 Sender 中发现并配对局域网里的房间")
                    .clicked()
                {
                    self.navigate(Page::Sender);
                }
            });
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui, state: &Snapshot, lanes: &[Lane]) {
        let lane = self
            .selected_lane
            .and_then(|k| lanes.iter().find(|l| l.key == k));
        widgets::surface(
            ui,
            lane.map(|l| l.source_color()),
            Margin::symmetric(16, 14),
            |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                match lane {
                    Some(lane) => self.inspect_lane(ui, lane),
                    None => self.inspect_room(ui, state, lanes),
                }
            },
        );
    }

    fn inspect_lane(&mut self, ui: &mut egui::Ui, lane: &Lane) {
        ui.horizontal(|ui| {
            if lane.is_airplay() {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    rect.center(),
                    15.0,
                    theme::SRC_AIRPLAY.gamma_multiply(0.18),
                );
                icons::paint(
                    ui.painter(),
                    icons::square(rect.center(), 16.0),
                    icons::Icon::AirPlay,
                    theme::SRC_AIRPLAY,
                );
            } else {
                widgets::avatar(ui, &lane.name, theme::SRC_NATIVE, 30.0);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(&lane.name)
                            .font(theme::heading(theme::SECTION))
                            .color(theme::TEXT),
                    )
                    .truncate(),
                );
                ui.label(
                    RichText::new(match lane.role {
                        Some(role) => format!("{role} · {}", lane.status),
                        None => lane.status.into(),
                    })
                    .size(theme::SMALL)
                    .color(lane.tone.color()),
                );
            });
        });
        widgets::meter(ui, ("inspect", lane.key), lane.peak, lane.rms);
        let mut commit = None;
        ui.add_enabled_ui(self.writable() && lane.can_mix, |ui| {
            let caption = widgets::caption(ui, "音量");
            let (c, response) = self.gain_control(
                ui,
                lane.key,
                lane.gain,
                &format!("「{}」音量", lane.name),
                FaderSize::Row,
            );
            response.labelled_by(caption.id);
            commit = c;
        });
        let mut toggles = (None, None);
        ui.horizontal(|ui| {
            let shown = self
                .gain_drafts
                .get(&lane.key)
                .copied()
                .unwrap_or(lane.gain);
            ui.label(
                RichText::new(format!("{} dB", widgets::gain_text(shown)))
                    .monospace()
                    .size(15.0)
                    .color(if shown > 0.0 {
                        theme::WARNING
                    } else {
                        theme::TEXT
                    }),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if lane.can_solo
                    && widgets::toggle(ui, self.writable(), lane.solo, "Solo", Tone::Solo).clicked()
                {
                    toggles.1 = Some(!lane.solo);
                }
                if widgets::toggle(
                    ui,
                    self.writable() && lane.can_mix,
                    lane.muted,
                    "静音",
                    Tone::Warning,
                )
                .clicked()
                {
                    toggles.0 = Some(!lane.muted);
                }
            });
        });
        widgets::kv_grid(
            ui,
            "inspect-facts",
            &[
                (
                    "Mixer 队列估计",
                    lane.queue_ms
                        .map_or("未取得".into(), |ms| format!("{ms:.1} ms")),
                ),
                (
                    "时钟漂移",
                    lane.drift_ppm
                        .map_or("未取得".into(), |p| format!("{p:+.1} ppm")),
                ),
                ("媒体网络", lane.network.0.into()),
            ],
        );
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, "在 Mixer 中打开", Kind::Secondary).clicked() {
                self.lane_details.insert(lane.key);
                self.navigate(Page::Mixer);
            }
            if widgets::button(ui, "取消选择", Kind::Secondary).clicked() {
                self.selected_lane = None;
            }
        });
        if let Some(gain) = commit {
            self.lane_gain(lane, gain);
        }
        if toggles.0.is_some() || toggles.1.is_some() {
            self.lane_toggle(lane.key, toggles.0, toggles.1);
        }
    }

    fn inspect_room(&mut self, ui: &mut egui::Ui, state: &Snapshot, lanes: &[Lane]) {
        ui.label(
            RichText::new("房间概况")
                .font(theme::heading(theme::SECTION))
                .color(theme::TEXT),
        );
        let native = lanes.iter().filter(|l| !l.is_airplay()).count();
        let airplay = lanes.len() - native;
        let queue = lanes
            .iter()
            .filter_map(|l| l.queue_ms)
            .fold(None, |m: Option<f64>, q| Some(m.map_or(q, |m| m.max(q))));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 22.0;
            widgets::metric(ui, "原生 Sender", &native.to_string(), None);
            widgets::metric(ui, "AirPlay", &airplay.to_string(), None);
            widgets::metric(
                ui,
                "最大队列估计",
                &queue.map_or("未取得".into(), |q| format!("{q:.1} ms")),
                None,
            );
        });
        if self.controls_room() {
            let current = state.output.gain_db;
            let mut commit = None;
            ui.add_enabled_ui(self.writable(), |ui| {
                let caption = widgets::caption(ui, "房间总音量");
                let (c, response) = self.gain_control(ui, 0, current, "总音量", FaderSize::Row);
                response.labelled_by(caption.id);
                commit = c;
            });
            ui.horizontal(|ui| {
                let shown = self.gain_drafts.get(&0).copied().unwrap_or(current);
                ui.label(
                    RichText::new(format!("{} dB", widgets::gain_text(shown)))
                        .monospace()
                        .size(15.0)
                        .color(if shown > 0.0 {
                            theme::WARNING
                        } else {
                            theme::TEXT
                        }),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                });
            });
            if let Some(gain) = commit {
                self.master_gain(current, gain);
            }
        }
        widgets::note(
            ui,
            if lanes.is_empty() {
                "来源接入后，点击左侧来源可在这里调节。"
            } else {
                "点击左侧来源查看并调节该路；点击房间核心打开 Mixer。"
            },
        );
    }

    fn events_card(&mut self, ui: &mut egui::Ui) {
        widgets::surface(ui, None, Margin::symmetric(16, 12), |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("房间动态")
                        .font(theme::heading(theme::SECTION))
                        .color(theme::TEXT),
                );
                widgets::note(ui, "本窗口打开以来");
            });
            if self.events.is_empty() {
                widgets::note(
                    ui,
                    "暂无变化。来源接入、离开、静音或网络状态变化会记录在这里。",
                );
                return;
            }
            for event in self.events.iter().take(8) {
                ui.horizontal(|ui| {
                    widgets::dot(ui, &event.text, event.tone);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        widgets::note(ui, crate::events::ago(event.at));
                    });
                });
            }
        });
    }
}

fn legend(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.label(RichText::new(label).size(theme::SMALL).color(theme::TEXT_3));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 10.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        egui::Stroke::new(2.5, color),
    );
}
