//! Packaged only on Unix. Parent and child control are private stdio pipes.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(clap::Parser)]
struct Args {
    #[arg(long)]
    child: std::path::PathBuf,
    #[arg(long)]
    pipe_child: bool,
    #[arg(last = true, required = true)]
    args: Vec<std::ffi::OsString>,
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use clap::Parser;
        let args = Args::parse();
        match neonmix_desktop_service::guardian::run(&args.child, &args.args, args.pipe_child).await
        {
            Ok(code) => std::process::exit(code),
            Err(_) => {
                eprintln!("runtime_cleanup_incomplete");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        eprintln!("guardian_unavailable");
        std::process::exit(1);
    }
}
