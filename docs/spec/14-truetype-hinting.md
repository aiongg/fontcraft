# TrueType hinting

Behaviour notes for `spec/inventory/14-truetype-hinting.toml`.
Sources: Glyphs 4 Handbook, chapter "TrueType Hinting", and its custom-parameter appendix for the
TTFAutohint options; defaults the handbook leaves out come from the help text of the ttfautohint
program, run as a black box. Row ids are given in `code`.

## 1. Model

- Sources are cubic. TrueType export converts to quadratic outlines and carries manual instructions
  across, so instructions reference source nodes (`hinting.tt.cubicSourceQuadraticOutput`).
- Glyph-level instructions live on the main master only: the first master, or the one named by the
  font parameter "Get Hints From Master" (`hinting.tt.mainMasterOnly`,
  `hinting.tt.param.getHintsFromMaster`).
- Every master has TT zones and TT stems. They are what allows the main master's instructions to be
  applied in the other masters and in instances (`hinting.tt.replicateToOtherMasters`).

## 2. Autohinting at export

Rows: `hinting.tt.export.autohintIgnoresManual`, `hinting.tt.export.manualWhenAutohintOff`,
`hinting.tt.param.ttfAutohintOptions`, `hinting.tt.param.ttfAutohintControlInstructions`,
`autohint.tt.run`, `autohint.tt.option.*`, `autohint.tt.controlInstructions.syntax`.

| Export "Autohint" | Result |
|---|---|
| on | Manual instructions are ignored; the whole font is autohinted (TTF Autohint in the reference) |
| off | Manual instructions are compiled into the font; uninstructed glyphs stay unhinted |

Mixing is not possible in one export.

Configuration is per export instance through two custom parameters: "TTFAutohint options" and
"TTFAutohint control instructions". The custom-parameter appendix lists the options; the defaults
are from ttfautohint 1.8's help text (M10 planning pass):

| Option | Default | Meaning |
|---|---|---|
| hinting-range-min / -max | 8 / 50 | PPM range that gets its own hint sets |
| hinting-limit | 200 | PPM above which hinting switches off; 0 = never |
| increase-x-height | 14 | Round x-height up more readily up to this PPM; 0 = off |
| x-height-snapping-exceptions | none | PPM list exempt from x-height snapping |
| fallback-stem-width | 50 units at 2,048 units per em, scaled | Stem width for scripts without a derived one |
| stem width mode (stem-width-mode) | qsq | natural / quantized / strong for grayscale, GDI ClearType, DirectWrite |
| default-script, fallback-script, fallback-scaling | latn, none, off | Script assignment for unclassified glyphs |
| windows-compatibility | off | Extra zones at usWinAscent / usWinDescent |
| hint-composites, adjust-subglyphs | off | Composite handling |
| symbol, ignore-restrictions | off | Fonts without standard characters; restricted fonts |
| dehint | off | Strip hinting |
| no-info / detailed-info | info on | Version-string annotation |
| ttfa-table | off | TTFA table recording every parameter |

Control instructions address points by index; Show Point Indexes (`hint.tt.showPointIndexes`,
headless `hint.tt.getPointIndexes`) displays them on on-curve nodes.

## 3. TT zones

Rows: `hinting.tt.zones.*`, `hinting.tt.param.ttfZones`, panel `dialog.ttfZones`, panel
`dialog.ttDeltas`, commands `hint.tt.getPsZones`, `hint.tt.addZone`, `hint.tt.removeZone`,
`hint.tt.setZone`.

- Default: the master's alignment zones. A "TTFZones" parameter on the master replaces them for
  TrueType only. Any number of zones.
- Editor: zone list with +, −, and an actions menu containing "Get PS zones".

| Property | Scope | Behaviour |
|---|---|---|
| Name | all masters | Unique free text. `xHeight` is special: more likely to round up at small sizes; gets the rounding threshold twice |
| Position | all masters | Flat edge, offset from baseline, negative below |
| Size | all masters | Positive = top zone, negative = bottom zone; must cover the overshoots to flatten |
| Align | all masters | Link to another zone; the distance between both positions is rounded and applied, so the zones move together |
| Deltas | main master only | Table masters × PPM; each cell: round up / round down / none |
| Filter | main master only | Smart Filter style rules limiting the zone to some glyphs |

Alignment rule: distance < ½ px → same row; ½ px → one pixel apart; and so on by rounding.

Indicators: the delta icon is filled when any cell is set; the globe icon is filled when a filter
exists.

## 4. TT stems

Rows: `hinting.tt.stems.*`, `hinting.tt.param.ttfStems`, panel `dialog.ttfStems`, commands
`hint.tt.getPsStems`, `hint.tt.addStemDefinition`, `hint.tt.removeStemDefinition`,
`hint.tt.setStemDefinition`.

- Default: the master's standard stems. A "TTFStems" parameter on the master replaces them.
- Editor: stem list with +, −, and an actions menu containing "Get PS stems".

| Property | Scope | Behaviour |
|---|---|---|
| Orientation | all masters | X-direction = thickness of an upright stroke; Y-direction = thickness of a horizontal stroke; toggled with the double-arrow icon |
| Name | all masters | Free text label |
| Size | all masters | Average thickness; similar stems render with the same pixel count |
| Deltas | main master only | Like zone deltas; effective only in ClearType-style (GDI) rendering |
| Filter | main master only | Like the zone filter |

## 5. Other font-level settings

| Parameter | Where | Default | Effect | Row |
|---|---|---|---|---|
| TTFBlueFuzz | font, master (interpolated) or instance (overrides) | 1 | Extends every TT zone by this many units above and below | `hinting.tt.param.ttfBlueFuzz` |
| TTFOvershootSuppressionBelowPPM | font | not stated | Below this PPM overshoots collapse onto the zone position | `hinting.tt.param.overshootSuppressionBelowPpm` |
| TTZoneRoundingThreshold | font or instance | 0.09375 | Added to positive zone positions (in pixels) before rounding; twice for `xHeight` | `hinting.tt.param.zoneRoundingThreshold` |
| TTMinimumDistance | instance | 0.25 px | Minimum rendered length of a stem that has a Stem instruction | `hinting.tt.param.minimumDistance` |
| CVT Table, prep Table Assembly, fpgm Table Assembly | masters | none | Imported hinting of an opened TTF, written back to `cvt`, `prep`, `fpgm` | `hinting.tt.param.controlValueProgram` |
| gasp Table | font or instance | 8 and 20 | Two PPM thresholds, three rendering ranges | `hinting.tt.param.gaspTable` |

Rounding-threshold example to use as a test: threshold 0.2; small-caps zone at 6.45 px → 6.65 → 7;
`xHeight` zone at 5.25 px → 5.65 → 6. Without a threshold the same zones give 6 and 5; the default
0.09375 gives 7 (6.54) and 5 (5.44).

### gasp ranges

Rows: `hinting.tt.gasp.ranges`, `hinting.tt.gasp.flagMapping` (inferred).

| Range | Name | Behaviour |
|---|---|---|
| up to threshold 1 (default 8) | No Hinting & Symmetric | No grid-fitting; antialiasing |
| threshold 1 to threshold 2 (default 20) | Hinting & Asymmetric | Grid-fitting, no grayscale; ClearType: vertical grid-fit, horizontal subpixels |
| above threshold 2 | Hinting & Symmetric | Grid-fitting and grayscale; ClearType: symmetric smoothing |

Renderers may ignore `gasp`.

## 6. TrueType Instructor tool

Rows: tool `trueTypeInstructor`; panel `infoBox.ttInstructor`; behaviours `hinting.tt.tool.*`,
`hinting.tt.hintOrder`, `hinting.tt.touchedPoints`, `hinting.tt.iup`.

### What is drawn

| Outline | Meaning | Notes |
|---|---|---|
| Black | Current master | Only this one is selectable. Nodes are gray discs; extra (intersection) nodes always shown |
| Blue | Previewed instance, unhinted | Hidden when the instance equals the current master |
| Red | Previewed instance after instructions and grid-fitting, for the current rendering intent and PPM | Must match the Windows rasterizer. Hidden when the glyph has no instructions |
| Pixel backdrop | Approximate rendering at the current PPM | Not exact: later filtering is unpredictable |

"Show Pixel Preview" in the context menu toggles blue and red (`hint.tt.showPixelPreview`). The
previewed instance comes from the instance menu beside the eye icon at the bottom left of Edit View.
Rendering intents: Grayscale, ClearType, DirectWrite (`hint.tt.setRenderingIntent`; control not
documented).

### Keys while the tool is active

| Key | Action | Row |
|---|---|---|
| A | Snap | `hint.tt.snapPoint` |
| S | Stem | `hint.tt.addStemHint` |
| F | Shift | `hint.tt.shiftPoints` |
| G | Interpolate | `hint.tt.interpolate` |
| E | Delta | `hint.tt.addDelta` |
| X | Toggle hinting direction | `hint.tt.toggleDirection` |
| . or + | PPM up | `hint.tt.increasePixelSize` |
| , or − | PPM down | `hint.tt.decreasePixelSize` |
| Up / Down in the PPM field | PPM up / down | same |
| Cmd+Ctrl+Alt+Shift+I | Autohint glyph(s) | `hint.tt.autohintGlyph` |

Plain letters do not switch tools while this tool is active
(`hinting.tt.tool.plainKeysAreInstructions`).

### Pixel size and direction

- PPM field with steppers in the Info box. One pixel = UPM ÷ PPM units.
- Direction: vertical (points move in Y, for horizontal stems) or horizontal (points move in X, for
  vertical stems). Instructions of the other direction are dimmed and locked.

### Execution model

1. Instructions run in creation order, per direction.
2. A point moved by an instruction becomes touched.
3. After the last instruction, every untouched point is interpolated from its nearest touched
   neighbours in point-index order (IUP).

Commands validate their selection against the touched state obtained by replaying the list.

## 7. Instructions

General rows: `hinting.tt.instructions.addFromSelection`, `hinting.tt.instructions.badgeColours`,
`hint.tt.autohintGlyph`, `hint.tt.removeHints`, `hint.tt.deleteInstructions`,
`hint.tt.listInstructions`, `hint.tt.setRounding`, `hinting.tt.rounding.modes`.

| Type | Key | Context-menu label | Selection required | Badge | Default rounding | Info box pop-up |
|---|---|---|---|---|---|---|
| Snap | A | Snap Point | one or more untouched points | purple | Round | Auto / No Zone / zone names |
| Stem | S | not stated | point pairs (two untouched, or one touched + one untouched); even count | sky blue (purple in a triple) | No rounding | Auto / No Stem / stem names |
| Shift | F | Shift Points | one touched origin + untouched targets | dark blue | No rounding | none |
| Interpolate | G | Interpolate | two touched points + one untouched | green | No rounding | none |
| Delta | E | Delta | any points; vertical direction only | not stated | n/a | Delta table button |

Rounding modes (all but Delta): Round, Round up, Round down, No rounding, Round only in GDI
ClearType. "Up" is above in vertical mode and right in horizontal mode. The GDI-only mode matters
only for vertical hinting.

### Snap

Rows: `hint.tt.snapPoint`, `hint.tt.setSnapZone`, `hinting.tt.snap.*`.

- Auto (recommended): in vertical mode, a point inside a zone goes to the pixel edge the zone
  rounds to; otherwise nearest pixel edge.
- No Zone: nearest pixel edge only.
- Named zone: keeps a constant distance to that zone and never crosses it. Not a "snap to zone".
- No rounding: point stays put but counts as touched; used to shield it from IUP.

### Stem

Rows: `hint.tt.addStemHint`, `hint.tt.reverse`, `hint.tt.setStemReference`,
`hint.tt.makeTripleHint`, `hinting.tt.stem.*`.

- Origin → target (arrowhead). The target follows the origin at a stem-controlled distance.
- "Reverse" in the instruction's context menu swaps origin and target.
- An origin inside a zone is snapped to the zone by the Stem instruction itself.
- Auto: nearest TT stem by size. No Stem: keep the original distance only, no grid fit. Named stem:
  force that stem.
- Rounding acts on the stem size: Round up = target on the pixel edge farther from the origin,
  Round down = nearer. Default No rounding still uses the stem width as rounded to the intent's
  oversampling grid; expect a gray edge, less distortion.
- GDI-only rounding suits stems away from zones (middle bars), combined with Interpolate.
- Triple hint: Shift+click three Stem instructions on three horizontal stems, then "Make Triple
  Hint". They turn purple and keep at least one pixel between stems.
- Minimum rendered stem length: TTMinimumDistance.

### Shift

Rows: `hint.tt.shiftPoints`, `hinting.tt.shift.*`.

Copies the origin's movement to each target; targets become touched. Use to keep separate parts in
step when another instruction interrupts the contour (legs of n, m).

### Interpolate

Rows: `hint.tt.interpolate`, `hinting.tt.interpolate.*`.

Keeps the target's relative position between two touched points. Use Round when a Stem instruction
starts from the target.

### Delta

Rows: `hint.tt.addDelta`, `hint.tt.setDeltas`, `hinting.tt.delta.restrictions`.

One pixel up or down for one point at one PPM in one static instance. Configured with the same
table as zone deltas. Vertical only. Ignored in variable fonts. Add last.

### Autohint and Remove Hints

- "Autohint" (context menu, Cmd+Ctrl+Alt+Shift+I, rebindable): adds Snap and Stem instructions
  only. Works on several selected glyphs via the shortcut. Failure message: "There was a problem
  compiling TrueType instructions".
- "Remove Hints": clears all glyph-level instructions of the glyph.

## 8. Previews

Rows: panel `preview.ttHintingWaterfall`, `hint.tt.setPreviewScale`, `hint.tt.showPreviewAddress`,
panel `dialog.ttPreviewAddress`, `hint.tt.renderPreview` (inferred).

- With the tool active the Preview Area sits at the right of Edit View and shows the tab's glyphs in
  the current instance at every size from 8 to 80 PPM. Drag to scroll. Scales 1×, 2×, 4× from the
  context menu. Also available in the Preview Panel window.
- "Show Preview Address…" starts a local web server with an equivalent page and shows its address
  with a Copy button, for viewing on Windows.

## 9. Export rules

Rows: `hinting.tt.export.*`, `hinting.tt.variable.*`, `hinting.tt.import.glyphPrograms`,
`hint.tt.removeImportedHinting`.

- Static export with Remove Overlaps: an instruction on a point that disappears moves to the closest
  intersection point that the removal creates.
- Variable export: overlaps stay; instructions on extra (intersection) nodes are dropped; Delta
  instructions are dropped.
- Instance values: zones, stems and master-level TTFBlueFuzz interpolate; instance parameters
  override font parameters.
- Imported fonts: `cvt`, `prep`, `fpgm` are kept as master parameters and written back.

## Open questions

Answers from the M10 planning pass (`docs/plans/M10.md` § 3); "guess" means a choice of ours
recorded in the row's `guesses`, to be checked in the reference app (§ 8 of the plan).

1. Tool shortcut and toolbar position of the TrueType Instructor; how node selection works in it
   (rectangle, Shift, Select All).
   **Answer:** the custom-parameter descriptions give the shortcut as I (handbook). Toolbar
   position after the Measurement tool, selection as with the Select tool (guess).
2. Where the rendering intent (Grayscale / ClearType / DirectWrite) is chosen.
   **Answer (guess):** a pop-up in the tool's Info box beside the pixel size.
3. Context-menu label of the Stem instruction; whether Delete removes selected instructions; how a
   triple hint is dissolved; whether instructions can be reordered.
   **Answer (guess):** "Stem"; Delete removes them; deleting or reversing a member dissolves the
   group; no reordering.
4. What "drag to reposition" changes: the attached node(s) or only the badge position?
   **Answer (guess):** only where the badge is drawn, as view state.
5. Colour of Delta badges; exact look of each rounding icon.
   **Answer (guess):** orange; the icons are FontCraft's own, from the theme.
6. Delta table: PPM row range; whether the columns for Delta instructions are masters or instances;
   whether zone/stem deltas survive variable export.
   **Answer (guess):** rows 8 to 80 PPM; masters for zones and stems, static instances for Delta
   instructions; in variable fonts only the default master's zone and stem deltas apply.
7. "Get PS zones" / "Get PS stems": replace or append; names given to imported entries.
   **Answer (guess):** replace; zones named after their metric (`xHeight` for the x-height),
   stems keep their names.
8. How zones and stems are matched across masters (name or index) and what happens when a master
   lacks an entry; whether stem names must be unique.
   **Answer (guess):** by name, as stems interpolate by name; a missing entry takes the main
   master's value; names unique, duplicates refused.
9. What happens to instructions referring to a removed or renamed zone or stem.
   **Answer (guess):** a removal falls back to Auto; a rename follows.
10. Exact option list, defaults and storage format of "TTFAutohint options", and the accepted
    control-instruction syntax.
    **Answer (handbook, black-box run):** the handbook's custom-parameter appendix lists the
    options and the control-instruction syntax; the help text of ttfautohint 1.8 gives the
    defaults (`autohint.tt.option.*`). No corpus source holds the parameter, so its stored form is
    taken as ttfautohint's long command-line options in one string (guess, until a file from the
    reference app shows it).
11. Default of TTFOvershootSuppressionBelowPPM when absent.
    **Answer (guess):** no size-based suppression beyond the zones'.
12. gasp: exact flag bits per range, table version, and whether the behaviour per range is fixed or
    editable.
    **Answer (corpus, oracle, guess):** the gasp Table parameter stores the records themselves, a
    dictionary from maximum PPM to flag bits (fontc's test source `WghtVarGasp.glyphs`: 8 → 10,
    20 → 7, 65535 → 15), so the behaviour per range is editable; fontc writes them into a version 1
    table. Without the parameter a hinted font gets the handbook's thresholds 8 and 20 with
    DOGRAY + SYMMETRIC_SMOOTHING, GRIDFIT + SYMMETRIC_GRIDFIT and all four bits (guess on
    `hinting.tt.gasp.flagMapping`).
13. How imported per-glyph instructions are stored and shown, and whether editor instructions and
    imported assembly can coexist in one font.
    **Answer (guess):** per layer, as assembly text with an outline fingerprint; they coexist in
    one font but not in one glyph, where editor instructions win.
14. Layout of the generated `cvt`, `fpgm` and `prep` (function numbering, CVT order); needed only
    for byte parity, not for behaviour.
    **Answer (ours):** FontCraft's own layout; byte parity with the reference is not a goal.
15. Behaviour of instructions on glyphs with components, and on Alternate/Intermediate layers.
    **Answer (guess):** composites get no program of their own, their components' programs apply;
    special layers that replace the main master's layer carry their own instructions.
16. Whether horizontal-direction Snap honours any zones (the text defines Auto for vertical mode
    only).
    **Answer (guess):** no; it rounds to the nearest pixel edge.
17. In variable exports, how per-master zones and stems reach the font (`cvar` is our assumption).
    **Answer (guess, waits for M9):** default-master `cvt ` values plus `cvar` deltas.
18. Lifetime and port of the web preview server.
    **Answer (plan § 7 question 5, guess):** a free port on the local network interfaces; it stops
    when the dialog closes.

## Implementation notes

### Data the source model must store

**Per font**

- Hint-source master id ("Get Hints From Master"); absent = first master.
- TTFBlueFuzz (optional; default 1).
- TTFOvershootSuppressionBelowPPM (optional).
- TTZoneRoundingThreshold (optional; default 0.09375).
- gasp thresholds: two PPM values (optional; default 8 and 20).

**Per master**

- TT zones (optional list; absent = use alignment zones). Each: `name` (unique), `position`,
  `size` (signed), `align` (name of another zone or none).
  - Main master only: `deltas` = map master id → PPM → up | down, and `filter` = Smart Filter
    rules.
- TT stems (optional list; absent = use standard stems). Each: `name`, `orientation` (x | y),
  `size`.
  - Main master only: `deltas` and `filter` as for zones.
- TTFBlueFuzz override (interpolated across masters).
- Imported hinting: `cvt` values (list of integers), `prep` assembly text, `fpgm` assembly text.

**Per instance (export)**

- Autohint on/off override.
- TTFAutohint options (structured map) and control instructions (text).
- TTFBlueFuzz, TTZoneRoundingThreshold, TTMinimumDistance (default 0.25), gasp thresholds:
  overrides of the font values.

**Per layer** (main-master layer; also special layers that replace it)

- Ordered list of TrueType instructions; list order = execution order. Each instruction:
  - `type`: snap | stem | shift | interpolate | delta
  - `direction`: vertical | horizontal
  - node references: `target`; plus `origin` (stem, shift); plus `reference1`, `reference2`
    (interpolate)
  - `zone` (snap): auto | none | zone name
  - `stem` (stem): auto | none | stem name
  - `rounding`: round | up | down | none | gdiOnly
  - `tripleGroup` (stem): id shared by the three linked instructions
  - `cells` (delta): map instance-or-master id → PPM → up | down
- Imported glyph program (assembly text) for glyphs opened from a hinted binary, with a marker of
  the outline it belongs to so it can be dropped when the outline changes.

**Per node**

- Nothing stored on nodes. Instructions hold node references that must:
  - address on-curve and off-curve nodes, and extra nodes (an intersection identified by the two
    crossing segments and its ordinal);
  - survive conversion to quadratic outlines and resolve to exported point numbers;
  - resolve in every compatible master and in interpolated instances;
  - be remappable to the nearest intersection point after overlap removal.

**View state (not in the font):** preview PPM, hinting direction, rendering intent, preview scale,
pixel-preview toggle, point-index toggle, previewed instance.

### Output tables affected

| Table | Manual hinting | Autohint on | Imported hinting kept |
|---|---|---|---|
| `glyf` instructions | Compiled per glyph from the ordered instruction list, IUP last | Written by the autohinter | Stored glyph programs |
| `cvt ` | Zone positions and sizes (with blue fuzz), stem sizes, for the instance | Autohinter's own values | "CVT Table" parameter |
| `fpgm` | Our support functions (zone rounding, stem width per rendering mode, triple hints, minimum distance) | Autohinter's functions | "fpgm Table Assembly" |
| `prep` | Per-PPM setup: zone alignment, zone and stem deltas, rounding threshold, overshoot suppression, control settings | Autohinter's program | "prep Table Assembly" |
| `gasp` | From the gasp thresholds (font or instance), three ranges | same | same |
| `maxp` | Stack, storage, function, twilight and instruction-size maxima recomputed | same | recomputed or kept |
| `cvar` (variable only) | Per-master differences of the zone- and stem-derived `cvt` entries (inferred) | n/a | n/a |
| `head` flags | Bits signalling that instructions depend on point size / may alter advance, as required by the programs | same | kept |

Our generated programs (M10.17, `crates/hinting/src/tt_font.rs`): `cvt ` holds per TT zone its
position and its fitted row, per TT stem its width and fitted width, then the family zone
positions; `prep` rounds the zones (threshold, family zones up to TTFFamilyZonesThreshold),
aligns them, applies zone deltas, rounds the stems and applies stem deltas under GDI ClearType;
`fpgm` holds the functions glyph programs call to snap a point to a zone (overshoots shown from
TTFOvershootSuppressionBelowPPM up), place a stem (five rounding modes, TTMinimumDistance) and
move a point at one size, round a point by a mode, keep a point at a rounded distance from a
zone, and keep a pixel between the stems of a triple hint. The module documentation lists the
function numbers and arguments.

Glyph programs (M10.18, `crates/hinting/src/tt_glyph.rs`) run the main master's instructions in
list order against the exported point numbers: Snap through the zone functions or point
rounding, Stem through the stem function (an untouched origin zone-snapped or rounded first),
Shift and Interpolate through `SHP` and `IP` followed by point rounding, Delta through the point
delta function at its place in the list, the triple-hint function after a group's third Stem,
then `IUP` for each direction used. Deltas run before `IUP` because a rasterizer in
backward-compatibility mode ignores moves after it. Unless TTFDontPreserveDiagonals is set, a
moved point with one flat and one slanted straight neighbour moves along the slanted line.
Ignore Vertical Hints leaves out the horizontal Stem instructions on vertical TT stems. An
instruction naming a point the outline lacks is left out with a warning; a program past 65,535
bytes is an error.

The export (M10.19, `crates/compile/src/tt_hints.rs` and `points.rs`):

| Step | Rule |
|---|---|
| when | a TrueType build, hints not stripped, Export TrueType Hints on, Autohint off; Autohint on writes no TrueType hints until the autohinter exists (M10.21) |
| imported tables | any of cvt Table, prep Table Assembly, fpgm Table Assembly on the build: those tables as they are; manual instructions are not compiled, with a warning (they call our `fpgm`) |
| glyphs | each glyph whose main-master layer has instructions and that exports as a simple outline; composites get none |
| point map | by position on the prepared outline: on-curve nodes at their place (never left out as implied points), off-curve nodes to the nearest off-curve point of their segment, nodes removed by overlap removal and intersection extra nodes to the nearest created on-curve point |
| static instance between masters | built from its interpolated font: glyphs take their main-master layer's hints, TTFZones and TTFStems values are interpolated by name |
| errors | a glyph program that cannot be compiled leaves that glyph unhinted, with a warning naming it; font programs that cannot be built leave the font unhinted, with a warning |
| `maxp` | the largest needs `ttasm::analyse` finds in `fpgm`, `prep` and every glyph program; the longest glyph program |
| `head` | bits 2 and 3 set when the font has programs (guess) |
| variable | the default master's `cvt `, `fpgm`, `prep`; no Delta instructions, no `cvar` yet (a warning says so) |

`hint.tt.getPointIndexes` returns the same point map for one glyph.

Unhinted export (no instructions, autohint off): no `cvt `, `fpgm`, glyph programs; a minimal `prep`
and a `gasp` suited to unhinted rendering are a separate export decision (see the compile spec).

Variable fonts: glyph programs come once from the main master; Delta instructions and instructions
on extra nodes are omitted; `cvt` holds default-master values and `cvar` the master deltas.
