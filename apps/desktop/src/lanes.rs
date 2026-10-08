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
    pub(crate) role: Option<Message>,
    pub(crate) status: Message,
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
    pub(crate) network: (Message, Option<Tone>),
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
        let diag = self.diagnostics_current();
        let lane_index = |id: u64| {
            diag.and_then(|d| d.get("lane_stream_ids")?.as_array().cloned())
                .and_then(|ids| ids.iter().position(|v| v.as_u64() == Some(id)))
        };
        let meter = |id: u64, session_id: Value, epoch: Option<u64>| {
            self.meters_current()
                .and_then(|v| v.get("lanes"))
                .and_then(Value::as_array)
                .and_then(|lanes| lanes.iter().find(|v| v["stream_id"].as_u64() == Some(id)))
                .filter(|v| v["available"].as_bool() != Some(false))
                .filter(|v| {
                    v.get("session_id")
                        .is_none_or(|s| s.is_null() || *s == session_id)
                })
                .filter(|v| v.get("stream_epoch").is_none_or(|e| e.as_u64() == epoch))
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
                let index = lane_index(stream.id);
                let starved = index
                    .and_then(|i| diag?.get("render_state_by_lane")?.get(i)?.as_u64())
                    == Some(neonmix_core::mixer::LaneRenderState::Starved as u64);
                let solo_elsewhere = any_solo && !stream.mix.solo;
                let (status, tone) = if !state.output.available {
                    (Message::LaneOutputLost, Tone::Warning)
                } else if stream.mix.muted {
                    (Message::LaneMuted, Tone::Warning)
                } else if solo_elsewhere {
                    (Message::LaneSoloMuted, Tone::Warning)
                } else if starved {
                    (Message::LaneAudioSupplyInterrupted, Tone::Warning)
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
                    Some(SessionStatus::Playing) => {
                        (Message::LaneNetworkHealthy, Some(Tone::Success))
                    }
                    Some(SessionStatus::Buffering) => (Message::LaneNetworkBuffering, None),
                    Some(SessionStatus::NetworkDegraded) => {
                        (Message::LaneNetworkDegraded, Some(Tone::Warning))
                    }
                    Some(SessionStatus::NetworkInterrupted) => {
                        (Message::LaneNetworkInterrupted, Some(Tone::Danger))
                    }
                    None => (Message::LaneNetworkUnavailable, None),
                    _ => (Message::LaneNetworkStopped, None),
                };
                let (peak, rms) = meter(
                    stream.id,
                    serde_json::json!(stream.session_id),
                    state
                        .sessions
                        .get(&stream.session_id)
                        .map(|s| s.offer.stream_epoch),
                );
                Lane {
                    key: stream.id,
                    airplay_target: None,
                    name: device
                        .map_or_else(|| self.tr(&Message::LaneUnknownDevice), |d| d.name.clone()),
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
            let current = self.airplay_is_current();
            let (peak, rms) = if current {
                meter(
                    stream_id,
                    serde_json::json!(session_id),
                    airplay["stream_epoch"].as_u64(),
                )
            } else {
                (None, None)
            };
            let index = lane_index(stream_id);
            let starved = index.and_then(|i| diag?.get("render_state_by_lane")?.get(i)?.as_u64())
                == Some(neonmix_core::mixer::LaneRenderState::Starved as u64);
            lanes.push(Lane {
                key: stream_id,
                airplay_target: Some((source_id.into(), session_id)),
                name: airplay["source_name"]
                    .as_str()
                    .filter(|n| !n.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.tr(&Message::LaneAirplaySource)),
                role: Some(Message::LaneAirplay),
                status: if !current {
                    Message::LaneNetworkUnavailable
                } else if muted {
                    Message::LaneMuted
                } else if solo_elsewhere {
                    Message::LaneSoloMuted
                } else if starved {
                    Message::LaneAudioSupplyInterrupted
                } else {
                    Message::LaneReceiving
                },
                tone: if !current {
                    Tone::Neutral
                } else if muted || solo_elsewhere || starved {
                    Tone::Warning
                } else {
                    Tone::Success
                },
                gain: mix("gain_db").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                muted,
                solo,
                can_mix: self.admin() && current,
                can_solo: self.admin() && current,
                audible: current && state.output.available && !muted && !solo_elsewhere,
                peak,
                rms,
                device_id: None,
                queue_ms: at("queues", index).map(|f| f / 48.0),
                drift_ppm: at("drift_ppm", index),
                network: (Message::LaneAirplay, None),
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
        let name = lane.name.clone();
        let label = match (mute, solo) {
            (Some(true), _) => Message::LaneMuteSource { name },
            (Some(false), _) => Message::LaneUnmuteSource { name },
            (_, Some(true)) => Message::LaneSoloSource { name },
            _ => Message::LaneUnsoloSource { name },
        };
        let (write, inverse) = if let Some((source_id, session_id)) = &lane.airplay_target {
            let next = AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: None,
                muted: mute,
                solo,
            };
            let prev = AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: None,
                muted: mute.map(|_| lane.muted),
                solo: solo.map(|_| lane.solo),
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
        let label = Message::LaneGainChanged {
            name: lane.name.clone(),
            before: widgets::gain_text(lane.gain),
            after: widgets::gain_text(gain),
        };
        let (write, inverse) = if let Some((source_id, session_id)) = &lane.airplay_target {
            let mix = |g| AirplayMix {
                source_id: source_id.clone(),
                session_id: *session_id,
                gain_db: Some(g),
                muted: None,
                solo: None,
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
        let d = self.diagnostics_current();
        let total = |p: &str| d.and_then(|d| d.pointer(p)).and_then(Value::as_u64);
        let network = d
            .and_then(|d| d.get("receivers"))
            .and_then(Value::as_array)
            .and_then(|r| {
                r.iter()
                    .map(|x| {
                        Some(
                            x["lost_packets"]
                                .as_u64()?
                                .saturating_add(x["late_packets"].as_u64()?),
                        )
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|values| values.into_iter().fold(0u64, u64::saturating_add))
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
            .sender_metrics_current()
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
    gain_db: Option<f32>,
    muted: Option<bool>,
    solo: Option<bool>,
}

impl AirplayMix {
    pub(crate) fn write(&self) -> Write {
        Write::Airplay(
            neonmix_airplay_adapter::control::AirplayActionV2::PatchMixSource {
                source_id: self.source_id.clone(),
                session_id: self.session_id,
                gain_db: self.gain_db,
                muted: self.muted,
                solo: self.solo,
            },
        )
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;
    #[test]
    fn current_mixer_starvation_and_failed_diagnostics_are_shared_by_all_lane_views() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/evidence/ui-rebuild-20261004/fixtures/full.json"
        ))
        .unwrap();
        let mut app = Desktop::empty(Client::new(".local/test-lane-freshness"), true, false);
        app.load_preview(&fixture);
        let mut state = app.snapshot.clone().unwrap();
        state.output.available = true;
        for stream in state.streams.values_mut() {
            stream.mix.muted = false;
            stream.mix.solo = false;
        }
        app.airplay = None;
        let id = *state.streams.keys().next().unwrap();
        let diag = serde_json::json!({"lane_stream_ids":[id],"render_state_by_lane":[3],"meters":{"lanes":[{"stream_id":id,"peak":0.1,"rms":0.05}]}});
        app.diagnostics_clock
            .success(&diag, Some(state.runtime_epoch), Instant::now());
        app.diagnostics = Some(diag);
        let lane = app
            .lanes(&state)
            .into_iter()
            .find(|lane| lane.key == id)
            .unwrap();
        assert_eq!(lane.status, Message::LaneAudioSupplyInterrupted);
        assert_eq!(lane.peak, Some(0.1));
        app.diagnostics.as_mut().unwrap()["meters"]["lanes"][0]["session_id"] =
            serde_json::json!(uuid::Uuid::new_v4());
        assert!(
            app.lanes(&state)
                .into_iter()
                .find(|lane| lane.key == id)
                .unwrap()
                .peak
                .is_none(),
            "a cached meter from another session cannot be reattached"
        );
        app.diagnostics.as_mut().unwrap()["meters"]["lanes"][0]["session_id"] =
            serde_json::json!(state.streams[&id].session_id);
        app.diagnostics.as_mut().unwrap()["meters"]["lanes"][0]["available"] =
            serde_json::json!(false);
        assert!(
            app.lanes(&state)
                .into_iter()
                .find(|lane| lane.key == id)
                .unwrap()
                .peak
                .is_none()
        );
        app.diagnostics_clock.failed();
        let lane = app
            .lanes(&state)
            .into_iter()
            .find(|lane| lane.key == id)
            .unwrap();
        assert!(lane.peak.is_none());
        assert!(lane.rms.is_none());
        assert_ne!(lane.status, Message::LaneAudioSupplyInterrupted);
    }
}
