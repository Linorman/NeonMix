mod native;
use clap::Parser;
use neonmix_credential_migrate::{Layout, Options, ProfileKind, cleanup_staging, migrate};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Parser)]
#[command(
    about = "Offline identity-preserving copy migration. Stop UI, background, Hub, Sender and AirPlay worker first. Native entries and source state are retained."
)]
struct Arguments {
    #[arg(long, value_enum, required_unless_present = "cleanup_staging")]
    layout: Option<Layout>,
    #[arg(long, required_unless_present = "cleanup_staging")]
    source: Option<PathBuf>,
    #[arg(long, required_unless_present = "cleanup_staging")]
    destination: Option<PathBuf>,
    #[arg(long, value_enum)]
    profile_kind: Option<ProfileKind>,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=300))]
    native_timeout_seconds: u64,
    #[arg(long, conflicts_with_all = ["layout", "source", "destination", "profile_kind"])]
    cleanup_staging: Option<PathBuf>,
}
struct CancelHooks(Arc<AtomicBool>);
impl neonmix_credential_migrate::Hooks for CancelHooks {
    fn checkpoint(
        &mut self,
        _: neonmix_credential_migrate::Phase,
    ) -> neonmix_credential_migrate::Result<()> {
        if self.0.load(Ordering::Acquire) {
            Err(neonmix_credential_migrate::Error("migration_cancelled"))
        } else {
            Ok(())
        }
    }
}
fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|v| v == "--native-read-helper")
    {
        std::process::exit(if native::helper(false) { 0 } else { 1 });
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|v| v == "--native-fixture-helper")
    {
        std::process::exit(if native::helper(true) { 0 } else { 1 });
    }
    let arguments = Arguments::parse();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    if ctrlc::set_handler(move || signal.store(true, Ordering::Release)).is_err() {
        eprintln!("migration_signal_handler_failed");
        std::process::exit(1);
    }
    let outcome = if let Some(path) = arguments.cleanup_staging {
        cleanup_staging(&path).map(|_| serde_json::json!({"event":"migration_staging_cleaned"}))
    } else {
        let options = Options {
            layout: arguments.layout.unwrap(),
            source: arguments.source.unwrap(),
            destination: arguments.destination.unwrap(),
            profile_kind: arguments.profile_kind,
        };
        let mut reader = native::NativeReader {
            timeout: Duration::from_secs(arguments.native_timeout_seconds),
            cancelled: cancelled.clone(),
        };
        migrate(&options, &mut reader, &mut CancelHooks(cancelled)).and_then(|report| {
            serde_json::to_value(report)
                .map_err(|_| neonmix_credential_migrate::Error("migration_report_failed"))
        })
    };
    match outcome {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({"event":"credential_migration_failed","code":error.0})
            );
            std::process::exit(1);
        }
    }
}
