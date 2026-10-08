//! Hub 设置: the room hero (name, output, sharing), setup steps until the
//! room is ready, then invitations and the AirPlay receiver.
use super::*;
use crate::widgets::{Kind, Step, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

impl Desktop {
    pub(crate) fn hub_page(&mut self, ui: &mut egui::Ui) {
        let text_hub_create_room = self.tr(&Message::HubCreateRoom);
        let text_hub_start_sharing = self.tr(&Message::HubStartSharing);
        let text_hub_invite_device = self.tr(&Message::HubInviteDevice);
        let configured = self
            .status
            .as_ref()
            .is_some_and(|s| s.hub_settings.is_some());
        let running = self.status.as_ref().is_some_and(|s| s.hub.running);
        let members = self.snapshot.as_ref().map_or(0, |s| s.devices.len());
        let invited = members > 1 || !self.issued_invitation.is_empty();
        if !(configured && running && invited) {
            let state = |done: bool, current: bool| {
                if done {
                    Step::Done
                } else if current {
                    Step::Current
                } else {
                    Step::Todo
                }
            };
            let steps = [
                (text_hub_create_room.as_str(), state(configured, true)),
                (text_hub_start_sharing.as_str(), state(running, configured)),
                (
                    text_hub_invite_device.as_str(),
                    state(invited, configured && running),
                ),
            ];
            if widgets::stepper(ui, &steps) == Some(2) {
                self.panels.insert("invite", true);
                self.scroll_to = Some("invite");
            }
        }
        self.room_hero(ui, configured, running);
        if ui.available_width() >= TWO_COLUMNS {
            ui.columns(2, |c| {
                for column in c.iter_mut() {
                    column.spacing_mut().item_spacing.y = 14.0;
                }
                self.invite_panel(&mut c[0]);
                self.airplay_panel(&mut c[1]);
            });
        } else {
            self.invite_panel(ui);
            self.airplay_panel(ui);
        }
    }

    fn room_hero(&mut self, ui: &mut egui::Ui, configured: bool, running: bool) {
        let text_hub_sharing = self.tr(&Message::HubSharing);
        let text_hub_not_sharing = self.tr(&Message::HubNotSharing);
        let text_hub_not_created = self.tr(&Message::HubNotCreated);
        let text_hub_stop_sharing = self.tr(&Message::HubStopSharing);
        let text_hub_start_sharing = self.tr(&Message::HubStartSharing);
        let text_hub_stop_sharing_the_room_stops_playing_paired_devices =
            self.tr(&Message::HubStopSharingTheRoomStopsPlayingPairedDevices);
        let text_hub_save_or_discard_your_changes_first =
            self.tr(&Message::HubSaveOrDiscardYourChangesFirst);
        let text_hub_start_sharing_devices_on_your_local_network_can =
            self.tr(&Message::HubStartSharingDevicesOnYourLocalNetworkCan);
        let text_hub_create_room = self.tr(&Message::HubCreateRoom);
        let text_hub_enter_a_room_name_and_select_a_physical =
            self.tr(&Message::HubEnterARoomNameAndSelectAPhysical);
        let text_hub_room_name = self.tr(&Message::HubRoomName);
        let text_hub_name_your_room = self.tr(&Message::HubNameYourRoom);
        let text_hub_the_output_is_bound_to_its_device_identity =
            self.tr(&Message::HubTheOutputIsBoundToItsDeviceIdentity);
        let saved = self.status.as_ref().and_then(|s| s.hub_settings.clone());
        let dirty = saved
            .as_ref()
            .is_some_and(|s| s.name != self.room.trim() || s.output != self.output);
        let (state, tone) = if running {
            (text_hub_sharing.as_str(), Tone::Success)
        } else if configured {
            (text_hub_not_sharing.as_str(), Tone::Neutral)
        } else {
            (text_hub_not_created.as_str(), Tone::Neutral)
        };
        let shown = widgets::surface(
            ui,
            (tone != Tone::Neutral).then(|| tone.color()),
            Margin {
                left: 20,
                right: 18,
                top: 16,
                bottom: 16,
            },
            |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                let wide = ui.available_width() >= 560.0;
                let action = |this: &mut Self, ui: &mut egui::Ui| {
                    if configured {
                        // One switch: its accessible name is the action it performs.
                        let starting = this.pending("hub-start");
                        let stopping = this.pending("hub-stop")
                            || (this.pending_stop
                                && matches!(this.urgent_target, crate::LocalStop::Hub));
                        let can_stop = running || starting;
                        let label = if can_stop {
                            text_hub_stop_sharing.as_str()
                        } else {
                            text_hub_start_sharing.as_str()
                        };
                        let switch = crate::viz::switch(
                            ui,
                            (can_stop || !dirty) && !stopping,
                            running,
                            starting || stopping,
                            label,
                        );
                        let switch = if can_stop {
                            switch.on_hover_text(
                                text_hub_stop_sharing_the_room_stops_playing_paired_devices
                                    .as_str(),
                            )
                        } else if dirty {
                            switch
                                .on_hover_text(text_hub_save_or_discard_your_changes_first.as_str())
                        } else {
                            switch.on_hover_text(
                                text_hub_start_sharing_devices_on_your_local_network_can.as_str(),
                            )
                        };
                        if switch.clicked() && !stopping && (can_stop || !dirty) {
                            this.request(if can_stop {
                                Request::HubStop
                            } else {
                                Request::HubStart
                            });
                        }
                    } else if widgets::button_busy(
                        ui,
                        !this.room.trim().is_empty() && !this.output.is_empty(),
                        this.pending("hub-settings"),
                        text_hub_create_room.as_str(),
                        Kind::Primary,
                    )
                    .on_disabled_hover_text(
                        text_hub_enter_a_room_name_and_select_a_physical.as_str(),
                    )
                    .clicked()
                    {
                        this.request(Request::HubSetup {
                            settings: HubSettings {
                                name: this.room.trim().into(),
                                output: this.output.clone(),
                            },
                        });
                    }
                    widgets::pill(ui, state, tone);
                };
                if wide {
                    ui.horizontal(|ui| {
                        self.room_emblem(ui, running);
                        let width = ui.available_width() - 240.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 56.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                widgets::title_field(
                                    ui,
                                    "hub-room-name",
                                    text_hub_room_name.as_str(),
                                    &mut self.room,
                                    text_hub_name_your_room.as_str(),
                                );
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 56.0),
                            Layout::right_to_left(Align::Center),
                            |ui| action(self, ui),
                        );
                    });
                } else {
                    widgets::title_field(
                        ui,
                        "hub-room-name",
                        text_hub_room_name.as_str(),
                        &mut self.room,
                        text_hub_name_your_room.as_str(),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT),
                        Layout::right_to_left(Align::Center),
                        |ui| action(self, ui),
                    );
                }
                self.output_picker(ui);
                if dirty && let Some(saved) = &saved {
                    self.unsaved_bar(ui, saved, running);
                }
                widgets::note(
                    ui,
                    text_hub_the_output_is_bound_to_its_device_identity.as_str(),
                );
                if let Some(process) = self
                    .status
                    .as_ref()
                    .map(|s| &s.hub)
                    .filter(|process| process.error.is_some() || process.fault.is_some())
                {
                    widgets::error_text(ui, self.tr(&process_error(process)));
                }
                if let Some(snapshot) = self.snapshot.clone() {
                    ui.separator();
                    self.member_strip(ui, &snapshot);
                }
            },
        );
        let rect = shown.response.rect;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 16.0),
            egui::vec2(3.0, (rect.height() - 32.0).max(8.0)),
        );
        let color = animation::color(ui.ctx(), ui.id().with("room-rail"), tone.color());
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }

    /// The room's identity glyph; a one-shot ripple when sharing starts.
    fn room_emblem(&mut self, ui: &mut egui::Ui, running: bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
        let hub_id = self.snapshot.as_ref().map(|s| s.hub_id).or_else(|| {
            self.status
                .as_ref()?
                .hub
                .last_event
                .as_ref()?
                .get("hub_id")?
                .as_str()?
                .parse()
                .ok()
        });
        let painter = ui.painter();
        match hub_id {
            Some(id) => crate::emblem::paint(
                painter,
                rect.center(),
                26.0,
                crate::emblem::Emblem::from_id(id),
                if running { 1.0 } else { 0.0 },
            ),
            None => {
                let pts: Vec<egui::Pos2> = (0..=48)
                    .map(|i| {
                        rect.center()
                            + egui::Vec2::angled(i as f32 / 48.0 * std::f32::consts::TAU) * 26.0
                    })
                    .collect();
                crate::fx::dashed(
                    painter,
                    &pts,
                    Stroke::new(1.0, theme::BORDER_STRONG),
                    3.0,
                    3.0,
                );
            }
        }
        if let Some(t) =
            animation::once(ui.ctx(), ui.id().with("share-ripple"), running as u64, 0.7)
            && running
        {
            let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("ripple"));
            crate::viz::ripple(
                &ui.ctx().layer_painter(layer),
                rect.center(),
                26.0,
                t,
                theme::SUCCESS,
            );
        }
    }

    fn output_picker(&mut self, ui: &mut egui::Ui) {
        let text_hub_physical_output_device = self.tr(&Message::HubPhysicalOutputDevice);
        let text_hub_select_a_device = self.tr(&Message::HubSelectADevice);
        let text_hub_physical_output = self.tr(&Message::HubPhysicalOutput);
        let text_hub_refresh_devices = self.tr(&Message::HubRefreshDevices);
        let text_hub_play_test_tone = self.tr(&Message::HubPlayTestTone);
        let text_hub_play_a_quiet_2_second_test_tone_at =
            self.tr(&Message::HubPlayAQuiet2SecondTestToneAt);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let label = widgets::caption(ui, text_hub_physical_output_device.as_str());
            ui.horizontal_wrapped(|ui| {
                let width = (ui.available_width() - 210.0).clamp(160.0, 360.0);
                let response = egui::ComboBox::from_id_salt("output")
                    .width(width)
                    .selected_text(
                        self.devices
                            .iter()
                            .find(|d| d.id == self.output)
                            .map_or(text_hub_select_a_device.as_str(), |d| d.name.as_str()),
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
                widgets::label_combo(&response, text_hub_physical_output.as_str());
                if widgets::button_busy(
                    ui,
                    true,
                    self.pending("devices"),
                    text_hub_refresh_devices.as_str(),
                    Kind::Secondary,
                )
                .clicked()
                {
                    self.request(Request::Devices);
                }
                if widgets::button_busy(
                    ui,
                    !self.output.is_empty(),
                    self.pending("test-tone"),
                    text_hub_play_test_tone.as_str(),
                    Kind::Secondary,
                )
                .on_hover_text(text_hub_play_a_quiet_2_second_test_tone_at.as_str())
                .clicked()
                {
                    self.request(Request::TestTone {
                        output: self.output.clone(),
                    });
                }
            });
            if !self.output.is_empty() {
                widgets::mono(ui, &self.output);
            }
        });
    }

    /// Edits apply only on 保存设置; while sharing they wait for a stop.
    fn unsaved_bar(&mut self, ui: &mut egui::Ui, saved: &HubSettings, running: bool) {
        let text_hub_unsaved_changes = self.tr(&Message::HubUnsavedChanges);
        let text_hub_stop_sharing_before_saving = self.tr(&Message::HubStopSharingBeforeSaving);
        let text_hub_save_settings = self.tr(&Message::HubSaveSettings);
        let text_hub_discard_changes = self.tr(&Message::HubDiscardChanges);
        egui::Frame::new()
            .fill(theme::WARNING.gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, theme::WARNING.gamma_multiply(0.35)))
            .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    widgets::dot(ui, text_hub_unsaved_changes.as_str(), Tone::Warning);
                    if running {
                        widgets::note(ui, text_hub_stop_sharing_before_saving.as_str());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button_busy(
                            ui,
                            !running && !self.room.trim().is_empty() && !self.output.is_empty(),
                            self.pending("hub-settings"),
                            text_hub_save_settings.as_str(),
                            Kind::Primary,
                        )
                        .clicked()
                        {
                            self.request(Request::HubSettings {
                                settings: HubSettings {
                                    name: self.room.trim().into(),
                                    output: self.output.clone(),
                                },
                            });
                        }
                        if widgets::button(ui, text_hub_discard_changes.as_str(), Kind::Secondary)
                            .clicked()
                        {
                            self.room = saved.name.clone();
                            self.output = saved.output.clone();
                        }
                    });
                });
            });
    }

    /// Who is in the room, at a glance; the full list lives in 设备管理.
    fn member_strip(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let text_hub_manage_devices = self.tr(&Message::HubManageDevices);
        let mut devices: Vec<_> = snapshot.devices.values().collect();
        devices.sort_by_key(|d| (d.revoked, !d.playback_allowed, d.name.clone()));
        let airplay = self.airplay_sources();
        let sending = snapshot
            .streams
            .values()
            .map(|s| s.device_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            + self.airplay_sessions().len();
        // Wrapping row: on narrow windows it must not widen the hero.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = -6.0;
            for device in devices.iter().take(6) {
                let color = if device.revoked {
                    theme::DANGER
                } else {
                    role_color(device.role)
                };
                let r = widgets::avatar(ui, &device.name, color, 28.0);
                ui.painter()
                    .circle_stroke(r.rect.center(), 14.5, Stroke::new(2.0, theme::SURFACE));
                r.on_hover_text(self.tr(&Message::HubMemberRole {
                    name: (device.name).to_string(),
                    role: self.tr(&role_name(device.role)),
                }));
            }
            for source in airplay.iter().take(6usize.saturating_sub(devices.len())) {
                widgets::avatar(
                    ui,
                    source["source_name"].as_str().unwrap_or("AirPlay"),
                    theme::ACCENT,
                    28.0,
                );
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(14.0);
            ui.label(
                RichText::new(self.tr(&Message::HubPairingRecordCount {
                    count: (devices.len() + airplay.len()) as u64,
                }))
                .color(theme::TEXT),
            );
            if sending > 0 {
                widgets::dot(
                    ui,
                    &self.tr(&Message::HubSendingDeviceCount {
                        count: (sending) as u64,
                    }),
                    Tone::Success,
                );
            }
            if widgets::small_button(ui, true, text_hub_manage_devices.as_str()).clicked() {
                self.navigate(Page::Devices);
            }
        });
    }

    fn invite_panel(&mut self, ui: &mut egui::Ui) {
        let text_hub_invite_device = self.tr(&Message::HubInviteDevice);
        let text_hub_one_time_invitation_valid_for_120_seconds =
            self.tr(&Message::HubOneTimeInvitationValidFor120Seconds);
        let text_hub_invitation_created = self.tr(&Message::HubInvitationCreated);
        let text_hub_an_invitation_lets_one_device_join_the_room =
            self.tr(&Message::HubAnInvitationLetsOneDeviceJoinTheRoom);
        let text_hub_create_one_time_invitation = self.tr(&Message::HubCreateOneTimeInvitation);
        let text_hub_requires_a_connected_local_administrator_identity =
            self.tr(&Message::HubRequiresAConnectedLocalAdministratorIdentity);
        let can_invite = self.writable() && self.admin();
        let open = self.panel_open("invite", true);
        let issued = !self.issued_invitation.is_empty();
        let panel = widgets::panel(
            ui,
            "invite",
            text_hub_invite_device.as_str(),
            Some(text_hub_one_time_invitation_valid_for_120_seconds.as_str()),
            open,
            |ui| {
                if issued {
                    widgets::pill(ui, text_hub_invitation_created.as_str(), Tone::Accent);
                }
            },
            |ui| {
                widgets::note(
                    ui,
                    text_hub_an_invitation_lets_one_device_join_the_room.as_str(),
                );
                if widgets::button_busy(
                    ui,
                    can_invite,
                    self.pending("invite"),
                    text_hub_create_one_time_invitation.as_str(),
                    Kind::Primary,
                )
                .on_disabled_hover_text(
                    text_hub_requires_a_connected_local_administrator_identity.as_str(),
                )
                .clicked()
                {
                    self.request(Request::Invite {
                        credential: self.credential.clone(),
                        hub: self.hub(),
                        out: PathBuf::from(format!("invitations/{}.json", uuid::Uuid::new_v4())),
                        seconds: 120,
                    });
                }
                if issued {
                    self.issued_invite(ui);
                }
            },
        );
        if panel.toggled {
            self.toggle_panel("invite", open);
        }
        if self.scroll_to == Some("invite") {
            ui.scroll_to_rect(panel.rect, Some(Align::TOP));
            self.scroll_to = None;
        }
    }

    fn issued_invite(&mut self, ui: &mut egui::Ui) {
        let text_hub_invitation_content = self.tr(&Message::HubInvitationContent);
        let text_hub_expired = self.tr(&Message::HubExpired);
        let text_hub_copied = self.tr(&Message::HubCopied);
        let text_hub_copy_invitation = self.tr(&Message::HubCopyInvitation);
        let text_hub_hide_invitation_content = self.tr(&Message::HubHideInvitationContent);
        let text_hub_show_invitation_content = self.tr(&Message::HubShowInvitationContent);
        let text_hub_cancel_invitation = self.tr(&Message::HubCancelInvitation);
        widgets::inset(ui, |ui| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let remaining = self
                .invite_expiry
                .map(|expiry| (expiry as f64 - now).max(0.0) as u64);
            let precise = self
                .invite_expiry
                .map(|expiry| (expiry as f64 - now).max(0.0) as f32);
            let caption = ui
                .horizontal(|ui| {
                    let caption = widgets::caption(ui, text_hub_invitation_content.as_str());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match remaining {
                        Some(0) => {
                            widgets::pill(ui, text_hub_expired.as_str(), Tone::Warning);
                        }
                        Some(s) => {
                            // Real time left of the 120 s validity.
                            crate::viz::countdown_ring(
                                ui,
                                precise.unwrap_or(s as f32),
                                120.0,
                                34.0,
                            )
                            .on_hover_text(
                                self.tr(&Message::HubInvitationSecondsLeft { seconds: s }),
                            );
                        }
                        None => {}
                    });
                    caption
                })
                .inner;
            ui.add(
                egui::TextEdit::singleline(&mut self.issued_invitation)
                    .password(!self.show_invitation)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width())
                    .interactive(false),
            )
            .labelled_by(caption.id);
            // Validity drains as a thin bar under the code.
            if let Some(left) = remaining {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 3.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(2), theme::INPUT);
                let mut fill = rect;
                fill.set_width(rect.width() * (left as f32 / 120.0).clamp(0.0, 1.0));
                let tone = if left <= 20 {
                    theme::WARNING
                } else {
                    theme::ACCENT
                };
                ui.painter().rect_filled(fill, CornerRadius::same(2), tone);
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            }
            ui.horizontal_wrapped(|ui| {
                // Confirm the copy on the button itself for a moment.
                let copied = self
                    .copied_at
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                if copied {
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                let label = if copied {
                    text_hub_copied.as_str()
                } else {
                    text_hub_copy_invitation.as_str()
                };
                if widgets::small_button(ui, true, label).clicked() {
                    ui.ctx().copy_text(self.issued_invitation.clone());
                    self.copied_at = Some(Instant::now());
                }
                let show = if self.show_invitation {
                    text_hub_hide_invitation_content.as_str()
                } else {
                    text_hub_show_invitation_content.as_str()
                };
                if widgets::small_button(ui, true, show).clicked() {
                    self.show_invitation = !self.show_invitation;
                }
                if let Some(invitation_id) = self.invite_id
                    && widgets::small_button(ui, true, text_hub_cancel_invitation.as_str())
                        .clicked()
                {
                    self.request(Request::CancelInvite {
                        credential: self.credential.clone(),
                        hub: self.hub(),
                        invitation_id,
                    });
                }
            });
        });
    }
}
