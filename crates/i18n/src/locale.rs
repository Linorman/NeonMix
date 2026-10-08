use crate::ResolvedLocale;
use language_tags::LanguageTag;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LanguagePreference {
    #[default]
    Auto,
    Explicit(String),
}

impl Serialize for LanguagePreference {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Auto => "auto",
            Self::Explicit(value) => value,
        })
    }
}
impl<'de> Deserialize<'de> for LanguagePreference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(if value == "auto" {
            Self::Auto
        } else {
            Self::Explicit(value)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionReason {
    Explicit,
    SystemMatch { index: usize },
    UnsupportedPreference,
    NoSystemMatch,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub locale: ResolvedLocale,
    pub reason: ResolutionReason,
}

/// Match the entire ordered system preference list. Explicit scripts take
/// precedence over regional inference; extensions are parsed as BCP 47 first.
pub fn resolve(preference: &LanguagePreference, candidates: &[String]) -> Resolution {
    match preference {
        LanguagePreference::Explicit(tag) => match match_locale(tag) {
            Some(locale) => Resolution {
                locale,
                reason: ResolutionReason::Explicit,
            },
            None => Resolution {
                locale: ResolvedLocale::En,
                reason: ResolutionReason::UnsupportedPreference,
            },
        },
        LanguagePreference::Auto => candidates
            .iter()
            .enumerate()
            .find_map(|(index, candidate)| {
                match_locale(candidate).map(|locale| Resolution {
                    locale,
                    reason: ResolutionReason::SystemMatch { index },
                })
            })
            .unwrap_or(Resolution {
                locale: ResolvedLocale::En,
                reason: ResolutionReason::NoSystemMatch,
            }),
    }
}

fn match_locale(input: &str) -> Option<ResolvedLocale> {
    // sys-locale supplies BCP 47. The POSIX form is accepted for injected Linux
    // fixtures and older platform adapters, without using the code page.
    let base = input.trim().split('.').next()?.replace('_', "-");
    if base.contains('@') {
        return None;
    }
    let tag = LanguageTag::parse(&base).ok()?;
    let language = tag.primary_language();
    let script = tag.script();
    let region = tag.region();
    // Build a script-aware base before consulting the sole alias registry.
    // Once script is explicit, the region cannot override that script.
    let normalized = if let Some(script) = script {
        format!("{language}-{script}")
    } else if let Some(region) = region {
        format!("{language}-{region}")
    } else {
        language.to_owned()
    };
    registry()?
        .locale
        .iter()
        .filter(|entry| entry.status == "shipped")
        .find(|entry| {
            entry.aliases.iter().any(|alias| {
                if let Some(prefix) = alias.strip_suffix("-*") {
                    normalized.eq_ignore_ascii_case(prefix)
                        || normalized
                            .to_ascii_lowercase()
                            .starts_with(&format!("{}-", prefix.to_ascii_lowercase()))
                } else {
                    normalized.eq_ignore_ascii_case(alias)
                }
            })
        })
        .and_then(|entry| ResolvedLocale::from_tag(&entry.tag))
}

fn registry() -> Option<&'static crate::check::Manifest> {
    static REGISTRY: std::sync::OnceLock<Result<crate::check::Manifest, String>> =
        std::sync::OnceLock::new();
    REGISTRY
        .get_or_init(|| crate::check::manifest_from_str(include_str!("../locales/manifest.toml")))
        .as_ref()
        .ok()
}
