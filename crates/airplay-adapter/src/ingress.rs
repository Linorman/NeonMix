//! Speaker deadline mapping lives in the Hub process. A worker Instant is never used.
use crate::control::PlaybackMode;
use neonmix_airplay_ipc::PcmPacket;
use neonmix_core::{
    AudioBlock, AudioFormat, Discontinuity, MAX_BLOCK_FRAMES, queue::BlockProducer,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MAX_PENDING_NS: u64 = 4_000_000_000;
pub const MAX_PENDING_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PENDING_FRAMES: usize = 48_000 * 4;
pub const HANDOFF_LOOKAHEAD_NS: u64 = 60_000_000;
pub const CLOCK_JUMP_TOLERANCE_NS: u64 = 20_000_000;
pub const MAX_UNCERTAINTY_NS: u64 = 20_000_000;
/// Receiver headroom before device submission, in addition to output backlog.
/// This is not E2E latency and does not enlarge the short PCM handoff queue.
pub const LOW_LATENCY_HEADROOM_NS: u64 = 120_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub session_id: u64,
    pub stream_id: u64,
    pub stream_epoch: u64,
    pub format_epoch: u64,
    pub mapping_id: u64,
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct IngressStats {
    pub accepted_packets: u64,
    pub released_packets: u64,
    pub identity_rejections: u64,
    pub malformed_packets: u64,
    pub late_packets: u64,
    pub future_packets: u64,
    pub overflow_packets: u64,
    pub clock_resets: u64,
    pub timeline_rejections: u64,
    pub coordinate_rejections: u64,
    pub pts_rejections: u64,
    pub source_gap_frames: u64,
    pub last_normalized_sample_position: u64,
    pub pending_frames: usize,
    pub pending_bytes: usize,
    /// Estimated submission-to-presentation device backlog, not analog latency.
    pub output_latency_ns: u64,
    pub latency_advance_ns: u64,
    pub protocol_lead_ns: u64,
    pub playout_lead_ns: u64,
    pub last_source_sample_position: u64,
    pub last_source_rate: u32,
    /// First normalized frame's mapped deadline in the Hub monotonic origin.
    pub last_presentation_time_ns: u64,
    pub last_mapping_id: u64,
    pub last_uncertainty_ns: u64,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IngressError {
    #[error("PCM session/stream/format/clock identity does not match")]
    Identity,
    #[error("invalid normalized PCM or timing uncertainty")]
    Malformed,
    #[error("presentation deadline already passed")]
    Late,
    #[error("presentation deadline exceeds the four-second limit")]
    TooFar,
    #[error("bounded timed PCM queue is full")]
    Full,
    #[error("source or presentation timeline moved backwards")]
    Timeline,
    #[error("wallclock mapping invalidated; a new mapping/epoch is required")]
    ClockReset,
}
struct Pending {
    block: AudioBlock,
    original_source_position: u64,
    original_source_rate: u32,
    original_uncertainty_ns: u64,
}
/// Owned and driven on the Hub control thread; never read by the audio callback.
pub struct Ingress {
    origin: Instant,
    context: Context,
    wall_anchor_ns: u64,
    mono_anchor_ns: u64,
    clock_valid: bool,
    pending: VecDeque<Pending>,
    stats: IngressStats,
    last: Option<(u64, u64, u64, u32, u64, u64)>,
    coordinate_anchor: Option<(u64, u64)>,
    playback_mode: PlaybackMode,
    latency_advance_ns: Option<u64>,
}
impl Ingress {
    pub fn new(origin: Instant, context: Context) -> Self {
        Self::new_at(origin, context, Instant::now(), SystemTime::now())
    }
    pub fn new_at(origin: Instant, context: Context, now: Instant, wall: SystemTime) -> Self {
        Self {
            origin,
            context,
            wall_anchor_ns: unix_ns(wall),
            mono_anchor_ns: mono_ns(origin, now),
            clock_valid: true,
            pending: VecDeque::new(),
            stats: IngressStats::default(),
            last: None,
            coordinate_anchor: None,
            playback_mode: PlaybackMode::Synchronized,
            latency_advance_ns: None,
        }
    }
    /// Return idle-session storage on the non-realtime owner. Keep counters.
    pub fn release_storage(&mut self) {
        self.clear();
        self.pending = VecDeque::new();
    }
    pub fn reset(&mut self, context: Context) {
        self.reset_at(context, Instant::now(), SystemTime::now());
    }
    pub fn reset_at(&mut self, context: Context, now: Instant, wall: SystemTime) {
        self.clear();
        self.context = context;
        self.wall_anchor_ns = unix_ns(wall);
        self.mono_anchor_ns = mono_ns(self.origin, now);
        self.clock_valid = true;
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.stats.pending_frames = 0;
        self.stats.pending_bytes = 0;
        self.last = None;
        self.coordinate_anchor = None;
        self.stats.last_normalized_sample_position = 0;
        self.latency_advance_ns = None;
        self.stats.latency_advance_ns = 0;
        self.stats.protocol_lead_ns = 0;
        self.stats.playout_lead_ns = 0;
        self.stats.last_source_sample_position = 0;
        self.stats.last_source_rate = 0;
        self.stats.last_presentation_time_ns = 0;
        self.stats.last_mapping_id = 0;
        self.stats.last_uncertainty_ns = 0;
    }
    pub fn stats(&self) -> IngressStats {
        self.stats
    }
    pub fn set_output_latency_ns(&mut self, ns: u64) {
        self.stats.output_latency_ns = ns;
    }
    /// Configure before accepting a new epoch. Never retime buffered samples.
    pub fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.clear();
        self.playback_mode = mode;
    }
    pub fn push(&mut self, packet: PcmPacket) -> Result<(), IngressError> {
        self.push_at(packet, Instant::now(), SystemTime::now())
    }
    pub fn push_at(
        &mut self,
        packet: PcmPacket,
        now: Instant,
        wall: SystemTime,
    ) -> Result<(), IngressError> {
        let h = packet.header;
        if (
            h.session_id,
            h.stream_id,
            h.stream_epoch,
            h.format_epoch,
            h.mapping_id,
        ) != (
            self.context.session_id,
            self.context.stream_id,
            self.context.stream_epoch,
            self.context.format_epoch,
            self.context.mapping_id,
        ) {
            self.stats.identity_rejections += 1;
            return Err(IngressError::Identity);
        }
        if !self.clock_valid {
            return Err(IngressError::ClockReset);
        }
        let now_ns = mono_ns(self.origin, now);
        let expected_wall = self
            .wall_anchor_ns
            .saturating_add(now_ns.saturating_sub(self.mono_anchor_ns));
        if unix_ns(wall).abs_diff(expected_wall) > CLOCK_JUMP_TOLERANCE_NS {
            self.clock_valid = false;
            self.clear();
            self.stats.clock_resets += 1;
            return Err(IngressError::ClockReset);
        }
        let frames = usize::from(h.frame_count);
        if packet.validate().is_err()
            || frames > MAX_BLOCK_FRAMES
            || h.uncertainty_ns > MAX_UNCERTAINTY_NS
        {
            self.stats.malformed_packets += 1;
            return Err(IngressError::Malformed);
        }
        let target_i = i128::from(self.mono_anchor_ns) + i128::from(h.presentation_time_ns)
            - i128::from(self.wall_anchor_ns);
        if target_i < 0 || target_i > i128::from(u64::MAX) {
            self.stats.late_packets += 1;
            return Err(IngressError::Late);
        }
        let target = target_i as u64;
        let duration = neonmix_core::clock::frames_to_ns(frames as u64, 48_000);
        if target.saturating_add(duration) <= now_ns {
            self.stats.late_packets += 1;
            return Err(IngressError::Late);
        }
        if target.saturating_add(duration).saturating_sub(now_ns) > MAX_PENDING_NS {
            self.stats.future_packets += 1;
            return Err(IngressError::TooFar);
        }
        // Pick one translation from the first valid packet of an epoch. Keep
        // every later PTS delta intact; packet arrival jitter must not modulate
        // pitch or cause cursor jumps. Synchronized mode keeps the sender PTS.
        let advance = self.latency_advance_ns.unwrap_or_else(|| {
            if self.playback_mode == PlaybackMode::LowLatency {
                target.saturating_sub(
                    now_ns
                        .saturating_add(self.stats.output_latency_ns)
                        .saturating_add(LOW_LATENCY_HEADROOM_NS),
                )
            } else {
                0
            }
        });
        let protocol_lead_ns = target.saturating_sub(now_ns);
        let target = target.saturating_sub(advance);
        if target.saturating_add(duration) <= now_ns {
            self.stats.late_packets += 1;
            return Err(IngressError::Late);
        }
        let q = h.normalized_sample_position;
        if self.last.is_some_and(|(seq, pos, time, rate, end, _)| {
            h.sequence <= seq
                || h.source_sample_position < pos
                || target <= time
                || h.source_rate != rate
                || q < end
        }) {
            self.stats.timeline_rejections += 1;
            return Err(IngressError::Timeline);
        }
        // Original integer source coordinates may truncate a fractional SRC
        // sample. Cross-check the explicit normalized coordinate against the
        // same fixed anchor, allowing two source/normalized quantization frames.
        if let Some((p0, q0)) = self.coordinate_anchor {
            let source_delta = i128::from(h.source_sample_position) - i128::from(p0);
            let normalized_delta = i128::from(q) - i128::from(q0);
            let error =
                (source_delta * 48_000 - normalized_delta * i128::from(h.source_rate)).abs();
            if error > 2 * (48_000 + i128::from(h.source_rate)) {
                self.stats.coordinate_rejections += 1;
                return Err(IngressError::Timeline);
            }
        }
        if let Some((_, _, time, _, _, previous_q)) = self.last {
            let delta = neonmix_core::clock::frames_to_ns(q - previous_q, 48_000);
            // Permit the existing 1000 ppm source-clock slope and nanosecond
            // timestamp quantization, without accepting a contradictory gap.
            if target.saturating_sub(time).abs_diff(delta) > 1_000_000 + delta / 1000 {
                self.stats.pts_rejections += 1;
                return Err(IngressError::Timeline);
            }
        }
        // Account for the full fixed callback block allocation, not only its active samples.
        let bytes = std::mem::size_of::<Pending>();
        if self.stats.pending_frames + frames > MAX_PENDING_FRAMES
            || self.stats.pending_bytes + bytes > MAX_PENDING_BYTES
        {
            self.stats.overflow_packets += 1;
            return Err(IngressError::Full);
        }
        // This owner is non-realtime. Allocate the fixed bounded storage only
        // after the first authenticated, valid packet of an admitted session.
        if self.pending.capacity() == 0 {
            self.pending.reserve_exact(MAX_PENDING_BYTES / bytes);
        }
        let mut block = AudioBlock::empty(h.stream_id, AudioFormat::INTERNAL);
        block.header.stream_epoch = h.stream_epoch;
        block.header.frame_count = h.frame_count;
        block.header.source_sample_position = q;
        block.header.presentation_time_ns = Some(target);
        block.header.arrival_ns = now_ns;
        block.header.discontinuity_flags = if h.flags != 0 {
            Discontinuity::START
        } else {
            Discontinuity::NONE
        };
        let gain = if h.gain_applied { 1.0 } else { h.protocol_gain };
        for (frame, samples) in block.pcm[..frames]
            .iter_mut()
            .zip(packet.samples.chunks_exact(2))
        {
            *frame = [samples[0] * gain, samples[1] * gain];
        }
        self.coordinate_anchor
            .get_or_insert((h.source_sample_position, q));
        if let Some((_, _, _, _, end, _)) = self.last {
            self.stats.source_gap_frames += q.saturating_sub(end);
        }
        self.stats.last_normalized_sample_position = q;
        self.latency_advance_ns = Some(advance);
        self.stats.latency_advance_ns = advance;
        self.stats.protocol_lead_ns = protocol_lead_ns;
        self.stats.playout_lead_ns = target.saturating_sub(now_ns);
        self.last = Some((
            h.sequence,
            h.source_sample_position,
            target,
            h.source_rate,
            q + frames as u64,
            q,
        ));
        self.stats.last_source_sample_position = h.source_sample_position;
        self.stats.last_source_rate = h.source_rate;
        self.stats.last_presentation_time_ns = target;
        self.stats.last_mapping_id = h.mapping_id;
        self.stats.last_uncertainty_ns = h.uncertainty_ns;
        self.pending.push_back(Pending {
            block,
            original_source_position: h.source_sample_position,
            original_source_rate: h.source_rate,
            original_uncertainty_ns: h.uncertainty_ns,
        });
        self.stats.accepted_packets += 1;
        self.stats.pending_frames += frames;
        self.stats.pending_bytes += bytes;
        Ok(())
    }
    /// Release only nearby PCM and leave queue capacity unchanged. The Mixer
    /// still gates every sample against its speaker presentation timestamp.
    pub fn release_due(&mut self, now: Instant, producer: &mut BlockProducer) -> usize {
        let now_ns = mono_ns(self.origin, now);
        let horizon = now_ns
            .saturating_add(HANDOFF_LOOKAHEAD_NS)
            .saturating_add(self.stats.output_latency_ns);
        let mut released = 0;
        while let Some(first) = self.pending.front() {
            let target = first.block.header.presentation_time_ns.expect("mapped PCM");
            let end = target.saturating_add(neonmix_core::clock::frames_to_ns(
                u64::from(first.block.header.frame_count),
                48_000,
            ));
            if target > horizon {
                break;
            }
            if end <= now_ns {
                self.pop_pending();
                self.stats.late_packets += 1;
                continue;
            }
            if producer.remaining_capacity() == 0 {
                break;
            }
            let mut entry = self.pop_pending().expect("pending head");
            entry.block.header.arrival_ns = now_ns;
            self.stats.last_source_sample_position = entry.original_source_position;
            self.stats.last_source_rate = entry.original_source_rate;
            self.stats.last_presentation_time_ns = target;
            self.stats.last_mapping_id = self.context.mapping_id;
            self.stats.last_uncertainty_ns = entry.original_uncertainty_ns;
            if producer.push(entry.block) {
                released += 1;
                self.stats.released_packets += 1;
            }
        }
        released
    }
    fn pop_pending(&mut self) -> Option<Pending> {
        let entry = self.pending.pop_front()?;
        self.stats.pending_frames -= usize::from(entry.block.header.frame_count);
        self.stats.pending_bytes -= std::mem::size_of::<Pending>();
        Some(entry)
    }
}
fn mono_ns(origin: Instant, now: Instant) -> u64 {
    now.checked_duration_since(origin)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}
fn unix_ns(wall: SystemTime) -> u64 {
    wall.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}
