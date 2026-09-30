//! Device abstraction.
//!
//! A *device* is the audio endpoint an AudioServerPlugin exposes to
//! the system — what shows up in System Settings ▸ Sound. A device
//! owns one or more [`StreamSpec`]s (its input and/or output) and
//! carries the metadata the HAL surfaces through the device
//! property protocol: the UID, the human-readable name, the nominal
//! sample rate.
//!
//! This module is cross-platform plain data. A [`DeviceSpec`] is the
//! declarative description a [`Driver`](crate::Driver) hands the
//! framework; the framework assigns it a [`DeviceId`] and answers
//! the HAL's property queries against it.

use crate::object::AudioObjectId;
use crate::stream::{StreamDirection, StreamSpec};

/// A device's identifier within the plug-in's object tree.
///
/// A thin newtype over an [`AudioObjectId`]. The framework mints one
/// per device when it builds the object tree from the driver's
/// [`DeviceSpec`]s.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct DeviceId(pub AudioObjectId);

impl DeviceId {
    /// Wrap a raw `AudioObjectID`.
    #[inline]
    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        Self(AudioObjectId::from_u32(value))
    }

    /// The underlying [`AudioObjectId`].
    #[inline]
    #[must_use]
    pub const fn object_id(self) -> AudioObjectId {
        self.0
    }

    /// The raw `u32`, ready for the FFI boundary.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0.as_u32()
    }
}

impl From<AudioObjectId> for DeviceId {
    #[inline]
    fn from(value: AudioObjectId) -> Self {
        Self(value)
    }
}

/// A declarative description of one device a driver exposes.
///
/// The framework turns a `DeviceSpec` into a `kAudioDeviceClassID`
/// audio object, answers the HAL's property queries about it (UID,
/// name, sample rate, stream list), and routes the device's IO into
/// [`Driver::process_io`](crate::Driver::process_io).
///
/// Build one with [`DeviceSpec::new`] and the builder-style stream
/// setters. A device with only an output stream is a virtual
/// speaker; with only an input stream, a virtual microphone; with
/// both, a loopback device.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct DeviceSpec {
    uid: &'static str,
    name: &'static str,
    manufacturer: &'static str,
    sample_rate: f64,
    input: Option<StreamSpec>,
    output: Option<StreamSpec>,
}

impl DeviceSpec {
    /// Begin a device description with the mandatory identity
    /// fields. The device starts with no streams; add them with
    /// [`Self::with_input`] / [`Self::with_output`].
    ///
    /// - `uid` is the stable, globally-unique device identifier
    ///   (`kAudioDevicePropertyDeviceUID`). It must not change
    ///   across launches — the system keeps per-device settings
    ///   keyed on it.
    /// - `name` is the human-readable name shown in the Sound
    ///   settings UI (`kAudioObjectPropertyName`).
    /// - `manufacturer` is shown alongside the name
    ///   (`kAudioObjectPropertyManufacturer`).
    #[inline]
    #[must_use]
    pub const fn new(uid: &'static str, name: &'static str, manufacturer: &'static str) -> Self {
        Self {
            uid,
            name,
            manufacturer,
            sample_rate: 48_000.0,
            input: None,
            output: None,
        }
    }

    /// Builder-style: set the nominal sample rate
    /// (`kAudioDevicePropertyNominalSampleRate`). Defaults to
    /// 48 000 Hz.
    #[inline]
    #[must_use]
    pub const fn with_sample_rate(mut self, sample_rate: f64) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Builder-style: attach an input stream. Replaces any
    /// previously-set input stream.
    #[inline]
    #[must_use]
    pub const fn with_input(mut self, stream: StreamSpec) -> Self {
        self.input = Some(stream);
        self
    }

    /// Builder-style: attach an output stream. Replaces any
    /// previously-set output stream.
    #[inline]
    #[must_use]
    pub const fn with_output(mut self, stream: StreamSpec) -> Self {
        self.output = Some(stream);
        self
    }

    /// The stable device UID.
    #[inline]
    #[must_use]
    pub const fn uid(&self) -> &'static str {
        self.uid
    }

    /// The human-readable device name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The manufacturer string.
    #[inline]
    #[must_use]
    pub const fn manufacturer(&self) -> &'static str {
        self.manufacturer
    }

    /// The nominal sample rate, in hertz.
    #[inline]
    #[must_use]
    pub const fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// The input stream, if this device has one.
    #[inline]
    #[must_use]
    pub const fn input(&self) -> Option<StreamSpec> {
        self.input
    }

    /// The output stream, if this device has one.
    #[inline]
    #[must_use]
    pub const fn output(&self) -> Option<StreamSpec> {
        self.output
    }

    /// The stream for `direction`, if this device has one.
    #[inline]
    #[must_use]
    pub const fn stream(&self, direction: StreamDirection) -> Option<StreamSpec> {
        match direction {
            StreamDirection::Input => self.input,
            StreamDirection::Output => self.output,
        }
    }

    /// Number of streams on this device (`0`, `1`, or `2`).
    #[inline]
    #[must_use]
    pub const fn stream_count(&self) -> usize {
        self.input.is_some() as usize + self.output.is_some() as usize
    }

    /// `true` iff this device has both an input and an output
    /// stream — i.e. it is a loopback device.
    #[inline]
    #[must_use]
    pub const fn is_loopback(&self) -> bool {
        self.input.is_some() && self.output.is_some()
    }
}

/// A device the framework has admitted into the object tree.
///
/// Pairs the [`DeviceId`] the framework assigned with the
/// [`DeviceSpec`] the driver declared. The framework constructs
/// these; drivers describe their devices with [`DeviceSpec`] and
/// never build a `Device` directly.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Device {
    id: DeviceId,
    spec: DeviceSpec,
}

impl Device {
    /// Pair an id with a spec. Called by the framework when it
    /// builds the object tree.
    #[inline]
    #[must_use]
    pub const fn new(id: DeviceId, spec: DeviceSpec) -> Self {
        Self { id, spec }
    }

    /// The framework-assigned device id.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> DeviceId {
        self.id
    }

    /// The declarative spec the driver supplied.
    #[inline]
    #[must_use]
    pub const fn spec(&self) -> &DeviceSpec {
        &self.spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::StreamFormat;

    fn loopback_spec() -> DeviceSpec {
        DeviceSpec::new("com.example.loopback", "Example Loopback", "tympan-aspl")
            .with_sample_rate(48_000.0)
            .with_input(StreamSpec::input(StreamFormat::float32(48_000.0, 2)))
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 2)))
    }

    #[test]
    fn device_id_round_trips() {
        let id = DeviceId::from_u32(7);
        assert_eq!(id.as_u32(), 7);
        assert_eq!(id.object_id(), AudioObjectId::from_u32(7));
        assert_eq!(DeviceId::from(AudioObjectId::from_u32(7)), id);
    }

    #[test]
    fn new_device_starts_streamless_at_48k() {
        let spec = DeviceSpec::new("uid", "name", "maker");
        assert_eq!(spec.sample_rate(), 48_000.0);
        assert_eq!(spec.stream_count(), 0);
        assert!(spec.input().is_none());
        assert!(spec.output().is_none());
        assert!(!spec.is_loopback());
    }

    #[test]
    fn identity_fields_round_trip() {
        let spec = loopback_spec();
        assert_eq!(spec.uid(), "com.example.loopback");
        assert_eq!(spec.name(), "Example Loopback");
        assert_eq!(spec.manufacturer(), "tympan-aspl");
    }

    #[test]
    fn loopback_has_both_streams() {
        let spec = loopback_spec();
        assert_eq!(spec.stream_count(), 2);
        assert!(spec.is_loopback());
        assert_eq!(
            spec.stream(StreamDirection::Input).unwrap().direction(),
            StreamDirection::Input
        );
        assert_eq!(
            spec.stream(StreamDirection::Output).unwrap().direction(),
            StreamDirection::Output
        );
    }

    #[test]
    fn output_only_device_is_not_loopback() {
        let spec = DeviceSpec::new("uid", "Speaker", "maker")
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 2)));
        assert_eq!(spec.stream_count(), 1);
        assert!(!spec.is_loopback());
        assert!(spec.input().is_none());
        assert!(spec.output().is_some());
    }

    #[test]
    fn device_pairs_id_and_spec() {
        let spec = loopback_spec();
        let device = Device::new(DeviceId::from_u32(2), spec);
        assert_eq!(device.id(), DeviceId::from_u32(2));
        assert_eq!(device.spec().uid(), "com.example.loopback");
    }

    #[test]
    fn with_setters_replace_streams() {
        let spec = DeviceSpec::new("uid", "name", "maker")
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 1)))
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 2)));
        // The second `with_output` replaces the first.
        assert_eq!(spec.output().unwrap().channels(), 2);
    }
}
