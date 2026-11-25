# Reusing shapes: components, smart components, corners, caps, segments, brushes, pixels

Source: Glyphs 4 Handbook, chapter "Reusing Shapes" (public web handbook), read 2026-10-07.
Rows: `spec/inventory/09-reusing-shapes.toml`. Ids in `code` are command, tool, panel or behavior
ids from that file unless another file is named. Statements tagged **[stated]** come straight from
the handbook; **[inferred]** are our reading; **[oracle]** come from fontc and fontmake output.

Commands this chapter leans on but which are defined in the Edit View inventory (03a/03b):
`component.setTransform`, `component.setBaseGlyph`, `component.editBaseGlyph`, `selection.selectNext`,
`edit.selectAll`, `edit.delete`, `path.alignSelection`, `glyph.setAnchors`, panels `infoBox.component`
and `infoBox.componentGlyphPicker`.

## 1. Data model

What the chapter implies each object must store.

| Object | Fields |
|---|---|
| Component (a shape on a layer, ordered among the layer's shapes) | base glyph name; offset x, y; scale x, y (percent); slant; rotation; automatic alignment setting (per component: off, by layer, forced, horizontal only); locked flag; chosen attachment anchor name (optional); smart values: property name → number |
| Smart glyph | list of properties (name, min, max); per layer and property a pole: `min`, `max` or none |
| Corner component (attached to one node or one extra node) | corner glyph name; scale x, y; alignment mode `left`, `right`, `both`, `none` |
| Cap component (attached to two adjacent nodes of one path) | cap glyph name; Fit flag; scale x, y; alignment mode (stored keys: § 8.5) |
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
- **AP-13 [oracle]** In a composite of category Ligature, the anchors propagated from each base are
  numbered by base (`top_1`, `top_2`, …), and a mark component whose anchor attribute is a
  numbered name attaches there. The handbook documents only `caret_1`, `caret_2` … for ligature
  carets; under AP-4 a numbered anchor is an alternative of the plain one.
  (`components.ligatureNumberedAnchors`) (Q-1)

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
- The glyph's first path is open. Its first and last nodes are the points that join the stem
  (W4), and they should reach past the butt line into the stem; otherwise `left` and `right`
  anchors give the directions in and out, as for corners (forum 18125).
- Anchor setups, pick one:
  1. `origin`: position of the first stem node relative to the cap (`cap.originAnchorSetup`).
     This runs the fitting of versions before 3.0.3, kept for cap glyphs without `node1` and
     `node2` (W5). Staff answers say the glyph is then drawn upside down around the origin, as
     for the bottom end of a stem (forum 7812, 11603, 17009).
  2. `node1` and `node2`: both stem nodes, the butt line of the receiving shape; they control
     position and scaling against both, for stems of varying width (`cap.node1Node2AnchorSetup`).
     W5 advises keeping them horizontal or vertical.
- Handbook tip: put these anchors on a known metric (baseline, x-height) so the host stem does not
  depend on the cap's overshoot.

### 8.2 Using

- `capComponent.add`: select the two nodes (or the straight segment between them, W5) → context
  menu Add Cap Component → `dialog.capPicker` (cap glyphs only) → Return.
- Blue highlight, click / Shift-click, Delete, copy and paste onto other segments
  (`cap.highlightAndSelection`, `cap.copyPasteOntoSegments`). The highlight is a wide, faint blue
  band along the fitted cap outline (W5 screenshot).
- `capComponent.decompose` (Decompose Cap) gives the fitted path (forum 20115).
- Caps are "replaced, transformed and aligned like corner components": `capComponent.setGlyph`,
  `capComponent.setScale`, `capComponent.setAlignment`.

### 8.3 Info box (`infoBox.capComponent`)

From the 3.0.3 release-notes screenshot (W5), a still image:

| Control | Command | Notes |
|---|---|---|
| name, arrow button | `capComponent.setGlyph`, open the cap glyph | as in the corner Info box |
| ↔ horizontal scale | `capComponent.setScale` | computed and greyed while Fit is on (forum 22406) |
| ↕ vertical scale | `capComponent.setScale` | applies with Fit on or off (W3) |
| four alignment buttons | `capComponent.setAlignment` | left arrow, double arrow (centre), right arrow, x |
| fit checkbox | `capComponent.setFit` | sizes the cap to the stroke it sits on |

With several caps selected, a field whose values differ shows `Multi`.

### 8.4 Fitting

What written sources state:

- With Fit on, the horizontal scale comes from the stroke and the stored horizontal scale is not
  used; the vertical scale still applies (W3). A horizontal scale such as -50 % cannot be combined
  with Fit (forum 30927).
- With `node1` and `node2` and centre alignment, the cap's angle and position follow the line
  between the two stem nodes, not the angles of the neighbouring segments (forum 18125).
- Alignment x keeps the cap's drawn orientation (forum 17009).
- Caps do not fit slanted stems seamlessly; two corners are the advice there (forum 21665, 2022).

What stays our choice, in the rows' `guesses`: the cut points on the stem sides
(`capComponent.add`), the origin-setup mapping (`cap.originAnchorSetup`), the reference width of
an origin-only cap under Fit (`capComponent.setFit`), the point scale applies about
(`capComponent.setScale`), left and right alignment and the default alignment
(`capComponent.setAlignment`).

FontCraft fits caps in `crates/model/src/attach.rs`, from those guesses, until an export of the
cap test font (`docs/plans/M13.md` § 8) settles them:

- The cap glyph's first path is placed with its reference point (`node1`, else `origin`, else
  the layer origin) on the first host node and the `node1` to `node2` line along the stem end; it
  is scaled in its own frame, then turned by the alignment.
- That path replaces the straight segment between the two nodes. The side before the first node
  is cut at its point nearest the cap's start node and ends there; the side after the second node
  starts at the cap's end node, cut the same way. Inserted nodes are rounded to whole units.
- The cap glyph's other paths are added to the layer, placed the same way.
- Left and right alignment lay the cap's y axis along the side they follow. On a clockwise path the
  side lies on the other hand of the stem end, so the axis points away from it: the cap's x axis
  runs from the first node to the second on every path.
- Fit on an origin-only cap: the horizontal scale is the stem end over the advance width. A cap
  with its ends at x 0 and 200, on a 400-unit stem end from (550, 550) to (150, 550):

  | Fit | Advance width | Scale | Cap ends at x |
  |---|---|---|---|
  | on | 200 | 400 / 200 = 2 | 550 and 150: both sides stay straight |
  | on | 600 (a new glyph's) | 400 / 600 = 0.67 | 550 and 417: the side after the cap slants to it |
  | off | any | 1 | 550 and 350: the same side slants |

  The handbook says that Fit matches the size of the cap to the size of the stroke, not how the
  size of an origin-only cap is measured; the observe steps of `capComponent.setFit` tell the
  advance width, the distance between the path's end nodes and the width of the drawn path apart.
- Outline resolution fits caps first, then moves the corner hints of the layer to the nodes they
  sat on and fits the corners. A layer whose caps cannot be fitted is drawn with its corners only.
- Limits: a scale (stored, or computed by Fit) beyond ±100 (10,000 %), a stem end shorter than
  10⁻⁶ units and more than 50,000,000 node moves per layer are refused with a message.

### 8.5 Stored form of caps, segments and brushes

Each is a hint on the layer, as a corner is (§ 7). The file-format specification (W1) lists the
hint `type` strings `Corner`, `Cap`, `Brush` and `Segment`, in both the Glyphs 2 and the Glyphs 3
format. `horizontal` is not written for these hints (W1).

| Key | Cap | Segment component | Brush |
|---|---|---|---|
| `type` | `Cap` | `Segment` | `Brush` |
| scripting constant (W2) | `CAP` = 17 | `SEGMENT` = 19 (since 3.1) | `BRUSH` = 18 (since 3.1) |
| `name` | the cap glyph | the segment glyph | the brush glyph |
| `origin` | first of the two nodes, in path order; the second is the next node (W4) | on-curve node ending the host segment (W4, forum 19101) | on-curve node ending the host segment (forum 21893) |
| `options` | alignment: left 0, right 1, centre 2, none 4; plus 8 for Fit (W2, W3) | not described | not described |
| `scale` | horizontal and vertical factors, default (1, 1) (W1, W3) | not described | not described |

glyphsLib's table of numeric hint types stops at `Cap` = 17; the two later values come from W2.
No source shows `options` or `scale` on a segment or brush hint; the guesses of
`segmentComponent.add` and `brush.add` record that FontCraft writes only `type`, `name` and
`origin` for them.

### 8.6 Written sources for § 7 to § 10

Read for M6.25. Facts only; none of their text is copied.

| Label | Source |
|---|---|
| W1 | GlyphsSDK file-format specification, versions 2 and 3 (`GlyphsFileFormat/GlyphsFileFormatv3.md`, hint and `indexPath`) |
| W2 | Glyphs scripting API documentation, `GSHint` and the hint-type constants (docu.glyphsapp.com), and the GlyphsSDK `ObjectWrapper` |
| W3 | mekkablue Glyphs-Scripts, *Cap and Corner Manager* (Apache-2.0) |
| W4 | mekkablue Glyphs-Scripts, *Convert Cap to Segment Component* (Apache-2.0) |
| W5 | Glyphs 3.0.3 release notes, section *Cap components*, with its two screenshots (glyphsapp.com/news/glyphs-3-0-3-released) |
| W6 | Tutorial *Reusing shapes: segment components* (glyphsapp.com/learn/reusing-shapes-segment-components) |
| W7 | Tutorial *Reusing shapes: brushes* (glyphsapp.com/learn/reusing-shapes-brushes) |
| W8 | Tutorial *Reusing shapes: corner components* (glyphsapp.com/learn/reusing-shapes-corner-components) |
| forum N | Thread N on forum.glyphsapp.com (`/t/N`); a staff answer is marked as such in the row |

Forum threads cited: 7812, 10109, 11603, 15594, 16992, 17009, 18125, 19101, 20115, 21014, 21665,
21893, 22406, 23125, 25610, 26036, 26221, 30927, 33636.

## 9. Segment components

- Glyph: `_segment.<name>`, one open path along the baseline, anchors `start` and `end`
  (`segment.glyphNaming`). The fitting assumes `start` on the left and `end` on the right
  (forum 26036).
- `glyph.setAnchors` places `start` / `end` on the path's first and last node. If the anchors are
  not on those nodes, the component will not match the host segment's length. The two end nodes
  should be on the baseline or equally far from it (`segment.startEndAnchors`).
- Works on straight and curved host segments (`segment.appliesToStraightAndCurved`). A 2025 staff
  answer says the menu item is not offered on smooth curve segments while pasting onto them works
  (forum 33636); FontCraft follows the handbook and records the conflict as a guess.
- The component is squeezed or stretched to the host segment's length; its height is not scaled
  (W6). It follows the host path's direction, so on a counter-clockwise outline the segment glyph
  is drawn turned 180 degrees (forum 26036).
- Bending never adds nodes: lower accuracy on tight curves, but a constant node count across
  masters (`segment.bendingAddsNoNodes`). Each point, handles included, keeps its own
  perpendicular distance from the host segment, so smooth handle-node-handle triples can kink (W6).
- `segmentComponent.add`: select a segment (click it, or select both its nodes) → context menu Add
  Segment Component → `dialog.segmentPicker` → Return.
- Blue highlight (a faint band along the fitted outline, W6), click / Shift-click, Delete, copy
  and paste onto other segments.
- `segmentComponent.decompose` (Decompose Segment Component) gives the fitted path (forum 20115).
- Info box (`infoBox.segmentComponent`): name (`segmentComponent.setGlyph`), arrow
  (`segmentComponent.editGlyph`). No scale, alignment or fit controls are mentioned; a 2021
  request to mirror applied segment components (forum 21014) agrees.

## 10. Brushes

- Glyph: `_brush.<name>`, one closed path, anchors `start` and `end` (`brush.glyphNaming`).
  Further shapes are ignored (W7); merge several shapes with Remove Overlap first.
- `glyph.setAnchors` puts `start` / `end` at the left and right of the layer; normally on the
  baseline or at equal height (`brush.startEndAnchors`).
- The outline between `start` and `end` stretches and bends; outline outside them keeps its shape
  (`brush.stretchZoneBetweenAnchors`). The host segment runs through the brush at the anchors'
  height (W7 screenshots).
- Only curve segments (with handles) bend; Option-click a line segment to give it handles
  (`brush.bendingNeedsHandles`). A brush wraps one host segment only (forum 23125).
- `brush.add`: select one or more segments → context menu Add Brush → `dialog.brushPicker` → Return.
  With nodes selected instead, a brush goes on each segment ending at a selected node, with its end
  anchor on that node (W7). Typical host: an open path of one segment (`brush.expandsSegments`).
- Blue highlight (a faint band along the brush outline, W7); click on the brush outline to select;
  Shift-click; Delete; copy and paste onto other segments. Cmd-A pressed repeatedly reaches all
  brushes (forum 21893).
- `brush.decompose` (Decompose Brush) gives the brushed outline (forum 20115).
- Info box (`infoBox.brush`): name (`brush.setGlyph`), arrow (`brush.editGlyph`).

### 10.1 How FontCraft bends segment components and brushes

`crates/model/src/attach.rs` bends both from the rows' guesses until an export of the segment
and brush test fonts (`docs/plans/M13.md` § 8) settles them:

- The glyph is read in a frame running from its `start` anchor to its `end` anchor: a point's
  position is its fraction of that length and its distance to the left of the line. Without
  anchors, a segment glyph uses its path's end nodes and a brush glyph (0, 0) and (advance
  width, 0).
- The host segment is measured by arc length. Each point, handles included, goes to the host
  point at the same fraction of the length and keeps its distance along the host's left normal.
  Only existing points move, so the node count never changes and a line stays a line.
- A segment component's open path replaces the host segment; its end nodes take the places of
  the host's end nodes. Points past `start` or `end` keep scaling along the host's end
  directions.
- A brush's closed path is added to the layer beside the host path, which stays. Outline before
  `start` and after `end` moves rigidly with the host's direction at that end.
- Outline resolution fits caps, then segment components, then brushes, then corners.
- Limits: a glyph whose start and end lie on one point, a host segment of zero length, a glyph
  path of more than 500,000 nodes and more than 50,000,000 node moves per layer are refused with a
  message; a brush glyph with more than one closed path is refused (a guess). The arc length is
  sampled at most 4,096 times per host segment, 64 times per curve piece.
- Hints store `type`, `name` and `origin` (the on-curve node that ends the host segment, in file
  order).

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

The M6 planning pass (2026-10-11, `docs/plans/M6.md`) answered each question below, next to it.
Kinds: **handbook** (documented, not yet observed), **format** (the GlyphsSDK file-format
specification v3, Apache-2.0), **oracle** (fontc 1.0.0 and fontmake 3.12.1, which gave identical
results on every probe), **corpus** (to be checked against what corpus files saved by the
reference app store, a measure of `docs/plans/M6.md` § 5) and **ours** (a guess, recorded in the
named row's `guesses`). Nothing here was settled by watching the reference app.

Alignment and anchors

- **Q-1** Numbered ligature anchors (`top_1`, `top_2`, `bottom_1` …): absent from the handbook.
  **Answered (oracle, handbook):** in a composite of category Ligature, propagation numbers the
  anchors of each base (`top` of the second base becomes `top_2`); a component's anchor attribute
  `top_2` attaches it there. Under the handbook's alternative-anchor rule `top_1` is an alternative
  of `top`, so the anchor menu offers it. Set Anchors adds only caret anchors to ligatures (ours)
  (`components.ligatureNumberedAnchors`, `glyph.setAnchors`).
- **Q-2** Exact target search for a mark's `_X`. **Answered (oracle, ours):** the anchors that
  propagation gives the components placed so far; a later component overwrites a name, so the most
  recent component that offers `X` wins (`components.marksStackInOrder`).
- **Q-3** Anchor menu contents and storage. **Answered (format, oracle, ours):** stored as the
  component's `anchor` attribute; the menu lists `X` and `X_suffix`; a vanished name falls back to
  `X` and is kept (`component.setAnchor`, `components.alternativeAnchorSuffix`).
- **Q-4** Scope of the top/bottom swap. **Answered (oracle):** every name containing `top` or
  `bottom` swaps under a negative vertical scale; left/right and entry/exit swap under a negative
  horizontal scale; a 180° rotation does both (`components.verticalFlipSwapsTopBottomAnchors`).
- **Q-5** "As if typed". **Answered (ours, corpus):** each base at the sum of the advances before
  it, no kerning, width the sum, shape order for right-to-left glyphs
  (`components.multipleBasesSetSideBySide`).
- **Q-6** `exit` / `entry` against `#exit` / `#entry`. **Answered (oracle, ours):** only the `#`
  names connect components; both oracles copy `#` anchors into composites and their mark writers
  ignore them, so they never reach `curs` (`components.exitEntryAnchorsConnectBases`).
- **Q-7** Which anchors propagate and how. **Answered (oracle, ours):** FontCraft runs fontc's
  propagation in the editor and the compiler: every name, `_` anchors only from the first
  component, `entry` only from the first, `*origin` shifts the others, the component's full
  transform applied, later components overwrite, the composite's own anchors win. The handbook
  names only `top` and `bottom` (`components.topBottomAnchorsPropagate`).
- **Q-8** Metrics of a flipped or transformed single component. **Answered (ours, corpus):** the
  base's width, the transform kept, a horizontal flip shifted to mirror the sidebearings
  (`components.autoAlignSingleComponent`).
- **Q-9** Vertical placement of a base. **Answered (ours, corpus):** offset (0, 0) for the first
  base, its scale, slant and rotation kept (`components.autoAlignBaseThenMarks`).
- **Q-10** Where alignment is stored. **Answered (format):** per component, as `alignment`: −1 off,
  0 decided by the layer (AA-1, AA-2), 1 forced, 3 horizontal only. Enable stores 0 or 1 (ours,
  `component.enableAutomaticAlignment`). Corpus: 7,097 components with 1, 6,041 with −1, none with 3.
- **Q-11** Positions after a path is added. **Answered (format, handbook):** they stay as stored;
  the file always holds placed positions (`components.pathOnLayerDisablesAutoAlign`).
- **Q-12** Lock on an aligned component. **Answered (ours):** offered only for components that are
  not aligned; the command refuses aligned ones; a lock blocks moves, Info box positions and
  Palette transformations (`component.lock`, `components.lockedCannotMove`).
- **Q-13** Font-wide option against per-component enable. **Answered (ours):** the font option wins,
  forced components included (`components.fontWideDisableAutoAlign`).
- **Q-14** Arrow button position and insertion side. **Answered (handbook, ours):** top right, base
  glyph inserted to the left, as the Measuring page says (`components.infoBoxExtraControls`,
  `components.doubleClickOpensBase`).

Composition and editing

- **Q-15** Create Composite without database data or with a missing part. **Answered (ours):**
  refused with a message naming the missing parts; anchors and metrics keys kept. The component
  list comes from Unicode canonical decomposition and our tables (open decision D6,
  `docs/plans/M6.md` § 7, question 3) (`glyph.createComposite`).
- **Q-16** Component from Selection details. **Answered (video for all masters, ours for the
  rest):** a Glyphs 3 tutorial shows the dialog stating it applies to all master layers; suggested
  name `_part.` plus the glyph name; a taken name is refused; shapes keep their coordinates and the
  component sits at zero; anchors stay (`glyph.componentFromSelection`).
- **Q-17** Recipe grammar. **Answered (ours):** names only, no spaces, any number of `+` parts; a
  part of category Mark attaches to the base before it (`components.recipeBasePlusMarks`).
- **Q-18** One-level decomposition. **Answered (ours):** no anchors added; smart components become
  their interpolated outline; attachments inside the base are applied first (`component.decompose`).
  Decompose Glyph adds the propagated anchors the layer lacks (`glyph.decomposeGlyph`).
- **Q-19** Where "Show all glyphs that use this glyph" shows them. **Answered (ours):** a new Edit
  View tab (`glyph.showGlyphsUsingAsComponent`).
- **Q-20** Tab order with components and anchors. **Answered (ours):** nodes, components in shape
  order, anchors, guides (`components.tabCyclesComponents`).

Smart components

- **Q-21** Interpolation formula. **Answered (oracle):** glyphsLib's variation model, already ported
  in `model/smart.rs` (M2): the master's first pole layer is the default, other pole layers sit at
  ±1 per property, and a missing corner is the sum of the per-property differences
  (`smart.cornerLayersInferred`).
- **Q-22** Default value and removed properties. **Answered (oracle, ours):** a component with no
  stored value draws at its master layer's poles, as glyphsLib does; values of a removed property
  stay stored and are ignored (`smartComponent.setValue`, `smartGlyph.removeProperty`).
- **Q-23** Smart glyphs with several masters. **Answered (format):** properties per glyph
  (`partsSettings`), poles per layer (`partSelection`), each smart layer tied to one master
  (`smart.layersHoldVariants`).
- **Q-24** Smart handle drag. **Answered (ours):** the pointer projected on the line between the
  anchor's positions at the two poles (`smart.handlesFromPropertyAnchors`).
- **Q-25** Export of smart components. **Answered (oracle):** drawn as outlines, as both oracles do;
  a plain component stays when no layer maps to a pole (`smart.axesAreLocalToGlyph`).

Corners, caps, segments, brushes

- **Q-26** Corner fitting geometry. **Answered for corners (oracle):** FontCraft ports fontc's
  `glyphs-reader/src/corner_components.rs` (a port of glyphsLib's filter): scale, then rotate by
  the alignment mode (left: outgoing segment; right: incoming; both: the mean; none: no rotation),
  then trim the host. The entry and exit anchors are ours, since neither oracle uses them yet
  (`cornerComponent.setAlignment`, `corner.entryAnchor`, `corner.exitAnchor`).
- **Q-27** Facing of a corner glyph. **Answered (handbook, oracle):** from the order of the open
  path's nodes and the end node's side of the origin (`corner.pathDirection`).
- **Q-28** Intermediate layers for corner scaling. **Answered (ours):** percentages read from the
  layer name, linear per axis (`corner.scaleInterpolatesWithIntermediateLayers`).
- **Q-29** Cap fitting maths. **Partly answered (written sources, § 8.4 and § 8.5):** the stored
  keys, the Fit bit, the alignment values, what Fit does to the two scales, how centre alignment
  with `node1` and `node2` turns the cap, and that the cap glyph's open path joins the stem at its
  two end nodes. Still provisional, in the rows' `guesses`: the cut points on the stem sides, the
  origin-setup mapping, the reference width of an origin-only cap, the point scale applies about,
  and left and right alignment (`capComponent.add`, `cap.originAnchorSetup`,
  `capComponent.setFit`, `capComponent.setScale`, `capComponent.setAlignment`). Neither oracle
  supports caps and no corpus source uses them; the exports listed in `docs/plans/M13.md` § 8
  settle the rest. If the trial ends without them, M13.40 to M13.42 build on the recorded guesses
  (`docs/plans/M13.md` § 7 question 6).
- **Q-30** Further controls in the cap, segment and brush Info boxes. **Answered for caps (W5
  screenshot, § 8.3):** name and arrow, horizontal and vertical scale, four alignment buttons, fit.
  **Open for segments and brushes:** the handbook lists only the name and the arrow; recorded as a
  guess in `infoBox.segmentComponent` and `infoBox.brush`, settled by a screenshot of each.
- **Q-31** Segment and brush bending maths. **Partly answered (written sources, § 9 and § 10):**
  segment components stretch to the host length without scaling their height, follow the host
  path's direction, and keep each point's perpendicular distance from the host; brushes keep the
  outline beyond `start` and `end`, wrap one host segment each and bend only curve segments. Still
  provisional: the fraction-of-length mapping and the rigid ends (`segmentComponent.add`,
  `brush.add`), settled by exported segment and brush test fonts (M13.41 and `docs/plans/M13.md`
  § 8).
- **Q-32** Paste onto a selection of the wrong kind. **Answered (ours):** refused with a message
  (`corner.copyPasteOntoNodes`).
- **Q-33** Attachments across masters and the order at export. **Answered (oracle, ours):** both
  oracles apply corner components when the source is loaded, before decomposition and overlap
  removal; FontCraft applies them in the model's outline resolution, so the editor, measurement and
  export agree. Matching across masters is checked with interpolation (M9).

Pixel tool

- **Q-34** Grid spacing below 2. **Answered (ours):** a message, nothing changes
  (`pixel.requiresGridSpacingTwoOrMore`).
- **Q-35** Cell origin and snapping. **Answered (ours):** pixels at multiples of the grid spacing,
  found by exact offset (`pixel.setPixels`).
- **Q-36** Export and alignment of pixels. **Answered (ours):** pixel components are written with
  alignment off; the `pixel` glyph exports; components follow the usual export rules
  (`pixel.usesGlyphNamedPixel`).
- **Q-37** Pixel and Pencil on one toolbar button. Open; it blocks nothing (the tool row's `group`
  value is our reading of the Shift+B shortcut).

Export

- **Q-38** How overlap is tested for keeping TrueType components. **Answered (oracle):** neither
  oracle tests overlap; both keep every component that is not mixed with contours, not out of the
  F2Dot14 range and not different between masters, transformed and nested ones included. FontCraft
  does the same since M2 and M3, and keeps that default (the owner's answer to
  `docs/plans/M6.md` § 7, question 5), which departs from what the texts of Keep Overlapping
  Components and Keep Transformed Components imply (`docs/spec/40-compile.md` § 4a).
- **Q-39** Composites referencing a non-exporting glyph. **Answered (oracle, M2):** that component
  is inlined with its transform composed (`export.underscoreGlyphsOmitted`).
