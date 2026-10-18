# Module templates

One Rust crate for each kind of FontCraft module. Each crate builds on its own; copy the folder
of the kind you need and start from there.

| Folder | Kind | What the example does |
|---|---|---|
| `filter/` | filter | moves the outline, components and anchors by `dx`, `dy`; only the selected nodes when there is a selection |
| `reporter/` | reporter | draws a dot on each on-curve node, a line at the x-height and the node count |
| `check/` | check | reports open paths and nodes off a grid |

Each crate has the same layout:

| File | What to do with it |
|---|---|
| `manifest.json` | give it your own `id`, `name`, `description` and parameters |
| `src/lib.rs` | the module's behaviour: JSON in, JSON out; rewrite it |
| `src/abi.rs` | the four functions the host calls; keep it as it is |
| `Cargo.toml` | rename the crate; keep `crate-type = ["cdylib"]` and the empty `[workspace]` table |

## Build

```sh
rustup target add wasm32-unknown-unknown
cd examples/module-template/filter
cargo test                                               # the logic, on your own machine
cargo build --release --target wasm32-unknown-unknown
# → target/wasm32-unknown-unknown/release/fontcraft_filter_template.wasm
```

Build for `wasm32-unknown-unknown`, not a WASI target: FontCraft gives modules no imports, and a
module that imports anything is refused.

## Try it

```sh
fontcraft-cli run --in MyFont.glyphs \
  --cmd plugin.installFromFile --params '{"path": "target/wasm32-unknown-unknown/release/fontcraft_filter_template.wasm"}' \
  --cmd plugin.runFilter --params '{"id": "org.example.shift", "glyphs": ["A"], "params": {"dx": 20}}' \
  --save Shifted.glyphs
```

In the desktop app, drop the `.wasm` file on the window, or put it in the Modules folder. The
author guide, [`docs/plugins.md`](../../docs/plugins.md), has the ABI, the input and output
formats, the limits and the ways to install and debug a module.
