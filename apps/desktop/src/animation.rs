//! Motion that explains state changes. Nothing here animates while idle:
//! every helper settles and stops requesting repaints.
use eframe::egui::{self, Color32, Id};
use std::time::Duration;

pub const FAST: f32 = 0.12;
pub const PAGE: f32 = 0.16;

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = 1.0 - t.clamp(0.0, 1.0);
    1.0 - t * t * t
}

pub fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

/// Colour that eases to `target` when a state changes (pill, lane rail).
pub fn color(ctx: &egui::Context, id: Id, target: Color32) -> Color32 {
    let [r, g, b, a] = target.to_array();
    let channel = |salt: &str, v: u8| {
        ctx.animate_value_with_time(id.with(salt), v as f32, 0.2)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_premultiplied(
        channel("r", r),
        channel("g", g),
        channel("b", b),
        channel("a", a),
    )
}

/// Hover/press easing for a widget whose id is known before it is added.
/// Reads last frame's response so the colour is ready when painting.
pub fn interaction(ctx: &egui::Context, id: Id) -> (f32, f32) {
    let previous = ctx.read_response(id);
    let hovered = previous
        .as_ref()
        .is_some_and(|r| r.hovered() || r.has_focus());
    let pressed = previous
        .as_ref()
        .is_some_and(|r| r.is_pointer_button_down_on());
    (
        ctx.animate_bool_with_time(id.with("hover"), hovered, FAST),
        ctx.animate_bool_with_time(id.with("press"), pressed, 0.06),
    )
}

/// Opacity for content that just appeared (page switch, new message).
pub fn fade_in(ctx: &egui::Context, since: Duration, length: f32) -> f32 {
    let t = since.as_secs_f32() / length;
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out_cubic(t)
}

/// Meter ballistics in dB: instant attack, 24 dB/s release, and a peak-hold
/// marker that stays 1.2 s before falling. Stored per meter in egui memory.
#[derive(Clone, Copy)]
struct Ballistics {
    level: f32,
    hold: f32,
    hold_at: f64,
    at: f64,
}

pub fn meter(ctx: &egui::Context, id: Id, target_db: f32) -> (f32, f32) {
    let now = ctx.input(|i| i.time);
    let mut state = ctx
        .data(|d| d.get_temp::<Ballistics>(id))
        .unwrap_or(Ballistics {
            level: target_db,
            hold: target_db,
            hold_at: now,
            at: now,
        });
    let dt = (now - state.at).clamp(0.0, 0.25) as f32;
    state.at = now;
    state.level = if target_db >= state.level {
        target_db
    } else {
        (state.level - 24.0 * dt).max(target_db)
    };
    if state.level >= state.hold {
        state.hold = state.level;
        state.hold_at = now;
    } else if now - state.hold_at > 1.2 {
        state.hold = (state.hold - 18.0 * dt).max(state.level);
    }
    let moving = state.level > target_db + 0.05 || state.hold > state.level + 0.05;
    ctx.data_mut(|d| d.insert_temp(id, state));
    if moving {
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    (state.level, state.hold)
}
