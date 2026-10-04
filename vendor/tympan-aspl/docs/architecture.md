English | [日本語](ja/architecture.md)

# Architecture

This document describes the planned architecture. Implementation has not
begun. Details may change as design feedback accumulates.

## Module layout

```
tympan-aspl/
├── src/
│   ├── lib.rs            # Re-exports; public API surface
│   ├── driver.rs         # Driver trait, plug-in entry point
│   ├── device.rs         # Device abstraction
│   ├── stream.rs         # Stream abstraction
│   ├── property.rs       # Property registry and dispatch
│   ├── io.rs             # IOProc registration; high-level wrapper
│   ├── raw/              # Low-level: vtables, C ABI bridges, FFI
│   │   ├── mod.rs
│   │   ├── vtable.rs     # CFPlugIn IUnknown vtable construction
│   │   ├── selectors.rs  # Property selector constants
│   │   └── host.rs       # Host interface (coreaudiod's side)
│   ├── realtime/         # Realtime-safe primitives
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext marker type
│   │   ├── ring.rs       # Lock-free SPSC ring buffer
│   │   └── state.rs      # Atomic state machine helpers
│   └── bundle/           # .driver bundle packaging helpers
│       ├── mod.rs
│       └── plist.rs      # Info.plist generation
├── examples/
│   └── minimal-loopback/  # Minimal virtual loopback device
└── tests/
    └── ...               # Integration tests where feasible
```

## Layer model

Three conceptual layers, isolated by module boundary:

### Layer 1: `raw` — unsafe FFI

- Sole owner of `unsafe extern "C"` declarations
- Sole consumer of `coreaudio-sys` and `core-foundation-sys`
- Maps CFPlugIn IUnknown vtables to Rust function pointers
- Provides raw type aliases (e.g., `RawObjectID = AudioObjectID`)

Users of `tympan-aspl` should not need to touch this module. It exists for
the framework's internal use and for advanced users who need to bypass
the higher-level abstractions.

### Layer 2: `realtime` — zero-allocation primitives

- No allocator usage
- No `std::sync::Mutex`, no `std::collections::HashMap`
- Lock-free SPSC / MPMC ring buffers (built on `crossbeam-utils`)
- Atomic state machines for driver lifecycle
- A `RealtimeContext` zero-sized marker that:
  - Is required as a parameter for any function safe to call from `IOProc`
  - Cannot be constructed outside the framework
  - Acts as a compile-time witness of realtime safety

This layer's invariant: any function reachable from `IOProc` must accept
`&RealtimeContext` and contain no heap operations.

### Layer 3: Public API — safe, idiomatic

- `Driver`, `Device`, `Stream` traits
- Builder-pattern constructors
- Result types for fallible operations
- Lifetime-bounded references to plug-in state

This is the layer 95% of users will interact with.

## Core abstractions

### `Driver`

The top-level trait implemented by consumers. Corresponds to the
AudioServerPlugIn object.

```text
trait Driver {
    fn devices(&self) -> &[Device];
    fn create_device(&mut self, spec: DeviceSpec) -> Result<DeviceId>;
    fn on_property_changed(&mut self, address: PropertyAddress);
    // …
}
```

The framework provides the plug-in entry point as a macro:

```text
tympan_aspl::plugin_entry!(MyDriver);
```

This expands to the `#[no_mangle] extern "C" fn` plug-in factory that
`coreaudiod` calls.

### `Device`

A single audio device exposed by the driver. Has one or more streams
(input / output).

```text
struct Device {
    uid: Uid,
    streams: Vec<Stream>,
    // …
}
```

Property dispatch is handled by the framework. Custom properties can be
registered via the `Property` trait.

### `Stream`

A single direction of audio flow on a device. Supplies samples for input
streams; receives samples for output streams.

The I/O entry point is the user's `process_io` method, called on the
realtime thread with a `RealtimeContext`:

```text
fn process_io(
    &mut self,
    rt: &RealtimeContext,
    timestamp: AudioTimeStamp,
    input: &[Sample],
    output: &mut [Sample],
);
```

### `RealtimeContext`

The central type-level guarantee. Instances are passed by reference from
the framework's I/O harness to user code. They have no fields and no
way to be constructed from user code.

Functions that allocate, lock, or call system services do not take
`&RealtimeContext`. This produces compile errors when realtime-unsafe code
is called from `process_io`.

## Cross-cutting concerns

### CoreFoundation interop

The AudioServerPlugin API uses `CFStringRef`, `CFArrayRef`, etc. for
property values. `tympan-aspl` wraps these in safe Rust types via
`core-foundation` (not `core-foundation-sys` directly):

- `CFString` <-> `&str` / `String` conversions
- Lifetime-bounded `&CFArray<T>` views
- Automatic `CFRetain` / `CFRelease` via Rust drop semantics

### Property protocol

CoreAudio property dispatch is the lion's share of AudioServerPlugin
boilerplate. The framework handles standard properties (sample rate,
format, latency, volume, mute, name, etc.) automatically. Custom
properties can be added via:

```text
impl Property for MyCustomProperty {
    const SELECTOR: PropertySelector = /* … */;
    type Value = f64;
    fn read(&self, ctx: &PropertyContext) -> Self::Value { /* … */ }
    fn write(&mut self, ctx: &PropertyContext, val: Self::Value) -> Result<()> { /* … */ }
}
```

### Logging

Realtime code cannot log via `tracing` or `log` (both allocate).
The `realtime` module provides a lock-free log queue for capturing diagnostic
events from `IOProc`. A separate non-realtime thread drains the queue and
forwards entries to the standard `log` crate.

## Open questions

Most of the questions carried into implementation have since been
settled and recorded as ADRs under `docs/decisions/`; the code's
provisional answers are now the documented ones:

- [x] Should `Driver` be an object-safe trait or a generic
  parameter? — generic, erased once to `dyn AnyDriver` at the FFI
  boundary; see
  [ADR 0002](decisions/0002-driver-trait-type-erasure.md).
- [x] How granular should the `RealtimeContext` distinction be? — a
  single marker; see
  [ADR 0003](decisions/0003-single-realtime-context.md).
- [ ] What is the minimum macOS version supported? AudioServerPlugin
  matured significantly in macOS 10.10 and again in 12.0. Still
  open — CI runs on `macos-15` and no lower bound has been committed
  to.
- [x] How does the framework interact with AudioDriverKit
  (DriverKit-based drivers in macOS 11+)? — out of scope; see
  [ADR 0004](decisions/0004-audiodriverkit-out-of-scope.md).
