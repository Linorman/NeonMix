//! 关于: product version, the commit it was built from and where the source
//! lives. Everything here is compile-time data, so the page never waits.
use super::*;
use crate::widgets::Kind;
use egui::{Align, Layout, Margin};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

impl Desktop {
    pub(crate) fn about_page(&mut self, ui: &mut egui::Ui) {
        let text_about_audio_mixing_across_devices_on_your_local_network =
            self.tr(&Message::AboutAudioMixingAcrossDevicesOnYourLocalNetwork);
        let text_about_display = self.tr(&Message::AboutDisplay);
        let text_about_only_affects_this_window = self.tr(&Message::AboutOnlyAffectsThisWindow);
        let text_about_reduced_motion_is_on = self.tr(&Message::AboutReducedMotionIsOn);
        let text_about_reduce_motion = self.tr(&Message::AboutReduceMotion);
        let text_about_signal_particles_and_radar_scans_become_static_and =
            self.tr(&Message::AboutSignalParticlesAndRadarScansBecomeStaticAnd);
        let text_about_build_information = self.tr(&Message::AboutBuildInformation);
        let text_about_version = self.tr(&Message::AboutVersion);
        let text_about_commit = self.tr(&Message::AboutCommit);
        let text_about_platform = self.tr(&Message::AboutPlatform);
        let text_about_license = self.tr(&Message::AboutLicense);
        let text_about_copied = self.tr(&Message::AboutCopied);
        let text_about_copy_version_information = self.tr(&Message::AboutCopyVersionInformation);
        let text_about_source_repository = self.tr(&Message::AboutSourceRepository);
        let text_about_open_in_browser = self.tr(&Message::AboutOpenInBrowser);
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
                RichText::new(
                    text_about_audio_mixing_across_devices_on_your_local_network.as_str(),
                )
                .size(theme::BODY)
                .color(theme::TEXT_2),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                widgets::pill(
                    ui,
                    &self.tr(&Message::AboutVersionValue {
                        version: (version).to_string(),
                    }),
                    widgets::Tone::Accent,
                );
                widgets::pill(ui, commit, widgets::Tone::Neutral);
            });
        });
        widgets::card_ex(
            ui,
            text_about_display.as_str(),
            Some(text_about_only_affects_this_window.as_str()),
            None,
            |_| {},
            |ui| {
                self.language_settings(ui);
                let reduce = animation::reduce_motion();
                ui.horizontal(|ui| {
                    if widgets::toggle(
                        ui,
                        true,
                        reduce,
                        if reduce {
                            text_about_reduced_motion_is_on.as_str()
                        } else {
                            text_about_reduce_motion.as_str()
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
                    text_about_signal_particles_and_radar_scans_become_static_and.as_str(),
                );
            },
        );
        widgets::card_ex(
            ui,
            text_about_build_information.as_str(),
            None,
            None,
            |_| {},
            |ui| {
                widgets::kv_grid(
                    ui,
                    "about-build",
                    &[
                        (text_about_version.as_str(), version.to_owned()),
                        (text_about_commit.as_str(), commit.to_owned()),
                        (
                            text_about_platform.as_str(),
                            format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
                        ),
                        (
                            text_about_license.as_str(),
                            env!("CARGO_PKG_LICENSE").to_owned(),
                        ),
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
                        text_about_copied.as_str()
                    } else {
                        text_about_copy_version_information.as_str()
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
            text_about_source_repository.as_str(),
            None,
            None,
            |_| {},
            |ui| {
                widgets::mono(ui, REPOSITORY);
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    if widgets::button(ui, text_about_open_in_browser.as_str(), Kind::Secondary)
                        .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(REPOSITORY));
                    }
                });
            },
        );
    }
}
