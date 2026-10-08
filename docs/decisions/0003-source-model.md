# 0003: The source model

**Status:** accepted 2026-10-07. Evidence: `docs/spec/30-source-formats.md`.

## Context

The model must round-trip three source formats without loss: the primary reference's `.glyphs`
(versions 2, 3, 4, single file or package), UFO 3 with designspace 5, and our own. They disagree on
almost everything structural: one document per family against one folder per master; kerning by
side groups on the glyph against named groups in a side table; layers as a typed list against a
directory of glyph sets; node types, coordinate precision, where instance names live.

## Decision

- **One document per family, shaped like `.glyphs` version 3 with the version 4 additions.** UFO
  and designspace map into it with a short carry list; the reverse would put most of the primary
  reference's data into untyped dictionaries.
- **A tree of `Arc`s**: the root is cloned for a snapshot; an edit copies only the path to what
  changed. Undo, background saves, background compiles and previews all work on snapshots.
- **`extra` on every struct**, tagged by the format family a key came from, and values that
  remember their spelling (number tokens, key order), so writers can put back what readers did not
  understand, byte for byte where the format allows.
- **Our own `.glyphs` reader and writer** on our own property-list crate. The existing Rust reader
  is read-only, refuses version 4 and normalises the document on load; no Rust writer is known to
  be faithful. This is M1 work and on the critical path (decision 0002).
- **Our own UFO and designspace reader and writer** (amended 2026-10-08, owner's answer to
  question 11 of the M1 plan), on the XML half of our `plist` crate and an XML tokenizer, written
  in the layout fontTools ufoLib and designspaceLib produce. norad stays in the tests as a second
  reader of what we write. A spike found that norad writes no corpus UFO or designspace file back
  unchanged (0 of 140 UFOs, 0 of 112 designspace files; `docs/plans/M1.md` § 8.2): its layout is
  fixed and differs from ufoLib's, it rejects unknown GLIF elements, drops comments, does not write
  UFO 2, and replaces the folder on save.
- **The compiler reads a resolved projection** of the model (instances applied, parameters
  merged, components and alignment computed), not the editable model itself.

The M0 skeleton implements a small subset (font, one kind of master, glyphs, layers, paths,
components, anchors, `extra` as a flat map). The full struct outline is in
`docs/spec/30-source-formats.md` § Recommended model; M1 grows the skeleton into it.

## Costs

- Our own UFO layer is more code than norad: GLIF, designspace, UFO 2 up- and down-conversion and
  UFOZ, each with limits and hostile-input tests.

- A UFO-native user sees a merged document; saving back to UFO must reproduce their file layout,
  including per-master differences the merged model has to carry in `extra`.
- Format version 4 of the primary's format is new and partly unexplained in its documentation
  (17 open questions in the study).

## What would change it

A source format that becomes dominant and is shaped differently (for example a package-per-glyph
format built for version control). The native `.fontcraft` package form is where we would meet it.
