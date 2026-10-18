//! glyphsLib's lib keys for the data of glyphs and layers that a GLIF file has no element for, and
//! the layer libs of `layerinfo.plist`. The shapes' keys are in [`crate::shapekeys`].
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0, see `NOTICE`): `builder/glyph.py` (the glyph keys,
//! metrics keys, the width of non-spacing marks, the colour layer mapping), `builder/layers.py`
//! (layer ids and order), `builder/user_data.py` (layer libs), `builder/hints.py`,
//! `builder/annotations.py`, `builder/background_image.py`, `builder/components.py` (the smart
//! glyph's axes) and `builder/constants.py`.
//!
//! | Model | Key (GLIF lib unless stated) |
//! |---|---|
//! | the glyph's category, subcategory and script | `com.schriftgestaltung.Glyphs.category`, `…subCategory`, `…script` |
//! | the glyph's export flag | `…Glyphs.Export` (read; written where the file had it: the writer uses `public.skipExportGlyphs`) |
//! | the glyph's last change | `…Glyphs.lastChange`, `2026/10/08 12:00:00` for the model's `2026-10-08 12:00:00 +0000` |
//! | the glyph's left, right and width metrics keys | `…Glyphs.glyph.leftMetricsKey`, `…rightMetricsKey`, `…widthMetricsKey` |
//! | the smart glyph's properties | `com.schriftgestaltung.smartComponentAxes` |
//! | the layer's left, right and width metrics keys | `…Glyphs.layer.leftMetricsKey`… |
//! | hints | `com.schriftgestaltung.hints`: glyphsLib's dictionaries; read as hints of the UFO family |
//! | annotations | `com.schriftgestaltung.annotations` |
//! | the image's crop, lock and opacity | `com.schriftgestaltung.backgroundImage.crop`, `…locked`, `…alpha` |
//! | the width of a non-spacing mark | the GLIF width 0 and `…Glyphs.originalWidth` |
//! | colour palette layers | the master layer's `com.github.googlei18n.ufo2ft.colorLayerMapping`: `[UFO layer, palette index]` |
//! | a background's width (the background's user data, from a `.glyphs` file) | the background GLIF's width |
//! | layer ids and order | `layerinfo.plist` lib `com.schriftgestaltung.layerId`, `…layerOrderInGlyph.<glyph>` |
//! | a UFO layer's lib | the master's user data `com.schriftgestaltung.Glyphs.layerLib.<layer>` (`public.default` for the default layer, whose own name goes in `…Glyphs.layerName`) |
//!
//! Glyph keys go to every layer's GLIF, as glyphsLib writes them: always to the default master's
//! master layer, to the other GLIFs when they are new or had the key when read. Keys glyphsLib
//! writes whatever their value (the image's three) follow the rule of [`crate::libkeys`]: a new
//! UFO of a document with a `.glyphs` format version gets them, a GLIF read from disk keeps the
//! ones it had.
//!
//! **Reading is checked by writing**, as in [`crate::libkeys`]: [`read`] types a key only when
//! the writer gives back the same value for it; the others stay user data, written as they are.

use std::collections::{BTreeMap, HashSet};

use fontcraft_model::{
    Annotation, Family, Font, Glyph, Hint, Layer, MAX_ANNOTATIONS, MAX_COORDINATE, MAX_LAYER_HINTS, MAX_PART_PROPERTIES, Master, PaletteEntry,
    PartProperty, UserData,
};
use fontcraft_plist::{Array, Dict, Num, Number, Str, Value};

use crate::libkeys::sorted_keys;
use crate::shapekeys;

pub(crate) const CATEGORY: &str = "com.schriftgestaltung.Glyphs.category";
const SUB_CATEGORY: &str = "com.schriftgestaltung.Glyphs.subCategory";
const SCRIPT: &str = "com.schriftgestaltung.Glyphs.script";
pub(crate) const EXPORT: &str = "com.schriftgestaltung.Glyphs.Export";
const LAST_CHANGE: &str = "com.schriftgestaltung.Glyphs.lastChange";
const ORIGINAL_WIDTH: &str = "com.schriftgestaltung.Glyphs.originalWidth";
const GLYPH_METRICS: [&str; 3] = [
    "com.schriftgestaltung.Glyphs.glyph.leftMetricsKey",
    "com.schriftgestaltung.Glyphs.glyph.rightMetricsKey",
    "com.schriftgestaltung.Glyphs.glyph.widthMetricsKey",
];
const LAYER_METRICS: [&str; 3] = [
    "com.schriftgestaltung.Glyphs.layer.leftMetricsKey",
    "com.schriftgestaltung.Glyphs.layer.rightMetricsKey",
    "com.schriftgestaltung.Glyphs.layer.widthMetricsKey",
];
const SMART_AXES: &str = "com.schriftgestaltung.smartComponentAxes";
const HINTS: &str = "com.schriftgestaltung.hints";
const ANNOTATIONS: &str = "com.schriftgestaltung.annotations";
const CROP: &str = "com.schriftgestaltung.backgroundImage.crop";
const LOCKED: &str = "com.schriftgestaltung.backgroundImage.locked";
const ALPHA: &str = "com.schriftgestaltung.backgroundImage.alpha";
const COLOR_LAYER_MAPPING: &str = "com.github.googlei18n.ufo2ft.colorLayerMapping";
/// A background's width in its user data (a `.glyphs` file glyphsLib made from a UFO).
const BACKGROUND_WIDTH: &str = "com.schriftgestaltung.Glyphs.backgroundWidth";
/// `layerinfo.plist` lib keys.
const LAYER_ID: &str = "com.schriftgestaltung.layerId";
const LAYER_ORDER: &str = "com.schriftgestaltung.layerOrderInGlyph.";
/// Master user data keys glyphsLib keeps UFO layer libs under, and the default layer's name.
pub(crate) const LAYER_LIB: &str = "com.schriftgestaltung.Glyphs.layerLib.";
const LAYER_NAME: &str = "com.schriftgestaltung.Glyphs.layerName";
const DEFAULT_LAYER: &str = "public.default";
/// A background whose glyphsLib keys stay its lib's user data (one of them did not read back).
const UNTYPED: &str = "untypedLib";
/// The keys of a background's GLIF lib this module and [`crate::shapekeys`] type.
const BACKGROUND_KEYS: [&str; 3] = [CROP, LOCKED, ALPHA];
/// glyphsLib's default image opacity, in percent.
const DEFAULT_ALPHA: f64 = 50.0;
/// glyphsLib's default annotation width.
const DEFAULT_ANNOTATION_WIDTH: f64 = 100.0;
/// Passes [`read`] makes for one glyph before it gives up typing.
const MAX_PASSES: usize = 8;

pub(crate) fn string(s: &str) -> Value {
    Value::String(Str::new(s))
}

pub(crate) fn int(i: i64) -> Value {
    Value::Number(Number::int(i))
}

pub(crate) fn number(v: f64) -> Value {
    crate::fontinfo::number(v)
}

pub(crate) fn array(items: Vec<Value>) -> Value {
    Value::Array(Array { items, ..Default::default() })
}

/// A dictionary of these entries, sorted by key as fontTools writes it.
pub(crate) fn dict(entries: Vec<(&str, Value)>) -> Value {
    let mut d = Dict::new();
    for (k, v) in entries {
        d.insert(k, v);
    }
    sorted_keys(Value::Dict(d))
}

/// A finite number within the model's coordinate limit.
pub(crate) fn coordinate(v: &Value) -> Option<f64> {
    v.as_num().map(Num::as_f64).filter(|f| f.is_finite() && f.abs() <= MAX_COORDINATE)
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.value().as_f64() != 0.0,
        Value::String(s) => !s.as_str().is_empty() && s.as_str() != "0",
        _ => false,
    }
}

// ── conversions ────────────────────────────────────────────────────────────────────────────────

fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// `HH:MM:SS`.
fn is_time(t: &str) -> bool {
    let p: Vec<&str> = t.split(':').collect();
    p.len() == 3 && p.iter().all(|x| digits(x, 2))
}

/// The model's `2026-10-08 12:00:00 +0000` as glyphsLib's UFO time, `2026/10/08 12:00:00`. Other
/// time zones have no UFO form.
pub(crate) fn last_change_to_ufo(s: &str) -> Option<String> {
    let (date, time) = s.strip_suffix(" +0000")?.split_once(' ')?;
    let d: Vec<&str> = date.split('-').collect();
    let [y, m, day] = d.as_slice() else { return None };
    (digits(y, 4) && digits(m, 2) && digits(day, 2) && is_time(time)).then(|| format!("{y}/{m}/{day} {time}"))
}

fn last_change_to_model(s: &str) -> Option<String> {
    let (date, time) = s.split_once(' ')?;
    let d: Vec<&str> = date.split('/').collect();
    let [y, m, day] = d.as_slice() else { return None };
    (digits(y, 4) && digits(m, 2) && digits(day, 2) && is_time(time)).then(|| format!("{y}-{m}-{day} {time} +0000"))
}

/// Whether the font asks for no last-change dates (the `Disable Last Change` parameter).
fn last_change_disabled(font: &Font) -> bool {
    font.info.params.get("Disable Last Change").is_some_and(truthy)
}

fn smart_axes_to_ufo(parts: &[PartProperty]) -> Value {
    array(
        parts
            .iter()
            .map(|p| {
                dict(vec![
                    ("bottomName", string(p.bottom_name.as_deref().unwrap_or(""))),
                    ("bottomValue", number(p.bottom_value)),
                    ("name", string(&p.name)),
                    ("topName", string(p.top_name.as_deref().unwrap_or(""))),
                    ("topValue", number(p.top_value)),
                ])
            })
            .collect(),
    )
}

fn smart_axes_to_model(v: &Value) -> Option<Vec<PartProperty>> {
    let items = &v.as_array()?.items;
    if items.len() > MAX_PART_PROPERTIES {
        return None;
    }
    items
        .iter()
        .map(|a| {
            let d = a.as_dict()?;
            let name_of = |k: &str| d.get(k).and_then(Value::as_str).map(|s| (!s.is_empty()).then(|| s.to_string()));
            Some(PartProperty {
                name: d.get("name")?.as_str()?.to_string(),
                bottom_value: coordinate(d.get("bottomValue")?)?,
                top_value: coordinate(d.get("topValue")?)?,
                bottom_name: name_of("bottomName")?,
                top_name: name_of("topName")?,
                extra: Default::default(),
            })
        })
        .collect()
}

/// A version 2 index path or point, `"{0, 3}"`, as its numbers.
fn braced(s: &str) -> Option<Vec<f64>> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    inner.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|f| f.is_finite())).collect()
}

/// An index path (`origin`, `other1`, `other2`, `target`) as glyphsLib's list: integers, or a
/// side-bearing name (`lsb`, `rsb`), from a version 3 list or a version 2 brace string.
fn index_path(v: &Value) -> Option<Value> {
    let index = |f: f64| {
        // Whole and small, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        (f.fract() == 0.0 && f.abs() < 1e9).then(|| int(f as i64))
    };
    let items = match v {
        Value::Array(a) => a
            .items
            .iter()
            .map(|x| match x {
                Value::String(s) => Some(string(s.as_str())),
                other => other.as_num().map(Num::as_f64).and_then(index),
            })
            .collect::<Option<Vec<_>>>()?,
        // A side bearing (`origin = lsb;`) is a path of one name, as glyphsLib reads it.
        Value::String(s) if !s.as_str().trim_start().starts_with('{') => vec![string(s.as_str())],
        Value::String(s) => braced(s.as_str())?.into_iter().map(index).collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    Some(array(items))
}

/// A point (`place`, `scale`) as glyphsLib's list of numbers.
fn point(v: &Value) -> Option<Value> {
    let numbers: Vec<f64> = match v {
        Value::Array(a) => a.items.iter().map(|x| x.as_num().map(Num::as_f64).filter(|f| f.is_finite())).collect::<Option<_>>()?,
        Value::String(s) => braced(s.as_str())?,
        _ => return None,
    };
    Some(array(numbers.into_iter().map(number).collect()))
}

/// A hint as glyphsLib's dictionary (`builder/hints.py`): `horizontal`, `options`, `stem`, `type`
/// and `name` always, the index paths and points it has. A hint read from a UFO is written as it
/// was read. `None` when it has no such form; the flag is false when the dictionary leaves out
/// part of the hint (`settings`, keys glyphsLib does not know, values it cannot read).
pub fn hint_to_ufo(h: &Hint) -> Option<(Value, bool)> {
    let d = h.value.as_dict()?;
    if h.family == Family::Ufo {
        return Some((h.value.without_spellings(), true));
    }
    if h.family != Family::Glyphs {
        return None;
    }
    let mut complete = true;
    let mut out: Vec<(&str, Value)> = Vec::new();
    out.push(("horizontal", Value::Bool(d.get("horizontal").is_some_and(truthy))));
    let num = |k: &str, default: i64| -> Option<Value> {
        match d.get(k) {
            None => Some(int(default)),
            Some(v) => v.as_num().map(Num::as_f64).filter(|f| f.is_finite()).map(number),
        }
    };
    out.push(("options", num("options", 0)?));
    out.push(("stem", num("stem", -2)?));
    out.push(("type", string(d.get("type").and_then(Value::as_str).unwrap_or(""))));
    out.push(("name", string(d.get("name").and_then(Value::as_str).unwrap_or(""))));
    for k in ["origin", "other1", "other2", "target"] {
        let Some(v) = d.get(k) else { continue };
        match v.as_str() {
            Some(t @ ("up" | "down")) if k == "target" => out.push((k, string(t))),
            _ => match index_path(v) {
                Some(p) => out.push((k, p)),
                None => complete = false,
            },
        }
    }
    for k in ["place", "scale"] {
        let Some(v) = d.get(k) else { continue };
        match point(v) {
            Some(p) => out.push((k, p)),
            None => complete = false,
        }
    }
    const KNOWN: [&str; 11] = ["horizontal", "name", "options", "origin", "other1", "other2", "place", "scale", "stem", "target", "type"];
    if d.entries.iter().any(|(k, _)| !KNOWN.contains(&k.as_str())) {
        complete = false;
    }
    Some((dict(out), complete))
}

/// A UFO hint dictionary glyphsLib's reader takes: the five keys it reads always, of their types.
fn hint_from_ufo(v: &Value) -> Option<Hint> {
    let d = v.as_dict()?;
    let ok = d.get("horizontal")?.as_bool().is_some()
        && d.get("options")?.as_num().is_some()
        && d.get("stem")?.as_num().is_some()
        && d.get("type")?.as_str().is_some()
        && d.get("name")?.as_str().is_some();
    ok.then(|| Hint { family: Family::Ufo, value: v.without_spellings() })
}

/// An annotation as glyphsLib's dictionary: `position` always, `type` and `width` (100 when the
/// model has none), `angle` and `text` when set.
fn annotation_to_ufo(a: &Annotation) -> Value {
    let mut e: Vec<(&str, Value)> = Vec::new();
    if a.angle != 0.0 {
        e.push(("angle", number(a.angle)));
    }
    e.push(("position", array(vec![number(a.x), number(a.y)])));
    if let Some(t) = a.text.as_deref().filter(|t| !t.is_empty()) {
        e.push(("text", string(t)));
    }
    if !a.kind.is_empty() {
        e.push(("type", string(&a.kind)));
    }
    e.push(("width", number(if a.width == 0.0 { DEFAULT_ANNOTATION_WIDTH } else { a.width })));
    dict(e)
}

fn annotation_from_ufo(v: &Value) -> Option<Annotation> {
    let d = v.as_dict()?;
    if d.entries.iter().any(|(k, _)| !["angle", "position", "text", "type", "width"].contains(&k.as_str())) {
        return None;
    }
    let (x, y) = match d.get("position")? {
        Value::Array(a) => match a.items.as_slice() {
            [x, y] => (coordinate(x)?, coordinate(y)?),
            _ => return None,
        },
        Value::String(s) => match braced(s.as_str())?.as_slice() {
            [x, y] => (*x, *y),
            _ => return None,
        },
        _ => return None,
    };
    let angle = match d.get("angle") {
        Some(a) => a.as_num()?.as_f64(),
        None => 0.0,
    };
    let width = match d.get("width") {
        Some(w) => w.as_num()?.as_f64(),
        None => 0.0,
    };
    Some(Annotation {
        kind: d.get("type").map(|t| t.as_str().map(str::to_string)).unwrap_or(Some(String::new()))?,
        x,
        y,
        angle,
        text: d.get("text").map(text).unwrap_or(None),
        width: if width == DEFAULT_ANNOTATION_WIDTH { 0.0 } else { width },
        extra: Default::default(),
    })
    .filter(|a| a.angle.is_finite() && a.width.is_finite())
}

/// The colour layer mapping of a master layer: each colour palette layer of its master, in the
/// glyph's order, with the UFO layer it goes to and its palette index (65535 for the text colour).
fn color_mapping(font: &Font, glyph: &Glyph, layer: &Layer) -> Option<Value> {
    if !layer.is_master_layer() {
        return None;
    }
    let items: Vec<Value> = glyph
        .layers
        .iter()
        .filter(|l| !l.is_master_layer() && l.master == layer.master)
        .filter_map(|l| {
            let index = match l.info.attrs.color_palette? {
                PaletteEntry::Index(i) => i,
                PaletteEntry::Foreground => u16::MAX,
            };
            let name = crate::ufo::ufo_layer_name(font, l)?;
            Some(Value::Array(Array { items: vec![string(&name), int(i64::from(index))], ..Default::default() }))
        })
        .collect();
    (!items.is_empty()).then(|| array(items))
}

/// Whether the glyph says it is a non-spacing mark, whose width glyphsLib writes as 0 with the
/// width in `originalWidth`.
fn is_nonspacing_mark(g: &Glyph) -> bool {
    g.info.category.as_deref() == Some("Mark") && g.info.sub_category.as_deref() == Some("Nonspacing")
}

/// Whether a width read as 0 with an `originalWidth` is that width: the glyph's classification,
/// where it states one, is a non-spacing mark's (glyphsLib also asks its glyph data, which the
/// model does not have yet).
fn may_be_nonspacing_mark(i: &fontcraft_model::GlyphInfo) -> bool {
    i.category.as_deref().is_none_or(|c| c == "Mark") && i.sub_category.as_deref().is_none_or(|c| c == "Nonspacing")
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// What writing one GLIF file needs.
pub(crate) struct Glif<'a> {
    pub font: &'a Font,
    pub glyph: &'a Glyph,
    /// The layer written; for a background, the layer the UFO writer makes of it.
    pub layer: &'a Layer,
    pub background: bool,
    /// The GLIF holds the glyph's own data: the master layer of the document's default master.
    pub primary: bool,
    /// The file was not read from disk.
    pub new: bool,
    /// The GLIF format (identifiers need 2).
    pub format: u32,
}

/// The lib entries glyphsLib's keys give one GLIF, and how its outline is written.
#[derive(Debug, Default)]
pub(crate) struct Written {
    pub map: BTreeMap<String, Value>,
    /// The contours are written before the components (the order is in `shapeOrder`).
    pub paths_first: bool,
    /// The width is written as 0 (it is in `originalWidth`).
    pub zero_width: bool,
    /// Identifiers given to anchors that have user data and none of their own, by anchor index.
    pub anchor_ids: Vec<(usize, String)>,
}

impl Written {
    pub(crate) fn put(&mut self, key: &str, v: Value) {
        self.map.insert(key.to_string(), v);
    }
}

impl Glif<'_> {
    /// The GLIF's lib as read.
    pub(crate) fn kept(&self) -> Option<&Dict> {
        self.layer.extra.get(Family::Ufo, "lib").and_then(Value::as_dict)
    }

    pub(crate) fn had(&self, key: &str) -> bool {
        self.kept().is_some_and(|d| d.get(key).is_some())
    }

    /// The layer's user data holds `key`, which then is written as it says.
    pub(crate) fn user(&self, key: &str) -> bool {
        self.layer.info.user_data.get(key).is_some()
    }

    /// A key of the glyph goes to this file (glyphsLib writes it to every layer's).
    fn glyph_level(&self, key: &str) -> bool {
        self.primary || self.new || self.had(key)
    }

    /// A key glyphsLib writes whatever its value goes to this file with its default value.
    pub(crate) fn defaults(&self, key: &str) -> bool {
        self.new && self.font.source.glyphs_version.is_some() || self.had(key)
    }
}

/// The lib entries of one GLIF.
pub(crate) fn entries(c: &Glif<'_>) -> Written {
    let mut w = Written::default();
    let (g, l) = (c.glyph, c.layer);
    if !c.background {
        let i = &*g.info;
        for (key, v) in [(CATEGORY, &i.category), (SUB_CATEGORY, &i.sub_category), (SCRIPT, &i.script)] {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty())
                && c.glyph_level(key)
            {
                w.put(key, string(v));
            }
        }
        if c.had(EXPORT) {
            w.put(EXPORT, Value::Bool(g.export));
        }
        if let Some(t) = i.last_change.as_deref().and_then(last_change_to_ufo)
            && !last_change_disabled(c.font)
            && c.glyph_level(LAST_CHANGE)
        {
            w.put(LAST_CHANGE, string(&t));
        }
        let keys = &i.metrics_keys;
        for (key, v) in GLYPH_METRICS.iter().zip([&keys.left, &keys.right, &keys.width]) {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty())
                && c.glyph_level(key)
            {
                w.put(key, string(v));
            }
        }
        if !i.parts.is_empty() && c.glyph_level(SMART_AXES) {
            w.put(SMART_AXES, smart_axes_to_ufo(&i.parts));
        }
        let li = &*l.info;
        let keys = &li.metrics_keys;
        for (key, v) in LAYER_METRICS.iter().zip([&keys.left, &keys.right, &keys.width]) {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                w.put(key, string(v));
            }
        }
        let hints: Vec<Value> = li.hints.iter().filter_map(hint_to_ufo).map(|(v, _)| v).collect();
        if !hints.is_empty() {
            w.put(HINTS, array(hints));
        }
        if !li.annotations.is_empty() {
            w.put(ANNOTATIONS, array(li.annotations.iter().map(annotation_to_ufo).collect()));
        }
        if l.width > 0.0 && !c.user(ORIGINAL_WIDTH) && (c.had(ORIGINAL_WIDTH) || c.new && is_nonspacing_mark(g)) {
            w.zero_width = true;
            w.put(ORIGINAL_WIDTH, number(l.width));
        }
        if let Some(m) = color_mapping(c.font, g, l) {
            w.put(COLOR_LAYER_MAPPING, m);
        }
    }
    if let Some(img) = &l.info.image {
        let crop = img.crop.unwrap_or_default();
        if img.crop.is_some() || c.defaults(CROP) {
            w.put(CROP, array(crop.iter().map(|v| number(*v)).collect()));
        }
        if img.locked || c.defaults(LOCKED) {
            w.put(LOCKED, Value::Bool(img.locked));
        }
        if img.alpha.is_some() || c.defaults(ALPHA) {
            w.put(ALPHA, number(img.alpha.unwrap_or(DEFAULT_ALPHA)));
        }
    }
    shapekeys::entries(c, &mut w);
    w
}

/// The width of a background's GLIF that a `.glyphs` file states in the background's user data.
pub(crate) fn background_width(b: &fontcraft_model::Background) -> Option<f64> {
    b.extra.get(Family::Glyphs, "userData").and_then(Value::as_dict).and_then(|d| d.get(BACKGROUND_WIDTH)).and_then(coordinate)
}

/// The keys of a background's lib that are not its user data: those [`read`] typed.
pub(crate) fn background_typed(b: &fontcraft_model::Background, key: &str) -> bool {
    b.extra.get(Family::Ufo, UNTYPED).is_none() && (BACKGROUND_KEYS.contains(&key) || shapekeys::is_background_key(key))
}

// ── layerinfo.plist ────────────────────────────────────────────────────────────────────────────

/// The id the UFO reader gives a layer of the UFO layer `name` in the master `master`.
pub(crate) fn synthetic_id(master: &str, name: &str) -> String {
    format!("{master}#{name}")
}

fn usable_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 255 && !id.chars().any(char::is_control)
}

/// Whether the glyph's layer order needs `layerOrderInGlyph` keys in a new UFO: the reader puts
/// the master layers first, in the masters' order, and then the other layers as it finds them,
/// so a glyph whose UFOs hold other layers, or whose master layers are not in that order, gets
/// them.
pub(crate) fn needs_order(font: &Font, g: &Glyph) -> bool {
    let layer_master = |id: &str| font.masters.iter().any(|m| m.id == id && m.extra.get(Family::Designspace, "layer").is_some());
    if g.layers.iter().any(|l| !l.is_master_layer() && !layer_master(&l.master) && crate::ufo::ufo_layer_name(font, l).is_some()) {
        return true;
    }
    let rank = |l: &Layer| font.masters.iter().position(|m| m.id == l.id);
    let ranks: Vec<Option<usize>> = g.layers.iter().filter(|l| l.is_master_layer()).map(rank).collect();
    ranks.windows(2).any(|w| matches!(w, [Some(a), Some(b)] if a > b))
}

/// The key under which a master's user data holds the lib of its UFO layer `name`.
fn layer_lib_key(name: &str, is_default: bool) -> String {
    format!("{LAYER_LIB}{}", if is_default { DEFAULT_LAYER } else { name })
}

/// A key glyphsLib leaves out of a layer lib's user data.
fn reserved(key: &str) -> bool {
    key.starts_with("com.schriftgestaltung.") || key.starts_with("public.")
}

/// The name a new UFO gives its default layer: the one a master's user data keeps under
/// `layerLib.public.default` (`…layerName`), else `public.default`.
pub(crate) fn default_layer_name(m: &Master) -> String {
    m.user_data
        .get(&layer_lib_key(DEFAULT_LAYER, true))
        .and_then(Value::as_dict)
        .and_then(|d| d.get(LAYER_NAME))
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
        .unwrap_or(DEFAULT_LAYER)
        .to_string()
}

/// Whether the master's user data key `key` goes to a UFO layer's lib rather than to
/// `lib.plist`: a layer lib of a layer the UFO has (`names`), not read from `lib.plist`.
pub(crate) fn in_layer_lib(key: &str, names: &[String], kept_lib: &Dict) -> bool {
    let Some(name) = key.strip_prefix(LAYER_LIB) else { return false };
    kept_lib.get(key).is_none() && (name == DEFAULT_LAYER || names.iter().any(|n| n == name))
}

/// One UFO layer of a master's UFO, as [`layerinfo`] needs it.
pub(crate) struct UfoLayer<'a> {
    pub name: &'a str,
    pub is_default: bool,
    /// Its `layerinfo.plist` as read.
    pub kept: Option<&'a Value>,
    /// The glyphs' layers in it (no backgrounds).
    pub parts: &'a [(&'a Glyph, &'a Layer)],
}

/// What one UFO layer's `layerinfo.plist` holds, starting from the kept one: its lib gets the
/// layer id, the glyphs' layer order and the master's user data for the layer. `new` says the
/// UFO was not read from disk; `kept_lib_plist` is the master's `lib.plist` as read.
pub(crate) fn layerinfo(font: &Font, master: &Master, layer: &UfoLayer<'_>, new: bool, kept_lib_plist: &Dict) -> Option<Value> {
    let UfoLayer { name, is_default, kept, parts } = *layer;
    let mut info = kept.and_then(Value::as_dict).cloned().unwrap_or_default();
    let mut lib = info.get("lib").and_then(Value::as_dict).cloned().unwrap_or_default();
    let had_lib = info.get("lib").is_some();
    let mut want: BTreeMap<String, Value> = BTreeMap::new();
    // The layers of a master stored as this UFO layer are master layers, with the master's id.
    if !is_default && !parts.is_empty() && parts.iter().all(|(_, l)| !l.is_master_layer()) {
        let first = parts.first().map(|(_, l)| l.id.as_str()).unwrap_or_default();
        if parts.iter().all(|(_, l)| l.id == first) && first != synthetic_id(&master.id, name) && usable_id(first) {
            want.insert(LAYER_ID.to_string(), string(first));
        }
    }
    for (g, l) in parts {
        let key = format!("{LAYER_ORDER}{}", g.name);
        let needed = if new { needs_order(font, g) } else { lib.get(&key).is_some() };
        if needed && let Some(i) = g.layers.iter().position(|x| std::ptr::eq(x, *l)) {
            want.insert(key, int(i64::try_from(i).unwrap_or(i64::MAX)));
        }
    }
    // A layer lib read from `lib.plist` stays there, and this one stays as read.
    let lib_key = layer_lib_key(name, is_default);
    let typed_user = kept_lib_plist.get(&lib_key).is_none();
    let user = typed_user.then(|| master.user_data.get(&lib_key).and_then(Value::as_dict)).flatten();
    if let Some(u) = user {
        for (k, v) in u.entries.iter().filter(|(k, _)| k.as_str() != LAYER_NAME) {
            want.insert(k.as_str().to_string(), v.without_spellings());
        }
    }
    let stale: Vec<String> = lib
        .entries
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| (*k == LAYER_ID || k.starts_with(LAYER_ORDER) || !reserved(k) && typed_user) && !want.contains_key(*k))
        .map(str::to_string)
        .collect();
    for k in stale {
        crate::ufo::patch(&mut lib, &k, None);
    }
    for (k, v) in want {
        crate::ufo::patch(&mut lib, &k, Some(v));
    }
    if !lib.is_empty() || had_lib {
        crate::ufo::patch(&mut info, "lib", Some(Value::Dict(lib)));
    }
    (kept.is_some() || !info.is_empty()).then_some(Value::Dict(info))
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

/// One typed key: the layer (by index), whether it is the layer's background, and the key.
type At = (usize, bool, String);

/// Types glyphsLib's glyph, layer and shape keys in the layers' user data (each GLIF's lib, but
/// the keys the GLIF reader types itself) into model fields where writing the fields gives the
/// same value; every key the writer gives back unchanged is no longer user data. `default_id`
/// names the master whose master layer gives the glyph's data. Before that, the `layerinfo.plist`
/// libs give the layers' ids, their order and the masters' layer libs, and each glyph's master
/// layers come first, in the masters' order.
pub(crate) fn read(font: &mut Font, default_id: &str) {
    read_layer_infos(font);
    for i in 0..font.glyphs.len() {
        let Some(g) = font.glyphs.as_arcs().get(i) else { continue };
        if !has_candidates(g) {
            continue;
        }
        if let Some(typed) = type_glyph(font, g, default_id)
            && let Some(mut slot) = font.glyphs.get_mut_at(i)
        {
            *slot = typed;
        }
    }
}

/// A glyph with a key this module or [`crate::shapekeys`] might type.
fn has_candidates(g: &Glyph) -> bool {
    let key = |k: &str| k.starts_with("com.schriftgestaltung.") || k == COLOR_LAYER_MAPPING || k == shapekeys::OBJECT_LIBS;
    g.layers.iter().any(|l| {
        l.info.user_data.0.entries.iter().any(|(k, _)| key(k.as_str()))
            || l.info.background.as_ref().is_some_and(|b| {
                b.extra.get(Family::Ufo, "lib").and_then(Value::as_dict).is_some_and(|d| d.entries.iter().any(|(k, _)| key(k.as_str())))
            })
    })
}

/// The glyph with its keys typed. When [`MAX_PASSES`] are not enough, nothing is typed.
fn type_glyph(font: &Font, g: &Glyph, default_id: &str) -> Option<Glyph> {
    let mut excluded: HashSet<At> = HashSet::new();
    for pass in 0..=MAX_PASSES {
        let mut t = g.clone();
        let typed = if pass == MAX_PASSES { Vec::new() } else { apply(&mut t, default_id, &excluded) };
        // A background is typed as a whole: one whose keys did not all type stays as read.
        for (li, l) in t.layers.iter_mut().enumerate() {
            let Some(orig) = g.layers.get(li).and_then(|o| o.info.background.as_ref()) else { continue };
            let lib = orig.extra.get(Family::Ufo, "lib").and_then(Value::as_dict);
            let present = lib.into_iter().flat_map(|d| d.entries.iter()).filter(|(k, _)| background_key(k.as_str()));
            let all = present.clone().all(|(k, _)| typed.contains(&(li, true, k.as_str().to_string())));
            if present.count() > 0 && !all {
                let mut b = orig.clone();
                b.extra.push(Family::Ufo, UNTYPED, Value::Bool(true));
                l.info.background = Some(b);
            }
        }
        let typed: Vec<At> = typed
            .into_iter()
            .filter(|(li, bg, _)| {
                !*bg || t.layers.get(*li).and_then(|l| l.info.background.as_ref()).is_some_and(|b| b.extra.get(Family::Ufo, UNTYPED).is_none())
            })
            .collect();
        let written: Vec<(Written, Option<Written>)> = t
            .layers
            .iter()
            .map(|l| {
                let format = glif_format(l);
                let w = entries(&Glif { font, glyph: &t, layer: l, background: false, primary: is_primary(l, default_id), new: false, format });
                let bw = l.info.background.as_ref().map(|b| {
                    let stand_in = crate::ufo::background_layer(b, &l.master);
                    entries(&Glif { font, glyph: &t, layer: &stand_in, background: true, primary: false, new: false, format })
                });
                (w, bw)
            })
            .collect();
        let mut bad: Vec<At> = Vec::new();
        for at in &typed {
            let (li, bg, key) = at;
            let Some(((w, bw), orig)) = written.get(*li).zip(g.layers.get(*li)) else { continue };
            let (got, said) = if *bg {
                let kept = orig.info.background.as_ref().and_then(|b| b.extra.get(Family::Ufo, "lib")).and_then(Value::as_dict);
                (bw.as_ref().and_then(|w| w.map.get(key)), kept.and_then(|d| d.get(key)))
            } else {
                (w.map.get(key), orig.info.user_data.get(key))
            };
            if got.is_none() || got.map(Value::without_spellings) != said.map(Value::without_spellings) {
                bad.push(at.clone());
            }
        }
        if bad.is_empty() {
            // Every key the writer gives back unchanged is no longer user data.
            for (l, (w, _)) in t.layers.iter_mut().zip(&written) {
                l.info.user_data.0.entries.retain(|(k, v)| w.map.get(k.as_str()).map(Value::without_spellings) != Some(v.without_spellings()));
                l.info.tidy();
            }
            return Some(t);
        }
        // A bad background key excludes its background's others too.
        for (li, bg, _) in &bad {
            if *bg {
                excluded.extend(typed.iter().filter(|(l, b, _)| l == li && *b).cloned());
            }
        }
        excluded.extend(bad);
    }
    None
}

fn background_key(k: &str) -> bool {
    BACKGROUND_KEYS.contains(&k) || shapekeys::is_background_key(k)
}

fn is_primary(l: &Layer, default_id: &str) -> bool {
    l.is_master_layer() && l.master == default_id
}

/// The GLIF format a layer was read with.
fn glif_format(l: &Layer) -> u32 {
    l.extra.get(Family::Ufo, "format").and_then(Value::as_num).map_or(2, |n| if n.as_f64() == 1.0 { 1 } else { 2 })
}

/// Types the keys of every layer's user data, but the `excluded` ones; returns the keys that
/// gave a field.
fn apply(g: &mut Glyph, default_id: &str, excluded: &HashSet<At>) -> Vec<At> {
    let mut typed: Vec<At> = Vec::new();
    let skip = |li: usize, bg: bool, k: &str| excluded.contains(&(li, bg, k.to_string()));
    // The glyph's own, from the default master's master layer.
    if let Some(li) = g.layers.iter().position(|l| is_primary(l, default_id)) {
        let data = g.layers.get(li).map(|l| l.info.user_data.0.clone()).unwrap_or_default();
        for (k, v) in &data.entries {
            let k = k.as_str();
            if skip(li, false, k) {
                continue;
            }
            let info = &mut g.info;
            let done = match k {
                CATEGORY => text(v).map(|t| info.category = Some(t)),
                SUB_CATEGORY => text(v).map(|t| info.sub_category = Some(t)),
                SCRIPT => text(v).map(|t| info.script = Some(t)),
                EXPORT => v.as_bool().map(|b| g.export = g.export && b),
                LAST_CHANGE => v.as_str().and_then(last_change_to_model).map(|t| info.last_change = Some(t)),
                SMART_AXES => smart_axes_to_model(v).map(|p| info.parts = p),
                _ => match GLYPH_METRICS.iter().position(|m| *m == k) {
                    Some(0) => text(v).map(|t| info.metrics_keys.left = Some(t)),
                    Some(1) => text(v).map(|t| info.metrics_keys.right = Some(t)),
                    Some(2) => text(v).map(|t| info.metrics_keys.width = Some(t)),
                    _ => None,
                },
            };
            if done.is_some() {
                g.info.tidy();
                typed.push((li, false, k.to_string()));
            }
        }
    }
    // Each layer's own. The colour layer mapping sets other layers' attributes.
    let mut palettes: Vec<(String, String, PaletteEntry)> = Vec::new();
    for (li, l) in g.layers.iter_mut().enumerate() {
        let data = l.info.user_data.0.clone();
        let mut consumed: Vec<String> = Vec::new();
        for (k, v) in &data.entries {
            let k = k.as_str();
            if skip(li, false, k) {
                continue;
            }
            let done = match k {
                HINTS => {
                    let items = v.as_array().map(|a| a.items.as_slice()).unwrap_or_default();
                    (v.as_array().is_some() && items.len() <= MAX_LAYER_HINTS)
                        .then(|| items.iter().map(hint_from_ufo).collect::<Option<Vec<_>>>())
                        .flatten()
                        .map(|h| l.info.hints = h)
                }
                ANNOTATIONS => {
                    let items = v.as_array().map(|a| a.items.as_slice()).unwrap_or_default();
                    (v.as_array().is_some() && items.len() <= MAX_ANNOTATIONS)
                        .then(|| items.iter().map(annotation_from_ufo).collect::<Option<Vec<_>>>())
                        .flatten()
                        .map(|a| l.info.annotations = a)
                }
                CROP | LOCKED | ALPHA => l.info.image.as_mut().and_then(|img| image_key(img, k, v)),
                ORIGINAL_WIDTH => coordinate(v).filter(|w| *w > 0.0 && l.width == 0.0 && may_be_nonspacing_mark(&g.info)).map(|w| l.width = w),
                COLOR_LAYER_MAPPING if l.is_master_layer() => palette_entries(v).map(|list| {
                    for (name, p) in list {
                        palettes.push((l.master.clone(), name, p));
                    }
                }),
                _ => match LAYER_METRICS.iter().position(|m| *m == k) {
                    Some(0) => text(v).map(|t| l.info.metrics_keys.left = Some(t)),
                    Some(1) => text(v).map(|t| l.info.metrics_keys.right = Some(t)),
                    Some(2) => text(v).map(|t| l.info.metrics_keys.width = Some(t)),
                    _ => None,
                },
            };
            if done.is_some() {
                consumed.push(k.to_string());
            }
        }
        let shapes = shapekeys::apply(&g.name, l, &|k| skip(li, false, k));
        consumed.extend(shapes.consumed);
        l.info.user_data.0.entries.retain(|(k, _)| !consumed.iter().any(|c| c == k.as_str()));
        l.info.tidy();
        typed.extend(consumed.into_iter().chain(shapes.kept).map(|k| (li, false, k)));
        // The background's, from its lib as read.
        if let Some(b) = l.info.background.as_mut() {
            let lib = b.extra.get(Family::Ufo, "lib").and_then(Value::as_dict).cloned().unwrap_or_default();
            let mut bg: Vec<String> = Vec::new();
            for (k, v) in &lib.entries {
                let k = k.as_str();
                if skip(li, true, k) {
                    continue;
                }
                if BACKGROUND_KEYS.contains(&k) && b.image.as_mut().and_then(|img| image_key(img, k, v)).is_some() {
                    bg.push(k.to_string());
                }
            }
            bg.extend(shapekeys::apply_background(&g.name, b, &lib, &|k| skip(li, true, k)));
            typed.extend(bg.into_iter().map(|k| (li, true, k)));
        }
    }
    for (master, name, p) in palettes {
        if let Some(l) = g.layers.iter_mut().find(|l| {
            !l.is_master_layer() && l.master == master && l.info.name.as_deref() == Some(name.as_str()) && l.info.attrs.color_palette.is_none()
        }) {
            l.info.attrs.color_palette = Some(p);
        }
    }
    typed
}

/// One of the image's three keys into its field.
fn image_key(img: &mut fontcraft_model::Image, k: &str, v: &Value) -> Option<()> {
    match k {
        CROP => {
            let items = &v.as_array()?.items;
            let n: Vec<f64> = items.iter().map(|x| x.as_num().map(Num::as_f64).filter(|f| f.is_finite())).collect::<Option<_>>()?;
            let [x, y, w, h] = n.as_slice() else { return None };
            let crop = [*x, *y, *w, *h];
            img.crop = (crop != [0.0; 4]).then_some(crop);
            Some(())
        }
        LOCKED => v.as_bool().map(|b| img.locked = b),
        ALPHA => v.as_num().map(Num::as_f64).filter(|f| f.is_finite()).map(|a| img.alpha = (a != DEFAULT_ALPHA).then_some(a)),
        _ => None,
    }
}

/// The colour layer mapping's entries: UFO layer name and palette entry.
pub(crate) fn palette_entries(v: &Value) -> Option<Vec<(String, PaletteEntry)>> {
    v.as_array()?
        .items
        .iter()
        .map(|e| {
            let [n, i] = e.as_array()?.items.as_slice() else { return None };
            let index = match i.as_num()? {
                Num::Int(65535) => PaletteEntry::Foreground,
                Num::Int(i) => PaletteEntry::Index(u16::try_from(i).ok().filter(|i| *i != u16::MAX)?),
                Num::Float(_) => return None,
            };
            Some((n.as_str()?.to_string(), index))
        })
        .collect()
}

/// The `layerinfo.plist` libs: layer ids, the glyphs' layer order, and the layer libs as the
/// masters' user data. Then each glyph's master layers go first, in the masters' order, unless
/// order keys sorted it.
fn read_layer_infos(font: &mut Font) {
    let masters: Vec<(String, Dict, String, Dict)> = font
        .masters
        .iter()
        .filter_map(|m| {
            let infos = m.extra.get(Family::Ufo, "layerinfo").and_then(Value::as_dict).cloned().unwrap_or_default();
            let default = crate::ufo::kept_default_layer(m);
            let lib = m.extra.get(Family::Ufo, "lib.plist").and_then(Value::as_dict).cloned().unwrap_or_default();
            m.extra.get(Family::Ufo, "metainfo.plist").map(|_| (m.id.clone(), infos, default, lib))
        })
        .collect();
    // Layer libs, as glyphsLib's `to_glyphs_layer_lib` keeps them.
    {
        let ms = std::sync::Arc::make_mut(&mut font.masters);
        for (id, infos, default, kept_lib) in &masters {
            let Some(m) = ms.iter_mut().find(|m| &m.id == id) else { continue };
            let mut names: Vec<(String, bool)> = infos.entries.iter().map(|(k, _)| (k.as_str().to_string(), k.as_str() == default)).collect();
            if default != DEFAULT_LAYER && !names.iter().any(|(_, d)| *d) {
                names.push((default.clone(), true));
            }
            for (name, is_default) in names {
                let lib = infos.get(&name).and_then(Value::as_dict).and_then(|d| d.get("lib")).and_then(Value::as_dict);
                let mut user = Dict::new();
                for (k, v) in lib.into_iter().flat_map(|d| d.entries.iter()).filter(|(k, _)| !reserved(k.as_str())) {
                    user.insert(k.as_str(), v.without_spellings());
                }
                if is_default && name != DEFAULT_LAYER {
                    user.insert(LAYER_NAME, string(&name));
                }
                let key = layer_lib_key(&name, is_default);
                if user.is_empty() || m.user_data.get(&key).is_some() || kept_lib.get(&key).is_some() {
                    continue;
                }
                let at = m.user_data.0.entries.iter().position(|(k, _)| k.as_str() > key.as_str()).unwrap_or(m.user_data.0.entries.len());
                m.user_data.0.insert_at(at, key.as_str(), sorted_keys(Value::Dict(user)));
            }
        }
    }
    let master_ids: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    // A UFO layer's id is taken only when every glyph's layer in it can have it, so that the
    // writer gives it back.
    let mut blocked: HashSet<(String, String)> = HashSet::new();
    for g in font.glyphs.iter() {
        for (id, infos, default, _) in &masters {
            for (name, info) in infos.entries.iter().filter(|(n, _)| n.as_str() != default) {
                let lib = info.as_dict().and_then(|d| d.get("lib")).and_then(Value::as_dict);
                let Some(new_id) = lib.and_then(|d| d.get(LAYER_ID)).and_then(Value::as_str) else { continue };
                let mine = |l: &&Layer| !l.is_master_layer() && &l.master == id && l.info.name.as_deref() == Some(name.as_str());
                let ok = usable_id(new_id)
                    && g.layers.iter().filter(mine).all(|l| l.id == synthetic_id(id, name.as_str()))
                    && !g.layers.iter().any(|l| l.id == new_id)
                    && g.layers.iter().filter(mine).count() <= 1;
                if !ok {
                    blocked.insert((id.clone(), name.as_str().to_string()));
                }
            }
        }
    }
    for i in 0..font.glyphs.len() {
        let Some(g) = font.glyphs.as_arcs().get(i) else { continue };
        let mut ids: Vec<(usize, String)> = Vec::new();
        let mut order: Vec<(usize, i64)> = Vec::new();
        for (id, infos, default, _) in &masters {
            for (name, info) in &infos.entries {
                let Some(lib) = info.as_dict().and_then(|d| d.get("lib")).and_then(Value::as_dict) else { continue };
                let is_default = name.as_str() == default;
                let at = g.layers.iter().position(|l| {
                    if is_default {
                        l.is_master_layer() && &l.master == id
                    } else {
                        !l.is_master_layer() && &l.master == id && l.info.name.as_deref() == Some(name.as_str())
                    }
                });
                let Some(at) = at else { continue };
                if let Some(new_id) = lib.get(LAYER_ID).and_then(Value::as_str).filter(|_| !is_default)
                    && !blocked.contains(&(id.clone(), name.as_str().to_string()))
                {
                    ids.push((at, new_id.to_string()));
                }
                if let Some(n) = lib.get(&format!("{LAYER_ORDER}{}", g.name)).and_then(Value::as_num).and_then(|n| match n {
                    Num::Int(i) if i >= 0 => Some(i),
                    _ => None,
                }) {
                    order.push((at, n));
                }
            }
        }
        let rank = |l: &Layer| master_ids.iter().position(|m| *m == l.id);
        let arranged = g.layers.iter().filter(|l| l.is_master_layer()).map(rank).collect::<Vec<_>>().windows(2).all(|w| match w {
            [Some(a), Some(b)] => a < b,
            _ => true,
        }) && {
            // Master layers before the others.
            let first_other = g.layers.iter().position(|l| !l.is_master_layer()).unwrap_or(g.layers.len());
            g.layers.iter().skip(first_other).all(|l| !l.is_master_layer())
        };
        if ids.is_empty() && order.is_empty() && arranged {
            continue;
        }
        let Some(mut gm) = font.glyphs.get_mut_at(i) else { continue };
        let mut taken: HashSet<String> = gm.layers.iter().map(|l| l.id.clone()).collect();
        for (at, id) in ids {
            if taken.insert(id.clone())
                && let Some(l) = gm.layers.get_mut(at)
            {
                l.id = id;
            }
        }
        let mut keyed: Vec<(i64, usize, Layer)> = std::mem::take(&mut gm.layers)
            .into_iter()
            .enumerate()
            .map(|(k, l)| {
                let key = order.iter().find(|(at, _)| *at == k).map(|(_, n)| *n);
                let fallback = if order.is_empty() {
                    // No order keys: master layers first, in the masters' order.
                    if l.is_master_layer() { i64::try_from(rank(&l).unwrap_or(0)).unwrap_or(0) } else { i64::MAX - 1 }
                } else {
                    i64::MAX
                };
                (key.unwrap_or(fallback), k, l)
            })
            .collect();
        keyed.sort_by_key(|(n, k, _)| (*n, *k));
        gm.layers = keyed.into_iter().map(|(_, _, l)| l).collect();
    }
}

/// Whether a layer's hints do not all travel: a background's, or one with settings or keys
/// glyphsLib's form has no place for.
pub(crate) fn hints_lost(l: &Layer) -> bool {
    l.info.hints.iter().any(|h| !hint_to_ufo(h).is_some_and(|(_, complete)| complete))
        || l.info.background.as_ref().is_some_and(|b| !b.hints.is_empty())
}

/// Whether the layer's annotations do not all travel: a background's, or one with keys
/// glyphsLib's form has no place for.
pub(crate) fn annotations_lost(l: &Layer) -> bool {
    l.info.annotations.iter().any(|a| !a.extra.is_empty() || a.text.as_deref() == Some("") || a.width == DEFAULT_ANNOTATION_WIDTH)
        || l.info.background.as_ref().is_some_and(|b| !b.annotations.is_empty())
}

/// Whether the glyph's smart-glyph properties do not all travel.
pub(crate) fn parts_lost(g: &Glyph) -> bool {
    g.info.parts.iter().any(|p| !p.extra.is_empty() || p.bottom_name.as_deref() == Some("") || p.top_name.as_deref() == Some(""))
}

/// Whether the glyph's last-change date has no UFO form (or the font asks for none).
pub(crate) fn last_change_lost(font: &Font, g: &Glyph) -> bool {
    g.info.last_change.as_deref().is_some_and(|t| last_change_to_ufo(t).is_none() || last_change_disabled(font))
}

/// Whether a layer's image opacity, crop or lock does not come back.
pub(crate) fn image_lost(i: &fontcraft_model::Image) -> bool {
    i.alpha == Some(DEFAULT_ALPHA) || i.crop == Some([0.0; 4])
}

/// The user data dictionary of a layer without the keys `typed` says are not user data.
pub(crate) fn untyped_user_data(lib: &Dict, typed: impl Fn(&str) -> bool) -> UserData {
    let mut user = Dict::new();
    for (k, v) in lib.entries.iter().filter(|(k, _)| !crate::glif::TYPED_LIB.contains(&k.as_str()) && !typed(k.as_str())) {
        user.insert(k.as_str(), v.without_spellings());
    }
    UserData(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{designspace, ufo};
    use fontcraft_model::{Background, Image, Path, Shape};
    use std::sync::Arc;

    fn text(files: &crate::Files, path: &str) -> String {
        String::from_utf8(files[path].clone()).unwrap()
    }

    fn v3(text: &str) -> Value {
        fontcraft_plist::openstep::parse_value(text).unwrap()
    }

    /// A one-master font as read from a `.glyphs` source, with data for every glyph and layer key.
    fn glyphs_font() -> Font {
        let mut font = Font::new("Keys Sans");
        font.source.glyphs_version = Some(3);
        {
            let mut a = font.glyph_mut("A").unwrap();
            let i = &mut a.info;
            i.category = Some("Letter".into());
            i.sub_category = Some("Uppercase".into());
            i.script = Some("latin".into());
            i.last_change = Some("2023-02-25 14:14:38 +0000".into());
            i.metrics_keys.left = Some("=10".into());
            i.metrics_keys.width = Some("H".into());
            i.user_data.0.insert("Some Key", string("Some Value"));
            let l = a.layers.get_mut(0).unwrap();
            l.info.metrics_keys.right = Some("=20".into());
            l.info.hints = vec![Hint { family: Family::Glyphs, value: v3("{horizontal = 1; origin = (0,0); target = (0,3); type = Stem;}") }];
            l.info.annotations = vec![Annotation {
                kind: "Text".into(),
                x: 250.5,
                y: 646.0,
                angle: 0.0,
                text: Some("a note".into()),
                width: 0.0,
                extra: Default::default(),
            }];
            let transform = fontcraft_model::Transform::Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            l.info.image =
                Some(Image { path: Some("A.png".into()), transform, crop: Some([0.0, 0.0, 140.0, 160.0]), locked: true, ..Default::default() });
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            let mut color = Layer::new("m01", 600.0);
            color.id = "C1".into();
            color.info.attrs.color_palette = Some(PaletteEntry::Index(1));
            a.layers.push(color);
            let mut backup = Layer::new("m01", 600.0);
            backup.id = "B1".into();
            backup.info.name = Some("Backup".into());
            backup.info.background = Some(Background {
                image: Some(Image { path: Some("bg.png".into()), alpha: Some(30.0), ..Default::default() }),
                ..Default::default()
            });
            // The backup layer before the master layer.
            a.layers.insert(0, backup);
        }
        {
            let mut m = font.glyph_mut("B").unwrap();
            m.info.category = Some("Mark".into());
            m.info.sub_category = Some("Nonspacing".into());
            m.info.parts = vec![PartProperty {
                name: "Width".into(),
                bottom_value: 0.0,
                top_value: 100.0,
                bottom_name: None,
                top_name: None,
                extra: Default::default(),
            }];
        }
        font
    }

    #[test]
    fn glyph_and_layer_data_go_to_glyphslib_keys_and_come_back() {
        let font = glyphs_font();
        let files = ufo::write(&font).unwrap();
        let a = text(&files, "glyphs/A_.glif");
        for expected in [
            "<key>com.schriftgestaltung.Glyphs.category</key>\n      <string>Letter</string>",
            "<key>com.schriftgestaltung.Glyphs.subCategory</key>\n      <string>Uppercase</string>",
            "<key>com.schriftgestaltung.Glyphs.script</key>\n      <string>latin</string>",
            "<key>com.schriftgestaltung.Glyphs.lastChange</key>\n      <string>2023/02/25 14:14:38</string>",
            "<key>com.schriftgestaltung.Glyphs.glyph.leftMetricsKey</key>\n      <string>=10</string>",
            "<key>com.schriftgestaltung.Glyphs.glyph.widthMetricsKey</key>\n      <string>H</string>",
            "<key>com.schriftgestaltung.Glyphs.layer.rightMetricsKey</key>\n      <string>=20</string>",
            "<key>com.schriftgestaltung.hints</key>\n      <array>\n        <dict>\n          <key>horizontal</key>\n          <true/>\n          <key>name</key>\n          <string></string>",
            "<key>target</key>\n          <array>\n            <integer>0</integer>\n            <integer>3</integer>",
            "<key>com.schriftgestaltung.annotations</key>\n      <array>\n        <dict>\n          <key>position</key>\n          <array>\n            <real>250.5</real>",
            "<key>width</key>\n          <integer>100</integer>",
            "<key>com.schriftgestaltung.backgroundImage.crop</key>\n      <array>\n        <integer>0</integer>",
            "<key>com.schriftgestaltung.backgroundImage.locked</key>\n      <true/>",
            "<key>com.schriftgestaltung.backgroundImage.alpha</key>\n      <integer>50</integer>",
            "<key>com.github.googlei18n.ufo2ft.colorLayerMapping</key>\n      <array>\n        <array>\n          <string>color.1</string>\n          <integer>1</integer>",
        ] {
            assert!(a.contains(expected), "{expected} in {a}");
        }
        // Every layer's file gets the glyph's keys, as glyphsLib writes them.
        let backup = text(&files, "glyphs.B_ackup/A_.glif");
        assert!(backup.contains("Glyphs.category") && !backup.contains("layer.rightMetricsKey"), "{backup}");
        let bg = text(&files, "glyphs.B_ackup.background/A_.glif");
        assert!(bg.contains("<key>com.schriftgestaltung.backgroundImage.alpha</key>\n      <integer>30</integer>"), "{bg}");
        // A non-spacing mark is written with no width, its width in the lib.
        let b = text(&files, "glyphs/B_.glif");
        assert!(!b.contains("<advance") && b.contains("<key>com.schriftgestaltung.Glyphs.originalWidth</key>\n      <integer>600</integer>"), "{b}");
        assert!(
            b.contains("<key>com.schriftgestaltung.smartComponentAxes</key>\n      <array>\n        <dict>\n          <key>bottomName</key>"),
            "{b}"
        );
        let lib = text(&files, "lib.plist");
        assert!(lib.contains("<key>com.schriftgestaltung.Glyphs.glyphUserData.A</key>\n    <dict>\n      <key>Some Key</key>"), "{lib}");
        // Layer ids and order: the backup layer came first.
        let info = text(&files, "glyphs.B_ackup/layerinfo.plist");
        assert!(info.contains("<key>com.schriftgestaltung.layerId</key>\n      <string>B1</string>"), "{info}");
        assert!(info.contains("<key>com.schriftgestaltung.layerOrderInGlyph.A</key>\n      <integer>0</integer>"), "{info}");
        assert!(text(&files, "glyphs/layerinfo.plist").contains("<key>com.schriftgestaltung.layerOrderInGlyph.A</key>\n      <integer>1</integer>"));

        let back = ufo::read(&files).unwrap();
        let (ga, gb) = (back.glyph("A").unwrap(), back.glyph("B").unwrap());
        let (fa, fb) = (font.glyph("A").unwrap(), font.glyph("B").unwrap());
        assert_eq!(ga.info, fa.info);
        assert_eq!(gb.info, fb.info);
        let ids: Vec<&str> = ga.layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["B1", "m01", "C1"], "ids and order come back");
        let (l, was) = (ga.layer("m01").unwrap(), fa.layer("m01").unwrap());
        assert_eq!((&l.info.metrics_keys, &l.info.annotations, &l.info.image), (&was.info.metrics_keys, &was.info.annotations, &was.info.image));
        assert_eq!(l.info.hints.len(), 1);
        assert_eq!(l.info.hints[0].family, Family::Ufo, "hints come back in glyphsLib's form");
        assert!(l.info.user_data.is_empty(), "{:?}", l.info.user_data);
        assert_eq!(ga.layers[2].info.attrs.color_palette, Some(PaletteEntry::Index(1)));
        let bg = ga.layers[0].info.background.as_ref().unwrap();
        assert_eq!(bg.image.as_ref().unwrap().alpha, Some(30.0));
        assert_eq!(gb.layer("m01").unwrap().width, 600.0, "the mark's width comes back");
        assert!(back.masters[0].user_data.is_empty(), "{:?}", back.masters[0].user_data);
        assert_eq!(ufo::write(&back).unwrap(), files, "it comes back unchanged");
    }

    #[test]
    fn an_edit_writes_the_new_value() {
        let files = ufo::write(&glyphs_font()).unwrap();
        let mut font = ufo::read(&files).unwrap();
        {
            let mut a = font.glyph_mut("A").unwrap();
            a.info.category = Some("Number".into());
            a.layer_mut("m01").unwrap().info.image.as_mut().unwrap().crop = None;
            a.layer_mut("m01").unwrap().info.annotations.clear();
        }
        let again = ufo::write(&font).unwrap();
        let a = text(&again, "glyphs/A_.glif");
        assert!(a.contains("<key>com.schriftgestaltung.Glyphs.category</key>\n      <string>Number</string>"), "{a}");
        assert!(a.contains("<key>com.schriftgestaltung.backgroundImage.crop</key>\n      <array>\n        <integer>0</integer>\n        <integer>0</integer>\n        <integer>0</integer>"), "the file had the key: {a}");
        assert!(!a.contains("annotations"), "{a}");
        // The other layer's file had the key, so it follows.
        assert!(text(&again, "glyphs.B_ackup/A_.glif").contains("<string>Number</string>"));
    }

    #[test]
    fn keys_of_another_form_stay_user_data() {
        let mut font = Font::new("Odd");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let u = &mut a.layers[0].info.user_data.0;
            u.insert(CATEGORY, int(5));
            u.insert(HINTS, string("none"));
            u.insert(SMART_AXES, array(vec![dict(vec![("name", string("Width"))])]));
            u.insert(LAST_CHANGE, string("yesterday"));
            u.insert(ORIGINAL_WIDTH, int(300));
            u.insert(COLOR_LAYER_MAPPING, array(vec![array(vec![string("color.1"), int(1)])]));
            u.insert(ANNOTATIONS, array(vec![dict(vec![("position", array(vec![int(1), int(2)])), ("type", string("Text")), ("width", int(0))])]));
        }
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        let a = back.glyph("A").unwrap();
        assert!(a.info.is_default(), "{:?}", a.info);
        assert_eq!(a.layers[0].width, 600.0, "a width that is not 0 stays");
        assert!(a.layers[0].info.annotations.is_empty() && a.layers[0].info.hints.is_empty());
        assert_eq!(a.layers[0].info.user_data.0.len(), 7, "{:?}", a.layers[0].info.user_data);
        assert_eq!(ufo::write(&back).unwrap(), files);
    }

    #[test]
    fn a_new_ufo_of_another_document_gets_no_defaults() {
        let mut font = Font::new("Plain");
        font.glyph_mut("A").unwrap().layers[0].info.image = Some(Image { path: Some("A.png".into()), ..Default::default() });
        let files = ufo::write(&font).unwrap();
        assert!(!text(&files, "glyphs/A_.glif").contains("backgroundImage"));
        assert!(!files.contains_key("glyphs/layerinfo.plist"), "no layer order to keep");
    }

    #[test]
    fn a_designspace_takes_glyph_data_from_the_default_source() {
        let mut font = glyphs_font();
        Arc::make_mut(&mut font.axes).push(fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        let mut bold = font.masters[0].clone();
        (bold.id, bold.name, bold.location) = ("m02".into(), "Bold".into(), vec![700.0]);
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        Arc::make_mut(&mut font.masters).push(bold);
        let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
        for n in names {
            let mut g = font.glyph_mut(&n).unwrap();
            let mut l = g.layer("m01").unwrap().clone();
            (l.id, l.master) = ("m02".into(), "m02".into());
            g.layers.push(l);
        }
        let out = designspace::write(&font).unwrap();
        let back = designspace::read(&out.document, &out.ufos).unwrap();
        assert_eq!(back.glyph("A").unwrap().info, font.glyph("A").unwrap().info);
        let ids: Vec<&str> = back.glyph("A").unwrap().layers.iter().map(|l| l.id.as_str()).collect();
        let was: Vec<&str> = font.glyph("A").unwrap().layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, was);
        // A source whose file says another category keeps it as its own.
        let mut ufos = out.ufos.clone();
        let (bold_ufo, files) = ufos.iter_mut().find(|(k, _)| k.contains("Bold")).unwrap();
        let path = "glyphs/A_.glif";
        let glif = text(files, path).replace("<string>Letter</string>", "<string>Number</string>");
        files.insert(path.into(), glif.into_bytes());
        let _ = bold_ufo;
        let back = designspace::read(&out.document, &ufos).unwrap();
        let a = back.glyph("A").unwrap();
        assert_eq!(a.info.category.as_deref(), Some("Letter"));
        assert_eq!(a.layer("m02").unwrap().info.user_data.get(CATEGORY), Some(&string("Number")));
        let again = designspace::write(&back).unwrap();
        assert_eq!(again.ufos, ufos, "it comes back unchanged");
    }

    #[test]
    fn layer_libs_and_the_default_layer_name_are_master_user_data() {
        let mut font = Font::new("Layers");
        {
            let m = &mut Arc::make_mut(&mut font.masters)[0];
            m.user_data.0.insert(format!("{LAYER_LIB}public.default"), dict(vec![(LAYER_NAME, string("foreground")), ("org.example.x", int(1))]));
        }
        let files = ufo::write(&font).unwrap();
        assert!(text(&files, "layercontents.plist").contains("<string>foreground</string>"));
        let info = text(&files, "glyphs/layerinfo.plist");
        assert!(info.contains("<key>org.example.x</key>") && !info.contains("layerName"), "{info}");
        assert!(!text(&files, "lib.plist").contains("layerLib"));
        let back = ufo::read(&files).unwrap();
        assert_eq!(back.masters[0].user_data, font.masters[0].user_data);
        assert_eq!(ufo::write(&back).unwrap(), files);
    }

    #[test]
    fn a_glyph_export_key_is_read_and_kept_where_it_was() {
        let mut font = Font::new("Export");
        font.glyph_mut("A").unwrap().layers[0].info.user_data.0.insert(EXPORT, Value::Bool(false));
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        assert!(!back.glyph("A").unwrap().export);
        assert!(back.glyph("A").unwrap().layers[0].info.user_data.is_empty());
        assert_eq!(ufo::write(&back).unwrap(), files, "no `public.skipExportGlyphs` is added");
    }

    #[test]
    fn last_change_dates_convert_both_ways() {
        assert_eq!(last_change_to_ufo("2023-02-25 14:14:38 +0000").as_deref(), Some("2023/02/25 14:14:38"));
        assert_eq!(last_change_to_ufo("2023-02-25 14:14:38 +0100"), None);
        assert_eq!(last_change_to_model("2017/10/27 21:28:35").as_deref(), Some("2017-10-27 21:28:35 +0000"));
        assert_eq!(last_change_to_model("2017-10-27 21:28:35"), None);
    }

    #[test]
    fn version_2_hints_become_glyphslib_dictionaries() {
        let h = Hint { family: Family::Glyphs, value: v3("{horizontal = 1; origin = \"{0, 0}\"; target = \"{0, 3}\"; type = Stem;}") };
        let (v, complete) = hint_to_ufo(&h).unwrap();
        assert!(complete);
        let d = v.as_dict().unwrap();
        assert_eq!(d.get("origin"), Some(&array(vec![int(0), int(0)])));
        assert_eq!(d.get("stem"), Some(&int(-2)));
        let with_settings = Hint { family: Family::Glyphs, value: v3("{settings = {zone = 1;}; type = TTStem; origin = lsb;}") };
        let (v, complete) = hint_to_ufo(&with_settings).unwrap();
        assert!(!complete, "settings have no place");
        assert_eq!(v.as_dict().unwrap().get("origin"), Some(&array(vec![string("lsb")])));
    }
}
