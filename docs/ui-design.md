# UI design

What the interface is meant to look like and how it is built. Details of each panel are in the
`[[panel]]` rows and the notes they point to; this page is the frame they fit in.

## Whose look

Two things meet here and both are requirements:

- **Layout, vocabulary and interaction follow the primary reference**, so its users know where
  things are: one window per font, tabs along the top with Font View always first, Edit View tabs
  after it, the tools in a toolbar, an inspector area under the canvas, a sidebar on the right.
- **Visual language follows the sibling Crafting Apps**: their dark medium theme by default with
  four brightness levels, flat panels, small radii, the app bar doubling as the title bar on
  Windows and Linux, a hint line, a command palette. A user with VectorCraft open beside FontCraft
  should see one family of apps.

Nothing is traced or measured from a reference's pixels beyond layout proportions; icons and
cursors are original or openly licensed, generated in code where possible (`ASSETS.md`).

## The window

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ File  Edit  Glyph  Path  Filter  View  Script  Window  Help            (app bar / title) │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ [tools: select draw pencil primitives erase/knife zoom hand text annotate measure …]     │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ [Font]  [ A ]  [ n o ]  [ a b c … ]                                    (tabs)  [sidebar] │
├───────────────┬──────────────────────────────────────────────────────────┬───────────────┤
│ Font View:    │                                                          │ Palette       │
│ categories    │   Font View: glyph grid or list                          │  Dimensions   │
│ languages     │   — or —                                                 │  Fit Curve    │
│ filters       │   Edit View: canvas with text line, outlines, nodes      │  Layers       │
│               │                                                          │  Transform    │
│               ├──────────────────────────────────────────────────────────┤  …            │
│               │ info box: glyph, metrics, kerning, selection             │               │
│ [inspector]   │ preview area                                             │               │
├───────────────┴──────────────────────────────────────────────────────────┴───────────────┤
│ status / hint line                       master switcher · zoom · features · instance    │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

Separate windows: Font Info (tabs: Font, Masters, Exports, Features, Other Settings, Notes),
Kerning, Preview Panel, Text Preview, Glyph Info, Settings, the audit panel, the macro and
plug-in panels. In the browser these are in-window panels.

**Today** the shell draws the menu bar, the Font View grid, Edit View with outline, nodes and
metric lines, a Palette stub and the status line.

## Rules

- **The UI is thin.** A panel reads the session and acts through `app.run(id, params)`. If a
  control does something no command does, the command is missing: add it first.
- **State is data.** View state lives in `UiState` (serde), so the control channel can read it and
  tests can set it. Document state never lives in the UI.
- **Tokens only.** Colours, radii, sizes and fonts come from `theme::Tokens`. No colour literal in
  a panel.
- **Menus come from the registry.** A command with a menu path appears in its menu, with its
  shortcut and its enabled state and the reason when it is disabled.
- **Every control has a stable id** for the control channel (`ui.click {id}`, planned for M3): name
  widgets after the command or parameter they edit.
- **Edit View colours carry meaning** and are configurable (`[[setting]]` rows under Appearance):
  corner nodes, smooth nodes, handles, components, guides, metric lines, selection. Smooth nodes
  are round and corners square, so the difference never depends on colour alone.
- **Text is English in code** and translated at draw time from per-language catalogues. Names that
  are user data (glyphs, masters, layers, features) are never translated. Errors and status
  messages stay English: agents and tests read them.
- **The same UI runs in the browser.** No file-system or window calls below the app crates; the
  shell hands in bytes and takes bytes out.
- **Look at what you built.** Run the frame, capture it, read the picture. A panel is not done
  until someone has seen it.

## Platform conventions

The primary reference exists only on macOS, so these are ours to define, following the siblings:

- Shortcuts are stored in Mac notation and mapped: `Cmd` → `Ctrl` on Windows and Linux; where a
  shortcut already uses `Ctrl` with `Cmd`, the row gets a documented alternative.
- macOS: native menu bar, standard window chrome. Windows and Linux: no OS decorations, the app
  bar is the title bar with its own caption buttons.
- Mac-only conveniences of the reference (system text shaping for Text Preview, the emoji picker,
  test-installing a font, system services) need cross-platform stand-ins or are left out per
  platform (open decision D10).
- Pointer, pen pressure and touch gestures: as the sibling apps handle them.

## Measuring the look

Once the owner can run the references, the layout is measured the way the siblings do it:
screenshots of the reference on synthetic fonts, kept in the git-ignored `plan/local/`, compared
with ours for proportions (bar heights, cell sizes, spacing), never used as assets. The measured
values go into `Tokens` and the date into `ROADMAP.md`.
