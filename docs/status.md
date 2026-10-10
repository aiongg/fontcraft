# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-10-10, M3 wave 1 and M3.1 integrated._

## State of the tree

- `cargo xtask ci` passes all nine steps (584 tests; about 68,600 lines of Rust in 19 crates).
  Built and run on Linux x86-64 with Rust 1.97.
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at. macOS and Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

## Where to start

**M3, Edit View drawing**, following `docs/plans/M3.md` § 6. Done: M3.0 (spec pass), the M3.6
spike (linesweeper 0.5.0 chosen; its rare hang and failed assertion are in § M3.6 "Spike
result"), and wave 1's crates without commands: `tools` (M3.3 core), `audit` (M3.17, 34 of 36
outline checks), `io-svg` (M3.8); M3.1 engine foundations (`params::Targets`, `Session::begin`/`preview`/
`commit`/`cancel`, `layer.inspect`, the glyph lock, `geom` segments). linesweeper is kept with
the time limit and not reported upstream (owner's decision, plan § M3.6). Next: the rest of the
`pathops` crate (M3.6 step 2), M3.2 canvas, then wave 2 (M3.4, M3.5 engine, M3.8 commands, M3.9 engine, M3.10, M3.16
engine). Static export keeps overlaps with a warning until M3.7.

M1 and M2 are done (`docs/plans/M1.md`, `docs/plans/M2.md`). M2 left on purpose: instances
between masters (M9), CFF subroutines (before M12), production names (M4), the compile plan's
`productionNames` and `features` sections, the CLI's `--name-by`, `--output-layout` and pattern
instance selection, and one reader case (a designspace master missing a metric before one it
states reads it as 0). `fontcraft-cli oracle --corpus` (fontc, or fontmake with `--format otf`)
measures the compiler; `crates/oracle/allow.toml` holds every accepted difference with its reason.

Before you start: `cargo xtask corpus --fetch` (about 300 MB) if `corpus/` is missing, and the
oracles in `docs/testing.md` (fontc via cargo, fontmake via a Python tool installer).
`cargo xtask roundtrip` shows where the round trips stand.

## What the planning session decided

See `docs/decisions/`. In one line each: Glyphs 4 and AFDKO are the primary references; our own
compile pipeline with fontc as oracle and bootstrap backend; a one-document model shaped like the
primary's format with our own `.glyphs` reader and writer; commands with explicit targets and
atomic failure; WebAssembly plug-ins and no Python; no compiled fonts in the repository.

## Known weak spots

- **The inventory is unreviewed line by line.** It passed mechanical checks and one sample audit
  (209 handbook rows re-read by a second reader: 1 wrong, 14 imprecise, all corrected; patterns in
  `docs/spec/README.md`). Duplicated meaning across files is likely (open decision D2 lists known
  pairs).
- **33 rows are from memory** (`grep -c '^status = "memory"' spec/inventory/*.toml`): 12 TrueType
  autohinter options in `14-…`, 9 Edit-menu basics in `00-…`, 5 secondary-reference rows in
  `90-…`, 2 each in `05-…` and `30-…`, 1 each in `09-…`, `10-…` and `16-…`. Verify or drop them
  before implementing.
- **Nothing has been observed in the reference app.** About 440 open questions are listed at the
  ends of the notes.
- **The compiler writes static fonts only.** TrueType and CFF, plain, WOFF and WOFF2; the core
  tables match fontc and fontmake over the corpus (with allow-listed causes: variable references,
  bracket and colour glyphs, the glyph database, smart components); no features, variations,
  hinting, overlap removal or CFF subroutines yet (`docs/plans/M2.md` § 5).
- **`docs/spec/20-cli.md` is very long** (about 26,000 words). Read the section for the subcommand
  you are building, not the whole file.

## Known limits in the skeleton's code

An adversarial review of the skeleton reported 31 findings. Most were fixed in the same session
(coordinate and point-count bounds before the glyph writer, glyph-name sanitising for `post`,
style linking, bounded reads on every channel, protocol id checks, the indexing lints, a
virtualised Font View). These were left, on purpose, for the milestone that reshapes the code
around them. None can lose data; most are about scale.

| Limit | Where | Fix it in |
|---|---|---|
| The history list is copied for each command's result | `crates/engine` | M1 |
| The menu tree is rebuilt every frame | `crates/ui-egui/src/menus.rs` | M3 |
| The vendor-names scanner does not see through `concat!` or string escapes | `xtask/src/brands.rs` | when it matters |

## Waiting on the owner

`docs/spec/00-open-decisions.md` › For the owner: the name (O2), access to the sibling apps' shared
standards (O3), the application id (rest of O4), FontLab (O5), platforms (O6). O1 is answered (no
licence; tutorial videos watched by the owner) and so is the repository's home.

## Dev log

Append one terse entry per session: what landed, numbers, what is still open.

- **2026-10-07, planning session.** Read VectorCraft and PhotoCraft conventions. Chose
  references. Spiked dependencies: 14 font crates check for native and wasm; fontc compiles an
  in-memory `.glyphs` source to a variable font in 21 ms as a library. Inventory compiled by 17
  parallel readers (3,618 rows, 22 notes), deduplicated, checked for verbatim overlap with the
  handbook, then audited in a sample and fact-checked against the code. Skeleton built: 8 crates,
  2 apps, xtask; 27 commands; 78 tests; gate green. Found and fenced a panic in the glyph writer
  for points more than 32,767 units apart; fixed most of a 31-finding code review. Still open:
  everything in "Where to start" and the table of known limits.
- **2026-10-07, set-up on the build box.** History rewritten to the owner's identity; repository
  pushed (private). Gate green locally (4 min 17 s). O1 (no licence; tutorial videos) and O4
  recorded. Corpus fetched, oracles installed. M1 plan drafted: 7 packages. No version 4
  `.glyphs` file in the corpus. The owner accepted the plan.
- **2026-10-07, M1.0 and M1.1 (OpenStep).** Version 4 corpora pinned (16 files); sparse corpus
  fetch; `cargo xtask roundtrip` (527 sources). Spikes: no existing crate round-trips any file
  (plan § 8); own plist crate, own UFO I/O recommended (owner asked). `crates/plist`
  OpenStep reader and writer: 215 of 274 `.glyphs` sources identical, every reference-written
  one outside fontc's test data. Next: the model (M1.2); the XML plist after question 11.
- **2026-10-08, M1.2, M1.3, M1.5, M1.6 (part).** Model restructured (tagged `Extra`, `Arc` parts,
  glyph index, metric definitions, layer ids, kept transforms, kerning, axes, instances, features,
  file store); native format version 2. `crates/io-glyphs`: versions 2 to 4 and packages, every
  reference-written corpus source byte-identical; package side files are model data, so a package
  and a single file of the same font read the same. `crates/decompile`: all 342 corpus binaries
  open. `file.open`/`file.save` on every source and compiled fonts. Oswald shown in the app.
  Next: question 11, then UFO; meanwhile the native format's layout and the CLI's
  inspection commands (both done since). M2 plan drafted (`docs/plans/M2.md`), waiting for review.
- **2026-10-08, M1.4.** Owner chose our own UFO I/O (decision 0003 amended) and accepted the M2
  plan. `crates/plist`: lossless XML tree and XML property lists. `crates/io-ufo`: GLIF, UFO 2
  and 3, designspace, UFOZ read; kept spellings used only while they still mean what the model
  holds. Corpus: 19,453 GLIF, 125 UFOs, 59 designspace documents with their UFOs, all identical
  through the model and through `.fontcraft`. Engine: open, save, save as, `file.exportUfo`;
  CLI `convert` to `.ufo`/`.designspace`. Oswald converted to designspace and compiled by fontc
  matches its `.glyphs` build in 822 of 850 glyphs at ExtraLight (the rest explained). Open:
  glyphsLib lib keys, non-master layers to UFO, UFO 2 kerning groups (plan § M1.4).
- **2026-10-08, M1.7, the typed model.** Steps 1 to 8c: font information, axes, masters,
  instances, glyphs, layers, shapes, kerning context, groups and feature markers are model
  fields in `.glyphs` 2 to 4, UFO, designspace and the native file; glyphsLib's lib keys carry
  Glyphs-only data; intermediate layers are sparse sources and alternate layers bracket glyphs
  with rules. Census: `.glyphs` keys in `Extra` from tens of thousands of entries to stored
  values, undocumented keys and hand-made spellings. Measures: round trips unchanged (`.glyphs`
  108/1/132/4, packages 22/0/7/0, UFO 125/15, designspace 59/53); 35 of 35 reference-written
  sources through a designspace and back (new test); fontmake from our Oswald designspace:
  4,614 of 4,626 static glyphs and 882 of 883 variable-font glyphs equal, names and OS/2 equal
  but `xAvgCharWidth` ±1 (non-spacing mark widths need the glyph database, M4). Fixes from the
  measures: slanted component matrices decompose; version 2 name parameters, instance names and
  palettes; virtual masters widen axis ranges; loss reports list the data UFO file records hold,
  not the records. Next: M2.0 and M2.1.
- **2026-10-08, M2.0, the oracle harness.** New crate `crates/oracle` (L3) and `fontcraft-cli
  oracle` (`--corpus`, `--against fontc|fontmake`, `--format ttf|otf`, `--json`): both builds
  static at the same master, fixed timestamp, read back and compared per table, normalised;
  source-reading differences apart; allow-list with reasons. Corpus set 354 sources: fontc built
  334, fontmake 330 (TrueType) and 331 (CFF), FontCraft 313; 300 compared with fontc (per-table
  numbers in the plan). glyphsLib does not read `.glyphs` version 4 (8 of 16 files, none with
  outlines); fontc writes vendor `NONE`. 24 tests (20 in the gate, 4 opt-in). Next: M2.1.
- **2026-10-08, M2.1, pipeline and resolved projection.** `Font::resolve(&Target)` in the model
  (a master, or an instance at a master; between masters an error until M9); `crates/compile`
  split into preflight, order, outline and build stages. Missing components dropped with a
  warning, non-exported glyphs inlined, cycles and depth refused, `.notdef` drawn as fontc draws
  it, composites kept (mixed glyphs and transforms beyond ±2 decomposed), cu2qu at 0.001 em,
  contours reversed, half-up rounding, composite boxes and `maxp` limits. Oracle against fontc:
  FontCraft builds 347 of 354 (was 313), 332 compared (was 300); glyf identical 218 and
  equivalent 18 (was 2 and 0), head 277 (22), maxp 275 (58), hmtx 171 + 132 (1 + 14). 406 tests.
  Next: M2.2.
- **2026-10-08, M2.2.** Core tables derived as fontc 1.0.0 derives them (`crates/compile/src/info.rs`,
  `ranges.rs`, `build.rs`): metric defaults per format at resolve time, names and fallbacks,
  style flags, vendor `NONE` (plan § 3 question 6 answered so), fsType, weight and width class,
  Unicode and code page ranges, `USE_MY_METRICS`, hmtx run compression, vhea and vmtx. Readers:
  version 2 vertical metrics no master states stay undefined, unusual code point and property
  spellings read, `*origin` anchors applied; the binary reader takes the typographic ascender
  and descender as the master's. Harness: vhea and vmtx by field, allow-list with
  several keys, sources and `*`. Against fontc over 332 sources every core table identical,
  equivalent or allowed; round trips: glyphs 122 identical (was 108). Open: four reader
  questions (see Where to start).
- **2026-10-08, M2.3a.** New standalone crate `crates/cff`: CFF tables from plain data, porting
  fontTools' number encodings, INDEX, standard strings and charsets, DICT order and defaults,
  charstring pen rounding, specialiser, width optimiser, BoundsPen FontBBox and the offset
  passes. Cross-check against ufo2ft at `--optimize-cff 1`: 115 of 115 fonts byte-identical
  (17,896 glyphs; tolerances 0, 0.3, 0.5). 45 tests, read back with read-fonts. Not yet used by
  the compiler (M2.3b).
- **2026-10-08, M2.3b.** `Flavour::Cff` builds static OTF: `crates/compile/src/cff.rs` fills
  the `cff` crate as ufo2ft does (names, blue zones and stems from the new `Resolved::zones`,
  `stems_h`, `stems_v`, widths); `maxp` 0.5, `post` 3.0, metrics from charstring bounds.
  Against fontmake over 331 sources: CFF 195 identical / 125 equivalent / 10 allowed / 1
  different; 347 OTF and 347 TTF read back by fontTools without error. Allow-list entries can
  name values (`ours`, `theirs`); fontmake/fontc disagreements allow-listed. fontc and round
  trips unchanged. 8 new tests. Export commands still TrueType only (M2.5).
- **2026-10-08, M2.4a.** New standalone crate `crates/container`: WOFF 1 and untransformed WOFF2,
  both directions, porting fontTools' sfnt.py and woff2.py choices (table order, checksums,
  zlib 6, Brotli 11/22 font mode, DSIG dropped, head bit 11). Corpus round trip 342 of 342 for
  both; fontTools reads all 340 TTF/OTF wrappings back to its own tables; byte identity WOFF 0
  (miniz_oxide is not zlib), WOFF2 19, the rest equal apart from the compressed streams.
  Transformed WOFF2 and collections refused with `Unsupported`. decompile's `woff.rs` moved
  into the crate. 12 new tests. Round trips unchanged.
- **2026-10-09, M2.4b.** WOFF2 `glyf`/`loca` (default) and `hmtx` (on request) transforms, encode
  and decode, ported from fontTools; `file.open`, `font.inspect` and `tables.list` read WOFF2.
  Corpus round trip 342 of 342 for WOFF, WOFF2 untransformed, default and with `hmtx`; 49 of 49
  corpus WOFF2 files read; byte identity with fontTools 21 of 340 (default) and 24 (with `hmtx`),
  the rest differ only in the Brotli stream; fontTools reads all 340 of ours; ours reads
  fontTools' 340 and 339 of 340 (fontTools shortens `hmtx` in Oswald-Stencil).
- **2026-10-09, M2.6.** Follow-ups of M2.2 and M2.3b: no core table differs from fontc or from
  fontmake (TTF and CFF) over the corpus; the rest allow-listed with reasons. fontmake CFF
  byte-identical 195 to 206. The four reader questions answered (`docs/spec/30-source-formats.md`
  › Values a source leaves out). Open: a designspace master missing a metric before one it
  states; alternate and colour glyphs (M9, M11).
- **2026-10-09, M2.5.** `file.exportStatic` (instances at masters, TrueType and CFF, plain, WOFF,
  WOFF2), `file.exportAll`, `font.compile`, `font.compilePlan`, CLI `compile` (`--plan`),
  `convert` to otf, woff, woff2. A failing export writes nothing. Instances between masters
  skipped (M9), overlaps kept with a warning (M3). Not built: the plan's `productionNames` and
  `features` sections, `--name-by`, `--output-layout`, `familyName`, `testInstall`, pattern
  instance selection.
- **2026-10-09, M2 closed.** § 5 measures: core tables and CFF outlines as in M2.6; the 347
  buildable comparison sources give 551 TTF and 551 OTF (masters and instances), all read back by
  skrifa and fontTools without error, all 1,102 round trip through WOFF2 (default and `hmtx`) to
  the same tables, and the 1,102 WOFF2 files `compile` wrote decode to the plain fonts' tables.
  New `crates/engine/tests/determinism.rs`: two sessions, 12 files (TTF and OTF, plain, WOFF,
  WOFF2), byte-identical. Next: M3's planning pass.
- **2026-10-09, M2.7.** Owner decision: a font with no family name (42 corpus sources) is
  exported as `<source file name>-<style>.ttf` instead of fontc's `-<style>.ttf`; only the file
  name, the `name` table keeps the empty family. Smart components can be written to a UFO or
  designspace as outlines (`file.saveAs` `smartComponents`, `convert --smart-components`,
  `file.exportUfo` by default), ported from glyphsLib with fontTools' `VariationModel`
  (`crates/geom/src/variation.rs`, `crates/model/src/smart.rs`): the 10 glyph layers with smart
  components in 7 corpus sources match glyphsLib's points exactly. Follow-up: the compiler can
  draw smart components with the same function and drop their allow-list entries.
- **2026-10-09, M3.0.** The owner accepted the M3 plan. Spec pass: the 42 secondary-reference
  rows moved to M13; super-smooth nodes, pen points, Erase and Select All Layers (12 rows) to the
  new milestone `fidelity` (the interaction-fidelity pass), each with its reason in `notes`; the
  image formats to M11; the export dialogs and `export.removeOverlapOption` from M2 to M3; the
  four `memory` M3 rows settled. New rows `layer.inspect`, `path.addInflections`, `tool.select`,
  `tool.list`. Every M3 command's `params` name their targets (decision 0004 › How commands name
  their targets). The schema gained `when`, `context` and `notes`; `cargo xtask spec` requires a
  `when` on every shortcut without `Cmd` or `Ctrl`. D13: a row a tutorial video confirms is
  `observed`. The § 3 answers are in the notes' open questions. M3 now holds 447 rows (176
  commands, 7 live). Next: the M3.6 spike, then M3.1.
