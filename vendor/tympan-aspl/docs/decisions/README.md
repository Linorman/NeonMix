# Architecture Decision Records

This directory contains the project's settled architectural decisions.

## Why ADRs

`CLAUDE.md` lists ADRs as a top-tier source of truth. When the same
question recurs across sessions or PRs, the answer belongs here so it
does not get rewritten each time.

## Lifecycle

- ADRs are numbered sequentially. Filenames follow
  `NNNN-kebab-case-summary.md`.
- An ADR is **Accepted** when merged. Status may later move to
  **Superseded by ADR NNNN** if a follow-up decision overturns it.
  The superseded record stays in the tree for historical context — it
  is not deleted.
- ADRs are short. A single page of context, decision, consequences,
  and (where useful) a documented reversal trigger.

## Index

| ID | Title | Status |
|---:|---|---|
| [0001](0001-ci-verification-strategy.md) | CI verification strategy and scope boundary | Accepted |
| [0002](0002-driver-trait-type-erasure.md) | The `Driver` trait is generic, erased once at the FFI boundary | Accepted |
| [0003](0003-single-realtime-context.md) | A single `RealtimeContext` marker, not a family | Accepted |
| [0004](0004-audiodriverkit-out-of-scope.md) | AudioDriverKit and iOS are out of scope | Accepted |
| [0005](0005-i18n-policy.md) | Internationalisation policy and file layout | Accepted |
