//! `feTurbulence` (M11.17e): Perlin noise as the reference code of Filter Effects Module
//! Level 1 (§ feTurbulence, the same code as SVG 1.1 § 15.24) computes it, written again in
//! Rust: the same lattice from the same seed, the same gradients, stitching and octave sums, so a
//! document gives the noise it gives elsewhere.
//!
//! This software includes material derived from Filter Effects Module Level 1
//! (https://www.w3.org/TR/filter-effects-1/). Copyright © 2018 W3C® (MIT, ERCIM, Keio, Beihang).
//! Used under the W3C Software and Document License (`LICENSE-W3C.txt`, `NOTICE`).
//!
//! - Each pixel takes the noise at its top left corner's position in the filter's user space.
//! - With `stitchTiles`, the primitive's subregion is the tile: the frequency is adjusted to a
//!   whole number of cycles over it, and the lattice wraps at its right and bottom edges.
//! - Channel values are `(sum · 255 + 255) / 2` for `fractalNoise` and `sum · 255` for
//!   `turbulence`, clamped to 0 to 255, in the primitive's colour space, then premultiplied.
//! - Octaves are at most `fontcraft_color::svg_filter::MAX_TURBULENCE_OCTAVES`, so a pixel's
//!   work is bounded.

use fontcraft_geom::{Affine, Point, Rect};

use super::put;

const SIZE: usize = 0x100;
const MASK: i64 = 0xff;
const PERLIN_N: i64 = 0x1000;
const RAND_M: i64 = 2_147_483_647;
const RAND_A: i64 = 16_807;
const RAND_Q: i64 = 127_773;
const RAND_R: i64 = 2_836;
const LATTICE: usize = SIZE + SIZE + 2;

/// The next value of the specification's random number generator.
fn random(seed: i64) -> i64 {
    let r = RAND_A * (seed % RAND_Q) - RAND_R * (seed / RAND_Q);
    if r <= 0 { r + RAND_M } else { r }
}

/// The lattice selector and the gradients of the four channels.
struct Lattice {
    selector: [usize; LATTICE],
    gradient: [[[f64; 2]; LATTICE]; 4],
}

impl Lattice {
    fn new(seed: i32) -> Self {
        let mut seed = i64::from(seed);
        if seed <= 0 {
            seed = -(seed % (RAND_M - 1)) + 1;
        }
        seed = seed.min(RAND_M - 1);
        let mut selector = [0usize; LATTICE];
        let mut gradient = [[[0.0f64; 2]; LATTICE]; 4];
        for channel in &mut gradient {
            for (i, g) in channel.iter_mut().enumerate().take(SIZE) {
                if let Some(s) = selector.get_mut(i) {
                    *s = i;
                }
                for v in g.iter_mut() {
                    seed = random(seed);
                    *v = ((seed % (2 * SIZE as i64)) - SIZE as i64) as f64 / SIZE as f64;
                }
                let n = g[0].hypot(g[1]);
                if n > 0.0 {
                    *g = g.map(|v| v / n);
                }
            }
        }
        for i in (1..SIZE).rev() {
            seed = random(seed);
            selector.swap(i, (seed % SIZE as i64) as usize);
        }
        for i in 0..SIZE + 2 {
            if let Some(&s) = selector.get(i)
                && let Some(t) = selector.get_mut(SIZE + i)
            {
                *t = s;
            }
            for channel in &mut gradient {
                if let Some(&g) = channel.get(i)
                    && let Some(t) = channel.get_mut(SIZE + i)
                {
                    *t = g;
                }
            }
        }
        Lattice { selector, gradient }
    }

    fn select(&self, i: i64) -> usize {
        usize::try_from(i).ok().and_then(|i| self.selector.get(i)).copied().unwrap_or(0)
    }

    fn noise2(&self, channel: usize, [vx, vy]: [f64; 2], stitch: Option<&Stitch>) -> f64 {
        let split = |v: f64| {
            let t = v + PERLIN_N as f64;
            let whole = t as i64;
            (whole, t - whole as f64)
        };
        let (mut bx0, rx0) = split(vx);
        let (mut by0, ry0) = split(vy);
        let (mut bx1, mut by1) = (bx0.saturating_add(1), by0.saturating_add(1));
        let (rx1, ry1) = (rx0 - 1.0, ry0 - 1.0);
        if let Some(s) = stitch {
            if bx0 >= s.wrap_x {
                bx0 = bx0.saturating_sub(s.width);
            }
            if bx1 >= s.wrap_x {
                bx1 = bx1.saturating_sub(s.width);
            }
            if by0 >= s.wrap_y {
                by0 = by0.saturating_sub(s.height);
            }
            if by1 >= s.wrap_y {
                by1 = by1.saturating_sub(s.height);
            }
        }
        let (bx0, bx1, by0, by1) = (bx0 & MASK, bx1 & MASK, by0 & MASK, by1 & MASK);
        let (i, j) = (self.select(bx0) as i64, self.select(bx1) as i64);
        let (b00, b10, b01, b11) = (self.select(i + by0), self.select(j + by0), self.select(i + by1), self.select(j + by1));
        let curve = |t: f64| t * t * (3.0 - 2.0 * t);
        let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
        let (sx, sy) = (curve(rx0), curve(ry0));
        let g = |b: usize| self.gradient.get(channel).and_then(|c| c.get(b)).copied().unwrap_or([0.0; 2]);
        let at = |b: usize, rx: f64, ry: f64| {
            let q = g(b);
            rx * q[0] + ry * q[1]
        };
        let a = lerp(sx, at(b00, rx0, ry0), at(b10, rx1, ry0));
        let b = lerp(sx, at(b01, rx0, ry1), at(b11, rx1, ry1));
        lerp(sy, a, b)
    }
}

/// Where the lattice wraps when tiles are stitched.
#[derive(Clone, Copy)]
struct Stitch {
    width: i64,
    height: i64,
    wrap_x: i64,
    wrap_y: i64,
}

/// An `feTurbulence` ready to run.
pub(super) struct Turbulence {
    pub base_frequency: (f64, f64),
    pub octaves: u32,
    pub seed: i32,
    /// The tile stitched to, user space: the primitive's subregion.
    pub stitch: Option<Rect>,
    pub fractal: bool,
}

/// The base frequency adjusted to a whole number of cycles over the tile, and the stitching.
fn stitching(t: &Turbulence) -> ((f64, f64), Option<Stitch>) {
    let (mut fx, mut fy) = t.base_frequency;
    let Some(tile) = t.stitch else { return ((fx, fy), None) };
    let (tw, th) = (tile.width(), tile.height());
    let adjust = |f: f64, size: f64| {
        if f == 0.0 || size <= 0.0 {
            return f;
        }
        let (lo, hi) = ((size * f).floor() / size, (size * f).ceil() / size);
        if f / lo < hi / f { lo } else { hi }
    };
    fx = adjust(fx, tw);
    fy = adjust(fy, th);
    let whole = |v: f64| if v.is_finite() { v.clamp(-1e15, 1e15) as i64 } else { 0 };
    let width = whole(tw * fx + 0.5);
    let height = whole(th * fy + 0.5);
    let stitch = Stitch {
        width,
        height,
        wrap_x: whole(tile.x0 * fx + PERLIN_N as f64 + width as f64),
        wrap_y: whole(tile.y0 * fy + PERLIN_N as f64 + height as f64),
    };
    ((fx, fy), Some(stitch))
}

fn sum(lattice: &Lattice, channel: usize, point: Point, t: &Turbulence, (fx, fy): (f64, f64), stitch: Option<Stitch>) -> f64 {
    let mut stitch = stitch;
    let mut v = [point.x * fx, point.y * fy];
    let (mut total, mut ratio) = (0.0, 1.0);
    for _ in 0..t.octaves {
        let n = lattice.noise2(channel, v, stitch.as_ref());
        total += if t.fractal { n } else { n.abs() } / ratio;
        v = v.map(|c| c * 2.0);
        ratio *= 2.0;
        if let Some(s) = &mut stitch {
            s.width = s.width.saturating_mul(2);
            s.wrap_x = s.wrap_x.saturating_mul(2).saturating_sub(PERLIN_N);
            s.height = s.height.saturating_mul(2);
            s.wrap_y = s.wrap_y.saturating_mul(2).saturating_sub(PERLIN_N);
        }
    }
    total
}

/// The noise over an image of `w` × `h` pixels; `to_user` maps its pixels to the filter's user
/// space. Clear when that cannot be inverted.
pub(super) fn turbulence(w: usize, h: usize, to_user: Option<Affine>, t: &Turbulence) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let Some(to_user) = to_user else { return out };
    let lattice = Lattice::new(t.seed);
    let (frequency, stitch) = stitching(t);
    for y in 0..h {
        for x in 0..w {
            let p = to_user * Point::new(x as f64, y as f64);
            let v = [0, 1, 2, 3].map(|c| {
                let s = sum(&lattice, c, p, t, frequency, stitch);
                let v = if t.fractal { (s * 255.0 + 255.0) / 2.0 } else { s * 255.0 };
                (if v.is_finite() { v.clamp(0.0, 255.0) } else { 0.0 } / 255.0) as f32
            });
            put(&mut out, y * w + x, [v[0] * v[3], v[1] * v[3], v[2] * v[3], v[3]]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: i32, fractal: bool, octaves: u32) -> Vec<u8> {
        turbulence(32, 32, Some(Affine::IDENTITY), &Turbulence { base_frequency: (0.1, 0.1), octaves, seed, stitch: None, fractal })
    }

    #[test]
    fn noise_is_deterministic_varied_and_seeded() {
        let a = noise(1, true, 2);
        assert_eq!(a, noise(1, true, 2), "the same seed gives the same noise");
        assert_ne!(a, noise(2, true, 2), "another seed gives other noise");
        // Fractal noise sits around half intensity and varies; every channel is premultiplied.
        let alphas: Vec<u8> = a.chunks(4).map(|p| p[3]).collect();
        let mean = alphas.iter().map(|v| f64::from(*v)).sum::<f64>() / alphas.len() as f64;
        assert!((64.0..192.0).contains(&mean), "{mean}");
        assert!(alphas.iter().max() > alphas.iter().min());
        assert!(a.chunks(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
        // Noise at a lattice point is zero: turbulence there is clear.
        let t = noise(1, false, 1);
        assert_eq!(t[3], 0, "the origin is a lattice point");
        // No octaves, no noise.
        assert!(noise(1, false, 0).iter().all(|v| *v == 0));
    }

    #[test]
    fn stitched_noise_repeats_across_the_tile() {
        // A 20-unit tile at frequency 0.1 holds two cycles; past its right edge the lattice wraps
        // to its left edge, so the noise just past the tile is the noise at its start.
        let t = Turbulence { base_frequency: (0.1, 0.1), octaves: 1, seed: 3, stitch: Some(Rect::new(0.0, 0.0, 20.0, 20.0)), fractal: true };
        let out = turbulence(40, 1, Some(Affine::IDENTITY), &t);
        for x in 0..10 {
            assert_eq!(out[x * 4..x * 4 + 4], out[(x + 20) * 4..(x + 20) * 4 + 4], "x = {x}");
        }
    }
}
