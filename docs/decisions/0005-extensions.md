# 0005: Extensions and scripting

**Status:** accepted 2026-10-07. Evidence: `docs/spec/17-extensions.md`.

## Context

The primary reference is extended with Python scripts and native plug-ins, and much professional
practice lives in them (kerning tools, interpolation checkers, proofing). FontCraft cannot run
those: they call the reference's own API. Embedding Python would break the Rust-only and
runs-in-the-browser rules and bring a large unsandboxed surface.

## Decision

- **The command registry is the scripting interface.** Anything a script would do is a sequence of
  commands: `fontcraft-cli run --script`, `command.batch`, recorded actions, or an agent over MCP.
  Scripts are data (command ids and parameters), in any language that can write JSON.
- **Plug-ins are sandboxed WebAssembly modules**, with the manifest and parameter schema the
  sibling apps use, so one `.wasm` runs on every platform and in the browser. Kinds: filters
  (layers in, layers out, one undo step), reporters (draw over Edit View from read-only data),
  checks (results for the audit panel), export steps.
- **The most-used third-party tools of the references become built-in features** where the
  secondary references show them built in (`docs/spec/90-secondary-references.md`): audit,
  kerning and spacing aids, interpolation checks, proof sheets.
- No compatibility layer for the reference's script API. If demand shows, a translation aid is a
  separate project.

## Costs

Users with a library of scripts for the reference must rewrite them. This is the largest barrier
to switching that the plan accepts, and the reason the built-in feature list is long.
