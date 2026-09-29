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
    fn next_frame(&mut self) -> [f32; 2] {
        let Some(resampler) = &mut self.resampler else {
            return self.source.next_frame();
        };
        if self.failed {
            return [0.0; 2];
        }
        if self.cursor >= self.available {
            for i in 0..CHUNK {
                let frame = self.source.next_frame();
                self.input[0][i] = frame[0];
                self.input[1][i] = frame[1];
            }
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
        frame
    }
}
/// Explicit mono downmix; wider/unknown layouts are rejected before stream creation.
pub fn stereo_to_mono(frame: [f32; 2]) -> f32 {
    (frame[0] + frame[1]) * 0.5
}
