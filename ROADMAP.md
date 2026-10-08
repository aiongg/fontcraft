# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-07 (planning complete; M0 skeleton built, reviewed and passing its gate)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,618 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, guard, CLI, control channel, MCP, UI shell, tests | **~35%** of what the siblings' "infrastructure" row covers |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~3%** |
| Commands that exist (not: that behave like the reference) | **27 of 919** (`docs/parity.md`) |
| Observed against the running reference app | **nothing** |

## Honest assessment (2026-10-07)

**In one line:** the project is specified and has a working skeleton; it is not yet a font editor.

**What is real.** A new font can be created, glyphs added, removed and encoded, outlines added
from node lists, widths and font info set, everything undone and redone, the document saved to
and opened from its own format, and a static TrueType font exported that fonttools and skrifa
read back correctly. All of that works headless (`fontcraft-cli run`), over MCP, over the control
channel into the running window. From the window's menus only the commands that need no
parameters work (New, Close, Undo, Redo, New Glyph, the View and Window toggles): there are no
dialogs yet. Font View and Edit View draw. 78 tests pass, including a sweep that runs every engine
command with hostile parameters. The whole gate
(`cargo xtask ci`: format, lints, tests, spec, parity, assets, vendor names, layering, wasm) is green.

**What is not.** There is no drawing with the pointer, no selection of nodes, no path operations,
no kerning, no components beyond flattening at export, no features, no variable fonts, no CFF, no
hinting, no `.glyphs` or UFO support, no opening of compiled fonts, no screenshots over the control
channel, no web app, no packaging. Edit View shows a glyph; it does not edit one.

**How much to trust the inventory.** It was compiled from public documentation in one day, each
source read once. 2,113 rows come from the primary reference's handbook, 1,097 from other public
documentation or permissive source, 375 are our own design, and 33 rest on recollection. **None
has been checked against the running reference app.** An independent re-reading of 209 sampled
handbook rows against the handbook found 1 wrong and 14 imprecise (`docs/spec/README.md`). The notes list about 440 open questions the
documentation does not answer. Treat rows as a careful first reading, and each milestone's rows as
due a planning pass before they are built (`docs/execution-plan.md` § 3).

### By dimension

| Dimension | Estimate | Evidence and what is missing |
|---|---|---|
| **Breadth:** commands, tools and panels exist | ~3% | 27 of 919 commands; 0 of 33 tools; 2 of 170 panels as stubs |
| **Depth:** features behave like the reference | ~1–2% | only the M0 slice has behaviour, and none of it has been compared with the reference |
| **File interop** | ~20% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage` in and out (every reference-written corpus source saves back byte for byte); compiled TrueType, CFF, collections and WOFF 1 open (all 342 corpus fonts); static TrueType out. No UFO, designspace, OTF/CFF export, variable, WOFF2 |
| **Compiler correctness** | unmeasured | 10 of 50 tables written, for one static flavour; read back by two independent readers; one code review's findings fixed (bounds, names, style linking); **no oracle comparison yet** (the harness is M2) |
| **Agent automation** | ~40% | every existing command is reachable from CLI, control channel and MCP; no pointer injection, dialogs, screenshots, resources or prompts yet |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests); one dependency panic found and fenced; one adversarial code review, most findings fixed and the rest listed in `docs/status.md`. No fuzzing, no corpus runs, and the known scaling limits in `docs/status.md` |
| **Performance** | unmeasured | targets written down (`docs/architecture.md`); no budget suite |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: libraries check, no web app |
| **Look and feel** | not started | a functional shell with the siblings' default dark tokens; nothing measured against any reference |

### What to build next (in priority order)

1. **M1, the source model and the formats**: grow the model to the full outline of
   `docs/spec/30-source-formats.md`; the `plist` crate; `.glyphs` 2/3/4 and package reading and
   writing with exact round trips; UFO and designspace through norad; opening compiled fonts. This
   unlocks every oracle and every real test font. Start here.
2. **M2, the compile core and the oracle harness**: the remaining static tables, the `cff` crate,
   `fontcraft-cli oracle`, and a first corpus-wide comparison with fontc and fontmake. From this
   point compiler correctness is a number, not an opinion.
3. **M3, Edit View drawing**, in parallel with 1 and 2 once the model settles: the `tools` crate,
   selection, the Draw tool, node editing, transforms, the renderer, `ui.pointer` and
   `ui.screenshot` on the control channel, path operations.
4. Then M4–M7 on the editor track and M8–M12 on the engine track, as `docs/execution-plan.md` § 4
   orders them.

**The primary reference cannot be observed directly** (open decision O1: no licence). Editor
behaviour comes from the handbook and from public tutorial videos the owner watches and reports
on, so the engine track, where oracles prove correctness, goes first.

## Shipped so far

- **Planning:** charter, clean-room policy, reference index with licences, six decision records,
  architecture, execution plan, testing plan, UI design frame, control-protocol and MCP documents.
- **Specification:** 24 inventory files (919 commands, 33 tools, 170 panels, 45 settings, 241
  parameters, 1,741 behaviour rules, 300 checks, 70 CLI rows, 49 formats, 50 tables) and 22
  behaviour notes (about 127,000 words), all in our own words.
- **Workspace:** 8 library crates in enforced layers, two apps, `xtask`; about 7,000 lines of Rust.
- **Model:** fonts, masters, glyphs, layers, paths, components, anchors; `Arc`-shared glyphs;
  unknown keys preserved; invariants.
- **Engine:** sessions, per-document history, the command registry (18 engine commands), atomic
  failure, the panic guard, the journal.
- **Formats:** `.fontcraft` version 2 (versioned JSON, stable output, atomic writes, upgrade from
  version 1). `.glyphs` versions 2, 3 and 4 and `.glyphspackage`, read into the model and written
  back: every corpus source the reference app saved comes back byte for byte (17 files and 17
  packages, Oswald and Inter among them); unmodelled keys travel in `Extra` with their spelling.
  `file.open` recognises sources by content; `file.save` keeps the format and version, rewrites
  only changed package files, refuses to overwrite a file changed on disk, and reports losses.
  Compiled fonts (TrueType, CFF, collections, WOFF 1) open view-only. Kerning, axes, instances,
  master locations, features, display strings and the note are fields of the model.
- **Compiler:** static TrueType from the first master, deterministic.
- **Agent surfaces:** CLI (`commands`, `run`, `convert`, `info`, `mcp`), control channel
  (7 methods, loopback, request-only), MCP server (3 tools, headless or attached).
- **UI:** egui shell with a registry-built menu bar, Font View grid, Edit View display, Palette
  stub, status line, 9 shell commands; runs in a window and in headless test frames.
- **Gates:** `cargo xtask ci` with nine steps; parity report generated from the inventory;
  `cargo xtask roundtrip` over 527 corpus sources (outside the gate).

## Milestones

This file does not estimate time: estimates of agent hours have proved unreliable. Progress is
measured by what works (the parity table below and the measures in each milestone's plan).

| # | Milestone | Status | Commands (live) |
|---|---|---|---:|
| M0 | Skeleton and vertical slice | ✅ done | 5 (5) |
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | 🔶 in progress (M1.0 done, M1.1 OpenStep done) | 10 (5) |
| M2 | Compile core: static TTF and OTF, oracle harness | ⬜ | 6 (1) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | ⬜ | 210 (7) |
| M4 | Font View and glyph management | ⬜ | 57 (6) |
| M5 | Spacing, kerning and text mode | ⬜ | 109 (1) |
| M6 | Components and anchors | ⬜ | 65 (0) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | ⬜ | 62 (2) |
| M8 | OpenType features | ⬜ | 40 (0) |
| M9 | Interpolation and variable fonts | ⬜ | 68 (0) |
| M10 | Hinting (PostScript and TrueType) | ⬜ | 57 (0) |
| M11 | Colour fonts and images | ⬜ | 50 (0) |
| M12 | Command-line tool suite (60 of the 70 subcommand rows) and quality checks (263 of 300) | ⬜ | 70 (0) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | ⬜ | 79 (0) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 31 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default) | ⬜ | |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 919 (27) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 43 | 7 | 5% | tabs, toolbar, settings window, shortcut editor, workspaces, title bar |
| Font View | 6 | 62 | 6 | 3% | list view, sidebar and filters, glyph data, Add Glyphs syntax, batch commands, inspector |
| Edit View text mode | 4 | 25 | 0 | 0% | everything |
| Drawing and editing outlines | 10 | 89 | 2 | 1% | all tools, selection, node editing, transforms; only headless path creation exists |
| Path operations and filters | 6 | 64 | 0 | 0% | everything |
| Reusing shapes | 7 | 77 | 0 | 0% | everything; components are only flattened at export |
| Layers | 3 | 21 | 0 | 0% | everything |
| Guides, metric lines, zones, grid | 3 | 34 | 1 | 1% | metric lines are drawn; nothing is editable |
| Spacing and kerning | 7 | 79 | 1 | 1% | set width only |
| Font Info | 6 | 48 | 2 | 2% | four font-wide fields can be read and set |
| Interpolation | 8 | 68 | 0 | 0% | everything |
| OpenType layout | 7 | 46 | 0 | 0% | everything |
| PostScript hinting | 3 | 27 | 0 | 0% | everything |
| TrueType hinting | 4 | 40 | 0 | 0% | everything |
| Colour fonts and images | 3 | 46 | 0 | 0% | everything |
| Source formats | 6 | 14 | 4 | 35% | UFO, designspace, WOFF2 import; custom parameters and layer attributes carried untyped; version conversion |
| Compiling | 9 | 11 | 1 | 2% | static TrueType core tables; no oracle comparison |
| Preview, proofing, printing | 2 | 23 | 0 | 0% | everything |
| Command-line tools | 8 | 42 | 2 | 1% | `commands` and a first `run`, `convert`, `info` |
| Quality checks | 3 | 33 | 0 | 0% | everything |
| Plug-ins and scripting | 3 | 16 | 0 | 0% | everything |
| Agent surfaces | 4 | 11 | 1 | 25% | execute, list, inspect over three surfaces; no pointer, dialogs, screenshots, resources |
| **Feature parity** | **117** | **919** | **27** | **~3%** | |

## Out of scope (by design or by law)

- **Undocumented proprietary formats** (FontLab's binary formats, the primary reference's
  metrics files where no public description exists).
- **A reference app's plug-ins and Python scripts.** Ours is a different extension model
  (decision 0005).
- **Cloud services, accounts, telemetry, font-library subscriptions.**
- **Any reference app's bundled content**: glyph data beyond what is openly licensed, sample
  strings, icons, example fonts.

## Where we aim to be better than the references

- **Agent-native:** every command, gesture and dialog drivable over MCP, the CLI and the control
  channel; headless batch.
- **One tool for the editor and the command line:** the same commands and the same compiler, so
  what the editor previews is what the build produces.
- **Provable output:** oracle comparison against the open compilers as a standing gate.
- **Lossless interchange** between `.glyphs`, UFO and designspace, in both directions.
- **Everywhere:** macOS, Windows, Linux and the browser from one codebase.
- **Never loses work:** no panics, atomic commands, atomic saves, deep undo (10,000 steps).

## How to update this file

After each task lands: the milestone row, the parity-table row (score, missing items), "Shipped so
far", and the "What to build next" list if it changed. Keep the README's Status in step with the headline here.
