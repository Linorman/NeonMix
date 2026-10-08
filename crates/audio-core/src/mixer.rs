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
    pub observed: bool,
    pub sampled_at_ns: u64,
    pub stream_id: u64,
    pub binding_generation: u64,
    pub stream_epoch: u64,
    pub output_epoch: u64,
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
    binding_generation: AtomicU64,
    stream_epoch: AtomicU64,
    output_epoch: AtomicU64,
    levels: AtomicU64,
    sampled_at_ns: AtomicU64,
}
impl MeterStats {
    fn publish(
        &self,
        stream_id: u64,
        binding: (u64, u64, u64),
        sampled_at_ns: u64,
        peak: f32,
        rms: f32,
    ) {
        self.sequence.fetch_add(1, SeqCst);
        self.stream_id.store(stream_id, SeqCst);
        self.binding_generation.store(binding.0, SeqCst);
        self.stream_epoch.store(binding.1, SeqCst);
        self.output_epoch.store(binding.2, SeqCst);
        self.levels.store(
            u64::from(peak.to_bits()) | (u64::from(rms.to_bits()) << 32),
            SeqCst,
        );
        self.sampled_at_ns.store(sampled_at_ns, SeqCst);
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
            let binding_generation = self.binding_generation.load(SeqCst);
            let stream_epoch = self.stream_epoch.load(SeqCst);
            let output_epoch = self.output_epoch.load(SeqCst);
            let sampled_at_ns = self.sampled_at_ns.load(SeqCst);
            if before == self.sequence.load(SeqCst) {
                return MeterSnapshot {
                    observed: before != 0,
                    sampled_at_ns,
                    stream_id,
                    binding_generation,
                    stream_epoch,
                    output_epoch,
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
    fn publish(
        &mut self,
        stats: &MeterStats,
        stream_id: u64,
        binding: (u64, u64, u64),
        sampled_at_ns: u64,
    ) {
        let rms = if self.frames == 0 {
            0.0
        } else {
            (self.squares / (2 * self.frames) as f64).sqrt() as f32
        };
        stats.publish(stream_id, binding, sampled_at_ns, self.peak, rms);
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackKind {
    #[default]
    NativeAdaptive,
    Timed,
}

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneRenderState {
    Inactive = 0,
    Priming = 1,
    Running = 2,
    Starved = 3,
    Stopped = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneMix {
    pub stream_id: u64,
    pub epoch: u64,
    pub binding_generation: u64,
    /// Control-plane binding; queued PCM never selects the lane's mode.
    pub playback_kind: PlaybackKind,
    pub gain_db: f32,
    pub muted: bool,
    pub solo: bool,
}
impl Default for LaneMix {
    fn default() -> Self {
        Self {
            stream_id: 0,
            epoch: 0,
            binding_generation: 1,
            playback_kind: PlaybackKind::NativeAdaptive,
            gain_db: 0.0,
            muted: false,
            solo: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Copy)]
struct ConfigCommand {
    sequence: u64,
    config: MixerConfig,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigProgress {
    pub desired_config_sequence: u64,
    pub applied_config_sequence: u64,
    pub queue_rejections: u64,
}
impl ConfigProgress {
    pub fn pending(self) -> bool {
        self.applied_config_sequence < self.desired_config_sequence
    }
}
pub struct MixerControl {
    commands: Producer<ConfigCommand>,
    desired: Option<(u64, MixerConfig)>,
    desired_since: Option<Instant>,
    stats: Arc<MixerStats>,
    authorizations: Vec<Arc<AtomicU64>>,
}
impl MixerControl {
    /// Revocation does not wait for free slots in the DSP command queue.
    pub fn revoke_lane(&self, lane: usize) {
        if let Some(authorization) = self.authorizations.get(lane) {
            authorization.store(0, std::sync::atomic::Ordering::Release);
        }
    }
    /// Single producer: while the owner serializes changes, a free slot can only
    /// become freer as the output consumer runs. Used for durable control commits.
    pub fn pending_for(&self, now: Instant) -> Option<std::time::Duration> {
        self.stats
            .config_progress()
            .pending()
            .then(|| {
                self.desired_since
                    .map(|at| now.saturating_duration_since(at))
            })
            .flatten()
    }
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
        let sequence = match self.desired {
            Some((sequence, previous)) if previous == config => sequence,
            _ => {
                let sequence = self
                    .desired
                    .map_or(0, |(sequence, _)| sequence)
                    .checked_add(1)
                    .ok_or_else(|| AudioError::Backend("mixer config sequence exhausted".into()))?;
                self.desired = Some((sequence, config));
                self.desired_since = Some(Instant::now());
                self.stats
                    .desired_config_sequence
                    .store(sequence, std::sync::atomic::Ordering::Release);
                sequence
            }
        };
        self.commands
            .push(ConfigCommand { sequence, config })
            .map_err(|_| {
                self.stats.config_queue_rejections.fetch_add(1, Relaxed);
                AudioError::Backend("mixer command queue full".into())
            })
    }
}
#[derive(Default)]
pub struct MixerStats {
    pub desired_config_sequence: AtomicU64,
    pub applied_config_sequence: AtomicU64,
    pub config_queue_rejections: AtomicU64,
    /// Actual PCM consumed on the internal 48 kHz clock, including silent PCM.
    pub rendered_pcm_frames_by_lane: [AtomicU64; LANES],
    /// LaneRenderState on the audio consumer; never inferred from ingress.
    pub render_state_by_lane: [AtomicU64; LANES],
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
impl MixerStats {
    /// Release from control and callback; read applied first to retain a valid
    /// ordered pair even while the producer publishes the next desired target.
    pub fn config_progress(&self) -> ConfigProgress {
        let applied = self
            .applied_config_sequence
            .load(std::sync::atomic::Ordering::Acquire);
        let desired = self
            .desired_config_sequence
            .load(std::sync::atomic::Ordering::Acquire);
        ConfigProgress {
            desired_config_sequence: desired,
            applied_config_sequence: applied,
            queue_rejections: self.config_queue_rejections.load(Relaxed),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct TimedSegment {
    start: u64,
    end: u64,
    anchor: (u64, u64),
    first_pts: u64,
}

struct TimedPlayback {
    kernel: Arc<TimedKernel>,
    // At most one descriptor per stored sample. Even legal one-frame packets
    // cannot exhaust descriptors before PCM; no callback allocation or drop.
    segments: [TimedSegment; FIFO],
    segment_head: usize,
    segment_count: usize,
    transitioned: bool,
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
            segments: [TimedSegment::default(); FIFO],
            segment_head: 0,
            segment_count: 0,
            transitioned: false,
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
        self.segment_head = 0;
        self.segment_count = 0;
        self.transitioned = false;
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
    fn activate_segment(&mut self) {
        let segment = self.segments[self.segment_head];
        self.base = segment.start;
        self.end = segment.end;
        self.anchor = Some(segment.anchor);
        self.slope_anchor = Some((segment.start, segment.first_pts));
        self.start_anchor = self.slope_anchor;
        self.waiting_anchor = None;
        self.position = None;
        self.filtered_error = 0.;
        self.source_rate = 1.;
        self.slope_known = false;
        self.step = 1.;
        self.starved = false;
    }
    fn advance_segment(&mut self) {
        let remaining = self.end.saturating_sub(self.base) as usize;
        self.head = (self.head + remaining) % FIFO;
        self.count -= remaining;
        self.segment_head = (self.segment_head + 1) % FIFO;
        self.segment_count -= 1;
        self.activate_segment();
        self.transitioned = true;
    }
    fn append(&mut self, block: &AudioBlock) {
        let h = block.header;
        let at = h.presentation_time_ns.expect("validated timed PCM");
        let tail = (self.segment_head + self.segment_count.saturating_sub(1)) % FIFO;
        let last = self.segments[tail];
        let expected_pts = last.anchor.1.saturating_add(crate::clock::frames_to_ns(
            h.source_sample_position.saturating_sub(last.anchor.0),
            48_000,
        ));
        let continuous = self.segment_count != 0
            && !(self.segment_count == 1 && self.starved)
            && last.end == h.source_sample_position
            && h.discontinuity_flags.0 == 0
            && at.abs_diff(expected_pts) <= TIMED_START_JITTER_NS;
        if continuous {
            self.segments[tail].end = h.source_sample_position + u64::from(h.frame_count);
            self.segments[tail].anchor = (h.source_sample_position, at);
        } else {
            let next = (self.segment_head + self.segment_count) % FIFO;
            self.segments[next] = TimedSegment {
                start: h.source_sample_position,
                end: h.source_sample_position + u64::from(h.frame_count),
                anchor: (h.source_sample_position, at),
                first_pts: at,
            };
            self.segment_count += 1;
        }
        for frame in block.frames() {
            self.fifo[(self.head + self.count) % FIFO] = [finite(frame[0]), finite(frame[1])];
            self.count += 1;
        }
        if self.segment_count == 1 {
            if self.anchor.is_none() {
                self.activate_segment();
            } else {
                if let Some((position, time)) = self.slope_anchor {
                    let ns = at.saturating_sub(time);
                    if ns >= 500_000_000 && h.source_sample_position >= position {
                        let rate = (h.source_sample_position - position) as f64 * 1e9
                            / (ns as f64 * 48000.);
                        if (0.99..=1.01).contains(&rate) {
                            if self.slope_known {
                                let seconds = ns as f64 / 1e9;
                                self.source_rate +=
                                    (rate - self.source_rate) * seconds / (2. + seconds);
                            } else {
                                self.source_rate = rate;
                                self.slope_known = true;
                            }
                        }
                        self.slope_anchor = Some((h.source_sample_position, at));
                    }
                }
                self.anchor = Some((h.source_sample_position, at));
                self.end = self.segments[self.segment_head].end;
            }
        }
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
        // Expired segments are retired with a fixed budget, never an unbounded
        // search for current audio. The next sample resumes the same scan.
        for _ in 0..PCM_QUEUE_BLOCKS {
            if self.position.is_some_and(|p| p >= self.end as f64 - 0.0001)
                && self.segment_count > 1
            {
                self.advance_segment();
                stats.discontinuities.fetch_add(1, Relaxed);
            }
            let frame = self.next_in_segment(presentation_ns, output, index, config, stats);
            if frame.is_some()
                || self.segment_count <= 1
                || !self.position.is_some_and(|p| p >= self.end as f64 - 0.0001)
            {
                return frame;
            }
        }
        None
    }
    fn next_in_segment(
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
                .min(self.end.saturating_sub(self.base));
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
        let discard = keep
            .saturating_sub(self.base)
            .min(self.end.saturating_sub(self.base)) as usize;
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
    scan_budget: usize,
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
    retire_binding_generation: u64,
    source_end: u64,
    have_source: bool,
    started: bool,
    /// Measurement lifetime is the binding, not the resampler reset cycle.
    ever_started: bool,
    rendering_pcm: bool,
    stopped: bool,
    drift: DriftController,
    timed: TimedPlayback,
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
            scan_budget: PCM_QUEUE_BLOCKS,
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
            retire_binding_generation: 0,
            source_end: 0,
            have_source: false,
            started: false,
            ever_started: false,
            rendering_pcm: false,
            stopped: false,
            drift: DriftController::default(),
            timed: TimedPlayback::new(kernel),
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
        self.rendering_pcm = false;
        self.drift.reset();
        self.timed.reset();
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
            self.observe(false, index, stats);
            return [0.0; 2];
        };
        // Bounded copies occur only at FIFO refill. Capacity and age bounds are
        // unchanged; lookahead crosses variable-sized packet boundaries.
        if self.timed.count < MAX_BLOCK_FRAMES + 2 * TIMED_SUPPORT {
            for _ in 0..PCM_QUEUE_BLOCKS {
                if self.timed.count + MAX_BLOCK_FRAMES > FIFO {
                    break;
                }
                let Some(block) = self.input.pop_fresh_budget(
                    now_ns,
                    80_000_000,
                    Some(self.config.binding_generation),
                    &mut self.scan_budget,
                ) else {
                    break;
                };
                let h = block.header;
                if h.stream_id != self.config.stream_id
                    || h.stream_epoch != self.config.epoch
                    || h.format != AudioFormat::INTERNAL
                    || h.frame_count == 0
                    || usize::from(h.frame_count) > MAX_BLOCK_FRAMES
                    || h.source_sample_position
                        .checked_add(u64::from(h.frame_count))
                        .is_none()
                    || h.presentation_time_ns.is_none()
                {
                    stats.rejected_blocks.fetch_add(1, Relaxed);
                    continue;
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
        if self.timed.transitioned {
            self.timed.transitioned = false;
            self.gain.restart_from_zero();
        }
        let Some(frame) = frame else {
            // A future first deadline is intentional silence. After playback
            // has started, exhausted PCM is an underrun, just like native lanes.
            self.observe(false, index, stats);
            return [0.; 2];
        };
        self.started = true;
        self.observe(true, index, stats);
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
            || h.presentation_time_ns.is_some()
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
            let Some(block) = self.input.pop_fresh_budget(
                now_ns,
                80_000_000,
                (self.config.stream_id != 0).then_some(self.config.binding_generation),
                &mut self.scan_budget,
            ) else {
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
            if self.config.stream_id != 0 && self.ever_started {
                stats.last_underrun_output_frame[index].store(output, Relaxed);
                stats.last_underrun_fifo_frames[index].store(self.count as u64, Relaxed);
                stats.last_underrun_needed_frames[index].store(needed as u64, Relaxed);
                stats.last_underrun_source_end[index].store(self.source_end, Relaxed);
                // Reset DSP once on loss. Continued starvation must retain
                // newly arriving priming PCM so recovery can refill normally.
                if self.started {
                    self.reset();
                }
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
                self.rendering_pcm = true;
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
        if self.config.stream_id != 0 && !self.input.binding_valid(self.config.binding_generation) {
            if !self.stopped {
                self.reset();
                self.stopped = true;
            }
            stats.render_state_by_lane[index].store(LaneRenderState::Stopped as u64, Relaxed);
            return [0.; 2];
        }
        if self.config.playback_kind == PlaybackKind::Timed {
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
        self.observe(self.rendering_pcm, index, stats);
        // Buffering zeros must not spend the recovery envelope. Start its
        // five-ms sample clock only when this lane can consume source PCM.
        let gain = if self.started {
            self.gain.next()
        } else {
            self.gain.value
        };
        let mut rendered = [frame[0] * gain, frame[1] * gain];
        if self.retire_remaining > 0 && !self.input.binding_valid(self.retire_binding_generation) {
            self.retire_remaining = 0;
            self.retire_tail = [0.; 2];
        }
        if self.retire_remaining > 0 {
            let fade = self.retire_remaining as f32 / 240.0;
            rendered[0] += self.retire_tail[0] * fade;
            rendered[1] += self.retire_tail[1] * fade;
            self.retire_remaining -= 1;
        }
        self.last_rendered = rendered;
        rendered
    }
    fn observe(&mut self, valid_pcm: bool, index: usize, stats: &MixerStats) {
        let state = if self.config.stream_id == 0 {
            if self.stopped {
                LaneRenderState::Stopped
            } else {
                LaneRenderState::Inactive
            }
        } else if valid_pcm {
            self.ever_started = true;
            stats.rendered_pcm_frames_by_lane[index].fetch_add(1, Relaxed);
            LaneRenderState::Running
        } else if self.ever_started {
            stats.underrun_frames.fetch_add(1, Relaxed);
            stats.underrun_frames_by_lane[index].fetch_add(1, Relaxed);
            LaneRenderState::Starved
        } else {
            LaneRenderState::Priming
        };
        stats.render_state_by_lane[index].store(state as u64, Relaxed);
    }
}
pub struct Mixer {
    apply_at_next_frame: bool,
    inside_render_block: bool,
    callback_time_ns: Option<u64>,
    lanes: Vec<Lane>,
    commands: Consumer<ConfigCommand>,
    config: MixerConfig,
    config_sequence: u64,
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
                apply_at_next_frame: false,
                inside_render_block: false,
                callback_time_ns: None,
                commands: c,
                config: MixerConfig::default(),
                config_sequence: 0,
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
            MixerControl {
                commands: p,
                desired: None,
                desired_since: None,
                stats: stats.clone(),
                authorizations: inputs.iter().map(BlockProducer::authorization).collect(),
            },
            inputs,
            stats,
        ))
    }
    pub fn render_block(&mut self, frames: &mut [[f32; 2]]) {
        let previous = self.inside_render_block;
        self.inside_render_block = true;
        self.reset_scan_budgets();
        for f in frames {
            *f = self.next_frame();
        }
        self.inside_render_block = previous;
    }
    fn reset_scan_budgets(&mut self) {
        for lane in &mut self.lanes {
            lane.scan_budget = PCM_QUEUE_BLOCKS;
        }
    }
    /// Explicit callback submission clock, independent of the speaker PTS.
    /// Embedders and deterministic simulations can supply the same clock used
    /// by ingress; render speed then cannot masquerade as packet queue age.
    /// Storage and loops are identical to render_block; no allocation or wait.
    pub fn render_block_at(&mut self, now: Instant, frames: &mut [[f32; 2]]) {
        let now_ns = now
            .checked_duration_since(self.origin)
            .map_or(0, |elapsed| {
                elapsed.as_nanos().min(u128::from(u64::MAX)) as u64
            });
        let previous = self.callback_time_ns.replace(now_ns);
        self.now_ns = now_ns;
        self.render_block(frames);
        self.callback_time_ns = previous;
    }
    /// Called on the control thread while no output callback owns this mixer.
    /// Clears stale audio and commands before reopening the same output device.
    pub fn discard_backlog(&mut self) {
        // Output reopening may interrupt a logical 480-frame block. Apply
        // the new output binding before consuming even its first fresh PCM.
        self.apply_at_next_frame = true;
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
            lane.ever_started = false;
        }
        self.master.restart_from_zero();
        self.limiter = 1.0;
        self.lane_meter_windows.fill(MeterWindow::default());
        self.output_meter_window = MeterWindow::default();
        self.meter_limiter_min = 1.0;
        for (lane, meter) in self.lanes.iter().zip(&self.stats.lane_meters) {
            meter.publish(
                lane.config.stream_id,
                (
                    lane.config.binding_generation,
                    lane.config.epoch,
                    self.config.output_epoch,
                ),
                self.now_ns,
                0.0,
                0.0,
            );
        }
        self.stats
            .output_meter
            .publish(0, (0, 0, self.config.output_epoch), self.now_ns, 0.0, 0.0);
        self.stats
            .limiter_gain_bits
            .store(u64::from(1.0f32.to_bits()), Relaxed);
    }
    fn apply(&mut self, command: ConfigCommand) {
        let config = command.config;
        if self.config.output_epoch != config.output_epoch {
            self.output_meter_window = MeterWindow::default();
            self.meter_limiter_min = 1.0;
            self.stats
                .output_meter
                .publish(0, (0, 0, config.output_epoch), self.now_ns, 0.0, 0.0);
        }
        let solo = config.lanes.iter().any(|l| l.stream_id != 0 && l.solo);
        for (index, (lane, next)) in self.lanes.iter_mut().zip(config.lanes).enumerate() {
            let authorized = lane.input.binding_valid(lane.config.binding_generation);
            let tail = if authorized {
                lane.last_rendered
            } else {
                [0.; 2]
            };
            let old_binding = lane.config.binding_generation;
            let removed = lane.config.stream_id != 0 && next.stream_id == 0;
            if lane.config.stream_id != next.stream_id
                || lane.config.epoch != next.epoch
                || lane.config.binding_generation != next.binding_generation
                || lane.config.playback_kind != next.playback_kind
                || self.config.output_epoch != config.output_epoch
            {
                lane.reset();
                lane.ever_started = false;
                self.lane_meter_windows[index] = MeterWindow::default();
                self.stats.lane_meters[index].publish(
                    next.stream_id,
                    (next.binding_generation, next.epoch, config.output_epoch),
                    self.now_ns,
                    0.0,
                    0.0,
                );
            }
            if removed {
                lane.stopped = true;
                lane.retire_tail = tail;
                lane.retire_remaining = if authorized { 240 } else { 0 };
                lane.retire_binding_generation = old_binding;
            }
            if next.stream_id != 0 {
                lane.stopped = false;
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
        self.config_sequence = command.sequence;
    }
}
impl StereoSource for Mixer {
    fn set_presentation_time(&mut self, at: Instant) {
        self.reset_scan_budgets();
        self.presentation_anchor = at
            .checked_duration_since(self.origin)
            .map(|d| (self.output, d.as_nanos().min(u128::from(u64::MAX)) as u64));
    }
    fn next_frame(&mut self) -> [f32; 2] {
        if self.apply_at_next_frame || self.output.is_multiple_of(MAX_BLOCK_FRAMES as u64) {
            self.apply_at_next_frame = false;
            // A source used without callback hooks gets a bounded logical
            // block budget. Real callbacks and render_block reset it once.
            if !self.inside_render_block && self.presentation_anchor.is_none() {
                self.reset_scan_budgets();
            }
            self.now_ns = self.callback_time_ns.unwrap_or_else(|| {
                self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
            });
            // Bounded command drain at the logical block boundary.
            for _ in 0..8 {
                let Ok(config) = self.commands.pop() else {
                    break;
                };
                self.apply(config);
            }
            self.stats
                .applied_config_sequence
                .store(self.config_sequence, std::sync::atomic::Ordering::Release);
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
                self.lane_meter_windows[index].publish(
                    &self.stats.lane_meters[index],
                    lane.config.stream_id,
                    (
                        lane.config.binding_generation,
                        lane.config.epoch,
                        self.config.output_epoch,
                    ),
                    self.now_ns,
                );
            }
            self.output_meter_window.publish(
                &self.stats.output_meter,
                0,
                (0, 0, self.config.output_epoch),
                self.now_ns,
            );
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
