//! Typed paint attributes of a full-colour layer's shapes: colours that are either RGBA or a
//! palette reference, gradients, and shadows (`docs/spec/15-color-fonts.md` § 6; plan
//! `docs/plans/M11.md` § 3 › Paint attributes in the model).
//!
//! These are the fields of [`crate::ShapeAttrs`] that `fontcraft-color` reads to build the paint
//! graph. The `.glyphs` readers fill them from the `attr` keys `fillColor`, `strokeColor`,
//! `gradient`, `shadow` and `shadowIn`; gray colours become RGBA colours with equal components.
//! A radial gradient from a version 3 file, which stores no radii, has both radii 0.
//!
//! Gradient geometry is stored as the file format stores it: positions as fractions of the
//! shape's bounding box (0,0 its bottom left, 1,1 its top right), so a gradient follows its path
//! when the path moves or is scaled.

use fontcraft_geom::Point;
use serde::{Deserialize, Serialize};

/// Most stops in one gradient. Offsets are finite; writers clamp them to 0..1.
pub const MAX_GRADIENT_STOPS: usize = 256;

/// A colour: red, green, blue and alpha from 0 to 1, or an entry of the font's colour palettes
/// with its own alpha (the version 4 form `("p", index, alpha)`).
///
/// Serialised untagged: an RGBA colour is a plain array of four numbers.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Color {
    Rgba([f64; 4]),
    Palette {
        /// The palette row; 65,535 is the text colour.
        index: u16,
        /// Opacity from 0 to 1, multiplied with the palette entry's own alpha.
        alpha: f64,
    },
}

impl Color {
    /// Every number, for range checks.
    pub fn numbers(&self) -> impl Iterator<Item = f64> + '_ {
        let (rgba, alpha) = match self {
            Color::Rgba(c) => (Some(c), None),
            Color::Palette { alpha, .. } => (None, Some(*alpha)),
        };
        rgba.into_iter().flatten().copied().chain(alpha)
    }

    /// Whether every component and the alpha lie from 0 to 1.
    pub fn is_valid(&self) -> bool {
        self.numbers().all(|c| (0.0..=1.0).contains(&c))
    }
}

/// What a gradient paints outside its stops' range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Extend {
    /// The end colours continue.
    #[default]
    Pad,
    /// The stops repeat.
    Repeat,
    /// The stops repeat, every other run mirrored.
    Reflect,
}

/// One colour of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorStop {
    /// Where the colour sits along the gradient, from 0 (start) to 1 (end).
    pub offset: f64,
    pub color: Color,
}

/// A gradient's geometry. Points are fractions of the shape's bounding box; radii are fractions
/// of its width; angles are in degrees, counter-clockwise from the positive x axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum GradientKind {
    Linear {
        start: Point,
        end: Point,
    },
    Radial {
        /// Centre of the start circle.
        start: Point,
        /// Centre of the end circle.
        end: Point,
        start_radius: f64,
        end_radius: f64,
        /// The rotation of the ellipse the circles become in a box that is not square.
        angle: f64,
    },
    Conic {
        center: Point,
        start_angle: f64,
        /// How far the sweep turns from `start_angle`.
        angle: f64,
        /// The radius of the editor's knob for the sweep; not painted.
        control_radius: f64,
    },
}

/// A gradient fill. A gradient replaces the shape's fill colour
/// (`color.fullColor.gradientReplacesFill`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    /// In offset order; at least two in a gradient a command made, at most
    /// [`MAX_GRADIENT_STOPS`].
    pub stops: Vec<ColorStop>,
    #[serde(default)]
    pub extend: Extend,
}

impl Gradient {
    /// Every number, for range checks.
    pub fn numbers(&self) -> impl Iterator<Item = f64> + '_ {
        let geometry: Vec<f64> = match self.kind {
            GradientKind::Linear { start, end } => vec![start.x, start.y, end.x, end.y],
            GradientKind::Radial { start, end, start_radius, end_radius, angle } => {
                vec![start.x, start.y, end.x, end.y, start_radius, end_radius, angle]
            }
            GradientKind::Conic { center, start_angle, angle, control_radius } => vec![center.x, center.y, start_angle, angle, control_radius],
        };
        geometry.into_iter().chain(self.stops.iter().flat_map(|s| std::iter::once(s.offset).chain(s.color.numbers())))
    }
}

/// A shadow or inner shadow: stored and drawn by image export only, never written to a font
/// (`color.fullColor.imageOnlyEffects`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    /// Blur radius in font units.
    pub blur: f64,
    pub color: Color,
    /// Offset in font units.
    pub offset: [f64; 2],
}

impl Shadow {
    /// Every number, for range checks.
    pub fn numbers(&self) -> impl Iterator<Item = f64> + '_ {
        let [x, y] = self.offset;
        [self.blur, x, y].into_iter().chain(self.color.numbers())
    }
}
