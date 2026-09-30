//! The device's audio backing store.
//!
//! A virtual audio device needs somewhere to *hold* audio between
//! the moment a client writes it and the moment another client
//! reads it — that is what makes a loopback loop. [`DeviceRing`] is
//! that store: a fixed-size circular buffer of interleaved `f32`
//! samples, indexed by Core Audio sample time.
//!
//! The HAL drives a device's IO on a realtime thread, so the ring
//! must be allocation-free and lock-free in steady state. It is:
//! the backing store is allocated once when the
//! [`DriverRuntime`](crate::raw::runtime::DriverRuntime) is built
//! (not on the realtime path), and every per-cycle access —
//! [`DeviceRing::write_from`] / [`DeviceRing::read_into`] — is a
//! bounded loop of relaxed atomic loads and stores. The cells are
//! [`AtomicU32`] holding `f32` bits, so the loopback's inherent
//! read/write overlap is a *defined* (benign) data race rather than
//! undefined behaviour.
//!
//! This module is cross-platform plain Rust — no FFI — so the
//! wraparound indexing is unit-tested on any host.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

/// A fixed-size circular store of interleaved `f32` audio samples,
/// indexed by sample time.
///
/// Holds `capacity_frames * channels` samples. A *frame* is one
/// sample across every channel; sample time advances one unit per
/// frame, and the store wraps every `capacity_frames`.
#[derive(Debug)]
pub struct DeviceRing {
    /// `capacity_frames * channels` cells, each an `f32`'s bits.
    cells: Box<[AtomicU32]>,
    /// Number of frames the ring holds before wrapping.
    capacity_frames: usize,
    /// Channels per frame.
    channels: usize,
}

impl DeviceRing {
    /// Allocate a zeroed ring holding `capacity_frames` frames of
    /// `channels`-channel audio.
    ///
    /// Allocates `capacity_frames * channels` cells on the heap.
    /// Call this off the realtime path — when the
    /// [`DriverRuntime`](crate::raw::runtime::DriverRuntime) is
    /// built — never from an IO callback.
    ///
    /// # Panics
    ///
    /// Panics if `capacity_frames` or `channels` is zero: a ring
    /// with no storage cannot hold audio, and rejecting it at
    /// construction keeps the realtime accessors free of degenerate
    /// cases.
    #[must_use]
    pub fn new(capacity_frames: usize, channels: usize) -> Self {
        assert!(
            capacity_frames > 0,
            "DeviceRing capacity_frames must be non-zero"
        );
        assert!(channels > 0, "DeviceRing channels must be non-zero");
        let cell_count = capacity_frames * channels;
        let mut cells = Vec::with_capacity(cell_count);
        for _ in 0..cell_count {
            cells.push(AtomicU32::new(0));
        }
        Self {
            cells: cells.into_boxed_slice(),
            capacity_frames,
            channels,
        }
    }

    /// Frames the ring holds before wrapping.
    #[inline]
    #[must_use]
    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Channels per frame.
    #[inline]
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Total sample cells — `capacity_frames * channels`.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Always `false` — [`DeviceRing::new`] rejects a zero-size
    /// ring, so a constructed ring always has storage.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The cell index `start_frame` maps to, wrapping modulo the
    /// ring capacity. A negative or fractional sample time is
    /// floored and reduced into `0..capacity_frames`.
    #[inline]
    fn frame_offset(&self, start_frame: f64) -> usize {
        let frame = start_frame.floor() as i64;
        frame.rem_euclid(self.capacity_frames as i64) as usize
    }

    /// Write `src` into the ring starting at sample time
    /// `start_frame`, wrapping at the ring boundary.
    ///
    /// `src` is interleaved `f32` audio; its length should be a
    /// whole number of frames (`src.len() % channels == 0`), though
    /// the write itself is sample-granular and tolerates a partial
    /// trailing frame. Realtime-safe: a bounded loop of relaxed
    /// atomic stores, no allocation, no locks.
    pub fn write_from(&self, start_frame: f64, src: &[f32]) {
        if self.cells.is_empty() {
            return;
        }
        let base = self.frame_offset(start_frame) * self.channels;
        for (i, &sample) in src.iter().enumerate() {
            let idx = (base + i) % self.cells.len();
            self.cells[idx].store(sample.to_bits(), Ordering::Relaxed);
        }
    }

    /// Read from the ring starting at sample time `start_frame`
    /// into `dst`, wrapping at the ring boundary.
    ///
    /// Fills every element of `dst` with interleaved `f32` audio.
    /// Realtime-safe: a bounded loop of relaxed atomic loads, no
    /// allocation, no locks.
    pub fn read_into(&self, start_frame: f64, dst: &mut [f32]) {
        if self.cells.is_empty() {
            dst.fill(0.0);
            return;
        }
        let base = self.frame_offset(start_frame) * self.channels;
        for (i, slot) in dst.iter_mut().enumerate() {
            let idx = (base + i) % self.cells.len();
            *slot = f32::from_bits(self.cells[idx].load(Ordering::Relaxed));
        }
    }

    /// Overwrite the whole ring with silence.
    ///
    /// Called off the realtime path — at `StartIO`, to clear stale
    /// audio from a previous run.
    pub fn clear(&self) {
        for cell in &self.cells {
            cell.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_impl_all;

    assert_impl_all!(DeviceRing: Send, Sync);

    #[test]
    fn new_ring_has_the_requested_geometry_and_is_silent() {
        let ring = DeviceRing::new(8, 2);
        assert_eq!(ring.capacity_frames(), 8);
        assert_eq!(ring.channels(), 2);
        assert_eq!(ring.len(), 16);
        assert!(!ring.is_empty());

        let mut out = [9.0_f32; 16];
        ring.read_into(0.0, &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "a fresh ring is silent");
    }

    #[test]
    #[should_panic(expected = "capacity_frames must be non-zero")]
    fn zero_capacity_panics() {
        let _ = DeviceRing::new(0, 2);
    }

    #[test]
    #[should_panic(expected = "channels must be non-zero")]
    fn zero_channels_panics() {
        let _ = DeviceRing::new(8, 0);
    }

    #[test]
    fn write_then_read_round_trips_at_the_same_sample_time() {
        let ring = DeviceRing::new(16, 2);
        let written = [0.1_f32, -0.2, 0.3, -0.4, 0.5, -0.6];
        ring.write_from(4.0, &written);
        let mut read = [0.0_f32; 6];
        ring.read_into(4.0, &mut read);
        assert_eq!(read, written);
    }

    #[test]
    fn writes_wrap_at_the_ring_boundary() {
        // Capacity 4 frames × 1 channel = 4 cells. Writing 6 samples
        // from frame 3 wraps: cells 3, 0, 1, 2, 3, 0.
        let ring = DeviceRing::new(4, 1);
        ring.write_from(3.0, &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0]);
        let mut read = [0.0_f32; 4];
        ring.read_into(0.0, &mut read);
        // Cell 0 was written twice (11.0 then 15.0), cell 3 twice
        // (10.0 then 14.0).
        assert_eq!(read, [15.0, 12.0, 13.0, 14.0]);
    }

    #[test]
    fn sample_time_reduces_modulo_capacity() {
        let ring = DeviceRing::new(8, 1);
        ring.write_from(2.0, &[42.0]);
        // Sample time 10 (= 2 + one full lap) addresses the same
        // cell.
        let mut read = [0.0_f32; 1];
        ring.read_into(10.0, &mut read);
        assert_eq!(read[0], 42.0);
        // And 8 laps later still.
        ring.read_into(2.0 + 8.0 * 8.0, &mut read);
        assert_eq!(read[0], 42.0);
    }

    #[test]
    fn negative_sample_time_is_floored_into_range() {
        let ring = DeviceRing::new(8, 1);
        // -1 frame reduces to cell 7.
        ring.write_from(-1.0, &[7.0]);
        let mut read = [0.0_f32; 1];
        ring.read_into(7.0, &mut read);
        assert_eq!(read[0], 7.0);
    }

    #[test]
    fn fractional_sample_time_floors_to_the_frame() {
        let ring = DeviceRing::new(8, 1);
        ring.write_from(3.0, &[3.0]);
        let mut read = [0.0_f32; 1];
        // 3.9 floors to frame 3.
        ring.read_into(3.9, &mut read);
        assert_eq!(read[0], 3.0);
    }

    #[test]
    fn clear_silences_the_whole_ring() {
        let ring = DeviceRing::new(4, 2);
        ring.write_from(0.0, &[1.0; 8]);
        ring.clear();
        let mut read = [9.0_f32; 8];
        ring.read_into(0.0, &mut read);
        assert!(read.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn channels_interleave_correctly() {
        // 4 frames × 2 channels. Frame 1 holds (L, R) = (1.0, 2.0).
        let ring = DeviceRing::new(4, 2);
        ring.write_from(1.0, &[1.0, 2.0]);
        let mut read = [0.0_f32; 8];
        ring.read_into(0.0, &mut read);
        // Frame 0 silent, frame 1 = (1.0, 2.0), rest silent.
        assert_eq!(read, [0.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_loopback_round_trip_across_threads() {
        use std::sync::Arc;
        use std::thread;

        // One thread writes a ramp into the ring; another reads it
        // back. The atomic cells make the overlap defined behaviour.
        let ring = Arc::new(DeviceRing::new(1024, 1));
        let writer_ring = Arc::clone(&ring);
        let writer = thread::spawn(move || {
            for frame in 0..1024 {
                writer_ring.write_from(frame as f64, &[frame as f32]);
            }
        });
        writer.join().unwrap();

        let mut read = [0.0_f32; 1024];
        ring.read_into(0.0, &mut read);
        for (frame, &sample) in read.iter().enumerate() {
            assert_eq!(sample, frame as f32);
        }
    }
}
