# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-10-08, M2.0 done (the oracle harness; first corpus numbers in `docs/plans/M2.md`)._

## State of the tree

- `cargo xtask ci` passes all nine steps (387 tests; about 43,800 lines of Rust in 13 crates).
  Built and run on Linux x86-64 with Rust 1.97.
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at. macOS and Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

## Where to start

**M2, the compile core** (`docs/plans/M2.md`, accepted): M2.0, the oracle harness, is done.
Next is M2.1 (the pipeline and the resolved projection), then M2.2 (core tables to fontc
parity). Use `fontcraft-cli oracle --corpus` (`docs/testing.md` › The oracle harness) to measure
each change: the plan's M2.0 section lists the first numbers and what the differences are, by
cause and package. Two answers came out of M2.0: glyphsLib does not read `.glyphs` version 4
(nor does fontc), and fontc 1.0.0 writes the vendor id `NONE`, not four spaces, so the plan's
answer to question 6 needs the owner again before M2.2 changes it.

M1 (`docs/plans/M1.md`) so far:

| Package | State |
|---|---|
| M1.0 corpus, harness, spikes | done (plan § 8) |
| M1.1 property lists | done: OpenStep and XML |
| M1.2 the model, with M1.3 `.glyphs` | done; M1.7 typed the rest. `crates/io-glyphs`: versions 2 to 4, files and packages, every reference-written source identical |
| M1.4 UFO and designspace | done: `crates/io-ufo`, UFO 2 and 3, designspace, UFOZ read; every readable corpus UFO and designspace identical; norad reads our output. Gaps in the plan's M1.4 section |
| M1.5 compiled fonts | done |
| M1.6 commands and CLI | done: open, save, save as and export for every source format, batch, document switching, inspection queries, CLI `info`, `tables list`, `glyphs list`, `convert` (`dump` moved to M12) |
| M1.7 the typed model | done: every documented field of the three formats is a model field; Glyphs-only data in glyphsLib's lib keys; intermediate and alternate layers in a designspace. Measures met: round trips unchanged; census explained; 35 of 35 reference-written sources through a designspace and back; fontmake builds of Oswald equal except non-spacing mark widths (plan § M1.7) |

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
- **Our compiler matches the reference compilers in few tables yet.** Over 300 corpus sources
  compared with fontc, cmap is identical or equivalent in 286 and post in 191; head, hhea, OS/2,
  name and glyf differ in nearly all (`docs/plans/M2.md` › M2.0). M2.1 and M2.2 close that.
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
| `OS/2` Unicode ranges: only two bits are set | `crates/compile` | M2 |
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
