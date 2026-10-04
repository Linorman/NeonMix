//! 关于: product version, the commit it was built from and where the source
//! lives. Everything here is compile-time data, so the page never waits.
use super::*;
use crate::widgets::Kind;
use egui::{Align, Layout, Margin};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

impl Desktop {
    pub(crate) fn about_page(&mut self, ui: &mut egui::Ui) {
        let version = env!("CARGO_PKG_VERSION");
        let commit = env!("NEONMIX_GIT_HASH");
        widgets::surface(ui, None, Margin::symmetric(20, 18), |ui| {
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
            "显示",
            Some("仅影响本机窗口"),
            None,
            |_| {},
            |ui| {
                let reduce = animation::reduce_motion();
                ui.horizontal(|ui| {
                    if widgets::toggle(
                        ui,
                        true,
                        reduce,
                        if reduce {
                            "已减少动态效果"
                        } else {
                            "减少动态效果"
                        },
                        widgets::Tone::Accent,
                    )
                    .clicked()
                    {
                        self.set_reduce_motion(!reduce);
                    }
                });
                widgets::note(
                    ui,
                    "开启后，信号流动光点与雷达扫描改为静态显示，一次性动画直接显示结果；电平与状态照常更新。也可用环境变量 NEONMIX_REDUCE_MOTION=1 开启。",
                );
            },
        );
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
