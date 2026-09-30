//! Low-level FFI to the Core Audio AudioServerPlugin C ABI.
//!
//! This module is the sole place that links `CoreFoundation` and
//! `CoreAudio` and the sole owner of the CFPlugIn `IUnknown`-style
//! vtable bookkeeping required to expose Rust types as a plug-in
//! `coreaudiod` can load.
//!
//! It is split so that the bulk of it *can* be tested off macOS —
//! an `extern "C"` function is ordinary Rust that merely uses the C
//! calling convention, and every type that crosses the ABI is a
//! plain pointer or scalar:
//!
//! - [`abi`] — hand-written `#[repr(C)]` mirrors of the
//!   AudioServerPlugin C structs and the
//!   `AudioServerPlugInDriverInterface` vtable.
//! - [`marshal`] — conversions between the [`abi`] C types and the
//!   framework's safe Rust types.
//! - [`runtime`] — the [`runtime::DriverObject`]
//!   CFPlugIn object layout and the
//!   [`runtime::DriverRuntime`] it carries.
//! - [`entry`] — the `extern "C"` entry points the HAL calls
//!   through the vtable.
//! - [`vtable`] — the `'static` `AudioServerPlugInDriverInterface`
//!   wiring those entry points together.
//!
//! All five compile and are unit-tested on every host. The
//! genuinely macOS-only machinery — the `#[no_mangle]` factory
//! symbol, the CoreFoundation marshalling of `CFStringRef` values,
//! the `coreaudio-sys` layout cross-checks, and the framework
//! linkage — is small and lands on top of these in a follow-up PR.
//!
//! Users of `tympan-aspl` are not expected to touch this module; the
//! public API in the crate root wraps it. It is `pub` only for the
//! framework's own [`plugin_entry!`](crate::plugin_entry) macro and
//! for advanced users who need to bypass the higher-level
//! abstractions.

pub mod abi;
pub mod clock;
pub mod entry;
pub mod marshal;
pub(crate) mod output_controls;
pub mod platform;
pub mod ring;
pub mod runtime;
pub(crate) mod trace;
pub mod vtable;

/// Minimal CoreFoundation FFI for the text-property `CFStringRef`
/// path. macOS-only — there is no CoreFoundation elsewhere.
#[cfg(target_os = "macos")]
pub mod cf;

use std::os::raw::c_void;
use std::sync::Arc;

use crate::driver::AnyDriver;
use crate::raw::runtime::{DriverObject, DriverRuntime};

/// Raw Core Audio object identifier — the C `AudioObjectID`.
///
/// The safe wrapper is [`AudioObjectId`](crate::object::AudioObjectId);
/// this alias exists for the FFI surface that will be built on top
/// of this module.
pub type RawObjectId = u32;

/// Raw Core Audio result code — the C `OSStatus`.
///
/// The safe wrapper is [`OsStatus`](crate::error::OsStatus).
pub type RawOsStatus = i32;

/// The CFPlugIn factory entry point, dispatched from the
/// [`plugin_entry!`](crate::plugin_entry) macro.
///
/// `coreaudiod` resolves the `#[no_mangle] extern "C"` factory
/// symbol the macro emits and calls it with a `CFAllocatorRef` and
/// the requested CFPlugIn type UUID; the macro forwards both, plus a
/// `create` constructor for the user's [`AnyDriver`], here.
///
/// This builds the framework's [`DriverRuntime`] around the user's
/// driver, wraps it in a heap-allocated [`DriverObject`] whose first
/// word points at the [`vtable::driver_interface`] table, and hands
/// the HAL the raw pointer — an `AudioServerPlugInDriverRef`. The
/// HAL owns that single reference; its final `Release` (slot 4 of
/// the vtable) frees the object.
///
/// # Safety
///
/// Called by `coreaudiod` across the C ABI. `allocator` and
/// `requested_type_uuid` are the `CFAllocatorRef` and `CFUUIDRef`
/// the loader passes; both may be null. Neither is dereferenced —
/// the framework registers under exactly one CFPlugIn type, so the
/// requested type is not re-checked here — so any pointer value is
/// sound.
pub unsafe fn driver_factory_dispatch(
    allocator: *const c_void,
    requested_type_uuid: *const c_void,
    create: fn() -> Arc<dyn AnyDriver>,
) -> *mut c_void {
    // The bundle's `Info.plist` scopes this factory to the
    // AudioServerPlugin type UUID, so the requested type need not be
    // re-checked; the allocator is the HAL's and we do not allocate
    // through it.
    let _ = (allocator, requested_type_uuid);
    trace::record(c"factory", 1, 0, 0);

    let runtime = DriverRuntime::new(create());
    let object = Box::new(DriverObject::new(vtable::driver_interface(), runtime));
    // Hand the HAL ownership of the single reference the
    // `DriverObject` started with; `entry::release` reclaims the
    // `Box` when the count returns to zero.
    Box::into_raw(object).cast::<c_void>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{Driver, DriverInstance};
    use crate::raw::abi::AudioServerPlugInDriverRef;
    use crate::{DeviceSpec, IoBuffer, RealtimeContext};

    struct NullDriver;

    impl Driver for NullDriver {
        const NAME: &'static str = "tympan-aspl raw factory driver";
        const MANUFACTURER: &'static str = "tympan-aspl";
        const VERSION: &'static str = "0.0.0";

        fn new() -> Self {
            Self
        }

        fn device(&self) -> DeviceSpec {
            DeviceSpec::new("com.tympan.test.raw", "Raw Factory", Self::MANUFACTURER)
        }

        fn process_io(&self, _rt: &RealtimeContext, _buffer: &mut IoBuffer<'_>) {}
    }

    fn create() -> Arc<dyn AnyDriver> {
        Arc::new(DriverInstance::<NullDriver>::new())
    }

    #[test]
    fn factory_dispatch_builds_a_live_driver_object() {
        // Safety: the factory dereferences neither pointer argument,
        // so null is sound for both.
        let ptr = unsafe { driver_factory_dispatch(core::ptr::null(), core::ptr::null(), create) };
        assert!(!ptr.is_null());

        // The returned pointer is a live `DriverObject`: its first
        // word is the framework's vtable, and the runtime carries the
        // user driver.
        let driver_ref = ptr.cast::<*const abi::AudioServerPlugInDriverInterface>();
        // Safety: `ptr` came straight from `driver_factory_dispatch`.
        let object = unsafe { DriverObject::from_ref(driver_ref) }.unwrap();
        assert_eq!(object.refcount().count(), 1);
        assert!(core::ptr::eq(
            // Safety: the vtable pointer is the object's first field.
            unsafe { *driver_ref },
            vtable::driver_interface(),
        ));
        assert_eq!(
            object.runtime().info().name,
            "tympan-aspl raw factory driver"
        );

        // Mirror the HAL's final `Release` so the object is freed.
        // Safety: the factory handed us the single owning reference.
        let remaining = unsafe { entry::release(ptr) };
        assert_eq!(remaining, 0);
    }

    #[test]
    fn factory_dispatch_drives_a_full_lifecycle_through_the_vtable() {
        // Safety: null is sound for both pointer arguments.
        let ptr = unsafe { driver_factory_dispatch(core::ptr::null(), core::ptr::null(), create) };
        let driver_ref: AudioServerPlugInDriverRef = ptr.cast();

        // Safety: `driver_ref` is the live object the factory built;
        // the entry points uphold their documented contracts.
        unsafe {
            assert_eq!(
                entry::initialize(driver_ref, core::ptr::null_mut()),
                crate::OsStatus::OK.as_i32()
            );
            assert_eq!(
                entry::start_io(driver_ref, 2, 0),
                crate::OsStatus::OK.as_i32()
            );
            assert_eq!(
                entry::stop_io(driver_ref, 2, 0),
                crate::OsStatus::OK.as_i32()
            );
            assert_eq!(entry::release(ptr), 0);
        }
    }

    #[test]
    fn raw_aliases_match_safe_wrapper_widths() {
        use core::mem::size_of;
        assert_eq!(size_of::<RawObjectId>(), size_of::<crate::AudioObjectId>());
        assert_eq!(size_of::<RawOsStatus>(), size_of::<crate::OsStatus>());
    }
}
