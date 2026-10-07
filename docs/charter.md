# FontCraft charter

What FontCraft is, what it is for, and the rules it is built under. Everything else in `docs/`
elaborates this page; where another page disagrees with it, this page wins until it is changed.

## The idea

A professional font editor and a professional font command-line tool suite, as one open-source
project: free to use, MIT OR Apache-2.0, written from scratch in Rust, running on macOS, Windows,
Linux and the web. A type designer who knows the leading commercial editor should be able to sit
down and work; a font engineer who lives in the command line should find every tool they rely on;
and an AI agent should be able to do anything either of them can, through the same commands.

It follows the pattern of the ArtCraft Crafting Apps (VectorCraft, PhotoCraft, DesignCraft and
their siblings): a clean-room reimplementation of an existing professional workflow, with a thin UI
over an engine in which every action is a command.

## Goals

1. **Parity with the professional tools.** Every menu item, tool, panel, dialog, setting, custom
   parameter and command-line tool of the references has a row in `spec/inventory/` and a
   behaviour note in `docs/spec/`. Parity is measured against that inventory, by behaviour.
2. **Agent-native.** Every action is a command with an id, a parameter description and a JSON
   result, reachable from the UI, the CLI, a JSON control channel and an MCP server. Nothing a
   person can do requires a pointer.
3. **Correct output, checked against oracles.** A font is an exact artefact. FontCraft's compiled
   fonts are compared table by table with what the reference compilers produce from the same
   source; every difference is either fixed or documented with its reason.
4. **Lossless interchange.** `.glyphs`, UFO and designspace sources open and save without losing
   data FontCraft does not understand. A family can move between FontCraft and the tools a studio
   already uses, in both directions.
5. **Never lose work.** No panics in shipped code, transactions around every command, atomic
   saves, recovery after a crash.
6. **Fit the family.** Same architecture, repository layout, gates, look and vocabulary as the
   Crafting Apps, so the project could join them without rework, and so their agents and
   contributors find their way here at once.

## Not goals

- Compatibility with a reference app's plug-ins or its Python scripting API. FontCraft's
  extension surface is its command registry plus sandboxed WebAssembly plug-ins (`docs/spec/17-extensions.md`).
- Reading undocumented proprietary formats. We read what has a public description.
- Cloud services, accounts, telemetry.
- Shipping any reference app's assets: glyph data, sample strings, icons, example fonts.

## References and what each is for

Decided 2026-10-07 ([decision 0001](decisions/0001-references.md)); the full index with licences is
[`references.md`](references.md).

| Reference | Licence | Role |
|---|---|---|
| Glyphs 4 | proprietary, Mac | **Primary for the editor**: layout, menus, tools, panels, shortcuts, data model. Command ids follow its menus. Known through its public handbook and file-format documentation |
| AFDKO | Apache-2.0 | **Primary for the command line** and the feature-file language. Its code may be read |
| fonttools, fontmake, fontc | MIT / Apache-2.0 | The compile pipeline's oracles; more command-line tools; fontc and its crates are also possible dependencies ([decision 0002](decisions/0002-compile-pipeline.md)) |
| FontLab 8 | proprietary, Mac and Windows | Secondary: features the primary lacks or leaves to plug-ins; Windows conventions |
| RoboFont | proprietary, Mac | Secondary: the UFO-native workflow |
| FontForge, Fontra | GPL-3.0 (Fontra verified; FontForge recalled, see `references.md`) | Public user documentation only, never source: the long tail of legacy and bitmap formats; collaborative editing |

## Principles

These are the short forms. `AGENTS.md` states them as rules; `architecture.md` shows how the code
holds them.

- **Clean-room.** Behaviour from public documentation and black-box observation; no copied code,
  no copied prose, no copied assets ([`clean-room.md`](clean-room.md)).
- **Everything is a command**, and commands take explicit targets.
- **Spec first.** The inventory row exists before or with the code.
- **Never crash.**
- **The UI is thin**, and the same UI runs natively and in the browser.
- **Rust only.**
- **Honest numbers.** Scores are graded by behaviour and err low. A row that rests on memory or
  inference says so.
- **Dependencies are welcome where they are permissive, build for the web and do the job**, as in
  the sibling apps, which use the same font crates. "From scratch" means no copied code, not no crates.

## Name

**FontCraft** is the working name: it follows the Crafting Apps' `{Function}Craft` pattern
(PascalCase in user-facing text; lowercase `fontcraft` for crates, binaries, file names and ids).
Two unrelated projects already use the name (a browser handwriting-to-font app and a GitHub
project; both seen in a web search on 2026-10-07), so it must be cleared with the ArtCraft team and checked for trademarks before any
release. **GlyphCraft** is the fallback. The sibling apps have been renamed after launch and kept
old file extensions working; if FontCraft is renamed, `.fontcraft` files must keep opening.

Until that is settled: the native extension is `.fontcraft`, the format name inside files is
`fontcraft`, the conventional control port is 7985 (what `fontcraft-cli mcp` tries; the app
listens only when started with `--control PORT`), and no application id is set yet.

## Relationship to ArtCraft

Independent for now. We keep to their conventions as read from the public VectorCraft and
PhotoCraft repositories on 2026-10-07:

- dual licence MIT OR Apache-2.0; Rust only; egui on wgpu; the web build is the same code as WebAssembly;
- the crate layering and its `cargo xtask layers` check; the command registry shape; the control
  protocol's request and reply shape; the MCP server with a headless and an attached mode;
- the no-panic lints, the panic guard, the command sweep; the asset register and its check; the
  vendor-names check; the honest-assessment roadmap with a weighted parity table;
- shared standards live in their `craftrules` repository, **which we could not read** (it needs
  access). Everything here that mirrors a craftrules standard was inferred from how VectorCraft
  and PhotoCraft quote it. When access is granted, check this repository against it
  ([`00-open-decisions.md`](spec/00-open-decisions.md), O3).

The Crafting Apps do not share code between repositories; they share written standards. FontCraft
does the same: nothing here is copied from them.

## What "done" means for a feature

1. Its inventory rows exist and say what it does.
2. It is a command (or a tool that emits commands) with tests, including a refusal test for bad
   parameters. The command sweep passes.
3. Its menu item, shortcut and panel controls reach it through `app.run`.
4. Where it produces font data, an oracle test compares the result with a reference tool.
5. `ROADMAP.md` and `docs/parity.md` are updated in the same change.

## Decisions that are the owner's

Agents decide everything else and record it ([`execution-plan.md`](execution-plan.md) § 7).

- The name, the repository's home and the application id.
- Joining the ArtCraft organisation, and anything said to them on the project's behalf.
- Adding a dependency whose licence is not MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0 or Unicode.
- Relaxing a non-negotiable in `AGENTS.md`.
- Anything that needs a licence to a reference app, or access to one's files.
