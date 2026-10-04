//! 关于: product version, the commit it was built from and where the source
//! lives. Everything here is compile-time data, so the page never waits.
use super::*;
use crate::widgets::Kind;
use egui::{Align, CornerRadius, Layout, Margin};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

impl Desktop {
    pub(crate) fn about_page(&mut self, ui: &mut egui::Ui) {
        let version = env!("CARGO_PKG_VERSION");
        let commit = env!("NEONMIX_GIT_HASH");
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(20, 18))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(
                    RichText::new("NeonMix")
                        .font(theme::heading(26.0))
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new("局域网多设备音频混音")
                        .size(theme::BODY)
                        .color(theme::TEXT_2),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    widgets::pill(ui, &format!("版本 {version}"), widgets::Tone::Accent);
                    widgets::pill(ui, commit, widgets::Tone::Neutral);
                });
            });
        widgets::card_ex(
            ui,
            "构建信息",
            None,
            None,
            |_| {},
            |ui| {
                widgets::kv_grid(
                    ui,
                    "about-build",
                    &[
                        ("版本", version.to_owned()),
                        ("提交", commit.to_owned()),
                        (
                            "平台",
                            format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
                        ),
                        ("许可", env!("CARGO_PKG_LICENSE").to_owned()),
                    ],
                );
                ui.horizontal_wrapped(|ui| {
                    let copied = self
                        .copied_at
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                    if copied {
                        ui.ctx().request_repaint_after(Duration::from_millis(200));
                    }
                    let label = if copied {
                        "已复制 ✓"
                    } else {
                        "复制版本信息"
                    };
                    if widgets::small_button(ui, true, label).clicked() {
                        ui.ctx()
                            .copy_text(format!("NeonMix {version} ({commit}) {REPOSITORY}"));
                        self.copied_at = Some(Instant::now());
                    }
                });
            },
        );
        widgets::card_ex(
            ui,
            "源码仓库",
            None,
            None,
            |_| {},
            |ui| {
                widgets::mono(ui, REPOSITORY);
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    if widgets::button(ui, "在浏览器中打开", Kind::Secondary).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(REPOSITORY));
                    }
                });
            },
        );
    }
}
