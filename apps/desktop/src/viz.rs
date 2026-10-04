//! State drawn as pictures: the Sender pipeline, the discovery radar, the
//! share switch, the invitation countdown and the shared input capacity.
//! Each one maps to real state; motion only while that state is changing.
use crate::{animation, fx, icons, theme};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Stage {
    Done,
    /// The next thing to do; carries the action label.
    Next,
    Todo,
    Fault,
}

pub struct Step<'a> {
    pub title: &'a str,
    pub detail: String,
    pub state: Stage,
    pub icon: icons::Icon,
}

impl Stage {
    fn color(self) -> Color32 {
        match self {
            Self::Done => theme::SUCCESS,
            Self::Next => theme::ACCENT,
            Self::Todo => theme::TEXT_3,
            Self::Fault => theme::WARNING,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Done => "已就绪",
            Self::Next => "下一步",
            Self::Todo => "未开始",
            Self::Fault => "需处理",
        }
    }
}

/// Stages joined left to right (top to bottom when narrow). A link between
/// two ready stages carries particles while `level` (0…1, this Sender's own
/// lane in the room) is above the floor. Returns the stage clicked.
pub fn pipeline(ui: &mut Ui, steps: &[Step<'_>], level: Option<f32>) -> Option<usize> {
    let id = ui.id().with("pipeline");
    let width = ui.available_width();
    let horizontal = width >= 640.0;
    let n = steps.len() as f32;
    let gap = if horizontal { 26.0 } else { 18.0 };
    let (node, total) = if horizontal {
        let w = (width - gap * (n - 1.0)) / n;
        (vec2(w, 84.0), vec2(width, 84.0))
    } else {
        (vec2(width, 52.0), vec2(width, n * 52.0 + (n - 1.0) * gap))
    };
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    let at = |i: usize| {
        let o = if horizontal {
            vec2(i as f32 * (node.x + gap), 0.0)
        } else {
            vec2(0.0, i as f32 * (node.y + gap))
        };
        Rect::from_min_size(rect.min + o, node)
    };
    let flowing = level.is_some_and(|l| l > 0.0);
    let phase = animation::live_phase(ui.ctx(), id.with("phase"), flowing);
    let mut picked = None;
    for i in 0..steps.len() {
        let r = at(i);
        if i + 1 < steps.len() {
            let next = at(i + 1);
            let (a, b) = if horizontal {
                (r.right_center(), next.left_center())
            } else {
                (r.center_bottom(), next.center_top())
            };
            let ready = steps[i].state == Stage::Done && steps[i + 1].state == Stage::Done;
            let painter = ui.painter();
            if ready {
                painter.line_segment(
                    [a, b],
                    Stroke::new(2.0, theme::SRC_NATIVE.gamma_multiply(0.7)),
                );
                if let Some(l) = level.filter(|l| *l > 0.0) {
                    for k in 0..3 {
                        let t = (phase * 0.8 + k as f32 / 3.0 + i as f32 * 0.21).fract();
                        let p = a + (b - a) * t;
                        let alpha = (std::f32::consts::PI * t).sin() * (0.4 + 0.6 * l);
                        painter.circle_filled(
                            p,
                            2.4,
                            fx::lift(theme::SRC_NATIVE, 0.4).gamma_multiply(alpha),
                        );
                    }
                }
            } else {
                fx::dashed(
                    painter,
                    &[a, b],
                    Stroke::new(1.2, theme::BORDER_STRONG),
                    4.0,
                    4.0,
                );
            }
            // Arrow head.
            let dir = (b - a).normalized();
            let side = vec2(-dir.y, dir.x);
            let tip = b - dir * 2.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    tip - dir * 6.0 + side * 3.5,
                    tip - dir * 6.0 - side * 3.5,
                ],
                if ready {
                    theme::SRC_NATIVE
                } else {
                    theme::BORDER_STRONG
                },
                Stroke::NONE,
            ));
        }
        let step = &steps[i];
        let response = ui.interact(r, id.with(i), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{}：{}（{}）", step.title, step.detail, step.state.word()),
            )
        });
        if response.clicked() {
            picked = Some(i);
        }
        let hover =
            ui.ctx()
                .animate_bool_with_time(response.id.with("h"), response.hovered(), 0.12);
        let color = step.state.color();
        let painter = ui.painter();
        let solid = step.state != Stage::Todo;
        if step.state == Stage::Next {
            fx::glow(
                painter,
                r.center(),
                r.height() * 0.8,
                theme::ACCENT.gamma_multiply(0.10),
            );
        }
        painter.rect(
            r,
            CornerRadius::same(12),
            animation::lerp_color(theme::SURFACE, theme::RAISED, hover),
            Stroke::new(
                1.0,
                if solid {
                    color.gamma_multiply(0.55)
                } else {
                    theme::BORDER_STRONG
                },
            ),
            StrokeKind::Inside,
        );
        if !solid {
            // Not reached yet: dashed outline reads as "to come".
            let pts = vec![
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            fx::dashed(
                painter,
                &pts,
                Stroke::new(1.0, theme::BORDER_STRONG),
                3.0,
                3.0,
            );
        }
        let icon_c = if horizontal {
            pos2(r.left() + 22.0, r.top() + 22.0)
        } else {
            pos2(r.left() + 24.0, r.center().y)
        };
        painter.circle_filled(icon_c, 13.0, color.gamma_multiply(0.14));
        icons::paint(painter, icons::square(icon_c, 15.0), step.icon, color);
        let text_x = icon_c.x + 20.0;
        let max = r.right() - text_x - 8.0;
        let title = label(painter, step.title, theme::heading(13.0), theme::TEXT, max);
        let detail = label(
            painter,
            &step.detail,
            FontId::proportional(theme::SMALL),
            theme::TEXT_2,
            if horizontal { r.width() - 20.0 } else { max },
        );
        let state = label(
            painter,
            step.state.word(),
            FontId::proportional(11.0),
            color,
            80.0,
        );
        if horizontal {
            painter.galley(
                pos2(text_x, icon_c.y - title.size().y / 2.0),
                title,
                theme::TEXT,
            );
            painter.galley(pos2(r.left() + 10.0, r.top() + 42.0), detail, theme::TEXT_2);
            painter.circle_filled(pos2(r.left() + 13.0, r.top() + 69.0), 3.0, color);
            painter.galley(
                pos2(r.left() + 20.0, r.top() + 69.0 - state.size().y / 2.0),
                state,
                color,
            );
        } else {
            painter.galley(pos2(text_x, r.top() + 8.0), title, theme::TEXT);
            painter.galley(pos2(text_x, r.top() + 28.0), detail, theme::TEXT_2);
            painter.galley(
                pos2(r.right() - state.size().x - 10.0, r.top() + 8.0),
                state,
                color,
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                r.expand(2.0),
                CornerRadius::same(14),
                Stroke::new(2.0, theme::ACCENT),
                StrokeKind::Outside,
            );
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    picked
}

fn label(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    painter.layout_job(job)
}

/// Discovery radar. Sweeps only while a discovery request is in flight;
/// found rooms sit as blips placed by a stable hash of their Hub id.
pub fn radar(ui: &mut Ui, scanning: bool, found: &[u64], size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let id = response.id.with("sweep");
    let phase = animation::live_phase(ui.ctx(), id, scanning);
    let c = rect.center();
    let r = size / 2.0 - 2.0;
    let painter = ui.painter();
    painter.circle_filled(c, r, theme::METER_TRACK);
    for k in [1.0, 0.66, 0.33] {
        painter.circle_stroke(
            c,
            r * k,
            Stroke::new(1.0, theme::BORDER_STRONG.gamma_multiply(0.8)),
        );
    }
    painter.line_segment(
        [c - vec2(r, 0.0), c + vec2(r, 0.0)],
        Stroke::new(1.0, theme::BORDER),
    );
    painter.line_segment(
        [c - vec2(0.0, r), c + vec2(0.0, r)],
        Stroke::new(1.0, theme::BORDER),
    );
    if scanning {
        let a = phase * 2.4;
        for k in 0..14 {
            let back = a - k as f32 * 0.05;
            painter.line_segment(
                [c, c + egui::Vec2::angled(back) * r],
                Stroke::new(
                    2.0,
                    theme::ACCENT.gamma_multiply(0.5 * (1.0 - k as f32 / 14.0)),
                ),
            );
        }
    }
    for hash in found {
        let angle = (*hash % 360) as f32 / 360.0 * std::f32::consts::TAU;
        let dist = 0.35 + 0.5 * ((*hash / 360) % 100) as f32 / 100.0;
        let p = c + egui::Vec2::angled(angle) * r * dist;
        fx::glow(painter, p, 8.0, theme::SUCCESS.gamma_multiply(0.4));
        painter.circle_filled(p, 3.5, theme::SUCCESS);
    }
    painter.circle_filled(c, 3.0, theme::ACCENT);
    response
}

/// Large on/off switch for room sharing. `busy` shows the round trip in the
/// knob; the caller decides what a click does.
pub fn switch(ui: &mut Ui, enabled: bool, on: bool, busy: bool, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(56.0, 30.0), Sense::click());
    let response = if enabled && !busy {
        response
    } else {
        response.on_hover_cursor(egui::CursorIcon::Default)
    };
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, label));
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("on"), on, 0.18);
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("h"), response.hovered() && enabled, 0.12);
    let track = animation::lerp_color(theme::RAISED, theme::SUCCESS.gamma_multiply(0.85), t);
    let painter = ui.painter();
    if t > 0.0 {
        fx::glow(
            painter,
            rect.center(),
            30.0,
            theme::SUCCESS.gamma_multiply(0.18 * t),
        );
    }
    painter.rect(
        rect,
        CornerRadius::same(255),
        if enabled { track } else { theme::SURFACE },
        Stroke::new(
            1.0,
            animation::lerp_color(theme::BORDER_STRONG, theme::SUCCESS, t * 0.6 + hover * 0.3),
        ),
        StrokeKind::Inside,
    );
    let x = rect.left() + 15.0 + (rect.width() - 30.0) * t;
    let knob = pos2(x, rect.center().y);
    painter.circle_filled(
        knob,
        11.0,
        if enabled { theme::TEXT } else { theme::TEXT_3 },
    );
    if busy {
        let a = animation::now(ui.ctx()) as f32 * 6.0;
        fx::arc(painter, knob, 6.0, a, a + 4.2, Stroke::new(2.0, theme::BG));
        ui.ctx().request_repaint();
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(255),
            Stroke::new(2.0, theme::ACCENT),
            StrokeKind::Outside,
        );
    }
    response
}

/// One-shot ripple: rings expanding from `center` while `t` runs 0→1.
pub fn ripple(painter: &egui::Painter, center: egui::Pos2, from: f32, t: f32, color: Color32) {
    for k in 0..3 {
        let local = (t * 1.4 - k as f32 * 0.2).clamp(0.0, 1.0);
        if local <= 0.0 || local >= 1.0 {
            continue;
        }
        let r = from + 60.0 * animation::ease_out_cubic(local);
        painter.circle_stroke(
            center,
            r,
            Stroke::new(2.0, color.gamma_multiply(1.0 - local)),
        );
    }
}

/// Invitation countdown: ring depletes with the real remaining time.
pub fn countdown_ring(ui: &mut Ui, remaining: f32, total: f32, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let c = rect.center();
    let r = size / 2.0 - 3.0;
    let frac = (remaining / total).clamp(0.0, 1.0);
    let color = if frac > 0.25 {
        theme::ACCENT
    } else {
        theme::WARNING
    };
    let painter = ui.painter();
    painter.circle_stroke(c, r, Stroke::new(4.0, theme::METER_TRACK));
    if frac > 0.0 {
        let top = -std::f32::consts::FRAC_PI_2;
        fx::arc(
            painter,
            c,
            r,
            top,
            top + std::f32::consts::TAU * frac,
            Stroke::new(4.0, color),
        );
        // Ten frames a second is enough for a ring that moves 3° a second.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    painter.text(
        c,
        Align2::CENTER_CENTER,
        format!("{}", remaining.ceil().max(0.0) as u32),
        FontId::monospace(size * 0.26),
        if frac > 0.0 {
            theme::TEXT
        } else {
            theme::TEXT_3
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            format!("邀请剩余 {} 秒", remaining.ceil().max(0.0) as u32),
        )
    });
    response
}

pub struct Slot {
    pub name: String,
    pub airplay: bool,
}

/// Shared room input capacity: one cell per lane, coloured by who holds it.
pub fn capacity_slots(ui: &mut Ui, limit: usize, used: &[Slot]) -> Response {
    let width = ui.available_width().min(420.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let gap = 6.0;
    let n = limit.max(1) as f32;
    let w = (width - gap * (n - 1.0)) / n;
    let painter = ui.painter();
    for i in 0..limit {
        let r = Rect::from_min_size(
            rect.min + vec2(i as f32 * (w + gap), 0.0),
            vec2(w, rect.height()),
        );
        match used.get(i) {
            Some(slot) => {
                let color = if slot.airplay {
                    theme::SRC_AIRPLAY
                } else {
                    theme::SRC_NATIVE
                };
                painter.rect(
                    r,
                    CornerRadius::same(8),
                    color.gamma_multiply(0.16),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                    StrokeKind::Inside,
                );
                painter.rect_filled(
                    Rect::from_min_size(r.min + vec2(8.0, r.height() / 2.0 - 3.0), vec2(6.0, 6.0)),
                    CornerRadius::same(3),
                    color,
                );
                let g = label(
                    painter,
                    &slot.name,
                    FontId::proportional(theme::SMALL),
                    theme::TEXT,
                    w - 24.0,
                );
                painter.galley(
                    pos2(r.left() + 18.0, r.center().y - g.size().y / 2.0),
                    g,
                    theme::TEXT,
                );
            }
            None => {
                let pts = vec![
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                fx::dashed(
                    painter,
                    &pts,
                    Stroke::new(1.0, theme::BORDER_STRONG),
                    3.0,
                    3.0,
                );
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "空闲",
                    FontId::proportional(theme::SMALL),
                    theme::TEXT_3,
                );
            }
        }
    }
    let summary = used
        .iter()
        .map(|s| {
            format!(
                "{}（{}）",
                s.name,
                if s.airplay { "AirPlay" } else { "原生" }
            )
        })
        .collect::<Vec<_>>()
        .join("、");
    let text = format!(
        "房间输入容量 {} / {limit}{}{summary}",
        used.len(),
        if used.is_empty() { "" } else { "：" }
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
    response.on_hover_text(text)
}
