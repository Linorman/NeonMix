//! Stream abstraction.
//!
//! A *stream* is one direction of audio flow on a device — an
//! AudioServerPlugin device owns one input stream, one output
//! stream, or both. Each stream carries its own [`StreamFormat`]
//! and occupies a contiguous range of channels within the device.
//!
//! This module is cross-platform plain data; the FFI that surfaces
//! a [`StreamSpec`] to the HAL as an `kAudioStreamClassID` object
//! lands in the `raw` module in a later PR.

use crate::format::StreamFormat;
use crate::property::PropertyScope;

/// The direction a stream carries audio.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum StreamDirection {
    /// A capture stream — audio flows from the device into the
    /// system (a virtual microphone).
    Input,
    /// A playback stream — audio flows from the system into the
    /// device (a virtual speaker).
    Output,
}

impl StreamDirection {
    /// The `kAudioStreamPropertyDirection` value the HAL expects:
    /// `1` for input, `0` for output.
    #[inline]
    #[must_use]
    pub const fn as_property_value(self) -> u32 {
        match self {
            Self::Input => 1,
            Self::Output => 0,
        }
    }

    /// Reconstruct a direction from a `kAudioStreamPropertyDirection`
    /// value. Any non-zero value is treated as input, matching the
    /// HAL's boolean interpretation.
    #[inline]
    #[must_use]
    pub const fn from_property_value(value: u32) -> Self {
        if value == 0 {
            Self::Output
        } else {
            Self::Input
        }
    }

    /// The [`PropertyScope`] a property access on this stream's
    /// direction is keyed to.
    #[inline]
    #[must_use]
    pub const fn scope(self) -> PropertyScope {
        match self {
            Self::Input => PropertyScope::INPUT,
            Self::Output => PropertyScope::OUTPUT,
        }
    }

    /// The opposite direction.
    #[inline]
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Input => Self::Output,
            Self::Output => Self::Input,
        }
    }
}

/// A declarative description of one stream on a device.
///
/// The framework turns a [`StreamSpec`] into an
/// `kAudioStreamClassID` audio object, answers the HAL's property
/// queries about it (direction, starting channel, virtual format),
/// and routes the stream's samples into
/// [`Driver::process_io`](crate::Driver::process_io). Build one with
/// [`StreamSpec::input`] / [`StreamSpec::output`] and the
/// builder-style setters.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct StreamSpec {
    direction: StreamDirection,
    starting_channel: u32,
    format: StreamFormat,
}

impl StreamSpec {
    /// An input stream carrying `format`, starting at channel `1`.
    #[inline]
    #[must_use]
    pub const fn input(format: StreamFormat) -> Self {
        Self {
            direction: StreamDirection::Input,
            starting_channel: 1,
            format,
        }
    }

    /// An output stream carrying `format`, starting at channel `1`.
    #[inline]
    #[must_use]
    pub const fn output(format: StreamFormat) -> Self {
        Self {
            direction: StreamDirection::Output,
            starting_channel: 1,
            format,
        }
    }

    /// Builder-style: set the 1-based channel this stream's first
    /// channel maps to within the owning device.
    #[inline]
    #[must_use]
    pub const fn with_starting_channel(mut self, channel: u32) -> Self {
        self.starting_channel = channel;
        self
    }

    /// Builder-style: replace the stream format.
    #[inline]
    #[must_use]
    pub const fn with_format(mut self, format: StreamFormat) -> Self {
        self.format = format;
        self
    }

    /// The direction this stream carries audio.
    #[inline]
    #[must_use]
    pub const fn direction(&self) -> StreamDirection {
        self.direction
    }

    /// The 1-based starting channel
    /// (`kAudioStreamPropertyStartingChannel`).
    #[inline]
    #[must_use]
    pub const fn starting_channel(&self) -> u32 {
        self.starting_channel
    }

    /// The stream's [`StreamFormat`]
    /// (`kAudioStreamPropertyVirtualFormat`).
    #[inline]
    #[must_use]
    pub const fn format(&self) -> StreamFormat {
        self.format
    }

    /// The stream's channel count, taken from its format.
    #[inline]
    #[must_use]
    pub const fn channels(&self) -> u32 {
        self.format.channels()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_property_values_match_core_audio() {
        assert_eq!(StreamDirection::Input.as_property_value(), 1);
        assert_eq!(StreamDirection::Output.as_property_value(), 0);
    }

    #[test]
    fn direction_round_trips_through_property_value() {
        assert_eq!(
            StreamDirection::from_property_value(1),
            StreamDirection::Input
        );
        assert_eq!(
            StreamDirection::from_property_value(0),
            StreamDirection::Output
        );
        // Any non-zero value reads back as input.
        assert_eq!(
            StreamDirection::from_property_value(99),
            StreamDirection::Input
        );
    }

    #[test]
    fn direction_scopes_and_opposites() {
        assert_eq!(StreamDirection::Input.scope(), PropertyScope::INPUT);
        assert_eq!(StreamDirection::Output.scope(), PropertyScope::OUTPUT);
        assert_eq!(StreamDirection::Input.opposite(), StreamDirection::Output);
        assert_eq!(StreamDirection::Output.opposite(), StreamDirection::Input);
    }

    #[test]
    fn input_stream_defaults() {
        let s = StreamSpec::input(StreamFormat::float32(48_000.0, 2));
        assert_eq!(s.direction(), StreamDirection::Input);
        assert_eq!(s.starting_channel(), 1);
        assert_eq!(s.channels(), 2);
        assert!(s.format().is_canonical());
    }

    #[test]
    fn output_stream_defaults() {
        let s = StreamSpec::output(StreamFormat::float32(44_100.0, 1));
        assert_eq!(s.direction(), StreamDirection::Output);
        assert_eq!(s.starting_channel(), 1);
        assert_eq!(s.channels(), 1);
    }

    #[test]
    fn builder_setters_apply() {
        let s = StreamSpec::output(StreamFormat::float32(48_000.0, 2))
            .with_starting_channel(3)
            .with_format(StreamFormat::float32(96_000.0, 4));
        assert_eq!(s.starting_channel(), 3);
        assert_eq!(s.format().sample_rate(), 96_000.0);
        assert_eq!(s.channels(), 4);
    }
}
