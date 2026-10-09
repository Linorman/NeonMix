use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle,
};
use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// "Studio Graphite": layers differ by lightness and shadow, not by outline.
/// Colour is reserved for where sound comes from, state and real levels.
/// Every colour token exists once per mode; the active one follows egui's
/// resolved theme (`sync`) so both modes share all painting code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Dark,
    Light,
}

pub struct Palette {
    // Elevation, lowest to highest. Wells are recessed below the canvas.
    pub well: Color32,
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub overlay: Color32,
    /// Rows recessed into a card (lists, lanes).
    pub inset: Color32,
    /// Hairlines are translucent so they read on every layer.
    pub border: Color32,
    pub border_strong: Color32,
    /// 1px light catching the top edge of raised surfaces.
    pub edge_light: Color32,
    pub text: Color32,
    pub text_2: Color32,
    pub text_3: Color32,
    /// Focus and selection only.
    pub accent: Color32,
    /// The one primary action per view: a solid, high-contrast cap.
    pub primary: Color32,
    pub primary_hover: Color32,
    pub primary_press: Color32,
    pub on_primary: Color32,
    // Where a sound comes from. Never reused for state.
    pub src_native: Color32,
    pub src_airplay: Color32,
    pub src_hub: Color32,
    pub src_output: Color32,
    /// Solo spotlight; distinct from the primary action.
    pub solo: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    pub on_danger: Color32,
    // Meter zones: up to −18 dBFS nominal, −18…−6 caution, above −6 hot.
    pub meter_low: Color32,
    pub meter_mid: Color32,
    pub meter_high: Color32,
    pub meter_track: Color32,
    /// Unlit LED cells stay faintly visible.
    pub meter_unlit: Color32,
    /// Fader cap body, top and bottom of its metal gradient.
    pub cap_top: Color32,
    pub cap_bottom: Color32,
    /// Shadow ink and its strength for this mode.
    pub shadow: Color32,
    pub shadow_strength: f32,
    /// Vertical lightness falloff of card gradients (0 = flat).
    pub ramp_depth: u8,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const DARK: Palette = Palette {
    well: rgb(0x07080a),
    bg: rgb(0x0a0b0e),
    sidebar: rgb(0x0f1115),
    surface: rgb(0x15181d),
    raised: rgb(0x1e2229),
    hover: rgb(0x272c35),
    overlay: rgb(0x232831),
    inset: rgb(0x101216),
    border: Color32::from_rgba_premultiplied(13, 13, 13, 13),
    border_strong: Color32::from_rgba_premultiplied(26, 26, 26, 26),
    edge_light: Color32::from_rgba_premultiplied(15, 15, 15, 15),
    text: rgb(0xedeff3),
    text_2: rgb(0xa3aab6),
    text_3: rgb(0x8a92a0),
    accent: rgb(0x3ccfe6),
    primary: rgb(0xe9ecf1),
    primary_hover: rgb(0xffffff),
    primary_press: rgb(0xc5cad3),
    on_primary: rgb(0x0a0b0e),
    src_native: rgb(0x3ccfe6),
    src_airplay: rgb(0xa08cff),
    src_hub: rgb(0xe9edf2),
    src_output: rgb(0xf2f4f7),
    solo: rgb(0xf5c451),
    success: rgb(0x45d483),
    warning: rgb(0xf2a541),
    danger: rgb(0xf06a6a),
    danger_hover: rgb(0xff8a8a),
    on_danger: rgb(0x2a0707),
    meter_low: rgb(0x45d483),
    meter_mid: rgb(0xf5c451),
    meter_high: rgb(0xf06a6a),
    meter_track: rgb(0x050607),
    meter_unlit: Color32::from_rgba_premultiplied(14, 14, 14, 14),
    cap_top: rgb(0xf2f4f7),
    cap_bottom: rgb(0xaab2bd),
    shadow: Color32::BLACK,
    shadow_strength: 1.0,
    ramp_depth: 44,
};

pub const LIGHT: Palette = Palette {
    well: rgb(0xe2e5ea),
    bg: rgb(0xeef0f3),
    sidebar: rgb(0xe6e9ed),
    surface: rgb(0xffffff),
    raised: rgb(0xfafbfc),
    hover: rgb(0xeff2f6),
    overlay: rgb(0xffffff),
    inset: rgb(0xf3f5f7),
    border: Color32::from_rgba_premultiplied(0, 0, 0, 20),
    border_strong: Color32::from_rgba_premultiplied(0, 0, 0, 38),
    edge_light: Color32::from_rgba_premultiplied(255, 255, 255, 255),
    text: rgb(0x14171c),
    text_2: rgb(0x474e59),
    text_3: rgb(0x626974),
    accent: rgb(0x066c7d),
    primary: rgb(0x1b1e24),
    primary_hover: rgb(0x2d3139),
    primary_press: rgb(0x0e1013),
    on_primary: rgb(0xffffff),
    src_native: rgb(0x066c7d),
    src_airplay: rgb(0x6b4fe0),
    src_hub: rgb(0x2b2f37),
    src_output: rgb(0x1b1e24),
    solo: rgb(0x875d04),
    success: rgb(0x137033),
    warning: rgb(0x994a07),
    danger: rgb(0xb32f2f),
    danger_hover: rgb(0xdc4a4a),
    on_danger: rgb(0xffffff),
    meter_low: rgb(0x22a35a),
    meter_mid: rgb(0xe0a91e),
    meter_high: rgb(0xe04848),
    meter_track: rgb(0xd9dde3),
    meter_unlit: Color32::from_rgba_premultiplied(0, 0, 0, 16),
    cap_top: rgb(0xffffff),
    cap_bottom: rgb(0xd3d8df),
    shadow: rgb(0x101828),
    shadow_strength: 0.32,
    ramp_depth: 6,
};

// UI state lives on the UI thread; thread-local keeps parallel tests apart.
thread_local! {
    static MODE: Cell<Mode> = const { Cell::new(Mode::Dark) };
}

pub fn mode() -> Mode {
    MODE.with(Cell::get)
}

pub fn set_mode(mode: Mode) {
    MODE.with(|m| m.set(mode));
}

pub fn is_light() -> bool {
    mode() == Mode::Light
}

pub fn palette() -> &'static Palette {
    match mode() {
        Mode::Dark => &DARK,
        Mode::Light => &LIGHT,
    }
}

/// Follow the theme egui resolved for this frame (explicit choice or the
/// system appearance). Call once at the start of every frame.
pub fn sync(ctx: &egui::Context) {
    set_mode(match ctx.theme() {
        egui::Theme::Light => Mode::Light,
        egui::Theme::Dark => Mode::Dark,
    });
}

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        $(
            #[inline]
            pub fn $name() -> Color32 {
                palette().$name
            }
        )*
    };
}

tokens!(
    well,
    bg,
    sidebar,
    surface,
    raised,
    hover,
    overlay,
    inset,
    border,
    border_strong,
    edge_light,
    text,
    text_2,
    text_3,
    accent,
    primary,
    primary_hover,
    primary_press,
    on_primary,
    src_native,
    src_airplay,
    src_hub,
    src_output,
    solo,
    success,
    warning,
    danger,
    danger_hover,
    on_danger,
    meter_low,
    meter_mid,
    meter_high,
    meter_track,
    meter_unlit,
    cap_top,
    cap_bottom,
);

/// Shadow ink at `alpha` (0…1) of this mode's strength.
pub fn shadow(alpha: f32) -> Color32 {
    let p = palette();
    p.shadow
        .gamma_multiply((alpha * p.shadow_strength).clamp(0.0, 1.0))
}

pub const RADIUS: u8 = 14;
pub const CONTROL_RADIUS: u8 = 9;
pub const CONTROL_HEIGHT: f32 = 32.0;
pub const COMPACT_HEIGHT: f32 = 28.0;

pub const TITLE: f32 = 24.0;
/// Room master readout, the largest number in the app.
pub const DISPLAY: f32 = 34.0;
pub const SECTION: f32 = 15.0;
pub const BODY: f32 = 14.0;
pub const SMALL: f32 = 12.0;
pub const MONO: f32 = 12.5;
/// Sidebar group captions.
pub const GROUP: f32 = 11.0;

/// Semibold CJK face for titles; falls back to the regular face when absent.
pub fn heading(size: f32) -> FontId {
    FontId::new(size, strong_family_name())
}

pub fn install(ctx: &egui::Context) -> bool {
    install_style(ctx);
    install_fonts(ctx)
}

/// Both modes are installed up front; egui picks one per frame from the
/// theme preference and the system appearance.
pub fn install_style(ctx: &egui::Context) {
    ctx.set_style_of(
        egui::Theme::Dark,
        style_for(&ctx.style_of(egui::Theme::Dark), &DARK, false),
    );
    ctx.set_style_of(
        egui::Theme::Light,
        style_for(&ctx.style_of(egui::Theme::Light), &LIGHT, true),
    );
}

fn style_for(base: &egui::Style, p: &Palette, light: bool) -> egui::Style {
    let mut style = base.clone();
    style.visuals = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let shadow = |alpha: f32| {
        p.shadow
            .gamma_multiply((alpha * p.shadow_strength).min(1.0))
    };
    let v = &mut style.visuals;
    v.panel_fill = p.bg;
    v.window_fill = p.overlay;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.window_corner_radius = CornerRadius::same(12);
    v.window_shadow = egui::Shadow {
        offset: [0, 18],
        blur: 48,
        spread: 0,
        color: shadow(0.7),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: shadow(0.55),
    };
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS + 1);
    v.extreme_bg_color = p.well;
    v.faint_bg_color = p.raised;
    v.code_bg_color = p.well;
    v.override_text_color = None;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(if light { 0.22 } else { 0.35 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.collapsing_header_frame = false;

    let radius = CornerRadius::same(CONTROL_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_2);
    w.noninteractive.corner_radius = radius;

    // `bg_fill` paints scrollbar handles and check boxes; `weak_bg_fill`
    // paints buttons and combo boxes.
    let handle = if light { rgb(0xc4c9d1) } else { rgb(0x353b45) };
    w.inactive.bg_fill = handle;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = if light { rgb(0xa9afb9) } else { rgb(0x4a5260) };
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = if light { rgb(0x8e95a1) } else { rgb(0x5d6676) };
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;

    let s = &mut style.spacing;
    s.scroll.floating = false;
    s.scroll.foreground_color = false;
    s.scroll.bar_width = 8.0;
    s.scroll.bar_inner_margin = 4.0;
    s.scroll.bar_outer_margin = 2.0;
    s.item_spacing = egui::vec2(8.0, 8.0);
    s.button_padding = egui::vec2(14.0, 6.0);
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
    style
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
    static FONT_FILES: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let mut cache = FONT_FILES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    load_cached(path, &mut cache)
}

// The candidate list is fixed for the process (system fonts plus one optional
// override). Keep an explicit cap as well, so repeated installation or changed
// overrides cannot accumulate unbounded process-lifetime font copies.
const MAX_CACHED_FONT_FILES: usize = 8;
fn load_cached(path: &Path, cache: &mut HashMap<PathBuf, &'static [u8]>) -> Option<&'static [u8]> {
    let path = std::fs::canonicalize(path).ok()?;
    if let Some(bytes) = cache.get(&path) {
        return Some(*bytes);
    }
    if cache.len() >= MAX_CACHED_FONT_FILES {
        return None;
    }
    let bytes: &'static [u8] = Box::leak(std::fs::read(&path).ok()?.into_boxed_slice());
    cache.insert(path, bytes);
    Some(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluent_direction_isolates_are_invisible_egui_glyphs() {
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            context.fonts(|fonts| {
                let plain = fonts.layout_no_wrap(
                    "Device name".into(),
                    FontId::proportional(14.0),
                    Color32::WHITE,
                );
                let isolated = fonts.layout_no_wrap(
                    "Device \u{2068}name\u{2069}".into(),
                    FontId::proportional(14.0),
                    Color32::WHITE,
                );
                // Invisible boundaries can affect adjacent kerning slightly.
                assert_eq!(plain.size().y, isolated.size().y);
                assert!((plain.size().x - isolated.size().x).abs() < 0.5);
                for glyph in isolated
                    .rows
                    .iter()
                    .flat_map(|row| &row.glyphs)
                    .filter(|glyph| matches!(glyph.chr, '\u{2068}' | '\u{2069}'))
                {
                    assert_eq!(glyph.advance_width, 0.0);
                    assert!(glyph.uv_rect.is_nothing());
                }
            });
        });
    }
    #[test]
    fn cached_font_bytes_are_read_once_and_reused() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/tmp");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("font-cache-test-{}", std::process::id()));
        std::fs::write(&path, b"initial font bytes").unwrap();
        let mut cache = HashMap::new();
        let first = load_cached(&path, &mut cache).unwrap();
        std::fs::write(&path, b"changed bytes that must not be reread").unwrap();
        let second = load_cached(&path, &mut cache).unwrap();
        assert!(std::ptr::eq(first, second));
        assert_eq!(second, b"initial font bytes");
        assert_eq!(cache.len(), 1);
        std::fs::remove_file(path).unwrap();
    }
}
