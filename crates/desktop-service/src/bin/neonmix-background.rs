use clap::Parser;
#[derive(Parser)]
#[command(about = "NeonMix per-user background audio owner (no system installation)")]
struct Args {
    #[arg(long, default_value = ".local/desktop")]
    state_dir: std::path::PathBuf,
}
#[tokio::main]
async fn main() {
    let args = Args::parse();
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
