//! Owned render selection and explicit administrative metadata sync. No routing change.
#![allow(unsafe_code)]
use windows::{
    Win32::{
        Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE},
        Media::Audio::{IMMDevice, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, eRender},
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            STGM_READ, STGM_READWRITE,
            StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToStringAlloc},
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
    core::{GUID, Interface, PCWSTR},
};

// This immutable package marker is published in EP\0 by NeonMixAudio.inx.
const OWNER_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x1a50c7b0_b766_4e50_9df8_3f49b6583bd1),
    pid: 2,
};
const NAME_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
const OWNER_VALUE: &str = "com.neonmix.audio.virtual-output";

struct Com(bool);
impl Com {
    fn enter() -> Result<Self, String> {
        // SAFETY: control-thread COM setup; balance only initialization performed here.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            Ok(Self(true))
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self(false))
        } else {
            Err(format!("COM initialization failed: {status:?}"))
        }
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: this thread successfully initialized COM in enter.
            unsafe {
                CoUninitialize();
            }
        }
    }
}
struct Value(PROPVARIANT);
impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: owns a property-store returned PROPVARIANT exactly once.
        let _ = unsafe { PropVariantClear(&mut self.0) };
    }
}
fn text(store: &IPropertyStore, key: &PROPERTYKEY) -> Result<String, String> {
    // SAFETY: live store and correctly typed property key; returned variant is owned here.
    let value = Value(unsafe { store.GetValue(key) }.map_err(|e| e.to_string())?);
    // SAFETY: conversion allocates a COM string, released below on success or decode error.
    let pointer = unsafe { PropVariantToStringAlloc(&value.0) }.map_err(|e| e.to_string())?;
    // SAFETY: returned pointer is a valid NUL-terminated UTF16 string.
    let result = unsafe { pointer.to_string() }.map_err(|e| e.to_string());
    // SAFETY: string was allocated by PropVariantToStringAlloc with COM allocation.
    unsafe {
        windows::Win32::System::Com::CoTaskMemFree(Some(pointer.0.cast()));
    }
    result
}
fn owned_device(id: &str) -> Result<IMMDevice, String> {
    let id = id
        .strip_prefix("wasapi:")
        .filter(|id| !id.is_empty() && !id.contains('\0'))
        .ok_or("invalid WASAPI device ID")?;
    let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
    // SAFETY: COM is initialized on this control thread and the endpoint ID buffer
    // lives through GetDevice. Only a render endpoint with our marker is accepted.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDevice(PCWSTR(wide.as_ptr()))
            .map_err(|e| e.to_string())?;
        let endpoint: IMMEndpoint = device.cast().map_err(|e| e.to_string())?;
        if endpoint.GetDataFlow().map_err(|e| e.to_string())? != eRender {
            return Err("virtual output must be a render endpoint".into());
        }
        let store = device
            .OpenPropertyStore(STGM_READ)
            .map_err(|e| e.to_string())?;
        if text(&store, &OWNER_KEY)? != OWNER_VALUE {
            return Err("endpoint is not owned by the NeonMix render package".into());
        }
        Ok(device)
    }
}
pub fn is_virtual_output(id: &str) -> bool {
    let Ok(_com) = Com::enter() else {
        return false;
    };
    owned_device(id).is_ok()
}
pub fn set_virtual_output_name(id: &str, name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("invalid virtual output name".into());
    }
    let _com = Com::enter()?;
    let device = owned_device(id)?;
    // SAFETY: the device belongs to our package. Read-only checking is available
    // to ordinary Sender users; requesting a write is an explicit sync-name action.
    unsafe {
        let current = device
            .OpenPropertyStore(STGM_READ)
            .map_err(|e| e.to_string())?;
        if text(&current, &NAME_KEY)? == name {
            return Ok(());
        }
        let writable = device.OpenPropertyStore(STGM_READWRITE).map_err(|e| {
            format!("native name sync requires administrative endpoint-property access: {e}")
        })?;
        // VT_LPWSTR references this live buffer; SetValue copies it. This borrowed
        // variant is not cleared, because it does not own the Vec's allocation.
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut value = PROPVARIANT::default();
        let fields = &mut *value.Anonymous.Anonymous;
        fields.vt = windows::Win32::System::Variant::VT_LPWSTR;
        fields.Anonymous.pwszVal = windows::core::PWSTR(wide.as_ptr().cast_mut());
        writable
            .SetValue(&NAME_KEY, &value)
            .map_err(|e| e.to_string())?;
        writable.Commit().map_err(|e| e.to_string())?;
        if text(
            &device
                .OpenPropertyStore(STGM_READ)
                .map_err(|e| e.to_string())?,
            &NAME_KEY,
        )? != name
        {
            return Err("native output name readback differs from request".into());
        }
        Ok(())
    }
}
