//! The device's zero-timestamp clock.
//!
//! A running AudioServerPlugin device must give the HAL a *zero
//! timestamp*: a `(sample_time, host_time, seed)` triple that
//! anchors the device's sample clock against the host's clock. The
//! HAL polls it through `GetZeroTimeStamp` and uses it to schedule
//! every IO cycle.
//!
//! For a virtual device the clock is synthetic: it does not track a
//! real piece of hardware, it just advances in lockstep with the
//! host clock. [`DeviceClock`] models exactly that. On `StartIO` it
//! records an anchor host time; each `GetZeroTimeStamp` then reports
//! how many whole *zero-timestamp periods* of host time have elapsed
//! since the anchor, mapped to a sample-time and host-time pair on a
//! period boundary.
//!
//! The arithmetic — [`DeviceClock::zero_timestamp`] — is pure and
//! cross-platform, so it is unit-tested on any host. Only *reading*
//! the host clock and converting its tick rate are macOS-specific;
//! those live in [`crate::raw::platform`] and feed their results in
//! as parameters.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A zero-timestamp sample: the anchor the HAL schedules an IO cycle
/// against.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct ZeroTimestamp {
    /// The sample-clock position, on a zero-timestamp-period
    /// boundary.
    pub sample_time: f64,
    /// The host-clock time corresponding to `sample_time`.
    pub host_time: u64,
    /// The timeline seed. It changes only across a discontinuity
    /// (a stop/start); within one run it is constant, which is how
    /// the HAL knows the timeline has not jumped.
    pub seed: u64,
}

/// The synthetic clock behind a virtual device.
///
/// Lives in the [`DriverRuntime`](crate::raw::runtime::DriverRuntime).
/// `StartIO` calls [`Self::start`] with the current host time;
/// `GetZeroTimeStamp` calls [`Self::zero_timestamp`]; `StopIO` calls
/// [`Self::stop`]. Lifecycle writers are serialized by the HAL control path;
/// bounded snapshot reads never wait and never mix an old anchor with a new seed.
#[derive(Debug)]
pub struct DeviceClock {
    /// The host time recorded at the most recent [`Self::start`].
    anchor_host_time: AtomicU64,
    /// The timeline seed, bumped on every [`Self::start`].
    seed: AtomicU64,
    /// Whether the clock is currently running (between `StartIO` and
    /// `StopIO`).
    running: AtomicBool,
    version: AtomicU64,
}

impl DeviceClock {
    /// A fresh, stopped clock with seed `0`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            anchor_host_time: AtomicU64::new(0),
            seed: AtomicU64::new(0),
            running: AtomicBool::new(false),
            version: AtomicU64::new(0),
        }
    }

    /// Anchor the clock at `now_host_time` and mark it running.
    ///
    /// Bumps the seed so the HAL sees a fresh timeline. Called from
    /// the `StartIO` entry point with the host clock's current
    /// value.
    pub fn start(&self, now_host_time: u64) {
        self.version.fetch_add(1, Ordering::SeqCst);
        self.anchor_host_time.store(now_host_time, Ordering::SeqCst);
        self.seed.fetch_add(1, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    /// Mark the clock stopped. Called from the `StopIO` entry point.
    pub fn stop(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    /// `true` between [`Self::start`] and [`Self::stop`].
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// The current timeline seed.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed.load(Ordering::Acquire)
    }

    /// Compute the zero timestamp for the current host time.
    ///
    /// Returns `None` if the clock is not running. Otherwise reports
    /// the most recent zero-timestamp-period boundary at or before
    /// `now_host_time`:
    ///
    /// - `periods` = whole periods of host time elapsed since the
    ///   anchor,
    /// - `sample_time` = `periods * period_frames`,
    /// - `host_time` = `anchor + periods * host_ticks_per_period`.
    ///
    /// `host_ticks_per_period` is the host-clock duration of one
    /// zero-timestamp period (the macOS caller derives it from the
    /// mach timebase; see [`crate::raw::platform`]). `period_frames`
    /// is the device's `kAudioDevicePropertyZeroTimeStampPeriod`.
    ///
    /// Pure arithmetic — no clock is read here — so the boundary
    /// logic is unit-tested directly.
    #[must_use]
    pub fn zero_timestamp(
        &self,
        now_host_time: u64,
        host_ticks_per_period: f64,
        period_frames: f64,
    ) -> Option<ZeroTimestamp> {
        // Read one coherent lifecycle epoch without taking a realtime lock.
        let mut snapshot = None;
        for _ in 0..3 {
            let before = self.version.load(Ordering::SeqCst);
            if before % 2 != 0 {
                continue;
            }
            let running = self.running.load(Ordering::SeqCst);
            let anchor = self.anchor_host_time.load(Ordering::SeqCst);
            let seed = self.seed.load(Ordering::SeqCst);
            if self.version.load(Ordering::SeqCst) == before {
                if !running {
                    return None;
                }
                snapshot = Some((anchor, seed));
                break;
            }
        }
        let (anchor, seed) = snapshot?;
        // A `now` before the anchor (a clock that ran backwards, or
        // a stale read racing `start`) clamps to the anchor itself —
        // period zero.
        let elapsed = now_host_time.saturating_sub(anchor);
        let periods = if host_ticks_per_period > 0.0 {
            (elapsed as f64 / host_ticks_per_period).floor()
        } else {
            0.0
        };
        Some(ZeroTimestamp {
            sample_time: periods * period_frames,
            host_time: anchor.saturating_add((periods * host_ticks_per_period) as u64),
            seed,
        })
    }
}

impl Default for DeviceClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    #[test]
    fn clock_snapshot_cannot_mix_anchors_and_seeds_during_restart() {
        use std::sync::Arc;
        let clock = Arc::new(DeviceClock::new());
        let writer = clock.clone();
        let worker = std::thread::spawn(move || {
            for index in 1..=10000 {
                writer.start(index * 1000);
                writer.stop();
            }
        });
        while !worker.is_finished() {
            if let Some(timestamp) = clock.zero_timestamp(0, 100.0, 480.0) {
                assert_eq!(timestamp.host_time, timestamp.seed * 1000);
                assert_eq!(timestamp.sample_time, 0.0);
            }
        }
        worker.join().unwrap();
    }

    assert_impl_all!(DeviceClock: Send, Sync);

    #[test]
    fn new_clock_is_stopped() {
        let clock = DeviceClock::new();
        assert!(!clock.is_running());
        assert_eq!(clock.seed(), 0);
        assert_eq!(clock.zero_timestamp(1000, 100.0, 480.0), None);
    }

    #[test]
    fn default_matches_new() {
        assert!(!DeviceClock::default().is_running());
    }

    #[test]
    fn start_anchors_running_and_bumps_the_seed() {
        let clock = DeviceClock::new();
        clock.start(5_000);
        assert!(clock.is_running());
        assert_eq!(clock.seed(), 1);
        clock.stop();
        clock.start(9_000);
        assert_eq!(clock.seed(), 2, "each start bumps the seed");
    }

    #[test]
    fn stop_halts_the_clock() {
        let clock = DeviceClock::new();
        clock.start(0);
        clock.stop();
        assert!(!clock.is_running());
        assert_eq!(clock.zero_timestamp(10_000, 100.0, 480.0), None);
    }

    #[test]
    fn zero_timestamp_reports_period_boundaries() {
        let clock = DeviceClock::new();
        clock.start(1_000);
        // 100 host ticks per period, 480 frames per period.
        // At the anchor itself: period 0.
        assert_eq!(
            clock.zero_timestamp(1_000, 100.0, 480.0),
            Some(ZeroTimestamp {
                sample_time: 0.0,
                host_time: 1_000,
                seed: 1,
            })
        );
        // 250 ticks elapsed → 2 whole periods.
        assert_eq!(
            clock.zero_timestamp(1_250, 100.0, 480.0),
            Some(ZeroTimestamp {
                sample_time: 960.0,
                host_time: 1_200,
                seed: 1,
            })
        );
        // 1000 ticks elapsed → exactly 10 periods.
        assert_eq!(
            clock.zero_timestamp(2_000, 100.0, 480.0),
            Some(ZeroTimestamp {
                sample_time: 4_800.0,
                host_time: 2_000,
                seed: 1,
            })
        );
    }

    #[test]
    fn zero_timestamp_floors_within_a_period() {
        let clock = DeviceClock::new();
        clock.start(0);
        // 99 ticks of a 100-tick period: still period 0.
        let ts = clock.zero_timestamp(99, 100.0, 480.0).unwrap();
        assert_eq!(ts.sample_time, 0.0);
        assert_eq!(ts.host_time, 0);
        // 199 ticks: period 1.
        let ts = clock.zero_timestamp(199, 100.0, 480.0).unwrap();
        assert_eq!(ts.sample_time, 480.0);
        assert_eq!(ts.host_time, 100);
    }

    #[test]
    fn zero_timestamp_clamps_a_backwards_clock_to_the_anchor() {
        let clock = DeviceClock::new();
        clock.start(10_000);
        // `now` before the anchor — clamp to period zero.
        assert_eq!(
            clock.zero_timestamp(9_000, 100.0, 480.0),
            Some(ZeroTimestamp {
                sample_time: 0.0,
                host_time: 10_000,
                seed: 1,
            })
        );
    }

    #[test]
    fn zero_timestamp_tolerates_a_zero_tick_rate() {
        // A degenerate `host_ticks_per_period` must not divide by
        // zero — it pins the timeline at period zero.
        let clock = DeviceClock::new();
        clock.start(0);
        let ts = clock.zero_timestamp(123_456, 0.0, 480.0).unwrap();
        assert_eq!(ts.sample_time, 0.0);
        assert_eq!(ts.host_time, 0);
    }

    #[test]
    fn seed_is_carried_into_the_timestamp() {
        let clock = DeviceClock::new();
        clock.start(0);
        clock.stop();
        clock.start(0);
        // Two starts → seed 2 → carried into every timestamp.
        assert_eq!(clock.zero_timestamp(0, 1.0, 1.0).unwrap().seed, 2);
    }
}
