//! Kerning maintenance from the Kerning window's actions menu: Clean Up, Compress, and renaming a
//! kerning group. The logic is `fontcraft_spacing::kerning::maintain`.

use fontcraft_spacing::kerning::{self, CompressReport, Direction, KeptWhy, MaintainError, NamedPair, Obsolete, Pair, Position, RenameScope, Side};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "kerning.cleanUp",
            "Clean Up",
            [],
            None,
            "{master?: id (default: the first master), direction?: \"ltr\" | \"rtl\" | \"vertical\" (default ltr)} delete the obsolete pairs of one master and direction: pairs that name a glyph the font no longer has, or a group no glyph belongs to on the side facing the pair. A master that shows another master's kerning is refused → {master, direction, removed: [{left, right, value, reason: \"missingGlyph\" | \"emptyGroup\", name}]}",
            has_doc,
            clean_up
        ),
        cmd!(
            "kerning.compress",
            "Compress",
            [],
            None,
            "{master?: id (default: the first master), direction?: \"ltr\" | \"rtl\" | \"vertical\" (default ltr), untilStable?: bool (default false: one run)} turn glyph sides of pairs into the glyphs' kerning groups. One run lifts one side per pair, the left side first. A pair whose group pair already exists is removed when the values are equal and kept as an exception when they differ. untilStable repeats runs until nothing changes, at most 8, as one undo step. A master that shows another master's kerning is refused → {master, direction, runs, pairsBefore, pairsAfter, lifted: [{run, from: {left, right}, to: {left, right}, value}], absorbed: [{run, left, right, value, into: {left, right}}], kept: [{left, right, value, reason: \"differentValue\" | \"memberPairs\", groupPair?: {left, right, value}}]}",
            has_doc,
            compress
        ),
        cmd!(
            "kerning.renameGroup",
            "Rename Kerning Group",
            [],
            None,
            "{side: \"left\" | \"right\" (the column of the pair list), from: group name, to: group name, scope: \"pair\" | \"font\", pair?: {left, right} (required for scope pair; sides as `A` or `@group`), master?: id (scope pair; default: the first master), direction?: \"ltr\" | \"rtl\" | \"vertical\" (default ltr)} rename a kerning group of the glyph side that faces the pair in that column. Scope font renames the group on every glyph and in every pair of every master and direction; the new name may not be a group of that side already. Scope pair points only that pair's side at the new name; the glyphs keep their group. Groups of the other side never change → {side, from, to, scope, glyphs: [names], pairs: [{direction, master, left, right}]}",
            has_doc,
            rename_group
        ),
    ]
}

fn maintain_error(p: &Params, e: MaintainError) -> EngineError {
    p.bad(e.to_string())
}

fn direction(p: &Params) -> Result<Direction> {
    match p.opt_str("direction")? {
        None => Ok(Direction::Ltr),
        Some(name) => {
            Direction::from_name(name).ok_or_else(|| p.bad(format!("`direction` must be \"ltr\", \"rtl\" or \"vertical\", got \"{name}\"")))
        }
    }
}

fn master(s: &Session, p: &Params) -> Result<String> {
    let font = s.font()?;
    Ok(match p.opt_str("master")? {
        Some(id) => font.master(id)?.id.clone(),
        None => font.default_master()?.id.clone(),
    })
}

fn sides(first: &Side, second: &Side) -> Value {
    json!({"left": first.command(), "right": second.command()})
}

fn pair_json(pair: &Pair) -> Value {
    json!({"left": pair.first.command(), "right": pair.second.command(), "value": pair.value})
}

fn named_json(pair: &NamedPair) -> Value {
    json!({
        "direction": pair.direction.name(),
        "master": pair.master,
        "left": Side::from_stored(&pair.first).command(),
        "right": Side::from_stored(&pair.second).command(),
    })
}

fn clean_up(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.cleanUp", v)?;
    let direction = direction(&p)?;
    let master = master(s, &p)?;
    let removed = s.edit("Clean Up Kerning", |font| kerning::clean_up(font, direction, &master).map_err(|e| maintain_error(&p, e)))?;
    let removed: Vec<Value> = removed
        .iter()
        .map(|r| {
            let (reason, name) = match &r.reason {
                Obsolete::MissingGlyph(n) => ("missingGlyph", n),
                Obsolete::EmptyGroup(n) => ("emptyGroup", n),
            };
            let mut out = pair_json(&r.pair);
            if let Some(o) = out.as_object_mut() {
                o.insert("reason".into(), json!(reason));
                o.insert("name".into(), json!(name));
            }
            out
        })
        .collect();
    Ok(json!({"master": master, "direction": direction.name(), "removed": removed}))
}

fn compress_json(master: &str, direction: Direction, r: &CompressReport) -> Value {
    let lifted: Vec<Value> = r
        .lifted
        .iter()
        .map(|l| json!({"run": l.run, "from": sides(&l.from.first, &l.from.second), "to": sides(&l.to.0, &l.to.1), "value": l.from.value}))
        .collect();
    let absorbed: Vec<Value> = r
        .absorbed
        .iter()
        .map(|a| {
            json!({"run": a.run, "left": a.pair.first.command(), "right": a.pair.second.command(), "value": a.pair.value, "into": sides(&a.into.0, &a.into.1)})
        })
        .collect();
    let kept: Vec<Value> = r
        .kept
        .iter()
        .map(|k| {
            let mut out = pair_json(&k.pair);
            if let Some(o) = out.as_object_mut() {
                match &k.why {
                    KeptWhy::DifferentValue(group) => {
                        o.insert("reason".into(), json!("differentValue"));
                        o.insert("groupPair".into(), pair_json(group));
                    }
                    KeptWhy::MemberPairs => {
                        o.insert("reason".into(), json!("memberPairs"));
                    }
                }
            }
            out
        })
        .collect();
    json!({
        "master": master,
        "direction": direction.name(),
        "runs": r.runs,
        "pairsBefore": r.pairs_before,
        "pairsAfter": r.pairs_after,
        "lifted": lifted,
        "absorbed": absorbed,
        "kept": kept,
    })
}

fn compress(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.compress", v)?;
    let direction = direction(&p)?;
    let master = master(s, &p)?;
    let runs = if p.bool_or("untilStable", false)? { kerning::MAX_COMPRESS_RUNS } else { 1 };
    let report = s.edit("Compress Kerning", |font| kerning::compress(font, direction, &master, runs).map_err(|e| maintain_error(&p, e)))?;
    Ok(compress_json(&master, direction, &report))
}

fn rename_group(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.renameGroup", v)?;
    let direction = direction(&p)?;
    let position = match p.str("side")? {
        "left" => Position::First,
        "right" => Position::Second,
        other => return Err(p.bad(format!("`side` must be \"left\" or \"right\", got \"{other}\""))),
    };
    let from = p.str("from")?;
    let to = p.str("to")?;
    let scope_name = p.str("scope")?;
    let scope = match scope_name {
        "font" => RenameScope::Font,
        "pair" => {
            let pair = p.opt_object("pair")?.ok_or_else(|| p.bad("`pair` is required when `scope` is \"pair\""))?;
            let side = |key: &str| -> Result<Side> {
                let text = pair.get(key).and_then(Value::as_str).ok_or_else(|| p.bad(format!("`pair.{key}` must be a string")))?;
                Side::parse(text).map_err(|e| p.bad(format!("`pair.{key}`: {e}")))
            };
            RenameScope::Pair { master: master(s, &p)?, first: side("left")?, second: side("right")? }
        }
        other => return Err(p.bad(format!("`scope` must be \"pair\" or \"font\", got \"{other}\""))),
    };
    let report =
        s.edit("Rename Kerning Group", |font| kerning::rename_group(font, direction, position, from, to, &scope).map_err(|e| maintain_error(&p, e)))?;
    let pairs: Vec<Value> = report.pairs.iter().map(named_json).collect();
    Ok(json!({
        "side": if position == Position::First { "left" } else { "right" },
        "from": from.strip_prefix('@').unwrap_or(from),
        "to": to.strip_prefix('@').unwrap_or(to),
        "scope": scope_name,
        "glyphs": report.glyphs,
        "pairs": pairs,
    }))
}
