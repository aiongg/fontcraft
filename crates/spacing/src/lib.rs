//! FontCraft spacing (L2): metrics keys, sidebearings, the out-of-sync scan, linked metrics and
//! kerning.
//! Pure functions over a [`fontcraft_model::Font`]; no I/O. Behaviour notes:
//! `docs/spec/08-spacing-kerning.md` § 2; rows `metricsKey.*` in
//! `spec/inventory/08-spacing-kerning.toml`.
//!
//! # Metrics keys ([`keys`])
//! A key is the text a sidebearing or width field holds instead of a number. [`parse_field`]
//! reads what a user typed (a plain number, a bare glyph name, `=…` or `==…`); [`parse_stored`]
//! reads a key as the model stores it (on the glyph for glyph-level keys, on the layer, without
//! `==`, for local ones). [`Key::display`] gives the field's text, [`Key::stored`] the stored
//! text. A glyph name inside a key is the longest leading part that names a glyph of the font,
//! so `=ka-deva/1.4` reads `ka-deva` even when `ka` exists.
//!
//! # Sidebearings ([`measure`])
//! [`metric`] measures LSB, RSB, TSB and BSB from the bounds of a layer's outline with its
//! components resolved, and gives the width; [`sidebearings_at_height`] measures from the first
//! and last crossing of a horizontal line. An empty layer has no sidebearings.
//!
//! # Evaluation ([`eval`])
//! [`effective_key`] is the key that applies to one field of one layer (the layer's own key wins
//! over the glyph's). [`evaluate`] gives the value of that key, [`evaluate_key`] of any parsed
//! key in the same context. A key reads the referenced glyph's *current* metric; keys are not
//! live, so a chain settles when Update Metrics runs in [`update_order`]. The referenced layer is
//! the referenced glyph's layer with the same id, else its master layer of the same master.
//! Results round halves up. Errors ([`EvalError`]): a missing glyph (the key is unlinked),
//! division by zero, a chain that returns to its start or runs deeper than [`MAX_CHAIN`] steps,
//! and a result outside [`WIDTH_RANGE`] or [`SIDEBEARING_RANGE`].
//!
//! # Out of sync ([`sync`])
//! [`scan`] lists every key whose value differs from the layer's metric, every unlinked key and
//! every key that cannot be evaluated. [`SyncCache`] keeps that result across edits and
//! recomputes only the glyphs whose `Arc` changed and the glyphs whose keys read them (a reverse
//! index). [`update_order`] puts the metrics Update Metrics will set in dependency order.
//!
//! # Linked metrics ([`link`])
//! [`metrics_source`] is the master whose widths and kerning a master shows, following the
//! *Link Metrics With First Master* and *Link Metrics With Master* parameters.
//!
//! # Kerning ([`kerning`])
//! Command sides and stored sides, the pair that applies to two glyphs with the step that
//! supplied it ([`kerning::KernIndex`]), lock states derived from the pairs, and kerning groups.
//! The module's own doc comment lists its API.
//!
//! # Kerning text ([`kerning_text`])
//! Our tab-separated format for one master's pairs, groups and metrics: [`kerning_text::write`]
//! and [`kerning_text::read`], with limits on hostile input; [`kerning_text::from_master`] takes
//! a master's pairs and groups. The module doc describes the lines.
//!
//! The public API:
//! ```text
//! parse_field(text: &str, field: Field, exists: impl Fn(&str) -> bool) -> Result<Entry, KeyError>
//! parse_stored(text: &str, scope: Scope, field: Field, exists: impl Fn(&str) -> bool) -> Result<Key, KeyError>
//! Key::display(&self) -> String;  Key::stored(&self) -> String;  Key::glyph(&self) -> Option<&str>
//! metric(font: &Font, layer: &Layer, field: Field) -> Result<Option<f64>, EvalError>
//! sidebearings_at_height(font: &Font, layer: &Layer, y: f64) -> Result<Option<(f64, f64)>, EvalError>
//! effective_key<'a>(glyph: &'a Glyph, layer: &'a Layer, field: Field) -> Option<(Scope, &'a str)>
//! evaluate(font: &Font, glyph: &str, layer: &str, field: Field) -> Result<Option<f64>, EvalError>
//! evaluate_key(font: &Font, glyph: &str, layer: &str, field: Field, key: &Key) -> Result<f64, EvalError>
//! scan(font: &Font) -> Vec<Finding>;  scan_glyph(font: &Font, glyph: &Glyph) -> Vec<Finding>
//! SyncCache::update(&mut self, font: &Arc<Font>);  SyncCache::glyph(&self, name: &str) -> &[Finding]
//! SyncCache::all<'a>(&'a self, font: &'a Font) -> impl Iterator<Item = &'a Finding>
//! update_order(font: &Font, slots: &[Slot]) -> Vec<Slot>
//! metrics_source<'a>(font: &'a Font, master: &str) -> Option<&'a Master>
//! kerning_text::read(text: &str) -> Result<KerningText, ReadError>
//! kerning_text::write(doc: &KerningText) -> Result<String, WriteError>
//! kerning_text::from_master(font: &Font, Direction, master: &str) -> KerningText
//! ```
#![forbid(unsafe_code)]

pub mod afm;
pub mod eval;
pub mod kerning;
pub mod kerning_text;
pub mod keys;
pub mod link;
pub mod measure;
pub mod pfm;
pub mod profile;
pub mod sync;

#[cfg(test)]
mod tests;

pub use eval::{EvalError, effective_key, evaluate, evaluate_key};
pub use keys::{Body, Entry, Key, KeyError, KeyErrorKind, Op, Operation, Scope, parse_field, parse_stored};
pub use link::{LINK_WITH_FIRST_MASTER, LINK_WITH_MASTER, metrics_source};
pub use measure::{metric, sidebearings_at_height};
pub use sync::{Finding, Problem, Slot, SyncCache, scan, scan_glyph, update_order};

use fontcraft_model::{Glyph, Layer, MetricsKeys};

/// Longest metrics key, in bytes.
pub const MAX_KEY_LEN: usize = 256;
/// Most references one key's chain may follow.
pub const MAX_CHAIN: usize = 32;
/// Largest number a key may hold (an operand, a constant or an `@` height), in absolute value.
pub const MAX_KEY_NUMBER: f64 = 1_000_000.0;
/// The values a width key may give.
pub const WIDTH_RANGE: std::ops::RangeInclusive<f64> = 0.0..=65_535.0;
/// The values a sidebearing key may give.
pub const SIDEBEARING_RANGE: std::ops::RangeInclusive<f64> = -32_768.0..=32_767.0;

/// A metric a key can set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    Left,
    Right,
    Width,
    Top,
    Bottom,
}

impl Field {
    pub const ALL: [Field; 5] = [Field::Left, Field::Right, Field::Width, Field::Top, Field::Bottom];

    /// The other side: left and right, top and bottom. The width is its own.
    pub fn opposite(self) -> Field {
        match self {
            Field::Left => Field::Right,
            Field::Right => Field::Left,
            Field::Width => Field::Width,
            Field::Top => Field::Bottom,
            Field::Bottom => Field::Top,
        }
    }

    /// The name used in parameters and in [`MetricsKeys::all`]: `left`, `right`, `width`, `top`,
    /// `bottom`.
    pub fn name(self) -> &'static str {
        match self {
            Field::Left => "left",
            Field::Right => "right",
            Field::Width => "width",
            Field::Top => "top",
            Field::Bottom => "bottom",
        }
    }

    pub fn from_name(name: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.name() == name)
    }

    fn is_sidebearing(self) -> bool {
        self != Field::Width
    }

    /// This field's key in a set of keys.
    pub fn key_in(self, keys: &MetricsKeys) -> Option<&str> {
        match self {
            Field::Left => keys.left.as_deref(),
            Field::Right => keys.right.as_deref(),
            Field::Width => keys.width.as_deref(),
            Field::Top => keys.top.as_deref(),
            Field::Bottom => keys.bottom.as_deref(),
        }
    }
}

/// The layer of `glyph` a key on `from` reads: the layer with the same id, else the master layer
/// of `from`'s master.
pub fn referenced_layer<'a>(glyph: &'a Glyph, from: &Layer) -> Option<&'a Layer> {
    glyph.layers.iter().find(|l| l.id == from.id).or_else(|| glyph.layer(&from.master))
}

/// Rounds half up (toward positive infinity), as both reference compilers round kerning.
pub fn round_half_up(v: f64) -> f64 {
    (v + 0.5).floor()
}
