use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle,
};
use std::sync::Arc;

// Surfaces, darkest to lightest. "Neon Console": near-black blue so the
// source colours and live signal carry the page.
pub const BG_DEEP: Color32 = Color32::from_rgb(0x05, 0x08, 0x0d);
pub const BG: Color32 = Color32::from_rgb(0x09, 0x0e, 0x15);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x07, 0x0b, 0x11);
pub const SURFACE: Color32 = Color32::from_rgb(0x0e, 0x15, 0x1f);
pub const RAISED: Color32 = Color32::from_rgb(0x14, 0x20, 0x2e);
pub const HOVER: Color32 = Color32::from_rgb(0x1a, 0x29, 0x39);
pub const INPUT: Color32 = Color32::from_rgb(0x04, 0x07, 0x0b);
pub const BORDER: Color32 = Color32::from_rgb(0x18, 0x24, 0x34);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x25, 0x36, 0x4a);
/// 1px highlight along the top edge of raised surfaces.
pub const EDGE_LIGHT: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);

pub const TEXT: Color32 = Color32::from_rgb(0xea, 0xf2, 0xf8);
pub const TEXT_2: Color32 = Color32::from_rgb(0xa5, 0xb8, 0xc8);
pub const TEXT_3: Color32 = Color32::from_rgb(0x7a, 0x90, 0xa6);

// Focus, selection and the primary action only.
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xe1, 0xff);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x7b, 0xeb, 0xff);
pub const ACCENT_PRESS: Color32 = Color32::from_rgb(0x22, 0xc3, 0xe0);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x02, 0x1a, 0x22);

// Where a sound comes from. Never reused for state.
pub const SRC_NATIVE: Color32 = Color32::from_rgb(0x38, 0xe1, 0xff);
pub const SRC_AIRPLAY: Color32 = Color32::from_rgb(0xa8, 0x8b, 0xff);
pub const SRC_HUB: Color32 = Color32::from_rgb(0xe8, 0xfb, 0xff);
pub const SRC_OUTPUT: Color32 = Color32::from_rgb(0xf2, 0xf6, 0xfa);
/// Solo spotlight; distinct from the primary action.
pub const SOLO: Color32 = Color32::from_rgb(0xff, 0xd3, 0x4d);

pub const SUCCESS: Color32 = Color32::from_rgb(0x4e, 0xe3, 0x9a);
pub const WARNING: Color32 = Color32::from_rgb(0xff, 0xb5, 0x47);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);
pub const DANGER_HOVER: Color32 = Color32::from_rgb(0xff, 0x8e, 0x8e);
pub const ON_DANGER: Color32 = Color32::from_rgb(0x2a, 0x07, 0x07);

// Meter zones: up to −18 dBFS nominal, −18…−6 caution, above −6 hot.
pub const METER_LOW: Color32 = Color32::from_rgb(0x3e, 0xe0, 0x8c);
pub const METER_MID: Color32 = Color32::from_rgb(0xff, 0xd3, 0x4d);
pub const METER_HIGH: Color32 = Color32::from_rgb(0xff, 0x5a, 0x5a);
pub const METER_TRACK: Color32 = Color32::from_rgb(0x06, 0x0b, 0x11);

pub const RADIUS: u8 = 14;
pub const CONTROL_RADIUS: u8 = 9;
pub const CONTROL_HEIGHT: f32 = 32.0;
pub const COMPACT_HEIGHT: f32 = 28.0;

pub const TITLE: f32 = 22.0;
/// Room master readout, the largest number in the app.
pub const DISPLAY: f32 = 34.0;
pub const SECTION: f32 = 15.0;
pub const BODY: f32 = 14.0;
pub const SMALL: f32 = 12.0;
pub const MONO: f32 = 12.5;

/// Semibold CJK face for titles; falls back to the regular face when absent.
pub fn heading(size: f32) -> FontId {
    FontId::new(size, strong_family_name())
}

pub fn install(ctx: &egui::Context) -> bool {
    install_style(ctx);
    install_fonts(ctx)
}

pub fn install_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(12);
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = RAISED;
    v.code_bg_color = INPUT;
    v.override_text_color = None;
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.collapsing_header_frame = false;

    let radius = CornerRadius::same(CONTROL_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_2);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = HOVER;
    w.hovered.weak_bg_fill = HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = HOVER;
    w.active.weak_bg_fill = HOVER;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = RAISED;
    w.open.weak_bg_fill = RAISED;
    w.open.bg_stroke = Stroke::new(1.0, ACCENT);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;

    let s = &mut style.spacing;
    s.scroll.floating = false;
    s.scroll.bar_width = 8.0;
    s.scroll.bar_inner_margin = 4.0;
    s.scroll.bar_outer_margin = 2.0;
    s.item_spacing = egui::vec2(8.0, 8.0);
    s.button_padding = egui::vec2(12.0, 6.0);
    // Combo boxes and text fields share the 32px button height.
    s.interact_size = egui::vec2(32.0, CONTROL_HEIGHT);
    s.menu_margin = Margin::same(6);
    s.window_margin = Margin::same(20);
    s.indent = 16.0;
    s.slider_width = 240.0;
    s.slider_rail_height = 6.0;
    s.combo_width = 220.0;
    s.text_edit_width = 320.0;
    s.icon_width = 16.0;
    s.icon_width_inner = 9.0;

    style.interaction.show_tooltips_only_when_still = false;
    style.interaction.tooltip_delay = 0.35;
    style.animation_time = 0.14;
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(SMALL)),
        (TextStyle::Body, FontId::proportional(BODY)),
        (TextStyle::Button, FontId::proportional(BODY)),
        (TextStyle::Heading, heading(TITLE)),
        (TextStyle::Monospace, FontId::monospace(MONO)),
    ]
    .into();
    ctx.set_style(style);
}

type Face = (std::path::PathBuf, u32);

/// Regular and optional semibold faces, most preferred first.
fn candidates() -> Vec<(Face, Option<Face>)> {
    let mut list: Vec<(Face, Option<Face>)> = Vec::new();
    if let Some(path) = std::env::var_os("NEONMIX_CJK_FONT") {
        list.push(((path.into(), 0), None));
    }
    // macOS ships PingFang as a MobileAsset in a hashed directory; the
    // collection's SC Regular/Semibold faces are 3 and 11 (0 is the HK face).
    let mut pingfang = vec![std::path::PathBuf::from(
        "/System/Library/Fonts/PingFang.ttc",
    )];
    for dir in std::fs::read_dir("/System/Library/AssetsV2")
        .into_iter()
        .flatten()
        .flatten()
    {
        if dir
            .file_name()
            .to_string_lossy()
            .starts_with("com_apple_MobileAsset_Font")
        {
            for asset in std::fs::read_dir(dir.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                pingfang.push(asset.path().join("AssetData/PingFang.ttc"));
            }
        }
    }
    for path in pingfang {
        list.push(((path.clone(), 3), Some((path, 11))));
    }
    let hiragino = std::path::PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc");
    list.push(((hiragino.clone(), 0), Some((hiragino, 2))));
    list.push((
        ("C:/Windows/Fonts/msyh.ttc".into(), 0),
        Some(("C:/Windows/Fonts/msyhbd.ttc".into(), 0)),
    ));
    // Noto Sans CJK collections order JP, KR, SC, TC, HK.
    list.push((
        (
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into(),
            2,
        ),
        Some((
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc".into(),
            2,
        )),
    ));
    list
}

/// Fonts live for the whole process; one leaked copy serves both faces of a
/// collection instead of duplicating a large system file.
fn load(path: &std::path::Path) -> Option<&'static [u8]> {
    std::fs::read(path)
        .ok()
        .map(|b| &*Box::leak(b.into_boxed_slice()))
}

fn install_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let proportional = fonts.families[&FontFamily::Proportional].clone();
    let mut found = false;
    for ((path, regular), strong) in candidates() {
        let Some(bytes) = (path.is_file()).then(|| load(&path)).flatten() else {
            continue;
        };
        let face = |data: &'static [u8], index| {
            let mut font = FontData::from_static(data);
            font.index = index;
            Arc::new(font)
        };
        fonts.font_data.insert("cjk".into(), face(bytes, regular));
        let strong = strong.and_then(|(strong_path, index)| {
            let data = if strong_path == path {
                Some(bytes)
            } else {
                load(&strong_path)
            };
            data.map(|d| face(d, index))
        });
        let mut strong_family = vec!["cjk".to_owned()];
        if let Some(font) = strong {
            fonts.font_data.insert("cjk-strong".into(), font);
            strong_family.insert(0, "cjk-strong".into());
        }
        let mut regular_family = vec!["cjk".to_owned()];
        regular_family.extend(proportional.iter().cloned());
        strong_family.extend(proportional.iter().cloned());
        fonts
            .families
            .insert(FontFamily::Proportional, regular_family);
        fonts.families.insert(strong_family_name(), strong_family);
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("cjk".into());
        found = true;
        break;
    }
    if !found {
        return fallback_fonts(ctx);
    }
    ctx.set_fonts(fonts);
    found
}

/// Built-in fonts only, with the named heading family still resolvable.
pub fn fallback_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = FontDefinitions::default();
    let proportional = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(strong_family_name(), proportional);
    ctx.set_fonts(fonts);
    false
}

fn strong_family_name() -> FontFamily {
    FontFamily::Name("strong".into())
}
