//! Four-character codes.
//!
//! Core Audio identifies selectors, scopes, format IDs, error codes,
//! and transport types with *four-character codes* — a `u32` whose
//! four bytes are printable ASCII, written in the C headers as
//! multi-character literals such as `'glob'` or `'lpcm'`. The bytes
//! are stored most-significant-first, so `'glob'` is
//! `0x676C6F62`.
//!
//! [`FourCharCode`] is a `#[repr(transparent)]` newtype over `u32`
//! so it round-trips losslessly through the FFI boundary while
//! giving the higher layers a type with a meaningful [`Debug`]
//! rendering.

use core::fmt;

/// A Core Audio four-character code.
///
/// Layout-compatible with the `u32` (and the C `OSType` /
/// `FourCharCode` typedefs) it bridges. Construct one from a
/// 4-byte ASCII literal with [`FourCharCode::new`], or from a raw
/// `u32` with [`FourCharCode::from_u32`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct FourCharCode(pub u32);

impl FourCharCode {
    /// The all-zero code. Core Audio uses this as a "wildcard" or
    /// "unset" sentinel in several property addresses.
    pub const ZERO: Self = Self(0);

    /// Build a code from four bytes, most-significant byte first.
    ///
    /// ```
    /// use tympan_aspl::FourCharCode;
    /// // 'glob' — kAudioObjectPropertyScopeGlobal.
    /// let scope = FourCharCode::new(*b"glob");
    /// assert_eq!(scope.as_u32(), 0x676C6F62);
    /// ```
    #[inline]
    #[must_use]
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(u32::from_be_bytes(bytes))
    }

    /// Build a code from a raw `u32`. Inverse of
    /// [`Self::as_u32`].
    #[inline]
    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    /// The raw `u32` value, ready for the FFI boundary.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// The four bytes, most-significant first.
    #[inline]
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    /// `true` iff every byte of the code is printable ASCII
    /// (`0x20..=0x7E`). Codes that fail this predicate still
    /// round-trip through the FFI boundary; the check only governs
    /// whether [`Display`](fmt::Display) renders the quoted form or
    /// the hexadecimal fallback.
    #[inline]
    #[must_use]
    pub const fn is_printable(self) -> bool {
        let b = self.to_bytes();
        let mut i = 0;
        while i < 4 {
            if b[i] < 0x20 || b[i] > 0x7E {
                return false;
            }
            i += 1;
        }
        true
    }
}

impl fmt::Display for FourCharCode {
    /// Renders the quoted four-character form (`'glob'`) when every
    /// byte is printable ASCII, falling back to `0x........` hex
    /// otherwise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_printable() {
            let b = self.to_bytes();
            write!(
                f,
                "'{}{}{}{}'",
                b[0] as char, b[1] as char, b[2] as char, b[3] as char
            )
        } else {
            write!(f, "0x{:08X}", self.0)
        }
    }
}

impl fmt::Debug for FourCharCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCharCode({self})")
    }
}

impl From<u32> for FourCharCode {
    #[inline]
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<FourCharCode> for u32 {
    #[inline]
    fn from(value: FourCharCode) -> Self {
        value.0
    }
}

impl From<[u8; 4]> for FourCharCode {
    #[inline]
    fn from(bytes: [u8; 4]) -> Self {
        Self::new(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_packs_bytes_big_endian() {
        assert_eq!(FourCharCode::new(*b"glob").as_u32(), 0x676C_6F62);
        assert_eq!(FourCharCode::new(*b"lpcm").as_u32(), 0x6C70_636D);
    }

    #[test]
    fn round_trips_through_u32() {
        let c = FourCharCode::new(*b"dev#");
        assert_eq!(FourCharCode::from_u32(c.as_u32()), c);
        let raw: u32 = c.into();
        assert_eq!(FourCharCode::from(raw), c);
    }

    #[test]
    fn round_trips_through_bytes() {
        for code in [*b"glob", *b"inpt", *b"outp", *b"uid ", *b"nsrt"] {
            let c = FourCharCode::new(code);
            assert_eq!(c.to_bytes(), code);
        }
    }

    #[test]
    fn zero_is_zero() {
        assert_eq!(FourCharCode::ZERO.as_u32(), 0);
        assert_eq!(FourCharCode::ZERO.to_bytes(), [0; 4]);
    }

    #[test]
    fn printable_detection() {
        assert!(FourCharCode::new(*b"glob").is_printable());
        assert!(FourCharCode::new(*b"uid ").is_printable());
        assert!(!FourCharCode::ZERO.is_printable());
        assert!(!FourCharCode::from_u32(0x0000_0001).is_printable());
    }

    #[test]
    fn display_quotes_printable_codes() {
        assert_eq!(format!("{}", FourCharCode::new(*b"glob")), "'glob'");
        assert_eq!(format!("{}", FourCharCode::new(*b"uid ")), "'uid '");
    }

    #[test]
    fn display_falls_back_to_hex_for_unprintable() {
        assert_eq!(format!("{}", FourCharCode::ZERO), "0x00000000");
        assert_eq!(
            format!("{}", FourCharCode::from_u32(0xDEAD_BEEF)),
            "0xDEADBEEF"
        );
    }

    #[test]
    fn debug_wraps_display() {
        assert_eq!(
            format!("{:?}", FourCharCode::new(*b"clas")),
            "FourCharCode('clas')"
        );
    }

    #[test]
    fn ordering_is_by_raw_value() {
        assert!(FourCharCode::from_u32(1) < FourCharCode::from_u32(2));
    }

    #[test]
    fn layout_is_transparent_u32() {
        use core::mem::{align_of, size_of};
        assert_eq!(size_of::<FourCharCode>(), size_of::<u32>());
        assert_eq!(align_of::<FourCharCode>(), align_of::<u32>());
    }
}
