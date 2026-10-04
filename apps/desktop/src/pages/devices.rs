//! 设备管理: search plus state filters over one list of room members (and the
//! AirPlay source). Each row: who (avatar, name, role), state, actions.
use super::*;
use crate::widgets::{Kind, Tone};
use egui::{Align, Layout, Margin};

const FILTERS: [&str; 4] = ["全部", "发送中", "已断开", "已撤销"];

impl Desktop {
    pub(crate) fn devices_page(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot.clone();
        let airplay = self.airplay_sources();
        let streams_of = |id: uuid::Uuid| {
            snapshot.as_ref().map_or(0, |s| {
                s.streams.values().filter(|x| x.device_id == id).count()
            })
        };
        let devices: Vec<neonmix_control::Device> = snapshot
            .as_ref()
            .map(|s| s.devices.values().cloned().collect())
            .unwrap_or_default();
        let counts = [
            devices.len() + airplay.len(),
            devices
                .iter()
                .filter(|d| !d.revoked && d.playback_allowed && streams_of(d.id) > 0)
                .count()
                + airplay
                    .iter()
                    .filter(|s| s["active"].as_bool() == Some(true))
                    .count(),
            devices
                .iter()
                .filter(|d| !d.revoked && streams_of(d.id) == 0)
                .count()
                + airplay
                    .iter()
                    .filter(|s| {
                        s["revoked"].as_bool() != Some(true) && s["active"].as_bool() != Some(true)
                    })
                    .count(),
            devices.iter().filter(|d| d.revoked).count()
                + airplay
                    .iter()
                    .filter(|s| s["revoked"].as_bool() == Some(true))
                    .count(),
        ];
        ui.horizontal_wrapped(|ui| {
            let changed = widgets::field_sized(ui, "搜索设备", &mut self.search, false, 280.0);
            let _ = changed;
            ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                if !self.search.is_empty() && widgets::button(ui, "清除", Kind::Secondary).clicked()
                {
                    self.search.clear();
                }
                ui.add_space(8.0);
                let items: Vec<(&str, usize)> = FILTERS
                    .iter()
                    .copied()
                    .zip(counts.iter().copied())
                    .collect();
                if let Some(pick) = widgets::chips(ui, self.device_filter, &items) {
                    self.device_filter = pick;
                }
            });
        });
        let Some(state) = snapshot.clone() else {
            empty_card(ui, "未连接房间", "连接 Hub 后可查看和管理配对设备。");
            self.local_devices(ui);
            return;
        };
        let needle = self.search.trim().to_lowercase();
        let filter = self.device_filter;
        let keep = |revoked: bool, allowed: bool, sending: bool| match filter {
            1 => !revoked && allowed && sending,
            2 => !revoked && !sending,
            3 => revoked,
            _ => true,
        };
        let mut matches: Vec<_> = devices
            .into_iter()
            .filter(|d| {
                (needle.is_empty()
                    || d.name.to_lowercase().contains(&needle)
                    || d.id.to_string().contains(&needle))
                    && keep(d.revoked, d.playback_allowed, streams_of(d.id) > 0)
            })
            .collect();
        // Allowed devices first, then disconnected, revoked last.
        matches.sort_by(|a, b| {
            (a.revoked, !a.playback_allowed, &a.name).cmp(&(
                b.revoked,
                !b.playback_allowed,
                &b.name,
            ))
        });
        let airplay_matches: Vec<_> = airplay
            .into_iter()
            .filter(|s| {
                airplay_source_matches(s, &needle)
                    && keep(
                        s["revoked"].as_bool() == Some(true),
                        s["blocked"].as_bool() != Some(true),
                        s["active"].as_bool() == Some(true),
                    )
            })
            .collect();
        let count = matches.len() + airplay_matches.len();
        if count == 0 {
            widgets::surface(ui, None, Margin::symmetric(16, 12), |ui| {
                ui.set_min_width(ui.available_width());
                if needle.is_empty() && filter == 0 {
                    widgets::empty(
                        ui,
                        "暂无设备",
                        "Sender 配对或 AirPlay 来源连接后，设备会显示在这里。",
                    );
                } else {
                    widgets::empty(
                        ui,
                        "无匹配结果",
                        "没有符合搜索和筛选条件的设备，清除条件后重试。",
                    );
                }
            });
        } else {
            // Active AirPlay first, then paired devices, then idle AirPlay.
            let active = |s: &&Value| s["active"].as_bool() == Some(true);
            let mut cards: Vec<Card> = airplay_matches
                .iter()
                .filter(active)
                .map(|s| Card::Airplay((*s).clone()))
                .collect();
            cards.extend(matches.into_iter().map(|d| {
                let n = streams_of(d.id);
                Card::Device(d, n)
            }));
            cards.extend(
                airplay_matches
                    .iter()
                    .filter(|s| !active(s))
                    .map(|s| Card::Airplay((*s).clone())),
            );
            let cols = widgets::columns_for(ui, 300.0, 3);
            ui.columns(cols, |c| {
                for column in c.iter_mut() {
                    column.spacing_mut().item_spacing.y = 10.0;
                }
                for (i, card) in cards.iter().enumerate() {
                    self.device_card(&mut c[i % cols], &state, card);
                }
            });
        }
        self.local_devices(ui);
    }

    fn device_card(&mut self, ui: &mut egui::Ui, state: &Snapshot, card: &Card) {
        let (key, color, revoked) = match card {
            Card::Airplay(s) => (
                s["source_id"].as_str().unwrap_or("airplay").to_owned(),
                theme::SRC_AIRPLAY,
                s["revoked"].as_bool() == Some(true),
            ),
            Card::Device(d, _) => (d.id.to_string(), theme::SRC_NATIVE, d.revoked),
        };
        let shown = widgets::surface(ui, None, Margin::symmetric(14, 12), |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            // `ui.columns` hands out justified layouts; cards keep natural
            // widths and left-aligned text.
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.push_id(&key, |ui| match card {
                    Card::Airplay(source) => self.airplay_device_row(ui, source),
                    Card::Device(device, streams) => self.device_row(ui, state, device, *streams),
                });
            });
        });
        let rect = shown.response.rect;
        let painter = ui.painter();
        painter.rect_filled(
            egui::Rect::from_min_size(
                rect.min + egui::vec2(16.0, 1.0),
                egui::vec2(rect.width() - 32.0, 3.0),
            ),
            egui::CornerRadius::same(2),
            if revoked { theme::TEXT_3 } else { color },
        );
        if revoked {
            // Revoked identities stay listed but read as struck out.
            crate::fx::hatch(
                painter,
                rect.shrink(2.0),
                theme::DANGER.gamma_multiply(0.10),
                9.0,
            );
        }
    }

    fn device_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &Snapshot,
        device: &neonmix_control::Device,
        streams: usize,
    ) {
        let (state_text, tone) = device_state(device, streams);
        let has_actions = !device.playback_allowed || device.role != Role::Admin;
        let show_actions = self.admin() && !device.revoked && has_actions;
        let wide = ui.available_width() >= 640.0;
        let info = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                let color = if device.revoked {
                    theme::DANGER
                } else {
                    role_color(device.role)
                };
                widgets::avatar(ui, &device.name, color, 32.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(&device.name)
                                .font(theme::heading(theme::BODY))
                                .color(theme::TEXT),
                        );
                        widgets::pill(ui, role_name(device.role), Tone::Neutral);
                        let permission = if device.revoked {
                            ("凭证已失效", Tone::Danger)
                        } else if device.playback_allowed {
                            (state_text, tone)
                        } else {
                            ("播放已禁止", Tone::Warning)
                        };
                        widgets::pill(ui, permission.0, permission.1);
                        if streams > 1 {
                            widgets::pill(ui, &format!("{streams} 路"), Tone::Accent);
                        }
                    });
                    widgets::mono(ui, format!("ID {}", device.id));
                });
            });
        };
        let actions = |this: &mut Self, ui: &mut egui::Ui| {
            ui.add_enabled_ui(this.writable(), |ui| {
                if device.role != Role::Admin {
                    let revoke = widgets::button(ui, "撤销配对…", Kind::Quiet);
                    if revoke.clicked() {
                        this.confirmation(
                            format!("撤销 {} 的配对", device.name),
                            "立即结束该设备的媒体和控制连接；旧凭证将失效，需要重新配对。".into(),
                            Request::Control {
                                credential: this.credential.clone(),
                                hub: this.hub(),
                                expected_revision: state.revision,
                                operation: Operation::Revoke {
                                    device_id: device.id,
                                },
                            },
                            revoke.id,
                        );
                    }
                }
                if device.playback_allowed
                    && device.role != Role::Admin
                    && widgets::button(ui, "断开设备", Kind::Secondary).clicked()
                {
                    this.operation(Operation::Disconnect {
                        device_id: device.id,
                    });
                }
                if !device.playback_allowed
                    && widgets::button(ui, "重新允许播放", Kind::Primary).clicked()
                {
                    this.operation(Operation::AllowPlayback {
                        device_id: device.id,
                    });
                }
            });
        };
        if wide && show_actions {
            ui.horizontal(|ui| {
                let width = ui.available_width() - 290.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 40.0),
                    Layout::left_to_right(Align::Center),
                    info,
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| actions(self, ui));
            });
        } else {
            info(ui);
            if show_actions {
                // Stacked: actions sit right-aligned under the identity.
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), theme::CONTROL_HEIGHT),
                    Layout::right_to_left(Align::Center),
                    |ui| actions(self, ui),
                );
            }
        }
    }

    fn local_devices(&mut self, ui: &mut egui::Ui) {
        let open = self.panel_open("local-devices", false);
        let mut refresh = false;
        let count = self.devices.len();
        let panel = widgets::panel(
            ui,
            "local-devices",
            "本机音频设备身份",
            Some(&format!("{count} 个设备 · 用于选择实体输出与虚拟输出")),
            open,
            |ui| {
                refresh = widgets::small_button(ui, true, "刷新设备").clicked();
            },
            |ui| {
                if self.devices.is_empty() {
                    widgets::note(ui, "未取得本机音频设备。");
                }
                for device in &self.devices {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&device.name).color(theme::TEXT));
                        if device.output.is_some() {
                            widgets::pill(ui, "输出", Tone::Neutral);
                        }
                        if device.input.is_some() {
                            widgets::pill(ui, "输入", Tone::Neutral);
                        }
                    });
                    widgets::mono(ui, &device.id);
                }
            },
        );
        if panel.toggled {
            self.toggle_panel("local-devices", open);
        }
        if refresh {
            self.request(Request::Devices);
        }
    }
}

pub(crate) fn empty_card(ui: &mut egui::Ui, title: &str, description: &str) {
    widgets::surface(ui, None, Margin::same(16), |ui| {
        ui.set_min_width(ui.available_width());
        widgets::empty(ui, title, description);
    });
}

/// One card in the device grid.
enum Card {
    Airplay(Value),
    Device(neonmix_control::Device, usize),
}
