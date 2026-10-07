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
| Engine behaviour | `crates/engine/tests/engine.rs` | undo restores exactly; snapshots share untouched glyphs; a failed command changes nothing; the vertical slice (new → draw → save → open → export → read back) |
| **Command sweep** | `crates/engine/tests/command_sweep.rs` | every engine command × three fixture states × 45 to 200 junk parameter objects each (about 5,600 calls): no panic, no change on error, invariants hold on success. A new command is swept without further work |
| Headless UI | `crates/ui-egui/src/tests.rs` | frames draw without a window; every menu item is a command; the control protocol drives the app and answers malformed requests |
| Protocol | `crates/mcp` | MCP negotiation, tools, JSON-RPC errors for malformed input, bad bytes on the wire |
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

None of these is needed to build or to run the ordinary tests. Oracle tests are opt-in (`cargo
test --features oracle`, planned with the harness in M2), skip with a message when a tool is
missing, and are meant to run always once a CI service exists.

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

`fontcraft-cli oracle` (planned, M2) runs this for one source or for a whole corpus and prints the
table of results. The corpus-wide run is how the "compile" row of the parity table is scored.

### Other oracles

- **Source round trips.** `.glyphs → model → .glyphs` is compared byte for byte, and semantically
  where the format allows several spellings. UFO likewise. The fontc `.glyphs` reader and norad
  are second readers of what we wrote.
  `cargo xtask roundtrip` runs this over the whole corpus and prints, per format and version,
  how many files come back identical; files saved by the format's reference tool are reported
  apart, since only they are held to byte identity. Accepted differences, each with its reason,
  are in `xtask/roundtrip-allow.toml`.
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
| `fontc` (`resources/testdata`) | about 160 small sources, each isolating one compiler behaviour: the first target of the oracle harness |
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
