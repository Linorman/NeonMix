# ADR 0001: CI verification strategy and scope boundary

- Status: Accepted
- Date: 2026-05-14

## Context

`tympan-aspl` is a macOS-only framework: the `raw` module links
`CoreAudio` / `CoreFoundation` and the `.driver` bundle is loaded by
`coreaudiod`. `CLAUDE.md` lists five explicit prohibitions, the
strongest being an allocation-free, lock-free `IOProc` path. The
question is which of these can be verified mechanically on
GitHub-hosted runners, and which must fall back to local or
self-hosted testing.

`docs/testing.md` already surveyed the runner landscape and the
System Integrity Protection (SIP) constraints. Findings relevant to
CI scoping:

- **GitHub-hosted `macos-15` / `macos-14` runners** ship Xcode and
  the Command Line Tools out of the box: `clang`, `lipo`,
  `codesign`, `plutil`, `nm`, `otool`, `system_profiler`,
  `launchctl`. Public repositories receive unlimited free minutes on
  standard runners.
- **SIP is disabled on GitHub-hosted Apple-silicon runners** (they
  run in a VM), but that does not unlock loading an unsigned HAL
  plug-in: the code-signing gate is AMFI, which `amfid` enforces
  independently of SIP. See the Tier 3 finding below — the project's
  original survey was wrong on this point.
- **The cross-platform layers compile and test on any host.** The
  `realtime`, `error`, `format`, `property`, and `bundle` modules
  contain no `cfg(target_os = "macos")` code, so their invariants
  (lock-free ring buffer, atomic state machine, Info.plist
  generation) are unit-testable on the macOS runner without a HAL
  round-trip.
- **Audio I/O verification needs hardware** the runners do not
  expose, so it stays manual.

Industry baseline in adjacent Rust audio projects (`coreaudio-rs`,
`cpal`): CI typically stops at `cargo build` and `cargo test` with no
plug-in lifecycle exercising. Adopting bundle/ABI verification and a
`coreaudiod` HAL-load harness puts this project above that baseline,
on par with the sibling `tympan-apo` and `tympan-ladspa` crates.

## Decision

CI verification is organised in four tiers. Each tier defines what
runs on which trigger and what is intentionally out of scope. The
operational details (commands, runner labels, fixtures) live in
[`docs/testing.md`](../testing.md); the tier boundaries below are
authoritative. CI is built up incrementally as implementation lands —
this ADR describes the target state, and the `.github/workflows/`
directory tracks how much of it is wired up so far.

### Tier 1 — `static` (every PR push, target < 7 min)

- `cargo build --release --all-targets`
- `cargo test --all-targets` for the cross-platform unit tests
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- `cargo doc --no-deps --document-private-items` with
  `RUSTDOCFLAGS=-D warnings`
- A `git grep` for `static mut` in `src/` enforcing the
  no-global-state invariant
- `cargo deny` for supply-chain hygiene

This tier blocks merge. **Wired up: yes** (`tier1.yml`).

### Tier 2 — `bundle` (every PR push, target < 10 min)

Tier 1 plus:

- `plutil -lint` over committed `Info.plist` fixtures and over
  Info.plists emitted by the `bundle` module
- `nm -gU` symbol-visibility check — the CFPlugIn factory symbol is
  exported, unmangled
- `lipo -info` architecture-coverage check
- `codesign` ad-hoc signature + `codesign --verify` over the
  assembled `.driver` bundle

The bridged struct layouts are checked for internal consistency by
the `static_assertions` in `raw::abi`, and proven end-to-end
against the real Core Audio C ABI by Tier 3's HAL load (a plug-in
with a wrong vtable or struct layout does not enumerate). A
`coreaudio-sys` `assert_eq_size!` cross-check was considered;
`coreaudio-sys`'s coverage of `<CoreAudio/AudioServerPlugIn.h>` is
not relied upon, and the Tier 3 round-trip is the stronger check.

This tier blocks merge. **Wired up: yes** (`tier2.yml`).

### Tier 3 — `plugin-load` (merge to `main` and nightly, target < 25 min)

Tier 2 plus:

- `sudo cp -R` the built `.driver` into `/Library/Audio/Plug-Ins/HAL/`
- `sudo launchctl kill` `coreaudiod` and wait for relaunch
- read `coreaudiod`'s unified log and confirm it **discovers and
  attempts to load** the plug-in (`HALS_RemotePlugInRegistrar`
  "Attempting to load" + the Core Audio Driver Service helper
  "Loading server plug-in com.tympan.aspl.MinimalLoopback")

That proves the `.driver` layout, the `Info.plist`, and the CFPlugIn
registration are well-formed enough for the HAL to parse and try.

**Finding (2026-05-14): the load does not complete on a hosted
runner.** The first Tier 3 run showed `coreaudiod` reaching the
bundle and then macOS AMFI rejecting the plug-in binary —
`AppleMobileFileIntegrityError -423`, "the file is adhoc signed or
signed by an unknown certificate chain". macOS 15's out-of-process
Core Audio Driver Service helper enforces code-signature validity;
an ad-hoc signature is not accepted, and a GitHub-hosted runner
cannot produce a Developer ID signature. The original survey's
premise — that an unsigned HAL plug-in loads freely and SIP is the
only relevant gate — was wrong: AMFI is the gate, and it rejects
ad-hoc signatures regardless of SIP state. So Tier 3 on hosted CI
asserts the *load attempt*, not device enumeration; confirming the
device actually appears, and exercising the IO path, moves to Tier 4.

**Follow-up (2026-05-14): no runner-applicable bypass exists.** A
dedicated experiment on a `macos-15` runner tried ad-hoc signing, a
self-signed certificate, that certificate installed as a trusted
root in the System keychain, and `DisableLibraryValidation` — alone
and combined. Every case stopped at `amfid` `-423`; the device
never enumerated. SIP is in fact already *disabled* on the runner
and that changes nothing. `security add-trusted-cert` satisfies
`codesign --verify` and `spctl`, but `amfid` does not consult the
System keychain — it logs `taskgated-helper: ... no eligible
provisioning profiles found` and still refuses. The `nvram`
`boot-args` AMFI knob is writable (SIP is off) but needs a reboot
the hosted runner cannot perform mid-job. The only way to exercise
a full load on hosted CI is a Developer ID-signed bundle supplied
via a GitHub secret.

Two in-process lifecycle harnesses cover the IO path under an
`assert_no_alloc` global-allocator guard, on every PR in Tier 1,
without depending on the code-signing constraint:
`tests/realtime_safety.rs` drives the safe `DriverInstance` API, and
`tests/raw_lifecycle.rs` drives the framework through its actual
`AudioServerPlugInDriverInterface` vtable — the factory, the entry
points, and the `DoIOOperation` data path `coreaudiod` would call.

An AddressSanitizer companion, `tier3-asan.yml`, re-runs those two
harnesses and the `raw`-module unit tests under
`-Zsanitizer=address` on a nightly schedule. `coreaudiod`'s own
HAL-load cannot be sanitized — it is a separate process — but the
in-process FFI exercise can be, and ASan is where a use-after-free,
double-free, or out-of-bounds access in the hand-written `raw`
layer's `Box` / refcount / ring lifecycle would surface. It shares
Tier 3's non-blocking, scheduled discipline. **Wired up: yes**
(`tier3-asan.yml`).

This tier does not block PR merge — `tier3.yml` runs on merges to
`main`, a daily schedule, and manual dispatch, never on a pull
request. A failure on `main` is a signal to investigate.
**Wired up: yes** (`tier3.yml`).

### Tier 4 — out of CI scope

Not tested on GitHub-hosted runners; documented in `docs/testing.md`
(§ Tier 4) as a pre-release manual checklist. It now also covers the
checks the AMFI code-signing constraint pushes out of hosted CI:
loading a Developer ID-signed `.driver` to completion, confirming
`system_profiler` enumerates the device, and exercising the IO
path. Plus the pre-existing Tier 4 items — audio output to physical
hardware, microphone capture, long-running stability,
notarization-gated behaviour, third-party application interaction,
and System Settings UI verification.

## Consequences

Positive:

- The cross-platform invariants (realtime safety, state machine
  correctness, Info.plist well-formedness) are mechanically enforced
  on every PR from the very first commit, before any FFI lands.
- The tier numbering maps cleanly onto the sibling tympan crates'
  conventions, so contributors moving between crates see consistent
  CI semantics.
- Tier 3's plug-in-load harness catches the most common
  bundle-level regression modes (Info.plist drift, CFPlugIn factory
  wiring, bundle-identifier mismatch) on `main` and nightly without
  any macOS infrastructure beyond a hosted runner.

Negative:

- Hosted CI cannot run a HAL plug-in to completion: macOS AMFI
  rejects the ad-hoc signature, so the device-enumeration and
  IO-path checks land in Tier 4, against a Developer ID-signed
  bundle. The hand-written `raw` ABI layer is therefore *exercised*
  only at Tier 4 — Tiers 1–3 prove it compiles, is internally
  consistent, and is structurally well-formed, but not that
  `coreaudiod` drives it correctly.
- Audio-data-path bugs (format negotiation, sample-rate conversion,
  glitching under load) are not caught until manual verification.
- Notarization-related issues surface only at release-time signing.

## Trigger for revisiting

Re-evaluate when any of the following holds:

- A self-hosted Mac runner with a virtual audio device becomes part
  of the project's CI budget — at that point Tier 4 audio-I/O
  verification is promoted into a workflow.
- A Developer ID certificate is added to the repository's GitHub
  secrets — at that point the device-enumeration check can move from
  Tier 4 back into hosted Tier 3, signing the `.driver` with the
  secret instead of an ad-hoc signature.
- A bug ships that would have been caught by `coreaudiod`-level
  loading but was not caught by the Tier 3 harness.
- Apple ships a first-party AudioServerPlugin test host; if that
  lands, evaluate whether it replaces or supplements the in-process
  lifecycle harness.

## References

- Apple: *Creating an Audio Server Driver Plug-in*
  - <https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in>
- GitHub-hosted macOS runner image inventory
  - <https://github.com/actions/runner-images/tree/main/images/macos>
- `assert_no_alloc` crate — <https://docs.rs/assert_no_alloc>
- Sibling tympan CI strategies:
  - `tympan-apo` ADR 0001 (Windows, in-process COM activation)
  - `tympan-ladspa` ADR 0005 (LADSPA, `applyplugin`-based Tier 2)
- `docs/testing.md` — tiered verification strategy and runner notes
