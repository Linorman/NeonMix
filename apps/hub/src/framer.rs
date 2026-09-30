//! Capture worker conversion/framing; never runs in a native audio callback.
use crate::Result;
use neonmix_core::{AudioBlock, AudioFormat, ChannelLayout, MAX_BLOCK_FRAMES};
use neonmix_media::Sender;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
const CHUNK: usize = 256;
pub struct CaptureFramer {
    format: AudioFormat,
    epoch: Option<u64>,
    next_native: Option<u64>,
    resampler: Option<SincFixedIn<f32>>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    native_filled: usize,
    frames: [[f32; 2]; MAX_BLOCK_FRAMES],
    filled: usize,
}
impl CaptureFramer {
    pub fn new(format: AudioFormat) -> Result<Self> {
        AudioFormat::new(format.sample_rate, format.channel_layout.channels() as u16)?;
        let resampler = if format.sample_rate == 48000 {
            None
        } else {
            Some(SincFixedIn::new(
                48000.0 / f64::from(format.sample_rate),
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
            )?)
        };
        let size = resampler
            .as_ref()
            .map_or(CHUNK, Resampler::output_frames_max);
        Ok(Self {
            format,
            epoch: None,
            next_native: None,
            resampler,
            input: vec![vec![0.0; CHUNK]; 2],
            output: vec![vec![0.0; size]; 2],
            native_filled: 0,
            frames: [[0.0; 2]; MAX_BLOCK_FRAMES],
            filled: 0,
        })
    }
    pub fn delay_frames(&self) -> usize {
        self.resampler.as_ref().map_or(0, Resampler::output_delay)
    }
    pub fn push(&mut self, block: &AudioBlock, sender: &mut Sender) -> Result<()> {
        if block.header.format != self.format
            || usize::from(block.header.frame_count) > MAX_BLOCK_FRAMES
        {
            return Err("capture format changed; fresh media context required".into());
        }
        if self
            .epoch
            .is_some_and(|epoch| epoch != block.header.stream_epoch)
            || self
                .next_native
                .is_some_and(|position| position != block.header.source_sample_position)
        {
            return Err("capture timeline interrupted; fresh media context required".into());
        }
        self.epoch = Some(block.header.stream_epoch);
        self.next_native = Some(
            block
                .header
                .source_sample_position
                .checked_add(u64::from(block.header.frame_count))
                .ok_or("capture sample position exhausted")?,
        );
        for frame in block.frames() {
            let frame = if self.format.channel_layout == ChannelLayout::Mono {
                [frame[0]; 2]
            } else {
                *frame
            };
            if let Some(resampler) = &mut self.resampler {
                self.input[0][self.native_filled] = frame[0];
                self.input[1][self.native_filled] = frame[1];
                self.native_filled += 1;
                if self.native_filled == CHUNK {
                    let (_, written) =
                        resampler.process_into_buffer(&self.input, &mut self.output, None)?;
                    self.native_filled = 0;
                    for i in 0..written {
                        self.frames[self.filled] = [self.output[0][i], self.output[1][i]];
                        self.filled += 1;
                        if self.filled == MAX_BLOCK_FRAMES {
                            sender.push_frames(&self.frames)?;
                            self.filled = 0;
                        }
                    }
                }
            } else {
                self.frames[self.filled] = frame;
                self.filled += 1;
                if self.filled == MAX_BLOCK_FRAMES {
                    sender.push_frames(&self.frames)?;
                    self.filled = 0;
                }
            }
        }
        Ok(())
    }
}
