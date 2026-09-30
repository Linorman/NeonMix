//! User-facing macros.
//!
//! The only macro for now is [`plugin_entry!`](crate::plugin_entry),
//! which emits the `#[no_mangle] extern "C"` CFPlugIn factory
//! function `coreaudiod` resolves to instantiate the plug-in. It is
//! the macOS counterpart of `tympan-apo`'s `register_apo!` and
//! `tympan-ladspa`'s `plugin_entry!`.

/// Expose a `T: Driver` implementation as the AudioServerPlugin this
/// `.driver` bundle provides.
///
/// Emits, in the calling crate's root:
///
/// - A hidden `__tympan_aspl_create` constructor that yields an
///   `Arc<dyn AnyDriver>` over [`DriverInstance<T>`](crate::driver::DriverInstance).
/// - A `#[no_mangle] pub unsafe extern "C"` factory function — by
///   default named `TympanAsplDriverFactory` — that forwards to
///   `raw::driver_factory_dispatch`
///   with that constructor.
///
/// The factory function name is the symbol the bundle `Info.plist`'s
/// `CFPlugInFactories` dictionary must point at — pass the same name
/// to [`BundleConfig`](crate::bundle::BundleConfig). An optional
/// second argument overrides the default symbol name.
///
/// ## Usage
///
/// ```ignore
/// use tympan_aspl::{Driver, DeviceSpec, IoBuffer, RealtimeContext};
///
/// struct MyDriver;
/// impl Driver for MyDriver {
///     const NAME: &'static str = "My Driver";
///     const MANUFACTURER: &'static str = "Me";
///     const VERSION: &'static str = "0.1.0";
///     fn new() -> Self { Self }
///     fn device(&self) -> DeviceSpec {
///         DeviceSpec::new("com.me.mydriver", "My Driver", "Me")
///     }
///     fn process_io(&self, _rt: &RealtimeContext, buf: &mut IoBuffer<'_>) {
///         buf.silence_output();
///     }
/// }
///
/// // Default factory symbol name (`TympanAsplDriverFactory`):
/// tympan_aspl::plugin_entry!(MyDriver);
///
/// // …or an explicit one, matched in the bundle Info.plist:
/// // tympan_aspl::plugin_entry!(MyDriver, MyDriverFactory);
/// ```
///
/// ## Single call per crate
///
/// The macro emits a `#[no_mangle]` symbol, which must be unique in
/// the link tree. Each `.driver` cdylib invokes `plugin_entry!`
/// exactly once at its crate root.
///
/// ## Cross-platform expansion
///
/// The macro expands on every platform: the CFPlugIn factory it
/// emits forwards to `raw::driver_factory_dispatch`, and the `raw`
/// module is ordinary cross-platform Rust — an `extern "C"`
/// function merely *uses* the C calling convention. So a driver
/// crate invoking `plugin_entry!` builds and unit-tests on any
/// host; only loading the resulting `.driver` bundle into
/// `coreaudiod` is macOS-specific.
#[macro_export]
macro_rules! plugin_entry {
    ($driver:ty) => {
        $crate::plugin_entry!($driver, TympanAsplDriverFactory);
    };
    ($driver:ty, $factory:ident) => {
        /// Constructor routed into
        /// `raw::driver_factory_dispatch`.
        #[doc(hidden)]
        fn __tympan_aspl_create() -> ::std::sync::Arc<dyn $crate::driver::AnyDriver> {
            ::std::sync::Arc::new($crate::driver::DriverInstance::<$driver>::new())
        }

        /// CFPlugIn factory entry point.
        ///
        /// # Safety
        ///
        /// Called by `coreaudiod` across the C ABI; the arguments
        /// follow the CFPlugIn factory function signature
        /// (`CFAllocatorRef`, `CFUUIDRef`).
        #[no_mangle]
        pub unsafe extern "C" fn $factory(
            allocator: *const ::core::ffi::c_void,
            requested_type_uuid: *const ::core::ffi::c_void,
        ) -> *mut ::core::ffi::c_void {
            // Safety: forwarded verbatim from the CFPlugIn loader.
            unsafe {
                $crate::raw::driver_factory_dispatch(
                    allocator,
                    requested_type_uuid,
                    __tympan_aspl_create,
                )
            }
        }
    };
}
