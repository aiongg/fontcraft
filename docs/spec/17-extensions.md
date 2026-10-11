# Extensions: behaviour notes

Source: Glyphs 4 Handbook, chapter "Extensions" (Macro Panel shortcut from "Settings › Addons").
Inventory: `spec/inventory/17-extensions.toml`.

Each section first records what the reference offers, then our proposed equivalent. The proposals
are design, not reference facts; that is why every inventory row of this chapter has
`status = "inferred"`.

Our model in one paragraph: every action is a command in one registry, reachable from the UI, the
CLI, the JSON control channel and the MCP server. Code that must run inside the app is a sandboxed
WebAssembly module of one of three kinds: filter, reporter or check (decision 0005). A script is a
macro: a data file of command steps, which is also FontLab's action set (open decision D8). There
is no embedded Python. The M13 plan (`docs/plans/M13.md`) gives the packages and § 3 there the
decisions; the module ABI follows the sibling apps' WebAssembly plug-in ABI.

## Reference: the extension families

| Family | What it is | Where it shows up |
|---|---|---|
| Scripts | Python files run on demand | Script menu |
| Plug-ins | bundles loaded at launch, six types | View menu, Filter menu, Palette, toolbar, open/export |
| Modules | libraries scripts and plug-ins depend on (including Python itself) | Plugin Manager › Modules |

Every script and the majority of plug-ins are Python; a Python version must be set in Settings ›
Addons.

## Plugin Manager

Reference (`window.pluginManager`, Window › Plugin Manager; no shortcut given):

- Two filter groups: All / Installed / Not Installed (`pluginManager.filterByInstallState`) and
  Plugins / Scripts / Modules (`pluginManager.filterByType`).
- Search field matching name, description and vendor (`pluginManager.search`).
- Per entry: green Install button (`pluginManager.install`) or Remove button
  (`pluginManager.remove`); preview image that enlarges on click (`pluginManager.enlargePreview`);
  grey website link (`pluginManager.openWebsite`); PayPal / Ko-fi donation icons where the vendor
  accepts donations (`pluginManager.openDonationPage`).
- Plug-ins and modules need a relaunch after install; scripts only need Script › Reload Scripts.
- Installed extensions update automatically.
- Content: main public repository plus alternate repositories from Settings › Addons, the latter
  visible only to users who registered them. New public entries are added by pull request.

FontCraft: an Extension Manager over extension indexes (JSON files listing modules and macros),
local and, if the owner allows network access, HTTPS (`docs/plans/M13.md` § 7 question 5).
Install and remove take effect immediately because modules load at run time. Each entry shows its
kind, size and checksum; updates are offered, and installed automatically only where the user
turned that on. `pluginManager.list` gives agents the catalogue.

## Scripts

Reference:

- Run from the Script menu (`script.run`); hovering an item shows its description after a moment.
- Re-run the last script: Cmd+Alt+R or the last menu item (`script.rerunLast`).
- The menu mirrors the Scripts folder; subfolders group scripts. Open the folder with Script › Open
  Scripts Folder, Cmd+Shift+Y (`script.openScriptsFolder`). Reload with Script › Reload Scripts,
  Cmd+Alt+Shift+Y, an item visible while Option is held (`script.reloadScripts`).
- A script is a `.py` file. First-line comment `# MenuTitle: Script Name` sets the menu title.
  `__doc__="Description"` below it sets the tooltip; a triple-quoted string may span lines.
- Scripts can be given shortcuts in Settings › Shortcuts.
- Some scripts need modules from the Modules tab.

FontCraft:

- A "script" splits into two things. A **macro** is a data file listing command invocations with
  parameters; it has a title and a description, lives in a macros folder mirrored by the Script
  menu (subfolders become submenus), is itself a command, runs as one undo group and stops at the
  first failing step. Anything needing logic is an **external program** in any language that calls
  the CLI, the control channel or MCP.
- Existing Python scripts for the reference do not load. We provide a porting table from common API
  tasks to command ids and an optional Python client for the control channel
  (`script.importExistingPython`).

## Macro Panel

Reference: Window › Macro Panel, Cmd+Alt+M (`window.macroPanel`). The chapters read here only say
that its console receives script and plug-in log output unless
`addons.useSystemConsoleForScriptOutput` redirects it.

FontCraft: a Command Console. Input is a command id with JSON params (or CLI syntax), with
completion and parameter docs from the registry; output is the command log and module log.

## Plug-in kinds

| Kind | Suffix | Reference behaviour | FontCraft |
|---|---|---|---|
| Reporter | `.glyphsReporter` | draws extra information in Edit View; toggled by its Show … entry at the foot of the View menu | reporter module: read-only layer data in, drawing primitives out; a Show entry per module (`view.showReporter`) |
| Filter | `.glyphsFilter` | processes glyph layers; in the Filter menu | filter module: one layer and params in, the layer's new contents out; run by `plugin.runFilter` from the Filter menu, the CLI, MCP, macros and the Filter export parameter |
| Palette | `.glyphsPalette` | adds a section to the Palette (Window › Palette, Cmd+Shift+P) | no module kind at first; a module may declare a parameter form that the app renders as a palette section |
| File Format | `.glyphsFileFormat` | adds formats for opening and exporting | no module kind at first; external converters through the CLI |
| Tool | `.glyphsTool` | adds a toolbar tool for Edit View | not supported in the sandbox; tools are contributed to the app |
| General | `.glyphsPlugin` | anything else | no catch-all; macros, external programs, or a new narrow module kind |

Rows: `plugin.kind.*`.

## Installing and removing plug-ins

Reference:

- One click in the Plugin Manager; loaded at launch, so relaunch afterwards.
- Manual install: drop the plug-in on the app's Dock icon (`plugin.installFromFile`); the app moves
  it into the Plugins folder, which sits next to the Scripts folder.
- Do not place files in the Plugins folder by hand; it conflicts with the system's security checks.
- Remove in the Plugin Manager, or delete the file from the Plugins folder.
- Plug-ins may need modules installed separately.

FontCraft: a module is one self-contained `.wasm` file with a manifest, kept in a modules folder
next to the macros folder. Dropping a file or calling `plugin.installFromFile {path}` validates it
(manifest, ABI version, no imports, limits) and copies it into the folder. A file copied in by
hand is listed as new and stays off until the user turns it on. Deleting the file removes its menu
entries.

## Authoring surface

Reference:

- Plug-ins in Python or Objective-C from the open-source SDK templates (`sdk.templates`).
- Python API and Objective-C API, each with its own reference documentation (`api.python`,
  `api.objectiveC`). The chapter names them but lists no API members.
- The SDK has means for an external application to interoperate with the app (`api.externalInterop`).
- Open Scripting Architecture support: AppleScript or JavaScript automation
  (`api.openScriptingArchitecture`).
- Learning material: API docs, tutorials, existing scripts, forum (`extensions.authoringDocs`).

FontCraft:

- The scripting API is the command registry; its reference is generated from command metadata.
- The module ABI is small and versioned: data and parameters in, results out. Module templates for the
  three kinds (filter, reporter, check), Rust first.
- External interoperation is the main path (control channel, MCP), not an extra.
- No OSA dictionary; the CLI and control channel cover it on all platforms.
- Sandbox: a module imports nothing (no file, network, clock or interface access), with an
  instruction budget, a memory cap and a time limit; ABI version 1 grants no capabilities
  (`extensions.sandboxCapabilities`).

## Open questions

Answered by the M13 planning pass, each with its source; the steps to settle the rest in the
reference app are in `docs/plans/M13.md` § 8.

- **Macro Panel layout, run shortcut, tabs, persistence.** Open: not in the handbook. FontCraft's
  Command Console is a one-line input with history (`window.macroPanel` guesses). § 8 checks it.
- **Label of the re-run item; Reload Scripts and the Option key.** Open: the handbook gives the
  shortcut and the place (foot of the Script menu) but no label. Our label is a guess
  (`script.rerunLast`); § 8 checks both.
- **What else the Script menu shows.** Answered for FontCraft (ours): the macros, then Re-run Last
  Script, Record Macro, Open Scripts Folder and Reload Scripts. The reference's menu is checked in § 8.
- **Script menu order and name clashes.** Ours: by title, case-insensitive, numbers in numeric
  order, folders first (`script.menu.mirrorsFolder` guess); § 8 checks the reference.
- **Folder locations.** Ours: `Macros` and `Modules` in the per-user application data folder
  (`plugin.pluginsFolder` guess). § 8 notes the reference's path.
- **Automatic updates.** Ours: offered, and installed automatically only where the user turned it
  on per index (`pluginManager.automaticUpdates` guess). § 8 looks for a switch in the reference.
- **What Remove does to manually installed plug-ins.** Open; § 8 checks it. FontCraft lists every
  module in its folder, however it got there, and Remove works on all of them.
- **What a reporter receives and when it is redrawn; a filter's dialog and export-string
  contract.** Answered from the reference's SDK documentation (GlyphsSDK, Python Templates ›
  Reporter and › Filter, Apache-2.0): a reporter draws for one layer at a time, in front of and
  behind the active glyph and the inactive glyphs, whenever the view redraws; a filter is called
  once per layer, from Edit View (with the selection), from Font View (once per selected glyph's
  layer) or at export through the `Filter` parameter, whose string is the plug-in name, optional
  `include:` and `exclude:` lists and `key:value` or bare values passed to the plug-in. FontCraft's
  ABI keeps that shape: reporter output is cached per layer revision and scale; filter export
  strings resolve to modules by name or id (`filter.customParameter.thirdParty`).
- **Which palette, tool and general plug-ins our kinds cannot express.** Answered (ours): tools
  and general plug-ins, which run interface code; file-format plug-ins. The M13 plan proposes
  dropping those rows (§ 7 question 2). Palette needs are met by parameter forms of filter modules.
- **Macro file format, variables, conditionals.** Answered (D8, `docs/plans/M13.md` § 3): JSON
  `.fcmacro` files with a format marker and version, title, description, declared arguments,
  `requires` and steps; placeholders fill arguments; no conditionals or loops, which belong in an
  external program.
