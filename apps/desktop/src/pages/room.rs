//! Hub 设置: the room hero (name, output, sharing), setup steps until the
//! room is ready, then invitations and the AirPlay receiver.
use super::*;
use crate::widgets::{Kind, Step, Tone};
use egui::{Align, CornerRadius, Layout, Margin, Stroke};

impl Desktop {
    pub(crate) fn hub_page(&mut self, ui: &mut egui::Ui) {
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
                ("创建房间", state(configured, true)),
                ("开始共享", state(running, configured)),
                ("邀请设备", state(invited, configured && running)),
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
        let saved = self.status.as_ref().and_then(|s| s.hub_settings.clone());
        let dirty = saved
            .as_ref()
            .is_some_and(|s| s.name != self.room.trim() || s.output != self.output);
        let (state, tone) = if running {
            ("共享中", Tone::Success)
        } else if configured {
            ("未共享", Tone::Neutral)
        } else {
            ("尚未创建", Tone::Neutral)
        };
        let shown = egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin {
                left: 20,
                right: 18,
                top: 16,
                bottom: 16,
            })
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 10.0;
                let wide = ui.available_width() >= 560.0;
                let action = |this: &mut Self, ui: &mut egui::Ui| {
                    if running {
                        if widgets::button_busy(
                            ui,
                            true,
                            this.pending("hub-stop"),
                            "停止共享",
                            Kind::Quiet,
                        )
                        .on_hover_text("房间停止播放；已配对设备与设置保留")
                        .clicked()
                        {
                            this.request(Request::HubStop);
                        }
                    } else if configured {
                        if widgets::button_busy(
                            ui,
                            !dirty,
                            this.pending("hub-start"),
                            "开始共享",
                            Kind::Primary,
                        )
                        .on_disabled_hover_text("先保存或放弃未保存的更改")
                        .clicked()
                        {
                            this.request(Request::HubStart);
                        }
                    } else if widgets::button_busy(
                        ui,
                        !this.room.trim().is_empty() && !this.output.is_empty(),
                        this.pending("hub-settings"),
                        "创建房间",
                        Kind::Primary,
                    )
                    .on_disabled_hover_text("填写房间名称并选择实体输出设备")
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
                        let width = ui.available_width() - 240.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, 56.0),
                            Layout::top_down(Align::Min),
                            |ui| {
                                widgets::title_field(ui, "房间名称", &mut self.room, "为房间命名");
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 56.0),
                            Layout::right_to_left(Align::Center),
                            |ui| action(self, ui),
                        );
                    });
                } else {
                    widgets::title_field(ui, "房间名称", &mut self.room, "为房间命名");
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
                    "输出按设备身份绑定，不跟随系统默认设备；共享仅面向局域网。",
                );
                if let Some(e) = self.status.as_ref().and_then(|s| s.hub.error.clone()) {
                    widgets::error_text(ui, user_error(e));
                }
                if let Some(snapshot) = self.snapshot.clone() {
                    ui.separator();
                    self.member_strip(ui, &snapshot);
                }
            });
        let rect = shown.response.rect;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 16.0),
            egui::vec2(3.0, (rect.height() - 32.0).max(8.0)),
        );
        let color = animation::color(ui.ctx(), ui.id().with("room-rail"), tone.color());
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }

    fn output_picker(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let label = widgets::caption(ui, "实体输出设备");
            ui.horizontal_wrapped(|ui| {
                let width = (ui.available_width() - 210.0).clamp(160.0, 360.0);
                let response = egui::ComboBox::from_id_salt("output")
                    .width(width)
                    .selected_text(
                        self.devices
                            .iter()
                            .find(|d| d.id == self.output)
                            .map_or("请选择设备", |d| d.name.as_str()),
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
                widgets::label_combo(&response, "实体输出");
                if widgets::button_busy(
                    ui,
                    true,
                    self.pending("devices"),
                    "刷新设备",
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
                    "播放测试音",
                    Kind::Secondary,
                )
                .on_hover_text("在所选设备上播放 −36 dBFS、2 秒的低音量测试音")
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
        egui::Frame::new()
            .fill(theme::WARNING.gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, theme::WARNING.gamma_multiply(0.35)))
            .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    widgets::dot(ui, "有未保存的更改", Tone::Warning);
                    if running {
                        widgets::note(ui, "停止共享后才能保存。");
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::button_busy(
                            ui,
                            !running && !self.room.trim().is_empty() && !self.output.is_empty(),
                            self.pending("hub-settings"),
                            "保存设置",
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
                        if widgets::button(ui, "放弃更改", Kind::Secondary).clicked() {
                            self.room = saved.name.clone();
                            self.output = saved.output.clone();
                        }
                    });
                });
            });
    }

    /// Who is in the room, at a glance; the full list lives in 设备管理.
    fn member_strip(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
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
                r.on_hover_text(format!("{}（{}）", device.name, role_name(device.role)));
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
                RichText::new(format!("{} 条配对记录", devices.len() + airplay.len()))
                    .color(theme::TEXT),
            );
            if sending > 0 {
                widgets::dot(ui, &format!("{sending} 台正在发送"), Tone::Success);
            }
            if widgets::small_button(ui, true, "管理设备").clicked() {
                self.navigate(Page::Devices);
            }
        });
    }

    fn invite_panel(&mut self, ui: &mut egui::Ui) {
        let can_invite = self.writable() && self.admin();
        let open = self.panel_open("invite", true);
        let issued = !self.issued_invitation.is_empty();
        let panel = widgets::panel(
            ui,
            "invite",
            "邀请设备",
            Some("一次性邀请，120 秒内有效"),
            open,
            |ui| {
                if issued {
                    widgets::pill(ui, "邀请已创建", Tone::Accent);
                }
            },
            |ui| {
                widgets::note(
                    ui,
                    "邀请允许一台设备加入房间。请私下交给目标设备，在其 Sender 页面粘贴。",
                );
                if widgets::button_busy(
                    ui,
                    can_invite,
                    self.pending("invite"),
                    "创建一次性邀请",
                    Kind::Primary,
                )
                .on_disabled_hover_text("需要已连接的本地管理员身份")
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
        widgets::inset(ui, |ui| {
            let remaining = self.invite_expiry.map(|expiry| {
                expiry.saturating_sub(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                )
            });
            let caption = ui
                .horizontal(|ui| {
                    let caption = widgets::caption(ui, "邀请内容");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match remaining {
                        Some(0) => {
                            widgets::pill(ui, "已过期", Tone::Warning);
                        }
                        Some(s) => {
                            widgets::pill(ui, &format!("剩余 {s} 秒"), Tone::Accent);
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
                    "已复制 ✓"
                } else {
                    "复制邀请"
                };
                if widgets::small_button(ui, true, label).clicked() {
                    ui.ctx().copy_text(self.issued_invitation.clone());
                    self.copied_at = Some(Instant::now());
                }
                let show = if self.show_invitation {
                    "隐藏邀请内容"
                } else {
                    "显示邀请内容"
                };
                if widgets::small_button(ui, true, show).clicked() {
                    self.show_invitation = !self.show_invitation;
                }
                if let Some(invitation_id) = self.invite_id
                    && widgets::small_button(ui, true, "取消邀请").clicked()
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
