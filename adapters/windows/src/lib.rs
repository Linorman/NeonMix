#![cfg(target_os = "windows")]
//! WASAPI shared-mode render + endpoint loopback. Virtual endpoint installation is E06.
use neonmix_core::AudioError;
use neonmix_io::NativeBackend;
pub fn backend() -> Result<NativeBackend, AudioError> {
    cpal::host_from_id(cpal::HostId::Wasapi)
        .map(|host| NativeBackend::new(host, "wasapi", true))
        .map_err(|error| AudioError::Backend(error.to_string()))
}
