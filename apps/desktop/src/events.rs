//! 房间动态: what changed between two authoritative states, in words. Built
//! only from consecutive snapshots in this window; never persisted.
use super::*;
use crate::widgets::Tone;
use neonmix_i18n::Localizer;
use std::collections::{BTreeMap, VecDeque};

pub(crate) const KEEP: usize = 20;

pub(crate) struct RoomEvent {
    pub(crate) at: Instant,
    pub(crate) text: EventText,
    pub(crate) tone: Tone,
}

#[derive(Clone, PartialEq)]
pub(crate) struct Mark {
    pub(crate) name: String,
    pub(crate) status: Message,
    pub(crate) tone: Tone,
    pub(crate) solo: bool,
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct Marks {
    pub(crate) lanes: BTreeMap<u64, Mark>,
    pub(crate) output_available: bool,
    pub(crate) output_muted: bool,
}

/// Event identity and arguments remain independent of the displayed locale.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EventText {
    Joined { name: String },
    Status { name: String, status: Message },
    Solo { name: String, enabled: bool },
    Left { name: String },
    Output { available: bool },
    MasterMute { muted: bool },
}

impl EventText {
    pub(crate) fn render(&self, localizer: &Localizer) -> String {
        let message = match self {
            Self::Joined { name } => Message::EventsSourceJoined { name: name.clone() },
            Self::Status { name, status } => Message::EventsSourceStatus {
                name: name.clone(),
                status: localizer.render(status),
            },
            Self::Solo {
                name,
                enabled: true,
            } => Message::EventsSourceSolo { name: name.clone() },
            Self::Solo {
                name,
                enabled: false,
            } => Message::EventsSourceUnsolo { name: name.clone() },
            Self::Left { name } => Message::EventsSourceLeft { name: name.clone() },
            Self::Output { available: true } => Message::EventsOutputRestored,
            Self::Output { available: false } => Message::EventsOutputLost,
            Self::MasterMute { muted: true } => Message::EventsMasterMuted,
            Self::MasterMute { muted: false } => Message::EventsMasterUnmuted,
        };
        localizer.render(&message)
    }
}

/// Changes from `prev` to `next`, oldest first. Rendering never adds events.
pub(crate) fn diff(prev: &Marks, next: &Marks) -> Vec<(EventText, Tone)> {
    let mut out = Vec::new();
    for (key, mark) in &next.lanes {
        match prev.lanes.get(key) {
            None => out.push((
                EventText::Joined {
                    name: mark.name.clone(),
                },
                Tone::Success,
            )),
            Some(old) => {
                if old.status != mark.status {
                    out.push((
                        EventText::Status {
                            name: mark.name.clone(),
                            status: mark.status.clone(),
                        },
                        mark.tone,
                    ));
                }
                if old.solo != mark.solo {
                    out.push((
                        EventText::Solo {
                            name: mark.name.clone(),
                            enabled: mark.solo,
                        },
                        Tone::Solo,
                    ));
                }
            }
        }
    }
    for (key, mark) in &prev.lanes {
        if !next.lanes.contains_key(key) {
            out.push((
                EventText::Left {
                    name: mark.name.clone(),
                },
                Tone::Neutral,
            ));
        }
    }
    if prev.output_available != next.output_available {
        out.push((
            EventText::Output {
                available: next.output_available,
            },
            if next.output_available {
                Tone::Success
            } else {
                Tone::Warning
            },
        ));
    }
    if prev.output_muted != next.output_muted {
        out.push((
            EventText::MasterMute {
                muted: next.output_muted,
            },
            if next.output_muted {
                Tone::Warning
            } else {
                Tone::Neutral
            },
        ));
    }
    out
}

impl Desktop {
    /// Compare the current state with the last one seen; cheap unless the
    /// snapshot or AirPlay revision moved.
    pub(crate) fn track_events(&mut self) {
        let key = self.snapshot.as_ref().map(|s| {
            (
                s.hub_id,
                s.revision,
                self.airplay.as_ref().and_then(|a| a["revision"].as_u64()),
                self.airplay_sessions().len(),
            )
        });
        if key == self.marks_key {
            return;
        }
        self.marks_key = key;
        let Some(state) = self.snapshot.clone() else {
            // Lost the room: the next state is a fresh baseline, not news.
            self.marks = None;
            return;
        };
        let next = Marks {
            lanes: self
                .lanes(&state)
                .into_iter()
                .map(|l| {
                    (
                        l.key,
                        Mark {
                            name: l.name,
                            status: l.status,
                            tone: l.tone,
                            solo: l.solo,
                        },
                    )
                })
                .collect(),
            output_available: state.output.available,
            output_muted: state.output.muted,
        };
        if let Some(prev) = &self.marks {
            let now = Instant::now();
            for key in next.lanes.keys().filter(|k| !prev.lanes.contains_key(k)) {
                self.joined.insert(*key, now);
            }
            self.joined
                .retain(|_, at| at.elapsed() < std::time::Duration::from_secs(2));
            for (text, tone) in diff(prev, &next) {
                push(
                    &mut self.events,
                    RoomEvent {
                        at: now,
                        text,
                        tone,
                    },
                );
            }
        }
        self.marks = Some(next);
    }
}

fn push(events: &mut VecDeque<RoomEvent>, event: RoomEvent) {
    events.push_front(event);
    events.truncate(KEEP);
}

/// Relative display time uses the current locale without changing event time.
pub(crate) fn ago(localizer: &Localizer, at: Instant) -> String {
    localizer.render(&neonmix_i18n::LocaleFormat::relative_time_message(
        at.elapsed().as_secs(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use neonmix_i18n::ResolvedLocale;

    fn mark(name: &str, status: Message, solo: bool) -> Mark {
        Mark {
            name: name.into(),
            status,
            tone: Tone::Success,
            solo,
        }
    }

    #[test]
    fn diff_names_joins_changes_leaves_and_output_in_order() {
        let prev = Marks {
            lanes: [
                (1, mark("A", Message::SessionPlaying, false)),
                (2, mark("B", Message::SessionPlaying, false)),
            ]
            .into(),
            output_available: true,
            output_muted: false,
        };
        let next = Marks {
            lanes: [
                (1, mark("A", Message::LaneMuted, true)),
                (3, mark("iPhone", Message::LaneReceiving, false)),
            ]
            .into(),
            output_available: false,
            output_muted: false,
        };
        let text: Vec<EventText> = diff(&prev, &next).into_iter().map(|e| e.0).collect();
        assert_eq!(
            text,
            [
                EventText::Status {
                    name: "A".into(),
                    status: Message::LaneMuted
                },
                EventText::Solo {
                    name: "A".into(),
                    enabled: true
                },
                EventText::Joined {
                    name: "iPhone".into()
                },
                EventText::Left { name: "B".into() },
                EventText::Output { available: false },
            ]
        );
        assert!(diff(&next, &next).is_empty());
    }

    #[test]
    fn switching_locale_rerenders_existing_history_without_new_events() {
        let marks = Marks {
            lanes: [(1, mark("中文设备", Message::SessionPlaying, false))].into(),
            output_available: true,
            output_muted: false,
        };
        let before = marks.clone();
        let at = Instant::now();
        let event = RoomEvent {
            at,
            text: EventText::Status {
                name: "中文设备".into(),
                status: Message::SessionPlaying,
            },
            tone: Tone::Success,
        };
        let mut localizer = Localizer::new(ResolvedLocale::ZhCn);
        let chinese = event.text.render(&localizer);
        localizer.set_locale(ResolvedLocale::En);
        let english = event.text.render(&localizer);
        assert_ne!(chinese, english);
        assert!(chinese.contains("中文设备"));
        assert!(english.contains("中文设备"));
        assert_eq!(event.at, at);
        assert!(diff(&before, &marks).is_empty());
        assert!(localizer.diagnostics().is_empty());
    }

    #[test]
    fn list_keeps_the_newest_events() {
        let mut events = VecDeque::new();
        for i in 0..30 {
            push(
                &mut events,
                RoomEvent {
                    at: Instant::now(),
                    text: EventText::Joined {
                        name: i.to_string(),
                    },
                    tone: Tone::Neutral,
                },
            );
        }
        assert_eq!(events.len(), KEEP);
        assert_eq!(events[0].text, EventText::Joined { name: "29".into() });
    }
}
