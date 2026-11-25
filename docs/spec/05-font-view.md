# Font View: behaviour notes

Sources: Glyphs 4 Handbook, chapter "Font View"; GlyphsInfo (`GlyphData.xml`, MIT until 2024-11-22, BSD-3-Clause after,
copyright Georg Seifert; its README carries the Adobe Glyph List's BSD-3-Clause notice); agl-aglfn
(BSD-3-Clause); fontc `glyphs-reader/src/glyphdata.rs` (Apache-2.0); GlyphsSDK file-format notes.
The M4 planning pass (2026-10-10) added the reference's written tutorials at glyphsapp.com/learn:
"Adding glyphs to your font", "Alter the glyph order in Font view", "Custom sidebar entries in Font
View", "Getting your glyph names right" and "Roll your own glyph data". Rows:
`spec/inventory/05-font-view.toml`. Date: 2026-10-07, open questions answered 2026-10-10.

Ids in `code` are command ids unless marked as a panel or behaviour row.

## 1. The view

Font View is the leftmost tab of a font window and the tab a new document opens on
(`fontView.show`, Cmd+Alt+1). It has two presentations of the same glyph list, switched with two
buttons at the top left (`fontView.setViewMode`). Panel rows: `fontView.window`, `fontView.grid`,
`fontView.list`, `fontView.inspector`, `fontView.sidebar`, `fontView.searchField`,
`fontView.statusBar`.

Three things decide which glyphs are listed, in this order: the sidebar selection, the search query,
and (for order only) the glyph order or the active list filter. The status area shows three counts:
selected, visible, total.

### 1.1 Grid

- Each cell draws the glyph's layer for the master selected in the window.
- Cell size comes from the slider at the bottom right (`fontView.setCellSize`). Large cells show
  outline, name and Unicode information; small cells show only the outline.
- Unicode information sits at the bottom right beside the name. By default it is a small marker;
  for CJK glyphs the marker is the character itself drawn in the system font; with the application
  setting *Display Unicode Value* on, the full hexadecimal value is printed.
- Cell badges:

  | Badge | Position | Meaning |
  |---|---|---|
  | warning triangle | top right | a metrics key is out of sync |
  | yellow note | not stated | annotation on the active master's layer |
  | black info mark | not stated | the glyph stores properties that differ from the database |
  | red corner | top left | layers are not all interpolation-compatible |
  | red ring with a stroke | not stated | glyph does not export |

- Colour labels: a glyph colour fills the cell. A layer colour alone fills the cell except for a
  cut-out at the top left. With both, the layer colour covers the right half.
- A placed image is drawn when View → Show Image is on, and always when the glyph has no paths and
  no components.

### 1.2 List

One row per glyph, one column per property. Header click sorts, a second click reverses
(`fontView.sortByColumn`); headers can be dragged to reorder and toggled from the header context
menu (`fontView.setColumns`). Columns the chapter documents individually:

| Column | Content | Editable |
|---|---|---|
| ID | position in glyph order (the export order) | no |
| Char | first code point rendered in the system font | no |
| Name | nice name | yes, single click |
| Unicode | all code points | yes |
| Note | free text | yes |
| Components | names of the components used | yes: adds/removes components (`glyph.setComponents`) |
| Last Changed | timestamp of the last modification | no |

The chapter says every glyph property has a column but does not enumerate the rest (behaviour row
`fontView.column.otherProperties`).

*Last Changed* is rewritten on every edit of a glyph unless the font carries the parameter
*Write lastChange* set to off. We must honour that flag so files under version control do not churn.

## 2. Glyph properties

Shown in the inspector at the bottom left (`fontView.toggleInspector`; the chevron next to the
Unicode field reveals production name, script, category, subcategory:
`fontView.toggleInspectorDetails`), in List View columns, in the Selection Info window
(`edit.infoForSelection`, Cmd+Alt+I; panel `window.selectionInfo`) and partly in the glyph context
menu (panel `fontView.glyphContextMenu`). `glyph.getInfo` is our read query.

Mixed selections: a text field shows a grey *Multiple Values* placeholder and typing overwrites the
value on every selected glyph; the Exports checkbox shows a line.

| Property | Stored as (source key) | Default | Set with | Notes |
|---|---|---|---|---|
| Name | `glyphname` | – | `glyph.rename` | unique per font; identity of the glyph |
| Unicode values | `unicode` (one value or ascending list) | from database by name | `glyph.setUnicodes` | zero, one or many |
| Production name | `production` | derived, see §4.4 | `glyph.setProductionName` | used only in exported files |
| Script | `script` | from database | `glyph.setScript` | empty for digits, punctuation, spaces |
| Category / Subcategory | `category`, `subCategory` | from database | `glyph.setCategory` | none for non-exporting helper glyphs |
| Case | `case` | from database / suffix | `glyph.setCase` | upper, lower, smallCaps, minor, none |
| Writing direction | `direction` | from database, LTR when absent | `glyph.setDirection` | LTR, RTL, BiDi |
| Exports | `export` | on, unless the name starts with `_` | `glyph.setExport` | instance parameter *Remove Glyphs* can drop more |
| Colour label | `color` on the glyph | none | `glyph.setColorLabel` | index 0–11 |
| Layer colour label | `color` on the layer | none | `glyph.setLayerColorLabel` | Option in the context menu |
| Tags | `tags` (sorted list) | none | `glyph.setTags`, `glyph.addTag`, `glyph.removeTag` | comma not allowed inside a tag |
| Sort name | `sortName` | database value, else the name | `glyph.setSortName` | Selection Info only, behind a checkbox |
| Note | `note` | empty | `glyph.setNote` | searchable |
| Last changed | `lastChange` | – | automatic | see §1.2 |
| Metrics, metrics keys | layer widths; `metricLeft` / `metricRight` / `metricWidth` / `metricTop` / `metricBottom` / `metricVertOrigin` / `metricVertWidth` | – | spacing commands | inspector toggles horizontal ↔ vertical (`fontView.toggleMetricsOrientation`) |
| Kerning groups | `kernLeft`, `kernRight` (`kernTop`, `kernBottom`) | none | kerning commands | inspector fields |

Every database-backed property except Unicode (production name, script, category, subcategory,
case, direction, sort name) is *unset* in the file unless the user overrides it; an unset property
is resolved live from the database. Unicode values are different: the source formats store them on
every encoded glyph, so FontCraft stores them too, sets them from the database when a glyph is
created or its base name changes, and compares them with the database for the badge. A glyph with
at least one override carries the black info badge and matches the smart-filter rule *Has Custom
Glyph Info*. `glyph.updateGlyphInfo` (Glyph → Update Glyph Info) re-derives the selected glyphs'
properties from the database, the name included (handbook, Use Custom Naming; tutorial "Getting
your glyph names right"): legacy and alternative names become nice names and the Unicode values
follow.

**Colour labels.** Twelve fixed colours, addressed by index or name in filters and scripts:
0 Red, 1 Orange, 2 Brown, 3 Yellow, 4 Light Green, 5 Dark Green, 6 Cyan, 7 Blue, 8 Purple, 9 Pink,
10 Light Gray, 11 Dark Gray. The context menu has twelve swatches and a cross that clears the label;
holding Option makes the same swatches act on the current layer's label. Labels never reach an
exported font.

**Tags.** In the inspector's Tags field a comma or Return commits the typed text as a tag on all
selected glyphs. The field lists tags only when all selected glyphs have identical tag sets. The
context menu's Tags submenu lists every tag used in the font (the submenu is absent when there are
none) with a checkmark when the selected glyphs have the tag and a dash when only some do; picking
one adds it to all selected glyphs. Tags feed smart filters, metric scopes and feature-code tokens.

**Case.** `.sc` and `.smcp` endings give small-capital case. Minor case covers superscript and
subscript forms (`five.sups`, `ainferior`, `equalinferior`). Upper/lower also apply to some
punctuation (`question` upper, `questiondown` lower) and to marks (`acutecomb` lower,
`acutecomb.case` upper).

## 3. Managing the glyph set

### 3.1 Adding glyphs

Four entry points.

1. **`glyph.addGlyphs`** (Glyph → Add Glyphs…, Cmd+Shift+G; panel `dialog.addGlyphs`). One text
   field; items separated by spaces, line breaks or tabs (the tutorial). Item grammar:

   | Item | Example | Result |
   |---|---|---|
   | glyph name | `Ccedilla`, `beh-ar.medi` | glyph with that name |
   | literal character | `Ç` | glyph for that code point, under its nice name |
   | code-point name | `uni00C7`, `u10330` | same, converted to the nice name when one exists |
   | range `a:b` | `uni0218:uni021B`, `alpha:omega` | every code point from a to b inclusive |
   | recipe | `x+acutecomb=xacute`, `Eth=Dtail` (components chapter) | composite built from named parts (M6) |

   Conversion to nice names, legacy names such as `dotlessi` included, is skipped when the font has
   *Use Custom Naming* on. The dialog's confirm button is *Generate* (tutorial).

   If any requested name already exists, nothing is added yet; a dialog
   (`dialog.addGlyphsExisting`) lists the existing names and offers *Replace* (they become new empty
   glyphs), *Keep Existing* (only the genuinely new names are added) or *Cancel* (nothing is added).
   Headless: `onExisting` selects the outcome; the default for agents should be `keep`.

2. **`glyph.newGlyph`** (Cmd+Alt+Shift+N, or the plus button): one empty glyph named `newGlyph`.
   Its name is selected for typing; a typed legacy name or character becomes the nice name on
   Return (tutorial).

3. **Glyph Info window** (`window.glyphInfo` → `glyphInfo.addToFont`): add selected database rows.

4. **Sidebar** (`fontView.listMissingGlyphs`, `fontView.generateMissingGlyphs`; panel
   `dialog.missingGlyphs`): entries with a known full set show a grey present/total badge when
   incomplete and a checkmark when complete. The entry's context menu opens the list of missing
   names; *Generate* adds the selected ones (Cmd+A selects all).

A new glyph receives its Unicode values, category, subcategory, script and default composition from
the database record of its name (§4).

### 3.2 Duplicating

`glyph.duplicateGlyph` (Cmd+D) copies each selected glyph within the font. The copy gets the
suffix `.001`, or `.002` when that is taken, and so on; the copies become the selection (tutorial
"Adding glyphs to your font").

### 3.3 Copy and paste between fonts

`edit.copy` / `edit.paste` in Font View transfer whole glyphs.

- Name clash on paste: the incoming glyph is renamed with the first free `.001`, `.002`, … suffix.
- Components are re-pointed by name at the target font's glyphs. A missing base leaves an
  unresolved component drawn as the *no base glyph* triangle.

`edit.pasteSpecial` (Cmd+Alt+V; shown in the Edit menu while Option is held; panel
`dialog.pasteSpecial`):

| Mode | Effect |
|---|---|
| Glyphs With Same Name | overwrite same-named targets; add copied glyphs that have no counterpart |
| Selected Glyphs | overwrite the selected targets in order, keeping the targets' names; surplus copied glyphs are dropped |
| As Components | insert each copied glyph as a component into the selected glyphs, keeping existing shapes; *All Masters* does it on every master |

For the first two modes a part selection limits what is transferred:

| Part | Transfers |
|---|---|
| All Data | everything: shapes, anchors, layer attributes, metrics (implies the rest) |
| Content of Active Layer | only the layer displayed at copy time |
| Kerning Groups | left and right group names |
| Metric Keys | sidebearing and width formulas only; run Update Metrics afterwards to apply them |
| LSB / RSB / Width | the numeric values; with RSB and Width both chosen, RSB wins |

### 3.4 Removing

`glyph.removeGlyph` (Cmd+Delete, or the minus button) acts on the Font View selection, the current
glyph in Edit View, or the Text-tool selection. Sequence:

1. Confirmation (`dialog.removeGlyph`): *Remove* or *Cancel*.
2. If a doomed glyph is used as a component elsewhere, the same dialog adds *Decompose*, which
   becomes the default button. *Remove* deletes the glyph and every component that points at it.
   *Decompose* first converts those components to outlines, then deletes.
3. If metrics keys in surviving glyphs point at a doomed glyph (`dialog.removeGlyphMetricsKeys`):
   *Remove* dissolves those keys; *Cancel* keeps them dangling. A dangling key is shown in
   turquoise and is repaired by re-pointing it, replacing it with a number, or recreating the glyph.

Headless parameters `components` and `metricsKeys` pick the answers; without them the command
refuses when either situation arises.

Kerning pairs that name a doomed glyph stay. They are obsolete pairs until Clean Up
(`kerning.obsoletePairs`).

## 4. Glyph data

### 4.1 What a record holds

The handbook lists, per database glyph: nice name, production name, Unicode value, components (for
composites), default anchors, marks for the mark cloud, script, category and subcategory.

The openly licensed data (GlyphsInfo) is XML: a `glyphData` root holding empty `glyph` elements.
Its DTD defines exactly these attributes:

| Attribute | Required | Meaning | Form |
|---|---|---|---|
| `name` | yes | nice name, the primary key | ASCII glyph name |
| `category` | yes | top-level class | `Letter`, `Mark`, `Number`, `Punctuation`, `Separator`, `Symbol` |
| `subCategory` | no | finer class | see list below |
| `unicode` | no | the code point the name maps to | uppercase hex, 4–6 digits, no prefix |
| `unicodeLegacy` | no | compatibility / presentation-form code point | same form |
| `production` | no | export name when it differs from `name` | glyph name |
| `script` | no | writing system | lowercase id, e.g. `latin`, `arabic`, `cyrillic`, `han` |
| `case` | no | `upper`, `lower`, `minor`, `smallCaps` | absent = no case |
| `direction` | no | `LTR`, `RTL` or `BIDI` | absent: from the script (see below), else LTR |
| `description` | no | Unicode character name or a descriptive phrase | text |
| `altNames` | no | other names that resolve to this record | comma-separated |

Subcategories present in the data: Arrow, Compatibility, Currency, Dash, Decimal Digit, Emoji,
Enclosing, Format, Fraction, Geometry, Halfform, Jamo, Letter, Ligature, Math, Matra, Modifier,
Nonspacing, Parenthesis, Quote, Radical, Small, Space, Spacing, Spacing Combining, Superscript,
Syllable. Format readers additionally accept the categories Space and Other and the subcategories
Composition, Conjunct, Lowercase, Number, Other, Smallcaps, Uppercase, so a parser must not reject
them.

Size and layout: about 34 000 records in the main file covering 82 script ids, and about 40 000 Han
ideographs in a second file (`GlyphData_Ideographs.xml`) whose names are code-point names and which
carry little beyond category, subcategory and `script="han"`. The data does not cover all of
Unicode.

The tab-separated form used for editing the data lists these columns: `unicode`, `name`, `sortName`,
`sortNameKeep`, `decompose`, `category`, `subCategory`, `script`, `production`, `altNames`,
`description`, `anchors`, `accents`. So the full record also has:

| Field | Meaning | In the published XML |
|---|---|---|
| `sortName` | default ordering key | no |
| `sortNameKeep` | ordering key for the mode that keeps alternates beside their base | no |
| `decompose` | component recipe of a composite | no |
| `anchors` | default anchor names | no |
| `accents` | marks offered in the mark cloud | no |

These five match the handbook's "components, anchors, marks" and the sort-name property, but their
values are not in the public data. The tutorial "Roll your own glyph data" documents them as XML
attributes too (`sortName`, `sortNameKeep`, `decompose`, `anchors`, and `marks`, called `accents`
in version 2), so a custom `GlyphData.xml` may carry them and FontCraft reads them there. The same
tutorial says a missing `direction` is filled from an internal list of directions per script
(a Hebrew glyph is right to left without the attribute); FontCraft keeps that list as data of its
own: the scripts Unicode writes right to left are RTL. FontCraft must either author them or derive them (for example
`decompose` from Unicode canonical decomposition). See Open questions.

Naming scheme of nice names: Adobe Glyph List names where they exist, adjusted in places; otherwise
a short readable name derived from the Unicode description. A hyphen plus script tag separates
scripts, short for common scripts (`-ar`, `-cy`) and the full script name for most others.

### 4.2 Resolving properties for a glyph

Each property is resolved independently, first hit wins:

1. **Value stored on the glyph** in the source file (the user's override).
2. **Custom glyph data** (a user `GlyphData.xml`): entry whose name equals the glyph name; failing
   that, an entry whose code point equals one of the glyph's code points.
3. **Built-in data**, tried by: nice name; then production name (so `uni0410` finds `A-cy`); then
   the glyph's code points in order.
   - Alternative names are registered as extra keys after all primary names, so they can never
     shadow a real name. A hit through an alternative name supplies category, subcategory, script
     and production name, but not a code point.
4. **Synthesis from the name** (below).
5. Otherwise the property is empty.

Commands: `glyphData.lookup` exposes this, with provenance per field.

#### Name → Unicode

- Direct record: its `unicode` value.
- `uniXXXX` (four hex digits) or `uXXXXX` / `uXXXXXX` (five or six): the code point itself.
- A suffixed name (`A.alt`) has no code point: the Unicode value belongs to the base glyph only.
- `unicodeLegacy` is never assigned: the tutorial "Roll your own glyph data" says such a value is
  not written into a font.
- No record and not a code-point name: no Unicode value; the user sets one by hand
  (`glyph.setUnicodes`). The handbook suggests the Private Use Area (for example U+E000–U+F8FF) for
  characters outside Unicode, and warns against reusing a registered name for a custom glyph.

#### Unicode → name (adding by character or by `uniXXXX`)

Find the record whose `unicode` equals the code point and use its `name`. Without a record, keep the
code-point name (`uni` + 4 digits, or `u` + 5–6 digits). Skipped when *Use Custom Naming* is on.

#### Name → category and subcategory (synthesis)

1. A name starting with `_` gets no category.
2. Split off the suffix. Normally the base is everything before the first dot. With more than one
   dot, try successively longer prefixes ending at the second, third, … dot and take the first that
   is a known name; this finds records whose own name contains a dot.
3. Look up the base (rules 2–3 of the list above). A hit supplies category and subcategory. This is
   the handbook's rule that `A.alt`, `A.ss01` and `A.001` behave like `A`, whereas `Aalt` or `A001`
   inherit nothing.
4. If the base contains `_`, treat it as a ligature:
   - Take a script suffix (`-xx`) found on the last part and, for each part lacking one, try
     `part-xx`; keep the bare part when the bare name is known and the suffixed one is not.
   - Look up every part. Let *first* be the first part's record.
     - *first* is a Mark → Mark with *first*'s subcategory.
     - *first* is a Letter and every other part is a Mark, a Separator or unknown → *first*'s
       category and subcategory.
     - *first* is a Letter otherwise → Letter / Ligature.
     - anything else → continue.
5. AGL fallback: interpret the base by the Adobe Glyph List rules (§4.5), take the first resulting
   character, and map its Unicode general category:

   | General category | Category | Subcategory |
   |---|---|---|
   | Lu, Ll, Lt, Lo | Letter | – |
   | Lm | Letter | Modifier |
   | Mn | Mark | Nonspacing |
   | Mc | Mark | Spacing Combining |
   | Me | Mark | Enclosing |
   | Nd, No | Number | Decimal Digit |
   | Nl | Number | – |
   | Zs | Separator | Space |
   | Zl, Zp, Cc | Separator | – |
   | Cf | Separator | Format |
   | Co | Letter | Compatibility |
   | Pd | Punctuation | Dash |
   | Ps, Pe | Punctuation | Parenthesis |
   | Pc, Po | Punctuation | – |
   | Pi, Pf | Punctuation | Quote |
   | Sm | Symbol | Math |
   | Sc | Symbol | Currency |
   | Sk | Mark | Spacing |
   | So, Cn | Symbol | – |

   If the base contains `_` and the result is not a Mark, the subcategory becomes Ligature.
6. Nothing matched: no category.

Script, case and direction are not synthesised by the open-source implementations beyond inheriting
from the base record; the handbook adds the `.sc` / `.smcp` → small-capital rule.

#### Renaming

`glyph.rename` re-runs resolution only when the base name changes (`eacute` → `plus` updates Unicode
and category; `eacute.sc` → `eacute.pc` changes nothing else). Since unset properties are resolved
live, "updating" means: drop nothing the user overrode, and let unset properties follow the new
name. Whether the reference also rewrites stored Unicode values on rename is an open question.

### 4.3 Naming rules

Validity (`font.validateGlyphNames`; behaviour rows `glyphName.*`):

- Allowed characters: `A–Z a–z 0–9 _ - .` and nothing else: no white space, no non-ASCII.
- The first character is a letter; only non-exporting glyphs (usually `_…`) and `.notdef` are
  exempt.
- Names are case-sensitive and unique within a font.
- An invalid name blocks export with an *Illegal glyph name* error that gives the position of the
  offending character and offers *Show* to open the glyph. Feature compilation reports the same.

Conventions that carry meaning:

| Pattern | Meaning | Example |
|---|---|---|
| `_name` | helper glyph, not exported by default | `_part.x`, `_smart.x`, `_cap.x`, `_corner.x` |
| `name-script` | script variant; Latin and Greek mostly bare | `A-cy`, `noon-ar`, `-hb`, `-ko`, `-thai`, `-math`, `-braille`, `-fraktur`, `-musical` |
| `name.suffix` | alternate of `name`, inherits its properties | `h.sc`, `beh-ar.medi` |
| `name.s1.s2` | chained suffixes, in the order of the features in Font Info | `x.sc.ss01` |
| `a_b_c` | ligature of a, b, c | `f_f_l` |
| `a_b-script.suffix` | script and variant suffixes once, at the end | `lam_alif-ar.fina` |
| `uniXXXX`, `uXXXXX` | glyph named by code point | `uni5B57`, `u20547` |

*Use Custom Naming* (Font Info → Other) disables conversion to nice names for a font. Automatic
features and automatic categories then stop working unless a custom glyph database describes the
names in use.

Batch renaming: `edit.findAndReplace` (Edit → Find → Find and Replace…, Cmd+Shift+F; panel
`dialog.findAndReplaceGlyphNames`) on the selected glyphs.

- Plain mode: every occurrence of *Find* in each name is replaced by *Replace*.
- Empty *Find*: *Replace* is appended to each name (adding `.ss05` to a selection).
- *Regex*: *Find* is a regular expression; *Replace* may reference groups as `\1`, `\2`, …
  (`\.cv(\d\d)` → `.ss\1` turns `x.cv03` into `x.ss03`).
- Each rename obeys §4.2 "Renaming". A result that collides with an existing name must be reported;
  the handbook does not say what the reference does.

Copying names: `glyph.copyGlyphNames` with a `format`:

| Format | Output for five, six, seven |
|---|---|
| Space Separated | `five six seven ` (space after every name) |
| One per Line | one name per line |
| Comma Separated | `five, six, seven, ` (comma and space after every name) |
| Slashed | `/five/six/seven` (pastes into Edit View as text) |
| Python List | `"five",` on a line each |
| Unicodes | one line per glyph: hex code points padded to ≥ 4 digits, several joined by `, `; glyphs without one as `- # name` |

### 4.4 Production names at export

Nice names are for people; exported fonts carry production names that follow the Adobe Glyph List
conventions so that text can be recovered from glyph names. For each exported glyph
(`glyphData.productionName`; behaviour rows `export.productionName*`):

1. The glyph's own production-name override, if set.
2. Else the record found by §4.2: its `production` attribute. A record without one exports under
   the glyph's own name; for a record found by its nice name that is the nice name, which is
   already acceptable (`Aacute`). Both oracles do the same for a record found by code point or
   alternative name.
3. Else synthesis:
   1. Split base and suffix as in §4.2.
   2. If there is a suffix and the base resolves to a record with a production name, the result is
      that production name + `.` + suffix (`A-cy.sc` → `uni0410.sc`).
   3. Otherwise split the base into ligature parts (script suffix distributed as in §4.2); a name
      without `_` is a single part. Resolve each part to its production name, or to itself when it
      is an AGLFN name. If any part cannot be resolved, there is no synthesised name.
   4. If at least one part is a `uniXXXX` name and no part is a `u`-name above the BMP, write
      `uni` followed by the four-digit hex of every part in order (AGLFN-named parts contribute
      their code point): `lam_alef-ar` style names become `uni06440627`. A part whose production
      name is none of these (`i.loclTRK`) leaves the name unsynthesised. A `uniXXXX` part with a
      suffix of its own (`uni0C2D.half` in `bh_a-telugu`) is joined as it is
      (`uni0C2D.half0C05`), as fontc does; glyphsLib gives up on such a name and writes the glyph
      name without the characters production names may not hold.
   5. Otherwise join the parts with `_`.
   6. Re-attach `.suffix`.
4. Else the glyph's own name unchanged.

Form of code-point names: `uni` + exactly four uppercase hex digits in the BMP; `u` + five or six
uppercase hex digits above it.

Related rules:

- A font-level switch can export nice names unchanged (stored as `disablesNiceNames` in the source
  format); it is outside this chapter.
- CID-keyed CJK fonts address glyphs by CID. A Registry–Ordering–Supplement maps nice names to
  CIDs; exporting glyphs it does not cover are appended after the mapped range
  (`export.cidMapping`).
- Opening a compiled font with *Keep Glyph Names from Imported Files* on preserves its order by
  adding a `glyphOrder` parameter (`import.keepGlyphNames`).

### 4.5 Adobe Glyph List files

- `glyphlist.txt` (AGL, about 4 300 records): `name;XXXX[ XXXX…]`, a glyph name mapped to one or
  more code points.
- `aglfn.txt` (AGLFN, 586 records): `XXXX;name;CHARACTER NAME`, the subset recommended for new
  fonts, omitting names whose AGL mapping is a PUA or presentation-form value (`Asmall`, `ffi`).
- `#` starts a comment; blank lines are ignored.

Name → text, per the AGL specification: drop everything from the first `.`; split the rest at `_`;
map each part by (a) the glyph list, (b) `uni` followed by one or more groups of four uppercase hex
digits, none a surrogate, (c) `u` followed by four to six uppercase hex digits naming a scalar
value; a part matching none maps to nothing. FontCraft uses AGLFN membership to decide whether a
nice name can be exported as is, and the AGL mapping for the category fallback in §4.2.

### 4.6 Glyph Info window

`window.glyphInfo` (Window → Glyph Info): table of all records, searchable and sortable by column,
with *Add to Font* (`glyphInfo.addToFont`). It is also the documented place to look up script
suffixes. `glyphData.search` is the headless equivalent. The tutorial "Adding glyphs to your font"
gives the columns (name, Unicode value, category, subcategory, script, composition) and says the
search matches names, Unicode values and characters, so typing `-deva` lists Devanagari.

A custom `GlyphData.xml` changes properties for every file on the machine; per-glyph overrides (§2)
change them for one file. A `GlyphData.xml` beside a source file, or in an `Info` folder beside it,
applies to that file unless its *Ignore custom GlyphData file* parameter is set. Custom records
complement the built-in ones: a custom file may hold only the glyphs it changes. A file that is
not well-formed XML is ignored as a whole (tutorial "Roll your own glyph data"); FontCraft also
reports it.

## 5. Selection and batch processing

- `edit.selectAll` (Cmd+A) selects all *visible* glyphs; `edit.deselectAll` (Cmd+Alt+A) clears;
  `edit.invertSelection` (Cmd+Alt+Shift+I) flips. `fontView.setSelection` sets it by name.
- Glyph- and Path-menu commands run from Font View apply to every selected glyph on the current
  master's layer. Those marked ✓ below extend to all masters while Option is held; for the rest,
  Option either does nothing documented or switches to the variant shown.

| Command | Shortcut | Option = all masters | Option variant |
|---|---|---|---|
| Glyph → Remove Glyph | Cmd+Delete | | |
| Glyph → Duplicate Glyph | Cmd+D | | |
| Glyph → Update Glyph Info | | | |
| Glyph → Update Metrics | Cmd+Ctrl+M | ✓ | |
| Glyph → Transform Metrics | | | |
| Glyph → Add Component | Cmd+Shift+C | ✓ | |
| Glyph → Create Composite | Cmd+Ctrl+C | ✓ | |
| Glyph → Decompose Components | Cmd+Shift+D | ✓ | |
| Glyph → Add Image… | | | |
| Glyph → Set Anchors | Cmd+U | ✓ | |
| Glyph → Reset Anchors | Cmd+Shift+U | ✓ | |
| Path → Reverse Contours | Cmd+Ctrl+Alt+R | | |
| Path → Correct Path Direction | Cmd+Shift+R | ✓ | |
| Path → Round Coordinates | | ✓ | |
| Path → Tidy up Paths | Cmd+Shift+T | ✓ | |
| Path → Add Extremes | | | Force Extremes |
| Path → Remove Overlap | Cmd+Shift+O | ✓ | |
| Path → Transformations | | | |
| Path → Selection to Background | Cmd+J | | Add Selection to Background (Cmd+Alt+J) |
| Path → Assign Background… | | | Clear Background |
| Path → Swap with Background | Cmd+Ctrl+J | | |
| Path → Interpolate with Background | | | |
| Path → Other → Convert to Cubic | | | |
| Path → Other → Convert to Quadratic | | | |

Also batch-capable: every Filter-menu filter; the Palette (Cmd+Alt+P) sections Fit Curve and
Transformations, where node-level controls use each glyph's Edit View node selection; plug-ins and
scripts that choose to. Behaviour rows: `fontView.batch.*`.

Design consequence: every such command takes `glyphs?` and `allMasters?` parameters; the UI fills
`glyphs` from the Font View selection and `allMasters` from the Option key.

## 6. Filtering

### 6.1 Search

`fontView.focusSearch` (Cmd+F), `fontView.setSearch`. The term is matched against glyph name, hex
code point, the literal character, and the note. The magnifier menu limits matching to name,
Unicode value or note, and has two switches:

- *Match Case*: off by default; when on, `Oslash` finds `Oslash` and `Oslashacute` but not `oslash`.
  Matching is by substring.
- *Regex*: the term is a regular expression. The handbook's examples (`.` finds one-character
  names, `..` two-character names) imply the expression must match the whole name.

### 6.2 Sidebar

`fontView.selectSidebarEntries`; headless evaluation with `fontView.queryGlyphs`.

- **All** shows the whole font (with an empty search field).
- **Categories**: one entry per category, expandable to subcategories with counts.
- **Languages**: scripts, expandable to language/subset filters. Only a default short list is shown
  plus any script the font has glyphs for; the plus button opens a checklist
  (`fontView.setVisibleScripts`).
- **Filters**: smart filters, list filters, folders.
- Custom Categories and Languages entries can come from a `Groups.plist` in the app's Glyphs Info
  folder (format in another chapter).

Combination rules:

| Selection | Result |
|---|---|
| several Categories entries (Cmd-click) | union |
| several scripts in Languages | union |
| entries from different sections | intersection |
| several entries in Filters (Cmd- or Shift-click) | intersection |
| a filter folder | union of its contents |

Filtering never affects export order.

### 6.3 Smart filters

`fontView.addSmartFilter`, `fontView.editFilter` (or double-click the gear icon); panel
`dialog.smartFilterEditor`. A filter is a name and a rule tree. A new filter has one rule; plus adds
a rule, minus removes one. Option turns plus into a dots button that adds a nested group of kind
**All**, **Any** or **None**; groups nest.

Rules and what they inspect:

| Rule | Scope | Type |
|---|---|---|
| Glyph Name | glyph | text |
| Count of Paths | first master's layer | number |
| Count of Components | first master's layer | number |
| Tags | glyph | text set |
| Script | glyph | choice |
| Category & Subcategory | glyph | choice |
| Case | glyph | choice: upper, lower, small capital, none |
| Master Compatible | glyph | bool |
| Export Glyph | glyph | bool |
| Has Unicode | glyph | bool |
| Has Components | selected master's layer | bool |
| Has Hints | selected master's layer | bool (manual hints of either kind) |
| Has PostScript Hints | selected master's layer | bool |
| Has TrueType Hints | selected master's layer | bool |
| Has Corners | selected master's layer | bool |
| Has Anchors | selected master's layer | bool |
| Contains Component | selected master's layer | glyph name |
| Has Special Layers | glyph | bool |
| Has Custom Glyph Info | glyph | bool |
| Has Annotations | selected master's layer | bool |
| Is Auto-Aligned | selected master's layer | bool |
| Has Metrics Keys | glyph | bool |
| Metrics Keys Out of Sync | glyph | bool |
| Has Kerning Groups | glyph | bool |
| Color Label | glyph | colour or Not Set |
| Layer Color Label | selected master's layer | colour or Not Set |
| Is Hangul Key | glyph, via *Hangul Composition Groups* | bool |
| Custom | – | glyph predicate expression (same language as feature-code predicate tokens) |

Note the two count rules read the *first* master while the other layer rules read the *selected*
master, so a filter's result can change with the master selection.

Proposed rule-tree JSON for `fontView.addSmartFilter` / `fontView.queryGlyphs`:
`{all|any|none: [rule…]}` with `rule = {property, op, value}` or a nested group.

### 6.4 List filters

`fontView.addListFilter`; panel `dialog.listFilterEditor` (Name, a text area of names separated by
spaces or line breaks, OK / Cancel). A new list filter is pre-filled with the selected glyphs.
Selecting it shows the font's glyphs that are in the list, **in list order**; sort names are
ignored. Listed names missing from the font can be generated from the sidebar (§3.1). *Copy Glyph
Names → One per Line* produces text in the accepted form.

### 6.5 Managing filters

Everything here goes through the bottom-left actions menu, or by direct manipulation:

- drag to reorder or move into a folder (`fontView.moveFilter`); folders nest
  (`fontView.addFilterFolder`);
- double-click the icon to edit; double-click the name, or select and press Return, to rename
  (`fontView.renameFilter`);
- *Remove Filter* deletes the selected filter or folder; a folder takes its contents with it
  (`fontView.removeFilter`).

**Where filters live.** The tutorial "Adding glyphs to your font" says sidebar entries, filters
included, are available in all files; "Custom sidebar entries in Font View" says the reference
keeps them in one `CustomFilter.plist` in its application support folder, and that a file named
`CustomFilter <title>.plist` beside a source adds a per-file section. FontCraft keeps filters in the
user settings (`settings.get`, `settings.set`; M4). Per-file filter files are not read in M4: their
smart filters are predicates over the reference's scripting properties, which no source lists.

**Custom Categories and Languages.** The tutorial "Custom sidebar entries in Font View" documents
`Groups*.plist` files (old-style property lists) in the application's `Info` folder: a
`categories` and a `languages` list of entries, each with a `name`, an optional `icon`, a `script`
or `predicate`, and either a `subGroup` list of entries or a `list` of glyph names (a character in
place of a name means its code-point name). FontCraft reads the same files from its own settings
folder.

## 7. Glyph order

Order in Font View (when no list filter is active) and in exported fonts is the same:

1. If the font has a `glyphOrder` parameter (Font Info → Font; glyph names, one per line), that
   order. Sort names are then ignored.
2. Otherwise by sort name: the glyph's own sort name if set, else the database sort name, else the
   glyph name.

The ID column of List View shows the resulting index. Sidebar filters and search never change it.

The tutorials "Alter the glyph order in Font view" and "Roll your own glyph data" add: the default
orders glyphs alphabetically within their category; glyph data can set a `sortName` (and a
`sortNameKeep` for *Keep Alternates Next to Base Glyph*); and a `glyphOrder` line written `**Title**`
shows as a section header in Font View. The published data has no sort names, so FontCraft's key is
its own (`glyphOrder.default` guesses). An opened file keeps its stored order, which is also the
order the oracles compile; a glyph added, renamed or given a sort name later takes the place the key
gives it.

## 8. Images

- Drop an image file on a cell: placed on that glyph's active layer.
- `glyph.addImage` (Glyph → Add Image…, defined in the Edit View inventory) with one file: placed
  on the selected glyph.
- With several files: each file goes to the glyph whose name equals the file name without
  extension (`Thorn.png` → `Thorn`), on the current master's layer.
- Glyph names are case-sensitive, many file systems are not: on those, uppercase and lowercase
  images must sit in different folders. On case-sensitive systems match exactly.
- Display: View → Show Image toggles images in cells; an empty glyph always shows its image.

## Open questions

Answers from the M4 planning pass (2026-10-10). "Source" answers come from the handbook or a
written tutorial and are documented, not yet observed. "Ours" answers are best guesses, recorded in
the named row's `guesses`; an observation settles them. Questions marked ▶ are worth a tutorial
video (`docs/plans/M4.md` § 7).

FontCraft does what every answer below says (M4, `docs/plans/M4.md` § 5 › Results). Still open:
the decomposition, anchor and mark data of question 1 (M6), the full Selection Info layout of
question 15, question 16 (M6) and question 20 (M11).

M4.20 (2026-11-25) read the reference's tutorials, release notes ("What's new in Glyphs",
updates.glyphsapp.com), scripting documentation (docu.glyphsapp.com) and forum answers for every row
with guesses. What they settle is in the rows' `notes` with the source; questions 21 to 25 list where
FontCraft differs. Every guess left that the reference could settle has a step in `observe`.

1. **Database fields not in the public data.** `sortName`, `sortNameKeep`, `decompose`, `anchors`
   and `accents` exist as columns but ship with no values in GlyphsInfo. What does the reference
   use, and can we derive equivalents (Unicode decomposition, our own anchor tables) well enough for
   Create Composite, Set Anchors, the mark cloud and the default glyph order to match?
   **Partly answered (source):** a custom `GlyphData.xml` may carry all five, and FontCraft reads
   them there (§ 4.1). The built-in values stay unpublished. For M4 only the sort key matters
   (question 2). Decomposition, anchors and marks stay open for M6 (open decision D6).
   **Answered for M6 (the owner's answer to `docs/plans/M6.md` § 7, question 3 (a)):** Create
   Composite takes Unicode canonical decomposition mapped to glyph names plus our own tables, and
   Set Anchors our own default-anchor table; a custom `GlyphData.xml` overrides both
   (`glyph.createComposite`, `glyph.setAnchors` guesses). The mark cloud is M6.33.
2. **Default sort order.** ▶ **Answered for M4 (source and ours):** the tutorials say glyphs sort
   alphabetically within their category, adjustable by sort names. The key's details are ours
   (`glyphOrder.default`). An opened file keeps its order.
   **Narrowed (release notes, tutorial):** from version 4 the reference sorts by script first, keeps
   numbers and punctuation in groups per script, and compares sort names with the glyph names in
   alphabetical order; question 23.
3. **`unicodeLegacy`.** **Answered (source):** never assigned and never written into a font
   (`glyphData.legacyUnicode`).
4. **Rename and stored Unicode.** ▶ **Answered for M4 (source and ours):** a base-name change takes
   the new name's database values (handbook); a name the database does not know keeps the stored
   values (tutorial). Ours: a new name with a database code point replaces values the user typed
   (`glyph.rename`).
5. **Update Glyph Info.** ▶ **Answered for M4 (source and ours):** it renames to nice names and
   re-syncs Unicode values, respecting custom data (handbook, tutorial). Ours: it drops stored
   overrides of the other database properties and keeps names under *Use Custom Naming*
   (`glyph.updateGlyphInfo`).
6. **Duplicate Glyph naming** and **New Glyph naming when `newGlyph` exists**. **Answered (source
   for duplicates, ours for New Glyph):** `.001`, then `.002`; the copies become the selection.
   `newGlyph.001` follows the same rule (`glyph.newGlyph`).
7. **Add Glyphs parsing.** **Answered for M4 (source and ours):** spaces, line breaks and tabs
   separate items (tutorial). Ours: commas do not; a range takes every assigned code point between
   its ends, at most 65,536; recipes wait for M6 (`glyph.addGlyphs`).
8. **Replace in Add Glyphs.** ▶ **Answered for M4 (ours):** a fresh empty glyph with database
   properties, in the old glyph's place; references by name stay (`glyph.addGlyphs`).
9. **Find and Replace collisions.** ▶ **Answered for M4 (ours):** a batch with a clash or an
   invalid result is refused as a whole; renames update components, kerning, the `glyphOrder`
   parameter and metrics keys, not feature code (`edit.findAndReplace`, `glyph.rename`).
   **Narrowed (release notes):** the reference prevents duplicate names here (build 3177) and, from
   version 4, brings up a dialog about the uses of renamed glyphs (build 3854); question 25.
10. **Smart filter operators.** ▶ **Answered for M4 (ours):** text rules offer is, is not,
    contains, does not contain, begins with, ends with and matches (regex); number rules =, ≠, <,
    ≤, >, ≥; yes-or-no rules is and is not; choice rules is and is not. The tutorial "Custom sidebar
    entries in Font View" shows the reference's stored predicates use `==`, `!=`, `<`, `>`,
    `CONTAINS`, `IN`, `BEGINSWITH` and `ENDSWITH`, which this set covers. *Case* offers minor too
    (`smartFilter.rule.case`).
11. **Smart filter refresh.** **Answered for M4 (ours):** live, after every edit and on a master
    change (`smartFilter.live`). **Narrowed:** a developer confirms that the reference's filter for
    invalid metrics keys is not always updated after a change (forum.glyphsapp.com/t/35652, 2026).
12. **Invert Selection scope.** **Answered for M4 (ours):** the visible glyphs, as Select All
    (`fontView.invertSelection`).
13. **Regex search anchoring and target.** **Answered for M4 (ours):** a regex matches a whole
    field, in every field the magnifier menu selects (`fontView.search.regex`). For plain terms see
    question 22.
14. **List View columns.** **Answered for M4 (source and ours):** one column per property of the
    Glyph Properties section (handbook). Ours: all shown by default; editable where a set command
    exists (`fontView.column.otherProperties`).
15. **Inspector and Selection Info fields.** ▶ **Answered for M4 (ours):** case and direction are
    edited in Selection Info and List View; the badge corners are in the cell rows' `guesses`.
    The full Selection Info layout stays open.
16. **Components cell syntax** in List View. **Answered for M6 (ours):** names separated by a
    comma and a space; an added name appends a component, a removed name deletes the last one of
    that name, on the layer of the master shown (`glyph.setComponents`).
17. **Tags context menu.** **Answered (source):** choosing a tag always adds it; tags are removed
    in the Tags field (`fontView.contextMenu.tags`).
18. **Paste Special pairing.** ▶ **Answered for M4 (ours):** copy order against the targets'
    glyph order; *Content of Active Layer* writes the shown master's layer (`edit.pasteSpecial`).
19. **`glyphOrder` gaps.** **Answered (source):** glyphs the parameter does not name follow in the
    default order (`glyphOrder` parameter row). `.notdef` follows the same rule in Font View; export
    still moves it to glyph 0.
20. **Batch image import.** ▶ **Answered for M11 (handbook, ours):** file names without extension
    match glyph names exactly, case included; each image goes on the current master's layer;
    unmatched files are listed in the result and create no glyph (`fontView.image.batchByFileName`).
21. **Missing glyphs: by name or by character.** **Answered (source, version 2):** by glyph name; a
    glyph that has the character under another name counts as missing (forum.glyphsapp.com/t/5422,
    a developer's answer of 2016). FontCraft also counts a glyph that has the expected code point.
    Code change: `Catalog::present` in `crates/glyphquery/src/sidebar.rs` (the name alone). The
    source is old and the code point rule serves fonts with other naming schemes, so observe first
    (`fontView.sidebar.missingBadge`).
22. **Search terms.** **Answered in part (release notes):** a term can hold several name parts
    (build 3201), and a search by character also finds its unencoded alternates (build 3193). How
    parts are separated and combined stays open. Code change: `Matcher::new` and `Matcher::matches`
    in `crates/glyphquery/src/search.rs`, which take the whole term as one part and match a
    character by the code points alone (`fontView.search.fields`).
23. **Sort key.** **Answered in part (release notes of version 4, tutorial):** script before
    category, numbers and punctuation in a group per script, alphabetical order within a category,
    and a sort name compared in that order in place of the glyph name. The order of the scripts
    and of the categories stays open. Code change: `GlyphData::sort_key` in
    `crates/glyphdata/src/order.rs`, which has no script, orders by code point and puts glyphs with
    a sort name last (`group: 2`) (`glyphOrder.default`).
24. **The user's glyph data files.** **Answered (tutorial, release notes):** the user's Info folder
    and, from build 3837, a system-wide one are read. Code change: nothing in `crates/ui` calls
    `Session::set_user_glyph_data` (`crates/engine/src/lib.rs`), so only files beside the document
    apply (`glyphData.customFile`).
25. **Features of the release notes without a row.** The detail view between grid and list
    (build 3831), the note as a tooltip (3835), the Unicode information popover (3841), the option
    that hides sidebar counts (3852), CJK blocks in the sidebar (3845), the smart filter rules
    Metrics Keys Invalid (3315) and Has Background Image (3328), single characters in a list filter
    (3177), production names as a seventh format of Copy Glyph Names (3330; code change:
    `COPY_FORMATS` in `crates/engine/src/cmd/glyph_info.rs` and the menu in
    `crates/ui/src/app/context_menu/glyph_menu.rs`), newly added glyphs shown whatever the filter
    (3826), the dialog about uses of a renamed glyph (3854; `rename` in
    `crates/engine/src/cmd/glyph_info.rs` renames without asking), and Find filled with the glyph
    names in Find and Replace (3854). Each needs a row, or a change of one, when it is built.
