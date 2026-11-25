//! Metrics keys: evaluating them, listing the ones out of sync, and Update Metrics.
//!
//! The parser, the evaluator and the out-of-sync scan are in `fontcraft_spacing`; the scan's
//! cache lives on the session ([`SessionSpacingExt::metrics_sync`]). Updating a metric sets it
//! to its key's value the way the setters do: a left sidebearing moves everything drawn in the
//! layer (`shared::shift_with_right_edge`) and keeps the right sidebearing; a right sidebearing or a
//! width moves the right edge; a top sidebearing moves the vertical origin and keeps the bottom one; a
//! bottom sidebearing or a vertical width changes the vertical width; a vertical origin moves the
//! vertical box and keeps its height.

use std::collections::HashSet;

use fontcraft_model::guide_follow::{BothGuides, follow_width};
use fontcraft_model::{Font, Glyph, Layer, Master};
use fontcraft_spacing::{
    EvalError, Field, Finding, Key, KeyError, Problem, Scope, Slot, WIDTH_RANGE, effective_key, metric, parse_field, parse_stored,
};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind};
use crate::prelude::*;

/// Most master ids `spacing.listOutOfSyncMetrics` takes.
const MAX_MASTER_IDS: usize = 1_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "spacing.updateMetricsKey",
            "Update Metrics Key",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), field: \"left\" | \"right\" | \"width\" | \"top\" | \"bottom\" | \"vertWidth\" | \"vertOrigin\"} set that metric of the layer to the value of the metrics key that applies to it (the layer's own key, else the glyph's). Refused when the field has no key, the key cannot be evaluated, the layer is empty (for a sidebearing), the glyph is locked, or the layer's master takes its metrics from another master. A metric already in sync is left as it is → {glyph, layer, field, value, previous, changed: bool}",
            has_doc,
            update_metrics_key
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "glyph.updateMetrics",
            "Update Metrics",
            ["Glyph"],
            Some("Cmd+Ctrl+M"),
            "{glyphs?: [names] (default: every glyph), layer?: id (default: the first master's layer)} set every metric of that layer of each glyph that has a metrics key to the key's value, in dependency order, so a chain of keys settles in one run. Keys that cannot be evaluated, empty layers, locked glyphs and glyphs without the layer are skipped and listed. Refused when the layer is a master that takes its metrics from another master → {glyphs: [names changed], updated: [{glyph, layer, field, from, to}], skipped: [{glyph, layer?, field?, reason}]}",
            has_doc,
            update_metrics
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "glyph.updateMetricsForAllMasters",
            "Update Metrics for All Masters",
            ["Glyph"],
            Some("Cmd+Ctrl+Alt+M"),
            "{glyphs?: [names] (default: every glyph)} Update Metrics on every master layer of each glyph and on its layers with attributes (intermediate, alternate, colour, smart), leaving out masters that take their metrics from another master → {glyphs: [names changed], updated: [{glyph, layer, field, from, to}], skipped: [{glyph, layer?, field?, reason}]}",
            has_doc,
            update_metrics_all_masters
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            query "spacing.getMetrics",
            "Get Metrics",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer)} the layer's width and sidebearings (null for a sidebearing of an empty layer), and for each field the metrics key that applies, where it is stored, its value and whether the metric matches it → {glyph, layer, master, linkedTo: master id | null, width, left, right, top, bottom, vertWidth, vertOrigin (the stored values, else the ascender to descender and 0), keys: {left|right|width|top|bottom|vertWidth|vertOrigin: {key (as a field shows it, `==` for a layer's own key), scope: \"glyph\" | \"local\", value: number | null, inSync: bool | null, error: string | null} | null}, derived: [fields that automatic alignment sets on this layer, shown greyed: width, left, right]}",
            has_doc,
            get_metrics
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "spacing.evaluateMetricsKey",
            "Evaluate Metrics Key",
            [],
            None,
            "{key: string (a number, a glyph name, `=…` or `==…`; at most 256 bytes), glyph: name, layer?: id (default: the first master's layer), field: \"left\" | \"right\" | \"width\" | \"top\" | \"bottom\" | \"vertWidth\" | \"vertOrigin\"} the value the text gives in that field of the layer, rounded half up for a key, without changing anything. A key that cannot be read names the byte where reading stopped → {value, key: text as a field shows it | null (a plain number), scope: \"glyph\" | \"local\" | null}",
            has_doc,
            evaluate_metrics_key
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "spacing.listOutOfSyncMetrics",
            "List Out-of-Sync Metrics",
            [],
            None,
            "{glyphs?: [names] (default: every glyph), masters?: [ids] (default: every master)} every metrics key whose value differs from the layer's metric rounded half up, every key that names a missing glyph and every key that cannot be evaluated, in glyph order. Master layers and layers with attributes are listed; masters that take their metrics from another master are not → {count, findings: [{glyph, layer, master, field, key, scope: \"glyph\" | \"local\", problem: \"outOfSync\" | \"unlinked\" | \"error\", value?, actual?, missing?, message?}]}",
            has_doc,
            list_out_of_sync
        ),
    ]
}

// ── Reading parameters ──────────────────────────────────────────────────────────────────────────

fn field(p: &Params) -> Result<Field> {
    let name = p.str("field")?;
    Field::from_name(name).ok_or_else(|| {
        p.bad(format!("`field` must be \"left\", \"right\", \"width\", \"top\", \"bottom\", \"vertWidth\" or \"vertOrigin\", got \"{name}\""))
    })
}

/// `glyph` and `layer?`, checked against `font`.
fn glyph_layer<'f>(p: &Params, font: &'f Font) -> Result<(&'f Glyph, &'f Layer)> {
    let name = p.str("glyph")?;
    let g = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
    let id = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let l = g.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{name}` has no layer `{id}`")))?;
    Ok((g, l))
}

/// The metrics key that applies to `f` of the layer, read, and the text its field shows: `==…`
/// for the layer's own key, the stored text when it cannot be read.
pub(super) fn shown_key(font: &Font, g: &Glyph, l: &Layer, f: Field) -> Option<(Scope, std::result::Result<Key, KeyError>, String)> {
    effective_key(g, l, f).map(|(scope, text)| {
        let parsed = parse_stored(text, scope, f, |n| font.glyph(n).is_some());
        let shown = match (&parsed, scope) {
            (Ok(k), _) => k.display(),
            (Err(_), Scope::Local) => format!("=={}", text.strip_prefix('=').unwrap_or(text)),
            (Err(_), Scope::Glyph) => text.to_string(),
        };
        (scope, parsed, shown)
    })
}

/// The master `master` takes its metrics from, when that is another master.
fn linked_to<'f>(font: &'f Font, master: &str) -> Option<&'f Master> {
    fontcraft_spacing::metrics_source(font, master).filter(|m| m.id != master)
}

fn linked_message(font: &Font, master: &str, source: &Master) -> String {
    let name = font.masters.iter().find(|m| m.id == master).map_or(master, |m| m.name.as_str());
    format!("master `{name}` takes its metrics from master `{}`: edit that master instead", source.name)
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Glyph => "glyph",
        Scope::Local => "local",
    }
}

/// The layers the out-of-sync scan covers: master layers and layers with attributes.
fn scanned(layer: &Layer) -> bool {
    layer.is_master_layer() || !layer.info.attrs.is_empty()
}

// ── Applying a value ────────────────────────────────────────────────────────────────────────────

/// What setting one metric does to a layer.
enum Change {
    /// Move everything sideways by `dx` and add it to the width.
    Left(f64),
    Width(f64),
    /// The vertical origin and the vertical width.
    Vertical {
        origin: Option<f64>,
        width: f64,
    },
    /// The vertical origin alone: the vertical box moves and keeps its height.
    Origin(f64),
}

/// How to bring `field` of `layer` from `actual` to `value`.
fn plan(font: &Font, layer: &Layer, field: Field, value: f64, actual: f64) -> std::result::Result<Change, String> {
    let delta = value - actual;
    let width = |w: f64| {
        if WIDTH_RANGE.contains(&w) { Ok(w) } else { Err(format!("the width would be {w}, outside 0 to 65535")) }
    };
    match field {
        Field::Left => {
            width(layer.width + delta)?;
            Ok(Change::Left(delta))
        }
        Field::Right => Ok(Change::Width(width(layer.width + delta)?)),
        Field::Width => Ok(Change::Width(width(value)?)),
        Field::Top | Field::Bottom => {
            let m = |f: Field| metric(font, layer, f).ok().flatten();
            let bounds = fontcraft_spacing::measure::bounds(font, layer).ok().flatten();
            let (Some(top), Some(bottom), Some(b)) = (m(Field::Top), m(Field::Bottom), bounds) else {
                return Err("the layer is empty, so it has no sidebearing".into());
            };
            let current = top + bottom + (b.y1 - b.y0);
            let vertical = current + delta;
            if !WIDTH_RANGE.contains(&vertical) {
                return Err(format!("the vertical width would be {vertical}, outside 0 to 65535"));
            }
            let origin = match field {
                Field::Top => Some(layer.info.vert_origin.unwrap_or(0.0) - delta),
                _ => layer.info.vert_origin,
            };
            Ok(Change::Vertical { origin, width: vertical })
        }
        Field::VertWidth => {
            if !WIDTH_RANGE.contains(&value) {
                return Err(format!("the vertical width would be {value}, outside 0 to 65535"));
            }
            Ok(Change::Vertical { origin: layer.info.vert_origin, width: value })
        }
        Field::VertOrigin => Ok(Change::Origin(value)),
    }
}

fn apply(layer: &mut Layer, change: &Change, both: BothGuides) -> std::result::Result<(), String> {
    match *change {
        Change::Left(dx) => {
            super::shared::shift_with_right_edge(layer, dx, both)?;
            layer.width += dx;
        }
        Change::Width(w) => {
            let before = layer.width;
            layer.width = w;
            follow_width(layer, before, both);
        }
        Change::Vertical { origin, width } => {
            if layer.info.vert_origin != origin {
                layer.info.vert_origin = origin;
            }
            layer.info.vert_width = Some(width);
        }
        Change::Origin(origin) => layer.info.vert_origin = Some(origin),
    }
    Ok(())
}

/// Sets one metric to its key's value. `Ok(None)`: no key applies, or the metric is in sync.
fn update_slot(font: &mut Font, slot: &Slot) -> std::result::Result<Option<(f64, f64)>, String> {
    let value = match fontcraft_spacing::evaluate(font, &slot.glyph, &slot.layer, slot.field) {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let glyph = font.glyph(&slot.glyph).ok_or_else(|| format!("there is no glyph `{}`", slot.glyph))?;
    let layer = glyph.layers.iter().find(|l| l.id == slot.layer).ok_or_else(|| format!("glyph `{}` has no layer `{}`", slot.glyph, slot.layer))?;
    let actual = match metric(font, layer, slot.field) {
        Ok(Some(a)) => a,
        Ok(None) => return Err("the layer is empty, so it has no sidebearing".into()),
        Err(e) => return Err(e.to_string()),
    };
    if fontcraft_spacing::round_half_up(actual) == value {
        return Ok(None);
    }
    let change = plan(font, layer, slot.field, value, actual)?;
    let both = BothGuides::of(font);
    let mut g = font.glyph_mut(&slot.glyph).map_err(|e| e.to_string())?;
    let layer = g.layers.iter_mut().find(|l| l.id == slot.layer).ok_or_else(|| format!("glyph `{}` has no layer `{}`", slot.glyph, slot.layer))?;
    apply(layer, &change, both)?;
    Ok(Some((actual, value)))
}

// ── Update Metrics Key ──────────────────────────────────────────────────────────────────────────

fn update_metrics_key(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.updateMetricsKey", v)?;
    let field = field(&p)?;
    let font = s.font()?;
    let (g, l) = glyph_layer(&p, font)?;
    if g.info.locked {
        return Err(EngineError::Locked(g.name.clone()));
    }
    if let Some(source) = linked_to(font, &l.master) {
        return Err(p.bad(linked_message(font, &l.master, source)));
    }
    if effective_key(g, l, field).is_none() {
        return Err(p.bad(format!("the {} of glyph `{}` has no metrics key on layer `{}`", field.name(), g.name, l.id)));
    }
    let slot = Slot { glyph: g.name.clone(), layer: l.id.clone(), field };
    let current = metric(font, l, field).ok().flatten();
    let done = s.edit("Update Metrics Key", |font| {
        update_slot(font, &slot).map_err(|m| EngineError::BadParams { command: "spacing.updateMetricsKey".into(), message: m })
    })?;
    let (previous, value) = match done {
        Some(pair) => pair,
        None => {
            let value = fontcraft_spacing::evaluate(s.font()?, &slot.glyph, &slot.layer, field).ok().flatten();
            (current.unwrap_or_default(), value.or(current).unwrap_or_default())
        }
    };
    Ok(json!({"glyph": slot.glyph, "layer": slot.layer, "field": field.name(), "value": value, "previous": previous, "changed": done.is_some()}))
}

// ── Update Metrics ──────────────────────────────────────────────────────────────────────────────

/// Which layers of each glyph Update Metrics sets.
enum Layers {
    One(String),
    Scanned,
}

fn update_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.updateMetrics", v)?;
    let font = s.font()?;
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    if let Some(source) = linked_to(font, &layer) {
        return Err(p.bad(linked_message(font, &layer, source)));
    }
    run_update(s, &p, "Update Metrics", Layers::One(layer))
}

fn update_metrics_all_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("glyph.updateMetricsForAllMasters", v)?;
    run_update(s, &p, "Update Metrics for All Masters", Layers::Scanned)
}

fn run_update(s: &mut Session, p: &Params, label: &str, which: Layers) -> Result<Value> {
    let font = s.font()?;
    let names = match p.opt_glyphs("glyphs", font)? {
        Some(n) if n.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(n) => n,
        None => font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect(),
    };
    let mut skipped = Vec::new();
    let mut slots = Vec::new();
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        if g.info.locked {
            skipped.push(json!({"glyph": name, "reason": "the glyph is locked"}));
            continue;
        }
        let layers: Vec<&Layer> = match &which {
            Layers::One(id) => match g.layers.iter().find(|l| l.id == *id) {
                Some(l) => vec![l],
                None => {
                    skipped.push(json!({"glyph": name, "layer": id, "reason": "the glyph has no such layer"}));
                    continue;
                }
            },
            Layers::Scanned => g.layers.iter().filter(|l| scanned(l)).collect(),
        };
        for l in layers {
            if let Some(source) = linked_to(font, &l.master) {
                if matches!(which, Layers::One(_)) {
                    skipped.push(json!({"glyph": name, "layer": l.id, "reason": linked_message(font, &l.master, source)}));
                }
                continue;
            }
            for field in Field::ALL {
                if effective_key(g, l, field).is_some() {
                    slots.push(Slot { glyph: name.clone(), layer: l.id.clone(), field });
                }
            }
        }
    }
    let order = fontcraft_spacing::update_order(font, &slots);
    let (updated, changed, failed) = s.edit(label, |font| {
        let mut updated = Vec::new();
        let mut changed: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut failed = Vec::new();
        for slot in &order {
            match update_slot(font, slot) {
                Ok(None) => {}
                Ok(Some((from, to))) => {
                    updated.push(json!({"glyph": slot.glyph, "layer": slot.layer, "field": slot.field.name(), "from": from, "to": to}));
                    if seen.insert(slot.glyph.clone()) {
                        changed.push(slot.glyph.clone());
                    }
                }
                Err(reason) => failed.push(json!({"glyph": slot.glyph, "layer": slot.layer, "field": slot.field.name(), "reason": reason})),
            }
        }
        Ok((updated, changed, failed))
    })?;
    skipped.extend(failed);
    // Changed glyphs in the order the font lists them, like the other glyph commands.
    let font = s.font()?;
    let changed_set: HashSet<&str> = changed.iter().map(String::as_str).collect();
    let glyphs: Vec<&str> = font.glyphs.as_arcs().iter().map(|g| g.name.as_str()).filter(|n| changed_set.contains(n)).collect();
    Ok(json!({"glyphs": glyphs, "updated": updated, "skipped": skipped}))
}

// ── Queries ─────────────────────────────────────────────────────────────────────────────────────

fn get_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.getMetrics", v)?;
    let font = s.font()?;
    let (g, l) = glyph_layer(&p, font)?;
    let measured = |f: Field| metric(font, l, f).map_err(|e| p.bad(e.to_string()));
    let mut out = serde_json::Map::new();
    let mut keys = serde_json::Map::new();
    for f in Field::ALL {
        let actual = measured(f)?;
        out.insert(f.name().into(), json!(actual));
        let entry = shown_key(font, g, l, f).map(|(scope, parsed, shown)| {
            let value = parsed.map_err(EvalError::from).and_then(|k| fontcraft_spacing::evaluate_key(font, &g.name, &l.id, f, &k));
            let (value, error) = match value {
                Ok(v) => (Some(v), None),
                Err(e) => (None, Some(e.to_string())),
            };
            let in_sync = value.zip(actual).map(|(v, a)| fontcraft_spacing::round_half_up(a) == v);
            json!({"key": shown, "scope": scope_name(scope), "value": value, "inSync": in_sync, "error": error})
        });
        keys.insert(f.name().into(), entry.unwrap_or(Value::Null));
    }
    out.insert("glyph".into(), json!(g.name));
    out.insert("layer".into(), json!(l.id));
    out.insert("master".into(), json!(l.master));
    out.insert("linkedTo".into(), json!(linked_to(font, &l.master).map(|m| m.id.clone())));
    out.insert("keys".into(), Value::Object(keys));
    let derived: Vec<&str> = Field::ALL.iter().filter(|&&f| fontcraft_spacing::derived_field(font, l, f)).map(|f| f.name()).collect();
    out.insert("derived".into(), json!(derived));
    Ok(Value::Object(out))
}

fn evaluate_metrics_key(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.evaluateMetricsKey", v)?;
    let text = p.str("key")?;
    let field = field(&p)?;
    let font = s.font()?;
    let (g, l) = glyph_layer(&p, font)?;
    match parse_field(text, field, |n| font.glyph(n).is_some()).map_err(|e| p.bad(format!("`key`: {e}")))? {
        fontcraft_spacing::Entry::Number(n) => Ok(json!({"value": n, "key": null, "scope": null})),
        fontcraft_spacing::Entry::Key(k) => {
            let value = fontcraft_spacing::evaluate_key(font, &g.name, &l.id, field, &k).map_err(|e| p.bad(format!("`key`: {e}")))?;
            Ok(json!({"value": value, "key": k.display(), "scope": scope_name(k.scope)}))
        }
    }
}

fn finding_json(font: &Font, f: &Finding) -> Value {
    let master = font.glyph(&f.glyph).and_then(|g| g.layers.iter().find(|l| l.id == f.layer)).map(|l| l.master.clone());
    let mut v = json!({
        "glyph": f.glyph,
        "layer": f.layer,
        "master": master,
        "field": f.field.name(),
        "key": f.key,
        "scope": scope_name(f.scope),
    });
    if let Some(o) = v.as_object_mut() {
        match &f.problem {
            Problem::OutOfSync { value, actual } => {
                o.insert("problem".into(), json!("outOfSync"));
                o.insert("value".into(), json!(value));
                o.insert("actual".into(), json!(actual));
            }
            Problem::Unlinked { glyph } => {
                o.insert("problem".into(), json!("unlinked"));
                o.insert("missing".into(), json!(glyph));
            }
            Problem::Error(e) => {
                o.insert("problem".into(), json!("error"));
                o.insert("message".into(), json!(e.to_string()));
            }
        }
    }
    v
}

fn list_out_of_sync(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.listOutOfSyncMetrics", v)?;
    let font = s.doc()?.font.clone();
    let glyphs = p.opt_glyphs("glyphs", &font)?;
    if let Some(Value::Array(a)) = p.get("masters")
        && a.len() > MAX_MASTER_IDS
    {
        return Err(p.bad(format!("`masters` holds {} ids; at most {MAX_MASTER_IDS} are allowed", a.len())));
    }
    let masters: Option<HashSet<String>> = match p.opt_strings("masters")? {
        Some(ids) => {
            let mut set = HashSet::new();
            for id in ids {
                set.insert(font.master(id).map_err(|_| p.bad(format!("`masters`: there is no master `{id}`")))?.id.clone());
            }
            Some(set)
        }
        None => None,
    };
    let cache = s.metrics_sync()?;
    let wanted: Option<HashSet<&str>> = glyphs.as_ref().map(|g| g.iter().map(String::as_str).collect());
    let findings: Vec<Value> = cache
        .all(&font)
        .filter(|f| wanted.as_ref().is_none_or(|w| w.contains(f.glyph.as_str())))
        .map(|f| finding_json(&font, f))
        .filter(|j| masters.as_ref().is_none_or(|m| j.get("master").and_then(Value::as_str).is_some_and(|id| m.contains(id))))
        .collect();
    Ok(json!({"count": findings.len(), "findings": findings}))
}
