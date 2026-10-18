//! Cross-axis mappings and Interpolate Fonts. Rows: `font.setCrossAxisMappings`,
//! `font.interpolateFonts` (the window is `window.crossAxisMappings` in the shell).
//!
//! | Data | Where it is kept |
//! |---|---|
//! | cross-axis mappings | the font's user data, [`fc_keys::CROSS_AXIS_MAPPINGS`]: `{description?, mappings: [{description?, input: {tag: number}, output: {tag: number}}]}` |
//!
//! A cross-axis mapping says: where the font's location reaches `input`, the axes in `output`
//! move to the values given there (the second version of the `avar` table). Locations are in
//! design coordinates, by axis tag. `.glyphs`, UFO and designspace keep the key as user data, so
//! the mappings survive every source format. [`avar_mappings`] gives them in the shape the `avar`
//! builder takes.
//!
//! Interpolate Fonts builds a new document between two single-master fonts with the same glyphs,
//! point by point: `amount` 0 gives the first font, 100 the second, values outside extrapolate.
//! It needs no designspace: each number is `a·(1 − t) + b·t`. The new font is a copy of the first
//! one with every glyph's master layer, the master's metrics, stems and zones, and the kerning
//! interpolated.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::plist::{Array, Dict, Num, Number, Str, Value as PValue};
use fontcraft_model::{
    Anchor, Component, Font, Glyph, GlyphTable, Kerning, Layer, MAX_COORDINATE, Master, NodeKind, Path, Shape, Transform, fc_keys,
};
use fontcraft_tables::sfnt::parse_tag;
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, kind};

/// Most cross-axis mappings a font holds (the `avar` builder's own limit).
pub const MAX_CROSS_AXIS_MAPPINGS: usize = fontcraft_tables::variations::MAX_CROSS_AXIS_MAPPINGS;
/// Longest description of a mapping or of the list, in bytes.
pub const MAX_DESCRIPTION_LEN: usize = 4_096;
/// Largest `amount` of Interpolate Fonts either way, in percent.
pub const MAX_INTERPOLATION_AMOUNT: f64 = 100_000.0;

/// One cross-axis mapping: axis tags with design coordinates, in the font's axis order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossAxisMapping {
    pub description: Option<String>,
    pub input: Vec<(String, f64)>,
    pub output: Vec<(String, f64)>,
}

/// A font's cross-axis mappings with the description of the whole list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossAxisMappings {
    pub description: Option<String>,
    pub mappings: Vec<CrossAxisMapping>,
}

impl CrossAxisMappings {
    pub fn is_empty(&self) -> bool {
        self.description.is_none() && self.mappings.is_empty()
    }

    /// The shape `font.setCrossAxisMappings` takes and returns.
    pub fn to_json(&self) -> Value {
        let side = |s: &[(String, f64)]| Value::Object(s.iter().map(|(t, v)| (t.clone(), json!(v))).collect::<Map<String, Value>>());
        let mappings: Vec<Value> = self
            .mappings
            .iter()
            .map(|m| {
                let mut o = Map::new();
                if let Some(d) = &m.description {
                    o.insert("description".into(), json!(d));
                }
                o.insert("input".into(), side(&m.input));
                o.insert("output".into(), side(&m.output));
                Value::Object(o)
            })
            .collect();
        let mut out = Map::new();
        if let Some(d) = &self.description {
            out.insert("description".into(), json!(d));
        }
        out.insert("mappings".into(), Value::Array(mappings));
        Value::Object(out)
    }

    /// The stored form, keys in sorted order so writers that sort keys keep it unchanged.
    fn to_plist(&self) -> PValue {
        let side = |s: &[(String, f64)]| {
            let mut d = Dict::new();
            let mut sorted: Vec<&(String, f64)> = s.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (tag, v) in sorted {
                d.insert(tag.as_str(), number(*v));
            }
            PValue::Dict(d)
        };
        let items = self
            .mappings
            .iter()
            .map(|m| {
                let mut d = Dict::new();
                if let Some(text) = &m.description {
                    d.insert("description", PValue::String(Str::new(text.as_str())));
                }
                d.insert("input", side(&m.input));
                d.insert("output", side(&m.output));
                PValue::Dict(d)
            })
            .collect();
        let mut d = Dict::new();
        if let Some(text) = &self.description {
            d.insert("description", PValue::String(Str::new(text.as_str())));
        }
        d.insert("mappings", PValue::Array(Array { items, ..Default::default() }));
        PValue::Dict(d)
    }
}

/// A coordinate as a plist number: an integer when it is whole, so text formats write `400`.
fn number(v: f64) -> PValue {
    if v.fract() == 0.0 && v.abs() <= MAX_COORDINATE {
        PValue::Number(Number::int(v as i64))
    } else {
        PValue::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
    }
}

/// The font's cross-axis mappings as stored. Malformed entries are skipped, and so are axis tags
/// the font does not have, entries with an empty side and entries past
/// [`MAX_CROSS_AXIS_MAPPINGS`].
pub fn mappings(font: &Font) -> CrossAxisMappings {
    let Some(root) = font.info.user_data.get(fc_keys::CROSS_AXIS_MAPPINGS).and_then(PValue::as_dict) else {
        return CrossAxisMappings::default();
    };
    let description = |d: &Dict| d.get("description").and_then(PValue::as_str).and_then(|s| clean_description(s).ok()).flatten();
    let side = |v: Option<&PValue>| -> Option<Vec<(String, f64)>> {
        let d = v?.as_dict()?;
        let mut out: Vec<(String, f64)> = Vec::new();
        for axis in font.axes.iter() {
            if let Some(value) = d.get(axis.tag.as_str()) {
                let v = value.as_num().map(Num::as_f64).filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)?;
                out.push((axis.tag.clone(), v));
            }
        }
        (!out.is_empty()).then_some(out)
    };
    let mut out = CrossAxisMappings { description: description(root), mappings: Vec::new() };
    let Some(list) = root.get("mappings").and_then(PValue::as_array) else { return out };
    for item in &list.items {
        if out.mappings.len() >= MAX_CROSS_AXIS_MAPPINGS {
            break;
        }
        let Some(d) = item.as_dict() else { continue };
        let (Some(input), Some(output)) = (side(d.get("input")), side(d.get("output"))) else { continue };
        out.mappings.push(CrossAxisMapping { description: description(d), input, output });
    }
    out
}

/// The font's cross-axis mappings as the `avar` version 2 builder takes them. A tag that is not
/// a valid table tag leaves its mapping out.
pub fn avar_mappings(font: &Font) -> Vec<fontcraft_tables::variations::CrossAxisMapping> {
    let side = |s: &[(String, f64)]| s.iter().map(|(t, v)| parse_tag(t).ok().map(|tag| (tag, *v))).collect::<Option<Vec<_>>>();
    mappings(font)
        .mappings
        .iter()
        .filter_map(|m| Some(fontcraft_tables::variations::CrossAxisMapping { input: side(&m.input)?, output: side(&m.output)? }))
        .collect()
}

/// A description with surrounding space trimmed; `None` when empty. Refuses one that is too long
/// or holds control characters other than line breaks and tabs.
fn clean_description(text: &str) -> std::result::Result<Option<String>, String> {
    let text = text.trim();
    if text.len() > MAX_DESCRIPTION_LEN {
        return Err(format!("a description holds at most {MAX_DESCRIPTION_LEN} bytes"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("a description may not hold control characters".into());
    }
    Ok((!text.is_empty()).then(|| text.to_string()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.setCrossAxisMappings",
            "Set Cross-Axis Mappings",
            [],
            None,
            "{mappings: [{input: {axisTag: number}, output: {axisTag: number}, description?: string}] (at most 1024), description?: string (of the whole list)} replaces the font's cross-axis mappings (the second version of the avar table): where the location reaches input, the axes in output take the values given there. Locations are design coordinates by axis tag; each side names at least one of the font's axes. Descriptions hold at most 4096 bytes. An empty list without a description removes the mappings. Exporting them waits for the variable-font compiler → {description?, mappings: [{description?, input, output}]}",
            has_doc,
            set_mappings
        ),
        cmd!(
            "font.interpolateFonts",
            "Interpolate Fonts…",
            ["Font"],
            None,
            "{a: document title | path, b: document title | path, amount: percent (-100000..100000)} opens a new font between, or beyond, two single-master fonts: 0 gives a, 100 gives b, 50 the midpoint, values below 0 or above 100 extrapolate. Both fonts need the same units per em and the same glyphs, and each glyph the same shapes in the same order (paths with the same node count, node types and closedness; components of the same glyph) and the same anchors; the first difference is reported and nothing opens. Widths, nodes, component placements, anchors, the master's metrics, stems and zones, and the kerning interpolate; everything else is copied from a → {title, glyphs, amount}",
            always,
            interpolate
        )
        .files(const { &[FileParam::read("a").document(), FileParam::read("b").document()] }),
    ]
}

/// Reads one side of a mapping: the font's axis tags with finite values, in the font's order.
fn parse_side(p: &Params, font: &Font, at: &str, v: Option<&Value>) -> Result<Vec<(String, f64)>> {
    let Some(Value::Object(o)) = v else {
        return Err(p.bad(format!("`{at}` must be an object of axis tags and numbers, got {}", v.map_or("nothing", kind))));
    };
    if o.is_empty() {
        return Err(p.bad(format!("`{at}` names no axis")));
    }
    for (tag, value) in o {
        if !font.axes.iter().any(|a| a.tag == *tag) {
            let tags: Vec<&str> = font.axes.iter().map(|a| a.tag.as_str()).collect();
            return Err(p.bad(format!("`{at}`: the font has no axis `{tag}` (its axes: {})", tags.join(", "))));
        }
        if !value.as_f64().is_some_and(|v| v.is_finite() && v.abs() <= MAX_COORDINATE) {
            return Err(p.bad(format!("`{at}.{tag}` must be a number from -{MAX_COORDINATE} to {MAX_COORDINATE}")));
        }
    }
    Ok(font.axes.iter().filter_map(|a| Some((a.tag.clone(), o.get(&a.tag)?.as_f64()?))).collect())
}

fn parse_description(p: &Params, at: &str, v: Option<&Value>) -> Result<Option<String>> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => clean_description(s).map_err(|e| p.bad(format!("`{at}`: {e}"))),
        Some(other) => Err(p.bad(format!("`{at}` must be a string, got {}", kind(other)))),
    }
}

fn set_mappings(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.setCrossAxisMappings", v)?;
    let list = p.array("mappings")?;
    if list.len() > MAX_CROSS_AXIS_MAPPINGS {
        return Err(p.bad(format!("`mappings` holds {} entries; at most {MAX_CROSS_AXIS_MAPPINGS} are allowed", list.len())));
    }
    let font = s.font()?;
    if font.axes.is_empty() && !list.is_empty() {
        return Err(p.bad("the font has no axes: add axes before mapping between them"));
    }
    let mut new = CrossAxisMappings { description: parse_description(&p, "description", p.get("description"))?, mappings: Vec::new() };
    for (i, item) in list.iter().enumerate() {
        let Value::Object(o) = item else { return Err(p.bad(format!("`mappings[{i}]` must be an object, got {}", kind(item)))) };
        new.mappings.push(CrossAxisMapping {
            description: parse_description(&p, &format!("mappings[{i}].description"), o.get("description"))?,
            input: parse_side(&p, font, &format!("mappings[{i}].input"), o.get("input"))?,
            output: parse_side(&p, font, &format!("mappings[{i}].output"), o.get("output"))?,
        });
    }
    s.edit("Cross-Axis Mappings", |font| {
        let root = &mut Arc::make_mut(&mut font.info).user_data.0;
        if new.is_empty() {
            if let Some(i) = root.entries.iter().position(|(k, _)| k.as_str() == fc_keys::CROSS_AXIS_MAPPINGS) {
                root.remove_at(i);
            }
        } else {
            root.insert(fc_keys::CROSS_AXIS_MAPPINGS, new.to_plist());
        }
        Ok(())
    })?;
    Ok(new.to_json())
}

/// An open document named by title or path, else a font file read from `r`.
fn font_ref(s: &Session, r: &str) -> Result<Arc<Font>> {
    if let Some(d) = s.documents().iter().find(|d| d.title == r || d.path.as_deref().is_some_and(|p| p == std::path::Path::new(r))) {
        return Ok(d.font.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (font, _) = crate::io::open(std::path::Path::new(r), None)?;
        Ok(Arc::new(font))
    }
    #[cfg(target_arch = "wasm32")]
    Err(crate::EngineError::Io(format!("no open document is called `{r}`")))
}

fn interpolate(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.interpolateFonts", v)?;
    let a = font_ref(s, p.str("a")?)?;
    let b = font_ref(s, p.str("b")?)?;
    let amount = p.f64("amount")?;
    if amount.abs() > MAX_INTERPOLATION_AMOUNT {
        return Err(p.bad(format!("`amount` must be from -{MAX_INTERPOLATION_AMOUNT} to {MAX_INTERPOLATION_AMOUNT} percent")));
    }
    let font = interpolate_fonts(&a, &b, amount / 100.0).map_err(|e| p.bad(e))?;
    let glyphs = font.glyphs.len();
    s.add_document(font, None)?;
    let title = s.doc()?.title.clone();
    Ok(json!({"title": title, "glyphs": glyphs, "amount": amount}))
}

/// Interpolates numbers at `t` and remembers the first result that is not a usable coordinate.
struct Blend {
    t: f64,
    bad: Option<f64>,
}

impl Blend {
    fn at(&mut self, a: f64, b: f64) -> f64 {
        let v = a * (1.0 - self.t) + b * self.t;
        if !(v.is_finite() && v.abs() <= MAX_COORDINATE) {
            self.bad.get_or_insert(v);
            return a;
        }
        v
    }

    fn pair(&mut self, a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        (self.at(a.0, b.0), self.at(a.1, b.1))
    }

    fn check(&self, what: &str) -> std::result::Result<(), String> {
        match self.bad {
            Some(v) => Err(format!("{what}: the result {v} is beyond ±{MAX_COORDINATE} units; use an amount closer to 0..100")),
            None => Ok(()),
        }
    }
}

/// The master of a single-master font.
fn only_master<'f>(font: &'f Font, which: &str) -> std::result::Result<&'f Master, String> {
    match font.masters.as_slice() {
        [m] => Ok(m),
        list => Err(format!("the {which} font has {} masters: Interpolate Fonts takes fonts with one master", list.len())),
    }
}

/// The font `t` of the way from `a` to `b` (0 is `a`, 1 is `b`), or the first difference that
/// keeps the two from interpolating.
pub fn interpolate_fonts(a: &Font, b: &Font, t: f64) -> std::result::Result<Font, String> {
    if !t.is_finite() {
        return Err("the amount must be a finite number".into());
    }
    let (ma, mb) = (only_master(a, "first")?, only_master(b, "second")?);
    if a.units_per_em != b.units_per_em {
        return Err(format!("the fonts have different units per em ({} and {})", a.units_per_em, b.units_per_em));
    }
    for g in b.glyphs.iter() {
        if a.glyph(&g.name).is_none() {
            return Err(format!("glyph `{}` is in the second font only", g.name));
        }
    }
    let mut glyphs: Vec<Arc<Glyph>> = Vec::with_capacity(a.glyphs.len());
    for ga in a.glyphs.iter() {
        let gb = b.glyph(&ga.name).ok_or_else(|| format!("glyph `{}` is in the first font only", ga.name))?;
        let what = format!("glyph `{}`", ga.name);
        let mut glyph = Glyph::clone(ga);
        match (ga.layer(&ma.id), gb.layer(&mb.id)) {
            (Some(la), Some(lb)) => {
                let layer = interpolate_layer(la, lb, t).map_err(|e| format!("{what}: {e}"))?;
                glyph.layers = vec![layer];
            }
            (None, None) => glyph.layers.clear(),
            (Some(_), None) => return Err(format!("{what} has no master layer in the second font")),
            (None, Some(_)) => return Err(format!("{what} has no master layer in the first font")),
        }
        glyphs.push(Arc::new(glyph));
    }
    let mut out = a.clone();
    out.glyphs = GlyphTable::from_glyphs(glyphs);
    out.masters = Arc::new(vec![interpolate_master(a, ma, b, mb, t)?]);
    out.kerning = Arc::new(interpolate_kerning(&a.kerning, &ma.id, &b.kerning, &mb.id, t)?);
    Ok(out)
}

fn node_kind(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Line => "a line point",
        NodeKind::Curve => "a curve point",
        NodeKind::QCurve => "a quadratic point",
        NodeKind::OffCurve => "an off-curve point",
    }
}

fn shape_kind(s: &Shape) -> &'static str {
    match s {
        Shape::Path(_) => "a path",
        Shape::Component(_) => "a component",
        Shape::Image(_) => "an image",
        Shape::Raw(_) => "a shape this version does not read",
    }
}

/// Why two paths cannot interpolate; `None` when they can.
fn path_difference(n: usize, a: &Path, b: &Path) -> Option<String> {
    if a.closed != b.closed {
        let c = |p: &Path| if p.closed { "closed" } else { "open" };
        return Some(format!("shape {n} is {} in the first font and {} in the second", c(a), c(b)));
    }
    if a.nodes.len() != b.nodes.len() {
        return Some(format!("shape {n} has {} nodes in the first font and {} in the second", a.nodes.len(), b.nodes.len()));
    }
    let (i, (x, y)) = a.nodes.iter().zip(&b.nodes).enumerate().find(|(_, (x, y))| x.kind != y.kind)?;
    Some(format!("node {} of shape {n} is {} in the first font and {} in the second", i + 1, node_kind(x.kind), node_kind(y.kind)))
}

fn blend_component(a: &Component, b: &Component, blend: &mut Blend) -> Transform {
    match (a.transform, b.transform) {
        (
            Transform::Decomposed { pos, scale, angle, slant },
            Transform::Decomposed { pos: pos_b, scale: scale_b, angle: angle_b, slant: slant_b },
        ) => Transform::Decomposed {
            pos: blend.pair(pos, pos_b),
            scale: blend.pair(scale, scale_b),
            angle: blend.at(angle, angle_b),
            slant: blend.pair(slant, slant_b),
        },
        _ => {
            let (x, y) = (a.affine().as_coeffs(), b.affine().as_coeffs());
            let mut m = [0.0; 6];
            for (slot, (p, q)) in m.iter_mut().zip(x.iter().zip(y.iter())) {
                *slot = blend.at(*p, *q);
            }
            Transform::Matrix(m)
        }
    }
}

/// The first anchor of each name, in order.
fn anchors_by_name(layer: &Layer) -> HashMap<&str, &Anchor> {
    let mut out = HashMap::with_capacity(layer.anchors.len());
    for a in &layer.anchors {
        out.entry(a.name.as_str()).or_insert(a);
    }
    out
}

/// `a` with its numbers moved `t` of the way to `b`'s; the first difference when they do not
/// correspond. Images and shapes the model keeps unread stay as `a` has them.
fn interpolate_layer(a: &Layer, b: &Layer, t: f64) -> std::result::Result<Layer, String> {
    if a.shapes.len() != b.shapes.len() {
        return Err(format!("it has {} shapes in the first font and {} in the second", a.shapes.len(), b.shapes.len()));
    }
    let (anchors_a, anchors_b) = (anchors_by_name(a), anchors_by_name(b));
    if let Some(name) = a.anchors.iter().map(|x| x.name.as_str()).find(|n| !anchors_b.contains_key(n)) {
        return Err(format!("anchor `{name}` is in the first font only"));
    }
    if let Some(name) = b.anchors.iter().map(|x| x.name.as_str()).find(|n| !anchors_a.contains_key(n)) {
        return Err(format!("anchor `{name}` is in the second font only"));
    }
    let mut blend = Blend { t, bad: None };
    let mut out = a.clone();
    out.width = blend.at(a.width, b.width);
    if let (Some(x), Some(y)) = (a.info.vert_width, b.info.vert_width) {
        out.info.vert_width = Some(blend.at(x, y));
    }
    if let (Some(x), Some(y)) = (a.info.vert_origin, b.info.vert_origin) {
        out.info.vert_origin = Some(blend.at(x, y));
    }
    for (i, (shape, other)) in out.shapes.iter_mut().zip(&b.shapes).enumerate() {
        let n = i + 1;
        match (shape, other) {
            (Shape::Path(p), Shape::Path(q)) => {
                if let Some(e) = path_difference(n, p, q) {
                    return Err(e);
                }
                for (x, y) in p.nodes.iter_mut().zip(&q.nodes) {
                    (x.x, x.y) = blend.pair((x.x, x.y), (y.x, y.y));
                }
            }
            (Shape::Component(c), Shape::Component(d)) => {
                if c.glyph != d.glyph {
                    return Err(format!("shape {n} is a component of `{}` in the first font and of `{}` in the second", c.glyph, d.glyph));
                }
                c.transform = blend_component(c, d, &mut blend);
                for v in c.info.smart_values.iter_mut() {
                    if let Some(w) = d.info.smart_values.iter().find(|w| w.property == v.property) {
                        v.value = blend.at(v.value, w.value);
                    }
                }
            }
            (shape, other) if std::mem::discriminant(&*shape) == std::mem::discriminant(other) => {}
            (shape, other) => {
                return Err(format!("shape {n} is {} in the first font and {} in the second", shape_kind(shape), shape_kind(other)));
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for anchor in out.anchors.iter_mut() {
        if !seen.insert(anchor.name.clone()) {
            continue;
        }
        if let Some(other) = anchors_b.get(anchor.name.as_str()) {
            (anchor.x, anchor.y) = blend.pair((anchor.x, anchor.y), (other.x, other.y));
        }
    }
    blend.check("the outline")?;
    Ok(out)
}

/// `a`'s master with its metrics, stems and zones interpolated. Metrics and stems match by
/// their definition in each font; one only `a` defines keeps its value.
fn interpolate_master(a: &Font, ma: &Master, b: &Font, mb: &Master, t: f64) -> std::result::Result<Master, String> {
    let mut blend = Blend { t, bad: None };
    let mut out = ma.clone();
    for (i, def) in a.metrics.iter().enumerate() {
        let Some(j) = b.metrics.iter().position(|d| d.kind == def.kind && d.name == def.name && d.filter == def.filter) else { continue };
        let (Some(x), Some(y)) = (ma.metric_values.get(i).copied(), mb.metric_values.get(j).copied()) else { continue };
        if let Some(slot) = out.metric_values.get_mut(i) {
            slot.pos = blend.at(x.pos, y.pos);
            slot.over = blend.at(x.over, y.over);
        }
    }
    for (i, def) in a.stems.iter().enumerate() {
        let Some(j) = b.stems.iter().position(|d| d.name == def.name && d.horizontal == def.horizontal) else { continue };
        let (Some(x), Some(y)) = (ma.stem_values.get(i).copied(), mb.stem_values.get(j).copied()) else { continue };
        if let Some(slot) = out.stem_values.get_mut(i) {
            *slot = blend.at(x, y);
        }
    }
    if ma.zones.len() == mb.zones.len() {
        for (z, other) in out.zones.iter_mut().zip(&mb.zones) {
            z.pos = blend.at(z.pos, other.pos);
            z.size = blend.at(z.size, other.size);
        }
    }
    blend.check("the master's metrics")?;
    Ok(out)
}

/// Each kerning direction of the two masters interpolated into one table for `ma`; a pair only
/// one font has counts as 0 in the other. Contextual kerning is `a`'s.
fn interpolate_kerning(a: &Kerning, ma: &str, b: &Kerning, mb: &str, t: f64) -> std::result::Result<Kerning, String> {
    let mut blend = Blend { t, bad: None };
    let mut out = a.clone();
    for (slot, ta, tb) in [(&mut out.ltr, &a.ltr, &b.ltr), (&mut out.rtl, &a.rtl, &b.rtl), (&mut out.vertical, &a.vertical, &b.vertical)] {
        let pairs = |table: &fontcraft_model::KernTable, master: &str| -> Vec<(String, String, f64)> {
            table
                .0
                .iter()
                .filter(|m| m.master == master)
                .flat_map(|m| m.firsts.iter())
                .flat_map(|f| f.seconds.iter().map(move |(second, v)| (f.first.clone(), second.clone(), *v)))
                .collect()
        };
        let (pa, pb) = (pairs(ta, ma), pairs(tb, mb));
        let in_b: HashMap<(&str, &str), f64> = pb.iter().map(|(f, s, v)| ((f.as_str(), s.as_str()), *v)).collect();
        let in_a: HashSet<(&str, &str)> = pa.iter().map(|(f, s, _)| (f.as_str(), s.as_str())).collect();
        let mut firsts: Vec<fontcraft_model::KernFirst> = Vec::with_capacity(pa.len() + pb.len());
        for (f, s, v) in &pa {
            let w = in_b.get(&(f.as_str(), s.as_str())).copied().unwrap_or(0.0);
            firsts.push(fontcraft_model::KernFirst { first: f.clone(), seconds: vec![(s.clone(), blend.at(*v, w))] });
        }
        for (f, s, w) in &pb {
            if !in_a.contains(&(f.as_str(), s.as_str())) {
                firsts.push(fontcraft_model::KernFirst { first: f.clone(), seconds: vec![(s.clone(), blend.at(0.0, *w))] });
            }
        }
        *slot = if firsts.is_empty() {
            fontcraft_model::KernTable::default()
        } else {
            fontcraft_model::KernTable::new(vec![fontcraft_model::MasterKerning { master: ma.to_string(), firsts }])
        };
    }
    blend.check("the kerning")?;
    Ok(out)
}
