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
        let path = self.0.join("fixture-child");
        // Exec replaces the shell: the fixture has no grandchildren to leak.
        std::fs::write(&path,br#"#!/bin/sh
case "$1" in
serve|send) printf '{"event":"media_started","token":"SECRET_TOKEN"}\n'; exec /bin/sleep 60;;
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
    assert!(client.request(&Request::Status).is_err());
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
    fixture.write("output/binding.json", json!({"revision":5,"enabled":true}));
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
