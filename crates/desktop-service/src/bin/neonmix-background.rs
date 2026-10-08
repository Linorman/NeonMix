use clap::Parser;
#[derive(Parser)]
#[command(about = "NeonMix per-user background audio owner (no system installation)")]
struct Args {
    #[arg(long, default_value = ".local/desktop")]
    state_dir: std::path::PathBuf,
    /// Stop an existing owner only. Never starts a background process.
    #[arg(long, conflicts_with = "shutdown_installation")]
    shutdown: bool,
    /// Installer maintenance: stop this installation and close its desktop UI.
    #[arg(long, requires = "installation_root")]
    shutdown_installation: bool,
    #[arg(long, requires = "shutdown_installation")]
    installation_root: Option<std::path::PathBuf>,
}
#[tokio::main]
async fn main() {
    let args = Args::parse();
    if args.shutdown || args.shutdown_installation {
        let result = tokio::task::spawn_blocking(move || {
            #[cfg(windows)]
            if args.shutdown_installation {
                return neonmix_desktop_service::maintenance::stop_installation(
                    &args.state_dir,
                    &args.installation_root.expect("required argument"),
                );
            }
            let client = neonmix_desktop_service::Client::new(args.state_dir);
            let reply = client.request(&neonmix_desktop_service::Request::Shutdown)?;
            if reply.ok {
                Ok(())
            } else {
                Err(reply.error.unwrap_or_else(|| "停止未完成".into()))
            }
        })
        .await;
        match result {
            Ok(Ok(())) => println!("{{\"event\":\"maintenance_complete\"}}"),
            error => {
                eprintln!("maintenance_failed: {error:?}");
                std::process::exit(1);
            }
        }
        return;
    }
    #[cfg(any(unix, windows))]
    if let Err(error) = neonmix_desktop_service::daemon::serve(args.state_dir).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = args;
        eprintln!("desktop background IPC is not implemented on this platform");
        std::process::exit(1);
    }
}
