use crate::MediaError;
use serde::Serialize;
pub const MAX_MEDIA_PACKET: usize = 1200;
pub const MAX_DTLS_PACKET: usize = 4096;

/// RFC 6716 sections 3.1/3.2: duration declared by the TOC/frame count at
/// 48 kHz. Only negotiated duration is checked here; native libopus retains
/// responsibility for validating and decoding the full packet framing.
pub fn opus_packet_frames(payload: &[u8]) -> Option<u32> {
    let toc = *payload.first()?;
    let config = toc >> 3;
    let frame_samples = if config < 12 {
        [480, 960, 1920, 2880][usize::from(config & 3)]
    } else if config < 16 {
        [480, 960][usize::from(config & 1)]
    } else {
        [120, 240, 480, 960][usize::from(config & 3)]
    };
    let frames = match toc & 3 {
        0 => 1,
        1 => 2,
        2 => {
            payload.get(1)?;
            2
        }
        _ => u32::from(*payload.get(1)? & 0x3f),
    };
    let total = frame_samples * frames;
    (frames != 0 && total <= 5760).then_some(total)
}

pub fn validate_datagram(bytes: &[u8]) -> Result<(), MediaError> {
    if bytes.is_empty()
        || bytes.len()
            > if (20..=63).contains(&bytes[0]) {
                MAX_DTLS_PACKET
            } else {
                MAX_MEDIA_PACKET
            }
    {
        return Err(MediaError::InvalidPacket);
    }
    Ok(())
}
#[derive(Default, Debug, Clone, Serialize)]
pub struct PacketStats {
    pub received: u64,
    pub duplicate: u64,
    pub reordered: u64,
    pub invalid: u64,
    pub highest_sequence: u64,
    pub highest_timestamp: u64,
    pub timestamp_step_errors: u64,
}
/// Track authenticated RTP only. A 128-packet window is diagnostics; SRTP's
/// independent replay window is enforced by libsrtp before this tracker.
#[derive(Default)]
pub struct RtpTimeline {
    highest_seq: Option<u64>,
    highest_ts: Option<u64>,
    bitmap: u128,
    pub stats: PacketStats,
}
pub(crate) fn extend(value: u64, highest: u64, bits: u32) -> u64 {
    let modulus = 1u64 << bits;
    let base = (highest & !(modulus - 1)) | value;
    if base.saturating_add(modulus / 2) < highest {
        base.saturating_add(modulus)
    } else if base > highest.saturating_add(modulus / 2) && base >= modulus {
        base - modulus
    } else {
        base
    }
}
impl RtpTimeline {
    pub fn observe(&mut self, sequence: u16, timestamp: u32) -> bool {
        let seq = self
            .highest_seq
            .map_or(u64::from(sequence), |h| extend(u64::from(sequence), h, 16));
        let ts = self.highest_ts.map_or(u64::from(timestamp), |h| {
            extend(u64::from(timestamp), h, 32)
        });
        if let (Some(highest), Some(last)) = (self.highest_seq, self.highest_ts) {
            let offset = seq.abs_diff(highest).wrapping_mul(480) as u32;
            let last = last as u32;
            let expected = if seq >= highest {
                last.wrapping_add(offset)
            } else {
                last.wrapping_sub(offset)
            };
            // Validate both directions in the wire's modulo-32-bit clock.
            // An older packet can precede our first observed timestamp across
            // rollover, so its extended timestamp need not fit a positive u64.
            if timestamp != expected {
                self.stats.timestamp_step_errors += 1;
                self.stats.invalid += 1;
                return false;
            }
            if seq <= highest {
                let distance = highest - seq;
                if distance >= 128 || self.bitmap & (1u128 << distance) != 0 {
                    self.stats.duplicate += 1;
                    return false;
                }
                self.bitmap |= 1u128 << distance;
                self.stats.reordered += 1;
            } else {
                let shift = seq - highest;
                self.bitmap = if shift >= 128 {
                    1
                } else {
                    (self.bitmap << shift) | 1
                };
            }
        } else {
            self.bitmap = 1;
        }
        if self.highest_seq.is_none_or(|highest| seq > highest) {
            self.highest_seq = Some(seq);
            self.highest_ts = Some(ts);
        }
        self.stats.received += 1;
        self.stats.highest_sequence = self.highest_seq.unwrap_or(0);
        self.stats.highest_timestamp = self.highest_ts.unwrap_or(0);
        true
    }
}

#[derive(Debug, Serialize, Clone, Copy)]
pub struct Feedback {
    pub loss_fraction: f64,
    pub late_packets: u64,
    pub queue_frames: u32,
    pub receiving: bool,
}
#[derive(Debug, Serialize, Clone, Copy)]
pub struct SendPolicy {
    pub bitrate: u32,
    pub paused: bool,
    bad_reports: u8,
    good_reports: u8,
    last_late: u64,
}
impl Default for SendPolicy {
    fn default() -> Self {
        Self {
            bitrate: 192_000,
            paused: false,
            bad_reports: 0,
            good_reports: 0,
            last_late: 0,
        }
    }
}
impl SendPolicy {
    /// Feedback is accepted only from this context's authenticated SRTCP/TLS.
    /// Calls must be paced to one per second by the transport worker.
    pub fn update(&mut self, feedback: Feedback) {
        let bad = !feedback.receiving
            || feedback.loss_fraction > 0.05
            || feedback.queue_frames > (neonmix_core::mixer::TARGET_WATER_FRAMES + 960) as u32
            || feedback.late_packets > self.last_late;
        self.last_late = feedback.late_packets;
        if bad {
            self.bad_reports = self.bad_reports.saturating_add(1);
            self.good_reports = 0;
        } else {
            self.good_reports = self.good_reports.saturating_add(1);
            self.bad_reports = 0;
        }
        if self.bad_reports >= 3 {
            if self.bitrate == 64_000 && (feedback.loss_fraction > 0.25 || !feedback.receiving) {
                self.paused = true;
            } else {
                self.bitrate = (self.bitrate * 3 / 4).max(64_000);
            }
            self.bad_reports = 0;
        }
        if self.good_reports >= 5 {
            self.paused = false;
            self.bitrate = (self.bitrate + 16_000).min(192_000);
            self.good_reports = 0;
        }
    }
}

/// Compound RTCP RR + APP with bounded per-stream queue/status feedback. Both
/// packets pass through dtlssrtpenc.rtcp_sink and its mature SRTCP protection.
pub fn receiver_report(
    local_ssrc: u32,
    remote_ssrc: u32,
    highest: u32,
    cumulative_lost: u32,
    feedback: Feedback,
) -> Vec<u8> {
    let mut b = vec![0u8; 56];
    b[0] = 0x81;
    b[1] = 201;
    b[2..4].copy_from_slice(&7u16.to_be_bytes());
    b[4..8].copy_from_slice(&local_ssrc.to_be_bytes());
    b[8..12].copy_from_slice(&remote_ssrc.to_be_bytes());
    b[12] = (feedback.loss_fraction.clamp(0.0, 1.0) * 256.0).min(255.0) as u8;
    b[13..16].copy_from_slice(&cumulative_lost.min(0x7fffff).to_be_bytes()[1..]);
    b[16..20].copy_from_slice(&highest.to_be_bytes());
    b[32] = 0x80;
    b[33] = 204;
    b[34..36].copy_from_slice(&5u16.to_be_bytes());
    b[36..40].copy_from_slice(&local_ssrc.to_be_bytes());
    b[40..44].copy_from_slice(b"NMX1");
    b[44..48].copy_from_slice(&feedback.queue_frames.to_be_bytes());
    b[48..52]
        .copy_from_slice(&(feedback.late_packets.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    b[52..56].copy_from_slice(&u32::from(feedback.receiving).to_be_bytes());
    b
}
pub fn parse_feedback(bytes: &[u8], expected_ssrc: u32) -> Option<Feedback> {
    if bytes.len() != 56
        || bytes[0] != 0x81
        || bytes[1] != 201
        || bytes[2..4] != 7u16.to_be_bytes()
        || bytes[8..12] != expected_ssrc.to_be_bytes()
        || bytes[32] != 0x80
        || bytes[33] != 204
        || bytes[34..36] != 5u16.to_be_bytes()
        || &bytes[40..44] != b"NMX1"
    {
        return None;
    }
    Some(Feedback {
        loss_fraction: f64::from(bytes[12]) / 256.0,
        queue_frames: u32::from_be_bytes(bytes[44..48].try_into().ok()?),
        late_packets: u64::from(u32::from_be_bytes(bytes[48..52].try_into().ok()?)),
        receiving: u32::from_be_bytes(bytes[52..56].try_into().ok()?) == 1,
    })
}
