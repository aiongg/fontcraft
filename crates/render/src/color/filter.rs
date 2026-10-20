//! SVG filters run on a glyph's sprite (M11.17d): the filters `fontcraft_color::svg_filter`
//! reads from an `SVG ` document, applied to the layer a filtered group is drawn on.
//!
//! Contract:
//! - A filter works on the pixels of its region's bounding box on the sprite, within the box its
//!   group can paint ([`reach`]), premultiplied RGBA with 8 bits a channel; the layer is clear
//!   outside that box afterwards. A box of more than [`MAX_FILTER_PIXELS`] pixels is not run,
//!   and the group is drawn unfiltered with a warning.
//! - Primitives whose result nothing reads, other than the last, are not run.
//! - Each primitive converts its inputs to its colour space (linear RGB unless
//!   `color-interpolation-filters` says sRGB) and its result is clear outside its subregion.
//! - Lengths scale from user space to pixels by the lengths of the transform's axes, as resvg
//!   scales them, so a rotated element's blur is not rotated. Offsets are rounded to whole pixels.
//! - `feGaussianBlur` follows the Filter Effects specification: a Gaussian kernel below a
//!   standard deviation of 2 pixels, three box blurs from 2 on, which costs the same whatever
//!   the deviation. Deviations past [`MAX_BLUR_SIGMA`] pixels blur at that deviation.
//! - Work is the region's pixels times a weight per primitive ([`Plan::cost`]), counted against
//!   the glyph's budget before the filter runs.

use std::borrow::Cow;

use fontcraft_color::svg_filter::{BlendMode, CompositeOperator, FilterInput, FilterKind, SvgFilter};
use fontcraft_geom::{Affine, Point};
use skrifa::color::CompositeMode;

use super::{MAX_BLUR_SIGMA, MAX_FILTER_PIXELS, PixBox, composite_px};

/// A filter ready to run on a sprite.
pub(super) struct Plan {
    /// The region's pixels on the sprite.
    region: PixBox,
    /// From the filter's user space to sprite pixels.
    to_pixels: Affine,
    /// Pixel operations the filter takes.
    pub(super) cost: u64,
}

/// The plan for `filter` on a `width` × `height` sprite whose pixels `to_pixels` maps the
/// filter's user space to, within `limit`; `Err` with a warning when it is not run.
pub(super) fn plan(filter: &SvgFilter, to_pixels: Affine, width: u32, height: u32, limit: PixBox) -> Result<Plan, String> {
    let region = PixBox::of(filter.region, to_pixels, width, height).intersect(limit);
    if region.area() > MAX_FILTER_PIXELS {
        return Err(format!("an SVG filter region of {} pixels is drawn unfiltered; at most {MAX_FILTER_PIXELS} are filtered", region.area()));
    }
    let weight: u64 = filter
        .primitives
        .iter()
        .map(|p| match &p.kind {
            FilterKind::Blur { .. } | FilterKind::DropShadow { .. } => 16,
            FilterKind::Merge { inputs } => 4 + 2 * inputs.len() as u64,
            _ => 6,
        })
        .sum();
    let sprite = u64::from(width) * u64::from(height);
    let cost = region.area().saturating_mul(weight.saturating_add(2)).saturating_add(sprite);
    Ok(Plan { region, to_pixels, cost })
}

/// How far `filter` can move or spread what it reads, font units along x and along y, at
/// `scale` sprite pixels per font unit: the sum over its primitives, as primitives can read each
/// other's results, with two pixels for each to cover rounding. `None` when a primitive can paint
/// where everything it reads is clear: a flood, a colour matrix adding alpha, or an arithmetic
/// composite with a positive `k4`.
pub(super) fn reach(filter: &SvgFilter, scale: f64) -> Option<(f64, f64)> {
    let pixel = if scale.is_finite() && scale > 0.0 { 1.0 / scale } else { return None };
    let [a, b, c, d, _, _] = filter.to_font.as_coeffs();
    let (axis_x, axis_y) = (a.hypot(b), c.hypot(d));
    let blur = |(sx, sy): (f64, f64)| {
        // Deviations are clamped in sprite pixels when the blur runs.
        let most = MAX_BLUR_SIGMA * pixel;
        (3.0 * (sx.abs() * axis_x).min(most), 3.0 * (sy.abs() * axis_y).min(most))
    };
    let shift = |dx: f64, dy: f64| {
        let v = filter.to_font * Point::new(dx, dy) - filter.to_font * Point::ORIGIN;
        (v.x.abs(), v.y.abs())
    };
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for p in &filter.primitives {
        let (gx, gy) = match &p.kind {
            FilterKind::Flood { .. } => return None,
            FilterKind::ColorMatrix { matrix, .. } if matrix.get(19).is_none_or(|v| v.is_nan() || *v > 0.0) => return None,
            FilterKind::Composite { operator: CompositeOperator::Arithmetic([_, _, _, k4]), .. } if k4.is_nan() || *k4 > 0.0 => return None,
            FilterKind::Blur { std_dev, .. } => blur(*std_dev),
            FilterKind::Offset { dx, dy, .. } => shift(*dx, *dy),
            FilterKind::DropShadow { dx, dy, std_dev, .. } => {
                let ((bx, by), (ox, oy)) = (blur(*std_dev), shift(*dx, *dy));
                (bx + ox, by + oy)
            }
            _ => (0.0, 0.0),
        };
        x += gx + 2.0 * pixel;
        y += gy + 2.0 * pixel;
    }
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Premultiplied RGBA over the filter region, rows top to bottom.
#[derive(Clone)]
struct Img {
    px: Vec<u8>,
    /// The colour channels hold linear RGB values; else sRGB.
    linear: bool,
}

/// Lookup tables between sRGB and linear RGB channel values.
struct Spaces {
    to_linear: [u8; 256],
    to_srgb: [u8; 256],
}

impl Spaces {
    fn new() -> Self {
        let mut to_linear = [0u8; 256];
        let mut to_srgb = [0u8; 256];
        for (i, (l, s)) in to_linear.iter_mut().zip(to_srgb.iter_mut()).enumerate() {
            let v = i as f64 / 255.0;
            let lin = if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            let srgb = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            *l = (lin * 255.0).round().clamp(0.0, 255.0) as u8;
            *s = (srgb * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        Spaces { to_linear, to_srgb }
    }

    /// An sRGB channel value from 0 to 1, as a linear one when `linear`.
    fn channel(&self, v: f64, linear: bool) -> f64 {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        if !linear {
            return v;
        }
        f64::from(self.to_linear.get((v * 255.0).round() as usize).copied().unwrap_or(0)) / 255.0
    }

    /// `img` with its colour channels in linear RGB when `linear`, else in sRGB.
    fn convert(&self, img: &mut Img, linear: bool) {
        if img.linear == linear {
            return;
        }
        img.linear = linear;
        let table = if linear { &self.to_linear } else { &self.to_srgb };
        for px in img.px.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            if a == 0 {
                continue;
            }
            for c in px.iter_mut().take(3) {
                let straight = ((u32::from(*c) * 255 + a / 2) / a).min(255) as usize;
                let mapped = u32::from(table.get(straight).copied().unwrap_or(0));
                *c = ((mapped * a + 127) / 255).min(a) as u8;
            }
        }
    }
}

fn get(px: &[u8], i: usize) -> [f32; 4] {
    match px.get(i * 4..i * 4 + 4) {
        Some(&[r, g, b, a]) => [r, g, b, a].map(|v| f32::from(v) / 255.0),
        _ => [0.0; 4],
    }
}

fn put(px: &mut [u8], i: usize, v: [f32; 4]) {
    let a = if v[3].is_finite() { v[3].clamp(0.0, 1.0) } else { 0.0 };
    if let Some(out) = px.get_mut(i * 4..i * 4 + 4) {
        for (o, c) in out.iter_mut().zip([v[0], v[1], v[2], a]) {
            // Premultiplied colour never exceeds its alpha.
            let c = if c.is_finite() { c.clamp(0.0, a) } else { 0.0 };
            *o = (c * 255.0).round() as u8;
        }
    }
}

/// Runs `filter` as `plan` prepared it on `layer`, the sprite's top layer (`width` pixels a row).
pub(super) fn run(filter: &SvgFilter, plan: &Plan, layer: &mut [u8], width: u32) {
    let r = plan.region;
    let (w, h) = (r.width() as usize, r.y1.saturating_sub(r.y0) as usize);
    let stride = width as usize * 4;
    let mut source = vec![0u8; w * h * 4];
    for y in 0..h {
        let from = (r.y0 as usize + y) * stride + r.x0 as usize * 4;
        if let (Some(dst), Some(src)) = (source.get_mut(y * w * 4..(y + 1) * w * 4), layer.get(from..from + w * 4)) {
            dst.copy_from_slice(src);
        }
    }
    let mut run = Run { w, h, spaces: Spaces::new(), source: Img { px: source, linear: false }, results: Vec::new(), to_pixels: plan.to_pixels };
    let last_use = last_uses(filter);
    for (i, p) in filter.primitives.iter().enumerate() {
        // A result nothing reads is not made.
        let out = (last_use.get(i).copied().flatten() != Some(i)).then(|| {
            let sub = PixBox::of(p.subregion, plan.to_pixels, u32::MAX, u32::MAX).intersect(r);
            let mut out = run.primitive(&p.kind, p.linear_rgb);
            clear_outside(&mut out.px, w, sub, r);
            out
        });
        run.results.push(out);
        // Results nothing later reads are dropped, so memory follows what is still needed.
        for (k, slot) in run.results.iter_mut().enumerate() {
            if last_use.get(k).copied().flatten() == Some(i) {
                *slot = None;
            }
        }
    }
    let mut out = match run.results.pop().flatten() {
        Some(img) => img,
        None => run.source.clone(),
    };
    run.spaces.convert(&mut out, false);
    layer.fill(0);
    for y in 0..h {
        let to = (r.y0 as usize + y) * stride + r.x0 as usize * 4;
        if let (Some(dst), Some(src)) = (layer.get_mut(to..to + w * 4), out.px.get(y * w * 4..(y + 1) * w * 4)) {
            dst.copy_from_slice(src);
        }
    }
}

/// For each primitive, the last primitive that reads its result, or its own index when none
/// does, so the result can be dropped (or never made) once that one has run; `None` for the last
/// primitive, whose result is the filter's.
fn last_uses(filter: &SvgFilter) -> Vec<Option<usize>> {
    let mut out: Vec<Option<usize>> = (0..filter.primitives.len()).map(Some).collect();
    for (i, p) in filter.primitives.iter().enumerate() {
        for input in inputs(&p.kind) {
            if let FilterInput::Result(k) = input
                && let Some(slot) = out.get_mut(k)
            {
                *slot = (*slot).max(Some(i));
            }
        }
    }
    if let Some(last) = out.last_mut() {
        *last = None;
    }
    out
}

fn inputs(kind: &FilterKind) -> Vec<FilterInput> {
    match kind {
        FilterKind::Blur { input, .. }
        | FilterKind::Offset { input, .. }
        | FilterKind::ColorMatrix { input, .. }
        | FilterKind::DropShadow { input, .. } => vec![*input],
        FilterKind::Composite { input, input2, .. } | FilterKind::Blend { input, input2, .. } => vec![*input, *input2],
        FilterKind::Merge { inputs } => inputs.clone(),
        FilterKind::Flood { .. } => Vec::new(),
    }
}

/// Clears what lies outside `sub` (sprite pixels) in `px`, an image over `region`.
fn clear_outside(px: &mut [u8], w: usize, sub: PixBox, region: PixBox) {
    let (x0, x1) = (sub.x0.saturating_sub(region.x0) as usize, sub.x1.saturating_sub(region.x0) as usize);
    let (y0, y1) = (sub.y0.saturating_sub(region.y0) as usize, sub.y1.saturating_sub(region.y0) as usize);
    for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = (i % w.max(1), i / w.max(1));
        if sub.is_empty() || x < x0 || x >= x1 || y < y0 || y >= y1 {
            *p = [0; 4];
        }
    }
}

/// One filter being run over its region.
struct Run {
    w: usize,
    h: usize,
    spaces: Spaces,
    source: Img,
    /// Each primitive's result so far; `None` once nothing later reads it.
    results: Vec<Option<Img>>,
    to_pixels: Affine,
}

impl Run {
    fn clear(&self) -> Img {
        Img { px: vec![0; self.w * self.h * 4], linear: false }
    }

    /// The image `input` names, in linear RGB when `linear`.
    fn input(&self, input: FilterInput, linear: bool) -> Img {
        let mut img = match input {
            FilterInput::SourceGraphic => Cow::Borrowed(&self.source),
            FilterInput::SourceAlpha => {
                let mut px = self.source.px.clone();
                for p in px.as_chunks_mut::<4>().0 {
                    *p = [0, 0, 0, p[3]];
                }
                Cow::Owned(Img { px, linear })
            }
            FilterInput::Result(k) => match self.results.get(k).and_then(Option::as_ref) {
                Some(img) => Cow::Borrowed(img),
                None => Cow::Owned(self.clear()),
            },
        }
        .into_owned();
        self.spaces.convert(&mut img, linear);
        img
    }

    /// A user-space length along x and along y in pixels.
    fn scale(&self, (sx, sy): (f64, f64)) -> (f64, f64) {
        let [a, b, c, d, _, _] = self.to_pixels.as_coeffs();
        (sx * a.hypot(b), sy * c.hypot(d))
    }

    /// A user-space offset in whole pixels.
    fn offset(&self, dx: f64, dy: f64) -> (i64, i64) {
        let v = self.to_pixels * Point::new(dx, dy) - self.to_pixels * Point::ORIGIN;
        let round = |v: f64| if v.is_finite() { v.round().clamp(-1e6, 1e6) as i64 } else { 0 };
        (round(v.x), round(v.y))
    }

    fn primitive(&self, kind: &FilterKind, linear: bool) -> Img {
        match kind {
            FilterKind::Blur { input, std_dev } => {
                let mut img = self.input(*input, linear);
                blur(&mut img.px, self.w, self.h, self.scale(*std_dev));
                img
            }
            FilterKind::Offset { input, dx, dy } => {
                let img = self.input(*input, linear);
                let (dx, dy) = self.offset(*dx, *dy);
                Img { px: shifted(&img.px, self.w, self.h, dx, dy), linear }
            }
            FilterKind::Flood { color } => {
                let mut img = self.clear();
                let [r, g, b, a] = color.map(|v| v as f32);
                let c = [r * a, g * a, b * a, a];
                for i in 0..self.w * self.h {
                    put(&mut img.px, i, c);
                }
                img
            }
            FilterKind::ColorMatrix { input, matrix } => {
                let mut img = self.input(*input, linear);
                let m = matrix.map(|v| v as f32);
                for i in 0..self.w * self.h {
                    let [r, g, b, a] = get(&img.px, i);
                    let s = if a > 0.0 { [r / a, g / a, b / a, a] } else { [0.0; 4] };
                    let row = |k: usize| {
                        let m = |j: usize| m.get(k * 5 + j).copied().unwrap_or(0.0);
                        (m(0) * s[0] + m(1) * s[1] + m(2) * s[2] + m(3) * s[3] + m(4)).clamp(0.0, 1.0)
                    };
                    let a = row(3);
                    put(&mut img.px, i, [row(0) * a, row(1) * a, row(2) * a, a]);
                }
                img
            }
            FilterKind::Composite { input, input2, operator } => {
                let (top, mut below) = (self.input(*input, linear), self.input(*input2, linear));
                for i in 0..self.w * self.h {
                    let (s, d) = (get(&top.px, i), get(&below.px, i));
                    let v = match operator {
                        CompositeOperator::Over => composite_px(CompositeMode::SrcOver, s, d),
                        CompositeOperator::In => composite_px(CompositeMode::SrcIn, s, d),
                        CompositeOperator::Out => composite_px(CompositeMode::SrcOut, s, d),
                        CompositeOperator::Atop => composite_px(CompositeMode::SrcAtop, s, d),
                        CompositeOperator::Xor => composite_px(CompositeMode::Xor, s, d),
                        CompositeOperator::Arithmetic(k) => {
                            let [k1, k2, k3, k4] = k.map(|v| v as f32);
                            [0, 1, 2, 3].map(|c| {
                                let (a, b) = (s.get(c).copied().unwrap_or(0.0), d.get(c).copied().unwrap_or(0.0));
                                k1 * a * b + k2 * a + k3 * b + k4
                            })
                        }
                    };
                    put(&mut below.px, i, v);
                }
                below
            }
            FilterKind::Merge { inputs } => {
                let mut out = self.clear();
                out.linear = linear;
                for input in inputs {
                    let img = self.input(*input, linear);
                    over(&img.px, &mut out.px);
                }
                out
            }
            FilterKind::Blend { input, input2, mode } => {
                let (top, mut below) = (self.input(*input, linear), self.input(*input2, linear));
                let mode = blend_mode(*mode);
                for i in 0..self.w * self.h {
                    let v = composite_px(mode, get(&top.px, i), get(&below.px, i));
                    put(&mut below.px, i, v);
                }
                below
            }
            FilterKind::DropShadow { input, dx, dy, std_dev, color } => {
                let img = self.input(*input, linear);
                let mut shadow = img.px.clone();
                let [r, g, b, alpha] = *color;
                let [r, g, b] = [r, g, b].map(|c| self.spaces.channel(c, linear) as f32);
                let alpha = alpha as f32;
                for p in shadow.as_chunks_mut::<4>().0 {
                    let a = f32::from(p[3]) / 255.0 * alpha;
                    *p = [r * a, g * a, b * a, a].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
                }
                blur(&mut shadow, self.w, self.h, self.scale(*std_dev));
                let (dx, dy) = self.offset(*dx, *dy);
                let mut out = shifted(&shadow, self.w, self.h, dx, dy);
                over(&img.px, &mut out);
                Img { px: out, linear }
            }
        }
    }
}

fn blend_mode(mode: BlendMode) -> CompositeMode {
    match mode {
        BlendMode::Normal => CompositeMode::SrcOver,
        BlendMode::Multiply => CompositeMode::Multiply,
        BlendMode::Screen => CompositeMode::Screen,
        BlendMode::Overlay => CompositeMode::Overlay,
        BlendMode::Darken => CompositeMode::Darken,
        BlendMode::Lighten => CompositeMode::Lighten,
        BlendMode::ColorDodge => CompositeMode::ColorDodge,
        BlendMode::ColorBurn => CompositeMode::ColorBurn,
        BlendMode::HardLight => CompositeMode::HardLight,
        BlendMode::SoftLight => CompositeMode::SoftLight,
        BlendMode::Difference => CompositeMode::Difference,
        BlendMode::Exclusion => CompositeMode::Exclusion,
        BlendMode::Hue => CompositeMode::HslHue,
        BlendMode::Saturation => CompositeMode::HslSaturation,
        BlendMode::Color => CompositeMode::HslColor,
        BlendMode::Luminosity => CompositeMode::HslLuminosity,
    }
}

/// Lays `top` over `below`, both premultiplied.
fn over(top: &[u8], below: &mut [u8]) {
    for (s, d) in top.as_chunks::<4>().0.iter().zip(below.as_chunks_mut::<4>().0) {
        let keep = 255 - u32::from(s[3]);
        for (d, s) in d.iter_mut().zip(s) {
            *d = (u32::from(*s) + (u32::from(*d) * keep + 127) / 255).min(255) as u8;
        }
    }
}

/// `px` (`w` × `h`) moved `dx` right and `dy` down; what comes in from outside is clear.
fn shifted(px: &[u8], w: usize, h: usize, dx: i64, dy: i64) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    let (wi, hi) = (w as i64, h as i64);
    if dx.abs() >= wi || dy.abs() >= hi {
        return out;
    }
    for y in 0..hi {
        let from_y = y - dy;
        if from_y < 0 || from_y >= hi {
            continue;
        }
        let (x0, x1) = (dx.max(0), (wi + dx).min(wi));
        let n = (x1 - x0).max(0) as usize * 4;
        let to = (y * wi + x0) as usize * 4;
        let from = (from_y * wi + x0 - dx) as usize * 4;
        if let (Some(dst), Some(src)) = (out.get_mut(to..to + n), px.get(from..from + n)) {
            dst.copy_from_slice(src);
        }
    }
    out
}

/// How one axis is blurred.
enum Axis {
    None,
    /// Gaussian weights, the middle one at the pixel.
    Kernel(Vec<f32>),
    /// Three box blurs, each reaching this many pixels before and after the pixel.
    Boxes([(usize, usize); 3]),
}

fn axis(sigma: f64) -> Axis {
    let sigma = if sigma.is_finite() { sigma.min(MAX_BLUR_SIGMA) } else { 0.0 };
    if sigma <= 0.0 {
        return Axis::None;
    }
    if sigma < 2.0 {
        let r = (sigma * 3.0).ceil() as i32;
        let weights: Vec<f64> = (-r..=r).map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp()).collect();
        let sum: f64 = weights.iter().sum();
        return Axis::Kernel(weights.iter().map(|w| (w / sum.max(f64::MIN_POSITIVE)) as f32).collect());
    }
    // The box size that three box blurs give a Gaussian of `sigma` with, by the specification.
    let d = (sigma * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5).floor().max(1.0) as usize;
    let h = d / 2;
    if d % 2 == 1 { Axis::Boxes([(h, h); 3]) } else { Axis::Boxes([(h, h.saturating_sub(1)), (h.saturating_sub(1), h), (h, h)]) }
}

/// Blurs `px` (`w` × `h`, premultiplied) by the standard deviations `(sx, sy)` in pixels; what
/// lies outside is clear.
fn blur(px: &mut [u8], w: usize, h: usize, (sx, sy): (f64, f64)) {
    let mut line: Vec<[f32; 4]> = Vec::new();
    let mut scratch: Vec<[f32; 4]> = Vec::new();
    let mut sums: Vec<[f64; 4]> = Vec::new();
    for (along, n, lines, step, next) in [(axis(sx), w, h, 4, w * 4), (axis(sy), h, w, w * 4, 4)] {
        if matches!(along, Axis::None) {
            continue;
        }
        for l in 0..lines {
            let at = |i: usize| l * next + i * step;
            line.clear();
            line.extend((0..n).map(|i| match px.get(at(i)..at(i) + 4) {
                Some(&[r, g, b, a]) => [r, g, b, a].map(f32::from),
                _ => [0.0; 4],
            }));
            match &along {
                Axis::None => {}
                Axis::Kernel(weights) => {
                    let r = weights.len() / 2;
                    scratch.clear();
                    scratch.extend((0..n).map(|i| {
                        let mut acc = [0.0f32; 4];
                        for (k, wt) in weights.iter().enumerate() {
                            let Some(j) = (i + k).checked_sub(r) else { continue };
                            if let Some(v) = line.get(j) {
                                for (a, v) in acc.iter_mut().zip(v) {
                                    *a += v * wt;
                                }
                            }
                        }
                        acc
                    }));
                    std::mem::swap(&mut line, &mut scratch);
                }
                Axis::Boxes(boxes) => {
                    for &(before, after) in boxes {
                        sums.clear();
                        sums.push([0.0; 4]);
                        let mut acc = [0.0f64; 4];
                        for v in &line {
                            for (a, v) in acc.iter_mut().zip(v) {
                                *a += f64::from(*v);
                            }
                            sums.push(acc);
                        }
                        let size = (before + after + 1) as f64;
                        for (i, v) in line.iter_mut().enumerate() {
                            let hi = sums.get((i + after + 1).min(n)).copied().unwrap_or([0.0; 4]);
                            let lo = sums.get(i.saturating_sub(before)).copied().unwrap_or([0.0; 4]);
                            for ((o, h), l) in v.iter_mut().zip(hi).zip(lo) {
                                *o = ((h - l) / size) as f32;
                            }
                        }
                    }
                }
            }
            for (i, v) in line.iter().enumerate() {
                if let Some(out) = px.get_mut(at(i)..at(i) + 4) {
                    for (o, v) in out.iter_mut().zip(v) {
                        *o = v.round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_color::svg_filter::FilterPrimitive;
    use fontcraft_geom::Rect;

    use super::*;

    #[test]
    fn a_blur_keeps_the_ink_and_spreads_it_evenly() {
        // A 1-pixel dot in the middle of 41 × 41, blurred by 3 pixels (the box blurs).
        let (w, h) = (41, 41);
        let mut px = vec![0u8; w * h * 4];
        let mid = (20 * w + 20) * 4;
        px[mid..mid + 4].copy_from_slice(&[255; 4]);
        let mut big = px.clone();
        blur(&mut px, w, h, (3.0, 3.0));
        let alpha = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
        assert_eq!(alpha(17, 20), alpha(23, 20), "symmetric left and right");
        assert_eq!(alpha(20, 17), alpha(20, 23), "symmetric up and down");
        assert!(alpha(20, 20) < 255 && alpha(20, 20) > 0);
        assert_eq!(alpha(0, 0), 0, "nothing reaches 20 pixels away");
        // A small deviation takes the Gaussian kernel and stays symmetric too.
        blur(&mut big, w, h, (1.0, 0.0));
        let a = |x: usize| big[(20 * w + x) * 4 + 3];
        assert!(a(19) == a(21) && a(19) > 0 && a(20) > a(19), "{} {} {}", a(19), a(20), a(21));
        assert_eq!(big[(19 * w + 20) * 4 + 3], 0, "no blur along y");
    }

    #[test]
    fn results_nothing_reads_are_not_kept() {
        let square = Rect::new(0.0, 0.0, 10.0, 10.0);
        let p = |kind| FilterPrimitive { subregion: square, linear_rgb: false, kind };
        let flood = || FilterKind::Flood { color: [1.0, 0.0, 0.0, 1.0] };
        let mut f = SvgFilter { to_font: Affine::IDENTITY, region: square, primitives: vec![p(flood()), p(flood()), p(flood())] };
        assert_eq!(last_uses(&f), vec![Some(0), Some(1), None], "three floods keep only the last result");
        // An offset that a merge two primitives on reads is kept until then.
        let offset = FilterKind::Offset { input: FilterInput::SourceGraphic, dx: 1.0, dy: 1.0 };
        let merge = FilterKind::Merge { inputs: vec![FilterInput::Result(0), FilterInput::Result(1)] };
        f.primitives = vec![p(offset), p(flood()), p(merge)];
        assert_eq!(last_uses(&f), vec![Some(2), Some(2), None]);
    }

    #[test]
    fn colour_spaces_convert_both_ways() {
        let s = Spaces::new();
        let mut img = Img { px: vec![128, 64, 32, 255, 0, 0, 0, 0], linear: false };
        s.convert(&mut img, true);
        assert!(img.px[0] < 128 && img.px[3] == 255, "mid grey is darker in linear values: {:?}", img.px);
        s.convert(&mut img, false);
        assert!((i16::from(img.px[0]) - 128).abs() <= 2, "{:?}", img.px);
    }
}
