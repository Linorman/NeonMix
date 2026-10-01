//! The Core Audio property protocol.
//!
//! Almost every interaction between `coreaudiod` and an
//! AudioServerPlugin is a property access. The HAL asks an object
//! "do you have this property?", "how big is its value?", "give me
//! its value", "is it settable?", "set it". A property is addressed
//! by an [`PropertyAddress`]: a `(selector, scope, element)` triple.
//!
//! - The [`PropertySelector`] names *what* is being asked for
//!   (sample rate, device name, stream list, …).
//! - The [`PropertyScope`] narrows it to a direction — global,
//!   input, or output.
//! - The [`PropertyElement`] narrows it further to a channel; `0`
//!   ([`PropertyElement::MAIN`]) addresses the object as a whole.
//!
//! This module is cross-platform: the address triple, the typed
//! [`PropertyValue`], and the [`crate::dispatch`] logic that maps
//! between them are all plain data — no FFI.

extern crate alloc;

use crate::fourcc::FourCharCode;

/// What a property access is asking for.
///
/// A thin newtype over a [`FourCharCode`]; the associated constants
/// cover the selectors an AudioServerPlugin's property dispatch is
/// expected to recognise. Selectors not listed here still round-trip
/// — the dispatch simply answers `kAudioHardwareUnknownPropertyError`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct PropertySelector(pub FourCharCode);

impl PropertySelector {
    /// Build a selector from a four-character code literal.
    #[inline]
    #[must_use]
    pub const fn new(code: [u8; 4]) -> Self {
        Self(FourCharCode::new(code))
    }

    /// The underlying four-character code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> FourCharCode {
        self.0
    }

    // --- AudioObject: properties common to every object ---

    /// `kAudioObjectPropertyBaseClass` (`'bcls'`).
    pub const BASE_CLASS: Self = Self::new(*b"bcls");
    /// `kAudioObjectPropertyClass` (`'clas'`).
    pub const CLASS: Self = Self::new(*b"clas");
    /// `kAudioObjectPropertyOwner` (`'stdv'`).
    pub const OWNER: Self = Self::new(*b"stdv");
    /// `kAudioObjectPropertyName` (`'lnam'`).
    pub const NAME: Self = Self::new(*b"lnam");
    /// `kAudioObjectPropertyManufacturer` (`'lmak'`).
    pub const MANUFACTURER: Self = Self::new(*b"lmak");
    /// `kAudioObjectPropertyOwnedObjects` (`'ownd'`).
    pub const OWNED_OBJECTS: Self = Self::new(*b"ownd");

    // --- AudioPlugIn ---

    /// `kAudioPlugInPropertyBoxList` (`'box#'`).
    pub const PLUGIN_BOX_LIST: Self = Self::new(*b"box#");
    /// `kAudioPlugInPropertyDeviceList` (`'dev#'`).
    pub const PLUGIN_DEVICE_LIST: Self = Self::new(*b"dev#");
    /// `kAudioPlugInPropertyTranslateUIDToDevice` (`'uidd'`).
    pub const PLUGIN_TRANSLATE_UID_TO_DEVICE: Self = Self::new(*b"uidd");
    /// `kAudioPlugInPropertyResourceBundle` (`'rsrc'`).
    pub const PLUGIN_RESOURCE_BUNDLE: Self = Self::new(*b"rsrc");

    // --- AudioDevice ---

    /// `kAudioDevicePropertyDeviceUID` (`'uid '`).
    pub const DEVICE_UID: Self = Self::new(*b"uid ");
    /// `kAudioDevicePropertyModelUID` (`'muid'`).
    pub const DEVICE_MODEL_UID: Self = Self::new(*b"muid");
    /// `kAudioDevicePropertyTransportType` (`'tran'`).
    pub const DEVICE_TRANSPORT_TYPE: Self = Self::new(*b"tran");
    /// `kAudioDevicePropertyDeviceIsAlive` (`'livn'`).
    pub const DEVICE_IS_ALIVE: Self = Self::new(*b"livn");
    /// `kAudioDevicePropertyDeviceIsRunning` (`'goin'`).
    pub const DEVICE_IS_RUNNING: Self = Self::new(*b"goin");
    /// `kAudioDevicePropertyStreams` (`'stm#'`).
    pub const DEVICE_STREAMS: Self = Self::new(*b"stm#");
    /// `kAudioDevicePropertyNominalSampleRate` (`'nsrt'`).
    pub const DEVICE_NOMINAL_SAMPLE_RATE: Self = Self::new(*b"nsrt");
    /// `kAudioDevicePropertyAvailableNominalSampleRates` (`'nsr#'`).
    pub const DEVICE_AVAILABLE_SAMPLE_RATES: Self = Self::new(*b"nsr#");
    /// `kAudioDevicePropertyLatency` (`'ltnc'`).
    pub const DEVICE_LATENCY: Self = Self::new(*b"ltnc");
    /// `kAudioDevicePropertySafetyOffset` (`'saft'`).
    pub const DEVICE_SAFETY_OFFSET: Self = Self::new(*b"saft");
    /// `kAudioDevicePropertyZeroTimeStampPeriod` (`'ring'`).
    pub const DEVICE_ZERO_TIMESTAMP_PERIOD: Self = Self::new(*b"ring");

    // --- AudioStream ---

    /// `kAudioStreamPropertyDirection` (`'sdir'`).
    pub const STREAM_DIRECTION: Self = Self::new(*b"sdir");
    /// `kAudioStreamPropertyTerminalType` (`'term'`).
    pub const STREAM_TERMINAL_TYPE: Self = Self::new(*b"term");
    /// `kAudioStreamPropertyStartingChannel` (`'schn'`).
    pub const STREAM_STARTING_CHANNEL: Self = Self::new(*b"schn");
    /// `kAudioStreamPropertyVirtualFormat` (`'sfmt'`).
    pub const STREAM_VIRTUAL_FORMAT: Self = Self::new(*b"sfmt");
    /// `kAudioStreamPropertyPhysicalFormat` (`'pft '`).
    pub const STREAM_PHYSICAL_FORMAT: Self = Self::new(*b"pft ");
    /// `kAudioStreamPropertyAvailableVirtualFormats` (`'sfma'`).
    pub const STREAM_AVAILABLE_VIRTUAL_FORMATS: Self = Self::new(*b"sfma");
    /// `kAudioStreamPropertyAvailablePhysicalFormats`.
    pub const STREAM_AVAILABLE_PHYSICAL_FORMATS: Self = Self::new(*b"pfta");
    /// `kAudioStreamPropertyIsActive`.
    pub const STREAM_IS_ACTIVE: Self = Self::new(*b"sact");
    /// `kAudioDevicePropertyClockDomain`.
    pub const DEVICE_CLOCK_DOMAIN: Self = Self::new(*b"clkd");
    /// `kAudioDevicePropertyDeviceCanBeDefaultDevice`.
    pub const DEVICE_CAN_BE_DEFAULT: Self = Self::new(*b"dflt");
    /// `kAudioDevicePropertyDeviceCanBeDefaultSystemDevice`.
    pub const DEVICE_CAN_BE_SYSTEM_DEFAULT: Self = Self::new(*b"sflt");
    /// `kAudioDevicePropertyIsHidden`.
    pub const DEVICE_IS_HIDDEN: Self = Self::new(*b"hidn");
    /// `kAudioDevicePropertyRelatedDevices`.
    pub const DEVICE_RELATED_DEVICES: Self = Self::new(*b"akin");
}

impl From<FourCharCode> for PropertySelector {
    #[inline]
    fn from(code: FourCharCode) -> Self {
        Self(code)
    }
}

/// The direction a property access is scoped to.
///
/// A thin newtype over a [`FourCharCode`]. Core Audio defines a
/// handful of well-known scopes; the three an AudioServerPlugin
/// almost always sees are [`Self::GLOBAL`], [`Self::INPUT`], and
/// [`Self::OUTPUT`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct PropertyScope(pub FourCharCode);

impl PropertyScope {
    /// `kAudioObjectPropertyScopeGlobal` (`'glob'`) — the property
    /// applies to the object as a whole, with no direction.
    pub const GLOBAL: Self = Self(FourCharCode::new(*b"glob"));
    /// `kAudioObjectPropertyScopeInput` (`'inpt'`) — the input
    /// (capture) side.
    pub const INPUT: Self = Self(FourCharCode::new(*b"inpt"));
    /// `kAudioObjectPropertyScopeOutput` (`'outp'`) — the output
    /// (playback) side.
    pub const OUTPUT: Self = Self(FourCharCode::new(*b"outp"));
    /// `kAudioObjectPropertyScopePlayThrough` (`'ptru'`) — the
    /// play-through (monitoring) side.
    pub const PLAY_THROUGH: Self = Self(FourCharCode::new(*b"ptru"));

    /// The underlying four-character code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> FourCharCode {
        self.0
    }
}

impl From<FourCharCode> for PropertyScope {
    #[inline]
    fn from(code: FourCharCode) -> Self {
        Self(code)
    }
}

/// The channel a property access is scoped to.
///
/// Layout-compatible with the C `AudioObjectPropertyElement`
/// (`u32`). Element `0` ([`Self::MAIN`]) addresses the object as a
/// whole; `1`, `2`, … address individual channels.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct PropertyElement(pub u32);

impl PropertyElement {
    /// `kAudioObjectPropertyElementMain` (`0`) — the whole object,
    /// rather than any single channel. (Spelled `…ElementMaster`
    /// before macOS 12.)
    pub const MAIN: Self = Self(0);

    /// Address channel `n` (1-based). Channel `0` is
    /// [`Self::MAIN`].
    #[inline]
    #[must_use]
    pub const fn channel(n: u32) -> Self {
        Self(n)
    }

    /// The raw `u32`.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// `true` iff this is [`Self::MAIN`].
    #[inline]
    #[must_use]
    pub const fn is_main(self) -> bool {
        self.0 == 0
    }
}

impl From<u32> for PropertyElement {
    #[inline]
    fn from(value: u32) -> Self {
        Self(value)
    }
}

/// A fully-qualified property address: `(selector, scope, element)`.
///
/// Cross-platform mirror of the C `AudioObjectPropertyAddress`
/// struct. The framework's property dispatch matches on one of
/// these to route a HAL request to the right handler.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct PropertyAddress {
    /// What is being asked for.
    pub selector: PropertySelector,
    /// Which direction the access applies to.
    pub scope: PropertyScope,
    /// Which channel the access applies to.
    pub element: PropertyElement,
}

impl PropertyAddress {
    /// Construct an address from its three components.
    #[inline]
    #[must_use]
    pub const fn new(
        selector: PropertySelector,
        scope: PropertyScope,
        element: PropertyElement,
    ) -> Self {
        Self {
            selector,
            scope,
            element,
        }
    }

    /// Construct a global-scope, main-element address — the most
    /// common shape for object-wide properties such as the device
    /// name or UID.
    #[inline]
    #[must_use]
    pub const fn global(selector: PropertySelector) -> Self {
        Self::new(selector, PropertyScope::GLOBAL, PropertyElement::MAIN)
    }

    /// `true` iff this address is global-scope and main-element —
    /// i.e. it addresses the object as a whole.
    #[inline]
    #[must_use]
    pub fn is_object_wide(&self) -> bool {
        self.scope == PropertyScope::GLOBAL && self.element.is_main()
    }
}

/// A `(min, max)` inclusive range of `f64` values.
///
/// Cross-platform mirror of the C `AudioValueRange` struct. Core
/// Audio uses arrays of these for properties such as
/// `kAudioDevicePropertyAvailableNominalSampleRates`; a fixed-rate
/// device reports a single range whose `min` equals its `max`.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct ValueRange {
    /// Inclusive lower bound (`mMinimum`).
    pub min: f64,
    /// Inclusive upper bound (`mMaximum`).
    pub max: f64,
}

impl ValueRange {
    /// The on-the-wire size of one `AudioValueRange` — two `f64`s.
    pub const SIZE: usize = 16;

    /// A range covering exactly the single value `v` (`min == max`).
    #[inline]
    #[must_use]
    pub const fn point(v: f64) -> Self {
        Self { min: v, max: v }
    }

    /// A `min..=max` range.
    #[inline]
    #[must_use]
    pub const fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    /// `true` iff `value` falls within `min..=max`.
    #[inline]
    #[must_use]
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// A typed property value crossing the Core Audio property protocol.
///
/// The HAL's `GetPropertyData` / `SetPropertyData` calls move opaque
/// byte buffers; [`PropertyValue`] is the framework's typed
/// intermediate. The property dispatcher produces one of these for a
/// read, and [`Self::byte_size`] answers the HAL's separate
/// `GetPropertyDataSize` query without re-deriving the value.
///
/// The variants cover the value shapes the standard plug-in,
/// device, and stream properties use; the FFI layer (landing in a
/// later PR) marshals each variant to and from the matching C type.
#[derive(Clone, PartialEq, Debug)]
pub enum PropertyValue {
    /// A 32-bit unsigned integer — class ids, channel counts,
    /// boolean flags expressed as `0` / `1`, latencies, and the
    /// like.
    U32(u32),
    /// An [`AudioObjectId`](crate::object::AudioObjectId) — the
    /// `kAudioObjectPropertyOwner` value and similar single-object
    /// references.
    ObjectId(crate::object::AudioObjectId),
    /// A 64-bit float — `kAudioDevicePropertyNominalSampleRate` and
    /// other scalar rates.
    F64(f64),
    /// A 32-bit scalar volume or decibel value.
    F32(f32),
    /// A UTF-8 string, marshalled to a `CFStringRef` at the FFI
    /// boundary — device names, manufacturer strings, UIDs.
    Text(alloc::string::String),
    /// A list of [`AudioObjectId`](crate::object::AudioObjectId)s —
    /// device lists, stream lists, owned-object lists.
    ObjectList(alloc::vec::Vec<crate::object::AudioObjectId>),
    /// A stream format, marshalled to an `AudioStreamBasicDescription`
    /// at the FFI boundary.
    Format(crate::format::StreamFormat),
    /// AudioStreamRangedDescription entries: one ASBD and one sample-rate range.
    RangedFormats(Vec<(crate::format::StreamFormat, ValueRange)>),
    /// AudioServerPlugInCustomPropertyInfo triples: selector, data type, qualifier type.
    CustomProperties(Vec<[u32; 3]>),
    /// A list of [`ValueRange`]s, marshalled to an `AudioValueRange`
    /// array — `kAudioDevicePropertyAvailableNominalSampleRates` and
    /// similar.
    RangeList(alloc::vec::Vec<ValueRange>),
}

impl PropertyValue {
    /// The number of bytes this value occupies in the HAL's
    /// property buffer — the answer to a `GetPropertyDataSize`
    /// query.
    ///
    /// - [`Self::U32`] / [`Self::ObjectId`] — 4 bytes.
    /// - [`Self::F64`] — 8 bytes.
    /// - [`Self::Text`] — the size of a `CFStringRef`, i.e. one
    ///   pointer. Core Audio string properties always report
    ///   `sizeof(CFStringRef)` regardless of the string's length.
    /// - [`Self::ObjectList`] — 4 bytes per `AudioObjectID`.
    /// - [`Self::Format`] — 40 bytes, the size of an
    ///   `AudioStreamBasicDescription`.
    /// - [`Self::RangeList`] — 16 bytes per `AudioValueRange`.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Self::U32(_) | Self::ObjectId(_) => core::mem::size_of::<u32>(),
            Self::F64(_) => core::mem::size_of::<f64>(),
            Self::F32(_) => core::mem::size_of::<f32>(),
            // A CFString property's value *is* a `CFStringRef` — a
            // single pointer — no matter how long the string is.
            Self::Text(_) => core::mem::size_of::<*const ()>(),
            Self::ObjectList(ids) => ids.len() * core::mem::size_of::<u32>(),
            Self::Format(_) => crate::format::ASBD_SIZE,
            Self::RangedFormats(formats) => {
                formats.len() * (crate::format::ASBD_SIZE + ValueRange::SIZE)
            }
            Self::CustomProperties(properties) => properties.len() * 12,
            Self::RangeList(ranges) => ranges.len() * ValueRange::SIZE,
        }
    }

    /// `true` iff this value carries no elements — an empty
    /// [`Self::ObjectList`] or [`Self::RangeList`]. Scalar variants
    /// are never empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::ObjectList(ids) => ids.is_empty(),
            Self::RangeList(ranges) => ranges.is_empty(),
            Self::RangedFormats(formats) => formats.is_empty(),
            Self::CustomProperties(properties) => properties.is_empty(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_constants_match_core_audio_codes() {
        assert_eq!(
            PropertySelector::DEVICE_UID.code(),
            FourCharCode::new(*b"uid ")
        );
        assert_eq!(
            PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE.code(),
            FourCharCode::new(*b"nsrt")
        );
        assert_eq!(
            PropertySelector::PLUGIN_DEVICE_LIST.code(),
            FourCharCode::new(*b"dev#")
        );
        assert_eq!(
            PropertySelector::STREAM_VIRTUAL_FORMAT.code(),
            FourCharCode::new(*b"sfmt")
        );
    }

    #[test]
    fn scope_constants_match_core_audio_codes() {
        assert_eq!(PropertyScope::GLOBAL.code(), FourCharCode::new(*b"glob"));
        assert_eq!(PropertyScope::INPUT.code(), FourCharCode::new(*b"inpt"));
        assert_eq!(PropertyScope::OUTPUT.code(), FourCharCode::new(*b"outp"));
    }

    #[test]
    fn element_main_is_zero() {
        assert_eq!(PropertyElement::MAIN.as_u32(), 0);
        assert!(PropertyElement::MAIN.is_main());
        assert!(!PropertyElement::channel(1).is_main());
        assert_eq!(PropertyElement::channel(3).as_u32(), 3);
    }

    #[test]
    fn global_address_is_object_wide() {
        let addr = PropertyAddress::global(PropertySelector::NAME);
        assert_eq!(addr.selector, PropertySelector::NAME);
        assert_eq!(addr.scope, PropertyScope::GLOBAL);
        assert_eq!(addr.element, PropertyElement::MAIN);
        assert!(addr.is_object_wide());
    }

    #[test]
    fn scoped_address_is_not_object_wide() {
        let input = PropertyAddress::new(
            PropertySelector::DEVICE_STREAMS,
            PropertyScope::INPUT,
            PropertyElement::MAIN,
        );
        assert!(!input.is_object_wide());

        let channel = PropertyAddress::new(
            PropertySelector::NAME,
            PropertyScope::GLOBAL,
            PropertyElement::channel(2),
        );
        assert!(!channel.is_object_wide());
    }

    #[test]
    fn address_equality_is_structural() {
        let a = PropertyAddress::global(PropertySelector::DEVICE_UID);
        let b = PropertyAddress::new(
            PropertySelector::DEVICE_UID,
            PropertyScope::GLOBAL,
            PropertyElement::MAIN,
        );
        assert_eq!(a, b);

        let c = PropertyAddress::new(
            PropertySelector::DEVICE_UID,
            PropertyScope::INPUT,
            PropertyElement::MAIN,
        );
        assert_ne!(a, c);
    }

    #[test]
    fn selectors_built_from_fourcc_round_trip() {
        let code = FourCharCode::new(*b"nsrt");
        assert_eq!(
            PropertySelector::from(code),
            PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE
        );
    }

    #[test]
    fn newtype_layouts_are_transparent() {
        use core::mem::size_of;
        assert_eq!(size_of::<PropertySelector>(), size_of::<u32>());
        assert_eq!(size_of::<PropertyScope>(), size_of::<u32>());
        assert_eq!(size_of::<PropertyElement>(), size_of::<u32>());
    }

    #[test]
    fn value_range_point_has_equal_bounds() {
        let r = ValueRange::point(48_000.0);
        assert_eq!(r.min, 48_000.0);
        assert_eq!(r.max, 48_000.0);
        assert!(r.contains(48_000.0));
        assert!(!r.contains(44_100.0));
    }

    #[test]
    fn value_range_contains_is_inclusive() {
        let r = ValueRange::new(44_100.0, 96_000.0);
        assert!(r.contains(44_100.0));
        assert!(r.contains(96_000.0));
        assert!(r.contains(48_000.0));
        assert!(!r.contains(44_099.0));
        assert!(!r.contains(96_001.0));
    }

    #[test]
    fn property_value_byte_sizes_match_the_c_types() {
        use crate::format::StreamFormat;
        use crate::object::AudioObjectId;

        assert_eq!(PropertyValue::U32(0).byte_size(), 4);
        assert_eq!(
            PropertyValue::ObjectId(AudioObjectId::PLUGIN).byte_size(),
            4
        );
        assert_eq!(PropertyValue::F64(0.0).byte_size(), 8);
        // A CFString property reports the size of a pointer,
        // independent of the string's length.
        assert_eq!(
            PropertyValue::Text("x".into()).byte_size(),
            PropertyValue::Text("a much longer string".into()).byte_size()
        );
        assert_eq!(
            PropertyValue::Text(alloc::string::String::new()).byte_size(),
            core::mem::size_of::<*const ()>()
        );
        assert_eq!(
            PropertyValue::ObjectList(alloc::vec![
                AudioObjectId::from_u32(2),
                AudioObjectId::from_u32(3)
            ])
            .byte_size(),
            8
        );
        assert_eq!(
            PropertyValue::Format(StreamFormat::float32(48_000.0, 2)).byte_size(),
            40
        );
        assert_eq!(
            PropertyValue::RangeList(alloc::vec![ValueRange::point(48_000.0)]).byte_size(),
            16
        );
    }

    #[test]
    fn property_value_is_empty_only_for_empty_lists() {
        use crate::object::AudioObjectId;
        assert!(PropertyValue::ObjectList(alloc::vec![]).is_empty());
        assert!(PropertyValue::RangeList(alloc::vec![]).is_empty());
        assert!(!PropertyValue::ObjectList(alloc::vec![AudioObjectId::PLUGIN]).is_empty());
        assert!(!PropertyValue::U32(0).is_empty());
        assert!(!PropertyValue::F64(0.0).is_empty());
        assert!(!PropertyValue::Text(alloc::string::String::new()).is_empty());
    }
}
