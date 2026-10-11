//! Interpolation queries and axis mapping commands (M9.17).
//!
//! Rows: `interpolation.interpolateGlyph`, `interpolation.describeDesignspace`,
//! `interpolation.checkCompatibility`, `axisMappings.addPoint`, `axisMappings.setPoint`,
//! `axisMappings.nudgePoint`, `axisMappings.removePoint`, `axisMappings.convertCoordinate`.

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
