# 0005: Extensions and scripting

**Status:** accepted 2026-10-07, amended 2026-10-11 by the M13 plan (`docs/plans/M13.md` § 3).
Evidence: `docs/spec/17-extensions.md`.

## Context

The primary reference is extended with Python scripts and native plug-ins, and much professional
practice lives in them (kerning tools, interpolation checkers, proofing). FontCraft cannot run
those: they call the reference's own API. Embedding Python would break the Rust-only and
runs-in-the-browser rules and bring a large unsandboxed surface.

## Decision

- **The command registry is the scripting interface.** Anything a script would do is a sequence of
  commands: `fontcraft-cli run --script`, `command.batch`, a macro, or an agent over MCP. Anything
  that needs logic is an external program on the CLI, the control channel or MCP, in any language
  that can write JSON.
- **Macros are the saved form of a command sequence** (open decision D8). A macro is one JSON
  `.fcmacro` file: `format`, `version` (the file format's; a reader refuses a newer one), `title`,
  `description`, `args`, `requires` (module ids) and `steps` (command id and parameters). A value
  `{"$arg": "name"}` in a step's parameters takes an argument's value. There are no conditionals or
  loops. Command ids and parameter names follow decision 0004 item 8: a rename keeps an alias. A run
  resolves every step's id and placeholders before any step runs, then runs the steps as one
  transaction and one undo step. Macros also serve as action sets, and recording writes one.
- **Plug-ins are sandboxed WebAssembly modules**, with the manifest and parameter schema the
  sibling apps use, so one `.wasm` runs on every platform and in the browser. There are three
  kinds: filters (one layer and parameters in, the layer's new contents out; one undo step for
  every layer of a run), reporters (read-only layer data in, drawing primitives over Edit View out)
  and checks (one layer in, findings for the audit out). An export step is a filter module named in
  the `Filter` or `PreFilter` export parameter; there is no separate export kind.
- **Modules do not add command ids.** `plugin.runFilter`, `view.showReporter` and `audit.run` take
  a module id, and menus list installed modules as items that call these commands. The registry
  stays static.
- **No module runs interface code, reads files or opens the network** (ABI version 1 grants no
  capabilities). The reference's tool, general and file-format plug-ins, its plug-in special layers
  and its Objective-C and OSA interfaces have no counterpart, and their inventory rows are dropped.
- **The most-used third-party tools of the references become built-in features** where the
  secondary references show them built in (`docs/spec/90-secondary-references.md`): audit,
  kerning and spacing aids, interpolation checks, proof sheets.
- No compatibility layer for the reference's script API. If demand shows, a translation aid is a
  separate project.

## Costs

Users with a library of scripts for the reference must rewrite them. This is the largest barrier
to switching that the plan accepts, and the reason the built-in feature list is long.
