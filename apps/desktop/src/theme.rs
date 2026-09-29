use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};
pub const BACKGROUND: Color32 = Color32::from_rgb(0x18, 0x24, 0x2e);
pub const SURFACE: Color32 = Color32::from_rgb(0x21, 0x34, 0x40);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xf3, 0xf7);
pub const MUTED: Color32 = Color32::from_rgb(0xad, 0xc1, 0xcc);
pub const PRIMARY: Color32 = Color32::from_rgb(0x82, 0xc8, 0xe8);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0xb0, 0xac);
pub fn install(ctx: &egui::Context) -> bool {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BACKGROUND;
    style.visuals.extreme_bg_color = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.selection.bg_fill = PRIMARY.gamma_multiply(0.35);
    style.visuals.selection.stroke.color = PRIMARY;
    style.visuals.widgets.hovered.bg_stroke.color = PRIMARY;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(16.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(28.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(16.0));
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
