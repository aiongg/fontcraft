# Testing

Tests are the gate. This page says what kinds exist, what the oracles are, and how real fonts get
into tests without getting into the repository.

## What runs today

`cargo xtask ci` runs all of it. 78 tests at the end of M0.

| Kind | Where | What it proves |
|---|---|---|
| Unit tests | each crate's `#[cfg(test)]` | node lists → curves; cubic → quadratic; the format's round trip and rejections; parameter readers |
| Read-back | `crates/compile` | the compiled font parses with skrifa and has the outlines, metrics, names and mapping we put in |
| Determinism | `crates/compile` | the same font and options give the same bytes; the timestamp is an input |
| Hostile values | `crates/compile`, `crates/format` | extreme coordinates, surrogates, truncated and corrupted files are errors or warnings, never panics |
| Engine behaviour | `crates/engine/tests/engine/engine.rs` | undo restores exactly; snapshots share untouched glyphs; a failed command changes nothing; the vertical slice (new → draw → save → open → export → read back) |
| **Command sweep** | `crates/engine/tests/engine/command_sweep.rs` | every engine command × three fixture states × 45 to 200 junk parameter objects each (about 5,600 calls): no panic, no change on error, invariants hold on success. A new command is swept without further work |
| Headless UI | `crates/ui-egui/src/tests.rs` | frames draw without a window; every menu item is a command; the control protocol drives the app and answers malformed requests |
| Protocol | `crates/mcp` | MCP negotiation, tools, JSON-RPC errors for malformed input, bad bytes on the wire |
| Oracle comparison | `crates/oracle` | the normalised comparison on fonts built in code (identical builds, keyed differences, start points, reversed contours, glyph set and order, cmap, the allow-list, broken input); the tool runner with stand-in tools (output, failure, time limit). Opt-in (`--features oracle`): fontc and fontmake builds of a fixture compare as expected |
| Repository checks | `xtask` tests | the inventory loads; the layering rules catch each kind of violation; the vendor-names scanner; the asset rules; the corpus pins |

The desktop app was also run on a virtual display during the skeleton's verification and driven
over the control channel; that is a manual check, not yet an automated one.

## The oracles

A compiled font is an exact artefact, and several independent programs produce one from the same
source. That is the main correctness tool FontCraft has, and the reason the compiler and the CLI
can be built to a provable standard.

| Oracle | Installs with | Use it for |
|---|---|---|
| fontc | `cargo install fontc` (and its `ttx_diff`, `otl-normalizer`) | static and variable TrueType from `.glyphs` and designspace sources. Fast, deterministic |
| fontmake | `pip install fontmake` | the same, plus static CFF and variable CFF2; overlap removal |
| makeotf, otfautohint, tx, spot, sfntedit, sfntdiff, mergefonts | `pip install afdko` | the CFF path, PostScript hinting, dumps, table edits, merges |
| fonttools | `pip install fonttools` | `ttx` dumps, subsetting, merging, instancing, feature compilation, interpolation checks |
| fontbakery | `pip install fontbakery` | check results on the same binaries |
| skrifa | already a dev-dependency | outlines, metrics and names of our own output |
| harfrust | planned with text mode (M5) | shaping results of our own output |

A crate with many integration-test areas has one test binary, `tests/<name>/main.rs`, with
each area as a module (`engine`, `decompile`, `io_glyphs`, `io_ufo`, `plist`):
`cargo test -p fontcraft-engine --test engine <module>::` runs one. Speed measures and oracle
runs use `--profile measure`; shipping builds use release.

None of these is needed to build or to run the ordinary tests. Oracle tests are opt-in
(`cargo test -p fontcraft-oracle --features oracle`), skip with a message when a tool is missing,
and are meant to run always once a CI service exists.

Where the harness looks for a tool: `--reference-bin PATH` on the command line, then the
environment variable (`FONTCRAFT_FONTC`, `FONTCRAFT_FONTMAKE`), then `PATH`, then the install
locations `~/.cargo/bin/fontc`, `~/.local/share/uv/tools/fontmake/bin/fontmake` and
`~/.local/bin/fontmake`.

### How outputs are compared

Byte equality is the wrong test: two correct compilers differ in table order, padding, offsets,
checksums, timestamps and equivalent encodings. The method (described in our own words from the
reference tools' documentation; see `docs/spec/20-cli.md` › Oracles):

1. Compile the same source with FontCraft and with the oracle, same options, fixed timestamp.
2. Dump both to a normalised form, table by table: values that are allowed to differ are removed
   or canonicalised (checksums, modification time, glyph and lookup ordering that carries no
   meaning, floating-point rounding within a stated tolerance).
3. Layout tables are compared by *effect*: expand lookups to the list of rules they apply per
   script, language and feature, sort, and compare. Kerning and mark positioning are compared as
   values per glyph pair across the variation space, not as table structure.
4. Outlines are compared as curves within a tolerance; variation deltas by evaluating at sample
   locations.
5. Report per table: identical, equivalent, or different with the first differences. A table
   that differs for a known, accepted reason is on an allow-list **with the reason**
   (`docs/spec/40-compile.md` § 4); anything else fails.

`fontcraft-cli oracle` runs this for one source or for a whole corpus and prints the table of
results. The corpus-wide run is how the "compile" row of the parity table is scored.

### The oracle harness

`crates/oracle` and `fontcraft-cli oracle`. How the two builds are made the same is in
`docs/plans/M2.md` › M2.0.

```sh
fontcraft-cli oracle corpus/fontc/resources/testdata/glyphs3/WghtVar.glyphs   # one source, fontc
fontcraft-cli oracle --corpus                       # the comparison set (fontc's test data, Oswald)
fontcraft-cli oracle --corpus --against fontmake    # the other TrueType oracle
fontcraft-cli oracle --corpus --format otf          # fontmake's CFF builds
fontcraft-cli oracle --corpus --json --differences 10000 > report.json
fontcraft-cli oracle SOURCE --output-dir out/       # keep both fonts and the tool's log
```

Each table comes out as one of:

| Status | Meaning |
|---|---|
| identical | the same bytes (`head`: apart from the checksum and the dates) |
| equivalent | different bytes, nothing different once normalised |
| coverage-equal | `glyf` or `CFF ` against fontmake: some glyphs have different points but cover the same pixels (below) |
| allowed | every difference is on `crates/oracle/allow.toml`, which gives the reason |
| different | the first differences are listed, keyed by field, record or glyph |
| skipped | not compared, and the report says why: layout tables until M8, variation tables until M9, tables FontCraft does not write yet |

What normalisation removes or canonicalises:

| Table | Normalisation |
|---|---|
| head | `checksumAdjustment`, `created` and `modified` removed; `indexToLocFormat` (an offset size) not compared |
| hhea, vhea | `numberOfHMetrics` and `numberOfLongVerMetrics` not compared (`hmtx` and `vmtx` compare the values); the rest field by field |
| hmtx, vmtx | advance and side bearing per glyph, by glyph name |
| name | a set of records; the reference compiler's stamp at the end of name ID 5 (`;fontc 1.0.0`) removed; IDs 256 and up left out (feature and variation names) |
| cmap | code point → glyph name over every Unicode subtable, the list of subtables, the variation sequences |
| glyf, CFF | outlines drawn by skrifa without hinting, components resolved, compared within half a unit with any start point; reversed contours are named as such. For `glyf` also each glyph's kind (empty, simple, composite with its components and flags), bounds and instruction length; against a variable reference the `USE_MY_METRICS` flag is left out (fontc never sets it in a variable font, and FontCraft's static build does) |
| loca | offsets: compared through `glyf` |
| maxp, OS/2 | field by field |
| post | field by field; each glyph the two fonts name differently, as `post glyphName <ours>` (both build with production names) |

Overlaps. fontc never removes overlaps, so against fontc FontCraft builds with overlaps kept
and outlines are compared point by point. fontmake removes them for static fonts (its default
backend), and so does FontCraft against it. Two overlap removals give the same shape with
different points, so a glyph whose points differ is then compared by rendered coverage
(`fontcraft_pathops::coverage_difference`): both outlines filled with the nonzero rule at one
pixel per unit, coverage exact across each row and sampled on 16 sub-rows per pixel. The glyph
is coverage-equal when no pixel differs by more than 0.5 and the differences add up to at most
0.5% of the reference glyph's ink. A coverage-equal simple glyph is not compared by contour
count or bounds. The summary counts the glyphs of each outline table: compared, the same,
coverage-equal, different.

Glyphs are matched by name. When both fonts have as many glyphs, a glyph whose name only one
font has is matched with the other font's glyph of the same id, if that glyph's name is also only
in its font and both have the same code points: the compilers named it differently, which `post`
reports, and the per-glyph tables still compare it under FontCraft's name. Allow-list keys name
glyphs by the names written to the font (production names, such as `uni0410`). Any other
different glyph set or glyph order is reported once, as a source-reading difference: the two programs read the source differently (fontc turns bracket
layers and colour layers into glyphs, for example). The per-glyph tables then compare the glyphs
both fonts have. Exit status 1 means a table differs or a build failed.

To accept a difference, add an entry to `crates/oracle/allow.toml` with the table, the difference
key as the report prints it (`key`, more in `keys`; a `*` matches any run of characters, as in
`hmtx * advance`), optionally the oracle, parts of the source path (`source`, `sources`) and the
two values as the report prints them (`ours`, `theirs`, with `*` as in keys), and the reason. An
entry with values accepts a cause wherever it shows (a source without a version: `ours = "0"`,
`theirs = "1"`) without listing every source. A difference FontCraft is meant to fix does not go
there.

Reading our own output back: every corpus run with `--output-dir` keeps FontCraft's font
(`fontcraft.ttf` or `fontcraft.otf`) in each source's folder, also when the reference fails, so
the fonts can be checked with other readers (fontTools' `TTFont(...).ensureDecompiled()`, `ttx`).

### Other oracles

- **Source round trips.** `.glyphs → model → .glyphs` is compared byte for byte, and semantically
  where the format allows several spellings. UFO likewise. The fontc `.glyphs` reader and norad
  are second readers of what we wrote.
  `cargo xtask roundtrip` runs this over the whole corpus and prints, per format and version,
  how many files come back identical; files saved by the format's reference tool are reported
  apart, since only they are held to byte identity. Accepted differences, each with its reason,
  are in `xtask/roundtrip-allow.toml`.
- **CFF tables.** `crates/cff/crosscheck/crosscheck.py` (run with the fontmake tool's Python,
  outside the gate) compiles each corpus UFO and a few synthetic fonts with ufo2ft at
  `--optimize-cff 1`, records what its charstring pen receives, has the `cff_from_json` example
  build the table from that recording, and compares the two tables byte for byte. On a
  difference it names the first differing DICT entry or charstring.
- **Glyph database.** `cargo xtask glyphdata --check` (outside the gate; needs
  `cargo xtask glyphdata --fetch` first) looks up about 200,000 names in FontCraft's database, in
  fontc's `glyphs-reader` 1.0.0 and in glyphsLib (through `crates/glyphdata/crosscheck/glyphslib.py`,
  with the fontmake tool's Python or `FONTMAKE_PYTHON`): every record of GlyphsInfo's main file,
  each with `.sc`, `.ss01` and `.001`, 10,000 seeded ligatures, `uniXXXX` for every BMP code point
  with a record, and 1,000 `_` names. It compares category, subcategory, production name and code
  point with both, and script, case and description with glyphsLib. Known differences are in
  `crates/glyphdata/crosscheck/allow.toml`, each with its cause and the number of names it covers.
- **Path operations.** Overlap removal is compared with the result's area and winding, and with
  an independent implementation where one is available; property tests (a union's area is at most
  the sum, removing overlap twice changes nothing).
- **Hinting.** The interpreter in skrifa runs our instructions; rendered pixels at sample sizes
  are compared with the reference autohinters' output within a tolerance.
- **The reference app itself**, where the owner can run it: export from both, compare as above.
  This is the only oracle for the editor's own export decisions (`docs/clean-room.md` › Observation).

## Corpora

Real fonts are never committed (`docs/decisions/0006-fonts-in-the-repository.md`). They are pinned
by commit in `xtask/corpus.toml` and fetched into the git-ignored `corpus/`:

```sh
cargo xtask corpus            # what is pinned, what is present
cargo xtask corpus --fetch    # download at the pinned commits (about 300 MB)
```

| Corpus | Use |
|---|---|
| `fontc` (`resources/testdata`) | 353 small sources (`.glyphs`, packages, designspace, UFO), each isolating one compiler behaviour: with Oswald, the oracle harness's comparison set |
| `fonttools` (`Tests/`) | feature files with expected output, designspace and variation-model cases, subset and merge fixtures |
| `oswald` | one real variable family in `.glyphs`: the first whole-family comparison |
| `inter` | a large real family in package form: performance budgets and round-trip stress |
| `glyphssdk` | the format documents' own sample files for versions 2 and 3, single file and package |
| `markow`, `glyphspack`, `glyphs-mcp`, `openplotfont` | the only version 4 files: one real family, one font as file and package, small documents and packages (16 files in all) |

A test that needs a corpus skips with a message when it is absent. To add a corpus, check its
licence, add a pin with the licence and what it holds, and say here which tests use it.

## Fixtures

- Build fonts in code: `Font::new`, the testkit's `Fixture`, `rect` and `oval`.
- A source fixture that must be a file (a `.glyphs` with one odd construct) is written by us,
  small, under the crate's `tests/data/`, with a row in `ASSETS.md`.
- Never a compiled font, never anything from a reference app.

## Fuzzing and property tests

Today's hostile-input tests are deterministic loops (truncation, corruption, a junk-value table).
From M1, importers and parsers get property tests (proptest) with a few dozen cases in the gate and a
deeper run before touching an importer; a found failure becomes a regular test. Targets: every
source reader, the binary reader, the property-list parser, the feature preprocessor, the control
channel and MCP readers.

## Measuring

`fontcraft-cli perf` and `bench` (planned, M3) check the targets in `architecture.md` on a
synthetic font and on the `inter` corpus. Numbers are recorded with the date and the machine;
a budget that is not measured is not claimed.
