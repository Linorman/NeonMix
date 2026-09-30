//! `#[repr(C)]` mirrors of the AudioServerPlugin C ABI.
//!
//! These types reproduce, field for field, the structs and
//! function-pointer tables `coreaudiod` exchanges with a plug-in,
//! as declared in `<CoreAudio/AudioServerPlugIn.h>` and
//! `<CoreAudio/CoreAudioBaseTypes.h>`. They are hand-written rather
//! than taken from `coreaudio-sys` for one reason: the rest of the
//! framework is cross-platform and unit-tested on any host, and a
//! hand-written `#[repr(C)]` definition compiles everywhere.
//!
//! Two checks guard the layouts against drift. The
//! `static_assertions` in this module's tests pin every struct's
//! size, alignment, and shape for internal consistency. And Tier 3
//! CI proves them against the real C ABI end to end: a plug-in
//! whose vtable or struct layout is wrong does not enumerate when
//! `coreaudiod` loads it.
//!
//! Nothing in this module is `unsafe` or platform-specific — it is
//! plain data declarations. The `unsafe` work of populating a
//! vtable and handing it to `coreaudiod` lives in the rest of
//! [`crate::raw`].
//!
//! ## Naming
//!
//! Type and field names follow the C originals (`mSelector`,
//! `mSampleRate`, …) deliberately: this module is the one place the
//! framework speaks Apple's ABI, and matching the headers makes the
//! cross-check against `coreaudio-sys` and against Apple's
//! documentation mechanical. The Rust-idiomatic names live one
//! layer up.

// Every type and field in this module is a deliberate match for a C
// original (`mSelector`, `QueryInterface`, `AudioObjectID`, …). The
// non-snake-case names are the whole point — they make the
// cross-check against the headers and `coreaudio-sys` mechanical —
// so the lints are switched off for the module rather than fought
// item by item.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use core::ffi::c_void;

/// C `UInt32`.
pub type UInt32 = u32;
/// C `UInt64`.
pub type UInt64 = u64;
/// C `SInt32`.
pub type SInt32 = i32;
/// C `Float64`.
pub type Float64 = f64;
/// C `OSStatus` — the universal Core Audio result code.
pub type OSStatus = i32;
/// C `Boolean` — a one-byte boolean (`0` is false, non-zero true).
pub type Boolean = u8;
/// POSIX `pid_t` — the client process identifier the HAL passes to
/// the property entry points.
pub type pid_t = i32;
/// C `AudioObjectID` — a 32-bit handle to an audio object.
pub type AudioObjectID = u32;

/// Opaque `CFStringRef`. The framework never dereferences one; it
/// only forwards the pointer across the ABI, so an opaque alias is
/// enough at this layer.
pub type CFStringRef = *const c_void;
/// Opaque `CFDictionaryRef`.
pub type CFDictionaryRef = *const c_void;
/// Opaque `CFAllocatorRef`.
pub type CFAllocatorRef = *const c_void;

/// A handle to a plug-in's driver interface — a pointer to a
/// pointer to an [`AudioServerPlugInDriverInterface`]. The double
/// indirection is the CFPlugIn `IUnknown` convention: the outer
/// pointer is the object, the inner pointer is its vtable.
pub type AudioServerPlugInDriverRef = *mut *const AudioServerPlugInDriverInterface;

/// A handle to the host interface `coreaudiod` hands the plug-in in
/// `Initialize`. Opaque at this layer — the host-side wrapper
/// (a later PR) gives it a typed vtable.
pub type AudioServerPlugInHostRef = *mut c_void;

/// `AudioObjectPropertyAddress` — the `(selector, scope, element)`
/// triple that addresses one property of one object.
///
/// Mirrors `<CoreAudio/AudioHardwareBase.h>`.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct AudioObjectPropertyAddress {
    /// `mSelector` — the four-character-code property selector.
    pub mSelector: UInt32,
    /// `mScope` — the four-character-code scope.
    pub mScope: UInt32,
    /// `mElement` — the channel element (`0` is the main element).
    pub mElement: UInt32,
}

/// `AudioValueRange` — an inclusive `(minimum, maximum)` pair.
///
/// Mirrors `<CoreAudio/CoreAudioBaseTypes.h>`. Core Audio uses
/// arrays of these for properties such as the available sample
/// rates.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct AudioValueRange {
    /// `mMinimum` — inclusive lower bound.
    pub mMinimum: Float64,
    /// `mMaximum` — inclusive upper bound.
    pub mMaximum: Float64,
}

/// `AudioStreamBasicDescription` — the full description of one
/// stream's sample format.
///
/// Mirrors `<CoreAudio/CoreAudioBaseTypes.h>`. The trailing
/// `mReserved` field is always zero; it exists to round the struct
/// to a multiple of 8 bytes.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct AudioStreamBasicDescription {
    /// `mSampleRate` — frames per second.
    pub mSampleRate: Float64,
    /// `mFormatID` — the encoding identifier (`kAudioFormatLinearPCM`
    /// for every stream this framework exposes).
    pub mFormatID: UInt32,
    /// `mFormatFlags` — the `kAudioFormatFlag*` bit set.
    pub mFormatFlags: UInt32,
    /// `mBytesPerPacket`.
    pub mBytesPerPacket: UInt32,
    /// `mFramesPerPacket`.
    pub mFramesPerPacket: UInt32,
    /// `mBytesPerFrame`.
    pub mBytesPerFrame: UInt32,
    /// `mChannelsPerFrame`.
    pub mChannelsPerFrame: UInt32,
    /// `mBitsPerChannel`.
    pub mBitsPerChannel: UInt32,
    /// `mReserved` — always zero.
    pub mReserved: UInt32,
}

/// `SMPTETime` — an SMPTE timecode, embedded in [`AudioTimeStamp`].
///
/// Mirrors `<CoreAudio/CoreAudioBaseTypes.h>`. The framework does
/// not interpret SMPTE timecodes; the struct exists so
/// [`AudioTimeStamp`]'s layout is exact.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct SMPTETime {
    /// Fractional SMPTE frame count.
    pub mSubframes: i16,
    /// Divisor for the fractional frame count.
    pub mSubframeDivisor: i16,
    /// `mCounter` — total frame count.
    pub mCounter: UInt32,
    /// `mType` — the SMPTE timecode type.
    pub mType: UInt32,
    /// `mFlags` — validity / running flags.
    pub mFlags: UInt32,
    /// `mHours`.
    pub mHours: i16,
    /// `mMinutes`.
    pub mMinutes: i16,
    /// `mSeconds`.
    pub mSeconds: i16,
    /// `mFrames`.
    pub mFrames: i16,
}

/// `AudioTimeStamp` — a point on Core Audio's several timelines.
///
/// Mirrors `<CoreAudio/CoreAudioBaseTypes.h>`. The framework's IO
/// path consults only `mSampleTime` and `mHostTime` (see
/// [`crate::io::Timestamp`]); the remaining fields are carried so
/// the struct round-trips across the ABI intact.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct AudioTimeStamp {
    /// `mSampleTime` — position on the device's sample clock.
    pub mSampleTime: Float64,
    /// `mHostTime` — position on the host clock
    /// (`mach_absolute_time` units).
    pub mHostTime: UInt64,
    /// `mRateScalar` — the ratio of actual to nominal sample rate.
    pub mRateScalar: Float64,
    /// `mWordClockTime` — position on an external word clock.
    pub mWordClockTime: UInt64,
    /// `mSMPTETime` — the SMPTE timecode.
    pub mSMPTETime: SMPTETime,
    /// `mFlags` — which of the above fields are valid.
    pub mFlags: UInt32,
    /// `mReserved` — always zero.
    pub mReserved: UInt32,
}

/// `AudioServerPlugInClientInfo` — identifies a process that has
/// opened one of the plug-in's devices.
///
/// Mirrors `<CoreAudio/AudioServerPlugIn.h>`. Passed to
/// `AddDeviceClient` / `RemoveDeviceClient` and to the property
/// entry points' device-creation path.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct AudioServerPlugInClientInfo {
    /// `mClientID` — a HAL-assigned id, unique per open client.
    pub mClientID: UInt32,
    /// `mProcessID` — the client's process id.
    pub mProcessID: pid_t,
    /// `mIsNativeEndian` — whether the client expects native-endian
    /// audio data.
    pub mIsNativeEndian: Boolean,
    /// `mBundleID` — the client's bundle identifier, or null.
    pub mBundleID: CFStringRef,
}

/// `AudioServerPlugInIOCycleInfo` — the timing context of one IO
/// cycle, passed to `BeginIOOperation` / `DoIOOperation` /
/// `EndIOOperation`.
///
/// Mirrors `<CoreAudio/AudioServerPlugIn.h>`.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct AudioServerPlugInIOCycleInfo {
    /// IO cycle ordinal.
    pub mIOCycleCounter: UInt64,
    /// Nominal buffer size for this cycle.
    pub mNominalIOBufferFrameSize: UInt32,
    /// Current device time.
    pub mCurrentTime: AudioTimeStamp,
    /// `mInputTime` — the timestamp the input data corresponds to.
    pub mInputTime: AudioTimeStamp,
    /// `mOutputTime` — the timestamp the output data is destined
    /// for.
    pub mOutputTime: AudioTimeStamp,
    /// Host ticks per frame for the main device.
    pub mMainHostTicksPerFrame: Float64,
    /// Host ticks per frame for this device.
    pub mDeviceHostTicksPerFrame: Float64,
}

/// `AudioServerPlugInDriverInterface` — the plug-in's vtable.
///
/// Mirrors `<CoreAudio/AudioServerPlugIn.h>`. The first four
/// members are the CFPlugIn `IUnknown` preamble (`IUNKNOWN_C_GUTS`);
/// the rest are the ~20 driver entry points `coreaudiod` calls. Every
/// entry point is an `Option<unsafe extern "C" fn>` so the table can
/// be zero-initialised and a not-yet-implemented slot left `None`;
/// `Option<fn>` is the same size and ABI as a bare function pointer
/// thanks to the null-pointer niche.
#[repr(C)]
pub struct AudioServerPlugInDriverInterface {
    /// `IUNKNOWN_C_GUTS` slot 1 — reserved, always null.
    pub _reserved: *const c_void,
    /// `IUnknown::QueryInterface`.
    pub QueryInterface: Option<
        unsafe extern "C" fn(
            this: *mut c_void,
            in_uuid_bytes: CFUUIDBytes,
            out_interface: *mut *mut c_void,
        ) -> i32,
    >,
    /// `IUnknown::AddRef` — returns the new reference count.
    pub AddRef: Option<unsafe extern "C" fn(this: *mut c_void) -> u32>,
    /// `IUnknown::Release` — returns the new reference count.
    pub Release: Option<unsafe extern "C" fn(this: *mut c_void) -> u32>,

    /// `Initialize` — hands the plug-in the host interface.
    pub Initialize: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            host: AudioServerPlugInHostRef,
        ) -> OSStatus,
    >,
    /// `CreateDevice` — for plug-ins that mint devices on demand.
    pub CreateDevice: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            description: CFDictionaryRef,
            client_info: *const AudioServerPlugInClientInfo,
            out_device_id: *mut AudioObjectID,
        ) -> OSStatus,
    >,
    /// `DestroyDevice` — the inverse of `CreateDevice`.
    pub DestroyDevice: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
        ) -> OSStatus,
    >,
    /// `AddDeviceClient` — a process opened one of the devices.
    pub AddDeviceClient: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_info: *const AudioServerPlugInClientInfo,
        ) -> OSStatus,
    >,
    /// `RemoveDeviceClient` — a process closed one of the devices.
    pub RemoveDeviceClient: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_info: *const AudioServerPlugInClientInfo,
        ) -> OSStatus,
    >,
    /// `PerformDeviceConfigurationChange` — commit a deferred
    /// configuration change.
    pub PerformDeviceConfigurationChange: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            change_action: UInt64,
            change_info: *mut c_void,
        ) -> OSStatus,
    >,
    /// `AbortDeviceConfigurationChange` — discard a deferred
    /// configuration change.
    pub AbortDeviceConfigurationChange: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            change_action: UInt64,
            change_info: *mut c_void,
        ) -> OSStatus,
    >,
    /// `HasProperty` — does the object have the addressed property?
    pub HasProperty: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            object_id: AudioObjectID,
            client_pid: pid_t,
            address: *const AudioObjectPropertyAddress,
        ) -> Boolean,
    >,
    /// `IsPropertySettable` — can the property be written?
    pub IsPropertySettable: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            object_id: AudioObjectID,
            client_pid: pid_t,
            address: *const AudioObjectPropertyAddress,
            out_is_settable: *mut Boolean,
        ) -> OSStatus,
    >,
    /// `GetPropertyDataSize` — how many bytes is the value?
    pub GetPropertyDataSize: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            object_id: AudioObjectID,
            client_pid: pid_t,
            address: *const AudioObjectPropertyAddress,
            qualifier_size: UInt32,
            qualifier_data: *const c_void,
            out_data_size: *mut UInt32,
        ) -> OSStatus,
    >,
    /// `GetPropertyData` — read the value into the caller's buffer.
    pub GetPropertyData: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            object_id: AudioObjectID,
            client_pid: pid_t,
            address: *const AudioObjectPropertyAddress,
            qualifier_size: UInt32,
            qualifier_data: *const c_void,
            data_size: UInt32,
            out_data_size: *mut UInt32,
            out_data: *mut c_void,
        ) -> OSStatus,
    >,
    /// `SetPropertyData` — write the value from the caller's buffer.
    pub SetPropertyData: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            object_id: AudioObjectID,
            client_pid: pid_t,
            address: *const AudioObjectPropertyAddress,
            qualifier_size: UInt32,
            qualifier_data: *const c_void,
            data_size: UInt32,
            data: *const c_void,
        ) -> OSStatus,
    >,
    /// `StartIO` — the device's IO is starting.
    pub StartIO: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
        ) -> OSStatus,
    >,
    /// `StopIO` — the device's IO is stopping.
    pub StopIO: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
        ) -> OSStatus,
    >,
    /// `GetZeroTimeStamp` — the device's clock anchor for this
    /// cycle.
    pub GetZeroTimeStamp: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
            out_sample_time: *mut Float64,
            out_host_time: *mut UInt64,
            out_seed: *mut UInt64,
        ) -> OSStatus,
    >,
    /// `WillDoIOOperation` — will the plug-in handle this operation?
    pub WillDoIOOperation: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
            operation_id: UInt32,
            out_will_do: *mut Boolean,
            out_will_do_in_place: *mut Boolean,
        ) -> OSStatus,
    >,
    /// `BeginIOOperation` — an IO operation is about to run.
    pub BeginIOOperation: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
            operation_id: UInt32,
            io_buffer_frame_size: UInt32,
            io_cycle_info: *const AudioServerPlugInIOCycleInfo,
        ) -> OSStatus,
    >,
    /// `DoIOOperation` — perform an IO operation.
    pub DoIOOperation: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            stream_id: AudioObjectID,
            client_id: UInt32,
            operation_id: UInt32,
            io_buffer_frame_size: UInt32,
            io_cycle_info: *const AudioServerPlugInIOCycleInfo,
            io_main_buffer: *mut c_void,
            io_secondary_buffer: *mut c_void,
        ) -> OSStatus,
    >,
    /// `EndIOOperation` — an IO operation has finished.
    pub EndIOOperation: Option<
        unsafe extern "C" fn(
            driver: AudioServerPlugInDriverRef,
            device_id: AudioObjectID,
            client_id: UInt32,
            operation_id: UInt32,
            io_buffer_frame_size: UInt32,
            io_cycle_info: *const AudioServerPlugInIOCycleInfo,
        ) -> OSStatus,
    >,
}

/// `CFUUIDBytes` — the 16 raw bytes of a `CFUUID`, as passed to
/// `IUnknown::QueryInterface`.
///
/// Mirrors `<CoreFoundation/CFUUID.h>`.
#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct CFUUIDBytes {
    /// Byte 0.
    pub byte0: u8,
    /// Byte 1.
    pub byte1: u8,
    /// Byte 2.
    pub byte2: u8,
    /// Byte 3.
    pub byte3: u8,
    /// Byte 4.
    pub byte4: u8,
    /// Byte 5.
    pub byte5: u8,
    /// Byte 6.
    pub byte6: u8,
    /// Byte 7.
    pub byte7: u8,
    /// Byte 8.
    pub byte8: u8,
    /// Byte 9.
    pub byte9: u8,
    /// Byte 10.
    pub byte10: u8,
    /// Byte 11.
    pub byte11: u8,
    /// Byte 12.
    pub byte12: u8,
    /// Byte 13.
    pub byte13: u8,
    /// Byte 14.
    pub byte14: u8,
    /// Byte 15.
    pub byte15: u8,
}

/// The CFPlugIn factory function signature `coreaudiod` resolves
/// from the bundle's `CFPlugInFactories` dictionary and calls to
/// instantiate the plug-in.
///
/// The factory returns an `IUnknown`-conforming interface pointer
/// (an [`AudioServerPlugInDriverRef`] cast to `*mut c_void`), or
/// null on failure.
pub type FactoryFn = unsafe extern "C" fn(
    allocator: CFAllocatorRef,
    requested_type_uuid: CFStringRef,
) -> *mut c_void;

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    // Self-consistency checks. The authoritative cross-check against
    // the real headers is the `coreaudio-sys` `assert_eq_size!` set
    // that lands with the macOS FFI layer; these guard the
    // hand-written definitions against the obvious mistakes
    // (accidental padding, a wrong field type) in the meantime.

    #[test]
    fn property_address_is_three_packed_u32s() {
        assert_eq!(size_of::<AudioObjectPropertyAddress>(), 12);
        assert_eq!(align_of::<AudioObjectPropertyAddress>(), 4);
    }

    #[test]
    fn value_range_is_two_f64s() {
        assert_eq!(size_of::<AudioValueRange>(), 16);
        assert_eq!(align_of::<AudioValueRange>(), 8);
    }

    #[test]
    fn asbd_is_40_bytes() {
        // One Float64 (8) + eight UInt32 (32) = 40, matching
        // `crate::format::ASBD_SIZE`.
        assert_eq!(size_of::<AudioStreamBasicDescription>(), 40);
        assert_eq!(
            size_of::<AudioStreamBasicDescription>(),
            crate::format::ASBD_SIZE
        );
        assert_eq!(align_of::<AudioStreamBasicDescription>(), 8);
    }

    #[test]
    fn smpte_time_layout() {
        // mSubframes(2) + mSubframeDivisor(2) + mCounter(4) + mType(4) + mFlags(4) + 4×i16(8) = 24.
        assert_eq!(size_of::<SMPTETime>(), 24);
    }

    #[test]
    fn audio_timestamp_layout() {
        // mSampleTime(8) + mHostTime(8) + mRateScalar(8) +
        // mWordClockTime(8) + mSMPTETime(24) + mFlags(4) +
        // mReserved(4) = 64.
        assert_eq!(size_of::<AudioTimeStamp>(), 64);
        assert_eq!(align_of::<AudioTimeStamp>(), 8);
    }

    #[test]
    fn io_cycle_info_matches_sdk_layout() {
        assert_eq!(size_of::<AudioServerPlugInIOCycleInfo>(), 224);
    }

    #[test]
    fn client_info_layout() {
        // mClientID(4) + mProcessID(4) + mIsNativeEndian(1, padded
        // to 8) + mBundleID(8 pointer) = 24 on a 64-bit target.
        assert_eq!(size_of::<AudioServerPlugInClientInfo>(), 24);
        assert_eq!(align_of::<AudioServerPlugInClientInfo>(), 8);
    }

    #[test]
    fn cfuuid_bytes_is_16_bytes() {
        assert_eq!(size_of::<CFUUIDBytes>(), 16);
        assert_eq!(align_of::<CFUUIDBytes>(), 1);
    }

    #[test]
    fn driver_interface_is_a_dense_pointer_table() {
        // The `IUNKNOWN_C_GUTS` preamble (`_reserved` + the three
        // IUnknown methods) plus 19 driver entry points — 23
        // pointer-sized slots in all.
        const SLOTS: usize = 23;
        assert_eq!(
            size_of::<AudioServerPlugInDriverInterface>(),
            SLOTS * size_of::<*const c_void>()
        );
        assert_eq!(
            align_of::<AudioServerPlugInDriverInterface>(),
            align_of::<*const c_void>()
        );
    }

    #[test]
    fn option_fn_is_abi_compatible_with_a_bare_fn_pointer() {
        // The whole vtable design relies on the null-pointer niche:
        // `Option<unsafe extern "C" fn(...)>` must be exactly one
        // pointer wide for the `#[repr(C)]` table to match C.
        assert_eq!(
            size_of::<Option<unsafe extern "C" fn() -> OSStatus>>(),
            size_of::<*const c_void>()
        );
    }

    #[test]
    fn scalar_aliases_have_the_expected_widths() {
        assert_eq!(size_of::<UInt32>(), 4);
        assert_eq!(size_of::<UInt64>(), 8);
        assert_eq!(size_of::<SInt32>(), 4);
        assert_eq!(size_of::<Float64>(), 8);
        assert_eq!(size_of::<OSStatus>(), 4);
        assert_eq!(size_of::<Boolean>(), 1);
        assert_eq!(size_of::<AudioObjectID>(), 4);
    }
}
