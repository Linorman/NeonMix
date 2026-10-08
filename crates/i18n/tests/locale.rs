use neonmix_i18n::{LanguagePreference, ResolutionReason, ResolvedLocale, resolve};

#[test]
fn system_language_matrix() {
    let matrix: &[(&[&str], ResolvedLocale)] = &[
        (&["zh-CN"], ResolvedLocale::ZhCn),
        (&["zh-SG"], ResolvedLocale::ZhCn),
        (&["zh-Hans"], ResolvedLocale::ZhCn),
        (&["zh-Hans-CN"], ResolvedLocale::ZhCn),
        (&["zh"], ResolvedLocale::ZhCn),
        (&["zh-Hans-HK"], ResolvedLocale::ZhCn),
        (&["en-US"], ResolvedLocale::En),
        (&["en-GB"], ResolvedLocale::En),
        (&["fr-FR", "zh-CN", "en"], ResolvedLocale::ZhCn),
        (&["zh-TW", "en-GB"], ResolvedLocale::En),
        (&["zh-Hant-CN"], ResolvedLocale::En),
        (&["zh-HK"], ResolvedLocale::En),
        (&["zh-MO"], ResolvedLocale::En),
        (&["zh-Hant", "zh-Hans"], ResolvedLocale::ZhCn),
        (&["en-US-u-ca-gregory"], ResolvedLocale::En),
        (&["zh-Hans-HK-u-nu-hanidec"], ResolvedLocale::ZhCn),
        (&["zh_CN.UTF-8"], ResolvedLocale::ZhCn),
        (&[], ResolvedLocale::En),
        (&["", "C", "POSIX", "???"], ResolvedLocale::En),
        (&["fr-FR", "invalid-!"], ResolvedLocale::En),
    ];
    for (candidates, expected) in matrix {
        let candidates = candidates.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            resolve(&LanguagePreference::Auto, &candidates).locale,
            *expected,
            "{candidates:?}"
        );
    }
}
#[test]
fn explicit_preference_is_retained_and_overrides_system() {
    assert_eq!(
        resolve(
            &LanguagePreference::Explicit("en".into()),
            &["zh-CN".into()]
        )
        .locale,
        ResolvedLocale::En
    );
    assert_eq!(
        resolve(
            &LanguagePreference::Explicit("zh-CN".into()),
            &["en".into()]
        )
        .locale,
        ResolvedLocale::ZhCn
    );
    let unknown = LanguagePreference::Explicit("future-language".into());
    assert_eq!(
        resolve(&unknown, &["zh-CN".into()]).reason,
        ResolutionReason::UnsupportedPreference
    );
    let json = serde_json::to_string(&unknown).unwrap();
    assert_eq!(
        serde_json::from_str::<LanguagePreference>(&json).unwrap(),
        unknown
    );
    assert_eq!(
        serde_json::to_string(&LanguagePreference::default()).unwrap(),
        "\"auto\""
    );
}
#[test]
fn reason_distinguishes_match_and_fallback() {
    assert_eq!(
        resolve(&LanguagePreference::Auto, &["fr-FR".into(), "en-GB".into()]).reason,
        ResolutionReason::SystemMatch { index: 1 }
    );
    assert_eq!(
        resolve(&LanguagePreference::Auto, &[]).reason,
        ResolutionReason::NoSystemMatch
    );
}

#[test]
fn shipped_enum_and_menu_metadata_are_generated_from_manifest() {
    let manifest =
        neonmix_i18n::check::manifest_from_str(include_str!("../locales/manifest.toml")).unwrap();
    let shipped = manifest
        .locale
        .iter()
        .filter(|entry| entry.status == "shipped")
        .collect::<Vec<_>>();
    assert_eq!(shipped.len(), ResolvedLocale::shipped().len());
    for entry in shipped {
        let locale = ResolvedLocale::from_tag(&entry.tag).unwrap();
        assert!(ResolvedLocale::shipped().contains(&locale));
        assert_eq!(locale.native_name(), entry.native_name);
        assert_eq!(locale.tag(), entry.tag);
        assert_eq!(locale.direction(), neonmix_i18n::TextDirection::Ltr);
    }
    assert!(ResolvedLocale::from_tag("auto").is_none());
    assert!(ResolvedLocale::from_tag("unshipped").is_none());
}
