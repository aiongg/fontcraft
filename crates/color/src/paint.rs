//! The paint graph (M11.6 for solid layers, M11.7 for full-colour layers, M11.19 for
//! variation): one tree per colour glyph, read by Edit View, Font View, Text Preview, the SVG
//! writer and the COLR writer alike.
//!
//! Rows: `color.export.colrFromPaletteLayers`, `color.export.fullColorToColrV1`,
//! `color.export.strokeInColr`, `compile.colrGradientGeometry`, `color.layered.stackingOrder`.
//!
//! Contract (`docs/plans/M11.md` § 3):
//! - [`glyph_paint`] gives `None` for a glyph without vector colour layers (palette or
//!   full-colour) for the master. Otherwise a [`Paint::Layers`] with one entry per layer glyph,
//!   bottom first: each a [`Paint::Glyph`] whose [`ShapeRef`] names the shapes it fills.
//! - Consecutive shapes of one layer with equal paint attributes are merged into one
//!   [`ShapeRef`], as both oracles merge them; each [`ShapeRef`] becomes one layer glyph
//!   `<base>.color<i>`, numbered by its position.
//! - A stroke has no COLR paint: it becomes its own [`ShapeRef`] with a [`StrokeSpec`], above the
//!   fill, which the COLR writer expands to an outline and the SVG writer may draw as a stroke.
//! - Gradient geometry is in font units (the model's fractions of the shape's bounds resolved
//!   against the merged shapes' bounds). Alpha is kept as given; colours that are not palette
//!   references are added to the palettes by the CPAL writer, not here.
//! - Outlines are not drawn here: callers draw a [`ShapeRef`] with their own outline code, so
//!   this crate needs no path operations.
//! - Depth at most [`crate::MAX_PAINT_DEPTH`]; stops at most [`crate::MAX_STOPS`].

use fontcraft_geom::Point;
use fontcraft_model::{Extend, Font, Glyph, PaletteEntry, Shape};

use crate::layers::{ColorLayerKind, color_layers};
use crate::palette::FOREGROUND_INDEX;
use crate::{ColorError, Palettes};

/// A node of the paint graph.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// Paints drawn in order, the first at the bottom.
    Layers(Vec<Paint>),
    /// `paint` clipped to the outline of `shape`.
    Glyph {
        shape: ShapeRef,
        paint: Box<Paint>,
    },
    Solid(ColorRef),
    /// A linear gradient from `p0` to `p1`, `p2` setting the rotation as in COLR version 1 (at
    /// right angles to `p0`–`p1` for an unrotated gradient).
    Linear {
        p0: Point,
        p1: Point,
        p2: Point,
        line: ColorLine,
    },
    /// A gradient between two circles.
    Radial {
        c0: Point,
        r0: f64,
        c1: Point,
        r1: f64,
        line: ColorLine,
    },
    /// A conic gradient around `center` from angle `start` to `end`, in degrees counter-clockwise
    /// from the positive x axis.
    Sweep {
        center: Point,
        start: f64,
        end: f64,
        line: ColorLine,
    },
}

/// A gradient's stops and what it paints beyond them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorLine {
    /// In offset order.
    pub stops: Vec<ColorStop>,
    pub extend: Extend,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f64,
    pub color: ColorRef,
}

/// A colour as the tables refer to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorRef {
    /// A palette row, with an alpha from 0 to 1 applied over the entry's own.
    Palette { index: u16, alpha: f64 },
    /// The text colour (palette index 65,535).
    Foreground { alpha: f64 },
    /// A colour that is not in the palettes: red, green, blue and alpha from 0 to 1.
    Rgba([f64; 4]),
}

/// Shapes of one layer that one layer glyph draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRef {
    /// The layer's id within the glyph (a master layer's id for the masters of a layered font).
    pub layer: String,
    /// Indices into the layer's shapes (paths and components), in order.
    pub shapes: Vec<usize>,
    /// `Some` when this layer glyph is the shapes' stroke rather than their fill.
    pub stroke: Option<StrokeSpec>,
}

/// A stroke to expand, in font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSpec {
    pub width: f64,
    /// The vertical size; the width when equal.
    pub height: f64,
    /// −1 inside, 0 centred, 1 outside (the version 4 convention).
    pub position: f64,
    /// 0 butt, 1 round, 2 square, 3 round inset.
    pub cap_start: u8,
    pub cap_end: u8,
    /// 0 miter, 1 round, 2 bevel, 3 round full.
    pub join: u8,
}

/// The paint graph of `glyph` for `master_id`, colours resolved against `palettes` where a
/// writer needs them baked in. `None` when the glyph has no vector colour layers for the master.
///
/// A palette layer is one layer glyph of all its paths and components in its palette entry; a
/// layer without them is left out, as fontc leaves it out. Intermediate colour layers are sources
/// of their layer glyph's variation (M11.19), not layers of their own.
pub fn glyph_paint(font: &Font, glyph: &Glyph, master_id: &str, palettes: &Palettes) -> Result<Option<Paint>, ColorError> {
    let _ = (font, palettes);
    let mut layers = Vec::new();
    for (layer, kind) in color_layers(glyph, master_id) {
        if !layer.info.attrs.coordinates.is_empty() {
            continue;
        }
        match kind {
            ColorLayerKind::Palette(entry) => {
                let shapes = drawn_shapes(&layer.shapes);
                if shapes.is_empty() {
                    continue;
                }
                let color = match entry {
                    PaletteEntry::Index(index) if index != FOREGROUND_INDEX => ColorRef::Palette { index, alpha: 1.0 },
                    PaletteEntry::Index(_) | PaletteEntry::Foreground => ColorRef::Foreground { alpha: 1.0 },
                };
                layers.push(Paint::Glyph { shape: ShapeRef { layer: layer.id.clone(), shapes, stroke: None }, paint: Box::new(Paint::Solid(color)) });
            }
            // M11.7: full-colour layers.
            ColorLayerKind::FullColor | ColorLayerKind::IColor(_) | ColorLayerKind::Svg => {}
        }
    }
    Ok((!layers.is_empty()).then_some(Paint::Layers(layers)))
}

/// The indices of the shapes in `shapes` that have an outline: paths and components.
pub fn drawn_shapes(shapes: &[Shape]) -> Vec<usize> {
    shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_) | Shape::Component(_))).map(|(i, _)| i).collect()
}

/// Whether every paint in `paint` is solid, so the glyph fits COLR version 0
/// (`color.export.colrVersionChoice`).
pub fn is_solid_stack(paint: &Paint) -> bool {
    // M11.7
    let _ = paint;
    true
}

/// The layer glyphs `paint` needs, in order: the [`ShapeRef`] of every [`Paint::Glyph`].
/// Nothing below [`crate::MAX_PAINT_DEPTH`] is visited.
pub fn shape_refs(paint: &Paint) -> Vec<&ShapeRef> {
    fn collect<'a>(paint: &'a Paint, depth: usize, out: &mut Vec<&'a ShapeRef>) {
        if depth > crate::MAX_PAINT_DEPTH {
            return;
        }
        match paint {
            Paint::Layers(list) => list.iter().for_each(|p| collect(p, depth + 1, out)),
            Paint::Glyph { shape, paint } => {
                out.push(shape);
                collect(paint, depth + 1, out);
            }
            Paint::Solid(_) | Paint::Linear { .. } | Paint::Radial { .. } | Paint::Sweep { .. } => {}
        }
    }
    let mut out = Vec::new();
    collect(paint, 0, &mut out);
    out
}
