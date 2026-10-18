//! Cursive attachment from live anchors (rows `text.shaping`, `features.previewScope`): in
//! right-to-left Edit View text, a glyph's `entry` anchor is placed on the `exit` anchor of the
//! glyph before it, so anchor edits show at once, without compiling the features.

use fontcraft_model::Layer;

/// The anchor a glyph's instroke starts at.
pub const ENTRY: &str = "entry";
/// The anchor a glyph's outstroke ends at.
pub const EXIT: &str = "exit";

/// The layer's `entry` and `exit` anchors, when finite.
pub(crate) fn anchors(layer: Option<&Layer>) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
    let Some(layer) = layer else { return (None, None) };
    let find = |name: &str| layer.anchors.iter().find(|a| a.name == name && a.x.is_finite() && a.y.is_finite()).map(|a| (a.x, a.y));
    (find(ENTRY), find(EXIT))
}

/// The distance along a right-to-left line to add before a glyph of advance `advance` so its
/// `entry` anchor meets the `exit` anchor of the glyph before it (both in their glyphs' own
/// coordinates). The glyph before stands to the right: its exit is at its origin plus `exit.0`,
/// and the glyph's own origin must then be `exit.0 - entry.0` from that origin.
pub(crate) fn shift(exit: (f64, f64), entry: (f64, f64), advance: f64) -> f64 {
    let d = entry.0 - exit.0 - advance;
    if d.is_finite() { d } else { 0.0 }
}
