# Assets

Every icon, image, font source, test fixture, data table and other non-code file that ships in
this repository has a row here. `cargo xtask assets` (part of `cargo xtask ci`) fails on a file
under `assets/`, under any `tests/data/` folder or under a crate's `data/` folder that has no row,
and on any compiled font anywhere in the tree.

An asset must be one of: original work made for FontCraft by a contributor (licensed MIT OR
Apache-2.0), OSI open source, public domain or CC0, or Creative Commons with redistribution
allowed. Never anything taken from a reference app: no icons, glyph artwork, sample text, presets,
example fonts or screenshots. If where a file came from is unclear, it does not go in.

Rules for fonts (the full text is in [docs/clean-room.md](docs/clean-room.md)):

- **Compiled fonts are never committed.** Tests build the fonts they need in code. Real fonts come
  from the pinned corpus (`cargo xtask corpus --fetch`, git-ignored).
- **Font sources** (`.glyphs`, `.ufo`, `.designspace`…) are committed only as small fixtures
  written for FontCraft, under a `tests/data/` folder, with a row below.
- Fonts the app itself draws its interface with, when it needs more than the toolkit's defaults,
  come from the shared craft-fonts repository as an optional build input, as in the sibling apps.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| `crates/ui-egui/src/icons.rs` | Toolbar and palette icons, drawn in code | FontCraft contributors | original | MIT OR Apache-2.0 |
| `crates/glyphdata/data/glyphdata.bin` | The built-in glyph database: the records of GlyphsInfo's `GlyphData.xml` and `GlyphData_Ideographs.xml`, the Adobe Glyph List and AGLFN, and the Unicode 17.0 general categories, bidi classes, scripts, canonical decompositions and combining classes, compiled by `cargo xtask glyphdata --generate` | Georg Seifert (GlyphsInfo); Adobe (AGL, AGLFN); Unicode, Inc. (UCD) | GlyphsInfo commit 1f126bd, agl-aglfn commit 4036a9c, UCD 17.0.0 (pins in `xtask/glyphdata.toml`) | BSD-3-Clause (GlyphsInfo, `LICENSE-GlyphsInfo.txt`); BSD-3-Clause (AGL, `LICENSE-AGL.txt`); Unicode-3.0 (`LICENSE-Unicode.txt`) |
| `crates/glyphdata/data/LICENSE-GlyphsInfo.txt` | GlyphsInfo's licence at the pinned commit, with the Adobe Glyph List notice of its README | Georg Seifert; Adobe | GlyphsInfo commit 1f126bd | BSD-3-Clause |
| `crates/glyphdata/data/LICENSE-AGL.txt` | The notice of the Adobe Glyph List files | Adobe | agl-aglfn commit 4036a9c | BSD-3-Clause |
| `crates/glyphdata/data/LICENSE-Unicode.txt` | The Unicode License v3 | Unicode, Inc. | unicode.org/license.txt | Unicode-3.0 |
| `crates/glyphdata/data/sets.bin` | The built-in language and glyph-set data: the main exemplar characters and English names of the Unicode CLDR locales, grouped by script, and the Google Fonts glyph sets (glyph names and code points), compiled by `cargo xtask glyphsets --generate` | Unicode, Inc. (CLDR); Google Fonts (glyph sets) | CLDR release-48-2 (commit 1129998), glyphsets commit 432104e (pins in `xtask/glyphsets.toml`) | Unicode-3.0 (`LICENSE-CLDR.txt`); Apache-2.0 (`LICENSE-glyphsets.txt`; its characters come from gflanguages, Apache-2.0, adapted from CLDR) |
| `crates/glyphdata/data/LICENSE-CLDR.txt` | The Unicode CLDR licence at the pinned release | Unicode, Inc. | CLDR release-48-2 | Unicode-3.0 |
| `crates/glyphdata/data/LICENSE-glyphsets.txt` | The Google Fonts glyph sets licence at the pinned commit | Google Fonts | glyphsets commit 432104e | Apache-2.0 |
| `crates/features/src/unicode18.rs` | The Unicode 18.0 changes to Script_Extensions and Bidi_Class over the ICU4X 2.1 data, as range tables the kern writer reads | Unicode, Inc. (UCD); the fontc authors (tables generated from the UCD) | fontc commit 4504578 (`fontdrasil/src/unicode18.rs`), UCD 18.0.0 | Apache-2.0 (fontc); Unicode-3.0 (`crates/glyphdata/data/LICENSE-Unicode.txt`) |
