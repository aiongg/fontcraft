//! Kerning pairs and exceptions: set, remove and read a pair, and open or close the locks of the
//! pair between two glyphs. Behaviour: `docs/spec/08-spacing-kerning.md` § 3.3, § 3.6, § 3.7.
//!
//! Every command takes `master` (default: the first master) and `direction` (`ltr`, `rtl` or
//! `vertical`; default `ltr`). Sides are in logical order, a glyph name or `@group`
//! (`fontcraft_spacing::kerning` › Sides). Commands that change kerning refuse a master whose
//! metrics are linked to another one, and name the master it follows.

use fontcraft_model::{Font, KernTable};
use fontcraft_spacing::kerning::{Applied, Direction, KernError, KernIndex, Lock, Position, Side, Which, facing, locks, stored_pair};
use fontcraft_spacing::{metrics_source, round_half_up};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Largest kerning value a command stores, either sign: what a compiled pair value can hold.
pub(crate) const MAX_KERN_VALUE: f64 = 32_767.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "kerning.setPair",
            "Set Kerning Pair",
            [],
            None,
            "{left: glyph name or \"@group\", right: glyph name or \"@group\", value: number, master?: master id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default \"ltr\")} store the kerning of a pair, sides in logical order; the value is rounded to a whole unit, from -32767 to 32767; a pair already stored keeps its spelling → {left, right, value, previous: number|null, master, direction}",
            has_doc,
            set_pair
        ),
        cmd!(
            "kerning.removePair",
            "Remove Kerning Pair",
            [],
            None,
            "{left: glyph name or \"@group\", right: glyph name or \"@group\", master?: master id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default \"ltr\")} delete one stored pair from a master → {left, right, value: the value it had, master, direction}",
            has_doc,
            remove_pair
        ),
        cmd!(
            query "kerning.getPair",
            "Get Kerning",
            [],
            None,
            "{left: glyph name, right: glyph name, master?: master id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default \"ltr\")} the kerning between two glyphs in logical order: the stored pair that applies (glyph–glyph, then glyph–group, group–glyph, group–group) and the locks of both sides → {value, step: \"glyphGlyph\"|\"glyphGroup\"|\"groupGlyph\"|\"groupGroup\"|null, pair: {left, right}|null, from: master id whose pair applies, locks: {left, right: \"closed\"|\"open\"|\"ungrouped\"}, groups: {left, right: name|null}, master, direction}",
            has_doc,
            get_pair
        ),
        cmd!(
            "kerning.addException",
            "Open Kerning Lock",
            [],
            None,
            "{left: glyph name, right: glyph name, side: \"left\"|\"right\"|\"both\", value?: number (default: the kerning that applies), master?: master id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default \"ltr\")} open the closed locks named by `side`: store the pair they name, so that edits of these two glyphs go to the glyph instead of its group; without `value` the kerning does not change → {left, right, value, master, direction}",
            has_doc,
            add_exception
        ),
        cmd!(
            "kerning.removeException",
            "Close Kerning Lock",
            [],
            None,
            "{left: glyph name, right: glyph name, side: \"left\"|\"right\"|\"both\", master?: master id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default \"ltr\")} close the open locks named by `side`: delete the exception that applies in this master, so the group kerning applies again → {removed: {left, right, value}, value: the kerning that applies now, master, direction}",
            has_doc,
            remove_exception
        ),
    ]
}

// ---------- shared parameter readers ----------

/// `direction`, `ltr` when absent.
pub(crate) fn direction(p: &Params) -> Result<Direction> {
    match p.opt_str("direction")? {
        None => Ok(Direction::Ltr),
        Some(name) => {
            Direction::from_name(name).ok_or_else(|| p.bad(format!("`direction` must be \"ltr\", \"rtl\" or \"vertical\", got \"{name}\"")))
        }
    }
}

/// `master`, the first master when absent.
pub(crate) fn master(p: &Params, font: &Font) -> Result<String> {
    match p.opt_str("master")? {
        Some(id) => font.master(id).map(|m| m.id.clone()).map_err(|_| p.bad(format!("the font has no master `{id}`"))),
        None => Ok(font.default_master()?.id.clone()),
    }
}

/// [`master`], refused when the master takes its kerning from another one.
pub(crate) fn editable_master(p: &Params, font: &Font) -> Result<String> {
    let id = master(p, font)?;
    match metrics_source(font, &id) {
        Some(source) if source.id != id => {
            Err(p.bad(format!("master `{id}` takes its metrics and kerning from master `{}` ({}); edit that master instead", source.id, source.name)))
        }
        _ => Ok(id),
    }
}

/// A command side: a glyph of the font or `@group`. A group must be the group of some glyph on
/// the side that faces the pair at `position`.
fn side(p: &Params, font: &Font, key: &str, direction: Direction, position: Position) -> Result<Side> {
    let text = p.str(key)?;
    let side = Side::parse(text).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    match &side {
        Side::Glyph(name) if font.glyph(name).is_none() => Err(p.bad(format!("`{key}`: the font has no glyph `{name}`"))),
        Side::Group(name) => {
            let facing = facing(direction, position);
            if font.glyphs.iter().any(|g| facing.of(&g.kern_groups) == Some(name.as_str())) {
                Ok(side)
            } else {
                Err(p.bad(format!("`{key}`: no glyph has the {} kerning group `{name}`", facing.name())))
            }
        }
        Side::Glyph(_) => Ok(side),
    }
}

/// A glyph name for the commands that take the two glyphs of a pair.
fn glyph<'p>(p: &Params<'p>, font: &Font, key: &str) -> Result<&'p str> {
    let name = p.str(key)?;
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("`{key}`: the font has no glyph `{name}`")));
    }
    Ok(name)
}

/// A typed kerning value: rounded half up to a whole unit, within [`MAX_KERN_VALUE`].
fn typed_value(p: &Params, key: &str) -> Result<Option<f64>> {
    let Some(v) = p.opt_f64(key)? else { return Ok(None) };
    let v = round_half_up(v);
    if v.abs() > MAX_KERN_VALUE {
        return Err(p.bad(format!("`{key}` must be between -{MAX_KERN_VALUE} and {MAX_KERN_VALUE}, got {v}")));
    }
    Ok(Some(v))
}

fn which(p: &Params) -> Result<Which> {
    let name = p.str("side")?;
    Which::from_name(name).ok_or_else(|| p.bad(format!("`side` must be \"left\", \"right\" or \"both\", got \"{name}\"")))
}

fn kern_error(p: &Params, e: KernError) -> EngineError {
    p.bad(e.to_string())
}

fn lock_name(lock: Lock) -> &'static str {
    match lock {
        Lock::Closed => "closed",
        Lock::Open => "open",
        Lock::Ungrouped => "ungrouped",
    }
}

// ---------- editing stored pairs ----------

/// Stores `value` for the pair `first`, `second` in `master`: in the spelling already stored when
/// the pair exists (in any of the master's entries), else in the spelling for its positions.
/// Returns the value it replaced.
fn store(font: &mut Font, direction: Direction, master: &str, first: &Side, second: &Side, value: f64) -> Option<f64> {
    let existing = stored_pair(font, direction, master, first, second).map(|s| (s.first.to_string(), s.second.to_string(), s.value));
    let kerning = std::sync::Arc::make_mut(&mut font.kerning);
    let table = direction.table_mut(kerning);
    match existing {
        Some((f, s, previous)) => {
            let slot = table
                .0
                .iter_mut()
                .filter(|m| m.master == master)
                .flat_map(|m| m.firsts.iter_mut())
                .filter(|k| k.first == f)
                .flat_map(|k| k.seconds.iter_mut())
                .find(|(name, _)| *name == s);
            match slot {
                Some(slot) => slot.1 = value,
                None => table.set(master, &f, &s, value),
            }
            Some(previous)
        }
        None => {
            table.set(master, &first.stored(direction, Position::First), &second.stored(direction, Position::Second), value);
            None
        }
    }
}

/// Removes the pair stored as `first`, `second` (stored spelling) from `master`'s entries in the
/// table, dropping first sides and master entries left empty. Returns its value.
fn unstore(table: &mut KernTable, master: &str, first: &str, second: &str) -> Option<f64> {
    for mi in 0..table.0.len() {
        let Some(m) = table.0.get_mut(mi).filter(|m| m.master == master) else { continue };
        let Some(fi) = m.firsts.iter().position(|k| k.first == first) else { continue };
        let Some(k) = m.firsts.get_mut(fi) else { continue };
        let Some(si) = k.seconds.iter().position(|(s, _)| s == second) else { continue };
        let (_, value) = k.seconds.remove(si);
        if k.seconds.is_empty() {
            m.firsts.remove(fi);
        }
        if m.firsts.is_empty() {
            table.0.remove(mi);
        }
        return Some(value);
    }
    None
}

// ---------- commands ----------

fn set_pair(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.setPair", p)?;
    let font = s.font()?;
    let direction = direction(&p)?;
    let master = editable_master(&p, font)?;
    let left = side(&p, font, "left", direction, Position::First)?;
    let right = side(&p, font, "right", direction, Position::Second)?;
    let value = typed_value(&p, "value")?.ok_or_else(|| p.bad("`value` is required"))?;
    let previous = s.edit("Set Kerning Pair", |font| Ok(store(font, direction, &master, &left, &right, value)))?;
    Ok(json!({
        "left": left.command(), "right": right.command(), "value": value, "previous": previous,
        "master": master, "direction": direction.name(),
    }))
}

fn remove_pair(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.removePair", p)?;
    let font = s.font()?;
    let direction = direction(&p)?;
    let master = editable_master(&p, font)?;
    let left = Side::parse(p.str("left")?).map_err(|e| p.bad(format!("`left`: {e}")))?;
    let right = Side::parse(p.str("right")?).map_err(|e| p.bad(format!("`right`: {e}")))?;
    let (first, second) = stored_pair(font, direction, &master, &left, &right)
        .map(|s| (s.first.to_string(), s.second.to_string()))
        .ok_or_else(|| p.bad(format!("master `{master}` has no {} pair {} {}", direction.name(), left.command(), right.command())))?;
    let value = s.edit("Remove Kerning Pair", |font| {
        let table = direction.table_mut(std::sync::Arc::make_mut(&mut font.kerning));
        unstore(table, &master, &first, &second).ok_or_else(|| EngineError::Internal(format!("the pair {first} {second} was not found")))
    })?;
    Ok(json!({
        "left": left.command(), "right": right.command(), "value": value,
        "master": master, "direction": direction.name(),
    }))
}

fn applied_json(a: Option<&Applied<'_>>) -> (Value, Value, Value, Value) {
    match a {
        Some(a) => (
            json!(a.value),
            json!(a.step.name()),
            json!({"left": Side::from_stored(a.first).command(), "right": Side::from_stored(a.second).command()}),
            json!(a.master),
        ),
        None => (json!(0.0), Value::Null, Value::Null, Value::Null),
    }
}

fn get_pair(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.getPair", p)?;
    let font = s.font()?;
    let direction = direction(&p)?;
    let master = master(&p, font)?;
    let left = glyph(&p, font, "left")?;
    let right = glyph(&p, font, "right")?;
    let index = KernIndex::new(font);
    let applied = index.lookup(direction, &master, left, right).map_err(|e| kern_error(&p, e))?;
    let l = locks(&index, direction, &master, left, right).map_err(|e| kern_error(&p, e))?;
    let (value, step, pair, from) = applied_json(applied.as_ref());
    Ok(json!({
        "value": value, "step": step, "pair": pair, "from": from,
        "locks": {"left": lock_name(l.first), "right": lock_name(l.second)},
        "groups": {
            "left": index.group(left, facing(direction, Position::First)),
            "right": index.group(right, facing(direction, Position::Second)),
        },
        "master": master, "direction": direction.name(),
    }))
}

fn add_exception(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.addException", p)?;
    let font = s.font()?;
    let direction = direction(&p)?;
    let master = editable_master(&p, font)?;
    let left = glyph(&p, font, "left")?.to_string();
    let right = glyph(&p, font, "right")?.to_string();
    let which = which(&p)?;
    let value = typed_value(&p, "value")?;
    let index = KernIndex::new(font);
    let edit = fontcraft_spacing::kerning::open_locks(&index, direction, &master, &left, &right, which)
        .map_err(|e| kern_error(&p, e))?
        .ok_or_else(|| p.bad(format!("the pair {left} {right} has no closed lock on the {} side to open", side_name(which))))?;
    let value = value.unwrap_or(edit.value);
    s.edit("Open Kerning Lock", |font| {
        store(font, direction, &master, &edit.first, &edit.second, value);
        Ok(())
    })?;
    Ok(json!({
        "left": edit.first.command(), "right": edit.second.command(), "value": value,
        "master": master, "direction": direction.name(),
    }))
}

fn side_name(which: Which) -> &'static str {
    match which {
        Which::First => "left",
        Which::Second => "right",
        Which::Both => "left or right",
    }
}

fn remove_exception(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.removeException", p)?;
    let font = s.font()?;
    let direction = direction(&p)?;
    let master = editable_master(&p, font)?;
    let left = glyph(&p, font, "left")?.to_string();
    let right = glyph(&p, font, "right")?.to_string();
    let which = which(&p)?;
    let index = KernIndex::new(font);
    let (first, second, removed) = fontcraft_spacing::kerning::close_locks(&index, direction, &master, &left, &right, which)
        .map_err(|e| kern_error(&p, e))?
        .map(|a| (a.first.to_string(), a.second.to_string(), a.value))
        .ok_or_else(|| p.bad(format!("the pair {left} {right} has no open lock on the {} side to close", side_name(which))))?;
    s.edit("Close Kerning Lock", |font| {
        let table = direction.table_mut(std::sync::Arc::make_mut(&mut font.kerning));
        unstore(table, &master, &first, &second).map(|_| ()).ok_or_else(|| EngineError::Internal(format!("the pair {first} {second} was not found")))
    })?;
    let font = s.font()?;
    let now = KernIndex::new(font).lookup(direction, &master, &left, &right).map_err(|e| kern_error(&p, e))?.map_or(0.0, |a| a.value);
    Ok(json!({
        "removed": {"left": Side::from_stored(&first).command(), "right": Side::from_stored(&second).command(), "value": removed},
        "value": now, "master": master, "direction": direction.name(),
    }))
}
