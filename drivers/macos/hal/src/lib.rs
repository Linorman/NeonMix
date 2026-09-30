//! NeonMix's stable Core Audio virtual speaker and readable loopback side.
//!
//! The plug-in performs no networking. Its fixed-capacity, sample-time-tagged ring transfers
//! the mixed render stream to Core Audio's input side, which Sender captures by UID. Neither
//! side waits for Sender or Hub connectivity.
#![allow(unsafe_code)] // `plugin_entry!` emits the audited CFPlugIn C ABI factory.

use std::sync::atomic::{AtomicU64, Ordering};
use tympan_aspl::{
    DeviceSpec, Driver, IoBuffer, IoOperation, RealtimeContext, StreamFormat, StreamSpec,
    plugin_entry,
};

/// Room names and network addresses never enter this identity.
pub const DEVICE_UID: &str = "com.neonmix.audio.virtual-output";
pub const DEVICE_NAME: &str = "NeonMix";
pub const SAMPLE_RATE: f64 = 48_000.0;
const RING_FRAMES: usize = 48_000;

struct Frame {
    // 0 is invalid; frame time + 1 is published only after both samples are written.
    stamp: AtomicU64,
    stereo: AtomicU64,
}

impl Frame {
    fn new() -> Self {
        Self {
            stamp: AtomicU64::new(0),
            stereo: AtomicU64::new(0),
        }
    }
}

pub struct NeonMixHal {
    frames: Box<[Frame]>,
}

impl NeonMixHal {
    fn render_sample(sample: f32, gain: f32) -> f32 {
        if gain == 0.0 || !sample.is_finite() {
            0.0
        } else {
            sample * gain
        }
    }
    fn sample_frame(time: f64) -> Option<u64> {
        (time.is_finite() && time >= 0.0 && time < (u64::MAX - RING_FRAMES as u64) as f64)
            .then(|| time.floor() as u64)
    }

    fn write(&self, start: u64, samples: &[f32], gain: f32) {
        for (index, pair) in samples.chunks_exact(2).enumerate() {
            let time = start + index as u64;
            let slot = &self.frames[time as usize % RING_FRAMES];
            // SeqCst orders stamp invalidation, the indivisible stereo pair and
            // publication against concurrent reads, including ring wraparound.
            slot.stamp.store(0, Ordering::SeqCst);
            let left = Self::render_sample(pair[0], gain).to_bits();
            let right = Self::render_sample(pair[1], gain).to_bits();
            slot.stereo
                .store(u64::from(left) | (u64::from(right) << 32), Ordering::SeqCst);
            slot.stamp.store(time + 1, Ordering::SeqCst);
        }
    }

    fn read(&self, start: u64, samples: &mut [f32]) {
        for (index, pair) in samples.chunks_exact_mut(2).enumerate() {
            let time = start + index as u64;
            let slot = &self.frames[time as usize % RING_FRAMES];
            let expected = time + 1;
            if slot.stamp.load(Ordering::SeqCst) == expected {
                let stereo = slot.stereo.load(Ordering::SeqCst);
                if slot.stamp.load(Ordering::SeqCst) == expected {
                    pair[0] = f32::from_bits(stereo as u32);
                    pair[1] = f32::from_bits((stereo >> 32) as u32);
                    continue;
                }
            }
            pair.fill(0.0);
        }
        if !samples.len().is_multiple_of(2) {
            samples[samples.len() - 1] = 0.0;
        }
    }
}

impl Driver for NeonMixHal {
    const NAME: &'static str = DEVICE_NAME;
    const MANUFACTURER: &'static str = "NeonMix";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn new() -> Self {
        Self {
            frames: std::iter::repeat_with(Frame::new)
                .take(RING_FRAMES)
                .collect(),
        }
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(SAMPLE_RATE, 2);
        DeviceSpec::new(DEVICE_UID, DEVICE_NAME, Self::MANUFACTURER)
            .with_sample_rate(SAMPLE_RATE)
            .with_output(StreamSpec::output(format))
            .with_input(StreamSpec::input(format))
    }

    fn start_io(&self) -> Result<(), tympan_aspl::OsStatus> {
        // StartIO is off the realtime path. Clearing stamps makes a new clock epoch silent
        // until the first current WriteMix, even if the old ring still contains PCM bits.
        for frame in &self.frames {
            frame.stamp.store(0, Ordering::Relaxed);
        }
        Ok(())
    }

    fn process_io(&self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        let Some(start) = Self::sample_frame(buffer.timestamp.sample_time) else {
            buffer.silence_output();
            return;
        };
        match buffer.operation {
            IoOperation::WRITE_MIX => {
                // This receives the already-mixed stream, so gain cannot be applied twice.
                // The framework also stores our output in its own ring; only this tagged
                // ring is used by the READ_INPUT branch below.
                self.write(start, buffer.input, buffer.render_gain);
                let copied = buffer.input.len().min(buffer.output.len());
                for (output, input) in buffer.output[..copied].iter_mut().zip(buffer.input) {
                    *output = Self::render_sample(*input, buffer.render_gain);
                }
                buffer.output[copied..].fill(0.0);
            }
            IoOperation::READ_INPUT => self.read(start, buffer.output),
            _ => buffer.silence_output(),
        }
    }
}

plugin_entry!(NeonMixHal, NeonMixHalFactory);

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_void;
    use tympan_aspl::raw::abi::{
        AudioServerPlugInDriverInterface, AudioServerPlugInDriverRef, AudioServerPlugInIOCycleInfo,
        AudioTimeStamp,
    };
    use tympan_aspl::{IoOperation, Timestamp};

    #[test]
    fn fixed_device_identity_and_format() {
        let spec = NeonMixHal::new().device();
        assert_eq!(spec.uid(), DEVICE_UID);
        assert_eq!(spec.sample_rate(), SAMPLE_RATE);
        assert_eq!(spec.input().unwrap().channels(), 2);
        assert_eq!(spec.output().unwrap().channels(), 2);
    }

    #[test]
    fn stale_frames_are_silent_after_wrap_and_restart() {
        // Driver::process_io is the only project code on the plug-in's realtime path.
        // SAFETY: this test directly invokes a pure sample-copy callback without HAL state.
        let rt = unsafe { RealtimeContext::new_unchecked() };
        let driver = NeonMixHal::new();
        let source = [0.25, -0.5, 0.75, -1.0];
        let mut scratch = [0.0; 4];
        driver.process_io(
            &rt,
            &mut IoBuffer::new(
                Timestamp::ZERO,
                IoOperation::WRITE_MIX,
                &source,
                &mut scratch,
            ),
        );
        let mut output = [3.0; 4];
        driver.process_io(
            &rt,
            &mut IoBuffer::new(Timestamp::ZERO, IoOperation::READ_INPUT, &[], &mut output),
        );
        assert_eq!(output, source);
        driver.process_io(
            &rt,
            &mut IoBuffer::new(
                Timestamp::new(RING_FRAMES as f64, 0),
                IoOperation::READ_INPUT,
                &[],
                &mut output,
            ),
        );
        assert_eq!(output, [0.0; 4]);
        driver.start_io().unwrap();
        driver.process_io(
            &rt,
            &mut IoBuffer::new(Timestamp::ZERO, IoOperation::READ_INPUT, &[], &mut output),
        );
        assert_eq!(output, [0.0; 4]);
    }

    #[test]
    fn concurrent_wraparound_never_returns_torn_stereo_or_old_frames() {
        use std::sync::{Arc, Barrier};
        let driver = Arc::new(NeonMixHal::new());
        let barrier = Arc::new(Barrier::new(2));
        let writer = driver.clone();
        let writer_barrier = barrier.clone();
        let worker = std::thread::spawn(move || {
            writer_barrier.wait();
            for time in 0..(RING_FRAMES as u64 * 6) {
                let sample = (time % 997 + 1) as f32 / 1000.0;
                writer.write(time, &[sample, -sample], 1.0);
            }
        });
        barrier.wait();
        for time in 0..(RING_FRAMES as u64 * 6) {
            let mut output = [9.0; 2];
            driver.read(time, &mut output);
            let sample = (time % 997 + 1) as f32 / 1000.0;
            assert!(output == [0.0; 2] || output == [sample, -sample]);
        }
        worker.join().unwrap();
        let mut output = [9.0; 2];
        driver.read(0, &mut output);
        assert_eq!(output, [0.0; 2]);
    }

    #[test]
    fn cfplugin_vtable_round_trips_mixed_audio_and_clock() {
        // SAFETY: the in-process factory accepts null CF arguments in this harness.
        // All subsequent buffers and output pointers are live for each ABI call;
        // Release is the last use of the returned object.
        unsafe {
            let object = NeonMixHalFactory(std::ptr::null(), std::ptr::null());
            assert!(!object.is_null());
            let driver = object as AudioServerPlugInDriverRef;
            let vtable: &AudioServerPlugInDriverInterface = &**driver;
            assert_eq!(vtable.Initialize.unwrap()(driver, std::ptr::null_mut()), 0);
            assert_eq!(vtable.StartIO.unwrap()(driver, 2, 1), 0);

            let mut sample_time = -1.0;
            let mut host_time = 0;
            let mut seed = 0;
            assert_eq!(
                vtable.GetZeroTimeStamp.unwrap()(
                    driver,
                    2,
                    1,
                    &mut sample_time,
                    &mut host_time,
                    &mut seed,
                ),
                0
            );
            assert!(sample_time.is_finite() && sample_time >= 0.0);
            assert!(seed > 0);

            let time = AudioTimeStamp {
                mSampleTime: 1024.0,
                ..Default::default()
            };
            let cycle = AudioServerPlugInIOCycleInfo {
                mInputTime: time,
                mOutputTime: time,
                ..Default::default()
            };
            let samples = [0.25_f32, -0.5, 0.75, -1.0, 0.125, -0.125, 0.0, 0.5];
            let mut readback = [0.0_f32; 8];
            let write = IoOperation::WRITE_MIX.code().as_u32();
            let read = IoOperation::READ_INPUT.code().as_u32();
            assert_eq!(
                vtable.DoIOOperation.unwrap()(
                    driver,
                    2,
                    4,
                    1,
                    write,
                    4,
                    &cycle,
                    samples.as_ptr() as *mut c_void,
                    std::ptr::null_mut(),
                ),
                0
            );
            assert_eq!(
                vtable.DoIOOperation.unwrap()(
                    driver,
                    2,
                    3,
                    1,
                    read,
                    4,
                    &cycle,
                    readback.as_mut_ptr().cast(),
                    std::ptr::null_mut(),
                ),
                0
            );
            assert_eq!(readback, samples);
            assert_eq!(vtable.StopIO.unwrap()(driver, 2, 1), 0);
            assert_eq!(vtable.Release.unwrap()(object), 0);
        }
    }
}
