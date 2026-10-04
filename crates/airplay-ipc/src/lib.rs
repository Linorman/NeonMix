//! Bounded worker IPC. Media is explicit little-endian, never a native struct.
//! Presentation timestamps are Unix wall-clock nanoseconds, not `Instant`.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, BufRead, Read, Write};

pub mod local;

pub const IPC_VERSION: u16 = 1;
/// Control JSON evolves independently of the unchanged PCM v1 layout.
pub const CONTROL_VERSION: u16 = 2;
pub const HEADER_BYTES: usize = 112;
pub const MAX_FRAMES: usize = 480;
pub const CHANNELS: usize = 2;
pub const PCM_RATE: u32 = 48_000;
pub const MAX_PAYLOAD_BYTES: usize = MAX_FRAMES * CHANNELS * 4;
pub const MAX_PACKET_BYTES: usize = HEADER_BYTES + MAX_PAYLOAD_BYTES;
pub const MAX_CONTROL_BYTES: usize = 16 * 1024;
pub const FLAG_DISCONTINUITY: u32 = 1;
pub const MAGIC: [u8; 4] = *b"NMAM";

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("invalid media header: {0}")]
    Header(&'static str),
    #[error("media payload length does not match frame count")]
    PayloadLength,
    #[error("PCM contains a non-finite sample")]
    NonFiniteSample,
    #[error("packet belongs to another or revoked session/stream/epoch/mapping")]
    StaleContext,
    #[error("sequence or source position moved backwards")]
    OutOfOrder,
    #[error("control line exceeds 16 KiB or lacks its LF terminator")]
    ControlLength,
    #[error("control line must contain exactly one JSON value")]
    ControlFraming,
    #[error("invalid control field: {0}")]
    ControlField(&'static str),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PcmHeader {
    pub flags: u32,
    pub session_id: u64,
    pub stream_id: u64,
    pub stream_epoch: u64,
    pub format_epoch: u64,
    pub sequence: u64,
    /// Position in the original decoder's `source_rate` timeline.
    pub source_sample_position: u64,
    /// First normalized frame's target presentation time in Unix ns.
    pub presentation_time_ns: u64,
    pub mapping_id: u64,
    pub uncertainty_ns: u64,
    pub source_rate: u32,
    pub frame_count: u16,
    /// Linear protocol volume; apply only if `gain_applied == false`.
    pub protocol_gain: f32,
    pub gain_applied: bool,
}

impl PcmHeader {
    pub fn validate(&self) -> Result<(), IpcError> {
        if self.flags & !FLAG_DISCONTINUITY != 0 {
            return Err(IpcError::Header("unknown flags"));
        }
        if [
            self.session_id,
            self.stream_id,
            self.stream_epoch,
            self.format_epoch,
            self.mapping_id,
        ]
        .contains(&0)
        {
            return Err(IpcError::Header("zero context identifier"));
        }
        if !(8_000..=384_000).contains(&self.source_rate) {
            return Err(IpcError::Header("source rate"));
        }
        if self.frame_count == 0 || usize::from(self.frame_count) > MAX_FRAMES {
            return Err(IpcError::Header("frame count"));
        }
        if !self.protocol_gain.is_finite() || !(0.0..=1.0).contains(&self.protocol_gain) {
            return Err(IpcError::Header("protocol gain"));
        }
        if self.presentation_time_ns == 0 {
            return Err(IpcError::Header("missing presentation anchor"));
        }
        Ok(())
    }

    pub fn payload_bytes(&self) -> usize {
        usize::from(self.frame_count) * CHANNELS * 4
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IpcError> {
        if bytes.len() != HEADER_BYTES {
            return Err(IpcError::Header("header length"));
        }
        if bytes[..4] != MAGIC || u16_at(bytes, 4) != IPC_VERSION {
            return Err(IpcError::Header("magic or version"));
        }
        if usize::from(u16_at(bytes, 6)) != HEADER_BYTES {
            return Err(IpcError::Header("declared header length"));
        }
        if bytes[94] != CHANNELS as u8 || bytes[95] != 1 || u32_at(bytes, 104) != PCM_RATE {
            return Err(IpcError::Header("normalized PCM format"));
        }
        if bytes[100] > 1 || bytes[101] != 1 || u16_at(bytes, 102) != 0 || u32_at(bytes, 108) != 0 {
            return Err(IpcError::Header("gain, clock domain or reserved fields"));
        }
        let header = Self {
            flags: u32_at(bytes, 12),
            session_id: u64_at(bytes, 16),
            stream_id: u64_at(bytes, 24),
            stream_epoch: u64_at(bytes, 32),
            format_epoch: u64_at(bytes, 40),
            sequence: u64_at(bytes, 48),
            source_sample_position: u64_at(bytes, 56),
            presentation_time_ns: u64_at(bytes, 64),
            mapping_id: u64_at(bytes, 72),
            uncertainty_ns: u64_at(bytes, 80),
            source_rate: u32_at(bytes, 88),
            frame_count: u16_at(bytes, 92),
            protocol_gain: f32::from_bits(u32_at(bytes, 96)),
            gain_applied: bytes[100] == 1,
        };
        header.validate()?;
        if u32_at(bytes, 8) as usize != header.payload_bytes() {
            return Err(IpcError::PayloadLength);
        }
        Ok(header)
    }

    pub fn encode(&self) -> Result<[u8; HEADER_BYTES], IpcError> {
        self.validate()?;
        let mut bytes = [0; HEADER_BYTES];
        bytes[..4].copy_from_slice(&MAGIC);
        put(&mut bytes, 4, IPC_VERSION.to_le_bytes());
        put(&mut bytes, 6, (HEADER_BYTES as u16).to_le_bytes());
        put(&mut bytes, 8, (self.payload_bytes() as u32).to_le_bytes());
        put(&mut bytes, 12, self.flags.to_le_bytes());
        for (offset, value) in [
            (16, self.session_id),
            (24, self.stream_id),
            (32, self.stream_epoch),
            (40, self.format_epoch),
            (48, self.sequence),
            (56, self.source_sample_position),
            (64, self.presentation_time_ns),
            (72, self.mapping_id),
            (80, self.uncertainty_ns),
        ] {
            put(&mut bytes, offset, value.to_le_bytes());
        }
        put(&mut bytes, 88, self.source_rate.to_le_bytes());
        put(&mut bytes, 92, self.frame_count.to_le_bytes());
        bytes[94] = CHANNELS as u8;
        bytes[95] = 1;
        put(&mut bytes, 96, self.protocol_gain.to_le_bytes());
        bytes[100] = u8::from(self.gain_applied);
        bytes[101] = 1;
        put(&mut bytes, 104, PCM_RATE.to_le_bytes());
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PcmPacket {
    pub header: PcmHeader,
    /// Interleaved L/R normalized float32. This allocation is off the audio callback.
    pub samples: Vec<f32>,
}

impl PcmPacket {
    pub fn validate(&self) -> Result<(), IpcError> {
        self.header.validate()?;
        if self.samples.len() != usize::from(self.header.frame_count) * CHANNELS {
            return Err(IpcError::PayloadLength);
        }
        if self.samples.iter().any(|sample| !sample.is_finite()) {
            return Err(IpcError::NonFiniteSample);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, IpcError> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(HEADER_BYTES + self.header.payload_bytes());
        bytes.extend_from_slice(&self.header.encode()?);
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IpcError> {
        if bytes.len() < HEADER_BYTES || bytes.len() > MAX_PACKET_BYTES {
            return Err(IpcError::Header("packet length"));
        }
        let header = PcmHeader::decode(&bytes[..HEADER_BYTES])?;
        if bytes.len() != HEADER_BYTES + header.payload_bytes() {
            return Err(IpcError::PayloadLength);
        }
        let samples = bytes[HEADER_BYTES..]
            .chunks_exact(4)
            .map(|sample| f32::from_le_bytes(sample.try_into().expect("four-byte chunk")))
            .collect();
        let packet = Self { header, samples };
        packet.validate()?;
        Ok(packet)
    }
}

/// Reads one frame. Invalid header is rejected before payload allocation. Caller
/// sets transport timeouts and closes the connection on any decoding error.
pub fn read_packet(reader: &mut impl Read) -> Result<Option<PcmPacket>, IpcError> {
    let mut bytes = [0; MAX_PACKET_BYTES];
    loop {
        match reader.read(&mut bytes[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    reader.read_exact(&mut bytes[1..HEADER_BYTES])?;
    let header = PcmHeader::decode(&bytes[..HEADER_BYTES])?;
    let end = HEADER_BYTES + header.payload_bytes();
    reader.read_exact(&mut bytes[HEADER_BYTES..end])?;
    PcmPacket::decode(&bytes[..end]).map(Some)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaGrant {
    pub session_id: u64,
    pub stream_id: u64,
    pub stream_epoch: u64,
    pub format_epoch: u64,
    pub mapping_id: u64,
}

/// Hub commands travel on the worker's private stdin, separate from PCM.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HubCommand {
    PairingAdmit {
        worker_generation: u64,
        trust_generation: u64,
        connection_id: u64,
        pairing_request_id: u64,
        allowed: bool,
        attempts: u32,
    },
    PairingWindow {
        worker_generation: u64,
        trust_generation: u64,
        pin: String,
        remaining_ms: u64,
        attempts: u32,
    },
    Admit {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        allowed: bool,
    },
    Grant {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_id: u64,
        stream_epoch: u64,
        format_epoch: u64,
        mapping_id: u64,
    },
    Allow {
        worker_generation: u64,
    },
    Disconnect {
        worker_generation: u64,
        connection_id: u64,
        session_id: u64,
        stream_epoch: u64,
    },
    Revoke {
        worker_generation: u64,
        connection_id: u64,
        session_id: u64,
        stream_epoch: u64,
    },
    TrustUpdate {
        worker_generation: u64,
        trust_generation: u64,
        known_client_keys: Vec<String>,
        blocked_client_keys: Vec<String>,
        pairing_allowed: bool,
    },
    Stop {},
}

impl MediaGrant {
    pub fn command(
        self,
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
    ) -> HubCommand {
        HubCommand::Grant {
            worker_generation,
            connection_id,
            request_id,
            session_id: self.session_id,
            stream_id: self.stream_id,
            stream_epoch: self.stream_epoch,
            format_epoch: self.format_epoch,
            mapping_id: self.mapping_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaResetReason {
    #[default]
    Protocol,
    StreamSetup,
    TimestampJump,
}

/// Events are observations, never authority to grant identity or advance epochs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerEvent {
    ProtocolTransport(ProtocolTransport),
    Protocol {
        method: String,
        route: String,
        status: u16,
    },
    ProtocolDetail {
        stage: String,
        a_bytes: u32,
        proof_bytes: u32,
        reason: String,
    },
    Ready {
        control_version: u16,
        worker_generation: u64,
        port: u16,
        public_key: String,
        features: u64,
    },
    PairingRequest {
        worker_generation: u64,
        trust_generation: u64,
        connection_id: u64,
        pairing_request_id: u64,
    },
    PairingEnded {
        worker_generation: u64,
        trust_generation: u64,
        connection_id: u64,
        pairing_request_id: u64,
    },
    PairingWindowApplied {
        worker_generation: u64,
        trust_generation: u64,
    },
    PairingAttempt {
        worker_generation: u64,
        attempts: u32,
    },
    PairingPin {
        worker_generation: u64,
        pin: String,
    },
    AdmitRequest {
        trust_generation: u64,
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        client_public_key: String,
        device_id: String,
        name: String,
    },
    SessionStarted {
        trust_generation: u64,
        session_id: u64,
        stream_epoch: u64,
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        client_public_key: String,
        device_id: String,
        name: String,
    },
    Registered {
        pairing_request_id: u64,
        trust_generation: u64,
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        client_public_key: String,
        device_id: String,
        name: String,
    },
    SessionEnded {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_epoch: u64,
    },
    GrantApplied {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_epoch: u64,
        stream_id: u64,
        format_epoch: u64,
        mapping_id: u64,
    },
    Format {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_epoch: u64,
        codec: String,
        source_rate: u32,
        source_frame_count: u16,
    },
    Volume {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_epoch: u64,
        volume_db: f32,
    },
    Flush {
        worker_generation: u64,
        connection_id: u64,
        request_id: u64,
        session_id: u64,
        stream_epoch: u64,
        #[serde(default)]
        reason: MediaResetReason,
    },
    Fatal {
        message: String,
    },
}

/// Safe observations from the actual RTSP socket write, never raw wire data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolTransport {
    pub connection_id: u64,
    pub event: String,
    pub relative_ms: u64,
    pub protocol: String,
    pub method: String,
    pub route: String,
    pub body_bytes: u32,
    pub cseq_present: bool,
    pub session_present: bool,
    pub status: u16,
    pub response_bytes: u32,
    pub sent_bytes: u32,
    pub outcome: String,
}
impl ProtocolTransport {
    pub fn validate(&self) -> Result<(), IpcError> {
        let bad = || IpcError::ControlField("protocol_transport");
        if self.connection_id == 0
            || self.connection_id >= (1u64 << 53)
            || self.relative_ms >= (1u64 << 53)
            || !["RTSP/1.0", "HTTP/1.1", "other", "none"].contains(&self.protocol.as_str())
            || ![
                "GET",
                "POST",
                "PUT",
                "OPTIONS",
                "SETUP",
                "RECORD",
                "GET_PARAMETER",
                "SET_PARAMETER",
                "FLUSH",
                "TEARDOWN",
                "ANNOUNCE",
                "OTHER",
            ]
            .contains(&self.method.as_str())
            || ![
                "info",
                "pair_pin_start",
                "pair_setup_pin",
                "pair_setup",
                "pair_verify",
                "fp_setup",
                "feedback",
                "audio_mode",
                "rate_anchor",
                "set_rate",
                "set_property",
                "options",
                "audio",
                "video",
                "other",
            ]
            .contains(&self.route.as_str())
            || self.body_bytes > 1_048_576
            || self.response_bytes > 1_064_960
            || self.sent_bytes > self.response_bytes
        {
            return Err(bad());
        }
        let valid = match self.event.as_str() {
            "open" => self.outcome == "opened",
            "close" => [
                "peer_closed",
                "recv_error",
                "parse_error",
                "requested",
                "software",
                "shutdown",
                "select_error",
                "send_error",
                "send_closed",
                "send_timeout",
            ]
            .contains(&self.outcome.as_str()),
            "response" => {
                [
                    "complete",
                    "send_error",
                    "send_closed",
                    "send_timeout",
                    "no_response",
                ]
                .contains(&self.outcome.as_str())
                    && ((100..=599).contains(&self.status)
                        || (self.outcome == "no_response" && self.status == 0))
                    && (self.outcome != "complete" || self.sent_bytes == self.response_bytes)
            }
            _ => false,
        };
        if !valid
            || (self.event != "response"
                && (self.status != 0 || self.response_bytes != 0 || self.sent_bytes != 0))
        {
            return Err(bad());
        }
        Ok(())
    }
}

impl MediaGrant {
    pub fn matches(&self, header: &PcmHeader) -> bool {
        self.session_id == header.session_id
            && self.stream_id == header.stream_id
            && self.stream_epoch == header.stream_epoch
            && self.format_epoch == header.format_epoch
            && self.mapping_id == header.mapping_id
    }
}

/// Context is installed only by Hub authorization; media cannot advance epochs.
#[derive(Default, Debug)]
pub struct MediaGuard {
    grant: Option<MediaGrant>,
    last: Option<(u64, u64, u32)>,
}

impl MediaGuard {
    pub fn grant(&mut self, grant: MediaGrant) {
        self.grant = Some(grant);
        self.last = None;
    }

    pub fn revoke(&mut self) {
        self.grant = None;
        self.last = None;
    }

    pub fn accept(&mut self, packet: &PcmPacket) -> Result<(), IpcError> {
        packet.validate()?;
        let header = &packet.header;
        if !self.grant.is_some_and(|grant| grant.matches(header)) {
            return Err(IpcError::StaleContext);
        }
        if self
            .last
            .is_some_and(|(_, _, rate)| header.source_rate != rate)
        {
            return Err(IpcError::Header(
                "source rate changed without a new context",
            ));
        }
        if self.last.is_some_and(|(sequence, position, _)| {
            header.sequence <= sequence || header.source_sample_position < position
        }) {
            return Err(IpcError::OutOfOrder);
        }
        self.last = Some((
            header.sequence,
            header.source_sample_position,
            header.source_rate,
        ));
        Ok(())
    }
}

/// All command/event reads are bounded, including an unterminated malicious line.
/// EOF in a partial line is an error; clean EOF between lines returns None.
pub fn read_control<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>, IpcError> {
    let mut bytes = Vec::with_capacity(512);
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(IpcError::ControlLength)
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len() + take > MAX_CONTROL_BYTES {
            return Err(IpcError::ControlLength);
        }
        let finished = available[take - 1] == b'\n';
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if finished {
            if bytes[..bytes.len() - 1].contains(&b'\r') {
                return Err(IpcError::ControlFraming);
            }
            return Ok(Some(serde_json::from_slice(&bytes[..bytes.len() - 1])?));
        }
    }
}

pub fn write_control<T: Serialize>(writer: &mut impl Write, message: &T) -> Result<(), IpcError> {
    // The writer bounds serialization itself, rather than allocating an arbitrary
    // JSON string and checking its size afterwards.
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len() + bytes.len() >= MAX_CONTROL_BYTES {
                return Err(io::Error::other("control length"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut bounded = Bounded(Vec::with_capacity(512));
    if let Err(error) = serde_json::to_writer(&mut bounded, message) {
        return if error.is_io() {
            Err(IpcError::ControlLength)
        } else {
            Err(error.into())
        };
    }
    bounded.0.push(b'\n');
    writer.write_all(&bounded.0)?;
    writer.flush()?;
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated header"),
    )
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated header"),
    )
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated header"),
    )
}
fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
    bytes[offset..offset + N].copy_from_slice(&value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn control_v2_requires_event_provenance() {
        assert!(
            serde_json::from_value::<WorkerEvent>(serde_json::json!({"type":"flush"})).is_err()
        );
        let event = serde_json::json!({"type":"flush","reason":"timestamp_jump", "worker_generation":2,
            "connection_id":3,"request_id":4,"session_id":5,"stream_epoch":6});
        assert!(serde_json::from_value::<WorkerEvent>(event.clone()).is_ok());
        let mut invalid = event;
        invalid["reason"] = serde_json::json!("arbitrary");
        assert!(serde_json::from_value::<WorkerEvent>(invalid).is_err());
    }

    #[test]
    fn transport_observations_reject_secrets_impossible_lengths_and_unknown_outcomes() {
        let value = serde_json::json!({"type":"protocol_transport","connection_id":1,"event":"response",
            "relative_ms":3,"protocol":"RTSP/1.0","method":"POST","route":"pair_pin_start",
            "body_bytes":0,"cseq_present":true,"session_present":false,"status":200,
            "response_bytes":70,"sent_bytes":70,"outcome":"complete"});
        let event: WorkerEvent = serde_json::from_value(value.clone()).unwrap();
        let WorkerEvent::ProtocolTransport(mut transport) = event else {
            panic!("wrong event");
        };
        assert!(transport.validate().is_ok());
        transport.sent_bytes = 69;
        assert!(transport.validate().is_err());
        transport.sent_bytes = 71;
        assert!(transport.validate().is_err());
        transport.sent_bytes = 70;
        transport.outcome = "raw private exception".into();
        assert!(transport.validate().is_err());
        transport.outcome = "complete".into();
        transport.route = "1234".into();
        assert!(transport.validate().is_err());
        let mut secret = value;
        secret["pairing_pin"] = serde_json::json!("1234");
        assert!(serde_json::from_value::<WorkerEvent>(secret).is_err());
    }

    fn packet() -> PcmPacket {
        PcmPacket {
            header: PcmHeader {
                flags: 0,
                session_id: 11,
                stream_id: 12,
                stream_epoch: 13,
                format_epoch: 14,
                sequence: 0,
                source_sample_position: 441,
                presentation_time_ns: 1_795_000_000_123_456_789,
                mapping_id: 15,
                uncertainty_ns: 1_000_000,
                source_rate: 44_100,
                frame_count: 480,
                protocol_gain: 0.5,
                gain_applied: false,
            },
            samples: (0..960).map(|value| value as f32 / 960.0).collect(),
        }
    }

    fn grant() -> MediaGrant {
        MediaGrant {
            session_id: 11,
            stream_id: 12,
            stream_epoch: 13,
            format_epoch: 14,
            mapping_id: 15,
        }
    }

    #[test]
    fn explicit_layout_preserves_original_rate_and_unix_time() {
        let original = packet();
        let bytes = original.encode().unwrap();
        assert_eq!(bytes.len(), MAX_PACKET_BYTES);
        assert_eq!(&bytes[..8], b"NMAM\x01\x00\x70\x00");
        assert_eq!(u32_at(&bytes, 8), 3_840);
        assert_eq!(u64_at(&bytes, 64), original.header.presentation_time_ns);
        assert_eq!(u32_at(&bytes, 88), 44_100);
        assert_eq!(u32_at(&bytes, 104), 48_000);
        assert_eq!(PcmPacket::decode(&bytes).unwrap(), original);
    }

    #[test]
    fn rejects_unbounded_and_inconsistent_lengths_before_payload_read() {
        let bytes = packet().encode().unwrap();
        for length in [0, 1, HEADER_BYTES - 1, MAX_PACKET_BYTES - 1] {
            assert!(PcmPacket::decode(&bytes[..length]).is_err());
        }
        let mut huge = bytes[..HEADER_BYTES].to_vec();
        put(&mut huge, 8, u32::MAX.to_le_bytes());
        assert!(matches!(
            read_packet(&mut Cursor::new(huge)),
            Err(IpcError::PayloadLength)
        ));
        let mut invalid_frames = bytes[..HEADER_BYTES].to_vec();
        put(&mut invalid_frames, 92, 481_u16.to_le_bytes());
        assert!(matches!(
            read_packet(&mut Cursor::new(invalid_frames)),
            Err(IpcError::Header("frame count"))
        ));
    }

    #[test]
    fn rejects_unknown_wire_formats_and_reserved_bits() {
        let bytes = packet().encode().unwrap();
        for offset in [0, 4, 6, 12, 94, 95, 101, 102, 104, 108] {
            let mut invalid = bytes.clone();
            invalid[offset] ^= 0x40;
            assert!(PcmPacket::decode(&invalid).is_err(), "offset {offset}");
        }
        let mut invalid = bytes;
        invalid[100] = 2;
        assert!(PcmPacket::decode(&invalid).is_err());
    }

    #[test]
    fn rejects_non_finite_samples_and_untrusted_gain() {
        for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bytes = packet().encode().unwrap();
            put(&mut bytes, HEADER_BYTES, sample.to_le_bytes());
            assert!(matches!(
                PcmPacket::decode(&bytes),
                Err(IpcError::NonFiniteSample)
            ));
            put(&mut bytes, 96, sample.to_le_bytes());
            assert!(matches!(
                PcmPacket::decode(&bytes),
                Err(IpcError::Header("protocol gain"))
            ));
        }
        for gain in [-0.001, 1.001] {
            let mut invalid = packet();
            invalid.header.protocol_gain = gain;
            assert!(invalid.encode().is_err());
        }
    }

    #[test]
    fn stream_reads_partial_frames_and_distinguishes_truncation_from_eof() {
        let bytes = packet().encode().unwrap();
        let mut cursor = Cursor::new(bytes.clone());
        assert_eq!(read_packet(&mut cursor).unwrap(), Some(packet()));
        assert_eq!(read_packet(&mut cursor).unwrap(), None);
        assert!(read_packet(&mut Cursor::new(&bytes[..bytes.len() - 1])).is_err());
        struct OneByte(Cursor<Vec<u8>>);
        impl Read for OneByte {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let length = bytes.len().min(1);
                self.0.read(&mut bytes[..length])
            }
        }
        assert_eq!(
            read_packet(&mut OneByte(Cursor::new(bytes))).unwrap(),
            Some(packet())
        );
    }

    #[test]
    fn revoke_and_hub_granted_epochs_reject_in_flight_pcm() {
        let mut guard = MediaGuard::default();
        assert!(matches!(
            guard.accept(&packet()),
            Err(IpcError::StaleContext)
        ));
        guard.grant(grant());
        guard.accept(&packet()).unwrap();
        guard.revoke();
        assert!(matches!(
            guard.accept(&packet()),
            Err(IpcError::StaleContext)
        ));
        let next = MediaGrant {
            stream_epoch: 14,
            ..grant()
        };
        guard.grant(next);
        assert!(matches!(
            guard.accept(&packet()),
            Err(IpcError::StaleContext)
        ));
        let mut updated = packet();
        updated.header.stream_epoch = 14;
        guard.accept(&updated).unwrap();
        updated.header.mapping_id += 1;
        updated.header.sequence += 1;
        assert!(matches!(
            guard.accept(&updated),
            Err(IpcError::StaleContext)
        ));
    }

    #[test]
    fn sequence_gaps_allowed_but_duplicate_reverse_or_flag_reset_rejected() {
        let mut guard = MediaGuard::default();
        guard.grant(grant());
        let mut media = packet();
        guard.accept(&media).unwrap();
        assert!(matches!(guard.accept(&media), Err(IpcError::OutOfOrder)));
        media.header.sequence = 9;
        media.header.source_sample_position += 441;
        guard.accept(&media).unwrap();
        media.header.sequence += 1;
        media.header.flags = FLAG_DISCONTINUITY;
        media.header.source_sample_position = 0;
        assert!(matches!(guard.accept(&media), Err(IpcError::OutOfOrder)));
    }

    #[test]
    fn control_rejects_overlong_unterminated_and_extra_json() {
        for bytes in [
            vec![b'a'; MAX_CONTROL_BYTES + 1],
            b"{\"x\":1}".to_vec(),
            b"{}{}\n".to_vec(),
            b"{}\r\n".to_vec(),
        ] {
            assert!(read_control::<serde_json::Value>(&mut Cursor::new(bytes)).is_err());
        }
        let mut reader = Cursor::new(b"{\"x\":1}\n{\"x\":2}\n");
        assert_eq!(
            read_control::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap()["x"],
            1
        );
        assert_eq!(
            read_control::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap()["x"],
            2
        );
        assert!(
            read_control::<serde_json::Value>(&mut reader)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn control_writer_never_writes_partial_oversized_message() {
        let mut output = Vec::new();
        assert!(matches!(
            write_control(&mut output, &"x".repeat(MAX_CONTROL_BYTES)),
            Err(IpcError::ControlLength)
        ));
        assert!(output.is_empty());
        write_control(&mut output, &grant()).unwrap();
        assert_eq!(
            read_control::<MediaGrant>(&mut Cursor::new(output)).unwrap(),
            Some(grant())
        );
    }

    #[test]
    fn sample_rate_change_requires_hub_granted_context() {
        let mut guard = MediaGuard::default();
        guard.grant(grant());
        let mut media = packet();
        guard.accept(&media).unwrap();
        media.header.sequence += 1;
        media.header.source_rate = 48_000;
        assert!(matches!(guard.accept(&media), Err(IpcError::Header(_))));
        guard.grant(MediaGrant {
            format_epoch: 15,
            ..grant()
        });
        assert!(matches!(guard.accept(&media), Err(IpcError::StaleContext)));
        media.header.format_epoch = 15;
        guard.accept(&media).unwrap();
    }

    #[test]
    fn typed_admission_requires_request_correlation_and_known_fields() {
        let mut reader = Cursor::new(b"{\"type\":\"admit\",\"allowed\":true}\n");
        assert!(read_control::<HubCommand>(&mut reader).is_err());
        let mut reader = Cursor::new(b"{\"type\":\"stop\",\"path\":\"/arbitrary\"}\n");
        assert!(read_control::<HubCommand>(&mut reader).is_err());
        let command = HubCommand::Admit {
            worker_generation: 1,
            connection_id: 2,
            request_id: 41,
            allowed: true,
        };
        let mut bytes = Vec::new();
        write_control(&mut bytes, &command).unwrap();
        assert_eq!(
            read_control::<HubCommand>(&mut Cursor::new(bytes)).unwrap(),
            Some(command)
        );
    }
}
