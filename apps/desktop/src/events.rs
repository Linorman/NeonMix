//! 房间动态: what changed between two authoritative states, in words. Built
//! only from consecutive snapshots in this window; never persisted.
use super::*;
use crate::widgets::Tone;
use std::collections::{BTreeMap, VecDeque};

pub(crate) const KEEP: usize = 20;

pub(crate) struct RoomEvent {
    pub(crate) at: Instant,
    pub(crate) text: String,
    pub(crate) tone: Tone,
}

#[derive(Clone, PartialEq)]
pub(crate) struct Mark {
    pub(crate) name: String,
    pub(crate) status: &'static str,
    pub(crate) tone: Tone,
    pub(crate) solo: bool,
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct Marks {
    pub(crate) lanes: BTreeMap<u64, Mark>,
    pub(crate) output_available: bool,
    pub(crate) output_muted: bool,
}

/// Sentences for the changes from `prev` to `next`, oldest first.
pub(crate) fn diff(prev: &Marks, next: &Marks) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    for (key, mark) in &next.lanes {
        match prev.lanes.get(key) {
            None => out.push((format!("「{}」开始接入", mark.name), Tone::Success)),
            Some(old) => {
                if old.status != mark.status {
                    out.push((format!("「{}」{}", mark.name, mark.status), mark.tone));
                }
                if old.solo != mark.solo {
                    out.push((
                        format!(
                            "「{}」{}",
                            mark.name,
                            if mark.solo { "Solo" } else { "取消 Solo" }
                        ),
                        Tone::Solo,
                    ));
                }
            }
        }
    }
    for (key, mark) in &prev.lanes {
        if !next.lanes.contains_key(key) {
            out.push((format!("「{}」已离开房间", mark.name), Tone::Neutral));
        }
    }
    if prev.output_available != next.output_available {
        out.push(if next.output_available {
            ("实体输出已恢复".into(), Tone::Success)
        } else {
            ("实体输出丢失，等待设备恢复".into(), Tone::Warning)
        });
    }
    if prev.output_muted != next.output_muted {
        out.push(if next.output_muted {
            ("房间总静音".into(), Tone::Warning)
        } else {
            ("取消房间总静音".into(), Tone::Neutral)
        });
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

/// "刚刚", "12 秒前", "3 分钟前".
pub(crate) fn ago(at: Instant) -> String {
    let s = at.elapsed().as_secs();
    match s {
        0..=4 => "刚刚".into(),
        5..=59 => format!("{s} 秒前"),
        60..=3599 => format!("{} 分钟前", s / 60),
        _ => format!("{} 小时前", s / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(name: &str, status: &'static str, solo: bool) -> Mark {
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
                (1, mark("A", "正在播放", false)),
                (2, mark("B", "正在播放", false)),
            ]
            .into(),
            output_available: true,
            output_muted: false,
        };
        let next = Marks {
            lanes: [
                (1, mark("A", "已静音", true)),
                (3, mark("iPhone", "正在接收", false)),
            ]
            .into(),
            output_available: false,
            output_muted: false,
        };
        let text: Vec<String> = diff(&prev, &next).into_iter().map(|e| e.0).collect();
        assert_eq!(
            text,
            [
                "「A」已静音",
                "「A」Solo",
                "「iPhone」开始接入",
                "「B」已离开房间",
                "实体输出丢失，等待设备恢复"
            ]
        );
        assert!(diff(&next, &next).is_empty());
    }

    #[test]
    fn list_keeps_the_newest_events() {
        let mut events = VecDeque::new();
        for i in 0..30 {
            push(
                &mut events,
                RoomEvent {
                    at: Instant::now(),
                    text: i.to_string(),
                    tone: Tone::Neutral,
                },
            );
        }
        assert_eq!(events.len(), KEEP);
        assert_eq!(events[0].text, "29");
    }
}
