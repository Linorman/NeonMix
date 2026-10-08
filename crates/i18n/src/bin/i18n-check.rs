use std::{env, path::PathBuf, process::ExitCode};

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let catalog = neonmix_i18n::check::load_catalog(&root)?;
    neonmix_i18n::check::validate(&catalog)?;
    println!(
        "i18n AST/schema validation passed: {} messages, {} shipped locales",
        catalog.messages.len(),
        catalog
            .manifest
            .locale
            .iter()
            .filter(|l| l.status == "shipped")
            .count()
    );
    while let Some(flag) = args.next() {
        let path = args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing path after {flag}"))?;
        match flag.as_str() {
            "--pseudo" => {
                let count = neonmix_i18n::check::generate_pseudo(&catalog, &path)?;
                println!("Generated {count} pseudo modules: {}", path.display());
            }
            "--ui-list" => {
                for finding in neonmix_i18n::ui_check::scan_product_calls(&path)? {
                    println!(
                        "{}:{}: {:?} via {}",
                        finding.file.display(),
                        finding.line,
                        finding.value,
                        finding.call
                    );
                }
            }
            "--ui" => {
                let findings = neonmix_i18n::ui_check::check_product_calls(
                    &path,
                    &path.join("crates/i18n/ui-exceptions.toml"),
                )?;
                for finding in &findings {
                    eprintln!(
                        "{}:{}: {:?} via {}",
                        finding.file.display(),
                        finding.line,
                        finding.value,
                        finding.call
                    );
                }
                if !findings.is_empty() {
                    return Err(format!(
                        "{} untranslated presentation literals",
                        findings.len()
                    ));
                }
                println!("Rust presentation-call AST validation passed");
            }
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("i18n validation failed: {error}");
            ExitCode::FAILURE
        }
    }
}
