use crate::AudioFormat;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_type: String,
    pub period_min_frames: Option<u32>,
    pub period_max_frames: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub backend: String,
    pub input: Option<FormatInfo>,
    pub output: Option<FormatInfo>,
    pub supported_input: Vec<FormatInfo>,
    pub supported_output: Vec<FormatInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DeviceEvent {
    Added { device: DeviceInfo },
    Removed { device_id: String },
    Changed { device: DeviceInfo },
}
pub fn device_changes(previous: &[DeviceInfo], current: &[DeviceInfo]) -> Vec<DeviceEvent> {
    let mut events = Vec::new();
    for old in previous {
        if !current.iter().any(|d| d.id == old.id) {
            events.push(DeviceEvent::Removed {
                device_id: old.id.clone(),
            });
        }
    }
    for new in current {
        match previous.iter().find(|d| d.id == new.id) {
            None => events.push(DeviceEvent::Added {
                device: new.clone(),
            }),
            Some(old) if old != new => events.push(DeviceEvent::Changed {
                device: new.clone(),
            }),
            _ => {}
        }
    }
    events
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub device_id: String,
    pub backend: String,
    pub format: AudioFormat,
    pub sample_type: String,
    pub requested_period_frames: Option<u32>,
    pub actual_period_frames: Option<u32>,
    pub stream_epoch: u64,
    pub resampler_delay_frames: usize,
}
