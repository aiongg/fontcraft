# FontCraft roadmap

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite, following the conventions of the ArtCraft Crafting Apps. This file tracks **how far it
is and what is left**. It is updated in the same change as every task that lands.

_Last updated: 2026-10-08 (M2.4a done: WOFF 1 and untransformed WOFF2 in the new `container` crate; M2.4b, the WOFF2 transforms, next)._

## Where we are

| Dimension | Status |
|---|---|
| Planning: charter, rules, references, decisions | **done** for this stage (`docs/`) |
| Feature inventory: every command, tool, panel, parameter, check, CLI tool, format, table | **3,621 rows**, complete in breadth, checked mechanically; depth varies (`docs/spec/README.md`) |
| Infrastructure: workspace, layering, gates, registry, history, guard, CLI, control channel, MCP, UI shell, tests | **~35%** of what the siblings' "infrastructure" row covers |
| Feature surface against the references (weighted, by behaviour, self-graded) | **~3%** |
| Commands that exist (not: that behave like the reference) | **32 of 920** (`docs/parity.md`) |
| Observed against the running reference app | **nothing** |

## Honest assessment (2026-10-08)

**In one line:** the project reads and writes `.glyphs`, UFO and designspace sources without loss and opens compiled fonts; it is not yet a font editor.

**What is real.** `.glyphs` files of versions 2, 3 and 4 and `.glyphspackage` folders open and
save back byte for byte (every corpus source the reference app wrote, Inter and Oswald among
them), and either form saves as the other. UFO 2 and 3 and designspace documents with their
UFOs open as one document and save back byte for byte (every readable corpus file: 125 UFOs, 59
designspace documents); a save rewrites only the files an edit touched, and norad reads what we
write. A `.glyphs` document saves as UFO or designspace and back: every reference-written
source comes back with the same model except what the loss report lists, and fontmake builds
Oswald from our designspace with the same outlines (but the advance widths of non-spacing marks),
names and OS/2 values as from the `.glyphs` source.
Compiled TrueType, CFF, collection and WOFF 1 fonts
open view-only (all 342 corpus fonts). The native `.fontcraft` format holds everything a `.glyphs`
source holds. A new font can be created, glyphs added, removed and encoded, outlines added from
node lists, widths and font info set, everything undone and redone, several edits batched into
one undo step, and a static TrueType font exported (of a master, or of an instance at a master,
with composites kept and `.notdef` drawn when the font has none) that fonttools and skrifa read
back correctly. All of that works headless (`fontcraft-cli run`), over MCP, over the control
channel into the running window. From the window's menus only the commands that need no
parameters work (New, Close, Undo, Redo, New Glyph, the View and Window toggles): there are no
dialogs yet. Font View and Edit View draw. `fontcraft-cli oracle` compiles a source with
FontCraft and with fontc or fontmake and compares the two fonts table by table; over 354 corpus
sources it measures the compiler: 332 compared with fontc, and every core table identical,
equivalent or allow-listed with a reason in all of them; 331 compared with fontmake's CFF builds,
whose outlines FontCraft's match in 320. 475 tests pass, including a sweep that runs every engine
command with hostile parameters. The whole gate
(`cargo xtask ci`: format, lints, tests, spec, parity, assets, vendor names, layering, wasm) is green.

**What is not.** There is no drawing with the pointer, no selection of nodes, no path operations,
no kerning editing, no component editing (components are exported as composites), no feature compilation, no
variable fonts, no CFF export command (the compiler builds CFF; the command comes in M2.5), no hinting, no WOFF2, no screenshots over the control channel,
no web app, no packaging. Glyphs-only data travels in glyphsLib's UFO lib keys where glyphsLib
has a key for it, and alternate and intermediate layers travel as designspace rules and sparse
sources; the data no key holds (glyph tags, locks, sort names, layer colours, drawing
attributes…) does not, and the loss report lists it. Production names and the widths of
non-spacing marks need a glyph database (M4). Kerning, features and
instances are read and written, not yet edited or compiled. Edit View shows a glyph; it does not edit one.

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
| **Breadth:** commands, tools and panels exist | ~3% | 32 of 920 commands; 0 of 33 tools; 2 of 170 panels as stubs |
| **Depth:** features behave like the reference | ~1–2% | only the M0 slice has behaviour, and none of it has been compared with the reference |
| **File interop** | ~35% | own format, `.glyphs` versions 2 to 4 and `.glyphspackage`, UFO 2 and 3 and designspace in and out (every reference-written corpus source saves back byte for byte, and through the other family with only reported losses); UFOZ and compiled TrueType, CFF, collections and WOFF 1 open (all 342 corpus fonts); static TrueType out from the export command; static CFF from the compiler and the oracle. Missing: the OTF export command, variable, WOFF2, version conversion, a glyph database for production names |
| **Compiler correctness** | measured, low | 13 of 50 tables written, for two static flavours (TrueType, CFF). Against fontc over 332 corpus sources every core table is identical, equivalent or allow-listed: allowed differences come from fontc building variable fonts, its bracket and colour glyphs, non-spacing marks only the glyph database knows (M4), smart components (M6) and four reader questions (`docs/plans/M2.md` › M2.2). Against fontmake's CFF builds over 331 sources, 320 CFF tables decode to the same outlines and 195 are byte-identical; where fontmake and fontc disagree FontCraft follows fontc and the allow-list says why; 9 to 11 sources per table are left different (`docs/plans/M2.md` › M2.3b). No variable font, no features |
| **Agent automation** | ~40% | every existing command is reachable from CLI, control channel and MCP; no pointer injection, dialogs, screenshots, resources or prompts yet |
| **Robustness** | good for its size | no panics found in shipped code (lints including indexing, guard, sweep, hostile-input tests); one dependency panic found and fenced; one adversarial code review, most findings fixed and the rest listed in `docs/status.md`. No fuzzing, no corpus runs, and the known scaling limits in `docs/status.md` |
| **Performance** | unmeasured | targets written down (`docs/architecture.md`); no budget suite |
| **Platforms** | Linux run verified; others untested | built and run on Linux (virtual display). macOS and Windows have not been built. wasm: libraries check, no web app |
| **Look and feel** | not started | a functional shell with the siblings' default dark tokens; nothing measured against any reference |

### What to build next (in priority order)

1. **M2, the compile core** (plan accepted, `docs/plans/M2.md`): M2.0 (the oracle harness),
   M2.1 (the staged pipeline), M2.2 (core tables at fontc parity) and M2.3 (static CFF) are
   done, and M2.4a (WOFF 1 and untransformed WOFF2, `crates/container`). Next M2.4b (the WOFF2
   `glyf`/`loca` and `hmtx` transforms, WOFF2 opening) and the export commands (M2.5, which also
   offer CFF and the containers). Four
   reader questions from M2.2 and the differences left against fontmake (plan › M2.3b) are open.
2. **M3, Edit View drawing**, in parallel with 1 now that the model has settled: the `tools` crate,
   selection, the Draw tool, node editing, transforms, the renderer, `ui.pointer` and
   `ui.screenshot` on the control channel, path operations.
3. Then M4–M7 on the editor track and M8–M12 on the engine track, as `docs/execution-plan.md` § 4
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
- **Workspace:** 15 library crates in enforced layers, two apps, `xtask`; about 52,800 lines of Rust.
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
  UFO 2 and 3 (a UFO is a master) and designspace documents with their UFOs (one document)
  open and save back byte for byte in fontTools' layout, keeping each file's own layout and
  spellings while they still mean what the model holds; UFOZ opens; `file.exportUfo` writes
  masters as UFOs. Compiled fonts (TrueType, CFF, collections, WOFF 1) open view-only. Kerning,
  axes, instances, master locations, features, display strings and the note are fields of the
  model, and so are the font's properties, custom parameters, settings, date and user data: UFO
  font information maps to the reference's names through a crosswalk ported from glyphsLib.
  From `.glyphs`, stems, numbers, master guides, alignment zones, version 2 axes and name parts,
  and the instances' classes, style linking and interpolation settings are fields too. UFO blue
  values, stem snaps and guidelines read into and write from the same fields, by glyphsLib's
  rules. Designspace axes (user range, discrete values, localised names, STAT labels) are axis
  fields, their maps the `Axis Mappings` parameter; designspace instances and variable fonts
  are the model's instances, with glyphsLib's instance lib keys. A `.glyphs` source written as a
  designspace gets glyphsLib's axis ranges, maps and instance attributes. A glyph's metadata
  (production name, classification, metrics keys, colour, tags, note, lock, sort names,
  smart-glyph properties, user data) is a model field; in a UFO the note, mark colour,
  production names and `GDEF` classes travel, the rest is reported lost. A layer's name,
  attributes, vertical metrics, metrics keys, colour, guides, hints, annotations, image,
  background and user data are model fields of `.glyphs` and the native file; in a UFO the
  backgrounds, backup, intermediate and colour layers, guides, images, vertical metrics and user
  data travel by glyphsLib's rules, the rest is reported lost. In a designspace, intermediate
  layers also get sparse sources and alternate layers become bracket glyphs with rules, as
  glyphsLib writes them, and both read back as layers. Shape data is typed too: node
  joins, locks, orientations, names and user data, path locks and drawing attributes, the
  components' alignment, attachment anchors, smart values, masters, locks and orientations, the
  anchors' locks and orientations, and version 4 image shapes; in a UFO the node joins and names
  and the identifiers and anchor colours travel, the rest is reported lost. A node stays 32
  bytes. Version 4 contextual kerning is kept as stored and feature labels are fields; a UFO's
  groups that are not kerning groups are the font's groups (glyphsLib's classes in `.glyphs`),
  UFO 2 kerning groups are the glyphs' kerning groups, and `features.fea` is written with
  glyphsLib's markers and split back into prefixes, classes and features when that gives the
  same text. The font's settings, app version, edit-tab texts, user data and custom parameters
  with no font-info key, and the masters' icons, name parts and such parameters travel in
  glyphsLib's UFO lib keys and are read back from them where they say what the model would
  write. So do the glyphs' categories, scripts, last changes, metrics keys, smart-glyph
  properties and user data, the layers' metrics keys, hints, annotations, image crops, colour
  palette entries, ids and order, the UFO layer libs, and the shapes' order, the components'
  alignment, anchors, locks and smart values, node user data and anchor user data. Numbers,
  stem names, master visibility, glyph tags, locks and sort names, layer colours, drawing
  attributes, and right-to-left, vertical and contextual kerning are not yet written to UFO
  (glyphsLib has no key for them). M1.7's measures: every round trip unchanged; the census of
  untyped keys explained; all 35 reference-written `.glyphs` sources through a designspace and
  back with only reported losses (a test in the suite); fontmake's builds of Oswald from our
  designspace and from the `.glyphs` equal in 4,614 of 4,626 static glyphs and 882 of 883
  variable-font glyphs (the rest are non-spacing mark widths), with equal names and OS/2 values
  but `xAvgCharWidth` in two instances. Saving a UFO or designspace in another format reports
  the data only its files held, not their layout.
- **Compiler:** static TrueType of a master or of an instance at a master, deterministic, in
  stages (`Font::resolve`, preflight, glyph order, outline preparation, table builders): missing
  components dropped, non-exported glyphs inlined, `.notdef` drawn as fontc draws it,
  composites kept unless mixed or beyond ±2, cu2qu at 0.001 em, contours reversed, half-up
  rounding, composite boxes and `maxp` limits as fontc computes them. The core tables (head,
  hhea, hmtx, maxp, OS/2, name, cmap, post, vhea, vmtx) derive every field as fontc does:
  metric defaults per source format, names with fontc's fallbacks, style flags, `NONE` as the
  vendor id, Unicode and code page ranges, `USE_MY_METRICS`.
- **CFF writer:** `crates/cff` (standalone): a static, name-keyed CFF table from plain data
  (names, Top and Private DICT values, glyph names, advances, cubic outlines), with the
  reference's coordinate rounding, operator specialiser, width optimiser, FontBBox, charset
  choice and layout. On 115 fonts (every UFO in the corpus the reference compiles, plus
  synthetic ones) its tables are byte-identical to fontTools'. No hints, no subroutines.
- **Static CFF in the compiler:** `Flavour::Cff` builds an OTF of a master as fontmake does:
  decomposed cubic outlines in their drawn direction, glyph names in the charset, Top DICT
  names from the `name` table, blue zones and stems from the master, `maxp` 0.5, `post` 3.0,
  metrics from the charstring bounds. Against fontmake over 331 corpus sources: 320 CFF tables
  decode to the same outlines, 195 are byte-identical. Every font built from the corpus (347
  OTF, 347 TTF) reads back in fontTools.
- **Containers:** `crates/container` (standalone): WOFF 1 and WOFF2 with untransformed tables,
  written and read with fontTools' table order, checksums, header fields and compression
  settings. All 342 corpus fonts wrap and unwrap to the same tables; fontTools reads all 340
  TTF and OTF wrappings back to its own tables; 19 WOFF2 files are byte-identical to fontTools'
  and the rest differ only in the compressed streams. Transformed WOFF2 is refused for now.
  `file.open` reads WOFF 1 through it.
- **Oracle harness:** `crates/oracle` and `fontcraft-cli oracle`: FontCraft's build and fontc's
  or fontmake's (TrueType, or CFF from fontmake) of the same master, compared per table after
  normalisation (identical, equivalent, allowed with a reason, different, skipped), with
  source-reading differences reported apart; `--corpus` over 354 sources; opt-in oracle tests.
- **Agent surfaces:** CLI (`commands`, `run`, `convert`, `info`, `oracle`, `mcp`), control channel
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
| M1 | Source model and file I/O (native, UFO, designspace, `.glyphs`, binary import) | ✅ done: every format read and written back byte for byte; the typed model (M1.7) with glyphsLib's lib keys, intermediate and alternate layers in a designspace, and its measures met; the gaps left on purpose are in `docs/plans/M1.md` (M1.4 and M1.7) | 10 (10) |
| M2 | Compile core: static TTF and OTF, oracle harness | 🟨 M2.0 to M2.3 done: the oracle harness, the staged pipeline, the core tables at fontc parity, static CFF compared with fontmake, and M2.4a (WOFF 1 and untransformed WOFF2 wrap and unwrap every corpus font table for table) (`docs/plans/M2.md`); M2.4b next | 6 (1) |
| M3 | Edit View drawing: tools, nodes, selection, transforms, path operations, guides | ⬜ | 210 (7) |
| M4 | Font View and glyph management | ⬜ | 57 (6) |
| M5 | Spacing, kerning and text mode | ⬜ | 109 (1) |
| M6 | Components and anchors | ⬜ | 65 (0) |
| M7 | Font Info and custom parameters (70 of the 241 parameter rows land here; 205 of the 241 are custom parameters) | ⬜ | 62 (2) |
| M8 | OpenType features | ⬜ | 40 (0) |
| M9 | Interpolation and variable fonts | ⬜ | 68 (0) |
| M10 | Hinting (PostScript and TrueType) | ⬜ | 57 (0) |
| M11 | Colour fonts and images | ⬜ | 50 (0) |
| M12 | Command-line tool suite (61 of the 70 subcommand rows) and quality checks (263 of 300) | ⬜ | 71 (0) |
| M13 | Plug-ins, scripting and automation depth (with the secondary references' features) | ⬜ | 79 (0) |
| M14 | 1.0 polish: settings, shortcuts editor, languages, accessibility, packaging, web | ⬜ | 31 (0) |
| – | Interaction-fidelity pass (side by side with the reference: every modifier, cursor, default) | ⬜ | |
| – | Hardening at scale (corpora, fuzzing, cross-platform and browser QA) | ⬜ | |
| | **Total** | | 920 (32) |

## Parity estimate

Method, as in the sibling apps: the feature surface is split into areas weighted by how much of
the product and of real users' work each is (`spec/areas.toml`). Each area is scored by depth of
behaviour, not by whether a command exists; an area is 100% only when every feature in it behaves
like the reference. Feature parity = Σ weight × score ÷ Σ weight. Scores are self-graded and
err low. `docs/parity.md` gives the mechanical counts behind the "exists" column.

| Area | Weight | Commands | Exist | Done (behaviour) | Missing (main items) |
|---|---:|---:|---:|---:|---|
| Window, tabs, toolbar, sidebars, settings, shortcuts | 5 | 44 | 8 | 5% | tabs, toolbar, settings window, shortcut editor, workspaces, title bar |
| Font View | 6 | 62 | 6 | 3% | list view, sidebar and filters, glyph data, Add Glyphs syntax, batch commands, inspector |
| Edit View text mode | 4 | 25 | 0 | 0% | everything |
| Drawing and editing outlines | 10 | 89 | 2 | 1% | all tools, selection, node editing, transforms; only headless path creation exists |
| Path operations and filters | 6 | 64 | 0 | 0% | everything |
| Reusing shapes | 7 | 77 | 0 | 0% | everything; components are only exported (as composites, decomposed where the format needs it) |
| Layers | 3 | 21 | 0 | 0% | everything |
| Guides, metric lines, zones, grid | 3 | 34 | 1 | 1% | metric lines are drawn; nothing is editable |
| Spacing and kerning | 7 | 79 | 1 | 1% | set width only |
| Font Info | 6 | 48 | 2 | 2% | four font-wide fields can be read and set |
| Interpolation | 8 | 68 | 0 | 0% | everything |
| OpenType layout | 7 | 46 | 0 | 0% | everything |
| PostScript hinting | 3 | 27 | 0 | 0% | everything |
| TrueType hinting | 4 | 40 | 0 | 0% | everything |
| Colour fonts and images | 3 | 46 | 0 | 0% | everything |
| Source formats | 6 | 14 | 5 | 55% | WOFF2 import; Glyphs-only data glyphsLib has no UFO key for; alternate layers' order and ids through a designspace, and alternate layers in a single UFO; STAT labels derived from instances; production names and mark widths from a glyph database; UFO 1; shape groups, gradients and shadows kept as stored values; right-to-left, vertical and contextual kerning in UFO; version conversion |
| Compiling | 9 | 11 | 1 | 4% | measured against fontc (TrueType) and fontmake (CFF): core tables identical, equivalent or allow-listed in most sources; static CFF from the compiler, not yet from the export command; WOFF 1 and untransformed WOFF2 written by the container crate (not yet from a command), no WOFF2 transforms, no variable fonts, no features |
| Preview, proofing, printing | 2 | 23 | 0 | 0% | everything |
| Command-line tools | 8 | 42 | 4 | 1% | `commands`, `run`, `convert`, `info`, `tables list`, `glyphs list`, `oracle`; no dump, subset, instance, merge |
| Quality checks | 3 | 33 | 0 | 0% | everything |
| Plug-ins and scripting | 3 | 16 | 0 | 0% | everything |
| Agent surfaces | 4 | 11 | 2 | 25% | execute, list, inspect over three surfaces; no pointer, dialogs, screenshots, resources |
| **Feature parity** | **117** | **920** | **32** | **~3%** | |

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
