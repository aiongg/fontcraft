# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-10-08, M1.7 step 5 done (layer fields; layers in UFO and designspace)._

## State of the tree

- `cargo xtask ci` passes all nine steps (284 tests; about 31,600 lines of Rust in 12 crates).
  Built and run on Linux x86-64 with Rust 1.97.
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at. macOS and Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

## Where to start

**M1.7, the typed model** (`docs/plans/M1.md` § M1.7, added 2026-10-08 at the owner's request):
every documented field becomes a model field, and Glyphs-only data travels in glyphsLib's UFO lib
keys. Eight steps; `cargo test -p fontcraft-engine --test extra_census -- --ignored --nocapture`
shows what is still untyped. After it, M2 (`docs/plans/M2.md`, accepted): M2.0 and M2.1 first;
check whether glyphsLib reads `.glyphs` version 4.

M1 (`docs/plans/M1.md`) so far:

| Package | State |
|---|---|
| M1.0 corpus, harness, spikes | done (plan § 8) |
| M1.1 property lists | done: OpenStep and XML |
| M1.2 the model, with M1.3 `.glyphs` | the fields M1.0 to M1.6 needed are typed; the rest of the outline in `docs/spec/30-source-formats.md` is M1.7. `crates/io-glyphs`: versions 2 to 4, files and packages, every reference-written source identical |
| M1.4 UFO and designspace | done: `crates/io-ufo`, UFO 2 and 3, designspace, UFOZ read; every readable corpus UFO and designspace identical; norad reads our output. Gaps in the plan's M1.4 section |
| M1.5 compiled fonts | done |
| M1.6 commands and CLI | done: open, save, save as and export for every source format, batch, document switching, inspection queries, CLI `info`, `tables list`, `glyphs list`, `convert` (`dump` moved to M12) |
| M1.7 the typed model | **in progress**: steps 1 to 4 done (font information everywhere; `.glyphs` stems, numbers, guides, zones, version 2 axes and name parts, instance fields; UFO blue values, stem snaps and guidelines; designspace axes, maps, instances and variable fonts; glyph metadata, with UFO notes, mark colours, production names and `GDEF` classes); step 5 done (layer fields in `.glyphs` and the native file; backgrounds, other layers, guides, images, vertical metrics and user data in UFO by glyphsLib's rules); step 6 (shapes) next (plan § M1.7 › Progress) |

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
- **No oracle has been run against our compiler.** The exported font was read back with skrifa
  and fonttools, which proves it is well-formed, not that it matches a reference compiler.
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
- **2026-10-08, M1.7 steps 1 and 2.** Parameters, properties, user data, settings and the date are
  model fields. UFO `fontinfo.plist` maps to them through a crosswalk ported from glyphsLib
  (`crates/io-ufo/src/fontinfo.rs`, `docs/spec/31-glyphslib-crosswalk.md`); `lib.plist` is the
  master's user data. Round trips unchanged; `.glyphs` → designspace keeps every master's font
  information except what the loss report lists. Next: step 3 (axes, masters, instances).
  Step 3a: `.glyphs` stems, numbers, master guides, extra alignment zones, version 2 axes (from
  `Axes` or synthesised), axis values, name parts and instances, and the instance fields are
  model fields, in the native file too; defaults a file writes anyway stay in `Extra`. Census:
  these keys drop from 781 entries to 38 (12 of them version 2 `baseline = 0`), each explained
  in the plan. Round trips unchanged. Next: 3b, 3c.
  Step 3b: UFO blue values, stem snaps and guidelines are master overshoots and zones, stems and
  guides, by glyphsLib's rules (`crates/io-ufo/src/design.rs`); `Guide` gained `color` and
  `identifier`. Our UFOs of three `.glyphs` sources hold what glyphsLib writes. Kept as written:
  blue values in 3 corpus sources, stem snaps in 4 designspaces. Round trips unchanged. Next: 3c.
  `.glyphs` versions 3 and 4 now report the zones they cannot hold.
  Step 3c: designspace axes (range, discrete values, localised names, STAT labels), maps (as the
  `Axis Mappings` parameter), instances and variable fonts are model fields, read and patched in
  the kept document (`crates/io-ufo/src/designspace/`); axis user locations and maps by
  glyphsLib's rules in `crates/model/src/axes.rs`. Every corpus designspace document reads and
  writes back unchanged (112, UFOs or not); designspace → `.glyphs` → designspace keeps axes and
  instances in all 59 readable ones, except localised axis names (13 axes, reported).
  Round trips and census unchanged. Next: step 4.
  Step 4: glyph metadata (production name, classification, case, direction, metrics keys,
  colour, tags, note, lock, last change, sort names, smart-glyph properties, user data) is
  `Glyph::info`, behind one pointer; `.glyphs` versions 2 to 4 and the native file type it. UFO
  notes, mark colours, `public.postscriptNames` and `public.openTypeCategories` read into and
  write from it, by glyphsLib's rules; the rest is in the loss report until step 8. Census:
  glyph keys from 10,808 entries to 25 (explained in the plan). Round trips unchanged. Our UFOs
  of seven `.glyphs` sources hold glyphsLib's notes, colours and production names. Next: step 5.
  Step 5a: a layer's name, attributes (intermediate, alternate, colour, smart poles), vertical
  metrics, metrics keys, colour, visibility, guides, hints (stored), annotations, image,
  background and user data are `Layer::info`, behind one pointer; `.glyphs` versions 2 to 4 and
  the native file type them (`crates/io-glyphs/src/layer.rs`). Census: layer keys from 21,201
  entries to none. Round trips unchanged. Saving as UFO lists the layer data as lost. Next: 5b.
  Step 5b: UFO `public.background` is the master layer's background, other UFO layers are named
  layers and `<name>.background` their backgrounds; `.glyphs` backup, intermediate and colour
  layers go to UFO layers named by glyphsLib's rule; GLIF guidelines, images, heights, vertical
  origins (glyphsLib's vertical-font rule) and lib keys are layer fields. Alternate layers and
  intermediate locations (designspace rules and sources) are reported, not written. Every corpus
  UFO and designspace unchanged; our UFOs of six `.glyphs` sources hold glyphsLib's layers.
  Foreign raw shapes are now left out of a GLIF instead of failing the save. Next: step 6.
