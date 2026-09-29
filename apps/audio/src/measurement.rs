//! Control-thread measurements over captured blocks. Never stores or exports PCM.
use neonmix_core::{AudioBlock, Discontinuity};
use serde::Serialize;

#[derive(Default)]
pub struct CaptureMeasurement {
    frames: u64,
    energy: f64,
    peak: f32,
    epoch: Option<u64>,
    rate: u32,
    next_position: Option<u64>,
    negative_armed: bool,
    quiet_frames: u64,
    crossings: u64,
    first_crossing: u64,
    last_crossing: u64,
    best_crossings: u64,
    best_frequency: Option<f64>,
}

#[derive(Serialize)]
pub struct MeasurementSnapshot {
    pub analyzed_frames: u64,
    pub rms: f64,
    pub peak: f32,
    pub positive_zero_crossings: u64,
    /// Longest left-channel signal segment; 50ms below threshold ends a segment.
    /// Not a spectral/quality analysis.
    pub estimated_frequency_hz: Option<f64>,
}

impl CaptureMeasurement {
    fn finish_segment(&mut self) {
        if self.crossings >= 32
            && self.crossings > self.best_crossings
            && self.last_crossing > self.first_crossing
        {
            self.best_crossings = self.crossings;
            self.best_frequency = Some(
                (self.crossings - 1) as f64 * f64::from(self.rate)
                    / (self.last_crossing - self.first_crossing) as f64,
            );
        }
    }

    pub fn observe(&mut self, block: &AudioBlock) {
        let header = block.header;
        let discontinuous = self.epoch != Some(header.stream_epoch)
            || self.rate != header.format.sample_rate
            || self.next_position != Some(header.source_sample_position)
            || header.discontinuity_flags != Discontinuity::NONE;
        if discontinuous {
            self.finish_segment();
            self.crossings = 0;
            self.negative_armed = false;
            self.quiet_frames = 0;
        }
        self.epoch = Some(header.stream_epoch);
        self.rate = header.format.sample_rate;
        self.next_position = Some(
            header
                .source_sample_position
                .saturating_add(u64::from(header.frame_count)),
        );
        for (offset, frame) in block.frames().iter().enumerate() {
            let position = header.source_sample_position.saturating_add(offset as u64);
            self.energy += (f64::from(frame[0]).powi(2) + f64::from(frame[1]).powi(2)) * 0.5;
            self.peak = self.peak.max(frame[0].abs()).max(frame[1].abs());
            self.frames += 1;
            // Hysteresis rejects near-zero resampler ringing instead of counting it as tone.
            // The absolute floor is -100 dBFS; this is a probe, not an audio-quality estimator.
            let threshold = (self.peak * 0.1).max(0.00001);
            if frame[0].abs() < threshold {
                self.quiet_frames = self.quiet_frames.saturating_add(1);
                // A virtual input can keep delivering zero samples after its source stops.
                // Those frames advance capture time but must not connect two tone segments.
                if self.quiet_frames >= u64::from(self.rate / 20) && self.crossings > 0 {
                    self.finish_segment();
                    self.crossings = 0;
                    self.negative_armed = false;
                }
            } else {
                self.quiet_frames = 0;
            }
            if frame[0] <= -threshold {
                self.negative_armed = true;
            }
            if self.negative_armed && frame[0] >= threshold {
                self.negative_armed = false;
                if self.crossings == 0 {
                    self.first_crossing = position;
                }
                self.last_crossing = position;
                self.crossings += 1;
            }
        }
    }

    pub fn snapshot(&mut self) -> MeasurementSnapshot {
        self.finish_segment();
        MeasurementSnapshot {
            analyzed_frames: self.frames,
            rms: if self.frames == 0 {
                0.0
            } else {
                (self.energy / self.frames as f64).sqrt()
            },
            peak: self.peak,
            positive_zero_crossings: self.best_crossings,
            estimated_frequency_hz: self.best_frequency,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neonmix_core::{
        AudioFormat,
        signal::{SignalKind, StereoSource, TestSignal},
    };
    #[test]
    fn measures_received_tone_and_does_not_invent_one_for_silence() {
        for rate in [44100, 48000] {
            let mut measurement = CaptureMeasurement::default();
            let mut block = AudioBlock::empty(1, AudioFormat::new(rate, 2).unwrap());
            block.header.frame_count = 127;
            let mut signal = TestSignal::new(SignalKind::Sine, rate, 437.0, -36.0).unwrap();
            for index in 0..400 {
                block.header.source_sample_position = index * 127;
                block.header.discontinuity_flags = if index == 0 {
                    Discontinuity::START
                } else {
                    Discontinuity::NONE
                };
                for frame in &mut block.pcm[..127] {
                    *frame = signal.next_frame();
                }
                measurement.observe(&block);
            }
            for index in 400..410 {
                block.header.source_sample_position = index * 127;
                for (i, frame) in block.pcm[..127].iter_mut().enumerate() {
                    *frame = if i % 2 == 0 {
                        [0.000001; 2]
                    } else {
                        [-0.000001; 2]
                    };
                }
                measurement.observe(&block);
            }
            let summary = measurement.snapshot();
            assert!((summary.estimated_frequency_hz.unwrap() - 437.0).abs() < 0.1);
            assert!((summary.rms - 0.0110694).abs() < 0.0001);
            assert_eq!(summary.analyzed_frames, 52070);
            let mut silent = CaptureMeasurement::default();
            block.pcm.fill([0.0; 2]);
            silent.observe(&block);
            assert_eq!(silent.snapshot().estimated_frequency_hz, None);
            assert_eq!(silent.snapshot().rms, 0.0);
        }
    }
    #[test]
    fn never_counts_a_gap_as_continuous_source_time() {
        let mut measurement = CaptureMeasurement::default();
        let mut block = AudioBlock::empty(1, AudioFormat::INTERNAL);
        block.header.frame_count = 480;
        let mut source = TestSignal::new(SignalKind::Sine, 48000, 1000.0, -24.0).unwrap();
        for i in 0..50 {
            block.header.source_sample_position = i * 480;
            block.header.discontinuity_flags = Discontinuity::NONE;
            for frame in &mut block.pcm {
                *frame = source.next_frame();
            }
            measurement.observe(&block);
        }
        block.header.source_sample_position = 10_000_000;
        block.header.discontinuity_flags = Discontinuity::GAP;
        measurement.observe(&block);
        assert!((measurement.snapshot().estimated_frequency_hz.unwrap() - 1000.0).abs() < 0.1);
    }

    #[test]
    fn source_restart_does_not_count_intervening_silence_as_tone_time() {
        let mut measurement = CaptureMeasurement::default();
        let mut block = AudioBlock::empty(1, AudioFormat::INTERNAL);
        block.header.frame_count = 480;
        block.header.discontinuity_flags = Discontinuity::NONE;
        let mut position = 0;
        for (kind, frequency, blocks) in [
            (SignalKind::Sine, 437.0, 200),
            (SignalKind::Silence, 437.0, 300),
            (SignalKind::Sine, 659.0, 300),
        ] {
            let mut source = TestSignal::new(kind, 48000, frequency, -36.0).unwrap();
            for _ in 0..blocks {
                block.header.source_sample_position = position;
                for frame in &mut block.pcm {
                    *frame = source.next_frame();
                }
                measurement.observe(&block);
                position += 480;
            }
            let expected = if frequency == 659.0 { 659.0 } else { 437.0 };
            assert!(
                (measurement.snapshot().estimated_frequency_hz.unwrap() - expected).abs() < 0.1
            );
        }
        assert_eq!(measurement.snapshot().analyzed_frames, 8 * 48000);
    }
}
