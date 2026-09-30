//! The AudioServerPlugin object tree.
//!
//! Core Audio addresses everything a plug-in exposes by
//! [`AudioObjectId`]. The HAL never sees the driver's Rust types
//! directly — it walks an object *tree* through the property
//! protocol: it asks the plug-in object for its device list, asks
//! each device for its streams, and so on.
//!
//! [`ObjectMap`] is that tree, materialised from a driver's
//! [`DeviceSpec`]. It assigns a stable [`AudioObjectId`] to the
//! plug-in, the device, and each stream, and answers the two
//! questions the property dispatcher needs:
//!
//! - *what* is this id? — [`ObjectMap::resolve`]
//! - *what does this id own?* — [`ObjectMap::owned_objects`]
//!
//! The map is cross-platform plain data: building it and walking it
//! needs no FFI, so the object-tree logic is unit-testable on any
//! host.
//!
//! ## Id assignment
//!
//! A driver exposes a single device (see [`Driver::device`]), so the
//! tree is small and its ids are fixed:
//!
//! | Id | Object |
//! |---:|---|
//! | `1` | the plug-in ([`AudioObjectId::PLUGIN`]) |
//! | `2` | the device |
//! | `3`… | the streams, in input-then-output order, present ones only |
//!
//! [`Driver`]: crate::Driver
//! [`Driver::device`]: crate::Driver::device

extern crate alloc;

use alloc::vec::Vec;

use crate::device::DeviceSpec;
use crate::object::{AudioObjectId, ObjectKind};
use crate::property::PropertyScope;
use crate::stream::{StreamDirection, StreamSpec};

/// What an [`AudioObjectId`] resolves to within an [`ObjectMap`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Object {
    /// The plug-in object itself ([`AudioObjectId::PLUGIN`]).
    PlugIn,
    /// The driver's device.
    Device,
    /// One of the device's streams.
    Stream(StreamDirection),
}

impl Object {
    /// The [`ObjectKind`] — and therefore the `kAudio*ClassID` — of
    /// this object.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> ObjectKind {
        match self {
            Self::PlugIn => ObjectKind::PlugIn,
            Self::Device => ObjectKind::Device,
            Self::Stream(_) => ObjectKind::Stream,
        }
    }
}

/// The object tree for one driver, materialised from its
/// [`DeviceSpec`].
///
/// Holds the spec plus the [`AudioObjectId`]s assigned to the
/// device and its streams. Construct it with [`ObjectMap::new`]; the
/// framework builds one when the HAL first asks the plug-in for its
/// object tree, and the property dispatcher consults it on every
/// property call.
#[derive(Clone, PartialEq, Debug)]
pub struct ObjectMap {
    spec: DeviceSpec,
    device_id: AudioObjectId,
    input_stream_id: Option<AudioObjectId>,
    output_stream_id: Option<AudioObjectId>,
}

impl ObjectMap {
    /// Build the object tree for `spec`, assigning ids from
    /// [`AudioObjectId::FIRST_DYNAMIC`] onwards.
    ///
    /// The device takes the first id; the streams follow in
    /// input-then-output order, skipping a direction the device does
    /// not have.
    #[must_use]
    pub fn new(spec: DeviceSpec) -> Self {
        let mut next = AudioObjectId::FIRST_DYNAMIC.as_u32();
        let device_id = AudioObjectId::from_u32(next);
        next += 1;

        let input_stream_id = spec.input().map(|_| {
            let id = AudioObjectId::from_u32(next);
            next += 1;
            id
        });
        let output_stream_id = spec.output().map(|_| AudioObjectId::from_u32(next));

        Self {
            spec,
            device_id,
            input_stream_id,
            output_stream_id,
        }
    }

    /// The driver's [`DeviceSpec`].
    #[inline]
    #[must_use]
    pub fn spec(&self) -> &DeviceSpec {
        &self.spec
    }

    /// The plug-in object's id — always [`AudioObjectId::PLUGIN`].
    #[inline]
    #[must_use]
    pub const fn plugin_id(&self) -> AudioObjectId {
        AudioObjectId::PLUGIN
    }

    /// The device object's id.
    #[inline]
    #[must_use]
    pub const fn device_id(&self) -> AudioObjectId {
        self.device_id
    }

    /// The id of the stream carrying `direction`, or `None` if the
    /// device has no such stream.
    #[inline]
    #[must_use]
    pub const fn stream_id(&self, direction: StreamDirection) -> Option<AudioObjectId> {
        match direction {
            StreamDirection::Input => self.input_stream_id,
            StreamDirection::Output => self.output_stream_id,
        }
    }

    /// The [`StreamSpec`] for `direction`, or `None` if the device
    /// has no such stream.
    #[inline]
    #[must_use]
    pub fn stream_spec(&self, direction: StreamDirection) -> Option<StreamSpec> {
        self.spec.stream(direction)
    }

    /// Resolve an [`AudioObjectId`] to the [`Object`] it names, or
    /// `None` if the id is not part of this tree.
    #[must_use]
    pub fn resolve(&self, id: AudioObjectId) -> Option<Object> {
        if id == AudioObjectId::PLUGIN {
            return Some(Object::PlugIn);
        }
        if id == self.device_id {
            return Some(Object::Device);
        }
        if Some(id) == self.input_stream_id {
            return Some(Object::Stream(StreamDirection::Input));
        }
        if Some(id) == self.output_stream_id {
            return Some(Object::Stream(StreamDirection::Output));
        }
        None
    }

    /// `true` iff `id` is part of this tree.
    #[inline]
    #[must_use]
    pub fn contains(&self, id: AudioObjectId) -> bool {
        self.resolve(id).is_some()
    }

    /// Every [`AudioObjectId`] in the tree, in tree order: plug-in,
    /// device, then streams.
    #[must_use]
    pub fn all_ids(&self) -> Vec<AudioObjectId> {
        let mut ids = Vec::with_capacity(4);
        ids.push(self.plugin_id());
        ids.push(self.device_id);
        ids.extend(self.input_stream_id);
        ids.extend(self.output_stream_id);
        ids
    }

    /// The objects `id` owns (its children in the tree), filtered to
    /// `scope`.
    ///
    /// - The plug-in owns the device. (The plug-in's scope is always
    ///   [`PropertyScope::GLOBAL`].)
    /// - The device owns its streams; [`PropertyScope::INPUT`] /
    ///   [`PropertyScope::OUTPUT`] narrow the list to that
    ///   direction, [`PropertyScope::GLOBAL`] returns both.
    /// - A stream owns nothing.
    ///
    /// An id not in the tree, or a scope that does not apply, yields
    /// an empty list.
    #[must_use]
    pub fn owned_objects(&self, id: AudioObjectId, scope: PropertyScope) -> Vec<AudioObjectId> {
        match self.resolve(id) {
            Some(Object::PlugIn) => alloc::vec![self.device_id],
            Some(Object::Device) => self.device_streams(scope),
            Some(Object::Stream(_)) | None => Vec::new(),
        }
    }

    /// The device's stream ids, filtered to `scope`.
    ///
    /// [`PropertyScope::GLOBAL`] returns every stream (input first,
    /// then output); [`PropertyScope::INPUT`] / [`PropertyScope::OUTPUT`]
    /// return just that direction's stream. Any other scope yields an
    /// empty list.
    #[must_use]
    pub fn device_streams(&self, scope: PropertyScope) -> Vec<AudioObjectId> {
        let mut ids = Vec::with_capacity(2);
        let want_input = scope == PropertyScope::GLOBAL || scope == PropertyScope::INPUT;
        let want_output = scope == PropertyScope::GLOBAL || scope == PropertyScope::OUTPUT;
        if want_input {
            ids.extend(self.input_stream_id);
        }
        if want_output {
            ids.extend(self.output_stream_id);
        }
        ids
    }

    /// The id of the object that owns `id` (its parent in the tree):
    /// the device for a stream, the plug-in for the device,
    /// [`AudioObjectId::UNKNOWN`] for the plug-in (it has no owner),
    /// and `None` for an id not in the tree.
    #[must_use]
    pub fn owner_of(&self, id: AudioObjectId) -> Option<AudioObjectId> {
        match self.resolve(id)? {
            Object::PlugIn => Some(AudioObjectId::UNKNOWN),
            Object::Device => Some(self.plugin_id()),
            Object::Stream(_) => Some(self.device_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::StreamFormat;

    fn loopback() -> DeviceSpec {
        let format = StreamFormat::float32(48_000.0, 2);
        DeviceSpec::new("com.example.loopback", "Loopback", "tympan-aspl")
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn output_only() -> DeviceSpec {
        DeviceSpec::new("com.example.speaker", "Speaker", "tympan-aspl")
            .with_output(StreamSpec::output(StreamFormat::float32(48_000.0, 2)))
    }

    #[test]
    fn loopback_assigns_ids_in_tree_order() {
        let map = ObjectMap::new(loopback());
        assert_eq!(map.plugin_id(), AudioObjectId::PLUGIN);
        assert_eq!(map.device_id(), AudioObjectId::from_u32(2));
        assert_eq!(
            map.stream_id(StreamDirection::Input),
            Some(AudioObjectId::from_u32(3))
        );
        assert_eq!(
            map.stream_id(StreamDirection::Output),
            Some(AudioObjectId::from_u32(4))
        );
    }

    #[test]
    fn output_only_device_skips_the_input_id() {
        let map = ObjectMap::new(output_only());
        assert_eq!(map.device_id(), AudioObjectId::from_u32(2));
        assert_eq!(map.stream_id(StreamDirection::Input), None);
        // The output stream takes id 3 — the input id is not
        // reserved-and-skipped, it is simply not allocated.
        assert_eq!(
            map.stream_id(StreamDirection::Output),
            Some(AudioObjectId::from_u32(3))
        );
    }

    #[test]
    fn streamless_device_has_only_plugin_and_device() {
        let map = ObjectMap::new(DeviceSpec::new("uid", "name", "maker"));
        assert_eq!(map.stream_id(StreamDirection::Input), None);
        assert_eq!(map.stream_id(StreamDirection::Output), None);
        assert_eq!(
            map.all_ids(),
            alloc::vec![AudioObjectId::PLUGIN, AudioObjectId::from_u32(2)]
        );
    }

    #[test]
    fn resolve_maps_ids_back_to_objects() {
        let map = ObjectMap::new(loopback());
        assert_eq!(map.resolve(AudioObjectId::PLUGIN), Some(Object::PlugIn));
        assert_eq!(
            map.resolve(AudioObjectId::from_u32(2)),
            Some(Object::Device)
        );
        assert_eq!(
            map.resolve(AudioObjectId::from_u32(3)),
            Some(Object::Stream(StreamDirection::Input))
        );
        assert_eq!(
            map.resolve(AudioObjectId::from_u32(4)),
            Some(Object::Stream(StreamDirection::Output))
        );
        assert_eq!(map.resolve(AudioObjectId::from_u32(99)), None);
        assert_eq!(map.resolve(AudioObjectId::UNKNOWN), None);
    }

    #[test]
    fn contains_matches_resolve() {
        let map = ObjectMap::new(loopback());
        for id in map.all_ids() {
            assert!(map.contains(id));
        }
        assert!(!map.contains(AudioObjectId::from_u32(100)));
    }

    #[test]
    fn object_kinds_follow_the_tree() {
        assert_eq!(Object::PlugIn.kind(), ObjectKind::PlugIn);
        assert_eq!(Object::Device.kind(), ObjectKind::Device);
        assert_eq!(
            Object::Stream(StreamDirection::Input).kind(),
            ObjectKind::Stream
        );
    }

    #[test]
    fn all_ids_is_tree_ordered() {
        let map = ObjectMap::new(loopback());
        assert_eq!(
            map.all_ids(),
            alloc::vec![
                AudioObjectId::PLUGIN,
                AudioObjectId::from_u32(2),
                AudioObjectId::from_u32(3),
                AudioObjectId::from_u32(4),
            ]
        );
    }

    #[test]
    fn plugin_owns_the_device() {
        let map = ObjectMap::new(loopback());
        assert_eq!(
            map.owned_objects(AudioObjectId::PLUGIN, PropertyScope::GLOBAL),
            alloc::vec![map.device_id()]
        );
    }

    #[test]
    fn device_owns_streams_filtered_by_scope() {
        let map = ObjectMap::new(loopback());
        let dev = map.device_id();
        assert_eq!(
            map.owned_objects(dev, PropertyScope::GLOBAL),
            alloc::vec![AudioObjectId::from_u32(3), AudioObjectId::from_u32(4)]
        );
        assert_eq!(
            map.owned_objects(dev, PropertyScope::INPUT),
            alloc::vec![AudioObjectId::from_u32(3)]
        );
        assert_eq!(
            map.owned_objects(dev, PropertyScope::OUTPUT),
            alloc::vec![AudioObjectId::from_u32(4)]
        );
    }

    #[test]
    fn streams_own_nothing() {
        let map = ObjectMap::new(loopback());
        assert!(map
            .owned_objects(AudioObjectId::from_u32(3), PropertyScope::GLOBAL)
            .is_empty());
    }

    #[test]
    fn device_streams_respects_a_one_sided_device() {
        let map = ObjectMap::new(output_only());
        assert_eq!(
            map.device_streams(PropertyScope::GLOBAL),
            alloc::vec![AudioObjectId::from_u32(3)]
        );
        assert!(map.device_streams(PropertyScope::INPUT).is_empty());
        assert_eq!(
            map.device_streams(PropertyScope::OUTPUT),
            alloc::vec![AudioObjectId::from_u32(3)]
        );
    }

    #[test]
    fn owner_walks_up_the_tree() {
        let map = ObjectMap::new(loopback());
        assert_eq!(
            map.owner_of(AudioObjectId::PLUGIN),
            Some(AudioObjectId::UNKNOWN)
        );
        assert_eq!(map.owner_of(map.device_id()), Some(AudioObjectId::PLUGIN));
        assert_eq!(
            map.owner_of(AudioObjectId::from_u32(3)),
            Some(map.device_id())
        );
        assert_eq!(map.owner_of(AudioObjectId::from_u32(99)), None);
    }

    #[test]
    fn stream_spec_is_forwarded_from_the_device_spec() {
        let map = ObjectMap::new(loopback());
        assert_eq!(
            map.stream_spec(StreamDirection::Input).unwrap().direction(),
            StreamDirection::Input
        );
        assert_eq!(
            ObjectMap::new(output_only()).stream_spec(StreamDirection::Input),
            None
        );
    }
}
