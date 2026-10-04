//! Motion that explains state changes or follows a live signal. Nothing here
//! animates while idle: transitions settle, one-shots play once, and live
//! motion (`live_phase` + `live_repaint`) only runs while the caller has real
//! data moving (audio above the floor, a request in flight, a countdown).
use eframe::egui::{self, Color32, Id};
use std::cell::Cell;
use std::time::Duration;

// UI state lives on the UI thread; thread-local keeps parallel tests apart.
thread_local! {
    static REDUCE_MOTION: Cell<bool> = const { Cell::new(false) };
}

/// Reduced motion: live motion becomes static, one-shots jump to their end.
pub fn reduce_motion() -> bool {
    REDUCE_MOTION.with(Cell::get)
}

pub fn set_reduce_motion(on: bool) {
    REDUCE_MOTION.with(|c| c.set(on));
}

/// Live motion frame budget: 20 fps focused, 10 fps in the background.
/// On macOS most of a frame's CPU is buffer presentation, so cost follows
/// frame rate rather than what is drawn; 20 fps keeps 4 flowing lanes
/// within the CPU budget recorded in docs/evidence/ui-rebuild-20261004.
pub const LIVE_FRAME: Duration = Duration::from_millis(50);

pub fn live_repaint(ctx: &egui::Context) {
    if reduce_motion() {
        return;
    }
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    ctx.request_repaint_after(if focused { LIVE_FRAME } else { LIVE_FRAME * 2 });
}

/// Clock for painting. Screenshot builds can pin it with
/// `NEONMIX_SCREENSHOT_PHASE=<seconds>` so captures are reproducible.
pub fn now(ctx: &egui::Context) -> f64 {
    #[cfg(feature = "screenshot")]
    if let Some(t) = std::env::var("NEONMIX_SCREENSHOT_PHASE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        return t;
    }
    ctx.input(|i| i.time)
}

#[derive(Clone, Copy)]
struct Phase {
    value: f64,
    at: f64,
}

/// Phase in seconds that advances only while `active`, so live motion
/// resumes where it stopped instead of jumping. Requests the live frame
/// budget while active.
pub fn live_phase(ctx: &egui::Context, id: Id, active: bool) -> f32 {
    let now = now(ctx);
    let mut phase = ctx.data(|d| d.get_temp::<Phase>(id)).unwrap_or(Phase {
        value: 0.0,
        at: now,
    });
    let dt = (now - phase.at).clamp(0.0, 0.1);
    phase.at = now;
    if active && !reduce_motion() {
        phase.value += dt;
        live_repaint(ctx);
    }
    ctx.data_mut(|d| d.insert_temp(id, phase));
    #[cfg(feature = "screenshot")]
    if std::env::var_os("NEONMIX_SCREENSHOT_PHASE").is_some() {
        return now as f32;
    }
    phase.value as f32
}

#[derive(Clone, Copy)]
struct Ease {
    from: f32,
    to: f32,
    start: f64,
}

/// Value that eases (cubic out) to `target` over `time` seconds whenever the
/// target changes. Used for positions and numeric readouts.
pub fn ease_to(ctx: &egui::Context, id: Id, target: f32, time: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let state = ctx.data(|d| d.get_temp::<Ease>(id));
    let current = |e: &Ease| {
        let t = ((now - e.start) as f32 / time).clamp(0.0, 1.0);
        e.from + (e.to - e.from) * ease_out_cubic(t)
    };
    let next = match state {
        None => Ease {
            from: target,
            to: target,
            start: now,
        },
        Some(e) if (e.to - target).abs() > f32::EPSILON => Ease {
            from: if reduce_motion() { target } else { current(&e) },
            to: target,
            start: now,
        },
        Some(e) => e,
    };
    ctx.data_mut(|d| d.insert_temp(id, next));
    let value = current(&next);
    if (value - next.to).abs() > f32::EPSILON {
        ctx.request_repaint();
    }
    value
}

#[derive(Clone, Copy)]
struct Once {
    key: u64,
    start: f64,
}

/// One-shot progress 0→1 after `key` changes (not on first sight). Returns
/// None when nothing is playing.
pub fn once(ctx: &egui::Context, id: Id, key: u64, length: f32) -> Option<f32> {
    let now = ctx.input(|i| i.time);
    let state = ctx.data(|d| d.get_temp::<Once>(id));
    let state = match state {
        Some(s) if s.key == key => s,
        Some(_) => Once { key, start: now },
        None => Once {
            key,
            start: f64::NEG_INFINITY,
        },
    };
    ctx.data_mut(|d| d.insert_temp(id, state));
    if reduce_motion() {
        return None;
    }
    let t = ((now - state.start) as f32 / length).max(0.0);
    (t < 1.0).then(|| {
        ctx.request_repaint();
        t
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn delay(ctx: &egui::Context, frame: impl FnMut(&egui::Context)) -> Duration {
        let output = ctx.run(Default::default(), frame);
        output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    #[test]
    fn live_motion_repaints_only_while_active_and_not_reduced() {
        let ctx = egui::Context::default();
        let id = Id::new("flow");
        // egui repaints its first passes on its own.
        for _ in 0..3 {
            _ = ctx.run(Default::default(), |_| {});
        }
        assert_eq!(delay(&ctx, |c| _ = live_phase(c, id, false)), Duration::MAX);
        assert!(delay(&ctx, |c| _ = live_phase(c, id, true)) <= LIVE_FRAME);
        set_reduce_motion(true);
        assert_eq!(delay(&ctx, |c| _ = live_phase(c, id, true)), Duration::MAX);
        set_reduce_motion(false);
    }

    #[test]
    fn one_shot_plays_on_change_not_on_first_sight() {
        let ctx = egui::Context::default();
        let id = Id::new("ripple");
        let mut seen = None;
        _ = ctx.run(Default::default(), |c| seen = Some(once(c, id, 1, 0.6)));
        assert_eq!(seen, Some(None));
        _ = ctx.run(Default::default(), |c| seen = Some(once(c, id, 2, 0.6)));
        assert!(matches!(seen, Some(Some(t)) if t < 0.2));
    }
}
