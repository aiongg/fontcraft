# 0004: Commands

**Status:** accepted 2026-10-07.

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
3. **Atomic failure.** A command that returns an error changes nothing; the engine snapshots and
   restores. A batch (`command.batch`, planned for M1) is one transaction and one undo step.
4. **One history per document.** Simpler and always consistent. Per-glyph undo in Edit View can be
   layered on later as a filter over the same history if users miss it (open decision D4).
5. **Context menus and tool-local keys are still commands.** The inventory gains optional fields
   `context` (which context menu shows the command) and `when` (the tool or panel a shortcut is
   scoped to) as the features need them; until a row has `when`, its shortcut is global.
6. **Dialogs edit parameters.** A dialog is a form over a command's parameters with a live
   preview inside a rolled-back interaction. Agents skip the dialog and pass the parameters.
7. **Locks are engine rules.** A locked glyph refuses every command that would change it,
   whoever calls.
8. **Command ids, parameter names and result keys are an interface.** Saved actions, scripts and
   agents depend on them. Renames keep the old id as an alias (`brand-ok` style marker in code)
   for at least one major version.

## Consequences

- The `enabled` check of some commands depends on document state that is expensive to compute
  (TrueType instruction commands depend on which points earlier instructions touched). `enabled`
  may be optimistic; `run` is always authoritative.
- Window and view commands do not exist in a headless session. The headless CLI and MCP backend
  answer that the command needs the running app; attached to the app, they work.
