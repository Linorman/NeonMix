//! Explicit offline migration/rollback utility. Never starts reception.
use neonmix_identity::{Result, airplay_profile, files, profiles};
use std::path::{Component, Path, PathBuf};

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: neonmix-airplay-profile migrate HUB_PROFILE LEGACY_NAME | export-v1 HUB_PROFILE OUTPUT_FILENAME";
    if args.is_empty() || matches!(args.as_slice(), [flag] if flag == "--help" || flag == "-h") {
        println!("{usage}");
        return Ok(());
    }
    if args.len() != 3 || !matches!(args[0].as_str(), "migrate" | "export-v1") {
        return Err(usage.into());
    }
    let hub_path = Path::new(&args[1]);
    let hub = profiles::hub(hub_path)?;
    let state_path = profiles::state_path(hub_path, &hub)?;
    // Exactly the same stable owner lock used by the Hub. An active Hub fails
    // here before any receiver data or credential is modified.
    let _owner = profiles::operation_lock(&state_path.with_extension("lock"))?;
    let bytes = files::read_private(&state_path, profiles::MAX_STATE_BYTES)?;
    let state: serde_json::Value = serde_json::from_slice(&bytes)?;
    let hub_id: uuid::Uuid =
        serde_json::from_value(state.get("hub_id").ok_or("credential_corrupt")?.clone())?;
    let parent = hub_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let profile_path = parent.join("airplay/receiver.json");
    match args[0].as_str() {
        "migrate" => {
            let profile = airplay_profile::migrate_v1(&profile_path, hub_id, &args[2])?;
            println!(
                "{}",
                serde_json::json!({"event":"airplay_profile_migrated","version":2,"receivers":profile.receivers.len()})
            );
        }
        "export-v1" => {
            let output = PathBuf::from(&args[2]);
            if output.components().count() != 1
                || !matches!(output.components().next(), Some(Component::Normal(_)))
            {
                return Err("output_must_be_filename_in_receiver_directory".into());
            }
            let profile = airplay_profile::load(&profile_path)?;
            airplay_profile::validate_secrets(&profile_path, &profile)?;
            let receiver = &profile.receivers[0];
            if receiver.receiver_uuid != hub_id {
                return Err("legacy_hub_identity_mismatch".into());
            }
            let legacy = airplay_profile::export_v1(&profile, receiver.receiver_id)?;
            let destination = profile_path.parent().unwrap().join(output);
            files::write_new(&destination, &serde_json::to_vec_pretty(&legacy)?)?;
            println!(
                "{}",
                serde_json::json!({"event":"airplay_v1_exported","file":destination,"receiver_uuid":receiver.receiver_uuid,"device_id":receiver.device_id,"name":receiver.name,"original_v2_preserved":true})
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
