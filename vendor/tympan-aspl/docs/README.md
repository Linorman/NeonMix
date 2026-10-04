English | [日本語](ja/README.md)

# tympan-aspl documentation

The `tympan-aspl` design and contributor documentation. The
[crate root README](../README.md) is the project entry point; this
directory is the longer-form material.

## Prose documents

| Doc | Content |
|---|---|
| [`overview.md`](overview.md) | Project purpose, scope, and comparison to existing AudioServerPlugin implementations |
| [`architecture.md`](architecture.md) | The layer model, the core abstractions, and the remaining open questions |
| [`plugin-author-guide.md`](plugin-author-guide.md) | Writing a driver: setup, identity, the realtime path, packaging, and pitfalls |
| [`testing.md`](testing.md) | The tiered CI strategy, the GitHub-hosted runner constraints, and the SIP / AMFI code-signing findings |
| [`references.md`](references.md) | Apple documentation, prior art, and related crates |
| [`handoff-protocol.md`](handoff-protocol.md) | The session-handoff protocol for long-running work |

## Architecture Decision Records

[`decisions/`](decisions/) holds the project's settled architectural
decisions, indexed in [`decisions/README.md`](decisions/README.md).
ADRs are English-only — see
[ADR 0005](decisions/0005-i18n-policy.md) for the internationalisation
policy that scopes what is and is not mirrored into Japanese.

## Japanese

The prose documents above have Japanese translations under
[`ja/`](ja/). Each English document links its translation in a
switcher on its first line; the English file is the source of truth
in every pair.
