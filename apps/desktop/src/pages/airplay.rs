//! Receiver setup and source controls use explicit v2 identities throughout.
use super::*;
use crate::widgets::{Kind, Tone};
use neonmix_airplay_adapter::control::{AirplayActionV2 as AirplayAction, PlaybackMode};

impl Desktop {
    pub(crate) fn airplay_sessions(&self) -> Vec<Value> {
        let Some(state) = self.airplay.as_ref() else {
            return Vec::new();
        };
        state["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|session| {
                let mut session = session.clone();
                if let Some(alias) = state["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|source| source["source_id"] == session["source_id"])
                    .and_then(|source| source["alias"].as_str())
                    .filter(|name| !name.trim().is_empty())
                {
                    session["source_name"] = Value::String(alias.into());
                }
                session
            })
            .collect()
    }
    pub(crate) fn airplay_sources(&self) -> Vec<Value> {
        let text_hub_airplay_source = self.tr(&Message::HubAirplaySource);
        let sessions = self.airplay_sessions();
        self.airplay
            .as_ref()
            .and_then(|s| s["sources"].as_array())
            .into_iter()
            .flatten()
            .map(|source| {
                let mut source = source.clone();
                let session = sessions
                    .iter()
                    .find(|s| s["source_id"] == source["source_id"]);
                source["active"] = Value::Bool(session.is_some());
                source["source_revoked"] = source["revoked"].clone();
                source["playback_allowed"] = Value::Bool(source["blocked"].as_bool() != Some(true));
                source["source_name"] = source
                    .get("alias")
                    .filter(|v| v.is_string())
                    .or_else(|| source.get("last_name").filter(|v| v.is_string()))
                    .cloned()
                    .or_else(|| session.map(|s| s["source_name"].clone()))
                    .unwrap_or(Value::String(text_hub_airplay_source.as_str().into()));
                if let Some(session) = session {
                    source["session_id"] = session["session_id"].clone();
                    source["receiver_id"] = session["receiver_id"].clone();
                }
                source
            })
            .collect()
    }
    pub(crate) fn airplay_request(&self, operation: AirplayAction) -> Option<Request> {
        if !(self.ready() || self.writable()) || !self.admin() {
            return None;
        }
        let target = match &operation {
            AirplayAction::MixSource {
                source_id,
                session_id,
                ..
            }
            | AirplayAction::PatchMixSource {
                source_id,
                session_id,
                ..
            }
            | AirplayAction::DisconnectSource {
                source_id,
                session_id,
            } => Some((source_id, *session_id)),
            AirplayAction::RevokeSource {
                source_id,
                session_id: Some(session_id),
            } => Some((source_id, *session_id)),
            _ => None,
        };
        if let Some((source, id)) = target
            && !self.airplay_sessions().iter().any(|s| {
                s["source_id"].as_str() == Some(source.as_str())
                    && s["session_id"].as_u64() == Some(id)
            })
        {
            return None;
        }
        Some(Request::AirplayV2 {
            credential: self.credential.clone(),
            hub: self.hub(),
            command: Some(self.bind_airplay(
                uuid::Uuid::new_v4(),
                self.intents.current.as_ref()?,
                operation,
            )?),
        })
    }
    pub(crate) fn airplay_operation(&mut self, operation: AirplayAction) {
        self.write(Write::Airplay(operation));
    }
    pub(crate) fn airplay_panel(&mut self, ui: &mut egui::Ui) {
        let text_hub_airplay_receiver_entries = self.tr(&Message::HubAirplayReceiverEntries);
        let text_hub_each_device_uses_a_free_entry_occupied_entries =
            self.tr(&Message::HubEachDeviceUsesAFreeEntryOccupiedEntries);
        let text_hub_on = self.tr(&Message::HubOn);
        let text_hub_off = self.tr(&Message::HubOff);
        let text_hub_connect_to_a_room_to_view_and_enable =
            self.tr(&Message::HubConnectToARoomToViewAndEnable);
        let text_hub_room_input_capacity = self.tr(&Message::HubRoomInputCapacity);
        let text_hub_turn_off_airplay_reception = self.tr(&Message::HubTurnOffAirplayReception);
        let text_hub_turn_on_airplay_reception = self.tr(&Message::HubTurnOnAirplayReception);
        let text_hub_receiver_entry_count = self.tr(&Message::HubReceiverEntryCount);
        let text_hub_adding_entries_keeps_existing_sources_connected_disconnect_sources =
            self.tr(&Message::HubAddingEntriesKeepsExistingSourcesConnectedDisconnectSources);
        let text_hub_airplay_entry = self.tr(&Message::HubAirplayEntry);
        let text_hub_entry_fault = self.tr(&Message::HubEntryFault);
        let text_hub_connected = self.tr(&Message::HubConnected);
        let text_hub_waiting_for_connection = self.tr(&Message::HubWaitingForConnection);
        let text_hub_preparing = self.tr(&Message::HubPreparing);
        let text_hub_publicly_discoverable = self.tr(&Message::HubPubliclyDiscoverable);
        let text_hub_hidden_from_discovery = self.tr(&Message::HubHiddenFromDiscovery);
        let text_hub_discovery_fault = self.tr(&Message::HubDiscoveryFault);
        let text_hub_updating_discovery = self.tr(&Message::HubUpdatingDiscovery);
        let text_hub_turn_off_this_entry = self.tr(&Message::HubTurnOffThisEntry);
        let text_hub_turn_on_this_entry = self.tr(&Message::HubTurnOnThisEntry);
        let text_hub_open_pairing_window = self.tr(&Message::HubOpenPairingWindow);
        let text_hub_pairing_code_for_this_entry = self.tr(&Message::HubPairingCodeForThisEntry);
        let text_hub_hide_pairing_code = self.tr(&Message::HubHidePairingCode);
        let text_hub_show_pairing_code = self.tr(&Message::HubShowPairingCode);
        let text_hub_room_administrators_manage_reception_and_pairing =
            self.tr(&Message::HubRoomAdministratorsManageReceptionAndPairing);
        let state = self.airplay.clone();
        let open = self.panel_open("airplay", true);
        let panel = widgets::panel(
            ui,
            "airplay",
            text_hub_airplay_receiver_entries.as_str(),
            Some(text_hub_each_device_uses_a_free_entry_occupied_entries.as_str()),
            open,
            |ui| {
                let enabled = state
                    .as_ref()
                    .is_some_and(|s| s["enabled"].as_bool() == Some(true));
                widgets::pill(
                    ui,
                    if enabled {
                        text_hub_on.as_str()
                    } else {
                        text_hub_off.as_str()
                    },
                    if enabled {
                        Tone::Success
                    } else {
                        Tone::Neutral
                    },
                );
            },
            |ui| {
                let Some(state) = state.as_ref() else {
                    widgets::note(ui, text_hub_connect_to_a_room_to_view_and_enable.as_str());
                    return;
                };
                let enabled = state["enabled"].as_bool() == Some(true);
                let receivers = state["receivers"].as_array().cloned().unwrap_or_default();
                let count = receivers
                    .iter()
                    .filter(|r| r["configured_enabled"].as_bool().unwrap_or(true))
                    .count();
                let multi = state["multi_receiver"].as_bool() == Some(true);
                ui.horizontal_wrapped(|ui| {
                    let limit = state
                        .pointer("/capacity/limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(4) as usize;
                    let used: Vec<crate::viz::Slot> = self
                        .snapshot
                        .clone()
                        .map(|s| self.lanes(&s))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|l| crate::viz::Slot {
                            airplay: l.is_airplay(),
                            name: l.name,
                        })
                        .collect();
                    widgets::caption(ui, text_hub_room_input_capacity.as_str());
                    crate::viz::capacity_slots(ui, limit, &used);
                    if self.admin()
                        && widgets::button_enabled(
                            ui,
                            self.writable() && (enabled || count > 0),
                            if enabled {
                                text_hub_turn_off_airplay_reception.as_str()
                            } else {
                                text_hub_turn_on_airplay_reception.as_str()
                            },
                            Kind::Primary,
                        )
                        .clicked()
                    {
                        self.airplay_operation(if enabled {
                            AirplayAction::Disable
                        } else {
                            AirplayAction::Enable
                        });
                    }
                });
                if self.admin() {
                    ui.horizontal_wrapped(|ui| {
                        widgets::caption(ui, text_hub_receiver_entry_count.as_str());
                        for n in 1..=4 {
                            if widgets::toggle(
                                ui,
                                self.writable(),
                                n == count,
                                &n.to_string(),
                                Tone::Accent,
                            )
                            .clicked()
                                && (n != count || !multi)
                            {
                                self.airplay_operation(AirplayAction::Configure {
                                    receiver_count: n,
                                    multi_receiver: true,
                                });
                            }
                        }
                    });
                    if enabled {
                        widgets::note(
                            ui,
                            text_hub_adding_entries_keeps_existing_sources_connected_disconnect_sources.as_str(),
                        );
                    }
                }
                for (receiver_index, receiver) in receivers.into_iter().enumerate() {
                    let Some(id) = receiver["receiver_id"].as_str() else {
                        continue;
                    };
                    ui.push_id(id, |ui| {
                        widgets::inset(ui, |ui| {
                            let active = receiver["active"].as_bool() == Some(true);
                            let ready = receiver["ready"].as_bool() == Some(true);
                            let on = receiver["enabled"].as_bool() == Some(true);
                            let can_enable = multi || receiver_index == 0;
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(
                                        receiver["name"]
                                            .as_str()
                                            .unwrap_or(text_hub_airplay_entry.as_str()),
                                    )
                                    .color(theme::TEXT),
                                );
                                let (label, tone) = if receiver["error"].is_string() {
                                    (text_hub_entry_fault.as_str(), Tone::Danger)
                                } else if active {
                                    (text_hub_connected.as_str(), Tone::Success)
                                } else if ready {
                                    (text_hub_waiting_for_connection.as_str(), Tone::Accent)
                                } else if on {
                                    (text_hub_preparing.as_str(), Tone::Neutral)
                                } else {
                                    (text_hub_off.as_str(), Tone::Neutral)
                                };
                                widgets::pill(ui, label, tone);
                                let discovery =
                                    receiver["discovery_state"].as_str().unwrap_or("hidden");
                                widgets::pill(
                                    ui,
                                    match discovery {
                                        "published" => text_hub_publicly_discoverable.as_str(),
                                        "hidden" => text_hub_hidden_from_discovery.as_str(),
                                        "error" => text_hub_discovery_fault.as_str(),
                                        _ => text_hub_updating_discovery.as_str(),
                                    },
                                    Tone::Neutral,
                                );
                            });
                            if self.admin() {
                                ui.horizontal_wrapped(|ui| {
                                    if widgets::small_button(
                                        ui,
                                        self.writable() && (on || can_enable),
                                        if on {
                                            text_hub_turn_off_this_entry.as_str()
                                        } else {
                                            text_hub_turn_on_this_entry.as_str()
                                        },
                                    )
                                    .clicked()
                                    {
                                        self.airplay_operation(if on {
                                            AirplayAction::DisableReceiver {
                                                receiver_id: id.into(),
                                            }
                                        } else {
                                            AirplayAction::EnableReceiver {
                                                receiver_id: id.into(),
                                            }
                                        });
                                    }
                                    if widgets::small_button(
                                        ui,
                                        self.writable() && ready && !active,
                                        text_hub_open_pairing_window.as_str(),
                                    )
                                    .clicked()
                                    {
                                        self.airplay_operation(AirplayAction::PairReceiver {
                                            receiver_id: id.into(),
                                        });
                                    }
                                });
                                self.airplay_name_editor(
                                    ui,
                                    id,
                                    receiver["name"].as_str().unwrap_or(""),
                                    true,
                                    !on && !active,
                                );
                                if let Some(pin) = receiver["pairing_pin"].as_str() {
                                    ui.horizontal_wrapped(|ui| {
                                        let label = widgets::caption(
                                            ui,
                                            text_hub_pairing_code_for_this_entry.as_str(),
                                        );
                                        let mut pin = pin.to_owned();
                                        ui.add(
                                            egui::TextEdit::singleline(&mut pin)
                                                .password(!self.show_airplay_pin)
                                                .interactive(false)
                                                .desired_width(100.0),
                                        )
                                        .labelled_by(label.id);
                                        if widgets::small_button(
                                            ui,
                                            true,
                                            if self.show_airplay_pin {
                                                text_hub_hide_pairing_code.as_str()
                                            } else {
                                                text_hub_show_pairing_code.as_str()
                                            },
                                        )
                                        .clicked()
                                        {
                                            self.show_airplay_pin = !self.show_airplay_pin;
                                        }
                                    });
                                }
                            }
                        })
                    });
                }
                if !self.admin() {
                    widgets::note(
                        ui,
                        text_hub_room_administrators_manage_reception_and_pairing.as_str(),
                    );
                }
            },
        );
        if panel.toggled {
            self.toggle_panel("airplay", open);
        }
    }
    pub(crate) fn airplay_device_row(&mut self, ui: &mut egui::Ui, source: &Value) {
        let text_hub_airplay_source = self.tr(&Message::HubAirplaySource);
        let text_hub_pairing_revoked = self.tr(&Message::HubPairingRevoked);
        let text_hub_playback_blocked = self.tr(&Message::HubPlaybackBlocked);
        let text_hub_connected = self.tr(&Message::HubConnected);
        let text_hub_disconnected = self.tr(&Message::HubDisconnected);
        let text_hub_disconnect_airplay_source = self.tr(&Message::HubDisconnectAirplaySource);
        let text_hub_allow_playback_again = self.tr(&Message::HubAllowPlaybackAgain);
        let text_hub_enable_a_free_entry_in_hub_settings_then =
            self.tr(&Message::HubEnableAFreeEntryInHubSettingsThen);
        let text_hub_entry = self.tr(&Message::HubEntry);
        let text_hub_revoke_airplay_pairing = self.tr(&Message::HubRevokeAirplayPairing);
        let text_hub_playback_mode = self.tr(&Message::HubPlaybackMode);
        let text_hub_low_latency = self.tr(&Message::HubLowLatency);
        let text_hub_audio_video_sync = self.tr(&Message::HubAudioVideoSync);
        let text_hub_disconnect_this_source_to_change_its_playback_mode =
            self.tr(&Message::HubDisconnectThisSourceToChangeItsPlaybackMode);
        let Some(id) = source["source_id"].as_str() else {
            return;
        };
        let active = source["active"].as_bool() == Some(true);
        let revoked = source["revoked"].as_bool() == Some(true);
        let blocked = source["blocked"].as_bool() == Some(true);
        let name = source["source_name"]
            .as_str()
            .unwrap_or(text_hub_airplay_source.as_str());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(name)
                    .font(theme::heading(theme::BODY))
                    .color(theme::TEXT),
            );
            widgets::pill(ui, "AirPlay", Tone::Neutral);
            let (label, tone) = if revoked {
                (text_hub_pairing_revoked.as_str(), Tone::Danger)
            } else if blocked {
                (text_hub_playback_blocked.as_str(), Tone::Warning)
            } else if active {
                (text_hub_connected.as_str(), Tone::Success)
            } else {
                (text_hub_disconnected.as_str(), Tone::Neutral)
            };
            widgets::pill(ui, label, tone);
            if let Some(receiver) = self
                .airplay
                .as_ref()
                .and_then(|s| s["receivers"].as_array())
                .into_iter()
                .flatten()
                .find(|r| r["receiver_id"] == source["receiver_id"])
            {
                widgets::caption(ui, receiver["name"].as_str().unwrap_or(""));
            }
        });
        widgets::mono(ui, format!("ID {id}"));
        if !self.admin() {
            return;
        }
        self.airplay_name_editor(ui, id, source["alias"].as_str().unwrap_or(""), false, true);
        ui.add_enabled_ui(self.writable(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if active
                    && let Some(session_id) = source["session_id"].as_u64()
                    && widgets::button(
                        ui,
                        text_hub_disconnect_airplay_source.as_str(),
                        Kind::Secondary,
                    )
                    .clicked()
                {
                    self.airplay_operation(AirplayAction::DisconnectSource {
                        source_id: id.into(),
                        session_id,
                    });
                }
                if blocked
                    && !revoked
                    && widgets::button(ui, text_hub_allow_playback_again.as_str(), Kind::Secondary)
                        .clicked()
                {
                    self.airplay_operation(AirplayAction::AllowSource {
                        source_id: id.into(),
                    });
                }
                if revoked {
                    let receivers = self
                        .airplay
                        .as_ref()
                        .and_then(|s| s["receivers"].as_array())
                        .cloned()
                        .unwrap_or_default();
                    if !receivers.iter().any(|r| {
                        r["active"].as_bool() != Some(true)
                            && r["configured_enabled"].as_bool().unwrap_or(true)
                    }) {
                        widgets::note(
                            ui,
                            text_hub_enable_a_free_entry_in_hub_settings_then.as_str(),
                        );
                    }
                    for receiver in receivers.iter().filter(|r| {
                        r["active"].as_bool() != Some(true)
                            && r["configured_enabled"].as_bool().unwrap_or(true)
                    }) {
                        let Some(receiver_id) = receiver["receiver_id"].as_str() else {
                            continue;
                        };
                        let receiver_name =
                            receiver["name"].as_str().unwrap_or(text_hub_entry.as_str());
                        let repair = widgets::button(
                            ui,
                            &self.tr(&Message::HubAirplayRepairEntry {
                                name: (receiver_name).to_string(),
                            }),
                            Kind::Secondary,
                        );
                        if repair.clicked()
                            && let Some(request) =
                                self.airplay_request(AirplayAction::RepairSource {
                                    source_id: id.into(),
                                    receiver_id: receiver_id.into(),
                                })
                        {
                            self.confirmation(
                                Message::HubAirplayRepairTitle {
                                    name: (name).to_string(),
                                },
                                Message::HubAirplayRepairConsequence {
                                    name: (receiver_name).to_string(),
                                },
                                request,
                                repair.id,
                            );
                        }
                    }
                }
                if !revoked {
                    let revoke =
                        widgets::button(ui, text_hub_revoke_airplay_pairing.as_str(), Kind::Quiet);
                    if revoke.clicked()
                        && let Some(request) = self.airplay_request(AirplayAction::RevokeSource {
                            source_id: id.into(),
                            session_id: source["session_id"].as_u64(),
                        })
                    {
                        self.confirmation(
                            Message::HubAirplayRevokeTitle {
                                name: (name).to_string(),
                            },
                            Message::HubEndThisDeviceSAudioAndRevokeIts,
                            request,
                            revoke.id,
                        );
                    }
                }
            });
            let mode = if source["playback_mode"].as_str() == Some("synchronized") {
                PlaybackMode::Synchronized
            } else {
                PlaybackMode::LowLatency
            };
            ui.horizontal_wrapped(|ui| {
                widgets::caption(ui, text_hub_playback_mode.as_str());
                for (value, label) in [
                    (PlaybackMode::LowLatency, text_hub_low_latency.as_str()),
                    (
                        PlaybackMode::Synchronized,
                        text_hub_audio_video_sync.as_str(),
                    ),
                ] {
                    if widgets::toggle(ui, !active && !revoked, mode == value, label, Tone::Accent)
                        .clicked()
                        && mode != value
                    {
                        self.airplay_operation(AirplayAction::PlaybackMode {
                            source_id: id.into(),
                            mode: value,
                        });
                    }
                }
            });
            if active {
                widgets::note(
                    ui,
                    text_hub_disconnect_this_source_to_change_its_playback_mode.as_str(),
                );
            }
        });
    }
    /// Drafts survive polling, conflicts and failed writes. Only the matching
    /// successful reply clears a draft; a later edit is never discarded.
    pub(crate) fn airplay_name_saved(&mut self, action: &AirplayAction) {
        let target = match action {
            AirplayAction::RenameReceiver { receiver_id, name } => {
                Some((format!("receiver:{receiver_id}"), name.as_str()))
            }
            AirplayAction::AliasSource { source_id, alias } => Some((
                format!("source:{source_id}"),
                alias.as_deref().unwrap_or(""),
            )),
            _ => None,
        };
        if let Some((key, saved)) = target
            && self
                .airplay_name_drafts
                .get(&key)
                .is_some_and(|draft| draft == saved)
        {
            self.airplay_name_drafts.remove(&key);
        }
    }
    fn airplay_name_editor(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        current: &str,
        receiver: bool,
        available: bool,
    ) {
        let text_hub_edit_entry_name = self.tr(&Message::HubEditEntryName);
        let text_hub_set_alias = self.tr(&Message::HubSetAlias);
        let text_hub_edit_alias = self.tr(&Message::HubEditAlias);
        let text_hub_turn_off_this_entry_to_edit_its_name =
            self.tr(&Message::HubTurnOffThisEntryToEditItsName);
        let text_hub_waiting_for_a_room_connection =
            self.tr(&Message::HubWaitingForARoomConnection);
        let text_hub_entry_name = self.tr(&Message::HubEntryName);
        let text_hub_source_alias = self.tr(&Message::HubSourceAlias);
        let text_hub_enter_a_name_without_control_characters_up_to =
            self.tr(&Message::HubEnterANameWithoutControlCharactersUpTo);
        let text_hub_enter_an_alias_without_control_characters_up_to =
            self.tr(&Message::HubEnterAnAliasWithoutControlCharactersUpTo);
        let text_hub_save = self.tr(&Message::HubSave);
        let text_hub_clear_alias = self.tr(&Message::HubClearAlias);
        let text_hub_cancel = self.tr(&Message::HubCancel);
        let text_hub_applies_on_this_entry_s_next_start_receiver =
            self.tr(&Message::HubAppliesOnThisEntrySNextStartReceiver);
        let text_hub_aliases_are_only_used_for_room_display_they =
            self.tr(&Message::HubAliasesAreOnlyUsedForRoomDisplayThey);
        let key = format!("{}:{id}", if receiver { "receiver" } else { "source" });
        if !self.airplay_name_drafts.contains_key(&key) {
            if widgets::small_button(
                ui,
                self.writable() && available,
                if receiver {
                    text_hub_edit_entry_name.as_str()
                } else if current.is_empty() {
                    text_hub_set_alias.as_str()
                } else {
                    text_hub_edit_alias.as_str()
                },
            )
            .on_disabled_hover_text(if receiver {
                text_hub_turn_off_this_entry_to_edit_its_name.as_str()
            } else {
                text_hub_waiting_for_a_room_connection.as_str()
            })
            .clicked()
            {
                self.airplay_name_drafts.insert(key.clone(), current.into());
            }
            return;
        }
        let mut draft = self.airplay_name_drafts[&key].clone();
        ui.push_id(&key, |ui| {
            widgets::field_sized(
                ui,
                "airplay-name",
                if receiver {
                    text_hub_entry_name.as_str()
                } else {
                    text_hub_source_alias.as_str()
                },
                &mut draft,
                false,
                ui.available_width().min(320.0),
            );
            let valid = airplay_name_valid(&draft, if receiver { 50 } else { 128 });
            if !valid {
                widgets::error_text(
                    ui,
                    if receiver {
                        text_hub_enter_a_name_without_control_characters_up_to.as_str()
                    } else {
                        text_hub_enter_an_alias_without_control_characters_up_to.as_str()
                    },
                );
            }
            ui.horizontal_wrapped(|ui| {
                if widgets::small_button(
                    ui,
                    self.writable() && available && valid && draft != current,
                    text_hub_save.as_str(),
                )
                .clicked()
                {
                    self.airplay_operation(if receiver {
                        AirplayAction::RenameReceiver {
                            receiver_id: id.into(),
                            name: draft.clone(),
                        }
                    } else {
                        AirplayAction::AliasSource {
                            source_id: id.into(),
                            alias: Some(draft.clone()),
                        }
                    });
                }
                if !receiver
                    && !current.is_empty()
                    && widgets::small_button(ui, self.writable(), text_hub_clear_alias.as_str())
                        .clicked()
                {
                    draft.clear();
                    self.airplay_operation(AirplayAction::AliasSource {
                        source_id: id.into(),
                        alias: None,
                    });
                }
                if widgets::small_button(ui, true, text_hub_cancel.as_str()).clicked() {
                    self.airplay_name_drafts.remove(&key);
                }
            });
            if receiver {
                widgets::note(
                    ui,
                    text_hub_applies_on_this_entry_s_next_start_receiver.as_str(),
                );
            } else {
                widgets::note(
                    ui,
                    text_hub_aliases_are_only_used_for_room_display_they.as_str(),
                );
            }
        });
        if self.airplay_name_drafts.contains_key(&key) {
            self.airplay_name_drafts.insert(key, draft);
        }
    }
}

fn airplay_name_valid(name: &str, limit: usize) -> bool {
    !name.trim().is_empty() && name.len() <= limit && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod name_tests {
    use super::*;
    #[test]
    fn names_use_utf8_byte_bounds_and_success_does_not_erase_newer_drafts() {
        assert!(airplay_name_valid(&"源".repeat(16), 50));
        assert!(!airplay_name_valid(&"源".repeat(17), 50));
        assert!(airplay_name_valid(&"名".repeat(42), 128));
        assert!(!airplay_name_valid(&"名".repeat(43), 128));
        for name in ["", "  ", "name\nnext", "name\t"] {
            assert!(!airplay_name_valid(name, 128));
        }
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.airplay_name_drafts
            .insert("source:source-a".into(), "新草稿".into());
        app.airplay_name_saved(&AirplayAction::AliasSource {
            source_id: "source-a".into(),
            alias: Some("旧回复".into()),
        });
        assert_eq!(app.airplay_name_drafts["source:source-a"], "新草稿");
        app.airplay_name_saved(&AirplayAction::AliasSource {
            source_id: "source-a".into(),
            alias: Some("新草稿".into()),
        });
        assert!(app.airplay_name_drafts.is_empty());
        app.airplay_name_drafts
            .insert("receiver:entry-a".into(), "入口名称".into());
        app.airplay_name_saved(&AirplayAction::RenameReceiver {
            receiver_id: "entry-b".into(),
            name: "入口名称".into(),
        });
        assert!(app.airplay_name_drafts.contains_key("receiver:entry-a"));
        app.airplay_name_drafts
            .insert("source:source-a".into(), String::new());
        app.airplay_name_saved(&AirplayAction::AliasSource {
            source_id: "source-a".into(),
            alias: None,
        });
        assert!(!app.airplay_name_drafts.contains_key("source:source-a"));
    }
    #[test]
    fn alias_display_never_changes_session_control_identity() {
        let mut app = Desktop::empty(Client::new(".local/test-desktop"), true, false);
        app.airplay = Some(
            serde_json::json!({"sources":[{"source_id":"source-a","last_name":"原始名称","alias":"演示手机"}],"sessions":[{"source_id":"source-a","source_name":"原始名称","session_id":7,"stream_id":9}]}),
        );
        let sessions = app.airplay_sessions();
        assert_eq!(sessions[0]["source_name"], "演示手机");
        assert_eq!(sessions[0]["session_id"], 7);
        assert_eq!(sessions[0]["stream_id"], 9);
        assert_eq!(app.airplay_sources()[0]["source_name"], "演示手机");
        app.airplay.as_mut().unwrap()["sources"][0]["alias"] = Value::Null;
        assert_eq!(app.airplay_sessions()[0]["source_name"], "原始名称");
    }
}
