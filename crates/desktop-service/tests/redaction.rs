use neonmix_desktop_service::daemon::redact;
use serde_json::json;

#[test]
fn airplay_timing_export_preserves_numbers_and_excludes_device_identity() {
    let value = json!({"airplay": {"source_name":"private name", "source_id":"private id",
        "playback_mode":"low_latency", "pairing_pin":"1234",
        "ingress":{"latency_advance_ns":1880000000u64,"playout_lead_ns":120000000,
        "protocol_lead_ns":2000000000u64,"unrecognized":42}}});
    assert_eq!(
        neonmix_desktop_service::daemon::export_whitelist(&value),
        json!({
            "airplay":{"ingress":{"latency_advance_ns":1880000000u64,
            "playout_lead_ns":120000000,"protocol_lead_ns":2000000000u64}}
        })
    );
}

#[test]
fn healthy_optional_error_stays_empty_and_secret_errors_are_removed() {
    let mut status = json!({"ready":true,"error":null,"error_count":0,"has_error":false,"worker_error":"private path or credential"});
    redact(&mut status, false);
    assert!(status["error"].is_null());
    assert_eq!(status["has_error"], false);
    assert_eq!(status["error_count"], 0);
    assert_eq!(status["worker_error"], "[redacted]");
}

#[test]
fn diagnostic_pin_and_nested_error_payloads_are_always_redacted() {
    let mut status = json!({"pairing_pin":"1234","error":{"message":"private detail"},"history":[{"client_public_key":"key material"}]});
    redact(&mut status, true);
    assert_eq!(status["pairing_pin"], "[redacted]");
    assert_eq!(status["error"], "[redacted]");
    assert_eq!(status["history"][0]["client_public_key"], "[redacted]");
}

#[test]
fn numeric_phase_diagnostics_survive_but_malformed_error_arrays_are_removed() {
    let mut status = json!({"timed_phase_error_ns":vec![-42;16],"error_details":["secret"]});
    redact(&mut status, true);
    assert_eq!(status["timed_phase_error_ns"], json!(vec![-42; 16]));
    assert_eq!(status["error_details"], "[redacted]");
    let mut malformed = json!({"timed_phase_error_ns":["secret"]});
    redact(&mut malformed, true);
    assert_eq!(malformed["timed_phase_error_ns"], "[redacted]");
}

#[test]
fn multi_receiver_aliases_and_profile_key_fields_never_enter_diagnostics() {
    let original = json!({"sources":[{"source_id":"id","alias":"private label","public_key":"public material","gain_db":-6}],
        "receiver":{"known_keys":["a"],"blocked_keys":["b"],"key_reference":"private ref"}});
    let mut functional = original.clone();
    redact(&mut functional, false);
    assert_eq!(functional["sources"][0]["alias"], "private label");
    assert_eq!(functional["sources"][0]["public_key"], "[redacted]");
    let mut diagnostic = original;
    redact(&mut diagnostic, true);
    assert_eq!(diagnostic["sources"][0]["alias"], "[redacted]");
    assert_eq!(diagnostic["sources"][0]["gain_db"], -6);
    for field in ["known_keys", "blocked_keys", "key_reference"] {
        assert_eq!(diagnostic["receiver"][field], "[redacted]");
    }
}
