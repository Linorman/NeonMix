#![cfg(windows)]
//! Native named-pipe acceptance without media SDKs or ambient audio devices.
use neonmix_desktop_service::{Client, Request, ServiceStatus, daemon};
use std::{path::PathBuf, time::Duration};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn request(client: &Client, command: Request) -> neonmix_desktop_service::Reply {
    let client = client.clone();
    tokio::task::spawn_blocking(move || client.request(&command).unwrap())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_user_pipe_rejects_duplicate_owner_and_reopens_after_shutdown() {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let fixture = Fixture(
        project
            .join(".local/tmp")
            .join(format!("windows-ipc-{}", uuid::Uuid::new_v4())),
    );
    let client = Client::new(&fixture.0);
    for generation in 0..25 {
        let server = tokio::spawn(daemon::serve_with_binaries(
            fixture.0.clone(),
            fixture.0.join("unused-hub.exe"),
            fixture.0.join("unused-audio.exe"),
        ));
        for attempt in 0..100 {
            let probe = client.clone();
            if tokio::task::spawn_blocking(move || probe.request(&Request::Status).is_ok())
                .await
                .unwrap()
            {
                break;
            }
            if server.is_finished() {
                panic!(
                    "native IPC generation {generation} stopped before binding: {:?}",
                    server.await.unwrap()
                );
            }
            assert!(attempt < 99, "native IPC server did not bind");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let reply = request(&client, Request::Status).await;
        assert!(reply.ok);
        let status: ServiceStatus = serde_json::from_value(reply.data).unwrap();
        assert_eq!(status.pid, std::process::id());
        assert!(!status.hub.running && !status.sender.running);
        assert!(
            daemon::serve_with_binaries(
                fixture.0.clone(),
                fixture.0.join("unused-hub.exe"),
                fixture.0.join("unused-audio.exe"),
            )
            .await
            .is_err()
        );
        assert!(request(&client, Request::Status).await.ok);
        assert!(request(&client, Request::Shutdown).await.ok);
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
