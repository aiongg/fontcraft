# Source formats and the font model

Field-by-field comparison of the three source formats the in-memory model must round-trip
(`.glyphs` 2/3/4, UFO 3, designspace 5), what the available Rust crates cover, and the model shape
we recommend. Inventory rows: `spec/inventory/30-formats.toml` (`[[format]]` ids are quoted here in
`code`, round-trip rules as `roundtrip.*`). Commands that use these readers and writers are in
`spec/inventory/07-import-export.toml` (`file.open`, `file.save`, `file.saveAs`, `file.exportUfo`,
`file.importMetrics`, `file.importFeatures`, `file.importOutlines`).

Sources, dated 2026-10-07:

- Glyphs format documents and JSON schemas: GlyphsSDK `GlyphsFileFormat/GlyphsFileFormatv2.md`,
  `v3.md`, `v4.md` (the version 4 document is dated 17 July 2026), `Schemas/glyphs-1.schema.json`,
  `glyphs-3.schema.json`, `glyphs-4.schema.json`, `fontinfo-*.schema.json`.
- UFO 3 specification (`ufo-spec/versions/ufo3`), UFO 2 for import and saving, UFO 1 for import.
- fontTools `Doc/source/designspaceLib/xml.rst` and `Lib/fontTools/designspaceLib/__init__.py`.
- Crates read as code: `norad` 0.19.0, `glyphs-reader` 1.0.0, `fontir` 1.0.0, `glyphs2fontir` 1.0.0,
  `ufo2fontir` 1.0.0, `fontra2fontir` 0.5.0 (all in the fontc and norad repositories).
- Crates known only from their crates.io and docs.rs pages: `glyphslib` 0.2.8, `openstep-plist` 1.1.0,
  `babelfont` 0.2.2.
- Converter lib keys (`com.schriftgestaltung.*`, `com.github.googlei18n.ufo2ft.*`): no specification
  was available. They are quoted only where they appear in test data or code of the fontc, fontTools,
  fontmake, fontbakery or AFDKO repositories; each such mention says "seen in".

Notation in the tables: `[4]` = only in Glyphs format version 4; `[≤3]` = removed in version 4;
`[2: …]` = the version 2 spelling. "DS" = designspace. "lib" without qualifier = the UFO font
`lib.plist`; "glyph lib" = the `<lib>` of a GLIF.

## 1. How the three formats are organised

| | `.glyphs` | UFO 3 | designspace 5 |
|---|---|---|---|
| Syntax | OpenStep-style text property list, UTF-8, sorted keys, no indentation | XML property lists plus one XML GLIF per glyph per layer | XML |
| Unit | One file (or one package directory) = whole family: all axes, masters, instances | One directory = one master | One file = the family, pointing at UFOs |
| Multi-master | Built in: `fontMaster[]`, every glyph carries a layer per master | Not expressible | `sources[]` reference UFOs (or a layer inside a UFO) |
| Layers | Per glyph: an ordered list of layers, each tied to a master | Per font: named layer directories, each holding any subset of glyphs | A source may name one UFO layer |
| Free-form data | `userData` dictionaries and `customParameters` lists on most objects | `lib` dictionaries (font, layer, glyph, and per object via identifiers), `data/` directory | `lib` on the document, each variable font, each instance |
| Numbers | Unquoted tokens; integers and decimals, no exponent; schema types them `f64`/`u32`/… | "integer or float" attributes and plist `<integer>`/`<real>` | Attribute strings |
| Versions | No `.formatVersion` key = version 2 (schema file `glyphs-1`); 3; 4 | `metainfo.plist` `formatVersion` 3 (+ `formatVersionMinor`) | `format="5.2"` on the root; missing = 3.0 |

## 2. Field tables

### 2.1 Font-level info and naming

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Family name | `familyName` `[≤3]`; `properties[]` entry `familyNames` with `values[] {language, value}` (the only place in v4) | `fontinfo.plist` `familyName` in every UFO; DS `<source familyname>`, `<instance familyname>` and localised `<familyname xml:lang>` children | UFO repeats it per master and the copies can disagree. Glyphs language tags are OpenType language-system style (`dflt`, `DEU`); DS uses `xml:lang` codes; UFO font info is not localisable except through `openTypeNameRecords` (numeric platform, encoding and language ids). |
| Style name | master `name`; instance `name` `[≤3]`, in v4 an instance property | `fontinfo.plist` `styleName`; DS `stylename` attributes and localised elements | A UFO has one style name, which doubles as the master name. |
| Other name strings | `properties[]`, either `{key, value}` or `{key, values[]}`. Keys seen in `glyphs-reader`: `copyrights`, `designers`, `designerURL`, `manufacturers`, `manufacturerURL`, `licenses`, `licenseURL`, `trademarks`, `descriptions`, `sampleTexts`, `compatibleFullNames`, `versionString`, `uniqueID`, `vendorID`, `postscriptFontName`, `postscriptFullName`, `preferredFamilyNames`, `preferredSubfamilyNames`. `[2: top-level copyright, designer, designerURL, manufacturer, manufacturerURL]` | `copyright`, `trademark`, `openTypeNameDesigner`, `openTypeNameDesignerURL`, `openTypeNameManufacturer`, `openTypeNameManufacturerURL`, `openTypeNameLicense`, `openTypeNameLicenseURL`, `openTypeNameDescription`, `openTypeNameSampleText`, `openTypeNameCompatibleFullName`, `openTypeNameVersion`, `openTypeNameUniqueID`, `openTypeNamePreferredFamilyName`, `openTypeNamePreferredSubfamilyName`, `openTypeNameWWSFamilyName`, `openTypeNameWWSSubfamilyName`, `openTypeOS2VendorID`, `postscriptFontName`, `postscriptFullName`, `styleMapFamilyName`, `styleMapStyleName` | The Glyphs format document does not enumerate property keys; the list is open. Properties exist on font, master and instance, so a name can be overridden per export; UFO has one value per file and DS adds per-instance overrides only through `lib` → `public.fontInfo`. |
| Arbitrary name records | font or instance custom parameter `Name Table Entry` | `openTypeNameRecords[] {nameID, platformID, encodingID, languageID, string}` | Glyphs encodes the ids inside a string; UFO has a structured record. |
| Version | `versionMajor`, `versionMinor` | `versionMajor`, `versionMinor`; `openTypeNameVersion` | Same split. |
| Units per em | `unitsPerEm` (integer) | `unitsPerEm` (integer or float) per UFO | Must agree across UFOs; a float UPM cannot be stored in Glyphs. |
| Creation date | `date`, `"YYYY-MM-DD HH:MM:SS +0000"` | `openTypeHeadCreated`, `"YYYY/MM/DD HH:MM:SS"`; modified date in lib `public.openTypeHeadModified` | Different separators; UFO has no time zone. |
| Font note | `note` (package v4: `note.md`) | `fontinfo.plist` `note` | Per UFO in UFO. |
| OS/2, hhea, vhea, post, head values | `customParameters[]` on font, master or instance; names seen in `glyphs-reader`: `typoAscender`, `typoDescender`, `typoLineGap`, `winAscent`, `winDescent`, `hheaAscender`, `hheaDescender`, `hheaLineGap`, `vheaVertAscender`…, `underlinePosition`, `underlineThickness`, `strikeoutPosition`, `strikeoutSize`, `subscriptX/YSize`, `subscriptX/YOffset`, `superscript…`, `panose`, `fsType`, `unicodeRanges`, `codePageRanges`, `isFixedPitch`, `Use Typo Metrics`, `Has WWS Names`, plus the UFO spellings (`openTypeOS2TypoAscender`, `openTypeHheaCaretSlopeRise`, …) as aliases | `fontinfo.plist` `openTypeOS2*`, `openTypeHhea*`, `openTypeVhea*`, `openTypeHead*`, `postscriptUnderlinePosition/Thickness`, `postscriptIsFixedPitch`; lib `public.openTypePostUnderlinePosition` | Glyphs: an ordered list with a `disabled` flag, repeated names allowed, values at three scopes. UFO: one typed field per value per master, no disabled state, no instance scope. Bit sets are lists of bit numbers in UFO. |
| `gasp` | custom parameter `GASP Table` | `openTypeGaspRangeRecords[] {rangeMaxPPEM, rangeGaspBehavior}` | |
| `meta` | custom parameter `meta Table` (`tag`, comma-separated `data`) | lib `public.openTypeMeta` | |
| Glyph order | order of `glyphs[]`, overridden by the `glyphOrder` custom parameter | lib `public.glyphOrder` (may list absent glyphs and omit present ones) | The Glyphs array order is itself data (Font View order); the UFO has no order without the lib key. |
| Document settings | `settings {disablesAutomaticAlignment, disablesNiceNames, gridLength, gridSubDivision, keyboardIncrement, keyboardIncrementBig, keyboardIncrementHuge, keepAlternatesTogether, previewRemoveOverlap, snapToObjects, fontType}`, `[4]` `dependencies`. `[2: same names at top level]` | none; seen in lib as `com.schriftgestaltung.disablesAutomaticAlignment`, `…useNiceNames`, `…keyboardIncrement`, `…customParameter.GSFont.*` (fontmake test data) | Editor state with no UFO home. `fontType` (`default`, `variable`, `layerFont`, `iconSet`) changes how static instances interpolate. |
| Edit View tabs | `DisplayStrings[]`; package: `UIState.plist` `displayStrings` | none; seen in lib as `com.schriftgestaltung.DisplayStrings` | UI state inside the document. |
| File and tool version | `.appVersion`, `.formatVersion` | `metainfo.plist` `creator`, `formatVersion`, `formatVersionMinor` | |
| WOFF metadata | none in the schema | `woffMajorVersion`, `woffMinorVersion`, `woffMetadata*` | UFO only. |
| Legacy fields | none | `year`, `macintoshFONDFamilyID`, `macintoshFONDName`, `postscriptUniqueID`, `postscriptWeightName`, `postscriptDefaultCharacter`, `postscriptWindowsCharacterSet`, `postscriptDefaultWidthX`, `postscriptNominalWidthX`, `postscriptSlantAngle`, `postscriptForceBold`, `openTypeOS2FamilyClass` | UFO only; carried as parameters under their UFO names. |
| Unicode variation sequences | none in the schema | lib `public.unicodeVariationSequences` | UFO only. |

### 2.2 Axes

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Axis list | `axes[] {name, tag, hidden, default}`, `[4]` `names[] {language, value}`, `userData` | DS `<axes><axis name tag minimum maximum default hidden>`, `<labelname xml:lang>` | Glyphs stores no minimum or maximum: the range is whatever the masters and instances span. DS minimum, default and maximum are user-space values. |
| Fixed axes `[2]` | none; masters carry `weightValue`, `widthValue`, `customValue`, `customValue1…3`; an `Axes` custom parameter may name them | — | `glyphs-reader` synthesises Weight/`wght`, Width/`wdth`, Custom/`XXXX` when nothing is declared and accepts at most three axes for version 2. FontCraft reads the `Axes` parameter when there is one, and otherwise makes as many of these axes as the highest key any master or instance uses (up to six; further ones `Custom1`/`XXX3` and so on). |
| Tag | may exceed four characters; the first four are the exported tag, the rest tells apart axes that share a tag (higher-order interpolation) | exactly four characters | A long tag must be shortened for DS and the suffix kept in the axis's `extra`. |
| Order and indexing | position in `axes[]`; `axesValues`, `axisRules`, `coordinates` are parallel arrays | locations are dictionaries keyed by axis **name** | Renaming an axis touches every location in DS; reordering one touches every parallel array in Glyphs (`roundtrip.glyphsDependentValues`). |
| User ↔ design mapping | font custom parameter `Axis Mappings`, or `Axis Location` on masters and instances (`glyphs-reader` uses the mappings when both exist) | `<map input output>` nodes inside the axis | Two alternative mechanisms in Glyphs, one in DS. Master locations in both formats are design coordinates. |
| Cross-axis mapping (avar 2) | none in the schema | `<mappings><mapping><input><output>` (5.1), `description` (5.2) | DS only. |
| Discrete axis | none | `<axis values="0 1">` | DS only; such a family is several interpolation spaces in one document. |
| STAT | custom parameters `Export STAT Table`, `Style Name as STAT entry`, `Elidable STAT Axis Value Name` | `<axes elidedfallbackname>`, per-axis `<labels ordering><label name uservalue userminimum usermaximum linkeduservalue elidable oldersibling>`, top-level `<labels><label><location>` (format 4) | DS is explicit and localisable; Glyphs derives most of STAT from instance names. |
| Glyph-local axes | glyph `partsSettings[] {name, bottomValue, topValue}`; `[4]` glyph `axes[]` | none | Smart glyphs only exist in Glyphs. |
| Extra extremes | custom parameter `Virtual Master` (repeatable) | not needed: axis ranges are explicit | |
| Default location | custom parameter `Variable Font Origin` (a master id); otherwise the first master | the source located at every axis default | |

### 2.3 Masters / sources

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Identity | `fontMaster[] {id, name, iconName, visible}`, `[4]` `active` | DS `<source filename name familyname stylename layer>`; the UFO itself | A Glyphs master is an id that layers point at. A DS source is a file path; seen in lib: `com.schriftgestaltung.fontMasterID`, `…fontMasterOrder`, `…customParameter.GSFontMaster.iconName`. |
| Location | `axesValues[]` (by axis index, design coordinates) `[2: weightValue, widthValue, customValue…]` | `<location><dimension name xvalue [yvalue]>` | DS allows an anisotropic `yvalue` and, for instances and labels, `uservalue`. |
| Vertical metrics | font `metrics[] {type, name, filter, horizontal}` with master `metricValues[] {pos, over}` in the same order; `type` ∈ `ascender`, `cap height`, `slant height`, `x-height`, `midHeight`, `bodyHeight`, `descender`, `baseline`, `italic angle`, `italic slope` `[2: ascender, capHeight, xHeight, descender, italicAngle, alignmentZones[]]` | `fontinfo.plist` `ascender`, `capHeight`, `xHeight`, `descender`, `italicAngle`; zones in `postscriptBlueValues`, `postscriptOtherBlues`, `postscriptFamilyBlues`, `postscriptFamilyOtherBlues` | Glyphs: an open, named list, each entry with an overshoot and an optional glyph filter (so "x-height for small caps" is one more metric). UFO: five fixed numbers and separate anonymous zone lists. `italicAngle` is counter-clockwise from vertical in UFO. |
| Stems | font `stems[] {name, horizontal, filter}` + master `stemValues[]` `[2: horizontalStems, verticalStems]` | `postscriptStemSnapH`, `postscriptStemSnapV` | Named and filterable versus two plain lists. |
| Named numbers | font `numbers[] {name}` + master `numberValues[]` | none | Glyphs only (usable in feature code and metrics keys). |
| Other PostScript privates | custom parameters | `postscriptBlueFuzz`, `postscriptBlueShift`, `postscriptBlueScale`, `postscriptForceBold` | |
| Per-master overrides | `customParameters[]`, `properties[]` | the `fontinfo.plist` of that UFO | In UFO every font-info value is per master by construction. |
| Global guides | master `guides[]` | `fontinfo.plist` `guidelines[]` | Equivalent scope, since one UFO is one master. |
| Sparse master | intermediate layers inside individual glyphs | `<source filename="X.ufo" layer="…">` pointing at a non-default UFO layer that holds only some glyphs | In fontc test data the converter's layer names are the coordinates, e.g. `{600, 400}`. |
| Linked metrics | master custom parameters `Link Metrics With First Master`, `Link Metrics With Master` | none | |
| Muting | none | deprecated `<glyph mute>`, `<kerning mute>`, `<info copy>`, `<lib copy>`, `<features copy>` inside `<source>` | DS only; kept verbatim. |
| User data | `userData`; autosave files add `tempData` | the UFO's `lib.plist` | |

### 2.4 Instances / exports

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Static instance | `instances[] {name [≤3], type: "single", axesValues, exports, visible, properties, customParameters, userData}`, `[4]` `id`. Schema also allows `type` `static`, `icon`, `particles` | DS `<instance name familyname stylename filename postscriptfontname stylemapfamilyname stylemapstylename location>` with `<location>` and localised name elements | DS needs an output `filename`; Glyphs derives file names. `exports` and `visible` have no DS attribute. |
| Variable font setting | an instance with `type: "variable"` | `<variable-fonts><variable-font name filename><axis-subsets><axis-subset name [userminimum userdefault usermaximum \| uservalue]>` + `lib` | DS slices or pins axes per variable font; Glyphs has no documented axis subsetting on the entry. Without any `<variable-font>`, a DS with only continuous axes describes one full-space variable font. |
| Location | `axesValues[]` (design coordinates); user-space position via `Axis Location` | `<dimension xvalue [yvalue]>` or `<dimension uservalue>`, or `location="<label name>"` | Anisotropic: DS `yvalue`; Glyphs `instanceInterpolations {masterId: n \| [x, y]}` with `manualInterpolation`. |
| Style linking | `isBold`, `isItalic`, `linkStyle` | `stylemapfamilyname`, `stylemapstylename` | Flags plus a partner name versus the resolved name pair. |
| Weight and width class | `weightClass` (1–1000, default 400), `widthClass` (1–9, default 5) `[2: names, such as Bold and Condensed]` | instance `lib` → `public.fontInfo` → `openTypeOS2WeightClass`, `openTypeOS2WidthClass` | |
| Per-instance info | `properties[]`, `customParameters[]` (filters, glyph renaming and removal, export folder, …) | instance `lib` → `public.fontInfo` (any `fontinfo.plist` key); other lib keys | DS has no notion of export-time operations; they go to lib. |
| Instance as UFO glyph overrides | none | deprecated `<glyphs><glyph name unicode mute><masters>…` | DS only; kept verbatim. |

### 2.5 Glyphs

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Name | `glyphname` | key in `glyphs/contents.plist`; the GLIF `name` attribute is to be ignored on read | UFO additionally stores a file name per glyph, which must be preserved (`roundtrip.ufoFileNames`). |
| Code points | `unicode`: one integer, or an ascending list of at least two `[2: hexadecimal string, comma-separated when several]` | repeated `<unicode hex="…">`, first one is the primary value | Decimal in v3 and later, hexadecimal in v2 and UFO. The Glyphs list is sorted, so "primary" is lost; UFO stores code points per layer file, and they can disagree between masters. |
| Production name | `production` | lib `public.postscriptNames {name: psName}` | Per glyph versus one font-level map. |
| Exported or not | `export` | lib `public.skipExportGlyphs[]` | Same. |
| Classification | `category`, `subCategory`, `script`, `case`, `direction` (all optional overrides of the glyph database) | lib `public.openTypeCategories {name: unassigned \| base \| mark \| ligature \| component}`; seen in glyph lib: `com.schriftgestaltung.Glyphs.category`, `…subCategory` | UFO standardises only the GDEF class. |
| Colour label | `color`: index into the label palette, or an RGBA tuple (`u8` in v3, 0–1 floats in v4) | glyph lib `public.markColor` (`"r,g,b,a"`, 0–1) | An index has no exact colour; the reverse needs nearest-match or an explicit colour. In UFO the mark is per layer file. |
| Tags | `tags[]` (sorted) | none | |
| Note | `note` | `<note>` in each GLIF | Per glyph versus per layer file. |
| Lock | `locked` | none | |
| Last change | `lastChange` | seen in glyph lib: `com.schriftgestaltung.Glyphs.lastChange` | |
| Kerning groups | `kernLeft`, `kernRight`, `kernTop`, `kernBottom` `[2: leftKerningGroup, rightKerningGroup, topKerningGroup, bottomKerningGroup]` | `groups.plist` membership (§2.10) | |
| Metrics keys | `metricLeft`, `metricRight`, `metricWidth`, `metricTop`, `metricBottom`, `metricVertWidth`, `metricVertOrigin` `[2: leftMetricsKey, rightMetricsKey, widthMetricsKey, …]` | none | |
| Sorting | `sortName`, `sortNameKeep` | none (`public.glyphOrder` is the result, not the rule) | |
| Smart glyph definition | `partsSettings[]`, `[4]` `axes[]` | none | |
| Grouping `[4]` | `group`, `groupIdx` | none | Meaning not explained in the format document. |
| User data | `userData` | glyph lib | A glyph lib exists once per layer file. |
| Existence per master | one glyph object holds every master's layer | a GLIF per UFO; a glyph can be absent from some UFOs | A Glyphs glyph always has every master layer. |
| TrueType overlap flag | none in the schema | glyph lib `public.truetype.overlap` | |

### 2.6 Layers

A Glyphs layer is `layers[]` entry `{layerId, associatedMasterId, name, width, shapes, anchors, guides,
hints, annotations, background, backgroundImage, attr, color, visible, userData, …}`. Its role is not
a field; it follows from the id and `attr`, and the attributes can combine (an intermediate layer can
also be an alternate).

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Master layer | `layerId` equals a master `id`; `associatedMasterId` omitted | the glyph's GLIF in the default layer (`glyphs/`, `public.default`) of that master's UFO | |
| Backup layer | `associatedMasterId` set, empty `attr`; `name` is real data | a further layer directory listed in `layercontents.plist` | A UFO layer is font-wide and named once; every Glyphs glyph has its own list, so N backups with different names in different glyphs become N sparse UFO layers. `glyphs-reader` discards these layers ("draft"). |
| Intermediate ("brace") layer | `attr.coordinates[]` in axis order; `[4]` an entry may be `"-"` (use the master's value) `[2: braces in the layer name, e.g. {120, 80}]` | a UFO layer plus a DS `<source layer="…">` at that location | The DS source is font-wide: one sparse master holding whichever glyphs have that location. |
| Alternate ("bracket") layer | `attr.axisRules[] {min, max}` in axis order `[2: square brackets in the layer name]` | DS `<rules processing><rule name><conditionset><condition name minimum maximum>` + `<sub name with>`; the alternate outlines live in a separate glyph (fontc names it `<glyph>.BRACKET.varAlt01`) | A layer attribute versus a font-level substitution of whole glyphs. Converting needs a generated glyph per distinct rule region and merging back needs to recognise them. Conditions are design coordinates in both. The lib key `com.github.fonttools.varLib.featureVarsFeatureTag` and `processing="first\|last"` choose the feature (`rvrn`, `rclt`); Glyphs uses the `Feature for Feature Variations` parameter. |
| Colour palette layer | `attr.colorPalette`: palette index, or `"*"` for the text colour | lib `com.github.googlei18n.ufo2ft.colorLayerMapping` / `…colorLayers` plus one UFO layer or glyph per colour (keys read by `ufo2fontir`) | |
| Full-colour layer | `attr.color` (true); shapes carry fills and gradients (§2.7) | none standard | |
| Bitmap colour layer | `attr.sbixSize`; the layer's image is the bitmap | none | |
| SVG colour layer | `attr.svg` | none | |
| Palettes | font custom parameter `Color Palettes`: v3 a list of colour lists, v4 `{palettes[] {name, backgroundStyle[], colors[]}, names {index: name}}` | lib `com.github.googlei18n.ufo2ft.colorPalettes` (read by `ufo2fontir`) | |
| Background | `background {shapes, anchors, guides, hints, annotations, backgroundImage}` nested in the layer | a layer named `public.background` | Per layer of each glyph (so one per master) versus one font-wide layer per UFO, which happens to line up because one UFO is one master. Backups of other layers have backgrounds too; UFO has no place for those. |
| Advance | `width` (required), `vertWidth`, `vertOrigin` | `<advance width height>`, glyph lib `public.verticalOrigin` | |
| Layer metrics keys | `metricLeft`, `metricRight`, `metricWidth`, `metricTop`, `metricBottom`, `metricVertWidth`, `metricVertOrigin` | none | Override the glyph-level keys for one master. |
| Smart-glyph pole | `partSelection {property: 1 \| 2}` (1 = bottom value, 2 = top) `[2: userData.PartSelection]`; `[4]` also via `attr.coordinates` | none | |
| Annotations | `annotations[] {type: Text \| Arrow \| Circle \| Plus \| Minus, pos, angle, text, width}` | none | |
| Layer colour, visibility | `color`, `visible`, `[4]` `active` | `layerinfo.plist` `color` (font-wide per layer) | |
| Layer name | `name`, stored only when not derived from the role | the layer name in `layercontents.plist` (mapped to a directory name) | |
| User data | `userData` | glyph lib; `layerinfo.plist` `lib` | |

### 2.7 Shapes: paths

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Container | `shapes[]`: paths and components in one ordered list; `[4]` also images and shape groups `[2: separate paths[] and components[]]` | `<outline>` with `<component>` and `<contour>` children | Seen in glyph lib: `com.schriftgestaltung.Glyphs.shapeOrder` to remember the interleaving (`roundtrip.shapeOrderKept`). |
| Path | `{closed, nodes[], locked, attr}` | `<contour identifier>` | UFO has no `closed` flag: a contour is open exactly when its first point is `move`. |
| Node | tuple `[x, y, "flags", attr?]` `[2: string "x y TYPE [SMOOTH]"]` | `<point x y type smooth name identifier>` | |
| On-curve types | `l` line, `c` cubic curve, `q` quadratic curve; `m` is listed as a type but paths use `closed` instead; `[4]` `u` quartic, `h` Hobby, `r` spiral `[2: LINE, CURVE, QCURVE]` | `line`, `curve`, `qcurve`, `move` | v4's extra curve types have no UFO form. |
| Off-curve | `o` `[2: OFFCURVE]` | `offcurve` (the default when `type` is absent) | A UFO `curve` may be preceded by 0, 1 or 2 off-curves (1 = a quadratic drawn as "curve"); a `qcurve` by any number, with implied on-curve points between them; a contour may consist of off-curves only. Whether Glyphs accepts all of these inside a `q` segment is not stated in the format document. |
| Smooth | suffix `s`; `[4]` suffix `t` for a tangent connection | `smooth="yes"` | UFO does not tell smooth from tangent. glyphsLib writes no `smooth` on an open contour's `move` point (Oswald has 7 smooth first nodes of open paths); FontCraft does the same and reports them. |
| Start node | closed path: the **last** entry of `nodes[]` is the start node (confirmed in `glyphs-reader` and `glyphs2fontir`) | closed contour: the first `<point>` | Rotate by one on conversion (`roundtrip.startNodeConvention`). |
| Node flags | `R` / `C` orientation (right or centre anchored instead of left), `X` locked | none | |
| Node name | `name` in the fourth tuple item `{name = hr00;}` `[2: a dictionary after the type, "131 250 LINE {name = hr00;}"]` (seen in fontc's test files and the format samples; not in the format documents) | `name` (used as `pointTag` by `public.postscript.hints`) | glyphsLib treats the whole dictionary as user data with `name` in it; FontCraft reads `name` as the name and `userData` as the user data. |
| Node identity | none (hints address nodes by index) | `identifier` | UFO only. |
| Node data | fourth tuple item: `attr`, containing `userData`; `[4]` also `hoi {axisTag: {ip} \| {rc, rd} \| {tl, tu, al, au}}` | glyph lib `public.objectLibs {identifier: dict}` | |
| Path drawing attributes | `attr {fill, fillColor, gradient, strokeColor, strokeWidth, strokeHeight, strokePos, lineCapStart, lineCapEnd, lineJoin, mask, reversePaths, shadow, shadowIn, color}`, `[4]` `{strokeGradient, opacity, hidden, compositing, group}` | none | Stroke sizes may be numbers or expression strings in v4. `strokePos` changes meaning: v3 0 = outside, 0.5 = centred, 1 = inside; v4 −1 = inside, 0 = centred, 1 = outside. `mask` is a boolean in v3 and 0/1/2 in v4. |
| Colours | tuple: `[r, g, b, a]`, `[gray, a]`, `[c, m, y, k, a]`, or `["p", index, alpha]`; components are `u8` in v3, 0–1 floats in v4 | UFO colour definition `"r,g,b,a"`, 0–1, sRGB | |
| Gradients | linear `{colors[[color, offset]], start, end}`; radial adds `type` (`circle` in v3, `radial` in v4) and `[4]` `startRadius`, `endRadius`, `angle`; `[4]` conic `{type: conic, center, angle, startAngle, controlRadius}`; `[4]` `extend: repeat \| reflect` | none | Start and end points are relative to the shape's bounds. |
| Shape groups `[4]` | shape `{groupId, attr}`; members name it in `attr.group` | none | |
| Image as shape `[4]` | an image object in `shapes[]` | `<image>` (one per GLIF, always behind the outline) | |
| Coordinate precision | `f64`; written as integer or decimal | integer or float | Neither format rounds. Grid rounding is an editor behaviour driven by `settings.gridLength` / `gridSubDivision`. |

### 2.8 Shapes: components

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Reference | `ref` `[2: name]` | `<component base>` | A UFO component must refer to a glyph in the same layer. |
| Transform | `pos [x, y]`, `scale [sx, sy]`, `angle`, `slant [x, y]` `[2: transform "{a, b, c, d, tx, ty}"]` | `xScale xyScale yxScale yScale xOffset yOffset` | Decomposed versus matrix. `glyphs-reader` composes translate · slant · rotate · scale and treats slant as degrees; a general matrix has no unique decomposition (`roundtrip.componentTransformKept`). |
| Automatic alignment | `alignment`: −1 disabled, 0 default, 1 forced, 3 horizontal only; font `settings.disablesAutomaticAlignment` | none: offsets are stored already resolved | An aligned component's stored position is derived data in Glyphs. On UFO import nothing says whether the offset was meant to be automatic (the reference has a setting to disable alignment for imported files). |
| Attachment anchor | `anchor` (which of several candidate anchors to attach to) | seen in glyph lib: `com.schriftgestaltung.Glyphs.ComponentInfo` = list of `{index, anchor}` (read by `ufo2fontir`) | |
| Smart component values | `piece {property: value}` | none | Must be decomposed or carried in lib. |
| Other master | `masterId` (take the component from another master's layer) | none | |
| Flags | `locked`, `orientation` (0 left, 1 right, 2 centre), `[4]` `traverseAnchors`, `keepWeight` (unused) | object lib via `identifier`: `public.truetype.useMyMetrics`, `public.truetype.roundOffsetToGrid` | Different sets on each side. |
| Drawing attributes | `attr` (same set as paths) | none | |
| User data | `userData` `[≤3]`; inside `attr` in v4 | `public.objectLibs` | |
| Corner, cap, segment, brush components | not shapes: `hints[]` entries of `type` `Corner`, `Cap`, `Segment`, `Brush` with `name` (the glyph), `origin` (node index path), `scale`, `options` (alignment) | none | Decomposed on UFO export. `glyphs-reader` applies corner components at load. |

### 2.9 Anchors and guides

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Anchor | layer `anchors[] {name, pos, locked, orientation}`, `userData` `[≤3]` / `attr` `[4]` `[2: position "{x, y}"]` | `<anchor x y name color identifier>` | UFO names are optional and may repeat; UFO has a colour. UFO 2 has no anchor element: a contour with a single named `move` point stands in. |
| Anchor conventions | by name (handled by the editor and compiler, not the file): `top`/`_top`, numbered ligature anchors, `caret_…`, `*origin`, entry and exit | `caret_…` and `vcaret_…` are in the specification for ligature carets | |
| Local guide | layer `guides[] {pos, angle, name, locked, lockAngle, showMeasurement, filter, orientation, type: Line \| Circle \| Rect, size, length, grid}`, `userData` `[≤3]` / `attr` `[4]`, `[4]` `slope` `[2: guideLines[] with position, alignment]` | `<guideline x y angle name color identifier>` | UFO: `x` alone is vertical, `y` alone is horizontal, all three for an angled line; angle is 0–360 counter-clockwise. The Glyphs document describes its angle as clockwise (to be verified, see Open questions). Circle and rectangle guides, filters and measurements have no UFO form. |
| Global guide | master `guides[]` | `fontinfo.plist` `guidelines[]`; object libs in lib `public.objectLibs` | Same scope. |
| Guide filter | `filter` (predicate text selecting the glyphs a global guide applies to) | none | |

### 2.10 Hints

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| PostScript stem and ghost hints | layer `hints[] {type: Stem \| TopGhost \| BottomGhost \| Flex, horizontal, origin, target, place [position, width], stem, options, name}`; `origin`/`target` are index paths: `[path, node]`, `[path, node, inflection]`, `[path1, node1, path2, node2]` (an intersection), or `"lsb"`/`"rsb"`; a ghost's `target` is `"up"`/`"down"` `[2: brace strings such as "{0, 1}"]` | glyph lib `public.postscript.hints {formatVersion, id, hintSetList[] {pointTag, stems[] "hstem pos width …"}, flexList[]}`; AFDKO data uses `com.adobe.type.autohint.v2` | Glyphs hints are attached to nodes, so they follow edits and interpolate. UFO hints are absolute coordinates guarded by an outline hash (`id`) and are void once the outline changes. |
| Zones and standard stems | master `metricValues[].over`, `stemValues[]` | `postscriptBlueValues` etc., `postscriptStemSnapH/V` | §2.3. |
| TrueType instructions | layer `hints[]` of `type` `TTStem`, `TTShift`, `TTSnap`, `TTInterpolate`, `TTDiagonal`, `TTDelta` with `origin`, `target`, `other1`, `other2`, `stem`, `options`, `settings`; control values and zones in custom parameters | glyph lib `public.truetype.instructions {formatVersion, id, assembly}`; lib `public.truetype.instructions {controlValue, controlValueProgram, fontProgram, maxFunctionDefs, maxInstructionDefs, maxStackElements, maxStorage, maxSizeOfInstructions, maxTwilightPoints, maxZones}` | High-level instructions on nodes versus compiled assembly. Neither converts to the other without a compiler or decompiler. |
| Shape modifiers stored as hints | `type` `Corner`, `Cap`, `Segment`, `Brush`, `[4]` `Head`; also `Tag`, `Auto`, `Unknown` | none | §2.8. |
| Hint transform | `scale` | none | |

### 2.11 Kerning

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Pair tables | `kerningLTR`, `kerningRTL`, `kerningVertical`, each `{masterId: {first: {second: value}}}` `[2: kerning, vertKerning]` | `kerning.plist {first: {second: value}}` in each UFO | One table per direction for all masters versus one table per master file. UFO has no vertical kerning. |
| Group reference in a pair | `@MMK_L_<group>` as first member, `@MMK_R_<group>` as second (seen in `glyphs2fontir` and in test files) | `public.kern1.<group>` as first member, `public.kern2.<group>` as second | The Glyphs prefixes are not in the format document. |
| Group definition | per glyph: `kernRight` = the group it belongs to as the left glyph of a pair, `kernLeft` = as the right glyph; `kernTop`, `kernBottom` for vertical | `groups.plist`: `public.kern1.X = [glyphs…]`, `public.kern2.X = [glyphs…]` | A glyph attribute versus a member list. Mapping: `kernRight` R ↔ `public.kern1.R`, `kernLeft` L ↔ `public.kern2.L` (`roundtrip.ufoKerningGroups`). UFO lists have an order, may contain glyphs that are not in the font, may be empty, and may differ between masters; the Glyphs attribute can express none of that. |
| Other groups | none (feature classes are separate, §2.13); glyphsLib writes them as classes after the feature classes and names them in font `userData` `com.schriftgestaltung.Glyphs.groupsNotInFeature` | any other key in `groups.plist`; order hint in lib `public.groupOrder` | UFO only. The model's `Font::groups` (M1.7): read from `groups.plist`, written to `.glyphs` as glyphsLib writes them and read back from there. |
| Right-to-left | a separate `kerningRTL` table; in v2 one table, and the doc notes `leftKerningGroup` means the right group for RTL glyphs | none: "first" is the first glyph in reading order, so for RTL the first is the right-hand glyph | UFO is direction-neutral; Glyphs is visual-side based with a table per direction. `glyphs2fontir` merges RTL pairs into the single table by swapping the group side prefixes. |
| Exceptions | glyph-versus-group pairs take precedence | same, with a defined lookup order: glyph–glyph, glyph–group, group–glyph, group–group | Equivalent. |
| Values | `f64` | integer or float; zero values should be omitted unless needed as an exception | |
| Contextual kerning `[4]` | `kerningContext {context: {masterId: value}}`; a package keeps it in `kerning.plist` | none (feature code) | The context key syntax is not described. The model keeps each context as stored (`Kerning::context`, M1.7). glyphsLib has no UFO form for it: saving as UFO reports it lost. |
| UFO 2 | — | group names without the mandatory prefixes; ufoLib and norad rename groups used as first or second pair members (and any named `@MMK_L_…`/`@MMK_R_…`) to `public.kern1.…`/`public.kern2.…` on load | The model holds the new names (ufoLib's rule, M1.7). The writer works the old names out again from the files it read; a new group gets `@MMK_L_`/`@MMK_R_`. |

### 2.12 Metrics and metrics keys

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Advance width | layer `width` | `<advance width>` | Sidebearings are derived in both. |
| Vertical metrics | layer `vertWidth`, `vertOrigin` | `<advance height>`, glyph lib `public.verticalOrigin` | |
| Metrics keys | glyph and layer `metricLeft`, `metricRight`, `metricWidth`, `metricTop`, `metricBottom`, `metricVertWidth`, `metricVertOrigin` (strings: a glyph name or a formula) | none | The stored `width` stays authoritative in the file; keys are re-evaluated by the editor. |
| Width before export tweaks | — | seen in glyph lib: `com.schriftgestaltung.Glyphs.originalWidth` (on marks whose width the converter zeroed) | A converter artefact that an importer must undo. |
| Font-level metrics | §2.3 | §2.3 | |

### 2.13 Features, classes, prefixes

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Features | `features[] {tag, code, automatic, disabled, notes, labels[] {language, value}}` `[2: name instead of tag]` | one `features.fea` text file per UFO | A structured list of snippets versus one text. `code` is the body only; the `feature tag { … } tag;` wrapper is implied. The same tag may occur more than once. |
| Classes | `classes[] {name, code, automatic, disabled, notes}` (name without `@`; code may contain comments) | `@Name = […];` statements in `features.fea` | |
| Prefixes | `featurePrefixes[] {name, code, automatic, disabled, notes}` | everything in `features.fea` outside feature blocks | |
| Automatic | `automatic` = the application regenerates the code | none; glyphsLib marks snippets with comment lines `# Prefix: <name>`, `# automatic`, `# notes:` and `# disabled` (seen in fontTools and fontc test data) | FontCraft writes and reads these markers as glyphsLib does (M1.7, `crates/io-ufo/src/features.rs`); a text is split into snippets only when writing them gives it back byte for byte, otherwise it stays one unnamed prefix. |
| Disabled | `disabled` | none (comment the code out) | glyphsLib comments out a disabled feature's code after `# disabled` and writes disabled prefixes and classes as live code; FontCraft comments those out too. |
| Stylistic-set names | `labels[]` | `featureNames { name … }` inside the feature | Written as glyphsLib writes them (Windows language ids; labels without text or with an unknown language left out); read back only from automatic features, as glyphsLib reads them. |
| Includes | not in the format document | `include(path);` resolved relative to the UFO | |
| Per master | one set for the font | one file per UFO; they normally match but need not (`fontir` models a feature source per set of locations) | |
| Package storage `[4]` | `features/<tag>.fea`, `features/@<Class>.fea`, `features/_<Prefix>.fea`; clashes get `.2`, `.3` | — | |
| Generated mark/kern code | not stored (compile time) | lib `com.github.googlei18n.ufo2ft.featureWriters`, `…filters` configure the UFO tool chain | |

### 2.14 Custom parameters, lib, user data, notes

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Typed, named settings | `customParameters[] {name, value, disabled}` on font, master, instance | the matching `fontinfo.plist` field when one exists; otherwise lib (seen: `com.schriftgestaltung.customParameter.GSFont.<name>`, `…GSFontMaster.<name>`, `…InstanceDescriptorAsGSInstance.<name>`, and a list under `com.schriftgestaltung.font.customParameters`) | Ordered, repeatable, can be disabled; a plist dictionary can do none of these. |
| Free-form data | `userData` on font, master, instance, glyph, layer; on anchor, guide, component `[≤3]`; inside `attr` for anchor, guide, component, node, image `[4]`. Empty containers are kept | lib (font), `layerinfo.plist` lib, glyph lib, `public.objectLibs`; DS `<lib>` on document, variable font, instance; `data/` for files | Keys in UFO and DS libs should be reverse-domain names; Glyphs `userData` keys are free. Seen: `com.schriftgestaltung.font.userData` holding Glyphs user data in a UFO, and `com.schriftgestaltung.Glyphs.ufoData` in a Glyphs file. |
| Notes | font `note`; glyph `note`; `notes` on features, classes, prefixes | `fontinfo.plist` `note`; GLIF `<note>` | |
| Binary data | plist data values `<hex>` | plist `<data>` (base64); files under `data/` | |

### 2.15 Images

| Concept | `.glyphs` v3/v4 key | UFO 3 + DS 5 location | Mismatch |
|---|---|---|---|
| Placed image | layer `backgroundImage {imagePath, imageURL, pos, scale, angle, slant, crop [x, y, w, h], alpha, locked}`, `[4]` `attr`; `[4]` images may also be shapes `[2: transform, crop as strings]` | `<image fileName xScale xyScale yxScale yScale xOffset yOffset color>`; the file must be a PNG inside `images/` | Reference by relative path (any image type the reference can draw) versus an embedded PNG. UFO has no crop, alpha or lock, and one image per GLIF. `imageURL` is an application bookmark and is only written by the application itself. |
| Image store | none (external files) | `images/` directory, flat, PNG only | |

## 3. Concepts that exist in only one format

What a lossless superset has to hold, and where each item travels when written to the other family.
"Seen" keys are the spellings found in the reference repositories; `org.fontcraft.*` marks keys we
would have to define because none was found (`roundtrip.ufoGlyphsLibKeys`, `roundtrip.extraIsPerFormat`).

### 3.1 Glyphs only → carried in UFO / designspace as

| Concept | Carrier |
|---|---|
| Master id, master order, icon, name parts | lib `com.schriftgestaltung.fontMasterID` (a single UFO; a designspace's source `name`), `…fontMasterOrder`, `…customParameter.GSFontMaster.iconName`, `…weight`, `…width`, `…customName` (implemented, M1.7 step 8a) |
| Custom parameters with no font-info field; `disabled` state; duplicates; order | lib `com.schriftgestaltung.customParameter.GSFont.<name>` / `…GSFontMaster.<name>`, and glyphsLib's own keys for `Color Palettes`, `meta Table`, `Don't use Production Names`, `Virtual Master` (implemented, M1.7 step 8a); disabled and duplicate entries are reported lost (an ordered `org.fontcraft.customParameters` list would hold them) |
| Instance parameters, `exports`, `visible`, `weightClass`, `isBold`/`isItalic`/`linkStyle` | DS instance attributes where they exist; `lib` → `public.fontInfo` for font-info values; the rest in the instance `lib` |
| `settings`, `DisplayStrings`, `.appVersion` | lib `com.schriftgestaltung.customParameter.GSFont.disablesAutomaticAlignment`, `…GSFont.useNiceNames`, `com.schriftgestaltung.keyboardIncrement`, `…GSFont.DisplayStrings`, `com.schriftgestaltung.appVersion` (implemented, M1.7 step 8a); the other settings have no glyphsLib key and are reported lost |
| Font, master `userData` | font: the designspace `<lib>`, or in a single UFO lib `com.schriftgestaltung.Glyphs.fontUserData`; master: the UFO lib's own keys (implemented, M1.7 steps 2 and 8a) |
| Extensible metrics with filters and overshoots, named stems, numbers | font-info fixed fields get the unfiltered values; the full lists go to lib `org.fontcraft.metrics`, `…stems`, `…numbers` |
| Glyph category, subcategory, script, case, direction, tags, lock, sort names, last change, user data | glyph lib `com.schriftgestaltung.Glyphs.category`, `…subCategory`, `…script`, `…lastChange`, and the masters' lib `…Glyphs.glyphUserData.<glyph>` (implemented, M1.7 step 8b); the others planned as `org.fontcraft.glyph.*`, reported lost until then; plus `public.openTypeCategories` derived for compilers from the categories and anchors when the font was read from another format, as glyphsLib derives it (`ufo.openTypeCategories`); a UFO's own map is written back as read |
| Colour label by index | `public.markColor` with the label colour glyphsLib uses for the index; reading that exact text gives the index back (implemented, M1.7), so no extra key is needed |
| Metrics keys (glyph and layer) | left, right and width: glyph lib `com.schriftgestaltung.Glyphs.glyph.leftMetricsKey`… and `…Glyphs.layer.leftMetricsKey`… (implemented, M1.7 step 8b); top, bottom and vertical: planned `org.fontcraft.metricsKeys`, reported lost |
| Kerning groups for top and bottom, `kerningVertical`, `kerningContext`, separate `kerningRTL` | planned: lib `org.fontcraft.kerning.vertical`, `…context`; RTL pairs are merged into `kerning.plist` with a lib marker listing which pairs came from the RTL table. Until then saving as UFO reports them lost (M1.7) |
| Backup layers, their names, ids and order | UFO layers named after the layers; `layerinfo.plist` lib `com.schriftgestaltung.layerId` (when every layer in the UFO layer has the same id) and `…layerOrderInGlyph.<glyph>`; a UFO layer's lib as master user data `…Glyphs.layerLib.<layer>` (implemented, M1.7 steps 5b and 8b). Per-glyph layer ids have no glyphsLib key and are reported lost |
| Intermediate layers | UFO layer named by the coordinates + DS `<source layer>` named `<master id> <layer>`, as glyphsLib writes it (implemented, M1.7 step 8c); read back as intermediate layers when in that form. A single UFO holds the layer without its location (reported) |
| Alternate layers | generated `<glyph>.BRACKET.varAltNN` glyphs in every master's UFO, with the layer's name in the GLIF lib `com.schriftgestaltung.Glyphs._originalLayerName`, + DS `<rules>`, as glyphsLib writes them (implemented, M1.7 step 8c); read back as alternate layers when in that form. A single UFO leaves them out (reported) |
| Colour palette layers | UFO layers `color.<index>`, and on the master layer's GLIF `com.github.googlei18n.ufo2ft.colorLayerMapping` (implemented, M1.7 steps 5b and 8b) |
| Layer background of non-master layers; annotations | backgrounds: UFO layers `<layer>.background` (implemented, M1.7 step 5b); annotations: glyph lib `com.schriftgestaltung.annotations` (implemented, step 8b; a background's are reported lost) |
| Shape order; component `anchor` | glyph lib `com.schriftgestaltung.Glyphs.shapeOrder`, `…ComponentInfo` (implemented, M1.7 step 8b) |
| Component alignment, `masterId`, lock, orientation; smart `piece` values | alignment in `…ComponentInfo`, locks and smart values in `com.schriftgestaltung.componentsLocked` and `…componentsSmartComponentValues` (implemented, M1.7 step 8b; smart components stay components); `masterId` and orientation planned as an object lib `org.fontcraft.component`, reported lost |
| Smart glyph definition and poles | glyph lib `com.schriftgestaltung.smartComponentAxes` (implemented, M1.7 step 8b); version 2 poles travel in layer user data `PartSelection`; version 3 and 4 poles planned as `org.fontcraft.smart`, reported lost |
| Corner, cap, segment, brush components; node-attached hints; TT instructions as authored | glyph lib `com.schriftgestaltung.hints` in glyphsLib's form, index paths as `.glyphs` counts them (implemented, M1.7 step 8b; hint `settings` and a background's hints reported lost); optionally decomposed outlines for other tools |
| Path and component drawing attributes, gradients, shape groups, node flags, node `hoi` data, v4 curve types | node user data and other node attribute keys: glyph lib `com.schriftgestaltung.Glyphs.nodeUserData.<path>.<node>` (implemented, M1.7 step 8b); anchor user data: `public.objectLibs` under the anchor's identifier (step 8b); the rest planned as object libs `org.fontcraft.shape`, `org.fontcraft.node`; v4 curve types additionally need a cubic approximation as the visible outline |
| Guide filter, measurement, lock, circle and rectangle guides | object lib `org.fontcraft.guide`; non-line guides are omitted from the visible guideline list |
| Feature structure (`automatic`, `disabled`, `notes`, `labels`, snippet boundaries) | comment markers in `features.fea` as glyphsLib writes them (`# Prefix: <name>`, `# automatic`, `# notes:`, `# disabled`, `featureNames`; implemented, M1.7). Notes of prefixes and classes have no marker and are reported lost |
| Image crop, alpha, lock, original path and type | crop, alpha, lock: glyph lib `com.schriftgestaltung.backgroundImage.crop`, `…alpha`, `…locked` (implemented, M1.7 step 8b); original path and type planned as `org.fontcraft.image` |
| Long axis tags, glyph-local axes, `fontType` | DS document `lib` (planned). `Virtual Master` travels in the UFOs' libs (M1.7 step 8a) and `Variable Font Origin` as the designspace's default source (step 3c) |

### 3.2 UFO / designspace only → carried in `.glyphs` as

| Concept | Carrier |
|---|---|
| Font-info fields with no Glyphs property or parameter (WOFF metadata, FOND, `year`, PostScript legacy fields, family blues) | custom parameters under the UFO key name (the reference already accepts many UFO spellings as parameter names) or font `userData` |
| Font-info values that differ per master where Glyphs has one font-level value | master custom parameter or master `properties[]` override |
| Float `unitsPerEm` | rounded value in `unitsPerEm`, exact value in `userData` (loss report) |
| Point `name`, point/contour/component/anchor/guideline `identifier`, object libs | node `attr.userData` (nodes), `userData`/`attr` of the object; contour identifiers in the path's `attr` |
| GLIF file names, layer directory names, `layerinfo.plist` colour and lib | font `userData` → `org.fontcraft.ufo` |
| Non-kerning groups | classes after the feature classes, named in font `userData` `com.schriftgestaltung.Glyphs.groupsNotInFeature`, as glyphsLib writes them (implemented, M1.7) |
| Kerning group member order, members not in the font, empty kerning groups, per-master group differences | font or master `userData` → `org.fontcraft.ufo.groups` (planned) |
| Primary code point order; code points, note, mark colour or lib that differ between masters' GLIFs | layer `userData` on the master layer that differs (planned; for now they stay in that layer's `Extra`, written back to the UFO and counted as lost when saving `.glyphs`) |
| Anchor and guideline colour; unnamed anchors | `userData` / `attr` of the object |
| `public.postscript.hints`, `public.truetype.instructions`, `public.truetype.overlap`, `public.verticalOrigin` (when not equal to `vertOrigin` semantics), `public.unicodeVariationSequences`, `public.openTypeMeta`, any foreign lib key | layer or font `userData` under the same key name, so that writing UFO again restores it verbatim |
| `data/` directory content | not embeddable in a text plist at sensible size: kept beside the Glyphs file only in the native format; loss report when saving as `.glyphs` |
| Embedded PNG images | written next to the Glyphs file in a subfolder and referenced by `imagePath` |
| Axis minimum and maximum, `<map>` nodes | `Axis Mappings` custom parameter, listing every axis (one without a map gets the identity over its range), as glyphsLib does (implemented, M1.7) |
| Discrete axes, avar 2 `<mappings>`, variable-font axis subsets, STAT labels with ranges and linked values, location labels | discrete values, localised axis names and STAT labels are axis fields of the model, but are not written to `.glyphs` yet (the loss report lists them); planned: font `userData` → `org.fontcraft.designspace`, the parts Glyphs can express (STAT names, variable settings) also written natively |
| Rules that swap unrelated glyphs (not expressible as alternate layers of one glyph) | font `userData`, plus feature code when a fixed feature is acceptable |
| DS `filename` paths, source and instance `name`, deprecated MutatorMath elements, the three libs | an instance's `filename` is its `UFO Filename` parameter and its lib its user data, a source's `name` the master id (implemented, M1.7); the rest: font, master and instance `userData` → `org.fontcraft.designspace` (planned) |
| Quadratic contours made only of off-curve points | stored as `q`/`o` nodes if the reference accepts it; otherwise an explicit on-curve is inserted and flagged (Open questions) |

### 3.3 Consequences for the model

1. One document holds all masters; a UFO family is merged on load and split on save, never the reverse.
2. Layers belong to glyphs and carry combinable attributes; UFO's font-wide layers are a serialisation detail kept in `extra`.
3. Paths and components share one ordered list; nodes need a flags field wider than "type + smooth", an optional name and an optional identifier.
4. Components store both the decomposed transform and, when read from a matrix, the original matrix.
5. Kerning is three direction tables keyed by master, group membership is a per-glyph attribute, and a side list of "UFO group facts" preserves what the attribute cannot say.
6. Font info is an ordered, scoped parameter list with typed accessors on top, not a struct of fixed fields; UFO font-info keys are parameter names.
7. Axes need explicit optional minimum, maximum, maps, labels and a discrete value list in addition to the Glyphs fields.
8. Every struct needs an `extra` map tagged by format family, and unknown values must keep their original token spelling.

## 4. What the Rust crates cover

### 4.1 `glyphs-reader` 1.0.0 (fontc; MIT/Apache-2.0)

Purpose: feed a compiler. It parses into private `Raw*` structs through a derive macro
(`ascii_plist_derive::FromPlist`), upgrades version 2 to the version 3 shape, then builds a reduced,
normalised public `Font`.

- **Versions.** No `.formatVersion` → version 2; `3` → version 3; any other value is a parse error
  ("'3' is the only known format version"), so **version 4 files are rejected**.
- **Packages.** Reads `fontinfo.plist`, `glyphs/*.glyph` and `order.plist`; ignores `UIState.plist`. It
  does not read `kerning.plist`, `note.md` or `features/*.fea`, the version 4 package files.
- **Kept.** Units per em, axes (name, tag, hidden), masters (id, name, axis values, metric values,
  number values, user data, a fixed set of custom parameters), instances (name, active, bold/italic,
  single or variable, axis values, user-to-design mappings, properties, the same parameter set), glyph
  order, glyphs (name, export, code points, left/right kerning group, category, subcategory, production
  name, smart-glyph axes), layers (ids, width, vertical width and origin, shapes, anchor name and
  position, `coordinates`, `axisRules`, `color`, `colorPalette`, smart poles, corner/cap hints), paths
  (closed, nodes with type and smooth only, fill colour and gradient), components (name, a composed
  affine, anchor, smart values), features/classes/prefixes flattened to code snippets, names, LTR and
  RTL kerning, version, date, font user data.
- **Dropped.** Guides, annotations, backgrounds, images, glyph note/colour/tags/lock/last change/
  script/case/direction/sort names, all metrics keys, top and bottom kerning groups, vertical and
  contextual kerning, stems, PostScript and TrueType hints (only type, name, origin, scale and options
  are parsed, for corner and cap components), node attributes and user data, component alignment, lock,
  orientation and `masterId`, strokes, shadows, masks and the other shape attributes, layer name,
  visibility, colour label and user data, backup layers, master guides, icon and stems, settings,
  display strings, `.appVersion`, font note, and every custom parameter not on its list (a warning is
  logged). Node flag strings other than `l`, `ls`, `c`, `cs`, `q`, `qs`, `o` are a parse error, so the
  documented `R`, `C` and `X` suffixes are not accepted.
- **Rewritten on load.** Smart components are instantiated, corner components are inserted, bracket
  layers are aligned across component users and reverse bracket setups are normalised, version 2 layer
  names are turned into attributes. The result is no longer the document the user saved.
- **Writing.** `Plist::to_string` serialises the generic property-list enum (marked unused), and
  `Plist::parse` reads one, but there is no conversion from `Font` (or the raw structs) back to a
  property list. The crate **cannot write a `.glyphs` file**.

Use for us: an oracle for compile-facing interpretation (kerning sides, bracket layers, transform
order, version 2 upgrade rules) and a source of test files; not a base for the editor's reader.

### 4.2 `norad` 0.19.0 (linebender; MIT/Apache-2.0)

- **UFO.** Reads versions 1, 2 and 3 and up-converts older ones in memory (font info, kerning group
  prefixes, features and lib moves); writes **version 3 only** (a downgrade is an error). Covers
  metainfo, the full `fontinfo.plist` with validation, groups, kerning (with a lookup resolver), features
  as one string, lib, layer contents and layer info, GLIF 1 and 2 with all elements, identifiers and
  object libs, `images/` and `data/` stores, selective loading, optional `rayon` parallelism, optional
  `kurbo` conversion.
- **UFO ZIP.** Reading through a zip source behind the `ufoz` feature; saving goes through a `FontSink`
  trait for which a directory and a closure implementation are provided, so a zip writer is ours to add.
- **Designspace.** Reads and writes format 5.x: axes with maps, label names and STAT labels, discrete
  values, `axis_mappings` with descriptions, location labels, rules with processing order, sources with
  layer and localised family names, variable fonts with axis subsets and lib, instances with localised
  names, location label reference and lib, document lib. Decimal values are written exactly.
- **Misses for an editor.**
  - The GLIF parser is strict: an unknown element or attribute is an error, not preserved data.
  - `public.objectLibs` is managed internally; a lib that already contains that key makes saving fail.
  - `Font::save` replaces the target directory rather than writing atomically or only changed files
    (its documentation recommends saving elsewhere and moving).
  - Default output indentation is a tab; indentation and quote style are configurable but not
    remembered per file.
  - The deprecated MutatorMath designspace elements are not modelled, so they are lost on rewrite.
  - No UFO 2 writer; original UFO 2 group names are not kept after up-conversion.
  - One `Font` is one UFO; nothing merges masters.

Use for us: the UFO and designspace reader/writer layer, behind our own model, plus thin additions
(zip sink, change-only atomic save, UFO 2 writer, preserved deprecated elements). The strict GLIF
parser needs either an upstream option or our own tolerant GLIF reader.

**Decided 2026-10-08 (decision 0003, amended):** not used as the UFO layer. A spike found that it
writes no corpus UFO or designspace file back unchanged (`docs/plans/M1.md` § 8.2); FontCraft
has its own reader and writer in `crates/io-ufo`, and norad reads FontCraft's output in tests.

### 4.3 `fontir` 1.0.0 (fontc; with `glyphs2fontir`, `ufo2fontir` 1.0.0, `fontra2fontir` 0.5.0)

A compiler's intermediate representation, produced through a `Source` trait whose methods create
units of work: static metadata, global metrics, one job per glyph, features, kerning locations,
kerning per location, colour palettes, colour glyphs.

- **Shape.** `StaticMetadata` (units per em, axes, avar 2 mappings, named instances, variation model,
  names keyed by name id, PostScript names, italic angle, misc OS/2-style values, feature variation
  rules, per-location number values); `GlobalMetrics` per normalised location; `Glyph { name,
  emit_to_binary, codepoints, sources: location → GlyphInstance }` with `GlyphInstance { width, height,
  vertical_origin, contours: Vec<BezPath>, components }`; `Component { base, transform, anchor }`;
  `GlyphAnchors` with classified anchor kinds; `KerningInstance { location, kerns, groups }` with
  `KernGroup::Side1/Side2` (groups are per source because UFO masters may group differently);
  `FeaturesSource` as file or memory text, optionally per master; `ColorPalettes`, `ColorGlyphs` with
  paint trees.
- **What it tells us.** A compile-facing model is keyed by normalised location, not by master id;
  outlines are plain Bézier paths with no node types or flags; everything is already resolved (smart
  and corner components applied, alternate layers turned into rules and extra glyphs, RTL kerning
  merged, anchors propagated).
- **Misses for an editor.** No layers other than those that compile, no backgrounds, guides, hints,
  metrics keys, notes, user data, parameters as authored, custom-parameter order, disabled items, or
  anything needed to write a source file. It is one-way by design.
- **`fontra2fontir`.** Reads a `.fontra` directory's JSON and CSV files; most `Source` methods are
  still `todo!()`. Not usable yet and not needed for our formats.

Use for us: the editor model gets a `to_ir()` projection (or implements `Source`) so that fontc can be
the compiler or the oracle; `fontir` is not the editing model.

### 4.4 Does `.glyphs` writing exist in Rust?

- **Not in fontc.** `glyphs-reader` is read-only as described above.
- **Yes, outside our reference set.** `glyphslib` 0.2.8 (MIT OR Apache-2.0, repository
  `simoncozens/glyphslib-rs`) describes itself as reading, writing and manipulating Glyphs sources; its
  documentation shows `Font::load` for `.glyphs` and `.glyphspackage`, separate `glyphs2` and `glyphs3`
  modules, an upgrade from 2 to 3, and writing by serialising the typed structs with
  `openstep_plist::to_string` (crate `openstep-plist` 1.1.0, same repository). `babelfont` 0.2.2 builds
  a converter on it and lists Glyphs 2 and 3 as load and save, UFO and designspace as load only.
  **None of these mention format version 4.** We have read only their crates.io and docs.rs pages, not
  their source: whether they keep unknown keys, reproduce the reference's bytes, or write packages is
  unverified.
- **Recommendation.** Write our own `.glyphs` reader and writer over our own model, because the
  commitments in `roundtrip.noOpSaveIsByteIdentical`, `roundtrip.unknownKeysPreserved` and
  `roundtrip.glyphsVersionDifferences` (including version 4) are stricter than a general-purpose
  library is likely to guarantee. Evaluate `openstep-plist` as the tokenizer and serializer layer, and
  use `glyphslib` and `glyphs-reader` as cross-check oracles in tests. The published JSON schemas give
  the key set, types, defaults, version conditions and the compact-tuple marker, so the per-key
  serialisation table can be generated from them rather than typed by hand.

## Recommended model

**Closest to: `.glyphs` version 3, extended with the version 4 additions.** Reasons:

- It is the only one of the three that is a single document for a whole family, which is what the
  editor shows and what undo has to snapshot.
- It is the richer side on almost every row of section 2; mapping UFO + designspace into it is a merge
  with a short carry list (§3.2), while mapping it into a UFO-shaped model would push most of the
  primary reference's data into untyped lib dictionaries.
- Byte-identical `.glyphs` round trips are the hardest promise to keep; a model that mirrors that
  format's structure makes the writer a direct walk.

Borrowed from UFO and designspace where they are more explicit: optional node names and identifiers,
object-level `extra`, explicit axis ranges, maps, labels and discrete values, variable-font axis subsets,
font-level substitution rules, an embedded data and image store, and groups-as-lists as a side table.
Borrowed from `fontir`: nothing structural; the model offers a resolved projection for compilers.

Sharing for cheap undo: a `Font` value is a tree of `Arc`s. An undo snapshot is `font.clone()`. Edits
go through `Arc::make_mut`, which copies only the nodes on the path from the root to the edited layer:
the glyph table (a vector of pointers), one `Glyph`, one `Layer`. Everything else is shared between
snapshots.

```rust
use std::sync::Arc;
use indexmap::IndexMap;
use smol_str::SmolStr;

// ── shared vocabulary ──────────────────────────────────────────────────────────────────────────

pub type GlyphName = SmolStr;
pub struct MasterId(SmolStr);          // opaque; a Glyphs master id, or minted for a UFO source
pub struct LayerId(SmolStr);           // equals the MasterId for a master layer
pub struct Identifier(SmolStr);        // UFO object identifier, kept verbatim

/// A property-list / JSON value that remembers how it was spelled.
pub enum Value {
    Dict(IndexMap<SmolStr, Value>),    // insertion order kept; writers sort where their format does
    Array(Vec<Value>),
    String(Arc<str>),
    Number(Number),                    // f64 or i64 plus the original token text, if read from a file
    Bool(bool),
    Data(Arc<[u8]>),
}

pub enum Family { Glyphs, Ufo, Designspace, Binary, Native }

/// Unknown keys, per struct. Empty for almost every object, so it is one pointer wide.
pub struct Extra(Option<Box<IndexMap<(Family, SmolStr), Value>>>);

/// Custom parameters and font-info values: ordered, repeatable, can be switched off.
pub struct Param { pub name: SmolStr, pub value: Value, pub disabled: bool, pub extra: Extra }
pub struct Params(Vec<Param>);         // typed getters/setters are generated from the parameter inventory

/// Glyphs `properties`: a single value or a list of localised values.
pub struct Property { pub key: SmolStr, pub values: Vec<(LangTag, Arc<str>)>, pub extra: Extra }

pub struct UserData(IndexMap<SmolStr, Value>);   // Glyphs userData and UFO lib share this type

// ── document ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Font {
    pub origin: Arc<Origin>,                       // which format, version, package flag, appVersion, paths
    pub info: Arc<FontInfo>,
    pub axes: Arc<Vec<Axis>>,
    pub metrics: Arc<MetricDefs>,                  // font-level metric, stem and number definitions
    pub masters: Arc<Vec<Arc<Master>>>,
    pub instances: Arc<Vec<Arc<Instance>>>,
    pub glyphs: Arc<GlyphTable>,                   // ordered: this order IS the glyph order
    pub kerning: Arc<Kerning>,
    pub features: Arc<Features>,
    pub rules: Arc<Vec<Rule>>,                     // designspace-style glyph swaps not tied to one glyph
    pub labels: Arc<Vec<LocationLabel>>,           // STAT format 4
    pub groups: Arc<UfoGroups>,                    // non-kerning groups + facts kern attributes cannot hold
    pub files: Arc<FileStore>,                     // UFO data/ and images/, by relative path, lazily loaded
    pub ui: Arc<UiState>,                          // display strings; saved, but never dirties the document
    pub extra: Extra,
}

pub struct GlyphTable { order: Vec<Arc<Glyph>>, by_name: HashMap<GlyphName, u32> }

pub struct FontInfo {
    pub units_per_em: f64,
    pub version: (u32, u32),
    pub date: Option<Timestamp>,
    pub properties: Vec<Property>,                 // names, localised
    pub params: Params,                            // OS/2, hhea, glyphOrder override, … and UFO-only fields
    pub settings: Settings,                        // grid, increments, alignment and naming switches, fontType
    pub note: Arc<str>,
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct Axis {
    pub name: SmolStr,
    pub tag: SmolStr,                              // may be longer than four characters (Glyphs)
    pub hidden: bool,
    pub default: Option<f64>,
    pub range: Option<AxisRange>,                  // Continuous{min,max} | Discrete(values); None = derive
    pub map: Vec<(f64, f64)>,                      // user → design
    pub names: Vec<(LangTag, Arc<str>)>,
    pub labels: Vec<AxisLabel>,                    // STAT formats 1–3
    pub ordering: Option<u32>,
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct MetricDefs {
    pub metrics: Vec<MetricDef>,                   // {kind, name, filter, horizontal}
    pub stems:   Vec<StemDef>,                     // {name, horizontal, filter}
    pub numbers: Vec<NumberDef>,                   // {name}
}

pub struct Master {
    pub id: MasterId,
    pub name: SmolStr,
    pub location: Vec<DesignCoord>,                // parallel to Font::axes; DesignCoord allows (x, y)
    pub metric_values: Vec<MetricValue>,           // {pos, over}; parallel to MetricDefs::metrics
    pub stem_values: Vec<f64>,
    pub number_values: Vec<f64>,
    pub guides: Vec<Guide>,                        // global guides
    pub properties: Vec<Property>,
    pub params: Params,                            // per-master overrides, incl. differing UFO font info
    pub icon: Option<SmolStr>,
    pub visible: bool,
    pub active: bool,
    pub source: Option<SourceRef>,                 // UFO path, layer name, designspace name/family/style
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct Instance {
    pub id: Option<SmolStr>,
    pub kind: InstanceKind,                        // Static | Variable { axis_subsets } | Icon | Particles
    pub name: SmolStr,
    pub location: Vec<InstanceCoord>,              // Design(x[, y]) | User(v) per axis, or a label name
    pub interpolation: Option<IndexMap<MasterId, (f64, f64)>>,   // manual master weights
    pub exports: bool,
    pub visible: bool,
    pub weight_class: Option<u16>,
    pub width_class: Option<u16>,
    pub style_link: StyleLink,                     // is_bold, is_italic, link_style
    pub properties: Vec<Property>,
    pub params: Params,
    pub file_name: Option<Arc<str>>,               // designspace filename, kept verbatim
    pub user_data: UserData,
    pub extra: Extra,
}

// ── glyphs and layers ──────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Glyph {
    pub name: GlyphName,
    pub unicodes: Vec<u32>,                        // source order kept; writers sort where required
    pub layers: Vec<Arc<Layer>>,                   // master layers and all others, in file order
    pub export: bool,
    pub production: Option<SmolStr>,
    pub category: Option<SmolStr>,
    pub sub_category: Option<SmolStr>,
    pub script: Option<SmolStr>,
    pub case: Option<Case>,
    pub direction: Option<Direction>,
    pub kern_groups: SideGroups,                   // left, right, top, bottom
    pub metric_keys: MetricKeys,                   // left, right, width, top, bottom, vert_width, vert_origin
    pub color: Option<ColorLabel>,                 // Index(u8) | Color
    pub tags: Vec<SmolStr>,
    pub note: Arc<str>,
    pub locked: bool,
    pub last_change: Option<Timestamp>,
    pub sort_name: Option<SmolStr>,
    pub sort_name_keep: Option<SmolStr>,
    pub smart: Option<Box<SmartGlyph>>,            // local axes / parts settings
    pub user_data: UserData,
    pub extra: Extra,                              // incl. UFO file name per master
}

#[derive(Clone)]
pub struct Layer {
    pub id: LayerId,
    pub master: MasterId,                          // the associated master (itself for a master layer)
    pub name: SmolStr,                             // stored name only; display name is derived from role()
    pub attrs: LayerAttrs,
    pub width: f64,
    pub vert_width: Option<f64>,
    pub vert_origin: Option<f64>,
    pub content: LayerContent,
    pub background: Option<Arc<LayerContent>>,
    pub metric_keys: MetricKeys,
    pub color: Option<ColorLabel>,
    pub visible: bool,
    pub active: bool,
    pub user_data: UserData,
    pub extra: Extra,                              // incl. UFO layer name, per-layer unicodes/note if they differ
}

/// Combinable, exactly as in the Glyphs `attr` dictionary. `Layer::role()` derives
/// Master | Backup | Intermediate | Alternate | Color… for the UI.
pub struct LayerAttrs {
    pub coordinates: Option<Vec<Option<f64>>>,     // intermediate layer; None entry = "use the master's value"
    pub axis_rules: Vec<AxisRule>,                 // alternate layer; {min, max} per axis
    pub color: Option<ColorLayer>,                 // Palette(index | Foreground) | FullColor | Sbix(size) | Svg
    pub smart_poles: IndexMap<SmolStr, Pole>,      // smart glyph: property → Bottom | Top
    pub extra: Extra,
}

pub struct LayerContent {
    pub shapes: Vec<Shape>,
    pub anchors: Vec<Anchor>,
    pub guides: Vec<Guide>,
    pub hints: Vec<Hint>,
    pub annotations: Vec<Annotation>,
    pub image: Option<Box<Image>>,
}

// ── shapes ─────────────────────────────────────────────────────────────────────────────────────

pub enum Shape {
    Path(Path),
    Component(Component),
    Image(Box<Image>),                             // Glyphs 4
    Group(ShapeGroup),                             // Glyphs 4
}

pub struct Path {
    pub closed: bool,
    pub nodes: Vec<Node>,                          // closed paths start at nodes[0] (see roundtrip.startNodeConvention)
    pub locked: bool,
    pub attrs: Option<Box<ShapeAttrs>>,            // fill, stroke, gradient, caps, joins, mask, shadows, group, …
    pub id: Option<Identifier>,
    pub extra: Extra,
}

/// 32 bytes: two f64, three one-byte fields and one pointer; rare data lives behind `more`.
pub struct Node {
    pub x: f64,
    pub y: f64,
    pub kind: NodeKind,                            // Line | Curve | QCurve | OffCurve | Quartic | Hobby | Spiral
    pub join: Join,                                // Corner | Smooth | Tangent
    pub flags: NodeFlags,                          // LOCKED, ORIENT_RIGHT, ORIENT_CENTER
    pub more: Option<Box<NodeMore>>,               // name, identifier, hoi data, user data, extra
}

pub struct Component {
    pub base: GlyphName,
    pub transform: Decomposed,                     // pos, scale, angle (degrees), slant; + original matrix if read from one
    pub alignment: Alignment,                      // Disabled(-1) | Default(0) | Forced(1) | Horizontal(3)
    pub anchor: Option<SmolStr>,                   // attach-to anchor
    pub smart_values: IndexMap<SmolStr, f64>,      // Glyphs `piece`
    pub from_master: Option<MasterId>,
    pub locked: bool,
    pub orientation: Orientation,
    pub traverse_anchors: bool,
    pub tt_flags: TtComponentFlags,                // use-my-metrics, round-to-grid (UFO object lib)
    pub attrs: Option<Box<ShapeAttrs>>,
    pub id: Option<Identifier>,
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct Anchor {
    pub name: SmolStr,                             // empty = unnamed (UFO)
    pub x: f64,
    pub y: f64,
    pub locked: bool,
    pub orientation: Orientation,
    pub color: Option<Color>,
    pub id: Option<Identifier>,
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct Guide {
    pub kind: GuideKind,                           // Line | Circle | Rect
    pub x: f64,
    pub y: f64,
    pub angle: f64,                                // model convention: degrees counter-clockwise from the x axis
    pub size: (f64, f64),
    pub length: f64,
    pub name: SmolStr,
    pub locked: bool,
    pub lock_angle: bool,
    pub show_measurement: bool,
    pub filter: Option<Arc<str>>,
    pub orientation: Orientation,
    pub color: Option<Color>,
    pub id: Option<Identifier>,
    pub user_data: UserData,
    pub extra: Extra,
}

pub struct Hint {
    pub kind: HintKind,   // Stem | TopGhost | BottomGhost | Flex | TtStem | TtShift | TtSnap | TtInterpolate
                          // | TtDiagonal | TtDelta | Corner | Cap | Segment | Brush | Head | Tag | Auto | Unknown(SmolStr)
    pub horizontal: bool,
    pub origin: Option<NodeRef>,
    pub target: Option<NodeRef>,
    pub other1: Option<NodeRef>,
    pub other2: Option<NodeRef>,
    pub place: Option<(f64, f64)>,                 // position, width — for hints not attached to nodes
    pub stem: Option<i32>,
    pub name: SmolStr,                             // glyph name for corner/cap/segment/brush
    pub scale: (f64, f64),
    pub options: u32,
    pub settings: Option<Value>,
    pub extra: Extra,
}

/// Glyphs index paths, kept as indices so files round-trip; editing operations renumber them.
pub enum NodeRef {
    Node { path: u32, node: u32 },
    Inflection { path: u32, node: u32, index: u32 },
    Intersection { path1: u32, node1: u32, path2: u32, node2: u32 },
    Lsb, Rsb, Up, Down,
}

// ── kerning and features ───────────────────────────────────────────────────────────────────────

pub struct Kerning {
    pub ltr: IndexMap<MasterId, Arc<PairTable>>,
    pub rtl: IndexMap<MasterId, Arc<PairTable>>,
    pub vertical: IndexMap<MasterId, Arc<PairTable>>,
    pub context: IndexMap<SmolStr, IndexMap<MasterId, f64>>,   // Glyphs 4, key kept verbatim
    pub extra: Extra,
}
pub struct PairTable(IndexMap<(KernSide, KernSide), f64>);     // file order kept
pub enum KernSide { Glyph(GlyphName), Group(SmolStr) }         // group name without any prefix

pub struct UfoGroups {
    pub plain: IndexMap<SmolStr, Vec<GlyphName>>,              // non-kerning groups
    pub kern_member_order: IndexMap<(Side, SmolStr), Vec<GlyphName>>,  // order, absent glyphs, empty groups
    pub per_master: IndexMap<MasterId, Box<UfoGroups>>,        // only where a UFO differed from the rest
}

pub struct Features {
    pub prefixes: Vec<Snippet>,
    pub classes: Vec<Snippet>,
    pub features: Vec<Snippet>,
    pub per_master: IndexMap<MasterId, Arc<str>>,              // only where a UFO's features.fea differed
    pub extra: Extra,
}
pub struct Snippet {
    pub name: SmolStr,                             // feature tag, class name without @, or prefix name
    pub code: Arc<str>,
    pub automatic: bool,
    pub disabled: bool,
    pub notes: Arc<str>,
    pub labels: Vec<(LangTag, Arc<str>)>,
    pub extra: Extra,
}
```

Notes on the outline:

- **Native file.** `fontcraft` is this tree as JSON, field for field, with `formatVersion`; `Arc`
  sharing is invisible in the file. `Extra` entries are written as `{"family", "key", "value"}` so
  nothing is lost between formats (`roundtrip.extraIsPerFormat`).
- **Parallel lists.** Master values indexed by axis, metric, stem and number position follow Glyphs so
  the writer is direct. All structural edits to those definitions go through model functions that fix
  every dependent list (`roundtrip.glyphsDependentValues`); nothing else may push to them.
- **Derived data is not stored.** Sidebearings, display names of layers, resolved component positions
  under automatic alignment, the kerning group member lists, and decomposed smart or corner shapes are
  computed (and cached outside the model). The one exception is the stored component position, which
  both formats write even when alignment makes it derivable.
- **Angles.** One convention inside the model (counter-clockwise, y up, as UFO); readers and writers
  convert once the Glyphs sign is confirmed.
- **Numbers.** `Value::Number` keeps the token text for unknown data so `1.0` does not become `1`;
  modelled fields are `f64` and use `roundtrip.floatFormatting`.
- **Compiler view.** `Font::resolve(target)` produces the resolved form of section 4.3 for one
  master or an instance at a master (M2.1; location-keyed for variable fonts in M9); nothing in
  the compiler reads `Extra`.
- **Size.** `Node` stays at 32 bytes and `Extra`/`UserData` at one pointer when empty, which matters
  for CJK-sized families with every master in memory.

## Values a source leaves out

Where a file leaves a value out, the readers fill what the reference compilers read (glyphsLib and
ufo2ft for fontmake, fontc's readers), checked with the oracle harness (`docs/plans/M2.md` › M2.6).

| Case | glyphsLib / ufo2ft | fontc | FontCraft |
|---|---|---|---|
| A `.glyphs` layer without `width` | 600 | 600 | 600, versions 2 to 4 |
| Version 2's top-level `copyright`, `designer`, `designerURL`, `manufacturer`, `manufacturerURL` in a version 3 or 4 file | names | names, where the font has no such property | the property, where the font has none; the key stays and is written back where it was, and the property is left out while the key holds the same text |
| A designspace source without a metric another source has | ufo2ft's default for that UFO (0.8, −0.2, 0.5, 0.7 em) | the same: it builds that UFO alone | absent when no later metric is stated, so the compiler uses the same default; 0 when the source states a later metric, since a master's value list has no gaps |
| A GLIF without a height, in a UFO with the three `openTypeVheaVertTypo…` keys | 0 (glyphsLib's crosswalk reads it as `vertWidth` 0) | 0 | 0, written back as no height |
| A GLIF whose height is the default (typographic ascender minus descender) | no `vertWidth` (glyphsLib) | the height | no height in the model; the compiler writes the default, the same number |

## Open questions

1. **Angle signs in Glyphs files.** The format document calls component, guide, image and annotation
   angles "degrees clockwise", yet `glyphs-reader` feeds the component angle to a standard
   counter-clockwise rotation and matches fontTools. Check a rotated component and an angled guide in
   the reference app against the file. The same for `slant`: degrees (as `glyphs-reader` assumes) or a
   factor (as the document's word "factor" suggests)?
2. **Byte-level details of the reference writer** that the document leaves open: the exact rule for
   when a string is written bare or quoted, which strings keep literal tabs and line feeds, whether
   floats are trimmed or fixed to a number of decimals, how `.appVersion` affects reading, and how
   glyph names map to `.glyph` file names in a package (case clashes, reserved characters).
3. **Version 4 semantics** that the schema names but does not explain: glyph `group`/`groupIdx`, the
   key syntax of `kerningContext`, `settings.dependencies`, where the instance style name lives now
   that `name` is gone, the meaning of master and layer `active`, shape-group nesting, node types `u`,
   `h`, `r` (control-point counts) and the `hoi` records. The document is under three months old and
   may still change.
4. **Property keys.** The full list of `properties[]` keys the reference writes and which are
   localisable; our list comes from `glyphs-reader` only.
5. **Quadratic paths in Glyphs.** Does the reference accept several consecutive off-curve nodes before
   a `q` node, and a closed path with no on-curve node, as UFO and TrueType allow?
6. **Kerning group prefixes in files.** `@MMK_L_`/`@MMK_R_` are confirmed by fontc and test files, not
   by the format document. Confirm they are unchanged in versions 3 and 4, how `kerningRTL` and
   `kerningVertical` name group sides, and how the top and bottom groups are prefixed.
7. **Converter lib keys.** The `com.schriftgestaltung.*` spellings in section 3 come from test data,
   without a specification. Before fixing our UFO carrier keys, read the glyphsLib (Apache-2.0)
   builder constants, which were not in the reference set, and decide which keys we adopt verbatim and
   which go under `org.fontcraft.`. Also confirm what the reference's own UFO export writes.
8. **Undocumented Glyphs files.** `.glyphsproject` and the proprietary Metrics File have no public
   description; both need sample files before a reader can be specified. Do we need our own project
   file, or are projects just extra instance lists inside a `.fontcraft` file?
9. **FontLab VFJ.** The task assumed a documented JSON format; FontLab's help pages give no schema and
   none was found. **Answered 2026-10-11 (`docs/plans/M13.md` § 7 question 2):** the `vfj` row is
   dropped. FontLab exports UFO and designspace, which FontCraft reads.
10. **Existing Rust writers.** Audit `glyphslib` 0.2.8 / `openstep-plist` 1.1.0 (unknown-key handling,
    byte fidelity against the reference, package writing, licence of bundled data) before deciding how
    much of our `.glyphs` layer is our own code.
11. **norad gaps.** Will upstream accept a tolerant GLIF mode, caller-managed `public.objectLibs`, and
    preserved deprecated designspace elements, or do we carry a patched reader for those three?
12. **UFO 2 writing.** The reference keeps a UFO's version on save. Is that worth a writer of our own,
    or is "UFO 2 opens, saves as UFO 3 after a prompt" acceptable?
13. **Glyph table sharing.** `Arc<Vec<Arc<Glyph>>>` copies one pointer per glyph on the first edit
    after a snapshot (about half a megabyte for 65 000 glyphs). Measure before choosing a chunked or
    persistent vector instead.
14. **Hints addressed by index.** Keeping Glyphs index paths means every node insertion or deletion
    must renumber hints. The alternative is stable node ids in memory with indices only in files;
    decide when the hinting milestones are specified.
15. **Native package flavour.** Large families and version control favour one file per glyph (as
    `.glyphspackage` and UFO do). Should `.fontcraft` have a directory flavour from the start?
16. **UFO formatting defaults.** We assumed fontTools output as the canonical layout; confirm its
    indentation and attribute order on real files, since norad's default (tab) differs.
17. **Milestones.** Variable-font export rows are placed in M9 and collection export in M12 although
    the general rule was "binary export in M2"; M2's title covers static fonts only. Confirm.
