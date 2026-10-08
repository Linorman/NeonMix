#![cfg(target_os = "macos")]
use neonmix_desktop_service::{
    Client, Envelope, HubSettings, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, Request, SenderOptions,
    ServiceStatus, daemon, transport,
};
use serde_json::json;
use std::{
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let path = project
            .join(".local/tmp")
            .join(format!("e07-{}", uuid::Uuid::new_v4()));
        transport::prepare(&path).unwrap();
        Self(path)
    }
    fn write(&self, relative: &str, value: serde_json::Value) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut value = value;
        if value.get("secret_ref").is_some() {
            value["version"] = json!(2);
            value["credential_store"] = json!("file");
            value["profile_kind"] = json!(if relative == "hub/admin.json" {
                "admin"
            } else {
                "member"
            });
            value["secret_ref"] = json!(uuid::Uuid::new_v4());
            if value.get("hub_id").is_none() {
                value["hub_id"] = json!(uuid::Uuid::new_v4());
            }
            if value.get("device_id").is_none() {
                value["device_id"] = json!(uuid::Uuid::new_v4());
            }
            value["request_id"] = json!(uuid::Uuid::new_v4());
            value["certificate"] = json!("fixture-public-certificate");
            value["name"] = json!("fixture");
            value["pending"] = json!(false);
            value["invitation_id"] = json!(null);
        }
        if value.get("private_key_ref").is_some() {
            value["version"] = json!(2);
            value["credential_store"] = json!("file");
            value["private_key_ref"] = json!(uuid::Uuid::new_v4());
            value["admin_token_ref"] = json!(uuid::Uuid::new_v4());
            value["certificate"] = json!("fixture-public-certificate");
            value["state_path"] = json!("state.json");
        }
        neonmix_identity::files::write_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn child(&self) -> PathBuf {
        // The production owner requires its packaged sibling. Pin the same
        // already-built guardian into this isolated fixture installation.
        std::fs::hard_link(
            PathBuf::from(env!("CARGO_BIN_EXE_neonmix-guardian")),
            self.0.join("neonmix-guardian"),
        )
        .unwrap();
        let path = self.0.join("fixture-child");
        // Exec replaces the shell: the fixture has no grandchildren to leak.
        std::fs::write(&path,br#"#!/bin/sh
case "$1" in
serve|send) if [ "$1" = serve ]; then event=hub_started; else event=sender_started; fi; printf '{"event":"%s","token":"SECRET_TOKEN"}\n' "$event"; IFS= read -r stop; printf '{"event":"shutdown_complete"}\n'; exit 0;;
snapshot) printf '%s' "$$" > "$0.query-pid"; exec /bin/sleep 30;;
devices) printf '[]\n';;
diagnostics) printf '{"meters":{"lanes":[{"stream_id":9,"peak":0.1,"rms":0.02}]},"private_key":"SECRET_KEY","device_name":"PRIVATE_DEVICE","address":"PRIVATE_HOST"}\n';;
invite)
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--out" ]; then shift; out="$1"; break; fi
        shift
    done
    if [ -e "$out" ]; then exit 1; fi
    printf '{"invitation_id":"00000000-0000-4000-8000-000000000001","secret":"INVITATION_SECRET"}' > "$out"
    printf '{"invitation_id":"00000000-0000-4000-8000-000000000001"}\n';;
cancel-invite) printf '{"event":"invitation_cancelled"}\n';;
output)
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--directory" ]; then shift; directory="$1"; break; fi
        shift
    done
    printf '{"revision":6,"enabled":false}' > "$directory/binding.json"
    printf '{"revision":6,"enabled":false}\n';;
forget)
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--credential" ]; then shift; credential="$1"; break; fi
        shift
    done
    rm -- "$credential"
    printf '{"event":"local_credential_forgotten"}\n';;
*) printf 'fixture failure containing SECRET_TOKEN\n' >&2; exit 1;;
esac
"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn request(client: &Client, request: Request) -> neonmix_desktop_service::Reply {
    let client = client.clone();
    tokio::task::spawn_blocking(move || client.request(&request).unwrap())
        .await
        .unwrap()
}
async fn status(client: &Client) -> ServiceStatus {
    serde_json::from_value(request(client, Request::Status).await.data).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bound_intent_rejects_identity_swap_and_preserves_id_and_frozen_credential_metadata() {
    let fixture = Fixture::new();
    let hub_id = uuid::Uuid::new_v4();
    let identity = uuid::Uuid::new_v4();
    fixture.write(
        "hub/admin.json",
        json!({"secret_ref":"placeholder","hub_id":hub_id,"device_id":identity}),
    );
    let helper = fixture.0.join("intent-helper.py");
    std::fs::write(&helper, br#"#!/usr/bin/env python3
import json,pathlib,sys
a=sys.argv
credential=pathlib.Path(a[a.index('--credential')+1])
command=pathlib.Path(a[a.index('--command')+1])
original=credential.parent/'admin.json'
p=json.loads(original.read_text());p['device_id']='00000000-0000-4000-8000-000000000009';original.write_text(json.dumps(p))
print(json.dumps({'command':json.loads(command.read_text()),'profile':json.loads(credential.read_text()),'frozen':credential.name.startswith('.neonmix-intent-')}))
"#).unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let running = daemon::serve_with_binaries(fixture.0.clone(), helper.clone(), helper);
    let server = tokio::spawn(running);
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert_eq!(
        status(&client).await.intent_version,
        neonmix_desktop_service::INTENT_VERSION
    );
    let id = uuid::Uuid::new_v4();
    let runtime = uuid::Uuid::new_v4();
    let command = neonmix_control::Command {
        control_version: 1,
        expected_config_revision: None,
        expected_event_sequence: None,
        request_id: id,
        runtime_epoch: Some(runtime),
        credential_id: Some(identity),
        expected_revision: Some(17),
        operation: neonmix_control::Operation::OutputMix {
            gain_db: Some(-9.),
            muted: None,
        },
    };
    let result = request(
        &client,
        Request::Intent {
            credential: "hub/admin.json".into(),
            hub: Some("https://localhost:7443".into()),
            hub_id,
            credential_id: identity,
            command: neonmix_desktop_service::IntentCommand::Native(command.clone()),
        },
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.data["command"]["request_id"], id.to_string());
    assert_eq!(result.data["command"]["expected_revision"], 17);
    assert_eq!(result.data["profile"]["device_id"], identity.to_string());
    assert_eq!(result.data["frozen"], true);
    assert!(
        std::fs::read_dir(fixture.0.join("hub"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".neonmix-intent-"))
    );
    assert!(
        std::fs::read_dir(fixture.0.join("commands"))
            .unwrap()
            .next()
            .is_none()
    );
    let swapped = request(
        &client,
        Request::Intent {
            credential: "hub/admin.json".into(),
            hub: None,
            hub_id,
            credential_id: identity,
            command: neonmix_desktop_service::IntentCommand::Native(command),
        },
    )
    .await;
    assert!(!swapped.ok);
    assert_eq!(swapped.error.as_deref(), Some("unauthenticated"));
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}
async fn wait(client: &Client) {
    for _ in 0..100 {
        let client = client.clone();
        if tokio::task::spawn_blocking(move || client.request(&Request::Status).is_ok())
            .await
            .unwrap()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("daemon did not bind");
}
fn raw(path: PathBuf, body: Vec<u8>, length: Option<u32>) -> neonmix_desktop_service::Reply {
    let mut stream = UnixStream::connect(path.join("ipc.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(&length.unwrap_or(body.len() as u32).to_be_bytes())
        .unwrap();
    if length.is_none() {
        stream.write_all(&body).unwrap();
    }
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ui_disconnect_reopen_stop_shutdown_and_failed_media_are_authoritative() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"房间","output":"physical"}),
    );
    fixture.write("hub/admin.json",json!({"secret_ref":"fixture-admin","hub_id":uuid::Uuid::new_v4(),"device_id":uuid::Uuid::new_v4(),"pending":false}));
    fixture.write("profiles/sender.json",json!({"secret_ref":"fixture-member","hub_id":uuid::Uuid::new_v4(),"device_id":uuid::Uuid::new_v4(),"pending":false}));
    let directory = fixture.0.clone();
    let daemon = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert!(request(&client, Request::HubStart).await.ok);
    let started = status(&client).await;
    let pid = started.hub.pid.unwrap();
    assert!(started.hub.running);
    assert_eq!(
        std::fs::metadata(fixture.0.join("ipc.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // A socket closed mid-request cannot own or stop media.
    let abandoned = UnixStream::connect(fixture.0.join("ipc.sock")).unwrap();
    drop(abandoned);
    drop(client);
    let reopened = Client::new(&fixture.0);
    assert_eq!(status(&reopened).await.hub.pid, Some(pid));
    assert!(
        !request(
            &reopened,
            Request::HubSettings {
                settings: HubSettings {
                    name: "新房间".into(),
                    output: "physical".into()
                }
            }
        )
        .await
        .ok
    );
    let diagnostics = request(
        &reopened,
        Request::Diagnostics {
            credential: "hub/admin.json".into(),
            hub: None,
        },
    )
    .await;
    assert!(diagnostics.ok);
    let text = diagnostics.data.to_string();
    assert!(!text.contains("SECRET_"));
    assert!(!text.contains("PRIVATE_"));
    assert_eq!(
        diagnostics.data["remote"]["meters"]["lanes"][0]["peak"],
        json!(0.1)
    );
    assert!(request(&reopened, Request::HubStop).await.ok);
    assert!(!status(&reopened).await.hub.running);
    assert!(
        request(
            &reopened,
            Request::SenderStart {
                options: SenderOptions {
                    credential: "profiles/sender.json".into(),
                    hub: None,
                    output_binding: "outputs/main".into()
                }
            }
        )
        .await
        .ok
    );
    assert!(status(&reopened).await.sender.running);
    assert!(request(&reopened, Request::SenderStop).await.ok);
    assert!(!status(&reopened).await.sender.running);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!status(&reopened).await.sender.running);
    assert!(request(&reopened, Request::HubStart).await.ok);
    let pid = status(&reopened).await.hub.pid.unwrap();
    #[allow(unsafe_code)]
    {
        // SAFETY: PID is the live child just reported by this test's daemon.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let failed = status(&reopened).await;
    assert!(!failed.hub.running);
    assert!(failed.hub.error.is_some());
    assert!(request(&reopened, Request::HubStart).await.ok);
    let final_pid = status(&reopened).await.hub.pid.unwrap();
    assert!(request(&reopened, Request::Shutdown).await.ok);
    daemon.await.unwrap().unwrap();
    assert!(!fixture.0.join("ipc.sock").exists());
    #[allow(unsafe_code)]
    {
        // SAFETY: kill with signal zero checks existence, without delivering a signal.
        assert_eq!(unsafe { libc::kill(final_pid as libc::pid_t, 0) }, -1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_transport_rejects_unknown_commands_versions_oversize_paths_and_double_daemon() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    let directory = fixture.0.clone();
    let server = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary.clone(),
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert!(
        daemon::serve_with_binaries(fixture.0.clone(), binary.clone(), binary)
            .await
            .is_err()
    );
    for value in [
        json!({"version":1,"request":{"type":"shell","command":"rm"}}),
        json!({"version":2,"request":{"type":"status"}}),
        json!({"version":1,"request":{"type":"status","unexpected":true}}),
        json!({"version":1,"request":{"type":"output","directory":"outputs/main","action":{"action":"show","unexpected":true}}}),
    ] {
        let directory = fixture.0.clone();
        assert!(
            !tokio::task::spawn_blocking(move || raw(
                directory,
                serde_json::to_vec(&value).unwrap(),
                None
            ))
            .await
            .unwrap()
            .ok
        );
    }
    let directory = fixture.0.clone();
    assert!(
        !tokio::task::spawn_blocking(move || raw(
            directory,
            Vec::new(),
            Some((MAX_MESSAGE_BYTES + 1) as u32)
        ))
        .await
        .unwrap()
        .ok
    );
    assert!(
        !request(
            &client,
            Request::Snapshot {
                credential: "../outside.json".into(),
                hub: None
            }
        )
        .await
        .ok
    );
    std::os::unix::fs::symlink("/etc", fixture.0.join("linked")).unwrap();
    assert!(
        !request(
            &client,
            Request::Snapshot {
                credential: "linked/passwd".into(),
                hub: None
            }
        )
        .await
        .ok
    );
    fixture.write("profiles/plaintext.json", json!({"token":"secret"}));
    assert!(
        !request(
            &client,
            Request::Snapshot {
                credential: "profiles/plaintext.json".into(),
                hub: None
            }
        )
        .await
        .ok
    );
    assert!(
        !request(&client, Request::Discover { seconds: 100 })
            .await
            .ok
    );
    std::fs::set_permissions(
        fixture.0.join("ipc.sock"),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(client.request(&Request::Devices).is_err());
    std::fs::set_permissions(
        fixture.0.join("lifecycle.sock"),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(client.request(&Request::Status).is_err());
    std::fs::set_permissions(
        fixture.0.join("lifecycle.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    std::fs::set_permissions(
        fixture.0.join("ipc.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(transport::prepare(&fixture.0).is_err());
    let encoded = serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        request: Request::Status,
    })
    .unwrap();
    assert!(encoded.len() < MAX_MESSAGE_BYTES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stalled_partial_request_times_out_without_blocking_authoritative_status() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    let directory = fixture.0.clone();
    let server = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let mut stalled = UnixStream::connect(fixture.0.join("ipc.sock")).unwrap();
    stalled
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stalled.write_all(&100u32.to_be_bytes()).unwrap();
    assert_eq!(status(&client).await.version, PROTOCOL_VERSION);
    let ended = tokio::task::spawn_blocking(move || {
        let mut byte = [0];
        stalled.read(&mut byte).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(ended, 0);
    assert_eq!(status(&client).await.version, PROTOCOL_VERSION);
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn urgent_stop_interrupts_a_hung_read_only_query_and_reaps_its_process() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"Room","output":"physical"}),
    );
    fixture.write("hub/admin.json", json!({"secret_ref":"fixture-admin"}));
    let directory = fixture.0.clone();
    let server = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary.clone(),
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert!(request(&client, Request::HubStart).await.ok);
    let poll = client.clone();
    let query = tokio::spawn(async move {
        request(
            &poll,
            Request::Snapshot {
                credential: "hub/admin.json".into(),
                hub: None,
            },
        )
        .await
    });
    let pid_path = binary.with_extension("query-pid");
    for _ in 0..100 {
        if pid_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid: u32 = std::fs::read_to_string(&pid_path).unwrap().parse().unwrap();
    let before = std::time::Instant::now();
    assert!(request(&client, Request::HubStop).await.ok);
    assert!(before.elapsed() < Duration::from_secs(3));
    assert!(!query.await.unwrap().ok);
    assert!(!status(&client).await.hub.running);
    for _ in 0..100 {
        #[allow(unsafe_code)]
        // SAFETY: signal zero only checks the fixture's own query process.
        let gone = unsafe { libc::kill(pid as libc::pid_t, 0) } != 0;
        if gone {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    #[allow(unsafe_code)]
    {
        // SAFETY: signal zero checks existence without delivering any signal.
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    }
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invitations_are_ephemeral_repeatable_and_export_contains_no_untrusted_strings() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write("hub/admin.json", json!({"secret_ref":"fixture-admin"}));
    fixture.write(
        "output/binding.json",
        json!({"revision":5,"display_name":"Persisted output","enabled":true}),
    );
    let directory = fixture.0.clone();
    let server = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    for _ in 0..2 {
        let reply = request(
            &client,
            Request::Invite {
                credential: "hub/admin.json".into(),
                hub: None,
                out: "invitations/same.json".into(),
                seconds: 60,
            },
        )
        .await;
        assert!(reply.ok);
        assert!(
            reply.data["invitation"]
                .as_str()
                .unwrap()
                .contains("INVITATION_SECRET")
        );
        assert!(!fixture.0.join("invitations/same.json").exists());
        assert!(
            request(
                &client,
                Request::CancelInvite {
                    credential: "hub/admin.json".into(),
                    hub: None,
                    invitation_id: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001")
                        .unwrap()
                }
            )
            .await
            .ok
        );
    }
    assert_eq!(
        status(&client).await.output_binding.unwrap()["revision"],
        json!(5)
    );
    let export = request(
        &client,
        Request::ExportDiagnostics {
            credential: "hub/admin.json".into(),
            hub: None,
        },
    )
    .await;
    assert!(export.ok);
    let path = fixture.0.join("diagnostics-redacted.json");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains("SECRET"));
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains("Persisted output"));
    assert!(text.contains("0.02"));
    let safe = daemon::export_whitelist(
        &json!({"remote":{"meters":{"peak":"SECRET","rms":0.2,"unknown":19},"errors":"password=SECRET","unknown_number":123,"lane_stream_ids":[null,9]}}),
    );
    assert_eq!(
        safe,
        json!({"remote":{"meters":{"rms":0.2},"lane_stream_ids":[null,9]}})
    );
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forgetting_a_sender_stops_media_disables_persisted_binding_and_removes_profile() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/admin.json",
        json!({"secret_ref":"admin-ref","hub_id":uuid::Uuid::new_v4()}),
    );
    fixture.write(
        "profiles/sender.json",
        json!({"secret_ref":"sender-ref","hub_id":uuid::Uuid::new_v4()}),
    );
    fixture.write(
        "output/binding.json",
        json!({"revision":5,"output_id":uuid::Uuid::new_v4(),"enabled":true}),
    );
    let directory = fixture.0.clone();
    let server = tokio::spawn(daemon::serve_with_binaries(
        directory,
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert!(
        request(
            &client,
            Request::SenderStart {
                options: SenderOptions {
                    credential: "profiles/sender.json".into(),
                    hub: None,
                    output_binding: "output".into()
                }
            }
        )
        .await
        .ok
    );
    assert!(status(&client).await.sender.running);
    assert!(
        !request(
            &client,
            Request::ForgetCredential {
                credential: "hub/admin.json".into()
            }
        )
        .await
        .ok
    );
    let forgotten = request(
        &client,
        Request::ForgetCredential {
            credential: "profiles/sender.json".into(),
        },
    )
    .await;
    assert!(forgotten.ok);
    assert_eq!(forgotten.data["output_authorization_required"], json!(true));
    assert!(!status(&client).await.sender.running);
    assert!(status(&client).await.sender_options.is_none());
    assert_eq!(
        status(&client).await.output_binding.unwrap()["enabled"],
        json!(false)
    );
    assert!(!fixture.0.join("profiles/sender.json").exists());
    assert!(fixture.0.join("hub/admin.json").exists());
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_versions_and_admin_aliases_fail_closed_without_loading_secrets() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write("hub/admin.json", json!({"secret_ref":"fixture"}));
    fixture.write("profiles/member.json", json!({"secret_ref":"fixture"}));
    let admin: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.0.join("hub/admin.json")).unwrap()).unwrap();
    let mut alias = admin.clone();
    alias["profile_kind"] = json!("member");
    neonmix_identity::files::write_new(
        &fixture.0.join("profiles/alias.json"),
        &serde_json::to_vec(&alias).unwrap(),
    )
    .unwrap();
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let legacy_hub = fixture.0.join("hub/server.json");
    neonmix_identity::files::write_new(
        &legacy_hub,
        &serde_json::to_vec(&json!({"version":1,"private_key_ref":uuid::Uuid::new_v4()})).unwrap(),
    )
    .unwrap();
    let setup = request(
        &client,
        Request::HubSetup {
            settings: HubSettings {
                name: "room".into(),
                output: "fixture-output".into(),
            },
        },
    )
    .await;
    assert!(!setup.ok);
    assert!(setup.error.as_deref().unwrap().contains("离线迁移"));
    assert!(legacy_hub.exists());
    let rejected = request(
        &client,
        Request::ForgetCredential {
            credential: "profiles/alias.json".into(),
        },
    )
    .await;
    assert!(!rejected.ok);
    assert!(fixture.0.join("profiles/alias.json").exists());
    let path = fixture.0.join("profiles/member.json");
    let member: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut legacy = member.clone();
    legacy["version"] = json!(1);
    legacy.as_object_mut().unwrap().remove("credential_store");
    legacy.as_object_mut().unwrap().remove("profile_kind");
    let mut unsupported = member.clone();
    unsupported["version"] = json!(99);
    let mut mixed = member;
    mixed["token"] = json!("must-not-leak");
    for value in [legacy, unsupported, mixed] {
        neonmix_identity::files::replace(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
        let reply = request(
            &client,
            Request::Diagnostics {
                credential: "profiles/member.json".into(),
                hub: None,
            },
        )
        .await;
        assert!(!reply.ok);
        assert!(
            !serde_json::to_string(&reply)
                .unwrap()
                .contains("must-not-leak")
        );
        assert!(
            status(&client)
                .await
                .profiles
                .iter()
                .all(|p| p.credential != Path::new("profiles/member.json"))
        );
    }
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn structured_faults_take_priority_and_unknown_stderr_is_never_a_ui_message() {
    use neonmix_desktop_service::FaultCode;
    let fixture = Fixture::new();
    let binary = fixture.child();
    let daemon = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary.clone(),
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let cases = [
        (
            r#"{"fault":{"code":"hub_port_in_use","params":{"port":9000}},"error":"permission_denied"}"#,
            "worker_unavailable SECRET_TOKEN",
            FaultCode::HubPortInUse,
            Some(9000),
        ),
        (
            r#"{"fault":{"code":"hub_port_in_use","params":{"port":7443,"token":"SECRET_TOKEN"}},"error":"session_changed"}"#,
            "SECRET_TOKEN",
            FaultCode::SessionChanged,
            None,
        ),
        (
            r#"{"error":"revision_conflict"}"#,
            "worker_unavailable SECRET_TOKEN",
            FaultCode::RevisionConflict,
            None,
        ),
        (
            r#"{"error":"unknown_future_error"}"#,
            "SECRET_TOKEN private arbitrary failure",
            FaultCode::GenericFailure,
            None,
        ),
        (
            "{}",
            "address already in use SECRET_TOKEN",
            FaultCode::HubPortInUse,
            Some(7443),
        ),
        (
            "{}",
            "Error: credential_permission_denied SECRET_TOKEN",
            FaultCode::CredentialPermissionDenied,
            None,
        ),
        (
            r#"{"error":"network_uac_cancelled"}"#,
            "SECRET_TOKEN",
            FaultCode::NetworkUacCancelled,
            None,
        ),
        (
            r#"{"error":"runtime_cleanup_incomplete"}"#,
            "SECRET_TOKEN",
            FaultCode::RuntimeCleanupIncomplete,
            None,
        ),
    ];
    for (stdout, stderr, code, port) in cases {
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{stdout}'\nprintf '%s\\n' '{stderr}' >&2\nexit 1\n"
            ),
        )
        .unwrap();
        let reply = request(&client, Request::Devices).await;
        assert!(!reply.ok);
        let wire = serde_json::to_string(&reply).unwrap();
        assert!(!wire.contains("SECRET_TOKEN"));
        let fault = reply.fault.unwrap();
        assert_eq!(fault.code, code);
        assert_eq!(fault.params.port, port);
    }
    assert!(request(&client, Request::Shutdown).await.ok);
    daemon.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_stays_available_during_a_hung_ordinary_query_and_stop_interrupts_it() {
    use neonmix_desktop_service::FaultCode;
    let fixture = Fixture::new();
    let binary = fixture.child();
    std::fs::write(
        &binary,
        b"#!/bin/sh\nprintf '%s' \"$$\" > \"$0.query-pid\"\nexec /bin/sleep 30\n",
    )
    .unwrap();
    let daemon = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary.clone(),
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let query_client = client.clone();
    let query = tokio::spawn(async move { request(&query_client, Request::Devices).await });
    // This barrier proves that the ordinary child is actually hung; startup
    // under the parallel lifecycle suite is not the Status/Stop latency target.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !binary.with_extension("query-pid").exists() && tokio::time::Instant::now() < deadline {
        if query.is_finished() {
            let result = query.await.unwrap();
            let _ = request(&client, Request::Shutdown).await;
            daemon.await.unwrap().unwrap();
            panic!(
                "ordinary query ended before its startup barrier: {:?}",
                result.error
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !binary.with_extension("query-pid").exists() {
        let _ = request(&client, Request::Shutdown).await;
        daemon.await.unwrap().unwrap();
        let _ = query.await;
        panic!("ordinary child did not reach startup barrier within five seconds");
    }
    let busy = request(&client, Request::Status).await;
    assert!(busy.ok, "independent Status failed: {:?}", busy.error);
    assert!(request(&client, Request::HubStop).await.ok);
    assert_eq!(
        query.await.unwrap().fault.unwrap().code,
        FaultCode::RequestInterrupted
    );
    assert!(request(&client, Request::Shutdown).await.ok);
    daemon.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ready_followed_by_stats_is_immediately_ready_through_real_ipc() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"room","output":"physical"}),
    );
    let script = std::fs::read_to_string(&binary).unwrap().replace(
        "IFS= read -r stop;",
        "printf '{\"event\":\"hub_stats\"}\n'; IFS= read -r stop;",
    );
    std::fs::write(&binary, script).unwrap();
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let start = std::time::Instant::now();
    let reply = request(&client, Request::HubStart).await;
    assert!(reply.ok, "{:?}", reply.error);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "ready lost behind the ordinary log"
    );
    let active = status(&client).await;
    assert!(active.hub.ready && active.hub.running);
    assert_eq!(active.hub.last_event.unwrap()["event"], "hub_stats");
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_remote_mutation_and_full_ordinary_quota_do_not_block_status_or_stop() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"room","output":"physical"}),
    );
    fixture.write("hub/admin.json", json!({"secret_ref":"fixture-admin"}));
    let script = std::fs::read_to_string(&binary).unwrap().replace(
        "snapshot)",
        r#"control) printf '%s' "$$" > "$0.mutation-pid"; /bin/sleep 20; printf '{"event":"control_completed"}\n';;
snapshot)"#,
    );
    std::fs::write(&binary, script).unwrap();
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary.clone(),
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    assert!(request(&client, Request::HubStart).await.ok);
    let pid = status(&client).await.hub.pid.unwrap();
    let remote_client = client.clone();
    let mutation = tokio::spawn(async move {
        request(
            &remote_client,
            Request::Control {
                credential: "hub/admin.json".into(),
                hub: None,
                expected_revision: 1,
                operation: neonmix_control::Operation::OutputMix {
                    gain_db: Some(-9.),
                    muted: None,
                },
            },
        )
        .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !fixture.0.join("fixture-child.mutation-pid").exists() {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut saturated = Vec::new();
    for _ in 0..7 {
        let mut stream = UnixStream::connect(fixture.0.join("ipc.sock")).unwrap();
        stream.write_all(&128u32.to_be_bytes()).unwrap();
        stream.write_all(b"{").unwrap();
        saturated.push(stream);
    }
    let start = std::time::Instant::now();
    for _ in 0..20 {
        assert!(status(&client).await.hub.running);
    }
    let stopped = request(&client, Request::HubStop).await;
    assert!(stopped.ok, "{:?}", stopped.error);
    assert!(stopped.data["operation_id"].is_string());
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "lifecycle request waited on a remote write or ordinary quota"
    );
    assert!(!status(&client).await.hub.running);
    assert!(
        !mutation.is_finished(),
        "write was cancelled by a local media stop"
    );
    #[allow(unsafe_code)]
    // SAFETY: zero-signal checks the media PID supplied by this fixture daemon.
    unsafe {
        assert_eq!(libc::kill(pid as libc::pid_t, 0), -1);
    }
    drop(saturated);
    // A mutating CLI may have committed remotely; let its existing request end.
    let completed = mutation.await.unwrap();
    assert!(completed.ok);
    assert_eq!(completed.data["event"], "control_completed");
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
    assert!(!fixture.0.join("lifecycle.sock").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disconnected_stop_client_still_reaps_and_late_ready_cannot_restart_media() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"room","output":"physical"}),
    );
    let script = std::fs::read_to_string(&binary).unwrap();
    // The real stdin STOP is the barrier: Ready is emitted only after Stop.
    let script=script.replace("serve|send)","serve) printf '{\"event\":\"booting\"}\n'; IFS= read -r stop; printf '{\"event\":\"hub_started\"}\n'; exit 0;;\nunused-serve|send)");
    std::fs::write(&binary, script).unwrap();
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let starting_client = client.clone();
    let starting = tokio::spawn(async move { request(&starting_client, Request::HubStart).await });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let pid = loop {
        let current = status(&client).await;
        if let Some(pid) = current.hub.pid {
            assert!(!current.hub.ready);
            break pid;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let bytes = serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        request: Request::HubStop,
    })
    .unwrap();
    let mut abandoned = UnixStream::connect(fixture.0.join("lifecycle.sock")).unwrap();
    abandoned
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    abandoned.write_all(&bytes).unwrap();
    drop(abandoned);
    assert!(!starting.await.unwrap().ok);
    let stopped = status(&client).await;
    assert!(!stopped.hub.running && !stopped.hub.ready);
    #[allow(unsafe_code)]
    // SAFETY: checks only this fixture's already stopped child.
    unsafe {
        assert_eq!(libc::kill(pid as libc::pid_t, 0), -1);
    }
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn frozen_lifecycle_identity_and_stop_generation_reject_late_start_and_wrong_instance_stop() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"room","output":"physical"}),
    );
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let view = status(&client).await.lifecycle.unwrap();
    let old_start = Request::LifecycleStart {
        instance_generation: view.instance_generation,
        expected_stop_generation: view.hub_stop_generation,
        request: Box::new(Request::HubStart),
    };
    assert!(request(&client, old_start.clone()).await.ok);
    let stop = request(
        &client,
        Request::LifecycleStop {
            instance_generation: view.instance_generation,
            request: Box::new(Request::HubStop),
        },
    )
    .await;
    assert!(stop.ok);
    assert_eq!(stop.data["lifecycle"]["hub_stop_generation"], 1);
    let rejected = request(&client, old_start).await;
    assert!(!rejected.ok);
    assert_eq!(
        rejected.fault.unwrap().code,
        neonmix_desktop_service::FaultCode::RequestInterrupted
    );
    assert!(!status(&client).await.hub.running);
    let fresh = status(&client).await.lifecycle.unwrap();
    assert!(
        request(
            &client,
            Request::LifecycleStart {
                instance_generation: fresh.instance_generation,
                expected_stop_generation: fresh.hub_stop_generation,
                request: Box::new(Request::HubStart)
            }
        )
        .await
        .ok
    );
    let pid = status(&client).await.hub.pid;
    let old_owner = request(
        &client,
        Request::LifecycleStop {
            instance_generation: uuid::Uuid::new_v4(),
            request: Box::new(Request::HubStop),
        },
    )
    .await;
    assert!(!old_owner.ok);
    assert_eq!(
        status(&client).await.hub.pid,
        pid,
        "another instance's request stopped current media"
    );
    assert!(
        !request(
            &client,
            Request::LifecycleStop {
                instance_generation: view.instance_generation,
                request: Box::new(Request::Devices)
            }
        )
        .await
        .ok
    );
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ipc_settings_and_background_restart_use_the_same_recoverable_config_owner() {
    let fixture = Fixture::new();
    let binary = fixture.child();
    fixture.write(
        "hub/server.json",
        json!({"private_key_ref":"fixture-tls","room_name":"Old room","output":"old-output"}),
    );
    let authority =
        neonmix_control::Authority::new("old-output".into(), "Admin".into(), &"a".repeat(64))
            .unwrap();
    neonmix_identity::files::write_new(
        &fixture.0.join("hub/state.json"),
        &serde_json::to_vec_pretty(&authority.persistent()).unwrap(),
    )
    .unwrap();
    transport::protect_directory(&fixture.0.join("hub")).unwrap();
    let profile = fixture.0.join("hub/server.json");
    let validate = |bytes: &[u8]| {
        neonmix_control::Authority::restore(
            serde_json::from_slice(bytes).map_err(|_| "invalid state")?,
        )
        .map(|_| ())
        .map_err(|_| "invalid authority".into())
    };
    let mut reached = false;
    let interrupted = neonmix_identity::hub_settings::update_with(
        &profile,
        "recovered-output",
        "Recovered room",
        &validate,
        &mut |stage| {
            if stage == neonmix_identity::hub_settings::Stage::StatePublished {
                reached = true;
                Err(std::io::ErrorKind::Other.into())
            } else {
                Ok(())
            }
        },
    );
    assert!(
        interrupted.is_err() && reached,
        "transaction never reached state publication: {interrupted:?}"
    );
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        binary.clone(),
        binary,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let recovered = status(&client).await.hub_settings.unwrap();
    assert_eq!(recovered.output, "recovered-output");
    assert_eq!(recovered.name, "Recovered room");
    let revision = |path: &Path| {
        serde_json::from_slice::<serde_json::Value>(
            &neonmix_identity::files::read_private(path, 8 * 1024 * 1024).unwrap(),
        )
        .unwrap()["revision"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(revision(&fixture.0.join("hub/state.json")), 2);
    let saved = request(
        &client,
        Request::HubSettings {
            settings: HubSettings {
                output: "final-output".into(),
                name: "Final room".into(),
            },
        },
    )
    .await;
    assert!(saved.ok, "{:?}", saved.error);
    let current = status(&client).await.hub_settings.unwrap();
    assert_eq!(current.output, "final-output");
    assert_eq!(current.name, "Final room");
    assert_eq!(revision(&fixture.0.join("hub/state.json")), 3);
    assert!(
        request(
            &client,
            Request::HubSettings {
                settings: HubSettings {
                    output: current.output,
                    name: current.name
                }
            }
        )
        .await
        .ok
    );
    assert_eq!(
        revision(&fixture.0.join("hub/state.json")),
        3,
        "retry applied another business revision"
    );
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn output_ipc_forwards_frozen_id_and_native_sync_uses_the_acknowledged_object() {
    use neonmix_desktop_service::OutputAction;
    let fixture = Fixture::new();
    let _binary = fixture.child();
    let captured = uuid::Uuid::new_v4();
    let replacement = uuid::Uuid::new_v4();
    let helper = fixture.0.join("output-cas-helper.py");
    let source = r#"#!/usr/bin/env python3
import json,pathlib,sys
args=sys.argv[1:]
directory=pathlib.Path(args[args.index('--directory')+1]);current=json.loads((directory/'binding.json').read_text())
operation=args[1]
(directory/'calls.jsonl').open('a').write(json.dumps({'operation':operation,'arguments':args})+'\n')
if operation=='rename':
    if args[args.index('--expected-output-id')+1]!=current['output_id']:print('output_object_replaced',file=sys.stderr);sys.exit(1)
    acknowledged=dict(current);acknowledged['revision']+=1
    # Another client replaces the object before the follow-up native sync.
    (directory/'binding.json').write_text(json.dumps({'output_id':'REPLACEMENT','revision':acknowledged['revision'],'provider':'neonmix'}))
    print(json.dumps(acknowledged))
elif operation=='sync-name':
    if args[args.index('--expected-output-id')+1]!=current['output_id']:print('output_object_replaced',file=sys.stderr);sys.exit(1)
    print(json.dumps(current))
else:print(json.dumps(current))
"#;
    std::fs::write(
        &helper,
        source.replace("REPLACEMENT", &replacement.to_string()),
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    fixture.write(
        "output/binding.json",
        json!({"output_id":captured,"revision":1,"provider":"neonmix"}),
    );
    let server = tokio::spawn(daemon::serve_with_binaries(
        fixture.0.clone(),
        helper.clone(),
        helper,
    ));
    let client = Client::new(&fixture.0);
    wait(&client).await;
    let renamed = request(
        &client,
        Request::Output {
            directory: "output".into(),
            action: OutputAction::Rename {
                expected_output_id: captured,
                expected_revision: 1,
                name: "saved name".into(),
            },
        },
    )
    .await;
    assert!(renamed.ok, "{:?}", renamed.error);
    assert_eq!(renamed.data["output_id"], captured.to_string());
    assert_eq!(renamed.data["native_name_synced"], false);
    let calls = std::fs::read_to_string(fixture.0.join("output/calls.jsonl")).unwrap();
    let calls: Vec<serde_json::Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(calls.len(), 2);
    for (index, revision) in [(0, "1"), (1, "2")] {
        let args = calls[index]["arguments"].as_array().unwrap();
        let id = args
            .iter()
            .position(|item| item == "--expected-output-id")
            .unwrap();
        let rev = args
            .iter()
            .position(|item| item == "--expected-revision")
            .unwrap();
        assert_eq!(args[id + 1], captured.to_string());
        assert_eq!(args[rev + 1], revision);
    }
    let old = request(
        &client,
        Request::Output {
            directory: "output".into(),
            action: OutputAction::Rename {
                expected_output_id: captured,
                expected_revision: 1,
                name: "old command".into(),
            },
        },
    )
    .await;
    assert!(!old.ok);
    assert_eq!(
        old.fault.unwrap().code,
        neonmix_desktop_service::FaultCode::OutputObjectReplaced
    );
    let mut legacy = std::os::unix::net::UnixStream::connect(fixture.0.join("ipc.sock")).unwrap();
    let value=serde_json::to_vec(&json!({"version":1,"request":{"type":"output","directory":"output","action":{"action":"disable","expected_revision":1}}})).unwrap();
    legacy
        .write_all(&(value.len() as u32).to_be_bytes())
        .unwrap();
    legacy.write_all(&value).unwrap();
    let mut header = [0; 4];
    legacy.read_exact(&mut header).unwrap();
    let mut body = vec![0; u32::from_be_bytes(header) as usize];
    legacy.read_exact(&mut body).unwrap();
    let rejected: neonmix_desktop_service::Reply = serde_json::from_slice(&body).unwrap();
    assert!(!rejected.ok, "legacy request acquired current UUID");
    assert!(request(&client, Request::Shutdown).await.ok);
    server.await.unwrap().unwrap();
}
