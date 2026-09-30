//! The IO cycle: timestamps, operations, and the realtime buffer
//! wrapper.
//!
//! Core Audio drives an AudioServerPlugin's IO on a dedicated
//! realtime thread. Each cycle the HAL calls `BeginIOOperation`,
//! one or more `DoIOOperation`s, and `EndIOOperation`, tagging each
//! with an [`IoOperation`] selector that says which phase of the
//! read / process / mix / write pipeline is running. The framework's
//! IO harness (added in a later PR) translates that sequence into a
//! single [`Driver::process_io`](crate::Driver::process_io) call
//! against typed `&[f32]` / `&mut [f32]` buffers.
//!
//! Everything in this module is cross-platform plain data so the IO
//! contract can be unit-tested without a HAL round-trip.

use crate::fourcc::FourCharCode;

/// One sample of canonical (32-bit float) linear PCM audio.
///
/// The framework's IO harness negotiates the canonical interchange
/// format (see [`StreamFormat::float32`](crate::StreamFormat::float32)),
/// so the realtime callback always sees `f32` samples regardless of
/// the device's physical format.
pub type Sample = f32;

/// A point on Core Audio's timeline.
///
/// A trimmed mirror of the C `AudioTimeStamp`: the two fields an
/// AudioServerPlugin's IO path actually consults. The HAL stamps
/// each IO cycle with one of these so the driver can align its
/// processing against the device's sample clock.
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Timestamp {
    /// `mSampleTime` — the cycle's position on the device's sample
    /// clock, in frames since the device started. Fractional values
    /// are possible after a sample-rate change.
    pub sample_time: f64,
    /// `mHostTime` — the cycle's position on the host clock, in
    /// `mach_absolute_time` units.
    pub host_time: u64,
}

impl Timestamp {
    /// Construct a timestamp from its sample-clock and host-clock
    /// positions.
    #[inline]
    #[must_use]
    pub const fn new(sample_time: f64, host_time: u64) -> Self {
        Self {
            sample_time,
            host_time,
        }
    }

    /// The zero timestamp — frame 0, host time 0. The HAL uses this
    /// shape before a device's clock has advanced.
    pub const ZERO: Self = Self {
        sample_time: 0.0,
        host_time: 0,
    };
}

/// Which phase of the HAL's IO pipeline a callback is running.
///
/// A thin newtype over a [`FourCharCode`]. The values mirror the
/// `kAudioServerPlugInIOOperation*` constants from
/// `<CoreAudio/AudioServerPlugIn.h>`; the framework's IO harness
/// matches on them to decide whether a cycle needs the driver to
/// read input, process, or write output.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct IoOperation(pub FourCharCode);

impl IoOperation {
    /// `kAudioServerPlugInIOOperationThread` (`'thrd'`) — the IO
    /// thread itself is starting or stopping.
    pub const THREAD: Self = Self(FourCharCode::new(*b"thrd"));
    /// `kAudioServerPlugInIOOperationCycle` (`'cycl'`) — a whole IO
    /// cycle boundary.
    pub const CYCLE: Self = Self(FourCharCode::new(*b"cycl"));
    /// `kAudioServerPlugInIOOperationReadInput` (`'read'`) — read
    /// the device's input data into the HAL's buffer.
    pub const READ_INPUT: Self = Self(FourCharCode::new(*b"read"));
    /// `kAudioServerPlugInIOOperationConvertInput` (`'cinp'`) —
    /// convert read input into the client's format.
    pub const CONVERT_INPUT: Self = Self(FourCharCode::new(*b"cinp"));
    /// `kAudioServerPlugInIOOperationProcessInput` (`'pinp'`) —
    /// the driver's chance to process the input stream.
    pub const PROCESS_INPUT: Self = Self(FourCharCode::new(*b"pinp"));
    /// `kAudioServerPlugInIOOperationProcessOutput` (`'pout'`) —
    /// the driver's chance to process a client's output stream.
    pub const PROCESS_OUTPUT: Self = Self(FourCharCode::new(*b"pout"));
    /// `kAudioServerPlugInIOOperationMixOutput` (`'mixo'`) — mix a
    /// client's output into the device's output buffer.
    pub const MIX_OUTPUT: Self = Self(FourCharCode::new(*b"mixo"));
    /// `kAudioServerPlugInIOOperationProcessMix` (`'pmix'`) — the
    /// driver's chance to process the fully-mixed output.
    pub const PROCESS_MIX: Self = Self(FourCharCode::new(*b"pmix"));
    /// `kAudioServerPlugInIOOperationConvertMix` (`'cmix'`) —
    /// convert the mixed output into the device's physical format.
    pub const CONVERT_MIX: Self = Self(FourCharCode::new(*b"cmix"));
    /// `kAudioServerPlugInIOOperationWriteMix` (`'rite'`) — write
    /// the converted mix out to the device.
    pub const WRITE_MIX: Self = Self(FourCharCode::new(*b"rite"));

    /// The underlying four-character code.
    #[inline]
    #[must_use]
    pub const fn code(self) -> FourCharCode {
        self.0
    }

    /// `true` iff this operation hands the driver audio to process —
    /// [`Self::PROCESS_INPUT`], [`Self::PROCESS_OUTPUT`], or
    /// [`Self::PROCESS_MIX`]. The framework's IO harness only routes
    /// these into [`Driver::process_io`](crate::Driver::process_io).
    #[inline]
    #[must_use]
    pub fn is_processing(self) -> bool {
        matches!(
            self,
            Self::PROCESS_INPUT | Self::PROCESS_OUTPUT | Self::PROCESS_MIX
        )
    }
}

impl From<FourCharCode> for IoOperation {
    #[inline]
    fn from(code: FourCharCode) -> Self {
        Self(code)
    }
}

/// A borrowed view of one IO cycle's input and output buffers.
///
/// The framework's IO harness builds one of these per processing
/// operation and hands it to
/// [`Driver::process_io`](crate::Driver::process_io). Both slices
/// are already in the canonical interleaved float32 format, and the
/// harness guarantees `output.len()` is a whole number of frames for
/// the device's channel count.
///
/// `input` is empty for an output-only device (and `output` is empty
/// for an input-only device); a loopback device sees both.
#[derive(Debug)]
pub struct IoBuffer<'a> {
    /// The cycle's timestamp on the device's sample clock.
    pub timestamp: Timestamp,
    /// Which IO operation produced this buffer.
    pub operation: IoOperation,
    /// Interleaved float32 input samples, or an empty slice for an
    /// output-only device.
    pub input: &'a [Sample],
    /// Interleaved float32 output samples to write into, or an
    /// empty slice for an input-only device.
    pub output: &'a mut [Sample],
    /// Device master gain for WriteMix; apply once before publishing loopback samples.
    /// ReadInput always receives 1.0 because its samples already include the render gain.
    pub render_gain: f32,
}

impl<'a> IoBuffer<'a> {
    /// Construct an IO buffer view from its parts.
    ///
    /// The framework's IO harness constructs these per cycle; tests
    /// construct them directly against synthetic slices.
    #[inline]
    #[must_use]
    pub fn new(
        timestamp: Timestamp,
        operation: IoOperation,
        input: &'a [Sample],
        output: &'a mut [Sample],
    ) -> Self {
        Self {
            timestamp,
            operation,
            input,
            output,
            render_gain: 1.0,
        }
    }

    /// Number of input samples available this cycle.
    #[inline]
    #[must_use]
    pub fn input_len(&self) -> usize {
        self.input.len()
    }

    /// Number of output samples to be written this cycle.
    #[inline]
    #[must_use]
    pub fn output_len(&self) -> usize {
        self.output.len()
    }

    /// Fill the output buffer with silence. Allocation-free and
    /// realtime-safe — a `memset` over the borrowed slice.
    #[inline]
    pub fn silence_output(&mut self) {
        self.output.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_zero_is_origin() {
        assert_eq!(Timestamp::ZERO.sample_time, 0.0);
        assert_eq!(Timestamp::ZERO.host_time, 0);
        assert_eq!(Timestamp::ZERO, Timestamp::new(0.0, 0));
    }

    #[test]
    fn timestamp_default_is_zero() {
        assert_eq!(Timestamp::default(), Timestamp::ZERO);
    }

    #[test]
    fn io_operation_constants_match_core_audio_codes() {
        assert_eq!(IoOperation::READ_INPUT.code(), FourCharCode::new(*b"read"));
        assert_eq!(
            IoOperation::PROCESS_OUTPUT.code(),
            FourCharCode::new(*b"pout")
        );
        assert_eq!(IoOperation::WRITE_MIX.code(), FourCharCode::new(*b"rite"));
    }

    #[test]
    fn only_process_operations_are_processing() {
        assert!(IoOperation::PROCESS_INPUT.is_processing());
        assert!(IoOperation::PROCESS_OUTPUT.is_processing());
        assert!(IoOperation::PROCESS_MIX.is_processing());

        assert!(!IoOperation::READ_INPUT.is_processing());
        assert!(!IoOperation::WRITE_MIX.is_processing());
        assert!(!IoOperation::CYCLE.is_processing());
        assert!(!IoOperation::THREAD.is_processing());
    }

    #[test]
    fn io_buffer_reports_slice_lengths() {
        let input = [0.1_f32, 0.2, 0.3, 0.4];
        let mut output = [0.0_f32; 4];
        let buf = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        assert_eq!(buf.input_len(), 4);
        assert_eq!(buf.output_len(), 4);
    }

    #[test]
    fn silence_output_zeroes_the_buffer() {
        let input: [f32; 0] = [];
        let mut output = [0.5_f32; 8];
        let mut buf = IoBuffer::new(
            Timestamp::ZERO,
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        buf.silence_output();
        assert!(buf.output.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn output_only_device_sees_empty_input() {
        let input: [f32; 0] = [];
        let mut output = [0.0_f32; 4];
        let buf = IoBuffer::new(
            Timestamp::new(128.0, 999),
            IoOperation::PROCESS_OUTPUT,
            &input,
            &mut output,
        );
        assert_eq!(buf.input_len(), 0);
        assert_eq!(buf.output_len(), 4);
        assert_eq!(buf.timestamp.sample_time, 128.0);
    }

    #[test]
    fn io_operation_layout_is_transparent() {
        use core::mem::size_of;
        assert_eq!(size_of::<IoOperation>(), size_of::<u32>());
    }
}
