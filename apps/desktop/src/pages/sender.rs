//! Sender: one sentence about where this computer's sound goes and the one
//! action that changes it; setup steps; pairing and virtual output panels.
use super::*;
use crate::widgets::{Kind, Tone};
use egui::{Align, Layout, Margin};

impl Desktop {
    fn sender_room_name(&self) -> String {
        if let Some(sender) = self.status.as_ref().map(|s| &s.sender)
            && sender.running
        {
            return sender
                .sender_target
                .as_ref()
                .and_then(|t| t.room_name.clone())
                .unwrap_or_else(|| self.tr(&Message::SenderTargetConfirming));
        }
        self.sender_snapshot()
            .and(self.remote_room.clone())
            .unwrap_or_else(|| self.tr(&Message::SenderTargetConfirming))
    }
    fn sender_snapshot(&self) -> Option<&Snapshot> {
        let status = self.status.as_ref()?;
        let hub_id = if status.sender.running {
            status.sender.sender_target.as_ref()?.hub_id
        } else {
            status
                .profiles
                .iter()
                .find(|p| p.credential == std::path::Path::new("profiles/sender.json"))?
                .hub_id?
        };
        self.snapshot.as_ref().filter(|s| s.hub_id == hub_id)
    }
    fn own_device(&self) -> Option<&neonmix_control::Device> {
        let status = self.status.as_ref()?;
        let id = if status.sender.running {
            status.sender.sender_target.as_ref()?.device_id?
        } else {
            status
                .profiles
                .iter()
                .find(|p| p.credential == std::path::Path::new("profiles/sender.json"))?
                .device_id?
        };
        self.sender_snapshot()?.devices.get(&id)
    }

    fn sender_paired(&self) -> bool {
        self.status.as_ref().is_some_and(|s| {
            s.profiles
                .iter()
                .any(|p| p.credential == std::path::Path::new("profiles/sender.json"))
        })
    }

    pub(crate) fn sender_page(&mut self, ui: &mut egui::Ui) {
        let paired = self.sender_paired();
        self.send_hero(ui);
        if ui.available_width() >= TWO_COLUMNS {
            ui.columns(2, |c| {
                for column in c.iter_mut() {
                    column.spacing_mut().item_spacing.y = 14.0;
                }
                self.pair_panel(&mut c[0], paired);
                self.binding_panel(&mut c[1]);
            });
        } else {
            self.pair_panel(ui, paired);
            self.binding_panel(ui);
        }
    }

    /// Where this computer's sound goes, stage by stage. Doubles as the
    /// setup steps: unfinished stages name the next action and open its panel.
    fn sender_pipeline(&mut self, ui: &mut egui::Ui) {
        let text_sender_virtual_output = self.tr(&Message::SenderVirtualOutput);
        let text_sender_room = self.tr(&Message::SenderRoom);
        let text_sender_app_audio = self.tr(&Message::SenderAppAudio);
        let text_sender_all_apps_using_this_output =
            self.tr(&Message::SenderAllAppsUsingThisOutput);
        let text_sender_select_an_output_in_system_sound_settings =
            self.tr(&Message::SenderSelectAnOutputInSystemSoundSettings);
        let text_sender_disabled = self.tr(&Message::SenderDisabled);
        let text_sender_add_virtual_output = self.tr(&Message::SenderAddVirtualOutput);
        let text_sender_add_after_pairing = self.tr(&Message::SenderAddAfterPairing);
        let text_sender_local_network_encrypted = self.tr(&Message::SenderLocalNetworkEncrypted);
        let text_sender_pairing_has_been_revoked = self.tr(&Message::SenderPairingHasBeenRevoked);
        let text_sender_disconnected_by_administrator =
            self.tr(&Message::SenderDisconnectedByAdministrator);
        let text_sender_paired_with_a_pinned_identity =
            self.tr(&Message::SenderPairedWithAPinnedIdentity);
        let text_sender_paste_an_invitation_to_pair =
            self.tr(&Message::SenderPasteAnInvitationToPair);
        let text_sender_discover_rooms_on_the_local_network =
            self.tr(&Message::SenderDiscoverRoomsOnTheLocalNetwork);
        let text_sender_connecting = self.tr(&Message::SenderConnecting);
        let text_sender_not_paired = self.tr(&Message::SenderNotPaired);
        let text_sender_physical_output = self.tr(&Message::SenderPhysicalOutput);
        let text_sender_output_unavailable = self.tr(&Message::SenderOutputUnavailable);
        let text_sender_shown_after_connecting_to_a_room =
            self.tr(&Message::SenderShownAfterConnectingToARoom);
        use crate::viz::{Stage, Step};
        let paired = self.sender_paired();
        let running = self.status.as_ref().is_some_and(|s| s.sender.running);
        let metrics = self.sender_metrics_current().cloned();
        let packets = metrics
            .as_ref()
            .and_then(|m| m["encoded_media_packets"].as_u64())
            .unwrap_or(0);
        let binding = self.binding.clone();
        let enabled = binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        let output_name = binding
            .as_ref()
            .and_then(|b| b["display_name"].as_str())
            .unwrap_or(text_sender_virtual_output.as_str())
            .to_owned();
        let device = self.own_device().cloned();
        let room = self.sender_room_name();
        let state = self.sender_snapshot().cloned();
        let level = state.as_ref().filter(|_| running).and_then(|s| {
            let device_id = self.own_device()?.id;
            let lane = self
                .lanes(s)
                .into_iter()
                .find(|l| l.device_id == Some(device_id))?;
            let db = widgets::to_db(lane.rms?)?;
            Some(((db - widgets::METER_FLOOR_DB) / -widgets::METER_FLOOR_DB).clamp(0.0, 1.0))
        });
        let steps = [
            Step {
                title: text_sender_app_audio.as_str(),
                detail: if binding.is_some() && enabled {
                    text_sender_all_apps_using_this_output.as_str().into()
                } else {
                    text_sender_select_an_output_in_system_sound_settings
                        .as_str()
                        .into()
                },
                state: if binding.is_some() && enabled {
                    Stage::Done
                } else {
                    Stage::Todo
                },
                icon: icons::Icon::Pulse,
            },
            Step {
                title: text_sender_virtual_output.as_str(),
                detail: match (&binding, enabled) {
                    (Some(_), true) => output_name.clone(),
                    (Some(_), false) => text_sender_disabled.as_str().into(),
                    (None, _) if paired => text_sender_add_virtual_output.as_str().into(),
                    _ => text_sender_add_after_pairing.as_str().into(),
                },
                state: match (&binding, enabled) {
                    (Some(_), true) => Stage::Done,
                    (Some(_), false) => Stage::Fault,
                    (None, _) if paired => Stage::Next,
                    _ => Stage::Todo,
                },
                icon: icons::Icon::Mixer,
            },
            Step {
                title: text_sender_local_network_encrypted.as_str(),
                detail: match &device {
                    Some(d) if d.revoked => text_sender_pairing_has_been_revoked.as_str().into(),
                    Some(d) if !d.playback_allowed => {
                        text_sender_disconnected_by_administrator.as_str().into()
                    }
                    _ if paired || running => {
                        text_sender_paired_with_a_pinned_identity.as_str().into()
                    }
                    _ if !self.candidates.is_empty() => {
                        text_sender_paste_an_invitation_to_pair.as_str().into()
                    }
                    _ => text_sender_discover_rooms_on_the_local_network
                        .as_str()
                        .into(),
                },
                state: match &device {
                    Some(d) if d.revoked || !d.playback_allowed => Stage::Fault,
                    _ if paired || running => Stage::Done,
                    _ => Stage::Next,
                },
                icon: icons::Icon::Sender,
            },
            Step {
                title: text_sender_room.as_str(),
                detail: if running && packets > 0 {
                    self.tr(&Message::SenderSendingRoom {
                        name: (room).to_string(),
                    })
                } else if running {
                    text_sender_connecting.as_str().into()
                } else if paired {
                    self.tr(&Message::SenderPairedRoomIdle {
                        name: (room).to_string(),
                    })
                } else {
                    text_sender_not_paired.as_str().into()
                },
                state: if running && packets > 0 {
                    Stage::Done
                } else if running || (paired && binding.is_some() && enabled) {
                    Stage::Next
                } else {
                    Stage::Todo
                },
                icon: icons::Icon::Flow,
            },
            Step {
                title: text_sender_physical_output.as_str(),
                detail: match &state {
                    Some(s) if s.output.available => self.output_name(s),
                    Some(_) => text_sender_output_unavailable.as_str().into(),
                    None => text_sender_shown_after_connecting_to_a_room.as_str().into(),
                },
                state: match &state {
                    Some(s) if s.output.available => Stage::Done,
                    Some(_) => Stage::Fault,
                    None => Stage::Todo,
                },
                icon: icons::Icon::Room,
            },
        ];
        match crate::viz::pipeline(ui, &steps, level) {
            Some(1) => {
                self.panels.insert("binding", true);
                self.scroll_to = Some("binding");
            }
            Some(2) => {
                self.panels.insert("pair", true);
                self.scroll_to = Some("pair");
            }
            _ => {}
        }
    }

    fn send_hero(&mut self, ui: &mut egui::Ui) {
        let text_sender_the_audio_process_is_still_running_sending_will =
            self.tr(&Message::SenderTheAudioProcessIsStillRunningSendingWill);
        let text_sender_audio_from_every_app_using_this_virtual_output =
            self.tr(&Message::SenderAudioFromEveryAppUsingThisVirtualOutput);
        let text_sender_establishing_an_encrypted_session_and_preparing_capture =
            self.tr(&Message::SenderEstablishingAnEncryptedSessionAndPreparingCapture);
        let text_sender_no_room_paired = self.tr(&Message::SenderNoRoomPaired);
        let text_sender_discover_a_hub_on_your_local_network_and =
            self.tr(&Message::SenderDiscoverAHubOnYourLocalNetworkAnd);
        let text_sender_pairing_has_been_revoked = self.tr(&Message::SenderPairingHasBeenRevoked);
        let text_sender_forget_this_computer_s_pairing_then_use_a =
            self.tr(&Message::SenderForgetThisComputerSPairingThenUseA);
        let text_sender_an_administrator_disconnected_this_device =
            self.tr(&Message::SenderAnAdministratorDisconnectedThisDevice);
        let text_sender_the_hub_must_allow_playback_again_you_can =
            self.tr(&Message::SenderTheHubMustAllowPlaybackAgainYouCan);
        let text_sender_add_a_virtual_output_as_the_destination_for =
            self.tr(&Message::SenderAddAVirtualOutputAsTheDestinationFor);
        let text_sender_virtual_output_disabled = self.tr(&Message::SenderVirtualOutputDisabled);
        let text_sender_enable_the_virtual_output_before_starting_to_send =
            self.tr(&Message::SenderEnableTheVirtualOutputBeforeStartingToSend);
        let text_sender_before_starting_select_this_virtual_device_as_the =
            self.tr(&Message::SenderBeforeStartingSelectThisVirtualDeviceAsThe);
        let text_sender_stop_sending = self.tr(&Message::SenderStopSending);
        let text_sender_start_sending = self.tr(&Message::SenderStartSending);
        let text_sender_requires_pairing_playback_permission_and_an_enabled_virtual =
            self.tr(&Message::SenderRequiresPairingPlaybackPermissionAndAnEnabledVirtual);
        let text_sender_virtual_output = self.tr(&Message::SenderVirtualOutput);
        let text_sender_not_added = self.tr(&Message::SenderNotAdded);
        let text_sender_capture_peak_session_maximum =
            self.tr(&Message::SenderCapturePeakSessionMaximum);
        let text_sender_unavailable = self.tr(&Message::SenderUnavailable);
        let text_sender_capture_frames_total = self.tr(&Message::SenderCaptureFramesTotal);
        let running = self.status.as_ref().is_some_and(|s| s.sender.running);
        let metrics = self.sender_metrics_current().cloned();
        let paired = self.sender_paired();
        let room = self.sender_room_name();
        let binding = self.binding.clone();
        let enabled = binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        let device = self.own_device().cloned();
        let blocked = device
            .as_ref()
            .is_some_and(|d| d.revoked || !d.playback_allowed);
        let packets = metrics
            .as_ref()
            .and_then(|m| m["encoded_media_packets"].as_u64())
            .unwrap_or(0);
        let control_lost = metrics
            .as_ref()
            .and_then(|m| m["control_connected"].as_bool())
            == Some(false);
        let (headline, detail, tone) = if running && control_lost {
            (
                self.tr(&Message::SenderRecoveringControl {
                    name: (room).to_string(),
                }),
                text_sender_the_audio_process_is_still_running_sending_will
                    .as_str()
                    .to_owned(),
                Tone::Warning,
            )
        } else if running && packets > 0 {
            (
                self.tr(&Message::SenderSendingRoom {
                    name: (room).to_string(),
                }),
                text_sender_audio_from_every_app_using_this_virtual_output
                    .as_str()
                    .to_owned(),
                Tone::Success,
            )
        } else if running {
            (
                self.tr(&Message::SenderConnectingRoom {
                    name: (room).to_string(),
                }),
                text_sender_establishing_an_encrypted_session_and_preparing_capture
                    .as_str()
                    .to_owned(),
                Tone::Accent,
            )
        } else if !paired {
            (
                text_sender_no_room_paired.as_str().to_owned(),
                text_sender_discover_a_hub_on_your_local_network_and
                    .as_str()
                    .to_owned(),
                Tone::Neutral,
            )
        } else if device.as_ref().is_some_and(|d| d.revoked) {
            (
                text_sender_pairing_has_been_revoked.as_str().to_owned(),
                text_sender_forget_this_computer_s_pairing_then_use_a
                    .as_str()
                    .to_owned(),
                Tone::Danger,
            )
        } else if blocked {
            (
                text_sender_an_administrator_disconnected_this_device
                    .as_str()
                    .to_owned(),
                text_sender_the_hub_must_allow_playback_again_you_can
                    .as_str()
                    .to_owned(),
                Tone::Warning,
            )
        } else if binding.is_none() {
            (
                self.tr(&Message::SenderPairedRoom {
                    name: (room).to_string(),
                }),
                text_sender_add_a_virtual_output_as_the_destination_for
                    .as_str()
                    .to_owned(),
                Tone::Accent,
            )
        } else if !enabled {
            (
                text_sender_virtual_output_disabled.as_str().to_owned(),
                text_sender_enable_the_virtual_output_before_starting_to_send
                    .as_str()
                    .to_owned(),
                Tone::Warning,
            )
        } else {
            (
                self.tr(&Message::SenderReadyRoom {
                    name: (room).to_string(),
                }),
                text_sender_before_starting_select_this_virtual_device_as_the
                    .as_str()
                    .to_owned(),
                Tone::Accent,
            )
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
                let text = |ui: &mut egui::Ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.label(
                            RichText::new(&headline)
                                .font(theme::heading(19.0))
                                .color(theme::text()),
                        );
                        ui.label(RichText::new(&detail).color(theme::text_2()));
                    });
                };
                let action = |this: &mut Self, ui: &mut egui::Ui| {
                    if running {
                        if widgets::button_busy(
                            ui,
                            true,
                            this.pending_stop,
                            text_sender_stop_sending.as_str(),
                            Kind::Danger,
                        )
                        .clicked()
                        {
                            this.stop_sender();
                        }
                    } else if widgets::button_busy(
                        ui,
                        binding.is_some() && enabled && this.sender_allowed(),
                        this.pending("sender-start"),
                        text_sender_start_sending.as_str(),
                        Kind::Primary,
                    )
                    .on_disabled_hover_text(
                        text_sender_requires_pairing_playback_permission_and_an_enabled_virtual
                            .as_str(),
                    )
                    .clicked()
                    {
                        this.request(Request::SenderStart {
                            options: SenderOptions {
                                credential: this.credential.clone(),
                                hub: this.hub(),
                                output_binding: PathBuf::from("output"),
                            },
                        });
                    }
                };
                if wide {
                    ui.horizontal(|ui| {
                        let width = ui.available_width() - 150.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 48.0),
                            Layout::top_down(Align::Min),
                            text,
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| action(self, ui));
                    });
                } else {
                    text(ui);
                    ui.horizontal(|ui| action(self, ui));
                }
                ui.add_space(4.0);
                self.sender_pipeline(ui);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    widgets::metric(
                        ui,
                        text_sender_virtual_output.as_str(),
                        binding
                            .as_ref()
                            .and_then(|b| b["display_name"].as_str())
                            .unwrap_or(text_sender_not_added.as_str()),
                        None,
                    );
                    // Session maximum, not a live level: shown as a value only.
                    widgets::metric(
                        ui,
                        text_sender_capture_peak_session_maximum.as_str(),
                        &metrics
                            .as_ref()
                            .and_then(|m| m.pointer("/capture_stats/peak"))
                            .and_then(Value::as_f64)
                            .map_or(text_sender_unavailable.as_str().into(), |p| {
                                format!("{} dBFS", widgets::db_text(p))
                            }),
                        None,
                    );
                    widgets::metric(
                        ui,
                        text_sender_capture_frames_total.as_str(),
                        &metrics
                            .as_ref()
                            .and_then(|m| m.pointer("/capture_stats/frames"))
                            .map_or(text_sender_unavailable.as_str().into(), |v| {
                                self.value_text(v)
                            }),
                        None,
                    );
                });
                if let Some(process) = self
                    .status
                    .as_ref()
                    .map(|s| &s.sender)
                    .filter(|process| process.error.is_some() || process.fault.is_some())
                {
                    widgets::error_text(ui, self.tr(&process_error(process)));
                }
            },
        );
        let color = animation::color(ui.ctx(), ui.id().with("send-rail"), tone.color());
        widgets::lead_cap(ui, shown.response.rect, color);
    }

    fn pair_panel(&mut self, ui: &mut egui::Ui, paired: bool) {
        let text_sender_discovery_and_pairing = self.tr(&Message::SenderDiscoveryAndPairing);
        let text_sender_this_computer_is_paired_expand_to_change_rooms =
            self.tr(&Message::SenderThisComputerIsPairedExpandToChangeRooms);
        let text_sender_find_a_room_and_establish_trust_with_a =
            self.tr(&Message::SenderFindARoomAndEstablishTrustWithA);
        let text_sender_paired = self.tr(&Message::SenderPaired);
        let text_sender_discover_rooms_on_the_local_network =
            self.tr(&Message::SenderDiscoverRoomsOnTheLocalNetwork);
        let text_sender_make_sure_the_hub_is_sharing_on_the =
            self.tr(&Message::SenderMakeSureTheHubIsSharingOnThe);
        let text_sender_searching_for_rooms_on_the_local_network_via =
            self.tr(&Message::SenderSearchingForRoomsOnTheLocalNetworkVia);
        let text_sender_unnamed_room = self.tr(&Message::SenderUnnamedRoom);
        let text_sender_untrusted_identity = self.tr(&Message::SenderUntrustedIdentity);
        let text_sender_discovery_results_are_untrusted_the_invitation_s_pinned =
            self.tr(&Message::SenderDiscoveryResultsAreUntrustedTheInvitationSPinned);
        let text_sender_unavailable = self.tr(&Message::SenderUnavailable);
        let text_sender_device_name = self.tr(&Message::SenderDeviceName);
        let text_sender_paste_a_one_time_invitation_from_the_hub =
            self.tr(&Message::SenderPasteAOneTimeInvitationFromTheHub);
        let text_sender_hide = self.tr(&Message::SenderHide);
        let text_sender_show = self.tr(&Message::SenderShow);
        let text_sender_show_or_hide_invitation = self.tr(&Message::SenderShowOrHideInvitation);
        let text_sender_pair_with_room = self.tr(&Message::SenderPairWithRoom);
        let text_sender_forget_this_computer_s_pairing_label =
            self.tr(&Message::SenderForgetThisComputerSPairingLabel);
        let open = self.panel_open("pair", !paired);
        let panel = widgets::panel(
            ui,
            "pair",
            text_sender_discovery_and_pairing.as_str(),
            Some(if paired {
                text_sender_this_computer_is_paired_expand_to_change_rooms.as_str()
            } else {
                text_sender_find_a_room_and_establish_trust_with_a.as_str()
            }),
            open,
            |ui| {
                if paired {
                    widgets::pill(ui, text_sender_paired.as_str(), Tone::Success);
                }
            },
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if widgets::button_busy(
                        ui,
                        true,
                        self.pending("discover"),
                        text_sender_discover_rooms_on_the_local_network.as_str(),
                        Kind::Secondary,
                    )
                    .clicked()
                    {
                        self.request(Request::Discover { seconds: 3 });
                    }
                    if self.candidates.is_empty() && !self.pending("discover") {
                        widgets::note(ui, text_sender_make_sure_the_hub_is_sharing_on_the.as_str());
                    }
                });
                if self.pending("discover") || !self.candidates.is_empty() {
                    let found: Vec<u64> = self
                        .candidates
                        .iter()
                        .map(|c| {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            c.get("hub_id").and_then(Value::as_str).hash(&mut h);
                            h.finish()
                        })
                        .collect();
                    ui.horizontal(|ui| {
                        crate::viz::radar(ui, self.pending("discover"), &found, 96.0);
                        widgets::note(
                            ui,
                            if self.pending("discover") {
                                text_sender_searching_for_rooms_on_the_local_network_via
                                    .as_str()
                                    .to_owned()
                            } else {
                                self.tr(&Message::SenderDiscoveryCount {
                                    count: (found.len()) as u64,
                                })
                            },
                        );
                    });
                }
                for candidate in self.candidates.clone() {
                    widgets::inset(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(
                                    candidate
                                        .get("room_name")
                                        .or_else(|| candidate.get("name"))
                                        .and_then(Value::as_str)
                                        .unwrap_or(text_sender_unnamed_room.as_str()),
                                )
                                .font(theme::heading(theme::BODY))
                                .color(theme::text()),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let _ = widgets::pill(ui, text_sender_untrusted_identity.as_str(), Tone::Warning)
                                    .on_hover_text(
                                        text_sender_discovery_results_are_untrusted_the_invitation_s_pinned.as_str(),
                                    );
                            });
                        });
                        widgets::mono(
                            ui,
                            format!(
                                "Hub {}",
                                candidate
                                    .get("hub_id")
                                    .map_or(text_sender_unavailable.as_str().into(), |v| self
                                        .value_text(v))
                            ),
                        );
                    });
                }
                widgets::field(
                    ui,
                    "sender-device-name",
                    text_sender_device_name.as_str(),
                    &mut self.sender_name,
                    false,
                );
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 84.0).min(440.0);
                    widgets::field_sized(
                        ui,
                        "sender-paste-a-one-time-invitation-from-the-hub",
                        text_sender_paste_a_one_time_invitation_from_the_hub.as_str(),
                        &mut self.invitation,
                        !self.show_invitation,
                        width,
                    );
                    ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                        let label = if self.show_invitation {
                            text_sender_hide.as_str()
                        } else {
                            text_sender_show.as_str()
                        };
                        if widgets::button(ui, label, Kind::Secondary)
                            .on_hover_text(text_sender_show_or_hide_invitation.as_str())
                            .clicked()
                        {
                            self.show_invitation = !self.show_invitation;
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    if widgets::button_busy(
                        ui,
                        !self.invitation.trim().is_empty() && !self.sender_name.trim().is_empty(),
                        self.pending("pair"),
                        text_sender_pair_with_room.as_str(),
                        Kind::Primary,
                    )
                    .clicked()
                    {
                        self.request(Request::PairText {
                            invitation: self.invitation.trim().into(),
                            name: self.sender_name.trim().into(),
                            hub: self.manual_hub(),
                        });
                    }
                    if paired {
                        let forget = widgets::button(
                            ui,
                            text_sender_forget_this_computer_s_pairing_label.as_str(),
                            Kind::Quiet,
                        );
                        if forget.clicked() {
                            self.confirmation(
                                Message::SenderForgetThisComputerSPairing,
                                Message::SenderStopSendingDisableTheLocalOutputAndRemove,
                                Request::ForgetCredential {
                                    credential: "profiles/sender.json".into(),
                                },
                                forget.id,
                            );
                        }
                    }
                });
                self.connection_override(ui);
            },
        );
        if panel.toggled {
            self.toggle_panel("pair", open);
        }
        if self.scroll_to == Some("pair") {
            ui.scroll_to_rect(panel.rect, Some(Align::TOP));
            self.scroll_to = None;
        }
    }

    fn binding_panel(&mut self, ui: &mut egui::Ui) {
        let text_sender_virtual_output = self.tr(&Message::SenderVirtualOutput);
        let text_sender_a_neonmix_device_available_in_system_sound_settings =
            self.tr(&Message::SenderANeonmixDeviceAvailableInSystemSoundSettings);
        let text_sender_enabled = self.tr(&Message::SenderEnabled);
        let text_sender_disabled = self.tr(&Message::SenderDisabled);
        let text_sender_not_added = self.tr(&Message::SenderNotAdded);
        let text_sender_virtual_output_name = self.tr(&Message::SenderVirtualOutputName);
        let text_sender_virtual_output_provider = self.tr(&Message::SenderVirtualOutputProvider);
        let text_sender_blackhole_external_provider =
            self.tr(&Message::SenderBlackholeExternalProvider);
        let text_sender_neonmix_virtual_output = self.tr(&Message::SenderNeonmixVirtualOutput);
        let text_sender_add_virtual_output = self.tr(&Message::SenderAddVirtualOutput);
        let text_sender_requires_a_connected_paired_sender_identity =
            self.tr(&Message::SenderRequiresAConnectedPairedSenderIdentity);
        let text_sender_load_existing_output = self.tr(&Message::SenderLoadExistingOutput);
        let text_sender_requires_a_paired_sender_identity_and_an_installed =
            self.tr(&Message::SenderRequiresAPairedSenderIdentityAndAnInstalled);
        let text_sender_unavailable = self.tr(&Message::SenderUnavailable);
        let text_sender_save_output_name = self.tr(&Message::SenderSaveOutputName);
        let text_sender_disable_output = self.tr(&Message::SenderDisableOutput);
        let text_sender_enable_output = self.tr(&Message::SenderEnableOutput);
        let text_sender_remove_binding = self.tr(&Message::SenderRemoveBinding);
        let binding = self.binding.clone();
        let enabled = binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        let open = self.panel_open("binding", true);
        let panel = widgets::panel(
            ui,
            "binding",
            text_sender_virtual_output.as_str(),
            Some(text_sender_a_neonmix_device_available_in_system_sound_settings.as_str()),
            open,
            |ui| match &binding {
                Some(_) if enabled => {
                    widgets::pill(ui, text_sender_enabled.as_str(), Tone::Success);
                }
                Some(_) => {
                    widgets::pill(ui, text_sender_disabled.as_str(), Tone::Warning);
                }
                None => {
                    widgets::pill(ui, text_sender_not_added.as_str(), Tone::Neutral);
                }
            },
            |ui| {
                widgets::field(
                    ui,
                    "sender-virtual-output-name",
                    text_sender_virtual_output_name.as_str(),
                    &mut self.binding_name,
                    false,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let label = widgets::caption(ui, text_sender_virtual_output_provider.as_str());
                    let response = egui::ComboBox::from_id_salt("provider")
                        .width((ui.available_width() - 4.0).min(300.0))
                        .selected_text(if self.provider == "blackhole" {
                            text_sender_blackhole_external_provider.as_str()
                        } else {
                            text_sender_neonmix_virtual_output.as_str()
                        })
                        .show_ui(ui, |ui| {
                            widgets::select_value(
                                ui,
                                &mut self.provider,
                                "neonmix".into(),
                                text_sender_neonmix_virtual_output.as_str(),
                            );
                            if cfg!(target_os = "macos") {
                                widgets::select_value(
                                    ui,
                                    &mut self.provider,
                                    "blackhole".into(),
                                    text_sender_blackhole_external_provider.as_str(),
                                );
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    widgets::label_combo(&response, text_sender_virtual_output_provider.as_str());
                });
                let Some(binding) = binding.clone() else {
                    ui.horizontal_wrapped(|ui| {
                        if widgets::button_busy(
                            ui,
                            self.writable()
                                && self.role().is_some()
                                && self.credential.as_path()
                                    != std::path::Path::new("hub/admin.json")
                                && !self.binding_name.trim().is_empty(),
                            self.pending("output"),
                            text_sender_add_virtual_output.as_str(),
                            Kind::Primary,
                        )
                        .on_disabled_hover_text(
                            text_sender_requires_a_connected_paired_sender_identity.as_str(),
                        )
                        .clicked()
                        {
                            self.request(Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Add {
                                    credential: self.credential.clone(),
                                    hub: self.hub(),
                                    name: self.binding_name.clone(),
                                    provider: self.provider.clone(),
                                    device: None,
                                },
                            });
                        }
                        if widgets::button(
                            ui,
                            text_sender_load_existing_output.as_str(),
                            Kind::Secondary,
                        )
                        .clicked()
                        {
                            self.request(Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Show,
                            });
                        }
                    });
                    widgets::note(
                        ui,
                        text_sender_requires_a_paired_sender_identity_and_an_installed.as_str(),
                    );
                    return;
                };
                let revision = binding.get("revision").and_then(Value::as_u64).unwrap_or(0);
                let Some(expected_output_id) = binding
                    .get("output_id")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .filter(|id| !id.is_nil())
                else {
                    widgets::error_text(ui, self.tr(&Message::FaultUpgradeRequired));
                    return;
                };
                widgets::inset(ui, |ui| {
                    ui.label(
                        RichText::new(
                            binding
                                .get("display_name")
                                .and_then(Value::as_str)
                                .unwrap_or(text_sender_virtual_output.as_str()),
                        )
                        .font(theme::heading(theme::BODY))
                        .color(theme::text()),
                    );
                    widgets::mono(
                        ui,
                        self.tr(&Message::SenderBindingIdentities {
                            device: (binding
                                .get("device_id")
                                .map_or(text_sender_unavailable.as_str().into(), |v| {
                                    self.value_text(v)
                                }))
                            .to_string(),
                            room: (binding
                                .get("hub_id")
                                .map_or(text_sender_unavailable.as_str().into(), |v| {
                                    self.value_text(v)
                                }))
                            .to_string(),
                        }),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, text_sender_save_output_name.as_str(), Kind::Secondary)
                        .clicked()
                    {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: OutputAction::Rename {
                                expected_revision: revision,
                                expected_output_id,
                                name: self.binding_name.clone(),
                            },
                        });
                    }
                    let toggle = if enabled {
                        text_sender_disable_output.as_str()
                    } else {
                        text_sender_enable_output.as_str()
                    };
                    if widgets::button(ui, toggle, Kind::Secondary).clicked() {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: if enabled {
                                OutputAction::Disable {
                                    expected_revision: revision,
                                    expected_output_id,
                                }
                            } else {
                                OutputAction::Enable {
                                    expected_revision: revision,
                                    expected_output_id,
                                }
                            },
                        });
                    }
                    let remove =
                        widgets::button(ui, text_sender_remove_binding.as_str(), Kind::Quiet);
                    if remove.clicked() {
                        self.confirmation(
                            Message::SenderRemoveOutputBinding,
                            Message::SenderStopSendingFromThisBindingAndKeepThe,
                            Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Remove {
                                    expected_revision: revision,
                                    expected_output_id,
                                },
                            },
                            remove.id,
                        );
                    }
                });
            },
        );
        if panel.toggled {
            self.toggle_panel("binding", open);
        }
        if self.scroll_to == Some("binding") {
            ui.scroll_to_rect(panel.rect, Some(Align::TOP));
            self.scroll_to = None;
        }
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;
    #[test]
    fn switching_control_room_and_identity_never_retargets_running_sender() {
        let mut app = Desktop::empty(Client::new(".local/test-sender-target"), true, false);
        let mut authority =
            neonmix_control::Authority::new("output".into(), "admin".into(), &"a".repeat(32))
                .unwrap();
        let target_hub = authority.current().hub_id;
        let target_device = *authority.current().devices.keys().next().unwrap();
        app.snapshot = Some(authority.snapshot());
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../docs/evidence/ui-rebuild-20261004/fixtures/full.json"
        ))
        .unwrap();
        app.status = Some(serde_json::from_value(fixture["status"].clone()).unwrap());
        app.status.as_mut().unwrap().sender = neonmix_desktop_service::ProcessStatus {
            running: true,
            ready: true,
            pid: Some(2),
            sender_target: Some(neonmix_desktop_service::SenderTarget {
                hub_id: target_hub,
                device_id: Some(target_device),
                room_name: Some("Send A".into()),
            }),
            ..Default::default()
        };
        app.remote_room = Some("Control A".into());
        assert_eq!(app.sender_room_name(), "Send A");
        assert_eq!(app.own_device().unwrap().id, target_device);
        app.credential = "profiles/another-controller.json".into();
        app.remote_room = Some("Control B".into());
        authority = neonmix_control::Authority::new(
            "other-output".into(),
            "other-admin".into(),
            &"b".repeat(32),
        )
        .unwrap();
        app.snapshot = Some(authority.snapshot());
        assert_eq!(app.sender_room_name(), "Send A");
        assert!(app.own_device().is_none());
        assert!(app.sender_snapshot().is_none());
        app.status.as_mut().unwrap().sender.sender_target = None;
        assert_eq!(
            app.sender_room_name(),
            app.tr(&Message::SenderTargetConfirming)
        );
        assert!(app.sender_snapshot().is_none());
    }
}
