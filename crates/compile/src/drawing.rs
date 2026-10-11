//! Strokes, masks and shape order as the compiler reads them (`docs/spec/03a-edit-view-drawing.md`
//! §§ 17–18, `compositing.shapesCombineInOrder`): a layer's outline is its shapes taken in order,
//! each stroked path expanded to its stroke outline and each mask applied to the shapes before it
//! ([`fontcraft_pathops::composite`]). The editor's canvas draws the same outline.
//!
//! A path's stroke and mask are drawing attributes ([`ShapeAttrs`]) stored as the source's format
//! version states them; [`stroke_style`] and [`mask_mode`] read them. The axis a forced
//! axis-aligned cap uses has no model field and no known key in the reference format: it is kept
//! as a native extra entry ([`CAP_AXIS_KEY`]).
//!
//! [`resolve`] is [`Font::resolve`] with that composite in place of the shapes of every layer
//! that has a stroke or a mask; the compiler's later stages (decomposition, overlap removal) see
//! only contours and components. It also gives the glyphs of a master with linked metrics the
//! advances of the master it follows ([`metrics_layer`]).

use std::collections::HashMap;

use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{
    Component, Dimension, Family, Font, Glyph, Layer, LayerSide, MAX_COMPONENT_DEPTH, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, ModelError, Resolved,
    Shape, ShapeAttrs, Target,
};
use fontcraft_pathops::{Cap, CapAxis, Join, LayerShape, MaskMode, StrokeStyle, composite, position_from_stored};

/// The native extra key that holds the axis a path's axis-aligned caps are forced to (`x` or
/// `y`); without it the cap picks the nearer axis.
pub const CAP_AXIS_KEY: &str = "lineCapAxis";

/// A stroke size or position as a number. A stored expression that is not a plain number cannot
/// be evaluated yet.
fn number(d: &Option<Dimension>, what: &str) -> Result<Option<f64>, String> {
    match d {
        None => Ok(None),
        Some(Dimension::Number(n)) => Ok(Some(*n)),
        Some(Dimension::Expression(e)) => match e.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Some(n)),
            _ => Err(format!("the {what} `{e}` is an expression, which cannot be evaluated yet")),
        },
    }
}

/// The `.glyphs` format version whose conventions the font's stroke positions and mask values
/// follow. A font that did not come from a `.glyphs` file is written as version 3, so it follows
/// version 3.
pub fn format_version(font: &Font) -> u32 {
    font.source.glyphs_version.unwrap_or(3)
}

/// The axis a path's axis-aligned caps are forced to.
pub fn cap_axis(attrs: &ShapeAttrs) -> CapAxis {
    attrs.extra.get(Family::Native, CAP_AXIS_KEY).and_then(Plist::as_str).and_then(CapAxis::from_name).unwrap_or_default()
}

/// The stroke that `attrs` describe in a font of [`format_version`] `glyphs_version`, or `None`
/// when there is no stroke width or it is 0 or less. Unknown join and cap codes read as miter
/// and butt.
pub fn stroke_style(attrs: &ShapeAttrs, glyphs_version: u32) -> Result<Option<StrokeStyle>, String> {
    let Some(width) = number(&attrs.stroke_width, "stroke width")? else { return Ok(None) };
    if width <= 0.0 {
        return Ok(None);
    }
    let height = number(&attrs.stroke_height, "stroke height")?.unwrap_or(width);
    // No stored position is a centred stroke in every format version.
    let position = number(&attrs.stroke_pos, "stroke position")?.map_or(0.0, |v| position_from_stored(v, glyphs_version));
    let cap = |code: Option<u8>| code.and_then(Cap::from_code).unwrap_or_default();
    Ok(Some(StrokeStyle {
        width,
        height,
        position,
        join: attrs.line_join.and_then(Join::from_code).unwrap_or_default(),
        cap_start: cap(attrs.line_cap_start),
        cap_end: cap(attrs.line_cap_end),
        cap_axis: cap_axis(attrs),
        fill: attrs.fill,
    }))
}

/// What a shape with `attrs` does as a mask, if it is one.
pub fn mask_mode(attrs: &ShapeAttrs, glyphs_version: u32) -> Option<MaskMode> {
    attrs.mask.and_then(|code| MaskMode::from_code(code, glyphs_version))
}

/// Does any of `shapes` have a stroke or act as a mask? A stroke that cannot be read counts: the
/// composite reports it.
pub fn has_strokes_or_masks(shapes: &[Shape], glyphs_version: u32) -> bool {
    shapes.iter().any(|s| match s {
        Shape::Path(p) => mask_mode(&p.info.attrs, glyphs_version).is_some() || !matches!(stroke_style(&p.info.attrs, glyphs_version), Ok(None)),
        Shape::Component(c) => mask_mode(&c.info.attrs, glyphs_version).is_some(),
        Shape::Image(_) | Shape::Raw(_) => false,
    })
}

/// The final outline of `layer`'s foreground: its shapes in order, each stroked path expanded to
/// its stroke outline, each mask applied to the shapes before it, components resolved on the
/// layer's master (their own strokes and masks applied). A layer without strokes or masks gives
/// its contours as they are. This is what the canvas draws in black and what export compiles
/// before overlap removal.
pub fn layer_outline(font: &Font, layer: &Layer) -> Result<Vec<BezPath>, String> {
    shapes_outline(font, layer.parts(LayerSide::Foreground).shapes, &layer.master)
}

/// [`layer_outline`] for a list of shapes on `master` (a layer's background, for one).
pub fn shapes_outline(font: &Font, shapes: &[Shape], master: &str) -> Result<Vec<BezPath>, String> {
    let mut composer = Composer { font, master, memo: HashMap::new(), budget: MAX_OUTLINE_ELEMENTS };
    composer.shapes(shapes, 0)
}

/// [`Font::resolve`] with each layer that has a stroke or a mask drawn as [`layer_outline`]: such
/// a glyph becomes contours only. A layer whose composite cannot be made keeps its shapes as they
/// are, with a note naming the glyph.
pub fn resolve(font: &Font, target: &Target) -> Result<Resolved, ModelError> {
    let version = format_version(font);
    let mut notes = Vec::new();
    let mut resolved = font.resolve_drawn(target, &mut |glyph: &Glyph, layer: &Layer| {
        if !has_strokes_or_masks(&layer.shapes, version) {
            return None;
        }
        match layer_outline(font, layer) {
            Ok(outline) => Some(Ok(outline)),
            Err(e) => {
                notes.push(format!("glyph `{}`: its strokes and masks were not applied ({e}); its paths were compiled as they are", glyph.name));
                None
            }
        }
    })?;
    resolved.notes.extend(notes);
    link_metrics(font, &mut resolved);
    Ok(resolved)
}

/// The layer whose width, vertical width and vertical origin a compile of `layer` uses. For a
/// master layer of a master that takes its metrics from another master
/// ([`fontcraft_spacing::metrics_source`]), that is the glyph's layer of the other master when
/// the glyph has one. Every other layer, intermediate layers included, keeps its own (fontc
/// `glyphs2fontir/src/source.rs`).
pub fn metrics_layer<'a>(font: &Font, glyph: &'a Glyph, layer: &'a Layer) -> &'a Layer {
    if !layer.is_master_layer() {
        return layer;
    }
    match fontcraft_spacing::metrics_source(font, &layer.master) {
        Some(source) if source.id != layer.master => glyph.layer(&source.id).unwrap_or(layer),
        _ => layer,
    }
}

/// Gives each resolved glyph the advances of its [`metrics_layer`]. A vertical width or origin
/// the linked layer does not state keeps the glyph's own, as fontc does.
fn link_metrics(font: &Font, resolved: &mut Resolved) {
    if fontcraft_spacing::metrics_source(font, &resolved.master).is_none_or(|s| s.id == resolved.master) {
        return;
    }
    for out in &mut resolved.glyphs {
        let Some(glyph) = font.glyph(&out.name) else { continue };
        let Some(own) = glyph.layer(&resolved.master) else { continue };
        let source = metrics_layer(font, glyph, own);
        out.width = source.width;
        out.vert_width = source.info.vert_width.or(out.vert_width);
        out.vert_origin = source.info.vert_origin.or(out.vert_origin);
    }
}

/// Resolves components once per base glyph and charges every outline it hands out against a
/// budget, so nested components cannot multiply the work.
struct Composer<'f> {
    font: &'f Font,
    master: &'f str,
    memo: HashMap<String, Vec<BezPath>>,
    budget: usize,
}

impl Composer<'_> {
    fn shapes(&mut self, shapes: &[Shape], depth: usize) -> Result<Vec<BezPath>, String> {
        let version = format_version(self.font);
        let mut list = Vec::with_capacity(shapes.len());
        for shape in shapes {
            let (outline, attrs, stroke) = match shape {
                Shape::Path(p) => (vec![p.to_bez()?], &p.info.attrs, stroke_style(&p.info.attrs, version)?),
                Shape::Component(c) => (self.component(c, depth)?, &c.info.attrs, None),
                Shape::Image(_) | Shape::Raw(_) => continue,
            };
            let elements: usize = outline.iter().map(|p| p.elements().len()).sum();
            self.budget = self.budget.checked_sub(elements).ok_or_else(|| format!("the outline has more than {MAX_OUTLINE_ELEMENTS} elements"))?;
            list.push(LayerShape { outline, stroke, mask: mask_mode(attrs, version) });
        }
        composite(&list).map_err(|e| e.to_string())
    }

    fn component(&mut self, c: &Component, depth: usize) -> Result<Vec<BezPath>, String> {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(format!("components nest more than {MAX_COMPONENT_DEPTH} deep (a component refers back to its own glyph?)"));
        }
        let affine = c.affine();
        if !affine.as_coeffs().iter().all(|v| v.is_finite()) {
            return Err("a component transform is not a number".into());
        }
        let base = match self.memo.get(&c.glyph) {
            Some(outline) => outline.clone(),
            None => {
                let font = self.font;
                let glyph = font.glyph(&c.glyph).ok_or_else(|| format!("a component refers to `{}`, which is not in the font", c.glyph))?;
                let shapes = glyph.layer(self.master).map_or(&[][..], |l| l.shapes.as_slice());
                let outline = self.shapes(shapes, depth + 1)?;
                self.memo.insert(c.glyph.clone(), outline.clone());
                outline
            }
        };
        let mut out = Vec::with_capacity(base.len());
        for mut p in base {
            p.apply_affine(affine);
            if !within_limits(&p) {
                return Err(format!("a component of `{}` puts points beyond ±{MAX_COORDINATE}", c.glyph));
            }
            out.push(p);
        }
        Ok(out)
    }
}

fn within_limits(p: &BezPath) -> bool {
    let ok = |q: &Point| q.is_finite() && q.x.abs() <= MAX_COORDINATE && q.y.abs() <= MAX_COORDINATE;
    p.elements().iter().all(|el| match el {
        PathEl::MoveTo(a) | PathEl::LineTo(a) => ok(a),
        PathEl::QuadTo(a, b) => ok(a) && ok(b),
        PathEl::CurveTo(a, b, c) => ok(a) && ok(b) && ok(c),
        PathEl::ClosePath => true,
    })
}
