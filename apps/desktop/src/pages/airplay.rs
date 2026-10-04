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
                    .unwrap_or(Value::String("AirPlay 来源".into()));
                if let Some(session) = session {
                    source["session_id"] = session["session_id"].clone();
                    source["receiver_id"] = session["receiver_id"].clone();
                }
                source
            })
            .collect()
    }
    pub(crate) fn airplay_request(&self, operation: AirplayAction) -> Option<Request> {
        if !self.ready() || !self.admin() {
            return None;
        }
        let target = match &operation {
            AirplayAction::MixSource {
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
            command: Some(neonmix_airplay_adapter::control::AirplayCommandV2 {
                command_id: uuid::Uuid::new_v4().to_string(),
                expected_revision: self.airplay.as_ref()?["revision"].as_u64()?,
                operation,
            }),
        })
    }
    pub(crate) fn airplay_operation(&mut self, operation: AirplayAction) {
        self.write(Write::Airplay(operation));
    }
    pub(crate) fn airplay_panel(&mut self, ui: &mut egui::Ui) {
        let state = self.airplay.clone();
        let open = self.panel_open("airplay", true);
        let panel = widgets::panel(
            ui,
            "airplay",
            "AirPlay 接收入口",
            Some("每台设备选择一个空闲入口；占用入口会从公开发现列表隐藏"),
            open,
            |ui| {
                let enabled = state
                    .as_ref()
                    .is_some_and(|s| s["enabled"].as_bool() == Some(true));
                widgets::pill(
                    ui,
                    if enabled { "已开启" } else { "已关闭" },
                    if enabled {
                        Tone::Success
                    } else {
                        Tone::Neutral
                    },
                );
            },
            |ui| {
                let Some(state) = state.as_ref() else {
                    widgets::note(ui, "连接房间后可查看和开启 AirPlay 接收。");
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
                    widgets::caption(ui, "房间输入容量");
                    crate::viz::capacity_slots(ui, limit, &used);
                    if self.admin()
                        && widgets::button_enabled(
                            ui,
                            self.writable() && (enabled || count > 0),
                            if enabled {
                                "关闭 AirPlay 接收"
                            } else {
                                "开启 AirPlay 接收"
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
                        widgets::caption(ui, "接收入口数量");
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
                            "增加入口不中断现有来源；减少前请先断开被移除入口的来源。",
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
                                        receiver["name"].as_str().unwrap_or("AirPlay 入口"),
                                    )
                                    .color(theme::TEXT),
                                );
                                let (label, tone) = if receiver["error"].is_string() {
                                    ("入口故障", Tone::Danger)
                                } else if active {
                                    ("已连接", Tone::Success)
                                } else if ready {
                                    ("等待连接", Tone::Accent)
                                } else if on {
                                    ("正在准备", Tone::Neutral)
                                } else {
                                    ("已关闭", Tone::Neutral)
                                };
                                widgets::pill(ui, label, tone);
                                let discovery =
                                    receiver["discovery_state"].as_str().unwrap_or("hidden");
                                widgets::pill(
                                    ui,
                                    match discovery {
                                        "published" => "公开可见",
                                        "hidden" => "已从发现列表隐藏",
                                        "error" => "发现状态异常",
                                        _ => "正在更新发现列表",
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
                                            "关闭此入口"
                                        } else {
                                            "开启此入口"
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
                                        "开启配对窗口",
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
                                        let label = widgets::caption(ui, "此入口配对码");
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
                                                "隐藏配对码"
                                            } else {
                                                "显示配对码"
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
                    widgets::note(ui, "接收开关与配对由房间管理员管理。");
                }
            },
        );
        if panel.toggled {
            self.toggle_panel("airplay", open);
        }
    }
    pub(crate) fn airplay_device_row(&mut self, ui: &mut egui::Ui, source: &Value) {
        let Some(id) = source["source_id"].as_str() else {
            return;
        };
        let active = source["active"].as_bool() == Some(true);
        let revoked = source["revoked"].as_bool() == Some(true);
        let blocked = source["blocked"].as_bool() == Some(true);
        let name = source["source_name"].as_str().unwrap_or("AirPlay 来源");
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(name)
                    .font(theme::heading(theme::BODY))
                    .color(theme::TEXT),
            );
            widgets::pill(ui, "AirPlay", Tone::Neutral);
            let (label, tone) = if revoked {
                ("配对已撤销", Tone::Danger)
            } else if blocked {
                ("播放已禁止", Tone::Warning)
            } else if active {
                ("已连接", Tone::Success)
            } else {
                ("未连接", Tone::Neutral)
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
                    && widgets::button(ui, "断开 AirPlay 来源", Kind::Secondary).clicked()
                {
                    self.airplay_operation(AirplayAction::DisconnectSource {
                        source_id: id.into(),
                        session_id,
                    });
                }
                if blocked
                    && !revoked
                    && widgets::button(ui, "重新允许播放", Kind::Secondary).clicked()
                {
                    self.airplay_operation(AirplayAction::AllowSource {
                        source_id: id.into(),
                    });
                }
                if revoked {
                    let receivers=self.airplay.as_ref().and_then(|s|s["receivers"].as_array()).cloned().unwrap_or_default();
                    if !receivers.iter().any(|r|r["active"].as_bool()!=Some(true) && r["configured_enabled"].as_bool().unwrap_or(true)) {
                        widgets::note(ui,"请先在 Hub 设置启用一个空闲入口，再为此来源重新配对。");
                    }
                    for receiver in receivers.iter().filter(|r|r["active"].as_bool()!=Some(true) && r["configured_enabled"].as_bool().unwrap_or(true)) {
                        let Some(receiver_id)=receiver["receiver_id"].as_str() else {continue;};
                        let receiver_name=receiver["name"].as_str().unwrap_or("入口");
                        let repair=widgets::button(ui,&format!("在 {receiver_name} 重新配对"),Kind::Secondary);
                        if repair.clicked() && let Some(request)=self.airplay_request(AirplayAction::RepairSource{source_id:id.into(),receiver_id:receiver_id.into()}) {
                            self.confirmation(format!("允许 {name} 重新配对"),format!("为 {receiver_name} 重新开放此来源的配对；其他入口的已撤销绑定保持失效。"),request,repair.id);
                        }
                    }
                }
                if !revoked {
                    let revoke = widgets::button(ui, "撤销 AirPlay 配对", Kind::Quiet);
                    if revoke.clicked()
                        && let Some(request) = self.airplay_request(AirplayAction::RevokeSource {
                            source_id: id.into(),
                            session_id: source["session_id"].as_u64(),
                        })
                    {
                        self.confirmation(
                            format!("撤销 {name} 的配对"),
                            "结束该设备音频，并撤销它在所有入口的配对。".into(),
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
                widgets::caption(ui, "播放方式");
                for (value, label) in [
                    (PlaybackMode::LowLatency, "低延迟"),
                    (PlaybackMode::Synchronized, "音画同步"),
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
                widgets::note(ui, "断开此来源后可修改播放方式。");
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
        let key = format!("{}:{id}", if receiver { "receiver" } else { "source" });
        if !self.airplay_name_drafts.contains_key(&key) {
            if widgets::small_button(
                ui,
                self.writable() && available,
                if receiver {
                    "修改入口名称"
                } else if current.is_empty() {
                    "设置别名"
                } else {
                    "修改别名"
                },
            )
            .on_disabled_hover_text(if receiver {
                "关闭此入口后可修改，下次启动生效"
            } else {
                "等待连接房间"
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
                if receiver {
                    "入口名称"
                } else {
                    "来源别名"
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
                        "名称不能为空、不能含控制字符，最多 50 个 UTF-8 字节。"
                    } else {
                        "别名不能为空、不能含控制字符，最多 128 个 UTF-8 字节；移除请用清除别名。"
                    },
                );
            }
            ui.horizontal_wrapped(|ui| {
                if widgets::small_button(
                    ui,
                    self.writable() && available && valid && draft != current,
                    "保存",
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
                    && widgets::small_button(ui, self.writable(), "清除别名").clicked()
                {
                    draft.clear();
                    self.airplay_operation(AirplayAction::AliasSource {
                        source_id: id.into(),
                        alias: None,
                    });
                }
                if widgets::small_button(ui, true, "取消").clicked() {
                    self.airplay_name_drafts.remove(&key);
                }
            });
            if receiver {
                widgets::note(ui, "下次启动此入口时生效；接收身份和配对保持不变。");
            } else {
                widgets::note(ui, "别名仅用于房间显示，不合并设备或改变配对权限。");
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
