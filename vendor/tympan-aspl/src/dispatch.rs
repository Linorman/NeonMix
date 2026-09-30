//! Core Audio property dispatch.
//!
//! The property protocol is the bulk of an AudioServerPlugin's
//! surface: `coreaudiod` discovers and configures the plug-in
//! almost entirely by reading and writing properties on the objects
//! in its [`ObjectMap`]. This module is the framework's answer to
//! those reads — given an [`ObjectMap`], the driver's [`DriverInfo`],
//! the device's runtime [`DeviceState`], an object id, and a
//! [`PropertyAddress`], it produces the typed [`PropertyValue`] (or
//! the [`OsStatus`] error the HAL expects).
//!
//! It is cross-platform pure logic — no FFI. The macOS `raw` layer
//! (a later PR) will marshal the HAL's opaque byte buffers to and
//! from these calls; isolating the logic here keeps every standard
//! property's behaviour unit-testable on any host.
//!
//! ## Coverage
//!
//! The dispatcher answers the standard properties the HAL needs to
//! enumerate and run a virtual device:
//!
//! - **Every object**: base class, class, owner, name, owned
//!   objects.
//! - **Plug-in**: manufacturer, device list, box list.
//! - **Device**: manufacturer, UID, model UID, transport type,
//!   alive / running flags, stream list, nominal and available
//!   sample rates, latency, safety offset, zero-timestamp period.
//! - **Stream**: direction, terminal type, starting channel,
//!   virtual and physical format.
//!
//! Any selector outside this set yields
//! [`OsStatus::UNKNOWN_PROPERTY`] — the correct Core Audio answer
//! for "this object does not have that property".

extern crate alloc;

use alloc::string::String;
use alloc::vec;

use crate::driver::DriverInfo;
use crate::error::OsStatus;
use crate::object::{AudioObjectId, ObjectKind};
use crate::objects::{Object, ObjectMap};
use crate::property::{PropertyAddress, PropertySelector, PropertyValue, ValueRange};
use crate::stream::StreamDirection;

/// `kAudioDeviceTransportTypeVirtual` (`'virt'`) — the
/// `kAudioDevicePropertyTransportType` value the framework reports
/// for every device it exposes. The devices are virtual: they have
/// no physical transport.
pub const TRANSPORT_TYPE_VIRTUAL: u32 = u32::from_be_bytes(*b"virt");

/// `kAudioStreamTerminalTypeUnknown` (`0`) — the
/// `kAudioStreamPropertyTerminalType` value the framework reports
/// for every stream. A virtual stream is not wired to any specific
/// physical terminal (speaker, microphone, line-in, …).
pub const TERMINAL_TYPE_UNKNOWN: u32 = 0;

/// The mutable runtime state of a device, distinct from the
/// immutable [`DeviceSpec`](crate::device::DeviceSpec) the driver
/// declared.
///
/// A `DeviceSpec` says what a device *is*; a `DeviceState` says what
/// it is *currently doing* — the sample rate it is running at (which
/// the HAL can change via `kAudioDevicePropertyNominalSampleRate`)
/// and whether its IO is running. The property dispatcher reads it
/// for the handful of properties whose value is not fixed at
/// construction.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct DeviceState {
    /// The sample rate the device is currently running at, in hertz.
    /// Starts at the spec's nominal rate; the HAL may change it.
    pub sample_rate: f64,
    /// `true` once the HAL has started the device's IO (`StartIO`),
    /// `false` before that and after `StopIO`.
    pub running: bool,
}

impl DeviceState {
    /// The initial runtime state for `spec`: its nominal sample
    /// rate, IO not yet running.
    #[inline]
    #[must_use]
    pub fn from_spec(spec: &crate::device::DeviceSpec) -> Self {
        Self {
            sample_rate: spec.sample_rate(),
            running: false,
        }
    }
}

/// Read a property's value.
///
/// Resolves `object_id` against `map`, then dispatches to the
/// property table for that object's kind. Returns:
///
/// - [`OsStatus::BAD_OBJECT`] if `object_id` is not in the tree,
/// - [`OsStatus::UNKNOWN_PROPERTY`] if the object does not have the
///   addressed property,
/// - the typed [`PropertyValue`] otherwise.
///
/// `address.element` is ignored: every property the framework
/// dispatches is object-wide. `address.scope` matters only for the
/// owned-objects and stream-list properties, which filter by
/// direction.
pub fn get_property_data(
    map: &ObjectMap,
    info: &DriverInfo,
    state: &DeviceState,
    object_id: AudioObjectId,
    address: &PropertyAddress,
) -> Result<PropertyValue, OsStatus> {
    match map.resolve(object_id).ok_or(OsStatus::BAD_OBJECT)? {
        Object::PlugIn => plugin_property(map, info, address),
        Object::Device => device_property(map, state, address),
        Object::Stream(direction) => stream_property(map, direction, address),
    }
}

/// The size, in bytes, a property's value occupies in the HAL's
/// buffer — the answer to a `GetPropertyDataSize` query.
///
/// Computed from [`get_property_data`], so it returns the same
/// [`OsStatus::BAD_OBJECT`] / [`OsStatus::UNKNOWN_PROPERTY`] errors
/// for an unknown object or property.
pub fn property_data_size(
    map: &ObjectMap,
    info: &DriverInfo,
    state: &DeviceState,
    object_id: AudioObjectId,
    address: &PropertyAddress,
) -> Result<usize, OsStatus> {
    get_property_data(map, info, state, object_id, address).map(|value| value.byte_size())
}

/// Whether a property can be written with `SetPropertyData`.
///
/// Returns [`OsStatus::BAD_OBJECT`] / [`OsStatus::UNKNOWN_PROPERTY`]
/// for an unknown object or property (the existence check reuses
/// [`get_property_data`]). For a property the object does have,
/// returns `Ok(true)` only for the writable ones — currently just
/// `kAudioDevicePropertyNominalSampleRate` — and `Ok(false)` for
/// everything else.
pub fn is_property_settable(
    map: &ObjectMap,
    info: &DriverInfo,
    state: &DeviceState,
    object_id: AudioObjectId,
    address: &PropertyAddress,
) -> Result<bool, OsStatus> {
    // The property must exist on the object before "is it settable?"
    // is a meaningful question; reuse the read path as the
    // existence check.
    get_property_data(map, info, state, object_id, address)?;
    let object = map.resolve(object_id).ok_or(OsStatus::BAD_OBJECT)?;
    Ok(matches!(
        (object, address.selector),
        (Object::Device, PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE)
    ))
}

/// Write a property's value.
///
/// `data` is the raw, native-endian `SetPropertyData` buffer the HAL
/// supplied. The property must exist on the object and be settable;
/// the only writable property today is the device's nominal sample
/// rate, which `data` must carry as a `Float64`.
///
/// On success the change is applied to `state`. Returns:
///
/// - [`OsStatus::BAD_OBJECT`] if `object_id` is not in the tree,
/// - [`OsStatus::UNKNOWN_PROPERTY`] if the object lacks the property,
/// - [`OsStatus::ILLEGAL_OPERATION`] if the property exists but is
///   read-only,
/// - [`OsStatus::BAD_PROPERTY_SIZE`] if `data` is too short to hold
///   the value,
/// - [`OsStatus::UNSUPPORTED_FORMAT`] if the requested sample rate
///   is not one the device offers.
pub fn set_property_data(
    map: &ObjectMap,
    info: &DriverInfo,
    state: &mut DeviceState,
    object_id: AudioObjectId,
    address: &PropertyAddress,
    data: &[u8],
) -> Result<(), OsStatus> {
    // Existence + read-only checks. `is_property_settable` reuses the
    // read path, so an unknown object / property surfaces the right
    // error here too.
    if !is_property_settable(map, info, state, object_id, address)? {
        return Err(OsStatus::ILLEGAL_OPERATION);
    }
    // `is_property_settable` returns `true` for exactly one property:
    // the device's nominal sample rate. The debug assertion documents
    // that invariant — if the settable set grows, this function must
    // grow with it.
    debug_assert_eq!(
        (
            map.resolve(object_id),
            address.selector == PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE
        ),
        (Some(Object::Device), true),
    );

    let rate = read_f64(data)?;
    // A fixed-rate virtual device offers exactly its spec's nominal
    // rate; any other request is an unsupported format.
    if rate != map.spec().sample_rate() {
        return Err(OsStatus::UNSUPPORTED_FORMAT);
    }
    state.sample_rate = rate;
    Ok(())
}

/// Read a native-endian `f64` from the head of a HAL property
/// buffer, or [`OsStatus::BAD_PROPERTY_SIZE`] if `data` is shorter
/// than eight bytes.
fn read_f64(data: &[u8]) -> Result<f64, OsStatus> {
    let bytes: [u8; 8] = data
        .get(..8)
        .ok_or(OsStatus::BAD_PROPERTY_SIZE)?
        .try_into()
        .expect("slice of length 8 converts to [u8; 8]");
    Ok(f64::from_ne_bytes(bytes))
}

/// Number of frames between the device's zero timestamps —
/// `kAudioDevicePropertyZeroTimeStampPeriod`.
///
/// The framework models a virtual device with a one-second ring
/// buffer, so the period is the nominal sample rate rounded to a
/// whole number of frames.
fn zero_timestamp_period(spec: &crate::device::DeviceSpec) -> u32 {
    spec.sample_rate() as u32
}

fn plugin_property(
    map: &ObjectMap,
    info: &DriverInfo,
    address: &PropertyAddress,
) -> Result<PropertyValue, OsStatus> {
    use PropertySelector as S;
    Ok(match address.selector {
        S::BASE_CLASS => PropertyValue::U32(ObjectKind::PlugIn.base_class_id().as_u32()),
        S::CLASS => PropertyValue::U32(ObjectKind::PlugIn.class_id().as_u32()),
        // The plug-in object sits at the root of the tree — it has
        // no owner.
        S::OWNER => PropertyValue::ObjectId(AudioObjectId::UNKNOWN),
        S::NAME => PropertyValue::Text(String::from(info.name)),
        S::MANUFACTURER => PropertyValue::Text(String::from(info.manufacturer)),
        S::OWNED_OBJECTS => {
            PropertyValue::ObjectList(map.owned_objects(map.plugin_id(), address.scope))
        }
        S::PLUGIN_DEVICE_LIST => PropertyValue::ObjectList(vec![map.device_id()]),
        // The raw ABI resolves this qualifier-dependent value. Its unqualified
        // shape still supplies HasProperty, size and read-only metadata.
        S::PLUGIN_TRANSLATE_UID_TO_DEVICE => PropertyValue::ObjectId(AudioObjectId::UNKNOWN),
        S::PLUGIN_RESOURCE_BUNDLE => PropertyValue::Text(String::new()),
        // The framework does not model boxes yet.
        S::PLUGIN_BOX_LIST => PropertyValue::ObjectList(vec![]),
        _ => return Err(OsStatus::UNKNOWN_PROPERTY),
    })
}

fn device_property(
    map: &ObjectMap,
    state: &DeviceState,
    address: &PropertyAddress,
) -> Result<PropertyValue, OsStatus> {
    use PropertySelector as S;
    let spec = map.spec();
    Ok(match address.selector {
        S::BASE_CLASS => PropertyValue::U32(ObjectKind::Device.base_class_id().as_u32()),
        S::CLASS => PropertyValue::U32(ObjectKind::Device.class_id().as_u32()),
        S::OWNER => PropertyValue::ObjectId(map.plugin_id()),
        S::NAME => PropertyValue::Text(String::from(spec.name())),
        S::MANUFACTURER => PropertyValue::Text(String::from(spec.manufacturer())),
        S::OWNED_OBJECTS => {
            PropertyValue::ObjectList(map.owned_objects(map.device_id(), address.scope))
        }
        S::DEVICE_UID => PropertyValue::Text(String::from(spec.uid())),
        // The framework has no notion of a separate model identity,
        // so the model UID reuses the device UID.
        S::DEVICE_MODEL_UID => PropertyValue::Text(String::from(spec.uid())),
        S::DEVICE_TRANSPORT_TYPE => PropertyValue::U32(TRANSPORT_TYPE_VIRTUAL),
        // The device is always alive once the plug-in exposes it.
        S::DEVICE_IS_ALIVE => PropertyValue::U32(1),
        S::DEVICE_IS_RUNNING => PropertyValue::U32(u32::from(state.running)),
        S::DEVICE_IS_HIDDEN | S::DEVICE_CLOCK_DOMAIN => PropertyValue::U32(0),
        S::DEVICE_RELATED_DEVICES => PropertyValue::ObjectList(vec![map.device_id()]),
        S::DEVICE_CAN_BE_DEFAULT => PropertyValue::U32(u32::from(match address.scope {
            crate::PropertyScope::INPUT => spec.input().is_some(),
            crate::PropertyScope::OUTPUT => spec.output().is_some(),
            _ => false,
        })),
        S::DEVICE_CAN_BE_SYSTEM_DEFAULT => PropertyValue::U32(u32::from(
            address.scope == crate::PropertyScope::OUTPUT && spec.output().is_some(),
        )),
        S::DEVICE_STREAMS => PropertyValue::ObjectList(map.device_streams(address.scope)),
        S::DEVICE_NOMINAL_SAMPLE_RATE => PropertyValue::F64(state.sample_rate),
        // A fixed-rate virtual device offers exactly its nominal
        // rate — a single zero-width range.
        S::DEVICE_AVAILABLE_SAMPLE_RATES => {
            PropertyValue::RangeList(vec![ValueRange::point(spec.sample_rate())])
        }
        // A purely virtual device introduces no presentation
        // latency and needs no safety offset.
        S::DEVICE_LATENCY | S::DEVICE_SAFETY_OFFSET => PropertyValue::U32(0),
        S::DEVICE_ZERO_TIMESTAMP_PERIOD => PropertyValue::U32(zero_timestamp_period(spec)),
        _ => return Err(OsStatus::UNKNOWN_PROPERTY),
    })
}

fn stream_property(
    map: &ObjectMap,
    direction: StreamDirection,
    address: &PropertyAddress,
) -> Result<PropertyValue, OsStatus> {
    use PropertySelector as S;
    let stream = map.stream_spec(direction).ok_or(OsStatus::BAD_STREAM)?;
    Ok(match address.selector {
        S::BASE_CLASS => PropertyValue::U32(ObjectKind::Stream.base_class_id().as_u32()),
        S::CLASS => PropertyValue::U32(ObjectKind::Stream.class_id().as_u32()),
        S::OWNER => PropertyValue::ObjectId(map.device_id()),
        // A stream is a leaf of the object tree.
        S::OWNED_OBJECTS => PropertyValue::ObjectList(vec![]),
        S::STREAM_DIRECTION => PropertyValue::U32(direction.as_property_value()),
        S::STREAM_IS_ACTIVE => PropertyValue::U32(1),
        S::DEVICE_LATENCY => PropertyValue::U32(0),
        S::STREAM_AVAILABLE_VIRTUAL_FORMATS | S::STREAM_AVAILABLE_PHYSICAL_FORMATS => {
            PropertyValue::RangedFormats(vec![(
                stream.format(),
                ValueRange::point(stream.format().sample_rate()),
            )])
        }
        S::STREAM_TERMINAL_TYPE => PropertyValue::U32(TERMINAL_TYPE_UNKNOWN),
        S::STREAM_STARTING_CHANNEL => PropertyValue::U32(stream.starting_channel()),
        // The framework negotiates one fixed format, so the virtual
        // and physical formats are the same.
        S::STREAM_VIRTUAL_FORMAT | S::STREAM_PHYSICAL_FORMAT => {
            PropertyValue::Format(stream.format())
        }
        _ => return Err(OsStatus::UNKNOWN_PROPERTY),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceSpec;
    use crate::format::StreamFormat;
    use crate::object::ObjectKind;
    use crate::property::{PropertyScope, PropertyValue};
    use crate::stream::StreamSpec;

    fn loopback_spec() -> DeviceSpec {
        let format = StreamFormat::float32(48_000.0, 2);
        DeviceSpec::new("com.example.loopback", "Example Loopback", "Acme Audio")
            .with_sample_rate(48_000.0)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn info() -> DriverInfo {
        DriverInfo {
            name: "Example Driver",
            manufacturer: "Acme Audio",
            version: "1.0.0",
        }
    }

    fn fixture() -> (ObjectMap, DriverInfo, DeviceState) {
        let map = ObjectMap::new(loopback_spec());
        let state = DeviceState::from_spec(map.spec());
        (map, info(), state)
    }

    fn get(
        map: &ObjectMap,
        info: &DriverInfo,
        state: &DeviceState,
        id: AudioObjectId,
        selector: PropertySelector,
    ) -> Result<PropertyValue, OsStatus> {
        get_property_data(map, info, state, id, &PropertyAddress::global(selector))
    }

    #[test]
    fn unknown_object_is_bad_object() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                AudioObjectId::from_u32(999),
                PropertySelector::CLASS
            ),
            Err(OsStatus::BAD_OBJECT)
        );
    }

    #[test]
    fn unknown_selector_is_unknown_property() {
        let (map, info, state) = fixture();
        let bogus = PropertySelector::new(*b"zzzz");
        assert_eq!(
            get(&map, &info, &state, map.plugin_id(), bogus),
            Err(OsStatus::UNKNOWN_PROPERTY)
        );
        assert_eq!(
            get(&map, &info, &state, map.device_id(), bogus),
            Err(OsStatus::UNKNOWN_PROPERTY)
        );
    }

    #[test]
    fn plugin_class_and_base_class() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::CLASS
            ),
            Ok(PropertyValue::U32(ObjectKind::PlugIn.class_id().as_u32()))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::BASE_CLASS
            ),
            Ok(PropertyValue::U32(
                ObjectKind::PlugIn.base_class_id().as_u32()
            ))
        );
    }

    #[test]
    fn plugin_has_no_owner() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::OWNER
            ),
            Ok(PropertyValue::ObjectId(AudioObjectId::UNKNOWN))
        );
    }

    #[test]
    fn plugin_identity_comes_from_driver_info() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(&map, &info, &state, map.plugin_id(), PropertySelector::NAME),
            Ok(PropertyValue::Text("Example Driver".into()))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::MANUFACTURER
            ),
            Ok(PropertyValue::Text("Acme Audio".into()))
        );
    }

    #[test]
    fn plugin_device_list_and_owned_objects_point_at_the_device() {
        let (map, info, state) = fixture();
        let expected = PropertyValue::ObjectList(vec![map.device_id()]);
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::PLUGIN_DEVICE_LIST
            ),
            Ok(expected.clone())
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::OWNED_OBJECTS
            ),
            Ok(expected)
        );
    }

    #[test]
    fn plugin_box_list_is_empty() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.plugin_id(),
                PropertySelector::PLUGIN_BOX_LIST
            ),
            Ok(PropertyValue::ObjectList(vec![]))
        );
    }

    #[test]
    fn device_identity_comes_from_the_spec() {
        let (map, info, state) = fixture();
        let dev = map.device_id();
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::NAME),
            Ok(PropertyValue::Text("Example Loopback".into()))
        );
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::DEVICE_UID),
            Ok(PropertyValue::Text("com.example.loopback".into()))
        );
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::DEVICE_MODEL_UID),
            Ok(PropertyValue::Text("com.example.loopback".into()))
        );
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::MANUFACTURER),
            Ok(PropertyValue::Text("Acme Audio".into()))
        );
    }

    #[test]
    fn device_owner_is_the_plugin() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.device_id(),
                PropertySelector::OWNER
            ),
            Ok(PropertyValue::ObjectId(map.plugin_id()))
        );
    }

    #[test]
    fn device_transport_type_is_virtual() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.device_id(),
                PropertySelector::DEVICE_TRANSPORT_TYPE
            ),
            Ok(PropertyValue::U32(u32::from_be_bytes(*b"virt")))
        );
    }

    #[test]
    fn device_is_alive_and_reflects_running_state() {
        let (map, info, mut state) = fixture();
        let dev = map.device_id();
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::DEVICE_IS_ALIVE),
            Ok(PropertyValue::U32(1))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_IS_RUNNING
            ),
            Ok(PropertyValue::U32(0))
        );
        state.running = true;
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_IS_RUNNING
            ),
            Ok(PropertyValue::U32(1))
        );
    }

    #[test]
    fn device_sample_rate_tracks_runtime_state() {
        let (map, info, mut state) = fixture();
        let dev = map.device_id();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE
            ),
            Ok(PropertyValue::F64(48_000.0))
        );
        // The HAL changed the rate at runtime.
        state.sample_rate = 96_000.0;
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE
            ),
            Ok(PropertyValue::F64(96_000.0))
        );
        // The *available* rates still come from the immutable spec.
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_AVAILABLE_SAMPLE_RATES
            ),
            Ok(PropertyValue::RangeList(vec![ValueRange::point(48_000.0)]))
        );
    }

    #[test]
    fn device_streams_filter_by_scope() {
        let (map, info, state) = fixture();
        let dev = map.device_id();
        let global = PropertyAddress::new(
            PropertySelector::DEVICE_STREAMS,
            PropertyScope::GLOBAL,
            crate::property::PropertyElement::MAIN,
        );
        let input = PropertyAddress::new(
            PropertySelector::DEVICE_STREAMS,
            PropertyScope::INPUT,
            crate::property::PropertyElement::MAIN,
        );
        assert_eq!(
            get_property_data(&map, &info, &state, dev, &global),
            Ok(PropertyValue::ObjectList(vec![
                map.stream_id(StreamDirection::Input).unwrap(),
                map.stream_id(StreamDirection::Output).unwrap(),
            ]))
        );
        assert_eq!(
            get_property_data(&map, &info, &state, dev, &input),
            Ok(PropertyValue::ObjectList(vec![map
                .stream_id(StreamDirection::Input)
                .unwrap()]))
        );
    }

    #[test]
    fn device_latency_and_safety_offset_are_zero() {
        let (map, info, state) = fixture();
        let dev = map.device_id();
        assert_eq!(
            get(&map, &info, &state, dev, PropertySelector::DEVICE_LATENCY),
            Ok(PropertyValue::U32(0))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                dev,
                PropertySelector::DEVICE_SAFETY_OFFSET
            ),
            Ok(PropertyValue::U32(0))
        );
    }

    #[test]
    fn device_zero_timestamp_period_is_one_second_of_frames() {
        let (map, info, state) = fixture();
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                map.device_id(),
                PropertySelector::DEVICE_ZERO_TIMESTAMP_PERIOD
            ),
            Ok(PropertyValue::U32(48_000))
        );
    }

    #[test]
    fn stream_class_owner_and_direction() {
        let (map, info, state) = fixture();
        let input = map.stream_id(StreamDirection::Input).unwrap();
        let output = map.stream_id(StreamDirection::Output).unwrap();
        assert_eq!(
            get(&map, &info, &state, input, PropertySelector::CLASS),
            Ok(PropertyValue::U32(ObjectKind::Stream.class_id().as_u32()))
        );
        assert_eq!(
            get(&map, &info, &state, input, PropertySelector::OWNER),
            Ok(PropertyValue::ObjectId(map.device_id()))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                input,
                PropertySelector::STREAM_DIRECTION
            ),
            Ok(PropertyValue::U32(1))
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                output,
                PropertySelector::STREAM_DIRECTION
            ),
            Ok(PropertyValue::U32(0))
        );
    }

    #[test]
    fn stream_formats_come_from_the_stream_spec() {
        let (map, info, state) = fixture();
        let input = map.stream_id(StreamDirection::Input).unwrap();
        let expected = PropertyValue::Format(StreamFormat::float32(48_000.0, 2));
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                input,
                PropertySelector::STREAM_VIRTUAL_FORMAT
            ),
            Ok(expected.clone())
        );
        assert_eq!(
            get(
                &map,
                &info,
                &state,
                input,
                PropertySelector::STREAM_PHYSICAL_FORMAT
            ),
            Ok(expected)
        );
    }

    #[test]
    fn stream_starting_channel_comes_from_the_spec() {
        let format = StreamFormat::float32(48_000.0, 2);
        let spec = DeviceSpec::new("uid", "name", "maker")
            .with_output(StreamSpec::output(format).with_starting_channel(5));
        let map = ObjectMap::new(spec);
        let state = DeviceState::from_spec(map.spec());
        let output = map.stream_id(StreamDirection::Output).unwrap();
        assert_eq!(
            get(
                &map,
                &info(),
                &state,
                output,
                PropertySelector::STREAM_STARTING_CHANNEL
            ),
            Ok(PropertyValue::U32(5))
        );
    }

    #[test]
    fn property_data_size_matches_the_value_size() {
        let (map, info, state) = fixture();
        let dev = map.device_id();
        // F64 sample rate → 8 bytes.
        assert_eq!(
            property_data_size(
                &map,
                &info,
                &state,
                dev,
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE)
            ),
            Ok(8)
        );
        // Two streams → two AudioObjectIDs → 8 bytes.
        assert_eq!(
            property_data_size(
                &map,
                &info,
                &state,
                dev,
                &PropertyAddress::global(PropertySelector::DEVICE_STREAMS)
            ),
            Ok(8)
        );
        // Stream format → AudioStreamBasicDescription → 40 bytes.
        let input = map.stream_id(StreamDirection::Input).unwrap();
        assert_eq!(
            property_data_size(
                &map,
                &info,
                &state,
                input,
                &PropertyAddress::global(PropertySelector::STREAM_VIRTUAL_FORMAT)
            ),
            Ok(40)
        );
    }

    #[test]
    fn property_data_size_propagates_errors() {
        let (map, info, state) = fixture();
        assert_eq!(
            property_data_size(
                &map,
                &info,
                &state,
                AudioObjectId::from_u32(999),
                &PropertyAddress::global(PropertySelector::CLASS)
            ),
            Err(OsStatus::BAD_OBJECT)
        );
    }

    #[test]
    fn only_the_sample_rate_is_settable() {
        let (map, info, state) = fixture();
        let dev = map.device_id();
        assert_eq!(
            is_property_settable(
                &map,
                &info,
                &state,
                dev,
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE)
            ),
            Ok(true)
        );
        // Every other known property is read-only.
        for selector in [
            PropertySelector::DEVICE_UID,
            PropertySelector::NAME,
            PropertySelector::DEVICE_STREAMS,
            PropertySelector::CLASS,
        ] {
            assert_eq!(
                is_property_settable(&map, &info, &state, dev, &PropertyAddress::global(selector)),
                Ok(false),
                "{selector:?} should be read-only"
            );
        }
    }

    #[test]
    fn is_property_settable_rejects_unknown_object_and_property() {
        let (map, info, state) = fixture();
        assert_eq!(
            is_property_settable(
                &map,
                &info,
                &state,
                AudioObjectId::from_u32(999),
                &PropertyAddress::global(PropertySelector::CLASS)
            ),
            Err(OsStatus::BAD_OBJECT)
        );
        assert_eq!(
            is_property_settable(
                &map,
                &info,
                &state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::new(*b"zzzz"))
            ),
            Err(OsStatus::UNKNOWN_PROPERTY)
        );
    }

    #[test]
    fn device_state_from_spec_is_idle_at_nominal_rate() {
        let spec = loopback_spec();
        let state = DeviceState::from_spec(&spec);
        assert_eq!(state.sample_rate, 48_000.0);
        assert!(!state.running);
    }

    #[test]
    fn set_sample_rate_applies_a_supported_rate() {
        let (map, info, mut state) = fixture();
        // The device offers 48 kHz; setting it to 48 kHz succeeds.
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE),
                &48_000.0_f64.to_ne_bytes(),
            ),
            Ok(())
        );
        assert_eq!(state.sample_rate, 48_000.0);
    }

    #[test]
    fn set_sample_rate_rejects_an_unsupported_rate() {
        let (map, info, mut state) = fixture();
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE),
                &96_000.0_f64.to_ne_bytes(),
            ),
            Err(OsStatus::UNSUPPORTED_FORMAT)
        );
        // The state is left untouched on a rejected set.
        assert_eq!(state.sample_rate, 48_000.0);
    }

    #[test]
    fn set_sample_rate_rejects_an_undersized_buffer() {
        let (map, info, mut state) = fixture();
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE),
                &[0u8; 4],
            ),
            Err(OsStatus::BAD_PROPERTY_SIZE)
        );
    }

    #[test]
    fn set_read_only_property_is_illegal() {
        let (map, info, mut state) = fixture();
        // The device UID is a known property, but read-only.
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::DEVICE_UID),
                b"whatever",
            ),
            Err(OsStatus::ILLEGAL_OPERATION)
        );
    }

    #[test]
    fn set_rejects_unknown_object_and_property() {
        let (map, info, mut state) = fixture();
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                AudioObjectId::from_u32(999),
                &PropertyAddress::global(PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE),
                &48_000.0_f64.to_ne_bytes(),
            ),
            Err(OsStatus::BAD_OBJECT)
        );
        assert_eq!(
            set_property_data(
                &map,
                &info,
                &mut state,
                map.device_id(),
                &PropertyAddress::global(PropertySelector::new(*b"zzzz")),
                &48_000.0_f64.to_ne_bytes(),
            ),
            Err(OsStatus::UNKNOWN_PROPERTY)
        );
    }
}
