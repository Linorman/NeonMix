use crate::animation;
use crate::theme;
use eframe::egui::{self, CornerRadius, RichText, Stroke, StrokeKind};

pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, secret: bool) {
    let id = ui.make_persistent_id(label);
    ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(label).color(theme::TEXT_SECONDARY).size(13.0));
        ui.add_space(2.0);
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .id(id)
                .password(secret)
                .font(egui::TextStyle::Body)
                .desired_width(ui.available_width().min(480.0))
                .margin(egui::vec2(12.0, 10.0)),
        );
        // 焦点动画
        let focus_anim =
            animation::animate_bool(ui, response.id.with("focus"), response.has_focus());
        if focus_anim > 0.01 {
            ui.painter().rect_stroke(
                response.rect,
                CornerRadius::same(6),
                Stroke::new(1.5 * focus_anim, theme::ACCENT),
                StrokeKind::Outside,
            );
        }
    });
}
pub fn card(ui: &mut egui::Ui, id: impl std::hash::Hash, content: impl FnOnce(&mut egui::Ui)) {
    ui.push_id(id, |ui| {
        let card_id = ui.id();

        // 先预测悬停区域
        let available = ui.available_rect_before_wrap();
        let outer_margin = egui::Margin::symmetric(0, 8);
        let margin_vec = egui::vec2(
            (outer_margin.left + outer_margin.right) as f32,
            (outer_margin.top + outer_margin.bottom) as f32,
        );
        let potential_rect = available.shrink2(margin_vec);

        let hovered = ui.rect_contains_pointer(potential_rect);
        let hover_anim = animation::animate_bool(ui, card_id.with("hover"), hovered);

        // 悬停时边框从深色变为带青色高光
        let border_color = animation::lerp_color(
            theme::SURFACE_3,
            theme::ACCENT.gamma_multiply(0.3),
            hover_anim,
        );

        // 悬停时背景稍微提亮
        let bg_color = animation::lerp_color(theme::SURFACE_2, theme::SURFACE_3, hover_anim * 0.4);

        egui::Frame::new()
            .fill(bg_color)
            .stroke(Stroke::new(1.0, border_color))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(20.0)
            .outer_margin(outer_margin)
            .shadow(egui::epaint::Shadow::NONE)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                content(ui);
            });
    });
}

pub fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(title)
            .size(18.0)
            .color(theme::TEXT_PRIMARY)
            .strong(),
    );
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);
    content(ui);
    ui.add_space(12.0);
}
pub fn meter(ui: &mut egui::Ui, peak: Option<f64>, rms: Option<f64>) {
    let peak_db = peak.map(|p| db_value(p));
    let rms_db = rms.map(|r| db_value(r));

    ui.horizontal(|ui| {
        ui.label(RichText::new("峰值").size(12.0).color(theme::TEXT_MUTED));

        let peak_fraction = peak.map_or(0.0, |p| {
            ((20.0 * p.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
        });

        let peak_color = if peak_fraction > 0.95 {
            theme::METER_DANGER
        } else if peak_fraction > 0.8 {
            theme::METER_CAUTION
        } else {
            theme::TEXT_PRIMARY
        };

        ui.label(
            RichText::new(peak_db.as_deref().unwrap_or("—"))
                .size(13.0)
                .color(peak_color)
                .monospace(),
        );
        ui.add_space(16.0);
        ui.label(RichText::new("RMS").size(12.0).color(theme::TEXT_MUTED));
        ui.label(
            RichText::new(rms_db.as_deref().unwrap_or("—"))
                .size(13.0)
                .color(theme::TEXT_PRIMARY)
                .monospace(),
        );
    });

    let target_fraction = peak.map_or(0.0, |p| {
        ((20.0 * p.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
    }) as f32;

    // 平滑动画
    let meter_id = ui.id().with("meter");
    let fraction = animation::animate_value(
        ui,
        meter_id,
        target_fraction,
        std::time::Duration::from_millis(80),
    );

    ui.add_space(8.0);
    let bar_height = 16.0;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(640.0), bar_height),
        egui::Sense::hover(),
    );

    // 背景
    ui.painter()
        .rect_filled(rect, CornerRadius::same(8), theme::SURFACE_0);

    // 填充 - 使用渐变效果
    if fraction > 0.0 {
        let fill_width = rect.width() * fraction;
        let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, bar_height));

        // 根据电平选择颜色
        let color = if fraction > 0.95 {
            theme::METER_DANGER
        } else if fraction > 0.8 {
            theme::METER_CAUTION
        } else {
            theme::METER_SAFE
        };

        ui.painter()
            .rect_filled(fill_rect, CornerRadius::same(8), color);

        // 添加高光效果
        let highlight_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, bar_height * 0.4));
        ui.painter().rect_filled(
            highlight_rect,
            CornerRadius::same(8),
            color.gamma_multiply(1.3),
        );
    }

    // 边框
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, theme::SURFACE_3),
        StrokeKind::Inside,
    );
}

fn db_value(value: f64) -> String {
    if value > 1e-6 {
        format!("{:.1} dBFS", 20.0 * value.log10())
    } else {
        "−∞ dBFS".into()
    }
}
pub fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).color(theme::TEXT_MUTED).size(12.5));
}

pub fn status_badge(ui: &mut egui::Ui, text: &str, status: BadgeStatus) {
    let (bg, fg) = match status {
        BadgeStatus::Success => (theme::SUCCESS.gamma_multiply(0.2), theme::SUCCESS),
        BadgeStatus::Warning => (theme::WARNING.gamma_multiply(0.2), theme::WARNING),
        BadgeStatus::Danger => (theme::DANGER.gamma_multiply(0.2), theme::DANGER),
        BadgeStatus::Info => (theme::INFO.gamma_multiply(0.2), theme::INFO),
        BadgeStatus::Neutral => (theme::SURFACE_3, theme::TEXT_SECONDARY),
    };

    // 成功状态添加脉冲动画
    let pulse = if matches!(status, BadgeStatus::Success) {
        animation::pulse_animation(ui.ctx(), 2.0) * 0.15 + 0.85
    } else {
        1.0
    };

    let animated_bg = egui::Color32::from_rgba_premultiplied(
        (bg.r() as f32 * pulse) as u8,
        (bg.g() as f32 * pulse) as u8,
        (bg.b() as f32 * pulse) as u8,
        bg.a(),
    );

    egui::Frame::new()
        .fill(animated_bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(fg).size(12.0));
        });
}

pub enum BadgeStatus {
    Success,
    Warning,
    Danger,
    Info,
    Neutral,
}

/// 状态指示器 - 显示简洁的在线/离线状态
pub fn status_indicator(ui: &mut egui::Ui, active: bool, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        // 状态圆点
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());

        let color = if active {
            theme::SUCCESS
        } else {
            theme::TEXT_MUTED
        };

        // 活动状态添加脉冲动画
        let pulse = if active {
            animation::pulse_animation(ui.ctx(), 2.0) * 0.3 + 0.7
        } else {
            1.0
        };

        let animated_color = egui::Color32::from_rgba_premultiplied(
            (color.r() as f32 * pulse) as u8,
            (color.g() as f32 * pulse) as u8,
            (color.b() as f32 * pulse) as u8,
            color.a(),
        );

        ui.painter()
            .circle_filled(rect.center(), 4.0, animated_color);

        // 标签
        ui.label(RichText::new(label).size(13.0).color(if active {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_MUTED
        }));
    });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let id = ui.id().with(text);

    // 悬停动画
    let is_hovered = ui.ctx().memory(|mem| {
        mem.data
            .get_temp::<bool>(id.with("hovered"))
            .unwrap_or(false)
    });
    let hover_anim = animation::animate_bool(ui, id.with("hover"), is_hovered);

    // 点击动画
    let is_pressed = ui.ctx().memory(|mem| {
        mem.data
            .get_temp::<bool>(id.with("pressed"))
            .unwrap_or(false)
    });
    let press_anim = animation::animate_bool(ui, id.with("press"), is_pressed);

    // 颜色变化：悬停时变亮，按下时变暗
    let base_color = theme::ACCENT;
    let hover_color = animation::lerp_color(
        base_color,
        egui::Color32::from_rgb(0x4d, 0xcb, 0xfa), // 更亮的青色
        hover_anim,
    );
    let final_color = animation::lerp_color(
        hover_color,
        egui::Color32::from_rgb(0x2a, 0xa0, 0xd8), // 更暗的青色
        press_anim,
    );

    let response = ui.add(
        egui::Button::new(RichText::new(text).color(theme::SURFACE_0).size(15.0))
            .fill(final_color)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(255))
            .min_size(egui::vec2(0.0, 38.0)),
    );

    // 更新状态
    ui.ctx().memory_mut(|mem| {
        mem.data.insert_temp(id.with("hovered"), response.hovered());
        mem.data
            .insert_temp(id.with("pressed"), response.is_pointer_button_down_on());
    });

    // 点击时请求重绘以触发动画
    if response.clicked() || response.is_pointer_button_down_on() {
        ui.ctx().request_repaint();
    }

    response
}

pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let id = ui.id().with(text);

    let is_hovered = ui.ctx().memory(|mem| {
        mem.data
            .get_temp::<bool>(id.with("hovered"))
            .unwrap_or(false)
    });
    let hover_anim = animation::animate_bool(ui, id.with("hover"), is_hovered);

    // 点击动画
    let is_pressed = ui.ctx().memory(|mem| {
        mem.data
            .get_temp::<bool>(id.with("pressed"))
            .unwrap_or(false)
    });
    let press_anim = animation::animate_bool(ui, id.with("press"), is_pressed);

    // 悬停时背景和边框变化
    let bg_color = animation::lerp_color(theme::SURFACE_3, theme::SURFACE_4, hover_anim);
    let final_bg = animation::lerp_color(bg_color, theme::SURFACE_2, press_anim);

    let border_color = animation::lerp_color(
        theme::SURFACE_4,
        theme::ACCENT.gamma_multiply(0.5),
        hover_anim,
    );

    let response = ui.add(
        egui::Button::new(RichText::new(text).size(15.0))
            .fill(final_bg)
            .stroke(Stroke::new(1.0, border_color))
            .corner_radius(CornerRadius::same(255))
            .min_size(egui::vec2(0.0, 38.0)),
    );

    ui.ctx().memory_mut(|mem| {
        mem.data.insert_temp(id.with("hovered"), response.hovered());
        mem.data
            .insert_temp(id.with("pressed"), response.is_pointer_button_down_on());
    });

    if response.clicked() || response.is_pointer_button_down_on() {
        ui.ctx().request_repaint();
    }

    response
}

pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let id = ui.id().with(text);
    let response = ui.add(
        egui::Button::new(RichText::new(text).color(theme::SURFACE_0).size(15.0))
            .fill(theme::DANGER)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(255))
            .min_size(egui::vec2(0.0, 36.0)),
    );

    // 悬停动画
    let hover_anim = animation::animate_bool(ui, id.with("hover"), response.hovered());
    if hover_anim > 0.01 {
        let darkened = egui::Color32::from_rgb(
            (0xdc as f32 * (1.0 - hover_anim * 0.1)) as u8,
            (0x26 as f32 * (1.0 - hover_anim * 0.1)) as u8,
            (0x26 as f32 * (1.0 - hover_anim * 0.1)) as u8,
        );
        ui.painter()
            .rect_filled(response.rect, CornerRadius::same(255), darkened);
    }

    response
}

pub fn label_combo(response: &egui::Response, label: &str) {
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, response.enabled(), label)
    });
}

pub fn select_value<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    label: impl Into<egui::WidgetText>,
) {
    if ui.selectable_value(current, value, label).clicked() {
        ui.close_menu();
    }
}

pub fn gain_slider(ui: &mut egui::Ui, value: &mut f32, label: &str) -> egui::Response {
    ui.vertical(|ui| {
        ui.add_space(4.0);

        let slider_id = ui.id().with(label);

        // 检查是否正在拖动
        let is_dragging = ui.ctx().memory(|mem| {
            mem.data
                .get_temp::<bool>(slider_id.with("dragging"))
                .unwrap_or(false)
        });

        // 标签和当前值
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(theme::TEXT_SECONDARY).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 根据音量值显示不同颜色
                let value_color = if *value < -60.0 {
                    theme::TEXT_MUTED
                } else if *value > 6.0 {
                    theme::DANGER
                } else if *value > 0.0 {
                    theme::WARNING
                } else {
                    theme::TEXT_PRIMARY
                };

                // 拖动时放大数值显示
                let value_size = if is_dragging { 15.0 } else { 13.0 };

                ui.label(
                    RichText::new(format!("{:.1} dB", value))
                        .color(value_color)
                        .monospace()
                        .size(value_size),
                );
            });
        });
        ui.add_space(8.0);

        // 自定义滑块样式
        let slider_response = ui.add(
            egui::Slider::new(value, -96.0..=12.0)
                .show_value(false)
                .step_by(0.5),
        );

        // 记录拖动状态
        ui.ctx().memory_mut(|mem| {
            mem.data
                .insert_temp(slider_id.with("dragging"), slider_response.dragged());
        });

        // 悬停或拖动时高亮滑块
        let interact_anim = animation::animate_bool(
            ui,
            slider_id.with("interact"),
            slider_response.hovered() || slider_response.dragged(),
        );

        if interact_anim > 0.01 {
            ui.painter().rect_stroke(
                slider_response.rect.expand(2.0),
                CornerRadius::same(6),
                Stroke::new(
                    1.5 * interact_anim,
                    theme::ACCENT.linear_multiply(interact_anim),
                ),
                StrokeKind::Outside,
            );
        }

        // 添加刻度标记
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("-96").size(10.0).color(theme::TEXT_MUTED));
            ui.add_space(ui.available_width() * 0.25);
            ui.label(RichText::new("-48").size(10.0).color(theme::TEXT_MUTED));
            ui.add_space(ui.available_width() * 0.25);
            ui.label(RichText::new("0").size(10.0).color(theme::ACCENT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("+12").size(10.0).color(theme::DANGER));
            });
        });

        ui.add_space(2.0);
        slider_response
    })
    .inner
}

pub fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let button = egui::Button::new(RichText::new(icon).size(16.0))
        .fill(theme::SURFACE_3)
        .stroke(Stroke::new(1.0, theme::SURFACE_4))
        .corner_radius(CornerRadius::same(255))
        .min_size(egui::vec2(32.0, 32.0));
    let response = ui.add(button);
    response.on_hover_text(tooltip)
}

pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
}

pub fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, description: &str) {
    let _id = ui.id().with("empty_state").with(title);

    // 呼吸动画
    let time = ui.input(|i| i.time);
    let pulse = ((time * 1.5).sin() * 0.5 + 0.5) as f32;
    let icon_alpha = 0.3 + pulse * 0.15;

    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        // 图标带背景圆
        let icon_size = 64.0;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover());

        // 绘制背景圆
        ui.painter().circle(
            rect.center(),
            icon_size / 2.0,
            theme::SURFACE_3.linear_multiply(0.5 + pulse * 0.2),
            egui::Stroke::new(1.0, theme::SURFACE_4.linear_multiply(icon_alpha)),
        );

        // 绘制图标
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(40.0),
            theme::TEXT_MUTED.linear_multiply(icon_alpha),
        );

        ui.add_space(24.0);
        ui.label(
            RichText::new(title)
                .size(20.0)
                .color(theme::TEXT_PRIMARY)
                .strong(),
        );
        ui.add_space(12.0);

        // 描述文本带最大宽度
        let max_width = ui.available_width().min(400.0);
        ui.set_max_width(max_width);
        ui.label(
            RichText::new(description)
                .size(14.0)
                .color(theme::TEXT_SECONDARY),
        );
    });
    ui.add_space(60.0);

    // 持续重绘以维持动画
    ui.ctx().request_repaint();
}

/// 响应式网格布局
pub fn grid(
    ui: &mut egui::Ui,
    min_card_width: f32,
    spacing: f32,
    content: impl FnOnce(&mut egui::Ui, usize),
) {
    let available_width = ui.available_width();
    let cols = ((available_width + spacing) / (min_card_width + spacing))
        .floor()
        .max(1.0) as usize;

    ui.with_layout(
        egui::Layout::left_to_right(egui::Align::TOP).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            ui.spacing_mut().item_spacing.y = spacing;
            content(ui, cols);
        },
    );
}

/// 脉动加载指示器
pub fn loading_pulse(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        let time = ui.ctx().input(|i| i.time);
        let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;

        // 脉动圆圈
        let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());

        let center = rect.center();
        let radius = 20.0 + pulse * 4.0;
        let alpha = 0.3 + pulse * 0.4;

        ui.painter().circle(
            center,
            radius,
            theme::ACCENT.linear_multiply(alpha),
            Stroke::new(2.0, theme::ACCENT),
        );

        ui.add_space(20.0);
        ui.label(
            RichText::new(text)
                .size(14.0)
                .color(theme::TEXT_SECONDARY.linear_multiply(0.7 + pulse * 0.3)),
        );

        ui.ctx().request_repaint();
    });
}

/// 进度条
pub fn progress_bar(ui: &mut egui::Ui, progress: f32, label: Option<&str>) {
    ui.vertical(|ui| {
        if let Some(text) = label {
            ui.label(RichText::new(text).size(13.0).color(theme::TEXT_SECONDARY));
            ui.add_space(6.0);
        }

        let height = 6.0;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(400.0), height),
            egui::Sense::hover(),
        );

        // 背景轨道
        ui.painter()
            .rect_filled(rect, CornerRadius::same(255), theme::SURFACE_3);

        // 进度填充
        let progress_clamped = progress.clamp(0.0, 1.0);
        if progress_clamped > 0.0 {
            let fill_width = rect.width() * progress_clamped;
            let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_width, height));

            ui.painter()
                .rect_filled(fill_rect, CornerRadius::same(255), theme::ACCENT);
        }
    });
}

/// 带图标的信息提示框
pub fn info_box(ui: &mut egui::Ui, icon: &str, text: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(icon).size(20.0).color(color));
        ui.add_space(8.0);
        ui.label(RichText::new(text).size(14.0).color(theme::TEXT_SECONDARY));
    });
}

/// 响应式网格布局
/// 根据可用宽度自动计算列数，每列最小宽度360px
pub fn responsive_grid(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    min_column_width: f32,
    mut content: impl FnMut(&mut egui::Ui, usize),
) {
    ui.push_id(id, |ui| {
        let available_width = ui.available_width();
        let columns = ((available_width + 16.0) / (min_column_width + 16.0))
            .floor()
            .max(1.0) as usize;

        let column_width = (available_width - (columns - 1) as f32 * 16.0) / columns as f32;

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 16.0);
            for col in 0..columns {
                ui.allocate_ui_with_layout(
                    egui::vec2(column_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        content(ui, col);
                    },
                );
            }
        });
    });
}

/// 加载动画 - 旋转的圆环
pub fn spinner(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());

    let time = ui.input(|i| i.time);
    let angle = (time * 2.0) as f32 % (2.0 * std::f32::consts::PI);

    ui.ctx().request_repaint();

    let center = rect.center();
    let radius = size / 2.0 - 2.0;

    // 绘制圆弧
    let n_points = 32;
    let start_angle = angle;
    let arc_length = std::f32::consts::PI * 1.5;

    for i in 0..n_points {
        let t = i as f32 / n_points as f32;
        let a = start_angle + t * arc_length;
        let next_a = start_angle + (t + 1.0 / n_points as f32) * arc_length;

        let p1 = center + egui::vec2(a.cos() * radius, a.sin() * radius);
        let p2 = center + egui::vec2(next_a.cos() * radius, next_a.sin() * radius);

        let alpha = (t * 255.0) as u8;
        let color = egui::Color32::from_rgba_unmultiplied(
            theme::ACCENT.r(),
            theme::ACCENT.g(),
            theme::ACCENT.b(),
            alpha,
        );

        ui.painter().line_segment([p1, p2], Stroke::new(3.0, color));
    }
}

/// 骨架屏加载占位符
pub fn skeleton_box(ui: &mut egui::Ui, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

    let time = ui.input(|i| i.time);
    let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;

    ui.ctx().request_repaint();

    let color = animation::lerp_color(theme::SURFACE_2, theme::SURFACE_3, pulse * 0.3);

    ui.painter().rect_filled(rect, CornerRadius::same(6), color);
}

/// 设备卡片 - 显示单个设备信息
pub fn device_card(
    ui: &mut egui::Ui,
    name: &str,
    device_type: &str,
    is_active: bool,
    is_available: bool,
) -> egui::Response {
    let card_id = ui.id().with(name);

    let hovered = ui.ctx().memory(|mem| {
        mem.data
            .get_temp::<bool>(card_id.with("hovered"))
            .unwrap_or(false)
    });
    let hover_anim = animation::animate_bool(ui, card_id.with("hover"), hovered);

    let bg_color = if is_active {
        animation::lerp_color(
            theme::ACCENT.gamma_multiply(0.1),
            theme::ACCENT.gamma_multiply(0.15),
            hover_anim,
        )
    } else {
        animation::lerp_color(theme::SURFACE_2, theme::SURFACE_3, hover_anim * 0.4)
    };

    let border_color = if is_active {
        theme::ACCENT.gamma_multiply(0.6)
    } else {
        animation::lerp_color(
            theme::SURFACE_3,
            theme::ACCENT.gamma_multiply(0.3),
            hover_anim,
        )
    };

    let response = egui::Frame::new()
        .fill(bg_color)
        .stroke(Stroke::new(if is_active { 1.5 } else { 1.0 }, border_color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.horizontal(|ui| {
                // 状态指示器
                let status_color = if is_available {
                    theme::SUCCESS
                } else {
                    theme::TEXT_MUTED
                };

                ui.painter().circle_filled(
                    ui.cursor().min + egui::vec2(6.0, 10.0),
                    4.0,
                    status_color,
                );
                ui.add_space(16.0);

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(name)
                            .size(15.0)
                            .color(if is_active {
                                theme::ACCENT
                            } else {
                                theme::TEXT_PRIMARY
                            })
                            .strong(),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(device_type)
                            .size(12.0)
                            .color(theme::TEXT_MUTED),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_active {
                        ui.label(RichText::new("✓").size(18.0).color(theme::ACCENT));
                    }
                });
            });
        })
        .response;

    ui.ctx().memory_mut(|mem| {
        mem.data
            .insert_temp(card_id.with("hovered"), response.hovered());
    });

    response
}

/// 音量预设按钮组
pub fn volume_presets(ui: &mut egui::Ui, current_db: f32) -> Option<f32> {
    let mut result = None;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("快速设置")
                .size(12.0)
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(8.0);

        let presets = [
            ("静音", -96.0),
            ("-12dB", -12.0),
            ("-6dB", -6.0),
            ("0dB", 0.0),
        ];

        for (label, db) in presets {
            let is_active = (current_db - db).abs() < 0.5;

            let button = if is_active {
                egui::Button::new(RichText::new(label).size(12.0).color(theme::SURFACE_0))
                    .fill(theme::ACCENT)
                    .stroke(Stroke::NONE)
            } else {
                egui::Button::new(RichText::new(label).size(12.0))
                    .fill(theme::SURFACE_3)
                    .stroke(Stroke::new(1.0, theme::SURFACE_4))
            }
            .corner_radius(CornerRadius::same(255))
            .min_size(egui::vec2(50.0, 24.0));

            if ui.add(button).clicked() {
                result = Some(db);
            }
            ui.add_space(4.0);
        }
    });

    result
}

/// 信息卡片 - 显示关键指标
pub fn metric_card(ui: &mut egui::Ui, label: &str, value: &str, trend: Option<&str>) {
    card(ui, label, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(label).size(13.0).color(theme::TEXT_MUTED));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(value)
                        .size(24.0)
                        .strong()
                        .color(theme::TEXT_PRIMARY),
                );
            });

            if let Some(trend_text) = trend {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(trend_text)
                            .size(20.0)
                            .color(theme::TEXT_MUTED),
                    );
                });
            }
        });
    });
}

/// 开关控件
pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let toggle_id = ui.id().with(label);

    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(14.0).color(theme::TEXT_PRIMARY));

        ui.add_space(8.0);

        // 动画
        let anim = animation::animate_bool(ui, toggle_id, *value);

        let width = 44.0;
        let height = 24.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());

        if response.clicked() {
            *value = !*value;
        }

        // 背景轨道
        let bg_color = animation::lerp_color(theme::SURFACE_3, theme::ACCENT, anim);

        ui.painter()
            .rect_filled(rect, CornerRadius::same(255), bg_color);

        // 滑块
        let knob_radius = 10.0;
        let knob_x = rect.min.x + knob_radius + 2.0 + (width - knob_radius * 2.0 - 4.0) * anim;
        let knob_center = egui::pos2(knob_x, rect.center().y);

        ui.painter()
            .circle_filled(knob_center, knob_radius, theme::SURFACE_0);

        // 悬停效果
        if response.hovered() {
            ui.painter().circle_stroke(
                knob_center,
                knob_radius + 2.0,
                Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.3)),
            );
        }

        response
    })
    .inner
}

/// 紧凑状态栏
pub fn compact_status_bar(ui: &mut egui::Ui, items: &[(&str, &str, egui::Color32)]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (label, value, color) in items {
            ui.label(RichText::new(*label).size(12.0).color(theme::TEXT_MUTED));
            ui.label(RichText::new(*value).size(13.0).color(*color));
        }
    });
}

/// 键值对显示（适用于设备信息等）
pub fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(13.0).color(theme::TEXT_SECONDARY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).size(13.0).color(theme::TEXT_PRIMARY));
        });
    });
}

/// 带复制按钮的代码块
pub fn code_block_with_copy(ui: &mut egui::Ui, code: &str, label: &str) -> bool {
    let mut copied = false;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut code.to_string())
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width() - 80.0)
                .interactive(false),
        );
        if secondary_button(ui, "复制").clicked() {
            ui.ctx().copy_text(code.to_string());
            copied = true;
        }
    });
    if !label.is_empty() {
        ui.add_space(4.0);
        note(ui, label);
    }
    copied
}

/// 水平分隔的信息组（用于统计数据）
pub fn stat_row(ui: &mut egui::Ui, stats: &[(&str, String)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        for (i, (label, value)) in stats.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("·").size(13.0).color(theme::SURFACE_4));
            }
            ui.label(RichText::new(*label).size(12.0).color(theme::TEXT_MUTED));
            ui.label(RichText::new(value).size(13.0).color(theme::TEXT_PRIMARY));
        }
    });
}

/// 可折叠的详细信息区域
pub fn collapsible_detail(
    ui: &mut egui::Ui,
    id_source: &str,
    header: &str,
    default_open: bool,
    content: impl FnOnce(&mut egui::Ui),
) {
    let header_id = ui.id().with(id_source);
    let mut is_open = ui
        .ctx()
        .data_mut(|d| d.get_temp::<bool>(header_id).unwrap_or(default_open));

    ui.horizontal(|ui| {
        let arrow = if is_open { "▼" } else { "▶" };
        if ui
            .button(RichText::new(arrow).size(12.0).color(theme::TEXT_MUTED))
            .clicked()
        {
            is_open = !is_open;
            ui.ctx().data_mut(|d| d.insert_temp(header_id, is_open));
        }
        ui.label(
            RichText::new(header)
                .size(15.0)
                .strong()
                .color(theme::TEXT_PRIMARY),
        );
    });

    if is_open {
        ui.add_space(8.0);
        ui.indent(id_source, content);
    }
}
