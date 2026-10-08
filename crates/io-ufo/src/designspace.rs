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
//! The designspace document itself is kept as read (the XML tree, comments and deprecated
//! elements included) in the font's [`Extra`] under [`Family::Designspace`] (`document`), and the
//! writer changes it only where the model changed: axis names, tags and hidden flags, source and
//! instance locations, sources and instances added or removed. A document that did not come from
//! a designspace gets one in the layout fontTools' `designspaceLib` writes (format 5.0).
//!
//! Locations are design coordinates in the model, parallel to [`Font::axes`]. A dimension given in
//! user coordinates is mapped through the axis's `map` elements when read and written back
//! unchanged while the design value stays the same (`roundtrip.designspaceCoordinates`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use fontcraft_model::{
    Axis, Extra, Family, Features, FileStore, Font, Glyph, GlyphTable, Instance, KernTable, Kerning, Master, MetricDef, MetricValue, Snippet, Source,
    SourceFormat,
};
use fontcraft_plist::xml::{self, Element, Node as XNode, Text};
use fontcraft_plist::{Array, Value, xmlplist};

use crate::ufo::{self, FEATURES_KEY, ReadUfo, Target};
use crate::{Files, Result, UfoError, bad};

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
    hidden: bool,
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
        out.push(AxisInfo {
            tag: a.attr("tag").unwrap_or("").to_string(),
            hidden: a.attr("hidden").is_some_and(|h| h == "1" || h == "true"),
            name,
            user_default,
            map,
        });
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

    // Metric kinds over every master.
    let mut kinds: Vec<&'static str> = Vec::new();
    for r in read.iter().flatten() {
        for (k, _) in &r.metrics {
            if !kinds.contains(k) {
                kinds.push(k);
            }
        }
    }
    kinds.sort_by_key(|k| ufo::METRICS.iter().position(|(_, m)| m == k));
    let default_features = default.features.clone();
    let (family_name, units_per_em, version_major, version_minor, note, ufo_version) = (
        default.family_name.clone(),
        default.units_per_em,
        default.version_major,
        default.version_minor,
        default.note.clone(),
        default.format_version,
    );
    let default_id = sources.get(default_index).map(|s| s.id.clone()).unwrap_or_default();

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
        master.location = s.location.clone();
        master.metric_values = values_of_owned(&kinds, &r.metrics);
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
                for l in
                    g.layers.iter_mut().filter(|l| l.master == main_id && l.extra.get(Family::Ufo, "layer").and_then(Value::as_str) == Some(layer))
                {
                    l.id = s.id.clone();
                    l.master = s.id.clone();
                    l.extra.remove(Family::Ufo, "layer");
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
    let mut features = Features::default();
    if let Some(code) = default_features {
        features.prefixes.push(Snippet { name: String::new(), code, ..Default::default() });
    }
    let mut extra = Extra::new();
    ds(&mut extra, "document", Value::String(text.into()));
    ds(&mut extra, "defaultSource", Value::String(default_id.as_str().into()));
    let font = Font {
        source: Source { format: SourceFormat::Designspace, ufo_version: Some(ufo_version), ..Default::default() },
        family_name: family_name.unwrap_or_default(),
        units_per_em: ufo::units_per_em(units_per_em)?,
        version_major: version_major.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        version_minor: version_minor.and_then(|v| u16::try_from(v).ok()).unwrap_or(0),
        axes: Arc::new(axes.iter().map(|a| Axis { name: a.name.clone(), tag: a.tag.clone(), hidden: a.hidden, extra: Extra::new() }).collect()),
        metrics: Arc::new(kinds.iter().map(|k| MetricDef::new(k)).collect()),
        masters: Arc::new(masters),
        instances: Arc::new(read_instances(root, &axes, &labels)?),
        glyphs: GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect()),
        info: Default::default(),
        kerning: Arc::new(Kerning { ltr: KernTable(kerning), ..Default::default() }),
        features: Arc::new(features),
        display_strings: Arc::default(),
        note: note.unwrap_or_default(),
        files: Arc::new(FileStore(files)),
        extra: Arc::new(extra),
    };
    font.check()?;
    Ok(font)
}

fn values_of_owned(kinds: &[&'static str], metrics: &[(&'static str, f64)]) -> Vec<MetricValue> {
    kinds.iter().map(|k| MetricValue { pos: metrics.iter().find(|(m, _)| m == k).map_or(0.0, |(_, v)| *v), over: 0.0 }).collect()
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
        if g.file_name != merged.file_name && l.extra.get(Family::Ufo, "fileName").is_none() {
            let own = g.file_name.clone().or_else(|| crate::names::glyph_file_name(&g.name, &std::collections::HashSet::new()));
            if let Some(own) = own {
                l.extra.push(Family::Ufo, "fileName", Value::String(own.into()));
            }
        }
    }
    merged.layers.extend(layers);
}

fn read_instances(root: &Element, axes: &[AxisInfo], labels: &HashMap<String, Vec<f64>>) -> Result<Vec<Instance>> {
    let Some(instances) = root.child("instances") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for el in instances.elements().filter(|e| e.name() == "instance") {
        if out.len() >= MAX_ITEMS {
            return Err(bad(DOC, format!("more than {MAX_ITEMS} instances")));
        }
        let name = el.attr("stylename").or_else(|| el.attr("name")).unwrap_or("").to_string();
        out.push(Instance { name, location: read_location(el, axes, labels)?, exports: true, ..Default::default() });
    }
    Ok(out)
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
        .or_else(|| font.masters.first())
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

fn patch_document(font: &Font, kept: &str, file_of: &HashMap<&str, String>) -> Result<String> {
    let mut doc = xml::parse(kept).map_err(|e| UfoError::Syntax { file: DOC.into(), error: e })?;
    let unit = indent_unit(&doc.root);
    let axes = read_axes(&doc.root)?;
    if axes.len() != font.axes.len() {
        return Err(UfoError::Unsupported("adding or removing axes of a designspace".into()));
    }
    let names: Vec<String> = font.axes.iter().map(|a| a.name.clone()).collect();
    // Axes.
    if let Some(axes_el) = doc.root.elements_mut().find(|e| e.name() == "axes") {
        for (el, axis) in axes_el.elements_mut().filter(|e| e.name() == "axis").zip(font.axes.iter()) {
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
    // Instances: by position.
    let count = doc.root.child("instances").map_or(0, |i| i.elements().filter(|e| e.name() == "instance").count());
    if let Some(instances) = doc.root.elements_mut().find(|e| e.name() == "instances") {
        for i in (font.instances.len()..count).rev() {
            remove_child(instances, i);
        }
        for (el, inst) in instances.elements_mut().filter(|e| e.name() == "instance").zip(font.instances.iter()) {
            patch_location(el, &axes, &names, &inst.location, &unit, 2);
        }
        for inst in font.instances.iter().skip(count) {
            append_child(instances, instance_element(font, inst, &names), &unit, 1);
        }
    } else if !font.instances.is_empty() {
        let mut instances = Element::new("instances");
        for inst in font.instances.iter() {
            instances.children.push(XNode::Element(instance_element(font, inst, &names)));
        }
        append_child(&mut doc.root, instances, &unit, 0);
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
        el.children.push(XNode::Element(location_element(names, &m.location)));
    }
    el
}

fn instance_element(font: &Font, inst: &Instance, names: &[String]) -> Element {
    let mut el = Element::new("instance");
    if !font.family_name.is_empty() {
        el.set_attr("familyname", &font.family_name);
    }
    if !inst.name.is_empty() {
        el.set_attr("stylename", &inst.name);
    }
    if !names.is_empty() {
        el.children.push(XNode::Element(location_element(names, &inst.location)));
    }
    el
}

/// A new designspace in designspaceLib's layout.
fn new_document(font: &Font, file_of: &HashMap<&str, String>) -> String {
    let names: Vec<String> = font.axes.iter().map(|a| a.name.clone()).collect();
    let default = font.masters.first();
    let mut root = Element::new("designspace").with_attr("format", "5.0");
    if !font.axes.is_empty() {
        let mut axes = Element::new("axes");
        for (i, a) in font.axes.iter().enumerate() {
            let values: Vec<f64> = font.masters.iter().filter_map(|m| m.location.get(i).copied()).collect();
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let def = default.and_then(|m| m.location.get(i).copied()).unwrap_or(0.0);
            let (min, max) = if min.is_finite() && max.is_finite() { (min, max) } else { (def, def) };
            let mut el = Element::new("axis")
                .with_attr("tag", &a.tag)
                .with_attr("name", &a.name)
                .with_attr("minimum", int_or_float(min))
                .with_attr("maximum", int_or_float(max))
                .with_attr("default", int_or_float(def));
            if a.hidden {
                el.set_attr("hidden", "1");
            }
            axes.children.push(XNode::Element(el));
        }
        root.children.push(XNode::Element(axes));
    }
    let mut sources = Element::new("sources");
    for m in font.masters.iter() {
        sources.children.push(XNode::Element(source_element(font, m, file_of, &names)));
    }
    root.children.push(XNode::Element(sources));
    if !font.instances.is_empty() {
        let mut instances = Element::new("instances");
        for inst in font.instances.iter() {
            instances.children.push(XNode::Element(instance_element(font, inst, &names)));
        }
        root.children.push(XNode::Element(instances));
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
    fn a_new_designspace_is_written_in_designspacelib_layout() {
        let mut font = fontcraft_model::Font::new("Fresh");
        Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), hidden: false, extra: Extra::new() });
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
