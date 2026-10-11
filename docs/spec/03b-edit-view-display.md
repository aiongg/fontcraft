# Edit View: info box, display, background, text, measuring, annotations, images, preview

Behaviour notes for `spec/inventory/03b-edit-view-display.toml`.
Source: Glyphs 4 Handbook, chapter "Edit View", sections Info Box to Previewing & Testing
(https://handbook.glyphsapp.com/edit-view/), read 2026-10-07. Drawing, path editing, components,
anchors and guides are covered in the other Edit View notes.

## Info box

**What it is.** A floating box under the current glyph, shown while `view.showInfo`
(Cmd+Shift+I) is on. The same toggle also shows or hides the context-dependent controls at the foot
of the Palette. Current glyph means the glyph open for editing; when text mode is on, it is the
glyph that comes right after the cursor.

**Glyph part, horizontal layout** (`infoBox.glyph`):

| Position | Field | Accepts |
|---|---|---|
| top centre | glyph name | text |
| top right | Unicode value, plus an arrow button | read-only here; button runs `glyph.openInUnicodeBrowser` |
| left of the metrics icon | left sidebearing | number or metrics key |
| right of the metrics icon | right sidebearing | number or metrics key |
| below the icon | width | number or metrics key |
| left edge | left kerning value and left kerning group | number / group name |
| right edge | right kerning value and right kerning group | number / group name |
| swatch | glyph colour (left half), layer colour (right half) | set from the context menu |

**Glyph part, vertical layout** (`infoBox.glyphVertical`), used when the text runs top to
bottom: T (top sidebearing), B (bottom sidebearing), O (vertical origin), H (vertical advance),
K (kerning, top and bottom), G (kerning group, top and bottom).

**Selection part** (`infoBox.selection`):

- X and Y give the position of the selection's bounding box, measured from the layer origin (0, 0)
  to one of nine reference points. The active point is blue; change it with
  `infoBox.setReferencePoint`.
- One node selected: its coordinates.
- One handle selected: its coordinates plus delta X, delta Y and the straight-line length to its
  on-curve node.
- Two nodes selected: the width (or height) field is their distance, the standard way to read a
  stem.
- Read with `selection.getInfo`; write with `selection.setPosition`. Size, scale and rotation
  fields belong to the Scaling & Rotating notes.

**Component part** (`infoBox.component`), a second box to the right while a component is
selected: base glyph name, X/Y offset, horizontal and vertical scale in percent, slant to the
right, rotation counterclockwise. Commands: `component.setTransform`,
`component.setBaseGlyph` (name click opens `infoBox.componentGlyphPicker`),
`component.editBaseGlyph` (arrow button: insert the base glyph left of the current glyph in the
tab text and start editing it). Offset edits do nothing while the component is auto-aligned.

**Other contents.** Image selected: see Images. Guide selected: name and a measurement toggle
(`guide.toggleMeasurement`). Annotation tool active: the annotation palette.

**Field editing rules** (all Info box fields):

- click a number to edit; Return or leaving the field commits;
- Tab commits and moves to the next field, Shift+Tab to the previous;
- Escape leaves the field without moving to another;
- Up/Down change the value by 1, with Shift by 10.

**How FontCraft builds it (M3.15, our choices where the handbook is silent).**

- The selection part sits at the foot of the canvas, centred, and shows only while something
  is selected; the glyph part is M5.
- The reference point starts at bottom left. The lock between width and height
  (`infoBox.setSizeLock`) starts open.
- Width and height edits run `transform.scale` about the reference point. A handle's dX, dY and
  length place the handle relative to its node through `selection.setPosition`.
- Every number field of the Palette, the Info box and the bottom bar follows the rules above.
  Clicking a field selects its text. Up and Down commit at once. A field left with its text
  unchanged commits nothing, so tabbing through fields adds no undo step.

**Unicode browser.** The arrow next to the Unicode value opens the code point in an external
app (UnicodeChecker in the reference). If the app is missing, the reference shows a download
link; the first launch needs an OS permission. FontCraft must choose a portable equivalent.

## Zooming and panning

| Input | Effect | Id |
|---|---|---|
| pinch on trackpad | zoom | behaviour `zoom.gestures` |
| Alt + scroll | zoom | behaviour `zoom.gestures` |
| Z, then click / Alt+click / drag a box | zoom in / out / to the box | tool `zoom`, `view.zoomToRect` |
| hold Cmd+Space / Cmd+Alt+Space | temporary zoom in / out tool | tool `zoom` |
| Cmd+Plus, Cmd+Minus | step in / out | `view.zoomIn`, `view.zoomOut` |
| Cmd+0 | fit descender-to-ascender span in the window | `view.zoomToActiveLayer` |
| Cmd+Alt+0 | one font unit = one display point | `view.zoomToActualSize` |
| buttons and field, bottom right | step, or type a value | `view.setZoom` |
| two-finger drag, wheel, Shift+wheel, scroll bars | pan | behaviour `pan.scrolling`, `view.scrollTo` |
| H then drag, or hold Space | pan | tool `hand` |

The zoom value is the size in display points at which 1000 units are drawn (300 means a
500-unit path is 150 points long; actual size is 1000). A display point is density independent.

FontCraft's view commands take positions in the edited glyph's font units. `view.scrollTo`
`{x, y}` names the point shown at the window's centre and `{dx, dy}` moves the content by screen
points; `view.zoomToRect` fits a rectangle with 8% of the window free on each side, as
`view.zoomToActiveLayer` does. The wheel and gestures run the same commands.

In text mode Space types a space. To pan there, press Cmd+Space and release Cmd. If the system
grabs Cmd+Space, pressing Space first and then Cmd still gives the zoom tool.

## View options

All are View-menu toggles and `ui` commands taking `{on?: bool}`; omitted `on` flips the state.
`view.getDisplayOptions` reads them all.

| Command | Shortcut | Shows |
|---|---|---|
| `view.showNodes` (submenu: `view.showNodesInForeground`, `view.showNodesInBackground`, `view.showExtraNodes`) | | on- and off-curve points, per side, and Extra Nodes |
| `view.showMetrics` | | vertical and horizontal metric lines and alignment zones |
| `view.showMetricsNames` | | labels on the metric lines |
| `view.showHints` | | PostScript hints only |
| `view.showAnchors` | | anchors of the current glyph |
| `view.showInfo` | Cmd+Shift+I | Info box and the Palette's contextual controls |
| `view.showBackground` | Cmd+Shift+B | the other side of the layer as an outline |
| `view.showImage` | | placed images |
| `view.showGuides` | | local and global guides |
| `view.showMeasurementLine` | | measurement line and spacing numbers, in text mode |
| `view.showAnnotations` | | annotations |
| `view.showBoundingBox` | Cmd+Alt+Shift+B | selection box with eight knobs |
| `view.fillPreview` | | closed paths filled with the foreground colour |

Edge cases:

- Fill Preview fills closed contours only. With it off, the tab's other glyphs are drawn as thin
  outlines (our decision).
- Fill Preview fills the glyph being edited only while one of the Text, Hand and Zoom tools
  is active. The fill colour is an Appearance setting.
- Images of glyphs without paths and components are shown even with Show Image off.
- Reporter plug-ins add their own entries to the View menu.

### Glyph and layer colours

Right-click (or Ctrl-click) the canvas of the active glyph and pick a colour: sets the glyph's
colour label (`glyph.setColorLabel`). With Alt held the entry becomes Set Layer Color and sets the
label of the active layer only (`glyph.setLayerColorLabel`). Both appear in the Info box swatch, glyph
colour left and layer colour right, as in Font View cells.

## Background

Every layer has one background that can hold paths and components. Uses: parking shapes,
before/after comparison, and non-destructive filters that keep the original there.

| Command | Shortcut | Effect |
|---|---|---|
| `view.showBackground` | Cmd+Shift+B | draw the inactive side as a faint red outline (colour is a setting) |
| `view.showNodesInBackground` | | also draw that side's points |
| `path.editBackground` | Cmd+B | toggle which side is edited; canvas darkens while in the background |
| `path.selectionToBackground` | Cmd+J | replace the background with the selection |
| `path.addSelectionToBackground` | Cmd+Alt+J | add the selection to the background |
| `path.swapWithBackground` | Cmd+Ctrl+J | exchange both sides |
| `path.clearBackground` | (menu, Alt held) | empty the backgrounds of the selected glyphs |
| `path.assignBackground` | | fill the backgrounds of the selected glyphs from another font file |

Rules:

- While the background is the active side, Selection to Background works in reverse (it writes
  to the foreground), and Show Background shows the foreground.
- While the background is displayed, its objects are snapping targets.
- Assigning the font to itself, or selecting all glyphs and running Selection to Background,
  snapshots the current outlines for later comparison.

FontCraft's rules where the handbook says nothing (our decisions):

| Command | Rule |
|---|---|
| Selection to Background | Copies shapes and anchors. A point or segment selects its whole path. The other side's shapes and anchors are replaced; its guides, hints, annotations and image stay. Without a selection the whole side is copied |
| Add Selection to Background | Appends the copied shapes; a copied anchor replaces the anchor of the same name |
| Swap with Background | Exchanges shapes, anchors, guides, hints and annotations. The placed image stays. A background the swap leaves empty is removed, so swapping twice restores a layer that had none |
| Clear Background | Removes the whole background, image included |
| Assign Background | Writes the master layers' backgrounds. Masters are matched by name, then by position; a source with fewer masters gives its first. Without `file` the source is the open font itself |
| All of them | With one glyph, a locked glyph or a missing layer is an error. With a `glyphs` list (Font View) such glyphs are skipped and listed in `skipped` |

How FontCraft draws the background and other layers (our decisions):

- Show Background draws the background's paths and components as a thin red outline behind the
  edited layer and the filled glyphs. Show Nodes › In Background adds its nodes and handles, drawn
  like the foreground's.
- A layer whose eye is on (`layers.toggleVisibility`) is drawn as a thin blue outline while
  another layer of the glyph is edited, whatever Show Background says.
- The background's nodes and handles are snap targets after the edited layer's own nodes and
  handles and before guides. They count whenever the layer has a background; the Show
  Background switch does not turn them off yet.
- Edit Background belongs to the tab: the side stays when another glyph, layer or master is
  edited. Switching sides clears the selection.
- While the background is edited, the canvas is a little darker, the background is drawn as the
  outline with its nodes, and the foreground is drawn as Show Background's thin outline. Show
  Nodes › In Background then shows the foreground's nodes, and the foreground's nodes are the
  snap targets.
- While the background is edited, the tools, the Palette and the Info box work on it, and every
  command the shell runs that takes `side` gets `side: "background"`. Commands that act on
  whole layers (Swap with Background, Clear Background) take no side and work as in the
  foreground. Audit markers describe the foreground and are not drawn.

## Text mode

**Entering.** Tool `text` (T) puts a text cursor into the tab's string. Escape toggles between
text mode and editing the current glyph (`editView.toggleTextMode`). The string may be a single
glyph or several lines; lines wrap at a maximum width taken from the settings.

**Editing.** Typing, arrow keys, selection and the clipboard behave as in a text field. Data
model: the tab holds a sequence of glyph references, placeholders and line breaks, plus a cursor
index. Commands: `editView.insertText`, `editView.setText`, `editView.getText`,
`editView.setCursor`. The system character picker (`edit.emojiAndSymbols`, Cmd+Ctrl+Space)
and OS text services are platform features.

**Find glyphs** (`edit.find`, Cmd+F; panel `editView.findGlyphs`; query `glyph.search`):

- results update on each keystroke; the first result is preselected;
- a query with spaces is split and every term must occur in the glyph name;
- a literal character finds the glyph encoded with it;
- the magnifying-glass menu sets the mode: Name, Unicode (query compared with hexadecimal code
  points), or All;
- Shift+click selects a range, Cmd+click toggles single results, Return inserts all selected
  glyphs at the cursor.

**Stepping through the font.** Home and End replace the current glyph with the previous or next
glyph of the font (`editView.previousGlyph`, `editView.nextGlyph`); with Shift they step through
the glyphs as currently listed in Font View, honouring its filter
(`editView.previousVisibleGlyph`, `editView.nextVisibleGlyph`). Fn+Left/Right stand in for
Home/End.

**Placeholders** (`edit.addPlaceholder`, Cmd+Alt+Shift+P). A placeholder renders as the current
glyph, whatever that is. A tab can hold many; all change together when the current glyph
changes. This makes one spacing string serve every letter in turn.

**Writing direction** (`editView.setWritingDirection`): three buttons at the bottom right for
left to right, right to left, top to bottom. Top to bottom switches the Info box to the vertical
form.

**Sample strings.** Stored application-wide in Settings > Sample Strings.
`edit.selectSampleText` (Cmd+Alt+F) opens the list (`editView.sampleStrings`), chosen by click
or arrow keys; a pin keeps it open (`sampleStrings.togglePinned`).
`edit.selectNextSampleString` and `edit.selectPreviousSampleString` (Edit > Other) cycle without
the list and have no default shortcut.

### Text Preview window

`window.textPreview` opens a separate window for long text; Edit View itself becomes slow with
much text. Controls: style pop-up (`textPreview.setStyle`), size field
(`textPreview.setFontSize`), pin (`textPreview.togglePinned`), text field
(`textPreview.setText`), context menu with Layout Orientation
(`textPreview.setLayoutOrientation`) and Writing Direction
(`textPreview.setWritingDirection`), and a find/replace bar (`textPreview.find`, Cmd+F) with
Insert Pattern (`textPreview.insertPattern`, Cmd+Ctrl+Alt+P).

The reference shapes this text with the operating system's engine, so it is a true preview of
all features, and draws unsupported characters in a fallback font. Until M8, FontCraft lays the
text out with the Edit View text layout (Unicode mapping, the document's kerning, the writing
direction) and draws characters the font lacks in the interface font (the owner's decision,
`docs/plans/M5.md` § 7 question 6). A shaping engine comes with M8's features.

Mixed directions in Text Preview follow a simple set of rules (`crates/textlayout/src/bidi.rs`),
short of the Unicode bidirectional algorithm, which M8 brings. Edit View keeps one direction for
its whole text.

- Letters of right-to-left scripts (Hebrew, Arabic, Syriac, Thaana, NKo and the other
  right-to-left blocks) form right-to-left runs; other letters form left-to-right runs.
- Digits, Arabic-Indic digits included, form left-to-right runs. So "12 34" in right-to-left
  text shows as "12 34", where the Unicode algorithm gives "34 12"; M8's UAX #9 settles it.
- A neutral (space, punctuation, symbol) between two runs of the same direction takes that
  direction; otherwise it takes the base direction of the text or of its range.
- A combining mark takes the direction of the character before it.
- A range set with Writing Direction has its own base direction, one level inside the text's.
- On each line, the runs are ordered by their direction and the base direction; a run keeps its
  letters in their own order. Trailing spaces take the text's direction.
- Kerning applies between neighbours in the same run, from the table of that run's direction.

## Measuring

**Info box.** See the selection part above.

**Measurement tool** (`measurement`, L):

- While active, every node and anchor of every visible glyph is labelled: on-curve coordinates
  blue, off-curve green, X/Y deltas between neighbouring on-curve nodes red.
- X deltas are taken relative to the master's italic angle; zero means the segment follows it.
- Dragging draws a ruler that prints the distance between consecutive crossings with the
  outlines; Shift constrains it to horizontal or vertical. The ruler's end shows its angle,
  counterclockwise, 0 for straight up.
- G during the drag leaves a measurement guide. Cmd held gives the Select tool.
- Holding Cmd+Ctrl+Alt with another tool gives the Measurement tool temporarily.
- Headless: `measure.ruler`, `measure.coordinates`.

**Measurement guides** (`guide.toggleMeasurement`): any local or global guide can be switched
to measurement mode from its Info box. It then prints the distances between its crossings with
paths and components whenever guides are shown, with any tool.

**Overlaps.** Rulers and measurement guides measure the merged shape; stretches inside
overlapping paths are skipped and drawn light grey.

How `measure.ruler` finds crossings (our design):

- Components are resolved and measured like paths. Paths that do not form a valid outline and
  components of missing glyphs are left out.
- The merged shape follows the non-zero winding rule: a crossing counts only where the line
  passes between outside and inside the outline. Crossings of open paths are all kept.
- A line that only touches a contour, at a node or along an edge, does not cross it.
- Each gap between two crossings reports its distance, `dx`, `dy`, `italicDx` (`dx − dy·tan` of the
  master's italic angle) and whether it lies inside the outline (a stem) or outside (a counter).
- With `guide` instead of `from` and `to`, the whole line of the guide is measured; this is what a
  measurement guide prints.

How FontCraft's Measurement tool behaves (our decisions where the handbook is silent):

- The labels cover the edited glyph only; the other glyphs of the tab are not labelled yet. A
  label shows at most two decimals. Deltas print `italicDx, dy`.
- The ruler's ends snap like a dragged point (`Ctrl` turns it off). With `Shift`, the ruler is
  horizontal or vertical, whichever is nearer to the pointer; it does not snap then.
- The ruler stays after the button comes up, until the next press, `Escape` or a tool change. A
  click without a drag removes it.
- `G` during the drag, with any modifiers, adds a local guide from the ruler's start along the
  ruler and turns it into a measurement guide, as one undo step (`command.batch` of `guide.add`
  and `guide.toggleMeasurement`). The guide's position is rounded to the grid like any new guide.
- Overlapped stretches are not drawn grey yet. Cmd for the Select tool and Cmd+Ctrl+Alt for a
  temporary Measurement tool are not built yet.
- Measurement guides print each gap's distance at its middle, in the guide's colour, with a dot
  at each crossing.

**Measurement line** (`view.showMeasurementLine`, text mode):

- one horizontal line across the text at an adjustable height (`view.setMeasurementLineHeight`;
  Cmd+Ctrl+Alt+click or drag, or drag with the Measurement tool);
- per glyph it prints the distance from each side of the advance to the first outline crossing
  on that line (`measure.sidebearingsAtHeight`);
- thin grey lines mark the advance widths; negative kerning is tinted light blue, positive
  yellow (both settings).

## Annotations

Tool `annotation` turns the Info box into a palette of six sub-tools (`annotation.setTool`).
Annotations are stored on one glyph layer and never exported; `view.showAnnotations` hides them.

| Sub-tool | Click | Handle |
|---|---|---|
| `annotationSelect` | select; Shift+click adds | |
| `annotationText` | new note; double-click edits text (multi-line) | right handle sets box width |
| `annotationArrow` | new red arrow | handle on the stem rotates it |
| `annotationCircle` | new red circle | bottom handle sets the diameter |
| `annotationPlus` / `annotationMinus` | stamp a plus / minus sign | |

With the tool active: Cmd+A selects all annotations of the layer (`annotation.selectAll`),
arrow keys move the selected ones by 1, Shift 10, Cmd 100 units (`annotation.move`), Delete
removes them (`annotation.delete`). Data commands: `annotation.add`, `annotation.update`,
`annotation.list`.

How FontCraft stores and edits them (our decisions where the handbook is silent):

| Type | Stored as | Values | Default from `annotation.add` |
|---|---|---|---|
| `text` | `Text` | text (several lines), box width | width 200 units |
| `arrow` | `Arrow` | angle in degrees, kept from 0 to 360 | 45° |
| `circle` | `Circle` | diameter, stored in the file's `width` key | 100 units |
| `plus`, `minus` | `Plus`, `Minus` | none | |

- Each layer side (foreground, background) has its own list, addressed by index like the
  other target lists. Deleting an annotation moves the later ones up.
- A value given for a type that does not use it is an error. An empty text removes the text.
- Positions are not rounded to the grid: they mark places on the drawing, not outline points.
- A width or diameter of 0 means the file stores none; the canvas draws the default then.
- The direction of a positive angle is open, as for every angle in the file format
  (`30-source-formats.md`, open question 1).

How the Annotation tool draws and edits them (our decisions; the handbook shows the marks but
gives no sizes):

- Each sub-tool is a tool of its own (`annotationSelect` … `annotationMinus`); `A` keeps the
  sub-tool in use, else returns to the one used last (Annotation Selection before any), and
  `Shift+A` steps to the next, as for every shared button. `annotation` names Annotation
  Selection.
- A note's position is the top-left corner of its box; its handle is the box's top-right
  corner. The text wraps to the box width; the box grows with the text.
- An arrow's position is its tip. Its angle runs counter-clockwise from the x axis and points
  from the tip to the tail; the tail is its handle. An arrow is 48 points long on screen at
  every zoom.
- A circle's position is its centre; its handle is the bottom of the circle, and dragging it
  sets the diameter to twice the handle's distance from the centre.
- Plus and minus signs are 16 points across on screen. A type FontCraft does not know is drawn
  as a dot with its name.
- A click on empty canvas with a drawing sub-tool runs `annotation.add` with the command's
  defaults. A click on an existing annotation selects it instead, with any sub-tool. Dragging an
  annotation moves the selected ones (`annotation.move`); dragging the handle of a selected one
  runs `annotation.update`. Each drag is one undo step.
- Handles show and work on selected annotations only. Editing a note's text by double-click is
  not built yet; `annotation.update` with `text` does it.

## Images

**Adding.** `glyph.addImage` (Glyph > Add Image…), or drop a file on a Font View cell or on a
glyph in Edit View. The reference takes whatever its OS decodes (JPEG, PNG, PDF, TIFF, PSD
named). Only a path relative to the document is saved; a broken path shows a missing-image
symbol. One image per layer is implied.

**Defaults.** Placed at the layer origin, one DTP point of the image per font unit.

**Editing.** Drag to move; resize with the bounding box or the Scale tool; rotate with the
Rotate tool; the Transformations palette also applies. Info box (`infoBox.image`): X, Y, width,
height, rotation (`image.setTransform`), reveal file (`image.revealFile`), lock (`image.lock`).
Locking makes an image unselectable; unlock from its context menu (`image.unlock`).

**Cropping.** Ctrl+Alt+drag a handle (`image.setCrop`); context menu: Set Crop to Layer Bounds
(`image.setCropToLayerBounds`) and Reset Crop (`image.resetCrop`).

**Display and export.** `view.showImage` toggles display, except that empty glyphs always show
their image. Images are skipped at export unless a bitmap colour font (sbix) is built.

## Previewing and testing

**Kerning modes.** Three modes (none, kerning, locked); see the kerning notes.

**Masters.** Edit View draws in the selected master. `view.selectMaster`: Cmd+1 to Cmd+9 or
the master icons in the toolbar. Picking a master layer in the Layers palette changes only the
edited or selected glyphs.
FontCraft: the master belongs to the tab, and a new tab takes the master of the tab it was
opened from. Selecting a master drops the per-glyph layers chosen in the Layers section. The
toolbar icons wait for M9. See `docs/spec/04-palette.md` › Layers for the active layer.

**Features menu** (`editView.featuresMenu`, bottom left):

- lists the compiled features; choosing one toggles it (`editView.toggleFeature`); several may
  be on; the dash on top turns all off (`editView.clearFeatures`);
- the button shows the active tags and is accent-coloured when any is on;
- typing while the menu is open jumps to an entry, the dash included;
- Localized Forms plus a script and language choice at the bottom
  (`editView.setLocalizedForms`); needs a valid `locl` feature;
- new features appear only after recompiling in Font Info > Features;
- Edit View applies substitutions, kerning and cursive attachment; other positioning must be
  tested in an exported font.

**Preview Area and Preview Panel.** The eye button splits the tab (`view.togglePreviewArea`);
Window > Preview Panel (`window.previewPanel`) gives the same in its own window, which follows
the active Edit View tab and is blank otherwise.

- Instance pop-up (`preview.setInstance`): a named instance, the dash (mirror the Edit View
  master), or Show All Instances (`preview.showAllInstances`), which shows only the current
  glyph in every instance; eye icons pick which instances take part
  (`preview.toggleInstanceVisibility`).
- Otherwise the whole tab text is drawn on one line, current glyph centred. Drag to shift it
  (`preview.setOffset`); double-click a glyph to edit it.
- Context menu Always Center Active Glyph (`preview.alwaysCenterActiveGlyph`); when off, the
  text is fitted flush left or right to use the space.
- Invert (`preview.invert`), Flip upside down (`preview.flip`), blur slider
  (`preview.setBlur`).
- Rendering uses intermediate and alternate layers and some custom parameters.
- `preview.render` (ours) returns the preview as an image for agents.

**Testing exported fonts** (behaviour rows only; the export dialog is specified elsewhere):

- OS and apps cache fonts; restart the testing app, or use the export dialog's Test Install,
  which activates the font from memory.
- Exporting into the Adobe fonts folder makes fonts available to Adobe apps at once.
- .woff2 / .woff export plus a minimal HTML page with `@font-face`; Shift-reload to skip the
  browser cache.

## Open questions

1. Submenu labels under View > Show Nodes: only "In Background" and "Extra Nodes" are confirmed;
   the foreground label is from memory. Shortcuts of most View toggles are not given.
2. Which Path menu item turns into Clear Background with Alt held, and whether it acts on the
   active layer only or on all layers of the selected glyphs.
   **Answered for M3 (our decision, 2026-10-09):** Clear Background is its own Path menu item
   (`path.clearBackground`); it acts on the active layer of the targeted glyphs.
3. Assign Background: how glyphs and masters are matched between the two fonts, and what happens
   to glyphs missing in the source.
   **Answered for M3 (our decision, 2026-10-09):** glyphs are matched by name, masters by name and
   then by order; glyphs missing in the source are skipped and listed in the result.
4. Which tool becomes active when Escape leaves text mode, and where the cursor lands when
   Escape returns to it.
   **Answered for M5 (our decision):** leaving text mode makes the Select tool active; returning
   puts the cursor before the glyph that was edited (`editView.toggleTextMode`).
5. Select Sample Text: replace the whole tab text or insert at the cursor? Does the string wrap
   around at the end of the list for Next/Previous?
   **Answered for M5 (our decision):** it replaces the whole tab text; Next and Previous wrap
   around (`edit.selectSampleText`, `edit.selectNextSampleString`).
6. Home/End at the first or last glyph: stop or wrap? Does Home/End act on a text selection of
   several glyphs?
   **Answered for M5 (our decision):** they stop at the ends of the glyph order, and with a
   selection of several glyphs they replace only the current glyph (`editView.previousGlyph`,
   `editView.nextGlyph`).
7. How placeholders serialise in the tab string of the file format and on the clipboard.
   **Partly answered (source), the rest our decision (M5):** display strings write a glyph without
   a character as `/name` (`/Smily` and `/_part.test` in corpus files the reference saved). Ours:
   an escape ends at the next slash, space or line break; a placeholder is `/Placeholder`; on the
   plain-text clipboard a placeholder is the current glyph's text (`text.displayStrings`,
   `editView.insertText`, `edit.addPlaceholder`).
8. Zoom step factor of Zoom In/Out and the minimum and maximum zoom values.
   **Answered for M3 (our decision, 2026-10-09):** a factor of √2 per step, within the existing
   `ZOOM_RANGE`.
9. Measurement line: visible with the Measurement tool as well as the Text tool? Default height?
   Is the height stored per tab, per document or per app?
   **Answered for M3 (our decision, 2026-10-09):** the line shows with the Measurement tool and
   stays until the next click or tool change. The text-mode measurement line (height, storage) is
   M5.
   **Answered for M5 (our decision):** the text-mode line height is kept per tab and starts at half
   the x-height of the tab's master (`view.setMeasurementLineHeight`).
10. Ruler: are components and other visible glyphs measured exactly like paths; is the ruler
    kept after the mouse is released?
    **Answered for M3 (our decision, 2026-10-09):** components are measured like paths; the ruler
    stays until the next click or tool change.
11. Keyboard shortcut of the Annotation tool; default width, angle and diameter of new
    annotations; the units and anchor point of each annotation's position.
    **Answered for M3 (our decision, 2026-10-09):** text width 200 units, circle diameter 100,
    arrow at 45°. `annotation.add` applies them when the caller gives no value, so a tool click
    and the command give the same annotation. The Annotation tool's key is `A`, as its row says.
12. Image crop: coordinate space of the stored rectangle; meaning of "layer bounds"; whether a
    layer can hold more than one image; which formats FontCraft decodes natively (PDF, PSD).
13. Preview blur range and unit; which custom parameters the instance preview honours.
    **Answered for M5 (our decision):** blur from 0 to 10 screen points of radius
    (`preview.setBlur`). Instance parameters wait for M9, when instances are interpolated.
14. Exact order of the bottom-bar controls and where the kerning mode button sits.
    **Answered for M5 (our decision):** the Preview Area's eye button sits at the left end; at the
    right end, from left to right, the three writing-direction buttons, the zoom controls of M3
    and the kerning mode button last, at the bottom right as the handbook places it.
15. Info box: can the Unicode value be edited in place; how the box lays out when several
    glyphs are selected in text mode.
    **Answered for M5 (our decision):** the Unicode value is shown, not edited; with several glyphs
    selected in the text the box describes the glyph after the cursor (`infoBox.glyph`).
16. Edit > Info for Selection is given as Cmd+Alt+I in the guides section of this chapter but
    as Cmd+Shift+I (the Show Info shortcut) in the Palette chapter; one of them is a typo.
17. The portable replacement for the UnicodeChecker hand-off and for the OS character picker.
