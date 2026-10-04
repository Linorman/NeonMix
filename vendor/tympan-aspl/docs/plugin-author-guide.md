English | [日本語](ja/plugin-author-guide.md)

# Plugin Author Guide

This guide collects the non-obvious bits of writing a macOS
AudioServerPlugin on top of `tympan-aspl` — the things scattered
across module docstrings, ADRs, and example READMEs, pulled into one
place. For an end-to-end working starting point use
[`examples/minimal-loopback/`](../examples/minimal-loopback/) (the
smallest viable driver); for per-instance configuration and
direction-aware processing see [`examples/gain/`](../examples/gain/);
for genuine per-instance processing state see
[`examples/lowpass/`](../examples/lowpass/).

## Project setup

Your driver is a crate that depends on this framework and builds as
a `cdylib` — the loadable binary inside the `.driver` bundle. Keep
`rlib` in the crate type too so your `Driver` implementation stays
unit-testable on any host. The minimum `Cargo.toml`:

```toml
[package]
name = "my-driver"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"

[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
tympan-aspl = { git = "https://github.com/penta2himajin/tympan-aspl" }

[profile.release]
panic = "abort"      # see § Panic strategy below
lto = "thin"         # smaller .dylib, marginal codegen win
codegen-units = 1
```

The framework crate has one small dependency of its own
(`crossbeam-utils`), so adding it keeps your dependency tree
shallow.

## Panic strategy

`coreaudiod` is C code, and a Rust `panic!()` that unwinds across
the C ABI boundary is **undefined behaviour**.

The framework's `extern "C"` entry points for the *lifecycle*
methods — `Initialize`, `StartIO`, `StopIO` — wrap your code in
[`catch_unwind`]: a panic out of `Driver::initialize`,
`Driver::start_io`, or `Driver::stop_io` is caught and reported to
the HAL as `OsStatus::UNSPECIFIED`, never unwound across the ABI.

**`Driver::process_io` is not wrapped.** It runs on the realtime IO
thread, where a `catch_unwind` on every cycle is unacceptable
overhead — so a panic in `process_io` *will* unwind across the C
ABI. Therefore:

1. Set `panic = "abort"` in your release profile. A panic anywhere
   in the driver then becomes an immediate process abort — no
   unwinding, no UB across the ABI, and a smaller binary.
2. Never `panic!` in `process_io`. `assert!`, `unwrap`, integer
   overflow in debug builds, slice indexing out of bounds — all
   panic. Use explicit bounds checks (every example clamps with
   `buffer.output.len().min(buffer.input.len())`), and the
   [`LogSink`](#diagnostic-events-logsink) pattern to record a
   diagnostic rather than crashing.
3. Failures in `Driver::initialize` / `Driver::start_io` are
   first-class: return `Err(OsStatus::...)` and the framework
   reports it to the HAL without a panic.

[`catch_unwind`]: https://doc.rust-lang.org/std/panic/fn.catch_unwind.html

## Driver identity: the device UID and the factory UUID

Two identifiers are part of your driver's stable ABI. Changing
either after release breaks things silently.

- **The device UID** — the first argument to `DeviceSpec::new`, also
  the bundle's `CFBundleIdentifier`. macOS keeps per-device user
  settings (chosen sample rate, volume, whether it is the default
  device) keyed on this string. Change it and every saved setting
  for the device is orphaned. Use reverse-DNS, e.g.
  `com.example.MyDriver`.
- **The factory UUID** — the key of the `Info.plist`'s
  `CFPlugInFactories` dictionary, and the value in its
  `CFPlugInTypes` array. It must be unique to your driver; generate
  a fresh UUID once (`uuidgen`) and never change it. The examples
  expose theirs as a `FACTORY_UUID` constant feeding
  `BundleConfig`.

Both the `NAME` / `MANUFACTURER` / `VERSION` associated constants of
the `Driver` trait are free to change between releases — they are
display strings, not identity.

## Symbol visibility

The framework's [`plugin_entry!`](../src/macros.rs) macro emits
`#[no_mangle] pub unsafe extern "C" fn TympanAsplDriverFactory` (the
name is overridable with a second macro argument). That is the only
symbol `coreaudiod` resolves — it is named in the bundle
`Info.plist`'s `CFPlugInFactories` dictionary, so the macro argument
and the `BundleConfig` factory-function argument must agree.

A Rust `cdylib` exports `#[no_mangle] extern "C"` items but not
plain Rust public items, so nothing else leaks out of the `.dylib`.
Tier 2 CI asserts the factory symbol is present and unmangled:

```sh
nm -gU target/release/libmy_driver.dylib | grep -E ' _TympanAsplDriverFactory$'
```

Invoke `plugin_entry!` exactly once, at your crate root — it emits a
`#[no_mangle]` symbol, which must be unique in the link tree.

## The `.driver` bundle

The loadable artefact `coreaudiod` scans for is a `.driver` bundle —
the committed `Info.plist` plus your built `cdylib`, in the CFBundle
layout:

```text
MyDriver.driver/
└── Contents/
    ├── Info.plist
    └── MacOS/
        └── MyDriver        ← the built cdylib, renamed
```

Do not hand-write the `Info.plist`. Generate it with
`bundle::plist::generate(&config)` from a `BundleConfig`, and commit
the result next to your crate. Pin the committed file to the
generator with a unit test, so the two cannot drift — the `gain` and
`lowpass` examples both carry one:

```rust
#[test]
fn committed_info_plist_matches_the_generator() {
    assert_eq!(
        tympan_aspl::bundle::plist::generate(&MyDriver::bundle_config()),
        include_str!("../Info.plist"),
    );
}
```

## Realtime path discipline

`Driver::process_io` executes on `coreaudiod`'s realtime IO thread.
Code reachable from it must not:

1. **Allocate.** Pre-allocate every buffer in `Driver::start_io`
   (allocation there is allowed) or `Driver::new`. The
   `lowpass` example carries its filter memory as a fixed-size
   array and resets it in `start_io`.
2. **Take a `std::sync::Mutex`.** Use atomics, the framework's
   [lock-free SPSC ring](../src/realtime/ring.rs), or per-instance
   state owned exclusively by the driver (`process_io` takes
   `&mut self`).
3. **Make blocking syscalls** — no `println!`, no file I/O, no
   `std::process`. For diagnostics use
   [`LogSink`](#diagnostic-events-logsink).
4. **Spawn or join threads.** `initialize` and `start_io` are fine
   for that; `process_io` is not.

The `&RealtimeContext` argument to `process_io` is a compile-time
witness of this: framework functions that allocate or block do not
accept `&RealtimeContext`, so calling one from `process_io` is a
type error. It cannot catch *everything* (a bare `Vec::new()` still
compiles), which is why CI also enforces the invariant
mechanically: Tier 1 runs `tests/realtime_safety.rs` and
`tests/raw_lifecycle.rs`, both of which drive the IO path inside an
`assert_no_alloc` global-allocator guard that aborts on any
allocation. Mirror that pattern in your own crate to extend the
guarantee — it is ~40 lines of test scaffolding.

## Direction-aware processing

A loopback device — one with both an input and an output stream —
has its `process_io` called for **two** IO operations: `WriteMix`
(audio entering the device from a client) and `ReadInput` (audio
leaving it to a client). A transform applied unconditionally is
therefore applied *twice* across the loopback.

Branch on `IoBuffer::operation` to apply your processing exactly
once. The convention the examples follow is to process on `WriteMix`
— "the device transforms audio on the way in" — and pass through on
`ReadInput`:

```rust
fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
    let n = buffer.output.len().min(buffer.input.len());
    match buffer.operation {
        IoOperation::WRITE_MIX => { /* your DSP, input → output */ }
        _ => buffer.output[..n].copy_from_slice(&buffer.input[..n]),
    }
    buffer.output[n..].fill(0.0);
}
```

See [`examples/gain/`](../examples/gain/) and
[`examples/lowpass/`](../examples/lowpass/) for the full pattern.

## Diagnostic events: `LogSink`

When you need to emit a diagnostic from `process_io` — "IO cycle
larger than expected", "value clamped", "unexpected operation" — push
it into a [`LogSink`](../src/realtime/log.rs). `LogSink::log` is a
single lock-free try-push: two atomic ops plus a `T`-sized memcpy, no
allocation and no syscall. A small `Copy` enum is the standard event
shape:

```rust
#[derive(Debug, Clone, Copy)]
enum MyLogEvent {
    CycleClamped { requested: u32 },
}
```

Construct the sink in `Driver::initialize` (it allocates and spawns
the off-thread drainer there) and let it drop when the driver object
is released. The drainer runs your `drain_one` closure off the
realtime thread; if that closure panics the drainer thread ends but
`process_io` keeps working — a failing log sink never crashes audio.

## Installing for the HAL

`coreaudiod` scans `/Library/Audio/Plug-Ins/HAL/` for `.driver`
bundles. After building:

```sh
cargo build --release -p my-driver

BUNDLE=MyDriver.driver
mkdir -p "$BUNDLE/Contents/MacOS"
cp Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/libmy_driver.dylib "$BUNDLE/Contents/MacOS/MyDriver"
codesign --force --sign - "$BUNDLE"

sudo cp -R "$BUNDLE" /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

**Code signing is not optional on macOS 15.** An ad-hoc signature
(`--sign -`) is enough for `coreaudiod` to *discover and attempt* the
bundle, but the out-of-process Core Audio Driver Service helper
rejects it at the AMFI layer (`AppleMobileFileIntegrityError -423`)
before the device enumerates. For the device to actually appear in
*System Settings ▸ Sound* you need a **Developer ID Application**
signature. See [`docs/testing.md`](testing.md) § "System Integrity
Protection and AMFI considerations" for the full constraint and what
it means for CI.

## Common pitfalls

| Pitfall | Symptom | Fix |
|---|---|---|
| Missing `crate-type = ["cdylib"]` | No `lib*.dylib` produced; nothing to put in the bundle | Add `[lib] crate-type = ["rlib", "cdylib"]`. |
| `Info.plist` factory UUID ≠ `BundleConfig` factory name target | `coreaudiod` finds the bundle but cannot instantiate it | Generate the plist from the same `BundleConfig`; pin it with the `committed_info_plist_matches_the_generator` test. |
| Applying a transform unconditionally in `process_io` | Effect applied twice across a loopback (e.g. gain²) | Branch on `IoBuffer::operation`; process on `WriteMix` only. |
| `println!` / `dbg!` / `unwrap` in `process_io` | `assert_no_alloc` test aborts, or UB / audio glitches | Use `LogSink`; replace `unwrap` with explicit handling. |
| `panic!` reachable from `process_io` without `panic = "abort"` | Undefined behaviour — unwinding across the C ABI | Set `panic = "abort"`; never panic in the IO path. |
| Allocating in `process_io` | `assert_no_alloc` harness aborts | Pre-allocate in `start_io` or `new`; carry it in the struct. |
| Changing the device UID between releases | Saved per-device settings orphaned | Pick the UID once; change `NAME` if you must signal a difference. |
| Spawning a thread in `process_io` | Audio dropouts | Spawn in `initialize` (e.g. via `LogSink::new`); never from `process_io`. |

## Where to read further

- [`docs/overview.md`](overview.md) — project scope and goals.
- [`docs/architecture.md`](architecture.md) — internal module layout
  and the layer model.
- [`docs/testing.md`](testing.md) — the tiered CI strategy and the
  macOS code-signing constraints.
- [`docs/decisions/`](decisions/) — ADRs covering each significant
  framework decision.
- The example crates under [`examples/`](../examples/) — full
  working drivers for the patterns described here.
