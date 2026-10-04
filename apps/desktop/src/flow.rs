//! 信号汇流图: sources → room core → physical output, drawn from the
//! authoritative snapshot and live meters. Every visual maps to real state:
//! line style is the session/mix state, particle brightness is the lane RMS,
//! the core ring is the room output level. With no signal above the meter
//! floor (or reduced motion) nothing moves and no frames are requested.
use crate::emblem::{self, Emblem};
use crate::{animation, fx, icons, theme, widgets};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2,
};
use std::sync::Arc;

/// What a source's line says about its path into the room.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Edge {
    /// Playing and audible.
    Live,
    /// Playing on a degraded network.
    Degraded,
    Buffering,
    /// Muted at the room: the gate sits on the line before the core.
    Muted,
    /// Silent because another lane is soloed.
    SoloedOut,
    /// Interrupted, revoked, disconnected or stopped.
    Broken,
    /// No session state.
    Unknown,
}

impl Edge {
    fn flows(self) -> bool {
        matches!(self, Self::Live | Self::Degraded | Self::Muted)
    }
}

pub struct Source {
    pub key: u64,
    pub name: String,
    pub detail: String,
    pub detail_color: Color32,
    pub airplay: bool,
    pub mine: bool,
    pub solo: bool,
    pub edge: Edge,
    pub gain_db: f32,
    pub rms: Option<f64>,
    /// Hover card rows (label, value).
    pub facts: Vec<(&'static str, String)>,
    /// 0→1 while a source that just joined connects to the core; 1 otherwise.
    pub appear: f32,
}

impl Source {
    fn color(&self) -> Color32 {
        if self.airplay {
            theme::SRC_AIRPLAY
        } else {
            theme::SRC_NATIVE
        }
    }

    fn accessible(&self) -> String {
        format!(
            "{}，{}，{}，{} dB",
            self.name,
            if self.airplay {
                "AirPlay"
            } else {
                "原生 Sender"
            },
            self.detail,
            widgets::gain_text(self.gain_db)
        )
    }
}

pub struct Hub {
    pub name: String,
    pub id: Option<uuid::Uuid>,
    pub state: String,
    pub state_color: Color32,
    /// Room is playing (sharing or connected and output available).
    pub active: bool,
    pub rms: Option<f64>,
    pub limiter: Option<f64>,
}

pub struct Output {
    pub name: String,
    pub gain_db: f32,
    pub muted: bool,
    pub available: bool,
}

pub struct Model {
    pub sources: Vec<Source>,
    pub offline: usize,
    pub hub: Hub,
    pub output: Option<Output>,
    pub selected: Option<u64>,
}

pub enum Action {
    Select(u64),
    OpenMixer,
    OpenDevices,
}

#[derive(Debug)]
pub struct Geometry {
    pub horizontal: bool,
    pub sources: Vec<Rect>,
    pub offline: Option<Rect>,
    pub hub: Pos2,
    pub hub_r: f32,
    pub output: Pos2,
}

const CARD_H: f32 = 54.0;
const GAP: f32 = 12.0;
const OFFLINE_H: f32 = 26.0;
const OUTPUT_HALF: f32 = 24.0;
/// Compact source node for the stacked (narrow) graph.
const CHIP_H: f32 = 84.0;
/// Below this width the graph stacks top to bottom.
pub const HORIZONTAL_MIN: f32 = 540.0;

/// Canvas height that fits `n` sources at `width`.
pub fn height_for(n: usize, offline: bool, width: f32) -> f32 {
    let extra = if offline { GAP + OFFLINE_H } else { 0.0 };
    if width >= HORIZONTAL_MIN {
        let column = n as f32 * CARD_H + n.saturating_sub(1) as f32 * GAP + extra;
        (column + 48.0).max(280.0)
    } else {
        let row = if n > 0 { CHIP_H } else { 0.0 };
        row + extra + 64.0 + 2.0 * 44.0 + 72.0 + 2.0 * OUTPUT_HALF + 56.0
    }
}

/// Node placement. Pure so it can be checked for overlap at every size.
pub fn layout(n: usize, offline: bool, rect: Rect) -> Geometry {
    let horizontal = rect.width() >= HORIZONTAL_MIN;
    let mut sources = Vec::with_capacity(n);
    if horizontal {
        let w = (rect.width() * 0.3).clamp(160.0, 236.0);
        let extra = if offline { GAP + OFFLINE_H } else { 0.0 };
        let total = n as f32 * CARD_H + n.saturating_sub(1) as f32 * GAP + extra;
        let mut y = rect.center().y - total / 2.0;
        for _ in 0..n {
            sources.push(Rect::from_min_size(
                pos2(rect.left() + 4.0, y),
                vec2(w, CARD_H),
            ));
            y += CARD_H + GAP;
        }
        let offline =
            offline.then(|| Rect::from_min_size(pos2(rect.left() + 4.0, y), vec2(w, OFFLINE_H)));
        let hub_r = (rect.height() * 0.2).clamp(42.0, 60.0);
        let output = pos2(rect.right() - 70.0, rect.center().y - 10.0);
        let left = rect.left() + 4.0 + w;
        let hub = pos2(left + (output.x - left) * 0.5, rect.center().y - 10.0);
        Geometry {
            horizontal,
            sources,
            offline,
            hub,
            hub_r,
            output,
        }
    } else {
        // One row of compact chips so every line drops straight to the core
        // without crossing another node.
        let w = if n > 0 {
            ((rect.width() - n.saturating_sub(1) as f32 * GAP) / n as f32).min(150.0)
        } else {
            0.0
        };
        let row = n as f32 * w + n.saturating_sub(1) as f32 * GAP;
        let mut x = rect.center().x - row / 2.0;
        let mut y = rect.top() + 4.0;
        for _ in 0..n {
            sources.push(Rect::from_min_size(pos2(x, y), vec2(w, CHIP_H)));
            x += w + GAP;
        }
        if n > 0 {
            y += CHIP_H + GAP;
        }
        let offline = offline.then(|| {
            let r = Rect::from_center_size(
                pos2(rect.center().x, y + OFFLINE_H / 2.0),
                vec2(rect.width().min(260.0), OFFLINE_H),
            );
            y += OFFLINE_H + GAP;
            r
        });
        let hub_r = 44.0;
        let hub = pos2(rect.center().x, y + 52.0 + hub_r);
        let output = pos2(rect.center().x, hub.y + hub_r + 72.0 + OUTPUT_HALF);
        Geometry {
            horizontal,
            sources,
            offline,
            hub,
            hub_r,
            output,
        }
    }
}

/// Level 0…1 above the meter floor, through the shared meter ballistics.
fn level(ctx: &egui::Context, id: egui::Id, rms: Option<f64>) -> Option<f32> {
    let db = widgets::to_db(rms?).unwrap_or(widgets::METER_FLOOR_DB - 6.0);
    let (smoothed, _) = animation::meter(ctx, id, db);
    Some(((smoothed - widgets::METER_FLOOR_DB) / -widgets::METER_FLOOR_DB).clamp(0.0, 1.0))
}

fn text(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

pub fn show(ui: &mut egui::Ui, model: &Model) -> Option<Action> {
    let ctx = ui.ctx().clone();
    let width = ui.available_width();
    let height = height_for(model.sources.len(), model.offline > 0, width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let geo = layout(
        model.sources.len(),
        model.offline > 0,
        rect.shrink2(vec2(0.0, 8.0)),
    );
    let id = ui.id().with("flow");
    let mut action = None;

    // Interaction first so hover can shape the painting.
    let mut hovered = None;
    let mut responses = Vec::with_capacity(model.sources.len());
    for (source, card) in model.sources.iter().zip(&geo.sources) {
        let response = ui.interact(*card, id.with(source.key), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                model.selected == Some(source.key),
                source.accessible(),
            )
        });
        if response.hovered() {
            hovered = Some(source.key);
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            action = Some(Action::Select(source.key));
        }
        responses.push(response);
    }
    let hub_rect = Rect::from_center_size(geo.hub, vec2(geo.hub_r * 2.0, geo.hub_r * 2.0));
    let hub_response = ui.interact(hub_rect, id.with("hub"), Sense::click());
    hub_response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!(
                "房间「{}」，{}，打开 Mixer",
                model.hub.name, model.hub.state
            ),
        )
    });
    if hub_response.clicked() {
        action = Some(Action::OpenMixer);
    }
    let hub_response = hub_response.on_hover_text("打开 Mixer");
    let mut offline_label = None;
    if let Some(offline) = geo.offline {
        let response = ui.interact(offline, id.with("offline"), Sense::click());
        let label = format!("另有 {} 台已配对设备未在发送", model.offline);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.clicked() {
            action = Some(Action::OpenDevices);
        }
        offline_label = Some((offline, label, response.hovered()));
    }

    let painter = ui.painter_at(rect.expand(4.0));
    let levels: Vec<Option<f32>> = model
        .sources
        .iter()
        .map(|s| level(&ctx, id.with(("lvl", s.key)), s.rms))
        .collect();
    let master = level(&ctx, id.with("master"), model.hub.rms);
    let output_open = model
        .output
        .as_ref()
        .is_some_and(|o| o.available && !o.muted);
    let moving = model
        .sources
        .iter()
        .zip(&levels)
        .any(|(s, l)| s.edge.flows() && l.is_some_and(|l| l > 0.0))
        || (output_open && model.hub.active && master.is_some_and(|l| l > 0.0));
    let phase = animation::live_phase(&ctx, id.with("phase"), moving);

    // The graph sits in a recessed well, darker than the card around it.
    painter.rect_filled(
        rect,
        CornerRadius::same(12),
        theme::BG_DEEP.gamma_multiply(0.55),
    );
    // Ambient pool of light under the core.
    fx::glow(
        &painter,
        geo.hub,
        geo.hub_r * 1.6,
        theme::ACCENT.gamma_multiply(if model.hub.active {
            0.05 + 0.08 * master.unwrap_or(0.0)
        } else {
            0.02
        }),
    );

    // Lines into the core.
    let n = model.sources.len();
    for (i, ((source, card), lvl)) in model
        .sources
        .iter()
        .zip(&geo.sources)
        .zip(&levels)
        .enumerate()
    {
        let focus = hovered.is_none_or(|h| h == source.key);
        let spread = if n > 1 {
            (i as f32 - (n - 1) as f32 / 2.0) * (geo.hub_r * 0.9 / (n - 1) as f32).min(16.0)
        } else {
            0.0
        };
        let curve = if geo.horizontal {
            fx::s_curve(
                card.right_center(),
                geo.hub + vec2(-geo.hub_r - 8.0, spread),
            )
        } else {
            fx::s_curve_vertical(
                card.center_bottom(),
                geo.hub + vec2(spread, -geo.hub_r - 8.0),
            )
        };
        if source.appear < 1.0 {
            // Joining: the line grows from the source into the core.
            let t = animation::ease_out_cubic(source.appear);
            painter.add(egui::Shape::line(
                fx::cubic_points(curve, 0.0, t, 32),
                Stroke::new(2.0, source.color()),
            ));
            fx::glow(
                &painter,
                fx::cubic_at(curve, t),
                10.0,
                source.color().gamma_multiply(0.5),
            );
            continue;
        }
        paint_edge(
            &painter,
            curve,
            source.edge,
            source.color(),
            source.gain_db,
            *lvl,
            phase + i as f32 * 0.37,
            if focus { 1.0 } else { 0.3 },
        );
    }
    // Core → output.
    if let Some(output) = &model.output {
        let curve = if geo.horizontal {
            fx::s_curve(
                geo.hub + vec2(geo.hub_r + 8.0, 0.0),
                geo.output - vec2(OUTPUT_HALF + 6.0, 0.0),
            )
        } else {
            fx::s_curve_vertical(
                geo.hub + vec2(0.0, geo.hub_r + 8.0),
                geo.output - vec2(0.0, OUTPUT_HALF + 6.0),
            )
        };
        let edge = if !output.available {
            Edge::Broken
        } else if output.muted {
            Edge::Muted
        } else if model.hub.active {
            Edge::Live
        } else {
            Edge::Unknown
        };
        paint_edge(
            &painter,
            curve,
            edge,
            theme::SRC_HUB,
            output.gain_db,
            master,
            phase,
            if hovered.is_none() { 1.0 } else { 0.55 },
        );
    }

    for ((source, card), (lvl, response)) in model
        .sources
        .iter()
        .zip(&geo.sources)
        .zip(levels.iter().zip(&responses))
    {
        let dim = source.appear.min(1.0)
            * if hovered.is_some_and(|h| h != source.key) {
                0.45
            } else if source.edge == Edge::SoloedOut {
                0.6
            } else {
                1.0
            };
        paint_source(
            ui,
            &painter,
            *card,
            source,
            *lvl,
            model.selected == Some(source.key),
            response,
            dim,
            geo.horizontal,
        );
    }
    paint_hub(
        &ctx,
        &painter,
        &geo,
        &model.hub,
        master,
        hub_response.hovered(),
    );
    if let Some((rect, label, hovered)) = offline_label {
        paint_offline(&painter, rect, &label, hovered);
    }
    if let Some(output) = &model.output {
        paint_output(&painter, &geo, output);
    }
    for (source, response) in model.sources.iter().zip(responses) {
        if !source.facts.is_empty() {
            response.on_hover_ui(|ui| {
                ui.label(
                    egui::RichText::new(&source.name)
                        .font(theme::heading(theme::BODY))
                        .color(theme::TEXT),
                );
                widgets::kv_grid(ui, "flow-facts", &source.facts);
            });
        }
    }
    action
}

#[allow(clippy::too_many_arguments)]
fn paint_edge(
    painter: &egui::Painter,
    curve: [Pos2; 4],
    edge: Edge,
    color: Color32,
    gain_db: f32,
    level: Option<f32>,
    phase: f32,
    alpha: f32,
) {
    let width = 1.4 + 2.4 * widgets::gain_to_t(gain_db);
    let line = |t0: f32, t1: f32| fx::cubic_points(curve, t0, t1, 32);
    let (tint, dashed) = match edge {
        Edge::Live => (color, false),
        Edge::Degraded => (theme::WARNING, false),
        Edge::Buffering => (theme::ACCENT.gamma_multiply(0.6), true),
        Edge::Muted => (color.gamma_multiply(0.7), false),
        Edge::SoloedOut => (color.gamma_multiply(0.28), false),
        Edge::Broken => (theme::DANGER.gamma_multiply(0.75), false),
        Edge::Unknown => (theme::TEXT_3.gamma_multiply(0.5), true),
    };
    let tint = tint.gamma_multiply(alpha);
    let gate = 0.8;
    match edge {
        Edge::Broken => {
            painter.add(egui::Shape::line(line(0.0, 0.42), Stroke::new(width, tint)));
            painter.add(egui::Shape::line(line(0.58, 1.0), Stroke::new(width, tint)));
            let c = fx::cubic_at(curve, 0.5);
            let s = 4.5;
            let x = Stroke::new(2.0, theme::DANGER.gamma_multiply(alpha));
            painter.line_segment([c + vec2(-s, -s), c + vec2(s, s)], x);
            painter.line_segment([c + vec2(-s, s), c + vec2(s, -s)], x);
        }
        Edge::Muted => {
            painter.add(egui::Shape::line(line(0.0, gate), Stroke::new(width, tint)));
            fx::dashed(
                painter,
                &line(gate, 1.0),
                Stroke::new(1.2, theme::TEXT_3.gamma_multiply(0.5 * alpha)),
                4.0,
                4.0,
            );
            // The gate: a bar across the line with an M cap.
            let c = fx::cubic_at(curve, gate);
            let r = Rect::from_center_size(c, vec2(16.0, 16.0));
            painter.rect(
                r,
                CornerRadius::same(4),
                theme::SURFACE,
                Stroke::new(1.2, theme::WARNING.gamma_multiply(alpha)),
                StrokeKind::Inside,
            );
            painter.text(
                c,
                Align2::CENTER_CENTER,
                "M",
                FontId::monospace(10.0),
                theme::WARNING.gamma_multiply(alpha),
            );
        }
        _ if dashed => fx::dashed(painter, &line(0.0, 1.0), Stroke::new(1.4, tint), 5.0, 5.0),
        _ => {
            if edge == Edge::Live || edge == Edge::Degraded {
                // Soft underlay so a live path reads as lit.
                painter.add(egui::Shape::line(
                    line(0.0, 1.0),
                    Stroke::new(width + 6.0, tint.gamma_multiply(0.08)),
                ));
            }
            painter.add(egui::Shape::line(line(0.0, 1.0), Stroke::new(width, tint)));
        }
    }
    // Particles: only where sound actually moves, brightness from the level.
    let Some(level) = level.filter(|l| *l > 0.0 && edge.flows()) else {
        return;
    };
    let end = if edge == Edge::Muted {
        gate - 0.02
    } else {
        1.0
    };
    let count = 8;
    for i in 0..count {
        let t = (phase * 0.3 + i as f32 / count as f32).fract();
        if t > end {
            continue;
        }
        let envelope = (std::f32::consts::PI * (t / end)).sin();
        let p = fx::cubic_at(curve, t);
        let a = alpha * envelope * (0.35 + 0.65 * level);
        let core = if edge == Edge::Degraded {
            theme::WARNING
        } else {
            fx::lift(color, 0.35)
        };
        // Halo as a plain translucent disc: a blurred shape per particle
        // costs more tessellation than the rest of the graph.
        painter.circle_filled(p, 3.5 + 3.5 * level, color.gamma_multiply(0.12 * a));
        painter.circle_filled(p, 1.6 + 1.8 * level, core.gamma_multiply(a));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_source(
    ui: &egui::Ui,
    painter: &egui::Painter,
    card: Rect,
    source: &Source,
    level: Option<f32>,
    selected: bool,
    response: &egui::Response,
    dim: f32,
    wide: bool,
) {
    let ctx = ui.ctx();
    let hover = ctx.animate_bool_with_time(response.id.with("h"), response.hovered(), 0.12);
    let sel = ctx.animate_bool_with_time(response.id.with("s"), selected, 0.16);
    let color = source.color();
    if source.solo {
        fx::glow(
            painter,
            card.center(),
            card.height() * 0.9,
            theme::SOLO.gamma_multiply(0.10 * dim),
        );
    }
    let fill = animation::lerp_color(theme::SURFACE, theme::RAISED, hover.max(sel));
    let stroke = if source.solo {
        theme::SOLO.gamma_multiply(0.8)
    } else {
        animation::lerp_color(theme::BORDER_STRONG, theme::ACCENT, sel)
    };
    painter.rect(
        card,
        CornerRadius::same(12),
        fill.gamma_multiply(dim.max(0.7)),
        Stroke::new(1.0 + sel * 0.5, stroke.gamma_multiply(dim)),
        StrokeKind::Inside,
    );
    // Source-type cap: left edge on cards, top edge on chips.
    let cap = if wide {
        Rect::from_min_size(card.min + vec2(0.0, 12.0), vec2(3.0, card.height() - 24.0))
    } else {
        Rect::from_min_size(card.min + vec2(14.0, 0.0), vec2(card.width() - 28.0, 3.0))
    };
    painter.rect_filled(cap, CornerRadius::same(2), color.gamma_multiply(dim));
    // Avatar with a live level ring.
    let c = if wide {
        pos2(card.left() + 30.0, card.center().y)
    } else {
        pos2(card.center().x, card.top() + 27.0)
    };
    let r = 15.0;
    painter.circle_filled(c, r, color.gamma_multiply(0.16 * dim));
    if source.airplay {
        icons::paint(
            painter,
            icons::square(c, 16.0),
            icons::Icon::AirPlay,
            color.gamma_multiply(dim),
        );
    } else {
        let initial: String = source
            .name
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_else(|| "?".into());
        painter.text(
            c,
            Align2::CENTER_CENTER,
            initial,
            theme::heading(13.0),
            color.gamma_multiply(dim),
        );
    }
    match level {
        Some(l) => {
            painter.circle_stroke(c, r + 4.0, Stroke::new(2.5, theme::METER_TRACK));
            if l > 0.0 {
                let a0 = -std::f32::consts::FRAC_PI_2;
                let db = widgets::METER_FLOOR_DB * (1.0 - l);
                fx::arc(
                    painter,
                    c,
                    r + 4.0,
                    a0,
                    a0 + std::f32::consts::TAU * l,
                    Stroke::new(2.5, fx::zone(db).gamma_multiply(dim)),
                );
            }
        }
        None => {
            painter.circle_stroke(
                c,
                r + 4.0,
                Stroke::new(1.0, theme::TEXT_3.gamma_multiply(0.4)),
            );
        }
    }
    let text_left = card.left() + 54.0;
    let width = if wide {
        card.right() - text_left - 10.0
    } else {
        card.width() - 12.0
    };
    let name = text(
        painter,
        &source.name,
        theme::heading(13.5),
        theme::TEXT.gamma_multiply(dim),
        width - if source.mine { 22.0 } else { 0.0 },
    );
    let detail = text(
        painter,
        &source.detail,
        FontId::proportional(theme::SMALL),
        source.detail_color.gamma_multiply(dim),
        width,
    );
    let name_w = name.size().x;
    let (name_at, detail_at) = if wide {
        (
            pos2(text_left, card.center().y - 17.0),
            pos2(text_left, card.center().y + 2.0),
        )
    } else {
        let badge = if source.mine { 23.0 } else { 0.0 };
        (
            pos2(card.center().x - (name_w + badge) / 2.0, card.top() + 48.0),
            pos2(card.center().x - detail.size().x / 2.0, card.top() + 65.0),
        )
    };
    painter.galley(name_at, name, theme::TEXT);
    painter.galley(detail_at, detail, theme::TEXT_2);
    if source.mine {
        let badge = Rect::from_min_size(
            pos2(name_at.x + name_w + 5.0, name_at.y + 2.0),
            vec2(18.0, 15.0),
        );
        painter.rect_filled(
            badge,
            CornerRadius::same(4),
            theme::ACCENT.gamma_multiply(0.2),
        );
        painter.text(
            badge.center(),
            Align2::CENTER_CENTER,
            "你",
            FontId::proportional(10.0),
            theme::ACCENT,
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            card.expand(2.0),
            CornerRadius::same(14),
            Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.85)),
            StrokeKind::Outside,
        );
    }
}

fn paint_hub(
    ctx: &egui::Context,
    painter: &egui::Painter,
    geo: &Geometry,
    hub: &Hub,
    master: Option<f32>,
    hovered: bool,
) {
    let c = geo.hub;
    let r = geo.hub_r;
    let lit = if hub.active { 1.0 } else { 0.0 };
    let lit = ctx.animate_value_with_time(egui::Id::new("flow-hub-lit"), lit, 0.3);
    painter.circle_filled(c, r, theme::SURFACE);
    let segments = 28;
    let on = master.unwrap_or(0.0) * segments as f32;
    fx::led_ring(painter, c, r - 4.0, 5.0, segments, on * lit, |i| {
        fx::zone(widgets::METER_FLOOR_DB * (1.0 - (i as f32 + 1.0) / segments as f32))
    });
    painter.circle_stroke(
        c,
        r,
        Stroke::new(
            1.0 + hovered as u8 as f32,
            animation::lerp_color(
                theme::BORDER_STRONG,
                theme::ACCENT,
                0.3 * lit + 0.5 * hovered as u8 as f32,
            ),
        ),
    );
    match hub.id {
        Some(id) => emblem::paint(painter, c, r - 14.0, Emblem::from_id(id), lit),
        None => {
            painter.circle_stroke(c, r - 14.0, Stroke::new(1.0, theme::BORDER_STRONG));
            icons::paint(
                painter,
                icons::square(c, r * 0.6),
                icons::Icon::Room,
                theme::TEXT_3,
            );
        }
    }
    if let Some(g) = hub.limiter.filter(|g| *g < 0.999 && *g > 0.0) {
        let reduction = -20.0 * g.log10() as f32;
        let span = (reduction / 6.0).clamp(0.08, 1.0) * std::f32::consts::FRAC_PI_2;
        let top = -std::f32::consts::FRAC_PI_2;
        fx::arc(
            painter,
            c,
            r + 6.0,
            top - span,
            top + span,
            Stroke::new(3.0, theme::WARNING),
        );
    }
    let below = pos2(c.x, c.y + r + 10.0);
    let name = text(
        painter,
        &hub.name,
        theme::heading(14.0),
        theme::TEXT,
        if geo.horizontal { 170.0 } else { 130.0 },
    );
    let state = text(
        painter,
        &hub.state,
        FontId::proportional(theme::SMALL),
        hub.state_color,
        if geo.horizontal { 280.0 } else { 130.0 },
    );
    let (nw, sw) = (name.size().x, state.size().x);
    if geo.horizontal {
        painter.galley(pos2(c.x - nw / 2.0, below.y), name, theme::TEXT);
        painter.galley(pos2(c.x - sw / 2.0, below.y + 20.0), state, theme::TEXT_2);
    } else {
        // Stacked: labels beside the core keep the line to the output clear.
        let x = c.x + r + 14.0;
        painter.galley(pos2(x, c.y - 19.0), name, theme::TEXT);
        painter.galley(pos2(x, c.y + 2.0), state, theme::TEXT_2);
    }
}

fn paint_output(painter: &egui::Painter, geo: &Geometry, output: &Output) {
    let c = geo.output;
    let rect = Rect::from_center_size(c, vec2(OUTPUT_HALF * 2.0, OUTPUT_HALF * 2.0));
    let tone = if !output.available {
        theme::WARNING
    } else {
        theme::SRC_OUTPUT
    };
    if output.available && !output.muted {
        fx::glow(
            painter,
            c,
            OUTPUT_HALF * 1.3,
            theme::SRC_OUTPUT.gamma_multiply(0.05),
        );
    }
    painter.rect(
        rect,
        CornerRadius::same(12),
        theme::RAISED,
        Stroke::new(1.0, tone.gamma_multiply(0.5)),
        StrokeKind::Inside,
    );
    icons::paint(painter, icons::square(c, 24.0), icons::Icon::Room, tone);
    let status = if !output.available {
        "输出丢失".to_owned()
    } else if output.muted {
        "总静音".to_owned()
    } else {
        format!("{} dB", widgets::gain_text(output.gain_db))
    };
    let status_color = if !output.available || output.muted {
        theme::WARNING
    } else {
        theme::TEXT_2
    };
    let name = text(
        painter,
        &output.name,
        theme::heading(13.0),
        theme::TEXT,
        128.0,
    );
    let status = text(
        painter,
        &status,
        FontId::monospace(theme::MONO),
        status_color,
        128.0,
    );
    if geo.horizontal {
        let y = rect.bottom() + 10.0;
        painter.galley(pos2(c.x - name.size().x / 2.0, y), name, theme::TEXT);
        painter.galley(
            pos2(c.x - status.size().x / 2.0, y + 20.0),
            status,
            theme::TEXT_2,
        );
    } else {
        let x = rect.right() + 14.0;
        painter.galley(pos2(x, c.y - 19.0), name, theme::TEXT);
        painter.galley(pos2(x, c.y + 2.0), status, theme::TEXT_2);
    }
}

fn paint_offline(painter: &egui::Painter, rect: Rect, label: &str, hovered: bool) {
    painter.rect(
        rect,
        CornerRadius::same(255),
        if hovered {
            theme::HOVER
        } else {
            theme::SURFACE
        },
        Stroke::new(1.0, theme::BORDER_STRONG),
        StrokeKind::Inside,
    );
    let galley = text(
        painter,
        label,
        FontId::proportional(theme::SMALL),
        theme::TEXT_3,
        rect.width() - 16.0,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, theme::TEXT_3);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.intersects(b) && a.intersect(b).area() > 0.5
    }

    #[test]
    fn nodes_never_overlap_or_leave_the_canvas() {
        for width in [380.0, 460.0, 540.0, 700.0, 900.0, 1300.0] {
            for n in 0..=4 {
                for offline in [false, true] {
                    let h = height_for(n, offline, width);
                    let canvas = Rect::from_min_size(Pos2::ZERO, vec2(width, h));
                    let g = layout(n, offline, canvas.shrink2(vec2(0.0, 8.0)));
                    let hub = Rect::from_center_size(g.hub, vec2(g.hub_r * 2.0, g.hub_r * 2.0));
                    let out = Rect::from_center_size(
                        g.output,
                        vec2(OUTPUT_HALF * 2.0, OUTPUT_HALF * 2.0),
                    );
                    let mut nodes: Vec<Rect> = g.sources.clone();
                    nodes.extend(g.offline);
                    nodes.push(hub.expand(8.0));
                    nodes.push(out);
                    for (i, a) in nodes.iter().enumerate() {
                        assert!(
                            canvas.contains_rect(*a),
                            "{width}/{n}/{offline}: node {i} {a:?} outside {canvas:?}"
                        );
                        for b in &nodes[i + 1..] {
                            assert!(
                                !overlaps(*a, *b),
                                "{width}/{n}/{offline}: {a:?} overlaps {b:?}"
                            );
                        }
                    }
                    assert_eq!(g.horizontal, width >= HORIZONTAL_MIN);
                }
            }
        }
    }
}
