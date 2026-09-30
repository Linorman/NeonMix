//! Minimal CoreFoundation FFI.
//!
//! The framework speaks Core Audio's C ABI almost entirely through
//! plain scalars and `#[repr(C)]` structs — but one property value
//! shape, the text property, crosses the boundary as a
//! `CFStringRef`: a CoreFoundation object pointer. Building one is
//! the single place the framework genuinely calls into
//! CoreFoundation.
//!
//! This module hand-declares exactly the two `CFString` entry
//! points the property layer needs — `CFStringCreateWithBytes` to
//! mint a string and `CFRelease` to drop one — and links
//! `CoreFoundation` for them. It is `cfg(target_os = "macos")`:
//! there is no CoreFoundation elsewhere, and the property layer's
//! text path is stubbed off macOS.

use std::ffi::c_void;

/// An opaque `CFStringRef` — a CoreFoundation string object.
///
/// The framework never inspects one; it only mints a string and
/// forwards the pointer to the HAL, which takes ownership of the
/// `+1` reference and releases it.
pub type CFStringRef = *const c_void;

/// `kCFStringEncodingUTF8` — from `<CoreFoundation/CFString.h>`.
const ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    /// `CFStringCreateWithBytes` — mint a `CFStringRef` from a byte
    /// range in a known encoding. Returns a `+1`-retained string,
    /// or null on failure.
    fn CFStringCreateWithBytes(
        alloc: *const c_void,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external_representation: u8,
    ) -> CFStringRef;

    /// `CFRelease` — drop one reference to a CoreFoundation object.
    fn CFRelease(cf: *const c_void);
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(cf: CFStringRef) -> isize;
    fn CFStringGetCharacters(cf: CFStringRef, range: CFRange, buffer: *mut u16);
}

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

/// Copy a borrowed HAL CFString into a bounded UTF-8 name.
/// # Safety
/// `cf` must be null or a live CF object supplied by the Core Audio property marshaller.
pub unsafe fn read_name(cf: CFStringRef) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    // SAFETY: HAL supplies a live CF object; its type is checked before String APIs.
    if unsafe { CFGetTypeID(cf) != CFStringGetTypeID() } {
        return None;
    }
    // SAFETY: type was verified above, and the borrowed reference outlives this call.
    let length = unsafe { CFStringGetLength(cf) };
    if !(1..=256).contains(&length) {
        return None;
    }
    let mut characters = [0u16; 256];
    // SAFETY: the range is inside this CFString and the destination has enough elements.
    unsafe {
        CFStringGetCharacters(
            cf,
            CFRange {
                location: 0,
                length,
            },
            characters.as_mut_ptr(),
        )
    };
    let value = String::from_utf16(&characters[..length as usize]).ok()?;
    if value.len() > 256 || value.trim().is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value)
}

/// Compare a borrowed UID qualifier without allocating or accepting a non-string.
/// # Safety
/// `cf` must be null or a live CF object supplied by the host for this call.
pub unsafe fn uid_equals(cf: CFStringRef, expected: &str) -> Option<bool> {
    if cf.is_null() {
        return None;
    }
    // SAFETY: the caller supplies a live object; type is checked before string APIs.
    if unsafe { CFGetTypeID(cf) != CFStringGetTypeID() } {
        return None;
    }
    // SAFETY: type checked above, reference remains borrowed for this call.
    let length = unsafe { CFStringGetLength(cf) };
    if length < 0 {
        return None;
    }
    if length > 256 {
        return Some(false);
    }
    let mut characters = [0u16; 256];
    // SAFETY: the exact bounded CFString range fits the stack destination.
    unsafe {
        CFStringGetCharacters(
            cf,
            CFRange {
                location: 0,
                length,
            },
            characters.as_mut_ptr(),
        );
    }
    Some(
        expected
            .encode_utf16()
            .eq(characters[..length as usize].iter().copied()),
    )
}

/// Mint a `CFStringRef` from a UTF-8 string.
///
/// The returned string carries a `+1` reference the caller owns —
/// the property getter hands it to the HAL, which releases it.
/// Returns null if CoreFoundation could not allocate the string.
#[must_use]
pub fn create_string(text: &str) -> CFStringRef {
    // Safety: `text.as_ptr()` / `text.len()` describe a valid UTF-8
    // byte range; `CFStringCreateWithBytes` copies those bytes and
    // does not retain the pointer. A null allocator selects the
    // default allocator.
    unsafe {
        CFStringCreateWithBytes(
            core::ptr::null(),
            text.as_ptr(),
            text.len() as isize,
            ENCODING_UTF8,
            0,
        )
    }
}

/// Drop one reference to a CoreFoundation object.
///
/// # Safety
///
/// `cf` must be a non-null pointer to a live CoreFoundation object
/// the caller owns a reference to. After this call the caller must
/// not use `cf` again unless it holds another reference.
pub unsafe fn release(cf: *const c_void) {
    // Safety: forwarded from the caller's contract.
    unsafe { CFRelease(cf) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_string_returns_a_live_object_that_releases() {
        let cfstr = create_string("Tympan Minimal Loopback");
        assert!(!cfstr.is_null(), "CFStringCreateWithBytes should succeed");
        // Safety: `cfstr` is the `+1`-retained string we just
        // created and have not yet released.
        unsafe { release(cfstr) };
    }

    #[test]
    fn create_string_handles_an_empty_string() {
        let cfstr = create_string("");
        assert!(
            !cfstr.is_null(),
            "an empty CFString is still a valid object"
        );
        // Safety: as above.
        unsafe { release(cfstr) };
    }

    #[test]
    fn create_string_handles_non_ascii() {
        // The UID and name strings are arbitrary UTF-8.
        let cfstr = create_string("Tympan — 鼓膜 — loopback");
        assert!(!cfstr.is_null());
        // Safety: as above.
        unsafe { release(cfstr) };
    }
}
