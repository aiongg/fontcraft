# Development

## Build, test, run

```sh
cargo xtask ci                                   # the gate (see below)
cargo test -p fontcraft-engine                   # one crate
cargo test -p fontcraft-engine --test engine kerning_pairs::   # one module of the crate's integration tests
cargo test --profile measure -p fontcraft-oracle --features oracle --test kern -- --ignored   # speed measures and oracle runs
cargo run -p fontcraft                           # the desktop app, with a new font
cargo run -p fontcraft -- file.fontcraft --control 7985   # + the JSON control channel
cargo run -p fontcraft-cli -- commands           # the command catalogue as JSON
cargo run -p fontcraft-cli -- run --cmd glyph.newGlyph --params '{"name":"A.alt"}' --export out.ttf
cargo run -p fontcraft-cli -- mcp --headless     # the MCP server on stdio
cargo xtask testfonts [--out DIR]                # synthetic test fonts as .glyphs files for the reference-app checks (default plan/local/testfonts/)
```

Speed measures and oracle runs use `--profile measure` (release optimisation, no link-time
optimisation, 16 codegen units, incremental), which rebuilds in a fraction of the release time.
Shipping builds use `--release`.

Rust 1.95 or newer (edition 2024), as in the sibling apps. `rustup target add
wasm32-unknown-unknown` once, for `cargo xtask wasm`. Nothing else is needed: no system
libraries, no Python, no network at build time beyond crates.io.

The first build of the desktop app compiles the windowing and GPU stack and takes several minutes
(over ten on a two-core machine). The libraries and the CLI do not depend on that stack.

## The gate

`cargo xtask ci` runs, in order, and stops at the first failure:

| Step | What | Fails when |
|---|---|---|
| `fmt` | `cargo fmt --all -- --check` | code is not formatted (`rustfmt.toml`: 150 columns) |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | any warning, including the no-panic lints (`unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, indexing and slicing) |
| `test` | `cargo test --workspace` | a test fails |
| `spec` | `cargo xtask spec` | the inventory breaks a rule of `spec/README.md` |
| `parity` | `cargo xtask parity --check` | a registered command has no inventory row; a command is engine in one place and UI in the other; `docs/parity.md` is stale |
| `guesses` | `cargo xtask guesses --check` | a live `inferred` or `memory` command has no `guesses` entry and no `own = true`; `docs/guesses.md` is stale |
| `observe` | `cargo xtask observe --check` | `docs/observe.md` is stale |
| `assets` | `cargo xtask assets` | an asset (an image anywhere in the tree, or any file under `assets/`) has no row in `ASSETS.md`; a row names a file that is gone; a compiled font is in the tree, whatever its extension |
| `brands` | `cargo xtask brands` | user-visible text names another vendor's product |
| `layers` | `cargo xtask layers` | a crate depends upward, a UI crate is used below L6, a crate is not registered |
| `wasm` | `cargo xtask wasm` | a library crate does not build for the browser |

Run a step alone with `cargo xtask <step>`. After adding or renaming a command run `cargo xtask
parity` and commit `docs/parity.md`; after changing a row's `guesses` run `cargo xtask guesses` and
commit `docs/guesses.md`; after changing a row's `guesses`, `observe` or `status` run `cargo xtask
observe` and commit `docs/observe.md`.

## Repository layout

```
AGENTS.md  CLAUDE.md      rules for agents (short; read first)
README.md  ROADMAP.md     what it is; where it stands
ASSETS.md  NOTICE         every asset's origin and licence; third-party notices
crates/                   libraries, by layer (docs/architecture.md)
apps/fontcraft            the desktop app (eframe) and the control server
apps/fontcraft-cli        the command-line tool and MCP entry point
xtask/                    the gate, the parity report, the corpus fetcher
spec/                     the feature inventory as data
docs/                     charter, architecture, decisions, the specification, guides
docs/spec/                behaviour notes, one per source chapter or tool family
corpus/  plan/local/  log/   git-ignored: fetched test fonts, reference screenshots, local notes
```

## Never crash: how to write code here

Shipped code does not panic. The lints catch the explicit forms; these are the implicit ones:

| Instead of | Write |
|---|---|
| `x.unwrap()`, `x.expect("…")` | `x?`, `x.ok_or(err)?`, `let Some(v) = x else { return … }`, `unwrap_or*` |
| `v[i]`, `&s[a..b]` (banned by lint, whatever the index) | `v.get(i)`, `s.get(a..b)`, iterators, `split_first` |
| `a / b`, `a % b` on integers | check `b != 0`; `checked_div` |
| `n - 1` on unsigned values | `saturating_sub`, or compare `i + 1 == n` |
| `x as u16` from a float | check `is_finite`, clamp, then cast (see `i16_of` in `crates/compile`) |
| a loop or allocation sized by input | cap it with a named constant |
| recursion over document structure | a depth limit (see `MAX_COMPONENT_DEPTH`) |
| `unreachable!()` in a match arm | return an error; the arm you believe impossible will be reached by a fuzzer |

Tests may unwrap and panic (`clippy.toml`). Integration tests under `tests/` and the test kit
start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`. `xtask` is build
tooling and does not take the workspace lints.

Don't silence a lint by discarding an error. `let _ = …` and `.ok()` are for failures that
truly do not matter, with a comment saying why.

A dependency can panic on input you hand it. When a test finds one, validate before the call, name
the limit, and add it to `docs/architecture.md` › Never crash.

## Vendor names

`cargo xtask brands` scans Rust string literals (outside comments and tests), `Cargo.toml`
descriptions and `packaging/`. Say "the reference app" or name the feature. Markdown and `spec/`
are not scanned: they are where the references are named. A line that must keep a name carries a
`brand-ok` comment with the reason.

## Working with the inventory

```sh
grep -n 'id = "kerning\.' spec/inventory/*.toml          # the rows of an area
cargo xtask spec                                         # check the files
cargo xtask parity                                       # regenerate docs/parity.md
```

Rows are TOML blocks separated by blank lines; edit them by hand. One file per source chapter, so
parallel agents do not collide. When you change behaviour, change the row's `summary` and
`status`; when you confirm a row against the running reference app, on our own fonts or in a
public tutorial video, set `status = "observed"` and name the video in `notes`.

## Driving the app

See [`control-protocol.md`](control-protocol.md). A minimal session:

```sh
cargo run -p fontcraft -- --control 7985 &
printf '%s\n' '{"id":1,"method":"engine.execute","params":{"command":"glyph.newGlyph","params":{"name":"A.alt"}}}' | nc 127.0.0.1 7985
```

Without a display (CI, a container), the app runs on a virtual one:
`xvfb-run -a cargo run -p fontcraft -- --control 7985`. The control channel works the same.

## Parallel agents

`docs/execution-plan.md` § 5. In short: your own `CARGO_TARGET_DIR`, your own crates and inventory
files, new commands in new modules, atomic manifest writes, clean up your target directory.

## Not here yet

Listed so nobody looks for them: the web app crate and its `trunk` build, packaging and release
scripts, localisation catalogues, the performance suite, property-test dependencies, a CI workflow
file (the gate is one command; wire it to whichever CI the repository's home provides).
