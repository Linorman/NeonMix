//! Test fixture only; never included in the installation package.
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().collect();
    #[cfg(windows)]
    if args.iter().any(|a| a == "--job-owner") {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "serve",
                "--managed-control-stdin",
                "--spawn-descendant",
                "--ignore-stop",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let (mut child, _job) = neonmix_lifecycle::windows_job::spawn(&mut command).unwrap();
        // Retain the writer even when a wait future is present.
        let _input = child.stdin.take();
        println!(
            "{{\"event\":\"job_child\",\"pid\":{}}}",
            child.id().unwrap()
        );
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
    if args.iter().any(|a| a == "--descendant") {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
    let managed = args.iter().any(|a| a == "--managed-control-stdin");
    let stop = neonmix_lifecycle::StopSignal::new(managed).expect("private control pipe");
    let mut descendant = if args.iter().any(|a| a == "--spawn-descendant") {
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("--descendant")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        println!("{{\"event\":\"descendant\",\"pid\":{}}}", child.id());
        Some(child)
    } else {
        None
    };
    println!("{{\"event\":\"hub_started\",\"managed_control_version\":1}}");
    if args.iter().any(|a| a == "--flood-log") {
        let line = [b'x'; 8192];
        for _ in 0..100_000 {
            if std::io::stdout().write_all(&line).is_err() {
                break;
            }
        }
    }
    if args.iter().any(|a| a == "--ignore-stop") {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
    stop.wait().await.unwrap();
    // Gives the cancellation test a deterministic period while owner cleanup
    // proceeds independently of the request future.
    tokio::time::sleep(Duration::from_millis(150)).await;
    if let Some(child) = descendant.as_mut() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    println!("{{\"event\":\"shutdown_complete\",\"forced\":false,\"cleanup_complete\":true}}");
}
