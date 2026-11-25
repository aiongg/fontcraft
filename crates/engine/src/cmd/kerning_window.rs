//! The Kerning window's commands: the pair list with its search options and sort, copy and paste
//! of pairs as kerning-text, and deletion. Behaviour notes: `docs/spec/08-spacing-kerning.md`
//! § 3.8; the format: `fontcraft_spacing::kerning_text`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{Font, MAX_NAME_LEN};
use fontcraft_spacing::kerning::pairs::{self, Column, EntryKind, Filter, Match, NewPair, SearchSide};
use fontcraft_spacing::kerning::{Direction, GroupSide, Position, Side, facing};
use fontcraft_spacing::kerning_text::{self, KerningText, MAX_PAIRS, MAX_TEXT_BYTES};
use fontcraft_spacing::{metrics_source, round_half_up};
use serde_json::{Value, json};

use super::shared::{MAX_KERN_VALUE, editable_master};
use super::*;
use crate::params::Params;

/// Most conflicts, unknown glyphs and unknown groups a paste lists by name; the counts are
/// always complete.
const MAX_REPORTED: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "kerning.listPairs",
            "List Kerning Pairs",
            [],
            None,
            "{master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default ltr), term?: string, match?: \"contains\"|\"is\"|\"beginsWith\"|\"endsWith\"|\"doesNotContain\" (default contains), kind?: \"group\"|\"notGroup\", side?: \"left\"|\"right\" (default both), glyphs?: [names] (only pairs whose searched side is one of them or a group one of them is in), sort?: \"left\"|\"value\"|\"right\" (default: stored order), descending?: bool} the stored pairs of the master and direction that match the search, in stored order or sorted (ties keep stored order). A term starting with @ is compared with the @ spelling of groups. A linked master lists the pairs of the master it follows → {master, source, direction, total, count, pairs: [{left, right, value, index}]}",
            has_doc,
            list_pairs
        ),
        cmd!(
            query "kerning.copyPairs",
            "Copy Kerning Pairs",
            [],
            None,
            "{pairs?: [{left: name|\"@group\", right: name|\"@group\"}] (default: every pair), query?: {term?, match?, kind?, side?, glyphs?} (the pairs kerning.listPairs lists for it, in place of pairs), master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default ltr)} the pairs with their values as kerning-text (tab-separated `left right value` lines after a header, master and direction line), for the clipboard. Every named pair must exist → {text, count, master, direction}",
            has_doc,
            copy_pairs
        ),
        cmd!(
            "kerning.pastePairs",
            "Paste Kerning Pairs",
            [],
            None,
            "{text?: kerning-text, pairs?: [{left, right, value}] (exactly one of the two), master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: the text's direction line, else ltr), onConflict?: \"ask\"|\"overwrite\"|\"keepExisting\" (default ask)} adds the pairs to the master. A pair the master has with another value is a conflict: overwrite replaces it, keepExisting leaves it, ask changes nothing when there is one and lists them. Values are rounded half up to whole units and must be from -32767 to 32767. Glyphs and groups the font lacks are pasted and listed. At most 500000 pairs → {master, direction, asked, added, overwritten, kept, unchanged, conflictCount, conflicts: [{left, right, value, existing}], unknownGlyphs, unknownGroups}",
            has_doc,
            paste_pairs
        ),
        cmd!(
            "kerning.deletePairs",
            "Delete Kerning Pairs",
            [],
            None,
            "{pairs?: [{left: name|\"@group\", right: name|\"@group\"}], query?: {term?, match?, kind?, side?, glyphs?} (the pairs kerning.listPairs lists for it; give one of the two), master?: id (default: the first master), allMasters?: bool (default false), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default ltr)} deletes the pairs from the master, or from every master that has them. Each pair must exist in the master (with allMasters: in some master) → {deleted, masters}",
            has_doc,
            delete_pairs
        ),
        cmd!(
            "kerning.setPairs",
            "Set Kerning Pairs",
            [],
            None,
            "{pairs?: [{left: name|\"@group\", right: name|\"@group\", value?: number}], query?: {term?, match?, kind?, side?, glyphs?} (the pairs kerning.listPairs lists for it; give one of the two), value?: number, add?: units, factor?: number (one of the three, or a value in every pair), master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default ltr)} sets the pairs of one master in one undo step: to `value` (or each to its own), each stored value plus `add`, or each stored value times `factor`. Results are rounded half up to whole units and must be from -32767 to 32767. `add` and `factor` need stored pairs; a new pair's glyphs and groups must exist. A refused pair leaves every pair as it was. At most 500000 pairs → {master, direction, count, pairs: [{left, right, value, previous: number|null}]}",
            has_doc,
            set_pairs
        ),
    ]
}

// ---------- parameters ----------

fn direction_param(p: &Params) -> Result<Option<Direction>> {
    match p.opt_str("direction")? {
        None => Ok(None),
        Some(name) => {
            Direction::from_name(name).map(Some).ok_or_else(|| p.bad(format!("`direction` must be ltr, rtl or vertical, got `{}`", clip(name))))
        }
    }
}

/// The master whose table `master` shows, and an error to refuse edits with when it is another.
fn linked_error(p: &Params, font: &Font, master: &str) -> Result<()> {
    match metrics_source(font, master) {
        Some(src) if src.id != master => {
            Err(p.bad(format!("master `{master}` takes its kerning from master `{}` (`{}`); edit that master instead", src.id, src.name)))
        }
        _ => Ok(()),
    }
}

fn source_of(font: &Font, master: &str) -> String {
    metrics_source(font, master).map_or_else(|| master.to_string(), |m| m.id.clone())
}

/// `pairs: [{left, right, value?}]`, capped at [`MAX_PAIRS`]. `value` is read when `with_value`.
fn pairs_param(p: &Params, key: &str, with_value: bool) -> Result<Option<Vec<NewPair>>> {
    let Some(list) = p.opt_array(key)? else { return Ok(None) };
    if list.len() > MAX_PAIRS {
        return Err(p.bad(format!("`{key}` holds {} pairs; at most {MAX_PAIRS} are allowed", list.len())));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, item) in list.iter().enumerate() {
        let Some(obj) = item.as_object() else { return Err(p.bad(format!("`{key}[{i}]` must be an object with left and right"))) };
        let side = |name: &str| -> Result<Side> {
            let text = obj.get(name).and_then(Value::as_str).ok_or_else(|| p.bad(format!("`{key}[{i}].{name}` must be a glyph name or @group")))?;
            Side::parse(text).map_err(|e| p.bad(format!("`{key}[{i}].{name}`: {e}")))
        };
        let (left, right) = (side("left")?, side("right")?);
        let value = if with_value {
            let v = obj.get("value").and_then(Value::as_f64).ok_or_else(|| p.bad(format!("`{key}[{i}].value` must be a number")))?;
            pasted_value(v).ok_or_else(|| p.bad(format!("`{key}[{i}].value` must be from -{MAX_KERN_VALUE} to {MAX_KERN_VALUE}, got {v}")))?
        } else {
            0.0
        };
        out.push(NewPair { left, right, value });
    }
    Ok(Some(out))
}

/// A pasted value as a typed one: rounded half up to a whole unit, within [`MAX_KERN_VALUE`].
fn pasted_value(v: f64) -> Option<f64> {
    let v = round_half_up(v);
    (v.is_finite() && v.abs() <= MAX_KERN_VALUE).then_some(v)
}

fn clip(text: &str) -> String {
    text.chars().take(40).collect()
}

// ---------- kerning.listPairs ----------

fn list_pairs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.listPairs", p)?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let direction = direction_param(&p)?.unwrap_or(Direction::Ltr);
    let filter = filter_param(&p, font)?;
    let order = match p.opt_str("sort")? {
        None => None,
        Some(c) => Some(Column::from_name(c).ok_or_else(|| p.bad(format!("`sort` must be left, value or right, got `{}`", clip(c))))?),
    };
    let descending = p.bool_or("descending", false)?;
    let source = source_of(font, &master);
    let total = pairs::stored_pairs(font, direction, &source).len();
    let listed = pairs::list(font, direction, &source, &filter, order.map(|c| (c, descending)));
    let rows: Vec<Value> =
        listed.iter().map(|r| json!({ "left": r.left().command(), "right": r.right().command(), "value": r.value, "index": r.index })).collect();
    Ok(json!({ "master": master, "source": source, "direction": direction.name(), "total": total, "count": rows.len(), "pairs": rows }))
}

/// The search options of `kerning.listPairs`.
fn filter_param(p: &Params, font: &Font) -> Result<Filter> {
    let term = p.opt_str("term")?.unwrap_or_default();
    if term.len() > MAX_NAME_LEN + 1 {
        return Err(p.bad(format!("`term` may be at most {} bytes long", MAX_NAME_LEN + 1)));
    }
    let matching = match p.opt_str("match")? {
        None => Match::default(),
        Some(m) => Match::from_name(m)
            .ok_or_else(|| p.bad(format!("`match` must be contains, is, beginsWith, endsWith or doesNotContain, got `{}`", clip(m))))?,
    };
    let kind = match p.opt_str("kind")? {
        None => None,
        Some(k) => Some(EntryKind::from_name(k).ok_or_else(|| p.bad(format!("`kind` must be group or notGroup, got `{}`", clip(k))))?),
    };
    let side = match p.opt_str("side")? {
        None => None,
        Some(k) => Some(SearchSide::from_name(k).ok_or_else(|| p.bad(format!("`side` must be left or right, got `{}`", clip(k))))?),
    };
    let glyphs = p.opt_glyphs("glyphs", font)?;
    Ok(Filter { term: term.to_string(), matching, kind, side, glyphs })
}

/// The pairs `pairs` names, or those `query` lists from `source`'s table; `None` when neither is given.
fn rows_param(p: &Params, font: &Font, direction: Direction, source: &str, with_value: bool) -> Result<Option<Vec<NewPair>>> {
    let Some(query) = p.get("query") else { return pairs_param(p, "pairs", with_value) };
    if p.get("pairs").is_some() {
        return Err(p.bad("give `pairs` or `query`, not both"));
    }
    let filter = filter_param(&Params::new(p.command(), query)?, font)?;
    let listed = pairs::list(font, direction, source, &filter, None);
    let mut seen: HashSet<Key> = HashSet::with_capacity(listed.len());
    Ok(Some(
        listed.iter().map(|r| NewPair { left: r.left(), right: r.right(), value: r.value }).filter(|n| seen.insert(key(&n.left, &n.right))).collect(),
    ))
}

// ---------- kerning.copyPairs ----------

/// Side keys that ignore the stored spelling.
type Key = ((bool, String), (bool, String));

fn key(left: &Side, right: &Side) -> Key {
    ((left.is_group(), left.name().to_string()), (right.is_group(), right.name().to_string()))
}

fn copy_pairs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.copyPairs", p)?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let direction = direction_param(&p)?.unwrap_or(Direction::Ltr);
    let source = source_of(font, &master);
    let stored: Vec<NewPair> =
        pairs::stored_pairs(font, direction, &source).iter().map(|r| NewPair { left: r.left(), right: r.right(), value: r.value }).collect();
    let chosen = match rows_param(&p, font, direction, &source, false)? {
        None => stored,
        Some(wanted) => {
            let mut values: HashMap<Key, f64> = HashMap::new();
            for r in &stored {
                values.entry(key(&r.left, &r.right)).or_insert(r.value);
            }
            let mut out = Vec::with_capacity(wanted.len());
            for (i, w) in wanted.into_iter().enumerate() {
                let Some(value) = values.get(&key(&w.left, &w.right)) else {
                    return Err(p.bad(format!("`pairs[{i}]`: master `{master}` has no pair {} {}", w.left.command(), w.right.command())));
                };
                out.push(NewPair { value: *value, ..w });
            }
            out
        }
    };
    let name = font.master(&master)?.name.clone();
    let count = chosen.len();
    let doc = KerningText { master: Some(name), direction: Some(direction), pairs: chosen, ..Default::default() };
    let text = kerning_text::write(&doc).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({ "text": text, "count": count, "master": master, "direction": direction.name() }))
}

// ---------- kerning.pastePairs ----------

#[derive(Clone, Copy, PartialEq)]
enum OnConflict {
    Ask,
    Overwrite,
    KeepExisting,
}

fn paste_pairs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.pastePairs", p)?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let on_conflict = match p.opt_str("onConflict")? {
        None | Some("ask") => OnConflict::Ask,
        Some("overwrite") => OnConflict::Overwrite,
        Some("keepExisting") => OnConflict::KeepExisting,
        Some(other) => return Err(p.bad(format!("`onConflict` must be ask, overwrite or keepExisting, got `{}`", clip(other)))),
    };
    let (pasted, text_direction) = match (p.opt_str("text")?, pairs_param(&p, "pairs", true)?) {
        (Some(_), Some(_)) => return Err(p.bad("give `text` or `pairs`, not both")),
        (None, None) => return Err(p.bad("`text` or `pairs` is required")),
        (None, Some(list)) => (list, None),
        (Some(text), None) => {
            if text.len() > MAX_TEXT_BYTES {
                return Err(p.bad(format!("`text` is {} bytes long; at most {MAX_TEXT_BYTES} are allowed", text.len())));
            }
            let mut doc = kerning_text::read(text).map_err(|e| p.bad(format!("`text`: {e}")))?;
            for pair in &mut doc.pairs {
                pair.value = pasted_value(pair.value).ok_or_else(|| {
                    p.bad(format!(
                        "`text`: the pair {} {} has the value {}; pasted values must be from -{MAX_KERN_VALUE} to {MAX_KERN_VALUE}",
                        pair.left.command(),
                        pair.right.command(),
                        pair.value
                    ))
                })?;
            }
            (doc.pairs, doc.direction)
        }
    };
    if pasted.is_empty() {
        return Err(p.bad("there are no pairs to paste"));
    }
    let direction = direction_param(&p)?.or(text_direction).unwrap_or(Direction::Ltr);
    linked_error(&p, font, &master)?;
    let plan = pairs::plan_paste(font, direction, &master, &pasted);
    let (unknown_glyphs, unknown_groups) = unknown_sides(font, direction, &pasted);
    let conflicts: Vec<Value> = plan
        .conflicts
        .iter()
        .take(MAX_REPORTED)
        .map(|c| json!({ "left": c.left.command(), "right": c.right.command(), "value": c.value, "existing": c.existing }))
        .collect();
    let asked = on_conflict == OnConflict::Ask && !plan.conflicts.is_empty();
    let overwrite = on_conflict == OnConflict::Overwrite;
    if !asked {
        s.edit("Paste Kerning Pairs", |font| {
            let k = Arc::make_mut(&mut font.kerning);
            pairs::apply_paste(direction.table_mut(k), direction, &master, &pasted, &plan, overwrite);
            Ok(())
        })?;
    }
    let (added, overwritten, kept) = match (asked, overwrite) {
        (true, _) => (0, 0, 0),
        (false, true) => (plan.added.len(), plan.conflicts.len(), 0),
        (false, false) => (plan.added.len(), 0, plan.conflicts.len()),
    };
    Ok(json!({
        "master": master,
        "direction": direction.name(),
        "asked": asked,
        "added": added,
        "overwritten": overwritten,
        "kept": kept,
        "unchanged": plan.unchanged.len(),
        "conflictCount": plan.conflicts.len(),
        "conflicts": conflicts,
        "unknownGlyphs": unknown_glyphs,
        "unknownGroups": unknown_groups,
    }))
}

// ---------- kerning.setPairs ----------

/// What `kerning.setPairs` does to each pair.
enum Change {
    /// Each pair gets the value it names.
    Each,
    To(f64),
    Add(f64),
    Times(f64),
}

fn change_param(p: &Params, list: &[Value]) -> Result<Change> {
    let given: Vec<Change> = [
        p.opt_f64("value")?.map(Change::To),
        p.opt_f64("add")?.map(Change::Add),
        p.opt_f64("factor")?.map(Change::Times),
        list.iter().any(|pair| pair.get("value").is_some()).then_some(Change::Each),
    ]
    .into_iter()
    .flatten()
    .collect();
    match <[Change; 1]>::try_from(given) {
        Ok([one]) => Ok(one),
        Err(_) => Err(p.bad("give one of `value`, `add` and `factor`, or a value in every pair")),
    }
}

fn set_pairs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.setPairs", p)?;
    let font = s.font()?;
    let master = editable_master(&p, font)?;
    let direction = direction_param(&p)?.unwrap_or(Direction::Ltr);
    let change = change_param(&p, p.opt_array("pairs")?.map(Vec::as_slice).unwrap_or_default())?;
    let wanted = rows_param(&p, font, direction, &master, matches!(change, Change::Each))?.ok_or_else(|| p.bad("`pairs` or `query` is required"))?;
    if wanted.is_empty() {
        return Err(p.bad("there are no pairs to set"));
    }
    if wanted.len() > MAX_PAIRS {
        return Err(p.bad(format!("the query lists {} pairs; at most {MAX_PAIRS} are allowed", wanted.len())));
    }
    let mut at: HashMap<Key, usize> = HashMap::with_capacity(wanted.len());
    for (i, w) in wanted.iter().enumerate() {
        if at.insert(key(&w.left, &w.right), i).is_some() {
            return Err(p.bad(format!("`pairs[{i}]`: the pair {} {} is given twice", w.left.command(), w.right.command())));
        }
    }
    // The stored pairs among them, the first spelling of each; a stored pair may be obsolete.
    let mut previous: Vec<Option<f64>> = vec![None; wanted.len()];
    for row in pairs::stored_pairs(font, direction, &master) {
        if let Some(slot) = at.get(&key(&row.left(), &row.right())).and_then(|i| previous.get_mut(*i))
            && slot.is_none()
        {
            *slot = Some(row.value);
        }
    }
    let mut values = Vec::with_capacity(wanted.len());
    let mut new = Vec::new();
    for (i, (w, before)) in wanted.iter().zip(&previous).enumerate() {
        let value = match (&change, before) {
            (Change::Each, _) => w.value,
            (Change::To(v), _) => *v,
            (Change::Add(by), Some(v)) => v + by,
            (Change::Times(by), Some(v)) => v * by,
            (Change::Add(_) | Change::Times(_), None) => {
                return Err(p.bad(format!("`pairs[{i}]`: master `{master}` has no pair {} {}", w.left.command(), w.right.command())));
            }
        };
        let value = pasted_value(value).ok_or_else(|| {
            let pair = format!("{} {}", w.left.command(), w.right.command());
            p.bad(format!("`pairs[{i}]`: {pair} would be {value}; values must be from -{MAX_KERN_VALUE} to {MAX_KERN_VALUE}"))
        })?;
        values.push(value);
        if before.is_none() {
            new.push(NewPair { left: w.left.clone(), right: w.right.clone(), value });
        }
    }
    let (glyphs, groups) = unknown_sides(font, direction, &new);
    if let Some(missing) = glyphs.first() {
        return Err(p.bad(format!("the font has no glyph `{missing}`")));
    }
    if let Some(missing) = groups.first() {
        return Err(p.bad(format!("no glyph is in the kerning group `{missing}` on that side")));
    }
    s.edit("Set Kerning Pairs", |font| {
        let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
        let mut open: HashMap<Key, f64> = HashMap::with_capacity(values.len());
        for (w, value) in wanted.iter().zip(&values) {
            open.insert(key(&w.left, &w.right), *value);
        }
        for m in table.0.iter_mut().filter(|m| m.master == master) {
            for first in &mut m.firsts {
                let left = Side::from_stored(&first.first);
                for (second, slot) in &mut first.seconds {
                    if let Some(value) = open.remove(&key(&left, &Side::from_stored(second))) {
                        *slot = value;
                    }
                }
            }
        }
        for pair in &new {
            table.set(&master, &pair.left.stored(direction, Position::First), &pair.right.stored(direction, Position::Second), pair.value);
        }
        Ok(())
    })?;
    let rows: Vec<Value> = wanted
        .iter()
        .zip(&values)
        .zip(&previous)
        .map(|((w, value), before)| json!({ "left": w.left.command(), "right": w.right.command(), "value": value, "previous": before }))
        .collect();
    Ok(json!({ "master": master, "direction": direction.name(), "count": rows.len(), "pairs": rows }))
}

/// The glyph sides the font has no glyph for, and the group sides no glyph of the font belongs
/// to on the side facing the pair (`@name`), each at most [`MAX_REPORTED`], in paste order.
fn unknown_sides(font: &Font, direction: Direction, pasted: &[NewPair]) -> (Vec<String>, Vec<String>) {
    let mut groups: HashSet<(GroupSide, &str)> = HashSet::new();
    for g in font.glyphs.iter() {
        for side in GroupSide::ALL {
            if let Some(name) = side.of(&g.kern_groups) {
                groups.insert((side, name));
            }
        }
    }
    let (mut glyphs_out, mut groups_out) = (Vec::new(), Vec::new());
    let mut seen: HashSet<String> = HashSet::new();
    for pair in pasted {
        for (side, position) in [(&pair.left, Position::First), (&pair.right, Position::Second)] {
            let missing = match side {
                Side::Glyph(name) => font.glyph(name).is_none(),
                Side::Group(name) => !groups.contains(&(facing(direction, position), name.as_str())),
            };
            if !missing {
                continue;
            }
            let list = if side.is_group() { &mut groups_out } else { &mut glyphs_out };
            if list.len() < MAX_REPORTED && seen.insert(side.command()) {
                list.push(side.command());
            }
        }
    }
    (glyphs_out, groups_out)
}

// ---------- kerning.deletePairs ----------

fn delete_pairs(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.deletePairs", p)?;
    let font = s.font()?;
    let master = super::shared::master(&p, font)?;
    let direction = direction_param(&p)?.unwrap_or(Direction::Ltr);
    let all = p.bool_or("allMasters", false)?;
    let wanted = rows_param(&p, font, direction, &source_of(font, &master), false)?.ok_or_else(|| p.bad("`pairs` or `query` is required"))?;
    if wanted.is_empty() {
        return Err(p.bad("there are no pairs to delete"));
    }
    let sides: Vec<(Side, Side)> = wanted.into_iter().map(|w| (w.left, w.right)).collect();
    let masters: Vec<String> = if all {
        font.masters.iter().map(|m| m.id.clone()).collect()
    } else {
        linked_error(&p, font, &master)?;
        vec![master.clone()]
    };
    let (deleted, touched) = s.edit("Delete Kerning Pairs", |font| {
        let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
        let mut found_any = vec![false; sides.len()];
        let mut deleted = 0usize;
        let mut touched = Vec::new();
        for m in &masters {
            let found = pairs::delete_pairs(table, m, &sides);
            let n = found.iter().filter(|f| **f).count();
            if n > 0 {
                touched.push(m.clone());
            }
            deleted += n;
            for (any, f) in found_any.iter_mut().zip(&found) {
                *any |= *f;
            }
        }
        if let Some(i) = found_any.iter().position(|f| !f) {
            let (l, r) = sides.get(i).map(|(l, r)| (l.command(), r.command())).unwrap_or_default();
            let place = if all { "no master has".to_string() } else { format!("master `{master}` has no") };
            return Err(p.bad(format!("`pairs[{i}]`: {place} pair {l} {r}")));
        }
        Ok((deleted, touched))
    })?;
    Ok(json!({ "deleted": deleted, "masters": touched }))
}
