# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-18 (this batch: 75 packages of M6 to M13 and their review fixes, built, integrated and gated: the TrueType autohinter and the Instructor, PostScript autohinting at export, COLR, CPAL, `sbix` and `SVG `, the axis, special-layer, instance and master commands and the interpolation windows, TTX writers, feature convert and compile, more quality checks and audit fixes, plug-in modules in the editor, display and drag aids, auto-spacing and auto-kerning)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,709 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, interactions, guard, CLI, control channel, MCP, UI shell, canvas, renderer, screenshots, pointer and key injection, toolbar, context menus, dialogs, panels, tests, atomic file writes through the Craft suite's shared library | **~65%** of what the siblings' "infrastructure" row covers; missing: the settings window, a performance budget suite |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~30%** |
| Commands that exist (not: that behave like the reference) | **852 of 985** (`docs/parity.md`) |
| Packages built, M6 to M13 | **218 of 277** (M6 19 of 20, M7 19 of 21, M8 18 of 25, M9 23 of 27, M10 31 of 33, M11 13 of 21, M12 43 of 68, M13 52 of 62; counted from commit subjects, M8.7b with M8.7; M9.8b is a follow-up of M9.8 and not counted) |
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
missed their targets (see "What is real: this batch").
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

**What is real: editing by command (M3).** Of the 985 commands, all 184 of M3, all 62 of M4 and all 97 of M5 exist.
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
| PostScript hinting | autohinting at export (M10.15); 11 hint commands (M10.4); hints drawn, selected and dragged in Edit View (M10.24); the Has Hints smart-filter rules and the Autohinting filter string (M10.31); CFF import keeps every declared stem |
| Colour | `CPAL` and `COLR` version 0 (M11.6) and version 1 (M11.7), `sbix` (M11.8), `SVG ` from svg layers and bitmaps (M11.10), `CPAL` and `COLR` read back into palettes and colour layers (M11.18), the palette editor, Master Color and Attributes inspector (M11.15); a UFO's colour layer list opens as palette layers. Palette colours are numbered in source glyph order, as both oracles number them |
| Interpolation | axis, special-layer, instance and master commands, queries and axis mappings (M9.15 to M9.18); special-layer labels and editors, the Master Compatibility view, the interpolation dialogs with the Variable Fonts export tab, Compare Fonts and Compare Family (M9.21 to M9.24); cross-axis mappings and Interpolate Fonts (M13.48) |
| Tables, TTX, CLI | TTX writers for CFF, CFF2, `VORG` (M12.5), the layout and variation tables (M12.6), the variable, colour and miscellaneous tables (M12.7), ttxn's normalised dump and the `@font` and `@glyphs` views (M12.10); `features compile`, `lint` and `convert` for FontDame and VOLT text (M12.13, M12.14); `diff` (M12.32); curve conversion and componentize (M12.29); `autohint` and `stems` (M12.39) |
| Checks and fixes | width (M12.49), glyph set (M12.47), table (M12.51) and layout (M12.52) checks; the CID check (M13.50); one source command per Font Info fix (M12.63) and every fix named by a command (M12.0); project values reach the checks of compiled fonts and `audit.fix` runs the Font Info fixes and more outline steps (X.audit1) |
| Modules and automation | module commands, Modules folder sync and safe mode (M13.3); module filters in export (M13.4); check modules in `audit.run` (M13.5); dynamic menus and reporter overlays (M13.6); the Actions dialog (M13.8); the Extension Manager (M13.10); MCP over HTTP (M13.12); author documentation and a template for each module kind (M13.14); PDF outlines and page images in `crates/io-pdf` (M13.58); BDF and PCF in `crates/io-bitmap` (M13.55) |
| Edit View and windows | component and corner Info boxes, smart settings windows, the Pixel tool (M6.11 to M6.13); the project window (M7.19); the Features tab (M8.17); display aids, drag aids, suggestions, auto-meter, power guides and the Magnet tool, Rapid and Spiro tools (M13.31, M13.33 to M13.37); Find and Replace Outline, the non-linear transform filter, autotrace, small caps and sub/superscripts, construction rules (M13.28, M13.29, M13.30, M13.19, M13.17) |
| Layout and shaping | the shaping crate on harfrust (M8.18; Edit View does not use it yet); `MATH` and `BASE` data (M13.47); optical bounds (M13.49) |
| Spacing and kerning | side profiles, auto-spacing, Center in Width and Copy Metrics (M13.20); auto-kerning, Kern to Distance, groups from side profiles, Remove Pairs (M13.21) |

Measures that met their targets: the TrueType autohinter's in-zone agreement 99.61% and size agreement 77.9%; the
construction rules build 196 of 200 measured glyphs and place 2,406 of 2,442 components within 1 unit; TTX against
ttx equals on 253 CFF, 700 layout and 143 other tables with 0 different; qu2cu equals on 3,192 with 0 different;
VOLT conversion is equal; `corpus_import` passes 27 CFF and 48 TrueType fonts; FontBakery's verdicts agree on 44
compared pairs (the `fsSelection` and `mac_style` checks are merged by design).

**Measures that missed their targets** (each has a tuning package under "What to build next"):

| Measure | Result | Target |
|---|---|---|
| TrueType autohinter, x-height rows (M10.21) | 61.7% agree with ttfautohint (it scales to the x-height; FontCraft does not yet) | 95% |
| TrueType autohinter, cap-height rows | 93.2% | 95% |
| Auto-spacing, median error (M13.20) | 23% to 96% by font | 10% (the speed target is met) |
| Auto-kerning, sign agreement (M13.21) | 74% to 97% by font; Inter Thin is below the target | 80% |
| FontDame conversion (M12.13) | 19 of 28 equal: 6 differ, and `gsubalternate` needs the AGL | all equal |

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

**What is not.** Nothing in the editor has been compared with the running reference app. M3's,
M4's and M5's behaviour comes from the handbook; 5 editor rows are confirmed by public tutorial videos. Where
a detail is unconfirmed, the handbook decides (Remove Overlap, for example, deletes open paths).
Where no source says, we choose, and each choice is listed in `docs/guesses.md` to revisit
(2,205 guesses on 1,334 rows).

| Gap | Detail |
|---|---|
| `path.sharpenCorner` | moved to the `fidelity` milestone: no written source says what it does, and the tutorial video shows the menu item without running it |
| Text mode | no shaping: text is laid out by code point, kerning and direction until features exist (M8); Text Preview orders right-to-left and left-to-right runs by simple rules of ours (full Unicode bidi at M8); Edit View keeps one direction per line |
| Spacing speed | on Inter, the out-of-sync update after an edit to one glyph takes 7.4 ms against a 2 ms target, and the full scan 53.5 ms against 50 ms; the per-edit follow-up hook takes 2.5 to 2.7 ms, almost all of it the alignment pass (2.4 ms), against 2 ms |
| Kerning | vertical kerning is stored and listed but not exported (neither reference compiler writes `vkrn`); automatically aligned glyphs wait for M6; vertical origin and vertical width have no commands; 12 metrics-key values in Inter Italic differ from the stored ones by up to 1.4 units, cause not settled |
| Masters | the toolbar has no master icons (M9) |
| Font View | images in cells wait for M11; filters cannot be renamed in place or dragged in the sidebar (they can by command); each automatic composite copies the glyph list, so adding very many composites at once is slow |
| Export dialog | OTF, UFO, Metrics and Variable Fonts tabs; variable fonts export with TrueType or CFF2 outlines through the dialog, `file.exportVariable` and `fontcraft-cli compile --variable` |
| Import Outlines | the missing-glyph question is asked once for all unmatched files, not once per file |
| TrueType instructions | the 14 hint commands edit stored instructions, and static exports and the default master of variable exports carry them as glyph programs (M10.19); variable fonts have no `cvar` or instruction deltas; the autohinter (M10.21) agrees with ttfautohint on 61.7% of x-height rows and 93.2% of cap-height rows (target 95%); the handbook's rounding-threshold example contradicts its default, and FontCraft follows the default |
| Hints | Shape Order does not renumber hints that refer to a path by index; hint upkeep (M10.3) does not renumber background layers or corner and cap hints |
| Strokes in files | a UFO keeps no stroke attribute or gradient (a save reports them); cap code 4 and the forced cap axis are not confirmed against the reference's files |
| Detect Composites | on Inter it finishes in under 2 s but rebuilds only 45.4% of the composites from the same bases (target 95%): it picks identical-outline bases such as `acute` for `acutecomb`, and composite bases instead of atoms |
| Audit | 34 of the 36 outline checks and the M12 checks (157 and the width, glyph-set, table, layout and CID groups of this batch) run through `audit.run` and `fontcraft-cli check`, with check modules and `summary.errors`; the outline checks give many warnings on decoded compiled fonts (not yet examined); no per-glyph hash cache; the reference tools' options are only partly taken (several `checkOutlines` and `compareFamily` options); `audit.fix` leaves the glyph-data fixes, component steps, stage 3 and file targets undone; `AuditServices::shape` is not wired, so the shaping-based checks skip; verdicts compared with FontBakery's on 44 pairs only |
| Subset | no CFF2 (refused), subroutine pruning or `--desubroutinize`; several pyftsubset options; no subsetting of sources; layout, variation and colour tables are dropped until M12.18 and M12.19 |

Exported CFF2 variable fonts carry no hints, subroutines or specialisation; variable TrueType exports have no instruction deltas or `cvar`, there are no `CBDT` or `CBLC` bitmap colour tables and no CFF subroutines; the tools and
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
| **Breadth:** commands, tools and panels exist | ~42% | 852 of 985 commands; 20 of 33 tool rows usable in the window (19 tools: every M3 tool row, the Annotation row being the group of its six tools, and M5's Text tool; the Thickness tool runs by `tool.select` and has no toolbar button); about 80 of 172 panels, many of them partial (Font View's grid, List View, sidebar and inspector, the toolbar, the Info box with its spacing and kerning fields, the Palette and five of its sections, the bottom bar, the Glyph Info, Selection Info, Start, Kerning and Text Preview windows, the Preview Area and Panel, the Font Info window with its tabs, the Command Console, the Related Glyphs, Selections and Reference Font sections, the M3 to M5 dialogs and the M13 dialogs) |
| **Depth:** features behave like the reference | ~11% | the M3, M4 and M5 commands, tools, panels and dialogs behave as the handbook describes them, and none has been compared with the reference app. The kerning lookup equals what fontc and fontmake compile. The variable export matches fontc's and fontmake's at the masters and between them for most corpus sources (outlines in 127 of the 135 sources compared, kerning and marks in all but a few), and static instances between masters match fontmake's in 169 of 234 compared. The CFF2 variable build equals fontmake's `variable-cff2` in 104 of 158 sources. The first waves of M10 to M12 match their oracles where tested (the autohinter AFDKO's scripts, the table tools small oracle outputs), but most of their work is not yet in exports or the window. The Font Info window and the M13 packages follow the handbook and the secondary references' public documentation, with 1,755 guesses listed. `path.sharpenCorner` waits in `fidelity`; the gaps are listed above |
| **File interop** | ~40% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFO 1 and UFOZ open; EPS outlines and, in `crates/io-sfd`, SFD sources can be read (SFD is not yet reachable from File › Open); compiled TrueType, CFF, collections, WOFF 1 and WOFF2 open (all 391 corpus fonts); variable TrueType and CFF2 fonts open into axes, masters and instances; Type 1 (PFA, PFB), bare CFF and SVG fonts open, and SVG fonts are written; static TTF and OTF out, plain, WOFF and WOFF2, from the export commands, the Export dialog and the CLI. Variable TrueType fonts and instances between masters are written. Paint attributes are typed in `.glyphs` 2 to 4; writing version 3 and UFO reports what they cannot hold. Missing: CFF2 variable fonts, version conversion, stroke attributes and gradients in UFO |
| **Compiler correctness** | measured, low | the core tables, `GPOS` (generated pair kerning and marks), `GDEF`, `gasp` and `SVG ` written for two static flavours (TrueType, CFF), plus the WOFF and WOFF2 containers; variable TrueType fonts with `fvar`, `avar`, `HVAR`, `MVAR`, `STAT`, `gvar`, feature variations, variable kerning and mark positions (`VVAR` is built but not exported); strokes, masks and overlap removal in the outline stage. Against fontc over 332 corpus sources and fontmake over 330 (TrueType) and 331 (CFF), no core table differs: each is identical, equivalent or allow-listed with a reason (fontc building variable fonts, bracket and colour glyphs, smart components (M6), fields where fontmake and fontc disagree and FontCraft follows fontc). 322 of 331 CFF tables decode to fontmake's outlines and 206 are byte-identical (`docs/plans/M2.md` § 5). With overlaps removed, 99.58% (TrueType) and 99.87% (CFF) of glyphs have no overlap-removal difference from fontmake's (`docs/plans/M3.md` § 5). The generated pair kerning equals both oracles' on every corpus source without `kern` feature code or a top-level include, but 27 known differences listed in `docs/spec/40-compile.md` § 4a (`docs/plans/M5.md` § 5). Layout tables are compared lookup by lookup (`crates/oracle`, `docs/plans/M8.md`): against fontmake, `GSUB` 25 of 26 sources equal, `GPOS` 82 of 88 and `GDEF` 47 of 55; against fontc, `GSUB` 29 of 46, `GPOS` 80 of 91 and `GDEF` 42 of 68. The remaining differences are bracket glyphs and colour glyphs (M11), each listed in `docs/spec/40-compile.md`. Variable fonts are compared with the same oracles (`fontcraft-cli oracle --variable`): against fontc 101 of 158 sources are equal, 37 differ and 20 fail; against fontmake 104 equal, 38 differ and 16 fail; per check, outlines 127 and 8 (fontc) and 128 and 11 (fontmake), `GSUB` variations 129 and 8 and 135 and 6, `GPOS` kerning 135 and 1 and 141 and 0, marks 134 and 2 and 139 and 2. fontc and fontmake agree with each other on 112 of 133 sources. Static instances between masters equal fontmake's `-i` build in 169 of 234 compared (41 differ, 24 fail in fontmake itself). The causes still open are listed under What is real: variable fonts and instances The name table and OS/2 of static instances equal fontmake's in 1,678 compared values and differ in 120, from three allow-listed causes (`docs/spec/40-compile.md`). The automatic feature code regenerated at export equals the reference compilers' in 69 of 95 compared entries; 23 differ (stale sources, older reference forms, sample fonts) and 3 are removed. CFF2 variable fonts (`--variable --format otf`) are compared with fontmake's `variable-cff2`: 104 equal, 34 different and 20 failed of 158 sources; every outline and advance difference also shows in the TrueType build against fontmake, and fontTools reads all 153 outputs without warnings. No hints, subroutines or specialisation in CFF2 and no CFF subroutines in exports. TrueType exports carry the glyph programs of manual instructions (M10.19: 0 interpreter errors over every exported glyph of the instructed corpus samples at 8 to 72 PPM), without `cvar` or an autohinter |
| **Agent automation** | ~58% | every existing command is reachable from CLI, control channel and MCP; MCP has permissions, five workflow prompts, resources, completion and logging, and MCP and the control channel can be confined to read and write folders; the control channel's 19 methods drive the window's tools, pointer, keys, text fields, dialogs and menus; `ui.screenshot` and `ui.render` show the result; an agent can draw, edit and export glyphs, build a font from glyph sets, and type, space and kern text, over the control channel alone (three end-to-end tests). MCP also runs over streamable HTTP (`--transport http --listen`). Missing: `ui.click` beyond dialogs, `ui.set` |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests, hostile SVG, a random-path test of every path operation, corpus runs of overlap removal and the audit); dependency failures found and fenced: the glyph writer's coordinate panic, the SVG parser's stack overflow on deep nesting, linesweeper's NaN panic. linesweeper also hangs on rare inputs (`docs/plans/M3.md` › M3.6); it is kept from crates.io, and the path-operation commands and export run each glyph on a worker thread under a time limit (a glyph that runs out of time stays as it was; the worker cannot be stopped and runs on detached). Adversarial code reviews of the skeleton and of each milestone's first wave, every blocking finding fixed with a test and the rest listed in `docs/status.md`. Later fix passes bounded the work of the EPS, SFD, UFO 1 and PFM readers, features previews, macros, outline matching, intersections and stem filters, and made multi-profile exports all-or-nothing. No fuzzing, and the known scaling limits in `docs/status.md` |
| **Performance** | a few measures | overlap removal over the corpus: median 38 µs, 99th percentile 403 µs per glyph layer; one node-drag preview step on Oswald: 142 µs; on Inter (3,017 glyphs) a Font View frame under 1 ms, a frame after an edit 8–10 ms (15–19 ms with every core busy), every glyph's effective properties 21–26 ms, a three-rule smart filter 28–35 ms, the first glyph-database lookup 24 ms; the kerning lookup over 490,911 pairs 39 ms, Update Metrics on all glyphs and masters 85 ms, a text-mode frame with 200 glyphs 5 ms, Text Preview layout of 1,000,000 characters about 210 ms; the out-of-sync update after one edit 7.4 ms, over its 2 ms target; the colour-layer update after a one-glyph edit 35.5 µs; console completion at worst 2.28 ms (target 5 ms, met); the per-edit follow-up hook 2.5 to 2.7 ms (target 2 ms, missed; the alignment pass is 2.4 ms of it); Detect Composites on Inter under 2 s (met); an Edit View text-mode frame with 200 composites 5.9 ms (target 16 ms, met). No budget suite for the targets in `docs/architecture.md` |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: 24 crates build for the browser; no web app |
| **Look and feel** | barely started | a functional shell with the siblings' default dark tokens; the toolbar, panels, dialogs and canvas overlays take their colours and sizes from the tokens, with drawn icons for the tools, the Transformations buttons and the Stroke section; Edit View follows the reference's node and handle display conventions as the handbook describes them; nothing measured against any reference |

### What to build next (in priority order)

1. **Tuning of the measures that missed** (each was measured and is tracked in "What is real: this batch"):
   - M10.21b: the TrueType autohinter's x-height scaling in `prep` (x-height rows 61.7%, cap-height 93.2%,
     target 95%), composite and path regression tests, left and right offsets and horizontal shifts.
   - M13.20: auto-spacing's median error (23% to 96% against 10%). Measure: `cargo test --release -p
     fontcraft-engine --test engine corpus_autospace:: -- --ignored --nocapture`.
   - M13.21: auto-kerning's sign agreement (74% to 97%; Inter Thin below 80%). Measure: `corpus_autokern::`.
   - M12.13b: FontDame conversion (19 of 28 equal: 6 differences and `gsubalternate` through the AGL).
2. **The remaining packages of each milestone**, in the order the plans' dependencies allow:
   - M6.19; M7.20; M8.8, M8.19, M8.20, M8.21, M8.23, M8.24; M9.20, M9.25, M9.26; M10.32.
   - M11.11, M11.13, M11.14, M11.16, M11.17, M11.19, M11.20.
   - M12.1a to M12.1c, M12.8, M12.9, M12.18 to M12.26, M12.30, M12.34, M12.36, M12.38, M12.40 to M12.42,
     M12.61, M12.62, M12.64, M12.65.
   - M13.22, M13.23, M13.40 to M13.42 (caps: trial), M13.51, M13.52, M13.59 (waits for a trial sample of a
     saved Glyphs project), M13.61. M13.60 is dropped.
3. **Follow-up packages the writers named:**
   - M12.7b: TTX for the bitmap tables, `COLR` version 1 and the Apple tables. M12.10b: spot listings.
   - M13.55b: bitmap strikes in the model (BDF and PCF import and export commands).
   - `io-pdf` wired into the engine (clipboard and image decoding).
   - X.audit1 leftovers: glyph-data fixes, component steps, stage 3 and file targets of `audit.fix`.
   - `AuditServices::shape` (needs M8.19), which the shaping-based checks wait for.
   - `edit.showAllMasters` (needs a tab model with a layer per text item).
   - Small UI leftovers: Shift+B cycling from Pencil to Pixel; the Smart windows from the inspector's Cmd+Alt+I;
     the Instances tab's Copy and Paste hooks; theme tokens for the new aids; menu checkmarks for the suggest
     switches; arrow-key nudge of hints; Instructor badge drag, extra nodes and Delete on instructions;
     `plugin.installFromFile` from a dropped file; web storage of modules.
4. **The builder packages the earlier waves left**, one at a time in the main checkout: M11.5b (full-colour layers
   in UFO and JPEG and TIFF to PNG for UFO images), M12.36 (the PDF proof through the suite's `craft-print`). Then the
   Craft suite's shared libraries (`docs/decisions/0008-shared-suite-libraries.md`), in this order:
   - X.suite2: `craft-quit`, native macOS quit and window-close handling for the desktop app.
     The save prompts stay FontCraft's.
   - X.suite3: `craft-settings`, settings storage in `config.toml` with backup and recovery,
     with a one-time import of FontCraft's existing JSON settings file. It pairs with the
     settings window.
   - Watch item: the suite's proposed shared control-port authentication (`craft-control`, an
     open upstream proposal). Adopt it for the control channel once it exists.

   Later: `craft-ui` in M14.

   Follow-ups: `cvar` and instruction deltas in variable TrueType fonts and special layers' own instructions
   (M10.19); hints, subroutines and specialisation in CFF2; the variation of TrueType
   composites with intermediate layers; variable values in table blocks (`fea.var.tableValues`: `BASE`, `GDEF` carets,
   `hhea`, `OS/2`, `vmtx`; fea-rs rejects them); the extras a kept `.glyphs` layout carries going
   stale after an edit (stale kept-layout extras); hint upkeep on background layers and for corner
   and cap hints; the audit runner's hash cache, remaining options and width checks, and a look at the outline
   warnings on decoded compiled fonts; subset for CFF2, subroutine pruning and `--desubroutinize`.
5. **The Detect Composites base choice (M13.15 follow-up):** prefer `*comb` bases among bases
   with identical outlines, and never use a detected composite as a base, to lift the recovery
   from 45.4% toward the 95% target. Also bring the per-edit alignment pass (2.4 ms on Inter)
   under the 2 ms target.
   Also: M8.11 (the ligature, fraction and aalt generators, which the automatic feature
   framework waits for), token expansion in `plan::postscript_name` and `plan::file_stem`, and
   selecting and deleting corner components in Edit View (needs a hint selection kind).
   Variable-font follow-ups: designspace instances taking names, `usWeightClass` and font info
   from `<instance>` (the main cause of the remaining instance differences), designspace
   `<rules>`, alternate layers with all bounds empty, fontc's combinatorial bracket kerning and
   decreasing-axis tables, brace sources for composites of non-exported brace glyphs, vertical
   phantom points and `VVAR`, and `OVERLAP_COMPOUND`.
6. **The measures**: M6's (`docs/plans/M6.md` § 5: mark attachment against the oracles,
   propagation against glyphsLib, composition and alignment over the corpus) and the corpus
   measures not yet run: ttxn and tx for the normalised dump, sfntdiff for `diff`, cu2qu and ttfcomponentizer,
   otfautohint for the `autohint` shell, pyftsubset for layout and variation tables, FontBakery and comparefamily
   over the corpus. M9's, the TTX writers', qu2cu's and the colour tables' are run.
7. **Shell hooks for finished M13 packages**: dialogs and menu entries several commands wait for
   (for example the metrics-file import chooser, Status Definitions, the name-list rename).
8. **Checks in the reference app**: `docs/observe.md` lists every row the running app could
   settle (1,296 rows), with the synthetic test fonts `cargo xtask testfonts` writes.

Left from M3 to M5 for later milestones: the toolbar's master icons and the
Export dialog's Variable tab (M9). Left from M2: CFF
subroutines (before M12), the compile plan's `features` section, and the CLI's `--name-by`, `--output-layout` and pattern instance
selection (`docs/plans/M2.md` › M2.5).

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (985 commands, 33 tools, 182 panels, 47 settings, 241
  parameters, 1,749 behaviour rules, 300 checks, 74 CLI rows, 48 formats, 50 tables) and 23
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
| M6 | Components and anchors | 🟨 first wave built: the component index, propagation, automatic alignment, composition, smart and corner components drawn and exported, the mark writers; the component, anchor, smart glyph, corner component and decompose commands; metrics of aligned composites. Second wave: Add Glyphs recipes and composition, and the Font View batch actions on components and anchors (M6.14). Edit View display and selection of components and anchors (M6.10). Later packages: component and corner Info boxes, glyph pickers and Component from Selection's dialog (M6.11), the Smart Glyph and Smart Component settings windows with the Palette's Smart Component section (M6.12), the Pixel tool (M6.13). Left: selecting and deleting corner components (needs a hint selection kind), M6.19 (close) and the measures (`docs/plans/M6.md`) | 47 (47) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | 🟨 first wave built: the parameter catalogue, Font Info fields at every scope, masters, instances, metric and stem definitions, scaling to a new UPM, export steps (filters, glyph sets, export folders), linked sources, text tokens, the legacy kern table. Second wave: projects (M7.14) and the Font Info window with its Font, Masters, Exports, Other Settings and Notes tabs (M7.15 to M7.18). The name table and OS/2 of static instances as fontmake builds them (M7.8). The project window with the instance list, instance editor and export controls (M7.19). Left: text-token expansion in PostScript and file names of instances, the Instances tab's copy and paste hooks, M7.20 (close) (`docs/plans/M7.md`) | 66 (66) |
| M8 | OpenType features | 🟨 first wave built: the fea-rs wrapper with its guards, assembly with a source map, the syntax extensions, number, property and predicate tokens, 60 automatic feature generators, the feature entry commands, feature file import and export, feature code snippets. Second wave: the compile state (M8.15), with errors, fixes, completion and token evaluation. Builder packages: the automatic feature framework with Update and regeneration at export (M8.9), and layout tables at export compared lookup by lookup with fontmake and fontc (M8.7, M8.7b). Later packages: the Features tab with its code editor (M8.17), the shaping crate on harfrust (M8.18; Edit View does not use it yet), `MATH`, `BASE` and optical bounds (M13.47, M13.49). Left: the ligature, fraction and aalt generators (M8.11), table blocks and `usMaxContext` (M8.8), Edit View shaping and the Features menu (M8.19), M8.20, contextual anchors (M8.21), layout reconstruction from compiled fonts (M8.23), M8.24 (close) (`docs/plans/M8.md`) | 41 (36) |
| M9 | Interpolation and variable fonts | 🟨 first wave built: axis ranges, mappings and normalisation as the oracles derive them; glyph sources and the compatibility check; `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, the variable-font names and `STAT`, byte-equal to fontc's on test sources; feature variations from alternate layers; variable TrueType and CFF2 fonts open; the compare commands. Builder packages: the oracle harness for variable fonts and instances (M9.10), static instances between and beyond masters (M9.4), the variable TrueType compile with `file.exportVariable` (M9.5), variable kerning and marks (M9.9), alternate layers in variable fonts (M9.8b), and the reference's variable feature syntax lowered as glyphsLib does and compiled on fea-rs (M9.11, M9.12). Variable TrueType fonts equal fontc's in 101 of 158 corpus sources and fontmake's in 104; variable CFF2 fonts (M9.13: blended charstrings, no hints or subroutines) equal fontmake's in 104; instances equal `fontmake -i` in 169 of 234. Later packages: axis commands (M9.15), special-layer commands and editors (M9.16, M9.21), queries and axis mappings (M9.17), instance and master commands (M9.18), the Master Compatibility view (M9.22), the interpolation dialogs with the Variable Fonts export tab (M9.23), Compare Fonts and Compare Family (M9.24), and cross-axis mappings (M13.48). Left: hints, subroutines and specialisation in CFF2; variable values in table blocks; designspace instance info and `<rules>`; drawn instances (M9.20), M9.25, M9.26; showing all masters in a tab (`edit.showAllMasters`) (`docs/plans/M9.md`) | 52 (47) |
| M10 | Hinting (PostScript and TrueType) | 🟨 first wave built: the PostScript autohinter ported from otfautohint, agreeing with AFDKO 5.0.1 on its oracle scripts' cases, with reports and override sets; typed hint data and settings; the TrueType assembler; `gasp` at export; hinted previews. Builder packages: node-reference upkeep (M10.3: hint ends and instruction points follow their nodes through edits, in the command's undo step) and CFF hints at export (M10.14: stem hints, masks, flex, manual hints per master, family zones). Builder packages M10.17 (the TrueType font programs: `cvt `, `fpgm`, `prep`, tested in skrifa's interpreter) and M10.6 and M10.7 (the touched-state model and 14 instruction-editing commands). Builder packages M10.18 and M10.19 (glyph programs compiled from manual instructions and written into static and variable TrueType exports, tested in skrifa's interpreter). Later packages: PostScript autohinting at export (M10.15) and 11 hint commands (M10.4); the TrueType autohinter (M10.21), whose x-height rows (61.7%) and cap-height rows (93.2%) agree with ttfautohint below the 95% target; import of hinted fonts (M10.22); hints in Edit View (M10.24); zone and stem commands and editors (M10.5, M10.28); the Instructor tool with its Info box, pixels and Hinting Preview (M10.25 to M10.27, M10.29); the web preview (M10.30); hint smart-filter rules (M10.31). Left: M10.21b (tuning), M10.32 (close), `cvar` for variable fonts; upkeep on background layers and for corner and cap hints; TrueType assembly and the debugger (M13.51, M13.52) (`docs/plans/M10.md`) | 57 (57) |
| M11 | Colour fonts and images | 🟨 first wave built: colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export. Builder package M11.5, part 1: paint attributes typed in `.glyphs` versions 2 to 4, with version conversion of stroke positions and loss reports on writing version 3 and UFO. Later packages: `CPAL` and `COLR` version 0 (M11.6) and version 1 (M11.7), `sbix` (M11.8), the `SVG ` table from svg layers and bitmaps (M11.10), the palette editor (M11.15), `CPAL` and `COLR` read back (M11.18), and a UFO's colour layer list as palette layers. `COLR` against fontc: 10 identical, 7 allowed, 0 different; against fontmake: 11, 7, 0. Left: M11.5b (full-colour layers as `colorLayers`, JPEG and TIFF to PNG for UFO images), `CBDT` and `CBLC` (M11.11), images in Edit View (M11.13), M11.14, M11.16, colour Text Preview (M11.17), variable `COLR` (M11.19), M11.20 (close) (`docs/plans/M11.md`) | 41 (39) |
| M12 | Command-line tool suite and quality checks | 🟨 first wave built: table actions and `tables show`, collections, WOFF, TTX dumps of the core tables, the layout normaliser, feature list and decompile, glyph rename and reorder, the metrics report, dehint, outline repairs on whole fonts, Type 1, bare CFF and SVG fonts, variations map and build-avar, SVG and HTML proofs, source diff and merge, `bench` and `completion`; 157 checks, audit profiles, the ignore list and report export. Builder packages M12.44 (the audit runner over sources, compiled fonts and families, with `fontcraft-cli check`; 3,676 corpus runs, no crash) and M12.17 (subset by pyftsubset's options; core tables equal to its in 337 or 338 of 340 corpus fonts per selection). The other corpus measures have not been run. Later packages: TTX writers for CFF, CFF2, layout, variation, colour and miscellaneous tables (M12.5 to M12.7; against ttx 253, 700 and 143 equal, 0 different), the normalised dump (M12.10), feature convert (M12.13; FontDame 19 of 28 equal, a miss) and compile (M12.14), qu2cu conversion and componentize (M12.29), diff (M12.32), `autohint` and `stems` (M12.39), the width, glyph-set, table and layout checks (M12.47, M12.49, M12.51, M12.52) and the fix commands (M12.0, M12.63). Left: M12.1a to M12.1c, the TTX reader (M12.8, M12.9), subset layout, variation and colour tables (M12.18, M12.19) and instance, merge, transform, subroutiniser, optimize (M12.20 to M12.26), M12.30, M12.34, the PDF proof (M12.36), the Table Viewer (M12.38), M12.40 to M12.42, fixes on compiled fonts and glyph fixes (M12.61, M12.62), the Font Audit panel (M12.64), M12.65 (close) (`docs/plans/M12.md`) | 90 (64) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | 🟨 first wave built (22 packages): curve tension, corner and geometry helpers, stem detection and filters, the Thickness tool, expressions, Detect Composites, stored selections and the reference font, status definitions, export profiles and artwork, external-change review, encoding and name-list tools, PFM and AFM import, UFO 1, EPS and SFD reading, the Command Console, macros, the URL scheme, MCP permissions, prompts and resources. Secondary-reference documentation only, nothing observed. Builder package M13.2, the module runtime (wasm modules under wasmi with resource caps; 18 hostile-module cases fail cleanly), is built and called by the module commands, export filters and check modules. Later packages: module commands, module filters in export, check modules, dynamic menus and the Actions dialog (M13.3 to M13.6, M13.8), the Extension Manager (M13.10), MCP over HTTP (M13.12), author documentation (M13.14), constructions (M13.17), small caps (M13.19), auto-spacing and auto-kerning (M13.20, M13.21; both missed their measures), Find and Replace Outline, the non-linear filter and autotrace (M13.28 to M13.30), display and drag aids, auto-meter, suggestions, power guides, Rapid and Spiro (M13.31, M13.33 to M13.37), `MATH`, `BASE`, cross-axis mappings, optical bounds, CID-keyed fonts (M13.47 to M13.50), BDF and PCF readers (M13.55; bitmap strikes in the model are M13.55b), PDF reading (M13.58). Left: M13.22 and M13.23 (pair lists, Space Center), M13.40 to M13.42 (caps, waiting for a trial), TrueType assembly and the debugger (M13.51, M13.52), Glyphs project files (M13.59, waiting for a trial sample), M13.61 (close; M13.60 shared editing is dropped), the remaining editing and glyph-data packages, the shell hooks several finished packages wait for (toolbar buttons, dialogs, menus), and SFD wired into File › Open; Detect Composites rebuilds 45.4% of Inter's composites from the same bases against a 95% target (`docs/plans/M13.md`) | 190 (133) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web. Adopts the suite's `craft-ui` (docking, tabs, icon buttons, widget-state theme; decision 0008) | ⬜ | 34 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default; milestone `fidelity` in the inventory) | ⬜ | 4 (0) |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 985 (852) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 45 | 27 | 22% | tabs, zoom and scroll work; user settings kept between runs; the Start Window with recent documents; the toolbar with shared buttons and their menus; tools chosen by their letters, shared letters cycle; context menus; menu items greyed out by the selection; bottom bar zoom controls; the Command Console; the external-change review on window focus; the project window. Missing: settings window, shortcut editor, workspaces, title bar, sidebars, the toolbar's master icons (M9); nothing compared with the reference |
| Font View | 6 | 68 | 62 | 41% | grid, List View, sidebar with categories, languages and filters, search, the glyph database, Add Glyphs, renaming with references, the inspector, Selection Info, Glyph Info, the Start Window, the glyph set dialogs, the inspector's metric and kerning-group fields and the out-of-sync badge; Add Glyphs recipes and composition, batch component and anchor actions, five component smart-filter rules; Filter by Encoding File and Compare Fonts in List; the Master Compatibility corner and the Incompatible masters filter; hint smart-filter rules. Missing: images in cells (M11), the rename dialog for name lists; nothing compared with the reference, and many choices are guesses |
| Edit View text mode | 4 | 25 | 19 | 40% | typing, the Text tool, cursor keys, selection, clipboard as text, three directions, three kerning modes, kerned-gap colours, metrics labels, tabs from display strings, the Preview Area and Panel, the measurement line, Text Preview, sample strings, Find Glyphs. Missing: shaping in Edit View (the shaping crate exists; M8.19 wires it), Latin letters inside right-to-left runs in Text Preview; nothing compared with the reference |
| Drawing and editing outlines | 10 | 98 | 97 | 31% | node, segment, transform, clipboard and selection commands; in the window Select, Lasso, Hand, Zoom, Draw, Pencil, Rectangle, Ellipse, Rotate and Scale, the bounding box and its knobs, context menus, the Info box, the Palette's Transformations and Fit Curve sections, the Transformations and Primitive Measurements dialogs, editing in any master; curve tension and harmonizing, corner and geometry helpers, the Thickness tool (by command), stored selections, the reference font; Pixel, Rapid, Spiro and Magnet tools; display aids (Tunni lines, curve tension, indexes, curve length, ghost outline, Lock Outline, metrics beam); drag aids (Tunni drags, sliding nodes, the lever, Power Nudge, servant nodes); snapping suggestions; Find and Replace Outline. Missing: a toolbar button for the Thickness tool, beam dragging, menu checkmarks for the suggest switches, `path.sharpenCorner` (no source, in `fidelity`); nothing compared with the reference |
| Path operations and filters | 6 | 68 | 63 | 33% | Remove Overlap, the booleans, path direction, extremes and inflections, Tidy up, Open Corner, Reconnect, Cut with Line, curve conversion and rounding as commands; the Knife; the six filters with dialogs and export strings; strokes and masks drawn on the canvas and applied at export; the Stroke section and the Shape Order dialog; stem detection and the four stem filters with dialogs; ink traps, Make Parallel, Space Points, Make Arc, Slice Along Guide; the non-linear transform filter; autotrace of a placed image into outlines. Missing: stroke attributes in UFO, hints renumbered after Shape Order and Expand Outline; nothing compared with the reference |
| Reusing shapes | 7 | 70 | 53 | 19% | the component index, anchor propagation, automatic alignment after every edit, composition data, smart and corner components drawn and exported; component, anchor, smart glyph, corner component and decompose commands; derived metrics of aligned composites; Add Glyphs recipes; Detect Composites and Detect Component (45.4% of Inter's composites rebuilt from the same bases against a 95% target), anchor expressions and automatic layers, Show Dependencies, Related Glyphs. Edit View draws components by alignment state, placeholders, anchors and corner tints, and its clicks, rubber band, Select All, Tab, double-click, Option-drag, Delete, smart handles and size knobs run their commands; component and corner Info boxes, glyph pickers, Component from Selection's dialog, the Smart Glyph and Smart Component settings windows, Build Glyphs from Constructions (corpus measure: 196 of 200 glyphs, 2,406 of 2,442 positions within 1 unit), small caps and sub/superscript glyphs. Missing: selecting and deleting corner components, the rest of the M6 UI packages (M6.19); nothing compared with the reference |
| Layers | 3 | 21 | 21 | 37% | background and layer commands; Edit Background; the active layer and the tab's master; the background and other visible layers drawn on the canvas; the Palette's Layers section with its row context menu; the Interpolate with Background dialog. Font View's master choice; bold bracket and brace labels in the Layers palette, with a double-click opening the special layer editor. Missing: nothing compared with the reference |
| Guides, metric lines, zones, grid | 3 | 40 | 40 | 30% | guide, ruler, coordinate and Dimensions commands; metric lines, guides, the grid, CJK lines and measurement guides drawn; guides dragged and edited on the canvas, with their context menu and the Guide Properties dialog; the Measurement tool; snapping to guides and their crossings; the Palette's Dimensions section; power guides (make, add from stems, unlink, move with proportional spacing) and the Magnet tool; the auto-meter and the Measurements Palette section; snapping suggestions. Missing: script-specific metric lines; nothing compared with the reference |
| Spacing and kerning | 7 | 80 | 75 | 44% | metrics keys, sidebearing and width setters, the spacing and kerning shortcuts in text mode, Update Metrics, Transform Metrics, kerning pairs, groups, exceptions and locks, Clean Up, Compress, group rename; the Info box's spacing and kerning fields, the Kerning window, Show Group Members, the Font View inspector fields and out-of-sync badge, metrics import and export (kerning-text, AFM, PFM), kerning read from compiled fonts; generated pair kerning in exported fonts, the lookup equal to fontc's and fontmake's compiled values over the corpus; side profiles, auto-spacing, Center in Width and Copy Metrics, the metrics beam on the sidebearing setters; auto-kerning, Kern to Distance, kerning groups from side profiles, Remove Pairs, vertical kerning from horizontal. Missed measures: auto-spacing's median error is 23% to 96% by font against the 10% target (its speed targets are met), and auto-kerning's sign agrees with the sources' in 74% to 97% of pairs, below the 80% target on Inter Thin. Missing: vertical kerning at export, automatically aligned glyphs (M6), vertical origin and width commands, the out-of-sync update at its speed target; nothing compared with the reference; many choices are guesses |
| Font Info | 6 | 60 | 60 | 33% | fields at font, master and instance scope, added and localized fields, row copy and paste; the custom-parameter catalogue with value checks; masters, instances, metric and stem definitions; scaling to a new UPM; filters, glyph sets and export folders at export; linked sources; text tokens; projects; the Font Info window (Cmd+I) with its Font, Masters, Exports, Other Settings and Notes tabs, shared row editing, and field and parameter pickers; status definitions; the name table and OS/2 of static instances from these fields, as fontmake builds them, with the version stamp; the Features tab (code editor with colouring, suggestions and error markers, Add Feature picker, snippets menu, stylistic-set names); dialogs for the Axis Location, Virtual Master, Axis Mappings, Color Palettes, Master Color and Master Color Dark values; the project window. Missing: the Instances tab, text-token expansion in instance PostScript and file names; nothing compared with the reference |
| Interpolation | 8 | 73 | 55 | 23% | Edit View draws and edits any master (`view.selectMaster`); axis ranges, mappings and normalisation; glyph sources and the compatibility check; the variable tables and `STAT`; variable fonts opened; feature variations and bracket glyphs from alternate layers; static instances between and beyond masters (alternate and intermediate layers, kerning, manual factors, Local Interpolation) equal to fontmake's in 169 of 234 compared; variable fonts compared with fontc and fontmake at masters, instances and midpoints; variable feature code compiled into variable fonts and resolved in static instances; the compare commands; axis commands, special-layer commands and editors, interpolation queries and axis mappings, instance and master commands, the Master Compatibility view with its Incompatible masters filter, the interpolation dialogs with the Variable Fonts export tab, the Compare Fonts and Compare Family windows, cross-axis mappings and Interpolate Fonts. Missing: designspace instance info and `<rules>`, the master icons, drawn instances (M9.20), M9.25, M9.26, showing all masters in a tab (`edit.showAllMasters`) |
| OpenType layout | 7 | 48 | 44 | 21% | feature code assembled with a source map, the reference's syntax extensions, number, property and predicate tokens, 60 automatic feature generators, feature entry commands, feature file import and export, snippets, the fea-rs wrapper with guards; feature compile state with errors, four fixes, completion and token evaluation; layout features listed and decompiled from compiled fonts; the `mark`, `mkmk`, `abvm`, `blwm` writers and `GDEF`; the automatic feature framework with Update and regeneration at export; feature code compiled into `GSUB`, `GPOS` and `GDEF` in exports, equal lookup by lookup to fontmake's in most corpus sources; the reference's variable feature syntax (conditions, variation blocks, variable positions and number tokens) lowered as glyphsLib does, compiled into feature variations and variable positions in variable fonts and resolved at the location in static ones; the Features tab's code editor; the shaping crate (harfrust with cached plans, private-use glyphs, live advances, bidi and script runs; Edit View does not use it yet); optical bounds (`lfbd`, `rtbd`); `MATH` and `BASE` data. Missing: the ligature, fraction and aalt generators (M8.11), table blocks and `usMaxContext` (M8.8), contextual anchors (M8.21), layout reconstruction from compiled fonts (M8.23), shaping in Edit View and the Features menu (M8.19), variable values in table blocks, the layout of colour fonts |
| PostScript hinting | 3 | 26 | 26 | 20% | typed hints and hint settings in every stored form; the autohinter ported from otfautohint with its reports and override sets, agreeing with AFDKO's oracle scripts; hinted previews; dehint of compiled fonts; hint ends staying on their nodes through edits (M10.3); stem hints, masks, flex, manual hints per master and family zones in CFF exports (M10.14); autohinting at export (the outline as the charstring draws it, masks on closing lines); 11 hint commands (add, ghost, values, links, delete, clear, list, font hints); hints drawn, selected and dragged in Edit View with the hint Info box and context-menu items; import of hinted fonts keeping every declared stem; the Has Hints smart-filter rules and the Autohinting filter string. Missing: upkeep on background layers and for corner and cap hints, arrow-key nudge of hints; nothing compared with the reference |
| TrueType hinting | 4 | 41 | 37 | 22% | typed instructions in every stored form, the assembler, `gasp` at export, hinted previews, instruction points staying on their nodes through edits (M10.3); the touched-state model and 14 commands that edit instructions (Snap Point, Set Snap Zone, Stem, Reverse, Set Stem Reference, Make Triple Hint, Set Instruction Rounding, Shift Points, Interpolate, Delta, Set Delta Cells, Delete Instructions, Remove Hints, List TrueType Instructions), each one undo step; the font programs `cvt `, `fpgm` and `prep` for zones and stems, tested in skrifa's interpreter (M10.17); glyph programs compiled from the manual instructions against the exported point map and written into static and variable TrueType exports (M10.18, M10.19; `hint.tt.getPointIndexes`), tested in skrifa's interpreter and read by fontTools without warnings; the autohinter (M10.21: autohinted export, `autohint.tt.run`, `hint.tt.autohintGlyph`, TTFAutohint options and control instructions; in-zone agreement 99.61% and size 77.9% meet their targets, but x-height rows agree with ttfautohint in 61.7% and cap-height rows in 93.2% against 95%); an external ttfautohint binary runs only when named by the `export.ttfAutohintBinary` setting or `--ttfautohint`; import of hinted fonts, written back at export; zone and stem commands and editors; the Instructor tool with badges, direction, pixel size and point numbers, its Info box and Delta table, blue and red outlines on the pixel backdrop; the Hinting Preview waterfall; the web preview. Missing: `cvar` and instruction deltas in variable fonts, the autohinter's x-height scaling (M10.21b), special layers' own instructions, Instructor badge dragging and extra nodes, TrueType assembly and the debugger (M13.51, M13.52); nothing compared with the reference |
| Colour fonts and images | 3 | 47 | 38 | 20% | colour palettes, colour layer commands, path attributes of full-colour layers, image commands with decoding under limits, the `SVG ` table from vector layers at export; paint attributes (colours with palette references, gradients, shadows) typed in `.glyphs` files; `CPAL` and `COLR` version 0 and version 1 (paint runs, gradients, strokes outlined as layer glyphs, clip boxes), `sbix`, the `SVG ` table from svg layers and bitmaps, `CPAL` and `COLR` read back into palettes and colour layers, the palette editor, Master Color picker and Attributes inspector, a UFO's colour layer list as palette layers; `COLR` against fontc: 10 identical, 7 allowed, 0 different, against fontmake 11, 7, 0. Missing: `CBDT` and `CBLC` (M11.11), images in Edit View (M11.13), colour Text Preview (M11.17), variable `COLR` (M11.19), paint attributes in UFO (M11.5b), colour drawing in Edit View and Font View, the colour panels; nothing compared with the reference |
| Source formats | 6 | 15 | 13 | 65% | saves through the suite's atomic writer (a read-only file is refused and kept); Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; shape groups, gradients and shadows kept as stored values and typed in `.glyphs` 2 to 4 with exact `strokePos` conversion and loss reports on version 3 and UFO writes; hint references stored in file node order; UFO 1 sources open; EPS outlines and SFD sources read (SFD not yet in File › Open); right-to-left, vertical and contextual kerning in UFO; version conversion; a UFO's colour layer list read as palette layers and written back; CID-keyed fonts with the AFDKO UFO keys; BDF and PCF readers and writers in `crates/io-bitmap` (not yet in the model) |
| Compiling | 9 | 13 | 13 | 34% | static TTF and OTF, plain, WOFF and WOFF2, from the commands and the Export dialog; production names and database mark widths; generated pair kerning in `GPOS`; linked metrics and Default Layer Width; strokes expanded, masks applied and overlaps removed (coverage-equal to fontmake's in over 99.5% of glyphs); core tables identical, equivalent or allow-listed against fontc and fontmake in every corpus source; `gasp` and the `SVG ` table; export profiles written all-or-nothing (Export Fonts As, glyph artwork); `avar` segment maps matching fontmake's; feature code compiled into the layout tables; the name table and OS/2 of static instances as fontmake builds them; TrueType glyph programs from manual instructions with `cvt `, `fpgm`, `prep`, `maxp` and `head` flags (M10.19); variable TrueType fonts (`glyf` from the origin, `gvar`, `fvar`, `avar`, `HVAR`, `MVAR`, `STAT`, names, feature variations, variable kerning and marks) equal to fontc's in 101 of 158 corpus sources and fontmake's in 104, with variable feature syntax (conditions, variation blocks, variable positions) compiled, and instances between masters, and variable fonts with CFF2 outlines (blended charstrings, `HVAR`, `post` 2.0; equal to fontmake's `variable-cff2` in 104 of 158 sources); `COLR` and `CPAL` (versions 0 and 1), `sbix`, `SVG `, `MATH`, `BASE`, optical bounds, TrueType autohinting at export, imported hinting written back. Missing: hints, subroutines and specialisation in CFF2, `VVAR`, `cvar` and instruction deltas in variable fonts, `CBDT` and `CBLC`, variable colour tables, CFF subroutines, most of the other tables |
| Preview, proofing, printing | 2 | 24 | 15 | 14% | the Preview Area and Preview Panel (invert, flip, blur, offset), Text Preview with its text, style, size, direction and find, sample strings; proof sheets as SVG and an HTML specimen; the Hinting Preview waterfall and a hinting web preview served on the local network under a random path. Missing: printing, PDF proofs, shaping (M8); nothing compared with the reference |
| Command-line tools | 8 | 56 | 35 | 24% | `commands`, `run`, `convert`, `compile` (static fonts; most of the reference compilers' options not yet), `info`, `oracle`; `tables` (extract, add, delete, edit, checksum, reorder, show), `collection`, `woff`, `dump` (the 17 core tables), `features` (normalize, list, decompile), `glyphs` (rename, reorder), `metrics`, `dehint`, `outlines`, `variations` (map, build-avar), `proof`, `sources`, `bench`, `completion`; `subset` with pyftsubset's option names (closure through `cmap`, components and `seac`; the core tables, equal to pyftsubset's in 337 or 338 of 340 corpus fonts per selection); `check` with `outlines`, `family` and `interpolation` and the documented exit statuses; tested against small oracle outputs, the other corpus measures not yet run; `dump` with ttxn's normalised view and the `@font` and `@glyphs` views, and TTX writers for the CFF, CFF2, layout, variation, colour and miscellaneous tables (equal to ttx on 253 CFF, 700 layout and 143 other corpus tables, 0 different); `features` compile, lint and convert (VOLT equal; FontDame 19 of 28 equal, a miss); `diff`; curve conversion (qu2cu equal in 3,192, 0 different) and componentize; `autohint` and `stems`. Missing: subset of CFF2, sources, layout, variation and colour tables (M12.18, M12.19), instance, merge, transform, optimize (M12.20 to M12.26), convert between flavours, the TTX reader (M12.8, M12.9), TTX for bitmap tables, `COLR` v1 and Apple tables (M12.7b), spot listings (M12.10b), the PDF proof (M12.36), the Table Viewer (M12.38), AFDKO project compile (M12.40 to M12.42); the ttxn, tx, sfntdiff, cu2qu, ttfcomponentizer and otfautohint measures are not run |
| Quality checks | 3 | 33 | 26 | 28% | 34 of the 36 outline checks through `audit.run`; `audit.fix` applies eight fixes through path commands and each clears its check; live audit and canvas markers; 157 of M12's checks (components and anchors, font-wide metrics, names, master compatibility, variable-font tables, hinting, kerning and colour, files and sources, family) with their data sets; profiles, the ignore list, report export; the audit runner over the open document, sources (compiled in memory), compiled fonts, collections, WOFF and WOFF2 files and families, with preconditions, glyph selections and ignored results counted apart (3,676 corpus runs over 4 profiles and 919 files, no crash); the three shortcut commands with the reference tools' option names; check modules in `audit.run`, with `summary.errors`; the width, glyph-set, table, layout and CID checks; project values in the checks of compiled fonts; Font Info fixes as source commands and more outline steps through `audit.fix`. Missing: the per-glyph hash cache, several options, source-field names beyond the common fields, the glyph-data fixes, component steps, stage 3 and file targets of `audit.fix`, `AuditServices::shape` for the shaping checks, fixes on compiled fonts (M12.61), glyph and outline fix commands (M12.62), the Font Audit panel (M12.64); the outline checks give many warnings on decoded compiled fonts (to examine); verdicts agree with FontBakery's on the 44 pairs compared (the `fsSelection` and `mac_style` checks are merged by design), not yet over the whole corpus |
| Plug-ins and scripting | 3 | 23 | 23 | 25% | the Command Console, the `.fcmacro` format with the Script commands, and the URL scheme; the module runtime (wasm modules under wasmi with resource caps, hostile modules fail cleanly, filter output checked against the model's limits); module commands with Modules folder sync and safe mode; module filters in the export Filter and PreFilter parameters; check modules in the audit; dynamic Script, Filter and View menu items, reporter overlays and Palette forms; the Extension Manager with index lists and automatic updates (HTTPS in the desktop app); the Actions dialog; author documentation and a module template for each kind. Missing: installing a module by dropping a file on the window or File › Open, web storage of modules |
| Agent surfaces | 4 | 11 | 10 | 55% | execute, list, inspect and batch over three surfaces; `ui.screenshot` and `ui.render`; tools, pointer, key, text, menu and dialog control over the control channel, enough to draw and export a font, to build one from glyph sets, and to type, space and kern text (three end-to-end tests); shortcuts sent with `ui.key` open their dialogs as the keyboard does; MCP permissions, workflow prompts, resources, completion and logging; read and write folder roots for MCP and the control channel; MCP over streamable HTTP (`--transport http --listen`). Missing: `ui.click` beyond dialogs, `ui.set` |
| **Feature parity** | **117** | **985** | **852** | **~30%** | |

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
