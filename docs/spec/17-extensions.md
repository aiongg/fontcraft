# Extensions: behaviour notes

Source: Glyphs 4 Handbook, chapter "Extensions" (Macro Panel shortcut from "Settings › Addons").
Inventory: `spec/inventory/17-extensions.toml`.

Each section first records what the reference offers, then our proposed equivalent. The proposals
are design, not reference facts; that is why every inventory row of this chapter has
`status = "inferred"`.

Our model in one paragraph: every action is a command in one registry, reachable from the UI, the
CLI, the JSON control channel and the MCP server. Code that must run inside the app is a sandboxed
WebAssembly module of one of two kinds, object filter or live reporter. There is no embedded Python.

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

FontCraft: an Extension Manager over a registry index of WebAssembly modules and command macros.
Install and remove take effect immediately because modules load at run time. Each entry shows the
capabilities the module asks for; an update that asks for more needs approval. `pluginManager.list`
gives agents the catalogue.

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
| Reporter | `.glyphsReporter` | draws extra information in Edit View; toggled by its Show … entry at the foot of the View menu | live-reporter module: read-only layer data in, drawing primitives out; registers a `view.show…` toggle |
| Filter | `.glyphsFilter` | processes glyph layers; in the Filter menu | object-filter module: layer objects and params in, replacements out; registers a `filter.*` command usable from menu, CLI, MCP and export strings |
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
and asks for approval of its capabilities. A file copied in by hand stays disabled until approved.
Deleting the file unregisters its commands.

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
- The module ABI is small and versioned: data and parameters in, results out. SDK templates for the
  two module kinds, Rust first.
- External interoperation is the main path (control channel, MCP), not an extra.
- No OSA dictionary; the CLI and control channel cover it on all platforms.
- Sandbox: no file, network or UI access by default; extra capabilities are declared in the
  manifest and approved by the user (`extensions.sandboxCapabilities`).

## Open questions

- Macro Panel: layout, run shortcut, tabs and whether its content persists; not described in these
  chapters.
- Exact label of the "re-run last script" item, and whether Reload Scripts replaces Open Scripts
  Folder in the menu while Option is held.
- Whether the Script menu shows anything besides scripts and the two folder commands.
- How script menu order is decided (alphabetical or by file name) and how name clashes are shown.
- Locations of the Scripts and Plugins folders on disk.
- How automatic extension updates are scheduled and whether they can be turned off per extension.
- What Remove does to plug-ins that were installed manually.
- What data a reporter receives and when it is redrawn; what a filter plug-in's dialog and export
  string contract look like. These live in the SDK documentation, which is the source to read
  before fixing the module ABI.
- Which of the reference's most used palette, tool and general plug-ins our two module kinds cannot
  express; that list decides whether more kinds are needed.
- Macro file format, and whether macros need variables or conditionals, is undecided.
