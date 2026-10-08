use neonmix_desktop_service::{Fault, FaultCode, ProcessStatus, Reply, StopResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// The exact pre-fault structs use Serde's original unknown-field behavior. These
// are real serialization/deserialization paths, not a manually stripped reply.
#[derive(Debug, Serialize, Deserialize)]
struct OldReply {
    ok: bool,
    data: Value,
    error: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
struct OldProcessStatus {
    running: bool,
    #[serde(default)]
    stopping: bool,
    #[serde(default)]
    stop_result: Option<StopResult>,
    pid: Option<u32>,
    error: Option<String>,
    last_event: Option<Value>,
    metrics: Option<Value>,
}

#[test]
fn real_reply_serialization_covers_all_four_client_backend_combinations() {
    let old_backend = OldReply {
        ok: false,
        data: Value::Null,
        error: Some("revision_conflict".into()),
    };
    let old_wire = serde_json::to_vec(&old_backend).unwrap();
    let old_old: OldReply = serde_json::from_slice(&old_wire).unwrap();
    assert_eq!(old_old.error.as_deref(), Some("revision_conflict"));
    let new_old: Reply = serde_json::from_slice(&old_wire).unwrap();
    assert!(new_old.fault.is_none());
    assert_eq!(
        Fault::from_machine_code(new_old.error.as_deref().unwrap())
            .unwrap()
            .code,
        FaultCode::RevisionConflict
    );

    let new_wire = serde_json::to_vec(&Reply::failure("revision_conflict")).unwrap();
    let new_new: Reply = serde_json::from_slice(&new_wire).unwrap();
    assert_eq!(new_new.fault.unwrap().code, FaultCode::RevisionConflict);
    let old_new: OldReply = serde_json::from_slice(&new_wire).unwrap();
    assert!(!old_new.ok);
    assert_eq!(old_new.error.as_deref(), Some("revision_conflict"));

    let success_wire = serde_json::to_vec(&Reply::success(json!({"event":"ok"}))).unwrap();
    assert!(
        serde_json::from_slice::<Reply>(&success_wire)
            .unwrap()
            .fault
            .is_none()
    );
    assert!(
        serde_json::from_slice::<OldReply>(&success_wire)
            .unwrap()
            .ok
    );
}

#[test]
fn real_persistent_status_serialization_is_compatible_in_both_directions() {
    let old = OldProcessStatus {
        running: false,
        stopping: false,
        stop_result: None,
        pid: None,
        error: Some("worker_unavailable".into()),
        last_event: None,
        metrics: None,
    };
    let old_wire = serde_json::to_vec(&old).unwrap();
    assert!(
        serde_json::from_slice::<OldProcessStatus>(&old_wire)
            .unwrap()
            .error
            .is_some()
    );
    let new_old: ProcessStatus = serde_json::from_slice(&old_wire).unwrap();
    assert!(new_old.fault.is_none());
    let new = ProcessStatus {
        fault: Some(Fault::new(FaultCode::WorkerUnavailable)),
        ..new_old
    };
    let new_wire = serde_json::to_vec(&new).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProcessStatus>(&new_wire)
            .unwrap()
            .fault
            .unwrap()
            .code,
        FaultCode::WorkerUnavailable
    );
    assert_eq!(
        serde_json::from_slice::<OldProcessStatus>(&new_wire)
            .unwrap()
            .error
            .as_deref(),
        Some("worker_unavailable")
    );
}

#[test]
fn closed_fault_parameters_reject_unknown_types_secrets_and_invalid_ports() {
    let invalid = [
        json!({"code":"unknown_future_code","params":{}}),
        json!({"code":"revision_conflict","params":{"port":7443}}),
        json!({"code":"revision_conflict","params":{"token":"SECRET"}}),
        json!({"code":"revision_conflict","params":{},"stderr":"SECRET"}),
        json!({"code":"hub_port_in_use","params":{}}),
        json!({"code":"hub_port_in_use","params":{"port":0}}),
        json!({"code":"hub_port_in_use","params":{"port":-1}}),
        json!({"code":"hub_port_in_use","params":{"port":65536}}),
        json!({"code":"hub_port_in_use","params":{"port":"7443"}}),
        json!({"code":"hub_port_in_use","params":{"port":1.5}}),
        json!({"code":"hub_port_in_use","params":{"port":7443,"pin":"1234"}}),
    ];
    for fault in invalid {
        assert!(serde_json::from_value::<Fault>(fault.clone()).is_err());
        let reply: Reply =
            serde_json::from_value(json!({"ok":false,"data":null,"error":"legacy","fault":fault}))
                .unwrap();
        assert_eq!(reply.fault.unwrap().code, FaultCode::GenericFailure);
        let status: ProcessStatus = serde_json::from_value(json!({"running":false,"pid":null,"error":"legacy","last_event":null,"metrics":null,"fault":fault})).unwrap();
        assert_eq!(status.fault.unwrap().code, FaultCode::GenericFailure);
    }
}

#[test]
fn all_codes_round_trip_and_legacy_mapping_never_uses_translated_sentences() {
    for &code in FaultCode::ALL {
        let fault = Fault::new(code);
        assert_eq!(
            serde_json::from_slice::<Fault>(&serde_json::to_vec(&fault).unwrap()).unwrap(),
            fault
        );
        assert_eq!(Fault::from_machine_code(code.as_str()).unwrap(), fault);
    }
    for unknown in [
        "unknown",
        "权限不足",
        "private path contains revision_conflict",
        "stderr token=SECRET",
    ] {
        assert!(Fault::from_machine_code(unknown).is_none());
    }
    let reply = Reply::failure("stderr token=SECRET private-key PEM PIN=1234 /private/path");
    let wire = serde_json::to_string(&reply).unwrap();
    assert_eq!(reply.fault.unwrap().code, FaultCode::GenericFailure);
    for private in ["SECRET", "PEM", "1234", "/private/path"] {
        assert!(!wire.contains(private));
    }
    let credential = Reply::failure("credential_missing: token=SECRET /private/path");
    assert!(
        !serde_json::to_string(&credential)
            .unwrap()
            .contains("SECRET")
    );
}

#[test]
fn fault_does_not_bypass_diagnostic_export_whitelist() {
    let raw = json!({"fault":{"code":"hub_port_in_use","params":{"port":7443}},"unknown":{"token":"SECRET","port":1234}});
    let safe = neonmix_desktop_service::daemon::export_whitelist(&raw).to_string();
    for denied in ["hub_port_in_use", "SECRET", "7443", "1234"] {
        assert!(!safe.contains(denied));
    }
}

#[cfg(unix)]
#[test]
fn disconnected_client_receives_public_connection_code_without_paths() {
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let directory = project
        .join(".local/tmp")
        .join(format!("fault-client-{}", uuid::Uuid::new_v4()));
    neonmix_desktop_service::transport::prepare(&directory).unwrap();
    let error = neonmix_desktop_service::Client::new(&directory)
        .request(&neonmix_desktop_service::Request::Status)
        .unwrap_err();
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(error, "connection_unavailable");
    assert_eq!(
        Fault::from_machine_code(&error).unwrap().code,
        FaultCode::ConnectionUnavailable
    );
}
