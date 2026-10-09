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

fn count(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer)?.as_u64()
}

impl Desktop {
    fn missing_health(&self, measurement: crate::measurement::Measurement<&Value>) -> Health {
        Health {
            value: self.tr(&Message::DiagnosticsUnavailable),
            detail: if measurement.current().is_some() {
                self.tr(&Message::DiagnosticsRequiredFieldsMissing)
            } else {
                self.observation_note(measurement)
            },
            tone: Tone::Neutral,
        }
    }
    fn health(&self) -> [Health; 4] {
        let measurement = self.diagnostic_measurement();
        let Some(d) = measurement.current() else {
            return [
                self.missing_health(measurement),
                self.missing_health(measurement),
                self.missing_health(measurement),
                self.capture_health(),
            ];
        };
        let output = if d.get("meters").is_some() && self.meters_current().is_none() {
            self.missing_health(self.meter_measurement())
        } else {
            match (
                count(d, "/output_stats/errors"),
                count(d, "/underrun_frames"),
                count(d, "/output_stats/callback_over_budget"),
                count(d, "/output_frames"),
            ) {
                (Some(errors), Some(underrun), Some(over), Some(frames)) => {
                    let historical = errors.saturating_add(underrun).saturating_add(over);
                    let recent = self
                        .diagnostic_counters
                        .delta("/output_stats/errors")
                        .zip(self.diagnostic_counters.delta("/underrun_frames"))
                        .zip(
                            self.diagnostic_counters
                                .delta("/output_stats/callback_over_budget"),
                        )
                        .map(|((a, b), c)| a.saturating_add(b).saturating_add(c));
                    let unavailable = self.snapshot.as_ref().is_some_and(|s| !s.output.available);
                    Health {
                        value: if unavailable {
                            self.tr(&Message::LaneOutputLost)
                        } else if recent.is_some_and(|n| n > 0) {
                            self.tr(&Message::DiagnosticsErrorCount {
                                count: recent.unwrap(),
                            })
                        } else if frames == 0 {
                            self.tr(&Message::DiagnosticsNoSamples)
                        } else if recent.is_none() && historical > 0 {
                            self.tr(&Message::DiagnosticsErrorCount { count: historical })
                        } else {
                            self.tr(&Message::DiagnosticsHealthy)
                        },
                        detail: if historical > 0 {
                            self.tr(&Message::DiagnosticsHistoricalErrors { count: historical })
                        } else {
                            self.tr(&Message::DiagnosticsPlaybackFrameValue {
                                value: frames.to_string(),
                            })
                        },
                        tone: if unavailable || recent.is_some_and(|n| n > 0) {
                            Tone::Warning
                        } else if frames == 0 || (recent.is_none() && historical > 0) {
                            Tone::Neutral
                        } else {
                            Tone::Success
                        },
                    }
                }
                _ => self.missing_health(measurement),
            }
        };
        let network = match d["receivers"].as_array() {
            None => self.missing_health(measurement),
            Some(receivers) if receivers.is_empty() => Health {
                value: self.tr(&Message::DiagnosticsNoReceivers),
                detail: self.tr(&Message::DiagnosticsThereAreNoMediaReceiversLabel),
                tone: Tone::Neutral,
            },
            Some(receivers) => {
                let totals = receivers.iter().try_fold((0u64, 0u64), |(lost, late), r| {
                    Some((
                        lost.saturating_add(r["lost_packets"].as_u64()?),
                        late.saturating_add(r["late_packets"].as_u64()?),
                    ))
                });
                match totals {
                    None => self.missing_health(measurement),
                    Some((lost, late)) => {
                        let recent = self.diagnostic_counters.network_delta();
                        Health {
                            value: if recent.is_some_and(|(lost, late)| lost == 0 && late == 0)
                                || lost == 0
                            {
                                self.tr(&Message::DiagnosticsNoPacketLoss)
                            } else {
                                self.tr(&Message::DiagnosticsPacketLossValue { count: lost })
                            },
                            detail: self.tr(&Message::DiagnosticsReceiverLateCount {
                                count: receivers.len() as u64,
                                late,
                            }),
                            tone: if recent.is_some_and(|(lost, late)| lost > 0 || late > 0) {
                                Tone::Warning
                            } else {
                                if recent.is_none() && (lost > 0 || late > 0) {
                                    Tone::Neutral
                                } else {
                                    Tone::Success
                                }
                            },
                        }
                    }
                }
            }
        };
        let buffer = match d["lane_stream_ids"].as_array().zip(d["queues"].as_array()) {
            None => self.missing_health(measurement),
            Some((ids, queues)) => {
                let active = ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| !id.is_null())
                    .collect::<Vec<_>>();
                if active.is_empty() {
                    Health {
                        value: self.tr(&Message::DiagnosticsNoChannels),
                        detail: self.tr(&Message::DiagnosticsThereAreNoActiveChannelsLabel),
                        tone: Tone::Neutral,
                    }
                } else {
                    let values = active
                        .iter()
                        .map(|(index, _)| {
                            queues
                                .get(*index)?
                                .as_f64()
                                .filter(|v| v.is_finite() && *v >= 0.)
                        })
                        .collect::<Option<Vec<_>>>();
                    match values {
                        None => self.missing_health(measurement),
                        Some(values) => Health {
                            value: format!(
                                "{:.1} ms",
                                values.into_iter().reduce(f64::max).unwrap() / 48.
                            ),
                            detail: self.tr(&Message::DiagnosticsMixerQueueEstimateLargestChannel),
                            tone: Tone::Neutral,
                        },
                    }
                }
            }
        };
        [output, network, buffer, self.capture_health()]
    }
    fn capture_health(&self) -> Health {
        if !self.status.as_ref().is_some_and(|s| s.sender.running) {
            return Health {
                value: self.tr(&Message::DiagnosticsNotSending),
                detail: self.tr(&Message::DiagnosticsCaptureStatisticsAppearAfterSendingStarts),
                tone: Tone::Neutral,
            };
        }
        let measurement = self.sender_measurement();
        let Some(m) = measurement.current() else {
            return self.missing_health(measurement);
        };
        match (
            count(m, "/capture_stats/errors"),
            count(m, "/queue_drops"),
            count(m, "/capture_stats/frames"),
            count(m, "/capture_stats/silent_frames"),
        ) {
            (Some(errors), Some(drops), Some(frames), Some(silent)) => {
                let total = errors.saturating_add(drops);
                let recent = self
                    .sender_counters
                    .delta("/capture_stats/errors")
                    .zip(self.sender_counters.delta("/queue_drops"))
                    .map(|(a, b)| a.saturating_add(b));
                let acquired = self.sender_counters.delta("/capture_stats/frames");
                let zeros = self.sender_counters.delta("/capture_stats/silent_frames");
                let silence = acquired.zip(zeros).is_some_and(|(a, b)| a > 0 && a == b)
                    || (acquired.is_none() && frames > 0 && silent == frames);
                Health {
                    value: if recent.is_some_and(|n| n > 0) {
                        self.tr(&Message::DiagnosticsCaptureAnomalyCount {
                            count: recent.unwrap(),
                        })
                    } else if frames == 0 {
                        self.tr(&Message::DiagnosticsNoSamples)
                    } else if silence {
                        self.tr(&Message::DiagnosticsSamplingSilence)
                    } else if acquired == Some(0) {
                        self.tr(&Message::DiagnosticsNoRecentSamples)
                    } else if recent.is_none() && total > 0 {
                        self.tr(&Message::DiagnosticsCaptureAnomalyCount { count: total })
                    } else {
                        self.tr(&Message::DiagnosticsHealthy)
                    },
                    detail: if total > 0 {
                        self.tr(&Message::DiagnosticsHistoricalErrors { count: total })
                    } else {
                        self.tr(&Message::DiagnosticsCaptureFrameValue {
                            value: frames.to_string(),
                        })
                    },
                    tone: if recent.is_some_and(|n| n > 0) {
                        Tone::Warning
                    } else if frames == 0
                        || silence
                        || acquired == Some(0)
                        || (recent.is_none() && total > 0)
                    {
                        Tone::Neutral
                    } else {
                        Tone::Success
                    },
                }
            }
            _ => self.missing_health(measurement),
        }
    }

    pub(crate) fn diagnostics_page(&mut self, ui: &mut egui::Ui) {
        let text_diagnostics_export_redacted_diagnostics =
            self.tr(&Message::DiagnosticsExportRedactedDiagnostics);
        let text_diagnostics_room_diagnostics_must_be_available_first =
            self.tr(&Message::DiagnosticsRoomDiagnosticsMustBeAvailableFirst);
        let text_diagnostics_playback_and_output = self.tr(&Message::DiagnosticsPlaybackAndOutput);
        let text_diagnostics_media_network = self.tr(&Message::DiagnosticsMediaNetwork);
        let text_diagnostics_buffering = self.tr(&Message::DiagnosticsBuffering);
        let text_diagnostics_sender_capture = self.tr(&Message::DiagnosticsSenderCapture);
        let available = self.diagnostics_current().is_some();
        ui.horizontal_wrapped(|ui| {
            widgets::note(ui, self.observation_note(self.diagnostic_measurement()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::button_busy(
                    ui,
                    available,
                    self.pending("export"),
                    text_diagnostics_export_redacted_diagnostics.as_str(),
                    Kind::Secondary,
                )
                .on_hover_text(self.tr(&Message::DiagnosticsExportDestinationTooltip {
                    path: (self.client.state_dir().display()).to_string(),
                }))
                .on_disabled_hover_text(
                    text_diagnostics_room_diagnostics_must_be_available_first.as_str(),
                )
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
        let titles = [
            text_diagnostics_playback_and_output.as_str(),
            text_diagnostics_media_network.as_str(),
            text_diagnostics_buffering.as_str(),
            text_diagnostics_sender_capture.as_str(),
        ];
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
                            Tone::Success | Tone::Neutral => theme::accent(),
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
            |ui| {
                if let Some(index) = PANELS.iter().position(|panel| *panel == key) {
                    let observation = if index == 3 {
                        self.sender_measurement()
                    } else {
                        self.diagnostic_measurement()
                    };
                    widgets::note(ui, self.observation_note(observation));
                }
                body(self, ui);
            },
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
        let text_diagnostics_connection_and_processes =
            self.tr(&Message::DiagnosticsConnectionAndProcesses);
        let text_diagnostics_running = self.tr(&Message::DiagnosticsRunning);
        let text_diagnostics_stopped = self.tr(&Message::DiagnosticsStopped);
        let text_diagnostics_room_status = self.tr(&Message::DiagnosticsRoomStatus);
        let text_diagnostics_unavailable = self.tr(&Message::DiagnosticsUnavailable);
        let text_diagnostics_local_background_pid =
            self.tr(&Message::DiagnosticsLocalBackgroundPid);
        let text_diagnostics_state_revision = self.tr(&Message::DiagnosticsStateRevision);
        self.detail_panel(
            ui,
            "diag-health",
            text_diagnostics_connection_and_processes.as_str(),
            None,
            |this, ui| {
                let run = |r: bool| {
                    if r {
                        text_diagnostics_running.as_str()
                    } else {
                        text_diagnostics_stopped.as_str()
                    }
                    .to_owned()
                };
                let mut rows = vec![(
                    text_diagnostics_room_status.as_str(),
                    match this.fresh {
                        Some(t) => this.tr(&Message::DiagnosticsStatusAge {
                            seconds: format!("{:.1}", t.elapsed().as_secs_f32()),
                        }),
                        None => text_diagnostics_unavailable.as_str().into(),
                    },
                )];
                if let Some(status) = &this.status {
                    rows.push((
                        text_diagnostics_local_background_pid.as_str(),
                        status.pid.to_string(),
                    ));
                    rows.push(("Hub", run(status.hub.running)));
                    rows.push(("Sender", run(status.sender.running)));
                }
                if let Some(snapshot) = &this.snapshot {
                    rows.push((
                        text_diagnostics_state_revision.as_str(),
                        snapshot.revision.to_string(),
                    ));
                }
                widgets::kv_grid(ui, "health", &rows);
                this.connection_override(ui);
            },
        );
    }

    fn output_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let text_diagnostics_playback_and_output = self.tr(&Message::DiagnosticsPlaybackAndOutput);
        let text_diagnostics_playback_frames_total =
            self.tr(&Message::DiagnosticsPlaybackFramesTotal);
        let text_diagnostics_output_errors_total = self.tr(&Message::DiagnosticsOutputErrorsTotal);
        let text_diagnostics_callbacks_over_budget_total =
            self.tr(&Message::DiagnosticsCallbacksOverBudgetTotal);
        let text_diagnostics_underrun_frames_total =
            self.tr(&Message::DiagnosticsUnderrunFramesTotal);
        let text_diagnostics_limited_frames_total =
            self.tr(&Message::DiagnosticsLimitedFramesTotal);
        let text_diagnostics_unavailable = self.tr(&Message::DiagnosticsUnavailable);
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(
            ui,
            PANELS[0],
            text_diagnostics_playback_and_output.as_str(),
            Some(health),
            |this, ui| {
                let rows: Vec<_> = [
                    (
                        text_diagnostics_playback_frames_total.as_str(),
                        "/output_frames",
                    ),
                    (
                        text_diagnostics_output_errors_total.as_str(),
                        "/output_stats/errors",
                    ),
                    (
                        text_diagnostics_callbacks_over_budget_total.as_str(),
                        "/output_stats/callback_over_budget",
                    ),
                    (
                        text_diagnostics_underrun_frames_total.as_str(),
                        "/underrun_frames",
                    ),
                    (
                        text_diagnostics_limited_frames_total.as_str(),
                        "/limited_frames",
                    ),
                ]
                .into_iter()
                .map(|(label, pointer)| {
                    (
                        label,
                        diagnostics
                            .pointer(pointer)
                            .map_or(text_diagnostics_unavailable.as_str().into(), |v| {
                                this.value_text(v)
                            }),
                    )
                })
                .collect();
                widgets::kv_grid(ui, "output", &rows);
                if let Some(errors) = diagnostics
                    .get("errors")
                    .and_then(Value::as_array)
                    .filter(|errors| !errors.is_empty())
                {
                    widgets::note(
                        ui,
                        this.tr(&Message::DiagnosticsOutputFaultCount {
                            count: errors.len() as u64,
                        }),
                    );
                }
            },
        );
    }

    fn network_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let text_diagnostics_media_network = self.tr(&Message::DiagnosticsMediaNetwork);
        let text_diagnostics_media_receiver_statistics_are_unavailable =
            self.tr(&Message::DiagnosticsMediaReceiverStatisticsAreUnavailable);
        let text_diagnostics_there_are_no_media_receivers =
            self.tr(&Message::DiagnosticsThereAreNoMediaReceivers);
        let text_diagnostics_lost_packets_total = self.tr(&Message::DiagnosticsLostPacketsTotal);
        let text_diagnostics_late_packets_total = self.tr(&Message::DiagnosticsLatePacketsTotal);
        let text_diagnostics_plc_samples_total = self.tr(&Message::DiagnosticsPlcSamplesTotal);
        let text_diagnostics_pcm_timing_gaps_total =
            self.tr(&Message::DiagnosticsPcmTimingGapsTotal);
        let text_diagnostics_pcm_queue_drops_total =
            self.tr(&Message::DiagnosticsPcmQueueDropsTotal);
        let text_diagnostics_unavailable = self.tr(&Message::DiagnosticsUnavailable);
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(
            ui,
            PANELS[1],
            text_diagnostics_media_network.as_str(),
            Some(health),
            |this, ui| {
                let Some(receivers) = diagnostics.get("receivers").and_then(Value::as_array) else {
                    widgets::note(
                        ui,
                        text_diagnostics_media_receiver_statistics_are_unavailable.as_str(),
                    );
                    return;
                };
                if receivers.is_empty() {
                    widgets::note(ui, text_diagnostics_there_are_no_media_receivers.as_str());
                }
                for (i, r) in receivers.iter().enumerate() {
                    widgets::inset(ui, |ui| {
                        widgets::caption(
                            ui,
                            &this.tr(&Message::DiagnosticsReceiverIndex {
                                index: (i + 1) as u64,
                            }),
                        );
                        let rows: Vec<_> = [
                            (text_diagnostics_lost_packets_total.as_str(), "lost_packets"),
                            (text_diagnostics_late_packets_total.as_str(), "late_packets"),
                            (text_diagnostics_plc_samples_total.as_str(), "plc_samples"),
                            (
                                text_diagnostics_pcm_timing_gaps_total.as_str(),
                                "pcm_timing_gap_count",
                            ),
                            (
                                text_diagnostics_pcm_queue_drops_total.as_str(),
                                "queue_drops",
                            ),
                        ]
                        .into_iter()
                        .map(|(label, key)| {
                            (
                                label,
                                r.get(key)
                                    .map_or(text_diagnostics_unavailable.as_str().into(), |v| {
                                        this.value_text(v)
                                    }),
                            )
                        })
                        .collect();
                        widgets::kv_grid(ui, &format!("receiver-{i}"), &rows);
                    });
                }
            },
        );
    }

    fn buffer_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let text_diagnostics_buffering = self.tr(&Message::DiagnosticsBuffering);
        let text_diagnostics_there_are_no_active_channels =
            self.tr(&Message::DiagnosticsThereAreNoActiveChannels);
        let text_diagnostics_mixer_queue_estimate_queued_frames_48_khz_it =
            self.tr(&Message::DiagnosticsMixerQueueEstimateQueuedFrames48KhzIt);
        let text_diagnostics_source = self.tr(&Message::DiagnosticsSource);
        let text_diagnostics_source_lead_time = self.tr(&Message::DiagnosticsSourceLeadTime);
        let text_diagnostics_playback_advance = self.tr(&Message::DiagnosticsPlaybackAdvance);
        let text_diagnostics_time_until_playback_after_reception =
            self.tr(&Message::DiagnosticsTimeUntilPlaybackAfterReception);
        let text_diagnostics_unavailable = self.tr(&Message::DiagnosticsUnavailable);
        let text_diagnostics_calculated_from_the_latest_received_packet_this_is =
            self.tr(&Message::DiagnosticsCalculatedFromTheLatestReceivedPacketThisIs);
        let Some(diagnostics) = self.diagnostics.clone() else {
            return;
        };
        self.detail_panel(
            ui,
            PANELS[2],
            text_diagnostics_buffering.as_str(),
            Some(health),
            |this, ui| {
                let ids = diagnostics.get("lane_stream_ids").and_then(Value::as_array);
                let queues = diagnostics.get("queues").and_then(Value::as_array);
                let drift = diagnostics.get("drift_ppm").and_then(Value::as_array);
                let mut rows = Vec::new();
                if let (Some(ids), Some(queues)) = (ids, queues) {
                    for (index, id) in ids.iter().enumerate() {
                        let Some(id) = id.as_u64() else { continue };
                        let ms = queues
                            .get(index)
                            .and_then(Value::as_f64)
                            .filter(|v| v.is_finite() && *v >= 0.)
                            .map(|v| v / 48.0);
                        let ppm = drift.and_then(|d| d.get(index)).and_then(Value::as_f64);
                        // The Hub publishes AirPlay on its last lane.
                        let name = if index + 1 == ids.len() {
                            "AirPlay".to_owned()
                        } else {
                            this.tr(&Message::DiagnosticsChannelId {
                                id: (short_id(id)).to_string(),
                            })
                        };
                        let value = match ppm {
                            Some(ppm) => this.tr(&Message::DiagnosticsQueueAndDrift {
                                milliseconds: ms
                                    .map_or(this.tr(&Message::DiagnosticsUnavailable), |value| {
                                        format!("{value:.1}")
                                    }),
                                drift: format!("{ppm:+.1}"),
                            }),
                            None => ms.map_or(this.tr(&Message::DiagnosticsUnavailable), |ms| {
                                format!("{ms:.1} ms")
                            }),
                        };
                        rows.push((name, value));
                    }
                }
                if rows.is_empty() {
                    widgets::note(ui, text_diagnostics_there_are_no_active_channels.as_str());
                } else {
                    let rows: Vec<_> = rows.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
                    widgets::kv_grid(ui, "queues", &rows);
                }
                widgets::note(
                    ui,
                    text_diagnostics_mixer_queue_estimate_queued_frames_48_khz_it.as_str(),
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
                        &this.tr(&Message::DiagnosticsAirplayChannel {
                            name: (session["source_name"]
                                .as_str()
                                .unwrap_or(text_diagnostics_source.as_str()))
                            .to_string(),
                            id: (short_id(stream)).to_string(),
                        }),
                    );
                    let rows: Vec<_> = [
                        (
                            text_diagnostics_source_lead_time.as_str(),
                            "protocol_lead_ns",
                        ),
                        (
                            text_diagnostics_playback_advance.as_str(),
                            "latency_advance_ns",
                        ),
                        (
                            text_diagnostics_time_until_playback_after_reception.as_str(),
                            "playout_lead_ns",
                        ),
                    ]
                    .into_iter()
                    .map(|(label, key)| {
                        (
                            label,
                            ingress[key]
                                .as_u64()
                                .map_or(text_diagnostics_unavailable.as_str().into(), |ns| {
                                    format!("{:.1} ms", ns as f64 / 1_000_000.0)
                                }),
                        )
                    })
                    .collect();
                    ui.push_id(stream, |ui| widgets::kv_grid(ui, "airplay-timing", &rows));
                    widgets::note(
                        ui,
                        text_diagnostics_calculated_from_the_latest_received_packet_this_is
                            .as_str(),
                    );
                }
            },
        );
    }

    fn capture_panel(&mut self, ui: &mut egui::Ui, health: &Health) {
        let text_diagnostics_sender_capture = self.tr(&Message::DiagnosticsSenderCapture);
        let text_diagnostics_capture_statistics_are_unavailable_start_sending_then_refresh =
            self.tr(&Message::DiagnosticsCaptureStatisticsAreUnavailableStartSendingThenRefresh);
        let text_diagnostics_capture_frames_total =
            self.tr(&Message::DiagnosticsCaptureFramesTotal);
        let text_diagnostics_silent_frames_total = self.tr(&Message::DiagnosticsSilentFramesTotal);
        let text_diagnostics_no_data_intervals_total =
            self.tr(&Message::DiagnosticsNoDataIntervalsTotal);
        let text_diagnostics_capture_errors_total =
            self.tr(&Message::DiagnosticsCaptureErrorsTotal);
        let text_diagnostics_sent_packets_total = self.tr(&Message::DiagnosticsSentPacketsTotal);
        let text_diagnostics_send_queue_drops_total =
            self.tr(&Message::DiagnosticsSendQueueDropsTotal);
        let text_diagnostics_unavailable = self.tr(&Message::DiagnosticsUnavailable);
        let text_diagnostics_capture_peak_session_maximum =
            self.tr(&Message::DiagnosticsCapturePeakSessionMaximum);
        let Some(status) = self.status.clone() else {
            return;
        };
        self.detail_panel(ui, PANELS[3], text_diagnostics_sender_capture.as_str(), Some(health), |this, ui| {
            let Some(metrics) = &status.sender.metrics else {
                widgets::note(ui, text_diagnostics_capture_statistics_are_unavailable_start_sending_then_refresh.as_str());
                if status.sender.error.is_some() || status.sender.fault.is_some() {
                    widgets::error_text(ui, this.tr(&process_error(&status.sender)));
                }
                return;
            };
            let mut rows: Vec<_> = [
                (text_diagnostics_capture_frames_total.as_str(), "/capture_stats/frames"),
                (text_diagnostics_silent_frames_total.as_str(), "/capture_stats/silent_frames"),
                (text_diagnostics_no_data_intervals_total.as_str(), "/capture_stats/no_data_intervals"),
                (text_diagnostics_capture_errors_total.as_str(), "/capture_stats/errors"),
                (text_diagnostics_sent_packets_total.as_str(), "/sent_packets"),
                (text_diagnostics_send_queue_drops_total.as_str(), "/queue_drops"),
            ]
            .into_iter()
            .map(|(label, pointer)| {
                (
                    label,
                    metrics.pointer(pointer).map_or(text_diagnostics_unavailable.as_str().into(), |v| this.value_text(v)),
                )
            })
            .collect();
            // Session maximum, not a live level: listed as a value, no meter.
            rows.push((
                text_diagnostics_capture_peak_session_maximum.as_str(),
                metrics
                    .pointer("/capture_stats/peak")
                    .and_then(Value::as_f64)
                    .map_or(text_diagnostics_unavailable.as_str().into(), |p| {
                        format!("{} dBFS", widgets::db_text(p))
                    }),
            ));
            widgets::kv_grid(ui, "capture", &rows);
            if status.sender.error.is_some() || status.sender.fault.is_some() {
                widgets::error_text(ui, this.tr(&process_error(&status.sender)));
            }
        });
    }
}

#[cfg(test)]
mod observation_health_tests {
    use super::*;
    fn app(value: Value) -> Desktop {
        let mut app = Desktop::empty(Client::new(".local/test-observation-health"), true, false);
        app.diagnostics_clock.success(&value, None, Instant::now());
        app.diagnostics = Some(value);
        app
    }
    fn complete() -> Value {
        serde_json::json!({"available":true,"output_stats":{"errors":0,"callback_over_budget":0},"underrun_frames":0,"output_frames":480,
        "receivers":[{"stream_id":1,"lost_packets":0,"late_packets":0}],"lane_stream_ids":[1],"queues":[0]})
    }
    #[test]
    fn available_zero_missing_unavailable_and_stale_are_distinct() {
        let valid = app(complete());
        assert_eq!(
            valid.health()[0].value,
            valid.tr(&Message::DiagnosticsHealthy)
        );
        assert_eq!(
            valid.health()[1].value,
            valid.tr(&Message::DiagnosticsNoPacketLoss)
        );
        assert_eq!(valid.health()[2].value, "0.0 ms");
        for value in [
            serde_json::json!({}),
            serde_json::json!({"available":false}),
            serde_json::json!({"available":true,"output_stats":{"errors":0},"output_frames":480}),
        ] {
            let invalid = app(value);
            assert_eq!(
                invalid.health()[0].value,
                invalid.tr(&Message::DiagnosticsUnavailable)
            );
        }
        let mut stale = app(complete());
        stale.diagnostics_clock.success(
            stale.diagnostics.as_ref().unwrap(),
            None,
            Instant::now() - Duration::from_secs(4),
        );
        assert_eq!(
            stale.health()[0].value,
            stale.tr(&Message::DiagnosticsUnavailable)
        );
        assert!(stale.health()[0].detail.contains("过期"));
        let mut noframes = complete();
        noframes["output_frames"] = serde_json::json!(0);
        let noframes = app(noframes);
        assert_eq!(
            noframes.health()[0].value,
            noframes.tr(&Message::DiagnosticsNoSamples)
        );
    }
    #[test]
    fn one_missing_receiver_or_queue_field_does_not_fabricate_zero_or_hide_valid_output() {
        let mut value = complete();
        value["receivers"][0]
            .as_object_mut()
            .unwrap()
            .remove("lost_packets");
        value["queues"] = serde_json::json!([]);
        let app = app(value);
        let health = app.health();
        assert_eq!(health[0].value, app.tr(&Message::DiagnosticsHealthy));
        assert_eq!(health[1].value, app.tr(&Message::DiagnosticsUnavailable));
        assert_eq!(health[2].value, app.tr(&Message::DiagnosticsUnavailable));
    }
    #[test]
    fn capture_silence_missing_fields_stale_and_recent_errors_have_explicit_states() {
        let mut app = app(complete());
        let mut status: ServiceStatus = serde_json::from_value(
            serde_json::from_str::<Value>(include_str!(
                "../../../../docs/evidence/ui-rebuild-20261004/fixtures/full.json"
            ))
            .unwrap()["status"]
                .clone(),
        )
        .unwrap();
        status.sender.running = true;
        let mut metrics = serde_json::json!({"capture_stats":{"frames":480,"silent_frames":480,"errors":0},"queue_drops":0});
        status.sender.metrics = Some(metrics.clone());
        app.status = Some(status);
        app.sender_clock.success(&metrics, None, Instant::now());
        assert_eq!(
            app.capture_health().value,
            app.tr(&Message::DiagnosticsSamplingSilence)
        );
        app.sender_counters.update(&metrics);
        app.sender_counters.update(&metrics);
        assert_eq!(
            app.capture_health().value,
            app.tr(&Message::DiagnosticsNoRecentSamples)
        );
        metrics["capture_stats"]["frames"] = serde_json::json!(960);
        metrics["capture_stats"]["errors"] = serde_json::json!(1);
        app.sender_counters.update(&metrics);
        app.status.as_mut().unwrap().sender.metrics = Some(metrics.clone());
        assert!(matches!(app.capture_health().tone, Tone::Warning));
        metrics.as_object_mut().unwrap().remove("queue_drops");
        app.status.as_mut().unwrap().sender.metrics = Some(metrics.clone());
        assert_eq!(
            app.capture_health().value,
            app.tr(&Message::DiagnosticsUnavailable)
        );
        assert_eq!(app.health()[0].value, app.tr(&Message::DiagnosticsHealthy));
        app.sender_clock
            .success(&metrics, None, Instant::now() - Duration::from_secs(4));
        assert!(app.capture_health().detail.contains("过期"));
    }
    #[test]
    fn fresh_diagnostic_get_does_not_make_frozen_audio_samples_fresh() {
        let mut value = complete();
        value["meters"] =
            serde_json::json!({"sample_age_ms":4000,"output":{"peak":0.2,"rms":0.1},"lanes":[]});
        let mut app = app(value.clone());
        for _ in 0..3 {
            app.diagnostics_clock.success(&value, None, Instant::now());
            assert!(app.diagnostics_current().is_some());
            assert!(app.meters_current().is_none());
            assert_eq!(
                app.health()[0].value,
                app.tr(&Message::DiagnosticsUnavailable)
            );
            assert!(app.health()[0].detail.contains("过期"));
            assert_eq!(
                app.health()[1].value,
                app.tr(&Message::DiagnosticsNoPacketLoss)
            );
        }
    }
    #[test]
    fn cumulative_errors_are_not_permanent_current_failure_and_new_increment_is_visible() {
        let mut value = complete();
        value["output_stats"]["errors"] = serde_json::json!(1);
        value["receivers"][0]["lost_packets"] = serde_json::json!(1);
        let mut app = app(value.clone());
        app.diagnostic_counters.update(&value);
        app.diagnostic_counters.update(&value);
        assert_eq!(app.health()[0].value, app.tr(&Message::DiagnosticsHealthy));
        assert_eq!(
            app.health()[1].value,
            app.tr(&Message::DiagnosticsNoPacketLoss)
        );
        value["output_stats"]["errors"] = serde_json::json!(2);
        app.diagnostic_counters.update(&value);
        app.diagnostics = Some(value);
        assert_eq!(
            app.health()[0].value,
            app.tr(&Message::DiagnosticsErrorCount { count: 1 })
        );
    }
}
