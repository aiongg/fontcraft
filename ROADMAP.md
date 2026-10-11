# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-11 (M5 closed: every measure of `docs/plans/M5.md` § 5 met but the two out-of-sync speed targets)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,648 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, pointer and key injection, toolbar, context menus, dialogs, panels, tests | **~60%** of what the siblings' "infrastructure" row covers; missing: the settings window, MCP resources and prompts, a performance budget suite |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~18%** |
| Commands that exist (not: that behave like the reference) | **365 of 933** (`docs/parity.md`) |
| Observed against the running reference | **52 rows**: 47 compiled-output rows checked against fontc and fontmake, and 5 editor rows confirmed by public tutorial videos of the previous major version. The reference app itself has not been run |

## Honest assessment (2026-10-11)

**In one line:** the project reads and writes `.glyphs`, UFO and designspace sources without loss
and compiles static fonts that match fontc and fontmake, overlaps removed and with production names;
its Edit View draws and edits outlines with 18 tools and types, spaces and kerns text, its Font
View manages the glyph set with a glyph database, filters, search and an inspector, and exported
fonts carry pair kerning equal to fontc's and fontmake's except 27 listed known differences.
None of the editor has been compared with the running reference app.

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

**What is real: editing by command (M3).** 365 of the 933 commands exist: all 184 of M3, all 62 of M4 and all 97 of M5.
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
back. 1,076 tests pass, including a sweep that runs every engine command with hostile parameters.
The whole gate (`cargo xtask ci`: format, lints, tests, spec, parity, guesses, assets, vendor
names, layering, wasm) is green.

**What is real: Font View and glyph management (M4).** A built-in glyph database
(`crates/glyphdata`, from GlyphsInfo, the Adobe Glyph List and the Unicode Character Database)
gives every glyph name its Unicode value, category, subcategory, case, script, direction and
production name, including suffixed names, ligatures and `uniXXXX` names. For 200,504 names its
category, subcategory and production name equal fontc's; glyphsLib's differences are listed by
cause. Users can add their own `GlyphData.xml` files. Exported fonts carry production names
cleaned as fontc cleans them, and combining marks the database knows get advance 0: the `post`
names equal fontc's and fontmake's for every corpus source but one allow-listed file.
Glyph commands: rename (references follow: components, kerning, groups, glyph order, metrics
keys), the property setters, tags, notes, Update Glyph Info, Find and Replace with regular
expressions, Copy Glyph Names in six formats, Add Glyphs (names, characters, ranges), Duplicate,
Remove with its component and metrics-key answers, copy and paste of whole glyphs between
fonts, Paste Special. `crates/glyphquery` evaluates search, smart filters (nested All, Any and
None groups, Custom predicates), list filters, folders, and the sidebar's categories and
languages (CLDR exemplar characters, 362 languages) with missing-glyph badges.
In the window, Font View has the grid (badges, colour labels, a master choice), List View with 22
sortable columns, the sidebar and search field, the inspector, the glyph context menu, the
Selection Info and Glyph Info windows, the Smart Filter, List Filter, Languages, Missing Glyphs,
Add Glyphs, Remove Glyphs, Paste Special and Find and Replace dialogs, and the Start Window with
recent documents and a chooser that builds a new font from 28 open glyph sets. User settings
persist between runs. A frame after the first takes under 1 ms on Inter's 3,017 glyphs. A second
end-to-end test builds a font from glyph sets over the control channel, renames, filters,
duplicates and exports it. When other glyphs use a removed glyph as a component, the Remove Glyph
dialog's default answer is Decompose.

**What is real: spacing, kerning and text mode (M5).** Two crates without egui hold the work:
`crates/spacing` (metrics keys, sidebearings, the kerning index with groups, exceptions and locks,
Clean Up, Compress, kerning-text) and `crates/textlayout` (Edit View and Text Preview layout with
kerning and writing direction); `crates/features` writes the generated pair kerning of exported
fonts, ported from fontc. Edit View has text mode: a tab holds typed text with a cursor,
placeholders, line breaks and three writing directions, opens from a document's display strings
and writes them back. Metrics keys (`=n`, `=|n`, `=n+10`, local keys) are parsed and evaluated in
dependency order; every one of the 4,413 keys in the corpus parses. The spacing and kerning
shortcuts work in text mode; the Info box edits sidebearings, widths, keys, kerning groups, pair
values and locks; the Kerning window lists, filters, sorts, edits, copies, pastes, compresses and
cleans up pairs; Font View's inspector edits metrics and groups and flags glyphs out of sync. The
Preview Area, the Preview Panel, the measurement line, Text Preview, sample strings and Find Glyphs
are there. Metrics import and export read and write kerning-text and AFM (fontTools' `afmLib`
reads ours back) and copy spacing from another font. Opening a compiled font recovers its pair
kerning: 945,310 pairs from 97 fontmake builds, 0 differences. The kerning lookup gives the value
fontc and fontmake compile for every covered pair of 54 sources (506,285 pairs per oracle) but
one pair, in a source that turns its kern writer off; exported fonts get the same pair kerning (`kern` and
`dist`) for those 54 sources, which have no `kern` feature code, except 27 known differences in two sources. An agent types, spaces, kerns,
compresses and moves the kerning to a second master over the control channel alone (a third
end-to-end test).

**What is not.** Nothing in the editor has been compared with the running reference app. M3's,
M4's and M5's behaviour comes from the handbook; 5 editor rows are confirmed by public tutorial videos. Where
a detail is unconfirmed, the handbook decides (Remove Overlap, for example, deletes open paths).
Where no source says, we choose, and each choice is listed in `docs/guesses.md` to revisit
(675 guesses on 434 rows).

| Gap | Detail |
|---|---|
| `path.sharpenCorner` | moved to the `fidelity` milestone: no written source says what it does, and the tutorial video shows the menu item without running it |
| Text mode | no shaping: text is laid out by code point, kerning and direction until features exist (M8); Text Preview orders right-to-left and left-to-right runs by simple rules of ours (full Unicode bidi at M8); Edit View keeps one direction per line |
| Spacing speed | on Inter, the out-of-sync update after an edit to one glyph takes 7.4 ms against a 2 ms target, and the full scan 53.5 ms against 50 ms |
| Kerning | a source with a hand-written `kern` feature exports without kerning until feature code compiles (M8); vertical kerning is stored and listed but not exported (neither reference compiler writes `vkrn`); automatically aligned glyphs wait for M6; vertical origin and vertical width have no commands; 12 metrics-key values in Inter Italic differ from the stored ones by up to 1.4 units, cause not settled |
| Masters | the toolbar has no master icons (M9) |
| Font View | recipe items in Add Glyphs wait for M6; images in cells wait for M11; filters cannot be renamed in place or dragged in the sidebar (they can by command) |
| Export dialog | OTF, UFO and Metrics tabs; the Variable tab is M9 |
| Import Outlines | the missing-glyph question is asked once for all unmatched files, not once per file |
| Hints | Shape Order and Expand Outline do not renumber hints that refer to a path by index |
| Strokes in files | a UFO keeps no stroke attribute; cap code 4 and the forced cap axis are not confirmed against the reference's files |
| Audit | 34 of the 36 outline checks; the other 264 checks are M12 |

Beyond M5 there is no component editing (components are exported as composites), no
feature-code compilation, no variable fonts and no instances between masters (M9),
no CFF subroutines, no hinting, no web app, no packaging. Glyphs-only data travels in
glyphsLib's UFO lib keys where glyphsLib has a key for it, and alternate and intermediate
layers travel as designspace rules and sparse sources; the data no key holds (glyph tags,
locks, sort names, layer colours, drawing attributes…) does not, and the loss report lists it.
Features and instances are read and written, not yet edited or compiled.

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
| **Breadth:** commands, tools and panels exist | ~22% | 365 of 933 commands; 20 of 33 tool rows usable in the window (19 tools: every M3 tool row, the Annotation row being the group of its six tools, and M5's Text tool); about 65 of 172 panels, many of them partial (Font View's grid, List View, sidebar and inspector, the toolbar, the Info box with its spacing and kerning fields, the Palette and five of its sections, the bottom bar, the Glyph Info, Selection Info, Start, Kerning and Text Preview windows, the Preview Area and Panel, the M3, M4 and M5 dialogs) |
| **Depth:** features behave like the reference | ~7% | the M3, M4 and M5 commands, tools, panels and dialogs behave as the handbook describes them, and none has been compared with the reference app. The kerning lookup equals what fontc and fontmake compile. `path.sharpenCorner` waits in `fidelity`; the gaps are listed above |
| **File interop** | ~38% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFOZ and compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts); static TTF and OTF out, plain, WOFF and WOFF2, from the export commands, the Export dialog and the CLI. Missing: variable fonts, instances between masters, version conversion, stroke attributes in UFO |
| **Compiler correctness** | measured, low | 14 of 50 tables written (`GPOS` with generated pair kerning), for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers; strokes, masks and overlap removal in the outline stage. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). With overlaps removed, 99.58% (TrueType) and 99.87% (CFF) of glyphs have no overlap-removal difference from fontmake's (`docs/plans/M3.md` § 5). The generated pair kerning equals both oracles' on every corpus source without `kern` feature code or a top-level include, but 27 known differences listed in `docs/spec/40-compile.md` § 4a (`docs/plans/M5.md` § 5). No variable font, no feature code, no hinting, no subroutines |
| **Agent automation** | ~55% | every existing command is reachable from CLI, control channel and MCP; the control channel's 19 methods drive the window's tools, pointer, keys, text fields, dialogs and menus; `ui.screenshot` and `ui.render` show the result; an agent can draw, edit and export glyphs, build a font from glyph sets, and type, space and kern text, over the control channel alone (three end-to-end tests). Missing: `ui.click` beyond dialogs, `ui.set`, MCP resources and prompts |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation, corpus runs of overlap removal and the audit); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and the path-operation commands and export run each glyph on a worker thread under a time limit (a glyph that runs out of time stays as it was; the worker cannot be stopped and runs on detached). One adversarial code review, most findings fixed and the rest listed in `docs/status.md`. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | a few measures | overlap removal over the corpus: median 38 µs, 99th percentile 403 µs per glyph layer; one node-drag preview step on Oswald: 142 µs; on Inter (3,017 glyphs) a Font View frame under 1 ms, a frame after an edit 8–10 ms (15–19 ms with every core busy), every glyph's effective properties 21–26 ms, a three-rule smart filter 28–35 ms, the first glyph-database lookup 24 ms; the kerning lookup over 490,911 pairs 39 ms, Update Metrics on all glyphs and masters 85 ms, a text-mode frame with 200 glyphs 5 ms, Text Preview layout of 1,000,000 characters about 210 ms; the out-of-sync update after one edit 7.4 ms, over its 2 ms target. No budget suite for the targets in `docs/architecture.md` |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: 24 crates build for the browser; no web app |
| **Look and feel** | barely started | a functional shell with the siblings' default dark tokens; the toolbar, panels, dialogs and canvas overlays take their colours and sizes from the tokens, with drawn icons for the tools, the Transformations buttons and the Stroke section; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

1. **The owner's answers to `docs/plans/M6.md` § 7**: accepting the plan, mark attachment at
   export in M6 or M8, the composition data, caps, segments and brushes in M6 or M13, the
   TrueType default for overlapping and transformed components, the milestone moves, and whether
   anchor propagation is ported from fontc (whose code derives from the reference app's source)
   or written from the oracles' outputs.
2. **M6, components and anchors**, package by package in the order of `docs/plans/M6.md` § 6,
   starting with M6.0 (the spec pass that applies those answers).

Then M6–M7 on the editor track and M8–M12 on the engine track, as `docs/execution-plan.md` § 4
orders them. Left from M3 to M5 for later milestones: Add Glyphs recipes and automatically aligned
glyphs (M6), shaping and kerning with feature code (M8), the toolbar's master icons and the
Export dialog's Variable tab (M9). Left from M2: instances between masters (M9), CFF
subroutines (before M12), the compile plan's `features` section, and the CLI's `--name-by`, `--output-layout` and pattern instance
selection (`docs/plans/M2.md` › M2.5).

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (933 commands, 33 tools, 172 panels, 46 settings, 241
  parameters, 1,754 behaviour rules, 300 checks, 70 CLI rows, 49 formats, 50 tables) and 23
  behaviour notes (about 150,000 words), all in our own words. Every M3 command's `params` name
  its targets; shortcuts carry a `when` (the context they apply in). Every choice of ours where
  no source says what the reference does is a row's `guesses` entry, listed in `docs/guesses.md`.
- **Workspace:** 25 library crates in enforced layers, two apps, `xtask`; about 168,000 lines of Rust.
- **Glyph database and queries (M4):** `crates/glyphdata` (standalone): GlyphsInfo 1f126bd, the
  AGL and AGLFN, and UCD 17.0 in one 538 KB compressed blob, with lookup, synthesis of suffixed,
  ligature and `uniXXXX` names, production names, nice names, name validation, sort keys,
  directions, search, and custom `GlyphData.xml` files under limits; CLDR 48.2 exemplar characters
  for 362 languages and 28 Google Fonts glyph sets in a 41 KB blob. `cargo xtask glyphdata` and
  `cargo xtask glyphsets` fetch, generate and (for the database) cross-check against fontc's
  glyphs-reader and glyphsLib. `crates/glyphquery`: search, smart-filter rule trees, the glyph
  predicate language, list filters and folders, sidebar entries with counts and missing-glyph
  badges. The model has effective glyph properties with their sources, one glyph order and
  sorted placement; the engine holds the glyph data, user settings and a clock for Last Changed.
- **Glyph management (M4):** 62 commands: renames that follow references, property setters,
  tags, notes, Update Glyph Info, Find and Replace with regular expressions and Match Case,
  Copy Glyph Names, Add Glyphs, Duplicate, Remove with its answers, Add to Font, whole-glyph copy
  and paste, Paste Special, glyph data queries, filters, search, settings, recent documents, New
  from Glyph Sets. Font View in the window: grid, List View, status area, sidebar and search,
  inspector, context menus, Selection Info, Glyph Info, the Start Window and nine dialogs; the
  dialog framework gained multi-line text, a third button, scrolling lists and a rule-tree
  control.
- **Spacing and kerning (M5):** `crates/spacing` (no egui): metrics keys (parse, evaluate in
  dependency order, the out-of-sync cache), sidebearings and widths, Transform Metrics, the
  kerning index (glyph, group and exception lookup in the oracles' order, linked masters,
  directions), locks, Clean Up, Compress, group renames and kerning-text. 97 commands: the
  spacing setters and shortcuts, Update Metrics, kerning pairs, groups, exceptions and their
  shortcuts, the Kerning window's list, filter, sort, copy, paste and delete, text mode, previews,
  the measurement line, Text Preview, sample strings, Find Glyphs, and metrics import and export
  (kerning-text, AFM, another font). `crates/textlayout`: text items, placeholders, line breaks,
  directions and kerning for Edit View and Text Preview. `crates/decompile` recovers pair kerning
  from a compiled font's `GPOS`. `crates/features`: the generated `kern` and `dist` features of
  exported fonts, ported from fontc's kern writer, and an evaluator that reads a compiled font's
  pair kerning. In the window: text mode in Edit View, the Info box's spacing and kerning fields,
  the Kerning window, Font View's metric and group fields and out-of-sync badge, the Preview Area
  and Panel, Text Preview, the Transform Metrics, Rename Kerning Group, paste-conflict and metrics
  dialogs, and the Export dialog's Metrics tab. Measured against fontc and fontmake on the corpus
  and Inter (`docs/plans/M5.md` § 5). Text Preview keeps left-to-right runs in order inside
  right-to-left text, and its Writing Direction ranges act as embeddings (simple rules in
  `textlayout::bidi`). Preview Descender defaults to `winDescent`, which departs from the
  handbook's wording. The owner accepted M5 with the two out-of-sync speed misses, which stay open.
- **M6 plan:** `docs/plans/M6.md`: 220 rows (59 commands), every recalled row checked against the
  handbook or the oracles, 35 of the 39 open questions in `docs/spec/09-reusing-shapes.md`
  answered, 11 rows moved between milestones, 20 packages and the § 5 measures.
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
  (M3.15): the Info box's selection part with its reference point and its annotation palette while an Annotation tool is chosen, the Palette with Dimensions,
  Fit Curve, Layers, Transformations and Stroke sections that collapse, and the bottom bar's
  zoom controls; number fields follow the field rules (Return, Tab, Escape, arrows with
  Shift). Dialogs (M3.14): a form over one command's parameters with a live preview inside an
  interaction, OK as one undo step and Cancel back to the font the dialog opened on. Fields
  include numbers, choices, multiple choices, folders, lock buttons, a reorderable list and
  "…" action menus, grouped under headings or tabs. Every filter with parameters gets a dialog
  from its schema; Round Corners, Transformations, Shape Order, Interpolate with Background,
  Primitive Measurements, Guide Properties, Export and two follow-up questions have their own
  files. Runs in a window and in headless test frames.
- **Gates:** `cargo xtask ci` with ten steps; parity and guesses reports generated from the inventory.
  Outside the gate: `cargo xtask roundtrip` over 527 corpus sources, `cargo xtask overlap`
  (overlap removal on every corpus layer) and `cargo xtask audit`.

## Milestones

This file does not estimate time: estimates of agent hours have proved unreliable. Progress is
measured by what works (the parity table below and the measures in each milestone's plan).

| # | Milestone | Status | Commands (live) |
|---|---|---|---:|
| M0 | Skeleton and vertical slice | ✅ done | 5 (5) |
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | ✅ done: every format read and written back byte for byte; the typed model (M1.7) with glyphsLib's lib keys, intermediate and alternate layers in a designspace, and its measures met; the gaps left on purpose are in `docs/plans/M1.md` (M1.4 and M1.7) | 10 (10) |
| M2 | Compile core: static TTF and OTF, oracle harness | ✅ done: the oracle harness, the staged pipeline, the core tables at fontc and fontmake parity (no table different), static CFF, WOFF and WOFF2 with transforms, the export commands and CLI `compile`; every measure met (`docs/plans/M2.md` § 5). Deferred on purpose: instances between masters (M9), overlap removal (M3), CFF subroutines, export dialogs (M3) | 5 (5) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | ✅ every measure met: every package M3.0 to M3.18 built; 184 of 184 commands live, gesture equals command for all 19 tool rows, an agent draws and exports over the control channel, overlap removal at export coverage-equal to fontmake's in over 99.5% of glyphs, M1 and M2 baselines unchanged (`docs/plans/M3.md` § 5). `path.sharpenCorner`, which no source describes, moved to `fidelity` | 184 (184) |
| M4 | Font View and glyph management | ✅ every measure met: every package M4.0 to M4.12 built; 62 of 62 commands live; the glyph database equal to fontc's on category, subcategory and production name for 200,504 names; `post` names equal both oracles' except one allow-listed source; every M4 dialog equals its command; an agent builds a font from glyph sets over the control channel; M1 to M3 baselines unchanged (`docs/plans/M4.md` § 5) | 62 (62) |
| M5 | Spacing, kerning and text mode | ✅ every measure met but the two out-of-sync speed targets: every package M5.0 to M5.18 built; 97 of 97 commands live; every corpus metrics key parses; the kerning lookup equals fontc's and fontmake's compiled value for 506,285 pairs per oracle but one source that turns its kern writer off; exported fonts carry the same generated pair kerning but 27 known differences in two sources; kerning read back from 97 compiled fonts with 0 differences; every M5 dialog equals its command; an agent types, spaces and kerns text over the control channel; M1 to M4 baselines unchanged. Missed: on Inter the out-of-sync update after one edit takes 7.4 ms (target 2) and the full scan 53.5 ms (target 50) (`docs/plans/M5.md` § 5) | 97 (97) |
| M6 | Components and anchors | ⬜ planned: `docs/plans/M6.md` waits for the owner's answers (§ 7) | 59 (0) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | ⬜ | 62 (2) |
| M8 | OpenType features | ⬜ | 40 (0) |
| M9 | Interpolation and variable fonts | ⬜ | 67 (0) |
| M10 | Hinting (PostScript and TrueType) | ⬜ | 57 (0) |
| M11 | Colour fonts and images | ⬜ | 50 (0) |
| M12 | Command-line tool suite (61 of the 70 subcommand rows) and quality checks (263 of 300) | ⬜ | 71 (0) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | ⬜ | 132 (0) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 28 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 3 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 933 (365) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 45 | 27 | 20% | tabs, zoom and scroll work; user settings kept between runs; the Start Window with recent documents; the toolbar with shared buttons and their menus; tools chosen by their letters, shared letters cycle; context menus; menu items greyed out by the selection; bottom bar zoom controls. Missing: settings window, shortcut editor, workspaces, title bar, sidebars, the toolbar's master icons (M9); nothing compared with the reference |
| Font View | 6 | 63 | 55 | 38% | grid, List View, sidebar with categories, languages and filters, search, the glyph database, Add Glyphs, renaming with references, the inspector, Selection Info, Glyph Info, the Start Window, the glyph set dialogs, the inspector's metric and kerning-group fields and the out-of-sync badge. Missing: recipes (M6), images in cells (M11); nothing compared with the reference, and many choices are guesses |
| Edit View text mode | 4 | 25 | 18 | 40% | typing, the Text tool, cursor keys, selection, clipboard as text, three directions, three kerning modes, kerned-gap colours, metrics labels, tabs from display strings, the Preview Area and Panel, the measurement line, Text Preview, sample strings, Find Glyphs. Missing: shaping (M8), Latin letters inside right-to-left runs in Text Preview; nothing compared with the reference |
| Drawing and editing outlines | 10 | 96 | 79 | 25% | node, segment, transform, clipboard and selection commands; in the window Select, Lasso, Hand, Zoom, Draw, Pencil, Rectangle, Ellipse, Rotate and Scale, the bounding box and its knobs, context menus, the Info box, the Palette's Transformations and Fit Curve sections, the Transformations and Primitive Measurements dialogs, editing in any master. Missing: `path.sharpenCorner` (no source, in `fidelity`); nothing compared with the reference |
| Path operations and filters | 6 | 66 | 40 | 28% | Remove Overlap, the booleans, path direction, extremes and inflections, Tidy up, Open Corner, Reconnect, Cut with Line, curve conversion and rounding as commands; the Knife; the six filters with dialogs and export strings; strokes and masks drawn on the canvas and applied at export; the Stroke section and the Shape Order dialog. Missing: stroke attributes in UFO, hints renumbered after Shape Order and Expand Outline; nothing compared with the reference |
| Reusing shapes | 7 | 77 | 0 | 0% | everything; components are only exported (as composites, decomposed where the format needs it) |
| Layers | 3 | 21 | 19 | 35% | background and layer commands; Edit Background; the active layer and the tab's master; the background and other visible layers drawn on the canvas; the Palette's Layers section with its row context menu; the Interpolate with Background dialog. Font View's master choice. Missing: nothing compared with the reference |
| Guides, metric lines, zones, grid | 3 | 36 | 16 | 26% | guide, ruler, coordinate and Dimensions commands; metric lines, guides, the grid, CJK lines and measurement guides drawn; guides dragged and edited on the canvas, with their context menu and the Guide Properties dialog; the Measurement tool; snapping to guides and their crossings; the Palette's Dimensions section. Missing: script-specific metric lines; nothing compared with the reference |
| Spacing and kerning | 7 | 79 | 65 | 40% | metrics keys, sidebearing and width setters, the spacing and kerning shortcuts in text mode, Update Metrics, Transform Metrics, kerning pairs, groups, exceptions and locks, Clean Up, Compress, group rename; the Info box's spacing and kerning fields, the Kerning window, Show Group Members, the Font View inspector fields and out-of-sync badge, metrics import and export (kerning-text, AFM), kerning read from compiled fonts; generated pair kerning in exported fonts, the lookup equal to fontc's and fontmake's compiled values over the corpus. Missing: kerning with a hand-written `kern` feature (M8), vertical kerning at export, automatically aligned glyphs (M6), vertical origin and width commands, the out-of-sync update at its speed target; nothing compared with the reference; many choices are guesses |
| Font Info | 6 | 48 | 2 | 2% | four font-wide fields can be read and set |
| Interpolation | 8 | 68 | 1 | 1% | Edit View draws and edits any master (`view.selectMaster`); everything else |
| OpenType layout | 7 | 46 | 0 | 3% | the generated `kern` and `dist` features, following the source's feature-writer setting. Missing: feature code, classes, the other automatic features, the mark writer (M8) |
| PostScript hinting | 3 | 27 | 0 | 0% | everything |
| TrueType hinting | 4 | 40 | 0 | 0% | everything |
| Colour fonts and images | 3 | 46 | 0 | 0% | everything |
| Source formats | 6 | 14 | 8 | 60% | Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; UFO 1; shape groups, gradients and shadows kept as stored values; right-to-left, vertical and contextual kerning in UFO; version conversion |
| Compiling | 9 | 11 | 5 | 17% | static TTF and OTF, plain, WOFF and WOFF2, from the commands and the Export dialog; production names and database mark widths; generated pair kerning in `GPOS`; linked metrics and Default Layer Width; strokes expanded, masks applied and overlaps removed (coverage-equal to fontmake's in over 99.5% of glyphs); core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source. Missing: variable fonts and instances between masters (M9), feature code (M8), hinting (M10), CFF subroutines, 36 of 50 tables |
| Preview, proofing, printing | 2 | 23 | 15 | 12% | the Preview Area and Preview Panel (invert, flip, blur, offset), Text Preview with its text, style, size, direction and find, sample strings. Missing: printing, proof sheets, shaping (M8); nothing compared with the reference |
| Command-line tools | 8 | 42 | 4 | 3% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `tables list`, `glyphs list`, `oracle`; no dump, subset, instance, merge |
| Quality checks | 3 | 33 | 8 | 10% | 34 of the 36 outline checks through `audit.run`; `audit.fix` applies eight fixes through path commands and each clears its check; live audit and canvas markers. Missing: the other 264 checks (M12); nothing compared with the reference |
| Plug-ins and scripting | 3 | 16 | 0 | 0% | everything |
| Agent surfaces | 4 | 11 | 3 | 42% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; tools, pointer, key, text, menu and dialog control over the control channel, enough to draw and export a font, to build one from glyph sets, and to type, space and kern text (three end-to-end tests); shortcuts sent with `ui.key` open their dialogs as the keyboard does. Missing: `ui.click` beyond dialogs, `ui.set`, MCP resources and prompts |
| **Feature parity** | **117** | **933** | **365** | **~18%** | |

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
