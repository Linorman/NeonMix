//! Opt-in, bounded control-property diagnostics. Never called by data/clock callbacks.
#[cfg(all(target_os = "macos", feature = "trace-properties"))]
pub(crate) fn record(event: &core::ffi::CStr, object: u32, selector: u32, value: u32) {
    use core::sync::atomic::{AtomicU32, Ordering};
    static BUDGET: AtomicU32 = AtomicU32::new(96);
    if BUDGET
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_err()
    {
        return;
    }
    extern "C" {
        fn syslog(priority: i32, format: *const core::ffi::c_char, ...);
    }
    // SAFETY: fixed printf format matches the borrowed C string and UInt32 arguments.
    unsafe {
        syslog(
            5,
            c"NeonMix HAL %s object=%u selector=0x%08x value=%u".as_ptr(),
            event.as_ptr(),
            object,
            selector,
            value,
        );
    }
}
#[cfg(not(all(target_os = "macos", feature = "trace-properties")))]
pub(crate) fn record(_event: &core::ffi::CStr, _object: u32, _selector: u32, _value: u32) {}
