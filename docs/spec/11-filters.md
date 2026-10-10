# Filters: behaviour notes

Source: Glyphs 4 Handbook, chapter "Filters", plus the `Filter` / `PreFilter` entries of the
appendix "Custom Parameters" for the export-string syntax. Inventory: `spec/inventory/11-filters.toml`.

A filter is an operation that takes glyph layers and rewrites them. The reference ships a fixed set
and lists installed filter plug-ins next to them. Every filter can be run in two ways: interactively
(menu, dialog) and at export time through a custom parameter string.

## Running a filter

### Targets

| Where it is run | What is processed |
|---|---|
| Font View | every selected glyph |
| Edit View, Text tool active | the glyphs in the text selection |
| Edit View, any other tool | the current glyph only |

- Only the layer currently shown is changed. Other masters stay as they are; the documented way to
  cover all masters is to switch master and run the filter again.
- Some filters narrow the target further when a point or path selection exists (Round Corners,
  Transformations, Remove Overlap; see each filter).
- Commands: every `filter.*` command takes `glyphs?` and `layers?`. Omitted, the table above
  applies (`filter.target.*` rows). Passing several layer ids is our headless answer to the
  "re-run per master" limitation (`filter.target.headlessExplicitTargets`, inferred).

### Repeat last filter

`filter.repeatLastFilter`, Cmd+R. Applies the last used filter again to the current target. The
source gives the shortcut only.

How FontCraft runs it (our design, unconfirmed): the last filter is the last `filter.*` command of
the registry, or `filter.apply`, in the session journal. It runs again with the same settings, on
the targets the repeat names, without a dialog. The command is disabled until a filter has run.

### Filter menu content

Built-in filters plus all installed filter plug-ins (`filter.menu.contents`,
`filter.menu.pluginFiltersListed`). Five operations that the chapter documents as filters sit in
other menus of the reference: Transform Metrics (Glyph), Transformations, Interpolate with
Background, Add Extremes and Remove Overlap (Path).

### Headless additions (our design)

- `filter.apply {filter, args, include?, exclude?, glyphs?, layers?}` runs any registered filter by
  filter name with export-string arguments.
- `filter.list` returns the filter registry with parameter schemas.

How FontCraft builds them:

| Piece | Where | What it does |
|---|---|---|
| Filter trait and registry | `crates/pathops/src/filters/mod.rs` | Each filter has a command id, an export name, a label, a parameter schema (kind, range, default, required, place in the string) and the operation on one layer's contours |
| Filter commands | `crates/engine/src/cmd/filter.rs` | One `filter.*` command per registry entry, generated from it: a new filter needs no engine change |
| `filter.apply` | same | `filter` is the export name (case ignored) or the command id. `args` as a list holds export-string items; as an object, the command's parameters. `string` holds a whole export string. Without `glyph` or `glyphs` it runs on every glyph, as export does; `include` and `exclude` narrow the list |
| Path menu filters | same | Transformations, AddExtremes and RemoveOverlap are listed under their export names and run their Path menu commands. Transformations reads its string with the parser of `path.transformations` |
| Export | same | `parse_filter_string` and `run_filter_string` serve the `Filter` and `PreFilter` parameters (M7) |

A filter changes paths only. Components and locked paths stay as they are. A surviving node keeps
its name and user data; a path keeps its attributes. A run over many glyphs is one undo step, and
the result of a filter command carries the export string of the settings it ran with (`string`).

## Built-in filters

Defaults are listed only where the source states them. Units are font units unless noted.

### Extrude (`filter.extrude`)

Sweeps the glyph shape along a vector to make a solid shadow body.

| Param | Meaning | Default |
|---|---|---|
| `offset` | length of the shadow | not stated |
| `angle` | direction in degrees, counterclockwise, 0 = to the right | not stated |
| `dontSubtract` | keep the original area inside the result | off |

With `dontSubtract` off the original outline is subtracted from the swept shape, so only the shadow
remains; on, the result is the whole swept shape. Reads and writes the layer's paths.

How FontCraft runs it (our design, documented behaviour not yet observed):

- The swept shape is the original area plus the band each piece of a closed path covers as it
  moves. Curves are split where they run parallel to the sweep, so each band is a simple area.
- The sweep vector is rounded to the grid, and so are new points. An offset of 0, or one that
  rounds to 0, changes nothing. Open paths stay as they are.
- Offset 0 to 1,000,000 units; angle -360 to 360 degrees. The dialog's first values are ours:
  offset 50, angle -45, Don't Subtract off.
- Export string (our choice; none is documented): `Extrude; offset; angle; dontsubtract`.

### Hatch Outline (`filter.hatchOutline`)

Fills the glyph shape with parallel strokes.

| Param | Meaning |
|---|---|
| `originX`, `originY` | point the pattern starts from |
| `stepWidth` | distance between strokes |
| `angle` | stroke direction: 0° horizontal, 90° vertical |
| `offsetPath` + `strokeWidth` | stroke thickness; with the option off the hatch lines are emitted as open paths |

The open-path output exists so that other filters or plug-ins can process the lines further.

How FontCraft runs it (our design, documented behaviour not yet observed):

- The hatched area is the filled area of the closed paths, by the nonzero rule. Open paths,
  single-node paths and components stay as they are.
- One line passes through the origin; the others repeat every `stepWidth` units, measured at
  right angles to the lines. The angle is in degrees, counter-clockwise.
- Offset Path on: each line becomes a rectangle `strokeWidth` units thick, intersected with the
  outline. Stroke ends follow the outline, and strokes as wide as the step merge.
- Offset Path off: each piece of line inside the outline becomes an open path of two line nodes,
  drawn in the direction of the angle. Its ends lie on the outline.
- A layer may get at most 10 000 lines; a finer hatch is an error. New points are rounded to the
  grid.
- First values: origin (0, 0), step width 20, angle 45, Offset Path on, thickness 5.
- Export string: `Offset: thickness` turns Offset Path on; without `Offset`, or with `Offset: 0`,
  the lines stay open.

### Offset Curve (`filter.offsetCurve`)

Changes stem thickness by moving the outline, or turns paths into strokes.

| Param | Meaning |
|---|---|
| `horizontal`, `vertical` | offset per axis; positive expands, negative contracts |
| lock button | when locked, `horizontal` is used for both (`vertical` omitted in the command) |
| `makeStroke` | offset to both sides and join into a stroke outline; the way to outline a closed shape |
| `autoStroke` | like Make Stroke but vertical dimensions of the shape are preserved; implies position 50 % |
| `position` | split of the stroke across the path: 0 % all left, 100 % all right, 50 % centred |
| `keepCompatible` | never add nodes or handles, so masters stay compatible; accuracy may drop |
| `capStyle` | end shape for open paths when stroking: flat, square, round, round inset, aligned to the axes |

"Left" and "right" are relative to path direction. A non-destructive equivalent exists as a stroke
style on paths; this filter bakes the result.

### Roughen (`filter.roughen`)

Two steps: split every segment into straight pieces of roughly `segmentLength`, then move every
node by a random amount no larger than `horizontal` in x and `vertical` in y. We add an optional
`seed` so headless runs are reproducible (not in the reference).

How FontCraft runs it (our design, documented behaviour not yet observed):

- A segment of length `L` is cut into `max(1, round(L / segmentLength))` equal pieces, measured
  along the curve. Curves become polylines: the result has line nodes only. A layer that would
  need more nodes than the node limit is an error and changes nothing.
- Every node, original or new, moves by a vector drawn uniformly from the box of
  ±`horizontal` by ±`vertical`. The draws come from a deterministic generator seeded with `seed`
  (default 0), in node order, so one seed gives one result.
- The export string's fourth value `angle` turns the box counter-clockwise by that many degrees
  (default 0). The handbook names the value without describing it. The seed has no slot in the
  string.
- Moves that leave the coordinate range are an error. New and moved points are rounded to the
  grid.
- The dialog's first values are ours: segment length 50, horizontal 10, vertical 10, angle 0.

### Round Corners (`filter.roundCorners`)

Rounds corners with `radius`.

- Corner nodes selected: exactly those corners are rounded, inward or outward.
- Nothing selected: only outward-pointing (convex) corners are rounded.
- `visualCorrections`: the radius is reduced at acute corners and increased at obtuse ones so the
  rounding looks even.

How FontCraft runs it (our design, documented behaviour not yet observed):

- A rounded corner becomes two smooth nodes joined by a cubic close to a circular arc. Each node
  is `radius / tan(a / 2)` units (straight-line distance) from a corner of inner angle `a`. A curved
  neighbour is split on its own curve, so it keeps its shape.
- That distance is at most half of either neighbouring segment, so the radius shrinks where it
  does not fit. A segment between two roundings that meet disappears.
- Selected corners are selected on-curve nodes and both end nodes of selected segments.
- Outward-pointing means the outline turns toward the filled side. The filled side is read from
  nesting, as Correct Path Direction does, so a glyph drawn in the wrong direction gets the same
  corners. An open path has no filled side: with nothing selected, all its corners are rounded.
- Visual corrections multiply the radius by `tan(a / 2)` (1 at a right angle), limited to a factor
  between 1/4 and 4, so every rounding starts `radius` units from its corner.
- Smooth nodes, nearly straight nodes (less than 1° of turn) and corners next to quadratic
  segments stay. New points are rounded to the grid.
- Export string `RoundCorner; radius; visualcorrection`. The dialog's first values are ours:
  radius 20, visual corrections off.

### Rounded Font (`filter.roundedFont`)

Rounds stem endings and adds a matching overshoot. Stem recognition uses the master's vertical stem
values (Font Info › Masters › Stems). The chapter describes no dialog fields; the export string has
one optional `stem` value.

### Transform Metrics (`glyph.transformMetrics`, Glyph menu)

Batch-sets widths or sidebearings on the selected glyph layers.

- **Width on**: every target gets the entered advance width. Point coordinates do not move unless
  `onBothSides` is set, in which case the change is shared equally by both sides.
- **Width off**: sidebearing mode. The LSB and RSB checkboxes decide which side is written.
- `relative`: entered LSB/RSB values are added to the current ones (negative subtracts); otherwise
  they replace them.

Writes layer width and, where the outline has to shift, point positions.

### Transformations (`path.transformations`, Path menu)

One affine transformation of the selected points, or of whole selected layers when no points are
selected.

- `translate {x, y}`.
- `origin`: reference point for scale and slant. The five choices (cap height, half cap height,
  x-height, half x-height, baseline) are taken from the export-string documentation.
- `scale {x, y}` in percent; locked means x is used for both.
- `slant` in degrees: plain shear. Choosing Cursivy from the Slant label gives an optically
  corrected shear instead; Cursivy needs horizontal and vertical stem values in the master.
- While the dialog is open the canvas shows the result; Cancel restores the outlines.
- A round "…" button at the bottom left opens a menu with Copy Filter Parameter
  (`path.transformations.copyFilterParameter`) and Copy PreFilter Parameter
  (`path.transformations.copyPreFilterParameter`). Copy Filter Parameter puts the dialog's values
  on the clipboard as a `Filter` custom parameter, ready to paste into an export. Slant 10° with
  Origin baseline gives `Transformations;Slant:10;SlantCorrection:0;Origin:4;`: no spaces, a
  semicolon after every item, and fields at their neutral value left out.

How FontCraft runs it (our design, unconfirmed):

- The map scales, then slants, about the origin, then translates.
- The default origin is the baseline. A metrics origin sits at half the advance width.
- `string` takes an export string in place of the other settings, so the dialog, an agent and
  export run the same code. A missing `ScaleX` or `ScaleY` means 100; the writer writes each axis
  that is not 100 and always writes `Origin`. `SlantCorrection:1` is Cursivy. `include` and
  `exclude` limit the glyphs; with neither `glyph` nor `glyphs`, every glyph is changed.
- `LSB`, `RSB` and `Width` belong to Transform Metrics (M5); a string holding them is refused
  until then.
- Cursivy is refused: with a clear error when the master has no horizontal and vertical stems,
  and as not supported yet otherwise (the correction is undocumented).

### Interpolate with Background (`path.interpolateWithBackground`, Path menu)

`amount` in percent: 0 keeps the foreground, 100 gives the background shape, between blends, outside
0–100 extrapolates. Requires compatible foreground and background; otherwise it does not work.
Writes the foreground only.

Compatible means: the same number of shapes, in the same order and of the same kind; paths with
the same closedness, node count and node types; components of the same glyph. Nodes, component
offsets and anchors are rounded to the grid (spacing 1 when the font sets none). Component
placements blend part by part when both are stored decomposed, else as matrices. Anchors blend
when the background has one of the same name; the others stay. The amount is capped at
±100,000 %.

### Add Extremes (`path.addExtremes`, Path menu)

Adds nodes at extremes that lack one. Export string `AddExtremes`, no arguments. The interactive
details (minimum segment length, Force Extremes) are in the Edit View notes; the row here is the
filter-registry entry for the same operation as `path.addExtremes`.

### Remove Overlap (`path.removeOverlap`, Path menu, Cmd+Shift+O)

- Merges the selected paths; all paths when nothing is selected or when several glyphs are targeted.
- Also deletes every open path and stray node from the processed glyphs.
- Assumes path directions are already correct.
- Export string `RemoveOverlap`, no arguments; the export dialog also has a Remove Overlap checkbox
  that does the same for all glyphs.

## Filters at export time

Two custom parameters on an instance carry filter strings:

| Parameter | When it runs | Limits |
|---|---|---|
| `Filter` | after composites are decomposed | none stated |
| `PreFilter` | before decomposition | static fonts only |

### String syntax

`Name; arg; arg; …` with semicolons between items. Arguments are positional or `Key: value`,
depending on the filter. Booleans are `1` / `0`.

| Filter | String |
|---|---|
| Add Extremes | `AddExtremes` |
| Autohint (app function) | `Autohinting` |
| Hatch Outline | `HatchOutlineFilter; OriginX: x; OriginY: y; StepWidth: distance; Angle: angle; Offset: thickness` |
| Offset Curve | `OffsetCurve; x; y; makestroke; position` where position is `0.0`–`1.0` or `auto` |
| Remove Overlap | `RemoveOverlap` |
| Roughen | `Roughenizer; segmentlength; x; y; angle` |
| Round Corners | `RoundCorner; radius; visualcorrection` |
| Rounded Font | `RoundedFont; stem` (stem optional) |
| Transformations | `Transformations; LSB: ±shift; RSB: ±shift; Width: ±shift; ScaleX: percent; ScaleY: percent; Slant: degrees; SlantCorrection: bool; OffsetX: amount; OffsetY: amount; Origin: 0–4` |

- Origin codes: 0 cap height, 1 half cap height, 2 x-height, 3 half x-height, 4 baseline.
- Glyph scoping: append `include: a b c` or `exclude: a,b,c` (space- or comma-separated names).
- Plug-in filters define their own strings and may not support `include` / `exclude`.

Our implementation parses a string into the params of the matching `filter.*` command and
dispatches it through the registry (`filter.customParameter.sameRegistryAsCommands`).

## Open questions

- Menu label of the Cmd+R command, and whether repeating reuses the last values silently.
- Default values of every dialog field (none are given), and whether dialogs remember last values.
  **Answered for M3 (our decision, 2026-10-09):** dialogs remember their last values for the
  session. The first defaults are ours, written in each filter's row when it is built.
- Extrude and Interpolate with Background: is there an export string? None is documented.
- Roughen export string has a fourth `angle` value with no dialog field in the chapter.
- Offset Curve export string has no slots for Keep Compatible or Cap Style; are they settable at export?
- Hatch Outline: can the export string turn Offset Path off (open paths), e.g. `Offset: 0`?
  **Answered for M3 (our decision, 2026-10-10):** yes. A string without `Offset`, or with
  `Offset: 0`, gives open paths.
- Transformations string: meaning of the `±shift` notation (the appendix marks LSB/RSB with an
  asterisk it does not explain), whether an unsigned value is absolute, and whether `SlantCorrection`
  is the same as Cursivy. In the same video a plain slant (not Cursivy) is copied as
  `SlantCorrection:0`, which fits the Cursivy reading but does not prove it.
- The appendix places the Origin picker under "Filter › Transformations › Transform", the chapter
  under Path › Transformations: which menu holds the dialog in the current version? In Glyphs 3 it
  is Path › Transformations (tutorial video "How To Make an Italic Font The Easy Method | Glyphs 3 For Beginners" (https://www.youtube.com/watch?v=4AJrNUWGRds), 1:49).
- Copy PreFilter Parameter: does it copy the same string as a `PreFilter` parameter? The video
  shows the menu item but does not run it. Do other filter dialogs carry the same "…" menu?
- Whether Interpolate with Background leaves an incompatible glyph untouched silently or reports it.
  **Answered for M3 (our decision, 2026-10-09):** an incompatible glyph is left unchanged and listed
  in the result (`skipped`).
- Rounded Font dialog contents and behaviour when the master has no vertical stems.
- Round Corners: does "selected corners" need the node itself selected, or do selected segments count?
  **Answered for M3 (our decision, 2026-10-09):** selected nodes, or the end nodes of selected
  segments.
- Whether any filter applies to more than the visible layer (the source says "usually").
  **Answered for M3 (our decision, 2026-10-09):** a filter changes the visible layer of each
  targeted glyph unless `layers` is given.
- How filters treat components, and whether a filter run on many glyphs is one undo step.
  **Answered for M3 (our decision, 2026-10-09):** filters change paths only, not components; a run
  over many glyphs is one undo step.
- Whether the random displacement in Roughen is uniform and whether it is re-seeded on each run.
  **Answered for M3 (our decision, 2026-10-09):** the displacement is uniform; Roughen takes a
  `seed` (fixed by default), so a run can be repeated exactly.
