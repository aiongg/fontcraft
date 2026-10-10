# spec/: the feature inventory as data

Everything FontCraft is meant to do, as rows a tool can read. `cargo xtask spec` checks the files;
`cargo xtask parity` compares them with the live command registry and writes `docs/parity.md`;
`cargo xtask guesses` writes every row's `guesses` to `docs/guesses.md`.
Prose that explains *how* a feature behaves lives in `docs/spec/*.md`; rows here point at it.

| File | Holds |
|---|---|
| `areas.toml` | The parity areas and their weights |
| `milestones.toml` | The milestones, in build order |
| `inventory/*.toml` | The rows, one file per source chapter or tool family |

## Row kinds

All text is **our own wording**. Feature names, menu labels, shortcuts and option names may match
the reference; sentences may not be copied from any manual (see `docs/clean-room.md`).

Common fields on every row:

| Field | Meaning |
|---|---|
| `area` | An id from `areas.toml` |
| `milestone` | An id from `milestones.toml` |
| `summary` | One or two sentences of behaviour, in our own words |
| `ref` | Where the reference documents it: `"<Chapter> › <Section>"` or a tool/doc name |
| `status` | `handbook` (taken from the reference's public docs), `docs` (another public doc), `memory` (recalled, unverified), `inferred` (our design), `observed` (checked against the running reference: the app on our own synthetic fonts, the app in a public tutorial video, or a reference tool's output; `notes` names a video by title, channel, URL and time) |

### `[[command]]`: something a menu item, button, shortcut or agent runs

The example rows in this file show the schema. They are illustrations, not copies of inventory
rows: the inventory is the authority for every value.

```toml
[[command]]
id = "path.reverseContours"       # <menu or noun>.<camelCaseVerb>; ids follow the reference's menu structure
label = "Reverse Contours"
menu = ["Path"]                   # menu path in FontCraft; [] when it is not in a menu
shortcut = "Cmd+Ctrl+Alt+R"       # Mac notation, "" when none; the UI maps it per platform
kind = "engine"                   # engine (changes or reads the document) | ui (view and window state only)
params = "{glyphs?, layer?}"      # proposed JSON params for agents and the CLI; "{}" when none
area = "path-ops"
milestone = "M3"
summary = "Reverses the point order of the selected paths, or of every path on the layer when nothing is selected."
ref = "Edit View › Controlling Path Direction"
status = "handbook"
```

### `[[tool]]`: a pointer tool in the toolbar

```toml
[[tool]]
id = "draw"
label = "Draw"
shortcut = "P"
group = ""                         # the toolbar button it shares with other tools, "" when alone
gestures = [                       # one line each: "<input>: <effect>"
  "click: add a corner node",
  "drag: add a smooth node and pull out both handles",
  "Alt+drag: move only the outgoing handle",
]
area = "drawing"
milestone = "M3"
summary = "…"
ref = "Edit View › Draw Tool"
status = "handbook"
```

### `[[panel]]`: a panel, sidebar section, inspector, window or dialog

```toml
[[panel]]
id = "palette.layers"
label = "Layers"
location = "palette"               # palette | sidebar | inspector | window | dialog | toolbar | statusbar
controls = ["layer list with visibility toggles", "+ / − / gear buttons", "…"]
area = "layers"
milestone = "M3"
summary = "…"
ref = "Palette › Layers"
status = "handbook"
```

### `[[setting]]`: an application preference

```toml
[[setting]]
id = "appearance.handleSize"
label = "Handle Size"
section = "Appearance"
kind = "choice"                    # bool | choice | number | string | color | shortcut | path
values = "small | regular | large" # "" when free-form
area = "ui-chrome"
milestone = "M14"
summary = "…"
ref = "Settings › Appearance"
status = "handbook"
```

### `[[parameter]]`: a per-font, per-master or per-export parameter stored in the source

```toml
[[parameter]]
name = "glyphOrder"
scope = ["font"]                   # font | master | instance
kind = "list"                      # bool | number | string | list | dict | choice
area = "font-info"
milestone = "M7"
summary = "…"
ref = "Appendix › Custom Parameters"
status = "handbook"
```

### `[[behavior]]`: a rule that is not a command (modifier keys, display conventions, automatic behaviour, file-format rules)

```toml
[[behavior]]
id = "nodes.smoothStaysCollinear"
area = "drawing"
milestone = "M3"
summary = "A smooth node keeps its two handles on one line while either is dragged."
ref = "Edit View › Draw Tool"
status = "handbook"
```

### `[[cli]]`: a command-line subcommand of `fontcraft-cli`

```toml
[[cli]]
id = "dump"                        # the subcommand: fontcraft-cli dump …
usage = "fontcraft-cli dump FONT [--table TAG]... [--format text|json|xml]"
options = ["--table TAG: only these tables", "--format: output syntax"]
command = "font.dumpTables"        # the engine command it runs ("" when it is a pure shell around others)
equivalents = ["spot", "ttx -t", "tx -dump"]   # reference tools that cover the same job
area = "cli-tools"
milestone = "M12"
summary = "…"
ref = "AFDKO docs/AFDKO-Overview.md; fontTools ttx"
status = "docs"
```

### `[[format]]`: a file format we read or write

```toml
[[format]]
id = "ufo3"
label = "Unified Font Object 3"
extensions = ["ufo", "ufoz"]
read = true
write = true
spec = "https://unifiedfontobject.org/versions/ufo3/"
area = "import-export"
milestone = "M1"
summary = "…"
status = "docs"
```

### `[[check]]`: one quality check the auditor runs on a source or a compiled font

```toml
[[check]]
id = "check.outlines.overlap"      # check.<group>.<camelCaseName>
severity = "warn"                  # fail | warn | info
scope = "source"                   # source | binary | family
fixable = true                     # a command can repair it
area = "qa"
milestone = "M12"
summary = "…"
ref = "AFDKO checkoutlinesufo; fontbakery <check id>"
status = "docs"
```

### `[[table]]`: one OpenType table (or table group) the compiler writes and the tools read

```toml
[[table]]
tag = "glyf"
flavour = ["ttf", "variable-ttf"]  # which outputs carry it
read = "read-fonts"                # the crate we read it with, or "own"
write = "write-fonts"              # the crate we write it with, "own", or "none-yet" when the crate lacks it
area = "compile"
milestone = "M2"
summary = "What the compiler must put in it and from which source data."
ref = "OpenType spec › glyf; fontmake; AFDKO MakeOTFUserGuide.md"
status = "docs"
```

## Optional fields

| Field | On | Meaning |
|---|---|---|
| `origin` | any row | `"fontlab"`, `"robofont"`, `"fontforge"` or `"fontra"`: the feature comes from a secondary reference, not the primary one |
| `effect` | `[[parameter]]` | what kind of thing the parameter does: `table-value`, `export-filter`, `naming`, `interpolation`, `hinting`, `features`, `ui`, `other` |
| `ref` | `[[format]]` | where the format is described for our purposes, besides its `spec` URL |
| `notes` | any row | facts about the row that are not behaviour: why it is deferred, which tutorial video confirmed it (title, channel, URL, time in the video), which parts wait for a later milestone |
| `guesses` | any row | a list of sentences, one per choice of ours where no source says what the reference does: what we chose and, when useful, what would settle it (an observation, an oracle). Behaviour that follows the handbook but is not yet observed is not a guess. `cargo xtask guesses` lists them all in `docs/guesses.md` |
| `when` | `[[command]]`, `[[tool]]` | where the shortcut works (decision 0004): `editView`, `textMode`, `fontView`, `dialog`, `tool:<tool id>` or `panel:<panel id>`. Without it the shortcut is global. A tool's letter always works in Edit View |
| `context` | `[[command]]` | the context menus that show the command: `canvas` (empty canvas), `canvas.node`, `canvas.segment`, `canvas.path`, `canvas.guide`, `canvas.anchor`, `canvas.component`, `layers.row` (a row of the Palette's layer list), `toolbar` (a tool button) |

A `[[cli]]` id may hold a space for an action of a subcommand: `"tables list"` is `fontcraft-cli
tables list`.

## Rules the checker enforces

- Every file parses as TOML and holds only the row kinds above.
- `id`s are unique per kind across all files (`name` for parameters, `tag` for tables).
- `area` and `milestone` exist; `status` is one of the five values; `summary` is not empty.
- A command id uses `noun.camelCase` with ASCII letters, digits and dots only.
- A command's shortcut without `Cmd` or `Ctrl` (a plain key, or one with only `Shift` or `Alt`) has
  a `when`, so it never fires while a text field has focus. A `when` names a known scope, tool or
  panel.
- `context` lists only the context menus above; `notes` is a string; `guesses` is a non-empty list of
  sentences.
- `cargo xtask guesses`: a live command whose `status` is `inferred` or `memory` has at least one
  `guesses` entry.
