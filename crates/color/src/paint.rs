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
//!   The fill [`ShapeRef`] of a stroked path is the path's own outline, not its stroke.
//! - A master layer marked as full colour (fontc's test sources keep their paints there) paints
//!   like a full-colour layer, below the master's colour layers.
//! - Gradient geometry is in font units (the model's fractions of the shape's bounds resolved
//!   against the merged shapes' bounds). Alpha is kept as given; colours that are not palette
//!   references are added to the palettes by the CPAL writer, not here.
//! - Outlines are not drawn here: callers draw a [`ShapeRef`] with their own outline code, so
//!   this crate needs no path operations.
//! - Depth at most [`crate::MAX_PAINT_DEPTH`]; stops at most [`crate::MAX_STOPS`].

use fontcraft_geom::{BezPath, Point, Rect, Shape as _};
use fontcraft_model::{Color, Dimension, Extend, Font, Glyph, Gradient, GradientKind, Layer, PaletteEntry, Shape, ShapeAttrs};

use crate::layers::{ColorLayerKind, export_kind};
use crate::palette::FOREGROUND_INDEX;
use crate::{ColorError, MAX_LAYER_GLYPHS, Palettes};

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

/// The fill of a shape on a full-colour layer with neither a fill colour nor a gradient: the
/// 50 % gray a new shape gets (`color.fullColor.defaultGrayFill`).
pub const DEFAULT_FILL: Color = Color::Rgba([0.5, 0.5, 0.5, 1.0]);
/// The colour of a stroke with a width and no stroke colour.
pub const DEFAULT_STROKE: Color = Color::Rgba([0.0, 0.0, 0.0, 1.0]);

/// The paint graph of `glyph` for `master_id`, colours resolved against `palettes` where a
/// writer needs them baked in. `None` when the glyph has no vector colour layers for the master.
///
/// A palette layer is one layer glyph of all its paths and components in its palette entry; a
/// layer without them is left out, as fontc leaves it out. A full-colour layer gives one layer
/// glyph per run of consecutive shapes with equal paint attributes, and for stroked paths a
/// stroke layer glyph above the run's fill. A master layer marked as full colour paints that way
/// too, below the master's colour layers, as both oracles read it. Intermediate colour layers are
/// sources of their layer glyph's variation (M11.19), not layers of their own.
pub fn glyph_paint(font: &Font, glyph: &Glyph, master_id: &str, palettes: &Palettes) -> Result<Option<Paint>, ColorError> {
    let _ = palettes;
    let mut layers = Vec::new();
    let painted = glyph.layers.iter().filter(|l| l.master == master_id).filter_map(|l| export_kind(l).map(|k| (l, k))).take(crate::MAX_COLOR_LAYERS);
    for (layer, kind) in painted {
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
            ColorLayerKind::FullColor => full_color_paints(font, glyph, layer, &mut layers)?,
            ColorLayerKind::IColor(_) | ColorLayerKind::Svg => {}
        }
        if layers.len() > MAX_LAYER_GLYPHS {
            return Err(ColorError::LimitExceeded { what: "layer glyphs in one colour glyph", limit: MAX_LAYER_GLYPHS });
        }
    }
    Ok((!layers.is_empty()).then_some(Paint::Layers(layers)))
}

/// What one shape of a full-colour layer paints; consecutive shapes that paint the same are one
/// run, as both oracles merge them.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    fill: Option<Fill>,
    stroke: Option<(StrokeSpec, ColorRef)>,
}

#[derive(Clone, Debug, PartialEq)]
enum Fill {
    Color(ColorRef),
    Gradient(Gradient),
}

/// The paints of the full-colour `layer`, appended to `out` bottom first.
fn full_color_paints(font: &Font, glyph: &Glyph, layer: &Layer, out: &mut Vec<Paint>) -> Result<(), ColorError> {
    let bad = |message: String| ColorError::BadPaint { glyph: glyph.name.clone(), message };
    let mut runs: Vec<(Look, Vec<usize>)> = Vec::new();
    for i in drawn_shapes(&layer.shapes) {
        let Some(shape) = layer.shapes.get(i) else { continue };
        let look = look(shape).map_err(bad)?;
        match runs.last_mut() {
            Some((last, shapes)) if *last == look => shapes.push(i),
            _ => runs.push((look, vec![i])),
        }
    }
    for (look, shapes) in runs {
        if let Some(fill) = &look.fill {
            let paint = match fill {
                Fill::Color(c) => Paint::Solid(*c),
                Fill::Gradient(g) => gradient_paint(g, run_bounds(font, layer, &shapes).map_err(bad)?).map_err(bad)?,
            };
            out.push(Paint::Glyph { shape: ShapeRef { layer: layer.id.clone(), shapes: shapes.clone(), stroke: None }, paint: Box::new(paint) });
        }
        if let Some((spec, color)) = look.stroke {
            out.push(Paint::Glyph { shape: ShapeRef { layer: layer.id.clone(), shapes, stroke: Some(spec) }, paint: Box::new(Paint::Solid(color)) });
        }
        if out.len() > MAX_LAYER_GLYPHS {
            return Err(ColorError::LimitExceeded { what: "layer glyphs in one colour glyph", limit: MAX_LAYER_GLYPHS });
        }
    }
    Ok(())
}

/// The paint of one shape. A gradient wins over the fill colour; a shape with neither is filled
/// with [`DEFAULT_FILL`] unless it is a stroke that is not also filled. Only paths have strokes.
/// A fill or stroke colour with no opacity paints nothing.
fn look(shape: &Shape) -> Result<Look, String> {
    let (attrs, stroke) = match shape {
        Shape::Path(p) => (&p.info.attrs, stroke_spec(&p.info.attrs)?),
        Shape::Component(c) => (&c.info.attrs, None),
        Shape::Image(_) | Shape::Raw(_) => return Ok(Look { fill: None, stroke: None }),
    };
    let fill = match (&attrs.gradient, attrs.fill_color) {
        (Some(g), _) => Some(Fill::Gradient(g.clone())),
        (None, Some(c)) => visible(&c).then(|| Fill::Color(color_ref(c))),
        (None, None) => (stroke.is_none() || attrs.fill).then(|| Fill::Color(color_ref(DEFAULT_FILL))),
    };
    let stroke = stroke.and_then(|spec| {
        let c = attrs.stroke_color.unwrap_or(DEFAULT_STROKE);
        visible(&c).then(|| (spec, color_ref(c)))
    });
    Ok(Look { fill, stroke })
}

/// The stroke `attrs` describe; `None` without a positive width.
fn stroke_spec(attrs: &ShapeAttrs) -> Result<Option<StrokeSpec>, String> {
    let Some(width) = dimension(&attrs.stroke_width, "stroke width")? else { return Ok(None) };
    if width <= 0.0 {
        return Ok(None);
    }
    Ok(Some(StrokeSpec {
        width,
        height: dimension(&attrs.stroke_height, "stroke height")?.unwrap_or(width),
        position: dimension(&attrs.stroke_pos, "stroke position")?.unwrap_or(0.0),
        cap_start: attrs.line_cap_start.unwrap_or(0),
        cap_end: attrs.line_cap_end.unwrap_or(0),
        join: attrs.line_join.unwrap_or(0),
    }))
}

/// A stroke size or position as a finite number; an expression that is not a plain number cannot
/// be evaluated here.
fn dimension(d: &Option<Dimension>, what: &str) -> Result<Option<f64>, String> {
    let n = match d {
        None => return Ok(None),
        Some(Dimension::Number(n)) => Some(*n),
        Some(Dimension::Expression(e)) => e.trim().parse::<f64>().ok(),
    };
    match n {
        Some(n) if n.is_finite() => Ok(Some(n)),
        _ => Err(format!("its {what} is not a number")),
    }
}

fn visible(c: &Color) -> bool {
    match c {
        Color::Rgba([_, _, _, a]) | Color::Palette { alpha: a, .. } => *a > 0.0,
    }
}

fn color_ref(c: Color) -> ColorRef {
    match c {
        Color::Rgba(rgba) => ColorRef::Rgba(rgba),
        Color::Palette { index, alpha } if index == FOREGROUND_INDEX => ColorRef::Foreground { alpha },
        Color::Palette { index, alpha } => ColorRef::Palette { index, alpha },
    }
}

/// The bounding box of the outlines of `shapes` of `layer`, curve extremes included, which is
/// what gradient fractions are fractions of (`compile.colrGradientGeometry`). The box of the
/// layer glyph these shapes make, so a compiled font opens back to the same fractions. All zero
/// without an outline.
fn run_bounds(font: &Font, layer: &Layer, shapes: &[usize]) -> Result<Rect, String> {
    let mut out: Option<Rect> = None;
    let mut add = |path: &BezPath| {
        if !path.elements().is_empty() {
            let b = path.bounding_box();
            out = Some(out.map_or(b, |o| o.union(b)));
        }
    };
    for s in shapes.iter().filter_map(|&i| layer.shapes.get(i)) {
        match s {
            Shape::Path(p) => add(&p.to_contour()?),
            Shape::Component(c) => {
                let path = font.outline(&c.glyph, &layer.master).map_err(|e| e.to_string())?;
                add(&(c.affine() * path));
            }
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    Ok(out.unwrap_or(Rect::ZERO))
}

/// `gradient` in font units over `bounds` (`compile.colrGradientGeometry`): points are fractions
/// of the box, radii fractions of its width. A linear gradient's third point is the start-to-end
/// vector turned −90° about the start. A radial gradient with both radii 0 (a version 3 file,
/// which stores none) is a circle around `start` from radius 0 to the farthest corner of the box,
/// as both oracles write it. Stops are sorted by offset and their offsets clamped to 0..1.
fn gradient_paint(gradient: &Gradient, bounds: Rect) -> Result<Paint, String> {
    if gradient.stops.len() > crate::MAX_STOPS {
        return Err(format!("a gradient has more than {} stops", crate::MAX_STOPS));
    }
    if !gradient.numbers().all(f64::is_finite) {
        return Err("a gradient has a value that is not a number".to_string());
    }
    let mut stops: Vec<ColorStop> =
        gradient.stops.iter().map(|s| ColorStop { offset: s.offset.clamp(0.0, 1.0), color: color_ref(s.color) }).collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let line = ColorLine { stops, extend: gradient.extend };
    let at = |p: Point| Point::new(bounds.x0 + bounds.width() * p.x, bounds.y0 + bounds.height() * p.y);
    Ok(match gradient.kind {
        GradientKind::Linear { start, end } => {
            let (p0, p1) = (at(start), at(end));
            Paint::Linear { p0, p1, p2: Point::new(p0.x + (p1.y - p0.y), p0.y - (p1.x - p0.x)), line }
        }
        GradientKind::Radial { start, end, start_radius, end_radius, .. } => {
            let c0 = at(start);
            if start_radius == 0.0 && end_radius == 0.0 {
                let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x0, bounds.y1), (bounds.x1, bounds.y1)];
                let r1 = corners.iter().map(|&(x, y)| (x - c0.x).hypot(y - c0.y)).fold(0.0, f64::max);
                Paint::Radial { c0, r0: 0.0, c1: c0, r1, line }
            } else {
                let w = bounds.width();
                Paint::Radial { c0, r0: (start_radius * w).abs(), c1: at(end), r1: (end_radius * w).abs(), line }
            }
        }
        GradientKind::Conic { center, start_angle, angle, .. } => {
            Paint::Sweep { center: at(center), start: start_angle, end: start_angle + angle, line }
        }
    })
}

/// The indices of the shapes in `shapes` that have an outline: paths and components.
pub fn drawn_shapes(shapes: &[Shape]) -> Vec<usize> {
    shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_) | Shape::Component(_))).map(|(i, _)| i).collect()
}

/// Whether every paint in `paint` is solid, so the glyph fits COLR version 0
/// (`color.export.colrVersionChoice`).
///
/// A stack of solid layers whose colours are opaque palette references, the text colour, or
/// colours of their own (their alpha goes into `CPAL`). A palette reference with its own alpha
/// needs version 1, which has an alpha per paint.
pub fn is_solid_stack(paint: &Paint) -> bool {
    let solid_glyph = |p: &Paint| match p {
        Paint::Glyph { paint, .. } => match paint.as_ref() {
            Paint::Solid(ColorRef::Rgba(_)) => true,
            Paint::Solid(ColorRef::Palette { alpha, .. } | ColorRef::Foreground { alpha }) => *alpha >= 1.0,
            _ => false,
        },
        _ => false,
    };
    match paint {
        Paint::Layers(list) => list.iter().all(solid_glyph),
        other => solid_glyph(other),
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

    use fontcraft_model::ColorStop as Stop;

    use super::*;

    fn path(rect: [f64; 4], style: impl FnOnce(&mut ShapeAttrs)) -> Shape {
        let mut p = fontcraft_model::Path::rect(rect[0], rect[1], rect[2], rect[3]);
        style(&mut p.info.attrs);
        Shape::Path(p)
    }

    fn colour_layer(master: &str, id: &str, shapes: Vec<Shape>) -> Layer {
        let mut l = Layer::new(master, 600.0);
        l.id = id.into();
        l.info.attrs.full_color = true;
        l.shapes = shapes;
        l
    }

    fn glyph_with(layers: impl FnOnce(&str) -> Vec<Layer>) -> (Font, Glyph) {
        let font = Font::new("Paints");
        let master = font.masters[0].id.clone();
        let mut g = Glyph::new("A");
        g.layers.push(Layer::new(master.as_str(), 600.0));
        g.layers.extend(layers(&master));
        (font, g)
    }

    fn paint_of(font: &Font, g: &Glyph) -> Result<Option<Paint>, ColorError> {
        glyph_paint(font, g, &font.masters[0].id, &Palettes::default())
    }

    fn solid(layer: &str, shapes: Vec<usize>, stroke: Option<StrokeSpec>, color: ColorRef) -> Paint {
        Paint::Glyph { shape: ShapeRef { layer: layer.into(), shapes, stroke }, paint: Box::new(Paint::Solid(color)) }
    }

    #[test]
    fn full_colour_layers_merge_runs_split_strokes_and_place_gradients_in_font_units() {
        const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];
        let gradient = Gradient {
            kind: GradientKind::Linear { start: Point::new(0.25, 0.5), end: Point::new(0.75, 1.0) },
            // Out of order: written sorted by offset.
            stops: vec![
                Stop { offset: 1.0, color: Color::Palette { index: 0, alpha: 1.0 } },
                Stop { offset: 0.0, color: Color::Palette { index: 1, alpha: 0.5 } },
            ],
            extend: Extend::Reflect,
        };
        let shapes = vec![
            path([0.0, 0.0, 100.0, 100.0], |a| a.fill_color = Some(Color::Rgba(RED))),
            path([200.0, 0.0, 300.0, 100.0], |a| a.fill_color = Some(Color::Rgba(RED))),
            path([100.0, 100.0, 500.0, 400.0], |a| a.gradient = Some(gradient.clone())),
            path([0.0, 0.0, 50.0, 50.0], |a| {
                a.stroke_width = Some(Dimension::Number(10.0));
                a.stroke_pos = Some(Dimension::Number(1.0));
                a.stroke_color = Some(Color::Palette { index: FOREGROUND_INDEX, alpha: 1.0 });
            }),
            path([0.0, 0.0, 50.0, 50.0], |a| a.fill_color = Some(Color::Rgba([0.0, 0.0, 0.0, 0.0]))),
            path([0.0, 0.0, 50.0, 50.0], |_| {}),
            path([0.0, 0.0, 400.0, 300.0], |a| {
                a.fill_color = Some(Color::Rgba(RED));
                a.stroke_width = Some(Dimension::Number(4.0));
                a.line_join = Some(1);
            }),
        ];
        let (font, g) = glyph_with(|m| vec![colour_layer(m, "c", shapes)]);
        let paint = paint_of(&font, &g).unwrap().unwrap();
        let spec = |width: f64, position: f64, join: u8| StrokeSpec { width, height: width, position, cap_start: 0, cap_end: 0, join };
        let expected = Paint::Layers(vec![
            solid("c", vec![0, 1], None, ColorRef::Rgba(RED)),
            Paint::Glyph {
                shape: ShapeRef { layer: "c".into(), shapes: vec![2], stroke: None },
                paint: Box::new(Paint::Linear {
                    // Fractions of the path's box (100, 100)–(500, 400); the third point is the
                    // start-to-end vector turned −90° about the start.
                    p0: Point::new(200.0, 250.0),
                    p1: Point::new(400.0, 400.0),
                    p2: Point::new(350.0, 50.0),
                    line: ColorLine {
                        stops: vec![
                            ColorStop { offset: 0.0, color: ColorRef::Palette { index: 1, alpha: 0.5 } },
                            ColorStop { offset: 1.0, color: ColorRef::Palette { index: 0, alpha: 1.0 } },
                        ],
                        extend: Extend::Reflect,
                    },
                }),
            },
            // A stroke without a fill paints only its stroke, in the text colour here.
            solid("c", vec![3], Some(spec(10.0, 1.0, 0)), ColorRef::Foreground { alpha: 1.0 }),
            // The transparent fill paints nothing; a path without paint attributes is gray.
            solid("c", vec![5], None, ColorRef::Rgba([0.5, 0.5, 0.5, 1.0])),
            // A filled, stroked path: the fill, then its stroke above it in black.
            solid("c", vec![6], None, ColorRef::Rgba(RED)),
            solid("c", vec![6], Some(spec(4.0, 0.0, 1)), ColorRef::Rgba([0.0, 0.0, 0.0, 1.0])),
        ]);
        assert_eq!(paint, expected);
        assert_eq!(shape_refs(&paint).len(), 6, "one layer glyph per paint");
        assert!(!is_solid_stack(&paint), "a gradient needs COLR version 1");

        // A version 3 radial gradient (no radii) is a circle around its start out to the farthest
        // corner of the box.
        let radial = Gradient {
            kind: GradientKind::Radial { start: Point::new(0.25, 0.5), end: Point::new(0.25, 0.5), start_radius: 0.0, end_radius: 0.0, angle: 0.0 },
            stops: gradient.stops.clone(),
            extend: Extend::Pad,
        };
        let (font, g) = glyph_with(|m| vec![colour_layer(m, "c", vec![path([0.0, 0.0, 400.0, 300.0], |a| a.gradient = Some(radial))])]);
        let Some(Paint::Layers(list)) = paint_of(&font, &g).unwrap() else { panic!("one layer") };
        let Paint::Glyph { paint, .. } = &list[0] else { panic!("a glyph paint") };
        let Paint::Radial { c0, r0, c1, r1, .. } = paint.as_ref() else { panic!("a radial gradient: {paint:?}") };
        assert_eq!((*c0, *r0, *c1), (Point::new(100.0, 150.0), 0.0, Point::new(100.0, 150.0)));
        assert!((r1 - 300f64.hypot(150.0)).abs() < 1e-9, "{r1}");

        // A stroke width that is not a number is refused with the glyph's name.
        let (font, g) = glyph_with(|m| {
            vec![colour_layer(m, "c", vec![path([0.0, 0.0, 10.0, 10.0], |a| a.stroke_width = Some(Dimension::Expression("w * 2".into())))])]
        });
        assert!(matches!(paint_of(&font, &g), Err(ColorError::BadPaint { glyph, .. }) if glyph == "A"));
    }

    #[test]
    fn solid_stacks_fit_version_0_and_a_full_colour_master_layer_paints_first() {
        let opaque = Paint::Layers(vec![
            solid("a", vec![0], None, ColorRef::Rgba([0.1, 0.2, 0.3, 0.4])),
            solid("b", vec![0], None, ColorRef::Palette { index: 2, alpha: 1.0 }),
            solid("b", vec![0], None, ColorRef::Foreground { alpha: 1.0 }),
        ]);
        assert!(is_solid_stack(&opaque), "a colour's own alpha goes into CPAL");
        let translucent = Paint::Layers(vec![solid("b", vec![0], None, ColorRef::Palette { index: 2, alpha: 0.5 })]);
        assert!(!is_solid_stack(&translucent), "a palette reference's alpha needs version 1");

        let (font, mut g) = glyph_with(|m| {
            let mut palette = Layer::new(m, 600.0);
            palette.id = "p".into();
            palette.info.attrs.color_palette = Some(PaletteEntry::Index(0));
            palette.shapes.push(path([0.0, 0.0, 10.0, 10.0], |_| {}));
            vec![palette]
        });
        g.layers[0].info.attrs.full_color = true;
        g.layers[0].shapes.push(path([0.0, 0.0, 20.0, 20.0], |a| a.fill_color = Some(Color::Palette { index: 1, alpha: 1.0 })));
        let master = font.masters[0].id.clone();
        let paint = paint_of(&font, &g).unwrap().unwrap();
        assert_eq!(
            paint,
            Paint::Layers(vec![
                solid(&master, vec![0], None, ColorRef::Palette { index: 1, alpha: 1.0 }),
                solid("p", vec![0], None, ColorRef::Palette { index: 0, alpha: 1.0 }),
            ])
        );
        assert!(is_solid_stack(&paint));
    }
}
