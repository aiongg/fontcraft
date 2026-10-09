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

## Dimensions (`palette.dimensions`)

- A notepad of typical measurements (stems, bowls and the like) laid out on a letter diagram.
- Click a number, or an empty field shown as `--`, to type a value (`dimensions.setValue`).
- Stored per master. Read with `dimensions.getValues`.
- The set of fields follows the script of the current glyph; the script can be overridden in
  Edit > Info for Selection.
- Purely informational: the chapter names nothing that consumes the values.

Data: a per-master map from field key to number. The keys per script are not documented.

## Fit Curve (`palette.fitCurve`)

Sets the handle lengths of curve segments as a percentage (the reference point for 100 % is
not defined in the handbook; see Open questions).

| Control | Command | Effect |
|---|---|---|
| left and right fields | `fitCurve.setRange` | lower and upper percentage |
| eight round buttons | `fitCurve.applyStep {step}` (Ctrl+Alt+1 … Ctrl+Alt+8) | step 1 = left value, step 8 = right value, steps 2 to 7 evenly between |
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

**Booleans.**

| Operation | With a selection | Without a selection |
|---|---|---|
| union | removes overlaps among the selected paths | all paths of the layer |
| subtraction | cuts the selected paths out of the paths that are not selected | frontmost path is subtracted from the rest |
| intersection | keeps where selected and unselected paths overlap | frontmost path is intersected with the rest |

Paths taking part in a subtraction are merged first so the outcome is stable. Union is the
operation the Remove Overlap export option runs on every glyph.

## Open questions

1. Dimensions: the field keys and diagram for each script, and whether any feature (autohinting,
   filters) reads them.
   **Answered for M3 (our decision, 2026-10-09):** read and write the key the corpus `.glyphs` files
   use in master user data, if any carries one; otherwise our own key there. Nothing else reads the
   values (`dimensions.noEffectOnFont`).
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
