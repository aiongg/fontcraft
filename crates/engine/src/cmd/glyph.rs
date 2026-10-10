//! Glyph menu: adding and removing glyphs, code points, locking.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_geom::{Affine, Point};
use fontcraft_model::{Font, KernTable, Layer, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, MetricsKeys, Shape, Transform, side_group};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, kind};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.newGlyph",
            "New Glyph",
            ["Glyph"],
            Some("Cmd+Alt+Shift+N"),
            "{name?: string, unicodes?: [hex | number], width?: number} add an empty glyph with a layer for every master, at the place the default sort order gives it among the glyphs already there (the others keep their order). name: at most 255 bytes, no control characters; defaults to newGlyph (newGlyph.001, .002… when taken). unicodes: default the code point the glyph data gives the name. width: 0 to 65535, default 600 → {name, index, unicodes}",
            has_doc,
            new_glyph
        ),
        cmd!(
            "glyph.removeGlyph",
            "Remove Glyph",
            ["Glyph"],
            Some("Cmd+Delete"),
            "{glyphs: [names], components?: \"remove\" | \"decompose\", metricsKeys?: \"dissolve\" | \"keep\", confirm?: bool (default true)} removes glyphs from the font. When glyphs that stay use a removed glyph as a component, `components` must say what happens to those components: remove deletes them, decompose turns them into outlines first. When metrics keys of glyphs that stay point at a removed glyph, `metricsKeys` must say whether those keys are dissolved or kept. Without a needed answer the command refuses and names it. Kerning pairs with a removed glyph on either side go with it, in every master and direction; pairs on a kerning group and contextual kerning stay. confirm: false changes nothing and reports what the answers would cover → {removed: [names], usedAsComponent: [{glyph, layer, component}], metricsKeys: [{glyph, layer?, key, value}], kerningPairs: number, needs: [\"components\" | \"metricsKeys\"]}",
            has_doc,
            remove_glyph
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.setUnicodes",
            "Set Unicode Values",
            [],
            None,
            "{glyphs: [names], unicodes: [hex | number]} replace the code points of each glyph (hex text such as \"0041\" or \"U+0041\", or a number); an empty list unencodes it → {glyphs: [names]}",
            has_doc,
            set_unicodes
        ),
        cmd!(
            "glyph.toggleLocked",
            "Locked",
            [],
            None,
            "{glyphs: [names] | glyph: name, locked?: bool (default: lock, unless every glyph is locked already)} locks or unlocks whole glyphs; a locked glyph refuses every change to its drawing → {glyphs, locked}",
            has_doc,
            toggle_locked
        )
        .targets(&[TargetKind::Glyphs])
        .context(&["canvas"]),
    ]
}

fn new_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.newGlyph", p)?;
    let width = p.opt_f64("width")?.unwrap_or(600.0);
    if !(0.0..=65_535.0).contains(&width) {
        return Err(p.bad(format!("`width` must be between 0 and 65535, got {width}")));
    }
    let codes = super::common::code_points(&p, "unicodes")?;
    let name = match p.opt_str("name")? {
        Some(name) => name.to_string(),
        None => {
            let font = s.font()?;
            let mut name = "newGlyph".to_string();
            let mut n = 0u32;
            while font.glyph(&name).is_some() {
                n += 1;
                name = format!("newGlyph.{n:03}");
            }
            name
        }
    };
    let data = s.glyph_data()?;
    let codes = match codes {
        Some(codes) => codes,
        None => fontcraft_model::data_props(&name, &[], &data, false).data_unicode.into_iter().collect(),
    };
    let index = s.edit("New Glyph", |font| {
        let mut probe = fontcraft_model::Glyph::new(name.as_str());
        probe.unicodes = codes.clone();
        let index = fontcraft_model::insertion_index(font, &data, &probe);
        font.add_glyph_at(index, &name, codes.clone(), width)?;
        Ok(index)
    })?;
    Ok(json!({ "name": name, "index": index, "unicodes": codes.iter().map(|c| format!("{c:04X}")).collect::<Vec<_>>() }))
}

/// A component, in a glyph that stays, pointing at a glyph being removed.
struct ComponentUse {
    glyph: String,
    layer: String,
    component: String,
}

/// A metrics key, in a glyph that stays, pointing at a glyph being removed.
struct KeyUse {
    glyph: String,
    /// `None` for the glyph's own keys, else the layer's.
    layer: Option<String>,
    key: &'static str,
    value: String,
}

/// The glyph a metrics key takes its value from: `=H`, `=|H`, `H`, `=O-4` and `=a-cy+10` name
/// `H`, `O` and `a-cy`. A name may contain `-`, so the longest leading run of name characters
/// that is a glyph of `font` wins.
fn key_glyph<'v>(value: &'v str, font: &Font) -> Option<&'v str> {
    let v = value.trim();
    let v = v.strip_prefix('=').unwrap_or(v).trim_start();
    let v = v.strip_prefix('|').unwrap_or(v).trim_start();
    let end = v.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))).unwrap_or(v.len());
    let mut candidate = v.get(..end)?;
    loop {
        if candidate.is_empty() {
            return None;
        }
        if font.glyph(candidate).is_some() {
            return Some(candidate);
        }
        let cut = candidate.rfind('-')?;
        candidate = candidate.get(..cut)?;
    }
}

fn clear_key(keys: &mut MetricsKeys, key: &str) {
    let slot = match key {
        "left" => &mut keys.left,
        "right" => &mut keys.right,
        "width" => &mut keys.width,
        "top" => &mut keys.top,
        "bottom" => &mut keys.bottom,
        "vertWidth" => &mut keys.vert_width,
        "vertOrigin" => &mut keys.vert_origin,
        _ => return,
    };
    *slot = None;
}

/// The components and metrics keys of the glyphs that stay that point at `doomed` glyphs.
fn references(font: &Font, doomed: &HashSet<&str>) -> (Vec<ComponentUse>, Vec<KeyUse>) {
    let (mut uses, mut keys) = (Vec::new(), Vec::new());
    let points_at_doomed = |v: &Option<String>| v.as_deref().and_then(|v| key_glyph(v, font)).is_some_and(|t| doomed.contains(t));
    for g in font.glyphs.iter().filter(|g| !doomed.contains(g.name.as_str())) {
        for (key, v) in g.info.metrics_keys.all() {
            if points_at_doomed(v) {
                keys.push(KeyUse { glyph: g.name.clone(), layer: None, key, value: v.clone().unwrap_or_default() });
            }
        }
        for l in &g.layers {
            for (key, v) in l.info.metrics_keys.all() {
                if points_at_doomed(v) {
                    keys.push(KeyUse { glyph: g.name.clone(), layer: Some(l.id.clone()), key, value: v.clone().unwrap_or_default() });
                }
            }
            let background = l.info.background.as_ref().map(|b| b.shapes.as_slice()).unwrap_or_default();
            let mut seen: HashSet<&str> = HashSet::new();
            for shape in l.shapes.iter().chain(background) {
                if let Shape::Component(c) = shape
                    && doomed.contains(c.glyph.as_str())
                    && seen.insert(c.glyph.as_str())
                {
                    uses.push(ComponentUse { glyph: g.name.clone(), layer: l.id.clone(), component: c.glyph.clone() });
                }
            }
        }
    }
    (uses, keys)
}

/// Adds to `out` the shapes of `base`'s layer that matches `layer` (the same id, else the
/// master layer), moved by `affine` (a path under a mirroring `affine` is reversed). Components
/// of other removed glyphs are decomposed too; components of glyphs that stay are kept, with the
/// transforms combined.
#[allow(clippy::too_many_arguments)]
fn expand(
    font: &Font,
    doomed: &HashSet<&str>,
    base: &str,
    layer: &Layer,
    affine: Affine,
    depth: usize,
    budget: &mut usize,
    out: &mut Vec<Shape>,
) -> Result<()> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(fontcraft_model::ModelError::ComponentCycle(base.to_string()).into());
    }
    let Some(g) = font.glyph(base) else { return Ok(()) };
    let Some(from) = g.layers.iter().find(|l| l.id == layer.id).or_else(|| g.layer(&layer.master)) else { return Ok(()) };
    for shape in &from.shapes {
        match shape {
            Shape::Path(path) => {
                *budget = budget.checked_sub(path.nodes.len()).ok_or_else(|| crate::EngineError::BadParams {
                    command: "glyph.removeGlyph".into(),
                    message: format!("decomposing would add more than {MAX_OUTLINE_ELEMENTS} nodes"),
                })?;
                let mut path = path.clone();
                for n in &mut path.nodes {
                    let at = affine * Point::new(n.x, n.y);
                    n.x = at.x;
                    n.y = at.y;
                }
                // A mirroring transform would turn the contour the other way; export
                // decomposition reverses it too, so the outline keeps its direction.
                if affine.determinant() < 0.0 {
                    path.reverse();
                }
                out.push(Shape::Path(path));
            }
            Shape::Component(c) if doomed.contains(c.glyph.as_str()) => {
                expand(font, doomed, &c.glyph, layer, affine * c.affine(), depth + 1, budget, out)?;
            }
            Shape::Component(c) => {
                let mut c = c.clone();
                c.transform = Transform::Matrix((affine * c.affine()).as_coeffs());
                out.push(Shape::Component(c));
            }
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    Ok(())
}

/// `shapes` without the components of `doomed` glyphs, which are decomposed into their shapes
/// with `decompose`; `None` when there are none.
fn without_doomed(
    font: &Font,
    doomed: &HashSet<&str>,
    shapes: &[Shape],
    layer: &Layer,
    decompose: bool,
    budget: &mut usize,
) -> Result<Option<Vec<Shape>>> {
    if !shapes.iter().any(|s| matches!(s, Shape::Component(c) if doomed.contains(c.glyph.as_str()))) {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(shapes.len());
    for shape in shapes {
        match shape {
            Shape::Component(c) if doomed.contains(c.glyph.as_str()) => {
                if decompose {
                    expand(font, doomed, &c.glyph, layer, c.affine(), 1, budget, &mut out)?;
                }
            }
            other => out.push(other.clone()),
        }
    }
    Ok(Some(out))
}

/// Up to five names, then how many more.
fn some_names(names: &[&str]) -> String {
    let shown: Vec<String> = names.iter().take(5).map(|n| format!("`{n}`")).collect();
    match names.len().saturating_sub(5) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

/// A layer's new foreground and background shapes.
type Rewrite = (String, String, Option<Vec<Shape>>, Option<Vec<Shape>>);

fn remove_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.removeGlyph", p)?;
    let font = s.font()?;
    let names = p.opt_glyphs("glyphs", font)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let decompose = match p.opt_str("components")? {
        None => None,
        Some("remove") => Some(false),
        Some("decompose") => Some(true),
        Some(other) => return Err(p.bad(format!("`components` must be \"remove\" or \"decompose\", got \"{other}\""))),
    };
    let dissolve = match p.opt_str("metricsKeys")? {
        None => None,
        Some("dissolve") => Some(true),
        Some("keep") => Some(false),
        Some(other) => return Err(p.bad(format!("`metricsKeys` must be \"dissolve\" or \"keep\", got \"{other}\""))),
    };
    let confirm = p.bool_or("confirm", true)?;
    let doomed: HashSet<&str> = names.iter().map(String::as_str).collect();
    let (uses, keys) = references(font, &doomed);
    let mut needs = Vec::new();
    if !uses.is_empty() && decompose.is_none() {
        needs.push("components");
    }
    if !keys.is_empty() && dissolve.is_none() {
        needs.push("metricsKeys");
    }
    let uses_json: Vec<Value> = uses.iter().map(|u| json!({"glyph": u.glyph, "layer": u.layer, "component": u.component})).collect();
    let keys_json: Vec<Value> = keys
        .iter()
        .map(|k| match &k.layer {
            Some(layer) => json!({"glyph": k.glyph, "layer": layer, "key": k.key, "value": k.value}),
            None => json!({"glyph": k.glyph, "key": k.key, "value": k.value}),
        })
        .collect();
    let pairs = [&font.kerning.ltr, &font.kerning.rtl, &font.kerning.vertical].iter().map(|t| pairs_naming(t, &doomed)).sum::<usize>();
    if !confirm {
        return Ok(json!({"removed": [], "usedAsComponent": uses_json, "metricsKeys": keys_json, "kerningPairs": pairs, "needs": needs}));
    }
    if !needs.is_empty() {
        let mut why = Vec::new();
        if needs.contains(&"components") {
            let mut users: Vec<&str> = uses.iter().map(|u| u.glyph.as_str()).collect();
            users.dedup();
            why.push(format!(
                "{} use a removed glyph as a component: give `components`: \"remove\" to delete those components, or \"decompose\" to turn them into outlines first",
                some_names(&users)
            ));
        }
        if needs.contains(&"metricsKeys") {
            let mut users: Vec<&str> = keys.iter().map(|k| k.glyph.as_str()).collect();
            users.dedup();
            why.push(format!(
                "metrics keys of {} point at a removed glyph: give `metricsKeys`: \"dissolve\" to clear those keys, or \"keep\" to leave them as they are",
                some_names(&users)
            ));
        }
        return Err(p.bad(why.join("; ")));
    }

    let mut rewrites: Vec<Rewrite> = Vec::new();
    if let Some(decompose) = decompose {
        let mut budget = MAX_OUTLINE_ELEMENTS;
        let mut done: HashSet<(&str, &str)> = HashSet::new();
        for u in &uses {
            if !done.insert((u.glyph.as_str(), u.layer.as_str())) {
                continue;
            }
            let Some(layer) = font.glyph(&u.glyph).and_then(|g| g.layers.iter().find(|l| l.id == u.layer)) else { continue };
            let fg = without_doomed(font, &doomed, &layer.shapes, layer, decompose, &mut budget)?;
            let bg = match layer.info.background.as_ref() {
                Some(b) => without_doomed(font, &doomed, &b.shapes, layer, decompose, &mut budget)?,
                None => None,
            };
            rewrites.push((u.glyph.clone(), u.layer.clone(), fg, bg));
        }
    }
    let dissolving: Vec<&KeyUse> = if dissolve == Some(true) { keys.iter().collect() } else { Vec::new() };
    s.edit("Remove Glyph", |font| {
        for (glyph, layer_id, fg, bg) in rewrites {
            let mut g = font.glyph_mut(&glyph)?;
            let Some(layer) = g.layers.iter_mut().find(|l| l.id == layer_id) else { continue };
            if let Some(fg) = fg {
                layer.shapes = fg;
            }
            if let Some(bg) = bg
                && let Some(b) = layer.info.background.as_mut()
            {
                b.shapes = bg;
            }
        }
        for k in &dissolving {
            let mut g = font.glyph_mut(&k.glyph)?;
            match &k.layer {
                None => clear_key(&mut g.info.metrics_keys, k.key),
                Some(id) => {
                    if let Some(layer) = g.layers.iter_mut().find(|l| l.id == *id) {
                        clear_key(&mut layer.info.metrics_keys, k.key);
                    }
                }
            }
        }
        if pairs > 0 {
            let k = Arc::make_mut(&mut font.kerning);
            for table in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
                remove_pairs_naming(table, &doomed);
            }
        }
        for name in &names {
            font.remove_glyph(name)?;
        }
        Ok(())
    })?;
    Ok(json!({"removed": names, "usedAsComponent": uses_json, "metricsKeys": keys_json, "kerningPairs": pairs, "needs": []}))
}

/// Whether a kerning side is one of the `doomed` glyphs; a group side never is.
fn names_doomed(side: &str, doomed: &HashSet<&str>) -> bool {
    side_group(side).is_none() && doomed.contains(side)
}

/// The pairs, over all masters, with a `doomed` glyph on either side.
fn pairs_naming(table: &KernTable, doomed: &HashSet<&str>) -> usize {
    let firsts = table.0.iter().flat_map(|m| &m.firsts);
    firsts
        .map(|f| if names_doomed(&f.first, doomed) { f.seconds.len() } else { f.seconds.iter().filter(|(s, _)| names_doomed(s, doomed)).count() })
        .sum()
}

/// Removes the pairs with a `doomed` glyph on either side. First sides and masters this empties
/// go with them; entries that were empty before stay as they are.
fn remove_pairs_naming(table: &mut KernTable, doomed: &HashSet<&str>) {
    table.0.retain_mut(|m| {
        let had_pairs = m.firsts.iter().any(|f| !f.seconds.is_empty());
        m.firsts.retain_mut(|f| {
            let had = f.seconds.len();
            if names_doomed(&f.first, doomed) {
                f.seconds.clear();
            } else {
                f.seconds.retain(|(s, _)| !names_doomed(s, doomed));
            }
            had == 0 || !f.seconds.is_empty()
        });
        !had_pairs || m.firsts.iter().any(|f| !f.seconds.is_empty())
    });
}

fn set_unicodes(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.setUnicodes", p)?;
    let names: Vec<String> = p.strings("glyphs")?.into_iter().map(str::to_string).collect();
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let codes = super::common::code_points(&p, "unicodes")?.ok_or_else(|| p.bad("`unicodes` is required"))?;
    s.edit("Set Unicode Values", |font| {
        for name in &names {
            font.glyph_mut(name)?.unicodes = codes.clone();
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names }))
}

fn toggle_locked(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.toggleLocked", v)?;
    let font = s.font()?;
    let names = match p.opt_glyphs("glyphs", font)? {
        Some(names) => names,
        None => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            vec![name.to_string()]
        }
    };
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let locked = match p.get("locked") {
        None => !names.iter().all(|n| font.glyph(n).is_some_and(|g| g.info.locked)),
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`locked` must be true or false, got {}", kind(other)))),
    };
    s.edit(if locked { "Lock Glyph" } else { "Unlock Glyph" }, |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if g.info.locked != locked {
                g.info.locked = locked;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names, "locked": locked }))
}
