use crate::AudioError;
use serde::{Deserialize, Serialize};

pub const INTERNAL_RATE: u32 = 48_000;
pub const MAX_BLOCK_FRAMES: usize = 480;
/// Holds a complete native callback up to the accepted 8192-frame period plus slack.
/// Time-based freshness remains independently limited to 100 ms by the consumer.
pub const CAPTURE_QUEUE_BLOCKS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelLayout {
    Mono,
    Stereo,
}
impl ChannelLayout {
    pub fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channel_layout: ChannelLayout,
}
impl AudioFormat {
    pub const INTERNAL: Self = Self {
        sample_rate: INTERNAL_RATE,
        channel_layout: ChannelLayout::Stereo,
    };
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, AudioError> {
        if ![44_100, 48_000, 96_000].contains(&sample_rate) {
            return Err(AudioError::UnsupportedFormat(format!(
                "sample rate {sample_rate}; supported: 44100, 48000, 96000"
            )));
        }
        let channel_layout = match channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => {
                return Err(AudioError::UnsupportedFormat(format!(
                    "{channels} channels; supported: mono, stereo"
                )));
            }
        };
        Ok(Self {
            sample_rate,
            channel_layout,
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discontinuity(pub u8);
impl Discontinuity {
    pub const NONE: Self = Self(0);
    pub const START: Self = Self(1);
    pub const GAP: Self = Self(2);
    pub const OVERFLOW: Self = Self(4);
    pub const FORMAT_CHANGED: Self = Self(8);
    pub const RESUMED: Self = Self(16);
    pub const CLOCK_RESET: Self = Self(32);
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BlockHeader {
    pub stream_id: u64,
    pub stream_epoch: u64,
    /// Frame index in this source's sample-rate domain, never interleaved sample index.
    pub source_sample_position: u64,
    pub format: AudioFormat,
    pub frame_count: u16,
    pub discontinuity_flags: Discontinuity,
    /// Native capture clock, local to this stream. Not comparable across machines.
    pub capture_timestamp_ns: u64,
    /// Process monotonic arrival time; used only for bounded-age queue eviction.
    pub arrival_ns: u64,
}

/// Fixed storage, no heap allocation or destructor in a callback. Unused tail is not audio.
#[derive(Clone, Copy)]
pub struct AudioBlock {
    pub header: BlockHeader,
    pub pcm: [[f32; 2]; MAX_BLOCK_FRAMES],
}
impl AudioBlock {
    pub fn empty(stream_id: u64, format: AudioFormat) -> Self {
        Self {
            header: BlockHeader {
                stream_id,
                stream_epoch: 0,
                source_sample_position: 0,
                format,
                frame_count: 0,
                discontinuity_flags: Discontinuity::START,
                capture_timestamp_ns: 0,
                arrival_ns: 0,
            },
            pcm: [[0.0; 2]; MAX_BLOCK_FRAMES],
        }
    }
    pub fn frames(&self) -> &[[f32; 2]] {
        &self.pcm[..usize::from(self.header.frame_count).min(MAX_BLOCK_FRAMES)]
    }
    pub fn is_silent(&self) -> bool {
        self.frames().iter().all(|f| f[0] == 0.0 && f[1] == 0.0)
    }
}
