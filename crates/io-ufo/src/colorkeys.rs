//! The font-wide colour layer list of a UFO: `com.github.googlei18n.ufo2ft.colorLayers` in a
//! master's lib, base glyph → its colour layers, whose layer glyphs are glyphs of the font. Key
//! and forms as ufo2ft (`outlineCompiler.py`, `constants.py`) hands them to fontTools'
//! `colorLib.builder.buildCOLR`, and as glyphsLib (`builder/color_layers.py`) writes them; the
//! paint dictionaries follow fontTools' `PaintFormat` numbers, and the transforms of the
//! transform paints are ported from fontTools' `otTables.Paint.getTransform` (fontTools and
//! ufo2ft MIT, glyphsLib Apache-2.0; see `NOTICE`). fontc (`ufo2fontir/src/source.rs`) reads only
//! the pair form.
//!
//! | Lib, per base glyph | Model, per item, in order, in that master |
//! |---|---|
//! | `[[layer glyph, index], …]` (the `COLR` version 0 form) | a colour palette layer holding the layer glyph as one unmoved component; index 65535 is the text colour |
//! | a paint: `PaintColrLayers` (format 1) of `PaintGlyph` (10) paints, or a list of them, or one alone | per `PaintGlyph`, a full-colour layer holding its glyph as one component, whose fill colour or gradient is the glyph paint's paint |
//! | `PaintSolid` (2): `PaletteIndex`, `Alpha` | the fill colour as a palette entry with that alpha |
//! | `PaintLinearGradient` (4), `PaintRadialGradient` (6), `PaintSweepGradient` (8) with a `ColorLine` (`Extend` as `pad`, `repeat` or `reflect`; `ColorStop`: `StopOffset`, `PaletteIndex`, `Alpha`) | the gradient, its points as fractions of the component's box (curve extremes included), radii as fractions of its width, the sweep as a start angle and the angle it turns |
//! | `PaintTransform` (12, a matrix as a list of six numbers or as `xx`…`dy`), `PaintTranslate` (14), and the scale, rotate and skew paints (16 to 30), around a `PaintGlyph` | the component's transform; the writer gives a translation `PaintTranslate`, any other transform `PaintTransform` with a list, and keeps the paints a file had while they still make the component's transform |
//!
//! The writer writes a glyph's colour layers as pairs when all are palette layers, else as a
//! `PaintColrLayers` (the form the file had: a list, or one paint alone, is kept). A palette layer
//! among full-colour layers becomes a `PaintGlyph` of a `PaintSolid` with alpha 1. Gradient
//! geometry is written as the compiler paints it (`fontcraft-color`'s `paint::glyph_paint`): a
//! linear gradient's third point is the start-to-end vector turned −90° about the start, a radial
//! gradient with both radii 0 runs from its start point to the box's farthest corner, stops are
//! sorted and their offsets clamped to 0..1. A gradient under a transform needs one that only
//! moves and scales evenly. A colour that is not a palette entry (the 50 % grey of a shape
//! without a fill included) is added to the end of every colour palette, as glyphsLib adds it,
//! with alpha 1 in the paint and its own alpha in the palette.
//!
//! **Reading is checked by writing**, as in [`crate::libkeys`], one base glyph at a time: an
//! entry becomes colour layers only when the writer gives the same entry back (in any key order,
//! numbers equal to nine significant digits); the other entries stay user data, which the writer
//! merges back. The list as read is written as it was while it says the same. The layers made
//! from it carry a mark in their UFO data, so the writer puts them back in the list rather than
//! in UFO layers, as long as each still holds one component of a glyph of the font, unmoved for a
//! palette layer.
//!
//! Gradients need the box of their layer glyph's outline. While a [`Scope`] is open each box is
//! measured once per glyph and master, and the measuring shares [`MAX_BOX_SEGMENTS`]; the reader
//! and the writer's entry points open one.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::{
    Color, ColorStop, Component, ComponentInfo, Extend, Family, Font, Glyph, Gradient, GradientKind, Layer, MAX_COORDINATE, MAX_GRADIENT_STOPS,
    Master, PaletteEntry, Shape, ShapeAttrs, Transform,
};
use fontcraft_plist::{Dict, Num, Number, Value};

use crate::glyphkeys::{array, coordinate, dict, int, number, string, synthetic_id};

pub(crate) const COLOR_LAYERS: &str = "com.github.googlei18n.ufo2ft.colorLayers";
pub(crate) const COLOR_PALETTES: &str = "com.github.googlei18n.ufo2ft.colorPalettes";
/// The mark in a layer's UFO data. Its value is the form of the glyph's entry: `true` for pairs
/// or a `PaintColrLayers`, `"list"` for a list of paints, `"single"` for one paint alone.
const MARK: &str = "colorLayers";
/// A component's transform paints as the file had them, outermost first, without their `Paint`.
const PAINT_TRANSFORM: &str = "paintTransform";
/// Most colour layers read for one base glyph.
const MAX_LIST_LAYERS: usize = 1024;
/// Most transform paints read around one glyph paint.
const MAX_WRAPPERS: usize = 16;
/// The name of a full-colour layer read from the list, as the reference names such layers.
const FULL_COLOR_NAME: &str = "Color";
/// The fill of a shape on a full-colour layer with neither a fill colour nor a gradient, as the
/// compiler paints it (`color.fullColor.defaultGrayFill`).
const DEFAULT_FILL: [f64; 4] = [0.5, 0.5, 0.5, 1.0];
const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// Numbers the check by writing takes as equal: nine significant digits.
const TOLERANCE: f64 = 1e-9;
/// Most outline segments the boxes measured while one [`Scope`] is open may hold together.
pub(crate) const MAX_BOX_SEGMENTS: usize = 1 << 24;

/// The boxes of layer glyphs measured while a [`Scope`] is open.
struct Boxes {
    /// (font address, glyph, master) → the untransformed box: `None` when the outline cannot be
    /// made, `Some(None)` when it has no segment.
    cache: HashMap<(usize, String, String), Option<Option<[f64; 4]>>>,
    left: usize,
}

thread_local! {
    static BOXES: RefCell<Option<Boxes>> = const { RefCell::new(None) };
}

/// Keeps the measured boxes of layer glyphs until dropped. Open one around work on a whole font
/// that does not change its outlines; scopes may nest, and the outermost one owns the boxes.
pub(crate) struct Scope {
    outer: bool,
}

impl Scope {
    pub(crate) fn enter() -> Scope {
        let outer = BOXES.with(|b| match b.try_borrow_mut() {
            Ok(mut b) if b.is_none() => {
                *b = Some(Boxes { cache: HashMap::new(), left: MAX_BOX_SEGMENTS });
                true
            }
            _ => false,
        });
        Scope { outer }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.outer {
            BOXES.with(|b| {
                if let Ok(mut b) = b.try_borrow_mut() {
                    *b = None;
                }
            });
        }
    }
}

/// fontTools' `PaintFormat` numbers.
mod format {
    pub const LAYERS: i64 = 1;
    pub const SOLID: i64 = 2;
    pub const LINEAR: i64 = 4;
    pub const RADIAL: i64 = 6;
    pub const SWEEP: i64 = 8;
    pub const GLYPH: i64 = 10;
    pub const TRANSFORM: i64 = 12;
    pub const TRANSLATE: i64 = 14;
    pub const SCALE: i64 = 16;
    pub const SCALE_CENTER: i64 = 18;
    pub const SCALE_UNIFORM: i64 = 20;
    pub const SCALE_UNIFORM_CENTER: i64 = 22;
    pub const ROTATE: i64 = 24;
    pub const ROTATE_CENTER: i64 = 26;
    pub const SKEW: i64 = 28;
    pub const SKEW_CENTER: i64 = 30;
}

/// A layer the list holds, as the writer writes it.
pub(crate) enum Item<'a> {
    /// A palette layer: its layer glyph and palette entry.
    Pair(&'a str, PaletteEntry),
    /// A full-colour layer: its one component.
    Paint(&'a Component),
}

/// What the list holds of `l`: a colour layer read from it that is still one component of a
/// glyph of the font (unmoved for a palette layer) and that the writer can write.
pub(crate) fn listed<'a>(font: &Font, l: &'a Layer) -> Option<Item<'a>> {
    if l.is_master_layer() || l.extra.get(Family::Ufo, MARK).is_none() || !l.info.attrs.coordinates.is_empty() || !l.anchors.is_empty() {
        return None;
    }
    let [Shape::Component(c)] = l.shapes.as_slice() else { return None };
    font.glyph(&c.glyph)?;
    if l.info.attrs.full_color {
        if l.info.attrs.color_palette.is_some() {
            return None;
        }
        leaf(font, &l.master, c, 0, &mut Vec::new())?;
        return Some(Item::Paint(c));
    }
    let entry = l.info.attrs.color_palette?;
    c.transform.is_identity().then_some(Item::Pair(c.glyph.as_str(), entry))
}

/// Whether the writer writes master `master`'s list from its layers ([`value`]): some layer of it
/// is listed, and its user data holds no list of another form than a dictionary.
pub(crate) fn writes_list(font: &Font, master: &Master) -> bool {
    let _boxes = Scope::enter();
    master.user_data.get(COLOR_LAYERS).is_none_or(|v| v.as_dict().is_some())
        && font.glyphs.iter().any(|g| g.layers.iter().any(|l| l.master == master.id && listed(font, l).is_some()))
}

/// The list the writer gives `master`: the entries its user data keeps, with the entry of each
/// glyph that has listed layers, in the font's glyph order; and the colours it adds to the
/// palettes, numbered from `base` (the length of the master's first palette). `None` when it has
/// no listed layers.
pub(crate) fn value(font: &Font, master: &Master, base: usize) -> Option<(Value, Vec<[f64; 4]>)> {
    let _boxes = Scope::enter();
    let mut d = match master.user_data.get(COLOR_LAYERS) {
        None => Dict::new(),
        Some(Value::Dict(d)) => match Value::Dict(d.clone()).without_spellings() {
            Value::Dict(d) => d,
            _ => Dict::new(),
        },
        Some(_) => return None,
    };
    let mut added = Vec::new();
    let mut any = false;
    for g in font.glyphs.iter() {
        if let Some(v) = glyph_value(font, g, &master.id, base, &mut added) {
            d.insert(g.name.as_str(), v);
            any = true;
        }
    }
    any.then_some((Value::Dict(d), added))
}

/// The length of the first palette of a `colorPalettes` value; 0 without one.
pub(crate) fn palette_len(palettes: Option<&Value>) -> usize {
    palettes.and_then(Value::as_array).and_then(|a| a.items.first()).and_then(Value::as_array).map_or(0, |p| p.items.len())
}

/// `palettes` with `added` at the end of each palette; one palette of them when there is none.
pub(crate) fn with_added(palettes: Option<&Value>, added: &[[f64; 4]]) -> Value {
    let colors: Vec<Value> = added.iter().map(|c| array(c.iter().map(|x| real(*x)).collect())).collect();
    let mut out: Vec<Value> =
        palettes.and_then(Value::as_array).map(|a| a.items.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| vec![array(Vec::new())]);
    for p in &mut out {
        if let Value::Array(a) = p {
            a.items.extend(colors.iter().cloned());
        }
    }
    array(out)
}

/// Whether `a` and `b` say the same: dictionaries in any key order, numbers equal to nine
/// significant digits, everything else equal.
pub(crate) fn close(a: &Value, b: &Value) -> bool {
    close_at(a, b, 0)
}

fn close_at(a: &Value, b: &Value, depth: usize) -> bool {
    if depth > fontcraft_plist::MAX_DEPTH {
        return false;
    }
    match (a, b) {
        (Value::Dict(x), Value::Dict(y)) => {
            x.entries.len() == y.entries.len() && x.entries.iter().all(|(k, v)| y.get(k.as_str()).is_some_and(|w| close_at(v, w, depth + 1)))
        }
        (Value::Array(x), Value::Array(y)) => x.items.len() == y.items.len() && x.items.iter().zip(&y.items).all(|(v, w)| close_at(v, w, depth + 1)),
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.value().as_f64(), y.value().as_f64());
            (x - y).abs() <= TOLERANCE * x.abs().max(y.abs()).max(1.0)
        }
        _ => a == b,
    }
}

/// What the list leaves out of the full-colour layers it holds, one line per kind with the
/// number of layers.
pub(crate) fn losses(font: &Font) -> Vec<String> {
    let _boxes = Scope::enter();
    let (mut added, mut other) = (0usize, 0usize);
    for l in font.glyphs.iter().flat_map(|g| g.layers.iter()) {
        let Some(Item::Paint(c)) = listed(font, l) else { continue };
        let a = &c.info.attrs;
        let colors: Vec<Color> = match &a.gradient {
            Some(g) => g.stops.iter().map(|s| s.color).collect(),
            None => vec![a.fill_color.unwrap_or(Color::Rgba(DEFAULT_FILL))],
        };
        added += usize::from(colors.iter().any(|c| matches!(c, Color::Rgba(_))));
        let kept = match &a.gradient {
            Some(g) => ShapeAttrs { gradient: Some(g.clone()), ..Default::default() },
            None => ShapeAttrs { fill_color: a.fill_color, ..Default::default() },
        };
        let geometry = a.gradient.as_ref().is_some_and(|g| match g.kind {
            GradientKind::Radial { angle, .. } => angle != 0.0,
            GradientKind::Conic { control_radius, .. } => control_radius != 0.0,
            GradientKind::Linear { .. } => false,
        });
        let info = ComponentInfo { attrs: kept, ..Default::default() };
        other += usize::from(*c.info != info || geometry);
    }
    let mut out = Vec::new();
    if added > 0 {
        out.push(format!(
            "the colours of full-colour layers in the colour layer list that are not palette entries, added to the end of each colour palette ({added})"
        ));
    }
    if other > 0 {
        out.push(format!(
            "the attributes of components on full-colour layers in the colour layer list other than a fill colour or gradient, and their gradients' ellipse angles and sweep knob radii ({other})"
        ));
    }
    out
}

/// Turns the entries of each master's list into colour layers, where the writer gives the entry
/// back; the other entries stay user data.
pub(crate) fn read(font: &mut Font) {
    // Adding colour layers leaves the master layers, and so the outlines, as they are.
    let _boxes = Scope::enter();
    let lists: Vec<(String, Dict)> =
        font.masters.iter().filter_map(|m| m.user_data.get(COLOR_LAYERS).and_then(Value::as_dict).map(|d| (m.id.clone(), d.clone()))).collect();
    for (master, list) in lists {
        let mut left = Dict { layout: list.layout, ..Default::default() };
        let mut typed = 0usize;
        for (base, entry) in &list.entries {
            if typed < font.glyphs.as_arcs().len() && add_glyph(font, &master, base.as_str(), entry) {
                typed += 1;
            } else {
                left.entries.push((base.clone(), entry.clone()));
            }
        }
        if typed == 0 {
            continue;
        }
        if let Some(m) = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == master) {
            if left.entries.is_empty() {
                m.user_data.0.entries.retain(|(k, _)| k.as_str() != COLOR_LAYERS);
            } else if let Some(v) = m.user_data.0.get_mut(COLOR_LAYERS) {
                *v = Value::Dict(left);
            }
        }
    }
}

/// Adds the colour layers `entry` describes to `base` in `master` when the writer gives `entry`
/// back; otherwise changes nothing and gives `false`.
fn add_glyph(font: &mut Font, master: &str, base: &str, entry: &Value) -> bool {
    let Some(layers) = layers_of(font, master, base, entry) else { return false };
    let ids: Vec<String> = layers.iter().map(|l| l.id.clone()).collect();
    match font.glyph_mut(base) {
        Ok(mut g) => g.layers.extend(layers),
        Err(_) => return false,
    }
    let mut added = Vec::new();
    let back = font.glyph(base).and_then(|g| glyph_value(font, g, master, 0, &mut added));
    if added.is_empty() && back.is_some_and(|b| close(&b, entry)) {
        return true;
    }
    if let Ok(mut g) = font.glyph_mut(base) {
        g.layers.retain(|l| !ids.contains(&l.id));
    }
    false
}

/// The colour layers `entry` describes for `base` in `master`; `None` when it names a glyph the
/// font lacks or the base glyph itself, when `base` has colour layers of that master already, or
/// when it has a form this reader does not type.
fn layers_of(font: &Font, master: &str, base: &str, entry: &Value) -> Option<Vec<Layer>> {
    let g = font.glyph(base)?;
    let width = g.layer(master)?.width;
    if g.layers.iter().any(|l| l.master == master && (l.info.attrs.color_palette.is_some() || l.info.attrs.full_color)) {
        return None;
    }
    let new_layer = |k: usize, made: &[Layer], form: Value| {
        let mut l = Layer::new(master, width);
        let mut id = synthetic_id(master, &format!("{MARK}.{k}"));
        while g.layers.iter().chain(made).any(|x| x.id == id) {
            id.push('#');
        }
        l.id = id;
        l.extra.insert(Family::Ufo, MARK, form);
        l
    };
    let mut out: Vec<Layer> = Vec::new();
    if let Some(pairs) = crate::glyphkeys::palette_entries(entry) {
        if pairs.is_empty() || pairs.len() > MAX_LIST_LAYERS || pairs.iter().any(|(name, _)| name == base || font.glyph(name).is_none()) {
            return None;
        }
        for (k, (name, palette)) in pairs.into_iter().enumerate() {
            let mut l = new_layer(k, &out, Value::Bool(true));
            l.shapes = vec![Shape::Component(Component::new(name, 0.0, 0.0))];
            l.info.name = Some(match palette {
                PaletteEntry::Index(i) => format!("Color {i}"),
                PaletteEntry::Foreground => "Color *".to_string(),
            });
            l.info.attrs.color_palette = Some(palette);
            out.push(l);
        }
        return Some(out);
    }
    let (leaves, form): (Vec<&Value>, Value) = match entry {
        Value::Dict(d) if paint_format(d) == Some(format::LAYERS) => (d.get("Layers")?.as_array()?.items.iter().collect(), Value::Bool(true)),
        Value::Dict(_) => (vec![entry], string("single")),
        Value::Array(a) => (a.items.iter().collect(), string("list")),
        _ => return None,
    };
    if leaves.is_empty() || leaves.len() > MAX_LIST_LAYERS {
        return None;
    }
    for (k, v) in leaves.into_iter().enumerate() {
        let c = read_leaf(font, master, base, v)?;
        let mut l = new_layer(k, &out, form.clone());
        l.shapes = vec![Shape::Component(c)];
        l.info.name = Some(FULL_COLOR_NAME.to_string());
        l.info.attrs.full_color = true;
        out.push(l);
    }
    Some(out)
}

/// The entry the writer gives glyph `g` in `master`; `None` when it has no listed layers there.
fn glyph_value(font: &Font, g: &Glyph, master: &str, base: usize, added: &mut Vec<[f64; 4]>) -> Option<Value> {
    let layers: Vec<(&Layer, Item)> = g.layers.iter().filter(|l| l.master == master).filter_map(|l| listed(font, l).map(|i| (l, i))).collect();
    if layers.is_empty() {
        return None;
    }
    if layers.iter().all(|(_, i)| matches!(i, Item::Pair(..))) {
        let pairs = layers
            .iter()
            .filter_map(|(_, i)| match i {
                Item::Pair(name, entry) => {
                    let index = match entry {
                        PaletteEntry::Index(i) => *i,
                        PaletteEntry::Foreground => u16::MAX,
                    };
                    Some(array(vec![string(name), int(i64::from(index))]))
                }
                Item::Paint(_) => None,
            })
            .collect();
        return Some(array(pairs));
    }
    let mut paints = Vec::with_capacity(layers.len());
    for (_, item) in &layers {
        paints.push(match item {
            Item::Pair(name, entry) => {
                let index = match entry {
                    PaletteEntry::Index(i) => *i,
                    PaletteEntry::Foreground => u16::MAX,
                };
                let solid = solid(Color::Palette { index, alpha: 1.0 }, base, added)?;
                dict(vec![("Format", int(format::GLYPH)), ("Glyph", string(name)), ("Paint", solid)])
            }
            Item::Paint(c) => leaf(font, master, c, base, added)?,
        });
    }
    let form = layers.first().and_then(|(l, _)| l.extra.get(Family::Ufo, MARK)).and_then(Value::as_str);
    Some(match form {
        Some("list") => array(paints),
        Some("single") if paints.len() == 1 => paints.into_iter().next()?,
        _ => dict(vec![("Format", int(format::LAYERS)), ("Layers", array(paints))]),
    })
}

/// The `PaintGlyph` of component `c` in its transform paints.
fn leaf(font: &Font, master: &str, c: &Component, base: usize, added: &mut Vec<[f64; 4]>) -> Option<Value> {
    let m = c.transform.affine().as_coeffs();
    if !m.iter().all(|v| v.is_finite()) {
        return None;
    }
    let fill = fill_value(font, master, c, &m, base, added)?;
    let mut paint = dict(vec![("Format", int(format::GLYPH)), ("Glyph", string(&c.glyph)), ("Paint", fill)]);
    let recorded = c.extra.get(Family::Ufo, PAINT_TRANSFORM).and_then(Value::as_array).map(|a| a.items.clone());
    let wrappers = match recorded {
        Some(r) if wrappers_transform(&r).is_some_and(|t| t.iter().zip(&m).all(|(a, b)| near(*a, *b))) => r,
        _ => default_wrappers(&m),
    };
    for w in wrappers.into_iter().rev() {
        let Value::Dict(mut d) = w else { return None };
        d.insert("Paint", paint);
        paint = crate::libkeys::sorted_keys(Value::Dict(d));
    }
    Some(paint)
}

/// The transform paints the writer puts around a glyph paint moved by `m`, outermost first,
/// without their `Paint`.
fn default_wrappers(m: &[f64; 6]) -> Vec<Value> {
    let [xx, yx, xy, yy, dx, dy] = *m;
    if *m == IDENTITY {
        Vec::new()
    } else if [xx, yx, xy, yy] == [1.0, 0.0, 0.0, 1.0] {
        vec![dict(vec![("Format", int(format::TRANSLATE)), ("dx", number(dx)), ("dy", number(dy))])]
    } else {
        vec![dict(vec![("Format", int(format::TRANSFORM)), ("Transform", array(m.iter().map(|v| number(*v)).collect()))])]
    }
}

/// The transform the transform paints `wrappers` make together, outermost first.
fn wrappers_transform(wrappers: &[Value]) -> Option<[f64; 6]> {
    if wrappers.len() > MAX_WRAPPERS {
        return None;
    }
    let mut m = IDENTITY;
    for w in wrappers {
        let d = w.as_dict()?;
        m = mul(&m, &wrapper_transform(paint_format(d)?, d)?);
    }
    Some(m)
}

/// The fill of component `c`, moved by `m`: its gradient, else its fill colour, else the default
/// grey.
fn fill_value(font: &Font, master: &str, c: &Component, m: &[f64; 6], base: usize, added: &mut Vec<[f64; 4]>) -> Option<Value> {
    let a = &c.info.attrs;
    match (&a.gradient, a.fill_color) {
        (Some(g), _) => gradient_value(font, master, &c.glyph, g, m, base, added),
        (None, Some(color)) => solid(color, base, added),
        (None, None) => solid(Color::Rgba(DEFAULT_FILL), base, added),
    }
}

/// The palette index and alpha of `color` in a paint: a palette entry keeps its alpha; another
/// colour is added to `added` once and painted opaque, its alpha going to the palettes.
fn color_fields(color: Color, base: usize, added: &mut Vec<[f64; 4]>) -> Option<(i64, f64)> {
    let (index, alpha) = match color {
        Color::Palette { index, alpha } => (usize::from(index), alpha),
        Color::Rgba(rgba) => {
            if !rgba.iter().all(|v| v.is_finite()) {
                return None;
            }
            let at = match added.iter().position(|c| *c == rgba) {
                Some(i) => i,
                None => {
                    added.push(rgba);
                    added.len().saturating_sub(1)
                }
            };
            let index = base.checked_add(at).filter(|i| *i < usize::from(u16::MAX))?;
            (index, 1.0)
        }
    };
    alpha.is_finite().then_some((i64::try_from(index).ok()?, alpha))
}

fn solid(color: Color, base: usize, added: &mut Vec<[f64; 4]>) -> Option<Value> {
    let (index, alpha) = color_fields(color, base, added)?;
    Some(dict(vec![("Alpha", number(alpha)), ("Format", int(format::SOLID)), ("PaletteIndex", int(index))]))
}

fn extend_name(e: Extend) -> &'static str {
    match e {
        Extend::Pad => "pad",
        Extend::Repeat => "repeat",
        Extend::Reflect => "reflect",
    }
}

/// The gradient `g` of a component of `glyph` moved by `m`, in the glyph paint's own space.
fn gradient_value(font: &Font, master: &str, glyph: &str, g: &Gradient, m: &[f64; 6], base: usize, added: &mut Vec<[f64; 4]>) -> Option<Value> {
    let (s, dx, dy) = even_scale(m)?;
    if g.stops.len() > MAX_GRADIENT_STOPS || !g.numbers().all(f64::is_finite) {
        return None;
    }
    let [x0, y0, x1, y1] = bounds(font, master, glyph, m)?;
    let (w, h) = (x1 - x0, y1 - y0);
    let mut stops: Vec<ColorStop> = g.stops.iter().map(|st| ColorStop { offset: st.offset.clamp(0.0, 1.0), color: st.color }).collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut stop_values = Vec::with_capacity(stops.len());
    for st in &stops {
        let (index, alpha) = color_fields(st.color, base, added)?;
        stop_values.push(dict(vec![("Alpha", number(alpha)), ("PaletteIndex", int(index)), ("StopOffset", number(st.offset))]));
    }
    let line = dict(vec![("ColorStop", array(stop_values)), ("Extend", string(extend_name(g.extend)))]);
    // Points in the font's space, as the compiler paints them, then in the glyph paint's own.
    let at = |x: f64, y: f64| (x0 + w * x, y0 + h * y);
    let own = |(x, y): (f64, f64)| ((x - dx) / s, (y - dy) / s);
    Some(match g.kind {
        GradientKind::Linear { start, end } => {
            let (p0, p1) = (at(start.x, start.y), at(end.x, end.y));
            let p2 = (p0.0 + (p1.1 - p0.1), p0.1 - (p1.0 - p0.0));
            let ((ax, ay), (bx, by), (cx, cy)) = (own(p0), own(p1), own(p2));
            dict(vec![
                ("ColorLine", line),
                ("Format", int(format::LINEAR)),
                ("x0", number(ax)),
                ("x1", number(bx)),
                ("x2", number(cx)),
                ("y0", number(ay)),
                ("y1", number(by)),
                ("y2", number(cy)),
            ])
        }
        GradientKind::Radial { start, end, start_radius, end_radius, .. } => {
            let c0 = at(start.x, start.y);
            let (c1, r0, r1) = if start_radius == 0.0 && end_radius == 0.0 {
                let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
                (c0, 0.0, corners.iter().map(|&(x, y)| (x - c0.0).hypot(y - c0.1)).fold(0.0, f64::max))
            } else {
                (at(end.x, end.y), (start_radius * w).abs(), (end_radius * w).abs())
            };
            let ((ax, ay), (bx, by)) = (own(c0), own(c1));
            dict(vec![
                ("ColorLine", line),
                ("Format", int(format::RADIAL)),
                ("r0", number(r0 / s)),
                ("r1", number(r1 / s)),
                ("x0", number(ax)),
                ("x1", number(bx)),
                ("y0", number(ay)),
                ("y1", number(by)),
            ])
        }
        GradientKind::Conic { center, start_angle, angle, .. } => {
            let (cx, cy) = own(at(center.x, center.y));
            dict(vec![
                ("ColorLine", line),
                ("Format", int(format::SWEEP)),
                ("centerX", number(cx)),
                ("centerY", number(cy)),
                ("endAngle", number(start_angle + angle)),
                ("startAngle", number(start_angle)),
            ])
        }
    })
}

/// The scale and offset of `m` when it only moves and scales evenly by a positive factor.
fn even_scale(m: &[f64; 6]) -> Option<(f64, f64, f64)> {
    let [xx, yx, xy, yy, dx, dy] = *m;
    (yx == 0.0 && xy == 0.0 && xx == yy && xx > 0.0).then_some((xx, dx, dy))
}

/// The component `v` describes: a `PaintGlyph` of a glyph of the font other than `base`, in at
/// most [`MAX_WRAPPERS`] transform paints.
fn read_leaf(font: &Font, master: &str, base: &str, v: &Value) -> Option<Component> {
    let mut m = IDENTITY;
    let mut wrappers = Vec::new();
    let mut v = v;
    let d = loop {
        let d = v.as_dict()?;
        let f = paint_format(d)?;
        if f == format::GLYPH {
            break d;
        }
        if wrappers.len() >= MAX_WRAPPERS {
            return None;
        }
        m = mul(&m, &wrapper_transform(f, d)?);
        let mut rest = d.clone();
        rest.entries.retain(|(k, _)| k.as_str() != "Paint");
        wrappers.push(Value::Dict(rest));
        v = d.get("Paint")?;
    };
    if !m.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE) {
        return None;
    }
    let name = d.get("Glyph")?.as_str()?;
    if name == base || font.glyph(name).is_none() {
        return None;
    }
    let attrs = read_fill(font, master, name, &m, d.get("Paint")?)?;
    let [xx, yx, xy, yy, dx, dy] = m;
    let mut c = Component::new(name, 0.0, 0.0);
    if [xx, yx, xy, yy] == [1.0, 0.0, 0.0, 1.0] {
        c.transform = Transform::translate(dx, dy);
    } else {
        c.transform = Transform::Matrix(m);
    }
    c.info.attrs = attrs;
    let recorded = array(wrappers);
    if !close(&recorded, &array(default_wrappers(&m))) {
        c.extra.insert(Family::Ufo, PAINT_TRANSFORM, recorded);
    }
    Some(c)
}

/// The fill colour or gradient the paint `v` gives a component of `glyph` moved by `m`.
fn read_fill(font: &Font, master: &str, glyph: &str, m: &[f64; 6], v: &Value) -> Option<ShapeAttrs> {
    let d = v.as_dict()?;
    let mut attrs = ShapeAttrs::default();
    let f = paint_format(d)?;
    if f == format::SOLID {
        attrs.fill_color = Some(read_color(d)?);
        return Some(attrs);
    }
    if ![format::LINEAR, format::RADIAL, format::SWEEP].contains(&f) {
        return None;
    }
    let (s, dx, dy) = even_scale(m)?;
    let [x0, y0, x1, y1] = bounds(font, master, glyph, m)?;
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let n = |k: &str| d.get(k).and_then(coordinate);
    let frac = |x: f64, y: f64| ((s * x + dx - x0) / w, (s * y + dy - y0) / h);
    let (stops, extend) = read_line(d.get("ColorLine")?)?;
    let kind = if f == format::LINEAR {
        GradientKind::Linear { start: frac(n("x0")?, n("y0")?).into(), end: frac(n("x1")?, n("y1")?).into() }
    } else if f == format::RADIAL {
        GradientKind::Radial {
            start: frac(n("x0")?, n("y0")?).into(),
            end: frac(n("x1")?, n("y1")?).into(),
            start_radius: s * n("r0")? / w,
            end_radius: s * n("r1")? / w,
            angle: 0.0,
        }
    } else {
        let start = n("startAngle")?;
        GradientKind::Conic {
            center: frac(n("centerX")?, n("centerY")?).into(),
            start_angle: start,
            angle: n("endAngle")? - start,
            control_radius: 0.0,
        }
    };
    let gradient = Gradient { kind, stops, extend };
    if !gradient.numbers().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE) {
        return None;
    }
    attrs.gradient = Some(gradient);
    Some(attrs)
}

/// A palette entry with an alpha from 0 to 1: `PaletteIndex` (65535 the text colour) and `Alpha`.
fn read_color(d: &Dict) -> Option<Color> {
    let index = match d.get("PaletteIndex")?.as_num()? {
        Num::Int(i) => u16::try_from(i).ok()?,
        Num::Float(_) => return None,
    };
    let alpha = d.get("Alpha").and_then(coordinate).filter(|a| (0.0..=1.0).contains(a))?;
    Some(Color::Palette { index, alpha })
}

fn read_line(v: &Value) -> Option<(Vec<ColorStop>, Extend)> {
    let d = v.as_dict()?;
    let extend = match d.get("Extend")?.as_str()? {
        "pad" => Extend::Pad,
        "repeat" => Extend::Repeat,
        "reflect" => Extend::Reflect,
        _ => return None,
    };
    let items = &d.get("ColorStop")?.as_array()?.items;
    if items.len() > MAX_GRADIENT_STOPS {
        return None;
    }
    let stops = items
        .iter()
        .map(|s| {
            let s = s.as_dict()?;
            Some(ColorStop { offset: s.get("StopOffset").and_then(coordinate)?, color: read_color(s)? })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((stops, extend))
}

/// A paint's `Format`, an integer.
fn paint_format(d: &Dict) -> Option<i64> {
    match d.get("Format")?.as_num()? {
        Num::Int(i) => Some(i),
        Num::Float(_) => None,
    }
}

/// The transform of the transform paint `d` of format `f`, as fontTools'
/// `otTables.Paint.getTransform` gives it; `None` for any other format.
fn wrapper_transform(f: i64, d: &Dict) -> Option<[f64; 6]> {
    let n = |k: &str| d.get(k).and_then(coordinate);
    let around = |t: [f64; 6]| -> Option<[f64; 6]> {
        let (cx, cy) = (n("centerX")?, n("centerY")?);
        Some(mul(&mul(&translate(cx, cy), &t), &translate(-cx, -cy)))
    };
    match f {
        format::TRANSFORM => {
            let v: Vec<f64> = match d.get("Transform")? {
                Value::Dict(t) => ["xx", "yx", "xy", "yy", "dx", "dy"].iter().map(|k| t.get(k).and_then(coordinate)).collect::<Option<_>>()?,
                Value::Array(a) => a.items.iter().map(coordinate).collect::<Option<_>>()?,
                _ => return None,
            };
            let [xx, yx, xy, yy, dx, dy] = v.as_slice() else { return None };
            Some([*xx, *yx, *xy, *yy, *dx, *dy])
        }
        format::TRANSLATE => Some(translate(n("dx")?, n("dy")?)),
        format::SCALE => Some(scale(n("scaleX")?, n("scaleY")?)),
        format::SCALE_CENTER => around(scale(n("scaleX")?, n("scaleY")?)),
        format::SCALE_UNIFORM => n("scale").map(|s| scale(s, s)),
        format::SCALE_UNIFORM_CENTER => around(n("scale").map(|s| scale(s, s))?),
        format::ROTATE => Some(rotate(n("angle")?)),
        format::ROTATE_CENTER => around(rotate(n("angle")?)),
        format::SKEW => Some(skew(n("xSkewAngle")?, n("ySkewAngle")?)),
        format::SKEW_CENTER => around(skew(n("xSkewAngle")?, n("ySkewAngle")?)),
        _ => None,
    }
}

/// `a` after `b`: `b` applies first.
fn mul(a: &[f64; 6], b: &[f64; 6]) -> [f64; 6] {
    let [a0, a1, a2, a3, a4, a5] = *a;
    let [b0, b1, b2, b3, b4, b5] = *b;
    [a0 * b0 + a2 * b1, a1 * b0 + a3 * b1, a0 * b2 + a2 * b3, a1 * b2 + a3 * b3, a0 * b4 + a2 * b5 + a4, a1 * b4 + a3 * b5 + a5]
}

fn translate(dx: f64, dy: f64) -> [f64; 6] {
    [1.0, 0.0, 0.0, 1.0, dx, dy]
}

fn scale(x: f64, y: f64) -> [f64; 6] {
    [x, 0.0, 0.0, y, 0.0, 0.0]
}

/// fontTools' `_normSinCos`: values within 1e-15 of 0 or ±1 snap to them.
fn snap(v: f64) -> f64 {
    const EPSILON: f64 = 1e-15;
    if v.abs() < EPSILON {
        0.0
    } else if (v - 1.0).abs() < EPSILON {
        1.0
    } else if (v + 1.0).abs() < EPSILON {
        -1.0
    } else {
        v
    }
}

/// A rotation by `degrees` counter-clockwise.
fn rotate(degrees: f64) -> [f64; 6] {
    let r = degrees.to_radians();
    let (c, s) = (snap(r.cos()), snap(r.sin()));
    [c, s, -s, c, 0.0, 0.0]
}

/// A skew as `PaintSkew` states it: the x angle counter-clockwise, as fontTools reads it.
fn skew(x_degrees: f64, y_degrees: f64) -> [f64; 6] {
    [1.0, y_degrees.to_radians().tan(), (-x_degrees).to_radians().tan(), 1.0, 0.0, 0.0]
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= TOLERANCE * a.abs().max(b.abs()).max(1.0)
}

/// The box of `glyph`'s outline in `master` moved by `m`, which only moves and scales evenly,
/// curve extremes included (as the compiler measures a layer glyph): `[x0, y0, x1, y1]`, all 0
/// without an outline.
fn bounds(font: &Font, master: &str, glyph: &str, m: &[f64; 6]) -> Option<[f64; 4]> {
    let (s, dx, dy) = even_scale(m)?;
    let Some([x0, y0, x1, y1]) = glyph_box(font, master, glyph)? else { return Some([0.0; 4]) };
    let out = [s * x0 + dx, s * y0 + dy, s * x1 + dx, s * y1 + dy];
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// The untransformed box of `glyph`'s outline in `master`, from the open [`Scope`]'s boxes when
/// there is one; `None` when the outline cannot be made or the scope's budget is spent.
fn glyph_box(font: &Font, master: &str, glyph: &str) -> Option<Option<[f64; 4]>> {
    let key = (std::ptr::from_ref(font) as usize, glyph.to_string(), master.to_string());
    let open = BOXES.with(|b| b.try_borrow().ok().and_then(|b| b.as_ref().map(|b| (b.cache.get(&key).copied(), b.left))));
    match open {
        None => measure(font, master, glyph).0,
        Some((Some(hit), _)) => hit,
        Some((None, 0)) => None,
        Some((None, _)) => {
            let (value, cost) = measure(font, master, glyph);
            BOXES.with(|b| {
                if let Ok(mut b) = b.try_borrow_mut()
                    && let Some(b) = b.as_mut()
                {
                    b.left = b.left.saturating_sub(cost);
                    b.cache.insert(key, value);
                }
            });
            value
        }
    }
}

/// The untransformed box of `glyph`'s outline in `master` and the segments it took.
fn measure(font: &Font, master: &str, glyph: &str) -> (Option<Option<[f64; 4]>>, usize) {
    let Ok(path) = font.outline(glyph, master) else { return (None, 1) };
    let mut out: Option<[f64; 4]> = None;
    let mut segments = 1usize;
    for seg in path.segments() {
        segments += 1;
        let cubic = seg.to_cubic();
        let p = [cubic.p0, cubic.p1, cubic.p2, cubic.p3];
        let (lx, hx) = extremes(p.map(|p| p.x));
        let (ly, hy) = extremes(p.map(|p| p.y));
        out = Some(match out {
            None => [lx, ly, hx, hy],
            Some([a, b, c, d]) => [a.min(lx), b.min(ly), c.max(hx), d.max(hy)],
        });
    }
    if out.is_some_and(|b| !b.iter().all(|v| v.is_finite())) {
        return (None, segments);
    }
    (Some(out), segments)
}

/// The lowest and highest value of one coordinate of a cubic Bézier over 0..1.
fn extremes(v: [f64; 4]) -> (f64, f64) {
    let [p0, p1, p2, p3] = v;
    let (mut lo, mut hi) = (p0.min(p3), p0.max(p3));
    // A third of the derivative: a·t² + b·t + c.
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let mut roots = [f64::NAN; 2];
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            roots[0] = -c / b;
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let r = disc.sqrt();
            roots = [(-b + r) / (2.0 * a), (-b - r) / (2.0 * a)];
        }
    }
    for t in roots {
        if t > 0.0 && t < 1.0 {
            let u = 1.0 - t;
            let x = u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3;
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo, hi)
}

fn real(v: f64) -> Value {
    Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
}

#[cfg(test)]
#[path = "colorkeys_tests.rs"]
mod tests;
