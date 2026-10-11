# Open decisions

Things the planning pass found that are not settled. **O** items need the owner; **D** items are
design decisions an agent can make and record in `docs/decisions/` when the feature is reached.
Each feature note also ends with its own open questions about the reference's behaviour (about
440 in all); those are answered by observation or by an oracle, not by a decision.

## For the owner

| # | Question | Why it matters | Default until answered |
|---|---|---|---|
| O1 | Is there a Mac with a licensed copy of the primary reference to observe? | Without it, every interaction detail the handbook omits is a guess: modifier keys, snapping distances, defaults, the many dialogs shown only as pictures | **Answered 2026-10-07: no licence.** The owner watches public tutorial videos and reports what they show (`docs/clean-room.md` › Tutorial videos). Engine track first; editor rows stay `handbook` or `inferred` until a video confirms them, and then become `observed` (D13) |
| O2 | The name. FontCraft is used by two unrelated projects | A rename after release costs file-extension and id migrations | Working name FontCraft, fallback GlyphCraft |
| O3 | Access to the ArtCraft `craftrules` repository | This repository mirrors their standards as inferred from two apps, not as written | Follow VectorCraft's and PhotoCraft's `AGENTS.md` |
| O4 | Where the repository lives, and the application id | CI, releases, the corpus fetch, `repository` in `Cargo.toml` | **Repository answered 2026-10-07:** private, `github.com/aiongg/fontcraft`. No hosted CI: the gate runs locally. Application id still open |
| O5 | Is FontLab available to observe? | It would be the observable app on Windows and for the secondary features | Documentation only |
| O6 | Which platforms matter first? | The primary is Mac-only; our users on Windows and Linux have no reference conventions | macOS, then Windows, Linux, web, as in the sibling apps |

## Design decisions to make when reached

| # | Decision | Arises in | Notes |
|---|---|---|---|
| D1 | Shortcut scoping: a `when` field for tool- and panel-local keys | M3 | The TrueType instructor's `A`, `S`, `F`… clash with tool keys; `Return` and `Delete` mean different things by selection. Decision 0004 item 5. **Answered 2026-10-09 (M3 plan):** an optional `when` on command and tool rows; a shortcut without `Cmd` or `Ctrl` always has one (decision 0004 item 5) |
| D2 | Merge document and whole-file command pairs | M3, M12 | `outlines.removeOverlaps` and `path.removeOverlap`, `font.transform` and `path.transformations`, `font.renameGlyphs` and the Font View rename, `audit.checkInterpolation` and `interpolation.checkCompatibility`, `transform.booleanUnion` and `path.removeOverlap`. **Answered for M3 (2026-10-09):** one implementation per action, the others aliases with a target or `allMasters` (decision 0004 item 2). The M12 pairs follow the same rule when built |
| D3 | Context-menu-only commands: a `context` field | M3 | 659 commands have `menu = []` (149 of them `ui`): context-menu items, panel controls and our headless forms of gestures. **Answered 2026-10-09 (M3 plan):** an optional `context` list of context menus (decision 0004 item 5) |
| D4 | Per-glyph undo on top of the per-document history | M3 | The reference undoes per glyph; ours per document. **Answered 2026-10-09 (M3 plan):** one history per document, global guides included; no per-glyph filter in M3 (decision 0004 item 4, `03a` § 21) |
| D5 | Which form of variable feature syntax we write | M8 | The feature-file spec requires unit letters and has `locationDef`; the Rust feature compiler differs. **Answered 2026-10-11 (M8 plan § 3):** FontCraft lowers the reference's variable syntax to the form fea-rs accepts (the feaLib form: optional units, no `locationDef`), writes that form in exported `.fea` files, and reads AFDKO's form where fea-rs does; the work is M9's (`fea.var.*`, `featureCode.variablePos*`) |
| D6 | Glyph data we must author | M4, M6 | Decomposition recipes, default anchors and sort order are not in the published glyph database. **Answered for M4 (2026-10-10, M4 plan):** the sort order is our own key (`glyphOrder.default` guesses), an opened file keeps its stored order, and a custom `GlyphData.xml` may supply `sortName`. Decomposition comes from Unicode canonical decomposition and anchors are authored; both stay open for M6. **Answered for M6 (2026-10-11, the owner's answer to M6 plan § 7, question 3 (a)):** Unicode canonical decomposition mapped to glyph names through the database, plus our own tables for the exceptions and for default anchors, measured against the composites and anchors stored in corpus sources. A custom `GlyphData.xml` that supplies decomposition or anchors overrides them |
| D7 | Element references (shared shapes with no owning glyph) | M13 | A secondary-reference feature with no place in either source format |
| D8 | Action sets as a file format | M13 | Command ids and parameter names become versioned data |
| D9 | `profile`, `fix` and threshold fields on checks | M12 | Today a check's profile, its repair command and its thresholds are only in `21-qa.md`; the audit needs them as data. **Answered for M3 (2026-10-09):** they live in the `audit` crate's registry as data; inventory fields wait for M12 (`21-qa.md` § 10). **Answered for M12 (M12 plan, 2026-10-11):** no inventory fields; the registry stays the one place, and a test compares its ids, scopes and severities with the `[[check]]` rows (package M12.1b) |
| D10 | Cross-platform stand-ins for Mac-only features | M5, M14 | System text shaping in Text Preview, the emoji picker, test install, system services. **Decided for M5 (the owner's answer, M5 plan § 7, question 6):** Text Preview uses FontCraft's own text layout (Unicode mapping, the document's kerning, direction) until M8 adds a shaping engine; characters the font lacks are drawn in the interface font. The rest stays open for M14 |
| D11 | Auto-spacing and auto-kerning algorithms | M13 | No reference documents its method |
| D12 | Fix target for outline repairs | M12 | In place with a backup layer (ours), or into a processed layer (the command-line reference). **Answered 2026-10-09 (M3 plan):** in place, the unfixed layer copied to a backup layer first (`21-qa.md` § 10) |
| D13 | How to mark a row confirmed from a tutorial video | M3 | **Answered 2026-10-09 by the owner: it counts as `observed`.** No new status value. A video shows the running app, so it is observation like running the app on our own fonts. The row's `notes` name the video: title, channel, URL and the time in the video (`docs/clean-room.md` › Tutorial videos) |

## Contradictions found in the sources

Recorded so nobody rediscovers them. Each is also in the note of the feature it affects.

- Info for Selection is `Cmd+Alt+I` in one handbook chapter and `Cmd+Shift+I` (also Show Info) in another.
- The default feature for alternate layers is `rlig` in the interpolation chapter and `rvrn` in the appendix. Both oracles use `rvrn`; the owner decides (`docs/plans/M9.md` § 7, question 2).
- Field increments: Shift ×10 and Cmd ×100 in the handbook; the format documentation says the largest step needs both.
- The Masters plus-menu entry is "Add Other Font" in one chapter and "Add Other Master" in another.
- The component info box's arrow button is described on opposite corners in two chapters.
- `Export SVG Table` is described as opt-in in the chapter and as on by default in the appendix.
- Component rotation: the format documentation says clockwise; the Rust reader treats it as counter-clockwise.
- Glyph order and alias database column order differs between the makeotf guide and the `tx` help. Settled by running tx 5.0.1 (M12 plan): the guide is right, the first column is the final name.
- Several appendix entries appear to carry a neighbour's description (`docs/spec/18-custom-parameters.md`, Open questions).
