# Reference index

Every external source FontCraft is specified from or built on, with its licence and what it is
used for. A source is added here, with its licence checked, **before** it is used
([`clean-room.md`](clean-room.md)). "Verified" means the licence file or the page was read on the
date given; "recalled" means it has not been checked yet and must be before the source is relied on.

Roles: **primary** defines behaviour we match; **oracle** is a tool whose output ours is compared
with; **dependency** is code we build on; **spec** defines a format; **secondary** fills gaps.

## Editor references

| Source | Where | Licence | Role | Status |
|---|---|---|---|---|
| Glyphs 4 handbook | https://handbook.glyphsapp.com/ (single-page edition `/en/single-page/`, PDF edition) | proprietary documentation, freely readable | **primary** for the editor. 18 chapters; `docs/spec/03a`–`18` are written from it | verified 2026-10-07 (covers version 4) |
| Glyphs file format documentation | https://github.com/schriftgestalt/GlyphsSDK › `GlyphsFileFormat/` (versions 2, 3 and 4, with JSON schemas) | Apache-2.0 | **spec** for `.glyphs` and `.glyphspackage` | verified 2026-10-07 |
| GlyphData | https://github.com/schriftgestalt/GlyphsInfo (`GlyphData.xml`, `GlyphData_Ideographs.xml`) | MIT ("Copyright (c) 2016 Georg Seifert") up to commit 9e03a49 of 2024-11-22; BSD-3-Clause ("Copyright (c) 2024, Georg Seifert") from that commit on. The LICENSE at the pinned commit decides the `NOTICE` text. The README carries Adobe's BSD-3-Clause notice for the Adobe Glyph List, which travels with the data | the glyph database (M4): glyph name → Unicode, category, script, production name. Usable with attribution | verified 2026-10-10 (LICENSE history) |
| Unicode CLDR | https://github.com/unicode-org/cldr (exemplar characters in `common/main/*.xml`) | Unicode License v3 for the data ("All other content in this repo"); the UTS #35 specification text in the same repository is under the Unicode Terms of Use and is not copied | languages and their characters for Font View's Languages section and its missing-glyph badges (M4). Pinned at release-48-2 (commit 1129998, the final 48.2 release of 2026-03-16) in `xtask/glyphsets.toml`; the LICENSE at that commit is the Unicode License v3 | verified 2026-10-10 |
| Google Fonts glyph sets | https://github.com/googlefonts/glyphsets (definitions in `Lib/glyphsets/definitions/*.yaml` and `data/definitions/`, built sets in `data/results/` as `.txt`, `.nam`, `.glyphs`, `.plist`) | Apache-2.0: the repository's single `LICENSE`, also declared by its `Cargo.toml` and `pyproject.toml`; no file or directory carries another licence. The encoded characters come from gflanguages (https://github.com/googlefonts/lang, Apache-2.0, `LICENSE.txt`), whose exemplar characters are adapted from CLDR; gflanguages dropped its Hyperglot (GPL-3.0) dependency in 0.2.0 for licensing reasons | the Start Window's glyph sets (M4). Usable with attribution; `NOTICE` carries Apache-2.0 and, for the CLDR-derived characters, the Unicode License v3. Pinned at commit 432104e in `xtask/glyphsets.toml`; the generator reads the built `.glyphs` files of `data/results/glyphs/` (glyph name and code point together) | verified 2026-10-10 at commit 432104e |
| FontLab 8 help | https://help.fontlab.com/fontlab/8/ (and the FontLab 7 manual for per-feature pages) | proprietary documentation, freely readable | **secondary**: `docs/spec/90-secondary-references.md` | read 2026-10-07; the pages are listed in that file |
| RoboFont documentation | https://robofont.com/documentation/ | proprietary documentation, freely readable | **secondary** | read 2026-10-07; pages listed there |
| FontForge user documentation | https://fontforge.org/docs/ | the program is GPL-3.0-or-later (recalled); **documentation only, never the source repository** | **secondary**: legacy and bitmap formats, the long tail of menu commands | docs read 2026-10-07; pages listed there |
| Fontra documentation | https://docs.fontra.xyz/ ; repository https://github.com/fontra/fontra (formerly googlefonts/fontra) | GPL-3.0 (verified 2026-10-07 on the archived repository page); **documentation only** | **secondary**: collaborative editing, variable components | docs read 2026-10-07; pages listed there |

The primary reference is known **only through its documentation** so far: no row about it has
`status = "observed"` yet. The owner has no licensed copy; public tutorial videos, watched by the
owner, are the substitute, and a row a video confirms becomes `observed` (D13) ([`spec/00-open-decisions.md`](spec/00-open-decisions.md), O1;
[`clean-room.md`](clean-room.md) › Tutorial videos).

## Command-line and compiler references

| Source | Where | Licence | Role | Status |
|---|---|---|---|---|
| AFDKO | https://github.com/adobe-type-tools/afdko (`docs/`, `python/afdko/`, `c/`) | Apache-2.0 | **primary** for the command line: makeotf, otfautohint, tx, spot, sfntedit, sfntdiff, mergefonts, rotatefont, otf2ttf, ttfcomponentizer, buildmasterotfs, buildcff2vf, checkoutlinesufo, comparefamily and the rest; **oracle** for the CFF path; otfautohint's algorithm is ported into `crates/autohint` (M10), with a credit in each file and in `NOTICE` | verified 2026-10-07 |
| OpenType Feature File Specification | in AFDKO: `docs/OpenTypeFeatureFileSpecification.md`, plus `Feature_Parser_Notes.md`, `Variable_Feature_Guide.md` | Apache-2.0 | **spec** for `.fea` | verified 2026-10-07 |
| Adobe Glyph List, AGLFN | https://github.com/adobe-type-tools/agl-aglfn | BSD-3-Clause | glyph name ↔ Unicode | verified 2026-10-07 |
| fonttools | https://github.com/fonttools/fonttools | MIT | **oracle** (ttx, subset, merge, varLib, instancer, feaLib, cu2qu, interpolatable) and reference for those tools' options; designspace documentation | verified 2026-10-07 |
| fontmake | https://github.com/googlefonts/fontmake | Apache-2.0 | **oracle**: the Python compile pipeline (static and variable, TrueType and CFF) | verified 2026-10-07 |
| fontc | https://github.com/googlefonts/fontc (crates `fontc`, `fontir`, `fontbe`, `fea-rs`, `glyphs-reader`, `ufo2fontir`, `glyphs2fontir`, tools `ttx_diff`, `otl-normalizer`, `fontc_crater`) | repository Apache-2.0; the published 1.0.0 crates declare MIT/Apache-2.0 | **oracle** for TrueType-flavoured output; possible **dependency** ([decision 0002](decisions/0002-compile-pipeline.md)); source of the comparison method | verified 2026-10-07 |
| glyphsLib | https://github.com/googlefonts/glyphsLib | Apache-2.0 (6.15.0) | **oracle** through fontmake for `.glyphs` sources; its `propagate_all_anchors` is the reference output of the M6 anchor-propagation measure | verified 2026-10-11 |
| fontbakery | https://github.com/fonttools/fontbakery | Apache-2.0 | the check catalogue behind `docs/spec/21-qa.md` | verified 2026-10-07 |
| ttfautohint | https://freetype.org/ttfautohint/ (documentation and command-line help) | FreeType License or GPLv2, at the user's choice | **oracle** for the TrueType autohinter measure (M10), and an optional external program that FontCraft runs on a finished font when the TTFAutohint binary path parameter names one. Its source is never read and it is never linked: the GPL alternative puts the code outside the clean-room rules | from the M10 plan, 2026-10-11 |

**Code derived from a reference app's source.** fontc's anchor propagation
(`fontir/src/propagate_anchors.rs`, in `glyphs-reader/src/` at older commits) says in a comment
that it reimplements a function of the primary reference app with a similar name. glyphsLib's
`builder/transformations/propagate_anchors.py` says in its header that it is based on fontc's
version, and that fontc's version is based on the reference app's original Objective-C code,
which its authors shared with the fontc team privately. Both files are Apache-2.0, but their
code comes from proprietary source. [`clean-room.md`](clean-room.md) says when such code may be
ported. The owner decided not to port either file (`docs/plans/M6.md` § 7, question 7):
FontCraft's anchor propagation is written from the handbook's description and from the oracles'
outputs observed as a black box, and its specification is the M6 propagation measure (equal to
glyphsLib's `propagate_all_anchors` on every corpus glyph). Nobody working on FontCraft reads the
two files. fontc's kerning writer (`fontbe/src/features/kern.rs`), its mark writer
(`fontbe/src/features/marks.rs`), its corner components (`glyphs-reader/src/corner_components.rs`,
based on glyphsLib's `filters/cornerComponents.py`) and glyphsLib's `cornerComponents.py` carry no
such note in their headers.

## Format specifications

| Format | Where | Notes |
|---|---|---|
| OpenType 1.9.x | https://learn.microsoft.com/typography/opentype/spec/ ; ISO/IEC 14496-22 (Open Font Format) | every table in `spec/inventory/40-tables.toml` (recalled URLs; link each table's page from its row as it is implemented) |
| TrueType instructions | Apple's TrueType Reference Manual; the OpenType spec's instruction-set chapters | M10 (recalled) |
| CFF and Type 2 charstrings | Adobe Technical Notes 5176 and 5177; CFF2 in the OpenType spec | we write these ourselves ([`spec/40-compile.md`](spec/40-compile.md) § 3) (recalled) |
| Type 1 | Adobe Type 1 Font Format ("the black book"), Technical Note 5015 | import only, M13 (recalled) |
| UFO 3 (and 2) | https://unifiedfontobject.org/ ; repository https://github.com/unified-font-object/ufo-spec | verified 2026-10-07; the repository has no licence file, the specification is public |
| designspace 5 | fonttools: `Doc/source/designspaceLib/` | MIT; verified 2026-10-07 |
| WOFF 1.0, WOFF 2.0 | W3C Recommendations | (recalled) |
| Unicode Character Database | https://www.unicode.org/ucd/ | Unicode License v3 (verified 2026-10-10, https://www.unicode.org/license.txt); categories, scripts, case mappings, decompositions |
| Model Context Protocol | https://modelcontextprotocol.io | `fontcraft-mcp` speaks revisions 2025-06-18 and 2024-11-05 (recalled; the sibling apps also accept 2025-03-26, which requires request batches) |

## Rust crates

Fourteen crates were checked with `cargo check` for the native target and for
`wasm32-unknown-unknown` on 2026-10-07 and all passed both: kurbo, write-fonts, skrifa, norad,
fea-rs, glyphs-reader, fontc, fontir, fontbe, ufo2fontir, glyphs2fontir, harfrust, linesweeper
and vello_cpu. (egui and eframe are checked by the gate itself.) Licences are the ones crates.io
reports.

| Crate | Version | Licence | Use | In the workspace |
|---|---|---|---|---|
| kurbo | 0.13.1 | Apache-2.0 OR MIT | curves, paths, affine maps, cubic → quadratic | yes |
| write-fonts | 0.54.0 | MIT OR Apache-2.0 | writing tables | yes |
| read-fonts | 0.45.0 | MIT OR Apache-2.0 | reading tables | transitive (through write-fonts; also skrifa in tests) |
| skrifa | 0.48.0 | MIT OR Apache-2.0 | outlines and metrics from compiled fonts; tests read our output back with it | dev |
| egui, eframe | 0.36.2 | MIT OR Apache-2.0 | UI, as in the sibling apps | yes |
| norad | 0.19.0 | MIT OR Apache-2.0 | UFO 1–3 read, UFO 3 write, designspace 5 | planned, M1 (its gaps: [`spec/30-source-formats.md`](spec/30-source-formats.md) § 4.2) |
| fea-rs | 1.0.0 | MIT/Apache-2.0 | feature-file compiler | planned, M8 (its gaps: [`spec/12-feature-code.md`](spec/12-feature-code.md)) |
| fontc, fontir, fontbe, ufo2fontir, glyphs2fontir | 1.0.0 | MIT/Apache-2.0 | whole-font compiler, usable as a library on an in-memory source | oracle; dependency question in decision 0002 |
| glyphs-reader | 1.0.0 | MIT/Apache-2.0 | reads `.glyphs` 2 and 3 | **not suitable as our reader**: read-only, rejects version 4, rewrites the document on load. Test oracle only |
| harfrust | 0.14.0 | MIT | text shaping for previews | planned, M5 (an older version is already in the lockfile through the UI toolkit) |
| linesweeper | 0.5.0 | MIT OR Apache-2.0 | curve booleans (overlap removal) | yes, in `pathops` |
| usvg | 0.47.0 | Apache-2.0 OR MIT | SVG parsing: resolves `use` references, transforms and basic shapes into paths; built without default features, so without text support | yes, in `io-svg` |
| regex | 1.13.1 | MIT OR Apache-2.0 | regular expressions for the glyph search field, smart-filter rules and Find and Replace; built without default features, with `std` and `unicode`, and within size and nesting limits | yes, in `glyphquery` |
| vello_cpu | 0.3.0 | Apache-2.0 OR MIT | CPU rasteriser for the canvas and headless renders | planned, M3 (an older version is already in the lockfile through the UI toolkit) |

`glyphslib` and `babelfont` (crates that advertise `.glyphs` writing) were seen only on their
crates.io pages; nothing is known about their fidelity. Evaluate before relying on either.

## The sibling apps

| Source | Where | Licence | Use |
|---|---|---|---|
| VectorCraft | https://github.com/storytold/vectorcraft | MIT OR Apache-2.0 | the model for this repository's layout, gates and conventions (read in full, 2026-10-07) |
| PhotoCraft | https://github.com/storytold/photocraft | MIT OR Apache-2.0 | its `AGENTS.md` (read 2026-10-07): layers, golden rules, parity and scorecard tooling |
| craft-fonts | https://github.com/storytold/craft-fonts | code MIT OR Apache-2.0; each font its own (OFL-1.1 for the five listed in its README, read 2026-10-07) | how the apps take UI fonts as an optional build input |
| craftrules | https://github.com/storytold/craftrules | unknown | the shared standards. **Not readable without access**; known only through what the two apps quote |

## Test corpora

Pinned in [`xtask/corpus.toml`](../xtask/corpus.toml); see [`testing.md`](testing.md). Licences
were read from each repository's licence file on 2026-10-07.

| Corpus | Licence | Holds |
|---|---|---|
| googlefonts/fontc `resources/testdata` | Apache-2.0 | about 160 small sources, one compiler behaviour each |
| fonttools/fonttools `Tests/` | MIT | feature files with expected output, designspace and variation cases |
| googlefonts/OswaldFont | OFL-1.1 | one real variable family in `.glyphs` |
| rsms/inter | OFL-1.1 | one large real family in `.glyphspackage` |
