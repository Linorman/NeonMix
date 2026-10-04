English | [日本語](README.ja.md)

# tympan-aspl

A Rust framework for writing macOS AudioServerPlugins.

`tympan-aspl` provides Rust abstractions over the Core Audio HAL
AudioServerPlugin interface, enabling Rust applications to implement
custom virtual audio devices and audio drivers on macOS without
writing C++ or Objective-C.

## Status

**Intended functionality complete.** Every "In scope" item from
[`docs/overview.md`](docs/overview.md) is implemented, and every
non-Tier-4 entry in
[ADR 0001](docs/decisions/0001-ci-verification-strategy.md) is wired
up on CI:

### Framework

- Layer 1 — low-level AudioServerPlugin FFI ([`src/raw/`](src/raw/)):
  the `extern "C"` CFPlugIn factory and IUnknown vtable
  ([`raw::entry`](src/raw/entry.rs), [`raw::vtable`](src/raw/vtable.rs)),
  the C-ABI struct mirrors with compile-time
  `static_assertions` ([`raw::abi`](src/raw/abi.rs)), property
  marshalling ([`raw::marshal`](src/raw/marshal.rs)), and the
  CoreFoundation glue ([`raw::cf`](src/raw/cf.rs),
  [`raw::clock`](src/raw/clock.rs)).
- Layer 2 — realtime primitives ([`src/realtime/`](src/realtime/)):
  the [`RealtimeContext`](src/realtime/context.rs) type-level
  marker, a lock-free SPSC ring ([`realtime::ring`](src/realtime/ring.rs)),
  the atomic AudioServerPlugin lifecycle
  [`State`](src/realtime/state.rs), a wait-free CFPlugIn
  [`Refcount`](src/realtime/refcount.rs), and the off-thread
  [`log`](src/realtime/log.rs) sink.
- Layer 3 — the object / property / dispatch surface
  ([`object`](src/object.rs), [`property`](src/property.rs),
  [`objects`](src/objects.rs), [`dispatch`](src/dispatch.rs)) and
  the public API: [`Driver`](src/driver.rs),
  [`Device`](src/device.rs), [`Stream`](src/stream.rs),
  [`IoBuffer`](src/io.rs), and the
  [`plugin_entry!`](src/macros.rs) declarative macro that exports
  the CFPlugIn factory symbol.
- [`bundle`](src/bundle/) — `.driver` bundle layout and
  `Info.plist` generation.

### Examples

Three reference drivers under [`examples/`](examples/), each a
buildable `.driver` cdylib pinned alloc-free in CI:

- [`minimal-loopback/`](examples/minimal-loopback/) — stateless
  stereo virtual loopback; the smallest viable driver.
- [`gain/`](examples/gain/) — fixed linear gain on the `WriteMix`
  direction; per-instance configuration template.
- [`lowpass/`](examples/lowpass/) — one-pole low-pass with
  per-channel filter memory; per-instance processing state,
  reset in `start_io`.

### CI

| Tier | Checks | Trigger |
|---|---|---|
| 1 | `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo build/test/doc` on `macos-15`, `no static mut in src/` grep, `cargo-deny` supply-chain hygiene | Every PR |
| 2 | `plutil -lint` of committed and generated Info.plists, `nm -gU` factory-symbol audit, `lipo -info` architecture coverage, `.driver` assembly + ad-hoc `codesign --verify` | Every PR |
| 3 | `coreaudiod` HAL load: install the `.driver` into `/Library/Audio/Plug-Ins/HAL/`, restart `coreaudiod`, confirm it discovers and *attempts* to load the plug-in. **Does not** assert device enumeration — AMFI rejects ad-hoc signatures on macos-15 (`AppleMobileFileIntegrityError -423`); a Developer ID signature would lift that, see ADR 0001 § Trigger for revisiting | Merge to `main`, daily, manual |
| 3 ASan | The in-process FFI harnesses (`raw_lifecycle`, `realtime_safety`) and `raw::*` unit tests under `-Zsanitizer=address` on nightly + `-Z build-std` | Daily, manual |
| 3 TSan | The realtime `ring`'s `cross_thread_push_pop_preserves_order` (100k concurrent exchange) and the `realtime::*` unit tests under `-Zsanitizer=thread` | Daily, manual |

Hosted CI cannot run a plug-in to *completion*: `amfid` rejects an
ad-hoc-signed HAL bundle on macOS 15 before any plug-in code runs,
and the runners produce no hardware audio I/O. Device-enumeration
and audio-data-path checks therefore live in
[`docs/testing.md`](docs/testing.md) Tier 4 as a pre-release
manual / self-hosted checklist. The framework's `raw_lifecycle`
in-process harness drives the actual `AudioServerPlugInDriverInterface`
vtable end-to-end on every PR, so the FFI layer is exercised
mechanically — just not by `coreaudiod` itself.

The framework is usable: write `impl Driver for MyDriver` and
`tympan_aspl::plugin_entry!(MyDriver)` in a `cdylib` crate and the
resulting `.driver` bundle loads in `coreaudiod` once Developer ID
signed. See [`examples/minimal-loopback/`](examples/minimal-loopback/)
for the minimal recipe and
[`docs/plugin-author-guide.md`](docs/plugin-author-guide.md) for the
collected practical recipes (identity, packaging, realtime
debugging, common pitfalls).

### Future work

The path to full Tier 4 verification is documented in
[ADR 0001 § Trigger for revisiting](docs/decisions/0001-ci-verification-strategy.md).
The next step is wiring a Developer ID signing identity (held as a
GitHub secret) into Tier 3 — at which point device enumeration
moves from Tier 4 back into hosted CI without a code change here.
Audio-data-path I/O verification stays Tier 4 (the runners have no
audio hardware) and requires a self-hosted runner or local
machine.

A `release.yml` workflow that runs `cargo publish --dry-run` on tag
push is planned; tracked separately.

## Naming

*Tympan* — the tympanal organ of moths, a membrane-based ultrasound sensor
on the abdomen of pyralid and noctuid moths. Evolved to detect the
echolocation calls of bats. The name reflects the library's role: a thin
membrane between the OS audio engine and user-space Rust code.

## Quickstart for driver authors

Add a new `cdylib` crate that depends on this framework, implement
[`Driver`](src/driver.rs), and invoke
[`plugin_entry!`](src/macros.rs):

```toml
# Cargo.toml
[package]
name = "my-driver"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
tympan-aspl = "0.1"
```

```rust
// src/lib.rs
use tympan_aspl::{
    plugin_entry, DeviceSpec, Driver, IoBuffer, RealtimeContext,
    StreamFormat, StreamSpec,
};

pub struct MyDriver;

impl Driver for MyDriver {
    const NAME: &'static str = "My Driver";
    const MANUFACTURER: &'static str = "your name";
    const VERSION: &'static str = "0.1.0";

    fn new() -> Self {
        Self
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(48_000.0, 2);
        DeviceSpec::new("com.example.mydriver", "My Driver", Self::MANUFACTURER)
            .with_sample_rate(48_000.0)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        // Allocation-free, lock-free path.
        let n = buffer.output.len().min(buffer.input.len());
        buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        buffer.output[n..].fill(0.0);
    }
}

plugin_entry!(MyDriver);
```

Build with `cargo build --release`, assemble the `.driver` bundle —
the committed `Info.plist` plus the built cdylib placed in
`Contents/MacOS/` — and install it under
`/Library/Audio/Plug-Ins/HAL/`. For local development on a
SIP-disabled developer machine, an ad-hoc signature is enough to
get the bundle on disk; on production macOS 15 the `coreaudiod`
helper requires the bundle to be Developer ID signed before it
will load to completion. See
[`docs/plugin-author-guide.md`](docs/plugin-author-guide.md) for
the packaging recipe and
[`examples/minimal-loopback/`](examples/minimal-loopback/) for the
same shape as a complete crate.

## Development

The project's CI tiers (described above) are split across
[`.github/workflows/tier1.yml`](.github/workflows/tier1.yml),
[`tier2.yml`](.github/workflows/tier2.yml),
[`tier3.yml`](.github/workflows/tier3.yml),
[`tier3-asan.yml`](.github/workflows/tier3-asan.yml), and
[`tier3-tsan.yml`](.github/workflows/tier3-tsan.yml);
[ADR 0001](docs/decisions/0001-ci-verification-strategy.md) records
the tiered verification strategy.

To run the same fmt and clippy checks locally before every `git push`,
opt into the repository's pre-push hook:

```sh
git config core.hooksPath .githooks
```

The hook lives in [`.githooks/pre-push`](.githooks/pre-push). It is
a no-op when no `*.rs`, `Cargo.toml`, or `Cargo.lock` files changed
in the pushed range, so documentation-only pushes are not slowed
down. Bypass it for a single push with `git push --no-verify`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

## Documentation

| Doc | Content |
|---|---|
| [`docs/overview.md`](docs/overview.md) | Project purpose, scope, comparison to existing implementations |
| [`docs/architecture.md`](docs/architecture.md) | API design and module layout |
| [`docs/plugin-author-guide.md`](docs/plugin-author-guide.md) | Writing a driver: setup, identity, the realtime path, packaging, pitfalls |
| [`docs/testing.md`](docs/testing.md) | Testing and CI strategy across tiers |
| [`docs/references.md`](docs/references.md) | Apple documentation, prior art, related crates |
| [`docs/decisions/`](docs/decisions/) | Architectural Decision Records |
| [`docs/handoff-protocol.md`](docs/handoff-protocol.md) | Session handoff protocol for long-running work |

## Examples

| Example | Description |
|---|---|
| [`examples/minimal-loopback/`](examples/minimal-loopback/) | Stateless stereo virtual loopback. Smallest viable consumer of the framework. Pinned alloc-free in CI. |
| [`examples/gain/`](examples/gain/) | Fixed linear gain on the `WriteMix` direction. Demonstrates per-instance configuration and `IoOperation` branching so the gain applies exactly once on the loopback round-trip. |
| [`examples/lowpass/`](examples/lowpass/) | One-pole low-pass with per-channel filter memory. Demonstrates per-instance processing state, reset in `start_io`, and a multiply-add realtime recurrence. |
