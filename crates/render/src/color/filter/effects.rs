//! `feComponentTransfer`, `feConvolveMatrix`, `feMorphology`, `feDisplacementMap` and `feTile`
//! on premultiplied RGBA images of `w` × `h` pixels, rows top to bottom, by the Filter Effects
//! specification (M11.17e).
//!
//! - Transfer functions and the colour of a convolution with `preserveAlpha` work on straight
//!   values; the rest on premultiplied ones.
//! - Pixels outside an image are transparent, except where a convolution's edge mode says
//!   otherwise; an erosion near an edge therefore erodes towards it.
//! - Morphology takes the least or greatest of each channel over the window with two sliding
//!   passes, so its work does not grow with the radius.

use std::collections::VecDeque;

use fontcraft_color::svg_filter::{ColorChannel, EdgeMode, TransferFunction};
use fontcraft_geom::Affine;

use super::{get, put};

/// A colour channel's straight value from a premultiplied pixel, 0 to 255.
fn straight(c: u8, a: u8) -> usize {
    if a == 0 { 0 } else { ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as usize }
}

/// `px` through `functions` (R, G, B, A), each on straight values.
pub(super) fn transfer(px: &mut [u8], functions: &[TransferFunction; 4]) {
    // The values in are 8-bit, so each function becomes a table of 256.
    let tables: [Vec<f32>; 4] = functions.each_ref().map(|f| (0..256).map(|i| f.apply(f64::from(i) / 255.0) as f32).collect());
    let look = |k: usize, v: usize| tables.get(k).and_then(|t| t.get(v)).copied().unwrap_or(0.0);
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3];
        let s = [straight(p[0], a), straight(p[1], a), straight(p[2], a), usize::from(a)];
        let v = [0, 1, 2, 3].map(|k| look(k, s.get(k).copied().unwrap_or(0)));
        let out = [v[0] * v[3], v[1] * v[3], v[2] * v[3], v[3]];
        *p = out.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    }
}

/// An `feConvolveMatrix` ready to run.
pub(super) struct Convolution<'a> {
    pub columns: usize,
    pub rows: usize,
    pub kernel: &'a [f64],
    pub target: (usize, usize),
    pub divisor: f64,
    pub bias: f64,
    pub edge: EdgeMode,
    pub preserve_alpha: bool,
}

/// The index along one axis of `n` pixels that `i` reads under `edge`; `None` for transparent.
fn edge_index(i: i64, n: usize, edge: EdgeMode) -> Option<usize> {
    let n = n as i64;
    if n <= 0 {
        return None;
    }
    let i = match edge {
        EdgeMode::None => i,
        EdgeMode::Duplicate => i.clamp(0, n - 1),
        EdgeMode::Wrap => i.rem_euclid(n),
    };
    (0..n).contains(&i).then_some(i as usize)
}

/// `px` convolved by `c`.
pub(super) fn convolve(px: &[u8], w: usize, h: usize, c: &Convolution) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    let divisor = if c.divisor.is_finite() && c.divisor != 0.0 { c.divisor as f32 } else { 1.0 };
    let bias = if c.bias.is_finite() { c.bias as f32 } else { 0.0 };
    // What the kernel reads: straight colour when alpha is kept, else premultiplied RGBA.
    let values: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let v = get(px, i);
            if c.preserve_alpha && v[3] > 0.0 { [v[0] / v[3], v[1] / v[3], v[2] / v[3], v[3]] } else { v }
        })
        .collect();
    // The kernel's cells, flipped as the specification's sum reads them, with their offsets.
    let mut cells: Vec<(i64, i64, f32)> = Vec::with_capacity(c.columns * c.rows);
    for i in 0..c.rows {
        for j in 0..c.columns {
            let k = (c.rows - 1 - i) * c.columns + (c.columns - 1 - j);
            let weight = c.kernel.get(k).copied().unwrap_or(0.0) as f32;
            cells.push((j as i64 - c.target.0 as i64, i as i64 - c.target.1 as i64, weight));
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for &(dx, dy, weight) in &cells {
                let (Some(sx), Some(sy)) = (edge_index(x as i64 + dx, w, c.edge), edge_index(y as i64 + dy, h, c.edge)) else { continue };
                if let Some(v) = values.get(sy * w + sx) {
                    for (s, v) in sum.iter_mut().zip(v) {
                        *s += v * weight;
                    }
                }
            }
            let i = y * w + x;
            let v = sum.map(|s| s / divisor + bias);
            if c.preserve_alpha {
                let a = values.get(i).map_or(0.0, |v| v[3]);
                let [r, g, b] = [v[0], v[1], v[2]].map(|c| c.clamp(0.0, 1.0) * a);
                put(&mut out, i, [r, g, b, a]);
            } else {
                put(&mut out, i, v);
            }
        }
    }
    out
}

/// One line of a channel: the least (or greatest, when `dilate`) value within `r` of each
/// pixel, what lies past the ends counting as 0.
fn slide(line: &[u8], out: &mut [u8], r: usize, dilate: bool, window: &mut VecDeque<usize>) {
    let n = line.len();
    let at = |i: usize| line.get(i).copied().unwrap_or(0);
    let keeps = |new: u8, old: u8| if dilate { new >= old } else { new <= old };
    window.clear();
    let mut next = 0;
    for (x, o) in out.iter_mut().enumerate().take(n) {
        let last = x.saturating_add(r).min(n.saturating_sub(1));
        while next <= last && next < n {
            let v = at(next);
            while let Some(&back) = window.back()
                && keeps(v, at(back))
            {
                window.pop_back();
            }
            window.push_back(next);
            next += 1;
        }
        while let Some(&front) = window.front()
            && front.saturating_add(r) < x
        {
            window.pop_front();
        }
        let reaches_out = x < r || x.saturating_add(r) >= n;
        *o = if !dilate && reaches_out { 0 } else { window.front().map_or(0, |&i| at(i)) };
    }
}

/// `px` eroded, or dilated when `dilate`, over a window reaching `(rx, ry)` pixels.
pub(super) fn morphology(px: &mut [u8], w: usize, h: usize, (rx, ry): (usize, usize), dilate: bool) {
    let mut window = VecDeque::new();
    let (mut line, mut out) = (Vec::new(), Vec::new());
    for (r, n, lines, step, next) in [(rx, w, h, 4, w * 4), (ry, h, w, w * 4, 4)] {
        if r == 0 {
            continue;
        }
        for l in 0..lines {
            for ch in 0..4 {
                let at = |i: usize| l * next + i * step + ch;
                line.clear();
                line.extend((0..n).map(|i| px.get(at(i)).copied().unwrap_or(0)));
                out.clear();
                out.resize(n, 0);
                slide(&line, &mut out, r, dilate, &mut window);
                for (i, v) in out.iter().enumerate() {
                    if let Some(p) = px.get_mut(at(i)) {
                        *p = *v;
                    }
                }
            }
        }
    }
}

/// `px` with each pixel read from where the channels of `map` send it: `scale · (C − ½)` user
/// units, which `linear` (the filter's user space to pixels, without its translation) turns
/// into pixels.
pub(super) fn displace(px: &[u8], map: &[u8], w: usize, h: usize, scale: f64, channels: (ColorChannel, ColorChannel), linear: Affine) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    let pick = |p: &[u8; 4], c: ColorChannel| -> f64 {
        let v = match c {
            ColorChannel::R => straight(p[0], p[3]),
            ColorChannel::G => straight(p[1], p[3]),
            ColorChannel::B => straight(p[2], p[3]),
            ColorChannel::A => usize::from(p[3]),
        };
        v as f64 / 255.0
    };
    let [a, b, c, d, _, _] = linear.as_coeffs();
    let map = map.as_chunks::<4>().0;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let Some(m) = map.get(i) else { continue };
            let (ux, uy) = (scale * (pick(m, channels.0) - 0.5), scale * (pick(m, channels.1) - 0.5));
            let (dx, dy) = (a * ux + c * uy, b * ux + d * uy);
            if !(dx.is_finite() && dy.is_finite()) {
                continue;
            }
            let (sx, sy) = ((x as f64 + dx).round(), (y as f64 + dy).round());
            if sx < 0.0 || sy < 0.0 || sx >= w as f64 || sy >= h as f64 {
                continue;
            }
            let from = (sy as usize * w + sx as usize) * 4;
            if let (Some(dst), Some(src)) = (out.get_mut(i * 4..i * 4 + 4), px.get(from..from + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    out
}

/// `px` with the tile `[x0, y0, x1, y1)` (pixels of the image) repeated over the whole image.
pub(super) fn tile(px: &[u8], w: usize, h: usize, [x0, y0, x1, y1]: [usize; 4]) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    let (tw, th) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    if tw == 0 || th == 0 {
        return out;
    }
    let wrap = |v: usize, start: usize, size: usize| start + (v as i64 - start as i64).rem_euclid(size as i64) as usize;
    for y in 0..h {
        let sy = wrap(y, y0, th);
        for x in 0..w {
            let from = (sy * w + wrap(x, x0, tw)) * 4;
            let to = (y * w + x) * 4;
            if let (Some(dst), Some(src)) = (out.get_mut(to..to + 4), px.get(from..from + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn morphology_erodes_and_dilates_by_the_radius() {
        // A 3-pixel opaque bar in the middle of a 11-pixel line.
        let (w, h) = (11, 1);
        let mut px = vec![0u8; w * 4];
        for x in 4..7 {
            px[x * 4..x * 4 + 4].copy_from_slice(&[255; 4]);
        }
        let mut dilated = px.clone();
        morphology(&mut dilated, w, h, (2, 0), true);
        let alpha: Vec<u8> = dilated.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0]);
        let mut eroded = px.clone();
        morphology(&mut eroded, w, h, (1, 0), false);
        let alpha: Vec<u8> = eroded.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_convolution_with_an_off_centre_target_moves_the_image() {
        // A 3 × 1 kernel `1 0 0` with its target on the right cell reads two pixels left.
        let (w, h) = (5, 1);
        let mut px = vec![0u8; w * 4];
        px[4..8].copy_from_slice(&[255, 0, 0, 255]);
        let c = Convolution {
            columns: 3,
            rows: 1,
            kernel: &[0.0, 0.0, 1.0],
            target: (2, 0),
            divisor: 1.0,
            bias: 0.0,
            edge: EdgeMode::None,
            preserve_alpha: false,
        };
        let out = convolve(&px, w, h, &c);
        assert_eq!(&out[12..16], &[255, 0, 0, 255], "{out:?}");
        assert_eq!(out[7], 0);
        // From the last pixel, wrapping brings it round to the second; without, it leaves.
        let mut last = vec![0u8; w * 4];
        last[16..20].copy_from_slice(&[255, 0, 0, 255]);
        let out = convolve(&last, w, h, &c);
        assert!(out.iter().all(|v| *v == 0), "{out:?}");
        let c = Convolution { edge: EdgeMode::Wrap, ..c };
        let out = convolve(&last, w, h, &c);
        assert_eq!(&out[4..8], &[255, 0, 0, 255], "{out:?}");
    }

    #[test]
    fn tiles_repeat_from_their_corner() {
        let (w, h) = (4, 1);
        let px: Vec<u8> = [[0, 0, 0, 0], [10, 10, 10, 10], [20, 20, 20, 20], [0, 0, 0, 0]].concat();
        let out = tile(&px, w, h, [1, 0, 3, 1]);
        let alpha: Vec<u8> = out.chunks(4).map(|p| p[3]).collect();
        assert_eq!(alpha, [20, 10, 20, 10]);
    }
}
