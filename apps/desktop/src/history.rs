//! Short client-side history of real readings: per-lane RMS for the Mixer
//! ribbon and room metrics for diagnostics sparklines. Kept in memory for
//! this window only; never exported or written to disk.
use crate::{fx, theme, widgets};
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, pos2, vec2};
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Sample {
    pub at: Instant,
    /// None: no reading for this poll.
    pub value: Option<f32>,
    /// Muted or silenced by another lane's Solo at the time.
    pub silenced: bool,
}

/// Readings for a set of keys over a fixed window.
pub struct Series<K: Ord> {
    window: Duration,
    rows: BTreeMap<K, VecDeque<Sample>>,
}

impl<K: Ord + Copy> Series<K> {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            rows: BTreeMap::new(),
        }
    }

    pub fn window(&self) -> Duration {
        self.window
    }

    pub fn push(&mut self, key: K, sample: Sample) {
        let window = self.window;
        let row = self.rows.entry(key).or_default();
        row.push_back(sample);
        while row
            .front()
            .is_some_and(|s| sample.at.duration_since(s.at) > window)
        {
            row.pop_front();
        }
    }

    /// Drop keys that are no longer present (lane left, identity changed).
    pub fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.rows.retain(|k, _| keep(k));
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    pub fn get(&self, key: &K) -> Option<&VecDeque<Sample>> {
        self.rows.get(key)
    }
}

/// x for a sample: now at the right edge, `window` ago at the left.
fn x_at(rect: Rect, now: Instant, at: Instant, window: Duration) -> f32 {
    let age = now.saturating_duration_since(at).as_secs_f32() / window.as_secs_f32();
    rect.right() - rect.width() * age.clamp(0.0, 1.0)
}

/// Filled level trace in dBFS between the meter floor and 0. Silenced spans
/// are hatched; gaps with no reading stay empty.
pub fn paint_levels(
    painter: &egui::Painter,
    rect: Rect,
    samples: &VecDeque<Sample>,
    window: Duration,
    color: Color32,
) {
    let now = Instant::now();
    painter.rect_filled(rect, CornerRadius::same(4), theme::meter_track());
    let y = |db: f32| {
        let t = ((db - widgets::METER_FLOOR_DB) / -widgets::METER_FLOOR_DB).clamp(0.0, 1.0);
        rect.bottom() - rect.height() * t
    };
    let mut run: Vec<Pos2> = Vec::new();
    let flush = |run: &mut Vec<Pos2>| {
        if run.len() >= 2 {
            // Columns of thin quads: the trace is not convex in general.
            for pair in run.windows(2) {
                let quad = vec![
                    pair[0],
                    pair[1],
                    pos2(pair[1].x, rect.bottom()),
                    pos2(pair[0].x, rect.bottom()),
                ];
                painter.add(egui::Shape::convex_polygon(
                    quad,
                    color.gamma_multiply(0.22),
                    Stroke::NONE,
                ));
            }
            painter.add(egui::Shape::line(run.clone(), Stroke::new(1.2, color)));
        }
        run.clear();
    };
    let mut silenced_from: Option<f32> = None;
    for s in samples {
        let x = x_at(rect, now, s.at, window);
        match (s.silenced, silenced_from) {
            (true, None) => silenced_from = Some(x),
            (false, Some(from)) => {
                fx::hatch(
                    painter,
                    Rect::from_x_y_ranges(from..=x, rect.y_range()),
                    theme::warning().gamma_multiply(0.25),
                    5.0,
                );
                silenced_from = None;
            }
            _ => {}
        }
        match s.value {
            Some(db) => run.push(pos2(x, y(db))),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    if let Some(from) = silenced_from {
        fx::hatch(
            painter,
            Rect::from_x_y_ranges(from..=rect.right(), rect.y_range()),
            theme::warning().gamma_multiply(0.25),
            5.0,
        );
    }
}

/// Sparkline of a non-negative metric scaled to its own maximum. With
/// `delta`, samples are running totals and the line shows each increase.
pub fn paint_spark(
    painter: &egui::Painter,
    rect: Rect,
    samples: &VecDeque<Sample>,
    window: Duration,
    color: Color32,
    delta: bool,
) {
    let now = Instant::now();
    let values: Vec<(Instant, Option<f32>)> = if delta {
        samples
            .iter()
            .zip(samples.iter().skip(1))
            .map(|(a, b)| (b.at, a.value.zip(b.value).map(|(a, b)| (b - a).max(0.0))))
            .collect()
    } else {
        samples.iter().map(|s| (s.at, s.value)).collect()
    };
    let max = values
        .iter()
        .filter_map(|s| s.1)
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let points: Vec<Pos2> = values
        .iter()
        .filter_map(|&(at, v)| {
            let v = v?;
            Some(pos2(
                x_at(rect, now, at, window),
                rect.bottom() - rect.height() * (v / max).clamp(0.0, 1.0),
            ))
        })
        .collect();
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::border()),
    );
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.4, color)));
    } else if let Some(p) = points.first() {
        painter.circle_filled(*p, 1.8, color);
    }
}

/// Fixed-height row allocation helper for ribbons.
pub fn row_rect(ui: &mut egui::Ui, height: f32) -> Rect {
    ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover())
        .0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_keeps_only_the_window_and_dropped_keys() {
        let mut series = Series::new(Duration::from_secs(60));
        let start = Instant::now();
        for i in 0..300u64 {
            series.push(
                1u64,
                Sample {
                    at: start + Duration::from_millis(250 * i),
                    value: Some(-20.0),
                    silenced: false,
                },
            );
        }
        let row = series.get(&1).unwrap();
        assert!(row.len() <= 241, "{}", row.len());
        series.push(
            2,
            Sample {
                at: start,
                value: None,
                silenced: true,
            },
        );
        series.retain(|k| *k == 2);
        assert!(series.get(&1).is_none());
        assert!(series.get(&2).is_some());
    }
}
