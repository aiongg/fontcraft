//! Kerning audit and cross-master matching. Rows: `kerning.auditExceptions`,
//! `kerning.matchAcrossMasters`. Plan: `docs/plans/M13.md`, package M13.21b.
//!
//! # Audit Kerning Exceptions
//! A class pair is a stored pair with at least one group side. The audit expands each class pair
//! to the glyph pairs it applies to (glyph pairs that a more specific stored pair covers are left
//! out) and measures each one's closest approach with the class value applied: Kern to
//! Distance's exact distance (`autokern::Kit::closest`) plus the value. A pair closer than
//! `threshold` is reported; with `fix`, a glyph–glyph exception moves it out to the threshold.
//!
//! # Match Kerning
//! Variable kerning needs every master to store the same pairs. Groups belong to glyphs in this
//! model, so every master already shares them. Each master is given the union of the pairs the
//! matched masters store (sides that name no glyph and no group member are left out), and no
//! glyph pair's value changes in any master:
//!
//! - Under the union, each glyph pair is decided by exactly one stored pair, the same in every
//!   master. A master that stores that pair already gives its own value to every glyph pair it
//!   decides.
//! - A master that lacks the pair gets the values most of those glyph pairs have across the
//!   masters (ties to the first in glyph order); with no glyph pair left to decide, 0.
//! - A glyph pair whose values differ from the pair's gets a glyph–glyph exception in every
//!   master. An exception decides only its own glyph pair, so one pass is enough.
//!
//! The result is what flattening every master to glyph pairs, filling missing pairs with 0 and
//! compressing across all masters at once gives, without building the flat table.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_spacing::SIDEBEARING_RANGE;
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, Side, Step, Table, facing};
use serde_json::{Map, Value, json};

use super::autokern::{Kit, MAX_PAIRS, horizontal, linked, read_pairs};
use super::autospace::{master_ids, opt_in};
use super::shared::MAX_KERN_VALUE;
use super::*;
use crate::params::Params;

/// Most glyph pairs one audit measures, over all its masters.
pub const MAX_AUDIT_PAIRS: usize = MAX_PAIRS;
/// Most glyph pairs one match looks at in one direction: the expansions of the stored pairs of
/// every matched master, each counted once.
pub const MAX_MATCH_PAIRS: usize = 4_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "kerning.auditExceptions",
            "Audit Kerning Exceptions",
            ["Font", "Kerning"],
            None,
            "{pairs?: [[left, right]] (glyph names or \"@group\", logical order; a group stands for its members), glyphsLeft?: [names], glyphsRight?: [names] (every pair of the two lists; default without pairs or lists: every class pair stored in the master), masters?: [master ids] (default: every master), direction?: \"ltr\" | \"rtl\" (default ltr), threshold?: units (default 0), fix?: bool (default false)} expand class kerning (stored pairs with a group side) to the glyph pairs it applies to, and measure each one's closest approach with the class value applied: the smallest horizontal distance between the two outlines at any height both reach. Pairs closer than threshold are reported (with the default, pairs whose outlines overlap). With fix, each reported pair gets a glyph–glyph exception that moves it out to the threshold, rounded up to a whole unit, in one undo step. Glyph pairs a glyph–glyph pair already covers are not measured. Masters that take their kerning from another are skipped → {pairs: [{master, left, right, classLeft, classRight, value, distance, fix?, reason?}], measured, skipped: [{master, left?, right?, reason}]}",
            has_doc,
            audit_exceptions
        ),
        cmd!(
            "kerning.matchAcrossMasters",
            "Match Kerning",
            ["Font", "Kerning"],
            None,
            "{masters?: [master ids] (default: every master; at least two with kerning of their own), mainMaster?: master id (default: the font's default master, else the first matched one)} give every matched master the same stored pairs in every direction, so kerning interpolates, without changing the kerning between any two glyphs: a master that lacks a pair another stores gets it, with the value most of the glyph pairs it decides have there (0 when none), and a glyph pair that would then change gets a glyph–glyph exception in every master. Added pairs take the main master's spelling, else the first spelling stored. Groups belong to glyphs, so every master shares them already. One undo step. Masters that take their kerning from another are skipped → {pairs: [{master, direction, left, right, value, kind: \"filled\" | \"exception\"}], masters: {master id: {before, after}}, skipped: [{master, reason}]}",
            has_doc,
            match_across_masters
        ),
    ]
}

// ── Shared ─────────────────────────────────────────────────────────────────────────────────────

/// The glyphs a side stands for at `position`: the glyph itself when the font has it, else the
/// members of the group facing the pair, in glyph order.
fn expand<'a>(index: &'a KernIndex, side: &'a Side, direction: Direction, position: Position) -> Vec<&'a str> {
    match side {
        Side::Glyph(n) => index.glyph_id(n).map(|_| n.as_str()).into_iter().collect(),
        Side::Group(g) => index.members(facing(direction, position), g).collect(),
    }
}

/// How many glyphs [`expand`] gives.
fn expand_count(index: &KernIndex, side: &Side, direction: Direction, position: Position) -> usize {
    match side {
        Side::Glyph(n) => usize::from(index.glyph_id(n).is_some()),
        Side::Group(g) => index.members(facing(direction, position), g).count(),
    }
}

/// How many glyph pairs a pair of sides stands for.
fn pair_count(index: &KernIndex, first: &Side, second: &Side, direction: Direction) -> usize {
    expand_count(index, first, direction, Position::First).saturating_mul(expand_count(index, second, direction, Position::Second))
}

// ── Audit ──────────────────────────────────────────────────────────────────────────────────────

/// A glyph pair under class kerning: its glyphs in logical order, the value that applies and the
/// class pair it comes from.
struct Audited<'a> {
    first: &'a str,
    second: &'a str,
    value: f64,
    class: (Side, Side),
}

fn audit_exceptions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.auditExceptions", v)?;
    let direction = horizontal(&p)?;
    let threshold = opt_in(&p, "threshold", SIDEBEARING_RANGE)?.unwrap_or(0.0);
    let fix = p.bool_or("fix", false)?;
    let font = s.font()?;
    let index = KernIndex::new(font);
    let given = read_pairs(&p, font, &index, direction)?;
    if given.as_ref().is_some_and(Vec::is_empty) {
        return Err(p.bad("there are no pairs to audit"));
    }
    let masters = master_ids(&p, font)?;
    let mut skipped = Vec::new();
    // Per master, the pairs of sides to expand.
    let mut plans: Vec<(&str, Vec<(Side, Side)>)> = Vec::new();
    let mut total = 0usize;
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        let sides: Vec<(Side, Side)> = match &given {
            Some(list) => list.clone(),
            None => {
                let mut seen = HashSet::new();
                stored_pairs(font, direction, id)
                    .iter()
                    .map(|lp| (lp.left(), lp.right()))
                    .filter(|(a, b)| (a.is_group() || b.is_group()) && seen.insert((a.clone(), b.clone())))
                    .collect()
            }
        };
        for (a, b) in &sides {
            total = total.saturating_add(pair_count(&index, a, b, direction));
        }
        if total > MAX_AUDIT_PAIRS {
            return Err(p.bad(format!("the pairs expand to more than {MAX_AUDIT_PAIRS} glyph pairs; give `pairs` or fewer `masters`")));
        }
        plans.push((id.as_str(), sides));
    }
    let mut found = Vec::new();
    // Exceptions to store: master, first glyph, second glyph, value.
    let mut changes: Vec<(String, String, String, f64)> = Vec::new();
    let mut measured = 0usize;
    for (id, sides) in &plans {
        let m = font.master(id)?;
        let table = index.table(direction, id).map_err(|e| p.bad(e.to_string()))?;
        let mut seen = HashSet::new();
        let mut jobs: Vec<Audited<'_>> = Vec::new();
        for (a, b) in sides {
            let seconds = expand(&index, b, direction, Position::Second);
            for first in expand(&index, a, direction, Position::First) {
                for &second in &seconds {
                    let (Some(fi), Some(si)) = (index.glyph_id(first), index.glyph_id(second)) else { continue };
                    let Some(applied) = table.lookup(fi, si) else { continue };
                    if applied.step == Step::GlyphGlyph {
                        continue;
                    }
                    let class = (Side::from_stored(applied.first), Side::from_stored(applied.second));
                    // Without `pairs`, every glyph pair is met under the one class pair that
                    // decides it; given pairs can name a glyph pair twice.
                    let keep = match &given {
                        None => &class.0 == a && &class.1 == b,
                        Some(_) => seen.insert((fi, si)),
                    };
                    if keep {
                        jobs.push(Audited { first, second, value: applied.value, class });
                    }
                }
            }
        }
        if jobs.is_empty() {
            continue;
        }
        let mut names = HashSet::new();
        let mut needs = Vec::new();
        for j in &jobs {
            for n in [j.first, j.second] {
                if names.insert(n) {
                    needs.push((n.to_string(), 0));
                }
            }
        }
        let kit = Kit::new(font, m, None, None, 0.0, &needs, true).map_err(|e| p.bad(e))?;
        for j in &jobs {
            let (left, right) = if direction == Direction::Rtl { (j.second, j.first) } else { (j.first, j.second) };
            let gap = match kit.closest(left, right) {
                Ok(g) => g,
                Err(reason) => {
                    skipped.push(json!({"master": id, "left": j.first, "right": j.second, "reason": reason}));
                    continue;
                }
            };
            measured = measured.saturating_add(1);
            let distance = gap + j.value;
            if distance.is_nan() || distance >= threshold {
                continue;
            }
            let mut entry = Map::new();
            entry.insert("master".into(), json!(id));
            entry.insert("left".into(), json!(j.first));
            entry.insert("right".into(), json!(j.second));
            entry.insert("classLeft".into(), json!(j.class.0.command()));
            entry.insert("classRight".into(), json!(j.class.1.command()));
            entry.insert("value".into(), json!(j.value));
            entry.insert("distance".into(), json!(distance));
            // Snapped to a millionth first, so float noise cannot round a whole value up by one.
            let needed = (((threshold - gap) * 1e6).round() / 1e6).ceil();
            if needed.is_finite() && needed.abs() <= MAX_KERN_VALUE {
                entry.insert("fix".into(), json!(needed));
                if fix {
                    changes.push((id.to_string(), j.first.to_string(), j.second.to_string(), needed));
                }
            } else {
                entry.insert("reason".into(), json!(format!("the exception would be {needed}, beyond ±{MAX_KERN_VALUE}")));
            }
            found.push(Value::Object(entry));
        }
    }
    if fix && !changes.is_empty() {
        s.edit("Audit Kerning Exceptions", |font| {
            let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
            for (master, first, second, value) in &changes {
                table.set(master, first, second, *value);
            }
            Ok(())
        })?;
    }
    Ok(json!({"pairs": found, "measured": measured, "skipped": skipped}))
}

// ── Match ──────────────────────────────────────────────────────────────────────────────────────

/// A side by id: a glyph by its index id, or a group by its place in the run's group list.
type SideKey = (bool, usize);

/// One pair of the union: its sides, and their spelling where it is first stored.
struct UnionPair {
    first: Side,
    second: Side,
    first_stored: String,
    second_stored: String,
}

/// A glyph pair the union pair decides, with the value that applies to it in each master.
struct Decided<'a> {
    first: &'a str,
    second: &'a str,
    values: Vec<f64>,
}

/// The values most of `rows` have, ties to the first; `None` without rows.
fn most_common(rows: &[Decided<'_>]) -> Option<Vec<f64>> {
    // Per distinct value list: how many rows have it, and the first of them.
    let mut counts: HashMap<Vec<u64>, (usize, usize)> = HashMap::new();
    for (i, row) in rows.iter().enumerate() {
        // Adding 0 turns −0 into 0, so both count as one value.
        let key = row.values.iter().map(|v| (v + 0.0).to_bits()).collect();
        let slot = counts.entry(key).or_insert((0, i));
        slot.0 = slot.0.saturating_add(1);
    }
    let (_, (_, first)) = counts.into_iter().max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.1.1.cmp(&a.1.1)))?;
    rows.get(first).map(|r| r.values.clone())
}

/// Stored pairs per master over every direction.
fn pair_counts(font: &Font, masters: &[String]) -> Map<String, Value> {
    masters
        .iter()
        .map(|id| {
            let n: usize = Direction::ALL
                .iter()
                .flat_map(|d| &d.table(&font.kerning).0)
                .filter(|m| m.master == *id)
                .flat_map(|m| &m.firsts)
                .map(|f| f.seconds.len())
                .sum();
            (id.clone(), json!(n))
        })
        .collect()
}

fn match_across_masters(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.matchAcrossMasters", v)?;
    let font = s.font()?;
    let index = KernIndex::new(font);
    let masters = master_ids(&p, font)?;
    let main = p.opt_str("mainMaster")?;
    if let Some(id) = main
        && !masters.iter().any(|m| m == id)
    {
        return Err(p.bad(format!("`mainMaster`: `{id}` is not one of the masters to match")));
    }
    let mut skipped = Vec::new();
    let mut own: Vec<String> = Vec::new();
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            if main == Some(id.as_str()) {
                return Err(p.bad(format!("`mainMaster`: {reason}")));
            }
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        own.push(id.clone());
    }
    if own.len() < 2 {
        return Err(p.bad("matching needs at least two masters with kerning of their own"));
    }
    let default = font.default_master().ok().map(|m| m.id.as_str());
    let main = main.or(default).filter(|id| own.iter().any(|m| m.as_str() == *id)).map(str::to_string);
    if let Some(at) = main.and_then(|id| own.iter().position(|m| *m == id)) {
        let id = own.remove(at);
        own.insert(0, id);
    }
    let before = pair_counts(font, &own);
    let mut out = Vec::new();
    // Pairs to store: direction, master, sides as stored, value.
    let mut changes: Vec<(Direction, String, String, String, f64)> = Vec::new();
    let mut added: HashMap<&str, usize> = HashMap::new();
    for direction in Direction::ALL {
        let tables: Vec<Table<'_>> =
            own.iter().map(|id| index.table(direction, id)).collect::<std::result::Result<_, _>>().map_err(|e| p.bad(e.to_string()))?;
        // The union of the stored pairs that can apply, main master first, in stored order.
        let mut union: Vec<UnionPair> = Vec::new();
        let mut at: HashMap<(Side, Side), usize> = HashMap::new();
        let mut stores: Vec<HashSet<usize>> = vec![HashSet::new(); own.len()];
        let mut total = 0usize;
        for (mi, id) in own.iter().enumerate() {
            for lp in stored_pairs(font, direction, id) {
                if !lp.value.is_finite() {
                    continue;
                }
                let key = (lp.left(), lp.right());
                let ui = match at.get(&key) {
                    Some(ui) => *ui,
                    None => {
                        let n = pair_count(&index, &key.0, &key.1, direction);
                        if n == 0 {
                            continue;
                        }
                        total = total.saturating_add(n);
                        if total > MAX_MATCH_PAIRS {
                            return Err(p.bad(format!(
                                "the {} pairs stand for more than {MAX_MATCH_PAIRS} glyph pairs; match fewer masters or compress the kerning first",
                                direction.name()
                            )));
                        }
                        union.push(UnionPair {
                            first: key.0.clone(),
                            second: key.1.clone(),
                            first_stored: lp.first.to_string(),
                            second_stored: lp.second.to_string(),
                        });
                        let ui = union.len().saturating_sub(1);
                        at.insert(key, ui);
                        ui
                    }
                };
                if let Some(set) = stores.get_mut(mi) {
                    set.insert(ui);
                }
            }
        }
        if union.is_empty() {
            continue;
        }
        // Side keys of the union, to find the pair that decides a glyph pair without strings.
        let side_key = |side: &Side, groups: &mut HashMap<String, usize>| -> Option<SideKey> {
            match side {
                Side::Glyph(n) => index.glyph_id(n).map(|id| (false, id)),
                Side::Group(g) => {
                    let next = groups.len();
                    Some((true, *groups.entry(g.clone()).or_insert(next)))
                }
            }
        };
        let mut groups: HashMap<String, usize> = HashMap::new();
        let mut decides: HashMap<(SideKey, SideKey), usize> = HashMap::new();
        for (ui, u) in union.iter().enumerate() {
            if let (Some(a), Some(b)) = (side_key(&u.first, &mut groups), side_key(&u.second, &mut groups)) {
                decides.insert((a, b), ui);
            }
        }
        let (ff, fs) = (facing(direction, Position::First), facing(direction, Position::Second));
        let group_of = |glyph: &str, side| index.group(glyph, side).and_then(|g| groups.get(g)).map(|id| (true, *id));
        let decider = |first: &str, second: &str| -> Option<usize> {
            let f = (false, index.glyph_id(first)?);
            let s = (false, index.glyph_id(second)?);
            let (fg, sg) = (group_of(first, ff), group_of(second, fs));
            [Some((f, s)), sg.map(|sg| (f, sg)), fg.map(|fg| (fg, s)), fg.zip(sg)].into_iter().flatten().find_map(|k| decides.get(&k).copied())
        };
        for (ui, u) in union.iter().enumerate() {
            let seconds = expand(&index, &u.second, direction, Position::Second);
            let mut rows: Vec<Decided<'_>> = Vec::new();
            for first in expand(&index, &u.first, direction, Position::First) {
                for &second in &seconds {
                    if decider(first, second) != Some(ui) {
                        continue;
                    }
                    let (Some(fi), Some(si)) = (index.glyph_id(first), index.glyph_id(second)) else { continue };
                    let values = tables.iter().map(|t| t.lookup(fi, si).map_or(0.0, |a| a.value)).collect();
                    rows.push(Decided { first, second, values });
                }
            }
            let reference = most_common(&rows).unwrap_or_else(|| vec![0.0; own.len()]);
            for (mi, id) in own.iter().enumerate() {
                if stores.get(mi).is_some_and(|set| set.contains(&ui)) {
                    continue;
                }
                let value = reference.get(mi).copied().unwrap_or(0.0);
                changes.push((direction, id.clone(), u.first_stored.clone(), u.second_stored.clone(), value));
                out.push(json!({
                    "master": id, "direction": direction.name(), "left": u.first.command(), "right": u.second.command(),
                    "value": value, "kind": "filled",
                }));
                let n = added.entry(id.as_str()).or_insert(0);
                *n = n.saturating_add(1);
            }
            for row in rows.iter().filter(|r| r.values != reference) {
                for (id, value) in own.iter().zip(&row.values) {
                    changes.push((direction, id.clone(), row.first.to_string(), row.second.to_string(), *value));
                    out.push(json!({
                        "master": id, "direction": direction.name(), "left": row.first, "right": row.second,
                        "value": value, "kind": "exception",
                    }));
                    let n = added.entry(id.as_str()).or_insert(0);
                    *n = n.saturating_add(1);
                }
            }
        }
    }
    let counts: Map<String, Value> = before
        .into_iter()
        .map(|(id, n)| {
            let b = n.as_u64().unwrap_or(0);
            let a = b.saturating_add(added.get(id.as_str()).map_or(0, |n| u64::try_from(*n).unwrap_or(u64::MAX)));
            (id, json!({"before": b, "after": a}))
        })
        .collect();
    if !changes.is_empty() {
        s.edit("Match Kerning", |font| {
            let kerning = Arc::make_mut(&mut font.kerning);
            for (direction, master, first, second, value) in &changes {
                direction.table_mut(kerning).set(master, first, second, *value);
            }
            Ok(())
        })?;
    }
    Ok(json!({"pairs": out, "masters": counts, "skipped": skipped}))
}
