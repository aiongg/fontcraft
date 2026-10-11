# Clean-room policy

FontCraft reimplements behaviour that exists in other people's software. This page says exactly
what may be looked at, what may be copied, and how to record it. It applies to people and agents
alike. When a case is not covered, the answer is no until this page says otherwise.

An honest limit first: the agents that write FontCraft are language models trained on public
text, which includes open-source code. That is not a clean room in the strict legal sense of two
separated teams. What we can and do guarantee is the working rule below: nothing is written by
consulting a forbidden source, nothing is copied, and every external source that was consulted is
on record.

## What you may read

| Source | Read | Port or copy |
|---|---|---|
| Public user documentation of any tool (handbooks, help sites, tutorials, release notes) | yes | **facts only**: feature names, menu labels, option names, shortcuts, numbers. Never sentences, examples, images or sample text |
| Public format specifications (OpenType, CFF, UFO, designspace, WOFF, the `.glyphs` format docs, the feature-file spec) | yes | facts, keys, constants, algorithms described there |
| Source code under MIT, Apache-2.0, BSD, ISC, Zlib (AFDKO, fonttools, fontmake, fontc, fontations, norad, fontbakery, GlyphsSDK, GlyphsInfo, the sibling Crafting Apps) | yes | yes, with attribution: say so in the file header, add the project and licence to `NOTICE`. Prefer depending on a crate to copying from it. The Crafting Apps' own convention is to share learnings, not code: follow it for their repositories |
| Source code under GPL, AGPL, LGPL or any copyleft licence (FontForge, Fontra, Inkscape, lib2geom…) | **no** | no. Do not open the repository. Their public *user documentation* is fine |
| Anything inside a proprietary app: binaries, resources, bundled scripts and plug-ins, glyph databases shipped only in the app, example fonts, templates | **no** | no. File and folder names in a listing are the most you may see |
| The running reference app, on fonts we made ourselves | yes, where the owner has a licence | observations only (see below) |
| The running reference app in a public tutorial video | yes, the owner watches | observations only (see Tutorial videos below) |
| Third-party plug-ins and scripts for a reference app | their documentation yes; their code only under a licence in the third row | as that row says |

A source's licence is checked, not assumed, and recorded in [`references.md`](references.md)
before the source is used.

### Code derived from a reference app's source

Some permissively licensed code says it was written from a proprietary app's own source, for
example from code the app's authors shared privately with that project. Its licence allows a
port, but its content comes from a source this page forbids. Such code is not ported, and not
read beyond the note that states where it comes from, unless the owner decides otherwise for
that file. Until then, the behaviour is written from public documentation and from the code's
outputs, observed as a black box. [`references.md`](references.md) lists the files known to carry
such a note.

## Writing

- **Write every sentence yourself.** In code comments, command labels and descriptions, error
  messages, documentation, `spec/` rows and commit messages. A summary that tracks a manual's
  sentence clause by clause is copying even with the words changed: read, understand, close the
  page, write.
- Names are facts. A menu item called Remove Overlap is called Remove Overlap; a custom parameter
  key is spelled as the files spell it, because we must read and write those files.
- **No examples from a manual.** Feature-code samples, sample strings, recipes and screenshots in
  a reference's documentation are its authors' work. Write your own examples.
- **Translations** are written from the meaning of our English text, never from another product's
  localisation.
- User-visible text does not name other vendors or products (`cargo xtask brands`). It says "the
  reference app" where a comparison is needed. Markdown documentation and `spec/` may name them.

## Observation

Watching a reference app is how the gaps in its documentation get filled: modifier keys, snapping
distances, defaults, edge cases. Running it ourselves needs a licensed copy, and it is done like
this:

- Only on **synthetic fonts made for the purpose** (in FontCraft, or by hand). Never open a
  commercial font, a client's font, or the app's own example files to study them.
- Record **what happened**, in our own words, in the behaviour note (`docs/spec/…`), and set the
  inventory row's `status` to `observed`.
- Screenshots are working material: they stay under the git-ignored `plan/local/`, are never
  committed, published or used as assets, and nothing is traced or redrawn from them.
- Compare **results**, not internals: export from the reference and from FontCraft, compare the
  files (`docs/testing.md`). Never attach a debugger, inspect memory, or read the app's files.
- If the app's licence terms forbid using it this way, stop and tell the owner.

### Tutorial videos

The owner has no licence for the primary reference (open decision O1). A public tutorial video
shows the running app, so watching one is observation of the running app, in the same way as
watching it run on our own synthetic fonts. The video itself is public material (first row of the
table above), and the same limits apply to it as to any documentation:

- Take **behaviour only**: which key or modifier does what, defaults shown on screen, the order of
  steps, what a dialog contains. Never the narration's wording, the presenter's examples or fonts,
  or images from the video. Every sentence we write about it is our own.
- Agents do not watch video. The owner watches and reports; the agent records the facts in our own
  words in the behaviour note.
- A row the video confirms gets `status = "observed"` (decision D13). Its `notes` name the video:
  title, channel, URL and the time in the video where the behaviour shows. When the video shows
  only part of what the row states, the row keeps its status and `notes` says which part the video
  confirms.

## Assets and data

- No icon, image, cursor, glyph artwork, sample text, preset, template, colour palette or example
  font from any reference app. Original work, open-source, public-domain or redistributable
  Creative Commons material only, each with a row in [`ASSETS.md`](../ASSETS.md).
- **Glyph data.** The mapping from glyph names to Unicode values, categories and production names
  is data FontCraft needs. Permitted sources: the Adobe Glyph List and AGLFN (BSD-3-Clause), the
  GlyphData XML published by the primary reference's authors in GlyphsInfo, and the Unicode
  Character Database (Unicode License v3). GlyphsInfo is MIT ("Copyright (c) 2016 Georg Seifert")
  up to commit 9e03a49 of 2024-11-22 and BSD-3-Clause ("Copyright (c) 2024, Georg Seifert") from
  that commit on. Both are permitted; the LICENSE at the pinned commit decides the `NOTICE` text.
  Its README carries Adobe's BSD-3-Clause notice for the Adobe Glyph List, which travels with the
  data. Fields those do not publish (decomposition recipes, default anchors, sort order) are
  derived by us from Unicode data or authored, never extracted from an app.
- **Language and glyph-set data.** Permitted: the Unicode CLDR data (Unicode License v3) and the
  Google Fonts glyph sets (Apache-2.0, characters adapted from CLDR). Not permitted: Hyperglot
  (GPL-3.0) and the reference app's own language and glyph-set lists.
- **Fonts.** Compiled fonts are never committed. Test fonts are built in code; real fonts come
  from the pinned corpus of openly licensed families (`xtask/corpus.toml`).
- Prefer generating defaults in code (sample strings, icons, cursors) over importing them.

## Dependencies

A crate may be added when its licence is MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0 or Unicode (the
UI toolkit's bundled default fonts come under their own font licences, listed in `NOTICE`), it builds for
`wasm32-unknown-unknown` if it sits below the app layer, and it does not pull in a copyleft
dependency. Anything else is the owner's decision. Note the licence in a comment beside the
dependency in `Cargo.toml`, as the sibling apps do.

## Recording

- A new source goes into [`references.md`](references.md) with its licence and what it was used for.
- Ported code carries a header naming the project, the file, the commit and the licence.
- A behaviour that rests on recollection is marked `status = "memory"` in its row; a design of our
  own, `inferred`. Neither is a fact about the reference until someone checks it.
- If you find you have seen something you should not have, say so in `docs/status.md`, do not
  work on that feature, and let the owner decide who does.
