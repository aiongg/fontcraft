# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-10 (M3 waves 1 and 2: engine foundations, canvas and screenshots, the node, transform, clipboard, guide, layer and annotation commands)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,638 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, tests | **~45%** of what the siblings' "infrastructure" row covers; missing: pointer and key injection, the dialog framework, panels |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~7%** |
| Commands that exist (not: that behave like the reference) | **200 of 932** (`docs/parity.md`) |
| Observed against the running reference | **52 rows**: 47 compiled-output rows checked against fontc and fontmake, and 5 editor rows confirmed by public tutorial videos of the previous major version. The reference app itself has not been run |

## Honest assessment (2026-10-10)

**In one line:** the project reads and writes `.glyphs`, UFO and designspace sources without loss, opens compiled fonts and edits outlines through commands; nobody can draw in its window yet.

**What is real: files and the compiler.** `.glyphs` files of versions 2, 3 and 4 and `.glyphspackage` folders open and
save back byte for byte (every corpus source the reference app wrote, Inter and Oswald among
them), and either form saves as the other. UFO 2 and 3 and designspace documents with their
UFOs open as one document and save back byte for byte (every readable corpus file: 125 UFOs, 59
designspace documents); a save rewrites only the files an edit touched, and norad reads what we
write. A `.glyphs` document saves as UFO or designspace and back: every reference-written
source comes back with the same model except what the loss report lists, and fontmake builds
Oswald from our designspace with the same outlines (but the advance widths of non-spacing marks),
names and OS/2 values as from the `.glyphs` source.
Compiled TrueType, CFF, collection, WOFF 1 and WOFF2 fonts
open view-only (all 391 corpus fonts). The native `.fontcraft` format holds everything a `.glyphs`
source holds. A new font can be created, glyphs added, removed and encoded, widths and font info
set, everything undone and redone, several edits batched into
one undo step, and static fonts exported with TrueType or CFF outlines, plain or as WOFF or
WOFF2, for every instance that sits at a master (or for the masters), named as the reference
names them, from `file.exportStatic`, `file.exportAll`, `font.compile` and `fontcraft-cli
compile`. Built from the 347 corpus sources FontCraft compiles, all 1,102 TTF and OTF fonts read
back in skrifa and fontTools without an error, wrap and unwrap as WOFF2 to the same tables, and
the same source and timestamp give the same bytes. `fontcraft-cli oracle` compiles a source with
FontCraft and with fontc or fontmake and compares the two fonts table by table; over 354 corpus
sources it measures the compiler: 332 compared with fontc, and every core table identical,
equivalent or allow-listed with a reason in all of them; 331 compared with fontmake's CFF builds,
whose outlines FontCraft's match in 322 (the other 9 are source reading, allow-listed).
`crates/container` writes and reads WOFF 1 and WOFF2 with the `glyf`/`loca` and `hmtx`
transforms; fontTools reads all of its output back.

**What is real: editing by command (M3).** 200 of the 932 commands exist, 172 of them from M3.
Every editing command takes explicit targets (shapes, nodes, segments, the foreground or the
background) and runs the same from `fontcraft-cli run`, MCP and the control channel into the
running window:

| Family | Commands |
|---|---|
| Nodes and segments | move and position a selection, node types, convert a segment to a curve, delete with or without breaking the path (also in all masters), insert and connect nodes, close open paths, make a node first, reverse contours (also in all masters), lock paths and glyphs, rectangles and ellipses, Fit Curve |
| Transformations | the Palette's mirror, scale, rotate, slant and align buttons, Align Selection, Path › Transformations and its export string |
| Clipboard and SVG | copy, cut and paste (our JSON fragment and SVG), SVG files imported into glyphs |
| Guides and measuring | local and global guides, the ruler and coordinate measures, Dimensions |
| Background and layers | the background commands, Interpolate with Background, backup layers, layer visibility, order and names |
| Annotations | add, update, move, delete, list |

An edit that a drag will drive is an interaction: any number of previews, then one commit and
one undo step. Points a command creates or moves round to the font's grid. `layer.inspect`
reads a layer's outline with addresses. Edit View draws nodes and handles by the reference's
display conventions, metric lines and a fill preview; it zooms, scrolls and holds several tabs.
`ui.screenshot` captures the window over the control channel; `ui.render` draws glyphs to PNG
without a window, the same bytes every time. Three crates have no commands yet: `tools` (the
Select, Lasso, Hand and Zoom tools, hit testing, snapping, the Edit View keymap), `pathops`
(booleans, Remove Overlap, Correct Path Direction, extremes, Tidy up, Open Corner, Reconnect,
Cut with Line, on linesweeper) and `audit` (34 of the 36 outline checks). 854 tests pass,
including a sweep that runs every engine command with hostile parameters. The whole gate
(`cargo xtask ci`: format, lints, tests, spec, parity, assets, vendor names, layering, wasm) is green.

**What is not.** In the window, the Select, Lasso, Hand and Zoom tools edit outlines with the
pointer (M3.3), but there are no drawing tools, no context menus, and menu items are not greyed
out by the selection. From the
window's menus only the commands that need no parameters work: there are no dialogs, no Palette
sections and no Info box. There are no path-operation commands (booleans, Remove Overlap,
extremes, Tidy up…), no overlap removal on export (overlaps are kept with a warning until M3.7),
no strokes, masks or filters, and no audit commands. No M3 behaviour has been compared with the
reference app; where a detail is unconfirmed, it follows the handbook (Remove Overlap, for
example, deletes open paths). There is no kerning editing, no component editing (components are
exported as composites), no feature compilation, no variable fonts and no instances between
masters (M9), no CFF subroutines, no hinting, no web app, no packaging. Glyphs-only data travels
in glyphsLib's UFO lib keys where glyphsLib has a key for it, and alternate and intermediate
layers travel as designspace rules and sparse sources; the data no key holds (glyph tags, locks,
sort names, layer colours, drawing attributes…) does not, and the loss report lists it.
Production names and the widths of non-spacing marks need a glyph database (M4). Kerning,
features and instances are read and written, not yet edited or compiled.

**How much to trust the inventory.** It was compiled from public documentation in one day, each
source read once. Of its 3,638 rows, 2,106 come from the primary reference's handbook, 1,055 from
other public documentation or permissive source, 395 are our own design, 30 rest on
recollection, and 52 are observed: 47 compiled-output rows checked against fontc and fontmake,
and 5 editor rows confirmed by public tutorial videos. **No other row has been checked against
the running reference app.** An independent re-reading of 209 sampled handbook rows against the
handbook found 1 wrong and 14 imprecise (`docs/spec/README.md`). The notes list about 450 open
questions the documentation does not answer. Treat rows as a careful first reading, and each
milestone's rows as due a planning pass before they are built (`docs/execution-plan.md` § 3).

### By dimension

| Dimension | Estimate | Evidence and what is missing |
|---|---|---|
| **Breadth:** commands, tools and panels exist | ~8% | 200 of 932 commands; 14 of 33 tools usable in the window; 2 of 170 panels as stubs |
| **Depth:** features behave like the reference | ~3% | the M3 commands behave as the handbook describes them, but only through the CLI, the control channel and MCP, and none has been compared with the reference app |
| **File interop** | ~38% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFOZ and compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts); static TTF and OTF out, plain, WOFF and WOFF2, from the export commands and the CLI. Missing: variable fonts, instances between masters, version conversion, a glyph database for production names |
| **Compiler correctness** | measured, low | 13 of 50 tables written, for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, non-spacing marks only the glyph database knows (M4), smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). No variable font, no features, no hinting, no subroutines |
| **Agent automation** | ~45% | every existing command is reachable from CLI, control channel and MCP; `ui.screenshot` and `ui.render` show the result; no pointer or key injection, dialogs, MCP resources or prompts yet |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and overlap removal at export (M3.7) is to run each glyph under a time limit. One adversarial code review, most findings fixed and the rest listed in `docs/status.md`. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | unmeasured | targets written down (`docs/architecture.md`); no budget suite |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: libraries check, no web app |
| **Look and feel** | not started | a functional shell with the siblings' default dark tokens; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

The rest of **M3, Edit View drawing**, in the order of `docs/plans/M3.md` § 6:

1. **Wave 3**, in parallel: the path-operation commands and the Knife tool on `pathops`
   (M3.6), strokes and masks (M3.11), and the filters, one each (M3.12).
2. **M3.7, overlap removal at export**, with a per-glyph time limit; the M2 warning goes away.
3. **Wave 4**: the drawing tools (M3.13), the tool files of transforms, guides and annotations,
   the Palette sections and Info box (M3.15), and the audit markers and commands (M3.17).
5. **Wave 5**: the dialog framework, then one dialog each (M3.14).
6. **M3.18, close**: the measures of `docs/plans/M3.md` § 5.

Then M4–M7 on the editor track and M8–M12 on the engine track, as `docs/execution-plan.md` § 4
orders them. Left from M2 for later milestones: instances between masters (M9), CFF
subroutines (before M12), production names (M4), the compile plan's `productionNames` and
`features` sections, and the CLI's `--name-by`, `--output-layout` and pattern instance
selection (`docs/plans/M2.md` › M2.5).

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (929 commands, 33 tools, 170 panels, 45 settings, 241
  parameters, 1,751 behaviour rules, 300 checks, 70 CLI rows, 49 formats, 50 tables) and 23
  behaviour notes (about 150,000 words), all in our own words. Every M3 command's `params` name
  its targets; shortcuts carry a `when` (the context they apply in).
- **Workspace:** 20 library crates in enforced layers, two apps, `xtask`; about 102,600 lines of Rust.
- **Model:** fonts, masters, glyphs, layers, paths, components, anchors; `Arc`-shared glyphs;
  unknown keys preserved; invariants.
- **Engine:** sessions, per-document history, the command registry, atomic failure, the panic
  guard, the journal. One target resolver for every command (`params::Targets`: shapes, nodes,
  segments, anchors, guides, the foreground or background side, with caps on every list).
  Interactions (`Session::begin`, `preview`, `commit`, `cancel`): previews change the font
  without history, the commit is one undo step and one journal entry, cancel restores the
  snapshot. The glyph lock refuses edits with a message. `layer.inspect` returns a layer's
  shapes, nodes, anchors, guides and annotations with their addresses. `geom` has segments with
  addresses, nearest points, splits, extremes, inflections and grid rounding.
- **Editing commands (M3 waves 1 and 2):** node and segment commands and Fit Curve (M3.4);
  the Palette transforms and alignments, Align Selection, and Path › Transformations with its
  export string (M3.5); copy, cut, paste and SVG outline import (M3.8); local and global guides,
  `measure.ruler`, `measure.coordinates` and Dimensions (M3.9); the background commands,
  Interpolate with Background and the layer commands (M3.10); annotations (M3.16). Points a
  command creates or moves round to the font's grid. Each family has behaviour tests on
  synthetic glyphs, and the command sweep covers every command.
- **Rendering:** `crates/render` (tiny-skia): a layer or a line of glyphs to RGBA and PNG,
  nonzero fill, anti-aliased, the same bytes for the same input.
- **Path operations:** `crates/pathops`: union, difference and intersection on linesweeper,
  Remove Overlap (open paths and stray nodes removed, as the handbook says), Correct Path
  Direction, Add and Force Extremes, Tidy up Paths, Open Corner, Reconnect Nodes, Cut with
  Line, and a coverage comparison. Input is checked and capped before linesweeper sees it.
  Union matches skia-pathops' coverage on all 2,368 corpus layers with closed paths (`cargo
  xtask overlap`). linesweeper 0.5.0 is kept from crates.io and its rare hang is not reported
  upstream for now; the rule for callers that run without a person waiting (export, batch
  commands) is a time limit per glyph (`docs/plans/M3.md` › M3.6, rule 8). No command calls the crate yet.
- **Tools:** `crates/tools` (no egui): the Select, Lasso, Hand and Zoom tools as state machines
  that emit interactions, hit testing of nodes, handles, segments, anchors, components and
  guides, snapping, the selection and its target object, the Edit View keymap and temporary
  tools. Not yet wired into the canvas.
- **Audit:** `crates/audit`: the check registry and 34 of the 36 outline checks, with
  severities, thresholds, fixes by command id and the result document of `docs/spec/21-qa.md`
  § 4. No `audit.*` command yet.
- **SVG and clipboard:** `crates/io-svg` (usvg, behind size, nesting and number limits): SVG
  geometry to paths, paths to SVG, and our JSON clipboard fragment.
- **Formats:** `.fontcraft` version 2 (versioned JSON, stable output, atomic writes, upgrade from
  version 1). `.glyphs` versions 2, 3 and 4 and `.glyphspackage`, read into the model and written
  back: every corpus source the reference app saved comes back byte for byte (17 files and 17
  packages, Oswald and Inter among them); unmodelled keys travel in `Extra` with their spelling.
  `file.open` recognises sources by content; `file.save` keeps the format and version, rewrites
  only changed package files, refuses to overwrite a file changed on disk, and reports losses.
  UFO 2 and 3 (a UFO is a master) and designspace documents with their UFOs (one document)
  open and save back byte for byte in fontTools' layout, keeping each file's own layout and
  spellings while they still mean what the model holds; UFOZ opens; `file.exportUfo` writes
  masters as UFOs. Smart components stay components in a UFO or designspace, or on request
  (`smartComponents: outlines`, `convert --smart-components outlines`, and `file.exportUfo` by
  default) are drawn as outlines interpolated from their pole layers, as glyphsLib draws them
  (identical to glyphsLib's points in all 10 glyph layers of the corpus that use them). Compiled fonts (TrueType, CFF, collections, WOFF 1) open view-only. Kerning,
  axes, instances, master locations, features, display strings and the note are fields of the
  model, and so are the font's properties, custom parameters, settings, date and user data: UFO
  font information maps to the reference's names through a crosswalk ported from glyphsLib.
  From `.glyphs`, stems, numbers, master guides, alignment zones, version 2 axes and name parts,
  and the instances' classes, style linking and interpolation settings are fields too. UFO blue
  values, stem snaps and guidelines read into and write from the same fields, by glyphsLib's
  rules. Designspace axes (user range, discrete values, localised names, STAT labels) are axis
  fields, their maps the `Axis Mappings` parameter; designspace instances and variable fonts
  are the model's instances, with glyphsLib's instance lib keys. A `.glyphs` source written as a
  designspace gets glyphsLib's axis ranges, maps and instance attributes. A glyph's metadata
  (production name, classification, metrics keys, colour, tags, note, lock, sort names,
  smart-glyph properties, user data) is a model field; in a UFO the note, mark colour,
  production names and `GDEF` classes travel, the rest is reported lost. A layer's name,
  attributes, vertical metrics, metrics keys, colour, guides, hints, annotations, image,
  background and user data are model fields of `.glyphs` and the native file; in a UFO the
  backgrounds, backup, intermediate and colour layers, guides, images, vertical metrics and user
  data travel by glyphsLib's rules, the rest is reported lost. In a designspace, intermediate
  layers also get sparse sources and alternate layers become bracket glyphs with rules, as
  glyphsLib writes them, and both read back as layers. Shape data is typed too: node
  joins, locks, orientations, names and user data, path locks and drawing attributes, the
  components' alignment, attachment anchors, smart values, masters, locks and orientations, the
  anchors' locks and orientations, and version 4 image shapes; in a UFO the node joins and names
  and the identifiers and anchor colours travel, the rest is reported lost. A node stays 32
  bytes. Version 4 contextual kerning is kept as stored and feature labels are fields; a UFO's
  groups that are not kerning groups are the font's groups (glyphsLib's classes in `.glyphs`),
  UFO 2 kerning groups are the glyphs' kerning groups, and `features.fea` is written with
  glyphsLib's markers and split back into prefixes, classes and features when that gives the
  same text. The font's settings, app version, edit-tab texts, user data and custom parameters
  with no font-info key, and the masters' icons, name parts and such parameters travel in
  glyphsLib's UFO lib keys and are read back from them where they say what the model would
  write. So do the glyphs' categories, scripts, last changes, metrics keys, smart-glyph
  properties and user data, the layers' metrics keys, hints, annotations, image crops, colour
  palette entries, ids and order, the UFO layer libs, and the shapes' order, the components'
  alignment, anchors, locks and smart values, node user data and anchor user data. Numbers,
  stem names, master visibility, glyph tags, locks and sort names, layer colours, drawing
  attributes, and right-to-left, vertical and contextual kerning are not yet written to UFO
  (glyphsLib has no key for them). M1.7's measures: every round trip unchanged; the census of
  untyped keys explained; all 35 reference-written `.glyphs` sources through a designspace and
  back with only reported losses (a test in the suite); fontmake's builds of Oswald from our
  designspace and from the `.glyphs` equal in 4,614 of 4,626 static glyphs and 882 of 883
  variable-font glyphs (the rest are non-spacing mark widths), with equal names and OS/2 values
  but `xAvgCharWidth` in two instances. Saving a UFO or designspace in another format reports
  the data only its files held, not their layout.
- **Compiler:** static TrueType of a master or of an instance at a master, deterministic, in
  stages (`Font::resolve`, preflight, glyph order, outline preparation, table builders): missing
  components dropped, non-exported glyphs inlined, `.notdef` drawn as fontc draws it,
  composites kept unless mixed or beyond ±2, cu2qu at 0.001 em, contours reversed, half-up
  rounding, composite boxes and `maxp` limits as fontc computes them. The core tables (head,
  hhea, hmtx, maxp, OS/2, name, cmap, post, vhea, vmtx) derive every field as fontc does:
  metric defaults per source format, names with fontc's fallbacks, style flags, `NONE` as the
  vendor id, Unicode and code page ranges, `USE_MY_METRICS`.
- **CFF writer:** `crates/cff` (standalone): a static, name-keyed CFF table from plain data
  (names, Top and Private DICT values, glyph names, advances, cubic outlines), with the
  reference's coordinate rounding, operator specialiser, width optimiser, FontBBox, charset
  choice and layout. On 115 fonts (every UFO in the corpus the reference compiles, plus
  synthetic ones) its tables are byte-identical to fontTools'. No hints, no subroutines.
- **Static CFF in the compiler:** `Flavour::Cff` builds an OTF of a master as fontmake does:
  decomposed cubic outlines in their drawn direction, glyph names in the charset, Top DICT
  names from the `name` table, blue zones and stems from the master, `maxp` 0.5, `post` 3.0,
  metrics from the charstring bounds. Against fontmake over 331 corpus sources: 322 CFF tables
  decode to the same outlines, 206 are byte-identical, the other 9 are allow-listed source
  reading. Every font built from the corpus (551 OTF, 551 TTF, masters and instances) reads back
  in skrifa and fontTools.
- **Containers:** `crates/container` (standalone): WOFF 1 and WOFF2, written and read with
  fontTools' table order, checksums, header fields and compression settings; the WOFF2 `glyf`/
  `loca` transform (by default) and `hmtx` transform (on request), both ways. All 342 corpus
  fonts and all 1,102 fonts compiled from the corpus wrap and unwrap to the same tables;
  fontTools reads all 340 corpus TTF and OTF wrappings back to its own tables, and our reader
  reads fontTools' (339 of 340 with `hmtx`: fontTools shortens one font's `hmtx`); 21 WOFF2
  files are byte-identical to fontTools' and the rest differ only in the Brotli stream.
  `file.open`, `font.inspect` and `tables.list` read WOFF 1 and WOFF2 through it.
- **Export:** `file.exportStatic`, `file.exportAll`, `font.compile` and `font.compilePlan`, and
  `fontcraft-cli compile` (`--plan` for the plan): static TTF and OTF of every exportable
  instance at a master, of the first master when there are none, or of each master; plain, WOFF
  and WOFF2; named by the `fileName` parameter or the PostScript name (the source file's name
  stands in for a missing family name), with `ttf` and `otf` subfolders when both are written. Instances between masters are skipped with a reason;
  overlaps are kept with a warning. A failing export writes nothing: the files are staged and
  renamed only when every font is built. `convert` also writes `.otf`, `.woff` and `.woff2`.
  The same source and timestamp give the same bytes (`crates/engine/tests/determinism.rs`).
- **Oracle harness:** `crates/oracle` and `fontcraft-cli oracle`: FontCraft's build and fontc's
  or fontmake's (TrueType, or CFF from fontmake) of the same master, compared per table after
  normalisation (identical, equivalent, allowed with a reason, different, skipped), with
  source-reading differences reported apart; `--corpus` over 354 sources; opt-in oracle tests.
- **Agent surfaces:** CLI (`commands`, `run`, `convert`, `compile`, `info`, `oracle`, `mcp`), control channel
  (9 methods, loopback, request-only, among them `ui.screenshot`, which captures the window as
  PNG, and `ui.render`, which draws glyphs without a window), MCP server (3 tools, headless or
  attached).
- **UI:** egui shell split into `shell/` and `canvas/`, with a registry-built menu bar cached per
  document revision, Font View grid, Palette stub and status line. Edit View draws the
  reference's display conventions (smooth and corner nodes, handles, open ends, start nodes,
  coincident nodes, metric lines, fill preview), zooms and scrolls, and holds tabs (new, show,
  next, previous, close, close others, reopen). The View toggles and `view.getDisplayOptions`
  hold the display state. Runs in a window and in headless test frames.
- **Gates:** `cargo xtask ci` with nine steps; parity report generated from the inventory;
  `cargo xtask roundtrip` over 527 corpus sources (outside the gate).

## Milestones

This file does not estimate time: estimates of agent hours have proved unreliable. Progress is
measured by what works (the parity table below and the measures in each milestone's plan).

| # | Milestone | Status | Commands (live) |
|---|---|---|---:|
| M0 | Skeleton and vertical slice | ✅ done | 5 (5) |
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | ✅ done: every format read and written back byte for byte; the typed model (M1.7) with glyphsLib's lib keys, intermediate and alternate layers in a designspace, and its measures met; the gaps left on purpose are in `docs/plans/M1.md` (M1.4 and M1.7) | 10 (10) |
| M2 | Compile core: static TTF and OTF, oracle harness | ✅ done: the oracle harness, the staged pipeline, the core tables at fontc and fontmake parity (no table different), static CFF, WOFF and WOFF2 with transforms, the export commands and CLI `compile`; every measure met (`docs/plans/M2.md` § 5). Deferred on purpose: instances between masters (M9), overlap removal (M3), CFF subroutines, export dialogs (M3) | 6 (4) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | 🟡 plan accepted (`docs/plans/M3.md`). Done: M3.0 spec pass (targets in every command's params, `when` and `context` fields, D13 answered, 5 editor rows confirmed by tutorial videos); M3.1 engine foundations (one target resolver, interactions, `layer.inspect`, the glyph lock, segment geometry, grid rounding); M3.2 canvas, `render` crate, `ui.screenshot` and `ui.render`, zoom, scroll, tabs, View toggles; wave 2's engine commands: nodes and Fit Curve (M3.4), transformations (M3.5), clipboard and SVG import (M3.8), guides, measuring and Dimensions (M3.9), background and layers (M3.10), annotations (M3.16); M3.3 pointer editing in the window (Select, Lasso, Hand, Zoom, snapping, keymap, `ui.pointer`, `ui.key`). Crates without commands yet: `pathops` (M3.6), `audit` (M3.17: 34 of 36 outline checks). Left: path-op commands, M3.7, strokes, masks, filters, drawing tools, panels, dialogs, audit commands | 181 (125) |
| M4 | Font View and glyph management | ⬜ | 57 (6) |
| M5 | Spacing, kerning and text mode | ⬜ | 109 (1) |
| M6 | Components and anchors | ⬜ | 65 (0) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | ⬜ | 62 (2) |
| M8 | OpenType features | ⬜ | 40 (0) |
| M9 | Interpolation and variable fonts | ⬜ | 68 (0) |
| M10 | Hinting (PostScript and TrueType) | ⬜ | 57 (0) |
| M11 | Colour fonts and images | ⬜ | 50 (0) |
| M12 | Command-line tool suite (61 of the 70 subcommand rows) and quality checks (263 of 300) | ⬜ | 71 (0) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | ⬜ | 114 (0) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 31 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 3 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 932 (200) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 44 | 20 | 10% | tabs, zoom and scroll work; missing: toolbar, settings window, shortcut editor, workspaces, title bar, sidebars |
| Font View | 6 | 62 | 6 | 3% | list view, sidebar and filters, glyph data, Add Glyphs syntax, batch commands, inspector |
| Edit View text mode | 4 | 25 | 1 | 0% | everything but the fill-preview toggle |
| Drawing and editing outlines | 10 | 94 | 68 | 11% | node, segment, transform, clipboard and selection commands work headless; Select, Lasso, Hand and Zoom work in the window; missing: the drawing tools (M3.13), context menus, the Info box and Palette panels, bounding-box knobs, the Transformations dialog; nothing compared with the reference |
| Path operations and filters | 6 | 66 | 7 | 3% | node-order commands (connect, close, make first, reverse, insert) work headless; the booleans, Remove Overlap, extremes, Tidy up, Open Corner and Cut with Line exist in `pathops` without commands; no filters, strokes or masks |
| Reusing shapes | 7 | 77 | 0 | 0% | everything; components are only exported (as composites, decomposed where the format needs it) |
| Layers | 3 | 21 | 15 | 15% | background and layer commands work headless; missing: the Layers panel, background drawing in the canvas, the Interpolate with Background dialog |
| Guides, metric lines, zones, grid | 3 | 36 | 16 | 10% | guide, ruler, coordinate and Dimensions commands work headless and metric lines are drawn; missing: guides and the grid on the canvas, guide dragging and snapping, the Measurement tool |
| Spacing and kerning | 7 | 79 | 1 | 1% | set width only |
| Font Info | 6 | 48 | 2 | 2% | four font-wide fields can be read and set |
| Interpolation | 8 | 68 | 0 | 0% | everything |
| OpenType layout | 7 | 46 | 0 | 0% | everything |
| PostScript hinting | 3 | 27 | 0 | 0% | everything |
| TrueType hinting | 4 | 40 | 0 | 0% | everything |
| Colour fonts and images | 3 | 46 | 0 | 0% | everything |
| Source formats | 6 | 14 | 6 | 57% | Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; production names and mark widths from a glyph database; UFO 1; shape groups, gradients and shadows kept as stored values; right-to-left, vertical and contextual kerning in UFO; version conversion |
| Compiling | 9 | 11 | 4 | 10% | static TTF and OTF, plain, WOFF and WOFF2, from the commands; core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source. Missing: variable fonts and instances between masters (M9), features (M8), hinting (M10), overlap removal (M3.7), CFF subroutines, production names (M4), 37 of 50 tables |
| Preview, proofing, printing | 2 | 23 | 0 | 0% | everything |
| Command-line tools | 8 | 42 | 4 | 3% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `tables list`, `glyphs list`, `oracle`; no dump, subset, instance, merge |
| Quality checks | 3 | 33 | 0 | 2% | `audit` holds 34 of the 36 outline checks with their fixes named, but no `audit.*` command, live audit or markers exist; the other 264 checks |
| Plug-ins and scripting | 3 | 16 | 0 | 0% | everything |
| Agent surfaces | 4 | 11 | 3 | 30% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; no pointer or key injection, dialogs, MCP resources or prompts |
| **Feature parity** | **117** | **932** | **200** | **~7%** | |

## Out of scope (by design or by law)

- **Undocumented proprietary formats** (FontLab's binary formats, the primary reference's
  metrics files where no public description exists).
- **A reference app's plug-ins and Python scripts.** Ours is a different extension model
  (decision 0005).
- **Cloud services, accounts, telemetry, font-library subscriptions.**
- **Any reference app's bundled content**: glyph data beyond what is openly licensed, sample
  strings, icons, example fonts.

## Where we aim to be better than the references

- **Agent-native:** every command, gesture and dialog drivable over MCP, the CLI and the control
  channel; headless batch.
- **One tool for the editor and the command line:** the same commands and the same compiler, so
  what the editor previews is what the build produces.
- **Provable output:** oracle comparison against the open compilers as a standing gate.
- **Lossless interchange** between `.glyphs`, UFO and designspace, in both directions.
- **Everywhere:** macOS, Windows, Linux and the browser from one codebase.
- **Never loses work:** no panics, atomic commands, atomic saves, deep undo (10,000 steps).

## How to update this file

After each task lands: the milestone row, the parity-table row (score, missing items), "Shipped so
far", and the "What to build next" list if it changed. Keep the README's Status in step with the headline here.
