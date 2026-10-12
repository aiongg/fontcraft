//! Path attribute commands of full-colour layers: stroke, fill, gradient and the image-only effects
//! of the given paths (`docs/spec/15-color-fonts.md` § 6). Refused on other layers.
//!
//! Attributes are per path (`color.fullColor.attributesPerPath`). Every command takes the layer
//! and the paths as a target object (`glyph`, `layer?`, `side?`, `shapes?`); without `shapes` it
//! acts on every path of the layer. A command that a single listed path refuses changes nothing.
//!
//! Gradient geometry is stored as the file format stores it, as fractions of the path's bounds
//! (0,0 bottom left, 1,1 top right); `attributes.setGradientPosition` takes font units and
//! converts them. A gradient replaces the fill colour without clearing it, so removing the
//! gradient brings the fill back (`color.fullColor.gradientReplacesFill`). Fully transparent
//! colours are how a removed fill or stroke is stored (`color.fullColor.transparentSwatch`).
//! Shadows and the pattern image are stored and never exported
//! (`color.fullColor.imageOnlyEffects`).

use std::collections::HashSet;

use fontcraft_color::Rgba;
use fontcraft_geom::{Point, Shape as _};
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Color, ColorStop, Dimension, Extend, Family, Gradient, GradientKind, Layer, MAX_GRADIENT_STOPS, Path, Shadow, ShapeAttrs};
use fontcraft_pathops::{MAX_STROKE_WIDTH, position_from_stored, position_to_stored};
use serde_json::{Map, Value, json};

use super::stroke::format_version;
use super::*;
use crate::params::{MAX_TARGETS, Params, TargetKind, Targets, kind};

/// The fill a new path on a full-colour layer gets: 50 % gray, opaque
/// (`color.fullColor.defaultGrayFill`).
pub const DEFAULT_FILL: Color = Color::Rgba([0.5, 0.5, 0.5, 1.0]);
/// The colour a stroke gets when a path without a stroke colour is given a width.
pub const DEFAULT_STROKE: Color = Color::Rgba([0.0, 0.0, 0.0, 1.0]);
/// What a removed fill or stroke is set to.
pub const TRANSPARENT: Color = Color::Rgba([0.0, 0.0, 0.0, 0.0]);
/// A shadow or inner shadow added without parameters.
const DEFAULT_SHADOW: Shadow = Shadow { blur: 10.0, color: Color::Rgba([0.0, 0.0, 0.0, 0.5]), offset: [10.0, -10.0] };
/// Largest shadow blur and offset, in font units.
const MAX_EFFECT: f64 = 10_000.0;
/// Largest gradient coordinate as a fraction of the path's bounds.
const MAX_RELATIVE: f64 = 1_000.0;
/// The native key a pattern image's file reference is kept under (the model has no field).
pub const PATTERN_IMAGE_KEY: &str = "patternImage";
/// The `.glyphs` keys the readers keep untyped until they fill the typed fields; a command that
/// writes the typed field drops them so a file never holds both.
const GLYPHS_GRADIENT_KEY: &str = "gradient";
const GLYPHS_SHADOW_KEY: &str = "shadow";
const GLYPHS_INNER_SHADOW_KEY: &str = "shadowIn";

pub fn specs() -> Vec<CommandSpec> {
    let specs = vec![
        cmd!(
            "attributes.setStrokeWidth",
            "Set Stroke Width",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?: \"foreground\" | \"background\", shapes?: [index] (paths; `paths` is accepted too; default: every path of the layer), width: number | null (font units, 0 to 10000; null is 0, no visible stroke)} set the stroke width of the paths. A path without a stroke colour gets an opaque black one. Refused on a layer that is not full-colour and on a locked glyph → {glyph, layer, shapes: [index], attributes: [{shape, fill, fillTransparent, stroke: {width, alignment, color, transparent}, gradient, shadow, innerShadow, patternImage}]}",
            has_doc,
            set_stroke_width
        ),
        cmd!(
            "attributes.setStrokeAlignment",
            "Set Stroke Alignment",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), alignment: \"center\" | \"inside\" | \"outside\"} place the stroke on the path, inside it (left of the path direction for a counter-clockwise path) or outside it → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            set_stroke_alignment
        ),
        cmd!(
            "attributes.setStrokeColor",
            "Set Stroke Color",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), color: \"#RRGGBB\" | \"#RRGGBBAA\"} set the stroke colour of the paths → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            set_stroke_color
        ),
        cmd!(
            "attributes.removeStroke",
            "Remove Stroke",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path)} set the stroke width of the paths to 0 and the stroke colour to fully transparent → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            remove_stroke
        ),
        cmd!(
            "attributes.setFillColor",
            "Set Fill Color",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), color: \"#RRGGBB\" | \"#RRGGBBAA\"} set the solid fill of the paths. Refused when a path has a gradient, which replaces the fill → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            set_fill_color
        ),
        cmd!(
            "attributes.removeFill",
            "Remove Fill",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path)} make the fill of the paths fully transparent. Refused when a path has a gradient → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            remove_fill
        ),
        cmd!(
            "attributes.addGradient",
            "Add Gradient",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), type?: \"linear\" | \"radial\" | \"conic\" (default linear), stops?: [{offset: 0..1, color: \"#RRGGBBAA\"}] (2 to 256; default: the path's fill colour at 0 and the same colour fully transparent at 1), extend?: \"pad\" | \"repeat\" | \"reflect\" (default pad)} give the paths a gradient, which replaces their fill; a gradient they had is replaced. A new linear gradient runs from the left middle to the right middle of the path's bounds; a radial one is centred, from radius 0 to half the width; a conic one is centred and turns 360° → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`; gradient positions are fractions of the path's bounds)",
            has_doc,
            add_gradient
        ),
        cmd!(
            "attributes.removeGradient",
            "Remove Gradient",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path)} remove the gradient of the paths, which shows their fill colour again. Refused when none of the paths has a gradient → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            remove_gradient
        ),
        cmd!(
            "attributes.setGradientType",
            "Set Gradient Type",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), type?: \"linear\" | \"radial\" | \"conic\", extend?: \"pad\" | \"repeat\" | \"reflect\"} change the gradient of the paths; at least one of `type` and `extend`. A new type places the gradient as `attributes.addGradient` does and keeps the stops. Refused when a path has no gradient → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            set_gradient_type
        ),
        cmd!(
            "attributes.addGradientStop",
            "Add Gradient Stop",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), offset: 0..1, color?: \"#RRGGBB\" | \"#RRGGBBAA\" (default: the gradient's colour at `offset`)} insert a colour stop into the gradient of each path, after the stops at the same offset. Refused when a path has no gradient or already has 256 stops → {glyph, layer, shapes, attributes, stops: [index] (the new stop's index, path by path)}",
            has_doc,
            add_gradient_stop
        ),
        cmd!(
            "attributes.removeGradientStop",
            "Remove Gradient Stop",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), stop: index} delete a colour stop from the gradient of each path. Refused when a path has no gradient, no such stop, or only two stops → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            remove_gradient_stop
        ),
        cmd!(
            "attributes.setGradientStopColor",
            "Set Gradient Stop Color",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), stop: index, color?: \"#RRGGBB\" | \"#RRGGBBAA\", offset?: 0..1} recolour or move one colour stop of the gradient of each path; at least one of `color` and `offset`. A moved stop is placed after the stops at its new offset. Refused when a path has no gradient or no such stop → {glyph, layer, shapes, attributes, stops: [index] (the stop's index afterwards, path by path)}",
            has_doc,
            set_gradient_stop_color
        ),
        cmd!(
            "attributes.setGradientPosition",
            "Set Gradient Position",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), start: [x, y], end?: [x, y], relative?: bool (default false: font units; true: fractions of the path's bounds, within ±1000)} place the gradient of each path: the start and end of a linear gradient, the centre of a radial gradient's start circle and of its end circle, the centre of a conic gradient (which takes no `end`). Without `end` the end moves with the start. Font units are converted to fractions of each path's bounds, so a path without width or height needs `relative`. Refused when a path has no gradient → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            set_gradient_position
        ),
        cmd!(
            "attributes.addAttribute",
            "Add Attribute",
            [],
            None,
            "{glyph: name, layer: id (a full-colour layer), side?, shapes?: [index] (default: every path), attribute: \"gradient\" | \"shadow\" | \"innerShadow\" | \"patternImage\", …} add a style to the paths. `gradient` takes the parameters of `attributes.addGradient`. `shadow` and `innerShadow` take blur?: number (font units, 0 to 10000), offset?: [x, y] (font units, within ±10000), color?: \"#RRGGBBAA\"; a new one is black at 50 % opacity with blur 10 and offset [10, −10], and given keys change an existing one. `patternImage` takes image?: path (a file reference). Shadows and pattern images are drawn by image export only and never written to a font → {glyph, layer, shapes, attributes} (as `attributes.setStrokeWidth`)",
            has_doc,
            add_attribute
        ),
    ];
    specs.into_iter().map(|c| c.targets(&[TargetKind::Layer, TargetKind::Shapes])).collect()
}

// ── Helpers shared with other commands ────────────────────────────────────────────────────────────

/// A path about to be added to `layer`: on a full-colour layer, a path with neither a fill colour
/// nor a gradient gets the gray fill (`color.fullColor.defaultGrayFill`).
pub fn style_new_path(layer: &Layer, mut path: Path) -> Path {
    if layer.info.attrs.full_color && path.info.attrs.fill_color.is_none() && !has_gradient(&path.info.attrs) {
        path.info.attrs.fill_color = Some(DEFAULT_FILL);
    }
    path
}

/// Whether a colour is fully transparent, which the swatches show crossed out.
pub fn is_transparent(color: &Color) -> bool {
    match color {
        Color::Rgba([_, _, _, a]) => *a <= 0.0,
        Color::Palette { alpha, .. } => *alpha <= 0.0,
    }
}

/// Whether the path has a gradient, typed or still kept as the `.glyphs` value it was read from.
pub fn has_gradient(attrs: &ShapeAttrs) -> bool {
    attrs.gradient.is_some() || attrs.extra.get(Family::Glyphs, GLYPHS_GRADIENT_KEY).is_some()
}

/// A colour as the commands report it: `#RRGGBBAA`, or `{palette, alpha}` for a palette entry.
pub fn color_json(color: &Color) -> Value {
    match color {
        Color::Rgba(c) => {
            let [r, g, b, a] = c.map(|v| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 });
            json!(Rgba::new(r, g, b, a).hex())
        }
        Color::Palette { index, alpha } => json!({"palette": index, "alpha": alpha}),
    }
}

/// The attributes of one path as the commands report them.
pub fn describe(index: usize, attrs: &ShapeAttrs, glyphs_version: u32) -> Value {
    let dimension = |d: &Option<Dimension>| match d {
        None => Value::Null,
        Some(Dimension::Number(n)) => json!(n),
        Some(Dimension::Expression(e)) => json!(e),
    };
    let alignment = match &attrs.stroke_pos {
        None => json!("center"),
        Some(Dimension::Number(n)) => json!(alignment_name(position_from_stored(*n, glyphs_version))),
        Some(Dimension::Expression(e)) => json!(e),
    };
    let shadow = |s: &Option<Shadow>| match s {
        None => Value::Null,
        Some(s) => json!({"blur": s.blur, "offset": s.offset, "color": color_json(&s.color)}),
    };
    let pattern = attrs.extra.get(Family::Native, PATTERN_IMAGE_KEY).map(|v| json!(v.as_str().unwrap_or_default()));
    json!({
        "shape": index,
        "fill": attrs.fill_color.as_ref().map(color_json),
        "fillTransparent": attrs.fill_color.as_ref().is_some_and(is_transparent),
        "stroke": {
            "width": dimension(&attrs.stroke_width),
            "alignment": alignment,
            "color": attrs.stroke_color.as_ref().map(color_json),
            "transparent": attrs.stroke_color.as_ref().is_some_and(is_transparent),
        },
        "gradient": attrs.gradient.as_ref().map(gradient_json),
        "shadow": shadow(&attrs.shadow),
        "innerShadow": shadow(&attrs.shadow_in),
        "patternImage": pattern,
    })
}

fn alignment_name(position: f64) -> &'static str {
    if position < -0.5 {
        "inside"
    } else if position > 0.5 {
        "outside"
    } else {
        "center"
    }
}

fn gradient_json(g: &Gradient) -> Value {
    let point = |p: Point| json!([p.x, p.y]);
    let mut m = Map::new();
    match g.kind {
        GradientKind::Linear { start, end } => {
            m.insert("type".into(), json!("linear"));
            m.insert("start".into(), point(start));
            m.insert("end".into(), point(end));
        }
        GradientKind::Radial { start, end, start_radius, end_radius, angle } => {
            m.insert("type".into(), json!("radial"));
            m.insert("start".into(), point(start));
            m.insert("end".into(), point(end));
            m.insert("startRadius".into(), json!(start_radius));
            m.insert("endRadius".into(), json!(end_radius));
            m.insert("angle".into(), json!(angle));
        }
        GradientKind::Conic { center, start_angle, angle, control_radius } => {
            m.insert("type".into(), json!("conic"));
            m.insert("center".into(), point(center));
            m.insert("startAngle".into(), json!(start_angle));
            m.insert("angle".into(), json!(angle));
            m.insert("controlRadius".into(), json!(control_radius));
        }
    }
    let stops: Vec<Value> = g.stops.iter().map(|s| json!({"offset": s.offset, "color": color_json(&s.color)})).collect();
    m.insert("stops".into(), Value::Array(stops));
    m.insert("extend".into(), json!(extend_name(g.extend)));
    Value::Object(m)
}

fn extend_name(e: Extend) -> &'static str {
    match e {
        Extend::Pad => "pad",
        Extend::Repeat => "repeat",
        Extend::Reflect => "reflect",
    }
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// The layer and paths a command acts on: a full-colour layer and the listed paths, or every
/// path of the layer when none is listed.
fn targets(p: &Params, s: &Session) -> Result<Targets> {
    let mut t = Targets::parse(p)?;
    if !(t.points.is_empty() && t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `shapes` only"));
    }
    if let Some(list) = p.opt_array("paths")? {
        if !t.shapes.is_empty() {
            return Err(p.bad("give `shapes` or `paths`, not both"));
        }
        if list.len() > MAX_TARGETS {
            return Err(p.bad(format!("`paths` holds {} entries; at most {MAX_TARGETS} are allowed", list.len())));
        }
        let mut seen = HashSet::new();
        for v in list {
            let i = v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| p.bad(format!("`paths` must hold whole numbers from 0, found {v}")))?;
            if seen.insert(i) {
                t.shapes.push(i);
            }
        }
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    if !r.layer.info.attrs.full_color {
        return Err(p.bad(format!(
            "layer `{}` of `{}` is not a full-colour layer: path attributes belong to the paths of full-colour layers",
            r.layer.id, t.glyph
        )));
    }
    let shapes: Vec<usize> = if t.shapes.is_empty() {
        let all: Vec<usize> = (0..r.parts.shapes.len()).filter(|i| r.parts.path(*i).is_some()).collect();
        if all.is_empty() {
            return Err(p.bad(format!("layer `{}` of `{}` has no paths", r.layer.id, t.glyph)));
        }
        all
    } else {
        if let Some(i) = t.shapes.iter().find(|i| r.parts.path(**i).is_none()) {
            return Err(p.bad(format!("shape {i} is not a path")));
        }
        t.shapes.clone()
    };
    let layer = r.layer.id.clone();
    t.shapes = shapes;
    t.layer = Some(layer);
    Ok(t)
}

/// Runs `f` on every target path as one undo step, then tidies the path's attribute box.
fn edit_paths<T>(s: &mut Session, p: &Params, t: &Targets, label: &str, mut f: impl FnMut(usize, &mut Path) -> Result<T>) -> Result<Vec<T>> {
    s.edit(label, |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::with_capacity(t.shapes.len());
            for &i in &t.shapes {
                let path = parts.path_mut(i).ok_or_else(|| p.bad(format!("shape {i} is not a path")))?;
                out.push(f(i, path)?);
                path.info.tidy();
            }
            Ok(out)
        })
    })
}

/// The command's result: the target and the attributes of every target path afterwards.
fn report(s: &Session, t: &Targets) -> Result<Value> {
    let font = s.font()?;
    let version = format_version(font);
    let r = t.resolve(font)?;
    let attributes: Vec<Value> = t.shapes.iter().filter_map(|&i| r.parts.path(i).map(|path| describe(i, &path.info.attrs, version))).collect();
    Ok(json!({"glyph": t.glyph, "layer": t.layer, "shapes": t.shapes, "attributes": attributes}))
}

fn report_with(s: &Session, t: &Targets, key: &str, value: Value) -> Result<Value> {
    let mut out = report(s, t)?;
    if let Value::Object(m) = &mut out {
        m.insert(key.into(), value);
    }
    Ok(out)
}

/// `#RRGGBB` or `#RRGGBBAA` as an RGBA colour.
fn parse_color(p: &Params, key: &str, v: &Value) -> Result<Color> {
    let text = v.as_str().ok_or_else(|| p.bad(format!("`{key}` must be a colour \"#RRGGBB\" or \"#RRGGBBAA\", found {}", kind(v))))?;
    let rgba = Rgba::parse_hex(text).ok_or_else(|| p.bad(format!("`{key}` must be a colour \"#RRGGBB\" or \"#RRGGBBAA\", got \"{text}\"")))?;
    Ok(Color::Rgba(rgba.to_f64()))
}

fn opt_color(p: &Params, key: &str) -> Result<Option<Color>> {
    p.get(key).map(|v| parse_color(p, key, v)).transpose()
}

fn color(p: &Params, key: &str) -> Result<Color> {
    opt_color(p, key)?.ok_or_else(|| p.bad(format!("`{key}` is required: \"#RRGGBB\" or \"#RRGGBBAA\"")))
}

fn check_offset(p: &Params, key: &str, offset: f64) -> Result<f64> {
    if (0.0..=1.0).contains(&offset) { Ok(offset) } else { Err(p.bad(format!("`{key}` must be from 0 to 1, got {offset}"))) }
}

fn opt_offset(p: &Params) -> Result<Option<f64>> {
    p.opt_f64("offset")?.map(|o| check_offset(p, "offset", o)).transpose()
}

fn stop_index(p: &Params) -> Result<usize> {
    let n = p.opt_int("stop", 0, MAX_GRADIENT_STOPS as i64)?.ok_or_else(|| p.bad("`stop` is required: the stop's index"))?;
    usize::try_from(n).map_err(|_| p.bad("`stop` must be a whole number from 0"))
}

/// `[x, y]` with both numbers finite and within ±`limit`.
fn point(p: &Params, key: &str, limit: f64) -> Result<Option<Point>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    let bad = || p.bad(format!("`{key}` must be [x, y]: two finite numbers within ±{limit}"));
    let [x, y] = list.as_slice() else { return Err(bad()) };
    match (x.as_f64(), y.as_f64()) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x.abs() <= limit && y.abs() <= limit => Ok(Some(Point::new(x, y))),
        _ => Err(bad()),
    }
}

fn default_kind(p: &Params, name: &str) -> Result<GradientKind> {
    let center = Point::new(0.5, 0.5);
    match name {
        "linear" => Ok(GradientKind::Linear { start: Point::new(0.0, 0.5), end: Point::new(1.0, 0.5) }),
        "radial" => Ok(GradientKind::Radial { start: center, end: center, start_radius: 0.0, end_radius: 0.5, angle: 0.0 }),
        "conic" => Ok(GradientKind::Conic { center, start_angle: 0.0, angle: 360.0, control_radius: 0.5 }),
        other => Err(p.bad(format!("`type` must be linear, radial or conic, got \"{other}\""))),
    }
}

fn kind_name(k: &GradientKind) -> &'static str {
    match k {
        GradientKind::Linear { .. } => "linear",
        GradientKind::Radial { .. } => "radial",
        GradientKind::Conic { .. } => "conic",
    }
}

fn opt_extend(p: &Params) -> Result<Option<Extend>> {
    match p.opt_str("extend")? {
        None => Ok(None),
        Some("pad") => Ok(Some(Extend::Pad)),
        Some("repeat") => Ok(Some(Extend::Repeat)),
        Some("reflect") => Ok(Some(Extend::Reflect)),
        Some(other) => Err(p.bad(format!("`extend` must be pad, repeat or reflect, got \"{other}\""))),
    }
}

/// Puts `stop` into `stops` after every stop at the same or a lower offset; returns its index.
fn insert_stop(stops: &mut Vec<ColorStop>, stop: ColorStop) -> usize {
    let at = stops.iter().position(|s| s.offset > stop.offset).unwrap_or(stops.len());
    stops.insert(at, stop);
    at
}

/// The colour a gradient paints at `offset`: the stops' colours mixed linearly, the nearer stop's
/// colour when one of the two is a palette entry, the end colour beyond the stops.
fn color_at(stops: &[ColorStop], offset: f64) -> Color {
    let before = stops.iter().filter(|s| s.offset <= offset).max_by(|a, b| a.offset.total_cmp(&b.offset));
    let after = stops.iter().filter(|s| s.offset >= offset).min_by(|a, b| a.offset.total_cmp(&b.offset));
    match (before, after) {
        (Some(a), Some(b)) => match (a.color, b.color) {
            (Color::Rgba(x), Color::Rgba(y)) => {
                let span = b.offset - a.offset;
                let f = if span > 0.0 { ((offset - a.offset) / span).clamp(0.0, 1.0) } else { 0.0 };
                let mut mixed = x;
                for (m, (u, v)) in mixed.iter_mut().zip(x.iter().zip(y.iter())) {
                    *m = u + (v - u) * f;
                }
                Color::Rgba(mixed)
            }
            _ if offset - a.offset <= b.offset - offset => a.color,
            _ => b.color,
        },
        (Some(a), None) => a.color,
        (None, Some(b)) => b.color,
        (None, None) => DEFAULT_FILL,
    }
}

fn transparent_version(c: Color) -> Color {
    match c {
        Color::Rgba([r, g, b, _]) => Color::Rgba([r, g, b, 0.0]),
        Color::Palette { index, .. } => Color::Palette { index, alpha: 0.0 },
    }
}

fn gradient_mut<'a>(p: &Params, i: usize, attrs: &'a mut ShapeAttrs) -> Result<&'a mut Gradient> {
    attrs.gradient.as_mut().ok_or_else(|| p.bad(format!("path {i} has no gradient")))
}

fn refuse_gradient(p: &Params, i: usize, attrs: &ShapeAttrs) -> Result<()> {
    if has_gradient(attrs) {
        return Err(p.bad(format!("path {i} has a gradient, which replaces the fill: remove the gradient first")));
    }
    Ok(())
}

// ── Stroke ──────────────────────────────────────────────────────────────────────────────────────

fn set_stroke_width(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setStrokeWidth", value)?;
    let width = match (value.get("width"), p.opt_f64("width")?) {
        (None, _) => return Err(p.bad("`width` is required: font units, or null for 0")),
        (Some(_), None) => 0.0,
        (Some(_), Some(w)) if (0.0..=MAX_STROKE_WIDTH).contains(&w) => w,
        (Some(_), Some(w)) => return Err(p.bad(format!("`width` must be from 0 to {MAX_STROKE_WIDTH}, got {w}"))),
    };
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Set Stroke Width", |_, path| {
        let a = &mut path.info.attrs;
        a.stroke_width = Some(Dimension::Number(width));
        if width > 0.0 && a.stroke_color.is_none() {
            a.stroke_color = Some(DEFAULT_STROKE);
        }
        Ok(())
    })?;
    report(s, &t)
}

fn set_stroke_alignment(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setStrokeAlignment", value)?;
    let position = match p.str("alignment")? {
        "center" => None,
        "inside" => Some(-1.0),
        "outside" => Some(1.0),
        other => return Err(p.bad(format!("`alignment` must be center, inside or outside, got \"{other}\""))),
    };
    let t = targets(&p, s)?;
    let version = format_version(s.font()?);
    edit_paths(s, &p, &t, "Set Stroke Alignment", |_, path| {
        path.info.attrs.stroke_pos = position.map(|pos| Dimension::Number(position_to_stored(pos, version)));
        Ok(())
    })?;
    report(s, &t)
}

fn set_stroke_color(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setStrokeColor", value)?;
    let c = color(&p, "color")?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Set Stroke Color", |_, path| {
        path.info.attrs.stroke_color = Some(c);
        Ok(())
    })?;
    report(s, &t)
}

fn remove_stroke(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.removeStroke", value)?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Remove Stroke", |_, path| {
        path.info.attrs.stroke_width = Some(Dimension::Number(0.0));
        path.info.attrs.stroke_color = Some(TRANSPARENT);
        Ok(())
    })?;
    report(s, &t)
}

// ── Fill ────────────────────────────────────────────────────────────────────────────────────────

fn set_fill_color(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setFillColor", value)?;
    let c = color(&p, "color")?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Set Fill Color", |i, path| {
        refuse_gradient(&p, i, &path.info.attrs)?;
        path.info.attrs.fill_color = Some(c);
        Ok(())
    })?;
    report(s, &t)
}

fn remove_fill(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.removeFill", value)?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Remove Fill", |i, path| {
        refuse_gradient(&p, i, &path.info.attrs)?;
        path.info.attrs.fill_color = Some(TRANSPARENT);
        Ok(())
    })?;
    report(s, &t)
}

// ── Gradient ────────────────────────────────────────────────────────────────────────────────────

/// What `attributes.addGradient` (and `attributes.addAttribute` with `gradient`) writes.
struct NewGradient {
    kind: GradientKind,
    stops: Option<Vec<ColorStop>>,
    extend: Extend,
}

impl NewGradient {
    fn read(p: &Params) -> Result<Self> {
        let gradient_kind = default_kind(p, p.opt_str("type")?.unwrap_or("linear"))?;
        let stops = match p.opt_array("stops")? {
            None => None,
            Some(list) => {
                if list.len() < 2 {
                    return Err(p.bad(format!("a gradient needs at least two stops, got {}", list.len())));
                }
                if list.len() > MAX_GRADIENT_STOPS {
                    return Err(p.bad(format!("`stops` holds {} stops; at most {MAX_GRADIENT_STOPS} are allowed", list.len())));
                }
                let mut stops = list
                    .iter()
                    .enumerate()
                    .map(|(n, v)| {
                        let Value::Object(o) = v else {
                            return Err(p.bad(format!("each stop must be an object {{offset, color}}, found {}", kind(v))));
                        };
                        let key = format!("stops[{n}].offset");
                        let offset = o
                            .get("offset")
                            .and_then(Value::as_f64)
                            .filter(|f| f.is_finite())
                            .ok_or_else(|| p.bad(format!("`{key}` must be a number from 0 to 1")))?;
                        let offset = check_offset(p, &key, offset)?;
                        let key = format!("stops[{n}].color");
                        let color = parse_color(p, &key, o.get("color").ok_or_else(|| p.bad(format!("`{key}` is required")))?)?;
                        Ok(ColorStop { offset, color })
                    })
                    .collect::<Result<Vec<_>>>()?;
                stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
                Some(stops)
            }
        };
        Ok(Self { kind: gradient_kind, stops, extend: opt_extend(p)?.unwrap_or_default() })
    }

    fn apply(&self, attrs: &mut ShapeAttrs) {
        let stops = self.stops.clone().unwrap_or_else(|| {
            let fill = attrs.fill_color.unwrap_or(DEFAULT_FILL);
            vec![ColorStop { offset: 0.0, color: fill }, ColorStop { offset: 1.0, color: transparent_version(fill) }]
        });
        attrs.gradient = Some(Gradient { kind: self.kind, stops, extend: self.extend });
        attrs.extra.remove(Family::Glyphs, GLYPHS_GRADIENT_KEY);
    }
}

fn add_gradient(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.addGradient", value)?;
    let new = NewGradient::read(&p)?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Add Gradient", |_, path| {
        new.apply(&mut path.info.attrs);
        Ok(())
    })?;
    report(s, &t)
}

fn remove_gradient(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.removeGradient", value)?;
    let t = targets(&p, s)?;
    {
        let r = t.resolve(s.font()?)?;
        if !t.shapes.iter().any(|&i| r.parts.path(i).is_some_and(|path| has_gradient(&path.info.attrs))) {
            return Err(p.bad("none of the paths has a gradient"));
        }
    }
    edit_paths(s, &p, &t, "Remove Gradient", |_, path| {
        path.info.attrs.gradient = None;
        path.info.attrs.extra.remove(Family::Glyphs, GLYPHS_GRADIENT_KEY);
        Ok(())
    })?;
    report(s, &t)
}

fn set_gradient_type(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setGradientType", value)?;
    let new_kind = p.opt_str("type")?.map(|name| default_kind(&p, name)).transpose()?;
    let extend = opt_extend(&p)?;
    if new_kind.is_none() && extend.is_none() {
        return Err(p.bad("give `type`, `extend` or both"));
    }
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Set Gradient Type", |i, path| {
        let g = gradient_mut(&p, i, &mut path.info.attrs)?;
        if let Some(k) = new_kind.filter(|k| kind_name(k) != kind_name(&g.kind)) {
            g.kind = k;
        }
        if let Some(e) = extend {
            g.extend = e;
        }
        Ok(())
    })?;
    report(s, &t)
}

fn add_gradient_stop(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.addGradientStop", value)?;
    let offset = opt_offset(&p)?.ok_or_else(|| p.bad("`offset` is required: from 0 to 1"))?;
    let c = opt_color(&p, "color")?;
    let t = targets(&p, s)?;
    let indices = edit_paths(s, &p, &t, "Add Gradient Stop", |i, path| {
        let g = gradient_mut(&p, i, &mut path.info.attrs)?;
        if g.stops.len() >= MAX_GRADIENT_STOPS {
            return Err(p.bad(format!("the gradient of path {i} already has {MAX_GRADIENT_STOPS} stops")));
        }
        let color = c.unwrap_or_else(|| color_at(&g.stops, offset));
        Ok(insert_stop(&mut g.stops, ColorStop { offset, color }))
    })?;
    report_with(s, &t, "stops", json!(indices))
}

fn remove_gradient_stop(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.removeGradientStop", value)?;
    let stop = stop_index(&p)?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Remove Gradient Stop", |i, path| {
        let g = gradient_mut(&p, i, &mut path.info.attrs)?;
        if stop >= g.stops.len() {
            return Err(p.bad(format!("the gradient of path {i} has no stop {stop}: it has {}", g.stops.len())));
        }
        if g.stops.len() <= 2 {
            return Err(p.bad(format!("the gradient of path {i} has two stops, the fewest a gradient can have")));
        }
        g.stops.remove(stop);
        Ok(())
    })?;
    report(s, &t)
}

fn set_gradient_stop_color(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setGradientStopColor", value)?;
    let stop = stop_index(&p)?;
    let c = opt_color(&p, "color")?;
    let offset = opt_offset(&p)?;
    if c.is_none() && offset.is_none() {
        return Err(p.bad("give `color`, `offset` or both"));
    }
    let t = targets(&p, s)?;
    let indices = edit_paths(s, &p, &t, "Set Gradient Stop Color", |i, path| {
        let g = gradient_mut(&p, i, &mut path.info.attrs)?;
        if stop >= g.stops.len() {
            return Err(p.bad(format!("the gradient of path {i} has no stop {stop}: it has {}", g.stops.len())));
        }
        let mut edited = g.stops.remove(stop);
        if let Some(c) = c {
            edited.color = c;
        }
        match offset {
            Some(o) => {
                edited.offset = o;
                Ok(insert_stop(&mut g.stops, edited))
            }
            None => {
                g.stops.insert(stop, edited);
                Ok(stop)
            }
        }
    })?;
    report_with(s, &t, "stops", json!(indices))
}

fn set_gradient_position(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.setGradientPosition", value)?;
    let relative = p.bool_or("relative", false)?;
    let limit = if relative { MAX_RELATIVE } else { fontcraft_model::MAX_COORDINATE };
    let start = point(&p, "start", limit)?.ok_or_else(|| p.bad("`start` is required: [x, y]"))?;
    let end = point(&p, "end", limit)?;
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Set Gradient Position", |i, path| {
        let to_fraction = if relative {
            None
        } else {
            let bounds = path.to_bez().map_err(|why| p.bad(format!("path {i}: {why}")))?.bounding_box();
            if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
                return Err(
                    p.bad(format!("path {i} has no width or no height, so a position in font units cannot be placed on it: give `relative: true`"))
                );
            }
            Some(bounds)
        };
        let place = |pt: Point| to_fraction.map_or(pt, |b| Point::new((pt.x - b.x0) / b.width(), (pt.y - b.y0) / b.height()));
        let (start, end) = (place(start), end.map(place));
        let g = gradient_mut(&p, i, &mut path.info.attrs)?;
        match &mut g.kind {
            GradientKind::Linear { start: s0, end: e0 } | GradientKind::Radial { start: s0, end: e0, .. } => {
                let moved = start - *s0;
                *e0 = end.unwrap_or(*e0 + moved);
                *s0 = start;
            }
            GradientKind::Conic { center, .. } => {
                if end.is_some() {
                    return Err(p.bad("a conic gradient has one centre: give `start` only"));
                }
                *center = start;
            }
        }
        if !g.numbers().all(|n| n.is_finite() && n.abs() <= MAX_RELATIVE) {
            return Err(p.bad(format!("the gradient of path {i} would lie more than {MAX_RELATIVE} times its size away from the path")));
        }
        Ok(())
    })?;
    report(s, &t)
}

// ── Add Attribute ───────────────────────────────────────────────────────────────────────────────

/// The keys of a shadow that `attributes.addAttribute` sets.
struct ShadowEdit {
    blur: Option<f64>,
    offset: Option<Point>,
    color: Option<Color>,
}

impl ShadowEdit {
    fn read(p: &Params) -> Result<Self> {
        let blur = match p.opt_f64("blur")? {
            Some(b) if !(0.0..=MAX_EFFECT).contains(&b) => return Err(p.bad(format!("`blur` must be from 0 to {MAX_EFFECT}, got {b}"))),
            other => other,
        };
        Ok(Self { blur, offset: point(p, "offset", MAX_EFFECT)?, color: opt_color(p, "color")? })
    }

    fn apply(&self, old: Option<Shadow>) -> Shadow {
        let mut s = old.unwrap_or(DEFAULT_SHADOW);
        if let Some(b) = self.blur {
            s.blur = b;
        }
        if let Some(o) = self.offset {
            s.offset = [o.x, o.y];
        }
        if let Some(c) = self.color {
            s.color = c;
        }
        s
    }
}

fn add_attribute(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("attributes.addAttribute", value)?;
    let attribute = p.str("attribute")?;
    enum Edit {
        Gradient(NewGradient),
        Shadow(ShadowEdit),
        InnerShadow(ShadowEdit),
        Pattern(String),
    }
    let edit = match attribute {
        "gradient" => Edit::Gradient(NewGradient::read(&p)?),
        "shadow" => Edit::Shadow(ShadowEdit::read(&p)?),
        "innerShadow" => Edit::InnerShadow(ShadowEdit::read(&p)?),
        "patternImage" => Edit::Pattern(p.opt_str("image")?.unwrap_or_default().to_string()),
        other => return Err(p.bad(format!("`attribute` must be gradient, shadow, innerShadow or patternImage, got \"{other}\""))),
    };
    let t = targets(&p, s)?;
    edit_paths(s, &p, &t, "Add Attribute", |_, path| {
        let a = &mut path.info.attrs;
        match &edit {
            Edit::Gradient(g) => g.apply(a),
            Edit::Shadow(e) => {
                a.shadow = Some(e.apply(a.shadow));
                a.extra.remove(Family::Glyphs, GLYPHS_SHADOW_KEY);
            }
            Edit::InnerShadow(e) => {
                a.shadow_in = Some(e.apply(a.shadow_in));
                a.extra.remove(Family::Glyphs, GLYPHS_INNER_SHADOW_KEY);
            }
            Edit::Pattern(image) => a.extra.insert(Family::Native, PATTERN_IMAGE_KEY, Plist::String(Str::from(image.as_str()))),
        }
        Ok(())
    })?;
    report(s, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_path_on_a_full_colour_layer_gets_the_gray_fill() {
        let mut layer = Layer::new("m01", 500.0);
        let path = Path::rect(0.0, 0.0, 100.0, 100.0);
        assert_eq!(style_new_path(&layer, path.clone()).info.attrs.fill_color, None, "a master layer leaves paths alone");
        layer.id = "color".into();
        layer.info.attrs.full_color = true;
        assert_eq!(style_new_path(&layer, path.clone()).info.attrs.fill_color, Some(DEFAULT_FILL));
        let mut red = path;
        red.info.attrs.fill_color = Some(Color::Rgba([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(style_new_path(&layer, red).info.attrs.fill_color, Some(Color::Rgba([1.0, 0.0, 0.0, 1.0])), "a path with a fill keeps it");
    }

    #[test]
    fn the_colour_between_two_stops_is_mixed() {
        let black = ColorStop { offset: 0.0, color: Color::Rgba([0.0, 0.0, 0.0, 1.0]) };
        let stops = [black, ColorStop { offset: 1.0, color: Color::Rgba([1.0, 1.0, 1.0, 0.0]) }];
        assert_eq!(color_at(&stops, 0.25), Color::Rgba([0.25, 0.25, 0.25, 0.75]));
        let palette = [black, ColorStop { offset: 1.0, color: Color::Palette { index: 2, alpha: 1.0 } }];
        assert_eq!(color_at(&palette, 0.75), Color::Palette { index: 2, alpha: 1.0 });
    }
}
