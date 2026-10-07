# Reusing shapes: components, smart components, corners, caps, segments, brushes, pixels

Source: Glyphs 4 Handbook, chapter "Reusing Shapes" (public web handbook), read 2026-10-07.
Rows: `spec/inventory/09-reusing-shapes.toml`. Ids in `code` are command, tool, panel or behavior
ids from that file unless another file is named. Statements tagged **[stated]** come straight from
the handbook; **[inferred]** are our reading; **[memory]** are recalled and absent from the handbook.

Commands this chapter leans on but which are defined in the Edit View inventory (03a/03b):
`component.setTransform`, `component.setBaseGlyph`, `component.editBaseGlyph`, `selection.selectNext`,
`edit.selectAll`, `edit.delete`, `path.alignSelection`, `glyph.setAnchors`, panels `infoBox.component`
and `infoBox.componentGlyphPicker`.

## 1. Data model

What the chapter implies each object must store.

| Object | Fields |
|---|---|
| Component (a shape on a layer, ordered among the layer's shapes) | base glyph name; offset x, y; scale x, y (percent); slant; rotation; automatic alignment on/off (per component); locked flag; chosen attachment anchor name (optional); smart values: property name → number |
| Smart glyph | list of properties (name, min, max); per layer and property a pole: `min`, `max` or none |
| Corner component (attached to one node or one extra node) | corner glyph name; scale x, y; alignment mode `left`, `right`, `both`, `none` |
| Cap component (attached to two adjacent nodes of one path) | cap glyph name; Fit flag; scale x, y; alignment mode |
| Segment component (attached to one path segment) | segment glyph name |
| Brush (attached to one path segment) | brush glyph name |
| Font | Disable Automatic Alignment option; `Propagate Anchors` parameter; Grid Spacing |
| Instance | `Keep Overlapping Components` parameter |

Terms: the *base glyph* of a component is the glyph it references. A *composite* (compound) glyph
is one built from components.

Glyph roles are decided by name prefix:

| Prefix | Role | Exported by default |
|---|---|---|
| `_` (any) | part glyph | no |
| `_smart.` , `_part.` | smart glyph | no |
| `_corner.` | corner glyph | no |
| `_cap.` | cap glyph | no |
| `_segment.` | segment glyph | no |
| `_brush.` | brush glyph | no |
| `pixel` (exact name) | pixel shape used by the Pixel tool | not stated |

CJK radical glyphs and Korean base glyphs are also smart glyphs, whatever their names.

## 2. Building composites

### 2.1 Automatic composition (`components.autoComposeOnAddGlyph`)

- Adding a glyph that the glyph info database describes as a composite creates it with its
  components already in place, in database order (eacute → e, acutecomb).
- All-or-nothing: if one required component glyph is missing from the font, nothing is inserted
  (`components.autoComposeNeedsAllParts`).
- Mark substitution while composing, when the preferred glyph exists in the font:
  1. a mark with the same dot suffix as the composite (`agrave.sc` → `gravecomb.sc`)
     (`components.preferMarkWithSameSuffix`);
  2. for uppercase composites, the `.case` mark (`Agrave` → `gravecomb.case`)
     (`components.preferCaseMarksForUppercase`);
  3. for composites based on i or j, a mark ending in `.i` or `.narrow`
     (`components.preferNarrowMarksForIJ`).
- i and j themselves compose as `idotless` / `jdotless` plus `dotaccentcomb`
  (`components.ijBuiltFromDotless`).

### 2.2 Create Composite (`glyph.createComposite`, `glyph.createCompositeAllMasters`)

- Glyph › Create Composite, Cmd+Ctrl+C. Deletes every path and component on the current layer and
  inserts the database components.
- With Option (Cmd+Ctrl+Alt+C) it does so on all masters of the glyph.
- Reads: glyph info database, font glyph set. Writes: the layer's shapes.

### 2.3 Add Component (`glyph.addComponent`, `glyph.addComponentAllMasters`)

- Glyph › Add Component, Cmd+Shift+C; Option variant Cmd+Alt+Shift+C adds on all masters.
- Opens `dialog.componentPicker`, which matches by glyph name or Unicode value and is the same picker
  the Text tool uses.
- The component is appended to the layer's shapes; order matters for alignment (section 5).

### 2.4 Component from Selection (`glyph.componentFromSelection`, `dialog.componentFromSelection`)

- Glyph menu and canvas context menu. Input: selected paths and components on one layer.
- A dialog asks for the new glyph's name, pre-filled with a suggestion; any name is accepted.
  A leading underscore makes the new glyph non-exporting.
- On OK: a glyph with that name is created holding the selected shapes; the selection in the
  original layer is replaced by one component referencing it; the new glyph is appended to the
  current Edit View tab and becomes the active glyph.

### 2.5 Recipes in Add Glyphs

Recipe syntax accepted by Glyph › Add Glyphs (the command belongs to the Font View inventory):

| Form | Example | Result |
|---|---|---|
| `component=glyph` | `Delta=increment` | `increment` with one component `Delta` (`components.recipeSingleComponent`) |
| `base+mark=glyph` | `q+tildecomb=qtilde` | components in the written order; marks anchor-aligned to the base (`components.recipeBasePlusMarks`) |
| `base+base=glyph` | `t+s.join=ts` | several base components side by side, for ligatures (`components.recipeMultipleBases`) |

The left side lists components separated by `+`; the right side of `=` is the new glyph's name.

### 2.6 Underscore glyphs (`components.underscoreNameNotExported`)

A glyph named with a leading underscore is non-exporting by default and exists to be used as a
component (for example an `_f.connect` shared by `fl` and `f_f`). The glyph's exports property is
the real switch; the name only sets its default.

## 3. Editing components

### 3.1 Selection

- Click selects (`components.clickSelects`).
- A rubber-band drag with the Select tool ignores components unless Option is held
  (`components.rubberBandIgnoresComponents`).
- Tab / Shift+Tab step to the next / previous component (`components.tabCyclesComponents`).
- Select All is two-stage on layers that contain paths: first press selects paths only, second
  press adds components (`components.selectAllTwoStage`).

### 3.2 Transforming

- Three routes: component Info box fields, bounding box, Palette transformations
  (`components.transformVia`).
- Info box fields (panel `infoBox.component`, 03b): base glyph name, X, Y, horizontal scale %,
  vertical scale %, slant to the right, counter-clockwise rotation, arrow button; plus an anchor
  menu and a Smart badge under the conditions in `components.infoBoxExtraControls`.
- Move by mouse or arrow keys (`component.move`): 1 unit, Shift 10, Cmd 100
  (`components.arrowKeyIncrements`).
- Auto-aligned components do not move, except the blue single-number case, which moves vertically
  only (rules AA-7, AA-8).
- Option-drag duplicates (`components.optionDragDuplicates`); Delete removes
  (`components.deleteKeyRemoves`).
- Flipping a component vertically swaps the roles of its top and bottom anchors (rule AP-5).

### 3.3 Swapping the base glyph

Click the glyph name in the component Info box and choose another glyph
(`component.setBaseGlyph`, 03b). What else is preserved is not stated
(`components.swapBaseKeepsPlacement`, inferred).

### 3.4 Navigating between base glyph and composites

- Double-click a component, or press the arrow button of its Info box, to insert the base glyph
  next to the composite in the tab text and activate it (`component.editBaseGlyph`, 03b;
  `components.doubleClickOpensBase`).
- Context menu "Show all glyphs that use this glyph as a component"
  (`glyph.showGlyphsUsingAsComponent`); headless query `glyph.listComposites`.

### 3.5 Locking (`component.lock`, `component.unlock`)

Context menu on a component that is not auto-aligned. A locked component cannot be moved
(`components.lockedCannotMove`). The handbook presents locking as the per-component counterpart of
disabling alignment font-wide.

### 3.6 Decomposing

| Command | Scope | Depth | Output |
|---|---|---|---|
| `glyph.decomposeGlyph` (Glyph › Decompose Glyph, Cmd+Shift+D) | all components of the layer | fully recursive | paths and anchors |
| `component.decompose` (context menu Decompose) | selected components only | one level; nested components stay components | not detailed |

### 3.7 Placeholders

Three error states, each drawn as a placeholder in place of the component:

| State | Condition | Row |
|---|---|---|
| empty base glyph | base glyph exists, its layer has no shapes | `components.placeholderEmptyBase` |
| no base glyph | the referenced glyph name is not in the font | `components.placeholderMissingBase` |
| bad reference | circular: the base glyph contains, at some depth, the glyph holding the component | `components.placeholderCircularReference` |

A placeholder is still clickable, and the Info box shows the referenced glyph name.

### 3.8 Paths and components on one layer

- Any path on the layer turns automatic alignment off (rule AA-2).
- While dragging a node or anchor, nodes inside components appear as snap targets
  (`components.nodesInsideComponentsSnap`).
- `path.alignSelection` with exactly one node and one component selected moves the component so its
  origin sits on the node; the node stays (`components.alignOriginToNode`). Origin = baseline ×
  left sidebearing for italic angle 0, or the base glyph's `origin` anchor when it has one.

## 4. Nested components and anchor propagation

- Components may nest to any depth (`components.nestingAllowed`).
- Circular nesting is an error state (3.7).
- Propagation rules are AP-9 to AP-11 below.

## 5. Automatic alignment: testable rules

Each rule states only what the handbook states. Anything needed for an implementation but missing
is listed under Open questions and cross-referenced as (Q-n).

### 5.1 When alignment applies (AA)

- **AA-1** A layer whose shapes are all components has automatic alignment on by default.
  (`components.autoAlignDefaultOnWhenComponentsOnly`)
- **AA-2** A layer that contains at least one path has automatic alignment off.
  (`components.pathOnLayerDisablesAutoAlign`)
- **AA-3** Context menu Disable Automatic Alignment on an auto-aligned component turns alignment
  off for it; Enable Automatic Alignment on a component turns it back on.
  (`component.disableAutomaticAlignment`, `component.enableAutomaticAlignment`)
- **AA-4** The Font Info › Other option Disable Automatic Alignment turns off both automatic
  component placement and automatic metrics synchronisation for all composites of the font.
  (`components.fontWideDisableAutoAlign`)
- **AA-5** The user setting Disable Automatic Alignment for Imported Files decides the state of
  components in fonts opened from OpenType or UFO files.
  (`components.importedFilesDisableAutoAlignSetting`)
- **AA-6** Display: auto-aligned components are green, manually placed ones gray, and the case of
  AA-8 blue. (`components.autoAlignColors`)
- **AA-7** An auto-aligned component cannot be moved, and position values typed into its Info box
  have no effect. (`components.autoAlignedCannotMove`)
- **AA-8** Exception to AA-7: if the composite has exactly one component and that component's glyph
  has category Number, the component can be moved vertically; its horizontal placement stays
  automatic. (`components.singleNumberComponentShiftsVertically`)

### 5.2 Metrics of an auto-aligned composite (AM)

- **AM-1** The composite's sidebearings are derived from its base components and follow any change
  to those glyphs' sidebearings. (`components.autoAlignMetricsFromBase`)
- **AM-2** Components placed through anchors (marks) do not change the composite's sidebearings; a
  mark may extend past the advance width. (`components.anchoredMarksDoNotAffectSidebearings`)
- **AM-3** Ordinary metrics keys cannot be used on an auto-aligned composite. The metrics fields
  show `auto` followed by the derived value in parentheses.
- **AM-4** The only accepted keys are `=+N` and `=-N`, which add N to or subtract N from the derived
  sidebearing. (`components.autoAlignMetricsOffsetKeys`)
- **AM-5** A single-component composite takes its metrics from that one component (stated through
  the examples Alpha/A/A-cy and nine from a flipped six). (`components.autoAlignSingleComponent`)

### 5.3 Anchor pairing and component placement (AP)

Shape order is the order components were added; it is the only ordering the rules use.

- **AP-1** A component whose glyph has an anchor named `_X` is placed so that this anchor coincides
  with the anchor named `X` of the base component: `_top` → `top`, `_bottom` → `bottom`, and so on
  for any name X. The result equals what mark-to-base positioning would give.
  (`components.underscoreAnchorSnapsToPlainAnchor`)
- **AP-2** The base component comes first; mark components follow it.
  (`components.autoAlignBaseThenMarks`)
- **AP-3** Marks stack in the order added: a later mark attaches to the mark placed before it. For
  this a stacking mark glyph must have both `_X` and `X`. (`components.marksStackInOrder`) (Q-2)
- **AP-4** Alternative anchors: a layer may have `X` and any number of `X_suffix` anchors (`top`,
  `top_alt`, `top_viet`), with a free suffix. The anchor a mark component attaches to can be changed
  with the anchor menu of the component Info box (`component.setAnchor`); the menu is shown only
  when more than one anchor is available. (`components.alternativeAnchorSuffix`,
  `components.anchorMenuOnlyWhenChoiceExists`) (Q-3)
- **AP-5** Flipping a component vertically swaps its top and bottom anchors, so that after the
  flip the anchor that is physically on top is treated as the top one. A flipped
  `commaaccentcomb` therefore attaches to the other side of the base.
  (`components.verticalFlipSwapsTopBottomAnchors`) (Q-4)
- **AP-6** Several base components are placed side by side as the same glyphs would be when typed
  one after another. (`components.multipleBasesSetSideBySide`) (Q-5)
- **AP-7** With several bases, each base is followed in shape order by its own marks; a mark
  attaches to the base that precedes it, not to a later one.
  (`components.marksBelongToPrecedingBase`)
- **AP-8** If a base component's glyph has an anchor `#exit` and the next base component's glyph has
  an anchor `#entry`, the second component is positioned so the two anchors coincide. A glyph may
  carry both, so the chain can continue (`_part.stem`, `_part.arch`, `_part.arch` → m).
  (`components.exitEntryAnchorsConnectBases`) (Q-6)
- **AP-9** Anchors named `top` and `bottom` of a component's glyph propagate to the composite: the
  composite behaves as if it had those anchors, for further composition and for mark attachment.
  (`components.topBottomAnchorsPropagate`) (Q-7)
- **AP-10** An anchor defined in the composite itself overrides a propagated anchor of the same
  name. (`components.ownAnchorOverridesPropagated`)
- **AP-11** Propagation is on by default and is switched off by a font-level `Propagate Anchors`
  parameter set to false. (`components.propagateAnchorsParameter`)
- **AP-12** An anchor named `origin` in a glyph redefines the origin used when a component of that
  glyph is aligned to a node (3.8). The same name redefines the origin of a corner glyph (7.1) and,
  in cap glyphs, marks the first stem node (8.1).
- **AP-13 [memory, not in the handbook]** Ligature glyphs carry numbered anchors per ligature part
  (`top_1`, `top_2`, …) and marks can target them. The handbook documents only `caret_1`,
  `caret_2` … for ligature carets. (`components.ligatureNumberedAnchors`) (Q-1)

### 5.4 The three supported setups

| Setup | Shape order | Placement |
|---|---|---|
| single component | `base` | AM-5; AA-8 if category Number |
| base and marks | `base, mark, mark…` | AP-1 to AP-5 |
| multiple bases | `base, marks…, base, marks…` | AP-6 to AP-8 |

## 6. Smart components

### 6.1 Smart glyph setup

- Detection by name or script category (`smart.glyphDetection`).
- Variants are extra layers on the smart glyph (`smart.layersHoldVariants`). They are created with
  the Layers palette plus button, get a date-and-time name and should be renamed.
- Smart layers must be compatible with the master layer (`smart.layersMustBeCompatible`).
- Settings window `dialog.smartGlyphSettings`, opened by `smartGlyph.showSettings` (context menu
  Show Smart Glyph Settings, or Edit › Info for Selection, Cmd+Alt+I). Two tabs:
  - **Properties**: plus button adds a property (`smartGlyph.addProperty`). Each has a name, a
    minimum and a maximum (`smartGlyph.setProperty`). Rules: min < max; negatives allowed; default
    range 0–100 (`smart.propertyRangeRules`); name is free (`smart.propertyNamesFree`).
  - **Layers**: for the selected layer, tick per property whether it is the minimum or the maximum,
    or neither (`smartGlyph.setLayerPole`).

### 6.2 Interpolation model

- Each property is an axis local to the glyph (`smart.axesAreLocalToGlyph`).
- Layers are not needed at every corner of the property space. For example, Width 200–600 and
  Height 50–250 need Base (200, 50), Broad (600, 50), High (200, 250); the shape at (600, 250) is
  derived from Broad and High. A layer at the combined extreme may be added to override it
  (`smart.cornerLayersInferred`).
- Values outside [min, max] extrapolate (`smart.valuesExtrapolate`).
- A 0–1 range is an on/off property (`smart.zeroOneRangeIsToggle`).

### 6.3 Using a smart component

- Added like any component (2.2, 2.3).
- The Info box shows a Smart badge (`smart.badgeInInfoBox`); clicking it or Info for Selection opens
  `dialog.smartComponentSettings` (`smartComponent.showSettings`). The same controls sit at the
  bottom of the Palette when it is open and Show Info is on (`palette.smartComponent`).
- Controls per property: slider (min…max), number field (any value), or a checkbox for 0–1 ranges.
- All routes write through `smartComponent.setValue`.

### 6.4 Width and Height (`smart.widthHeightFollowBoundingBox`)

Properties named exactly `Width` and `Height` are driven by the bounding box (View › Show Bounding
Box): horizontal resize changes Width, vertical resize changes Height. A smart component with
neither cannot be resized by its bounding box.

### 6.5 Smart handles (`smart.handlesFromPropertyAnchors`)

- In the smart glyph, put an anchor named exactly like a property (case-sensitive) on every layer;
  on layers that vary that property, move the anchor along with the outline.
- Selected components of that glyph then show gray handles; dragging one changes the property.
- Optional per property.

## 7. Corner components

### 7.1 Corner glyph

- Name `_corner.<name>` (`corner.glyphNaming`).
- One open path lying around the origin. Origin = (0, 0), or the `origin` anchor
  (`corner.originPoint`).
- Direction: same as the path would have in the host outline. Start node normally shares X or Y
  with the origin; the end node's side decides the direction of growth (end node right of the
  origin → corner grows to the left of the stem). Wrong growth direction is fixed by reversing
  the contour (`corner.pathDirection`).
- Additional closed paths are allowed; the open path must be shape 1
  (`corner.closedPathsAllowed`).

### 7.2 Attaching and managing

- `cornerComponent.add`: select a node → context menu Add Corner Component → picker
  (`dialog.cornerPicker`) → Return.
- Extra nodes at path intersections also accept corners (`corner.extraNodesAcceptCorners`).
- Displayed with a faint blue background; click / Shift-click to select; Delete removes
  (`corner.highlightAndSelection`).
- Copy a selected corner and paste it onto other selected nodes (`corner.copyPasteOntoNodes`).
- `cornerComponent.decompose` (context menu Decompose Corner) bakes it into the path.

### 7.3 Info box (`infoBox.cornerComponent`)

| Control | Command | Notes |
|---|---|---|
| name | `cornerComponent.setGlyph` | click to choose another corner glyph |
| arrow button | `cornerComponent.editGlyph` | opens the corner glyph in the tab |
| horizontal / vertical scale | `cornerComponent.setScale` | negative flips (−100 mirrors); Palette mirror buttons do the same (`corner.mirrorViaPalette`) |
| alignment mode | `cornerComponent.setAlignment` | see below |

Alignment modes, relevant on diagonal stems:

| Mode | Icon | Effect | Typical use |
|---|---|---|---|
| `left` | left arrow | start node follows the diagonal, end node stays | bottom-left, top-right serifs |
| `right` | right arrow | end node follows the diagonal, start node stays | bottom-right, top-left serifs |
| `both` | left-right arrow | blend of the two | ink traps |
| `none` | x-mark | neither node is adjusted | |

### 7.4 Entry and exit anchors

- **Entry** (`corner.entryAnchor`): where the stem meets the corner. Default: the origin. Override
  with an anchor `left`; if the open path faces right, name it `right`. It must lie on the line
  through the origin and the start node. Moving it from the start node toward the baseline widens
  the corner on the wide-angle side of a diagonal and shortens it on the narrow-angle side; the
  other direction reverses that.
- **Exit** (`corner.exitAnchor`): where the outline continues after the corner. Default: the end
  node. Override with an anchor `right` (`left` for a right-facing corner). The end node may then sit
  above or below the baseline (cupped serifs).

So in a left-facing corner glyph `left` = entry and `right` = exit; in a right-facing one the names
swap.

### 7.5 Scaling (`corner.scaleStretchesByDefault`, `corner.scaleInterpolatesWithIntermediateLayers`)

- Default: geometric stretch.
- If the corner glyph has Intermediate layers given as Width % and Height %, scaling interpolates.
  Stated example: a layer at 150 % × 100 %; a component at 125 % horizontal gets an interpolated
  outline; vertical scaling still stretches because no layer has a height other than 100 %.

## 8. Cap components

### 8.1 Cap glyph

- Name `_cap.<name>` (`cap.glyphNaming`).
- Attaches to two neighbouring nodes of one path (`cap.attachesToTwoAdjacentNodes`).
- Anchor setups, pick one:
  1. `origin`: position of the first stem node relative to the cap (`cap.originAnchorSetup`).
  2. `node1` and `node2`: both stem nodes; controls position and scaling against both, for stems of
     varying width (`cap.node1Node2AnchorSetup`).
- Handbook tip: put these anchors on a known metric (baseline, x-height) so the host stem does not
  depend on the cap's overshoot.

### 8.2 Using

- `capComponent.add`: select the two nodes → context menu Add Cap Component → `dialog.capPicker`
  (cap glyphs only) → Return.
- Blue highlight, click / Shift-click, Delete, copy and paste onto other segments
  (`cap.highlightAndSelection`, `cap.copyPasteOntoSegments`).
- `capComponent.decompose` (Decompose Cap).
- Info box (`infoBox.capComponent`): **Fit** checkbox (`capComponent.setFit`) sizes the cap to the
  stroke it is attached to. Caps are also "replaced, transformed and aligned like corner
  components": `capComponent.setGlyph`, `capComponent.setScale`, `capComponent.setAlignment`.

## 9. Segment components

- Glyph: `_segment.<name>`, one open path along the baseline, anchors `start` and `end`
  (`segment.glyphNaming`).
- `glyph.setAnchors` places `start` / `end` on the path's first and last node. If the anchors are
  not on those nodes, the component will not match the host segment's length. The two end nodes
  should be on the baseline or equally far from it (`segment.startEndAnchors`).
- Works on straight and curved host segments (`segment.appliesToStraightAndCurved`).
- Bending never adds nodes: lower accuracy on tight curves, but a constant node count across
  masters (`segment.bendingAddsNoNodes`).
- `segmentComponent.add`: select a segment (click it, or select both its nodes) → context menu Add
  Segment Component → `dialog.segmentPicker` → Return.
- Blue highlight, click / Shift-click, Delete, copy and paste onto other segments.
- `segmentComponent.decompose` (Decompose Segment Component).
- Info box (`infoBox.segmentComponent`): name (`segmentComponent.setGlyph`), arrow
  (`segmentComponent.editGlyph`). No scale, alignment or fit controls are mentioned.

## 10. Brushes

- Glyph: `_brush.<name>`, exactly one closed path, anchors `start` and `end` (`brush.glyphNaming`).
  Merge several shapes with Remove Overlap first.
- `glyph.setAnchors` puts `start` / `end` at the left and right of the layer; normally on the
  baseline or at equal height (`brush.startEndAnchors`).
- The outline between `start` and `end` stretches and bends; outline outside them keeps its shape
  (`brush.stretchZoneBetweenAnchors`).
- Only curve segments (with handles) bend; Option-click a line segment to give it handles
  (`brush.bendingNeedsHandles`).
- `brush.add`: select one or more segments → context menu Add Brush → `dialog.brushPicker` → Return.
  With nodes selected instead, the selected nodes become the end nodes of the brushes that are
  added. Typical host: an open path of one segment (`brush.expandsSegments`).
- Blue highlight; click on the brush outline to select; Shift-click; Delete; copy and paste onto
  other segments.
- `brush.decompose` (Decompose Brush).
- Info box (`infoBox.brush`): name (`brush.setGlyph`), arrow (`brush.editGlyph`).

## 11. Pixel tool and pixel fonts

- Tool `pixel`, shortcut X; Shift+B when the Pencil tool is active.
- Precondition: Grid Spacing ≥ 2 (Font Info › Other) (`pixel.requiresGridSpacingTwoOrMore`).
- Pixels are components of the glyph named `pixel` (`pixel.usesGlyphNamedPixel`).
- If `pixel` is missing, the first click offers to add it (`dialog.addPixelGlyph`); Add runs
  `pixel.addPixelGlyph`, which creates a square with side = grid spacing
  (`pixel.defaultShapeIsGridSquare`).
- Gestures:

| Input | Effect |
|---|---|
| click empty cell | add a pixel |
| click existing pixel | remove it |
| drag from an empty cell | add pixels along the drag |
| drag from an existing pixel | remove pixels along the drag |

- Headless equivalent: `pixel.setPixels` (inferred design).
- The `pixel` glyph may hold any shape. To draw sub-pixel detail in it, lower the grid spacing
  (default 1), then raise it again before using the tool (`pixel.customShape`).

## 12. Export

| Condition | Result | Row |
|---|---|---|
| Remove Overlaps on | all components decomposed to paths | `export.componentsDecomposedWithRemoveOverlap` |
| TrueType flavour, Remove Overlaps off (e.g. variable fonts) | non-overlapping components kept as composite references; overlapping ones decomposed | `export.trueTypeKeepsNonOverlappingComponents` |
| same, plus instance parameter `Keep Overlapping Components` | overlapping components kept too | `export.keepOverlappingComponentsParameter` |
| CFF flavour | `Keep Overlapping Components` has no effect | same row |
| glyph name starts with `_` | glyph not exported by default | `export.underscoreGlyphsOmitted` |

With `Propagate Anchors` off, composites get no inherited `top` / `bottom` anchors and therefore no
mark / mkmk rules from them (AP-11).

## Open questions

Alignment and anchors

- **Q-1** Numbered ligature anchors (`top_1`, `top_2`, `bottom_1` …): absent from the handbook.
  Confirm they exist, how a mark component picks one (by order of marks, or only through the anchor
  menu), and how Set Anchors generates them.
- **Q-2** Exact target search for a mark's `_X`: the nearest preceding component having `X`, the
  first base, or the propagated `X` of everything placed so far? AP-3 and AP-7 fit "the most recent
  preceding component that offers `X`", but that is not stated.
- **Q-3** Which anchors appear in the anchor menu for a `_top` mark: only `top` and `top_*`? Is the
  choice stored as an anchor name on the component? What happens when the stored name disappears
  from the base glyph?
- **Q-4** Does the top/bottom swap apply only to the names `top`/`bottom` and `_top`/`_bottom`, or to
  suffixed ones too? Is it triggered by any negative vertical scale, or also by a 180° rotation?
- **Q-5** "As if typed": is the offset of the next base exactly the sum of preceding base advance
  widths? Is kerning between the bases applied? What is the composite's advance width (sum of base
  widths)? How do right-to-left glyphs order?
- **Q-6** Are the unprefixed cursive anchors `exit` / `entry` also used to connect components, or
  only the `#`-prefixed ones the chapter shows? Does a `#exit` / `#entry` connection alter the
  composite's advance width, and are `#` anchors excluded from the generated `curs` feature?
- **Q-7** Do only `top` and `bottom` propagate, or also suffixed and numbered variants, `exit` /
  `entry`, and carets? After a mark is attached, does the composite's `top` become the mark's `top`
  (needed for stacking on composites)? How do offset, scale and flips of the component transform
  the propagated anchor?
- **Q-8** How metrics are derived when the single component is flipped, scaled or rotated (nine
  from six), and with slanted masters.
- **Q-9** Vertical placement of a base component under alignment: always baseline offset 0? What is
  preserved of scale, slant and rotation on an auto-aligned component?
- **Q-10** Is automatic alignment stored per component, per layer, or both? AA-1 and AA-2 are layer
  conditions while AA-3 acts on one component. What does the Enable item do on a layer with paths?
- **Q-11** When a path is added to a previously aligned layer, do the components keep their last
  aligned positions as explicit offsets?
- **Q-12** Lock on an auto-aligned component: is the menu item hidden, disabled, or a no-op? Does a
  lock block Info box edits and Palette transforms as well as moves?
- **Q-13** Does the font-wide Disable Automatic Alignment override per-component enable?
- **Q-14** Position of the Info box arrow button: this chapter says top-left, the Edit View chapter
  says top-right; and whether the base glyph is inserted to the left or right of the composite.

Composition and editing

- **Q-15** Create Composite when the glyph is not in the database or a part is missing: no-op,
  error, or partial insert? Does it also reset anchors and metrics keys?
- **Q-16** How the Component from Selection name suggestion is formed; behaviour when the name
  already exists; whether anchors move with the shapes; where the new component's origin sits;
  whether other masters are processed.
- **Q-17** Recipe grammar details: Unicode values or suffixes on the left side, whitespace, and
  whether more than one `+` chain mixes bases and marks freely.
- **Q-18** `component.decompose` output: does one-level decomposition also bring in anchors?
  Decomposition of components with smart values, and of corner/cap/segment/brush attachments inside
  the base glyph.
- **Q-19** Where "Show all glyphs that use this glyph as a component" shows its result.
- **Q-20** Tab order when a layer has both components and anchors.

Smart components

- **Q-21** The exact interpolation formula with more than one layer per pole, with a layer assigned
  to poles of several properties, and with the optional combined-extreme layer; which layer is the
  origin (all-minimum is not always the master layer: in the stated example Regular is the maximum
  of Width).
- **Q-22** Default value of a property on a freshly added component; behaviour when a property is
  removed or renamed while components hold values for it.
- **Q-23** Smart glyphs in multi-master fonts: are properties per glyph and smart layers per master?
- **Q-24** Smart handle mapping from drag distance to value; what constrains the drag direction.
- **Q-25** Export of smart components: always decomposed, or ever kept as references?

Corners, caps, segments, brushes

- **Q-26** The geometry of corner fitting: how the host node's two segments are trimmed, how the
  corner is rotated to the incoming segment, and the precise maths of the `left` / `right` / `both`
  alignment modes and of the entry-anchor offset.
- **Q-27** How "facing left / right" of a corner glyph is determined (from the end node's side?).
- **Q-28** Intermediate layer naming or attributes for corner scaling (how Width % / Height % are
  recorded) and whether negative scales interpolate too.
- **Q-29** Cap fitting maths: with `origin` only versus `node1` / `node2`; what Fit changes exactly;
  which of the two selected nodes counts as the first; what the alignment modes mean for caps.
- **Q-30** Whether the cap, segment and brush Info boxes have further controls (scale on segments
  and brushes, an arrow button on caps).
- **Q-31** Segment and brush bending maths (mapping of baseline distance to curve parameter, and of
  height above the baseline to normal offset); which side of the host path the brush body falls on;
  how brushes on consecutive segments join.
- **Q-32** What a paste does when the selection does not match the component type (a corner pasted
  with a segment selected, a cap pasted on non-adjacent nodes).
- **Q-33** Whether corner/cap/segment/brush attachments must match across masters for
  interpolation, and when they are applied at export relative to interpolation and overlap removal.

Pixel tool

- **Q-34** Tool behaviour when grid spacing is below 2 (disabled, warning, or silent no-op).
- **Q-35** Cell origin and snapping: are pixel origins at multiples of the grid spacing from (0, 0)?
  Does the tool recognise pixels by exact position only?
- **Q-36** Whether the `pixel` glyph is exported, whether pixel components are auto-aligned
  (a pixel-only layer satisfies AA-1), and how they are decomposed and merged at export.
- **Q-37** Whether Pixel and Pencil really share one toolbar button (suggested by the Shift+B
  shortcut only).

Export

- **Q-38** How "overlapping" is tested for TrueType component retention (bounding boxes or true
  outline intersection), and whether transformed (scaled, flipped, rotated) or nested components are
  kept as references.
- **Q-39** Whether a composite that references a non-exporting glyph is always decomposed.
