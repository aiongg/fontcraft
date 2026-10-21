# Depth plans: the second planning pass

The first plans built breadth: most commands exist. This pass plans depth: features that behave like
the reference. It is an index. Each package is written out in its milestone plan, in the section
"Depth packages" of `M<n>.md`.

## 1. Method

- Each of the 22 parity areas (`spec/areas.toml`) was graded against its inventory rows and its notes in
  `docs/spec/`, by what works today, erring low.
- Each area's gaps were ranked by weight × gap, and each gap became a package, sized for one agent.
- A **writer** package is written in a worktree with one compile check; its tests run after integration.
  A **builder** package runs in the main checkout, one at a time, because it needs builds, oracles or
  measures.
- Parity = Σ weight × score ÷ 117, as in ROADMAP.md › Parity estimate.

## 2. Areas: now and after

"After" counts every package that needs no open decision and no upstream merge. The last column adds
those.

| Area | Weight | Now | After | With the waiting packages | Plan |
|---|---:|---:|---:|---:|---|
| Window, tabs, settings, shortcuts | 5 | 24% | 39% | 46% (M14.7, M14.8) | [M14](M14.md) |
| Font View | 6 | 42% | 54% | 56% (M4.22, M4.23) | [M4](M4.md) |
| Edit View text mode | 4 | 50% | 60% | 66% (M5.31, M5.32) | [M5](M5.md) |
| Drawing | 10 | 31% | 46% | 50% (M3.27, M3.28) | [M3](M3.md) |
| Path operations | 6 | 33% | 47% | 49% (M3.38) | [M3](M3.md) |
| Reusing shapes | 7 | 21% | 38% | | [M6](M6.md), [M13](M13.md) |
| Layers | 3 | 37% | 55% | | [M3](M3.md) |
| Guides, metric lines | 3 | 30% | 47% | | [M3](M3.md) |
| Spacing and kerning | 7 | 49% | 60% | | [M5](M5.md), [M13](M13.md) |
| Font Info | 6 | 33% | 55% | | [M7](M7.md) |
| Interpolation | 8 | 29% | 45% | | [M9](M9.md) |
| OpenType layout | 7 | 25% | 42% | | [M8](M8.md) |
| PostScript hinting | 3 | 21% | 30% | | [M10](M10.md) |
| TrueType hinting | 4 | 27% | 35% | | [M10](M10.md) |
| Colour fonts and images | 3 | 39% | 50% | | [M11](M11.md), [M13](M13.md) |
| Source formats | 6 | 67% | 80% | | [M1](M1.md) |
| Compiling | 9 | 43% | 55% | | [M2](M2.md), [M3](M3.md), [M9](M9.md), [M12](M12.md), [M13](M13.md) |
| Preview, proofing, printing | 2 | 16% | 55% | | [M12](M12.md), [M13](M13.md) |
| Command-line tools | 8 | 41% | 60% | | [M12](M12.md) |
| Quality checks | 3 | 35% | 55% | | [M12](M12.md) |
| Plug-ins and scripting | 3 | 25% | 33% | 45% (M13.72 to M13.77) | [M13](M13.md) |
| Agent surfaces | 4 | 55% | 80% | | [M13](M13.md) |
| **Feature parity** | **117** | **36.0%** | **51.3%** | **52.7%** | |

The command-line grade of 41% predates three batches; graded by the same method as the other rows, today's
command-line tools reach about 58%. The table keeps 41% so that the gain is not counted twice.

| Point | Parity |
|---|---:|
| Now | 36.0% |
| After wave 1 | 38.1% |
| After wave 2 | 39.8% |
| After waves 1 to 13 (13 builders) | 48.6% |
| After every package and the builder tail | 51.3% |
| With the packages that wait on decisions or upstream | 52.7% |

## 3. Packages

167 packages: 110 writers, 49 builders and 8 close builders. Each milestone plan lists its own:

| Plan | Packages |
|---|---|
| [M1](M1.md) | M1.8 to M1.12 |
| [M2](M2.md) | M2.8 to M2.13 |
| [M3](M3.md) | M3.19 to M3.44 |
| [M4](M4.md) | M4.17 to M4.24 |
| [M5](M5.md) | M5.19 to M5.23, M5.29 to M5.32 |
| [M6](M6.md) | M6.20 to M6.25 |
| [M7](M7.md) | M7.21 to M7.23 |
| [M8](M8.md) | M8.25 to M8.35 |
| [M9](M9.md) | M9.27 to M9.39 |
| [M10](M10.md) | M10.21c, M10.33 to M10.41 |
| [M11](M11.md) | M11.21 |
| [M12](M12.md) | M12.36, M12.66 to M12.84 |
| [M13](M13.md) | M13.20b, M13.21b, M13.21c, M13.23c, M13.40 to M13.42, M13.62 to M13.82, X.pdf3 |
| [M14](M14.md) | M14.1 to M14.11, X.suite3 |

The close builders M6.19, M7.20, M8.24, M9.26, M10.32, M11.20, M12.65 and M13.61 keep their sections in the
first plans. M13.59 (Glyphs project files) can take a spare writer slot in wave 11 or 12.

## 4. Waves

Up to nine writers and one builder per wave. Integrate in the listed order. Lines that only register a
command, dialog, panel, setting or UI command merge easily.

| Wave | Writers | Builder | Parity after |
|---|---|---|---:|
| 1 | M1.8, M9.27, M13.78, M3.19, M7.21, M4.17, M3.40, M14.1, M3.21 | M14.3 | 38.1% |
| 2 | M12.67 (first), M7.22, M3.23, M13.80, M5.29, M4.18, M3.22, M9.29, M12.36 | M8.25 | 39.8% |
| 3 | M6.25 (first), M14.2, M8.26, M8.30, M6.20, M6.21, M12.66, M12.69, M4.19 | M2.10 | 41.2% |
| 4 | M3.39, M8.27, M2.8, M9.30, M3.24, M3.20, M4.21, M13.81, M5.20 | M13.40 | 42.4% |
| 5 | M2.12, M2.13, M8.28, M8.29, M7.23, M13.41, M9.32, M5.22, M12.68 | M12.81 | 43.4% |
| 6 | M14.4, M14.5, M14.6, M3.29, M3.41, M3.43, M5.30, M8.31, M13.42 | M12.71 | 44.5% |
| 7 | M3.42, M9.34, M9.36, M3.33, M10.34, M10.36, M10.40, M12.70, M12.73 | M13.79 | 45.3% |
| 8 | M10.35, M10.37, M12.74, M12.76, M12.78, M13.63, M11.21, M13.65, M13.71 | M3.30 | 46.0% |
| 9 | M13.64, M13.66, M13.68, M13.69, M13.74, M13.76, M12.77, M12.79, M13.21b | M13.20b | 46.9% |
| 10 | M13.70, M13.23c, M12.84, M1.9, M3.36, M8.35, M12.80, M13.62, X.pdf3 | M10.33 | 47.3% |
| 11 | M1.11, M8.32, M14.11, M4.24, M10.41, M4.20, M3.26, M12.75 | M10.21c | 48.0% |
| 12 | M5.19, M3.37; waiting on decisions: M3.28, M4.22, M5.31, M13.72 | X.suite3 | 48.1% |
| 13 | M3.44; waiting on decisions: M4.23, M13.73, M13.77 | M6.22 | 48.6% |

Wave 1 starts with the two gaps that lose or block work: closing a window with unsaved changes loses them
(M14.3), and File › Open… has no file chooser (M1.8). Only `app.rs` is shared between them. The rest of wave
1 gives the most parity per package.

## 5. Builder queue

Builders run one at a time in the main checkout, so the queue is the longest path. The measure-only builders
(M12.71, M12.72, M12.83, M8.34, M2.11, M10.39) can run in a second lane with their own clone and target
directory.

| Step | Builders, in order |
|---|---|
| Waves 1 to 13 | M14.3, M8.25, M2.10, M13.40, M12.81, M12.71, M13.79, M3.30, M13.20b, M10.33, M10.21c, X.suite3, M6.22 |
| Tail (to 51.3%) | M12.72, M3.25, M14.9, M13.67, M13.82, M12.82, M1.10, M12.83, M3.31, M9.31, M1.12, M8.34, M3.32, M5.23, M9.38, M9.39, M14.10, M9.28, M9.33, M9.37, M9.35, M3.34, M3.35, M2.9, M2.11, M5.21, M13.21c, M6.23 (then the writer M6.24), M10.38, M10.39, M8.33 |
| Waiting | M14.7, M14.8 (upstream); M3.27, M3.38, M5.32 (decisions); then the writer M13.75 |
| Closes | M6.19, M7.20, M8.24, M9.26, M10.32, M11.20, M12.65, M13.61 |

## 6. Open decisions

Each question is open. The packages named wait on it; nothing else does.

| # | Question | Options | Waiting |
|---|---|---|---|
| 1 | Reopen the deferred drawing features: super-smooth nodes, the Select All Layers tool, pen points, master stroke colour (M3 plan § 7) | keep them deferred; build them on recorded guesses; build each one a public video shows | M3.27, M3.28, M3.38, part of M14.6 |
| 2 | Auto-spacing target if no model reaches a 10% median error on Inter | accept the best model; switch to an absolute target (median within 2% of the em) | the close-out of M13.20b |
| 3 | Targets for M13.21c | a Rand index of at least 0.6 for Build Groups and a median of at most 20 units for auto-kerning; other targets | M13.21c's verdict (the package can run first) |
| 4 | Move the Glyph Data Editor rows from M14 to M4 | move; keep in M14 | M4.22, M4.23 |
| 5 | An in-app Emoji & Symbols picker | the macOS system picker only; an in-app picker; both | M5.31 |
| 6 | Reorder mixed-direction text in Edit View | build it on a guess; observe the reference first | M5.32 |
| 7 | Amend decision 0005 for a `script` module kind, macro step results and a `format` module kind | approve each; refuse | M13.72, M13.73, M13.75, M13.77 |
| 8 | Merge the upstream craft-keymap pull requests 164 and 186 | merge; wait | M14.7, M14.8 |

Settled by standing rules, so no package waits on them:

- Undo stays per document (decision D4).
- Public tutorial videos may be fetched over the network; they are free.
- Alternate layers with all bounds empty follow the handbook; the difference from both oracles is written
  down in `docs/spec/40-compile.md`.
- Invisible open corners are erased at export where fontmake erases them.
- Clipboard flavours come through native crates on Windows and macOS (X.pdf3); Linux keeps the clipboard tools
  unless a crate carries custom media types.
- Printing uses craft-ui's print panel after M14.9 (decision 0008).
- The control channel's element list uses AccessKit.
- Webfont Only and the switch that disables automatic behaviour follow the handbook, with guesses recorded.
- No update or crash-report service: it would be hosted.

## 7. Checks in the reference app

The plans name 87 checks to make in the running reference app and about 440 one-step entries for the
observation passes (M3.26, M3.37, M3.44, M4.20, M5.19). The checks that settle the most guesses:

1. Font View keys, the bottom-left buttons, the glyph context menu and Selection Info (M4.18, M4.19).
2. Caps, segments and brushes on the M13 § 8 test fonts (M13.40 to M13.42).
3. The spacing Info box with mixed values, locks, and shortcuts in vertical and right-to-left text.
4. Alt-drag on nodes, handles and segments, and the drag badges (M3.19, M3.20).
5. The Settings window: section order, defaults, handle sizes, colours, light mode (M14.1, M14.2, M3.23).
6. Option held in every menu (M14.4).
7. Export: kept settings, Export All and the export-time switches (M3.39, M2.10).
8. The feature editor: errors while typing, find, comment and indent keys, the sidebar menu (M8.25 to M8.29).
9. Path operations on test shapes: Remove Overlap, Add Extremes, Offset Curve, open-corner size (M3.30 to M3.34).
10. Higher-order interpolation: points, rotation, keys, `gvar` and Show All Masters (M9.28 to M9.33).

## 8. Inventory rows known to be out of date

| Row | What is wrong | Fixed in |
|---|---|---|
| `export.instancesBetweenMasters` | the summary predates the instance export | M2.8 |
| `export.fileNaming` | status `memory`; check it against fontc's and fontmake's naming | M2.8 |
| `path.sharpenCorner` | the status does not match its notes | M3.37 |
| `hoi.*` | the rows say M13; the work is M9's | M9.30 |
| `dialog.shapeOrder`, `window.shapeOrder` | two rows for one window | the next package that touches Shape Order |
| TrueType open question 4 in `docs/spec/14-truetype-hinting.md` | out of date: a badge drop runs `hint.tt.moveInstruction` | M10.34 |
