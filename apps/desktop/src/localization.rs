//! UI-owned localization. The context shares the same renderer with controls.
use super::*;
use neonmix_i18n::{LanguagePreference, Localizer, Message, ResolvedLocale, resolve};
use std::sync::Arc;

pub(crate) trait SystemLocaleProvider {
    fn candidates(&self) -> Result<Vec<String>, ()>;
}
pub(crate) struct NativeLocaleProvider;
impl SystemLocaleProvider for NativeLocaleProvider {
    fn candidates(&self) -> Result<Vec<String>, ()> {
        let candidates: Vec<String> = sys_locale::get_locales().collect();
        // sys-locale folds native API failures into an empty iterator. Treat
        // that unavailable read conservatively; injectable providers can
        // separately represent a successful, genuinely empty preference list.
        if candidates.is_empty() {
            Err(())
        } else {
            Ok(candidates)
        }
    }
}
pub(crate) fn install(ctx: &egui::Context, renderer: Arc<Localizer>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("neonmix-localizer"), renderer));
}
pub(crate) fn renderer(ctx: &egui::Context) -> Arc<Localizer> {
    ctx.data(|d| d.get_temp::<Arc<Localizer>>(egui::Id::new("neonmix-localizer")))
        .unwrap_or_else(|| Arc::new(Localizer::new(ResolvedLocale::En)))
}
pub(crate) fn text(ui: &egui::Ui, message: &Message) -> String {
    renderer(ui.ctx()).render(message)
}

pub(crate) struct Localization {
    pub renderer: Arc<Localizer>,
    pub locale: ResolvedLocale,
    pub fallback: bool,
    pub composing: bool,
    pub focus_language: bool,
    language_popup_open: bool,
    was_focused: bool,
    pending: Option<ResolvedLocale>,
    candidates: Option<Vec<String>>,
}
impl Localization {
    pub fn new(preference: &LanguagePreference, candidates: Option<Vec<String>>) -> Self {
        let resolution = resolve(preference, candidates.as_deref().unwrap_or(&[]));
        Self {
            renderer: Arc::new(Localizer::new(resolution.locale)),
            locale: resolution.locale,
            fallback: matches!(preference, LanguagePreference::Auto)
                && !has_match(candidates.as_deref().unwrap_or(&[])),
            composing: false,
            focus_language: false,
            language_popup_open: false,
            was_focused: false,
            pending: None,
            candidates,
        }
    }
    pub fn detect(&mut self, preference: &LanguagePreference, provider: &dyn SystemLocaleProvider) {
        if !matches!(preference, LanguagePreference::Auto) {
            return;
        }
        let Ok(candidates) = provider.candidates() else {
            return;
        };
        self.fallback = !has_match(&candidates);
        self.pending = Some(resolve(preference, &candidates).locale);
        self.candidates = Some(candidates);
    }
    pub fn select(&mut self, preference: &LanguagePreference) {
        self.fallback = matches!(preference, LanguagePreference::Auto)
            && !has_match(self.candidates.as_deref().unwrap_or(&[]));
        self.pending = Some(resolve(preference, self.candidates.as_deref().unwrap_or(&[])).locale);
    }
    pub fn frame(
        &mut self,
        ctx: &egui::Context,
        preference: &LanguagePreference,
        preview: bool,
    ) -> bool {
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Ime(ime) = event {
                match ime {
                    egui::ImeEvent::Preedit(text) => self.composing = !text.is_empty(),
                    egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => self.composing = false,
                    egui::ImeEvent::Enabled => {}
                }
            }
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if focused && !self.was_focused && !preview {
            self.detect(preference, &NativeLocaleProvider);
        }
        self.was_focused = focused;
        let changed = if !self.composing
            && let Some(locale) = self.pending.take()
            && locale != self.locale
        {
            self.locale = locale;
            Arc::make_mut(&mut self.renderer).set_locale(locale);
            true
        } else {
            false
        };
        install(ctx, self.renderer.clone());
        changed
    }
}

fn has_match(candidates: &[String]) -> bool {
    matches!(
        resolve(&LanguagePreference::Auto, candidates).reason,
        neonmix_i18n::ResolutionReason::SystemMatch { .. }
    )
}

impl Desktop {
    pub(crate) fn tr(&self, message: &Message) -> String {
        self.localization.renderer.render(message)
    }
    pub(crate) fn relative_time(&self, at: Instant) -> String {
        events::ago(&self.localization.renderer, at)
    }
    pub(crate) fn language_settings(&mut self, ui: &mut egui::Ui) {
        let label = widgets::caption(ui, &self.tr(&Message::PreferencesLanguage));
        let mut selected = self.preferences.value.language.clone();
        let auto = self.tr(&Message::PreferencesAuto);
        let shown = match &selected {
            LanguagePreference::Auto => auto.as_str(),
            LanguagePreference::Explicit(tag) => ResolvedLocale::from_tag(tag)
                .unwrap_or(ResolvedLocale::En)
                .native_name(),
        };
        let response = egui::ComboBox::from_id_salt("display-language")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                widgets::select_value(ui, &mut selected, LanguagePreference::Auto, &auto);
                for locale in ResolvedLocale::shipped() {
                    widgets::select_value(
                        ui,
                        &mut selected,
                        LanguagePreference::Explicit(locale.tag().into()),
                        locale.native_name(),
                    );
                }
            })
            .response
            .labelled_by(label.id);
        if response.clicked() && !self.preview {
            self.localization
                .detect(&self.preferences.value.language, &NativeLocaleProvider);
        }
        widgets::label_combo(&response, &self.tr(&Message::PreferencesLanguage));
        let popup_open = egui::ComboBox::is_open(ui.ctx(), response.id);
        if self.localization.language_popup_open && !popup_open {
            response.request_focus();
        }
        self.localization.language_popup_open = popup_open;
        if self.localization.focus_language {
            response.request_focus();
            response.scroll_to_me(Some(egui::Align::Center));
            self.localization.focus_language = false;
        }
        if selected != self.preferences.value.language {
            if matches!(selected, LanguagePreference::Auto) && !self.preview {
                self.localization.detect(&selected, &NativeLocaleProvider);
            }
            self.preferences.language(selected);
            self.localization.select(&self.preferences.value.language);
            if self.preferences.save().is_err() {
                self.message = Message::PreferencesSaveFailed;
                self.error = true;
            }
            response.request_focus();
            ui.ctx().request_repaint();
        }
        if matches!(self.preferences.value.language, LanguagePreference::Auto) {
            let language = self.localization.locale.native_name();
            widgets::note(
                ui,
                self.tr(&Message::PreferencesUsing {
                    language: language.into(),
                }),
            );
            if self.localization.fallback {
                widgets::note(ui, self.tr(&Message::PreferencesFallback));
            }
        } else if matches!(&self.preferences.value.language,LanguagePreference::Explicit(tag) if ResolvedLocale::from_tag(tag).is_none())
        {
            widgets::note(ui, self.tr(&Message::PreferencesUnknown));
        }
        if self.preferences.read_failed {
            widgets::error_text(ui, self.tr(&Message::PreferencesReadFailed));
        }
        if self.preferences.pending {
            widgets::error_text(ui, self.tr(&Message::PreferencesSaveFailed));
            if widgets::button(
                ui,
                &self.tr(&Message::PreferencesRetry),
                widgets::Kind::Secondary,
            )
            .clicked()
            {
                let _ = self.preferences.save();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Provider(Result<Vec<String>, ()>);
    impl SystemLocaleProvider for Provider {
        fn candidates(&self) -> Result<Vec<String>, ()> {
            self.0.clone()
        }
    }
    fn frame(
        localization: &mut Localization,
        preference: &LanguagePreference,
        events: Vec<egui::Event>,
    ) {
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                localization.frame(ctx, preference, true);
            },
        );
    }
    #[test]
    fn detection_empty_failure_manual_override_and_auto_return_are_distinct() {
        let auto = LanguagePreference::Auto;
        let mut l = Localization::new(
            &auto,
            Some(vec!["fr-FR".into(), "zh-Hans-CN".into(), "en".into()]),
        );
        assert_eq!(l.locale, ResolvedLocale::ZhCn);
        assert!(!l.fallback);
        l.detect(&auto, &Provider(Err(())));
        frame(&mut l, &auto, vec![]);
        assert_eq!(l.locale, ResolvedLocale::ZhCn);
        l.detect(&auto, &Provider(Ok(vec![])));
        frame(&mut l, &auto, vec![]);
        assert_eq!(l.locale, ResolvedLocale::En);
        assert!(l.fallback);
        let manual = LanguagePreference::Explicit("zh-CN".into());
        l.select(&manual);
        frame(&mut l, &manual, vec![]);
        l.detect(&manual, &Provider(Ok(vec!["en-US".into()])));
        frame(&mut l, &manual, vec![]);
        assert_eq!(l.locale, ResolvedLocale::ZhCn);
        l.detect(&auto, &Provider(Ok(vec!["zh-TW".into(), "en-GB".into()])));
        l.select(&auto);
        frame(&mut l, &auto, vec![]);
        assert_eq!(l.locale, ResolvedLocale::En);
        assert!(!l.fallback);
    }
    #[test]
    fn locale_commit_waits_for_ime_and_retains_generation_when_unchanged() {
        let auto = LanguagePreference::Auto;
        let mut l = Localization::new(&auto, Some(vec!["zh-CN".into()]));
        l.composing = true;
        l.select(&LanguagePreference::Explicit("en".into()));
        frame(&mut l, &auto, vec![]);
        assert_eq!(l.locale, ResolvedLocale::ZhCn);
        frame(
            &mut l,
            &auto,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("原始草稿".into()))],
        );
        assert_eq!(l.locale, ResolvedLocale::En);
        assert_eq!(l.renderer.generation(), 1);
        l.select(&LanguagePreference::Explicit("en".into()));
        frame(&mut l, &auto, vec![]);
        assert_eq!(l.renderer.generation(), 1);
    }
}
