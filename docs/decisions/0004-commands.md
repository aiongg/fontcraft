# 0004: Commands

**Status:** accepted 2026-10-07; amended 2026-10-09 with the answers to open decisions D1 to D4 (the M3 plan).

## Context

"Everything is a command" is the sibling apps' rule. Writing the inventory showed where a font
editor strains it: the reference does much through direct manipulation with no menu item (info box
fields, list cells, context menus, drags); the same operation exists for a selection, a glyph,
many glyphs, all masters, and a font file on disk; shortcuts mean different things in different
tools; and undo in the reference is per glyph.

## Decision

1. **Explicit targets.** Every engine command can name what it acts on in its parameters. The UI
   passes its selection; the engine never reads UI state. (Inventory rows already propose these
   parameters; 176 command rows are our own design, mostly headless forms of pointer-only actions,
   among 375 rows marked `inferred` in all.)
2. **One command per action.** Variants by scope are parameters (`glyphs`, `allMasters`,
   `document: path`), not separate commands. The inventory still has scope variants as separate
   rows where the reference has separate menu items or modifier-key alternates; keep the ids the
   reference's menus imply as thin aliases of one implementation.
   For M3 (D2): `path.removeOverlap` is the implementation, and `transform.booleanUnion` is the
   same operation restricted to the targeted closed paths; `outlines.removeOverlaps` (M12) will be
   an alias with a `document` target. `path.transformations` is the implementation and
   `font.transform` (M12) an alias. Every "… for All Masters" command is an alias that sets
   `allMasters: true`.
3. **Atomic failure.** A command that returns an error changes nothing; the engine snapshots and
   restores. A batch (`command.batch`, planned for M1) is one transaction and one undo step.
4. **One history per document.** Simpler and always consistent. Every edit is undoable, global
   guides included, which differs from the reference on purpose (`undo.perGlyphHistory`). There is
   no per-glyph filter in M3 (D4); one can be layered on later over the same history if users
   miss it.
5. **Context menus and tool-local keys are still commands.** Two optional inventory fields say
   where a command is offered (D1, D3; schema in `spec/README.md`):
   - `when` names where a shortcut works: `editView`, `textMode`, `fontView`, `dialog`,
     `tool:<id>` or `panel:<id>`. A row without it has a global shortcut. A shortcut without
     `Cmd` or `Ctrl` (tool letters, `Return`, `Delete`, `Tab`, arrows, and their `Shift` and `Alt`
     forms) always has a `when`, so it never fires while a text field has focus; `cargo xtask
     spec` enforces this. Tool letters work in Edit View.
   - `context` lists the context menus that show the command: `canvas`, `canvas.node`,
     `canvas.segment`, `canvas.path`, `canvas.guide`, `canvas.anchor`, `canvas.component`,
     `layers.row`, `toolbar`. The shell builds context menus from the registry, as it builds the
     menu bar.
   - One keymap in the shell resolves a key: the focused text field or panel first, then the
     active tool, then Edit View or Font View, then the global shortcuts.
6. **Dialogs edit parameters.** A dialog is a form over a command's parameters with a live
   preview inside a rolled-back interaction. Agents skip the dialog and pass the parameters.
7. **Locks are engine rules.** A locked glyph refuses every command that would change it,
   whoever calls.
8. **Command ids, parameter names and result keys are an interface.** Saved actions, scripts and
   agents depend on them. Renames keep the old id as an alias (`brand-ok` style marker in code)
   for at least one major version.

## How commands name their targets

The shell knows what a command acts on and fills those parameters from its selection; agents
pass them directly. The addresses are positions in the layer's lists, so they are only valid for
the document revision they were read from (`layer.inspect` returns them):

| Parameter | Names |
|---|---|
| `glyph`, `layer?` | one layer: the glyph by name, the layer by id (default: the first master's layer) |
| `side?` | `foreground` (default) or `background`; every command that takes `glyph` takes it |
| `glyphs`, `layer?` or `layers?` | the same layer of several glyphs (Font View, batch commands) |
| `points` | nodes and handles as `[shape, node]` pairs: the shape's index in the layer, the node's index in the path's node list (off-curve points included) |
| `segments`, `segment` | a segment by the `[shape, node]` address of the on-curve node that ends it |
| `shapes` | paths and components by index |
| `anchors` | anchors by name |
| `guides` | `{scope, index}`, `scope` being `"local"` or `"global"`; global guides are indexed in the layer's master |
| `annotations` | annotations by index |

- A target list may be given only with one glyph. Without any target list a command acts on what
  its row says, usually the whole layer.
- An address that does not resolve is an error, and the command changes nothing. Every list is
  capped like `path.addPath`'s nodes.
- A command that changes the structure of a layer returns `selection` in the same shape, with the
  new indices; the shell replaces its selection with it. After undo or redo the shell drops the
  entries that no longer resolve.
- A command that creates or moves points rounds them to the font's grid (spacing ÷ subdivision;
  spacing 0 means no rounding). Tools pass unrounded coordinates, so a gesture and the command
  with the same coordinates give the same result.
- A drag is one interaction: `begin` takes a snapshot, `preview` runs the command on it without
  history, `commit` records one undo step and journals the command, `cancel` restores the
  snapshot. Dialog previews use the same mechanism (item 6).

## Consequences

- The `enabled` check of some commands depends on document state that is expensive to compute
  (TrueType instruction commands depend on which points earlier instructions touched). `enabled`
  may be optimistic; `run` is always authoritative.
- Window and view commands do not exist in a headless session. The headless CLI and MCP backend
  answer that the command needs the running app; attached to the app, they work.
