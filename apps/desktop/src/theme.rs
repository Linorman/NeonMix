use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

// 色阶系统：从深到浅的表面层次
pub const SURFACE_0: Color32 = Color32::from_rgb(0x05, 0x0d, 0x14); // 最深背景，3% 亮度
pub const SURFACE_1: Color32 = Color32::from_rgb(0x0a, 0x15, 0x1f); // 主背景
pub const SURFACE_2: Color32 = Color32::from_rgb(0x0f, 0x1d, 0x29); // 卡片背景
pub const SURFACE_3: Color32 = Color32::from_rgb(0x16, 0x27, 0x35); // 悬停表面
pub const SURFACE_4: Color32 = Color32::from_rgb(0x1e, 0x32, 0x42); // 活动表面
pub const SURFACE_5: Color32 = Color32::from_rgb(0x28, 0x3e, 0x51); // 最亮表面

// 文本系统
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xf0, 0xf6, 0xfa);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xb8, 0xcc, 0xd9);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x7a, 0x91, 0xa3);
pub const TEXT_DISABLED: Color32 = Color32::from_rgb(0x4a, 0x5b, 0x69);

// 音频应用：青色主题（代表数字音频、信号流）
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xbd, 0xf8); // 高饱和青色
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x4d, 0xcb, 0xfa);
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x27, 0xa7, 0xde);
pub const ACCENT_MUTED: Color32 = Color32::from_rgb(0x2d, 0x7a, 0x9a); // 低饱和辅助色

// 语义色
pub const SUCCESS: Color32 = Color32::from_rgb(0x6e, 0xe7, 0xb7); // 翡翠绿
pub const WARNING: Color32 = Color32::from_rgb(0xfb, 0xbc, 0x4e); // 琥珀黄
pub const DANGER: Color32 = Color32::from_rgb(0xf8, 0x7c, 0x7c); // 柔和红
pub const INFO: Color32 = Color32::from_rgb(0x7d, 0xd3, 0xfc); // 浅青

// 电平颜色（从安全到危险的渐变）
pub const METER_SAFE: Color32 = Color32::from_rgb(0x6e, 0xe7, 0xb7);
pub const METER_CAUTION: Color32 = Color32::from_rgb(0xfb, 0xbc, 0x4e);
pub const METER_DANGER: Color32 = Color32::from_rgb(0xf8, 0x7c, 0x7c);

// 向后兼容
pub const BACKGROUND: Color32 = SURFACE_1;
pub const SURFACE: Color32 = SURFACE_2;
pub const TEXT: Color32 = TEXT_PRIMARY;
pub const MUTED: Color32 = TEXT_MUTED;
pub const PRIMARY: Color32 = ACCENT;
pub fn install(ctx: &egui::Context) -> bool {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();

    // 背景与面板
    style.visuals.panel_fill = SURFACE_1;
    style.visuals.extreme_bg_color = SURFACE_0;
    style.visuals.window_fill = SURFACE_2;
    style.visuals.faint_bg_color = SURFACE_2;

    // 文本
    style.visuals.override_text_color = Some(TEXT_PRIMARY);
    style.visuals.warn_fg_color = WARNING;
    style.visuals.error_fg_color = DANGER;
    style.visuals.hyperlink_color = ACCENT;

    // 选择与交互
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.25);
    style.visuals.selection.stroke.color = ACCENT;

    // 组件默认状态
    style.visuals.widgets.noninteractive.bg_fill = SURFACE_2;
    style.visuals.widgets.noninteractive.weak_bg_fill = SURFACE_1;
    style.visuals.widgets.noninteractive.bg_stroke.color = SURFACE_3;
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT_SECONDARY;

    // 非活动状态
    style.visuals.widgets.inactive.bg_fill = SURFACE_2;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE_1;
    style.visuals.widgets.inactive.bg_stroke.color = SURFACE_3;
    style.visuals.widgets.inactive.fg_stroke.color = TEXT_SECONDARY;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(255);

    // 悬停状态
    style.visuals.widgets.hovered.bg_fill = SURFACE_3;
    style.visuals.widgets.hovered.weak_bg_fill = SURFACE_2;
    style.visuals.widgets.hovered.bg_stroke.color = ACCENT.gamma_multiply(0.6);
    style.visuals.widgets.hovered.bg_stroke.width = 1.5;
    style.visuals.widgets.hovered.fg_stroke.color = TEXT_PRIMARY;
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(255);

    // 活动状态
    style.visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.3);
    style.visuals.widgets.active.weak_bg_fill = SURFACE_3;
    style.visuals.widgets.active.bg_stroke.color = ACCENT;
    style.visuals.widgets.active.bg_stroke.width = 2.0;
    style.visuals.widgets.active.fg_stroke.color = TEXT_PRIMARY;
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(255);

    // 开关组件
    style.visuals.widgets.open.bg_fill = SURFACE_3;
    style.visuals.widgets.open.weak_bg_fill = SURFACE_2;
    style.visuals.widgets.open.bg_stroke.color = ACCENT;
    style.visuals.widgets.open.bg_stroke.width = 1.5;
    style.visuals.widgets.open.corner_radius = egui::CornerRadius::same(255);

    // 滚动条
    style.spacing.scroll.floating = false;
    style.spacing.scroll.bar_width = 10.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.bar_outer_margin = 0.0;

    // 间距系统（8px 网格）
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.indent = 20.0;
    style.spacing.window_margin = egui::Margin::same(16);

    // 交互
    style.interaction.resize_grab_radius_side = 6.0;
    style.interaction.resize_grab_radius_corner = 8.0;
    style.interaction.show_tooltips_only_when_still = false;

    // 字体大小（流式缩放基础）
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(15.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));

    ctx.set_style(style);
    let paths = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/PingFangUI.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "C:/Windows/Fonts/msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    let paths = std::env::var("NEONMIX_CJK_FONT")
        .ok()
        .into_iter()
        .chain(paths.into_iter().map(str::to_owned));
    for path in paths {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".into(), FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".into());
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push("cjk".into());
            ctx.set_fonts(fonts);
            return true;
        }
    }
    false
}
