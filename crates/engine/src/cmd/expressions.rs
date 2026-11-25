//! Anchor position expressions and automatic layer recipes, on `fontcraft_geom::expr` (M13.16).
//! Rows: `anchor.setExpressions`, `anchor.positionExpressions`, `anchor.removeExpressions`,
//! `glyph.setAutoLayerRecipe`, `recipe.directives`. Expressions and recipes are stored under the
//! keys of `fontcraft_model::fc_keys`; the logic is `fontcraft_components::expressions` and
//! `fontcraft_components::auto_layer`.
//!
//! These commands only store what they are given (and work out the anchors at once, to report
//! what fails). Keeping anchors and automatic layers up to date after every later change is the
//! engine's hook, `fontcraft_components::expressions::follow_changes`, which runs where
//! automatic alignment runs, inside each command's undo step.

use std::collections::HashSet;

use fontcraft_components::auto_layer::{has_auto_layer, layer_recipe, parse_auto_recipe, reaches, set_layer_recipe, wanted_parts};
use fontcraft_components::expressions::{AnchorExpressions, ExprFailure, anchor_expressions, evaluate_layer, set_anchor_expressions};
use fontcraft_geom::expr::Expr;
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::common::Outcome;
use super::*;
use crate::check_unlocked;
use crate::params::{Params, Targets};

/// Most layer ids `layers` may name (as the other whole-layer commands).
const MAX_LAYER_IDS: usize = 1024;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "anchor.setExpressions",
            "Set Anchor Expressions",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), anchor: name, x?: string | null, y?: string | null} store expressions for the anchor's position; an empty string or null removes that coordinate's expression, an absent key keeps it. Expressions use + - * /, parentheses, numbers, the functions min, max, abs, round, floor, ceil, sqrt, sin, cos, tan, atan2, and the names width, ascender, descender, baseline, xHeight, capHeight, anchor:NAME.x, anchor:NAME.y, guide:NAME, zone:NAME, param:NAME. The anchor moves to the values at once (rounded to the grid) and follows every later change. A syntax error refuses the command; a name that means nothing on the layer, or a value that is not finite, leaves that coordinate where it is and is listed in warnings. A locked anchor or glyph refuses it → {anchor: name, x, y, expressions: {x?, y?}, warnings: [{anchor, axis, expression, message}], selection: {glyph, layer, anchors: [name]}}",
            has_doc,
            set_expressions
        ),
        cmd!(
            "anchor.removeExpressions",
            "Remove Anchor Expressions",
            ["Glyph", "Anchors"],
            None,
            "{glyph?: name, glyphs?: [names] (default: every glyph), layers?: [ids] (default: every layer), anchors?: [names] (default: every anchor)} remove the position expressions of the anchors named; each keeps the position it has. Locked glyphs are skipped and listed (a single locked `glyph` refuses it) → {glyphs: [names changed], removed: count, skipped: [{glyph, reason}]}",
            has_doc,
            remove_expressions
        )
        .targets(&[crate::params::TargetKind::Glyphs]),
        cmd!(
            "glyph.setAutoLayerRecipe",
            "Set Auto Layer Recipe",
            ["Glyph"],
            None,
            "{glyph?: name, glyphs?: [names] (one of the two), masters?: [ids] (default: every master), recipe?: string, auto?: bool (default true)} make each glyph's master layers automatic, built from the recipe: component names joined by `+` (`a+acutecomb`), optionally `=` and the glyph's own name; on any line `#!` copies the rest of the line into the glyph note, `#?noanchors` stops the glyph from inheriting its components' anchors, and any other `#` starts a comment. Without `recipe`, a glyph keeps the recipe one of its layers has, else takes the components its name decomposes into. An automatic layer holds the recipe's components, aligned automatically, and is rebuilt when they change; while a component glyph is missing it stays empty and fills in once the glyph is added. auto false makes the layers ordinary again, keeping what they hold. A recipe that cannot be read, or whose components contain the glyph, refuses the command; locked glyphs and glyphs without a master's layer are skipped and listed → {glyphs: [names changed], layers: count, pending: [{glyph, missing: [names]}], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            set_auto_layer_recipe
        )
        .targets(&[crate::params::TargetKind::Glyphs]),
    ]
}

fn warning(f: &ExprFailure) -> Value {
    json!({"anchor": f.anchor, "axis": f.axis, "expression": f.expression, "message": f.message})
}

// ── anchor.setExpressions ───────────────────────────────────────────────────────────────────────

/// An expression parameter: `None` when absent (keep), `Some(None)` to remove, `Some(Some(text))`
/// to set. The text must parse. Reads `v` itself because [`Params::get`] treats null as absent.
fn expression_param(p: &Params, v: &Value, key: &str) -> Result<Option<Option<String>>> {
    match v.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => {
            Expr::parse(s).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
            Ok(Some(Some(s.clone())))
        }
        Some(other) => Err(p.bad(format!("`{key}` must be a string or null, found {}", crate::params::kind(other)))),
    }
}

fn set_expressions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.setExpressions", v)?;
    let mut t = Targets::parse(&p)?;
    if t.has_lists() || p.get("side").is_some() {
        return Err(p.bad("names one anchor with `anchor`; takes no target lists and no `side`"));
    }
    let name = p.str("anchor")?.to_string();
    let (x, y) = (expression_param(&p, v, "x")?, expression_param(&p, v, "y")?);
    if x.is_none() && y.is_none() {
        return Err(p.bad("give `x`, `y` or both"));
    }
    let font = s.font()?;
    let r = t.resolve(font)?;
    let (_, a) = r.parts.anchor(&name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
    if a.info.locked {
        return Err(p.bad(format!("anchor `{name}` is locked: unlock it to change it")));
    }
    let mut wanted = anchor_expressions(a).unwrap_or_default();
    if let Some(x) = x {
        wanted.x = x;
    }
    if let Some(y) = y {
        wanted.y = y;
    }
    let layer_id = r.layer.id.clone();
    t.layer = Some(layer_id.clone());
    let glyph = t.glyph.clone();
    let (position, failures) = s.edit("Set Anchor Expressions", |font| {
        t.edit_layer(font, |l| {
            let a = l.anchors.iter_mut().find(|a| a.name == name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
            set_anchor_expressions(a, &wanted);
            Ok(())
        })?;
        let layer = font
            .glyph(&glyph)
            .and_then(|g| g.layers.iter().find(|l| l.id == layer_id))
            .ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{layer_id}`")))?;
        let (moved, failures) = evaluate_layer(font, &glyph, layer);
        let failures: Vec<ExprFailure> = failures.into_iter().filter(|f| f.anchor == name).collect();
        let mut g = font.glyph_mut(&glyph)?;
        let l = g.layers.iter_mut().find(|l| l.id == layer_id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{layer_id}`")))?;
        if let Some(anchors) = moved {
            l.anchors = anchors;
        }
        let a = l.anchors.iter().find(|a| a.name == name).ok_or_else(|| p.bad(format!("the layer has no anchor `{name}`")))?;
        Ok(((a.x, a.y), failures))
    })?;
    t.anchors = vec![name.clone()];
    let mut expressions = serde_json::Map::new();
    if let Some(x) = &wanted.x {
        expressions.insert("x".into(), json!(x));
    }
    if let Some(y) = &wanted.y {
        expressions.insert("y".into(), json!(y));
    }
    Ok(json!({
        "anchor": name,
        "x": position.0,
        "y": position.1,
        "expressions": expressions,
        "warnings": failures.iter().map(warning).collect::<Vec<_>>(),
        "selection": t.to_json(),
    }))
}

// ── anchor.removeExpressions ────────────────────────────────────────────────────────────────────

/// `glyph` or `glyphs`; every glyph when neither is given.
fn glyph_list(p: &Params, font: &Font) -> Result<(Vec<String>, bool)> {
    match (p.get("glyph"), p.get("glyphs")) {
        (Some(_), Some(_)) => Err(p.bad("give `glyph` or `glyphs`, not both")),
        (Some(_), None) => {
            let name = p.str("glyph")?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("there is no glyph `{name}`")));
            }
            Ok((vec![name.to_string()], true))
        }
        (None, Some(_)) => Ok((p.opt_glyphs("glyphs", font)?.unwrap_or_default(), false)),
        (None, None) => Ok((font.glyphs.iter().map(|g| g.name.clone()).collect(), false)),
    }
}

/// An optional list of names, at most `limit`, repeats dropped.
fn name_set(p: &Params, key: &str, limit: usize) -> Result<Option<HashSet<String>>> {
    if let Some(Value::Array(a)) = p.get(key)
        && a.len() > limit
    {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {limit} are allowed", a.len())));
    }
    Ok(p.opt_strings(key)?.map(|v| v.into_iter().map(str::to_string).collect()))
}

fn remove_expressions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("anchor.removeExpressions", v)?;
    let font = s.font()?;
    let (glyphs, single) = glyph_list(&p, font)?;
    let layers = name_set(&p, "layers", MAX_LAYER_IDS)?;
    let anchors = name_set(&p, "anchors", fontcraft_components::MAX_LAYER_ANCHORS)?;
    let mut removed = 0usize;
    let out = s.edit("Remove Anchor Expressions", |font| {
        let mut out = Outcome::default();
        for name in &glyphs {
            let Some(g) = font.glyph(name) else { continue };
            let wanted =
                |layer: &str, anchor: &str| layers.as_ref().is_none_or(|l| l.contains(layer)) && anchors.as_ref().is_none_or(|a| a.contains(anchor));
            let hits = g
                .layers
                .iter()
                .flat_map(|l| l.anchors.iter().map(move |a| (l, a)))
                .filter(|(l, a)| wanted(&l.id, &a.name) && anchor_expressions(a).is_some())
                .count();
            if hits == 0 {
                continue;
            }
            if g.info.locked {
                if single {
                    check_unlocked(g)?;
                }
                out.skip(name, None, "the glyph is locked");
                continue;
            }
            let mut g = font.glyph_mut(name)?;
            for l in &mut g.layers {
                let id = l.id.clone();
                for a in &mut l.anchors {
                    if wanted(&id, &a.name) && anchor_expressions(a).is_some() {
                        set_anchor_expressions(a, &AnchorExpressions::default());
                    }
                }
            }
            removed = removed.saturating_add(hits);
            out.glyphs.push(name.clone());
        }
        Ok(out)
    })?;
    Ok(json!({"glyphs": out.glyphs, "removed": removed, "skipped": out.skipped}))
}

// ── glyph.setAutoLayerRecipe ────────────────────────────────────────────────────────────────────

fn set_auto_layer_recipe(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.setAutoLayerRecipe", v)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let (glyphs, single) = match (p.get("glyph"), p.get("glyphs")) {
        (None, None) => return Err(p.bad("`glyph` or `glyphs` is required")),
        _ => glyph_list(&p, font)?,
    };
    if glyphs.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let masters: Vec<String> = match name_set(&p, "masters", fontcraft_fontinfo::MAX_MASTERS)? {
        Some(ids) => {
            if ids.is_empty() {
                return Err(p.bad("`masters` is empty"));
            }
            if let Some(id) = ids.iter().find(|id| font.master(id).is_err()) {
                return Err(p.bad(format!("there is no master `{id}`")));
            }
            font.masters.iter().filter(|m| ids.contains(&m.id)).map(|m| m.id.clone()).collect()
        }
        None => font.masters.iter().map(|m| m.id.clone()).collect(),
    };
    let auto = p.bool_or("auto", true)?;
    let given = p.opt_str("recipe")?;
    if given.is_some() && !auto {
        return Err(p.bad("`recipe` makes layers automatic: leave it out with auto false"));
    }

    // Work out each glyph's recipe before changing anything, so a bad one changes nothing.
    let mut plans: Vec<(String, Option<String>)> = Vec::new();
    let mut out = Outcome::default();
    for name in &glyphs {
        let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        if g.info.locked {
            if single {
                check_unlocked(g)?;
            }
            out.skip(name, None, "the glyph is locked");
            continue;
        }
        if !auto {
            if has_auto_layer(g) {
                plans.push((name.clone(), None));
            }
            continue;
        }
        let text = match given {
            Some(t) => t.to_string(),
            None => match g.layers.iter().find_map(layer_recipe) {
                Some(t) => t.to_string(),
                None => match fontcraft_components::compose::components_for(font, &data, name) {
                    Ok(Some(parts)) => parts.join("+"),
                    Ok(None) => {
                        if single {
                            return Err(p.bad(format!("`{name}` has no recipe and its name does not decompose: give `recipe`")));
                        }
                        out.skip(name, None, "no recipe given, and the name does not decompose");
                        continue;
                    }
                    Err(e) => {
                        if single {
                            return Err(p.bad(format!("`{name}` has no recipe and none can be worked out ({e}): give `recipe`")));
                        }
                        out.skip(name, None, format!("no recipe given, and none can be worked out: {e}"));
                        continue;
                    }
                },
            },
        };
        let recipe = parse_auto_recipe(&text, name).map_err(|e| p.bad(format!("`{name}`: {e}")))?;
        if let Some(part) = recipe.parts.iter().find(|part| font.glyph(part).is_some() && reaches(font, part, name)) {
            return Err(p.bad(format!("`{name}`: its component `{part}` contains `{name}`, so the glyph would contain itself")));
        }
        plans.push((name.clone(), Some(text)));
    }

    let mut layers = 0usize;
    let mut pending: Vec<Value> = Vec::new();
    let label = if auto { "Set Auto Layer Recipe" } else { "Remove Auto Layer Recipe" };
    let changed = s.edit(label, |font| {
        let mut changed = Vec::new();
        for (name, text) in &plans {
            let note = match text {
                Some(t) => parse_auto_recipe(t, name).map_err(|e| p.bad(format!("`{name}`: {e}")))?.note,
                None => None,
            };
            let mut g = font.glyph_mut(name)?;
            let mut touched = false;
            for master in &masters {
                match g.layer_mut(master) {
                    Some(l) => {
                        if layer_recipe(l) != text.as_deref() {
                            set_layer_recipe(l, text.as_deref());
                            touched = true;
                        }
                        layers = layers.saturating_add(1);
                    }
                    None if text.is_some() => out.skip(name, Some(master.as_str()), "the glyph has no layer for this master"),
                    None => {}
                }
            }
            if let Some(note) = note
                && g.info.note != note
            {
                g.info.note = note;
                touched = true;
            }
            drop(g);
            if touched {
                changed.push(name.clone());
            }
            if let Some(t) = text
                && let Some(want) = wanted_parts(font, name, t)
                && want.is_empty()
            {
                let missing: Vec<String> =
                    parse_auto_recipe(t, name).map(|r| r.parts.into_iter().filter(|part| font.glyph(part).is_none()).collect()).unwrap_or_default();
                pending.push(json!({"glyph": name, "missing": missing}));
            }
        }
        Ok(changed)
    })?;
    Ok(json!({"glyphs": changed, "layers": layers, "pending": pending, "skipped": out.skipped}))
}
