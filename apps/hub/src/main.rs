mod binding;
mod connection;
mod control_client;
mod framer;
mod identity;
mod media_log;
mod media_wait;
mod media_worker;
mod pairing_api;
mod probe;
mod pump_timing;
mod qos;
mod recoverable;
mod sender;
mod server;
mod virtual_output;
use clap::{Parser, Subcommand};
use neonmix_control::Role;
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Parser)]
#[command(version, about = "NeonMix discovered and authenticated Sender/Hub")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Configure a Hub with native platform secrets and stable identity.
    Setup {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        output: String,
        #[arg(long)]
        name: String,
    },
    /// Stream untrusted DNS-SD candidates and removal events.
    Discover {
        #[arg(long, default_value_t = 5)]
        seconds: u32,
    },
    /// Create an out-of-band, single-use invitation (administrator only).
    Invite {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 120)]
        seconds: u32,
    },
    CancelInvite {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        invitation_id: uuid::Uuid,
    },
    Pair {
        #[arg(long)]
        invite: PathBuf,
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        name: String,
        /// Diagnostic address override; default uses DNS-SD and the invite's identity.
        #[arg(long)]
        hub: Option<String>,
    },
    Forget {
        #[arg(long)]
        credential: PathBuf,
    },
    VaultProbe,

    /// Explicit plaintext laboratory provisioning; product credentials use setup.
    Init {
        #[arg(long = "host")]
        hosts: Vec<String>,
        #[arg(long, default_value = ".local/hub-lab")]
        directory: PathBuf,
        #[arg(long)]
        output: String,
    },
    Serve {
        #[arg(long)]
        config: PathBuf,
        /// Product profiles share on 0.0.0.0:7443; laboratory profiles use loopback.
        #[arg(long)]
        listen: Option<SocketAddr>,
    },
    Send {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long, default_value_t = 10)]
        seconds: u32,
        #[arg(long)]
        capture: Option<String>,
        /// Capture NeonMix's stable virtual output instead of a manually supplied device.
        #[arg(long, conflicts_with = "capture")]
        virtual_output: bool,
        /// Explicit external provider for development; otherwise use NeonMix's device.
        #[arg(long, requires = "virtual_output")]
        virtual_output_provider: Option<virtual_output::Provider>,
        /// Use a persisted Hub/output/device binding and observe local revocation.
        #[arg(long, conflicts_with_all = ["capture", "virtual_output", "virtual_output_provider"])]
        output_binding: Option<PathBuf>,
        #[arg(long, default_value_t = 440.0)]
        frequency: f64,
    },
    Snapshot {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
    },
    /// Authenticated viewer identity and role, without any credential material.
    Me {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
    },
    Watch {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long, default_value_t = 30)]
        seconds: u32,
    },
    Control {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long)]
        command: PathBuf,
    },
    Runtime,
    /// Inspect the exact virtual-device binding without starting a media session.
    VirtualOutput {
        #[arg(long, default_value = "neonmix")]
        provider: virtual_output::Provider,
    },
    Output {
        #[command(subcommand)]
        command: OutputCommand,
    },
    Diagnostics {
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
    },
    Probe {
        #[arg(long, default_value_t = 5)]
        seconds: u32,
        #[arg(long, default_value_t = 2)]
        streams: usize,
        #[arg(long)]
        output: Option<String>,
        #[arg(long, default_value_t = 0)]
        drop_every: u32,
        #[arg(long)]
        wrong_fingerprint: bool,
        #[arg(long)]
        replay: bool,
    },
}
#[derive(Subcommand)]
enum OutputCommand {
    SyncName {
        #[arg(long)]
        directory: PathBuf,
    },
    Add {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        credential: PathBuf,
        #[arg(long)]
        hub: Option<String>,
        #[arg(long, default_value = "neonmix")]
        provider: virtual_output::Provider,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        name: String,
    },
    Show {
        #[arg(long)]
        directory: PathBuf,
    },
    Rename {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        name: String,
    },
    Enable {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    Disable {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    Remove {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Provisioned {
    pub name: String,
    pub role: Role,
    pub token: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub room_name: Option<String>,
    #[serde(default)]
    pub state_path: Option<PathBuf>,
    pub output: String,
    pub pem: String,
    pub certificate: String,
    pub private_key: String,
    pub devices: Vec<Provisioned>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    #[serde(skip)]
    pub route: Option<neonmix_identity::discovery::Endpoint>,
    #[serde(default)]
    pub hub_id: Option<uuid::Uuid>,
    pub token: String,
    pub certificate: String,
}
pub fn emit(value: impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}
pub fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn backend() -> std::result::Result<neonmix_io::NativeBackend, neonmix_core::AudioError> {
    #[cfg(target_os = "macos")]
    {
        neonmix_macos::backend()
    }
    #[cfg(target_os = "windows")]
    {
        neonmix_windows::backend()
    }
    #[cfg(target_os = "linux")]
    {
        neonmix_linux::backend()
    }
}
pub fn certificate() -> Result<(String, String, String)> {
    certificate_for_hosts(Vec::new())
}
fn certificate_for_hosts(mut hosts: Vec<String>) -> Result<(String, String, String)> {
    if hosts.len() > 16 {
        return Err("certificate permits at most 16 additional hostnames/addresses".into());
    }
    hosts.extend(["localhost".into(), "127.0.0.1".into(), "::1".into()]);
    hosts.sort();
    hosts.dedup();
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(hosts)?;
    let cert_pem = cert.pem();
    let key = signing_key.serialize_pem();
    Ok((format!("{cert_pem}{key}"), cert_pem, key))
}
fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(neonmix_identity::files::write_new(path, bytes)?)
}

fn init(directory: &Path, output: String, hosts: Vec<String>) -> Result<()> {
    if output.is_empty() {
        return Err("output ID is required".into());
    }
    std::fs::create_dir_all(directory)?;
    let (pem, certificate, private_key) = certificate_for_hosts(hosts)?;
    let devices: Vec<_> = [
        ("admin", Role::Admin),
        ("sender-a", Role::Member),
        ("sender-b", Role::Member),
    ]
    .into_iter()
    .map(|(name, role)| Provisioned {
        name: name.into(),
        role,
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    })
    .collect();
    let config = ServerConfig {
        room_name: None,
        state_path: Some(directory.canonicalize()?.join("state.json")),
        output,
        pem,
        certificate: certificate.clone(),
        private_key,
        devices: devices.clone(),
    };
    write_secret(
        &directory.join("server.json"),
        &serde_json::to_vec_pretty(&config)?,
    )?;
    for device in devices {
        write_secret(
            &directory.join(format!("{}.json", device.name)),
            &serde_json::to_vec_pretty(&Credential {
                hub_id: None,
                route: None,
                token: device.token,
                certificate: certificate.clone(),
            })?,
        )?;
    }
    emit(
        serde_json::json!({"directory":directory.canonicalize()?,"hub_certificate_sha256":neonmix_media::certificate_fingerprint(&certificate)?,"credentials":"explicit laboratory provisioning; E05 platform credential storage is separate"}),
    )
}
#[tokio::main]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let result: Result<()> = async {
        match Cli::parse().command {
            Command::Setup {directory,output,name} => identity::setup(&directory,output,name),
            Command::Discover {seconds} => {
                tokio::task::spawn_blocking(move || {
                    let candidates=neonmix_identity::discovery::browse(seconds,|event,candidate|emit(serde_json::json!({"event":event,"candidate":candidate,"trusted":false})))?;
                    emit(serde_json::json!({"event":"discovery_complete","candidates":candidates}))
                }).await?
            },
            Command::Invite {credential,hub,out,seconds}=>identity::invite(&credential,hub,&out,seconds).await,
            Command::CancelInvite {credential,hub,invitation_id}=>identity::cancel(&credential,hub,invitation_id).await,
            Command::Pair {invite,credential,name,hub}=>identity::pair(&invite,&credential,name,hub).await,
            Command::Forget {credential}=>identity::forget(&credential),
            Command::VaultProbe=>{neonmix_identity::vault::probe()?;emit(serde_json::json!({"event":"platform_vault_verified"}))},
            Command::Init {
                directory,
                output,
                hosts,
            } => init(&directory, output, hosts),
            Command::Serve { config, listen } => {
                let config=identity::config(&config)?;
                let listen=listen.unwrap_or_else(||SocketAddr::from((if config.room_name.is_some() {[0,0,0,0]} else {[127,0,0,1]},7443)));
                server::serve(config,listen).await
            },
            Command::Send {
                credential,
                hub,
                seconds,
                capture,
                virtual_output,
                virtual_output_provider,
                output_binding,
                frequency,
            } => {
                let mut c=identity::credential(&credential)?;
                let hub=identity::endpoint(&mut c,hub).await?;
                sender::run(
                    c,
                    &hub,
                    seconds,
                    sender::SendOptions {capture,virtual_output,provider:virtual_output_provider.unwrap_or(virtual_output::Provider::Neonmix),frequency,output_binding},
                )
                .await
            }
            Command::Snapshot { credential, hub } => {
                let mut c: Credential = identity::credential(&credential)?;
                let hub=identity::endpoint(&mut c,hub).await?;
                let client = sender::client(&c)?;
                let state:neonmix_control::Snapshot=client.get(format!("{hub}/v1/hub")).bearer_auth(&c.token).send().await?.error_for_status()?.json().await?;
                identity::check_hub(&c,state.hub_id)?;
                emit(state)
            }
            Command::Me { credential, hub } => {
                let mut c = identity::credential(&credential)?;
                let hub = identity::endpoint(&mut c, hub).await?;
                let value: serde_json::Value = sender::client(&c)?
                    .get(format!("{hub}/v1/me")).bearer_auth(&c.token)
                    .send().await?.error_for_status()?.json().await?;
                identity::check_hub(&c, serde_json::from_value(value["hub_id"].clone())?)?;
                emit(value)
            }
            Command::VirtualOutput { provider } => {
                emit(virtual_output::resolve(provider, backend()?.devices()?)?)
            }
            Command::Output {command}=>binding::execute(command).await,
            Command::Diagnostics { credential, hub } => {
                let mut c: Credential = identity::credential(&credential)?;
                let hub=identity::endpoint(&mut c,hub).await?;
                let client = sender::client(&c)?;
                emit(
                    client
                        .get(format!("{hub}/v1/diagnostics"))
                        .bearer_auth(&c.token)
                        .send()
                        .await?
                        .error_for_status()?
                        .json::<serde_json::Value>()
                        .await?,
                )
            }
            Command::Watch {credential,hub,seconds}=>{
                if !(1..=86400).contains(&seconds) {return Err("seconds must be 1..86400".into());}
                let mut c=identity::credential(&credential)?;
                let hub=identity::endpoint(&mut c,hub).await?;
                let mut subscription=control_client::subscribe(c,hub)?;
                let until=tokio::time::Instant::now()+std::time::Duration::from_secs(u64::from(seconds));
                loop {tokio::select!{
                    _=tokio::time::sleep_until(until)=>break,
                    change=subscription.view.changed()=>{
                        if change.is_err() {break;}
                        let view=subscription.view.borrow_and_update().clone();
                        emit(serde_json::json!({"event":"control_view","connected":view.connected,"snapshots":view.snapshots,"subscriptions":view.subscriptions,"applied_events":view.applied_events,"rejected":view.rejected,"state":view.state.as_deref()}))?;
                        if view.rejected {return Err("control authorization revoked".into());}
                    }
                }}
                Ok(())
            }
            Command::Control {
                credential,
                hub,
                command,
            } => {
                let mut c: Credential = identity::credential(&credential)?;
                let hub=identity::endpoint(&mut c,hub).await?;
                let client = sender::client(&c)?;
                let body: neonmix_control::Command = read(&command)?;
                let response = client
                    .post(format!("{hub}/v1/commands"))
                    .bearer_auth(&c.token)
                    .json(&body)
                    .send()
                    .await?;
                let status = response.status();
                emit(response.json::<serde_json::Value>().await?)?;
                if status.is_success() {
                    Ok(())
                } else {
                    Err(format!("control rejected: {status}").into())
                }
            }
            Command::Runtime => emit(neonmix_media::runtime_probe()?),
            Command::Probe {
                seconds,
                streams,
                output,
                drop_every,
                wrong_fingerprint,
                replay,
            } => probe::run(
                seconds,
                streams,
                output,
                drop_every,
                wrong_fingerprint,
                replay,
            ),
        }
    }
    .await;
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
