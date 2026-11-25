//! Which curves new segments take: quadratic (TrueType) or cubic (PostScript)
//! (`draw.curveTypeFollowsGlyph`).

use crate::{Font, NodeKind, Shape};

/// Most glyphs [`Font::draws_quadratic`] reads; a font without a curve in these draws cubics.
const MAX_SCANNED_GLYPHS: usize = 5_000;

/// Whether the curve segments of `shapes` are quadratic: true with any quadratic segment, false
/// with cubic ones only, `None` without a curve.
pub fn quadratic_curves(shapes: &[Shape]) -> Option<bool> {
    let mut cubic = false;
    for shape in shapes {
        let Shape::Path(path) = shape else { continue };
        for node in &path.nodes {
            match node.kind {
                NodeKind::QCurve => return Some(true),
                NodeKind::Curve => cubic = true,
                NodeKind::Line | NodeKind::OffCurve => {}
            }
        }
    }
    cubic.then_some(false)
}

impl Font {
    /// Whether the font is drawn with quadratic curves, as the first of its layers that has a
    /// curve, in glyph order, says.
    pub fn draws_quadratic(&self) -> bool {
        self.glyphs.iter().take(MAX_SCANNED_GLYPHS).flat_map(|g| g.layers.iter()).find_map(|l| quadratic_curves(&l.shapes)).unwrap_or(false)
    }
}
