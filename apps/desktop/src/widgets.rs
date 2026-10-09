//! Shared controls. Colours come from `theme`; elevation from `fx`; motion
//! from `animation`.
use crate::animation;
use crate::fx::{self, Level};
use crate::localization::text;
use crate::theme;
use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Margin, Response, RichText, Stroke, StrokeKind, Ui,
    epaint::RectShape,
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
            Self::Neutral => theme::text_3(),
            Self::Accent => theme::accent(),
            Self::Success => theme::success(),
            Self::Warning => theme::warning(),
            Self::Danger => theme::danger(),
            Self::Solo => theme::solo(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// The one main action of a view: solid, high-contrast cap.
    Primary,
    /// Raised neutral button.
    Secondary,
    Danger,
    /// Destructive action that should not dominate (tinted, not raised).
    Quiet,
    /// Text-weight action; only hover shows its shape.
    Ghost,
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
            base: theme::primary(),
            hover: theme::primary_hover(),
            press: theme::primary_press(),
            text: theme::on_primary(),
            stroke: Color32::TRANSPARENT,
        },
        Kind::Secondary => Palette {
            base: theme::raised(),
            hover: theme::hover(),
            press: theme::surface(),
            text: theme::text(),
            stroke: theme::border_strong(),
        },
        Kind::Danger => Palette {
            base: theme::danger(),
            hover: theme::danger_hover(),
            press: theme::danger().gamma_multiply(0.85),
            text: theme::on_danger(),
            stroke: Color32::TRANSPARENT,
        },
        Kind::Quiet => Palette {
            base: theme::danger().gamma_multiply(0.10),
            hover: theme::danger().gamma_multiply(0.17),
            press: theme::danger().gamma_multiply(0.24),
            text: danger_text(),
            stroke: theme::danger().gamma_multiply(0.32),
        },
        Kind::Ghost => Palette {
            base: Color32::TRANSPARENT,
            hover: theme::hover().gamma_multiply(0.85),
            press: theme::raised(),
            text: theme::text_2(),
            stroke: Color32::TRANSPARENT,
        },
    }
}

/// Danger as text: the brighter tint on dark, the base on light paper.
fn danger_text() -> Color32 {
    if theme::is_light() {
        theme::danger()
    } else {
        theme::danger_hover()
    }
}

/// Latched state (Mute/Solo, selected chips): lit and pressed in, so it
/// carries no shadow, a tinted fill, a ring and text in its tone.
fn engaged(color: Color32) -> Palette {
    Palette {
        base: color.gamma_multiply(0.16),
        hover: color.gamma_multiply(0.22),
        press: color.gamma_multiply(0.28),
        text: color,
        stroke: color.gamma_multiply(0.48),
    }
}

fn unavailable() -> Palette {
    Palette {
        base: theme::surface(),
        hover: theme::surface(),
        press: theme::surface(),
        text: theme::text_3(),
        stroke: theme::border(),
    }
}

fn paint_focus(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            CornerRadius::same(theme::CONTROL_RADIUS + 2),
            Stroke::new(2.0, theme::accent().gamma_multiply(0.85)),
            StrokeKind::Outside,
        );
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Height {
    Regular,
    Compact,
    /// Segment inside a well (26px).
    Inner,
}

/// Keyboard focus ring for custom-painted controls.
pub fn focus_ring(ui: &Ui, response: &Response) {
    paint_focus(ui, response);
}

struct Spec<'a> {
    text: &'a str,
    palette: Palette,
    enabled: bool,
    selected: Option<bool>,
    height: Height,
    /// Work started by this control is running: keep its colour and size,
    /// swap the label for a spinner and ignore further clicks.
    busy: bool,
    radius: CornerRadius,
    /// Lit state: a soft halo of its own colour.
    halo: Option<Color32>,
}

impl<'a> Spec<'a> {
    fn new(text: &'a str, palette: Palette) -> Self {
        Self {
            text,
            palette,
            enabled: true,
            selected: None,
            height: Height::Regular,
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

fn metrics(height: Height, ui: &Ui) -> (f32, f32, f32) {
    match height {
        Height::Regular => (
            theme::BODY,
            theme::CONTROL_HEIGHT,
            ui.spacing().button_padding.x,
        ),
        Height::Compact => (theme::SMALL + 0.5, theme::COMPACT_HEIGHT, 10.0),
        Height::Inner => (theme::SMALL + 0.5, 26.0, 11.0),
    }
}

/// Button body: an opaque palette is a raised cap with a contact shadow
/// that sinks while pressed; a translucent one (latched, quiet, ghost) is
/// flat and reads as pressed in.
fn button_shapes(
    ctx: &egui::Context,
    rect: egui::Rect,
    radius: CornerRadius,
    fill: Color32,
    stroke: Color32,
    raised: f32,
) -> Vec<egui::Shape> {
    let mut shapes = Vec::new();
    if raised > 0.01 {
        let r = radius.nw.max(radius.ne);
        for shape in fx::shadow(rect, r, Level::Control) {
            if let egui::Shape::Rect(mut s) = shape {
                s.fill = s.fill.gamma_multiply(raised);
                s.corner_radius = radius;
                shapes.push(egui::Shape::Rect(s));
            }
        }
        shapes.push(egui::Shape::Rect(
            RectShape::filled(rect, radius, fill).with_texture(fx::ramp_texture(ctx), fx::FULL_UV),
        ));
        shapes.push(egui::Shape::Rect(RectShape::stroke(
            rect,
            radius,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        )));
        let inset = r as f32;
        if rect.width() > 2.0 * inset {
            shapes.push(egui::Shape::line_segment(
                [
                    egui::pos2(rect.left() + inset, rect.top() + 1.0),
                    egui::pos2(rect.right() - inset, rect.top() + 1.0),
                ],
                Stroke::new(1.0, theme::edge_light().gamma_multiply(raised)),
            ));
        }
    } else {
        shapes.push(egui::Shape::Rect(RectShape::new(
            rect,
            radius,
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        )));
    }
    shapes
}

fn draw(ui: &mut Ui, spec: Spec<'_>) -> Response {
    {
        let (size, height, pad) = metrics(spec.height, ui);
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
    // Unavailable actions read as neutral and flat, not as dimmed colour.
    let available = spec.enabled || spec.busy;
    let palette = if available {
        spec.palette
    } else {
        unavailable()
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
        let (size, height, pad) = metrics(spec.height, ui);
        ui.spacing_mut().button_padding = egui::vec2(pad, 4.0);
        let text_color = if spec.busy {
            Color32::TRANSPARENT
        } else {
            palette.text
        };
        let mut button = egui::Button::new(RichText::new(spec.text).size(size).color(text_color))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
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
        let body_slot = ui.painter().add(egui::Shape::Noop);
        let response = ui.add(button);
        // Opaque palettes are raised caps; translucent ones lie flat.
        let raised = if available {
            (palette.base.a() as f32 / 255.0).powi(4) * (1.0 - 0.8 * press)
        } else {
            0.0
        };
        ui.painter().set(
            body_slot,
            button_shapes(
                ui.ctx(),
                response.rect,
                spec.radius,
                fill,
                palette.stroke,
                raised,
            ),
        );
        if let Some(color) = spec.halo.filter(|_| spec.enabled) {
            let strength = if theme::is_light() { 0.08 } else { 0.14 } + 0.14 * hover;
            ui.painter().set(
                halo_slot,
                RectShape::filled(
                    response.rect.translate(egui::vec2(0.0, 1.0)),
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

pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    draw(ui, Spec::new(text, palette(kind)))
}

pub fn button_enabled(ui: &mut Ui, enabled: bool, text: &str, kind: Kind) -> Response {
    draw(
        ui,
        Spec {
            enabled,
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
            ..Spec::new(text, palette(kind))
        },
    )
}

pub fn small_button(ui: &mut Ui, enabled: bool, text: &str) -> Response {
    draw(
        ui,
        Spec {
            enabled,
            height: Height::Compact,
            ..Spec::new(text, palette(Kind::Secondary))
        },
    )
}

/// Latching control (Mute/Solo). Eases from a raised cap to a lit,
/// pressed-in state in `tone`.
pub fn toggle(ui: &mut Ui, enabled: bool, on: bool, text: &str, tone: Tone) -> Response {
    toggle_sized(ui, enabled, on, text, tone, Height::Regular)
}

/// Compact latching control for dense console strips.
pub fn toggle_small(ui: &mut Ui, enabled: bool, on: bool, text: &str, tone: Tone) -> Response {
    toggle_sized(ui, enabled, on, text, tone, Height::Compact)
}

fn toggle_sized(
    ui: &mut Ui,
    enabled: bool,
    on: bool,
    text: &str,
    tone: Tone,
    height: Height,
) -> Response {
    let key = ui.next_auto_id().with("engaged");
    let t = ui.ctx().animate_bool_with_time(key, on, 0.16);
    let palette = palette(Kind::Secondary).mix(engaged(tone.color()), t);
    draw(
        ui,
        Spec {
            enabled,
            selected: Some(on),
            height,
            halo: on.then(|| tone.color()),
            ..Spec::new(text, palette)
        },
    )
}

/// Selected segment: a small raised cap inside the well.
fn segment_on() -> Palette {
    Palette {
        base: theme::hover(),
        hover: theme::hover(),
        press: theme::raised(),
        text: theme::text(),
        stroke: theme::border_strong(),
    }
}

fn segment_off() -> Palette {
    Palette {
        base: Color32::TRANSPARENT,
        hover: theme::text().gamma_multiply(0.05),
        press: theme::text().gamma_multiply(0.08),
        text: theme::text_3(),
        stroke: Color32::TRANSPARENT,
    }
}

/// Segmented single choice: segments sit in a recessed well and the chosen
/// one rises out of it. Returns the index picked this frame.
pub fn segments(
    ui: &mut Ui,
    enabled: bool,
    labels: &[&str],
    current: Option<usize>,
) -> Option<usize> {
    segments_clicked(ui, enabled, labels, current).filter(|i| current != Some(*i))
}

/// As `segments`, but also reports a click on the chosen segment (for
/// choices whose current value can still be applied).
pub fn segments_clicked(
    ui: &mut Ui,
    enabled: bool,
    labels: &[&str],
    current: Option<usize>,
) -> Option<usize> {
    let mut picked = None;
    let slot = ui.painter().add(egui::Shape::Noop);
    let shown = egui::Frame::new()
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, label) in labels.iter().enumerate() {
                    let active = current == Some(i);
                    let response = draw(
                        ui,
                        Spec {
                            enabled,
                            selected: Some(active),
                            height: Height::Inner,
                            radius: CornerRadius::same(7),
                            ..Spec::new(label, if active { segment_on() } else { segment_off() })
                        },
                    );
                    if response.clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
    let rect = shown.response.rect;
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .set(
            slot,
            egui::Shape::Vec(fx::well_shapes(
                rect,
                theme::CONTROL_RADIUS + 1,
                theme::well(),
            )),
        );
    picked
}

/// Joined single-choice buttons (gain presets). Returns the picked value
/// when it differs from `current`.
pub fn segmented(ui: &mut Ui, enabled: bool, items: &[(&str, f32)], current: f32) -> Option<f32> {
    let labels: Vec<&str> = items.iter().map(|(label, _)| *label).collect();
    let current = items
        .iter()
        .position(|(_, value)| (current - value).abs() < 0.25);
    segments(ui, enabled, &labels, current).map(|i| items[i].1)
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
            theme::surface(),
            glow,
        ),
    );
    egui::InnerResponse::new(shown.inner, shown.response)
}

/// Surface at an explicit elevation and base colour (console strips, the
/// master strip, hero cards).
pub fn surface_at<R>(
    ui: &mut Ui,
    level: Level,
    base: Color32,
    glow: Option<Color32>,
    margin: Margin,
    body: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = ui.painter().add(egui::Shape::Noop);
    let shown = egui::Frame::new().inner_margin(margin).show(ui, body);
    ui.painter().set(
        slot,
        fx::elevated(
            ui.ctx(),
            shown.response.rect,
            theme::RADIUS,
            base,
            level,
            glow,
        ),
    );
    egui::InnerResponse::new(shown.inner, shown.response)
}

/// Numeric readout set into a small recessed display.
pub fn readout_well(ui: &mut Ui, text: RichText) -> Response {
    let slot = ui.painter().add(egui::Shape::Noop);
    let shown = egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| ui.label(text));
    let rect = shown.response.rect;
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .set(
            slot,
            egui::Shape::Vec(fx::well_shapes(rect, 7, theme::well())),
        );
    shown.inner
}

/// Status cap at the top-left of a card: names the card's state or source.
pub fn lead_cap(ui: &Ui, rect: egui::Rect, color: Color32) {
    let cap = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 20.0, rect.top()),
        egui::vec2(36.0_f32.min(rect.width() - 40.0).max(8.0), 3.0),
    );
    fx::glow(ui.painter(), cap.center(), 14.0, color.gamma_multiply(0.35));
    ui.painter().rect_filled(
        cap,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 2,
            se: 2,
        },
        color,
    );
}

/// Shadow for cards drawn with a plain `egui::Frame`.
pub fn card_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: theme::shadow(0.4),
    }
}

/// Short lit cap on the top edge of a surface: the colour names what the
/// surface carries (a source type, a state).
pub fn top_cap(ui: &Ui, rect: egui::Rect, color: Color32, width: f32) {
    let width = width.min(rect.width() - 36.0).max(8.0);
    let cap = egui::Rect::from_min_size(
        egui::pos2(rect.center().x - width / 2.0, rect.top()),
        egui::vec2(width, 3.0),
    );
    fx::glow(
        ui.painter(),
        cap.center(),
        width * 0.35,
        color.gamma_multiply(0.3),
    );
    ui.painter().rect_filled(
        cap,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 2,
            se: 2,
        },
        color,
    );
}

/// Card with an optional subtitle under the title and a status cap on the
/// top edge (live surfaces such as mixer lanes). The cap colour eases.
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
                            .color(theme::text()),
                    )
                    .truncate(),
                );
                if let Some(subtitle) = subtitle {
                    ui.add(
                        egui::Label::new(
                            RichText::new(subtitle)
                                .size(theme::SMALL)
                                .color(theme::text_3()),
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
        lead_cap(ui, shown.response.rect, color);
    }
    shown.inner
}

/// Inset row inside a card (list items, lanes).
pub fn inset<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::inset())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS + 1))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui)
        })
        .inner
}

pub fn caption(ui: &mut Ui, text: &str) -> Response {
    ui.label(
        RichText::new(text)
            .size(theme::SMALL)
            .color(theme::text_2()),
    )
}

pub fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .size(theme::SMALL)
            .color(theme::text_3()),
    );
}

pub fn error_text(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(theme::danger()));
}

pub fn mono(ui: &mut Ui, text: impl Into<String>) {
    ui.add(
        egui::Label::new(
            RichText::new(text.into())
                .monospace()
                .size(theme::SMALL)
                .color(theme::text_3()),
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
            egui::Label::new(
                RichText::new(label)
                    .size(theme::SMALL)
                    .color(theme::text_2()),
            )
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
            egui::Label::new(
                RichText::new(label)
                    .size(theme::SMALL)
                    .color(theme::text_3()),
            )
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
                        .hint_text(RichText::new(hint).color(theme::text_3()))
                        .font(theme::heading(22.0))
                        .text_color(theme::text())
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
                theme::border_strong(),
                ui.ctx()
                    .animate_bool_with_time(id.with("hover"), response.hovered(), 0.12),
            ),
            theme::accent(),
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
    let dot = if tone == Tone::Neutral { 0.0 } else { 11.0 };
    let ink = if tone == Tone::Neutral {
        theme::text_2()
    } else {
        color
    };
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL),
        ink,
        (max_width - 20.0 - dot).max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let size = galley.size() + egui::vec2(20.0 + dot, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let tint = if tone == Tone::Neutral {
        theme::text().gamma_multiply(0.07)
    } else {
        color.gamma_multiply(if theme::is_light() { 0.11 } else { 0.13 })
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(255), tint);
    if dot > 0.0 {
        ui.painter()
            .circle_filled(egui::pos2(rect.left() + 12.0, rect.center().y), 3.0, color);
    }
    ui.painter().galley(
        egui::pos2(
            rect.left() + 10.0 + dot,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
}

/// Coloured dot followed by text, for compact state rows.
pub fn dot(ui: &mut Ui, text: &str, tone: Tone) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL),
        theme::text_2(),
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
        theme::text_2(),
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
                        .color(theme::text_2()),
                );
                ui.label(
                    RichText::new(value)
                        .monospace()
                        .size(theme::MONO)
                        .color(theme::text()),
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
        ui.label(RichText::new(label).size(11.5).color(theme::text_3()));
        ui.label(
            RichText::new(value)
                .monospace()
                .size(theme::MONO + 0.5)
                .color(tone.map_or(theme::text(), Tone::color)),
        );
    });
}

pub fn empty(ui: &mut Ui, title: &str, description: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(title)
                .font(theme::heading(theme::SECTION))
                .color(theme::text_2()),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(description)
                .size(theme::SMALL + 0.5)
                .color(theme::text_3()),
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
    let r = (bar.height() / 2.0).min(4.0) as u8;
    fx::well(painter, bar.expand(1.0), r + 1, theme::meter_track());
    painter.rect_filled(bar, CornerRadius::same(r), theme::meter_unlit());
    let zones = [
        (METER_FLOOR_DB, -18.0, theme::meter_low()),
        (-18.0, -6.0, theme::meter_mid()),
        (-6.0, 0.0, theme::meter_high()),
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
            Stroke::new(2.0, theme::text()),
        );
    }
    // Segment gaps give the LED-ladder reading without per-cell drawing.
    let mut x = bar.left() + 4.0;
    while x < bar.right() {
        painter.line_segment(
            [egui::pos2(x, bar.top()), egui::pos2(x, bar.bottom())],
            Stroke::new(1.0, theme::meter_track()),
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
    fx::well(painter, bar.expand(1.5), 4, theme::meter_track());
    painter.rect_filled(bar, CornerRadius::same(2), theme::meter_unlit());
    let zones = [
        (METER_FLOOR_DB, -18.0, theme::meter_low()),
        (-18.0, -6.0, theme::meter_mid()),
        (-6.0, 0.0, theme::meter_high()),
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
            Stroke::new(1.0, theme::text_3().gamma_multiply(0.35)),
            StrokeKind::Inside,
        );
    }
    if peak.is_some() && hold > METER_FLOOR_DB {
        let y = meter_y(bar, hold);
        painter.line_segment(
            [egui::pos2(bar.left(), y), egui::pos2(bar.right(), y)],
            Stroke::new(2.0, theme::text()),
        );
    }
    let mut y = bar.bottom() - 4.0;
    while y > bar.top() {
        painter.line_segment(
            [egui::pos2(bar.left(), y), egui::pos2(bar.right(), y)],
            Stroke::new(1.0, theme::meter_track()),
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
            Stroke::new(1.0, theme::text_3()),
        );
        painter.text(
            egui::pos2(bar.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            egui::FontId::proportional(9.5),
            theme::text_3(),
        );
    }
}

/// Limiter gain reduction, drawn down from the top (0…12 dB).
pub fn paint_reduction(painter: &egui::Painter, rect: egui::Rect, limiter_gain: Option<f64>) {
    fx::well(painter, rect.expand(1.0), 3, theme::meter_track());
    if let Some(g) = limiter_gain.filter(|g| *g > 0.0 && *g < 0.999) {
        let reduction = (-20.0 * g.log10()) as f32;
        let h = rect.height() * (reduction / 12.0).clamp(0.02, 1.0);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h)),
            CornerRadius::same(2),
            theme::warning(),
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
            Stroke::new(1.0, theme::text_3()),
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
                theme::text_3(),
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
                        .color(theme::text_3()),
                );
                ui.label(
                    RichText::new(db_text(p))
                        .monospace()
                        .size(theme::MONO)
                        .color(if hot {
                            theme::meter_high()
                        } else {
                            theme::text()
                        }),
                );
            }
            None => {
                ui.label(
                    RichText::new(text(ui, &Message::WidgetsLevelUnavailable))
                        .size(11.5)
                        .color(theme::text_3()),
                );
            }
        }
        if let Some(r) = rms {
            ui.label(RichText::new("RMS").size(11.5).color(theme::text_3()));
            ui.label(
                RichText::new(db_text(r))
                    .monospace()
                    .size(theme::MONO)
                    .color(theme::text_2()),
            );
        }
        if peak.is_some() || rms.is_some() {
            ui.label(RichText::new("dBFS").size(11.0).color(theme::text_3()));
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
    fx::well(painter, rail.expand(1.0), 4, theme::meter_track());
    let mut filled = rail;
    if vertical {
        filled.min.y = x;
    } else {
        filled.max.x = x;
    }
    let fill = if shown > 0.0 {
        theme::warning()
    } else {
        theme::accent()
    };
    let fill = if enabled { fill } else { theme::text_3() };
    painter.rect_filled(filled, CornerRadius::same(3), fill.gamma_multiply(0.85));
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
        Stroke::new(1.0, theme::text_3()),
    );
    if size != FaderSize::Row {
        for db in [-48.0, -24.0, -12.0, -6.0, 6.0] {
            tick(
                at(gain_to_t(db)),
                rail_h / 2.0 + 3.0,
                rail_h / 2.0 + 6.0,
                Stroke::new(1.0, theme::border_strong()),
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
            theme::text().gamma_multiply(bubble),
        );
        let size = galley.size() + egui::vec2(12.0, 6.0);
        let bubble_rect = if vertical {
            let left = handle.right() + 6.0 + 4.0 * (1.0 - bubble);
            egui::Rect::from_min_size(egui::pos2(left, x - size.y / 2.0), size)
        } else {
            let at = egui::pos2(x, handle.top() - 6.0 - 4.0 * (1.0 - bubble));
            egui::Rect::from_center_size(egui::pos2(at.x, at.y - size.y / 2.0), size)
        };
        for shape in fx::shadow(bubble_rect, 6, Level::Raised) {
            if let egui::Shape::Rect(mut s) = shape {
                s.fill = s.fill.gamma_multiply(bubble);
                top.add(s);
            }
        }
        top.rect(
            bubble_rect,
            CornerRadius::same(6),
            theme::overlay().gamma_multiply(bubble),
            Stroke::new(1.0, theme::border_strong().gamma_multiply(bubble)),
            StrokeKind::Inside,
        );
        top.galley(
            bubble_rect.center() - galley.size() / 2.0,
            galley,
            theme::text().gamma_multiply(bubble),
        );
    }
    Fader {
        response,
        committed,
    }
}

/// Fader cap: a light metal cap with a contact shadow, a dark groove that
/// carries the fill colour, and a halo while hovered or dragged. `vertical`
/// turns the groove sideways.
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
            handle.width().max(handle.height()) * 0.8,
            color.gamma_multiply(0.25 * hot),
        );
    }
    let r = CornerRadius::same(5);
    for shape in fx::shadow(handle, 5, Level::Raised) {
        painter.add(shape);
    }
    let (top, bottom) = if enabled {
        (theme::cap_top(), theme::cap_bottom())
    } else {
        (theme::raised(), theme::hover())
    };
    painter.rect_filled(handle, r, bottom);
    // Upper half catches the light.
    let upper = egui::Rect::from_min_max(
        handle.min,
        egui::pos2(handle.max.x, handle.center().y + 1.0),
    );
    painter.rect_filled(
        upper,
        CornerRadius {
            nw: 5,
            ne: 5,
            sw: 2,
            se: 2,
        },
        animation::lerp_color(top, bottom, 0.15),
    );
    painter.rect_stroke(
        handle,
        r,
        Stroke::new(1.0, theme::shadow(0.35)),
        StrokeKind::Inside,
    );
    painter.line_segment(
        [
            egui::pos2(handle.left() + 5.0, handle.top() + 1.0),
            egui::pos2(handle.right() - 5.0, handle.top() + 1.0),
        ],
        Stroke::new(1.0, Color32::WHITE.gamma_multiply(0.6)),
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
    let groove = if enabled {
        animation::lerp_color(color, Color32::BLACK, 0.25)
    } else {
        theme::text_3()
    };
    painter.line_segment(line, Stroke::new(2.0, groove));
}

/// Section label inside a page, with optional trailing content.
pub fn section(ui: &mut Ui, title: &str, trailing: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .font(theme::heading(theme::SECTION))
                .color(theme::text()),
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
            crate::icons::chevron(ui.painter(), chev, openness, theme::text_2());
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
                            .color(theme::text()),
                    )
                    .selectable(false),
                );
                if let Some(subtitle) = subtitle {
                    ui.add(
                        egui::Label::new(
                            RichText::new(subtitle)
                                .size(theme::SMALL)
                                .color(theme::text_3()),
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
                            theme::success().gamma_multiply(0.7)
                        } else {
                            theme::border_strong()
                        },
                    ),
                );
            }
            let galley = ui.painter().layout_no_wrap(
                label.to_string(),
                egui::FontId::proportional(theme::SMALL + 1.0),
                theme::text(),
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
                Step::Done => (theme::success(), theme::success(), theme::text_2()),
                Step::Current => (theme::accent(), theme::accent(), theme::text()),
                Step::Todo => (
                    Color32::TRANSPARENT,
                    theme::border_strong(),
                    theme::text_3(),
                ),
            };
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect.expand(2.0), CornerRadius::same(12), theme::hover());
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
                    theme::success(),
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

/// Filter chips with counts: the chosen one is a raised cap, the rest are
/// text until hovered. Returns the chip picked this frame.
pub fn chips(ui: &mut Ui, current: usize, items: &[(&str, usize)]) -> Option<usize> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (label, count)) in items.iter().enumerate() {
            let on = i == current;
            let text = format!("{label} {count}");
            let palette = if on {
                palette(Kind::Secondary)
            } else {
                palette(Kind::Ghost)
            };
            let response = draw(
                ui,
                Spec {
                    height: Height::Compact,
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
    let lift = hover.max(selected as u8 as f32);
    let fill = animation::lerp_color(theme::surface(), theme::raised(), lift);
    let back = ui.painter().add(egui::Shape::Noop);
    let frame = egui::Frame::new()
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(width - 30.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                dot(ui, title, tone);
                ui.label(RichText::new(value).font(theme::heading(20.0)).color(
                    if tone == Tone::Neutral || tone == Tone::Success {
                        theme::text()
                    } else {
                        tone.color()
                    },
                ));
                ui.add(
                    egui::Label::new(
                        RichText::new(detail)
                            .size(theme::SMALL)
                            .color(theme::text_3()),
                    )
                    .truncate(),
                );
            });
        });
    let rect = frame.response.rect;
    let mut shapes = Vec::new();
    if matches!(tone, Tone::Warning | Tone::Danger) {
        // Something needs attention: the tile glows in its state colour.
        shapes.push(egui::Shape::Rect(
            RectShape::filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                tone.color().gamma_multiply(0.16),
            )
            .with_blur_width(18.0),
        ));
    }
    let level = if lift > 0.5 {
        Level::Raised
    } else {
        Level::Card
    };
    shapes.extend(fx::elevated(
        ui.ctx(),
        rect,
        theme::RADIUS,
        fill,
        level,
        None,
    ));
    if selected {
        shapes.push(egui::Shape::Rect(RectShape::stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(1.5, tone.color().gamma_multiply(0.6)),
            StrokeKind::Inside,
        )));
    }
    ui.painter().set(back, egui::Shape::Vec(shapes));
    let response = ui.interact(rect, id, egui::Sense::click());
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
        theme::text_2(),
    );
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    paint_keycap(ui.painter(), rect);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, theme::text_2());
}

/// Quick-action entry that reads as a search field: a recessed well with a
/// magnifier, placeholder and shortcut cap. Opens the command palette.
/// `width` at or below 40 shows the magnifier only.
pub fn search_field(
    ui: &mut Ui,
    width: f32,
    placeholder: &str,
    shortcut: &str,
    label: &str,
) -> Response {
    let height = theme::CONTROL_HEIGHT + 2.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered(),
        animation::FAST,
    );
    let painter = ui.painter();
    fx::well(
        painter,
        rect,
        theme::CONTROL_RADIUS + 1,
        animation::lerp_color(theme::well(), theme::inset(), hover * 0.6),
    );
    let compact = width <= 40.0;
    let icon_at = if compact {
        rect.center()
    } else {
        egui::pos2(rect.left() + 18.0, rect.center().y)
    };
    crate::icons::paint(
        painter,
        crate::icons::square(icon_at, 14.0),
        crate::icons::Icon::Search,
        theme::text_3(),
    );
    if !compact {
        let cap = painter.layout_no_wrap(
            shortcut.to_owned(),
            egui::FontId::monospace(11.0),
            theme::text_3(),
        );
        let cap_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - 8.0 - cap.size().x - 10.0,
                rect.center().y - (cap.size().y + 4.0) / 2.0,
            ),
            cap.size() + egui::vec2(10.0, 4.0),
        );
        let text_max = cap_rect.left() - (rect.left() + 32.0) - 6.0;
        if text_max > 24.0 {
            let mut job = egui::text::LayoutJob::simple(
                placeholder.to_owned(),
                egui::FontId::proportional(theme::SMALL + 1.0),
                theme::text_3(),
                text_max,
            );
            job.wrap.max_rows = 1;
            let galley = painter.layout_job(job);
            painter.galley(
                egui::pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
                galley,
                theme::text_3(),
            );
        }
        paint_keycap(painter, cap_rect);
        painter.galley(cap_rect.center() - cap.size() / 2.0, cap, theme::text_3());
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint_focus(ui, &response);
    response
}

/// Square icon-only button. `label` is its accessible name and tooltip.
pub fn icon_button(
    ui: &mut Ui,
    icon: crate::icons::Icon,
    label: &str,
    kind: Kind,
    size: f32,
    busy: bool,
) -> Response {
    let palette = palette(kind);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size, size),
        if busy {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let (hover, press) = animation::interaction(ui.ctx(), response.id);
    let fill = animation::lerp_color(
        animation::lerp_color(palette.base, palette.hover, hover),
        palette.press,
        press,
    );
    let raised = (palette.base.a() as f32 / 255.0).powi(4) * (1.0 - 0.8 * press);
    let radius = CornerRadius::same(((size / 4.0).round() as u8).max(5));
    ui.painter().extend(button_shapes(
        ui.ctx(),
        rect,
        radius,
        fill,
        palette.stroke,
        raised,
    ));
    if busy {
        egui::Spinner::new()
            .size(size * 0.45)
            .color(palette.text)
            .paint_at(
                ui,
                egui::Rect::from_center_size(rect.center(), egui::vec2(size * 0.5, size * 0.5)),
            );
    } else {
        crate::icons::paint(
            ui.painter(),
            crate::icons::square(rect.center(), size * 0.5),
            icon,
            palette.text,
        );
    }
    paint_focus(ui, &response);
    response.on_hover_text(label)
}

/// Full-width row in a popup menu. `danger` colours a destructive item.
pub fn menu_item(
    ui: &mut Ui,
    icon: Option<crate::icons::Icon>,
    text: &str,
    checked: bool,
    danger: bool,
) -> Response {
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    response.widget_info(|| {
        if icon.is_none() {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, checked, text)
        } else {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text)
        }
    });
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered() || response.has_focus(),
        animation::FAST,
    );
    let ink = if danger { danger_text() } else { theme::text() };
    let painter = ui.painter();
    if hover > 0.0 {
        let tint = if danger {
            theme::danger().gamma_multiply(0.12)
        } else {
            theme::text().gamma_multiply(0.07)
        };
        painter.rect_filled(rect, CornerRadius::same(7), tint.gamma_multiply(hover));
    }
    if let Some(icon) = icon {
        crate::icons::paint(
            painter,
            crate::icons::square(egui::pos2(rect.left() + 16.0, rect.center().y), 14.0),
            icon,
            if danger { ink } else { theme::text_2() },
        );
    } else if checked {
        crate::icons::paint(
            painter,
            crate::icons::square(egui::pos2(rect.left() + 16.0, rect.center().y), 12.0),
            crate::icons::Icon::Check,
            theme::accent(),
        );
    }
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(theme::SMALL + 1.0),
        ink,
        (rect.width() - 40.0).max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    response
}

fn paint_keycap(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 1.0)),
        CornerRadius::same(5),
        theme::shadow(0.5),
    );
    painter.rect(
        rect,
        CornerRadius::same(5),
        theme::raised(),
        Stroke::new(1.0, theme::border()),
        StrokeKind::Inside,
    );
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
