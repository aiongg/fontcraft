//! Dependencies, related glyphs and component transform copy. Rows:
//! `glyph.listDependencies`, `palette.relatedGlyphs` (the engine query [`related_glyphs`]),
//! `component.copyTransformation` (`spec/inventory/90-secondary.toml`).
//!
//! - Show Dependencies walks the reverse component index level by level, so a glyph used
//!   through other composites is listed with its depth and the glyph it reaches it through.
//!   Each glyph is visited once, which makes the walk linear and safe on cyclic fonts. The
//!   substitution half needs the compiled substitution graph and answers "not supported yet".
//! - [`related_glyphs`] gathers what the Related Glyphs & Characters Palette section lists: the
//!   alternates, the components in both directions, and the characters in both directions of
//!   Unicode's canonical decompositions. The reverse decomposition table is built once.
//! - Copy Component Transformation writes one component's transform onto other components in
//!   one undo step. An automatically aligned target keeps the position alignment gives it on the
//!   aligned axes; a locked target refuses the whole command.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use fontcraft_components::ComponentIndex;
use fontcraft_components::align::{self, AlignMode};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Component, Font, Layer, Shape, Transform};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::params::{MAX_TARGETS, Params, TargetKind as K, Targets, kind};

const LIST: &str = "glyph.listDependencies";
const COPY: &str = "component.copyTransformation";

/// Most entries one section of [`RelatedGlyphs`] holds.
pub const MAX_RELATED: usize = 1000;

/// What `kind: substitutions` answers until the substitution graph exists.
const SUBSTITUTIONS_UNSUPPORTED: &str = "substitution dependencies are not supported yet: they need the compiled substitution graph";

/// Canonical decompositions end at U+2FA1D (CJK compatibility ideographs supplement); the
/// reverse table scans the code points below this bound once.
const DECOMPOSITION_SCAN_END: u32 = 0x3_0000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "glyph.listDependencies",
            "Show Dependencies",
            ["Glyph"],
            None,
            "{glyph: name, kind?: \"components\" | \"substitutions\" | \"both\" = \"both\"} lists the glyphs that use the glyph as a component, directly (depth 1) or through other composites (depth 2 and more, `via` naming the glyph they contain on the way), nearest first and in font order within a depth. Substitution dependencies are not supported yet: `kind: \"substitutions\"` is refused and `both` answers `substitutions: null` with a note → {glyph, kind, components?: [{glyph, depth, via}], substitutions?: null, note?}",
            has_doc,
            list_dependencies
        )
        .targets(&[K::Layer]),
        cmd!(
            "component.copyTransformation",
            "Copy Component Transformation",
            ["Edit"],
            None,
            "{glyph: name, layer?: id (default the first master's layer), component: index, to: [{glyph?: name (default `glyph`), layer?: id (default the source layer's id), components: [indices]}]} copies the source component's transform (position, scale, rotation, slant) onto every listed component, in one undo step. An automatically aligned target keeps its position on the aligned axes; a locked component or glyph refuses the whole command → {glyph, layer, component, targets: [{glyph, layer, components: [index], positionKept: [index]}]}",
            has_doc,
            copy_transformation
        )
        .targets(&[K::Layer]),
    ]
}

// ── Show Dependencies ───────────────────────────────────────────────────────────────────────────

/// A glyph that uses another as a component.
#[derive(Clone, Debug, PartialEq)]
pub struct Dependent {
    pub glyph: String,
    /// 1 for a direct user; n for a glyph that reaches the glyph through n − 1 other composites.
    pub depth: usize,
    /// The glyph this one has a component of on the shortest way to the glyph.
    pub via: String,
}

/// Every glyph that uses `glyph`, directly or through other composites, nearest first. The
/// glyph itself is left out even when it sits on a cycle.
pub fn component_dependents(index: &ComponentIndex, glyph: &str) -> Vec<Dependent> {
    let mut seen: HashSet<&str> = HashSet::from([glyph]);
    let mut out = Vec::new();
    let mut level: Vec<&str> = vec![glyph];
    let mut depth = 0usize;
    // Every glyph enters `seen` once, so the loop ends after at most as many levels as glyphs.
    while !level.is_empty() {
        depth = depth.saturating_add(1);
        let mut next = Vec::new();
        for base in level {
            for user in index.direct_users(base) {
                if seen.insert(user.as_str()) {
                    next.push(user.as_str());
                    out.push(Dependent { glyph: user.clone(), depth, via: base.to_string() });
                }
            }
        }
        level = next;
    }
    out
}

fn list_dependencies(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(LIST, v)?;
    let font = s.font()?;
    let glyph = p.str("glyph")?;
    if font.glyph(glyph).is_none() {
        return Err(p.bad(format!("there is no glyph `{glyph}`")));
    }
    let kind = p.opt_str("kind")?.unwrap_or("both");
    let (components, substitutions) = match kind {
        "components" => (true, false),
        "substitutions" => (false, true),
        "both" => (true, true),
        other => return Err(p.bad(format!("`kind` must be \"components\", \"substitutions\" or \"both\", got \"{other}\""))),
    };
    if !components {
        return Err(p.bad(SUBSTITUTIONS_UNSUPPORTED));
    }
    let mut out = Map::new();
    out.insert("glyph".into(), json!(glyph));
    out.insert("kind".into(), json!(kind));
    let index = ComponentIndex::new(font);
    let list: Vec<Value> =
        component_dependents(&index, glyph).into_iter().map(|d| json!({"glyph": d.glyph, "depth": d.depth, "via": d.via})).collect();
    out.insert("components".into(), Value::Array(list));
    if substitutions {
        out.insert("substitutions".into(), Value::Null);
        out.insert("note".into(), json!(SUBSTITUTIONS_UNSUPPORTED));
    }
    Ok(Value::Object(out))
}

// ── Related glyphs ──────────────────────────────────────────────────────────────────────────────

/// One character of a decomposition section, with the font's glyph for it if there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct RelatedChar {
    pub code_point: u32,
    /// The first glyph in font order that carries the code point.
    pub glyph: Option<String>,
}

/// What the Related Glyphs & Characters section lists for one glyph; each list holds at most
/// [`MAX_RELATED`] entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelatedGlyphs {
    pub glyph: String,
    /// Glyphs with the same base name (the part before the first dot) and another suffix, in
    /// font order.
    pub alternates: Vec<String>,
    /// The glyphs this glyph's components refer to, on any layer, in first-use order.
    pub components: Vec<String>,
    /// The glyphs with a component of this glyph on any layer, in font order.
    pub used_by: Vec<String>,
    /// The characters the glyph's characters decompose into: the one-level canonical
    /// decomposition first, then what the full decomposition adds.
    pub decomposes_into: Vec<RelatedChar>,
    /// The characters whose one-level or full canonical decomposition contains one of the
    /// glyph's characters, by code point.
    pub decomposed_from: Vec<RelatedChar>,
}

/// The part of a glyph name before the first dot; empty for names that start with one.
fn base_name(name: &str) -> &str {
    name.split('.').next().unwrap_or_default()
}

/// The related glyphs and characters of `glyph`; `None` when the font has no such glyph.
pub fn related_glyphs(font: &Font, data: &GlyphData, glyph: &str) -> Option<RelatedGlyphs> {
    let g = font.glyph(glyph)?;
    let base = base_name(glyph);
    let alternates: Vec<String> = if base.is_empty() {
        Vec::new()
    } else {
        font.glyphs.iter().filter(|o| o.name != glyph && base_name(&o.name) == base).take(MAX_RELATED).map(|o| o.name.clone()).collect()
    };
    let mut components: Vec<String> = Vec::new();
    for c in g.layers.iter().flat_map(|l| l.shapes.iter()).filter_map(as_component) {
        if components.len() >= MAX_RELATED {
            break;
        }
        if !components.contains(&c.glyph) {
            components.push(c.glyph.clone());
        }
    }
    let used_by: Vec<String> = font
        .glyphs
        .iter()
        .filter(|o| o.name != glyph && o.layers.iter().flat_map(|l| l.shapes.iter()).filter_map(as_component).any(|c| c.glyph == glyph))
        .take(MAX_RELATED)
        .map(|o| o.name.clone())
        .collect();

    let mut glyph_of: HashMap<u32, &str> = HashMap::new();
    for o in font.glyphs.iter() {
        for cp in &o.unicodes {
            glyph_of.entry(*cp).or_insert(o.name.as_str());
        }
    }
    let related = |cp: u32| RelatedChar { code_point: cp, glyph: glyph_of.get(&cp).map(|n| n.to_string()) };
    let mut own: Vec<u32> = Vec::new();
    for cp in &g.unicodes {
        if !own.contains(cp) {
            own.push(*cp);
        }
    }
    let mut into: Vec<u32> = Vec::new();
    for cp in &own {
        let one = data.canonical_decomposition(*cp).unwrap_or_default();
        let full = data.full_canonical_decomposition(*cp).unwrap_or_default();
        for part in one.into_iter().chain(full) {
            if !into.contains(&part) && !own.contains(&part) {
                into.push(part);
            }
        }
    }
    let reverse = reverse_decompositions(data);
    let mut from: Vec<u32> = own.iter().filter_map(|cp| reverse.get(cp)).flatten().copied().filter(|cp| !own.contains(cp)).collect();
    from.sort_unstable();
    from.dedup();
    Some(RelatedGlyphs {
        glyph: glyph.to_string(),
        alternates,
        components,
        used_by,
        decomposes_into: into.into_iter().take(MAX_RELATED).map(related).collect(),
        decomposed_from: from.into_iter().take(MAX_RELATED).map(related).collect(),
    })
}

fn as_component(shape: &Shape) -> Option<&Component> {
    match shape {
        Shape::Component(c) => Some(c),
        _ => None,
    }
}

/// Code point → the code points whose one-level or full canonical decomposition contains it,
/// ascending. Unicode's decompositions are part of the built-in data and the same in every
/// glyph database, so the table is built from the first one asked and kept.
fn reverse_decompositions(data: &GlyphData) -> &'static HashMap<u32, Vec<u32>> {
    static TABLE: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: HashMap<u32, Vec<u32>> = HashMap::new();
        for cp in 0..DECOMPOSITION_SCAN_END {
            let Some(one) = data.canonical_decomposition(cp) else { continue };
            let full = data.full_canonical_decomposition(cp).unwrap_or_default();
            for part in one.into_iter().chain(full) {
                let list = table.entry(part).or_default();
                if list.last() != Some(&cp) {
                    list.push(cp);
                }
            }
        }
        table
    })
}

// ── Copy Component Transformation ───────────────────────────────────────────────────────────────

fn component_at(layer: &Layer, i: usize) -> Option<&Component> {
    layer.shapes.get(i).and_then(as_component)
}

/// The translation of a transform: the outermost step of both forms.
fn position(t: &Transform) -> (f64, f64) {
    match *t {
        Transform::Matrix([_, _, _, _, dx, dy]) => (dx, dy),
        Transform::Decomposed { pos, .. } => pos,
    }
}

fn with_position(t: Transform, (x, y): (f64, f64)) -> Transform {
    match t {
        Transform::Matrix([a, b, c, d, _, _]) => Transform::Matrix([a, b, c, d, x, y]),
        Transform::Decomposed { scale, angle, slant, .. } => Transform::Decomposed { pos: (x, y), scale, angle, slant },
    }
}

/// One entry of `to`: the target layer and its components.
struct Target {
    targets: Targets,
    layer: String,
    components: Vec<usize>,
    /// (index, new transform, whether the position stayed for alignment).
    writes: Vec<(usize, Transform, bool)>,
}

fn index_value(p: &Params, key: &str, v: &Value) -> Result<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` must be a whole number from 0, found {v}")))
}

fn opt_entry_str<'a>(p: &Params, n: usize, entry: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>> {
    match entry.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(other) => Err(p.bad(format!("`to[{n}].{key}` must be text, got {}", kind(other)))),
    }
}

fn copy_transformation(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new(COPY, v)?;
    let source = Targets::new(COPY, p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
    let font = s.doc()?.font.clone();
    let r = source.resolve(&font)?;
    let i = index_value(&p, "component", p.get("component").ok_or_else(|| p.bad("`component` is required"))?)?;
    let transform = match r.layer.shapes.get(i) {
        Some(Shape::Component(c)) => c.transform,
        Some(_) => return Err(p.bad(format!("`component`: shape {i} is not a component"))),
        None => return Err(p.bad(format!("`component`: {i} does not resolve: the layer has {} shapes", r.layer.shapes.len()))),
    };
    if !transform.numbers().iter().all(|v| in_range(*v)) {
        return Err(p.bad(format!("the transform of component {i} is out of range")));
    }
    let source_layer = r.layer.id.clone();
    let to = p.array("to")?;
    if to.is_empty() {
        return Err(p.bad("`to` is empty"));
    }
    if to.len() > MAX_TARGETS {
        return Err(p.bad(format!("`to` holds {} entries; at most {MAX_TARGETS} are allowed", to.len())));
    }
    let mut total = 0usize;
    let mut plan: Vec<Target> = Vec::with_capacity(to.len());
    for (n, entry) in to.iter().enumerate() {
        let Value::Object(entry) = entry else {
            return Err(p.bad(format!("`to[{n}]` must be an object, got {}", kind(entry))));
        };
        let glyph = opt_entry_str(&p, n, entry, "glyph")?.unwrap_or(source.glyph.as_str());
        let layer = opt_entry_str(&p, n, entry, "layer")?.unwrap_or(source_layer.as_str());
        let targets = Targets::new(COPY, glyph, Some(layer.to_string()));
        let rt = targets.resolve(&font)?;
        crate::check_unlocked(rt.glyph)?;
        let Some(Value::Array(list)) = entry.get("components") else {
            return Err(p.bad(format!("`to[{n}].components` must be a list of shape indices")));
        };
        if list.is_empty() {
            return Err(p.bad(format!("`to[{n}].components` is empty")));
        }
        total = total.saturating_add(list.len());
        if total > MAX_TARGETS {
            return Err(p.bad(format!("`to` names more than {MAX_TARGETS} components")));
        }
        let key = format!("to[{n}].components");
        let mut components: Vec<usize> = Vec::with_capacity(list.len());
        let mut writes = Vec::with_capacity(list.len());
        for item in list {
            let t = index_value(&p, &key, item)?;
            if components.contains(&t) {
                continue;
            }
            let c =
                component_at(rt.layer, t).ok_or_else(|| p.bad(format!("`{key}`: shape {t} of `{glyph}` on layer `{layer}` is not a component")))?;
            if c.info.locked {
                return Err(p.bad(format!("component {t} of `{glyph}` is locked: unlock it to change its transform")));
            }
            let (free_x, free_y) = match align::component_alignment(&font, rt.layer, t) {
                Some(AlignMode::Full) => (false, false),
                Some(AlignMode::Horizontal) => (false, true),
                None => (true, true),
            };
            let (sx, sy) = position(&transform);
            let (cx, cy) = position(&c.transform);
            let pos = (if free_x { sx } else { cx }, if free_y { sy } else { cy });
            let new = with_position(transform, pos);
            if !new.numbers().iter().all(|v| in_range(*v)) || !new.affine().as_coeffs().iter().all(|v| in_range(*v)) {
                return Err(p.bad(format!("the transform copied onto component {t} of `{glyph}` is out of range")));
            }
            components.push(t);
            writes.push((t, new, !(free_x && free_y)));
        }
        let layer = rt.layer.id.clone();
        match plan.iter_mut().find(|e| e.targets.glyph == targets.glyph && e.layer == layer) {
            Some(e) => {
                for w in writes {
                    if !e.components.contains(&w.0) {
                        e.components.push(w.0);
                        e.writes.push(w);
                    }
                }
            }
            None => plan.push(Target { targets, layer, components, writes }),
        }
    }
    s.edit("Copy Component Transformation", |f| {
        for e in &plan {
            e.targets.edit_layer(f, |layer| {
                for (t, new, _) in &e.writes {
                    if let Some(Shape::Component(c)) = layer.shapes.get_mut(*t) {
                        c.transform = *new;
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    })?;
    let targets: Vec<Value> = plan
        .iter()
        .map(|e| {
            let kept: Vec<usize> = e.writes.iter().filter(|w| w.2).map(|w| w.0).collect();
            json!({"glyph": e.targets.glyph, "layer": e.layer, "components": e.components, "positionKept": kept})
        })
        .collect();
    Ok(json!({"glyph": source.glyph, "layer": source_layer, "component": i, "targets": targets}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Glyph, Path};

    use super::*;

    fn run(s: &mut Session, id: &str, p: Value) -> Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    fn glyph(name: &str, cp: Option<u32>, parts: &[&str]) -> Glyph {
        let mut g = Glyph::new(name);
        g.unicodes.extend(cp);
        let mut l = Layer::new("m01", 600.0);
        for part in parts {
            l.shapes.push(Shape::Component(Component::new(*part, 0.0, 0.0)));
        }
        g.layers.push(l);
        g
    }

    /// The starter font (A–Z, a–z) plus `acutecomb`, `Aacute` (A + acutecomb), `Aacute.ss01`
    /// (a component of Aacute), the alternates `A.sc` and `A.ss01`, and `X` and `Y` using each
    /// other.
    fn session() -> Session {
        let mut s = Session::with_document();
        s.edit("fixture", |f| {
            f.glyphs.push(glyph("acutecomb", Some(0x301), &[]));
            f.glyphs.push(glyph("Aacute", Some(0xC1), &["A", "acutecomb"]));
            f.glyphs.push(glyph("Aacute.ss01", None, &["Aacute"]));
            f.glyphs.push(glyph("A.sc", None, &[]));
            f.glyphs.push(glyph("A.ss01", None, &[]));
            f.glyphs.push(glyph("Xc", None, &["Yc"]));
            f.glyphs.push(glyph("Yc", None, &["Xc"]));
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn list_dependencies_follows_nested_components_nearest_first() {
        let mut s = session();
        let out = run(&mut s, LIST, json!({"glyph": "A"}));
        assert_eq!(
            out["components"],
            json!([
                {"glyph": "Aacute", "depth": 1, "via": "A"},
                {"glyph": "Aacute.ss01", "depth": 2, "via": "Aacute"},
            ])
        );
        assert_eq!(out["substitutions"], Value::Null, "both answers the component half");
        assert!(out["note"].as_str().unwrap().contains("not supported yet"));
        let only = run(&mut s, LIST, json!({"glyph": "acutecomb", "kind": "components"}));
        assert_eq!(only["components"].as_array().unwrap().len(), 2);
        assert!(only.get("note").is_none());
        assert_eq!(run(&mut s, LIST, json!({"glyph": "Aacute.ss01"}))["components"], json!([]));
    }

    #[test]
    fn list_dependencies_ends_on_a_cycle_and_refuses_substitutions_and_unknown_glyphs() {
        let mut s = session();
        let out = run(&mut s, LIST, json!({"glyph": "Xc", "kind": "components"}));
        assert_eq!(out["components"], json!([{"glyph": "Yc", "depth": 1, "via": "Xc"}]), "the glyph itself is left out");
        let e = s.execute(LIST, &json!({"glyph": "A", "kind": "substitutions"})).unwrap_err().to_string();
        assert!(e.contains("not supported yet"), "{e}");
        assert!(s.execute(LIST, &json!({"glyph": "A", "kind": "ligatures"})).is_err());
        assert!(s.execute(LIST, &json!({"glyph": "nosuchglyph"})).is_err());
    }

    #[test]
    fn related_glyphs_fills_the_five_sections() {
        let s = session();
        let data = s.glyph_data().unwrap();
        let font = s.font().unwrap();
        let a = related_glyphs(font, &data, "A").unwrap();
        assert_eq!(a.alternates, ["A.sc", "A.ss01"]);
        assert!(a.components.is_empty());
        assert_eq!(a.used_by, ["Aacute"], "direct users only");
        assert!(a.decomposes_into.is_empty());
        assert!(a.decomposed_from.contains(&RelatedChar { code_point: 0xC1, glyph: Some("Aacute".into()) }));
        assert!(a.decomposed_from.contains(&RelatedChar { code_point: 0xC0, glyph: None }), "characters the font lacks are listed");
        // U+1EA4 decomposes to U+00C2 U+0301, and fully to A, circumflex, acute.
        assert!(a.decomposed_from.iter().any(|c| c.code_point == 0x1EA4));

        let aacute = related_glyphs(font, &data, "Aacute").unwrap();
        assert_eq!(aacute.alternates, ["Aacute.ss01"]);
        assert_eq!(aacute.components, ["A", "acutecomb"]);
        assert_eq!(aacute.used_by, ["Aacute.ss01"]);
        assert_eq!(
            aacute.decomposes_into,
            [RelatedChar { code_point: 0x41, glyph: Some("A".into()) }, RelatedChar { code_point: 0x301, glyph: Some("acutecomb".into()) }]
        );
        let acute = related_glyphs(font, &data, "acutecomb").unwrap();
        assert!(acute.decomposed_from.iter().any(|c| c.code_point == 0xC1 && c.glyph.as_deref() == Some("Aacute")));
        assert!(related_glyphs(font, &data, "nosuchglyph").is_none());
    }

    /// `B` and `C` each hold a path, a component of `A` and one of `E`, placed by hand; `B`'s
    /// first component carries a full transform.
    fn manual() -> Session {
        let mut s = Session::with_document();
        s.edit("manual", |f| {
            for name in ["B", "C"] {
                let mut g = f.glyph_mut(name)?;
                let l = &mut g.layers[0];
                l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
                l.shapes.push(Shape::Component(Component::new("A", 10.0, 20.0)));
                l.shapes.push(Shape::Component(Component::new("E", 30.0, 40.0)));
            }
            let mut b = f.glyph_mut("B")?;
            if let Shape::Component(c) = &mut b.layers[0].shapes[1] {
                c.transform = Transform::Decomposed { pos: (50.0, 60.0), scale: (0.8, 0.9), angle: 10.0, slant: (5.0, 0.0) };
            }
            Ok(())
        })
        .unwrap();
        s
    }

    fn transform(s: &Session, glyph: &str, i: usize) -> Transform {
        match &s.font().unwrap().glyph(glyph).unwrap().layers[0].shapes[i] {
            Shape::Component(c) => c.transform,
            _ => panic!("shape {i} of {glyph} is not a component"),
        }
    }

    #[test]
    fn copy_transformation_writes_every_target_in_one_undo_step() {
        let mut s = manual();
        let before = s.doc().unwrap().font.clone();
        let source = transform(&s, "B", 1);
        let out = run(&mut s, COPY, json!({"glyph": "B", "component": 1, "to": [{"glyph": "C", "components": [1, 2]}, {"components": [2]}]}));
        assert_eq!(transform(&s, "C", 1), source);
        assert_eq!(transform(&s, "C", 2), source);
        assert_eq!(transform(&s, "B", 2), source, "`glyph` defaults to the source glyph");
        assert_eq!(out["targets"][0], json!({"glyph": "C", "layer": "m01", "components": [1, 2], "positionKept": []}));
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "one undo step restores the font");
    }

    #[test]
    fn copy_transformation_refuses_locked_and_wrong_targets_and_changes_nothing() {
        let mut s = manual();
        s.edit("lock", |f| {
            if let Shape::Component(c) = &mut f.glyph_mut("C")?.layers[0].shapes[2] {
                c.info.locked = true;
            }
            Ok(())
        })
        .unwrap();
        let before = s.doc().unwrap().font.clone();
        for p in [
            json!({"glyph": "B", "component": 1, "to": [{"components": [2]}, {"glyph": "C", "components": [2]}]}),
            json!({"glyph": "B", "component": 1, "to": [{"glyph": "C", "components": [0]}]}),
            json!({"glyph": "B", "component": 0, "to": [{"glyph": "C", "components": [1]}]}),
            json!({"glyph": "B", "component": 1, "to": []}),
            json!({"glyph": "B", "component": 1, "to": [{"glyph": "C", "layer": "nolayer", "components": [1]}]}),
        ] {
            assert!(s.execute(COPY, &p).is_err(), "{p}");
            assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "a refused copy changes nothing: {p}");
        }
    }

    #[test]
    fn copy_transformation_keeps_the_position_of_an_aligned_target() {
        let mut s = manual();
        s.edit("composite", |f| {
            f.glyphs.push(glyph("Acomp", None, &["A"]));
            Ok(())
        })
        .unwrap();
        let out = run(&mut s, COPY, json!({"glyph": "B", "component": 1, "to": [{"glyph": "Acomp", "components": [0]}]}));
        assert_eq!(out["targets"][0]["positionKept"], json!([0]));
        let Transform::Decomposed { scale, angle, .. } = transform(&s, "Acomp", 0) else { panic!("a decomposed transform") };
        assert_eq!((scale, angle), ((0.8, 0.9), 10.0));
    }
}
