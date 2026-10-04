//! Painting helpers for the Neon Console look: surface gradients, glows,
//! hatching for silenced areas, curves and LED segments. Pure painting; no
//! state, no repaint requests.
use crate::theme;
use eframe::egui::{
    self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureId, Vec2,
    epaint::RectShape, pos2, vec2,
};

/// Vertical white→grey ramp; multiplied with a fill colour it gives raised
/// surfaces a lit top edge without a second shape.
fn ramp(ctx: &egui::Context) -> TextureId {
    let id = egui::Id::new("fx-ramp");
    if let Some(handle) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return handle.id();
    }
    let pixels = (0..64)
        .map(|y| {
            let t = y as f32 / 63.0;
            let v = (255.0 - 52.0 * t).round() as u8;
            Color32::from_rgb(v, v, v)
        })
        .collect();
    let image = egui::ColorImage {
        size: [1, 64],
        pixels,
    };
    let handle = ctx.load_texture("fx-ramp", image, egui::TextureOptions::LINEAR);
    let tex = handle.id();
    ctx.data_mut(|d| d.insert_temp(id, handle));
    tex
}

/// Lighten towards white by `t`.
pub fn lift(color: Color32, t: f32) -> Color32 {
    crate::animation::lerp_color(color, Color32::WHITE, t)
}

/// Card background: lit gradient, hairline border, top highlight and an
/// optional inner glow in the top-left corner that names the card's state.
pub fn surface(
    ctx: &egui::Context,
    rect: Rect,
    radius: u8,
    base: Color32,
    glow: Option<Color32>,
) -> Vec<Shape> {
    let r = CornerRadius::same(radius);
    let mut shapes = vec![Shape::Rect(
        RectShape::filled(rect, r, lift(base, 0.04)).with_texture(
            ramp(ctx),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        ),
    )];
    if let Some(color) = glow {
        let size = (rect.height() * 0.9).clamp(60.0, 180.0);
        let spot = Rect::from_min_size(rect.min + vec2(18.0, 14.0), vec2(size * 1.6, size * 0.7));
        shapes.push(Shape::Rect(
            RectShape::filled(spot, CornerRadius::same(255), color.gamma_multiply(0.07))
                .with_blur_width(size * 0.55),
        ));
    }
    shapes.push(Shape::Rect(RectShape::stroke(
        rect,
        r,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Inside,
    )));
    let inset = radius as f32;
    shapes.push(Shape::line_segment(
        [
            pos2(rect.left() + inset, rect.top() + 1.0),
            pos2(rect.right() - inset, rect.top() + 1.0),
        ],
        Stroke::new(1.0, theme::EDGE_LIGHT),
    ));
    shapes
}

/// Soft round glow. Blur is done by the tessellator, so this is one shape.
pub fn glow(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    if radius <= 0.5 || color.a() == 0 {
        return;
    }
    // Blur close to the shape's own size folds back on itself and leaves a
    // seam; keep it to two thirds of the core.
    let size = radius * 1.2;
    painter.add(Shape::Rect(
        RectShape::filled(
            Rect::from_center_size(center, vec2(size, size)),
            CornerRadius::same((size / 2.0).min(255.0) as u8),
            color,
        )
        .with_blur_width(size * 0.66),
    ));
}

/// Diagonal hatching clipped to `rect` (muted lanes, revoked devices).
pub fn hatch(painter: &Painter, rect: Rect, color: Color32, spacing: f32) {
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let stroke = Stroke::new(1.0, color);
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        clipped.line_segment(
            [pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())],
            stroke,
        );
        x += spacing;
    }
}

/// Cubic Bézier point at `t`.
pub fn cubic_at(c: [Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let p = c[0].to_vec2() * (u * u * u)
        + c[1].to_vec2() * (3.0 * u * u * t)
        + c[2].to_vec2() * (3.0 * u * t * t)
        + c[3].to_vec2() * (t * t * t);
    p.to_pos2()
}

/// Polyline approximation of a cubic Bézier between `t0` and `t1`.
pub fn cubic_points(c: [Pos2; 4], t0: f32, t1: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| cubic_at(c, t0 + (t1 - t0) * i as f32 / steps as f32))
        .collect()
}

/// Horizontal S-curve from `a` to `b` (control points pulled sideways).
pub fn s_curve(a: Pos2, b: Pos2) -> [Pos2; 4] {
    let dx = (b.x - a.x) * 0.5;
    [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b]
}

/// Vertical S-curve from `a` to `b` (control points pulled up/down).
pub fn s_curve_vertical(a: Pos2, b: Pos2) -> [Pos2; 4] {
    let dy = (b.y - a.y) * 0.5;
    [a, a + vec2(0.0, dy), b - vec2(0.0, dy), b]
}

/// Dashes along a polyline.
pub fn dashed(painter: &Painter, points: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    painter.extend(Shape::dashed_line(points, stroke, dash, gap));
}

/// Ring of `segments` LED cells starting at 12 o'clock, clockwise. The first
/// `lit` cells are painted with `color_at(i)`, the rest as the track.
pub fn led_ring(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    width: f32,
    segments: usize,
    lit: f32,
    color_at: impl Fn(usize) -> Color32,
) {
    let gap = 0.22;
    let step = std::f32::consts::TAU / segments as f32;
    for i in 0..segments {
        let a0 = -std::f32::consts::FRAC_PI_2 + i as f32 * step + gap * step / 2.0;
        let a1 = a0 + step * (1.0 - gap);
        let on = (lit - i as f32).clamp(0.0, 1.0);
        let color = if on > 0.0 {
            crate::animation::lerp_color(theme::METER_TRACK, color_at(i), on)
        } else {
            theme::BORDER
        };
        arc(painter, center, radius, a0, a1, Stroke::new(width, color));
    }
}

/// Arc from angle `a0` to `a1` (radians, 0 = 3 o'clock, clockwise).
pub fn arc(painter: &Painter, center: Pos2, radius: f32, a0: f32, a1: f32, stroke: Stroke) {
    let steps = (((a1 - a0).abs() * radius / 3.0).ceil() as usize).clamp(2, 64);
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            center + Vec2::angled(a) * radius
        })
        .collect();
    painter.add(Shape::line(points, stroke));
}

/// Meter zone colour for a level in dBFS.
pub fn zone(db: f32) -> Color32 {
    if db > -6.0 {
        theme::METER_HIGH
    } else if db > -18.0 {
        theme::METER_MID
    } else {
        theme::METER_LOW
    }
}
