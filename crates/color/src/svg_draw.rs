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
//! - [`items`] also gives, for Text Preview's colour glyphs: the file's embedded PNG and JPEG
//!   images, undecoded, with their placement; and as [`SvgGroup`]s, groups with a clip path or
//!   a mask, and shapes filled or stroked with a pattern. A pattern becomes its tiles over the
//!   shape, each tile its content clipped to the tile, clipped together to the shape. An image
//!   whose `preserveAspectRatio` slices it comes clipped to its box, since usvg gives it a clip
//!   path. [`shapes`] gives none of these: it draws a clipped or masked group's content whole
//!   and leaves out pattern fills and images.
//! - Clip paths, masks and pattern tiles are placed as resvg places them: in the coordinates of
//!   the element that uses them, the content of a pattern tile from the tile's top left.
//!   Nested clip paths (a `clip-path` on a `clipPath`) and nested masks all apply.
//! - Not drawn: text (usvg reads files without text) and filters (drawing them needs resvg).
//!   Even-odd fills and clip rules are filled with the non-zero rule, which differs only for
//!   overlapping contours. A radial gradient under a non-uniform transform keeps circles, with
//!   radii scaled by the transform's mean scale.
//! - At most [`MAX_SVG_SHAPES`] items and [`MAX_PATTERN_TILES`] pattern tiles; walking stops at
//!   [`crate::MAX_SVG_DEPTH`] groups.

use std::sync::Arc;

use fontcraft_geom::{Affine, BezPath, Point, Rect, Shape as _};
use fontcraft_model::Extend;
use usvg::tiny_skia_path::{self, PathSegment};

use crate::paint::{ColorLine, ColorRef, ColorStop, Paint};
use crate::{ColorError, MAX_STOPS, MAX_SVG_DEPTH};

/// Most outlines one SVG file gives (a fill and a stroke count as two), with its images and
/// groups.
pub const MAX_SVG_SHAPES: usize = 100_000;

/// Most pattern tiles one SVG file draws, all its patterns together.
pub const MAX_PATTERN_TILES: usize = 4_096;

/// Most elements visited while walking one SVG file: a pattern's content is walked once per tile.
const MAX_SVG_VISITS: usize = 1_000_000;

/// Tiles, of the tile size, by which a shape may pass a pattern tile's edge without reaching
/// into the next tile.
const EDGE: f64 = 1e-6;

/// One filled outline of an SVG file.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgShape {
    /// Font units, y up.
    pub outline: BezPath,
    /// [`Paint::Solid`] with an RGBA colour, [`Paint::Linear`] or [`Paint::Radial`], in font units.
    pub paint: Paint,
}

/// An embedded PNG or JPEG image of an SVG file, not yet decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgImage {
    /// The encoded image.
    pub data: Arc<Vec<u8>>,
    /// From the unit square of the image (y down from its top left) to font units, y up.
    pub place: Affine,
    /// Group opacity, from 0 to 1.
    pub opacity: f64,
}

/// What an SVG file draws, in paint order.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgItem {
    Shape(SvgShape),
    Image(SvgImage),
    Group(SvgGroup),
}

/// Items drawn together and then cut by clip paths and masks: a group of the file with a
/// `clip-path` or a `mask`, a shape filled with a pattern, or one tile of a pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgGroup {
    pub items: Vec<SvgItem>,
    /// Each clip keeps only what lies inside the union of its items, which are opaque
    /// [`SvgShape`]s or groups of them. All clips apply.
    pub clips: Vec<Vec<SvgItem>>,
    /// Each mask multiplies the group by its items drawn inside its region. All masks apply.
    pub masks: Vec<SvgMask>,
}

/// A mask of an [`SvgGroup`].
#[derive(Clone, Debug, PartialEq)]
pub struct SvgMask {
    /// The mask's luminance is its value; else its alpha.
    pub luminance: bool,
    /// Font units: nothing of the mask outside this outline counts.
    pub region: BezPath,
    pub items: Vec<SvgItem>,
}

/// The outlines of the SVG file `svg` placed by `transform` (the layer's image transform), in
/// paint order, the first at the bottom.
pub fn shapes(svg: &[u8], transform: Affine) -> Result<Vec<SvgShape>, ColorError> {
    let items = read(svg, |_, _| transform, false)?;
    Ok(items.into_iter().filter_map(|i| if let SvgItem::Shape(s) = i { Some(s) } else { None }).collect())
}

/// The outlines, groups and embedded PNG and JPEG images of the SVG file `svg`, in paint order, placed
/// by the transform `place` chooses from the width and height of the file's viewport (its
/// `width` and `height`, else its `viewBox`'s, else 100). The transform maps the viewport, y up
/// with its bottom left at the origin, to font units.
pub fn items(svg: &[u8], place: impl FnOnce(f64, f64) -> Affine) -> Result<Vec<SvgItem>, ColorError> {
    read(svg, place, true)
}

fn read(svg: &[u8], place: impl FnOnce(f64, f64) -> Affine, full: bool) -> Result<Vec<SvgItem>, ColorError> {
    let document = crate::svg_import::import_placed(svg, place, 0)?;
    let resolve_data: usvg::ImageHrefDataResolverFn<'static> = if full { Box::new(bitmap_data) } else { Box::new(|_, _, _| None) };
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver { resolve_data, resolve_string: Box::new(|_, _| None) },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&document, &options).map_err(|e| ColorError::BadSvg(e.to_string()))?;
    let root = tree.root();
    let mut walker = Walker { full, made: 0, visited: 0, tiles: 0 };
    let cx = Context { opacity: f64::from(root.opacity().get()), clip: false, depth: 0 };
    let mut out = Vec::new();
    walker.children(root, affine(root.transform()), cx, &mut out)?;
    Ok(out)
}

/// PNG and JPEG data, by their signatures; nothing else is drawn.
fn bitmap_data(_: &str, data: Arc<Vec<u8>>, _: &usvg::Options) -> Option<usvg::ImageKind> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(usvg::ImageKind::PNG(data))
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(usvg::ImageKind::JPEG(data))
    } else {
        None
    }
}

fn affine(t: tiny_skia_path::Transform) -> Affine {
    Affine::new([f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)])
}

/// The document's coordinates run y down; the glyph's run y up.
fn flip() -> Affine {
    Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0])
}

fn finite(a: Affine) -> bool {
    a.as_coeffs().iter().all(|v| v.is_finite())
}

fn limit(what: &'static str, limit: usize) -> ColorError {
    ColorError::LimitExceeded { what, limit }
}

/// An outline filled opaque, for a clip.
fn solid(outline: BezPath) -> SvgItem {
    SvgItem::Shape(SvgShape { outline, paint: Paint::Solid(ColorRef::Rgba([0.0, 0.0, 0.0, 1.0])) })
}

/// The rectangle `r` mapped by `t`, as an outline.
fn rect_outline(r: Rect, t: Affine) -> BezPath {
    let mut out = BezPath::new();
    out.move_to(t * Point::new(r.x0, r.y0));
    out.line_to(t * Point::new(r.x1, r.y0));
    out.line_to(t * Point::new(r.x1, r.y1));
    out.line_to(t * Point::new(r.x0, r.y1));
    out.close_path();
    out
}

fn rect_of(r: usvg::NonZeroRect) -> Rect {
    Rect::new(f64::from(r.left()), f64::from(r.top()), f64::from(r.right()), f64::from(r.bottom()))
}

/// What the walk inherits from the groups around a node.
#[derive(Clone, Copy)]
struct Context {
    /// Group opacity so far, from 0 to 1.
    opacity: f64,
    /// Inside a clip path: fills are opaque, strokes, images, masks and opacity ignored.
    clip: bool,
    depth: usize,
}

/// Walks a usvg tree into items, counting what it makes and visits.
struct Walker {
    /// Clip paths, masks, patterns and images given; else outlines only, as [`shapes`] gives them.
    full: bool,
    made: usize,
    visited: usize,
    tiles: usize,
}

impl Walker {
    fn make(&mut self, n: usize) -> Result<(), ColorError> {
        self.made = self.made.saturating_add(n);
        if self.made > MAX_SVG_SHAPES {
            return Err(limit("outlines drawn from one SVG file", MAX_SVG_SHAPES));
        }
        Ok(())
    }

    /// The children of `group`, whose coordinates `ctm` maps to font units.
    fn children(&mut self, group: &usvg::Group, ctm: Affine, cx: Context, out: &mut Vec<SvgItem>) -> Result<(), ColorError> {
        if cx.depth > MAX_SVG_DEPTH + 4 {
            return Ok(());
        }
        for node in group.children() {
            self.visited += 1;
            if self.visited > MAX_SVG_VISITS {
                return Err(limit("elements visited while drawing one SVG file", MAX_SVG_VISITS));
            }
            match node {
                usvg::Node::Group(g) => self.group(g, ctm, cx, out)?,
                usvg::Node::Path(p) if p.is_visible() => self.path(p, ctm, cx, out)?,
                usvg::Node::Image(i) if i.is_visible() && self.full && !cx.clip => self.image(i, ctm, cx.opacity, out)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// A group; one with a clip path or a mask becomes an [`SvgGroup`] when items are wanted.
    /// Clip paths and masks are drawn in the group's own coordinates (its transform applied),
    /// their contents' root transforms not applied, as resvg draws them.
    fn group(&mut self, g: &usvg::Group, parent: Affine, cx: Context, out: &mut Vec<SvgItem>) -> Result<(), ColorError> {
        let ctm = parent * affine(g.transform());
        let opacity = if cx.clip { 1.0 } else { cx.opacity * f64::from(g.opacity().get()) };
        let inner = Context { opacity, clip: cx.clip, depth: cx.depth + 1 };
        let masked = g.mask().is_some() && !cx.clip;
        if !(self.full && (g.clip_path().is_some() || masked)) {
            return self.children(g, ctm, inner, out);
        }
        let mut clips = Vec::new();
        let mut next = g.clip_path();
        while let Some(clip) = next {
            if clips.len() > MAX_SVG_DEPTH {
                return Err(limit("clip paths applied to one group", MAX_SVG_DEPTH));
            }
            let mut items = Vec::new();
            let in_clip = Context { opacity: 1.0, clip: true, depth: inner.depth };
            self.children(clip.root(), ctm * affine(clip.transform()), in_clip, &mut items)?;
            clips.push(items);
            next = clip.clip_path();
        }
        let mut masks = Vec::new();
        let mut next = if masked { g.mask() } else { None };
        while let Some(mask) = next {
            if masks.len() > MAX_SVG_DEPTH {
                return Err(limit("masks applied to one group", MAX_SVG_DEPTH));
            }
            let mut items = Vec::new();
            self.children(mask.root(), ctm, Context { opacity: 1.0, clip: false, depth: inner.depth }, &mut items)?;
            let luminance = mask.kind() == usvg::MaskType::Luminance;
            masks.push(SvgMask { luminance, region: rect_outline(rect_of(mask.rect()), flip() * ctm), items });
            next = mask.mask();
        }
        let mut items = Vec::new();
        self.children(g, ctm, inner, &mut items)?;
        self.make(1)?;
        out.push(SvgItem::Group(SvgGroup { items, clips, masks }));
        Ok(())
    }

    fn image(&mut self, image: &usvg::Image, ctm: Affine, opacity: f64, out: &mut Vec<SvgItem>) -> Result<(), ColorError> {
        let data = match image.kind() {
            usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) => d.clone(),
            _ => return Ok(()),
        };
        self.make(1)?;
        let size = image.size();
        let place = flip() * ctm * Affine::scale_non_uniform(f64::from(size.width()), f64::from(size.height()));
        out.push(SvgItem::Image(SvgImage { data, place, opacity: opacity.clamp(0.0, 1.0) }));
        Ok(())
    }

    fn path(&mut self, path: &usvg::Path, ctm: Affine, cx: Context, out: &mut Vec<SvgItem>) -> Result<(), ColorError> {
        let place = flip() * ctm;
        let fill = path.fill().map(|f| (f.paint(), f64::from(f.opacity().get()), bez(path.data(), place)));
        let stroke = path.stroke().filter(|_| !cx.clip).and_then(|s| {
            let outline = path.data().stroke(&s.to_tiny_skia(), 1.0)?;
            Some((s.paint(), f64::from(s.opacity().get()), bez(&outline, place)))
        });
        let ordered = match path.paint_order() {
            usvg::PaintOrder::FillAndStroke => [fill, stroke],
            usvg::PaintOrder::StrokeAndFill => [stroke, fill],
        };
        for (paint, opacity, outline) in ordered.into_iter().flatten() {
            if outline.elements().is_empty() {
                continue;
            }
            if cx.clip {
                self.make(1)?;
                out.push(solid(outline));
                continue;
            }
            let opacity = cx.opacity * opacity;
            match paint {
                usvg::Paint::Pattern(pattern) if self.full => self.pattern(pattern, outline, ctm, opacity, cx.depth, out)?,
                _ => {
                    if let Some(paint) = paint_of(paint, opacity, place) {
                        self.make(1)?;
                        out.push(SvgItem::Shape(SvgShape { outline, paint }));
                    }
                }
            }
        }
        Ok(())
    }

    /// `outline` (font units) filled with `pattern` used in the coordinates `ctm` maps: the
    /// tiles over the outline, each its content clipped to its rectangle, clipped together to
    /// the outline. Tile content is drawn from the tile's top left, as resvg draws it.
    fn pattern(
        &mut self,
        pattern: &usvg::Pattern,
        outline: BezPath,
        ctm: Affine,
        opacity: f64,
        depth: usize,
        out: &mut Vec<SvgItem>,
    ) -> Result<(), ColorError> {
        let space = ctm * affine(pattern.transform());
        let to_font = flip() * space;
        let inverse = to_font.inverse();
        if !(finite(to_font) && finite(inverse)) {
            return Ok(());
        }
        let tile = rect_of(pattern.rect());
        let (w, h) = (tile.width(), tile.height());
        let over = inverse.transform_rect_bbox(outline.bounding_box());
        // A shape edge within rounding of a tile edge does not reach into the next tile.
        let (i0, j0) = (((over.x0 - tile.x0) / w + EDGE).floor(), ((over.y0 - tile.y0) / h + EDGE).floor());
        let (columns, rows) = (((over.x1 - tile.x0) / w - EDGE).ceil() - i0, ((over.y1 - tile.y0) / h - EDGE).ceil() - j0);
        if !(columns >= 1.0 && rows >= 1.0) {
            return Ok(());
        }
        let count = columns * rows;
        if !(count.is_finite() && i0.is_finite() && j0.is_finite()) || count > (MAX_PATTERN_TILES - self.tiles.min(MAX_PATTERN_TILES)) as f64 {
            return Err(limit("pattern tiles drawn in one SVG file", MAX_PATTERN_TILES));
        }
        self.tiles += count.max(0.0) as usize;
        let mut tiles = Vec::new();
        for j in 0..rows.max(0.0) as usize {
            for i in 0..columns.max(0.0) as usize {
                let at = (tile.x0 + (i0 + i as f64) * w, tile.y0 + (j0 + j as f64) * h);
                let place = space * Affine::translate(at);
                let mut items = Vec::new();
                self.children(pattern.root(), place, Context { opacity, clip: false, depth: depth + 1 }, &mut items)?;
                if items.is_empty() {
                    continue;
                }
                self.make(2)?;
                let clip = rect_outline(Rect::new(0.0, 0.0, w, h), flip() * place);
                tiles.push(SvgItem::Group(SvgGroup { items, clips: vec![vec![solid(clip)]], masks: Vec::new() }));
            }
        }
        if !tiles.is_empty() {
            self.make(2)?;
            out.push(SvgItem::Group(SvgGroup { items: tiles, clips: vec![vec![solid(outline)]], masks: Vec::new() }));
        }
        Ok(())
    }
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
    fn clip_paths_masks_and_patterns_become_groups_for_text_preview_only() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><clipPath id="c"><rect x="0" y="0" width="50" height="100"/></clipPath>
              <mask id="m"><rect x="0" y="0" width="100" height="100" fill="#ffffff"/></mask>
              <pattern id="p" patternUnits="userSpaceOnUse" width="25" height="25"><rect width="10" height="10" fill="#ff0000"/></pattern></defs>
            <rect width="100" height="100" fill="#00ff00" clip-path="url(#c)" mask="url(#m)"/>
            <rect width="100" height="50" fill="url(#p)"/>
        </svg>"##;
        let drawn = items(svg, |_, _| Affine::IDENTITY).unwrap();
        let [SvgItem::Group(clipped), SvgItem::Group(patterned)] = drawn.as_slice() else { panic!("{drawn:?}") };
        assert_eq!((clipped.items.len(), clipped.clips.len(), clipped.masks.len()), (1, 1, 1));
        assert!(clipped.masks[0].luminance);
        assert_eq!(patterned.clips.len(), 1, "clipped to the shape");
        assert_eq!(patterned.items.len(), 8, "4 × 2 tiles of 25 units over 100 × 50");
        // Edit View draws the clipped square whole and leaves the pattern out.
        assert_eq!(shapes(svg, Affine::IDENTITY).unwrap().len(), 1);

        // A pattern of more tiles than the limit refuses the file.
        let fine_grained = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="1000">
            <defs><pattern id="p" patternUnits="userSpaceOnUse" width="1" height="1"><rect width="1" height="1" fill="#ff0000"/></pattern></defs>
            <rect width="1000" height="1000" fill="url(#p)"/></svg>"##;
        assert!(matches!(items(fine_grained, |_, _| Affine::IDENTITY), Err(ColorError::LimitExceeded { .. })));
    }

    #[test]
    fn a_file_export_refuses_draws_nothing() {
        let external = br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///etc/passwd" width="1" height="1"/></svg>"#;
        assert!(matches!(shapes(external, Affine::IDENTITY), Err(ColorError::BadSvg(_))));
        let deep = format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>", "<g>".repeat(10_000), "</g>".repeat(10_000));
        assert!(shapes(deep.as_bytes(), Affine::IDENTITY).is_err());
    }
}
