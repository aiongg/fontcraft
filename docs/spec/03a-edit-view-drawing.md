# Edit View: drawing, editing paths, strokes, compositing, anchors, guides

Behaviour notes for `spec/inventory/03a-edit-view-drawing.toml`.
Source: Glyphs 4 Handbook, chapters "Glyphs", "Create" and "Edit View" up to and including
"Guides" (2026-10-07). Info Box, glyph display, background, text entry, measuring, annotating,
images and previewing are in the sibling notes for the second half of the chapter.

Notation: ids in `code` are rows of the inventory. Shortcuts use Mac notation (`Alt` = Option).
"Inferred" means the command is our own headless form of something the reference only offers as a
pointer gesture or a palette field.

---

## 1. Application basics

- Starting the app with `Alt+Shift` held disables all plug-ins for that session
  (`app.launchWithoutPlugins`). This is the recovery path for plug-in crashes.
- After a crash, the next launch offers to send a crash report (`app.crashReportOnNextLaunch`).

## 2. Creating and saving documents

### Start Window (`window.start`, `window.startWindow`)

Appears at launch when no document is open, and on demand from Window › Start Window.

- Left: recent documents. Double-click opens. Context menu: Show in Finder (`file.showInFinder`).
- Right: version information and website links; a "Show window on startup" checkbox (persisted;
  when off the window is not shown at launch, see `start.shownAtLaunchWithoutDocuments`).
- Plus button: replaces the list with the script chooser. The chooser is also what the window
  shows when there are no recent documents.

### Script chooser (`file.newFromGlyphSets`, `start.scriptChooserRules`)

- List of writing scripts; a search field filters by script name.
- Clicking a script shows its glyph sets on the right. A No/Yes toggle includes the script;
  included scripts get a coloured icon. Checkboxes switch individual glyph sets on or off.
- Create Document makes a new document with one **empty** glyph per glyph of the chosen sets.
- File › New from Glyph Sets… opens the same chooser at any time and restores the previous
  selection of scripts and sets.
- Headless: `file.newFromGlyphSets {scripts: [{script, glyphSets?}]}` skips the chooser.

### New, Save, Save As

- `file.new` (`Cmd+N`): opens a new document holding the basic ASCII glyphs, no questions.
- `file.save` (`Cmd+S`): first save asks for name, folder and one of three source formats; the
  native single-file format is the default.
- `file.saveAs` (`Cmd+Shift+S`): also the way to change an existing document's source format.

## 3. Tabs and Edit View modes

A window is a tab strip: Font View is always tab 1, Edit View tabs follow
(`tabs.windowHoldsFontViewPlusEditTabs`).

| Action | Command | Input |
|---|---|---|
| Open selected glyphs in a new tab | `view.newTab` | `Cmd+T` |
| Open Font View selection for editing | `view.editSelectedGlyphs` | double-click, `Cmd+Down` |
| Go to Font View | `fontView.show` | `Cmd+Alt+1` |
| Go to tab n (2–9) | `view.showTab {index}` | `Cmd+Alt+2` … `Cmd+Alt+9` |
| Next / previous tab | `view.showNextTab`, `view.showPreviousTab` | `Ctrl+Tab`, `Ctrl+Shift+Tab` |
| Close tab | `view.closeTab` | `Cmd+Shift+W`, hover close button |
| Close all other tabs | `view.closeOtherTabs` | `Alt`+click on close button |
| Reopen last closed tab | `view.reopenLastClosedTab` | `Cmd+Alt+Shift+W` (Option alternate) |

Our decisions for the tab commands (M3.2):

- Tab commands name a tab by its position in the strip, counting Font View as 1. View commands
  name a tab by its stable `id`, which `view.newTab` returns.
- A tab holds a list of glyphs, the edited one, its camera and its selection. Until text mode
  (M5) the first glyph of a new tab is the edited one.
- `view.editSelectedGlyphs` shows the tab that holds exactly the given glyphs, else opens one.
- Next and previous wrap around, Font View included. Closing the showing tab shows the tab to its
  left. The last 32 closed tabs can be reopened, each at its old position.
- A glyph that is removed leaves every tab; a tab left with no glyph closes.

Edit View has two modes (`editView.textModeAndEditMode`, `editView.enteringEditMode`):

- **Text mode**: active while the Text tool (`T`) is chosen. Typing inserts glyphs; spacing and
  kerning are adjusted here.
- **Edit mode**: entered by choosing any other tool, by double-clicking a glyph in the tab, or by
  pressing `Escape` in text mode (the glyph after the cursor becomes the edited glyph).

Option alternates: several menu and context-menu items change while `Alt` is held
(`menus.optionRevealsAlternateItems`). In our registry each alternate is its own command.

## 4. Data model facts the chapter fixes

- **Points**: node = on-curve, handle = off-curve, point = either (`points.terminology`).
- **Node type**: smooth or corner, plus an independent super-smooth state (section 8). A smooth
  node forces its two handles onto one line; a corner node does not
  (`nodes.smoothAndCornerTypes`).
- **Paths** store points in direction order from the start node
  (`paths.pointsStoredInDirectionOrder`), can be open or closed, and carry: a locked flag, stroke
  attributes, pen points, and a mask flag.
- **Layer** holds an ordered list of shapes (paths and components), anchors with layer-unique
  names, and local guides. **Master** holds global guides. **Glyph** holds a locked flag and its
  own undo history.

Display conventions:

| Item | Drawn as | Row |
|---|---|---|
| Smooth node | green circle | `nodes.smoothAndCornerTypes` |
| Corner node | blue square | same |
| Handle | small circle with thin grey line to its node | `handles.display` |
| Open path ends | short blue strokes across the path; first node also a blue triangle | `paths.openEndMarkers` |
| Start node of closed path | arrowhead in path direction (green if smooth) | `paths.startNodeDisplay` |
| Node on a vertical metric line | beige diamond behind it | `nodes.metricLineHighlight` |
| Node in an alignment zone | beige circle behind it | same |
| Coincident neighbouring nodes | red circle | `nodes.duplicateHighlight` |
| Point of a locked path under the pointer | red | `paths.lockedPointsHoverRed` |

`view.showNodesInForeground` (`Cmd+Shift+N`, View › Show Nodes › In Foreground) toggles node display.

How FontCraft reads the conventions (M3.2, our decisions where the handbook is silent):

- Each item has its own colour and size in `theme::Tokens`, so a headless test can count them.
- The start node is the first on-curve node of the node list.
- A cubic segment's handles are joined to their own end nodes; a quadratic segment's handles are
  joined in order, as its control polygon.
- "Vertical metric lines" are the master's height metrics (ascender to descender, every metric
  but the italic angle). A node on one gets the diamond; a node inside a zone and on no line
  gets the circle.
- Coincident neighbours are consecutive on-curve nodes, the last and the first included on a
  closed path, with equal coordinates.

### Addressing (our design)

Every editing command names what it acts on; none reads the shell's selection. The full rules
are in decision 0004 › How commands name their targets. In short:

- `glyph` and `layer?` name one layer (default: the first master's layer); `side?` picks the
  foreground (default) or the background. Batch commands take `glyphs`.
- `points: [[shape, node]]` are indices in the layer's shape list and the path's node list, off-curve
  points included. A segment is named by the on-curve node that ends it. `shapes`, `anchors` (by
  name), `guides` (`{scope, index}`) and `annotations` complete the target object.
- `segment: [shape, node]` is a list of one. Each list holds at most 100,000 entries; repeated
  entries count once. Target lists need `glyph`; with `glyphs` they are an error.
- An address that does not resolve is an error and changes nothing. Commands that change a
  layer's structure return the new `selection`. Created or moved points are rounded to the grid.
- `layer.inspect` returns a layer with every address, so an agent can read before it edits.
- A glyph lock (`glyph.toggleLocked`) refuses every command that changes the glyph's drawing
  (its layers, with their outlines, anchors, guides and width, and its metrics keys), with a
  message naming it. Unlocking, renaming, re-encoding and removing the glyph stay allowed.
- A drag or a dialog preview is one interaction: begin, any number of previews, then commit or
  cancel. While it runs, other editing commands are refused until it ends; queries still run. A
  preview that fails leaves the previous preview in place.

## 5. Drawing tools

### Draw (`draw`, `P`)

- Click: add a node; consecutive nodes form one path.
- Drag: add a node and pull out two handles (curve). Handle length sets curvature.
  - `Alt` during the drag: only the outgoing handle follows the pointer; the incoming handle
    stays as it was (gives a corner with handles).
  - `Cmd` during the drag: only the outgoing handle's length changes; both handles keep their
    common angle (asymmetric smooth node).
  - `Space` during the drag: move the node instead of shaping handles.
- Click the path's first node: close the path.
- Click a node of an existing closed path: open the path there (`paths.openAtNodeWithDrawTool`).
- `Shift`+click a segment: insert a node at the nearest extremum or inflection (section 14).

Headless: `path.addPath`, `path.insertNode` (inferred).

### Pencil (`pencil`, `B`)

Freehand drag converted to a Bézier path. Output is deliberately faithful and node-heavy; follow
with Tidy up Paths or manual cleanup. Works with tablets and styluses.

### Primitives (`rectangle`, `ellipse`, `F`)

One toolbar button with two modes. `Shift+F`, click-and-hold on the button, or the context-menu
items `tool.drawRectangle` / `tool.drawCircle` switch mode.

- Drag: draw the shape. `Shift` = square/circle. `Alt` = draw from the centre.
- Click without dragging: a dialog (`dialog.primitiveMeasurements`) takes typed measurements and
  creates the shape at the click.

Headless: `path.addRectangle`, `path.addEllipse` (inferred).

### Shared toolbar buttons (`tools.sharedButtonCycling`)

Tools sharing a button share a shortcut letter; `Shift`+letter switches to the other one, and
click-and-hold on the button lists them. Groups seen in this chapter: Select/Lasso (`V`), the two
Primitives modes (`F`), Erase/Knife (`E`).

## 6. Selecting

Tools: `select` (`V`), `lasso` (`V` / `Shift+V`).

- Marquee drag selects nodes **and** handles inside it; `Alt` restricts it to nodes
  (`selection.marqueeSelectsNodesAndHandles`).
- `Ctrl` during a marquee drag rotates the marquee; release `Ctrl` to continue resizing at that
  angle (`selection.slantedMarquee`).
- Click a point or segment to select it; `Shift` toggles membership
  (`selection.shiftExtendsOrReduces`). Handles are selectable independently of their nodes.
- Double-click near a segment selects the whole path (`selection.doubleClickSelectsPath`).
- Lasso: drag a free-form region.
- A segment counts as selected when both end nodes are selected or one of its handles is
  (`selection.segmentCountsAsSelected`). This definition is used by Shift+Delete and Open Corner.

Commands:

| Command | Shortcut | Notes |
|---|---|---|
| `edit.selectAll` | `Cmd+A` | Two-stage: first all paths; if all paths are already selected, anchors and components too |
| `edit.deselectAll` | `Cmd+Alt+A` | |
| `edit.invertSelection` | `Cmd+Alt+Shift+I` | |
| `selection.selectNext` / `selection.selectPrevious` | `Tab` / `Shift+Tab` | Needs exactly one node or handle selected; also steps through anchors and through guides |
| `selection.set`, `selection.selectInRegion` | – | Inferred headless forms of click / marquee / lasso |

## 7. Moving

Implemented by `selection.move {dx, dy, mode}`; arrow keys call it with ±1, `Shift` ±10,
`Cmd` ±100 (`move.arrowKeyIncrements`).

Nodes:

- Unselected handles attached to a moved node travel with it (`move.nodesCarryHandles`).
- `Alt`: move only the selected points (`mode: pointsOnly`).
- `Ctrl+Alt`, or `Alt` pressed after a drag has started: **nudge**. Selected nodes move and the
  neighbouring unselected handles are rescaled proportionally so adjacent curves adapt
  (`move.nudge`, `mode: nudge`).

Handles (`move.handles`, `move.mirrorHandleAcrossSmoothNode`):

- Drag or arrow keys; all selected handles move together.
- `Alt`: angle preserved, only length changes.
- `Ctrl+Alt` on a handle of a smooth node: the opposite handle takes the same length and the
  matching angle.

Segments (`move.dragSegment`):

- Drag: both end nodes and their handles move.
- `Alt`+drag: handle lengths change along their existing angles.

Dragging is subject to magnetic guides and snapping (section 20); `Ctrl` suspends both.

Construction details where the handbook is silent (our design, `inferred`):

- Nudge scales each unselected handle of a segment with one moved end, per axis, between the
  fixed end and the moved one. Horizontal and vertical handles stay so. When the two ends line
  up on an axis, the handle next to the moved node moves with it and the other one stays.
- Moving a handle of a smooth node that stays turns the opposite handle onto the same line,
  keeping its length. When the other side of the node is straight, the handle slides along that
  line instead.
- Moved points are rounded to the grid; points that do not move keep their coordinates.
- `duplicate` with part of a path selected copies the segments whose two end nodes are selected,
  as open paths. A copied anchor gets the first free name of the form `top.1`.
- `selection.setPosition` and `selection.getInfo` measure a whole path (every point named) and a
  component by their outlines, and anything else by its points.

## 8. Converting nodes and segments

- `path.toggleNodeType` (`Return`, or double-click a node): smooth ↔ corner for the selected
  nodes. Corner → smooth adds missing handles on both sides and may move points so the handles
  line up, which can change the outline.
- `path.toggleNodeTypeKeepingOutline` (`Alt+Return`, `Alt`+double-click): same, but nothing moves.
- `path.convertSegmentToCurve` (`Alt`+click a line segment): adds a handle at each end node.
- Curve → line: delete one or both handles of the segment (`segments.deleteHandleMakesLine`).

### Tidy up Paths (`path.tidyUpPaths` `Cmd+Shift+T`, `path.tidyUpPathsForAllMasters` `Cmd+Alt+Shift+T`)

Scope: selected nodes, else all nodes of the layer; the all-masters form covers every master of
the selected glyphs. It

1. sets each node's smooth/corner type heuristically,
2. removes handles of segments that are in fact straight,
3. removes on-curve nodes lying exactly on the straight line between their neighbours,
4. merges adjacent nodes with identical coordinates.

Caution carried by the reference: removing "redundant" points in one master can break point
compatibility with others (`tidyUp.mayBreakCompatibility`).

### Super-smooth nodes (`path.toggleSuperSmooth`, `Alt+Shift+Return` or `Shift`+double-click)

A per-node state whose effect depends on the neighbouring segments:

- **Curve–curve** (star icon, `nodes.superSmoothBetweenCurves`): the rendered outline has G2
  continuity at the join (same tangent and same curvature radius on both sides). The effective
  on-curve position differs from the drawn node; the drawn path shows grey underneath and a
  small grey dot marks the effective node.
- **Line–curve** (half square / half circle icon, `nodes.superSmoothBetweenLineAndCurve`): a
  curvature-continuous blend from the straight into the curve. Consumes part of the straight
  segment, so that segment must be long enough; grey dots on the line show the consumed length,
  grey dots near the curve show the points that export will create.
- The exported font contains ordinary points (`nodes.superSmoothResolvedAtExport`).

## 9. Scaling and rotating

- **Bounding box** (`view.showBoundingBox`, `Cmd+Alt+Shift+B`): shown for multi-node selections,
  eight knobs. Drag a knob: scale about the opposite knob; `Shift` proportional; `Alt` about the
  box centre (`boundingBox.scaleKnobs`). Drag just outside the box: rotate about the opposite
  knob; `Alt` about the selection centre (`boundingBox.rotateOutside`).
- **Rotate tool** (`rotate`, `R`) and **Scale tool** (`scale`, `S`): click sets the origin, drag
  transforms the selection. `Shift` = 90° steps (rotate) or proportional (scale).
- **Info box** with several nodes selected (`infoBox.selectionGeometryFields`,
  `infoBox.pointCounts`): X, Y, width, height; a reference-point grid choosing the origin; a lock
  linking width and height; counts of selected points and of all points on the layer.
- All numeric fields step with `Up`/`Down`, ×10 with `Shift` (`numberFields.arrowKeyStepping`).
- Further transforms live in the Palette (documented with the Palette).

## 10. Aligning (`path.alignSelection`, `Cmd+Shift+A`)

General case: selected nodes, handles and anchors are brought to one shared X or Y. The axis is
chosen automatically, flattening whichever extent (width or height) of the selection is smaller.
Which coordinate they land on (min / centre / max) follows the transformation origin currently
set in the Palette's transformations section.

Special cases:

| Selection | Result | Row |
|---|---|---|
| 1 anchor + 1 point | anchor moves horizontally over the point, italic-angle corrected | `align.anchorToPoints` |
| 1 anchor + 2 points | anchor centred between the points, italic-angle corrected | same |
| 1 node + 1 component | component moves so its origin point is on the node; node fixed | `align.componentOriginToNode` |
| 1 node only | node moves onto the nearest background-layer node | `align.singleNodeToBackground` |

Component origin point (`align.componentOriginPoint`): the `origin` anchor of the referenced
glyph if present; else baseline × left sidebearing for upright masters; for italic masters,
baseline × the vertical through the point where the slanted LSB crosses half the x-height.

Equivalent manual route: set width or height to 0 in the Info box (`align.viaZeroDimension`).

Headless form (our design): `axis` is `auto`, `vertical` (the points share one x) or
`horizontal` (they share one y). The coordinate is the origin's: for a box origin, the left,
centre or right (bottom, centre or top) of the box around the points; for the other modes, the
origin point's x or y. Whole paths named in `shapes` take part with all their points; a single
node with no background node to go to, or fewer than two points, changes nothing and the result
carries a message. The anchor and component cases are M6.

## 11. Duplicating and deleting

- `Alt`+drag a selection drags a copy (`duplicate.optionDrag`). With a partial path selected the
  selected segments are copied. Works for anchors and guides too. Copy/paste is the alternative.
- `edit.delete` (`Delete`): removes selected nodes while keeping the path closed, refitting the
  neighbouring handles to approximate the old segment. Also removes anchors and free pen points.
- `edit.deleteInAllMasters` (`Alt+Delete`): the same in every master.
- `path.deleteAndBreak` (`Shift+Delete`): removes the node **and** both adjoining segments,
  opening the path. With whole segments selected, only those segments go.
- `path.deleteAndBreakInAllMasters` (`Alt+Shift+Delete`).

Construction details (our design): `edit.delete` removes a path left with fewer than two on-curve
nodes. `path.deleteAndBreak` removes only the segment between two neighbouring selected nodes (or
a segment named in `segments`); a selected node with no selected neighbour goes with both its
segments. What is left becomes open paths, in the place of the original.

## 12. Opening, closing and cutting paths

- Open: Draw tool click on a node (section 5). Then pull the ends apart with Select.
- Close by hand: drag one open end onto another with Select (`paths.closeByDraggingEnds`).
- `path.connectNodes` (context menu, two open ends selected): adds a straight segment between them.
- `path.closeOpenPaths` (context menu): closes each selected open path.
- Ends within half a grid step of each other become one node when connected or closed; otherwise
  a straight segment joins them (our design). Dragging an end onto another runs the move and
  `path.connectNodes` as one step. Two joined paths become one at the lower shape index, keeping
  that path's direction.
- **Knife** (`knife`, `E` / `Shift+E`): draw a cutting line over a path with a drag; the path is
  split and each piece closed along the cut. Across several overlapping paths the cut pieces are
  rewired with one another. Headless: `path.cutWithLine` (inferred).
- **Erase** (`erase`) shares the button; undocumented in this chapter.

## 13. Open corners and reconnecting

- `path.openCorner` (context menu): a selected **corner** node becomes two nodes with a small
  crossing overlap, making the two segments independently editable and easing interpolation.
  No effect on smooth nodes. With a segment selected, both of its end nodes are opened.
- `path.reconnectNodes` (context menu): on the two nodes of an open corner, restores the single
  corner. On any even number of selected nodes, pairs each with its nearest selected neighbour
  and rewires through the pairs.
- Overlap size ≈ half the master's first vertical stem and half its first horizontal stem
  (`openCorner.overlapSize`). Reads master stem values from Font Info.
- An opened corner whose triangle is small relative to the adjacent visible segments is treated
  as invisible in the result, so it may also sit on the outside of a shape; above the threshold
  it shows (`openCorner.invisibleWhenSmall`).
- `path.sharpenCorner` (context menu, below Open Corner): seen in tutorial videos with nodes, a
  segment or a path selected; what it does is unknown (question 31).

## 14. Path direction

Rule (`paths.directionConvention`): outer contours counter-clockwise, enclosed ones clockwise.

- `path.makeNodeFirst` (node context menu): choose the start node of a closed path.
- `path.reverseContours` (Path menu; context menu "Reverse Selected Contours" / "Reverse All
  Contours"): reverse every selected path; with no selection, every path on the layer.
  `Alt` → all masters (`path.reverseContoursForAllMasters`). A closed path keeps its start node.
- `path.correctPathDirection` (`Cmd+Shift+R`): for the current layer or every selected glyph
  layer, derive directions from nesting. Side effects: shape order is rearranged and each start
  node moves to the path's bottom-left node. Pressing it on two overlapping paths is the quick
  way to punch a counter.
- `path.correctPathDirectionForAllMasters` (`Cmd+Alt+Shift+R`): all master layers plus alternate
  and intermediate layers; other non-master layers are left alone.

## 15. Extremes and inflections

Definitions in `extremes.definitions`.

- Draw tool `Shift`+click on a segment: node at the nearest extremum **or** inflection.
- `path.addExtremes`: all paths of the active layer. Skips an extreme that would leave a very
  short segment. A node just off an extreme is moved onto it, shape preserved as well as
  possible, and its handles made exactly horizontal/vertical.
- `path.forceExtremes` (`Alt` alternate): no short-segment exception.
- Export-time alternative: Filter custom parameter `AddExtremes`
  (`extremes.addedAtExportByFilter`).

## 16. Focusing and locking

| Feature | Commands | Effect |
|---|---|---|
| Path focus | `path.focusOnSelectedPath`, `path.clearFocusOnPaths` | View state: only paths with a selected point keep their on-canvas controls. Scripts and filters unaffected. |
| Path lock | `path.lockPath`, `path.unlockPath` | Stored on the path: its points cannot be selected or edited in Edit View. Scripts and filters can still change it. |
| Glyph lock | `glyph.toggleLocked` ("Locked" in the glyph context menu) | Stored on the glyph: **nothing** may modify it, including filters, plug-ins and scripts. Lock icon top-right in Edit View and Font View. |

Engine consequence: every engine command must check the glyph lock and refuse; the path lock is
enforced only by the interactive tools. The engine checks the lock on every edit, whichever
command makes it, so a command that forgets the check still cannot change a locked glyph.

## 17. Strokes

Panel `palette.stroke` (bottom right of the Palette; visible only with Show Info on and a path
selected). Headless: `path.setStroke` (inferred).

- Width and height in font units; no height → width used for both
  (`stroke.nonDestructiveAttribute`). Applies to open and closed paths.
- Fill (closed paths): also fill the interior (`stroke.fillForClosedPaths`).
- Position in percent: 0 centre, −100 fully left, 100 fully right, any value between
  (`stroke.position`).
- Join: four styles. Caps (open paths, start and end separately): five styles, including round
  inset and axis-aligned. Axis-aligned snaps to X or Y, whichever is nearer the cap's
  orientation; click-and-hold forces an axis (`stroke.joinsAndCaps`).
- Display: source path light grey, stroke outline black (`stroke.display`).
- `path.expandOutline` (context menu) bakes the stroke into normal outlines;
  `path.expandOutlineForAllMasters` with `Alt`.

How FontCraft builds a stroke (our construction, `fontcraft_pathops::stroke_outline`):

| Part | Geometry |
|---|---|
| Band | Between two offset curves of the path (kurbo's offset curves), `width` across; `position` p puts the edges at (p − 1) and (p + 1) half widths to the right of the path |
| Elliptical pen | Width is the pen's horizontal size and height its vertical size. The band is built with a round pen in a space squeezed along the longer axis, then stretched back, so round parts become elliptical. The two sizes may differ by a factor of at most 100 |
| Miter join | The outer edges extended until they meet; beyond 4 half widths, a bevel |
| Bevel join | A straight line across the outer side of the corner |
| Round join | A circular arc around the node on the outer side |
| Round full join | Round on the outer side; the inner corner is filled with an arc of half the stroke width |
| Butt cap | Square across the band at the end node |
| Square cap | The band extended by half its width |
| Round cap | A half circle beyond the end node |
| Round inset cap | The band shortened by half its width, then a half circle, so the stroke ends at the end node |
| Axis-aligned cap | Cut along the X or Y axis through the middle of the band at the end node: the axis nearer the butt cut, or the forced one. A path that ends within about 6° of a forced axis gets a butt end |
| Fill | A closed path's own area is added to the ring |

The pieces (one per segment, join and cap) are united with the nonzero rule, so the outline has no
overlaps. The result does not interpolate yet (M9).

Storage: the path attributes `strokeWidth`, `strokeHeight`, `strokePos`, `lineJoin` (0 miter,
1 round, 2 bevel, 3 round full), `lineCapStart` and `lineCapEnd` (0 butt, 1 round, 2 square,
3 round inset, 4 axis-aligned) and `fill`, as `docs/spec/30-source-formats.md` § 2.7 lists them.
`strokePos` follows the format version (version 3: 0 right, 0.5 centred, 1 left; version 4: −1
left, 0 centred, 1 right); a document not read from a `.glyphs` file is saved as version 3 and uses
that form. No stored position is a centred stroke. Cap code 4 and the forced cap axis (the native
key `lineCapAxis`, `x` or `y`) are not confirmed against the reference's files; UFO keeps none of
these attributes.

### Pen points

Local overrides along a stroked path (`penPoints.overrideStrokeLocally`).

- On a node: select the node and fill the pen-point fields in its Info box. Remove by clearing
  those fields.
- Between nodes: `path.addPenPoint` (Select tool, right-click on the stroked path). Remove by
  selecting it and pressing `Delete` (`penPoints.deleting`).
- Fields: width, height, angle (pen rotation), position, progression in, progression out.
  Headless: `path.setPenPoint` (inferred).
- Progression range −1.0 … 2.0; low pulls the change close to the pen point, high spreads it
  along the segment; "in" governs the approach, "out" the departure (`penPoints.progression`).
- Open paths: end nodes keep path settings by default. Closed paths: one pen point overrides the
  whole path; two or more are needed for modulation (`penPoints.openVersusClosedPaths`).

## 18. Compositing and masks

- The layer outline is the composite of its shapes in order (`compositing.shapesCombineInOrder`).
- `path.setMask` (Palette "Mask" checkbox): the path is subtracted from all shapes **below** it.
  Non-destructive; the mask path stays editable.
- Display: removed parts light grey, result black (`mask.display`).
- `filter.shapeOrder` (Filter › Shape Order, `dialog.shapeOrder`): reorder shapes by dragging,
  confirm with OK.

How FontCraft composites a layer (`fontcraft_pathops::composite`, and in the engine
`cmd::stroke::layer_outline`, which resolves components first):

1. Shapes are taken in their stored order; the first is at the bottom.
2. A stroked path counts as its stroke outline.
3. A mask shape draws nothing. It subtracts its outline from everything before it, components
   included (version 4's intersecting mask, value 2, keeps only what lies inside it instead).
   Open paths without a stroke fill nothing and are dropped once a mask applies.
4. A layer with no stroke and no mask is passed on contour for contour.

The canvas draws this composite in black, and export compiles it before overlap removal.
`path.setMask` writes mask value 1, which every format version reads as a subtracting mask.
`filter.shapeOrder` takes the new order as a list of every shape index; hints that refer to a path
by index are not renumbered.

## 19. Anchors

Purpose and naming conventions: `anchors.purpose`.

### Editing

| Action | How | Row |
|---|---|---|
| Add | canvas context menu › Add Anchor; named `new anchor`, name in edit state; `Return` or canvas click confirms | `anchor.add` |
| Rename | double-click, or select + `Return`, or Info box name field | `anchor.rename` |
| Select | click the orange dot; `Shift`+click for several; `Tab` / `Shift+Tab` | `anchors.selection` |
| Select all | `Cmd+A` (second stage) | `edit.selectAll` |
| Move | drag, arrow keys, Info box coordinates | `anchors.moveLikeNodes`, `anchor.setPosition` |
| Duplicate | `Alt`+drag; copy gets a trailing underscore | `anchors.uniqueNamesPerLayer` |
| Delete | `Delete` | `edit.delete`, `anchor.remove` |
| Align to nodes | `Cmd+Shift+A` | section 10 |
| Add defaults | Glyph › Set Anchors `Cmd+U`; all masters `Cmd+Alt+U` | `glyph.setAnchors`, `glyph.setAnchorsForAllMasters` |
| Rebuild defaults | Glyph › Reset Anchors `Cmd+Shift+U`; `Alt` for all masters | `glyph.resetAnchors`, `glyph.resetAnchorsForAllMasters` |

- Names are shown only for selected anchors.
- Marker shape: diamond on a metric line, square in an alignment zone, circle otherwise; with a
  hole when the name starts with `_` (`anchors.shapeShowsMetricPosition`).
- Set Anchors reads the glyph info database; for ligature glyphs named from parts joined with
  underscores it adds caret anchors.

### What anchor names generate

| Names | Result | Row |
|---|---|---|
| `_name` on a mark, `name` on a base | `mark` feature | `anchors.markToBase` |
| same, with `idotless` / `jdotless` present | also `ccmp` | `anchors.ccmpAlongsideMark` |
| `_name` and `name` on one mark | `mkmk` feature | `anchors.markToMark` |
| `exit`, `entry` | cursive attachment; live preview in right-to-left mode | `anchors.cursiveAttachment` |
| `caret_<suffix>` on the baseline of a ligature | GDEF `LigatureCaretByPos` | `anchors.ligatureCarets` |
| `origin` | component origin for alignment | `anchors.originAnchor` |
| `*name[.suffix]` | contextual replacement of `name` | `anchors.contextual` |

### Contextual anchors (`palette.anchorContext`, `anchor.setContext`)

- The Anchor Context field sits at the bottom of the Palette and needs Show Info on.
- Content: feature-code glyph sequence; `*` = the glyph that owns the anchor; classes and tokens
  allowed; optional `&` = position of the mark (required when it is not right after the base);
  optional leading `lookupflag …;` (`anchors.contextSyntax`).
- When the context matches, the starred anchor's position is used instead of the plain one.
- Several contexts per base name: `*bottom.noon`, `*bottom.reh`, each with its own context.

## 20. Guides and snapping

### Automatic aids

- **Magnetic guides** (`guides.magnetic`): red lines that appear during a node drag whenever the
  node lines up with another node or with a vertical metric.
- **Snapping** (`snapping.toNodesAndHandles`): dragged objects snap to nodes and handles of paths
  and of components; component nodes fade in as small markers when the pointer is near.
- `Ctrl` held during the drag turns both off.

### Local and global guides

| | Local | Global |
|---|---|---|
| Owner | glyph layer | master |
| Colour | blue | red |
| Visible on | that layer | all glyphs of the master, or a filtered subset |
| Add | context menu › Add Guide (`guide.add`) | `Alt` + context menu › Add Global Guide (`guide.addGlobal`) |
| Undoable | yes | **no** (section 21) |

Ownership, colour and visibility are in `guides.localVersusGlobal`. The chapter does not say how a
guide is deleted; `guide.remove` is our inferred command for it.

- A new guide is horizontal through the click; with two nodes selected it passes through them.
- Measurement tool: while dragging a measurement line, `Cmd+Ctrl+Alt+G` makes a guide of it
  (`guides.fromMeasurementTool`).
- `view.showGuides` (`Cmd+Shift+L`) toggles display.
- Select by clicking anywhere on the guide; selected = filled knob. Drag the knob to move, drag
  elsewhere on the guide to rotate, double-click the knob to turn 90° (`guides.knobAndSelection`).
- `Tab` / `Shift+Tab` step through guides; arrow keys move (1 / 10 / 100) (`guides.keyboard`).
- `Alt`+drag duplicates (`guides.optionDragDuplicates`).
- `guide.makeGlobal` / `guide.makeLocal` (context menu) convert between the kinds.
- `guide.lock` ("Lock Guides"): not selectable, lock icon instead of knob. `guide.unlock`
  ("Unlock Guide") from the context menu on that icon.

Info box fields for a selected guide (`guides.infoBoxFields`; headless `guide.setProperties`):

- Position and angle. Angle lock keeps the angle fixed against dragging (`guides.angleLock`).
- Sidebearing reference: left (default), right, or both (`guides.sidebearingReference`).
- Measurement toggle: makes it a measurement guide (`guides.measurementFlag`).
- Name: shown at the guide's left end (`guides.names`).

Global guide scope (`guides.globalScopeFilter`): Edit › Info for Selection (`Cmd+Alt+I`) on a
selected global guide defines Smart-Filter-style rules for the glyphs that show it.

### FontCraft's guide commands

| Command | Does |
|---|---|
| `guide.add`, `guide.addGlobal` | Add a guide through `x, y` at `angle`, or (local) through two points |
| `guide.makeGlobal`, `guide.makeLocal` | Move guides between the layer and its master, keeping every property |
| `guide.lock`, `guide.unlock` | Lock or unlock guides |
| `guide.setProperties` | Position, angle, angle lock, sidebearing reference, measurement, name, colour, filter |
| `guide.toggleMeasurement` | Measurement guide on or off |
| `guide.remove` | Remove guides |
| `guide.list` | Every guide the layer shows, with all properties |
| `guide.displayLines` | The grid, metrics, zones and CJK grids Edit View draws for the layer |

Rules (our design where the handbook is silent):

- Angles are degrees counter-clockwise from horizontal, kept from 0 to 360. Positions given as
  numbers are rounded to the font's grid; a guide through two points keeps their positions.
- A locked guide refuses every change but `guide.unlock`, since the reference keeps it from being
  selected. A locked glyph refuses changes to its local guides; global guides belong to the
  master, so a glyph lock does not block them.
- An angle-locked guide refuses a new angle unless the same call sets `angleLocked: false`. A drag
  that rotates goes through `guide.setProperties`, so the lock holds whatever tool calls it.
- The sidebearing reference is the guide's `alignment` key in `.glyphs` data: absent for left,
  `right`, or `center` for both. Commands that change a glyph's width do not yet move guides that
  follow the right side or both sides.
- A global guide's `filter` is kept as predicate text. Showing a global guide only on the glyphs
  its filter selects waits for the Smart Filter rules (M4).

## 21. Undo (`undo.perGlyphHistory`)

Each glyph has its own undo stack, in both Edit View and Font View. Edits to data outside a
glyph are not captured; the reference calls out global guides (owned by the master).

FontCraft keeps one history per document (decision 0004 item 4, D4). Every edit is undoable,
global guides included: a deliberate difference from the reference, noted on
`undo.perGlyphHistory`. There is no per-glyph filter in M3. A drag, a dialog run and a filter run
over many glyphs are one undo step each.

---

## Open questions

Settle by observing the reference app, on our own fonts or in a tutorial video. An answer marked
"for M3" is our decision: the row it changes is marked `inferred` when it is built, until an
observation confirms it. Questions 25, 26 (M6) and 30 (M4) stay open; they block nothing in M3.
Question 31 stays open and blocks only `path.sharpenCorner`.

1. **Option-drag conflict**: `Alt` while moving nodes is documented both as "move only the
   selected points" and as "drag a duplicate". Which wins when (key pressed before vs after
   mouse-down, whole path vs partial selection, mouse vs arrow keys)?
   **Answered for M3 (our decision, 2026-10-09):** `Alt` held at mouse-down on a selection drags a
   copy; `Alt` pressed after the drag started switches to nudge; `Alt` with an arrow key moves only
   the selected points. A tutorial video showing it would confirm or correct this.
2. **Nudge with arrow keys**: is `Ctrl+Alt`+arrow a nudge, and with which increments?
   **Answered for M3 (our decision, 2026-10-09):** `Ctrl+Alt` with an arrow key nudges, with the
   arrow increments (1, 10, 100). A tutorial video showing it would confirm or correct this.
3. **Draw tool on a segment** without Shift: does a plain click insert a node? What does a click
   on an open path's end do (continue the path)? How is an unfinished path ended?
   **Answered for M3 (our decision, 2026-10-09):** a plain click on a segment inserts a node at the
   click; a click on an open end node continues that path; `Escape` or another tool ends the path. A
   tutorial video showing it would confirm or correct this.
   **Video evidence (Glyphs 3):** "Design Amazing Fonts | The Best Beginners tutorial to Glyphs 3
   and Type Design" (https://www.youtube.com/watch?v=VGe96Qjlvvc) shows at 22:56–23:04 that a
   click on the open end node of another path continues that path. At 22:57 a click on a segment
   of another path starts the new line at that point; whether it also inserts a node into the
   clicked path is not visible. That part stays open, and so does ending a path.
4. **Draw tool `Shift`**: is there angle constraint while placing nodes or dragging handles?
   **Answered for M3 (our decision, 2026-10-09):** `Shift` constrains the new node or handle to 45°
   steps from the previous node.
5. **Primitives dialog**: field names, defaults, remembered values, anchor point of the shape,
   and node structure of the ellipse (four nodes at extremes assumed).
   **Answered for M3 (our decision, 2026-10-09):** width and height, created with the click as the
   lower-left corner; the ellipse has four nodes at its extremes; the values are remembered for the
   session. A tutorial video showing it would confirm or correct this.
6. **Erase tool**: purpose and gestures. Also confirm the toolbar grouping of Rotate and Scale.
   **Answered for M3 (our decision, 2026-10-09):** Erase is deferred to the interaction-fidelity
   pass until a source shows what it does. Rotate and Scale are separate tools with `R` and `S`, as
   their rows say.
7. **Marquee and segments**: does a marquee select handles whose nodes are outside it; what
   exactly does clicking a segment select (both nodes, the handles)?
   **Answered for M3 (our decision, 2026-10-09):** a marquee selects the handles inside it whatever
   their node; clicking a segment selects its two end nodes.
8. **Tab navigation**: does `Tab` wrap at path end, cross to the next path, and how does it order
   anchors and guides?
   **Answered for M3 (our decision, 2026-10-09):** nodes in path order, then the next path, then
   anchors, then guides, then back to the start.
9. **Corner → smooth conversion**: the exact rule for placing the new handles and which points
   move; and the heuristics Tidy up Paths uses to classify nodes (angle tolerance).
   **Answered for M3 (our decision, 2026-10-09):** missing handles are added at one third of the
   segment; two handles that are not collinear are turned to the bisector of their directions,
   keeping their lengths. Tidy up Paths classifies a node as smooth within 1° (the
   `kinkedSmoothNode` threshold). A tutorial video showing it would confirm or correct this.
10. **Delete with reconstruction**: fitting method and tolerance; behaviour when the selection
    includes handles only, or all nodes of a path.
    **Answered for M3 (our decision, 2026-10-09):** the new segment keeps its two end nodes and the
    directions the old curve leaves and enters them in, so the neighbouring segments still meet it
    as before. The two handle lengths are fitted by least squares over samples of the removed
    segments (P. J. Schneider's method, Graphics Gems, 1990); when the removed segments are one
    cubic split in two, the unsplit cubic is used. A node that was inserted into a curve and is
    then deleted gives the curve back within 0.5 unit; one cubic cannot always follow several
    segments that closely. With only handles selected, the segment becomes a line. With every node
    of a path selected, the path is removed.
11. **"All masters" variants**: how nodes are matched across masters (by index?) and what happens
    with incompatible masters; whether alternate and intermediate layers are included for
    commands other than Correct Path Direction.
    **Answered for M3 (our decision, 2026-10-09):** nodes are matched by address. A layer whose
    structure differs is skipped and listed in the result (`skipped: [{layer, reason}]`). Alternate
    and intermediate layers are included only by Correct Path Direction, as the handbook says.
12. **Super-smooth**: the construction used for star and raindrop nodes, the minimum straight
    length, and what points export emits.
    **Deferred** to the interaction-fidelity pass: without the construction, a guess would change
    exported outlines. The super-smooth rows carry the reason in `notes`.
13. **Align Selection**: the tie-break when width equals height; how the transformation origin
    maps to the target coordinate; which point moves when handles of smooth nodes are involved.
    **Answered for M3 (our decision, 2026-10-09):** when width equals height, the selection aligns
    on one X. The transformation origin's column or row picks min, centre or max. Handles of smooth
    nodes move with their nodes.
14. **Open Corner**: the exact overlap geometry, the invisibility threshold, behaviour when stem
    values are missing.
    **Answered for M3 (our decision, 2026-10-09):** the overlap is half the master's first vertical
    stem by half its first horizontal stem, or 20 units when the master has no stems. The
    invisibility threshold stays open; overlap removal at export takes the small triangle away.
15. **Reconnect Nodes** on four or more nodes: pairing rule when distances tie.
    **Answered for M3 (our decision, 2026-10-09):** ties go to the lower address first.
16. **Correct Path Direction**: the resulting shape order (sort key) and tie-breaks for the
    bottom-left start node.
    **Answered for M3 (our decision, 2026-10-09):** outer contours counter-clockwise and counters
    clockwise, decided by nesting; shapes sorted by bounding box (left, then bottom); the start node
    is the lowest, then leftmost, on-curve node (the `startPointPosition` rule).
17. **Add Extremes**: the "very short segment" threshold and the "slightly off" tolerance; whether
    a selection limits the command; whether inflections can be added in bulk.
    **Answered for M3 (our decision, 2026-10-09):** an extreme is skipped when it would leave a
    segment shorter than 3 units (the `shortSegments` threshold). A node within 2 units and 2° of an
    extreme keeps its place and its handles are made exactly horizontal or vertical, which puts the
    extreme on the node (the `almostOrthogonalHandle` thresholds). The command acts on the
    targets given, else the whole layer. Inflections in bulk: `path.addInflections`, our command,
    also the audit's fix.
18. **Reverse Contours**: shortcut (none stated in this chapter) and the label of the Option
    alternate; same for Reset Anchors and Expand Outline alternates.
    **Answered for M3 (our decision, 2026-10-09):** the alternates are labelled "… for All Masters",
    with the shortcuts their rows give.
19. **Path focus**: is it persistent view state per tab or per layer; does it survive switching
    glyphs?
    **Answered for M3 (our decision, 2026-10-09):** path focus belongs to the tab and is cleared
    when the tab's edited glyph changes.
20. **Locks**: are path locks and glyph locks stored in the file; does glyph lock also block
    metrics, anchors and kerning edits?
    **Answered for M3 (our decision, 2026-10-09):** locks are stored in the file (the model and both
    source formats hold them). A glyph lock refuses every command that changes the glyph's own data:
    layers, outlines, anchors, guides, width and metrics keys. Kerning is font data and is not
    blocked.
21. **Stroke**: names and geometry of the four joins and five caps; whether the Fill checkbox
    and Mask checkbox sit in the Stroke section; whether left/right follows path direction; how
    strokes export (auto-expanded?) and interpolate.
    **Answered for M3 (our decision, 2026-10-09):** joins: miter, round, bevel, round full. Caps:
    butt, round, square, round inset, axis-aligned. A positive position is to the right of the path
    direction. Every join and cap is our construction on kurbo's offset curves (§ 17). Export
    expands strokes; their interpolation is M9. A tutorial video showing it would confirm or
    correct this.
22. **Pen points**: units and zero direction of the angle; interpolation law behind the
    progression values; default progression.
    **Deferred** to the interaction-fidelity pass: the pen-point rows carry the reason in `notes`.
23. **Mask**: interaction with components and with stroked paths; export behaviour.
    **Answered for M3 (our decision, 2026-10-09):** a mask subtracts from every shape below it,
    components included (decomposed for the operation); stroked shapes are expanded first; export
    applies masks while it prepares outlines.
24. **Shape Order dialog**: what each row shows, and whether it applies to all masters.
    **Answered for M3 (our decision, 2026-10-09):** one row per shape (kind, component name or path
    index, a thumbnail); the dialog acts on the current layer.
25. **Anchors**: placement rules of Set Anchors (database positions relative to metrics and
    outline), whether Set Anchors keeps existing anchors untouched, and renaming to a name that
    already exists.
26. **Anchor context**: storage key in the file format and validation of the code.
27. **Guides**: how a guide is deleted; rotation pivot when dragging; whether angle-locked guides
    still move; exact semantics of the "both sidebearings" reference; snapping of nodes to guides.
    **Answered for M3 (our decision, 2026-10-09):** `Delete` removes the selected guide
    (`guide.remove`). Dragging off the knob rotates the guide about the knob. Angle-locked guides
    move but do not rotate. Sidebearing reference: left keeps x, right moves with the width, both
    keeps the guide's fraction of the width. Guides are snap targets. A tutorial video showing it
    would confirm or correct this.
28. **Magnetic guides / snapping**: snap distance in screen pixels, priority among targets, and
    whether metrics and guides are snap targets.
    **Answered for M3 (our decision, 2026-10-09):** snap distance 4 screen points (a `theme::Tokens`
    value). Priority: nodes, handles, guides, metric lines. `Ctrl` suspends snapping and magnetic
    guides. A tutorial video showing it would confirm or correct this.
29. **Undo**: what else besides global guides escapes per-glyph undo (font info, kerning,
    classes), and whether there is any document-level undo.
    **Answered for M3 (our decision, 2026-10-09):** one history per document, global guides included
    (decision 0004 item 4, D4). No per-glyph filter in M3.
30. **Start Window**: the list of scripts and glyph sets comes from the app's glyph data; we need
    our own data source for it.
31. **Sharpen Corner**: what does it do, and on what selection? Tutorial videos show the item in
    the canvas context menu with corner nodes, a segment or a path selected, below Open Corner, but
    never run it. The handbook does not describe it. The name suggests it turns an opened or
    rounded corner back into one sharp corner.
