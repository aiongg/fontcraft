//! Painted paths as glyph paths: placement, contours, smooth nodes and the stroke width.

use fontcraft_model::{Dimension, MAX_COORDINATE, Node, NodeKind, Path};

use crate::content::{PageDrawing, Painted, Pt, Seg};
use crate::{MAX_PDF_NODES, MAX_PDF_PATHS, PdfError, Placement};

/// Rounds to 0.001 units, without a negative zero.
fn round(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 { 0.0 } else { r }
}

/// The bounds of every point of the painted paths, control points included, as
/// `[llx, lly, urx, ury]`.
pub(crate) fn bounds(painted: &[Painted]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |(x, y): Pt| {
        b = Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    };
    for s in painted.iter().flat_map(|p| p.segs.iter()) {
        match *s {
            Seg::Move(p) | Seg::Line(p) => add(p),
            Seg::Curve(c1, c2, p) => {
                add(c1);
                add(c2);
                add(p);
            }
            Seg::Close => {}
        }
    }
    b
}

/// Shown page space to glyph coordinates: `x' = (x - x0) * scale`, `y' = top - (y1 - y) * scale`,
/// where `y1` is the frame's top edge.
struct PageMap {
    scale: f64,
    x0: f64,
    y1: f64,
    top: f64,
}

impl PageMap {
    fn new(placement: Placement, [llx, lly, _, ury]: [f64; 4]) -> Result<PageMap, PdfError> {
        let fits = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        match placement {
            Placement::Unscaled { top } => {
                if !fits(top) {
                    return Err(PdfError::BadPlacement(format!("the top {top} is not a number or is beyond ±{MAX_COORDINATE}")));
                }
                Ok(PageMap { scale: 1.0, x0: 0.0, y1: ury, top })
            }
            Placement::FitEm { units_per_em, ascender } => {
                if !(fits(units_per_em) && units_per_em > 0.0 && fits(ascender)) {
                    return Err(PdfError::BadPlacement(format!(
                        "units per em {units_per_em} and ascender {ascender} must be finite, with a positive em"
                    )));
                }
                let height = ury - lly;
                if !(height.is_finite() && height > 0.0) {
                    return Err(PdfError::BadPlacement("the drawing has no height to scale to the em".into()));
                }
                Ok(PageMap { scale: units_per_em / height, x0: llx, y1: ury, top: ascender })
            }
        }
    }

    fn apply(&self, (x, y): Pt) -> Result<Pt, PdfError> {
        let fx = round((x - self.x0) * self.scale);
        let fy = round(self.top - (self.y1 - y) * self.scale);
        if fx.is_finite() && fy.is_finite() && fx.abs() <= MAX_COORDINATE && fy.abs() <= MAX_COORDINATE {
            Ok((fx, fy))
        } else {
            Err(PdfError::CoordinateOutOfRange)
        }
    }
}

#[derive(Default)]
struct Collected {
    paths: Vec<Path>,
    nodes: usize,
}

#[derive(Default)]
struct Contour {
    nodes: Vec<Node>,
    closed: bool,
    start: Option<Pt>,
}

impl Contour {
    fn start(&mut self, p: Pt) {
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Line));
        self.start = Some(p);
    }

    fn line(&mut self, p: Pt) {
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Line));
    }

    fn cubic(&mut self, c1: Pt, c2: Pt, p: Pt) {
        self.nodes.push(Node::new(c1.0, c1.1, NodeKind::OffCurve));
        self.nodes.push(Node::new(c2.0, c2.1, NodeKind::OffCurve));
        self.nodes.push(Node::new(p.0, p.1, NodeKind::Curve));
    }

    /// Closes the contour; a last node on the start point merges into the first node.
    fn close(&mut self) {
        let repeats_start = self.nodes.len() > 1 && self.nodes.last().map(|n| (n.x, n.y)) == self.start;
        if repeats_start
            && let Some(end) = self.nodes.pop()
            && let Some(first) = self.nodes.first_mut()
        {
            first.kind = end.kind;
        }
        self.closed = true;
    }

    /// Moves the contour into `out` when it has at least two nodes, and starts afresh.
    fn finish(&mut self, out: &mut Collected, painted: &Painted, scale: f64) -> Result<(), PdfError> {
        let closed = std::mem::take(&mut self.closed);
        let mut nodes = std::mem::take(&mut self.nodes);
        self.start = None;
        if nodes.len() < 2 {
            return Ok(());
        }
        if out.paths.len() >= MAX_PDF_PATHS {
            return Err(PdfError::TooManyPaths);
        }
        out.nodes = out.nodes.saturating_add(nodes.len());
        if out.nodes > MAX_PDF_NODES {
            return Err(PdfError::TooManyNodes);
        }
        mark_smooth(&mut nodes, closed);
        let mut path = Path::new(closed, nodes);
        if let Some(width) = painted.stroke {
            // A hairline (width 0) draws the thinnest line a device can; one unit stands for it.
            let w = round(width * scale);
            if !(w.is_finite() && w.abs() <= MAX_COORDINATE) {
                return Err(PdfError::CoordinateOutOfRange);
            }
            path.info.attrs.stroke_width = Some(Dimension::Number(if w > 0.0 { w } else { 1.0 }));
            path.info.attrs.fill = painted.fill.is_some();
        }
        out.paths.push(path);
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

/// The contours of one painted path. A fill closes every subpath; a stroke keeps a subpath open
/// unless the content closed it.
fn build(p: &Painted, map: &PageMap, out: &mut Collected) -> Result<(), PdfError> {
    let end = |c: &mut Contour, out: &mut Collected| -> Result<(), PdfError> {
        if p.fill.is_some() && c.nodes.len() > 1 {
            c.close();
        }
        c.finish(out, p, map.scale)
    };
    let mut c = Contour::default();
    for s in &p.segs {
        match *s {
            Seg::Move(pt) => {
                end(&mut c, out)?;
                c.start(map.apply(pt)?);
            }
            Seg::Line(pt) => c.line(map.apply(pt)?),
            Seg::Curve(c1, c2, pt) => c.cubic(map.apply(c1)?, map.apply(c2)?, map.apply(pt)?),
            Seg::Close => {
                let start = c.start;
                c.close();
                c.finish(out, p, map.scale)?;
                // A segment after a close without a move starts at the closed subpath's start.
                if let Some(s) = start {
                    c.start(s);
                }
            }
        }
    }
    end(&mut c, out)
}

pub(crate) fn paths(page: &PageDrawing, placement: Placement) -> Result<Vec<Path>, PdfError> {
    if page.painted.is_empty() {
        return Ok(Vec::new());
    }
    let Some(frame) = page.frame.or_else(|| bounds(&page.painted)) else { return Ok(Vec::new()) };
    let map = PageMap::new(placement, frame)?;
    let mut out = Collected::default();
    for p in &page.painted {
        build(p, &map, &mut out)?;
    }
    Ok(out.paths)
}
