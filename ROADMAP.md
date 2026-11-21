# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-11-20 (this batch: 47 writer packages from depth waves 2 to 11 with three follow-ups, the first three steps of the engine split, and a second window on the Craft suite's craft-next framework (`crates/ui`, `apps/fontcraft-next`) that sits beside the egui window until it reaches parity; merged on `master`, with test fixes under way and the gate not yet run)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,786 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, pointer and key injection, toolbar, context menus, dialogs, panels, tests, atomic file writes through the Craft suite's shared library | **~66%** of what the siblings' "infrastructure" row covers; missing: five of the settings window's seven sections, a performance budget suite. A second window on the suite's craft-next framework (`crates/ui`, `apps/fontcraft-next`) ports the panels, canvas, dialogs and control channel; it sits beside the egui window (`crates/ui-egui`, `apps/fontcraft`) until it reaches parity, and has not been tried in the running app |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~38%** |
| Commands that exist (not: that behave like the reference) | **960 of 1039** (`docs/parity.md`) |
| Packages built, M6 to M13 | **294 of 360** (M6 21 of 26, M7 21 of 24, M8 27 of 36, M9 30 of 40, M10 35 of 42, M11 20 of 22, M12 77 of 87, M13 63 of 83; the totals include the depth packages of `docs/plans/depth.md` that carry a new number; counted from commit subjects, M8.7b with M8.7; M9.8b is a follow-up of M9.8 and not counted; nor are the follow-ups M8.20b, M8.20c, M8.23b, M10.19b, M10.21b, M11.5b, M11.5c, M11.7b, M11.13b, M11.14b, M11.17b to M11.17e, M12.7b to M12.7f, M12.10b, M12.13b, M12.18b, M12.18c, M12.19b to M12.19d, M12.20b, M12.21b, M12.22b to M12.22e, M12.23b, M12.23c, M12.24b, M12.24c, M12.25b to M12.25e, M12.30b to M12.30d, M12.40b to M12.40d, M12.41b, M12.64b, M13.15b, M13.15c, M13.23b, M13.55b to M13.55f, M10.37b, M10.37c and M13.65b) |
| Depth packages (`docs/plans/depth.md`) | **55 of 167** built: wave 1 (M1.8, M3.19, M3.21, M3.40, M4.17, M7.21, M9.27, M13.78, M14.1, M14.3) and 45 writer packages of waves 2 to 11 (wave 2: M12.67, M13.80, M9.29, M12.36; wave 3: M6.21, M12.66, M12.69; wave 4: M2.8, M9.30, M5.20; wave 5: M2.12, M2.13, M8.28, M8.29, M7.23, M5.22, M12.68; wave 6: M3.29; wave 7: M9.34, M9.36, M10.36, M10.40, M12.70, M12.73; wave 8: all nine; wave 9: M13.66, M13.68, M12.77, M12.79, M13.21b; wave 10: M1.9, M3.36, M8.35, M12.80; wave 11: M1.11, M8.32, M12.75). No builder of waves 2 to 13 is built. M3.22, M3.23, M4.18, M4.19, M5.29, M7.22 and M9.32 have editor parts in the new window only and are not counted; M6.25 (written down, no code) and X.pdf3 (built on no platform yet) are not counted either |
| Observed against the running reference | **62 rows**: 57 compiled-output rows checked against fontc, fontmake and makeotf, and 5 editor rows confirmed by public tutorial videos of the previous major version. The reference app itself has not been run |

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
OS/2 table as fontmake does, and draws and selects components and anchors in Edit View. A third
batch exports variable TrueType fonts, compared with fontc's and fontmake's at masters, virtual
masters, instances and midpoints, and interpolates static instances between and beyond masters.
A fourth batch runs the quality checks over sources, compiled fonts and families, subsets fonts
by the reference tool's options and runs plug-in modules under resource limits.
A fifth batch lowers the reference's variable feature syntax (conditions, variation blocks and
variable positions) as glyphsLib does and compiles it into variable exports as feature variations
and variable positions, and into static exports at the instance's location.
A sixth batch exports variable fonts with CFF2 outlines, builds the TrueType font programs (`cvt `,
`fpgm` and `prep`) and adds 14 commands that edit TrueType instructions, each checking its
selection against the touched state of the points.
A seventh batch (75 packages) adds the TrueType autohinter, PostScript autohinting at export and the TrueType
Instructor with its Info box, pixels and Hinting Preview; `COLR` (versions 0 and 1), `CPAL`, `sbix` and
`SVG ` colour tables; the axis, special-layer, instance and master commands with their windows; TTX writers
for the layout, variation, colour and CFF tables; feature code compile, lint and convert; about 80 more quality
checks and the audit fixes; plug-in modules in the editor, in export and in the audit; display aids, drag aids,
power guides and four more tools in Edit View; auto-spacing and auto-kerning; MCP over HTTP. Four of its measures
missed their targets (see "What is real: this batch"; the tuning pass after it met one and narrowed another).
An eighth batch (17 packages) shapes text in Edit View with the font's feature code; writes feature-code
table blocks and `usMaxContext`; attaches marks by context; builds `CBDT` and `CBLC` and variable `COLR`; places
images in Edit View; reads TTX back into fonts; and adds static instances, merge, transform and the CFF
subroutiniser to the command-line suite; adds TrueType assembly and a debugger; and adds pair lists, the Kerning Pairs
Preview, the Groups Editor and the Space Center. A tuning pass lifted auto-kerning above its target.
A ninth batch (14 packages and three small tasks) shapes text and orders mixed-direction runs in Text Preview; draws
colour glyphs in Edit View and the previews; adds the Space Center's display options; builds instances from
designspace and multi-master sources, takes instance names, classes and font info from designspace `<instance>` elements
and compiles designspace `<rules>`; makes partial instances of variable TrueType fonts; subroutinises CFF exports and
adds `optimize`; adds the `variations` tools; reads the layout, variation and colour tables from TTX and writes TTX
for the Apple, bitmap and `COLR` version 1 tables; fixes compiled fonts through `audit.fix`; and adds the Font Audit
panel. A tuning pass raised the TrueType autohinter's x-height rows and Detect Composites.
A tenth batch (17 packages and six small tasks) rebuilds a compiled font's feature code when it opens; subsets
layout, variation, colour and CFF2 tables and prunes CFF subroutines; converts fonts between TrueType, CFF and CFF2
outlines; compiles an AFDKO project and CID-keyed CFF, with the compile extras of fontmake and fontc; makes CFF2
instances and merges CFF fonts; reads the colour, bitmap and Apple tables and split dumps from TTX; adds the Table
Viewer; draws images and colour in Font View cells and in Text Preview; keeps full-colour layers in UFO; and runs
audits in steps with progress, cancel and a report export. Integration found and fixed one bug that dropped GDEF
classes from subsets.
An eleventh batch (1 new and 20 follow-up packages, twelve small tasks) keeps higher-order interpolation data on
nodes; subsets `VARC` and compressed `SVG ` documents and takes pyftsubset's list options; reads and writes `mort` and
`morx` in TTX; converts from Type 1, bare CFF and TTX; transforms CFF, CFF2 and variable fonts; makes partial
instances of `avar` version 2, `VARC` and variable `COLR` fonts, and layout-only instances; fills in `optimize`; lets
the CFF subroutiniser and convert handle full CJK fonts; compiles makeotf's remaining project keys; writes
`PaintColrGlyph` into UFO and compiled `COLR`; breaks Text Preview lines on shaped widths and sets vertical text;
edits images in Edit View; holds bitmap strikes in the model; and finishes `audit.fix`. Its review found and fixed an
undercount in the feature-code lookup guard that could let fea-rs abort, and an out-of-memory path in the `mort`
reader.
A twelfth batch (12 follow-up packages, two small tasks) merges variable fonts as variable fonts; specialises,
subroutinises and hints CFF2 exports; exports the ROS parameter as CID-keyed CFF; writes `EBDT` and `EBLC` and opens
bitmap-only fonts; draws `PaintColrGlyph` references and `SVG ` clip paths, masks and patterns; and sets vertical
text in Text Preview.
A thirteenth batch (10 follow-up packages, seven small tasks) writes `cvar`; hints the intermediate and alternate
layers of CFF2 exports; interpolates instance layout from compiled masters; converts to UFO with glyph subsets,
GOADB files, CID keying, a location and overlap removal; adds spot listings to `dump`; recompiles CFF in `audit.fix`;
draws `SVG ` filters; pastes and imports PDF outlines; reads and writes the Windows, TeX, Mac and gzipped bitmap
font formats; and raises Detect Composites on Inter from 63.0% to 78.1%. Its review found and fixed an audit cache
that ignored a profile's thresholds and a CID-keyed convert that kept `seac` glyphs.
A fourteenth batch (2 follow-up packages, five small tasks) places components and groups lookups in project compiles
as makeotf does; draws the remaining `SVG ` filter primitives and filters in Edit View; cleans up after a
`checkOutlines` fix as checkoutlinesufo does; recalculates the bounds of CFF fonts read from TTX as ttx does;
compiles designspace documents without axes; pastes the system clipboard's PDF in Edit View; and keeps the layouts of
`.glyphs` node attributes. Its review found and fixed a font saved as a designspace that would not compile, and
filtered glyphs re-rasterised on every frame once off screen.
A fifteenth batch, wave 1 of the depth plans (ten packages), asks about unsaved documents before the window closes or
the app quits; opens files through a chooser, Open Folder and drop; adds the settings window with its User Settings and
Extensions sections; edits axes and coordinates in Font Info; checks the values of 64 custom parameters; renames and
drags filters in the sidebar and rows in the Layers section; gives the Select tool its Alt and Ctrl+Alt drags; measures
across every glyph of the line; and takes pointer, wheel, view-switch, resize and context-menu input over the control
channel. Its review found and fixed a measurement that failed on long lines and recomputed every frame, keys that
reached the canvas behind the save question, and synthetic input applied before the frame that draws it.
A sixteenth batch builds 47 writer packages of depth waves 2 to 11 and starts a second window. In the engine and
the CLI it adds the reference compilers' compile options and MakeOTF Compatibility Mode; common options for every
subcommand; PDF proof sheets with the proof tool's layouts and spot sheets; readable listings, subsets by the
converter's glyph selection and of sources, mergefonts parity, the converter's extras and the dump leftovers;
autohinting of compiled CFF and CFF2 fonts; higher-order interpolation in instances, Match Masters and the
designspace helpers; Indic positional features and lookup debug info; `CPAL` version 1 and a dark palette; colour
artwork with shadows and patterns; bitmap glyph commands; External Preview in a browser; MCP window tools; and nice
names on import with UFOZ save and format warnings. The engine split moves errors, the guard, settings and the
command registry into `engine-core` and gives extensions typed session slots (ES.1 to ES.3). The second window
(`crates/ui`, `apps/fontcraft-next`) is built on the Craft suite's craft-next framework and ports the egui window's
panels, canvas, tools, dialogs and control channel; it sits beside the egui window until it reaches parity, and
none of it has been tried in the running app. The batch is merged; its gate has not run.
Much of the rest runs by command and CLI only, and much of it does not yet reach exported fonts.
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

**What is real: editing by command (M3).** Of the 1009 commands, all 185 of M3, all 62 of M4 and all 97 of M5 exist.
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
| M9 | axis ranges, mappings and normalisation as the oracles derive them; glyph sources and the compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, the variable-font names and `STAT`, byte-equal to fontc's on test sources; feature variations from alternate layers; variable TrueType and CFF2 fonts open into axes, masters and instances; the Compare Fonts and Compare Family commands. The builder packages below put them into exports |
| M10 | the PostScript autohinter, ported from otfautohint and agreeing with AFDKO 5.0.1 on its oracle scripts' cases, with its reports and override sets; typed hint data and settings in every stored form; the TrueType assembler; `gasp` at export; hinted previews through skrifa's interpreter. Hint ends and instruction points follow their nodes through edits (M10.3). CFF exports carry stem hints, masks, flex and the source's manual PostScript hints (M10.14). TrueType exports carry the glyph programs compiled from manual instructions (M10.18, M10.19); the TrueType autohinter (M10.21) is not built |
| M11 | colour palettes in their three stored forms, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits and paths rebased on save; the `SVG ` table from vector layers at export; paint attributes (colours, gradients, shadows) typed in `.glyphs` versions 2 to 4 (M11.5, part 1). No `COLR`, `CPAL`, `sbix` or `CBDT` |
| M12 | table actions and `tables show`, collections, WOFF compress and decompress, TTX, JSON and hex dumps of the 17 core tables, the layout normaliser, feature list and decompile, glyph rename and reorder, the metrics report, dehint, outline repairs on whole fonts, Type 1, bare CFF and SVG font reading (SVG fonts written too), variations map and build-avar, SVG and HTML proofs, source diff and merge, `bench` and `completion`; audit data sets, 157 checks, audit profiles, the ignore list and report export; the audit runner over every scope (M12.44) and font subsetting (M12.17), below. Tested against small oracle outputs; the corpus measures of `docs/plans/M12.md` § 5 have been run for the audit runner and subset only |

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
| M8.15 | the feature compile state: cached per document while the code is unchanged, errors mapped to entry, range, line and column with four fixes, jump and details, token evaluation, completion, name preview, syntax-colouring data. The preview font comes from M8.19 |
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

**What is real: variable fonts and instances (M9.10, M9.4, M9.5, M9.9, M9.8b, M9.11, M9.12).** Integrated and
gated with the rest. The compiled output is measured against fontc and fontmake; the editor
behaviour follows the handbook and is not compared with the reference app.

| Package | What works |
|---|---|
| M9.10 | the oracle harness for variable fonts: `fontcraft-cli oracle --corpus --variable` compares a variable font with fontc's or fontmake's at the masters, the virtual masters, the instances and the midpoints between masters (outlines, advances, phantom points, `MVAR`, `GSUB` feature variations, `GPOS` kerning and marks; `fvar`, `avar` and `STAT` as structures). `--instances` compares each static instance with `fontmake -i` |
| M9.4 | static instances between and beyond masters (`interp::instance_font`): alternate and intermediate layers, metrics, stems, numbers and number parameters, kerning with group fallback, manual interpolation factors, Local Interpolation and InterpolationWeightY. An instance at a master's location is interpolated when it sets manual weights, InterpolationWeightY or Local Interpolation. Extrapolated instances are named in the export report and allow-listed against fontmake, which clamps (the handbook says to extrapolate) |
| M9.5 | variable TrueType export (`file.exportVariable`, `fontcraft-cli compile --variable`): `compile_variable` builds `glyf` from the origin and `gvar` from every master and intermediate layer (cu2qu across sources, phantom points, IUP at 0.5, shared tuples, rounding as fontTools rounds), with `fvar`, `avar`, `HVAR`, `MVAR`, `STAT` and the names. Composites whose transforms differ are decomposed. Whole-font parameters and Disable Masters apply per variable font setting. Incompatible glyphs are refused with their names. A tuple variation too big for its 16-bit size field, and any `gvar`, `HVAR` or `MVAR` delta outside 16 bits, are refused with a message. Oswald takes 0.19 s (fontc 0.16 s, target 0.5 s); Inter Roman takes about 1.3 to 1.6 s (fontc 1.66 s, target 3 s) |
| M9.9 | variable kerning and mark positions: device tables and the `GDEF` variation store through fea-rs (new dependency `fontdrasil` in `crates/fea`). The origin follows the handbook (the first master unless Variable Font Origin is set); the oracles choose the master named "Regular" as glyphsLib does, so Oswald is allow-listed |
| M9.8b | alternate layers in variable fonts: `.BRACKET.varAltNN` glyphs, composites that take their components' alternate ranges, kerning copied to the alternate glyphs as glyphsLib copies it (through index maps, one lookup per pair), and feature variations merged into the feature code's `GSUB` with the `rvrn` lookups first |
| M9.11 | the reference's variable feature syntax lowered as glyphsLib does (ported in part, credited in `NOTICE`): `condition` statements become `conditionset` and `variation` blocks, with internal limits mapped to user coordinates and overlapping regions overlaid; `(tag:coord) value` positions become variable scalars, also in value records and anchors. The output equals glyphsLib's converter on 32 synthetic texts. Static exports compile with values and conditional rules resolved at their location, instances between masters included |
| M9.12 | feature code of variable exports compiled on fea-rs with a real variation info: the axes with their maps, the masters and the number values. `condition` and `#ifdef VARIABLE` become `GSUB` feature variations merged with the alternate layers' records; variable `pos` values and per-master number tokens resolve through the same model as the generated kerning, into one `GDEF` store. An inverted condition is an export warning. Equal to fontmake on variable `pos` plus alternate layers, and to fontc on condition sets plus number tokens; one hand-written source (VarFeaOneShot) builds and is equal |

Measures (equal, different and failed):

| Measure | Result |
|---|---|
| fontc and fontmake variable fonts against each other | 112 of 133 sources agree (`docs/spec/40-compile.md` § 4) |
| Variable font against fontc | 101 equal, 37 different, 20 failed; before M9.12: 100, 37, 21 |
| Variable font against fontmake | 104 equal, 38 different, 16 failed; before M9.12: 103, 38, 17 |
| Outlines, against fontc / fontmake | 127 equal and 8 different / 128 and 11 |
| `GSUB` feature variations | 129 and 8 / 135 and 6 |
| `GPOS` kerning | 135 and 1 / 141 and 0 |
| `GPOS` marks | 134 and 2 / 139 and 2 |
| Static instance against `fontmake -i` | 169 equal, 41 different, 0 not built, 24 failed (all 24 are fontmake's own failures); before M9.4: 111, 25, 83, 15 |

Before M9.5 all 158 sources were not built. The static comparison summaries and the round trips
are unchanged. The main remaining cause among the instance differences: a designspace instance
should take its names, `usWeightClass` and font info from its `<instance>` element.

The review of this batch (X.fix23) fixed: the variable-syntax scanner is linear and refuses a value
record over a size limit; conditions on axes wider than ±1 keep their ranges; a glyph named
`condition` compiles; number-token expressions in metric statements become per-master variable
values (a token elsewhere takes the origin value); two masters at one location with different
token values are an error; one-line `.glyphs` dictionaries and lists (for example
`numbers = ({name = gap;})` or `axisRules = ({min = 600;})`) reach the model and keep their layout
on write, matched by position in linear time. The round trips and the static oracle summary are
unchanged, and so are the instances (169/41/0/24).

Left: alternate layers whose bounds are all empty (the handbook disagrees with both oracles; 3
sources), designspace `<rules>` (3 sources), fontc's combinatorial bracket kerning and its tables
for decreasing axes, brace sources for composites of non-exported brace glyphs, vertical
phantom points and `VVAR`, the `OVERLAP_COMPOUND` flag, hints, subroutines and specialisation in CFF2, the variation of TrueType composites with intermediate layers, variable values in table
blocks (`fea.var.tableValues`: `BASE`, `GDEF` carets, `hhea`, `OS/2`; fea-rs rejects them), the
axis, special-layer, instance and master commands and the UI.

**What is real: hints and paint (M10.3, M10.14, X.fix20, M11.5 part 1, X.fix21).** Integrated and
gated with the rest. The compiled output is checked with fontTools and skrifa; the editor behaviour
follows the handbook and is not compared with the reference app.

| Package | What works |
|---|---|
| M10.3 | node-reference upkeep: after every font-changing command, PostScript hint ends and TrueType instruction points stay on their nodes through node inserts and deletes, contour reversal, start-point changes, Remove Overlap and Expand Outline, in the command's own undo step. A deleted node's PostScript end becomes a number; its TrueType instruction is removed and reported under `hintUpkeep` in the command result. Remove Overlap moves instructions on removed nodes to the nearest new on-curve node. Paths that reappear permuted are matched by place (hashed place keys), and identical contours moved onto each other's places keep their own links. The command sweep checks that no reference dangles. Per edit on Inter: 3.1 µs for a glyph without hints, 83 µs with hints. Left: background layers are not renumbered, nor are corner and cap hints |
| M10.14 | CFF hints at export: charstrings carry stem hints (sorted, positive, ghost widths for zero-width stems), hint and counter masks (where stems overlap, built from stems rounded as the charstring stores them) and the four flex forms. Manual PostScript hints export per master through node links: Get Hints From Master, the fallback master, special layers and Export PostScript Hints; a manual stem outside the charstring limits is dropped with a warning. Family Alignment Zones are written as `FamilyBlues` and `FamilyOtherBlues`; `BlueScale` 0.037 and `BlueFuzz` 1 are written when the source sets neither (handbook), with an export warning when the tallest zone breaks the `BlueScale` limit. fontTools decodes 43 exported fonts with 0 warnings; skrifa's PostScript hinter draws every glyph; the CFF oracle against fontmake has 0 different glyphs (1,838 same, as before). Left: flex for hand-hinted glyphs (M10.15) |
| X.fix20 | hint references stored in `.glyphs` files count nodes in file order (a closed path's start node last); typed hints use the model's order; the conversion is `NodeOrder` in the hinting crate. Native files store the same values |
| M11.5 (part 1) | `.glyphs` paint attributes typed in versions 2, 3 and 4: fill, stroke, stop and shadow colours with palette references; linear, radial and conic gradients with extend; shadows. The model stores `strokePos` in the version 4 convention and converts to version 3 exactly (to nine decimals, expressions saved verbatim); native format version 3 upgrades older files. Glyphs 2 colour layer names read. Writing version 3 reports paint it cannot hold; a UFO save reports layer images that do not travel (a kept image is a PNG in that UFO's own images folder). All 23 corpus colour sources keep their colour keys. Left (M11.5b): UFO full-colour layers as ufo2ft `colorLayers`, and JPEG and TIFF to PNG conversion for UFO images |

The adversarial review of this batch (X.fix21) fixed: hint upkeep matching by place only for permuted
paths; manual CFF stems rounded as the charstring stores them before duplicates are removed; the
native-format flag for version 2 kerning keys; and the UFO image-loss check, which looks where each
writer puts a master's files.

Measures: the round trips are unchanged (`.glyphs` 122/1/118/4 for model and native,
`.glyphspackage` 22/0/7/0, UFO 125 identical, 2 different, 13 failed, designspace 59 identical, 53
failed). The static oracle summary is unchanged. The variable measure is unchanged (fontc
100/37/21, fontmake 103/38/17), as are the instances (169/41/0/24).

**What is real: the audit runner, subset and the module runtime (M12.44, M12.17, M13.2, X.fix22).**
Integrated and gated with the rest. The audit runner and subset are run over the corpus; subset
is compared with pyftsubset. The editor behaviour of nothing here is compared with the reference
app.

| Package | What works |
|---|---|
| M12.44 | `audit.run` runs every check scope on the open document, on sources, compiled fonts, collections, WOFF and WOFF2 files, and on several files or a folder as a family. Sources are compiled in memory, and binary findings name the source field. Preconditions, the glyph selection syntax, profile resolution and the ignore list apply; ignored results are counted apart and left out of the exit status. `audit.checkOutlines`, `audit.checkInterpolation` and `audit.compareFamily`, and `fontcraft-cli check` with `outlines`, `family` and `interpolation`, take the reference tools' option names and exit with 0 (clean), 1 (a result at or above `--fail-on`), 2 (usage), 3 (unreadable input) or 4 (run failed) |
| M12.17 | `font.subset` and `fontcraft-cli subset` with pyftsubset's option names. Selections by text, code points, glyph names, glyph ids and `*`. The closure follows `cmap` (bidi mirrors, variation sequences), `glyf` components and CFF `seac`. It writes `cmap`, `glyf`, `loca`, CFF, `hmtx`, `vmtx`, `post`, `name`, `OS/2` and `maxp`, keeps `.notdef`, and takes `--retain-gids` and `--no-hinting`. Layout, variation and colour tables are dropped with a stated reason until M12.18 and M12.19. No new dependency (fontations' subsetter is not published) |
| M13.2 | the plug-in module runtime in `crates/plugins`: wasm modules run under wasmi with no imports and with caps on fuel, wall-clock time, memory, call depth, output size, manifest size and ABI version. Filter output is checked against the model's limits before it replaces a layer. Reporter and check outputs are parsed under item budgets. Reporter drawings are cached by module load, parameters, layer revision and scale bucket. 18 hostile-module test cases each fail with their own error and change nothing. The host is ported with credit (`NOTICE`). It builds for wasm32. No command, menu or dialog uses it yet (M13.3 to M13.6) |

Measures:

- **Audit runner:** 3,676 runs over 4 profiles and 919 corpus files, no crash. The readers refuse
  70 sources as unreadable.
- **Subset against pyftsubset:** over 340 corpus fonts and 4 selections each, the glyph map is
  equal in 338 or 339 per selection and the core tables in 337 or 338. The differences are one
  CFF2 font (refused) and two `cmap` test fonts.
- **Module runtime, 1,000 synthetic layers:** a filter takes 292 ms (target under 1 s). A reporter
  takes 0.35 ms per layer uncached (target 5 ms) and 0.4 µs cached (target 1 ms).
- Round trips, the static, variable and instance oracle summaries are unchanged (`.glyphs`
  122/1/118/4, `.glyphspackage` 22/0/7/0, UFO 125 identical, 2 different, 13 failed, designspace
  59 identical, 53 failed; variable fontc 100/37/21, fontmake 103/38/17; instances 169/41/0/24).

The review of this batch (X.fix22) fixed: subset parses CFF subroutine INDEXes once (shared local
subroutines share a slot) and keeps one charstring operation budget per font; a component past
the glyph count is an error; a format 4 `cmap` that maps U+FFFF stays format 4; the audit shares
the bytes of the faces of one file and caps the bytes, glyphs and folder entries of a run; folder
entries and the outputs of whole-file commands (subset, dehint and the rest) pass the automation
read and write root checks; plug-in output values are counted before anything is built, long logs
are skipped, and the reporter cache is capped by size.

What is left of each:

- **Audit runner:** a per-glyph hash cache; several `checkOutlines` and `compareFamily` options;
  the `BASE`, ligature and accent width checks; `summary.errors`; source-field names beyond the
  common `OS/2`, `hhea`, `head`, `post` and `name` fields. The outline checks give many warnings
  on decoded compiled fonts; the cause is not yet examined.
- **Subset:** CFF2, subroutine pruning, `--desubroutinize`, several pyftsubset options, and
  subsetting sources.
- **Module runtime:** the module commands, filters in export and checks in the audit (M13.3 to
  M13.6), menus and the Actions dialog, the Extension Manager.

**Test layout and fixes.** The 93 engine integration-test files are modules of one test binary
(`crates/engine/tests/engine/main.rs`; `cargo test -p fontcraft-engine --test engine <filter>`),
and the decompile, io-glyphs, io-ufo and plist tests are one binary each. The `measure` profile
(release optimisation without link-time optimisation) serves speed measures and oracle runs. The
gate runs the tests with cargo nextest (all test binaries in parallel, doctests apart), and the
command sweep is 16 tests (4 states times 4 chunks, chunked by a stable hash of the command id);
a full gate on a warm build takes about 5 minutes. One randomised pathops test reserves 8 threads
(`.config/nextest.toml`). The
web build compiles. Every export path reads feature includes from the source's include
folder, and the code editor's compile uses the same folder. A `.glyphs` 3 file keeps deleted
kerning deleted and keeps a version 2 kerning key the reader did not take kerning from. An include
candidate the include check refuses does not hide an allowed one; the window between the check
and the open is documented as accepted.

**What is real: CFF2 variable fonts and TrueType hinting (M9.13, M10.17, M10.6, M10.7, X.fix24).**
Integrated and gated with the rest. The CFF2 output is compared with fontmake and read with
fontTools; the TrueType font programs run in skrifa's interpreter. The editor behaviour of the
commands follows the handbook and is not compared with the reference app.

| Package | What works |
|---|---|
| M9.13 | CFF2 variable fonts from `fontcraft-cli compile --variable --format otf` (or `--format variable-cff2`) and `file.exportVariable {outlines: "cff2"}`; the Export dialog writes TrueType only. Blended charstrings (`vsindex`, blends split at the 513-value stack limit), one Font DICT with a blended Private DICT, and the item variation store; `hmtx` with `HVAR`, `post` format 2.0 and `maxp` 0.5. No hints (an export warning when the source has manual PostScript hints), no subroutines and no specialisation. `oracle --variable --format otf` against fontmake's `-o variable-cff2`: 104 equal, 34 different, 20 failed of 158 sources. Every outline and advance difference also shows in the TrueType build against fontmake. fontTools reads all 153 outputs without warnings. The TrueType variable measure is unchanged (fontc 101/37/20) |
| M10.17 | `hinting::tt_font` builds the font programs from the TrueType specification and the handbook. `cvt ` holds zone positions with their fitted rows, stem widths with their fitted widths and family zones. `fpgm` is a library of seven functions: zone rounding and alignment, rounding, cvt delta, zone snap with overshoot suppression, stem with five rounding modes and the minimum distance, and point delta. `prep` is built per size. Each behaviour the handbook describes is tested in skrifa's interpreter. M10.19 writes them into exports. The handbook's rounding-threshold example contradicts its stated default (0.09375 lifts 6.45 px to 7, the example says 6); FontCraft follows the default, and a trial check is listed |
| M10.6 | the TrueType execution model: `TouchedState` replays the touched points per direction after the first n instructions. Seven commands check their selection against it: Snap Point, Set Snap Zone, Stem, Reverse, Set Stem Reference, Make Triple Hint and Set Instruction Rounding. Each is one undo step, and a refusal leaves the font pointer-equal |
| M10.7 | seven commands: Shift Points, Interpolate, Delta and Set Delta Cells (checked against the touched state; Delta works vertically and on the main master only, with bounded PPM cells), Delete Instructions (reports the later instructions it leaves unmet and dissolves broken triple hints), Remove Hints and List TrueType Instructions. Each is one undo step |
| X.fix24 | review fixes: the component closure of decomposed CFF2 composites is linear; PPM parameters past 32767 are clamped; `prep` sizes are capped to signed words; CFF2 Private DICT deltas are checked against the region list, and region lists past one blend are refused up front; a glyph whose outline and location counts differ is an error; the dropped-hints note counts PostScript hints only; delta column keys in braces are normalised |

Measures: the round trips and the static oracle summary are unchanged. The command sweep passes
with the 14 new commands. The gate passes all eleven steps with 3,015 tests.

Left: hints, subroutines and specialisation in CFF2; the variation of TrueType composites
with intermediate layers.

**What is real: TrueType glyph programs and export (M10.18, M10.19, X.suite1, X.fix25).**
Integrated and gated with the rest. Instructed glyphs run in skrifa's interpreter, and fontTools
reads the exported fonts without warnings. The editor behaviour of the hint commands follows the
handbook and is not compared with the reference app.

| Package | What works |
|---|---|
| M10.18 | `fontcraft_hinting::glyph_program` compiles a layer's TrueType instructions plus the exported point map to `glyf` bytecode, in list order. Snap goes through the zone functions (zones, named-zone distance, five rounding modes) or rounds the point; Stem (Auto, named or No Stem) snaps its untouched origin first; Shift and Interpolate round; Delta runs before IUP; triple hints keep a pixel between stems; diagonals are preserved unless `TTFDontPreserveDiagonals` is set; Ignore Vertical Hints is honoured. `fpgm` has 10 functions. 14 behaviour tests run the programs in skrifa's interpreter and check the result in both directions. Warnings for points the outline lacks |
| M10.19 | TrueType export carries the instructions. Manual instructions compile into `glyf` programs against a point map that follows cu2qu, overlap removal and decomposition (a linear pass under one work budget per glyph). The font gets the generated `cvt `, `fpgm` and `prep`, or the imported `cvt Table` and the `prep` and `fpgm Table Assembly` parameters, `maxp` covering the analysed needs, and the `head` and `gasp` flags. Static instances interpolate TrueType zones and stems by name and take the main master's hints (TrueType and PostScript) at matching nodes. A glyph whose program fails to compile exports unhinted with a warning. Variable fonts take the default master's programs, without deltas or `cvar`; an instructed source built without glyph programs writes plain points. `hint.tt.getPointIndexes` is live (680 of 960 commands) |
| X.suite1 | FontCraft's atomic file writes go through the Craft suite's `craft-atomic` library (craft-libs, pinned by git commit; native builds only, the browser build refuses to write). Saving over a read-only file is refused and the original is kept; the folder is flushed after the rename. `docs/decisions/0008-shared-suite-libraries.md` records which suite libraries FontCraft adopts and when |
| X.fix25 | review fixes: the point map is linear (on-curve neighbours per path in one pass, contour lengths from starts, one lookup budget for coincident points, handles and created points); an off-curve node next to a node that overlap removal took away is left unmapped, with a warning; the PostScript rule for static instances (the main master's hints) is documented and has a CFF instance test; guesses on created points and remapped segments |

Measures: skrifa runs every exported glyph of the instructed corpus samples at 8 to 72 PPM in three
intents with 0 errors. The static oracle summary against fontmake is unchanged (2,360 compared,
2,202 the same), and so are the variable summary against fontc (101 equal, 37 different, 20 failed)
and the round trips. The gate passes all eleven steps with 3,043 tests.

Left: `cvar` and instruction deltas in variable fonts; the TrueType autohinter at export (M10.21);
drawing of point indexes (`showPointIndexes`); special layers' own instructions.

**What is real: this batch (75 packages and their fixes, X.integ1 to X.integ5, X.fix26, X.fix27, X.audit1, X.deps1).**
Integrated in groups and gated; the package list is under "Shipped so far". Editor behaviour follows the handbook and
the secondary references' public documentation and is not compared with the reference app.

| Area | What works |
|---|---|
| TrueType hinting | the autohinter (M10.21) with autohinted TrueType export, `autohint.tt.run`, `hint.tt.autohintGlyph`, TTFAutohint options and control instructions; the Instructor tool (M10.25) with badges, direction, pixel size and point numbers, its Info box and Delta table (M10.27), blue and red outlines on the pixel backdrop (M10.26), the Hinting Preview from 8 to 80 PPM at 1x, 2x and 4x (M10.29), the web preview (M10.30), zone and stem commands and editors (M10.5, M10.28); import of hinted fonts, written back at export (M10.22). Instructor speed: frame 0.52 ms, edit 0.53 ms, waterfall 0.024 ms |
| PostScript hinting | autohinting at export (M10.15); 11 hint commands (M10.4); hints drawn, selected, dragged and moved with the arrow keys in Edit View (M10.24); the Has Hints smart-filter rules and the Autohinting filter string (M10.31); CFF import keeps every declared stem |
| Colour | `CPAL` and `COLR` version 0 (M11.6) and version 1 (M11.7), `sbix` (M11.8), `SVG ` from svg layers and bitmaps (M11.10), `CPAL` and `COLR` read back into palettes and colour layers (M11.18), the palette editor, Master Color and Attributes inspector (M11.15); a UFO's colour layer list opens as palette layers. Palette colours are numbered in source glyph order, as both oracles number them |
| Interpolation | axis, special-layer, instance and master commands, queries and axis mappings (M9.15 to M9.18); special-layer labels and editors, the Master Compatibility view, the interpolation dialogs with the Variable Fonts export tab, Compare Fonts and Compare Family (M9.21 to M9.24); cross-axis mappings and Interpolate Fonts (M13.48) |
| Tables, TTX, CLI | TTX writers for CFF, CFF2, `VORG` (M12.5), the layout and variation tables (M12.6), the variable, colour and miscellaneous tables (M12.7), ttxn's normalised dump and the `@font` and `@glyphs` views (M12.10); `features compile`, `lint` and `convert` for FontDame and VOLT text (M12.13, M12.14); `diff` (M12.32); curve conversion and componentize (M12.29); `autohint` and `stems` (M12.39) |
| Checks and fixes | width (M12.49), glyph set (M12.47), table (M12.51) and layout (M12.52) checks; the CID check (M13.50); one source command per Font Info fix (M12.63) and every fix named by a command (M12.0); project values reach the checks of compiled fonts and `audit.fix` runs the Font Info fixes and more outline steps (X.audit1) |
| Modules and automation | module commands, Modules folder sync and safe mode (M13.3); module filters in export (M13.4); check modules in `audit.run` (M13.5); dynamic menus and reporter overlays (M13.6); the Actions dialog (M13.8); the Extension Manager (M13.10); MCP over HTTP (M13.12); author documentation and a template for each module kind (M13.14); PDF outlines and page images in `crates/io-pdf` (M13.58); BDF and PCF in `crates/io-bitmap` (M13.55) |
| Edit View and windows | component and corner Info boxes, smart settings windows, the Pixel tool (M6.11 to M6.13); the project window (M7.19); the Features tab (M8.17); display aids, drag aids, suggestions, auto-meter, power guides and the Magnet tool, Rapid and Spiro tools (M13.31, M13.33 to M13.37); Find and Replace Outline, the non-linear transform filter, autotrace, small caps and sub/superscripts, construction rules (M13.28, M13.29, M13.30, M13.19, M13.17) |
| Layout and shaping | the shaping crate on harfrust (M8.18, used by Edit View through M8.19); `MATH` and `BASE` data (M13.47); optical bounds (M13.49) |
| Spacing and kerning | side profiles, auto-spacing, Center in Width and Copy Metrics (M13.20); auto-kerning, Kern to Distance, groups from side profiles, Remove Pairs (M13.21) |

Measures that met their targets: the TrueType autohinter's size agreement (78.8% of ttfautohint's output) and cap-height rows (100%); the
construction rules build 196 of 200 measured glyphs and place 2,406 of 2,442 components within 1 unit; TTX against
ttx equals on 253 CFF, 700 layout and 143 other tables with 0 different; qu2cu equals on 3,192 with 0 different;
VOLT conversion is equal; `corpus_import` passes 27 CFF and 48 TrueType fonts; FontBakery's verdicts agree on 44
compared pairs (the `fsSelection` and `mac_style` checks are merged by design).

**Measures that missed their targets** (each has a tuning package under "What to build next", except FontDame conversion, whose differences are written down; auto-kerning and the autohinter's cap-height rows met their targets and left the list):

| Measure | Result | Target |
|---|---|---|
| TrueType autohinter, x-height rows (M10.21, M10.21b) | 92.9% agree with ttfautohint | 95% |
| TrueType autohinter, hinted extremes that land on their zone's rounded pixel row (M10.21b) | 98.29% | 99% |
| Detect Composites, composites rebuilt from the source's bases on Inter (M13.15, M13.15b, M13.15c) | 78.1% (63.0% before M13.15c) | 95% |
| Auto-spacing, median error (M13.20) | 5% to 12.5% on Oswald and 30% to 75% on Inter after the tuning pass (23% to 96% before it) | 10% (the speed target is met) |
| FontDame conversion (M12.13, M12.13b) | 24 of 28 equal; the other 4 (chained-glyph `GSUB` and `GPOS`, `gdefmarkattach`, `gdefmarkfilter`) cannot be expressed alike in both syntaxes and are written down in `docs/spec/40-compile.md` § 4b | all equal |

Oracle summaries: static against fontmake, TrueType 2,370 compared and 2,212 the same, with 39 of 354 sources
differing; CFF 2,371 compared and 1,848 the same, with 38 of 354 differing. Colour glyphs add 10 glyphs to the
TrueType counts. `COLR` against fontc: 10 identical, 7 allowed, 0 different; against fontmake: 11, 7, 0. Round trips:
`.glyphs` 122/1/118/4 (native and model), UFO 133/7, designspace 112, `.glyphspackage` 22/0/7/0. The gate passes all
eleven steps with 3,729 passed, 0 failed, 42 skipped tests.

Decisions: a trusted external ttfautohint binary runs only when it comes from the `export.ttfAutohintBinary`
setting or `--ttfautohint`, never from the document (a departure from the reference, for safety); CFF import keeps
every declared stem, because the handbook says hints are preserved and the measure counts declared stems; the hinting
web preview defaults to the local network (the M10 plan's choice), with an "Only this computer" option; `CPAL`
colours are numbered in source glyph order. Fixes from the pre-push review: the external binary no longer comes from
the document, the control channel's instruction budget, the MCP HTTP host check, plugin index caps, and a stack
overflow on deeply nested SVG.

Left from this batch: see "What to build next".

**What is real: the next batch (17 packages, X.tune1, X.integ6, X.fix29, X.fix30).** Integrated and gated; the
package list is under "Shipped so far". Editor behaviour follows the handbook and the secondary references' public
documentation and is not compared with the reference app.

| Area | What works |
|---|---|
| Layout and shaping | Edit View shapes text through a preview font compiled from the font's feature code, with cursive attachment and live advances, and a Features menu of four commands (M8.19); `AuditServices::shape` runs the same shaping for the checks; feature-code table blocks and `usMaxContext` computed from the compiled lookups (M8.8); contextual anchors (`anchor.setContext`; ufo2ft's `marks_contextual` ported, M8.21) |
| Colour and images | `CBDT` and `CBLC` (M11.11, `Options::cbdt`); variable `COLR` with `PaintVar*` paints, delta-set indexes and layer glyphs varied through `gvar` (M11.19), and instances between masters interpolate colour layers; images in Edit View with an image cache, PDF pages, drag gestures, an info box, a context menu and drop onto the canvas (M11.13) |
| Tables and command line | the TTX reader for the core tables, CFF, CFF2 and `VORG` (M12.8; `tables import`, `tables.importTtx`); static instances of variable TrueType fonts (M12.20; `font.instantiate`, `instance`); merge (M12.23; `font.merge`, `merge`); transform and upem scaling (M12.24; `font.transform`, `transform`); the CFF subroutiniser in `crates/cff` (M12.25) |
| TrueType hinting | `hint.tt.setAssembly` with the TrueType Instructions dialog and a gloss for each opcode, and the Review Hints dialog (M13.51); the TrueType Debugger window, `hint.tt.debugStep` and `hint.tt.suggestDeltas` on an interpreter ported from skrifa (M13.52); the preview's instance pop-up (M9.20). A glyph's own program wins over its Instructor hints at export |
| Spacing and kerning | the Pairs and Phrases section with auto-kern review, the Kerning Pairs Preview and the Groups Editor (M13.22); the Space Center with text, control glyphs, size, line modes, the cut-off band, the preferred suffix and a matrix of widths and sidebearings (M13.23); auto-spacing and auto-kerning tuned (X.tune1) |
| Fixes | `path.eraseOpenCorners`, `component.flatten`, and `audit.fix` layer steps 7 to 33 with binary results fixed on the source masters (M12.62); `glyph.setShapeOrder` renumbers hints and stands in for the removed duplicate `path.setShapeOrder` |

Measures from this batch:

| Measure | Result |
|---|---|
| Oracle, static TrueType against fontmake, full corpus | 2,376 compared, 2,218 the same, 139 coverage-equal, 19 different; 39 of 354 sources differ (before: 2,370, 2,212, 139, 19, 39). `GSUB` 25 of 26, `GPOS` 82 of 88 and `GDEF` 47 of 55 equal. M8.8 removed 5 allow-list entries and no differences came back |
| Contextual anchors (M8.21) | output identical to fontmake's on the probe source; the `GPOS` lookup order matches (the `abvm` and `blwm` mark-to-mark lookups last). fontc ignores contextual anchors |
| `COLR`, static | against fontc 10 identical, 7 allowed, 0 different; against fontmake 11, 7, 0 (unchanged) |
| `COLR`, variable (M11.19) | COLRv1-var, COLRv0-var and COLRv0-2masters-brace equal to fontmake's variable builds (compared with ttx; one allow-listed difference in a layer glyph's name). fontc fails on COLRv0-2masters-brace itself |
| CFF subroutiniser (M12.25) | 233 corpus fonts, 0 outline differences, 0 fontTools warnings; total size 0.785 of cffsubr's; 3 small test fonts are more than 2% over (Lobster.subset 1.020, ttLib I.otf 1.167, TestOTF 1.096) |
| Static instances against fontTools `varLib.instancer` (M12.20) | 26 cases on 8 corpus variable TrueType fonts: every table identical except `GPOS` in 18 (the subtable layout is kept from the input; fontTools rebuilds it) and `maxp` in 2 (not recalculated). At most 1,024 instances per run, with staged outputs |
| Transform against fontTools `scaleUpem` (M12.24) | `glyf` identical in 12 of 12; `cvt ` differs in 4 (expected), `maxp` and `head` flags differ (no `maxp` recalculation); the `avar2` item variation store is left unscaled; `.null` stays in `post`. A cubic flag dropped on rewrite was fixed |
| Merge against fontTools merge (M12.23) | 8 pairs: `OS/2`, `cmap`, `name`, `hhea` and `loca` identical; `glyf` and `head` differ in 5 (fontTools recalculates bounds); duplicate glyph names are `#n` here and `.n` there in 2 |
| TTX reader against fontTools `importXML` and save (M12.8) | 16 fonts: `maxp`, `OS/2`, `name`, `cmap`, `post`, `hmtx`, `CFF `, `glyf`, `loca`, `cvt `, `gasp`, `prep` and `fpgm` identical; `head` and `hhea` differ on CFF fonts (fontTools recompiles the font bounding box). The layout tables, `BASE` and `kern` are not readable yet |
| Auto-kerning, sign agreement with the sources' kerning (X.tune1) | 85.7% to 99.2% in every master; target 80%, met |
| Auto-spacing, median error (X.tune1) | Oswald 5% to 12.5%, Inter 30% to 75%; target 10%, missed on Inter. This method's best fit to the sources' own spacing is about 60% there |
| Space Center | a frame with 200 glyphs takes 1.40 ms (release) |
| TrueType debugger (M13.52) | the end points of glyph programs equal the hinted preview's |

The gate passes all eleven steps with 3,897 tests passed, 0 failed, 41 skipped. Decisions: the TrueType debugger runs on a ported copy of skrifa's
interpreter (credited in `NOTICE`), which departs from the M13 plan's rule of one interpreter; the copy's end points are
tested against the preview's, and the copy can go if skrifa gains a stepping API. A glyph's own program wins over its
Instructor hints at export; it is kept when the Instructor tables cannot be built and left out with a warning when it
needs font programs that are not exported. Auto-spacing clips at 10% of the x-height and fits its depth weight on the
control glyphs n, o and v (H, O and V); auto-kerning clips at 30% of the x-height and takes the power mean of order -2 of
the row distances; Build Kerning Groups keeps 15%. Review fixes (X.fix29, X.fix30): shared work budgets and memoised
`maxp` values for composite chains in TTX import and the instancer; instancing capped with staged outputs; split TTX
parts confined to the dump's folder and read-checked; a memory budget for the TTX tree; an anchor context that names an
unknown glyph or class is left out with a warning; the TTX oracle fixtures hold a SHA-256 for each table.

Left from this batch: see "What to build next".

**What is real: the ninth batch (14 packages, X.tables1, X.ds1, X.small1, X.integ7 to X.integ9, X.fix31 to X.fix34).**
Integrated and gated; the package list is under "Shipped so far". Editor behaviour follows the handbook and the
secondary references' public documentation and is not compared with the reference app.

| Area | What works |
|---|---|
| Text and layout | Text Preview shapes each line through the preview font and orders mixed-direction runs by UAX #9 (M8.20; the preview font is built in the background, and text falls back to code point order with a message while it builds); the Space Center's sample-strings menu, waterfall, right-to-left, centred and upside-down modes, fill, stroke and inverse display, guides, zones, up to eight overlay layers, line space, tracking and the metrics beam (M13.23b; `view.setSpaceCenterDisplay`) |
| Colour | Edit View and the previews draw colour glyphs: palette colours, gradients, strokes, `iColor` layers and `SVG ` glyphs, with the layered Master Color and gradient knobs (`attributes.setGradientPosition`); each glyph is rasterised on the CPU once and cached as a texture (M11.14). A colour glyph that needs more paint than the budget allows draws as one plain fill, the silhouette of the colour drawing |
| Instances | `font.instantiate` on designspace and multi-master sources, written as UFOs or compiled (M12.22); designspace `<instance>` names, `usWeightClass`, `usWidthClass` and font info, and designspace `<rules>` in static instances and variable fonts as glyph swaps and feature variations (X.ds1); partial instances of variable TrueType fonts that pin axes, limit ranges, move the default or drop axes, on a port of the instancer's region solver (M12.21). Instance file names resolve from the source's folder as fontmake resolves them, and an output on or inside a source file is refused |
| Command line and tables | static CFF exports are subroutinised by default and kept only when smaller; `font.optimize` subroutinises, desubroutinizes and repairs CFF widths, and keeps the input's container (.ttc, WOFF, WOFF2) and extension (M12.26); `variations` `build-hvar`, `model`, `plot` and `plan-avar` (M12.34); the TTX reader for `GSUB`, `GPOS`, `GDEF`, `BASE`, `JSTF`, `MATH`, `STAT`, `fvar`, `avar`, `meta`, `kern`, `gvar`, `cvar`, `HVAR`, `VVAR`, `MVAR`, `CPAL`, `COLR` version 0 and `SVG ` (M12.9); TTX writers for `COLR` version 1, the bitmap tables and the Apple tables `ankr`, `bsln`, `cidg`, `feat`, `gcid`, `lcar`, `ltag`, `opbd` and `trak` (M12.7b); FontDame conversion with AGLFN names for code points (M12.13b); one `maxp` and `head` recalculation shared by instance, transform, merge and TTX import (X.tables1) |
| Checks and fixes | `audit.fix` on compiled font files: all-or-nothing in a temporary copy, with `dryRun`, `safeOnly` and skipped reasons, and the table fixes `tables.recalculate`, `cleanNameRecords`, `rebuildCmap`, `recompileLayout`, `cleanInstances` and `dropAxis` (M12.61); the Font Audit panel and the Audit Profile dialog, with `audit.profile.list` listing built-in, project and user profiles (M12.64) |
| Hinting and small features | the TrueType autohinter scales to the x-height in `prep`, rounds zones as ttfautohint does, and applies left and right offsets and point shifts after IUP (M10.21b); Detect Composites chooses bases by combining marks, suffixes and the shortest shift, and the per-edit alignment pass keeps a users index (M13.15b); a `CBDT` option in `font.compile`, the export commands, the Export dialog and the CLI; `hint.tt.setAssembly` warns when export would leave out a program that needs font programs; arrow keys move selected PostScript hints (X.small1) |

Measures from this batch:

| Measure | Result |
|---|---|
| Oracle, static CFF against fontmake | 2,377 compared, 1,854 the same, 511 coverage-equal, 12 different; 38 of 354 sources differ (before: 2,371, 1,848, 511, 12, 38). Static TrueType was not measured again. Exported CFF fonts are 0.957 of the size of fontmake's unsubroutinised builds |
| CFF subroutines against cffsubr (M12.26) | 14 fonts: 0.974 of cffsubr's size |
| Static instances against `fontmake -i` (X.ds1) | 180 the same, 30 different, 0 not built, 24 failed in fontmake itself (before: 169, 41, 0, 24) |
| Designspace `<rules>` against fontmake | 3 sources: 2 the same, 1 different (CustomFeatures: a rule that feeds several feature tags) |
| Instances from sources against fontmake's UFO instances (M12.22) | 39 instances: 6 differ in a glyph (`iacute`) and 8 in font info |
| Partial instances against fontTools `varLib.instancer` (M12.21) | 38 cases on corpus variable TrueType fonts: 9 equal, 5 refused (`avar` 2), 3 different (each equals fontTools' own full instance), 21 failed inside fontTools |
| `variations` tools against fontTools (M12.34) | `model` agrees in 104 of 112 cases; `build-hvar` 8 of 8; `plan-avar` on NotoSans-VF-cubic differs by 0.004; `plot` refuses varc-6868 and Amstelvar-avar2 |
| TTX reader (M12.9) | 39 corpus fonts: all tags identical after reading the TTX back; 239 of 240 CFF fonts identical |
| TTX writers for the Apple, bitmap and `COLR` version 1 tables (M12.7b) | 31 of 34 cases equal to ttx; the other 3 are the `prop` dump (written raw) and `cmap` format 2 |
| `maxp`, `head` and `post` after instance, transform and merge (X.tables1) | equal to fontTools' on the 12 comparison cases; `maxSizeOfInstructions` keeps its stored value unless glyph programs were removed, as fontTools does |
| FontDame conversion (M12.13b) | 24 of 28 equal; VOLT equal |
| Round trips | unchanged: `.glyphs` 122/1/118/4, UFO 133/7, designspace 112 |
| TrueType autohinter (M10.21b) | x-height rows 92.9% (target 95%), cap-height rows 100%, zone agreement 98.29% (target 99%), size 78.8% of ttfautohint's output |
| Detect Composites (M13.15b) | 63.0% of Inter's composites rebuilt from the same bases (target 95%) in 224 ms; the per-edit alignment pass takes 0.24 ms (target 2 ms, met) |
| Colour drawing (M11.14), measure profile | a steady frame 0.53 ms and a scrolling frame 0.53 ms; the first frame took 27.2 ms before the paint budget |
| Text Preview (M8.20), release | preview font build 186 ms; bidi over 100,000 characters 7.5 ms (target 30 ms); a 30-line page shaped in 0.38 ms (target 8 ms); a million characters laid out in 264 ms (limit 500 ms) |
| Space Center (M13.23b), 200 glyphs | 1.30 ms plain, 1.44 ms with every display option |

The times were taken at a load of 15 to 18 on 16 cores and are upper bounds. The gate passes all eleven steps with
4,042 tests passed, 0 failed, 43 skipped. Decisions: instance file names from a designspace may contain `..` and
resolve from the designspace's folder, as fontmake does, while outputs on or inside a source file are refused and names
built from family and style names may not contain `/`; a designspace instance's own `weightClass` and `widthClass`
lib keys are ignored, as fontmake ignores them; `font.optimize` keeps the input's container and extension;
`audit.fix` with `inPlace` through a symlink writes the linked file and keeps the link (a guess on the row). Review
fixes (X.fix31 to X.fix34): sparse `gvar` and `cvar` delta sets with an early tuple cap, glyph point counts checked
against the flags, one subtable budget for each bitmap location table, `gvar` decoding charged and cached per glyph,
name strings capped and deduplicated; a linear `cmap` conflict report, hashed feature-variation and `HVAR` region
deduplication under a cap on store regions, one draw budget and hashed measurement tables for each `avar` plan, a byte
cap on each rebased delta item, one budget for designspace rules with composed glyph swaps, a linear `prep` search,
glyph grouping by font dictionary, a stack check on widths and a cell cap on the control
corrections; instance outputs that never replace or leave the source, `plan-avar` writing the font and the plot
together, the colour paint budget, resolved paths for output collisions, a write check before staging, the face chosen
by PostScript name on single-font files, and a warning when the rule budget cuts designspace rules.

Left from this batch: see "What to build next".

**What is real: the tenth batch (17 packages, X.conv2, X.small2 to X.small6, X.integ10 to X.integ12, X.fix35 to X.fix40).**
Integrated and gated; the package list is under "Shipped so far". Outputs are compared with the open tools named
below. The editor parts follow the handbook and the secondary references' public documentation and are not compared
with the reference app.

| Area | What works |
|---|---|
| Opening compiled fonts | `GSUB`, `GPOS`, `GDEF` and `BASE` become feature code on open: classes, languagesystems and prefix code, single-use lookups written inline, lookup order kept, pair kerning back in the kerning data when it can be (M8.23). 302 corpus binaries export again after the fixes (13 failed before them), and InterVariable opens in 135 ms. Tables the model does not hold are kept on open and pass through compile unchanged until the glyph order changes (X.small2) |
| Subset | `GSUB` closure, `GSUB`, `GPOS` and `GDEF` pruning with `--layout-features`, `--layout-scripts` and the closure switch (M12.18); `gvar`, `HVAR`, `VVAR` and `GDEF` stores, `COLR` with layer reuse, `CPAL`, `SVG `, `sbix`, `CBDT` and `CBLC`, `VORG`, `hdmx`, `kern`, `MATH` and the name records still referenced (M12.19); CFF2 with its Font DICTs and Private DICTs, unused Font DICTs dropped, subroutine pruning for CFF and CFF2 and `--desubroutinize` (M12.19b). Where pruning cannot be done safely the subroutines stay and a warning says so |
| Convert | `font.convert` and `fontcraft-cli convert`: CFF and CFF2 to TrueType as otf2ttf does, TrueType to CFF and CFF2, CFF to CFF2 and back, bare `cff` and `cff2` tables, WOFF and WOFF2 wrapping; variable fonts are refused (M12.30). The curve error is one font unit, fractions are kept, FontMatrix is ignored as fontTools ignores it, and seac becomes composites in TrueType and is drawn out in CFF2 (X.conv2). Outputs are staged and written together |
| Project compile | `font.compile` and the CLI take an AFDKO project: the project file, option order, `fontinfo`, FontMenuNameDB, GOADB renaming and code pages (M12.40). The compile then follows makeotf 5.0.1: names 1 to 6 and the menu names only, `OS/2` and `hhea` computed as makeotf computes them, the version from the feature code's `FontRevision`, overlaps kept, `kerning.plist` ignored, composites as the file places them (M12.40b). CID-keyed CFF with a CID writer (ROS, CIDCount, FDArray, FDSelect, CID charset), CMap files and one-byte Mac CMaps (M12.41). Compile extras: TrueType curve modes, implied on-curve points dropped by default as fontc does, the subset switch, `meta`, `VORG`, stub `DSIG`, pass-through tables and `.ttc` and `.otc` output (M12.42) |
| Instances and merge | CFF2 instances, full and partial, with the downgrade to CFF, variable `BASE` and `--update-name-table` (M12.20b). `merge` joins CFF fonts by glyph name, with `MATH` and `BASE`; variable TrueType fonts merge as their default outlines with a warning, and CFF2, CID-keyed fonts and `JSTF` are refused as fontTools refuses them (M12.23b). Merged glyph bounds are recomputed as fontTools does (X.small3) |
| TTX | readers for `COLR` version 1, the bitmap tables (`EBLC`, `EBDT`, `CBLC`, `CBDT`, `bloc`, `bdat`, `sbix`, `EBSC`), the Apple tables, `DSIG`, `FFTM`, `LTSH`, `VDMX` and `cmap` format 2, and `ttx -g` split dumps by glyph, written and read back (M12.7c) |
| Colour and images | images and colour glyphs in Font View cells, with image files dropped onto cells (M11.16); a colour Text Preview that draws `COLR`, `SVG ` and `sbix` from the compiled font under paint budgets (M11.17); a UFO's full-colour layers read and written as ufo2ft `colorLayers` paints, and JPEG and TIFF images saved as PNG (M11.5b) |
| Audit and save | the Table Viewer window over `tables.show`, `font.dumpTables` and `font.diff`, with Fields, TTX and Differences views, a hex pane and search (M12.38); audit run ids, runs in steps with progress and cancel, a fix preview, report export and the `recommendedStyleLinking` check (M12.64b); save choosers for the Font Audit report, the project window, File › Save As and the Actions window, with the format chosen by the extension (X.small5, X.small6); a `CBDT` setting in export profiles and a warning for glyph programs that call undefined functions (X.small3); silent truncations of selections, external-change text and autohint reports now warn, and export profiles autohint when asked (X.small4) |

Measures from this batch:

| Measure | Result |
|---|---|
| Subset against pyftsubset, layout (M12.18) | 909 runs: `GSUB`, `GPOS` and `GDEF` equal wherever both tools run |
| Subset against pyftsubset, variation and colour tables (M12.19) | 33 of 33 equal |
| Subset against pyftsubset, CFF and CFF2 (M12.19b) | 1,608 runs: 536 of 536 equal in each of prune, desubroutinize and no-hinting |
| Convert against otf2ttf, tx and fontTools (X.conv2, M12.30) | CFF to TrueType 43,967 identical, 0 off by one, 26 different (before X.conv2: 12,449, 446 and 31,098); TrueType to CFF 36,466 the same and 203 different (the difference is glyph names of `post` format 3 fonts, written down); CFF to CFF2 43,963 equal and 30 different (seac, written down); CFF2 to CFF 43,993 equal, 0 different; WOFF2 331 equal, 0 different |
| Project compile against makeotf (M12.40, M12.40b) | 7 of 10 corpus UFOs give a dump equal to makeotf's (makeotf itself fails on 114 others); the 3 others differ in advance widths and in composite outlines |
| CID-keyed CFF against makeotf (M12.41) | 13 of 13 keys equal, with the `dev` and `noMac` options |
| Compile extras (M12.42) | implied on-curve modes equal to fontc's and fontmake's in 484 of 484 glyphs each way; pass-through tables 31 of 31 after `tables add` and `optimize` |
| Merge against fontTools (M12.23b) | 10 pairs merged by both and 2 refused by both; 143 tables the same, 2 different (`hhea` on fontTools' CFF test data; `BASE` tag lists kept once, a guess) |
| CFF2 instances against `varLib.instancer` (M12.20b) | every full instance and downgrade equal; the partial range cases differ by fontTools' own partial-then-full rounding; `--update-name-table` 19 equal, 0 different |
| TTX readers and split dumps (M12.7c) | every tag identical in the measure; 30 of 30 split dumps read back, and 2,750 split files equal to ttx's |
| Layout rebuilt on open (M8.23) | 302 corpus binaries export again; `GDEF` 282 equal, 20 different; `GSUB` 269, 3 and 30; `GPOS` 202, 51 and 49 (equal, different, not comparable) |
| Speed | Font View with 691 image cells, steady frame 1.08 ms; colour Text Preview, steady frame 0.39 ms |
| Oracle summaries | unchanged: fontmake CFF 38 of 354 sources differ; fontc TrueType 63 of 354 |

The gate passes all eleven steps with 4,237 tests passed, 0 failed, 46 skipped. Decisions: convert follows the oracles
(one font unit of curve error, fractions kept, FontMatrix ignored, seac as composites in TrueType and drawn out in
CFF2); implied on-curve points are dropped by default in a TrueType compile, as fontc does, and
`--keep-implied-oncurves` gives fontmake's output; project compile follows makeotf 5.0.1 (see the table above);
merged bounds follow fontTools 4.66, which skips components that use point bounds; save choosers show one filter
and the extension picks the format (the reference has a format pop-up; a guess on the rows); untrusted fonts are
held to a charstring memory limit of 1 GiB (256 MiB on wasm) and a TrueType-to-CFF point limit of 16 million (8
million on wasm). Integration fixes: subsets lost `GDEF` classes because the class remap clamped ranges to the output
glyph count; opening a compiled font failed to export again for 13 corpus fonts because a call to an empty lookup
made fea-rs panic, a repeated ligature stopped the rebuild, and virtual glyph ids confused entries, which now stay
off; a CFF2 subset prunes before it dehints. Review fixes (X.fix35 to X.fix40): list and plain-fill edge cases,
recorded names with a root or drive, and staged links out of the write root; project-compile paths checked against
the read root, convert outputs staged and committed together, a byte cap on collections, which need a .ttc or .otc name, a
per-frame decode budget for cell images; empty-lookup calls and 16-bit overflows of lookup and feature counts
refused before fea-rs sees them, backwards `SVG ` and `COLR` ranges skipped, `BASE` and `MATH` counted under the
layout limit, one parse per `SVG ` table, caps on bitmap strikes and entries; a shared charstring memory budget in
convert, instance, subset and dehint, and shared point and visit budgets in convert; class pair tables with both
value formats 0 kept when patch, transform, instance, merge and subset read `GPOS` again; seac parts parsed and
drawn once per table, TTX bitmap image sizes refused before the padding is allocated, `--desubroutinize` held to the
memory limit (pruning that passes a limit keeps the subroutines with a warning), linear-time split-dump glyph file
names, split-dump parts that cannot lead outside the dump folder through links, staged audit findings kept until the
report is written, save choosers adding the offered extension to a bare name, and stored selections never written
back cut.

M12.19c, X.fix41 and X.fix42 were integrated with the eleventh batch.

Left from this batch: see "What to build next".

**What is real: the eleventh batch (M9.25, 20 follow-up packages, X.fix41 to X.fix45, X.tables2, X.ui1, X.audit2, X.audit3, X.opt1, X.opt2, X.edit1, X.integ13 to X.integ16, X.fix47 to X.fix54).**
Integrated and gated; the package list is under "Shipped so far". Outputs are compared with the open tools named
below. The editor parts follow the handbook and the secondary references' public documentation and are not compared
with the reference app.

| Area | What works |
|---|---|
| Subset | `VARC`: closure over variable components together with the `glyf` components, glyph records renumbered, `ConditionList` and `AxisIndicesList` pruned, the multi-item variation store cut to the delta sets and regions still used (M12.19c). Compressed `SVG ` documents are inflated and compressed again; composite bitmaps keep their components in the output numbering (M12.19d). List options take `=`, `+=` and `-=` as pyftsubset does; drop and no-subset table lists, name obfuscation, recalculated bounds, timestamp, average width, maximum context, canonical order and WOFF or WOFF2 output; the output container follows the input (M12.18b) |
| TTX | `mort` and `morx` dumped and read as ttx writes and compiles them, with limits on chains, states, classes and items (M12.7d); bitmap images in the row and bitwise forms of `ttx -z` read back; external-file images are refused (M12.7e) |
| Convert and transform | convert reads Type 1 (PFA, PFB) as `tx -cff` writes it, bare CFF tables and TTX (M12.30b). `transform --upem` on CFF and CFF2 as fontTools' `scaleUpem`, and `--to` (M12.24b); a matrix on CFF and CFF2 outlines as rotatefont applies it, `--glyph-table`, `--fd` and `--exclude-fd`, and a matrix on variable TrueType fonts through the `gvar` deltas (M12.24c). Instance, transform and merge write `post` versions 1, 3 and 4 again as fontTools does, merge names glyphs as fontTools' `getGlyphOrder`, and outline repairs recalculate `maxp` (X.tables2) |
| Instances | partial instances of `avar` version 2 fonts, `VARC` axes checked and renumbered, and variable `COLR` fields and store instanced (M12.21b). Instances from sources: of two same-named instances the later wins with a warning, as fontmake; components stay where the steps put them; numbers only some sources set are interpolated; the instance's custom parameters apply as glyphsLib applies them (M12.22b). `--layout-only` interpolates `GPOS` from a designspace's compiled masters as fontTools' `interpolate_layout`, with a master finder; UFO instances hold only the style-map names the designspace states (M12.22c) |
| `optimize` | implied on-curve points dropped, `gvar` deltas optimised, the `GDEF` variation store merged and `COLR` version 1 rebuilt with layer reuse, each ported from fontTools (X.opt1); `GPOS` class-kerning compaction at levels 1 to 9 (X.opt2). With no step named, `optimize` keeps implied on-curve points and does not compact `GPOS` |
| CFF and CJK | the subroutiniser reads one glyph at a time and charges its arrays to a 4 GiB budget (512 MiB on wasm), so full CJK fonts subroutinise (M12.25b); charstring operators keep their operands in one buffer per glyph, and CFF to TrueType encodes glyph by glyph under a limit of 2^26 points (X.fix42) |
| Project compile | makeotf's project keys: input, output and feature paths, the licence code, `-osbOn`, `-osbOff` and `-osv` (OS/2 version 3 by default, as makeotf), Unicode Variation Sequences as a `cmap` format 14 subtable, CMap keys, and two-byte Mac CMaps as a format 2 subtable (M12.40d) |
| Colour and images | UFO full-colour layers of paths as glyphsLib's `.colorN` layer glyphs, folded back on reading; `PaintColrGlyph`, variable paints and gradients under turned components (M11.5c). A component of a colour glyph compiles to `PaintColrGlyph`, moved by `PaintTranslate` or `PaintTransform`, with clip boxes following the reference (M11.7b). The colour Text Preview reads compressed `SVG ` documents, fits a document's viewBox, draws embedded PNG and JPEG images and `CBDT` strikes, and shows why the colour font could not be built (M11.17b). Edit View images: Palette transformations, arrow-key moves, Delete, and PDF pages placed at their size in points (M11.13b) |
| Text | Text Preview breaks lines at UAX #14 opportunities on shaped widths and sets vertical text shaped with `vert` (M8.20b); one feature compile per revision serves `features.compile`, Edit View and Text Preview (X.edit1) |
| Model and files | higher-order interpolation data on nodes, typed in the model, lossless in `.glyphs` version 4, the native file and the UFO node lib key, and reported before version 4 (M9.25); bitmap strikes in the model and the native format, with `bitmap.importStrikes`, `bitmap.exportStrikes` (BDF, PCF) and `bitmap.setStrikes` (M13.55b); a kerning entry turned back on kerns each pair once, and one feature-code writer (M8.23b) |
| Audit | `audit.fix` runs the glyph-data steps 1 to 6, the kerning and autohint steps of the third stage, and reorders shapes by the full proposed contour order; `tables.recalculate` recomputes the `maxp` maxima and the `vhea` extents (X.audit2); feature-code steps 35 to 37, path reversal before a start-point move, tabular kerning, the CFF FontBBox and `isFixedPitch`, and families of files repaired together (X.audit3) |
| Window | Shift with a tool letter steps through a shared button; Info for Selection opens the Smart Glyph Settings of `_part.` and `_smart.` glyphs; Copy and Paste of instances in the project window; check marks on the View › Suggest switches; theme tokens for the display aids and the auto-meter (X.ui1) |

Measures from this batch:

| Measure | Result |
|---|---|
| Subset of `VARC` against pyftsubset (M12.19c) | 14 of 14 runs equal; a 15th hangs inside fontTools (its `VARC` closure loops on a self-referencing component) |
| Subset, other (M12.19d, M12.18b) | compressed `SVG ` 6 of 6 equal; the earlier subset measures unchanged |
| TTX `mort` and `morx` against ttx (M12.7d) | 100 of 100 dumps and reads equal, on fonts built with fontTools (the corpus has none) |
| UFO colour layers against glyphsLib (M11.5c) | 10 of 10 round trips equal |
| Partial instances (M12.21b) | `avar` version 2: 5 of 5 equal on Amstelvar; `VARC` instancing has no oracle (fontTools 4.66.1 cannot read the corpus `VARC` fonts) |
| Layout-only instances against `interpolate_layout` (M12.22c) | 6 the same, 2 refused by both |
| Instances from sources against fontmake (M12.22b) | 39 instances: 0 glyph differences (was 6, `iacute`), 1 font-info difference (was 8) |
| `optimize` against fontTools (X.opt1, X.opt2) | `GPOS` compaction 117 equal (fontTools crashes on 1); `gvar` IUP 9 equal, 0 different; implied on-curve points 92 and 0; variation store 3 and 0; `COLR` reuse 5 and 0 |
| Units per em on CFF and CFF2 against `scaleUpem` (M12.24b, X.fix52) | 21 fonts: the only differences are the `OS/2` first and last character index (fontTools recomputes them at save), CFF2 blended Private values (written down), and one font's empty-coverage `GPOS` pair subtable fitted to its used classes |
| Merge against fontTools (M12.23b, X.tables2) | 143 tables the same, 2 different (unchanged) |
| CJK: Source Han Serif TC Bold (X.fix42, M12.25b) | to TrueType in 7.6 s, 354 MB peak; charstring memory peak 141.5 MiB (limit 1 GiB), 10.56 million points (limit 2^26, about 67.1 million); subroutinised in 136 s, 692 MB peak (budget 4 GiB), 0.883 of the unsubroutinised size (refused above 8.4 million tokens before) |
| Feature compile shared with the previews (X.edit1) | an edit to compile state and preview fonts on Inter: 200 ms (was 374 ms) |
| Variable fonts against fontc and fontmake | fontc 103 equal, 35 different, 20 failed (was 101, 37, 20); fontmake 106, 36, 16 (was 104, 38, 16) |
| Other oracle summaries | unchanged: static TrueType and CFF against fontmake, `COLR` against fontc and fontmake, static instances (180, 30, 0, 24) |
| Speed (release, under load) | preview font build 169 ms; bidi layout 30.9 ms; a 30-line page 0.38 ms; shaped-width layout with an empty cache 29.0 ms; colour Text Preview 0.39 ms; a million characters 286 ms, 319 ms on shaped widths (limit 500 ms) |

Not run in this batch: the measures that run AFDKO's tools (Type 1 and bare CFF input against tx, the makeotf
project compile and CID keys, rotatefont matrices, the subroutiniser against cffsubr, and the CFF2 variable compile
against fontmake). They are pending.

The gate passes all eleven steps with 4,431 tests passed, 0 failed, 49 skipped. Decisions: the feature-code lookup
guard counts lookups as fea-rs groups them and is an upper bound, so it may refuse valid code only very near 65,535
lookups written in rare forms; `optimize` with no step named keeps implied on-curve points, as fontTools does, and
when the step is named it tests every tuple peak and keeps points that composites, anchors or carets number;
`GPOS` compaction is off unless a level is given; project compile writes OS/2 version 3 by default, as makeotf;
variable `COLR` is instanced (fontTools leaves it untouched; a guess); of two same-named instances the later wins
with a warning, as fontmake; audit folder entries outside the read root are skipped with a warning; subset
`--drop-tables` is a whole list and the output container follows the input; every command that writes `GPOS` fits
class-pair record counts to the class definitions. Integration fixes: CFF2 instancing kept the blends of a glyph's
first operator; layout-only instances create their output folder; the preview build cache counts only includes from
outside the assembly. Review fixes (X.fix47 to X.fix54): the lookup guard counted too few lookups after a nested
lookup block, which could let fea-rs abort on wasm; a colour reference to a removed glyph is drawn as an outline;
colour reference checks take linear work per master; Unicode Variation Sequence values are checked; a stub `cmap`
keeps the Mac and format 14 subtables; `mort` state cells are charged for their entries' actions (a hostile table
could exhaust memory); `VARC` records are parsed once; the UFO colour-layer fold looks up each layer glyph's users
instead of scanning the font; removing a glyph removes its strike bitmaps; a layout-only master is checked against
the read root before it is read; shared variation-store subtables are charged per use; `interpolate_layout` charges
every master's `GPOS` against one budget; `avar` version 2 regions merge by hash; `gvar` zero point counts mean all
points; CFF bounds count move points; dehint copies a glyph's data only up to its end.

Left from this batch: see "What to build next".

**What is real: the twelfth batch (12 follow-up packages, X.ui2, X.fix55, X.integ17, X.integ18, X.fix56 to X.fix58, X.gate16).**
Integrated and gated; the package list is under "Shipped so far". Outputs are compared with the open tools named
below. The editor parts follow the handbook and are not compared with the reference app.

| Area | What works |
|---|---|
| Merge | variable TrueType fonts with the same axes merge as variable fonts: the variation stores are joined with shared regions, and fonts whose axes differ are refused (M12.23c). A merge that drops `fvar` gives a static font with the default values and no variation stores. fontTools has no variable merge, so every rule is ours |
| CFF2 export | variable CFF2 exports are specialised as fontTools specialises merged charstrings, and subroutinised when that makes them smaller (M12.25c); manual and autohinted PostScript hints blend across masters as varLib merges hinted masters, and a glyph whose masters' hint structures differ loses its hints with a warning (M12.25d) |
| CID-keyed CFF | the ROS custom parameter exports a CID-keyed CFF: CIDs come from `cid<n>` names, then from a user CMap, and the other glyphs are appended (M12.41b). No character collection data is bundled |
| TTX | bitmap images written in the row, bitwise and external-file forms of `ttx -z` (`dump -z`, the `bitmapFormat` parameter), and external-file images read back from the dump's folder (M12.7f) |
| Bitmap strikes | an `EBDT` and `EBLC` writer for the model's strikes; `bitmap.exportStrikes` writes embedded strikes and `.otb` files, and `bitmap.setStrikes` rasterises new strikes from the outlines (M13.55c). BDF, PCF and `.otb` fonts open as fonts, and a compiled font's `EBLC` and `EBDT` (or `bloc` and `bdat`) strikes are read into the model (M13.55d) |
| Colour | Edit View and the `SVG ` table draw a reference to a colour glyph as that glyph's colour layers, moved by the component's transform; cycles and references nested too deep keep the grey outline (M11.14b). The colour Text Preview draws `SVG ` clip paths, masks, patterns and sliced images (M11.17c); filters are not drawn |
| Text | vertical Text Preview is shaped top to bottom through the preview font, with `vkrn`, `vrt2` when the font has it, and vertical mark positioning (M8.20c) |
| Subset | `--pretty-svg`, `--xml`, `--OPTION?` and `--show-defaults` (M12.18c); no pyftsubset option is refused |
| Instances | `--layout-only` for `.glyphs` sources finds the compiled masters under the UFO names glyphsLib gives them, and `--interpolate-binary-layout [DIR]` interpolates `GPOS` from compiled masters as fontmake does (M12.22d) |
| Transform | the CFF matrix reads glyphs as rotatefont does (single-precision sums, end points in hundredths) and writes stems and hint groups as its writer does (X.fix55) |
| Window | check marks on all 40 menu switches (X.ui2) |

Measures from this batch:

| Measure | Result |
|---|---|
| Variable merge against fontTools' instancer on the merged halves (M12.23c) | the same at 4 locations for the Oswald halves, Oswald with itself, I.ttf and Amstelvar; NotoSans needs a name table on the fontTools side; InterVariable's `GPOS` differs only by 2 Extension lookups that fontTools' serializer adds when it saves the instanced half; `VARC` is dropped; fonts with different axes are refused. The static merge is unchanged (143 tables the same, 2 different) |
| CFF2 export against fontmake (M12.25c, M12.25d) | variable `otf` oracle: 106 equal, 32 different, 20 failed, unchanged by the CFF2 work (104, 34, 20 before the eleventh batch). On Oswald the subroutinised CFF2 table is 1.009 of fontmake's size; the charstrings have the same operators and differ in their numbers only because the default master differs (the allow-listed Oswald origin) |
| CFF version 1 and TrueType output | unchanged: CFF against fontmake 2,377 compared, 1,854 the same, 511 coverage-equal, 12 different; TrueType 2,376, 2,218, 139, 19 |
| TTX bitmap forms against ttx (M12.7f) | row form: 18 written and 36 read equal; bitwise: 15 and 30 (3 skipped, ttx fails on `CBDT`); external files: 18 dumps and 17 image folders written equal, 36 read equal; 0 different |
| `--interpolate-binary-layout` against fontmake (M12.22d) | `GPOS` identical on 3 of 3 sources (Oswald-AE-comb, 18 tables; WdthVar, 9; kerning_ltr_and_rtl, 3) |
| `EBDT` and `EBLC` writer (M13.55c) | bytes equal to fontTools' on the test strikes |
| rotatefont matrices (M12.24c, X.fix55) | 5 corpus CFF fonts, 6 matrices, 21,774 glyphs: 0 outline, 0 stem, 0 width and 0 Private DICT differences (before X.fix55: LinLibertine_RBI 615 outline and 490 stem differences, Markow 164 stem differences when scaled) |
| AFDKO measures of the eleventh batch, run now | Type 1, bare CFF and TTX input against tx (M12.30b): TTX input, 3,172 glyphs the same, 0 different, 3 fonts failed; makeotf project compile 7 of 10 corpus UFOs (M12.40d); CID keys 13 of 13; the subroutiniser against cffsubr 233 fonts, 0 outline differences, 0.785 of cffsubr's size |
| Speed (release, at a load of 19 to 20) | Text Preview: a million characters 290 to 325 ms (323 to 404 ms on shaped widths; limit 500 ms), a page 0.377 ms, a colour frame 0.419 ms; preview font build 276 to 383 ms, and 170 to 173 ms at a load of 8 (baseline 169 ms) |
| Round trips | `.glyphs` native 122/1/118/4 (unchanged) |

The gate passes all eleven steps with 4,501 tests passed, 0 failed, 49 skipped. Decisions: CJK exports subroutinise
by default, as the handbook documents (Disable Subroutines turns it off), now that the subroutiniser handles full CJK
fonts; the variable merge rules are ours, and a static merge keeps default values and drops variation stores where
fontTools refuses; CFF2 hints blend as varLib merges them, except that mismatched hint structures drop that glyph's
hints with a warning (fontTools fails the font); CFF2 exports write `maxstack 513` only when a charstring needs more
than 193 operands, as the specification requires, so such fonts differ from fontTools' output by that Top DICT entry;
bitmap-only fonts open at 1,000 units per em (a guess); `.otb` export writes a full-length `loca` and keeps every
glyph (guesses; FontForge's documentation differs); vertical Text Preview keeps `vpal` and `vhal` off (a guess);
every menu switch shows a check mark, though the handbook documents them only for Show Info and Show Metrics;
`--show-defaults` is ours (pyftsubset has none), and `--OPTION?` prints pyftsubset's line for the option; the
rotatefont port keeps its reader's single-precision sums, end points in hundredths, the stem swap for negative widths
and its hint regrouping, and its `-rt` half-angle quirk stays a written-down difference. Integration fixes: a
combining mark stays in the script run of the character before it; a `.glyphs` source without a family name finds
its compiled masters under glyphsLib's "Unnamed" font name; a merged name table with more records than its 16-bit
storage offsets can address is refused before writing (the table writer would panic). Review fixes (X.fix56 to
X.fix58): `--interpolate-binary-layout` compiles into a scratch folder and writes nothing on failure;
`bitmap.setStrikes` measures every strike against one budget per call; the parser decides whether an optional
option value is given; `SVG ` export bounds the fan-out of colour references with a paint visit limit and a running
table size; CFF2 glyphs whose stem blends overflow the stack are reported; strikes share a font-wide decode budget
and an image cache; static merges drop `GDEF` stores and variation devices; joined variation stores refuse mixed axis
counts and deduplicate regions by hash.

Left from this batch: see "What to build next".

**What is real: the thirteenth batch (10 follow-up packages, X.fix59, X.pdf1, X.var3, X.cff3, X.audit3, X.ui3,
X.glyphs3, X.integ19, X.integ20, X.fix60 to X.fix66, X.gate17).**
Integrated and gated; the package list is under "Shipped so far". Outputs are compared with the open tools named
below. The editor parts follow the handbook and are not compared with the reference app.

| Area | What works |
|---|---|
| Variable TrueType | `cvar` from each master's control values (zones and stems) as fontTools merges hinting, rounded half to even; masters whose control values do not line up give no `cvar` and a warning, where fontTools fails the font (M10.19b). Instruction deltas are not written. An alternate layer's own instructions export for its alternate glyph when the layer belongs to the main master (a guess) |
| CFF2 export | intermediate and alternate layers carry hints: from the main master's manual hints through node links, or from the autohinter run on all sources together, without flex (M12.25e). Each master's family blues go into the Private DICT, blended where the masters differ (X.fix59) |
| Instances | `compile --interpolate-binary-layout [DIR]` gives each instance the `GPOS` interpolated from the compiled masters, and a `.glyphs` source without a family name compiles to `Unnamedfont-*` files as fontmake names them (M12.22e). Instances from sources write fontmake's PostScript weight name, shared PANOSE digits, `fsType` and underline defaults for non-default sources, and a designspace with locations but no axes is refused (X.var3) |
| Feature variations | a designspace rule that feeds several features (CustomFeatures) goes under every tag of the feature list (X.var3) |
| Convert | UFO output from a binary; `--glyphs`, GOADB rename, subset and order; `--location` (a variable font converted to its own flavour keeps its variations); overlap removal for TrueType, CFF and CFF2; a default snapshot of variable fonts; CID keying (`--cid-keyed`) with `seac` glyphs drawn out; Font DICT selection and `/CID` selectors; the GOADB order for CFF2 (M12.30c, M12.30d) |
| TTX and dump | spot's table listings ported from AFDKO, the CFF literal view and `--exclude-glyphs` (M12.10b) |
| CFF | the instancer reads and writes CFF and CFF2 through `crates/cff`; its output bytes are unchanged (X.cff3). It accepts the long expansions, 513-value blend runs, real `vsindex` and `dotsection` its old reader took (X.fix64) |
| Audit | `tables.recompileCff` expands subroutines, draws `seac` glyphs out and drops `dotsection`; `audit.fix` runs it on compiled fonts. `audit.run` on the open document keeps the layer checks' results per glyph between runs, keyed on the resolved thresholds. `checkOutlines` takes all and quiet; `compareFamily` adds its reports (X.audit3, X.fix61) |
| Colour | the colour Text Preview draws `SVG ` filters (blur, offset, flood, colour matrix, composite, merge, blend, drop shadow) under the enclosing clips, and clips a sliced root element to the em square; a filter it cannot draw leaves the element unfiltered with a warning (M11.17d, X.fix60) |
| PDF | paste reads a PDF flavour; Import Outlines reads PDF files; a placed PDF decodes for autotrace (X.pdf1) |
| Bitmap formats | gzipped PCF and BDF, Windows FNT (versions 2 and 3 read, 3 written) and FON (read and written) (M13.55e); TeX PK (read and written) and GF (read), Mac `NFNT` in resource forks, `.dfont`, AppleSingle, AppleDouble and MacBinary files (read) (M13.55f) |
| Detect Composites | among bases with equal outlines it picks the Latin letter over a look-alike copy, then the glyph database's recipe, then name relations, suffix case, fewer extra suffixes, a combining mark and the shorter shift (M13.15c, X.fix66) |
| `.glyphs` files | stale kept spellings and layouts are left out on save (X.glyphs3); node flags read in any order with unknown letters kept (X.fix61); one-line layer guides reach the model (X.fix66) |
| Window | one undo step for an arrow-key nudge of several hints; Instructor badges dragged to another node, and Delete on instructions; a module installed from a dropped file; a Font View master per document; the metrics beam dragged; the Exports tab previews instances between masters (X.ui3) |

Measures from this batch:

| Measure | Result |
|---|---|
| Detect Composites on Inter (M13.15c, X.fix66) | 78.1% of 2,017 composites rebuilt from the source's bases (63.0% before; target 95%), 241 rebuilt from other bases, 200 missed; 0.4 to 0.7 s (target 2 s). The rest are merged attachments (ogonek, cedilla, stroke letters) and bases with equal outlines that no name or data tells apart (`uni1FBE` and `ypogegrammeni` 21, `macronsub` 19, `koronis` and `psili` 11, `tonos.top` 8, `uni1FC0` 8). The 95% target is out of reach without anchor or source hints |
| Variable CFF2 against fontmake's `variable-cff2` | 107 equal, 31 different, 20 failed of 158 (106, 32, 20 before) |
| CFF version 1 and TrueType output | unchanged: CFF against fontmake 2,377 compared, 1,854 the same, 511 coverage-equal, 12 different |
| AFDKO measures, run again | unchanged: TTX input against tx 3,172 glyphs the same, 0 different, 3 fonts failed; makeotf project compile 7 of 10; CID keys 13 of 13; rotatefont 21,774 glyphs, 0 differences; the subroutiniser against cffsubr 233 fonts, 0 outline differences, 0.785 of cffsubr's size; CFF2 export on Oswald 1.009 of fontmake's size |
| The instancer on `crates/cff` (X.cff3) | output bytes unchanged for every input the old and new readers both accept |
| Round trips | `.glyphs` native 122/1/118/4 (unchanged) |

The gate passes all eleven steps with 4,625 tests passed, 0 failed, 49 skipped. Decisions: a `.glyphs` source without
a family name compiles to `Unnamedfont-*` files, as fontmake names them, and other sources take the source file's
name; `.glyphs` node flags are read in any order and unknown letters are kept (`lXR` reads as locked and Right);
Detect Composites orders bases with equal outlines as listed above, with `sub` read as `below` and the case suffixes
`.uc`, `.case`, `.tonos`, `.lc`, `.sc`, `.c2sc` and `.smcp`; one-line layer guides, hints and annotations in `.glyphs`
files reach the model and keep their layout; mismatched masters give no `cvar` (fontTools fails); autohinted CFF2
glyphs have no flex; a CID-keyed convert draws `seac` glyphs out; a glyph whose CFF overlaps are removed loses its
stem hints; FNT glyph names, PK and GF pixel sizes and names, and the FON writer's layout are guesses; PK and GF
codes match glyphs by name only, as they are in TeX encodings. Review fixes (X.fix60 to X.fix66): `SVG ` filters
honour enclosing clips and fall back to unfiltered drawing; a placed PDF keeps its bottom edge; the audit cache keys
on the resolved thresholds; `cvar` warns on duplicate locations; the CFF2 instancer charges expansions against the
charstring memory budget; CID keying draws `seac` glyphs out and caps GOADB files and CID selectors; Mac strike
listings are bounded and deduplicated, and TeX codes stay out of Unicode; Detect Composites' Latin-copy rule does not
depend on glyph order and its suffix comparison is charged to the work limit.

**What is real: the fourteenth batch (M12.40c, M11.17e, X.glyphs4, X.audit4, X.ttx4, X.inst4, X.pdf2, X.integ21,
X.integ22, X.fix67 to X.fix72, X.gate18).**
Integrated and gated; the package list is under "Shipped so far". Outputs are compared with the open tools named
below. The editor parts follow the handbook and are not compared with the reference app.

| Area | What works |
|---|---|
| Project compile | components placed in single precision and one lookup per rule type, as makeotf does; compile messages keep the file and position of included feature code (M12.40c, X.fix67) |
| Colour | the remaining `SVG ` filter primitives but `feImage` (component transfer, convolution, morphology, displacement, tile, turbulence, diffuse and specular lighting) in the colour Text Preview and in Edit View, clipped to the canvas; the canvas says when it drops filters; Text Preview shows the raster warnings (M11.17e, X.fix71) |
| Audit | `checkOutlines` cleans up after a fix as checkoutlinesufo does (decimal coordinates, keepContourOrder by default, one undo step) and keeps smooth flags, node names and component offsets; `check family -l` writes a log; the CLI's list options repeat (X.audit4, X.fix69) |
| TTX reading | a CFF font read from TTX compiles its own CFF again and recalculates the `head`, `hhea` and `vhea` bounds as ttx does; a CFF table whose strings XML text cannot carry keeps its bytes, with a warning (X.ttx4, X.fix69) |
| Instances | a designspace without axes compiles: masters build, instances come from one source only, and a stated location is refused; style-map styles outside the four RIBBI names (Thin) are kept; file names expand the name tokens (X.inst4, X.fix70, X.fix72) |
| PDF | Edit View pastes the system clipboard's PDF, read off the UI thread through the platform's clipboard tools; one parsed PDF is shared by drawing and measuring; placed PDFs draw into new bitmap strikes of glyphs without outlines (X.pdf2, X.fix68) |
| `.glyphs` files | one-line layouts of node attribute dictionaries are kept; version 2 top-level settings and names in version 3 and 4 files are kept, and stale ones dropped (X.glyphs4) |

Measures from this batch:

| Measure | Result |
|---|---|
| Project compile against makeotf (M12.40c) | 7 of 10 corpus UFOs equal; the other 3 differ only in written-down makeotf faults: two negative advance widths that makeotf wraps (to 65503 and 65458) and a move that makeotf loses in `threequarters` |
| TTX read bounds against fontTools (X.ttx4) | 253 of 253 corpus CFF fonts equal, with the default tags and with `head,hhea,hmtx` |
| Instance font info against fontmake (X.inst4) | 0 instances with a font-info difference |
| AFDKO measures, run again | unchanged: TTX input against tx 3,172 glyphs the same, 0 different, 3 fonts failed; CID keys 13 of 13; rotatefont 21,774 glyphs, 0 differences; the subroutiniser against cffsubr 233 fonts, 0.785 of cffsubr's size; variable CFF2 against fontmake 107, 31, 20; CFF2 export on Oswald 1.009 of fontmake's size |
| Oracle and round trips | unchanged: CFF against fontmake 2,377 compared, 1,854 the same, 511 coverage-equal, 12 different; `.glyphs` native 122/1/118/4; designspace 59/0/0/53; UFO 125/0/2/13 |

The gate passes all eleven steps with 4,682 tests passed, 0 failed, 49 skipped. Decisions: instance UFOs keep a
style-map style outside the four RIBBI names in the lib (`com.fontcraft.styleMapStyleName`), not in `fontinfo.plist`,
because ufoLib with validation and fontc refuse it there (fontmake writes it to `fontinfo.plist`); UFO style-map
styles follow ufo2ft (exact comparison, name ID 2 title-cased, no REGULAR fallback); a designspace without axes writes
no `<location>` and always writes `<axis-subsets>` for variable-font instances; `checkOutlines` clean-up keeps smooth
flags, node names and component offsets, which the reference tool loses or rounds; clipboard PDF paste uses the
platform's clipboard tools (wl-paste, xclip, osascript, PowerShell) until native clipboard crates replace them
(X.pdf3); `SVG ` filters in Edit View draw clipped to the canvas, and the turbulence code ported from the W3C
specification is credited in `LICENSE-W3C.txt`; project-compile feature code has one lookup per rule type, as makeotf
does. Review fixes (X.fix67 to X.fix72): compile messages map through the include table of the files, not fea-rs'
debug output, and shared component trees are charged to the work limit; clipboard reads run on a worker thread with
one paste per key stroke, bounded waits and a SHA-256 cache key; contour matching in `checkOutlines` is linear; a font
saved as a designspace from another format compiles; off-screen filtered glyphs are not rasterised again on every
frame.

Left from this batch: see "What to build next".

**What is real: the fifteenth batch, depth wave 1 (M14.3, M1.8, M9.27, M13.78, M3.19, M7.21, M4.17, M3.40, M14.1,
M3.21, X.integ23, X.integ24, X.fix73 to X.fix77, X.gate19).**
Integrated and gated; the package list is under "Shipped so far". The editor parts follow the handbook and are not
compared with the reference app. macOS was not compiled for this batch (it needs the platform SDK), so the native
quit path is untested.

| Area | What works |
|---|---|
| Closing and quitting | the close box, Quit (Cmd+Q, the app menu's Quit on the Mac) and File › Close ask about each document with unsaved changes, fonts first, then projects: Save, Don't Save or Cancel; a failed or cancelled save stops the quit; a document edited while the question is open is asked about again; `app.quit` (refused headless with unsaved work unless `discard`) and `window.closePrompt`; the title names the active document and shows Edited; the suite's `craft-quit` handles the platform's quit (M14.3, X.fix76) |
| Opening files | File › Open… shows a chooser of every extension the readers take, and several chosen files each open; Open Folder… for `.ufo`, `.glyphspackage` and `.sfdir`; files dropped on the window open (at most 64 at a time); `.fontcraftproject` opens through `project.open` (M1.8) |
| Settings window | Edit › Settings… (Cmd+Comma) opens a window with seven sections; User Settings and Extensions are built, the other five are greyed until their packages; six User Settings switches, off by default (M14.1) |
| Font Info | the Font tab adds, reorders by drag, names (with the predefined axes), tags, hides and removes axes; the Masters and Exports tabs take coordinates in number fields through `font.setAxisLocation`; the Exports tab warns of extrapolation through `font.instance.placement`; axes that share a tag are edited by index (M9.27, X.fix74). Value lists and syntax checks for 64 custom parameters (10 before), warnings for a master or palette the font lacks, and pasted rows skip a refused value (M7.21, X.fix74) |
| Font View sidebar | filters renamed in place (double-click, or Return when the sidebar has the keyboard), dragged to reorder and into folders; Alt on the rule editor's plus adds a group; a new `Groups.plist` entry shows at once (M4.17, X.fix75) |
| Layers section | a row dragged within its master reorders the layers, with an insertion line; a drop under another master is refused; a backup dropped on the master row becomes the master; a master-row click with text selected switches every selected glyph and the edited glyph (M3.40, X.fix75) |
| Select tool | Alt on a handle moves points only; Ctrl+Alt on a smooth node's handle mirrors the opposite handle; Alt-drag on a curve segment runs `path.reshapeSegment`, which solves both handle lengths so the point under the pointer follows it (M3.19, X.fix73) |
| Measurement tool | the ruler and the coordinates cover every glyph of the line (at most 200; more are left out, not refused); overlapped stretches of outline are drawn grey; Cmd in the Measurement tool holds the Select tool, and Cmd+Alt+Ctrl on the Mac or Ctrl+Alt+Shift elsewhere holds the Measurement tool; an edit measures only the changed glyphs again (M3.21, X.fix73) |
| Control channel | `ui.click`, `ui.drag`, `ui.move` and `ui.wheel` at window points, `ui.set` over the view switches, `ui.resize`, `ui.focus`, `ui.contextMenu.list` and `ui.contextMenu.invoke` (27 methods); synthetic input is applied one event per drawn frame, and the person's own input or the save question drops what waits; `ui.resize` keeps to the monitor and the GPU's texture size (M13.78, X.fix77) |

Measures from this batch:

| Measure | Result |
|---|---|
| Measurement over a heavy line | 200 large glyphs are measured up to the element limit and the rest left out, not refused; an edit to one glyph measures that glyph again only |
| Parameter value checks | 64 parameters checked; every corpus value passes but three in fontc's test sources (an Elidable STAT Axis Value Name of `Regular`, a `gasp` range of 65536, a PostScript font name with a space) |
| Round trips | unchanged: `.glyphs` native 122/1/118/4. No compile output changed, so the oracles were not run again |

The gate passes all eleven steps with 4,767 tests passed, 0 failed, 49 skipped. Decisions: the close and quit question
asks per document, fonts first, and `window.closePrompt save` from automation on a document without a file returns an
error instead of opening a chooser; while the question is open only Return and Escape reach it; off the Mac the
Measurement tool is held with Ctrl+Alt+Shift, because Ctrl+Alt is AltGr there and the Select tool's mirror chord; a
text-mode master-row click switches the selected glyphs and the edited glyph; axes are addressed by index where tags
are shared, and a shared tag in a command is refused as ambiguous; Settings… is in the Edit menu on every platform;
control-channel input waits for the frame that draws it. Review fixes (X.fix73 to X.fix77): long measurement lines
are truncated instead of failing, and results are kept per revision; the save question takes the keyboard; sidebar
Return and drops act only where the sidebar has them; panel copies of the extrapolation logic are replaced by an
engine query; context-menu items are checked again on invoke. Integration (X.integ23, X.integ24) also fixed
measurement corners reported as overlaps and Edit View keys that reached the canvas behind the save question.

Left from this batch: see "What to build next".

**What is real: the sixteenth batch, depth waves 2 to 11 in part and the craft-next window (47 depth packages,
M10.37b, M10.37c, M13.65b, ES.1 to ES.3, UI.1 to UI.29 with UI.4b, UI.9b, UI.14b, UI.15b and UI.21b, X.fix78 to
X.fix87, X.integ26 to X.integ80, X.inte1).**
Merged on `master`; the package list is under "Shipped so far". The writers ran no builds, and most branches were
merged without one; a workspace check and clippy at the end are clean. The first full test run (5,304 tests) had 33
failures: X.fix86 fixed the 24 outside the window crates and X.fix87 the 9 in `crates/ui` and the apps. A review
found 7 more (X.fix88, X.fix89) and the wasm check one (X.fix90); the gate is green. None of the oracle measures the packages name has run (see "What to build next").

| Area | What works |
|---|---|
| Compiling | `compile` takes output formats, the output layout, file naming, instance patterns, variable fonts and delta optimisation (M2.8); feature files, include folders, MTI sources, feature writers, `GDEF` generation, a debug file, a stub `GSUB` and a names override, with the plan's `features` section (M2.12); outline, CFF, glyph-set and variable options: `useMyMetrics`, glyph closure, `.notdef`, production names, version bump, `post` format 3, excluded and raw tables (M2.13); MakeOTF Compatibility Mode gives an export the project compile's lookup grouping (M12.80). A source with several masters and an axis compiles to a variable TrueType font unless a static format is asked for, as fontc does (fontmake builds statics); the owner has to confirm this default |
| Command-line tools | common options on every subcommand: message levels, log file, colour, timing, jobs, version, backtrace; `run --script` and `--params-file`; `commands --cli`; `help --all` (M12.67); option prefixes in every parser (X.fix80, X.fix86); `info`, `glyphs list` and `tables list` print text and take several inputs (M12.68); subset by the converter's glyph selection and over several inputs (M12.73), and subset of sources in their own format with kerning, groups, classes and feature code pruned (M12.74); mergefonts parity: CID-keyed results, map headers, a hint font, Font DICT selection (M12.69); convert to SVG fonts and UFOZ, rebuilt tables and tx's CFF-writing switches (M12.75, M12.70); dump's Unicode names, virtual glyph ids, `avar` as a designspace and glyph selections by Font DICT (M12.76); outline repairs on CFF and static CFF2 fonts (M12.66); `cargo xtask cli-options` measures option coverage against the reference tools (M12.77) |
| PDF proofs | proof sheets as one PDF with waterfall text as kerned outlines (M12.36); the proof tool's layouts (palette and outline, boxes, hints, digi, compare) with 46 drawing settings and panels (M12.78); the spot sheets synopsis, features, cmap, vorg and text, with show and hide per element (M12.79); Export Window Contents writes a tab's text as PDF or SVG (M13.68) |
| Hinting | autohint of compiled CFF fonts (M10.37) and of CFF2 variable fonts, the masters of each region list hinted together (M10.37b); overlapping glyphs of a compiled CFF font hinted on their overlap-free outline (M10.37c); the autohinter's fontinfo, overlap, allow-changes, keep-conflicts and decimal options (M10.36); hint upkeep on background layers and corner components (M10.40); commands that rebuild shapes renumber hints by path (M3.36, X.fix78, X.fix79, X.fix85); extra nodes at intersections selected in the Instructor (M10.35) |
| Interpolation | higher-order interpolation curves in static instances (M9.29) and the six HOI commands (M9.30); Match Masters and Relocate Start Points (M9.34); Set Master Locations, Adjust by Variation, Add Variation and Merge Fonts to Masters (M9.36); Overlay Fonts as palette layers or one master per source (M13.66) |
| Features | the Indic positional features pref, vatu, pres, abvs, blws and psts from anchors, with Indic script tags in the automatic languagesystems (M8.32, X.fix81); lookup debug info (`Debg`) in feaLib's form (M8.35); feature entries disabled, copied and pasted (M8.29); a free name for a new number value (M8.28) |
| Colour and bitmaps | `CPAL` version 1 with palette types and labels, and a dark palette derived in OKLab (M13.65); version 4 palettes written as lists with a loss report in `.glyphs` 2 and 3 and UFO, and `CPAL` version 1 read back (M13.65b); glyph artwork with shadows, inner shadows and pattern images, and Filter › Glyph as Image (M11.21); bitmap glyph commands (M13.63) |
| Sources and editing | nice names on import and production names in UFO export (M1.9; the CLI keeps the source's names, as fontmake does); UFOZ save, and format warnings from the save loss report on the glyphs an edit touches (M1.11, X.fix82); path attributes in UFO glyph libs (M3.36); Round Coordinates on the selection and the Tidy rules (M3.29); edit guards for locked and aligned components (M6.21); the file keys of caps, segments and brushes from written sources (M6.25, no code); vertical origin and width setters (M5.20); shared metrics of several glyphs (M5.22); a kerning audit of class-pair collisions and cross-master matching (M13.21b); Font and Exports tab warnings from one engine function each (M7.23) |
| Automation | MCP window tools (screenshot, render, pointer, click, key, text, menus, dialogs) when MCP is attached to a running window (M13.80); External Preview in a browser with trusted devices (M13.71); the clipboard's PDF on Windows and macOS (X.pdf3, built on neither) |
| Engine split | `crates/engine-core` holds errors, the guard, the clock, parameters and settings, and the command registry installs the command tables once per process against a golden list of ids (ES.1); helpers decoupled inside the engine (ES.2); typed session slots and document extensions, with interpolation, spacing, features and audit state behind extension traits and a `fontcraft_engine::prelude` (ES.3) |
| Second window | `apps/fontcraft-next` on `crates/ui`, built on the suite's craft-next framework (pinned by git commit; egui 0.37 from the suite's fork, beside the egui window's 0.36). Ported: the canvas with tools, selection, text mode, overlays, hinting and colour glyphs (UI.2 to UI.5, UI.4b, UI.27, UI.28); the control channel (UI.6); the dialog framework and the form dialogs (UI.7, UI.24); Font View with its sidebar, inspector and filter editors (UI.8, UI.9, UI.9b); Font Info with every tab (UI.10, UI.22, UI.23); the Palette panels (UI.11); Layers (UI.12); glyph sections, kerning review and Glyph Info (UI.13); the Kerning window, pairs preview and groups editor (UI.14, UI.14b); Space Center and Preview (UI.15, UI.15b); Text Preview (UI.16); Font Audit (UI.17); Table Viewer and Compare (UI.18); Start window, open, close and Settings (UI.19); the Project panel, extension menus and external changes (UI.20); console, command palette, About and Actions (UI.21, UI.21b); context menus and the clipboard (UI.25); component Info boxes and smart settings (UI.26); TrueType dialogs and the Hinting Preview (UI.29). Each port has headless tests; none has been tried in the running window |

Measures from this batch:

| Measure | Result |
|---|---|
| CLI option coverage (`cargo xtask cli-options`, M12.77) | 794 mapping rows: 672 present, 98 missing, 24 unchecked; 87.3% |
| Oracle measures the packages name | not run: `COLR` over the corpus with ttx (M13.65), fontTools decoding the autohinted charstrings (M10.37), `GPOS` and `GDEF` against fontmake (M2.12), subset of sources against the compiled subset (M12.74), and the others listed under "What to build next" |
| Round trips | not run again |

Decisions: a multi-master source compiles to a variable font by default (M2.8, confirmed by the owner); the CLI's
`open` keeps the source's glyph names; overlapping multi-master glyphs are autohinted on their overlap-free outline by
default, as otfautohint does (M10.36); every CFF the converter writes takes tx's defaults (M12.70); source includes
apply only when no feature folder is given (M2.12); the second window's control server keeps FontCraft's own transport
without a token, so MCP attach is unchanged (the owner decides on the suite's token-authenticated protocol); the Font
Audit panel runs one audit step per frame; in the second window, Return in a dialog field confirms the dialog.

Left from this batch: see "What to build next".

**What is not.** Nothing in the editor has been compared with the running reference app. M3's,
M4's and M5's behaviour comes from the handbook; 5 editor rows are confirmed by public tutorial videos. Where
a detail is unconfirmed, the handbook decides (Remove Overlap, for example, deletes open paths).
Where no source says, we choose, and each choice is listed in `docs/guesses.md` to revisit
(3,081 guesses on 1,606 rows).

| Gap | Detail |
|---|---|
| `path.sharpenCorner` | moved to the `fidelity` milestone: no written source says what it does, and the tutorial video shows the menu item without running it |
| Text mode | Edit View and Text Preview shape text through a preview font built by the same compile as `features.compile`; Text Preview orders mixed-direction runs by UAX #9, breaks lines on shaped widths and sets vertical text with `vkrn` and `vrt2`; Edit View keeps one direction per line |
| Spacing speed | on Inter, the out-of-sync update after an edit to one glyph takes 7.4 ms against a 2 ms target, and the full scan 53.5 ms against 50 ms; the per-edit alignment pass takes 0.24 ms against 2 ms |
| Kerning | vertical kerning is stored and listed, and not exported, as neither reference compiler writes `vkrn`; vertical origin and vertical width have no commands; 12 metrics-key values in Inter Italic differ from the stored ones by up to 1.4 units, cause not settled |
| Masters | master switcher icons come from M9.20; showing all masters in one tab (`edit.showAllMasters`) needs a tab model with a layer per text item |
| Font View | each automatic composite copies the glyph list, so adding very many composites at once is slow |
| Export dialog | OTF, UFO, Metrics and Variable Fonts tabs; variable fonts export with TrueType or CFF2 outlines through the dialog, `file.exportVariable` and `fontcraft-cli compile --variable` |
| Import Outlines | the missing-glyph question is asked once for all unmatched files, not once per file |
| TrueType instructions | the 14 hint commands edit stored instructions, and static exports and the default master of variable exports carry them as glyph programs (M10.19); variable fonts carry `cvar` (M10.19b) and, as the handbook says the reference does, no instruction deltas; the autohinter (M10.21, M10.21b) agrees with ttfautohint on 92.9% of x-height rows (target 95%) and 100% of cap-height rows, and 98.29% of zone extremes land on their rounded pixel row (target 99%), but that measure covers 7 of 325 sources (3 distinct fonts, no Inter); the handbook's rounding-threshold example contradicts its default, and FontCraft follows the default |
| Hints | Shape Order renumbers hints that refer to a path by index (`glyph.setShapeOrder`; M12.62 removed the duplicate `path.setShapeOrder`); hint upkeep (M10.3) does not renumber background layers or corner and cap hints |
| Strokes in files | a UFO keeps no stroke attribute or gradient (a save reports them); cap code 4 and the forced cap axis are not confirmed against the reference's files |
| Detect Composites | on Inter it finishes in 0.4 to 0.7 s and rebuilds 78.1% of the composites from the source's bases (target 95%); 241 take other bases with equal outlines that no name or glyph data tells apart, and 200 are merged attachments (ogonek, cedilla, stroke letters). The target is out of reach without anchor or source hints |
| Audit | 34 of the 36 outline checks and the M12 checks (157 and the width, glyph-set, table, layout and CID groups of this batch) run through `audit.run` and `fontcraft-cli check`, with check modules and `summary.errors`; the outline checks give many warnings on decoded compiled fonts (not yet examined); the layer checks keep their results per glyph between runs on the open document; the reference tools' options are only partly taken; verdicts compared with FontBakery's on 44 pairs only, and the FontBakery corpus run has not been made |
| Colour previews | the colour Text Preview draws `COLR`, `SVG ` (compressed too), `sbix` and `CBDT` from the compiled font, with `SVG ` clip paths, masks, patterns, the clip of a sliced root element and every filter primitive but `feImage`; Edit View draws filters too |
| Subset | no subsetting of sources |

Variable TrueType exports have `cvar`; the tools and
panels of M6's later UI packages are not built; there is no web app and no
packaging. Glyphs-only data travels in
glyphsLib's UFO lib keys where glyphsLib has a key for it, and alternate and intermediate
layers travel as designspace rules and sparse sources; the data no key holds (glyph tags,
locks, sort names, layer colours, drawing attributes…) does not, and the loss report lists it.
Instances between masters are read and written, not yet compiled.

**How much to trust the inventory.** It was compiled from public documentation in one day, each
source read once. Of its 3,786 rows, 2,130 come from the primary reference's handbook, 1,075 from
other public documentation or permissive source, 513 are our own design, 6 rest on
recollection, and 62 are observed: 57 compiled-output rows checked against fontc, fontmake and makeotf,
and 5 editor rows confirmed by public tutorial videos. **No other row has been checked against
the running reference app.** An independent re-reading of 209 sampled handbook rows against the
handbook found 1 wrong and 14 imprecise (`docs/spec/README.md`). The notes list about 450 open
questions the documentation does not answer. Treat rows as a careful first reading, and each
milestone's rows as due a planning pass before they are built (`docs/execution-plan.md` § 3).

### By dimension

| Dimension | Estimate | Evidence and what is missing |
|---|---|---|
| **Breadth:** commands, tools and panels exist | ~43% | 960 of 1039 commands; 20 of 33 tool rows usable in the window (19 tools: every M3 tool row, the Annotation row being the group of its six tools, and M5's Text tool; the Thickness tool runs by `tool.select` and has no toolbar button); about 87 of 172 panels, many of them partial (Font View's grid, List View, sidebar and inspector, the toolbar, the Info box with its spacing and kerning fields, the Palette and five of its sections, the bottom bar, the Glyph Info, Selection Info, Start, Kerning and Text Preview windows, the Preview Area and Panel, the Font Info window with its tabs, the Command Console, the Related Glyphs, Selections and Reference Font sections, the Pairs and Phrases section, the Kerning Pairs Preview, the Groups Editor, the Space Center, the TrueType Debugger, the Table Viewer, the M3 to M5 dialogs and the M13 dialogs); the second window ports most of these and is not counted until it replaces the egui window |
| **Depth:** features behave like the reference | ~12% | the M3, M4 and M5 commands, tools, panels and dialogs behave as the handbook describes them, and none has been compared with the reference app. The kerning lookup equals what fontc and fontmake compile. The variable export matches fontc's and fontmake's at the masters and between them for most corpus sources (outlines in 127 of the 135 sources compared, kerning and marks in all but a few), and static instances between masters match fontmake's in 180 of 234 compared. The CFF2 variable build equals fontmake's `variable-cff2` in 107 of 158 sources. Variable `COLR` equals fontmake's on the three variable colour sources. The first waves of M10 to M12 match their oracles where tested (the autohinter AFDKO's scripts, the table tools small oracle outputs), but most of their work is not yet in exports or the window. The Font Info window and the M13 packages follow the handbook and the secondary references' public documentation, with 2,641 guesses listed. `path.sharpenCorner` waits in `fidelity`; the gaps are listed above |
| **File interop** | ~40% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFO 1 and UFOZ open; EPS outlines and, in `crates/io-sfd`, SFD sources can be read (SFD is not yet reachable from File › Open); compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts), with their layout tables rebuilt as feature code (302 corpus binaries export again); variable TrueType and CFF2 fonts open into axes, masters and instances; Type 1 (PFA, PFB), bare CFF and SVG fonts open, and SVG fonts are written; BDF, PCF (plain or gzipped), `.otb`, Windows FNT and FON, TeX PK and GF and Mac bitmap fonts open, with the strikes of compiled fonts, and FNT, FON and PK are written; PDF outlines paste and import; static TTF and OTF out, plain, WOFF and WOFF2, from the export commands, the Export dialog and the CLI. Variable TrueType fonts and instances between masters are written. Paint attributes are typed in `.glyphs` 2 to 4; writing version 3 and UFO reports what they cannot hold. Missing: CFF2 variable fonts, version conversion, stroke attributes and gradients in UFO |
| **Compiler correctness** | measured, low | the core tables, `GPOS` (generated pair kerning and marks), `GDEF`, `gasp` and `SVG ` written for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers; variable TrueType fonts with `fvar`, `avar`, `HVAR`, `MVAR`, `STAT`, `gvar`, feature variations, variable kerning and mark positions (`VVAR` is built but not exported); strokes, masks and overlap removal in the outline stage. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). With overlaps removed, 99.58% (TrueType) and 99.87% (CFF) of glyphs have no overlap-removal difference from fontmake's (`docs/plans/M3.md` § 5). The generated pair kerning equals both oracles' on every corpus source without `kern` feature code or a top-level include, but 27 known differences listed in `docs/spec/40-compile.md` § 4a (`docs/plans/M5.md` § 5). Layout tables are compared lookup by lookup (`crates/oracle`, `docs/plans/M8.md`): against fontmake, `GSUB` 25 of 26 sources equal, `GPOS` 82 of 88 and `GDEF` 47 of 55; against fontc, `GSUB` 29 of 46, `GPOS` 80 of 91 and `GDEF` 42 of 68. The remaining differences are bracket glyphs and colour glyphs (M11), each listed in `docs/spec/40-compile.md`. Variable fonts are compared with the same oracles (`fontcraft-cli oracle --variable`): against fontc 103 of 158 sources are equal, 35 differ and 20 fail; against fontmake 106 equal, 36 differ and 16 fail; per check (counted before the eleventh batch), outlines 127 and 8 (fontc) and 128 and 11 (fontmake), `GSUB` variations 129 and 8 and 135 and 6, `GPOS` kerning 135 and 1 and 141 and 0, marks 134 and 2 and 139 and 2. fontc and fontmake agree with each other on 112 of 133 sources. Static instances between masters equal fontmake's `-i` build in 180 of 234 compared (30 differ, 24 fail in fontmake itself). The causes still open are listed under What is real: variable fonts and instances The name table and OS/2 of static instances equal fontmake's in 1,678 compared values and differ in 120, from three allow-listed causes (`docs/spec/40-compile.md`). The automatic feature code regenerated at export equals the reference compilers' in 69 of 95 compared entries; 23 differ (stale sources, older reference forms, sample fonts) and 3 are removed. CFF2 variable fonts (`--variable --format otf`) are compared with fontmake's `variable-cff2`: 107 equal, 31 different and 20 failed of 158 sources; every outline and advance difference also shows in the TrueType build against fontmake, and fontTools reads all 153 outputs without warnings. CFF2 exports are specialised, carry blended hints (for intermediate and alternate layers too) and the masters' family blues, and are subroutinised when that makes them smaller (1.009 of fontmake's size on Oswald); static CFF exports carry subroutines when that makes them smaller. An AFDKO project compile gives a dump equal to makeotf's on 7 of 10 corpus UFOs (the other 3 differ only in written-down makeotf faults), and CID-keyed CFF equals makeotf's on 13 of 13 keys. Implied on-curve modes equal fontc's and fontmake's in 484 of 484 glyphs. Feature-code table blocks and `usMaxContext` equal the oracles' on the corpus, and contextual anchors equal fontmake's on the probe source. TrueType exports carry the glyph programs of manual instructions (M10.19: 0 interpreter errors over every exported glyph of the instructed corpus samples at 8 to 72 PPM) and of the autohinter, and variable TrueType fonts carry `cvar` |
| **Agent automation** | ~62% | every existing command is reachable from CLI, control channel and MCP; MCP has permissions, five workflow prompts, resources, completion and logging, and MCP and the control channel can be confined to read and write folders; the control channel's 27 methods drive the window's tools, pointer and wheel at any window point, keys, text fields, dialogs, menus, context menus and view switches, and resize and focus the window; `ui.screenshot` and `ui.render` show the result; an agent can draw, edit and export glyphs, build a font from glyph sets, and type, space and kern text, over the control channel alone (three end-to-end tests). MCP also runs over streamable HTTP (`--transport http --listen`). MCP's window tools drive a running window when MCP is attached to it (M13.80). Missing: an element list, waiting for idle and an event log (M13.79, M13.81) |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation, corpus runs of overlap removal and the audit); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and the path-operation commands and export run each glyph on a worker thread under a time limit (a glyph that runs out of time stays as it was; the worker cannot be stopped and runs on detached). Adversarial code reviews of the skeleton and of each milestone's first wave, every blocking finding fixed with a test and the rest listed in `docs/status.md`. Later fix passes bounded the work of the EPS, SFD, UFO 1 and PFM readers, features previews, macros, outline matching, intersections and stem filters, and made multi-profile exports all-or-nothing. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | a few measures | overlap removal over the corpus: median 38 µs, 99th percentile 403 µs per glyph layer; one node-drag preview step on Oswald: 142 µs; on Inter (3,017 glyphs) a Font View frame under 1 ms, a frame after an edit 8–10 ms (15–19 ms with every core busy), every glyph's effective properties 21–26 ms, a three-rule smart filter 28–35 ms, the first glyph-database lookup 24 ms; the kerning lookup over 490,911 pairs 39 ms, Update Metrics on all glyphs and masters 85 ms, a text-mode frame with 200 glyphs 5 ms, Text Preview layout of 1,000,000 characters 286 ms, 319 ms on shaped widths (limit 500 ms), bidi over 100,000 characters 7.5 ms and a shaped 30-line page 0.38 ms; the out-of-sync update after one edit 7.4 ms, over its 2 ms target; the colour-layer update after a one-glyph edit 35.5 µs; console completion at worst 2.28 ms (target 5 ms, met); the per-edit alignment pass 0.24 ms (target 2 ms, met); Detect Composites on Inter 0.4 to 0.7 s (target 2 s, met); an Edit View text-mode frame with 200 composites 5.9 ms (target 16 ms, met); a Space Center frame with 200 glyphs 1.30 ms, or 1.44 ms with every display option, and a colour-glyph frame in Edit View 0.53 ms (release); Font View with 691 image cells a steady 1.08 ms and a colour Text Preview 0.39 ms; InterVariable opens with its feature code rebuilt in 135 ms. No budget suite for the targets in `docs/architecture.md` |
| **Platforms** | Linux and macOS run; Windows untested | built and run on Linux (virtual display) and on macOS 15 on Apple Silicon (both windows build and open; a font opens). Not yet checked on the Mac: the quit prompt with unsaved work, the Open chooser and drop, and X.pdf3's pasteboard PDF. Windows has not been built. wasm: 42 crates build for the browser, the second window's crate among them; no web app |
| **Look and feel** | barely started | a functional shell with the siblings' default dark tokens; the toolbar, panels, dialogs and canvas overlays take their colours and sizes from the tokens, with drawn icons for the tools, the Transformations buttons and the Stroke section; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

1. **Finish the sixteenth batch.** Merged, reviewed, gate green and pushed. Left: the oracle
   measures the packages name: `COLR` over the corpus colour sources compared with ttx (M13.65); the autohinted
   charstrings decoded by fontTools (M10.37); `GPOS` and `GDEF` against `fontmake --feature-writer KernFeatureWriter
   --no-generate-GDEF` on three sources (M2.12); subset of sources against the compiled subset on 10 corpus sources
   (M12.74); the PDF proof's page counts against AFDKO's on Oswald (M12.78) and 20 corpus PDF proofs read back
   (M12.36); tx against mergefonts (M12.69); Match Masters on the corpus, target 95% (M9.34, an ignored test); a drag
   timed on a large colour UFO for the format warnings (M1.11, X.fix82). Checks on the Mac (it builds and runs): wave 1's quit
   path, chooser and drop, and X.pdf3's pasteboard; on Windows (not built yet), X.pdf3's clipboard.
2. **The second window to parity, then retire the egui window.** Landed since the sixteenth batch: the Shape Order window
   and drag reordering in order fields (UI.24b); colour drawn through `render::color::artwork`, iColor and svg
   layers, colour in the text and Font View cells, a colour picker and a gradient stop bar (UI.27b); Font and
   Exports tab warnings, the Number Values editor and the Features sidebar menu (UI.30a); multi-glyph Info box
   fields, `.ufoz` saving with format warnings in the status line, the Glyph as Image and Overlay Fonts dialogs
   and result notices (UI.30b); the current master and shown layer in Font View, menus, paste and panels, a
   person's copy only reaching the system clipboard, and the instruction badge menu (UI.31); Info box kern fields
   and locks, the Modules panel and View › Audit › Live Audit (UI.32). All built and tested once together. The
   export dialog gets no feature-compiler section: the reference's export dialog has none, and the options stay
   on `font.compile`. Then: Space Center drags and arrow nudges, colour in Text Preview, the Table Viewer's file
   pickers, the TrueType preview address dialog and user settings kept between runs (UI.33); the Extension
   Manager, Project panel copy and paste (also Cmd+C and Cmd+V), custom parameters added to project entries and
   reporter drawing (UI.20b); the first framework pieces in `crates/craft-ext`: struck list rows, edit commands
   reaching a focused list, a mixed mark on menu entries (UI.34); Text Preview colour from the compiled colour
   font of the shown master or instance, the Compare Fonts in List panel, reporter switches kept between runs in
   the user's settings and the Instructor section of the canvas menu (UI.35); the Extension Manager's Indexes tab,
   shared with Settings › Extensions, its HTTPS downloads of indexes, entry files and screenshots, Website and
   Donate (UI.37). The ports left:
   - Smaller follow-ups: open-path thumbnails in Shape Order; svg filters and PDF images in colour layers; the image
     Info box with Reveal Image File (`image.revealFile` through `FontCraftApp::opener`).
   - Framework gaps, built in `crates/craft-ext` in craft-libs' layout so they can move there later (decision
     0009). Built (UI.34, UI.36, UI.37a, UI.38, UI.39): a strikethrough list row, edit commands reaching a focused list,
     a mixed mark on menu entries, a danger tone on menu entries, modifiers on context-menu requests, open URL and
     reveal in the file manager, a text-field highlight hook (feature code colours and error underlines), caret and
     selection access, Up/Down stepping on text fields that hold a number, an async completer (feature code
     completion), union, intersect and slant icons, and alternate menu rows with their own label and shortcut. Taken
     from craft itself (UI.39): the Stroke palette's click-and-hold Cap Axis menu (a menu opened by `Trigger::Hold`)
     and paste outside a text field (`craft::platform::clipboard`, which reads the system clipboard on the desktop;
     the browser build still waits for a paste event that craft drops). The craft pin stays at `4bd0107`.
3. **The engine split, step 4 on.** ES.4 moves the kerning-index and metrics-sync readers outside the spacing crate
   behind its extension and moves the slots with `Session`; the later steps move command areas out of the engine
   crate along command ownership.
4. **The rest of the depth waves** ([`docs/plans/depth.md`](docs/plans/depth.md)). Writers left: wave 2 M7.22, M3.23,
   M5.29, M4.18, M3.22 (their editor parts are in the second window only); wave 3 M14.2, M8.26, M8.30, M6.20, M4.19;
   wave 4 M3.39, M8.27, M3.24, M3.20, M4.21, M13.81; wave 5 M13.41, M9.32; wave 6 M14.4, M14.5, M14.6, M3.41, M3.43,
   M5.30, M8.31, M13.42; wave 7 M3.42, M3.33, M10.34; wave 9 M13.64, M13.69, M13.74, M13.76; wave 10 M13.70, M13.23c,
   M12.84, M13.62; wave 11 M14.11, M4.24, M10.41, M4.20, M3.26; waves 12 and 13. No builder of waves 2 to 13 is built:
   the builder queue starts again at M8.25. The close builders (M6.19, M7.20, M8.24, M9.26, M10.32, M11.20, M12.65,
   M13.61) come last.
5. **The measures that missed** are depth packages: M10.21c (the TrueType autohinter's x-height rows, 92.9%
   against 95%, and zone agreement, 98.29% against 99%, measured again over more than the 7 sources the measure
   covers today) and M13.20b (auto-spacing on Inter, 30% to 75% median error against 10%, with a model of shape
   classes; measure: `cargo test --release -p fontcraft-engine --test engine corpus_autospace:: -- --ignored
   --nocapture`). Detect Composites on Inter stays at 78.1% against 95% (M13.15c): the 200 misses are merged
   attachments (ogonek, cedilla, stroke letters), and 241 composites take other bases whose outlines equal the
   source's bases and that no name or glyph data tells apart. A further step needs anchor or source hints; no
   package is queued. Measure: `cargo test --release -p fontcraft-engine --test engine detect:: -- --ignored
   --nocapture`.
6. **Follow-up packages the writers named:**
   - TTX: the TrueType `post` table compiled again when a dump lacks it; glyph-order renames in a TTX import into a
     CFF font (X.ttx4).
   - Instances: the file names of masters exported as instances, against fontmake's (`FontInfo-Thin.ttf`).
   - PDF: `draw_placed_pdf` hashes again when glyphs alternate between two PDFs.
   - Merge: CFF2, CID-keyed fonts and `JSTF`, which fontTools refuses too.
   - Convert (M12.30): locations beyond the axis ranges, the other output targets and options for UFO output.
   - Project compile (M12.40 to M12.42): mirrored components in single precision and makeotf's lookup grouping
     across lookup flags, which are not followed (M12.40c); a guess entry for the compile's fallback warning;
     bundled character collection data and `GSUB` resources for the ROS parameter.
   - Colour: `feImage` in `SVG ` filters; filtered sprites keyed by glyph, not by occurrence (a large glyph clipped
     at both canvas edges runs its filters again on every frame). Colour drawing: screenshots.
   - Table Viewer (M12.38): the source of the last export, a control-channel measure and screenshots.
   - Small UI leftovers: the Exports tab's aliases (not yet defined).
   - Sixteenth batch, engine: the `stems` subcommand's overlap options (M10.37c); colour SVG artwork and PDF
     compression (M11.21); `glyph.replaceWithInstance` declares no glyph targets, which the second window's dialog
     needs (M9.36); Erase Open Corners at compile is refused until M3.31 (M2.13); the cap commands wait for M13.40
     (M6.25); autohint on a special layer hints the master (M10.40); new glyphs from component commands are opened only
     for a person's run, not over the control channel (X.integ72, untested).
   - Wave 1 (M14.3, M13.78, M3.19, M3.21, M9.27, M7.21): the project asked about in the save question is not brought
     forward; `ui.text` can type into a dialog behind the question, and Tab does not move focus inside it; check that
     tool letters stay blocked while it is open. `ui.contextMenu.invoke` does not notice a master switch since the
     menu opened, and a multi-step item is not one transaction. The Text tool moves the measurement line with
     Cmd+Ctrl+Alt, which is AltGr off the Mac (`canvas/text.rs`); off the Mac a Ctrl-drag also turns on the lever (Ctrl
     reports as Cmd). Cmd+Delete on an axis row; command warnings do not reach the panels. The extrapolation audit
     check (`instanceOutsideAxisRange`) keeps its own tolerance rule: decide whether it shares the placement helper. A
     corpus listing of the values the new parameter checks refuse.
7. **The Craft suite's shared libraries** (`docs/decisions/0008-shared-suite-libraries.md`): `craft-quit` is adopted
   (M14.3); X.suite3 (`craft-settings`, settings in `config.toml` with backup and recovery and a one-time import of
   FontCraft's JSON settings file) is the builder of wave 12; `craft-ui` is M14.9; `craft-keymap` (M14.7, M14.8)
   waits on two upstream pull requests. The suite's craft-next control server authenticates with a token; FontCraft's control channel has none. Whether to
   adopt the token protocol (a protocol change and an MCP backend change) is the owner's decision.
8. **Smaller follow-ups:** an intermediate layer's own TrueType instructions and the variation of TrueType
   composites with intermediate layers (M10.19, M10.19b); variable values in table blocks (`fea.var.tableValues`:
   `BASE`, `GDEF` carets, `hhea`, `OS/2`, `vmtx`; fea-rs rejects them); the layout of InterpolateLayout3 (two
   `.notdef` glyphs), fontc's combinatorial bracket kerning and decreasing-axis tables, and brace sources for
   composites of non-exported brace glyphs (with M9.39).
9. **The measures not yet run:** the sixteenth batch's (item 1); ttxn and tx for the normalised dump, sfntdiff for `diff`, cu2qu and
   ttfcomponentizer, otfautohint for the `autohint` shell and over the corpus (M10.33), FontBakery and
   comparefamily over the corpus (M12.81), and the § 5 measures of M6 (M6.22; mark attachment is run), M7 (names
   are run) and M8 (M8.34). M9's, the TTX writers' and readers', qu2cu's, the colour tables', subset's, convert's,
   project compile's and merge's are run, and so are the measures that need AFDKO's tools (tx, makeotf, rotatefont,
   cffsubr and the CFF2 variable compile against fontmake).
10. **Shell hooks for finished M13 packages**: dialogs and menu entries several commands wait for
   (for example the metrics-file import chooser, Status Definitions, the name-list rename). In the second window,
   UI.30 takes them with this batch's engine packages.
11. **Checks in the reference app**: `docs/observe.md` lists every row the running app could
   settle (1,500 rows), with the synthetic test fonts `cargo xtask testfonts` writes; depth.md § 7 names the
   checks that settle the most guesses.

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (1,009 commands, 33 tools, 182 panels, 47 settings, 241
  parameters, 1,754 behaviour rules, 301 checks, 74 CLI rows, 53 formats, 50 tables) and 23
  behaviour notes (about 150,000 words), all in our own words. Every M3 command's `params` name
  its targets; shortcuts carry a `when` (the context they apply in). Every choice of ours where
  no source says what the reference does is a row's `guesses` entry, listed in `docs/guesses.md`.
- **Workspace:** 43 library crates in enforced layers, three apps (the egui window, the craft-next window, the CLI), `xtask`; about 711,000 lines of Rust in the crates' and apps' `src` folders, outside the test folders and `xtask`.
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
- **Builder packages M10.3, M10.14 and M11.5 (part 1), with X.fix20 and X.fix21:** hint ends and
  TrueType instruction points that stay on their nodes through edits; stem hints, masks, flex and
  manual PostScript hints in CFF exports; typed paint attributes in `.glyphs` files of versions 2
  to 4. Numbers and what is left are under "What is real: hints and paint".
- **Builder packages M12.44, M12.17 and M13.2, with X.fix22:** the audit runner over sources,
  compiled fonts and families with profiles, the ignore list and exit statuses (`fontcraft-cli
  check`); font subsetting with pyftsubset's options (`fontcraft-cli subset`); the plug-in module
  runtime under resource limits. Numbers and what is left are under "What is real: the audit
  runner, subset and the module runtime".
- **Builder packages M9.13, M10.17, M10.6 and M10.7, with X.fix24:** variable fonts with CFF2
  outlines (`--variable --format otf`, `file.exportVariable {outlines: "cff2"}`); the TrueType
  font programs (`cvt `, `fpgm`, `prep`) tested in skrifa's interpreter; 14 commands that edit
  TrueType instructions against a touched-state model. Numbers and what is left are under "What
  is real: CFF2 variable fonts and TrueType hinting".
- **Builder packages M10.18 and M10.19, X.suite1 and X.fix25:** TrueType instructions compiled
  into `glyf` programs and written into static and variable TrueType exports, with the generated
  or imported `cvt `, `fpgm` and `prep`; `hint.tt.getPointIndexes`; atomic file writes through the
  Craft suite's `craft-atomic` library (decision 0008); review fixes to the point map. Numbers
  and what is left are under "What is real: TrueType glyph programs and export".
- **Build layout:** the engine's integration tests are one binary, as are the decompile,
  io-glyphs, io-ufo and plist tests; the `measure` profile serves speed measures and oracle runs
  (`docs/development.md`).
- **Fixes after the gate:** the web build compiles; every export path and the code editor read
  feature includes from the source's include folder; `.glyphs` 3 files keep deleted kerning
  deleted and keep a version 2 kerning key the reader did not use; a refused include candidate does
  not hide an allowed one.
- **This batch (75 packages, X.deps1, X.integ1 to X.integ5, X.fix26, X.fix27, X.audit1):**
  - Components and windows: M6.11 component and corner Info boxes and glyph pickers; M6.12 smart settings
    windows; M6.13 Pixel tool; M7.19 project window; M8.17 Features tab; M9.24 Compare Fonts and Compare Family.
  - Features and shaping: M8.18 shaping crate; M13.47 `MATH` and `BASE`; M13.49 optical bounds.
  - Interpolation: M9.15 axes; M9.16 special layers; M9.17 queries and axis mappings; M9.18 instances and
    masters; M9.21 special-layer editors; M9.22 Master Compatibility view; M9.23 interpolation dialogs;
    M13.48 cross-axis mappings.
  - PostScript hinting: M10.15 autohinting at export; M10.4 hint commands; M10.24 hints in Edit View; M10.22
    hinted import; M10.31 hint smart filters.
  - TrueType hinting: M10.21 autohinter; M10.5 zones and stems; M10.25 to M10.27 Instructor tool, pixels, Info
    box; M10.28 zone and stem editors; M10.29 Hinting Preview; M10.30 web preview.
  - Colour: M11.6 `CPAL` and `COLR` v0; M11.7 `COLR` v1; M11.8 `sbix`; M11.10 `SVG `; M11.15 palette editor;
    M11.18 colour read-back.
  - Command-line and tables: M12.5, M12.6, M12.7, M12.10 TTX writers and dump views; M12.13 convert; M12.14
    compile and lint; M12.29 curve conversion; M12.32 diff; M12.39 autohint and stems.
  - Checks: M12.0 fix commands; M12.47, M12.49, M12.51, M12.52 checks; M12.63 Font Info fixes; M13.50 CID.
  - Edit View: M13.28 Find and Replace Outline; M13.29 non-linear transform; M13.30 autotrace; M13.31, M13.33,
    M13.34, M13.35 display aids, drag aids, auto-meter, suggestions; M13.36 power guides; M13.37 Rapid and Spiro.
  - Glyph data and metrics: M13.17 constructions; M13.19 small caps and sub/superscripts; M13.20 auto-spacing;
    M13.21 auto-kerning.
  - Modules and automation: M13.3 module commands; M13.4 filters in export; M13.5 check modules; M13.6 dynamic
    menus; M13.8 Actions dialog; M13.10 Extension Manager; M13.12 MCP over HTTP; M13.14 author documentation;
    M13.55 BDF and PCF; M13.58 PDF reading.
  - Fixes: X.fix26 (CFF import keeps every declared stem, UFO colour layers), X.fix27 (pre-push review: trusted
    ttfautohint binary, control budget, MCP host check), X.audit1 (project values in compiled-font checks, more
    `audit.fix` steps), X.deps1 (lopdf, ureq, `craft-print` as workspace dependencies).
  Numbers and the four missed measures are under "What is real: this batch".
- **The next batch (17 packages, X.tune1, X.integ6, X.fix29, X.fix30):**
  - Features and shaping: M8.8 table blocks and `usMaxContext`; M8.19 shaping and the Features menu in Edit View;
    M8.21 contextual anchors (NOTICE credits ufo2ft, MIT).
  - Interpolation and preview: M9.20 the preview's instance pop-up, `preview.render` and master switcher icons.
  - Colour and images: M11.11 `CBDT` and `CBLC`; M11.13 images in Edit View (`ui-egui` depends on `io-pdf`);
    M11.19 variable `COLR`.
  - Command line and tables: M12.8 TTX reader; M12.20 static instances (fontTools instancer ported); M12.23 merge
    (pyftmerge ported); M12.24 transform (scaleUpem ported); M12.25 CFF subroutiniser (compreffor method ported);
    each with its NOTICE entry.
  - Fixes and checks: M12.62 `path.eraseOpenCorners`, `component.flatten`, `audit.fix` layer steps 7 to 33.
  - Spacing, kerning and hinting: M13.22 pair lists, Pairs and Phrases, Kerning Pairs Preview, Groups Editor;
    M13.23 Space Center; M13.51 TrueType assembly, Instructions dialog and Review Hints; M13.52 TrueType
    debugger (interpreter ported from skrifa, in `crates/engine/src/ttvm`).
  - Tuning and fixes: X.tune1 (auto-spacing and auto-kerning), X.integ6 (integration; the duplicate
    `path.setShapeOrder` removed), X.fix29 (review findings: work budgets, staged outputs, confined TTX parts,
    anchor contexts), X.fix30 (TTX oracle fixtures as per-table SHA-256).
  Numbers are under "What is real: the next batch".
- **The ninth batch (14 packages, X.tables1, X.ds1, X.small1, X.integ7 to X.integ9, X.fix31 to X.fix34):**
  - Text and layout: M8.20 shaping and bidi in Text Preview; M13.23b the Space Center's display options.
  - Hinting and composites: M10.21b x-height scaling, zone rounding and shifts in the TrueType autohinter; M13.15b
    the Detect Composites base choice and a kept users index for the alignment pass.
  - Colour: M11.14 colour drawing in Edit View and the previews.
  - Instances and tables: M12.22 instances from sources; M12.21 partial instances (the instancer's region solver
    ported); M12.26 CFF subroutines at export and `optimize`; M12.34 the `variations` tools; M12.9 TTX reader for the
    layout, variation and colour tables; M12.7b TTX writers for `COLR` version 1, the bitmap and the Apple tables;
    M12.13b FontDame conversion; each port with its NOTICE entry.
  - Audit: M12.61 `audit.fix` on compiled fonts; M12.64 the Font Audit panel and the Audit Profile dialog.
  - Tasks: X.tables1 (`maxp` recalculation and output fidelity for instance, transform and merge), X.ds1
    (designspace instance data and `<rules>`), X.small1 (the `CBDT` option, the assembly warning, hint nudging).
  - Integration and fixes: X.integ7 (four groups), X.integ8, X.integ9; review fixes X.fix31 to X.fix34 (work budgets
    in the variation and bitmap readers, staged and confined instance outputs, the colour paint budget, fontmake-style
    instance file names).
  Numbers are under "What is real: the ninth batch".
- **The tenth batch (17 packages, X.conv2, X.small2 to X.small6, X.integ10 to X.integ12, X.fix35 to X.fix40):**
  - Opening compiled fonts: M8.23 feature code rebuilt from `GSUB`, `GPOS`, `GDEF` and `BASE`; X.small2 stored tables
    kept on open.
  - Subset: M12.18 `GSUB`, `GPOS` and `GDEF`; M12.19 variation, colour, bitmap and other tables; M12.19b CFF2 and
    subroutine pruning.
  - Convert and compile: M12.30 `font.convert` and X.conv2 its oracle fixes; M12.40 AFDKO project compile and M12.40b
    its makeotf parity; M12.41 CID-keyed CFF; M12.42 compile extras.
  - Instances, merge and TTX: M12.20b CFF2 instances; M12.23b merge of CFF, `MATH` and `BASE`; M12.7c TTX readers for
    `COLR` version 1, the bitmap, Apple and miscellaneous tables, and split dumps.
  - Colour and images: M11.16 Font View cells; M11.17 colour Text Preview; M11.5b UFO full-colour layers and
    JPEG and TIFF images saved as PNG.
  - Audit and windows: M12.38 the Table Viewer; M12.64b staged audit runs with progress and cancel, fix preview and
    report export; X.small5 and X.small6 save choosers.
  - Tasks: X.small3 (`CBDT` in export profiles, the assembly warning, merged bounds), X.small4 (truncations that
    warn, autohinting from export profiles).
  - Integration and fixes: X.integ10 (three groups), X.integ11 (three groups), X.integ12; review fixes X.fix35 to
    X.fix40 (work and memory budgets in convert, subset, instance and the TTX readers, confined and staged outputs,
    the empty-lookup and index-overflow checks before fea-rs, `GPOS` class pair tables kept through every rewrite).
  Numbers are under "What is real: the tenth batch".
- **The eleventh batch (M9.25 and 20 follow-up packages, X.fix41 to X.fix45, X.tables2, X.ui1, X.audit2, X.audit3,
  X.opt1, X.opt2, X.edit1, X.integ13 to X.integ16, X.fix47 to X.fix54, X.gate15):**
  - Model and files: M9.25 higher-order interpolation data on nodes; M13.55b bitmap strikes in the model with BDF and
    PCF import and export; M8.23b one feature-code writer, and a kerning entry turned back on kerns once.
  - Subset: M12.19c `VARC`; M12.19d compressed `SVG ` documents and composite bitmaps; M12.18b pyftsubset's list
    options and the rest of its table and output options.
  - TTX: M12.7d `mort` and `morx` written and read; M12.7e bitmap images in the row and bitwise forms read.
  - Convert and transform: M12.30b Type 1, bare CFF and TTX input; M12.24b units per em on CFF and CFF2, and `--to`;
    M12.24c matrices on CFF, CFF2 and variable TrueType fonts, `--glyph-table` and `--fd`; X.tables2 `post`
    versions and `maxp` after instance, transform, merge and outline repairs.
  - Instances: M12.21b partial instances of `avar` version 2, `VARC` and variable `COLR` fonts; M12.22b same-named
    instances, unaligned components and custom parameters in instances from sources; M12.22c `--layout-only` and
    stated style-map names.
  - Optimise and CFF: X.opt1 implied on-curve points, `gvar` IUP, the `GDEF` variation store and `COLR` reuse;
    X.opt2 `GPOS` compaction; M12.25b the subroutiniser on full CJK fonts; X.fix42 charstring memory and point
    limits sized for full CJK fonts.
  - Project compile: M12.40d makeotf's project paths, licence code, OS/2 settings, UVS and CMap keys.
  - Colour and images: M11.5c UFO full-colour layers with paths, `PaintColrGlyph` and variable paints; M11.7b
    `PaintColrGlyph` in compiled `COLR`; M11.17b the rest of the colour Text Preview; M11.13b image editing in Edit
    View.
  - Text: M8.20b line breaking on shaped widths and vertical text in Text Preview; X.edit1 one feature compile for
    `features.compile` and the previews.
  - Audit and window: X.audit2 and X.audit3 the rest of `audit.fix` and `tables.recalculate`; X.ui1 small UI
    leftovers (tool letter cycling, Smart Glyph Settings, instance Copy and Paste, check marks, theme tokens).
  - Fixes: X.fix41 (`SVG ` subset reads charged, the lookup guard), X.fix43 to X.fix45 (low findings of earlier
    reviews: staged audit undo, extension case, named pipes, WOFF declared size, audit folder entries outside the
    read root).
  - Integration and review fixes: X.integ13 to X.integ16; X.fix47 to X.fix54 (the lookup guard as an upper bound,
    the `mort` cell charge, colour reference checks in linear work, `GPOS` class counts fitted on every write, the
    `optimize` and `interpolate_layout` budgets, dehint padding); X.gate15.
  Numbers are under "What is real: the eleventh batch".
- **The twelfth batch (12 follow-up packages, X.ui2, X.fix55, X.integ17, X.integ18, X.fix56 to X.fix58,
  X.gate16):**
  - Merge: M12.23c variable TrueType fonts merged as variable fonts.
  - CFF2 and CFF: M12.25c specialised and subroutinised CFF2 exports; M12.25d blended CFF2 hints; M12.41b the ROS
    parameter as CID-keyed CFF.
  - TTX: M12.7f bitmap images in the row, bitwise and external-file forms, and external-file images read back.
  - Bitmap strikes: M13.55c the `EBDT` and `EBLC` writer, `.otb` export and strike rasterising; M13.55d BDF, PCF
    and `.otb` fonts opened, and the strikes of compiled fonts read into the model.
  - Colour: M11.14b `PaintColrGlyph` references drawn in Edit View and the `SVG ` table; M11.17c `SVG ` clip paths,
    masks, patterns and sliced images in the colour Text Preview.
  - Text: M8.20c vertical Text Preview with `vkrn`, `vrt2` and vertical mark positioning.
  - Subset and instances: M12.18c `--pretty-svg`, `--xml`, `--OPTION?` and `--show-defaults`; M12.22d
    `--layout-only` for `.glyphs` sources and `--interpolate-binary-layout`.
  - Transform and window: X.fix55 rotatefont's reading precision, stems and hint groups; X.ui2 check marks on every
    menu switch.
  - Integration and review fixes: X.integ17, X.integ18; X.fix56 to X.fix58 (a failed binary-layout interpolation
    writes nothing, one rasterising budget per call, `SVG ` reference fan-out bounded, CFF2 stem-blend overflow
    reported, a font-wide strike decode budget, static merges without variation stores); X.gate16 (the name-table
    record limit).
  Numbers are under "What is real: the twelfth batch".
- **The thirteenth batch (10 follow-up packages, X.fix59, X.pdf1, X.var3, X.cff3, X.audit3, X.ui3, X.glyphs3,
  X.integ19, X.integ20, X.fix60 to X.fix66, X.gate17):**
  - Variable fonts: M10.19b `cvar`; M12.25e CFF2 hints of intermediate and alternate layers, sources autohinted
    together; X.fix59 each master's family blues in the CFF2 Private DICT; X.var3 feature variations under every
    tag of a feature list and fontmake's instance font info for non-default sources.
  - Instances: M12.22e `compile --interpolate-binary-layout` and `Unnamedfont-*` file names.
  - Convert and dump: M12.30c convert to UFO with glyph subsets, GOADB, `--location` and overlap removal; M12.30d
    default snapshots, CID keying, Font DICT and `/CID` selection, the CFF2 glyph order and CFF overlap removal;
    M12.10b spot listings, the CFF literal view and `--exclude-glyphs`.
  - CFF and audit: X.cff3 the instancer on `crates/cff`; X.audit3 `tables.recompileCff`, the per-glyph layer cache,
    `checkOutlines` all and quiet and `compareFamily` reports.
  - Colour and PDF: M11.17d `SVG ` filters and the sliced root clip in the colour Text Preview; X.pdf1 PDF in paste
    and Import Outlines.
  - Bitmap formats: M13.55e gzipped PCF, Windows FNT and FON; M13.55f TeX PK and GF and Mac bitmap fonts.
  - Detect Composites: M13.15c the base among equal outlines chosen by glyph data, names and case.
  - Window and files: X.ui3 the small UI leftovers (hint nudge undo, Instructor badge drag and Delete, module
    install from a drop, a Font View master per document, metrics beam drag, interpolated Exports preview);
    X.glyphs3 stale kept `.glyphs` spellings and layouts left out on save.
  - Integration and review fixes: X.integ19, X.integ20 (9 test expectations, clippy); X.fix60 to X.fix65 (filters
    under enclosing clips, the audit cache key, node flags in any order, sparse Dehint masters, the CFF2 instancer's
    limits, `seac` drawn out when CID keying, bounded Mac strike listings); X.fix66 (one-line layer guides, an
    order-independent Latin-copy rule in Detect Composites); X.gate17.
  Numbers are under "What is real: the thirteenth batch".
- **The fourteenth batch (M12.40c, M11.17e, X.glyphs4, X.audit4, X.ttx4, X.inst4, X.pdf2, X.integ21, X.integ22,
  X.fix67 to X.fix72, X.gate18):**
  - Project compile: M12.40c components in single precision and one lookup per rule type, as makeotf does.
  - Colour and PDF: M11.17e the remaining `SVG ` filter primitives, filters in Edit View and raster warnings in Text
    Preview; X.pdf2 the system clipboard's PDF in Edit View, one shared PDF parse and placed PDFs in bitmap strikes.
  - Command line and audit: X.audit4 `checkOutlines` clean-up after a fix, `check family -l` and repeatable list
    options; X.ttx4 the bounds of CFF fonts read from TTX.
  - Instances: X.inst4 designspace documents without axes, Thin style-map names and name tokens in file names.
  - Files: X.glyphs4 the layouts of `.glyphs` node attributes and version 2 settings in version 3 and 4 files.
  - Integration and review fixes: X.integ21, X.integ22 (test expectations, clippy, test-only fixes); X.fix67
    (compile messages through the include table, bounded component trees), X.fix68 (clipboard reads on a worker
    thread), X.fix69 (linear contour matching, CFF strings TTX cannot carry), X.fix70 and X.fix72 (a font saved as a
    designspace compiles, style-map styles as ufo2ft reads them, the lib key for other styles), X.fix71 (filters
    clipped to the canvas, the W3C notice); X.gate18.
  Numbers are under "What is real: the fourteenth batch".
- **The fifteenth batch, depth wave 1 (M14.3, M1.8, M9.27, M13.78, M3.19, M7.21, M4.17, M3.40, M14.1, M3.21,
  X.integ23, X.integ24, X.fix73 to X.fix77, X.gate19):**
  - Window: M14.3 save prompts on close, quit and File › Close, `app.quit`, `window.closePrompt` and the window title,
    with the suite's `craft-quit` (X.suite2 folded in); M1.8 the Open chooser, Open Folder, drop to open and project
    routing; M14.1 the settings window with its User Settings and Extensions sections.
  - Font Info: M9.27 axes and coordinates edited in the Font, Masters and Exports tabs; M7.21 value lists and checks
    for 64 custom parameters.
  - Font View and Layers: M4.17 filter rename, drag and Alt groups in the sidebar; M3.40 drag reorder and backup to
    master in the Layers section, and a master per glyph in text.
  - Drawing: M3.19 the Select tool's Alt and Ctrl+Alt drags and `path.reshapeSegment`; M3.21 the Measurement tool over
    every glyph of the line, overlaps in grey and the held tools.
  - Agent surfaces: M13.78 pointer, drag, wheel, `ui.set`, `ui.resize`, `ui.focus` and context menus over the control
    channel.
  - Integration and review fixes: X.integ23, X.integ24 (test fixes; measurement corners reported as overlaps, Edit
    View keys behind the save question); X.fix73 (off-Mac measurement keys, long lines truncated, a damped segment
    solve), X.fix74 (one placement query for instances, axes by index, paste skips refused rows), X.fix75 (sidebar
    refresh and keyboard, the edited glyph in master-row clicks), X.fix76 (the save question takes the keyboard,
    automation saves never open a chooser), X.fix77 (input waits for the frame that draws it, `ui.resize` clamps,
    context-menu items checked on invoke); X.gate19.
  Numbers are under "What is real: the fifteenth batch".

- **The sixteenth batch, depth waves 2 to 11 in part and the craft-next window (47 depth packages, M10.37b,
  M10.37c, M13.65b, ES.1 to ES.3, UI.1 to UI.29 with UI.4b, UI.9b, UI.14b, UI.15b and UI.21b, X.fix78 to X.fix87,
  X.integ26 to X.integ80, X.inte1):**
  - Compile: M2.8 formats, output selection and the CLI option table; M2.12 feature options and the plan's
    `features` section; M2.13 outline, CFF, glyph-set and variable options; M12.80 MakeOTF Compatibility Mode.
  - Command line: M12.67 common options; M12.68 readable listings; M12.73 and M12.74 subsets by the converter's
    selection and of sources; M12.69 mergefonts parity; M12.70 and M12.75 the converter's CFF switches and extras;
    M12.76 dump leftovers; M12.66 outline repairs on CFF; M12.77 the option-coverage measure; X.fix80 option
    prefixes in every parser.
  - Proofs: M12.36 PDF proof sheets; M12.78 the proof tool's layouts; M12.79 spot sheets; M13.68 Export Window
    Contents.
  - Hinting: M10.36 autohint options; M10.37, M10.37b and M10.37c autohint of compiled CFF and CFF2 fonts; M10.40
    upkeep on backgrounds and corner components; M10.35 extra nodes in the Instructor; X.fix78, X.fix79 and X.fix85
    hints renumbered by path.
  - Interpolation: M9.29 higher-order interpolation in instances; M9.30 HOI commands; M9.34 Match Masters; M9.36
    designspace helpers; M13.66 Overlay Fonts.
  - Features: M8.32 Indic positional features, with X.fix81; M8.35 lookup debug info; M8.28 number values; M8.29
    feature entries disabled, copied and pasted.
  - Colour and bitmaps: M13.65 and M13.65b `CPAL` version 1, a dark palette and palettes in older formats; M11.21
    colour artwork; M13.63 bitmap glyph commands.
  - Sources and editing: M1.9 nice names; M1.11 UFOZ save and format warnings, with X.fix82; M3.36 path attributes
    in UFO; M3.29 Round Coordinates and Tidy rules; M6.21 component edit guards; M6.25 written sources for caps,
    segments, brushes and corners; M5.20 vertical metrics setters; M5.22 shared metrics; M13.21b kerning audit;
    M7.23 Font Info warnings.
  - Automation: M13.80 MCP window tools; M13.71 External Preview; X.pdf3 the clipboard's PDF on Windows and macOS.
  - Engine split: ES.1 `engine-core` and the command registry; ES.2 helpers decoupled; ES.3 extension slots.
  - The craft-next window: UI.1 the skeleton (the framework pinned, icu 2.3.0, the glyph-data oracle moved out of
    `xtask`); UI.2 to UI.5 and UI.4b canvas, tools, text mode and overlays; UI.6 control channel; UI.7 and UI.24
    dialogs; UI.8, UI.9 and UI.9b Font View; UI.10, UI.22 and UI.23 Font Info; UI.11 Palette panels; UI.12 Layers;
    UI.13 glyph sections; UI.14 and UI.14b kerning; UI.15 and UI.15b Space Center and Preview; UI.16 Text Preview;
    UI.17 Font Audit; UI.18 Table Viewer and Compare; UI.19 Start window, open, close and Settings; UI.20 Project
    panel and extensions; UI.21 and UI.21b console, command palette, About and Actions; UI.25 context menus and
    clipboard; UI.26 component Info boxes; UI.27 colour glyphs; UI.28 hinting on the canvas; UI.29 TrueType dialogs.
    X.fix83 canvas tools edit the active layer; X.fix84 dialogs open on the Font View selection.
  - Integration and test fixes: X.integ26 to X.integ80 and X.inte1 (merge fixes, clippy, blessed command ids,
    generated lists, fmt); X.fix86 (24 test failures: fixtures that assumed an empty new font, `head` checksum
    compares, palette and compile-option tests, response files after option prefixes, proof title metadata);
    X.fix87 (UI test fixes: the Instructor's Delete, dialog targets, the Preview Area strip); X.fix88 and X.fix89
    (review fixes); X.fix90 (wasm).
  Numbers are under "What is real: the sixteenth batch".

## Milestones

This file does not estimate time: estimates of agent hours have proved unreliable. Progress is
measured by what works (the parity table below and the measures in each milestone's plan).

| # | Milestone | Status | Commands (live) |
|---|---|---|---:|
| M0 | Skeleton and vertical slice | ✅ done | 5 (5) |
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | ✅ done: every format read and written back byte for byte; the typed model (M1.7) with glyphsLib's lib keys, intermediate and alternate layers in a designspace, and its measures met; the gaps left on purpose are in `docs/plans/M1.md` (M1.4 and M1.7). Depth packages: 3 of 5 built (M1.8, the Open chooser, Open Folder and drop to open; M1.9 nice names on import; M1.11 UFOZ save and format warnings) | 11 (11) |
| M2 | Compile core: static TTF and OTF, oracle harness | ✅ done: the oracle harness, the staged pipeline, the core tables at fontc and fontmake parity (no table different), static CFF, WOFF and WOFF2 with transforms, the export commands and CLI `compile`; every measure met (`docs/plans/M2.md` § 5). Deferred on purpose: instances between masters (M9), overlap removal (M3), CFF subroutines, export dialogs (M3). Depth packages built: M2.8 (formats, output selection, the CLI option table), M2.12 (feature options) and M2.13 (outline, CFF, glyph-set and variable options); a multi-master source compiles to a variable TrueType font by default (confirmed by the owner) | 5 (5) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | ✅ every measure met: every package M3.0 to M3.18 built; 184 of 184 commands live, gesture equals command for all 19 tool rows, an agent draws and exports over the control channel, overlap removal at export coverage-equal to fontmake's in over 99.5% of glyphs, M1 and M2 baselines unchanged (`docs/plans/M3.md` § 5). `path.sharpenCorner`, which no source describes, moved to `fidelity`. Depth packages: 5 of 26 built (M3.19 Select drag modifiers and `path.reshapeSegment`, M3.21 the Measurement tool over the line, M3.40 Layers section gestures, M3.29 Round Coordinates and Tidy rules, M3.36 path attributes in UFO and hints renumbered after Expand Outline); M3.22 and M3.23 have editor parts in the second window only | 185 (185) |
| M4 | Font View and glyph management | ✅ every measure met: every package M4.0 to M4.12 built; 62 of 62 commands live; the glyph database equal to fontc's on category, subcategory and production name for 200,504 names; `post` names equal both oracles' except one allow-listed source; every M4 dialog equals its command; an agent builds a font from glyph sets over the control channel; M1 to M3 baselines unchanged (`docs/plans/M4.md` § 5). Depth packages: 1 of 8 built (M4.17 filter management in the sidebar); M4.18 and M4.19 have editor parts in the second window only | 68 (62) |
| M5 | Spacing, kerning and text mode | ✅ every measure met but the two out-of-sync speed targets: every package M5.0 to M5.18 built; 97 of 97 commands live; every corpus metrics key parses; the kerning lookup equals fontc's and fontmake's compiled value for 506,285 pairs per oracle but one source that turns its kern writer off; exported fonts carry the same generated pair kerning but 27 known differences in two sources; kerning read back from 97 compiled fonts with 0 differences; every M5 dialog equals its command; an agent types, spaces and kerns text over the control channel; M1 to M4 baselines unchanged. Missed: on Inter the out-of-sync update after one edit takes 7.4 ms (target 2) and the full scan 53.5 ms (target 50) (`docs/plans/M5.md` § 5). Depth packages built: M5.20 (vertical origin and width setters) and M5.22 (shared metrics of several glyphs); M5.29's Sample Strings settings are in the second window only | 100 (100) |
| M6 | Components and anchors | 🟨 first wave built: the component index, propagation, automatic alignment, composition, smart and corner components drawn and exported, the mark writers; the component, anchor, smart glyph, corner component and decompose commands; metrics of aligned composites. Second wave: Add Glyphs recipes and composition, and the Font View batch actions on components and anchors (M6.14). Edit View display and selection of components and anchors (M6.10). Later packages: component and corner Info boxes, glyph pickers and Component from Selection's dialog (M6.11), the Smart Glyph and Smart Component settings windows with the Palette's Smart Component section (M6.12), the Pixel tool (M6.13). Depth packages: M6.21 (component edit guards) and M6.25 (written sources for caps, segments, brushes and corners). Left: selecting and deleting corner components (needs a hint selection kind), M6.19 (close) and the measures (`docs/plans/M6.md`) | 47 (47) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | 🟨 first wave built: the parameter catalogue, Font Info fields at every scope, masters, instances, metric and stem definitions, scaling to a new UPM, export steps (filters, glyph sets, export folders), linked sources, text tokens, the legacy kern table. Second wave: projects (M7.14) and the Font Info window with its Font, Masters, Exports, Other Settings and Notes tabs (M7.15 to M7.18). The name table and OS/2 of static instances as fontmake builds them (M7.8). The project window with the instance list, instance editor and export controls (M7.19). Text-token expansion in instance PostScript and file names (X.inst4). Value lists and checks for 64 custom parameters (M7.21, depth wave 1). Font and Exports tab warnings from one engine function each (M7.23; shown in the second window only). Left: the Exports tab's copy and paste hooks, M7.22 (its parameter editors are in the second window only), M7.20 (close) (`docs/plans/M7.md`) | 69 (66) |
| M8 | OpenType features | 🟨 first wave built: the fea-rs wrapper with its guards, assembly with a source map, the syntax extensions, number, property and predicate tokens, 60 automatic feature generators, the feature entry commands, feature file import and export, feature code snippets. Second wave: the compile state (M8.15), with errors, fixes, completion and token evaluation. Builder packages: the automatic feature framework with Update and regeneration at export (M8.9), and layout tables at export compared lookup by lookup with fontmake and fontc (M8.7, M8.7b). Later packages: the Features tab with its code editor (M8.17), the shaping crate on harfrust (M8.18), table blocks and `usMaxContext` (M8.8), shaping in Edit View and the Features menu (M8.19), contextual anchors (M8.21), shaping and bidi in Text Preview (M8.20), layout reconstruction from compiled fonts (M8.23, M8.23b: one feature-code writer), `MATH`, `BASE` and optical bounds (M13.47, M13.49), line breaking on shaped widths and vertical text in Text Preview (M8.20b), one feature compile shared by `features.compile` and the previews (X.edit1), and vertical positioning (`vkrn`, `vrt2`) in Text Preview (M8.20c). The ligature, fraction, ordinal, ornament and aalt generators (M8.11). Depth packages: Indic positional features (M8.32), lookup debug info (M8.35), number values (M8.28), feature entries disabled, copied and pasted (M8.29). Left: M8.25 to M8.27, M8.30, M8.31, M8.33, M8.34, M8.24 (close) (`docs/plans/M8.md`) | 45 (44) |
| M9 | Interpolation and variable fonts | 🟨 first wave built: axis ranges, mappings and normalisation as the oracles derive them; glyph sources and the compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, the variable-font names and `STAT`, byte-equal to fontc's on test sources; feature variations from alternate layers; variable TrueType and CFF2 fonts open; the compare commands. Builder packages: the oracle harness for variable fonts and instances (M9.10), static instances between and beyond masters (M9.4), the variable TrueType compile with `file.exportVariable` (M9.5), variable kerning and marks (M9.9), alternate layers in variable fonts (M9.8b), and the reference's variable feature syntax lowered as glyphsLib does and compiled on fea-rs (M9.11, M9.12). Variable TrueType fonts equal fontc's in 103 of 158 corpus sources and fontmake's in 106; variable CFF2 fonts (M9.13: blended charstrings; specialised, subroutinised and with blended hints since M12.25c and M12.25d) equal fontmake's in 106; instances equal `fontmake -i` in 180 of 234. Later packages: axis commands (M9.15), special-layer commands and editors (M9.16, M9.21), queries and axis mappings (M9.17), instance and master commands (M9.18), the Master Compatibility view (M9.22), the interpolation dialogs with the Variable Fonts export tab (M9.23), Compare Fonts and Compare Family (M9.24), and cross-axis mappings (M13.48), the preview's instance pop-up and master switcher icons (M9.20). Designspace instances take their names, classes and font info from `<instance>`, and designspace `<rules>` compile into static instances and variable fonts (X.ds1). Higher-order interpolation data on nodes (M9.25). Layout-only instances of `.glyphs` sources and compiled instances (M12.22d). CFF2 hints of intermediate and alternate layers (M12.25e) and each master's family blues (X.fix59); feature variations under every tag of a feature list, and fontmake's font info for instances of non-default sources (X.var3). Axes and coordinates edited in Font Info, with the placement query `font.instance.placement` (M9.27, depth wave 1). Depth packages: higher-order interpolation in instances (M9.29) and its commands (M9.30), Match Masters (M9.34), the designspace helpers (M9.36). Left: variable values in table blocks; M9.28, M9.31 to M9.33, M9.35, M9.37 to M9.39; M9.26; showing all masters in a tab (`edit.showAllMasters`) (`docs/plans/M9.md`) | 66 (64) |
| M10 | Hinting (PostScript and TrueType) | 🟨 first wave built: the PostScript autohinter ported from otfautohint, agreeing with AFDKO 5.0.1 on its oracle scripts' cases, with reports and override sets; typed hint data and settings; the TrueType assembler; `gasp` at export; hinted previews. Builder packages: node-reference upkeep (M10.3: hint ends and instruction points follow their nodes through edits, in the command's undo step) and CFF hints at export (M10.14: stem hints, masks, flex, manual hints per master, family zones). Builder packages M10.17 (the TrueType font programs: `cvt `, `fpgm`, `prep`, tested in skrifa's interpreter) and M10.6 and M10.7 (the touched-state model and 14 instruction-editing commands). Builder packages M10.18 and M10.19 (glyph programs compiled from manual instructions and written into static and variable TrueType exports, tested in skrifa's interpreter). Later packages: PostScript autohinting at export (M10.15) and 11 hint commands (M10.4); the TrueType autohinter (M10.21, M10.21b), whose x-height rows agree with ttfautohint in 92.9% (target 95%), cap-height rows in 100% and zone extremes in 98.29% (target 99%); import of hinted fonts (M10.22); hints in Edit View (M10.24); zone and stem commands and editors (M10.5, M10.28); the Instructor tool with its Info box, pixels and Hinting Preview (M10.25 to M10.27, M10.29); the web preview (M10.30); hint smart-filter rules (M10.31). Later packages in M13: TrueType assembly and the debugger (M13.51, M13.52). Blended hints in CFF2 exports (M12.25d). `cvar` in variable TrueType fonts (M10.19b); one undo step for a hint nudge, Instructor badge drag and Delete on instructions (X.ui3). Depth packages: extra nodes in the Instructor (M10.35), the autohint options (M10.36), autohint of compiled CFF and CFF2 fonts (M10.37, M10.37b, M10.37c), upkeep on backgrounds and corner components (M10.40). Left: the autohinter's last misses (M10.21b, measured on 7 of 325 sources; M10.21c), M10.33, M10.34, M10.38, M10.39, M10.41, M10.32 (close); upkeep for cap hints (`docs/plans/M10.md`) | 60 (60) |
| M11 | Colour fonts and images | 🟨 first wave built: colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export. Builder package M11.5, part 1: paint attributes typed in `.glyphs` versions 2 to 4, with version conversion of stroke positions and loss reports on writing version 3 and UFO. Later packages: `CPAL` and `COLR` version 0 (M11.6) and version 1 (M11.7), `sbix` (M11.8), the `SVG ` table from svg layers and bitmaps (M11.10), the palette editor (M11.15), `CPAL` and `COLR` read back (M11.18), and a UFO's colour layer list as palette layers. `COLR` against fontc: 10 identical, 7 allowed, 0 different; against fontmake: 11, 7, 0. Further packages: `CBDT` and `CBLC` (M11.11), images in Edit View (M11.13), and variable `COLR` (M11.19; equal to fontmake's on three variable colour sources), and colour drawing in Edit View and the previews (M11.14). UFO full-colour layers as `colorLayers` paints and JPEG and TIFF images saved as PNG (M11.5b), images and colour in Font View cells (M11.16), and a colour Text Preview that draws `COLR`, `SVG ` and `sbix` from the compiled font (M11.17). Then: UFO full-colour layers with paths, `PaintColrGlyph` and variable paints (M11.5c), `PaintColrGlyph` in compiled `COLR` (M11.7b), image editing in Edit View (M11.13b), and compressed `SVG `, embedded images, `CBDT` and build errors in the colour Text Preview (M11.17b). Then: `PaintColrGlyph` references drawn in Edit View and the `SVG ` table (M11.14b), and `SVG ` clip paths, masks, patterns and sliced images in the colour Text Preview (M11.17c). Then: `SVG ` filters and the sliced root clip in the colour Text Preview (M11.17d), and PDF in paste and Import Outlines (X.pdf1). Then: the remaining filter primitives but `feImage` and filters in Edit View (M11.17e), and the clipboard's PDF in Edit View with placed PDFs in bitmap strikes (X.pdf2). Glyph artwork with shadows, inner shadows and pattern images, and Glyph as Image (M11.21). Left: `feImage`, M11.20 (close) (`docs/plans/M11.md`) | 42 (42) |
| M12 | Command-line tool suite and quality checks | 🟨 first wave built: table actions and `tables show`, collections, WOFF, TTX dumps of the core tables, the layout normaliser, feature list and decompile, glyph rename and reorder, the metrics report, dehint, outline repairs on whole fonts, Type 1, bare CFF and SVG fonts, variations map and build-avar, SVG and HTML proofs, source diff and merge, `bench` and `completion`; 157 checks, audit profiles, the ignore list and report export. Builder packages M12.44 (the audit runner over sources, compiled fonts and families, with `fontcraft-cli check`; 3,676 corpus runs, no crash) and M12.17 (subset by pyftsubset's options; core tables equal to its in 337 or 338 of 340 corpus fonts per selection). The other corpus measures have not been run. Later packages: TTX writers for CFF, CFF2, layout, variation, colour and miscellaneous tables (M12.5 to M12.7; against ttx 253, 700 and 143 equal, 0 different), the normalised dump (M12.10), feature convert (M12.13, M12.13b; FontDame 24 of 28 equal, the other 4 written down) and compile (M12.14), qu2cu conversion and componentize (M12.29), diff (M12.32), `autohint` and `stems` (M12.39), the width, glyph-set, table and layout checks (M12.47, M12.49, M12.51, M12.52) and the fix commands (M12.0, M12.63). Further packages: the TTX reader (M12.8, M12.9, M12.7c), static instances (M12.20), instances from sources (M12.22), partial instances (M12.21), merge (M12.23), transform (M12.24; each against fontTools on corpus fonts), the CFF subroutiniser (M12.25; 0.785 of cffsubr's size), used by static CFF exports and `optimize` (M12.26), glyph fixes (M12.62), the `variations` tools (M12.34), TTX writers for the Apple, bitmap and `COLR` version 1 tables (M12.7b), FontDame conversion with AGLFN names (M12.13b), audit fixes on compiled fonts (M12.61) and the Font Audit panel (M12.64). Then: subsetting of layout, variation, colour and CFF2 tables with subroutine pruning (M12.18, M12.19, M12.19b), conversion between TrueType, CFF and CFF2 (M12.30), the Table Viewer (M12.38), AFDKO project compile (M12.40, M12.40b), CID-keyed CFF (M12.41), compile extras (M12.42), CFF2 instances (M12.20b), merge of CFF fonts (M12.23b) and staged audit runs (M12.64b). Follow-ups: subset of `VARC`, compressed `SVG ` and pyftsubset's list options (M12.19c, M12.19d, M12.18b), TTX `mort`, `morx` and bitmap image forms (M12.7d, M12.7e), convert from Type 1, bare CFF and TTX (M12.30b), transform of CFF, CFF2 and variable fonts (M12.24b, M12.24c), partial instances of `avar` version 2, `VARC` and variable `COLR` fonts (M12.21b), the rest of instances from sources and `--layout-only` (M12.22b, M12.22c), the subroutiniser on full CJK fonts (M12.25b), makeotf's project keys (M12.40d), `optimize` (X.opt1, X.opt2) and the rest of `audit.fix` (X.audit2, X.audit3). Then: variable merge (M12.23c), specialised and subroutinised CFF2 with blended hints (M12.25c, M12.25d), the TTX bitmap writer forms (M12.7f), the ROS parameter as CID data (M12.41b), the last pyftsubset options (M12.18c) and layout-only instances of `.glyphs` sources and compiled instances (M12.22d). Then: `compile --interpolate-binary-layout` (M12.22e), spot listings and the CFF literal view (M12.10b), CFF2 hints of special layers (M12.25e), convert to UFO with subsets, GOADB, CID keying, a location and overlap removal (M12.30c, M12.30d), the instancer on `crates/cff` (X.cff3) and `recompileCff` with `checkOutlines` and `compareFamily` options (X.audit3). Then: project compiles that place components and group lookups as makeotf does (M12.40c), `checkOutlines` clean-up and `check family -l` (X.audit4), and TTX bounds of CFF fonts (X.ttx4). Depth packages, 14 built: the PDF proof (M12.36), its layouts (M12.78) and spot sheets (M12.79), outline repairs on CFF (M12.66), common options (M12.67), readable listings (M12.68), mergefonts parity (M12.69), tx's CFF switches (M12.70), subsets by the converter's selection (M12.73) and of sources (M12.74), convert extras (M12.75), dump leftovers (M12.76), the option-coverage measure (M12.77), MakeOTF Compatibility Mode (M12.80). Left: M12.71, M12.72, M12.81 to M12.84, M12.65 (close) (`docs/plans/M12.md`) | 103 (99) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | 🟨 first wave built (22 packages): curve tension, corner and geometry helpers, stem detection and filters, the Thickness tool, expressions, Detect Composites, stored selections and the reference font, status definitions, export profiles and artwork, external-change review, encoding and name-list tools, PFM and AFM import, UFO 1, EPS and SFD reading, the Command Console, macros, the URL scheme, MCP permissions, prompts and resources. Secondary-reference documentation only, nothing observed. Builder package M13.2, the module runtime (wasm modules under wasmi with resource caps; 18 hostile-module cases fail cleanly), is built and called by the module commands, export filters and check modules. Later packages: module commands, module filters in export, check modules, dynamic menus and the Actions dialog (M13.3 to M13.6, M13.8), the Extension Manager (M13.10), MCP over HTTP (M13.12), author documentation (M13.14), constructions (M13.17), small caps (M13.19), auto-spacing and auto-kerning (M13.20, M13.21; tuned in X.tune1: auto-kerning meets its measure, auto-spacing misses it on Inter), Find and Replace Outline, the non-linear filter and autotrace (M13.28 to M13.30), display and drag aids, auto-meter, suggestions, power guides, Rapid and Spiro (M13.31, M13.33 to M13.37), `MATH`, `BASE`, cross-axis mappings, optical bounds, CID-keyed fonts (M13.47 to M13.50), BDF and PCF readers (M13.55) and bitmap strikes in the model (M13.55b), the `EBDT` and `EBLC` writer with strike rasterising (M13.55c), bitmap-only fonts and the strikes of compiled fonts opened (M13.55d), gzipped PCF, Windows FNT and FON (M13.55e), TeX PK and GF and Mac bitmap fonts (M13.55f), Detect Composites' choice among equal outlines (M13.15c), PDF reading (M13.58), pair lists with the Pairs and Phrases section, the Kerning Pairs Preview and the Groups Editor (M13.22), the Space Center (M13.23, M13.23b), TrueType assembly and the debugger (M13.51, M13.52). Depth wave 1: pointer, drag, wheel, view-switch, resize, focus and context-menu input over the control channel (M13.78). Left: the other depth packages (M13.62 to M13.82 and the lettered follow-ups), M13.40 to M13.42 (caps, waiting for a trial), Glyphs project files (M13.59, waiting for a trial sample), M13.61 (close; M13.60 shared editing is dropped), the remaining editing and glyph-data packages, the shell hooks several finished packages wait for (toolbar buttons, dialogs, menus), and SFD wired into File › Open; Detect Composites rebuilds 78.1% of Inter's composites from the source's bases against a 95% target, which needs anchor or source hints to reach (M13.15c). Depth packages of the sixteenth batch: bitmap glyph commands (M13.63), `CPAL` version 1 and a dark palette (M13.65, M13.65b), Overlay Fonts (M13.66), Export Window Contents (M13.68), External Preview (M13.71), MCP window tools (M13.80), the kerning audit (M13.21b) (`docs/plans/M13.md`) | 197 (167) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web. Adopts the suite's `craft-ui` (docking, tabs, icon buttons, widget-state theme; decision 0008) | 🟨 depth wave 1: save prompts on close and quit, `app.quit` and the window title with the suite's `craft-quit` (M14.3), the settings window with its User Settings and Extensions sections (M14.1); the macOS quit path is not yet built on a Mac. A second window on the suite's craft-next framework (`crates/ui`, `apps/fontcraft-next`, UI.1 to UI.29) ports the egui window and sits beside it until it reaches parity. Left: M14.2, M14.4 to M14.11, X.suite3 (`docs/plans/M14.md`) | 32 (3) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 4 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 1039 (960) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 48 | 30 | 28% | tabs, zoom and scroll work; user settings kept between runs; the Start Window with recent documents; the toolbar with shared buttons and their menus; tools chosen by their letters, shared letters cycle; context menus; menu items greyed out by the selection, and check marks on every menu switch; bottom bar zoom controls; the Command Console; the external-change review on window focus; the project window; the Features menu; master switcher icons; save prompts on close, quit and File › Close, and the document name in the title (M14.3); the settings window with its User Settings and Extensions sections (M14.1). Missing: the settings window's other five sections, shortcut editor, resizable sidebars, the macOS quit path built on a Mac; nothing compared with the reference |
| Font View | 6 | 71 | 62 | 45% | grid, List View, sidebar with categories, languages and filters, search, the glyph database, Add Glyphs, renaming with references, the inspector, Selection Info, Glyph Info, the Start Window, the glyph set dialogs, the inspector's metric and kerning-group fields and the out-of-sync badge; Add Glyphs recipes and composition, batch component and anchor actions, five component smart-filter rules; Filter by Encoding File and Compare Fonts in List; the Master Compatibility corner and the Incompatible masters filter; hint smart-filter rules. images and colour glyphs in cells, with image files dropped on them (M11.16); filters renamed in place, dragged to reorder and into folders, and Alt groups in the rule editor (M4.17). Missing: the rename dialog for name lists; nothing compared with the reference, and many choices are guesses |
| Edit View text mode | 4 | 25 | 20 | 50% | typing, the Text tool, cursor keys, selection, clipboard as text, three directions, three kerning modes, kerned-gap colours, metrics labels, tabs from display strings, the Preview Area and Panel, the measurement line, Text Preview, sample strings, Find Glyphs; Text Preview shaped through the preview font, with UAX #9 bidi runs; shaping in Edit View through a preview font compiled from the feature code, with cursive attachment, live advances and a Features menu; one compile for the preview font and `features.compile`; Text Preview lines broken at UAX #14 opportunities on shaped widths, and vertical text shaped with `vert`, `vrt2` and `vkrn`. Missing: more than one direction per line in Edit View; nothing compared with the reference |
| Drawing and editing outlines | 10 | 99 | 98 | 33% | node, segment, transform, clipboard and selection commands; in the window Select, Lasso, Hand, Zoom, Draw, Pencil, Rectangle, Ellipse, Rotate and Scale, the bounding box and its knobs, context menus, the Info box, the Palette's Transformations and Fit Curve sections, the Transformations and Primitive Measurements dialogs, editing in any master; curve tension and harmonizing, corner and geometry helpers, the Thickness tool (by command), stored selections, the reference font; Pixel, Rapid, Spiro and Magnet tools; display aids (Tunni lines, curve tension, indexes, curve length, ghost outline, Lock Outline, metrics beam); drag aids (Tunni drags, sliding nodes, the lever, Power Nudge, servant nodes); snapping suggestions; Find and Replace Outline; the Select tool's Alt and Ctrl+Alt drags and `path.reshapeSegment` (M3.19); the Measurement tool over every glyph of the line, with overlaps in grey and the held tools (M3.21). Missing: drag badges, a toolbar button for the Thickness tool, the Select All Layers tool, `path.sharpenCorner` (no source, in `fidelity`); nothing compared with the reference |
| Path operations and filters | 6 | 67 | 64 | 34% | Remove Overlap, the booleans, path direction, extremes and inflections, Tidy up, Open Corner, Reconnect, Cut with Line, curve conversion and rounding as commands; the Knife; the six filters with dialogs and export strings; strokes and masks drawn on the canvas and applied at export; the Stroke section and the Shape Order dialog; stem detection and the four stem filters with dialogs; ink traps, Make Parallel, Space Points, Make Arc, Slice Along Guide; the non-linear transform filter; autotrace of a placed image into outlines; Erase Open Corners; Shape Order with hint renumbering. Round Coordinates on the selection and the Tidy rules (M3.29); hints renumbered by path after Expand Outline, Decompose, Component from Selection and the other commands that rebuild shapes (M3.36, X.fix78, X.fix79, X.fix85). Missing: stroke attributes in UFO; nothing compared with the reference |
| Reusing shapes | 7 | 70 | 55 | 22% | the component index, anchor propagation, automatic alignment after every edit, composition data, smart and corner components drawn and exported; component, anchor, smart glyph, corner component and decompose commands; derived metrics of aligned composites; Add Glyphs recipes; Detect Composites and Detect Component (78.1% of Inter's composites rebuilt from the source's bases against a 95% target, with bases of equal outlines chosen by the glyph database, names and case), anchor expressions and automatic layers, Show Dependencies, Related Glyphs. Edit View draws components by alignment state, placeholders, anchors and corner tints, and its clicks, rubber band, Select All, Tab, double-click, Option-drag, Delete, smart handles and size knobs run their commands; component and corner Info boxes, glyph pickers, Component from Selection's dialog, the Smart Glyph and Smart Component settings windows, Build Glyphs from Constructions (corpus measure: 196 of 200 glyphs, 2,406 of 2,442 positions within 1 unit), small caps and sub/superscript glyphs, Flatten Components. edit guards for locked and aligned components (M6.21); the file keys of caps, segments and brushes from written sources (M6.25). Missing: selecting and deleting corner components, the rest of the M6 UI packages (M6.19); nothing compared with the reference |
| Layers | 3 | 21 | 21 | 40% | background and layer commands; Edit Background; the active layer and the tab's master; the background and other visible layers drawn on the canvas; the Palette's Layers section with its row context menu; the Interpolate with Background dialog. Font View's master choice; bold bracket and brace labels in the Layers palette, with a double-click opening the special layer editor; rows dragged to reorder within a master, a backup dropped on the master row, and a master per glyph from a text selection (M3.40). Missing: nothing compared with the reference |
| Guides, metric lines, zones, grid | 3 | 40 | 40 | 30% | guide, ruler, coordinate and Dimensions commands; metric lines, guides, the grid, CJK lines and measurement guides drawn; guides dragged and edited on the canvas, with their context menu and the Guide Properties dialog; the Measurement tool; snapping to guides and their crossings; the Palette's Dimensions section; power guides (make, add from stems, unlink, move with proportional spacing) and the Magnet tool; the auto-meter and the Measurements Palette section; snapping suggestions. Missing: script-specific metric lines in place of the master's (they are drawn beside it); nothing compared with the reference |
| Spacing and kerning | 7 | 93 | 90 | 50% | metrics keys, sidebearing and width setters, the spacing and kerning shortcuts in text mode, Update Metrics, Transform Metrics, kerning pairs, groups, exceptions and locks, Clean Up, Compress, group rename; the Info box's spacing and kerning fields, the Kerning window, Show Group Members, the Font View inspector fields and out-of-sync badge, metrics import and export (kerning-text, AFM, PFM), kerning read from compiled fonts; generated pair kerning in exported fonts, the lookup equal to fontc's and fontmake's compiled values over the corpus; side profiles, auto-spacing, Center in Width and Copy Metrics, the metrics beam on the sidebearing setters; auto-kerning, Kern to Distance, kerning groups from side profiles, Remove Pairs, vertical kerning from horizontal; the Pairs and Phrases section with auto-kern review, the Kerning Pairs Preview, the Kerning Groups Editor, group-member stepping; the Space Center (text, control glyphs, size, line modes, cut-off band, preferred suffix, matrix of widths and sidebearings, sample-strings menu, waterfall, right-to-left, centred and upside-down modes, fill, stroke and inverse display, overlays and the metrics beam). Measures after tuning: auto-kerning's sign agrees with the sources' in 85.7% to 99.2% of pairs in every master (target 80%, met); auto-spacing's median error is 5% to 12.5% on Oswald and 30% to 75% on Inter against the 10% target (missed on Inter; this method's best fit to Inter's spacing is about 60%; its speed targets are met). vertical origin and width setters and keys (M5.20); shared metrics of several glyphs (M5.22); a kerning audit of class-pair collisions and cross-master matching (M13.21b). Missing: images in the Space Center's display menu, the out-of-sync update at its speed target; nothing compared with the reference; many choices are guesses |
| Font Info | 6 | 63 | 60 | 38% | fields at font, master and instance scope, added and localized fields, row copy and paste; the custom-parameter catalogue with value checks; masters, instances, metric and stem definitions; scaling to a new UPM; filters, glyph sets and export folders at export; linked sources; text tokens; projects; the Font Info window (Cmd+I) with its Font, Masters, Exports, Other Settings and Notes tabs, shared row editing, and field and parameter pickers; status definitions; the name table and OS/2 of static instances from these fields, as fontmake builds them, with the version stamp; the Features tab (code editor with colouring, suggestions and error markers, Add Feature picker, snippets menu, stylistic-set names); dialogs for the Axis Location, Virtual Master, Axis Mappings, Color Palettes, Master Color and Master Color Dark values; the project window; axes and coordinates edited in the Font, Masters and Exports tabs, with extrapolation warnings (M9.27); value lists and checks for 64 parameters (M7.21). the Font and Exports tab warnings from one engine function each (M7.23). Missing: parameter value editors and those warnings in the egui window (the second window has both); nothing compared with the reference |
| Interpolation | 8 | 75 | 68 | 32% | Edit View draws and edits any master (`view.selectMaster`); axis ranges, mappings and normalisation; glyph sources and the compatibility check; the variable tables and `STAT`; variable fonts opened; feature variations and bracket glyphs from alternate layers; static instances between and beyond masters (alternate and intermediate layers, kerning, manual factors, Local Interpolation) equal to fontmake's in 180 of 234 compared; variable fonts compared with fontc and fontmake at masters, instances and midpoints; variable feature code compiled into variable fonts and resolved in static instances; designspace instances taking their names, classes and font info from `<instance>`, and designspace `<rules>` compiled into static instances and variable fonts; the compare commands; axis commands, special-layer commands and editors, interpolation queries and axis mappings, instance and master commands, the Master Compatibility view with its Incompatible masters filter, the interpolation dialogs with the Variable Fonts export tab, the Compare Fonts and Compare Family windows, cross-axis mappings and Interpolate Fonts; the preview's instance pop-up with instances drawn in the Preview Area, the master switcher icons, colour layers interpolated in instances; higher-order interpolation data kept on nodes (M9.25); instances from sources with same-named instances, custom parameters and `--layout-only`, also for `.glyphs` sources; `GPOS` of compiled instances interpolated from compiled masters, identical to fontmake's on 3 of 3 sources; feature variations under every tag of a feature list; fontmake's font info for instances of non-default sources; axes and coordinates edited in Font Info and the placement query `font.instance.placement` (M9.27). higher-order interpolation in static instances (M9.29) and its commands (M9.30); Match Masters and Relocate Start Points (M9.34); Set Master Locations, Adjust by Variation, Add Variation and Merge Fonts to Masters (M9.36); Overlay Fonts (M13.66); by command, with the M9.36 dialogs in the second window only. Missing: M9.26, showing all masters in a tab (`edit.showAllMasters`) |
| OpenType layout | 7 | 52 | 51 | 27% | feature code assembled with a source map, the reference's syntax extensions, number, property and predicate tokens, 60 automatic feature generators, feature entry commands, feature file import and export, snippets, the fea-rs wrapper with guards; feature compile state with errors, four fixes, completion and token evaluation; layout features listed and decompiled from compiled fonts; the `mark`, `mkmk`, `abvm`, `blwm` writers and `GDEF`; the automatic feature framework with Update and regeneration at export; feature code compiled into `GSUB`, `GPOS` and `GDEF` in exports, equal lookup by lookup to fontmake's in most corpus sources; the reference's variable feature syntax (conditions, variation blocks, variable positions and number tokens) lowered as glyphsLib does, compiled into feature variations and variable positions in variable fonts and resolved at the location in static ones; the Features tab's code editor; the shaping crate (harfrust with cached plans, private-use glyphs, live advances, bidi and script runs) used by Edit View through a preview font and by the shaping checks, with the Features menu; feature-code table blocks written into their tables and `usMaxContext` computed from the compiled lookups (5 allow-list entries removed, no differences back); contextual anchors (`anchor.setContext`, output identical to fontmake's on the probe source; fontc ignores them); optical bounds (`lfbd`, `rtbd`); `MATH` and `BASE` data. the six Indic positional generators, with Indic script tags in the automatic languagesystems (M8.32, X.fix81); lookup debug info (M8.35); feature entries disabled, copied and pasted (M8.29); the reference compilers' feature options (M2.12). Missing: variable values in table blocks, the layout of colour fonts |
| PostScript hinting | 3 | 29 | 29 | 24% | typed hints and hint settings in every stored form; the autohinter ported from otfautohint with its reports and override sets, agreeing with AFDKO's oracle scripts; hinted previews; dehint of compiled fonts; hint ends staying on their nodes through edits (M10.3); stem hints, masks, flex, manual hints per master and family zones in CFF exports (M10.14); autohinting at export (the outline as the charstring draws it, masks on closing lines); 11 hint commands (add, ghost, values, links, delete, clear, list, font hints); hints drawn, selected and dragged in Edit View with the hint Info box and context-menu items; import of hinted fonts keeping every declared stem; the Has Hints smart-filter rules and the Autohinting filter string; the Review Hints dialog; blended hints in CFF2 exports, merged as varLib merges hinted masters, for intermediate and alternate layers too, with each master's family blues; several hints moved by an arrow key in one undo step (`hint.ps.moveHints`). autohint of compiled CFF fonts and CFF2 variable fonts, overlapping glyphs on their overlap-free outline (M10.37, M10.37b, M10.37c); the fontinfo, overlap, allow-changes, keep-conflicts and decimal options (M10.36); upkeep on background layers and corner components (M10.40). Missing: upkeep for cap hints; the fontTools decode of autohinted fonts not yet run; nothing compared with the reference |
| TrueType hinting | 4 | 44 | 43 | 28% | typed instructions in every stored form, the assembler, `gasp` at export, hinted previews, instruction points staying on their nodes through edits (M10.3); the touched-state model and 14 commands that edit instructions (Snap Point, Set Snap Zone, Stem, Reverse, Set Stem Reference, Make Triple Hint, Set Instruction Rounding, Shift Points, Interpolate, Delta, Set Delta Cells, Delete Instructions, Remove Hints, List TrueType Instructions), each one undo step; the font programs `cvt `, `fpgm` and `prep` for zones and stems, tested in skrifa's interpreter (M10.17); glyph programs compiled from the manual instructions against the exported point map and written into static and variable TrueType exports (M10.18, M10.19; `hint.tt.getPointIndexes`), tested in skrifa's interpreter and read by fontTools without warnings; the autohinter (M10.21: autohinted export, `autohint.tt.run`, `hint.tt.autohintGlyph`, TTFAutohint options and control instructions; size 78.8% of ttfautohint's output and cap-height rows in 100% meet their targets; it scales to the x-height in `prep`, rounds zones as ttfautohint does and shifts points after IUP, and its x-height rows agree in 92.9% against 95% and zone extremes in 98.29% against 99%); an external ttfautohint binary runs only when named by the `export.ttfAutohintBinary` setting or `--ttfautohint`; import of hinted fonts, written back at export; zone and stem commands and editors; the Instructor tool with badges, direction, pixel size and point numbers, its Info box and Delta table, blue and red outlines on the pixel backdrop; the Hinting Preview waterfall; the web preview; assembly text for a glyph's program, `fpgm` and `prep` with a gloss for each opcode (`hint.tt.setAssembly`, the TrueType Instructions dialog), a glyph's own program written at export in place of its Instructor hints; the TrueType Debugger (stepping, breakpoints, stack, storage and zones, suggested deltas) on an interpreter ported from skrifa whose end points equal the preview's; the preview's instance pop-up; `cvar` in variable fonts from each master's control values (M10.19b); Instructor badges dragged to another node (`hint.tt.moveInstruction`) and Delete on instructions. extra nodes at intersections selected in the Instructor, their instructions dropped from variable exports with one warning per glyph (M10.35). Missing: the autohinter's last x-height and zone misses (M10.21b, measured on 7 of 325 sources), an intermediate layer's own instructions; nothing compared with the reference |
| Colour fonts and images | 3 | 50 | 50 | 41% | colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export; paint attributes (colours with palette references, gradients, shadows) typed in `.glyphs` files; `CPAL` and `COLR` version 0 and version 1 (paint runs, gradients, strokes outlined as layer glyphs, clip boxes), `sbix`, the `SVG ` table from svg layers and bitmaps, `CPAL` and `COLR` read back into palettes and colour layers, the palette editor, Master Color picker and Attributes inspector, a UFO's colour layer list as palette layers; `CBDT` and `CBLC` bitmap colour tables; variable `COLR` (COLRv1-var, COLRv0-var and COLRv0-2masters-brace equal to fontmake's variable builds; instances interpolate colour layers); images in Edit View (cache, PDF pages, gestures, info box, context menu, drop); colour glyphs drawn in Edit View and the previews (palette colours, gradients, strokes, `iColor` layers, `SVG ` glyphs, cached textures); a `CBDT` option in the export commands, the Export dialog and the CLI; `COLR` against fontc: 10 identical, 7 allowed, 0 different, against fontmake 11, 7, 0. images and colour in Font View cells; a colour Text Preview drawn from the compiled font (`COLR`, `SVG `, `sbix`); a UFO's full-colour layers as `colorLayers` paints, and JPEG and TIFF images saved as PNG; full-colour layers with paths, `PaintColrGlyph` and variable paints in UFO (10 of 10 round trips equal to glyphsLib's); `PaintColrGlyph` in compiled `COLR`; compressed `SVG ` documents, embedded images and `CBDT` in the colour Text Preview; image transforms, nudging, delete and PDF page sizes in Edit View; bitmap strikes in the model, with an `EBDT` and `EBLC` writer and strike rasterising; `PaintColrGlyph` references drawn in Edit View and the `SVG ` table; `SVG ` clip paths, masks, patterns and sliced images in the colour Text Preview; `SVG ` filters (every primitive but `feImage`) and the sliced root clip there, and filters in Edit View; PDF in paste and Import Outlines, the clipboard's PDF in Edit View, placed PDFs traced and drawn into bitmap strikes. `CPAL` version 1 and a dark palette derived in OKLab (M13.65); version 4 palettes written in older formats with a loss report (M13.65b); glyph artwork with shadows, inner shadows and pattern images (M11.21); bitmap glyph commands (M13.63). Missing: `feImage`, colour SVG artwork, the corpus `COLR` measure for `CPAL` version 1; nothing compared with the reference |
| Source formats | 6 | 16 | 16 | 72% | saves through the suite's atomic writer (a read-only file is refused and kept); Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; shape groups, gradients and shadows kept as stored values and typed in `.glyphs` 2 to 4 with exact `strokePos` conversion and loss reports on version 3 and UFO writes; hint references stored in file node order; UFO 1 sources open; EPS outlines and SFD sources read (SFD not yet in File › Open); right-to-left, vertical and contextual kerning in UFO; version conversion; a UFO's colour layer list read as palette layers and written back; CID-keyed fonts with the AFDKO UFO keys; BDF and PCF strikes imported into the model and exported; BDF, PCF and `.otb` fonts opened as fonts, and the strikes of compiled fonts read into the model; gzipped PCF and BDF, Windows FNT and FON, TeX PK and GF and Mac bitmap fonts read, and FNT, FON and PK written; `.glyphs` node flags in any order, one-line layer guides, and stale kept spellings left out on save; higher-order interpolation data in `.glyphs` version 4, the native file and UFO; File › Open… with a chooser, Open Folder, drop to open and project files routed to `project.open` (M1.8). nice names on import and production names in UFO export (M1.9); UFOZ save and format warnings from the save loss report (M1.11, X.fix82); path attributes in UFO glyph libs (M3.36). Missing: lenient reading and the semantic round trip (M1.10, M1.12) |
| Compiling | 9 | 13 | 13 | 44% | static TTF and OTF, plain, WOFF and WOFF2, from the commands and the Export dialog; production names and database mark widths; generated pair kerning in `GPOS`; linked metrics and Default Layer Width; strokes expanded, masks applied and overlaps removed (coverage-equal to fontmake's in over 99.5% of glyphs); core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source; `gasp` and the `SVG ` table; export profiles written all-or-nothing (Export Fonts As, glyph artwork); `avar` segment maps matching fontmake's; feature code compiled into the layout tables; the name table and OS/2 of static instances as fontmake builds them; TrueType glyph programs from manual instructions with `cvt `, `fpgm`, `prep`, `maxp` and `head` flags (M10.19); variable TrueType fonts (`glyf` from the origin, `gvar`, `fvar`, `avar`, `HVAR`, `MVAR`, `STAT`, names, feature variations, variable kerning and marks) equal to fontc's in 103 of 158 corpus sources and fontmake's in 106, with variable feature syntax (conditions, variation blocks, variable positions) compiled, and instances between masters, and variable fonts with CFF2 outlines (blended charstrings, specialised and subroutinised when smaller, blended hints, `HVAR`, `post` 2.0; equal to fontmake's `variable-cff2` in 106 of 158 sources); `COLR` and `CPAL` (versions 0 and 1, also variable), `sbix`, `CBDT` and `CBLC`, `SVG `, `MATH`, `BASE`, optical bounds, feature-code table blocks with `usMaxContext`, contextual anchors, TrueType autohinting at export, imported hinting written back, a glyph's own TrueType program, CFF subroutines in static exports (kept when smaller), designspace instance names and rules. an AFDKO project compile that gives makeotf's dump on 7 of 10 corpus UFOs, with makeotf's project keys (paths, licence code, OS/2 settings, UVS, CMap keys), CID-keyed CFF (13 of 13 keys equal), also from the ROS parameter, `EBDT` and `EBLC` from the model's strikes, `PaintColrGlyph` for components of colour glyphs, TrueType curve modes, implied on-curve points, pass-through tables and `.ttc` output (484 of 484 glyphs and 31 of 31 tables equal to the oracles); conversion between TrueType, CFF and CFF2 outlines; `cvar` in variable TrueType fonts and CFF2 hints of intermediate and alternate layers. the reference compilers' formats, output selection, feature, outline, CFF and glyph-set options (M2.8, M2.12, M2.13) and MakeOTF Compatibility Mode (M12.80), not yet measured against the oracles. Missing: `VVAR`, export-time parameters, the rest of the reference compilers' options (`docs/cli-options.md`), most of the other tables |
| Preview, proofing, printing | 2 | 26 | 22 | 20% | the Preview Area and Preview Panel (invert, flip, blur, offset), Text Preview with its text, style, size, direction and find, shaping through the preview font and UAX #9 bidi, sample strings; proof sheets as SVG and an HTML specimen; the Hinting Preview waterfall and a hinting web preview served on the local network under a random path; the Preview Area's instance pop-up. a colour Text Preview that draws colour glyphs from the compiled font. PDF proof sheets with the proof tool's layouts and spot sheets (M12.36, M12.78, M12.79); Export Window Contents as PDF or SVG (M13.68); External Preview in a browser for trusted devices (M13.71). Missing: printing, the PDF proofs' page counts against AFDKO's; nothing compared with the reference |
| Command-line tools | 8 | 56 | 53 | 45% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `oracle`; `tables` (extract, add, delete, edit, checksum, reorder, show), `collection`, `woff`, `dump` (the 17 core tables), `features` (normalize, list, decompile), `glyphs` (rename, reorder), `metrics`, `dehint`, `outlines`, `variations` (map, build-avar), `proof`, `sources`, `bench`, `completion`; `subset` with pyftsubset's option names (closure through `cmap`, components and `seac`; the core tables, equal to pyftsubset's in 337 or 338 of 340 corpus fonts per selection; layout closure and pruning, variation, colour and bitmap tables, and CFF and CFF2 with subroutine pruning and `--desubroutinize`: 909 layout runs equal wherever both tools run, 33 of 33 variation and colour, and 536 of 536 CFF runs equal in each of three modes); `check` with `outlines`, `family` and `interpolation` and the documented exit statuses; tested against small oracle outputs, the other corpus measures not yet run; `dump` with ttxn's normalised view and the `@font` and `@glyphs` views, and TTX writers for the CFF, CFF2, layout, variation, colour and miscellaneous tables (equal to ttx on 253 CFF, 700 layout and 143 other corpus tables, 0 different); `features` compile, lint and convert (VOLT equal; FontDame 24 of 28 equal, the other 4 written down); `diff`; curve conversion (qu2cu equal in 3,192, 0 different) and componentize; `autohint` and `stems`; the TTX reader (`tables import`: the core tables, CFF, CFF2 and `VORG`, equal to fontTools' `importXML` and save on 16 fonts except the CFF fonts' `head` and `hhea` bounds; the layout, variation, `COLR` and `SVG ` tables read back with all tags identical on 39 fonts; `COLR` version 1, the bitmap, Apple and miscellaneous tables and `cmap` format 2 read back with every tag identical, and `ttx -g` split dumps: 30 of 30 read back, 2,750 split files equal to ttx's); `instance` (static instances of variable TrueType fonts; 26 cases against fontTools' instancer, every table identical except `GPOS` in 18; instances of designspace and multi-master sources, CFF2 instances (every full instance and downgrade equal), and partial instances that pin, limit, move or drop axes, 9 equal of 38 cases against fontTools with 5 refused, 3 different and 21 failed inside fontTools); `optimize` (subroutinize, desubroutinize, CFF widths, implied on-curve points, `gvar` IUP, variation store, `COLR` reuse and `GPOS` compaction, equal to fontTools on the corpus but for one font fontTools crashes on); `variations` (`build-hvar`, `model`, `plot`, `plan-avar`); `merge` (TrueType and CFF fonts, with `MATH` and `BASE`; against fontTools 10 pairs merged by both and 2 refused by both, with 143 tables the same and 2 different); `convert` between TrueType, CFF and CFF2 outlines, bare tables and WOFF containers (CFF to TrueType 43,967 identical and 26 different; TrueType to CFF 36,466 the same and 203 different; CFF2 to CFF 43,993 equal); `compile` with an AFDKO project file and CID-keyed CFF; `transform` with `--upem` (`glyf` identical in 12 of 12); the CFF subroutiniser in `crates/cff` (233 corpus fonts, 0 outline differences, 0.785 of cffsubr's size). The eleventh batch: subset of `VARC` (14 of 14 equal to pyftsubset) and compressed `SVG `, pyftsubset's list options; TTX `mort` and `morx` (100 of 100 equal) and the bitmap image forms; convert from Type 1, bare CFF and TTX; transform of CFF and CFF2 (`--upem` as `scaleUpem`), matrices on CFF, CFF2 and variable fonts, `--glyph-table`, `--fd`, `--to`; partial instances of `avar` version 2 (5 of 5 equal), `VARC` and variable `COLR` fonts; `instance --layout-only`. This batch: variable TrueType fonts merged as variable fonts (the same as fontTools' instancer on the merged halves at 4 locations); the TTX bitmap writer forms and external image files (0 different from ttx); the last four pyftsubset options; `instance --interpolate-binary-layout` (`GPOS` identical to fontmake's on 3 of 3 sources); rotatefont matrices with 0 differences over 21,774 glyphs; Type 1, bare CFF and TTX input against tx 3,172 the same and 0 different. The thirteenth batch: `compile --interpolate-binary-layout` and `Unnamedfont-*` file names; convert to UFO, with `--glyphs`, GOADB, CID keying, Font DICT and `/CID` selection, `--location` and overlap removal; spot's table listings, the CFF literal view and `--exclude-glyphs` in `dump`; the instancer on `crates/cff` with unchanged output; `tables.recompileCff`. common options on every subcommand (M12.67); readable listings (M12.68); subsets by the converter's selection and of sources (M12.73, M12.74); mergefonts parity (M12.69); convert extras and tx's CFF switches (M12.75, M12.70); dump leftovers (M12.76); outline repairs on CFF (M12.66); PDF proofs (M12.36, M12.78, M12.79); autohint of compiled CFF and CFF2 fonts (M10.37); option prefixes in every parser (X.fix80); 672 of 770 reference options present (`cargo xtask cli-options`). Missing: merge of CFF2 and CID-keyed fonts; this batch's measures (subset of sources, mergefonts, proof page counts); the ttxn, sfntdiff, cu2qu, ttfcomponentizer and otfautohint measures are not run. The 45% is low on purpose while this batch's measures have not run; the second planning pass graded the behaviour before this batch at about 58% on its own scale |
| Quality checks | 3 | 47 | 41 | 35% | 34 of the 36 outline checks through `audit.run`; `audit.fix` applies eight fixes through path commands and each clears its check; live audit and canvas markers; 157 of M12's checks (components and anchors, font-wide metrics, names, master compatibility, variable-font tables, hinting, kerning and colour, files and sources, family) with their data sets; profiles, the ignore list, report export; the audit runner over the open document, sources (compiled in memory), compiled fonts, collections, WOFF and WOFF2 files and families, with preconditions, glyph selections and ignored results counted apart (3,676 corpus runs over 4 profiles and 919 files, no crash); the three shortcut commands with the reference tools' option names; check modules in `audit.run`, with `summary.errors`; the width, glyph-set, table, layout and CID checks; project values in the checks of compiled fonts; Font Info fixes as source commands and more outline steps through `audit.fix` (layer steps 7 to 33, with a compiled font's binary results fixed on the source masters); Erase Open Corners and Flatten Components as fix commands; the shaping checks run on the font's shaping; `audit.fix` on compiled font files, all-or-nothing with a dry run (M12.61); the Font Audit panel and the Audit Profile dialog (M12.64), with run ids, steps with progress and cancel, a fix preview and a report export (M12.64b); the Table Viewer (M12.38). `audit.fix` with the glyph-data steps, component steps, the third stage, the full contour order and families of files; `tables.recalculate` for `maxp`, `vhea` and the CFF FontBBox; `recompileCff` on compiled font files; a per-glyph cache of the layer checks on the open document; `checkOutlines` all and quiet, `compareFamily` reports. Missing: source-field names beyond the common fields; the outline checks give many warnings on decoded compiled fonts (to examine); verdicts agree with FontBakery's on the 44 pairs compared (the `fsSelection` and `mac_style` checks are merged by design), not yet over the whole corpus |
| Plug-ins and scripting | 3 | 23 | 23 | 25% | the Command Console, the `.fcmacro` format with the Script commands, and the URL scheme; the module runtime (wasm modules under wasmi with resource caps, hostile modules fail cleanly, filter output checked against the model's limits); module commands with Modules folder sync and safe mode; module filters in the export Filter and PreFilter parameters; check modules in the audit; dynamic Script, Filter and View menu items, reporter overlays and Palette forms; the Extension Manager with index lists and automatic updates (HTTPS in the desktop app); the Actions dialog; author documentation and a module template for each kind. Missing: installing a module through File › Open, web storage of modules |
| Agent surfaces | 4 | 11 | 11 | 62% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; tools, pointer, key, text, menu and dialog control over the control channel, enough to draw and export a font, to build one from glyph sets, and to type, space and kern text (three end-to-end tests); shortcuts sent with `ui.key` open their dialogs as the keyboard does; MCP permissions, workflow prompts, resources, completion and logging; read and write folder roots for MCP and the control channel; MCP over streamable HTTP (`--transport http --listen`); pointer, drag, wheel, view-switch, resize, focus and context-menu input at window points (M13.78). MCP window tools (screenshot, render, pointer, key, text, menus, dialogs) when MCP is attached to a running window (M13.80). Missing: an element list, waiting for idle and an event log (M13.79, M13.81) |
| **Feature parity** | **117** | **1039** | **960** | **~38%** | |

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
