use std::process::Command;
#[test]
fn offline_probe_returns_parseable_metrics() {
    let result = Command::new(env!("CARGO_BIN_EXE_neonmix-audio"))
        .args([
            "simulate",
            "--input-rate",
            "44100",
            "--output-rate",
            "48000",
            "--seconds",
            "1",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(value["frames"], 48000);
    assert!(value["rms"].as_f64().unwrap() > 0.04);
}
#[test]
fn invalid_format_fails_without_claiming_success() {
    let result = Command::new(env!("CARGO_BIN_EXE_neonmix-audio"))
        .args(["simulate", "--input-rate", "8000"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["event"], "error");
}
#[test]
fn unsupported_platform_sink_fails_explicitly() {
    if cfg!(target_os = "linux") {
        return;
    }
    let result = Command::new(env!("CARGO_BIN_EXE_neonmix-audio"))
        .args(["sink", "--seconds", "1"])
        .output()
        .unwrap();
    assert!(!result.status.success());
}
