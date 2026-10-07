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
| GlyphData | https://github.com/schriftgestalt/GlyphsInfo (`GlyphData.xml`, `GlyphData_Ideographs.xml`) | BSD-3-Clause | glyph name → Unicode, category, script, production name. Usable with attribution | verified 2026-10-07 |
| FontLab 8 help | https://help.fontlab.com/fontlab/8/ (and the FontLab 7 manual for per-feature pages) | proprietary documentation, freely readable | **secondary**: `docs/spec/90-secondary-references.md` | read 2026-10-07; the pages are listed in that file |
| RoboFont documentation | https://robofont.com/documentation/ | proprietary documentation, freely readable | **secondary** | read 2026-10-07; pages listed there |
| FontForge user documentation | https://fontforge.org/docs/ | the program is GPL-3.0-or-later (recalled); **documentation only, never the source repository** | **secondary**: legacy and bitmap formats, the long tail of menu commands | docs read 2026-10-07; pages listed there |
| Fontra documentation | https://docs.fontra.xyz/ ; repository https://github.com/fontra/fontra (formerly googlefonts/fontra) | GPL-3.0 (verified 2026-10-07 on the archived repository page); **documentation only** | **secondary**: collaborative editing, variable components | docs read 2026-10-07; pages listed there |

The primary reference is known **only through its documentation** so far. Nothing in `spec/` has
`status = "observed"`. Whether the owner has a licensed copy to observe is open
([`spec/00-open-decisions.md`](spec/00-open-decisions.md), O1).

## Command-line and compiler references

| Source | Where | Licence | Role | Status |
|---|---|---|---|---|
| AFDKO | https://github.com/adobe-type-tools/afdko (`docs/`, `python/afdko/`, `c/`) | Apache-2.0 | **primary** for the command line: makeotf, otfautohint, tx, spot, sfntedit, sfntdiff, mergefonts, rotatefont, otf2ttf, ttfcomponentizer, buildmasterotfs, buildcff2vf, checkoutlinesufo, comparefamily and the rest; **oracle** for the CFF path | verified 2026-10-07 |
| OpenType Feature File Specification | in AFDKO: `docs/OpenTypeFeatureFileSpecification.md`, plus `Feature_Parser_Notes.md`, `Variable_Feature_Guide.md` | Apache-2.0 | **spec** for `.fea` | verified 2026-10-07 |
| Adobe Glyph List, AGLFN | https://github.com/adobe-type-tools/agl-aglfn | BSD-3-Clause | glyph name ↔ Unicode | verified 2026-10-07 |
| fonttools | https://github.com/fonttools/fonttools | MIT | **oracle** (ttx, subset, merge, varLib, instancer, feaLib, cu2qu, interpolatable) and reference for those tools' options; designspace documentation | verified 2026-10-07 |
| fontmake | https://github.com/googlefonts/fontmake | Apache-2.0 | **oracle**: the Python compile pipeline (static and variable, TrueType and CFF) | verified 2026-10-07 |
| fontc | https://github.com/googlefonts/fontc (crates `fontc`, `fontir`, `fontbe`, `fea-rs`, `glyphs-reader`, `ufo2fontir`, `glyphs2fontir`, tools `ttx_diff`, `otl-normalizer`, `fontc_crater`) | repository Apache-2.0; the published 1.0.0 crates declare MIT/Apache-2.0 | **oracle** for TrueType-flavoured output; possible **dependency** ([decision 0002](decisions/0002-compile-pipeline.md)); source of the comparison method | verified 2026-10-07 |
| fontbakery | https://github.com/fonttools/fontbakery | Apache-2.0 | the check catalogue behind `docs/spec/21-qa.md` | verified 2026-10-07 |

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
| Unicode Character Database | https://www.unicode.org/ucd/ | Unicode licence (recalled); categories, scripts, case mappings, decompositions |
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
| linesweeper | 0.5.0 | MIT OR Apache-2.0 | curve booleans (overlap removal) | planned, M3 |
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
