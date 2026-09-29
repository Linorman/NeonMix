//! Bounded, allocation-free publication of a coherent native clock observation.
use crate::clock::OutputPosition;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering::SeqCst},
};

struct Slot {
    revision: AtomicU64,
    fields: [AtomicU64; 12],
}
/// Unique writer: publication requires &mut self and this type cannot be cloned.
pub struct PositionPublisher(Arc<Slot>);
#[derive(Clone)]
pub struct PositionReader(Arc<Slot>);

pub fn position_channel() -> (PositionPublisher, PositionReader) {
    let slot = Arc::new(Slot {
        revision: AtomicU64::new(0),
        fields: std::array::from_fn(|_| AtomicU64::new(0)),
    });
    (PositionPublisher(slot.clone()), PositionReader(slot))
}
impl PositionPublisher {
    pub fn publish(&mut self, p: OutputPosition) {
        let flags = u64::from(p.reset)
            | (u64::from(p.epoch_exhausted) << 1)
            | (u64::from(p.native_device_frame_position.is_some()) << 2)
            | (u64::from(p.native_stream_frames.is_some()) << 3)
            | (u64::from(p.native_internal_frames.is_some()) << 4);
        let values = [
            p.epoch,
            u64::from(p.sample_rate),
            p.clock_timestamp_ns,
            p.device_timestamp_ns,
            p.submitted_frames,
            p.presented_frames,
            p.internal_frames,
            flags,
            p.native_device_frame_position.unwrap_or(0) as u64,
            p.native_stream_frames.unwrap_or(0),
            p.native_internal_frames.unwrap_or(0),
            p.callback_frames,
        ];
        // SC order makes the odd/even revision enclose every field in one total order.
        // All payload fields are atomic too: a rejected concurrent read is still race-free.
        self.0.revision.fetch_add(1, SeqCst);
        for (field, value) in self.0.fields.iter().zip(values) {
            field.store(value, SeqCst);
        }
        self.0.revision.fetch_add(1, SeqCst);
    }
}
impl PositionReader {
    /// Latest complete observation, not proof of liveness. None means not published yet
    /// or concurrent publication; retries are bounded and never wait for the audio thread.
    pub fn latest(&self) -> Option<OutputPosition> {
        for _ in 0..3 {
            let before = self.0.revision.load(SeqCst);
            if before == 0 || before & 1 != 0 {
                continue;
            }
            let v: [u64; 12] = std::array::from_fn(|i| self.0.fields[i].load(SeqCst));
            if before != self.0.revision.load(SeqCst) {
                continue;
            }
            return Some(OutputPosition {
                epoch: v[0],
                sample_rate: v[1] as u32,
                clock_timestamp_ns: v[2],
                device_timestamp_ns: v[3],
                submitted_frames: v[4],
                presented_frames: v[5],
                internal_frames: v[6],
                reset: v[7] & 1 != 0,
                epoch_exhausted: v[7] & 2 != 0,
                native_device_frame_position: (v[7] & 4 != 0).then_some(v[8] as i64),
                native_stream_frames: (v[7] & 8 != 0).then_some(v[9]),
                native_internal_frames: (v[7] & 16 != 0).then_some(v[10]),
                callback_frames: v[11],
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{
        AtomicBool,
        Ordering::{Acquire, Release},
    };
    fn observation(n: u64) -> OutputPosition {
        OutputPosition {
            epoch: n,
            sample_rate: 44100,
            clock_timestamp_ns: n * 100,
            device_timestamp_ns: n * 100 + 9,
            submitted_frames: n * 3,
            presented_frames: n * 3 - 1,
            internal_frames: n * 7,
            reset: n.is_multiple_of(5),
            epoch_exhausted: false,
            native_device_frame_position: Some(n as i64 - 5000),
            native_stream_frames: Some(n * 11),
            native_internal_frames: Some(n * 13),
            callback_frames: 127,
        }
    }
    #[test]
    fn preserves_clock_anchor_and_negative_native_coordinates() {
        let (mut writer, reader) = position_channel();
        assert!(reader.latest().is_none());
        writer.publish(observation(1));
        assert_eq!(reader.latest(), Some(observation(1)));
        let mut missing = observation(2);
        missing.native_device_frame_position = None;
        missing.native_internal_frames = None;
        writer.publish(missing);
        assert_eq!(reader.latest(), Some(missing));
    }
    #[test]
    fn concurrent_reader_never_combines_two_observations() {
        let (mut writer, reader) = position_channel();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        let worker = std::thread::spawn(move || {
            for n in 1..=10_000 {
                writer.publish(observation(n));
            }
            finished.store(true, Release);
        });
        while !done.load(Acquire) {
            if let Some(value) = reader.latest() {
                assert_eq!(value, observation(value.epoch));
            }
            std::thread::yield_now();
        }
        worker.join().unwrap();
        assert_eq!(reader.latest(), Some(observation(10_000)));
    }
}
