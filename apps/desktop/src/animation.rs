use eframe::egui;
use std::time::Duration;

/// 动画缓动函数
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t - 1.0;
    t * t * t + 1.0
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let t = t - 1.0;
        1.0 + 4.0 * t * t * t
    }
}

/// 平滑动画值
pub fn animate_bool(ui: &mut egui::Ui, id: impl Into<egui::Id>, target: bool) -> f32 {
    let id = id.into();
    let animation_time = 0.15; // 150ms

    ui.ctx().animate_bool_with_time(id, target, animation_time)
}

/// 带自定义时长的动画
pub fn animate_value(
    ui: &mut egui::Ui,
    id: impl Into<egui::Id>,
    target: f32,
    duration: Duration,
) -> f32 {
    let id = id.into();
    let current = ui
        .ctx()
        .animate_value_with_time(id, target, duration.as_secs_f32());
    current
}

/// 颜色插值
pub fn lerp_color(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from_rgba_premultiplied(
        lerp_u8(from.r(), to.r(), t),
        lerp_u8(from.g(), to.g(), t),
        lerp_u8(from.b(), to.b(), t),
        lerp_u8(from.a(), to.a(), t),
    )
}

fn lerp_u8(from: u8, to: u8, t: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * t) as u8
}

/// 脉冲动画（用于状态指示器）
pub fn pulse_animation(ui: &egui::Context, speed: f32) -> f32 {
    let time = ui.input(|i| i.time);
    ((time * speed as f64).sin() * 0.5 + 0.5) as f32
}

/// 平滑滚动效果
pub fn smooth_scroll_offset(ui: &mut egui::Ui, id: impl Into<egui::Id>, target: f32) -> f32 {
    animate_value(ui, id, target, Duration::from_millis(250))
}

/// 页面过渡动画
pub fn page_transition(ui: &egui::Ui, page_id: impl std::hash::Hash) -> f32 {
    let id = egui::Id::new(page_id).with("page_transition");
    let animation_time = 0.25; // 250ms

    // 检查页面是否刚切换
    let is_new = ui
        .ctx()
        .memory(|mem| mem.data.get_temp::<bool>(id.with("is_new")).unwrap_or(true));

    if is_new {
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(id.with("is_new"), false);
        });
    }

    let progress = ui.ctx().animate_bool_with_time(id, !is_new, animation_time);
    ease_in_out_cubic(progress)
}

/// 重置页面过渡（当切换页面时调用）
pub fn reset_page_transition(ctx: &egui::Context, page_id: impl std::hash::Hash) {
    let id = egui::Id::new(page_id).with("page_transition");
    ctx.memory_mut(|mem| {
        mem.data.insert_temp(id.with("is_new"), true);
    });
}
