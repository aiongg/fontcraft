# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-13 (the first waves of M6 to M12, the second-wave packages M6.14, M7.14 to M7.18 and M8.15, the 22 first-wave packages of M13, and the builder packages M8.9, M8.7, M7.8 and M6.10 built, integrated and gated; the automation file roots)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,682 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, pointer and key injection, toolbar, context menus, dialogs, panels, tests | **~65%** of what the siblings' "infrastructure" row covers; missing: the settings window, the MCP HTTP transport, a performance budget suite |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~25%** |
| Commands that exist (not: that behave like the reference) | **660 of 960** (`docs/parity.md`) |
| Packages built, M6 to M13 | **125 of 277** (M6 16 of 20, M7 18 of 21, M8 16 of 25, M9 8 of 27, M10 11 of 33, M11 6 of 21, M12 26 of 68, M13 24 of 62; counted from commit subjects, M8.7b with M8.7) |
| Observed against the running reference | **52 rows**: 47 compiled-output rows checked against fontc and fontmake, and 5 editor rows confirmed by public tutorial videos of the previous major version. The reference app itself has not been run |

## Honest assessment (2026-10-12)

**In one line:** the project reads and writes `.glyphs`, UFO and designspace sources without loss
and compiles static fonts that match fontc and fontmake, overlaps removed and with production names;
its Edit View draws and edits outlines with 18 tools and types, spaces and kerns text, its Font
View manages the glyph set with a glyph database, filters, search and an inspector, and exported
fonts carry pair kerning equal to fontc's and fontmake's except 27 listed known differences.
The first waves of M6 to M12 add components, Font Info, feature code, variable-font tables, a
PostScript autohinter, colour layers, command-line table tools and 157 quality checks. The second
wave adds the Font Info window with all its tabs, projects, Add Glyphs recipes and the feature
compile state. The first wave of M13 adds a command console, macros, stem filters, curve tension,
export profiles, external-change review and file-confined automation. A later batch compiles
feature code into exports (GSUB, GPOS and GDEF compared with fontmake and fontc lookup by
lookup), regenerates automatic feature code at export, names static instances and fills their
OS/2 table as fontmake does, and draws and selects components and anchors in Edit View. Much of
the rest runs by command and CLI only, and much of it does not yet reach exported fonts.
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

**What is real: editing by command (M3).** Of the 960 commands, all 184 of M3, all 62 of M4 and all 97 of M5 exist.
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

**What is real: the first waves of M6 to M12.** Each milestone's first-wave packages were written
in parallel against its plan's interface skeleton, then integrated, reviewed and gated together.
They run as commands and, for M12, as CLI subcommands. The second-wave packages that follow the
table add M7's window.

| Milestone | What works |
|---|---|
| M6 | the component index, anchor propagation, automatic alignment, composition recipes, smart and corner components drawn and exported, the mark writers and `GDEF`; component, anchor, smart glyph, corner component and decompose commands; Add Glyphs builds recipes and composes database composites; Edit View draws and selects components and anchors (M6.10, below) |
| M7 | the custom-parameter catalogue, Font Info fields at every scope, masters, instances, metric and stem definitions, Scale to UPM, export filters, glyph sets and export folders, linked sources, text tokens, the legacy `kern` table; projects; the Font Info window; the name table and OS/2 of static instances as fontmake builds them (M7.8) |
| M8 | the fea-rs wrapper with its guards, feature assembly with a source map, the syntax extensions and tokens, 60 automatic feature generators, feature entry commands, feature files and snippets; the compile state (errors with four fixes, completion, token evaluation). Feature code is compiled into exports (`GSUB`, `GPOS`, `GDEF`) and the automatic entries are regenerated there (M8.7, M8.9) |
| M9 | axis ranges, mappings and normalisation as the oracles derive them; glyph sources and the compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, the variable-font names and `STAT`, byte-equal to fontc's on test sources; feature variations from alternate layers; variable TrueType and CFF2 fonts open into axes, masters and instances; the Compare Fonts and Compare Family commands. None of the variable tables reaches an export yet |
| M10 | the PostScript autohinter, ported from otfautohint and agreeing with AFDKO 5.0.1 on its oracle scripts' cases, with its reports and override sets; typed hint data and settings in every stored form; the TrueType assembler; `gasp` at export; hinted previews through skrifa's interpreter. Exported fonts carry no hints yet |
| M11 | colour palettes in their three stored forms, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits and paths rebased on save; the `SVG ` table from vector layers at export. No `COLR`, `CPAL`, `sbix` or `CBDT` |
| M12 | table actions and `tables show`, collections, WOFF compress and decompress, TTX, JSON and hex dumps of the 17 core tables, the layout normaliser, feature list and decompile, glyph rename and reorder, the metrics report, dehint, outline repairs on whole fonts, Type 1, bare CFF and SVG font reading (SVG fonts written too), variations map and build-avar, SVG and HTML proofs, source diff and merge, `bench` and `completion`; audit data sets, 157 checks, audit profiles, the ignore list and report export. Tested against small oracle outputs; the corpus measures of `docs/plans/M12.md` § 5 have not been run |

Adversarial reviews of the M9 to M12 code found unbounded work and allocation on hostile input;
every blocking finding is fixed with a test that failed before the fix.

**What is real: second-wave packages and the first wave of M13.** Seven second-wave packages and
all 22 first-wave packages of M13 are integrated and gated with the rest. None is compared with
the reference app; the M13 packages follow the secondary references' public documentation.

| Package | What works |
|---|---|
| M6.14 | Add Glyphs builds recipes and composes database composites (Add to Font too); the Components cell edits the shown master; Add Component, Create Composite, Decompose and Set or Reset Anchors act on the Font View selection as one step; five component smart-filter rules; glyphs whose name starts with an underscore do not export |
| M7.14 | projects: the `.fontcraftproject` file, 14 commands, project documents with their own instances linked to a font read from disk and never written |
| M7.15 to M7.18 | the Font Info window (Cmd+I) with six tabs: shared rows with selection, Alt and minus, localizable sub-rows and value editors; the Font tab (family-name warning, UPM scale prompt, version stepper, creation date); Other Settings; Notes with Markdown highlighting; the Masters tab (master list with reorder, 25 drawn icons, metric editor, multiple values); the Exports tab (export list, style linking, instance preview). Instances between masters show a message until M9 interpolates them |
| M8.15 | the feature compile state: cached per document while the code is unchanged, errors mapped to entry, range, line and column with four fixes, jump and details, token evaluation, completion, name preview, syntax-colouring data. The preview font waits for M8.19 |
| M13 editing | curve tension, Tunni lines and G2 or G3 harmonizing; ink traps, corner helpers, geometry helpers (Make Parallel, Space Points, Make Arc, Slice Along Guide); stem detection and the four stem filters with dialogs; the Thickness tool with per-node stroke widths exported by a variable-width stroker; Revert Glyph, Copy Layer to Layer, Paste After, Compare Layers; Detect Composites and Detect Component on an outline matcher |
| M13 glyph data | anchor and automatic-layer expressions that follow width, metric, guide, zone and parameter changes; Show Dependencies and the Related Glyphs section; stored selections and a reference font behind the edited glyph; status definitions with a canvas tint |
| M13 files | export profiles (OpenType PS, OpenType TT, Web, user profiles) with one staged write, and glyph artwork as SVG, PDF or PNG; the three-way external-change review on window focus; Filter by Encoding File, Rename Glyphs by Name List, Compare Fonts in List; PFM and AFM metrics import; UFO 1 sources open (converted to UFO 3); EPS outlines read through a fixed PostScript subset (Import Outlines accepts them); SFD and SFDir import in `crates/io-sfd` (not yet reachable from File › Open or the CLI) |
| M13 automation | the Command Console (command and run-style lines, completion, history, log); the `.fcmacro` format with eight script commands; the `fontcraft://` URL scheme; MCP permissions (`--read-only`, `--allow`, `--deny`), five workflow prompts, resources, completion and logging; file confinement for MCP and the control channel (below) |

**Automation file confinement.** `fontcraft-cli mcp` takes `--automation-read-root DIR` and
`--automation-write-root DIR`; the desktop app's control channel takes the same flags or the
variables `FONTCRAFT_AUTOMATION_READ_ROOT` and `FONTCRAFT_AUTOMATION_WRITE_ROOT`. Without them,
file access is unrestricted. Reads and writes are separate. While a root is in force, every file
a command names, every implicit place (export folders, packages, snippet folders, screenshots)
and every linked file is checked against it, and a linked file outside the read root reads as
missing. `docs/mcp.md` has the details.

**Hardening.** Fix passes bounded the work done on hostile input in the EPS, SFD, UFO 1 and PFM
readers, features previews, macros, outline matching, intersections and stem filters; name-table
reads are bounded; exports across several profiles write all of their files or none. `avar`
segment maps for axis maps outside the axis range match fontmake's (`docs/spec/40-compile.md`).
Detect Composites keeps automatic alignment on where it reproduces the detected placement and
turns it off only where alignment would move a component (a guess on the row).

**What is real: the builder packages M8.9, M8.7, M7.8 and M6.10.** Integrated and gated with the
rest. The compiled output is measured against fontmake and fontc; the editor behaviour follows
the handbook and is not compared with the reference app.

| Package | What works |
|---|---|
| M8.9 | automatic feature code: the framework, Update (merging by the handbook's rules), Update Entry, the automatic classes, and regeneration of the existing automatic entries at export. Over the corpus sources, 69 entries equal the reference compilers' text, 23 differ (stale sources, older reference forms, samples) and 3 are removed (`cargo test --release -p fontcraft-features --test corpus_auto -- --ignored --nocapture`). `salt` sits beside `ss01`; Uppercase, Lowercase and AllLetters are optional; Update removes entries whose glyphs are gone (guesses on the rows) |
| M8.7, M8.7b | layout tables at export: feature code compiled with the generated kerning and mark attachment into `GSUB`, `GPOS` and `GDEF`; the oracle compares each table lookup by lookup (numbers under Compiler correctness). The mark writer applies `*origin` anchors and closes `abvm` over the feature code; UFO `GDEF` classes come from the stated categories; kerning floats are kept. Feature includes may climb out of the source folder as the reference compilers read them; absolute include paths are refused, and under automation read roots each include passes the read check. The marks measure passes with its known rows |
| M7.8 | the name table and OS/2 of static instances as fontmake builds them: style linking, `familyNames` and `styleNames`, Weight and Width classes. 1,678 compared values are equal and 120 differ, from three allow-listed causes. Exports carry the version stamp `;FontCraft <version>` unless the `versionStamp` parameter turns it off. Mac records are written, and unsupported encodings are dropped with a warning. Prevent Name ID adds up across levels (a guess on the row). Left: `plan::postscript_name` and `plan::file_stem` do not expand text tokens, and a `.glyphs` 2 instance whose PostScript metric is 2^63 fails |
| M6.10 | Edit View draws components by alignment state (green aligned, grey manual, blue horizontal only), the three placeholders, anchors at their metric position with names when selected (View › Show Anchors) and corner tints, and shows component nodes as snap targets during drags; composites redraw while their base glyph is edited. Gestures run their commands: click, Option rubber band (`selection.selectInRegion` with `components`), two-stage Select All, Tab through components, double-click (`component.editBaseGlyph`), Option-drag duplicate, Delete, and the smart handles and the Width and Height box knobs (`smartComponent.setValue`). A text-mode frame with 200 composites takes 5.9 ms (target 16 ms). Left: selecting and deleting corner components (needs a hint selection kind) and clicking a missing-base placeholder |

**Test layout and fixes.** The 93 engine integration-test files are modules of one test binary
(`crates/engine/tests/engine/main.rs`; `cargo test -p fontcraft-engine --test engine <filter>`),
and the decompile, io-glyphs, io-ufo and plist tests are one binary each. The `measure` profile
(release optimisation without link-time optimisation) serves speed measures and oracle runs. The
web build compiles. Every export path reads feature includes from the source's include
folder, and the code editor's compile uses the same folder. A `.glyphs` 3 file keeps deleted
kerning deleted and keeps a version 2 kerning key the reader did not take kerning from. An include
candidate the include check refuses does not hide an allowed one; the window between the check
and the open is documented as accepted.

**What is not.** Nothing in the editor has been compared with the running reference app. M3's,
M4's and M5's behaviour comes from the handbook; 5 editor rows are confirmed by public tutorial videos. Where
a detail is unconfirmed, the handbook decides (Remove Overlap, for example, deletes open paths).
Where no source says, we choose, and each choice is listed in `docs/guesses.md` to revisit
(1,683 guesses on 1,089 rows).

| Gap | Detail |
|---|---|
| `path.sharpenCorner` | moved to the `fidelity` milestone: no written source says what it does, and the tutorial video shows the menu item without running it |
| Text mode | no shaping: text is laid out by code point, kerning and direction until features exist (M8); Text Preview orders right-to-left and left-to-right runs by simple rules of ours (full Unicode bidi at M8); Edit View keeps one direction per line |
| Spacing speed | on Inter, the out-of-sync update after an edit to one glyph takes 7.4 ms against a 2 ms target, and the full scan 53.5 ms against 50 ms; the per-edit follow-up hook takes 2.5 to 2.7 ms, almost all of it the alignment pass (2.4 ms), against 2 ms |
| Kerning | vertical kerning is stored and listed but not exported (neither reference compiler writes `vkrn`); automatically aligned glyphs wait for M6; vertical origin and vertical width have no commands; 12 metrics-key values in Inter Italic differ from the stored ones by up to 1.4 units, cause not settled |
| Masters | the toolbar has no master icons (M9) |
| Font View | images in cells wait for M11; filters cannot be renamed in place or dragged in the sidebar (they can by command); each automatic composite copies the glyph list, so adding very many composites at once is slow |
| Export dialog | OTF, UFO and Metrics tabs; the Variable tab is M9 |
| Import Outlines | the missing-glyph question is asked once for all unmatched files, not once per file |
| Hints | Shape Order and Expand Outline do not renumber hints that refer to a path by index |
| Strokes in files | a UFO keeps no stroke attribute; cap code 4 and the forced cap axis are not confirmed against the reference's files |
| Detect Composites | on Inter it finishes in under 2 s but rebuilds only 45.4% of the composites from the same bases (target 95%): it picks identical-outline bases such as `acute` for `acutecomb`, and composite bases instead of atoms |
| Audit | 34 of the 36 outline checks and 157 of M12's 240 checks; `audit.run` does not yet apply profiles, the ignore list, thresholds or project values (M12.44), and there is no `check` subcommand |

Exported fonts have no variable fonts and no instances between masters (M9.4, M9.5), no hints
(M10.14, M10.19), no `COLR`, `CPAL` or bitmap colour tables and no CFF subroutines; the tools and
panels of M6's later UI packages are not built; there is no web app and no
packaging. Glyphs-only data travels in
glyphsLib's UFO lib keys where glyphsLib has a key for it, and alternate and intermediate
layers travel as designspace rules and sparse sources; the data no key holds (glyph tags,
locks, sort names, layer colours, drawing attributes…) does not, and the loss report lists it.
Instances between masters are read and written, not yet compiled.

**How much to trust the inventory.** It was compiled from public documentation in one day, each
source read once. Of its 3,682 rows, 2,123 come from the primary reference's handbook, 1,060 from
other public documentation or permissive source, 439 are our own design, 8 rest on
recollection, and 52 are observed: 47 compiled-output rows checked against fontc and fontmake,
and 5 editor rows confirmed by public tutorial videos. **No other row has been checked against
the running reference app.** An independent re-reading of 209 sampled handbook rows against the
handbook found 1 wrong and 14 imprecise (`docs/spec/README.md`). The notes list about 450 open
questions the documentation does not answer. Treat rows as a careful first reading, and each
milestone's rows as due a planning pass before they are built (`docs/execution-plan.md` § 3).

### By dimension

| Dimension | Estimate | Evidence and what is missing |
|---|---|---|
| **Breadth:** commands, tools and panels exist | ~33% | 660 of 960 commands; 20 of 33 tool rows usable in the window (19 tools: every M3 tool row, the Annotation row being the group of its six tools, and M5's Text tool; the Thickness tool runs by `tool.select` and has no toolbar button); about 80 of 172 panels, many of them partial (Font View's grid, List View, sidebar and inspector, the toolbar, the Info box with its spacing and kerning fields, the Palette and five of its sections, the bottom bar, the Glyph Info, Selection Info, Start, Kerning and Text Preview windows, the Preview Area and Panel, the Font Info window with its tabs, the Command Console, the Related Glyphs, Selections and Reference Font sections, the M3 to M5 dialogs and the M13 dialogs) |
| **Depth:** features behave like the reference | ~9% | the M3, M4 and M5 commands, tools, panels and dialogs behave as the handbook describes them, and none has been compared with the reference app. The kerning lookup equals what fontc and fontmake compile. The first waves of M9 to M12 match their oracles where tested (the variable tables fontc's on test sources, the autohinter AFDKO's scripts, the table tools small oracle outputs), but most of their work is not yet in exports or the window. The Font Info window and the M13 packages follow the handbook and the secondary references' public documentation, with 1,658 guesses listed. `path.sharpenCorner` waits in `fidelity`; the gaps are listed above |
| **File interop** | ~40% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFO 1 and UFOZ open; EPS outlines and, in `crates/io-sfd`, SFD sources can be read (SFD is not yet reachable from File › Open); compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts); variable TrueType and CFF2 fonts open into axes, masters and instances; Type 1 (PFA, PFB), bare CFF and SVG fonts open, and SVG fonts are written; static TTF and OTF out, plain, WOFF and WOFF2, from the export commands, the Export dialog and the CLI. Missing: variable fonts and instances between masters out, version conversion, stroke attributes in UFO |
| **Compiler correctness** | measured, low | the core tables, `GPOS` (generated pair kerning and marks), `GDEF`, `gasp` and `SVG ` written for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, `STAT` and feature variations built and byte-equal to fontc's on test sources, but not yet part of an export; strokes, masks and overlap removal in the outline stage. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). With overlaps removed, 99.58% (TrueType) and 99.87% (CFF) of glyphs have no overlap-removal difference from fontmake's (`docs/plans/M3.md` § 5). The generated pair kerning equals both oracles' on every corpus source without `kern` feature code or a top-level include, but 27 known differences listed in `docs/spec/40-compile.md` § 4a (`docs/plans/M5.md` § 5). Layout tables are compared lookup by lookup (`crates/oracle`, `docs/plans/M8.md`): against fontmake, `GSUB` 25 of 26 sources equal, `GPOS` 82 of 88 and `GDEF` 47 of 55; against fontc, `GSUB` 29 of 46, `GPOS` 80 of 91 and `GDEF` 42 of 68. The remaining differences are variable fonts (M9) and bracket and colour glyphs (M9, M11), each listed in `docs/spec/40-compile.md`. The name table and OS/2 of static instances equal fontmake's in 1,678 compared values and differ in 120, from three allow-listed causes (`docs/spec/40-compile.md`). The automatic feature code regenerated at export equals the reference compilers' in 69 of 95 compared entries; 23 differ (stale sources, older reference forms, sample fonts) and 3 are removed. No variable font, no hinting and no subroutines in exports |
| **Agent automation** | ~55% | every existing command is reachable from CLI, control channel and MCP; MCP has permissions, five workflow prompts, resources, completion and logging, and MCP and the control channel can be confined to read and write folders; the control channel's 19 methods drive the window's tools, pointer, keys, text fields, dialogs and menus; `ui.screenshot` and `ui.render` show the result; an agent can draw, edit and export glyphs, build a font from glyph sets, and type, space and kern text, over the control channel alone (three end-to-end tests). Missing: `ui.click` beyond dialogs, `ui.set`, the MCP HTTP transport |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation, corpus runs of overlap removal and the audit); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and the path-operation commands and export run each glyph on a worker thread under a time limit (a glyph that runs out of time stays as it was; the worker cannot be stopped and runs on detached). Adversarial code reviews of the skeleton and of each milestone's first wave, every blocking finding fixed with a test and the rest listed in `docs/status.md`. Later fix passes bounded the work of the EPS, SFD, UFO 1 and PFM readers, features previews, macros, outline matching, intersections and stem filters, and made multi-profile exports all-or-nothing. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | a few measures | overlap removal over the corpus: median 38 µs, 99th percentile 403 µs per glyph layer; one node-drag preview step on Oswald: 142 µs; on Inter (3,017 glyphs) a Font View frame under 1 ms, a frame after an edit 8–10 ms (15–19 ms with every core busy), every glyph's effective properties 21–26 ms, a three-rule smart filter 28–35 ms, the first glyph-database lookup 24 ms; the kerning lookup over 490,911 pairs 39 ms, Update Metrics on all glyphs and masters 85 ms, a text-mode frame with 200 glyphs 5 ms, Text Preview layout of 1,000,000 characters about 210 ms; the out-of-sync update after one edit 7.4 ms, over its 2 ms target; the colour-layer update after a one-glyph edit 35.5 µs; console completion at worst 2.28 ms (target 5 ms, met); the per-edit follow-up hook 2.5 to 2.7 ms (target 2 ms, missed; the alignment pass is 2.4 ms of it); Detect Composites on Inter under 2 s (met); an Edit View text-mode frame with 200 composites 5.9 ms (target 16 ms, met). No budget suite for the targets in `docs/architecture.md` |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: 24 crates build for the browser; no web app |
| **Look and feel** | barely started | a functional shell with the siblings' default dark tokens; the toolbar, panels, dialogs and canvas overlays take their colours and sizes from the tokens, with drawn icons for the tools, the Transformations buttons and the Stroke section; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

1. **The skeleton pass for waves 2 to 5**, so their packages can again be written in parallel:
   fix the interfaces the later packages of each plan share.
2. **The builder packages** the waves left, one at a time in the main checkout: M9.4 (static
   instances), M9.5 (the variable compile), M9.9, M9.10, M10.3, M10.14 and M10.19 (hints at
   export), M11.5 (paint attributes in the formats), M12.44 (the audit runner), M12.17, M13.2,
   and the rest each plan's § 6 lists.
3. **The Detect Composites base choice (M13.15 follow-up):** prefer `*comb` bases among bases
   with identical outlines, and never use a detected composite as a base, to lift the recovery
   from 45.4% toward the 95% target. Also bring the per-edit alignment pass (2.4 ms on Inter)
   under the 2 ms target.
   Also: M8.11 (the ligature, fraction and aalt generators, which the automatic feature
   framework waits for), token expansion in `plan::postscript_name` and `plan::file_stem`, and
   selecting and deleting corner components in Edit View (needs a hint selection kind).
4. **The measures**: M6's (`docs/plans/M6.md` § 5: mark attachment against the oracles,
   propagation against glyphsLib, composition and alignment over the corpus) and the corpus
   measures of the M9 to M12 packages.
5. **Shell hooks for finished M13 packages**: dialogs and menu entries several commands wait for
   (for example the metrics-file import chooser, Status Definitions, the name-list rename).
6. **Checks in the reference app**: `docs/observe.md` lists every row the running app could
   settle, with the synthetic test fonts `cargo xtask testfonts` writes.

Left from M3 to M5 for later milestones: shaping and kerning with feature code (M8), the toolbar's master icons and the
Export dialog's Variable tab (M9). Left from M2: instances between masters (M9), CFF
subroutines (before M12), the compile plan's `features` section, and the CLI's `--name-by`, `--output-layout` and pattern instance
selection (`docs/plans/M2.md` › M2.5).

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (960 commands, 33 tools, 172 panels, 46 settings, 241
  parameters, 1,754 behaviour rules, 300 checks, 70 CLI rows, 49 formats, 50 tables) and 23
  behaviour notes (about 150,000 words), all in our own words. Every M3 command's `params` name
  its targets; shortcuts carry a `when` (the context they apply in). Every choice of ours where
  no source says what the reference does is a row's `guesses` entry, listed in `docs/guesses.md`.
- **Workspace:** 39 library crates in enforced layers, two apps, `xtask`; about 327,000 lines of Rust outside the test folders and `xtask`.
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
  The same source and timestamp give the same bytes (`crates/engine/tests/engine/determinism.rs`).
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
- **Gates:** `cargo xtask ci` with eleven steps; parity and guesses reports generated from the inventory.
  Outside the gate: `cargo xtask roundtrip` over 527 corpus sources, `cargo xtask overlap`
  (overlap removal on every corpus layer) and `cargo xtask audit`.
- **Components and anchors, engine (M6 wave A):** `crates/components`: an index of which glyphs
  use which, anchor propagation through nested and smart components (equal to glyphsLib's on
  42,734 of 42,736 corpus layers before the review fixes), automatic alignment with one re-align
  step after every edit (one undo step with the edit), composition recipes from Unicode and our
  own tables; smart and corner components drawn everywhere and exported; the mark, mkmk, abvm
  and blwm feature writers and `GDEF`. Every count read from a file is capped, including the
  work of propagation per layer and per master.
- **Plans M7 to M13 and their skeletons:** each milestone has a plan (`docs/plans/M7.md` to
  `M13.md`) whose open questions are answered in the inventory, and an interface skeleton (new
  crates and command modules with fixed signatures, no behaviour yet) so their packages can be
  written in parallel.
- **Checks for the reference app:** `cargo xtask observe` writes `docs/observe.md`, every row
  the running app could settle (1,107 rows); `cargo xtask testfonts` writes 45 synthetic `.glyphs`
  test fonts for those checks.
- **First waves of M6, M7 and M8 (written in parallel, then integrated):** 111 more commands
  live. M6: component, anchor, smart glyph, corner component and decompose commands; metrics
  of automatically aligned composites. M7: the custom-parameter catalogue (209 parameters
  generated from the inventory), Font Info fields at every scope, masters, instances, metric and
  stem definitions, Scale to UPM, export filters, glyph sets and export folders, linked sources,
  text tokens, the legacy `kern` table. M8: the fea-rs wrapper (includes resolved and capped,
  guards against nine fea-rs panics, a test per feature-language row), feature assembly with a
  source map, the syntax extensions, tokens, 60 automatic feature generators, feature entry
  commands and feature files; feature code snippets (a built-in list and the user's plist and TOML
  files under limits, with list, insert and show-folder commands).
- **First waves of M9, M10 and M11 (written in parallel, then integrated):** M9: designspace
  axes, mappings and normalisation as the oracles derive them, the first-master origin and
  virtual masters; glyph sources (master, intermediate and alternate layers) and the
  compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, name ID allocation and `STAT`
  (fontc's derivation from instances ported), byte-equal to fontc's on test sources; feature
  variations and bracket glyphs from alternate layers; variable TrueType and CFF2 fonts opened
  into axes, masters, intermediate layers, instances and STAT labels; `compareFonts.diff`,
  `useLeft`, `useRight` and `compareFamily.report`. M10: `crates/autohint`, a port of
  otfautohint (glyph data, segments and flex, stem values, masks and hint substitution, several
  masters, override sets and stem and zone reports) checked against AFDKO 5.0.1 oracle scripts;
  typed PostScript hints and TrueType instructions read from and written back to every stored
  form, with hint settings and their scopes; `crates/ttasm`, the TrueType assembler with static
  analysis, round-tripping the corpus; `gasp` at export; hinted rendering through skrifa's
  interpreter and PostScript hinter. M11: colour palettes in their three stored forms and six
  palette commands; colour layer commands; path attributes of full-colour layers (stroke, fill,
  gradients, shadows, pattern image); image commands with PNG, JPEG and TIFF decoding under
  limits and image paths rebased on Save As and undo; the `SVG ` table from vector layers at
  export.
- **First wave of M12 (written in parallel, then integrated):** `crates/tables`: table extract,
  add, delete, edit, checksum, reorder and `tables show`; collections (list, pack in otf2otc's
  layout, unpack, WOFF2 collections) and WOFF compress and decompress; TTX, JSON and hex dumps,
  the 17 core tables as ttx writes them; the layout normaliser in fontc's normaliser's format;
  feature list and decompile of `GSUB`, `GPOS`, `GDEF` and `BASE` to feature code; glyph rename
  (GOADB, maps, production names) and reorder through every glyph-indexed table, in compiled
  fonts and sources; the metrics report (text, CSV, AFM, JSON); dehint of TrueType, CFF and
  CFF2; outline repairs on whole fonts; variations map and build-avar. `crates/decompile` reads
  Type 1 (PFA, PFB) and bare CFF fonts, `crates/io-svg` reads and writes SVG fonts; `file.open`
  reads SVG fonts. `crates/proof`: palette, outline and waterfall sheets as SVG and an HTML
  specimen with the font embedded. `source.diff`, `source.merge` and `font.compareGlyphs`. CLI
  subcommands for each, plus `bench` and `completion`. The audit gains its data sets (OpenType
  tags, Unicode and code-page ranges, expected contours, vendor IDs, character sets) and 157
  checks: components and anchors, font-wide metrics, names, master compatibility (fontTools'
  interpolatable heuristics ported), variable-font tables, hinting, kerning and colour, files
  and sources, and family checks after comparefamily and FontBakery; audit profiles, settings,
  the ignore list and the cache command; result export as JSON, text, Markdown and HTML with
  menu-name, metrics, PANOSE and feature-language reports. Every command that takes an
  `outputDir` creates it when missing. The adversarial reviews added named limits for name
  records read per font, interpolation-check work per glyph, layout tables before they are
  expanded, font-wide charstring and dehint work, TTX output, summed table sizes in collections,
  unique-name suffixes, component depth, avar points per axis, layers read from variable fonts,
  autohinter element walks and segment counts, and alignment zones per master.
- **Second-wave packages (M6.14, M7.14 to M7.18, M8.15), integrated:** Add Glyphs recipes and
  composition, the Font View batch actions on components and anchors, five component
  smart-filter rules; projects (`.fontcraftproject`, 14 commands); the Font Info window with
  its Font, Masters, Exports, Other Settings and Notes tabs and the shared row editor; the
  feature compile state with errors, four fixes, completion and token evaluation. Handbook
  behaviour, not observed.
- **First wave of M13 (22 packages), integrated:**
  - Editing: curve tension and harmonizing; ink traps and corner helpers; geometric helpers;
    stem detection and the four stem filters; the Thickness tool and variable-width strokes;
    Revert Glyph, Copy Layer to Layer, Paste After, Compare Layers; the outline matcher with
    Detect Composites and Detect Component.
  - Glyph data: anchor and automatic-layer expressions; Show Dependencies, Copy Component
    Transformation and the Related Glyphs section; stored selections and the reference font;
    status definitions.
  - Files: export profiles and glyph artwork; the external-change review; Filter by Encoding
    File, Rename Glyphs by Name List and Compare Fonts in List; PFM and AFM metrics import;
    UFO 1 (`io-ufo`); EPS outlines (`io-svg`); SFD import (`crates/io-sfd`, not yet reachable
    from File › Open).
  - Automation: the Command Console, the `.fcmacro` format and the Script commands, the
    `fontcraft://` URL scheme, MCP permissions, prompts, resources, completion and logging.
  Secondary-reference features follow public documentation; each choice of ours is a guess on
  its row.
- **Automation file confinement:** `--automation-read-root` and `--automation-write-root` on
  `fontcraft-cli mcp` and on the desktop app's control channel (also
  `FONTCRAFT_AUTOMATION_READ_ROOT` and `FONTCRAFT_AUTOMATION_WRITE_ROOT`). Read and write roots
  are separate; every declared file parameter, implicit place and linked file is checked while a
  root is in force; a linked file outside the read root reads as missing; no root, no
  restriction. `--read-only`, `--allow` and `--deny` are unchanged (`docs/mcp.md`).
- **Hardening passes:** bounded work on hostile input in the EPS, SFD, UFO 1 and PFM readers,
  features previews, macros, outline matching, intersections and stem filters; all-or-nothing
  exports across profiles; bounded name-table reads; `avar` segment maps for axis maps outside
  the axis range match fontmake, documented in `docs/spec/40-compile.md`.
- **Builder packages M8.9, M8.7 (with M8.7b), M7.8 and M6.10:** automatic feature code with
  Update and regeneration at export; `GSUB`, `GPOS` and `GDEF` compiled from feature code and
  compared with fontmake and fontc lookup by lookup; the name table and OS/2 of static instances
  as fontmake builds them; Edit View's component and anchor display and selection. Numbers and
  what is left are under "What is real: the builder packages".
- **Build layout:** the engine's integration tests are one binary, as are the decompile,
  io-glyphs, io-ufo and plist tests; the `measure` profile serves speed measures and oracle runs
  (`docs/development.md`).
- **Fixes after the gate:** the web build compiles; every export path and the code editor read
  feature includes from the source's include folder; `.glyphs` 3 files keep deleted kerning
  deleted and keep a version 2 kerning key the reader did not use; a refused include candidate does
  not hide an allowed one.

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
| M6 | Components and anchors | 🟨 first wave built: the component index, propagation, automatic alignment, composition, smart and corner components drawn and exported, the mark writers; the component, anchor, smart glyph, corner component and decompose commands; metrics of aligned composites. Second wave: Add Glyphs recipes and composition, and the Font View batch actions on components and anchors (M6.14). Edit View display and selection of components and anchors (M6.10). Left: selecting and deleting corner components, clicking a missing-base placeholder, the other UI packages and the measures (`docs/plans/M6.md`) | 46 (40) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | 🟨 first wave built: the parameter catalogue, Font Info fields at every scope, masters, instances, metric and stem definitions, scaling to a new UPM, export steps (filters, glyph sets, export folders), linked sources, text tokens, the legacy kern table. Second wave: projects (M7.14) and the Font Info window with its Font, Masters, Exports, Other Settings and Notes tabs (M7.15 to M7.18). The name table and OS/2 of static instances as fontmake builds them (M7.8). Left: text-token expansion in PostScript and file names of instances, the Instances tab, the Features tab until M8.17, the remaining plan packages (`docs/plans/M7.md`) | 66 (66) |
| M8 | OpenType features | 🟨 first wave built: the fea-rs wrapper with its guards, assembly with a source map, the syntax extensions, number, property and predicate tokens, 60 automatic feature generators, the feature entry commands, feature file import and export, feature code snippets. Second wave: the compile state (M8.15), with errors, fixes, completion and token evaluation. Builder packages: the automatic feature framework with Update and regeneration at export (M8.9), and layout tables at export compared lookup by lookup with fontmake and fontc (M8.7, M8.7b). Left: the ligature, fraction and aalt generators (M8.11), the feature editor window (M8.17), the preview font (M8.19), layout tables of variable and colour fonts (M9, M11), shaping (`docs/plans/M8.md`) | 41 (36) |
| M9 | Interpolation and variable fonts | 🟨 first wave built: axis ranges, mappings and normalisation as the oracles derive them; glyph sources and the compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, the variable-font names and `STAT`, byte-equal to fontc's on test sources; feature variations from alternate layers; variable TrueType and CFF2 fonts open; the compare commands. Nothing variable reaches an export. Left: the builder packages M9.4 (static instances), M9.5 (the variable compile), M9.9, M9.10, M9.12, M9.13; the axis, special-layer, instance and master commands and the UI (`docs/plans/M9.md`) | 48 (4) |
| M10 | Hinting (PostScript and TrueType) | 🟨 first wave built: the PostScript autohinter ported from otfautohint, agreeing with AFDKO 5.0.1 on its oracle scripts' cases, with reports and override sets; typed hint data and settings; the TrueType assembler; `gasp` at export; hinted previews. Exports carry no hints. Left: the builder packages M10.3, M10.14 (CFF hints at export), M10.15, M10.19 (TrueType export), M10.21; the hint commands, font and glyph programs, import of hinted fonts and the UI (`docs/plans/M10.md`) | 56 (5) |
| M11 | Colour fonts and images | 🟨 first wave built: colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export. Left: the builder packages M11.5 (paint attributes in the formats), M11.6 and M11.7 (`CPAL` and `COLR`), M11.13, M11.14, M11.17, M11.19; `sbix`, `CBDT`, the `SVG ` table from svg layers and bitmaps, and the panels (`docs/plans/M11.md`) | 41 (37) |
| M12 | Command-line tool suite and quality checks | 🟨 first wave built: table actions and `tables show`, collections, WOFF, TTX dumps of the core tables, the layout normaliser, feature list and decompile, glyph rename and reorder, the metrics report, dehint, outline repairs on whole fonts, Type 1, bare CFF and SVG fonts, variations map and build-avar, SVG and HTML proofs, source diff and merge, `bench` and `completion`; 157 checks, audit profiles, the ignore list and report export. The corpus measures have not been run. Left: the builder packages M12.44 (the audit runner, which applies profiles, the ignore list and thresholds), M12.17 to M12.21 (subset, instance), M12.23 to M12.26 (merge, transform, subroutiniser, optimize), M12.38, M12.40 to M12.42, M12.61, M12.64; the TTX writer past the core tables and the TTX reader, convert, diff, the PDF proof and the other check groups (`docs/plans/M12.md`) | 81 (46) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | 🟨 first wave built (22 packages): curve tension, corner and geometry helpers, stem detection and filters, the Thickness tool, expressions, Detect Composites, stored selections and the reference font, status definitions, export profiles and artwork, external-change review, encoding and name-list tools, PFM and AFM import, UFO 1, EPS and SFD reading, the Command Console, macros, the URL scheme, MCP permissions, prompts and resources. Secondary-reference documentation only, nothing observed. Left: the later waves (the module runtime and its commands, menus and Actions dialog, the Extension Manager, MCP over HTTP, the author documentation, the remaining editing and glyph-data packages), the shell hooks several finished packages wait for (toolbar buttons, dialogs, menus), and SFD wired into File › Open; Detect Composites rebuilds 45.4% of Inter's composites from the same bases against a 95% target (`docs/plans/M13.md`) | 180 (63) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 34 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 4 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 960 (660) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 45 | 27 | 22% | tabs, zoom and scroll work; user settings kept between runs; the Start Window with recent documents; the toolbar with shared buttons and their menus; tools chosen by their letters, shared letters cycle; context menus; menu items greyed out by the selection; bottom bar zoom controls; the Command Console; the external-change review on window focus. Missing: settings window, shortcut editor, workspaces, title bar, sidebars, the toolbar's master icons (M9); nothing compared with the reference |
| Font View | 6 | 68 | 62 | 40% | grid, List View, sidebar with categories, languages and filters, search, the glyph database, Add Glyphs, renaming with references, the inspector, Selection Info, Glyph Info, the Start Window, the glyph set dialogs, the inspector's metric and kerning-group fields and the out-of-sync badge; Add Glyphs recipes and composition, batch component and anchor actions, five component smart-filter rules; Filter by Encoding File and Compare Fonts in List. Missing: images in cells (M11), the rename dialog for name lists; nothing compared with the reference, and many choices are guesses |
| Edit View text mode | 4 | 25 | 18 | 40% | typing, the Text tool, cursor keys, selection, clipboard as text, three directions, three kerning modes, kerned-gap colours, metrics labels, tabs from display strings, the Preview Area and Panel, the measurement line, Text Preview, sample strings, Find Glyphs. Missing: shaping (M8), Latin letters inside right-to-left runs in Text Preview; nothing compared with the reference |
| Drawing and editing outlines | 10 | 98 | 87 | 27% | node, segment, transform, clipboard and selection commands; in the window Select, Lasso, Hand, Zoom, Draw, Pencil, Rectangle, Ellipse, Rotate and Scale, the bounding box and its knobs, context menus, the Info box, the Palette's Transformations and Fit Curve sections, the Transformations and Primitive Measurements dialogs, editing in any master; curve tension and harmonizing, corner and geometry helpers, the Thickness tool (by command), stored selections, the reference font. Missing: toolbar button and Tunni-line display for the new commands, `path.sharpenCorner` (no source, in `fidelity`); nothing compared with the reference |
| Path operations and filters | 6 | 68 | 60 | 31% | Remove Overlap, the booleans, path direction, extremes and inflections, Tidy up, Open Corner, Reconnect, Cut with Line, curve conversion and rounding as commands; the Knife; the six filters with dialogs and export strings; strokes and masks drawn on the canvas and applied at export; the Stroke section and the Shape Order dialog; stem detection and the four stem filters with dialogs; ink traps, Make Parallel, Space Points, Make Arc, Slice Along Guide. Missing: stroke attributes in UFO, hints renumbered after Shape Order and Expand Outline; nothing compared with the reference |
| Reusing shapes | 7 | 69 | 45 | 16% | the component index, anchor propagation, automatic alignment after every edit, composition data, smart and corner components drawn and exported; component, anchor, smart glyph, corner component and decompose commands; derived metrics of aligned composites; Add Glyphs recipes; Detect Composites and Detect Component (45.4% of Inter's composites rebuilt from the same bases against a 95% target), anchor expressions and automatic layers, Show Dependencies, Related Glyphs. Edit View draws components by alignment state, placeholders, anchors and corner tints, and its clicks, rubber band, Select All, Tab, double-click, Option-drag, Delete, smart handles and size knobs run their commands. Missing: selecting and deleting corner components, clicking a missing-base placeholder, the other tools, panels and Palette sections (M6 UI packages); nothing compared with the reference |
| Layers | 3 | 21 | 21 | 35% | background and layer commands; Edit Background; the active layer and the tab's master; the background and other visible layers drawn on the canvas; the Palette's Layers section with its row context menu; the Interpolate with Background dialog. Font View's master choice. Missing: nothing compared with the reference |
| Guides, metric lines, zones, grid | 3 | 36 | 21 | 26% | guide, ruler, coordinate and Dimensions commands; metric lines, guides, the grid, CJK lines and measurement guides drawn; guides dragged and edited on the canvas, with their context menu and the Guide Properties dialog; the Measurement tool; snapping to guides and their crossings; the Palette's Dimensions section. Missing: script-specific metric lines; nothing compared with the reference |
| Spacing and kerning | 7 | 80 | 66 | 41% | metrics keys, sidebearing and width setters, the spacing and kerning shortcuts in text mode, Update Metrics, Transform Metrics, kerning pairs, groups, exceptions and locks, Clean Up, Compress, group rename; the Info box's spacing and kerning fields, the Kerning window, Show Group Members, the Font View inspector fields and out-of-sync badge, metrics import and export (kerning-text, AFM, PFM), kerning read from compiled fonts; generated pair kerning in exported fonts, the lookup equal to fontc's and fontmake's compiled values over the corpus. Missing: vertical kerning at export, automatically aligned glyphs (M6), vertical origin and width commands, the out-of-sync update at its speed target; nothing compared with the reference; many choices are guesses |
| Font Info | 6 | 52 | 49 | 30% | fields at font, master and instance scope, added and localized fields, row copy and paste; the custom-parameter catalogue with value checks; masters, instances, metric and stem definitions; scaling to a new UPM; filters, glyph sets and export folders at export; linked sources; text tokens; projects; the Font Info window (Cmd+I) with its Font, Masters, Exports, Other Settings and Notes tabs, shared row editing, and field and parameter pickers; status definitions; the name table and OS/2 of static instances from these fields, as fontmake builds them, with the version stamp. Missing: the Instances tab, the Features tab until M8.17, text-token expansion in instance PostScript and file names; nothing compared with the reference |
| Interpolation | 8 | 68 | 10 | 6% | Edit View draws and edits any master (`view.selectMaster`); axis ranges, mappings and normalisation; glyph sources and the compatibility check; the variable tables and `STAT` built (byte-equal to fontc's on test sources) and variable fonts opened; feature variations from alternate layers; the compare commands. Missing: variable fonts and instances between masters at export (M9.4, M9.5), the axis, special-layer, instance and master commands, the master icons, the Variable export tab and every interpolation window |
| OpenType layout | 7 | 47 | 39 | 17% | feature code assembled with a source map, the reference's syntax extensions, number, property and predicate tokens, 60 automatic feature generators, feature entry commands, feature file import and export, snippets, the fea-rs wrapper with guards; feature compile state with errors, four fixes, completion and token evaluation; layout features listed and decompiled from compiled fonts; the `mark`, `mkmk`, `abvm`, `blwm` writers and `GDEF`; the automatic feature framework with Update and regeneration at export; feature code compiled into `GSUB`, `GPOS` and `GDEF` in exports, equal lookup by lookup to fontmake's in most corpus sources. Missing: the ligature, fraction and aalt generators (M8.11), the layout of variable and colour fonts, shaping |
| PostScript hinting | 3 | 26 | 10 | 8% | typed hints and hint settings in every stored form; the autohinter ported from otfautohint with its reports and override sets, agreeing with AFDKO's oracle scripts; hinted previews; dehint of compiled fonts. Missing: hints at export (M10.14, M10.15), the hint commands and Edit View display; nothing compared with the reference |
| TrueType hinting | 4 | 40 | 1 | 3% | typed instructions in every stored form, the assembler, `gasp` at export, hinted previews. Missing: TrueType export (M10.19), the autohinter (M10.21), the hint commands, font and glyph programs, the Instructor tool and its panels |
| Colour fonts and images | 3 | 47 | 37 | 10% | colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export. Missing: `COLR`, `CPAL`, `sbix`, `CBDT`, paint attributes in the source formats (M11.5), colour drawing in Edit View and Font View, the colour panels; nothing compared with the reference |
| Source formats | 6 | 15 | 13 | 62% | Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; shape groups, gradients and shadows kept as stored values; UFO 1 sources open; EPS outlines and SFD sources read (SFD not yet in File › Open); right-to-left, vertical and contextual kerning in UFO; version conversion |
| Compiling | 9 | 13 | 11 | 22% | static TTF and OTF, plain, WOFF and WOFF2, from the commands and the Export dialog; production names and database mark widths; generated pair kerning in `GPOS`; linked metrics and Default Layer Width; strokes expanded, masks applied and overlaps removed (coverage-equal to fontmake's in over 99.5% of glyphs); core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source; `gasp` and the `SVG ` table; export profiles written all-or-nothing (Export Fonts As, glyph artwork); `avar` segment maps matching fontmake's; feature code compiled into the layout tables; the name table and OS/2 of static instances as fontmake builds them; the variable tables built but not exported. Missing: variable fonts and instances between masters at export (M9.4, M9.5), hinting (M10.14, M10.19), `COLR` and `CPAL`, CFF subroutines, most of the other tables |
| Preview, proofing, printing | 2 | 24 | 15 | 13% | the Preview Area and Preview Panel (invert, flip, blur, offset), Text Preview with its text, style, size, direction and find, sample strings; proof sheets as SVG and an HTML specimen. Missing: printing, PDF proofs, shaping (M8); nothing compared with the reference |
| Command-line tools | 8 | 55 | 31 | 15% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `oracle`; `tables` (extract, add, delete, edit, checksum, reorder, show), `collection`, `woff`, `dump` (the 17 core tables), `features` (normalize, list, decompile), `glyphs` (rename, reorder), `metrics`, `dehint`, `outlines`, `variations` (map, build-avar), `proof`, `sources`, `bench`, `completion`; tested against small oracle outputs, the corpus measures not yet run. Missing: subset, instance, merge, transform, optimize, convert between flavours, diff, `check`, TTX past the core tables and TTX reading |
| Quality checks | 3 | 33 | 23 | 18% | 34 of the 36 outline checks through `audit.run`; `audit.fix` applies eight fixes through path commands and each clears its check; live audit and canvas markers; 157 of M12's checks (components and anchors, font-wide metrics, names, master compatibility, variable-font tables, hinting, kerning and colour, files and sources, family) with their data sets; profiles, the ignore list, report export. Missing: the audit runner that applies profiles, the ignore list and thresholds (M12.44), the glyph-set, width, table and layout checks, fixes on compiled fonts, the Font Audit panel; verdicts not yet compared with FontBakery's and comparefamily's over the corpus |
| Plug-ins and scripting | 3 | 20 | 6 | 5% | the Command Console, the `.fcmacro` format with the Script commands, and the URL scheme. Missing: the module runtime, module commands, menus and forms, the Actions dialog, the Extension Manager, author documentation |
| Agent surfaces | 4 | 10 | 8 | 50% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; tools, pointer, key, text, menu and dialog control over the control channel, enough to draw and export a font, to build one from glyph sets, and to type, space and kern text (three end-to-end tests); shortcuts sent with `ui.key` open their dialogs as the keyboard does; MCP permissions, workflow prompts, resources, completion and logging; read and write folder roots for MCP and the control channel. Missing: `ui.click` beyond dialogs, `ui.set`, the MCP HTTP transport |
| **Feature parity** | **117** | **960** | **660** | **~25%** | |

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
