//! `OSStatus` wrapper and Core Audio error constants.
//!
//! Every Core Audio HAL and AudioServerPlugin entry point returns an
//! `OSStatus` — a signed 32-bit result code where `0`
//! (`kAudioHardwareNoError`) is success and any non-zero value is a
//! failure. Most HAL error codes are themselves four-character codes
//! (`'!obj'`, `'who?'`, …) reinterpreted as `i32`.
//!
//! [`OsStatus`] is a `#[repr(transparent)]` newtype over `i32` so it
//! can sit in `extern "C"` return positions unchanged while giving
//! the higher layers `is_ok` / `ok()` ergonomics and a four-char
//! aware [`Debug`] rendering.

use core::fmt;

use crate::fourcc::FourCharCode;

/// A Core Audio `OSStatus` result code.
///
/// Layout-compatible with the C `OSStatus` (`i32`). `0` is success;
/// every other value is a failure. The associated constants cover
/// the HAL error codes an AudioServerPlugin is expected to return
/// from its property and IO entry points.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct OsStatus(pub i32);

impl OsStatus {
    /// `kAudioHardwareNoError` — the operation succeeded.
    pub const OK: Self = Self(0);

    /// `kAudioHardwareNotRunningError` (`'stop'`) — the operation
    /// could not be completed because the hardware is not running.
    pub const NOT_RUNNING: Self = Self::from_fourcc(FourCharCode::new(*b"stop"));
    /// `kAudioHardwareUnspecifiedError` (`'what'`) — an
    /// unspecified, catch-all failure.
    pub const UNSPECIFIED: Self = Self::from_fourcc(FourCharCode::new(*b"what"));
    /// `kAudioHardwareUnknownPropertyError` (`'who?'`) — the object
    /// does not recognise the requested property.
    pub const UNKNOWN_PROPERTY: Self = Self::from_fourcc(FourCharCode::new(*b"who?"));
    /// `kAudioHardwareBadPropertySizeError` (`'!siz'`) — the buffer
    /// supplied for a property value is the wrong size.
    pub const BAD_PROPERTY_SIZE: Self = Self::from_fourcc(FourCharCode::new(*b"!siz"));
    /// `kAudioHardwareIllegalOperationError` (`'nope'`) — the
    /// operation is not legal in the object's current state.
    pub const ILLEGAL_OPERATION: Self = Self::from_fourcc(FourCharCode::new(*b"nope"));
    /// `kAudioHardwareBadObjectError` (`'!obj'`) — the
    /// `AudioObjectID` does not map to a known object.
    pub const BAD_OBJECT: Self = Self::from_fourcc(FourCharCode::new(*b"!obj"));
    /// `kAudioHardwareBadDeviceError` (`'!dev'`) — the
    /// `AudioObjectID` is not a device.
    pub const BAD_DEVICE: Self = Self::from_fourcc(FourCharCode::new(*b"!dev"));
    /// `kAudioHardwareBadStreamError` (`'!str'`) — the
    /// `AudioObjectID` is not a stream.
    pub const BAD_STREAM: Self = Self::from_fourcc(FourCharCode::new(*b"!str"));
    /// `kAudioHardwareUnsupportedOperationError` (`'unop'`) — the
    /// object does not support the requested operation.
    pub const UNSUPPORTED_OPERATION: Self = Self::from_fourcc(FourCharCode::new(*b"unop"));
    /// `kAudioHardwareNotReadyError` (`'nrdy'`) — the object is not
    /// ready to perform the operation yet.
    pub const NOT_READY: Self = Self::from_fourcc(FourCharCode::new(*b"nrdy"));
    /// `kAudioDeviceUnsupportedFormatError` (`'!dat'`) — the device
    /// does not support the requested stream format.
    pub const UNSUPPORTED_FORMAT: Self = Self::from_fourcc(FourCharCode::new(*b"!dat"));
    /// `kAudioDevicePermissionsError` (`'!hog'`) — the device
    /// cannot be used because another process holds it exclusively.
    pub const PERMISSIONS: Self = Self::from_fourcc(FourCharCode::new(*b"!hog"));

    /// Reinterpret a [`FourCharCode`] as an `OsStatus`. The HAL
    /// error codes are defined this way in `<CoreAudio/AudioHardwareBase.h>`.
    #[inline]
    #[must_use]
    pub const fn from_fourcc(code: FourCharCode) -> Self {
        Self(code.as_u32() as i32)
    }

    /// The raw `i32`, ready for an `extern "C"` return position.
    #[inline]
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self.0
    }

    /// `true` iff this is [`Self::OK`].
    #[inline]
    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }

    /// `true` iff this is any value other than [`Self::OK`].
    #[inline]
    #[must_use]
    pub const fn is_err(self) -> bool {
        self.0 != 0
    }

    /// Convert to a `Result<(), Self>`: [`Self::OK`] maps to
    /// `Ok(())`, every other code to `Err(self)`.
    #[inline]
    pub const fn ok(self) -> Result<(), Self> {
        if self.is_ok() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl fmt::Debug for OsStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_ok() {
            return write!(f, "OsStatus::OK");
        }
        // Most HAL error codes are four-character codes; render them
        // that way when the bytes are printable, otherwise show the
        // signed and hex forms.
        let code = FourCharCode::from_u32(self.0 as u32);
        if code.is_printable() {
            write!(f, "OsStatus({code})")
        } else {
            write!(f, "OsStatus({} / 0x{:08X})", self.0, self.0 as u32)
        }
    }
}

impl fmt::Display for OsStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl From<i32> for OsStatus {
    #[inline]
    fn from(value: i32) -> Self {
        Self(value)
    }
}

impl From<OsStatus> for i32 {
    #[inline]
    fn from(value: OsStatus) -> Self {
        value.0
    }
}

impl From<Result<(), OsStatus>> for OsStatus {
    /// Flatten a `Result` back into a raw status for the FFI return
    /// position: `Ok(())` becomes [`OsStatus::OK`], `Err(s)` becomes
    /// `s`.
    #[inline]
    fn from(value: Result<(), OsStatus>) -> Self {
        match value {
            Ok(()) => Self::OK,
            Err(status) => status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_classifies_correctly() {
        assert!(OsStatus::OK.is_ok());
        assert!(!OsStatus::OK.is_err());
        assert_eq!(OsStatus::OK.as_i32(), 0);
    }

    #[test]
    fn error_codes_classify_correctly() {
        for e in [
            OsStatus::UNSPECIFIED,
            OsStatus::UNKNOWN_PROPERTY,
            OsStatus::BAD_OBJECT,
            OsStatus::BAD_DEVICE,
            OsStatus::BAD_STREAM,
            OsStatus::ILLEGAL_OPERATION,
            OsStatus::UNSUPPORTED_FORMAT,
        ] {
            assert!(e.is_err(), "{e:?} should be an error");
            assert!(!e.is_ok());
        }
    }

    #[test]
    fn ok_method_converts_to_result() {
        assert_eq!(OsStatus::OK.ok(), Ok(()));
        assert_eq!(OsStatus::BAD_OBJECT.ok(), Err(OsStatus::BAD_OBJECT));
    }

    #[test]
    fn result_flattens_back_to_status() {
        assert_eq!(OsStatus::from(Ok(())), OsStatus::OK);
        assert_eq!(
            OsStatus::from(Err(OsStatus::NOT_READY)),
            OsStatus::NOT_READY
        );
    }

    #[test]
    fn error_constants_match_fourcc_values() {
        // Spot-check against the documented `AudioHardwareBase.h`
        // four-character codes.
        assert_eq!(OsStatus::BAD_OBJECT.as_i32(), 0x216F_626A); // '!obj'
        assert_eq!(OsStatus::UNKNOWN_PROPERTY.as_i32(), 0x77686F3F); // 'who?'
        assert_eq!(OsStatus::NOT_RUNNING.as_i32(), 0x73746F70); // 'stop'
    }

    #[test]
    fn round_trips_through_i32() {
        for e in [OsStatus::OK, OsStatus::BAD_DEVICE, OsStatus::PERMISSIONS] {
            let raw: i32 = e.into();
            assert_eq!(OsStatus::from(raw), e);
        }
    }

    #[test]
    fn debug_renders_ok_specially() {
        assert_eq!(format!("{:?}", OsStatus::OK), "OsStatus::OK");
    }

    #[test]
    fn debug_renders_fourcc_errors() {
        assert_eq!(format!("{:?}", OsStatus::BAD_OBJECT), "OsStatus('!obj')");
        assert_eq!(
            format!("{:?}", OsStatus::UNKNOWN_PROPERTY),
            "OsStatus('who?')"
        );
    }

    #[test]
    fn debug_renders_non_fourcc_errors_numerically() {
        let s = OsStatus(-1);
        assert_eq!(format!("{s:?}"), "OsStatus(-1 / 0xFFFFFFFF)");
    }

    #[test]
    fn layout_is_transparent_i32() {
        use core::mem::{align_of, size_of};
        assert_eq!(size_of::<OsStatus>(), size_of::<i32>());
        assert_eq!(align_of::<OsStatus>(), align_of::<i32>());
    }
}
