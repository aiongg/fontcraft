# 0007: The feature compiler and the shaper

**Status:** accepted 2026-10-11 (M8 plan § 7, questions 2 and 3, decided by the orchestrator on
the owner's behalf). Review when fea-rs releases on the workspace's `write-fonts` version.

## Context

Decision 0002 makes fea-rs FontCraft's feature compiler. Its released version, 1.0.0, is built
on `write-fonts` 0.52; the workspace uses 0.54 for M2's table builders. fontc's main branch is
on 0.54 but unreleased, and its parse interface is still changing.

fea-rs panics on some invalid input and can run out of memory on some valid input (large class
products, long ligatures, deep nesting). FontCraft must never crash on a font file or on code a
user types.

Edit View and Text Preview need text shaping. The reference app's Text Preview uses the operating
system's shaper, which does not exist on every platform FontCraft runs on, nor on wasm.

## Decision

1. **fea-rs 1.0.0 from crates.io, inside a new standalone crate `fea`.** `fea` depends on no
   FontCraft crate and is the only crate that sees `write-fonts` 0.52 (a renamed dependency).
   Compiled tables leave it as bytes, which `compile` adds as raw tables. Generated kerning and
   mark lookups enter it in neutral types of its own and reach fea-rs through its
   `FeatureProvider`. Variable data enters through `fea`'s own `FeaVariations` interface, adapted
   inside `fea` to fea-rs's.
2. **Every fea-rs call runs behind pre-checks and the guard.** The pre-checks cap the text size,
   the include files (a closed set of virtual files, paths normalised inside the source folder),
   brace depth, class sizes, the product of ligature and contextual class sizes, ligature length
   and the total rule count, before fea-rs validates and compiles. The call runs under the engine
   guard on a thread with a 64 MB stack on native targets; wasm, where a panic aborts, uses lower
   caps. The limits are in the M8 plan § 3.
3. **harfrust 0.14 with unicode-bidi, inside a new L2 crate `shaping`.** harfrust is the HarfBuzz
   project's Rust port, on the `read-fonts` version the workspace uses, and it passes HarfBuzz's
   shaping test suites. unicode-bidi orders mixed directions (UAX #9). harfrust is used with its
   `std` feature only, never `icu`, which would pin a second `icu_properties`. Its version moves
   with `read-fonts`.
4. **The preview font.** `compile` builds a layout-only font of one master (`head`, `maxp`,
   `cmap`, `hhea`, `hmtx`, `GDEF`, `GSUB`, `GPOS`, `post`), with every unencoded glyph mapped to
   a private-use code point outside `cmap`. The engine keeps the last one per document as session
   state; it is never saved. Outline and width edits reach the shaper live through its font
   functions, so they need no recompile.

## Alternatives

| Option | Why not |
|---|---|
| A git dependency on fontc's main branch | shares the workspace's `write-fonts`, but builds on unreleased code whose parse interface still changes |
| The workspace back on `write-fonts` 0.52 | undoes the version M2's table builders use |
| Our own feature compiler | repeats work fea-rs has done and tested against fontTools |
| rustybuzz as the shaper | its repository is archived and points to harfrust |
| The operating system's shaper, as the reference app | not portable and not on wasm |

## What would change it

- A fea-rs release on the workspace's `write-fonts`: the renamed dependency goes, and nothing
  outside `fea` changes.
- A fea-rs or harfrust licence other than MIT or Apache-2.0, or a crate that stops building for
  wasm.
