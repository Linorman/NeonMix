use crate::{AudioError, AudioFormat, signal::StereoSource};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
const CHUNK: usize = 256;
/// Fixed nominal conversion only. Long-term feedback/ppm control belongs to E03.
pub struct RateConverter<S> {
    source: S,
    resampler: Option<SincFixedIn<f32>>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    cursor: usize,
    available: usize,
    failed: bool,
    delay: usize,
    output_rate: u32,
    input_rate: u32,
    presentation: Option<std::time::Instant>,
    output_anchor_frame: u64,
    rendered_frames: u64,
    drawn_frames: u64,
}
impl<S: StereoSource> RateConverter<S> {
    pub fn new(source: S, input_rate: u32, output_rate: u32) -> Result<Self, AudioError> {
        AudioFormat::new(input_rate, 2)?;
        AudioFormat::new(output_rate, 2)?;
        let resampler = if input_rate == output_rate {
            None
        } else {
            Some(
                SincFixedIn::new(
                    f64::from(output_rate) / f64::from(input_rate),
                    1.0,
                    SincInterpolationParameters {
                        sinc_len: 128,
                        f_cutoff: 0.95,
                        interpolation: SincInterpolationType::Cubic,
                        oversampling_factor: 128,
                        window: WindowFunction::BlackmanHarris2,
                    },
                    CHUNK,
                    2,
                )
                .map_err(|e| AudioError::Backend(e.to_string()))?,
            )
        };
        let size = resampler
            .as_ref()
            .map_or(CHUNK, Resampler::output_frames_max);
        let delay = resampler.as_ref().map_or(0, Resampler::output_delay);
        Ok(Self {
            source,
            resampler,
            input: vec![vec![0.0; CHUNK]; 2],
            output: vec![vec![0.0; size]; 2],
            cursor: 0,
            available: 0,
            failed: false,
            delay,
            output_rate,
            input_rate,
            presentation: None,
            output_anchor_frame: 0,
            rendered_frames: 0,
            drawn_frames: 0,
        })
    }
    pub fn delay_frames(&self) -> usize {
        self.delay
    }
    pub fn failed(&self) -> bool {
        self.failed
    }
}
impl<S: StereoSource> StereoSource for RateConverter<S> {
    fn set_presentation_time(&mut self, at: std::time::Instant) {
        self.presentation = Some(at);
        self.output_anchor_frame = self.rendered_frames;
        if self.resampler.is_none() {
            self.source.set_presentation_time(at);
        }
    }
    fn next_frame(&mut self) -> [f32; 2] {
        let Some(resampler) = &mut self.resampler else {
            return self.source.next_frame();
        };
        if self.failed {
            return [0.0; 2];
        }
        if self.cursor >= self.available {
            if let Some(at) = self.presentation {
                // SincFixedIn retains source lookahead and returns a shortened
                // first output chunk. Its output_delay is not extra leading
                // silence (verified by an impulse). Map actual source/output
                // coordinates instead of adding that delay again per chunk.
                let source_ns = crate::clock::frames_to_ns(self.drawn_frames, self.input_rate);
                let output_ns =
                    crate::clock::frames_to_ns(self.output_anchor_frame, self.output_rate);
                let mapped = if source_ns >= output_ns {
                    at.checked_add(std::time::Duration::from_nanos(source_ns - output_ns))
                } else {
                    at.checked_sub(std::time::Duration::from_nanos(output_ns - source_ns))
                };
                if let Some(mapped) = mapped {
                    self.source.set_presentation_time(mapped);
                }
            }
            for i in 0..CHUNK {
                let frame = self.source.next_frame();
                self.input[0][i] = frame[0];
                self.input[1][i] = frame[1];
            }
            self.drawn_frames = self.drawn_frames.saturating_add(CHUNK as u64);
            match resampler.process_into_buffer(&self.input, &mut self.output, None) {
                Ok((_, written)) if written > 0 => {
                    self.available = written;
                    self.cursor = 0;
                }
                _ => {
                    self.failed = true;
                    return [0.0; 2];
                }
            }
        }
        let frame = [self.output[0][self.cursor], self.output[1][self.cursor]];
        self.cursor += 1;
        self.rendered_frames = self.rendered_frames.saturating_add(1);
        frame
    }
}
/// Explicit mono downmix; wider/unknown layouts are rejected before stream creation.
pub fn stereo_to_mono(frame: [f32; 2]) -> f32 {
    (frame[0] + frame[1]) * 0.5
}
