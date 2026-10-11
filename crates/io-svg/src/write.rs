//! Outlines as a compact SVG document, for other apps (the SVG flavour of `edit.copy`).

use std::fmt::Write as _;

use fontcraft_geom::{BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{Component, MAX_OUTLINE_ELEMENTS, Shape};

use crate::SvgError;

/// `shapes` as an SVG document: paths as they are, components through `component_outline` (the
/// outline a component draws, placed by its transform, or `None` to leave it out). Images and raw
/// shapes are left out. See [`bez_to_svg`] for the document's form.
pub fn shapes_to_svg(
    shapes: &[Shape],
    component_outline: &dyn Fn(&Component) -> Option<BezPath>,
    view_box: Option<Rect>,
) -> Result<String, SvgError> {
    let mut all = BezPath::new();
    for shape in shapes {
        match shape {
            Shape::Path(p) => all.extend(p.to_bez().map_err(SvgError::BadPath)?),
            Shape::Component(c) => {
                if let Some(outline) = component_outline(c) {
                    all.extend(outline);
                }
            }
            Shape::Image(_) | Shape::Raw(_) => {}
        }
        if all.elements().len() > MAX_OUTLINE_ELEMENTS {
            return Err(SvgError::BadPath(format!("the outline has more than {MAX_OUTLINE_ELEMENTS} segments")));
        }
    }
    Ok(bez_to_svg(&all, view_box))
}

/// `outline` (glyph coordinates, y up) as an SVG document with one `path` element, in SVG
/// coordinates (y down: SVG y is minus glyph y, so one font unit is one SVG unit and reading the
/// document back with an unscaled placement at top 0 gives the same points). The `viewBox` is
/// `view_box` (glyph coordinates), or the outline's bounding box; width and height match it.
/// Numbers are written with at most three decimals; a number that is not finite is written as 0.
pub fn bez_to_svg(outline: &BezPath, view_box: Option<Rect>) -> String {
    let frame = view_box.unwrap_or_else(|| outline.bounding_box());
    let (x0, y0, x1, y1) = (finite(frame.min_x()), finite(frame.min_y()), finite(frame.max_x()), finite(frame.max_y()));
    // A viewBox needs a positive size after rounding to three decimals.
    let width = if x1 - x0 >= 0.001 { x1 - x0 } else { 1.0 };
    let height = if y1 - y0 >= 0.001 { y1 - y0 } else { 1.0 };
    let (w, h) = (num(width), num(height));
    let mut out = String::new();
    let _ = write!(out, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{} {} {w} {h}">"#, num(x0), num(-y1));
    let d = path_data(outline);
    if !d.is_empty() {
        let _ = write!(out, r#"<path d="{d}"/>"#);
    }
    out.push_str("</svg>");
    out
}

/// SVG path data: absolute commands, no spaces before a command letter.
fn path_data(outline: &BezPath) -> String {
    let mut d = String::new();
    let point = |d: &mut String, p: Point| {
        d.push_str(&num(p.x));
        d.push(' ');
        d.push_str(&num(-p.y));
    };
    for el in outline.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                d.push('M');
                point(&mut d, p);
            }
            PathEl::LineTo(p) => {
                d.push('L');
                point(&mut d, p);
            }
            PathEl::QuadTo(c, p) => {
                d.push('Q');
                point(&mut d, c);
                d.push(' ');
                point(&mut d, p);
            }
            PathEl::CurveTo(c1, c2, p) => {
                d.push('C');
                point(&mut d, c1);
                d.push(' ');
                point(&mut d, c2);
                d.push(' ');
                point(&mut d, p);
            }
            PathEl::ClosePath => d.push('Z'),
        }
    }
    d
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// `v` rounded to three decimals, in its shortest form, with no negative zero.
fn num(v: f64) -> String {
    let r = (finite(v) * 1000.0).round() / 1000.0;
    if r == 0.0 || !r.is_finite() { "0".to_string() } else { format!("{r}") }
}
