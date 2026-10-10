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

Transport: `apps/fontcraft/src/control_server.rs`. Dispatch: `crates/ui-egui/src/control.rs`.
`ui.screenshot` needs the window, so the dispatch hands it back to the transport, which asks eframe
for the next frame and replies when the frame arrives.

## Methods today

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `{command, params?}` | runs any engine or UI command; the command's own result |
| `engine.commands` | | `{commands: [{id, label, menu, shortcut, params, query, enabled, disabled_reason?}]}`; UI commands carry `ui: true` |
| `document.inspect` | | `{documents: [{index, active, title, path, modified, revision, familyName, glyphs, masters, undo, redo}], active}` |
| `ui.inspect` | | the UI state: `selected_glyphs`, `tabs` (each `{id, glyphs, edited, view: {scale, center}, selection, focus, selection_glyph}`: `selection` and the path `focus` belong to the glyph `selection_glyph`), `active_tab` (an index into `tabs`; `null` while Font View shows), `editing` (the edited glyph's name), `closed_tabs`, `display` (the View toggles), `show_palette`, `canvas_size`, `status` |
| `ui.menu.list` | | the menu tree: `[{title, items: [{command, label, path, shortcut, enabled, disabled_reason?}]}]` |
| `ui.menu.invoke` | `{command, params?}` | runs a menu item as a click on it does: the follow-up question its result asks (a pasted outline outside the glyph, imported files without a glyph) opens as a dialog; an id that is in no menu is an error |
| `ui.screenshot` | `{path?}` | captures the window as PNG, written to `path` or returned as base64 `png`; `{width, height, pixelsPerPoint, path?, png?}`. Sizes are in pixels. The reply comes after the next frame is drawn. Headless hosts (tests, MCP) answer with an error |
| `ui.render` | `{path?, glyphs?, text?, layer?, size?, scale?}` | draws glyphs filled by the nonzero rule, black on white, without a window; `{width, height, glyphs, path?, png?}`. `glyphs` names them, `text` maps characters to glyphs; neither: the edited glyph, else the Font View selection. `size` is pixels per em (default 256), `scale` pixels per font unit instead. `layer` draws that layer of the one glyph named. The same request gives the same bytes |
| `ui.tool.select` | `{tool}` | makes a pointer tool active (the command `tool.select`): `{tool, previous}`. A gesture in progress is cancelled |
| `ui.tool.list` | | the pointer tools (the command `tool.list`): `{active, tools: [{id, label, shortcut, group}]}` |
| `ui.pointer` | `{events: [{kind, x, y, space?, mods?}]}` | drives the active tool like the mouse; see below |
| `ui.key` | `{key, shift?, alt?, cmd?, ctrl?, action?}` | a key as the keyboard gives it; see below |
| `ui.text` | `{text}` | types into the focused text field at the next frame (`{field: true}`); with none focused, each character is a `ui.key` tap (`{keys, handled}`) |
| `ui.dialog.inspect` | | the open dialog: `{open, dialog, title, command, mode: "preview" \| "runOnOk", message, tabs, fields: [{id, label, kind, value, focused, min?, max?, choices?, labels?, default, doc}], editing, params, error, actions, buttons, captions: {ok, cancel}, canChooseFolder}`; `{open: false}` when none is. `tabs` (`[{id, tab, label, active}]`) and `actions` (`[{id, label, command}]`) are `null` when the dialog has none |
| `ui.dialog.set` | `{field, value}` | gives a field of the open dialog a value and previews it; a number field also takes text. The `ui.dialog.inspect` reply |
| `ui.dialog.confirm` | | OK: the preview becomes one undo step, `{confirmed, changed, command, params}`; in a `runOnOk` dialog the command runs once, `{confirmed, command, params, result}`, and a failure keeps the dialog open |
| `ui.dialog.cancel` | | Cancel: the font as it was when the dialog opened; `{cancelled}` |
| `ui.click` | `{id}` | presses a control of the open dialog: `ok`, `cancel`, a tab by its dialog id (shows that tab), an action by its id (runs it: `{action, command, text, result}`; `text` also goes to the system clipboard), or a field by its key (a checkbox toggles; a number field takes the keyboard, its text selected, for `ui.text` and `ui.key`; a folder field opens the folder chooser, or without one takes the keyboard) |
| `control.methods` | | this table |

`ui.inspect` also names the active pointer tool (`tool`), the running interaction
(`interaction`, the label of a drag or an open dialog, else `null`) and the open dialog's id
(`dialog`, else `null`).

### Pointer and keys

`ui.pointer` runs its events in order through the same path as the mouse: the active tool turns
them into commands, a drag is one interaction (one undo step), and the tab's selection follows.
Every event is checked before the first one runs.

| Field | Values |
|---|---|
| `kind` | `down`, `drag`, `up`, `move` (hover), `doubleclick` (send it after the two clicks' `down` and `up`) |
| `x`, `y` | the position |
| `space` | `font` (default): font units of the edited glyph. `screen`: window points, as the mouse reports them |
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
took the key), and `command` and `result` (or `error`) when a menu shortcut ran.

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
| `bool` | `true` or `false` |
| `choice` | one word of `choices`; `labels` is what the menu shows |
| `multi` | a list of words of `choices` (any order; kept in the list's order) |
| `folder` | a folder path; `""` is none, and the parameter is left out |
| `order` | every row index of `labels` once, the first row on top (`dialog.shapeOrder`: the first is the bottom shape) |

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
it to learn whether the font changed since you last looked.

## Planned, in the order the milestones need them

The names and shapes follow the sibling apps so their agents' habits carry over.

| Method | Params | Milestone |
|---|---|---|
| `ui.click` beyond dialogs | `{id}` press any control with a stable id (`ui-design.md`) | M3 |
| `ui.set` | `{panel?, theme?, …}` | M3 |
| `ui.resize` / `ui.focus` | | M3 |
| `app.open` / `app.save` / `app.export` / `app.quit` | `{path}` … through the host (file dialogs, the browser's download) | M1 |

Check drawing with `ui.screenshot` on the running app, or with a headless frame test
(`crates/ui-egui/src/tests.rs`). Without a display the app runs on a virtual one
(`docs/development.md`).

## Errors agents will see

| Error text begins | Meaning |
|---|---|
| `unknown command` | no such id: call `engine.commands` |
| `` `x` is not available: `` | the command is disabled now; the reason follows (`no document open`, `nothing to undo`) |
| `<command>: ` followed by a parameter name | a parameter is missing, of the wrong type or out of range |
| `not supported yet:` | the command exists but this part is a later milestone |
| `internal error (please report it)` | a bug. The document was put back as it was |
