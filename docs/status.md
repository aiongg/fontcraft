# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-10-07, at the end of the planning session._

## State of the tree

- `cargo xtask ci` passes all nine steps (78 tests; about 7,000 lines of Rust). Built and run on
  Linux x86-64 with Rust 1.97.
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at. macOS and Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

## Where to start

**M1, the source model and the formats** (`ROADMAP.md` › What to build next). First steps:

1. Write `docs/plans/M1.md` (`docs/execution-plan.md` § 3). The material is
   `docs/spec/30-source-formats.md` (field tables, crate findings, the recommended model, 17 open
   questions) and `spec/inventory/30-formats.toml`.
2. `cargo xtask corpus --fetch` (needs network; several hundred megabytes) and install the oracles
   you will use (`docs/testing.md`).
3. Suggested packages, disjoint enough for parallel agents:
   - `crates/plist`: OpenStep and XML property lists, reading and writing, preserving spelling.
   - `crates/model`: grow to the recommended outline (axes, instances, kerning, features, layer
     kinds, parameters, tagged `extra`). This one is serial: everything else depends on it.
   - `crates/io-glyphs`: read and write versions 2, 3, 4 and the package form; byte-exact round
     trips over the `fontc`, `oswald` and `inter` corpora.
   - `crates/io-ufo`: UFO and designspace through norad.
   - `crates/decompile`: compiled TrueType fonts into the model.
   - Engine: `file.open` by content sniffing, `file.saveAs` with a format, loss reports.

## What the planning session decided

See `docs/decisions/`. In one line each: Glyphs 4 and AFDKO are the primary references; our own
compile pipeline with fontc as oracle and bootstrap backend; a one-document model shaped like the
primary's format with our own `.glyphs` reader and writer; commands with explicit targets and
atomic failure; WebAssembly plug-ins and no Python; no compiled fonts in the repository.

## Known weak spots

- **The inventory is unreviewed line by line.** It passed mechanical checks and one sample audit
  (209 handbook rows re-read by a second reader: 1 wrong, 14 imprecise, all corrected; patterns in
  `docs/spec/README.md`). Duplicated meaning across files is likely (open decision D2 lists known
  pairs).
- **33 rows are from memory** (`grep -c '^status = "memory"' spec/inventory/*.toml`): 12 TrueType
  autohinter options in `14-…`, 9 Edit-menu basics in `00-…`, 5 secondary-reference rows in
  `90-…`, 2 each in `05-…` and `30-…`, 1 each in `09-…`, `10-…` and `16-…`. Verify or drop them
  before implementing.
- **Nothing has been observed in the reference app.** About 440 open questions are listed at the
  ends of the notes.
- **No oracle has been run against our compiler.** The exported font was read back with skrifa
  and fonttools, which proves it is well-formed, not that it matches a reference compiler.
- **`docs/spec/20-cli.md` is very long** (about 26,000 words). Read the section for the subcommand
  you are building, not the whole file.

## Known limits in the skeleton's code

An adversarial review of the skeleton reported 31 findings. Most were fixed in the same session
(coordinate and point-count bounds before the glyph writer, glyph-name sanitising for `post`,
style linking, bounded reads on every channel, protocol id checks, the indexing lints, a
virtualised Font View). These were left, on purpose, for the milestone that reshapes the code
around them. None can lose data; most are about scale.

| Limit | Where | Fix it in |
|---|---|---|
| Glyph lookup by name is a linear scan | `crates/model` | M1 (index beside the glyph list) |
| Every edit clones the font's top level (glyphs themselves are shared) | `Session::edit` | M1 |
| The history list is copied for each command's result | `crates/engine` | M1 |
| The menu tree is rebuilt every frame | `crates/ui-egui/src/menus.rs` | M3 |
| A file is parsed twice on load (once to check, once to build) | `crates/format` | M1 |
| A key in an `extra` map that collides with a known field is not rejected | `crates/model` | M1 |
| `write_atomic` replaces a symlink with a file and does not keep the old file's mode | `crates/format` | M1 |
| No command switches the active document; the same file can be opened twice | `crates/engine` | M1 |
| An edit that changes nothing still makes an undo step | `Session::edit` | M1 |
| `OS/2` Unicode ranges: only two bits are set | `crates/compile` | M2 |
| The vendor-names scanner does not see through `concat!` or string escapes | `xtask/src/brands.rs` | when it matters |

## Waiting on the owner

`docs/spec/00-open-decisions.md` › For the owner: the name (O2), access to the sibling apps' shared
standards (O3), the application id (rest of O4), FontLab (O5), platforms (O6). O1 is answered (no
licence; tutorial videos watched by the owner) and so is the repository's home.

## Dev log

Append one terse entry per session: what landed, numbers, what is still open.

- **2026-10-07, planning session.** Read VectorCraft and PhotoCraft conventions. Chose
  references. Spiked dependencies: 14 font crates check for native and wasm; fontc compiles an
  in-memory `.glyphs` source to a variable font in 21 ms as a library. Inventory compiled by 17
  parallel readers (3,618 rows, 22 notes), deduplicated, checked for verbatim overlap with the
  handbook, then audited in a sample and fact-checked against the code. Skeleton built: 8 crates,
  2 apps, xtask; 27 commands; 78 tests; gate green. Found and fenced a panic in the glyph writer
  for points more than 32,767 units apart; fixed most of a 31-finding code review. Still open:
  everything in "Where to start" and the table of known limits.
