//! Control-thread naming of the owned NeonMix HAL device, never a third-party driver.
#![allow(unsafe_code)]
use neonmix_core::AudioError;
use std::ffi::c_void;

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}
#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(
        allocator: *const c_void,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

pub fn set_virtual_output_name(name: &str) -> Result<(), AudioError> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(AudioError::Backend("invalid virtual output name".into()));
    }
    let uid = super::VIRTUAL_OUTPUT_UID;
    // SAFETY: both strings are bounded valid UTF8; CF copies their bytes. All HAL
    // pointers refer to live correctly-sized values, and CF references are released.
    unsafe {
        let cf_uid = CFStringCreateWithBytes(
            std::ptr::null(),
            uid.as_ptr(),
            uid.len() as isize,
            0x08000100,
            0,
        );
        if cf_uid.is_null() {
            return Err(AudioError::Backend("could not create device UID".into()));
        }
        let address = Address {
            selector: u32::from_be_bytes(*b"uidd"),
            scope: u32::from_be_bytes(*b"glob"),
            element: 0,
        };
        let mut device = 0u32;
        let mut size = 4u32;
        let status = AudioObjectGetPropertyData(
            1,
            &address,
            std::mem::size_of_val(&cf_uid) as u32,
            (&cf_uid as *const *const c_void).cast(),
            &mut size,
            (&mut device as *mut u32).cast(),
        );
        CFRelease(cf_uid);
        if status != 0 || device == 0 || size != 4 {
            return Err(AudioError::DeviceUnavailable(uid.into()));
        }
        let value = CFStringCreateWithBytes(
            std::ptr::null(),
            name.as_ptr(),
            name.len() as isize,
            0x08000100,
            0,
        );
        if value.is_null() {
            return Err(AudioError::Backend("could not create output name".into()));
        }
        let address = Address {
            selector: u32::from_be_bytes(*b"lnam"),
            scope: u32::from_be_bytes(*b"glob"),
            element: 0,
        };
        let status = AudioObjectSetPropertyData(
            device,
            &address,
            0,
            std::ptr::null(),
            std::mem::size_of_val(&value) as u32,
            (&value as *const *const c_void).cast(),
        );
        CFRelease(value);
        if status != 0 {
            return Err(AudioError::Backend(format!(
                "native output rename failed: OSStatus {status}"
            )));
        }
        Ok(())
    }
}
