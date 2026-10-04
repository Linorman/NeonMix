# tympan-aspl

## Overview

Rust framework for writing macOS AudioServerPlugins. The library provides
safe abstractions over the Core Audio HAL AudioServerPlugin interface so that
Rust applications can implement custom virtual audio devices without using
C++ or Objective-C.

Detailed design lives under @docs/overview.md and @docs/architecture.md.

## Project Structure

```
src/                     # Public API (high-level, safe)
src/raw/                 # Low-level FFI to AudioServerPlugin.h
src/realtime/            # Realtime-safe primitives (lock-free, alloc-free)
src/bundle/              # .driver bundle layout and Info.plist generation
examples/                # Reference plugins (e.g. minimal-loopback)
tests/                   # Integration tests
docs/                    # Design, architecture, decisions, references
.github/                 # CI workflows, issue/PR templates
```

## Development Setup

Required toolchain:

- Rust 1.80+ (matches `coreaudio-sys` minimum requirement)
- Xcode Command Line Tools (macOS SDK and `clang`)
- macOS host (cross-compilation from Linux/Windows is not supported initially)

Optional:

- Apple Developer Program membership for code signing and Notarization of
  consuming plugins. Not needed for library development itself.

## Build & Test

```bash
cargo build
cargo test
```

For manual testing on a macOS host, install an example plugin into the HAL
plug-in directory and reset `coreaudiod`:

```bash
sudo cp -R target/release/example.driver /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

Verification: the plugin should appear in System Settings > Sound.

## Development Principles

- **Realtime safety is non-negotiable.** The `IOProc` callback runs on a
  realtime audio thread. Code in the I/O path must be allocation-free,
  lock-free, and free of system calls. Use `realtime` module primitives.
- **Wrap the C ABI carefully.** AudioServerPlugin uses CFPlugIn IUnknown-style
  vtables. The `raw` module exposes these as-is; higher layers add Rust
  idioms (RAII, lifetime-bounded references).
- **Match Apple's semantics, not Apple's naming.** Property addresses,
  selectors, and qualifiers retain their original semantics, but APIs use
  Rust-natural names (e.g., `process_io` not `IOProcessing`).
- **No global state.** AudioServerPlugin instances are first-class objects;
  the library never relies on `static mut` or singletons.

## Architectural Boundaries

- `raw` module is the only place that links to CoreFoundation and CoreAudio.
- `realtime` module never allocates and never returns `Result` values
  containing `String` or other heap types. Errors are represented as
  `OSStatus` codes.
- Public API surface lives in `lib.rs` and re-exports from internal modules.
- `examples/` plugins must be buildable as `.driver` bundles; non-bundle
  examples belong in `tests/` or as doc-tests.

## Prohibitions

1. Do not allocate memory in any function called from `IOProc` or its
   transitive callees. Pre-allocate buffers during initialization.
2. Do not call `std::sync::Mutex::lock()` from realtime code paths. Use
   lock-free primitives (`crossbeam`, atomics) instead.
3. Do not introduce dependencies on async runtimes (`tokio`, `async-std`).
   This is a sync, realtime-oriented library.
4. Do not depend on external C libraries beyond what macOS provides
   (CoreFoundation, CoreAudio, AudioToolbox).
5. Do not expose `unsafe fn` in the public API without a clearly documented
   safety contract. Internal `unsafe` is encapsulated behind safe wrappers.

## Git Conventions

- Scoped Conventional Commits: `feat(raw):`, `fix(realtime):`, `docs(arch):`.
- Scopes follow the module structure: `raw`, `realtime`, `api`, `examples`,
  `docs`, `meta` (CI, README, license).
- Breaking changes use `!` notation and require a corresponding entry in
  `docs/decisions/` (when that directory exists).
- PRs link a handoff issue with `Closes #N` or `Refs #N`.

## Session Handoff

Long-running workstreams use GitHub issues for cross-session continuity.
See @docs/handoff-protocol.md for the full protocol.

- Label: `session-handoff`
- One issue per workstream (not per session)
- On session start, read the relevant handoff issue and confirm the
  **Next action** with the user before executing.
