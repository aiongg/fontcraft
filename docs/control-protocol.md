# Control protocol

`fontcraft --control <port>` listens on `127.0.0.1:<port>` (loopback only). One JSON request per
line, one JSON reply per line:

```
→ {"id": 1, "method": "engine.execute", "params": {"command": "glyph.newGlyph", "params": {"name": "A.alt"}}}
← {"id": 1, "ok": true, "result": {"name": "A.alt"}}
← {"id": 1, "ok": false, "error": "glyph `A.alt` already exists"}
```

The shapes are the sibling Crafting Apps'. There is no default: the app listens only when started with `--control`. The conventional
port is **7985** (the siblings use 7979), so one agent can drive two apps. If the app logs that it could not bind the port, another program holds
it: pick another, or your requests reach that program.

**Only requests are read.** A line must be a JSON object with a string `method` (`id` and
`params` are optional; blank lines are skipped). Anything else gets one error reply and the
server closes the connection: text that is not JSON, an array or a number, an object without
`method`, bytes that are not UTF-8, a line longer than 4 MiB. A web page's cross-origin request to
the port is therefore rejected at its first line and its body never runs. At most 16 connections
are served at once. There is no authentication: enable the port only while you use it.

Requests are answered on the UI thread between frames, exactly as a click would be. A bug inside a
request becomes an error reply; the app keeps running.

Transport: `crates/ui/src/control/server.rs`, given its listener by the desktop app. Dispatch:
`crates/ui/src/control/mod.rs`. `ui.screenshot` needs the window, so the reply waits until the
next frame is captured.

## Confining file access

```sh
fontcraft --control 7985 --automation-read-root /work/fonts --automation-write-root /work/fonts/build
```

`--automation-read-root <dir>` and `--automation-write-root <dir>` confine the files that control
requests read and write. They are the same flags as `fontcraft-cli mcp` takes, and also come as
`--flag=<dir>` or as the variables `FONTCRAFT_AUTOMATION_READ_ROOT` and
`FONTCRAFT_AUTOMATION_WRITE_ROOT` (a flag replaces its variable).

- Every file a request reads (open, import, a source's linked files, images, feature includes…)
  must lie inside the read root. Every file it writes (save, export, `ui.render` and
  `ui.screenshot` with `path`…) must lie inside the write root. Anything else is an ordinary error
  reply that names the root.
- A root left out grants none of its access. Without either flag nothing is confined.
- Each root must be an existing folder, or the app does not start. Each flag is given once.
  Without `--control` the flags are ignored.
- Paths are made absolute against the app's working directory and their links are followed (for
  a file that does not exist yet, its nearest existing folder's). They are compared with the root
  by whole folder names, ignoring case on Windows. `..` below a missing folder, a link that leads
  nowhere and Windows device names are refused.
- Macros, recent documents and URLs (`script.run`, `file.openRecent`, `url.open`, `url.show`…) are
  refused while a root is set: the steps they run cannot be checked first.
- Only requests are confined, together with everything they run, including the keys, pointer
  events and dialog buttons they inject, and the frames that carry their window input
  (`ui.click`, `ui.drag`, `ui.wheel`, `ui.move`), so a menu item an agent clicks is confined too.
  The person at the keyboard is not: files opened from the
  command line or a file dialog, and everything done with the mouse and keyboard, work as usual.
  A mouse press or key press of the person's drops the window input still waiting, so it never
  shares a frame with the agent's.
- A paste that waits for the system clipboard (Edit › Paste clicked through window input) runs in
  a later frame, outside the confinement. A paste reads no files.
- A link in an open document that points outside the read root is treated as a missing file when
  a request reads it.

[MCP › Confining file access](mcp.md#confining-file-access) has the details of what each command
is checked for.

## Methods today

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `{command, params?}` | runs any engine or UI command; the command's own result |
| `engine.commands` | | `{commands: [{id, label, menu, shortcut, params, query, enabled, disabled_reason?}]}`; UI commands carry `ui: true` |
| `document.inspect` | | `{documents: [{index, active, title, path, modified, revision, familyName, glyphs, masters, undo, redo}], active}` |
| `ui.inspect` | | the UI state: `selected_glyphs`, `tabs` (each `{id, glyphs, edited, view: {scale, center}, selection, focus, selection_glyph}`: `selection` and the path `focus` belong to the glyph `selection_glyph`), `active_tab` (an index into `tabs`; `null` while Font View shows), `editing` (the edited glyph's name), `closed_tabs`, `display` (the View toggles), `show_palette`, `canvas_size`, `status`, `pendingInput` (window input events not yet drawn), `windows` (the app's OS windows with their rects in main-window points; see [Window input](#window-input)) |
| `ui.menu.list` | | the menu tree: `[{title, items: [{command, label, path, shortcut, enabled, disabled_reason?, checked?, alternate?, alternateOf?}]}]`. A row with an Option alternate names its command in `alternate`; the alternate follows as an item of its own, with `alternateOf` naming the row: what the row reads and runs while Alt is held |
| `ui.menu.invoke` | `{command, params?}` | runs a menu item as a click on it does, and returns the command's result: a command with a dialog fired without parameters opens the dialog on the selection and returns `{dialog}`; Close and Quit with unsaved work ask about it and return null; the follow-up question its result asks (a pasted outline outside the glyph, imported files without a glyph) opens as a dialog; an Option alternate (`alternateOf` in `ui.menu.list`) is invoked by its own id, as a click with Alt held; an id that is in no menu is an error |
| `ui.screenshot` | `{path?}` | captures the window as PNG, written to `path` or returned as base64 `png`; `{width, height, pixelsPerPoint, path?, png?}`. Sizes are in pixels. The reply comes after the next frame is drawn. The image is the main window only: panels floating in OS windows of their own are not in it. Headless hosts (tests, MCP) answer with an error |
| `ui.render` | `{path?, glyphs?, text?, master?, layer?, size?, scale?}` | draws glyphs filled by the nonzero rule, black on white, without a window; `{width, height, glyphs, master?, path?, png?}`. `glyphs` names them, `text` maps characters to glyphs; neither: the showing tab's line, else the Font View selection. `master` (an id) defaults to the master the showing tab draws in; Font View draws the first master. `size` is pixels per em (default 256), `scale` pixels per font unit instead. `layer` draws that layer of the one glyph named. The same request gives the same bytes |
| `ui.tool.select` | `{tool}` | makes a pointer tool active (the command `tool.select`): `{tool, previous}`. A gesture in progress is cancelled |
| `ui.tool.list` | | the pointer tools (the command `tool.list`): `{active, tools: [{id, label, shortcut, group}]}` |
| `ui.pointer` | `{events: [{kind, x, y, space?, mods?}]}` | drives the active tool like the mouse; see below |
| `ui.key` | `{key, shift?, alt?, cmd?, ctrl?, action?}` | a key as the keyboard gives it; see below |
| `ui.text` | `{text}` | types as the keyboard does, at most 4,096 characters: into the open dialog's field that has the keyboard, else into the text field that has it, in the window that has the keyboard (a floating panel's OS window after a `ui.click` in it), at the next frame (`{field: true}`); else, in text mode, into the tab's text at the cursor (`{typed: true}`); otherwise it types nothing (`{typed: false}`) |
| `ui.dialog.inspect` | | the open dialog: `{open, dialog, title, command, mode: "preview" \| "runOnOk", message, tabs, fields: [{id, label, kind, value, focused, min?, max?, choices?, labels?, default, doc}], editing, params, error, actions, buttons, captions: {ok, cancel}, canChooseFolder}`; a dialog with a third button lists its id in `buttons` (`["ok", "replace", "cancel"]`) and its caption in `captions`; `{open: false}` when none is. `tabs` (`[{id, tab, label, active}]`) and `actions` (`[{id, label, command}]`) are `null` when the dialog has none |
| `ui.dialog.set` | `{field, value}` | gives a field of the open dialog a value and previews it; a number field also takes text. The `ui.dialog.inspect` reply |
| `ui.dialog.confirm` | | OK: the preview becomes one undo step, `{confirmed, changed, command, params}`; in a `runOnOk` dialog the command runs once, `{confirmed, command, params, result}`, and a failure keeps the dialog open |
| `ui.dialog.cancel` | | Cancel: the font as it was when the dialog opened; `{cancelled}` |
| `ui.dialog.click` | `{id}` | the same as `ui.click {id}` |
| `ui.dialog.key` | `{key, shift?}` | a key for the open dialog (`Return`, `Escape`, `Tab`, `ArrowUp`, `ArrowDown`) as `ui.key` gives it: `{handled, open, error?}`; `handled` is false for any other key or with no dialog open |
| `ui.dialog.text` | `{text}` | types into the dialog field that has the keyboard, as `ui.text` does: `{field}`, false when none has it |
| `ui.click` | `{id}` | presses a control of the open dialog: `ok`, `cancel`, a third button by its id (`replace`, `decompose`), a tab by its dialog id (shows that tab), an action by its id (runs it: `{action, command, text, result}`; `text` also goes to the system clipboard), or a field by its key (a checkbox or lock toggles; a number field takes the keyboard, its text selected, for `ui.text` and `ui.key`; a folder field opens the folder chooser, or without one takes the keyboard) |
| `ui.click` | `{x, y, button?, count?, mods?}` | a mouse click at a window point; see [Window input](#window-input) |
| `ui.drag` | `{x, y, toX, toY, steps?, count?, button?, mods?}` | press, move in steps, release; see [Window input](#window-input) |
| `ui.move` | `{x, y}` | the pointer moves to a window point (hover) |
| `ui.wheel` | `{x, y, dy?, dx?, unit?, mods?}` | a wheel turn over a window point |
| `ui.set` | `{key: bool, …}` | turns view switches on or off with their menu commands; see [View switches](#view-switches) |
| `ui.resize` | `{width, height}` | the window's inner size in points, width 640 to 16,384 and height 400 to 16,384, at the next frame. A size larger than the monitor, or than the graphics device can draw (8,192 points at most), is clamped; `{width, height, clamped}` has the size asked of the window |
| `ui.focus` | | brings the window to the front with the keyboard, at the next frame; `{focus: true}` |
| `ui.contextMenu.list` | `{x?, y?, space?}` | the open canvas context menu: `{open, glyph, screen, at, hit, sections: [{context, items: [{command, label, enabled, disabled_reason?, checked?, alternate?, choices?: [{value, label, checked}]}]}]}`; `{open: false}` when none is. With `x` and `y` it first opens the menu there, as a right-click does (`space` as in `ui.pointer`, default `font`); that is refused while a tool is dragging or a dialog is open |
| `ui.contextMenu.invoke` | `{command, value?}` | runs an item of the open canvas context menu as a click on it does, and closes the menu; the item's result. A submenu item (one `ui.contextMenu.list` shows with `choices`) needs `value`, one of its choices' values; a plain item takes none. The item is checked as the menu would show it now: a disabled or missing item is refused and the menu stays open; a menu whose tab or glyph changed, or that a dialog covers, closes and the request is refused. Refused while the question about unsaved changes is open |
| `ui.wait` | `{frames?, idle?, timeoutMs?}` | the reply waits until `frames` frames (at most 3,600) have been drawn, or until the window is idle, or both: `{frames, idle, timedOut, elapsedMs}`. Without `frames` it waits for idle; with `frames` alone only for the frames. Idle means no window input queued or running, no screenshot or input reply waiting, and at least two frames drawn since the request, so what the request before it fired has been applied and drawn. `timeoutMs` (default 5,000, at most 30,000) ends the wait with `timedOut: true`. Headless hosts answer with an error |
| `events.list` | `{since?}` | what happened, oldest first: `{events: [{seq, kind, …}], last, dropped}`; see [Events](#events) |
| `events.clear` | | empties the event ring: `{cleared}`. Sequence numbers keep counting |
| `control.methods` | | this table |

`ui.inspect` answers with camelCase keys. Beside the active pointer tool (`tool`), `glyph`,
`panels`, `canvas`, `status`, `tabs`, `activeTab`, `editText` and the panels' own keys, it reports:

| Key | Holds |
|---|---|
| `display` | the View toggles, by the names above: `nodes`, `nodesInBackground`, `extraNodes`, `metrics`, `metricsNames`, `info`, `background`, `guides`, `boundingBox`, `annotations`, `fillPreview`, `anchors`, `groupMembers`, `hints`, `masterCompatibility`, `keepLayerSelectionInSync`, `showImage` |
| `switches` | every menu row that turns something on or off, by command id, and whether it is on (the row's check mark) |
| `selectedGlyphs` | Font View's selection, in the order it was made |
| `editing` | the edited glyph while an Edit View tab shows, else `null` |
| `interaction` | the label of the interaction in progress (a drag, a previewing dialog), else `null` |
| `dialog` | the open form dialog's id, else `null`; `ui.dialog.inspect` has its fields |
| `transform` | the Transformations panel's settings, as `transform.getSettings` returns them |
| `fontView` | Font View's state, as `fontView.getState` returns it |

Five read-only commands return the same state through `engine.execute`: `editView.getText` (the `editText` reply),
`editView.getFeatures` (the Features menu), `fontView.getState`, `transform.getSettings` and `view.getDisplayOptions`
(`display` with the showing tab, zoom, centre, `palette`, `direction` and `previewFeatures`). They change nothing.

### Pointer and keys

`ui.pointer` runs its events in order through the same path as the mouse: the active tool turns
them into commands, a drag is one interaction (one undo step), and the tab's selection follows.
Every event is checked before the first one runs.

| Field | Values |
|---|---|
| `kind` | `down`, `drag`, `up`, `move` (hover), `doubleclick` (send it after the two clicks' `down` and `up`) |
| `x`, `y` | the position |
| `space` | `font` (default): font units of the edited glyph; in text mode, font units of the laid-out text (the first line's baseline at y = 0, its start at x = 0). `screen`: window points, as the mouse reports them |
| `mods` | modifier names held: `shift`, `alt`, `cmd`, `ctrl`. `cmd` is Command on a Mac and Control elsewhere; `ctrl` is the key that suspends snapping |

At most 10,000 events per request. The reply is `{tool, busy, interaction, selection, overlays,
errors}`: `busy` is true while a gesture is unfinished (send its `up`), `overlays` lists what the
canvas draws for the tool (marquee, snapping lines, hover), `errors` the messages of commands that
failed. Use `space: "screen"` with the Hand tool: it scrolls by the pointer's movement on screen.

`ui.key` resolves a key as the keyboard does: first the Edit View keys of the tools (tool letters
`V` Select, `Shift+V` Lasso Select, `P` Draw, `B` Pencil, `F` Primitives, `H` Hand, `Z` Zoom,
`E` Knife, `R` Rotate, `S` Scale, `L` Measurement, `A` Annotation; a letter shared by several
tools keeps the one in use, else picks the one used last, and `Shift` with it steps to the next; `Space` held for the Hand tool; arrows with the font's increments;
`Return`, `Delete`, `Tab`, `Escape`), then the menu shortcuts. `key` is a character or a name:
`ArrowLeft`, `Return`, `Delete`, `Backspace`, `Tab`, `Escape`, `Space`. `action` is `tap`
(default: press and release), `press` or `release`; hold `Space` with `press`, send pointer
events, then `release`. The reply is the `ui.pointer` reply plus `handled` (whether anything
took the key), and `command` and `result` (or `error`) when a menu shortcut ran. A shortcut
whose command has a dialog opens it, as the keyboard does: `result` is then the
`ui.dialog.inspect` reply. Other shortcuts act on the selection (Font View's selected glyphs, or
Edit View's edited glyph and selection). A shortcut works only where its command's `when` says
(`commands.list`): one for `editView`, `textMode` or a tool (`tool:trueTypeInstructor`) does not
fire while Font View shows, and a tool's one only while that tool is current; the keyboard follows
the same rule. While Font View shows, the tools take no key: the arrows, `Home` and `End`
(alone or with `Shift`) and `Return`, `Delete` and `Backspace` go to the window as the keyboard
sends them, so the grid or list that has the keyboard (after a `ui.click` on a cell) moves the
selection, opens it or asks to remove it; the reply is `{queued, handled: true, fontView: true}`.
A character no shortcut there takes (a letter with no `cmd`, `ctrl` or `alt`) is typed into the
window the same way, and the grid or list that has the keyboard selects the first glyph whose
name starts with what was typed (type-ahead); with nothing focused it does nothing. Every other
key goes on to the menu shortcuts. While a context menu is open (the canvas menu or Font View's
glyph menu), every key goes to the window that has the keyboard as the keyboard sends it, and the
menu takes it as it takes the keyboard's (`Escape` closes it). The reply is `{queued, handled: true, contextMenu: true}`.
While a text field has the keyboard (a panel's field or the macro editor, after a `ui.click` on
it, in the main window or a floating panel's OS window), every key goes to that field's window the
same way, before the tools and Font View: the
arrows move the caret, `shift` with them selects, and a character with no `cmd`, `ctrl` or `alt`
is typed. The reply is `{queued, handled, field: true}`; a shortcut a key fires there is not named
in it. `ui.text` types into such a field too (`{field: true}`). While a modal dialog shows over the
window (About FontCraft, Settings, …) and no field of it has the keyboard, every key goes to the
window the same way, and the dialog takes it as it takes the keyboard's: `Escape` closes it. The
reply is `{queued, handled: true, dialog: true}`. A form dialog (`ui.dialog.inspect`) takes its own
keys first, as below.

In text mode (the Text tool), `ui.key` and `ui.text` type into the tab's text at the cursor, as
the keyboard does: characters, `Space`, `Return`, `Backspace`, `Delete` and the arrows (with
`Shift` to select); `Home` and `End` step to the previous and next glyph of the font; `Escape`
leaves text mode. The spacing and kerning shortcuts act on the glyphs at the cursor in the
tab's master. A character with `cmd` or `ctrl` goes to the menu shortcuts; `Cmd+A`, and
`ui.menu.invoke` of `edit.copy`, `edit.cut` and `edit.selectAll`, act on the text.

### Window input

`ui.click {x, y}`, `ui.drag`, `ui.wheel` and `ui.move` act on the window as the mouse does. They
reach every widget: Font View cells and list rows, panel fields and lists, the tab strip, menus,
and the canvas, where a right-click (`button: "right"`) opens the context menu that
`ui.contextMenu.list` reads. `ui.pointer` stays the way to drive a tool in font units.

| Field | Values |
|---|---|
| `x`, `y` (`toX`, `toY`) | main-window points from the top left, as `ui.screenshot` divided by `pixelsPerPoint` shows them; at most 100,000 either way. The desktop around the main window uses the same origin and unit, so a point over a floating panel's OS window reaches that window (see below) |
| `button` | `left` (default), `right`, `middle` |
| `count` | clicks, 1 (default) to 16; `2` is a double click. On a drag the extra clicks come at the end point |
| `steps` | moves of a drag between press and release, 1 to 256 (default 8) |
| `dy`, `dx` | wheel notches: `dy` up (+) or down (−), `dx` right (+) or left (−); at most 100 each. `unit: "point"` makes them points |
| `mods` | as in `ui.pointer`; the flags `shift`, `alt`, `cmd` and `ctrl` (true) add to it |

Panels floating in OS windows of their own (Font Info, the Bitmap Glyph Editor, Space Center, …)
are palettes above the main window. A point inside one of them goes to that window, at the same
place on the desktop; every other point goes to the main window. `ui.inspect` lists the windows
under `windows`: `[{window, rect: [x0, y0, x1, y1], focused}]`, the main window first as `main`,
each `rect` the window's inside in main-window points, so the centre of a control at (cx, cy) in a
floating window is at (x0 + cx, y0 + cy). A drag stays in the window its press lands in, as a held
mouse button does. A click or drag gives its window the keyboard, as a click on the desktop does, so
`ui.text` and `ui.key` then reach the field it focused; nothing is asked of the system for a click
in the main window while no window of the app has the keyboard. Where the main window does not
report its place on the desktop (Wayland), panels float inside it and every point is the main
window's.

The reply is `{queued, pending, window}`: the events this request added, the events not yet drawn
in all, and the window they went to (`main` or the id `ui.inspect` lists). The events are handed to the window one per drawn frame (each press and release after a move
to its position), so egui sees hover, clicks, double clicks and drags as it does from a mouse.
While the window is minimized or hidden no event is handed out. `ui.inspect` reports the count as
`pendingInput`; once it is 0 the last event has been drawn and the commands it ran are done, so
read the result then. The modifiers a request names are held for the frames its input spans,
from the press through every move to the release; the keyboard's own come back after. Off the
Mac, `cmd` and `ctrl` both hold the Control key, as the keyboard reports it.

The input still waiting is dropped when the person presses a mouse button or a key in the window,
and when the question about unsaved changes opens other than from the agent's own window input
(the person closes the window or quits).

Limits, as in the sibling apps: at most 256 `steps` and 16 `count`; 1,024 events waiting across
all requests. A request that breaks a limit, or does not fit in the queue, is refused whole
before any of its events is queued; retry once the waiting input has drained. Counts must be
non-negative integers; `0` acts as `1`.

`ui.click {id}` presses a dialog control (below); give `id` or `x` and `y`, not both (`id: null`
counts as no `id`).

### View switches

`ui.set` takes switches by key, each `true` or `false`, and runs the menu command of each switch
whose state differs, so the result is what the View or Window menu item gives. The reply has each
key's state afterwards. Every key is checked before the first switch moves; if a command fails,
the switches already moved are put back (Show Indexes to the labels it showed).

A key is a switch's command id (`view.showTunniLines`) or its short key: the id after its area,
without a leading `show`, `set` or `toggle`, first letter lower case.

| Keys | Switch |
|---|---|
| `nodes`, `nodesInForeground`, `nodesInBackground`, `extraNodes` | Show Nodes and its submenu |
| `metrics`, `metricsNames`, `info`, `background`, `guides`, `boundingBox`, `annotations`, `anchors`, `hints`, `image`, `groupMembers`, `masterCompatibility` | the View menu's Show items |
| `fillPreview`; `outline` | Fill Preview; `outline: true` is Fill Preview off |
| `tunniLines`, `curveTension`, `curveLength`, `indexes`, `detailsAcrossGlyphs`, `lockOutline`, `metricsBeam`, `stemWidths`, `counterWidths`, `cornerAngles`, `quickMeasurement`, `measurementLine`, `spacingCutOff` | the editing aids |
| `suggestHandles`, `suggestLines`, `suggestDistance`, `suggestStems` | the suggestions |
| `keepLayerSelectionInSync`, `editBackground`, `powerNudge` | Edit and Path switches |
| `palette`, `previewPanel`, `auditPanel` | the Palette, the Preview panel, the Audit panel |

The list follows the menus' switches; an unknown key is an error that lists them all. The
sibling apps' keys `panel`, `brightness`, `controlBar` and `status` have no switch here and are
unknown keys. Two keys for one switch (`fillPreview` and `outline`, or a short key and its
command id) that ask for opposite states are refused.

### Dialogs

A dialog is a form over one command's parameters. Open it with the UI command `dialog.open`
(`{dialog: "dialog.filterRoundCorners"}` or `{command: "filter.roundCorners"}`, with `params`
for first values and targets). Each field's id is its parameter's key. Every change previews the
command on the canvas inside an interaction (`ui.inspect` shows its label in `interaction`);
`ui.dialog.confirm` keeps it as one undo step, `ui.dialog.cancel` puts the font back. `params`
in `ui.dialog.inspect` is what OK runs: the same command with those parameters gives the same
font. While a dialog is open, `ui.key` goes to it (`Return`, `Escape`, `Tab`, `ArrowUp`,
`ArrowDown`, the rules of `dialog.fieldKeys`), `ui.text` types into its focused field, and
editing commands fail with "an interaction is running".

A dialog in `runOnOk` mode (the tabs of `dialog.export`, opened by `file.export`) previews
nothing, because its command writes files: `ui.dialog.confirm` runs the command once, and a
failure keeps the dialog open with the message in `error`.

Field kinds and how to set them with `ui.dialog.set`:

| `kind` | `value` |
|---|---|
| `number`, `integer` | a number, or its text |
| `bool`, `lock` | `true` or `false` (a `lock` is a lock button, closed while `true`) |
| `text` | a string of at most `max` characters; `""` is allowed. `lines` above 1 is a box in which Return starts a line and Cmd+Return is OK |
| `choice` | one word of `choices`; `labels` is what the menu shows |
| `multi` | a list of words of `choices` (any order; kept in the list's order) |
| `folder` | a folder path; `""` is none, and the parameter is left out |
| `order` | every row index of `labels` once, the first row on top (`dialog.shapeOrder`: the first is the bottom shape) |
| `list` | with `select: true`, the selected rows of `rows` (any order; kept in the list's order). Without it the list only shows `rows` and cannot be set |
| `rules` | a rule tree as `fontView.addSmartFilter` takes it, `{all \| any \| none: [rule or group]}` |

A field with a `group` sits under that heading in the window; the heading does not change the
parameters.

A dialog's fields need not be its command's parameters one to one: `params` in
`ui.dialog.inspect` is always what OK runs. `dialog.transformations` has Translate X and Y, Scale
horizontal and vertical with a lock and a slant kind, and runs `path.transformations` with
`translate`, `scale` and `cursivy`. `dialog.open` takes either shape in its `params`.

A dialog with tabs (`dialog.export`) shows one tab at a time; each tab is a dialog of its own
command. `ui.click {id: "dialog.exportUfo"}` shows another tab; a tab shown again has its values
back, and a tab shown first takes the values of the fields it shares with the one before. OK
with an empty folder field is an error over the control channel: set it first. A person
pressing OK in the window is asked for the folder. Follow-up questions (`dialog.pastedOutlineBounds`,
`dialog.importOutlinesMissingGlyph`) open after a person's run of their command, or after
`ui.menu.invoke`; `engine.execute` never opens one. Open one yourself with `dialog.open
{dialog}` and the parameters from the command's result.

`revision` in `document.inspect` grows with every change to a document, whoever made it. Compare
it to learn whether the font changed since you last looked. The numbers come from one counter for
all documents, so they are not consecutive, and a command that fails puts the number back with
the font.

## Clients

Any language with TCP sockets and JSON can drive the app. A client connects to
`127.0.0.1:<port>`, writes one request per line and reads one reply per line. Send a request,
then wait for its reply before sending the next. The clients below are complete; copy one and
build on it. Each expects the app started with `fontcraft --control 7985`.

`engine.execute` runs any command; `engine.commands` lists them with their parameters. For the
reference app's usual script tasks, [`porting.md`](porting.md) names the commands to use. An
agent that speaks MCP can use `fontcraft-cli mcp` instead ([`mcp.md`](mcp.md)); a script that
works on files without the app can use `fontcraft-cli run`.

### Python

Python 3, standard library only:

```python
import json
import socket


class FontCraft:
    def __init__(self, port=7985):
        self.sock = socket.create_connection(("127.0.0.1", port))
        self.file = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.next_id = 0

    def request(self, method, params=None):
        self.next_id += 1
        self.file.write(json.dumps({"id": self.next_id, "method": method, "params": params or {}}) + "\n")
        self.file.flush()
        reply = json.loads(self.file.readline())
        if not reply.get("ok"):
            raise RuntimeError(reply.get("error"))
        return reply.get("result")

    def run(self, command, **params):
        return self.request("engine.execute", {"command": command, "params": params})


fc = FontCraft()
fc.run("glyph.newGlyph", name="a.alt")
print(fc.run("spacing.getMetrics", glyph="a.alt"))
```

### JavaScript

Node.js, no packages (save as `fontcraft.mjs`):

```js
import net from "node:net";
import readline from "node:readline";

export function connect(port = 7985) {
  const socket = net.connect(port, "127.0.0.1");
  const waiting = new Map();
  let nextId = 0;
  readline.createInterface({ input: socket }).on("line", (line) => {
    const reply = JSON.parse(line);
    const pending = waiting.get(reply.id);
    waiting.delete(reply.id);
    if (pending) reply.ok ? pending.resolve(reply.result) : pending.reject(new Error(reply.error));
  });
  const request = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++nextId;
      waiting.set(id, { resolve, reject });
      socket.write(JSON.stringify({ id, method, params }) + "\n");
    });
  return {
    request,
    run: (command, params = {}) => request("engine.execute", { command, params }),
    close: () => socket.end(),
  };
}

const fc = connect();
await fc.run("glyph.newGlyph", { name: "a.alt" });
console.log(await fc.run("spacing.getMetrics", { glyph: "a.alt" }));
fc.close();
```

### Rust

The standard library and `serde_json`:

```rust
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use serde_json::{Value, json};

struct FontCraft {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next_id: u64,
}

impl FontCraft {
    fn connect(port: u16) -> std::io::Result<Self> {
        let writer = TcpStream::connect(("127.0.0.1", port))?;
        let reader = BufReader::new(writer.try_clone()?);
        Ok(Self { writer, reader, next_id: 0 })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let line = json!({ "id": self.next_id, "method": method, "params": params });
        writeln!(self.writer, "{line}").map_err(|e| e.to_string())?;
        let mut reply = String::new();
        self.reader.read_line(&mut reply).map_err(|e| e.to_string())?;
        let reply: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
        match reply.get("ok") {
            Some(Value::Bool(true)) => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
            _ => Err(reply.get("error").and_then(Value::as_str).unwrap_or("no reply").to_string()),
        }
    }

    fn run(&mut self, command: &str, params: Value) -> Result<Value, String> {
        self.request("engine.execute", json!({ "command": command, "params": params }))
    }
}

fn main() -> Result<(), String> {
    let mut fc = FontCraft::connect(7985).map_err(|e| e.to_string())?;
    fc.run("glyph.newGlyph", json!({ "name": "a.alt" }))?;
    println!("{}", fc.run("spacing.getMetrics", json!({ "glyph": "a.alt" }))?);
    Ok(())
}
```

### Shell

One request with `nc`, or an interactive session with `socat`:

```sh
printf '%s\n' '{"id": 1, "method": "engine.execute", "params": {"command": "glyph.newGlyph", "params": {"name": "a.alt"}}}' \
  | nc -w 2 127.0.0.1 7985

socat READLINE TCP:127.0.0.1:7985
```

### Events

The app keeps the last 1,000 events in a ring. Each has a sequence number `seq`, counted from 1 and never reused,
and a `kind`:

| `kind` | Fields | When |
|---|---|---|
| `command` | `command`, `ok`, `revision`, `error?` | a command ran, from any source: a request, a menu, a key, a panel. `revision` is the active document's after it. Read-only queries are left out |
| `document` | `action` (`opened`, `saved`, `closed`), `document`, `title`, `path` | a document was opened, saved (also under a new path) or closed |
| `dialog` | `action` (`opened`, `closed`), `dialog` | a form dialog opened or closed |
| `error` | `message` | a failure that was not a command run: a tool's input, a view change, a file chooser's answer |

Pass the reply's `last` as `since` next time to read only newer events. `dropped: true` says that events after
`since` have already left the ring. Error texts are cut at 2,000 bytes.

## Planned, in the order the milestones need them

The names and shapes follow the sibling apps so their agents' habits carry over.

| Method | Params | Milestone |
|---|---|---|
| `ui.click` beyond dialogs | `{element}` press any control by its id in the accessibility tree | M13 |
| `app.open` / `app.save` / `app.export` / `app.quit` | `{path}` … through the host (file dialogs, the browser's download) | M1 |

Check drawing with `ui.screenshot` on the running app, or with a headless frame test
(`crates/ui/src/testing.rs`). Without a display the app runs on a virtual one
(`docs/development.md`).

## Errors agents will see

| Error text begins | Meaning |
|---|---|
| `unknown command` | no such id: call `engine.commands` |
| `` `x` is not available: `` | the command is disabled now; the reason follows (`no document open`, `nothing to undo`) |
| `<command>: ` followed by a parameter name | a parameter is missing, of the wrong type or out of range |
| `not supported yet:` | the command exists but this part is a later milestone |
| `internal error (please report it)` | a bug. The document was put back as it was |
