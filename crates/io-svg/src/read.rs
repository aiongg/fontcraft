//! SVG drawings as glyph paths.

use fontcraft_model::{MAX_COORDINATE, Node, NodeKind, Path};
use usvg::tiny_skia_path::{self, PathSegment};

use crate::sanitize::{Sanitized, sanitize};
use crate::{Dropped, MAX_SVG_BYTES, MAX_SVG_DEPTH, MAX_SVG_NODES, MAX_SVG_PATHS, SvgError};

/// Where a drawing lands in glyph coordinates. SVG's y axis points down and a glyph's points up,
/// so every placement flips y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    /// One SVG user unit per font unit, the scale vector apps use for the clipboard
    /// (`paste.onePointIsOneUnit`): a point at SVG `(x, y)` lands at `(x, top - y)`.
    Unscaled { top: f64 },
    /// The drawing's box (the root `viewBox`, or else its width and height) scaled to the em:
    /// the box's height becomes `units_per_em`, its top edge lands at `ascender` and its left
    /// edge at x = 0.
    FitEm { units_per_em: f64, ascender: f64 },
}

/// What [`read_svg`] found.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgOutlines {
    /// The contours, in document order: closed or open, lines and cubic curves, on-curve nodes
    /// marked smooth where the curve runs through them without a corner. Coordinates are
    /// rounded to 0.001 units (SVG geometry is computed in 32-bit floats).
    pub paths: Vec<Path>,
    /// The root `viewBox` as `[x, y, width, height]`, in SVG user units, when it has one.
    pub view_box: Option<[f64; 4]>,
    /// What was left out because it is not geometry.
    pub dropped: Dropped,
}

/// The geometry of an SVG document (UTF-8 text) as glyph paths, placed by `placement`.
///
/// Paths and basic shapes (`rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`) are read,
/// through groups, `use` references, symbols and nested `svg` elements, with every transform
/// applied; arcs and quadratic segments become cubic ones. Styling and everything that is not
/// geometry is left out and reported in [`SvgOutlines::dropped`]. A stroked path gives its centre
/// line, not the stroke's outline.
pub fn read_svg(data: &[u8], placement: Placement) -> Result<SvgOutlines, SvgError> {
    if data.len() > MAX_SVG_BYTES {
        return Err(SvgError::TooLarge { size: data.len() });
    }
    if data.starts_with(&[0x1f, 0x8b]) {
        return Err(SvgError::Compressed);
    }
    let text = std::str::from_utf8(data).map_err(|_| SvgError::NotUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Sanitized { text: clean, view_box, dropped } = sanitize(text)?;

    let options = usvg::Options {
        // Nothing is loaded from outside the document. The sanitized document has no images;
        // this is the second guard.
        image_href_resolver: usvg::ImageHrefResolver { resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None) },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&clean, &options).map_err(|e| SvgError::Parse(e.to_string()))?;
    let size = tree.size();
    let frame = match view_box {
        Some(vb) => vb,
        None => [0.0, 0.0, f64::from(size.width()), f64::from(size.height())],
    };
    let mapping = Mapping::new(placement, frame)?;
    let mut out = Collected { paths: Vec::new(), nodes: 0 };
    collect_group(tree.root(), &mapping, &mut out, 0)?;
    Ok(SvgOutlines { paths: out.paths, view_box, dropped })
}

/// SVG user space to glyph coordinates: `x' = (x - x0) * scale`, `y' = top - (y - y0) * scale`.
struct Mapping {
    scale: f64,
    x0: f64,
    y0: f64,
    top: f64,
}

impl Mapping {
    fn new(placement: Placement, frame: [f64; 4]) -> Result<Self, SvgError> {
        let usable = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        match placement {
            Placement::Unscaled { top } => {
                if !usable(top) {
                    return Err(SvgError::BadPlacement(format!("the top {top} is not a number or is beyond ±{MAX_COORDINATE}")));
                }
                Ok(Mapping { scale: 1.0, x0: 0.0, y0: 0.0, top })
            }
            Placement::FitEm { units_per_em, ascender } => {
                if !(usable(units_per_em) && units_per_em > 0.0 && usable(ascender)) {
                    return Err(SvgError::BadPlacement(format!(
                        "units per em {units_per_em} and ascender {ascender} must be finite, with a positive em"
                    )));
                }
                let [x0, y0, _, height] = frame;
                if !(height.is_finite() && height > 0.0 && x0.is_finite() && y0.is_finite()) {
                    return Err(SvgError::BadPlacement("the drawing has no height to scale to the em".into()));
                }
                Ok(Mapping { scale: units_per_em / height, x0, y0, top: ascender })
            }
        }
    }

    fn apply(&self, x: f64, y: f64) -> Result<(f64, f64), SvgError> {
        let fx = round((x - self.x0) * self.scale);
        let fy = round(self.top - (y - self.y0) * self.scale);
        if fx.is_finite() && fy.is_finite() && fx.abs() <= MAX_COORDINATE && fy.abs() <= MAX_COORDINATE {
            Ok((fx, fy))
        } else {
            Err(SvgError::CoordinateOutOfRange)
        }
    }
}

fn round(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    // No negative zero in the output.
    if r == 0.0 { 0.0 } else { r }
}

struct Collected {
    paths: Vec<Path>,
    nodes: usize,
}

/// usvg adds a few groups of its own (for `use`, nested `svg`, the root), so its tree may be
/// somewhat deeper than the source.
const MAX_TREE_DEPTH: usize = 2 * MAX_SVG_DEPTH + 8;

fn collect_group(group: &usvg::Group, mapping: &Mapping, out: &mut Collected, depth: usize) -> Result<(), SvgError> {
    if depth > MAX_TREE_DEPTH {
        return Err(SvgError::TooDeep);
    }
    for child in group.children() {
        match child {
            usvg::Node::Group(g) => collect_group(g, mapping, out, depth + 1)?,
            usvg::Node::Path(p) => collect_path(p, mapping, out)?,
            // Images and text never reach here: the sanitized document has neither.
            _ => {}
        }
    }
    Ok(())
}

fn collect_path(path: &usvg::Path, mapping: &Mapping, out: &mut Collected) -> Result<(), SvgError> {
    let t = path.abs_transform();
    let (sx, kx, tx, ky, sy, ty) = (f64::from(t.sx), f64::from(t.kx), f64::from(t.tx), f64::from(t.ky), f64::from(t.sy), f64::from(t.ty));
    let place = |p: tiny_skia_path::Point| {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        mapping.apply(sx * x + kx * y + tx, ky * x + sy * y + ty)
    };
    let mut contour = Contour::default();
    for segment in path.data().segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                contour.finish(out)?;
                contour.start(place(p)?);
            }
            PathSegment::LineTo(p) => contour.line(place(p)?),
            PathSegment::QuadTo(c, p) => contour.quad(place(c)?, place(p)?),
            PathSegment::CubicTo(c1, c2, p) => contour.cubic(place(c1)?, place(c2)?, place(p)?),
            PathSegment::Close => {
                contour.close();
                contour.finish(out)?;
            }
        }
        if out.nodes.saturating_add(contour.nodes.len()) > MAX_SVG_NODES {
            return Err(SvgError::TooManyNodes);
        }
    }
    contour.finish(out)
}

type Pt = (f64, f64);

/// One contour being built. The first node is the start point; segments append their off-curve
/// nodes and end node. Closing removes an end node that repeats the start and gives its kind to
/// the start node, so the last segment wraps around as the model expects.
#[derive(Default)]
struct Contour {
    nodes: Vec<Node>,
    closed: bool,
    start: Option<Pt>,
    last: Option<Pt>,
}

impl Contour {
    fn start(&mut self, p: Pt) {
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Line));
        self.start = Some(p);
        self.last = Some(p);
    }

    /// A segment that follows a close without a move starts at the closed contour's start.
    fn ensure_started(&mut self) -> Pt {
        let from = self.last.unwrap_or((0.0, 0.0));
        if self.nodes.is_empty() {
            self.start(from);
        }
        from
    }

    fn line(&mut self, p: Pt) {
        self.ensure_started();
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Line));
        self.last = Some(p);
    }

    /// Degree elevation: the cubic with the same shape as the quadratic.
    fn quad(&mut self, c: Pt, p: Pt) {
        let from = self.ensure_started();
        let c1 = (round(from.0 + 2.0 / 3.0 * (c.0 - from.0)), round(from.1 + 2.0 / 3.0 * (c.1 - from.1)));
        let c2 = (round(p.0 + 2.0 / 3.0 * (c.0 - p.0)), round(p.1 + 2.0 / 3.0 * (c.1 - p.1)));
        self.cubic(c1, c2, p);
    }

    fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt) {
        self.ensure_started();
        self.nodes.push(Node::new(c1.0, c1.1, NodeKind::OffCurve));
        self.nodes.push(Node::new(c2.0, c2.1, NodeKind::OffCurve));
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Curve));
        self.last = Some(p);
    }

    fn close(&mut self) {
        let repeats_start = self.nodes.len() > 1 && self.nodes.last().map(|n| (n.x, n.y)) == self.start;
        if repeats_start
            && let Some(end) = self.nodes.pop()
            && let Some(first) = self.nodes.first_mut()
        {
            first.kind = end.kind;
        }
        self.closed = true;
        self.last = self.start;
    }

    /// Moves the contour into `out` when it has at least two nodes, and starts afresh.
    fn finish(&mut self, out: &mut Collected) -> Result<(), SvgError> {
        let closed = self.closed;
        let mut nodes = std::mem::take(&mut self.nodes);
        self.closed = false;
        if nodes.len() < 2 {
            return Ok(());
        }
        if out.paths.len() >= MAX_SVG_PATHS {
            return Err(SvgError::TooManyPaths);
        }
        out.nodes = out.nodes.saturating_add(nodes.len());
        if out.nodes > MAX_SVG_NODES {
            return Err(SvgError::TooManyNodes);
        }
        mark_smooth(&mut nodes, closed);
        out.paths.push(Path::new(closed, nodes));
        Ok(())
    }
}

/// Marks an on-curve node smooth when a curve passes through it without a corner: at least one
/// neighbour is an off-curve node, and the incoming and outgoing directions agree to within
/// about 0.06°.
fn mark_smooth(nodes: &mut [Node], closed: bool) {
    let count = nodes.len();
    let flags: Vec<bool> = (0..count)
        .map(|i| {
            let open_end = !closed && (i == 0 || i + 1 == count);
            let prev = nodes.get((i + count - 1) % count);
            let next = nodes.get((i + 1) % count);
            match (nodes.get(i), prev, next) {
                (Some(n), Some(a), Some(b)) if !open_end && n.on_curve() && (!a.on_curve() || !b.on_curve()) => {
                    let (ix, iy) = (n.x - a.x, n.y - a.y);
                    let (ox, oy) = (b.x - n.x, b.y - n.y);
                    let lengths = ix.hypot(iy) * ox.hypot(oy);
                    lengths > 0.0 && ix * ox + iy * oy > 0.0 && (ix * oy - iy * ox).abs() <= 1e-3 * lengths
                }
                _ => false,
            }
        })
        .collect();
    for (node, smooth) in nodes.iter_mut().zip(flags) {
        node.smooth = smooth;
    }
}
