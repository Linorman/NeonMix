use serde::Serialize;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU64,
    Ordering::{Acquire, Relaxed, Release},
};

#[derive(Default)]
pub struct AudioStats {
    pub callbacks: AtomicU64,
    pub frames: AtomicU64,
    pub silent_frames: AtomicU64,
    pub dropped_frames: AtomicU64,
    pub stale_frames: AtomicU64,
    pub no_data_intervals: AtomicU64,
    pub discontinuities: AtomicU64,
    pub errors: AtomicU64,
    pub last_error: AtomicU64,
    pub last_arrival_ns: AtomicU64,
    pub last_device_ns: AtomicU64,
    pub submitted_frames: AtomicU64,
    pub presented_frames: AtomicU64,
    pub clock_epoch: AtomicU64,
    pub callback_max_ns: AtomicU64,
    pub callback_over_budget: AtomicU64,
    pub period_min: AtomicU64,
    pub period_max: AtomicU64,
    pub peak_bits: AtomicU64,
    pub native_position_valid: AtomicBool,
    pub native_position: AtomicI64,
    pub native_stream_frames: AtomicU64,
    pub native_internal_frames: AtomicU64,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatsSnapshot {
    pub callbacks: u64,
    pub frames: u64,
    pub silent_frames: u64,
    pub dropped_frames: u64,
    pub stale_frames: u64,
    pub no_data_intervals: u64,
    pub discontinuities: u64,
    pub errors: u64,
    pub last_error: u64,
    pub last_arrival_ns: u64,
    pub last_device_ns: u64,
    pub submitted_frames: u64,
    pub presented_frames: u64,
    pub clock_epoch: u64,
    pub callback_max_ns: u64,
    pub callback_over_budget: u64,
    pub period_min: u64,
    pub period_max: u64,
    pub peak: f32,
    pub native_device_frame_position: Option<i64>,
    pub native_stream_frames: Option<u64>,
    pub native_internal_frames: Option<u64>,
}
impl AudioStats {
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            callbacks: self.callbacks.load(Relaxed),
            frames: self.frames.load(Relaxed),
            silent_frames: self.silent_frames.load(Relaxed),
            dropped_frames: self.dropped_frames.load(Relaxed),
            stale_frames: self.stale_frames.load(Relaxed),
            no_data_intervals: self.no_data_intervals.load(Relaxed),
            discontinuities: self.discontinuities.load(Relaxed),
            errors: self.errors.load(Acquire),
            last_error: self.last_error.load(Relaxed),
            last_arrival_ns: self.last_arrival_ns.load(Relaxed),
            last_device_ns: self.last_device_ns.load(Relaxed),
            submitted_frames: self.submitted_frames.load(Relaxed),
            presented_frames: self.presented_frames.load(Relaxed),
            clock_epoch: self.clock_epoch.load(Relaxed),
            callback_max_ns: self.callback_max_ns.load(Relaxed),
            callback_over_budget: self.callback_over_budget.load(Relaxed),
            period_min: self.period_min.load(Relaxed),
            period_max: self.period_max.load(Relaxed),
            peak: f32::from_bits(self.peak_bits.load(Relaxed) as u32),
            native_device_frame_position: self
                .native_position_valid
                .load(Relaxed)
                .then(|| self.native_position.load(Relaxed)),
            native_stream_frames: self
                .native_position_valid
                .load(Relaxed)
                .then(|| self.native_stream_frames.load(Relaxed)),
            native_internal_frames: self
                .native_position_valid
                .load(Relaxed)
                .then(|| self.native_internal_frames.load(Relaxed)),
        }
    }
    /// Nominal period overrun, including preemption; distinct from a driver-reported Xrun.
    pub fn callback_at_rate(&self, frames: usize, rate: u32, elapsed_ns: u64) {
        if frames > 0 && rate > 0 && elapsed_ns > crate::clock::frames_to_ns(frames as u64, rate) {
            self.callback_over_budget.fetch_add(1, Relaxed);
        }
        self.callback(frames, elapsed_ns);
    }
    pub fn callback(&self, frames: usize, elapsed_ns: u64) {
        self.callbacks.fetch_add(1, Relaxed);
        self.frames.fetch_add(frames as u64, Relaxed);
        self.callback_max_ns.fetch_max(elapsed_ns, Relaxed);
        self.period_max.fetch_max(frames as u64, Relaxed);
        if self.period_min.load(Relaxed) == 0 {
            self.period_min.store(frames as u64, Relaxed);
        } else {
            self.period_min.fetch_min(frames as u64, Relaxed);
        }
    }
    pub fn error(&self, code: u64) {
        self.last_error.store(code, Relaxed);
        self.errors.fetch_add(1, Release);
    }
}
/// Called by a non-real-time monitor at most once per interval. Silence counts as data.
#[derive(Default)]
pub struct ActivityMonitor {
    last_frames: u64,
}
impl ActivityMonitor {
    pub fn tick(&mut self, stats: &AudioStats) -> bool {
        let current = stats.frames.load(Relaxed);
        let no_data = current == self.last_frames;
        if no_data {
            stats.no_data_intervals.fetch_add(1, Relaxed);
        }
        self.last_frames = current;
        no_data
    }
}
