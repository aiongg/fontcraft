# Custom parameters

Reference for the 205 custom parameters listed in the reference handbook's appendix. Inventory rows:
`spec/inventory/18-custom-parameters.toml` (`[[parameter]]`, keyed by `name`). Names are file-format
keys and are spelled exactly as in the reference; FontCraft must read and write them unchanged.

## Conventions

- Parameters live in the custom-parameter lists of the Font, Masters and Exports tabs of Font Info.
  Each has a checkbox; unchecked parameters are kept but disabled and shown grey
  (`customParameters.enableCheckbox`).
- camelCase names are UFO 3 font-info keys; Title Case names are application-specific and mostly act
  on the font at export (`customParameters.nameCaseConvention`).
- UFO-derived names are accepted with or without their keyword prefix: `description` and
  `openTypeNameDescription`, `blueScale` and `postscriptBlueScale`
  (`customParameters.shortAndLongNames`).
- Scope column: F font, M master, I instance, as stated by the appendix entry itself. `?` means the
  entry does not say; settle it from the Font Info chapter or by observation.
- Table field column: stated by the source unless marked otherwise. `-` means none or not stated.
- Type words used by the source map to row kinds as boolean = bool, integer and float = number.


## Table values (`effect = "table-value"`, 39)

Each parameter overrides one field (or a small set of fields) of an output table.

### OS/2

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `codePageRanges` | ? | Code pages the font declares as supported. | OS/2.ulCodePageRange1, OS/2.ulCodePageRange2 |
| `fsType` | ? | Embedding permission bits. | OS/2.fsType |
| `Has WWS Names` | ? | Declares the names already follow the weight/width/slope model. | OS/2.fsSelection bit 8 |
| `panose` | ? | The ten PANOSE classification numbers. | OS/2.panose |
| `strikeoutPosition` | ? | Where the upper edge of the strikeout line sits, measured from the baseline. | OS/2.yStrikeoutPosition |
| `strikeoutSize` | ? | Thickness of the strikeout stroke. | OS/2.yStrikeoutSize |
| `subscriptXOffset` | ? | Horizontal offset for synthesised subscripts. | OS/2.ySubscriptXOffset |
| `subscriptXSize` | ? | Horizontal size for synthesised subscripts. | OS/2.ySubscriptXSize |
| `subscriptYOffset` | ? | Vertical offset for synthesised subscripts. | OS/2.ySubscriptYOffset |
| `subscriptYSize` | ? | Vertical size for synthesised subscripts. | OS/2.ySubscriptYSize |
| `superscriptXOffset` | ? | Horizontal offset for synthesised superscripts. | OS/2.ySuperscriptXOffset |
| `superscriptXSize` | ? | Horizontal size for synthesised superscripts. | OS/2.ySuperscriptXSize |
| `superscriptYOffset` | ? | Vertical offset for synthesised superscripts. | OS/2.ySuperscriptYOffset |
| `superscriptYSize` | ? | Vertical size for synthesised superscripts. | OS/2.ySuperscriptYSize |
| `typoAscender` | ? | Typographic ascender. | OS/2.sTypoAscender |
| `typoDescender` | ? | Typographic descender (negative). | OS/2.sTypoDescender |
| `typoLineGap` | ? | Typographic line gap. | OS/2.sTypoLineGap |
| `unicodeRanges` | ? | Unicode ranges the font declares as supported. | OS/2.ulUnicodeRange1-4 |
| `Use Typo Metrics` | ? | Tells applications to use the typo line metrics. | OS/2.fsSelection bit 7 |
| `vendorID` | ? | Four-character vendor identifier. | OS/2.achVendID |
| `winAscent` | ? | Top of the Windows clipping box. | OS/2.usWinAscent |
| `winDescent` | ? | Bottom of the Windows clipping box. | OS/2.usWinDescent |

### hhea

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `hheaAscender` | ? | Ascender in the horizontal header table. | hhea.ascender |
| `hheaDescender` | ? | Descender (negative) in the horizontal header table. | hhea.descender |
| `hheaLineGap` | ? | Line gap in the horizontal header table. | hhea.lineGap |

### vhea

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `vheaVertAscender` | ? | Ascender for vertical typesetting. | vhea.vertTypoAscender |
| `vheaVertDescender` | ? | Descender for vertical typesetting. | vhea.vertTypoDescender |
| `vheaVertLineGap` | ? | Line gap for vertical typesetting. | vhea.vertTypoLineGap |

### post

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `isFixedPitch` | ? | Marks the font as monospaced. | post.isFixedPitch |
| `Post Table Type` | ? | Version of the post table. | post.version |
| `underlinePosition` | ? | How far the upper edge of the underline lies from the baseline. | post.underlinePosition |
| `underlineThickness` | ? | Underline thickness. | post.underlineThickness |

### post / CFF / hhea

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `italicAngle` | ? | Italic angle override, for the whole font or one script. | post.italicAngle, CFF ItalicAngle, hhea.caretSlopeRise/Run, OS/2 sub/superscript x offsets |

### head

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `unitsPerEm` | ? | Units per em, without scaling the outlines. | head.unitsPerEm |

### CFF

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `ROS` | ? | Character collection of a CID-keyed font. | CFF ROS (Registry-Ordering-Supplement) |

### meta

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `meta Table` | ? | Design and supported language/script tags. | meta dlng, meta slng |

### CPAL / SVG

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Color Palette for CPAL` | ? | Index of the palette written to CPAL. | CPAL |
| `Color Palette for SVG` | ? | Index of the palette used to colour the SVG table. | SVG |
| `Color Palettes` | ? | The font's colour palettes; also drives colour preview in the editor. | CPAL |

## Naming (`effect = "naming"`, 34)

Parameters that set name-table strings or decide which name records exist.

### name table records

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `compatibleFullNames` | ? | Mac-only compatible full name. | name ID 18 |
| `copyrights` | ? | Copyright notice, replacing the Font tab value. | name ID 0 |
| `descriptions` | ? | Free-text description of the typeface. | name ID 10 |
| `designers` | I | Designer name for this instance, replacing the Font tab value. | name ID 9 |
| `designerURL` | I | Designer URL for this instance, replacing the Font tab value. | name ID 12 |
| `familyNames` | ? | Family name, replacing the Font tab value. | name IDs 1 and 4 (feeds 3, 4, 6) |
| `licenses` | ? | Plain-language licence description. | name ID 13 |
| `licenseURL` | ? | URL with licence information. | name ID 14 |
| `manufacturers` | ? | Manufacturer name, replacing the Font tab value. | name ID 8 |
| `manufacturerURL` | ? | Manufacturer or vendor URL, replacing the Font tab value. | name ID 11 |
| `postscriptFontName` | ? | PostScript font name. | name ID 6 |
| `postscriptFullNames` | ? | Full font name shown to users. | name ID 4, CFF FullName |
| `preferredFamilyNames` | ? | Typographic family name. | name ID 16 |
| `preferredSubfamilyNames` | ? | Typographic subfamily name. | name ID 17 |
| `sampleTexts` | ? | Sample text shown by font managers. | name ID 19 |
| `styleMapFamilyNames` | ? | Family name of the four-style linking group. | name (ID not stated) |
| `styleMapStyleNames` | ? | Localised subfamily name. | name ID 2 |
| `styleNames` | ? | Style (subfamily) name. | name ID 2 |
| `trademarks` | ? | Trademark notice. | name ID 7 |
| `uniqueID` | ? | Unique font identifier. | name ID 3 |
| `versionString` | ? | Version string template. | name ID 5 (ID not named in source) |
| `WWSFamilyName` | ? | Weight/width/slope family name. | name ID 21 |
| `WWSSubfamilyName` | ? | Weight/width/slope subfamily name. | name ID 22 |

### name table structure

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Compatible name Table` | ? | Writes a legacy-style name table for certain old Mac applications. | name |
| `Export Mac Name Table Entries` | ? | Allows or prevents writing Macintosh-platform name records. | name (platform 1) |
| `Name Table Entry` | ? | Adds one arbitrary name record. | name (any ID except 1, 2, 3, 5, 6) |
| `Prevent Name ID` | ? | Leaves one name ID out of the export. | name |

### Variable-font names and file

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Variable Font Family Name` | ? | Family name used in the variable font export. | name (family) |
| `Variable Font Style Name` | ? | Style name used in the variable font export (source text says family name). | name |
| `Variable SubfamilyName` | ? | Style name of the variable font's origin master. | name (subfamily) |
| `variablePostscriptFontName` | ? | PostScript name of an instance inside a variable font. | name ID 6, fvar instance postScriptNameID |
| `variableStyleName` | ? | Style name for variable fonts (source text says family name). | name |
| `variableStyleNames` | ? | Instance style name used only in variable fonts. | fvar instance names |
| `variationsPostScriptNamePrefix` | ? | Prefix for generated PostScript names of variable instances. | name ID 25 |

## Export filters (`effect = "export-filter"`, 18)

Operations run on the outlines or the glyph set while exporting; the source is not changed.

### Glyph set and encoding

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Don't use Production Names` | ? | Exports the working glyph names instead of production names. | post / CFF glyph names |
| `Export Glyphs` | ? | Glyphs to export even when marked as non-exporting. | - |
| `Keep Glyphs` | ? | Exports only the listed glyphs. | - |
| `Reencode Glyphs` | ? | Reassigns or removes Unicode values at export. | cmap |
| `Remove Glyphs` | ? | Glyphs to leave out of the export, by name, wildcard or property query. | - |
| `Rename Glyphs` | ? | Swaps pairs of glyphs at export. | - |
| `Use Arabic Presentation Form Unicodes` | ? | Encodes Arabic glyphs with legacy presentation-form code points. | cmap |

### Component decomposition

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Decompose Brace Layers` | ? | In variable exports, decomposes components that point at glyphs with intermediate layers. | - |
| `Decompose Components` | ? | Turns every component into outlines at export. | glyf |
| `Decompose Components in Variable Font` | ? | Turns every component into outlines in a TrueType variable font. | glyf |
| `Decompose Glyphs` | ? | Decomposes the listed composite glyphs at export. | - |
| `Keep Overlapping Components` | ? | Keeps composites whose components overlap from being decomposed. | glyf |
| `Keep Transformed Components` | ? | Keeps composites with scaled or vertically shifted components from being decomposed. | glyf |

### Outline operations

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Filter` | I | Runs a named filter on an instance after components are decomposed. | - |
| `Grid Spacing` | ? | Coordinate rounding precision for CFF output. | CFF |
| `PreFilter` | ? | Like Filter, but runs before components are decomposed. | - |
| `Scale to UPM` | ? | Scales the whole font to a new units-per-em value. | head.unitsPerEm |
| `TrueType Curve Error` | ? | Tolerance for converting cubic curves to quadratic. | glyf |

## Features (`effect = "features"`, 17)

Parameters that change the feature code or how layout tables are generated.

### Feature code edits

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Add Class` | ? | Adds a glyph class to the exported feature code. | - |
| `Add Feature` | ? | Appends a feature to the exported feature code. | - |
| `Add Prefix` | ? | Adds a named prefix block to the exported feature code. | - |
| `Remove Classes` | ? | Classes to leave out of the exported feature code. | - |
| `Remove Features` | ? | Features to leave out of the exported font. | - |
| `Remove Prefixes` | ? | Feature prefixes to leave out of the export. | - |
| `Replace Class` | ? | Replaces the contents of an existing class. | - |
| `Replace Feature` | ? | Replaces the code of an existing feature. | - |
| `Replace Prefix` | ? | Replaces the code of an existing prefix. | - |

### Automatic feature generation

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Keep UI-Font Bounding Box` | ? | Stops South Asian conjuncts from stacking below the line. | GSUB cjct |
| `Optical Size` | ? | Builds the size feature from five values. | GPOS size feature |
| `Update Features` | ? | Forces or suppresses regeneration of automatic feature code. | - |

### Kerning and mark output

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Keep Kerning in one Lookup` | ? | Tries to put all pair kerning in the first GPOS lookup. | GPOS |
| `Propagate Anchors` | ? | Lets composites inherit top and bottom anchors from their base glyphs. | GPOS mark, mkmk |
| `Use Extension Kerning` | ? | Puts kerning in extension lookups to avoid offset overflow. | GPOS lookup type 9 |

### Layout compilation

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `MakeOTF Compatibility Mode` | ? | Builds layout tables the way Adobe's makeotf would. | OpenType layout tables |

### Variable-font layout

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Feature for Feature Variations` | ? | Feature tag that carries bracket-layer substitutions in variable fonts. | GSUB feature variations |

## Interpolation and variable fonts (`effect = "interpolation"`, 15)

### Axes and design space

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Axis Location` | M, I | Maps a master's or instance's internal axis position to user-facing coordinates. | - |
| `Axis Mappings` | ? | Per-axis internal-to-external coordinate map; produces the avar table. | avar |

### Masters

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Disable Masters` | ? | Masters to leave out of interpolation. | - |
| `Enforce Compatibility Check` | ? | Checks outline compatibility between all masters, not only those sharing an instance. | - |
| `Variable Font Origin` | ? | Master whose outlines become the variable font's default. | default outlines (glyf / CFF2) |
| `Variation Font Origin` | ? | Older spelling of Variable Font Origin. | default outlines (glyf / CFF2) |
| `Virtual Master` | ? | Adds a design-space master that exists only as intermediate layers. | - |

### Instances

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Ignore out of bounds instances` | ? | Skips instances that lie outside the design space. | - |
| `InterpolationWeightY` | I | Separate weight value for interpolating y coordinates. | - |
| `Local Interpolation` | ? | Different interpolation position for selected glyphs. | - |

### STAT

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Elidable STAT Axis Value Name` | I | Marks this instance's style name as elidable for one axis. | STAT axis value flags |
| `Italic Style Linking` | ? | Forces or suppresses the regular-to-italic link in STAT. | STAT format 3 axis value (ital) |
| `Style Name as STAT entry` | I | Uses this instance's style name as the STAT name for one axis. | STAT axis value names |

### Variation data

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Optimize Variable Deltas` | ? | Drops variation deltas of contours that barely move. | gvar |
| `Variable Font Optimize Deltas` | ? | Drops variation deltas of contours that barely move. | gvar |

## Hinting (`effect = "hinting"`, 25)

All parameters of the two hinting areas, including those that write a table field.

### PostScript hinting values

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `blueFuzz` | ? | Units by which alignment zones are widened in both directions. | CFF Private BlueFuzz |
| `blueScale` | ? | Size limit below which overshoots are suppressed. | CFF Private BlueScale |
| `blueShift` | ? | Overshoot depth from which overshoots show above the blueScale size. | CFF Private BlueShift |
| `Family Alignment Zones` | ? | Family-wide alignment zones shared by all weights. | - |

### Autohinting

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Autohint` | I | Forces autohinting for this instance whatever the export dialog says. | - |
| `Disable autohinting for glyphs` | ? | Glyphs skipped by the PostScript autohinter. | - |

### Hint export switches

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Export PostScript Hints` | ? | Allows or prevents writing PostScript hints. | CFF hints |
| `Export TrueType Hints` | ? | Allows or prevents writing TrueType hints. | TrueType instructions |

### Manual hinting source

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Get Hints From Master` | ? | Names the master whose manual hints are used. | - |

### TrueType hinting tables

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `gasp Table` | ? | Two PPM thresholds defining three rendering-behaviour ranges. | gasp |

### Preserved TrueType programs

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `cvt Table` | ? | Stored control values from an imported TTF, re-exported unchanged. | cvt |
| `fpgm Table Assembly` | ? | Stored font program from an imported TTF, re-exported unchanged. | fpgm |
| `prep Table Assembly` | ? | Stored control value program from an imported TTF, re-exported unchanged. | prep |

### Manual TrueType hinting

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Ignore Vertical Hints` | ? | Drops manual TrueType stem hints tied to vertical stems. | - |
| `TTFBlueFuzz` | ? | Widens TrueType zones in both directions. | - |
| `TTFDontPreserveDiagonals` | ? | Stops manual TrueType hinting from protecting diagonal stem angles. | - |
| `TTFFamilyZonesThreshold` | ? | PPM size up to which family alignment zones apply. | - |
| `TTFMinimumDistance` | ? | Smallest rendered thickness of a hinted stem, in pixels. | - |
| `TTFOvershootSuppressionBelowPPM` | ? | PPM size up to which overshoots are flattened. | - |
| `TTFStems` | M | Stem definitions for manual TrueType hinting. | - |
| `TTFZoneRoundingThreshold` | ? | Bias pushing positive zones up to the next pixel. | - |
| `TTFZones` | F, M | Zone definitions for manual TrueType hinting. | - |

### TrueType autohinter

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `TTFAutohint binary path` | ? | Path to an external ttfautohint executable. | - |
| `TTFAutohint control instructions` | ? | Control instructions passed to the TrueType autohinter. | - |
| `TTFAutohint options` | ? | Command-line options for the TrueType autohinter. | - |

## Editing and display only (`effect = "ui"`, 24)

Nothing here reaches the exported font.

### Per-script metrics

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `ascender` | ? | Per-script ascender used for glyphs of that script while editing. | - |
| `capHeight` | ? | Per-script cap height used for glyphs of that script while editing. | - |
| `descender` | ? | Per-script descender used for glyphs of that script while editing. | - |
| `shoulderHeight` | ? | Shoulder-height metric line for Middle Eastern and South/Southeast Asian scripts. | - |
| `smallCapHeight` | ? | Small-cap height metric line. | - |

### Editing grids and guides

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `CJK Grid` | ? | Row and column count of the dotted grid shown in CJK glyphs. | - |
| `CJK Grid Horizontal` | ? | Column count of the dotted CJK editing grid. | - |
| `CJK Grid Vertical` | ? | Row count of the dotted CJK editing grid. | - |
| `CJK Guide` | ? | Inset percentage of a second square guide (virtual body) in CJK glyphs. | - |
| `Design Grid` | ? | Dotted editing grid given as columns and rows, optionally per script. | - |

### Edit View display

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `EditView Line Height` | ? | Line spacing of text in an Edit tab. | - |
| `Preview Ascender` | M | Top edge of the preview area, in units above the baseline. | - |
| `Preview Descender` | M | Bottom edge of the preview area, in units from the baseline. | - |

### Master display

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Master Background Color` | M | Canvas colour while this master is active. | - |
| `Master Background Color Dark` | M | Canvas colour for this master in dark mode. | - |
| `Master Color` | M | Fill colour used to draw this master's outlines. | - |
| `Master Color Dark` | M | Fill colour for this master in dark mode. | - |
| `Master Icon Glyph Name` | M | Glyph drawn on this master's button in the font window. | - |
| `Master Stroke Color` | M | Outline colour for this master with the Select All Layers tool. | - |
| `Master Stroke Color Dark` | M | Outline colour for this master in dark mode. | - |

### Editing defaults

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Default Layer Width` | ? | Advance width given to newly created glyphs and layers. | - |
| `Fit Curve Panel Settings` | ? | Remembers the minimum and maximum percentages of the Fit Curve panel. | - |
| `Hangul Composition Groups` | ? | Jamo grouping used to auto-compose Hangul syllables. | - |

### Font Info display

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Instance Preview` | I | Glyphs shown in the instance's preview string in Font Info. | - |

## Other export and source-file behaviour (`effect = "other"`, 33)

### Table on/off switches

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Export AAT` | ? | Allows or prevents compiling the morx prefix into the font. | morx |
| `Export COLR Table` | ? | Allows or prevents writing COLR and CPAL. | COLR, CPAL |
| `Export kern Table` | ? | Also writes a legacy kern table in TrueType exports. | kern |
| `Export morx Table` | ? | Compiles the prefix named morx into a morx table. | morx |
| `Export OpenType` | ? | Allows or prevents writing most OpenType tables. | OpenType layout tables |
| `Export sbix Table` | ? | Allows or prevents writing sbix. | sbix |
| `Export STAT Table` | ? | Allows or prevents writing STAT. | STAT |
| `Export SVG Table` | ? | Allows or prevents writing the SVG table. | SVG |
| `Export vmtx Table` | ? | Forces or suppresses the vertical metrics table. | vmtx |

### Colour table switches

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Color Layers to COLR` | ? | Builds COLR and CPAL from the colour setup. | COLR, CPAL |
| `Color Layers to SVG` | ? | Builds an SVG table from palette colour layers or coloured masters. | SVG |
| `SBIX to SVG` | ? | Copies sbix bitmaps into the SVG table. | SVG |

### Export behaviour switches

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Disable Subroutines` | ? | Turns off CFF subroutinisation. | CFF |
| `DisableAllAutomaticBehaviour` | ? | Turns off most automatic adjustments made at export. | - |
| `Remove post names for webfonts` | ? | Strips glyph names from web font exports. | post |
| `Webfont Only` | ? | Strips data needed for desktop installation. | - |

### Export format and destination

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Export Folder` | ? | Subfolder of the export destination to write into. | - |
| `fileName` | ? | Base file name of the exported font. | - |
| `Point To Unit Ratio` | ? | Font units per DTP point for PDF and clipboard vector output. | - |
| `Save as TrueType` | I | Exports this instance with TrueType outlines whatever the export dialog says. | - |
| `Variable Font File Name` | ? | File name of the variable font export. | - |
| `Webfont Formats` | I | Web font formats exported for this instance whatever the export dialog says. | - |

### Glyph order

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `glyphOrder` | ? | Glyph order for both the source and the exported font. | glyph order |
| `TrueType Keep GlyphOrder` | ? | Keeps the source glyph order in TrueType exports. | glyph order |

### Linked metrics

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Link Metrics With First Master` | M | Keeps this master's sidebearings and kerning identical to the first master's. | - |
| `Link Metrics With Master` | M | Keeps this master's sidebearings and kerning identical to a named master's. | - |

### Linked sources

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Import Font` | ? | Links in the glyphs of another source file, read-only. | - |
| `Import Master` | ? | Links in one master from another source file. | - |

### Source file behaviour

| Name | Scope | What it does | Table field |
|---|---|---|---|
| `Ignore custom GlyphData file` | ? | Ignores a GlyphData.xml stored beside the source file. | - |
| `note` | ? | Free-form font note, stored only in the source. | - |
| `Use Line Breaks` | ? | Stores feature code with real line breaks in the source file. | - |
| `Write DisplayStrings` | ? | Allows or prevents saving Edit tab text in the source file. | - |
| `Write lastChange` | ? | Allows or prevents saving the last-changed date in the source file. | - |

## Value syntaxes

Syntaxes too long for the inventory summaries.

### `Filter` and `PreFilter`

Value: filter name, then arguments separated by semicolons. `Filter` runs after components are
decomposed; `PreFilter` runs before and only in static fonts. Built-in filters and their arguments:

| Filter | Arguments |
|---|---|
| `AddExtremes` | none |
| `Autohinting` | none |
| `HatchOutlineFilter` | `OriginX: x; OriginY: y; StepWidth: distance; Angle: angle; Offset: thickness` |
| `OffsetCurve` | `x; y; makestroke; position` where `position` is 0.0 to 1.0 (0 % to 100 %) or `auto` |
| `RemoveOverlap` | none |
| `Roughenizer` | `segmentlength; x; y; angle` |
| `RoundCorner` | `radius; visualcorrection` |
| `RoundedFont` | `stem` (optional) |
| `Transformations` | any of `LSB: ±shift; RSB: ±shift; Width: ±shift; ScaleX: percent; ScaleY: percent; Slant: degrees; SlantCorrection: bool; OffsetX: amount; OffsetY: amount; Origin: select` |

- Boolean arguments (`makestroke`, `visualcorrection`, `SlantCorrection`) are `1` or `0`.
- `Origin` is 0 cap height, 1 half cap height, 2 x-height, 3 half x-height, 4 baseline.
- A trailing `include:` or `exclude:` argument with space- or comma-separated glyph names limits the
  filter to, or keeps it away from, those glyphs: `RemoveOverlap; exclude:x,y,z`.
- Third-party filters define their own argument strings and may not support `include`/`exclude`.

### `Remove Glyphs` and `Keep Glyphs`

One entry per line. An entry is a full glyph name, a name pattern with `*` anywhere (`*ogonek`,
`K*`, `H*.ss01`), or a query `key=value`; string values in queries accept `*` too.

| Key | Matches on |
|---|---|
| `name` | glyph name (same as a bare name pattern) |
| `unicode` | first code point, as hex text: `unicode=1E*` matches 1E00 to 1EFF |
| `note` | glyph note: `note=*draft*` |
| `script` | script as shown in the Glyph Info window, case-sensitive: `script=greek` |
| `category`, `subCategory` | category and subcategory as shown in Glyph Info: `category=Mark`, `subCategory=Nonspacing` |
| `production` | production name |
| `leftMetricsKey`, `rightMetricsKey`, `widthMetricsKey`, `topMetricsKey`, `bottomMetricsKey`, `vertWidthMetricsKey` | the metrics key: `widthMetricsKey=*.tosf*` |
| `leftKerningGroup`, `rightKerningGroup`, `topKerningGroup`, `bottomKerningGroup` | kerning group name |
| `colorIndex` | palette index of colour layers; removes the shapes of that colour |
| `countOfLayers` | number of layers including master layers |
| `mastersCompatible` | `0` or `1`: interpolatable across all masters |
| `export` | `0` or `1`; `export=0` also drops components whose base glyph does not export, plus corner, cap and smart glyphs |
| `isAnyColorGlyph` | `0` or `1`: has COLR/CPAL, sbix or full-colour data |
| `isAppleColorGlyph` | `0` or `1`: has a bitmap colour layer for sbix |
| `hasSpecialLayers` | `0` or `1`: has an alternate (bracket) or intermediate (brace) layer |

Automatic feature code is regenerated for the reduced glyph set; `Keep Glyphs` also prunes kerning.
The two parameters are alternatives, not to be combined.

### `Reencode Glyphs`, `Rename Glyphs`

- `Reencode Glyphs`: `glyph=HEX[,HEX...]`; `glyph=` clears the glyph's code points. A code point
  claimed here is taken away from whichever glyph had it.
- `Rename Glyphs`: `old=new`; the two glyphs swap names and export flags, and composites and
  automatic features follow.

### `Name Table Entry`

`nameID; text`, `nameID platformID; text` or `nameID platformID encID langID; text`.
`platformID` is 1 (Macintosh) or 3 (Windows). Defaults: platform 3, encoding 1, Windows English;
with only platform 1 given, encoding 0 and language 0. `encID` and `langID` are numbers from 0 to
65536 in decimal (first digit not 0), octal (leading 0) or hexadecimal (`0x`). Name IDs 1, 2, 3, 5
and 6 are refused.

### `Optical Size`

`designSize; subfamilyID; rangeStart; rangeEnd; menuName`, sizes in decipoints. The range excludes
its start and includes its end. `180; 2; 139; 240; Eighteen`: designed for 18 pt, used above
13.9 pt up to 24 pt, menu name Eighteen, grouped with other fonts that use subfamily id 2.

### `Local Interpolation`

`value[; value...]; include: glyph, glyph` with one value per axis.

### Per-script values

`ascender`, `capHeight`, `descender`, `italicAngle`, `Default Layer Width`, `CJK Guide`,
`CJK Grid*` and `Design Grid` accept a lowercase script name and a colon before the value
(`greek: 705`); the parameter is repeated for each script.

### `meta Table`

Two entries, `dlng` and `slng`, each a comma-separated list of tags of the form
`[language-]Script[-REGION]` (`Cyrl`, `sr-Cyrl`, `sr-Cyrl-BA`). `slng` should contain everything in
`dlng`.

### `gasp Table`

Two PPM thresholds, default 8 and 20, giving three ranges: up to the first, no grid-fitting with
symmetric smoothing; between them, grid-fitting with asymmetric (ClearType) rendering and no
greyscale; above the second, grid-fitting with symmetric smoothing.

### `postscriptFontName` style abbreviations

When the name must be shortened: Bd Bold, Bk Book, Blk Black, Cm Compressed, Cn Condensed,
Ct Compact, Dm Demi, Ds Display, Ex Extended, Hv Heavy, Ic Inclined, It Italic, Ks Kursiv, Lt Light,
Md Medium, Nd Nord, Nr Narrow, Obl Oblique, Po Poster, Rg Regular, Sl Slanted, Sm Semi, Su Super,
Th Thin, Ult Ultra, Up Upright, X Extra. Dm, Sm, Ult and X are prefixes. Abbreviate consistently
across the family.

### `TTFAutohint control instructions`

One instruction per line; `#` starts a comment; instruction words may be shortened to their first
letter.

```
glyphnames left pointIDs [offset]
glyphnames right pointIDs [offset]
glyphnames nodir pointIDs
glyphnames touch pointIDs [xshift x] [yshift y] @ PPMs
glyphnames point pointIDs [xshift x] [yshift y] @ PPMs
```

`glyphnames` is one or more comma-separated production names. `offset` defaults to 0. Shift
amounts are between 0.0 and 1.0. `PPMs` is a size, a range `a-b`, or a comma-separated mix.

### `TTFAutohint options`

Edited in a dialog and stored as an option string for ttfautohint.

| Option | Meaning and default |
|---|---|
| Hint Set Range | PPM range the instructions are optimised for; wide ranges inflate the file |
| Default Script | the script that OpenType features use by default |
| Fallback Script | script for glyphs that cannot be assigned one automatically |
| Hinting Limit | PPM above which hinting stops; default 200; must exceed the hint set range maximum |
| Fallback Stem Width | positive integer in units for scripts without standard characters; ttfautohint's own default is 50 units at 2048 UPM, scaled linearly |
| x-Height Increase Limit | from this PPM down to 6 the x-height tends to round up; default 14; 0 turns it off |
| x-Height Snapping Exceptions | PPM values and ranges without x-height snapping (`9, 12-15, 18`); empty = none; `-` = all sizes |
| Adjust Subglyphs | run the font's existing bytecode on all glyphs before replacing it |
| Dehint | remove all TrueType hinting; overrides everything else |
| Detailed Info | add ttfautohint version and command line to name ID 5; exclusive with No Autohint Info; with neither, a short `ttfautohint (vNNN)` note is added |
| Hint Composites | hint the composite as a whole instead of its components; adds a `.ttfautohint` helper component to every glyph |
| Ignore Restrictions | process fonts whose OS/2.fsType bit 1 is set |
| No Autohint Info | add nothing to name ID 5 |
| Symbol Font | process fonts with no standard characters for any script, using default stem widths |
| ttfa table | add a TTFA table holding all parameters and control instructions |
| Windows Compatibility | add two artificial zones at winAscent and winDescent to limit clipping |
| Strong Stems | per rendering target (Grayscale, GDI ClearType, DW ClearType): snap stems and zones to whole pixels when on, quantise them lightly when off |

### `TTFStems` and `TTFZones`

- Stem: orientation (horizontal or vertical), name (unique, shown in the TT Instructor info box),
  width (the size stems round towards, also used to recognise stems when autohinting from that
  tool).
- Zone: name, position (the flat edge), size (positive for top zones, negative for bottom zones;
  below half a pixel, shapes reaching into the zone are flattened), align (another zone to which
  the distance is rounded: equal height when under half a pixel apart, otherwise at least one
  pixel apart).
- Both, in the hint-source master only (first master, or the one named by `Get Hints From
  Master`): deltas per PPM and instance, cycling none, up, down; and a scope, a tree of Name,
  Category, Subcategory and Script filters joined by AND/OR, restricting the stem or zone to
  matching glyphs.
- Stems and zones can be imported from the master's PostScript stems and alignment zones.
- Related numbers: `TTFBlueFuzz` (default 1), `TTFMinimumDistance` (default 0.25 px),
  `TTFZoneRoundingThreshold` (default 0.09375, counted twice for the zone named `xHeight`),
  `TTFOvershootSuppressionBelowPPM`, `TTFFamilyZonesThreshold`.

### PostScript blue values

- `blueScale` = (point size at 300 dpi - 0.49) / 240; default 0.037 (overshoots from 40 PPM).
  Upper limit from the tallest zone z: maximum PPM = 2.04 + 1000 / z, maximum point size =
  0.49 + 240 / z. For z = 21: PPM 49, value at most 0.04762.
- `blueShift` default 7; `blueFuzz` default 1, zones at least 2 x blueFuzz + 1 units apart.

## Oracle support of the M7 parameters

Checked by the M7 planning pass with fontmake 3.12.1 (glyphsLib 6.15.0, ufo2ft 3.9.1) and fontc
1.0.0 on synthetic sources (`docs/plans/M7.md` § 2 has the full table). In short:

- fontmake applies font parameters to every master, master parameters over them, and instance
  parameters and properties to the static instance it builds. fontc builds no static instance:
  instance values reach only `fvar`, and of master and instance parameters it reads only
  metrics, panose and meta.
- Read by fontmake on instances (and masters): the name properties and their format 2
  parameters, Name Table Entry (added up over the levels), Remove Glyphs and Keep Glyphs (exact
  names only; it subsets after compiling), Rename Glyphs, Reencode Glyphs, Filter and PreFilter
  (ufo2ft filter names only).
- Read by neither: Export Glyphs, Prevent Name ID, Export Mac Name Table Entries, Compatible name
  Table, Scale to UPM, Export kern Table, ROS, Import Font, Import Master, Point To Unit Ratio,
  Instance Preview, Use Arabic Presentation Form Unicodes, the wildcard and query forms of Remove
  and Keep Glyphs, `%d` templates in versionString, and the properties postscriptFullNames,
  styleNames, styleMapFamilyNames, styleMapStyleNames and compatibleFullNames.
- Where no oracle reads a parameter, FontCraft follows the handbook and records each choice of
  its own as a guess on the row.

## Open questions

Answers from the M7 planning pass are written after each question; questions about parameters
of other milestones stay open for those milestones.

Scope
- Only 24 of the 205 entries state where the parameter may be set. For the rest (`scope = []`) the
  allowed tabs must be taken from the Font Info chapter or observed in the reference app.
  **Decided (guess on `font.customParameters.add`):** a parameter whose entry names no scope is
  allowed in all three tabs; the catalogue (M7.2) records the stated scopes. § 8 check 13.
- `TTFStems`: recorded as master because the entry speaks of "the main master"; can it also sit on
  the font like `TTFZones`? **Open for M10.**
- `Local Interpolation`, `Post Table Type`, `compatibleFullNames`: the wording mentions "the
  instance" in passing without saying the parameter belongs there; left unscoped. **Answered for
  compatibleFullNames:** a property allowed on the font and on instances, like the other name
  properties. The others stay open for M9 and M2.

Value types the appendix does not give
- The plural name keys (`familyNames`, `styleNames`, `copyrights`, `designers`, `manufacturers`,
  `licenses`, `trademarks`, `descriptions`, `sampleTexts`, `compatibleFullNames`,
  `postscriptFullNames`, `preferredFamilyNames`, `preferredSubfamilyNames`, `styleMapFamilyNames`,
  `styleMapStyleNames`, `variableStyleNames`): recorded as `dict` (language to text) by inference.
  Confirm the stored shape against the file-format documentation, and whether singular spellings
  are also accepted. **Answered (GlyphsSDK, glyphsLib):** in format 3 they are `properties` with a
  `values` list of `{language, value}` (OpenType language tags, `dflt` the default); format 2
  stores the singular spelling as a custom parameter, which M1 reads into the same property.
  Both spellings are accepted; the rows' summaries say so.
- `Axis Mappings`, `gasp Table`, `Virtual Master`, `Import Font`, `Import Master`,
  `Fit Curve Panel Settings`, `variablePostscriptFontName`: no type stated; kinds are inferred.
  **Answered for Import Font and Import Master:** text, a path (guesses on the rows); Fit Curve
  Panel Settings was settled by M3. The others stay open for M9 and M10.
- `Color Palettes`, `Hangul Composition Groups`, `Family Alignment Zones`, `Axis Location`,
  `TTFStems`, `TTFZones`, `panose`, `fsType`, `unicodeRanges`, `codePageRanges`: typed "list" with
  no element format. **Answered for panose, fsType, unicodeRanges, codePageRanges** by M2's
  reader (numbers, bit numbers, code page numbers or `bit N`). The others stay open for their
  milestones.
- `Master Color`, `Master Stroke Color`, `Master Background Color` and their Dark variants: typed
  "string"; the colour notation is not given. **Open for M11 and M14.**
- `Default Layer Width` is typed integer but accepts `script: value`; `Point To Unit Ratio` is typed
  float but described as an integer; `SBIX to SVG` is typed integer but described as a switch.
  **Answered for Point To Unit Ratio:** any finite number above 0 is accepted (catalogue kind
  number). Default Layer Width was settled by M5; SBIX to SVG stays open for M11.
- `Design Grid`: said to take three values while the syntax shows a script and two numbers; unit
  and relation to `CJK Grid` unclear. **Settled by M3** (`cmd/guide.rs`).

Apparent duplicates and conflicting descriptions
- `Optimize Variable Deltas` and `Variable Font Optimize Deltas` have identical descriptions.
  Aliases, or old and new name? Which one does the reference write? **Open for M9.**
- `Variable Font Origin` and `Variation Font Origin`: same question. **Open for M9.**
- `Variable Font Style Name` and `variableStyleName` carry the description of
  `Variable Font Family Name` (they say "family name"). Presumably style names; how do they relate
  to `Variable SubfamilyName` and `variableStyleNames`, and which name IDs does each set? **Open
  for M9.**
- `styleNames` and `styleMapStyleNames` are both said to be name ID 2. Which wins, and does
  `styleNames` actually feed ID 17? **Decided (M7 plan § 7, question 2):** styleNames replaces
  the style name, so it reaches ID 2 for style-linked fonts and ID 17 otherwise;
  styleMapStyleNames sets ID 2 and the bold and italic bits and wins over styleNames for ID 2.
  Neither oracle reads the properties.
- `styleMapFamilyNames`: the name ID (presumably 1) is not stated. **Decided:** ID 1 of
  style-linked fonts, as fontmake writes the format 2 parameter styleMapFamilyName.
- `Export AAT` and `Export morx Table` both govern the morx table: one is described as a way to
  suppress it, the other as inserting it. Precedence when both are present? **Open for M8.**
- `Export SVG Table` is said to be exported by default, yet `Color Layers to SVG` and
  `SBIX to SVG` exist to request SVG output. What exactly triggers an SVG table? **Open for M11.**
- `Decompose Components` and `Decompose Components in Variable Font`: does the first also apply to
  variable exports? **Open for M6 and M9.**

Behaviour left unclear
- `Autohint`: PostScript, TrueType, or whichever applies to the output format? **Open for M10.**
- `DisableAllAutomaticBehaviour`: only two of the "most" automatic steps are named. **Open for
  M2's row.**
- `Export OpenType`: which tables count as "most OpenType tables"? **Open for M8.**
- `Webfont Only`: which data is removed. **Open for M2's row.**
- `Use Line Breaks`: the entry says checked means unescaped line breaks, then says unchecked helps
  version control; which state is the default and which is meant to help diffs? **Open for M1's
  row.**
- `Use Arabic Presentation Form Unicodes`: which glyphs get which code points; does it change
  `cmap` only? **Guess** on the row: unencoded positional and ligature forms get Unicode's
  presentation-form code points; cmap only. § 8 check 40.
- `isFixedPitch`: are widths really forced to the space width at export, and what is the
  `.monospace` number in the master settings? **Open for M2's row.**
- `Preview Descender`: default given as winAscent, which reads as a misprint. FontCraft uses
  winDescent (as a distance below the baseline), departing from the handbook's words. **Settled
  by M5.**
- `superscriptYOffset`: "positive goes below the baseline" looks copied from the subscript entry.
  **Open for M2's row.**
- `Name Table Entry`: default language printed as 0x0049; the Windows English id is 0x0409. The
  range "0 to 65536" presumably means 0 to 65535. **Answered:** read as 0x0409 and 0–65535, as M2's
  reader does.
- `Filter` > `Transformations`: `LSB` and `RSB` shifts carry an asterisk in the source that is
  never explained (possibly a percentage or relative form). **Settled by M3.12**'s parser.
- `Filter` > `Autohinting`: listed as a filter value without saying what it does per instance or how
  it relates to the `Autohint` parameter. **Answered (guess, M10.31):** a Filter or PreFilter
  string `Autohinting` turns autohinting on for the instance's export in either flavour, even
  against an Autohint parameter that is off; a master's export reports it as not used
  (`filter.customParameter.autohinting`).
- `Grid Spacing`: behaviour at exactly 1.0 is stated twice with slightly different bounds; treated
  as "below 1 = hundredths, 1 or more = whole units". **Open for M2's row.**
- `Scale to UPM`: does it scale kerning, metrics, hint values and guides as well as outlines, and
  how is rounding done? **Guess** (`font.info.scaleToUnitsPerEm`, shared): everything measured in
  units, rounded to the grid. § 8 check 43.
- `TrueType Keep GlyphOrder` against `glyphOrder` and `DisableAllAutomaticBehaviour`: precedence.
  **Open for M2's and M4's rows.**
- `Keep Glyphs`: "mutually exclusive" with `Remove Glyphs`; what happens when both are present?
  **Guess** on Keep Glyphs: Keep first, then Remove, with a warning. Check 37.
- `Rename Glyphs` is described as a swap; what if `newname` does not exist? **Guess** on the row:
  the glyph is renamed, with a warning. Check 38.
- `Link Metrics With First Master`: is the link live while editing or resolved at export?
  **Settled by M5.**
- `Import Font` / `Import Master`: path form (relative or absolute) and behaviour when the file is
  missing. **Guess** on the rows: relative to the host's folder unless absolute; a missing file
  is a warning. Check 50.
- `Family Alignment Zones`: which table entry it writes (presumably the CFF family blues) is not
  stated; `TTFFamilyZonesThreshold` uses a font-level `TTFZones` instead. **Open for M10.**
- `Get Hints From Master`: value is a master name or id? **Open for M10.**
- `Disable Masters`, `Variable Font Origin`, `Link Metrics With Master`: masters referenced by name;
  behaviour with duplicate names. **Open for M9; Link Metrics With Master settled by M5.**
- `Keep UI-Font Bounding Box`: stated as Oriya only; confirm. **Open for M8.**
- `ROS`: whether glyph naming or CID assignment requirements apply for the non-Identity
  collections. **Open for M12**, where the row moved.
- `Export Mac Name Table Entries` and `Compatible name Table`: interaction. **Decided:** Compatible
  name Table writes Macintosh records even when Export Mac Name Table Entries is absent; with the
  latter off, it writes none (guess on Compatible name Table).
- `Has WWS Names`, `Use Typo Metrics`: are the bits set automatically when the parameter is absent?
  **Answered (oracle, M2):** no; the bits come only from the parameters.

Overlap with other inventory files
- `spec/inventory/10-interpolation.toml` already holds `[[parameter]]` rows for `Axis Location`,
  `Axis Mappings`, `Virtual Master`, `Variable Font Origin`, `Disable Masters`,
  `Enforce Compatibility Check`, `Feature for Feature Variations`, `Style Name as STAT entry` and
  `Elidable STAT Axis Value Name`. Parameter names must be unique across files, so one copy of each
  has to go; the interpolation rows give scopes this appendix does not. **Answered:** each name
  now has one row (checked by the M7 planning pass); the M7.2 catalogue generator fails on a
  duplicate name.
