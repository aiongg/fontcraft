//! Image commands: add images to glyphs (one file, or many matched to glyph names), move, lock,
//! unlock and crop them, and report what they are. Files are read through the session's asset
//! store.
//!
//! Package M11.12 (`docs/plans/M11.md` § 4 › M11.12) fills [`specs`]; rows: `glyph.addImage`,
//! `image.setTransform`, `image.lock`, `image.unlock`, `image.setCrop`,
//! `image.setCropToLayerBounds`, `image.resetCrop`, `image.getInfo`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M11.12
    Vec::new()
}
