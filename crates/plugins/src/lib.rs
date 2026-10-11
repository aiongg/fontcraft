//! FontCraft modules (L2): sandboxed WebAssembly extensions of three kinds, run under resource
//! limits. Plan: `docs/plans/M13.md` § 3 (the extension model, ABI v1, limits) and § 4 (M13.2
//! to M13.6). Decision: `docs/decisions/0005-*.md` (extensions are wasm modules and commands).
//!
//! A module is a WebAssembly binary that exports the FontCraft module ABI v1:
//! `memory`, `fc_abi_version() -> i32` (returns [`ABI_VERSION`]), `fc_manifest() -> i64`,
//! `fc_alloc(size) -> ptr` and `fc_run(input, len, params, len) -> i64`. A returned `i64` packs a
//! pointer (high 32 bits) and a length (low 32 bits) of UTF-8 JSON in the module's memory. It runs
//! in [`wasmi`] with no host imports (a module that imports anything is refused), an instruction
//! budget per call, a wall-clock deadline on native builds, a memory cap, a call-depth cap and a
//! fresh instance per run, so no state survives between runs.
//!
//! | Kind | Input | Output | Used by |
//! |---|---|---|---|
//! | filter | one layer as [`layer_json::LayerInput`], parameters | [`layer_json::FilterOutput`]: the layer's new paths, components, anchors, log lines | `plugin.runFilter`, the `Filter` and `PreFilter` export parameters, Palette forms |
//! | reporter | one layer, view scale, active flag | [`reporter::Drawing`]: background and foreground primitives | `view.showReporter`, the canvas |
//! | check | one layer | [`check::Finding`]s | `audit.run` |
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`error`] | M13.1 | [`Error`], every limit of § 3, [`LIMITS_TABLE`] |
//! | [`manifest`] | M13.2 | the manifest (`fc_manifest`) and its parameter schema |
//! | [`runtime`] | M13.2 | [`Plugin`]: loading, validation, one call under [`Limits`] |
//! | [`layer_json`] | M13.2 | the layer JSON handed to modules; a filter's output checked and applied to a layer |
//! | [`reporter`] | M13.2 | reporter drawings, capped, and their cache |
//! | [`check`] | M13.2 | check findings, capped |
//! | [`registry`] | M13.2, M13.3 | the installed modules of a session, folder scans from bytes |
//! | [`export_filter`] | M13.4 | the `Filter` / `PreFilter` parameter string: name, arguments, `include:` and `exclude:` |
//!
//! Who calls what: the engine keeps one [`registry::Registry`] in its `ExtensionState` (per
//! process, not per document). Commands build a [`layer_json::LayerInput`], call
//! [`Plugin::call`] and parse the result with the kind's parser; nothing here touches files,
//! the network or the interface. Every failure is an [`Error`], never a panic.
#![forbid(unsafe_code)]

pub mod check;
pub mod error;
pub mod export_filter;
pub mod layer_json;
pub mod manifest;
pub mod registry;
pub mod reporter;
pub mod runtime;

pub use error::{Error, LIMITS_TABLE, Result};
pub use manifest::{Kind, Manifest, ParamSpec};
pub use registry::{FolderReport, Installed, Registry};
pub use runtime::{ABI_VERSION, Limits, Plugin};

#[cfg(test)]
mod tests;
