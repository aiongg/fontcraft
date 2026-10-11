//! Variable TrueType outlines: one glyph's sources made point-compatible and quadratic (M9.5).
//!
//! Rows: `compile.cu2quAcrossMasters`, `intermediateLayer.sparseInExport`.
//!
//! Contract: every cubic segment is converted in all sources of the glyph at once
//! (`fontcraft_geom::cu2qu_multi`), so each source gets the same points; `glyf` is then written
//! from the origin's outlines by the static writer. Sources that are not point-compatible after
//! conversion are refused.

use fontcraft_geom::BezPath;

use crate::CompileError;

/// `sources` (one list of contours per source, the same structure in each) with every cubic
/// segment replaced by the same number of quadratic segments in each source, within `tolerance`
/// font units.
pub fn quadratic_sources(glyph: &str, sources: &[Vec<BezPath>], tolerance: f64) -> Result<Vec<Vec<BezPath>>, CompileError> {
    // M9.5
    let _ = (glyph, sources, tolerance);
    Err(CompileError::NotYetImplemented("variable TrueType outlines"))
}
