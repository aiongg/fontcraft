# FontCraft

A professional font editor and command-line font tool suite: open source, written from scratch in
Rust, and drivable by AI agents as fully as by people. It follows the conventions of the ArtCraft
Crafting Apps (VectorCraft, PhotoCraft and their siblings) and aims at parity with the leading
commercial font editor and with the standard open font-engineering tools.

**Status: `.glyphs`, UFO and designspace sources open and save, compiled fonts open, and static fonts and variable TrueType and CFF2 fonts compile with overlaps removed. Edit View, Font View, spacing, kerning and text mode (M3 to M5) are built, and the first waves of M6 to M13 add components, Font Info, feature code (compiled into exported fonts), variable-font tables, hinting, colour layers, command-line table tools, quality checks, font subsetting, a plug-in module runtime, a command console and macros, though much of that does not yet reach exported fonts. 917 of 1004 commands run and feature parity is about 36% by our own grading; nothing is yet compared with the reference app.** See
[ROADMAP.md](ROADMAP.md) for the honest assessment.

## What is here

| | |
|---|---|
| **The plan** | [charter](docs/charter.md) · [architecture](docs/architecture.md) · [decisions](docs/decisions/) · [execution plan](docs/execution-plan.md) · [testing and oracles](docs/testing.md) · [clean-room policy](docs/clean-room.md) · [references](docs/references.md) |
| **The specification** | [`spec/inventory/`](spec/): 3,618 machine-readable rows covering every command, tool, panel, setting, custom parameter, quality check, CLI subcommand, file format and OpenType table we mean to support. [`docs/spec/`](docs/spec/README.md): 22 behaviour notes written from public documentation, each ending with its open questions |
| **The skeleton** | A Rust workspace that passes its own gate: the font model, the native file format, a static TrueType compiler, the engine with its command registry and undo, a CLI, a JSON control channel, an MCP server and an egui desktop shell |
| **The gate** | `cargo xtask ci`: formatting, lints that forbid panics, tests, the inventory check, the parity report, asset and vendor-name checks, crate layering, a WebAssembly build |

## Try it

```sh
cargo xtask ci                      # everything below, checked

# headless: draw a triangle in "A", set its width, save the source, export a font
cargo run -p fontcraft-cli -- run \
  --cmd path.addPath --params '{"glyph":"A","nodes":[{"x":60,"y":0},{"x":560,"y":0},{"x":310,"y":700}]}' \
  --cmd spacing.setWidth --params '{"glyphs":["A"],"value":620}' \
  --save demo.fontcraft --export demo.ttf

cargo run -p fontcraft-cli -- commands            # every engine command, with its parameters
cargo run -p fontcraft -- demo.fontcraft --control 7985    # the window, drivable over 127.0.0.1:7985
cargo run -p fontcraft-cli -- mcp                 # MCP server: attaches to the window, or runs headless
```

Use it from Claude Code and other agents:

```sh
cargo build --release -p fontcraft-cli
claude mcp add fontcraft -- "$PWD/target/release/fontcraft-cli" mcp
```

Details: [`docs/development.md`](docs/development.md), [`docs/mcp.md`](docs/mcp.md),
[`docs/control-protocol.md`](docs/control-protocol.md).

## How it is built

- **Everything is a command.** One registry of commands with ids, parameter descriptions and JSON
  results. The menus, the CLI, the control channel and the MCP server all run the same ones.
- **Spec first.** A command has a row in the inventory before it has code; `cargo xtask parity`
  reports how much of the inventory exists.
- **Never crash.** No panics in shipped code; a command that fails changes nothing; every command
  is swept with hostile parameters on every test run.
- **Checked against oracles.** `fontcraft-cli oracle` compares compiled fonts, table by table,
  with what the open reference compilers produce from the same source
  ([testing](docs/testing.md)). Over the 354-source corpus, every core table matches them or
  has a written reason.
- **Clean-room.** Specified from public documentation; no code, prose or assets copied from any
  reference ([policy](docs/clean-room.md)).
- **Rust only.** Built and run on Linux so far; macOS, Windows and the browser are targets (the
  library crates already build for WebAssembly).

## For agents and contributors

Start with [`AGENTS.md`](AGENTS.md), then [`docs/status.md`](docs/status.md).

## License and credits

Code and original assets are dual-licensed under MIT or Apache-2.0, at your option
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE)).

FontCraft is an independent project that follows the ArtCraft Crafting Apps' conventions; it is
not part of that organisation. Product names in the documentation are trademarks of their owners
and are used only to identify what FontCraft is compared with or interoperates with.
