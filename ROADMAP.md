# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-10 (M3 at its close: every measure of `docs/plans/M3.md` § 5 met except one command, `path.sharpenCorner`, which no source describes)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,645 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, pointer and key injection, toolbar, context menus, dialogs, panels, tests | **~60%** of what the siblings' "infrastructure" row covers; missing: the settings window, MCP resources and prompts, a performance budget suite |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~12%** |
| Commands that exist (not: that behave like the reference) | **213 of 932** (`docs/parity.md`) |
| Observed against the running reference | **52 rows**: 47 compiled-output rows checked against fontc and fontmake, and 5 editor rows confirmed by public tutorial videos of the previous major version. The reference app itself has not been run |

## Honest assessment (2026-10-10)

**In one line:** the project reads and writes `.glyphs`, UFO and designspace sources without loss
and compiles static fonts that match fontc and fontmake, overlaps removed; its Edit View draws and
edits outlines with 18 tools, a toolbar, context menus, five Palette sections and the M3 dialogs,
but none of the editor has been compared with the running reference app.

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
names them, from `file.exportStatic`, `file.exportAll`, `font.compile`, File › Export and
`fontcraft-cli compile`. Export expands strokes, applies masks and removes overlaps, each glyph
under a time limit. Built from the 347 corpus sources FontCraft compiles, all 1,102 TTF and OTF fonts read
back in skrifa and fontTools without an error, wrap and unwrap as WOFF2 to the same tables, and
the same source and timestamp give the same bytes. `fontcraft-cli oracle` compiles a source with
FontCraft and with fontc or fontmake and compares the two fonts table by table; over 354 corpus
sources it measures the compiler: 332 compared with fontc, and every core table identical,
equivalent or allow-listed with a reason in all of them; 331 compared with fontmake's CFF builds,
whose outlines FontCraft's match in 322 (the other 9 are source reading, allow-listed). With
overlaps removed on both sides, 99.58% of TrueType glyphs and 99.87% of CFF glyphs show no
overlap-removal difference from fontmake's; the rest differ by less than one unit and are
allow-listed. `crates/container` writes and reads WOFF 1 and WOFF2 with the `glyf`/`loca` and `hmtx`
transforms; fontTools reads all of its output back.

**What is real: editing by command (M3).** 213 of the 932 commands exist, 184 of them from M3.
Every editing command takes explicit targets (shapes, nodes, segments, the foreground or the
background) and runs the same from the window, `fontcraft-cli run`, MCP and the control channel:

| Family | Commands |
|---|---|
| Nodes and segments | move and position a selection, node types, convert a segment to a curve, delete with or without breaking the path (also in all masters), insert and connect nodes, close open paths, make a node first, reverse contours (also in all masters), lock paths and glyphs, rectangles and ellipses, Fit Curve |
| Transformations | the Palette's mirror, scale, rotate, slant and align buttons, Align Selection, Path › Transformations and its export string, the transformation origin and settings |
| Path operations | Remove Overlap, union, difference and intersection, Correct Path Direction, Add and Force Extremes, Add Inflections, Tidy up Paths, Open Corner, Reconnect Nodes, Cut with Line, Convert to Cubic and to Quadratic, Round Coordinates; several also for all masters. Each layer runs under a time limit: a layer that runs out of time stays as it was and is listed as skipped |
| Filters | `filter.apply` with the export-string syntax, `filter.list`, Repeat Last Filter, and the six built-in filters: Round Corners, Extrude, Hatch Outline, Offset Curve, Roughen, Rounded Font |
| Strokes, masks, shape order | stroke attributes, Expand Outline, masks and shape order; the canvas draws the result and export applies it |
| Clipboard and SVG | copy, cut and paste (our JSON fragment and SVG; the window uses the system clipboard), SVG files imported into glyphs |
| Guides and measuring | local and global guides, the ruler and coordinate measures, Dimensions |
| Background, layers, masters | the background commands and Edit Background, Interpolate with Background, backup layers, layer visibility, order and names, the active layer and the tab's master |
| Annotations | add, update, move, delete, list, select all |
| Audit | run, list, describe, results, fix and profiles: 34 of the 36 outline checks; each fix runs a path command as one undo step |

**What is real: the window (M3).** Edit View has 18 tools, which cover all 19 M3 tool rows (the
Annotation row is the group of its six tools): Select, Lasso, Hand, Zoom, Draw, Pencil,
Rectangle, Ellipse, Knife, Rotate, Scale, Measurement and the six Annotation tools. A tool is
chosen from the toolbar, by its letter (Shift and a shared letter step to the next tool on that
letter) or over the control channel. The toolbar has one button per tool group, with drawn
icons; holding a shared button opens a menu of its tools. Tools edit through the commands above.
A drag is an interaction: any number of previews, then one commit and one undo step. Points a
command creates or moves round to the font's grid. The canvas draws:

- nodes and handles by the reference's display conventions, metric lines and the fill preview;
- the bounding box with its knobs and the transformation origin;
- guides, which can be dragged and edited in place, the grid and CJK lines, and measurement guides;
- the background, the outlines of other visible layers, and annotations;
- strokes and masks as export builds them, with the paths themselves in grey underneath;
- audit markers, from live audit or from an explicit run; a click on one selects what it reports.

A tab draws in one master: Cmd+1 to Cmd+9 picks it. A row of the Palette's Layers section makes
that layer of the edited glyph the active layer, which the canvas draws and the tools edit. Edit
Background (Cmd+B) moves the tools, the Palette and the Info box to the background. A
right-click opens a context menu of the commands for the clicked node, segment, path, guide or
glyph, or for a row of the Layers section. Menu bar and context items are greyed out when the selection supplies no target.
Snapping finds nodes, handles, the background's points, guides and their crossings, and metric
lines, and lines a point up with other nodes. The Info box shows and edits the selected node,
handle or box at a reference point. The Palette has Dimensions, Fit Curve, Layers,
Transformations and Stroke sections; the bottom bar has the zoom controls.

A dialog is a form over one command's parameters with a live preview: OK keeps the result as one
undo step, Cancel restores the font. There are dialogs for each filter (built from its parameter
schema), Transformations, Shape Order, Interpolate with Background, Primitive Measurements, Guide
Properties, the pasted-outline and missing-glyph questions, and File › Export with its OTF and
UFO tabs. Agents drive the window with `ui.pointer`, `ui.key`, `ui.text`, `ui.tool.select`,
`ui.menu.list`, `ui.menu.invoke` and the `ui.dialog.*` methods, and see it with `ui.screenshot`;
`ui.render` draws glyphs to PNG without a window, the same bytes every time. `layer.inspect`
reads a layer's outline with addresses. An end-to-end test draws "H", "O" and "n" on a new font
over the control channel alone, edits them and exports a font that skrifa and fontTools read
back. 931 tests pass, including a sweep that runs every engine command with hostile parameters.
The whole gate (`cargo xtask ci`: format, lints, tests, spec, parity, assets, vendor names,
layering, wasm) is green.

**What is not.** Nothing in the editor has been compared with the running reference app. M3's
behaviour comes from the handbook; 5 editor rows are confirmed by public tutorial videos. Where
a detail is unconfirmed, the handbook decides (Remove Overlap, for example, deletes open paths).

| Gap | Detail |
|---|---|
| `path.sharpenCorner` | the one M3 command not built: no written source says what it does, and the tutorial video shows the menu item without running it. Moving it to the `fidelity` milestone awaits the owner |
| Text mode | Edit View edits glyphs, not typed text; text mode is M5 |
| Masters | Font View draws the first master and has no master choice of its own (M4); the toolbar has no master icons (M9) |
| Export dialog | OTF and UFO tabs; the Variable tab is M9 and the Metrics tab M5 |
| Import Outlines | the missing-glyph question is asked once for all unmatched files, not once per file |
| Hints | Shape Order and Expand Outline do not renumber hints that refer to a path by index |
| Strokes in files | a UFO keeps no stroke attribute; cap code 4 and the forced cap axis are not confirmed against the reference's files |
| Info box | no annotation palette; the Annotation tools are chosen from the toolbar |
| Audit | 34 of the 36 outline checks; the other 264 checks are M12 |

Beyond M3 there is no kerning editing, no component editing (components are exported as
composites), no feature compilation, no variable fonts and no instances between masters (M9),
no CFF subroutines, no hinting, no web app, no packaging. Glyphs-only data travels in
glyphsLib's UFO lib keys where glyphsLib has a key for it, and alternate and intermediate
layers travel as designspace rules and sparse sources; the data no key holds (glyph tags,
locks, sort names, layer colours, drawing attributes…) does not, and the loss report lists it.
Production names and the widths of non-spacing marks need a glyph database (M4). Kerning,
features and instances are read and written, not yet edited or compiled.

**How much to trust the inventory.** It was compiled from public documentation in one day, each
source read once. Of its 3,645 rows, 2,107 come from the primary reference's handbook, 1,055 from
other public documentation or permissive source, 401 are our own design, 30 rest on
recollection, and 52 are observed: 47 compiled-output rows checked against fontc and fontmake,
and 5 editor rows confirmed by public tutorial videos. **No other row has been checked against
the running reference app.** An independent re-reading of 209 sampled handbook rows against the
handbook found 1 wrong and 14 imprecise (`docs/spec/README.md`). The notes list about 450 open
questions the documentation does not answer. Treat rows as a careful first reading, and each
milestone's rows as due a planning pass before they are built (`docs/execution-plan.md` § 3).

### By dimension

| Dimension | Estimate | Evidence and what is missing |
|---|---|---|
| **Breadth:** commands, tools and panels exist | ~12% | 213 of 932 commands; 19 of 33 tool rows usable in the window (18 tools: every M3 tool row, the Annotation row being the group of its six tools); 26 of 172 panels, many of them partial (the Font View grid, the toolbar, the Info box, the Palette and five of its sections, the bottom bar, 16 dialog rows) |
| **Depth:** features behave like the reference | ~5% | the M3 commands, tools, panels and dialogs behave as the handbook describes them, and none has been compared with the reference app. `path.sharpenCorner` has no source; the gaps are listed above |
| **File interop** | ~38% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFOZ and compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts); static TTF and OTF out, plain, WOFF and WOFF2, from the export commands, the Export dialog and the CLI. Missing: variable fonts, instances between masters, version conversion, a glyph database for production names, stroke attributes in UFO |
| **Compiler correctness** | measured, low | 13 of 50 tables written, for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers; strokes, masks and overlap removal in the outline stage. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, non-spacing marks only the glyph database knows (M4), smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). With overlaps removed, 99.58% (TrueType) and 99.87% (CFF) of glyphs have no overlap-removal difference from fontmake's (`docs/plans/M3.md` § 5). No variable font, no features, no hinting, no subroutines |
| **Agent automation** | ~55% | every existing command is reachable from CLI, control channel and MCP; the control channel's 19 methods drive the window's tools, pointer, keys, text fields, dialogs and menus; `ui.screenshot` and `ui.render` show the result; an agent can draw, edit and export glyphs over the control channel alone (end-to-end test). Missing: `ui.click` beyond dialogs, `ui.set`, MCP resources and prompts |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation, corpus runs of overlap removal and the audit); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and the path-operation commands and export run each glyph on a worker thread under a time limit (a glyph that runs out of time stays as it was; the worker cannot be stopped and runs on detached). One adversarial code review, most findings fixed and the rest listed in `docs/status.md`. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | two measures | overlap removal over the corpus: median 38 µs, 99th percentile 403 µs per glyph layer; one node-drag preview step on Oswald: 142 µs. No budget suite for the targets in `docs/architecture.md` |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: 19 crates build for the browser; no web app |
| **Look and feel** | barely started | a functional shell with the siblings' default dark tokens; the toolbar, panels, dialogs and canvas overlays take their colours and sizes from the tokens, with drawn icons for the tools, the Transformations buttons and the Stroke section; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

1. **The owner's review of M3**: the measures and their results (`docs/plans/M3.md` § 5), the
   window, and the open decision on `path.sharpenCorner` (§ M3.18).
2. **M4's planning pass, Font View and glyph management**, as `docs/execution-plan.md` § 3
   describes it: list the milestone's rows, answer the open questions of its notes, verify or
   drop its `memory` rows, and write `docs/plans/M4.md` for the owner to accept.

Then M5–M7 on the editor track and M8–M12 on the engine track, as `docs/execution-plan.md` § 4
orders them. Left from M3 for later milestones: text mode (M5), Font View's master choice
(`fontView.cell.followsMaster`, M4), the toolbar's master icons (M9), the Export dialog's
Variable (M9) and Metrics (M5) tabs. Left from M2: instances between masters (M9), CFF
subroutines (before M12), production names (M4), the compile plan's `productionNames` and
`features` sections, and the CLI's `--name-by`, `--output-layout` and pattern instance
selection (`docs/plans/M2.md` › M2.5).

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (932 commands, 33 tools, 172 panels, 45 settings, 241
  parameters, 1,753 behaviour rules, 300 checks, 70 CLI rows, 49 formats, 50 tables) and 23
  behaviour notes (about 150,000 words), all in our own words. Every M3 command's `params` name
  its targets; shortcuts carry a `when` (the context they apply in).
- **Workspace:** 20 library crates in enforced layers, two apps, `xtask`; about 113,000 lines of Rust.
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
- **Editing commands (M3):** node and segment commands and Fit Curve (M3.4); the Palette
  transforms and alignments, Align Selection, Path › Transformations with its export string,
  and the transformation origin and settings (M3.5); the path-operation commands on `pathops`
  (M3.6); copy, cut, paste and SVG outline import (M3.8); local and global guides,
  `measure.ruler`, `measure.coordinates` and Dimensions (M3.9); the background commands,
  Edit Background, Interpolate with Background, the layer commands, the active layer and the
  tab's master (M3.10); stroke attributes, Expand Outline, masks and shape order (M3.11);
  annotations (M3.16); the audit commands and Add Inflections (M3.17). Points a command
  creates or moves round to the font's grid. Each family has behaviour tests on synthetic
  glyphs, and the command sweep covers every command.
- **Filters (M3.12):** `filter.apply` with the parser of the export-string syntax (M7 reuses it
  at export), `filter.list` with each filter's parameter schema, Repeat Last Filter, and Round
  Corners, Extrude, Hatch Outline, Offset Curve, Roughen (seeded, so the same seed gives the same
  font) and Rounded Font in `crates/pathops/src/filters/`.
- **Rendering:** `crates/render` (tiny-skia): a layer or a line of glyphs to RGBA and PNG,
  nonzero fill, anti-aliased, the same bytes for the same input.
- **Path operations:** `crates/pathops`: union, difference and intersection on linesweeper,
  Remove Overlap (open paths and stray nodes removed, as the handbook says), Correct Path
  Direction, Add and Force Extremes, Tidy up Paths, Open Corner, Reconnect Nodes, Cut with
  Line, strokes (kurbo's stroker plus our round inset caps, round full joins and axis-aligned
  caps), masks, and a coverage comparison. Input is checked and capped before linesweeper sees it.
  Union matches skia-pathops' coverage on all 2,368 corpus layers with closed paths (`cargo
  xtask overlap`). linesweeper 0.5.0 is kept from crates.io and its rare hang is not reported
  upstream for now; the rule for callers that run without a person waiting (export, batch
  commands) is a time limit per glyph (`docs/plans/M3.md` › M3.6, rule 8). The path-operation
  commands and export run each glyph on a worker thread under that limit; the commands carry
  node names, identifiers, locks and user data through each operation.
- **Tools:** `crates/tools` (no egui): 18 tools as state machines that emit interactions:
  Select, Lasso, Hand, Zoom, Draw, Pencil (freehand points fitted to curves), Rectangle,
  Ellipse, Knife, Rotate, Scale, Measurement and six Annotation tools. Hit testing of nodes,
  handles, segments, anchors, components and guides; snapping to nodes, handles, the
  background, guides and their crossings, and metric lines; the selection and its target
  object; the bounding box and its knobs; the Edit View keymap, temporary tools and tool
  letters shared by several tools. The shell drives them from the pointer and from `ui.pointer`
  and `ui.key`.
- **Audit:** `crates/audit`: the check registry and 34 of the 36 outline checks, with
  severities, thresholds, fixes by command id and the result document of `docs/spec/21-qa.md`
  § 4. Commands (M3.17): `audit.run`, `audit.list`, `audit.describe`, `audit.results`,
  `audit.fix` (eight path commands, one undo step, the unfixed layer kept as a backup) and
  `audit.profile.list`; every fix clears its check. In the window, live audit re-runs the
  outline checks on the edited layer after each change, and the canvas marks what they report.
  `cargo xtask audit` runs the audit over the corpus.
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
  stands in for a missing family name), with `ttf` and `otf` subfolders when both are written.
  Instances between masters are skipped with a reason. The outline stage expands strokes,
  applies masks and removes overlaps (unless the `removeOverlap` option keeps them), each glyph
  under a time limit; a glyph that runs out of time keeps its overlaps, with a warning. File ›
  Export opens the Export dialog (OTF and UFO tabs). A failing export writes nothing: the files are staged and
  renamed only when every font is built. `convert` also writes `.otf`, `.woff` and `.woff2`.
  The same source and timestamp give the same bytes (`crates/engine/tests/determinism.rs`).
- **Oracle harness:** `crates/oracle` and `fontcraft-cli oracle`: FontCraft's build and fontc's
  or fontmake's (TrueType, or CFF from fontmake) of the same master, compared per table after
  normalisation (identical, equivalent, allowed with a reason, different, skipped), with
  source-reading differences reported apart; `--corpus` over 354 sources; opt-in oracle tests.
- **Agent surfaces:** CLI (`commands`, `run`, `convert`, `compile`, `info`, `oracle`, `mcp`), control channel
  (19 methods, loopback, request-only: commands, menus, tools, pointer and key events, text,
  dialogs, `ui.screenshot`, which captures the window as PNG, and `ui.render`, which draws
  glyphs without a window; `ui.menu.list` and `ui.menu.invoke` show and click the greyed menu
  tree), MCP server (3 tools, headless or attached).
- **UI:** egui shell split into `shell/`, `canvas/`, `panels/` and `dialogs/`, with a
  registry-built menu bar cached per document revision, Font View grid and status line. Edit
  View draws the reference's display conventions (smooth and corner nodes, handles, open ends,
  start nodes, coincident nodes, metric lines, fill preview), the bounding box and its knobs,
  guides, the grid and CJK lines, measurement guides, the background, other visible layers'
  outlines, annotations, strokes and masks, audit markers and each tool's overlay, all styled
  from the tokens. It zooms and scrolls, and holds tabs (new, show, next, previous, close,
  close others, reopen); each tab draws in its own master (Cmd+1 to Cmd+9) and edits the
  foreground or, after Edit Background, the background. The View toggles and
  `view.getDisplayOptions` hold the display state. The toolbar has a button per tool group
  with drawn icons. Context menus list the commands for the clicked object; menu bar and
  context items are greyed out by the selection. Copy, Cut and Paste use the system clipboard;
  system choosers pick the export folder and the files of File › Import › Outlines. Panels
  (M3.15): the Info box's selection part with its reference point, the Palette with Dimensions,
  Fit Curve, Layers, Transformations and Stroke sections that collapse, and the bottom bar's
  zoom controls; number fields follow the field rules (Return, Tab, Escape, arrows with
  Shift). Dialogs (M3.14): a form over one command's parameters with a live preview inside an
  interaction, OK as one undo step and Cancel back to the font the dialog opened on. Fields
  include numbers, choices, multiple choices, folders, lock buttons, a reorderable list and
  "…" action menus, grouped under headings or tabs. Every filter with parameters gets a dialog
  from its schema; Round Corners, Transformations, Shape Order, Interpolate with Background,
  Primitive Measurements, Guide Properties, Export and two follow-up questions have their own
  files. Runs in a window and in headless test frames.
- **Gates:** `cargo xtask ci` with nine steps; parity report generated from the inventory.
  Outside the gate: `cargo xtask roundtrip` over 527 corpus sources, `cargo xtask overlap`
  (overlap removal on every corpus layer) and `cargo xtask audit`.

## Milestones

This file does not estimate time: estimates of agent hours have proved unreliable. Progress is
measured by what works (the parity table below and the measures in each milestone's plan).

| # | Milestone | Status | Commands (live) |
|---|---|---|---:|
| M0 | Skeleton and vertical slice | ✅ done | 5 (5) |
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | ✅ done: every format read and written back byte for byte; the typed model (M1.7) with glyphsLib's lib keys, intermediate and alternate layers in a designspace, and its measures met; the gaps left on purpose are in `docs/plans/M1.md` (M1.4 and M1.7) | 10 (10) |
| M2 | Compile core: static TTF and OTF, oracle harness | ✅ done: the oracle harness, the staged pipeline, the core tables at fontc and fontmake parity (no table different), static CFF, WOFF and WOFF2 with transforms, the export commands and CLI `compile`; every measure met (`docs/plans/M2.md` § 5). Deferred on purpose: instances between masters (M9), overlap removal (M3), CFF subroutines, export dialogs (M3) | 6 (5) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | 🟡 measures met except `path.sharpenCorner` (owner decision pending): every package M3.0 to M3.18 built; 184 of 185 commands live, gesture equals command for all 19 tool rows, an agent draws and exports over the control channel, overlap removal at export coverage-equal to fontmake's in over 99.5% of glyphs, M1 and M2 baselines unchanged (`docs/plans/M3.md` § 5). `path.sharpenCorner` has no written source; moving it to `fidelity` awaits the owner | 185 (184) |
| M4 | Font View and glyph management | ⬜ | 57 (6) |
| M5 | Spacing, kerning and text mode | ⬜ | 109 (1) |
| M6 | Components and anchors | ⬜ | 65 (0) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | ⬜ | 62 (2) |
| M8 | OpenType features | ⬜ | 40 (0) |
| M9 | Interpolation and variable fonts | ⬜ | 67 (0) |
| M10 | Hinting (PostScript and TrueType) | ⬜ | 57 (0) |
| M11 | Colour fonts and images | ⬜ | 50 (0) |
| M12 | Command-line tool suite (61 of the 70 subcommand rows) and quality checks (263 of 300) | ⬜ | 71 (0) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | ⬜ | 114 (0) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 31 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 3 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 932 (213) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 45 | 22 | 16% | tabs, zoom and scroll work; the toolbar with shared buttons and their menus; tools chosen by their letters, shared letters cycle; context menus; menu items greyed out by the selection; bottom bar zoom controls. Missing: settings window, shortcut editor, workspaces, title bar, sidebars, the toolbar's master icons (M9); nothing compared with the reference |
| Font View | 6 | 62 | 6 | 3% | list view, sidebar and filters, glyph data, Add Glyphs syntax, batch commands, inspector |
| Edit View text mode | 4 | 25 | 1 | 0% | everything but the fill-preview toggle |
| Drawing and editing outlines | 10 | 96 | 79 | 25% | node, segment, transform, clipboard and selection commands; in the window Select, Lasso, Hand, Zoom, Draw, Pencil, Rectangle, Ellipse, Rotate and Scale, the bounding box and its knobs, context menus, the Info box, the Palette's Transformations and Fit Curve sections, the Transformations and Primitive Measurements dialogs, editing in any master. Missing: `path.sharpenCorner` (no source), the Info box's annotation palette; nothing compared with the reference |
| Path operations and filters | 6 | 66 | 40 | 28% | Remove Overlap, the booleans, path direction, extremes and inflections, Tidy up, Open Corner, Reconnect, Cut with Line, curve conversion and rounding as commands; the Knife; the six filters with dialogs and export strings; strokes and masks drawn on the canvas and applied at export; the Stroke section and the Shape Order dialog. Missing: stroke attributes in UFO, hints renumbered after Shape Order and Expand Outline; nothing compared with the reference |
| Reusing shapes | 7 | 77 | 0 | 0% | everything; components are only exported (as composites, decomposed where the format needs it) |
| Layers | 3 | 21 | 19 | 35% | background and layer commands; Edit Background; the active layer and the tab's master; the background and other visible layers drawn on the canvas; the Palette's Layers section with its row context menu; the Interpolate with Background dialog. Missing: Font View's master choice (M4); nothing compared with the reference |
| Guides, metric lines, zones, grid | 3 | 36 | 16 | 26% | guide, ruler, coordinate and Dimensions commands; metric lines, guides, the grid, CJK lines and measurement guides drawn; guides dragged and edited on the canvas, with their context menu and the Guide Properties dialog; the Measurement tool; snapping to guides and their crossings; the Palette's Dimensions section. Missing: script-specific metric lines; nothing compared with the reference |
| Spacing and kerning | 7 | 79 | 1 | 1% | set width only |
| Font Info | 6 | 48 | 2 | 2% | four font-wide fields can be read and set |
| Interpolation | 8 | 68 | 1 | 1% | Edit View draws and edits any master (`view.selectMaster`); everything else |
| OpenType layout | 7 | 46 | 0 | 0% | everything |
| PostScript hinting | 3 | 27 | 0 | 0% | everything |
| TrueType hinting | 4 | 40 | 0 | 0% | everything |
| Colour fonts and images | 3 | 46 | 0 | 0% | everything |
| Source formats | 6 | 14 | 6 | 57% | Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; production names and mark widths from a glyph database; UFO 1; shape groups, gradients and shadows kept as stored values; right-to-left, vertical and contextual kerning in UFO; version conversion |
| Compiling | 9 | 11 | 5 | 13% | static TTF and OTF, plain, WOFF and WOFF2, from the commands and the Export dialog; strokes expanded, masks applied and overlaps removed (coverage-equal to fontmake's in over 99.5% of glyphs); core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source. Missing: variable fonts and instances between masters (M9), features (M8), hinting (M10), CFF subroutines, production names (M4), 37 of 50 tables |
| Preview, proofing, printing | 2 | 23 | 0 | 0% | everything |
| Command-line tools | 8 | 42 | 4 | 3% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `tables list`, `glyphs list`, `oracle`; no dump, subset, instance, merge |
| Quality checks | 3 | 33 | 8 | 10% | 34 of the 36 outline checks through `audit.run`; `audit.fix` applies eight fixes through path commands and each clears its check; live audit and canvas markers. Missing: the other 264 checks (M12); nothing compared with the reference |
| Plug-ins and scripting | 3 | 16 | 0 | 0% | everything |
| Agent surfaces | 4 | 11 | 3 | 40% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; tools, pointer, key, text, menu and dialog control over the control channel, enough to draw and export a font (end-to-end test). Missing: `ui.click` beyond dialogs, `ui.set`, MCP resources and prompts |
| **Feature parity** | **117** | **932** | **213** | **~12%** | |

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
