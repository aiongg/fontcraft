//! Glyph composition and decomposition (M8.13).
//!
//! Rows: `autoFeature.ccmp`, `anchors.ccmpAlongsideMark`.

use super::Generator;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    // M8.13
    Vec::new()
}
