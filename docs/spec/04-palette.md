# Palette

Behaviour notes for `spec/inventory/04-palette.toml`.
Source: Glyphs 4 Handbook, chapter "Palette" (https://handbook.glyphsapp.com/palette/), read
2026-10-07.

## The sidebar

The Palette (`palette`) is the right-hand sidebar of the font window. Toggle it with
`window.palette` (Window > Palette, Cmd+Alt+P) or the sidebar button at the top right.
It has four built-in sections, each with a chevron to collapse it (`palette.toggleSection`):
Dimensions, Fit Curve, Layers, Transformations. Plug-ins can add sections. With
`view.showInfo` on, context-dependent controls appear at its bottom (documented with the
features that own them).

The buttons in the reference are icons without text, so most command labels in the inventory
are ours.

FontCraft's Transformations buttons are icons too, drawn with egui shapes
(`crates/ui-egui/src/icons.rs`), so they show even where the interface font has no arrow
symbols. Each keeps its command label as hover text.

**How FontCraft builds it (M3.15).** The section settings are UI state, changed only by shell
commands: `palette.toggleSection`, `fitCurve.setRange`, `transform.setOrigin`,
`transform.setScaleLock`, `transform.setSettings` (the field values) and the two Layers filters.
Every button passes the values it applies to its engine command, so the engine never reads the
panel. Not built yet: `layers.select` and dragging layer rows, and the Dimensions letter diagram. The Transformations fields start at
scale 110 % (lock closed), rotation 90° and slant 10°; the handbook gives no defaults.

## Dimensions (`palette.dimensions`)

- A notepad of typical measurements (stems, bowls and the like) laid out on a letter diagram.
- Click a number, or an empty field shown as `--`, to type a value (`dimensions.setValue`).
- Stored per master. Read with `dimensions.getValues`.
- The set of fields follows the script of the current glyph; the script can be overridden in
  Edit > Info for Selection.
- Purely informational: the chapter names nothing that consumes the values.

Data: a per-master map from field key to number. The corpus `.glyphs` files keep it in the font's
user data under `GSDimensionPlugin.Dimensions`, one dictionary per master id; some write the
numbers as text. FontCraft reads and writes that key and keeps a field's text form when it has
one. The keys per script are not documented; the corpus has `HH`, `HV`, `OH`, `OV`, `nV`, `nd`,
`oH`, `oV`, `tH` (Latin) and `arAlef`, `arBar` (Arabic).

## Fit Curve (`palette.fitCurve`)

Sets the handle lengths of curve segments as a percentage (the reference point for 100 % is
not defined in the handbook; see Open questions).

| Control | Command | Effect |
|---|---|---|
| left and right fields | `fitCurve.setRange` | lower and upper percentage |
| eight round buttons | `fitCurve.applyStep {step}` (Ctrl+Alt+1 … Ctrl+Alt+8; see Open questions 11) | step 1 = left value, step 8 = right value, steps 2 to 7 evenly between |
| plus / minus | `fitCurve.increase`, `fitCurve.decrease` | lengthen or shorten uniformly |
| scale button | `fitCurve.balanceHandles` | balance the two handles, keeping the overall curvature |
| grey marker | `fitCurve.getHandleLength` | current length of the selected handle |

Rules:

- Both handles of a segment are always treated together, even if one is selected.
- Range of valid values: 1 to 100 %. `fitCurve.setHandleLength` (ours) sets any value directly.
- Around 55 % gives the closest match to a circular or elliptical arc. Longer handles flatten
  the curve near the nodes; shorter ones flatten the middle and tighten the ends.
- A font custom parameter `Fit Curve Panel Settings` holding two comma-separated numbers
  (for example `52, 78`) fixes the two field values for that document.

Step formula implied by "even intervals": `percent(step) = min + (step - 1) * (max - min) / 7`.

Targets (our design): a curve segment is named by one of its handles, by both its end nodes, or
in `segments`. Only cubic segments have handles to set. The commands take the range as `min` and
`max`, since the two fields are panel state. New handles are rounded to the grid.

## Layers (`palette.layers`)

**Model.** A glyph has at least one layer per font master. Three kinds:

| Kind | Count | Row style | Can add / delete / rename here |
|---|---|---|---|
| master layer | exactly one per master | bold master name, top level | no (manage masters in Font Info) |
| backup layer | any number | indented, regular weight, default name = creation date and time | yes |
| special layer (intermediate, alternate, colour, plug-in defined) | any | indented, bold type label | created by their own features |

**List behaviour.**

- Rows are grouped by master. Reordering by drag works only within one master
  (`layers.reorder`).
- Click selects a layer (`layers.select`); Shift+click a range; Cmd+click single rows.
- The eye toggles display (`layers.toggleVisibility`): a visible layer is drawn as a thin blue
  outline while another layer of the glyph is active. The colour is a setting.
- The handle at the bottom edge resizes the section.
- `layers.list` (ours) returns the rows as data.

**Buttons and menus.**

| Control | Command | Effect |
|---|---|---|
| plus | `layers.addBackupLayer` | new backup layer named with date and time |
| minus | `layers.delete` | delete the selected layer (never a master layer) |
| double-click name | `layers.rename` | rename a non-master layer |
| actions menu > Use as Master | `layers.useAsMaster` | restore a backup |
| filter menu > Only Show Layers from Current Master | `layers.onlyShowLayersFromCurrentMaster` | hide other masters' rows |
| filter menu > Hide Backup Layers | `layers.hideBackupLayers` | hide backup rows |

**Use as Master, step by step.** (1) The master layer's current content goes to a new backup
layer. (2) The chosen backup's content becomes the master layer's content. (3) The chosen
backup layer is deleted. Dragging a backup row onto a master row does the same.

**FontCraft's rules** (our decisions where the handbook is silent):

- `layers.list` orders rows by master in master order: the master layer, then that master's other
  layers in file order. Kinds: `master`, `backup` (no special attributes), `intermediate`,
  `alternate`, `color`, `other`.
- A new backup copies the master layer (or the layer named by `from`) and goes after that
  master's other layers. Its default name is the date and time in UTC (`2026-10-10 14:05`). Its
  id has the usual 8-4-4-4-12 hex form and is the same in every targeted glyph. The id depends
  only on the glyphs and their layers, so replaying a journal gives the same id.
- `layers.reorder` counts `toIndex` among the master's other layers; 0 is right under the master
  layer. Master layers do not move.
- `layers.delete` and `layers.rename` refuse master layers. Delete removes each id from every
  targeted glyph that has it; an id no targeted glyph has is an error.
- Use as Master takes backup layers only. The new backup takes the old master content and the
  restored layer's place in the list. A glyph without a master layer for that master gets the
  backup as its master layer and no new backup.
- Visibility is stored with the layer and saved.

## Transformations (`palette.transformations`)

Applies to points, paths, components, anchors, guides and images. Scope follows the selection:
selected segments or paths in Edit View, else whole layers, including all glyphs selected in
Font View. `transform.getSettings` reads the section's field values.

### Origin (`transform.setOrigin`)

Every operation except alignment is relative to an origin chosen in the top row:

| Mode | Origin |
|---|---|
| transformation box | one of nine points of the selection's bounding box (corners, edge centres, centre) |
| reference point | the point clicked with the Scale (S) or Rotate (R) tool; drawn as a red ring with crosshair under those tools, a plain crosshair otherwise |
| metrics point | baseline, half x-height, x-height, half cap height or cap height of the master |
| layer dimensions (CJK glyphs, replaces the metrics point) | half the width, midway between ascender and descender |

### Operations

| Buttons | Commands | Notes |
|---|---|---|
| mirror | `transform.mirrorHorizontal`, `transform.mirrorVertical` | reflects through the origin |
| scale, reverse scale | `transform.scale`, `transform.reverseScale`, `transform.setScaleLock` | separate X and Y percent; lock makes X drive both |
| rotate | `transform.rotateClockwise`, `transform.rotateCounterclockwise` | one angle field |
| slant | `transform.slantLeft`, `transform.slantRight`, `transform.slantDown`, `transform.slantUp` | one angle field |
| align | `transform.alignLeft`, `transform.alignCenterHorizontal`, `transform.alignRight`, `transform.alignTop`, `transform.alignCenterVertical`, `transform.alignBottom` | relative to the selection's bounding box |
| boolean | `transform.booleanUnion`, `transform.booleanSubtract`, `transform.booleanIntersect` | closed paths |

**Opposite buttons.** The two buttons flanking a field are exact inverses. Doing one and then
the other restores the original coordinates without the rounding error the unit grid would
normally add.

**Mirroring.**

- With a metrics point as origin, horizontal mirroring reflects about the horizontal centre of
  the layer.
- Mirroring vertically about the half x-height or half cap-height line preserves overshoots.
- A mirrored corner component swaps handedness (left corner to right corner).
- A component mirrored upside down exchanges the meaning of the base glyph's top and bottom
  anchors, so a top mark can serve as a bottom mark.

**Scaling.** Factor = percent / 100 per axis; 100 % is identity. Reverse scale applies
100 / percent, so it undoes a scale of the same value in either direction (120 % is undone by
83.33… %, not by 80 %).

**Slanting.** Not additive: two slants of an angle differ from one slant of twice the angle.

**Aligning.** Works on points, whole and partial paths, anchors and components, always against
the selection's bounding box, ignoring the origin. Path > Align Selection (Cmd+Shift+A) is a
separate command that does respect the origin setting (see the path editing notes).

**How FontCraft applies them (our design).**

- Every button is one affine map about the origin. A tool or the bounding box passes its own
  point as `origin` (`{mode: "reference", point: [x, y]}`) and runs the same command inside an
  interaction, so a drag is one undo step.
- `origin` takes the four modes of the table above. The shell keeps the chosen mode
  (`transform.setOrigin` is a shell command); the engine command receives it each time.
  Default: the centre of the selection's box.
- The selection's box is measured from the addressed nodes and handles, the curves of whole
  paths, the outlines of components, and the anchors, guides and annotations named. For a whole
  layer it is the box of its shapes, or of its anchors when it has none.
- An addressed on-curve node carries its handles. A whole layer means its shapes and anchors;
  its guides stay where they are.
- A component or image keeps its decomposed placement when the map has no rotation or slant;
  otherwise its placement becomes a matrix.
- Points and anchors are rounded to the grid after the map; guides and annotations are not.
- Align: the objects are each shape, each addressed on-curve node with its handles, each lone
  handle and each anchor; without target lists, every shape of the layer. Fewer than two
  objects change nothing and the result carries a message (question 7).

**Booleans.**

| Operation | With a selection | Without a selection |
|---|---|---|
| union | removes overlaps among the selected paths | all paths of the layer |
| subtraction | cuts the selected paths out of the paths that are not selected | frontmost path is subtracted from the rest |
| intersection | keeps where selected and unselected paths overlap | frontmost path is intersected with the rest |

Paths taking part in a subtraction are merged first so the outcome is stable. Union is the
operation the Remove Overlap export option runs on every glyph.

FontCraft's buttons pass the selected paths as `shapes`: a path counts as selected when it is
selected whole or any of its nodes or handles is. With none selected they pass no `shapes`, so
the command takes every path; in Font View they pass the selected glyphs.

## Open questions

1. Dimensions: the field keys and diagram for each script, and whether any feature (autohinting,
   filters) reads them.
   **Answered for M3 (our decision, 2026-10-09):** read and write the key the corpus `.glyphs` files
   use: `GSDimensionPlugin.Dimensions` in the font's user data, one dictionary per master id.
   Nothing else reads the values (`dimensions.noEffectOnFont`).
2. Fit Curve: default values of the two fields; the increment of plus and minus; the exact rule
   of the balance button; the precise definition of the percentage (relative to the handle
   intersection point, and what happens when the handles are parallel); what the buttons do
   when the field values exceed the range from the custom parameter.
   **Answered for M3 (our decision, 2026-10-09):** the eight steps spread evenly between the lower
   and upper percentage. A handle's percentage is its length over the distance from its node to the
   intersection of the two handles; parallel handles use the chord. The range comes from the `Fit
   Curve Panel Settings` parameter, else 48 and 74 (the handbook's example).
   Plus and minus move to the next or previous step; balance sets both handles of a segment to
   their mean percentage; values beyond the range are clamped to it.
3. Layers: does the plus button copy the active layer or create an empty one; which master a
   new backup attaches to when several layers are selected; is layer visibility saved in the
   file; the full contents of the actions and filter menus; what clicking a row does to the
   glyphs selected in a text tab versus the edited glyph.
   **Answered for M3 (our decision, 2026-10-09):** plus copies the active layer as a backup named by
   date and time; visibility is saved; clicking a row edits that layer.
4. Use as Master on a special layer: allowed, and with what result?
   **Answered for M3 (our decision, 2026-10-09):** allowed for backup layers only.
5. Transformations: the direction and fixed side of slant up/down; whether slant uses the
   angle's tangent as the shear factor; the horizontal position of the metrics-point origin for
   rotate, scale and slant (only mirroring is described); how exactness of opposite buttons is
   implemented (kept sub-grid coordinates or a remembered pre-state).
   **Answered for M3 (our decision, 2026-10-09):** slant is a shear by the tangent of the angle
   about the origin. Opposite buttons are exact for mirroring and 90° rotation; other pairs are
   exact up to grid rounding. The rest of this question stays open.
   **Further choices made when building it (2026-10-10), unconfirmed:** Slant Up moves what lies
   right of the origin up, Slant Down moves it down. The metrics-point origin sits at half the
   advance width for every operation, not only mirroring. Opposite buttons are exact because
   quarter turns use exact sines and cosines and each result is rounded to the grid; no
   unrounded positions are kept. A rotation by another angle is undone only up to grid rounding
   and only about a fixed origin (reference or metrics point): the box of a turned selection is
   a different box, so a box origin moves between the two clicks.
6. Which icon is "centre horizontally" and which "centre vertically": the handbook lists six
   align buttons but the wording of the two centre buttons can be read either way.
7. Align with a single selected object or a single point: no-op, or aligned to something else
   (metrics, layer)? Behaviour in Font View with whole glyphs selected.
   **Answered for M3 (our decision, 2026-10-09):** no change, with a message.
8. Booleans on open paths and on components; the definition of "frontmost" (last path in the
   layer's path order?); which path's start point and direction the result inherits.
   **Answered for M3 (our decision, 2026-10-09):** open paths are left as they are; components are
   not changed; "frontmost" is the last shape in the layer; the result's start node follows the
   Correct Path Direction rule (03a question 16).
9. Edit > Info for Selection is given with Cmd+Shift+I here and Cmd+Alt+I in the Edit View
   chapter; Cmd+Shift+I is also View > Show Info.
10. Fit Curve default range. Question 2 takes 48 % and 74 % (the handbook's example) when the
    `Fit Curve Panel Settings` parameter is absent. The Glyphs 3 palette in the tutorial video
    "Design Amazing Fonts | The Best Beginners tutorial to Glyphs 3 and Type Design"
    (https://www.youtube.com/watch?v=VGe96Qjlvvc) shows 50 % and 70 % at 10:31 and 26:01. These
    may be the presenter's own values or the Glyphs 3 default; the Glyphs 4 default is still
    unknown.
11. Fit Curve shortcut. `fitCurve.applyStep` gives Ctrl+Alt+1 … 8. In the same video (10:31,
    26:06) an on-screen key overlay shows Cmd+Ctrl+Alt+1 … 8 while the handles change. Glyphs 3
    and 4 may differ; check the current handbook and a Glyphs 4 observation before changing the
    row.
