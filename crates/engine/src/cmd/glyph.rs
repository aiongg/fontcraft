//! Glyph menu: adding and removing glyphs, code points, locking.

use std::collections::HashSet;

use fontcraft_model::{Font, Layer, MAX_OUTLINE_ELEMENTS, MetricsKeys, Shape};
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
            "{name?: string, unicodes?: [hex | number], width?: number} add an empty glyph with a layer for every master, at the place the default sort order gives it among the glyphs already there (the others keep their order). name: at most 255 bytes, no control characters; defaults to newGlyph (newGlyph.001, .002… when taken). unicodes: default the code point the glyph data gives the name. width: 0 to 65535, for every master; by default each master's layer takes the Default Layer Width parameter's entry for the glyph's script, else its entry without a script, else 600 (1000 for a CJK glyph, 200 for a `_corner.` glyph) → {name, index, unicodes}",
            has_doc,
            new_glyph
        ),
        cmd!(
            "glyph.removeGlyph",
            "Remove Glyph",
            ["Glyph"],
            Some("Cmd+Delete"),
            "{glyphs: [names], components?: \"remove\" | \"decompose\", metricsKeys?: \"dissolve\" | \"keep\", confirm?: bool (default true)} removes glyphs from the font. When glyphs that stay use a removed glyph as a component, `components` must say what happens to those components: remove deletes them, decompose turns them into outlines first. When metrics keys of glyphs that stay point at a removed glyph, `metricsKeys` must say whether those keys are dissolved or kept. Without a needed answer the command refuses and names it. Kerning pairs that name a removed glyph stay. confirm: false changes nothing and reports what the answers would cover → {removed: [names], usedAsComponent: [{glyph, layer, component}], metricsKeys: [{glyph, layer?, key, value}], needs: [\"components\" | \"metricsKeys\"]}",
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
    let width = p.opt_f64("width")?;
    if let Some(width) = width
        && !(0.0..=65_535.0).contains(&width)
    {
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
        let layers = super::widths::new_glyph_layers(font, Some(&data), &name, &codes);
        font.add_glyph_at(index, &name, codes.clone(), width.unwrap_or(super::widths::BUILT_IN_LAYER_WIDTH))?;
        if width.is_none() {
            font.glyph_mut(&name)?.layers = layers;
        }
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
    if !decompose {
        return Ok(Some(shapes.iter().filter(|s| !matches!(s, Shape::Component(c) if doomed.contains(c.glyph.as_str()))).cloned().collect()));
    }
    let refuse = |message: String| crate::EngineError::BadParams { command: "glyph.removeGlyph".into(), message };
    let out = super::decompose::decompose_where(font, layer, shapes, |g| doomed.contains(g)).map_err(refuse)?;
    // One budget for every layer the removal rewrites.
    let nodes = |shapes: &[Shape]| shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).sum::<usize>();
    let added = nodes(&out).saturating_sub(nodes(shapes));
    *budget = budget.checked_sub(added).ok_or_else(|| refuse(format!("decomposing would add more than {MAX_OUTLINE_ELEMENTS} nodes")))?;
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
    if !confirm {
        return Ok(json!({"removed": [], "usedAsComponent": uses_json, "metricsKeys": keys_json, "needs": needs}));
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
        for name in &names {
            font.remove_glyph(name)?;
        }
        Ok(())
    })?;
    Ok(json!({"removed": names, "usedAsComponent": uses_json, "metricsKeys": keys_json, "needs": []}))
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
