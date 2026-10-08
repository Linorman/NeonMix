use neonmix_i18n::{Localizer, Message, ResolvedLocale, representative_messages};

#[test]
fn shipped_typed_messages_render_without_fallback() {
    for locale in [ResolvedLocale::En, ResolvedLocale::ZhCn] {
        let localizer = Localizer::new(locale);
        for message in representative_messages() {
            assert!(!localizer.render(&message).is_empty(), "{}", message.id());
        }
        assert!(
            localizer.diagnostics().is_empty(),
            "{:?}",
            localizer.diagnostics()
        );
    }
}
#[test]
fn generation_and_static_cache_invalidate_only_on_change() {
    let mut localizer = Localizer::new(ResolvedLocale::En);
    assert_eq!(localizer.render(&Message::CommonSave), "Save");
    localizer.set_locale(ResolvedLocale::En);
    assert_eq!(localizer.generation(), 0);
    localizer.set_locale(ResolvedLocale::ZhCn);
    assert_eq!(localizer.generation(), 1);
    assert_eq!(localizer.render(&Message::CommonSave), "保存");
    let mut cloned = localizer.clone();
    cloned.set_locale(ResolvedLocale::En);
    assert_eq!(cloned.generation(), 2);
    assert_eq!(localizer.locale(), ResolvedLocale::ZhCn);
}
#[test]
fn nested_semantic_messages_rerender_on_language_change() {
    let mut localizer = Localizer::new(ResolvedLocale::En);
    let message = Message::ShellRestored {
        change: Box::new(Message::CommonSave),
    };
    assert!(localizer.render(&message).contains("Save"));
    localizer.set_locale(ResolvedLocale::ZhCn);
    let rendered = localizer.render(&message);
    assert!(rendered.contains("保存"));
    assert!(!rendered.contains("Save"));
}

#[test]
fn localizer_can_be_shared_with_egui_context_data() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Localizer>();
}
