//! Linked sources: the Import Font and Import Master parameters. When a document opens and before
//! each export, the files they name are read with the existing readers, and the glyphs the font
//! lacks (Import Font) or the named master with its layers (Import Master) are added, marked
//! linked and locked. The source writers leave linked glyphs and masters out.
//!
//! No nesting: a linked file's own Import Font is ignored with a warning. At most
//! [`fontcraft_fontinfo::MAX_IMPORT_FONT`] and [`fontcraft_fontinfo::MAX_IMPORT_MASTER`]
//! parameters; each file within the readers' input limit. A missing file is a warning, not an
//! error. In the browser there are no files to read: the parameters are kept and warned about.
//!
//! Rows: `Import Font`, `Import Master` (M7.12).

use std::path::Path;

use fontcraft_model::Font;

use crate::Result;

/// `font` with its linked sources added, or `None` when it has no Import Font or Import Master
/// parameter. `path` is the document's file, which relative paths start from.
pub(crate) fn resolve(font: &Font, path: Option<&Path>, warnings: &mut Vec<String>) -> Result<Option<Font>> {
    // M7.12: read the files, add the glyphs and masters marked linked and locked.
    let _ = (font, path, warnings);
    Ok(None)
}
