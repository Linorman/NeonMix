English | [日本語](ja/testing.md)

# Testing and CI

This document describes the testing and continuous-integration strategy
for `tympan-aspl`, including what can be verified automatically on
GitHub-hosted runners, what requires manual or self-hosted execution,
and the constraints imposed by macOS itself.

## Tiered verification strategy

Verification is organised in four tiers by depth and environment
requirements. Each tier subsumes the previous one. Lower tiers run on
every pull request; higher tiers run on schedule or on demand.

### Tier 1: Static and unit verification

Standard Rust toolchain checks runnable on any macOS-hosted runner.

| Check | Command | Purpose |
|---|---|---|
| Build | `cargo build --all-targets` | Compilation across all crate features |
| Test | `cargo test` | Unit tests for logic that does not require HAL |
| Lint | `cargo clippy --all-targets -- -D warnings` | Including project-specific realtime-safety lints |
| Format | `cargo fmt --check` | Style consistency |
| Doc | `cargo doc --no-deps --document-private-items` | Documentation coverage and rustdoc errors |

Required on every pull request. Total time: 5-10 minutes on `macos-14`
or `macos-15` runners.

### Tier 2: Bundle and ABI verification

Verify that the built `.driver` bundle has the structural properties
required by CoreAudio to be a loadable AudioServerPlugin.

| Check | Tool | Purpose |
|---|---|---|
| Bundle layout | `plutil -lint Info.plist` | Info.plist syntax and required keys |
| Symbol visibility | `nm -gU` | Only the entry-point symbol is exported |
| Architecture coverage | `lipo -info` | Universal binary or arm64-only as configured |
| Code signing (ad-hoc) | `codesign -v` | Ad-hoc signature applied during build |
| ABI sizes | Compile-time `static_assertions` | Bridged struct sizes match the C definitions |

The ABI size check uses `static_assertions::assert_eq_size!` against
the `coreaudio-sys`-generated types, catching layout drift before
runtime.

Runs on every pull request once a buildable `.driver` example exists.

### Tier 3: HAL load verification

Place the built `.driver` bundle into the HAL plugin directory and
verify that `coreaudiod` enumerates it.

Sequence:

1. `sudo cp -R target/release/example.driver /Library/Audio/Plug-Ins/HAL/`
2. `sudo launchctl kill KILL system/com.apple.audio.coreaudiod`
3. Wait briefly for `coreaudiod` to relaunch automatically
4. read `coreaudiod`'s unified log and confirm it discovers and
   attempts to load the plug-in

GitHub-hosted runners support steps 1–4: `sudo` is passwordless,
and `coreaudiod` does scan the HAL directory and try to load the
bundle.

**They do not, however, run the plug-in to completion.** On
macOS 15 the out-of-process Core Audio Driver Service helper
enforces code-signature validity, and macOS AMFI rejects an
ad-hoc-signed plug-in binary with `AppleMobileFileIntegrityError
-423` before any plug-in code executes. A GitHub-hosted runner
cannot produce a Developer ID signature, so hosted Tier 3 stops at
"the load was attempted". Confirming the device enumerates
(`system_profiler SPAudioDataType | grep <device-name>`) and
exercising the IO path require a Developer ID-signed bundle and are
Tier 4 checks.

Runs on every merge to `main` and on a daily schedule.

### Tier 4: Audio I/O verification

Actual audio data flow through the plugin. Out of scope for standard
GitHub-hosted runners because the runners lack physical or virtual
audio hardware exposed to applications. Performed via:

- Developer's local machine during PR review
- A self-hosted runner on a developer Mac registered with GitHub
  Actions
- macOS-in-cloud services (MacStadium, MacinCloud, AWS mac.metal)
  when funded

## GitHub-hosted macOS runners

Current runners (as of April 2026):

| Label | OS | Architecture | Specs | Public-repo cost |
|---|---|---|---|---|
| `macos-15` / `macos-latest` | Sequoia | Apple Silicon (M1) | 3 vCPU, 7 GB RAM, 14 GB disk | Free |
| `macos-14` | Sonoma | Apple Silicon (M1) | Same | Free |
| `macos-13` | Ventura | Intel x86_64 | Same | Free |
| `macos-14-xlarge` / `macos-15-xlarge` | Sequoia / Sonoma | M2 Pro | 5 vCPU, 14 GB RAM | Paid only ($0.16/min) |

Public repositories receive unlimited free minutes on standard runners
across all GitHub plans. For private forks or downstream consumers,
macOS runners cost $0.048/min after the January 2026 price reduction;
the free monthly quota of 2,000 Linux-equivalent minutes converts to
~200 macOS minutes per month on private repositories.

`tympan-aspl` is a public repository, so standard runner usage is
unconstrained by cost.

### Runner image inventory

GitHub publishes per-image software inventories for each runner. The
images relevant to AudioServerPlugin development include:

- Xcode Command Line Tools and full Xcode installations
- `clang`, `lipo`, `codesign`, `plutil`, `nm`, `otool`, `dyld_info`
- `system_profiler`, `launchctl`, `audio_*` introspection utilities

No additional Apple Developer Program enrolment is required for
ad-hoc signing.

## System Integrity Protection (SIP) and AMFI considerations

GitHub-hosted Apple-silicon runners run in a VM and ship with **SIP
disabled** — `csrutil status` reports "disabled", and the boot log
shows `AMFI: Booted in a VM`. This is *not* a useful lever: the
code-signing gate that stops an unsigned HAL plug-in is **AMFI**,
which `amfid` enforces independently of SIP. With SIP off the runner
still rejects an ad-hoc-signed plug-in with
`AppleMobileFileIntegrityError -423`.

| Operation | Allowed | Notes |
|---|---|---|
| Place `.driver` in `/Library/Audio/Plug-Ins/HAL/` | Yes | Requires `sudo`, which runners provide |
| Restart `coreaudiod` | Yes | Via `launchctl kill` |
| `coreaudiod` discovers / attempts the plug-in | Yes | It scans the HAL directory and parses the bundle |
| `coreaudiod` loads an **ad-hoc-signed** plug-in to completion | **No** | `amfid` rejects it (`AppleMobileFileIntegrityError -423`); SIP being off does not change this |
| `coreaudiod` loads a **self-signed** plug-in to completion | **No** | Rejected the same way — even with the certificate installed as a trusted System-keychain root, since `amfid` does not consult that trust store |
| `coreaudiod` loads a **Developer ID-signed** plug-in | Yes | CI can only do this with the certificate supplied via a GitHub secret |
| Write `nvram boot-args` (the AMFI `amfi_get_out_of_my_way` knob) | Write succeeds | SIP is off, so the write is accepted — but it only takes effect after a reboot, which a hosted runner cannot perform mid-job |
| Modify SIP itself | N/A | Already disabled on the runner |

Key observation: **AMFI prevents *completing* the load of any
plug-in whose signature does not chain to an Apple-issued
certificate.** On macOS 15 the out-of-process Core Audio Driver
Service helper hands the binary to `amfid`, which rejects an ad-hoc
*or* self-signed signature with `AppleMobileFileIntegrityError -423`
("the file is adhoc signed or signed by an unknown certificate
chain"). `coreaudiod` still discovers the bundle and *attempts* the
load — which is what hosted Tier 3 verifies — but the device does
not enumerate.

This was confirmed empirically on a `macos-15` runner: ad-hoc
signing, a self-signed certificate, that certificate installed as a
trusted root in the System keychain, and `DisableLibraryValidation`
were all tried — alone and combined — and every case stopped at
`-423`. (`security add-trusted-cert` satisfies `codesign --verify`
and `spctl`, but `amfid` logs `taskgated-helper: ... no eligible
provisioning profiles found` and still refuses: its trust path is
Apple-issued certificates, not the System keychain.) Running a
plug-in to completion therefore needs a Developer ID-signed bundle —
supplied to CI via a GitHub secret — and is a Tier 4 check until
that secret exists. (This corrects the project's original survey,
which assumed unsigned HAL plug-ins load freely: true for older
models, not for the macOS 15 driver-service helper, and not
unlocked by disabling SIP.)

## What cannot be verified on GitHub-hosted runners

Hard limits of the standard runner environment:

- **Audio output to physical speakers** — runners have no audio output
  hardware exposed to applications
- **Microphone capture** — runners have no input devices
- **Long-running stability** — jobs time out at 6 hours; realistic
  stability tests run for days or weeks
- **Notarization-gated behaviour** — notarization requires an Apple
  Developer Program account and submission to Apple's servers; CI
  cannot meaningfully perform this on every commit
- **Third-party audio application interaction** — DAWs, conferencing
  apps, and similar tools are not installed and cannot be reliably
  driven headlessly
- **System Settings UI verification** — confirming that a device
  appears in System Settings > Sound requires a logged-in UI session,
  which runners do not provide reliably

These gaps motivate the tier 4 manual / self-hosted verification step.

## Self-hosted alternatives

When automated tier 4 verification is required, options include:

### Self-hosted GitHub Actions runner

A developer's Mac registered as a GitHub Actions runner. Cost-effective
for solo development; requires the machine to be powered on and
networked when CI runs.

Public repositories incur no platform fee for self-hosted runners.
Private repositories pay a $0.002/min platform fee starting March 2026.

Steps to register:

1. Settings > Actions > Runners > New self-hosted runner
2. Follow the printed installation script on the target Mac
3. Optionally configure the runner as a launch agent for auto-start

### macOS-in-cloud services

| Service | Model | Approximate cost | When useful |
|---|---|---|---|
| MacStadium | Dedicated Mac mini, monthly | $80-200/mo | Persistent state, full control |
| MacinCloud | Shared and dedicated, hourly | $1-3/hr | Ad-hoc verification |
| AWS EC2 mac.metal | Bare metal, 24-hour minimum | ~$25/day | Brief intensive campaigns |
| Xcode Cloud | Apple's CI for Xcode projects | Included with Developer Program | iOS/macOS app CI; not ideal for libraries |

These services are appropriate when tier 4 verification must run
automatically as part of pipelines, and a local developer machine is
insufficient (e.g., for release validation).

## Workflow files

The `.github/workflows/` layout (`tier1`–`tier3` are present;
`release.yml` is still planned):

```
.github/workflows/
├── tier1.yml           # cargo build/test/clippy/fmt/doc on every PR
├── tier2.yml           # bundle and ABI verification on every PR
├── tier3.yml           # HAL load verification on merge to main + nightly
├── tier3-asan.yml      # in-process harnesses under AddressSanitizer, nightly
└── release.yml         # Tagged release publishing (cargo publish dry-run) — planned
```

Tier 4 is intentionally omitted from the workflow set; it is performed
manually or on a self-hosted runner outside the standard pipeline.

## Realtime safety enforcement in CI

In addition to the lints provided by Clippy, the framework will define
a set of project-specific lints that fail CI when realtime-unsafe
patterns appear in the realtime code paths:

- Allocation calls within functions reachable from `IOProc`
- `std::sync::Mutex` use within realtime modules
- Blocking system calls (e.g., file I/O, network) in realtime code

These lints will be enforced via:

- A custom `cargo clippy` configuration (`clippy.toml`) restricting
  the realtime module's allowed dependencies
- `cargo-deny` rules preventing accidental introduction of
  realtime-unsafe transitive dependencies
- Compile-time `#[deny(...)]` directives in module-level attributes

Implementation details will be added once the first realtime module
lands.

## Implementation status

Tiers 1, 2, and 3 are wired up:

- `tier1.yml` — build, test, `clippy`, `fmt`, `doc`, `cargo deny`,
  and a no-`static mut` grep, on every pull request.
- `tier2.yml` — `plutil -lint` of the committed and generated
  Info.plists, the example `.driver` cdylib build, an `nm` factory-
  symbol check, `lipo -info`, and `.driver` bundle assembly + ad-hoc
  `codesign`, on every pull request.
- `tier3.yml` — `coreaudiod` plug-in-load verification: install the
  `.driver`, restart `coreaudiod`, and confirm from `coreaudiod`'s
  unified log that it discovers and *attempts* to load the plug-in.
  It does not assert device enumeration: AMFI rejects the
  ad-hoc-signed binary on a hosted runner (see the SIP section
  above). Runs on merges to `main`, a daily schedule, and manual
  dispatch — never on a pull request — per ADR 0001.
- `tier3-asan.yml` — the in-process harnesses (`raw_lifecycle`,
  `realtime_safety`) and the `raw`-module unit tests re-run under
  `-Zsanitizer=address` to catch use-after-free / double-free /
  out-of-bounds in the hand-written FFI layer. Nightly and on
  manual dispatch; non-blocking, like `tier3.yml`.

Tier 4 remains a manual, pre-release checklist — and now also owns
the checks the AMFI code-signing constraint pushes out of hosted
CI: loading a Developer ID-signed `.driver` to completion,
confirming `system_profiler` enumerates the device, and exercising
the IO path.
