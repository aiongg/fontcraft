# Architecture

How FontCraft is put together, and why. The M0 skeleton implements the shape described here with a
thin slice of behaviour; sections say **today** for what exists and **planned** for what the
milestones add. Decisions with alternatives are recorded in [`decisions/`](decisions/).

## One picture

```
            people                         agents                       scripts, CI
              │                              │                               │
        ┌─────▼─────┐   JSON lines    ┌──────▼──────┐  stdio JSON-RPC  ┌─────▼──────┐
        │ fontcraft │◄────────────────│ control     │◄─────────────────│ fontcraft- │
        │ (desktop, │  127.0.0.1:port │ protocol    │   MCP (attached) │ cli        │
        │  web)     │                 └─────────────┘                  │ run, mcp…  │
        └─────┬─────┘                                                  └─────┬──────┘
   L6   ui-egui: menus, panels, tools' overlays            mcp: tools ───────┤ (headless)
              │  app.run(id, params)                                         │
   L5   engine: Session ── execute(id, params) ── command registry ── history, journal, guard
              │
   L4   tools: pointer and key events → commands (begin / preview / commit)        (4 tools today)
   L3   format · compile · decompile · io-glyphs · io-ufo · io-svg · oracle · render · tables   (8 of 9 today)
   L2   pathops · interp · spacing · textlayout · components · features · hinting · audit · glyphquery · plugins   (pathops, audit, glyphquery today)
   L1   model: Font ─ masters ─ glyphs ─ layers ─ shapes (pure data, Arc-shared)
   L0   geom (kurbo)  ·  standalone crates: plist, cff, ttinstr, container, glyphdata  (all but ttinstr today)
```

Everything a person does goes through `app.run(id, params)`, which is the same call the control
channel, the MCP server and the CLI make. There is no second way in.

## Layers

`cargo xtask layers` enforces this table (`xtask/src/layers.rs`). A crate uses only lower layers.
Nothing below L6 may depend on a UI toolkit. Everything from L0 to L6 builds for
`wasm32-unknown-unknown` (`cargo xtask wasm`). The test kit is a dev-dependency only.

| Layer | Crate | State | Holds |
|---|---|---|---|
| L0 | `geom` | today | kurbo re-exports; cubic → quadratic; rounding. Later: extremes, inflections, curve fitting |
| L0 alone | `plist` | today (OpenStep); XML in M1 | OpenStep and XML property lists, reading and writing, keeping spelling, key order and layout (for `.glyphs` and UFO) |
| L0 alone | `glyphdata` | today (M4.1) | the glyph database: record lookup by nice, alternative and production name and code point, synthesis for suffixes, ligatures and code-point names, production names, nice and canonical names, the naming rules, direction, the default sort key, search, and custom `GlyphData.xml` files read within limits. Built-in tables (`data/glyphdata.bin`) compiled from GlyphsInfo, the Adobe Glyph List and the UCD by `cargo xtask glyphdata`; equal to fontc's and glyphsLib's answers on the cross-check apart from listed causes |
| L0 alone | `cff` | today (M2.3a: static CFF writer) | CFF tables from plain data: DICTs, INDEXes, charsets, Type 2 charstrings with the reference's specialiser, width optimisation and FontBBox; byte-identical to fontTools on the cross-check. Later: subroutiniser, hints (M10), CFF2 (M9), CID-keyed fonts (M12). Not in any crate we can use |
| L0 alone | `container` | today (M2.4a: WOFF 1, untransformed WOFF2) | WOFF and WOFF2 wrapping and unwrapping of sfnt fonts, as fontTools writes them; `crates/decompile` reads WOFF through it. Later: the WOFF2 glyf/loca and hmtx transforms (M2.4b), collections (M12) |
| L0 alone | `ttinstr` | M10 | TrueType instruction assembler and disassembler |
| L1 | `model` | today (M0 subset) | the font source model, pure data |
| L2 | `pathops` | M3 | overlap removal, booleans, offset, corner operations, clean-up (on linesweeper) |
| L2 | `interp` | M9 | variation model, interpolation of every master-bound value, compatibility checks |
| L2 | `spacing` | planned, built by M5.1 (kerning by M5.2) | metrics keys, sidebearings, kerning lookup, groups, locks, Clean Up and Compress, the kerning-text and AFM formats. Depends on `model` and `geom` |
| L2 | `textlayout` | planned, built by M5.6 | text as a list of items (glyph, placeholder, line break), characters mapped to glyphs by Unicode value and `/name` escapes, and the layout of those items with advances, kerning by mode, writing direction and wrapping; Edit View, the previews, Text Preview, `ui.render` and hit testing use it. Depends on `model` and `spacing` |
| L2 | `components` | M6 | automatic alignment, anchor propagation, smart, corner, cap and segment components |
| L2 | `features` | the pair-kerning writer in M5.17; the rest M8 | feature-code preprocessing (tokens), automatic features, compiling (on fea-rs), kern and mark writers |
| L2 | `hinting` | M10 | PostScript and TrueType hinting models and autohinters |
| L2 | `glyphquery` | today (M4.5) | which glyphs Font View shows: the search matcher, smart-filter rule trees compiled to the glyph predicate language (the Custom rule, global-guide filters and, in M8, the feature-code glyph tokens), list filters and folders, sidebar entries (categories, scripts and languages, `Groups*.plist` entries) and how a selection of them combines, and the glyphs an entry expects that the font lacks. Rule trees, predicates and patterns are read within depth, length and regex size limits |
| L2 | `audit` | today (M3.17: 34 outline checks); the rest M12 | the checks of `spec/inventory/21-qa-checks.toml`: a registry with ids, severities, profiles, thresholds and fixes, and pure check functions from one layer to findings. The `audit.*` commands run them |
| L2 | `plugins` | M13 | sandboxed WebAssembly plug-ins (same manifest as the sibling apps) |
| L3 | `format` | today | the native `.fontcraft` format, atomic writes |
| L3 | `compile` | today (M2.1: staged pipeline) | model → OpenType binary |
| L3 | `decompile` | M1 | OpenType binary → model, for opening compiled fonts |
| L3 | `io-glyphs` | M1 | `.glyphs` 2, 3, 4 and `.glyphspackage`, reading and writing, lossless |
| L3 | `io-ufo` | M1 | UFO and designspace, reading and writing, lossless |
| L3 | `io-svg` | today (M3.8) | SVG outlines in and out, and the clipboard fragment format. Reading runs on usvg, after a first pass that checks the limits and rewrites the document as geometry only |
| L3 | `oracle` | M2 | compiled fonts compared table by table with a reference compiler's build (the oracle harness behind `fontcraft-cli oracle`); runs the reference tools as subprocesses |
| L3 | `tables` | M1/M12 | dump, table edit, subset, merge, instance: the engine behind the CLI tools |
| L3 | `render` | today (M3.2) | glyph layers and lines of glyphs to RGBA images and PNG (tiny-skia, nonzero fill): `ui.render`, Edit View's filled glyphs, coverage measures |
| L4 | `tools` | today (M3.3: Select, Lasso Select, Hand, Zoom) | pointer tools: events in, commands and overlays out; hit testing, snapping, the selection and the Edit View keys |
| L5 | `engine` | today | sessions, history, the command registry |
| L6 | `ui-egui` | today (shell) | the UI |
| L6 | `mcp` | today | the MCP server |
| – | `testkit` | today | fixtures, junk parameters, invariants |
| L7 | `fontcraft`, `fontcraft-cli`, `xtask` | today | apps and tooling. `fontcraft-web` (the same app in the browser) joins at M3 |

A new crate is created in the lowest layer that works, starts with the no-panic lints (they are
inherited from the workspace), and is registered in the layering table in the same change.

## The model (L1)

**Today (M1.2 in progress):** `Font { source, family_name, units_per_em, version_major,
version_minor, metrics, masters, glyphs, extra }` with `metrics`, `masters`, `extra` and the glyph
table behind `Arc`s; the glyph table has a name index. Metrics are font-level definitions with
per-master values, as format version 3 stores them (`Font::metric`). `Glyph { name, unicodes,
export, layers }`; `Layer { id, master, width, shapes, anchors }`, where a master layer's id is its
master's; `Shape::{Path, Component}`; `Component { glyph, transform }` with the transform kept as
written (`Decomposed` or `Matrix`). Every struct keeps what it has no field for in an `Extra`:
entries tagged by format family, each a property-list value with its spelling and layout.

**Planned (M1):** the full model of [`docs/spec/30-source-formats.md`](spec/30-source-formats.md)
§ Recommended model. In short ([decision 0003](decisions/0003-source-model.md)):

- **One document per family**, shaped like the primary reference's file format (version 3 plus
  the version 4 additions): axes, masters, instances, glyphs with a layer per master plus backup,
  intermediate, alternate and colour layers, kerning by master, features as prefixes, classes and
  features, ordered repeatable custom parameters. UFO and designspace map into it with a short
  carry list; the reverse would push most data into untyped dictionaries.
- **A tree of `Arc`s.** A snapshot is a clone of the root. An edit copies the path from the root
  to what changed (the glyph table, one glyph, one layer) and shares the rest.
- **Lossless by construction.** Every struct carries an `extra` map tagged with the format family
  a key came from; numbers remember how they were written. Writers put back what readers did not
  understand.
- **Pure data.** No I/O, no caches, no UI state. `Font::check()` states the invariants: what a
  file must pass to load and what every command must leave true (names, code points, one layer
  per master, finite coordinates within a limit, well-formed node lists). `check_changed` applies
  them only to the glyphs an edit touched, so an edit does not cost a pass over the font.
- **Bounded work.** Resolving an outline is limited in component depth, in total segments and in
  coordinate range, so a hostile source cannot make it run away.

Node convention: nodes are in path direction; off-curve nodes precede the on-curve node that ends
their segment; a closed path's first segment may wrap. `Path::to_bez()` turns a node list into
Bézier segments or says what is wrong with it; nothing else interprets node lists.

## Commands (L5)

A command is a `CommandSpec { id, label, menu, shortcut, params, enabled, run, journal }`
(`crates/engine/src/cmd/mod.rs`). `run` is `fn(&mut Session, &Value) -> Result<Value>`: JSON in,
JSON out. Ids follow the reference's menu structure (`file.*`, `edit.*`, `glyph.*`, `path.*`,
`filter.*`, `view.*`…), and noun prefixes for what has no menu (`kerning.*`, `font.info.*`,
`hint.tt.*`, `audit.*`, `tables.*`).

What `Session::execute` guarantees for every command, checked for the whole registry by
`crates/engine/tests/command_sweep.rs`:

1. **It never panics.** Parameters are read through `params::Params`, whose accessors return
   errors naming the command and the key. A panic from a bug or a dependency is caught by
   `guard::catch_panic` and reported as an internal error.
2. **A failed command changes nothing.** `Session::edit` works on a copy and swaps it in only
   after the checks pass; around that, `execute` snapshots the session (pointers and labels)
   before a top-level command that is not a query, and restores it on any error.
3. **A successful command leaves every font valid** (`Font::check`).
4. **Changes are undo steps.** Fonts change only inside `Session::edit(label, |font| …)`, which
   pushes the previous root to the history. A document is modified when its font is not the one
   on disk, so undoing back to the saved state makes it unmodified again.
5. **Queries are not journaled**; changes are, with their parameters, so a session can be replayed
   (actions, bug reports, tests).

Rules for writing one ([decision 0004](decisions/0004-commands.md)):

- **Explicit targets.** A command names what it acts on in its parameters (`glyphs`, `layer`,
  node and shape references). The UI passes its selection; nothing in the engine reads UI state.
- **One command per action, not per place.** The same command serves the open document and, with
  a `document` path, a file on disk; the CLI's whole-font tools and the editor's menu items are the
  same commands with different targets. (The inventory still holds some pairs that must be
  merged: [`spec/00-open-decisions.md`](spec/00-open-decisions.md), D2.)
- **Dialogs are parameter editors.** A menu item with "…" opens a dialog that fills the command's
  parameters and previews by running it inside an interaction that is rolled back on Cancel.
- **Shell commands** (`UI_COMMANDS` in `ui-egui`, gathered from one list per module in `shell/`)
  change only view and window state: zoom, panels, selection, which tab shows. They are reachable from the control channel and MCP like any other.
- Nested commands share the outer command's transaction. `command.batch` runs a list of commands
  as one transaction and one undo step per document it changed; actions and scripts build on it.

## Tools (L4)

A pointer tool turns events into commands and overlays; it never edits the document. A drag is
`begin` (snapshot), any number of `preview`s (the command re-run on the snapshot), and `commit`
(one undo step) or `cancel`. The control channel injects the same pointer and key events
(`ui.pointer`, `ui.key`), so an agent can drive the Draw tool exactly as a hand does, and every
tool gesture also has a headless command (`path.addPath`, `selection.move`…) with explicit
coordinates. The gestures are specified in `[[tool]]` rows.

**Today:** `crates/tools` has the Select, Lasso Select, Hand and Zoom tools as state machines,
the toolbox (active tool, temporary tools on Space and Cmd+Space, key routing), hit testing,
snapping and the selection model. Events arrive in font units with the view they were measured
in, so tolerances given in screen points hold at every zoom. A tool sees the document as a
`Scene`: one layer, its master's guides and metric lines, and its components' outlines.

## History

One history per document, of whole-font snapshots. The reference keeps a history per glyph, with
font-level edits partly outside it; ours is simpler to reason about and never leaves the font in a
state no user created. Whether Edit View should offer per-glyph undo on top is open (D4). The
limit is 10,000 steps; a step costs a few pointers plus the data that changed.

## Compiling (L3)

**Today:** `compile(font, target, flavour, options)` builds a static TrueType font of one master,
or of an instance that sits at a master (`compile_ttf` is the first master). It runs in stages,
each a function of what the stages before it produced:

| Stage | Where | What it does |
|---|---|---|
| Resolve | `Font::resolve` in `crates/model` | the glyphs, contours, components, metrics and names at the target; no `Extra`, no editing data |
| Preflight | `crates/compile/src/preflight.rs` | component cycles refused, missing components dropped, non-exported glyphs inlined and removed, code point conflicts refused |
| Glyph order | `order.rs` | the font's order, `.notdef` first and drawn when absent, at most 65,535 glyphs |
| Outline preparation | `outline.rs` | mixed glyphs and transforms beyond ±2 decomposed (everything for CFF), cu2qu, contours reversed |
| Table builders, assembly | `build.rs` | `glyf` with composites, `loca`, `cmap`, `head`, `hhea`, `hmtx`, `maxp`, `name`, `OS/2`, `post` |

It is a pure function: the timestamp is an input, the same font gives the same bytes. The resolved
projection lives in the model because it reads every part of the font and nothing outside it; the
compiler's own choices (what to decompose, the order, the defaults) stay in `crates/compile`.

**Planned** ([decision 0002](decisions/0002-compile-pipeline.md), specified in
[`spec/40-compile.md`](spec/40-compile.md)):

```
source model
  → preflight (audit: what would fail or surprise)
  → resolve (export-time parameters, filters, glyph set, production names)
  → instance or keep masters (interpolate, or build the variation model)
  → outlines (decompose where required, remove overlaps for static, cubic → quadratic for TrueType)
  → glyph order
  → table builders (one module per table, each a pure function of the resolved font)
  → layout (preprocess feature code → fea-rs → GSUB, GPOS, GDEF, BASE; kern and mark writers)
  → variations (gvar, HVAR, MVAR, avar, STAT, fvar; CFF2 blends)
  → hinting (autohint or manual; instructions; CFF hints)
  → assemble (table order, checksums) → container (plain, WOFF, WOFF2, collection)
```

- We write tables with `write-fonts` where it has them, and ourselves where it does not: CFF and
  CFF2 in full, TrueType instructions, WOFF and WOFF2, collections, `kern`, the bitmap and SVG
  colour tables (`docs/spec/40-compile.md` § 3).
- **Oracles decide.** For every table, FontCraft's output from a corpus source is compared with
  fontc's and fontmake's (TrueType flavour) and fontmake's and makeotf's (CFF), by the `oracle`
  crate and `fontcraft-cli oracle`. See [`testing.md`](testing.md).
- **Incremental by design.** Table builders are functions of small parts of the resolved font, so
  editing one glyph recompiles that glyph's entries and the tables that depend on bounds. The
  editor's live preview and the final export are the same code.

## File formats (L3)

- **Native** `.fontcraft`: JSON, `{format, version, generator, font}`, stable key order, two-space
  indent, written atomically. Readers reject newer versions and oversized files. Planned: optional
  gzip, an embedded preview, a package (folder) form for version control.
- **`.glyphs` 2, 3 and 4 and `.glyphspackage`:** our own reader and writer, on our `plist` crate.
  No Rust crate writes the format, and the one that reads it is lossy by design
  (`docs/spec/30-source-formats.md` § 4). Round trips are byte-exact where the format allows.
- **UFO 2 and 3, designspace 4 and 5:** our own reader and writer, in the layout fontTools
  writes; norad reads our output in tests (decision 0003). Round trips are byte-exact for files
  ufoLib and designspaceLib wrote.
- **Compiled fonts** open for editing through `decompile` and are written by `compile`.
- **SVG** (`io-svg`): outlines are read through usvg and written as a compact document. Only
  geometry is read; styling, text, images and effects are left out and reported. A first pass
  checks the file against size, nesting and expansion limits and rewrites it as geometry only,
  so usvg never loads files or decodes images.
- Everything else (`spec/inventory/30-formats.toml`, 49 rows) is import or export around these.

Saving in a foreign format reports what that format cannot hold as structured warnings in the
command's result, so an agent can read them.

## Agent surfaces

| Surface | Transport | Where | Today |
|---|---|---|---|
| CLI | process, JSON on stdout | `apps/fontcraft-cli` | `commands`, `run`, `convert`, `info`, `mcp`. Planned set: `spec/inventory/20-cli.toml` (70 rows) |
| Control channel | JSON lines, loopback TCP | `apps/fontcraft/src/control_server.rs`, `crates/ui-egui/src/control.rs` | `engine.execute`, `engine.commands`, `document.inspect`, `ui.inspect`, `ui.menu.list`, `ui.menu.invoke`, `ui.screenshot`, `ui.render`, `control.methods`. Planned: pointer and key injection, dialogs ([`control-protocol.md`](control-protocol.md)) |
| MCP | stdio JSON-RPC | `crates/mcp` | three tools (`list_commands`, `run_command`, `inspect_document`), headless or attached to the app. Planned: resources, prompts, completions, renders ([`mcp.md`](mcp.md)) |

The request and reply shapes are the sibling apps'. The conventional control port is 7985 (theirs
is 7979), so an agent can drive FontCraft and VectorCraft side by side; the app listens only when
started with `--control PORT`.

## The UI (L6)

egui on wgpu through eframe; the web app is the same crate compiled to WebAssembly. `FontcraftApp`
owns the `Session` and a serializable `UiState`, and draws into any egui context, which is how
tests run frames without a window. The menu bar is generated from the registry: a command with a
menu path appears by itself; the tree is rebuilt only when the document or the selection changes.
Colours, radii and sizes come from `theme::Tokens`.

The crate has two halves. `shell/` holds the chrome and the shell commands: menus, keymap (menu
shortcuts with a modifier), tab strip, status bar, Font View, Palette. `canvas/` draws Edit View:
the camera (`canvas/view.rs`, which hands the tools crate its `View` transform), metric lines and
zones, filled glyphs (rasterised by `render` into a texture), the edited outline and the nodes
with the display conventions of `docs/spec/03a` § 4.

**Today:** menu bar, tab strip with Edit View tabs, Font View grid, Edit View (filled neighbours,
outline, nodes, metric lines and zones, wheel and pinch zoom and scroll), a Palette stub, the
status bar. **Planned:** the layout of [`ui-design.md`](ui-design.md).

Planned: text in the UI stays English in code and is translated at draw time from per-language
catalogues, as in the sibling apps. Command ids, the control channel, the CLI and MCP always use
the English ids and labels.

## Plug-ins and scripting (planned, M13)

No embedded Python. The scripting surface is the command registry itself (CLI `run --script`,
`command.batch`, recorded actions), and plug-ins are WebAssembly modules run in a sandbox, with the
manifest and parameter schema the sibling apps use: filters (glyph in, glyph out), reporters
(drawing over Edit View), checks and export steps ([decision 0005](decisions/0005-extensions.md)).

## Never crash, concretely

- Lints: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable`,
  `indexing_slicing` and `string_slice` are denied outside tests for every shipped crate (`xtask`,
  build tooling, is exempt); `unsafe_code` is forbidden.
- Limits on everything read from input, each a named constant: file size, node count, glyph
  count, code points per glyph, name length, component depth, segments per resolved outline,
  coordinate range, points per compiled glyph, line length and reply size on every protocol,
  connections, requests answered per frame.
- `guard::catch_panic` around command dispatch, each control request, each MCP request, each UI frame.
- The command sweep, format fuzzing by truncation and corruption, protocol tests with malformed
  input. Extend them with every importer, parser and protocol method.
- Dependencies are input too. Tests and an independent review found that the table writer
  overflows on points more than 32,767 units apart and on oversized name strings, panics on more
  glyph names than its `post` format can index, and that curve conversion and flattening allocate
  without bound on huge coordinates. Each now has a limit checked before the call
  (`MAX_TT_COORDINATE`, `MAX_TT_POINTS`, `MAX_FONT_GLYPHS`, `MAX_COMPONENTS`, `MAX_POST_NAMES` in `crates/compile`; `MAX_COORDINATE`,
  `MAX_OUTLINE_ELEMENTS`, `MAX_NAME_LEN` in `crates/model`) and a regression test.
- The panic guard cannot catch an out-of-memory abort, which is why unbounded work is treated as
  a crash.

## Performance targets

To be turned into a measured budget suite (`fontcraft-cli bench`, in the inventory at M12; bring
it forward when M3 starts). Until measured, these are aims, not facts.

| Operation | Target |
|---|---|
| Open a 3,000-glyph two-axis family | under 500 ms |
| Undo or redo | under 1 ms |
| Recompile one edited glyph for the live preview | under 16 ms |
| Full static compile, 3,000 glyphs | under 2 s |
| Font View scrolling, 10,000 glyphs | 60 frames per second |
| Remove overlap on one glyph | under 5 ms |

The plan is that the UI thread never blocks on compiling, saving or rendering previews: they run
on snapshots on worker threads, which the `Arc`-shared model makes cheap. Today everything runs on
the calling thread.

## Known scaling limits of the M0 code

Measured by the review of 2026-10-07. Fixed in M1.2: glyph lookup by name (now an index), the
copy of the font's top level on every edit (the parts are now behind `Arc`s), the double parse on
load, and undo steps for edits that change nothing. Still open:

- **An edit to a glyph copies the glyph pointer list** (one pointer per glyph): 80 µs at 3,017
  glyphs, under 2 ms extrapolated to 65,535, so the flat vector stays (`docs/plans/M1.md`,
  question 13). History is bounded by count, not yet by bytes.
- **A command that is not a query copies each document's history list** for its rollback snapshot
  (pointers and labels: about 0.5 ms at 10,000 undo steps). A persistent list removes it.
- **The menu tree is rebuilt every frame**, asking each command whether it is enabled. Fine at 27
  commands; cache it per document revision before there are hundreds.

