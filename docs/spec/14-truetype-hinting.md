# TrueType hinting

Behaviour notes for `spec/inventory/14-truetype-hinting.toml`.
Source: Glyphs 4 Handbook, chapter "TrueType Hinting". Rows marked `memory` (the individual
autohinter options) are recalled from the ttfautohint documentation and need checking. Row ids are
given in `code`.

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
"TTFAutohint control instructions". The handbook lists no options. Recalled option set (verify):

| Option | Recalled default | Meaning |
|---|---|---|
| hinting-range-min / -max | 8 / 50 | PPM range that gets its own hint sets |
| hinting-limit | 200 | PPM above which hinting switches off; 0 = never |
| increase-x-height | 14 | Round x-height up more readily up to this PPM; 0 = off |
| x-height-snapping-exceptions | none | PPM list exempt from x-height snapping |
| fallback-stem-width | none | Stem width for scripts without a derived one |
| stem width mode (strong-stem-width) | per target | natural / quantized / strong for grayscale, GDI ClearType, DirectWrite |
| default-script, fallback-script, fallback-scaling | latn, none, off | Script assignment for unclassified glyphs |
| windows-compatibility | off | Extra zones at usWinAscent / usWinDescent |
| hint-composites, adjust-subglyphs | off | Composite handling |
| symbol, ignore-restrictions | off | Fonts without standard characters; restricted fonts |
| dehint | off | Strip hinting |
| no-info / detailed-info | info on | Version-string annotation |

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
`xHeight` zone at 5.25 px → 5.65 → 6. With the default the same zones give 6 and 5.

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

1. Tool shortcut and toolbar position of the TrueType Instructor; how node selection works in it
   (rectangle, Shift, Select All).
2. Where the rendering intent (Grayscale / ClearType / DirectWrite) is chosen.
3. Context-menu label of the Stem instruction; whether Delete removes selected instructions; how a
   triple hint is dissolved; whether instructions can be reordered.
4. What "drag to reposition" changes: the attached node(s) or only the badge position?
5. Colour of Delta badges; exact look of each rounding icon.
6. Delta table: PPM row range; whether the columns for Delta instructions are masters or instances;
   whether zone/stem deltas survive variable export.
7. "Get PS zones" / "Get PS stems": replace or append; names given to imported entries.
8. How zones and stems are matched across masters (name or index) and what happens when a master
   lacks an entry; whether stem names must be unique.
9. What happens to instructions referring to a removed or renamed zone or stem.
10. Exact option list, defaults and storage format of "TTFAutohint options", and the accepted
    control-instruction syntax.
11. Default of TTFOvershootSuppressionBelowPPM when absent.
12. gasp: exact flag bits per range, table version, and whether the behaviour per range is fixed or
    editable.
13. How imported per-glyph instructions are stored and shown, and whether editor instructions and
    imported assembly can coexist in one font.
14. Layout of the generated `cvt`, `fpgm` and `prep` (function numbering, CVT order); needed only
    for byte parity, not for behaviour.
15. Behaviour of instructions on glyphs with components, and on Alternate/Intermediate layers.
16. Whether horizontal-direction Snap honours any zones (the text defines Auto for vertical mode
    only).
17. In variable exports, how per-master zones and stems reach the font (`cvar` is our assumption).
18. Lifetime and port of the web preview server.

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

Unhinted export (no instructions, autohint off): no `cvt `, `fpgm`, glyph programs; a minimal `prep`
and a `gasp` suited to unhinted rendering are a separate export decision (see the compile spec).

Variable fonts: glyph programs come once from the main master; Delta instructions and instructions
on extra nodes are omitted; `cvt` holds default-master values and `cvar` the master deltas.
