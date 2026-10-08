//! Format version 2 (files without `.formatVersion`).
//!
//! What differs from version 3, as the GlyphsSDK version 2 document and the version 2 schema
//! (`glyphs-1.schema.json`) describe it and the corpus confirms:
//!
//! - Master metrics are fixed keys (`ascender`, `capHeight`, `xHeight`, `descender`, always
//!   written; `italicAngle`, left out when zero) and overshoots are a separate list of alignment
//!   zones `"{position, size}"`, written in descending order of position. In the model they are
//!   the metric definitions ascender, cap height, x-height, baseline, descender and italic angle;
//!   a zone becomes the overshoot of the metric at its position, and a zone that matches no
//!   metric is one of the master's other zones.
//! - Code points are uppercase hexadecimal, several joined by commas.
//! - A layer has separate `paths` and `components` lists. In the model the paths come first.
//! - Nodes are strings `"x y TYPE"` with an optional ` SMOOTH`; types `LINE`, `CURVE`, `QCURVE`,
//!   `OFFCURVE`. A closed path's start node is written last, as in version 3.
//! - A component names its glyph in `name` and has a `transform` string `"{a, b, c, d, tx, ty}"`;
//!   an anchor has a `position` string `"{x, y}"`.
//! - Axes are not listed: a master has up to six axis values in fixed keys (`weightValue`,
//!   `widthValue`, `customValue`, `customValue1` to `customValue3`), an instance in
//!   `interpolationWeight` and its neighbours. An `Axes` custom parameter, when the font has one,
//!   names and tags the axes; otherwise the model gets Weight (`wght`), Width (`wdth`), Custom
//!   (`XXXX`) and so on, as many as the highest key any master or instance uses. A value at its
//!   default (100, 100, then 0) is left out, as glyphsLib's writer leaves it out.
//! - A master's name may be left out: the reference then derives it from the `Master Name`
//!   parameter or from the name parts `weight`, `width` and `custom`.
//! - Stems are two plain lists, `horizontalStems` and `verticalStems`; the model's stem
//!   definitions (without names) are made from them when every master has the same number of each.
//!   Guides are `guideLines`, with a `position` string `"{x, y}"`.
//! - An instance is left out of the export by `exports = 0` (the format document) or `active = 0`
//!   (glyphsLib writes both); either key is kept as written.
//!
//! Numbers inside those strings are written canonically. A string whose numbers were written
//! another way (`1.0`) is kept beside the values and written again while it still reads as them.
//!
//! Keys are sorted byte-wise except in glyphs, layers, masters and instances, which have a fixed
//! order. Those orders ([`GLYPH_ORDER`], [`LAYER_ORDER`], [`MASTER_ORDER`], [`INSTANCE_ORDER`]),
//! the default axes and the axis value defaults are taken from glyphsLib's format 2 writer
//! (`glyphsLib/classes.py`, version 6.15.0, Apache-2.0), which reproduces the reference app's
//! output; the corpus agrees wherever it has the keys.

use std::sync::Arc;

use fontcraft_model::{
    AlignmentZone, Anchor, Axis, Component, Extra, Family, Font, FontInfo, Glyph, GlyphTable, Guide, Instance, Layer, MAX_NAME_LEN, Master,
    MetricDef, MetricValue, NameParts, Node, NodeKind, Param, Params, Path, Properties, RawShape, Settings, Shape, Source, SourceFormat, StemDef,
    Transform, metric,
};
use fontcraft_plist::openstep::canonical_number;
use fontcraft_plist::{Array, Dict, Layout, Num, Number, Str, Value};

use crate::design::{self, same_text, take_text};
use crate::{
    GlyphsError, Result, as_f64, as_items, as_u16, bad, finish, int, into_extra, list, num, opt_string, push_features, push_kern_groups,
    push_ui_and_note, string, take, take_features, take_kern_groups, take_kern_table, take_note, take_strings, write_kern_table,
};
use fontcraft_model::Kerning;

const KERN_GROUP_KEYS: [&str; 4] = ["leftKerningGroup", "rightKerningGroup", "topKerningGroup", "bottomKerningGroup"];

/// The metric definitions of a version 2 document, in this order.
const KINDS: [&str; 6] = [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER, metric::ITALIC_ANGLE];
/// The master keys that hold the positions of [`KINDS`] (the baseline has none).
const KEYS: [Option<&str>; 6] = [Some("ascender"), Some("capHeight"), Some("xHeight"), None, Some("descender"), Some("italicAngle")];

/// Glyph keys in the order format 2 writes them (from glyphsLib, see the module notes).
const GLYPH_ORDER: &[&str] = &[
    "color",
    "export",
    "glyphname",
    "production",
    "lastChange",
    "layers",
    "locked",
    "leftKerningGroup",
    "leftMetricsKey",
    "widthMetricsKey",
    "rightKerningGroup",
    "rightMetricsKey",
    "vertWidthMetricsKey",
    "note",
    "topKerningGroup",
    "topMetricsKey",
    "bottomKerningGroup",
    "bottomMetricsKey",
    "unicode",
    "script",
    "category",
    "subCategory",
    "userData",
    "partsSettings",
];
const LAYER_ORDER: &[&str] = &[
    "anchors",
    "annotations",
    "associatedMasterId",
    "background",
    "backgroundImage",
    "color",
    "components",
    "guideLines",
    "hints",
    "layerId",
    "leftMetricsKey",
    "rightMetricsKey",
    "widthMetricsKey",
    "name",
    "paths",
    "userData",
    "vertOrigin",
    "vertWidth",
    "visible",
    "width",
];
const MASTER_ORDER: &[&str] = &[
    "alignmentZones",
    "ascender",
    "capHeight",
    "custom",
    "customValue",
    "customValue1",
    "customValue2",
    "customValue3",
    "customParameters",
    "descender",
    "guideLines",
    "horizontalStems",
    "iconName",
    "id",
    "italicAngle",
    "name",
    "userData",
    "verticalStems",
    "visible",
    "weight",
    "weightValue",
    "width",
    "widthValue",
    "xHeight",
];
const INSTANCE_ORDER: &[&str] = &[
    "active",
    "exports",
    "customParameters",
    "interpolationCustom",
    "interpolationCustom1",
    "interpolationCustom2",
    "interpolationCustom3",
    "interpolationWeight",
    "interpolationWidth",
    "instanceInterpolations",
    "isBold",
    "isItalic",
    "linkStyle",
    "manualInterpolation",
    "name",
    "userData",
    "weightClass",
    "widthClass",
];

/// How many axes version 2 can hold.
const SLOTS: usize = 6;
/// A master's axis values, axis by axis.
const MASTER_SLOTS: [&str; SLOTS] = ["weightValue", "widthValue", "customValue", "customValue1", "customValue2", "customValue3"];
/// An instance's axis values, axis by axis.
const INSTANCE_SLOTS: [&str; SLOTS] =
    ["interpolationWeight", "interpolationWidth", "interpolationCustom", "interpolationCustom1", "interpolationCustom2", "interpolationCustom3"];
/// The value an axis has when its key is left out.
const SLOT_DEFAULTS: [f64; SLOTS] = [100.0, 100.0, 0.0, 0.0, 0.0, 0.0];
/// The axes without an `Axes` parameter (glyphsLib's default definitions; its tag rule for the
/// further custom axes).
const SLOT_AXES: [(&str, &str); SLOTS] =
    [("Weight", "wght"), ("Width", "wdth"), ("Custom", "XXXX"), ("Custom1", "XXX3"), ("Custom2", "XXX4"), ("Custom3", "XXX5")];

/// [`finish`], then the keys in `order` by their place in it. A key not in `order` goes just
/// before the first listed key that sorts after it, so the result is still a total order.
fn finish_ordered(entries: Vec<(Str, Value)>, extra: &Extra, order: &[&str]) -> Dict {
    let mut d = finish(entries, extra);
    let rank = |k: &str| -> (usize, String) {
        match order.iter().position(|o| *o == k) {
            Some(i) => (2 * i + 1, String::new()),
            None => {
                let j = order.iter().position(|o| o.as_bytes() > k.as_bytes()).unwrap_or(order.len());
                (2 * j, k.to_string())
            }
        }
    };
    d.entries.sort_by_cached_key(|(k, _)| rank(k.as_str()));
    d
}

fn fmt(f: f64) -> String {
    canonical_number(Num::Float(f))
}

/// The numbers of a string such as `"{1, 0, 0, 1, 10, 0}"`, if it has exactly `n`.
fn braced_numbers(s: &str, n: usize) -> Option<Vec<f64>> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let nums: Vec<f64> = inner.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|f| f.is_finite())).collect::<Option<_>>()?;
    (nums.len() == n).then_some(nums)
}

fn braced(nums: &[f64]) -> String {
    format!("{{{}}}", nums.iter().map(|f| fmt(*f)).collect::<Vec<_>>().join(", "))
}

/// The string to write for `nums`: the one the file had, if it still reads as them, or the
/// canonical one.
fn spelled(kept: Option<&Value>, nums: &[f64]) -> String {
    match kept.and_then(Value::as_str) {
        Some(s) if braced_numbers(s, nums.len()).as_deref() == Some(nums) => s.to_string(),
        _ => braced(nums),
    }
}

/// Version 2 keeps these names at the top level; the model keeps them as properties.
const NAMES: [(&str, &str, bool); 5] = [
    ("copyright", "copyrights", true),
    ("designer", "designers", true),
    ("designerURL", "designerURL", false),
    ("manufacturer", "manufacturers", true),
    ("manufacturerURL", "manufacturerURL", false),
];

/// The font information of a version 2 root: names, parameters, settings, date and user data.
fn take_font_info(root: &mut Dict) -> FontInfo {
    let mut properties = Properties::default();
    for (key, property, localized) in NAMES {
        if let Some(v) = crate::take_plain_string(root, key) {
            properties.set(property, &v, localized);
        }
    }
    let mut settings = Settings::default();
    crate::info::take_settings_keys(root, &mut settings);
    FontInfo {
        properties,
        params: crate::info::take_params(root),
        settings,
        date: crate::take_plain_string(root, "date"),
        user_data: crate::info::take_user_data(root),
    }
}

/// [`take_font_info`] in reverse. A property version 2 has no key for is not written.
fn push_font_info(e: &mut Vec<(Str, Value)>, font: &Font) -> Result<()> {
    crate::push_font_info_common(e, font, &*params_with_axes(font)?);
    for (key, property, _) in NAMES {
        if let Some(v) = font.info.properties.value(property) {
            e.push((key.into(), string(v)));
        }
    }
    e.extend(crate::info::settings_entries(&font.info.settings)?);
    Ok(())
}

pub(crate) fn read_font(mut root: Dict) -> Result<Font> {
    let app_version = crate::take_app_version(&mut root);
    let font_info = take_font_info(&mut root);
    let family_name = opt_string(&mut root, "familyName", "the file")?.unwrap_or_default();
    let units_per_em = as_u16(&take(&mut root, "unitsPerEm").ok_or_else(|| bad("the file", "no unitsPerEm"))?, "unitsPerEm")?;
    let version_major = take(&mut root, "versionMajor").map(|v| as_u16(&v, "versionMajor")).transpose()?.unwrap_or(0);
    let version_minor = take(&mut root, "versionMinor").map(|v| as_u16(&v, "versionMinor")).transpose()?.unwrap_or(0);
    let master_values = match take(&mut root, "fontMaster") {
        Some(v) => as_items(v, "fontMaster")?,
        None => Vec::new(),
    };
    let axes = axes_of(&font_info.params, &master_values, root.get("instances"));
    let n = axes.len();
    let stem_layout = stem_layout(&master_values);
    let masters = master_values
        .into_iter()
        .enumerate()
        .map(|(i, m)| read_master(m, &format!("fontMaster[{i}]"), n, stem_layout.is_some()))
        .collect::<Result<_>>()?;
    let instances = design::take_checked(&mut root, "instances", |v| read_instance(v, n), |i| write_instance(i, n).ok());
    let stems = match stem_layout {
        Some((h, v)) => {
            let def = |horizontal| StemDef { horizontal, ..Default::default() };
            std::iter::repeat_n(def(true), h).chain(std::iter::repeat_n(def(false), v)).collect()
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
    let kerning =
        Kerning { ltr: take_kern_table(&mut root, "kerning"), rtl: Default::default(), vertical: take_kern_table(&mut root, "vertKerning") };
    let features = take_features(&mut root, "name");
    let display_strings = take_strings(&mut root, "DisplayStrings");
    let note = take_note(&mut root);
    Ok(Font {
        source: Source { format: SourceFormat::Glyphs, glyphs_version: Some(2), app_version, ..Default::default() },
        family_name,
        units_per_em,
        version_major,
        version_minor,
        axes: Arc::new(axes),
        instances: Arc::new(instances),
        metrics: Arc::new(KINDS.iter().map(|k| MetricDef::new(k)).collect()),
        stems: Arc::new(stems),
        numbers: Arc::default(),
        masters: Arc::new(masters),
        glyphs: GlyphTable::from_glyphs(glyphs),
        info: Arc::new(font_info),
        kerning: Arc::new(kerning),
        features: Arc::new(features),
        display_strings: Arc::new(display_strings),
        note,
        files: Arc::default(),
        extra: Arc::new(into_extra(root)),
    })
}

fn read_master(v: Value, at: &str, axes: usize, stems: bool) -> Result<Master> {
    let mut d = crate::as_dict(v, at)?;
    let id = opt_string(&mut d, "id", at)?.ok_or_else(|| bad(at, "a master has no id"))?;
    let mut values: Vec<MetricValue> = Vec::with_capacity(KINDS.len());
    for key in KEYS {
        let pos = match key.and_then(|k| take(&mut d, k)) {
            Some(v) => as_f64(&v, &format!("{at}.{}", key.unwrap_or_default()))?,
            None => 0.0,
        };
        values.push(MetricValue { pos, over: 0.0 });
    }
    // Each zone becomes the overshoot of the first metric at its position that has none yet.
    let (mut leftover, mut zones) = (Vec::new(), Vec::new());
    if let Some(list) = take(&mut d, "alignmentZones") {
        for z in as_items(list, &format!("{at}.alignmentZones"))? {
            let parsed = z.as_str().and_then(|s| braced_numbers(s, 2));
            let slot = parsed.as_ref().and_then(|p| {
                let (pos, size) = (*p.first()?, *p.get(1)?);
                let i = values.iter().enumerate().position(|(i, v)| i != 5 && v.pos == pos && v.over == 0.0)?;
                Some((i, size))
            });
            let canonical = z.as_str().is_some_and(|s| s == braced(parsed.as_deref().unwrap_or_default()));
            match (slot, parsed.as_deref()) {
                (Some((i, size)), _) if canonical && size != 0.0 => {
                    if let Some(v) = values.get_mut(i) {
                        v.over = size;
                    }
                }
                (_, Some([pos, size])) if canonical => zones.push(AlignmentZone { pos: *pos, size: *size }),
                _ => leftover.push(z),
            }
        }
    }
    if !leftover.is_empty() {
        d.entries.push(("alignmentZones".into(), Value::Array(Array { items: leftover, layout: Layout::Multiline, ..Default::default() })));
    }
    let (icon, visible) = crate::take_master_display(&mut d)?;
    let location = take_slots(&mut d, &MASTER_SLOTS, axes);
    let name_parts = NameParts { weight: take_text(&mut d, "weight"), width: take_text(&mut d, "width"), custom: take_text(&mut d, "custom") };
    let mut stem_values = Vec::new();
    if stems {
        stem_values = design::take_values(&mut d, "horizontalStems");
        stem_values.extend(design::take_values(&mut d, "verticalStems"));
    }
    let guides = design::take_checked(&mut d, "guideLines", read_guide, write_guide);
    let params = crate::info::take_params(&mut d);
    let user_data = crate::info::take_user_data(&mut d);
    let italic = values.get(5).is_some_and(|v| v.pos != 0.0);
    let derived = derived_name(&params, &name_parts, italic);
    let name = match d.get("name") {
        // A name the reference would derive anyway stays as written (the writer leaves it out).
        Some(v) => match v.as_str().filter(|s| usable_name(s)).map(str::to_string) {
            Some(n) => {
                if n != derived && same_text(v, &string(&n)) {
                    take(&mut d, "name");
                }
                n
            }
            None => String::new(),
        },
        None if usable_name(&derived) => derived,
        None => String::new(),
    };
    Ok(Master {
        id,
        name,
        location,
        metric_values: values,
        icon,
        visible,
        name_parts,
        stem_values,
        zones,
        guides,
        params,
        user_data,
        extra: into_extra(d),
        ..Default::default()
    })
}

fn hex_code_points(v: &Value) -> Option<Vec<u32>> {
    let text = match v {
        Value::String(s) => s.as_str().to_string(),
        Value::Number(n) => match n.value() {
            Num::Int(i) => i.to_string(),
            Num::Float(_) => return None,
        },
        _ => return None,
    };
    text.split(',').map(|h| u32::from_str_radix(h.trim(), 16).ok()).collect()
}

fn read_glyph(v: Value, at: &str) -> Result<Glyph> {
    let mut d = crate::as_dict(v, at)?;
    let name = opt_string(&mut d, "glyphname", at)?.ok_or_else(|| bad(at, "a glyph has no glyphname"))?;
    let at = format!("glyph `{name}`");
    let mut unicodes = Vec::new();
    if let Some(v) = take(&mut d, "unicode") {
        match hex_code_points(&v) {
            Some(u) if write_unicode(&u).as_ref() == Some(&v) => unicodes = u,
            _ => d.entries.push(("unicode".into(), v)),
        }
    }
    let export = take(&mut d, "export").map(|v| crate::as_bool01(&v, &format!("{at}.export"))).transpose()?.unwrap_or(true);
    let kern_groups = take_kern_groups(&mut d, KERN_GROUP_KEYS, &at)?;
    let layers = match take(&mut d, "layers") {
        Some(v) => as_items(v, &at)?.into_iter().map(|l| read_layer(l, &at)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Glyph { name, unicodes, export, kern_groups, layers, file_name: None, extra: into_extra(d) })
}

fn read_layer(v: Value, at: &str) -> Result<Layer> {
    let mut d = crate::as_dict(v, at)?;
    let id = opt_string(&mut d, "layerId", at)?.ok_or_else(|| bad(at, "a layer has no layerId"))?;
    let at = format!("{at} layer `{id}`");
    let master = opt_string(&mut d, "associatedMasterId", &at)?.unwrap_or_else(|| id.clone());
    let width = take(&mut d, "width").map(|v| as_f64(&v, &format!("{at}.width"))).transpose()?.unwrap_or(0.0);
    let mut shapes = Vec::new();
    if let Some(v) = take(&mut d, "paths") {
        shapes.extend(as_items(v, &at)?.into_iter().map(|p| read_path(p.clone()).unwrap_or_else(|| raw(p))));
    }
    if let Some(v) = take(&mut d, "components") {
        shapes.extend(as_items(v, &at)?.into_iter().map(|c| read_component(c.clone()).unwrap_or_else(|| raw(c))));
    }
    let anchors = match take(&mut d, "anchors") {
        Some(v) => as_items(v, &at)?.into_iter().map(|a| read_anchor(a, &at)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Layer { id, master, width, shapes, anchors, extra: into_extra(d) })
}

fn raw(value: Value) -> Shape {
    Shape::Raw(RawShape { family: Family::Glyphs, value })
}

fn read_node(v: &Value) -> Option<Node> {
    let s = v.as_str()?;
    let mut parts = s.split(' ');
    let x = parts.next()?.parse::<f64>().ok().filter(|f| f.is_finite())?;
    let y = parts.next()?.parse::<f64>().ok().filter(|f| f.is_finite())?;
    let kind = match parts.next()? {
        "LINE" => NodeKind::Line,
        "CURVE" => NodeKind::Curve,
        "QCURVE" => NodeKind::QCurve,
        "OFFCURVE" => NodeKind::OffCurve,
        _ => return None,
    };
    let smooth = match parts.next() {
        None => false,
        Some("SMOOTH") => true,
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    let node = Node { x, y, kind, smooth, extra: Extra::new() };
    // Only a node string written the canonical way becomes a node; anything else keeps the path
    // as written.
    (node_string(&node) == s).then_some(node)
}

fn node_string(n: &Node) -> String {
    let kind = match n.kind {
        NodeKind::Line => "LINE",
        NodeKind::Curve => "CURVE",
        NodeKind::QCurve => "QCURVE",
        NodeKind::OffCurve => "OFFCURVE",
    };
    let smooth = if n.smooth { " SMOOTH" } else { "" };
    format!("{} {} {kind}{smooth}", fmt(n.x), fmt(n.y))
}

fn read_path(v: Value) -> Option<Shape> {
    let Value::Dict(mut d) = v else { return None };
    let closed = crate::as_bool01(&take(&mut d, "closed")?, "closed").ok()?;
    let mut nodes = as_items(take(&mut d, "nodes")?, "nodes").ok()?.iter().map(read_node).collect::<Option<Vec<_>>>()?;
    if closed && !nodes.is_empty() {
        nodes.rotate_right(1);
    }
    let path = Path { closed, nodes, extra: into_extra(d) };
    path.to_bez().ok()?;
    Some(Shape::Path(path))
}

fn read_component(v: Value) -> Option<Shape> {
    let Value::Dict(mut d) = v else { return None };
    let glyph = take(&mut d, "name")?.as_str()?.to_string();
    let m = match d.get("transform") {
        Some(t) => {
            let n = braced_numbers(t.as_str()?, 6)?;
            let m: [f64; 6] = n.try_into().ok()?;
            // A transform not written canonically stays in `extra` beside the values.
            if t.as_str() == Some(braced(&m).as_str()) {
                take(&mut d, "transform");
            }
            m
        }
        None => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    };
    Some(Shape::Component(Component { glyph, transform: Transform::Matrix(m), extra: into_extra(d) }))
}

fn read_anchor(v: Value, at: &str) -> Result<Anchor> {
    let mut d = crate::as_dict(v, at)?;
    let name = opt_string(&mut d, "name", at)?.unwrap_or_default();
    let (x, y) = match d.get("position").and_then(Value::as_str).and_then(|s| braced_numbers(s, 2)) {
        Some(p) => {
            let (x, y) = (p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0));
            if d.get("position").and_then(Value::as_str) == Some(braced(&[x, y]).as_str()) {
                take(&mut d, "position");
            }
            (x, y)
        }
        None => (0.0, 0.0),
    };
    Ok(Anchor { name, x, y, extra: into_extra(d) })
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// Code points as version 2 writes them: bare when the hexadecimal text has no letters and no
/// leading zero (it reads as a number then), otherwise as a string.
fn write_unicode(u: &[u32]) -> Option<Value> {
    if u.is_empty() {
        return None;
    }
    let text = u.iter().map(|c| format!("{c:04X}")).collect::<Vec<_>>().join(",");
    Some(match text.parse::<i64>() {
        Ok(i) if !text.starts_with('0') => Value::Number(Number::int(i)),
        _ => Value::String(Str::new(text)),
    })
}

pub(crate) fn write_font(font: &Font) -> Result<Dict> {
    let mut e: Vec<(Str, Value)> = Vec::new();
    if !font.family_name.is_empty() {
        e.push(("familyName".into(), string(&font.family_name)));
    }
    e.push(("unitsPerEm".into(), int(font.units_per_em)));
    e.push(("versionMajor".into(), int(font.version_major)));
    e.push(("versionMinor".into(), int(font.version_minor)));
    push_font_info(&mut e, font)?;
    e.push(("fontMaster".into(), list(font.masters.iter().map(|m| write_master(font, m)).collect::<Result<_>>()?)));
    e.push(("glyphs".into(), list(font.glyphs.iter().map(|g| write_glyph(g).map(Value::Dict)).collect::<Result<_>>()?)));
    push_features(&mut e, &font.features, "name");
    push_ui_and_note(&mut e, font);
    if !font.instances.is_empty() {
        let n = font.axes.len();
        e.push(("instances".into(), list(font.instances.iter().map(|i| write_instance(i, n)).collect::<Result<_>>()?)));
    }
    if !font.kerning.ltr.is_empty() {
        e.push(("kerning".into(), write_kern_table(&font.kerning.ltr)?));
    }
    if !font.kerning.vertical.is_empty() {
        e.push(("vertKerning".into(), write_kern_table(&font.kerning.vertical)?));
    }
    if !font.kerning.rtl.is_empty() {
        return Err(GlyphsError::Unsupported("right-to-left kerning in format version 2".into()));
    }
    Ok(finish(e, &font.extra))
}

fn write_master(font: &Font, m: &Master) -> Result<Value> {
    let at = format!("master `{}`", m.id);
    let mut e = vec![("id".into(), string(&m.id))];
    let value = |kind: &str| font.metric(m, kind).unwrap_or_default();
    let derived = derived_name(&m.params, &m.name_parts, value(metric::ITALIC_ANGLE).pos != 0.0);
    let name_differs = match m.extra.get(Family::Glyphs, "name") {
        Some(kept) => kept.as_str() != Some(m.name.as_str()),
        None => m.name != derived,
    };
    if !m.name.is_empty() && name_differs {
        e.push(("name".into(), string(&m.name)));
    }
    for (key, part) in [("weight", &m.name_parts.weight), ("width", &m.name_parts.width), ("custom", &m.name_parts.custom)] {
        design::push_text(&mut e, key, part);
    }
    push_slots(&mut e, &m.extra, &MASTER_SLOTS, &m.location, font.axes.len())?;
    let (mut h, mut v) = (Vec::new(), Vec::new());
    for (def, x) in font.stems.iter().zip(&m.stem_values) {
        if def.horizontal { &mut h } else { &mut v }.push(num(*x, &at)?);
    }
    for (key, stems) in [("horizontalStems", h), ("verticalStems", v)] {
        if !stems.is_empty() {
            e.push((key.into(), list(stems)));
        }
    }
    if !m.guides.is_empty() {
        let guides =
            m.guides.iter().map(|g| write_guide(g).ok_or_else(|| bad(&at, "a guide position is not a finite number"))).collect::<Result<_>>()?;
        e.push(("guideLines".into(), list(guides)));
    }
    for (kind, key) in KINDS.iter().zip(KEYS) {
        let Some(key) = key else { continue };
        let pos = value(kind).pos;
        if *kind != metric::ITALIC_ANGLE || pos != 0.0 {
            e.push((key.into(), num(pos, &at)?));
        }
    }
    if m.zones.iter().any(|z| !(z.pos.is_finite() && z.size.is_finite())) {
        return Err(bad(&at, "an alignment zone is not a finite number"));
    }
    // Zones: the overshoots of the vertical metrics, the master's other zones and the zones kept
    // as written, in descending order of position.
    let mut zones: Vec<(f64, Value)> = KINDS
        .iter()
        .filter(|k| **k != metric::ITALIC_ANGLE)
        .map(|k| value(k))
        .filter(|v| v.over != 0.0)
        .map(|v| (v.pos, Value::String(Str::new(braced(&[v.pos, v.over])))))
        .chain(m.zones.iter().map(|z| (z.pos, Value::String(Str::new(braced(&[z.pos, z.size]))))))
        .collect();
    if let Some(Value::Array(kept)) = m.extra.get(Family::Glyphs, "alignmentZones") {
        for z in &kept.items {
            let pos = z.as_str().and_then(|s| braced_numbers(s, 2)).and_then(|p| p.first().copied()).unwrap_or(f64::NEG_INFINITY);
            zones.push((pos, z.clone()));
        }
    }
    if !zones.is_empty() {
        zones.sort_by(|a, b| b.0.total_cmp(&a.0));
        e.push(("alignmentZones".into(), list(zones.into_iter().map(|(_, z)| z).collect())));
    }
    crate::push_master_info(&mut e, m);
    Ok(Value::Dict(finish_ordered(e, &m.extra, MASTER_ORDER)))
}

fn write_glyph(g: &Glyph) -> Result<Dict> {
    let at = format!("glyph `{}`", g.name);
    let mut e = vec![("glyphname".into(), string(&g.name))];
    if let Some(u) = write_unicode(&g.unicodes) {
        e.push(("unicode".into(), u));
    }
    if !g.export {
        e.push(("export".into(), int(0)));
    }
    push_kern_groups(&mut e, &g.kern_groups, KERN_GROUP_KEYS);
    e.push(("layers".into(), list(g.layers.iter().map(|l| write_layer(l, &at)).collect::<Result<_>>()?)));
    Ok(finish_ordered(e, &g.extra, GLYPH_ORDER))
}

fn write_layer(l: &Layer, at: &str) -> Result<Value> {
    let at = format!("{at} layer `{}`", l.id);
    let mut e = vec![("layerId".into(), string(&l.id))];
    if l.id != l.master {
        e.push(("associatedMasterId".into(), string(&l.master)));
    }
    e.push(("width".into(), num(l.width, &at)?));
    let (mut paths, mut components) = (Vec::new(), Vec::new());
    for s in &l.shapes {
        match s {
            Shape::Path(p) => paths.push(write_path(p, &at)?),
            Shape::Component(c) => components.push(write_component(c, &at)?),
            Shape::Raw(r) if r.family == Family::Glyphs => {
                if r.value.as_dict().is_some_and(|d| d.get("nodes").is_some()) {
                    paths.push(r.value.clone());
                } else {
                    components.push(r.value.clone());
                }
            }
            Shape::Raw(_) => return Err(GlyphsError::Unsupported(format!("{at}: a shape read from another format"))),
        }
    }
    if !paths.is_empty() {
        e.push(("paths".into(), list(paths)));
    }
    if !components.is_empty() {
        e.push(("components".into(), list(components)));
    }
    if !l.anchors.is_empty() {
        e.push(("anchors".into(), list(l.anchors.iter().map(write_anchor).collect())));
    }
    Ok(Value::Dict(finish_ordered(e, &l.extra, LAYER_ORDER)))
}

fn write_path(p: &Path, at: &str) -> Result<Value> {
    if p.nodes.iter().any(|n| !(n.x.is_finite() && n.y.is_finite())) {
        return Err(bad(at, "a node coordinate is not a finite number"));
    }
    let mut nodes: Vec<Value> = p.nodes.iter().map(|n| Value::String(Str::new(node_string(n)))).collect();
    if p.closed && !nodes.is_empty() {
        nodes.rotate_left(1);
    }
    let e = vec![("closed".into(), int(i64::from(p.closed))), ("nodes".into(), list(nodes))];
    Ok(Value::Dict(finish(e, &p.extra)))
}

fn write_component(c: &Component, at: &str) -> Result<Value> {
    let mut e = vec![("name".into(), string(&c.glyph))];
    if !c.transform.is_identity() {
        let m = c.affine().as_coeffs();
        if m.iter().any(|f| !f.is_finite()) {
            return Err(bad(at, "a component transform is not a finite number"));
        }
        e.push(("transform".into(), Value::String(Str::new(spelled(c.extra.get(Family::Glyphs, "transform"), &m)))));
    }
    Ok(Value::Dict(finish(e, &c.extra)))
}

fn write_anchor(a: &Anchor) -> Value {
    let e = vec![
        ("name".into(), string(&a.name)),
        ("position".into(), Value::String(Str::new(spelled(a.extra.get(Family::Glyphs, "position"), &[a.x, a.y])))),
    ];
    Value::Dict(finish(e, &a.extra))
}

// ── axes, master names, stems, guides and instances ────────────────────────────────────────────

/// The font's axes: from the `Axes` parameter, or the default axes for as many slots as the
/// masters and instances use.
fn axes_of(params: &Params, masters: &[Value], instances: Option<&Value>) -> Vec<Axis> {
    if let Some(axes) = params.get("Axes").and_then(axes_from_param) {
        return axes;
    }
    let mut used = 0;
    let mut look = |v: &Value, keys: &[&str; SLOTS]| {
        if let Some(d) = v.as_dict() {
            for (i, k) in keys.iter().enumerate() {
                if d.get(k).is_some() {
                    used = used.max(i + 1);
                }
            }
        }
    };
    masters.iter().for_each(|m| look(m, &MASTER_SLOTS));
    if let Some(Value::Array(a)) = instances {
        a.items.iter().for_each(|i| look(i, &INSTANCE_SLOTS));
    }
    default_axes(used)
}

fn default_axes(n: usize) -> Vec<Axis> {
    SLOT_AXES.iter().take(n).map(|(name, tag)| Axis { name: (*name).into(), tag: (*tag).into(), hidden: false, extra: Extra::new() }).collect()
}

/// The axes an `Axes` parameter names (`{Name, Tag?, Hidden?}` each), if it can be read and has no
/// more than version 2 can hold. A missing tag is glyphsLib's stand-in (`XXXX`, then `XXX<i>`).
fn axes_from_param(v: &Value) -> Option<Vec<Axis>> {
    let items = &v.as_array()?.items;
    if items.len() > SLOTS {
        return None;
    }
    items
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut d = a.as_dict()?.clone();
            let name = take(&mut d, "Name")?.as_str()?.to_string();
            let tag = match take(&mut d, "Tag") {
                Some(t) => t.as_str()?.to_string(),
                None if i == 0 => "XXXX".into(),
                None => format!("XXX{i}"),
            };
            let hidden = match take(&mut d, "Hidden") {
                Some(h) => matches!(h.as_num(), Some(Num::Int(1))) || h.as_bool() == Some(true),
                None => false,
            };
            Some(Axis { name, tag, hidden, extra: into_extra(d) })
        })
        .collect()
}

fn axes_param_value(axes: &[Axis]) -> Value {
    let items = axes
        .iter()
        .map(|a| {
            let mut e = vec![("Name".into(), string(&a.name)), ("Tag".into(), string(&a.tag))];
            if a.hidden {
                e.push(("Hidden".into(), int(1)));
            }
            Value::Dict(finish(e, &a.extra))
        })
        .collect();
    list(items)
}

/// The font's parameters as version 2 writes them: the `Axes` parameter is rewritten only when the
/// axes no longer match what it (or, without one, the default axes) says.
fn params_with_axes(font: &Font) -> Result<std::borrow::Cow<'_, Params>> {
    if font.axes.len() > SLOTS {
        return Err(GlyphsError::Unsupported(format!("{} axes in format version 2, which holds {SLOTS}", font.axes.len())));
    }
    let expected = font.info.params.get("Axes").and_then(axes_from_param).unwrap_or_else(|| default_axes(font.axes.len()));
    if *font.axes == expected {
        return Ok(std::borrow::Cow::Borrowed(&font.info.params));
    }
    let mut params = font.info.params.clone();
    if font.axes.is_empty() {
        params.remove("Axes");
    } else {
        let value = axes_param_value(&font.axes);
        match params.0.iter_mut().find(|p| p.name == "Axes" && !p.disabled) {
            Some(p) => p.value = value,
            None => params.0.insert(0, Param::new("Axes", value)),
        }
    }
    Ok(std::borrow::Cow::Owned(params))
}

/// The number of horizontal and vertical stems, when every master has the same numbers and each
/// list reads and writes back the same.
fn stem_layout(masters: &[Value]) -> Option<(usize, usize)> {
    let count = |d: &Dict, key: &str| -> Option<usize> {
        let v = d.get(key)?;
        let mut alone = Dict::new();
        alone.entries.push((key.into(), v.clone()));
        let n = design::take_values(&mut alone, key).len();
        (n > 0).then_some(n)
    };
    let mut layout = None;
    for m in masters {
        let d = m.as_dict()?;
        let one = |key: &str| if d.get(key).is_some() { count(d, key) } else { Some(0) };
        let this = (one("horizontalStems")?, one("verticalStems")?);
        if layout.is_some_and(|l| l != this) {
            return None;
        }
        layout = Some(this);
    }
    layout.filter(|(h, v)| h + v > 0)
}

/// The location in the first `n` slots. A key at its default, or written in another way than
/// the writer would, stays in `d` and is written from there.
fn take_slots(d: &mut Dict, keys: &[&str; SLOTS], n: usize) -> Vec<f64> {
    keys.iter()
        .zip(SLOT_DEFAULTS)
        .take(n)
        .map(|(key, default)| {
            let Some(v) = d.get(key) else { return default };
            let Some(x) = v.as_num().map(Num::as_f64) else { return default };
            if x != default && num(x, key).is_ok_and(|w| same_text(&w, v)) {
                take(d, key);
            }
            x
        })
        .collect()
}

/// [`take_slots`] in reverse: a value is written unless it is the default or the kept key
/// already says it.
fn push_slots(e: &mut Vec<(Str, Value)>, extra: &Extra, keys: &[&str; SLOTS], location: &[f64], n: usize) -> Result<()> {
    for (i, (key, default)) in keys.iter().zip(SLOT_DEFAULTS).take(n).enumerate() {
        let x = location.get(i).copied().unwrap_or(default);
        let write = match extra.get(Family::Glyphs, key) {
            Some(kept) => kept.as_num().map_or(default, Num::as_f64) != x,
            None => x != default,
        };
        if write {
            e.push(((*key).into(), num(x, key)?));
        }
    }
    Ok(())
}

/// The name the reference gives a master that states none: its `Master Name` parameter, or its
/// name parts joined.
fn derived_name(params: &Params, parts: &NameParts, italic: bool) -> String {
    match params.get("Master Name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => parts.joined(italic),
    }
}

/// Whether the model would accept `s` as a master name.
fn usable_name(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_NAME_LEN && !s.chars().any(char::is_control)
}

fn read_guide(v: &Value) -> Option<Guide> {
    let mut d = v.as_dict()?.clone();
    let mut g = Guide::default();
    if let Some(p) = take(&mut d, "position") {
        let n = braced_numbers(p.as_str()?, 2)?;
        (g.x, g.y) = (*n.first()?, *n.get(1)?);
    }
    design::take_guide_rest(&mut d, &mut g)?;
    g.extra = into_extra(d);
    Some(g)
}

fn write_guide(g: &Guide) -> Option<Value> {
    let mut e = Vec::new();
    if (g.x, g.y) != (0.0, 0.0) {
        if !(g.x.is_finite() && g.y.is_finite()) {
            return None;
        }
        e.push(("position".into(), Value::String(Str::new(braced(&[g.x, g.y])))));
    }
    design::push_guide_rest(&mut e, g)?;
    Some(Value::Dict(finish(e, &g.extra)))
}

/// Whether a kept `active` or `exports` key says 0.
fn says_zero(v: Option<&Value>) -> bool {
    v.and_then(Value::as_num).is_some_and(|n| n.as_f64() == 0.0)
}

fn read_instance(v: &Value, axes: usize) -> Option<Instance> {
    let mut d = v.as_dict()?.clone();
    let mut i = Instance { name: take_text(&mut d, "name").unwrap_or_default(), ..Default::default() };
    i.location = take_slots(&mut d, &INSTANCE_SLOTS, axes);
    // Both keys stay as written; the writer adds one only when the field changes.
    i.exports = !(says_zero(d.get("active")) || says_zero(d.get("exports")));
    design::take_instance_fields(&mut d, &mut i);
    i.properties = crate::info::take_properties(&mut d);
    i.params = crate::info::take_params(&mut d);
    i.user_data = crate::info::take_user_data(&mut d);
    i.extra = into_extra(d);
    Some(i)
}

fn write_instance(i: &Instance, axes: usize) -> Result<Value> {
    let mut e = Vec::new();
    if !i.name.is_empty() {
        e.push(("name".into(), string(&i.name)));
    }
    push_slots(&mut e, &i.extra, &INSTANCE_SLOTS, &i.location, axes)?;
    let off: Vec<&str> = ["active", "exports"].into_iter().filter(|k| says_zero(i.extra.get(Family::Glyphs, k))).collect();
    if i.exports {
        e.extend(off.into_iter().map(|k| (k.into(), int(1))));
    } else if off.is_empty() {
        e.push(("exports".into(), int(0)));
    }
    design::push_instance_fields(&mut e, i)?;
    crate::info::push_properties(&mut e, &i.properties);
    crate::info::push_params(&mut e, &i.params);
    crate::info::push_user_data(&mut e, &i.user_data);
    Ok(Value::Dict(finish_ordered(e, &i.extra, INSTANCE_ORDER)))
}
