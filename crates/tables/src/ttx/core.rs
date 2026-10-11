//! TTX writer: the core TrueType and OpenType tables (M12.4).

use crate::Result;
use crate::sfnt::{SfntFont, Tag};

use super::XmlWriter;

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    // M12.4
    let _ = (tag, data, font, w);
    Ok(false)
}
