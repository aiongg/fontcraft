# Interpolation, masters, instances and variable fonts

Source: Glyphs 4 Handbook, chapter "Interpolation" (public web handbook), with labels cross-checked
against its "Font Info", "Feature Code" and "Appendix › Custom Parameters" chapters. Variable-font
terminology checked against fonttools (`designspaceLib` docs, `varLib`), the GlyphsSDK file-format
description and fontc's Glyphs reader. Date: 2026-10-07.
Inventory: `spec/inventory/10-interpolation.toml`. Ids in `code` below are rows in that file.

Rows for commands that another chapter documents as their home live in that chapter's file and
are only referenced here: `font.axes.*`, `font.master.*`, `font.instance.*` and the
`fontInfo.*` behaviours (06-font-info), `path.correctPathDirection*` and `filter.shapeOrder`
(03a-edit-view-drawing), `layers.addBackupLayer` and `layers.toggleVisibility` (04-palette),
`fontView.cell.badgeIncompatible` (05-font-view).

Coordinates are written "internal" (what the designer types on masters, instances and special
layers) and "external" (what a variable font shows its user). Designspace 5 calls them design
coordinates and user coordinates.

## 1. Concepts

- **Master**: a complete set of glyph layers plus metrics and kerning at one designspace location.
  A new document has one master, Regular (`fontInfo.masters.newFileHasRegular`).
- **Axis**: one varying aspect of the design (`axes.definition`). Any number of axes; the space
  they span is the designspace.
- **Static instance**: one exported font file at one location, at, between or beyond the masters.
- **Variable font**: one exported file holding the origin master plus the differences to all
  other masters, with the instances listed as named locations.

A handful of masters yields many instances: two masters on Weight are enough for six weights.

## 2. Axes

Edited in Font Info › Font › Axes. Each axis carries a name, a tag of four characters and a Hidden
flag.

| Action | Command |
|---|---|
| Add (plus button beside the heading) | `font.axes.add` |
| Rename, retag, hide | `font.axes.set` |
| Pick from the predefined list (fills name and tag) | `font.axes.set`; query `font.axes.listPredefined` |
| Remove | `font.axes.remove` |
| Reorder (not described; needed) | `font.axes.reorder` |

Rules:

- The predefined list holds the registered OpenType axes and axes proposed for registration. The
  chapter does not print the list.
- A private axis takes any name and a tag of four capital letters, e.g. `SWLN`
  (`axes.tagRules`). Treat a non-conforming private tag as a warning.
- Hidden asks applications not to show a control for the axis; they may ignore it. Default off
  (`fontInfo.axes.hidden`).
- Internal coordinates are free numbers (`axes.coordinatesAreFreeNumbers`): stem width for Weight
  (45 … 160), 0–100 for abstract axes or a smoothly slanting Italic, 0–1 for an axis whose outlines
  flip instead of blending.
- Internal coordinates are spread linearly between masters (`axes.linearSpread`). Anything
  non-linear is an export-time mapping (section 10).
- Internal and external coordinates coincide unless Axis Location or Axis Mappings is present
  (`axes.internalVersusExternal`).

Adding an axis gives every master, instance, virtual master and special layer a slot for it. The
chapter does not say which value existing objects receive.

## 3. Masters

Managed in Font Info › Masters (`font.master.add`, `font.master.setAxisLocation`,
`font.master.addFromOtherFont`). Each master has a name and an icon, both organisational
(`masters.namingAndIcon`).

- Interpolation needs two masters at least, placed so the wanted region lies between them
  (`masters.mustCoverDesignspace`): Light 300 and Bold 700 suffice for Weight; a Regular at 400
  refines it.
- **Minimal setup** (`masters.minimalSetup`): an origin master plus one master per axis, each
  differing from the origin on exactly one axis. Example: Thin Narrow (200, 60), Thin
  (200, 100), Black (900, 100); Thin is the only possible origin.
- **Corner setup**: masters at all corners give control where several axes are away from the
  origin at once (Bold Condensed). Masters inside the space refine further; a 3 × 3 grid of
  weights 300/400/700 and widths 50/80/150 is the chapter's large example
  (`masters.cornerAndIntermediateSetups`).
- The origin is a user choice only for variable fonts; static interpolation finds its own
  (`masters.originAutomaticForStatic`).
- `font.master.addFromOtherFont` copies a master out of another open document (Masters tab plus
  menu). This chapter labels the menu entry Add Other Master; the Font Info chapter's row labels
  it Add Other Font.

What interpolates: outlines, and by implication everything else an instance needs (widths,
anchors, component placement, vertical metrics, kerning, named stems and number values). Only
"outlines and metrics" is stated in this chapter (`masters.metricsInterpolate`, inferred).

The Font Info chapter adds a per-font choice between two interpolation models for static
instances: Default (the Adobe Multiple Master style model, used for new documents) and Variable
(the same model a variable font uses). They agree for minimal setups and differ when there are
more masters than the minimum. `interpolation.interpolateGlyph` takes the model as a parameter.

## 4. Instances and variable font settings

Font Info › Exports, plus menu at the bottom left:

| Entry | Command | Effect |
|---|---|---|
| Add Instance | `font.instance.add` | new static instance |
| Add Instance for Each Master | `font.instance.addForEachMaster` | one instance per master at the master's location |
| Add Variable Font Setting | `font.instance.addVariableFontSetting` | an export entry describing one variable font |
| Instance as Master | `font.instance.toMaster` | interpolates the selected instance and adds it as a master |

`font.instance.setAxisLocation` sets an instance's location.

Static instances:

- One coordinate per axis (`instances.multiAxisCoordinates`).
- May lie outside the masters: extrapolation (`fontInfo.instance.extrapolation`). Allowed for static
  export only; extrapolated instances are left out of variable fonts
  (`instances.extrapolatedExcludedFromVariable`).
- Weight Class and Width Class are independent fields (`instances.weightWidthClassIndependent`).
- Instances inside the designspace are also written into variable fonts as named instances
  (`instances.namedInstancesInVariable`).
- Their custom parameters, filters included, apply only when the instance is exported as its own
  file (`instances.parametersStaticOnly`).

Variable font settings:

- Each one exports one variable font; give them different names
  (`variableFontSetting.splitFamily`). With none defined, export uses an implicit default setting
  (`variableFontSetting.defaultWhenNone`).
- They accept whole-font parameters such as filters, glyph removal and added features. A
  parameter that breaks compatibility is an export error
  (`variableFontSetting.wholeFontParameters`).
- `Disable Masters` (names separated by comma + space) removes masters from that export; axes the
  remaining masters do not vary on disappear (`variableFontSetting.disableMasters`,
  `axes.unusedAxesDropped`).

Turning instances back into sources:

- `file.generateInstances` (File › Generate Instances) interpolates the instances, converts them
  to masters and creates new documents from them.
- `font.instance.toMaster` does it for one instance inside the same document. After adding a
  Regular master this way between Thin and Bold, Light interpolates Thin–Regular and Semibold
  Regular–Bold.

Family grouping (`family.groupingByFamilyName`): exported fonts group by family name from Font
Info › Font › General, overridable per instance or variable font setting with `familyNames`. Only
the Default-language name groups; style linking supplies the Bold and Italic relations.

Headless additions: `interpolation.interpolateGlyph` (one glyph at any location) and
`interpolation.describeDesignspace` (the whole setup as data).

## 5. Outline compatibility

### 5.1 The rule

Layers of a glyph interpolate only if they agree in (`compatibility.rules`):

1. path count and, within each path, the node order (read as: node count and the sequence of line
   and curve segments from the start node);
2. anchors, by count and by name;
3. number of components and the glyph each references;
4. the order of shapes, paths and components in one list.

Start node and path direction follow from 1: the same nodes in a different rotation or direction
pair the wrong points.

### 5.2 Indicators

- Edit View: red bar above the ascender line (`compatibility.editViewRedBar`).
- Font View: red top-left corner on the cell (`fontView.cell.badgeIncompatible`).
- Smart Filter "Incompatible masters", present by default
  (`compatibility.smartFilterIncompatibleMasters`).
- Scope (`compatibility.checkScope`): only pairs of masters that some instance interpolates
  between are tested. `Enforce Compatibility Check` checked tests all masters; added and
  unchecked hides the indicators (colour fonts).
- Export of interpolated instances and of variable fonts fails while incompatible glyphs remain
  (`compatibility.exportBlocked`).

`interpolation.checkCompatibility` returns the same verdicts as data, with the first mismatch per
layer pair.

### 5.3 Fixes

| Fix | Command | Notes |
|---|---|---|
| Correct Path Direction, Cmd+Shift+R | `path.correctPathDirection` | sets directions from path nesting, moves each start node to a canonical node (normally leftmost of the lowest), sorts shapes (normally bottom-left to top-right) |
| … for All Masters, Cmd+Alt+Shift+R | `path.correctPathDirectionForAllMasters` | Option-key alternate; all master, intermediate and alternate layers of the selected glyphs |
| Filter › Shape Order | `filter.shapeOrder`, panel `window.shapeOrder`, headless `glyph.setShapeOrder` | drag shapes until every layer has the same order |
| Drag a connecting-line end in Master Compatibility view | `glyph.swapShapes` | swaps two correspondences on a non-current layer |

There is no command that repairs a node-count mismatch. The source's method is diagnostic: find
the yellow or first red segment, select nodes and watch where they land on the other masters
(`masterCompatibility.selectionProbe`), then add or remove the node by hand.

The Shape Order window has two panel rows at present: `dialog.shapeOrder` (03a-edit-view-drawing,
one layer, OK button) and `window.shapeOrder` (here, all layers of the glyph side by side, as
this chapter describes it). They are the same window and should be merged once observed.

### 5.4 Master Compatibility view

View › Show Master Compatibility, Cmd+Ctrl+Alt+N (`view.showMasterCompatibility`, panel
`editView.masterCompatibility`). It draws the glyph on all masters together:

- Shapes are tinted by shape-order index; components are checkered
  (`masterCompatibility.shapeColours`).
- The current layer shows the index beside each path's start node and in each component's centre
  (`masterCompatibility.indexLabels`).
- Slanted lines connect matching start nodes, shape centres and anchors across masters; anchor
  lines are dashed; selected points get lines too (`masterCompatibility.connectingLines`).
- Segment colours (`masterCompatibility.segmentColours`): green compatible; yellow compatible, yet
  from one master to another the segment turns by over 20° (a node is probably missing); red
  incompatible (no counterpart, or line against curve). Everything after a missing node on that
  path is red.
- A shape or anchor missing on a master sends its line to that master's (0, 0)
  (`masterCompatibility.missingElementPointsToOrigin`).
- Mismatched component references and anchor names are labelled in red
  (`masterCompatibility.redMismatchLabels`).
- Line ends on non-current layers can be dragged onto another line's end to swap the two
  (`masterCompatibility.dragLineEnds`).
- Crossing lines reveal "shape-shifters": compatible layers whose same-index shapes sit in
  different places (`masterCompatibility.shapeShifters`).
- Intermediate and alternate layers are shown too; alternate layers stand apart after a gap
  (`masterCompatibility.alternateLayersOffset`).

## 6. Intermediate (brace) layers

Purpose: an extra support point for one glyph at one location, instead of a new master
(`intermediateLayer.purpose`).

Setup:

1. Select the most similar master layer in the Layers palette.
2. Add a layer with the plus button (`layers.addBackupLayer`, 04-palette).
3. Context menu › Intermediate (`layers.setIntermediate`); one number field per axis appears with
   the axis range beside it (`popover.intermediateLayer`). Return confirms.

Rules:

- An empty field inherits that axis's coordinate from the master the layer is attached to
  (`intermediateLayer.inheritsEmptyCoordinates`).
- Label: coordinates in axis order in curly braces, `{110}`, `{65, 140}`
  (`intermediateLayer.braceLabel`). Double-click the label to edit
  (`layers.editSpecialLayerSettings`).
- It must be compatible with the layers it interpolates with
  (`intermediateLayer.mustBeCompatible`, inferred).
- Reverting to a plain backup layer: `layers.clearIntermediate` (inferred).
- Export: affects that glyph only (`intermediateLayer.sparseInExport`). Components pointing at a
  glyph with intermediate layers are decomposed in variable export by default (appendix parameter
  Decompose Brace Layers).

Data read and written: per layer, an optional list of per-axis coordinates where each entry is a
number or "inherit".

### Virtual masters

A font-level location that acts as a master for the glyphs that have an intermediate layer there
and for nothing else: no master layers, no kerning (`virtualMaster.concept`). The chapter's case
is an axis for crossbar height that concerns A, E, F, H only.

Setup (`virtualMaster.setupSteps`): add the axis; give all real masters a coordinate on it; add a
`Virtual Master` parameter in Font Info › Font and set its coordinates (`dialog.virtualMaster`);
add intermediate layers at those coordinates to the affected glyphs.

Effects (`virtualMaster.extendsAxisRange`): the virtual master's coordinates extend the axis
range, so an axis on which all real masters coincide still has an extent; the variable font
stores deltas for the affected glyphs only.

## 7. Switching shapes

Three methods (`switching.threeMethods`):

| Method | Static | Variable |
|---|---|---|
| Alternate layers | yes | yes |
| Rename Glyphs / Remove Glyphs on an export entry | yes | yes (per variable font setting, whole font) |
| `condition` in feature code | no | yes |

### 7.1 Alternate (bracket) layers

An alternate layer is a second drawing of a glyph plus the region where it takes over
(`alternateLayer.definition`). Removing the range again: `layers.clearAlternate` (inferred).

Setup, per master: select the master layer, add a layer, context menu › Alternate
(`layers.setAlternate`). A min and a max field per axis open (`popover.alternateLayer`); either
may be empty. Example: Weight axis 44–130, switch from 90 up: min 90, max empty. Return confirms.

Rules:

- One alternate layer under each master, so the alternate shape interpolates everywhere
  (`alternateLayer.onePerMaster`).
- Label, bold: ranges in square brackets with `‹` around an axis abbreviation: `[120‹wg]`,
  `[140‹wg, 60‹wd‹85]` (`alternateLayer.bracketLabel`). Abbreviations: ital→it, opsz→oz,
  slnt→sl, wdth→wd, wght→wg; other axes use the full tag (`alternateLayer.axisAbbreviations`).
- Compatible with each other, not necessarily with the master layers
  (`alternateLayer.compatibilityAmongThemselves`).
- A layer may be alternate and intermediate at once; both labels show and each is edited by
  double-clicking it (`alternateLayer.combinedWithIntermediate`).
- A master layer may carry the range itself. Its partner is an extra Alternate layer with both
  bounds empty, labelled `[]`; the master layer then holds the alternate drawing and `[]` the normal
  one. Exports are identical either way (`alternateLayer.onMasterLayer`).
- Range edges: both inclusive, as fontTools evaluates designspace rules and as OpenType feature
  variations store them (`alternateLayer.rangeSemantics`, inferred; a guess until observed).

Export:

- Static: the instance uses the alternate layers when its location is inside the range
  (`alternateLayer.staticExport`).
- Variable: a feature variation switches the glyph inside the range. The feature is set by
  `Feature for Feature Variations`; `rvrn` is a common choice
  (`alternateLayer.variableExport`). Features run at different stages of shaping depending on
  system and application, which is why the choice is exposed.

### 7.2 Replacing glyphs at export

On an instance or variable font setting (`switching.renameGlyphsAtExport`): `Rename Glyphs` with
one `name=othername` per line swaps two glyphs in that export (`cent=cent.simple`). Add
`Remove Glyphs` listing the names that now hold the unwanted outline to drop them.

### 7.3 Conditional substitutions

Variable fonts only (`switching.conditionalSubstitution`):

```
#ifdef VARIABLE
condition 520 < wght;
sub cent by cent.simple;
#endif
```

Syntax from the Feature Code chapter: `lower < tag < upper`, either bound optional, several
ranges separated by commas and all required; the lower bound behaves as ≤. Pair it with Rename
Glyphs on the heavy static instances to get the same result in static fonts.

## 8. Editing several masters

- Layers palette lists masters, with special layers indented below them
  (`mastersAsLayers.paletteListing`); the eye button keeps a non-active layer visible
  (`layers.toggleVisibility`).
- Edit › Show All Masters (`edit.showAllMasters`) puts all master layers of the current glyph,
  and its intermediate and alternate layers, into the Edit View tab.
- Edit › Keep Layer Selection in Sync (`edit.keepLayerSelectionInSync`) carries the selection
  over when switching masters; works on compatible glyphs only.
- `layers.reInterpolate` (recalled, not in the chapter) refills a layer from the interpolation of
  the others.

## 9. Higher-order interpolation (HOI)

Tool `higherOrderInterpolation`, shortcut I. Needs one axis and two masters. Applies to static
and variable export (`hoi.appliesToAllExports`).

Default behaviour: each node moves on a straight line at constant speed between its positions on
two layers (`hoi.defaultLinear`). HOI lets a node follow a curve, which also changes its speed
profile.

Display:

- Diamonds mark node positions: blue for the current layer, light yellow for the next layer down
  the selected axis, dark purple for the next layer up; further layers are omitted
  (`hoi.nodeMarkers`). The axis is chosen in a pop-up at the bottom right of the window
  (`hoi.selectAxis`).
- Each linear path is a grey dotted line with a grey knob in the middle
  (`hoi.knobOnDottedLine`).

Types (`hoi.curveTypes`, `hoi.setInterpolationType`):

| Type | How to get it | Data |
|---|---|---|
| Linear | default; Delete on a knob (`hoi.removeCurve`); type Linear | none |
| Intermediate Point | drag the knob | one point |
| Handles | Return on an intermediate point (`hoi.convertToHandles`); Info box; context menu › Interpolation › Handles | two points |
| Rotation | Info box or context menu › Rotation | centre point, direction |

Rotation: the centre starts at the centre of the path's bounding box; a button puts it back
there (`hoi.centerRotation`). Direction is Automatic (smaller angle), Clockwise or
Counterclockwise (`hoi.setRotationDirection`). A true arc is not always possible; then it is
approximated (`hoi.rotationApproximation`).

Timing (`hoi.timingGraph`, panel `infoBox.higherOrderInterpolation`): a read-only graph beside
the Info box; diagonal for linear, shallow where slow, steep where fast. A white dot animates
linearly and a green dot follows the graph.

Headless: `hoi.setControlPoints`.

## 10. Variable font options

### 10.1 Origin

- The origin master's outlines are stored whole; other masters as deltas
  (`variable.originMaster`). Default: first master in the Masters tab. Override: `Variable Font
  Origin` in Font Info › Font.
- Software without variable support shows the origin, which argues for a regular master. A corner
  master needs less delta data and gives a smaller file (`variable.originIsFallback`).
- The setup must be at least minimal relative to the origin
  (`variable.originNeedsMinimalSetup`).

### 10.2 Axis Location

Without it, exported axes use internal coordinates unchanged. An `Axis Location` parameter on
every master gives, per axis, the external coordinate the master stands for
(`axisLocation.perMasterRemap`, editor `dialog.axisLocation`). Typical use: stem-based Weight
exposed on the 1–1000 scale. The external range of an axis is the span of these values
(`axisLocation.definesExternalRange`).

### 10.3 Axis Mappings

- Default is identity: Weight 44–130, user picks 95, interpolation at 95
  (`axisMappings.identityByDefault`).
- An `Axis Mappings` parameter holds, per axis, control points pairing an internal with an
  external coordinate, linear in between (`axisMappings.controlPoints`). Off-diagonal points make
  equal slider steps produce unequal design changes.
- Per axis only; no dependence on other axes (`axisMappings.perAxisOnly`).
- Font-level in Font Info › Font; a variable font setting may carry its own
  (`axisMappings.fontAndExportLevel`).
- The external column already includes Axis Location
  (`axisMappings.externalIncludesAxisLocation`).

Dialog `dialog.axisMappings`: axis list; table of control points (internal left, external
right); graph. Plus adds a point (`axisMappings.addPoint`), as does a click on the blue line;
values are edited in the table or by dragging a point up and down (`axisMappings.setPoint`);
arrow keys move the selected point, ×10 with Shift, ×100 with Cmd (`axisMappings.nudgePoint`);
minus deletes (`axisMappings.removePoint`). Query: `axisMappings.convertCoordinate`.

Precedence used by fontc when reading Glyphs sources, worth matching: Axis Mappings, else Axis
Location (only if every master has it), else locations of instances, else external = internal.

### 10.4 Style attributes

- Names positions on each axis so any setting gets a style name (`stat.purpose`).
- A name may be elidable: dropped when combined with others (`stat.elidableNames`).
- Built automatically from instance names and coordinates (`stat.automaticFromInstances`).
- Overrides on instances (`stat.manualOverrides`): `Style Name as STAT entry` = axis tag, for an
  instance differing from the origin on that axis only; `Elidable STAT Axis Value Name` = axis
  tag, once per axis on which the name is elidable.

## 11. Several fonts

- **Compare Fonts** (Edit › Compare Fonts…; `edit.compareFonts`, `window.compareFonts`): two
  document pop-ups (open documents only), a master pop-up under each, and a two-column list
  grouped into font info, master metrics, glyph outlines and kerning pairs, each group
  collapsible. Select a row, then Use Left or Use Right copies that value across
  (`compareFonts.useLeft`, `compareFonts.useRight`). Same document twice compares two of its
  masters. Headless: `compareFonts.diff`.
- **Compare Family** (Edit › Compare Family…; `edit.compareFamily`, `window.compareFamily`): one
  column per open document, one row per glyph. Attribute menu: Component Names, Anchors, Left
  Kerning Group, Right Kerning Group, Glyphs (`compareFamily.setAttribute`). Empty value `--`;
  absent glyph `(missing glyph)`, or a struck-through red ring in Glyphs mode. Filter: All,
  Missing in One, Ignore If Missing in One (`compareFamily.setFilter`). Double-click opens the
  glyph (`compareFamily.openGlyph`). Pin keeps the window on top (`compareFamily.togglePin`).
  Update reloads; nothing refreshes by itself (`compareFamily.update`). Headless:
  `compareFamily.report`.

## Model implications

The source model must represent the following. Field names are suggestions.

1. **Axis**: `name`, `tag` (4 chars), `hidden`, order in the font. No range is stored: ranges
   are derived from masters and virtual masters.
2. **Master**: `id`, `name`, `icon`, `location` (one internal number per axis), per-master
   metrics, stems, number values, kerning, custom parameters, including `Axis Location` (one
   external number per axis).
3. **Virtual master**: a font-level `location` with no layers, metrics or kerning. It takes part
   in axis-range computation and in the variation model of exactly the glyphs that have a layer
   at that location. Zero or more per font.
4. **Layer**: belongs to a glyph, has its own `id` and an `associatedMaster`. A master layer is
   the one whose id equals the master's. Any layer may carry two optional attributes:
   - `coordinates`: per axis, a number or "inherit from associated master" (intermediate);
   - `axisRules`: per axis, optional `min` and optional `max` (alternate). An empty rule list on
     a layer flagged alternate is meaningful (the `[]` partner of a ranged master layer), so
     "is alternate" cannot be derived from "has a bounded rule".
   Both may be set on one layer. A master layer may carry `axisRules`.
5. **Glyph-level derived views** the engine needs: for a glyph, the set of *sources* = master
   layers plus intermediate layers, each with a resolved full location; and for each distinct
   `axisRules` value, a parallel set of sources forming the alternate shape. Compatibility is
   judged inside each set, never across sets.
6. **Instance**: `name`, `location` (internal), weight and width class (independent), names,
   custom parameters, export flag; classified as inside or outside the designspace.
7. **Variable font setting**: an export entry with a name, names, custom parameters (`Disable
   Masters`, `Axis Mappings`, filters, glyph renames and removals) and no location.
8. **Mapping**: per axis an ordered list of (internal, external) pairs; font-level with a
   per-variable-font override. Must compose with `Axis Location`.
9. **Origin**: optional reference to a master; default first master.
10. **Feature for feature variations**: one tag (possibly a list, as fonttools allows).
11. **STAT hints** on instances: zero or more (axis tag → "name is the label") and (axis tag →
    "name is elidable") entries. Repeatable parameters must be storable, as must several
    `Virtual Master` entries: custom parameters are an ordered multi-map, not a map.
12. **HOI data**: per node, per axis and per neighbouring-layer segment, an interpolation type
    and up to two control points, or a centre plus a direction. Where it is stored is unknown
    (open question); the model must leave room for per-node attributes keyed by axis.
13. **Compatibility scope flag**: tri-state from `Enforce Compatibility Check` (absent, on, off).
14. **Interpolation model choice** for static instances (Default or Variable), from Font Info.
15. Glyph swaps (`Rename Glyphs`, `Remove Glyphs`) and filters are export-time transformations
    on a copy of the font; the model needs an instancing pipeline in which they run after
    interpolation for static instances and before the variation build for variable fonts.

Invariants worth enforcing as checks: unique master locations; every master has a coordinate for
every axis; alternate-layer sets cover every master; intermediate coordinates fall inside the
axis range (masters plus virtual masters); mapping points are monotonic; the origin has, for each
axis, a master that differs from it on that axis only.

## Mapping to designspace 5 and OpenType

Stated only where we are confident. "Internal" = design coordinates (`xvalue`), "external" = user
coordinates (`uservalue`).

| Concept | Designspace 5 | OpenType |
|---|---|---|
| Axis | `<axis name tag minimum default maximum hidden>`; min/default/max in user coordinates | `fvar` axis record: tag, min, default, max, hidden flag, name id |
| Axis default | user coordinate of the origin master on that axis | `fvar` default value; normalised 0 |
| Internal coordinate | design coordinate in `<location><dimension xvalue>` | not stored; becomes a normalised coordinate in −1…+1 |
| Axis Location, Axis Mappings | `<map input="user" output="design"/>` children of `<axis>`; a master's Axis Location gives one such pair, mapping control points give more | `avar` segment map per axis, from normalised user to normalised design coordinates; absent or identity when external = internal |
| Master | `<source filename familyname stylename>` with a `<location>` | contributes a region and deltas to every variation table |
| Origin master | the source located at every axis's default | `glyf` outlines, `hmtx` widths and all default values are the origin's |
| Master outlines | per-source glyph | `gvar`: per glyph, tuple variations (peak tuple, optional intermediate start and end) holding point deltas, including the four phantom points |
| Master advance widths | per-source glyph advance | `HVAR` (deltas through an item variation store); optional because `gvar` phantom points carry the same |
| Master vertical metrics, x-height, cap height, underline, strikeout… | per-source font info | `MVAR`, by value tag |
| Master kerning and anchors | per-source kerning and anchors | `GDEF` item variation store referenced from `GPOS` variation-index device tables |
| Intermediate layer | a sparse source: `<source layer="…">` naming a layer in a master's file that contains only the glyphs concerned, at the layer's resolved location | extra `gvar` tuples for those glyphs only, with intermediate regions |
| Virtual master | sparse sources at a location no full source occupies; the axis range must include it | wider `fvar` axis range; `gvar` data for the affected glyphs only; no kerning or metric deltas there |
| Alternate layers | extra glyphs (fontc and glyphsLib name them `<glyph>.BRACKET.varAltNN`) plus `<rules>` → `<rule>` → `<conditionset>` of `<condition name minimum maximum>` in design coordinates → `<sub name="glyph" with="alternate">` | `GSUB` FeatureVariations: records pairing a condition set (axis index, min, max in normalised coordinates) with a feature-table substitution that points the feature at a lookup replacing the glyph |
| Feature for Feature Variations | `<rules processing="first">` → `rvrn`, `"last"` → `rclt`; any other tag through lib key `com.github.fonttools.varLib.featureVarsFeatureTag` | the feature whose lookup list is substituted |
| `condition` in feature code | no designspace form; lives in feature source | `GSUB` (or `GPOS`) FeatureVariations, same structure |
| Static instance | `<instance familyname stylename filename>` with a location | static font; inside a variable font, an `fvar` named-instance record (subfamily name id, user coordinates, optional PostScript name id) |
| Alternate layers in a static instance | rules evaluated at the instance location, glyphs swapped | none; outlines are already swapped |
| Variable font setting | `<variable-fonts>` → `<variable-font name filename>` with `<axis-subsets>` and a `<lib>` for names | one output file |
| Disable Masters | no single element; export a designspace without those sources. `<axis-subset>` restricts or pins axes in user coordinates and is the closest construct | fewer regions; axes with no remaining variation are omitted from `fvar` |
| Style attributes | `<axis>` → `<labels ordering>` → `<label uservalue [userminimum usermaximum] [linkeduservalue] name elidable>`; top-level `<labels>` for multi-axis names; `elidedfallbackname` on `<axes>` | `STAT`: axis records and axis value tables (formats 1–3 per axis, 4 for locations), elidable flag, elided fallback name id |
| Elidable STAT Axis Value Name | `elidable="true"` on the label at the instance's value on that axis | `ELIDABLE_AXIS_VALUE_NAME` flag |
| Style Name as STAT entry | a `<label>` on that axis named after the instance, at its user value | a format 1 or 2 axis value |
| Hidden axis | `hidden="1"` | `fvar` hidden-axis flag |

Consequences for the compiler:

- Normalisation is anchored on the origin: origin → 0, axis minimum → −1, maximum → +1, each
  side scaled separately. Changing the origin changes every normalised coordinate, every region
  and the `avar` map.
- Alternate-layer and `condition` bounds are stored in internal coordinates and must pass through
  the same normalisation as master locations; they are not affected by `avar` a second time.
- In FeatureVariations the first record whose condition set matches is the one applied, so
  overlapping regions from several glyphs must be resolved into non-overlapping or correctly
  ordered records, as fonttools does.
- Glyphs with intermediate layers use a variation model with more sources than the font-wide
  one; the model must be built per glyph (fonttools uses sub-models for this).
- TrueType flavour only in the reference (`variable.alwaysTrueTypeFlavour`), hence `gvar`. A CFF2 variable flavour would put the
  same deltas in `CFF2` blend operators; not something the reference does.
- Designspace 5 also allows discrete axes (a list of values, no interpolation). The reference has
  no such notion: its 0–1 Italic axis is continuous. Import must accept discrete axes; export
  never needs them.

Not mapped with confidence: HOI data (no designspace construct; see open questions), and the
exact STAT formats the reference emits.

## Open questions

Answers from the M9 planning pass follow each question (`docs/plans/M9.md` § 2 and § 3 give the
evidence). "Guess" means a choice of ours recorded in the row's `guesses`; § 8 of the plan lists
the check in the reference app that would settle it.

1. Default feature for alternate-layer switching: the chapter says `rlig`, the appendix says
   `rvrn`. Observe an export.
   **Oracles:** fontc and glyphsLib both default to `rvrn`. **Decided** (`docs/plans/M9.md` § 7,
   question 2 (a), decided by the orchestrator on the owner's behalf): `rvrn`. An export from the
   reference app would still settle which page is right.
2. Direction of the mapping wording: the chapter calls internal coordinates the master values and
   external the result of mapping, and also says external values are "used for interpolation".
   We assume the OpenType sense (user picks external; font maps it to internal). Confirm with an
   exported `avar`.
   **Answered (format and oracles):** the file stores Axis Mappings per axis from external to
   internal value, and both oracles write `avar` from normalised external to normalised internal
   (Oswald: external 700 maps to the master at internal 710; `fvar` runs 200 to 700).
3. Alternate-layer range edges: inclusive or exclusive at min and max; behaviour of an instance
   exactly on a bound.
   **Guess:** both inclusive, as fontTools' rule evaluation and OpenType feature variations.
4. How Axis Location and Axis Mappings combine when both are present (fontc prefers Axis
   Mappings and warns), and whether Axis Location on instances feeds the map or only `fvar` and
   `STAT`.
   **Oracles:** Axis Mappings wins, then Axis Location when every master has it, then the
   instances' locations, then identity (fontc `glyphs-reader`; the model's `axes::mapping`
   already follows glyphsLib's equivalent). An instance's Axis Location gives its external
   coordinate (glyphsLib `get_user_loc`, the model's `axes::instance_user_value`), which feeds
   the map only when the masters set no mapping of their own. Built as the oracles do; M9.4
   confirms it on fontc's `AxisLocation*` and `InstanceMapping*` test sources.
5. `File › Generate Instances`: one new document per instance, or one document with all
   instances as masters; whether special layers, kerning and features are carried along.
   **Guess:** one document per instance (the Several Fonts section says a file per instance), with
   kerning and features and no special layers.
6. `Add Other Master`: handling of glyphs missing on either side and of axis coordinates.
   **Guess:** missing glyphs get empty layers; coordinates matched by axis tag.
7. Value given to existing masters, instances and special layers when an axis is added; what
   happens to them when an axis is removed or reordered.
   **Guess:** 0 on a new axis (intermediate layers inherit); removal drops the coordinate;
   reordering permutes every list.
8. Contents of the predefined axis list.
   **Ours:** the registered OpenType axes (ital, opsz, slnt, wdth, wght) plus the axes the
   registry lists as proposed (`font.axes.listPredefined` returns them); a guess on the extra
   entries until observed.
9. Shape Order window: layout, whether it includes special layers, selection and drag rules.
   Open; the M3 dialog stays, and the all-layers window is built from this note (blocks nothing).
10. Master Compatibility view: colour palette, whether dragging works for start nodes and
    anchors as well as shapes, what "the angle of a segment" is measured from, and whether the
    20° threshold is fixed.
    **Guess:** eight theme tints; shapes only are dragged; the chord angle; a fixed 20°.
11. Whether compatibility is also required of intermediate layers against all masters or only
    their neighbours, and how the "only masters needed by an instance" rule treats them.
    **Guess:** against every master layer of the glyph.
12. How to revert a layer from Intermediate or Alternate (unticking the menu item is assumed).
    **Guess:** unticking the item; `layers.clearIntermediate` and `layers.clearAlternate`.
13. Whether several alternate ranges per glyph may overlap, and which wins.
    **Oracles:** allowed; overlapping regions are split so the most specific (smallest) region
    applies, as fontTools' `overlayFeatureVariations` does. One corpus glyph has overlapping
    ranges (`BracketTestFontKerning`, glyph `x`).
14. HOI: where curve data is stored (per node, per layer pair, per axis); the exact curve
    mathematics for the three types; the rotation approximation; how curves are compiled into a
    variable font; interplay with intermediate layers; which key moves knobs.
    **Partly answered (format):** per node, keyed by axis tag, in the lower node of a span
    (GlyphsSDK version 4 description: `ip`, `rc`/`rd`, `tl`/`tu`/`al`/`au`). The mathematics,
    the variable encoding and the keys stay open. **Decided** (`docs/plans/M9.md` § 7,
    question 5 (b), decided by the orchestrator on the owner's behalf): M9 reads, keeps and
    writes the data; the tool, its commands, the Info box and the export are M13's.
15. Axis Mappings dialog: base arrow-key step, which arrow pair moves which coordinate, whether
    internal values of a point are editable and constrained to master positions, what happens
    with non-monotonic points.
    **Guess:** step 1; Up and Down change the external value; internal values editable, not tied
    to masters; a change that breaks the order is refused.
16. STAT derivation from instance names: the algorithm, which axis value formats are written,
    how range bounds are chosen, treatment of names that combine several axes.
    **Oracles:** both reproduce the reference's output (fontc `glyphs2fontir/src/stat.rs`,
    glyphsLib `builder/stat.py`); FontCraft ports fontc's.
17. Whether `Variable Font Origin` and `Virtual Master` may also sit on a variable font setting.
    **Guess:** font level only, as both oracles read them.
18. Extrapolation limits: whether the reference clamps or warns, and how extrapolation behaves
    with more than two masters on an axis and with the Variable interpolation model.
    **Handbook and oracle differ:** the handbook extrapolates static instances; fontmake clamps.
    **Decided** (`docs/plans/M9.md` § 7, question 6 (a), decided by the orchestrator on the
    owner's behalf): FontCraft extrapolates, and the difference from fontmake is in
    `40-compile.md` § 4a. With more masters, the variation model's outermost regions extend.
19. Compare Fonts: whether identical rows are listed, how outline differences are shown, what
    Use Left/Right copies for a glyph row (outline only, or metrics and anchors too), undo scope
    across two documents.
    **Guess:** differing rows only; shapes, anchors and width copied; one undo step in the
    receiving document.
20. Compare Family: which master of each document is shown, column order, and what "relevant
    information" means for the All filter.
    **Guess:** the first master; documents in the order they were opened; a glyph with a
    non-empty value in any document.
21. `Re-Interpolate`: exact label, location and scope (recalled, absent from the chapter).
    **Verified absent from the handbook copy:** the row is our own command (`own = true`).
