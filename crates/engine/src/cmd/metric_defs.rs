//! Metric and stem definitions: `font.metrics.list/add/set/remove`, `font.master.setMetric`, `font.stems.list/add/set/remove`, `font.master.setStem`. Adding a definition adds a value to every master; removing one removes them.
//!
//! The font holds the definitions ([`Font::metrics`], [`Font::stems`]); each master holds one
//! value per definition, in the same order ([`Master::metric_values`], [`Master::stem_values`]).
//! A master's list may be shorter than the font's: a missing value reads as zero, and a command
//! that writes one fills the gap with zeros first.
//!
//! A metric is addressed by its index or by a name: its own name, else its category (format
//! spelling or readable label, any case); where several share a category, the one without a name
//! and scope wins. A stem is addressed by its index or its name. Masters are addressed through
//! [`fontcraft_fontinfo::target`].
//!
//! New documents made from glyph sets take the default metrics of their scripts
//! ([`script_defaults`], `fontInfo.metrics.defaultsByScript`).

use std::sync::Arc;

use fontcraft_fontinfo::target::{self, TargetRef};
use fontcraft_fontinfo::{MAX_METRIC_DEFS, MAX_STEM_DEFS};
use fontcraft_model::{Font, MAX_NAME_LEN, Master, MetricDef, MetricValue, StemDef, metric};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::Params;

/// The categories a metric may take: the format's spelling and the label people read.
const CATEGORIES: [(&str, &str); 9] = [
    (metric::ASCENDER, "Ascender"),
    (metric::CAP_HEIGHT, "Cap Height"),
    ("slant height", "Slant Height"),
    (metric::X_HEIGHT, "x-Height"),
    ("midHeight", "Mid Height"),
    ("bodyHeight", "Body Height"),
    (metric::DESCENDER, "Descender"),
    (metric::BASELINE, "Baseline"),
    (metric::ITALIC_ANGLE, "Italic Angle"),
];

/// The italic angle, in degrees clockwise.
const ITALIC_RANGE: std::ops::RangeInclusive<f64> = -89.0..=89.0;
/// A stem width, in units.
const STEM_RANGE: std::ops::RangeInclusive<f64> = 0.0..=16_384.0;
/// The name a stem added without one starts from.
const STEM_NAME: &str = "Stem";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.metrics.list",
            "List Metrics",
            [],
            None,
            "{master?: id|name|index} the font's metric definitions in order and, with a master, that master's value for each → {master?: id, metrics: [{index, category, label, name?, scope?, position?, zone?}]}. Italic Angle has a position (degrees clockwise) and no zone",
            has_doc,
            list_metrics
        ),
        cmd!(
            "font.metrics.add",
            "Add Metric",
            [],
            None,
            "{category: string, name?: string, scope?: predicate} append a metric definition and give every master position 0 and zone 0. Categories: ascender, cap height, slant height, x-height, midHeight, bodyHeight, descender, baseline, italic angle (the readable labels such as `Cap Height` work too); an empty category needs a name. A definition equal to an existing one (same category, name and scope) is refused; so is a scope that does not read as a glyph predicate. At most 256 metrics → {index, metric}",
            has_doc,
            add_metric
        ),
        cmd!(
            "font.metrics.set",
            "Edit Metric",
            [],
            None,
            "{metric: index|name, category?: string, name?: string, scope?: predicate} change a metric's category, name or glyph scope; an empty name or scope removes it. Every master keeps its values; a category change those values do not fit (a baseline off 0, an angle beyond ±89°) is refused → {index, metric}",
            has_doc,
            set_metric
        ),
        cmd!(
            "font.metrics.remove",
            "Remove Metric",
            [],
            None,
            "{metric: index|name} delete a metric definition and its value in every master → {removed: {index, category, label, name?, scope?}, metrics: count}",
            has_doc,
            remove_metric
        ),
        cmd!(
            "font.master.setMetric",
            "Set Master Metric",
            [],
            None,
            "{masters: [id|name|index], metric: index|name, position?: number, zone?: number} set the masters' value for one metric: the position is the offset from the baseline, the zone the size of the alignment zone from that position (negative for a zone below it). At least one of position and zone. The baseline stays at 0; Italic Angle takes a position from -89 to 89 degrees clockwise and no zone. All masters change in one undo step → {metric: index, masters: [{master, position, zone}]}",
            has_doc,
            set_master_metric
        ),
        cmd!(
            query "font.stems.list",
            "List Stems",
            [],
            None,
            "{master?: id|name|index} the font's stem definitions in order and, with a master, that master's width for each → {master?: id, stems: [{index, name, orientation: \"vertical\" | \"horizontal\", scope?, value?}]}",
            has_doc,
            list_stems
        ),
        cmd!(
            "font.stems.add",
            "Add Stem",
            [],
            None,
            "{name?: string, orientation: \"vertical\" | \"horizontal\", scope?: predicate, values?: {master: number}} append a stem definition. A vertical stem is measured left to right, a horizontal one top to bottom. Without a name it is called Stem (Stem.1, Stem.2… while taken); names are unique. Every master gets 0 unless `values` (keyed by master id, name or index) gives its width, from 0 to 16384. At most 256 stems → {index, stem}",
            has_doc,
            add_stem
        ),
        cmd!(
            "font.stems.set",
            "Edit Stem",
            [],
            None,
            "{stem: index|name, name?: string, orientation?: \"vertical\" | \"horizontal\", scope?: predicate} change a stem's name, orientation or glyph scope; an empty scope removes it. A name another stem has is refused → {index, stem}",
            has_doc,
            set_stem
        ),
        cmd!(
            "font.stems.remove",
            "Remove Stem",
            [],
            None,
            "{stem: index|name} delete a stem definition and its width in every master → {removed: {index, name, orientation, scope?}, stems: count}",
            has_doc,
            remove_stem
        ),
        cmd!(
            "font.master.setStem",
            "Set Master Stem Value",
            [],
            None,
            "{masters: [id|name|index], stem: index|name, value: number} set the width the masters record for one stem, from 0 to 16384, in one undo step → {stem: index, masters: [{master, value}]}",
            has_doc,
            set_master_stem
        ),
    ]
}

// ── metrics ─────────────────────────────────────────────────────────────────────────────────────

fn list_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.metrics.list", v)?;
    let font = s.font()?;
    let master = one_master(&p, font, "master")?;
    let metrics: Vec<Value> = font
        .metrics
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut out = metric_json(i, d);
            if let (Some(m), Some(o)) = (master, out.as_object_mut()) {
                let value = m.metric_values.get(i).copied().unwrap_or_default();
                o.insert("position".into(), json!(value.pos));
                if d.kind != metric::ITALIC_ANGLE {
                    o.insert("zone".into(), json!(value.over));
                }
            }
            out
        })
        .collect();
    let mut out = json!({ "metrics": metrics });
    if let (Some(m), Some(o)) = (master, out.as_object_mut()) {
        o.insert("master".into(), json!(m.id));
    }
    Ok(out)
}

fn add_metric(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.metrics.add", v)?;
    let kind = category(&p, p.str("category")?)?;
    let name = opt_name(&p, "name")?;
    let filter = opt_scope(&p)?;
    if kind.is_empty() && name.is_none() {
        return Err(p.bad("a metric without a category needs a `name`"));
    }
    let font = s.font()?;
    if font.metrics.len() >= MAX_METRIC_DEFS {
        return Err(p.bad(format!("the font has {MAX_METRIC_DEFS} metrics, the most it can hold")));
    }
    let def = MetricDef { kind, name, filter, ..MetricDef::new("") };
    if font.metrics.iter().any(|d| same_metric(d, &def)) {
        return Err(p.bad(format!("the font has the metric {} already; give the new one a name or a scope", display(&def))));
    }
    let index = font.metrics.len();
    let out = json!({ "index": index, "metric": metric_json(index, &def) });
    s.edit("Add Metric", |font| {
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            m.metric_values.resize(index, MetricValue::default());
            m.metric_values.push(MetricValue::default());
        }
        Arc::make_mut(&mut font.metrics).push(def);
        Ok(())
    })?;
    Ok(out)
}

fn set_metric(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.metrics.set", v)?;
    let font = s.font()?;
    let index = metric_index(&p, font)?;
    let Some(old) = font.metrics.get(index) else { return Err(p.bad(format!("there is no metric #{index}"))) };
    let mut def = old.clone();
    if let Some(text) = p.opt_str("category")? {
        def.kind = category(&p, text)?;
    }
    if p.get("name").is_some() {
        def.name = opt_name(&p, "name")?;
    }
    if p.get("scope").is_some() {
        def.filter = opt_scope(&p)?;
    }
    if def.kind.is_empty() && def.name.is_none() {
        return Err(p.bad("a metric without a category needs a name"));
    }
    if font.metrics.iter().enumerate().any(|(i, d)| i != index && same_metric(d, &def)) {
        return Err(p.bad(format!("the font has the metric {} already", display(&def))));
    }
    if def.kind != old.kind {
        for m in font.masters.iter() {
            let value = m.metric_values.get(index).copied().unwrap_or_default();
            if let Some(problem) = misfit(&def.kind, value.pos, None) {
                return Err(p.bad(format!("master {}: {problem}", master_label(m))));
            }
        }
    }
    let out = json!({ "index": index, "metric": metric_json(index, &def) });
    s.edit("Edit Metric", |font| {
        if let Some(d) = Arc::make_mut(&mut font.metrics).get_mut(index) {
            *d = def;
        }
        Ok(())
    })?;
    Ok(out)
}

fn remove_metric(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.metrics.remove", v)?;
    let font = s.font()?;
    let index = metric_index(&p, font)?;
    let Some(def) = font.metrics.get(index) else { return Err(p.bad(format!("there is no metric #{index}"))) };
    let removed = metric_json(index, def);
    let count = font.metrics.len().saturating_sub(1);
    s.edit("Remove Metric", |font| {
        let metrics = Arc::make_mut(&mut font.metrics);
        if index < metrics.len() {
            metrics.remove(index);
        }
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            if index < m.metric_values.len() {
                m.metric_values.remove(index);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "removed": removed, "metrics": count }))
}

fn set_master_metric(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.master.setMetric", v)?;
    let position = p.opt_f64("position")?;
    let zone = p.opt_f64("zone")?;
    if position.is_none() && zone.is_none() {
        return Err(p.bad("give a `position`, a `zone` or both"));
    }
    let font = s.font()?;
    let masters = masters(&p, font)?;
    let index = metric_index(&p, font)?;
    let Some(def) = font.metrics.get(index) else { return Err(p.bad(format!("there is no metric #{index}"))) };
    if let Some(problem) = misfit(&def.kind, position.unwrap_or(0.0), zone) {
        return Err(p.bad(format!("{}: {problem}", display(def))));
    }
    for n in position.iter().chain(zone.iter()) {
        if !super::common::in_range(*n) {
            return Err(p.bad(format!("{n} is beyond ±{}", fontcraft_model::MAX_COORDINATE)));
        }
    }
    let mut report = Vec::with_capacity(masters.len());
    for &i in &masters {
        let Some(m) = font.masters.get(i) else { continue };
        let old = m.metric_values.get(index).copied().unwrap_or_default();
        let new = MetricValue { pos: position.unwrap_or(old.pos), over: zone.unwrap_or(old.over) };
        report.push(json!({ "master": m.id, "position": new.pos, "zone": new.over }));
    }
    s.edit("Set Master Metric", |font| {
        for (i, m) in Arc::make_mut(&mut font.masters).iter_mut().enumerate() {
            if !masters.contains(&i) {
                continue;
            }
            if m.metric_values.len() <= index {
                m.metric_values.resize(index + 1, MetricValue::default());
            }
            if let Some(value) = m.metric_values.get_mut(index) {
                if let Some(pos) = position {
                    value.pos = pos;
                }
                if let Some(over) = zone {
                    value.over = over;
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "metric": index, "masters": report }))
}

/// Why `position` (and `zone`, when given) cannot be a value of a metric of category `kind`.
fn misfit(kind: &str, position: f64, zone: Option<f64>) -> Option<String> {
    match kind {
        metric::ITALIC_ANGLE if !ITALIC_RANGE.contains(&position) => Some(format!("the italic angle must be from -89 to 89 degrees, got {position}")),
        metric::ITALIC_ANGLE if zone.is_some() => Some("the italic angle has no zone".into()),
        metric::BASELINE if position != 0.0 => Some(format!("the baseline is always at 0, got {position}")),
        _ => None,
    }
}

/// The index of the metric the `metric` parameter names.
fn metric_index(p: &Params, font: &Font) -> Result<usize> {
    let count = font.metrics.len();
    match p.get("metric") {
        None => Err(p.bad("`metric` is required: an index or a name")),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < count)
            .ok_or_else(|| p.bad(format!("`metric`: there is no metric #{n}; the font has {count}"))),
        Some(Value::String(text)) => {
            let by_name: Vec<usize> =
                font.metrics.iter().enumerate().filter(|(_, d)| d.name.as_deref() == Some(text.as_str())).map(|(i, _)| i).collect();
            let found = if by_name.is_empty() {
                let by_kind: Vec<usize> =
                    font.metrics.iter().enumerate().filter(|(_, d)| !d.kind.is_empty() && category_matches(&d.kind, text)).map(|(i, _)| i).collect();
                let plain: Vec<usize> =
                    by_kind.iter().copied().filter(|i| font.metrics.get(*i).is_some_and(|d| d.name.is_none() && d.filter.is_none())).collect();
                if by_kind.len() > 1 && plain.len() == 1 { plain } else { by_kind }
            } else {
                by_name
            };
            match found.as_slice() {
                [i] => Ok(*i),
                [] => Err(p.bad(format!("`metric`: the font has no metric `{}`", clip(text)))),
                several => {
                    let list: Vec<String> = several.iter().map(|i| format!("#{i}")).collect();
                    Err(p.bad(format!("`metric`: more than one metric is called `{}`: use one of the indexes {}", clip(text), list.join(", "))))
                }
            }
        }
        Some(_) => Err(p.bad("`metric` is an index or a name")),
    }
}

/// Whether `text` names the category `kind`: its format spelling or its label, in any case.
fn category_matches(kind: &str, text: &str) -> bool {
    kind.eq_ignore_ascii_case(text) || label(kind).eq_ignore_ascii_case(text)
}

/// The format spelling of the category `text` names; empty for none.
fn category(p: &Params, text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    CATEGORIES
        .iter()
        .find(|(kind, label)| kind.eq_ignore_ascii_case(text) || label.eq_ignore_ascii_case(text))
        .map(|(kind, _)| (*kind).to_string())
        .ok_or_else(|| {
            let list: Vec<&str> = CATEGORIES.iter().map(|(kind, _)| *kind).collect();
            p.bad(format!("`category`: `{}` is not a metric category; the categories are {}", clip(text), list.join(", ")))
        })
}

/// The label people read for the category `kind`; the stored spelling for one this list lacks.
fn label(kind: &str) -> &str {
    CATEGORIES.iter().find(|(k, _)| *k == kind).map_or(kind, |(_, label)| label)
}

/// Two definitions the user cannot tell apart.
fn same_metric(a: &MetricDef, b: &MetricDef) -> bool {
    a.kind == b.kind && a.name == b.name && a.filter == b.filter
}

/// A metric as messages quote it.
fn display(def: &MetricDef) -> String {
    match &def.name {
        Some(name) => format!("`{name}`"),
        None => format!("`{}`", label(&def.kind)),
    }
}

fn metric_json(index: usize, def: &MetricDef) -> Value {
    let mut o = Map::new();
    o.insert("index".into(), json!(index));
    o.insert("category".into(), json!(def.kind));
    o.insert("label".into(), json!(label(&def.kind)));
    if let Some(name) = &def.name {
        o.insert("name".into(), json!(name));
    }
    if let Some(filter) = &def.filter {
        o.insert("scope".into(), json!(filter));
    }
    Value::Object(o)
}

// ── stems ───────────────────────────────────────────────────────────────────────────────────────

fn list_stems(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.stems.list", v)?;
    let font = s.font()?;
    let master = one_master(&p, font, "master")?;
    let stems: Vec<Value> = font
        .stems
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut out = stem_json(i, d);
            if let (Some(m), Some(o)) = (master, out.as_object_mut()) {
                o.insert("value".into(), json!(m.stem_values.get(i).copied().unwrap_or(0.0)));
            }
            out
        })
        .collect();
    let mut out = json!({ "stems": stems });
    if let (Some(m), Some(o)) = (master, out.as_object_mut()) {
        o.insert("master".into(), json!(m.id));
    }
    Ok(out)
}

fn add_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.stems.add", v)?;
    let horizontal = orientation(&p)?.ok_or_else(|| p.bad("`orientation` is required: \"vertical\" or \"horizontal\""))?;
    let filter = opt_scope(&p)?;
    let font = s.font()?;
    if font.stems.len() >= MAX_STEM_DEFS {
        return Err(p.bad(format!("the font has {MAX_STEM_DEFS} stems, the most it can hold")));
    }
    let name = match opt_name(&p, "name")? {
        Some(name) if stem_named(font, &name, None) => return Err(p.bad(format!("the font has a stem called `{name}` already"))),
        Some(name) => name,
        None => free_stem_name(font),
    };
    let mut values = vec![0.0; font.masters.len()];
    if let Some(given) = p.opt_object("values")? {
        if given.len() > fontcraft_fontinfo::MAX_MASTERS {
            return Err(p.bad(format!("`values` names more than {} masters", fontcraft_fontinfo::MAX_MASTERS)));
        }
        for (key, value) in given {
            let i = target::master(font, &TargetRef::Key(key.clone())).map_err(|e| p.bad(format!("`values`: {e}")))?;
            let width =
                value.as_f64().filter(|n| n.is_finite()).ok_or_else(|| p.bad(format!("`values`: the width for `{}` must be a number", clip(key))))?;
            stem_width(&p, width)?;
            if let Some(slot) = values.get_mut(i) {
                *slot = width;
            }
        }
    }
    let index = font.stems.len();
    let def = StemDef { name, horizontal, filter, ..StemDef::default() };
    let out = json!({ "index": index, "stem": stem_json(index, &def) });
    s.edit("Add Stem", |font| {
        for (m, width) in Arc::make_mut(&mut font.masters).iter_mut().zip(values) {
            m.stem_values.resize(index, 0.0);
            m.stem_values.push(width);
        }
        Arc::make_mut(&mut font.stems).push(def);
        Ok(())
    })?;
    Ok(out)
}

fn set_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.stems.set", v)?;
    let font = s.font()?;
    let index = stem_index(&p, font)?;
    let Some(old) = font.stems.get(index) else { return Err(p.bad(format!("there is no stem #{index}"))) };
    let mut def = old.clone();
    if p.get("name").is_some() {
        let name = opt_name(&p, "name")?.ok_or_else(|| p.bad("`name`: a stem's name must not be empty"))?;
        if stem_named(font, &name, Some(index)) {
            return Err(p.bad(format!("the font has a stem called `{name}` already")));
        }
        def.name = name;
    }
    if let Some(horizontal) = orientation(&p)? {
        def.horizontal = horizontal;
    }
    if p.get("scope").is_some() {
        def.filter = opt_scope(&p)?;
    }
    let out = json!({ "index": index, "stem": stem_json(index, &def) });
    s.edit("Edit Stem", |font| {
        if let Some(d) = Arc::make_mut(&mut font.stems).get_mut(index) {
            *d = def;
        }
        Ok(())
    })?;
    Ok(out)
}

fn remove_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.stems.remove", v)?;
    let font = s.font()?;
    let index = stem_index(&p, font)?;
    let Some(def) = font.stems.get(index) else { return Err(p.bad(format!("there is no stem #{index}"))) };
    let removed = stem_json(index, def);
    let count = font.stems.len().saturating_sub(1);
    s.edit("Remove Stem", |font| {
        let stems = Arc::make_mut(&mut font.stems);
        if index < stems.len() {
            stems.remove(index);
        }
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            if index < m.stem_values.len() {
                m.stem_values.remove(index);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "removed": removed, "stems": count }))
}

fn set_master_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.master.setStem", v)?;
    let width = p.f64("value")?;
    stem_width(&p, width)?;
    let font = s.font()?;
    let masters = masters(&p, font)?;
    let index = stem_index(&p, font)?;
    let report: Vec<Value> = masters.iter().filter_map(|i| font.masters.get(*i)).map(|m| json!({ "master": m.id, "value": width })).collect();
    s.edit("Set Master Stem Value", |font| {
        for (i, m) in Arc::make_mut(&mut font.masters).iter_mut().enumerate() {
            if !masters.contains(&i) {
                continue;
            }
            if m.stem_values.len() <= index {
                m.stem_values.resize(index + 1, 0.0);
            }
            if let Some(slot) = m.stem_values.get_mut(index) {
                *slot = width;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "stem": index, "masters": report }))
}

/// The index of the stem the `stem` parameter names.
fn stem_index(p: &Params, font: &Font) -> Result<usize> {
    let count = font.stems.len();
    match p.get("stem") {
        None => Err(p.bad("`stem` is required: an index or a name")),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < count)
            .ok_or_else(|| p.bad(format!("`stem`: there is no stem #{n}; the font has {count}"))),
        Some(Value::String(text)) => {
            let found: Vec<usize> = font.stems.iter().enumerate().filter(|(_, d)| !d.name.is_empty() && d.name == *text).map(|(i, _)| i).collect();
            match found.as_slice() {
                [i] => Ok(*i),
                [] => Err(p.bad(format!("`stem`: the font has no stem `{}`", clip(text)))),
                several => {
                    let list: Vec<String> = several.iter().map(|i| format!("#{i}")).collect();
                    Err(p.bad(format!("`stem`: more than one stem is called `{}`: use one of the indexes {}", clip(text), list.join(", "))))
                }
            }
        }
        Some(_) => Err(p.bad("`stem` is an index or a name")),
    }
}

/// Whether a stem other than `except` is called `name`.
fn stem_named(font: &Font, name: &str, except: Option<usize>) -> bool {
    font.stems.iter().enumerate().any(|(i, d)| Some(i) != except && d.name == name)
}

/// `Stem`, or `Stem.1`, `Stem.2`… while the name is taken.
fn free_stem_name(font: &Font) -> String {
    if !stem_named(font, STEM_NAME, None) {
        return STEM_NAME.to_string();
    }
    // At most MAX_STEM_DEFS names are taken, so one of the first MAX_STEM_DEFS + 1 is free.
    (1..=MAX_STEM_DEFS + 1)
        .map(|n| format!("{STEM_NAME}.{n}"))
        .find(|name| !stem_named(font, name, None))
        .unwrap_or_else(|| format!("{STEM_NAME}.{}", font.stems.len() + 1))
}

/// `orientation`: true for horizontal; `None` when not given.
fn orientation(p: &Params) -> Result<Option<bool>> {
    match p.opt_str("orientation")? {
        None => Ok(None),
        Some(text) => match text.trim().to_ascii_lowercase().as_str() {
            "vertical" => Ok(Some(false)),
            "horizontal" => Ok(Some(true)),
            _ => Err(p.bad(format!("`orientation` is \"vertical\" or \"horizontal\", got `{}`", clip(text)))),
        },
    }
}

fn stem_width(p: &Params, width: f64) -> Result<()> {
    if STEM_RANGE.contains(&width) {
        Ok(())
    } else {
        Err(p.bad(format!("a stem width must be from {} to {}, got {width}", STEM_RANGE.start(), STEM_RANGE.end())))
    }
}

fn stem_json(index: usize, def: &StemDef) -> Value {
    let mut o = Map::new();
    o.insert("index".into(), json!(index));
    o.insert("name".into(), json!(def.name));
    o.insert("orientation".into(), json!(if def.horizontal { "horizontal" } else { "vertical" }));
    if let Some(filter) = &def.filter {
        o.insert("scope".into(), json!(filter));
    }
    Value::Object(o)
}

// ── shared ──────────────────────────────────────────────────────────────────────────────────────

/// The masters the `masters` parameter names, each once; at least one.
fn masters(p: &Params, font: &Font) -> Result<Vec<usize>> {
    let value = p.get("masters").ok_or_else(|| p.bad("`masters` is required: master ids, names or indexes"))?;
    let refs = target::parse_list(value).map_err(|e| p.bad(format!("`masters`: {e}")))?;
    let found = target::masters(font, &refs).map_err(|e| p.bad(format!("`masters`: {e}")))?;
    if found.is_empty() {
        return Err(p.bad("`masters` names no master"));
    }
    Ok(found)
}

/// The master the optional parameter `key` names.
fn one_master<'f>(p: &Params, font: &'f Font, key: &str) -> Result<Option<&'f Master>> {
    let Some(value) = p.get(key) else { return Ok(None) };
    let r = target::parse(value).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    let i = target::master(font, &r).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    font.masters.get(i).map(Some).ok_or_else(|| p.bad(format!("`{key}`: there is no master #{i}")))
}

/// A name parameter: `None` when absent or empty, else a usable name.
fn opt_name(p: &Params, key: &str) -> Result<Option<String>> {
    let Some(text) = p.opt_str(key)? else { return Ok(None) };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{key}` is longer than {MAX_NAME_LEN} bytes")));
    }
    if text.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` contains a control character")));
    }
    Ok(Some(text.to_string()))
}

/// The `scope` parameter: `None` when absent or empty, else a glyph predicate that parses.
fn opt_scope(p: &Params) -> Result<Option<String>> {
    let Some(text) = p.opt_str("scope")? else { return Ok(None) };
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    fontcraft_glyphquery::parse_predicate(text).map_err(|e| p.bad(format!("`scope`: {e}")))?;
    Ok(Some(text.to_string()))
}

fn master_label(m: &Master) -> String {
    if m.name.is_empty() { format!("`{}`", m.id) } else { format!("`{}`", m.name) }
}

/// At most 64 characters of untrusted text, for messages.
fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(64).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

// ── new documents ───────────────────────────────────────────────────────────────────────────────

/// The Arabic default metrics at 1000 units per em: category (empty for a named metric without
/// one), name, position, zone.
const ARABIC: [(&str, Option<&str>, f64, f64); 6] = [
    (metric::ASCENDER, None, 800.0, 16.0),
    ("", Some("Alef Height"), 700.0, 16.0),
    ("", Some("Joining Line"), 150.0, 0.0),
    (metric::BASELINE, None, 0.0, -16.0),
    (metric::DESCENDER, None, -200.0, -16.0),
    ("", Some("Meem Depth"), -250.0, -16.0),
];

/// Gives a new font the default metrics of the scripts chosen for it: the Arabic set when the
/// scripts include Arabic and not Latin, else the font's own set with Italic Angle (0°) added.
pub(crate) fn script_defaults<'a>(font: &mut Font, scripts: impl IntoIterator<Item = &'a str>) {
    let (mut arabic, mut latin) = (false, false);
    for script in scripts {
        arabic |= script.eq_ignore_ascii_case("arabic");
        latin |= script.eq_ignore_ascii_case("latin");
    }
    if arabic && !latin {
        let defs = ARABIC.iter().map(|(kind, name, _, _)| MetricDef { name: name.map(str::to_string), ..MetricDef::new(kind) }).collect();
        font.metrics = Arc::new(defs);
        let values: Vec<MetricValue> = ARABIC.iter().map(|(_, _, pos, over)| MetricValue { pos: *pos, over: *over }).collect();
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            m.metric_values = values.clone();
        }
    } else if !font.metrics.iter().any(|d| d.kind == metric::ITALIC_ANGLE) {
        let index = font.metrics.len();
        Arc::make_mut(&mut font.metrics).push(MetricDef::new(metric::ITALIC_ANGLE));
        for m in Arc::make_mut(&mut font.masters).iter_mut() {
            m.metric_values.resize(index + 1, MetricValue::default());
        }
    }
}
