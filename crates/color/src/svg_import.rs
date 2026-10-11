//! Referenced SVG files as glyph documents (M11.10): the file of an svg layer, parsed with usvg
//! and written back with the layer's image transform and the `glyph<ID>` id.
//!
//! Rows: `color.svg.threeSources`, `color.svg.imageTransform`, `color.svg.layerHoldsFile`.
//!
//! Contract (`docs/plans/M11.md` § 3 › Limits):
//! - Files over [`crate::MAX_SVG_FILE_BYTES`], nested deeper than [`crate::MAX_SVG_DEPTH`], or
//!   with external references are refused with [`crate::ColorError::BadSvg`].
//! - usvg runs without text and without system fonts; embedded bitmaps are kept.

use fontcraft_geom::Affine;

use crate::ColorError;

/// The document of glyph `gid` drawing the SVG file `svg`, placed by `transform` (the layer's
/// image transform, font units, y up).
pub fn import(svg: &[u8], transform: Affine, gid: u16) -> Result<String, ColorError> {
    // M11.10
    let _ = (svg, transform, gid);
    Err(ColorError::NotYetImplemented("SVG files in svg layers"))
}
