English | [日本語](ja/overview.md)

# Overview

## Purpose

`tympan-aspl` is a Rust framework for implementing macOS
**AudioServerPlugins** — the user-space audio drivers that run inside
`coreaudiod` and appear to the system as audio devices.

The goal is to enable Rust applications to:

- Implement virtual audio devices (input and output)
- Implement custom audio routing or processing drivers
- Build noise-suppression, voice-effect, or audio-bridging plugins

… without writing the C++ or Objective-C code that has historically been
required.

## Why this exists

The AudioServerPlugin API is documented in C headers
(`<CoreAudio/AudioServerPlugIn.h>`). There is no first-party Rust binding
or framework. Existing options for AudioServerPlugin development are:

| Approach | Language | Maturity | Trade-off |
|---|---|---|---|
| Apple sample code (`SimpleAudioDriver`) | C++ | First-party | Reference only; not a framework |
| **libASPL** (gavv/libASPL) | C++17 | Production-grade | C++ only; no Rust path |
| **BlackHole** | C / Objective-C | Production-grade | Specific to loopback; not a framework |
| **BackgroundMusic** | C++ / Objective-C | Production-grade | Application-specific |
| Hand-rolled FFI from Rust | Rust + unsafe FFI | None public | Each user reinvents the wheel |

This framework fills the Rust gap. It is intended to occupy the same niche
as libASPL but for Rust users: a generic, reusable substrate on top of
which application-specific drivers can be built.

## Scope

### In scope

- AudioServerPlugin entry point and CFPlugIn vtable wiring
- Object hierarchy: Driver, Device, Stream, Box, Plug-In
- Property registry and dispatch (CoreAudio property protocol)
- I/O cycle: `StartIO`, `StopIO`, `BeginIOOperation`, `EndIOOperation`,
  `WillDoIOOperation`, the realtime read/write callbacks
- Realtime-safe primitives (lock-free ring buffers, atomic state machines)
- Bundle layout and Info.plist generation helpers for `.driver` packaging
- Example: minimal virtual loopback device

### Out of scope

- Signal-processing algorithms (DSP, ML, codecs) — these belong in consumer
  crates that depend on `tympan-aspl`
- GUI / preference panes
- Application code
- iOS or DriverKit (separate concerns; possibly a future sibling crate)
- Non-macOS audio backends (Linux ALSA/PipeWire, Windows WASAPI/WDM)

## Naming

*Tympan* refers to the tympanal organ — a membrane-based hearing organ on
the abdomen of moths such as those in the family Pyralidae. The organ
evolved as a defence against bat echolocation: it captures ultrasound and
converts vibration into neural signals via attached chordotonal
receptors.

The analogy:

- A tympanal organ sits between the outside world and the moth's nervous
  system, converting one physical domain (air pressure) into another
  (nerve impulses).
- `tympan-aspl` sits between the macOS audio engine and user-space Rust
  code, converting one programming domain (C ABI, vtables, realtime
  callbacks) into another (safe Rust types, ownership, lifetimes).

The second word `aspl` is the conventional abbreviation for
AudioServerPlugin used in Apple sample code and existing libraries
(libASPL).

## Status

**Implemented.** The framework crate is built out across `src/` —
the `raw` FFI layer (CFPlugIn factory, vtable, entry points,
marshalling), the safe object/property/dispatch layers, the
`realtime` primitives, and the `bundle` Info.plist helpers — with a
`minimal-loopback` reference example. The CI tiers in
[`testing.md`](testing.md) (static, bundle/ABI, `coreaudiod`
HAL-load) run on every change.

- API design documented in [`architecture.md`](architecture.md)
- Reference material gathered in [`references.md`](references.md)
- Verification strategy in [`testing.md`](testing.md) and
  [`decisions/0001-ci-verification-strategy.md`](decisions/0001-ci-verification-strategy.md)

## Target audience

- Rust developers building macOS audio applications who need virtual
  devices or custom drivers
- Audio plug-in authors comfortable with realtime constraints
- Researchers prototyping audio processing pipelines that need to integrate
  with macOS at the device level

Not intended for:

- Application-level audio playback (use `cpal`, `rodio`, `coreaudio-rs`)
- DAW plug-in formats (AU, VST3, AAX) — those are different APIs entirely

## Comparison to alternatives

### vs. coreaudio-rs

`coreaudio-rs` provides Rust bindings to the **Core Audio HAL client API**
(reading from / writing to existing devices, querying properties). It does
*not* let you implement a new device that the HAL serves to other clients.
`tympan-aspl` is the complement: implementing the *driver* side.

A full audio application might use both: `tympan-aspl` to expose a virtual
device, and `coreaudio-rs` (or `cpal`) inside the consumer-facing app to
render into it.

### vs. libASPL

`libASPL` (C++17) is the reference framework for AudioServerPlugin
development. It is well-architected and battle-tested. `tympan-aspl` aims
to provide equivalent capabilities to Rust users, with the following
differences:

- Rust ownership model for resource lifetime (vs. C++ `shared_ptr` and
  manual ref-counting via CFRetain/CFRelease)
- Realtime-path guarantees enforced at the type level (a `RealtimeContext`
  marker prevents accidental heap allocation in I/O callbacks)
- API names follow Rust conventions (`snake_case`, `Result` types where
  fallibility is meaningful)
- No attempt at C ABI compatibility with libASPL

### vs. hand-rolled FFI

Any Rust developer can call AudioServerPlugin directly via `coreaudio-sys`
(which has the raw bindings). The result is hundreds of lines of `unsafe`
FFI per driver. `tympan-aspl` centralises that boilerplate and provides
safe defaults.
