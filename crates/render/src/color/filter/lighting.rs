//! `feDiffuseLighting` and `feSpecularLighting` (M11.17e): the input's alpha as a height map,
//! lit by a distant, point or spot light, by the Filter Effects specification.
//!
//! - The surface normal comes from Sobel kernels over the alpha times `surfaceScale`; along the
//!   image's edges the specification's edge kernels apply: a one-sided difference counts double,
//!   and the smoothing weights `1 2 1` are divided by the weights of the rows that exist.
//! - Positions are pixel centres in image pixels. A light's position and its `z` go through the
//!   filter's user space to pixels: x and y by the transform, z by the square root of its
//!   determinant, as is a distant light's direction.
//! - The specular exponent is clamped to 1 to 128, the range SVG 1.1 gives it; the spot light's
//!   cone has a hard edge.

use fontcraft_color::svg_filter::LightSource;
use fontcraft_geom::{Affine, Point};

use super::put;

/// How the light is reflected.
pub(super) enum Shading {
    Diffuse { constant: f64 },
    Specular { constant: f64, exponent: f64 },
}

/// A light in image pixels.
pub(super) enum Placed {
    /// The unit vector towards the light.
    Distant([f64; 3]),
    Point([f64; 3]),
    Spot {
        position: [f64; 3],
        /// The unit vector from the light to where it points.
        axis: [f64; 3],
        exponent: f64,
        cos_cone: Option<f64>,
    },
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n.is_finite() && n > 0.0 { v.map(|c| c / n) } else { [0.0, 0.0, 0.0] }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `light` placed by `to_image`, the filter's user space to image pixels.
pub(super) fn place(light: &LightSource, to_image: Affine) -> Placed {
    let [a, b, c, d, _, _] = to_image.as_coeffs();
    let z_scale = (a * d - b * c).abs().sqrt();
    let point = |[x, y, z]: [f64; 3]| {
        let p = to_image * Point::new(x, y);
        [p.x, p.y, z * z_scale]
    };
    match *light {
        LightSource::Distant { azimuth, elevation } => {
            let (az, el) = (azimuth.to_radians(), elevation.to_radians());
            let (ux, uy) = (az.cos() * el.cos(), az.sin() * el.cos());
            Placed::Distant(normalize([a * ux + c * uy, b * ux + d * uy, el.sin() * z_scale]))
        }
        LightSource::Point { position } => Placed::Point(point(position)),
        LightSource::Spot { position, points_at, exponent, cone } => {
            let (p, at) = (point(position), point(points_at));
            Placed::Spot {
                position: p,
                axis: normalize([at[0] - p[0], at[1] - p[1], at[2] - p[2]]),
                exponent,
                cos_cone: cone.map(|c| c.abs().min(180.0).to_radians().cos()),
            }
        }
    }
}

/// The image `px` (`w` × `h`) lit: `color` is the light's colour in the primitive's colour space.
pub(super) fn light(px: &[u8], w: usize, h: usize, surface_scale: f64, color: [f64; 3], light: &Placed, shading: &Shading) -> Vec<u8> {
    let mut out = vec![0u8; px.len()];
    let alpha = |x: usize, y: usize| f64::from(px.get((y * w + x) * 4 + 3).copied().unwrap_or(0)) / 255.0;
    let ss = if surface_scale.is_finite() { surface_scale } else { 0.0 };
    for y in 0..h {
        for x in 0..w {
            // Rows (or columns) that exist around the pixel, with their smoothing weights.
            let rows = [(y.checked_sub(1), 1.0), (Some(y), 2.0), (Some(y + 1).filter(|v| *v < h), 1.0)];
            let columns = [(x.checked_sub(1), 1.0), (Some(x), 2.0), (Some(x + 1).filter(|v| *v < w), 1.0)];
            let (left, right) = (x.saturating_sub(1), if x + 1 < w { x + 1 } else { x });
            let (up, down) = (y.saturating_sub(1), if y + 1 < h { y + 1 } else { y });
            let gx = if left == right {
                0.0
            } else {
                let rows = rows.iter().filter_map(|&(r, k)| Some((r?, k)));
                let weights: f64 = rows.clone().map(|r| r.1).sum();
                let twice = if right - left == 2 { 1.0 } else { 2.0 };
                twice / weights * rows.map(|(r, k)| k * (alpha(right, r) - alpha(left, r))).sum::<f64>()
            };
            let gy = if up == down {
                0.0
            } else {
                let columns = columns.iter().filter_map(|&(c, k)| Some((c?, k)));
                let weights: f64 = columns.clone().map(|c| c.1).sum();
                let twice = if down - up == 2 { 1.0 } else { 2.0 };
                twice / weights * columns.map(|(c, k)| k * (alpha(c, down) - alpha(c, up))).sum::<f64>()
            };
            let normal = normalize([-ss * gx, -ss * gy, 1.0]);
            let z = ss * alpha(x, y);
            let here = [x as f64 + 0.5, y as f64 + 0.5, z];
            let (to_light, strength) = match light {
                Placed::Distant(l) => (*l, 1.0),
                Placed::Point(p) => (normalize([p[0] - here[0], p[1] - here[1], p[2] - here[2]]), 1.0),
                Placed::Spot { position: p, axis, exponent, cos_cone } => {
                    let l = normalize([p[0] - here[0], p[1] - here[1], p[2] - here[2]]);
                    let along = -dot(l, *axis);
                    let lit = along > 0.0 && cos_cone.is_none_or(|c| along >= c);
                    (l, if lit { along.powf(*exponent) } else { 0.0 })
                }
            };
            let c = color.map(|c| c * strength);
            let v = match shading {
                Shading::Diffuse { constant } => {
                    let k = constant * dot(normal, to_light).max(0.0);
                    let [r, g, b] = c.map(|c| (k * c).clamp(0.0, 1.0));
                    [r, g, b, 1.0]
                }
                Shading::Specular { constant, exponent } => {
                    let half = normalize([to_light[0], to_light[1], to_light[2] + 1.0]);
                    let k = constant * dot(normal, half).max(0.0).powf(exponent.clamp(1.0, 128.0));
                    let [r, g, b] = c.map(|c| (k * c).clamp(0.0, 1.0));
                    let a = r.max(g).max(b);
                    [r * a, g * a, b * a, a]
                }
            };
            put(&mut out, y * w + x, v.map(|v| if v.is_finite() { v as f32 } else { 0.0 }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_surface_takes_the_cosine_of_the_light() {
        // Opaque everywhere: the normal points straight up, and a light at 30° elevation lights
        // it by sin 30° = ½.
        let (w, h) = (4, 4);
        let px = vec![255u8; w * h * 4];
        let placed = place(&LightSource::Distant { azimuth: 0.0, elevation: 30.0 }, Affine::IDENTITY);
        let out = light(&px, w, h, 5.0, [1.0, 1.0, 1.0], &placed, &Shading::Diffuse { constant: 1.0 });
        for p in out.chunks(4) {
            assert!((i16::from(p[0]) - 128).abs() <= 1 && p[3] == 255, "{p:?}");
        }
        // A light straight above, seen specularly, is at its brightest.
        let placed = place(&LightSource::Distant { azimuth: 0.0, elevation: 90.0 }, Affine::IDENTITY);
        let out = light(&px, w, h, 5.0, [0.0, 1.0, 0.0], &placed, &Shading::Specular { constant: 1.0, exponent: 20.0 });
        assert_eq!(&out[0..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn a_slope_facing_the_light_is_brighter_than_one_facing_away() {
        // Alpha rising left to right; the light comes from the left (azimuth 180°).
        let (w, h) = (8, 3);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                px[(y * w + x) * 4 + 3] = (x * 32) as u8;
            }
        }
        let from = |azimuth: f64| {
            let placed = place(&LightSource::Distant { azimuth, elevation: 45.0 }, Affine::IDENTITY);
            light(&px, w, h, 4.0, [1.0, 1.0, 1.0], &placed, &Shading::Diffuse { constant: 1.0 })[(w + 4) * 4]
        };
        assert!(from(180.0) > from(0.0), "{} {}", from(180.0), from(0.0));
    }
}
