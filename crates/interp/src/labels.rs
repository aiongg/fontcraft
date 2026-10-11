//! The names special layers are shown with in the Layers palette and elsewhere (M9.16; the
//! palette that shows them is M9.21's).
//!
//! Rows: `intermediateLayer.braceLabel`, `alternateLayer.bracketLabel`,
//! `alternateLayer.axisAbbreviations`.
//!
//! Contract: pure text functions of the font and a layer. An intermediate layer reads
//! `{coordinate, …}`, an alternate layer `[range …]` with each axis abbreviated, a layer that is
//! both shows both. Non-special layers have no label.

use fontcraft_model::{Axis, Font, Layer};

/// The intermediate-layer label of `layer` (`{400}`, `{400, 100}`); `None` when it is not an
/// intermediate layer.
pub fn brace_label(font: &Font, layer: &Layer) -> Option<String> {
    // M9.16
    let _ = (font, layer);
    None
}

/// The alternate-layer label of `layer` (`[600‹wg]`); `None` when it is not an alternate layer.
pub fn bracket_label(font: &Font, layer: &Layer) -> Option<String> {
    // M9.16
    let _ = (font, layer);
    None
}

/// The short form of `axis` used in alternate-layer labels.
pub fn axis_abbreviation(axis: &Axis) -> String {
    // M9.16
    axis.tag.clone()
}
