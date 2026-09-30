#![cfg(target_os = "macos")]
//! Core Audio HAL I/O. Capture opens the readable side of an installed virtual device by UID.
use neonmix_core::AudioError;
use neonmix_io::NativeBackend;
mod name;
pub use name::set_virtual_output_name;
/// Stable Core Audio UID exposed by the NeonMix HAL plug-in.
pub const VIRTUAL_OUTPUT_UID: &str = "com.neonmix.audio.virtual-output";
pub fn backend() -> Result<NativeBackend, AudioError> {
    cpal::host_from_id(cpal::HostId::CoreAudio)
        .map(|host| {
            NativeBackend::new(host, "core_audio", false)
                .with_capture_preflight(ensure_capture_permission)
        })
        .map_err(|error| AudioError::Backend(error.to_string()))
}

#[allow(unsafe_code)]
fn ensure_capture_permission() -> Result<(), AudioError> {
    use objc2_avf_audio::{AVAudioApplication, AVAudioApplicationRecordPermission as Permission};
    // SAFETY: AVAudioApplication is available since macOS 14, below our 14.6 target.
    // These thread-safe getters neither request permission nor open any input device.
    let status = unsafe { AVAudioApplication::sharedInstance().recordPermission() };
    if status == Permission::Granted {
        return Ok(());
    }
    let state = if status == Permission::Denied {
        "denied"
    } else if status == Permission::Undetermined {
        "not yet granted"
    } else {
        "unavailable"
    };
    Err(AudioError::PermissionDenied(format!(
        "macOS Microphone access is {state}. Enable it in System Settings > Privacy & Security > Microphone for the app or terminal launching NeonMix. Virtual inputs such as BlackHole require this permission too."
    )))
}
