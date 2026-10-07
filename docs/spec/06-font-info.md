# Font Info

Source: Glyphs 4 Handbook, chapter "Font Info" (https://handbook.glyphsapp.com/font-info/), read
2026-10-07. Source-file key names come from the GlyphsSDK file-format documentation
(`GlyphsFileFormatv3.md`, `GlyphsFileFormatv4.md`) and are marked as such.
Inventory: `spec/inventory/06-font-info.toml`.

Font Info is one window per document with six tabs: Font, Masters, Exports, Features, Other
Settings, Notes. Opened with `font.info` (File > Font Info…, Cmd+I, or the Info button at the
top-left of the main window). Panels: `fontInfo`, `fontInfo.font`, `fontInfo.masters`,
`fontInfo.exports`, `fontInfo.features`, `fontInfo.other`, `fontInfo.notes`.

## Command model

The window is a view over three scopes of data: the **font**, each **master**, each **export**
(static instance or variable font setting). Every command takes the scope and an explicit target
(`id`, name or index) so it runs without a UI selection; the UI passes its selection.

| Job | Commands |
|---|---|
| Read / write any built-in or added field | `font.info.get`, `font.info.set` |
| Optional fields under a General heading | `font.info.listFields`, `font.info.addField`, `font.info.removeField` |
| Localizations of a field | `font.info.setLocalizedValue`, `font.info.removeLocalizedValue` |
| Row clipboard | `font.info.copyRows`, `font.info.pasteRows` |
| Custom parameters, all three scopes | `font.customParameters.list / describe / add / set / remove` |
| Axes | `font.axes.list / add / set / remove / listPredefined` |
| Masters | `font.master.list / get / set / add / addFromOtherFont / duplicate / remove / reorder / setIcon / setAxisLocation / setMetric / setStem / setNumber` |
| Metric, stem and number definitions | `font.metrics.*`, `font.stems.*`, `font.numbers.*` |
| Exports | `font.instance.list / get / set / add / addForEachMaster / toMaster / addVariableFontSetting / remove / reorder / setActive / setAxisLocation / setStyleLinking / setWeightClass / setWidthClass` |
| Features tab | `font.features.*` |
| Other Settings | `font.settings.get`, `font.settings.set` |
| Notes | `font.note.get`, `font.note.set` |

The dedicated setters (`setWeightClass`, `setStyleLinking`, …) and the generic `font.info.set` /
`font.master.set` / `font.instance.set` write the same data; the dedicated ones exist so each UI
control maps to one command with validation of its own.

## Shared row mechanics (Font, Masters, Exports)

- Fields are rows grouped under bold headings. A plus button beside a heading adds a field or entry
  to that group; a minus button at the right of a row deletes the row.
- Rows that show a plus button at the right (localizable fields) show the minus button only while
  Alt is held.
- Click selects a row, Cmd+click selects several. Cmd+Delete deletes the selection
  (`font.info.removeField`, or the remove command of the entry type).
- Edit > Copy (Cmd+C) and Edit > Paste (Cmd+V) work on selected rows (`font.info.copyRows`,
  `font.info.pasteRows`).
- Localizable fields have a language menu and a plus button; each extra row is the same field in
  another language.
- Fields under the General heading of the Font tab and of the Exports tab accept text tokens
  (`fontInfo.general.textTokens`); the token grammar is specified with the Text Tokens section.

## Font tab

Data that applies to every exported file. Panel `fontInfo.font`; optional fields are chosen in
`fontInfo.fieldPicker` (select, then Add).

### Family Name

- Names the exported fonts and groups them into one family submenu in applications.
- Safe characters: A–Z, a–z, 0–9, space. Anything else may break export or installation, so warn
  on entry (`fontInfo.familyName.allowedCharacters`).
- For non-ASCII or localized family names add the `familyNames` field (General plus button).
- Export uses it for the file name, name IDs 1, 3, 4 and 6, and in CFF fonts for FontName and
  FullName. `postscriptFontName` and `postscriptFullName` fields override those two independently
  (`fontInfo.familyName.derivedNames`).
- An export with its own `familyNames` field replaces the family name for that export
  (`fontInfo.familyName.instanceOverride`).

### Units per Em

- Coordinate units per em; 1000 in a new file.
- Limits to surface as warnings (`fontInfo.unitsPerEm.limits`): valid 16–16384; some applications
  fail above 3000; InDesign and Illustrator above 5000; CFF glyph widths become a problem past
  4096 UPM; no node or handle coordinate may exceed ±32768.
- Typing a number changes only the UPM value (`font.info.set {unitsPerEm}`).
- The double-arrow button rescales the font to a new UPM so glyphs keep their apparent size
  (`font.info.scaleToUnitsPerEm`).
- Resizing all glyphs is the two-step combination: type a smaller UPM (no scaling), then scale
  back to the old UPM to enlarge; type a larger UPM first to shrink
  (`fontInfo.unitsPerEm.resizeTrick`).
- Export-only alternatives: the `Scale to UPM` instance parameter (scales outlines and metrics) and
  the `unitsPerEm` parameter (changes the number only) (`fontInfo.unitsPerEm.exportOverrides`).
- When more precision is wanted, the handbook recommends grid subdivision over a very large UPM.

### Version

- Major and minor part separated by a period; 1.000 in a new file. Stepper buttons change the minor
  part (`font.info.stepVersion`).
- Export writes both to `head.fontRevision` and builds name ID 5 as `Version <major>.<minor>`, a
  semicolon, and the exporting application's name and build. FontCraft writes its own name there.
- A `versionString` parameter replaces the generated string; some applications need it to begin
  with `Version <major>.<minor>` (`fontInfo.version.headAndNameId5`).

### Creation Date

- One date-time value used for both `head.created` and `head.modified`
  (`fontInfo.date.headCreatedModified`).
- The update button sets it to now (`font.info.updateDate`; headless callers may pass a timestamp
  for reproducible builds).

### Axes

- Each axis: name, four-character tag, Hidden checkbox. Added with the heading's plus button
  (`font.axes.add`), edited with `font.axes.set`, deleted with the row's minus button
  (`font.axes.remove`).
- The name field offers predefined names; choosing one sets the tag
  (`fontInfo.axes.predefinedNameSetsTag`, `font.axes.listPredefined`).
- Hidden keeps the axis out of sight of font users (`fontInfo.axes.hidden`).
- The axis list defines the designspace that master and instance coordinates use; adding an axis
  therefore adds one coordinate to every master and export.

### Custom Parameters

- Added through `fontInfo.customParameterPicker`: search or scroll, click a name for its
  description, Add inserts the selected parameters (`font.customParameters.add`).
- The same UI and commands serve the Masters and Exports tabs with `scope` set accordingly.
- Precedence: export over master over font (`fontInfo.customParameters.precedence`).
- A parameter set on every master is interpolated for instances
  (`fontInfo.customParameters.masterInterpolation`).

## Masters tab

A master is a complete set of outlines and metrics; instances are computed from the masters.
Panel `fontInfo.masters`.

### Managing masters

- New fonts have one master, Regular (`fontInfo.masters.newFileHasRegular`).
- Plus menu at the bottom:
  - **Add Master** (`font.master.add`): empty layers in every glyph, default metric values.
  - **Add Other Font** (`font.master.addFromOtherFont`, dialog `fontInfo.addOtherFont`): lists all
    masters of all open fonts; OK imports the chosen ones, Cancel none.
  - **Duplicate Selected** (`font.master.duplicate`): copies name, layers and metrics; the copy
    keeps the same name.
- Minus button at the bottom-left deletes the selected masters (`font.master.remove`).
- Dragging reorders (`font.master.reorder`). The first master supplies glyph-level hinting
  information unless `Get Hints From Master` says otherwise (`fontInfo.masters.firstMasterHints`).
- Several masters can be selected and edited together; fields that differ show a gray
  "Multiple Values" placeholder (`fontInfo.masters.multipleValues`). Headless: pass several ids to
  `font.master.set`.

### General

- **Name**: internal only, never exported (`fontInfo.masters.nameIsInternal`).
- **Icon** (`font.master.setIcon`, `fontInfo.masterIconPicker`): a pick from the built-in set of 25
  lowercase-n drawings in different weights and widths, or any glyph of the font given by name in
  the field at the bottom of the picker.

### Axes Coordinates

One number per font axis places the master in the designspace (`font.master.setAxisLocation`).

### Metrics & Alignment Zones

- A metric is defined once for the font (category, name, optional scope) and valued per master.
  Definitions: `font.metrics.list / add / set / remove`; values: `font.master.setMetric`.
- Default set depends on script and language (`fontInfo.metrics.defaultsByScript`):
  - Latin: Ascender, Cap Height, x-Height, Baseline, Descender, Italic Angle.
  - Arabic: Ascender, Baseline, Descender, Alef Height, Joining Line, Meem Depth.
- Baseline is always 0, positions are offsets from it, Descender is negative, Italic Angle is in
  degrees clockwise (`fontInfo.metrics.baselineAndSigns`).
- Italic Angle is read by sidebearing calculation, by anchor alignment between selected nodes and
  by italic-aware transformations (`fontInfo.metrics.italicAngleUses`).
- Clicking a metric's name opens an editor for category, name and scope (`fontInfo.metricEditor`).
  The scope is a glyph filter with the smart-filter rule model; the metric line is hidden in glyphs
  outside the scope (`fontInfo.metrics.scopeHidesLine`).
- Row layout: left number = offset from the baseline; right number = size of the alignment zone,
  which starts at that offset. Italic Angle has one number only.
- Edit View draws the metrics as orange lines while View > Show Metrics is on
  (`fontInfo.metrics.shownInEditView`) and highlights nodes and anchors inside a zone.
- Zones feed the PostScript autohinter (overshoot suppression at small sizes); TrueType hinting may
  have its own zones (`fontInfo.zones.highlightAndHinting`).

### Stems

- Standard stem widths per master, read by the autohinter; TrueType hinting may have its own
  (`fontInfo.stems.autohinterInput`).
- Plus button beside the heading adds a named definition (`font.stems.add`), either vertical (a
  left-to-right measurement) or horizontal (a top-to-bottom measurement).
- The name both describes the stem and pairs it across masters for interpolation
  (`fontInfo.stems.interpolateByName`).
- The actions button on a stem row sets a glyph scope, same model as metric scopes
  (`font.stems.set`). Values per master: `font.master.setStem`.

### Custom Parameters

See Font tab; scope `master`.

### Number Values

Named numbers valued per master (`font.numbers.add / rename / remove`, `font.master.setNumber`).
Feature code inserts them through number value tokens and the value interpolates for instances
(`fontInfo.numbers.tokensInterpolate`).

## Exports tab

The list of fonts File > Export (Cmd+E) produces. Panel `fontInfo.exports`.

### Managing exports

- Plus menu at the bottom-left:
  - **Add Instance** (`font.instance.add`): one static instance called Regular.
  - **Add Instance for Each Master** (`font.instance.addForEachMaster`): one instance per master at
    that master's coordinates, named like the master.
  - **Instance as Master** (`font.instance.toMaster`): turns the selected instance's interpolated
    outlines and metrics into a new master. With masters Thin and Bold and a Regular instance in
    between, the result is a Regular master; Light then interpolates Thin–Regular and Semibold
    Regular–Bold.
  - **Add Variable Font Setting** (`font.instance.addVariableFontSetting`): an entry that exports a
    variable font.
- Minus deletes the selection (`font.instance.remove`); Cmd+click and Shift+click select several.
- Dragging reorders (`font.instance.reorder`); order never affects exported files
  (`fontInfo.exports.orderIsCosmetic`).

### Fields

- **Active** (`font.instance.setActive`): off means skipped at export and shown with a gray icon.
- **Style Name**: shown in font menus. ASCII letters, digits, spaces and light punctuation such as
  the hyphen; use `styleNames` for Unicode names. Keep family + style to about 20 characters or
  add abbreviated `postscriptFontName` / `postscriptFullName` (`fontInfo.styleName.limits`).
- **Weight Class** (`font.instance.setWeightClass`): 1–1000; the disclosure menu lists 100–900.
  In-between values are legal but rejected by some applications.
- **Width Class** (`font.instance.setWidthClass`): 1–9 from the disclosure menu.
- Both classes follow the OS/2 table definition, are used by other applications (and CSS for
  weight) to order and pick fonts, and are unrelated to the coordinates of weight and width axes
  (`fontInfo.weightClass.range`).
- **Axes Coordinates** (`font.instance.setAxisLocation`): inside the masters' range the instance is
  interpolated, outside it is extrapolated (`fontInfo.instance.extrapolation`).
- **Style Linking** (`font.instance.setStyleLinking`): Bold checkbox, Italic checkbox, name of the
  base instance. A Bold Italic sets both boxes and names Regular.
  - Works only within one family name; instances use the Font tab's family name unless they carry
    `familyNames`; instances from separate source files can share a family
    (`fontInfo.styleLinking.sameFamily`).
  - Recommended: Bold, Italic, Bold Italic link to Regular; any other italic links to its upright.
    Linking, say, Semibold as the bold of Light hides Semibold in four-style font pickers
    (`fontInfo.styleLinking.recommendedScheme`; a candidate lint).
- **Custom Parameters**: scope `instance`; they override master and font values.

## Features tab

Only the window is specified here; the feature language, tokens and automatic generators are
specified with the feature-code chapter. Panel `fontInfo.features`.

### Sidebar

- Three fixed groups, each heading with a plus button (`fontInfo.features.sidebarGroups`):
  - **Prefix**: free code outside any feature, typically shared lookups (`font.features.addPrefix`).
  - **Classes**: glyph names separated by spaces or line breaks (`font.features.addClass`).
  - **Features**: rules under a four-character tag (`font.features.addFeature`). The plus button
    opens `fontInfo.featurePicker`: search by tag or name, Up/Down to move, Return or double-click
    to add.
- Minus button at the bottom-left deletes the selected entries (`font.features.remove`).
- With a feature selected, a Spec button at the top-right opens that feature's page of the
  OpenType specification (`font.features.openSpec`).

### Automatic code

- Update (bottom-left, `font.features.update`) regenerates all automatic entries and adds entries
  that have become possible, driven by glyph names (`fontInfo.features.updateDetectsNew`):
  underscore names give `liga`; suffixes such as `.sc` or `.tf` fill the matching feature; a suffix
  equal to a feature tag (`g.ss01`) fills that feature.
- Automatic entries show an update button beside their name; clicking it regenerates only that
  entry (`font.features.updateEntry`).
- Above the editor, the Generate Feature Automatically checkbox switches an entry between automatic
  and manual (`font.features.setAutomatic`). Manual entries are skipped by Update.
- Entries that could be generated but are not added by Update, e.g. the class `All` and the feature
  `cpsp`, must be added by hand (`fontInfo.features.optionalAutomatic`).

### Manual code

- Entries with no generator (e.g. `calt`) are always manual.
- Compile (bottom-left, `font.features.compile`) builds the code for the Edit View preview. Only
  manual substitutions are previewed there, not manual positioning.
- Editor: syntax colouring by token class (`fontInfo.features.syntaxColouring`); autocomplete for
  glyph, class and lookup names with Up/Down and Return (`fontInfo.features.autocomplete`,
  `font.features.complete`); Alt+click on a glyph or class name shows a visual preview
  (`font.features.previewName`).
- Errors (`fontInfo.features.inlineErrors`, `font.features.listErrors`): inline marker; click for
  the long description; a wrench icon means a Fix button is available (`font.features.applyFix`);
  the sidebar entry gets an icon that jumps to the fault (`font.features.jumpToError`).
- Notes field under the editor, resized by its divider (`font.features.setNotes`).
- Snippets button at the bottom-right inserts predefined code (`font.features.insertSnippet`,
  `font.features.listSnippets`).

### Stylistic set names

For `ss01`–`ss20`, rows above the editor hold localized names: language picker plus text, plus and
minus buttons per row (`font.features.setLabel`, `font.features.removeLabel`). The Default
language is the fallback and is commonly treated as English. Names accept text tokens.

### Implicit features

- `kern`, `dist`, `mark`, `abvm`, `blwm`, `mkmk`, `curs` are generated at export from kerning and
  anchors and are not listed, because their code differs per master and instance
  (`fontInfo.features.implicit`).
- To extend one, add the feature by tag and put the line `# Automatic Code` in it: code above goes
  before the generated rules, code below after (`fontInfo.features.automaticCodePlaceholder`). The
  snippets button inserts that line.

### Export-specific features

Per-instance changes use the custom parameters Add / Remove / Replace × Prefix / Class / Feature.
Automatic entries adapt to each instance without them (`fontInfo.features.exportSpecific`).

## Other Settings tab

Panel `fontInfo.other`; read and written with `font.settings.get` / `font.settings.set`.

| Setting | Behaviour | Row |
|---|---|---|
| Grid Spacing | Rounding step for coordinates. 1 (default) = whole units; 5 = multiples of 5; 0 = no rounding. All tools and operations snap to it. | `fontInfo.grid.spacing` |
| Subdivision | Divides the step: spacing 100 / subdivision 5 = 20-unit grid; spacing 1 / subdivision 10 = one decimal. | `fontInfo.grid.subdivision` |
| Keyboard Increments | Arrow-key step with Shift (default 10) and with Cmd (default 100), for nudging points and stepping number fields. | `fontInfo.keyboardIncrements` |
| Use Custom Naming | Keeps glyph names that are not in the glyph database. Turning it off renames nothing until Glyph > Update Glyph Info runs on a selection. | `fontInfo.useCustomNaming` |
| Disable Automatic Alignment | Turns off component auto-alignment and metric syncing of composites font-wide. | `fontInfo.disableAutomaticAlignment` |
| Keep Alternates Next to Base Glyph | Sorts dot-suffixed variants right after their base glyph. | `fontInfo.keepAlternatesNextToBase` |
| File Format Version | Which Glyphs format version the file is saved in; older files stay in their version until switched. | `fontInfo.fileFormatVersion` |
| Font Type | Interpolation model for static instances: Default (Multiple Master style) or Variable (matches variable-font results). | `fontInfo.fontType` |

Further rules:

- Changing the grid moves nothing; Path > Round Coordinates rounds the selection on request
  (`fontInfo.grid.noRetroactiveRounding`).
- Fractional coordinates are valid in CFF output and must survive compilation
  (`fontInfo.grid.fractionalCff`).
- Glyph-name handling on import follows the user setting Keep Glyph Names from Imported Files;
  automatic alignment for imported files follows Disable Automatic Alignment for Imported Files.

## Notes tab

One Markdown-highlighted text field (`font.note.get`, `font.note.set`). The text is the font's
`note` custom parameter and is never exported. Highlighted constructs: `*italic*` / `_italic_`,
`**bold**` / `__bold__`, `[text](url)` and `<url>` links, `#` to `######` headings. Highlighting
only; the stored text is the raw Markdown.

## Where each field ends up

"Output" lists only what the handbook chapter states. "Source key" is the Glyphs file key from the
GlyphsSDK format documentation (status `docs`), relevant for `.glyphs` import and export. A blank
Output cell means the chapter does not say; those fields are repeated under Open questions.

### Font tab

| Field | Output (per handbook) | Source key (GlyphsSDK) |
|---|---|---|
| Family Name | Export file name; name IDs 1, 3, 4, 6; CFF FontName and FullName | `familyName` |
| familyNames (optional, localizable) | Replaces/localizes the family name (IDs not listed separately) | `properties[key=familyNames].values[{language, value}]` |
| postscriptFontName (optional) | CFF FontName, independent of the family name | `properties[key=postscriptFontName]` |
| postscriptFullName (optional) | CFF FullName, independent of the family name | `properties[key=postscriptFullName]` |
| Units per Em | | `unitsPerEm` |
| Version | `head.fontRevision`; name ID 5 (`Version M.mmm;<app and build>`) | `versionMajor`, `versionMinor` |
| Creation Date | `head.created` and `head.modified` | `date` |
| Axis name / tag / Hidden | | `axes[].name`, `.tag`, `.hidden` |
| Custom Parameters | Per parameter (appendix) | `customParameters[{name, value}]` |

### Masters tab

| Field | Output (per handbook) | Source key (GlyphsSDK) |
|---|---|---|
| Name | None (internal only) | `fontMaster[].name` |
| Icon | None | `fontMaster[].iconName` |
| Axes Coordinates | | `fontMaster[].axesValues[]` |
| Metric definition (category, name, scope) | | `metrics[{type, name, filter, horizontal}]` |
| Metric position / zone size | Zones are input to PostScript autohinting (tables not named) | `fontMaster[].metricValues[{pos, over}]` |
| Italic Angle | | a `metrics` entry of type `italic angle` with its `metricValues` item |
| Stem definition (name, orientation, scope) | | `stems[{name, horizontal, filter}]` |
| Stem value | Input to the autohinter (tables not named) | `fontMaster[].stemValues[]` |
| Number Value name / value | Substituted into feature code by tokens | `numbers[]`, `fontMaster[].numberValues[]` |
| Custom Parameters | Per parameter (appendix) | `fontMaster[].customParameters` |

### Exports tab

| Field | Output (per handbook) | Source key (GlyphsSDK) |
|---|---|---|
| Entry kind (instance / variable font setting) | One static font / one variable font per entry | `instances[].type` |
| Active | Whether a file is exported | `instances[].exports` |
| Style Name | Font menu style name (name IDs not listed) | `instances[].name` |
| Weight Class | OS/2 weight class (field name not given) | `instances[].weightClass` |
| Width Class | OS/2 width class (field name not given) | `instances[].widthClass` |
| Axes Coordinates | | `instances[].axesValues[]` |
| Style Linking: Bold, Italic | | `instances[].isBold`, `.isItalic` |
| Style Linking: base style name | | `instances[].linkStyle` |
| familyNames (optional) | Family name of this export | `instances[].properties` |
| Custom Parameters | Per parameter (appendix) | `instances[].customParameters` |
| (manual interpolation factors; not in this chapter) | | `instances[].manualInterpolation`, `.instanceInterpolations` |

### Features, Other Settings, Notes

| Field | Output (per handbook) | Source key (GlyphsSDK) |
|---|---|---|
| Prefix (name, code, automatic, notes) | Feature code compiled into GSUB / GPOS | `featurePrefixes[{name, code, automatic, notes, disabled}]` |
| Class (name, code, automatic, notes) | Feature code compiled into GSUB / GPOS | `classes[{name, code, automatic, notes, disabled}]` |
| Feature (tag, code, automatic, notes) | GSUB (substitution) or GPOS (positioning) | `features[{tag, code, automatic, notes, disabled}]` |
| Stylistic set names | | `features[].labels[{language, value}]` |
| Implicit kern, dist, mark, abvm, blwm, mkmk, curs | Positioning features (GPOS) generated at export | none (derived from kerning and anchors) |
| Grid Spacing | None (CFF output may hold fractional coordinates) | `settings.gridLength` |
| Subdivision | None | `settings.gridSubDivision` |
| Keyboard Increments | None | `settings.keyboardIncrementBig`, `settings.keyboardIncrementHuge` (base step: `keyboardIncrement`) |
| Use Custom Naming | None | `settings.disablesNiceNames` |
| Disable Automatic Alignment | None | `settings.disablesAutomaticAlignment` |
| Keep Alternates Next to Base Glyph | None | `settings.keepAlternatesTogether` |
| File Format Version | None (source format only) | `.formatVersion` |
| Font Type | Affects static-instance interpolation | `settings.fontType` (`default`, `variable`; the format also lists `layerFont`, `iconSet`) |
| Note | None (not exported) | `note` |

## Open questions

Mapping to output, not stated in this chapter:

- Units per Em: the table field (`head.unitsPerEm` by the OpenType spec) is not named.
- Axes: which tables receive name, tag and the Hidden flag (`fvar`, `STAT`, name table by the
  spec), and what a hidden axis changes besides the flag.
- Style Name: which name IDs it fills (2, 4, 6, 16, 17 by the spec) and how they are split when
  style linking is set.
- Style Linking: the exact effect on name IDs 1/2 versus 16/17, `OS/2.fsSelection`,
  `head.macStyle`, and what an empty link-name field means.
- Weight / Width Class: the OS/2 field names are implied, not written; is the width class limited
  to 1–9 when typed, and what are the labels in the two disclosure menus.
- familyNames: which name IDs and which platform/language records each localization writes.
- Stylistic set names: storage in the output (feature parameters plus name records by the spec).
- Master metrics: how Ascender / Descender / Cap Height / x-Height / Italic Angle reach `OS/2`,
  `hhea` and `post`, and how zones and stems reach the CFF Private dictionary.

Behaviour to observe in the reference:

- Token delimiters: the converted handbook text shows the braces ambiguously; confirm the exact
  token syntax from the Text Tokens section.
- The complete list of optional General fields per tab, which of them are localizable, and whether
  the Masters tab offers any.
- What the Scale to UPM button scales besides outlines (metrics, zones, stems, kerning, anchors,
  guides, components, custom parameter values) and how it rounds.
- Version stepper: step size, upper limit of the minor part, and whether the major part has a
  control of its own.
- Creation Date: display format and time-zone handling.
- Axes: default name/tag of a new axis, the predefined name list, whether axes can be reordered,
  what removing an axis does to stored coordinates, tag validation (length, duplicates).
- Custom parameters: whether duplicates are allowed in one scope, whether rows can be reordered,
  and the disabled state that the file format records (`disabled`) but this chapter does not show.
- Row copy and paste: clipboard format, conflict handling, pasting between tabs and fonts.
- Add Master: default name, icon and axis coordinates of the new master.
- Add Other Font: handling of glyphs missing in either font, of differing axes, UPM and metrics.
  The Interpolation chapter calls the entry "Add Other Master"; confirm the label.
- Removing a master: what happens to instances and to intermediate layers tied to it; refusal on
  the last master.
- Metrics: is there a plus button to add metrics, which categories exist (the format lists
  ascender, cap height, slant height, x-height, midHeight, bodyHeight, descender, baseline,
  italic angle, italic slope), can default metrics be deleted, may the zone size be negative
  (it must be for bottom zones), and where the horizontal flag appears.
- Stems and numbers: default names, whether renaming a number updates tokens, reordering.
- Exports: fields of a variable font setting, default weight/width class of a new instance,
  naming rules of Add Instance for Each Master and whether it skips existing locations,
  what Instance as Master does with several instances selected.
- Interpolation values (manual per-master factors): present in the file format, not described
  here; find the UI.
- Features: renaming and reordering sidebar entries, whether order affects compiled lookup order,
  the disabled state recorded by the file format, the snippet list, whether editing an automatic
  entry is blocked, shortcuts for Update and Compile, whether Compile runs automatically on export.
- Keyboard Increments: the chapter gives Shift = 10 and Cmd = 100, while the format documentation
  describes the largest step as Shift and Cmd together; confirm the modifiers and whether the base
  step of 1 is editable.
- Use Custom Naming: the source key is the inverse-sounding `disablesNiceNames`; confirm the
  polarity.
- File Format Version: the chapter says new files are created in version 3 format even in
  Glyphs 4; confirm, and list which features force version 4.
- Font Type: the format lists two more values (`layerFont`, `iconSet`) that this chapter does not
  mention.
- Notes: the chapter text places the tab "in Font View"; taken here to mean the Notes tab of Font
  Info. Confirm whether Markdown is rendered anywhere or only highlighted.
