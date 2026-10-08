//! The primary reference's source format, `.glyphs` (L3): reading into the model and writing back.
//!
//! The reader moves each key the model has a field for out of its dictionary and into that field;
//! every other key goes to the object's [`Extra`] as a property-list value with its spelling and
//! layout. The writer builds the dictionaries back from the fields and the extra entries, and
//! sorts their keys byte-wise, as the reference does (`docs/plans/M1.md` § 5). Keys whose value is
//! the documented default are left out, as the reference leaves them out.
//!
//! Written from the format documents and JSON schemas in GlyphsSDK (Apache-2.0) and from files in
//! the test corpus. Format versions 2 ([`v2`]), 3 and 4 are read and written.
//!
//! Shapes the model cannot hold yet (format 4 images and groups, paths with quartic, Hobby or
//! spiral nodes, paths whose node list does not make an outline) are kept as [`Shape::Raw`].
#![forbid(unsafe_code)]

use std::sync::Arc;

use fontcraft_model::{
    Anchor, Axis, Component, Extra, Family, Font, Glyph, GlyphTable, Instance, KernFirst, KernGroups, KernTable, Kerning, Layer, Master,
    MasterKerning, MetricDef, MetricValue, ModelError, Node, NodeKind, Path, RawShape, Shape, Source, SourceFormat, Transform,
};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};

mod package;
mod v2;

pub use package::{Files, glyph_file_stem, read_package, write_package};

#[derive(Debug, thiserror::Error)]
pub enum GlyphsError {
    #[error(transparent)]
    Syntax(#[from] fontcraft_plist::Error),
    #[error("{at}: {message}")]
    Bad { at: String, message: String },
    #[error("not supported yet: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Model(#[from] ModelError),
}

type Result<T> = std::result::Result<T, GlyphsError>;

fn bad(at: &str, message: impl Into<String>) -> GlyphsError {
    GlyphsError::Bad { at: at.to_string(), message: message.into() }
}

/// Reads a single-file `.glyphs` document.
pub fn read(text: &str) -> Result<Font> {
    let doc = openstep::parse(text)?;
    let Value::Dict(root) = doc.root else { return Err(bad("the file", "the top level is not a dictionary")) };
    let font = read_root(root)?;
    font.check()?;
    Ok(font)
}

/// The document a root dictionary holds, in whichever format version it states.
fn read_root(root: Dict) -> Result<Font> {
    read_font(root)
}

/// Writes a document as a single `.glyphs` file in the format version it was read in (version 3
/// for a document that did not come from a `.glyphs` file).
pub fn write(font: &Font) -> Result<String> {
    let (root, dialect) = write_root(font)?;
    Ok(openstep::write(&openstep::Document::new(Value::Dict(root)), dialect))
}

/// The root dictionary of a document, and the dialect its version is written in.
fn write_root(font: &Font) -> Result<(Dict, Dialect)> {
    let version = font.source.glyphs_version.unwrap_or(3);
    let dialect = match version {
        2 | 3 => Dialect::Glyphs3,
        4 => Dialect::Glyphs4,
        v => return Err(GlyphsError::Unsupported(format!("writing format version {v}"))),
    };
    let root = if version == 2 { v2::write_font(font)? } else { write_font(font, version)? };
    Ok((root, dialect))
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

fn take(d: &mut Dict, key: &str) -> Option<Value> {
    let i = d.entries.iter().position(|(k, _)| k.as_str() == key)?;
    Some(d.entries.remove(i).1)
}

fn into_extra(d: Dict) -> Extra {
    let mut x = Extra::new();
    for (k, v) in d.entries {
        x.push(Family::Glyphs, k, v);
    }
    x
}

fn as_f64(v: &Value, at: &str) -> Result<f64> {
    match v {
        Value::Number(n) => Ok(n.value().as_f64()),
        _ => Err(bad(at, "expected a number")),
    }
}

fn as_int(v: &Value, at: &str) -> Result<i64> {
    match v.as_num() {
        Some(Num::Int(i)) => Ok(i),
        _ => Err(bad(at, "expected a whole number")),
    }
}

fn as_u16(v: &Value, at: &str) -> Result<u16> {
    u16::try_from(as_int(v, at)?).map_err(|_| bad(at, "expected a number from 0 to 65535"))
}

fn as_string(v: Value, at: &str) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.as_str().to_string()),
        _ => Err(bad(at, "expected a string")),
    }
}

fn as_dict(v: Value, at: &str) -> Result<Dict> {
    match v {
        Value::Dict(d) => Ok(d),
        _ => Err(bad(at, "expected a dictionary")),
    }
}

fn as_items(v: Value, at: &str) -> Result<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.items),
        _ => Err(bad(at, "expected a list")),
    }
}

fn as_bool01(v: &Value, at: &str) -> Result<bool> {
    match as_int(v, at)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(bad(at, "expected 0 or 1")),
    }
}

/// A compact pair such as `(10,20)`.
fn as_pair(v: &Value, at: &str) -> Result<(f64, f64)> {
    match v.as_array().map(|a| a.items.as_slice()) {
        Some([x, y]) => Ok((as_f64(x, at)?, as_f64(y, at)?)),
        _ => Err(bad(at, "expected a pair of numbers")),
    }
}

/// The family name of a version 4 file, which keeps it among the `properties`.
fn v4_family_name(root: &Dict) -> Option<String> {
    let props = root.get("properties")?.as_array()?;
    props.items.iter().find_map(|p| {
        let d = p.as_dict()?;
        if d.get("key")?.as_str()? != "familyNames" {
            return None;
        }
        let first = d.get("values")?.as_array()?.items.first()?.as_dict()?;
        first.get("value")?.as_str().map(str::to_string)
    })
}

fn read_font(mut root: Dict) -> Result<Font> {
    let version = match take(&mut root, ".formatVersion") {
        Some(v) => as_int(&v, ".formatVersion")?,
        None => return v2::read_font(root),
    };
    if !(3..=4).contains(&version) {
        return Err(GlyphsError::Unsupported(format!("format version {version}")));
    }
    let family_name = if version >= 4 {
        v4_family_name(&root).unwrap_or_default()
    } else {
        take(&mut root, "familyName").map(|v| as_string(v, "familyName")).transpose()?.unwrap_or_default()
    };
    let units_per_em = as_u16(&take(&mut root, "unitsPerEm").ok_or_else(|| bad("the file", "no unitsPerEm"))?, "unitsPerEm")?;
    let version_major = take(&mut root, "versionMajor").map(|v| as_u16(&v, "versionMajor")).transpose()?.unwrap_or(0);
    let version_minor = take(&mut root, "versionMinor").map(|v| as_u16(&v, "versionMinor")).transpose()?.unwrap_or(0);
    let axes = take_typed_list(&mut root, "axes", read_axis);
    let instances = take_typed_list(&mut root, "instances", read_instance);
    let metrics = match take(&mut root, "metrics") {
        Some(v) => as_items(v, "metrics")?.into_iter().enumerate().map(|(i, m)| read_metric(m, &format!("metrics[{i}]"))).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let masters = match take(&mut root, "fontMaster") {
        Some(v) => {
            as_items(v, "fontMaster")?.into_iter().enumerate().map(|(i, m)| read_master(m, &format!("fontMaster[{i}]"))).collect::<Result<_>>()?
        }
        None => Vec::new(),
    };
    let glyphs = match take(&mut root, "glyphs") {
        Some(v) => as_items(v, "glyphs")?
            .into_iter()
            .enumerate()
            .map(|(i, g)| read_glyph(g, &format!("glyphs[{i}]")).map(Arc::new))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let kerning = Kerning {
        ltr: take_kern_table(&mut root, "kerningLTR"),
        rtl: take_kern_table(&mut root, "kerningRTL"),
        vertical: take_kern_table(&mut root, "kerningVertical"),
    };
    Ok(Font {
        source: Source { format: SourceFormat::Glyphs, glyphs_version: u32::try_from(version).ok() },
        family_name,
        units_per_em,
        version_major,
        version_minor,
        axes: Arc::new(axes),
        instances: Arc::new(instances),
        metrics: Arc::new(metrics),
        masters: Arc::new(masters),
        glyphs: GlyphTable::from_glyphs(glyphs),
        kerning: Arc::new(kerning),
        files: Arc::default(),
        extra: Arc::new(into_extra(root)),
    })
}

/// A kerning table `{master: {first: {second: value}}}`, taken out of `d` when every value in it
/// is a number; otherwise it stays in `d` as written.
fn take_kern_table(d: &mut Dict, key: &str) -> KernTable {
    let Some(v) = d.get(key) else { return KernTable::default() };
    match kern_table(v) {
        Some(t) => {
            take(d, key);
            t
        }
        None => KernTable::default(),
    }
}

fn kern_table(v: &Value) -> Option<KernTable> {
    let masters = v.as_dict()?;
    let mut out = Vec::with_capacity(masters.len());
    for (master, firsts) in &masters.entries {
        let firsts = firsts.as_dict()?;
        let mut fs = Vec::with_capacity(firsts.len());
        for (first, seconds) in &firsts.entries {
            let seconds = seconds.as_dict()?;
            let pairs = seconds.entries.iter().map(|(s, v)| Some((s.as_str().to_string(), v.as_num()?.as_f64()))).collect::<Option<Vec<_>>>()?;
            fs.push(KernFirst { first: first.as_str().to_string(), seconds: pairs });
        }
        out.push(MasterKerning { master: master.as_str().to_string(), firsts: fs });
    }
    Some(KernTable(out))
}

/// A kerning table as the format writes it: in the table's own order, not sorted.
fn write_kern_table(t: &KernTable) -> Result<Value> {
    let mut masters = Dict::new();
    for m in &t.0 {
        let mut firsts = Dict::new();
        for f in &m.firsts {
            let mut seconds = Dict::new();
            for (s, v) in &f.seconds {
                seconds.entries.push((Str::new(s.as_str()), num(*v, "kerning")?));
            }
            firsts.entries.push((Str::new(f.first.as_str()), Value::Dict(seconds)));
        }
        masters.entries.push((Str::new(m.master.as_str()), Value::Dict(firsts)));
    }
    Ok(Value::Dict(masters))
}

/// A glyph's kerning groups under the given keys (left, right, top, bottom).
fn take_kern_groups(d: &mut Dict, keys: [&str; 4], at: &str) -> Result<KernGroups> {
    let [l, r, t, b] = keys;
    Ok(KernGroups { left: opt_string(d, l, at)?, right: opt_string(d, r, at)?, top: opt_string(d, t, at)?, bottom: opt_string(d, b, at)? })
}

fn push_kern_groups(e: &mut Vec<(Str, Value)>, g: &KernGroups, keys: [&str; 4]) {
    for (value, key) in [&g.left, &g.right, &g.top, &g.bottom].into_iter().zip(keys) {
        if let Some(v) = value {
            e.push((key.into(), string(v)));
        }
    }
}

const KERN_GROUP_KEYS: [&str; 4] = ["kernLeft", "kernRight", "kernTop", "kernBottom"];

fn opt_string(d: &mut Dict, key: &str, at: &str) -> Result<Option<String>> {
    take(d, key).map(|v| as_string(v, &format!("{at}.{key}"))).transpose()
}

/// The items of the list under `key`, when every one of them reads as a `T`; otherwise the list
/// stays in `d` as written and the result is empty.
fn take_typed_list<T>(d: &mut Dict, key: &str, read: fn(&Value) -> Option<T>) -> Vec<T> {
    let Some(Value::Array(a)) = d.get(key) else { return Vec::new() };
    match a.items.iter().map(read).collect::<Option<Vec<T>>>() {
        Some(items) => {
            take(d, key);
            items
        }
        None => Vec::new(),
    }
}

/// A list of numbers, such as `axesValues`.
fn numbers(v: &Value) -> Option<Vec<f64>> {
    v.as_array()?.items.iter().map(|n| n.as_num().map(Num::as_f64)).collect()
}

fn read_axis(v: &Value) -> Option<Axis> {
    let mut d = v.as_dict()?.clone();
    let name = take(&mut d, "name")?.as_str()?.to_string();
    let tag = take(&mut d, "tag")?.as_str()?.to_string();
    let hidden = match take(&mut d, "hidden") {
        Some(h) => as_bool01(&h, "hidden").ok()?,
        None => false,
    };
    Some(Axis { name, tag, hidden, extra: into_extra(d) })
}

fn read_instance(v: &Value) -> Option<Instance> {
    let mut d = v.as_dict()?.clone();
    let name = match take(&mut d, "name") {
        Some(n) => n.as_str()?.to_string(),
        None => String::new(),
    };
    let location = match take(&mut d, "axesValues") {
        Some(a) => numbers(&a)?,
        None => Vec::new(),
    };
    let exports = match take(&mut d, "exports") {
        Some(x) => as_bool01(&x, "exports").ok()?,
        None => true,
    };
    Some(Instance { name, location, exports, extra: into_extra(d) })
}

fn write_axis(a: &Axis) -> Value {
    let mut e = vec![("name".into(), string(&a.name)), ("tag".into(), string(&a.tag))];
    if a.hidden {
        e.push(("hidden".into(), int(1)));
    }
    Value::Dict(finish(e, &a.extra))
}

fn write_numbers(n: &[f64], at: &str) -> Result<Value> {
    Ok(list(n.iter().map(|v| num(*v, at)).collect::<Result<_>>()?))
}

fn write_instance(i: &Instance) -> Result<Value> {
    let mut e = Vec::new();
    if !i.name.is_empty() {
        e.push(("name".into(), string(&i.name)));
    }
    if !i.location.is_empty() {
        e.push(("axesValues".into(), write_numbers(&i.location, "instance")?));
    }
    if !i.exports {
        e.push(("exports".into(), int(0)));
    }
    Ok(Value::Dict(finish(e, &i.extra)))
}

fn read_metric(v: Value, at: &str) -> Result<MetricDef> {
    let mut d = as_dict(v, at)?;
    let kind = opt_string(&mut d, "type", at)?.unwrap_or_default();
    let name = opt_string(&mut d, "name", at)?;
    let filter = opt_string(&mut d, "filter", at)?;
    Ok(MetricDef { kind, name, filter, extra: into_extra(d) })
}

fn read_master(v: Value, at: &str) -> Result<Master> {
    let mut d = as_dict(v, at)?;
    let id = opt_string(&mut d, "id", at)?.ok_or_else(|| bad(at, "a master has no id"))?;
    let name = opt_string(&mut d, "name", at)?.unwrap_or_default();
    let mut location = Vec::new();
    if let Some(v) = take(&mut d, "axesValues") {
        match numbers(&v) {
            Some(n) => location = n,
            None => d.entries.push(("axesValues".into(), v)),
        }
    }
    // Metric values become fields only when every entry is a plain `{over, pos}`; anything else
    // stays as written.
    let mut metric_values = Vec::new();
    if let Some(values) = take(&mut d, "metricValues") {
        match plain_metric_values(&values) {
            Some(mv) => metric_values = mv,
            None => d.entries.push(("metricValues".into(), values)),
        }
    }
    Ok(Master { id, name, location, metric_values, extra: into_extra(d) })
}

fn plain_metric_values(v: &Value) -> Option<Vec<MetricValue>> {
    v.as_array()?
        .items
        .iter()
        .map(|m| {
            let d = m.as_dict()?;
            if d.entries.iter().any(|(k, _)| !matches!(k.as_str(), "pos" | "over")) {
                return None;
            }
            let get = |k: &str| d.get(k).map_or(Some(0.0), |v| v.as_num().map(Num::as_f64));
            Some(MetricValue { pos: get("pos")?, over: get("over")? })
        })
        .collect()
}

fn read_glyph(v: Value, at: &str) -> Result<Glyph> {
    let mut d = as_dict(v, at)?;
    let name = opt_string(&mut d, "glyphname", at)?.ok_or_else(|| bad(at, "a glyph has no glyphname"))?;
    let at = format!("glyph `{name}`");
    // Code points written in any other way than numbers are kept as written.
    let unicodes = match take(&mut d, "unicode") {
        None => Vec::new(),
        Some(v) => match code_points(&v) {
            Some(u) => u,
            None => {
                d.entries.push(("unicode".into(), v));
                Vec::new()
            }
        },
    };
    let export = take(&mut d, "export").map(|v| as_bool01(&v, &format!("{at}.export"))).transpose()?.unwrap_or(true);
    let kern_groups = take_kern_groups(&mut d, KERN_GROUP_KEYS, &at)?;
    let layers = match take(&mut d, "layers") {
        Some(v) => as_items(v, &at)?.into_iter().map(|l| read_layer(l, &at)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Glyph { name, unicodes, export, kern_groups, layers, file_name: None, extra: into_extra(d) })
}

fn code_points(v: &Value) -> Option<Vec<u32>> {
    let one = |v: &Value| match v.as_num() {
        Some(Num::Int(i)) => u32::try_from(i).ok(),
        _ => None,
    };
    match v {
        Value::Array(a) => a.items.iter().map(one).collect(),
        v => one(v).map(|u| vec![u]),
    }
}

fn read_layer(v: Value, at: &str) -> Result<Layer> {
    let mut d = as_dict(v, at)?;
    let id = opt_string(&mut d, "layerId", at)?.ok_or_else(|| bad(at, "a layer has no layerId"))?;
    let at = format!("{at} layer `{id}`");
    let master = opt_string(&mut d, "associatedMasterId", &at)?.unwrap_or_else(|| id.clone());
    // The reference always writes a width; a hand-edited file may not, and reads as zero.
    let width = take(&mut d, "width").map(|v| as_f64(&v, &format!("{at}.width"))).transpose()?.unwrap_or(0.0);
    let shapes = match take(&mut d, "shapes") {
        Some(v) => as_items(v, &at)?.into_iter().map(read_shape).collect(),
        None => Vec::new(),
    };
    let anchors = match take(&mut d, "anchors") {
        Some(v) => as_items(v, &at)?.into_iter().map(|a| read_anchor(a, &at)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Layer { id, master, width, shapes, anchors, extra: into_extra(d) })
}

fn raw(value: Value) -> Shape {
    Shape::Raw(RawShape { family: Family::Glyphs, value })
}

/// A shape the model can hold, or the shape kept as written.
fn read_shape(v: Value) -> Shape {
    let typed = match &v {
        Value::Dict(d) if d.get("ref").is_some() => read_component(d.clone()),
        Value::Dict(d) if d.get("nodes").is_some() => read_path(d.clone()),
        _ => None,
    };
    typed.unwrap_or_else(|| raw(v))
}

fn read_component(mut d: Dict) -> Option<Shape> {
    let glyph = take(&mut d, "ref")?.as_str()?.to_string();
    let pair = |d: &mut Dict, k: &str, default: (f64, f64)| -> Option<(f64, f64)> {
        match take(d, k) {
            Some(v) => as_pair(&v, k).ok(),
            None => Some(default),
        }
    };
    let pos = pair(&mut d, "pos", (0.0, 0.0))?;
    let scale = pair(&mut d, "scale", (1.0, 1.0))?;
    let slant = pair(&mut d, "slant", (0.0, 0.0))?;
    let angle = match take(&mut d, "angle") {
        Some(v) => v.as_num()?.as_f64(),
        None => 0.0,
    };
    Some(Shape::Component(Component { glyph, transform: Transform::Decomposed { pos, scale, angle, slant }, extra: into_extra(d) }))
}

fn read_path(mut d: Dict) -> Option<Shape> {
    let closed = as_bool01(&take(&mut d, "closed")?, "closed").ok()?;
    let mut nodes = as_items(take(&mut d, "nodes")?, "nodes").ok()?.iter().map(read_node).collect::<Option<Vec<_>>>()?;
    // The format writes a closed path's start node last; the model keeps it first.
    if closed && !nodes.is_empty() {
        nodes.rotate_right(1);
    }
    let path = Path { closed, nodes, extra: into_extra(d) };
    // A node list that does not make an outline is kept as written, not rejected with the file.
    path.to_bez().ok()?;
    Some(Shape::Path(path))
}

fn read_node(v: &Value) -> Option<Node> {
    let items = &v.as_array()?.items;
    let (x, y, flags, attr) = match items.as_slice() {
        [x, y, f] => (x, y, f, None),
        [x, y, f, a] => (x, y, f, Some(a)),
        _ => return None,
    };
    let flags = flags.as_str()?;
    let mut chars = flags.chars();
    let kind = match chars.next()? {
        'l' => NodeKind::Line,
        'c' => NodeKind::Curve,
        'q' => NodeKind::QCurve,
        'o' => NodeKind::OffCurve,
        _ => return None,
    };
    let rest = chars.as_str();
    let (smooth, rest) = match rest.strip_prefix('s') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let mut extra = Extra::new();
    if !rest.is_empty() {
        extra.push(Family::Glyphs, "flags", Value::String(rest.into()));
    }
    if let Some(a) = attr {
        extra.push(Family::Glyphs, "attr", a.clone());
    }
    Some(Node { x: x.as_num()?.as_f64(), y: y.as_num()?.as_f64(), kind, smooth, extra })
}

fn read_anchor(v: Value, at: &str) -> Result<Anchor> {
    let mut d = as_dict(v, at)?;
    let name = opt_string(&mut d, "name", at)?.unwrap_or_default();
    let (x, y) = match take(&mut d, "pos") {
        Some(p) => as_pair(&p, &format!("{at} anchor `{name}`"))?,
        None => (0.0, 0.0),
    };
    Ok(Anchor { name, x, y, extra: into_extra(d) })
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

fn num(f: f64, at: &str) -> Result<Value> {
    Number::float(f).map(Value::Number).ok_or_else(|| bad(at, "a value is not a finite number"))
}

fn int(i: impl Into<i64>) -> Value {
    Value::Number(Number::int(i.into()))
}

fn string(s: &str) -> Value {
    Value::String(Str::new(s))
}

fn list(items: Vec<Value>) -> Value {
    Value::Array(Array { items, layout: Layout::Multiline })
}

fn pair(a: f64, b: f64, at: &str) -> Result<Value> {
    Ok(Value::Array(Array { items: vec![num(a, at)?, num(b, at)?], layout: Layout::Compact }))
}

/// The fields' entries plus the extra entries of this family, keys sorted byte-wise. A field
/// wins over an extra entry with the same key.
fn finish(mut entries: Vec<(Str, Value)>, extra: &Extra) -> Dict {
    for e in extra.of(Family::Glyphs) {
        if !entries.iter().any(|(k, _)| k.as_str() == e.key.as_str()) {
            entries.push((e.key.clone(), e.value.clone()));
        }
    }
    entries.sort_by(|a, b| a.0.as_str().as_bytes().cmp(b.0.as_str().as_bytes()));
    Dict { entries, layout: Layout::Multiline }
}

/// Writes the family name into a version 4 file's `familyNames` property (its first value),
/// adding the property when the file has none.
fn sync_v4_family_name(extra: &Extra, family: &str) -> Extra {
    let mut extra = extra.clone();
    if v4_family_name_of_extra(&extra).as_deref() == Some(family) {
        return extra;
    }
    let mut props = match extra.get(Family::Glyphs, "properties") {
        Some(Value::Array(a)) => a.clone(),
        _ => Array { items: Vec::new(), layout: Layout::Multiline },
    };
    let new_value = || {
        let mut v = Dict::new();
        v.insert("language", string("dflt"));
        v.insert("value", string(family));
        Value::Dict(v)
    };
    let found = props.items.iter_mut().find_map(|p| match p {
        Value::Dict(d) if d.get("key").and_then(Value::as_str) == Some("familyNames") => Some(d),
        _ => None,
    });
    match found {
        Some(d) => match d.get_mut("values") {
            Some(Value::Array(values)) => match values.items.first_mut() {
                Some(Value::Dict(first)) => first.insert("value", string(family)),
                _ => values.items.insert(0, new_value()),
            },
            _ => d.insert("values", list(vec![new_value()])),
        },
        None => {
            let mut entry = Dict::new();
            entry.insert("key", string("familyNames"));
            entry.insert("values", list(vec![new_value()]));
            props.items.insert(0, Value::Dict(entry));
        }
    }
    extra.insert(Family::Glyphs, "properties", Value::Array(props));
    extra
}

fn v4_family_name_of_extra(extra: &Extra) -> Option<String> {
    let mut d = Dict::new();
    d.insert("properties", extra.get(Family::Glyphs, "properties")?.clone());
    v4_family_name(&d)
}

fn write_font(font: &Font, version: u32) -> Result<Dict> {
    let mut e: Vec<(Str, Value)> = vec![(".formatVersion".into(), int(version))];
    let synced;
    let extra: &Extra = if version >= 4 && !font.family_name.is_empty() {
        synced = sync_v4_family_name(&font.extra, &font.family_name);
        &synced
    } else {
        &font.extra
    };
    if version < 4 && !font.family_name.is_empty() {
        e.push(("familyName".into(), string(&font.family_name)));
    }
    e.push(("unitsPerEm".into(), int(font.units_per_em)));
    e.push(("versionMajor".into(), int(font.version_major)));
    e.push(("versionMinor".into(), int(font.version_minor)));
    if !font.metrics.is_empty() {
        e.push(("metrics".into(), list(font.metrics.iter().map(write_metric).collect())));
    }
    if !font.axes.is_empty() {
        e.push(("axes".into(), list(font.axes.iter().map(write_axis).collect())));
    }
    if !font.instances.is_empty() {
        e.push(("instances".into(), list(font.instances.iter().map(write_instance).collect::<Result<_>>()?)));
    }
    e.push(("fontMaster".into(), list(font.masters.iter().map(write_master).collect::<Result<_>>()?)));
    e.push(("glyphs".into(), list(font.glyphs.iter().map(|g| write_glyph(g).map(Value::Dict)).collect::<Result<_>>()?)));
    for (key, table) in [("kerningLTR", &font.kerning.ltr), ("kerningRTL", &font.kerning.rtl), ("kerningVertical", &font.kerning.vertical)] {
        if !table.is_empty() {
            e.push((key.into(), write_kern_table(table)?));
        }
    }
    Ok(finish(e, extra))
}

fn write_metric(m: &MetricDef) -> Value {
    let mut e = Vec::new();
    if !m.kind.is_empty() {
        e.push(("type".into(), string(&m.kind)));
    }
    if let Some(n) = &m.name {
        e.push(("name".into(), string(n)));
    }
    if let Some(f) = &m.filter {
        e.push(("filter".into(), string(f)));
    }
    Value::Dict(finish(e, &m.extra))
}

fn write_master(m: &Master) -> Result<Value> {
    let mut e = vec![("id".into(), string(&m.id))];
    if !m.name.is_empty() {
        e.push(("name".into(), string(&m.name)));
    }
    if !m.location.is_empty() {
        e.push(("axesValues".into(), write_numbers(&m.location, "axesValues")?));
    }
    if !m.metric_values.is_empty() {
        let values = m
            .metric_values
            .iter()
            .map(|v| {
                let mut d = Vec::new();
                if v.over != 0.0 {
                    d.push(("over".into(), num(v.over, "metricValues")?));
                }
                if v.pos != 0.0 {
                    d.push(("pos".into(), num(v.pos, "metricValues")?));
                }
                Ok(Value::Dict(Dict { entries: d, layout: Layout::Multiline }))
            })
            .collect::<Result<_>>()?;
        e.push(("metricValues".into(), list(values)));
    }
    Ok(Value::Dict(finish(e, &m.extra)))
}

/// One glyph's dictionary (a package writes each to its own file).
pub fn write_glyph(g: &Glyph) -> Result<Dict> {
    let at = format!("glyph `{}`", g.name);
    let mut e = vec![("glyphname".into(), string(&g.name))];
    match g.unicodes.as_slice() {
        [] => {}
        [u] => e.push(("unicode".into(), int(*u))),
        // Several code points go on one line, unlike other lists.
        many => e.push(("unicode".into(), Value::Array(Array { items: many.iter().map(|u| int(*u)).collect(), layout: Layout::Compact }))),
    }
    if !g.export {
        e.push(("export".into(), int(0)));
    }
    push_kern_groups(&mut e, &g.kern_groups, KERN_GROUP_KEYS);
    e.push(("layers".into(), list(g.layers.iter().map(|l| write_layer(l, &at)).collect::<Result<_>>()?)));
    Ok(finish(e, &g.extra))
}

fn write_layer(l: &Layer, at: &str) -> Result<Value> {
    let at = format!("{at} layer `{}`", l.id);
    let mut e = vec![("layerId".into(), string(&l.id))];
    if l.id != l.master {
        e.push(("associatedMasterId".into(), string(&l.master)));
    }
    e.push(("width".into(), num(l.width, &at)?));
    if !l.shapes.is_empty() {
        e.push(("shapes".into(), list(l.shapes.iter().map(|s| write_shape(s, &at)).collect::<Result<_>>()?)));
    }
    if !l.anchors.is_empty() {
        e.push(("anchors".into(), list(l.anchors.iter().map(|a| write_anchor(a, &at)).collect::<Result<_>>()?)));
    }
    Ok(Value::Dict(finish(e, &l.extra)))
}

fn write_shape(s: &Shape, at: &str) -> Result<Value> {
    match s {
        Shape::Raw(r) if r.family == Family::Glyphs => Ok(r.value.clone()),
        Shape::Raw(_) => Err(GlyphsError::Unsupported(format!("{at}: a shape read from another format"))),
        Shape::Component(c) => write_component(c, at),
        Shape::Path(p) => write_path(p, at),
    }
}

fn write_component(c: &Component, at: &str) -> Result<Value> {
    let (pos, scale, angle, slant) = match c.transform {
        Transform::Decomposed { pos, scale, angle, slant } => (pos, scale, angle, slant),
        Transform::Matrix(m) => decompose(m).ok_or_else(|| GlyphsError::Unsupported(format!("{at}: a component transform with shear")))?,
    };
    let mut e = vec![("ref".into(), string(&c.glyph))];
    if pos != (0.0, 0.0) {
        e.push(("pos".into(), pair(pos.0, pos.1, at)?));
    }
    if scale != (1.0, 1.0) {
        e.push(("scale".into(), pair(scale.0, scale.1, at)?));
    }
    if angle != 0.0 {
        e.push(("angle".into(), num(angle, at)?));
    }
    if slant != (0.0, 0.0) {
        e.push(("slant".into(), pair(slant.0, slant.1, at)?));
    }
    Ok(Value::Dict(finish(e, &c.extra)))
}

/// A matrix as position, scale and rotation, when it has no shear.
type Decomposed = ((f64, f64), (f64, f64), f64, (f64, f64));
fn decompose([a, b, c, d, dx, dy]: [f64; 6]) -> Option<Decomposed> {
    if b == 0.0 && c == 0.0 {
        return Some(((dx, dy), (a, d), 0.0, (0.0, 0.0)));
    }
    let sx = a.hypot(b);
    if sx == 0.0 {
        return None;
    }
    let angle = b.atan2(a);
    let sy = (a * d - b * c) / sx;
    // Without shear the second column is the first turned a quarter and scaled by sy / sx.
    let (cc, dd) = (-angle.sin() * sy, angle.cos() * sy);
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * (1.0 + x.abs().max(y.abs()));
    (close(c, cc) && close(d, dd)).then(|| ((dx, dy), (sx, sy), angle.to_degrees(), (0.0, 0.0)))
}

fn write_path(p: &Path, at: &str) -> Result<Value> {
    let mut nodes: Vec<Value> = p.nodes.iter().map(|n| write_node(n, at)).collect::<Result<_>>()?;
    if p.closed && !nodes.is_empty() {
        nodes.rotate_left(1);
    }
    let e = vec![("closed".into(), int(i64::from(p.closed))), ("nodes".into(), list(nodes))];
    Ok(Value::Dict(finish(e, &p.extra)))
}

fn write_node(n: &Node, at: &str) -> Result<Value> {
    let mut flags = String::from(match n.kind {
        NodeKind::Line => "l",
        NodeKind::Curve => "c",
        NodeKind::QCurve => "q",
        NodeKind::OffCurve => "o",
    });
    if n.smooth {
        flags.push('s');
    }
    if let Some(rest) = n.extra.get(Family::Glyphs, "flags").and_then(Value::as_str) {
        flags.push_str(rest);
    }
    let mut items = vec![num(n.x, at)?, num(n.y, at)?, string(&flags)];
    if let Some(attr) = n.extra.get(Family::Glyphs, "attr") {
        items.push(attr.clone());
    }
    Ok(Value::Array(Array { items, layout: Layout::Compact }))
}

fn write_anchor(a: &Anchor, at: &str) -> Result<Value> {
    let mut e = vec![("name".into(), string(&a.name))];
    if (a.x, a.y) != (0.0, 0.0) {
        e.push(("pos".into(), pair(a.x, a.y, at)?));
    }
    Ok(Value::Dict(finish(e, &a.extra)))
}

#[cfg(test)]
mod tests;
