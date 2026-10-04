# ADR 0002: The `Driver` trait is generic, erased once at the FFI boundary

- Status: Accepted
- Date: 2026-05-14

## Context

`docs/architecture.md` raised it as an open question: should the
user-facing `Driver` trait be **object-safe** (so the framework can
hold a `dyn Driver`) or a **generic parameter** (so the framework
monomorphises over the concrete type)?

The constraint that forces the issue: a `.driver` bundle is loaded
by `coreaudiod`, which holds the plug-in only as an opaque
`*mut c_void`. The framework *must* store something type-erased
behind that pointer — it can never name the user's concrete type at
the ABI boundary. The question is whether that erasure should leak
into the user-facing trait.

A second pressure: the natural way to express a plug-in's identity
(name, manufacturer, version) is associated `const` items. A trait
with associated `const`s is not object-safe.

## Decision

`Driver` is a **generic, non-object-safe trait** — `Sized + Send`
with associated `const NAME` / `MANUFACTURER` / `VERSION` and the
`new` / `device` / `initialize` / `start_io` / `stop_io` /
`process_io` methods. Users implement it directly on a concrete
type.

The framework wraps each `T: Driver` in `DriverInstance<T>`, a
monomorphised wrapper that owns the user state plus the lifecycle
`StateCell` and the CFPlugIn `Refcount`. Type erasure happens
**exactly once**, at the CFPlugIn boundary: `DriverInstance<T>`
implements the object-safe `AnyDriver` trait, and
`driver_factory_dispatch` stores an `Arc<dyn AnyDriver>`. The HAL
vtable dispatches through `dyn AnyDriver`, never naming `T`.

No `#[derive(Driver)]` proc-macro ships — associated `const`s and a
hand-written `impl` are enough (the same reasoning as
`tympan-ladspa` ADR 0003).

## Consequences

Positive:

- The user-facing API is fully generic and concrete: a driver is a
  plain type with an `impl Driver`, unit-testable directly, with no
  `dyn` in sight. `DriverInstance<T>` and its state machine are
  exercised concretely by `tests/realtime_safety.rs`.
- Associated `const`s express plug-in identity at compile time;
  `DriverInfo::of::<T>()` snapshots them for the `bundle` module and
  the HAL property protocol with no runtime cost.
- Erasure is confined to one well-defined seam. Everything above the
  CFPlugIn factory is generic; everything the HAL touches is
  `dyn AnyDriver`. The two traits have a clear division of labour —
  `Driver` is the user contract, `AnyDriver` is the internal
  vtable-facing surface.

Negative:

- There are two traits to understand instead of one. This is
  mitigated by `AnyDriver` being `pub` only for the framework's own
  `plugin_entry!` macro and advanced users; a typical driver author
  never names it.
- Reaching the user's `process_io` from the realtime `DoIOOperation`
  entry point is one virtual call through `dyn AnyDriver`. It is a
  single predictable indirection per IO operation — the user's own
  `process_io` body is still fully monomorphised and inlinable — but
  it is not literally zero-cost.

## Trigger for revisiting

- A realtime entry path appears that cannot tolerate the one virtual
  call at the `AnyDriver` boundary (profiling would have to show it
  matters — unlikely at audio-cycle granularity).
- The `Driver` / `AnyDriver` split becomes a genuine maintenance
  burden — e.g. every new method has to be threaded through both by
  hand and the duplication causes recurring bugs.

## References

- `docs/architecture.md` § `Driver`, and open question 1, resolved
  by this ADR.
- `src/driver.rs` — `Driver`, `AnyDriver`, `DriverInstance<T>`,
  `DriverInfo`.
- `tympan-ladspa` ADR 0003 — the parallel "trait impl, no derive
  macro" decision.
