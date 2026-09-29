use serde::Serialize;
pub fn frames_at_rate(frames: u64, from: u32, to: u32) -> u64 {
    if from == 0 {
        return 0;
    }
    ((u128::from(frames) * u128::from(to) / u128::from(from)).min(u128::from(u64::MAX))) as u64
}
pub fn ns_to_frames(ns: u64, rate: u32) -> u64 {
    ((u128::from(ns) * u128::from(rate)) / 1_000_000_000) as u64
}
pub fn frames_to_ns(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    ((u128::from(frames) * 1_000_000_000 / u128::from(rate)).min(u128::from(u64::MAX))) as u64
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OutputPosition {
    /// Native clock anchor paired with native_device_frame_position in this callback.
    pub clock_timestamp_ns: u64,
    pub sample_rate: u32,
    pub callback_frames: u64,
    pub epoch: u64,
    /// Predicted playback stamp (legacy field name); not the raw clock observation anchor.
    pub device_timestamp_ns: u64,
    pub submitted_frames: u64,
    /// Timestamp-derived presentation estimate, not a measurement of analog output.
    pub presented_frames: u64,
    pub internal_frames: u64,
    pub reset: bool,
    pub epoch_exhausted: bool,
    pub native_device_frame_position: Option<i64>,
    pub native_stream_frames: Option<u64>,
    pub native_internal_frames: Option<u64>,
}
pub struct OutputClock {
    rate: u32,
    epoch: u64,
    submitted: u64,
    last_playback: Option<u64>,
    native_origin: Option<i64>,
    last_native: Option<i64>,
    failed: bool,
}
impl OutputClock {
    pub fn new(rate: u32, epoch: u64) -> Self {
        Self {
            rate,
            epoch,
            submitted: 0,
            last_playback: None,
            native_origin: None,
            last_native: None,
            failed: false,
        }
    }
    pub fn observe(&mut self, callback_ns: u64, playback_ns: u64, frames: usize) -> OutputPosition {
        self.observe_native(callback_ns, playback_ns, frames, None)
    }

    pub fn observe_native(
        &mut self,
        callback_ns: u64,
        playback_ns: u64,
        frames: usize,
        native: Option<i64>,
    ) -> OutputPosition {
        let native_reset = native.zip(self.last_native).is_some_and(|(now, last)| {
            now < last || i128::from(now) - i128::from(last) > i128::from(self.rate)
        });
        let reset = native_reset
            || self.last_playback.is_some_and(|last| {
                playback_ns < last || playback_ns.saturating_sub(last) > 1_000_000_000
            });
        if reset {
            if let Some(epoch) = crate::epoch::reserve_epoch_after(self.epoch) {
                self.epoch = epoch;
            } else {
                self.failed = true;
            }
            self.submitted = 0;
            self.native_origin = None;
        }
        let native_stream_frames = native.map(|position| {
            let origin = *self.native_origin.get_or_insert(position);
            (i128::from(position) - i128::from(origin)).clamp(0, i128::from(u64::MAX)) as u64
        });
        if native.is_some() {
            self.last_native = native;
        }
        let queued = ns_to_frames(playback_ns.saturating_sub(callback_ns), self.rate);
        let presented = self.submitted.saturating_sub(queued);
        self.submitted = self.submitted.saturating_add(frames as u64);
        self.last_playback = Some(playback_ns);
        OutputPosition {
            clock_timestamp_ns: callback_ns,
            sample_rate: self.rate,
            callback_frames: frames as u64,
            epoch: self.epoch,
            device_timestamp_ns: playback_ns,
            submitted_frames: self.submitted,
            presented_frames: presented,
            internal_frames: frames_at_rate(presented, self.rate, 48_000),
            reset,
            epoch_exhausted: self.failed,
            native_device_frame_position: native,
            native_stream_frames,
            native_internal_frames: native_stream_frames
                .map(|n| frames_at_rate(n, self.rate, 48_000)),
        }
    }
}
