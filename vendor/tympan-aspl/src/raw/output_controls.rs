//! NeonMix local extension: master output volume/Mute objects and legacy device aliases.
use super::{
    abi::{AudioObjectPropertyAddress, AudioServerPlugInHostRef},
    runtime::DriverRuntime,
};
use crate::{AudioObjectId, OsStatus, PropertyAddress, PropertyScope, PropertyValue, ValueRange};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

const DEVICE: u32 = 2;
const VOLUME: u32 = 5;
const MUTE: u32 = 6;
const MIN_DB: f32 = -96.0;
const fn code(value: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*value)
}

pub(crate) struct OutputControls {
    scalar: AtomicU32,
    muted: AtomicBool,
    name: Mutex<String>,
}

impl OutputControls {
    pub(crate) fn new(name: &str) -> Self {
        Self {
            scalar: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
            name: Mutex::new(name.into()),
        }
    }
    fn scalar(&self) -> f32 {
        f32::from_bits(self.scalar.load(Ordering::Acquire))
    }
    pub(crate) fn gain(&self) -> f32 {
        if self.muted.load(Ordering::Acquire) {
            0.0
        } else {
            self.scalar()
        }
    }
}

fn scalar_to_db(value: f32) -> f32 {
    if value <= 0.0 {
        MIN_DB
    } else {
        (20.0 * value.log10()).max(MIN_DB)
    }
}
fn db_to_scalar(value: f32) -> f32 {
    if value <= MIN_DB {
        0.0
    } else {
        10.0_f32.powf(value / 20.0)
    }
}

pub(crate) fn get(
    runtime: &DriverRuntime,
    id: u32,
    address: &PropertyAddress,
    convert: Option<f32>,
) -> Option<Result<PropertyValue, OsStatus>> {
    if runtime.objects().spec().output().is_none() {
        return None;
    }
    let selector = address.selector.0.as_u32();
    let control = runtime.output_controls();
    let global = address.scope == PropertyScope::GLOBAL;
    let output = address.scope == PropertyScope::OUTPUT;
    let main = address.element.is_main();
    let value = if id == DEVICE && main {
        match selector {
            v if v == code(b"lnam") && global => PropertyValue::Text(
                control
                    .name
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone(),
            ),
            v if v == code(b"ctrl") => PropertyValue::ObjectList(if global || output {
                vec![AudioObjectId(VOLUME), AudioObjectId(MUTE)]
            } else {
                vec![]
            }),
            v if v == code(b"ownd") => {
                let mut ids = runtime.objects().device_streams(address.scope);
                if global || output {
                    ids.extend([AudioObjectId(VOLUME), AudioObjectId(MUTE)]);
                }
                PropertyValue::ObjectList(ids)
            }
            v if v == code(b"volm") && output => PropertyValue::F32(control.scalar()),
            v if v == code(b"vold") && output => PropertyValue::F32(scalar_to_db(control.scalar())),
            v if v == code(b"mute") && output => {
                PropertyValue::U32(u32::from(control.muted.load(Ordering::Acquire)))
            }
            _ => return None,
        }
    } else if (id == VOLUME || id == MUTE) && global && main {
        match selector {
            v if v == code(b"bcls") => PropertyValue::U32(if id == VOLUME {
                code(b"levl")
            } else {
                code(b"togl")
            }),
            v if v == code(b"clas") => PropertyValue::U32(if id == VOLUME {
                code(b"vlme")
            } else {
                code(b"mute")
            }),
            v if v == code(b"stdv") => PropertyValue::ObjectId(AudioObjectId(DEVICE)),
            v if v == code(b"lnam") => PropertyValue::Text(
                if id == VOLUME {
                    "Output volume"
                } else {
                    "Output mute"
                }
                .into(),
            ),
            v if v == code(b"ownd") => PropertyValue::ObjectList(vec![]),
            v if v == code(b"cscp") => PropertyValue::U32(code(b"outp")),
            v if v == code(b"celm") => PropertyValue::U32(0),
            v if v == code(b"lcsv") && id == VOLUME => PropertyValue::F32(control.scalar()),
            v if v == code(b"lcdv") && id == VOLUME => {
                PropertyValue::F32(scalar_to_db(control.scalar()))
            }
            v if v == code(b"lcdr") && id == VOLUME => {
                PropertyValue::RangeList(vec![ValueRange::new(f64::from(MIN_DB), 0.0)])
            }
            v if (v == code(b"lcsd") || v == code(b"lcds")) && id == VOLUME => {
                let input = convert.unwrap_or(0.0);
                if !input.is_finite() {
                    return Some(Err(OsStatus::ILLEGAL_OPERATION));
                }
                PropertyValue::F32(if v == code(b"lcsd") {
                    scalar_to_db(input.clamp(0.0, 1.0))
                } else {
                    db_to_scalar(input.clamp(MIN_DB, 0.0))
                })
            }
            v if v == code(b"bcvl") && id == MUTE => {
                PropertyValue::U32(u32::from(control.muted.load(Ordering::Acquire)))
            }
            _ => return Some(Err(OsStatus::UNKNOWN_PROPERTY)),
        }
    } else {
        return None;
    };
    Some(Ok(value))
}

pub(crate) fn settable(
    runtime: &DriverRuntime,
    id: u32,
    address: &PropertyAddress,
) -> Option<Result<bool, OsStatus>> {
    get(runtime, id, address, None).map(|value| {
        value.map(|_| {
            let selector = address.selector.0.as_u32();
            (id == DEVICE
                && [code(b"volm"), code(b"vold"), code(b"mute"), code(b"lnam")].contains(&selector))
                || (id == VOLUME && [code(b"lcsv"), code(b"lcdv")].contains(&selector))
                || (id == MUTE && selector == code(b"bcvl"))
        })
    })
}

pub(crate) fn set(
    runtime: &DriverRuntime,
    id: u32,
    address: &PropertyAddress,
    data: &[u8],
) -> Option<Result<(), OsStatus>> {
    let writable = settable(runtime, id, address)?;
    Some((|| {
        if !writable? {
            return Err(OsStatus::ILLEGAL_OPERATION);
        }
        let bytes: [u8; 4] = data.try_into().map_err(|_| OsStatus::BAD_PROPERTY_SIZE)?;
        let selector = address.selector.0.as_u32();
        let mute = selector == code(b"mute") || selector == code(b"bcvl");
        if mute {
            runtime
                .output_controls()
                .muted
                .store(u32::from_ne_bytes(bytes) != 0, Ordering::Release);
        } else {
            let value = f32::from_ne_bytes(bytes);
            let decibel = selector == code(b"vold") || selector == code(b"lcdv");
            if !value.is_finite()
                || if decibel {
                    !(MIN_DB..=0.0).contains(&value)
                } else {
                    !(0.0..=1.0).contains(&value)
                }
            {
                return Err(OsStatus::ILLEGAL_OPERATION);
            }
            let scalar = if decibel { db_to_scalar(value) } else { value };
            runtime
                .output_controls()
                .scalar
                .store(scalar.to_bits(), Ordering::Release);
        }
        notify(runtime, mute);
        Ok(())
    })())
}

// The SDK's host interface starts with PropertiesChanged. Only that prefix is needed.
#[repr(C)]
struct HostPrefix {
    changed: Option<
        unsafe extern "C" fn(
            AudioServerPlugInHostRef,
            u32,
            u32,
            *const AudioObjectPropertyAddress,
        ) -> i32,
    >,
}

pub(crate) fn notify_running(runtime: &DriverRuntime) {
    let host = runtime.host();
    if host.is_null() {
        return;
    }
    // SAFETY: the live SDK host interface begins with PropertiesChanged.
    let Some(callback) = (unsafe { &*host.cast::<HostPrefix>() }).changed else {
        return;
    };
    let address = AudioObjectPropertyAddress {
        mSelector: code(b"goin"),
        mScope: code(b"glob"),
        mElement: 0,
    };
    // SAFETY: notification runs off realtime, after all framework list/state locks are released.
    unsafe {
        callback(host, DEVICE, 1, &address);
    }
}

pub(crate) fn set_name(runtime: &DriverRuntime, value: String) {
    *runtime
        .output_controls()
        .name
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = value;
    let host = runtime.host();
    if host.is_null() {
        return;
    }
    // SAFETY: Initialize supplies the stable SDK host interface; prefix matches PropertiesChanged.
    let Some(callback) = (unsafe { &*host.cast::<HostPrefix>() }).changed else {
        return;
    };
    let address = AudioObjectPropertyAddress {
        mSelector: code(b"lnam"),
        mScope: code(b"glob"),
        mElement: 0,
    };
    // SAFETY: control-thread notification with live host and correctly sized address.
    unsafe {
        callback(host, DEVICE, 1, &address);
    }
}

fn notify(runtime: &DriverRuntime, mute: bool) {
    let host = runtime.host();
    if host.is_null() {
        return;
    }
    // SAFETY: Initialize's host interface remains live for the plug-in lifetime; its first
    // field matches AudioServerPlugInHostInterface.PropertiesChanged in the macOS SDK.
    let Some(callback) = (unsafe { &*host.cast::<HostPrefix>() }).changed else {
        return;
    };
    let selectors = if mute {
        vec![code(b"bcvl")]
    } else {
        vec![code(b"lcsv"), code(b"lcdv")]
    };
    let addresses: Vec<_> = selectors
        .into_iter()
        .map(|selector| AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: code(b"glob"),
            mElement: 0,
        })
        .collect();
    let device_selectors = if mute {
        vec![code(b"mute")]
    } else {
        vec![code(b"volm"), code(b"vold")]
    };
    let aliases: Vec<_> = device_selectors
        .into_iter()
        .map(|selector| AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: code(b"outp"),
            mElement: 0,
        })
        .collect();
    // SAFETY: control-thread-only calls; both slices and the host outlive each notification.
    unsafe {
        callback(
            host,
            if mute { MUTE } else { VOLUME },
            addresses.len() as u32,
            addresses.as_ptr(),
        );
        callback(host, DEVICE, aliases.len() as u32, aliases.as_ptr());
    }
}
