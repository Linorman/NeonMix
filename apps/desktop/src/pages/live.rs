//! 现场: the room as a signal graph. Sources flow into the room core and out
//! of the physical output; the inspector beside it adjusts whichever node
//! is selected, and 房间动态 lists what changed while this window watched.
use super::*;
use crate::flow::{self, Edge};
use crate::lanes::Lane;
use crate::widgets::{FaderSize, Kind, Tone};
use egui::{Align, Layout, Margin};

const INSPECTOR_WIDTH: f32 = 300.0;
const SIDE_BY_SIDE: f32 = 1000.0;

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
        let text_live_room = self.tr(&Message::LiveRoom);
        self.remote_room
            .clone()
            .or_else(|| {
                self.status
                    .as_ref()
                    .and_then(|s| s.hub_settings.as_ref())
                    .map(|h| h.name.clone())
            })
            .unwrap_or_else(|| text_live_room.as_str().into())
    }

    pub(crate) fn output_name(&self, state: &Snapshot) -> String {
        self.devices
            .iter()
            .find(|d| d.id == state.output.id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| state.output.id.clone())
    }

    fn flow_model(&self, state: &Snapshot, lanes: &[Lane]) -> flow::Model {
        let text_live_type = self.tr(&Message::LiveType);
        let text_live_native_sender = self.tr(&Message::LiveNativeSender);
        let text_live_status = self.tr(&Message::LiveStatus);
        let text_live_volume = self.tr(&Message::LiveVolume);
        let text_live_mixer_queue_estimate = self.tr(&Message::LiveMixerQueueEstimate);
        let text_live_unavailable = self.tr(&Message::LiveUnavailable);
        let text_live_clock_drift = self.tr(&Message::LiveClockDrift);
        let text_live_media_network = self.tr(&Message::LiveMediaNetwork);
        let text_live_output_unavailable_waiting_for_device_recovery =
            self.tr(&Message::LiveOutputUnavailableWaitingForDeviceRecovery);
        let text_live_status_is_stale = self.tr(&Message::LiveStatusIsStale);
        let text_live_sharing = self.tr(&Message::LiveSharing);
        let text_live_connected = self.tr(&Message::LiveConnected);
        let any_solo = lanes.iter().any(|l| l.solo);
        let meters = self.meters_current();
        let sources = lanes
            .iter()
            .map(|lane| {
                let detail = match lane.role.as_ref() {
                    Some(role) => self.tr(&Message::MixerRoleStatus {
                        role: self.tr(role),
                        status: self.tr(&lane.status),
                    }),
                    None => self.tr(&lane.status),
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
                    status: self.tr(&lane.status),
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
                            text_live_type.clone(),
                            if lane.is_airplay() {
                                "AirPlay".into()
                            } else {
                                text_live_native_sender.clone()
                            },
                        ),
                        (text_live_status.clone(), self.tr(&lane.status)),
                        (
                            text_live_volume.clone(),
                            format!("{} dB", widgets::gain_text(lane.gain)),
                        ),
                        (
                            text_live_mixer_queue_estimate.clone(),
                            lane.queue_ms
                                .map_or(text_live_unavailable.clone(), |ms| format!("{ms:.1} ms")),
                        ),
                        (
                            text_live_clock_drift.clone(),
                            lane.drift_ppm
                                .map_or(text_live_unavailable.clone(), |p| format!("{p:+.1} ppm")),
                        ),
                        (text_live_media_network.clone(), self.tr(&lane.network.0)),
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
            (
                text_live_output_unavailable_waiting_for_device_recovery.as_str(),
                Tone::Warning,
            )
        } else if !self.writable() {
            (text_live_status_is_stale.as_str(), Tone::Warning)
        } else if sharing {
            (text_live_sharing.as_str(), Tone::Success)
        } else {
            (text_live_connected.as_str(), Tone::Success)
        };
        let limiter = meters
            .and_then(|m| m.get("limiter_gain"))
            .and_then(Value::as_f64);
        let hub_state = match limiter.filter(|g| *g < 0.999 && *g > 0.0) {
            Some(g) => self.tr(&Message::LiveHubLimiterState {
                state: (hub_state).to_string(),
                value: (widgets::db_text(g)).to_string(),
            }),
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
        let text_live_signal_flow = self.tr(&Message::LiveSignalFlow);
        let text_live_no_inputs = self.tr(&Message::LiveNoInputs);
        let text_live_native_sender = self.tr(&Message::LiveNativeSender);
        let text_live_sources_appear_on_the_left_and_join_the =
            self.tr(&Message::LiveSourcesAppearOnTheLeftAndJoinThe);
        widgets::surface(ui, None, Margin::symmetric(18, 14), |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(text_live_signal_flow.as_str())
                        .font(theme::heading(theme::SECTION))
                        .color(theme::TEXT),
                );
                widgets::pill(
                    ui,
                    &if count == 0 {
                        text_live_no_inputs.as_str().to_owned()
                    } else {
                        self.tr(&Message::LiveInputCount {
                            count: (count) as u64,
                        })
                    },
                    Tone::Neutral,
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    legend(ui, theme::SRC_AIRPLAY, "AirPlay");
                    legend(ui, theme::SRC_NATIVE, text_live_native_sender.as_str());
                });
            });
            let action = flow::show(ui, model);
            if count == 0 {
                widgets::note(
                    ui,
                    text_live_sources_appear_on_the_left_and_join_the.as_str(),
                );
            }
            action
        })
        .inner
    }

    /// No authoritative room state: say which of the three situations this
    /// is (local room not sharing, paired room unreachable, nothing yet)
    /// and offer the one action that moves it forward.
    fn live_empty(&mut self, ui: &mut egui::Ui) {
        let text_live_paired_room = self.tr(&Message::LivePairedRoom);
        let text_live_not_sharing_start_sharing_to_let_devices_connect =
            self.tr(&Message::LiveNotSharingStartSharingToLetDevicesConnect);
        let text_live_cannot_connect_right_now_make_sure_the_hub =
            self.tr(&Message::LiveCannotConnectRightNowMakeSureTheHub);
        let text_live_no_room_connected_yet = self.tr(&Message::LiveNoRoomConnectedYet);
        let text_live_create_a_room_or_join_one_on_your =
            self.tr(&Message::LiveCreateARoomOrJoinOneOnYour);
        let text_live_signal_flow = self.tr(&Message::LiveSignalFlow);
        let text_live_neonmix_brings_audio_from_devices_on_your_local =
            self.tr(&Message::LiveNeonmixBringsAudioFromDevicesOnYourLocal);
        let text_live_start_sharing = self.tr(&Message::LiveStartSharing);
        let text_live_let_devices_on_the_local_network_send_audio =
            self.tr(&Message::LiveLetDevicesOnTheLocalNetworkSendAudio);
        let text_live_hub_settings = self.tr(&Message::LiveHubSettings);
        let text_live_view_sender = self.tr(&Message::LiveViewSender);
        let text_live_diagnostics = self.tr(&Message::LiveDiagnostics);
        let text_live_create_room = self.tr(&Message::LiveCreateRoom);
        let text_live_set_up_this_computer_as_a_room_in =
            self.tr(&Message::LiveSetUpThisComputerAsARoomIn);
        let text_live_join_room = self.tr(&Message::LiveJoinRoom);
        let text_live_discover_and_pair_with_rooms_on_your_local =
            self.tr(&Message::LiveDiscoverAndPairWithRoomsOnYourLocal);
        enum Empty {
            LocalStopped(String, Option<uuid::Uuid>),
            Unreachable(String),
            Nothing,
        }
        let status = self.status.as_ref();
        let local_admin = self.credential == std::path::Path::new("hub/admin.json");
        let situation = match status.and_then(|s| s.hub_settings.as_ref()) {
            Some(settings) if local_admin && !status.is_some_and(|s| s.hub.running) => {
                let id = status
                    .and_then(|s| s.hub.last_event.as_ref())
                    .and_then(|e| e.get("hub_id")?.as_str()?.parse().ok());
                Empty::LocalStopped(settings.name.clone(), id)
            }
            _ if !local_admin
                && status.is_some_and(|s| {
                    s.profiles
                        .iter()
                        .any(|p| p.credential == self.credential && !p.pending)
                }) =>
            {
                Empty::Unreachable(
                    self.remote_room
                        .clone()
                        .unwrap_or_else(|| text_live_paired_room.as_str().into()),
                )
            }
            _ => Empty::Nothing,
        };
        let (name, id, state) = match &situation {
            Empty::LocalStopped(name, id) => (
                name.clone(),
                *id,
                text_live_not_sharing_start_sharing_to_let_devices_connect
                    .as_str()
                    .to_owned(),
            ),
            Empty::Unreachable(name) => (
                name.clone(),
                None,
                text_live_cannot_connect_right_now_make_sure_the_hub
                    .as_str()
                    .to_owned(),
            ),
            Empty::Nothing => (
                text_live_no_room_connected_yet.as_str().to_owned(),
                None,
                text_live_create_a_room_or_join_one_on_your
                    .as_str()
                    .to_owned(),
            ),
        };
        let model = flow::Model {
            sources: vec![],
            offline: 0,
            hub: flow::Hub {
                name,
                id,
                state,
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
                RichText::new(text_live_signal_flow.as_str())
                    .font(theme::heading(theme::SECTION))
                    .color(theme::TEXT),
            );
            flow::show(ui, &model);
            ui.vertical_centered(|ui| {
                widgets::note(
                    ui,
                    text_live_neonmix_brings_audio_from_devices_on_your_local.as_str(),
                );
            });
            ui.horizontal(|ui| {
                let pad = ((ui.available_width() - 300.0) / 2.0).max(0.0);
                ui.add_space(pad);
                match situation {
                    Empty::LocalStopped(..) => {
                        if widgets::button_busy(
                            ui,
                            true,
                            self.pending("hub-start"),
                            text_live_start_sharing.as_str(),
                            Kind::Primary,
                        )
                        .on_hover_text(
                            text_live_let_devices_on_the_local_network_send_audio.as_str(),
                        )
                        .clicked()
                        {
                            self.request(Request::HubStart);
                        }
                        if widgets::button(ui, text_live_hub_settings.as_str(), Kind::Secondary)
                            .clicked()
                        {
                            self.navigate(Page::Hub);
                        }
                    }
                    Empty::Unreachable(_) => {
                        if widgets::button(ui, text_live_view_sender.as_str(), Kind::Primary)
                            .clicked()
                        {
                            self.navigate(Page::Sender);
                        }
                        if widgets::button(ui, text_live_diagnostics.as_str(), Kind::Secondary)
                            .clicked()
                        {
                            self.navigate(Page::Diagnostics);
                        }
                    }
                    Empty::Nothing => {
                        if widgets::button(ui, text_live_create_room.as_str(), Kind::Primary)
                            .on_hover_text(text_live_set_up_this_computer_as_a_room_in.as_str())
                            .clicked()
                        {
                            self.navigate(Page::Hub);
                        }
                        if widgets::button(ui, text_live_join_room.as_str(), Kind::Secondary)
                            .on_hover_text(
                                text_live_discover_and_pair_with_rooms_on_your_local.as_str(),
                            )
                            .clicked()
                        {
                            self.navigate(Page::Sender);
                        }
                    }
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
                // Under the graph the card is wide: facts left, controls
                // right, instead of one tall column with a very long fader.
                let split = ui.available_width() >= 560.0;
                let info = |this: &mut Self, ui: &mut egui::Ui| match lane {
                    Some(lane) => this.lane_info(ui, lane),
                    None => this.room_info(ui, lanes),
                };
                let controls = |this: &mut Self, ui: &mut egui::Ui| match lane {
                    Some(lane) => this.lane_controls(ui, lane),
                    None => this.room_controls(ui, state),
                };
                if split {
                    ui.columns(2, |c| {
                        for column in c.iter_mut() {
                            column.spacing_mut().item_spacing.y = 10.0;
                        }
                        let (l, r) = c.split_at_mut(1);
                        l[0].with_layout(Layout::top_down(Align::Min), |ui| info(self, ui));
                        r[0].with_layout(Layout::top_down(Align::Min), |ui| controls(self, ui));
                    });
                } else {
                    info(self, ui);
                    controls(self, ui);
                }
            },
        );
    }

    fn lane_info(&mut self, ui: &mut egui::Ui, lane: &Lane) {
        let text_live_mixer_queue_estimate = self.tr(&Message::LiveMixerQueueEstimate);
        let text_live_unavailable = self.tr(&Message::LiveUnavailable);
        let text_live_clock_drift = self.tr(&Message::LiveClockDrift);
        let text_live_media_network = self.tr(&Message::LiveMediaNetwork);
        let text_live_open_in_mixer = self.tr(&Message::LiveOpenInMixer);
        let text_live_deselect = self.tr(&Message::LiveDeselect);
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
                    RichText::new(match lane.role.as_ref() {
                        Some(role) => self.tr(&Message::MixerRoleStatus {
                            role: self.tr(role),
                            status: self.tr(&lane.status),
                        }),
                        None => self.tr(&lane.status),
                    })
                    .size(theme::SMALL)
                    .color(lane.tone.color()),
                );
            });
        });
        widgets::meter(ui, ("inspect", lane.key), lane.peak, lane.rms);
        widgets::kv_grid(
            ui,
            "inspect-facts",
            &[
                (
                    text_live_mixer_queue_estimate.as_str(),
                    lane.queue_ms
                        .map_or(text_live_unavailable.as_str().into(), |ms| {
                            format!("{ms:.1} ms")
                        }),
                ),
                (
                    text_live_clock_drift.as_str(),
                    lane.drift_ppm
                        .map_or(text_live_unavailable.as_str().into(), |p| {
                            format!("{p:+.1} ppm")
                        }),
                ),
                (text_live_media_network.as_str(), self.tr(&lane.network.0)),
            ],
        );
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, text_live_open_in_mixer.as_str(), Kind::Secondary).clicked() {
                self.lane_details.insert(lane.key);
                self.navigate(Page::Mixer);
            }
            if widgets::button(ui, text_live_deselect.as_str(), Kind::Secondary).clicked() {
                self.selected_lane = None;
            }
        });
    }

    fn lane_controls(&mut self, ui: &mut egui::Ui, lane: &Lane) {
        let text_live_volume = self.tr(&Message::LiveVolume);
        let text_live_mute = self.tr(&Message::LiveMute);
        let mut commit = None;
        ui.add_enabled_ui(self.writable() && lane.can_mix, |ui| {
            let caption = widgets::caption(ui, text_live_volume.as_str());
            let (c, response) = self.gain_control(
                ui,
                lane.key,
                lane.gain,
                &self.tr(&Message::MixerChannelVolume {
                    name: (lane.name).to_string(),
                }),
                FaderSize::Row,
            );
            response.labelled_by(caption.id);
            commit = c;
        });
        let mut toggles = (None, None);
        self.command_note(ui, lane.key);
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
                    && widgets::toggle(
                        ui,
                        self.writable(),
                        lane.solo,
                        &self.tr(&Message::MixerSolo),
                        Tone::Solo,
                    )
                    .clicked()
                {
                    toggles.1 = Some(!lane.solo);
                }
                if widgets::toggle(
                    ui,
                    self.writable() && lane.can_mix,
                    lane.muted,
                    text_live_mute.as_str(),
                    Tone::Warning,
                )
                .clicked()
                {
                    toggles.0 = Some(!lane.muted);
                }
            });
        });
        if let Some(gain) = commit {
            self.lane_gain(lane, gain);
        }
        if toggles.0.is_some() || toggles.1.is_some() {
            self.lane_toggle(lane.key, toggles.0, toggles.1);
        }
    }

    fn room_info(&mut self, ui: &mut egui::Ui, lanes: &[Lane]) {
        let text_live_room_overview = self.tr(&Message::LiveRoomOverview);
        let text_live_native_sender = self.tr(&Message::LiveNativeSender);
        let text_live_largest_queue_estimate = self.tr(&Message::LiveLargestQueueEstimate);
        let text_live_unavailable = self.tr(&Message::LiveUnavailable);
        let text_live_after_sources_connect_select_one_on_the_left =
            self.tr(&Message::LiveAfterSourcesConnectSelectOneOnTheLeft);
        let text_live_select_a_source_on_the_left_to_view =
            self.tr(&Message::LiveSelectASourceOnTheLeftToView);
        ui.label(
            RichText::new(text_live_room_overview.as_str())
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
            widgets::metric(
                ui,
                text_live_native_sender.as_str(),
                &native.to_string(),
                None,
            );
            widgets::metric(ui, "AirPlay", &airplay.to_string(), None);
            widgets::metric(
                ui,
                text_live_largest_queue_estimate.as_str(),
                &queue.map_or(text_live_unavailable.as_str().into(), |q| {
                    format!("{q:.1} ms")
                }),
                None,
            );
        });
        widgets::note(
            ui,
            if lanes.is_empty() {
                text_live_after_sources_connect_select_one_on_the_left.as_str()
            } else {
                text_live_select_a_source_on_the_left_to_view.as_str()
            },
        );
    }

    fn room_controls(&mut self, ui: &mut egui::Ui, state: &Snapshot) {
        let text_live_room_master_volume = self.tr(&Message::LiveRoomMasterVolume);
        let text_live_master_volume = self.tr(&Message::LiveMasterVolume);
        let text_live_unmute_master = self.tr(&Message::LiveUnmuteMaster);
        let text_live_mute_master = self.tr(&Message::LiveMuteMaster);
        let text_live_master_controls_require_a_room_controller_or_administrator =
            self.tr(&Message::LiveMasterControlsRequireARoomControllerOrAdministrator);
        if self.controls_room() {
            let current = state.output.gain_db;
            let mut commit = None;
            ui.add_enabled_ui(self.writable(), |ui| {
                let caption = widgets::caption(ui, text_live_room_master_volume.as_str());
                let (c, response) = self.gain_control(
                    ui,
                    0,
                    current,
                    text_live_master_volume.as_str(),
                    FaderSize::Row,
                );
                response.labelled_by(caption.id);
                commit = c;
            });
            self.command_note(ui, 0);
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
                            text_live_unmute_master.as_str()
                        } else {
                            text_live_mute_master.as_str()
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
        if !self.controls_room() {
            widgets::note(
                ui,
                text_live_master_controls_require_a_room_controller_or_administrator.as_str(),
            );
        }
    }

    fn events_card(&mut self, ui: &mut egui::Ui) {
        let text_live_room_activity = self.tr(&Message::LiveRoomActivity);
        let text_live_since_this_window_opened = self.tr(&Message::LiveSinceThisWindowOpened);
        let text_live_no_changes_yet_source_connections_disconnections_muting_and =
            self.tr(&Message::LiveNoChangesYetSourceConnectionsDisconnectionsMutingAnd);
        widgets::surface(ui, None, Margin::symmetric(16, 12), |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(text_live_room_activity.as_str())
                        .font(theme::heading(theme::SECTION))
                        .color(theme::TEXT),
                );
                widgets::note(ui, text_live_since_this_window_opened.as_str());
            });
            if self.events.is_empty() {
                widgets::note(
                    ui,
                    text_live_no_changes_yet_source_connections_disconnections_muting_and.as_str(),
                );
                return;
            }
            for event in self.events.iter().take(8) {
                ui.horizontal(|ui| {
                    widgets::dot(
                        ui,
                        &event.text.render(&self.localization.renderer),
                        event.tone,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        widgets::note(ui, self.relative_time(event.at));
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
