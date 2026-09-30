//! Explicit virtual-device bindings; provider selection never falls back to a default input.
use neonmix_core::{AudioError, DeviceInfo};
use serde::Serialize;

pub use neonmix_output_binding::Provider;

#[derive(Serialize)]
pub struct Binding {
    pub provider: Provider,
    pub external_driver: bool,
    pub device: DeviceInfo,
}

pub fn resolve(provider: Provider, devices: Vec<DeviceInfo>) -> Result<Binding, AudioError> {
    #[cfg(target_os = "macos")]
    let expected = match provider {
        Provider::Neonmix => "coreaudio:com.neonmix.audio.virtual-output",
        Provider::Blackhole => "coreaudio:BlackHole2ch_UID",
    };
    #[cfg(target_os = "linux")]
    let expected = match provider {
        Provider::Neonmix => "pipewire:neonmix.sink.default",
        Provider::Blackhole => return Err(AudioError::Backend("BlackHole requires macOS".into())),
    };
    #[cfg(target_os = "windows")]
    {
        if provider != Provider::Neonmix {
            return Err(AudioError::Backend("BlackHole requires macOS".into()));
        }
        let mut owned = devices
            .into_iter()
            .filter(|device| neonmix_windows::is_virtual_output(&device.id));
        let device = owned.next().ok_or_else(|| {
            AudioError::DeviceUnavailable("installed NeonMix render endpoint".into())
        })?;
        if owned.next().is_some() {
            return Err(AudioError::Backend(
                "multiple NeonMix render endpoints found; select an exact --device identity".into(),
            ));
        }
        resolve_id(provider, &device.id.clone(), vec![device])
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    resolve_id(provider, expected, devices)
}

pub fn resolve_id(
    provider: Provider,
    expected: &str,
    devices: Vec<DeviceInfo>,
) -> Result<Binding, AudioError> {
    #[cfg(target_os = "macos")]
    let valid = match provider {
        Provider::Neonmix => expected == "coreaudio:com.neonmix.audio.virtual-output",
        Provider::Blackhole => expected == "coreaudio:BlackHole2ch_UID",
    };
    #[cfg(target_os = "linux")]
    let valid = provider == Provider::Neonmix && expected == "pipewire:neonmix.sink.default";
    #[cfg(target_os = "windows")]
    let valid = provider == Provider::Neonmix && neonmix_windows::is_virtual_output(expected);
    if !valid {
        return Err(AudioError::Backend(
            "binding provider or device ID does not match this platform".into(),
        ));
    }
    {
        let mut matching = devices.into_iter().filter(|device| device.id == expected);
        let device = matching
            .next()
            .ok_or_else(|| AudioError::DeviceUnavailable(expected.into()))?;
        if matching.next().is_some() {
            return Err(AudioError::Backend(
                "duplicate stable virtual device identity; refusing ambiguous capture".into(),
            ));
        }
        let capture_available = {
            #[cfg(target_os = "windows")]
            {
                true
            } // WASAPI reads the render side through loopback.
            #[cfg(not(target_os = "windows"))]
            {
                device
                    .input
                    .as_ref()
                    .is_some_and(|format| format.channels == 2)
            }
        };
        if !capture_available
            || device
                .output
                .as_ref()
                .is_none_or(|format| format.channels != 2)
        {
            return Err(AudioError::Backend(
                "virtual output must have stereo render and capture sides".into(),
            ));
        }
        Ok(Binding {
            provider,
            external_driver: provider == Provider::Blackhole,
            device,
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use neonmix_core::FormatInfo;

    fn device(id: &str) -> DeviceInfo {
        let format = FormatInfo {
            sample_rate: 48000,
            channels: 2,
            sample_type: "f32".into(),
            period_min_frames: None,
            period_max_frames: None,
        };
        DeviceInfo {
            id: id.into(),
            name: "BlackHole 2ch".into(),
            backend: "core_audio".into(),
            input: Some(format.clone()),
            output: Some(format),
            supported_input: vec![],
            supported_output: vec![],
        }
    }

    #[test]
    fn binding_requires_exact_provider_uid_and_both_sides() {
        let id = "coreaudio:BlackHole2ch_UID";
        assert!(
            resolve(
                Provider::Blackhole,
                vec![device(&format!("coreaudio:other:{id}"))]
            )
            .is_err()
        );
        assert!(resolve(Provider::Neonmix, vec![device(id)]).is_err());
        assert!(resolve(Provider::Blackhole, vec![device(id), device(id)]).is_err());
        let mut input_only = device(id);
        input_only.output = None;
        assert!(resolve(Provider::Blackhole, vec![input_only]).is_err());
        let selected = resolve(Provider::Blackhole, vec![device(id)]).unwrap();
        assert_eq!(selected.device.id, id);
        assert!(selected.external_driver);
    }
}
