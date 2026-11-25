//! Copies of unsaved work, kept beside the user's own files until the work is saved or closed.
//!
//! The app decides when a copy is written and where; this module says what it holds ([`encode`]: the document in the
//! native format, whatever it was opened from) and `file.openRecovered` opens one. A recovered document carries the
//! file it came from but no stamp of it, so saving over that file asks first, as it does for a file something else
//! changed.

use fontcraft_model::Font;

/// The extension of a copy, without the dot.
pub const EXTENSION: &str = "fontcraft";

/// The bytes of a copy of `font`. The source format it was opened from is in them, so a recovered document is saved in
/// that format again.
pub fn encode(font: &Font) -> Result<Vec<u8>, String> {
    fontcraft_format::save(font).map_err(|e| e.to_string())
}
