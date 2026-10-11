//! Sidebearings from a layer's outline, components resolved.
//!
//! LSB = x-min; RSB = width − x-max. Vertically the advance box runs from the top, the master's
//! ascender less the layer's vertical origin (the model measures it down from the ascender), down
//! by the vertical width (the ascender to the descender when the layer has none): TSB = top −
//! y-max; BSB = y-min − bottom.
//!
//! In a master with an italic angle the left and right sidebearings are measured along lines at
//! that angle, through the point at half the x-height: the outline is slanted back around that
//! height before its bounds are taken (the handbook: the italic angle feeds the sidebearing
//! calculation; Inter Italic's stored keys and metrics agree).

use fontcraft_geom::{Affine, BezPath, Line, ParamCurve, Point, Rect, Shape as _};
use fontcraft_model::{Font, Layer, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, Shape, metric};

use crate::{EvalError, Field};

/// The outline of one layer (master or not), its components read from their glyphs' layers of
/// the layer's master. Bounded by the model's component-depth and outline-size limits.
pub fn outline(font: &Font, layer: &Layer) -> Result<BezPath, EvalError> {
    let mut out = BezPath::new();
    let mut budget = MAX_OUTLINE_ELEMENTS;
    for shape in &layer.shapes {
        let part = match shape {
            Shape::Path(p) => p.to_bez().map_err(EvalError::Measure)?,
            Shape::Component(c) => {
                let numbers = c.transform.numbers();
                if numbers.iter().any(|v| !v.is_finite()) {
                    return Err(EvalError::Measure("a component transform is not a number".into()));
                }
                let mut inner = font.outline(&c.glyph, &layer.master).map_err(|e| EvalError::Measure(e.to_string()))?;
                inner.apply_affine(c.affine());
                inner
            }
            Shape::Image(_) | Shape::Raw(_) => continue,
        };
        budget = budget.checked_sub(part.elements().len()).ok_or_else(|| EvalError::Measure("the outline is too large to measure".into()))?;
        out.extend(part);
    }
    Ok(out)
}

/// For a layer of a master with an italic angle: the tangent of the angle and the height the
/// slanted measuring lines are taken through.
fn slant(font: &Font, layer: &Layer) -> Option<(f64, f64)> {
    let m = font.masters.iter().find(|m| m.id == layer.master)?;
    let angle = font.metric_pos(m, metric::ITALIC_ANGLE, 0.0);
    if !angle.is_finite() || angle == 0.0 || angle.abs() >= 89.0 {
        return None;
    }
    let half_x = font.metric_pos(m, metric::X_HEIGHT, f64::from(font.units_per_em) * 0.5) / 2.0;
    half_x.is_finite().then(|| (angle.to_radians().tan(), half_x))
}

/// The bounds of a layer's outline, slanted back in an italic master; `None` for an empty layer.
pub fn bounds(font: &Font, layer: &Layer) -> Result<Option<Rect>, EvalError> {
    let mut path = outline(font, layer)?;
    if let Some((tan, y0)) = slant(font, layer) {
        path.apply_affine(Affine::new([1.0, 0.0, -tan, 1.0, tan * y0, 0.0]));
    }
    path_bounds(&path)
}

/// The bounds of a layer's outline as drawn, not slanted back in an italic master (the box an
/// AFM file gives); `None` for an empty layer.
pub fn upright_bounds(font: &Font, layer: &Layer) -> Result<Option<Rect>, EvalError> {
    path_bounds(&outline(font, layer)?)
}

fn path_bounds(path: &BezPath) -> Result<Option<Rect>, EvalError> {
    if path.elements().is_empty() {
        return Ok(None);
    }
    let r = path.bounding_box();
    let ok = [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE);
    if !ok {
        return Err(EvalError::Measure(format!("the outline reaches beyond ±{MAX_COORDINATE}")));
    }
    Ok(Some(r))
}

/// The top and bottom of a layer's vertical advance box.
fn vertical_box(font: &Font, layer: &Layer) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let (asc, desc) = match font.masters.iter().find(|m| m.id == layer.master) {
        Some(m) => (font.metric_pos(m, metric::ASCENDER, upm * 0.8), font.metric_pos(m, metric::DESCENDER, -upm * 0.2)),
        None => (upm * 0.8, -upm * 0.2),
    };
    let top = asc - layer.info.vert_origin.unwrap_or(0.0);
    (top, top - layer.info.vert_width.unwrap_or(asc - desc))
}

/// One metric of a layer from given bounds: `None` for a sidebearing of an empty layer.
pub(crate) fn metric_from(font: &Font, layer: &Layer, field: Field, b: Option<Rect>) -> Option<f64> {
    match field {
        Field::Width => Some(layer.width),
        Field::Left => b.map(|b| b.x0),
        Field::Right => b.map(|b| layer.width - b.x1),
        Field::Top => b.map(|b| vertical_box(font, layer).0 - b.y1),
        Field::Bottom => b.map(|b| b.y0 - vertical_box(font, layer).1),
    }
}

/// A layer's metric as it is: its width, or a sidebearing from its outline's bounds (`None` for
/// an empty layer).
pub fn metric(font: &Font, layer: &Layer, field: Field) -> Result<Option<f64>, EvalError> {
    let b = if field == Field::Width { None } else { bounds(font, layer)? };
    Ok(metric_from(font, layer, field, b))
}

/// The left and right sidebearings measured on the horizontal line at height `y`: from the left
/// edge of the advance to the first point where the outline meets the line, and from the last
/// such point to the right edge. `None` when the outline does not reach that height.
pub fn sidebearings_at_height(font: &Font, layer: &Layer, y: f64) -> Result<Option<(f64, f64)>, EvalError> {
    if !y.is_finite() {
        return Err(EvalError::Measure("the height is not a number".into()));
    }
    let path = outline(font, layer)?;
    Ok(at_height(font, layer, &path, y).map(|(first, last)| (first, layer.width - last)))
}

/// The first and last x where `path`, the outline of `layer`, meets the line at height `y`;
/// in an italic master, moved along the italic angle to the height the sidebearings are
/// measured at.
pub(crate) fn at_height(font: &Font, layer: &Layer, path: &BezPath, y: f64) -> Option<(f64, f64)> {
    let (first, last) = crossings(path, y)?;
    let shift = slant(font, layer).map_or(0.0, |(tan, y0)| tan * (y - y0));
    Some((first - shift, last - shift))
}

/// The smallest and largest x where `path` meets the line at height `y`.
fn crossings(path: &BezPath, y: f64) -> Option<(f64, f64)> {
    if path.elements().is_empty() {
        return None;
    }
    let r = path.bounding_box();
    if !(r.y0..=r.y1).contains(&y) {
        return None;
    }
    let line = Line::new(Point::new(r.x0 - 1.0, y), Point::new(r.x1 + 1.0, y));
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for seg in path.segments() {
        for hit in seg.intersect_line(line) {
            let x = seg.eval(hit.segment_t).x;
            if x.is_finite() {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}
