//! Line icons painted from primitives (no icon font dependency). Each icon
//! fits a square `rect` and uses a single stroke colour.
use eframe::egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2, vec2};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Flow,
    Mixer,
    Room,
    Sender,
    Devices,
    Pulse,
    Info,
    Search,
    Check,
    AirPlay,
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0;
    let stroke = Stroke::new((1.6 * u).max(1.2), color);
    let p = |x: f32, y: f32| c + vec2((x - 8.0) * u, (y - 8.0) * u);
    match icon {
        Icon::Flow => {
            // Three sources converging on one node.
            for y in [3.5, 8.0, 12.5] {
                painter.circle_filled(p(2.5, y), 1.5 * u, color);
                painter.add(eframe::egui::Shape::CubicBezier(
                    eframe::egui::epaint::CubicBezierShape::from_points_stroke(
                        [p(4.0, y), p(8.0, y), p(8.0, 8.0), p(10.5, 8.0)],
                        false,
                        eframe::egui::Color32::TRANSPARENT,
                        stroke,
                    ),
                ));
            }
            painter.circle_stroke(p(12.5, 8.0), 2.4 * u, stroke);
        }
        Icon::Mixer => {
            for (x, knob) in [(4.0, 10.0), (8.0, 5.0), (12.0, 8.0)] {
                painter.line_segment([p(x, 2.0), p(x, 14.0)], stroke);
                painter.circle_filled(p(x, knob), 1.9 * u, color);
            }
        }
        Icon::Room => {
            // Speaker cabinet with woofer and tweeter.
            painter.rect_stroke(
                Rect::from_min_max(p(3.5, 1.5), p(12.5, 14.5)),
                2.0 * u,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
            painter.circle_stroke(p(8.0, 10.0), 2.6 * u, stroke);
            painter.circle_filled(p(8.0, 4.8), 1.1 * u, color);
        }
        Icon::Sender => {
            painter.circle_filled(p(8.0, 11.0), 1.6 * u, color);
            arc(painter, p(8.0, 11.0), 4.2 * u, stroke);
            arc(painter, p(8.0, 11.0), 7.4 * u, stroke);
        }
        Icon::Devices => {
            painter.circle_stroke(p(6.0, 5.5), 2.5 * u, stroke);
            painter.circle_stroke(p(11.5, 6.5), 2.0 * u, stroke);
            half(painter, p(6.0, 14.5), 4.6 * u, stroke);
            half(painter, p(11.5, 14.5), 3.6 * u, stroke);
        }
        Icon::Pulse => {
            let points = [
                p(1.0, 9.0),
                p(4.5, 9.0),
                p(6.5, 3.5),
                p(9.5, 13.0),
                p(11.5, 7.0),
                p(15.0, 7.0),
            ];
            painter.add(eframe::egui::Shape::line(points.to_vec(), stroke));
        }
        Icon::Info => {
            painter.circle_stroke(p(8.0, 8.0), 6.4 * u, stroke);
            painter.line_segment([p(8.0, 7.2), p(8.0, 11.4)], stroke);
            painter.circle_filled(p(8.0, 4.9), 0.95 * u, color);
        }
        Icon::Search => {
            painter.circle_stroke(p(7.0, 7.0), 4.4 * u, stroke);
            painter.line_segment([p(10.4, 10.4), p(14.0, 14.0)], stroke);
        }
        Icon::Check => {
            painter.add(eframe::egui::Shape::line(
                vec![p(3.0, 8.5), p(6.5, 12.0), p(13.0, 4.5)],
                Stroke::new(stroke.width * 1.2, color),
            ));
        }
        Icon::AirPlay => {
            painter.rect_stroke(
                Rect::from_min_max(p(1.5, 2.0), p(14.5, 11.0)),
                1.5 * u,
                stroke,
                eframe::egui::StrokeKind::Middle,
            );
            painter.add(eframe::egui::Shape::convex_polygon(
                vec![p(8.0, 9.0), p(12.0, 14.5), p(4.0, 14.5)],
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// Rotated chevron for disclosure: `t` 0 points right, 1 points down.
pub fn chevron(painter: &Painter, rect: Rect, t: f32, color: Color32) {
    let angle = t * std::f32::consts::FRAC_PI_2;
    let c = rect.center();
    let u = rect.width().min(rect.height()) / 16.0;
    let rot = |v: Vec2| {
        let (s, k) = angle.sin_cos();
        c + vec2(v.x * k - v.y * s, v.x * s + v.y * k)
    };
    let points = vec![
        rot(vec2(-2.0, -4.5) * u),
        rot(vec2(2.5, 0.0) * u),
        rot(vec2(-2.0, 4.5) * u),
    ];
    painter.add(eframe::egui::Shape::line(
        points,
        Stroke::new(1.6 * u, color),
    ));
}

fn arc(painter: &Painter, center: Pos2, r: f32, stroke: Stroke) {
    let points: Vec<Pos2> = (0..=14)
        .map(|i| {
            let a = std::f32::consts::PI * (1.18 + 0.64 * i as f32 / 14.0);
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(eframe::egui::Shape::line(points, stroke));
}

fn half(painter: &Painter, base: Pos2, r: f32, stroke: Stroke) {
    let points: Vec<Pos2> = (0..=14)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 14.0);
            base + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(eframe::egui::Shape::line(points, stroke));
}

pub fn square(center: Pos2, size: f32) -> Rect {
    Rect::from_center_size(center, vec2(size, size))
}
