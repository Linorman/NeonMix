//! Sender: one sentence about where this computer's sound goes and the one
//! action that changes it; setup steps; pairing and virtual output panels.
use super::*;
use crate::widgets::{Kind, Tone};
use egui::{Align, CornerRadius, Layout, Margin};

impl Desktop {
    fn own_device(&self) -> Option<&neonmix_control::Device> {
        let id = self
            .status
            .as_ref()?
            .profiles
            .iter()
            .find(|p| p.credential == self.credential)?
            .device_id?;
        self.snapshot.as_ref()?.devices.get(&id)
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
        use crate::viz::{Stage, Step};
        let paired = self.sender_paired();
        let running = self.status.as_ref().is_some_and(|s| s.sender.running);
        let metrics = self.status.as_ref().and_then(|s| s.sender.metrics.clone());
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
            .unwrap_or("虚拟输出")
            .to_owned();
        let device = self.own_device().cloned();
        let room = self.remote_room.clone().unwrap_or_else(|| "房间".into());
        let state = self.snapshot.clone();
        let level = state.as_ref().filter(|_| running).and_then(|s| {
            let lane = self.lanes(s).into_iter().find(|l| l.mine)?;
            let db = widgets::to_db(lane.rms?)?;
            Some(((db - widgets::METER_FLOOR_DB) / -widgets::METER_FLOOR_DB).clamp(0.0, 1.0))
        });
        let steps = [
            Step {
                title: "应用声音",
                detail: if binding.is_some() && enabled {
                    "选到该输出的所有应用".into()
                } else {
                    "在系统声音设置中选择输出".into()
                },
                state: if binding.is_some() && enabled {
                    Stage::Done
                } else {
                    Stage::Todo
                },
                icon: icons::Icon::Pulse,
            },
            Step {
                title: "虚拟输出",
                detail: match (&binding, enabled) {
                    (Some(_), true) => output_name.clone(),
                    (Some(_), false) => "已禁用".into(),
                    (None, _) if paired => "添加虚拟输出".into(),
                    _ => "配对后添加".into(),
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
                title: "局域网 · 加密",
                detail: match &device {
                    Some(d) if d.revoked => "配对已被撤销".into(),
                    Some(d) if !d.playback_allowed => "管理员已断开".into(),
                    _ if paired || running => "已配对，固定身份".into(),
                    _ if !self.candidates.is_empty() => "粘贴邀请完成配对".into(),
                    _ => "发现局域网房间".into(),
                },
                state: match &device {
                    Some(d) if d.revoked || !d.playback_allowed => Stage::Fault,
                    _ if paired || running => Stage::Done,
                    _ => Stage::Next,
                },
                icon: icons::Icon::Sender,
            },
            Step {
                title: "房间",
                detail: if running && packets > 0 {
                    format!("正在发送到「{room}」")
                } else if running {
                    "正在连接…".into()
                } else if paired {
                    format!("「{room}」· 未在发送")
                } else {
                    "尚未配对".into()
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
                title: "实体输出",
                detail: match &state {
                    Some(s) if s.output.available => self.output_name(s),
                    Some(_) => "输出丢失".into(),
                    None => "连接房间后显示".into(),
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
        let running = self.status.as_ref().is_some_and(|s| s.sender.running);
        let metrics = self.status.as_ref().and_then(|s| s.sender.metrics.clone());
        let paired = self.sender_paired();
        let room = self.remote_room.clone().unwrap_or_else(|| "房间".into());
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
                format!("与「{room}」的控制连接正在恢复"),
                "音频进程仍在运行；恢复后继续发送。".to_owned(),
                Tone::Warning,
            )
        } else if running && packets > 0 {
            (
                format!("正在发送到「{room}」"),
                "系统选到此虚拟输出的所有应用声音都会送往房间。".to_owned(),
                Tone::Success,
            )
        } else if running {
            (
                format!("正在连接「{room}」…"),
                "建立加密会话并准备采集。".to_owned(),
                Tone::Accent,
            )
        } else if !paired {
            (
                "尚未配对房间".to_owned(),
                "发现局域网中的 Hub，粘贴它给出的一次性邀请完成配对。".to_owned(),
                Tone::Neutral,
            )
        } else if device.as_ref().is_some_and(|d| d.revoked) {
            (
                "配对已被撤销".to_owned(),
                "删除本机配对后，使用新邀请重新配对。".to_owned(),
                Tone::Danger,
            )
        } else if blocked {
            (
                "管理员已断开此设备".to_owned(),
                "需要 Hub 重新允许播放，然后手动开始发送。".to_owned(),
                Tone::Warning,
            )
        } else if binding.is_none() {
            (
                format!("已配对「{room}」"),
                "还需要添加一个虚拟输出，作为系统声音的去向。".to_owned(),
                Tone::Accent,
            )
        } else if !enabled {
            (
                "虚拟输出已禁用".to_owned(),
                "启用虚拟输出后才能开始发送。".to_owned(),
                Tone::Warning,
            )
        } else {
            (
                format!("可以开始发送到「{room}」"),
                "开始前，请在系统声音设置里把输出选为该虚拟设备。".to_owned(),
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
                                .color(theme::TEXT),
                        );
                        ui.label(RichText::new(&detail).color(theme::TEXT_2));
                    });
                };
                let action = |this: &mut Self, ui: &mut egui::Ui| {
                    if running {
                        if widgets::button_busy(
                            ui,
                            true,
                            this.pending_stop,
                            "停止发送",
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
                        "开始发送",
                        Kind::Primary,
                    )
                    .on_disabled_hover_text("需要已配对、获准播放且已启用的虚拟输出")
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
                        "虚拟输出",
                        binding
                            .as_ref()
                            .and_then(|b| b["display_name"].as_str())
                            .unwrap_or("未添加"),
                        None,
                    );
                    // Session maximum, not a live level: shown as a value only.
                    widgets::metric(
                        ui,
                        "采集峰值（会话最大）",
                        &metrics
                            .as_ref()
                            .and_then(|m| m.pointer("/capture_stats/peak"))
                            .and_then(Value::as_f64)
                            .map_or("未取得".into(), |p| {
                                format!("{} dBFS", widgets::db_text(p))
                            }),
                        None,
                    );
                    widgets::metric(
                        ui,
                        "采集帧（累计）",
                        &metrics
                            .as_ref()
                            .and_then(|m| m.pointer("/capture_stats/frames"))
                            .map_or("未取得".into(), value_text),
                        None,
                    );
                });
                if let Some(e) = self.status.as_ref().and_then(|s| s.sender.error.clone()) {
                    widgets::error_text(ui, user_error(e));
                }
            },
        );
        let rect = shown.response.rect;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 16.0),
            egui::vec2(3.0, (rect.height() - 32.0).max(8.0)),
        );
        let color = animation::color(ui.ctx(), ui.id().with("send-rail"), tone.color());
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }

    fn pair_panel(&mut self, ui: &mut egui::Ui, paired: bool) {
        let open = self.panel_open("pair", !paired);
        let panel = widgets::panel(
            ui,
            "pair",
            "发现与配对",
            Some(if paired {
                "本机已配对；需要更换房间时再展开"
            } else {
                "找到房间并用一次性邀请建立信任"
            }),
            open,
            |ui| {
                if paired {
                    widgets::pill(ui, "已配对", Tone::Success);
                }
            },
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if widgets::button_busy(
                        ui,
                        true,
                        self.pending("discover"),
                        "发现局域网房间",
                        Kind::Secondary,
                    )
                    .clicked()
                    {
                        self.request(Request::Discover { seconds: 3 });
                    }
                    if self.candidates.is_empty() && !self.pending("discover") {
                        widgets::note(ui, "确认 Hub 正在共享且位于同一局域网。");
                    }
                });
                if self.pending("discover") || !self.candidates.is_empty() {
                    let found: Vec<u64> = self
                        .candidates
                        .iter()
                        .map(|c| {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            c.get("hub_id").map(value_text).hash(&mut h);
                            h.finish()
                        })
                        .collect();
                    ui.horizontal(|ui| {
                        crate::viz::radar(ui, self.pending("discover"), &found, 96.0);
                        widgets::note(
                            ui,
                            if self.pending("discover") {
                                "正在通过 mDNS 搜索局域网中的房间…".to_owned()
                            } else {
                                format!("发现 {} 个房间；身份以邀请为准。", found.len())
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
                                        .unwrap_or("未命名房间"),
                                )
                                .font(theme::heading(theme::BODY))
                                .color(theme::TEXT),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let _ = widgets::pill(ui, "身份未信任", Tone::Warning)
                                    .on_hover_text(
                                        "发现结果不可信；首次配对以邀请中的固定身份为准。",
                                    );
                            });
                        });
                        widgets::mono(
                            ui,
                            format!(
                                "Hub {}",
                                candidate.get("hub_id").map_or("未取得".into(), value_text)
                            ),
                        );
                    });
                }
                widgets::field(ui, "设备名称", &mut self.sender_name, false);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 84.0).min(440.0);
                    widgets::field_sized(
                        ui,
                        "粘贴 Hub 给出的一次性邀请",
                        &mut self.invitation,
                        !self.show_invitation,
                        width,
                    );
                    ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                        let label = if self.show_invitation {
                            "隐藏"
                        } else {
                            "显示"
                        };
                        if widgets::button(ui, label, Kind::Secondary)
                            .on_hover_text("显示或遮挡邀请")
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
                        "配对房间",
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
                        let forget = widgets::button(ui, "删除本机配对…", Kind::Quiet);
                        if forget.clicked() {
                            self.confirmation(
                                "删除本机配对".into(),
                                "停止发送并禁用本地输出，删除本机的配对凭证；需要新邀请才能重新配对。Hub 中的设备记录保留。".into(),
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
        let binding = self.binding.clone();
        let enabled = binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        let open = self.panel_open("binding", true);
        let panel = widgets::panel(
            ui,
            "binding",
            "虚拟输出",
            Some("系统声音设置中可选择的 NeonMix 设备"),
            open,
            |ui| match &binding {
                Some(_) if enabled => {
                    widgets::pill(ui, "已启用", Tone::Success);
                }
                Some(_) => {
                    widgets::pill(ui, "已禁用", Tone::Warning);
                }
                None => {
                    widgets::pill(ui, "未添加", Tone::Neutral);
                }
            },
            |ui| {
                widgets::field(ui, "虚拟输出名称", &mut self.binding_name, false);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let label = widgets::caption(ui, "虚拟输出提供者");
                    let response = egui::ComboBox::from_id_salt("provider")
                        .width((ui.available_width() - 4.0).min(300.0))
                        .selected_text(if self.provider == "blackhole" {
                            "BlackHole（外部提供者）"
                        } else {
                            "NeonMix 虚拟输出"
                        })
                        .show_ui(ui, |ui| {
                            widgets::select_value(
                                ui,
                                &mut self.provider,
                                "neonmix".into(),
                                "NeonMix 虚拟输出",
                            );
                            if cfg!(target_os = "macos") {
                                widgets::select_value(
                                    ui,
                                    &mut self.provider,
                                    "blackhole".into(),
                                    "BlackHole（外部提供者）",
                                );
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    widgets::label_combo(&response, "虚拟输出提供者");
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
                            "添加虚拟输出",
                            Kind::Primary,
                        )
                        .on_disabled_hover_text("需要已连接的 Sender 配对身份")
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
                        if widgets::button(ui, "读取已添加输出", Kind::Secondary).clicked() {
                            self.request(Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Show,
                            });
                        }
                    });
                    widgets::note(
                        ui,
                        "需要已配对的 Sender 身份和已安装的虚拟设备；缺少驱动时请按 E06 安装说明处理。",
                    );
                    return;
                };
                let revision = binding.get("revision").and_then(Value::as_u64).unwrap_or(0);
                widgets::inset(ui, |ui| {
                    ui.label(
                        RichText::new(
                            binding
                                .get("display_name")
                                .and_then(Value::as_str)
                                .unwrap_or("虚拟输出"),
                        )
                        .font(theme::heading(theme::BODY))
                        .color(theme::TEXT),
                    );
                    widgets::mono(
                        ui,
                        format!(
                            "设备 {} · 房间 {}",
                            binding.get("device_id").map_or("未取得".into(), value_text),
                            binding.get("hub_id").map_or("未取得".into(), value_text)
                        ),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    if widgets::button(ui, "保存输出名称", Kind::Secondary).clicked() {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: OutputAction::Rename {
                                expected_revision: revision,
                                name: self.binding_name.clone(),
                            },
                        });
                    }
                    let toggle = if enabled {
                        "禁用输出"
                    } else {
                        "启用输出"
                    };
                    if widgets::button(ui, toggle, Kind::Secondary).clicked() {
                        self.request(Request::Output {
                            directory: PathBuf::from("output"),
                            action: if enabled {
                                OutputAction::Disable {
                                    expected_revision: revision,
                                }
                            } else {
                                OutputAction::Enable {
                                    expected_revision: revision,
                                }
                            },
                        });
                    }
                    let remove = widgets::button(ui, "删除绑定…", Kind::Quiet);
                    if remove.clicked() {
                        self.confirmation(
                            "删除输出绑定".into(),
                            "停止此绑定的发送，保留系统音频驱动；需要重新添加后才能发送。".into(),
                            Request::Output {
                                directory: PathBuf::from("output"),
                                action: OutputAction::Remove {
                                    expected_revision: revision,
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
