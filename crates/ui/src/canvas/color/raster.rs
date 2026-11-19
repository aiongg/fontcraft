//! Rasterising a colour glyph's fills: each fill's clip outlines become a coverage mask (`fontcraft_render`), and
//! its leaf paint (solid, linear, radial or sweep gradient) is evaluated per covered pixel and laid over the image.
//!
//! One glyph's drawing has a work budget ([`MAX_PAINT_WORK`], more for glyphs drawn large); a glyph that needs more
//! is drawn as one plain fill of its outlines, so a file of many large layers cannot stall a frame and no glyph is
//! shown with layers missing.

use fontcraft_geom::{Affine, Point, Shape as _};
use fontcraft_model::Extend;
use fontcraft_render::{Style, render_path};

use super::content::{Drawable, Leaf, Stops, concat};

/// Pixel operations (a fill's pixel box times its clips and its colour, summed over the fills) one glyph may take to
/// draw: at least this, or [`PAINT_WORK_PER_PIXEL`] per pixel of the glyph's own box when that is more, up to
/// [`MAX_GLYPH_PAINT_WORK`].
pub const MAX_PAINT_WORK: u64 = 64 << 20;
/// Pixel operations one pixel of a glyph's box allows, for glyphs drawn large: room for a few hundred layers that each
/// cover the whole glyph.
pub const PAINT_WORK_PER_PIXEL: u64 = 256;
/// Most pixel operations one glyph may take to draw, however large it is drawn.
pub const MAX_GLYPH_PAINT_WORK: u64 = 1 << 30;

/// Premultiplied RGBA pixels, rows top to bottom.
pub struct Pixels<'a> {
    pub data: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

const MASK: Style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };

/// `t` within 0 to 1 as the extend mode says.
fn extend(t: f64, mode: Extend) -> f64 {
    match mode {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let m = t.rem_euclid(2.0);
            if m > 1.0 { 2.0 - m } else { m }
        }
    }
}

fn sample(stops: &Stops, t: f64) -> [f32; 4] {
    let t = if t.is_finite() { extend(t, stops.extend) } else { 0.0 };
    let (Some(first), Some(last)) = (stops.at.first(), stops.at.last()) else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let i = stops.at.partition_point(|(o, _)| *o <= t);
    match (stops.at.get(i.wrapping_sub(1)), stops.at.get(i)) {
        (Some((o0, c0)), Some((o1, c1))) if o1 > o0 => {
            let k = ((t - o0) / (o1 - o0)) as f32;
            let mut out = [0.0; 4];
            for ((o, a), b) in out.iter_mut().zip(c0).zip(c1) {
                *o = a + (b - a) * k;
            }
            out
        }
        (Some((_, c)), _) | (_, Some((_, c))) => *c,
        _ => last.1,
    }
}

/// The colour of `leaf` at font point `p`; `None` where a radial gradient paints nothing.
pub fn eval(leaf: &Leaf, p: Point) -> Option<[f32; 4]> {
    match leaf {
        Leaf::Solid(c) => Some(*c),
        Leaf::Linear { p0, v, stops } => Some(sample(stops, (p.x - p0.x) * v.0 + (p.y - p0.y) * v.1)),
        Leaf::Radial { c0, r0, c1, r1, stops } => {
            // The largest t with |p − c(t)| = r(t) and r(t) ≥ 0, c and r moving from the start
            // circle to the end circle.
            let (cdx, cdy, dr) = (c1.x - c0.x, c1.y - c0.y, r1 - r0);
            let (pdx, pdy) = (p.x - c0.x, p.y - c0.y);
            let a = cdx * cdx + cdy * cdy - dr * dr;
            let b = pdx * cdx + pdy * cdy + r0 * dr;
            let c = pdx * pdx + pdy * pdy - r0 * r0;
            let r = |t: f64| r0 + t * dr;
            let t = if a.abs() < 1e-9 {
                (b.abs() > 1e-12).then(|| c / (2.0 * b)).filter(|t| r(*t) >= 0.0)
            } else {
                let disc = b * b - a * c;
                if disc < 0.0 {
                    None
                } else {
                    let s = disc.sqrt();
                    let (t1, t2) = ((b + s) / a, (b - s) / a);
                    let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
                    [hi, lo].into_iter().find(|t| r(*t) >= 0.0)
                }
            }?;
            Some(sample(stops, t))
        }
        Leaf::Sweep { center, start, end, stops } => {
            let span = end - start;
            if span.abs() < 1e-9 {
                return stops.at.last().map(|(_, c)| *c);
            }
            let angle = (p.y - center.y).atan2(p.x - center.x).to_degrees().rem_euclid(360.0);
            Some(sample(stops, (angle - start) / span))
        }
        Leaf::Moved { to_leaf, leaf } => match leaf.as_ref() {
            Leaf::Moved { .. } => None,
            inner => eval(inner, *to_leaf * p),
        },
    }
}

/// Lays `color` (straight) at `coverage` over the premultiplied pixel at `at`.
fn blend(data: &mut [u8], at: usize, color: [f32; 4], coverage: f32) {
    let [r, g, b, a] = color;
    let alpha = (a * coverage).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let Some(px) = data.get_mut(at..at + 4) else { return };
    let keep = 1.0 - alpha;
    for (d, s) in px.iter_mut().zip([r * alpha, g * alpha, b * alpha, alpha]) {
        let v = s * 255.0 + f32::from(*d) * keep;
        *d = v.round().clamp(0.0, 255.0) as u8;
    }
}

/// The pixel box of `rect` under `t` within `width × height`; `None` when it is empty.
fn pixel_box(rect: fontcraft_geom::Rect, t: Affine, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let b = t.transform_rect_bbox(rect);
    if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
        return None;
    }
    let x0 = b.x0.floor().clamp(0.0, f64::from(width)) as u32;
    let y0 = b.y0.floor().clamp(0.0, f64::from(height)) as u32;
    let x1 = b.x1.ceil().clamp(0.0, f64::from(width)) as u32;
    let y1 = b.y1.ceil().clamp(0.0, f64::from(height)) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Paints `drawables` into `px`; `to_pixels` maps the glyph's font units to pixels. A glyph past
/// the work budget ([`MAX_PAINT_WORK`]) is painted as one fill of its outlines in `plain`.
pub fn paint_drawables(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, plain: [f32; 4]) {
    paint_drawables_within(px, drawables, to_pixels, MAX_PAINT_WORK, plain);
}

/// What painting `drawables` into a `width × height` image takes.
struct Work {
    /// Each drawable's pixel box, `None` when it paints nothing.
    boxes: Vec<Option<(u32, u32, u32, u32)>>,
    /// The box of them all.
    all: Option<(u32, u32, u32, u32)>,
    /// Pixel operations needed.
    cost: u64,
    /// Pixel operations allowed, with at least `least`.
    budget: u64,
}

fn work(drawables: &[Drawable], to_pixels: Affine, width: u32, height: u32, least: u64) -> Work {
    let boxes: Vec<Option<(u32, u32, u32, u32)>> = drawables
        .iter()
        .map(|d| d.clips.first().filter(|f| !f.elements().is_empty()).and_then(|f| pixel_box(f.bounding_box(), to_pixels, width, height)))
        .collect();
    let all = boxes.iter().flatten().copied().reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));
    let area = all.map_or(0, |(x0, y0, x1, y1)| u64::from(x1 - x0) * u64::from(y1 - y0));
    let budget = least.max(PAINT_WORK_PER_PIXEL.saturating_mul(area).min(MAX_GLYPH_PAINT_WORK));
    let cost = drawables.iter().zip(&boxes).fold(0u64, |sum, (d, b)| {
        let Some((x0, y0, x1, y1)) = *b else { return sum };
        let clips = u64::try_from(d.clips.len()).unwrap_or(u64::MAX);
        sum.saturating_add((u64::from(x1 - x0) * u64::from(y1 - y0)).saturating_mul(clips.saturating_add(1)))
    });
    Work { boxes, all, cost, budget }
}

/// [`paint_drawables`] with at least `least` pixel operations to spend. Answers whether the
/// drawables were painted in colour; when they would cost more than the budget, nothing of
/// them is painted and their outlines are filled in `plain` instead.
pub fn paint_drawables_within(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, least: u64, plain: [f32; 4]) -> bool {
    let inverse = to_pixels.inverse();
    if !inverse.as_coeffs().iter().all(|v| v.is_finite()) {
        return true;
    }
    let work = work(drawables, to_pixels, px.width, px.height, least);
    if work.cost > work.budget {
        paint_plain(px, drawables, to_pixels, &work, plain);
        return false;
    }
    for (d, b) in drawables.iter().zip(&work.boxes) {
        let Some((x0, y0, x1, y1)) = *b else { continue };
        let (w, h) = (x1 - x0, y1 - y0);
        let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
        let mut coverage = vec![1.0f32; w as usize * h as usize];
        let mut drawn = true;
        for clip in &d.clips {
            let Ok(mask) = render_path(clip, shift, w, h, &MASK) else {
                drawn = false;
                break;
            };
            for (c, p) in coverage.iter_mut().zip(mask.pixels.as_chunks::<4>().0) {
                *c *= f32::from(p[3]) / 255.0;
            }
        }
        if !drawn {
            continue;
        }
        for (i, cov) in coverage.iter().enumerate() {
            if *cov <= 0.0 {
                continue;
            }
            let (col, row) = (i as u32 % w, i as u32 / w);
            let (x, y) = (x0 + col, y0 + row);
            let Some(color) = eval(&d.leaf, inverse * Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5)) else { continue };
            let at = (y as usize * px.width as usize + x as usize) * 4;
            blend(px.data, at, color, *cov);
        }
    }
    true
}

/// A pixel box: left, top, right, bottom.
type PixelBox = (u32, u32, u32, u32);

/// Fills the outer outline of every drawable in `color` within the box `work.all`.
fn paint_plain(px: &mut Pixels, drawables: &[Drawable], to_pixels: Affine, work: &Work, color: [f32; 4]) {
    let Some(all) = work.all else { return };
    let coverage = match layer_coverage(drawables, &work.boxes, to_pixels, all, work.budget) {
        Some(c) => c,
        None => {
            let Some(c) = joined_coverage(drawables, to_pixels, all) else { return };
            c
        }
    };
    let (x0, y0, x1, _) = all;
    let w = x1.saturating_sub(x0).max(1);
    for (i, cov) in coverage.iter().enumerate() {
        if *cov <= 0.0 {
            continue;
        }
        let (x, y) = (x0 + i as u32 % w, y0 + i as u32 / w);
        blend(px.data, (y as usize * px.width as usize + x as usize) * 4, color, *cov);
    }
}

/// The coverage of the box `all`, row by row, by the outer outlines of `drawables`: each
/// outline is rendered on its own box (`boxes`) and a pixel takes the most any one gives it, so
/// an outline cannot cut a hole in another whatever their windings. `None` when that costs more
/// than `budget` pixel operations.
fn layer_coverage(drawables: &[Drawable], boxes: &[Option<PixelBox>], to_pixels: Affine, all: PixelBox, budget: u64) -> Option<Vec<f32>> {
    let area = |(x0, y0, x1, y1): PixelBox| u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));
    let cost = boxes.iter().flatten().fold(area(all), |sum, &b| sum.saturating_add(area(b)));
    if cost > budget {
        return None;
    }
    let (ax0, ay0, ax1, ay1) = all;
    let aw = ax1.saturating_sub(ax0);
    let mut coverage = vec![0.0f32; aw as usize * ay1.saturating_sub(ay0) as usize];
    for (d, b) in drawables.iter().zip(boxes) {
        let (Some(outline), Some((x0, y0, x1, y1))) = (d.clips.first(), *b) else { continue };
        let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if w == 0 || h == 0 {
            continue;
        }
        let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
        let Ok(mask) = render_path(outline, shift, w, h, &MASK) else { continue };
        for (i, p) in mask.pixels.as_chunks::<4>().0.iter().enumerate() {
            if p[3] == 0 {
                continue;
            }
            let col = x0.saturating_sub(ax0) + i as u32 % w;
            let row = y0.saturating_sub(ay0) + i as u32 / w;
            if let Some(c) = coverage.get_mut(row as usize * aw as usize + col as usize) {
                *c = c.max(f32::from(p[3]) / 255.0);
            }
        }
    }
    Some(coverage)
}

/// The coverage of the box `all`, row by row, by the outer outlines of `drawables` joined as one
/// path: one pass over the glyph's pixels whatever the number of drawables. Each outline is
/// turned to one winding first, so layers wound in opposite directions add up under the nonzero
/// rule; a layer that holds parts of both windings can still cancel another where they overlap.
fn joined_coverage(drawables: &[Drawable], to_pixels: Affine, all: PixelBox) -> Option<Vec<f32>> {
    let (x0, y0, x1, y1) = all;
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let outline =
        concat(drawables.iter().filter_map(|d| d.clips.first()).map(|c| if c.area() < 0.0 { c.reverse_subpaths() } else { c.clone() }).collect());
    let shift = Affine::translate((-f64::from(x0), -f64::from(y0))) * to_pixels;
    let mask = render_path(&outline, shift, w, h, &MASK).ok()?;
    Some(mask.pixels.as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect())
}
