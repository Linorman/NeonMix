//! 诊断: four health tiles summarise output, media network, buffering and
//! capture; each opens its detail panel. Counters are cumulative and named so.
use super::*;
use crate::widgets::{Kind, Tone};
use egui::{Align, Layout};

const PANELS: [&str; 4] = ["diag-output", "diag-network", "diag-buffer", "diag-capture"];

struct Health {
    value: String,
    detail: String,
    tone: Tone,
}

/// Trend key and whether it is a running total, per health tile.
const TRENDS: [(&str, bool); 4] = [
    ("output", true),
    ("network", true),
    ("buffer", false),
    ("capture", true),
];

fn n(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

impl Desktop {
    fn health(&self) -> [Health; 4] {
        let missing = || Health {
            value: "未取得".into(),
            detail: "需要先取得房间诊断".into(),
            tone: Tone::Neutral,
        };
        let Some(d) = &self.diagnostics else {
            let capture = self.capture_health();
            return [missing(), missing(), missing(), capture];
        };
        let errors = n(d.pointer("/output_stats/errors"));
        let underrun = n(d.pointer("/underrun_frames"));
        let over = n(d.pointer("/output_stats/callback_over_budget"));
        let output = if errors > 0 {
            Health {
                value: format!("{errors} 次错误"),
                detail: "输出错误（累计）".into(),
                tone: Tone::Danger,
            }
        } else if underrun > 0 || over > 0 {
            Health {
                value: format!("欠载 {}", group_digits(underrun)),
                detail: format!("欠载帧 · 回调超预算 {over}（累计）"),
                tone: Tone::Warning,
            }
        } else {
            Health {
                value: "正常".into(),
                detail: format!(
                    "播放帧 {}（累计）",
                    d.pointer("/output_frames")
                        .map_or("未取得".into(), value_text)
                ),
                tone: Tone::Success,
            }
        };
        let receivers = d.get("receivers").and_then(Value::as_array);
        let (lost, late) = receivers.map_or((0, 0), |r| {
            r.iter().fold((0, 0), |(l, t), x| {
                (l + n(x.get("lost_packets")), t + n(x.get("late_packets")))
            })
        });
        let network = match receivers {
            None => missing(),
            Some(r) if r.is_empty() => Health {
                value: "无接收".into(),
                detail: "当前没有媒体接收器".into(),
                tone: Tone::Neutral,
            },
            Some(r) => Health {
                value: if lost == 0 {
                    "无丢包".into()
                } else {
                    format!("丢包 {}", group_digits(lost))
                },
                detail: format!("{} 个接收器 · 迟到 {late}（累计）", r.len()),
                tone: if lost > 0 {
                    Tone::Warning
                } else {
                    Tone::Success
                },
            },
        };
        let ids = d.get("lane_stream_ids").and_then(Value::as_array);
        let queues = d.get("queues").and_then(Value::as_array);
        let max_ms = ids.zip(queues).and_then(|(ids, q)| {
            ids.iter()
                .enumerate()
                .filter(|(_, id)| !id.is_null())
                .filter_map(|(i, _)| q.get(i)?.as_f64())
                .map(|f| f / 48.0)
                .reduce(f64::max)
        });
        let buffer = match max_ms {
            Some(ms) => Health {
                value: format!("{ms:.1} ms"),
                detail: "Mixer 队列估计（最大一路）".into(),
                tone: Tone::Neutral,
            },
            None => Health {
                value: "无通道".into(),
                detail: "当前没有活动通道".into(),
                tone: Tone::Neutral,
            },
        };
        [output, network, buffer, self.capture_health()]
    }

    fn capture_health(&self) -> Health {
        let metrics = self.status.as_ref().and_then(|s| s.sender.metrics.as_ref());
        match metrics {
            None => Health {
                value: "未发送".into(),
                detail: "开始发送后显示采集统计".into(),
                tone: Tone::Neutral,
            },
            Some(m) => {
                let errors = n(m.pointer("/capture_stats/errors"));
                let drops = n(m.pointer("/queue_drops"));
                Health {
                    value: if errors + drops == 0 {
                        "正常".into()
                    } else {
                        format!("{} 次异常", errors + drops)
                    },
                    detail: format!(
                        "采集帧 {}（累计）",
                        m.pointer("/capture_stats/frames")
                            .map_or("未取得".into(), value_text)
                    ),
                    tone: if errors > 0 {
                        Tone::Danger
                    } else if drops > 0 {
                        Tone::Warning
                    } else {
                        Tone::Success
                    },
                }
            }
        }
    }

    pub(crate) fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        let available = self.diagnostics.is_some();
        ui.horizontal_wrapped(|ui| {
            widgets::note(
                ui,
                match self.fresh {
                    Some(t) => format!(
                        "房间状态 {:.1} 秒前更新 · 计数为本次运行累计",
                        t.elapsed().as_secs_f32()
                    ),
                    None => "未取得有效房间状态；请检查共享状态、发现或配对身份。".into(),
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button_busy(
                    ui,
                    available,
                    self.pending("export"),
                    "导出脱敏诊断",
                    Kind::Secondary,
                )
                .on_hover_text(format!(
                    "写入 {}/diagnostics-redacted.json；只保留白名单数值和状态，不含邀请、令牌、证书、地址、设备名称和本地路径。",
                    self.client.state_dir().display()
                ))
                .on_disabled_hover_text("需要先取得房间诊断")
                .clicked()
                {
                    self.request(Request::ExportDiagnostics {
                        credential: self.credential.clone(),
                        hub: self.hub(),
                    });
                }
            });
        });
        let health = self.health();
        let titles = ["播放与输出", "媒体网络", "缓冲", "Sender 采集"];
        let columns = widgets::columns_for(ui, 170.0, 4);
        let gap = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
        let mut picked = None;
        for row in (0..4).collect::<Vec<_>>().chunks(columns) {
            ui.horizontal(|ui| {
                for &i in row {
                    let open = self.panel_open(PANELS[i], health[i].tone != Tone::Success);
                    let tile = widgets::tile(
                        ui,
                        width,
                        titles[i],
                        &health[i].value,
                        &health[i].detail,
                        health[i].tone,
                        open,
                    );
                    // Last two minutes: increases for running totals, the
                    // value itself for the queue estimate.
                    let (key, delta) = TRENDS[i];
                    if let Some(samples) = self.metric_history.get(&key).filter(|s| s.len() >= 3) {
                        let r = tile.rect;
                        let spark = egui::Rect::from_min_max(
                            egui::pos2(r.right() - 86.0, r.top() + 30.0),
                            egui::pos2(r.right() - 14.0, r.top() + 50.0),
                        );
                        let color = match health[i].tone {
                            Tone::Success | Tone::Neutral => theme::ACCENT,
                            tone => tone.color(),
                        };
                        crate::history::paint_spark(
                            ui.painter(),
                            spark,
                            samples,
                            self.metric_history.window(),
                            color,
                            delta,
                        );
                    }
                    if tile.clicked() {
                        picked = Some(i);
                    }
                }
            });
        }
        if let Some(i) = picked {
            self.panels.insert(PANELS[i], true);
            self.scroll_to = Some(PANELS[i]);
        }
        self.health_panel(ui);
        self.output_panel(ui, &health[0]);
        self.network_panel(ui, &health[1]);
        self.buffer_panel(ui, &health[2]);
        self.capture_panel(ui, &health[3]);
    }

    fn detail_panel(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        title: &str,
        health: Option<&Health>,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        let open = self.panel_open(key, health.is_some_and(|h| h.tone != Tone::Success));
        let (pill, tone) = health.map_or(("", Tone::Neutral), |h| (h.value.as_str(), h.tone));
        let panel = widgets::panel(
            ui,
            key,
            title,
            None,
            open,
            |ui| {
                if !pill.is_empty() {
                    widgets::pill(ui, pill, tone);
                }
            },
            |ui| body(self, ui),
        );
        if panel.toggled {
            self.toggle_panel(key, open);
        }
        if self.scroll_to == Some(key) {
            ui.scroll_to_rect(panel.rect, Some(Align::TOP));
            self.scroll_to = None;
        }
    }

    fn health_panel(&mut self, ui: &mut egui::Ui) {
        self.detail_panel(ui, "diag-health", "连接与进程", None, |this, ui| {
            let run = |r: bool| if r { "运行" } else { "停止" }.to_owned();
            let mut rows = vec![(
                "房间状态",
                match this.fresh {
                    Some(t) => format!("{:.1} 秒前更新", t.elapsed().as_secs_f32()),
                    None => "未取得".into(),
                },
            )];
            if let Some(status) = &this.status {
                rows.push(("本地后台 PID", status.pid.to_string()));
                rows.push(("Hub", run(status.hub.running)));
                rows.push(("Sender", run(status.sender.running)));
            }
            if let Some(snapshot) = &this.snapshot {
                rows.push(("状态版本", snapshot.revision.to_string()));
            }
            widgets::kv_grid(ui, "health", &rows);
            this.connection_override(ui);
        });
    }

    fn output_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(ui, PANELS[0], "播放与输出", Some(health), |_, ui| {
            let rows: Vec<_> = [
                ("播放帧（累计）", "/output_frames"),
                ("输出错误（累计）", "/output_stats/errors"),
                ("回调超预算（累计）", "/output_stats/callback_over_budget"),
                ("欠载帧（累计）", "/underrun_frames"),
                ("限幅帧（累计）", "/limited_frames"),
            ]
            .into_iter()
            .map(|(label, pointer)| {
                (
                    label,
                    diagnostics
                        .pointer(pointer)
                        .map_or("未取得".into(), value_text),
                )
            })
            .collect();
            widgets::kv_grid(ui, "output", &rows);
            if let Some(errors) = diagnostics.get("errors").filter(|e| !e.is_null()) {
                widgets::note(ui, format!("输出故障：{}", value_text(errors)));
            }
        });
    }

    fn network_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(ui, PANELS[1], "媒体网络", Some(health), |_, ui| {
            let Some(receivers) = diagnostics.get("receivers").and_then(Value::as_array) else {
                widgets::note(ui, "未取得媒体接收器统计。");
                return;
            };
            if receivers.is_empty() {
                widgets::note(ui, "当前没有媒体接收器。");
            }
            for (i, r) in receivers.iter().enumerate() {
                widgets::inset(ui, |ui| {
                    widgets::caption(ui, &format!("媒体接收器 {}", i + 1));
                    let rows: Vec<_> = [
                        ("丢包（累计）", "lost_packets"),
                        ("迟到包（累计）", "late_packets"),
                        ("PLC 样本（累计）", "plc_samples"),
                        ("PCM 缺口（累计）", "pcm_timing_gap_count"),
                        ("PCM 队列丢弃（累计）", "queue_drops"),
                    ]
                    .into_iter()
                    .map(|(label, key)| (label, r.get(key).map_or("未取得".into(), value_text)))
                    .collect();
                    widgets::kv_grid(ui, &format!("receiver-{i}"), &rows);
                });
            }
        });
    }

    fn buffer_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(ui, PANELS[2], "缓冲", Some(health), |this, ui| {
            let ids = diagnostics.get("lane_stream_ids").and_then(Value::as_array);
            let queues = diagnostics.get("queues").and_then(Value::as_array);
            let drift = diagnostics.get("drift_ppm").and_then(Value::as_array);
            let mut rows = Vec::new();
            if let (Some(ids), Some(queues)) = (ids, queues) {
                for (index, id) in ids.iter().enumerate() {
                    let Some(id) = id.as_u64() else { continue };
                    let ms = queues.get(index).and_then(Value::as_f64).unwrap_or(0.0) / 48.0;
                    let ppm = drift
                        .and_then(|d| d.get(index))
                        .and_then(Value::as_f64)
                        .map_or(String::new(), |p| format!(" · 漂移 {p:+.1} ppm"));
                    // The Hub publishes AirPlay on its last lane.
                    let name = if index + 1 == ids.len() {
                        "AirPlay".to_owned()
                    } else {
                        format!("通道 …{}", short_id(id))
                    };
                    rows.push((name, format!("{ms:.1} ms{ppm}")));
                }
            }
            if rows.is_empty() {
                widgets::note(ui, "当前没有活动通道。");
            } else {
                let rows: Vec<_> = rows.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
                widgets::kv_grid(ui, "queues", &rows);
            }
            widgets::note(
                ui,
                "Mixer 队列估计 = 队列帧 / 48 kHz；不含设备、网络与模拟端，不是端到端声音延迟。",
            );
            for session in this.airplay_sessions() {
                let Some(ingress) = session.get("ingress") else {
                    continue;
                };
                let stream = session["stream_id"].as_u64().unwrap_or(0);
                let meter = diagnostics
                    .pointer("/meters/lanes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .find(|m| m["stream_id"].as_u64() == Some(stream));
                widgets::meter(
                    ui,
                    ("airplay", stream),
                    meter.and_then(|m| m["peak"].as_f64()),
                    meter.and_then(|m| m["rms"].as_f64()),
                );
                widgets::caption(
                    ui,
                    &format!(
                        "AirPlay · {} · 通道 …{}",
                        session["source_name"].as_str().unwrap_or("来源"),
                        short_id(stream)
                    ),
                );
                let rows: Vec<_> = [
                    ("来源提前量", "protocol_lead_ns"),
                    ("播放提前量", "latency_advance_ns"),
                    ("接收后待播", "playout_lead_ns"),
                ]
                .into_iter()
                .map(|(label, key)| {
                    (
                        label,
                        ingress[key].as_u64().map_or("未取得".into(), |ns| {
                            format!("{:.1} ms", ns as f64 / 1_000_000.0)
                        }),
                    )
                })
                .collect();
                ui.push_id(stream, |ui| widgets::kv_grid(ui, "airplay-timing", &rows));
                widgets::note(ui, "按最近接收包的时间计算；不是端到端或扬声器实测延迟。");
            }
        });
    }

    fn capture_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let Some(status) = self.status.clone() else {
            return;
        };
        self.detail_panel(ui, PANELS[3], "Sender 采集", Some(health), |_, ui| {
            let Some(metrics) = &status.sender.metrics else {
                widgets::note(ui, "当前没有采集统计。开始发送后刷新。");
                if let Some(error) = &status.sender.error {
                    widgets::error_text(ui, user_error(error.clone()));
                }
                return;
            };
            let mut rows: Vec<_> = [
                ("采集帧（累计）", "/capture_stats/frames"),
                ("静音帧（累计）", "/capture_stats/silent_frames"),
                ("无数据间隔（累计）", "/capture_stats/no_data_intervals"),
                ("采集错误（累计）", "/capture_stats/errors"),
                ("发送包（累计）", "/sent_packets"),
                ("发包队列丢弃（累计）", "/queue_drops"),
            ]
            .into_iter()
            .map(|(label, pointer)| {
                (
                    label,
                    metrics.pointer(pointer).map_or("未取得".into(), value_text),
                )
            })
            .collect();
            // Session maximum, not a live level: listed as a value, no meter.
            rows.push((
                "采集峰值（会话最大）",
                metrics
                    .pointer("/capture_stats/peak")
                    .and_then(Value::as_f64)
                    .map_or("未取得".into(), |p| {
                        format!("{} dBFS", widgets::db_text(p))
                    }),
            ));
            widgets::kv_grid(ui, "capture", &rows);
            if let Some(error) = &status.sender.error {
                widgets::error_text(ui, user_error(error.clone()));
            }
        });
    }
}
