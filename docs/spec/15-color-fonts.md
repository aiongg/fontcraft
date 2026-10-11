# Colour fonts

Source: Glyphs 4 Handbook, chapter "Color Fonts" (https://handbook.glyphsapp.com/color-fonts/),
with parameter names checked against the handbook appendix, layer attribute keys against the
GlyphsSDK file-format documents, and table terminology against fontations and fontc.
Inventory: `spec/inventory/15-color-fonts.toml`. Date: 2026-10-07.

Status words used below: **stated** = the chapter says so; **appendix** = only the handbook appendix
says so; **docs** = taken from a permissively licensed compiler or the OpenType spec; **ours** = a
FontCraft design decision the reference does not document.

## 1. Data model

Colour is expressed through layers attached to a master of a glyph, plus font-level palettes and a
handful of custom parameters. Four layer types exist besides ordinary master layers:

| Layer type | Label in the Layers palette | Glyphs file attribute | Carries | Written to |
|---|---|---|---|---|
| Colour palette layer | `Color <index>` (e.g. `Color 0`); `*` for the text colour | `colorPalette`: integer or `"*"` | outlines painted in one palette entry | CPAL + COLR v0, or SVG |
| Full-colour layer | `Color`, with a colour-spectrum disc | `color`: true | paths with per-path fill, stroke, gradient | SVG; COLR v1 (docs) |
| Bitmap layer | `iColor <size>` (e.g. `iColor 256`) | `sbixSize`: integer | one referenced bitmap at one pixel size | sbix, or SVG |
| SVG layer | `svg` | `svg`: true | one referenced SVG file | SVG |

All four are "special layers": indented under their master in the Layers palette, set in bold, one
set per master. They are created the same way: add a layer with the palette's plus button, then pick
the type from the context menu of the new entry (`layer.convertToColorPalette`,
`layer.convertToColor`, `layer.convertToIColor`, `layer.convertToSvg`). Headless callers pass the
glyph and layer explicitly. `layer.listColorLayers` (ours) reports the result.

A layered colour font uses none of these: its colour "layers" are whole masters.

Font-level and master-level data:

- `Color Palettes` (font): list of palettes, each a list of RGBA colours. A master may carry its
  own, which wins for that master (docs: both oracles; `color.palette.perMaster`).
- `Master Color`, `Master Color Dark` (master): editor-only preview colours.
- `Link Metrics With First Master`, `Link Metrics With Master` (master): metric sharing.
- `Enforce Compatibility Check` (font): switched off for non-interpolating colour masters.
- Instance switches: see section 9.

The parameter rows themselves belong to the appendix inventory; this chapter's inventory records
their effect on colour output as `color.*` behaviour rows.

## 2. Working rules common to all colour fonts

**Metrics in sync.** Colour layers only register if all masters share sidebearings and kerning. Add
`Link Metrics With First Master` to every master but the first, or `Link Metrics With Master` (which
names its source master) to follow some other master. Rows: `color.metrics.linkWithFirstMaster`,
`color.metrics.linkWithMaster`.

**Preview.**

- Edit View draws the *other* glyphs of the tab in colour while the selected layer of the current
  glyph is a colour layer (`color.preview.editViewOtherGlyphs`).
- The Preview area under Edit View and the Preview Panel draw a glyph in colour only while one of
  its colour layers is selected (`color.preview.previewArea`).
- Text Preview shows the system text engine's rendering, i.e. the compiled result
  (`color.preview.textPreview`). FontCraft has to compile the instance and render that; the editor's
  own layer compositing must not be reused there, or table bugs stay invisible.
- Font View cells show palette colours once `Color Palettes` exists (appendix;
  `color.preview.paletteDrivesFontView`).

**Export flavours.** Colour tables are independent of outline flavour and container: CFF `.otf`,
TrueType `.ttf`, WOFF and WOFF2 all accept them (`color.export.anyFlavour`).

## 3. Layered colour fonts

One font file per colour; the user stacks text frames in a layout program.

Setup (all stated):

1. Add an axis in Font Info › Font named `Color`, with a four-letter tag such as `COLR`
   (`color.layered.colorAxis`). Nothing is interpolated along it.
2. Add `Enforce Compatibility Check` to the font and uncheck it, so masters may differ in structure
   and the incompatibility warnings stop (`color.layered.noCompatibilityCheck`).
3. Add one master per colour, with a descriptive name and a distinct numeric Color coordinate
   (1, 2, …) (`color.layered.masterPerColor`).
4. Give each master a `Master Color` (`color.layered.masterColor`). The picker has an Opacity slider
   (`dialog.masterColorPicker`); the colour is never exported. The parameter can be added to several
   selected masters at once. `Master Color Dark` overrides it under dark appearance.
5. In Font Info › Exports, plus button › Add Instance for each Master
   (`color.layered.instancePerMaster`).

Editing: the layers are plain master layers. Only the selected master is drawn until others are
switched on with the eye buttons in the Layers palette; showing all is the normal working state
(`color.layered.editLikeMasters`). Master order in Font Info is z-order, first master on top
(`color.layered.stackingOrder`).

Export:

- Default: one colourless font per instance, no colour table (`color.layered.exportSeparateFonts`).
- One-file SVG: on an additional instance check `Color Layers to SVG` and `Export SVG Table`
  (`color.layered.toSvg`).
- One-file COLR (appendix): `Color Layers to COLR` builds CPAL + COLR out of every master carrying a
  `Master Color` (`color.layered.toColr`).

## 4. Colour palettes

Stored in the font-level `Color Palettes` parameter and edited in a dialog opened by clicking the
parameter's value (`colorPalette.edit`, panel `dialog.colorPalettes`).

- The dialog is a table: rows are colour indices starting at 0, columns are palettes whose header
  shows the palette number, also starting at 0.
- Plus adds a colour row (`colorPalette.addColor`); selecting a row and pressing minus deletes it
  (`colorPalette.removeColor`); clicking a swatch opens the colour picker
  (`colorPalette.setColor`).
- Gear menu › Add Palette adds a column (`colorPalette.addPalette`); clicking a column heading
  offers Remove Palette (`colorPalette.removePalette`).
- OK commits, Cancel restores the previous palettes (`color.palette.editorIsTransactional`). In
  FontCraft the dialog yields a single undo step; the engine commands above remain individually
  callable for agents.
- Because rows span all columns, every palette has the same number of entries, which is what CPAL
  needs (`color.palette.multiplePalettes`).

`colorPalette.list` (ours) returns the table as data.

## 5. Colour palette layers (COLR v0 style)

- Create: add a layer, context menu › Color Palette (`layer.convertToColorPalette`).
- Assign a colour: the swatch right of the layer name opens a chooser with the palette entries and
  an asterisk (`layer.setColorPaletteIndex`, panel `palette.layers.paletteColorChooser`).
- The asterisk means "the text colour the user picks". It previews black (white in dark appearance)
  and corresponds to palette index 0xFFFF in the file format and in COLR
  (`color.paletteLayer.foregroundColor`).
- The layer label is `Color` plus the index (`color.paletteLayer.naming`).
- No uniformity is required: layers may repeat an index, indices may be unused, and glyphs may have
  different numbers of layers (`color.paletteLayer.freeAssignment`).
- **Fallback.** The master layer's outline stays in the base glyph. Renderers without COLR support
  show it; renderers with support never do (`color.fallback.masterLayerColr`).

Export (stated unless marked):

- CPAL and COLR are written when palette layers exist (`color.export.colrFromPaletteLayers`).
- The first palette is used unless the instance has `Color Palette for CPAL` set to another 0-based
  palette number (`color.export.defaultPalette`, `color.export.colorPaletteForCpal`).
- `Export COLR Table`, unchecked, removes both CPAL and COLR and leaves the fallback outlines
  (`color.export.exportColrTableOff`).
- Conversion to SVG: `Color Layers to SVG` checked, `Color Palette for SVG` = palette number whose
  colours are baked in, `Export SVG Table` checked (`color.export.colrToSvg`,
  `color.export.colorPaletteForSvg`).

## 6. Full-colour layers with gradients (COLR v1 style)

- Create: add a layer, context menu › Color (`layer.convertToColor`). Label `Color`, spectrum disc.
- Draw with the normal tools; new shapes start with a gray fill (`color.fullColor.defaultGrayFill`).
- Style is per path, edited for the selected paths in the Attributes inspector at the bottom of the
  Palette, shown or hidden with View › Show Info (Cmd+Shift+I) (panel `inspector.attributes`).

Attributes:

| Attribute | Controls | Commands | Notes |
|---|---|---|---|
| Stroke | width field, alignment centre / left / right, swatch, minus | `attributes.setStrokeWidth`, `attributes.setStrokeAlignment`, `attributes.setStrokeColor`, `attributes.removeStroke` | Width in font units; empty = 0; centred by default; left/right move the stroke to one side of the path; minus sets width 0 and a transparent colour |
| Fill | swatch, minus | `attributes.setFillColor`, `attributes.removeFill` | Minus makes the fill transparent; the row is hidden while a gradient exists |
| Gradient | linear / radial icons, gradient bar | `attributes.addGradient`, `attributes.setGradientType`, `attributes.addGradientStop`, `attributes.removeGradientStop`, `attributes.setGradientStopColor`, `attributes.setGradientPosition` | Added with the plus button; replaces Fill; at least two stops |
| Shadow, Inner Shadow, Pattern Image | (not described) | `attributes.addAttribute` | Only used by Filter › Glyph as Image (PNG/PDF); never exported to a font |

Gradient editing: knobs under the bar are the stops (click to recolour, click the bar to add, drag a
knob away to delete; panel `inspector.attributes.gradientBar`). In Edit View the geometry has its
own knobs: start and end for linear, one origin for radial (`color.fullColor.gradientCanvasKnobs`).
The Glyphs file format stores gradient points relative to the shape's bounds, and its version 4 adds
a conic gradient, radii, an angle and an extend mode that the chapter does not cover
(`color.fullColor.gradientTypes`).

A swatch crossed by a red line means fully transparent (`color.fullColor.transparentSwatch`).

Export:

- Stated: `Color Layers to SVG` on an instance writes each glyph's Color layer as an SVG document
  (`color.export.fullColorToSvg`).
- Docs, not in the chapter: the same layers can be compiled to COLR v1
  (`color.export.fullColorToColrV1`). Both oracles merge consecutive paths with equal attributes
  into one layer glyph and build, per colour glyph, a `PaintColrLayers` whose entries are
  `PaintGlyph` over `PaintSolid`, `PaintLinearGradient` or `PaintRadialGradient`, with a clip box.
  fontc falls back to COLR v0 when every glyph is a stack of solid layers and reads only the
  master layer's own attributes; fontmake writes version 1 and reads separate Color layers
  (`color.export.colrVersionChoice`; M11 plan § 2).
- Ours: strokes have no COLR paint, so they are expanded to outlines for COLR and kept as strokes in
  SVG (`color.export.strokeInColr`).

## 7. Bitmap colour (sbix)

Images:

- Sources are PNG, JPEG or TIFF (`color.sbix.bitmapFormats`).
- One image per glyph per pixel size; several sizes are allowed and advisable. Sizes are pixels per
  em, not font units (`color.sbix.imagePerSize`, `color.sbix.sizeIsPixels`).
- All sizes of one glyph must share one width ÷ height ratio (`color.sbix.sameAspectRatio`).
  `color.describeExport` (ours) reports violations.
- Naming advice: `<glyph name> <pixel size>.png`, e.g. `sun 48.png`
  (`color.sbix.fileNamingTip`). Nothing says the app resolves files by this name.
- Files are referenced, not embedded; keep them in a folder beside the source
  (`color.images.referencedNotEmbedded`).

Layers:

- The master layer holds no artwork. Put two tiny paths at the bottom-left and top-right; their
  bounding box positions the images on the iColor layers and acts as a clip mask in some software.
  The paths may be degenerate (all nodes on one point); the coinciding-node warning marker is then
  expected (`color.sbix.masterLayerBounds`, `color.sbix.zeroSizeCornerPaths`).
- Create: add a layer, context menu › iColor (`layer.convertToIColor`). Label `iColor <size>`.
- Double-click the entry to change the size (`layer.setIColorSize`). On-canvas size is inversely
  proportional to it: a larger number shrinks the image (`color.sbix.sizeScalesImage`).
- One iColor layer per resolution. Their width follows the master layer
  (`color.layers.widthFollowsMaster`).
- Attaching the image: assumed to use the general image commands; `layer.setIColorImage` is the
  explicit headless form (ours).

Export:

- sbix is written whenever iColor layers exist (`color.export.sbixAutomatic`).
- `Export sbix Table`, unchecked, omits it (`color.export.exportSbixTableOff`).
- `SBIX to SVG` = pixel size also writes those bitmaps into an SVG table
  (`color.export.sbixToSvg`). Per glyph the layer picked is: exact size, else the smallest larger
  one, else the largest smaller one (`color.export.sbixToSvgSizeChoice`).
- Both together give an SVG-only bitmap font (`color.export.svgOnlyFromSbix`).

## 8. SVG colour glyphs

Three routes into the SVG table (`color.svg.threeSources`):

1. Conversion from layered masters, palette layers or iColor layers (sections 3, 5, 7).
2. Existing SVG files: add a layer, context menu › svg (`layer.convertToSvg`), then drop the file on
   it (`layer.setSvgImage`). The file may contain bitmap data. It is referenced, not embedded;
   naming advice is `<glyph name>.svg` in a folder `Images`. It is moved and scaled like any placed
   image, and the layer width follows the master (`color.svg.imageTransform`,
   `color.layers.widthFollowsMaster`).
3. Full-colour layers drawn in the editor (section 6) with `Color Layers to SVG`.

Fallback is again the master layer outline (`color.fallback.masterLayerSvg`).

## 9. Export switches

All are per instance, in Font Info › Exports (`color.export.perInstance`).

| Parameter | Type | Effect | Row |
|---|---|---|---|
| `Color Layers to SVG` | bool | Vector colour layers (Master Color masters, palette layers, full-colour layers) become SVG documents | `color.export.colorLayersToSvg` |
| `Color Layers to COLR` | bool | Layered masters with `Master Color` become CPAL + COLR (appendix) | `color.layered.toColr` |
| `Color Palette for CPAL` | int, 0-based | Palette used for the instance's CPAL; default 0 | `color.export.colorPaletteForCpal` |
| `Color Palette for SVG` | int, 0-based | Palette baked into SVG on conversion | `color.export.colorPaletteForSvg` |
| `Export COLR Table` | bool | Unchecked: no CPAL, no COLR | `color.export.exportColrTableOff` |
| `Export sbix Table` | bool | Unchecked: no sbix | `color.export.exportSbixTableOff` |
| `Export SVG Table` | bool | Whether the SVG table is included | `color.export.exportSvgTable` |
| `SBIX to SVG` | int, pixel size | Copies bitmaps of that size into SVG | `color.export.sbixToSvg` |

Glyph filters at export can test `colorIndex`, `isAnyColorGlyph` and `isAppleColorGlyph`
(appendix; `color.export.glyphFilterTokens`).

`color.describeExport` (ours) resolves these switches for an instance and says which tables will be
written and why.

## 10. Workflow to output tables

| Workflow | Source data | Default output | Optional output | Fallback |
|---|---|---|---|---|
| Layered colour font | one master per colour, `Master Color` | none: one plain font per instance | SVG (`Color Layers to SVG` + `Export SVG Table`); CPAL + COLR (`Color Layers to COLR`, appendix) | not applicable (each file is a plain font) |
| Palettes | `Color Palettes` | CPAL: `numPalettes` palettes × `numPaletteEntries` colours, BGRA records | palette choice via `Color Palette for CPAL` | not applicable |
| Colour palette layers | `Color <n>` layers | CPAL + COLR v0: a BaseGlyph record per colour glyph pointing at Layer records (layer glyph id + palette index; 0xFFFF = text colour) | SVG (`Color Layers to SVG`, `Color Palette for SVG`, `Export SVG Table`); none (`Export COLR Table` off) | master layer outline in the base glyph |
| Full-colour layers | `Color` layers with path attributes | SVG, when `Color Layers to SVG` is checked (stated) | CPAL + COLR v1: BaseGlyphList → PaintColrLayers → PaintGlyph → PaintSolid / PaintLinearGradient / PaintRadialGradient, plus ClipList (docs) | master layer outline |
| Bitmap layers | `iColor <size>` layers with referenced images | sbix: one Strike per pixel size, GlyphData per glyph with a graphic type tag (`png `, `jpg `, `tiff`) | SVG (`SBIX to SVG`); none (`Export sbix Table` off) | master layer (corner paths only) |
| SVG layers | `svg` layers with referenced files | SVG: SVGDocumentList of records mapping glyph id ranges to documents | none stated | master layer outline |
| (not in the reference) | `iColor` layers | none | CBDT + CBLC as a FontCraft-only option (`color.export.cbdtNotInReference`) | master layer |

Notes for the compiler:

- COLR layer glyphs are extra, unencoded glyphs in the glyph order, one per colour layer; they carry
  the layer's outline in `glyf` or `CFF `. Both oracles name them `<base>.color<i>` and share none
  between glyphs (open question 2).
- An SVG document in the table must address glyphs by element id `glyph<ID>`, so the SVG is written
  after the final glyph order is known. Referenced files need their coordinates mapped from the
  layer placement (y axis flipped, origin at the baseline).
- sbix requires `hmtx` advances from the master layer; `originOffsetX/Y` come from the image
  placement relative to the glyph origin.
- Converting bitmaps to SVG means wrapping each bitmap in an `<image>` element with embedded data.

## 11. Table support in fontations

Inspected in the local clone at commit `d6a9007c` (2026-10-07): **read-fonts 0.45.0**,
**write-fonts 0.54.0**, with skrifa 0.48.0 for the higher-level readers. fontc in the local clone
pins write-fonts 0.54.0 and skrifa 0.48.0.

| Table | read-fonts 0.45.0 | write-fonts 0.54.0 | Remarks |
|---|---|---|---|
| CPAL | yes (`tables/cpal.rs`): versions 0 and 1 (palette types and label arrays) | yes (`tables/cpal.rs`) | fontc already emits it (`fontbe/src/cpal.rs`) |
| COLR v0 | yes (`tables/colr.rs`): BaseGlyph and Layer records, `v0_base_glyph`, `v0_layer` | yes (`tables/colr.rs`) | Version is computed on write: 0 unless a v1-only field is set |
| COLR v1 | yes: BaseGlyphList, LayerList, ClipList, all Paint formats including the variable ones, ColorLine / VarColorLine, composite modes; closure helpers in `tables/colr/` | yes: generated from the same definitions, with DeltaSetIndexMap and ItemVariationStore | fontc emits Glyph, Solid, LinearGradient, RadialGradient and ColrLayers paints with clip boxes; skrifa `color` traverses paints for rendering |
| sbix | yes (`tables/sbix.rs`): header, Strike, GlyphData, `glyph_data` | partly (`tables/sbix.rs`) | On write the header flags are masked to bit 0, so the "draw outlines" flag (bit 1) cannot currently be emitted. A `Strike` holds only offsets and does not own its `GlyphData`, so the caller lays out the strike bytes (M11 planning pass) |
| SVG | yes (`tables/svg.rs`): document list, `glyph_data` returns the raw, possibly gzip-compressed document | **no** module | Reading stops at the bytes; no SVG parsing or rendering in fontations. Writing needs our own serializer |
| CBDT | yes (`tables/cbdt.rs`, shared code in `tables/bitmap.rs`) | **no** module | skrifa `bitmap` reads strikes from sbix, CBDT and EBDT, in that order of preference |
| CBLC | yes (`tables/cblc.rs`, shared code in `tables/bitmap.rs`) | **no** module | as CBDT |

Consequences for FontCraft:

- CPAL and COLR v0/v1 can be written with write-fonts today (`write = "write-fonts"`). sbix is
  serialised by FontCraft (`write = "own"`) and added with `FontBuilder::add_raw`, as SVG, CBDT
  and CBLC will be.
- SVG, CBDT and CBLC need `write = "own"` (or `"none-yet"` until built). The SVG table container is
  small (header, document records, concatenated documents); the work is generating the documents.
- Opening an existing colour font for editing can use read-fonts for all six tables.
- The sbix "draw outlines" limitation matters only if we want the outline drawn together with the
  bitmap; the reference workflow relies on the outline as fallback only.

## 12. Conflicts and notes for the command model

- Several actions exist in the reference only as direct manipulation (dropping an SVG file on a
  layer, dragging gradient knobs, dragging a stop knob off the bar, double-clicking a layer entry).
  Each has an explicit command with coordinates or paths as parameters; the gesture dispatches it.
- The palette editor is modal with OK/Cancel, while engine commands apply immediately. The dialog
  keeps a staged copy and issues one combined change on OK.
- The layer-type items sit in a context menu, so their `menu` field is empty.
- Custom parameters carry most of this chapter's export behaviour. They are `[[parameter]]` rows in
  the appendix inventory; to avoid duplicate names they appear here only as behaviours.

## Open questions

Settle these by observing the reference app or its exported fonts.

1. Stacking order of colour palette layers in COLR: does the top entry in the Layers palette become
   the top (last) COLR layer?
   **Answered for M11 (oracle):** both fontc 1.0.0 and fontmake 3.12.1 write the first colour layer
   in the file as the first layer record, painted at the bottom; FontCraft does the same (guess on
   `color.export.colrFromPaletteLayers`, to observe).
2. Names and order of the generated COLR layer glyphs, and whether identical layers across glyphs
   are shared.
   **Answered for M11 (oracle):** both oracles name layer glyphs `<base>.color<i>` by position among
   the colour layers, merge consecutive shapes with equal attributes into one layer glyph, and share
   nothing between glyphs. fontc names from the production name and orders numerically; fontmake
   names from the source name and sorts as strings. FontCraft follows fontc.
3. Does `Color Palette for CPAL` write only the chosen palette, or all palettes with the chosen one
   moved to position 0?
   **Answered for M11 (ours):** every palette is written, the chosen one moved to position 0.
   Neither oracle reads the parameter (guess on `color.export.colorPaletteForCpal`).
4. Is CPAL version 0 always written? Are palette labels or light/dark palette type flags ever set?
   **Answered for M11 (oracle):** both oracles write CPAL version 0 with no labels or palette types;
   FontCraft does the same. fontc also writes CPAL when no colour layer uses it, fontmake does not;
   FontCraft follows fontmake.
5. When a colour row is removed in the palette editor, are layers using that or higher indices
   renumbered, left dangling, or flagged?
   **Answered for M11 (ours):** layers that use higher indices are renumbered down; removing a row
   that a layer uses is refused with a message naming the glyphs (guess on
   `colorPalette.removeColor`).
6. Initial colour of a new palette entry and initial contents of a new palette; can the last
   palette or the last colour be removed?
   **Answered for M11 (ours):** a new row is opaque black; a new palette copies palette 0; the last
   palette cannot be removed; the last colour can be removed when no layer uses it, which leaves no
   CPAL (guesses on the palette commands).
7. Which palette does the Layers palette chooser and the Edit View preview use when several palettes
   exist: always palette 0?
   **Answered for M11 (ours):** always palette 0 (guess on `palette.layers.paletteColorChooser`).
8. Does the reference app itself export full-colour layers as COLR v1, under which conditions, and
   is there a switch for it? The chapter mentions only SVG; the mapping here follows fontc.
   **Answered for M11 (oracle, to observe):** both oracles compile full-colour layers to COLR. fontc
   writes version 0 when every fill is solid and version 1 otherwise, and reads only colour
   attributes on the master layer itself; fontmake writes version 1 for every full-colour glyph and
   reads separate Color layers. FontCraft reads separate Color layers and chooses the version as
   fontc does; no switch (guesses on `color.export.colrVersionChoice`,
   `color.export.fullColorToColrV1`).
9. `Export SVG Table`: the chapter asks for it to be checked for conversions, the appendix says SVG
   is exported by default. Is an SVG table written for svg layers without any parameter?
   **Answered for M11 (appendix):** an SVG table is written whenever svg layers exist or a
   conversion asks for it; Export SVG Table set off removes it (guess on
   `color.export.exportSvgTable`).
10. Are SVG documents stored compressed, one document per glyph or grouped, and how is the placed
    image's transform expressed (viewBox, wrapper group)?
    **Answered for M11 (ours, nanoemoji's layout):** one uncompressed document per glyph, the
    glyph's group with the id `glyph<ID>` and a transform that flips y at the baseline and applies
    the placed image's transform. nanoemoji writes the same layout in its `untouchedsvg` mode;
    neither oracle writes an SVG table from a Glyphs source.
11. How is an image attached to an iColor layer: drag-and-drop, Glyph › Add Image, or automatic
    lookup by file name? What is the default size of a new iColor layer?
    **Answered for M11 (ours):** Add Image with the iColor layer as target, a drop on the layer, or
    `layer.setIColorImage`; a new iColor layer is 128 pixels per em (guesses on
    `layer.convertToIColor`, `layer.setIColorImage`).
12. Exact fitting rule of an image into the master-layer bounding box (scale to height, to width, or
    both), and what happens when the master layer has no paths.
    **Answered for M11 (handbook, ours):** the iColor size alone sets the scale (unitsPerEm ÷ size
    units per pixel); the image's bottom-left corner sits at the bottom-left of the master layer's
    path bounds, or at the origin when the master layer has no paths.
13. Are JPEG and TIFF sources stored as such in sbix or converted to PNG? What graphic type tags are
    written?
    **Answered for M11 (ours):** JPEG is stored as `jpg `, TIFF converted to PNG (guess on
    `color.sbix.bitmapFormats`).
14. What does the reference do when the image sizes of a glyph have different aspect ratios, or when
    a referenced file is missing at export?
    **Answered for M11 (ours):** unequal ratios export with a warning; a missing file leaves the
    glyph out of that strike with a warning (guesses on `color.sbix.sameAspectRatio`,
    `color.export.sbixAutomatic`).
15. Stroke alignment: which side is "left" relative to path direction, and how are strokes written
    to SVG (native stroke or expanded outline)?
    **Answered for M11 (format, ours):** the file format stores inside and outside (`strokePos`:
    version 3 0 outside, 0.5 centred, 1 inside; version 4 −1 inside, 0 centred, 1 outside), so left
    is taken as inside. Centred strokes are SVG strokes; others are expanded. fontc refuses stroked
    colour paths; fontmake expands centred ones only.
16. How is a gradient removed from a path, and can stops be moved along the bar?
    **Answered for M11 (ours):** `attributes.removeGradient` (a new row); stops move by offset
    (guesses on both rows).
17. Defaults for a new gradient (type, stop colours, start and end points), and whether the
    file-format-only features (conic gradient, extend mode, radii) have any UI.
    **Answered for M11 (format, ours):** linear, from the fill colour to the same colour
    transparent, left middle to right middle of the bounds; conic gradients and extend modes are
    accepted headless only (guesses on `attributes.addGradient`).
18. Default values of the gray fill and of a new stroke colour.
    **Answered for M11 (ours):** 50 % gray, opaque; a new stroke is black (guesses on
    `color.fullColor.defaultGrayFill`, `attributes.setStrokeWidth`).
19. Do Shadow, Inner Shadow and Pattern Image have further controls? The chapter lists only their
    names.
    **Answered for M11 (format):** Shadow and Inner Shadow store blur, x and y offset and colour
    (`shadow`, `shadowIn`); FontCraft stores them and never exports them. Pattern Image's fields are
    unknown (guess on `attributes.addAttribute`).
20. Can a glyph mix colour layer types (e.g. palette layers and an svg layer), and which tables are
    then written?
    **Answered for M11 (ours):** each kind feeds its own table; an svg layer wins over a conversion
    for its glyph (guess on `color.fourKinds`).
21. Do colour layers interpolate in variable fonts (variable COLR), or are they exported from one
    master only? The chapter is silent.
    **Answered for M11 (oracle, to observe):** fontmake writes variable COLR for fontc's COLRv1-var
    source (PaintVarLinearGradient, an ItemVariationStore and a DeltaSetIndexMap); fontc writes it
    static. Palette-layer glyphs vary through their own outline variations in both. FontCraft
    follows fontmake in M11.19, after M9 (`color.export.variableColr`).
22. In a layered-to-SVG or layered-to-COLR conversion, which master supplies the fallback outline,
    and is the master order (first on top) the paint order?
    **Answered for M11 (ours):** the instance's own master supplies the fallback outline; the first
    master is on top, as in Edit View (guesses on `color.layered.toColr`, `color.layered.toSvg`).
