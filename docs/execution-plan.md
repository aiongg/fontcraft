# Execution plan

How the work is done: the loop every task follows, how a milestone is planned, how parallel agents
stay out of each other's way, and which decisions belong to the owner.

## 1. The task loop

**Orient → plan → implement with tests → verify → record → commit.** Do not stop to ask unless § 7
lists the decision as the owner's.

1. **Orient.** Read `docs/status.md` and the ROADMAP row. Read the feature's inventory rows
   (`grep -n 'id = "path.' spec/inventory/*.toml`) and its section in `docs/spec/`. Read the
   module you will change and its tests.
2. **Plan.** For anything larger than one command, write the task list first (in
   `docs/plans/M<n>.md`): the commands, the model changes, the tests, the oracle comparisons, what
   is out of scope. Settle the open questions that block you (§ 3).
3. **Implement with tests.** Model first, then the pure function that does the work, then the
   command that calls it, then the UI that calls the command. § 2 lists the tests a command
   needs. While working, run `cargo check` or the tests of the crate you changed.
4. **Verify.** `cargo xtask ci`, once, when the task or batch is done. For UI work, look at the
   result (a headless frame test now; a screenshot over the control channel from M3). For
   compiler work, run the oracle comparison.
5. **Record.** Fix inventory rows that turned out wrong. Run `cargo xtask parity`. Update
   `ROADMAP.md` (milestone row, parity-table row, "Shipped so far") and `docs/status.md`.
6. **Commit.** One task per commit, named by its id: `M3.4: knife tool`. The tree is green at
   the end of every task or batch.

## 2. Adding a command: the checklist

- [ ] Its row exists in `spec/inventory/*.toml` (add it if it is our own design, `status = "inferred"`).
- [ ] Every choice of ours where no source says what the reference does is a sentence in the
      row's `guesses`; a live `inferred` or `memory` command needs at least one, unless it has no
      counterpart in the reference app (`own = true`).
- [ ] The work is a function in the right lower-layer crate.
- [ ] `cmd!(…)` in `crates/engine/src/cmd/<area>.rs`: the id from the inventory, label, menu
      path, shortcut, a params doc that starts with `{…}` and says what is returned after `→`.
- [ ] Parameters read through `Params`; every count and size capped; targets explicit.
- [ ] Fonts change only inside `Session::edit`.
- [ ] Tests: the behaviour; undo restores the font exactly; bad parameters are errors that name
      the parameter; the command sweep passes without special-casing.
- [ ] If the junk-parameter generator has no value that could break your parameter kind, add one
      to `fontcraft_testkit::junk_values`.
- [ ] `cargo xtask parity`, `cargo xtask guesses` and `cargo xtask observe` (commit the regenerated
      `docs/parity.md`, `docs/guesses.md` and `docs/observe.md`).

## 3. Planning a milestone

Before a milestone's first task, one planning pass turns its notes into a task list. This is where
depth is added, just in time: the inventory is complete in breadth, and each milestone's rows are
made exact when they are about to be built.

1. List the milestone's rows (`docs/parity.md` › By milestone gives the counts).
2. Go through the open questions of its notes. For each: answer it by observation if the owner
   has the reference app or a tutorial video shows it (set `status = "observed"`), by an oracle if it is about output, or by a
   decision of ours recorded in the note and in the row's `guesses`. A question that cannot be answered and blocks nothing
   stays open.
3. Verify every `memory` row or drop it.
4. Group rows into packages that touch disjoint crates and files (§ 5), ordered by dependency.
5. Write the plan to `docs/plans/M<n>.md` with measurable done-criteria per package, and review it against
   the notes for missing depth before coding. (The sibling apps found that this review is where
   most of the missing work is discovered.)

## 4. Milestone order and what unlocks what

```
M0 skeleton ─► M1 model + source formats ─► M2 compile core + oracle harness ─┬─► M8 features ──► M9 variable
                    │                                                         ├─► M10 hinting
                    │                                                         ├─► M11 colour
                    │                                                         └─► M12 CLI suite + checks
                    └─► M3 drawing ─► M4 font view ─► M5 spacing ─► M6 components ─► M7 font info
                                                                         M13 plug-ins · M14 polish
```

Two tracks run side by side from M1: the **engine track** (formats, compiler, CLI: checkable
against oracles without the reference app) and the **editor track** (drawing, views, panels:
needs the reference app's behaviour). If the owner cannot observe the reference (open decision
O1), put the agents on the engine track first; it is where correctness can be proved.

## 5. Parallel agents

The layering makes crates independent; the shared files are few. Rules, as in the sibling apps:

- Own `CARGO_TARGET_DIR` per agent (`target/agent-<n>`); delete it when done (they are large).
- An agent owns named crates or modules for a package and edits nothing else.
- New commands go in a **new module** (`cmd/<area>.rs` with `specs()`), not into a shared file.
  The shared files get one-line edits, re-read before editing: `cmd/mod.rs` (the `v.extend` list),
  `xtask/src/layers.rs` (the table), `crates/ui/src/shell/commands.rs` (`MODULES`), root `Cargo.toml`.
- Each agent edits its own inventory files (one per chapter), never another's.
- Create or rewrite a `Cargo.toml` atomically (write elsewhere, then move): one broken manifest
  breaks every build in the workspace.
- If someone else's change breaks the build, wait and retry; do not fix their files.
- Integration is serial: one agent merges packages, runs `cargo xtask ci`, regenerates
  `docs/parity.md`.

Packages that split well: one table builder each (M2), one importer each (M1), one tool each (M3),
one filter each (M3), one check group each (M12), one CLI subcommand each (M12).

## 6. No time estimates

Do not estimate agent hours, for packages, milestones or the whole project: such estimates have
been far off every time. A plan states scope, order, dependencies and how "done" is measured
(a test, an oracle comparison, a round-trip count). Progress is reported by those measures and by
the parity table.

## 7. Decisions that are the owner's

Stop and ask for these; decide everything else and record it.

- Anything in `docs/charter.md` › "Decisions that are the owner's": the name, the repository's
  home, joining another organisation, a non-permissive dependency, relaxing a non-negotiable,
  anything needing a licence to or the files of a reference app.
- Publishing anything outside this repository: a release, a package, a post, an issue or message
  on someone else's project.
- Deleting or rewriting history, or discarding another agent's or the owner's uncommitted work.

## 8. When things go wrong

- **A panic in a test or a sweep.** Fix the code, add the input as a regular test, and note the
  cause in the commit. If a dependency panicked, guard the input and add the limit to
  `docs/architecture.md` › Never crash.
- **An oracle disagrees.** Find out which is right against the format specification. If we are
  wrong, fix it. If the oracle is wrong or the difference is a legitimate choice, record it in
  `docs/spec/40-compile.md` § 4 and in the comparison's allow-list with the reason.
- **The documentation contradicts itself or is silent.** Record it under the note's open
  questions, choose the conservative behaviour, mark the row `inferred`.
- **You saw something you should not have** (copyleft source, a proprietary app's internals).
  Say so in `docs/status.md`, do not work on that feature, and tell the owner.
