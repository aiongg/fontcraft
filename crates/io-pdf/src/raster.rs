//! Drawing the painted paths of a page to RGBA pixels, with anti-aliasing.
//!
//! Each painted path is drawn on its own within its bounding box: its edges add signed coverage
//! to an accumulation buffer, a running sum along each row gives the winding number at each
//! pixel (fractional at the edges), and the fill rule turns that into coverage, which is
//! composited over the page in the path's colour. Strokes are drawn as the union of one
//! rectangle per flattened segment and a small octagon at each vertex (round joins and caps,
//! approximately); the page is a template to draw over, not a proof.

use fontcraft_imagery::{MAX_PIXELS, MAX_SIDE, Rgba8};

use crate::content::{FillRule, PageDrawing, Painted, Pt, Seg};
use crate::outline::bounds;
use crate::{MAX_PDF_RENDER_WORK, MAX_PDF_SEGMENTS, PdfError};

/// The flatness of curves, in pixels.
const TOLERANCE: f64 = 0.2;
/// Most lines one curve becomes.
const MAX_CURVE_STEPS: usize = 128;
/// The highest resolution asked for that is honoured.
const MAX_PIXELS_PER_POINT: f64 = 16.0;

/// A polygon edge in pixel space (y down).
type Edge = (Pt, Pt);

struct Canvas {
    width: usize,
    height: usize,
    /// Straight RGBA, four bytes a pixel.
    pixels: Vec<u8>,
    work: u64,
    edges: usize,
}

pub(crate) fn render(page: &PageDrawing, pixels_per_point: f64, max_pixels: u64) -> Result<(Rgba8, f64), PdfError> {
    let frame = page.frame.or_else(|| bounds(&page.painted)).ok_or(PdfError::NoPage)?;
    let [x0, y0, x1, y1] = frame;
    let (w_pt, h_pt) = (x1 - x0, y1 - y0);
    if !(w_pt.is_finite() && h_pt.is_finite() && w_pt > 0.0 && h_pt > 0.0) {
        return Err(PdfError::NoPage);
    }
    let budget = max_pixels.min(MAX_PIXELS) as f64;
    let mut scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point.min(MAX_PIXELS_PER_POINT) } else { 1.0 };
    let side = f64::from(MAX_SIDE);
    scale = scale.min(side / w_pt).min(side / h_pt).min((budget / (w_pt * h_pt)).sqrt());
    let mut width = (w_pt * scale).ceil().max(1.0);
    let mut height = (h_pt * scale).ceil().max(1.0);
    // Rounding the sides up can take the page just past the budget; shrink until it fits.
    for _ in 0..8 {
        if width * height <= budget {
            break;
        }
        scale *= (budget / (width * height)).sqrt() * 0.999;
        width = (w_pt * scale).ceil().max(1.0);
        height = (h_pt * scale).ceil().max(1.0);
    }
    if !(scale > 0.0 && width <= side && height <= side && width * height <= budget.max(1.0)) {
        return Err(PdfError::TooMuchWork);
    }
    // The page's bottom edge is the image's, as the image is placed from its bottom-left corner;
    // a part row from rounding the height up lies at the top.
    let bottom = height;
    let (width, height) = (width as usize, height as usize);
    let mut canvas = Canvas { width, height, pixels: vec![0; width.saturating_mul(height).saturating_mul(4)], work: 0, edges: 0 };
    let to_px = |(x, y): Pt| ((x - x0) * scale, bottom - (y - y0) * scale);

    for p in &page.painted {
        if let Some(rule) = p.fill {
            let edges = fill_edges(p, &to_px, &mut canvas)?;
            canvas.paint(&edges, rule, p.fill_rgb)?;
        }
        if let Some(width) = p.stroke {
            let edges = stroke_edges(p, &to_px, (width * scale).max(1.0), &mut canvas)?;
            canvas.paint(&edges, FillRule::NonZero, p.stroke_rgb)?;
        }
    }

    Ok((Rgba8 { width: width as u32, height: height as u32, pixels: canvas.pixels }, scale))
}

/// The points of a cubic as line ends, the start excluded.
fn flatten(p0: Pt, c1: Pt, c2: Pt, p3: Pt, out: &mut Vec<Pt>) {
    let dd = |a: Pt, b: Pt, c: Pt| (a.0 - 2.0 * b.0 + c.0).hypot(a.1 - 2.0 * b.1 + c.1);
    let d = dd(p0, c1, c2).max(dd(c1, c2, p3));
    let steps = ((0.75 * d / TOLERANCE).sqrt().ceil() as usize).clamp(1, MAX_CURVE_STEPS);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let u = 1.0 - t;
        let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((a * p0.0 + b * c1.0 + c * c2.0 + e * p3.0, a * p0.1 + b * c1.1 + c * c2.1 + e * p3.1));
    }
}

/// The subpaths of a painted path as polylines in pixel space, each with whether it was closed.
fn polylines(p: &Painted, to_px: &impl Fn(Pt) -> Pt, canvas: &mut Canvas) -> Result<Vec<(Vec<Pt>, bool)>, PdfError> {
    let mut out: Vec<(Vec<Pt>, bool)> = Vec::new();
    let mut current: Vec<Pt> = Vec::new();
    for s in &p.segs {
        let before = current.len();
        match *s {
            Seg::Move(pt) => {
                if current.len() > 1 {
                    out.push((std::mem::take(&mut current), false));
                }
                current.clear();
                current.push(to_px(pt));
            }
            Seg::Line(pt) => current.push(to_px(pt)),
            Seg::Curve(c1, c2, pt) => {
                let from = current.last().copied().unwrap_or_else(|| to_px(c1));
                flatten(from, to_px(c1), to_px(c2), to_px(pt), &mut current);
            }
            Seg::Close => {
                let start = current.first().copied();
                if current.len() > 1 {
                    out.push((std::mem::take(&mut current), true));
                }
                current.clear();
                current.extend(start);
            }
        }
        canvas.edges = canvas.edges.saturating_add(current.len().saturating_sub(before));
        if canvas.edges > MAX_PDF_SEGMENTS.saturating_mul(4) {
            return Err(PdfError::TooMuchWork);
        }
    }
    if current.len() > 1 {
        out.push((current, false));
    }
    Ok(out)
}

fn fill_edges(p: &Painted, to_px: &impl Fn(Pt) -> Pt, canvas: &mut Canvas) -> Result<Vec<Edge>, PdfError> {
    let mut edges = Vec::new();
    for (line, _) in polylines(p, to_px, canvas)? {
        // A fill closes every subpath.
        for (a, b) in line.iter().zip(line.iter().skip(1).chain(line.first())) {
            edges.push((*a, *b));
        }
    }
    Ok(edges)
}

/// Adds `poly` as a closed polygon turning the positive way, so that overlapping pieces add up.
fn add_positive(poly: &[Pt], edges: &mut Vec<Edge>) {
    let area: f64 = poly.iter().zip(poly.iter().skip(1).chain(poly.first())).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum();
    let pairs = poly.iter().zip(poly.iter().skip(1).chain(poly.first()));
    if area >= 0.0 {
        edges.extend(pairs.map(|(a, b)| (*a, *b)));
    } else {
        edges.extend(pairs.map(|(a, b)| (*b, *a)));
    }
}

fn stroke_edges(p: &Painted, to_px: &impl Fn(Pt) -> Pt, width: f64, canvas: &mut Canvas) -> Result<Vec<Edge>, PdfError> {
    let r = width / 2.0;
    let mut edges = Vec::new();
    let octagon = |c: Pt| -> Vec<Pt> {
        (0..8)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_4 * (i as f64 + 0.5);
                (c.0 + r * a.cos(), c.1 + r * a.sin())
            })
            .collect()
    };
    for (line, closed) in polylines(p, to_px, canvas)? {
        let next: Box<dyn Iterator<Item = &Pt>> =
            if closed { Box::new(line.iter().skip(1).chain(line.first())) } else { Box::new(line.iter().skip(1)) };
        for (a, b) in line.iter().zip(next) {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = dx.hypot(dy);
            if len > 0.0 {
                let (nx, ny) = (-dy / len * r, dx / len * r);
                add_positive(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], &mut edges);
            }
        }
        for v in &line {
            add_positive(&octagon(*v), &mut edges);
        }
    }
    Ok(edges)
}

impl Canvas {
    /// Composites `edges`, filled by `rule`, over the page in `rgb`.
    fn paint(&mut self, edges: &[Edge], rule: FillRule, rgb: [f64; 3]) -> Result<(), PdfError> {
        let mut b: Option<[f64; 4]> = None;
        for (p, q) in edges {
            for (x, y) in [*p, *q] {
                b = Some(match b {
                    None => [x, y, x, y],
                    Some([a, c, d, e]) => [a.min(x), c.min(y), d.max(x), e.max(y)],
                });
            }
        }
        let Some([bx0, by0, bx1, by1]) = b else { return Ok(()) };
        if !(bx0.is_finite() && by0.is_finite() && bx1.is_finite() && by1.is_finite()) {
            return Err(PdfError::NumberOutOfRange);
        }
        // The box in whole pixels, within the canvas.
        let left = bx0.floor().clamp(0.0, self.width as f64) as usize;
        let right = bx1.ceil().clamp(0.0, self.width as f64) as usize;
        let top = by0.floor().clamp(0.0, self.height as f64) as usize;
        let bottom = by1.ceil().clamp(0.0, self.height as f64) as usize;
        if left >= right || top >= bottom {
            return Ok(());
        }
        let (w, h) = (right - left, bottom - top);
        // An edge costs the rows it crosses and the cells it runs along within them.
        let edge_cost: f64 = edges.iter().map(|(p, q)| 1.0 + (q.1 - p.1).abs().min(h as f64) + (q.0 - p.0).abs().min(w as f64 + 2.0)).sum();
        self.work = self.work.saturating_add((w as u64).saturating_mul(h as u64)).saturating_add(edge_cost as u64);
        if self.work > MAX_PDF_RENDER_WORK {
            return Err(PdfError::TooMuchWork);
        }
        // Two spare columns take coverage from edges right of the box.
        let stride = w + 2;
        let mut acc = vec![0.0f32; stride.saturating_mul(h)];
        for (p, q) in edges {
            let shift = |(x, y): Pt| (x - left as f64, y - top as f64);
            accumulate(&mut acc, stride, h, shift(*p), shift(*q));
        }
        let [r, g, bl] = rgb.map(|v| v as f32);
        for row in 0..h {
            let mut winding = 0.0f32;
            for col in 0..w {
                winding += acc.get(row * stride + col).copied().unwrap_or(0.0);
                let cover = match rule {
                    FillRule::NonZero => winding.abs().min(1.0),
                    FillRule::EvenOdd => {
                        let m = winding.abs() % 2.0;
                        if m > 1.0 { 2.0 - m } else { m }
                    }
                };
                if cover <= 0.0 {
                    continue;
                }
                let at = ((top + row) * self.width + left + col) * 4;
                let Some(px) = self.pixels.get_mut(at..at + 4) else { continue };
                let unit = |v: u8| f32::from(v) / 255.0;
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let (dr, dg, db, da) = match &*px {
                    [r, g, b, a] => (unit(*r), unit(*g), unit(*b), unit(*a)),
                    _ => continue,
                };
                let out_a = cover + da * (1.0 - cover);
                if out_a > 0.0 {
                    let mix = |s: f32, d: f32| (s * cover + d * da * (1.0 - cover)) / out_a;
                    px.copy_from_slice(&[byte(mix(r, dr)), byte(mix(g, dg)), byte(mix(bl, db)), byte(out_a)]);
                }
            }
        }
        Ok(())
    }
}

fn add_at(acc: &mut [f32], index: usize, v: f32) {
    if let Some(cell) = acc.get_mut(index) {
        *cell += v;
    }
}

/// Adds the signed area one line contributes to each cell of the buffer (`stride` cells a row,
/// `rows` rows). Points left of the buffer act on its first column and points right of it on
/// its spare columns, which keeps the winding of every cell inside.
fn accumulate(acc: &mut [f32], stride: usize, rows: usize, p: Pt, q: Pt) {
    if p.1 == q.1 {
        return;
    }
    let (dir, p, q) = if p.1 < q.1 { (1.0, p, q) } else { (-1.0, q, p) };
    let max_x = (stride - 1) as f64 - 1e-6;
    let dxdy = (q.0 - p.0) / (q.1 - p.1);
    let first = p.1.max(0.0);
    let last = q.1.min(rows as f64);
    if first >= last {
        return;
    }
    let mut x = p.0 + (first - p.1) * dxdy;
    let mut y = first.floor();
    while y < last {
        let row = y as usize;
        let y_top = y.max(first);
        let y_bottom = (y + 1.0).min(last);
        let dy = y_bottom - y_top;
        let x_next = x + dxdy * dy;
        let d = (dy * dir) as f32;
        let (xa, xb) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let (xa, xb) = (xa.clamp(0.0, max_x), xb.clamp(0.0, max_x));
        let base = row * stride;
        let a_floor = xa.floor();
        let ai = a_floor as usize;
        let b_ceil = xb.ceil();
        let bi = b_ceil as usize;
        if bi <= ai + 1 {
            // The line stays in one cell: split its area between that cell and the next.
            let mid = (0.5 * (xa + xb) - a_floor) as f32;
            add_at(acc, base + ai, d - d * mid);
            add_at(acc, base + ai + 1, d * mid);
        } else {
            let s = (1.0 / (xb - xa)) as f32;
            let fa = (xa - a_floor) as f32;
            let a0 = 0.5 * s * (1.0 - fa) * (1.0 - fa);
            let fb = (xb - b_ceil + 1.0) as f32;
            let am = 0.5 * s * fb * fb;
            add_at(acc, base + ai, d * a0);
            if bi == ai + 2 {
                add_at(acc, base + ai + 1, d * (1.0 - a0 - am));
            } else {
                let a1 = s * (1.5 - fa);
                add_at(acc, base + ai + 1, d * (a1 - a0));
                for xi in ai + 2..bi - 1 {
                    add_at(acc, base + xi, d * s);
                }
                let a2 = a1 + (bi - ai - 3) as f32 * s;
                add_at(acc, base + bi - 1, d * (1.0 - a2 - am));
            }
            add_at(acc, base + bi, d * am);
        }
        x = x_next;
        y += 1.0;
    }
}
