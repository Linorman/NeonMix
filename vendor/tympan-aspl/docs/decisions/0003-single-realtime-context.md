# ADR 0003: A single `RealtimeContext` marker, not a family

- Status: Accepted
- Date: 2026-05-14

## Context

`docs/architecture.md` raised it as an open question: how granular
should the realtime-witness type be? One `RealtimeContext` marker,
or a family — `PropertyContext`, `BoundaryContext`, and so on — each
witnessing a different kind of callback context?

The witness pattern itself is settled: a zero-sized type that user
code cannot construct, handed to user code by reference from the
framework's harness, and required as a parameter by any function
safe to call from the realtime thread. Functions that allocate or
block simply do not take one, so they cannot be called from a
context that only holds a witness. The open question was *how many*
such types there should be.

## Decision

There is **one** witness type: `RealtimeContext`. No
`PropertyContext` / `BoundaryContext` family.

It is zero-sized, `!Send + !Sync` (a `PhantomData<*const ()>`
field), and constructed only inside the crate via the `unsafe`
`new_unchecked`.

## Consequences

The decision follows from where realtime code actually meets user
code in this framework:

- **`Driver::process_io` is the only realtime entry point into user
  code.** It is driven by `DoIOOperation` on the Core Audio realtime
  thread, and that is the single place the framework mints a
  `RealtimeContext` and hands it out.
- **The property protocol is not realtime.** `HasProperty`,
  `GetPropertyData`, `SetPropertyData`, and the device-client and
  configuration-change callbacks all run on the HAL's serialised
  non-realtime thread. They need no witness — a `PropertyContext`
  would witness a *non*-realtime context, which is the absence of a
  constraint, not a constraint worth a type.
- **`BeginIOOperation` / `EndIOOperation` do run on the realtime
  thread**, but the framework services them itself as no-ops; no
  user code runs there, so no witness is handed out.

So there is exactly one realtime user surface, and one marker covers
it. A family of context types would be ceremony with nothing to
distinguish.

Positive:

- Minimal API surface — one concept for a driver author to learn,
  and an unambiguous one: holding a `&RealtimeContext` *means* the
  call originated from `process_io`.
- `!Send + !Sync` stops a witness from being smuggled to another
  thread, where the realtime assumption would no longer hold.

Negative:

- If a future realtime entry path into user code appears with
  materially different constraints, the single marker cannot
  distinguish it from the `process_io` context. Splitting the type
  at that point is a small, mostly-additive change — but it is a
  change.

## Trigger for revisiting

A second realtime entry path into user code lands — for example a
realtime-thread property-change notification — with constraints
that differ from `process_io`'s. At that point introduce a distinct
witness for it rather than overloading `RealtimeContext`.

## References

- `docs/architecture.md` § `RealtimeContext`, and open question 2,
  resolved by this ADR.
- `src/realtime/context.rs` — the marker and its `new_unchecked`
  contract.
- `CLAUDE.md` prohibitions 1 and 2 — the realtime constraints the
  witness makes visible.
