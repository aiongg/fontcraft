//! A layer's data ([`LayerInfo`]) and its background, as each format version writes it.
//!
//! | Field | Versions 3 and 4 | Version 2 |
//! |---|---|---|
//! | name | `name` | the same; brace, bracket and colour layers are named by their role (`{100, 20}`, `[300]`, `]300]`, `Color 1`) |
//! | attributes | `attr`: `coordinates` (version 4: `"-"` for the master's value), `axisRules` `{min?, max?}`, `colorPalette` (an index or `"*"`), `color`, `sbixSize`, `svg` | read from the name, as glyphsLib 6.15.0 reads it (`classes.py`, Apache-2.0; see `NOTICE`) |
//! | smart-glyph poles | `partSelection` `{property: 1 or 2}` | `userData.PartSelection` |
//! | vertical metrics | `vertWidth`, `vertOrigin` | the same |
//! | metrics keys | `metricLeft` … `metricVertOrigin` | `leftMetricsKey`, `rightMetricsKey`, `widthMetricsKey` |
//! | colour, visibility | `color` (as a glyph's), `visible = 1` | the same |
//! | guides | `guides` (as a master's) | `guideLines` |
//! | hints | `hints`, kept as stored values; hints read from a UFO (glyphsLib's dictionaries) are converted, as glyphsLib 6.15.0's hint builder reads them (`builder/hints.py`, Apache-2.0; see `NOTICE`) | the same, index paths and points as brace strings |
//! | annotations | `annotations` `{type, pos?, angle?, text?, width?}` | the same with `position` `"{x, y}"` |
//! | image | `backgroundImage` `{imagePath, pos, scale, angle, slant, crop [x, y, w, h], alpha, locked}` | `transform` `"{a, b, c, d, tx, ty}"`, `crop` `"{{x, y}, {w, h}}"`, `locked = "1"` |
//! | background | `background` `{shapes, anchors, guides, hints, annotations, backgroundImage}` | `paths` and `components` for `shapes`, `guideLines` for `guides` |
//! | user data | `userData` | the same |
//!
//! A key becomes a field only when writing the field back gives the same text in the file's
//! version; a list only when every item does; the background only as a whole. Anything else stays
//! in the layer's `Extra` as written, and so does a value the writer leaves out (`visible = 0`, an
//! empty list or dictionary), which the writer then writes from there. Version 2 names stay names:
//! the attributes read from them are the model's view, and a change to the attributes that the
//! name does not say is reported by [`losses`].
//!
//! Written from the GlyphsSDK format documents (Apache-2.0).

use fontcraft_model::{
    Anchor, Annotation, AxisRule, Background, Extra, Guide, Hint, Image, LayerAttrs, LayerInfo, MAX_ANNOTATIONS, MAX_AXIS_VALUES, MAX_COORDINATE,
    MAX_LAYER_GUIDES, MAX_LAYER_HINTS, MAX_PART_PROPERTIES, MetricsKeys, PaletteEntry, Shape, SmartPole, Transform,
};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Layout, Num, Str, Value};

use crate::design::str_of;
use crate::{Result, as_pair, bad, finish, int, into_extra, list, num, pair, string, take};

/// A layer-like dictionary's shapes and anchors.
pub(crate) type Drawing = (Vec<Shape>, Vec<Anchor>);
/// A dictionary's entries as they are built.
pub(crate) type Entries = Vec<(Str, Value)>;

/// Pushes a drawing's shapes and anchors as entries, in a format version.
pub(crate) type PushShapes = fn(&mut Entries, &[Shape], &[Anchor], &str, u32) -> Result<()>;

/// How one version reads and writes the shapes and anchors of a layer-like dictionary.
pub(crate) struct Shapes {
    /// Takes the shapes and anchors out of a dictionary in a format version; `None` when one of
    /// them does not read.
    pub take: fn(&mut Dict, u32) -> Option<Drawing>,
    /// Pushes them as entries.
    pub push: PushShapes,
    /// Builds the dictionary in the version's key order.
    pub finish: fn(Vec<(Str, Value)>, &Extra) -> Dict,
}

pub(crate) fn dialect(version: u32) -> Dialect {
    match version {
        2 => Dialect::Glyphs2,
        4 => Dialect::Glyphs4,
        _ => Dialect::Glyphs3,
    }
}

pub(crate) fn same_text(a: &Value, b: &Value, version: u32) -> bool {
    let (mut x, mut y) = (String::new(), String::new());
    openstep::write_value(&mut x, a, dialect(version));
    openstep::write_value(&mut y, b, dialect(version));
    x == y
}

/// The value under `key`, taken out of `d` when writing it back gives the same text.
pub(crate) fn take_same<T>(
    d: &mut Dict,
    key: &str,
    version: u32,
    read: impl Fn(&Value) -> Option<T>,
    write: impl Fn(&T) -> Option<Value>,
) -> Option<T> {
    let v = d.get(key)?;
    let t = read(v)?;
    if !write(&t).is_some_and(|w| same_text(&w, v, version)) {
        return None;
    }
    take(d, key);
    Some(t)
}

/// [`take_same`], also taking a value the writer would lay out otherwise (a one-line
/// dictionary): the value as read stays beside the field for its layout ([`crate::LAYOUT_KEY`]).
fn take_relaid<T>(d: &mut Dict, key: &str, version: u32, read: impl Fn(&Value) -> Option<T>, write: impl Fn(&T) -> Option<Value>) -> Option<T> {
    let v = d.get(key)?;
    let t = read(v)?;
    let w = write(&t)?;
    let relayout = !same_text(&w, v, version);
    if relayout && !same_text(&crate::relaid(&w, v), v, version) {
        return None;
    }
    if let Some(source) = take(d, key)
        && relayout
    {
        crate::keep_layout(d, key, source);
    }
    Some(t)
}

/// The list under `key`, item by item, when it has from 1 to `max` items and every item writes
/// back as the same text; otherwise empty, and the list stays in `d`.
fn take_list<T>(
    d: &mut Dict,
    key: &str,
    version: u32,
    max: usize,
    read: impl Fn(&Value) -> Option<T>,
    write: impl Fn(&T) -> Option<Value>,
) -> Vec<T> {
    let Some(Value::Array(a)) = d.get(key) else { return Vec::new() };
    if a.items.is_empty() || a.items.len() > max {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(a.items.len());
    for item in &a.items {
        match read(item) {
            Some(t) if write(&t).is_some_and(|w| same_text(&w, item, version)) => out.push(t),
            _ => return Vec::new(),
        }
    }
    take(d, key);
    out
}

fn push_list<T>(e: &mut Vec<(Str, Value)>, key: &str, items: &[T], write: impl Fn(&T) -> Option<Value>, what: &str) -> Result<()> {
    if !items.is_empty() {
        let values = items.iter().map(|i| write(i).ok_or_else(|| bad(what, "a value is not a finite number"))).collect::<Result<_>>()?;
        e.push((key.into(), list(values)));
    }
    Ok(())
}

/// A number the model accepts: finite and within [`MAX_COORDINATE`]. Anything else keeps its
/// key as written.
pub(crate) fn number(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64).filter(|f| in_range(*f))
}

fn in_range(f: f64) -> bool {
    f.is_finite() && f.abs() <= MAX_COORDINATE
}

/// Whether the model accepts an anchor's position (a background with one that it does not stays
/// as written).
pub(crate) fn anchor_in_range(a: &Anchor) -> bool {
    in_range(a.x) && in_range(a.y)
}

pub(crate) fn take_text(d: &mut Dict, key: &str, version: u32) -> Option<String> {
    take_same(d, key, version, |v| str_of(v).filter(|s| !s.is_empty()), |s: &String| Some(string(s)))
}

pub(crate) fn take_flag(d: &mut Dict, key: &str, version: u32) -> bool {
    take_same(d, key, version, |v| matches!(v.as_num(), Some(Num::Int(1))).then_some(()), |_| Some(int(1))).is_some()
}

fn compact(items: Vec<Value>) -> Value {
    Value::Array(Array { items, layout: Layout::Compact, ..Default::default() })
}

// ── attributes ─────────────────────────────────────────────────────────────────────────────────

fn read_rule(v: &Value) -> Option<AxisRule> {
    let d = v.as_dict()?;
    if d.entries.iter().any(|(k, _)| !matches!(k.as_str(), "min" | "max")) {
        return None;
    }
    let end = |k: &str| match d.get(k) {
        Some(v) => number(v).map(Some),
        None => Some(None),
    };
    Some(AxisRule { min: end("min")?, max: end("max")? })
}

fn write_rule(r: &AxisRule) -> Option<Value> {
    let mut e = Vec::new();
    if let Some(m) = r.max {
        e.push(("max".into(), num(m, "axisRules").ok()?));
    }
    if let Some(m) = r.min {
        e.push(("min".into(), num(m, "axisRules").ok()?));
    }
    Some(Value::Dict(Dict { entries: e, layout: Layout::Multiline, ..Default::default() }))
}

fn read_attrs(v: &Value, version: u32) -> Option<LayerAttrs> {
    let mut d = v.as_dict()?.clone();
    if d.is_empty() {
        return None;
    }
    let mut a = LayerAttrs::default();
    if let Some(c) = take(&mut d, "coordinates") {
        let items = &c.as_array()?.items;
        if items.is_empty() || items.len() > MAX_AXIS_VALUES {
            return None;
        }
        a.coordinates =
            items.iter().map(|i| if version >= 4 && i.as_str() == Some("-") { Some(None) } else { number(i).map(Some) }).collect::<Option<_>>()?;
    }
    if let Some(r) = take(&mut d, "axisRules") {
        let items = &r.as_array()?.items;
        if items.is_empty() || items.len() > MAX_AXIS_VALUES {
            return None;
        }
        a.axis_rules = items.iter().map(read_rule).collect::<Option<_>>()?;
    }
    if let Some(p) = take(&mut d, "colorPalette") {
        a.color_palette = Some(match &p {
            Value::String(s) if s.as_str() == "*" => PaletteEntry::Foreground,
            _ => match p.as_num()? {
                Num::Int(i) => PaletteEntry::Index(u16::try_from(i).ok()?),
                Num::Float(_) => return None,
            },
        });
    }
    let flag = |d: &mut Dict, k: &str| -> Option<bool> {
        match take(d, k) {
            Some(v) => matches!(v.as_num(), Some(Num::Int(1))).then_some(true),
            None => Some(false),
        }
    };
    a.full_color = flag(&mut d, "color")?;
    a.svg = flag(&mut d, "svg")?;
    if let Some(s) = take(&mut d, "sbixSize") {
        a.sbix_size = match s.as_num()? {
            Num::Int(i) => Some(u16::try_from(i).ok()?),
            Num::Float(_) => return None,
        };
    }
    a.extra = into_extra(d);
    Some(a)
}

fn write_attrs(a: &LayerAttrs) -> Option<Value> {
    let mut e: Vec<(Str, Value)> = Vec::new();
    if !a.coordinates.is_empty() {
        let items = a.coordinates.iter().map(|c| match c {
            Some(v) => num(*v, "coordinates").ok(),
            None => Some(string("-")),
        });
        e.push(("coordinates".into(), list(items.collect::<Option<_>>()?)));
    }
    if !a.axis_rules.is_empty() {
        e.push(("axisRules".into(), list(a.axis_rules.iter().map(write_rule).collect::<Option<_>>()?)));
    }
    match a.color_palette {
        Some(PaletteEntry::Index(i)) => e.push(("colorPalette".into(), int(i))),
        Some(PaletteEntry::Foreground) => e.push(("colorPalette".into(), string("*"))),
        None => {}
    }
    if a.full_color {
        e.push(("color".into(), int(1)));
    }
    if let Some(s) = a.sbix_size {
        e.push(("sbixSize".into(), int(s)));
    }
    if a.svg {
        e.push(("svg".into(), int(1)));
    }
    Some(Value::Dict(finish(e, &a.extra)))
}

/// The attributes without the smart-glyph poles, which have a key of their own.
fn has_attr_keys(a: &LayerAttrs) -> bool {
    !LayerAttrs { smart_poles: Vec::new(), ..a.clone() }.is_empty()
}

fn read_poles(v: &Value) -> Option<Vec<SmartPole>> {
    let d = v.as_dict()?;
    if d.is_empty() || d.len() > MAX_PART_PROPERTIES {
        return None;
    }
    d.entries
        .iter()
        .map(|(k, v)| {
            let top = match v.as_num()? {
                Num::Int(1) => false,
                Num::Int(2) => true,
                _ => return None,
            };
            Some(SmartPole { property: k.as_str().to_string(), top })
        })
        .collect()
}

fn write_poles(poles: &[SmartPole]) -> Value {
    let entries = poles.iter().map(|p| (Str::new(p.property.as_str()), int(if p.top { 2 } else { 1 }))).collect();
    Value::Dict(Dict { entries, layout: Layout::Multiline, ..Default::default() })
}

/// The attributes a version 2 layer name says, as glyphsLib 6.15.0 reads them
/// (`GSLayer._is_brace_layer`, `_bracket_axis_rules`, `_color_palette_index`): coordinates between
/// the first `{` and `}` (not in a background's name); a bracket rule `[n]` (from n) or `]n]` (up
/// to n), the last one in the name, on the first axis; `Color n` or `Color *`. The reference app's
/// other colour layer names, which glyphsLib does not read, give the other colour attributes:
/// `Color` a full-colour layer, `iColor n` an sbix layer of n pixels per em, `svg` an svg layer.
pub(crate) fn v2_attrs(name: &str) -> LayerAttrs {
    let mut a = LayerAttrs::default();
    if !name.contains(".background")
        && let (Some(open), Some(close)) = (name.find('{'), name.find('}'))
        && let Some(inner) = name.get(open + 1..close)
        && let Some(c) = inner.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|f| f.is_finite()).map(Some)).collect::<Option<Vec<_>>>()
        && c.len() <= MAX_AXIS_VALUES
    {
        a.coordinates = c;
    }
    if let Some((reverse, value)) = v2_bracket(name) {
        a.axis_rules = vec![if reverse { AxisRule { min: None, max: Some(value) } } else { AxisRule { min: Some(value), max: None } }];
    }
    let trimmed = name.trim();
    match trimmed {
        "Color" => a.full_color = true,
        "svg" => a.svg = true,
        _ => {}
    }
    if let Some(rest) = trimmed.strip_prefix("iColor ")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        a.sbix_size = rest.parse::<u16>().ok();
    }
    if let Some(rest) = trimmed.strip_prefix("Color ") {
        if rest == "*" {
            a.color_palette = Some(PaletteEntry::Foreground);
        } else if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
            a.color_palette = rest.parse::<u16>().ok().map(PaletteEntry::Index);
        }
    }
    a
}

/// The last `[n]` or `]n]` in a name (spaces allowed around `n`): whether it opens with `]`, and
/// `n`.
fn v2_bracket(name: &str) -> Option<(bool, f64)> {
    let bytes = name.as_bytes();
    let mut found = None;
    for (i, b) in bytes.iter().enumerate() {
        if !matches!(b, b'[' | b']') {
            continue;
        }
        let rest = name.get(i + 1..)?;
        let trimmed = rest.trim_start();
        let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let after = trimmed.get(digits.len()..)?.trim_start();
        if after.starts_with(']')
            && let Ok(n) = digits.parse::<f64>()
        {
            found = Some((*b == b']', n));
        }
    }
    found
}

// ── annotations, images ────────────────────────────────────────────────────────────────────────

fn read_annotation(v: &Value, version: u32) -> Option<Annotation> {
    let mut d = v.as_dict()?.clone();
    let kind = str_of(&take(&mut d, "type")?)?;
    let (mut x, mut y) = (0.0, 0.0);
    if version == 2 {
        if let Some(p) = take(&mut d, "position") {
            let n = crate::v2::braced_numbers(p.as_str()?, 2)?;
            (x, y) = (*n.first()?, *n.get(1)?);
        }
    } else if let Some(p) = take(&mut d, "pos") {
        (x, y) = as_pair(&p, "pos").ok()?;
    }
    let angle = match take(&mut d, "angle") {
        Some(a) => number(&a)?,
        None => 0.0,
    };
    let text = match take(&mut d, "text") {
        Some(t) => Some(str_of(&t)?),
        None => None,
    };
    let width = match take(&mut d, "width") {
        Some(w) => number(&w)?,
        None => 0.0,
    };
    Some(Annotation { kind, x, y, angle, text, width, extra: into_extra(d) })
}

fn write_annotation(a: &Annotation, version: u32) -> Option<Value> {
    let mut e = vec![("type".into(), string(&a.kind))];
    if (a.x, a.y) != (0.0, 0.0) {
        if version == 2 {
            if !(a.x.is_finite() && a.y.is_finite()) {
                return None;
            }
            e.push(("position".into(), string(&crate::v2::braced(&[a.x, a.y]))));
        } else {
            e.push(("pos".into(), pair(a.x, a.y, "annotation").ok()?));
        }
    }
    if a.angle != 0.0 {
        e.push(("angle".into(), num(a.angle, "annotation").ok()?));
    }
    if let Some(t) = &a.text {
        e.push(("text".into(), string(t)));
    }
    if a.width != 0.0 {
        e.push(("width".into(), num(a.width, "annotation").ok()?));
    }
    Some(Value::Dict(finish(e, &a.extra)))
}

/// A version 2 crop, `"{{x, y}, {w, h}}"`.
fn read_crop_v2(s: &str) -> Option<[f64; 4]> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let (a, b) = inner.split_once("},")?;
    let p = crate::v2::braced_numbers(&format!("{a}}}"), 2)?;
    let q = crate::v2::braced_numbers(b.trim(), 2)?;
    Some([*p.first()?, *p.get(1)?, *q.first()?, *q.get(1)?])
}

fn write_crop_v2(c: &[f64; 4]) -> String {
    let [x, y, w, h] = *c;
    format!("{{{}, {}}}", crate::v2::braced(&[x, y]), crate::v2::braced(&[w, h]))
}

pub(crate) fn read_image(v: &Value, version: u32) -> Option<Image> {
    let mut d = v.as_dict()?.clone();
    let mut i = Image { path: take_text(&mut d, "imagePath", version), ..Default::default() };
    if version == 2 {
        if let Some(t) = take(&mut d, "transform") {
            let m: [f64; 6] = crate::v2::braced_numbers(t.as_str()?, 6)?.try_into().ok()?;
            i.transform = Transform::Matrix(m);
        } else {
            i.transform = Transform::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        }
        if let Some(c) = take(&mut d, "crop") {
            i.crop = Some(read_crop_v2(c.as_str()?)?);
        }
        if let Some(l) = take(&mut d, "locked") {
            i.locked = l.as_str() == Some("1");
            if !i.locked {
                return None;
            }
        }
    } else {
        let pair_or = |d: &mut Dict, k: &str, default: (f64, f64)| -> Option<(f64, f64)> {
            match take(d, k) {
                Some(v) => as_pair(&v, k).ok(),
                None => Some(default),
            }
        };
        let pos = pair_or(&mut d, "pos", (0.0, 0.0))?;
        let scale = pair_or(&mut d, "scale", (1.0, 1.0))?;
        let slant = pair_or(&mut d, "slant", (0.0, 0.0))?;
        let angle = match take(&mut d, "angle") {
            Some(a) => number(&a)?,
            None => 0.0,
        };
        i.transform = Transform::Decomposed { pos, scale, angle, slant };
        if let Some(c) = take(&mut d, "crop") {
            let items = &c.as_array()?.items;
            let [x, y, w, h] = items.as_slice() else { return None };
            i.crop = Some([number(x)?, number(y)?, number(w)?, number(h)?]);
        }
        i.locked = take_flag(&mut d, "locked", version);
    }
    if let Some(a) = take(&mut d, "alpha") {
        i.alpha = Some(number(&a)?);
    }
    i.extra = into_extra(d);
    Some(i)
}

pub(crate) fn write_image(i: &Image, version: u32) -> Option<Value> {
    let mut e = Vec::new();
    if let Some(p) = &i.path {
        e.push(("imagePath".into(), string(p)));
    }
    if version == 2 {
        if !i.transform.is_identity() {
            let m = i.transform.affine().as_coeffs();
            if m.iter().any(|f| !f.is_finite()) {
                return None;
            }
            e.push(("transform".into(), string(&crate::v2::braced(&m))));
        }
        if let Some(c) = &i.crop {
            if c.iter().any(|f| !f.is_finite()) {
                return None;
            }
            e.push(("crop".into(), string(&write_crop_v2(c))));
        }
        if i.locked {
            e.push(("locked".into(), string("1")));
        }
    } else {
        let (pos, scale, angle, slant) = match i.transform {
            Transform::Decomposed { pos, scale, angle, slant } => (pos, scale, angle, slant),
            Transform::Matrix(m) => crate::decompose(m)?,
        };
        if pos != (0.0, 0.0) {
            e.push(("pos".into(), pair(pos.0, pos.1, "image").ok()?));
        }
        if scale != (1.0, 1.0) {
            e.push(("scale".into(), pair(scale.0, scale.1, "image").ok()?));
        }
        if angle != 0.0 {
            e.push(("angle".into(), num(angle, "image").ok()?));
        }
        if slant != (0.0, 0.0) {
            e.push(("slant".into(), pair(slant.0, slant.1, "image").ok()?));
        }
        if let Some(c) = &i.crop {
            e.push(("crop".into(), compact(c.iter().map(|v| num(*v, "crop")).collect::<Result<_>>().ok()?)));
        }
        if i.locked {
            e.push(("locked".into(), int(1)));
        }
    }
    if let Some(a) = i.alpha {
        e.push(("alpha".into(), num(a, "alpha").ok()?));
    }
    Some(Value::Dict(finish(e, &i.extra)))
}

// ── guides, hints, annotations and image: what a layer and its background share ────────────────

/// What a layer and its background both hold besides shapes and anchors.
#[derive(Default)]
struct Content {
    guides: Vec<Guide>,
    hints: Vec<Hint>,
    annotations: Vec<Annotation>,
    image: Option<Image>,
}

fn guide_key(version: u32) -> &'static str {
    if version == 2 { "guideLines" } else { "guides" }
}

fn read_guide(v: &Value, version: u32) -> Option<Guide> {
    let g = if version == 2 { crate::v2::read_guide(v) } else { crate::design::read_guide(v) }?;
    (in_range(g.x) && in_range(g.y) && g.angle.is_finite()).then_some(g)
}

fn write_guide(g: &Guide, version: u32) -> Option<Value> {
    if version == 2 { crate::v2::write_guide(g) } else { crate::design::write_guide(g) }
}

fn take_content(d: &mut Dict, version: u32) -> Content {
    Content {
        guides: take_list(d, guide_key(version), version, MAX_LAYER_GUIDES, |v| read_guide(v, version), |g| write_guide(g, version)),
        hints: take_list(
            d,
            "hints",
            version,
            MAX_LAYER_HINTS,
            |v| Some(Hint { family: fontcraft_model::Family::Glyphs, value: v.clone() }),
            |h: &Hint| Some(h.value.clone()),
        ),
        annotations: take_list(d, "annotations", version, MAX_ANNOTATIONS, |v| read_annotation(v, version), |a| write_annotation(a, version)),
        image: take_same(d, "backgroundImage", version, |v| read_image(v, version), |i| write_image(i, version)),
    }
}

fn push_content(e: &mut Vec<(Str, Value)>, c: Content, version: u32, at: &str) -> Result<()> {
    push_list(e, guide_key(version), &c.guides, |g| write_guide(g, version), at)?;
    // Hints read from a UFO are converted; those of other formats have no place here, and
    // [`losses`] lists them.
    let hints: Vec<Value> = c.hints.iter().filter_map(|h| hint_value(h, version)).collect();
    push_list(e, "hints", &hints, |h| Some(h.clone()), at)?;
    push_list(e, "annotations", &c.annotations, |a| write_annotation(a, version), at)?;
    if let Some(i) = &c.image {
        e.push(("backgroundImage".into(), write_image(i, version).ok_or_else(|| bad(at, "an image value is not a finite number"))?));
    }
    Ok(())
}

/// A hint as this version writes it: as stored, or converted from the dictionary a UFO's
/// glyphsLib key holds ([`hint_from_ufo`]). `None` for a hint of another format.
fn hint_value(h: &Hint, version: u32) -> Option<Value> {
    match h.family {
        fontcraft_model::Family::Glyphs => Some(h.value.clone()),
        fontcraft_model::Family::Ufo => hint_from_ufo(&h.value, version),
        _ => None,
    }
}

/// A hint read from a UFO, glyphsLib's dictionary (`builder/hints.py`, Apache-2.0; see `NOTICE`),
/// in this version's form: keys sorted, defaults left out (`horizontal` false, `options` 0,
/// `stem` -2, an empty `name` or `type`), node index paths and points as lists, in version 2 as
/// brace strings. `None` when a key or value has no such form.
pub(crate) fn hint_from_ufo(v: &Value, version: u32) -> Option<Value> {
    let d = v.as_dict()?;
    let number = |n: Num| -> Option<Value> {
        match n {
            Num::Int(i) => Some(int(i)),
            Num::Float(f) => num(f, "hint").ok(),
        }
    };
    let numbers = |a: &Array| a.items.iter().map(|x| x.as_num().map(Num::as_f64).filter(|f| f.is_finite())).collect::<Option<Vec<f64>>>();
    let compact = |items: Vec<Value>| Value::Array(Array { items, layout: Layout::Compact, ..Default::default() });
    let mut e: Vec<(Str, Value)> = Vec::new();
    for (k, x) in &d.entries {
        let key = k.as_str();
        let item = match key {
            "horizontal" => x.as_bool()?.then(|| int(1)),
            "name" | "type" => Some(x.as_str()?).filter(|t| !t.is_empty()).map(string),
            "options" => match x.as_num()? {
                n if n.as_f64() == 0.0 => None,
                n => Some(number(n)?),
            },
            "stem" => match x.as_num()? {
                n if n.as_f64() == -2.0 => None,
                n => Some(number(n)?),
            },
            "origin" | "other1" | "other2" | "target" => Some(match x {
                Value::String(t) if key == "target" && matches!(t.as_str(), "up" | "down") => string(t.as_str()),
                // A side bearing, `origin = lsb;`.
                Value::Array(a) if matches!(a.items.as_slice(), [Value::String(_)]) => string(a.items.first()?.as_str()?),
                Value::Array(a) if version == 2 => string(&crate::v2::braced(&numbers(a)?)),
                Value::Array(a) => compact(
                    a.items
                        .iter()
                        .map(|i| match i {
                            Value::String(t) => Some(string(t.as_str())),
                            other => other.as_num().and_then(number),
                        })
                        .collect::<Option<Vec<_>>>()?,
                ),
                _ => return None,
            }),
            "place" | "scale" => {
                let n = numbers(x.as_array()?)?;
                Some(if version == 2 {
                    string(&crate::v2::braced(&n))
                } else {
                    compact(n.into_iter().map(|f| num(f, "hint").ok()).collect::<Option<Vec<_>>>()?)
                })
            }
            _ => return None,
        };
        if let Some(item) = item {
            e.push((key.into(), item));
        }
    }
    e.sort_by(|a, b| a.0.as_str().as_bytes().cmp(b.0.as_str().as_bytes()));
    Some(Value::Dict(Dict { entries: e, layout: Layout::Multiline, ..Default::default() }))
}

// ── background ─────────────────────────────────────────────────────────────────────────────────

fn read_background(v: &Value, version: u32, shapes: &Shapes) -> Option<Background> {
    let mut d = v.as_dict()?.clone();
    let (shape_list, anchors) = (shapes.take)(&mut d, version)?;
    let c = take_content(&mut d, version);
    Some(Background {
        shapes: shape_list,
        anchors,
        guides: c.guides,
        hints: c.hints,
        annotations: c.annotations,
        image: c.image,
        extra: into_extra(d),
    })
}

fn write_background(b: &Background, version: u32, shapes: &Shapes, at: &str) -> Result<Value> {
    let mut e = Vec::new();
    (shapes.push)(&mut e, &b.shapes, &b.anchors, at, version)?;
    let c = Content { guides: b.guides.clone(), hints: b.hints.clone(), annotations: b.annotations.clone(), image: b.image.clone() };
    push_content(&mut e, c, version, at)?;
    Ok(Value::Dict((shapes.finish)(e, &b.extra)))
}

// ── the layer ──────────────────────────────────────────────────────────────────────────────────

/// The layer keys a version 2 file names metrics keys with, in [`MetricsKeys::all`] order.
const METRIC_KEYS_V2: [Option<&str>; 7] = [Some("leftMetricsKey"), Some("rightMetricsKey"), Some("widthMetricsKey"), None, None, None, None];

fn metric_keys(version: u32) -> [Option<&'static str>; 7] {
    if version == 2 { METRIC_KEYS_V2 } else { crate::glyph::METRIC_KEYS }
}

/// Version 2's user data key for the smart-glyph poles.
const PART_SELECTION: &str = "PartSelection";

/// The user data with the poles under [`PART_SELECTION`], placed where a sorted dictionary has
/// it.
fn user_data_with_poles(rest: &Dict, poles: &[SmartPole]) -> Dict {
    let mut d = rest.clone();
    let at = d.entries.iter().position(|(k, _)| k.as_str().as_bytes() > PART_SELECTION.as_bytes()).unwrap_or(d.entries.len());
    d.insert_at(at, PART_SELECTION, write_poles(poles));
    d
}

/// The layer's data, taken out of its dictionary.
pub(crate) fn take_layer_info(d: &mut Dict, version: u32, shapes: &Shapes) -> LayerInfo {
    let mut info = LayerInfo { name: take_text(d, "name", version), ..Default::default() };
    if version == 2 {
        info.attrs = info.name.as_deref().map(v2_attrs).unwrap_or_default();
    } else {
        info.attrs = take_relaid(d, "attr", version, |v| read_attrs(v, version), write_attrs).unwrap_or_default();
        info.attrs.smart_poles = take_same(d, "partSelection", version, read_poles, |p: &Vec<SmartPole>| Some(write_poles(p))).unwrap_or_default();
    }
    info.vert_width = take_same(d, "vertWidth", version, number, |v| num(*v, "vertWidth").ok());
    info.vert_origin = take_same(d, "vertOrigin", version, number, |v| num(*v, "vertOrigin").ok());
    let mut keys = [None, None, None, None, None, None, None];
    for (slot, key) in keys.iter_mut().zip(metric_keys(version)) {
        *slot = key.and_then(|k| take_text(d, k, version));
    }
    let [left, right, width, top, bottom, vert_width, vert_origin] = keys;
    info.metrics_keys = MetricsKeys { left, right, width, top, bottom, vert_width, vert_origin };
    info.color = take_same(d, "color", version, |v| crate::glyph::read_color(v, version), |c| Some(crate::glyph::write_color(c, version)));
    info.visible = take_flag(d, "visible", version);
    let c = take_content(d, version);
    (info.guides, info.hints, info.annotations, info.image) = (c.guides, c.hints, c.annotations, c.image);
    info.background =
        take_same(d, "background", version, |v| read_background(v, version, shapes), |b| write_background(b, version, shapes, "background").ok());
    info.user_data = crate::info::take_user_data(d);
    if version == 2
        && let Some(poles) = info.user_data.get(PART_SELECTION).and_then(read_poles)
    {
        let mut rest = info.user_data.0.clone();
        rest.entries.retain(|(k, _)| k.as_str() != PART_SELECTION);
        if same_text(&Value::Dict(user_data_with_poles(&rest, &poles)), &Value::Dict(info.user_data.0.clone()), version) {
            info.attrs.smart_poles = poles;
            info.user_data.0 = rest;
        }
    }
    info
}

/// The layer's data as entries of its dictionary.
pub(crate) fn push_layer_info(e: &mut Vec<(Str, Value)>, info: &LayerInfo, version: u32, shapes: &Shapes, at: &str) -> Result<()> {
    if let Some(n) = info.name.as_deref().filter(|n| !n.is_empty()) {
        e.push(("name".into(), string(n)));
    }
    if version != 2 {
        if has_attr_keys(&info.attrs) {
            e.push(("attr".into(), write_attrs(&info.attrs).ok_or_else(|| bad(at, "a layer attribute is not a finite number"))?));
        }
        if !info.attrs.smart_poles.is_empty() {
            e.push(("partSelection".into(), write_poles(&info.attrs.smart_poles)));
        }
    }
    if let Some(v) = info.vert_width {
        e.push(("vertWidth".into(), num(v, at)?));
    }
    if let Some(v) = info.vert_origin {
        e.push(("vertOrigin".into(), num(v, at)?));
    }
    for (key, (_, value)) in metric_keys(version).into_iter().zip(info.metrics_keys.all()) {
        if let (Some(key), Some(v)) = (key, value.as_deref().filter(|v| !v.is_empty())) {
            e.push((key.into(), string(v)));
        }
    }
    if let Some(c) = &info.color {
        e.push(("color".into(), crate::glyph::write_color(c, version)));
    }
    if info.visible {
        e.push(("visible".into(), int(1)));
    }
    let c = Content { guides: info.guides.clone(), hints: info.hints.clone(), annotations: info.annotations.clone(), image: info.image.clone() };
    push_content(e, c, version, at)?;
    if let Some(b) = &info.background {
        e.push(("background".into(), write_background(b, version, shapes, at)?));
    }
    if version == 2 && !info.attrs.smart_poles.is_empty() {
        let d = user_data_with_poles(&info.user_data.0, &info.attrs.smart_poles);
        e.push(("userData".into(), Value::Dict(d)));
    } else {
        crate::info::push_user_data(e, &info.user_data);
    }
    Ok(())
}

/// What a version cannot hold of the layers' data, one line each.
pub(crate) fn losses(glyphs: &[std::sync::Arc<fontcraft_model::Glyph>], version: u32) -> Vec<String> {
    let layers = || glyphs.iter().flat_map(|g| g.layers.iter());
    let mut out = Vec::new();
    if version == 2 {
        let n = layers()
            .filter(|l| {
                let said = l.info.name.as_deref().map(v2_attrs).unwrap_or_default();
                LayerAttrs { smart_poles: Vec::new(), ..l.info.attrs.clone() } != said
            })
            .count();
        if n > 0 {
            out.push(format!("layer attributes that the layers' names do not say ({n})"));
        }
        let keys = layers().filter(|l| l.info.metrics_keys.all().iter().skip(3).any(|(_, v)| v.is_some())).count();
        if keys > 0 {
            out.push(format!("the layers' top, bottom and vertical metrics keys ({keys})"));
        }
    }
    let foreign = layers()
        .filter(|l| {
            let bg = l.info.background.as_ref();
            l.info.hints.iter().chain(bg.into_iter().flat_map(|b| b.hints.iter())).any(|h| hint_value(h, version).is_none())
        })
        .count();
    if foreign > 0 {
        out.push(format!("hints read from another format ({foreign} layers)"));
    }
    if version < 4 {
        let n = layers().filter(|l| l.info.color.as_ref().is_some_and(crate::glyph::color_needs_floats)).count();
        if n > 0 {
            out.push(format!("the layers' colours, rounded to 8 bits ({n})"));
        }
        let dashes = layers().filter(|l| l.info.attrs.coordinates.iter().any(Option::is_none)).count();
        if dashes > 0 && version == 3 {
            out.push(format!("intermediate-layer coordinates that use the master's value ({dashes})"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
