//! The primary reference's source format, `.glyphs` (L3): reading into the model and writing back.
//!
//! The reader moves each key the model has a field for out of its dictionary and into that field;
//! every other key goes to the object's [`Extra`] as a property-list value with its spelling and
//! layout. The writer builds the dictionaries back from the fields and the extra entries, and
//! sorts their keys byte-wise, as the reference does (`docs/plans/M1.md` § 5). Keys whose value is
//! the documented default are left out, as the reference leaves them out.
//!
//! Written from the format documents and JSON schemas in GlyphsSDK (Apache-2.0) and from files in
//! the test corpus. Format versions 2 ([`v2`]), 3 and 4 are read and written. The decomposition of
//! a matrix into the format's transform fields (`decompose`) is ported from glyphsLib's
//! `decomposeGlyphs3Transform` (Apache-2.0) and fontTools' `solveQuadratic` (MIT); see `NOTICE`.
//!
//! Paths, nodes, components, anchors and format 4 image shapes are read and written by
//! [`shape`]. Shapes the model cannot hold yet (format 4 shape groups, paths with quartic, Hobby
//! or spiral nodes, paths whose node list does not make an outline) are kept as [`Shape::Raw`].
#![forbid(unsafe_code)]

use std::sync::Arc;

use fontcraft_model::{
    Anchor, Axis, Extra, Family, Features, Font, FontInfo, Glyph, GlyphTable, Instance, KernContext, KernFirst, KernGroups, KernTable, Kerning,
    Layer, Master, MasterKerning, MetricDef, MetricValue, ModelError, PropertyValue, RawShape, Shape, Snippet, Source, SourceFormat,
};
use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};

mod design;
mod glyph;
mod groups;
mod info;
mod layer;
mod package;
mod shape;
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

/// What [`write`] leaves out of the document because the format version it writes has no place
/// for it, one line each: zones beyond the overshoots (versions 3 and 4), the axis data only a
/// designspace states, and glyph metadata the version has no key for.
pub fn info_losses(font: &Font) -> Vec<String> {
    let mut out = glyph::losses(&font.glyphs, font.source.glyphs_version.unwrap_or(3));
    // The format has no booleans: user data read from a UFO or a designspace comes back with 1
    // and 0 for them.
    let n = bool_user_data(font);
    if n > 0 {
        out.push(format!("booleans in user data ({n} objects), which the format holds as 1 and 0"));
    }
    out.extend(layer::losses(&font.glyphs, font.source.glyphs_version.unwrap_or(3)));
    out.extend(shape::losses(&font.glyphs, font.source.glyphs_version.unwrap_or(3)));
    // Versions 3 and 4 hold alignment zones only as the metrics' overshoots; version 2 writes
    // the other zones into `alignmentZones`.
    if font.source.glyphs_version.unwrap_or(3) != 2 {
        for m in font.masters.iter().filter(|m| !m.zones.is_empty()) {
            let name = if m.name.is_empty() { &m.id } else { &m.name };
            out.push(format!("master `{name}`: alignment zones other than overshoots ({})", m.zones.len()));
        }
        // Name parts are version 2's.
        for m in font.masters.iter().filter(|m| !m.name_parts.is_empty()) {
            let name = if m.name.is_empty() { &m.id } else { &m.name };
            out.push(format!("master `{name}`: name parts"));
        }
    }
    // Contextual kerning is version 4's; feature labels are not in version 2.
    let version = font.source.glyphs_version.unwrap_or(3);
    if version < 4 && !font.kerning.context.is_empty() {
        out.push(format!("contextual kerning ({} contexts)", font.kerning.context.len()));
    }
    if version == 2 {
        out.extend(v2::property_losses(font));
        out.extend(v2::instance_property_losses(font));
        let labelled = font.features.features.iter().filter(|f| !f.labels.is_empty()).count();
        if labelled > 0 {
            out.push(format!("feature labels ({labelled} features)"));
        }
    }
    // A designspace's axis data the format has no key for. The range travels while the axis
    // parameters give it back.
    for (i, a) in font.axes.iter().enumerate() {
        let mut parts = Vec::new();
        if let Some(r) = a.range {
            let m = fontcraft_model::axes::mapping(font, i);
            if (m.minimum(), m.default, m.maximum()) != (r.minimum, r.default, r.maximum) {
                parts.push("its range");
            }
        }
        if !a.values.is_empty() {
            parts.push("its discrete values");
        }
        if !a.label_names.is_empty() {
            parts.push("its name in other languages");
        }
        if !a.labels.is_empty() || a.ordering.is_some() {
            parts.push("its STAT labels");
        }
        if !parts.is_empty() {
            out.push(format!("axis `{}`: {}", a.name, parts.join(", ")));
        }
    }
    out
}

/// The root dictionary of a document, and the dialect its version is written in.
fn write_root(font: &Font) -> Result<(Dict, Dialect)> {
    let version = font.source.glyphs_version.unwrap_or(3);
    let dialect = match version {
        2 => Dialect::Glyphs2,
        3 => Dialect::Glyphs3,
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

/// The family name of a version 4 file, which keeps it among the `properties`: the first value
/// of `familyNames`.
fn v4_family_name(props: &fontcraft_model::Properties) -> Option<String> {
    match &props.get("familyNames")?.value {
        PropertyValue::Localized(v) => v.first().map(|l| l.value.clone()),
        PropertyValue::Single(s) => Some(s.clone()),
    }
}

/// `.appVersion`, when the writer would spell it the same way.
pub(crate) fn take_app_version(d: &mut Dict) -> Option<String> {
    match d.get(".appVersion") {
        Some(Value::String(s)) if s.spelling().is_none() => take(d, ".appVersion").and_then(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

/// The font information of a version 3 or 4 root: properties, parameters, settings, date and
/// user data.
fn take_font_info(d: &mut Dict) -> Result<FontInfo> {
    Ok(FontInfo {
        properties: info::take_properties(d),
        params: info::take_params(d),
        settings: info::take_settings(d),
        date: take_plain_string(d, "date"),
        user_data: info::take_user_data(d),
    })
}

/// Version 2's top-level names that a version 3 or 4 file holds (hand-written files do): each
/// fills its property when the file has none, as fontc reads them. The key stays in the font's
/// extra data, and [`without_kept_v2_names`] keeps the writer from writing the name twice.
fn v2_names_as_properties(root: &Dict, properties: &mut fontcraft_model::Properties) {
    for (key, property, localized) in v2::NAMES {
        if properties.get(property).is_none()
            && let Some(text) = root.get(key).and_then(Value::as_str)
        {
            properties.set(property, text, localized);
        }
    }
}

/// The properties to write: those [`v2_names_as_properties`] made from a top-level key that
/// still holds the same text are left out (the key is written instead).
fn without_kept_v2_names<'a>(properties: &'a fontcraft_model::Properties, extra: &Extra) -> std::borrow::Cow<'a, fontcraft_model::Properties> {
    let kept = |p: &fontcraft_model::Property| {
        v2::NAMES.iter().any(|(key, property, _)| {
            p.key == *property
                && extra.get(Family::Glyphs, key).and_then(Value::as_str).is_some_and(|text| match &p.value {
                    PropertyValue::Single(s) => s == text,
                    PropertyValue::Localized(v) => {
                        matches!(v.as_slice(), [only] if only.language == fontcraft_model::DEFAULT_LANGUAGE && only.value == text)
                    }
                })
        })
    };
    if properties.0.iter().any(kept) {
        std::borrow::Cow::Owned(fontcraft_model::Properties(properties.0.iter().filter(|p| !kept(p)).cloned().collect()))
    } else {
        std::borrow::Cow::Borrowed(properties)
    }
}

/// The font's extra data without the version 2 names whose property was deleted: the reader made
/// a property from each, so writing the key back would bring the property back on reopening.
pub(crate) fn without_deleted_v2_names<'a>(properties: &fontcraft_model::Properties, extra: &'a Extra) -> std::borrow::Cow<'a, Extra> {
    let deleted: Vec<&str> = v2::NAMES
        .iter()
        .filter(|(key, property, _)| properties.get(property).is_none() && extra.get(Family::Glyphs, key).and_then(Value::as_str).is_some())
        .map(|(key, _, _)| *key)
        .collect();
    if deleted.is_empty() {
        return std::borrow::Cow::Borrowed(extra);
    }
    let mut out = extra.clone();
    for key in deleted {
        out.remove(Family::Glyphs, key);
    }
    std::borrow::Cow::Owned(out)
}

/// A string the writer would spell the same way.
pub(crate) fn take_plain_string(d: &mut Dict, key: &str) -> Option<String> {
    match d.get(key) {
        Some(Value::String(s)) if s.spelling().is_none() => take(d, key).and_then(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

/// The parts of the font information every version writes the same way, with the font's
/// parameters as `params` (version 2 may rewrite its `Axes` parameter).
pub(crate) fn push_font_info_common(e: &mut Vec<(Str, Value)>, font: &Font, params: &fontcraft_model::Params) {
    if let Some(v) = &font.source.app_version {
        e.push((".appVersion".into(), string(v)));
    }
    if let Some(d) = &font.info.date {
        e.push(("date".into(), string(d)));
    }
    info::push_params(e, params);
    info::push_user_data(e, &groups::user_data(font));
}

fn read_font(mut root: Dict) -> Result<Font> {
    let version = match take(&mut root, ".formatVersion") {
        Some(v) => as_int(&v, ".formatVersion")?,
        None => return v2::read_font(root),
    };
    if !(3..=4).contains(&version) {
        return Err(GlyphsError::Unsupported(format!("format version {version}")));
    }
    let app_version = take_app_version(&mut root);
    let mut font_info = take_font_info(&mut root)?;
    v2_names_as_properties(&root, &mut font_info.properties);
    let family_name = if version >= 4 {
        v4_family_name(&font_info.properties).unwrap_or_default()
    } else {
        take(&mut root, "familyName").map(|v| as_string(v, "familyName")).transpose()?.unwrap_or_default()
    };
    let units_per_em = as_u16(&take(&mut root, "unitsPerEm").ok_or_else(|| bad("the file", "no unitsPerEm"))?, "unitsPerEm")?;
    let version_major = take(&mut root, "versionMajor").map(|v| as_u16(&v, "versionMajor")).transpose()?.unwrap_or(0);
    let version_minor = take(&mut root, "versionMinor").map(|v| as_u16(&v, "versionMinor")).transpose()?.unwrap_or(0);
    let mut features = take_features(&mut root, "tag");
    let groups = groups::take_groups(&mut features.classes, &mut font_info.user_data);
    let display_strings = take_strings(&mut root, "DisplayStrings");
    let note = take_note(&mut root);
    let axes = take_typed_list(&mut root, "axes", read_axis);
    let instances = take_typed_list(&mut root, "instances", read_instance);
    let stems = design::take_stem_defs(&mut root);
    let numbers = design::take_number_defs(&mut root);
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
            .map(|(i, g)| read_glyph(g, &format!("glyphs[{i}]"), version).map(Arc::new))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let kerning = Kerning {
        ltr: take_kern_table(&mut root, "kerningLTR"),
        rtl: take_kern_table(&mut root, "kerningRTL"),
        vertical: take_kern_table(&mut root, "kerningVertical"),
        context: if version >= 4 { take_kern_context(&mut root) } else { Vec::new() },
    };
    Ok(Font {
        source: Source { format: SourceFormat::Glyphs, glyphs_version: u32::try_from(version).ok(), app_version, ..Default::default() },
        info: Arc::new(font_info),
        family_name,
        units_per_em,
        version_major,
        version_minor,
        features: Arc::new(features),
        groups: Arc::new(groups),
        display_strings: Arc::new(display_strings),
        note,
        axes: Arc::new(axes),
        instances: Arc::new(instances),
        metrics: Arc::new(metrics),
        stems: Arc::new(stems),
        numbers: Arc::new(numbers),
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
    Some(KernTable::new(out))
}

/// A kerning table as the format writes it: in the table's own order, not sorted. Group sides
/// read from a UFO get this format's prefixes.
fn write_kern_table(t: &KernTable) -> Result<Value> {
    let mut masters = Dict::new();
    for m in &t.0 {
        let mut firsts = Dict::new();
        for f in &m.firsts {
            let mut seconds = Dict::new();
            for (s, v) in &f.seconds {
                seconds.entries.push((Str::new(fontcraft_model::glyphs_side(s).as_ref()), num(*v, "kerning")?));
            }
            firsts.entries.push((Str::new(fontcraft_model::glyphs_side(&f.first).as_ref()), Value::Dict(seconds)));
        }
        masters.entries.push((Str::new(m.master.as_str()), Value::Dict(firsts)));
    }
    Ok(Value::Dict(masters))
}

/// Version 4's `kerningContext`, `{context: {master: value}}`, taken out of `d` when every value
/// is a number and writing it back gives the same text; otherwise it stays as written.
fn take_kern_context(d: &mut Dict) -> Vec<KernContext> {
    const KEY: &str = "kerningContext";
    let Some(v) = d.get(KEY) else { return Vec::new() };
    let Some(contexts) = v.as_dict() else { return Vec::new() };
    if contexts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(contexts.len());
    for (context, values) in &contexts.entries {
        let Some(values) = values.as_dict() else { return Vec::new() };
        let Some(values) = values.entries.iter().map(|(m, v)| Some((m.as_str().to_string(), v.as_num()?.as_f64()))).collect::<Option<Vec<_>>>()
        else {
            return Vec::new();
        };
        out.push(KernContext { context: context.as_str().to_string(), values });
    }
    match write_kern_context(&out) {
        Ok(w) if design::same_text(&w, v) => {
            take(d, KEY);
            out
        }
        _ => Vec::new(),
    }
}

/// `kerningContext` as version 4 writes it, in the model's order.
fn write_kern_context(contexts: &[KernContext]) -> Result<Value> {
    let mut out = Dict::new();
    for c in contexts {
        let mut values = Dict::new();
        for (master, v) in &c.values {
            values.entries.push((Str::new(master.as_str()), num(*v, "kerningContext")?));
        }
        out.entries.push((Str::new(c.context.as_str()), Value::Dict(values)));
    }
    Ok(Value::Dict(out))
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

/// The prefixes, classes and features; `tag_key` is the key a feature names its tag with
/// (`tag`, or `name` in format version 2).
fn take_features(d: &mut Dict, tag_key: &'static str) -> Features {
    let prefixes = take_typed_list(d, "featurePrefixes", |v| read_snippet(v, "name"));
    let classes = take_typed_list(d, "classes", |v| read_snippet(v, "name"));
    let features = if tag_key == "tag" {
        take_typed_list(d, "features", |v| read_snippet(v, "tag"))
    } else {
        take_typed_list(d, "features", |v| read_snippet(v, "name"))
    };
    Features { prefixes, classes, features }
}

fn read_snippet(v: &Value, name_key: &str) -> Option<Snippet> {
    let mut d = v.as_dict()?.clone();
    let name = take(&mut d, name_key)?.as_str()?.to_string();
    let code = take(&mut d, "code")?.as_str()?.to_string();
    let flag = |d: &mut Dict, k: &str| match take(d, k) {
        Some(x) => as_bool01(&x, k).ok(),
        None => Some(false),
    };
    let automatic = flag(&mut d, "automatic")?;
    let disabled = flag(&mut d, "disabled")?;
    let notes = match take(&mut d, "notes") {
        Some(n) => n.as_str()?.to_string(),
        None => String::new(),
    };
    let labels = design::take_checked(&mut d, "labels", info::read_localized, |l| Some(info::write_localized(l)));
    Some(Snippet { name, code, automatic, disabled, notes, labels, file_name: None, extra: into_extra(d) })
}

fn write_snippet(s: &Snippet, name_key: &str) -> Value {
    let mut e = vec![(name_key.into(), string(&s.name)), ("code".into(), string(&s.code))];
    if s.automatic {
        e.push(("automatic".into(), int(1)));
    }
    if s.disabled {
        e.push(("disabled".into(), int(1)));
    }
    if !s.notes.is_empty() {
        e.push(("notes".into(), string(&s.notes)));
    }
    if !s.labels.is_empty() {
        e.push(("labels".into(), list(s.labels.iter().map(info::write_localized).collect())));
    }
    Value::Dict(finish(e, &s.extra))
}

/// The prefixes, the classes (the font's groups after the feature classes) and the features.
fn push_features(e: &mut Vec<(Str, Value)>, font: &Font, tag_key: &str) {
    let f = &font.features;
    let classes = groups::classes(font);
    for (key, list_of, name_key) in
        [("featurePrefixes", f.prefixes.as_slice(), "name"), ("classes", &classes, "name"), ("features", f.features.as_slice(), tag_key)]
    {
        if !list_of.is_empty() {
            e.push((key.into(), list(list_of.iter().map(|s| write_snippet(s, name_key)).collect())));
        }
    }
}

/// A list of strings, such as `DisplayStrings`; it stays as written if it holds anything else.
fn take_strings(d: &mut Dict, key: &str) -> Vec<String> {
    let Some(Value::Array(a)) = d.get(key) else { return Vec::new() };
    match a.items.iter().map(|s| s.as_str().map(str::to_string)).collect::<Option<Vec<_>>>() {
        Some(s) => {
            take(d, key);
            s
        }
        None => Vec::new(),
    }
}

fn take_note(d: &mut Dict) -> String {
    match d.get("note").and_then(Value::as_str).map(str::to_string) {
        Some(n) => {
            take(d, "note");
            n
        }
        None => String::new(),
    }
}

fn push_ui_and_note(e: &mut Vec<(Str, Value)>, font: &Font) {
    if !font.display_strings.is_empty() {
        e.push(("DisplayStrings".into(), list(font.display_strings.iter().map(|s| string(s)).collect())));
    }
    if !font.note.is_empty() {
        e.push(("note".into(), string(&font.note)));
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
    Some(Axis { name, tag, hidden, extra: into_extra(d), ..Default::default() })
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
    let mut i = Instance { name, location, exports, ..Default::default() };
    design::take_instance_fields(&mut d, &mut i);
    i.properties = info::take_properties(&mut d);
    i.params = info::take_params(&mut d);
    i.user_data = info::take_user_data(&mut d);
    i.extra = into_extra(d);
    Some(i)
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
    design::push_instance_fields(&mut e, i)?;
    info::push_properties(&mut e, &i.properties);
    info::push_params(&mut e, &i.params);
    info::push_user_data(&mut e, &i.user_data);
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
    let (icon, visible) = take_master_display(&mut d)?;
    let stem_values = design::take_values(&mut d, "stemValues");
    let number_values = design::take_values(&mut d, "numberValues");
    let guides = design::take_guides(&mut d);
    let properties = info::take_properties(&mut d);
    let params = info::take_params(&mut d);
    let user_data = info::take_user_data(&mut d);
    Ok(Master {
        id,
        name,
        location,
        metric_values,
        icon,
        visible,
        stem_values,
        number_values,
        guides,
        properties,
        params,
        user_data,
        extra: into_extra(d),
        ..Default::default()
    })
}

/// A master's `iconName` and `visible`.
pub(crate) fn take_master_display(d: &mut Dict) -> Result<(Option<String>, bool)> {
    let icon = take_plain_string(d, "iconName");
    let visible = match d.get("visible") {
        Some(v) if matches!(v.as_num(), Some(Num::Int(1))) => {
            take(d, "visible");
            true
        }
        _ => false,
    };
    Ok((icon, visible))
}

/// What [`take_master_display`] took, and the master's properties, parameters and user data.
pub(crate) fn push_master_info(e: &mut Vec<(Str, Value)>, m: &Master) {
    if let Some(i) = &m.icon {
        e.push(("iconName".into(), string(i)));
    }
    if m.visible {
        e.push(("visible".into(), int(1)));
    }
    info::push_properties(e, &m.properties);
    info::push_params(e, &m.params);
    info::push_user_data(e, &m.user_data);
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

fn read_glyph(v: Value, at: &str, version: i64) -> Result<Glyph> {
    let mut d = as_dict(v, at)?;
    let name = opt_string(&mut d, "glyphname", at)?.ok_or_else(|| bad(at, "a glyph has no glyphname"))?;
    let at = format!("glyph `{name}`");
    // Code points the writer would spell otherwise (`0062`, read as decimal like other numbers)
    // keep their spelling beside the values; those that are not numbers are kept as written.
    let unicodes = match take(&mut d, "unicode") {
        None => Vec::new(),
        Some(v) => match code_points(&v) {
            Some(u) => {
                if write_code_points(&u).as_ref() != Some(&v) {
                    d.entries.push(("unicode".into(), v));
                }
                u
            }
            None => {
                d.entries.push(("unicode".into(), v));
                Vec::new()
            }
        },
    };
    let export = take(&mut d, "export").map(|v| as_bool01(&v, &format!("{at}.export"))).transpose()?.unwrap_or(true);
    let kern_groups = take_kern_groups(&mut d, KERN_GROUP_KEYS, &at)?;
    let info = glyph::take_glyph_info(&mut d, u32::try_from(version).unwrap_or(3));
    let layers = match take(&mut d, "layers") {
        Some(v) => as_items(v, &at)?.into_iter().map(|l| read_layer(l, &at, u32::try_from(version).unwrap_or(3))).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Glyph { name, unicodes, export, kern_groups, info: fontcraft_model::Rare::new(info), layers, file_name: None, extra: into_extra(d) })
}

fn code_points(v: &Value) -> Option<Vec<u32>> {
    // A number, or a string of digits read as a decimal number (as fontc reads `0062`).
    let one = |v: &Value| match v {
        Value::Number(n) => match n.value() {
            Num::Int(i) => u32::try_from(i).ok(),
            Num::Float(_) => None,
        },
        Value::String(s) if !s.as_str().is_empty() && s.as_str().bytes().all(|b| b.is_ascii_digit()) => s.as_str().parse().ok(),
        _ => None,
    };
    match v {
        Value::Array(a) => a.items.iter().map(one).collect(),
        v => one(v).map(|u| vec![u]),
    }
}

/// The canonical `unicode` value of code points: a number, or a one-line list of them.
fn write_code_points(u: &[u32]) -> Option<Value> {
    match u {
        [] => None,
        [u] => Some(int(*u)),
        // Several code points go on one line, unlike other lists.
        many => Some(Value::Array(Array { items: many.iter().map(|u| int(*u)).collect(), layout: Layout::Compact, ..Default::default() })),
    }
}

/// How versions 3 and 4 read and write the shapes and anchors of a background.
const SHAPES: layer::Shapes = layer::Shapes { take: take_shapes, push: push_shapes, finish };

/// A background's `shapes` and `anchors`; an empty list stays as written.
fn take_shapes(d: &mut Dict, version: u32) -> Option<(Vec<Shape>, Vec<Anchor>)> {
    let filled = |d: &Dict, k: &str| matches!(d.get(k), Some(Value::Array(a)) if !a.items.is_empty());
    let shapes = if filled(d, "shapes") {
        as_items(take(d, "shapes")?, "shapes").ok()?.into_iter().map(|s| shape::read_shape(s, version)).collect()
    } else {
        Vec::new()
    };
    let anchors = if filled(d, "anchors") {
        as_items(take(d, "anchors")?, "anchors")
            .ok()?
            .into_iter()
            .map(|a| shape::read_anchor(a, "background", version).ok().filter(crate::layer::anchor_in_range))
            .collect::<Option<_>>()?
    } else {
        Vec::new()
    };
    Some((shapes, anchors))
}

fn push_shapes(e: &mut Vec<(Str, Value)>, shapes: &[Shape], anchors: &[Anchor], at: &str, version: u32) -> Result<()> {
    if !shapes.is_empty() {
        e.push(("shapes".into(), list(shapes.iter().map(|s| shape::write_shape(s, at, version)).collect::<Result<_>>()?)));
    }
    if !anchors.is_empty() {
        e.push(("anchors".into(), list(anchors.iter().map(|a| shape::write_anchor(a, at, version)).collect::<Result<_>>()?)));
    }
    Ok(())
}

/// The width of a layer that states none. The reference app always writes one; hand-written
/// files may not, and glyphsLib and fontc read 600 then.
pub(crate) const DEFAULT_LAYER_WIDTH: f64 = 600.0;

fn read_layer(v: Value, at: &str, version: u32) -> Result<Layer> {
    let mut d = as_dict(v, at)?;
    let id = opt_string(&mut d, "layerId", at)?.ok_or_else(|| bad(at, "a layer has no layerId"))?;
    let at = format!("{at} layer `{id}`");
    let master = opt_string(&mut d, "associatedMasterId", &at)?.unwrap_or_else(|| id.clone());
    let width = take(&mut d, "width").map(|v| as_f64(&v, &format!("{at}.width"))).transpose()?.unwrap_or(DEFAULT_LAYER_WIDTH);
    let shapes = match take(&mut d, "shapes") {
        Some(v) => as_items(v, &at)?.into_iter().map(|s| shape::read_shape(s, version)).collect(),
        None => Vec::new(),
    };
    let anchors = match take(&mut d, "anchors") {
        Some(v) => as_items(v, &at)?.into_iter().map(|a| shape::read_anchor(a, &at, version)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let info = layer::take_layer_info(&mut d, version, &SHAPES);
    Ok(Layer { id, master, width, shapes, anchors, info: fontcraft_model::Rare::new(info), extra: into_extra(d) })
}

pub(crate) fn raw(value: Value) -> Shape {
    Shape::Raw(RawShape { family: Family::Glyphs, value })
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
    Value::Array(Array { items, layout: Layout::Multiline, ..Default::default() })
}

fn pair(a: f64, b: f64, at: &str) -> Result<Value> {
    Ok(Value::Array(Array { items: vec![num(a, at)?, num(b, at)?], layout: Layout::Compact, ..Default::default() }))
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
    Dict { entries, layout: Layout::Multiline, ..Default::default() }
}

/// Writes the family name into a version 4 file's `familyNames` property (its first value),
/// adding the property when the file has none.
/// The properties with the family name as the first value of `familyNames` (added first when
/// there is none).
fn sync_v4_family_name(props: &fontcraft_model::Properties, family: &str) -> fontcraft_model::Properties {
    let mut props = props.clone();
    if v4_family_name(&props).as_deref() == Some(family) {
        return props;
    }
    let value = || fontcraft_model::Localized { language: fontcraft_model::DEFAULT_LANGUAGE.into(), value: family.into(), extra: Extra::new() };
    match props.0.iter_mut().find(|p| p.key == "familyNames") {
        Some(p) => match &mut p.value {
            PropertyValue::Localized(v) => match v.first_mut() {
                Some(first) => first.value = family.into(),
                None => v.push(value()),
            },
            PropertyValue::Single(s) => *s = family.into(),
        },
        None => props
            .0
            .insert(0, fontcraft_model::Property { key: "familyNames".into(), value: PropertyValue::Localized(vec![value()]), extra: Extra::new() }),
    }
    props
}

fn write_font(font: &Font, version: u32) -> Result<Dict> {
    let mut e: Vec<(Str, Value)> = vec![(".formatVersion".into(), int(version))];
    let extra = without_deleted_v2_names(&font.info.properties, &font.extra);
    let extra: &Extra = &extra;
    if version < 4 && !font.family_name.is_empty() {
        e.push(("familyName".into(), string(&font.family_name)));
    }
    push_font_info_common(&mut e, font, &font.info.params);
    design::push_defs(&mut e, &font.stems, &font.numbers);
    let properties = without_kept_v2_names(&font.info.properties, extra);
    if version >= 4 && !font.family_name.is_empty() {
        info::push_properties(&mut e, &sync_v4_family_name(&properties, &font.family_name));
    } else {
        info::push_properties(&mut e, &properties);
    }
    info::push_settings(&mut e, &font.info.settings)?;
    e.push(("unitsPerEm".into(), int(font.units_per_em)));
    e.push(("versionMajor".into(), int(font.version_major)));
    e.push(("versionMinor".into(), int(font.version_minor)));
    if !font.metrics.is_empty() {
        e.push(("metrics".into(), list(font.metrics.iter().map(write_metric).collect())));
    }
    if !font.axes.is_empty() {
        e.push(("axes".into(), list(font.axes.iter().map(write_axis).collect())));
    }
    push_features(&mut e, font, "tag");
    push_ui_and_note(&mut e, font);
    if !font.instances.is_empty() {
        e.push(("instances".into(), list(font.instances.iter().map(write_instance).collect::<Result<_>>()?)));
    }
    e.push(("fontMaster".into(), list(font.masters.iter().map(write_master).collect::<Result<_>>()?)));
    e.push(("glyphs".into(), list(font.glyphs.iter().map(|g| write_glyph(g, version).map(Value::Dict)).collect::<Result<_>>()?)));
    for (key, table) in [("kerningLTR", &font.kerning.ltr), ("kerningRTL", &font.kerning.rtl), ("kerningVertical", &font.kerning.vertical)] {
        if !table.is_empty() {
            e.push((key.into(), write_kern_table(table)?));
        }
    }
    if version >= 4 && !font.kerning.context.is_empty() {
        e.push(("kerningContext".into(), write_kern_context(&font.kerning.context)?));
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
                Ok(Value::Dict(Dict { entries: d, layout: Layout::Multiline, ..Default::default() }))
            })
            .collect::<Result<_>>()?;
        e.push(("metricValues".into(), list(values)));
    }
    design::push_values(&mut e, "stemValues", &m.stem_values)?;
    design::push_values(&mut e, "numberValues", &m.number_values)?;
    design::push_guides(&mut e, &m.guides)?;
    push_master_info(&mut e, m);
    Ok(Value::Dict(finish(e, &m.extra)))
}

/// One glyph's dictionary in format version 3 or 4.
fn write_glyph(g: &Glyph, version: u32) -> Result<Dict> {
    let at = format!("glyph `{}`", g.name);
    let mut e = vec![("glyphname".into(), string(&g.name))];
    // A spelling kept from reading, while it still reads as the code points.
    let kept = g.extra.get(Family::Glyphs, "unicode").filter(|v| code_points(v).as_deref() == Some(g.unicodes.as_slice())).cloned();
    if let Some(v) = kept.or_else(|| write_code_points(&g.unicodes)) {
        e.push(("unicode".into(), v));
    }
    if !g.export {
        e.push(("export".into(), int(0)));
    }
    push_kern_groups(&mut e, &g.kern_groups, KERN_GROUP_KEYS);
    glyph::push_glyph_info(&mut e, &g.info, version);
    e.push(("layers".into(), list(g.layers.iter().map(|l| write_layer(l, &at, version)).collect::<Result<_>>()?)));
    Ok(finish(e, &g.extra))
}

fn write_layer(l: &Layer, at: &str, version: u32) -> Result<Value> {
    let at = format!("{at} layer `{}`", l.id);
    let mut e = vec![("layerId".into(), string(&l.id))];
    if l.id != l.master {
        e.push(("associatedMasterId".into(), string(&l.master)));
    }
    e.push(("width".into(), num(l.width, &at)?));
    push_shapes(&mut e, &l.shapes, &l.anchors, &at, version)?;
    layer::push_layer_info(&mut e, &l.info, version, &SHAPES, &at)?;
    Ok(Value::Dict(finish(e, &l.extra)))
}

/// A matrix as position, scale, rotation in degrees and slant in degrees.
pub(crate) type Decomposed = ((f64, f64), (f64, f64), f64, (f64, f64));

/// The real roots of `a·x² + b·x + c` (fontTools' `solveQuadratic`: a linear equation when `a`
/// is close to 0).
fn quadratic_roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    const EPSILON: f64 = 1e-10;
    if a.abs() < EPSILON {
        return if b.abs() < EPSILON { Vec::new() } else { vec![-c / b] };
    }
    let dd = b * b - 4.0 * a * c;
    if dd < 0.0 {
        return Vec::new();
    }
    let r = dd.sqrt();
    vec![(-b + r) / 2.0 / a, (-b - r) / 2.0 / a]
}

/// A matrix as the format's component fields, applied as translate · slant · rotate · scale
/// ([`fontcraft_model::Transform::affine`]); `None` for a singular matrix. Ported from
/// glyphsLib's `decomposeGlyphs3Transform` (Apache-2.0): of the slants that leave a rotation and
/// a scale, the one without a vertical slant is preferred, then the gentlest; a mirror is a
/// negative horizontal scale rather than a turn by 180°.
pub(crate) fn decompose([a, b, c, d, dx, dy]: [f64; 6]) -> Option<Decomposed> {
    const TINY: f64 = 1e-12;
    if ![a, b, c, d, dx, dy].iter().all(|v| v.is_finite()) {
        return None;
    }
    if b == 0.0 && c == 0.0 {
        return Some(((dx, dy), (a, d), 0.0, (0.0, 0.0)));
    }
    // The linear part as rows [[a, c], [b, d]]: M = K · R · S with K = [[1, kx], [ky, 1]],
    // kx = tan(slant x), ky = tan(slant y). That holds exactly when the columns of K⁻¹ · M are
    // orthogonal, which puts (kx, ky) on a conic; its points on the axes, and the one without
    // rotation, are the candidates.
    let linear = a * d + c * b;
    let constant = a * c + b * d;
    let mut candidates: Vec<(f64, f64)> = quadratic_roots(b * d, -linear, constant).into_iter().map(|kx| (kx, 0.0)).collect();
    candidates.extend(quadratic_roots(a * c, -linear, constant).into_iter().map(|ky| (0.0, ky)));
    if a.abs() > TINY && d.abs() > TINY {
        candidates.push((c / d, b / a));
    }
    let close = |x: [f64; 4], y: [f64; 4]| {
        let magnitude = y.iter().fold(1.0f64, |m, v| m.max(v.abs()));
        x.iter().zip(&y).all(|(p, q)| (p - q).abs() <= 1e-9 * magnitude)
    };
    let mut best: Option<((u8, f64), Decomposed)> = None;
    for (kx, ky) in candidates {
        let det = 1.0 - kx * ky;
        if det.abs() < 1e-9 {
            continue;
        }
        let (n00, n01) = ((a - kx * b) / det, (c - kx * d) / det);
        let (n10, n11) = ((b - ky * a) / det, (d - ky * c) / det);
        let sx = n00.hypot(n10);
        if sx < TINY {
            continue;
        }
        let angle = n10.atan2(n00);
        let sy = -angle.sin() * n01 + angle.cos() * n11;
        let fields: Decomposed = ((dx, dy), (sx, sy), angle.to_degrees(), (kx.atan().to_degrees(), ky.atan().to_degrees()));
        let t = fontcraft_model::Transform::Decomposed { pos: fields.0, scale: fields.1, angle: fields.2, slant: fields.3 };
        let m = t.affine().as_coeffs();
        if !close([m[0], m[1], m[2], m[3]], [a, b, c, d]) {
            continue;
        }
        let rank = (u8::from(ky.abs() >= TINY), kx * kx + ky * ky);
        if best.as_ref().is_none_or(|(r, _)| rank.0 < r.0 || (rank.0 == r.0 && rank.1 < r.1)) {
            best = Some((rank, fields));
        }
    }
    let (_, (pos, (mut sx, mut sy), mut angle, slant)) = best?;
    if (angle.abs() - 180.0).abs() < 1e-9 && sy < 0.0 {
        (angle, sx, sy) = (0.0, -sx, -sy);
    }
    let snap = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v };
    let fields = (pos, (sx, sy), snap(angle), (snap(slant.0), snap(slant.1)));
    Some(tidy(fields, [a, b, c, d]))
}

/// The decomposed fields with the noise of the arithmetic taken off: each value rounded to the
/// fewest decimals (4 to 10; the reference app writes up to 4) that still give the same matrix.
fn tidy(fields: Decomposed, m: [f64; 4]) -> Decomposed {
    let (pos, scale, angle, slant) = fields;
    for decimals in 4..=10 {
        let f = 10f64.powi(decimals);
        let r = |v: f64| (v * f).round() / f;
        let t = fontcraft_model::Transform::Decomposed { pos, scale: (r(scale.0), r(scale.1)), angle: r(angle), slant: (r(slant.0), r(slant.1)) };
        let c = t.affine().as_coeffs();
        let magnitude = m.iter().fold(1.0f64, |x, v| x.max(v.abs()));
        if c.iter().zip(&m).all(|(p, q)| (p - q).abs() <= 1e-12 * magnitude) {
            return (pos, (r(scale.0), r(scale.1)), r(angle), (r(slant.0), r(slant.1)));
        }
    }
    fields
}

#[cfg(test)]
mod tests;

/// How many objects of the font have a boolean somewhere in their user data.
fn bool_user_data(font: &Font) -> usize {
    use fontcraft_model::UserData;
    let has = |u: &UserData| {
        let mut found = false;
        Value::Dict(u.0.clone()).visit(&mut |v| found |= matches!(v, Value::Bool(_)));
        found
    };
    let mut n = usize::from(has(&font.info.user_data));
    n += font.masters.iter().filter(|m| has(&m.user_data)).count();
    for g in font.glyphs.iter() {
        n += usize::from(has(&g.info.user_data));
        for l in &g.layers {
            n += usize::from(has(&l.info.user_data));
            for s in &l.shapes {
                match s {
                    fontcraft_model::Shape::Path(p) => n += p.nodes.iter().filter(|x| has(&x.info.user_data)).count(),
                    fontcraft_model::Shape::Component(c) => n += usize::from(has(&c.info.user_data)),
                    _ => {}
                }
            }
            n += l.anchors.iter().filter(|a| has(&a.info.user_data)).count();
        }
    }
    n
}
