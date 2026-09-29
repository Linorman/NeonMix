use crate::{
    AudioBlock, AudioFormat, Discontinuity, MAX_BLOCK_FRAMES,
    clock::{frames_to_ns, ns_to_frames},
    queue::BlockProducer,
    stats::AudioStats,
};
use std::sync::{Arc, atomic::Ordering::Relaxed};
/// Native callbacks feed this bridge. It splits arbitrary periods into bounded source blocks.
pub struct CaptureBridge {
    stream_id: u64,
    epoch: u64,
    format: AudioFormat,
    position: u64,
    failed: bool,
    expected_ns: Option<u64>,
    flags: Discontinuity,
    producer: BlockProducer,
    stats: Arc<AudioStats>,
}
impl CaptureBridge {
    pub fn new(
        stream_id: u64,
        epoch: u64,
        format: AudioFormat,
        producer: BlockProducer,
        stats: Arc<AudioStats>,
    ) -> Self {
        Self {
            stream_id,
            epoch,
            format,
            position: 0,
            failed: false,
            expected_ns: None,
            flags: Discontinuity::START,
            producer,
            stats,
        }
    }
    pub fn reset(&mut self, format: AudioFormat, flag: Discontinuity) {
        let Some(epoch) = crate::epoch::reserve_epoch_after(self.epoch) else {
            self.failed = true;
            self.stats.error(10);
            self.producer.invalidate();
            return;
        };
        self.epoch = epoch;
        self.format = format;
        self.position = 0;
        self.expected_ns = None;
        self.flags = flag;
        self.producer.invalidate();
    }
    pub fn ingest(
        &mut self,
        frames: usize,
        capture_ns: u64,
        arrival_ns: u64,
        mut sample: impl FnMut(usize) -> [f32; 2],
    ) {
        if frames == 0 || self.failed {
            return;
        }
        if let Some(expected) = self.expected_ns {
            // Native timestamp jitter below 2ms does not create false timeline resets.
            if capture_ns.saturating_add(2_000_000) < expected {
                self.reset(self.format, Discontinuity::CLOCK_RESET);
            } else if capture_ns > expected.saturating_add(2_000_000) {
                self.position = self
                    .position
                    .saturating_add(ns_to_frames(capture_ns - expected, self.format.sample_rate));
                self.flags.insert(Discontinuity::GAP);
                self.producer.invalidate();
            }
        }
        if self.failed {
            return;
        }
        self.expected_ns =
            Some(capture_ns.saturating_add(frames_to_ns(frames as u64, self.format.sample_rate)));
        self.stats.last_arrival_ns.store(arrival_ns, Relaxed);
        self.stats.last_device_ns.store(capture_ns, Relaxed);
        self.stats.clock_epoch.store(self.epoch, Relaxed);
        for offset in (0..frames).step_by(MAX_BLOCK_FRAMES) {
            let count = (frames - offset).min(MAX_BLOCK_FRAMES);
            let mut block = AudioBlock::empty(self.stream_id, self.format);
            block.header.stream_epoch = self.epoch;
            block.header.source_sample_position = self.position;
            block.header.frame_count = count as u16;
            block.header.capture_timestamp_ns =
                capture_ns.saturating_add(frames_to_ns(offset as u64, self.format.sample_rate));
            block.header.arrival_ns = arrival_ns;
            block.header.discontinuity_flags = self.flags;
            let mut silent = 0;
            let mut peak = 0.0f32;
            for i in 0..count {
                let frame = sample(offset + i).map(|v| if v.is_finite() { v } else { 0.0 });
                if frame == [0.0; 2] {
                    silent += 1;
                }
                peak = peak.max(frame[0].abs()).max(frame[1].abs());
                block.pcm[i] = frame;
            }
            self.stats.silent_frames.fetch_add(silent, Relaxed);
            self.stats
                .peak_bits
                .store(u64::from(peak.to_bits()), Relaxed);
            if self.flags != Discontinuity::NONE {
                self.stats.discontinuities.fetch_add(1, Relaxed);
            }
            self.position = self.position.saturating_add(count as u64);
            self.flags = if self.producer.push(block) {
                Discontinuity::NONE
            } else {
                Discontinuity::OVERFLOW
            };
        }
    }
}
