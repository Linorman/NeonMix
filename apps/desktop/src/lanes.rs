//! One mixer input, native or AirPlay, as every page sees it: identity,
//! state, mix, meters and buffer metrics keyed by the actual stream id.
//! Mixer rows, the live signal flow and the Sender pipeline share it.
use super::*;
use crate::pages::role_name;
use crate::widgets::Tone;

pub(crate) struct Lane {
    pub(crate) key: u64,
    pub(crate) airplay_target: Option<(String, u64)>,
    pub(crate) name: String,
    pub(crate) role: Option<&'static str>,
    pub(crate) status: &'static str,
    pub(crate) tone: Tone,
    pub(crate) gain: f32,
    pub(crate) muted: bool,
    pub(crate) solo: bool,
    pub(crate) can_mix: bool,
    pub(crate) can_solo: bool,
    pub(crate) audible: bool,
    pub(crate) peak: Option<f64>,
    pub(crate) rms: Option<f64>,
    pub(crate) device_id: Option<uuid::Uuid>,
    pub(crate) queue_ms: Option<f64>,
    pub(crate) drift_ppm: Option<f64>,
    pub(crate) network: (&'static str, Option<Tone>),
    /// Session state for native lanes; AirPlay sessions in the list are live.
    pub(crate) session: Option<SessionStatus>,
    /// This window's own device.
    pub(crate) mine: bool,
}

impl Lane {
    pub(crate) fn is_airplay(&self) -> bool {
        self.airplay_target.is_some()
    }

    /// Colour that names where the sound comes from.
    pub(crate) fn source_color(&self) -> egui::Color32 {
        if self.is_airplay() {
            theme::SRC_AIRPLAY
        } else {
            theme::SRC_NATIVE
        }
    }
}

impl Desktop {
    pub(crate) fn lanes(&self, state: &Snapshot) -> Vec<Lane> {
        let diag = self.diagnostics.as_ref();
        let lane_index = |id: u64| {
            diag.and_then(|d| d.get("lane_stream_ids")?.as_array().cloned())
                .and_then(|ids| ids.iter().position(|v| v.as_u64() == Some(id)))
        };
        let meter = |id: u64| {
            diag.and_then(|v| v.pointer("/meters/lanes"))
                .and_then(Value::as_array)
                .and_then(|lanes| lanes.iter().find(|v| v["stream_id"].as_u64() == Some(id)))
                .map(|v| (v["peak"].as_f64(), v["rms"].as_f64()))
                .unwrap_or((None, None))
        };
        let at =
            |key: &str, index: Option<usize>| index.and_then(|i| diag?.get(key)?.get(i)?.as_f64());
        let mine = self
            .status
            .as_ref()
            .and_then(|s| s.profiles.iter().find(|p| p.credential == self.credential))
            .and_then(|p| p.device_id);
        let airplay_sessions = self.airplay_sessions();
        let any_solo = state.streams.values().any(|s| s.mix.solo)
            || airplay_sessions
                .iter()
                .any(|s| s.pointer("/mix/solo").and_then(Value::as_bool) == Some(true));
        let mut streams: Vec<_> = state.streams.values().collect();
        streams.sort_by_key(|s| s.id);
        let mut lanes: Vec<Lane> = streams
            .into_iter()
            .map(|stream| {
                let device = state.devices.get(&stream.device_id);
                let session = state.sessions.get(&stream.session_id).map(|s| s.status);
                let solo_elsewhere = any_solo && !stream.mix.solo;
                let (status, tone) = if !state.output.available {
                    ("输出丢失", Tone::Warning)
                } else if stream.mix.muted {
                    ("已静音", Tone::Warning)
                } else if solo_elsewhere {
                    ("因 Solo 静音", Tone::Warning)
                } else {
                    let tone = match session {
                        Some(SessionStatus::Playing) => Tone::Success,
                        Some(SessionStatus::Buffering) => Tone::Accent,
                        Some(SessionStatus::NetworkDegraded) => Tone::Warning,
                        Some(
                            SessionStatus::NetworkInterrupted
                            | SessionStatus::Revoked
                            | SessionStatus::OutputLost
                            | SessionStatus::AdminDisconnected,
                        ) => Tone::Danger,
                        _ => Tone::Neutral,
                    };
                    (status_text(session), tone)
                };
                let network = match session {
                    Some(SessionStatus::Playing) => ("良好", Some(Tone::Success)),
                    Some(SessionStatus::Buffering) => ("缓冲", None),
                    Some(SessionStatus::NetworkDegraded) => ("降级", Some(Tone::Warning)),
                    Some(SessionStatus::NetworkInterrupted) => ("中断", Some(Tone::Danger)),
                    None => ("未取得", None),
                    _ => ("已停止", None),
                };
                let (peak, rms) = meter(stream.id);
                let index = lane_index(stream.id);
                Lane {
                    key: stream.id,
                    airplay_target: None,
                    name: device.map_or("未知设备".into(), |d| d.name.clone()),
                    role: device.map(|d| role_name(d.role)),
                    status,
                    tone,
                    gain: stream.mix.gain_db,
                    muted: stream.mix.muted,
                    solo: stream.mix.solo,
                    can_mix: self.controls_room() || mine == Some(stream.device_id),
                    can_solo: self.controls_room(),
                    audible: state.output.available && !stream.mix.muted && !solo_elsewhere,
                    peak,
                    rms,
                    device_id: Some(stream.device_id),
                    queue_ms: at("queues", index).map(|f| f / 48.0),
                    drift_ppm: at("drift_ppm", index),
                    network,
                    session,
                    mine: mine == Some(stream.device_id),
                }
            })
            .collect();
        for airplay in &airplay_sessions {
            let (Some(stream_id), Some(session_id), Some(source_id)) = (
                airplay["stream_id"].as_u64(),
                airplay["session_id"].as_u64(),
                airplay["source_id"].as_str(),
            ) else {
                continue;
            };
            let mix = |k: &str| airplay.pointer(&format!("/mix/{k}"));
            let muted = mix("muted").and_then(Value::as_bool).unwrap_or(false);
            let solo = mix("solo").and_then(Value::as_bool).unwrap_or(false);
            let solo_elsewhere = any_solo && !solo;
            let (peak, rms) = meter(stream_id);
            let index = lane_index(stream_id);
            lanes.push(Lane {
                key: stream_id,
                airplay_target: Some((source_id.into(), session_id)),
                name: airplay["source_name"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or("AirPlay 来源")
                    .to_owned(),
                role: Some("AirPlay"),
                status: if muted {
                    "已静音"
                } else if solo_elsewhere {
                    "因 Solo 静音"
                } else {
                    "正在接收"
                },
                tone: if muted || solo_elsewhere {
                    Tone::Warning
                } else {
                    Tone::Success
                },
                gain: mix("gain_db").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                muted,
                solo,
                can_mix: self.admin(),
                can_solo: self.admin(),
                audible: state.output.available && !muted && !solo_elsewhere,
                peak,
                rms,
                device_id: None,
                queue_ms: at("queues", index).map(|f| f / 48.0),
                drift_ppm: at("drift_ppm", index),
                network: ("AirPlay", None),
                session: Some(SessionStatus::Playing),
                mine: false,
            });
        }
        lanes
    }

    /// Mute/Solo for a lane by key, with a restorable inverse (palette and
    /// keyboard reuse this).
    pub(crate) fn lane_toggle(&mut self, key: u64, mute: Option<bool>, solo: Option<bool>) {
        let Some(state) = self.snapshot.clone() else {
            return;
        };
        let Some(lane) = self.lanes(&state).into_iter().find(|l| l.key == key) else {
            return;
        };
        if (mute.is_some() && !lane.can_mix) || (solo.is_some() && !lane.can_solo) {
            return;
        }
        let what = match (mute, solo) {
            (Some(true), _) => "静音",
            (Some(false), _) => "取消静音",
            (_, Some(true)) => "Solo",
            _ => "取消 Solo",
        };
        let label = format!("{what}「{}」", lane.name);
        let (write, inverse) = if let Some((source_id, session_id)) = &lane.airplay_target {
            let next = AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: lane.gain,
                muted: mute.unwrap_or(lane.muted),
                solo: solo.unwrap_or(lane.solo),
            };
            let prev = AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: lane.gain,
                muted: lane.muted,
                solo: lane.solo,
            };
            (next.write(), prev.write())
        } else {
            let op = |muted, solo| {
                Write::Control(Operation::StreamMix {
                    stream_id: key,
                    gain_db: None,
                    muted,
                    solo,
                })
            };
            (
                op(mute, solo),
                op(mute.map(|_| lane.muted), solo.map(|_| lane.solo)),
            )
        };
        self.mix_change(label, write, inverse);
    }

    pub(crate) fn lane_gain(&mut self, lane: &Lane, gain: f32) {
        let label = format!(
            "「{}」音量 {} → {} dB",
            lane.name,
            widgets::gain_text(lane.gain),
            widgets::gain_text(gain)
        );
        let (write, inverse) = if let Some((source_id, session_id)) = &lane.airplay_target {
            let mix = |g| AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: g,
                muted: lane.muted,
                solo: lane.solo,
            };
            (mix(gain).write(), mix(lane.gain).write())
        } else {
            let op = |g| {
                Write::Control(Operation::StreamMix {
                    stream_id: lane.key,
                    gain_db: Some(g),
                    muted: None,
                    solo: None,
                })
            };
            (op(gain), op(lane.gain))
        };
        self.mix_change(label, write, inverse);
    }
}

impl Desktop {
    /// Append the latest readings to the in-memory histories.
    pub(crate) fn record_history(&mut self) {
        let now = Instant::now();
        self.record_metrics(now);
        let Some(state) = self.snapshot.clone() else {
            return;
        };
        let lanes = self.lanes(&state);
        for lane in &lanes {
            self.level_history.push(
                lane.key,
                crate::history::Sample {
                    at: now,
                    value: lane
                        .rms
                        .map(|r| widgets::to_db(r).unwrap_or(widgets::METER_FLOOR_DB)),
                    silenced: !lane.audible,
                },
            );
        }
        self.level_history
            .retain(|k| lanes.iter().any(|l| l.key == *k));
    }

    /// Running totals (and the queue gauge) behind the diagnostics tiles.
    fn record_metrics(&mut self, now: Instant) {
        let d = self.diagnostics.as_ref();
        let total = |p: &str| d.and_then(|d| d.pointer(p)).and_then(Value::as_u64);
        let network = d
            .and_then(|d| d.get("receivers"))
            .and_then(Value::as_array)
            .map(|r| {
                r.iter()
                    .map(|x| {
                        x["lost_packets"].as_u64().unwrap_or(0)
                            + x["late_packets"].as_u64().unwrap_or(0)
                    })
                    .sum::<u64>()
            });
        let queue = d.and_then(|d| {
            let ids = d.get("lane_stream_ids")?.as_array()?;
            let q = d.get("queues")?.as_array()?;
            ids.iter()
                .enumerate()
                .filter(|(_, id)| !id.is_null())
                .filter_map(|(i, _)| q.get(i)?.as_f64())
                .map(|f| f / 48.0)
                .reduce(f64::max)
        });
        let capture = self
            .status
            .as_ref()
            .and_then(|s| s.sender.metrics.as_ref())
            .and_then(|m| m.pointer("/capture_stats/frames"))
            .and_then(Value::as_u64);
        for (key, value) in [
            ("output", total("/underrun_frames").map(|v| v as f32)),
            ("network", network.map(|v| v as f32)),
            ("buffer", queue.map(|v| v as f32)),
            ("capture", capture.map(|v| v as f32)),
        ] {
            self.metric_history.push(
                key,
                crate::history::Sample {
                    at: now,
                    value,
                    silenced: false,
                },
            );
        }
    }
}

/// AirPlay mix is set as one triple; this keeps call sites symmetrical.
pub(crate) struct AirplayMix {
    source_id: String,
    session_id: u64,
    gain_db: f32,
    muted: bool,
    solo: bool,
}

impl AirplayMix {
    pub(crate) fn write(&self) -> Write {
        Write::Airplay(
            neonmix_airplay_adapter::control::AirplayActionV2::MixSource {
                source_id: self.source_id.clone(),
                session_id: self.session_id,
                gain_db: self.gain_db,
                muted: self.muted,
                solo: self.solo,
            },
        )
    }
}
