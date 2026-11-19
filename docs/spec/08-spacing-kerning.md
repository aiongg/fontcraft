# Spacing & Kerning: behaviour notes

Source: Glyphs 4 Handbook, chapter "Spacing & Kerning" (https://handbook.glyphsapp.com/), read 2026-10-07.
Rows: `spec/inventory/08-spacing-kerning.toml`. Metrics import/export is in `07-import-export.md`.

Terms: LSB/RSB = left/right sidebearing. "Current glyph" in text mode = the glyph directly after
the text cursor (`spacing.currentGlyphFollowsCursor`).

## 1. Spacing

### 1.1 Display

- Text tool active + View › Show Metrics (Cmd+Shift+M): every glyph in the line shows LSB, RSB
  and width (`spacing.metricsOverlayInTextMode`).
- Info box (View › Show Info, Cmd+Shift+I), `infoBox.spacing`: for horizontal text in either
  direction, LSB and RSB sit left and right of a central metrics icon; for top-to-bottom text
  the fields are T and B, top and bottom sidebearing (`spacing.infoBoxFollowsLayoutDirection`),
  with O, the vertical origin, and H, the vertical width (height).

### 1.2 Editing in the Info box

- Fields accept a number or a metrics key (section 2). Engine commands:
  `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`, `spacing.setWidth`,
  `spacing.setTopSidebearing`, `spacing.setBottomSidebearing`, and for O and H
  `spacing.setVerticalOrigin` and `spacing.setVerticalWidth`.
- The vertical origin is stored as `vertOrigin`, measured down from the master's ascender (0
  when the layer has none); the vertical width as `vertWidth` (the ascender to the descender
  when the layer has none). Setting the origin moves the vertical box and keeps its height, so
  TSB and BSB change in opposite directions; setting the vertical width moves the bottom edge.
  Their keys (`metricVertOrigin` and `metricVertWidth` in format 3; format 2 has only
  `vertWidthMetricsKey`) take the width's forms: no `|`, no `@`.
- Up/Down in a field: ±1. With Shift: ±10. With Cmd+Shift: ±100 (`spacing.infoBoxArrowSteps`).

### 1.3 Spacing shortcuts (Text tool)

| Shortcut | Command | Effect on current glyph |
|---|---|---|
| Ctrl+Left | `spacing.increaseLeftSidebearing` | LSB +1 |
| Ctrl+Shift+Left | `spacing.increaseLeftSidebearingBy10` | LSB +10 |
| Ctrl+Right | `spacing.decreaseLeftSidebearing` | LSB −1 |
| Ctrl+Shift+Right | `spacing.decreaseLeftSidebearingBy10` | LSB −10 |
| Cmd+Right | `spacing.increaseRightSidebearing` | RSB +1 |
| Cmd+Shift+Right | `spacing.increaseRightSidebearingBy10` | RSB +10 |
| Cmd+Left | `spacing.decreaseRightSidebearing` | RSB −1 |
| Cmd+Shift+Left | `spacing.decreaseRightSidebearingBy10` | RSB −10 |
| Cmd+Ctrl+Left | `spacing.shiftOutlineLeft` | LSB −1, RSB +1, width unchanged |
| Cmd+Ctrl+Shift+Left | `spacing.shiftOutlineLeftBy10` | LSB −10, RSB +10 |
| Cmd+Ctrl+Right | `spacing.shiftOutlineRight` | LSB +1, RSB −1, width unchanged |
| Cmd+Ctrl+Shift+Right | `spacing.shiftOutlineRightBy10` | LSB +10, RSB −10 |

- Scheme: Ctrl = left side, Cmd = right side, both = shift within the width, Shift = ×10
  (`spacing.shortcutModifierScheme`). The arrow shows which way the addressed edge moves.
- There is no 100-unit shortcut step; that exists only in Info box fields.
- In top-to-bottom text (`vertical: true`) the left-side shortcuts act on TSB and the right-side
  ones on BSB; a shift moves the outline up (Left) or down (Right) inside the vertical width by
  changing the vertical origin, so the outline's coordinates and the horizontal metrics stay.
  Our guess; no source says (`spacing.shortcutModifierScheme`).
- Text tool required; disabled in locked-kerning mode (`spacing.shortcutsNeedTextTool`).
- macOS reserves Ctrl+Left/Right for Mission Control by default (`spacing.ctrlArrowSystemConflict`).

## 2. Metrics keys

A metrics key is text stored in place of a number for LSB, RSB or width. It links the metric to
another glyph's metric, optionally with one arithmetic step.

### 2.1 Grammar (as far as the handbook states it)

```
field        = number                      ; plain value, no key stored
             | glyph-name                  ; shorthand for "=" glyph-name
             | key

key          = scope body [ "@" number ]
scope        = "="                         ; glyph-level: applies on all masters
             | "=="                        ; layer-local: this master only

body         = number                      ; constant            =64
             | reference [ op number ]     ; link / calculation  =n   =h+8   =hyphen/2
             | "|"                         ; own opposite side   =|
             | sign number                 ; auto-aligned glyphs only   =+18   =-12

reference    = [ "|" ] glyph-name          ; "|" = opposite sidebearing   =|n
op           = "+" | "-" | "*" | "/"
sign         = "+" | "-"
number       = digits [ "." digits ]       ; decimals allowed: =o*1.2
```

Documented forms and their meaning (F = the field holding the key: LSB, RSB or width):

| Form | Example | Value |
|---|---|---|
| bare name | `l` | metric F of glyph `l` (`metricsKey.linkByGlyphName`) |
| `=name` | `=n` | same as the bare name (`metricsKey.bareNameEqualsReference`) |
| `=name op number` | `=h+8`, `=hyphen/2`, `=c*1.1`, `=ka-deva/1.4` | metric F of the glyph, then the operation (`metricsKey.calculationSyntax`, `metricsKey.operators`, `metricsKey.decimalOperands`) |
| `=number` | `=64` | the constant (`metricsKey.constant`) |
| `=|name` | `=|n` | the opposite sidebearing of the glyph: in an RSB field, n's LSB; in an LSB field, n's RSB (`metricsKey.mirrored`) |
| `=|` | `=|` | the opposite sidebearing of this same glyph; centres the outline (`metricsKey.selfMirrorCentres`) |
| `==…` | `==u`, `==endash+12` | same body, but stored on this layer and valid for this master only (`metricsKey.localPrefix`) |
| `…@number` | `=H@0`, `=H@150` | the glyph's sidebearing measured on the horizontal line that many units above the baseline, instead of at the bounding box (`metricsKey.baselineOffset`) |
| `=±number` | `=+18`, `=-12` | on an automatically aligned glyph only: the derived value plus or minus the amount, alignment kept (`metricsKey.autoAlignedOffset`) |

Evaluation rules stated:

- The result is rounded to the nearest whole unit (`metricsKey.resultRounded`).
- A key with one `=` or a bare name is glyph-level and applies to all masters; `==` makes a
  layer-local exception (`metricsKey.glyphLevelByDefault`, `metricsKey.localPrefix`).
- The handbook describes calculations and the `|` and `@` forms for sidebearing fields. For the
  width field it only establishes that a glyph name links width to width.

Parsing hazards the implementation has to settle (see Open questions):

- Glyph names may contain `-` and `.` (`koKai-thai`), so `-` is not always an operator.
  FontCraft takes the longest leading part that names a glyph in the font (our rule; the
  reference's is unknown). Without a leading `=` the whole text is one glyph name.
- `=-12` is an offset on an auto-aligned glyph (M6) and a negative constant elsewhere.

In the file, a glyph-level key is stored on the glyph and a local key on the layer, without the
`==` (GlyphsSDK file-format specification; Inter Roman has 448 layers with their own keys). `==`
is only how the Info box shows and takes a local key.

### 2.2 Syncing

- Keys are not live. When the referenced glyph changes, the dependent metric keeps its old
  number and the key is out of sync (`metricsKey.notSyncedAutomatically`).
- Edit View: an out-of-sync key is shown in red with an update button; the button fixes that
  one metric (`metricsKey.outOfSyncInEditView`, `spacing.updateMetricsKey`).
- Font View: a warning triangle in the cell's top-right corner; a smart filter can list such
  glyphs (`metricsKey.outOfSyncInFontView`).
- `glyph.updateMetrics` (Glyph › Update Metrics, Cmd+Ctrl+M): all keys of the current layer, for
  the current glyph or all selected glyphs.
- `glyph.updateMetricsForAllMasters` (with Option, Cmd+Ctrl+Alt+M): the same on every master.
- Agent queries (ours): `spacing.getMetrics`, `spacing.evaluateMetricsKey`,
  `spacing.listOutOfSyncMetrics`.

### 2.3 Automatic alignment

A glyph under automatic alignment shows `auto` and the derived value in parentheses in its
metric fields (`metricsKey.autoAlignedDisplay`). `=+N` / `=-N` offsets the derived value without
ending alignment; meant for composites whose marks reach outside the glyph box.

## 3. Kerning

### 3.1 Data model as stated

- A kerning pair is (left side, right side, value). Each side is a glyph or a kerning group.
- Left-to-right and right-to-left kerning are stored separately. The writing direction selected
  in Edit View decides which set is shown and edited (`kerning.directionsStoredSeparately`).
- Kerning is per master (pairs are copied between masters, and deletion has an all-masters
  variant).
- Top-to-bottom glyphs have top and bottom groups instead of left and right
  (`kerning.verticalGroups`). Nothing else about vertical kerning is in this chapter.

### 3.2 Display and modes

- Text tool: kerned gaps are tinted, light blue for negative and yellow for positive values;
  colours come from the Appearance settings (`kerning.highlightColors`).
- Three modes, cycled by the button at the bottom right of the canvas
  (`editView.kerningModeButton`, `kerning.setMode`):

| Mode | Kerning shown | Kerning shortcuts | Spacing shortcuts |
|---|---|---|---|
| no kerning | no | off | on |
| kerning | yes | on | on |
| locked kerning | yes | on | off |

### 3.3 Info box

- `infoBox.kerning`: one kern field per neighbour of the current glyph; grey placeholder `Kern`
  when no pair is set. A typed value is committed by Return or by leaving the field; clearing
  the field removes the pair (`kerning.infoBoxFieldEditing`, `kerning.setPair`,
  `kerning.removePair`). Up and Down step the value as in every Info box field: ±1, ±10 with
  Shift, ±100 with Cmd+Shift (`spacing.infoBoxArrowSteps`).
- Group fields hold the glyph's kerning group names; a lock per side controls exceptions.
- With the Text tool, a second small box shows the preceding glyph's name, lock and group
  (`infoBox.precedingGlyph`).

### 3.4 Kerning shortcuts (Text tool, cursor between the two glyphs)

| Shortcut | Command | Handbook wording |
|---|---|---|
| Ctrl+Alt+Left | `kerning.increaseLeft` | increase kerning on the left side |
| Ctrl+Alt+Shift+Left | `kerning.increaseLeftBy10` | the same, 10 units |
| Ctrl+Alt+Right | `kerning.decreaseLeft` | decrease kerning on the left side |
| Ctrl+Alt+Shift+Right | `kerning.decreaseLeftBy10` | the same, 10 units |
| Cmd+Alt+Right | `kerning.increaseRight` | increase kerning on the right side |
| Cmd+Alt+Shift+Right | `kerning.increaseRightBy10` | the same, 10 units |
| Cmd+Alt+Left | `kerning.decreaseRight` | decrease kerning on the right side |
| Cmd+Alt+Shift+Left | `kerning.decreaseRightBy10` | the same, 10 units |

- "Left side" = the pair (previous glyph, current glyph); "right side" = (current glyph, next
  glyph). Ctrl picks the left pair, Cmd the right pair, Alt distinguishes kerning from spacing,
  Shift = ×10 (`kerning.shortcutModifierScheme`).
- Step sizes are 1 and 10 only.
- The sign of "increase" is not defined in the text, and the highlight colours do not settle
  it. The key layout mirrors the spacing shortcuts, where the same arrows add space, so we take
  increase = value +1 (an inference). To be verified.
- Disabled in no-kerning mode (`kerning.shortcutsNeedTextTool`).

### 3.5 Kerning groups

- A glyph has up to two groups: left and right (top and bottom for vertical). The two sides are
  separate sets of names (`kerning.twoGroupsPerGlyph`).
- A pair that names a group covers every glyph in that group on that side
  (`kerning.groupSemantics`).
- Names: `A–Z a–z 0–9 _ . -` only (`kerning.groupNameCharset`). Convention: a representative
  glyph's name.
- Assignment: Group fields in the Info box; for many glyphs at once, the Font View inspector or
  a multi-glyph selection in Edit View (`kerning.groupsEditedForManyGlyphs`). Commands:
  `kerning.setLeftGroup`, `kerning.setRightGroup`, `kerning.setTopGroup`,
  `kerning.setBottomGroup`; query `kerning.listGroups`.
- Renaming is done in the Kerning window (`kerning.renameGroup`, section 3.8).
- View › Show Group Members (`view.showGroupMembers`): draws fellow members in the background
  for the current glyph and the glyph before it only, each with the group that faces the pair
  (current glyph: left group) (`kerning.groupMembersDisplayScope`).

### 3.6 Exceptions and locks

- An exception is a pair stored against a single glyph that would otherwise be covered by its
  group; it replaces the group value for that glyph (`kerning.exceptionOverridesGroup`).
- Each side of the pair at the cursor has a lock (`kerning.lockStates`):
  - closed: edits go to the glyph's group on that side;
  - open: edits go to the glyph itself.
- Resulting stored pair for an edit between glyphs L and R (gR(L) = L's right group,
  gL(R) = R's left group):

| L lock | R lock | Pair written |
|---|---|---|
| closed | closed | @gR(L) – @gL(R) |
| closed | open | @gR(L) – R |
| open | closed | L – @gL(R) |
| open | open | L – R |

  Only the closed/open row is spelled out in the handbook (T closed, ö open gives an exception
  for ö that still holds for every member of T's right group); the other rows follow from the
  same rule. A glyph without a group on that side can only be written as itself.
- With the cursor between the pair, opening a glyph's lock makes that side an exception
  (`kerning.addException`); closing it removes the exception (`kerning.closingLockRemovesException`,
  `kerning.removeException`). An older answer on the reference forum, not the handbook, says that
  closing a lock applies the existing group value, so the number shown can change.
- Closing one lock of a glyph–glyph exception while the other side stays open (our choice, not
  documented): the exception becomes the closed side's group against the open side's glyph, at
  the value that applied, unless that pair is already stored. Closing the left lock alone is
  refused when L – @gL(R) is stored, since that pair comes before @gR(L) – R (§ 3.7).

### 3.7 Pair lookup order

What the handbook states: group members share the group's kerning unless an exception is
defined, and an exception overrides the group value for its specific pair. It does not list a
complete precedence.

Order we implement (from the UFO 3 `kerning.plist` rules, status `docs`,
`kerning.pairLookupOrder`), for left glyph L and right glyph R:

1. L – R (glyph–glyph)
2. L – @gL(R) (glyph–group)
3. @gR(L) – R (group–glyph)
4. @gR(L) – @gL(R) (group–group)
5. no pair: 0

The first stored pair wins. Steps 2 and 3 can both exist for the same two glyphs; UFO gives
glyph–group priority, and so do both oracles' compiled fonts (fontc 1.0.0 and fontmake 3.12.1
on a test font with both mixed pairs). Whether the reference's Edit View resolves that tie the
same way is still to be observed.
`kerning.getPair` reports the value and the step that supplied it.

### 3.8 Kerning window (`window.kerning`, Window › Kerning, Cmd+Alt+K)

Lists all pairs of the current master for the current writing direction
(`kerning.windowListsCurrentDirection`). Data source: `kerning.listPairs`.

Viewing:

- Columns Left, Value, Right. Header click sorts; second click reverses (`kerning.sortPairs`).
- Groups appear blue with a leading `@`; glyphs gold (`kerning.windowEntryColors`).
- Search field filters to pairs containing the term. Options behind the search icon
  (`kerningWindow.searchOptions`, `kerning.filterPairs`):
  - match: contains | is exactly | begins with | ends with | does not contain
  - entry kind: is a group | is not a group
  - side searched: left | right
  - only glyphs selected in Font View
  - Reset (`kerning.resetFilter`)
- Selecting a row shows the pair in Edit View and replaces the pair that was there; Up/Down walk
  the list (`kerning.showPairInEditView`, `kerning.selectNextPair`,
  `kerning.selectPreviousPair`). A row with groups shows one example pair; actions menu ›
  Show All Glyphs inserts every member combination (`kerning.showAllGlyphs`).

Editing:

- Double-click a cell to edit. Value cell: sets the kerning. Name cell with a group: rename,
  followed by a choice between this pair only and the whole font
  (`dialog.renameKerningGroup`, `kerning.renameGroup`). Left and Right groups are renamed
  independently (`kerning.renameSidesIndependent`).
- Copy (Cmd+C) the selected rows, Select All (Cmd+A) for every row, switch master, Paste (Cmd+V)
  (`kerning.copyPairs`, `kerning.pastePairs`). If pasted pairs already exist: Overwrite replaces
  them, Keep Existing pastes only the new ones (`dialog.pasteKerningConflict`).
- Minus button (bottom left): delete the selected pairs; with Option, from all masters
  (`kerning.deletePairs`).

Maintenance (actions menu, `kerningWindow.actionsMenu`):

- Clean Up (`kerning.cleanUp`): delete obsolete pairs, i.e. pairs naming a glyph that no longer
  exists or a group without members (`kerning.obsoletePairs`).
- Compress (`kerning.compress`, `kerning.compressRules`, `kerning.compressOneStepPerRun`):
  - turns glyph–glyph and glyph–group pairs into pairs that use the glyphs' groups wherever
    that is possible;
  - keeps an exception when its value differs from the group kerning, drops it when the value
    is the same;
  - may need several runs. Example: T–m with T in right group T and m in left group n becomes
    @T–m, then @T–@n; an existing T–ň with a different value stays as an exception.

### 3.9 Kerning in feature code

- Pair kerning is compiled as an implicit `kern` feature that is not shown in
  Font Info › Features (`kerning.implicitKernFeature`).
- A feature named `kern` can be added by hand for contextual positioning rules
  (`kerning.explicitKernFeature`).
- Group classes usable in code (`kerning.groupClassNames`):

| Kerning group | Class name | Reason for the letter |
|---|---|---|
| left group `X` of a glyph | `@MMK_R_X` | the glyph stands on the right of the pair |
| right group `X` of a glyph | `@MMK_L_X` | the glyph stands on the left of the pair |

- Tokens may be used in kerning code to compute values from master number values or glyph
  sidebearings (`kerning.kernCodeTokens`).

## Not covered by the source chapter

No statement on: automatic spacing, limits on kerning values or pair counts, how the implicit
`kern` feature is split into lookups, vertical kerning beyond the existence of top/bottom groups
and T/B sidebearings, and kerning import/export other than copy and paste (file-based transfer
is in `07-import-export.md`, section 4.2).

## Open questions

1. Sign of the kerning shortcuts: does "increase" add to the value (more space) or make the
   pair tighter?
   **Answered for M5 (our decision):** increase adds to the value (more space), decrease subtracts.
   The handbook names the keys and the directions without a sign; the reference kerning tutorial
   (glyphsapp.com/learn/kerning) says negative values tighten. Reading "increase" as a more
   positive value is an inference. The Info box value after one keypress settles it. Guess on
   `kerning.increaseLeft`.
2. Do the spacing and kerning shortcuts follow the visual or the logical side in right-to-left
   and vertical text?
   **Answered for M5 (our decision):** in right-to-left text the shortcuts follow the visual side,
   as in left-to-right text; in top-to-bottom text they do nothing in M5. Guesses on
   `spacing.shortcutModifierScheme` and `kerning.shortcutModifierScheme`.
3. Spacing shortcuts on a metric that holds a metrics key, or on an auto-aligned glyph: is the
   key removed, kept and flagged, or is the keypress ignored?
   **Answered for M5 (our decision):** the number changes, the key stays and shows as out of sync
   (`spacing.shortcutsNeedTextTool`). Auto-aligned glyphs wait for M6.
4. Up/Down in an Info box field that holds a metrics key or a kern field: what happens?
   **Answered for M5 (handbook and our decision):** a kern field is an Info box value, so Up and
   Down add and subtract 1, 10 with Shift and 100 with Cmd+Shift, as the handbook gives for Info
   box values; a user report on the reference forum describes the 1 and 10 steps in a kern field.
   Ours: in a field with a key, Up and Down leave the key unchanged (`spacing.infoBoxArrowSteps`).
5. Numeric width edit: which sidebearing absorbs the change?
   **Answered for M5 (our decision):** the right sidebearing absorbs it; the left one stays
   (`spacing.setWidth`). `glyph.transformMetrics` without *on both sides* does the same.
6. Metrics key grammar: is whitespace allowed? Can `|` and `@` be combined with an operator, and
   in what order (`=|n+10`, `=h+8@200`)? More than one operation? Negative constants (`=-12` on
   a non-aligned glyph)? How are hyphenated glyph names told apart from subtraction?
   **Partly answered by sources, the rest our decision (M5):** text without a leading `=` is one
   glyph name, so `n-20` names a glyph and `=n-20` subtracts (forum answer by the reference
   developers, thread "Build 1015 metrics keys", 2017). `|` combines with an operator: files the
   reference saved hold `=|quoteleft*0.5`. `=-30` on a glyph that is not aligned automatically is a
   constant (forum bug reports treat it as a working key). Ours: spaces are ignored, one operation
   at most, `@` comes last (`=H+8@200`), and the glyph name is the longest leading part that names
   a glyph in the font. Guesses on `metricsKey.calculationSyntax`, `metricsKey.decimalOperands`,
   `metricsKey.constant`. Of the corpus's 4,413 keys, 2,929 are bare names, 616 `=` and a signed
   number, 405 `=|name`, 203 constants, 153 calculations, 104 `=name` and 3 `=|name` with an
   operation; none uses `@` or `==` (local keys are stored on the layer without `==`).
7. Which key forms are valid in the width field (calculation, `|`, `@`)? What do `|` and `@`
   mean for the T/B fields?
   **Answered for M5 (our decision):** the width field takes a glyph name, a constant or a
   calculation and refuses `|` and `@` (`spacing.setWidth`). The T and B fields take the same forms
   as LSB and RSB with top and bottom in place of left and right; `@` is refused there.
8. Evaluation details: division by zero, a referenced glyph that does not exist, a reference
   cycle, a referenced glyph that is itself out of sync, rounding of exact halves, and which
   layer of the referenced glyph is read for intermediate or alternate layers.
   **Answered for M5 (our decision):** division by zero, a cycle, a chain deeper than 32 steps and
   a result out of range are errors: the metric keeps its number and the key is reported. A forum
   answer by the reference developers shows that the reference does not stop cycles (thread
   "Possible bug with spacing formula"). A missing glyph makes the key unlinked
   (`metricsKeys.unlinkedDisplay`). Update Metrics evaluates in dependency order, so a chain
   settles in one run. Halves round up, as the oracles round kerning. A layer that is not a master
   layer reads the referenced glyph's layer with the same id, else the layer of its own master.
   Guesses on `metricsKey.notSyncedAutomatically`, `metricsKey.resultRounded`,
   `metricsKey.glyphLevelByDefault`, `glyph.updateMetrics`.
9. `@offset`: how is the sidebearing measured when the outline does not cross that height?
   **Answered for M5 (our decision):** the key uses the bounding-box sidebearing
   (`metricsKey.baselineOffset`).
10. A `==` local key next to a glyph-level key on the same field: does the local key simply win
    on its layer? How is a local key removed?
   **Answered for M5 (our decision):** on its layer the local key wins. A plain number typed into a
   field removes the key the field shows: a local key from the layer, a glyph-level key from the
   glyph (`metricsKey.localPrefix`). In the file a local key is the layer's own key, written
   without `==`.
11. Kerning lookup: is the precedence in section 3.7 what the reference does, in particular
    glyph–group before group–glyph?
   **Answered for compiled fonts by the oracles:** in a test font with A–@V = −30 and @A–V = −40,
   fontc 1.0.0 and fontmake 3.12.1 both give A V −30, so glyph–group wins; fontc resolves pairs in
   this order (`fontbe/src/features/kern.rs`). What the reference's Edit View shows stays to be
   observed; FontCraft uses the same order there.
12. Locks: does opening a lock immediately create a pair with the group's value? Does closing
    delete the exception pair for that master only? Is lock state derived from stored pairs or
    stored separately?
   **Answered for M5 (sources and our decision):** the lock state follows from the stored pairs:
   the file format has no lock field, and the reference scripting API reads and sets whether the
   kerning to the neighbour is an exception. The handbook says opening a lock makes that side an
   exception and closing it removes the exception; an older forum answer adds that closing applies
   the group value, so the number can change. Ours: opening a lock stores the pair that the open
   lock names at the value that applied; closing deletes that exception in the current master only
   (`kerning.lockStates`, `kerning.closingLockRemovesException`).
13. Are kerning groups per glyph across all masters (assumed) and shared between LTR and RTL?
   **Answered (sources):** per glyph across all masters: the file format stores `kernLeft` and
   `kernRight` on the glyph. Shared with right-to-left kerning: both oracles read right-to-left
   pairs with the group of the side that faces the pair (the first glyph's left group, written
   `@MMK_R_…`; the second glyph's right group, `@MMK_L_…`), and a test font compiles that way in
   both (`kerning.directionsStoredSeparately`).
14. Kerning window: default search options, the default sort order, whether further actions
    exist in the menu, the clipboard text format for copied pairs, and whether Clean Up and
    Compress act on one master or all.
   **Answered for M5 (source and our decisions):** the reference kerning tutorial lists the same
   three actions (Clean Up, Compress, Show All Glyphs). Ours: the search defaults are contains,
   every entry kind, both sides, not limited to the selection; the list keeps the stored order
   until a header is clicked; copied pairs are kerning-text lines; Clean Up and Compress act on the
   current master and direction.
15. Compress: order of side promotion, and what happens when promoted pairs collide with
    different values.
   **Answered for M5 (source and our decision):** the handbook example lifts the left side first.
   A run takes the pairs in stored order, and each pair sees the lifts made before it. A pair that
   meets a stored group pair follows the handbook rule: absorbed when the values are equal, kept
   as an exception when they differ. So when two pairs would lift to the same group pair, the
   first is lifted and the second meets it. With T–m and T–ň this gives @T–@n and the exception
   @T–ň, which keeps every value the two pairs had.
16. Rename with scope "pair": does it create a new group, and what membership does it get?
   **Answered for M5 (our decision):** no new group: only the selected pair's side is renamed and
   the glyphs keep their group (`kerning.renameGroup`).
17. Zero-valued pairs: stored (as a deliberate exception) or removed? Are fractional kerning
    values allowed?
   **Answered for M5 (oracles and our decision):** zero pairs are stored and override group values;
   both oracles keep them in the compiled font. Values read from files keep their fractions; typed
   values are rounded to whole units. Both oracles round compiled kerning half up (−10.5 → −10,
   10.5 → 11, −10.6 → −11).
18. Vertical kerning: storage, shortcuts, Kerning window listing, and the feature generated.
   **Partly answered (oracles), the rest our decision (M5):** neither fontc nor fontmake exports
   vertical kerning (no `vkrn` from a `kerningVertical` pair). M5 stores and edits vertical pairs
   and shows them in top-to-bottom text; there are no vertical kerning shortcuts in M5.
19. Implicit `kern` feature: lookup structure, script/language handling, interaction with a
    hand-written `kern` feature, and any value or size limits.
   **Answered for M5 (oracle):** M5.17 builds the lookups as fontc's kern writer does: both
   oracles split the generated kern lookups by script (a test font with Latin and Hebrew pairs gets
   a lookup per script). Guess on `kerning.implicitKernFeature`. **Open for M8:** the interaction
   with a hand-written `kern` feature; the handbook's Layout chapter says a hand-written feature of
   the same name places the generated code where it holds the line `# Automatic Code`.
