//! Masks and the composite of a layer's shapes (`docs/spec/03a-edit-view-drawing.md` § 18).
//!
//! A layer's outline is its shapes taken in order. A stroked shape counts as its stroke outline.
//! A mask shape draws nothing itself: it subtracts its outline from everything before it in the
//! order (or, as an intersecting mask, keeps only what lies inside it). [`composite`] is what the
//! canvas draws and what export compiles; overlap removal comes after it.

use fontcraft_geom::{BezPath, PathEl};

use crate::stroke::{StrokeStyle, stroke_outline};
use crate::{PathOpError, difference, intersection};

/// Most mask shapes one layer may have: each one is a boolean operation over the shapes before
/// it.
pub const MAX_MASKS: usize = 256;

/// What a mask shape does to the shapes before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskMode {
    Subtract,
    Intersect,
}

impl MaskMode {
    /// The mode of the `mask` value a source of `.glyphs` format version `glyphs_version` stores.
    /// 0 is no mask. Versions before 4 store 1 for a mask; version 4 stores 1 to subtract and 2
    /// to intersect.
    pub fn from_code(code: u8, glyphs_version: u32) -> Option<Self> {
        match code {
            0 => None,
            2 if glyphs_version >= 4 => Some(MaskMode::Intersect),
            _ => Some(MaskMode::Subtract),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            MaskMode::Subtract => 1,
            MaskMode::Intersect => 2,
        }
    }
}

/// One shape of a layer as [`composite`] takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerShape {
    /// The shape's contours in font units: a path's contour, or a component's outline (itself a
    /// composite of the base glyph's layer) placed by the component's transform.
    pub outline: Vec<BezPath>,
    /// The stroke the outline is drawn with, if any.
    pub stroke: Option<StrokeStyle>,
    pub mask: Option<MaskMode>,
}

impl LayerShape {
    /// An ordinary filled shape.
    pub fn fill(outline: Vec<BezPath>) -> Self {
        Self { outline, stroke: None, mask: None }
    }
}

/// The outline of a layer whose shapes are `shapes`, in order: each stroke expanded, each mask
/// applied to the shapes before it. A layer with neither strokes nor masks comes back as it is,
/// contour for contour. Otherwise open contours that are not stroked draw nothing and are left
/// out once a mask applies, and the result of a mask has no overlaps.
pub fn composite(shapes: &[LayerShape]) -> Result<Vec<BezPath>, PathOpError> {
    if shapes.iter().all(|s| s.stroke.is_none() && s.mask.is_none()) {
        return Ok(shapes.iter().flat_map(|s| s.outline.iter().cloned()).collect());
    }
    let masks = shapes.iter().filter(|s| s.mask.is_some()).count();
    if masks > MAX_MASKS {
        return Err(PathOpError::Invalid(format!("the layer has {masks} masks; at most {MAX_MASKS} are allowed")));
    }
    let mut below: Vec<BezPath> = Vec::new();
    for shape in shapes {
        let outline = match &shape.stroke {
            Some(style) => {
                let mut out = Vec::new();
                for path in &shape.outline {
                    out.extend(stroke_outline(path, style)?);
                }
                out
            }
            None => shape.outline.clone(),
        };
        let Some(mode) = shape.mask else {
            below.extend(outline);
            continue;
        };
        let mask = closed_only(&outline);
        below = match mode {
            MaskMode::Subtract if mask.is_empty() => below,
            MaskMode::Intersect if mask.is_empty() => Vec::new(),
            MaskMode::Subtract => difference(&closed_only(&below), &mask)?,
            MaskMode::Intersect => intersection(&closed_only(&below), &mask)?,
        };
    }
    Ok(below)
}

/// The closed subpaths of `paths` that have a segment, one path each.
fn closed_only(paths: &[BezPath]) -> Vec<BezPath> {
    let mut out = Vec::new();
    for path in paths {
        let mut current = BezPath::new();
        let mut drawn = false;
        for el in path.elements() {
            match el {
                PathEl::MoveTo(_) => {
                    current = BezPath::new();
                    current.push(*el);
                    drawn = false;
                }
                PathEl::ClosePath => {
                    current.push(*el);
                    let done = std::mem::take(&mut current);
                    if drawn {
                        out.push(done);
                    }
                    drawn = false;
                }
                PathEl::LineTo(_) | PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
                    current.push(*el);
                    drawn = true;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use fontcraft_geom::{Point, Shape};

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        BezPath::from_svg(&format!("M{x0},{y0} L{x1},{y0} L{x1},{y1} L{x0},{y1} Z")).unwrap()
    }

    fn area(out: &[BezPath]) -> f64 {
        out.iter().map(Shape::area).sum()
    }

    fn covers(out: &[BezPath], x: f64, y: f64) -> bool {
        out.iter().map(|p| p.winding(Point::new(x, y))).sum::<i32>() != 0
    }

    fn mask(outline: Vec<BezPath>, mode: MaskMode) -> LayerShape {
        LayerShape { outline, stroke: None, mask: Some(mode) }
    }

    #[test]
    fn a_layer_without_strokes_or_masks_is_unchanged() {
        let shapes = [LayerShape::fill(vec![rect(0.0, 0.0, 100.0, 100.0)]), LayerShape::fill(vec![rect(50.0, 50.0, 150.0, 150.0)])];
        let out = composite(&shapes).unwrap();
        assert_eq!(out, vec![rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 150.0, 150.0)]);
    }

    #[test]
    fn a_mask_cuts_only_the_shapes_below_it() {
        let shapes = [
            LayerShape::fill(vec![rect(0.0, 0.0, 100.0, 100.0)]),
            mask(vec![rect(50.0, 0.0, 150.0, 100.0)], MaskMode::Subtract),
            LayerShape::fill(vec![rect(100.0, 0.0, 200.0, 100.0)]),
        ];
        let out = composite(&shapes).unwrap();
        assert!((area(&out) - 15_000.0).abs() < 1e-6, "{}", area(&out));
        assert!(covers(&out, 25.0, 50.0) && !covers(&out, 75.0, 50.0) && covers(&out, 125.0, 50.0), "the shape above the mask stays whole");
        // Moved to the top, the mask cuts both squares.
        let shapes = [shapes[0].clone(), shapes[2].clone(), shapes[1].clone()];
        let out = composite(&shapes).unwrap();
        assert!((area(&out) - 10_000.0).abs() < 1e-6, "{}", area(&out));
        assert!(!covers(&out, 125.0, 50.0));
    }

    #[test]
    fn an_intersecting_mask_keeps_what_is_inside_it() {
        let shapes = [LayerShape::fill(vec![rect(0.0, 0.0, 100.0, 100.0)]), mask(vec![rect(50.0, 50.0, 150.0, 150.0)], MaskMode::Intersect)];
        assert!((area(&composite(&shapes).unwrap()) - 2_500.0).abs() < 1e-6);
    }

    #[test]
    fn a_stroked_mask_cuts_with_its_stroke() {
        // A horizontal line stroked 20 wide cuts a band out of the square.
        let line = BezPath::from_svg("M-10,50 L110,50").unwrap();
        let shapes = [
            LayerShape::fill(vec![rect(0.0, 0.0, 100.0, 100.0)]),
            LayerShape { outline: vec![line], stroke: Some(StrokeStyle::new(20.0)), mask: Some(MaskMode::Subtract) },
        ];
        let out = composite(&shapes).unwrap();
        assert!((area(&out) - 8_000.0).abs() < 1e-3, "{}", area(&out));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn mask_codes_follow_the_format_version() {
        assert_eq!(MaskMode::from_code(0, 4), None);
        assert_eq!(MaskMode::from_code(1, 3), Some(MaskMode::Subtract));
        assert_eq!(MaskMode::from_code(2, 3), Some(MaskMode::Subtract));
        assert_eq!(MaskMode::from_code(2, 4), Some(MaskMode::Intersect));
    }
}
