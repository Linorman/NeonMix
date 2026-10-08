//! Private fixture executable; never included in the installation package.
use clap::Parser;
use neonmix_identity::{files, hub_settings, profiles};
use std::{io::Read, path::PathBuf};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    directory: PathBuf,
    #[arg(long)]
    init: bool,
    #[arg(long)]
    recover: bool,
    #[arg(long)]
    stop_at: Option<u32>,
}
fn validate(bytes: &[u8]) -> Result<(), String> {
    neonmix_control::Authority::restore(serde_json::from_slice(bytes).map_err(|_| "invalid state")?)
        .map(|_| ())
        .map_err(|_| "invalid authority".into())
}
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let profile = args.directory.join("server.json");
    if args.init {
        files::private_dir(&args.directory)?;
        let authority =
            neonmix_control::Authority::new("old-output".into(), "Admin".into(), &"a".repeat(64))?;
        files::write_new(
            &args.directory.join("state.json"),
            &serde_json::to_vec_pretty(&authority.persistent())?,
        )?;
        let metadata = profiles::HubProfile {
            version: 2,
            credential_store: profiles::CredentialStore::File,
            room_name: "Old room".into(),
            state_path: "state.json".into(),
            output: "old-output".into(),
            certificate: "public fixture".into(),
            private_key_ref: uuid::Uuid::new_v4().to_string(),
            admin_token_ref: uuid::Uuid::new_v4().to_string(),
        };
        files::write_new(&profile, &serde_json::to_vec(&metadata)?)?;
        return Ok(());
    }
    if args.recover {
        hub_settings::recover(&profile, validate)?;
        let metadata = profiles::hub(&profile)?;
        let state = files::read_private(
            &args.directory.join("state.json"),
            profiles::MAX_STATE_BYTES,
        )?;
        validate(&state)?;
        let saved: serde_json::Value = serde_json::from_slice(&state)?;
        if saved["output"]["id"] != metadata.output {
            return Err("cross_file_mismatch".into());
        }
        println!(
            "{}",
            serde_json::json!({"event":"recovered","revision":saved["revision"],"new":metadata.output=="new-output"})
        );
        return Ok(());
    }
    let mut count = 0;
    let receipt = hub_settings::update_with(
        &profile,
        "new-output",
        "New room",
        &validate,
        &mut |stage| {
            count += 1;
            if args.stop_at == Some(count) {
                println!(
                    "{}",
                    serde_json::json!({"event":"barrier","index":count,"stage":format!("{stage:?}")})
                );
                use std::io::Write;
                std::io::stdout().flush()?;
                let mut byte = [0];
                let _ = std::io::stdin().read_exact(&mut byte);
            }
            Ok(())
        },
    )?;
    println!(
        "{}",
        serde_json::json!({"event":"completed","boundaries":count,"revision":receipt.revision})
    );
    Ok(())
}
