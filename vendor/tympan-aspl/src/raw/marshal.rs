//! Marshalling between the [`abi`] C types and the safe Rust types.
//!
//! The macOS FFI layer (a later PR) receives the HAL's calls in
//! terms of the [`abi`] structs and opaque byte buffers; the rest of
//! the framework speaks [`PropertyAddress`], [`StreamFormat`],
//! [`PropertyValue`], and friends. This module is the bridge — and,
//! crucially, it is plain data conversion with no FFI, so every
//! conversion is unit-tested on any host.
//!
//! ## What this module does *not* do
//!
//! [`PropertyValue::Text`] carries a string that crosses the ABI as
//! a `CFStringRef` — a CoreFoundation object pointer. Creating one
//! needs CoreFoundation, so [`write_property_value`] rejects
//! `Text`; the FFI layer detects that variant first and builds the
//! `CFStringRef` itself. Every other variant is pure bytes and is
//! handled here.
//!
//! [`abi`]: crate::raw::abi

use crate::error::OsStatus;
use crate::format::StreamFormat;
use crate::io::Timestamp;
use crate::property::{
    PropertyAddress, PropertyElement, PropertyScope, PropertySelector, PropertyValue, ValueRange,
};
use crate::raw::abi::{
    AudioObjectPropertyAddress, AudioStreamBasicDescription, AudioTimeStamp, AudioValueRange,
};

/// Convert a raw [`AudioObjectPropertyAddress`] into the safe
/// [`PropertyAddress`].
#[must_use]
pub fn address_from_raw(raw: &AudioObjectPropertyAddress) -> PropertyAddress {
    PropertyAddress::new(
        PropertySelector::from(crate::fourcc::FourCharCode::from_u32(raw.mSelector)),
        PropertyScope::from(crate::fourcc::FourCharCode::from_u32(raw.mScope)),
        PropertyElement::from(raw.mElement),
    )
}

/// Convert a safe [`PropertyAddress`] into the raw
/// [`AudioObjectPropertyAddress`].
#[must_use]
pub fn address_to_raw(address: &PropertyAddress) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: address.selector.code().as_u32(),
        mScope: address.scope.code().as_u32(),
        mElement: address.element.as_u32(),
    }
}

/// Convert a raw [`AudioStreamBasicDescription`] into the safe
/// [`StreamFormat`].
#[must_use]
pub fn format_from_asbd(asbd: &AudioStreamBasicDescription) -> StreamFormat {
    StreamFormat::from_raw(
        asbd.mSampleRate,
        asbd.mFormatID,
        asbd.mFormatFlags,
        asbd.mBytesPerPacket,
        asbd.mFramesPerPacket,
        asbd.mBytesPerFrame,
        asbd.mChannelsPerFrame,
        asbd.mBitsPerChannel,
    )
}

/// Convert a safe [`StreamFormat`] into a raw
/// [`AudioStreamBasicDescription`]. `mReserved` is zeroed, as the
/// HAL expects.
#[must_use]
pub fn format_to_asbd(format: &StreamFormat) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: format.sample_rate(),
        mFormatID: format.format_id(),
        mFormatFlags: format.format_flags(),
        mBytesPerPacket: format.bytes_per_packet(),
        mFramesPerPacket: format.frames_per_packet(),
        mBytesPerFrame: format.bytes_per_frame(),
        mChannelsPerFrame: format.channels(),
        mBitsPerChannel: format.bits_per_channel(),
        mReserved: 0,
    }
}

/// Convert a raw [`AudioValueRange`] into the safe [`ValueRange`].
#[must_use]
pub fn range_from_raw(raw: &AudioValueRange) -> ValueRange {
    ValueRange::new(raw.mMinimum, raw.mMaximum)
}

/// Convert a safe [`ValueRange`] into a raw [`AudioValueRange`].
#[must_use]
pub fn range_to_raw(range: &ValueRange) -> AudioValueRange {
    AudioValueRange {
        mMinimum: range.min,
        mMaximum: range.max,
    }
}

/// Extract the safe [`Timestamp`] from a raw [`AudioTimeStamp`].
///
/// Only `mSampleTime` and `mHostTime` are carried; the framework's
/// IO path does not consult the rate scalar, word clock, or SMPTE
/// fields.
#[must_use]
pub fn timestamp_from_raw(raw: &AudioTimeStamp) -> Timestamp {
    Timestamp::new(raw.mSampleTime, raw.mHostTime)
}

/// Build a raw [`AudioTimeStamp`] from a safe [`Timestamp`].
///
/// The sample-time and host-time fields are filled; the rest are
/// zeroed and `mFlags` is set to mark exactly those two as valid
/// (`kAudioTimeStampSampleHostTimeValid`).
#[must_use]
pub fn timestamp_to_raw(timestamp: &Timestamp) -> AudioTimeStamp {
    // kAudioTimeStampSampleTimeValid (1<<0) | kAudioTimeStampHostTimeValid (1<<1).
    const SAMPLE_HOST_TIME_VALID: u32 = 0b11;
    AudioTimeStamp {
        mSampleTime: timestamp.sample_time,
        mHostTime: timestamp.host_time,
        mFlags: SAMPLE_HOST_TIME_VALID,
        ..AudioTimeStamp::default()
    }
}

/// Read a native-endian `u32` from the head of a HAL property
/// buffer.
///
/// Returns [`OsStatus::BAD_PROPERTY_SIZE`] if `buf` is shorter than
/// four bytes — the error the HAL expects for an undersized
/// `SetPropertyData` buffer.
pub fn read_u32(buf: &[u8]) -> Result<u32, OsStatus> {
    let bytes: [u8; 4] = buf
        .get(..4)
        .ok_or(OsStatus::BAD_PROPERTY_SIZE)?
        .try_into()
        .expect("slice of length 4 converts to [u8; 4]");
    Ok(u32::from_ne_bytes(bytes))
}

/// Read a native-endian `f64` from the head of a HAL property
/// buffer.
///
/// Returns [`OsStatus::BAD_PROPERTY_SIZE`] if `buf` is shorter than
/// eight bytes.
pub fn read_f64(buf: &[u8]) -> Result<f64, OsStatus> {
    let bytes: [u8; 8] = buf
        .get(..8)
        .ok_or(OsStatus::BAD_PROPERTY_SIZE)?
        .try_into()
        .expect("slice of length 8 converts to [u8; 8]");
    Ok(f64::from_ne_bytes(bytes))
}

/// Read an [`AudioStreamBasicDescription`] from the head of a HAL
/// property buffer, returning it as a safe [`StreamFormat`].
///
/// Returns [`OsStatus::BAD_PROPERTY_SIZE`] if `buf` is shorter than
/// [`ASBD_SIZE`](crate::format::ASBD_SIZE) bytes.
pub fn read_asbd(buf: &[u8]) -> Result<StreamFormat, OsStatus> {
    if buf.len() < crate::format::ASBD_SIZE {
        return Err(OsStatus::BAD_PROPERTY_SIZE);
    }
    // The nine fields, in declaration order: one f64 then eight u32.
    let sample_rate = read_f64(&buf[0..8])?;
    let mut u = [0u32; 8];
    for (i, slot) in u.iter_mut().enumerate() {
        let off = 8 + i * 4;
        *slot = read_u32(&buf[off..off + 4])?;
    }
    Ok(StreamFormat::from_raw(
        sample_rate,
        u[0],
        u[1],
        u[2],
        u[3],
        u[4],
        u[5],
        u[6],
    ))
}

/// Serialise a [`PropertyValue`] into a HAL property buffer.
///
/// Writes the value's native-endian bytes into `buf` and returns
/// the number of bytes written. The layout matches what the HAL's
/// `GetPropertyData` caller expects for each variant:
///
/// - [`PropertyValue::U32`] / [`PropertyValue::ObjectId`] — a
///   4-byte `UInt32` / `AudioObjectID`.
/// - [`PropertyValue::F64`] — an 8-byte `Float64`.
/// - [`PropertyValue::ObjectList`] — a packed `AudioObjectID`
///   array.
/// - [`PropertyValue::Format`] — a 40-byte
///   `AudioStreamBasicDescription`.
/// - [`PropertyValue::RangeList`] — a packed `AudioValueRange`
///   array.
///
/// # Errors
///
/// - [`OsStatus::BAD_PROPERTY_SIZE`] if `buf` is too small to hold
///   the value.
/// - [`OsStatus::UNSPECIFIED`] for [`PropertyValue::Text`]: a text
///   value crosses the ABI as a `CFStringRef`, which needs
///   CoreFoundation to construct — the FFI layer handles that
///   variant before reaching this function.
pub fn write_property_value(value: &PropertyValue, buf: &mut [u8]) -> Result<usize, OsStatus> {
    let needed = value.byte_size();
    if buf.len() < needed {
        return Err(OsStatus::BAD_PROPERTY_SIZE);
    }
    match value {
        PropertyValue::U32(v) => {
            buf[..4].copy_from_slice(&v.to_ne_bytes());
        }
        PropertyValue::ObjectId(id) => {
            buf[..4].copy_from_slice(&id.as_u32().to_ne_bytes());
        }
        PropertyValue::F64(v) => {
            buf[..8].copy_from_slice(&v.to_ne_bytes());
        }
        PropertyValue::F32(v) => {
            buf[..4].copy_from_slice(&v.to_ne_bytes());
        }
        PropertyValue::ObjectList(ids) => {
            for (i, id) in ids.iter().enumerate() {
                let off = i * 4;
                buf[off..off + 4].copy_from_slice(&id.as_u32().to_ne_bytes());
            }
        }
        PropertyValue::Format(format) => {
            write_asbd(
                &format_to_asbd(format),
                &mut buf[..crate::format::ASBD_SIZE],
            );
        }
        PropertyValue::RangeList(ranges) => {
            for (i, range) in ranges.iter().enumerate() {
                let off = i * ValueRange::SIZE;
                buf[off..off + 8].copy_from_slice(&range.min.to_ne_bytes());
                buf[off + 8..off + 16].copy_from_slice(&range.max.to_ne_bytes());
            }
        }
        PropertyValue::RangedFormats(formats) => {
            for (i, (format, range)) in formats.iter().enumerate() {
                let off = i * (crate::format::ASBD_SIZE + ValueRange::SIZE);
                write_asbd(
                    &format_to_asbd(format),
                    &mut buf[off..off + crate::format::ASBD_SIZE],
                );
                let off = off + crate::format::ASBD_SIZE;
                buf[off..off + 8].copy_from_slice(&range.min.to_ne_bytes());
                buf[off + 8..off + 16].copy_from_slice(&range.max.to_ne_bytes());
            }
        }
        PropertyValue::Text(_) => return Err(OsStatus::UNSPECIFIED),
        PropertyValue::CustomProperties(properties) => {
            for (i, property) in properties.iter().enumerate() {
                for (field, value) in property.iter().enumerate() {
                    let off = i * 12 + field * 4;
                    buf[off..off + 4].copy_from_slice(&value.to_ne_bytes());
                }
            }
        }
    }
    Ok(needed)
}

/// Write an [`AudioStreamBasicDescription`]'s native-endian bytes
/// into a 40-byte buffer. `buf.len()` must be exactly
/// [`ASBD_SIZE`](crate::format::ASBD_SIZE).
fn write_asbd(asbd: &AudioStreamBasicDescription, buf: &mut [u8]) {
    debug_assert_eq!(buf.len(), crate::format::ASBD_SIZE);
    buf[0..8].copy_from_slice(&asbd.mSampleRate.to_ne_bytes());
    let u = [
        asbd.mFormatID,
        asbd.mFormatFlags,
        asbd.mBytesPerPacket,
        asbd.mFramesPerPacket,
        asbd.mBytesPerFrame,
        asbd.mChannelsPerFrame,
        asbd.mBitsPerChannel,
        asbd.mReserved,
    ];
    for (i, value) in u.iter().enumerate() {
        let off = 8 + i * 4;
        buf[off..off + 4].copy_from_slice(&value.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use super::*;
    use crate::object::AudioObjectId;
    use alloc::vec;

    #[test]
    fn property_address_round_trips() {
        let address = PropertyAddress::new(
            PropertySelector::DEVICE_NOMINAL_SAMPLE_RATE,
            PropertyScope::INPUT,
            PropertyElement::channel(2),
        );
        let raw = address_to_raw(&address);
        assert_eq!(raw.mSelector, address.selector.code().as_u32());
        assert_eq!(raw.mScope, PropertyScope::INPUT.code().as_u32());
        assert_eq!(raw.mElement, 2);
        assert_eq!(address_from_raw(&raw), address);
    }

    #[test]
    fn stream_format_round_trips_through_asbd() {
        for format in [
            StreamFormat::float32(48_000.0, 2),
            StreamFormat::int16(44_100.0, 1),
            StreamFormat::int24(96_000.0, 6),
        ] {
            let asbd = format_to_asbd(&format);
            assert_eq!(asbd.mReserved, 0);
            assert_eq!(asbd.mSampleRate, format.sample_rate());
            assert_eq!(asbd.mChannelsPerFrame, format.channels());
            assert_eq!(format_from_asbd(&asbd), format);
        }
    }

    #[test]
    fn value_range_round_trips() {
        let range = ValueRange::new(44_100.0, 96_000.0);
        let raw = range_to_raw(&range);
        assert_eq!(raw.mMinimum, 44_100.0);
        assert_eq!(raw.mMaximum, 96_000.0);
        assert_eq!(range_from_raw(&raw), range);
    }

    #[test]
    fn timestamp_extracts_sample_and_host_time() {
        let raw = AudioTimeStamp {
            mSampleTime: 128.0,
            mHostTime: 999_999,
            mRateScalar: 1.0,
            ..AudioTimeStamp::default()
        };
        let ts = timestamp_from_raw(&raw);
        assert_eq!(ts.sample_time, 128.0);
        assert_eq!(ts.host_time, 999_999);
    }

    #[test]
    fn timestamp_to_raw_marks_the_two_fields_valid() {
        let raw = timestamp_to_raw(&Timestamp::new(256.0, 42));
        assert_eq!(raw.mSampleTime, 256.0);
        assert_eq!(raw.mHostTime, 42);
        // kAudioTimeStampSampleTimeValid | kAudioTimeStampHostTimeValid.
        assert_eq!(raw.mFlags, 0b11);
        assert_eq!(raw.mRateScalar, 0.0);
    }

    #[test]
    fn read_u32_and_f64_parse_native_endian() {
        assert_eq!(read_u32(&1234u32.to_ne_bytes()), Ok(1234));
        assert_eq!(read_f64(&48_000.0f64.to_ne_bytes()), Ok(48_000.0));
        // Extra trailing bytes are ignored — only the head is read.
        let mut padded = vec![0u8; 16];
        padded[..4].copy_from_slice(&7u32.to_ne_bytes());
        assert_eq!(read_u32(&padded), Ok(7));
    }

    #[test]
    fn read_scalars_reject_short_buffers() {
        assert_eq!(read_u32(&[0u8; 3]), Err(OsStatus::BAD_PROPERTY_SIZE));
        assert_eq!(read_f64(&[0u8; 7]), Err(OsStatus::BAD_PROPERTY_SIZE));
        assert_eq!(read_asbd(&[0u8; 39]), Err(OsStatus::BAD_PROPERTY_SIZE));
    }

    #[test]
    fn asbd_round_trips_through_a_buffer() {
        let format = StreamFormat::float32(48_000.0, 2);
        let value = PropertyValue::Format(format);
        let mut buf = vec![0u8; 64];
        let written = write_property_value(&value, &mut buf).unwrap();
        assert_eq!(written, 40);
        assert_eq!(read_asbd(&buf).unwrap(), format);
    }

    #[test]
    fn write_u32_value() {
        let mut buf = [0u8; 4];
        assert_eq!(
            write_property_value(&PropertyValue::U32(0xABCD), &mut buf),
            Ok(4)
        );
        assert_eq!(u32::from_ne_bytes(buf), 0xABCD);
    }

    #[test]
    fn write_object_id_value() {
        let mut buf = [0u8; 4];
        let id = AudioObjectId::from_u32(7);
        assert_eq!(
            write_property_value(&PropertyValue::ObjectId(id), &mut buf),
            Ok(4)
        );
        assert_eq!(u32::from_ne_bytes(buf), 7);
    }

    #[test]
    fn write_f64_value() {
        let mut buf = [0u8; 8];
        assert_eq!(
            write_property_value(&PropertyValue::F64(96_000.0), &mut buf),
            Ok(8)
        );
        assert_eq!(f64::from_ne_bytes(buf), 96_000.0);
    }

    #[test]
    fn write_object_list_value() {
        let ids = vec![
            AudioObjectId::from_u32(2),
            AudioObjectId::from_u32(3),
            AudioObjectId::from_u32(4),
        ];
        let mut buf = vec![0u8; 12];
        assert_eq!(
            write_property_value(&PropertyValue::ObjectList(ids), &mut buf),
            Ok(12)
        );
        assert_eq!(read_u32(&buf[0..4]), Ok(2));
        assert_eq!(read_u32(&buf[4..8]), Ok(3));
        assert_eq!(read_u32(&buf[8..12]), Ok(4));
    }

    #[test]
    fn write_range_list_value() {
        let ranges = vec![
            ValueRange::point(48_000.0),
            ValueRange::new(44_100.0, 96_000.0),
        ];
        let mut buf = vec![0u8; 32];
        assert_eq!(
            write_property_value(&PropertyValue::RangeList(ranges), &mut buf),
            Ok(32)
        );
        assert_eq!(read_f64(&buf[0..8]), Ok(48_000.0));
        assert_eq!(read_f64(&buf[8..16]), Ok(48_000.0));
        assert_eq!(read_f64(&buf[16..24]), Ok(44_100.0));
        assert_eq!(read_f64(&buf[24..32]), Ok(96_000.0));
    }

    #[test]
    fn write_rejects_an_undersized_buffer() {
        let mut buf = [0u8; 3];
        assert_eq!(
            write_property_value(&PropertyValue::U32(1), &mut buf),
            Err(OsStatus::BAD_PROPERTY_SIZE)
        );
    }

    #[test]
    fn write_rejects_text_values() {
        // A `Text` value crosses the ABI as a `CFStringRef`; the FFI
        // layer builds that, not this module.
        let mut buf = vec![0u8; 64];
        assert_eq!(
            write_property_value(&PropertyValue::Text("Device".into()), &mut buf),
            Err(OsStatus::UNSPECIFIED)
        );
    }

    #[test]
    fn write_accepts_an_oversized_buffer_and_reports_the_used_length() {
        let mut buf = vec![0u8; 100];
        // Only the first 8 bytes are written; the reported length is
        // the value's size, not the buffer's.
        assert_eq!(
            write_property_value(&PropertyValue::F64(1.0), &mut buf),
            Ok(8)
        );
    }
}
