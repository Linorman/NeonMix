//! Shared controls. Colours come from `theme`; motion from `animation`.
use crate::animation;
use crate::localization::text;
use crate::theme;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Margin, Response, RichText, Stroke, StrokeKind, Ui,
};
use neonmix_i18n::Message;

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
    /// Solo spotlight.
    Solo,
}

impl Tone {
    pub fn color(self) -> Color32 {
        match self {
            Self::Neutral => theme::TEXT_3,
            Self::Accent => theme::ACCENT,
            Self::Success => theme::SUCCESS,
            Self::Warning => theme::WARNING,
            Self::Danger => theme::DANGER,
            Self::Solo => theme::SOLO,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
    /// Destructive action that should not dominate (outline only).
    Quiet,
}

#[derive(Clone, Copy)]
struct Palette {
    base: Color32,
    hover: Color32,
    press: Color32,
    text: Color32,
    stroke: Color32,
}

impl Palette {
    fn mix(self, other: Self, t: f32) -> Self {
        let m = |a, b| animation::lerp_color(a, b, t);
        Self {
            base: m(self.base, other.base),
            hover: m(self.hover, other.hover),
            press: m(self.press, other.press),
            text: m(self.text, other.text),
            stroke: m(self.stroke, other.stroke),
        }
    }
}

fn palette(kind: Kind) -> Palette {
    match kind {
        Kind::Primary => Palette {
            base: theme::ACCENT,
            hover: theme::ACCENT_HOVER,
            press: theme::ACCENT_PRESS,
            text: theme::ON_ACCENT,
            stroke: Color32::TRANSPARENT,
        },
        Kind::Secondary => Palette {
            base: theme::RAISED,
            hover: theme::HOVER,
            press: theme::SURFACE,
            text: theme::TEXT,
            stroke: theme::BORDER_STRONG,
        },
        Kind::Danger => Palette {
            base: theme::DANGER,
            hover: theme::DANGER_HOVER,
            press: theme::DANGER.gamma_multiply(0.85),
            text: theme::ON_DANGER,
            stroke: Color32::TRANSPARENT,
        },
        Kind::Quiet => Palette {
            base: Color32::TRANSPARENT,
            hover: theme::DANGER.gamma_multiply(0.14),
            press: theme::DANGER.gamma_multiply(0.22),
            text: theme::DANGER,
            stroke: theme::DANGER.gamma_multiply(0.55),
        },
    }
}

fn engaged(color: Color32) -> Palette {
    Palette {
        base: color,
        hover: animation::lerp_color(color, Color32::WHITE, 0.15),
        press: color.gamma_multiply(0.85),
        text: theme::ON_ACCENT,
        stroke: Color32::TRANSPARENT,
    }
}

const UNAVAILABLE: Palette = Palette {
    base: theme::SURFACE,
    hover: theme::SURFACE,
    press: theme::SURFACE,
    text: theme::TEXT_3,
    stroke: theme::BORDER,
};

fn paint_focus(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            CornerRadius::same(theme::CONTROL_RADIUS + 2),
            Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.85)),
            StrokeKind::Outside,
        );
    }
}

struct Spec<'a> {
    text: &'a str,
    palette: Palette,
    enabled: bool,
    selected: Option<bool>,
    compact: bool,
    /// Work started by this control is running: keep its colour and size,
    /// swap the label for a spinner and ignore further clicks.
    busy: bool,
    radius: CornerRadius,
    /// Filled call to action: a soft halo of its own colour on hover.
    halo: Option<Color32>,
}

impl<'a> Spec<'a> {
    fn new(text: &'a str, palette: Palette) -> Self {
        Self {
            text,
            palette,
            enabled: true,
            selected: None,
            compact: false,
            busy: false,
            radius: CornerRadius::same(theme::CONTROL_RADIUS),
            halo: None,
        }
    }
}

/// Scoped children never wrap inside `horizontal_wrapped`; measure first and
/// start a new row when the item would not fit the rest of this one.
pub fn wrap_for(ui: &mut Ui, width: f32) {
    let layout = ui.layout();
    if layout.main_wrap()
        && layout.is_horizontal()
        && ui.available_size_before_wrap().x < width
        && ui.cursor().min.x > ui.max_rect().min.x + 0.5
    {
        ui.end_row();
    }
}

fn draw(ui: &mut Ui, spec: Spec<'_>) -> Response {
    {
        let (size, height, pad) = if spec.compact {
            (theme::SMALL + 0.5, theme::COMPACT_HEIGHT, 10.0)
        } else {
            (
                theme::BODY,
                theme::CONTROL_HEIGHT,
                ui.spacing().button_padding.x,
            )
        };
        let text_w = ui.fonts(|f| {
            f.layout_no_wrap(
                spec.text.to_owned(),
                egui::FontId::proportional(size),
                Color32::WHITE,
            )
            .size()
            .x
        });
        wrap_for(ui, (text_w + 2.0 * pad).max(height));
    }
    // Unavailable actions read as neutral, not as dimmed colour.
    let palette = if spec.enabled || spec.busy {
        spec.palette
    } else {
        UNAVAILABLE
    };
    ui.scope(|ui| {
        if !spec.enabled {
            ui.disable();
        }
        let id = ui.next_auto_id();
        let (hover, press) = animation::interaction(ui.ctx(), id);
        let fill = animation::lerp_color(
            animation::lerp_color(palette.base, palette.hover, hover),
            palette.press,
            press,
        );
        let (size, height) = if spec.compact {
            ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
            (theme::SMALL + 0.5, theme::COMPACT_HEIGHT)
        } else {
            (theme::BODY, theme::CONTROL_HEIGHT)
        };
        let text_color = if spec.busy {
            Color32::TRANSPARENT
        } else {
            palette.text
        };
        let mut button = egui::Button::new(RichText::new(spec.text).size(size).color(text_color))
            .fill(fill)
            .stroke(Stroke::new(1.0, palette.stroke))
            .corner_radius(spec.radius)
            .min_size(egui::vec2(height, height))
            // A label never breaks inside its button; rows wrap instead.
            .wrap_mode(egui::TextWrapMode::Extend);
        if let Some(selected) = spec.selected {
            button = button.selected(selected);
        }
        if spec.busy {
            // Not clickable, but not faded either.
            button = button.sense(egui::Sense::hover());
        }
        let halo_slot = ui.painter().add(egui::Shape::Noop);
        let response = ui.add(button);
        if let Some(color) = spec.halo.filter(|_| spec.enabled) {
            let strength = 0.10 + 0.22 * hover;
            ui.painter().set(
                halo_slot,
                egui::epaint::RectShape::filled(
                    response.rect.translate(egui::vec2(0.0, 2.0)),
                    spec.radius,
                    color.gamma_multiply(strength),
                )
                .with_blur_width(14.0),
            );
        }
        if spec.busy {
            let spinner =
                egui::Rect::from_center_size(response.rect.center(), egui::vec2(14.0, 14.0));
            let painter = ui.painter();
            let t = ui.input(|i| i.time) as f32;
            let start = t * 6.0;
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let a = start + i as f32 / 24.0 * 4.4;
                    spinner.center() + 6.0 * egui::vec2(a.cos(), a.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(2.0, palette.text)));
            ui.ctx().request_repaint();
        }
        paint_focus(ui, &response);
        response
    })
    .inner
}

fn halo(kind: Kind) -> Option<Color32> {
    match kind {
        Kind::Primary => Some(theme::ACCENT),
        Kind::Danger => Some(theme::DANGER),
        _ => None,
    }
}

pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    draw(
        ui,
        Spec {
            halo: halo(kind),
            ..Spec::new(text, palette(kind))
        },
    )
}

pub fn button_enabled(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    draw(
        ui,
        Spec {
            enabled,
            halo: halo(kind),
            ..Spec::new(text, palette(kind))
        },
    )
}

/// Button for an action that takes a round trip; shows progress in place.
pub fn button_busy(ui: &mut Ui, enabled: bool, busy: bool, text: &str, kind: Kind) -> Response {
    draw(
        ui,
        Spec {
            enabled,
            busy,
            halo: halo(kind),
            ..Spec::new(text, palette(kind))
        },
    )
}

/// Compact button of any kind, for tight rows such as the sidebar footer.
pub fn button_compact(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    draw(
        ui,
        Spec {
            compact: true,
            ..Spec::new(text, palette(kind))
        },
    )
}

pub fn small_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    draw(
        ui,
        Spec {
            enabled,
            compact: true,
            ..Spec::new(text, palette(Kind::Secondary))
        },
    )
}

/// Latching control (Mute/Solo). Fill eases to `tone` while engaged.
pub fn toggle(ui: &mut Ui, enabled: bool, on: bool, text: &str, tone: Tone) -> Response {
    let key = ui.next_auto_id().with("engaged");
    let t = ui.ctx().animate_bool_with_time(key, on, 0.16);
    let palette = palette(Kind::Secondary).mix(engaged(tone.color()), t);
    draw(
        ui,
        Spec {
            enabled,
            selected: Some(on),
            halo: on.then(|| tone.color()),
            ..Spec::new(text, palette)
        },
    )
}

/// Compact latching control for dense console strips.
pub fn toggle_small(ui: &mut Ui, enabled: bool, on: bool, text: &str, tone: Tone) -> Response {
    let key = ui.next_auto_id().with("engaged");
    let t = ui.ctx().animate_bool_with_time(key, on, 0.16);
    let palette = palette(Kind::Secondary).mix(engaged(tone.color()), t);
    draw(
        ui,
        Spec {
            enabled,
            selected: Some(on),
            compact: true,
            halo: on.then(|| tone.color()),
            ..Spec::new(text, palette)
        },
    )
}

/// Joined single-choice buttons (gain presets). Returns the picked value
/// when it differs from `current`.
pub fn segmented(ui: &mut Ui, enabled: bool, items: &[(&str, f32)], current: f32) -> Option<f32> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let last = items.len().saturating_sub(1);
        for (i, (label, value)) in items.iter().enumerate() {
            let active = (current - value).abs() < 0.25;
            let r = theme::CONTROL_RADIUS;
            let radius = CornerRadius {
                nw: if i == 0 { r } else { 0 },
                sw: if i == 0 { r } else { 0 },
                ne: if i == last { r } else { 0 },
                se: if i == last { r } else { 0 },
            };
            let palette = if active {
                Palette {
                    base: theme::ACCENT.gamma_multiply(0.2),
                    hover: theme::ACCENT.gamma_multiply(0.26),
                    press: theme::ACCENT.gamma_multiply(0.3),
                    text: theme::ACCENT_HOVER,
                    stroke: theme::ACCENT.gamma_multiply(0.6),
                }
            } else {
                palette(Kind::Secondary)
            };
            let response = draw(
                ui,
                Spec {
                    enabled,
                    selected: Some(active),
                    radius,
                    ..Spec::new(label, palette)
                },
            );
            if response.clicked() && !active {
                picked = Some(*value);
            }
        }
    });
    picked
}

/// Raised card: lit gradient, hairline border and top highlight; `glow`
/// tints the top-left corner with the card's state colour.
pub fn surface<R>(
    ui: &mut Ui,
    glow: Option<Color32>,
    margin: Margin,
    body: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = ui.painter().add(egui::Shape::Noop);
    let shown = egui::Frame::new().inner_margin(margin).show(ui, body);
    // Drawn as given: the frame's id is positional, so easing it would let a
    // card briefly take the glow colour of whichever card held that id before.
    ui.painter().set(
        slot,
        crate::fx::surface(
            ui.ctx(),
            shown.response.rect,
            theme::RADIUS,
            theme::SURFACE,
            glow,
        ),
    );
    egui::InnerResponse::new(shown.inner, shown.response)
}

/// Card with an optional subtitle under the title and a status rail on the
/// left edge (live surfaces such as mixer lanes). The rail colour eases.
pub fn card_ex<R>(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    rail: Option<Color32>,
    trailing: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    // Callers scope dynamic cards by business identity. Display titles never
    // participate in rail animation identity across language changes.
    let rail_id = ui.next_auto_id().with("rail");
    let margin = Margin {
        left: 18,
        right: 16,
        top: 14,
        bottom: 16,
    };
    let shown = surface(ui, rail, margin, |ui| {
        ui.set_min_width(ui.available_width());
        // Top-aligned so the pill lines up with the title's cap height
        // rather than centring in a full control-height row.
        ui.horizontal_top(|ui| {
            let reserve = (ui.available_width() * 0.35).clamp(90.0, 220.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - reserve);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(title)
                            .font(theme::heading(theme::SECTION))
                            .color(theme::TEXT),
                    )
                    .truncate(),
                );
                if let Some(subtitle) = subtitle {
                    ui.add(
                        egui::Label::new(
                            RichText::new(subtitle)
                                .size(theme::SMALL)
                                .color(theme::TEXT_3),
                        )
                        .truncate(),
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Min), trailing);
        });
        ui.add_space(4.0);
        // `ui.columns` hands out justified layouts; cards keep natural
        // control widths regardless of where they are placed.
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            body(ui)
        })
        .inner
    });
    if let Some(color) = rail {
        let color = animation::color(ui.ctx(), rail_id, color);
        let rect = shown.response.rect;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 14.0),
            egui::vec2(3.0, (rect.height() - 28.0).max(8.0)),
        );
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }
    shown.inner
}

/// Inset row inside a card (list items, lanes).
pub fn inset<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::RAISED)
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui)
        })
        .inner
}

pub fn caption(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).size(theme::SMALL).color(theme::TEXT_2))
}

pub fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .size(theme::SMALL)
            .color(theme::TEXT_3),
    );
}

pub fn error_text(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(theme::DANGER));
}

pub fn mono(ui: &mut Ui, text: impl Into<String>) {
    ui.add(
        egui::Label::new(
            RichText::new(text.into())
                .monospace()
                .size(theme::SMALL)
                .color(theme::TEXT_3),
        )
        .wrap()
        .selectable(true),
    );
}

/// Labelled single-line input. The caption is the accessible name and
/// focuses the input when clicked. Enter never submits (IME safety).
pub fn field(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    label: &str,
    value: &mut String,
    secret: bool,
) -> Response {
    field_sized(ui, id_salt, label, value, secret, 440.0)
}

pub fn field_sized(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    label: &str,
    value: &mut String,
    secret: bool,
    max_width: f32,
) -> Response {
    let id = ui.make_persistent_id(("field", id_salt));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let caption = ui.add(
            egui::Label::new(RichText::new(label).size(theme::SMALL).color(theme::TEXT_2))
                .sense(egui::Sense::click()),
        );
        let response = ui
            .add(
                egui::TextEdit::singleline(value)
                    .id(id)
                    .password(secret)
                    .desired_width(ui.available_width().min(max_width))
                    .margin(egui::vec2(10.0, 7.0)),
            )
            .labelled_by(caption.id);
        if caption.clicked() {
            response.request_focus();
        }
        response
    })
    .inner
}

/// Inline-editable title: reads as a heading, edits in place. The small
/// caption above is its accessible name; an underline shows hover/focus.
pub fn title_field(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    label: &str,
    value: &mut String,
    hint: &str,
) -> Response {
    let id = ui.make_persistent_id(("title-field", id_salt));
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let caption = ui.add(
            egui::Label::new(RichText::new(label).size(theme::SMALL).color(theme::TEXT_3))
                .sense(egui::Sense::click()),
        );
        let response = ui
            .scope(|ui| {
                let v = &mut ui.style_mut().visuals;
                v.extreme_bg_color = Color32::TRANSPARENT;
                ui.add(
                    egui::TextEdit::singleline(value)
                        .id(id)
                        .frame(false)
                        .hint_text(RichText::new(hint).color(theme::TEXT_3))
                        .font(theme::heading(22.0))
                        .text_color(theme::TEXT)
                        .desired_width(ui.available_width().min(520.0))
                        .margin(egui::vec2(0.0, 2.0)),
                )
            })
            .inner
            .labelled_by(caption.id);
        if caption.clicked() {
            response.request_focus();
        }
        let line = animation::lerp_color(
            animation::lerp_color(
                Color32::TRANSPARENT,
                theme::BORDER_STRONG,
                ui.ctx()
                    .animate_bool_with_time(id.with("hover"), response.hovered(), 0.12),
            ),
            theme::ACCENT,
            ui.ctx()
                .animate_bool_with_time(id.with("focus"), response.has_focus(), 0.12),
        );
        let r = response.rect;
        ui.painter().line_segment(
            [
                r.left_bottom() + egui::vec2(0.0, 1.0),
                r.right_bottom() + egui::vec2(0.0, 1.0),
            ],
            Stroke::new(1.5, line),
        );
        response
    })
    .inner
}

pub fn label_combo(response: &Response, label: &str) {
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, response.enabled(), label)
    });
}

pub fn select_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<egui::WidgetText>,
) -> Response {
    let response = ui.selectable_value(current, value, label);
    if response.clicked() {
        // Returning focus to the parent must not deliver this activation key
        // again to the selector in another layout pass.
        ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
        });
        // ComboBox uses Memory's popup manager rather than Ui's menu_state.
        // Keyboard selection has no pointer click to close it automatically.
        ui.memory_mut(|memory| memory.close_popup());
        ui.close_menu();
    }
    response
}

/// Small rounded status label. Colour always accompanies text. Painted
/// directly so its size never depends on the surrounding layout.
pub fn pill(ui: &mut Ui, text: &str, tone: Tone) -> Response {
    pill_sized(ui, text, tone, f32::INFINITY)
}

/// A bounded status chip; its full value remains in accessibility output.
pub fn pill_sized(ui: &mut Ui, text: &str, tone: Tone, max_width: f32) -> Response {
    // The surrounding business scope plus control position owns animation;
    // localized display text never changes this identity.
    let color = animation::color(ui.ctx(), ui.next_auto_id().with("pill"), tone.color());
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL),
        color,
        (max_width - 18.0).max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let size = galley.size() + egui::vec2(18.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(255),
        color.gamma_multiply(0.14),
        Stroke::new(1.0, color.gamma_multiply(0.28)),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
}

/// Coloured dot followed by text, for compact state rows.
pub fn dot(ui: &mut Ui, text: &str, tone: Tone) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL),
        theme::TEXT_2,
    );
    let size = egui::vec2(14.0 + galley.size().x, galley.size().y.max(14.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let center = egui::pos2(rect.left() + 4.0, rect.center().y);
    if tone != Tone::Neutral {
        crate::fx::glow(ui.painter(), center, 5.0, tone.color().gamma_multiply(0.35));
    }
    ui.painter().circle_filled(center, 3.5, tone.color());
    ui.painter().galley(
        egui::pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::TEXT_2,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
}

/// Label/value rows in a two-column grid. Values are monospace so columns of
/// counters line up.
pub fn kv_grid<L: AsRef<str>>(ui: &mut Ui, id: &str, rows: &[(L, String)]) {
    ui.spacing_mut().interact_size.y = 18.0;
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 6.0])
        .min_col_width(96.0)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(
                    RichText::new(label.as_ref())
                        .size(theme::SMALL)
                        .color(theme::TEXT_2),
                );
                ui.label(
                    RichText::new(value)
                        .monospace()
                        .size(theme::MONO)
                        .color(theme::TEXT),
                );
                ui.end_row();
            }
        });
}

/// Inline metric: small caption above a value.
pub fn metric(ui: &mut Ui, label: &str, value: &str, tone: Option<Tone>) {
    let width = ui.fonts(|f| {
        let a = f.layout_no_wrap(
            label.to_owned(),
            egui::FontId::proportional(11.5),
            Color32::WHITE,
        );
        let b = f.layout_no_wrap(
            value.to_owned(),
            egui::FontId::monospace(theme::MONO + 0.5),
            Color32::WHITE,
        );
        a.size().x.max(b.size().x)
    });
    wrap_for(ui, width);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.label(RichText::new(label).size(11.5).color(theme::TEXT_3));
        ui.label(
            RichText::new(value)
                .monospace()
                .size(theme::MONO + 0.5)
                .color(tone.map_or(theme::TEXT, Tone::color)),
        );
    });
}

pub fn empty(ui: &mut Ui, title: &str, description: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(title)
                .font(theme::heading(theme::SECTION))
                .color(theme::TEXT_2),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(description)
                .size(theme::SMALL + 0.5)
                .color(theme::TEXT_3),
        );
    });
    ui.add_space(12.0);
}

pub const METER_FLOOR_DB: f32 = -60.0;

pub fn to_db(linear: f64) -> Option<f32> {
    (linear > 1e-6).then(|| (20.0 * linear.log10()) as f32)
}

pub fn db_text(linear: f64) -> String {
    to_db(linear).map_or_else(|| "−∞".into(), |db| format!("{db:.1}").replace('-', "−"))
}

pub fn gain_text(db: f32) -> String {
    if db <= GAIN_MIN + 0.05 {
        "−∞".to_owned()
    } else {
        format!("{db:+.1}").replace('-', "−")
    }
}

fn meter_x(rect: egui::Rect, db: f32) -> f32 {
    let t = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
    rect.left() + rect.width() * t
}

/// Paints the level ladder into `bar`: solid RMS, translucent peak and a
/// peak-hold tick, coloured by −18/−6 dBFS zones. Ballistics live in memory
/// under `id`, so the same meter keeps moving smoothly between polls.
pub fn paint_level(ui: &Ui, id: egui::Id, bar: egui::Rect, peak: Option<f64>, rms: Option<f64>) {
    let peak_db = peak.and_then(to_db).unwrap_or(METER_FLOOR_DB - 6.0);
    let rms_db = rms.and_then(to_db).unwrap_or(METER_FLOOR_DB - 6.0);
    let (peak_level, hold) = animation::meter(ui.ctx(), id.with("peak"), peak_db);
    let (rms_level, _) = animation::meter(ui.ctx(), id.with("rms"), rms_db);
    let painter = ui.painter();
    let r = CornerRadius::same((bar.height() / 2.0).min(4.0) as u8);
    painter.rect_filled(bar, r, theme::METER_TRACK);
    let zones = [
        (METER_FLOOR_DB, -18.0, theme::METER_LOW),
        (-18.0, -6.0, theme::METER_MID),
        (-6.0, 0.0, theme::METER_HIGH),
    ];
    let fill = |level: f32, alpha: f32| {
        for (from, to, color) in zones {
            if level <= from {
                break;
            }
            let segment = egui::Rect::from_x_y_ranges(
                meter_x(bar, from)..=meter_x(bar, level.min(to)),
                bar.y_range(),
            );
            painter.rect_filled(segment, CornerRadius::ZERO, color.gamma_multiply(alpha));
        }
    };
    if peak.is_some() {
        fill(peak_level, 0.35);
    }
    if rms.is_some() {
        fill(rms_level, 1.0);
    }
    if peak.is_some() && hold > METER_FLOOR_DB {
        let x = meter_x(bar, hold);
        painter.line_segment(
            [egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())],
            Stroke::new(2.0, theme::TEXT),
        );
    }
    // Segment gaps give the LED-ladder reading without per-cell drawing.
    let mut x = bar.left() + 4.0;
    while x < bar.right() {
        painter.line_segment(
            [egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())],
            Stroke::new(1.0, theme::METER_TRACK.gamma_multiply(0.85)),
        );
        x += 4.0;
    }
}

fn meter_y(rect: egui::Rect, db: f32) -> f32 {
    let t = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
    rect.bottom() - rect.height() * t
}

/// Vertical LED ladder for console strips; same zones and ballistics as
/// `paint_level`. One bar: the Hub publishes a combined stereo reading.
pub fn paint_level_vertical(
    ui: &Ui,
    id: egui::Id,
    bar: egui::Rect,
    peak: Option<f64>,
    rms: Option<f64>,
) {
    let peak_db = peak.and_then(to_db).unwrap_or(METER_FLOOR_DB - 6.0);
    let rms_db = rms.and_then(to_db).unwrap_or(METER_FLOOR_DB - 6.0);
    let (peak_level, hold) = animation::meter(ui.ctx(), id.with("peak"), peak_db);
    let (rms_level, _) = animation::meter(ui.ctx(), id.with("rms"), rms_db);
    let painter = ui.painter();
    painter.rect_filled(bar, CornerRadius::same(3), theme::METER_TRACK);
    let zones = [
        (METER_FLOOR_DB, -18.0, theme::METER_LOW),
        (-18.0, -6.0, theme::METER_MID),
        (-6.0, 0.0, theme::METER_HIGH),
    ];
    let fill = |level: f32, alpha: f32| {
        for (from, to, color) in zones {
            if level <= from {
                break;
            }
            let segment = egui::Rect::from_x_y_ranges(
                bar.x_range(),
                meter_y(bar, level.min(to))..=meter_y(bar, from),
            );
            painter.rect_filled(segment, CornerRadius::ZERO, color.gamma_multiply(alpha));
        }
    };
    if peak.is_some() {
        fill(peak_level, 0.35);
    }
    if rms.is_some() {
        fill(rms_level, 1.0);
    } else {
        painter.rect_stroke(
            bar,
            CornerRadius::same(3),
            Stroke::new(1.0, theme::TEXT_3.gamma_multiply(0.35)),
            StrokeKind::Inside,
        );
    }
    if peak.is_some() && hold > METER_FLOOR_DB {
        let y = meter_y(bar, hold);
        painter.line_segment(
            [egui::pos2(bar.left(), y), egui::pos2(bar.right(), y)],
            Stroke::new(2.0, theme::TEXT),
        );
    }
    let mut y = bar.bottom() - 4.0;
    while y > bar.top() {
        painter.line_segment(
            [egui::pos2(bar.left(), y), egui::pos2(bar.right(), y)],
            Stroke::new(1.0, theme::METER_TRACK.gamma_multiply(0.85)),
        );
        y -= 4.0;
    }
}

/// dBFS labels to the left of a vertical meter.
pub fn paint_scale_vertical(painter: &egui::Painter, bar: egui::Rect) {
    for (db, label) in [
        (-48.0, "−48"),
        (-24.0, "−24"),
        (-12.0, "−12"),
        (-6.0, "−6"),
        (0.0, "0"),
    ] {
        let y = meter_y(bar, db);
        painter.line_segment(
            [
                egui::pos2(bar.left() - 4.0, y),
                egui::pos2(bar.left() - 1.0, y),
            ],
            Stroke::new(1.0, theme::TEXT_3),
        );
        painter.text(
            egui::pos2(bar.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            egui::FontId::proportional(9.5),
            theme::TEXT_3,
        );
    }
}

/// Limiter gain reduction, drawn down from the top (0…12 dB).
pub fn paint_reduction(painter: &egui::Painter, rect: egui::Rect, limiter_gain: Option<f64>) {
    painter.rect_filled(rect, CornerRadius::same(2), theme::METER_TRACK);
    if let Some(g) = limiter_gain.filter(|g| *g > 0.0 && *g < 0.999) {
        let reduction = (-20.0 * g.log10()) as f32;
        let h = rect.height() * (reduction / 12.0).clamp(0.02, 1.0);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h)),
            CornerRadius::same(2),
            theme::WARNING,
        );
    }
}

fn paint_scale(ui: &Ui, bar: egui::Rect) {
    let painter = ui.painter();
    for (db, label) in [
        (-48.0, "−48"),
        (-36.0, "−36"),
        (-24.0, "−24"),
        (-18.0, "−18"),
        (-12.0, "−12"),
        (-6.0, "−6"),
        (0.0, "0"),
    ] {
        let x = meter_x(bar, db);
        painter.line_segment(
            [
                egui::pos2(x, bar.bottom() + 2.0),
                egui::pos2(x, bar.bottom() + 5.0),
            ],
            Stroke::new(1.0, theme::TEXT_3),
        );
        if bar.width() > 260.0 || matches!(label, "−48" | "−24" | "−12" | "0") {
            let align = if db == 0.0 {
                egui::Align2::RIGHT_TOP
            } else {
                egui::Align2::CENTER_TOP
            };
            painter.text(
                egui::pos2(x, bar.bottom() + 5.0),
                align,
                label,
                egui::FontId::proportional(9.5),
                theme::TEXT_3,
            );
        }
    }
}

fn readout(ui: &mut Ui, peak: Option<f64>, rms: Option<f64>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        match peak {
            Some(p) => {
                let hot = to_db(p).is_some_and(|db| db > -6.0);
                ui.label(
                    RichText::new(text(ui, &Message::WidgetsPeak))
                        .size(11.5)
                        .color(theme::TEXT_3),
                );
                ui.label(
                    RichText::new(db_text(p))
                        .monospace()
                        .size(theme::MONO)
                        .color(if hot { theme::METER_HIGH } else { theme::TEXT }),
                );
            }
            None => {
                ui.label(
                    RichText::new(text(ui, &Message::WidgetsLevelUnavailable))
                        .size(11.5)
                        .color(theme::TEXT_3),
                );
            }
        }
        if let Some(r) = rms {
            ui.label(RichText::new("RMS").size(11.5).color(theme::TEXT_3));
            ui.label(
                RichText::new(db_text(r))
                    .monospace()
                    .size(theme::MONO)
                    .color(theme::TEXT_2),
            );
        }
        if peak.is_some() || rms.is_some() {
            ui.label(RichText::new("dBFS").size(11.0).color(theme::TEXT_3));
        }
    });
}

/// Full meter with scale and numeric readout (beside when wide, below when
/// narrow). Readings never sit on the coloured fill.
pub fn meter(ui: &mut Ui, id: impl std::hash::Hash, peak: Option<f64>, rms: Option<f64>) {
    meter_sized(ui, id, peak, rms, 10.0);
}

pub fn meter_sized(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    peak: Option<f64>,
    rms: Option<f64>,
    height: f32,
) {
    let id = ui.id().with(("meter", id));
    let readout_width = 176.0;
    let gap = ui.spacing().item_spacing.x;
    let wide = ui.available_width() >= 460.0;
    let bar_width = if wide {
        ui.available_width() - readout_width - gap
    } else {
        ui.available_width()
    };
    let paint_bar = |ui: &mut Ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(bar_width, height + 12.0), egui::Sense::hover());
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), height));
        paint_level(ui, id, bar, peak, rms);
        paint_scale(ui, bar);
    };
    if wide {
        ui.horizontal(|ui| {
            paint_bar(ui);
            ui.allocate_ui_with_layout(
                egui::vec2(readout_width, height + 12.0),
                Layout::left_to_right(Align::Min),
                |ui| readout(ui, peak, rms),
            );
        });
    } else {
        paint_bar(ui);
        readout(ui, peak, rms);
    }
}

/// Bare level lane for dense rows and the top bar: no scale, no readout.
pub fn level(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    size: egui::Vec2,
    peak: Option<f64>,
    rms: Option<f64>,
) -> Response {
    let id = ui.id().with(("level", id));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    paint_level(ui, id, rect, peak, rms);
    response
}

pub const GAIN_MIN: f32 = -96.0;
pub const GAIN_MAX: f32 = 12.0;
/// Fader taper: the lowest 20 % of travel covers −96…−48 dB, the rest
/// −48…+12 dB, so the useful range gets most of the throw (0 dB at 84 %).
const KNEE_DB: f32 = -48.0;
const KNEE_T: f32 = 0.2;

pub(crate) fn gain_to_t(db: f32) -> f32 {
    let db = db.clamp(GAIN_MIN, GAIN_MAX);
    if db <= KNEE_DB {
        KNEE_T * (db - GAIN_MIN) / (KNEE_DB - GAIN_MIN)
    } else {
        KNEE_T + (1.0 - KNEE_T) * (db - KNEE_DB) / (GAIN_MAX - KNEE_DB)
    }
}

fn t_to_gain(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let db = if t <= KNEE_T {
        GAIN_MIN + (KNEE_DB - GAIN_MIN) * t / KNEE_T
    } else {
        KNEE_DB + (GAIN_MAX - KNEE_DB) * (t - KNEE_T) / (1.0 - KNEE_T)
    };
    (db * 2.0).round() / 2.0
}

#[derive(Clone, Copy, PartialEq)]
pub enum FaderSize {
    Hero,
    Row,
    /// Vertical console fader of the given travel height.
    Strip(f32),
}

pub struct Fader {
    pub response: Response,
    /// A value the user settled on: drag released, key step, scroll step,
    /// double-click reset or an accessibility set/increment.
    pub committed: bool,
}

#[derive(Clone, Copy)]
struct Drag {
    start_t: f32,
    /// Pointer position along the travel axis when the grab started.
    start: f32,
}

/// Gain fader. Dragging is relative to where the grab started, so a click
/// never jumps the level of a live room; Shift drags finely. Double-click
/// returns to 0 dB. ←/→ step 0.5 dB (Shift: 3 dB); a vertical strip steps
/// with ↑/↓ instead. The wheel adjusts only
/// when the fader has focus or Option is held, so scrolling the page is safe.
/// Exposed to assistive tech as a slider with set/increment/decrement.
pub fn fader(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    value: &mut f32,
    label: &str,
    size: FaderSize,
) -> Fader {
    let id = ui.make_persistent_id(("fader", id_salt));
    let vertical = matches!(size, FaderSize::Strip(_));
    let (height, length) = match size {
        FaderSize::Hero => (30.0, ui.available_width().max(80.0)),
        FaderSize::Row => (24.0, ui.available_width().max(80.0)),
        FaderSize::Strip(travel) => (30.0, travel),
    };
    let (_, rect) = ui.allocate_space(if vertical {
        egui::vec2(height, length)
    } else {
        egui::vec2(length, height)
    });
    let mut response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let enabled = ui.is_enabled();
    let handle_w = if size == FaderSize::Hero { 16.0 } else { 12.0 };
    let track = if vertical {
        rect.shrink2(egui::vec2(0.0, handle_w / 2.0))
    } else {
        rect.shrink2(egui::vec2(handle_w / 2.0, 0.0))
    };
    // Travel position 0…1 → screen coordinate and pointer → travel delta.
    let along = |p: egui::Pos2| if vertical { -p.y } else { p.x };
    let span = if vertical {
        track.height()
    } else {
        track.width()
    };
    let before = *value;
    let mut committed = false;
    let set = |v: f32, value: &mut f32| {
        let v = v.clamp(GAIN_MIN, GAIN_MAX);
        if (v - *value).abs() > f32::EPSILON {
            *value = v;
        }
    };

    if enabled {
        if response.drag_started()
            && let Some(pos) = response.interact_pointer_pos()
        {
            ui.data_mut(|d| {
                d.insert_temp(
                    id,
                    Drag {
                        start_t: gain_to_t(*value),
                        start: along(pos),
                    },
                )
            });
        }
        if response.dragged()
            && let (Some(drag), Some(pos)) = (
                ui.data(|d| d.get_temp::<Drag>(id)),
                response.interact_pointer_pos(),
            )
        {
            let fine = if ui.input(|i| i.modifiers.shift) {
                0.25
            } else {
                1.0
            };
            let t = drag.start_t + (along(pos) - drag.start) / span * fine;
            set(t_to_gain(t), value);
        }
        if response.drag_stopped() {
            committed = true;
        }
        if response.double_clicked() {
            set(0.0, value);
            committed = true;
        }
        if response.has_focus() {
            let step = |i: &mut egui::InputState, key| {
                if i.consume_key(egui::Modifiers::SHIFT, key) {
                    Some(3.0)
                } else if i.consume_key(egui::Modifiers::NONE, key) {
                    Some(0.5)
                } else {
                    None
                }
            };
            let (up, down) = if vertical {
                (egui::Key::ArrowUp, egui::Key::ArrowDown)
            } else {
                (egui::Key::ArrowRight, egui::Key::ArrowLeft)
            };
            let delta = ui.input_mut(|i| {
                step(i, up)
                    .or_else(|| step(i, down).map(|s| -s))
                    .or_else(|| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Num0)
                            .then_some(f32::NAN)
                    })
            });
            match delta {
                Some(d) if d.is_nan() => {
                    set(0.0, value);
                    committed = true;
                }
                Some(d) => {
                    set(*value + d, value);
                    committed = true;
                }
                None => {}
            }
        }
        // Wheel: only when intentional (focused or Option), and consumed so the
        // page does not scroll at the same time.
        if response.hovered() && (response.has_focus() || ui.input(|i| i.modifiers.alt)) {
            let delta = ui.input(|i| i.smooth_scroll_delta.x + i.smooth_scroll_delta.y);
            if delta != 0.0 {
                let acc_id = id.with("wheel");
                let mut acc = ui.data(|d| d.get_temp::<f32>(acc_id)).unwrap_or(0.0) + delta;
                while acc.abs() >= 12.0 {
                    set(*value + 0.5 * acc.signum(), value);
                    acc -= 12.0 * acc.signum();
                    committed = true;
                }
                ui.data_mut(|d| d.insert_temp(acc_id, acc));
                ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        }
        let requests: Vec<_> = ui.input(|i| {
            i.accesskit_action_requests(id, egui::accesskit::Action::SetValue)
                .chain(i.accesskit_action_requests(id, egui::accesskit::Action::Increment))
                .chain(i.accesskit_action_requests(id, egui::accesskit::Action::Decrement))
                .cloned()
                .collect()
        });
        for request in requests {
            match request.action {
                egui::accesskit::Action::Increment => set(*value + 0.5, value),
                egui::accesskit::Action::Decrement => set(*value - 0.5, value),
                _ => {
                    if let Some(egui::accesskit::ActionData::NumericValue(v)) = request.data {
                        set(((v as f32) * 2.0).round() / 2.0, value);
                    }
                }
            }
            committed = true;
        }
    }
    if (*value - before).abs() > f32::EPSILON {
        response.mark_changed();
        // Read the clock first: nesting `input` inside `data_mut` would
        // take two context locks at once and deadlock.
        let now = ui.input(|i| i.time);
        ui.data_mut(|d| d.insert_temp(id.with("changed_at"), now));
    }
    let shown = *value;
    response.widget_info(|| egui::WidgetInfo::slider(enabled, shown as f64, label));

    // Paint.
    let painter = ui.painter();
    let rail_h = if size == FaderSize::Hero { 6.0 } else { 4.0 };
    // `at(t)` is the screen position along the travel; `cross` the centre
    // line across it.
    let at = |t: f32| {
        if vertical {
            track.bottom() - track.height() * t
        } else {
            track.left() + track.width() * t
        }
    };
    let rail = if vertical {
        egui::Rect::from_center_size(
            egui::pos2(rect.center().x, track.center().y),
            egui::vec2(rail_h, track.height()),
        )
    } else {
        egui::Rect::from_center_size(
            egui::pos2(track.center().x, rect.center().y),
            egui::vec2(track.width(), rail_h),
        )
    };
    let x = at(gain_to_t(shown));
    let hot = ui.ctx().animate_bool_with_time(
        id.with("hot"),
        response.hovered() || response.dragged(),
        0.12,
    );
    painter.rect_filled(rail, CornerRadius::same(3), theme::METER_TRACK);
    painter.rect_stroke(
        rail,
        CornerRadius::same(3),
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Outside,
    );
    let mut filled = rail;
    if vertical {
        filled.min.y = x;
    } else {
        filled.max.x = x;
    }
    let fill = if shown > 0.0 {
        theme::WARNING
    } else {
        theme::ACCENT
    };
    let fill = if enabled { fill } else { theme::TEXT_3 };
    painter.rect_filled(filled, CornerRadius::same(3), fill.gamma_multiply(0.9));
    let tick = |p: f32, from: f32, to: f32, stroke: Stroke| {
        let seg = if vertical {
            [
                egui::pos2(rail.center().x + from, p),
                egui::pos2(rail.center().x + to, p),
            ]
        } else {
            [
                egui::pos2(p, rail.center().y + from),
                egui::pos2(p, rail.center().y + to),
            ]
        };
        painter.line_segment(seg, stroke);
    };
    let unity = at(gain_to_t(0.0));
    tick(
        unity,
        -rail_h / 2.0 - 5.0,
        rail_h / 2.0 + 5.0,
        Stroke::new(1.0, theme::TEXT_3),
    );
    if size != FaderSize::Row {
        for db in [-48.0, -24.0, -12.0, -6.0, 6.0] {
            tick(
                at(gain_to_t(db)),
                rail_h / 2.0 + 3.0,
                rail_h / 2.0 + 6.0,
                Stroke::new(1.0, theme::BORDER_STRONG),
            );
        }
    }
    let handle = if vertical {
        egui::Rect::from_center_size(
            egui::pos2(rect.center().x, x),
            egui::vec2(height - 4.0 + 2.0 * hot, handle_w + 2.0 * hot),
        )
    } else {
        egui::Rect::from_center_size(
            egui::pos2(x, rect.center().y),
            egui::vec2(handle_w + 2.0 * hot, height - 4.0 + 2.0 * hot),
        )
    };
    paint_knob(painter, handle, fill, hot, enabled, vertical);
    paint_focus(ui, &response);
    // Value bubble while adjusting; drawn above neighbours on the tooltip layer.
    let changed_at = ui
        .data(|d| d.get_temp::<f64>(id.with("changed_at")))
        .unwrap_or(f64::NEG_INFINITY);
    let recent = ui.input(|i| i.time) - changed_at < 0.9;
    let bubble =
        ui.ctx()
            .animate_bool_with_time(id.with("bubble"), response.dragged() || recent, 0.12);
    if recent {
        ui.ctx().request_repaint();
    }
    if bubble > 0.01 {
        let text = format!("{} dB", gain_text(shown));
        let layer = egui::LayerId::new(egui::Order::Tooltip, id.with("bubble-layer"));
        let top = ui.ctx().layer_painter(layer);
        let galley = top.layout_no_wrap(
            text,
            egui::FontId::monospace(12.0),
            theme::ON_ACCENT.gamma_multiply(bubble),
        );
        let size = galley.size() + egui::vec2(12.0, 6.0);
        let bubble_rect = if vertical {
            let left = handle.right() + 6.0 + 4.0 * (1.0 - bubble);
            egui::Rect::from_min_size(egui::pos2(left, x - size.y / 2.0), size)
        } else {
            let at = egui::pos2(x, handle.top() - 6.0 - 4.0 * (1.0 - bubble));
            egui::Rect::from_center_size(egui::pos2(at.x, at.y - size.y / 2.0), size)
        };
        top.rect_filled(
            bubble_rect,
            CornerRadius::same(5),
            theme::ACCENT.gamma_multiply(bubble),
        );
        top.galley(
            bubble_rect.center() - galley.size() / 2.0,
            galley,
            theme::ON_ACCENT,
        );
    }
    Fader {
        response,
        committed,
    }
}

/// Fader cap: dark body, lit edge, a centre line in the fill colour and a
/// halo while hovered or dragged. `vertical` turns the centre line sideways.
fn paint_knob(
    painter: &egui::Painter,
    handle: egui::Rect,
    color: Color32,
    hot: f32,
    enabled: bool,
    vertical: bool,
) {
    if enabled && hot > 0.0 {
        crate::fx::glow(
            painter,
            handle.center(),
            handle.width().max(handle.height()) * 0.7,
            color.gamma_multiply(0.22 * hot),
        );
    }
    painter.rect(
        handle,
        CornerRadius::same(5),
        animation::lerp_color(theme::RAISED, theme::HOVER, hot),
        Stroke::new(
            1.0,
            animation::lerp_color(theme::BORDER_STRONG, color, 0.35 + 0.4 * hot),
        ),
        StrokeKind::Inside,
    );
    let c = handle.center();
    let line = if vertical {
        [
            egui::pos2(handle.left() + 4.0, c.y),
            egui::pos2(handle.right() - 4.0, c.y),
        ]
    } else {
        [
            egui::pos2(c.x, handle.top() + 4.0),
            egui::pos2(c.x, handle.bottom() - 4.0),
        ]
    };
    painter.line_segment(
        line,
        Stroke::new(2.0, if enabled { color } else { theme::TEXT_3 }),
    );
}

/// Section label inside a page, with optional trailing content.
pub fn section(ui: &mut Ui, title: &str, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .font(theme::heading(theme::SECTION))
                .color(theme::TEXT),
        );
        trailing(ui);
    });
}

pub struct Panel {
    pub toggled: bool,
    pub rect: egui::Rect,
}

/// Collapsible card. The whole header toggles; the body reveals with a height
/// animation. Open state is owned by the caller so defaults can follow data
/// (e.g. pairing collapses once paired) while user choices stick.
pub fn panel(
    ui: &mut Ui,
    key: &str,
    title: &str,
    subtitle: Option<&str>,
    open: bool,
    trailing: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) -> Panel {
    let id = ui.make_persistent_id(("panel", key));
    let mut toggled = false;
    let margin = Margin {
        left: 16,
        right: 16,
        top: 12,
        bottom: 12,
    };
    let shown = surface(ui, None, margin, |ui| {
        ui.set_min_width(ui.available_width());
        let mut state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
        state.set_open(open);
        let openness = state.openness(ui.ctx());
        let header = ui.horizontal_top(|ui| {
            let (chev, _) = ui.allocate_exact_size(egui::vec2(16.0, 20.0), egui::Sense::hover());
            crate::icons::chevron(ui.painter(), chev, openness, theme::TEXT_2);
            // Leave room for trailing pills; long text wraps rather than
            // widening the panel past its column.
            let reserve = (ui.available_width() * 0.35).clamp(90.0, 180.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - reserve);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(title)
                            .font(theme::heading(theme::SECTION))
                            .color(theme::TEXT),
                    )
                    .selectable(false),
                );
                if let Some(subtitle) = subtitle {
                    ui.add(
                        egui::Label::new(
                            RichText::new(subtitle)
                                .size(theme::SMALL)
                                .color(theme::TEXT_3),
                        )
                        .wrap()
                        .selectable(false),
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Min), trailing);
        });
        // The header area toggles; trailing widgets drawn later win clicks.
        let hit = ui.interact(
            header.response.rect,
            id.with("toggle"),
            egui::Sense::click(),
        );
        hit.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, title));
        if hit.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if hit.clicked() {
            toggled = true;
        }
        paint_focus(ui, &hit);
        state.show_body_unindented(ui, |ui| {
            ui.add_space(10.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                body(ui)
            })
        });
    });
    Panel {
        toggled,
        rect: shown.response.rect,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Step {
    Done,
    Current,
    Todo,
}

/// Setup progress: numbered nodes joined by a line; done steps show a check.
/// Returns the step the user picked (to open its section).
pub fn stepper(ui: &mut Ui, steps: &[(&str, Step)]) -> Option<usize> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, (label, step)) in steps.iter().enumerate() {
            if i > 0 {
                let (line, _) =
                    ui.allocate_exact_size(egui::vec2(22.0, 24.0), egui::Sense::hover());
                let done = steps[i - 1].1 == Step::Done;
                ui.painter().line_segment(
                    [line.left_center(), line.right_center()],
                    Stroke::new(
                        1.5,
                        if done {
                            theme::SUCCESS.gamma_multiply(0.7)
                        } else {
                            theme::BORDER_STRONG
                        },
                    ),
                );
            }
            let galley = ui.painter().layout_no_wrap(
                label.to_string(),
                egui::FontId::proportional(theme::SMALL + 1.0),
                theme::TEXT,
            );
            let size = egui::vec2(28.0 + galley.size().x + 6.0, 24.0);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            let state = match step {
                Step::Done => text(ui, &Message::WidgetsStepDone),
                Step::Current => text(ui, &Message::WidgetsStepCurrent),
                Step::Todo => text(ui, &Message::WidgetsStepTodo),
            };
            let access_label = text(
                ui,
                &Message::WidgetsStepAccessible {
                    number: (i + 1) as u64,
                    label: (*label).into(),
                    state,
                },
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, access_label.clone())
            });
            let circle = egui::pos2(rect.left() + 12.0, rect.center().y);
            let (fill, ring, text) = match step {
                Step::Done => (theme::SUCCESS, theme::SUCCESS, theme::TEXT_2),
                Step::Current => (theme::ACCENT, theme::ACCENT, theme::TEXT),
                Step::Todo => (Color32::TRANSPARENT, theme::BORDER_STRONG, theme::TEXT_3),
            };
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect.expand(2.0), CornerRadius::same(12), theme::HOVER);
            }
            painter.circle(
                circle,
                10.0,
                fill.gamma_multiply(0.2),
                Stroke::new(1.5, ring),
            );
            if *step == Step::Done {
                crate::icons::paint(
                    painter,
                    crate::icons::square(circle, 13.0),
                    crate::icons::Icon::Check,
                    theme::SUCCESS,
                );
            } else {
                painter.text(
                    circle,
                    egui::Align2::CENTER_CENTER,
                    (i + 1).to_string(),
                    egui::FontId::proportional(11.5),
                    ring,
                );
            }
            painter.galley(
                egui::pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
                galley,
                text,
            );
            paint_focus(ui, &response);
            if response.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// Filter chips with counts. Returns the chip picked this frame.
pub fn chips(ui: &mut Ui, current: usize, items: &[(&str, usize)]) -> Option<usize> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, (label, count)) in items.iter().enumerate() {
            let on = i == current;
            let text = format!("{label} {count}");
            let palette = if on {
                Palette {
                    base: theme::ACCENT.gamma_multiply(0.18),
                    hover: theme::ACCENT.gamma_multiply(0.24),
                    press: theme::ACCENT.gamma_multiply(0.3),
                    text: theme::ACCENT_HOVER,
                    stroke: theme::ACCENT.gamma_multiply(0.55),
                }
            } else {
                Palette {
                    base: Color32::TRANSPARENT,
                    hover: theme::HOVER,
                    press: theme::RAISED,
                    text: theme::TEXT_2,
                    stroke: theme::BORDER_STRONG,
                }
            };
            let response = draw(
                ui,
                Spec {
                    compact: true,
                    selected: Some(on),
                    radius: CornerRadius::same(255),
                    ..Spec::new(&text, palette)
                },
            );
            if response.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// Initial in a tinted circle; colour carries role, the letter carries name.
pub fn avatar(ui: &mut Ui, name: &str, color: Color32, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let initial: String = name
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into());
    ui.painter()
        .circle_filled(rect.center(), size / 2.0, color.gamma_multiply(0.2));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        theme::heading(size * 0.45),
        color,
    );
    response
}

/// Overview tile: tone dot + title, a prominent value, one line of detail.
/// Clicking opens the matching detail section.
pub fn tile(
    ui: &mut Ui,
    width: f32,
    title: &str,
    value: &str,
    detail: &str,
    tone: Tone,
    selected: bool,
) -> Response {
    let id = ui.next_auto_id();
    let (hover, _) = animation::interaction(ui.ctx(), id);
    let fill = animation::lerp_color(
        theme::SURFACE,
        theme::RAISED,
        hover.max(selected as u8 as f32),
    );
    let stroke = if selected {
        tone.color().gamma_multiply(0.6)
    } else {
        theme::BORDER
    };
    let halo = ui.painter().add(egui::Shape::Noop);
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(width - 30.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                dot(ui, title, tone);
                ui.label(RichText::new(value).font(theme::heading(20.0)).color(
                    if tone == Tone::Neutral || tone == Tone::Success {
                        theme::TEXT
                    } else {
                        tone.color()
                    },
                ));
                ui.add(
                    egui::Label::new(
                        RichText::new(detail)
                            .size(theme::SMALL)
                            .color(theme::TEXT_3),
                    )
                    .truncate(),
                );
            });
        });
    if matches!(tone, Tone::Warning | Tone::Danger) {
        // Something needs attention: the tile glows in its state colour.
        ui.painter().set(
            halo,
            egui::epaint::RectShape::filled(
                frame.response.rect,
                CornerRadius::same(theme::RADIUS),
                tone.color().gamma_multiply(0.14),
            )
            .with_blur_width(16.0),
        );
    }
    let response = ui.interact(frame.response.rect, id, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{title}：{value}"))
    });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_focus(ui, &response);
    response
}

/// Key cap hint, e.g. ⌘K.
pub fn kbd(ui: &mut Ui, keys: &str) {
    let galley = ui.painter().layout_no_wrap(
        keys.to_owned(),
        egui::FontId::monospace(11.0),
        theme::TEXT_2,
    );
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        theme::RAISED,
        Stroke::new(1.0, theme::BORDER_STRONG),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme::TEXT_2);
}

/// Number of equal columns of at least `min_width` that fit.
pub fn columns_for(ui: &Ui, min_width: f32, max: usize) -> usize {
    let gap = ui.spacing().item_spacing.x;
    (((ui.available_width() + gap) / (min_width + gap)).floor() as usize).clamp(1, max)
}

/// Match egui's COMMAND modifier on each desktop platform.
pub fn command_hint(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

#[cfg(test)]
mod localization_tests {
    use super::*;

    #[test]
    fn translated_field_labels_keep_focus_draft_and_identity() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut draft = "未保存的中文草稿".to_owned();
        let mut first = None;
        for (frame, caption) in ["设备名称", "Device name"].into_iter().enumerate() {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = field(ui, "device-name", caption, &mut draft, false);
                    if frame == 0 {
                        response.request_focus();
                        first = Some(response.id);
                    } else {
                        assert_eq!(Some(response.id), first);
                        assert!(response.has_focus());
                    }
                });
            });
        }
        assert_eq!(draft, "未保存的中文草稿");
    }

    #[test]
    fn identical_display_labels_do_not_alias_business_field_ids() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let a = field(ui, "first-name", "Name", &mut String::new(), false).id;
                let b = field(ui, "second-name", "Name", &mut String::new(), false).id;
                assert_ne!(a, b);
            });
        });
    }

    #[test]
    fn keyboard_combo_selection_closes_popup_and_returns_focus() {
        fn draw(
            ctx: &egui::Context,
            input: egui::RawInput,
            selected: &mut bool,
        ) -> (egui::Id, Option<egui::Id>) {
            let original = *selected;
            let mut combo_id = None;
            let mut option_id = None;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let shown = egui::ComboBox::from_id_salt("keyboard-language")
                        .selected_text(if *selected { "English" } else { "简体中文" })
                        .show_ui(ui, |ui| {
                            select_value(ui, selected, false, "简体中文");
                            option_id = Some(select_value(ui, selected, true, "English").id);
                        });
                    combo_id = Some(shown.response.id);
                    if *selected {
                        shown.response.request_focus();
                    }
                    if *selected != original {
                        ctx.request_discard("simulate locale layout change");
                    }
                });
            });
            (combo_id.unwrap(), option_id)
        }
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut selected = false;
        let (combo_id, _) = draw(&ctx, egui::RawInput::default(), &mut selected);
        ctx.memory_mut(|memory| memory.open_popup(combo_id.with("popup")));
        let (_, option) = draw(&ctx, egui::RawInput::default(), &mut selected);
        let option_id = option.expect("open popup must publish the English option");
        ctx.memory_mut(|memory| memory.request_focus(option_id));
        let key = |pressed| egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        draw(&ctx, key(true), &mut selected);
        assert!(selected, "Space must select the focused option");
        assert!(
            !ctx.input(|input| input.key_pressed(egui::Key::Space)),
            "handled Space must be consumed before parent focus returns"
        );
        assert!(
            !egui::ComboBox::is_open(&ctx, combo_id),
            "keyboard selection must close the popup"
        );
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(combo_id));
        draw(&ctx, key(false), &mut selected);
        assert!(selected);
        assert!(
            !egui::ComboBox::is_open(&ctx, combo_id),
            "keyup must not reopen the popup"
        );
    }

    #[test]
    fn translated_inline_title_keeps_identity_and_focus() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut title = "房间".to_owned();
        let mut first = None;
        for (frame, caption) in ["房间名称", "Room name"].into_iter().enumerate() {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = title_field(ui, "room-name", caption, &mut title, "");
                    if frame == 0 {
                        response.request_focus();
                        first = Some(response.id);
                    } else {
                        assert_eq!(Some(response.id), first);
                        assert!(response.has_focus());
                    }
                });
            });
        }
        assert_eq!(title, "房间");
    }
}
