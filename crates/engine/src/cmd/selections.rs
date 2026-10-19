//! Stored selections: node, anchor and component selections kept under names in the glyph's user
//! data ([`STORED_SELECTIONS`]), so the same parts can be selected again later, on any compatible
//! layer. Rows: `edit.storeSelection`, `edit.removeStoredSelection`, `edit.getStoredSelections`.
//! The shell's `edit.recallSelection` selects what `edit.getStoredSelections` maps onto the edited
//! layer; the reference font (`view.setReferenceFont`) is view state and lives in the shell.
//!
//! An entry is `{name, nodes: [[shape, node]…], anchors: [names], components: [shape indices]}`.
//! Indexes are kept, not positions, so on another master the nodes with the same indexes are
//! selected; entries a file holds in another shape are skipped, never an error.

use fontcraft_model::fc_keys::STORED_SELECTIONS;
use fontcraft_model::plist::{Array, Dict, Num, Number, Str, Value as PValue};
use fontcraft_model::{Font, MAX_NAME_LEN};
use serde_json::{Map, json};

use super::*;
use crate::params::{MAX_TARGETS, Params, Targets};

/// Stored selections per glyph (`docs/plans/M13.md` § 3, limits).
pub const MAX_STORED_SELECTIONS: usize = 256;

const TARGETS: &[TargetKind] = &[TargetKind::Layer, TargetKind::Points, TargetKind::Shapes, TargetKind::Anchors];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.storeSelection",
            "Store Selection",
            ["Edit", "Selection"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", name: string, points?: [[shape, node]], shapes?: [index], anchors?: [name]} keep the given nodes, components and anchors under `name` in the glyph's data, replacing an entry of that name; every address must resolve in the layer and at least one must be given. A glyph keeps at most 256 entries. Refused while another entry the file holds has more than 100000 items in a list: writing the entries back would cut it → {glyph, name, replaced: bool, count: entries, warnings: [text]}",
            has_doc,
            store
        )
        .targets(TARGETS),
        cmd!(
            "edit.removeStoredSelection",
            "Remove Stored Selection",
            [],
            None,
            "{glyph: name, name: string} delete the stored selection called `name` from the glyph's data. Refused while another entry the file holds has more than 100000 items in a list: writing the entries back would cut it → {glyph, name, count: entries left, warnings: [text]}",
            has_doc,
            remove
        ),
        cmd!(
            query "edit.getStoredSelections",
            "Get Stored Selections",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", name?: string (only that entry)} the glyph's stored selections in stored order, each mapped onto the layer by index: what resolves there as target lists, and how many stored items do not → {glyph, layer, selections: [{name, points: [[shape, node]], shapes: [index], anchors: [name], missing: count}], warnings: [text] (entries the file holds with more than 100000 items in a list, listed cut)}",
            has_doc,
            get
        ),
    ]
}

/// One stored selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stored {
    pub name: String,
    pub nodes: Vec<(usize, usize)>,
    pub anchors: Vec<String>,
    pub components: Vec<usize>,
    /// The file held more than [`MAX_TARGETS`] items in one of the lists, and the rest were left
    /// out. Not written back.
    pub truncated: bool,
}

impl Stored {
    fn to_plist(&self) -> PValue {
        let index = |i: usize| PValue::Number(Number::int(i64::try_from(i).unwrap_or(i64::MAX)));
        let array = |items: Vec<PValue>| PValue::Array(Array { items, ..Array::default() });
        let mut d = Dict::new();
        d.insert("name", PValue::String(Str::new(self.name.clone())));
        if !self.nodes.is_empty() {
            d.insert("nodes", array(self.nodes.iter().map(|&(s, n)| array(vec![index(s), index(n)])).collect()));
        }
        if !self.anchors.is_empty() {
            d.insert("anchors", array(self.anchors.iter().map(|a| PValue::String(Str::new(a.clone()))).collect()));
        }
        if !self.components.is_empty() {
            d.insert("components", array(self.components.iter().map(|&s| index(s)).collect()));
        }
        PValue::Dict(d)
    }

    /// An entry as a file holds it; `None` when it has no usable name. Malformed items are left
    /// out, and each list is cut at [`MAX_TARGETS`] (marked in `truncated`).
    fn from_plist(v: &PValue) -> Option<Self> {
        let d = v.as_dict()?;
        let name = d.get("name")?.as_str()?;
        if !usable_name(name) {
            return None;
        }
        let mut truncated = false;
        let mut items = |key: &str| {
            let all = d.get(key).and_then(PValue::as_array).map(|a| a.items.as_slice()).unwrap_or_default();
            truncated |= all.len() > MAX_TARGETS;
            all.iter().take(MAX_TARGETS).collect::<Vec<_>>()
        };
        let nodes = items("nodes")
            .into_iter()
            .filter_map(|p| {
                let pair = p.as_array()?;
                match pair.items.as_slice() {
                    [s, n] => Some((plist_index(s)?, plist_index(n)?)),
                    _ => None,
                }
            })
            .collect();
        let anchors = items("anchors").into_iter().filter_map(|a| a.as_str().filter(|a| usable_name(a)).map(str::to_string)).collect();
        let components = items("components").into_iter().filter_map(plist_index).collect();
        Some(Self { name: name.to_string(), nodes, anchors, components, truncated })
    }
}

/// A whole number of zero or more, written as a number or as text.
fn plist_index(v: &PValue) -> Option<usize> {
    let n = match v {
        PValue::Number(n) => n.value(),
        PValue::String(s) => Num::Int(s.as_str().trim().parse::<i64>().ok()?),
        _ => return None,
    };
    match n {
        Num::Int(i) => usize::try_from(i).ok(),
        Num::Float(f) if f.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&f) => Some(f as usize),
        Num::Float(_) => None,
    }
}

fn usable_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= MAX_NAME_LEN && !name.chars().any(char::is_control)
}

/// The stored selections of `glyph` in `font`, in stored order, at most
/// [`MAX_STORED_SELECTIONS`]. A later entry with a name seen before is left out.
pub fn stored(font: &Font, glyph: &str) -> Vec<Stored> {
    let Some(g) = font.glyph(glyph) else { return Vec::new() };
    let Some(list) = g.info.user_data.get(STORED_SELECTIONS).and_then(PValue::as_array) else { return Vec::new() };
    let mut out: Vec<Stored> = Vec::new();
    for item in &list.items {
        if out.len() >= MAX_STORED_SELECTIONS {
            break;
        }
        if let Some(s) = Stored::from_plist(item)
            && !out.iter().any(|o| o.name == s.name)
        {
            out.push(s);
        }
    }
    out
}

/// What of a stored selection resolves in one side of a layer: the target lists, and the count
/// of stored items that do not resolve there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapped {
    pub points: Vec<(usize, usize)>,
    pub shapes: Vec<usize>,
    pub anchors: Vec<String>,
    pub missing: usize,
}

/// Maps `s` onto `parts` by index (nodes, components) and by name (anchors).
pub fn map_onto(s: &Stored, parts: fontcraft_model::LayerParts) -> Mapped {
    let mut m = Mapped::default();
    for &(shape, node) in &s.nodes {
        if parts.node(shape, node).is_some() {
            m.points.push((shape, node));
        } else {
            m.missing += 1;
        }
    }
    for &shape in &s.components {
        if parts.shapes.get(shape).is_some() {
            m.shapes.push(shape);
        } else {
            m.missing += 1;
        }
    }
    for a in &s.anchors {
        if parts.anchor(a).is_some() {
            m.anchors.push(a.clone());
        } else {
            m.missing += 1;
        }
    }
    m
}

/// One warning for each entry of `list` that the file held longer than [`MAX_TARGETS`] items;
/// `kept` says what happened to the rest ("are listed").
fn truncation_warnings<'a>(list: impl IntoIterator<Item = &'a Stored>, kept: &str) -> Vec<String> {
    list.into_iter()
        .filter(|e| e.truncated)
        .map(|e| format!("stored selection `{}` holds more than {MAX_TARGETS} items in a list; only the first {MAX_TARGETS} {kept}", e.name))
        .collect()
}

/// Refuses to write `list` back while an entry the file held is longer than [`MAX_TARGETS`]
/// items in a list: only the first ones were read, so writing it would lose the rest.
fn refuse_cut(p: &Params<'_>, list: &[Stored]) -> Result<()> {
    match list.iter().find(|e| e.truncated) {
        Some(e) => Err(p.bad(format!(
            "stored selection `{}` holds more than {MAX_TARGETS} items in a list, which cannot be written back; store a smaller selection under its name or remove it first",
            e.name
        ))),
        None => Ok(()),
    }
}

fn selection_name<'a>(p: &Params<'a>) -> Result<&'a str> {
    let name = p.str("name")?;
    if !usable_name(name) {
        return Err(p.bad(format!("`name` must be 1 to {MAX_NAME_LEN} bytes of text without control characters")));
    }
    Ok(name)
}

/// Writes `list` as the glyph's stored selections; an empty list removes the key.
fn write(font: &mut Font, glyph: &str, list: &[Stored]) -> Result<()> {
    let mut g = font.glyph_mut(glyph)?;
    let data = &mut g.info.user_data.0;
    match data.entries.iter().position(|(k, _)| k.as_str() == STORED_SELECTIONS) {
        Some(i) if list.is_empty() => {
            data.remove_at(i);
        }
        _ if list.is_empty() => {}
        _ => data.insert(STORED_SELECTIONS, PValue::Array(Array { items: list.iter().map(Stored::to_plist).collect(), ..Array::default() })),
    }
    g.info.tidy();
    Ok(())
}

fn store(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.storeSelection", v)?;
    let name = selection_name(&p)?.to_string();
    let t = Targets::parse(&p)?;
    if !t.segments.is_empty() || !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("only `points`, `shapes` and `anchors` can be stored"));
    }
    if !t.has_lists() {
        return Err(p.bad("nothing to store: give `points`, `shapes` or `anchors`"));
    }
    let font = s.font()?;
    t.resolve(font)?;
    let mut list = stored(font, &t.glyph);
    let entry = Stored { name: name.clone(), nodes: dedup(&t.points), anchors: dedup(&t.anchors), components: dedup(&t.shapes), truncated: false };
    let replaced = match list.iter_mut().find(|e| e.name == name) {
        Some(e) => {
            *e = entry;
            true
        }
        None => {
            if list.len() >= MAX_STORED_SELECTIONS {
                return Err(p.bad(format!("glyph `{}` already keeps {MAX_STORED_SELECTIONS} stored selections; remove one first", t.glyph)));
            }
            list.push(entry);
            false
        }
    };
    refuse_cut(&p, &list)?;
    let count = list.len();
    let warnings: Vec<String> = Vec::new();
    s.edit("Store Selection", |font| write(font, &t.glyph, &list))?;
    Ok(json!({"glyph": t.glyph, "name": name, "replaced": replaced, "count": count, "warnings": warnings}))
}

/// `items` in their order, each kept once.
fn dedup<T: Clone + Eq + std::hash::Hash>(items: &[T]) -> Vec<T> {
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    items.iter().filter(|item| seen.insert(*item)).cloned().collect()
}

fn remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.removeStoredSelection", v)?;
    let glyph = p.str("glyph")?;
    let name = selection_name(&p)?;
    let font = s.font()?;
    if font.glyph(glyph).is_none() {
        return Err(p.bad(format!("there is no glyph `{glyph}`")));
    }
    let mut list = stored(font, glyph);
    let before = list.len();
    list.retain(|e| e.name != name);
    if list.len() == before {
        return Err(p.bad(format!("glyph `{glyph}` has no stored selection `{name}`")));
    }
    refuse_cut(&p, &list)?;
    let count = list.len();
    let warnings: Vec<String> = Vec::new();
    s.edit("Remove Stored Selection", |font| write(font, glyph, &list))?;
    Ok(json!({"glyph": glyph, "name": name, "count": count, "warnings": warnings}))
}

fn get(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.getStoredSelections", v)?;
    let only = match p.get("name") {
        Some(_) => Some(selection_name(&p)?),
        None => None,
    };
    let t = Targets::parse(&p)?;
    let font = s.font()?;
    let r = t.resolve(font)?;
    let list = stored(font, &t.glyph);
    if let Some(name) = only
        && !list.iter().any(|e| e.name == name)
    {
        return Err(p.bad(format!("glyph `{}` has no stored selection `{name}`", t.glyph)));
    }
    let shown = || list.iter().filter(|e| only.is_none_or(|n| n == e.name));
    let warnings = truncation_warnings(shown(), "are listed");
    let selections: Vec<Value> = shown()
        .map(|e| {
            let m = map_onto(e, r.parts);
            let mut o = Map::new();
            o.insert("name".into(), json!(e.name));
            o.insert("points".into(), Value::Array(m.points.iter().map(|(s, n)| json!([s, n])).collect()));
            o.insert("shapes".into(), json!(m.shapes));
            o.insert("anchors".into(), json!(m.anchors));
            o.insert("missing".into(), json!(m.missing));
            Value::Object(o)
        })
        .collect();
    Ok(json!({"glyph": t.glyph, "layer": r.layer.id, "selections": selections, "warnings": warnings}))
}

#[cfg(test)]
mod tests {
    use super::dedup;

    /// A selection of every target a command takes is kept once each, in order, in linear time.
    #[test]
    fn dedup_keeps_the_first_of_each_in_order_and_scales() {
        assert_eq!(dedup(&[3, 1, 3, 2, 1]), [3, 1, 2]);
        let many: Vec<(usize, usize)> = (0..crate::params::MAX_TARGETS).map(|i| (i / 7, i % 7)).collect();
        assert_eq!(dedup(&many).len(), many.len());
    }
}
