//! Room emblem: a small glyph derived from the Hub UUID, so two rooms with
//! the same name still look different. Identity, not decoration: the same
//! room always gets the same emblem, and no room gets one before it exists.
use crate::{fx, theme};
use eframe::egui::{Color32, Painter, Pos2, Shape, Stroke, Vec2, ecolor::Hsva, vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emblem {
    pub hues: [f32; 3],
    pub pattern: u8,
}

impl Emblem {
    /// Hues stay in the cyan→violet band so they never read as a state colour.
    pub fn from_id(id: uuid::Uuid) -> Self {
        let b = id.as_bytes();
        let hue = |i: usize| {
            (0.49 + 0.30 * (u16::from_le_bytes([b[i], b[i + 1]]) as f32 / 65535.0)) % 1.0
        };
        Self {
            hues: [hue(0), hue(4), hue(8)],
            pattern: b[15] % 6,
        }
    }

    pub fn color(&self, i: usize, value: f32) -> Color32 {
        Hsva::new(self.hues[i % 3], 0.62, value, 1.0).into()
    }
}

/// Paint inside a circle of `radius`. `lit` 0…1 dims an idle room.
pub fn paint(painter: &Painter, center: Pos2, radius: f32, emblem: Emblem, lit: f32) {
    let v = 0.55 + 0.45 * lit;
    let a = emblem.color(0, v);
    let b = emblem.color(1, v);
    let c = emblem.color(2, v);
    painter.circle_filled(center, radius, a.gamma_multiply(0.16));
    let w = (radius / 11.0).max(1.4);
    let s = |color: Color32| Stroke::new(w, color);
    let r = radius * 0.78;
    match emblem.pattern {
        0 => {
            for (k, color) in [(1.0, a), (0.66, b), (0.33, c)] {
                painter.circle_stroke(center, r * k, s(color));
            }
        }
        1 => {
            for i in 0..6 {
                let a0 = i as f32 * std::f32::consts::TAU / 6.0;
                let color = [a, b, c][i % 3];
                fx::arc(
                    painter,
                    center,
                    r * 0.72,
                    a0,
                    a0 + 0.8,
                    Stroke::new(w * 2.2, color),
                );
            }
            painter.circle_filled(center, r * 0.22, b);
        }
        2 => {
            for (k, color) in [(1.0, a), (0.62, b)] {
                let d = r * k;
                let pts = vec![
                    center + vec2(0.0, -d),
                    center + vec2(d, 0.0),
                    center + vec2(0.0, d),
                    center + vec2(-d, 0.0),
                ];
                painter.add(Shape::closed_line(pts, s(color)));
            }
            painter.circle_filled(center, r * 0.16, c);
        }
        3 => {
            let tri = |rot: f32, k: f32| -> Vec<Pos2> {
                (0..3)
                    .map(|i| {
                        let ang = rot + i as f32 * std::f32::consts::TAU / 3.0;
                        center + Vec2::angled(ang) * r * k
                    })
                    .collect()
            };
            painter.add(Shape::closed_line(
                tri(-std::f32::consts::FRAC_PI_2, 0.95),
                s(a),
            ));
            painter.add(Shape::closed_line(
                tri(std::f32::consts::FRAC_PI_2, 0.55),
                s(b),
            ));
        }
        4 => {
            painter.circle_filled(center, r * 0.2, a);
            for i in 0..6 {
                let p = center + Vec2::angled(i as f32 * std::f32::consts::TAU / 6.0) * r * 0.72;
                painter.circle_filled(p, r * 0.13, [b, c][i % 2]);
            }
        }
        _ => {
            for (i, h) in [0.45, 0.85, 0.65, 1.0, 0.5].into_iter().enumerate() {
                let x = center.x + (i as f32 - 2.0) * r * 0.36;
                let half = r * 0.75 * h;
                painter.line_segment(
                    [Pos2::new(x, center.y - half), Pos2::new(x, center.y + half)],
                    Stroke::new(w * 1.6, [a, b, c][i % 3]),
                );
            }
        }
    }
    painter.circle_stroke(center, radius, Stroke::new(1.0, theme::border_strong()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_room_same_emblem_and_rooms_differ() {
        let a = uuid::Uuid::parse_str("13ebafde-12c1-4fc4-b91d-d0eed7bdbc9a").unwrap();
        let b = uuid::Uuid::parse_str("5818dd9d-bb5c-45ee-aaef-4800ccca4b53").unwrap();
        assert_eq!(Emblem::from_id(a), Emblem::from_id(a));
        assert_ne!(Emblem::from_id(a), Emblem::from_id(b));
        for hue in Emblem::from_id(a).hues {
            assert!(
                (0.49..=0.79).contains(&hue),
                "hue {hue} outside cyan–violet"
            );
        }
    }
}
