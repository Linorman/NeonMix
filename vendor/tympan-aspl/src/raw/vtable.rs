//! The framework's `AudioServerPlugInDriverInterface` vtable.
//!
//! [`driver_interface`] returns a reference to the one `'static`
//! [`AudioServerPlugInDriverInterface`] the framework hands every
//! plug-in instance. Its slots point at the `extern "C"` entry
//! points in [`crate::raw::entry`]; the factory
//! ([`crate::raw::driver_factory_dispatch`]) stores a pointer to
//! this table as the first word of every [`DriverObject`], which is
//! what makes a `*mut DriverObject` a valid
//! [`AudioServerPlugInDriverRef`].
//!
//! ## Populated slots
//!
//! This table wires the `IUnknown` preamble, `Initialize`, the
//! device-client hooks, the full property protocol, `StartIO` /
//! `StopIO`, and the entire IO cycle — the clock callbacks, the
//! per-operation brackets, and `DoIOOperation`, the realtime data
//! movement. The only `None` slots are `CreateDevice` /
//! `DestroyDevice` (the framework's device is static, not
//! HAL-created) and the configuration-change pair.
//!
//! Building the table needs no FFI, so its shape is unit-tested on
//! any host.
//!
//! [`DriverObject`]: crate::raw::runtime::DriverObject
//! [`AudioServerPlugInDriverRef`]: crate::raw::abi::AudioServerPlugInDriverRef

use crate::raw::abi::AudioServerPlugInDriverInterface;
use crate::raw::entry;

/// A `Sync` wrapper so the vtable can live in a `static`.
struct StaticInterface(AudioServerPlugInDriverInterface);

// Safety: the only non-`Sync` field of
// `AudioServerPlugInDriverInterface` is `_reserved`, a
// `*const c_void` the framework holds as null and never mutates or
// dereferences. Every other field is an `Option<extern "C" fn>`,
// which is `Sync`. Sharing the table across threads is therefore
// sound — which is exactly what a `static` requires.
unsafe impl Sync for StaticInterface {}

/// The single `'static` driver vtable, shared by every plug-in
/// instance the framework creates.
static DRIVER_INTERFACE: StaticInterface = StaticInterface(AudioServerPlugInDriverInterface {
    // `IUNKNOWN_C_GUTS`: the reserved slot is a null data pointer;
    // the three IUnknown methods follow.
    _reserved: core::ptr::null(),
    QueryInterface: Some(entry::query_interface),
    AddRef: Some(entry::add_ref),
    Release: Some(entry::release),

    Initialize: Some(entry::initialize),
    // The framework exposes a single static device declared by the
    // driver, so it never creates or destroys devices on HAL
    // request.
    CreateDevice: Some(entry::create_device),
    DestroyDevice: Some(entry::destroy_device),
    AddDeviceClient: Some(entry::add_device_client),
    RemoveDeviceClient: Some(entry::remove_device_client),
    // Deferred configuration changes are not yet modelled.
    PerformDeviceConfigurationChange: Some(entry::perform_device_configuration_change),
    AbortDeviceConfigurationChange: Some(entry::abort_device_configuration_change),

    HasProperty: Some(entry::has_property),
    IsPropertySettable: Some(entry::is_property_settable),
    GetPropertyDataSize: Some(entry::get_property_data_size),
    GetPropertyData: Some(entry::get_property_data),
    SetPropertyData: Some(entry::set_property_data),

    StartIO: Some(entry::start_io),
    StopIO: Some(entry::stop_io),

    // IO-cycle: the device clock, the per-operation brackets, and
    // the realtime data movement. `DoIOOperation` reads and writes
    // the device's audio ring; `WillDoIOOperation` tells the HAL it
    // handles `ReadInput` and `WriteMix`.
    GetZeroTimeStamp: Some(entry::get_zero_time_stamp),
    WillDoIOOperation: Some(entry::will_do_io_operation),
    BeginIOOperation: Some(entry::begin_io_operation),
    DoIOOperation: Some(entry::do_io_operation),
    EndIOOperation: Some(entry::end_io_operation),
});

/// A reference to the framework's `'static` driver vtable.
///
/// The factory stores this pointer as the first word of every
/// [`DriverObject`](crate::raw::runtime::DriverObject); the HAL then
/// calls through it.
#[must_use]
pub fn driver_interface() -> &'static AudioServerPlugInDriverInterface {
    &DRIVER_INTERFACE.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iunknown_preamble_is_wired() {
        let vtable = driver_interface();
        assert!(vtable._reserved.is_null());
        assert!(vtable.QueryInterface.is_some());
        assert!(vtable.AddRef.is_some());
        assert!(vtable.Release.is_some());
    }

    #[test]
    fn lifecycle_property_and_io_timing_slots_are_wired() {
        let vtable = driver_interface();
        assert!(vtable.Initialize.is_some());
        assert!(vtable.AddDeviceClient.is_some());
        assert!(vtable.RemoveDeviceClient.is_some());
        assert!(vtable.HasProperty.is_some());
        assert!(vtable.IsPropertySettable.is_some());
        assert!(vtable.GetPropertyDataSize.is_some());
        assert!(vtable.GetPropertyData.is_some());
        assert!(vtable.SetPropertyData.is_some());
        assert!(vtable.StartIO.is_some());
        assert!(vtable.StopIO.is_some());
        // The full IO cycle — clock, brackets, and the realtime
        // data movement.
        assert!(vtable.GetZeroTimeStamp.is_some());
        assert!(vtable.WillDoIOOperation.is_some());
        assert!(vtable.BeginIOOperation.is_some());
        assert!(vtable.DoIOOperation.is_some());
        assert!(vtable.EndIOOperation.is_some());
    }

    #[test]
    fn unsupported_operations_have_callable_slots() {
        let vtable = driver_interface();
        // Static-device plug-in: no HAL-driven device lifecycle.
        assert!(vtable.CreateDevice.is_some());
        assert!(vtable.DestroyDevice.is_some());
        assert!(vtable.PerformDeviceConfigurationChange.is_some());
        assert!(vtable.AbortDeviceConfigurationChange.is_some());
    }

    #[test]
    fn the_table_is_a_single_shared_instance() {
        // Two calls hand back the same `'static` table.
        assert!(core::ptr::eq(driver_interface(), driver_interface()));
    }
}
