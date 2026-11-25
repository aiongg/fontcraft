//! The masters and interpolation dialogs: the Masters tab's dialogs ([`masters`]), Axis Location and Virtual Master
//! ([`location`]), the editors of special layers ([`special`]), and the interpolation commands of the Path, Glyph and Font menus ([`variation`]). All are specs.

pub mod location;
pub mod masters;
pub mod special;
#[cfg(test)]
mod tests;
pub mod variation;

use serde_json::{Map, Value};

use super::spec::DialogSpec;

pub const SPECS: &[DialogSpec] = &[
    location::AXIS_LOCATION_DIALOG,
    location::VIRTUAL_MASTER_DIALOG,
    masters::ADD_OTHER_FONT,
    masters::ICON_PICKER,
    masters::ADD_METRIC,
    masters::METRIC_EDITOR,
    masters::STEM_SCOPE,
    special::INTERMEDIATE,
    special::ALTERNATE,
    variation::INTERPOLATE_WITH_BACKGROUND,
    variation::ADJUST_BY_VARIATION,
    variation::SET_MASTER_LOCATIONS,
    variation::ADD_VARIATION,
    variation::MERGE_FONTS,
];

/// A dialog whose fields keep no step with each other.
pub fn no_link(_: &mut Map<String, Value>, _: &str) {}
