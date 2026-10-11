//! Colour layer commands: convert a layer to a palette, full-colour, iColor or svg layer, set its
//! palette index, size or file, and list a glyph's colour layers. A master layer is never
//! converted.
//!
//! Package M11.3 (`docs/plans/M11.md` § 4 › M11.3) fills [`specs`]; rows:
//! `layer.convertToColorPalette`, `layer.setColorPaletteIndex`, `layer.convertToIColor`,
//! `layer.setIColorSize`, `layer.setIColorImage`, `layer.convertToSvg`, `layer.setSvgImage`,
//! `layer.convertToColor`, `layer.listColorLayers`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M11.3
    Vec::new()
}
