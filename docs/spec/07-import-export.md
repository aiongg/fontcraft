# Import & Export: behaviour notes

Source: Glyphs 4 Handbook, chapter "Import & Export" (https://handbook.glyphsapp.com/), read 2026-10-07.
Rows: `spec/inventory/07-import-export.toml`. File formats themselves are `[[format]]` rows elsewhere.

## 1. Export dialog

`file.export` (File › Export…, Cmd+E) opens `dialog.export` on the current document. Four tabs:
OTF, Variable Fonts, UFO, Metrics. Next… runs the export, after a folder or save chooser where
one is needed. Each tab has a headless engine command that takes the same options as parameters.

`file.exportAll` (Cmd+Alt+E; the menu item appears in place of Export… while Option is held)
exports all open documents and shows no dialog.

Rules for every export:

- Files already present at the target are replaced (`export.overwritesExisting`).

### 1.1 OTF tab: static fonts (`file.exportStatic`, `dialog.exportOtf`)

What is exported: one font per instance defined in the font. No instances defined: one default
instance built from the first master (`export.staticUsesInstancesOrDefault`).

| Option | Values | Effect |
|---|---|---|
| Outline Flavor | PostScript/CFF, or TrueType (one of) | CFF: cubic outlines in `CFF`, suffix `.otf`. TrueType: quadratic outlines in `glyf` plus related tables, suffix `.ttf`. |
| File Format | plain (.otf/.ttf), WOFF, WOFF2 (any combination) | Every ticked container is written. |
| Remove Overlap | on/off | On: each exported glyph layer has paths and components merged into non-overlapping outlines; same operation as Path › Remove Overlap. |
| Autohint | on/off | CFF: autohint only glyphs with no manual PostScript hints. TrueType: autohint the whole font and ignore manual TrueType hints. |
| Export Destination | checkbox + folder path | Write files to this folder. Clicking the path picks another folder. |
| Test Install | checkbox | Register the fonts with the OS font system without writing files. |

Pipeline rules stated:

- Sources are cubic. TrueType export converts to quadratic during compilation; the result can
  deviate very slightly from the drawing (`export.cubicToQuadraticOnTrueType`). Sources may
  already contain quadratic paths (from opened TrueType fonts), which must compile as they are.
- TrueType export keeps components as components (`export.truetypeKeepsComponents`).
- Remove Overlap is expected on for release builds; off is for quick test exports or for fonts
  that already run the Remove Overlap filter through custom parameters
  (`export.removeOverlapOption`). The compiler must therefore not apply it twice in that case.
- Autohint scope differs per flavour (`export.autohintCff`, `export.autohintTrueType`).

Destinations:

- Folder and Test Install are independent checkboxes (`export.destinationFolder`,
  `export.testInstall`). With neither ticked, Next… asks for a folder
  (`export.promptWhenNoDestination`, `dialog.chooseExportFolder`).
- Test Install: fonts appear in applications that use the system text framework, not in
  applications with their own font machinery; avoids font-cache problems; does not survive a
  reboot.
- Per-instance sub-folder: an instance that carries an `Export Folder` custom parameter is written
  to that path relative to the destination. `/` nests folders; `..` leaves the destination
  (`export.instanceExportFolder`). The engine must resolve the path, create missing folders, and
  should refuse or warn on paths that are not valid folder names.

### 1.2 Variable Fonts tab (`file.exportVariable`, `dialog.exportVariable`)

- One font per variable font setting in the font's exports; none defined: one default setting
  (`export.variableUsesSettingsOrDefault`).
- Always TrueType outlines (`export.variableAlwaysTrueType`).
- File formats and destination behave as on the OTF tab. The handbook names nothing else on
  this tab.

### 1.3 UFO tab (`file.exportUfo`, `dialog.exportUfo`)

- A masters list; Cmd-click toggles, Shift-click selects a range, Cmd+A selects all
  (`export.ufoMasterSelection`). One UFO per selected master, since a UFO holds one master.
- Convert Glyph Names to Production Names: write production names instead of the readable names
  (`export.ufoProductionNames`).
- Decompose Smart Stuff: on, editor-only constructs (smart components, corner components and
  similar) become plain paths; off, they are omitted from the UFO
  (`export.ufoDecomposeSmartStuff`).
- Next… asks for the destination folder. Output is always UFO 3 (`export.ufoWritesVersion3`).

### 1.4 Metrics tab (`file.exportMetrics`, `dialog.exportMetrics`)

- Exports the master currently selected in the toolbar (`export.metricsCurrentMaster`).
- Next… opens a save dialog where destination and format are chosen:
  - Metrics File: the reference's own format; holds all spacing and kerning of the master
    (`export.metricsFileIsComplete`).
  - AFM File: for other editors; cannot hold group kerning or metrics keys
    (`export.afmIsLossy`).

## 2. Source formats and saving

`file.saveAs` (File › Save As…, Cmd+Shift+S; command row in `03a-edit-view-drawing.toml`) picks the
source format (`dialog.saveAs`, `source.saveAsChoosesFormat`). Three built in; plug-ins may add
more (`source.threeBuiltInFormats`, `source.pluginFormats`). A `.glyphs` file is one flat text
property list (`source.glyphsFileIsFlatPlist`).

| Format | Suffix | Shape | Notes |
|---|---|---|---|
| Glyphs File | `.glyphs` | one flat text property list (NeXTSTEP style) | default for new documents; whole file rewritten on save |
| Glyphs File Package | `.glyphspackage` | folder shown as one file; same data split up, one file per glyph | only changed files rewritten; `UIState.plist` holds the Edit View tab strings |
| Unified Font Object | `.ufo` | folder shown as one file; one master | versions 2 and 3 read and written |

Format version (`source.formatVersionKeptOnOpen`, `source.newFilesAreVersion3`):

- New Glyphs files are Version 3.
- A file from an older application version stays in its older format version when saved, so it
  remains openable there. Features that version cannot store are not saved. The user upgrades
  explicitly: Font Info › Other › File Saving › File format version → Version 3.
- Consequence for us: the writer must be able to emit the older version as well as Version 3,
  and the document must remember which one it was read as.

Package specifics (`source.packageLayout`, `source.packageSavesOnlyChangedFiles`): saving must
diff against what is on disk and touch only changed files; UI state is isolated so version
control can ignore it. Recommended for very large glyph sets and for version control.

UFO as a source (`source.ufoIsSingleMaster`, `source.ufoVersionPreserved`,
`source.ufoUnsupportedFeatureWarning`, `source.ufoPerFileImportOptions`):

- The UFO version found on open is the version written on save. Only export creates version 3
  unconditionally.
- Features that UFO cannot store raise a warning when the user activates them.
- Per-file import behaviour lives in Font Info › Other: Use Custom Naming, Disable Automatic
  Alignment.

Interoperability (`interop.otherEditors`): UFO is the general exchange format; exchange can
flatten or drop data in either direction.

## 3. Opening compiled fonts (`file.open`)

Openable: `.otf` (CFF), `.ttf`, `.ttc` (TrueType flavour), `.pfb` (Type 1 binary); plug-ins may
add formats (`open.compiledFormats`).

- Reconstruction is lossy: some hinting and some OpenType tables are not recovered, so
  open-then-export differs from the original (`open.compiledIsLossy`).
- The document opens view-only until saved to a source format with Save As
  (`open.compiledIsViewOnly`).
- Glyph order is stored as a `glyphOrder` custom parameter on the font
  (`open.recordsGlyphOrder`).
- Glyph names are converted to the built-in scheme unless the setting Keep Glyph Names from
  Imported Files is on (`open.appliesNamingScheme`).
- Composite metrics are tied to their bases through automatic alignment unless the setting
  Disable Automatic Alignment for Imported Files is on (`open.alignsCompositeMetrics`).
- TrueType: quadratic curves and components are kept as they are
  (`open.truetypeKeepsQuadratic`, `open.truetypeKeepsComponents`). Quadratic paths are editable
  but cannot be drawn new; Path › Other › Convert to Cubic converts selected paths, layers or
  glyphs.
- Layout: features are rebuilt into Font Info › Features where possible; a lookup shared by
  several features goes into a prefix named `Prefix` (`open.reconstructsFeatures`). Kerning and
  kerning groups mostly survive, contextual kerning does not (`open.kerningMostlyPreserved`).
- PostScript hints: glyph-level stem hints (mostly), alignment zones and standard stems are
  kept. Glyph hints arrive detached from nodes; for multi-master work each hint's two handles
  (blue circle, blue triangle) are dragged onto fitting nodes (`open.postscriptHintsPreserved`).
- Several fonts into one document: add them as masters with Font Info › Masters › plus ›
  Add Other Font; masters must be compatible to interpolate (`open.fontsAsMasters`).

## 4. Importing font data

### 4.1 Outlines (`file.importOutlines`, File › Import › Outlines…)

- Accepts PDF, EPS, SVG.
- Font View: multi-file; each file goes to the glyph whose name equals the file's base name
  (`import.outlinesBatchByFileName`). No such glyph: prompt per file, OK skips, Add Glyph creates
  the glyph and imports (`dialog.importOutlinesMissingGlyph`,
  `import.outlinesMissingGlyphPrompt`). Headless: `onMissingGlyph`.
- Edit View: one file into the current glyph (`import.outlinesSingleInEditView`).
- Only geometry is kept. Fill and other styling are dropped; PDF stroke width becomes a stroke
  style on the path (`import.outlinesDropStyling`).

FontCraft's choices where the handbook is silent:

- Scale: one font unit per SVG unit, with SVG y = 0 at the master's ascender. `scale: em` fits the
  drawing's box (its `viewBox`, or its width and height) to the em instead.
- Imported paths are added after the layer's shapes and rounded to the grid, as a paste is.
- A file that cannot be read is listed in `failed` and the others are imported. The command fails
  only when no file was imported and at least one failed.

Raster images are not covered by this chapter.

### 4.2 Metrics (`file.importMetrics`, File › Import › Metrics…, from Font View)

Sources: metrics file, AFM, Glyphs file, UFO (`import.metricsSources`). After the file is
chosen one of two dialogs appears.

| | Metrics / AFM file (`dialog.importMetricsFile`) | Glyphs / UFO file (`dialog.importMetricsFont`) |
|---|---|---|
| Kerning values | checkbox | checkbox; not supported from a Glyphs file |
| Kerning classes (groups) | checkbox | checkbox |
| Glyph metrics | Import Metrics + mode | not offered |
| Keep Aligned Components | checkbox | (not stated) |
| Selected Glyphs | checkbox | (not stated) |

Import Metrics modes (`import.metricsModes`), where "follows" means recomputed from the outline:

1. take LSB and width; RSB follows
2. take RSB and width; LSB follows
3. take LSB and RSB; width follows
4. take width; centre the outline (LSB = RSB)

Other rules:

- An imported value that contradicts automatic alignment switches alignment off for those
  components, unless Keep Aligned Components is ticked (`import.metricsBreaksAutoAlignment`).
- Default scope is all glyphs. Selected Glyphs limits it to the Font View selection, and an
  empty selection means all glyphs (`import.metricsSelectedGlyphs`).
- Kerning values between Glyphs files: copy and paste in the Kerning window
  (`import.metricsNoKerningValuesFromGlyphsFile`; see `kerning.copyPairs`, `kerning.pastePairs`).

### 4.3 Feature files (`file.importFeatures`, File › Import › Features…)

Pick a `.fea` file (AFDKO feature syntax); its features are added to Font Info › Features.

## 5. Pasting from vector drawing applications

Edit › Paste (Cmd+V) onto the current glyph accepts outlines from common vector applications
(`paste.vectorOutlines`).

- Scale: 1 pt = 1 font unit; there is no automatic scaling to the em
  (`paste.onePointIsOneUnit`).
- Coordinates are rounded to the grid unless the font's grid spacing is 0 (`paste.roundsToGrid`).
- If the result lies far outside the glyph box, the editor offers to correct its bounds
  (`paste.outOfBoundsCorrection`, `dialog.pastedOutlineBounds`).
- Affinity Designer: both its standard clipboard data and its SVG clipboard mode are accepted;
  shapes always arrive at (0, 0) (`paste.affinityClipboardModes`).
- Sketch: shapes should be stroke-only (1 pt, no fill) to avoid doubled outlines
  (`paste.strokeAndFillMayDouble`, inferred cause).

FontCraft's clipboard (`edit.copy`, `edit.cut`, `edit.paste`):

- Copy returns two flavours: our JSON fragment (format `fontcraft-clipboard`, with a version) and
  an SVG document. The engine never touches the system clipboard; the desktop shell does.
- A path is copied whole when it is a target shape or all its on-curve nodes are target points.
  Otherwise each run of its selected segments becomes an open path.
- Paste prefers the fragment and falls back to SVG. SVG is read at one unit per SVG unit with
  SVG y = 0 on the baseline, so our own SVG flavour pastes back at the copied coordinates.
- "Far outside" means more than half an em outside the glyph box (advance width by descender to
  ascender). The suggested correction moves the pasted box's bottom left to the origin. Both
  are our choices; the dialog (`dialog.pastedOutlineBounds`) offers that move.
- A pasted anchor replaces the layer's anchor of the same name. Global guides go to the master.

## 6. Projects

A project (`.glyphsproject`) is a separate document with instance definitions and a link to one
font file; the font file is never modified by it (`project.separateFromFont`).

Setup (`window.project`):

- `file.newProject` creates it; File › Save stores it.
- `project.linkFont`: Choose button (top right) or click on the path. The font may be closed
  (`project.linkedFontNeedNotBeOpen`). The sidebar then lists the font's instances
  (`project.startsFromFontInstances`).
- List editing: drag to reorder (`project.moveInstances`); Option-drag or copy and paste to
  duplicate (`project.duplicateInstances`); plus adds an instance or a variable font setting
  (`project.addInstance`); minus deletes the selection (`project.removeInstances`); the arrow
  button resets the list to the font's instances (`project.revertInstances`).
- Right half: the instance editor, same fields as Font Info › Exports (`project.editInstance`).

Export:

- Preconditions: project saved; link resolves. A renamed, moved or deleted font file breaks the
  link (`project.exportPreconditions`).
- `project.setExportDestination` (click on the Export Destination path), then `project.export`
  (Export button, bottom right) compiles all configured instances into that folder. The
  `Export Folder` parameter works per instance as in section 1.1
  (`project.instanceExportFolder`).

## Open questions

1. Export All: which options does it use per document (last dialog state per tab? all tabs?),
   and where do files go if no destination was ever chosen?
2. Variable Fonts tab: are Remove Overlap and Autohint present, and what do they do there?
3. What happens when no File Format box is ticked? Are both destination boxes allowed together?
4. Is the destination folder remembered per document, per tab, or application-wide?
5. Which pipeline steps besides overlap removal and autohinting run on every export
   (production names, glyph subsetting through instance parameters, feature generation,
   component decomposition for CFF, rounding)? This chapter states none of them; they must come
   from other chapters and from observation.
6. Cubic-to-quadratic conversion: tolerance, and whether masters are converted compatibly for
   static exports.
7. Remove Overlap at export when the filter is also set by custom parameter: is it really run
   twice, or detected?
8. Test Install: mechanism per platform, and what replaces it on Linux, Windows and the web.
9. Metrics file: actual syntax and extension of the reference's format; do we read it at all?
10. AFM export: are group pairs expanded to glyph pairs, and are exceptions applied?
11. Which features are blocked while the source format is UFO, and what does the warning offer?
12. Older Glyphs format versions: exactly which data is dropped when saving in a compatible
    mode, and is the user told at save time?
13. View-only mode for compiled fonts: which commands are disabled?
14. `.ttc`: does every member font open as its own document?
15. UFO export with Decompose Smart Stuff off: is the whole glyph, the shape, or only the smart
    behaviour removed?
16. Import Outlines: does it append or replace, on which layer, and how are units, the origin
    and the baseline mapped for PDF/EPS/SVG? Is there a multi-file mode in Edit View?
17. Import Metrics: target master (current?), source master for multi-master files, default
    mode, exact dialog captions, and whether the Glyphs/UFO dialog has the glyph-scope options.
18. Import Features: merge or replace on name clashes; are classes and prefixes imported too?
19. Pasted-outline correction: threshold, and whether it translates only.
20. Projects: file syntax; is the font link stored relative or absolute; does the project keep
    its own copy of the instances or overrides on top of the font's?
