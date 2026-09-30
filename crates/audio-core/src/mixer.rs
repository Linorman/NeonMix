//! Fixed-lane output-clock-driven mixer. All lanes/resamplers are constructed
//! on the control thread and retained until the output stream is destroyed there.
use crate::{
    AudioBlock, AudioError, AudioFormat, MAX_BLOCK_FRAMES,
    queue::{BlockConsumer, BlockProducer, block_queue},
    signal::StereoSource,
    stats::AudioStats,
};
use rtrb::{Consumer, Producer, RingBuffer};
use rubato::{
    Resampler, SincFixedOut, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::Instant,
};

pub const LANES: usize = 16;
pub const PCM_QUEUE_BLOCKS: usize = 8;
// 70 ms target includes the current output chunk. Keep at least 60 ms
// of source PCM after feeding sinc, plus its 10 ms output chunk. The extra
// FIFO packet accommodates bounded packet quantization at the refill boundary.
const FIFO: usize = MAX_BLOCK_FRAMES * 9;
const MIN_RESERVE_FRAMES: usize = MAX_BLOCK_FRAMES * 6;
pub const TARGET_WATER_FRAMES: usize = MAX_BLOCK_FRAMES * 7;
pub const MAX_WATER_FRAMES: usize = FIFO + PCM_QUEUE_BLOCKS * MAX_BLOCK_FRAMES;
const GAIN_RAMP_FRAMES: usize = 240; // 5 ms at 48 kHz, independent of gain level.

struct GainRamp {
    value: f32,
    target: f32,
    step: f32,
    remaining: usize,
}
impl GainRamp {
    fn new(target: f32) -> Self {
        let mut ramp = Self {
            value: 0.0,
            target,
            step: 0.0,
            remaining: 0,
        };
        ramp.restart_from_zero();
        ramp
    }
    fn set_target(&mut self, target: f32) {
        if self.target != target {
            self.target = target;
            self.plan();
        }
    }
    fn plan(&mut self) {
        self.step = (self.target - self.value) / GAIN_RAMP_FRAMES as f32;
        self.remaining = GAIN_RAMP_FRAMES;
    }
    fn restart_from_zero(&mut self) {
        self.value = 0.0;
        self.plan();
    }
    fn next(&mut self) -> f32 {
        if self.remaining != 0 {
            self.value += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.value = self.target;
            }
        }
        self.value
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LaneMix {
    pub stream_id: u64,
    pub epoch: u64,
    pub gain_db: f32,
    pub muted: bool,
    pub solo: bool,
}
impl Default for LaneMix {
    fn default() -> Self {
        Self {
            stream_id: 0,
            epoch: 0,
            gain_db: 0.0,
            muted: false,
            solo: false,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct MixerConfig {
    pub lanes: [LaneMix; LANES],
    pub master_db: f32,
    pub muted: bool,
    pub output_epoch: u64,
}
impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            lanes: [LaneMix::default(); LANES],
            master_db: -12.0,
            muted: false,
            output_epoch: 0,
        }
    }
}
pub struct MixerControl {
    commands: Producer<MixerConfig>,
}
impl MixerControl {
    /// Single producer: while the owner serializes changes, a free slot can only
    /// become freer as the output consumer runs. Used for durable control commits.
    pub fn has_capacity(&self) -> bool {
        self.commands.slots() > 0
    }
    pub fn apply(&mut self, config: MixerConfig) -> Result<(), AudioError> {
        if !config.master_db.is_finite()
            || !(-96.0..=12.0).contains(&config.master_db)
            || config
                .lanes
                .iter()
                .any(|l| !l.gain_db.is_finite() || !(-96.0..=12.0).contains(&l.gain_db))
        {
            return Err(AudioError::InvalidArgument(
                "mixer gain must be -96..12 dB".into(),
            ));
        }
        self.commands
            .push(config)
            .map_err(|_| AudioError::Backend("mixer command queue full".into()))
    }
}
#[derive(Default)]
pub struct MixerStats {
    pub output_frames: AtomicU64,
    pub limited_frames: AtomicU64,
    pub underrun_frames: AtomicU64,
    pub underrun_frames_by_lane: [AtomicU64; LANES],
    pub last_underrun_output_frame: [AtomicU64; LANES],
    pub last_underrun_fifo_frames: [AtomicU64; LANES],
    pub last_underrun_needed_frames: [AtomicU64; LANES],
    pub last_underrun_source_end: [AtomicU64; LANES],
    pub rejected_blocks: AtomicU64,
    pub discontinuities: AtomicU64,
    pub drift_ppm_milli: [AtomicU64; LANES],
    pub queue_frames: [AtomicU64; LANES],
    pub filtered_queue_frames: [AtomicU64; LANES],
    pub pcm_queue_age_max_ns: [AtomicU64; LANES],
    pub fifo_frames: [AtomicU64; LANES],
    pub sinc_delay_frames: AtomicU64,
}

/// Long-window source/output slope plus filtered queue correction. A five-second
/// observation window and 30-second low-pass prevent packet arrival jitter from
/// becoming a resampling-rate command. Max ±1000 ppm, slew 50 ppm/s.
pub struct DriftController {
    anchor: Option<(u64, u64)>,
    estimated: f64,
    filtered_water: f64,
    commanded: f64,
}
impl Default for DriftController {
    fn default() -> Self {
        Self {
            anchor: None,
            estimated: 0.0,
            filtered_water: TARGET_WATER_FRAMES as f64,
            commanded: 0.0,
        }
    }
}
impl DriftController {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn observe(
        &mut self,
        source: u64,
        output: u64,
        water: usize,
        elapsed_frames: usize,
    ) -> f64 {
        let dt = elapsed_frames as f64 / 48_000.0;
        self.filtered_water += (water as f64 - self.filtered_water) * (dt / (2.0 + dt));
        let anchor = self.anchor.get_or_insert((source, output));
        if source < anchor.0 || output < anchor.1 {
            self.reset();
            return 0.0;
        }
        let delta_out = output - anchor.1;
        if delta_out >= 240_000 {
            let slope = ((source - anchor.0) as f64 / delta_out as f64 - 1.0) * 1_000_000.0;
            let seconds = delta_out as f64 / 48_000.0;
            if slope.abs() <= 10_000.0 {
                self.estimated += (slope - self.estimated) * seconds / (30.0 + seconds);
            }
            *anchor = (source, output);
        }
        let target = (self.estimated + (self.filtered_water - TARGET_WATER_FRAMES as f64) * 2.0)
            .clamp(-1000.0, 1000.0);
        self.commanded += (target - self.commanded).clamp(-50.0 * dt, 50.0 * dt);
        self.commanded
    }
}

struct Lane {
    input: BlockConsumer,
    resampler: SincFixedOut<f32>,
    planar_in: Vec<Vec<f32>>,
    planar_out: Vec<Vec<f32>>,
    fifo: [[f32; 2]; FIFO],
    head: usize,
    count: usize,
    cursor: usize,
    available: usize,
    config: LaneMix,
    gain: GainRamp,
    last_rendered: [f32; 2],
    retire_tail: [f32; 2],
    retire_remaining: usize,
    source_end: u64,
    have_source: bool,
    started: bool,
    drift: DriftController,
}
impl Lane {
    fn new(input: BlockConsumer) -> Result<Self, AudioError> {
        let resampler = SincFixedOut::<f32>::new(
            1.0,
            1.01,
            SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Cubic,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            },
            MAX_BLOCK_FRAMES,
            2,
        )
        .map_err(|e| AudioError::Backend(e.to_string()))?;
        let max_in = resampler.input_frames_max();
        let max_out = resampler.output_frames_max();
        Ok(Self {
            input,
            resampler,
            planar_in: vec![vec![0.0; max_in]; 2],
            planar_out: vec![vec![0.0; max_out]; 2],
            fifo: [[0.0; 2]; FIFO],
            head: 0,
            count: 0,
            cursor: 0,
            available: 0,
            config: LaneMix::default(),
            gain: GainRamp::new(0.0),
            last_rendered: [0.0; 2],
            retire_tail: [0.0; 2],
            retire_remaining: 0,
            source_end: 0,
            have_source: false,
            started: false,
            drift: DriftController::default(),
        })
    }
    fn reset(&mut self) {
        self.resampler.reset();
        self.head = 0;
        self.count = 0;
        self.cursor = 0;
        self.available = 0;
        self.gain.restart_from_zero();
        self.last_rendered = [0.0; 2];
        self.retire_tail = [0.0; 2];
        self.retire_remaining = 0;
        self.have_source = false;
        self.started = false;
        self.drift.reset();
    }
    fn accept(&mut self, block: AudioBlock, stats: &MixerStats) {
        let h = block.header;
        if h.stream_id != self.config.stream_id
            || h.stream_epoch != self.config.epoch
            || h.format != AudioFormat::INTERNAL
            || h.frame_count == 0
            || usize::from(h.frame_count) > MAX_BLOCK_FRAMES
        {
            stats.rejected_blocks.fetch_add(1, Relaxed);
            return;
        }
        if self.have_source
            && (h.source_sample_position != self.source_end || h.discontinuity_flags.0 != 0)
        {
            self.reset();
            stats.discontinuities.fetch_add(1, Relaxed);
        }
        if self.count + usize::from(h.frame_count) > FIFO {
            self.reset();
            stats.discontinuities.fetch_add(1, Relaxed);
        }
        for frame in block.frames() {
            self.fifo[(self.head + self.count) % FIFO] = [finite(frame[0]), finite(frame[1])];
            self.count += 1;
        }
        self.source_end = h
            .source_sample_position
            .saturating_add(u64::from(h.frame_count));
        self.have_source = true;
    }
    fn refill(&mut self, now_ns: u64, output: u64, index: usize, stats: &MixerStats) {
        // At most eight queued blocks examined per refill; inactive lanes also
        // consume their queues so remove/re-add can never replay old audio.
        let needed = self.resampler.input_frames_next();
        for _ in 0..PCM_QUEUE_BLOCKS {
            if self.count >= needed + MIN_RESERVE_FRAMES {
                break;
            }
            let Some(block) = self.input.pop_fresh(now_ns, 80_000_000) else {
                break;
            };
            stats.pcm_queue_age_max_ns[index]
                .fetch_max(now_ns.saturating_sub(block.header.arrival_ns), Relaxed);
            if self.config.stream_id != 0 {
                self.accept(block, stats);
            }
        }
        let water = self.count + self.input.queued_blocks() * MAX_BLOCK_FRAMES;
        stats.queue_frames[index].store(water as u64, Relaxed);
        stats.fifo_frames[index].store(self.count as u64, Relaxed);
        let ppm = if self.started && self.have_source {
            self.drift.observe(
                self.source_end + (self.input.queued_blocks() * MAX_BLOCK_FRAMES) as u64,
                output,
                water,
                MAX_BLOCK_FRAMES,
            )
        } else {
            0.0
        };
        stats.filtered_queue_frames[index].store(self.drift.filtered_water.round() as u64, Relaxed);
        stats.drift_ppm_milli[index].store((ppm * 1000.0).round() as i64 as u64, Relaxed);
        if self
            .resampler
            .set_resample_ratio(1.0 / (1.0 + ppm / 1_000_000.0), true)
            .is_err()
        {
            self.reset();
        }
        let needed = self.resampler.input_frames_next();
        if !self.started && self.count >= needed + MIN_RESERVE_FRAMES {
            self.started = true;
        }
        if self.config.stream_id == 0 || !self.started || self.count < needed {
            self.planar_out.iter_mut().for_each(|c| c.fill(0.0));
            if self.config.stream_id != 0 && self.started {
                stats.last_underrun_output_frame[index].store(output, Relaxed);
                stats.last_underrun_fifo_frames[index].store(self.count as u64, Relaxed);
                stats.last_underrun_needed_frames[index].store(needed as u64, Relaxed);
                stats.last_underrun_source_end[index].store(self.source_end, Relaxed);
                stats
                    .underrun_frames
                    .fetch_add(MAX_BLOCK_FRAMES as u64, Relaxed);
                stats.underrun_frames_by_lane[index].fetch_add(MAX_BLOCK_FRAMES as u64, Relaxed);
                self.reset();
            }
            self.cursor = 0;
            self.available = MAX_BLOCK_FRAMES;
            return;
        }
        for i in 0..needed {
            let f = self.fifo[self.head];
            self.head = (self.head + 1) % FIFO;
            self.count -= 1;
            self.planar_in[0][i] = f[0];
            self.planar_in[1][i] = f[1];
        }
        match self
            .resampler
            .process_into_buffer(&self.planar_in, &mut self.planar_out, None)
        {
            Ok((_, written)) => {
                self.cursor = 0;
                self.available = written;
            }
            Err(_) => {
                self.reset();
                self.planar_out.iter_mut().for_each(|c| c.fill(0.0));
                self.available = MAX_BLOCK_FRAMES;
            }
        }
    }
    fn next(&mut self, now_ns: u64, output: u64, index: usize, stats: &MixerStats) -> [f32; 2] {
        if self.cursor >= self.available {
            self.refill(now_ns, output, index, stats);
        }
        let frame = [
            self.planar_out[0][self.cursor],
            self.planar_out[1][self.cursor],
        ];
        self.cursor += 1;
        // Buffering zeros must not spend the recovery envelope. Start its
        // five-ms sample clock only when this lane can consume source PCM.
        let gain = if self.started {
            self.gain.next()
        } else {
            self.gain.value
        };
        let mut rendered = [frame[0] * gain, frame[1] * gain];
        if self.retire_remaining > 0 {
            let fade = self.retire_remaining as f32 / 240.0;
            rendered[0] += self.retire_tail[0] * fade;
            rendered[1] += self.retire_tail[1] * fade;
            self.retire_remaining -= 1;
        }
        self.last_rendered = rendered;
        rendered
    }
}
pub struct Mixer {
    lanes: Vec<Lane>,
    commands: Consumer<MixerConfig>,
    config: MixerConfig,
    stats: Arc<MixerStats>,
    origin: Instant,
    output: u64,
    now_ns: u64,
    master: GainRamp,
    limiter: f32,
}
pub type MixerParts = (Mixer, MixerControl, Vec<BlockProducer>, Arc<MixerStats>);
impl Mixer {
    pub fn new(origin: Instant) -> Result<MixerParts, AudioError> {
        let mut lanes = Vec::with_capacity(LANES);
        let mut inputs = Vec::with_capacity(LANES);
        for _ in 0..LANES {
            let (p, c) = block_queue(PCM_QUEUE_BLOCKS, Arc::new(AudioStats::default()))?;
            inputs.push(p);
            lanes.push(Lane::new(c)?);
        }
        let (p, c) = RingBuffer::new(8);
        let stats = Arc::new(MixerStats::default());
        stats.sinc_delay_frames.store(
            lanes.first().map_or(0, |l| l.resampler.output_delay()) as u64,
            Relaxed,
        );
        Ok((
            Self {
                lanes,
                commands: c,
                config: MixerConfig::default(),
                stats: stats.clone(),
                origin,
                output: 0,
                now_ns: 0,
                master: GainRamp::new(db_gain(-12.0)),
                limiter: 1.0,
            },
            MixerControl { commands: p },
            inputs,
            stats,
        ))
    }
    pub fn render_block(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.next_frame();
        }
    }
    /// Called on the control thread while no output callback owns this mixer.
    /// Clears stale audio and commands before reopening the same output device.
    pub fn discard_backlog(&mut self) {
        for _ in 0..8 {
            let Ok(config) = self.commands.pop() else {
                break;
            };
            self.apply(config);
        }
        for lane in &mut self.lanes {
            for _ in 0..PCM_QUEUE_BLOCKS {
                if lane.input.pop_fresh(u64::MAX, 0).is_none() {
                    break;
                }
            }
            lane.reset();
        }
        self.master.restart_from_zero();
        self.limiter = 1.0;
    }
    fn apply(&mut self, config: MixerConfig) {
        let solo = config.lanes.iter().any(|l| l.stream_id != 0 && l.solo);
        for (lane, next) in self.lanes.iter_mut().zip(config.lanes) {
            let tail = lane.last_rendered;
            let removed = lane.config.stream_id != 0 && next.stream_id == 0;
            if lane.config.stream_id != next.stream_id
                || lane.config.epoch != next.epoch
                || self.config.output_epoch != config.output_epoch
            {
                lane.reset();
            }
            if removed {
                lane.retire_tail = tail;
                lane.retire_remaining = 240;
            }
            lane.config = next;
            lane.gain.set_target(
                if next.stream_id == 0 || next.muted || (solo && !next.solo) {
                    0.0
                } else {
                    db_gain(next.gain_db)
                },
            );
        }
        self.master.set_target(if config.muted {
            0.0
        } else {
            db_gain(config.master_db)
        });
        self.config = config;
    }
}
impl StereoSource for Mixer {
    fn next_frame(&mut self) -> [f32; 2] {
        if self.output.is_multiple_of(MAX_BLOCK_FRAMES as u64) {
            self.now_ns = self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            // Bounded command drain at the logical block boundary.
            for _ in 0..8 {
                let Ok(config) = self.commands.pop() else {
                    break;
                };
                self.apply(config);
            }
        }
        let now_ns = self.now_ns;
        let mut sum = [0.0f32; 2];
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            let f = lane.next(now_ns, self.output, i, &self.stats);
            sum[0] += f[0];
            sum[1] += f[1];
        }
        let master = self.master.next();
        sum.iter_mut().for_each(|s| *s *= master);
        let peak = sum[0].abs().max(sum[1].abs());
        let target = if peak > 0.98 { 0.98 / peak } else { 1.0 };
        if target < self.limiter {
            self.limiter = target;
        } else {
            self.limiter += (1.0 - self.limiter) / 2400.0;
        }
        if self.limiter < 0.999 {
            self.stats.limited_frames.fetch_add(1, Relaxed);
        }
        self.output = self.output.saturating_add(1);
        self.stats.output_frames.store(self.output, Relaxed);
        [
            finite(sum[0] * self.limiter).clamp(-0.98, 0.98),
            finite(sum[1] * self.limiter).clamp(-0.98, 0.98),
        ]
    }
}
fn db_gain(db: f32) -> f32 {
    if db <= -96.0 {
        0.0
    } else {
        10.0f32.powf(db / 20.0)
    }
}
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}
