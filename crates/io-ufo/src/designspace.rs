//! A designspace and its UFOs: one document with a master per source.
//!
//! The reader reads each referenced UFO ([`crate::ufo`]) and merges them: the masters in source
//! order, the glyphs in the default source's order followed by glyphs only other sources have,
//! font-wide values from the default source (the one at every axis's default, else the first).
//! A source that names a `layer` of a UFO becomes a master whose layers are that UFO layer's
//! glyphs. Values a master's UFO held differently from the default source's (code points, file
//! names, feature code, export lists, groups, font info) are kept per master and written back as
//! long as the model has not changed the default master's.
//!
//! The axes ([`axes`]) and the instances and variable fonts ([`instances`]) are model fields; the
//! axes' maps are the font's `Axis Mappings` parameter, and a default source that glyphsLib would
//! not take as the origin is named by a `Variable Font Origin` parameter.
//!
//! The designspace document itself is kept as read (the XML tree, comments and deprecated
//! elements included) in the font's [`Extra`] under [`Family::Designspace`] (`document`), and the
//! writer changes it only where the model changed, reading the kept element the way the reader
//! does and comparing: axes, maps, source names and locations, instances and variable fonts, and
//! sources and instances added or removed. A document that did not come from a designspace gets
//! one in the layout fontTools' `designspaceLib` writes (format 5.0), with what glyphsLib writes
//! for a `.glyphs` source.
//!
//! Locations are design coordinates in the model, parallel to [`Font::axes`]. A dimension given in
//! user coordinates is mapped through the axis's `map` elements when read and written back
//! unchanged while the design value stays the same (`roundtrip.designspaceCoordinates`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use fontcraft_model::axes::{AXIS_MAPPINGS, ORIGIN};
use fontcraft_model::{
    Extra, Family, FileStore, Font, FontInfo, Glyph, GlyphTable, Instance, KernTable, Kerning, Master, MetricDef, MetricValue, Param, Source,
    SourceFormat,
};
use fontcraft_plist::xml::{self, Element, Node as XNode, Text};
use fontcraft_plist::{Array, Value, xmlplist};

use crate::fontinfo;
use crate::ufo::{self, FEATURES_KEY, ReadUfo, Target};
use crate::{Files, Result, UfoError, bad};

mod axes;
mod instances;

const DOC: &str = "the designspace";
/// Most sources, instances or axes one designspace may have.
pub const MAX_ITEMS: usize = 10_000;

/// A designspace written out: the document and each UFO's files, by the `filename` the document
/// gives it.
pub struct Written {
    pub document: String,
    pub ufos: BTreeMap<String, Files>,
}

struct AxisInfo {
    name: String,
    tag: String,
    /// User-space default, and the user → design map.
    user_default: f64,
    map: Vec<(f64, f64)>,
}

impl AxisInfo {
    fn to_design(&self, user: f64) -> f64 {
        piecewise(&self.map, user)
    }

    fn design_default(&self) -> f64 {
        self.to_design(self.user_default)
    }
}

/// Piecewise-linear interpolation through `(input, output)` points, sorted by input; outside them,
/// the nearest end's output (as designspaceLib maps).
fn piecewise(map: &[(f64, f64)], v: f64) -> f64 {
    let mut pts: Vec<(f64, f64)> = map.to_vec();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return v };
    if v <= first.0 {
        return first.1;
    }
    if v >= last.0 {
        return last.1;
    }
    for w in pts.windows(2) {
        if let [(x0, y0), (x1, y1)] = w
            && v >= *x0
            && v <= *x1
        {
            return if x1 == x0 { *y0 } else { y0 + (v - x0) * (y1 - y0) / (x1 - x0) };
        }
    }
    v
}

fn number(el: &Element, attr: &str) -> Result<Option<f64>> {
    match el.attr(attr) {
        None => Ok(None),
        Some(v) => match v.trim().parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(Some(f)),
            _ => Err(bad(DOC, format!("`<{} {attr}=\"{v}\">` is not a number", el.name()))),
        },
    }
}

fn read_axes(root: &Element) -> Result<Vec<AxisInfo>> {
    let Some(axes) = root.child("axes") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for a in axes.elements().filter(|e| e.name() == "axis") {
        if out.len() >= MAX_ITEMS {
            return Err(bad(DOC, format!("more than {MAX_ITEMS} axes")));
        }
        let name = a.attr("name").ok_or_else(|| bad(DOC, "an axis has no name"))?.to_string();
        let user_default = match number(a, "default")? {
            Some(d) => d,
            // A discrete axis without a default takes its first value.
            None => a.attr("values").and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok()).unwrap_or(0.0),
        };
        let mut map = Vec::new();
        for m in a.elements().filter(|e| e.name() == "map") {
            if let (Some(i), Some(o)) = (number(m, "input")?, number(m, "output")?) {
                map.push((i, o));
            }
        }
        out.push(AxisInfo { tag: a.attr("tag").unwrap_or("").to_string(), name, user_default, map });
    }
    Ok(out)
}

/// A location element's design coordinates, parallel to the axes; dimensions not given take the
/// axis default.
fn read_location(parent: &Element, axes: &[AxisInfo], labels: &HashMap<String, Vec<f64>>) -> Result<Vec<f64>> {
    if let Some(label) = parent.attr("location")
        && let Some(loc) = labels.get(label)
    {
        return Ok(loc.clone());
    }
    let mut loc: Vec<f64> = axes.iter().map(AxisInfo::design_default).collect();
    let Some(location) = parent.child("location") else { return Ok(loc) };
    for d in location.elements().filter(|e| e.name() == "dimension") {
        let Some(name) = d.attr("name") else { continue };
        let Some(i) = axes.iter().position(|a| a.name == name) else { continue };
        let value = match (number(d, "xvalue")?, number(d, "uservalue")?) {
            (Some(x), _) => x,
            (None, Some(u)) => axes.get(i).map_or(u, |a| a.to_design(u)),
            (None, None) => continue,
        };
        if let Some(slot) = loc.get_mut(i) {
            *slot = value;
        }
    }
    Ok(loc)
}

/// The top-level location labels (format 5), by name.
fn read_labels(root: &Element, axes: &[AxisInfo]) -> Result<HashMap<String, Vec<f64>>> {
    let mut out = HashMap::new();
    if let Some(labels) = root.child("labels") {
        for l in labels.elements().filter(|e| e.name() == "label") {
            if let Some(name) = l.attr("name") {
                let loc = read_location(l, axes, &HashMap::new())?;
                out.insert(name.to_string(), loc);
            }
        }
    }
    Ok(out)
}

struct SourceInfo {
    id: String,
    filename: String,
    layer: Option<String>,
    style_name: Option<String>,
    location: Vec<f64>,
}

fn source_elements(root: &Element) -> Vec<&Element> {
    root.child("sources").map(|s| s.elements().filter(|e| e.name() == "source").collect()).unwrap_or_default()
}

/// The master id of each source: its `name`, or `source<n>` where it has none or repeats one.
fn source_ids(sources: &[&Element]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let id = match s.attr("name") {
                Some(n) if !n.is_empty() && !seen.iter().any(|x| x == n) => n.to_string(),
                _ => format!("source{i}"),
            };
            seen.push(id.clone());
            id
        })
        .collect()
}

/// The `filename` of every source, as written, without repeats, in order: the UFOs to read.
pub fn ufo_paths(text: &str) -> Result<Vec<String>> {
    let doc = xml::parse(text).map_err(|e| UfoError::Syntax { file: DOC.into(), error: e })?;
    let mut out: Vec<String> = Vec::new();
    for s in source_elements(&doc.root) {
        if let Some(f) = s.attr("filename")
            && !out.iter().any(|x| x == f)
        {
            out.push(f.to_string());
        }
    }
    Ok(out)
}

fn ds(x: &mut Extra, key: &str, v: Value) {
    x.push(Family::Designspace, key, v);
}

fn ds_str<'a>(x: &'a Extra, key: &str) -> Option<&'a str> {
    x.get(Family::Designspace, key).and_then(Value::as_str)
}

/// Reads a designspace and the UFOs it refers to, given by their `filename` as written.
pub fn read(text: &str, ufos: &BTreeMap<String, Files>) -> Result<Font> {
    let doc = xml::parse(text).map_err(|e| UfoError::Syntax { file: DOC.into(), error: e })?;
    let root = &doc.root;
    if root.name() != "designspace" {
        return Err(bad(DOC, "the root element is not `<designspace>`"));
    }
    let axes = read_axes(root)?;
    let labels = read_labels(root, &axes)?;
    let elements = source_elements(root);
    if elements.is_empty() {
        return Err(bad(DOC, "it has no sources"));
    }
    if elements.len() > MAX_ITEMS {
        return Err(bad(DOC, format!("more than {MAX_ITEMS} sources")));
    }
    let ids = source_ids(&elements);
    let mut sources = Vec::new();
    for (el, id) in elements.iter().zip(ids) {
        let filename = el.attr("filename").ok_or_else(|| bad(DOC, format!("the source `{id}` has no filename")))?.to_string();
        sources.push(SourceInfo {
            id,
            filename,
            layer: el.attr("layer").map(str::to_string),
            style_name: el.attr("stylename").map(str::to_string),
            location: read_location(el, &axes, &labels)?,
        });
    }
    let defaults: Vec<f64> = axes.iter().map(AxisInfo::design_default).collect();
    let default_index = sources.iter().position(|s| s.layer.is_none() && s.location == defaults).unwrap_or(0);

    // Read each source's UFO as its master; a UFO two sources share is read for each.
    let mut read: Vec<Option<ReadUfo>> = Vec::new();
    let mut main_of: HashMap<String, usize> = HashMap::new();
    for (i, s) in sources.iter().enumerate() {
        if s.layer.is_some() {
            read.push(None);
            continue;
        }
        let files = ufos.get(&s.filename).ok_or_else(|| bad(&s.filename, "the UFO is missing"))?;
        let r = ufo::read_master(files, &s.id).map_err(|e| prefix_error(&s.filename, e))?;
        main_of.entry(s.filename.clone()).or_insert(i);
        read.push(Some(r));
    }
    for s in &sources {
        if s.layer.is_some() && !main_of.contains_key(&s.filename) {
            return Err(UfoError::Unsupported(format!("`{}` is used only through layers", s.filename)));
        }
    }
    let default_index =
        if read.get(default_index).is_some_and(Option::is_some) { default_index } else { main_of.values().copied().min().unwrap_or(0) };
    let Some(Some(default)) = read.get(default_index) else { return Err(bad(DOC, "no source can be read")) };

    // Metric kinds over every master; stem definitions from the default source, else the first
    // with stems.
    let kinds = ufo::metric_kinds(read.iter().flatten());
    let stems = ufo::stem_counts(std::iter::once(default).chain(read.iter().flatten()));
    let default_features = default.features.clone();
    let groups = default.groups.clone();
    let (family_name, units_per_em, version_major, version_minor, note, ufo_version) = (
        default.family_name.clone(),
        default.units_per_em,
        default.version_major,
        default.version_minor,
        default.note.clone(),
        default.format_version,
    );
    let default_id = sources.get(default_index).map(|s| s.id.clone()).unwrap_or_default();
    // The default source's font information is the font's; another master keeps what differs.
    let font_info = default.info.clone();

    // Masters, and the glyphs of each.
    let mut masters: Vec<Master> = Vec::new();
    let mut per_master_glyphs: Vec<(usize, Vec<Glyph>)> = Vec::new();
    let mut kerning = Vec::new();
    let mut files = BTreeMap::new();
    let mut main_values: HashMap<String, Vec<MetricValue>> = HashMap::new();
    for (i, slot) in read.iter_mut().enumerate() {
        let Some(r) = slot.take() else { continue };
        let Some(s) = sources.get(i) else { continue };
        let mut master = r.master;
        if i != default_index {
            let own = fontinfo::split(&font_info, r.info);
            master.properties = own.properties;
            master.params = own.params;
        }
        master.location = s.location.clone();
        ufo::place_design(&mut master, &r.metrics, r.design, r.design_raw, &kinds, stems);
        main_values.insert(s.filename.clone(), master.metric_values.clone());
        ds(&mut master.extra, "filename", Value::String(s.filename.as_str().into()));
        if i != default_index && r.features != default_features {
            let pair = vec![opt_text(default_features.as_deref()), opt_text(r.features.as_deref())];
            master.extra.push(Family::Ufo, FEATURES_KEY, Value::Array(Array { items: pair, ..Default::default() }));
        }
        if !r.kerning.firsts.is_empty() {
            kerning.push(r.kerning);
        }
        for (path, bytes) in r.files {
            files.entry(format!("{}/{path}", s.filename)).or_insert(bytes);
        }
        masters.push(master);
        per_master_glyphs.push((i, r.glyphs));
    }
    // Layer sources: masters made of a UFO layer's glyphs.
    let mut layer_masters: Vec<(usize, Master)> = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        let Some(layer) = &s.layer else { continue };
        let main_id = main_of.get(&s.filename).and_then(|&m| sources.get(m)).map(|m| m.id.clone()).unwrap_or_default();
        let mut master = Master {
            id: s.id.clone(),
            name: s.style_name.clone().unwrap_or_else(|| layer.clone()),
            location: s.location.clone(),
            metric_values: main_values.get(&s.filename).cloned().unwrap_or_default(),
            ..Default::default()
        };
        ds(&mut master.extra, "filename", Value::String(s.filename.as_str().into()));
        ds(&mut master.extra, "layer", Value::String(layer.as_str().into()));
        for (_, glyphs) in &mut per_master_glyphs {
            for g in glyphs.iter_mut() {
                for l in g.layers.iter_mut().filter(|l| l.master == main_id && !l.is_master_layer() && l.info.name.as_deref() == Some(layer.as_str()))
                {
                    l.id = s.id.clone();
                    l.master = s.id.clone();
                    l.info.name = None;
                }
            }
        }
        layer_masters.push((i, master));
    }
    // Masters in source order.
    let mut ordered: Vec<(usize, Master)> = per_master_glyphs.iter().map(|(i, _)| *i).zip(masters).collect();
    ordered.extend(layer_masters);
    ordered.sort_by_key(|(i, _)| *i);
    let masters: Vec<Master> = ordered.into_iter().map(|(_, m)| m).collect();

    // Glyphs: the default source's, then the others' new ones.
    per_master_glyphs.sort_by_key(|(i, _)| usize::from(*i != default_index));
    let mut glyphs: Vec<Glyph> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (_, list) in per_master_glyphs {
        for g in list {
            match index.get(&g.name) {
                None => {
                    index.insert(g.name.clone(), glyphs.len());
                    glyphs.push(g);
                }
                Some(&at) => {
                    let Some(merged) = glyphs.get_mut(at) else { continue };
                    merge_glyph(merged, g);
                }
            }
        }
    }
    let features = default_features.as_deref().map(crate::features::read).unwrap_or_default();
    let mut extra = Extra::new();
    ds(&mut extra, "document", Value::String(text.into()));
    ds(&mut extra, "defaultSource", Value::String(default_id.as_str().into()));
    let family = family_name.unwrap_or_default();
    let model_axes = model_axes(root)?;
    // The axes' maps, as the `Axis Mappings` parameter.
    let mut params = font_info.params;
    let sources: Vec<&[f64]> = masters.iter().map(|m| m.location.as_slice()).collect();
    if let Some(v) = doc_mappings(root, &model_axes, &sources)? {
        params.0.push(Param::new(AXIS_MAPPINGS, v));
    }
    let mut font = Font {
        source: Source { format: SourceFormat::Designspace, ufo_version: Some(ufo_version), ..Default::default() },
        family_name: family.clone(),
        units_per_em: ufo::units_per_em(units_per_em)?,
        version_major: version_major.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        version_minor: version_minor.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        axes: Arc::new(model_axes),
        metrics: Arc::new(kinds.iter().map(|k| MetricDef::new(k)).collect()),
        stems: Arc::new(ufo::stem_defs(stems)),
        numbers: Arc::default(),
        masters: Arc::new(masters),
        instances: Arc::new(read_instances(root, &instances::Ctx { family: &family, axes: &axes, labels: &labels, unit: &indent_unit(root) })?),
        glyphs: GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect()),
        info: Arc::new(FontInfo { properties: font_info.properties, params, date: font_info.date, ..Default::default() }),
        kerning: Arc::new(Kerning { ltr: KernTable(kerning), ..Default::default() }),
        features: Arc::new(features),
        groups: Arc::new(groups),
        display_strings: Arc::default(),
        note: note.unwrap_or_default(),
        files: Arc::new(FileStore(files)),
        extra: Arc::new(extra),
    };
    // A default source glyphsLib would not find by the masters' names is named.
    if font.masters.len() > 1 && fontcraft_model::axes::origin_without_parameter(&font).is_some_and(|m| m.id != default_id) {
        Arc::make_mut(&mut font.info).params.0.push(Param::new(ORIGIN, Value::String(default_id.as_str().into())));
    }
    font.check()?;
    Ok(font)
}

/// The names a designspace instance element states for an instance, derived where the model
/// leaves them to the reference's rules.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceNames {
    pub family: String,
    pub postscript: Option<String>,
    /// The instance UFO's file name.
    pub filename: String,
    pub style_map_family: String,
    pub style_map_style: &'static str,
}

/// The names of a static instance as a designspace states them.
pub fn instance_names(font: &Font, inst: &Instance) -> InstanceNames {
    let family = instances::family_of(inst, &font.family_name);
    let (style_map_family, style_map_style) = instances::stylemap(inst, &family);
    InstanceNames {
        postscript: instances::ps_name_of(inst),
        filename: instances::filename_of(inst, &family),
        style_map_family,
        style_map_style,
        family,
    }
}

/// What saving `font` as a designspace and its UFOs leaves out of its font information, axes and
/// instances, one line each.
pub fn info_losses(font: &Font) -> Vec<String> {
    let maps_back = mappings_come_back(font);
    let origin_back = font.info.params.get(ORIGIN).and_then(Value::as_str).is_some_and(|id| {
        let origin = fontcraft_model::axes::origin(font).map(|m| m.id.as_str());
        origin == Some(id) && fontcraft_model::axes::origin_without_parameter(font).map(|m| m.id.as_str()) != Some(id)
    });
    let carried = |line: &str| line == format!("parameter `{AXIS_MAPPINGS}`") && maps_back || line == format!("parameter `{ORIGIN}`") && origin_back;
    let mut out: Vec<String> = ufo::font_losses(font).into_iter().filter(|l| !carried(l)).collect();
    for i in font.instances.iter() {
        let fields = [("id", i.id.is_some()), ("type", i.kind.as_deref().is_some_and(|k| k != "variable")), ("visibility", !i.visible)];
        let set: Vec<&str> = fields.iter().filter(|(_, on)| *on).map(|(what, _)| *what).collect();
        if !set.is_empty() {
            out.push(format!("instance `{}`: {}", i.name, set.join(", ")));
        }
        for p in i.params.0.iter().filter(|p| p.disabled) {
            out.push(format!("instance `{}`: parameter `{}`, switched off", i.name, p.name));
        }
        // Parameters an attribute holds come back as what the reader makes of the attribute.
        for (param, back) in [("familyName", "the `familyNames` property"), ("postscriptFontName", "the property of that name")] {
            if i.params.get(param).is_some() && i.kind.as_deref() != Some("variable") {
                out.push(format!("instance `{}`: parameter `{param}`, as {back}", i.name));
            }
        }
        let other_languages = |p: &&fontcraft_model::Property| match &p.value {
            fontcraft_model::PropertyValue::Single(_) => false,
            fontcraft_model::PropertyValue::Localized(v) => v.iter().any(|l| l.language != fontcraft_model::DEFAULT_LANGUAGE),
        };
        for p in i.properties.0.iter().filter(other_languages) {
            out.push(format!("instance `{}`: property `{}` in languages other than the default", i.name, p.key));
        }
    }
    out
}

/// Whether the `Axis Mappings` parameter comes back from the designspace written for `font`: the
/// maps and ranges the writer gives the axes make the same parameter when read.
fn mappings_come_back(font: &Font) -> bool {
    let Some(model) = axes::model_mappings(font) else { return true };
    let written: Vec<(fontcraft_model::Axis, Vec<(f64, f64)>)> = (0..font.axes.len())
        .filter_map(|i| {
            let a = font.axes.get(i)?;
            let (map, range) = axes::map_and_range(font, i);
            Some((fontcraft_model::Axis { range: Some(range), ..a.clone() }, map))
        })
        .collect();
    let locations: Vec<Vec<f64>> =
        font.masters.iter().map(|m| (0..font.axes.len()).map(|i| fontcraft_model::axes::design_value(&m.location, i)).collect()).collect();
    let extents = axes::extents(locations.iter().map(Vec::as_slice), font.axes.len());
    let sources: Vec<axes::MapSource<'_>> =
        written.iter().zip(extents).map(|((axis, map), extent)| axes::MapSource { axis, map: map.clone(), extent }).collect();
    let Some(back) = axes::mappings_param(&sources) else { return false };
    let pairs = |v: &Value, tag: &str| -> Option<Vec<(f64, f64)>> {
        let mut f = Font::new("x");
        Arc::make_mut(&mut f.info).params.0.push(Param::new(AXIS_MAPPINGS, v.clone()));
        fontcraft_model::axes::mapping_parameter(&f, tag)
    };
    let tags = |v: &Value| v.as_dict().map(|d| d.entries.iter().map(|(k, _)| k.as_str().to_string()).collect::<Vec<_>>()).unwrap_or_default();
    let (mut a, mut b) = (tags(model), tags(&back));
    a.sort();
    b.sort();
    a == b && a.iter().all(|t| pairs(model, t) == pairs(&back, t))
}

/// The model's axes, from the `<axis>` elements.
fn model_axes(root: &Element) -> Result<Vec<fontcraft_model::Axis>> {
    axis_elements(root).into_iter().map(axes::read_axis).collect()
}

fn axis_elements(root: &Element) -> Vec<&Element> {
    root.child("axes").map(|a| a.elements().filter(|e| e.name() == "axis").take(MAX_ITEMS).collect()).unwrap_or_default()
}

/// The `Axis Mappings` parameter the document's maps make, given the sources' design locations.
fn doc_mappings(root: &Element, model_axes: &[fontcraft_model::Axis], sources: &[&[f64]]) -> Result<Option<Value>> {
    let extents = axes::extents(sources.iter().copied(), model_axes.len());
    let mut list = Vec::new();
    for ((el, axis), extent) in axis_elements(root).into_iter().zip(model_axes).zip(extents) {
        list.push(axes::MapSource { axis, map: axes::read_map(el)?, extent });
    }
    Ok(axes::mappings_param(&list))
}

fn opt_text(t: Option<&str>) -> Value {
    match t {
        Some(t) => Value::String(t.into()),
        // No file.
        None => Value::Bool(false),
    }
}

fn prefix_error(path: &str, e: UfoError) -> UfoError {
    match e {
        UfoError::Syntax { file, error } => UfoError::Syntax { file: format!("{path}/{file}"), error },
        UfoError::Bad { at, message } => UfoError::Bad { at: format!("{path}/{at}"), message },
        other => other,
    }
}

/// Adds another master's copy of a glyph to the merged glyph, keeping what differs per master.
fn merge_glyph(merged: &mut Glyph, g: Glyph) {
    let master_id = g.layers.iter().find(|l| l.id == l.master).map(|l| l.master.clone());
    let mut layers = g.layers;
    if let Some(m) = master_id
        && let Some(l) = layers.iter_mut().find(|l| l.id == m && l.master == m)
    {
        if g.unicodes != merged.unicodes {
            let list = |u: &[u32]| {
                Value::Array(Array {
                    items: u.iter().map(|c| Value::Number(fontcraft_plist::Number::int(i64::from(*c)))).collect(),
                    ..Default::default()
                })
            };
            l.extra.push(Family::Ufo, "unicodes", list(&g.unicodes));
            l.extra.push(Family::Ufo, "unicodesRead", list(&merged.unicodes));
        }
        // The note and colour of this master's file, where they differ from the glyph's.
        if g.info.note != merged.info.note {
            l.extra.push(Family::Ufo, "note", Value::String(g.info.note.as_str().into()));
            l.extra.push(Family::Ufo, "noteRead", Value::String(merged.info.note.as_str().into()));
        }
        if g.info.color != merged.info.color {
            let text = merged.info.color.as_ref().and_then(crate::glif::mark_color_text).unwrap_or_default();
            l.extra.push(Family::Ufo, "markColorRead", Value::String(text.into()));
        }
        if g.file_name != merged.file_name && l.extra.get(Family::Ufo, "fileName").is_none() {
            let own = g.file_name.clone().or_else(|| crate::names::glyph_file_name(&g.name, &std::collections::HashSet::new()));
            if let Some(own) = own {
                l.extra.push(Family::Ufo, "fileName", Value::String(own.into()));
            }
        }
    }
    merged.layers.extend(layers);
}

fn static_elements(root: &Element) -> Vec<&Element> {
    root.child("instances").map(|i| i.elements().filter(|e| e.name() == "instance").collect()).unwrap_or_default()
}

fn variable_elements(root: &Element) -> Vec<&Element> {
    root.child("variable-fonts").map(|i| i.elements().filter(|e| e.name() == "variable-font").collect()).unwrap_or_default()
}

/// The instances, then the variable fonts as variable instances.
fn read_instances(root: &Element, ctx: &instances::Ctx<'_>) -> Result<Vec<Instance>> {
    let (statics, variables) = (static_elements(root), variable_elements(root));
    if statics.len() + variables.len() > MAX_ITEMS {
        return Err(bad(DOC, format!("more than {MAX_ITEMS} instances")));
    }
    let mut out = Vec::new();
    for el in statics {
        out.push(instances::read_static(el, ctx)?);
    }
    for el in variables {
        out.push(instances::read_variable(el, ctx)?);
    }
    Ok(out)
}

fn is_variable(i: &Instance) -> bool {
    i.kind.as_deref() == Some("variable")
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// designspaceLib's number format: an integer when whole, otherwise up to six decimals without
/// trailing zeros.
pub fn int_or_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        let i = v as i64;
        i.to_string()
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Writes a document as a designspace and its UFOs.
pub fn write(font: &Font) -> Result<Written> {
    let default_id = font.extra.get(Family::Designspace, "defaultSource").and_then(Value::as_str);
    let default = default_id
        .and_then(|id| font.masters.iter().find(|m| m.id == id))
        .or_else(|| fontcraft_model::axes::origin(font))
        .ok_or(fontcraft_model::ModelError::NoMaster)?;
    // Each master's UFO, and which masters are layers of which UFO.
    let mut filenames: Vec<String> = Vec::new();
    let mut file_of: HashMap<&str, String> = HashMap::new();
    for m in font.masters.iter() {
        let f = match ds_str(&m.extra, "filename") {
            Some(f) => f.to_string(),
            None => ufo_file_name(font, m, &filenames),
        };
        if !filenames.contains(&f) {
            filenames.push(f.clone());
        }
        file_of.insert(&m.id, f);
    }
    let mut ufos = BTreeMap::new();
    for f in &filenames {
        let on_file: Vec<&Master> = font.masters.iter().filter(|m| file_of.get(m.id.as_str()) == Some(f)).collect();
        let mains: Vec<&Master> = on_file.iter().filter(|m| ds_str(&m.extra, "layer").is_none()).copied().collect();
        let Some(main) = mains.first().copied() else {
            return Err(UfoError::Unsupported(format!("`{f}` has only layer sources")));
        };
        let layer_masters: Vec<(&Master, String)> = on_file.iter().filter_map(|m| ds_str(&m.extra, "layer").map(|l| (*m, l.to_string()))).collect();
        let target = Target { master: main, default, layer_masters: layer_masters.clone(), prefix: format!("{f}/") };
        let files = ufo::write_master(font, &target).map_err(|e| prefix_error(f, e))?;
        // Masters that share a UFO must still agree on what is in it.
        for other in mains.iter().skip(1) {
            let target = Target { master: other, default, layer_masters: layer_masters.clone(), prefix: format!("{f}/") };
            if ufo::write_master(font, &target).map_err(|e| prefix_error(f, e))? != files {
                return Err(UfoError::Unsupported(format!("the masters `{}` and `{}` share `{f}` but now differ", main.id, other.id)));
            }
        }
        ufos.insert(f.clone(), files);
    }
    let document = match font.extra.get(Family::Designspace, "document").and_then(Value::as_str) {
        Some(kept) => patch_document(font, kept, &file_of)?,
        None => new_document(font, &file_of),
    };
    Ok(Written { document, ufos })
}

/// A file name for a master's UFO: `Family-Style.ufo` with only letters, digits, `-` and `_`,
/// made unique among `taken`.
pub fn ufo_file_name(font: &Font, m: &Master, taken: &[String]) -> String {
    let clean = |s: &str| s.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect::<String>();
    let family = if font.family_name.is_empty() { "Font".to_string() } else { clean(&font.family_name) };
    let style = if m.name.is_empty() { clean(&m.id) } else { clean(&m.name) };
    let base = format!("{family}-{style}");
    let mut name = format!("{base}.ufo");
    let mut n = 2;
    while taken.contains(&name) {
        name = format!("{base}-{n}.ufo");
        n += 1;
    }
    name
}

/// The design value a dimension element holds for an axis.
fn dimension_value(d: &Element, axis: Option<&AxisInfo>) -> Option<f64> {
    let x = d.attr("xvalue").and_then(|v| v.trim().parse::<f64>().ok());
    let u = d.attr("uservalue").and_then(|v| v.trim().parse::<f64>().ok());
    x.or_else(|| u.map(|u| axis.map_or(u, |a| a.to_design(u))))
}

/// Updates a location element under `parent` to `loc` where the model differs.
fn patch_location(parent: &mut Element, axes: &[AxisInfo], names: &[String], loc: &[f64], unit: &str, level: usize) {
    let current = read_location(parent, axes, &HashMap::new()).unwrap_or_default();
    if current.len() == loc.len() && current.iter().zip(loc).all(|(a, b)| a == b) {
        return;
    }
    parent.remove_attr("location");
    if parent.child("location").is_none() {
        append_child(parent, Element::new("location"), unit, level);
    }
    let Some(location) = parent.elements_mut().find(|e| e.name() == "location") else { return };
    for (i, (name, v)) in names.iter().zip(loc).enumerate() {
        let axis = axes.get(i);
        let existing = location.elements_mut().find(|d| d.name() == "dimension" && d.attr("name") == Some(name.as_str()));
        match existing {
            Some(d) => {
                if dimension_value(d, axis) != Some(*v) {
                    d.remove_attr("uservalue");
                    d.remove_attr("yvalue");
                    d.set_attr("xvalue", int_or_float(*v));
                }
            }
            None => {
                let d = Element::new("dimension").with_attr("name", name).with_attr("xvalue", int_or_float(*v));
                append_child(location, d, unit, level + 1);
            }
        }
    }
}

/// Appends `child` to `parent`, indented like a pretty printer would at `level` (the parent's).
fn append_child(parent: &mut Element, child: Element, unit: &str, level: usize) {
    let had_children = parent.elements().next().is_some();
    // Drop the whitespace before the end tag, add the child on its own line, put it back.
    if let Some(XNode::Text(t)) = parent.children.last()
        && t.is_whitespace()
    {
        parent.children.pop();
    }
    let _ = had_children;
    parent.children.push(XNode::Text(Text::new(format!("\n{}", unit.repeat(level + 1)))));
    let mut text = String::new();
    child.write_pretty(&mut text, unit, level + 1);
    let child = xml::parse_element(&text).unwrap_or(child);
    parent.children.push(XNode::Element(child));
    parent.children.push(XNode::Text(Text::new(format!("\n{}", unit.repeat(level)))));
}

/// Removes the `i`-th child element and the whitespace before it.
fn remove_child(parent: &mut Element, i: usize) {
    let mut seen = 0;
    let Some(pos) = parent.children.iter().position(|n| {
        let hit = matches!(n, XNode::Element(_)) && seen == i;
        if matches!(n, XNode::Element(_)) {
            seen += 1;
        }
        hit
    }) else {
        return;
    };
    parent.children.remove(pos);
    if pos > 0 && matches!(parent.children.get(pos - 1), Some(XNode::Text(t)) if t.is_whitespace()) {
        parent.children.remove(pos - 1);
    }
}

fn indent_unit(root: &Element) -> String {
    for n in &root.children {
        if let XNode::Text(t) = n
            && let Some(line) = t.as_str().rsplit('\n').next()
            && t.as_str().contains('\n')
            && !line.is_empty()
        {
            return line.to_string();
        }
    }
    "  ".into()
}

/// Inserts `child` before the `at`-th child element of `parent` (appends it when there is no
/// such element), indented like a pretty printer would at `level` (the parent's).
fn insert_child(parent: &mut Element, at: usize, child: Element, unit: &str, level: usize) {
    let pos = parent.children.iter().enumerate().filter(|(_, n)| matches!(n, XNode::Element(_))).nth(at).map(|(i, _)| i);
    let Some(pos) = pos else { return append_child(parent, child, unit, level) };
    let mut text = String::new();
    child.write_pretty(&mut text, unit, level + 1);
    let child = xml::parse_element(&text).unwrap_or(child);
    parent.children.insert(pos, XNode::Text(Text::new(format!("\n{}", unit.repeat(level + 1)))));
    parent.children.insert(pos, XNode::Element(child));
}

/// Replaces the child elements called `name` with `new`: at the place of the first of them, else
/// after the last child element called one of `after`, else first.
fn replace_children(parent: &mut Element, name: &str, new: Vec<Element>, after: &[&str], unit: &str, level: usize) {
    let names: Vec<String> = parent.elements().map(|e| e.name().to_string()).collect();
    let first = names.iter().position(|n| n == name);
    for i in (0..names.len()).rev().filter(|i| names.get(*i).is_some_and(|n| n == name)) {
        remove_child(parent, i);
    }
    let at = first.unwrap_or_else(|| names.iter().rposition(|n| after.contains(&n.as_str())).map_or(0, |i| i + 1));
    for (k, c) in new.into_iter().enumerate() {
        insert_child(parent, at + k, c, unit, level);
    }
    // An element left without child elements closes itself, as designspaceLib writes it.
    if parent.elements().next().is_none() && !parent.has_text() {
        parent.children.clear();
        parent.set_self_closing(true);
    }
}

/// A source's `stylename` and `familyname` follow the master's and the font's names while they
/// agreed with the source's UFO when read.
fn patch_source_names(el: &mut Element, font: &Font, m: &Master) {
    let layer = ds_str(&m.extra, "layer").is_some();
    if let Some(s) = el.attr("stylename")
        && s != m.name
        && (layer || ufo::kept_info_str(m, "styleName").as_deref() == Some(s))
    {
        el.set_attr("stylename", &m.name);
    }
    if let Some(f) = el.attr("familyname")
        && f != font.family_name
        && !layer
        && ufo::kept_info_str(m, "familyName").as_deref() == Some(f)
    {
        el.set_attr("familyname", &font.family_name);
    }
}

fn patch_document(font: &Font, kept: &str, file_of: &HashMap<&str, String>) -> Result<String> {
    let mut doc = xml::parse(kept).map_err(|e| UfoError::Syntax { file: DOC.into(), error: e })?;
    let unit = indent_unit(&doc.root);
    let axes = read_axes(&doc.root)?;
    if axes.len() != font.axes.len() {
        return Err(UfoError::Unsupported("adding or removing axes of a designspace".into()));
    }
    let names: Vec<String> = font.axes.iter().map(|a| a.name.clone()).collect();
    // What the kept document says, read as the reader reads it.
    let kept_axes = model_axes(&doc.root)?;
    let labels = read_labels(&doc.root, &axes)?;
    let kept_locations: Vec<Vec<f64>> = source_elements(&doc.root).iter().map(|s| read_location(s, &axes, &labels)).collect::<Result<_>>()?;
    let refs: Vec<&[f64]> = kept_locations.iter().map(Vec::as_slice).collect();
    let maps_changed = doc_mappings(&doc.root, &kept_axes, &refs)?.as_ref() != axes::model_mappings(font);
    // Axes.
    if let Some(axes_el) = doc.root.elements_mut().find(|e| e.name() == "axes") {
        let axis_els = axes_el.elements_mut().filter(|e| e.name() == "axis");
        for (i, ((el, axis), was)) in axis_els.zip(font.axes.iter()).zip(&kept_axes).enumerate() {
            el.set_attr("name", &axis.name);
            el.set_attr("tag", &axis.tag);
            let hidden = el.attr("hidden").is_some_and(|h| h == "1" || h == "true");
            if hidden != axis.hidden {
                if axis.hidden {
                    el.set_attr("hidden", "1");
                } else {
                    el.remove_attr("hidden");
                }
            }
            if was.range != axis.range || was.values != axis.values {
                axes::set_range(el, axis, fontcraft_model::axes::range(font, i));
            }
            if was.label_names != axis.label_names {
                replace_children(el, "labelname", axes::label_name_elements(axis), &[], &unit, 2);
            }
            if (&was.labels, was.ordering) != (&axis.labels, axis.ordering) {
                replace_children(el, "labels", axes::labels_element(axis).into_iter().collect(), &["labelname", "map"], &unit, 2);
            }
            if maps_changed {
                let desired = fontcraft_model::axes::mapping_parameter(font, &axis.tag).filter(|m| m.iter().any(|(u, d)| u != d)).unwrap_or_default();
                if axes::read_map(el)? != desired {
                    replace_children(el, "map", axes::map_elements(&desired), &["labelname"], &unit, 2);
                }
            }
        }
    }
    let axes = read_axes(&doc.root)?;
    // Sources: by master id.
    let ids = source_ids(&source_elements(&doc.root));
    if let Some(sources) = doc.root.elements_mut().find(|e| e.name() == "sources") {
        let gone: Vec<usize> = ids.iter().enumerate().filter(|(_, id)| !font.masters.iter().any(|m| &m.id == *id)).map(|(i, _)| i).collect();
        for &i in gone.iter().rev() {
            remove_child(sources, i);
        }
        let kept_ids: Vec<&String> = ids.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, id)| id).collect();
        for (el, id) in sources.elements_mut().filter(|e| e.name() == "source").zip(kept_ids.iter()) {
            if let Some(m) = font.masters.iter().find(|m| &m.id == *id) {
                patch_location(el, &axes, &names, &m.location, &unit, 2);
                patch_source_names(el, font, m);
            }
        }
        for m in font.masters.iter().filter(|m| !ids.contains(&m.id)) {
            append_child(sources, source_element(font, m, file_of, &names), &unit, 1);
        }
    } else if !font.masters.is_empty() {
        let mut sources = Element::new("sources");
        for m in font.masters.iter() {
            sources.children.push(XNode::Element(source_element(font, m, file_of, &names)));
        }
        append_child(&mut doc.root, sources, &unit, 0);
    }
    // Instances and variable fonts: by position.
    let labels = read_labels(&doc.root, &axes)?;
    let ctx = instances::Ctx { family: &font.family_name, axes: &axes, labels: &labels, unit: &unit };
    let statics: Vec<&Instance> = font.instances.iter().filter(|i| !is_variable(i)).collect();
    let variables: Vec<&Instance> = font.instances.iter().filter(|i| is_variable(i)).collect();
    let kept_statics: Vec<Instance> = static_elements(&doc.root).into_iter().map(|e| instances::read_static(e, &ctx)).collect::<Result<_>>()?;
    let kept_variables: Vec<Instance> = variable_elements(&doc.root).into_iter().map(|e| instances::read_variable(e, &ctx)).collect::<Result<_>>()?;
    if let Some(list) = doc.root.elements_mut().find(|e| e.name() == "instances") {
        for i in (statics.len()..kept_statics.len()).rev() {
            remove_child(list, i);
        }
        for ((el, model), kept) in list.elements_mut().filter(|e| e.name() == "instance").zip(&statics).zip(&kept_statics) {
            if kept != *model {
                instances::patch_static(el, kept, model, font, &axes, &names, &unit)?;
            }
        }
        for inst in statics.iter().skip(kept_statics.len()) {
            append_child(list, instances::static_element(font, inst, &names, &axes), &unit, 1);
        }
    } else if !statics.is_empty() {
        let mut list = Element::new("instances");
        for inst in &statics {
            list.children.push(XNode::Element(instances::static_element(font, inst, &names, &axes)));
        }
        append_child(&mut doc.root, list, &unit, 0);
    }
    if let Some(list) = doc.root.elements_mut().find(|e| e.name() == "variable-fonts") {
        for i in (variables.len()..kept_variables.len()).rev() {
            remove_child(list, i);
        }
        for ((el, model), kept) in list.elements_mut().filter(|e| e.name() == "variable-font").zip(&variables).zip(&kept_variables) {
            if kept != *model {
                instances::patch_variable(el, kept, model, font, &unit)?;
            }
        }
        for inst in variables.iter().skip(kept_variables.len()) {
            append_child(list, instances::variable_element(font, inst, &names), &unit, 1);
        }
    } else if !variables.is_empty() {
        let mut list = Element::new("variable-fonts");
        for inst in &variables {
            list.children.push(XNode::Element(instances::variable_element(font, inst, &names)));
        }
        let at = doc.root.elements().position(|e| e.name() == "instances").unwrap_or(usize::MAX);
        insert_child(&mut doc.root, at, list, &unit, 0);
    }
    Ok(doc.write())
}

fn location_element(names: &[String], loc: &[f64]) -> Element {
    let mut el = Element::new("location");
    for (n, v) in names.iter().zip(loc) {
        el.children.push(XNode::Element(Element::new("dimension").with_attr("name", n).with_attr("xvalue", int_or_float(*v))));
    }
    el
}

fn source_element(font: &Font, m: &Master, file_of: &HashMap<&str, String>, names: &[String]) -> Element {
    let mut el = Element::new("source");
    if let Some(f) = file_of.get(m.id.as_str()) {
        el.set_attr("filename", f);
    }
    el.set_attr("name", &m.id);
    if !font.family_name.is_empty() {
        el.set_attr("familyname", &font.family_name);
    }
    if !m.name.is_empty() {
        el.set_attr("stylename", &m.name);
    }
    if let Some(l) = ds_str(&m.extra, "layer") {
        el.set_attr("layer", l);
    }
    if !names.is_empty() {
        let loc: Vec<f64> = (0..names.len()).map(|i| fontcraft_model::axes::design_value(&m.location, i)).collect();
        el.children.push(XNode::Element(location_element(names, &loc)));
    }
    el
}

/// The axes as a new document states them, for reading its instances' classes back.
fn new_axis_infos(font: &Font) -> Vec<AxisInfo> {
    (0..font.axes.len())
        .filter_map(|i| {
            let a = font.axes.get(i)?;
            let (map, range) = axes::map_and_range(font, i);
            Some(AxisInfo { name: a.name.clone(), tag: a.tag.clone(), user_default: range.default, map })
        })
        .collect()
}

/// A new designspace in designspaceLib's layout.
fn new_document(font: &Font, file_of: &HashMap<&str, String>) -> String {
    let names: Vec<String> = font.axes.iter().map(|a| a.name.clone()).collect();
    let infos = new_axis_infos(font);
    let mut root = Element::new("designspace").with_attr("format", "5.0");
    if !font.axes.is_empty() {
        let mut axes_el = Element::new("axes");
        for i in 0..font.axes.len() {
            axes_el.children.push(XNode::Element(axes::axis_element(font, i)));
        }
        root.children.push(XNode::Element(axes_el));
    }
    let mut sources = Element::new("sources");
    for m in font.masters.iter() {
        sources.children.push(XNode::Element(source_element(font, m, file_of, &names)));
    }
    root.children.push(XNode::Element(sources));
    let variables: Vec<&Instance> = font.instances.iter().filter(|i| is_variable(i)).collect();
    if !variables.is_empty() {
        let mut list = Element::new("variable-fonts");
        for inst in variables {
            list.children.push(XNode::Element(instances::variable_element(font, inst, &names)));
        }
        root.children.push(XNode::Element(list));
    }
    let statics: Vec<&Instance> = font.instances.iter().filter(|i| !is_variable(i)).collect();
    if !statics.is_empty() {
        let mut list = Element::new("instances");
        for inst in statics {
            list.children.push(XNode::Element(instances::static_element(font, inst, &names, &infos)));
        }
        root.children.push(XNode::Element(list));
    }
    xml::Document::new(xmlplist::DECLARATION, root).write_pretty("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_masters() -> (String, BTreeMap<String, Files>) {
        let mut light = fontcraft_model::Font::new("Duo");
        Arc::make_mut(&mut light.masters)[0].name = "Light".into();
        let mut bold = light.clone();
        Arc::make_mut(&mut bold.masters)[0].name = "Bold".into();
        bold.glyph_mut("a").unwrap().layer_mut("m01").unwrap().width = 650.0;
        let doc = "<?xml version='1.0' encoding='UTF-8'?>
<designspace format=\"5.0\">
  <!-- kept -->
  <axes>
    <axis tag=\"wght\" name=\"Weight\" minimum=\"300\" maximum=\"700\" default=\"300\">
      <map input=\"300\" output=\"0\"/>
      <map input=\"700\" output=\"1000\"/>
    </axis>
  </axes>
  <sources>
    <source filename=\"Duo-Light.ufo\" name=\"light\" stylename=\"Light\">
      <location>
        <dimension name=\"Weight\" xvalue=\"0\"/>
      </location>
    </source>
    <source filename=\"Duo-Bold.ufo\" name=\"bold\" stylename=\"Bold\">
      <location>
        <dimension name=\"Weight\" uservalue=\"700\"/>
      </location>
    </source>
  </sources>
  <instances>
    <instance name=\"Duo Medium\" stylename=\"Medium\">
      <location>
        <dimension name=\"Weight\" xvalue=\"500\"/>
      </location>
    </instance>
  </instances>
</designspace>
";
        let ufos =
            BTreeMap::from([("Duo-Light.ufo".to_string(), ufo::write(&light).unwrap()), ("Duo-Bold.ufo".to_string(), ufo::write(&bold).unwrap())]);
        (doc.to_string(), ufos)
    }

    #[test]
    fn a_designspace_is_one_document_and_comes_back_unchanged() {
        let (doc, ufos) = two_masters();
        assert_eq!(ufo_paths(&doc).unwrap(), ["Duo-Light.ufo", "Duo-Bold.ufo"]);
        let font = read(&doc, &ufos).unwrap();
        assert_eq!(font.masters.len(), 2);
        assert_eq!(font.masters[1].location, [1000.0], "user coordinates map to design coordinates");
        assert_eq!(font.axes[0].tag, "wght");
        assert_eq!(font.instances[0].location, [500.0]);
        assert_eq!(font.glyph("a").unwrap().layer("bold").unwrap().width, 650.0);
        let out = write(&font).unwrap();
        assert_eq!(out.document, doc);
        assert_eq!(out.ufos, ufos);
    }

    #[test]
    fn each_master_keeps_its_own_zones_stems_and_guides() {
        use fontcraft_model::Guide;
        let (doc, mut ufos) = two_masters();
        let edit = |ufos: &mut BTreeMap<String, Files>, file: &str, from: &str, to: &str| {
            let ufo = ufos.get_mut(file).unwrap();
            let info = String::from_utf8(ufo["fontinfo.plist"].clone()).unwrap();
            assert!(info.contains(from), "{info}");
            ufo.insert("fontinfo.plist".into(), info.replacen(from, to, 1).into_bytes());
        };
        let stems = |h: &str, v: &str| {
            format!(
                "<key>postscriptStemSnapH</key>\n    <array>\n      <integer>{h}</integer>\n    </array>\n    \
                 <key>postscriptStemSnapV</key>\n    <array>\n      <integer>{v}</integer>\n    </array>\n    <key>postscriptUnderlinePosition</key>"
            )
        };
        edit(&mut ufos, "Duo-Light.ufo", "<key>postscriptUnderlinePosition</key>", &stems("30", "60"));
        edit(&mut ufos, "Duo-Bold.ufo", "<key>postscriptUnderlinePosition</key>", &stems("50", "120"));
        // The bold master's x-height overshoot is larger, and it has a guide.
        edit(&mut ufos, "Duo-Bold.ufo", "<integer>516</integer>", "<integer>520</integer>");
        edit(
            &mut ufos,
            "Duo-Bold.ufo",
            "<key>openTypeOS2Type</key>",
            "<key>guidelines</key>\n    <array>\n      <dict>\n        <key>y</key>\n        <integer>520</integer>\n      </dict>\n    </array>\n    <key>openTypeOS2Type</key>",
        );
        let font = read(&doc, &ufos).unwrap();
        assert_eq!(font.stems.len(), 2);
        let (light, bold) = (&font.masters[0], &font.masters[1]);
        assert_eq!((light.stem_values.as_slice(), bold.stem_values.as_slice()), ([30.0, 60.0].as_slice(), [50.0, 120.0].as_slice()));
        let x = |m: &Master| font.metric(m, fontcraft_model::metric::X_HEIGHT).unwrap().over;
        assert_eq!((x(light), x(bold)), (16.0, 20.0));
        assert!(light.guides.is_empty());
        assert_eq!(bold.guides, [Guide { y: 520.0, ..Default::default() }]);
        assert!(font.masters.iter().all(|m| m.params.is_empty()));
        assert_eq!(font.info.params.0.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), [AXIS_MAPPINGS], "the axis map");
        let out = write(&font).unwrap();
        assert_eq!(out.ufos, ufos);

        // An edit of the default master leaves the other's values alone.
        let mut edited = font.clone();
        Arc::make_mut(&mut edited.masters)[0].stem_values = vec![32.0, 60.0];
        let out = write(&edited).unwrap();
        assert_eq!(out.ufos["Duo-Bold.ufo"], ufos["Duo-Bold.ufo"]);
        assert!(String::from_utf8_lossy(&out.ufos["Duo-Light.ufo"]["fontinfo.plist"]).contains("<integer>32</integer>"));

        // A master with another number of stems keeps its stems as written.
        edit(&mut ufos, "Duo-Bold.ufo", "<integer>50</integer>\n    </array>", "<integer>50</integer>\n      <integer>52</integer>\n    </array>");
        let font = read(&doc, &ufos).unwrap();
        assert!(font.masters[1].stem_values.is_empty());
        assert_eq!(font.masters[1].params.0.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["postscriptStemSnapH", "postscriptStemSnapV"]);
        assert_eq!(write(&font).unwrap().ufos, ufos);
    }

    #[test]
    fn edits_change_only_what_they_touch() {
        let (doc, ufos) = two_masters();
        let mut font = read(&doc, &ufos).unwrap();
        Arc::make_mut(&mut font.masters)[1].location = vec![900.0];
        font.glyph_mut("b").unwrap().layer_mut("light").unwrap().width = 1.0;
        let out = write(&font).unwrap();
        assert!(out.document.contains("<dimension name=\"Weight\" xvalue=\"900\"/>"), "{}", out.document);
        assert!(out.document.contains("<!-- kept -->"));
        assert_eq!(out.ufos["Duo-Bold.ufo"], ufos["Duo-Bold.ufo"]);
        let changed: Vec<&String> =
            out.ufos["Duo-Light.ufo"].iter().filter(|(k, v)| ufos["Duo-Light.ufo"].get(*k) != Some(*v)).map(|(k, _)| k).collect();
        assert_eq!(changed, ["glyphs/b.glif"]);
    }

    #[test]
    fn per_master_differences_survive() {
        let (doc, mut ufos) = two_masters();
        // The bold master's `a` has another code point, and its UFO no family name.
        let bold = ufos.get_mut("Duo-Bold.ufo").unwrap();
        let glif = String::from_utf8(bold["glyphs/a.glif"].clone()).unwrap().replace("<unicode hex=\"0061\"/>", "<unicode hex=\"0062\"/>");
        bold.insert("glyphs/a.glif".into(), glif.into_bytes());
        let info = String::from_utf8(bold["fontinfo.plist"].clone()).unwrap().replace("    <key>familyName</key>\n    <string>Duo</string>\n", "");
        bold.insert("fontinfo.plist".into(), info.into_bytes());
        let font = read(&doc, &ufos).unwrap();
        assert_eq!(font.glyph("a").unwrap().unicodes, [0x61]);
        assert_eq!(write(&font).unwrap().ufos, ufos);
    }

    #[test]
    fn each_master_keeps_its_own_glyph_note_colour_and_production_name() {
        use fontcraft_model::ColorLabel;
        let (doc, mut ufos) = two_masters();
        let edit = |ufos: &mut BTreeMap<String, Files>, ufo: &str, file: &str, from: &str, to: &str| {
            let files = ufos.get_mut(ufo).unwrap();
            let text = String::from_utf8(files[file].clone()).unwrap();
            assert!(text.contains(from), "{text}");
            files.insert(file.into(), text.replacen(from, to, 1).into_bytes());
        };
        let lib = |name: &str| {
            format!("<dict>\n    <key>public.postscriptNames</key>\n    <dict>\n      <key>a</key>\n      <string>{name}</string>\n    </dict>\n")
        };
        let glif = |note: &str, color: &str| {
            format!(
                "  <note>\n{note}\n</note>\n  <lib>\n    <dict>\n      <key>public.markColor</key>\n      <string>{color}</string>\n    </dict>\n  </lib>\n</glyph>"
            )
        };
        edit(&mut ufos, "Duo-Light.ufo", "lib.plist", "<dict>\n", &lib("uni0061"));
        edit(&mut ufos, "Duo-Bold.ufo", "lib.plist", "<dict>\n", &lib("a.bold"));
        edit(&mut ufos, "Duo-Light.ufo", "glyphs/a.glif", "</glyph>", &glif("light", "1,0,0,1"));
        edit(&mut ufos, "Duo-Bold.ufo", "glyphs/a.glif", "</glyph>", &glif("bold", "0,0,1,1"));
        let font = read(&doc, &ufos).unwrap();
        let a = font.glyph("a").unwrap();
        assert_eq!(a.info.note, "light", "the default source's");
        assert_eq!(a.info.color, Some(ColorLabel::Rgba([1.0, 0.0, 0.0, 1.0])));
        assert_eq!(a.info.production.as_deref(), Some("uni0061"));
        assert_eq!(write(&font).unwrap().ufos, ufos, "each master keeps its own");

        // An edit goes to every master.
        let mut edited = font.clone();
        {
            let mut a = edited.glyph_mut("a").unwrap();
            a.info.note = "both".into();
            a.info.color = Some(ColorLabel::Index(0));
            a.info.production = Some("uni0061.alt".into());
        }
        let out = write(&edited).unwrap();
        for ufo in ["Duo-Light.ufo", "Duo-Bold.ufo"] {
            let a = String::from_utf8(out.ufos[ufo]["glyphs/a.glif"].clone()).unwrap();
            assert!(a.contains("<note>\nboth\n</note>") && a.contains("<string>0.85,0.26,0.06,1</string>"), "{ufo}: {a}");
            assert!(String::from_utf8_lossy(&out.ufos[ufo]["lib.plist"]).contains("<string>uni0061.alt</string>"), "{ufo}");
        }
    }

    #[test]
    fn a_new_designspace_is_written_in_designspacelib_layout() {
        let mut font = fontcraft_model::Font::new("Fresh");
        Arc::make_mut(&mut font.axes).push(fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        let mut bold = font.masters[0].clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        bold.location = vec![700.0];
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        Arc::make_mut(&mut font.masters).push(bold);
        let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
        for n in names {
            let mut g = font.glyph_mut(&n).unwrap();
            let mut l = g.layers[0].clone();
            l.id = "m02".into();
            l.master = "m02".into();
            g.layers.push(l);
        }
        let out = write(&font).unwrap();
        assert_eq!(
            out.document,
            "<?xml version='1.0' encoding='UTF-8'?>
<designspace format=\"5.0\">
  <axes>
    <axis tag=\"wght\" name=\"Weight\" minimum=\"400\" maximum=\"700\" default=\"400\"/>
  </axes>
  <sources>
    <source filename=\"Fresh-Regular.ufo\" name=\"m01\" familyname=\"Fresh\" stylename=\"Regular\">
      <location>
        <dimension name=\"Weight\" xvalue=\"400\"/>
      </location>
    </source>
    <source filename=\"Fresh-Bold.ufo\" name=\"m02\" familyname=\"Fresh\" stylename=\"Bold\">
      <location>
        <dimension name=\"Weight\" xvalue=\"700\"/>
      </location>
    </source>
  </sources>
</designspace>
"
        );
        let back = read(&out.document, &out.ufos).unwrap();
        assert_eq!(back.masters.len(), 2);
        assert_eq!(back.glyphs.len(), font.glyphs.len());
        assert_eq!(write(&back).unwrap().ufos, out.ufos);
    }

    #[test]
    fn broken_designspaces_are_errors() {
        let (doc, ufos) = two_masters();
        assert!(read("<nope/>", &ufos).is_err());
        assert!(read("<designspace/>", &ufos).is_err());
        assert!(read(&doc.replace("Duo-Bold.ufo", "Missing.ufo"), &ufos).is_err());
        assert!(read(&doc.replace("xvalue=\"0\"", "xvalue=\"zero\""), &ufos).is_err());
        assert!(read(&doc.replace(" filename=\"Duo-Bold.ufo\"", ""), &ufos).is_err());
    }

    /// The document's own fields as a font: a stand-in master for each source, no glyphs.
    fn document_font(text: &str) -> (Font, HashMap<String, String>) {
        let doc = xml::parse(text).unwrap();
        let root = &doc.root;
        let axes = read_axes(root).unwrap();
        let labels = read_labels(root, &axes).unwrap();
        let elements = source_elements(root);
        let mut font = Font::new("Stand-in");
        let mut file_of = HashMap::new();
        let mut masters = Vec::new();
        for (el, id) in elements.iter().zip(source_ids(&elements)) {
            let location = read_location(el, &axes, &labels).unwrap();
            file_of.insert(id.clone(), el.attr("filename").unwrap_or("").to_string());
            masters.push(Master { id, name: el.attr("stylename").unwrap_or("").into(), location, ..Default::default() });
        }
        let model_axes = model_axes(root).unwrap();
        let refs: Vec<&[f64]> = masters.iter().map(|m| m.location.as_slice()).collect();
        if let Some(v) = doc_mappings(root, &model_axes, &refs).unwrap() {
            Arc::make_mut(&mut font.info).params.0.push(Param::new(AXIS_MAPPINGS, v));
        }
        let ctx = instances::Ctx { family: &font.family_name, axes: &axes, labels: &labels, unit: &indent_unit(root) };
        font.instances = Arc::new(read_instances(root, &ctx).unwrap());
        font.axes = Arc::new(model_axes);
        font.masters = Arc::new(masters);
        (font, file_of)
    }

    fn patched(font: &Font, text: &str, file_of: &HashMap<String, String>) -> String {
        let file_of: HashMap<&str, String> = file_of.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        patch_document(font, text, &file_of).unwrap()
    }

    /// Every designspace document in the corpus, its UFOs or not: its axes, instances and variable
    /// fonts read, and written back unchanged.
    #[test]
    fn every_corpus_document_reads_and_comes_back() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
        let mut stack = vec![corpus.clone()];
        let (mut checked, mut labelled, mut mapped, mut libs) = (0, 0, 0, 0);
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() && !p.ends_with(".git") {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "designspace") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    let (font, file_of) = document_font(&text);
                    assert_eq!(patched(&font, &text, &file_of), text, "{}", p.display());
                    checked += 1;
                    labelled += usize::from(font.axes.iter().any(|a| !a.labels.is_empty() || !a.label_names.is_empty()));
                    mapped += usize::from(font.info.params.get(AXIS_MAPPINGS).is_some());
                    libs += usize::from(font.instances.iter().any(|i| !i.user_data.is_empty()));
                }
            }
        }
        if checked > 0 {
            assert!(checked >= 100 && labelled > 0 && mapped > 0 && libs > 0, "{checked} {labelled} {mapped} {libs}");
        }
    }

    /// [`two_masters`] with a richer document: localised axis names, STAT labels, a discrete
    /// hidden axis, instances with attributes and libs, and a variable font.
    fn rich() -> (String, BTreeMap<String, Files>) {
        let (_, ufos) = two_masters();
        let doc = "<?xml version='1.0' encoding='UTF-8'?>
<designspace format=\"5.0\">
  <axes>
    <axis tag=\"wght\" name=\"Weight\" minimum=\"300\" maximum=\"700\" default=\"300\">
      <labelname xml:lang=\"de\">Gewicht</labelname>
      <labelname xml:lang=\"fr\">Graisse</labelname>
      <map input=\"300\" output=\"0\"/>
      <map input=\"700\" output=\"1000\"/>
      <labels ordering=\"1\">
        <label uservalue=\"300\" name=\"Light\" elidable=\"true\"/>
        <label uservalue=\"700\" userminimum=\"600\" usermaximum=\"700\" name=\"Bold\" linkeduservalue=\"300\">
          <labelname xml:lang=\"de\">Fett</labelname>
        </label>
      </labels>
    </axis>
    <axis tag=\"ital\" name=\"Italic\" values=\"0 1\" default=\"0\" hidden=\"1\"/>
  </axes>
  <sources>
    <source filename=\"Duo-Light.ufo\" name=\"light\" familyname=\"Duo\" stylename=\"Light\">
      <location>
        <dimension name=\"Weight\" xvalue=\"0\"/>
      </location>
    </source>
    <source filename=\"Duo-Bold.ufo\" name=\"bold\" familyname=\"Duo\" stylename=\"Bold\">
      <location>
        <dimension name=\"Weight\" uservalue=\"700\"/>
      </location>
    </source>
  </sources>
  <variable-fonts>
    <variable-font name=\"Upright\" filename=\"DuoVF\">
      <axis-subsets>
        <axis-subset name=\"Weight\"/>
      </axis-subsets>
    </variable-font>
  </variable-fonts>
  <instances>
    <instance name=\"Duo Medium\" familyname=\"Duo\" stylename=\"Medium\" filename=\"instances/Duo-Medium.ufo\" postscriptfontname=\"Duo-Medium\" stylemapfamilyname=\"Duo Medium\" stylemapstylename=\"regular\">
      <location>
        <dimension name=\"Weight\" xvalue=\"500\"/>
      </location>
      <lib>
        <dict>
          <key>com.example.note</key>
          <string>kept</string>
          <key>com.schriftgestaltung.customParameters</key>
          <array>
            <array>
              <string>Remove Glyphs</string>
              <array>
                <string>b</string>
              </array>
            </array>
          </array>
          <key>com.schriftgestaltung.export</key>
          <false/>
        </dict>
      </lib>
    </instance>
    <instance familyname=\"Other\" stylename=\"Bold\" stylemapfamilyname=\"Other\" stylemapstylename=\"bold\">
      <location>
        <dimension name=\"Weight\" uservalue=\"700\"/>
      </location>
    </instance>
  </instances>
</designspace>
";
        (doc.to_string(), ufos)
    }

    /// An instance with its properties, parameters and user data in key order: the lib holds
    /// them sorted, and the attributes' values come after the lib's.
    fn sorted(i: &Instance) -> Instance {
        let mut i = i.clone();
        i.properties.0.sort_by(|a, b| a.key.cmp(&b.key));
        i.params.0.sort_by(|a, b| a.name.cmp(&b.name));
        i.user_data.0.entries.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        i.interpolation.sort_by(|a, b| a.0.cmp(&b.0));
        i
    }

    /// What the document part of a font holds: axes, axis maps, origin and instances.
    #[allow(clippy::type_complexity)]
    fn fields(f: &Font) -> (Vec<fontcraft_model::Axis>, Vec<Option<Vec<(f64, f64)>>>, Option<Value>, Vec<Instance>) {
        let maps = f.axes.iter().map(|a| fontcraft_model::axes::mapping_parameter(f, &a.tag)).collect();
        (f.axes.to_vec(), maps, f.info.params.get(ORIGIN).cloned(), f.instances.iter().map(sorted).collect())
    }

    #[test]
    fn axes_and_instances_are_model_fields() {
        use fontcraft_model::{AxisLabel, AxisRange, LangName, PropertyValue, StyleClass};
        let (doc, ufos) = rich();
        let font = read(&doc, &ufos).unwrap();
        let (w, i) = (&font.axes[0], &font.axes[1]);
        assert_eq!(w.range, Some(AxisRange { minimum: 300.0, default: 300.0, maximum: 700.0 }));
        assert_eq!(
            w.label_names,
            [LangName { language: "de".into(), name: "Gewicht".into() }, LangName { language: "fr".into(), name: "Graisse".into() }]
        );
        assert_eq!(w.ordering, Some(1));
        assert_eq!(
            w.labels[1],
            AxisLabel {
                name: "Bold".into(),
                user_value: 700.0,
                user_minimum: Some(600.0),
                user_maximum: Some(700.0),
                linked_user_value: Some(300.0),
                names: vec![LangName { language: "de".into(), name: "Fett".into() }],
                ..Default::default()
            }
        );
        assert!(w.labels[0].elidable);
        assert_eq!((i.values.as_slice(), i.hidden, i.range.map(|r| r.default)), ([0.0, 1.0].as_slice(), true, Some(0.0)));
        // The map is the `Axis Mappings` parameter, with every axis.
        assert_eq!(fontcraft_model::axes::mapping_parameter(&font, "wght").unwrap(), [(300.0, 0.0), (700.0, 1000.0)]);
        assert_eq!(fontcraft_model::axes::mapping_parameter(&font, "ital").unwrap(), [(0.0, 0.0), (1.0, 1.0)]);
        // Instances: attributes, lib keys, user data, derived classes.
        let (medium, bold, vf) = (&font.instances[0], &font.instances[1], &font.instances[2]);
        assert_eq!((medium.name.as_str(), medium.exports, medium.location.as_slice()), ("Medium", false, [500.0, 0.0].as_slice()));
        assert_eq!(medium.params.0.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Remove Glyphs", "UFO Filename"]);
        assert_eq!(medium.properties.get("postscriptFontName").map(|p| p.value.clone()), Some(PropertyValue::Single("Duo-Medium".into())));
        assert!(medium.properties.get("familyNames").is_none(), "the font's family name is not repeated");
        assert_eq!(medium.user_data.get("com.example.note").and_then(Value::as_str), Some("kept"));
        assert_eq!((medium.weight_class.clone(), medium.link_style.clone()), (Some(StyleClass::Number(500.0)), None));
        assert_eq!(bold.properties.get("familyNames").and_then(|p| p.default_value()), Some("Other"));
        assert_eq!((bold.is_bold, bold.is_italic, bold.link_style.clone()), (true, false, None));
        assert_eq!((bold.location.as_slice(), bold.weight_class.clone()), ([1000.0, 0.0].as_slice(), Some(StyleClass::Number(700.0))));
        assert_eq!((vf.name.as_str(), vf.kind.as_deref()), ("Upright", Some("variable")));
        assert_eq!(vf.params.get("fileName").and_then(Value::as_str), Some("DuoVF"), "not the default, Duo-UprightVF");
        assert_eq!(write(&font).unwrap().document, doc);
    }

    #[test]
    fn edits_to_axes_and_instances_are_written_and_read_back() {
        use fontcraft_model::{AxisLabel, LangName, StyleClass};
        let (doc, ufos) = rich();
        let mut font = read(&doc, &ufos).unwrap();
        {
            let axes = Arc::make_mut(&mut font.axes);
            axes[0].label_names.remove(1);
            axes[0].labels.push(AxisLabel { name: "Medium".into(), user_value: 500.0, ..Default::default() });
            axes[0].range.as_mut().unwrap().maximum = 800.0;
            axes[1].range.as_mut().unwrap().default = 1.0;
            axes[1].label_names.push(LangName { language: "de".into(), name: "Kursiv".into() });
        }
        {
            let info = Arc::make_mut(&mut font.info);
            let mut map = info.params.get(AXIS_MAPPINGS).unwrap().clone();
            if let Value::Dict(d) = &mut map
                && let Some(Value::Dict(w)) = d.get_mut("wght")
            {
                w.insert("500", fontcraft_model::axes::number_value(400.0).unwrap());
            }
            info.params.set(AXIS_MAPPINGS, map);
        }
        {
            let list = Arc::make_mut(&mut font.instances);
            let m = &mut list[0];
            m.name = "Book".into();
            m.exports = true;
            m.manual_interpolation = true;
            m.interpolation = vec![("light".into(), 0.5), ("bold".into(), 0.5)];
            m.params.0.retain(|p| p.name != "UFO Filename");
            m.user_data.0.insert("com.example.more", Value::Bool(true));
            m.location = vec![600.0, 0.0];
            let b = &mut list[1];
            b.is_italic = true;
            b.link_style = Some("Light".into());
            b.weight_class = Some(StyleClass::Name("Bold".into()));
            list[2].params.set("fileName", Value::String("Other-VF".into()));
            let mut extra = list[0].clone();
            extra.name = "Black".into();
            extra.location = vec![1000.0, 1.0];
            // Instances come before variable fonts in the document.
            list.insert(2, extra);
        }
        let out = write(&font).unwrap();
        for expected in [
            "<labels ordering=\"1\">",
            "<label uservalue=\"500\" name=\"Medium\"/>",
            "<map input=\"500\" output=\"400\"/>",
            "maximum=\"800\"",
            "values=\"0 1\" default=\"1\"",
            "<labelname xml:lang=\"de\">Kursiv</labelname>",
            "name=\"Duo Book\"",
            "filename=\"instance_ufos/Duo-Book.ufo\"",
            "stylemapfamilyname=\"Other Light\" stylemapstylename=\"bold italic\"",
            "filename=\"Other-VF\"",
            "<key>com.schriftgestaltung.intanceInterpolations</key>",
        ] {
            assert!(out.document.contains(expected), "{expected} in {}", out.document);
        }
        assert!(!out.document.contains("Graisse") && !out.document.contains("com.schriftgestaltung.export"), "{}", out.document);
        assert!(out.document.contains("<!--") || !doc.contains("<!--"));
        let back = read(&out.document, &out.ufos).unwrap();
        assert_eq!(fields(&back), fields(&font));
        // Unchanged parts keep their text: the first axis's map elements and labels are rewritten
        // only where they changed.
        assert!(out.document.contains("<label uservalue=\"300\" name=\"Light\" elidable=\"true\"/>"));
    }

    #[test]
    fn a_new_document_holds_the_instance_fields() {
        use fontcraft_model::{Localized, Property, PropertyValue, StyleClass};
        let (doc, ufos) = rich();
        let mut font = read(&doc, &ufos).unwrap();
        // Drop the kept document: written as new, from the model.
        Arc::make_mut(&mut font.extra).remove(Family::Designspace, "document");
        {
            let list = Arc::make_mut(&mut font.instances);
            let m = &mut list[0];
            m.weight_class = Some(StyleClass::Name("SemiBold".into()));
            m.width_class = Some(StyleClass::Number(3.0));
            m.manual_interpolation = true;
            m.interpolation = vec![("light".into(), 0.25)];
            m.properties.0.push(Property {
                key: "designers".into(),
                value: PropertyValue::Localized(vec![Localized { language: "dflt".into(), value: "A. Designer".into(), extra: Extra::new() }]),
                extra: Extra::new(),
            });
            m.params.0.push(Param::new("fileName", Value::String("Medium-File".into())));
            m.params.remove("UFO Filename");
            list[1].link_style = Some("Light".into());
            list[2].params.0.push(Param::new("Export STAT Table", Value::Bool(false)));
        }
        let out = write(&font).unwrap();
        assert!(out.document.contains("filename=\"Medium-File.ufo\""), "{}", out.document);
        assert!(out.document.contains("<variable-fonts>\n    <variable-font name=\"Upright\" filename=\"DuoVF\">"), "{}", out.document);
        let back = read(&out.document, &out.ufos).unwrap();
        assert_eq!(back.instances.iter().map(sorted).collect::<Vec<_>>(), font.instances.iter().map(sorted).collect::<Vec<_>>());
        let ranges = |f: &Font| (0..f.axes.len()).map(|i| fontcraft_model::axes::range(f, i)).collect::<Vec<_>>();
        assert_eq!(ranges(&back), ranges(&font));
        assert_eq!(fontcraft_model::axes::mapping_parameter(&back, "wght"), fontcraft_model::axes::mapping_parameter(&font, "wght"));
    }

    #[test]
    fn numbers_are_written_as_designspacelib_writes_them() {
        assert_eq!(int_or_float(400.0), "400");
        assert_eq!(int_or_float(0.5), "0.5");
        assert_eq!(int_or_float(-12.25), "-12.25");
        assert_eq!(int_or_float(1.0 / 3.0), "0.333333");
    }

    #[test]
    fn maps_are_piecewise_linear() {
        let m = [(300.0, 0.0), (500.0, 500.0), (700.0, 1000.0)];
        assert_eq!(piecewise(&m, 400.0), 250.0);
        assert_eq!(piecewise(&m, 600.0), 750.0);
        assert_eq!(piecewise(&m, 100.0), 0.0);
        assert_eq!(piecewise(&[], 5.0), 5.0);
    }
}
