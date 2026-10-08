# Feature specification

What FontCraft is meant to do, in two forms that point at each other:

- **`spec/inventory/*.toml`: the inventory.** 3,618 rows a tool can read: every command, tool,
  panel, setting, custom parameter, behaviour rule, quality check, CLI subcommand, file format and
  OpenType table. `cargo xtask spec` checks it; `cargo xtask parity` compares it with the code and
  writes [`../parity.md`](../parity.md). The row schema is [`../../spec/README.md`](../../spec/README.md).
- **`docs/spec/*.md`: the behaviour notes** (this folder). How each feature behaves, in our own
  words, with the ids of the rows that implement it and, at the end of every file, the **open
  questions** the public documentation leaves unanswered.

Breadth is complete; depth varies. The notes are deep where the documentation is, and each file's
open questions say where it is not. Before starting a milestone, its notes get a planning pass
(`../execution-plan.md` § 3): open questions are settled by observing the reference app where that
is possible, by an oracle where there is one, or by a recorded decision of ours.

## Reading order

| Notes | Inventory | Source | Rows | Open questions |
|---|---|---|---:|---:|
| [00 Open decisions](00-open-decisions.md) | `00-app-basics.toml` | ours | 17 | – |
| [03a Edit View: drawing](03a-edit-view-drawing.md) | `03a-edit-view-drawing.toml` | handbook ch. 1–3 | 190 | 30 |
| [03b Edit View: display, text, measuring](03b-edit-view-display.md) | `03b-edit-view-display.toml` | handbook ch. 3 | 187 | 17 |
| [04 Palette](04-palette.md) | `04-palette.toml` | handbook ch. 4 | 75 | 9 |
| [05 Font View](05-font-view.md) | `05-font-view.toml` | handbook ch. 5, GlyphData | 284 | 20 |
| [06 Font Info](06-font-info.md) | `06-font-info.toml` | handbook ch. 6 | 200 | 29 |
| [07 Import and export](07-import-export.md) | `07-import-export.toml` | handbook ch. 7 | 102 | 20 |
| [08 Spacing and kerning](08-spacing-kerning.md) | `08-spacing-kerning.toml` | handbook ch. 8 | 126 | 19 |
| [09 Reusing shapes](09-reusing-shapes.md) | `09-reusing-shapes.toml` | handbook ch. 9 | 167 | 39 |
| [10 Interpolation](10-interpolation.md) | `10-interpolation.toml` | handbook ch. 10, designspace docs | 131 | 21 |
| [11 Filters](11-filters.md) | `11-filters.toml` | handbook ch. 11 | 85 | 14 |
| [12 Feature code](12-feature-code.md) | `12-feature-code.toml`, `12b-feature-language.toml` | handbook ch. 12 and appendix, the feature-file spec | 361 | 26 |
| [13 PostScript hinting](13-postscript-hinting.md) | `13-postscript-hinting.toml` | handbook ch. 13, otfautohint | 93 | 12 |
| [14 TrueType hinting](14-truetype-hinting.md) | `14-truetype-hinting.toml` | handbook ch. 14 | 121 | 18 |
| [15 Colour fonts](15-color-fonts.md) | `15-color-fonts.toml` | handbook ch. 15 | 109 | 22 |
| [16 Settings](16-settings.md) | `16-settings.toml` | handbook ch. 16 | 104 | 13 |
| [17 Extensions](17-extensions.md) | `17-extensions.toml` | handbook ch. 17, our design | 54 | 10 |
| [18 Custom parameters](18-custom-parameters.md) | `18-custom-parameters.toml` | handbook appendix (all 205) | 208 | 44 |
| [20 Command line](20-cli.md) | `20-cli.toml` | AFDKO, fonttools, fontmake, fontc | 151 | 20 |
| [21 Quality checks](21-qa.md) | `21-qa-checks.toml` | fontbakery, AFDKO, fonttools | 343 | 13 |
| [30 Source formats](30-source-formats.md) | `30-formats.toml` | format specifications, crate sources | 94 | 17 |
| [31 glyphsLib crosswalk](31-glyphslib-crosswalk.md) | — | glyphsLib 6.15.0 source | — | — |
| [40 Compiling](40-compile.md) | `40-tables.toml` | fontations, fontc, fontmake, makeotf | 181 | 18 |
| [90 Secondary references](90-secondary-references.md) | `90-secondary.toml` | FontLab, RoboFont, FontForge and Fontra documentation | 235 | 12 |

## How much to trust a row

| `status` | Rows | Means |
|---|---:|---|
| `handbook` | 2,113 | stated in the primary reference's public handbook |
| `docs` | 1,097 | stated in another public document or in permissively licensed source |
| `inferred` | 375 | our own design: headless forms of pointer actions, queries, FontCraft-only features |
| `memory` | 33 | recalled, not found in a source read for this work. **Verify before implementing** |
| `observed` | 0 | checked against the running reference app. None yet |

The inventory was compiled on 2026-10-07 by reading each source once, in parallel, one chapter per
reader. It has been checked mechanically (schema, unique ids, valid references, no long verbatim
overlap with the handbook) and reviewed in samples, not line by line. Expect wrong details. When
you find one, fix the row and its note in the same change as the code.

### What the sample audit found

A second reader, who had not written the rows, re-read 209 randomly sampled `handbook` rows
against the handbook text: 1 was wrong (0.5%) and 14 were imprecise (6.7%); all 15 were corrected.
Scaled to the 2,113 handbook rows that suggests roughly 10 wrong and 140 imprecise rows still in
the files. The patterns worth knowing before you rely on a row:

- **Shortcuts taken from another chapter.** A row's shortcut is sometimes stated in a different
  chapter from the one in its `ref`, and the handbook contradicts itself in a few places
  (`00-open-decisions.md` › Contradictions).
- **Coined labels.** About 38% of sampled command labels are ours, because the action has no
  menu item in the reference (a panel control, a gesture, a headless form). Such a label is a
  proposal, not a fact about the reference.
- **Parameter scopes are mostly empty.** The handbook appendix says where a custom parameter
  applies (font, master, instance) for only some of them; the rest wait for observation.
- The `docs`, `inferred` and `memory` rows were **not** part of the audit.

## Conventions the rows use

- Shortcuts are in Mac notation, `Cmd+Ctrl+Alt+Shift+Key`. Per-platform mapping is the shell's
  job (`shortcuts.platformMapping`).
- A command whose `menu` is empty has no menu item in the reference: it is a context-menu item, a
  panel control, or our headless form of a gesture. Its `summary` says which.
- Modifier-key alternates of a menu item ("hold Option for all masters") are separate rows with
  an `allMasters`-style parameter on the base command; they are aliases of one implementation
  ([decision 0004](../decisions/0004-commands.md)).
- A key sequence that maps to a family of actions (`Cmd+1` to `Cmd+9`) is one parametrised command
  holding the first shortcut; the `summary` gives the range.
- `[[parameter]]` names are spelled exactly as the source files spell them. Built-in Font Info
  fields are prefixed with their scope (`font.unitsPerEm`) to keep them apart from custom
  parameters of the same name.
