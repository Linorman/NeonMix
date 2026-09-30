//! The two genuinely platform-specific primitives the IO clock
//! needs: reading the host clock, and the host clock's tick rate.
//!
//! Everything else in [`crate::raw`] is plain Rust that merely
//! *uses* the C calling convention; these two functions actually
//! call into the platform. They are isolated here, behind a uniform
//! cross-platform signature, so [`crate::raw::clock`] and
//! [`crate::raw::entry`] stay testable on any host.
//!
//! - On macOS, [`host_time_now`] is `mach_absolute_time` and
//!   [`host_ticks_per_period`] derives the period's host-tick
//!   duration from the mach timebase.
//! - Off macOS — where the IO path is never actually driven by
//!   `coreaudiod` — both are backed by [`std::time`] in plain
//!   nanoseconds, purely so the surrounding code compiles and its
//!   tests run.

#[cfg(target_os = "macos")]
pub use macos::{host_ticks_per_period, host_time_now};

#[cfg(not(target_os = "macos"))]
pub use portable::{host_ticks_per_period, host_time_now};

#[cfg(target_os = "macos")]
mod macos {
    /// `mach_timebase_info_data_t` — the rational `numer / denom`
    /// that converts mach host ticks to nanoseconds.
    #[repr(C)]
    struct MachTimebaseInfo {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        /// `mach_absolute_time` — the host's monotonic clock, in
        /// mach ticks. Exported by `libSystem`, always linked.
        fn mach_absolute_time() -> u64;
        /// `mach_timebase_info` — fills in the tick→nanosecond
        /// ratio.
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    }

    /// The host clock's current value, in mach ticks.
    #[must_use]
    pub fn host_time_now() -> u64 {
        // Safety: `mach_absolute_time` takes no arguments and only
        // reads the monotonic clock.
        unsafe { mach_absolute_time() }
    }

    /// The host-tick duration of one zero-timestamp period — a
    /// period of `period_frames` frames at `sample_rate` Hz.
    ///
    /// `period_seconds = period_frames / sample_rate`;
    /// `host_ticks = period_nanoseconds * denom / numer`.
    #[must_use]
    pub fn host_ticks_per_period(period_frames: f64, sample_rate: f64) -> f64 {
        let mut timebase = MachTimebaseInfo { numer: 0, denom: 0 };
        // Safety: `mach_timebase_info` writes the timebase ratio
        // through the pointer and reads nothing else.
        unsafe { mach_timebase_info(&mut timebase) };
        if timebase.numer == 0 || sample_rate <= 0.0 {
            return 0.0;
        }
        let period_nanoseconds = period_frames / sample_rate * 1.0e9;
        // ticks = ns / (numer/denom) = ns * denom / numer.
        period_nanoseconds * f64::from(timebase.denom) / f64::from(timebase.numer)
    }
}

#[cfg(not(target_os = "macos"))]
mod portable {
    use std::sync::OnceLock;
    use std::time::Instant;

    /// A monotonic "host clock" in nanoseconds since first use.
    ///
    /// The IO path is never driven by `coreaudiod` off macOS, so
    /// this exists only to keep the code compiling and to give the
    /// entry-point tests a clock that at least advances.
    #[must_use]
    pub fn host_time_now() -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }

    /// The "host clock" off macOS is plain nanoseconds, so one
    /// period is simply its duration in nanoseconds.
    #[must_use]
    pub fn host_ticks_per_period(period_frames: f64, sample_rate: f64) -> f64 {
        if sample_rate <= 0.0 {
            return 0.0;
        }
        period_frames / sample_rate * 1.0e9
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_time_advances_monotonically() {
        let first = host_time_now();
        let second = host_time_now();
        assert!(second >= first, "the host clock must not run backwards");
    }

    #[test]
    fn host_ticks_per_period_is_positive_for_a_real_format() {
        // One second of 48 kHz audio is a positive span of host
        // ticks on every platform.
        let ticks = host_ticks_per_period(48_000.0, 48_000.0);
        assert!(ticks > 0.0, "a one-second period must span > 0 host ticks");
    }

    #[test]
    fn host_ticks_per_period_rejects_a_nonpositive_sample_rate() {
        assert_eq!(host_ticks_per_period(48_000.0, 0.0), 0.0);
        assert_eq!(host_ticks_per_period(48_000.0, -1.0), 0.0);
    }

    #[test]
    fn host_ticks_per_period_scales_with_the_frame_count() {
        // Two periods' worth of frames spans twice the host ticks.
        let one = host_ticks_per_period(48_000.0, 48_000.0);
        let two = host_ticks_per_period(96_000.0, 48_000.0);
        assert!((two - one * 2.0).abs() < one * 1.0e-9);
    }
}
