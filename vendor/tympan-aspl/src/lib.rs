//! `tympan-aspl` — a Rust framework for writing macOS
//! AudioServerPlugins.
//!
//! AudioServerPlugins are the user-space audio drivers that run
//! inside `coreaudiod` and appear to the system as audio devices.
//! `tympan-aspl` provides safe Rust abstractions over the Core Audio
//! HAL AudioServerPlugin C interface so a Rust crate can implement a
//! virtual audio device — a loopback, a virtual microphone, a
//! routing or processing driver — without writing C++ or
//! Objective-C.
//!
//! See `docs/overview.md` for the project's scope and
//! `docs/architecture.md` for the design that drives this crate.
//!
//! ## Layering
//!
//! The crate is built bottom-up in conceptual layers, isolated by
//! module boundary:
//!
//! - [`raw`] — the low-level FFI to the AudioServerPlugin C ABI.
//!   Its ABI type definitions ([`raw::abi`]) and marshalling
//!   ([`raw::marshal`]) are plain data, compiled and tested on every
//!   host; only the live `extern "C"` entry points and
//!   CoreFoundation calls (a later PR) are macOS-gated.
//! - [`realtime`] — allocation-free, lock-free primitives for the
//!   `IOProc` realtime callback. Cross-platform, so the realtime
//!   invariants are unit-testable on any host.
//! - The object model — [`error`], [`fourcc`], [`object`],
//!   [`property`], [`mod@format`], [`io`] — cross-platform
//!   plain-data mirrors of the Core Audio C types.
//! - The property protocol — [`objects`] (the object tree) and
//!   [`dispatch`] (the cross-platform answer to the HAL's property
//!   reads).
//! - The public API — [`driver`], [`device`], [`stream`] — the
//!   traits and declarative types a consumer implements against.
//! - [`bundle`] — `.driver` bundle `Info.plist` generation.
//!
//! ## Realtime safety
//!
//! Any code reachable from the `IOProc` realtime callback must be
//! allocation-free, lock-free, and free of blocking syscalls. The
//! [`realtime`] module exposes a [`RealtimeContext`] marker that
//! acts as a compile-time witness for the realtime context;
//! [`Driver::process_io`] takes one. Mechanical enforcement lives in
//! `tests/realtime_safety.rs` via an `assert_no_alloc`
//! global-allocator guard.
//!
//! ## Implementing a driver
//!
//! A consumer implements [`Driver`] for a type carrying its
//! per-plug-in state, describes the device it exposes with a
//! [`DeviceSpec`], and — on macOS — exposes the type as a CFPlugIn
//! with the [`plugin_entry!`] macro.

// Crate-wide policy for the realtime-safety lints declared in
// `clippy.toml`. The `disallowed-types` and `disallowed-methods`
// lists target items that have no business in a realtime path —
// blocking syncs, kernel syscalls, sleeping primitives — and the
// rest of the crate (initialisation paths, bundle generation,
// tests, non-realtime support code) is free to use them. The
// `realtime` module re-enables the lints at `deny` in its own
// `mod.rs`, so the strict allow-list applies exactly where the
// realtime invariants live.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

pub mod bundle;
pub mod device;
pub mod dispatch;
pub mod driver;
pub mod error;
pub mod format;
pub mod fourcc;
pub mod io;
pub mod object;
pub mod objects;
pub mod property;
pub mod realtime;
pub mod stream;

// The `raw` module's ABI type definitions and marshalling are
// plain data — they compile and are tested on every host. Only its
// live `extern "C"` entry points and CoreFoundation calls (a later
// PR) are `cfg(target_os = "macos")`-gated, from within the module.
pub mod raw;

// `plugin_entry!` is defined on every platform so its documentation
// and intra-doc links resolve in `cargo doc`, but it can only be
// *expanded* on macOS: the factory it emits forwards to the
// `cfg(target_os = "macos")`-gated `raw` module.
mod macros;

pub use bundle::BundleConfig;
pub use device::{Device, DeviceId, DeviceSpec};
pub use dispatch::DeviceState;
pub use driver::{AnyDriver, Driver, DriverInfo};
pub use error::OsStatus;
pub use format::{FormatNegotiation, SampleFormat, StreamFormat};
pub use fourcc::FourCharCode;
pub use io::{IoBuffer, IoOperation, Sample, Timestamp};
pub use object::{AudioObjectId, ObjectKind};
pub use objects::{Object, ObjectMap};
pub use property::{
    PropertyAddress, PropertyElement, PropertyScope, PropertySelector, PropertyValue, ValueRange,
};
pub use realtime::RealtimeContext;
pub use stream::{StreamDirection, StreamSpec};
