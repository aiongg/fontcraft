# Writing modules

A module is a WebAssembly file that adds a filter, a reporter or a check to FontCraft. It gets
one layer as JSON and returns JSON. It runs in a sandbox: it cannot read files, use the network,
read the clock or draw interface, and every run has an instruction, memory and time budget.

Modules are one of two ways to extend FontCraft. The other is macros: JSON files of command
steps, described in [`macros.md`](macros.md). Anything that needs files, the network or a
language runtime runs outside the app and drives it over the control channel, MCP or the
command line ([`control-protocol.md`](control-protocol.md), [`mcp.md`](mcp.md)). Authors coming
from the reference app's Python API will find their usual tasks in [`porting.md`](porting.md).

[Decision 0005](decisions/0005-extensions.md) explains why extensions take this form.

## Quick start

1. Copy a template from [`examples/module-template/`](../examples/module-template/): `filter/`,
   `reporter/` or `check/`.
2. Edit `manifest.json` (your id, name and parameters) and the logic in `src/lib.rs`.
3. Build it:

   ```sh
   rustup target add wasm32-unknown-unknown
   cargo build --release --target wasm32-unknown-unknown
   ```

4. Install the `.wasm` file: drop it on the app window, open it with File › Open, or run
   `plugin.installFromFile {"path": "…/my_module.wasm"}`. No relaunch is needed.
5. Run it: a filter from the Filter menu or `plugin.runFilter`, a reporter from the View menu or
   `view.showReporter`, a check with `audit.run`.

## Kinds of module

| Kind | Input | Output | Where it appears | Command |
|---|---|---|---|---|
| `filter` | one layer, the parameters, the Edit View selection | the layer's new shapes, anchors and width, log lines | Filter menu (below a separator), a Palette section when it has parameters, the `Filter` and `PreFilter` export parameters, macros | `plugin.runFilter` |
| `reporter` | one layer, the parameters, the view scale, whether the glyph is being edited | drawing primitives behind and in front of the outline | a Show entry in the View menu, drawn on the canvas | `view.showReporter` |
| `check` | one layer, the parameters | findings with a name, severity, message and position | the audit's results and the canvas markers | `audit.run` |

A module never adds a command id. The menus list the installed modules as items that call these
commands with the module's id, so the list of commands stays the same whatever is installed.

## The ABI, version 1

A module exports a memory and four functions. It must import nothing: a module with any import
is refused when it loads.

| Export | Signature | What it does |
|---|---|---|
| `memory` | linear memory | where the host writes the input and reads the output |
| `fc_abi_version` | `() -> i32` | returns `1` |
| `fc_manifest` | `() -> i64` | returns the manifest JSON, packed (below) |
| `fc_alloc` | `(size: i32) -> i32` | returns the address of a free block of `size` bytes, or `0` when there is no memory |
| `fc_run` | `(input: i32, input_len: i32, params: i32, params_len: i32) -> i64` | runs once and returns the output JSON, packed; a negative value is an error code |

**Packed results.** An `i64` result holds the byte length of UTF-8 JSON in its high 32 bits and
its address in the module's memory in its low 32 bits: `(len << 32) | address`.

**What happens when.**

1. Loading: the host checks the size and the binary, refuses imports, calls `fc_abi_version`
   and `fc_manifest` once, and checks the manifest.
2. Each run: the host makes a fresh instance, calls `fc_alloc` twice and copies the input and
   the parameters into the two blocks, calls `fc_run` with their addresses and lengths, and
   reads the output it returns.
3. The instance is then thrown away. Nothing survives between runs, so a module need not free
   memory, and it cannot keep state from one run to the next.

**Errors.** A module that cannot do its work returns `{"error": "message"}` as its output; the
message reaches the user. A negative `fc_run` result, a trap (a Rust panic is one) or a broken
output also fails the run. In every case nothing changes in the font.

The Rust templates implement the ABI in `src/abi.rs`; copy that file unchanged.

## The manifest

`fc_manifest` returns a JSON object of at most 64 KiB:

```json
{
  "id": "org.example.shift",
  "name": "Shift Outline",
  "version": "0.1.0",
  "kind": "filter",
  "author": "Your Name",
  "description": "Moves the outline by a distance.",
  "params": {
    "dx": { "type": "number", "min": -1000, "max": 1000, "default": 10 },
    "mode": { "type": "choice", "options": ["all", "outline only"], "default": "all" }
  },
  "capabilities": []
}
```

| Key | Required | Rule |
|---|---|---|
| `id` | yes | 1 to 64 characters from `A-Z a-z 0-9 . _ -`; a reverse domain name is recommended. It names the module in commands, export parameters, macros and check ids, so keep it stable across versions |
| `name` | yes | 1 to 64 printable characters; the menu label |
| `kind` | yes | `filter`, `reporter` or `check` |
| `version` | no | at most 32 bytes; shown in the module list and compared by extension indexes |
| `author` | no | at most 128 bytes |
| `description` | no | at most 1,024 bytes; shown as the menu tooltip and in the module list |
| `params` | no | the parameters, in the order the form shows them (below) |
| `capabilities` | no | must be empty: ABI version 1 grants none |

### Parameters

`params` maps each parameter's name to its type. Names are 1 to 64 characters of letters, digits
and `_`, do not start with `_`, and are not `id`, `ids`, `params`, `preview`, `glyphs` or
`layers`. A manifest holds at most 64 parameters.

| `type` | Keys | The form shows | Example |
|---|---|---|---|
| `number` | `min`, `max`, `default` (all required, finite, `min ≤ default ≤ max`) | a number field | `{"type": "number", "min": 0, "max": 100, "default": 50}` |
| `int` | `min`, `max`, `default` (defaults `-1000000`, `1000000`, `0`; within ±2⁵³) | a whole-number field | `{"type": "int", "min": 1, "max": 10, "default": 2}` |
| `bool` | `default` (default `false`) | a checkbox | `{"type": "bool", "default": true}` |
| `choice` | `options` (1 to 64 strings of at most 64 bytes), `default` (one of them; the first when left out) | a pop-up menu | `{"type": "choice", "options": ["soft", "hard"]}` |

The module always receives a complete parameter object. The host fills in defaults for missing
parameters, clamps numbers into their range, rounds whole numbers and drops unknown keys. A value
of the wrong type (text for a number, a choice that is not an option) is refused before the
module runs. The Command Console, MCP and `plugin.info` show the parameters in the same short
form as command parameters: `{dx: -1000..1000 (10), mode: "all"|"outline only" (all)}`.

## The input

Every kind receives one JSON object per run:

| Key | Type | What it holds |
|---|---|---|
| `glyph` | string | the glyph's name |
| `unicodes` | [number] | the glyph's code points |
| `layer` | object | the layer in FontCraft's native layer form (below) |
| `metrics` | {kind: number} | the positions of the layer's master's metrics: `ascender`, `cap height`, `x-height`, `baseline`, `descender`, `italic angle`, and any other metric kind the font defines |
| `selection` | [{shape, node}] | filters run from Edit View: the selected nodes, as indexes into `layer.shapes` and that path's `nodes`. Left out when nothing is selected |
| `scale` | number | reporters: the view scale in screen pixels per font unit |
| `active` | bool | reporters: whether this glyph is the one being edited |

### The layer form

The layer is the same JSON the native `.fontcraft` format stores. Empty lists, `false` flags and
default values are left out, so read every key as optional.

```json
{
  "id": "m01",
  "master": "m01",
  "width": 600,
  "shapes": [
    { "path": { "nodes": [
        { "x": 100, "y": 0, "type": "line" },
        { "x": 100, "y": 300, "type": "offcurve" },
        { "x": 200, "y": 400, "type": "offcurve" },
        { "x": 300, "y": 400, "type": "curve", "smooth": true }
    ] } },
    { "component": { "glyph": "acutecomb", "transform": { "decomposed": {
        "pos": [150, 0], "scale": [1, 1], "angle": 0, "slant": [0, 0] } } } }
  ],
  "anchors": [ { "name": "top", "x": 300, "y": 700 } ]
}
```

| Item | Keys |
|---|---|
| shape | one of `{"path": …}`, `{"component": …}`, `{"image": …}`, `{"raw": …}`. Images and raw shapes (data FontCraft keeps as it was read) are best returned unchanged |
| path | `nodes`; `closed` (left out when `true`); `info` (lock, attributes, identifier) |
| node | `x`, `y`, `type` (`line`, `curve`, `qcurve`, `offcurve`); `smooth`, `locked`, `name` and others when set. Off-curve nodes come before the on-curve node that ends their segment; in a closed path the first segment wraps around from the last nodes |
| component | `glyph`; `transform`, either `{"decomposed": {"pos", "scale", "angle", "slant"}}` (angle and slant in degrees) or `{"matrix": [xx, yx, xy, yy, dx, dy]}`, and left out when it is the identity; `info` |
| anchor | `name`, `x`, `y`; `info` |
| layer | `id`, `master`, `width`, `shapes`, `anchors`; `info` holds the layer's name, guides, hints, background and other data |

Locked and hidden paths are passed to filters like the others, and a filter may change them. A
glyph that is locked is refused before any module runs.

## The output

Every kind may return `{"error": "message"}` instead of its output. The message is shown to the
user and nothing changes.

### Filters

```json
{ "shapes": [ … ], "anchors": [ … ], "width": 620, "log": ["moved 12 nodes"] }
```

| Key | Required | Meaning |
|---|---|---|
| `shapes` | yes | the layer's new shapes, in the layer form. They replace every shape of the layer, so return the shapes you do not change as you received them |
| `anchors` | no | the layer's new anchors; left out, the layer keeps its own |
| `width` | no | the new advance width; left out, the layer keeps its own |
| `log` | no | lines for the Command Console: at most 1,000 lines, each cut at 1,000 characters |

Other keys are ignored, so a filter may return the `layer` object it was given, changed. Every
value is checked against the model's limits (node and element counts, coordinates within
±1,000,000, finite numbers, name lengths) before anything changes. `plugin.runFilter` runs the
module on every target layer first and then applies all the results as one undo step; if one
layer fails, no layer changes.

`plugin.runFilter` with `selectionOnly: true` keeps the result only for the paths that hold a
selected node. The module must then return the layer's shapes in their order.

### Reporters

```json
{
  "background": [ { "type": "line", "from": [0, 500], "to": [600, 500], "stroke": { "color": [0, 0, 255, 90], "width": 1 } } ],
  "foreground": [ { "type": "circle", "center": [100, 0], "radius": 4, "fill": [0, 0, 255, 255] } ]
}
```

`background` is drawn behind the outline, `foreground` in front of it. Coordinates and radii
are font units in the layer's space. Stroke widths and text sizes are screen pixels from 0 to
1,000; a size that should look the same at every zoom is divided by `scale` when it becomes font
units. Colours are `[red, green, blue, alpha]`, each 0 to 255.

| `type` | Keys |
|---|---|
| `path` | `commands`: a list of `{"op": "moveTo", "to": [x, y]}`, `{"op": "lineTo", "to"}`, `{"op": "quadTo", "c", "to"}`, `{"op": "curveTo", "c1", "c2", "to"}`, `{"op": "close"}`; `fill` and/or `stroke` |
| `line` | `from`, `to`, `stroke` |
| `circle` | `center`, `radius`; `fill` and/or `stroke` |
| `text` | `at`, `text` (at most 1,000 characters), `color`, `size` |

A stroke is `{"color": [r, g, b, a], "width": pixels}`. A drawing holds at most 100,000 items,
where each primitive and each step of a path counts as one.

The app runs a reporter again only when the layer, the module, its parameters or the zoom
changes; between those it draws the stored result. A reporter's time budget is 5 seconds.

### Checks

```json
{ "findings": [ { "name": "openPath", "severity": "warning", "message": "path 0 is open", "position": [100, 0] } ] }
```

| Key | Rule |
|---|---|
| `name` | the problem's name, 1 to 64 characters from `A-Z a-z 0-9 . _ -`. The audit shows the finding under the check id `check.plugin.<module id>.<name>` |
| `severity` | `info`, `warning` or `error` |
| `message` | 1 to 1,000 characters |
| `position` | optional `[x, y]` in font units; the canvas marks it |

A check returns at most 10,000 findings per layer. Turned-on check modules run in `audit.run`
on the same layers as the built-in layer checks, under any profile that has layer checks; their
results are in the group `plugin` and have no automatic fix. A check module that fails is listed
once among the checks with status `error` and its message.

## Limits

A module that goes past a limit fails with a message naming the limit, and changes nothing.
`app.info` returns the current values under `limits`.

| What | Limit |
|---|---|
| Module file | 32 MiB |
| Manifest | 64 KiB; 64 parameters |
| Installed modules | 256 per session |
| Instructions per run | 50,000,000 plus 4,000 per byte of input |
| Memory | 512 MiB (256 MiB in the web app); the input and parameters may use half |
| Time per run | 60 s for filters and checks, 5 s for reporters (the desktop app and the command line; the web app has the instruction budget only) |
| Call depth | 1,024 nested calls |
| Output | 64 MiB of JSON |
| Filter output | the model's limits per layer; 1,000 log lines of 1,000 characters |
| Reporter output | 100,000 items; text 1,000 characters; widths and sizes 0 to 1,000 pixels |
| Check output | 10,000 findings; messages 1,000 characters |
| `Filter` export string | 4,096 bytes; 256 parts |

## Installing and running

### In the desktop app

- **Install**: drop the `.wasm` file on the window, open it with File › Open, or run
  `plugin.installFromFile`. The module is checked first (ABI version, imports, manifest, limits);
  a module that fails the check is not copied anywhere. It is then copied into the Modules folder
  and turned on, with no relaunch.
- **The Modules folder** is in the settings folder:

  | System | Folder |
  |---|---|
  | macOS | `~/Library/Application Support/FontCraft/Modules` |
  | Windows | `%APPDATA%\FontCraft\Modules` |
  | Linux and others | `$XDG_CONFIG_HOME/fontcraft/Modules` (`~/.config/fontcraft/Modules`) |

  `FONTCRAFT_SETTINGS_DIR` replaces the settings folder. The app checks the folder when it gains
  focus and every 5 seconds while it has focus. A module copied into the folder by hand is listed
  as new and stays off until you turn it on (`plugin.setEnabled`, or `plugin.installFromFile`
  with its file `name`).
- **Replace**: install a module with the same id and `replace: true`.
- **Remove**: `pluginManager.remove {"id"}`. The file moves to the Removed folder beside the
  Modules folder, so you can take it back by hand.
- **Safe mode**: start with `fontcraft --safe-mode`, or hold Alt and Shift while the window
  opens. Modules are listed but none runs; Help › Load Extensions (`app.loadExtensions`) turns
  them on for the session.

### From the command line, MCP and the control channel

A session without a Modules folder (the command line, headless MCP, the web app) keeps an
installed module for that session only. `plugin.installFromFile` also takes the file as
`dataBase64`, for clients that cannot share a path with the app.

```sh
fontcraft-cli run --in MyFont.glyphs \
  --cmd plugin.installFromFile --params '{"path": "shift.wasm"}' \
  --cmd plugin.runFilter --params '{"id": "org.example.shift", "glyphs": ["A", "B"], "params": {"dx": 20}}' \
  --save Shifted.glyphs
```

| Command | Use |
|---|---|
| `plugin.installFromFile` | install or replace a module (or a macro) from a path or base64 |
| `pluginManager.list` | the installed modules, the macros and what the extension indexes offer |
| `plugin.info` | one module's manifest, parameters, defaults, size, SHA-256, switch and last error |
| `plugin.setEnabled` | turn a module on or off |
| `pluginManager.remove` | uninstall a module |
| `plugin.runFilter` | run a filter on glyphs and layers, with parameters and an optional node selection |
| `view.showReporter` | show or hide a reporter (`{"id", "on": true}`), desktop app only |
| `audit.run` | run the checks, check modules included |
| `app.info` | the installed modules, the ones that failed to load, the folders, safe mode and the limits |
| `app.loadExtensions` | leave safe mode for the session |

### In export parameters

A font's `Filter` and `PreFilter` export parameters can name a filter module by its id or its
name: `org.example.shift; dx:20; include:A,B`. `key:value` parts set parameters by name; bare
values set them by position, in the manifest's order; `include:` and `exclude:` choose the
glyphs and are not passed to the module. A document that names a module keeps the name when the
module is not installed, and its export then fails with a message naming the module. The same
module gives the same outline from the Filter menu and at export.

### In macros

A macro that runs a module lists its id in `requires`; running the macro without that module
installed fails before any step runs, with a message naming the module. See
[`macros.md`](macros.md).

## Debugging

- **Test the logic natively.** Keep the module's work in a function from input bytes to output
  bytes, as the templates do, and call it from `cargo test` on your own machine with layer JSON
  you write by hand.
- **See a real input.** `layer.inspect` shows a layer's shapes and nodes; the native `.fontcraft`
  file holds layers in exactly the form a module receives.
- **Log lines** from a filter's `log` appear in the Command Console with the command that ran.
- **The last error** of each module is in `plugin.info` under `lastError`.

The error message says where the problem is:

| Message begins | Meaning |
|---|---|
| `not a valid module:` | not WebAssembly, malformed, too large, or the module imports something |
| `module ABI:` | an export is missing or has the wrong type, `fc_abi_version` is not 1, or an address points outside memory |
| `module manifest:` | the manifest is not JSON, too long, or breaks a rule above |
| `module trapped:` | the module hit a trap (a Rust panic, an out-of-bounds access, a division by zero) |
| `module exceeded its` | the instruction, memory, time, call-depth or output budget ran out |
| `module parameters:` | a parameter has the wrong type |
| `module output:` | the output is not the JSON its kind expects, or breaks a model limit |
| `module failed:` | the module returned `{"error": …}`, a negative code, or `fc_alloc` returned 0 |

## Other languages

Any language that compiles to WebAssembly without imports can produce a module: C, C++, Zig,
AssemblyScript, TinyGo and others, as long as the build

- targets bare WebAssembly (`wasm32-unknown-unknown`, `wasm32-freestanding` or the language's
  equivalent), not WASI, which imports functions from the host;
- exports the memory as `memory` and the four functions with the names and types above;
- keeps the input and the output in the module's own memory.

[`crates/plugins/src/wat.rs`](../crates/plugins/src/wat.rs) has small modules written by hand in
WebAssembly text: a filter that shifts every node, a reporter that draws the bounding box and a
check that flags every path. They show the ABI without any language's runtime.

## Versions

The host implements ABI version 1. A later host that changes the ABI will refuse modules whose
`fc_abi_version` it does not implement, with a message naming both versions. The manifest's
`version` is the module's own version; the host shows it and extension indexes compare it to
offer updates, but it does not affect loading.
