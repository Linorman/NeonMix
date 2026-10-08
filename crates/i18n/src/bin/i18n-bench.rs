//! Offline developer measurements; no UI or audio service is started.
use neonmix_i18n::{Localizer, ResolvedLocale, representative_messages};
use std::time::Instant;

fn main() {
    let started = Instant::now();
    let mut renderer = Localizer::new(ResolvedLocale::En);
    let resource_parse_ms = started.elapsed().as_secs_f64() * 1000.0;
    let messages = representative_messages();
    let mut locales = Vec::new();
    for locale in ResolvedLocale::shipped() {
        renderer.set_locale(*locale);
        let started = Instant::now();
        for message in &messages {
            std::hint::black_box(renderer.render(message));
        }
        let first_render_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        for _ in 0..20 {
            for message in &messages {
                std::hint::black_box(renderer.render(message));
            }
        }
        // Tags are validated ASCII BCP 47 identifiers from the manifest.
        locales.push(format!(r#"{{"locale":"{}","representative_messages":{},"first_render_ms":{},"warm_render_us_per_message":{}}}"#,
            locale.tag(),messages.len(),first_render_ms,started.elapsed().as_secs_f64()*1_000_000.0/(20*messages.len()) as f64));
    }
    let started = Instant::now();
    for index in 0..50 {
        renderer.set_locale(ResolvedLocale::shipped()[index % ResolvedLocale::shipped().len()]);
        for message in &messages {
            std::hint::black_box(renderer.render(message));
        }
    }
    println!(
        r#"{{"resource_parse_ms":{},"locales":[{}],"fifty_switches_and_full_renders_ms":{},"fallback_diagnostics":{}}}"#,
        resource_parse_ms,
        locales.join(","),
        started.elapsed().as_secs_f64() * 1000.0,
        renderer.diagnostics().len()
    );
}
