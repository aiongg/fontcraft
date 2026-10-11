//! Special layer commands (M9.16): intermediate and alternate layers, Re-Interpolate, shape
//! order.
//!
//! Rows: `layers.setIntermediate`, `layers.clearIntermediate`, `layers.setAlternate`,
//! `layers.clearAlternate`, `layers.reInterpolate`, `glyph.setShapeOrder`, `glyph.swapShapes`.
//! The labels these layers are shown with are `fontcraft_interp::labels` (same package).

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
