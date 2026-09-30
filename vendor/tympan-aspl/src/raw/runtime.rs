//! The CFPlugIn object layout and the framework runtime it carries.
//!
//! `coreaudiod` loads an AudioServerPlugin as a CFPlugIn: it calls
//! the bundle's factory function, which returns a pointer to an
//! object whose first word is a pointer to a vtable
//! ([`AudioServerPlugInDriverInterface`]). [`DriverObject`] is that
//! object — its first field is the vtable pointer, so a
//! `*mut DriverObject` *is* an [`AudioServerPlugInDriverRef`](crate::raw::abi::AudioServerPlugInDriverRef).
//!
//! Behind the vtable pointer sits the [`DriverRuntime`]: the user's
//! [`AnyDriver`], the [`ObjectMap`] built from its device, the
//! cached [`DriverInfo`], and the mutable [`DeviceState`]. The
//! `extern "C"` entry points in [`crate::raw::entry`] recover
//! `&DriverObject` from the ref the HAL hands them and operate on
//! this runtime.
//!
//! Everything here is cross-platform: the layout is plain
//! `#[repr(C)]` data and the runtime is plain Rust, so the
//! object-recovery and refcount logic is unit-tested on any host.
//! Only the *factory* that allocates a `DriverObject` and the
//! framework linkage are macOS-specific (a later PR).

use core::sync::atomic::{AtomicPtr, Ordering};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use crate::dispatch::DeviceState;
use crate::driver::{AnyDriver, DriverInfo};
use crate::objects::ObjectMap;
use crate::raw::abi::{AudioServerPlugInDriverInterface, AudioServerPlugInHostRef};
use crate::raw::clock::DeviceClock;
use crate::raw::ring::DeviceRing;
use crate::realtime::Refcount;

/// The framework state behind a [`DriverObject`]'s vtable pointer.
///
/// Built once, when the factory instantiates the plug-in, and
/// thereafter shared (by `&`) across every HAL thread that calls an
/// entry point. The HAL serialises its non-realtime calls, but the
/// type is still `Sync`: `driver` is an `Arc`, `state` is behind a
/// `Mutex`, and `host` is an `AtomicPtr`.
pub struct DriverRuntime {
    /// The user's driver, type-erased.
    driver: Arc<dyn AnyDriver>,
    /// The object tree the HAL walks via the property protocol.
    objects: ObjectMap,
    /// The driver's identity constants, cached for the plug-in
    /// property queries.
    info: DriverInfo,
    /// The device's mutable runtime state (sample rate, running
    /// flag). The HAL serialises property and lifecycle calls, but
    /// the `Mutex` keeps the type `Sync` and guards against a
    /// poisoned-state read.
    state: Mutex<DeviceState>,
    /// The host interface `coreaudiod` hands the plug-in in
    /// `Initialize`, or null before then. Stored for the
    /// property-changed notification path (a later PR); written
    /// once, read never (yet).
    host: AtomicPtr<c_void>,
    /// The device's synthetic zero-timestamp clock. `StartIO`
    /// anchors it, `GetZeroTimeStamp` reads it, `StopIO` halts it.
    clock: DeviceClock,
    /// The device's audio backing store — what makes a loopback
    /// loop. Allocated here, off the realtime path; `DoIOOperation`
    /// reads and writes it lock-free.
    ring: DeviceRing,
    controls: super::output_controls::OutputControls,
    io_clients: Mutex<Vec<u32>>,
    host_ticks_per_period: f64,
}

impl DriverRuntime {
    /// Build the runtime for `driver`: materialise its object tree,
    /// cache its identity, seed the device state, and allocate the
    /// device's audio ring.
    ///
    /// The ring holds one zero-timestamp period — one second at the
    /// device's nominal sample rate — of audio, wide enough for the
    /// device's busiest stream.
    #[must_use]
    pub fn new(driver: Arc<dyn AnyDriver>) -> Self {
        let spec = driver.device();
        let objects = ObjectMap::new(spec);
        let info = driver.info();
        let state = Mutex::new(DeviceState::from_spec(objects.spec()));

        // One second of audio, sized for whichever stream carries
        // the most channels; `max(1)` keeps a streamless device's
        // ring construction from panicking.
        let capacity_frames = (spec.sample_rate() as usize).max(1);
        let input_channels = spec.input().map_or(0, |stream| stream.channels() as usize);
        let output_channels = spec.output().map_or(0, |stream| stream.channels() as usize);
        let channels = input_channels.max(output_channels).max(1);

        Self {
            driver,
            objects,
            info,
            state,
            host: AtomicPtr::new(core::ptr::null_mut()),
            clock: DeviceClock::new(),
            ring: DeviceRing::new(capacity_frames, channels),
            controls: super::output_controls::OutputControls::new(spec.name()),
            io_clients: Mutex::new(Vec::new()),
            host_ticks_per_period: super::platform::host_ticks_per_period(
                spec.sample_rate(),
                spec.sample_rate(),
            ),
        }
    }

    /// The user's type-erased driver.
    #[inline]
    #[must_use]
    pub fn driver(&self) -> &Arc<dyn AnyDriver> {
        &self.driver
    }

    /// The object tree.
    #[inline]
    #[must_use]
    pub fn objects(&self) -> &ObjectMap {
        &self.objects
    }

    /// The driver's identity constants.
    #[inline]
    #[must_use]
    pub fn info(&self) -> &DriverInfo {
        &self.info
    }

    /// Lock the device state. On a poisoned mutex the guard is
    /// recovered rather than panicking — an entry point must never
    /// unwind across the C ABI.
    pub fn state(&self) -> std::sync::MutexGuard<'_, DeviceState> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Record the host interface `coreaudiod` supplied in
    /// `Initialize`. `Release` ordering pairs with the future
    /// `Acquire` load on the notification path.
    pub fn set_host(&self, host: AudioServerPlugInHostRef) {
        self.host.store(host.cast(), Ordering::Release);
    }

    /// The host interface, or null if `Initialize` has not run.
    #[must_use]
    pub fn host(&self) -> AudioServerPlugInHostRef {
        self.host.load(Ordering::Acquire).cast()
    }

    /// The device's zero-timestamp clock.
    #[inline]
    #[must_use]
    pub fn clock(&self) -> &DeviceClock {
        &self.clock
    }

    /// The device's audio backing store.
    #[inline]
    #[must_use]
    pub fn ring(&self) -> &DeviceRing {
        &self.ring
    }

    pub(crate) fn output_controls(&self) -> &super::output_controls::OutputControls {
        &self.controls
    }

    pub(crate) fn host_ticks_per_period(&self) -> f64 {
        self.host_ticks_per_period
    }

    pub(crate) fn io_clients(&self) -> std::sync::MutexGuard<'_, Vec<u32>> {
        self.io_clients
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// The CFPlugIn object `coreaudiod` holds a plug-in through.
///
/// `#[repr(C)]` with the vtable pointer **first** is load-bearing:
/// the HAL's [`AudioServerPlugInDriverRef`](crate::raw::abi::AudioServerPlugInDriverRef) is a
/// `*mut *const AudioServerPlugInDriverInterface`, and that is
/// exactly a pointer to this struct's first field. The factory
/// heap-allocates one of these and hands the HAL a pointer to it;
/// [`DriverObject::from_ref`] performs the inverse.
#[repr(C)]
pub struct DriverObject {
    /// The vtable pointer. **Must stay the first field.** Points at
    /// the `'static` interface table built by
    /// [`crate::raw::vtable`].
    vtable: *const AudioServerPlugInDriverInterface,
    /// The CFPlugIn `IUnknown` reference count. Starts at `1` — the
    /// reference the factory hands back to the HAL.
    refcount: Refcount,
    /// The framework runtime behind the vtable.
    runtime: DriverRuntime,
}

// Safety: `vtable` points at a `'static`, immutable interface table
// — sending that pointer to another thread is sound. `refcount` is
// atomic and `runtime` is `Send` by construction (Arc + Mutex +
// AtomicPtr).
unsafe impl Send for DriverObject {}
// Safety: as above — the vtable pointer is to `'static` immutable
// data, and every other field is `Sync` (atomics + a `Mutex`). The
// HAL additionally serialises its non-realtime calls.
unsafe impl Sync for DriverObject {}

impl DriverObject {
    /// Construct a `DriverObject` around `runtime`, pointing at
    /// `vtable`, with the CFPlugIn refcount at its initial `1`.
    ///
    /// The factory wraps the result in a `Box` and hands the raw
    /// pointer to the HAL; tests construct one directly.
    #[must_use]
    pub fn new(vtable: &'static AudioServerPlugInDriverInterface, runtime: DriverRuntime) -> Self {
        Self {
            vtable,
            refcount: Refcount::new(),
            runtime,
        }
    }

    /// The framework runtime behind the vtable.
    #[inline]
    #[must_use]
    pub fn runtime(&self) -> &DriverRuntime {
        &self.runtime
    }

    /// The CFPlugIn reference count.
    #[inline]
    #[must_use]
    pub fn refcount(&self) -> &Refcount {
        &self.refcount
    }

    /// Recover a `&DriverObject` from the
    /// [`AudioServerPlugInDriverRef`](crate::raw::abi::AudioServerPlugInDriverRef)
    /// the HAL passes to an entry point, or `None` if the ref is
    /// null.
    ///
    /// # Safety
    ///
    /// `driver` must either be null or a pointer the framework's
    /// factory produced — i.e. a live `*mut DriverObject`. The HAL
    /// upholds this for every entry-point call: the ref it passes is
    /// the one the factory returned. The returned reference borrows
    /// the object for `'a`; the caller must not let it outlive the
    /// HAL's ownership of the object.
    #[inline]
    #[must_use]
    pub unsafe fn from_ref<'a>(
        driver: crate::raw::abi::AudioServerPlugInDriverRef,
    ) -> Option<&'a DriverObject> {
        // The ref is `*mut *const Interface`; the `DriverObject`'s
        // first field is exactly that `*const Interface`, so the ref
        // reinterprets as `*const DriverObject`.
        // Safety: forwarded from the caller's contract.
        unsafe { driver.cast::<DriverObject>().as_ref() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{Driver, DriverInstance};
    use crate::raw::abi::AudioServerPlugInDriverRef;
    use crate::realtime::State;
    use crate::{DeviceSpec, IoBuffer, RealtimeContext, StreamFormat, StreamSpec};

    struct Loopback;

    impl Driver for Loopback {
        const NAME: &'static str = "tympan-aspl runtime fixture";
        const MANUFACTURER: &'static str = "tympan-aspl";
        const VERSION: &'static str = "0.0.0";

        fn new() -> Self {
            Self
        }

        fn device(&self) -> DeviceSpec {
            let format = StreamFormat::float32(48_000.0, 2);
            DeviceSpec::new(
                "com.tympan.test.runtime",
                "Runtime Fixture",
                Self::MANUFACTURER,
            )
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
        }

        fn process_io(&self, _rt: &RealtimeContext, _buffer: &mut IoBuffer<'_>) {}
    }

    fn runtime() -> DriverRuntime {
        DriverRuntime::new(Arc::new(DriverInstance::<Loopback>::new()))
    }

    // The framework's real `'static` vtable — these tests only need
    // *a* valid `&'static` table to point a `DriverObject` at.
    fn vtable() -> &'static AudioServerPlugInDriverInterface {
        crate::raw::vtable::driver_interface()
    }

    #[test]
    fn runtime_materialises_the_object_tree_and_identity() {
        let rt = runtime();
        assert_eq!(rt.info().name, "tympan-aspl runtime fixture");
        assert!(rt.objects().spec().is_loopback());
        assert_eq!(rt.state().sample_rate, 48_000.0);
        assert!(!rt.state().running);
        assert_eq!(rt.driver().state(), State::Uninitialized);
    }

    #[test]
    fn host_pointer_round_trips() {
        let rt = runtime();
        assert!(rt.host().is_null());
        let fake_host = 0xDEAD_BEEF_usize as *mut c_void;
        rt.set_host(fake_host);
        assert_eq!(rt.host(), fake_host);
    }

    #[test]
    fn state_lock_is_mutable() {
        let rt = runtime();
        rt.state().sample_rate = 96_000.0;
        assert_eq!(rt.state().sample_rate, 96_000.0);
    }

    #[test]
    fn driver_object_starts_with_refcount_one() {
        let object = DriverObject::new(vtable(), runtime());
        assert_eq!(object.refcount().count(), 1);
    }

    #[test]
    fn from_ref_recovers_the_object() {
        // The factory's path: box the object, hand out the raw
        // pointer, recover it.
        let object = Box::new(DriverObject::new(vtable(), runtime()));
        let raw: *mut DriverObject = Box::into_raw(object);
        let driver_ref = raw.cast::<*const AudioServerPlugInDriverInterface>();

        // Safety: `driver_ref` came from a live `Box<DriverObject>`.
        let recovered = unsafe { DriverObject::from_ref(driver_ref) }.unwrap();
        assert_eq!(recovered.refcount().count(), 1);
        assert_eq!(
            recovered.runtime().info().name,
            "tympan-aspl runtime fixture"
        );

        // Reclaim the box so the test does not leak.
        // Safety: `raw` is the pointer `Box::into_raw` produced and
        // it has not been freed.
        drop(unsafe { Box::from_raw(raw) });
    }

    #[test]
    fn from_ref_rejects_null() {
        // Safety: null is explicitly allowed by the contract.
        let recovered =
            unsafe { DriverObject::from_ref(core::ptr::null_mut() as AudioServerPlugInDriverRef) };
        assert!(recovered.is_none());
    }

    #[test]
    fn driver_object_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DriverObject>();
        assert_send_sync::<DriverRuntime>();
    }
}
