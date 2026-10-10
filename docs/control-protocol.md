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
| `ui.inspect` | | the UI state: `selected_glyphs`, `tabs` (each `{id, glyphs, edited, view: {scale, center}, selection}`), `active_tab` (an index into `tabs`; `null` while Font View shows), `editing` (the edited glyph's name), `closed_tabs`, `display` (the View toggles), `show_palette`, `canvas_size`, `status` |
| `ui.menu.list` | | the menu tree: `[{title, items: [{command, label, path, shortcut, enabled, disabled_reason?}]}]` |
| `ui.menu.invoke` | `{command, params?}` | runs a menu item; an id that is in no menu is an error |
| `ui.screenshot` | `{path?}` | captures the window as PNG, written to `path` or returned as base64 `png`; `{width, height, pixelsPerPoint, path?, png?}`. Sizes are in pixels. The reply comes after the next frame is drawn. Headless hosts (tests, MCP) answer with an error |
| `ui.render` | `{path?, glyphs?, text?, layer?, size?, scale?}` | draws glyphs filled by the nonzero rule, black on white, without a window; `{width, height, glyphs, path?, png?}`. `glyphs` names them, `text` maps characters to glyphs; neither: the edited glyph, else the Font View selection. `size` is pixels per em (default 256), `scale` pixels per font unit instead. `layer` draws that layer of the one glyph named. The same request gives the same bytes |
| `control.methods` | | this table |

`revision` in `document.inspect` grows with every change to a document, whoever made it. Compare
it to learn whether the font changed since you last looked.

## Planned, in the order the milestones need them

The names and shapes follow the sibling apps so their agents' habits carry over.

| Method | Params | Milestone |
|---|---|---|
| `ui.tool.select` | `{tool}` make a pointer tool active; runs the command `tool.select` and returns `{tool, previous}` | M3 |
| `ui.tool.list` | `{}` the pointer tools and the active one; runs the command `tool.list` and returns `{active, tools: [{id, label, shortcut, group}]}` | M3 |
| `ui.pointer` | `{events: [{kind: down \| drag \| up \| move \| doubleclick, x, y, space?: "font" \| "screen", mods?}]}` drive the active tool like the mouse | M3 |
| `ui.click` | `{id}` press the control with that stable id (`ui-design.md`) | M3 |
| `ui.key` / `ui.text` | `{key, shift?, alt?, cmd?, ctrl?}` / `{text}` | M3 |
| `ui.dialog.set` / `.confirm` / `.cancel` | `{field, value}` fill and submit the open dialog | M3 |
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
