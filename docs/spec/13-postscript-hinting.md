# PostScript hinting

Behaviour notes for `spec/inventory/13-postscript-hinting.toml`.
Sources: Glyphs 4 Handbook, chapter "PostScript Hinting"; AFDKO `otfautohint` help text, fontinfo
documentation and `docs/otfautohint_Notes.md` (Apache-2.0). Row ids are given in `code`.

## 1. Model

Two levels of data (`hinting.ps.twoLevels`):

- **Font-level hints**, per master: alignment zones and standard stems, plus optional scalar
  parameters. They become CFF Private dict entries.
- **Glyph-level hints**, per layer: an ordered list of stem hints and ghost hints.

Hinting is optional (`hinting.ps.hintingIsOptional`). A font may export with neither level; hints
only help regular designs rendered at low resolution by a rasterizer that honours them.

## 2. Standard stems

Rows: `hinting.ps.standardStems`, `.firstValueIsPrimary`, `.interpolateByName`,
`.privateDictMapping`, `hinting.ps.measureStems`.

- Each master has a horizontal list (thickness of horizontal strokes) and a vertical list
  (thickness of upright strokes). Up to twelve values per list are usable. Fewer is better: one
  value, or two (lowercase/uppercase; or stroke/serif for horizontals). Two values that differ
  slightly may render with different pixel counts at the same size, which is the reason to merge
  near-equal values.
- The first value of each list is primary. It is read by non-hinting features too (Cursivy, Rounded
  Font filter are named). Later values are hinting-only. Horizontal stems are also the default
  source of TrueType stems.
- Stems have names. Interpolation pairs stems across masters by name.
- Finding values: measure node-to-node distances (Info box with two nodes selected, or the
  Measurement tool), collect them, take a median. `autohint.ps.reportStems` gives the same as a
  histogram over many glyphs.
- Export mapping (as glyphsLib and ufo2ft write it, used since M2 and equal to fontmake's builds):
  first values → `StdHW`, `StdVW`; full sorted lists → `StemSnapH`, `StemSnapV`; written only when
  both directions have stems.

## 3. Alignment zones

Rows: `hinting.ps.alignmentZones`, `.effect`, `.limits`, `.gridCompensation`,
`.blueValuesMapping`, `.metricDerived`, `hint.ps.validateFontHints`, `hint.ps.getFontHints`.

- A zone = position (flat edge) + size (maximum overshoot). Size > 0: top zone, extends upward.
  Size < 0: bottom zone, extends downward.
- Effect: at small sizes, an outline feature that (a) has a horizontal stem hint or ghost hint and
  (b) has that hint's edge inside a zone is aligned to the flat edge. No hint, no alignment.
- Limits to validate:

  | Rule | Value |
  |---|---|
  | Top zones | at most 6 |
  | Bottom zones other than baseline | at most 5 |
  | Baseline zone | exactly at position 0 |
  | Overlap | forbidden |
  | Gap between zones | at least 1 unit |
  | Zone size | at most 25 units, and at most 240 / (240 × blueScale − 0.98) |

- On a non-default grid, widen each zone by one unit at each end (position shifted by 1 away from
  the zone, size grown by 2). Baseline zone: position stays 0, size grows by 1.
- Export mapping (glyphsLib's `blue_values.py`, used since M2): zones sorted by position; a zone at
  0 or with a positive size → `BlueValues`, the others → `OtherBlues`; each as (lower, upper) pairs
  in ascending order.

## 4. Optional Private dict parameters

Rows: `hinting.ps.blueScale`, `hinting.ps.blueShift`, `hinting.ps.blueFuzz`,
`hinting.ps.familyAlignmentZones`.

The handbook names four custom parameters; their descriptions in the custom-parameter appendix give
the defaults below:

| Parameter | Private dict entry | What we know |
|---|---|---|
| blueScale | `BlueScale` | Limits zone size (formula above). Default 0.037 when unset (parameter description; 0.039625 is its 10-point example). fontmake computes it from the tallest zone instead. |
| blueShift | `BlueShift` | Must be ≥ flex depth + 1 for flex. Set on the Font tab. Default 7 (parameter description). |
| blueFuzz | `BlueFuzz` | Widens zones at both ends for zone tests; otfautohint requires zone gaps ≥ 2 × BlueFuzz + 1. Default 1 when unset (the Type 1 default the parameter description quotes). fontmake writes 0 instead. |
| Family Alignment Zones | `FamilyBlues`, `FamilyOtherBlues` | Zone list shared across a family. |

## 5. Autohinting at export

Rows: `hinting.ps.export.autohintOption`, `.autohintParameter`, `.manualGlyphsSkipped`,
`.stemHintsMadePositive`, `.hintReplacement`, `autohint.ps.run`.

- Export dialog checkbox "Autohint"; an "Autohint" custom parameter overrides the checkbox.
- Per glyph: if the hint-source layer has any manual hint, write exactly those hints; otherwise run
  the autohinter with the instance's zones and stems.
- Normalisation at export: stem hints with negative size are flipped to positive width.
- Overlapping stem hints in one direction are legal in the source; the exporter partitions them into
  non-overlapping groups and emits hint masks (hint replacement). No user control.

## 6. Flex

Rows: `hinting.ps.flex.automatic`, `.conditions`, `.zonePlacement`, `autohint.ps.noFlex`,
`autohint.ps.flexDetection`.

Flex is never stored and never placed by hand. The exporter emits it when all of these hold:

1. Exactly two consecutive curve segments between three on-curve nodes.
2. Node 1 and node 3 share Y (cupped serif) or share X (tapered stem).
3. All three nodes are extremes of the two segments; handles stay inside the span of the nodes;
   handles need not be exactly horizontal/vertical; the halves need not be symmetric.
4. Depth ≤ 20 units.
5. blueShift ≥ depth + 1.

Recommended for cupped serifs: all three nodes inside the zone, middle node exactly on the flat
edge, outer nodes reaching into the zone (bottom serifs dip slightly below the baseline).

otfautohint adds a length test: the two halves must not be more unequal than roughly 1:3, and the
pair must be long relative to its depth.

## 7. Manual hints in Edit View

Rows: commands `hint.ps.autohintGlyph`, `hint.ps.addHorizontalHint`, `hint.ps.addVerticalHint`,
`hint.ps.makeGhostHint`, `hint.ps.setGhostDirection`, `hint.ps.setHintValues`,
`hint.ps.attachHintToNode`, `hint.ps.deleteHints`, `hint.ps.clearGlyphHints`, `hint.ps.listHints`,
`hint.ps.selectNextHint`, `hint.ps.selectPreviousHint`; panel `infoBox.psHint`; behaviours
`hinting.ps.manual.*`.

### Hint kinds

| Kind | Direction | Data | Purpose |
|---|---|---|---|
| Stem hint | horizontal or vertical | origin, size (either end may be linked to a node) | Brackets a stroke, serif or crossbar |
| Ghost hint | horizontal only | position (may be linked to a node), top or bottom | Marks an edge where no horizontal stem exists |

Horizontal hints only act on nodes whose Y equals a hint edge exactly; one hint per height serves
all nodes at that height.

### Creating

| Action | Selection | Result |
|---|---|---|
| Context menu › Add Horizontal Hint / Add Vertical Hint | nothing | Unlinked stem hint (initial origin and size not stated) |
| same | two nodes | Stem hint linked to both nodes |
| same | several node pairs, each pair on its own contour | One linked stem hint per pair |
| Context menu › Add Horizontal Hint | exactly one node | Ghost hint attached to that node |
| Context menu on a hint badge › Make Ghost Hint | a hint | That hint becomes a ghost hint |
| Context menu › Autohint | glyph | Autohinter result stored as editable hints; the glyph is now "manual" |

### Display and editing

- Stem hint: gray bar plus badge with origin and size. Figures use green for vertical, yellow for
  horizontal, blue for ghost hints.
- Ghost hint badge: position plus ↑ (top) or ↓ (bottom).
- Click badge: select. Shift+click: multi-select. Tab / Shift+Tab: next / previous hint.
- Info box: edit origin and size numerically; for a ghost hint, up/down buttons set top or bottom.
- Blue circle = origin marker, blue triangle = far edge and direction. Drag a marker to move that
  edge; drop it on a node to link that end to the node. A linked end follows the node.
- Delete: removes selected hints.
- Link to extremum nodes for best results.

### Mixing rule

A glyph with at least one manual hint gets no automatic hints at export. Hand-hinted glyphs must be
complete.

## 8. Multiple masters and variable fonts

Rows: `hinting.ps.masters.mainMasterOnly`, `.transferByNodeLink`, `.fallbackToOtherMasters`,
`.specialLayers`, `hinting.ps.cff2.sharedHintStructure`, `hinting.ps.cff2.privateDictBlends`,
`autohint.ps.variable.*`.

- Hints are read from the main master: first master, or the one named by the font parameter "Get
  Hints From Master".
- Transfer to other masters and instances happens through node links: a hint end linked to node *n*
  takes node *n*'s coordinate in the interpolated outline. Requires compatible paths. Unlinked hints
  only carry main-master numbers.
- Hints on a non-main master are ignored unless the main master's layer has none.
- Alternate and Intermediate layers: put hints on the special layer that replaces the hint-carrying
  master layer.
- Variable CFF2 (from otfautohint, not the handbook): one hint list and one mask set per glyph for
  the whole design space, coordinates blended.
  - Stems must be in increasing order (lower edge, then upper edge), no duplicates, in every
    master. Out-of-order stems are removed unless conflicts are kept.
  - Two stems overlap (cannot share a mask) if they change order in any master versus the default,
    or if in any master the gap between them is below the overlap margin.
  - Checking masters is enough, provided the design space is interpolation only.
  - Overlapping contours stay in variable fonts; the hinter analyses the overlap-removed shape and
    maps results back to the original segments.

## 9. Testing

Rows: `hinting.ps.testing.external`, `hint.ps.renderPreview` (inferred).

The reference offers no in-app PostScript hint preview: export, set small text in an Adobe
application, magnify with the OS zoom. FontCraft plans an in-app and headless raster preview.

## 10. Autohinter engine (`autohint.ps.*`)

Derived from otfautohint. Commands: `autohint.ps.run`, `autohint.ps.reportStems`,
`autohint.ps.reportZones`, `autohint.ps.listFdDicts`.

### Options

| otfautohint option | Row | Default | Effect |
|---|---|---|---|
| `-g`, `--glyphs-file`, `-x`, `--exclude-glyphs-file` | `autohint.ps.glyphSelection` | all glyphs | Include or exclude list; names, indexes, `/CID`, ranges; mutually exclusive |
| `-m`, `--overlaps-file`, `--force-overlap`, `--force-no-overlap` | `autohint.ps.overlapCorrection` | on for variable / reference runs, off for static | Hint overlapped glyphs as if overlap were removed |
| `--loose-overlap-mapping` | `autohint.ps.looseOverlapMapping` | off | Looser segment matching after overlap removal |
| `-o` | `autohint.ps.output` | in place | Write to new files, one per input |
| `-r`, `-b` | `autohint.ps.referenceFont` | none | Hint several masters compatibly from a reference |
| `-a` | `autohint.ps.hintAll` | off | Re-hint glyphs even if unchanged (UFO) |
| `-w` | `autohint.ps.writeToDefaultLayer` | off | Write into the default layer (UFO) |
| `-k` | `autohint.ps.variable.keepConflicts` | off | Keep stems whose order inverts in the design space |
| `-c` | `autohint.ps.allowChanges` | off | May reorder contours and flatten near-straight curves |
| `--report-only` | `autohint.ps.output` | off | Analyse, change nothing; exclusive with `-c` |
| `-d` | `autohint.ps.decimalCoordinates` | off | Keep fractional coordinates |
| `--no-flex` | `autohint.ps.noFlex` | flex on | No flex |
| `--no-hint-sub` | `autohint.ps.hintSubstitution` | substitution on | One hint set per glyph, no masks |
| `--no-zones-stems` | `autohint.ps.noZonesStems` | off | Accept a font without zones or stems |
| `--fontinfo-file`, `--ignore-fontinfo` | `autohint.ps.fontinfoOverrides` | `fontinfo` beside the font | Per-glyph-group zone and stem sets |
| `--print-list-fddict`, `--print-all-fddict` | `autohint.ps.listFdDicts` | n/a | List zone/stem sets and their glyphs |
| `--max-segments` | `autohint.ps.maxSegments` | 100 | Skip a direction for over-complex glyphs (not for explicitly named glyphs) |
| `-p` | `autohint.ps.parallelism` | CPU count | Worker processes; negative = cores minus N |
| `-v`, `--log` | `autohint.ps.diagnostics` | quiet | Verbosity, log file, near-miss reports |
| stemhist `-z`, `-a` | `autohint.ps.reportZones`, `autohint.ps.reportStems` | stems, straight only | Zone report; include curved stems |

### Concepts

- **Stem candidates** (`autohint.ps.stemCandidates`): opposing segment pairs, scored; widths
  matching known stems preferred; initial width cap 150 units; inflected curves split for analysis.
- **Ghost stems** (`autohint.ps.ghostStems`): an in-zone edge with no partner gets a synthetic
  stem so it can still align.
- **Zone membership** (`autohint.ps.zoneMembership`): zone widened by BlueFuzz at both ends;
  `at`, `bullet`, `copyright`, `currency`, `registered` never count as in a zone.
- **Counter hints** (`autohint.ps.counterHints`): built-in lists (V: m, M, T, ellipsis; H: element,
  equivalence, notelement, divide), extendable by 64 names each.
- **Hint substitution** (`autohint.ps.hintSubstitution`): conflicting stems distributed over masks.
- **Per-group overrides** (`autohint.ps.fontinfoOverrides*`, `autohint.ps.dominantStems`): named
  zone/stem sets plus glyph lists (names or regular expressions). Once any override exists, global
  values are ignored and unmatched glyphs get a set with zones outside the font bounding box.
  Minimum content of a set: baseline pair, one top-zone pair, DominantH, DominantV.

FontCraft should hold overrides as structured data in the source (named hint sets with glyph
filters) and may import/export the `fontinfo` text syntax; a CID-keyed or multi-FDDict export maps
each set to one Private dict.

## Open questions

Answers from the M10 planning pass (`docs/plans/M10.md` § 3); "guess" means a choice of ours
recorded in the row's `guesses`, to be checked in the reference app (§ 8 of the plan).

1. Which tool must be active for PostScript hints to be editable in Edit View. (Visibility is the
   `view.showHints` toggle inventoried in `03b-edit-view-display.toml`; this chapter does not
   mention it.)
   **Answer (guess):** the Select tool, while Show Hints is on.
2. Initial origin and size of a hint added with nothing selected.
   **Answer (guess):** centred on the glyph's bounding box, sized to the master's first stem of
   that direction, 50 units without one.
3. Default orientation of a new ghost hint (from single-node creation and from Make Ghost Hint); can
   a ghost hint be turned back into a stem hint?
   **Answer (guess):** top when the edge lies in or nearer to a top zone, else bottom; giving a
   ghost a size makes it a stem hint again.
4. Does typing a value in the Info box break a node link? How is a link removed in the UI?
   **Answer (guess):** a typed value unlinks that end; dragging a marker off its node unlinks it;
   `hint.ps.attachHintToNode` with `null` does the same headless.
5. Does the context-menu Autohint replace existing hints or add to them? Does it act on all
   selected glyphs?
   **Answer (guess):** it replaces them, on every selected glyph.
6. Wrap-around of Tab / Shift+Tab; behaviour with multiple hints selected.
   **Answer (guess):** both wrap; with several selected, Tab moves on from the last selected.
7. Scope of the Autohint, blueScale, blueFuzz and Family Alignment Zones parameters (font, master,
   instance) and their defaults when absent; whether absent means "omit the entry" or "write the
   CFF default".
   **Answer (handbook, guess):** Autohint is an instance parameter that forces autohinting
   (custom-parameter descriptions); FontCraft also reads it on the font (guess). blueShift,
   blueScale, blueFuzz and Family Alignment Zones are font parameters (the handbook sets blueShift
   in Font Info › Font); the inventory's parameter rows give the scopes. Absent values: the export
   writes the parameter descriptions' defaults, BlueScale 0.037, BlueFuzz 1 and BlueShift 7, as
   the handbook gives them (plan § 7, question 6). fontmake computes BlueScale from the tallest
   zone and writes BlueFuzz 0; `docs/spec/40-compile.md` § 4a records the difference. As in
   fontmake, the three entries are written only when the master has alignment zones (guess).
   FamilyBlues are omitted. Documented, not yet observed: a reference export with no zone
   settings would confirm it.
8. Exact Private dict mapping of zones and stems (order, which stems go to StemSnap, treatment of
   more than twelve stems or too many zones: truncate, error or warn).
   **Answer (oracle, guess):** glyphsLib's rule, used since M2 and equal to fontmake's builds:
   zones sorted by position, a zone at 0 or with a positive size to BlueValues, the rest to
   OtherBlues; first stems to StdHW and StdVW, sorted lists to StemSnap, only when both directions
   have stems. Beyond the CFF limits the export keeps the first entries and warns (guess).
9. When several non-main masters carry hints and the main master has none, which one is used?
   **Answer (guess):** the first of them in master order.
10. For variable CFF2 exports, how the reference derives hints (main-master manual hints with
    linked nodes versus running an autohinter on the variable font) and whether unlinked hints are
    dropped.
    **Answer (guess, waits for M9):** manual hints resolved through node links in every master,
    unlinked ends keeping one value; otherwise the multiple-master autohinter.
11. How hints behave on layers with components (are component hints reused at export?) and after
    Remove Overlap at export when linked nodes disappear.
    **Answer (ours):** CFF output decomposes composites; a composite without manual hints is
    autohinted as its decomposed outline, and its components' manual hints are not reused (guess).
    Linked ends are resolved before overlap removal, since a PostScript hint is a coordinate.
12. Which autohinter the reference runs and with which options; whether flex follows the handbook's
    conditions or the autohinter's own tests when they differ.
    **Answer (ours, plan § 7 question 3):** a port of otfautohint with its defaults; flex follows
    otfautohint's test, which contains the handbook's conditions and adds a length ratio (guess).

## Implementation notes

### Data the source model must store

**Per font**

- Hint-source master id ("Get Hints From Master"); absent = first master.
- Optional scalars: blueShift, blueScale, blueFuzz (each optional, no implicit value stored).
- Family alignment zones: list of (position, size).
- Autohint override for exports (tri-state: unset / on / off), also storable per instance.
- Named hint sets for glyph groups (zones, stems, FlexOK, counter-glyph lists, glyph filter) for
  FDDict-style overrides; optional.

**Per master**

- Alignment zones: ordered list of (position: number, size: signed number), tied to metrics where
  the Font Info model does so.
- Stems: ordered list of (name: string, orientation: horizontal | vertical, value: number). Names
  are the cross-master key.
- Any master-level overrides of the font scalars if the parameter model allows them.

**Per layer** (master layers, and Alternate/Intermediate layers)

- Ordered list of PostScript hints. Each hint:
  - `direction`: horizontal | vertical
  - `kind`: stem | ghostTop | ghostBottom
  - `origin`: number, or a node reference
  - `size`: signed number (stem only), or a second node reference for the far edge
  - optional name/other metadata needed to round-trip `.glyphs` files
- `autohinted` marker with a fingerprint of the outline the stored autohints were computed from
  (lets a re-run skip unchanged glyphs; never written to the font).
- Nothing for flex, hint masks or counter masks: all computed at export.

**Per node**

- Nothing is stored on the node. Hints reference nodes; a node reference is (path index, node
  index) or a stable node id that survives reordering, start-point changes and compatible-master
  lookup. Deleting a referenced node must degrade the hint end to a plain number.

### Output affected

**CFF / CFF2 Private dict** (one per font, or one per FDDict when hint sets are used)

| Entry | From |
|---|---|
| `BlueValues` | baseline zone + top zones |
| `OtherBlues` | remaining bottom zones |
| `FamilyBlues`, `FamilyOtherBlues` | Family Alignment Zones |
| `StdHW`, `StdVW` | first horizontal / vertical stem |
| `StemSnapH`, `StemSnapV` | full stem lists, sorted, max 12 |
| `BlueScale`, `BlueShift`, `BlueFuzz` | the three scalar parameters when set |
| `LanguageGroup` | hint set flag (CJK counter control), when used |

In CFF2 variable output the array and number entries carry `blend` operands built from the
per-master values; all masters need matching zone counts and stem names.

**CharStrings**

- `hstem` / `vstem` / `hstemhm` / `vstemhm` from the layer's hints (or the autohinter), stems
  sorted and positive; ghost hints as the reserved edge widths.
- `hintmask` for hint replacement; `cntrmask` for counter hints.
- Flex operators (`flex`, `hflex`, `hflex1`, `flex1`) replacing qualifying curve pairs.
- CFF2: the same operators with blended coordinates; one structure for all masters.

**Other tables:** none. PostScript hinting does not touch `gasp`, `cvt`, `fpgm`, `prep` or `cvar`.

**Source formats:** UFO stores autohint results per glyph in glif lib data (and in a processed layer
unless written to the default layer) and Private dict values in `fontinfo.plist` (`postscriptBlueValues`,
`postscriptOtherBlues`, `postscriptFamilyBlues`, `postscriptFamilyOtherBlues`, `postscriptStemSnapH`,
`postscriptStemSnapV`, `postscriptBlueScale`, `postscriptBlueShift`, `postscriptBlueFuzz`); import
and export must map to and from the model above.
