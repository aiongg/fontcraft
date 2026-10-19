//! The SVG file of an svg layer as filled outlines for the editor to draw (M11.14): each visible
//! path of the file, its fill and its stroke, as an outline in font units with a paint of the
//! paint graph.
//!
//! Rows: `color.svg.layerHoldsFile` (drawing), `color.svg.imageTransform` (drawing).
//!
//! Contract:
//! - The file goes through [`crate::svg_import::import`] first, so it meets every limit and check
//!   an export applies (size, nesting, no external references); the document that gives is read
//!   back with usvg and walked. A file export refuses draws nothing and gives the same error.
//! - Placement is the export's: the file's user space placed by the layer's image transform, so
//!   the canvas shows what the `SVG ` table holds.
//! - Fills are solid colours or gradients; strokes are expanded to outlines by usvg's stroker.
//!   Group and paint opacity multiply into the paint's alpha.
//! - Not drawn: text (usvg reads files without text), embedded images, patterns, clip paths,
//!   masks and filters. Even-odd fills are filled with the non-zero rule, which differs only for
//!   overlapping contours. A radial gradient under a non-uniform transform keeps circles, with
//!   radii scaled by the transform's mean scale.
//! - At most [`MAX_SVG_SHAPES`] outlines; walking stops at [`crate::MAX_SVG_DEPTH`] groups.

use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::Extend;
use usvg::tiny_skia_path::{self, PathSegment};

use crate::paint::{ColorLine, ColorRef, ColorStop, Paint};
use crate::{ColorError, MAX_STOPS, MAX_SVG_DEPTH};

/// Most outlines one SVG file gives (a fill and a stroke count as two).
pub const MAX_SVG_SHAPES: usize = 100_000;

/// One filled outline of an SVG file.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgShape {
    /// Font units, y up.
    pub outline: BezPath,
    /// [`Paint::Solid`] with an RGBA colour, [`Paint::Linear`] or [`Paint::Radial`], in font units.
    pub paint: Paint,
}

/// The outlines of the SVG file `svg` placed by `transform` (the layer's image transform), in
/// paint order, the first at the bottom.
pub fn shapes(svg: &[u8], transform: Affine) -> Result<Vec<SvgShape>, ColorError> {
    let document = crate::svg_import::import(svg, transform, 0)?;
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver { resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None) },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&document, &options).map_err(|e| ColorError::BadSvg(e.to_string()))?;
    let mut out = Vec::new();
    walk(tree.root(), 1.0, 0, &mut out)?;
    Ok(out)
}

fn affine(t: tiny_skia_path::Transform) -> Affine {
    Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)])
}

/// The document's coordinates run y down; the glyph's run y up.
fn to_font(t: tiny_skia_path::Transform) -> Affine {
    Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]) * affine(t)
}

fn walk(group: &usvg::Group, opacity: f64, depth: usize, out: &mut Vec<SvgShape>) -> Result<(), ColorError> {
    if depth > MAX_SVG_DEPTH + 4 {
        return Ok(());
    }
    let opacity = opacity * f64::from(group.opacity().get());
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk(g, opacity, depth + 1, out)?,
            usvg::Node::Path(p) if p.is_visible() => path_shapes(p, opacity, out)?,
            _ => {}
        }
    }
    Ok(())
}

fn path_shapes(path: &usvg::Path, opacity: f64, out: &mut Vec<SvgShape>) -> Result<(), ColorError> {
    let place = to_font(path.abs_transform());
    let fill = path.fill().and_then(|f| {
        let paint = paint_of(f.paint(), opacity * f64::from(f.opacity().get()), place)?;
        Some(SvgShape { outline: bez(path.data(), place), paint })
    });
    let stroke = path.stroke().and_then(|s| {
        let paint = paint_of(s.paint(), opacity * f64::from(s.opacity().get()), place)?;
        let outline = path.data().stroke(&s.to_tiny_skia(), 1.0)?;
        Some(SvgShape { outline: bez(&outline, place), paint })
    });
    let ordered = match path.paint_order() {
        usvg::PaintOrder::FillAndStroke => [fill, stroke],
        usvg::PaintOrder::StrokeAndFill => [stroke, fill],
    };
    for shape in ordered.into_iter().flatten() {
        if out.len() >= MAX_SVG_SHAPES {
            return Err(ColorError::LimitExceeded { what: "outlines drawn from one SVG file", limit: MAX_SVG_SHAPES });
        }
        if !shape.outline.elements().is_empty() {
            out.push(shape);
        }
    }
    Ok(())
}

/// `path` mapped by `place` into font units.
fn bez(path: &tiny_skia_path::Path, place: Affine) -> BezPath {
    let p = |q: tiny_skia_path::Point| place * Point::new(f64::from(q.x), f64::from(q.y));
    let mut out = BezPath::new();
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(a) => out.move_to(p(a)),
            PathSegment::LineTo(a) => out.line_to(p(a)),
            PathSegment::QuadTo(a, b) => out.quad_to(p(a), p(b)),
            PathSegment::CubicTo(a, b, c) => out.curve_to(p(a), p(b), p(c)),
            PathSegment::Close => out.close_path(),
        }
    }
    out
}

/// The paint of an SVG paint at `opacity`, its geometry mapped by `place` (the path's own space
/// to font units, which a gradient's space is in after its transform); `None` for a pattern
/// or a gradient that cannot be drawn.
fn paint_of(paint: &usvg::Paint, opacity: f64, place: Affine) -> Option<Paint> {
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 0.0 };
    match paint {
        usvg::Paint::Color(c) => Some(Paint::Solid(rgba(*c, opacity))),
        usvg::Paint::LinearGradient(g) => {
            let t = place * affine(g.transform());
            let (a, b) = (Point::new(f64::from(g.x1()), f64::from(g.y1())), Point::new(f64::from(g.x2()), f64::from(g.y2())));
            // The isolines of an SVG gradient are at right angles to its vector in gradient space.
            let perp = Point::new(a.x + (b.y - a.y), a.y - (b.x - a.x));
            Some(Paint::Linear { p0: t * a, p1: t * b, p2: t * perp, line: line(g, opacity)? })
        }
        usvg::Paint::RadialGradient(g) => {
            let t = place * affine(g.transform());
            let scale = t.determinant().abs().sqrt();
            let c0 = t * Point::new(f64::from(g.fx()), f64::from(g.fy()));
            let c1 = t * Point::new(f64::from(g.cx()), f64::from(g.cy()));
            Some(Paint::Radial { c0, r0: f64::from(g.fr().get()) * scale, c1, r1: f64::from(g.r().get()) * scale, line: line(g, opacity)? })
        }
        usvg::Paint::Pattern(_) => None,
    }
}

fn rgba(c: usvg::Color, alpha: f64) -> ColorRef {
    ColorRef::Rgba([f64::from(c.red) / 255.0, f64::from(c.green) / 255.0, f64::from(c.blue) / 255.0, alpha])
}

fn line(g: &usvg::BaseGradient, opacity: f64) -> Option<ColorLine> {
    let stops: Vec<ColorStop> = g
        .stops()
        .iter()
        .take(MAX_STOPS)
        .map(|s| ColorStop { offset: f64::from(s.offset().get()), color: rgba(s.color(), opacity * f64::from(s.opacity().get())) })
        .collect();
    if stops.is_empty() {
        return None;
    }
    let extend = match g.spread_method() {
        usvg::SpreadMethod::Pad => Extend::Pad,
        usvg::SpreadMethod::Reflect => Extend::Reflect,
        usvg::SpreadMethod::Repeat => Extend::Repeat,
    };
    Some(ColorLine { stops, extend })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_file_gives_its_fills_and_strokes_in_font_units() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><linearGradient id="g" x1="0" y1="0" x2="100" y2="0" gradientUnits="userSpaceOnUse">
              <stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
            <rect x="0" y="0" width="100" height="50" fill="#00ff00" opacity="0.5"/>
            <rect x="0" y="50" width="100" height="50" fill="url(#g)" stroke="#000000" stroke-width="4"/>
        </svg>"##;
        let out = shapes(svg, Affine::translate((10.0, 0.0))).unwrap();
        assert_eq!(out.len(), 3, "a fill, a gradient fill and a stroke");
        // The file's top half lies at the top: y 50 to 100 in font units, moved 10 to the right.
        let top = fontcraft_geom::Shape::bounding_box(&out[0].outline);
        assert!((top.x0 - 10.0).abs() < 1e-3 && (top.y0 - 50.0).abs() < 1e-3 && (top.y1 - 100.0).abs() < 1e-3, "{top:?}");
        assert_eq!(out[0].paint, Paint::Solid(ColorRef::Rgba([0.0, 1.0, 0.0, 0.5])));
        let Paint::Linear { p0, p1, line, .. } = &out[1].paint else { panic!("{:?}", out[1].paint) };
        assert!((p0.x - 10.0).abs() < 1e-3 && (p1.x - 110.0).abs() < 1e-3, "{p0:?} {p1:?}");
        assert_eq!(line.stops.len(), 2);
        let stroke = fontcraft_geom::Shape::bounding_box(&out[2].outline);
        assert!(stroke.x0 < 10.0 && stroke.x1 > 110.0, "the stroke reaches half its width beyond the rectangle: {stroke:?}");
    }

    #[test]
    fn a_file_export_refuses_draws_nothing() {
        let external = br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///etc/passwd" width="1" height="1"/></svg>"#;
        assert!(matches!(shapes(external, Affine::IDENTITY), Err(ColorError::BadSvg(_))));
        let deep = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>", "<g>".repeat(10_000), "</g>".repeat(10_000));
        assert!(shapes(deep.as_bytes(), Affine::IDENTITY).is_err());
    }
}
