# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-10-11, M5 closed (M5.18): every § 5 measure met but the two out-of-sync speed targets; the owner's review of M5 next, then M6's planning pass._

## State of the tree

- `cargo xtask ci` (ten steps) was not run in M5.18: it ran `cargo fmt`, clippy on the crates
  it changed, `xtask spec`, `parity` and `guesses`, and the tests it changed. Run the gate once
  before the next push.
  About 168,000 lines of Rust in 25 library crates, two apps and `xtask`. Built and run on Linux
  x86-64 with Rust 1.97.
- 365 of the 933 inventory commands exist (`docs/parity.md`); M3's 184, M4's 62 and M5's 97
  commands are all live. Feature parity is about 18% (ROADMAP.md › Parity estimate). 625 guesses on 394
  rows are listed in `docs/guesses.md`.
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at, and the screenshots of each M3, M4 and M5 UI package were reviewed. macOS and
  Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

## Where to start

1. **The owner's review of M5**: `docs/plans/M5.md` § 5 › Results, and the open items below.
2. **M6's planning pass** (components and anchors): `docs/execution-plan.md` § 3, the M6 rows
   in `spec/inventory/*.toml` and `docs/spec/09-reusing-shapes.md`, written up as
   `docs/plans/M6.md` for the owner to accept.

Open items for the owner from M5:

- The out-of-sync update after an edit to one glyph takes 7.4 ms on Inter against a 2 ms
  target, and the full scan 53.5 ms against 50 ms: accept, or ask for the cache to be reworked.
- 12 metrics-key values in Inter Italic differ from the stored values by up to 1.4 units; the
  cause is not settled.
- Preview Descender defaults to winAscent as the handbook says, probably a misprint; Preview
  Ascender 1000 cuts Inter's capitals.
- Text Preview reverses Latin letters inside right-to-left runs (no shaping before M8).
- Vertical origin and vertical width have no commands and no rows.
- Import Metrics' file choosers.

M5, spacing, kerning and text mode (`docs/plans/M5.md`), as built:

| Done | What it gave |
|---|---|
| M5.0 | spec pass: the rows, the crates registered in the layers, the owner's answers |
| M5.1, M5.2 | `crates/spacing`: metrics keys, sidebearings, the out-of-sync cache, linked masters; the kerning index, groups, exceptions and locks |
| M5.3, M5.4 | the spacing setters, shortcuts, Update Metrics and Transform Metrics; the kerning pair, group, exception and shortcut commands |
| M5.5 | Clean Up, Compress, group rename, the Kerning window's pair list, kerning-text |
| M5.6, M5.7 | `crates/textlayout`; text mode in Edit View, tabs from display strings |
| M5.8 to M5.13 | the Info box's spacing and kerning fields, the Kerning window, Font View spacing, the Preview Area and Panel, the measurement line, Text Preview, sample strings and Find Glyphs |
| M5.14, M5.15 | metrics import and export (kerning-text, AFM, another font); pair kerning recovered from compiled fonts |
| M5.16, M5.17 | linked metrics and Default Layer Width at export; `crates/features`, the generated pair kerning of exported fonts, ported from fontc |
| M5.18 | the § 5 measures (all met but the two out-of-sync speed targets); the kerning lookup measured against both oracles; the spacing and kerning undo sweep; the end-to-end test to the second master |

Left for later milestones: automatically aligned glyphs (M6), shaping and kerning with a
hand-written `kern` feature (M8), vertical kerning at export.


M4, Font View and glyph management (`docs/plans/M4.md`), as built:

| Done | What it gave |
|---|---|
| M4.0 | spec pass: the owner's answers in the rows; licence history of GlyphsInfo (MIT until 2024-11-22, BSD-3-Clause after) |
| M4.1 | `crates/glyphdata`: the glyph database and `cargo xtask glyphdata`, cross-checked against fontc and glyphsLib |
| M4.2 | effective properties, glyph order, sorted placement, Last Changed; production names and database mark widths at export; `post` names compared with both oracles |
| M4.3 | 22 glyph property commands, renames that follow references, Find and Replace |
| M4.4 | Add Glyphs, Duplicate, Remove with its answers, Add to Font, whole-glyph copy and paste, Paste Special |
| M4.5 | `crates/glyphquery` and the search and filter commands |
| M4.6 | the settings store and the app's settings file; CLDR language data and glyph sets (`cargo xtask glyphsets`, sources in git-ignored `data-sources/`); `file.newFromGlyphSets` |
| M4.7 | Font View grid, List View, status area, batch targets |
| M4.8 | inspector, Selection Info, glyph and canvas context menus, Info box colours |
| M4.9 | sidebar, search field, filter dialogs, global guides drawn and hit only where their filter allows |
| M4.10 | Add Glyphs, Remove Glyphs, Paste Special and Find and Replace dialogs; Font View clipboard |
| M4.11 | Glyph Info window, Start Window, recent documents in the engine, `glyphData.glyphSets` |
| M4.12 | the § 5 measures, all met; undo-step sweep; the glyph-set end-to-end test |

Left for later milestones: the inspector's metric and kerning fields (M5), Add Glyphs recipes
(M6), images in cells (M11), renaming and dragging filters in the sidebar.

M3, Edit View drawing (`docs/plans/M3.md`), as built:

| Done | What it gave |
|---|---|
| M3.0, M3.0b | spec pass: targets in every M3 command's params, `when` and `context` fields, D13 answered; 5 editor rows confirmed by tutorial videos |
| M3.1 | engine foundations: `params::Targets`, `Session::begin`/`preview`/`commit`/`cancel`, `layer.inspect`, the glyph lock, `geom` segments, grid rounding |
| M3.2 | the canvas split (`shell/`, `canvas/`), display conventions, zoom and scroll, View toggles, tabs, keymap, cached menus; `crates/render`, `ui.screenshot`, `ui.render`; the toolbar with drawn icons |
| Wave 1 | crates without commands: `tools` (M3.3 core), `audit` (M3.17, 34 of 36 outline checks), `io-svg` (M3.8), `pathops` (M3.6) |
| M3.3 | pointer editing in the window: Select, Lasso, Hand, Zoom, snapping, keymap with `when`, selection commands, `ui.pointer`, `ui.key`, `ui.text`, `ui.tool.*`; context menus, menu items greyed out by the selection, `ui.menu.list`, `ui.menu.invoke` |
| Wave 2 | engine commands: M3.4 nodes and Fit Curve, M3.5 transformations, M3.8 clipboard and SVG import, M3.9 guides, measuring and Dimensions, M3.10 background and layers, M3.16 annotations |
| Wave 3 | M3.6 path-operation commands (each layer under a time limit) and the Knife; M3.11 strokes, masks and shape order in the engine; M3.12 the filter framework (`filter.apply`, `filter.list`, Repeat Last Filter) and the six filters |
| Wave 4 | M3.13 Draw, Pencil, Rectangle, Ellipse and shared tool letters; M3.5 Rotate and Scale tools and the bounding box; M3.9, M3.10, M3.16 on the canvas: guides, grid, measurement, the Measurement tool, background, annotations and the Annotation tools; M3.15 Info box, Palette (Dimensions, Fit Curve, Layers, Transformations), bottom bar |
| M3.14 | the dialog framework: a form over a command's parameters with a live preview, OK one undo step, Cancel restores the font; run-on-OK mode, follow-up questions, shaped fields, "…" actions, reorderable lists, multi-choice and folder fields, tabs. Dialogs: each filter, Transformations, Shape Order, Interpolate with Background, Primitive Measurements, Guide Properties, the missing-glyph and pasted-outline questions, Export (OTF and UFO tabs) |
| M3.7 | overlap removal at export: the compiler's outline stage, each glyph under a time limit, the `removeOverlap` option, coverage-equal comparison in the oracle |
| M3.8 (window) | the system clipboard in Edit View; system choosers for the export folder and File › Import › Outlines |
| M3.9 (window) | guides dragged and edited on the canvas, guide context menu items |
| M3.10 (window) | the active layer (`layers.select`), the tab's master (`view.selectMaster`, Cmd+1 to Cmd+9), Edit Background (Cmd+B) |
| M3.11 (rest) | strokes and masks drawn on the canvas and applied at export; the Palette's Stroke section |
| M3.17 | audit commands (`audit.run`, `list`, `describe`, `results`, `fix`, `profile.list`), eight fixes that each clear their check, `path.addInflections`, Tidy up Paths' `straighten`; live audit and canvas markers; `cargo xtask audit` |
| M3.18 | the § 5 measures, all met (184 of 184 commands live); the end-to-end test (an agent draws H, O and n over the control channel and exports them) |

Moved to `fidelity`: `path.sharpenCorner` (no source). Left for later milestones: text mode
(M5), the toolbar's master icons (M9), the Export dialog's
Variable (M9) and Metrics (M5) tabs.

Decisions that hold:

- linesweeper 0.5.0 stays, from crates.io, and its rare hang and failed assertion are not
  reported upstream for now (owner's decision, plan § M3.6 "Spike result"). Callers that run
  without a person waiting (export, batch commands) run each glyph under a time limit.
- Behaviour not yet seen in the reference app or a video follows the handbook and is the
  default; the row's `notes` say it is documented but not observed. Example: Remove Overlap
  deletes open paths and stray nodes.
- Points a command creates or moves round to the font's grid (`path.addPath` included).

M1 and M2 are done (`docs/plans/M1.md`, `docs/plans/M2.md`). M2 left on purpose: instances
between masters (M9), CFF subroutines (before M12), the compile plan's `features` section, the CLI's `--name-by`, `--output-layout` and pattern
instance selection, and one reader case (a designspace master missing a metric before one it
states reads it as 0). `fontcraft-cli oracle --corpus` (fontc, or fontmake with `--format otf`)
measures the compiler; `crates/oracle/allow.toml` holds every accepted difference with its reason.

Before you start: `cargo xtask corpus --fetch` (about 300 MB) if `corpus/` is missing, and the
oracles in `docs/testing.md` (fontc via cargo, fontmake via a Python tool installer).
`cargo xtask roundtrip` shows where the round trips stand; `cargo xtask overlap` measures
overlap removal on the corpus; `cargo xtask audit` runs the outline audit on it.

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
- **30 rows are from memory** (`docs/parity.md` › By source): 12 TrueType autohinter options in
  `14-…`, 5 each in `00-…` and `90-…`, 2 each in `05-…` and `30-…`, 1 each in `07-…`, `09-…`,
  `10-…` and `16-…`. Verify or drop them before implementing.
- **The reference app has not been run.** 52 rows are `observed`: 47 compiled-output rows
  checked against fontc and fontmake, and 5 editor rows confirmed by public tutorial videos.
  About 450 open questions are listed at the ends of the notes.
- **No M3 behaviour has been compared with the reference app.** Tools, panels and dialogs
  follow the handbook; 5 editor rows are confirmed by tutorial videos.
- **M3 gaps** (ROADMAP.md › What is not): `path.sharpenCorner` waits in `fidelity`; Font View draws
  the first master only (M4); the Export dialog has OTF and UFO tabs only; the Import Outlines
  missing-glyph question is asked once for all unmatched files, not once per file; Shape Order
  and Expand Outline do not renumber hints that refer to a path by index; a UFO keeps no stroke
  attribute, and cap code 4 and the forced cap axis are unconfirmed against the reference's
  files.
- **linesweeper can hang** on rare inputs (plan § M3.6 "Spike result"). The path-operation
  commands and export run each glyph on a worker thread under a time limit; a glyph that runs
  out of time stays as it was (at export, it keeps its overlaps, with a warning), and its worker
  cannot be stopped and runs on detached. The 10,000-case random-path test in `crates/pathops`
  is ignored in the gate.
- **The compiler writes static fonts only.** TrueType and CFF, plain, WOFF and WOFF2; the core
  tables match fontc and fontmake over the corpus (with allow-listed causes: variable references,
  bracket and colour glyphs, the glyph database, smart components); overlaps are removed, strokes
  expanded and masks applied at export; no features, variations, hinting or CFF subroutines yet
  (`docs/plans/M2.md` § 5, `docs/plans/M3.md` § 5).
- **`docs/spec/20-cli.md` is very long** (about 26,000 words). Read the section for the subcommand
  you are building, not the whole file.

## Known limits in the skeleton's code

An adversarial review of the skeleton reported 31 findings. Most were fixed in the same session
(coordinate and point-count bounds before the glyph writer, glyph-name sanitising for `post`,
style linking, bounded reads on every channel, protocol id checks, the indexing lints, a
virtualised Font View). These were left on purpose. None can lose data.

| Limit | Where | Fix it in |
|---|---|---|
| Each command's rollback snapshot copies every document's history list (pointers and labels, no glyph data) | `crates/engine` | when it matters |
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
- **2026-10-10, M3 waves 1 and 2, M3.1, M3.2.** M3.6 spike: linesweeper 0.5.0 unions all 2,368
  corpus layers with closed paths to skia-pathops' coverage; kept from crates.io with input
  rules and a per-glyph time limit, no upstream report (owner). Wave 1 crates: `tools`,
  `audit` (34 of 36 outline checks), `io-svg` (deep nesting fenced before the XML parser),
  `pathops`. M3.1: targets, interactions, `layer.inspect`, glyph lock, segment geometry.
  M3.2: `render`, `ui.screenshot`, `ui.render`, canvas and shell split, tabs, zoom, cached
  menus. Wave 2: node, transform, clipboard, guide, measure, Dimensions, background, layer and
  annotation commands. Remove Overlap deletes open paths and stray nodes, as the handbook says.
  679 tests; 142 of 929 commands live (M3: 114 of 181). Next: the M3.3 wiring.
- **2026-10-10, M3 waves 3 and 4, the dialog framework.** Path-operation commands on `pathops`,
  each layer under a time limit, and the Knife; strokes, masks and shape order in the engine;
  the filter framework and all six filters; dialogs with live preview and Cancel, one per filter
  from its schema; Draw, Pencil, Rectangle, Ellipse, Rotate, Scale, Measurement and the six
  Annotation tools (18 tools in the window); the bounding box; guides, grid, measurement,
  background and annotations on the canvas; Info box, four Palette sections, bottom bar. 854
  tests; 200 of 932 commands live (M3: 172 of 184); about 102,600 lines of Rust. Open: audit
  commands and the other dialogs (on branches), M3.7, strokes and masks on the canvas and at
  export, live audit, context menus, M3.18.
- **2026-10-10, M3 close.** Since waves 3 and 4: M3.7 overlap removal at export, each glyph
  under a time limit; strokes and masks drawn and exported, the Stroke section; the rest of the
  dialog framework and the Transformations, Shape Order, Guide Properties and Export (OTF, UFO)
  dialogs; the toolbar, context menus and menu greying; the active layer, master switching and
  Edit Background; live audit, markers and the audit commands; the system clipboard. § 5
  measures: all met but "commands live", 184 of 185 (`path.sharpenCorner`: no source; moving it
  to `fidelity` awaits the owner). Overlap removal: 2,368 of 2,368 corpus layers within the
  coverage rule, median 38 µs; against fontmake 99.58% (TrueType) and 99.87% (CFF) of glyphs
  without an overlap-removal difference. Node-drag preview step on Oswald 142 µs. Audit over 348
  corpus sources: 0 failures. Round trips and oracle equal the baselines. 931 tests; 213 of
  932 commands live; about 113,000 lines of Rust. Next: the owner's review of M3, then M4's
  planning pass.
- **2026-10-10, owner's review of M3.** The owner accepted the agents' choices where the
  handbook is silent, except Extrude's export string: Extrude has none until a source shows
  one (`filter.apply` refuses it). `path.sharpenCorner` moved to `fidelity`, so M3 is closed
  with 184 of 184 commands live. Every choice of ours where no source says what the reference
  does is a row's `guesses` entry; `cargo xtask guesses` writes `docs/guesses.md` (203 on 160
  rows) and fails when a live `inferred` or `memory` command has none; `own = true` marks rows
  with no reference counterpart. 934 tests. Next: M4's planning pass.
- **2026-10-10, M4 planning pass.** `docs/plans/M4.md` proposed: 244 rows (61 commands, 6
  live; 155 behaviours, 24 panels); 19 rows moved in (production names from M2, `glyphOrder`
  from M7, settings and recent documents from M14); no `memory` rows left; 16 of the Font View
  note's 20 open questions answered. Waits for the owner.
- **2026-10-10, M4 wave 1.** Plan accepted. M4.1: `crates/glyphdata` (GlyphsInfo 1f126bd, AGL,
  UCD 17.0; 538 KB blob; `cargo xtask glyphdata --fetch|--generate|--check`): equal to fontc's
  glyphs-reader on category, subcategory and production name for 200,504 names; glyphsLib
  differences allow-listed by cause. M4.2: effective properties, `glyph_order`, sorted placement
  (`insert_by_sort_key`), Last Changed; production names at export (fontc's cleaning), database
  marks at advance 0, File › Export refuses invalid names; `post` names equal fontc's and
  fontmake's except CornerComponents (Use Custom Naming, allow-listed); the M4 marks allow-list
  is gone. M4.3: 22 glyph property commands, renames that follow references, Find and Replace
  with regex. M4.4: Add Glyphs, Duplicate, Remove with its answers, Add to Font, whole-glyph
  copy and paste, Paste Special. M4.5: `crates/glyphquery` (search, smart and list filters,
  predicates, sidebar entries, missing glyphs) and 11 commands. M4.6: settings store and the
  app's settings file. 1,021 tests; 252 of 932 commands live (M4 45 of 61).
- **2026-10-10, M4 close.** Wave 2 (M4.8 to M4.11) built in worktrees and integrated; the dialog
  framework gained multi-line text, a third button, scrolling lists and a rule-tree control;
  engine follow-ups: Match Case, several tags in one step, recent documents in the engine, an
  empty glyph data search, `glyphData.glyphSets`, readable script names. M4.12: every § 5 measure
  met (62 of 62 commands; glyph database equal to fontc's; `post` names equal both oracles'
  except one allow-listed source; dialogs equal commands; the glyph-set end-to-end test; on Inter
  a Font View frame under 1 ms). 1,076 tests; 269 of 933 commands live; about 138,000 lines of
  Rust. Next: the owner's review of M4, then M5's planning pass.
- **2026-10-10, M4 review and M5 planning.** The owner's answers to M4's guesses: hidden selected
  glyphs are deselected; a rename takes the database's Unicode value; generated glyphs are 600
  units wide; removing a glyph keeps the kerning pairs that name it, as the handbook says
  (M4.16). M4.14: Remove Glyph's dialog defaults
  to Decompose. M5.0: the planning pass (222 rows, 97 commands; 27 rows
  moved; 19 packages; 463 guesses on 328 rows). Next: the owner's acceptance of the M5 plan.
- **2026-10-11, M5 started.** The owner accepted the M5 plan: the pair-kerning writer comes into
  M5 (M5.17), tabs open from the document's display strings, Text Preview uses our own layout.
  M5.0: the answers and the handbook's kerning details in the rows (5 kern-export rows moved from
  M8; 463 guesses on 328 rows).
  M5.1: `crates/spacing` parses metrics keys, measures sidebearings (italic masters along the
  angle, as the handbook says), evaluates keys with limits, scans for out-of-sync keys with a
  cache, orders Update Metrics and resolves linked masters. Corpus: 4,413 of 4,413 keys parse;
  17,217 master values equal the stored metric; 12 Inter Italic differences up to 1.4 units are
  not explained. Open: the cache update after editing a much-used glyph takes 7.7 ms (target 2).
  M5.2: the kerning core (sides, lookup with its step, locks, groups); lookup over Inter Regular's
  490,911 covered pairs takes 52.6 ms. M5.3–M5.5: 48 spacing and kerning commands (setters with
  keys, shortcuts, Update Metrics, Transform Metrics, Default Layer Width, pairs, exceptions,
  groups, Clean Up, Compress, rename, the pair list, kerning-text copy and paste). Update Metrics
  orders composites after their bases and settles Inter in one run (about 130 ms). Compress
  follows stored order so the handbook's example settles. M5.6: `crates/textlayout`; 45 of 45
  corpus display strings round trip; 10,000 items lay out in 2.15 ms. M5.15: kerning recovered
  from compiled fonts, 945,310 covered pairs in 97 fontmake builds with 0 differences.
  317 of 933 commands live.
  M5.fix1: seven review fixes (a work budget on the GPOS script walk, closing one kerning lock,
  control characters in kerning sides, duplicate master entries, Compress with one group in two
  spellings, GPOS classes past the count, paste values like Set Pair). M5.kernmerge: loaders keep
  one kerning entry per master. M5.16: exported masters that link their metrics take the linked
  master's widths (17 of 17 master builds equal fontmake's hmtx and vmtx); new glyphs take
  Default Layer Width (1000 for CJK, 200 for `_corner.` glyphs without the parameter).
  M5.7: text mode in Edit View. Tabs hold typed text (cursor, selection, direction, kerning
  mode); the Text tool (T), typing, cursor keys, Escape between text entry and editing,
  Cmd+Space panning, copy and paste as text; the canvas draws the caret, the selection, kerned
  gaps (negative light blue, positive yellow) and metrics labels; clicks place the cursor; the
  bottom bar's direction and kerning-mode buttons; spacing and kerning shortcuts act at the
  cursor; tabs open from the document's display strings and are written on save. The § 5
  end-to-end test runs up to the Kerning window step. A frame with 10,000 items takes about
  8–15 ms (target 16). 329 of 933 commands live.
  M5.fix2: tabs belong to their document (each saves only its own display strings); the text
  layout follows a picked layer. M5.8–M5.14: the Info box's spacing and kerning fields (numbers
  and keys, out-of-sync red with an update button, unlinked turquoise, kern fields and locks),
  the Kerning window (search, sort, in-place editing, copy and paste between masters), the rename
  and paste-conflict dialogs, Show Group Members, Font View's editable metric and group fields
  and out-of-sync badge, three smart-filter rules, the Transform Metrics dialog, the Preview Area
  and Panel, the measurement line, Text Preview (1,000,000 characters in about 360 ms), sample
  strings, Find Glyphs and `glyph.search`, metrics import and export (kerning-text; AFM read back
  by afmLib both ways on Inter). M5.syncfast: after a rename the out-of-sync cache rescans only
  the glyphs that name it; Font View frames after an edit on Inter take 6–10 ms. Open: the cache
  update after editing `n` takes 7.2 ms (target 2). 365 of 933 commands live; M5's 97 all live.
  M5.fix3: nine review fixes (Replace All and AFM expansion refused before allocating; AFM import
  leaves italic masters unchanged; nested composites import after their components; menu
  shortcuts work while a list has the keyboard; Selected Glyphs works in Edit View tabs; the Find
  Glyphs cache follows the document revision; dialogs open during a drag).
  M5.17: exported fonts get generated pair kerning (GPOS `kern` and `dist`) for sources without
  feature code, ported from fontc 4504578 into `crates/features`, following the ufo2ft
  feature-writer setting. Against the oracles on the 37 corpus sources without feature code:
  129 pairs each, 0 differences with fontmake, 1 known difference with fontc (written in
  `docs/spec/40-compile.md` § 4a). Sources with feature code (21 kerned ones) get no generated
  kerning until M8.
- **2026-10-11, M5.18, M5's close.** The kerning lookup against both oracles
  (`index_lookup_equals_both_references`): 54 sources without a hand-written `kern` feature,
  506,285 pairs per oracle, 1 difference (`FeatureWritersEmpty.glyphs` turns its kern writer
  off); 5 sources with one, 5 pairs, 1 difference (`FeaInc-Bold.ufo`'s included feature). Round
  trips equal the M4 baselines (glyphs 122/1/118/4, packages 22/0/7/0, UFO 125/15, designspace
  59/53). New sweep: 45 undo runs over 42 spacing and kerning commands, 11 refusals pointer-equal.
  The end-to-end test reaches the Kerning window, Compress and the import into a second master.
  Speed on Inter (load 14 to 19): out-of-sync full scan 53.5 ms (50), after editing `n` 7.4 ms
  (2), both missed; every other target met. Inter's `Tcommaaccent` (U+0162) and `uni021A`:
  the references write `uni021A` and `uni021A.1` because they ignore Inter's `disablesNiceNames`;
  FontCraft keeps the source names as documented. M5 is done but those two speed targets.
