//! Compare commands across two open documents (M9.19).
//!
//! Rows: `compareFonts.useLeft`, `compareFonts.useRight`, `compareFonts.diff`,
//! `compareFamily.report`.
//!
//! `compareFonts.diff` lists, for two masters (of one document or of two), the values that differ
//! in four categories. Each row has a `key` that is stable for the pair of fonts; Use Left and Use
//! Right take the same `{category, key}` and write the value from one master to the other.
//!
//! | Category | Keys |
//! |---|---|
//! | `fontInfo` | `familyName`, `unitsPerEm`, `versionMajor`, `versionMinor`, `date`, `property:<key>`, `parameter:<name>` |
//! | `masterMetrics` | `metric:<kind>[:<name>][\|<filter>]`, `stem:<name>` (`stem:#<n>` when unnamed), `number:<name>` |
//! | `glyph` | the glyph name; compares the master layer's width, shapes and anchors |
//! | `kerning` | `<first> <second>`, the left-to-right pairs of the master |

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, Layer, MAX_GLYPHS, MetricDef, MetricValue, Param, Shape};
use serde_json::json;

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most rows `compareFonts.diff` returns; past it the result says `truncated`.
pub const MAX_DIFF_ROWS: usize = 100_000;
/// Most entries in `compareFamily.report`'s `documents` list.
pub const MAX_REPORT_DOCUMENTS: usize = 4_096;
/// Longest row key Use Left and Use Right accept.
const MAX_KEY_BYTES: usize = 4_096;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "compareFonts.useLeft",
            "Use Left",
            [],
            None,
            "{left?: {document?: index|title|path, master?: id|name}, right?: {document?, master?}, row: {category: fontInfo|masterMetrics|glyph|kerning, key: string}} write the row's value from the left master to the right one, as one undo step in the right document; `key` is the row key compareFonts.diff returns. A glyph row copies the master layer's width, shapes and anchors (adding the glyph when the right font lacks it); a row the left side has no value for removes the value on the right for font info and kerning, and is refused for metrics and glyphs → {changed, document, master, category, key}",
            has_doc,
            use_left
        ),
        cmd!(
            "compareFonts.useRight",
            "Use Right",
            [],
            None,
            "{left?: {document?: index|title|path, master?: id|name}, right?: {document?, master?}, row: {category: fontInfo|masterMetrics|glyph|kerning, key: string}} write the row's value from the right master to the left one, as one undo step in the left document (see compareFonts.useLeft) → {changed, document, master, category, key}",
            has_doc,
            use_right
        ),
        cmd!(
            query "compareFonts.diff",
            "Diff Two Masters",
            [],
            None,
            "{left?: {document?: index|title|path, master?: id|name}, right?: {document?, master?}, categories?: [fontInfo|masterMetrics|glyph|kerning]} the rows whose values differ between two masters of one or two open documents, grouped by category in that order; a value the side lacks is null. Glyph rows summarise each layer and list `differences` (width, shapes, anchors) → {left: {document, title, master}, right: {…}, rows: [{category, key, label, left, right, differences?}], truncated}",
            has_doc,
            diff
        ),
        cmd!(
            query "compareFamily.report",
            "Family Comparison Report",
            [],
            None,
            "{documents?: [index|title|path], attribute: componentNames|anchors|leftKerningGroup|rightKerningGroup|glyphs, filter?: all|missingInOne|ignoreIfMissingInOne, offset?: number, limit?: number} one glyph attribute across open documents (default: all, in the order they were opened), read from each document's first master. A cell is null when the value is empty and {missing: true} when the document has no such glyph. all: glyphs with a value in some document; missingInOne: glyphs absent from at least one document; ignoreIfMissingInOne: glyphs with a value that every document has → {columns: [{document, title, master}], total, rows: [{glyph, cells, same}]}",
            has_doc,
            report
        ),
    ]
}

// ---------- sides ----------

/// One side of a comparison: an open document's position and one of its masters.
struct Side {
    index: usize,
    master: String,
}

fn active_index(s: &Session) -> Result<usize> {
    let id = s.doc()?.id;
    s.position_of(id).ok_or(EngineError::NoDocument)
}

/// An open document named by its position, its title or the path it was opened from.
fn document_index(s: &Session, p: &Params, what: &str, v: &Value) -> Result<usize> {
    let docs = s.documents();
    match v {
        Value::Number(n) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < docs.len())
            .ok_or_else(|| p.bad(format!("`{what}`: there is no open document {n}"))),
        Value::String(name) => docs
            .iter()
            .position(|d| d.title == *name || d.path.as_deref().is_some_and(|path| path == Path::new(name)))
            .or_else(|| super::session::find_open(s, Path::new(name)))
            .ok_or_else(|| p.bad(format!("`{what}`: no open document is called or saved as `{name}`"))),
        _ => Err(p.bad(format!("`{what}` must be an open document's position, title or path"))),
    }
}

fn side(s: &Session, p: &Params, key: &str) -> Result<Side> {
    let obj = p.opt_object(key)?;
    let field = |name: &str| obj.and_then(|o| o.get(name)).filter(|v| !v.is_null());
    let index = match field("document") {
        Some(v) => document_index(s, p, &format!("{key}.document"), v)?,
        None => active_index(s)?,
    };
    let font = &s.documents().get(index).ok_or(EngineError::NoDocument)?.font;
    let master = match field("master") {
        None => font.masters.first().map(|m| m.id.clone()).ok_or_else(|| p.bad(format!("`{key}`: the document has no master")))?,
        Some(Value::String(want)) => font
            .masters
            .iter()
            .find(|m| m.id == *want)
            .or_else(|| font.masters.iter().find(|m| m.name == *want))
            .map(|m| m.id.clone())
            .ok_or_else(|| p.bad(format!("`{key}.master`: the document has no master `{want}`")))?,
        Some(_) => return Err(p.bad(format!("`{key}.master` must be a master id or name"))),
    };
    Ok(Side { index, master })
}

fn side_json(s: &Session, side: &Side) -> Value {
    let title = s.documents().get(side.index).map(|d| d.title.clone()).unwrap_or_default();
    json!({ "document": side.index, "title": title, "master": side.master })
}

fn font_at(s: &Session, index: usize) -> Result<Arc<Font>> {
    s.documents().get(index).map(|d| d.font.clone()).ok_or(EngineError::NoDocument)
}

/// [`Session::edit`] on the document at `index`, whichever is active; the active document stays
/// the same.
fn edit_document<T>(s: &mut Session, index: usize, label: &str, f: impl FnOnce(&mut Font) -> Result<T>) -> Result<T> {
    let active = active_index(s)?;
    s.set_active(index)?;
    let out = s.edit(label, f);
    let restored = s.set_active(active);
    let out = out?;
    restored?;
    Ok(out)
}

// ---------- categories ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Category {
    FontInfo,
    MasterMetrics,
    Glyph,
    Kerning,
}

impl Category {
    const ALL: [Category; 4] = [Category::FontInfo, Category::MasterMetrics, Category::Glyph, Category::Kerning];

    fn name(self) -> &'static str {
        match self {
            Category::FontInfo => "fontInfo",
            Category::MasterMetrics => "masterMetrics",
            Category::Glyph => "glyph",
            Category::Kerning => "kerning",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// A keyed value of one side: key, label for people, value.
type Entry = (String, String, Value);

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| EngineError::Internal(format!("could not describe a value: {e}")))
}

fn font_info_entries(font: &Font) -> Result<Vec<Entry>> {
    let mut out: Vec<Entry> = vec![
        ("familyName".into(), "Family Name".into(), json!(font.family_name)),
        ("unitsPerEm".into(), "Units per Em".into(), json!(font.units_per_em)),
        ("versionMajor".into(), "Version Major".into(), json!(font.version_major)),
        ("versionMinor".into(), "Version Minor".into(), json!(font.version_minor)),
        ("date".into(), "Date".into(), json!(font.info.date)),
    ];
    let mut seen = HashSet::new();
    for prop in &font.info.properties.0 {
        if seen.insert(prop.key.as_str()) {
            out.push((format!("property:{}", prop.key), prop.key.clone(), to_json(&prop.value)?));
        }
    }
    let mut seen = HashSet::new();
    for param in &font.info.params.0 {
        if seen.insert(param.name.as_str()) {
            let all: Vec<&Param> = font.info.params.0.iter().filter(|q| q.name == param.name).collect();
            out.push((format!("parameter:{}", param.name), param.name.clone(), to_json(&all)?));
        }
    }
    Ok(out)
}

fn metric_key(d: &MetricDef) -> String {
    let mut k = format!("metric:{}", d.kind);
    if let Some(n) = &d.name {
        k.push(':');
        k.push_str(n);
    }
    if let Some(f) = &d.filter {
        k.push('|');
        k.push_str(f);
    }
    k
}

fn metric_label(d: &MetricDef) -> String {
    let mut k = d.kind.clone();
    if let Some(n) = &d.name {
        k = format!("{k} {n}");
    }
    if let Some(f) = &d.filter {
        k = format!("{k} ({f})");
    }
    k
}

fn stem_key(name: &str, i: usize) -> String {
    if name.is_empty() { format!("stem:#{i}") } else { format!("stem:{name}") }
}

fn master_metric_entries(font: &Font, master: &str) -> Result<Vec<Entry>> {
    let m = font.master(master)?;
    let mut out = Vec::new();
    for (i, d) in font.metrics.iter().enumerate() {
        let v = m.metric_values.get(i).copied().unwrap_or_default();
        out.push((metric_key(d), metric_label(d), json!({ "pos": v.pos, "over": v.over })));
    }
    for (i, d) in font.stems.iter().enumerate() {
        let label = if d.name.is_empty() { format!("Stem {}", i + 1) } else { d.name.clone() };
        out.push((stem_key(&d.name, i), label, json!(m.stem_values.get(i).copied().unwrap_or(0.0))));
    }
    for (i, d) in font.numbers.iter().enumerate() {
        out.push((format!("number:{}", d.name), d.name.clone(), json!(m.number_values.get(i).copied().unwrap_or(0.0))));
    }
    Ok(out)
}

fn kern_entries(font: &Font, master: &str) -> Vec<Entry> {
    let Some(m) = font.kerning.ltr.0.iter().find(|k| k.master == master) else { return Vec::new() };
    m.firsts
        .iter()
        .flat_map(|f| {
            f.seconds.iter().map(move |(second, v)| {
                let key = format!("{} {second}", f.first);
                (key.clone(), key, json!(v))
            })
        })
        .collect()
}

/// The rows collected so far, up to [`MAX_DIFF_ROWS`].
#[derive(Default)]
struct Rows {
    rows: Vec<Value>,
    truncated: bool,
}

impl Rows {
    /// False once the limit is reached.
    fn push(&mut self, row: Value) -> bool {
        if self.rows.len() >= MAX_DIFF_ROWS {
            self.truncated = true;
            return false;
        }
        self.rows.push(row);
        true
    }
}

/// Rows for the keys whose values differ: the left side's keys in its order, then the keys only
/// the right side has. A key repeated on one side counts once, with its first value.
fn diff_entries(category: Category, left: Vec<Entry>, right: Vec<Entry>, rows: &mut Rows) {
    let mut right_map: HashMap<&str, (&str, &Value)> = HashMap::with_capacity(right.len());
    for (k, label, v) in &right {
        right_map.entry(k.as_str()).or_insert((label.as_str(), v));
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(left.len());
    for (k, label, v) in &left {
        if !seen.insert(k.as_str()) {
            continue;
        }
        let other = right_map.get(k.as_str()).map(|(_, v)| *v);
        if other != Some(v) {
            let row = json!({ "category": category.name(), "key": k, "label": label, "left": v, "right": other });
            if !rows.push(row) {
                return;
            }
        }
    }
    for (k, label, v) in &right {
        if !seen.insert(k.as_str()) {
            continue;
        }
        let row = json!({ "category": category.name(), "key": k, "label": label, "left": Value::Null, "right": v });
        if !rows.push(row) {
            return;
        }
    }
}

fn layer_summary(l: &Layer) -> Value {
    let mut paths = 0usize;
    let mut nodes = 0usize;
    let mut components = Vec::new();
    for shape in &l.shapes {
        match shape {
            Shape::Path(p) => {
                paths += 1;
                nodes += p.nodes.len();
            }
            Shape::Component(c) => components.push(c.glyph.as_str()),
            _ => {}
        }
    }
    let anchors: Vec<Value> = l.anchors.iter().map(|a| json!({ "name": a.name, "x": a.x, "y": a.y })).collect();
    json!({ "width": l.width, "paths": paths, "nodes": nodes, "components": components, "anchors": anchors })
}

fn same_anchors(a: &Layer, b: &Layer) -> bool {
    a.anchors.len() == b.anchors.len() && a.anchors.iter().zip(&b.anchors).all(|(x, y)| x.name == y.name && x.x == y.x && x.y == y.y)
}

fn master_layer<'a>(font: &'a Font, glyph: &str, master: &str) -> Option<&'a Layer> {
    font.glyph(glyph).and_then(|g| g.layer(master))
}

fn diff_glyphs(left: &Font, lm: &str, right: &Font, rm: &str, rows: &mut Rows) {
    let row = |name: &str, l: Option<&Layer>, r: Option<&Layer>| -> Option<Value> {
        let differences: Vec<&str> = match (l, r) {
            (None, None) => return None,
            (Some(a), Some(b)) => {
                let mut d = Vec::new();
                if a.width != b.width {
                    d.push("width");
                }
                if a.shapes != b.shapes {
                    d.push("shapes");
                }
                if !same_anchors(a, b) {
                    d.push("anchors");
                }
                if d.is_empty() {
                    return None;
                }
                d
            }
            _ => vec!["missing"],
        };
        Some(json!({
            "category": Category::Glyph.name(),
            "key": name,
            "label": name,
            "left": l.map_or(Value::Null, layer_summary),
            "right": r.map_or(Value::Null, layer_summary),
            "differences": differences,
        }))
    };
    for g in left.glyphs.iter() {
        if let Some(v) = row(&g.name, g.layer(lm), master_layer(right, &g.name, rm))
            && !rows.push(v)
        {
            return;
        }
    }
    for g in right.glyphs.iter().filter(|g| left.glyph(&g.name).is_none()) {
        if let Some(v) = row(&g.name, None, g.layer(rm))
            && !rows.push(v)
        {
            return;
        }
    }
}

// ---------- compareFonts.diff ----------

fn diff(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("compareFonts.diff", p)?;
    let left = side(s, &p, "left")?;
    let right = side(s, &p, "right")?;
    let categories: Vec<Category> = match p.opt_strings("categories")? {
        None => Category::ALL.to_vec(),
        Some(names) => {
            let mut want = Vec::new();
            for n in names {
                let c = Category::parse(n).ok_or_else(|| p.bad(format!("unknown category `{n}`: use fontInfo, masterMetrics, glyph or kerning")))?;
                if !want.contains(&c) {
                    want.push(c);
                }
            }
            // Always in the window's order.
            Category::ALL.into_iter().filter(|c| want.contains(c)).collect()
        }
    };
    let (lf, rf) = (font_at(s, left.index)?, font_at(s, right.index)?);
    let mut rows = Rows::default();
    for c in categories {
        if rows.truncated {
            break;
        }
        match c {
            Category::FontInfo => diff_entries(c, font_info_entries(&lf)?, font_info_entries(&rf)?, &mut rows),
            Category::MasterMetrics => {
                diff_entries(c, master_metric_entries(&lf, &left.master)?, master_metric_entries(&rf, &right.master)?, &mut rows)
            }
            Category::Glyph => diff_glyphs(&lf, &left.master, &rf, &right.master, &mut rows),
            Category::Kerning => diff_entries(c, kern_entries(&lf, &left.master), kern_entries(&rf, &right.master), &mut rows),
        }
    }
    Ok(json!({
        "left": side_json(s, &left),
        "right": side_json(s, &right),
        "rows": rows.rows,
        "truncated": rows.truncated,
    }))
}

// ---------- Use Left, Use Right ----------

fn use_left(s: &mut Session, p: &Value) -> Result<Value> {
    use_side(s, p, "compareFonts.useLeft", true)
}

fn use_right(s: &mut Session, p: &Value) -> Result<Value> {
    use_side(s, p, "compareFonts.useRight", false)
}

fn use_side(s: &mut Session, p: &Value, command: &'static str, from_left: bool) -> Result<Value> {
    let p = Params::new(command, p)?;
    let left = side(s, &p, "left")?;
    let right = side(s, &p, "right")?;
    let row = p.opt_object("row")?.ok_or_else(|| p.bad("`row` is required: {category, key}"))?;
    let category = row
        .get("category")
        .and_then(Value::as_str)
        .and_then(Category::parse)
        .ok_or_else(|| p.bad("`row.category` must be fontInfo, masterMetrics, glyph or kerning"))?;
    let key = row.get("key").and_then(Value::as_str).ok_or_else(|| p.bad("`row.key` must be a string"))?;
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(p.bad(format!("`row.key` must have 1 to {MAX_KEY_BYTES} bytes")));
    }
    let (from, to) = if from_left { (left, right) } else { (right, left) };
    let source = font_at(s, from.index)?;
    let before = s.documents().get(to.index).map(|d| d.revision);
    let label = if from_left { "Use Left" } else { "Use Right" };
    let names = if from_left { ("left", "right") } else { ("right", "left") };
    edit_document(s, to.index, label, |font| {
        let copy = Transfer { p: &p, source: &source, from: &from.master, to: &to.master, names };
        match category {
            Category::FontInfo => copy.font_info(font, key),
            Category::MasterMetrics => copy.master_metric(font, key),
            Category::Glyph => copy.glyph(font, key),
            Category::Kerning => copy.kerning(font, key),
        }
    })?;
    let changed = s.documents().get(to.index).map(|d| d.revision) != before;
    Ok(json!({ "changed": changed, "document": to.index, "master": to.master, "category": category.name(), "key": key }))
}

/// One Use Left or Use Right: copies a row's value from `source`'s master `from` into the
/// receiving font's master `to`.
struct Transfer<'a> {
    p: &'a Params<'a>,
    source: &'a Font,
    from: &'a str,
    to: &'a str,
    /// The names of the giving and the receiving side, for messages.
    names: (&'static str, &'static str),
}

impl Transfer<'_> {
    fn font_info(&self, font: &mut Font, key: &str) -> Result<()> {
        let src = self.source;
        match key {
            "familyName" => font.family_name.clone_from(&src.family_name),
            "unitsPerEm" => font.units_per_em = src.units_per_em,
            "versionMajor" => font.version_major = src.version_major,
            "versionMinor" => font.version_minor = src.version_minor,
            "date" => Arc::make_mut(&mut font.info).date.clone_from(&src.info.date),
            _ => {
                if let Some(name) = key.strip_prefix("property:") {
                    let props = &mut Arc::make_mut(&mut font.info).properties.0;
                    match src.info.properties.get(name) {
                        Some(prop) => match props.iter_mut().find(|q| q.key == name) {
                            Some(slot) => *slot = prop.clone(),
                            None => props.push(prop.clone()),
                        },
                        None => props.retain(|q| q.key != name),
                    }
                } else if let Some(name) = key.strip_prefix("parameter:") {
                    let given: Vec<Param> = src.info.params.0.iter().filter(|q| q.name == name).cloned().collect();
                    let list = &mut Arc::make_mut(&mut font.info).params.0;
                    // The first entry's place; nothing named `name` comes before it.
                    let at = list.iter().position(|q| q.name == name).unwrap_or(list.len());
                    list.retain(|q| q.name != name);
                    let at = at.min(list.len());
                    list.splice(at..at, given);
                } else {
                    return Err(self.p.bad(format!("`{key}` is not a font info row")));
                }
            }
        }
        Ok(())
    }

    fn master_metric(&self, font: &mut Font, key: &str) -> Result<()> {
        let src = self.source;
        let sm = src.master(self.from)?;
        let (giver, _) = self.names;
        let missing = || self.p.bad(format!("the {giver} font has no `{key}`; there is no value to copy"));
        if key.starts_with("metric:") {
            let si = src.metrics.iter().position(|d| metric_key(d) == key).ok_or_else(missing)?;
            let def = src.metrics.get(si).cloned().ok_or_else(missing)?;
            let value = sm.metric_values.get(si).copied().unwrap_or_default();
            let ti = match font.metrics.iter().position(|d| metric_key(d) == key) {
                Some(i) => i,
                None => {
                    Arc::make_mut(&mut font.metrics).push(def);
                    font.metrics.len().saturating_sub(1)
                }
            };
            let m = self.receiving_master(font)?;
            if m.metric_values.len() <= ti {
                m.metric_values.resize(ti + 1, MetricValue::default());
            }
            if let Some(slot) = m.metric_values.get_mut(ti) {
                *slot = value;
            }
        } else if key.starts_with("stem:") {
            let si = src.stems.iter().enumerate().position(|(i, d)| stem_key(&d.name, i) == key).ok_or_else(missing)?;
            let def = src.stems.get(si).cloned().ok_or_else(missing)?;
            let value = sm.stem_values.get(si).copied().unwrap_or(0.0);
            let ti = match font.stems.iter().enumerate().position(|(i, d)| stem_key(&d.name, i) == key) {
                Some(i) => i,
                None if !def.name.is_empty() => {
                    Arc::make_mut(&mut font.stems).push(def);
                    font.stems.len().saturating_sub(1)
                }
                None => return Err(self.p.bad(format!("the receiving font has no `{key}`; an unnamed stem is not added"))),
            };
            let m = self.receiving_master(font)?;
            if m.stem_values.len() <= ti {
                m.stem_values.resize(ti + 1, 0.0);
            }
            if let Some(slot) = m.stem_values.get_mut(ti) {
                *slot = value;
            }
        } else if let Some(name) = key.strip_prefix("number:") {
            let si = src.numbers.iter().position(|d| d.name == name).ok_or_else(missing)?;
            let def = src.numbers.get(si).cloned().ok_or_else(missing)?;
            let value = sm.number_values.get(si).copied().unwrap_or(0.0);
            let ti = match font.numbers.iter().position(|d| d.name == name) {
                Some(i) => i,
                None => {
                    Arc::make_mut(&mut font.numbers).push(def);
                    font.numbers.len().saturating_sub(1)
                }
            };
            let m = self.receiving_master(font)?;
            if m.number_values.len() <= ti {
                m.number_values.resize(ti + 1, 0.0);
            }
            if let Some(slot) = m.number_values.get_mut(ti) {
                *slot = value;
            }
        } else {
            return Err(self.p.bad(format!("`{key}` is not a master metrics row")));
        }
        Ok(())
    }

    fn receiving_master<'f>(&self, font: &'f mut Font) -> Result<&'f mut fontcraft_model::Master> {
        let to = self.to;
        Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == to).ok_or_else(|| self.p.bad(format!("the receiving font has no master `{to}`")))
    }

    fn glyph(&self, font: &mut Font, name: &str) -> Result<()> {
        let (giver, _) = self.names;
        let src_glyph = self
            .source
            .glyph(name)
            .ok_or_else(|| self.p.bad(format!("the {giver} font has no glyph `{name}`; Use copies values, it does not delete glyphs")))?;
        let layer = src_glyph.layer(self.from).ok_or_else(|| self.p.bad(format!("glyph `{name}` has no layer for the {giver} master")))?;
        if font.glyph(name).is_none() {
            add_glyph_like(font, src_glyph, layer.width)?;
        }
        let to = self.to;
        let mut g = font.glyph_mut(name)?;
        if g.layer(to).is_none() {
            g.layers.push(Layer::new(to, layer.width));
        }
        let target = g.layer_mut(to).ok_or_else(|| EngineError::Internal(format!("glyph `{name}` lost its new layer")))?;
        target.width = layer.width;
        target.shapes.clone_from(&layer.shapes);
        target.anchors.clone_from(&layer.anchors);
        Ok(())
    }

    fn kerning(&self, font: &mut Font, key: &str) -> Result<()> {
        let (first, second) = key
            .split_once(' ')
            .filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .ok_or_else(|| self.p.bad(format!("`{key}` is not a kerning row: give `<first> <second>`")))?;
        let value = self.source.kerning.ltr.get(self.from, first, second);
        let table = &mut Arc::make_mut(&mut font.kerning).ltr;
        match value {
            Some(v) => table.set(self.to, first, second, v),
            None => {
                table.remove(self.to, first, second);
            }
        }
        Ok(())
    }
}

/// Adds a glyph called like `like` to `font` with an empty layer per master, at the end of the glyph order,
/// with the code points of `like` that no glyph of `font` maps yet.
fn add_glyph_like(font: &mut Font, like: &Glyph, width: f64) -> Result<()> {
    let used: HashSet<u32> = font.glyphs.iter().flat_map(|g| g.unicodes.iter().copied()).collect();
    let unicodes: Vec<u32> = like.unicodes.iter().copied().filter(|u| !used.contains(u)).collect();
    font.add_glyph(&like.name, unicodes, width)?;
    Ok(())
}

// ---------- compareFamily.report ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Attribute {
    ComponentNames,
    Anchors,
    LeftKerningGroup,
    RightKerningGroup,
    Glyphs,
}

impl Attribute {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "componentNames" => Attribute::ComponentNames,
            "anchors" => Attribute::Anchors,
            "leftKerningGroup" => Attribute::LeftKerningGroup,
            "rightKerningGroup" => Attribute::RightKerningGroup,
            "glyphs" => Attribute::Glyphs,
            _ => return None,
        })
    }

    /// The cell of a glyph that exists: null when it has no value.
    fn cell(self, g: &Glyph, master: &str) -> Value {
        let layer = g.layer(master);
        let list = |items: Vec<&str>| if items.is_empty() { Value::Null } else { json!(items) };
        let group = |name: &Option<String>| name.as_deref().filter(|n| !n.is_empty()).map_or(Value::Null, |n| json!(n));
        match self {
            Attribute::ComponentNames => list(
                layer
                    .map(|l| {
                        l.shapes
                            .iter()
                            .filter_map(|s| match s {
                                Shape::Component(c) => Some(c.glyph.as_str()),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            Attribute::Anchors => list(layer.map(|l| l.anchors.iter().map(|a| a.name.as_str()).collect()).unwrap_or_default()),
            Attribute::LeftKerningGroup => group(&g.kern_groups.left),
            Attribute::RightKerningGroup => group(&g.kern_groups.right),
            Attribute::Glyphs => {
                let (width, shapes) = layer.map_or((0.0, 0), |l| (l.width, l.shapes.len()));
                json!({ "width": width, "shapes": shapes })
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    MissingInOne,
    IgnoreIfMissingInOne,
}

fn report(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("compareFamily.report", p)?;
    let attribute = p.str("attribute")?;
    let attribute = Attribute::parse(attribute).ok_or_else(|| {
        p.bad(format!("unknown attribute `{attribute}`: use componentNames, anchors, leftKerningGroup, rightKerningGroup or glyphs"))
    })?;
    let filter = match p.opt_str("filter")? {
        None | Some("all") => Filter::All,
        Some("missingInOne") => Filter::MissingInOne,
        Some("ignoreIfMissingInOne") => Filter::IgnoreIfMissingInOne,
        Some(other) => return Err(p.bad(format!("unknown filter `{other}`: use all, missingInOne or ignoreIfMissingInOne"))),
    };
    let offset = p.opt_int("offset", 0, i64::from(u32::MAX))?.and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let max_limit = i64::try_from(MAX_GLYPHS).unwrap_or(i64::MAX);
    let limit = p.opt_int("limit", 0, max_limit)?.and_then(|n| usize::try_from(n).ok()).unwrap_or(MAX_GLYPHS);
    let indices: Vec<usize> = match p.opt_array("documents")? {
        None => (0..s.documents().len()).collect(),
        Some(list) => {
            if list.len() > MAX_REPORT_DOCUMENTS {
                return Err(p.bad(format!("`documents` has {} entries; at most {MAX_REPORT_DOCUMENTS}", list.len())));
            }
            let mut out = Vec::new();
            for v in list {
                let i = document_index(s, &p, "documents", v)?;
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            out
        }
    };
    let mut columns = Vec::with_capacity(indices.len());
    for &i in &indices {
        let doc = s.documents().get(i).ok_or(EngineError::NoDocument)?;
        let master = doc.font.masters.first().map(|m| m.id.clone()).unwrap_or_default();
        columns.push((
            doc.font.clone(),
            master,
            json!({ "document": i, "title": doc.title, "master": doc.font.masters.first().map(|m| m.id.as_str()) }),
        ));
    }
    if columns.is_empty() {
        return Err(EngineError::NoDocument);
    }
    // Glyph names in the first document's order, then the names each later document adds.
    let mut names: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (font, _, _) in &columns {
        for g in font.glyphs.iter() {
            if seen.insert(g.name.as_str()) {
                names.push(g.name.as_str());
            }
        }
    }
    let mut total = 0usize;
    let mut rows = Vec::new();
    for name in names {
        let mut present = 0usize;
        let mut relevant = false;
        let cells: Vec<Value> = columns
            .iter()
            .map(|(font, master, _)| match font.glyph(name) {
                Some(g) => {
                    present += 1;
                    let v = attribute.cell(g, master);
                    relevant |= !v.is_null();
                    v
                }
                None => json!({ "missing": true }),
            })
            .collect();
        let keep = match filter {
            Filter::All => relevant,
            Filter::MissingInOne => present < columns.len(),
            Filter::IgnoreIfMissingInOne => present == columns.len() && relevant,
        };
        if !keep {
            continue;
        }
        total += 1;
        if total > offset && rows.len() < limit {
            let same = cells.windows(2).all(|w| matches!(w, [a, b] if a == b));
            rows.push(json!({ "glyph": name, "cells": cells, "same": same }));
        }
    }
    let columns: Vec<Value> = columns.into_iter().map(|(_, _, c)| c).collect();
    Ok(json!({ "columns": columns, "total": total, "rows": rows }))
}
