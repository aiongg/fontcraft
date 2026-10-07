# 0002: The compile pipeline

**Status:** accepted 2026-10-07. Review when M9 (variable fonts) starts.

## Context

Compiling a font source to OpenType is most of the engineering risk: dozens of tables, each with
rules that fonts in the wild depend on. A Rust compiler exists (fontc 1.0.0, from the Google Fonts
team) on the same table-writing crates the sibling apps already use. The question was whether
FontCraft depends on it, copies its approach, or writes its own.

## What was checked (2026-10-07)

- **Building.** `fontc`, `fontir`, `fontbe`, `fea-rs`, `glyphs-reader`, `ufo2fontir`,
  `glyphs2fontir`, `write-fonts`, `skrifa`, `norad`, `kurbo`, `harfrust`, `linesweeper` and
  `vello_cpu` all pass `cargo check` for the native target **and for `wasm32-unknown-unknown`**.
  (Checked, not run, on wasm.)
- **Licences.** The fontc crates declare MIT/Apache-2.0 on crates.io; the fontations crates
  MIT OR Apache-2.0.
- **As a library.** `fontc::generate_font(source, options) -> Vec<u8>` exists, and
  `fontc::Input::from_glyphs(String)` builds a source from `.glyphs` text held in memory. A
  twelve-line program compiled fontc's `WghtVar.glyphs` test source from a string into a variable
  font with GDEF, GPOS, HVAR, MVAR, STAT, fvar, gvar and the core tables **in 21 ms**.
- **What it does not do** (`docs/spec/40-compile.md` § 3 and § 4, read from its source): no CFF or
  CFF2 output at all; no overlap removal; no overlap flags; no TrueType instructions; no WOFF.
  Its `.glyphs` reader rejects format version 4 and rewrites the document while loading.
- **write-fonts** has no CFF writer (a generated module exists but is not compiled in), no
  instruction assembler, no container code, and panics on outline points more than 32,767 units
  apart (found by our own test; we now refuse such outlines before calling it).

## Decision

1. **FontCraft owns its compile pipeline**: one pure function per table over our resolved model,
   in the `compile` crate, writing through `write-fonts` where it has the table and through our
   own crates where it does not (`cff`, `ttinstr`, `container`).
2. **fontc is an oracle from the first day.** Every corpus source that both can compile is
   compiled by both and compared (`docs/testing.md`). fontmake and makeotf are the oracles for
   what fontc cannot produce.
3. **fontc may be a bootstrap backend, behind our own interface, through its public file format
   only.** Once the `.glyphs` writer exists (M1), `model → .glyphs 3 text in memory → fontc →
   bytes` gives a correct variable TrueType compile long before our own builders reach M9. It is
   used where it is exact, selected by an export option, and never the only path: the editor's
   live preview, the CFF path, hinting and anything it lacks use our builders. We do not build on
   fontc's internal representation (`fontir`), which is its own to change.
4. **fea-rs is a dependency** for feature compilation (M8), wrapped by our `features` crate, which
   adds the preprocessing the primary reference's syntax needs. Its gaps are listed in
   `docs/spec/12-feature-code.md`.

## Why

- Parity with the primary reference means matching *its* export decisions (custom parameters,
  filters at export, production names, CFF, autohinting). A compiler built for another pipeline's
  defaults cannot be bent to that from outside.
- The editor needs incremental compiles for a live preview. Our builders are functions of small
  parts of the model; fontc is a whole-font batch.
- Half of what a professional exports is CFF, and neither fontc nor write-fonts writes it.
- Going through the documented file format instead of internal types keeps the coupling to
  something that has a specification.
- The sibling apps make the same trade: they depend on the font crates freely, and own the logic.

## Consequences

- We write and maintain CFF/CFF2, instruction assembly and the containers.
- The `.glyphs` writer is on the critical path for the bootstrap backend, which is one more reason
  it is M1 work.
- Differences between our output and the oracles' are a standing backlog, tracked in
  `docs/spec/40-compile.md` § 4.

## What would change it

If, at M9, our variation builders are far from matching the oracles while fontc matches, make the
bootstrap backend the default for variable TrueType export and keep ours for preview. If fontc
gains CFF output and a stable library interface over an in-memory model, reconsider item 1.
