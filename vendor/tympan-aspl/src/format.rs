//! Audio stream formats.
//!
//! [`StreamFormat`] is a cross-platform mirror of the Core Audio
//! `AudioStreamBasicDescription` (ASBD) — the struct that describes
//! the sample rate, sample type, channel count, and byte layout of
//! one stream. An AudioServerPlugin negotiates one of these with the
//! HAL for each stream's virtual and physical format.
//!
//! The layout is plain Rust so the type is constructible and
//! inspectable from any host; the conversion to and from the FFI
//! `AudioStreamBasicDescription` will live under
//! `cfg(target_os = "macos")` in the `raw` module once that lands.
//!
//! ## Canonical format
//!
//! The HAL's preferred interchange format — and the one the
//! framework's IO callback assumes — is 32-bit IEEE float, packed,
//! native-endian, interleaved linear PCM. [`StreamFormat::float32`]
//! builds exactly that.

/// `kAudioFormatLinearPCM` (`'lpcm'`) — uncompressed linear PCM.
/// The only `format_id` an AudioServerPlugin stream uses in
/// practice.
pub const FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");

/// The on-the-wire size of a C `AudioStreamBasicDescription`: one
/// `f64` plus seven `u32`s plus the trailing reserved `u32` —
/// `8 + 8 * 4 = 40` bytes. The property dispatcher reports this as
/// the value size for the stream-format properties.
pub const ASBD_SIZE: usize = 40;

/// Linear-PCM format flags, mirroring `kAudioFormatFlag*` from
/// `<CoreAudio/CoreAudioBaseTypes.h>`.
pub mod flags {
    /// `kAudioFormatFlagIsFloat` — samples are IEEE floating point
    /// rather than integer.
    pub const IS_FLOAT: u32 = 1 << 0;
    /// `kAudioFormatFlagIsBigEndian` — samples are big-endian.
    /// Absent means native/little-endian on Apple silicon and
    /// Intel.
    pub const IS_BIG_ENDIAN: u32 = 1 << 1;
    /// `kAudioFormatFlagIsSignedInteger` — integer samples are
    /// signed (ignored when [`IS_FLOAT`] is set).
    pub const IS_SIGNED_INTEGER: u32 = 1 << 2;
    /// `kAudioFormatFlagIsPacked` — every bit of the sample word is
    /// significant; there is no padding.
    pub const IS_PACKED: u32 = 1 << 3;
    /// `kAudioFormatFlagIsAlignedHigh` — sample bits are aligned to
    /// the high end of the sample word.
    pub const IS_ALIGNED_HIGH: u32 = 1 << 4;
    /// `kAudioFormatFlagIsNonInterleaved` — each channel occupies a
    /// separate buffer rather than being interleaved.
    pub const IS_NON_INTERLEAVED: u32 = 1 << 5;
    /// `kAudioFormatFlagIsNonMixable` — the stream cannot be mixed
    /// with others by the HAL.
    pub const IS_NON_MIXABLE: u32 = 1 << 6;
}

/// The sample type and width of a linear-PCM stream.
///
/// Covers the sample formats an AudioServerPlugin realistically
/// negotiates. Anything outside this set is still representable as a
/// raw [`StreamFormat`] via [`StreamFormat::from_raw`], but the
/// typed constructors and [`StreamFormat::sample_format`] only know
/// these.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum SampleFormat {
    /// 16-bit signed integer samples.
    Int16,
    /// 24-bit signed integer samples packed into 3-byte words.
    Int24,
    /// 32-bit signed integer samples.
    Int32,
    /// 32-bit IEEE float samples — the canonical interchange
    /// format.
    Float32,
}

impl SampleFormat {
    /// Bits per individual sample (one channel of one frame).
    #[inline]
    #[must_use]
    pub const fn bits_per_channel(self) -> u32 {
        match self {
            Self::Int16 => 16,
            Self::Int24 => 24,
            Self::Int32 | Self::Float32 => 32,
        }
    }

    /// Bytes occupied by one sample in a packed stream.
    #[inline]
    #[must_use]
    pub const fn bytes_per_channel(self) -> u32 {
        match self {
            Self::Int16 => 2,
            Self::Int24 => 3,
            Self::Int32 | Self::Float32 => 4,
        }
    }

    /// The `kAudioFormatFlag*` bits this sample format implies for a
    /// packed, interleaved, native-endian stream.
    #[inline]
    #[must_use]
    pub const fn format_flags(self) -> u32 {
        match self {
            Self::Float32 => flags::IS_FLOAT | flags::IS_PACKED,
            Self::Int16 | Self::Int24 | Self::Int32 => flags::IS_SIGNED_INTEGER | flags::IS_PACKED,
        }
    }
}

/// A cross-platform mirror of `AudioStreamBasicDescription`.
///
/// Constructed with the typed constructors ([`Self::float32`],
/// [`Self::int16`], …) for the standard interleaved linear-PCM
/// streams, or with [`Self::from_raw`] for round-tripping an ASBD
/// the framework received over the FFI boundary. The derived
/// per-packet / per-frame byte counts are computed at construction.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct StreamFormat {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
}

impl StreamFormat {
    /// Construct a format directly from the nine ASBD fields
    /// (`mReserved` excluded — it is always zero).
    ///
    /// Prefer the typed constructors for standard streams; this raw
    /// form exists for round-tripping an `AudioStreamBasicDescription`
    /// received from the HAL.
    // Eight parameters, one per ASBD field the framework models; a
    // builder would only obscure the 1:1 mapping to the C struct.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[must_use]
    pub const fn from_raw(
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
    ) -> Self {
        Self {
            sample_rate,
            format_id,
            format_flags,
            bytes_per_packet,
            frames_per_packet,
            bytes_per_frame,
            channels_per_frame,
            bits_per_channel,
        }
    }

    /// Build a packed, interleaved, native-endian linear-PCM format
    /// from a [`SampleFormat`].
    #[must_use]
    pub const fn linear_pcm(sample: SampleFormat, sample_rate: f64, channels: u32) -> Self {
        let bytes_per_frame = sample.bytes_per_channel() * channels;
        Self {
            sample_rate,
            format_id: FORMAT_LINEAR_PCM,
            format_flags: sample.format_flags(),
            // Linear PCM is one frame per packet.
            bytes_per_packet: bytes_per_frame,
            frames_per_packet: 1,
            bytes_per_frame,
            channels_per_frame: channels,
            bits_per_channel: sample.bits_per_channel(),
        }
    }

    /// 32-bit IEEE float interleaved linear PCM — the canonical
    /// AudioServerPlugin interchange format.
    #[must_use]
    pub const fn float32(sample_rate: f64, channels: u32) -> Self {
        Self::linear_pcm(SampleFormat::Float32, sample_rate, channels)
    }

    /// 16-bit signed integer interleaved linear PCM.
    #[must_use]
    pub const fn int16(sample_rate: f64, channels: u32) -> Self {
        Self::linear_pcm(SampleFormat::Int16, sample_rate, channels)
    }

    /// 24-bit signed integer interleaved linear PCM.
    #[must_use]
    pub const fn int24(sample_rate: f64, channels: u32) -> Self {
        Self::linear_pcm(SampleFormat::Int24, sample_rate, channels)
    }

    /// 32-bit signed integer interleaved linear PCM.
    #[must_use]
    pub const fn int32(sample_rate: f64, channels: u32) -> Self {
        Self::linear_pcm(SampleFormat::Int32, sample_rate, channels)
    }

    /// `mSampleRate` — frames per second.
    #[inline]
    #[must_use]
    pub const fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// `mFormatID` — the codec / encoding identifier. Always
    /// [`FORMAT_LINEAR_PCM`] for the typed constructors.
    #[inline]
    #[must_use]
    pub const fn format_id(&self) -> u32 {
        self.format_id
    }

    /// `mFormatFlags` — the `kAudioFormatFlag*` bit set.
    #[inline]
    #[must_use]
    pub const fn format_flags(&self) -> u32 {
        self.format_flags
    }

    /// `mBytesPerPacket`.
    #[inline]
    #[must_use]
    pub const fn bytes_per_packet(&self) -> u32 {
        self.bytes_per_packet
    }

    /// `mFramesPerPacket`.
    #[inline]
    #[must_use]
    pub const fn frames_per_packet(&self) -> u32 {
        self.frames_per_packet
    }

    /// `mBytesPerFrame` — bytes for one frame across all channels.
    #[inline]
    #[must_use]
    pub const fn bytes_per_frame(&self) -> u32 {
        self.bytes_per_frame
    }

    /// `mChannelsPerFrame` — the channel count.
    #[inline]
    #[must_use]
    pub const fn channels(&self) -> u32 {
        self.channels_per_frame
    }

    /// `mBitsPerChannel` — significant bits per sample.
    #[inline]
    #[must_use]
    pub const fn bits_per_channel(&self) -> u32 {
        self.bits_per_channel
    }

    /// `true` iff this is a linear-PCM stream
    /// ([`FORMAT_LINEAR_PCM`]).
    #[inline]
    #[must_use]
    pub const fn is_linear_pcm(&self) -> bool {
        self.format_id == FORMAT_LINEAR_PCM
    }

    /// `true` iff the samples are IEEE floating point.
    #[inline]
    #[must_use]
    pub const fn is_float(&self) -> bool {
        self.format_flags & flags::IS_FLOAT != 0
    }

    /// `true` iff every bit of the sample word is significant.
    #[inline]
    #[must_use]
    pub const fn is_packed(&self) -> bool {
        self.format_flags & flags::IS_PACKED != 0
    }

    /// `true` iff each channel occupies its own buffer.
    #[inline]
    #[must_use]
    pub const fn is_non_interleaved(&self) -> bool {
        self.format_flags & flags::IS_NON_INTERLEAVED != 0
    }

    /// Identify the [`SampleFormat`] of this stream, or `None` if it
    /// is not one of the four standard packed interleaved layouts.
    #[must_use]
    pub fn sample_format(&self) -> Option<SampleFormat> {
        if !self.is_linear_pcm() || !self.is_packed() {
            return None;
        }
        [
            SampleFormat::Int16,
            SampleFormat::Int24,
            SampleFormat::Int32,
            SampleFormat::Float32,
        ]
        .into_iter()
        .find(|candidate| {
            candidate.bits_per_channel() == self.bits_per_channel
                && candidate.format_flags() == self.format_flags
        })
    }

    /// `true` iff this is the canonical interchange format: packed,
    /// interleaved 32-bit float linear PCM.
    #[inline]
    #[must_use]
    pub fn is_canonical(&self) -> bool {
        self.sample_format() == Some(SampleFormat::Float32) && !self.is_non_interleaved()
    }
}

/// The outcome of a stream-format negotiation.
///
/// Mirrors the three ways an AudioServerPlugin can answer the HAL's
/// "can you do this format?" query
/// (`AudioServerPlugInDriverInterface::GetPropertyData` over
/// `kAudioStreamPropertyAvailableVirtualFormats` and the format
/// setters):
///
/// - [`Self::Accept`] — the proposed format is supported as-is.
/// - [`Self::Suggest`] — the proposed format is not supported, but
///   the named alternative is the closest one that is.
/// - [`Self::Reject`] — the format is not supported and there is no
///   sensible alternative to offer.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum FormatNegotiation {
    /// The proposed format is supported; adopt it unchanged.
    Accept,
    /// The proposed format is not supported; this alternative is.
    Suggest(StreamFormat),
    /// The proposed format is not supported and no alternative is
    /// offered.
    Reject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float32_48k_stereo_has_expected_fields() {
        let f = StreamFormat::float32(48_000.0, 2);
        assert_eq!(f.sample_rate(), 48_000.0);
        assert_eq!(f.format_id(), FORMAT_LINEAR_PCM);
        assert!(f.is_linear_pcm());
        assert!(f.is_float());
        assert!(f.is_packed());
        assert!(!f.is_non_interleaved());
        assert_eq!(f.channels(), 2);
        assert_eq!(f.bits_per_channel(), 32);
        assert_eq!(f.bytes_per_frame(), 8);
        assert_eq!(f.bytes_per_packet(), 8);
        assert_eq!(f.frames_per_packet(), 1);
    }

    #[test]
    fn int16_44k1_mono_has_expected_fields() {
        let f = StreamFormat::int16(44_100.0, 1);
        assert!(!f.is_float());
        assert_eq!(f.bits_per_channel(), 16);
        assert_eq!(f.bytes_per_frame(), 2);
    }

    #[test]
    fn int24_packs_into_three_bytes() {
        let f = StreamFormat::int24(48_000.0, 2);
        assert_eq!(f.bits_per_channel(), 24);
        assert_eq!(f.bytes_per_frame(), 6);
    }

    #[test]
    fn int32_uses_four_byte_words() {
        let f = StreamFormat::int32(96_000.0, 4);
        assert_eq!(f.bits_per_channel(), 32);
        assert_eq!(f.bytes_per_frame(), 16);
    }

    #[test]
    fn sample_format_round_trips_through_constructors() {
        let cases = [
            (SampleFormat::Int16, StreamFormat::int16(48_000.0, 2)),
            (SampleFormat::Int24, StreamFormat::int24(48_000.0, 2)),
            (SampleFormat::Int32, StreamFormat::int32(48_000.0, 2)),
            (SampleFormat::Float32, StreamFormat::float32(48_000.0, 2)),
        ];
        for (sample, format) in cases {
            assert_eq!(format.sample_format(), Some(sample));
        }
    }

    #[test]
    fn only_float32_interleaved_is_canonical() {
        assert!(StreamFormat::float32(48_000.0, 2).is_canonical());
        assert!(!StreamFormat::int16(48_000.0, 2).is_canonical());
        assert!(!StreamFormat::int32(48_000.0, 2).is_canonical());
    }

    #[test]
    fn non_interleaved_float32_is_not_canonical() {
        let mut raw = StreamFormat::float32(48_000.0, 2);
        raw = StreamFormat::from_raw(
            raw.sample_rate(),
            raw.format_id(),
            raw.format_flags() | flags::IS_NON_INTERLEAVED,
            raw.bytes_per_packet(),
            raw.frames_per_packet(),
            raw.bytes_per_frame(),
            raw.channels(),
            raw.bits_per_channel(),
        );
        assert!(raw.is_non_interleaved());
        assert!(!raw.is_canonical());
    }

    #[test]
    fn unknown_format_id_has_no_sample_format() {
        let raw =
            StreamFormat::from_raw(48_000.0, u32::from_be_bytes(*b"aac "), 0, 0, 1024, 0, 2, 0);
        assert!(!raw.is_linear_pcm());
        assert_eq!(raw.sample_format(), None);
        assert!(!raw.is_canonical());
    }

    #[test]
    fn format_linear_pcm_constant_is_lpcm() {
        assert_eq!(FORMAT_LINEAR_PCM, 0x6C70_636D);
    }

    #[test]
    fn sample_format_flag_sets_are_distinct() {
        assert_ne!(
            SampleFormat::Float32.format_flags(),
            SampleFormat::Int32.format_flags()
        );
        assert!(SampleFormat::Float32.format_flags() & flags::IS_FLOAT != 0);
        assert!(SampleFormat::Int16.format_flags() & flags::IS_SIGNED_INTEGER != 0);
    }

    #[test]
    fn negotiation_variants_distinguish() {
        let accept = FormatNegotiation::Accept;
        let suggest = FormatNegotiation::Suggest(StreamFormat::float32(48_000.0, 2));
        let reject = FormatNegotiation::Reject;
        assert_eq!(accept, FormatNegotiation::Accept);
        assert_ne!(accept, reject);
        assert_ne!(suggest, accept);
        assert_ne!(suggest, reject);
    }
}
