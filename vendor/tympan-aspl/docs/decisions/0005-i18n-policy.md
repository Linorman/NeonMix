# ADR 0005: Internationalisation policy and file layout

- Status: Accepted
- Date: 2026-05-14

## Context

The project author writes Japanese natively, and a Japanese
rendering of the user-facing documentation is worth maintaining. But
the canonical language for code, comments, commit messages,
engineering design records, and external communication (issues, PRs)
is English.

A layout convention had already grown up organically before this ADR
existed:

- `README.md` has a `README.ja.md` twin (the **suffix** pattern).
- The `docs/` prose tree has a parallel `docs/ja/` directory
  mirroring it file-for-file (the **parallel-directory** pattern).
- `docs/decisions/*.md` — these ADRs — are English-only.

That convention was never written down, so it was applied by
imitation and risked drifting. This ADR records it, explains the one
place it deliberately differs from the sibling `tympan-ladspa`, and
fixes the scope so the question does not get re-litigated per PR.

`tympan-ladspa` ADR 0006 surveyed the layout options and chose an
all-suffix scheme (`docs/foo.ja.md` next to `docs/foo.md`), partly
to avoid migrating existing `docs/...` paths. `tympan-aspl` reaches a
different layout for the `docs/` tree, for reasons recorded below;
the two crates are siblings, not clones, and an i18n layout is
exactly the kind of low-stakes local choice where divergence is
acceptable.

## Decision

### 1. Layout: hybrid — suffix for the root README, a directory for `docs/`

- **`README.md` uses the suffix pattern**: its translation is
  `README.ja.md`, beside it at the crate root. Cargo requires the
  crate README to be `README.md` at the package root and
  `cargo publish` ships exactly that file — the root README cannot
  move into a language directory.
- **The `docs/` tree uses a parallel `docs/ja/` directory** that
  mirrors `docs/`'s structure file-for-file
  (`docs/overview.md` ↔ `docs/ja/overview.md`, and so on).

This differs from `tympan-ladspa` ADR 0006's all-suffix choice, and
the difference is deliberate: `tympan-aspl` mirrors *more* of its
`docs/` tree than ladspa does (six prose documents and growing), and
a parallel directory keeps `ls docs/` uncluttered — the English tree
reads cleanly, and the existence of the mirror is one `ls docs/ja/`
away. The `docs/ja/` directory was also already established in the
repository when this ADR was written; codifying it costs nothing,
migrating away from it would be churn for no user benefit.

The English file is the source of truth in every pair.

### 2. Scope: user-facing prose only

In scope for Japanese translation:

- `README.md`.
- The prose documents under `docs/`: `overview.md`,
  `architecture.md`, `testing.md`, `references.md`,
  `handoff-protocol.md`, `plugin-author-guide.md`, and the
  `docs/README.md` index. New prose docs join this tier by
  convention; this ADR need not be amended for each one.

Out of scope:

- **`docs/decisions/*.md`** — the ADRs, including this one. An ADR
  is an engineering working record, not user-facing prose: it is
  read by contributors, it cross-links other ADRs and source paths
  heavily, and it is most useful in the one language the code and
  commits are written in. Mirroring the ADR tree would double the
  maintenance of a working-record artefact for little readership
  gain.
- `CLAUDE.md` and other project-instruction files.
- Code comments, doc-comments, commit messages, PR descriptions,
  issue text — English, always.

### 3. Translations never block

A PR that changes an English document is **not** required to update
its Japanese twin in the same PR. CI does not enforce parity, and
reviewers do not hold a PR for translation. Drift is expected; a
manual pass before a release is the backstop. (No commit-SHA source
header is required in translated files — the lighter switcher
convention below is enough, and a manual pre-release audit catches
drift.)

### 4. Language switcher header

Every English document that has a translation carries a switcher as
its first line:

```
English | [日本語](ja/<name>.md)
```

— or `[日本語](README.ja.md)` for the root README. The translated
file carries the mirror back-link as *its* first line:

```
[English](../<name>.md) | 日本語
```

— or `[English](README.md) | 日本語` for `README.ja.md`. GitHub does
not switch languages by `Accept-Language`, so this explicit switcher
is the only discovery path between a pair.

## Consequences

Positive:

- The convention is now explicit: a contributor adding a doc knows
  where its translation goes and that the ADRs stay English-only.
- No migration. Every existing path — `CLAUDE.md`'s `@docs/...`
  references, ADR cross-links, the README documentation table —
  stays valid.
- `cargo publish` keeps using the root `README.md` with no
  configuration.
- The English `docs/` tree is uncluttered by translation files; the
  mirror is a single directory.

Negative:

- The scheme is a hybrid — suffix at the root, directory under
  `docs/` — rather than one uniform rule. The Cargo constraint on
  the root README makes a fully uniform scheme impossible anyway, so
  the hybrid is the honest description.
- A `docs/ja/` file does not auto-render on GitHub the way a
  subdirectory `README.md` does when browsed; the switcher links
  (and the `docs/ja/README.md` index) are the discovery path.

## Trigger for revisiting

- A third language is requested — at three or more, reconsider a
  bucketed `docs/<lang>/` scheme for the whole tree, root README
  included as far as Cargo allows.
- A static-site generator (mdBook with i18n, Docusaurus) is adopted;
  its i18n conventions would supersede this layout.
- The "translations never block" rule produces drift severe enough
  that a reader is misled — at which point add a lightweight source
  marker or a CI staleness check.

## References

- `tympan-ladspa` ADR 0006 — the sibling crate's all-suffix i18n
  decision, from which this ADR deliberately diverges for the
  `docs/` tree.
- `docs/README.md` / `docs/ja/README.md` — the documentation index
  pair this ADR's § 1 layout produces.
