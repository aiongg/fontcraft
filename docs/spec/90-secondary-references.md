# 90 · Secondary references: FontLab, RoboFont, FontForge, Fontra

Inventory: `spec/inventory/90-secondary.toml` (148 `[[command]]`, 32 `[[behavior]]`, 24 `[[panel]]`,
13 `[[check]]`, 8 `[[tool]]`, 6 `[[setting]]`, 4 `[[parameter]]` rows; every row carries `origin`).
Sources: public user documentation only, read on 2026-10-07 (URLs per product below). No source code of
any of the four products was opened.

A row is in the inventory only when (a) the primary reference's handbook does not describe the feature
(checked by searching the handbook text) and (b) no other inventory file already has it. Rows were
checked against all other `spec/inventory/*.toml` files for id collisions and for doing the same job
under another name; the duplicates that were dropped are listed in section 7.

## 1. Comparison matrix

Cells marked *(m)* are recalled, not read on a page this session.

| | Glyphs 4 (primary) | FontLab 8 | RoboFont 4 | FontForge | Fontra |
|---|---|---|---|---|---|
| Platforms | macOS only | macOS 10.15+, Windows 8.1/10/11; Linux only through Wine or CrossOver, unsupported | macOS only (10.12+, Intel and Apple Silicon) | Linux, macOS, Windows *(m)* | Any modern browser; desktop wrapper (Fontra Pak) for macOS, Windows, Linux |
| Licence | Proprietary, paid | Proprietary, paid | Proprietary, paid | Free software, GPL | Free software, GPL-3 (docs under CC BY 4.0) |
| Native source format | `.glyphs` single file or `.glyphspackage` folder | `.vfc` (binary) and `.vfj` (JSON) | UFO 3 folder (also reads and writes UFO 2) | `.sfd` text file, or a folder with one file per glyph | `.fontra` folder *(m: layout)*; also edits designspace + UFO in place |
| Other sources opened | UFO, compiled fonts, Type 1 `.pfb` | `.glyphs` 2 and 3, UFO, designspace, `.vfb`, FontForge `.sfd`, BDF, Type 1 incl. Mac suitcases, compiled and colour fonts | OpenType (features not editable), WOFF, `.vfb` with a helper | Type 1 (`pfa`, `pfb`), Type 3, Type 42, CFF, CID, TrueType, OpenType, collections, SVG fonts, Ikarus, Metafont, BDF, PCF, PK/GF, Mac resource fonts, PDF-embedded fonts | UFO, designspace, `.glyphs`, `.rcjk`, compiled TTF/OTF |
| Variable fonts | Masters, intermediate and alternate layers, axis mappings, variable TTF and CFF2 export | Masters, font-less (per-glyph) masters, per-glyph axes, conditional substitution, axis graph (avar), STAT from named locations, variable TTF, CFF2 and variable COLRv1 export | No built-in design-space editor; designspace + UFO workflow through bundled libraries and extensions | Type 1 Multiple Master and Apple distortable fonts only; no OpenType variations in the docs read | Variable-first: every glyph has its own source list and optional private axes; variable components; cross-axis mappings (avar 2); TTF/OTF export incl. CFF2 |
| Hinting tools | PostScript autohinter and manual hints; TrueType instructor tool; ttfautohint | PostScript autohint and linked manual hints (Magnet tool); visual TrueType hinting tool with text program, stems, zones, ClearType preview; ttfautohint; keeps imported instructions | Autohint option at generation through the bundled AFDKO; no manual hint editor in the pages read | PostScript autohint, hint substitution, counter hints, manual and reviewed stem hints, stem and zone histograms; TrueType auto-instructor, bytecode editor, `fpgm`/`prep`/`cvt` editors, step debugger, grid-fit view | None documented |
| Scripting model | Python scripts, Macro Panel, plug-ins from a plug-in manager | Embedded Python 3.11, start-up scripts, bundled TypeRig and fontTools; batch Actions with saved JSON action sets | The application itself is Python 3.12; FontParts object API, event subscribers, extension bundles (`.roboFontExt`), extension manager and store, command-line launcher | Own script language (`.pe`, `.ff`) and Python module; headless `-script` / `-c`; user script menu; Python plug-ins | Plug-in manager in the app; Python server behind a browser client; no in-app scripting documented |

## 2. Conventions used for the rows

- **Shortcuts are not adopted.** The primary reference owns the keymap, and several secondary keys clash
  with it (FontLab's `L` for Tunni lines is the primary's Measurement tool; `Cmd+Shift+T` opens Actions
  in FontLab and tidies paths in the primary). Every row has `shortcut = ""` and quotes the reference
  key in its summary so it can be offered as an alternative binding.
- **Ids follow the prefixes already in the inventory** (`guide.`, `hint.tt.`, `hint.ps.`, `kerning.`,
  `spacing.`, `audit.`, `script.`, `app.`, `component.`). New nouns introduced here: `bitmap.` (strikes)
  and `cid.`.
- **Menus are our placement**, not the reference's. FontLab's Contour menu maps to our Path menu, its
  Tools › Actions to Script.
- **Status.** `docs` = read on a public page; `memory` = recalled, not found on a page; `inferred` = our
  own command shape for a documented capability. Two rows are `memory` and one `inferred` (section 8); the M13 planning pass verified three former `memory` rows.

## 3. FontLab 8

**What it is.** A commercial font editor for macOS and Windows with the widest built-in feature set of
the four: drawing aids, non-destructive "elements" and filters, auditing, batch actions, colour and
variable font production. **Licence:** proprietary.

**Documentation note.** The FontLab 8 help site consists of long "What's new" chapters plus tutorials;
its Manual link lists no pages, and the per-feature manual pages exist only under the FontLab 7 path.
Feature detail below therefore comes from the 8 "What's new" chapters and, where named, the FontLab 7
manual page for the same feature.

### 3.1 What we take

| Feature | Our id | Why it matters |
|---|---|---|
| Tunni lines | `view.showTunniLines`, `path.selectTunniLines`, `tunni.dragLineSymmetric`, `tunni.dragPointAsymmetric` | The standard way to adjust curve tension with both handles in balance; the primary has only a palette slider (Fit Curve) |
| Numeric curve tension | `path.setCurveTension`, `view.showCurveTension`, parameter `curveTension` | Same tension across a whole design, by number; scriptable |
| Harmonize (G2) and superharmonize (G3) | `path.harmonizeNodes`, `path.harmonizeHandles` | One-shot curvature smoothing; the primary's star node is a persistent node type, not a command |
| Power Nudge and servant nodes | `path.togglePowerNudge`, `path.setServantNode` | Moves a feature while joining curves adapt, beyond the primary's proportional nudge |
| Slide along curve, lever dragging | `nodes.slideAlongCurve`, `drag.leverPrecision`, setting `editing.leverRatio` | Precision without zooming |
| Rapid tool | tool `rapid` | Fast tracing with automatic, consistent handles |
| Thickness tool | tool `thickness` | Per-node stroke width on canvas for skeleton-based drafting |
| Ink traps, looped corners, replace with corner | `path.makeInkTrap`, `path.loopCorner`, `path.replaceWithCorner`, parameter `inkTrapWidth` | Common production edits as single commands |
| Point alignment extras | `path.alignToMask`, `path.collapsePoints`, `path.scaleToSameSize` | Small but frequent clean-up steps |
| Stored selections | panel `palette.selections`, `edit.storeSelection`, `edit.recallSelection` | Reselect the same nodes across masters; gives agents named node sets |
| Outline lock for a view | `view.lockOutline` | Space or hint without moving a node by accident |
| Auto-meter | `view.showStemWidths`, `view.showCounterWidths`, `view.showCornerAngles`, `autometer.editStemNumerically` | Stem consistency visible and editable while drawing |
| Quick measurement | `view.showQuickMeasurement` | Measure by hovering, no tool switch |
| Measurements table | panel `palette.measurements`, `font.recalculateMeasurements` | Design values per master, recalculated from outlines |
| Details across glyphs | `view.showDetailsAcrossGlyphs` | Compare stems and overshoots across a word |
| Snapping suggestions | `view.suggestHandles`, `view.suggestLines`, `view.suggestDistance`, `view.suggestStems`, parameter `suggestDistance`, setting `snapping.suggestActivation` | Draw parallel edges and consistent stems without guides |
| Power guides | tool `magnet`, `guide.makePowerGuide`, `guide.addPowerGuides`, `guide.unlinkPowerGuides`, `guide.powerGuideScope` | Re-proportion glyphs by moving guides while stems keep their width |
| Thick guides and guide markers | `guide.setThickness`, `guide.addMarkers`, setting `appearance.guideOpacity` | Bands with centreline snapping; measured positions along a guide |
| Copy component transformation | `component.copyTransformation` | One component's position, scale, rotation and slant applied to others in one step |
| Detect composites and components | `font.detectComposites`, `path.detectComponent` | Rebuild components after opening a compiled font |
| Auto layers with stored recipes | `glyph.setAutoLayerRecipe`, `recipe.directives` | Composites that rebuild themselves and fill in when a missing part is added later |
| Component pointing at an instance | `component.setInstanceLocation` | A component can use a lighter or narrower interpolation of its source |
| Anchor expressions | `anchor.positionExpressions` *(memory)*, `anchor.removeExpressions` | Anchors that follow metrics |
| FontAudit additions | `audit.fontViewCellMarks`, 4 checks, setting `audit.irregularStemTolerance` | See 3.3 |
| Actions and action sets | panel `dialog.actions`, `script.runActionSet`, `script.saveActionSet`, `script.repeatLastAction` | Batch operations with preview, and macros as data instead of code. Fits "everything is a command" directly |
| Adjust stems | `filter.adjustStems` | Change stem weights by range without redrawing |
| Command search, quick help, more info | `app.commandPalette`, `app.quickHelp`, `edit.moreInfo`, `ui.numericFieldExpressions` | Free by-products of a command registry with descriptions |
| Auto-spacing and metrics beam | `spacing.autoSpace`, setting `spacing.autoSpaceMethod`, `view.showMetricsBeam` | First-pass spacing; sidebearings measured at a chosen height |
| Auto-kerning | `kerning.autoKern`, `kerning.kernToDistance` | First-pass kerning for a pair list |
| Kerning audit and clean-up | `kerning.auditExceptions`, `kerning.removePairsByRule`, `kerning.matchAcrossMasters` | Find collisions hidden by class kerning; keep kerning interpolable |
| Pair list work panel | panel `palette.pairsAndPhrases`, `kerning.loadPairList`, `kerning.stepPairList`, `kerning.representativeGlyph` | Walk a kerning list with the keyboard |
| Optical bounds | `features.opticalBoundsFromGuides`, parameter `opticalOffset` | `lfbd`/`rtbd` from guides |
| Matchmaker | tool `matchmaker`, `glyph.matchMasters`, `glyph.relocateStartPoints`, `edit.toggleMatchWhenEditing` | Fixing compatibility by direct manipulation and automatically |
| Master set-up helpers | `font.mergeFontsToMasters`, `font.addVariation`, `glyph.replaceWithInstance`, `font.setMasterLocations`, `view.animateAxis` | Turn a static family into a variable source; correct single glyphs by interpolation |
| TrueType hint program as text | panel `palette.ttHintProgram`, `hint.tt.setProgramText`, `hint.tt.referToNodeNames`, `hint.tt.deltaWaterfallMarks` | Textual, diffable manual hinting |
| Colour helpers | `font.overlayFonts`, `color.generateDarkPalette` | Chromatic families into one colour font; dark-mode palette |
| Export profiles | panel `dialog.exportProfiles`, `file.exportWithProfiles`, `file.saveExportProfile` | Named, reusable export settings; many fonts through many profiles in one run |
| Vector output of views and glyphs | `file.exportWindowContents`, `glyph.exportArtwork` | Proofs and artwork exchange |
| Encoding files, cross-font table | `view.filterByEncodingFile`, `font.compareInTable` | Legacy code-page views; compare a property across a family |
| Waterfall preview, important note | `view.previewWaterfall`, `fontInfo.importantNote` | Proofing; hand-over notes |

### 3.2 What we do not take

- **Sketchboard** (free-form canvas outside any font) and **stickers**: the primary's annotation tool
  and background layers cover the need.
- **Glue and Skin filters**: Skin is the primary's corner, cap and segment components under another
  name; Glue is a second, overlapping mechanism.
- **Creative effects** (Engrave, Scribble, Warp, shadow and extrusion): the primary's filters cover the
  common ones; the rest are illustration features.
- **Smart Pencil shape recognition, Power Brush, Erasing Brush, Text Shape elements**: low value for
  font production.
- **Embedded Python, TypeRig, `initScript.py` UI customisation**: we script through the command
  registry, not an embedded interpreter.
- **VOLT project integration, `.vfb` writing, EOT export, digital-signature setup**: legacy.
- **Widgets, scrollable panel dock, floating toolboxes**: UI choices that conflict with the primary
  reference's layout.
- **Element references, the delta filter, live filters with Expand and Remove, and Fusion** (open
  decision D7): neither `.glyphs` nor UFO can store them. Components of non-exporting glyphs, corner
  components and the mask attribute cover their uses; Copy Element Transformation is taken for
  components only (`component.copyTransformation`).
- **Direction-filtered removal of TrueType hints, import of TT stems and zones, keeping imported
  instructions**: the primary already has these in some form (`hint.tt.removeHints`,
  `hint.tt.getPsStems`, imported hinting parameters); a `direction` option on `hint.tt.removeHints` is
  the only gap.

### 3.3 FontAudit tests against our checks

Our audit already runs live, marks the canvas and fixes in one step (`audit.toggleLiveAudit`,
`audit.canvasMarkers`, `audit.fix`). FontAudit's tests map as follows; "nearest" means the existing
check with the closest name, to be confirmed against its row.

| FontAudit test | Our check |
|---|---|
| Curvature | `check.outlines.curvatureBreak` (nearest) |
| Disproportional | `check.outlines.unbalancedHandles` (nearest) |
| Empty lines and curves | `check.outlines.coincidentPoints` |
| Empty contours | `check.outlines.zeroAreaContour` |
| Open contour | `check.outlines.openPath` |
| Vectors on ClosePaths | **new** `check.outlines.redundantClosingLine` |
| Flat curves | `check.outlines.flatCurve` |
| Collinear vectors | `check.outlines.collinearLines` |
| Inflections on curves | `check.outlines.inflections` |
| Weak extremum points | **new** `check.outlines.weakExtremum` |
| Normal extremum points | `check.outlines.missingExtremes` |
| Incorrect smooth connection | `check.outlines.kinkedSmoothNode` / `check.outlines.unflaggedSmoothNode` (nearest) |
| Cusp curve, Loop | `check.outlines.crossedHandles` |
| Zero-length handles | `check.outlines.zeroLengthHandle` |
| Semi-horizontal and vertical vectors | `check.outlines.almostOrthogonalLine` |
| Contour is not closed | **new** `check.outlines.closedButFlaggedOpen` |
| Object is too short | `check.outlines.shortSegments` |
| Irregular stems (FontLab 8) | **new** `check.outlines.irregularStem` |

### 3.4 Pages read

- https://help.fontlab.com/fontlab/8/ (navigation)
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-since-fontlab-vi/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-01-explore-prepare/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-02-draft-draw/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-03-edit-refine/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-04-consistency-precision/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-05-build-assemble/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-06-metrics-kerning/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-07-families-variation/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-08-test-adjust/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-09-color/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-10-glyphs-fonts/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-11-formats/
- https://help.fontlab.com/fontlab/8/whats-new/whats-new-12-scripts-extensions/
- FontLab 7 manual: https://help.fontlab.com/fontlab/7/manual/Tunni-Lines/ , `/Power-Guides/` ,
  `/FontAudit/` , `/FontAudit-panel/` , `/TrueType-Hinting-tool/` , `/Hinting-a-Font/` , `/Actions/` ,
  `/Export-Profiles/` , `/Pairs-and-Phrases-panel/`
- Not readable: `https://help.fontlab.com/fontlab/8/manual/Tunni-Lines/` and `/manual/FontAudit/`
  (404; the 8 manual has no per-feature pages at those paths).

## 4. RoboFont 4

**What it is.** A macOS font editor that works directly on UFO files and is itself a Python
application. Its design principle is a small core that users extend: the documentation says outright
that functions should be separate tools rather than built-in features. **Licence:** proprietary.

### 4.1 What we take

| Feature | Our id | Why it matters |
|---|---|---|
| Space Center | panel `window.spaceCenter`, `spaceCenter.inputSyntax`, `spaceCenter.matrixEntry`, `spaceCenter.pointerAndKeys` | Multi-line spacing with a numeric matrix, without opening glyphs for drawing |
| Cut-off band, preferred suffix | `view.setSpacingCutOff`, `view.setPreferredSuffix` | Judge rhythm without extenders; proof an alternate set without a feature |
| Beam | `view.showMetricsBeam` (row filed under FontLab, which has the same aid) | Sidebearings at a chosen height |
| Groups editor with stacked view | panel `dialog.kerningGroupsEditor` | See at once which member does not fit its kerning group |
| Group pair preview | `kerning.previewGroupCombinations` | Every glyph combination a class pair covers |
| Guide colour, magnetism, slicing | `guide.setAppearance`, `guide.sliceContours` | Per-guide snapping strength; cut along a guide |
| Display toggles | `view.showOutlineErrors`, `view.showIndexes`, `view.showCurveLength` | Cheap always-on warnings; indexes give people and agents a shared way to name parts |
| External changes review | panel `dialog.externalChanges`, `file.reviewExternalChanges`, `io.watchSourceOnDisk` | Safe to run scripts, agents or a version-control checkout against an open source |
| Stable source serialisation | `io.stableSourceSerialization` *(memory)* | Clean diffs |
| Glyph construction rules | `font.buildFromConstructions`, `constructions.ruleGrammar` *(memory)* | Composites from a rule file with richer positioning than anchors alone |
| Preferences editor | `app.openPreferencesEditor` | Every setting reachable |

### 4.2 What we do not take

- **The Python object model as the extension surface** (FontParts, `mojo`, subscribers, vanilla UI
  toolkit, Merz drawing layer). Our extension surface is the command registry and its change events.
  The ideas that carry over are the bundle with a manifest that adds menu items and tools, an in-app
  manager for finding and updating them, and safe mode; those belong to the `extensions` area and the
  primary's plug-in manager rows already cover them (safe mode is `app.launchWithoutPlugins`).
- **Smart Sets**: the primary's smart filters and lists are the same thing.
- **Kern Center as a separate sheet**: the primary's Kerning window already lists pairs; only the
  group-combination preview is taken.
- **Feature preview through a generated test font, Test Install**: in the primary.
- **MetricsMachine and Prepolator**: third-party extensions, not covered by the RoboFont pages read.
- **The command-line launcher** (`roboFont -p / -c / -o`): our CLI dispatches commands directly.

### 4.3 Pages read

- https://robofont.com/documentation/ , `/documentation/topics/` , `/documentation/reference/`
- https://robofont.com/documentation/topics/robofont-features-overview/ ,
  `/topics/robofont-design-principles/` , `/topics/smartsets/` , `/topics/extensions/` ,
  `/topics/accented-glyphs/` , `/topics/kerning/` , `/topics/preparing-for-interpolation/`
- https://robofont.com/documentation/reference/workspace/space-center/ ,
  `/reference/workspace/font-overview/kern-center/` , `/reference/workspace/font-overview/groups-editor/` ,
  `/reference/workspace/external-changes-window/` , `/reference/workspace/glyph-editor/guidelines/` ,
  `/reference/workspace/glyph-editor/display-options/` ,
  `/reference/workspace/glyph-editor/tools/editing-tool/` , `/tools/measurement-tool/` , `/tools/slice-tool/`
- https://robofont.com/documentation/reference/workspace/safe-mode/ ,
  `/reference/workspace/preferences-editor/`
- https://robofont.com/documentation/tutorials/using-test-install/ ,
  `/tutorials/using-robofont-from-command-line/`
- https://robofont.com/technical-specification/
- Not read: the hinting and variable-font tutorials (so the hinting and variable-font cells of the
  matrix for RoboFont rest on the test-install page, the bundled-library list and tutorial titles), and
  the licensing pages.

## 5. FontForge

**What it is.** The long-standing free font editor. Strongest in breadth of formats (bitmap strikes,
Type 1, Type 3, CID-keyed, Mac resource fonts, TeX metrics), in low-level access (raw TrueType
instructions with a debugger, direct editing of layout tables) and in checking dialogs. **Licence:**
GPL. Only the user documentation was read.

### 5.1 What we take

| Feature | Our id | Why it matters |
|---|---|---|
| Bitmap strikes | `bitmap.setStrikes`, `bitmap.regenerateGlyphs`, `bitmap.removeGlyphs`, `bitmap.setPixels` *(inferred)*, `bitmap.transform`, `view.setFontViewStrike` | Embedded bitmaps for small sizes and bitmap-only fonts; absent from the primary |
| Bitmap glyph editor | panel `window.bitmapEditor`, tools `bitmapPencil`, `bitmapLine`, `bitmapShift`, panel `palette.shades`, panel `dialog.strikeProperties`, `bitmap.widthOnlyInBitmapFonts`, `bitmap.references` | Pixel editing over the scaled outline, black-and-white and grey |
| Strike import and export | `bitmap.importStrikes`, `bitmap.exportStrikes` | BDF, PCF, PK/GF and embedded strikes in; BDF, embedded and bitmap-only sfnt out |
| Bitmap checks | `check.bitmaps.missingGlyph`, `check.bitmaps.advanceMismatch` | Strike completeness |
| Find and replace outlines | panel `dialog.findOutline`, `path.findOutlinePattern`, `path.replaceOutlinePattern` | Swap a serif or terminal design across a font |
| Path commands | `path.findIntersections`, `path.makeParallel`, `path.spacePoints`, `path.roundToCluster`, `path.makeArc`, `path.insertNodeAt`, `path.nameContour` | Small geometric helpers the primary lacks |
| Style transformations | `filter.changeWeight`, `filter.condenseExtend`, `filter.changeXHeight`, `filter.nonLinearTransform` | Drafting a weight or width variant while holding heights or stems |
| Derived glyph sets | `glyph.addSmallCaps`, `glyph.addSubSuperscripts` | First drafts of small caps and inferiors together with their features |
| Autotrace | `glyph.autotraceImage` | Scan to outline inside the app |
| Spiro drawing | tool `spiro` | Curves from on-curve points only |
| Whole-font comparison | `font.compareGlyphs`, `font.compareLayers` | Glyph-level diff with tolerance, complementing table-level `font.diff` |
| Two-font interpolation | `font.interpolateFonts` | Quick blend or extrapolation without setting up masters |
| Dependencies | `glyph.listDependencies` | Where a glyph is used, by components and by substitutions |
| Edit helpers | `file.revertGlyph`, `edit.copyMetrics`, `edit.copyLayerToLayer`, `edit.pasteAfter`, `spacing.centerInWidth` | Per-glyph revert; metrics-only copy; layer copy across a selection |
| Printing | `file.print` | Menu front end to the proof generator with four standard layouts |
| Validation additions | `audit.selectGlyphsWithIssue`, `audit.nearTolerance`, setting `export.validateBeforeExport`, checks `check.outlines.handleBeyondSegment`, `.negligibleHandle`, `.pointCountLimit`, `check.metrics.advanceDiffersFrom`, `.boundsBeyond`, `check.hinting.stemNearStandardWidth`, `check.cid.definedTwiceOrMissing` | The parts of Find Problems and the Validation window our audit did not have |
| Raw TrueType instructions | panel `dialog.ttInstructions`, `hint.tt.setAssembly` | Assembly-level editing of glyph programs, `fpgm` and `prep` |
| TrueType debugger | panel `window.ttDebugger`, `hint.tt.debugStep` | Step through a glyph program and watch points move |
| Delta suggestions | `hint.tt.suggestDeltas` | Find sizes that need a delta |
| PostScript hint review | panel `dialog.stemHistogram` *(memory for its controls)*, panel `dialog.reviewHints` | Pick standard stems and zones from a distribution; inspect hints one by one |
| Kerning review and groups | panel `window.kerningPairsPreview`, `kerning.buildGroups`, `kerning.verticalFromHorizontal` | See every pair rendered; propose kerning groups from side profiles |
| MATH and BASE editors | panel `dialog.mathTable`, `font.setMathData`, panel `dialog.baselines` | Math fonts; explicit baseline data |
| Name lists | `font.renameGlyphsByNamelist` | Switch a font between naming schemes |
| CID-keyed fonts | `cid.convertToCid`, `cid.flatten`, `cid.editSubfonts` | Sub-font management beyond a registry/ordering/supplement parameter |
| Legacy metrics | `file.importMetricsFile` | Kerning for Type 1 fonts lives in AFM/PFM files |

Find Problems items that our audit already has (open paths, intersecting paths, direction, missing
extremes, points too close, flipped or transformed or nested references, mixed contours and
references, duplicate names and code points, name/code-point mismatch, missing anchors, substitutions
naming missing glyphs) were not repeated.

### 5.2 What we do not take

- **Type 3 and SVG-font multi-layer editing** (per-layer fill, stroke, dash, pattern, clip path): COLR
  and OpenType-SVG are the current answers and the primary's colour chapter covers them.
- **Type 1 Multiple Master and Apple distortable font menus**: superseded by OpenType variations.
- **Writing legacy formats**: Type 0, Type 3, Type 42, Type 1 CID, Mac resource fonts and FOND
  families, Windows FON/FNT, Palm fonts, TeX TFM/OFM metrics, the `PfEd` private table.
- **Device tables** and the Kern Pair Closeup / Anchor Control size-specific corrections: unused by
  current rasterisers.
- **JSTF justification data, Mac state-machine (AAT) editor, lookup-list dialogs**: our layout source is
  feature code, as in the primary.
- **Encoding slots as the font's organising principle** (Reencode, Compact, Force Encoding, Detach
  Glyphs, "same glyph as"): we use a glyph-set model; only name lists and encoding-file filters are
  taken.
- **Outline, inline, shadow and wireframe effects; 3D rotate and perspective tools; tile path**:
  illustration effects.
- **Italic transformation**: the reference calls its own result incomplete.
- **The native scripting language and the Python module**; the user-defined Tools menu.
- **Auto Width as a separate dialog**: folded into `spacing.autoSpace` parameters.
- **Diagonal PostScript hints**: neither CFF nor the source formats can store them.

### 5.3 Pages read

- https://fontforge.org/docs/ , `/docs/ui.html` , `/docs/ui/dialogs.html`
- Menus: `/docs/ui/menus/filemenu.html` , `editmenu.html` , `pointmenu.html` , `elementmenu.html`
  (twice, second time for detail) , `toolsmenu.html` , `hintsmenu.html` , `encodingmenu.html` ,
  `viewmenu.html` , `metricsmenu.html` , `cidmenu.html` , `mmmenu.html`
- Views: `/docs/ui/mainviews/charview.html` , `bitmapview.html` , `metricsview.html`
- Dialogs: `/docs/ui/dialogs/problems.html` , `validation.html` , `autowidth.html` , `search.html` ,
  `Styles.html` , `generate.html` , `kernpairs.html` , `multilayer.html` , `ttfinstrs.html` ,
  `lookups.html` , `display.html`
- Scripting: `/docs/scripting/scripting.html`
- Not read: the Window and Help menu pages (nothing to take), `histogram.html`, `math.html`,
  `baseline.html`, `bdfinfo.html`, `transform.html`, `charinfo.html`, `fontinfo.html`. The rows for the
  histogram, MATH, baseline and strike-property panels rest on the menu page descriptions and the
  dialog index; their control lists need those pages before implementation.

## 6. Fontra

**What it is.** A browser-based, variable-first font editor, begun for large CJK projects, with a
server that holds the document and browser clients that edit it. **Licence:** GPL-3; documentation
CC BY 4.0. Only the documentation site was read. The documentation is young: several reference pages
are a title and a screenshot.

### 6.1 What we take

| Feature | Our id | Why it matters |
|---|---|---|
| Per-glyph source panel | panel `palette.glyphSources`, `glyph.setSourceActive`, `view.showSourceInfluence` | Shows the primary's master and intermediate layers as one list per glyph, with which are missing and how much each contributes; lets one glyph skip a master |
| Cross-axis mappings | panel `window.crossAxisMappings`, `font.setCrossAxisMappings` | avar version 2; our CLI already compiles it, this is the editor for it |
| Reference font | panel `palette.referenceFont`, `view.setReferenceFont` | Draw against a compiled font in the background |
| Related glyphs and characters | panel `palette.relatedGlyphs` | Navigate between a letter, its composites, its alternates and its Unicode decomposition |
| Status definitions | `font.setStatusDefinitions`, `glyph.setStatus` *(memory for where it is stored)* | Named workflow states instead of bare colour labels |
| Ghost outline while dragging | `drag.ghostOutline` | See the change against the starting shape |
| App info | `app.info` | Version, start time, control-channel and MCP state and the loaded modules in one query |
| Browser client | `app.browserClient` | The web target planned for 1.0 |

### 6.2 What we do not take

- **Glyph axes and variable components as a separate mechanism.** The primary's smart glyphs already
  give a glyph private axes (`smartGlyph.*`). The one extension worth keeping, a component placed at a
  location on *font* axes, is `component.setInstanceLocation` (filed under FontLab, which documents
  it in detail).
- **Adding a glyph source at a location** as a new command: it is the primary's intermediate layer
  (`layers.setIntermediate`).
- **The `.rcjk` project format and CJK design frame.**
- **The Python server and its plug-in mechanism**; our server side is the Rust command dispatcher.
- **Export as conversion** (TTF to designspace, `.glyphs` to another source): our CLI `convert` covers it.
- **Glyph locking**: already in the primary (`glyph.toggleLocked`).
- **Live shared editing and serving a project to other editors**, not in 1.0: it opens the document
  to the network and needs a conflict policy and permissions that no reference documents. The control
  channel and MCP already let agents and the app work on one document. Server Info is taken as
  `app.info`.

### 6.3 Pages read

- https://docs.fontra.xyz/ , `/introduction` , `/reference` , `/explanations` , `/how-tos`
- `/explanations/understanding-variable-components` (page holds a video and a link only)
- `/how-tos/draw/designing-with-variable-components`
- `/reference/editor-view/panels/designspace` , `/panels/reference-font` , `/panels/transformations` ,
  `/panels/related-glyphs-and-characters`
- `/reference/editor-view/tools/pointer` , `/tools/knife`
- `/reference/menu/view/glyph-editor-appearance`
- `/reference/menu/font/cross-axis-mapping` , `/menu/font/sources` , `/menu/font/status-colors`
- `/reference/menu/file/export-as`
- `/reference/menu/fontra/plugin-manager` , `/menu/fontra/editor-behavior` , `/menu/fontra/server-info`

## 7. Rows drafted and then dropped as duplicates

Dropped because another inventory file already has the feature:

- Live outline audit, canvas markers, audit panel, check/fix/next commands, the guided problem walk and
  the validation window: `audit.*` and `window.fontAudit` in `21-qa-checks.toml`.
- Fifteen FontAudit tests and six Find Problems checks: existing `check.outlines.*`,
  `check.components.*`, `check.glyphset.*`, `check.anchors.*` (table in 3.3).
- Slanted marquee selection: `selection.slantedMarquee` (the primary has it).
- Glyph locking: `glyph.toggleLocked`.
- Reproducible output with a fixed timestamp: `compile.reproducibleOutput` in `40-tables.toml`.
- Stem histogram as a query: `autohint.ps.reportStems`; only the chart panel is kept.
- Removing TrueType hints and imported hinting tables, importing TT stems from PostScript stems:
  `hint.tt.removeHints`, `hint.tt.removeImportedHinting`, `hint.tt.getPsStems`.
- Merging fonts: CLI `merge` (`font.merge`).
- Per-master copies of kerning groups: the primary stores groups per glyph, not per master.

## 8. What rests on memory or inference

| Row | Status | What is unconfirmed |
|---|---|---|
| `anchor.positionExpressions` | docs (M13 pass) | Verified against the FontLab 7 manual page Using Expressions: operators, variables and examples, no formal grammar; our grammar is a guess on the row |
| `dialog.stemHistogram` | memory | The chart's controls. The three histogram kinds are documented |
| `io.stableSourceSerialization` | memory | That RoboFont normalises on save and writes only changed glyph files. The page read only lists a UFO normaliser among bundled libraries |
| `constructions.ruleGrammar` | docs (M13 pass) | Verified against the Glyph Construction README (MIT) |
| `glyph.setStatus` | docs (M13 pass) | The definitions and the tint are documented; where a status is stored is not, so per glyph is our guess |
| `bitmap.setPixels` | inferred | Our headless form of pixel editing |

In the matrix: FontForge's platforms, the absence of OpenType variation support in FontForge, the layout
of the `.fontra` format, and the licences of FontLab and RoboFont as "proprietary" (their pages link
to a paid licence and EULA, not read) are recalled or deduced.

## 9. Open questions

1. **Element references in the source model.** A shared shape with no owning glyph has no equivalent in
   the primary's file format or in UFO. Decide whether elements are a native-format feature that
   decomposes to hidden glyphs plus components on export to `.glyphs` and UFO, or are dropped.
   **Decided 2026-10-11 (D7, `docs/plans/M13.md` § 7 question 3):** dropped, with the delta filter and Fusion.
2. **Live filters versus the primary's attributes.** Delta filter, Fusion and per-node stroke width need
   a place in the shape model next to the primary's mask and stroke attributes. One stack, or two?
   **Decided 2026-10-11:** no second stack; the delta filter and Fusion are dropped with D7. Per-node stroke width (the Thickness tool) is stored in the path's stroke attributes and node user data (M13.38).
3. **Power guides and the undo model.** Moving a font-level power guide edits many glyphs from one
   drag. Confirm that this is one undo step and how it interacts with locked glyphs.
   **M13 pass (ours):** one undo step for every glyph a drag reshapes; locked glyphs keep their outline, with a warning (`guide.makePowerGuide` guess).
4. **Auto-kerning and auto-spacing algorithms.** Neither FontLab nor FontForge documents its method
   beyond "optical" and a target separation. We need our own, with a stated definition of the optical
   gap, and a test corpus.
   **M13 pass (ours, D11):** a side-profile method of our own, calibrated on each font's control glyphs, with a corpus measure (`docs/plans/M13.md` § 3 and § 5).
5. **Bitmap strikes in the native format.** Where strikes live (per master? per font?), how grey depth
   is stored, and whether strike editing belongs to M11 or should wait for demand.
   **M13.63 (ours):** strike glyphs are edited by command (`bitmap.setPixels`, `bitmap.transform`,
   `bitmap.regenerateGlyphs`, `bitmap.removeGlyphs`, `bitmap.setStrikeProperties`). A strike glyph
   may hold references to other bitmaps of its strike; they are stored as references and drawn into
   the pixels on export.
6. **Action sets and command stability.** A saved action set is a list of command ids with parameters,
   so command ids and parameter names become a file format. Needs a versioning rule.
   **M13 pass (D8):** action sets are macros: JSON `.fcmacro` files with a format version; command ids and parameter names follow decision 0004 item 8 (aliases for renames); a run checks every step before running any.
7. **Shared editing.** Conflict policy (last writer wins per glyph layer, or operation merging),
   permissions, and whether undo is per client. Fontra's pages do not describe theirs.
   **Decided 2026-10-11 (`docs/plans/M13.md` § 7 question 4):** not in 1.0; Server Info is kept as the `app.info` query.
8. **TrueType debugger scope.** It needs a complete bytecode interpreter of our own. Confirm that the
   interpreter planned for hinting preview is complete enough, or defer the debugger.
   **M13 pass:** built on M10's interpreter if that can stop after each instruction; otherwise it waits (M13.52).
9. **Curve tension definition.** FontLab states that the displayed tension equals the Tunni percentage
   (handle length relative to the distance to the handle intersection). Behaviour for segments whose
   handles are parallel or diverge, where no intersection exists, must be observed in the application.
   **M13 pass (ours, until FontLab is observed):** tension is shown as a dash and left unchanged where handles do not meet (`path.setCurveTension` guess).
10. **Space Center tokens.** The reference spellings (`/?`, `/!`, `/:group`, `@layer`) may clash with
    the primary's text-mode escapes and placeholders; choose spellings once for both views.
    **M13 pass:** RoboFont's spellings are kept in Space Center; none clashes with the primary's escapes, since `?`, `!` and `:` cannot start a glyph name (`spaceCenter.inputSyntax`).
11. **Match Masters heuristics.** "Weak" and "strong" matching are named but not specified.
    **M9.34 pass (ours, until FontLab is observed):** weak matching leaves layers that are already compatible alone; strong matching matches them too. Both pair paths by centroid and area and never add nodes (`glyph.matchMasters` guesses).
12. **Find outline matching.** FontForge matches point for point; decide whether ours should also match
    geometrically equal shapes with different point structure.
    **M13 pass (ours):** point for point only, as FontForge documents (`path.findOutlinePattern` guess).
