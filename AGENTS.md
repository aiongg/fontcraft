# FontCraft: instructions for agents

FontCraft is a clean-room, open-source, Rust-only professional font editor and command-line font
tool suite. It follows the conventions of the ArtCraft Crafting Apps (VectorCraft, PhotoCraft…):
every action is a command, and the UI, the CLI, a JSON control channel and an MCP server all run
the same commands. The primary reference for the editor is Glyphs 4; for the command line, AFDKO
(fonttools, fontmake and fontc are oracles). Code and original assets are MIT OR Apache-2.0.

## Start every session here
1. Read [`docs/status.md`](docs/status.md): where the work stands and what to pick up.
2. Read the honest assessment in [`ROADMAP.md`](ROADMAP.md). Unless the user gives you a task,
   take the next one from its "What to build next" list.
3. Before coding a feature, read its rows in `spec/inventory/*.toml` and its notes in
   `docs/spec/`, then follow [`docs/execution-plan.md`](docs/execution-plan.md): orient → plan →
   implement with tests → verify → record → commit.

Deeper reading, on demand: [`docs/architecture.md`](docs/architecture.md) (crates, layers, the
model, commands), [`docs/decisions/`](docs/decisions/) (why things are the way they are),
[`docs/testing.md`](docs/testing.md) (oracles and corpora), [`docs/development.md`](docs/development.md).

## Non-negotiables
- **Clean-room.** Behaviour comes from public documentation and from watching the reference apps
  run on our own synthetic fonts. Never read, disassemble or copy anything inside a proprietary
  app. Never read or copy GPL/AGPL source (FontForge, Fontra, Inkscape…); their public user docs
  are fine. Permissive code (Apache/MIT/BSD: AFDKO, fonttools, fontc, fontations) may be read and
  ported with attribution in the file header and `NOTICE`. Write every sentence yourself: feature
  names, menu labels, shortcuts and file-format keys may match a reference; prose may not. Full
  rules: [`docs/clean-room.md`](docs/clean-room.md).
- **Assets.** Nothing from a reference app, ever. Every asset has a row in
  [`ASSETS.md`](ASSETS.md). No compiled font is committed anywhere: build test fonts in code, fetch
  real ones with `cargo xtask corpus --fetch`. Reference screenshots stay in the git-ignored
  `plan/local/`.
- **Never crash.** A crash loses unsaved work, and files, pasted data, MCP and control messages
  and command parameters are all untrusted.
  - Fallible work returns `Result`; failures reach the user as a message, never as an abort.
  - Workspace lints ban `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` and
    `unimplemented!` outside tests. Use `?`, `ok_or`, `let … else`, `if let`, `unwrap_or*`.
  - Indexing and slicing (`v[i]`, `&s[a..b]`) are banned by lint too: use `.get(…)`.
  - Not caught by lints: integer division by something that can be zero, unsigned subtraction,
    counts and sizes read from input (cap every one with a named limit), floats that are not
    finite, recursion without a depth limit, work that grows faster than its input.
  - A dependency can panic, or allocate without bound, on what you hand it. Validate first (see
    the limits at the top of `crates/model` and `crates/compile`); every entry point also runs
    under `fontcraft_engine::guard`, which cannot catch an out-of-memory abort.
  - Every panic fix comes with a test that panicked before it.
- **Everything is a command.** User-visible behaviour is a command in `crates/engine/src/cmd/`
  (id, label, menu path, shortcut, params doc, `enabled`, `run`) with behaviour tests. Commands
  take explicit targets in their parameters; none depends on UI state. A command that fails
  changes nothing. View and window state only: `UI_COMMANDS` in `crates/ui-egui/src/menus.rs`.
- **Spec first.** A command must have a row in `spec/inventory/*.toml` before or with its code
  (`cargo xtask parity` fails otherwise). When behaviour turns out different from a row, fix the
  row in the same change.
- **Layering** is enforced by `cargo xtask layers`. Nothing below L6 depends on egui, eframe,
  winit or rfd. Register a new crate in `xtask/src/layers.rs`.
- **The UI is thin.** Panels read engine state and act through `app.run(id, params)`. Colours and
  sizes come from `theme::Tokens`.
- **Rust only**, no handwritten JS/TS, no webview shells. **Never break wasm** (`cargo xtask wasm`).
- **No vendor names in user-visible text** (labels, params docs, MCP tools, CLI help):
  `cargo xtask brands` fails on them. Say "the reference app". Markdown docs and `spec/` may name
  the references.
- **Oracles decide compiler questions.** Where FontCraft and a reference compiler disagree on a
  table, the difference is either fixed or written down in `docs/spec/40-compile.md` with the reason.
- **The gate**: `cargo xtask ci` (fmt, clippy with the no-panic lints, tests, spec, parity,
  guesses, assets, brands, layers, wasm), run once when a task or batch is done, or when the brief says.
  While working, `cargo check` and the tests of the crate you changed are enough. One task per
  commit, named by its id (`M1.3: …`).

## Running and looking at the app
- `cargo run -p fontcraft -- --control 7985 [file.fontcraft]`: the desktop app with the control
  channel. The sibling apps default to port 7979; if the log says the port could not be bound,
  pick another, or your requests reach a different app.
- Drive it with JSON lines on `127.0.0.1:7985`, e.g.
  `{"id":1,"method":"engine.execute","params":{"command":"glyph.newGlyph","params":{"name":"A.alt"}}}`.
  Methods: `docs/control-protocol.md`.
- Headless: `cargo run -p fontcraft-cli -- run --cmd path.addPath --params '{…}' --export out.ttf`,
  `fontcraft-cli commands`, `fontcraft-cli mcp --headless` (`docs/mcp.md`).
- For UI work, look at the result: until `ui.screenshot` lands (M3), cover panels with a headless
  frame test (`crates/ui-egui/src/tests.rs`).
- Parallel agents: one `CARGO_TARGET_DIR` each, edit only the crates you own, put new commands in
  a new `cmd/<area>.rs` module, write manifests atomically, delete your target dir when done.

## Keeping the picture honest
- `ROADMAP.md` is the shared picture: update its milestone row, parity-table row and "Shipped so
  far" in the same change that lands a task. Grade by behaviour against `docs/spec/`, not by
  whether a command exists, and err low.
- No time estimates: never estimate agent hours (`docs/execution-plan.md` § 6). Report progress
  by what works and by the parity table.
- `docs/status.md` is the hand-over note: sessions end abruptly, so a green tree plus that file is
  how the next agent picks up. Update it before you stop.
- A row whose `status` is `memory` or `inferred`, and everything under "Open questions" in
  `docs/spec/`, has not been confirmed. Don't treat it as fact; mark it `observed` once checked
  against the running reference app, on our own fonts or in a public tutorial video named in the
  row's `notes`.
- A choice of ours where no source says what the reference does goes into the row's `guesses`
  list, one sentence each; `docs/guesses.md` is generated from them (`cargo xtask guesses`).
