# ADR 0004: AudioDriverKit and iOS are out of scope

- Status: Accepted
- Date: 2026-05-14

## Context

`docs/architecture.md` raised it as an open question: how should the
framework interact with **AudioDriverKit** — the DriverKit-based
audio driver model available since macOS 11 — two separate code
paths inside `tympan-aspl`, or a unified API spanning both?
`docs/overview.md` had already listed iOS and DriverKit under "Out
of scope", but without recording *why*.

macOS now has two distinct user-space audio driver mechanisms:

- **AudioServerPlugin** — a CFPlugIn `.driver` bundle loaded into
  the existing `coreaudiod` process. Its ABI is
  `<CoreAudio/AudioServerPlugIn.h>`; it is what `tympan-aspl`
  targets.
- **AudioDriverKit** — a DriverKit extension (`.dext`) that runs in
  its own OS-managed driver-extension process, activated via
  `systemextensionsd`, sandboxed, and separately signed and
  notarised with a managed entitlement.

They share *concepts* (audio objects, properties, IO cycles) but not
ABI, packaging, lifecycle, signing, or process model.

## Decision

`tympan-aspl` targets the `coreaudiod`-hosted **AudioServerPlugin**
model only. AudioDriverKit is **out of scope**. iOS — which has no
AudioServerPlugin model at all — is likewise out of scope.

If AudioDriverKit support is ever pursued, it belongs in a separate
sibling crate, not as a second code path inside this one.

## Consequences

Positive:

- A single, sharp target. The `raw` module can mirror
  `<CoreAudio/AudioServerPlugIn.h>` exactly — its hand-written
  `#[repr(C)]` structs and the CFPlugIn vtable — without bending the
  design to also accommodate DriverKit's `IOUserAudioDevice` class
  hierarchy.
- A "unified API" is avoided. Abstracting over two incompatible
  loading and lifecycle models would leak through the abstraction at
  every packaging, signing, and activation seam, and serve neither
  model well.
- The decision matches the project's established shape: a distinct
  driver model gets a distinct sibling crate, exactly as
  `tympan-apo` (Windows APO) and `tympan-ladspa` (LADSPA) are
  siblings rather than modules of one mega-crate.

Negative:

- A consumer who needs a `.dext` — for instance to ship a driver on
  a configuration where the AudioServerPlugin path is restricted, or
  one backed by real hardware — gets nothing from this crate and
  must look elsewhere or wait for a future sibling.

## Trigger for revisiting

- Apple deprecates or materially restricts the AudioServerPlugin
  model in favour of AudioDriverKit.
- A concrete, funded need for a DriverKit audio crate appears — at
  which point it is scoped as a *new sibling crate*, and this ADR is
  superseded only insofar as it would then point at that crate.

## References

- `docs/overview.md` § Out of scope.
- `docs/architecture.md` open question 4, resolved by this ADR.
- Apple: *Creating an audio device driver* (AudioDriverKit) vs.
  *Creating an Audio Server Driver Plug-in* (AudioServerPlugin) —
  the two distinct models.
