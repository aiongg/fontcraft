# 31 glyphsLib crosswalk: `.glyphs` ↔ UFO and designspace

What glyphsLib 6.15.0 (Apache-2.0) does when it converts a `.glyphs` document into UFOs and a
designspace document, and back. FontCraft's UFO reader and writer follow it (M1.7), so a UFO made
from a `.glyphs` document reads in fontmake as one glyphsLib made; where glyphsLib loses data, we
may keep our own carriers instead, and say so in the code.

The tables restate behaviour in our own words; key names, parameter names and constants are
facts. `file:line` references are relative to the installed `glyphsLib/` package. Implemented so
far: font information (§1 to §3) in `crates/io-ufo/src/fontinfo.rs`; master alignment zones,
stems and guides (§4.1, §4 stem rows, §9.6) in `crates/io-ufo/src/design.rs`; axes, user
locations and the origin master (§6.1 to §6.5) in `crates/model/src/axes.rs` and
`crates/io-ufo/src/designspace/axes.rs`; instances and variable fonts (§5.1, §5.2, §5.4, §5.5)
in `crates/io-ufo/src/designspace/instances.rs`; the glyph's note, colour and production name
(§7) in `crates/io-ufo/src/glif.rs` and `ufo.rs`; UFO layer names, backgrounds, vertical metrics,
layer user data, layer guides and images (§8.1, §8.4, §8.5, §8.7, §9.6) in `crates/io-ufo/src/ufo.rs`
and `glif.rs`. Not yet: STAT (§6.6), applying instance data (§5.3), bracket glyphs and rules
(§8.2), sparse sources for brace layers (§8.3), colour layer mapping (§8.6), the glyph lib keys
(§7, §8.4, §12.3), `public.openTypeCategories` from categories (§11.4: only a UFO's own map is
read and written back). Where FontCraft differs from glyphsLib on purpose: `docs/plans/M1.md` § M1.7.

Direction labels used in the tables:

- **G→U**: `to_ufos` / `to_designspace` (Glyphs to UFO + designspace).
- **U→G**: `to_glyphs` (UFO/designspace to Glyphs).
- **Lossy** means information is dropped or changed on that path; **missing** means
  the direction has no code at all.

## 0. Builder modes that change the output

Many rows depend on these flags. Defaults in brackets.

| Flag | Direction | Effect | Where |
|---|---|---|---|
| `minimal` [False] | G→U | Production-only output. Skips round-trip lib keys (appVersion, keyboardIncrement, names lib keys, glyph note/colour/metric keys, background, guides, annotations, smart axes, glyph/layer userData, layer libs). Treats `disabled` custom parameters, prefixes, classes and features as absent. Explodes colour layers into separate glyphs. | `builder/font.py:40`, `builder/glyph.py:182-188`, `builder/custom_params.py:1131`, `builder/features.py:102,115,128`, `builder/builders.py:243` |
| `minimize_glyphs_diffs` [False] | G→U | Writes extra keys so G→U→G is stable: `fontMasterID`, layer id / layer order lib keys, instance export/weight/width/interpolation keys, `lastChange`; keeps inactive instances; suppresses style-map name synthesis. | `builder/masters.py:101`, `builder/layers.py:62`, `builder/instances.py:60,145,160`, `builder/glyph.py:199` |
| `minimize_ufo_diffs` [False] | U→G | Writes extra Glyphs-side data so U→G→U is stable: `UFO Filename` parameters, original kerning groups, groups-not-in-feature list, per-master original feature code. | `builder/masters.py:114-119`, `builder/groups.py:148,165`, `builder/features.py:360`, `builder/instances.py:338` |
| `write_skipexportglyphs` [False] | G→U | Non-exported glyphs go to `public.skipExportGlyphs` instead of a per-glyph lib key. fontmake turns this on. | `builder/glyph.py:86-94`, `builder/builders.py:260-268` |
| `store_editor_state` [True] | G→U | Allows `DisplayStrings` to be written. | `builder/custom_params.py:786-794` |
| `generate_GDEF` [True] | G→U | Writes `public.openTypeCategories`. | `builder/features.py:190-192` |
| `use_designspace` (True in `to_designspace`, False in `to_ufos`) | G→U | Font userData goes to the designspace lib instead of each UFO's lib. | `builder/user_data.py:40-50`, `builder/__init__.py:152` |
| `expand_includes` [False] | both | Inlines feature-file `include` statements. | `builder/features.py:201-215`, `builder/features.py:377-386` |
| `family_name` [None] | G→U | Overrides `familyName` of master UFOs and filters instances to that family. | `builder/builders.py:178-186`, `builder/instances.py:230-233` |
| `glyph_data` [None] | G→U | Custom GlyphData overrides category/subCategory/script/production name lookups. | `builder/glyph.py:114-140` |
| preflight `propagate_all_anchors` (on unless font parameter `Propagate Anchors` is false), `apply_origin_anchor`, `align_alternate_layers` (always) | G→U | Mutates the GSFont before conversion. See §8.10. | `builder/__init__.py:164-207`, `builder/transformations/__init__.py:8-24` |

Order of work in G→U, per master UFO: fontinfo from font → names → family userData →
ufo2ft filters → **font-level custom parameters** → master attributes → glyphs and
layers → layer libs → features → **master-level custom parameters** → colour layers;
then skip-export list, groups, kerning; then (designspace only) axes, sources,
instances, family userData, bracket layers, STAT (`builder/font.py:36-69`,
`builder/builders.py:224-274`, `builder/builders.py:351-378`).

Order in U→G: sort sources by `fontMasterOrder`; per full source: font attributes
(first UFO only) → master attributes → bracket glyphs back to layers → glyphs per
layer (backgrounds last); then features, groups, kerning, glyph order, layer order,
designspace userData, axes, source locations, instances (`builder/builders.py:516-611`).

---

## 1. Font level (GSFont)

| GSFont attribute | G→U target (exact key) | Conversion | U→G | Where |
|---|---|---|---|---|
| `familyName` | `info.familyName` of every master UFO; also `source.familyName` | Builder `family_name` overrides it. A `familyNames` property sets `info.familyName` first but `to_ufo_names` then overwrites it with `font.familyName`; only the localized `familyNames` values survive (as name ID 1 records, §2). | First UFO's `info.familyName` → `font.familyName`; later UFOs must match or the build raises. | `builder/font.py:78-89`, `builder/names.py:28`, `builder/sources.py:77`, `builder/names.py:109-116` |
| `versionMajor` | `info.versionMajor` | identity | `info.versionMajor` → `versionMajor` (if not None) | `builder/font.py:72-76,95-97,182-183` |
| `versionMinor` | `info.versionMinor` | identity (GSFont enforces 0..999) | same | `builder/font.py:72-76,184-185`, `classes.py:4849-4852` |
| `unitsPerEm` (`upm`) | `info.unitsPerEm` | identity | same | `builder/font.py:73,180-181` |
| `date` | `info.openTypeHeadCreated` | Formatted `%Y/%m/%d %H:%M:%S`; omitted when the date is missing. Glyphs dates parse as `YYYY-MM-DD HH:MM:SS [+-HHMM]` or 12-hour with AM/PM; the offset is added to the time. | Parsed back with the same `%Y/%m/%d %H:%M:%S` format. | `builder/font.py:128-133,176-179`, `builder/common.py:21,38-45`, `types.py:271-292` |
| `copyright` (property `copyrights`) | `info.copyright` | Text tokens `{{{attr}}}` expanded against the font; localized values → name ID 0 records | `info.copyright` → `font.copyright` (writes property `copyrights`) | `builder/font.py:79,103-124,187-188`, `classes.py:5068-5074` |
| `designer` (`designers`) | `info.openTypeNameDesigner` | as above, name ID 9 | `info.openTypeNameDesigner` → `font.designer` | `builder/font.py:84,189-190` |
| `designerURL` | `info.openTypeNameDesignerURL` | tokens expanded; localized values ignored (no name ID) | → `font.designerURL` | `builder/font.py:85,191-192` |
| `manufacturer` (`manufacturers`) | `info.openTypeNameManufacturer` | name ID 8 for localized | → `font.manufacturer` | `builder/font.py:82,193-194` |
| `manufacturerURL` | `info.openTypeNameManufacturerURL` | no localized records | → `font.manufacturerURL` | `builder/font.py:83,195-196` |
| `note` | **Not mapped to `info.note`.** In format 2 `font.note` is the custom parameter `note`, so it lands in the generic key `com.schriftgestaltung.customParameter.GSFont.note`. In format 3 it is `_note` and is dropped. | — | v2: generic key restores the `note` parameter. v3: lossy. (`info.note` maps to *master* userData, §4.) | `classes.py:4971-4987`, `builder/custom_params.py:1142-1144` |
| `properties` | See §2 | | | |
| `settings.disablesAutomaticAlignment` | font lib `com.schriftgestaltung.customParameter.GSFont.disablesAutomaticAlignment` | bool, **always written** (also when False) | lib key → attribute | `builder/custom_params.py:768-778`, `builder/custom_params.py:294-305` |
| `settings.disablesNiceNames` | font lib `com.schriftgestaltung.customParameter.GSFont.useNiceNames` | `int(not disablesNiceNames)`, always written | `disablesNiceNames = not bool(value)` | `builder/custom_params.py:800-807` |
| `settings.gridLength`, `gridSubDivision` | — | **Missing** (lossy both ways) | — | `classes.py:4764-4776` (parsed, never used by the builder) |
| `settings.keyboardIncrement` | font lib `com.schriftgestaltung.keyboardIncrement` | identity; skipped in `minimal` | lib key → attribute | `builder/font.py:141-144,173-174` |
| `keyboardIncrementBig/Huge`, `keepAlternatesTogether` | — | **Missing** | — | `classes.py:4771-4776` |
| `appVersion` | font lib `com.schriftgestaltung.appVersion` | string; skipped in `minimal`. A value below 895 only triggers a warning. | lib key → attribute | `builder/font.py:141-143,171-172`, `builder/builders.py:160-164` |
| `DisplayStrings` | font lib `com.schriftgestaltung.customParameter.GSFont.DisplayStrings` | Only when `store_editor_state` and non-empty; never during instance application | lib key → attribute | `builder/custom_params.py:782-797` |
| `userData` | `to_designspace`: designspace lib, one key per userData key that does not start with `com.schriftgestaltung.` or `public.`. `to_ufos`: font lib `com.schriftgestaltung.Glyphs.fontUserData` = whole dict (no filtering). | identity | Designspace lib keys (same prefix filter; `ufo2ft.featureWriters` skipped when equal to the default list) → `font.userData`; then the first UFO's `fontUserData` dict fills keys not already present. | `builder/user_data.py:40-50,129-151,243-244` |
| `classes` | Written into `features.fea` (§11). Also into `groups.plist` when the class name is in font userData `com.schriftgestaltung.Glyphs.groupsNotInFeature`. | | Non-kerning UFO groups → `GSClass` (space-joined); classes parsed from `features.fea` are added as well. | `builder/features.py:113-124`, `builder/groups.py:69-80,132-150` |
| `featurePrefixes`, `features` | `features.fea` (§11) | | parsed from first UFO's `features.fea` | `builder/features.py:50-63,349-394` |
| `kerningLTR` / `kerningRTL` / `kerningVertical` | `kerning.plist` (§10) | | | `builder/kerning.py:31-95` |
| glyph order (order of `glyphs`) | font lib `public.glyphOrder` (always, from `font.glyphs` order); the `glyphOrder` custom parameter then takes precedence (§3) | | First UFO's `public.glyphOrder` sorts `font.glyphs` (unknown names last) **and** becomes the `glyphOrder` custom parameter. Bracket glyph names are removed first. | `builder/font.py:135-138`, `builder/custom_params.py:862-888`, `builder/builders.py:533-539,590-598` |
| `customParameters` | §3 | | | |
| `axes` (v3) / `Axes` parameter (v2) | designspace axes (§6) | | | `builder/axes.py:161-314` |
| `metrics`, `stems`, `numbers` (v3 font-level definitions) | Used only to resolve master values (§4) and number tokens in feature code | | — | `classes.py:1887-1996`, `builder/tokens.py:96-108` |
| `filepath` | Base directory for `include` expansion | | — | `builder/features.py:207` |
| — (synthetic) | font lib `com.github.googlei18n.ufo2ft.filters`: appends `{"namespace": "glyphsLib.filters", "name": "cornerComponents", "pre": True}` when any master layer of this master has a `CORNER` hint, then always `{"namespace": "glyphsLib.filters", "name": "eraseOpenCorners", "pre": True}` | | U→G: this key has no special prefix, so it is stored in master userData and returned as-is. | `builder/font.py:46-57,222-233`, `builder/user_data.py:154-160` |
| — (synthetic) | font lib `com.github.googlei18n.ufo2ft.featureWriters` = `[{"class": "CursFeatureWriter"}, {"class": "KernFeatureWriter"}, {"class": "MarkFeatureWriter"}, {"class": "GdefFeatureWriter"}]` | always | same as above (master userData) | `builder/font.py:58`, `builder/constants.py:207-212` |
| — (synthetic) | font lib `com.schriftgestaltung.fontMasterOrder` = master index | int | Sources sorted by it; missing → sorted last | `builder/font.py:63,208-219` |

---

## 2. Properties (Glyphs 3 `properties`, `GSFontInfoValue`)

### 2.1 Value model

| Rule | Where |
|---|---|
| A property has a `key` and either a plain `value` or a list of `{language, value}` pairs. | `classes.py:3396-3420` |
| Reading `value` on a localized property returns the first of `dflt`, `default`, `ENG` that exists, else the first localized entry. | `classes.py:3430-3437` |
| Text tokens `{{{name}}}` in a value are replaced by `getattr(owner, name)`; unknown names are left as written. The owner is the font for font properties and the instance for instance properties. | `builder/common.py:24-35` |

### 2.2 Font-level properties with localized name records (G→U)

Default value → fontinfo attribute. Each localized value whose language is not `dflt`,
`default` or `ENG` becomes a record in `info.openTypeNameRecords` with
`platformID 3`, `encodingID 1`, `languageID = LANGUAGE_MAPPING[lang]` and the
expanded string. An unknown language tag **raises ValueError** (aborts the build).
Records are appended to any existing list. Source: `builder/font.py:78-124`.

| Glyphs property key | fontinfo attribute | Name ID for localized records |
|---|---|---|
| `copyrights` | `copyright` | 0 |
| `familyNames` | `familyName` (then overwritten, §1) | 1 |
| `trademarks` | `trademark` | 7 |
| `manufacturers` | `openTypeNameManufacturer` | 8 |
| `manufacturerURL` | `openTypeNameManufacturerURL` | none |
| `designers` | `openTypeNameDesigner` | 9 |
| `designerURL` | `openTypeNameDesignerURL` | none |
| `descriptions` | `openTypeNameDescription` | 10 |
| `licenses` | `openTypeNameLicense` | 13 |
| `sampleTexts` | `openTypeNameSampleText` | 19 |

`LANGUAGE_MAPPING` (Glyphs/OpenType language tag → Windows LCID) is the table at
`builder/constants.py:230-317`, e.g. `DEU 0x0407`, `FRA 0x040C`, `JPN 0x0411`,
`ZHS 0x0804`, `ZHT 0x0404`, `ZHH 0x0C04`, `PTG 0x0816`, `PTG-BR 0x0416`,
`ARA 0x0C01`, `ESP 0x0C0A`; `dflt` maps to None. The reverse table is used when parsing
`featureNames` (§11).

### 2.3 Properties read through custom-parameter handlers

These handlers look for the **property first**, then the short custom parameter name,
then the long (fontinfo) name. They use the default value only; no localized records.
They run for the font, for instances (§5) and, with no effect, for masters (masters have
no properties). Source: `builder/custom_params.py:253-270,366-417`.

| Property key | Custom parameter (short / long) | fontinfo attribute |
|---|---|---|
| `versionString` | `versionString` / `openTypeNameVersion` | `openTypeNameVersion` |
| `vendorID` | `vendorID` / `openTypeOS2VendorID` | `openTypeOS2VendorID` |
| `uniqueID` | `uniqueID` / `openTypeNameUniqueID` | `openTypeNameUniqueID` |
| `licenses` | `license` / `openTypeNameLicense` | `openTypeNameLicense` |
| `licenseURL` | `licenseURL` / `openTypeNameLicenseURL` | `openTypeNameLicenseURL` |
| `trademarks` | `trademark` / `trademark` | `trademark` |
| `descriptions` | `description` / `openTypeNameDescription` | `openTypeNameDescription` |
| `sampleTexts` | `sampleText` / `openTypeNameSampleText` | `openTypeNameSampleText` |
| `postscriptFullName` | `postscriptFullName` | `postscriptFullName` |
| `postscriptFontName` | `postscriptFontName` | `postscriptFontName` |
| `preferredFamilyNames` | `preferredFamilyName` / `openTypeNamePreferredFamilyName` | `openTypeNamePreferredFamilyName` |
| `preferredSubfamilyNames` | `preferredSubfamilyName` / `openTypeNamePreferredSubfamilyName` | `openTypeNamePreferredSubfamilyName` |
| `WWSFamilyName` | `WWSFamilyName` / `openTypeNameWWSFamilyName` | `openTypeNameWWSFamilyName` |
| `WWSSubfamilyName` | `WWSSubfamilyName` / `openTypeNameWWSSubfamilyName` | `openTypeNameWWSSubfamilyName` |

Not mapped to any fontinfo attribute: `postscriptFullNames` (plural),
`compatibleFullNames`, `styleNames`, `styleMapFamilyNames`, `styleMapStyleNames`,
`variableStyleNames`, `variationsPostScriptNamePrefix`. They are still copied into the
instance lib (§5) but nothing applies them (`builder/custom_params.py:369-406`).

### 2.4 U→G for names

| UFO | Glyphs | Notes | Where |
|---|---|---|---|
| `copyright`, `openTypeNameDesigner(URL)`, `openTypeNameManufacturer(URL)` | `font.copyright` etc. (properties `copyrights`, `designers`, …) | default value only | `builder/font.py:187-196` |
| `openTypeNameVersion`, `openTypeOS2VendorID`, `openTypeNameUniqueID`, `openTypeNameLicense(URL)`, `trademark`, `openTypeNameDescription`, `openTypeNameSampleText`, `postscriptFullName`, `postscriptFontName`, preferred/WWS names | **Custom parameters** with the short name (Glyphs 2 style), not properties | written on the font and again on each master (see §3.6) | `builder/custom_params.py:272-283` |
| `openTypeNameRecords` | Font custom parameters `Name Table Entry`, one per record, text `"nameID platformID encodingID languageID; string"` | Localized properties are **not** rebuilt (lossy). | `builder/custom_params.py:663-751` |

### 2.5 Level summary

| Level | What carries names |
|---|---|
| Font | `properties` (all of §2.2 and §2.3), plus custom parameters for the §2.3 keys (v2 files). |
| Master | No properties. Name-like custom parameters on a master are applied to that master's UFO (§3). |
| Instance | `properties` (whitelisted keys, §5) and custom parameters; applied to the instance UFO by `apply_instance_data` (§5.3). `familyNames`, `postscriptFontName` and `variablePostscriptFontName` feed descriptor attributes directly. |

---

## 3. Custom parameters

### 3.1 How handlers work

| Rule | Where |
|---|---|
| Each registered handler converts one parameter in both directions. G→U runs every handler on the font (once per master UFO), on each master, and on each instance (during `apply_instance_data`). | `builder/custom_params.py:308-312,1115-1147` |
| A handler reads the property (if it has one), then the short name, then the long name. Reading a name marks it "handled". If the short name is found, the long name is never looked up, so a duplicate under the long name falls through to the generic key. If the property is found, neither parameter name is marked. | `builder/custom_params.py:253-270,111-124` |
| More than one parameter with the same name, read through a single-valued handler, **raises RuntimeError**. | `builder/custom_params.py:111-119` |
| Values are passed through text-token expansion against the owning object. | `builder/custom_params.py:118,124` |
| Target choice when writing: if the UFO fontinfo object has an attribute named `ufo_name` (and the handler allows info), write there; else write lib key `com.schriftgestaltung.customParameter.<Class>.<ufo_name>`, where `<Class>` is `GSFont`, `GSFontMaster`, `GSInstance` or `InstanceDescriptorAsGSInstance`. Handlers with `ufo_prefix=""` write the bare key. | `builder/custom_params.py:285-305`, `builder/custom_params.py:92` |
| `ufo_default`: if the converted value equals it, nothing is written (only used by `Virtual Master`, default `[]`). | `builder/custom_params.py:294-296,1241-1245` |
| U→G writes parameters with the **short** name only, appended (never replacing). | `builder/custom_params.py:126-140,272-283` |

### 3.2 Generic fallback and normalization

| Rule | Where |
|---|---|
| G→U: every parameter on the object that no handler marked is written to `com.schriftgestaltung.customParameter.<Class>.<name>` with its value unchanged. Curly quotes in the name (U+2018, U+2019 → `'`; U+201C, U+201D → `"`) are replaced first. | `builder/custom_params.py:1142-1144,1172-1181` |
| The `<Class>` part: `GSFont.` for font parameters, `GSFontMaster.` for master parameters, `GSInstance.` for variable-font instances (dummy UFO), `InstanceDescriptorAsGSInstance.` for static instances applied by `apply_instance_data` (verified by test). | `builder/custom_params.py:92`, `builder/instances.py:101-102,481-482` |
| `UFO Filename` is always marked handled, so it never reaches the generic key. | `builder/custom_params.py:1137` |
| U→G: lib keys starting with `com.schriftgestaltung.customParameter.` that no handler read are normalized (curly quotes), and those that start with this object's prefix (`…GSFont.` on the font pass, `…GSFontMaster.` on the master pass) become custom parameters named by the remainder. Other classes' keys are ignored. | `builder/custom_params.py:193-196,1150-1169` |
| Parameter names that axes/masters code reads through `customParameters[...]` (`Axis Location`, `Axis Mappings`, `Axes` (v2), `Variable Font Origin`, `Variation Font Origin`, `Link Metrics With Master`, `Master Name`, `Feature for Feature Variations`, `Export STAT Table`, `Elidable STAT Axis Value Name`, `Style Name as STAT entry`, `Propagate Anchors`, …) are **not** marked handled, so they also land in the generic key and round-trip through it. | `builder/axes.py:167,445,580,636-659`, `builder/bracket_layers.py:64` |
| Format 2 only: the font's synthesized `Axes` parameter is part of iteration, so it is written as `…GSFont.Axes`. Format 3 does not synthesize it during iteration. | `classes.py:1276-1301,5092-5112` |

### 3.3 `disabled` parameters

| Path | Behaviour | Where |
|---|---|---|
| G→U, `minimal=True` (and `apply_instance_data`) | Disabled parameters are skipped entirely. | `builder/custom_params.py:1118-1134` |
| G→U, `minimal=False` | Disabled parameters are **applied and written** like enabled ones; the `disabled` flag is not stored anywhere, so U→G turns them into enabled parameters (lossy). | `builder/custom_params.py:1121-1123` |
| Lookups through `customParameters["name"]` (axes, masters, instances, sources) | Always skip disabled parameters. | `classes.py:1286-1294` |
| Instance lib `com.schriftgestaltung.customParameters` | Disabled parameters included unless `minimal`. | `builder/instances.py:181-187` |
| `Virtual Master`, `Elidable STAT Axis Value Name`, `Style Name as STAT entry` | Disabled entries ignored explicitly. | `builder/axes.py:168-173`, `builder/stat.py:45-58` |

### 3.4 Defaults (Glyphs defaults that differ from ufo2ft)

| Parameter | fontinfo | Glyphs default written when the UFO value is None | U→G removal | Where |
|---|---|---|---|---|
| `fsType` | `openTypeOS2Type` | `[3]` (bit list) | parameter deleted if equal to `[3]` | `builder/custom_params.py:1184-1223` |
| `underlineThickness` | `postscriptUnderlineThickness` | `50` | deleted if 50 | same |
| `underlinePosition` | `postscriptUnderlinePosition` | `-100` | deleted if -100 | same |

Defaults are applied after both the font pass and the master pass, and during
`apply_instance_data`; not for variable-font descriptors (`builder/instances.py:102`).

### 3.5 Parameter ↔ UFO table

Level column: F = font, M = master, I = instance (applied to the instance UFO).
"all" = F, M and I. Identity conversion unless stated. Glyphs-side value types are
coerced when parsed: ints for the names in `classes.py:1393-1468`, floats for
`postscriptSlantAngle` and `postscriptBlueScale`, bools for the names in
`classes.py:1471-1485`, int lists for `fsType`, `openTypeOS2FamilyClass`,
`openTypeOS2Panose`, `openTypeOS2Type`, `openTypeOS2UnicodeRanges`, `panose`,
`unicodeRanges`, `openTypeHeadFlags` (`classes.py:1486-1497,1517-1532`).

#### 3.5.1 Simple renames (short name, long name accepted on read)

`builder/custom_params.py:315-362`

| Glyphs parameter | fontinfo attribute | Level | Value |
|---|---|---|---|
| `compatibleFullName` | `openTypeNameCompatibleFullName` | all | string |
| `hheaAscender` | `openTypeHheaAscender` | all | int |
| `hheaDescender` | `openTypeHheaDescender` | all | int |
| `hheaLineGap` | `openTypeHheaLineGap` | all | int |
| `panose` | `openTypeOS2Panose` | all | list of 10 ints, identity |
| `fsType` | `openTypeOS2Type` | all | list of **bit numbers** (e.g. `[3]`), identity both ways |
| `typoAscender` | `openTypeOS2TypoAscender` | all | int |
| `typoDescender` | `openTypeOS2TypoDescender` | all | int |
| `typoLineGap` | `openTypeOS2TypoLineGap` | all | int |
| `unicodeRanges` | `openTypeOS2UnicodeRanges` | all | list of **bit numbers**, identity |
| `strikeoutSize` | `openTypeOS2StrikeoutSize` | all | int |
| `strikeoutPosition` | `openTypeOS2StrikeoutPosition` | all | int |
| `subscriptXSize`, `subscriptYSize`, `subscriptXOffset`, `subscriptYOffset` | `openTypeOS2Subscript{XSize,YSize,XOffset,YOffset}` | all | int |
| `superscriptXSize`, `superscriptYSize`, `superscriptXOffset`, `superscriptYOffset` | `openTypeOS2Superscript{XSize,YSize,XOffset,YOffset}` | all | int |
| `vheaVertAscender` and `vheaVertTypoAscender` | `openTypeVheaVertTypoAscender` | all | int (two handlers, same target) |
| `vheaVertDescender` and `vheaVertTypoDescender` | `openTypeVheaVertTypoDescender` | all | int |
| `vheaVertLineGap` and `vheaVertTypoLineGap` | `openTypeVheaVertTypoLineGap` | all | int |
| `blueScale` | `postscriptBlueScale` | all | float |
| `blueShift` | `postscriptBlueShift` | all | int |
| `isFixedPitch` | `postscriptIsFixedPitch` | all | bool |
| `underlinePosition` | `postscriptUnderlinePosition` | all | int; default -100 (§3.4) |
| `underlineThickness` | `postscriptUnderlineThickness` | all | int; default 50 (§3.4) |

Also the property-aware handlers in §2.3.

#### 3.5.2 Parameters whose name equals the fontinfo attribute

`builder/custom_params.py:421-449` (no short name; read and written under the UFO name)

`openTypeHheaCaretSlopeRun`, `openTypeVheaCaretSlopeRun`, `openTypeHheaCaretSlopeRise`,
`openTypeVheaCaretSlopeRise`, `openTypeHheaCaretOffset`, `openTypeVheaCaretOffset`,
`openTypeHeadLowestRecPPEM`, `openTypeHeadFlags`, `openTypeOS2FamilyClass`,
`postscriptSlantAngle`, `postscriptUniqueID`, `postscriptBlueFuzz`,
`postscriptForceBold`, `postscriptDefaultWidthX`, `postscriptNominalWidthX`,
`postscriptWeightName`, `postscriptDefaultCharacter`, `postscriptWindowsCharacterSet`,
`macintoshFONDFamilyID`, `macintoshFONDName`, `styleMapFamilyName`,
`styleMapStyleName`. Level: all.

`postscriptFamilyBlues`, `postscriptFamilyOtherBlues`: same, but U→G skips an empty
list (`builder/custom_params.py:452-463`).

#### 3.5.3 Converted parameters

| Glyphs parameter | UFO target | G→U conversion | U→G conversion | Level | Where |
|---|---|---|---|---|---|
| `codePageRanges` (or `openTypeOS2CodePageRanges` as fallback name) | `openTypeOS2CodePageRanges` | Each entry: an int or digit string is a **code page number** mapped to a bit through `CODEPAGE_RANGES`; a string `"bit N"` is bit N directly; anything else raises. Extra bits from `codePageRangesUnsupportedBits` are appended. Result sorted. An unknown code page number raises KeyError. | Each bit with a known code page → code page number in `codePageRanges`; other bits → `codePageRangesUnsupportedBits` (only written if non-empty). An empty UFO list gives an empty `codePageRanges`. | all | `builder/custom_params.py:466-532`, `builder/constants.py:156-197` |
| `winAscent` | `openTypeOS2WinAscent` | `abs()` | `abs()` | all | `builder/custom_params.py:534-545` |
| `winDescent` | `openTypeOS2WinDescent` | `abs()` | `abs()` | all | same |
| `weightClass` | `openTypeOS2WeightClass` | `int()`; short name only (no long-name read) | identity | all | `builder/custom_params.py:547-550` |
| `widthClass` | `openTypeOS2WidthClass` | `int()` | identity | all | same |
| `GASP Table` and `gasp Table` | `openTypeGaspRangeRecords` | Dict `{maxPPEM: behaviourBits}` → list sorted by maxPPEM of `{"rangeMaxPPEM": int, "rangeGaspBehavior": [bit indices set in the integer]}`. Keys and values cast to int. | Records → `{str(maxPPEM): sum(1 << bit)}`; written as `GASP Table` (the first registered handler; the second handler also writes `gasp Table` with the same value). | all | `builder/custom_params.py:553-592`, `util.py:101-111` |
| `meta Table` | font lib `public.openTypeMeta` (bare key, never fontinfo) | List of `{tag, data}` → dict tag → list of data; tags `appl` and `bild` take a single value (last wins, warning on duplicates). | Dict → list of `{tag, data}`, one entry per list element. | all | `builder/custom_params.py:595-634`, `builder/constants.py:205` |
| `Color Palettes` | font lib `com.github.googlei18n.ufo2ft.colorPalettes` | Each palette: list of Glyphs colours. A colour is a list (or comma string) of 0–255 ints: 2 values = grey + alpha → `(g,g,g,a)`; 4 values = RGBA; 5 values (CMYK) raises NotImplementedError. Each component divided by 255. | Each colour: if r == g == b, write `[r, a]`; then `",".join(str(round(v*255)))` (a comma-separated string). | all | `builder/custom_params.py:637-660`, `builder/common.py:57-66` |
| `Name Table Entry` (multi-valued) | `openTypeNameRecords` (appended) | Text `"<ids>; <string>"`, split at the first `;`; string stripped. ids are space-separated: nameID [platformID [encodingID [languageID]]]. Numbers: `0x…` hex, leading `0` octal, else decimal. Defaults: platform 3, encoding 1, language 0x409; with platform 1 the defaults are encoding 0, language 0. Malformed entries are logged and skipped. | Each record → `"nameID platformID encodingID languageID; string"` as **font** parameters only. | G→U all; U→G font | `builder/custom_params.py:663-751` |
| `Disable Last Change` | lib `com.schriftgestaltung.customParameter.<Class>.disablesLastChange` | identity (fontinfo has no such attribute) | identity | all | `builder/custom_params.py:754` |
| `Don't use Production Names` (straight apostrophe only) | lib `com.github.googlei18n.ufo2ft.useProductionNames` (bare key) | `not value` | `not value` | all | `builder/custom_params.py:756-765` |
| `Use Typo Metrics`, `Has WWS Names`, `openTypeOS2SelectionUnsupportedBits` | `openTypeOS2Selection` | True `Use Typo Metrics` adds bit 7, True `Has WWS Names` adds bit 8, plus unsupported bits; merged into any existing list, sorted, de-duplicated. Nothing written when none of the three is present. | Bit 7 → `Use Typo Metrics` = True, bit 8 → `Has WWS Names` = True, all other bits → `openTypeOS2SelectionUnsupportedBits` (written even when empty, so an empty UFO list survives). | all | `builder/custom_params.py:820-859` |
| `glyphOrder` | font lib `public.glyphOrder` | The parameter's list first, then every glyph of the font order that the parameter does not list. | First UFO's `public.glyphOrder` → `glyphOrder` parameter. | F only | `builder/custom_params.py:862-888` |
| `Filter`, `PreFilter` (multi-valued) | font lib `com.github.googlei18n.ufo2ft.filters` (appended) | One-way. String `"name;arg;key:value;…;include:a,b"` → `{"name", "args": [...], "kwargs": {...}, "include"/"exclude": [...], "pre": True}` for PreFilter. Numbers and booleans in args are cast. `include`/`exclude` must be last or they are ignored with an error log. | **Missing**: never written back as Glyphs parameters (master-level ones are lost; ufo2ft filters in the master UFO lib return through master userData instead). | M and I (not F) | `builder/custom_params.py:891-964`, `builder/filters.py:24-75` |
| `Replace Prefix` (multi-valued) | rewrites `features.fea` | `"PrefixName; code"`; the named `# Prefix:` sections are replaced (last value per name wins), then the feature text is re-serialized through the Glyphs model. | Missing | M, I (no effect at F: features are empty at that point) | `builder/custom_params.py:967-992`, `builder/features.py:320-343` |
| `Replace Feature` (multi-valued) | rewrites `features.fea` | `"tag; code"`; replaces the body between `feature tag {` and `} tag;` (first match, line-anchored). | Missing | M, I (no-op at F) | `builder/custom_params.py:995-1013`, `builder/features.py:300-313` |
| `Reencode Glyphs` | changes glyph unicodes in the UFO | Entries `"name=HEX"`: give the glyph that code point and remove it from its previous owner; `"name="` removes the glyph's code point. Missing glyphs skipped. | Missing | intended I | `builder/custom_params.py:1016-1059` |
| `Rename Glyphs` | swaps glyphs in the UFO | Entries `"a=b"`: swap the glyph data but keep each name's unicodes. Ignored with a warning for variable-font instances and when either glyph is missing. | Missing | intended I | `builder/custom_params.py:1062-1112` |
| `Virtual Master` (multi-valued) | font lib `com.schriftgestaltung.customParameter.GSFont.Virtual Master` = list of values | Not written when there are none. | One parameter per list item. | F only | `builder/custom_params.py:1226-1245` |
| `UFO Filename` | designspace `source.filename` / `instance.filename` | Never stored in lib | Written on masters and instances only with `minimize_ufo_diffs` | M, I | `builder/custom_params.py:1137`, `builder/sources.py:83-84`, `builder/masters.py:114-119`, `builder/instances.py:190-199,338-339` |

#### 3.5.4 Object attributes routed through the parameter machinery

These read and write a **Python attribute** of the Glyphs object, not a custom
parameter, but use the same lib-key naming. Written whenever the attribute exists and
is not None (so default values are written too). `builder/custom_params.py:768-812`

| Glyphs attribute | Object | Lib key | Conversion |
|---|---|---|---|
| `disablesAutomaticAlignment` | GSFont | `com.schriftgestaltung.customParameter.GSFont.disablesAutomaticAlignment` | identity |
| `disablesNiceNames` | GSFont | `com.schriftgestaltung.customParameter.GSFont.useNiceNames` | `int(not v)` / `not bool(v)` |
| `DisplayStrings` | GSFont | `com.schriftgestaltung.customParameter.GSFont.DisplayStrings` | identity, gated (§1) |
| `iconName` | GSFontMaster | `com.schriftgestaltung.customParameter.GSFontMaster.iconName` | identity (empty string written) |
| `weightValue`, `widthValue`, `customValue`, `customValue1`, `customValue2`, `customValue3` | GSFontMaster | `com.schriftgestaltung.customParameter.GSFontMaster.<attr>` | identity; these are `axes[0..5]` (§4) |

### 3.6 Level rules and duplication

| Rule | Where |
|---|---|
| G→U: font parameters are applied to **every** master UFO, then each master's own parameters are applied on top (master wins for fontinfo targets; lib keys use different prefixes so both survive). | `builder/font.py:60`, `builder/builders.py:256` |
| U→G: the first UFO's fontinfo is converted into **font** parameters and every UFO's fontinfo into **master** parameters, so the first master duplicates the font-level values (verified: `hheaAscender` and `fsType` appear on both). | `builder/font.py:200`, `builder/masters.py:151` |
| Font-only handlers: `glyphOrder`, `Virtual Master`, and `Name Table Entry` (U→G direction). Non-font only: `Filter`/`PreFilter`. | `builder/custom_params.py:869-885,733,949,1226-1235` |
| Vertical metrics: when the font is vertical (§8.7) and a master has none of `vheaVert(Typo)Ascender` / `Descender` / `LineGap` as master parameters, the master UFO gets `openTypeVheaVertTypoAscender = int(upm/2)`, `…Descender = -int(upm/2)`, `…LineGap = upm`. This runs after the font-level parameters, so it **overwrites** font-level vhea values. | `builder/masters.py:74-92` |

---

## 4. Masters (GSFontMaster)

| Glyphs | G→U | U→G | Where |
|---|---|---|---|
| `id` | font lib `com.schriftgestaltung.fontMasterID` only with `minimize_glyphs_diffs`; also used as the key of `_sources` | If the lib key exists and no master has that id yet, `master.id` = it; otherwise a fresh uppercase UUID4 | `builder/masters.py:100-102,108-112`, `classes.py:1762` |
| `name` | `info.styleName`; `source.styleName`; `source.name = "<family> <style>"` | `info.styleName` plus lib weight/width/customName → `set_all_name_components` (below) | `builder/names.py:28-29`, `builder/sources.py:77-81`, `builder/names.py:119-125` |
| name parts `weight`, `width`, `customName` | font lib `com.schriftgestaltung.weight`, `com.schriftgestaltung.width` (always: default "Regular" is truthy), `com.schriftgestaltung.customName` (if non-empty). Skipped in `minimal`. | `weight`/`width` default "Regular", `customName` default "". If joining the parts reproduces `styleName`, no `Master Name` parameter; otherwise `Master Name` = styleName. | `builder/names.py:46-56`, `classes.py:1819-1846` |
| Name joining rule | width, weight, customName, drop empties and redundant "Regular"; if italicAngle ≠ 0: `["Regular"]` → "Italic", else append "Italic" unless customName contains "Italic" or "Oblique". `Master Name` parameter overrides everything. | — | `classes.py:1801-1846` |
| style-map names | `info.styleMapFamilyName` / `styleMapStyleName` from `build_stylemap_names` with `is_bold = name in {"Bold", "Bold Italic", "Bold Oblique"}`, `is_italic = italicAngle != 0`. Not written with `minimize_glyphs_diffs`. | Ignored (styleMap master fields are not read; they would fall into custom params via the `styleMapFamilyName`/`styleMapStyleName` handlers, §3.5.2) | `builder/names.py:31-43` |
| `iconName` | `com.schriftgestaltung.customParameter.GSFontMaster.iconName` | same key → attribute | §3.5.4 |
| `visible` | **Missing** | — | — |
| `axes` (`axesValues` v3) / `weightValue`, `widthValue`, `customValue`, `customValue1..3` (v2) | Designspace `source.location[axisName]` = `axes[i]` for axis definition index i (§6). Also font lib `com.schriftgestaltung.weightValue` (always, not None), `com.schriftgestaltung.widthValue` (if truthy), `com.schriftgestaltung.customValue{,1,2,3}` (if truthy), and the §3.5.4 keys. Defaults: index 0 and 1 → 100, others → 0. | Design location from `source.location` by axis name (`_to_glyphs_source`). The `com.schriftgestaltung.weightValue` etc. keys are not read back (they start with the reserved prefix, so they do not enter userData either). | `builder/masters.py:51-64`, `builder/sources.py:128-134,216-237`, `classes.py:2038-2102` |
| user location | Master UFO `openTypeOS2WeightClass` / `openTypeOS2WidthClass` set from the user location **only when every master has `Axis Location`** (font uses axis locations) | When the font uses axis locations, user location = reverse axis map of the design location → master `weightClass`/`widthClass` parameter and `Axis Location` entry updated | `builder/masters.py:66-72`, `builder/sources.py:228-237`, `builder/axes.py:453-503` |
| `ascender` | `info.ascender` (always) | if not None | `builder/masters.py:29,121-122` |
| `capHeight` | `info.capHeight` | same | `builder/masters.py:30,123-124` |
| `descender` | `info.descender` | same | `builder/masters.py:31,125-126` |
| `xHeight` | `info.xHeight` | same | `builder/masters.py:32,127-128` |
| v3 metric values | `ascender` etc. read from `metricValues[i].position` where i is the index of the font metric with type `ascender`, `cap height`, `x-height`, `descender`, `italic angle`. Missing → defaults 800 / 700 / 500 / -200 / 0. | setter adds the font metric if absent and pads with zeros | `classes.py:1725-1736,1887-1915,1998-2036` |
| `italicAngle` | `info.italicAngle = -italicAngle` (Glyphs: clockwise positive; UFO: counter-clockwise); always written | `italicAngle = -info.italicAngle` if non-zero | `builder/masters.py:36,41-42,132-140` |
| alignment zones | `postscriptBlueValues` / `postscriptOtherBlues` (exact rule below) | rule below | `builder/blue_values.py:18-55` |
| v3 alignment zones | Derived: for each font metric except `italic angle`, if the master's value has non-zero `overshoot`, zone = (position, overshoot). An explicit list (v2 or set by U→G) takes precedence. | — | `classes.py:1917-1938` |
| `horizontalStems` | `info.postscriptStemSnapH` (if non-empty) | `info.postscriptStemSnapH` → `horizontalStems` | `builder/masters.py:34-40,130-138` |
| `verticalStems` | `info.postscriptStemSnapV` | same | same |
| v3 `stemValues` | `horizontalStems` = stems whose font stem definition has `horizontal = true`; `verticalStems` = the rest | — | `classes.py:1958-1996` |
| `numbers` (v3 `numberValues`) | **Missing** (used only for number tokens in feature code) | — | `builder/tokens.py:96-108` |
| `guides` | font-level `fontinfo.plist` `guidelines` (§9.6) | font `guidelines` → master guides | `builder/masters.py:95`, `builder/masters.py:150` |
| `customParameters` | §3 (applied after glyphs and features) | §3 | `builder/builders.py:256`, `builder/masters.py:151` |
| `userData` | Each key not starting with `com.schriftgestaltung.` or `public.` → font lib key of the same name. Special keys below. | Each UFO lib key not starting with those prefixes (and not a MATH key) → master userData | `builder/user_data.py:53-62,154-170` |
| userData `com.schriftgestaltung.Glyphs.ufoYear` | `info.year` | `info.year` → this key | `builder/masters.py:44-46,142-143` |
| userData `com.schriftgestaltung.Glyphs.ufoNote` | `info.note` | `info.note` → this key | `builder/masters.py:47-49,144-145` |
| userData `com.schriftgestaltung.Glyphs.ufoData` | UFO `data/` files: `{posix path: bytes}` | every data file → this dict (paths converted to POSIX) | `builder/user_data.py:59-62,162-170` |
| userData `com.schriftgestaltung.Glyphs.ufoFilename` | designspace `source.filename` (if no `UFO Filename` parameter) | — | `builder/sources.py:83-88` |
| userData `com.schriftgestaltung.Glyphs.layerLib.<layerName>` | merged into that UFO layer's `layerinfo` lib (§8.4) | written from each layer's lib | `builder/user_data.py:86-103,187-204` |
| userData `com.schriftgestaltung.Glyphs.originalFeatureCode` | replaces the whole `features.fea` | written when UFO feature files differ (§11) | `builder/features.py:52-54,444-457` |
| parameters `Link Metrics With First Master` / `Link Metrics With Master` (id, then name) | Glyph widths and kerning are taken from the linked master | — | `classes.py:1774-1799`, `builder/glyph.py:261-278`, `builder/kerning.py:33-35` |
| master source flags | The regular master (§6.4) gets `copyLib`, `copyInfo`, `copyGroups`, `copyFeatures` = True | ignored | `builder/sources.py:71-75` |
| source filename | `UFO Filename` parameter → `ufoFilename` userData → `"<family>-<style>.ufo"` with spaces removed; clashes get `#1`, `#2`… before the extension | — | `builder/sources.py:83-126`, `util.py:26-33` |

### 4.1 Alignment zone ↔ blue values (exact)

G→U (`builder/blue_values.py:18-33`):

1. Sort zones by (position, size).
2. For each zone: if `position == 0` **or** `size >= 0`, append `sorted(position, position + size)` to `postscriptBlueValues`; otherwise append `sorted(position, position + size)` to `postscriptOtherBlues`.
3. Write each list only if non-empty.

U→G (`builder/blue_values.py:36-55`), with values taken in pairs `(y1, y2)`:

| Source | Rule |
|---|---|
| BlueValues pair | `size = y2 - y1`. If `y2 == 0`: position 0, size `-(y2 - y1)` (a baseline overshoot below zero). Else position `y1`, size `y2 - y1`. |
| OtherBlues pair | position `y2`, size `y1 - y2` (negative). |
| Result | zones sorted by descending position. |

Example (verified): BlueValues `[-10, 0, 500, 510]`, OtherBlues `[-210, -200]` →
zones (500, 10), (0, -10), (-200, -10) → back to the same lists.

FontCraft then places the zones in the model (`crates/io-ufo/src/ufo.rs`, `place_design`): a zone
at the position of one of the UFO's metrics becomes that metric's overshoot; a zone at 0 becomes
the overshoot of a `baseline` metric added for it; a zone of size 0 and the rest are the master's
other zones. A file whose lists the rule would not give back unchanged keeps them as written.

---

## 5. Instances (GSInstance)

### 5.1 G→U (designspace `InstanceDescriptor`)

An instance is converted when `minimize_glyphs_diffs` is on, or when it is active
(`exports` and `active`, which are the same field) and, with a `family_name` filter,
its family matches. `type == variable` instances become variable-font descriptors
(§5.4). `builder/instances.py:57-73`, `builder/axes.py:691-695`.

| Glyphs | Designspace | Conversion | Where |
|---|---|---|---|
| `familyName` (property `familyNames` → parameter `familyName` → font family) | `familyName` | tokens expanded | `builder/instances.py:123`, `classes.py:3552-3558` |
| `name` | `styleName` | tokens expanded | `builder/instances.py:124` |
| — | `name` = `"<familyName> <styleName>"` | | `builder/instances.py:156-158` |
| property `variablePostscriptFontName` → property `postscriptFontName` → parameter `postscriptFontName` | `postScriptFontName` | tokens expanded; None if none set | `builder/instances.py:125-130` |
| parameters `UFO Filename` → `com.schriftgestaltung.Glyphs.fullFilename` → `fileName` | `filename` | first two used as paths (basename joined to `instance_dir` if given); `fileName` gets `.ufo` appended; fallback `"<instance_dir or instance_ufos>/<Family>-<Style>.ufo"` (spaces removed) | `builder/instances.py:190-210` |
| `axes` (`axesValues` v3; `interpolationWeight`, `interpolationWidth`, `interpolationCustom{,1,2,3}` v2) | `location[axisName]` = design value for each axis that exists in the designspace | defaults 100, 100, then 0 | `builder/instances.py:133-139`, `classes.py:3671-3737` |
| `isBold`, `isItalic`, `linkStyle` | `styleMapFamilyName`, `styleMapStyleName` | Only when `linkStyle` is set or not `minimize_glyphs_diffs`. Rule in §5.5. | `builder/instances.py:145-154` |
| `active` / `exports` | lib `com.schriftgestaltung.export` (bool) | only with `minimize_glyphs_diffs` | `builder/instances.py:160-161` |
| `weight` (`weightClass` plist: UI string in v2, number in v3) | lib `com.schriftgestaltung.weight` | `minimize_glyphs_diffs` only; also feeds axis mapping (§6.2) | `builder/instances.py:162` |
| `width` (`widthClass`) | lib `com.schriftgestaltung.width` | same | `builder/instances.py:163` |
| `instanceInterpolations` | lib `com.schriftgestaltung.intanceInterpolations` (**misspelled** key, kept on purpose) | dict, `minimize_glyphs_diffs` only | `builder/instances.py:165`, `builder/constants.py:72-73` |
| `manualInterpolation` | lib `com.schriftgestaltung.manualInterpolation` | bool, `minimize_glyphs_diffs` only | `builder/instances.py:166` |
| `customParameters` | lib `com.schriftgestaltung.customParameters` = **ordered list of `[name, value]` pairs** | tokens expanded; excludes the blacklist below; disabled ones excluded only in `minimal` | `builder/instances.py:171-173,181-187` |
| `properties` | lib `com.schriftgestaltung.properties` = ordered list of `[key, value]` pairs | Whitelisted keys only (below). Without `minimize_glyphs_diffs`, font-level whitelisted properties whose value changes when tokens are expanded against the instance are added first; instance properties override. | `builder/instances.py:174-176,213-227` |
| `visible`, `userData`, `custom` | **Missing** | | — |

Parameter blacklist (not copied into the instance lib, `builder/constants.py:75-87`):
`familyName`, `postscriptFontName`, `fileName`, `weightClass`, `widthClass`,
`com.schriftgestaltung.Glyphs.fullFilename`, `UFO Filename`.

Property whitelist (`builder/constants.py:91-122`): `designers`, `designerURL`,
`manufacturers`, `manufacturerURL`, `copyrights`, `versionString`, `vendorID`,
`uniqueID`, `licenses`, `licenseURL`, `trademarks`, `descriptions`, `sampleTexts`,
`postscriptFullNames`, `postscriptFullName`, `compatibleFullNames`, `styleNames`,
`styleMapFamilyNames`, `styleMapStyleNames`, `preferredFamilyNames`,
`preferredSubfamilyNames`, `variableStyleNames`, `WWSFamilyName`, `WWSSubfamilyName`,
`variationsPostScriptNamePrefix`. (`familyNames` and `postscriptFontName` are
deliberately absent: they go to descriptor attributes.)

### 5.2 U→G

| Designspace | Glyphs | Rule | Where |
|---|---|---|---|
| lib `com.schriftgestaltung.export` | `active` | default True | `builder/instances.py:244-248` |
| `styleName` | `name` | | `builder/instances.py:250` |
| `location[axisName]` | design value per axis definition | missing axes skipped | `builder/instances.py:252-259` |
| (wght, wdth) design location | user location = reverse of the designspace axis map (piecewise linear); then `set_user_loc`: `weight`/`width` = nearest UI code; if the code's value differs, also `weightClass`/`widthClass` parameter = class | | `builder/instances.py:261-273`, `builder/axes.py:453-481` |
| lib `com.schriftgestaltung.weight` | `weight` | used if no weight yet or if it maps to the same class as the computed one (resolves e.g. ExtraLight vs UltraLight) | `builder/instances.py:275-293` |
| lib `com.schriftgestaltung.width` | `width` | only if width unset | `builder/instances.py:295-300` |
| `familyName` | parameter `familyName` | only if different from the font family | `builder/instances.py:302-304`, `classes.py:3560-3564` |
| `styleMapFamilyName` | `linkStyle` | strip a leading `familyName` and whitespace | `builder/instances.py:306-310` |
| `styleMapStyleName` | `isBold` = contains "bold"; `isItalic` = contains "italic" | | `builder/instances.py:312-315` |
| `postScriptFontName` | parameter `postscriptFontName` | | `builder/instances.py:317-318`, `classes.py:3654-3658` |
| lib `com.schriftgestaltung.manualInterpolation`, `…intanceInterpolations` | attributes | | `builder/instances.py:320-332` |
| lib `com.schriftgestaltung.customParameters` | appended parameters in list order | | `builder/instances.py:334-336` |
| `filename` | `UFO Filename` parameter | `minimize_ufo_diffs` only | `builder/instances.py:338-339` |
| lib `com.schriftgestaltung.properties` | **Missing** (properties not restored; lossy) | | — |
| `ds.variableFonts` | **Missing** (variable instances not restored) | | — |

### 5.3 Applying instance data to interpolated UFOs (`apply_instance_data`)

| Step | Rule | Where |
|---|---|---|
| Weight/width class | If the designspace has a `wght` (`wdth`) axis: user location from the instance's design location through the reverse map (identity without map; axis default when the location lacks the axis) → `openTypeOS2WeightClass` = `int(user)`, `openTypeOS2WidthClass` = nearest class by percent (§6.1). | `builder/instances.py:365-407,476-479` |
| Parameters | Lib `customParameters` list and `properties` list are wrapped as a fake instance (`InstanceDescriptorAsGSInstance`) and run through all handlers with disabled-skipping on and defaults on (§3). | `builder/instances.py:345-362,481-482` |
| Name properties | For each §2.2 property present in the instance properties: set the fontinfo attribute (default value; no localized records). | `builder/instances.py:484-490` |

### 5.4 Variable-font instances (`type = variable`)

| Glyphs | Designspace | Where |
|---|---|---|
| `name` | variable-font descriptor `name` | `builder/instances.py:94-98` |
| parameter `fileName`, else `<family>[-<name> unless name is "Regular"]VF` with spaces removed | descriptor `filename` | `builder/instances.py:86-92` |
| — | `axisSubsets` = one full-range subset per designspace axis | `builder/instances.py:97` |
| custom parameters | run through all handlers on a dummy UFO (no defaults); every non-None fontinfo attribute → descriptor lib `public.fontInfo` (dict); every dummy lib key copied into the descriptor lib (generic keys use `GSInstance.`) | `builder/instances.py:100-113` |
| design location | `None` for variable instances (not part of axis mapping or STAT) | `classes.py:3671-3673`, `builder/axes.py:134-135` |
| U→G | **Missing** | — |

### 5.5 Style-map names (`build_stylemap_names`)

`builder/names.py:59-106`

| Output | Rule |
|---|---|
| `styleMapStyleName` | "bold italic", "bold", "italic" or "regular" from the flags. |
| linked style | `linkStyle` if set and not "Regular"; otherwise the style name with the **last** occurrence of "Regular" (when neither flag), "Bold" (when bold), and "Italic" or "Oblique" (when italic) removed, words re-joined by spaces. |
| `styleMapFamilyName` | `familyName + " " + linked style` if the linked style is non-empty, else `familyName`. |

---

## 6. Axes

### 6.1 Axis definitions

| Rule | Where |
|---|---|
| Axis list = font `Axes` (v3 `font.axes`; v2 `Axes` parameter) → one definition per entry `{Name, Tag, Hidden}`; design value of axis i is `master/instance.axes[i]`. Missing tag → `XXXX` (index 0) or `XXX<i>`. | `builder/axes.py:523-589` |
| Without an `Axes` value (default font axes Weight/wght + Width/wdth, none hidden, no `Axis Mappings`): three default definitions: Weight `wght` (index 0), Width `wdth` (index 1), Custom `XXXX` (index 2). | `builder/axes.py:570-582`, `classes.py:5092-5105` |
| Per-tag defaults: `wght` default design 100, default user 400, UI key `weight`, class parameter `weightClass`; `wdth` default design and user 100, key `width`, parameter `widthClass`; others default 0, no key. | `builder/axes.py:545-553` |
| Weight UI codes → user value: Thin 100, ExtraLight/UltraLight 200, Light 300, Normal/Regular (or missing) 400, Medium 500, DemiBold/SemiBold 600, Bold 700, UltraBold/ExtraBold 800, Black/Heavy 900. Integer values used as-is. Lookup retries with spaces removed. | `classes.py:220-236`, `builder/axes.py:42-79` |
| Width UI codes → class: Ultra Condensed 1, Extra Condensed 2, Condensed 3, SemiCondensed 4, Medium (normal) 5, Semi Expanded 6, Expanded 7, Extra Expanded 8, Ultra Expanded 9; class → percent: 50, 62.5, 75, 87.5, 100, 112.5, 125, 150, 200. | `classes.py:238-249`, `builder/constants.py:218-228` |
| Percent → width class: nearest table value (ties → lower class). Weight class = `int(user)`. | `builder/axes.py:85-103` |

### 6.2 User location of a master or instance (`get_user_loc`)

Applied in this order; each later rule overrides (`builder/axes.py:391-439`):

1. Start: 400 for `wght`; the design location for every other axis.
2. If the object has the UI key attribute (`weight`/`width`; masters also have these as
   name parts) and the string maps → that value.
3. `weightClass`/`widthClass` parameter → `class_to_value` (weight: int; width: percent table).
4. `Axis Location` parameter entry whose `Axis` equals the axis **name** → its `Location`.

### 6.3 Designspace axis (G→U, `to_designspace_axes`)

`builder/axes.py:161-314`

| Case | Mapping (user → design) | Default (user) |
|---|---|---|
| Font parameter `Axis Mappings` exists | `{float(user): design}` from `Axis Mappings[tag]`; axes not listed are **dropped** | reverse map of the regular master's design location |
| Every master has `Axis Location` | masters: user from §6.2, design from axes; plus active instances using **only** their `Axis Location` (or all instances with `minimize_glyphs_diffs`) | regular master's §6.2 user location |
| Otherwise | instance mapping (all §6.2 rules) if non-empty and not identity; else masters with user = design | reverse map of the regular master's design location |

Then:

| Rule | Where |
|---|---|
| Conflicting user locations log a warning; the later one wins. | `builder/axes.py:147-153,215-221` |
| If the mapping is still identity, each enabled `Virtual Master` coordinate for this axis name is added as user = design. | `builder/axes.py:268-282` |
| `minimum` = min user, `maximum` = max user, `default` = regular user clamped to [min, max]. | `builder/axes.py:284-286` |
| Axis written only if min < max, or min ≠ default user location, or the mapping is not identity, or the tag is listed in `Axes`. `map` = sorted (user, design) pairs, only when not identity. `tag`, `name`, `hidden` copied. Localized label names: missing. | `builder/axes.py:176-179,288-299` |
| No axis survives → one Weight axis `wght` with min = default = max = 400. | `builder/axes.py:307-314` |
| Variable instances never contribute. | `builder/axes.py:134-135` |

### 6.4 Regular (origin) master

`builder/axes.py:632-688`

1. Font parameter `Variable Font Origin` = master **id**.
2. Else legacy `Variation Font Origin` = master **name** (warning logged, `builder/builders.py:170-176`).
3. Else the base style: words common to all master names (order of the first), or "Regular"; first master whose name equals it.
4. Else first master whose name, with "Regular" words removed, equals the base style.
5. Else the first master.

### 6.5 U→G axes

| Rule | Where |
|---|---|
| Without a designspace (plain UFO list): build Weight and Width axes from `openTypeOS2WeightClass`/`WidthClass`: design = class value (weight int, width percent); axis created only if there are at least 2 distinct (user, design) pairs; map = sorted pairs; default = default user location clamped. | `builder/builders.py:667-699` |
| With a designspace: axes without a map get an identity map `[(min, min), (max, max)]` (or one pair when min = max) so the range survives. STAT-only `ital` axis skipped. | `builder/builders.py:652-664` |
| `GSAxis(name or "Weight"/"Width" for wght/wdth, tag, hidden)`; `font.axes` set only if the list is **not** a prefix of (Weight wght, Width wdth, Custom XXXX) without hidden axes. | `builder/axes.py:328-347,591-601` |
| Font parameter `Axis Mappings` = `{tag: {str(user): design}}` for **all** axes, written when any axis map is non-identity, or is identity but its first/last input differs from the sources' min/max design locations. | `builder/axes.py:349-351,604-629` |
| Master locations from `source.location`; `Axis Location`/class parameters only when all masters already carry `Axis Location` (normally from the generic lib key). | `builder/sources.py:216-237` |

### 6.6 STAT (G→U only, designspace v5 axis labels)

`builder/stat.py:69-257`. Runs only when there is an active variable instance and not
every variable instance has `Export STAT Table` = false.

| Rule | Where |
|---|---|
| Instance user location on an axis = `axis.map_backward(design location)`. | `builder/stat.py:104-108` |
| Manual mode (any active static instance has `Style Name as STAT entry` = tag): for each axis, labels from instances whose parameter lists that tag; name = instance name; elidable if `Elidable STAT Axis Value Name` = tag; first instance per value wins. | `builder/stat.py:150-158` |
| Automatic mode: per axis, one label per distinct user value. Non-default values: name of the instance that is at the default on all other axes (else the first one there). Default value: on the first axis where instances vary, the default instance's name; elsewhere "Regular" (`wdth`: "Normal"), elidable. A label is elidable if its name equals that default name or the elidable parameter is set. | `builder/stat.py:185-221` |
| If the default instance is named just "Italic", "italic" is removed from label names. | `builder/stat.py:114-117,178-182` |
| `wght`: if its default label is elidable and a bold-linked instance exists, the default label's `linkedUserValue` = that instance's weight. Real `ital` axis: default links to the highest label value. | `builder/stat.py:223-252` |
| `elidedFallbackName` = "Regular". | `builder/stat.py:128` |
| Without a real `ital` axis: add discrete axis `ital` "Italic" with one label: "Italic" (1, not elidable) if the default instance is italic (flag or "italic" in name), else "Roman" (0, elidable, linked to 1). | `builder/stat.py:130-147` |
| U→G: the STAT-only `ital` axis (values `[0]` or `[1]`) is ignored; labels are not read back. | `builder/stat.py:255-257` |

---

## 7. Glyphs (GSGlyph)

Each glyph is written into every master UFO (default layer) and into each non-master
layer's UFO layer (§8). Glyph-level fields below are repeated in every UFO glyph made
from that glyph.

| Glyphs | G→U | U→G | Where |
|---|---|---|---|
| `name` | glif name | glif name; glyphs with the same name across UFOs merge into one GSGlyph | `builder/builders.py:298`, `builder/glyph.py:441-450` |
| `unicodes` (hex strings) | `unicodes` (ints). Not for colour-layer glyphs; cleared for bracket glyphs. | `unicodes` → `"%04X"` strings (only if non-empty) | `builder/glyph.py:81-84,452-453`, `builder/bracket_layers.py:123` |
| `.uvNNN` glyph names (001–256) | If exported and the base glyph exists with a unicode: font lib `public.unicodeVariationSequences[selectorHex][baseUnicodeHex] = glyphName`. Selectors: uv001–uv016 → FE00–FE0F, uv017–uv256 → E0100–E01EF. | Missing (derived again) | `builder/glyph.py:53-69,96-106` |
| `production` | font lib `public.postscriptNames[name]` when the production name differs from the glyph name. Production name = explicit `production`, else custom GlyphData if it differs from built-in, else built-in GlyphData. A bracket glyph's production name gets the `.BRACKET.varAltNN` suffix. | `public.postscriptNames[name]` → `production` (derived names become explicit; lossy) | `builder/glyph.py:108-152,471-474` |
| `export` | False and `write_skipexportglyphs`: name added to `public.skipExportGlyphs` (designspace lib and every UFO lib, sorted). False otherwise: glyph lib `com.schriftgestaltung.Glyphs.Export` = False. | glyph lib key → `export`; name in designspace `public.skipExportGlyphs` → False. Plain UFOs: their `public.skipExportGlyphs` lists are promoted if all equal (else ValueError). | `builder/glyph.py:86-94,463-469`, `builder/builders.py:260-268,507-512,715-727` |
| `category` | glyph lib `com.schriftgestaltung.Glyphs.category` (explicit, or custom GlyphData value that differs from built-in) | → `category` | `builder/glyph.py:123-125,159-162,504-509` |
| `subCategory` | glyph lib `com.schriftgestaltung.Glyphs.subCategory` (same rule) | → `subCategory` | `builder/glyph.py:126-128,163-166,510-514` |
| `script` | glyph lib `com.schriftgestaltung.Glyphs.script` | → `script` | `builder/glyph.py:129-131,154-155,501-502` |
| GDEF class | font lib `public.openTypeCategories` (§11.4) | stored in font userData for reuse | `builder/features.py:218-297,389-393` |
| `case`, `direction`, `tags`, `locked`, `sortName`, `sortNameKeep`, `leftKerningKey`, `rightKerningKey`, `topKerningGroup`, `bottomKerningGroup`, `topMetricsKey`, `bottomMetricsKey`, `vertWidthMetricsKey` | **Missing** (lossy) | — | (no builder reference; only used by feature tokens, `builder/tokens.py:170-215`) |
| `color` | glif `public.markColor`. Index 0–11 → the fixed string (table below). List of 4 ints 0–255 → `"r,g,b,a"`, each `0` → "0", `255` → "1", else `v/255` with 3 decimals. Anything else: warning, nothing written. Skipped in `minimal`. | Exact match with a table string → that index; otherwise list of `round(float(c) * 255)` per component. | `builder/glyph.py:229-250,460-461,562-573`, `builder/constants.py:141-154` |
| `note` | glif `note` | `note = ufo note or ""` | `builder/glyph.py:225-227,454` |
| `lastChange` | glyph lib `com.schriftgestaltung.Glyphs.lastChange` = `%Y/%m/%d %H:%M:%S`; only with `minimize_glyphs_diffs` and font parameter `Disable Last Change` not True | parsed as that format, else as a Glyphs date | `builder/glyph.py:199-204,455-459`, `builder/common.py:48-54` |
| `leftMetricsKey`, `rightMetricsKey`, `widthMetricsKey` (glyph level; v3 `metricLeft` etc.) | glyph lib `com.schriftgestaltung.Glyphs.glyph.<key>` (if truthy) | → glyph attribute | `builder/glyph.py:252-258,485-499` |
| `leftKerningGroup` | groups `public.kern2.<group>` (§10) | membership → attribute | `builder/groups.py:115-123,164-195` |
| `rightKerningGroup` | groups `public.kern1.<group>` | same | same |
| `partsSettings` (`smartComponentAxes`) | glyph lib `com.schriftgestaltung.smartComponentAxes` = list of `{name, bottomName, bottomValue, topName, topValue}`; skipped in `minimal` | → `GSSmartComponentAxis` list | `builder/components.py:227-256`, `classes.py:4530-4535` |
| `userData` | font lib `com.schriftgestaltung.Glyphs.glyphUserData.<glyphName>` = dict of all keys except `com.nagwa.MATHPlugin.*` (skipped in `minimal`) | that font lib key → `glyph.userData` (replaces) | `builder/user_data.py:65-83,173-176` |
| userData `com.nagwa.MATHPlugin.extendedShape` | font lib `com.nagwa.MATHPlugin.extendedShape` (list of glyph names, appended) | name in list → userData key = True | `builder/user_data.py:72-73,178-180` |
| userData `com.nagwa.MATHPlugin.variants` | glyph lib `com.nagwa.MATHPlugin.variants` (dict, merged with layer-level values) | `vVariants`/`hVariants` sub-keys → glyph userData; other sub-keys → layer userData | `builder/user_data.py:74-77,182-184,207-229` |
| width of non-spacing marks | If category is Mark and subCategory Nonspacing and width > 0: glyph width 0 and glyph lib `com.schriftgestaltung.Glyphs.originalWidth` = width | width 0 + key present → width restored | `builder/glyph.py:168-180,516-528` |

Colour table (`builder/constants.py:141-154`), index → `public.markColor`:

| Index | String | Index | String |
|---|---|---|---|
| 0 | `0.85,0.26,0.06,1` | 6 | `0,0.67,0.91,1` |
| 1 | `0.99,0.62,0.11,1` | 7 | `0.18,0.16,0.78,1` |
| 2 | `0.65,0.48,0.2,1` | 8 | `0.5,0.09,0.79,1` |
| 3 | `0.97,1,0,1` | 9 | `0.98,0.36,0.67,1` |
| 4 | `0.67,0.95,0.38,1` | 10 | `0.75,0.75,0.75,1` |
| 5 | `0.04,0.57,0.04,1` | 11 | `0.25,0.25,0.25,1` |

---

## 8. Layers (GSLayer)

### 8.1 Layer kinds and UFO layer names (G→U)

`builder/layers.py:32-81`, `builder/builders.py:276-348`, `classes.py:4183-4286`

| Glyphs layer | Detection | UFO destination |
|---|---|---|
| Master layer | `layerId == associatedMasterId` | default layer of that master's UFO |
| Colour palette layer | v3: `attributes.colorPalette` exists; v2: name `Color N` or `Color *` | layer `color.<N>` (`*` → 65535). With `minimal`: separate glyphs (§8.6). |
| Brace / intermediate layer | v3: `attributes.coordinates`; v2: name contains `{` and `}` and not `.background` | layer named `{c1, c2, …}` built from the coordinates (v3, `str()` of each value) or the layer name as written (v2); plus a sparse designspace source (§8.3) |
| Bracket / alternate layer | v3: `attributes.axisRules`; v2: name matches `…[N]…` (min N) or `…]N]…` (max N) on the first axis | free-standing BRACKET glyphs (§8.2), not a UFO layer; layers whose name contains `.background` are excluded |
| Other named layer (backup etc.) | everything else | layer named `layer.name`; a second glyph with the same layer name gets `<name> #1`, `#2`, … (warning). Skipped with `minimal`. |
| Unnamed non-special layer | empty name | skipped (warning with `minimize_glyphs_diffs`) |
| Dangling layer | neither `layerId` nor `associatedMasterId` is a master | skipped (warning with `minimize_glyphs_diffs`) |
| Background of a master layer | `layer.background` exists | layer `public.background` |
| Background of another layer | | layer `<layer name>.background` |

### 8.2 Bracket layers → BRACKET glyphs + rules

`builder/bracket_layers.py:16-185`

| Step | Rule |
|---|---|
| Requirement | at least one designspace axis, else ValueError |
| Box | per axis (in designspace axis order, zipped with the rules): missing min/max replaced by the axis design min/max (from the map's design values, else axis min/max); axes whose range is the full range are left out. Values are design coordinates. Keyed by axis tag. |
| Glyph name | `<glyph>.BRACKET.varAlt<NN>`, NN = two-digit 1-based index of the distinct box within that glyph |
| Rules | for exported glyphs only: one entry `([box by axis name], {glyph: bracketGlyph})`; overlapping boxes are split and merged with fontTools `overlayFeatureVariations`; each result → rule named `BRACKET.` + `.`-joined `<axisName>_<min>_<max>` sorted by axis name, one condition set `{name, minimum, maximum}` per axis, `subs` = mapping items |
| Feature tag | font parameter `Feature for Feature Variations`: `rclt` → `rulesProcessingLast = True`; any other value except `rvrn` → designspace lib `com.github.fonttools.varLib.featureVarsFeatureTag` |
| Implicit layers | a master with no bracket layer for a box uses its master layer as the bracket glyph source |
| Glyph content | full glyph conversion of the layer into the default layer of its master's UFO; unicodes cleared; glyph lib `com.schriftgestaltung.Glyphs._originalLayerName` = layer name, or "" for implicit copies |
| Components | a component whose base glyph has a bracket glyph for the same box is pointed at it |
| Kerning | the bracket glyph joins every kern1/kern2 group of its parent and copies the parent's glyph-level pairs |
| Non-exported parents | glyphs are still made (and added to `public.skipExportGlyphs`), no rule |
| After | GDEF categories regenerated |
| Preflight | `align_alternate_layers` copies the component's bracket boxes onto composites that lack them (v2: name suffix ` [min]`/` ]max]`; v3: `axisRules`) |
| U→G | each BRACKET glyph is moved into a UFO layer named by `_originalLayerName` (if non-empty) under the base name, component suffixes stripped, all BRACKET glyphs deleted; missing `_originalLayerName` raises NotImplementedError. Designspace rules are ignored; v3 `axisRules` attributes are **not** rebuilt (lossy). Bracket names are removed from `public.glyphOrder`, groups and kerning. (`builder/builders.py:533-579`, `builder/groups.py:141`, `builder/kerning.py:85-88`) |

### 8.3 Brace layers → sparse sources

`builder/sources.py:137-208`

| Rule |
|---|
| Group brace layers by (UFO layer name, coordinates); each group yields one source per associated master. |
| Source: `filename` and `font` of the master source, `layerName` = brace layer name, `name` = `"<master source name> <layer name>"`. |
| Location: coordinates zipped with designspace axes in order. Fewer coordinates than axes → the rest from the master's location. More → warning, extras ignored. |
| Inserted right after the master's source, sorted by location values. Duplicate locations log a warning. |
| U→G: sources with `layerName` are not masters; their glyphs come back as ordinary named layers (`to_glyphs_layer`). v3 `coordinates` attributes are **not** rebuilt (lossy). |

### 8.4 Layer metadata

| Glyphs | G→U | U→G | Where |
|---|---|---|---|
| `name` | UFO layer name (§8.1) | layer name → `name` (master layer name forced to master name) | `builder/layers.py:117-145` |
| `associatedMasterId` | which UFO the layer goes to | master of the UFO being read | `builder/layers.py:33,127` |
| `layerId` | layerinfo lib `com.schriftgestaltung.layerId` (`minimize_glyphs_diffs`). The lib belongs to the UFO layer, so the last glyph written wins. | → `layerId` | `builder/layers.py:62-63,128-129` |
| order within glyph | layerinfo lib `com.schriftgestaltung.layerOrderInGlyph.<glyphName>` = index (`minimize_glyphs_diffs`) | stored in temp userData `__layerOrder`, layers sorted by it (missing → last), key removed | `builder/layers.py:64-66,84-89,132-135,148-160` |
| UFO layer lib | from master userData `com.schriftgestaltung.Glyphs.layerLib.<layerName>` (and legacy font userData of that name); inner key `com.schriftgestaltung.Glyphs.layerName` renames the layer (used for a default layer not called `public.default`) | layer lib keys without reserved prefixes → master userData `…layerLib.<name>`; a default layer not named `public.default` stores its name under `…layerName` and is keyed `public.default` | `builder/user_data.py:86-103,187-204` |
| `width` | glif `width` (linked-metrics rule, mark rule §7) | glif width → `width` (background: §8.5) | `builder/glyph.py:168-180,516-524` |
| `vertWidth` | glif `height` = vertWidth, else typoAscender − typoDescender (master parameters, else master ascender/descender); only when the font is vertical | set if height differs from that default | `builder/glyph.py:369-399,576-579` |
| `vertOrigin` | glif `verticalOrigin` = ascender − vertOrigin, else ascender (same ascender rule) | `vertOrigin = ascender − verticalOrigin` when set and ≠ ascender | `builder/glyph.py:381-384,581-582` |
| `leftMetricsKey`, `rightMetricsKey`, `widthMetricsKey` (layer; v3 `metricLeft`…) | glyph lib `com.schriftgestaltung.Glyphs.layer.<key>` | also legacy `com.schriftgestaltung.Glyphs.<key>` → layer | `builder/glyph.py:252-258,485-499` |
| `userData` | each key without reserved prefixes (and not MATH) → glyph lib key of the same name (skipped in `minimal`) | each glif lib key without reserved prefixes → layer userData | `builder/user_data.py:106-119,207-231` |
| `guides` | glif `guidelines` (§9.6) | → guides | `builder/glyph.py:185`, `builder/glyph.py:532` |
| `annotations` | glyph lib `com.schriftgestaltung.annotations` = list of dicts with the truthy fields among `angle`, `position` (list), `text`, `type`, `width` | each dict → GSAnnotation (`position` accepts "{x, y}" or a pair) | `builder/annotations.py:20-54` |
| `backgroundImage` | glif `image.fileName` = path, `image.transformation` = matrix; glyph lib `com.schriftgestaltung.backgroundImage.crop` = `[x, y, w, h]`, `…locked` = bool, `…alpha` = int (10–100, else 50) | reverse; crop rebuilt as rect | `builder/background_image.py:20-48`, `classes.py:3816-3825` |
| `hints` | glyph lib `com.schriftgestaltung.hints` = list of dicts: `horizontal`, `options`, `stem`, `type`, `name` (always present, may be None); `origin`, `other1`, `other2`, `target` as index-path lists when complete (`target` "up"/"down" as plain string); `place`, `scale` as point lists | reverse; `target` spelled as character lists is re-joined | `builder/hints.py:21-63` |
| hint `settings` | **Missing** | — | `classes.py:3130` |
| `CORNER` hints | add the `cornerComponents` ufo2ft pre-filter (§1) | — | `builder/font.py:46-53` |
| `anchors` | glif anchors (§9.5) | | `builder/glyph.py:219` |
| shape order | glyph lib `com.schriftgestaltung.Glyphs.shapeOrder` = string of `P` (path) / `C` (component) when the layer has both | shapes reordered by it; other characters raise ValueError | `builder/glyph.py:209-218,542-556` |
| `color` (layer colour), `visible`, `attributes` (other than kind detection), `partSelection` (v3 smart pole mapping) | **Missing** | — | `classes.py:3932-3942` |
| v2 smart pole mapping | stored in layer userData `PartSelection`, so it travels as the glyph lib key `PartSelection` | → layer userData | `classes.py:4057-4071` |

### 8.5 Background layers

| Rule | Where |
|---|---|
| Only written outside `minimal`, and only when `layer.background` exists. | `builder/glyph.py:186,402-406` |
| Background glyph gets: width from background userData `com.schriftgestaltung.Glyphs.backgroundWidth` (if set), background image, paths, anchors, guides. **Components are decomposed** (with a warning), resolved against master layers or same-named layers. | `builder/glyph.py:402-420`, `builder/components.py:41-47,104-154` |
| U→G: layer `public.background` → master layer's background; `<name>.background` → background of the same-named layer for that master (created if missing). Backgrounds are processed after foregrounds. A non-zero UFO width is stored in background userData `com.schriftgestaltung.Glyphs.backgroundWidth`. | `builder/layers.py:95-114`, `builder/glyph.py:516-522`, `builder/builders.py:763-767` |

### 8.6 Colour layers

| Mode | G→U | Where |
|---|---|---|
| non-minimal | master glyph lib `com.github.googlei18n.ufo2ft.colorLayerMapping` = list of `[UFO layer name, palette index]` for this master's palette layers (layer ids replaced by final layer names after all glyphs are built) | `builder/glyph.py:294-306`, `builder/layers.py:23-29` |
| minimal, palette layers | each non-master palette layer → glyph `<glyph>.color<i>` in the default layer (no unicodes); brace palette layers emitted at their intermediate layers; font lib `com.github.googlei18n.ufo2ft.colorLayers[glyph] = [[layerGlyph, paletteIndex], …]` | `builder/color_layers.py:33-107,299-304` |
| minimal, v3 full-colour layers (`attributes.color`) | paths grouped by equal attributes; each group → glyph `<glyph>.color<i>` with a COLRv1 paint (solid `fillColor`, linear/radial `gradient`, stroke via skia-pathops; components of colour glyphs as `PaintColrGlyph` with optional translate/transform); font lib `com.github.googlei18n.ufo2ft.colorLayers[glyph] = {PaintColrLayers}`; new colours appended to every palette in `com.github.googlei18n.ufo2ft.colorPalettes` | `builder/glyph.py:317-366`, `builder/color_layers.py:110-296` |
| Components inside palette layers | point at the component glyph's palette layer glyph `<name>.color<i>` with the same palette index when that layer is not its master layer | `builder/components.py:52-72` |
| U→G | **Missing** (no colour layer reconstruction) | — |

### 8.7 Vertical font detection

`builder/builders.py:192-221`: the font is vertical if the font or any master has all
three vhea metrics (each of `vheaVertAscender`/`vheaVertTypoAscender`,
`vheaVertDescender`/`vheaVertTypoDescender`, `vheaVertLineGap`/`vheaVertTypoLineGap`),
or any master layer has `vertWidth` or `vertOrigin`.

### 8.8 Smart component glyph layers

The smart glyph's own layers are ordinary layers (§8.1). Their v3 pole mapping
(`partSelection`) is not stored (§8.4).

### 8.9 Layer user data special cases

Layer userData keys starting with `com.schriftgestaltung.` or `public.` are never copied
(`builder/user_data.py:243-244`).

### 8.10 Anchor propagation and other preflight transforms (G→U)

| Transform | Rule | Where |
|---|---|---|
| `apply_origin_anchor` | Every layer with a `*origin` anchor: shift paths, component positions and other anchors by minus its position; remove `*origin`. | `builder/transformations/apply_origin_anchor.py:11-35` |
| `align_alternate_layers` | §8.2 preflight row. | `builder/transformations/align_alternate_layers.py:8-117` |
| `propagate_all_anchors` (font parameter `Propagate Anchors`, default on) | Composites processed in component-depth order; master, bracket, brace and smart-pole layers. Marks that have their own anchors keep them. For each component: take its (already propagated) anchors at this layer (interpolated for brace layers and smart components); a component after the first with an explicit `anchor` renames e.g. `top` to `top_1`; underscore anchors are taken only from the first component and only if the layer has none; `entry*` only from the first component; `exit*` from earlier components dropped when a later one lacks `_*`/`exit*`; flipped components swap left↔right, entry↔exit (x) and top↔bottom (y); ligatures number base anchors `name_N`; positions transformed by the component matrix and rounded to 6 decimals. Then the layer's own anchors (origin-adjusted) override. `_bottom` removes `top`/`_top`; `_top` removes `bottom`/`_bottom`. Only glyphs with components get the result. | `builder/transformations/propagate_anchors.py:38-102,437-600,621-691` |
| Legacy UFO-side propagation | deprecated `propagate_anchors` builder argument | `builder/anchor_propagation.py:15-173`, `builder/builders.py:228-229` |

---

## 9. Paths, nodes, components, anchors, guides

### 9.1 Nodes

| Glyphs | UFO point | Where |
|---|---|---|
| `line` | `line` | `pens.py:84-95` |
| `curve` | `curve` | same |
| `qcurve` | `qcurve` | same |
| `offcurve` | `segmentType` None | same |
| first node of an open path (must be `line`, asserted) | `move`, without `smooth` | `builder/paths.py:35-43` |
| U→G: `move` | `line`; None → `offcurve` | `pens.py:84-89` |
| `smooth` | `smooth` | `builder/paths.py:50-57,80` |
| userData `name` | point `name` | `builder/paths.py:41,55`, `builder/paths.py:81`, `classes.py:2267-2279` |
| userData `UFO.identifier` | point `identifier` | `builder/paths.py:42,56,82-83` |
| other node userData | glyph lib `com.schriftgestaltung.Glyphs.nodeUserData.<pathIndex>.<nodeIndex>` (indices in Glyphs node order) | U→G: same key → node userData, ignoring a `name` entry | `builder/paths.py:58-67`, `builder/user_data.py:122-126,234-240` |

Start-node rotation: Glyphs stores a closed contour's start node **last**. G→U moves
the last node to the front; U→G moves the first point to the end
(`builder/paths.py:44-47,85-87`). Empty paths produce an empty UFO contour
(`builder/paths.py:31-33`).

### 9.2 Paths

| Glyphs | UFO | Where |
|---|---|---|
| `closed` | contour open = not closed | `builder/paths.py:35,85` |
| `attributes` (v3 `attr`: fill, stroke, gradient, …) | **Missing**, except for minimal colour layers (§8.6) | `builder/color_layers.py:264-283` |

### 9.3 Components

| Glyphs | G→U | U→G | Where |
|---|---|---|---|
| `name` (v3 `ref`) | `baseGlyph` | same | `builder/components.py:51,80,159` |
| transform | UFO `transformation` = the 6-value affine matrix (xx, xy, yx, yy, dx, dy) | matrix stored as-is (matrix-born component) | `builder/components.py:80,160` |
| v2 `transform` | the matrix as written | | `classes.py:3016` |
| v3 `pos`, `scale`, `angle`, `slant` → matrix | No angle and no slant: `(sx, 0, 0, sy, x, y)` exactly. Otherwise M = T(pos) · K(slant) · R(angle) · S(scale): scale first, then rotate (degrees, counter-clockwise), then slant with K = [[1, tan(slantX)], [tan(slantY), 1]] (x′ = x + tan(slantX)·y, y′ = tan(slantY)·x + y), then translate. A single slant number means slantX only; a single scale number means both axes. | | `classes.py:301-320,2713-2725` |
| matrix → v3 fields (when writing format 3) | Solve M = K · R · S, preferring slantY = 0, then the smallest shear; a 180° rotation with negative scaleY is rewritten as scaleX negated, angle 0; near-zero values snapped to 0. Singular matrices fall back to a scale/rotation approximation without shear. | | `classes.py:332-406,2766-2790` |
| `alignment` (int), `anchor` (str) | glyph lib `com.schriftgestaltung.Glyphs.ComponentInfo` = list of `{name, index, anchor?, alignment?}`, one entry per component that has a non-empty anchor or non-zero alignment; `index` is the Glyphs component index | entries applied when `index` exists and the component name matches; mismatches logged and skipped | `builder/components.py:82-93,173-216` |
| `locked` | glyph lib `com.schriftgestaltung.componentsLocked` = list of bools, one per component, written if any is true | list zipped onto components (None skipped) | `builder/components.py:98-101,163-171,219-221` |
| `smartComponentValues` (v3 `piece`) | glyph lib `com.schriftgestaltung.componentsSmartComponentValues` = list of dicts, written if any is non-empty | same | same |
| — (legacy) | — | glyph lib `com.schriftgestaltung.componentsAlignment` list read into `alignment` | `builder/components.py:163-168` |
| smart components | If the referenced glyph has smart axes and more than one pole layer for this master: **decomposed** into outlines interpolated with a fontTools VariationModel over the pole layers (axis values normalized with the bottom/top values; the first pole layer is the default), nested smart components first; the component matrix applied, contours reversed when the determinant is negative. With one pole layer: plain component. | Decomposed outlines come back as paths (lossy) | `builder/components.py:77-80`, `builder/smart_components.py:43-216` |
| `attributes` (v3 `attr`) | **Missing** | — | `classes.py:3019` |
| components in background layers | decomposed (§8.5) | | `builder/components.py:41-47` |

Note: after smart-component decomposition the UFO component list is shorter than the
Glyphs one, but the `ComponentInfo` indices and the `componentsLocked` /
`componentsSmartComponentValues` lists still use Glyphs positions.

### 9.4 Smart component axes

See §7 (`partsSettings`). Glyph lib `com.schriftgestaltung.smartComponentAxes`.

### 9.5 Anchors

| Glyphs | G→U | U→G | Where |
|---|---|---|---|
| `name`, `position` | anchor `name`, `x`, `y` | same | `builder/anchors.py:27-48` |
| `userData` | anchor gets a random uppercase UUID4 `identifier`; glyph lib `public.objectLibs[identifier]` = userData dict | **Missing** (userData and identifiers dropped) | `builder/anchors.py:33-38` |

### 9.6 Guides (master guides → fontinfo `guidelines`; layer guides → glif `guidelines`)

`builder/guidelines.py:27-94`

G→U, with angle `a = guide.angle mod 360`:

| Condition | UFO guideline fields |
|---|---|
| y is 0 and a ∈ {90, 270} (vertical) | `x` only |
| else x is 0 and a ∈ {0, 180} (horizontal) | `y` only |
| else | `x`, `y`, `angle = a` (angle convention unchanged: degrees, counter-clockwise from the x axis) |

Name handling G→U: if the Glyphs name ends with ` [#<id>]`, that becomes
`identifier`; then if it ends with ` [<digits , . e E + ->]`, that becomes `color`;
if `locked`, ` [locked]` is appended to the remaining name; empty names are omitted.

U→G:

| Rule |
|---|
| Name ending ` [locked]` → `locked = True`, suffix removed. |
| `color` → append ` [<color>]` to the name; `identifier` → append ` [#<identifier>]`. With no name these give a leading space (e.g. `" [1,0,0,1] [#abc]"`, verified). |
| `position = (x or 0, y or 0)`. |
| `angle` present → `angle mod 360`. Absent and the guideline looks vertical (y missing or 0) → 90. Otherwise 0. **Defect (verified):** a UFO guideline with only `y: 0` is turned into a vertical guide (angle 90) at x = 0. |
| GSGuide `alignment`, `filter`, `lockAngle`, `showMeasurement`: **missing** both ways. |

FontCraft differs in two places. A guideline with only `y` reads as a horizontal line (angle 0),
as the UFO specification says, so the defect above does not apply. A guideline's `color` and
`identifier` are fields of the model's guide; they join the name as above only when a `.glyphs`
file is written, and the UFO writer takes them from the end of a name only when the fields are
empty.

---

## 10. Kerning and groups

### 10.1 Groups

| Rule | Direction | Where |
|---|---|---|
| Glyph `rightKerningGroup = X` → member of `public.kern1.X`; `leftKerningGroup = X` → `public.kern2.X`. | G→U | `builder/groups.py:115-123,182-195` |
| RTL glyphs (referenced from `kerningRTL` directly or through a group): the sides are swapped (`rightKerningGroup` → kern2, `leftKerningGroup` → kern1). A glyph cannot be kerned correctly in both directions. | G→U | `builder/groups.py:28-62,108-123` |
| Font userData `com.schriftgestaltung.Glyphs.originalKerningGroups` = `{groupName: [glyphs]}`: used first, to keep member order, empty groups and members not in the font; a member is kept only if missing from the font or its attribute still matches. | G→U | `builder/groups.py:82-105` |
| Classes named in font userData `com.schriftgestaltung.Glyphs.groupsNotInFeature` → plain groups (code split on single spaces; empty code → empty list). | G→U | `builder/groups.py:69-80` |
| Groups are identical in every master UFO. | G→U | `builder/groups.py:125-129` |
| From the **first** UFO only: kerning groups set the glyphs' attributes (`public.kern1.X` → `rightKerningGroup = X`, `kern2` → `leftKerningGroup`); other groups → `GSClass(name, " ".join(glyphs))`. BRACKET members removed. With `minimize_ufo_diffs`: `originalKerningGroups` and `groupsNotInFeature` stored. Differences in other UFOs only logged. | U→G | `builder/groups.py:132-195` |

### 10.2 Kerning

| Rule | Direction | Where |
|---|---|---|
| Per master; a master with linked metrics uses its source master's kerning. | G→U | `builder/kerning.py:31-35` |
| `kerningLTR[masterId]`: left key `@MMK_L_X` → `public.kern1.X`, right key `@MMK_R_X` → `public.kern2.X`, glyph names as-is. Missing groups only log a warning. | G→U | `builder/kerning.py:59-78` |
| `kerningRTL[masterId]`: in both keys swap the prefix `@MMK_L_` ↔ `@MMK_R_` (pair order unchanged), then merge into the LTR table per first key, RTL values winning per pair. | G→U | `builder/kerning.py:23-54` |
| `kerningVertical` | **Missing** | `classes.py:4955-4965` |
| Every UFO pair → `setKerningForPair(masterId, …)` in **LTR**; `public.kern1.X` → `@MMK_L_X`, `public.kern2.X` → `@MMK_R_X`; pairs with BRACKET glyphs skipped. RTL tables are not rebuilt (lossy). | U→G | `builder/kerning.py:81-95` |
| Bracket glyphs inherit their parent's groups and glyph pairs. | G→U | `builder/bracket_layers.py:163-185` |

### 10.3 Where FontCraft differs (M1.7)

| glyphsLib | FontCraft |
|---|---|
| A UFO 2's renamed kerning groups also stay groups under their old names (ufoLib copies them), so they become classes too | The old names are only the file's spelling of the kerning groups; they do not become classes |
| Classes named in `groupsNotInFeature` go to `groups.plist` and into `features.fea` as well (reading back gives each class twice) | Groups go to `groups.plist` only |
| `kerningRTL` pairs are merged into `kerning.plist` with swapped group sides | Not written; the loss report lists them, with vertical and contextual kerning and top and bottom groups |

---

## 11. Features

### 11.1 Writing `features.fea` (G→U)

`builder/features.py:50-215`

| Element | Output |
|---|---|
| Override | If master userData `com.schriftgestaltung.Glyphs.originalFeatureCode` is set, it is the whole file. |
| Order | classes, then prefixes, then features; blocks joined by a blank line (`"\n\n"`); file ends with `"\n"`; an all-whitespace result becomes `""`. |
| Disabled items | dropped only in `minimal`. Otherwise disabled prefixes and classes are written as normal code (flag lost); disabled features use the `# disabled` form below. |
| Prefix | `# Prefix: <name>\n` (omitted when the name is `<anonymous>`), then `# automatic\n` if automatic, then the code. |
| Class | `# automatic\n` if automatic, then `@<name> = [ <code>\n];` (`@` not doubled if the name already starts with it). |
| Feature header | `feature <tag> {` |
| Notes | `# notes:` then each note line as `# <line>` |
| Feature names, format 2 | from the notes: a `featureNames {…};` block is cut out of the notes and emitted as-is; else a first line `Name: <text>` is cut out and emitted as `featureNames {` / `  name "<text>";` / `};` |
| Feature names, format 3 | from `labels`: each `{language, value}` with a non-empty value → `  name "<v>";` for `dflt`, else `  name 3 1 0x<LCID hex> "<v>";`, wrapped in `featureNames {` … `};`. Unknown language raises ValueError. |
| Escaping in names | backslash → `\005c`, double quote → `\0022` |
| Automatic | `# automatic` line after the names |
| Disabled feature | `# disabled` line, then every code line prefixed with `#` |
| Code | the code after token expansion |
| Manual kern | a `kern` feature that is not automatic and has no line matching `\s*# Automatic Code.*` gets `# Automatic Code\n` appended |
| Footer | `} <tag>;` |
| Token expansion | per master: `$[predicate]` glyph-class predicates and `${number}` / `$name` number tokens from master numbers and glyph data are expanded (`builder/tokens.py:51-411`); one-way. |
| Variable feature syntax | conditional `#ifdef VARIABLE`/`#ifndef VARIABLE`, `condition` blocks and variable GPOS values are rewritten to feaLib syntax (`builder/variable_features.py:151-488`); one-way. |
| Includes | with `expand_includes`, inlined through feaLib relative to the `.glyphs` file and re-serialized. |
| Font parameters | `Replace Feature` / `Replace Prefix` applied afterwards at master and instance level (§3.5.3). |

### 11.2 Parsing `features.fea` (U→G)

`builder/features.py:349-844`

| Rule |
|---|
| Only the first full source's feature text is parsed. If whitespace-normalized texts differ across UFOs: with `minimize_ufo_diffs`, add a prefix named `WARNING` (comment explaining the file is not editable) and store each UFO's text in its master's `com.schriftgestaltung.Glyphs.originalFeatureCode`; otherwise warn and use the first UFO. |
| Parsed with feaLib (glyph names from the UFO); original text recovered by statement locations. |
| Comment matching `^# Prefix: (.*)$` starts a prefix; it collects following statements until a glyph class definition, feature block, table block, another `# Prefix:` comment, or a `# automatic` comment directly followed by a class definition. A `^# automatic$` comment inside sets `automatic`. Code keeps the original text minus up to two trailing newlines. |
| Glyph class definition, optionally preceded by `# automatic`: name without `@`; code = elements joined by spaces (class references as `@name`). |
| Feature block: remove the first `^# automatic$` comment; remove a `^# disabled$` comment plus the following run of comments → code = that run with `#` removed and dedented, `disabled = True`; remove a `^# notes:$` comment plus the following comments → `notes`. |
| `featureNames` in an **automatic** feature only: format 2 (the default for a new GSFont) → prepended to notes as `Name: <string>` (single name with 3/1/0x409) or the block text; format 3 → `labels` when every name is platform 3, encoding 1 (unknown LCID raises ValueError). |
| Feature code = remaining statements' text minus one trailing newline. |
| Everything else at the root (language systems, lookups, tables including GDEF, markClass, …) → prefix named `<anonymous>`, flushed before the next handled element. |
| `public.openTypeCategories` of the first UFO → font userData `com.schriftgestaltung.Glyphs.originalOpenTypeCategory`. |

### 11.5 Where FontCraft differs (M1.7)

FontCraft writes the §11.1 form (`crates/io-ufo/src/features.rs`) and reads a text back into snippets
only when writing them gives the same bytes; any other text stays one unnamed prefix, written back
unchanged.

| glyphsLib | FontCraft |
|---|---|
| Disabled prefixes and classes are written as live code | Commented out after `# disabled`, as a disabled feature; read back as disabled |
| A disabled anonymous prefix has no header | It gets `# Prefix: <anonymous>`, so it reads back as one |
| A prefix's code loses two trailing line breaks when named, one when anonymous, wherever it stands | Two before the next item, one at the end of the text: what the writer added |
| Class code is rebuilt from the parsed glyph list; a comment inside the brackets becomes an anonymous prefix | The text between `[ ` and the line break before `];`, comments included |
| Feature code starts at its first statement (leading indentation is lost) | The text between the header line and the closing line |
| Disabled code and notes are dedented and end with a line break | Exactly the commented lines, one `#` (or `# `) taken off each |
| `name "…";` reads back as language `ENG` (platform defaults) | As `dflt`, the language it is written for |
| A manual `kern` feature's added `# Automatic Code` line stays in its code | Taken off again when the writer added it |
| Unknown label languages raise an error | Left out; the loss report lists them |
| Format 2: a `Name:` line or `featureNames` block in a feature's notes becomes the feature's names | Notes stay notes (no format 2 labels yet) |
| Feature tokens (`$[…]`, `${…}`) and conditional variable syntax are expanded per master | Not expanded: the code is written as the model holds it |

### 11.3 Markers summary

| Marker | Meaning | Where |
|---|---|---|
| `# Prefix: <name>` | start of a named prefix | `builder/features.py:105-106,584` |
| `<anonymous>` | name of a prefix without a header | `builder/constants.py:41` |
| `# automatic` | automatic prefix/class/feature | `builder/features.py:46-47,157-158,585` |
| `# disabled` | disabled feature, code commented out | `builder/features.py:159-161,586` |
| `# notes:` | feature notes follow as comments | `builder/features.py:152-154,587` |
| `# Automatic Code` | insertion point for generated kerning in a manual `kern` feature | `builder/constants.py:45-46`, `builder/features.py:178-184` |

### 11.4 GDEF categories

`builder/features.py:218-297` (written when `generate_GDEF`, default on; again after
bracket glyphs are added)

| Category | Rule (category/subCategory from glyph lib, else GlyphData) |
|---|---|
| `ligature` | subCategory `Ligature` and an anchor whose name does not start with `_` |
| `mark` | category `Mark` and subCategory `Nonspacing` or `Spacing Combining` |
| `base` | otherwise, any anchor not starting with `_` |
| override | font userData `com.schriftgestaltung.Glyphs.originalOpenTypeCategory` entries replace computed ones |
| target | font lib `public.openTypeCategories` (only if non-empty) |

---

## 12. Key index

Value types use plist terms. "Font lib" = UFO `lib.plist`; "glyph lib" = glif
`<lib>`; "layerinfo lib" = UFO layer `layerinfo.plist` lib; "DS lib" =
designspace `<lib>`; "instance lib" = `InstanceDescriptor.lib`; "VF lib" =
variable-font descriptor lib. Read = U→G uses it.

### 12.1 `public.*`

| Key | Lives on | Type | Written | Read | Where |
|---|---|---|---|---|---|
| `public.glyphOrder` | font lib | array of strings | always | yes | `builder/font.py:138`, `builder/custom_params.py:871-885`, `builder/builders.py:533-539,590-598` |
| `public.postscriptNames` | font lib | dict name → name | when production names differ | yes | `builder/glyph.py:148-152,472-474` |
| `public.skipExportGlyphs` | DS lib and font lib | array of strings (sorted) | with `write_skipexportglyphs` | DS lib; font lib only for plain UFO lists | `builder/builders.py:265-268,507-512,715-727`, `builder/bracket_layers.py:77-80` |
| `public.openTypeCategories` | font lib | dict name → `base`/`mark`/`ligature` | with `generate_GDEF` | yes (into font userData) | `builder/features.py:297,391-393` |
| `public.unicodeVariationSequences` | font lib | dict selector hex → dict base hex → name | `.uvNNN` glyphs | no | `builder/glyph.py:105-106` |
| `public.openTypeMeta` | font lib | dict tag → string or array | `meta Table` | yes | `builder/custom_params.py:625-634` |
| `public.objectLibs` | glyph lib | dict identifier → dict | anchors with userData | no | `builder/anchors.py:33-38` |
| `public.markColor` | glyph lib (glif `markColor`) | string `"r,g,b,a"` | glyph colour | yes | `builder/glyph.py:229-250,460-461` |
| `public.background` | UFO layer **name** | — | master layer backgrounds | yes | `builder/layers.py:74,95` |
| `public.default` | UFO layer name (as key part) | — | — | layer lib naming | `builder/user_data.py:193-200`, `builder/constants.py:214` |
| `public.kern1.<name>`, `public.kern2.<name>` | group names | array of strings | always | yes | `builder/groups.py:122`, `builder/kerning.py:68,75` |
| `public.fontInfo` | VF lib | dict fontinfo attr → value | variable instances | no | `builder/instances.py:104-110` |

### 12.2 `com.github.*`

| Key | Lives on | Type | Written | Read | Where |
|---|---|---|---|---|---|
| `com.github.googlei18n.ufo2ft.filters` | font lib (master UFO and instance UFO); VF lib | array of dicts | always (`eraseOpenCorners`, maybe `cornerComponents`); plus `Filter`/`PreFilter` | via master userData | `builder/font.py:46-57`, `builder/custom_params.py:948-961` |
| `com.github.googlei18n.ufo2ft.featureWriters` | font lib | array of dicts | always (default list) | via master userData; DS-lib copy skipped if default | `builder/font.py:58`, `builder/user_data.py:133-136` |
| `com.github.googlei18n.ufo2ft.useProductionNames` | font lib | bool | `Don't use Production Names` | yes | `builder/custom_params.py:756-765` |
| `com.github.googlei18n.ufo2ft.colorPalettes` | font lib | array of arrays of 4-number arrays (0–1) | `Color Palettes`; extended by colour layers | yes | `builder/custom_params.py:651-660`, `builder/color_layers.py:230,294-296` |
| `com.github.googlei18n.ufo2ft.colorLayerMapping` | glyph lib (master glyph) | array of `[layerName, int]` | palette layers, non-minimal | no | `builder/glyph.py:298-306`, `builder/layers.py:23-29` |
| `com.github.googlei18n.ufo2ft.colorLayers` | font lib | dict glyph → array of `[glyph, int]` or a paint dict | minimal colour layers | no | `builder/color_layers.py:299-304` |
| `com.github.fonttools.varLib.featureVarsFeatureTag` | DS lib | string | `Feature for Feature Variations` not `rvrn`/`rclt` | no | `builder/bracket_layers.py:64-68` |

### 12.3 `com.schriftgestaltung.*` in UFO/designspace objects

| Key | Lives on | Type | Written | Read | Where |
|---|---|---|---|---|---|
| `com.schriftgestaltung.appVersion` | font lib | string | non-minimal | yes | `builder/font.py:143,171-172` |
| `com.schriftgestaltung.keyboardIncrement` | font lib | number | non-minimal | yes | `builder/font.py:144,173-174` |
| `com.schriftgestaltung.fontMasterOrder` | font lib | integer | always | yes (ordering) | `builder/font.py:63,214-219` |
| `com.schriftgestaltung.fontMasterID` | font lib | string | `minimize_glyphs_diffs` | yes | `builder/masters.py:100-112` |
| `com.schriftgestaltung.weight` / `.width` / `.customName` | font lib | string | non-minimal (customName if set) | yes | `builder/names.py:46-56,119-125` |
| `com.schriftgestaltung.weightValue`, `.widthValue`, `.customValue`, `.customValue1`, `.customValue2`, `.customValue3` | font lib | number | always / if truthy | no | `builder/masters.py:55-64` |
| `com.schriftgestaltung.customParameter.GSFont.<name>` | font lib | any | unhandled font parameters; §3.5.4 attributes; `disablesLastChange`; `Virtual Master` | yes | `builder/custom_params.py:294-305,1142-1144,1150-1169` |
| `com.schriftgestaltung.customParameter.GSFontMaster.<name>` | font lib | any | unhandled master parameters; §3.5.4 attributes | yes | same |
| `com.schriftgestaltung.customParameter.GSInstance.<name>` | VF lib | any | unhandled variable-instance parameters | no | `builder/instances.py:100-113` |
| `com.schriftgestaltung.customParameter.InstanceDescriptorAsGSInstance.<name>` | instance UFO font lib | any | unhandled static-instance parameters during `apply_instance_data` | no | `builder/instances.py:481-482` |
| `com.schriftgestaltung.Glyphs.fontUserData` | font lib | dict | `to_ufos` (no designspace) | yes | `builder/user_data.py:47-50,141-151` |
| `com.schriftgestaltung.Glyphs.glyphUserData.<glyph>` | font lib | dict | glyph userData, non-minimal | yes | `builder/user_data.py:80-83,173-176` |
| `com.schriftgestaltung.Glyphs.Export` | glyph lib | bool | non-exported, without `write_skipexportglyphs` | yes | `builder/glyph.py:94,466-467` |
| `com.schriftgestaltung.Glyphs.category` / `.subCategory` / `.script` | glyph lib | string | explicit values | yes | `builder/glyph.py:154-166,501-514` |
| `com.schriftgestaltung.Glyphs.originalWidth` | glyph lib | number | zeroed non-spacing marks | yes | `builder/glyph.py:177,525-528` |
| `com.schriftgestaltung.Glyphs.lastChange` | glyph lib | string date | `minimize_glyphs_diffs` | yes | `builder/glyph.py:199-204,455-459` |
| `com.schriftgestaltung.Glyphs.glyph.{left,right,width}MetricsKey` | glyph lib | string | glyph metric keys | yes | `builder/glyph.py:252-258,485-499` |
| `com.schriftgestaltung.Glyphs.layer.{left,right,width}MetricsKey` | glyph lib | string | layer metric keys | yes | same |
| `com.schriftgestaltung.Glyphs.{left,right,width}MetricsKey` | glyph lib | string | never (legacy) | yes, onto the layer | `builder/glyph.py:485-499` |
| `com.schriftgestaltung.Glyphs.shapeOrder` | glyph lib | string of `P`/`C` | mixed layers | yes | `builder/glyph.py:209-218,542-556` |
| `com.schriftgestaltung.Glyphs.ComponentInfo` | glyph lib | array of dicts `{name, index, anchor?, alignment?}` | components with anchor/alignment | yes | `builder/components.py:82-93,173-216` |
| `com.schriftgestaltung.componentsLocked` | glyph lib | array of bools | any locked component | yes | `builder/components.py:98-101,163-171` |
| `com.schriftgestaltung.componentsSmartComponentValues` | glyph lib | array of dicts | any smart values | yes | same |
| `com.schriftgestaltung.componentsAlignment` | glyph lib | array of integers | never (legacy) | yes | `builder/components.py:163-168` |
| `com.schriftgestaltung.smartComponentAxes` | glyph lib | array of dicts | glyphs with part settings, non-minimal | yes | `builder/components.py:227-256` |
| `com.schriftgestaltung.Glyphs.nodeUserData.<p>.<n>` | glyph lib | dict | node userData | yes | `builder/user_data.py:122-126,234-240` |
| `com.schriftgestaltung.hints` | glyph lib | array of dicts | layers with hints | yes | `builder/hints.py:21-63` |
| `com.schriftgestaltung.annotations` | glyph lib | array of dicts | non-minimal | yes | `builder/annotations.py:20-54` |
| `com.schriftgestaltung.backgroundImage.crop` / `.locked` / `.alpha` | glyph lib | array of 4 numbers / bool / integer | layers with a background image, non-minimal | yes | `builder/background_image.py:20-48` |
| `com.schriftgestaltung.Glyphs._originalLayerName` | glyph lib (BRACKET glyph) | string ("" = implicit) | bracket glyphs | yes | `builder/bracket_layers.py:127-129`, `builder/builders.py:554-562` |
| `com.schriftgestaltung.layerId` | layerinfo lib | string | `minimize_glyphs_diffs` | yes | `builder/layers.py:62-63,128-129` |
| `com.schriftgestaltung.layerOrderInGlyph.<glyph>` | layerinfo lib | integer | `minimize_glyphs_diffs` | yes | `builder/layers.py:64-66,132-135` |
| `com.schriftgestaltung.Glyphs.layerName` | inside master userData `…layerLib.public.default`; popped before reaching layerinfo | string | U→G of a renamed default layer | G→U renames the layer | `builder/user_data.py:96-103,193-200` |
| `com.schriftgestaltung.export` | instance lib | bool | `minimize_glyphs_diffs` | yes | `builder/instances.py:161,245` |
| `com.schriftgestaltung.weight` / `com.schriftgestaltung.width` | instance lib | string or number | `minimize_glyphs_diffs` | yes | `builder/instances.py:162-163,279-300` |
| `com.schriftgestaltung.intanceInterpolations` (sic) | instance lib | dict | `minimize_glyphs_diffs` | yes | `builder/instances.py:165,325-332` |
| `com.schriftgestaltung.manualInterpolation` | instance lib | bool | `minimize_glyphs_diffs` | yes | `builder/instances.py:166,320-323` |
| `com.schriftgestaltung.customParameters` | instance lib | array of `[name, value]` | instances with parameters | yes | `builder/instances.py:171-173,334-336,356-358` |
| `com.schriftgestaltung.properties` | instance lib | array of `[key, value]` | instances with whitelisted properties | only by `apply_instance_data` | `builder/instances.py:174-176,359-362` |

### 12.4 Keys stored on the Glyphs side (userData) by the builder

| Key | Lives on | Type | Purpose | Where |
|---|---|---|---|---|
| `com.schriftgestaltung.Glyphs.ufoYear` | master userData | integer | `info.year` | `builder/masters.py:44-46,142-143` |
| `com.schriftgestaltung.Glyphs.ufoNote` | master userData | string | `info.note` | `builder/masters.py:47-49,144-145` |
| `com.schriftgestaltung.Glyphs.ufoData` | master userData | dict path → data | UFO `data/` | `builder/user_data.py:59-62,162-170` |
| `com.schriftgestaltung.Glyphs.ufoFilename` | master userData | string | source filename | `builder/sources.py:85-86` |
| `com.schriftgestaltung.Glyphs.layerLib.<layer>` | master userData (legacy: font userData) | dict | layerinfo lib | `builder/user_data.py:86-103,187-204` |
| `com.schriftgestaltung.Glyphs.originalFeatureCode` | master userData | string | verbatim features | `builder/features.py:52-54,457` |
| `com.schriftgestaltung.Glyphs.originalKerningGroups` | font userData | dict group → array | group order | `builder/groups.py:88,165-171` |
| `com.schriftgestaltung.Glyphs.groupsNotInFeature` | font userData | array | plain groups | `builder/groups.py:71-72,148-149` |
| `com.schriftgestaltung.Glyphs.originalOpenTypeCategory` | font userData | dict | GDEF categories | `builder/features.py:292-294,393` |
| `com.schriftgestaltung.Glyphs.backgroundWidth` | background layer userData | number | background glyph width | `builder/glyph.py:412-414,522` |
| `__layerOrder` | layer userData (temporary, removed) | integer | layer ordering | `builder/layers.py:135,148-160` |
| `PartSelection` | layer userData (v2) | dict | smart pole mapping | `classes.py:4057-4071` |
| `name`, `UFO.identifier` | node userData | string | point name / identifier | `builder/paths.py:41-42,81-83` |
| `UFO Filename`, `Master Name` | master / instance custom parameters | string | source/instance filename; master name | `builder/masters.py:114-119`, `classes.py:1827-1833` |
| `com.nagwa.MATHPlugin.extendedShape`, `com.nagwa.MATHPlugin.variants` | glyph/layer userData ↔ font lib / glyph lib | bool / dict | MATH plugin data (§7) | `builder/user_data.py:65-77,178-229`, `builder/constants.py:321-324` |
