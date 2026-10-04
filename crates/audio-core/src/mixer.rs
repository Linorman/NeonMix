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
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{
            AtomicU64,
            Ordering::{Relaxed, SeqCst},
        },
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
// Match the small callback phase variation covered by timed SRC quality tests.
// Larger changes still reacquire the observed deadline and account for lost PCM.
const TIMED_START_JITTER_NS: u64 = 1_000_000;
/// Latest complete 50 ms window on the internal 48 kHz sample clock.
pub const METER_WINDOW_FRAMES: usize = 2400;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct MeterSnapshot {
    pub stream_id: u64,
    /// Maximum absolute sample across both channels, linear full scale.
    pub peak: f32,
    /// Root mean square across both channels, linear full scale.
    pub rms: f32,
}

/// A bounded, allocation-free publication from the single audio writer. The
/// sequence also protects stream identity when a control update reuses a lane.
#[derive(Default)]
pub struct MeterStats {
    sequence: AtomicU64,
    stream_id: AtomicU64,
    levels: AtomicU64,
}
impl MeterStats {
    fn publish(&self, stream_id: u64, peak: f32, rms: f32) {
        self.sequence.fetch_add(1, SeqCst);
        self.stream_id.store(stream_id, SeqCst);
        self.levels.store(
            u64::from(peak.to_bits()) | (u64::from(rms.to_bits()) << 32),
            SeqCst,
        );
        self.sequence.fetch_add(1, SeqCst);
    }
    /// Called off the audio thread. Contention returns silence rather than
    /// spinning without a bound or combining one stream's identity and levels.
    pub fn snapshot(&self) -> MeterSnapshot {
        for _ in 0..3 {
            let before = self.sequence.load(SeqCst);
            if before & 1 != 0 {
                continue;
            }
            let stream_id = self.stream_id.load(SeqCst);
            let levels = self.levels.load(SeqCst);
            if before == self.sequence.load(SeqCst) {
                return MeterSnapshot {
                    stream_id,
                    peak: f32::from_bits(levels as u32),
                    rms: f32::from_bits((levels >> 32) as u32),
                };
            }
        }
        MeterSnapshot::default()
    }
}

#[derive(Clone, Copy, Default)]
struct MeterWindow {
    frames: usize,
    peak: f32,
    squares: f64,
}
impl MeterWindow {
    fn observe(&mut self, frame: [f32; 2]) {
        let left = finite(frame[0]);
        let right = finite(frame[1]);
        self.frames += 1;
        self.peak = self.peak.max(left.abs()).max(right.abs());
        self.squares += f64::from(left).powi(2) + f64::from(right).powi(2);
    }
    fn publish(&mut self, stats: &MeterStats, stream_id: u64) {
        let rms = if self.frames == 0 {
            0.0
        } else {
            (self.squares / (2 * self.frames) as f64).sqrt() as f32
        };
        stats.publish(stream_id, self.peak, rms);
        *self = Self::default();
    }
}

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
    /// Post lane gain/Mute/Solo, before room master and limiter.
    pub lane_meters: [MeterStats; LANES],
    /// Actual rendered samples after master and limiter, before device conversion.
    pub output_meter: MeterStats,
    /// Minimum linear limiter gain in the latest complete meter window.
    pub limiter_gain_bits: AtomicU64,
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
    pub timed_late_frames: AtomicU64,
    /// Physical-lane lifetime counters; preserve across stream/epoch changes.
    pub timed_late_frames_by_lane: [AtomicU64; LANES],
    pub last_timed_late_stream_id: [AtomicU64; LANES],
    pub last_timed_late_epoch: [AtomicU64; LANES],
    pub last_timed_late_output_frame: [AtomicU64; LANES],
    /// First buffered PCM deadline and the output timestamp that skipped it.
    pub last_timed_late_target_ns: [AtomicU64; LANES],
    pub last_timed_late_presentation_ns: [AtomicU64; LANES],
    /// Timed SRC rate relative to 48 kHz, signed milli-ppm encoded as i64.
    pub timed_drift_ppm_milli: [AtomicU64; LANES],
    /// Filtered deadline phase error, signed nanoseconds encoded as i64.
    pub timed_phase_error_ns: [AtomicU64; LANES],
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

// Centered FIR support is fetched from the existing short handoff queue. Its
// 32-frame (0.667 ms) lookahead compensates group delay: cursor is the source
// sample due at the speaker, never the sample fed into a delayed causal filter.
const TIMED_TAPS: usize = 64;
const TIMED_PHASES: usize = 512;
const TIMED_SUPPORT: usize = TIMED_TAPS / 2;
type TimedKernel = Vec<[f32; TIMED_TAPS]>;
fn timed_kernel() -> Arc<TimedKernel> {
    let mut table = Vec::with_capacity(TIMED_PHASES + 1);
    for phase in 0..=TIMED_PHASES {
        let fraction = phase as f64 / TIMED_PHASES as f64;
        let mut weights = [0.; TIMED_TAPS];
        let mut total = 0.;
        for (tap, weight) in weights.iter_mut().enumerate() {
            let x = tap as f64 - (TIMED_SUPPORT - 1) as f64 - fraction;
            let r = x / TIMED_SUPPORT as f64;
            let window = 0.35875
                + 0.48829 * (std::f64::consts::PI * r).cos()
                + 0.14128 * (std::f64::consts::TAU * r).cos()
                + 0.01168 * (3. * std::f64::consts::PI * r).cos();
            let argument = std::f64::consts::PI * 0.95 * x;
            let sinc = if argument.abs() < 1e-12 {
                0.95
            } else {
                0.95 * argument.sin() / argument
            };
            *weight = (sinc * window) as f32;
            total += f64::from(*weight);
        }
        for weight in &mut weights {
            *weight /= total as f32;
        }
        table.push(weights);
    }
    Arc::new(table)
}

/// Deadline-controlled asynchronous SRC, separate from native queue feedback.
/// Callback timestamps are observations of phase, not commands to seek PCM.
struct TimedPlayback {
    kernel: Arc<TimedKernel>,
    fifo: [[f32; 2]; FIFO],
    head: usize,
    count: usize,
    base: u64,
    end: u64,
    anchor: Option<(u64, u64)>,
    slope_anchor: Option<(u64, u64)>,
    start_anchor: Option<(u64, u64)>,
    waiting_anchor: Option<(u64, u64)>,
    source_rate: f64,
    slope_known: bool,
    position: Option<f64>,
    filtered_error: f64,
    step: f64,
    starved: bool,
}
impl TimedPlayback {
    fn new(kernel: Arc<TimedKernel>) -> Self {
        Self {
            kernel,
            fifo: [[0.; 2]; FIFO],
            head: 0,
            count: 0,
            base: 0,
            end: 0,
            anchor: None,
            slope_anchor: None,
            start_anchor: None,
            waiting_anchor: None,
            source_rate: 1.,
            slope_known: false,
            position: None,
            filtered_error: 0.,
            step: 1.,
            starved: false,
        }
    }
    fn reset(&mut self) {
        self.head = 0;
        self.count = 0;
        self.base = 0;
        self.end = 0;
        self.anchor = None;
        self.slope_anchor = None;
        self.start_anchor = None;
        self.waiting_anchor = None;
        self.source_rate = 1.;
        self.slope_known = false;
        self.position = None;
        self.filtered_error = 0.;
        self.step = 1.;
        self.starved = false;
    }
    fn append(&mut self, block: &AudioBlock) {
        let h = block.header;
        let at = h.presentation_time_ns.expect("validated timed PCM");
        if self.anchor.is_none() {
            self.base = h.source_sample_position;
            self.end = self.base;
            self.slope_anchor = Some((self.base, at));
            self.start_anchor = Some((self.base, at));
        }
        if let Some((position, time)) = self.slope_anchor {
            let ns = at.saturating_sub(time);
            if ns >= 500_000_000 && h.source_sample_position >= position {
                let rate =
                    (h.source_sample_position - position) as f64 * 1e9 / (ns as f64 * 48000.);
                if (0.99..=1.01).contains(&rate) {
                    if self.slope_known {
                        let seconds = ns as f64 / 1e9;
                        self.source_rate += (rate - self.source_rate) * seconds / (2. + seconds);
                    } else {
                        self.source_rate = rate;
                        self.slope_known = true;
                    }
                }
                self.slope_anchor = Some((h.source_sample_position, at));
            }
        }
        self.anchor = Some((h.source_sample_position, at));
        for frame in block.frames() {
            self.fifo[(self.head + self.count) % FIFO] = [finite(frame[0]), finite(frame[1])];
            self.count += 1;
        }
        self.end = h
            .source_sample_position
            .saturating_add(u64::from(h.frame_count));
    }
    fn desired_position(&self, presentation_ns: u64) -> Option<f64> {
        self.anchor.map(|(position, time)| {
            position as f64
                + (presentation_ns as i128 - time as i128) as f64 * 48000. * self.source_rate / 1e9
        })
    }
    fn next(
        &mut self,
        presentation_ns: u64,
        output: u64,
        index: usize,
        config: LaneMix,
        stats: &MixerStats,
    ) -> Option<[f32; 2]> {
        let desired = self.desired_position(presentation_ns)?;
        let position = if let Some(position) = self.position {
            position
        } else {
            let (first, at) = self.start_anchor.expect("PCM start anchor");
            // PCM observed before its deadline already has a place on the
            // output sample grid. Advance that grid while waiting instead of
            // seeking at each callback phase observation. The raw observation
            // still drives the phase servo below. A first packet seen late, or
            // a clock jump larger than the jitter budget, retains real seeking.
            let start_presentation_ns = if let Some((frame, time)) = self.waiting_anchor {
                let predicted = time.saturating_add(crate::clock::frames_to_ns(
                    output.saturating_sub(frame),
                    48_000,
                ));
                if predicted.abs_diff(presentation_ns) <= TIMED_START_JITTER_NS {
                    predicted
                } else {
                    self.waiting_anchor = Some((output, presentation_ns));
                    presentation_ns
                }
            } else {
                presentation_ns
            };
            let start = first as f64
                + (start_presentation_ns as i128 - at as i128) as f64 * 48000. * self.source_rate
                    / 1e9;
            if start < self.base as f64 {
                self.waiting_anchor.get_or_insert((output, presentation_ns));
                return None;
            }
            self.waiting_anchor = None;
            // Only acquiring/reacquiring a genuinely elapsed timeline seeks.
            let late = (start.floor() as u64)
                .saturating_sub(self.base)
                .min(self.count as u64);
            stats.timed_late_frames.fetch_add(late, Relaxed);
            if late > 0 {
                stats.timed_late_frames_by_lane[index].fetch_add(late, Relaxed);
                stats.last_timed_late_stream_id[index].store(config.stream_id, Relaxed);
                stats.last_timed_late_epoch[index].store(config.epoch, Relaxed);
                stats.last_timed_late_output_frame[index].store(output, Relaxed);
                stats.last_timed_late_target_ns[index].store(at, Relaxed);
                stats.last_timed_late_presentation_ns[index].store(presentation_ns, Relaxed);
            }
            self.position = Some(start);
            start
        };
        // 100 ms phase filtering rejects callback jitter. A 0.5 s phase servo
        // and bounded 1000 ppm/s slew keep the cursor continuous, including at
        // new packet anchors. Long-window PTS slope supplies rate feed-forward.
        self.filtered_error += (desired - position - self.filtered_error) / 4800.;
        let target = (self.source_rate + self.filtered_error / 24000.).clamp(0.999, 1.001);
        self.step += (target - self.step).clamp(-0.001 / 48000., 0.001 / 48000.);
        self.position = Some(position + self.step);
        let center = position.floor() as u64;
        // Preserve past FIR support while evicting only samples already used.
        let keep = center.saturating_sub(TIMED_SUPPORT as u64);
        let discard = keep.saturating_sub(self.base).min(self.count as u64) as usize;
        self.head = (self.head + discard) % FIFO;
        self.count -= discard;
        self.base += discard as u64;
        if self.count == 0 || center < self.base || position >= self.end as f64 - 0.0001 {
            self.starved = true;
            return None;
        }
        self.starved = false;
        let fraction = (position - position.floor()) * TIMED_PHASES as f64;
        let phase = (fraction.floor() as usize).min(TIMED_PHASES - 1);
        let blend = (fraction - phase as f64) as f32;
        let mut result = [0.; 2];
        // Endpoint extension covers stream start/end without an added delay.
        // Refill ensures ordinary packet seams have real past/future support.
        for tap in 0..TIMED_TAPS {
            let sample = (i128::from(center) + tap as i128 - (TIMED_SUPPORT - 1) as i128)
                .clamp(i128::from(self.base), i128::from(self.end - 1))
                as u64;
            let frame = self.fifo[(self.head + (sample - self.base) as usize) % FIFO];
            let weight = self.kernel[phase][tap]
                + (self.kernel[phase + 1][tap] - self.kernel[phase][tap]) * blend;
            result[0] += frame[0] * weight;
            result[1] += frame[1] * weight;
        }
        Some(result)
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
    timed: TimedPlayback,
    timed_mode: bool,
}
impl Lane {
    fn new(input: BlockConsumer, kernel: Arc<TimedKernel>) -> Result<Self, AudioError> {
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
            timed: TimedPlayback::new(kernel),
            timed_mode: false,
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
        self.timed.reset();
        self.timed_mode = false;
    }
    fn timed_next(
        &mut self,
        now_ns: u64,
        presentation_ns: Option<u64>,
        output: u64,
        index: usize,
        stats: &MixerStats,
    ) -> [f32; 2] {
        let Some(presentation_ns) = presentation_ns else {
            return [0.0; 2];
        };
        self.timed_mode = true;
        // Bounded copies occur only at FIFO refill. Capacity and age bounds are
        // unchanged; lookahead crosses variable-sized packet boundaries.
        if self.timed.count < MAX_BLOCK_FRAMES + 2 * TIMED_SUPPORT {
            for _ in 0..PCM_QUEUE_BLOCKS {
                if self.timed.count + MAX_BLOCK_FRAMES > FIFO {
                    break;
                }
                let Some(block) = self.input.pop_fresh(now_ns, 80_000_000) else {
                    break;
                };
                let h = block.header;
                if h.stream_id != self.config.stream_id
                    || h.stream_epoch != self.config.epoch
                    || h.format != AudioFormat::INTERNAL
                    || h.frame_count == 0
                    || usize::from(h.frame_count) > MAX_BLOCK_FRAMES
                    || h.presentation_time_ns.is_none()
                {
                    stats.rejected_blocks.fetch_add(1, Relaxed);
                    continue;
                }
                if self.timed.anchor.is_some()
                    && (self.timed.starved
                        || h.source_sample_position != self.timed.end
                        || h.discontinuity_flags.0 != 0)
                {
                    // After PCM starvation (e.g. between AirPlay tracks), the
                    // free-running cursor no longer describes the next decoded
                    // frame. Reacquire its deadline instead of correcting a
                    // multi-second gap with the drift servo. next() still waits
                    // for future audio and skips genuinely elapsed samples.
                    self.timed.reset();
                    self.gain.restart_from_zero();
                    stats.discontinuities.fetch_add(1, Relaxed);
                }
                self.timed.append(&block);
            }
        }
        let frame = self
            .timed
            .next(presentation_ns, output, index, self.config, stats);
        if output.is_multiple_of(MAX_BLOCK_FRAMES as u64) {
            stats.timed_drift_ppm_milli[index].store(
                ((self.timed.step - 1.) * 1e9).round() as i64 as u64,
                Relaxed,
            );
            stats.timed_phase_error_ns[index].store(
                (self.timed.filtered_error / 48000. * 1e9).round() as i64 as u64,
                Relaxed,
            );
        }
        let Some(frame) = frame else {
            // A future first deadline is intentional silence. After playback
            // has started, exhausted PCM is an underrun, just like native lanes.
            if self.started && self.timed.starved {
                stats.underrun_frames.fetch_add(1, Relaxed);
                stats.underrun_frames_by_lane[index].fetch_add(1, Relaxed);
            }
            return [0.; 2];
        };
        self.started = true;
        let gain = self.gain.next();
        let rendered = [finite(frame[0]) * gain, finite(frame[1]) * gain];
        self.last_rendered = rendered;
        rendered
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
    fn next(
        &mut self,
        now_ns: u64,
        presentation_ns: Option<u64>,
        output: u64,
        index: usize,
        stats: &MixerStats,
    ) -> [f32; 2] {
        if self.timed_mode || self.input.next_is_timed() {
            return self.timed_next(now_ns, presentation_ns, output, index, stats);
        }
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
    presentation_anchor: Option<(u64, u64)>,
    master: GainRamp,
    limiter: f32,
    lane_meter_windows: [MeterWindow; LANES],
    output_meter_window: MeterWindow,
    meter_limiter_min: f32,
}
pub type MixerParts = (Mixer, MixerControl, Vec<BlockProducer>, Arc<MixerStats>);
impl Mixer {
    pub fn new(origin: Instant) -> Result<MixerParts, AudioError> {
        let mut lanes = Vec::with_capacity(LANES);
        let mut inputs = Vec::with_capacity(LANES);
        let timed_kernel = timed_kernel();
        for _ in 0..LANES {
            let (p, c) = block_queue(PCM_QUEUE_BLOCKS, Arc::new(AudioStats::default()))?;
            inputs.push(p);
            lanes.push(Lane::new(c, timed_kernel.clone())?);
        }
        let (p, c) = RingBuffer::new(8);
        let stats = Arc::new(MixerStats::default());
        stats
            .limiter_gain_bits
            .store(u64::from(1.0f32.to_bits()), Relaxed);
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
                presentation_anchor: None,
                master: GainRamp::new(db_gain(-12.0)),
                limiter: 1.0,
                lane_meter_windows: [MeterWindow::default(); LANES],
                output_meter_window: MeterWindow::default(),
                meter_limiter_min: 1.0,
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
        self.lane_meter_windows.fill(MeterWindow::default());
        self.output_meter_window = MeterWindow::default();
        self.meter_limiter_min = 1.0;
        for (lane, meter) in self.lanes.iter().zip(&self.stats.lane_meters) {
            meter.publish(lane.config.stream_id, 0.0, 0.0);
        }
        self.stats.output_meter.publish(0, 0.0, 0.0);
        self.stats
            .limiter_gain_bits
            .store(u64::from(1.0f32.to_bits()), Relaxed);
    }
    fn apply(&mut self, config: MixerConfig) {
        let solo = config.lanes.iter().any(|l| l.stream_id != 0 && l.solo);
        for (index, (lane, next)) in self.lanes.iter_mut().zip(config.lanes).enumerate() {
            let tail = lane.last_rendered;
            let removed = lane.config.stream_id != 0 && next.stream_id == 0;
            if lane.config.stream_id != next.stream_id
                || lane.config.epoch != next.epoch
                || self.config.output_epoch != config.output_epoch
            {
                lane.reset();
                self.lane_meter_windows[index] = MeterWindow::default();
                self.stats.lane_meters[index].publish(next.stream_id, 0.0, 0.0);
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
    fn set_presentation_time(&mut self, at: Instant) {
        self.presentation_anchor = at
            .checked_duration_since(self.origin)
            .map(|d| (self.output, d.as_nanos().min(u128::from(u64::MAX)) as u64));
    }
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
        let presentation_ns = self.presentation_anchor.map(|(frame, at)| {
            at.saturating_add(crate::clock::frames_to_ns(
                self.output.saturating_sub(frame),
                48_000,
            ))
        });
        let mut sum = [0.0f32; 2];
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            let f = lane.next(now_ns, presentation_ns, self.output, i, &self.stats);
            if lane.config.stream_id != 0 {
                self.lane_meter_windows[i].observe(f);
            }
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
        let rendered = [
            finite(sum[0] * self.limiter).clamp(-0.98, 0.98),
            finite(sum[1] * self.limiter).clamp(-0.98, 0.98),
        ];
        self.output_meter_window.observe(rendered);
        self.meter_limiter_min = self.meter_limiter_min.min(self.limiter);
        if self.output_meter_window.frames == METER_WINDOW_FRAMES {
            for (index, lane) in self.lanes.iter().enumerate() {
                self.lane_meter_windows[index]
                    .publish(&self.stats.lane_meters[index], lane.config.stream_id);
            }
            self.output_meter_window
                .publish(&self.stats.output_meter, 0);
            self.stats
                .limiter_gain_bits
                .store(u64::from(self.meter_limiter_min.to_bits()), Relaxed);
            self.meter_limiter_min = 1.0;
        }
        rendered
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
