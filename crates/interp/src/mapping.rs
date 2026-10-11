//! Axis mappings: editing the points of one axis's external-to-internal map, and converting a
//! value between the two spaces (M9.2; the commands are M9.17's).
//!
//! Rows: `axisMappings.identityByDefault`, `axisMappings.controlPoints`,
//! `axisMappings.perAxisOnly`, `axisMappings.fontAndExportLevel`,
//! `axisMappings.externalIncludesAxisLocation`, `axisMappings.addPoint`, `axisMappings.setPoint`,
//! `axisMappings.nudgePoint`, `axisMappings.removePoint`, `axisMappings.convertCoordinate`.
//!
//! Contract:
//! - A mapping is a list of `(external, internal)` points sorted by external value, at most
//!   [`MAX_MAPPING_POINTS`]; both coordinates finite, within [`crate::MAX_COORDINATE`] and
//!   strictly increasing. An edit that would break that is refused and changes nothing.
//! - An edit returns the new value of the font's Axis Mappings parameter (every axis, the edited
//!   one changed), ready for [`crate::params::set_axis_mappings`]; the command writes it in one
//!   undo step. Removing the last point of an axis removes the axis's entry (identity).
//! - A mapping read at export that breaks the rules is ignored with a warning (identity).

use fontcraft_model::Font;

use crate::designspace::{Designspace, Space};
use crate::{InterpError, MAX_MAPPING_POINTS};

/// One edit of an axis mapping. Point indices count in external order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MappingEdit {
    /// Adds a point.
    Add { external: f64, internal: f64 },
    /// Moves the point at `index` to new coordinates.
    Set { index: usize, external: f64, internal: f64 },
    /// Moves the point at `index` by the given amounts (the arrow keys of the mapping editor).
    Nudge { index: usize, external: f64, internal: f64 },
    /// Removes the point at `index`.
    Remove { index: usize },
}

/// The points of axis `tag`'s mapping after `edit`, checked; refuses an unknown axis, a bad
/// index, a point that is not a usable number, points out of order and more than
/// [`MAX_MAPPING_POINTS`] points.
pub fn edited_points(font: &Font, tag: &str, edit: &MappingEdit) -> Result<Vec<(f64, f64)>, InterpError> {
    // M9.2
    let _ = (font, tag, edit, MAX_MAPPING_POINTS);
    Err(InterpError::NotYetImplemented("editing axis mappings"))
}

/// Checks a mapping's points: finite, within range, strictly increasing on both coordinates, at
/// most [`MAX_MAPPING_POINTS`]. `axis` names the axis in the refusal.
pub fn check_points(axis: &str, points: &[(f64, f64)]) -> Result<(), InterpError> {
    // M9.2
    let _ = (axis, points);
    Err(InterpError::NotYetImplemented("checking axis mappings"))
}

/// `value` on the axis at `axis` (an index into the designspace's axes) converted from the
/// space `from` to the other one.
pub fn convert(designspace: &Designspace, axis: usize, value: f64, from: Space) -> Result<f64, InterpError> {
    // M9.2
    let _ = (designspace, axis, value, from);
    Err(InterpError::NotYetImplemented("converting coordinates"))
}
