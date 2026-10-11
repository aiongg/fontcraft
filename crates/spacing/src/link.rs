//! Linked metrics: masters that take their widths and kerning from another master.
//!
//! The rule follows fontc's `glyphs-reader` (`font.rs`, the metrics-source lookup): *Link
//! Metrics With First Master* wins; *Link Metrics With Master* names a master by id, then by
//! name. One step only: the source's own parameters are not followed.

use fontcraft_model::{Font, Master, param_flag};

pub const LINK_WITH_FIRST_MASTER: &str = "Link Metrics With First Master";
pub const LINK_WITH_MASTER: &str = "Link Metrics With Master";

/// The master whose widths and kerning `master` shows: the one its parameters name, else itself.
/// `None` when the font has no master with that id. A parameter that names no master is ignored.
pub fn metrics_source<'a>(font: &'a Font, master: &str) -> Option<&'a Master> {
    let m = font.masters.iter().find(|m| m.id == master)?;
    if m.params.get(LINK_WITH_FIRST_MASTER).and_then(param_flag) == Some(true) {
        return font.masters.first();
    }
    if let Some(target) = m.params.get(LINK_WITH_MASTER).and_then(|v| v.as_str())
        && let Some(source) = font.masters.iter().find(|s| s.id == target).or_else(|| font.masters.iter().find(|s| s.name == target))
    {
        return Some(source);
    }
    Some(m)
}
