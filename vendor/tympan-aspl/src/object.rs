//! Audio object identifiers and the AudioServerPlugin object model.
//!
//! Every entity an AudioServerPlugin exposes — the plug-in itself,
//! its boxes, devices, streams, and controls — is an *audio object*
//! addressed by a 32-bit [`AudioObjectId`]. The HAL hands the
//! plug-in a single well-known id at startup
//! ([`AudioObjectId::PLUGIN`]) and the plug-in mints ids for every
//! object it creates.
//!
//! The object graph is a tree: the plug-in owns boxes and devices,
//! a device owns streams and controls. [`ObjectKind`] tags which
//! layer of that tree a given id sits at; the framework's property
//! dispatch uses it to route a request to the right handler.

use crate::fourcc::FourCharCode;

/// A Core Audio object identifier.
///
/// Layout-compatible with the C `AudioObjectID` (`u32`).
/// [`AudioObjectId::UNKNOWN`] (`0`) is the reserved "no object"
/// sentinel; [`AudioObjectId::PLUGIN`] (`1`) is the fixed id the
/// HAL uses to address the plug-in object itself. Every other id is
/// minted by the plug-in as it creates boxes, devices, and streams.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct AudioObjectId(pub u32);

impl AudioObjectId {
    /// `kAudioObjectUnknown` — the reserved "no object" id.
    pub const UNKNOWN: Self = Self(0);

    /// `kAudioObjectPlugInObject` — the fixed id by which the HAL
    /// addresses the plug-in object. The plug-in does not mint this
    /// id; it is assigned by Core Audio before the first property
    /// call.
    pub const PLUGIN: Self = Self(1);

    /// The first id the plug-in is free to mint for its own
    /// objects. Ids `0` and `1` are reserved (see [`Self::UNKNOWN`]
    /// and [`Self::PLUGIN`]).
    pub const FIRST_DYNAMIC: Self = Self(2);

    /// Wrap a raw `AudioObjectID`.
    #[inline]
    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    /// The raw `u32`, ready for the FFI boundary.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// `true` iff this is [`Self::UNKNOWN`].
    #[inline]
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        self.0 == 0
    }

    /// `true` iff this is [`Self::PLUGIN`].
    #[inline]
    #[must_use]
    pub const fn is_plugin(self) -> bool {
        self.0 == Self::PLUGIN.0
    }
}

impl From<u32> for AudioObjectId {
    #[inline]
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<AudioObjectId> for u32 {
    #[inline]
    fn from(value: AudioObjectId) -> Self {
        value.0
    }
}

/// Which layer of the AudioServerPlugin object tree an id sits at.
///
/// The values mirror the `kAudio*ClassID` four-character codes from
/// `<CoreAudio/AudioHardwareBase.h>`. The framework's property
/// dispatch reads this off an id to pick the property table that
/// applies — global object properties, plug-in properties, device
/// properties, and so on.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum ObjectKind {
    /// `kAudioPlugInClassID` (`'aplg'`) — the plug-in object.
    PlugIn,
    /// `kAudioBoxClassID` (`'abox'`) — a box: a container that
    /// groups devices, modelling a physical or virtual enclosure.
    Box,
    /// `kAudioDeviceClassID` (`'adev'`) — a device exposed to the
    /// system as an audio endpoint.
    Device,
    /// `kAudioStreamClassID` (`'astr'`) — one direction of audio
    /// flow on a device.
    Stream,
    /// `kAudioControlClassID` (`'actl'`) — a control (volume, mute,
    /// data source, …) attached to a device or stream.
    Control,
}

impl ObjectKind {
    /// The `kAudio*ClassID` four-character code for this kind.
    #[inline]
    #[must_use]
    pub const fn class_id(self) -> FourCharCode {
        match self {
            Self::PlugIn => FourCharCode::new(*b"aplg"),
            Self::Box => FourCharCode::new(*b"abox"),
            Self::Device => FourCharCode::new(*b"adev"),
            Self::Stream => FourCharCode::new(*b"astr"),
            Self::Control => FourCharCode::new(*b"actl"),
        }
    }

    /// The `kAudio*ClassID` of this kind's base class, or `None` for
    /// the root of the hierarchy.
    ///
    /// Core Audio's `kAudioObjectPropertyBaseClass` walks this
    /// chain; every class but `AudioObject` itself ultimately bases
    /// on `kAudioObjectClassID` (`'aobj'`).
    #[inline]
    #[must_use]
    pub const fn base_class_id(self) -> FourCharCode {
        // Every first-class object in this model bases directly on
        // `kAudioObjectClassID`; the deeper inheritance chains
        // Core Audio defines (e.g. volume control → level control →
        // control) are not yet modelled.
        FourCharCode::new(*b"aobj")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_ids_have_fixed_values() {
        assert_eq!(AudioObjectId::UNKNOWN.as_u32(), 0);
        assert_eq!(AudioObjectId::PLUGIN.as_u32(), 1);
        assert_eq!(AudioObjectId::FIRST_DYNAMIC.as_u32(), 2);
    }

    #[test]
    fn predicates_classify_reserved_ids() {
        assert!(AudioObjectId::UNKNOWN.is_unknown());
        assert!(!AudioObjectId::UNKNOWN.is_plugin());
        assert!(AudioObjectId::PLUGIN.is_plugin());
        assert!(!AudioObjectId::PLUGIN.is_unknown());
        assert!(!AudioObjectId::from_u32(42).is_unknown());
        assert!(!AudioObjectId::from_u32(42).is_plugin());
    }

    #[test]
    fn round_trips_through_u32() {
        for raw in [0, 1, 2, 99, u32::MAX] {
            let id = AudioObjectId::from_u32(raw);
            assert_eq!(id.as_u32(), raw);
            let back: u32 = id.into();
            assert_eq!(back, raw);
            assert_eq!(AudioObjectId::from(raw), id);
        }
    }

    #[test]
    fn class_ids_match_core_audio_codes() {
        assert_eq!(ObjectKind::PlugIn.class_id(), FourCharCode::new(*b"aplg"));
        assert_eq!(ObjectKind::Box.class_id(), FourCharCode::new(*b"abox"));
        assert_eq!(ObjectKind::Device.class_id(), FourCharCode::new(*b"adev"));
        assert_eq!(ObjectKind::Stream.class_id(), FourCharCode::new(*b"astr"));
        assert_eq!(ObjectKind::Control.class_id(), FourCharCode::new(*b"actl"));
    }

    #[test]
    fn every_kind_bases_on_audio_object() {
        for kind in [
            ObjectKind::PlugIn,
            ObjectKind::Box,
            ObjectKind::Device,
            ObjectKind::Stream,
            ObjectKind::Control,
        ] {
            assert_eq!(kind.base_class_id(), FourCharCode::new(*b"aobj"));
        }
    }

    #[test]
    fn ids_order_numerically() {
        assert!(AudioObjectId::UNKNOWN < AudioObjectId::PLUGIN);
        assert!(AudioObjectId::PLUGIN < AudioObjectId::FIRST_DYNAMIC);
    }

    #[test]
    fn layout_is_transparent_u32() {
        use core::mem::{align_of, size_of};
        assert_eq!(size_of::<AudioObjectId>(), size_of::<u32>());
        assert_eq!(align_of::<AudioObjectId>(), align_of::<u32>());
    }
}
