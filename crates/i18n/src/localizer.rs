use crate::{EMBEDDED_RESOURCES, Message, ResolvedLocale};
use fluent_bundle::{FluentArgs, FluentResource, concurrent::FluentBundle};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

const GENERIC_FAILURE: &str = "Something went wrong. Please try again.";
const STATIC_CACHE_LIMIT: usize = 1024;
const DIAGNOSTIC_LIMIT: usize = 128;
type Bundle = FluentBundle<FluentResource>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    Resource,
    MissingMessage,
    MissingAttribute,
    Format,
}
/// Diagnostics retain only schema identity, locale and failure category. User
/// strings, paths and formatted Fluent errors never enter diagnostic storage.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    pub locale: &'static str,
    pub message_id: String,
    pub kind: DiagnosticKind,
}
struct Resources {
    bundles: HashMap<ResolvedLocale, Bundle>,
    diagnostics: Vec<Diagnostic>,
}
#[derive(Default, Clone)]
struct State {
    cache: HashMap<&'static str, String>,
    diagnostics: BTreeSet<Diagnostic>,
}
pub struct Localizer {
    resources: Arc<Resources>,
    locale: ResolvedLocale,
    generation: u64,
    state: Mutex<State>,
}
impl Clone for Localizer {
    fn clone(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            locale: self.locale,
            generation: self.generation,
            state: Mutex::new(self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()),
        }
    }
}
impl Localizer {
    pub fn new(locale: ResolvedLocale) -> Self {
        static SHIPPED: OnceLock<Arc<Resources>> = OnceLock::new();
        Self {
            resources: Arc::clone(
                SHIPPED.get_or_init(|| Arc::new(load_resources(EMBEDDED_RESOURCES))),
            ),
            locale,
            generation: 0,
            state: Mutex::new(State::default()),
        }
    }
    pub const fn locale(&self) -> ResolvedLocale {
        self.locale
    }
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    pub fn set_locale(&mut self, locale: ResolvedLocale) {
        if self.locale == locale {
            return;
        }
        self.locale = locale;
        self.generation = self.generation.wrapping_add(1);
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cache
            .clear();
    }
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.resources
            .diagnostics
            .iter()
            .chain(state.diagnostics.iter())
            .cloned()
            .collect()
    }
    pub fn render(&self, message: &Message) -> String {
        let args = message.args(self);
        if args.iter().next().is_none() {
            let cached = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .cache
                .get(message.id())
                .cloned();
            if let Some(value) = cached {
                return value;
            }
            let value = self.render_pattern(message.id(), &args);
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.cache.len() < STATIC_CACHE_LIMIT {
                state.cache.insert(message.id(), value.clone());
            }
            value
        } else {
            self.render_pattern(message.id(), &args)
        }
    }
    fn record(&self, locale: ResolvedLocale, id: &str, kind: DiagnosticKind) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.diagnostics.len() < DIAGNOSTIC_LIMIT {
            state.diagnostics.insert(Diagnostic {
                locale: locale.tag(),
                message_id: id.to_owned(),
                kind,
            });
        }
    }
    fn render_pattern(&self, id: &str, args: &FluentArgs<'_>) -> String {
        for locale in [self.locale, ResolvedLocale::En].into_iter().take(
            if self.locale == ResolvedLocale::En {
                1
            } else {
                2
            },
        ) {
            let Some(bundle) = self.resources.bundles.get(&locale) else {
                self.record(locale, id, DiagnosticKind::Resource);
                continue;
            };
            let (key, attribute) = id.split_once('.').map_or((id, None), |(k, a)| (k, Some(a)));
            let Some(message) = bundle.get_message(key) else {
                self.record(locale, id, DiagnosticKind::MissingMessage);
                continue;
            };
            let pattern = if let Some(attribute) = attribute {
                message.get_attribute(attribute).map(|a| a.value())
            } else {
                message.value()
            };
            let Some(pattern) = pattern else {
                self.record(locale, id, DiagnosticKind::MissingAttribute);
                continue;
            };
            let mut errors = vec![];
            let rendered = bundle.format_pattern(pattern, Some(args), &mut errors);
            if errors.is_empty() {
                return rendered.into_owned();
            }
            self.record(locale, id, DiagnosticKind::Format);
        }
        GENERIC_FAILURE.to_owned()
    }
}
fn load_resources(sources: &[(&str, &str)]) -> Resources {
    let mut bundles = HashMap::new();
    let mut diagnostics = vec![];
    for &locale in ResolvedLocale::shipped() {
        let mut bundle = FluentBundle::new_concurrent(vec![
            locale
                .tag()
                .parse()
                .expect("registered language identifier"),
        ]);
        for (_, source) in sources.iter().filter(|(tag, _)| *tag == locale.tag()) {
            let Ok(resource) = FluentResource::try_new((*source).to_owned()) else {
                diagnostics.push(Diagnostic {
                    locale: locale.tag(),
                    message_id: String::new(),
                    kind: DiagnosticKind::Resource,
                });
                continue;
            };
            if bundle.add_resource(resource).is_err() {
                diagnostics.push(Diagnostic {
                    locale: locale.tag(),
                    message_id: String::new(),
                    kind: DiagnosticKind::Resource,
                });
            }
        }
        // Keep Fluent's bidi isolation enabled. Copy actions use original data.
        bundles.insert(locale, bundle);
    }
    Resources {
        bundles,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(sources: &[(&str, &str)]) -> Localizer {
        Localizer {
            resources: Arc::new(load_resources(sources)),
            locale: ResolvedLocale::ZhCn,
            generation: 0,
            state: Mutex::new(State::default()),
        }
    }
    #[test]
    fn repeated_switches_reuse_resources_and_dynamic_cache_stays_empty() {
        let mut localizer = Localizer::new(ResolvedLocale::En);
        let resources = Arc::clone(&localizer.resources);
        for index in 0..50 {
            localizer.set_locale(if index % 2 == 0 {
                ResolvedLocale::ZhCn
            } else {
                ResolvedLocale::En
            });
            for count in 0..20 {
                localizer.render(&Message::FlowOfflineCount { count });
            }
            assert!(localizer.state.lock().unwrap().cache.is_empty());
            assert!(Arc::ptr_eq(&resources, &localizer.resources));
        }
        assert_eq!(localizer.generation(), 50);
        assert!(localizer.diagnostics().is_empty());
    }
    #[test]
    fn fallback_is_bounded_and_diagnostics_are_deduplicated() {
        let localizer = fixture(&[("en", "test = Readable\n")]);
        for _ in 0..3 {
            assert_eq!(
                localizer.render_pattern("test", &FluentArgs::new()),
                "Readable"
            );
        }
        assert_eq!(localizer.diagnostics().len(), 1);
        assert_eq!(
            localizer.render_pattern("missing", &FluentArgs::new()),
            GENERIC_FAILURE
        );
    }
    #[test]
    fn format_errors_fall_back_without_recording_secret_values() {
        let localizer = fixture(&[
            ("zh-CN", "test = { $secret }\n"),
            ("en", "test = Try again\n"),
        ]);
        assert_eq!(
            localizer.render_pattern("test", &FluentArgs::new()),
            "Try again"
        );
        assert_eq!(localizer.diagnostics()[0].kind, DiagnosticKind::Format);
        assert!(!format!("{:?}", localizer.diagnostics()).contains("secret"));
    }
    #[test]
    fn literal_values_are_isolated_and_not_parsed() {
        let localizer = fixture(&[("en", "test = Connecting { $name }\n")]);
        let mut args = FluentArgs::new();
        args.set("name", "{ $evil } العربية 😀");
        let value = localizer.render_pattern("test", &args);
        assert!(value.contains("\u{2068}{ $evil } العربية 😀\u{2069}"));
    }
}
