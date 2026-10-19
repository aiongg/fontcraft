//! A layer's placed image under the pointer: where it lies, its knobs while it is selected, and
//! the drags that move, scale, rotate and crop it. Rows `image.directManipulation` and
//! `image.lockedNotSelectable` (the gestures); package M11.13.
//!
//! This module is geometry only. The shell keeps which image is selected, starts an interaction
//! for a drag, and previews what [`ImageDrag::invocation`] returns at each pointer position, so a
//! gesture ends as one `image.setTransform` or `image.setCrop`: the command an agent would run.
//!
//! | Gesture | Command |
//! |---|---|
//! | drag the image (Select tool) | `image.setTransform` with `x`, `y`; `Shift` keeps the larger of the two moves |
//! | drag a knob (Select tool, Show Bounding Box on) | `image.setTransform` with `x`, `y`, `width`, `height`, about the opposite knob; a side knob scales one axis; `Shift`: the same factor on both axes; `Alt`: about the centre |
//! | `Ctrl+Alt` drag a knob | `image.setCrop`: the knob's edges follow the pointer, in whole pixels, within the image |
//! | drag with the Rotate tool | `image.setTransform` with `x`, `y`, `rotation`, about the centre of the shown part; `Shift`: steps of 90° |
//! | drag with the Scale tool | `image.setTransform` with `x`, `y`, `width`, `height`, about the centre of the shown part; `Shift`: the same factor on both axes |
//!
//! Spaces: the image's own space is in its pixels (for a PDF, its points), y up from the image's
//! bottom-left corner, as crops are stored; the image's transform maps it into font units. Knobs
//! are named in the image's own space, so on a mirrored or rotated image `Top` is the knob on
//! the image's top edge wherever that lies on screen. Scaling happens along the image's own axes,
//! so a rotated image stays a rotated rectangle.
//!
//! A locked image is never hit ([`ImageGeom::contains`] says no), so clicks and drags pass
//! through it; the context menu finds it with [`ImageGeom::covers`].

use fontcraft_geom::{Affine, Point, Rect, Vec2};
use fontcraft_model::{Image, MAX_COORDINATE, Transform};
use serde_json::{Map, Value, json};

use crate::bbox::{Knob, rotation_angle, scale_factors, snap_angle};
use crate::event::Modifiers;
use crate::output::Invocation;
use crate::view::View;

/// The smallest factor a scale drag sends: the image never shrinks to nothing or flips.
pub const MIN_FACTOR: f64 = 1e-3;
/// The narrowest crop a crop drag leaves, in image pixels.
pub const MIN_CROP: f64 = 1.0;
/// Values sent are rounded to this many parts per unit, so float noise does not reach the font.
const PRECISION: f64 = 1e6;

fn round(v: f64) -> f64 {
    (v * PRECISION).round() / PRECISION + 0.0
}

/// An image transform split as format versions 3 and 4 store it: position, scale, rotation and
/// slant, angles in degrees, counter-clockwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub pos: Point,
    pub scale: (f64, f64),
    pub angle: f64,
    pub slant: (f64, f64),
}

impl Parts {
    /// `t` split; `None` for a matrix with shear, which has no rotation-and-scale form (the
    /// engine's `image.setTransform` splits a matrix the same way).
    pub fn of(t: &Transform) -> Option<Parts> {
        match *t {
            Transform::Decomposed { pos, scale, angle, slant } => Some(Parts { pos: Point::new(pos.0, pos.1), scale, angle, slant }),
            Transform::Matrix([a, b, c, d, dx, dy]) => {
                let sx = a.hypot(b);
                let size = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
                if !sx.is_finite() || sx <= 0.0 || (a * c + b * d).abs() > 1e-9 * size * size {
                    return None;
                }
                Some(Parts { pos: Point::new(dx, dy), scale: (sx, (a * d - b * c) / sx), angle: b.atan2(a).to_degrees(), slant: (0.0, 0.0) })
            }
        }
    }

    /// Whether the transform has no slant, so a rotation about any point is a new position and
    /// angle.
    pub fn unslanted(&self) -> bool {
        self.slant == (0.0, 0.0)
    }
}

/// Where a placed image lies.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageGeom {
    /// The image's own space to font units.
    pub affine: Affine,
    /// The image's size in its own space, when its file has been read.
    pub size: Option<(f64, f64)>,
    /// The shown part, `[x, y, width, height]` in the image's own space.
    pub crop: Option<[f64; 4]>,
    pub locked: bool,
    pub parts: Option<Parts>,
}

impl ImageGeom {
    /// The geometry of `image`, whose file is `size` large in its own space (`None` when it
    /// could not be read).
    pub fn new(image: &Image, size: Option<(f64, f64)>) -> Self {
        let size = size.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0);
        let crop = image.crop.filter(|&[x, y, w, h]| [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0.0 && h > 0.0);
        Self { affine: image.transform.affine(), size, crop, locked: image.locked, parts: Parts::of(&image.transform) }
    }

    /// The shown part in the image's own space: the crop within the image, or the whole image.
    /// `None` when neither the size nor a crop is known, or the transform cannot be inverted.
    pub fn visible(&self) -> Option<Rect> {
        let det = self.affine.determinant();
        if !det.is_finite() || det.abs() <= f64::EPSILON {
            return None;
        }
        let whole = self.size.map(|(w, h)| Rect::new(0.0, 0.0, w, h));
        let crop = self.crop.map(|[x, y, w, h]| Rect::new(x, y, x + w, y + h));
        let r = match (crop, whole) {
            (Some(c), Some(w)) => {
                let i = c.intersect(w);
                if i.width() > 0.0 && i.height() > 0.0 { i } else { w }
            }
            (Some(c), None) => c,
            (None, w) => w?,
        };
        Some(r)
    }

    /// The shown part's corners in font units: bottom-left, bottom-right, top-right, top-left
    /// of the image's own space.
    pub fn corners(&self) -> Option<[Point; 4]> {
        let r = self.visible()?;
        Some([Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].map(|p| self.affine * p))
    }

    /// The font-unit box around the shown part.
    pub fn bounds(&self) -> Option<Rect> {
        let [a, b, c, d] = self.corners()?;
        Some(Rect::from_points(a, c).union_pt(b).union_pt(d))
    }

    /// `p` (font units) in the image's own space.
    pub fn to_image(&self, p: Point) -> Option<Point> {
        let det = self.affine.determinant();
        if !det.is_finite() || det.abs() <= f64::EPSILON {
            return None;
        }
        let q = self.affine.inverse() * p;
        (q.x.is_finite() && q.y.is_finite()).then_some(q)
    }

    /// Whether `p` lies on the shown part, locked or not.
    pub fn covers(&self, p: Point) -> bool {
        match (self.visible(), self.to_image(p)) {
            (Some(r), Some(q)) => q.x >= r.x0 && q.x <= r.x1 && q.y >= r.y0 && q.y <= r.y1,
            _ => false,
        }
    }

    /// Whether a click at `p` reaches the image: it covers `p` and is not locked.
    pub fn contains(&self, p: Point) -> bool {
        !self.locked && self.covers(p)
    }

    /// The knobs on the shown part's corners and edge midpoints, in font units.
    pub fn knobs(&self) -> Vec<(Knob, Point)> {
        let Some(r) = self.visible() else { return Vec::new() };
        Knob::ALL.iter().map(|&k| (k, self.affine * k.point_in(r))).collect()
    }

    /// The knob within `tolerance` screen points of `p`, the nearest when several are; never on
    /// a locked image.
    pub fn knob_at(&self, p: Point, view: &View, tolerance: f64) -> Option<Knob> {
        if self.locked {
            return None;
        }
        let at = view.to_screen(p);
        self.knobs()
            .into_iter()
            .map(|(k, q)| (k, (view.to_screen(q) - at).hypot()))
            .filter(|(_, d)| d.is_finite() && *d <= tolerance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(k, _)| k)
    }

    /// The shown part's centre in the image's own space.
    fn center(&self) -> Option<Point> {
        self.visible().map(|r| r.center())
    }

    /// The transform's translation: the position `image.setTransform` takes as `x`, `y`.
    fn position(&self) -> Point {
        let [_, _, _, _, x, y] = self.affine.as_coeffs();
        Point::new(x, y)
    }

    /// The linear part of the transform applied to `v`.
    fn linear(&self, v: Vec2) -> Vec2 {
        let [a, b, c, d, _, _] = self.affine.as_coeffs();
        Vec2::new(a * v.x + c * v.y, b * v.x + d * v.y)
    }
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    Move,
    /// A knob dragged: the image scales.
    Knob(Knob),
    /// A knob dragged with `Ctrl+Alt`: the crop's edges follow.
    Crop(Knob),
    /// The Rotate tool.
    Rotate,
    /// The Scale tool.
    Scale,
}

/// A drag on an image, from the press to the release.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageDrag {
    pub kind: DragKind,
    /// The press, in font units.
    pub down: Point,
    /// The image when the press happened; every preview starts from it.
    pub geom: ImageGeom,
    /// The command's targets: `glyph`, `layer` and `side`.
    pub target: Map<String, Value>,
}

impl ImageDrag {
    pub fn new(kind: DragKind, down: Point, geom: ImageGeom, target: Map<String, Value>) -> Self {
        Self { kind, down, geom, target }
    }

    /// The undo step's name.
    pub fn label(&self) -> &'static str {
        match self.kind {
            DragKind::Move => "Move Image",
            DragKind::Knob(_) | DragKind::Scale => "Scale Image",
            DragKind::Crop(_) => "Crop Image",
            DragKind::Rotate => "Rotate Image",
        }
    }

    /// The command that puts the image where the pointer at `pos` says; `None` when the image
    /// cannot take this drag (no pixel size to scale by, a slanted image rotated, a pointer on
    /// the fixed point).
    pub fn invocation(&self, pos: Point, mods: Modifiers) -> Option<Invocation> {
        if !pos.x.is_finite() || !pos.y.is_finite() {
            return None;
        }
        let g = &self.geom;
        match self.kind {
            DragKind::Move => {
                let mut d = pos - self.down;
                if mods.shift {
                    if d.x.abs() >= d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                let to = g.position() + d;
                self.transform(json!({"x": round(to.x), "y": round(to.y)}))
            }
            DragKind::Knob(knob) => {
                let r = g.visible()?;
                let fixed = if mods.alt { r.center() } else { knob.opposite().point_in(r) };
                let (sx, sy) = knob.side();
                let (fx, fy) = scale_factors(fixed, knob.point_in(r), g.to_image(pos)?, (sx != 0, sy != 0), mods.shift, 0.0);
                self.scaled(fixed, fx, fy)
            }
            DragKind::Scale => {
                let fixed = g.center()?;
                let (fx, fy) = scale_factors(fixed, g.to_image(self.down)?, g.to_image(pos)?, (true, true), mods.shift, 0.0);
                self.scaled(fixed, fx, fy)
            }
            DragKind::Rotate => {
                let parts = g.parts.filter(Parts::unslanted)?;
                let pivot = g.affine * g.center()?;
                let mut turn = rotation_angle(pivot, self.down, pos);
                if mods.shift {
                    turn = snap_angle(turn, 90.0);
                }
                let (sin, cos) = turn.to_radians().sin_cos();
                let v = g.position() - pivot;
                let to = pivot + Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
                let angle = (parts.angle + turn).rem_euclid(360.0) + 0.0;
                self.transform(json!({"x": round(to.x), "y": round(to.y), "rotation": round(angle)}))
            }
            DragKind::Crop(knob) => {
                let r = g.visible()?;
                let q = g.to_image(pos)?;
                let (w, h) = g.size.unwrap_or((MAX_COORDINATE, MAX_COORDINATE));
                let (sx, sy) = knob.side();
                let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
                let edge = |v: f64, lo: f64, hi: f64| if lo <= hi { v.round().clamp(lo, hi) } else { lo };
                match sx {
                    s if s < 0 => x0 = edge(q.x, 0.0, x1 - MIN_CROP),
                    s if s > 0 => x1 = edge(q.x, x0 + MIN_CROP, w),
                    _ => {}
                }
                match sy {
                    s if s < 0 => y0 = edge(q.y, 0.0, y1 - MIN_CROP),
                    s if s > 0 => y1 = edge(q.y, y0 + MIN_CROP, h),
                    _ => {}
                }
                let rect = [x0, y0, x1 - x0, y1 - y0].map(round);
                let [_, _, cw, ch] = rect;
                if rect.iter().any(|v| !v.is_finite()) || cw <= 0.0 || ch <= 0.0 {
                    return None;
                }
                let mut m = self.target.clone();
                m.insert("rect".into(), json!(rect));
                Some(Invocation::new("image.setCrop", Value::Object(m)))
            }
        }
    }

    /// `image.setTransform` with the targets and `extra`.
    fn transform(&self, extra: Value) -> Option<Invocation> {
        let mut m = self.target.clone();
        if let Value::Object(e) = extra {
            m.extend(e);
        }
        Some(Invocation::new("image.setTransform", Value::Object(m)))
    }

    /// The image scaled by `fx`, `fy` along its own axes about `fixed` (its own space), which
    /// stays where it is in the glyph.
    fn scaled(&self, fixed: Point, fx: f64, fy: f64) -> Option<Invocation> {
        let g = &self.geom;
        let parts = g.parts?;
        let (w, h) = g.size?;
        let clamp = |f: f64| if f.is_finite() { f.max(MIN_FACTOR) } else { 1.0 };
        let (fx, fy) = (clamp(fx), clamp(fy));
        let at = g.affine * fixed;
        // The image's own vector to `fixed`, scaled, then through the unscaled transform.
        let to = at - g.linear(Vec2::new(fixed.x * fx, fixed.y * fy));
        let width = w * (parts.scale.0 * fx).abs();
        let height = h * (parts.scale.1 * fy).abs();
        let values = [to.x, to.y, width, height];
        if values.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE) || width <= 0.0 || height <= 0.0 {
            return None;
        }
        self.transform(json!({"x": round(to.x), "y": round(to.y), "width": round(width), "height": round(height)}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(transform: Transform, crop: Option<[f64; 4]>) -> Image {
        Image { path: Some("A.png".into()), transform, crop, ..Default::default() }
    }

    fn target() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("glyph".into(), json!("A"));
        m
    }

    fn params(inv: Option<Invocation>) -> (String, Value) {
        let inv = inv.expect("an invocation");
        (inv.id, inv.params)
    }

    #[test]
    fn a_cropped_rotated_image_is_hit_only_on_its_shown_part() {
        let t = Transform::Decomposed { pos: (100.0, 0.0), scale: (2.0, 2.0), angle: 90.0, slant: (0.0, 0.0) };
        let g = ImageGeom::new(&image(t, Some([0.0, 0.0, 50.0, 100.0])), Some((100.0, 100.0)));
        // Pixel (25, 50) → scaled (50, 100) → turned a quarter → (-100, 50) → moved (0, 50).
        assert!(g.contains(Point::new(0.0, 50.0)));
        // Pixel (75, 50) lies in the cropped-away half.
        assert!(!g.contains(Point::new(0.0, 150.0)));
        let locked = ImageGeom { locked: true, ..g.clone() };
        assert!(!locked.contains(Point::new(0.0, 50.0)) && locked.covers(Point::new(0.0, 50.0)));
        assert_eq!(locked.knob_at(Point::new(100.0, 0.0), &View::new(1.0, Point::ZERO), 10.0), None);
    }

    #[test]
    fn a_corner_knob_scales_about_the_opposite_corner() {
        let t = Transform::Decomposed { pos: (10.0, 20.0), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) };
        let g = ImageGeom::new(&image(t, None), Some((100.0, 50.0)));
        let drag = ImageDrag::new(DragKind::Knob(Knob::TopRight), Point::new(110.0, 70.0), g, target());
        let (id, p) = params(drag.invocation(Point::new(210.0, 120.0), Modifiers::NONE));
        assert_eq!(id, "image.setTransform");
        assert_eq!(p, json!({"glyph": "A", "x": 10.0, "y": 20.0, "width": 200.0, "height": 100.0}));
        // About the bottom-left knob's opposite: the top-right corner stays.
        let drag = ImageDrag { kind: DragKind::Knob(Knob::BottomLeft), ..drag };
        let (_, p) = params(drag.invocation(Point::new(-40.0, 20.0), Modifiers::NONE));
        assert_eq!(p, json!({"glyph": "A", "x": -40.0, "y": 20.0, "width": 150.0, "height": 50.0}));
    }

    #[test]
    fn a_rotate_drag_turns_about_the_centre() {
        let t = Transform::Decomposed { pos: (0.0, 0.0), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) };
        let g = ImageGeom::new(&image(t, None), Some((100.0, 100.0)));
        let drag = ImageDrag::new(DragKind::Rotate, Point::new(100.0, 50.0), g, target());
        let (_, p) = params(drag.invocation(Point::new(50.0, 100.0), Modifiers::NONE));
        // A quarter turn about (50, 50) takes the corner (0, 0) to (100, 0).
        assert_eq!(p, json!({"glyph": "A", "x": 100.0, "y": 0.0, "rotation": 90.0}));
    }

    #[test]
    fn a_crop_drag_stays_within_the_image_in_whole_pixels() {
        let t = Transform::Decomposed { pos: (0.0, 0.0), scale: (2.0, 2.0), angle: 0.0, slant: (0.0, 0.0) };
        let g = ImageGeom::new(&image(t, None), Some((100.0, 100.0)));
        let drag = ImageDrag::new(DragKind::Crop(Knob::TopRight), Point::new(200.0, 200.0), g, target());
        let (id, p) = params(drag.invocation(Point::new(121.3, 500.0), Modifiers::NONE));
        assert_eq!(id, "image.setCrop");
        assert_eq!(p["rect"], json!([0.0, 0.0, 61.0, 100.0]));
    }
}
