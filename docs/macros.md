# Macros

A macro is a JSON file that lists command steps. Running it runs the steps in order, as one
undo step. Macros are FontCraft's scripts: they appear in the Script menu, run from the command
line, MCP and the control channel, and can be recorded from what you do in the app.

A macro has no conditions, loops or variables beyond its arguments. Work that needs logic runs
as a program outside the app that sends commands over the control channel or MCP
([`control-protocol.md`](control-protocol.md), [`mcp.md`](mcp.md)); work on one layer's outline
can be a filter module ([`plugins.md`](plugins.md)).

## A first macro

Save this as `Clean Up Outlines.fcmacro` in the Macros folder:

```json
{
  "format": "fontcraft.macro",
  "version": 1,
  "title": "Clean Up Outlines",
  "description": "Removes overlaps, corrects path directions, adds nodes at extremes\nand rounds coordinates, on the selected glyphs.",
  "steps": [
    { "command": "path.removeOverlap", "params": { "glyphs": { "$arg": "glyphs" } } },
    { "command": "path.correctPathDirection", "params": { "glyphs": { "$arg": "glyphs" } } },
    { "command": "path.addExtremes", "params": { "glyphs": { "$arg": "glyphs" } } },
    { "command": "path.roundCoordinates", "params": { "glyphs": { "$arg": "glyphs" } } }
  ]
}
```

It appears in the Script menu as "Clean Up Outlines", with the description as its tooltip.
Choosing it runs the four commands on the glyphs selected in Font View, or on the glyph being
edited. The same file runs from the command line, named by its path:

```sh
fontcraft-cli run --in MyFont.glyphs \
  --cmd script.run --params '{"macro": "Clean Up Outlines.fcmacro", "glyphs": ["a", "b", "c"]}' \
  --save MyFont.glyphs
```

## The file format

A macro file is UTF-8 JSON of at most 4 MiB, with the extension `.fcmacro`.

| Key | Required | What it holds |
|---|---|---|
| `format` | yes | `"fontcraft.macro"` |
| `version` | yes | `1`. A newer version is refused with a message, so an older FontCraft never runs a macro it might misread |
| `title` | no | the Script menu label; the file name when empty |
| `description` | no | the menu tooltip and the text agents read; may hold line breaks |
| `args` | no | the arguments the macro takes (below) |
| `requires` | no | ids of the modules the steps use. Running without one of them installed fails before any step runs, with a message naming it |
| `steps` | yes | `[{"command": id, "params": object}]`, run in order. `params` may be left out |

Each step is a command with the parameters it takes anywhere else. `commands.list` (or
`fontcraft-cli commands`) lists every command with its parameters.

These commands cannot be steps: `edit.undo`, `edit.redo`, `script.recordMacro`,
`script.rerunLast` and `script.repeatLastAction`. Undo and redo act on history outside the
macro's own undo step, and the others would make a macro depend on what ran before it.

## Arguments

`args` declares the values a run can pass in. A value `{"$arg": "name"}` anywhere in a step's
parameters is replaced by that argument before the step runs.

```json
{
  "format": "fontcraft.macro",
  "version": 1,
  "title": "Mark for Review",
  "description": "Gives the selected glyphs a colour label and a tag.",
  "args": {
    "color": { "type": "choice", "options": ["red", "orange", "yellow"], "default": "orange" },
    "tag": { "type": "string", "default": "review" }
  },
  "steps": [
    { "command": "glyph.setColorLabel", "params": { "glyphs": { "$arg": "glyphs" }, "color": { "$arg": "color" } } },
    { "command": "glyph.addTag", "params": { "glyphs": { "$arg": "glyphs" }, "tag": { "$arg": "tag" } } }
  ]
}
```

| `type` | Keys | Accepts |
|---|---|---|
| `number` | `min`, `max`, `default` (all optional) | a finite number in range |
| `int` | `min`, `max`, `default` | a whole number in range |
| `bool` | `default` | `true` or `false` |
| `choice` | `options`, `default` | one of the options |
| `string` | `default` | text of at most 4,096 bytes |

A declaration without `type` accepts any JSON value. An argument that a step uses must be given
or have a default; otherwise the run stops before any step, naming the argument and the step.
At most 64 arguments are declared.

Two arguments are built in and need no declaration:

| Argument | Filled with |
|---|---|
| `glyphs` | the glyph names selected in Font View, or the glyph being edited, when the macro runs from the menu; the `glyphs` parameter of `script.run` otherwise |
| `layers` | the `layers` parameter of `script.run`: layer ids, or `"active"`, `"visible"` or `"masters"` |

## Running a macro

`script.run` takes the macro in one of three forms:

| `macro` | Example |
|---|---|
| a macro of the Macros folder: its path there, with or without `.fcmacro`, or its title | `{"macro": "Cleanup/Clean Up Outlines"}` |
| the path of a `.fcmacro` file | `{"macro": "/work/macros/mark.fcmacro"}` |
| the macro itself, where `format` and `version` may be left out | `{"macro": {"steps": [{"command": "glyph.setTags", "params": {"glyphs": ["a"], "tags": ["done"]}}]}}` |

Its other parameters are `args` (an object of argument values), `glyphs`, `layers` and
`dryRun`. With `dryRun: true` it returns the steps with every argument filled in and runs
nothing.

What a run does:

1. It checks every step first: each command id exists and may be a step, and each placeholder
   names a declared or built-in argument with a value. An unknown command or argument changes
   nothing.
2. It runs the steps in order as one transaction. If a step fails, none of the steps takes
   effect, and the error names the failing step, counted from 1.
3. The changes are one undo step, labelled with the macro's title.

A macro may run another macro with a `script.run` step, up to 8 levels deep. A macro that would
run itself, directly or through others, is refused.

`script.rerunLast` runs the last macro again with the same arguments; `glyphs` and `layers` given
to it replace the ones of the last run. `script.repeatLastAction` does the same.

Action sets are macros: `script.runActionSet` runs one as `script.run` does, and
`script.saveActionSet` writes a list of steps as a macro file.

## Recording

`script.recordMacro {"action": "start"}` records every command that changes a font and succeeds,
from any surface: the menus, the Command Console, MCP, the control channel. Queries, interface
and view commands, undo and redo are left out.

`script.recordMacro {"action": "stop", "name": "My Steps"}` ends the recording and writes it as
`My Steps.fcmacro` in the Macros folder; without `name` it returns the macro without writing it.
`{"action": "cancel"}` drops the recording.

A recording keeps the parameters each command ran with, so a recorded macro repeats the same
edits on the same glyphs. To make it work on other glyphs, replace the glyph lists in its steps
with `{"$arg": "glyphs"}`.

A recording holds at most 100,000 steps and 64 MiB of step parameters. Past either limit it
stops taking steps, and `stop` reports `full: true` with a message; the steps it has are kept.

## The Macros folder

The Macros folder is beside the Modules folder in the settings folder:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/FontCraft/Macros` |
| Windows | `%APPDATA%\FontCraft\Macros` |
| Linux and others | `$XDG_CONFIG_HOME/fontcraft/Macros` (`~/.config/fontcraft/Macros`) |

`FONTCRAFT_SETTINGS_DIR` replaces the settings folder. `script.openScriptsFolder` creates the
folder when it is missing and shows it.

- The Script menu lists the folder: subfolders first, as submenus (up to 8 levels), then the
  macros by title, in natural order (`Step 2` before `Step 10`), ignoring case.
- A file that cannot be read appears greyed out; `script.reloadScripts` returns its error.
- The app checks the folder when it gains focus and every 5 seconds while it has focus.
  `script.reloadScripts` reads it again at once. No relaunch is needed.
- `plugin.installFromFile` with a `.fcmacro` file copies it into the folder.

## Modules in macros

A macro runs a filter module with a `plugin.runFilter` step, and names the module in `requires`:

```json
{
  "format": "fontcraft.macro",
  "version": 1,
  "title": "Shift Right",
  "description": "Moves the selected glyphs' outlines 20 units to the right.",
  "requires": ["org.example.shift"],
  "steps": [
    { "command": "plugin.runFilter", "params": { "id": "org.example.shift", "glyphs": { "$arg": "glyphs" }, "params": { "dx": 20, "dy": 0 } } }
  ]
}
```

## Renamed commands

When a command id or parameter name changes, the old name keeps working in macros: the run
resolves it to the new one. For example, a macro that names `collab.showServerInfo` runs
`app.info`.

## Limits

| What | Limit |
|---|---|
| Macro file | 4 MiB |
| Steps in one macro | 100,000 |
| Steps in one run, nested macros included | 1,000,000 |
| Macro files read in one run | 256 MiB |
| Nesting | 8 levels |
| Declared arguments | 64 |
| Placeholder depth in a step's parameters | 32 |
| Required modules | 256 |
| Text arguments, names and paths | 4,096 bytes |
| Recording | 100,000 steps; 64 MiB of parameters |

While the control channel or MCP confines file access to a root folder, macros are refused:
their steps cannot be checked against the root before they run.
