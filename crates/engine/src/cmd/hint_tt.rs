//! TrueType Snap and Stem instructions, reversal, stem reference, triple hints and rounding,
//! validated against the touched state (`fontcraft_hinting::TouchedState`).
//!
//! Rows: `hint.tt.snapPoint`, `hint.tt.setSnapZone`, `hint.tt.addStemHint`, `hint.tt.reverse`,
//! `hint.tt.setStemReference`, `hint.tt.makeTripleHint`, `hint.tt.setRounding`,
//! `hinting.tt.instructions.addFromSelection`, `hinting.tt.rounding.modes`, `hinting.tt.snap.*`,
//! `hinting.tt.stem.*`.
//!
//! Targets are explicit: `glyph`, `layer` (the main master's layer when absent, since only its
//! instructions count), points as `[path, node]` in the model's node order with `path` counting
//! paths only (components skipped), `[p1, n1, p2, n2]` for the intersection of two segments, or
//! `"lsb"` / `"rsb"`; instructions by their position in the layer's instruction list, which is
//! execution order. New instructions go to the end of the list and are checked against the
//! touched state after every instruction before them, so a refused selection adds nothing. Each
//! command is one undo step. Zone and stem names are looked up in the main master's TT zones and
//! TT stems.

use std::collections::BTreeSet;

use fontcraft_hinting::replay::describe;
use fontcraft_hinting::settings::main_master;
use fontcraft_hinting::{
    Direction, HintSettings, MAX_LAYER_HINTS, NodeRef, Rounding, StemRef, TouchedState, TtInstruction, TtKind, TtPoint, ZoneRef, set_tt_instructions,
    tt_instructions,
};
use fontcraft_model::{Font, Layer};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, Targets, kind};

const ONE_LAYER: &[TargetKind] = &[TargetKind::Layer];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "hint.tt.snapPoint",
            "Snap Point",
            [],
            Some("A"),
            "{glyph: name, layer?: id (default: the main master's layer), nodes: [point], direction?: \"vertical\" | \"horizontal\" (default vertical), zone?: \"auto\" | \"none\" | TT zone name (default auto; a named zone needs the vertical direction), rounding?: \"round\" | \"up\" | \"down\" | \"none\" | \"gdiOnly\" (default round)} add one Snap instruction per point, at the end of the list. A point is `[path, node]` (model order, paths only), `[p1, n1, p2, n2]` (an intersection) or \"lsb\" / \"rsb\". Every point must be untouched in that direction by the instructions before it; otherwise nothing is added → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            snap_point
        )
        .targets(ONE_LAYER)
        .when("tool:trueTypeInstructor"),
        cmd!(
            "hint.tt.setSnapZone",
            "Set Snap Zone",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index], zone: \"auto\" | \"none\" | TT zone name} set the zone of Snap instructions, named by their position in the layer's instruction list. Another type, or a named zone on a horizontal instruction, is refused → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            set_snap_zone
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.addStemHint",
            "Stem",
            [],
            Some("S"),
            "{glyph: name, layer?: id (default: the main master's layer), nodes: [point] (an even number, paired in order), direction?: \"vertical\" | \"horizontal\" (default vertical), stem?: \"auto\" | \"none\" | TT stem name (default auto; a named stem must suit the direction), rounding?: \"round\" | \"up\" | \"down\" | \"none\" | \"gdiOnly\" (default none)} add one Stem instruction per pair, at the end of the list. A pair is two untouched points (the first is the origin) or one touched and one untouched point (the touched one is the origin); two touched points are refused, and then nothing is added. Points as for Snap Point → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            add_stem
        )
        .targets(ONE_LAYER)
        .when("tool:trueTypeInstructor"),
        cmd!(
            "hint.tt.reverse",
            "Reverse",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index]} swap origin and target of Stem instructions. Reversing a member of a triple hint dissolves that triple hint → {instructions: [index], dissolved: [index], selection: {glyph, layer}}",
            has_doc,
            reverse
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.setStemReference",
            "Set Stem Reference",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index], stem: \"auto\" | \"none\" | TT stem name} set the stem of Stem instructions. A named stem must suit each instruction's direction: a horizontal stem for vertical instructions, a vertical one for horizontal instructions → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            set_stem_reference
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.makeTripleHint",
            "Make Triple Hint",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index, index, index]} link three vertical Stem instructions (on horizontal stems) into a triple hint. Each must not belong to a triple hint already, and the three must not interleave with another triple hint in list order → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            make_triple
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.setRounding",
            "Set Instruction Rounding",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index], rounding: \"round\" | \"up\" | \"down\" | \"none\" | \"gdiOnly\"} set the rounding of Snap, Stem, Shift and Interpolate instructions; a Delta instruction has none and is refused → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            set_rounding
        )
        .targets(ONE_LAYER),
    ]
}

// ── shared ──────────────────────────────────────────────────────────────────────────────────────

/// The addressed layer's instruction list, read from the font.
pub(super) struct Read {
    pub(super) target: Targets,
    pub(super) list: Vec<TtInstruction>,
}

pub(super) fn read(p: &Params, font: &Font) -> Result<Read> {
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => main_master(font),
    };
    let target = Targets::new(p.command(), p.str("glyph")?, Some(layer));
    let r = target.resolve(font)?;
    let list = tt_instructions(r.layer).map_err(|e| p.bad(e.to_string()))?.into_iter().map(|(_, t)| t).collect();
    Ok(Read { target, list })
}

/// Writes `list` back to the addressed layer as one undo step and returns the command's result.
pub(super) fn write(s: &mut Session, p: &Params, label: &str, r: &Read, list: &[TtInstruction], mut out: Value) -> Result<Value> {
    s.edit(label, |font| r.target.edit_layer(font, |l| set_tt_instructions(l, list).map_err(|e| p.bad(e.to_string()))))?;
    if let Some(m) = out.as_object_mut() {
        m.insert("selection".into(), r.target.to_json());
    }
    Ok(out)
}

pub(super) fn direction(p: &Params) -> Result<Direction> {
    match p.opt_str("direction")? {
        None | Some("vertical") => Ok(Direction::Vertical),
        Some("horizontal") => Ok(Direction::Horizontal),
        Some(other) => Err(p.bad(format!("`direction` must be \"vertical\" or \"horizontal\", got \"{other}\""))),
    }
}

pub(super) fn rounding_named(p: &Params, text: &str) -> Result<Rounding> {
    match text {
        "round" => Ok(Rounding::Round),
        "up" => Ok(Rounding::Up),
        "down" => Ok(Rounding::Down),
        "none" => Ok(Rounding::NoRounding),
        "gdiOnly" => Ok(Rounding::GdiOnly),
        other => Err(p.bad(format!("`rounding` must be \"round\", \"up\", \"down\", \"none\" or \"gdiOnly\", got \"{other}\""))),
    }
}

/// The main master's settings, for zone and stem names.
pub(super) fn settings(p: &Params, font: &Font) -> Result<HintSettings> {
    HintSettings::for_master(font, &main_master(font)).map_err(|e| p.bad(e.to_string()))
}

fn zone(p: &Params, font: &Font, text: &str) -> Result<ZoneRef> {
    match text {
        "auto" => Ok(ZoneRef::Auto),
        "none" => Ok(ZoneRef::NoZone),
        name => {
            let zones = settings(p, font)?.tt_zones;
            zones
                .iter()
                .position(|z| z.name == name)
                .map(ZoneRef::Index)
                .ok_or_else(|| p.bad(format!("the main master has no TT zone named `{name}`")))
        }
    }
}

/// A stem reference and, for a named stem, whether that stem is horizontal.
fn stem(p: &Params, font: &Font, text: &str) -> Result<(StemRef, Option<bool>)> {
    match text {
        "auto" => Ok((StemRef::Auto, None)),
        "none" => Ok((StemRef::NoStem, None)),
        name => {
            let stems = settings(p, font)?.tt_stems;
            let i = stems.iter().position(|s| s.name == name).ok_or_else(|| p.bad(format!("the main master has no TT stem named `{name}`")))?;
            Ok((StemRef::Index(i), stems.get(i).map(|s| s.horizontal)))
        }
    }
}

/// Refuses a named stem whose orientation does not suit `direction`: vertical instructions fit
/// horizontal stems and horizontal instructions vertical ones.
fn stem_suits(p: &Params, horizontal: Option<bool>, direction: Direction, text: &str) -> Result<()> {
    match horizontal {
        Some(h) if h != (direction == Direction::Vertical) => {
            let stem_kind = if h { "horizontal" } else { "vertical" };
            let dir = if direction == Direction::Vertical { "vertical" } else { "horizontal" };
            Err(p.bad(format!("TT stem `{text}` is a {stem_kind} stem, which does not suit a {dir} instruction")))
        }
        _ => Ok(()),
    }
}

/// The `nodes` list as points of `layer`; every node must exist.
pub(super) fn points(p: &Params, layer: &Layer) -> Result<Vec<TtPoint>> {
    points_in(p, "nodes", layer)
}

/// The list under `key` as points of `layer`; every node must exist.
pub(super) fn points_in(p: &Params, key: &str, layer: &Layer) -> Result<Vec<TtPoint>> {
    let values = p.array(key)?;
    if values.is_empty() {
        return Err(p.bad(format!("`{key}` lists no point")));
    }
    if values.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`{key}` lists more than {MAX_LAYER_HINTS} points")));
    }
    values.iter().map(|v| point(p, key, v, layer)).collect()
}

/// One point given as `v` under `key`; its nodes must exist.
pub(super) fn point(p: &Params, key: &str, v: &Value, layer: &Layer) -> Result<TtPoint> {
    let shape = || p.bad(format!("`{key}`: each point is [path, node], [path, node, path, node], \"lsb\" or \"rsb\", found {v}"));
    let point = match v {
        Value::String(s) if s == "lsb" => TtPoint::Lsb,
        Value::String(s) if s == "rsb" => TtPoint::Rsb,
        Value::Array(a) => {
            let n: Vec<usize> = a.iter().map(|x| x.as_u64().and_then(|x| usize::try_from(x).ok())).collect::<Option<_>>().ok_or_else(shape)?;
            match n.as_slice() {
                [path, node] => TtPoint::Node(NodeRef::new(*path, *node)),
                [p1, n1, p2, n2] => TtPoint::Intersection { a: NodeRef::new(*p1, *n1), b: NodeRef::new(*p2, *n2) },
                _ => return Err(shape()),
            }
        }
        _ => return Err(shape()),
    };
    let refs = match point {
        TtPoint::Node(r) => vec![r],
        TtPoint::Intersection { a, b } => vec![a, b],
        TtPoint::Lsb | TtPoint::Rsb => Vec::new(),
    };
    for r in refs {
        r.check(layer).map_err(|_| p.bad(format!("`{key}`: {} does not resolve on the layer", describe(&TtPoint::Node(r)))))?;
    }
    Ok(point)
}

/// The `instructions` list: positions in `list`, repeats dropped, in the order given.
pub(super) fn indexes(p: &Params, list: &[TtInstruction]) -> Result<Vec<usize>> {
    let values = p.array("instructions")?;
    if values.is_empty() {
        return Err(p.bad("`instructions` lists no instruction"));
    }
    if values.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`instructions` lists more than {MAX_LAYER_HINTS} entries")));
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for v in values {
        let i =
            v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`instructions` must hold indexes, found {}", kind(v))))?;
        if i >= list.len() {
            return Err(p.bad(format!("`instructions`: {i} does not resolve: the layer has {} instructions", list.len())));
        }
        if seen.insert(i) {
            out.push(i);
        }
    }
    Ok(out)
}

pub(super) fn type_name(t: &TtInstruction) -> &'static str {
    match t.kind {
        TtKind::Snap { .. } => "a Snap",
        TtKind::Stem { .. } => "a Stem",
        TtKind::Shift => "a Shift",
        TtKind::Interpolate => "an Interpolate",
        TtKind::Delta { .. } => "a Delta",
    }
}

/// Refuses unless every listed instruction is of the type `is` accepts.
pub(super) fn all_of(p: &Params, list: &[TtInstruction], ix: &[usize], wanted: &str, is: impl Fn(&TtKind) -> bool) -> Result<()> {
    for &i in ix {
        let t = list.get(i).ok_or_else(|| p.bad(format!("`instructions`: {i} does not resolve")))?;
        if !is(&t.kind) {
            return Err(p.bad(format!("instruction {i} is {} instruction: the command acts on {wanted} instructions", type_name(t))));
        }
    }
    Ok(())
}

// ── adding ──────────────────────────────────────────────────────────────────────────────────────

/// Appends `new` to the list, each checked against the touched state after everything before it.
pub(super) fn append(p: &Params, list: &mut Vec<TtInstruction>, new: Vec<TtInstruction>) -> Result<Vec<usize>> {
    let mut state = TouchedState::replay(list, usize::MAX);
    let first = list.len();
    for t in new {
        state.check(&t).map_err(|e| p.bad(e))?;
        state.run(&t);
        list.push(t);
    }
    Ok((first..list.len()).collect())
}

fn snap_point(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.snapPoint", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let nodes = points(&p, layer)?;
    let dir = direction(&p)?;
    let zone = match p.opt_str("zone")? {
        Some(text) => zone(&p, font, text)?,
        None => ZoneRef::Auto,
    };
    if matches!(zone, ZoneRef::Index(_)) && dir == Direction::Horizontal {
        return Err(p.bad("zones are vertical: a named zone needs a vertical Snap instruction"));
    }
    let rounding = p.opt_str("rounding")?.map(|text| rounding_named(&p, text)).transpose()?;
    let new = nodes
        .into_iter()
        .map(|n| {
            let mut t = TtInstruction::new(TtKind::Snap { zone: zone.clone() }, dir, n);
            t.rounding = rounding.unwrap_or(t.rounding);
            t
        })
        .collect();
    let mut list = std::mem::take(&mut r.list);
    let added = append(&p, &mut list, new)?;
    write(s, &p, "Snap Point", &r, &list, json!({"instructions": added}))
}

fn add_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.addStemHint", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let nodes = points(&p, layer)?;
    if nodes.len() % 2 != 0 {
        return Err(p.bad(format!("`nodes` lists {} points: Stem instructions need an even number, one pair per instruction", nodes.len())));
    }
    let dir = direction(&p)?;
    let (stem_ref, horizontal, text) = match p.opt_str("stem")? {
        Some(text) => {
            let (r, h) = stem(&p, font, text)?;
            (r, h, text)
        }
        None => (StemRef::Auto, None, "auto"),
    };
    stem_suits(&p, horizontal, dir, text)?;
    let rounding = p.opt_str("rounding")?.map(|text| rounding_named(&p, text)).transpose()?;
    let mut list = std::mem::take(&mut r.list);
    // Orient each pair against the state the earlier pairs leave, so the touched point leads.
    let mut state = TouchedState::replay(&list, usize::MAX);
    let mut new = Vec::new();
    for pair in nodes.chunks(2) {
        let [a, b] = pair else { continue };
        let (origin, target) = if state.is_touched(b, dir) && !state.is_touched(a, dir) { (*b, *a) } else { (*a, *b) };
        let mut t = TtInstruction::new(TtKind::Stem { stem: stem_ref.clone(), triple: None }, dir, target);
        t.origin = Some(origin);
        t.rounding = rounding.unwrap_or(t.rounding);
        state.check(&t).map_err(|e| p.bad(e))?;
        state.run(&t);
        new.push(t);
    }
    let added = append(&p, &mut list, new)?;
    write(s, &p, "Stem", &r, &list, json!({"instructions": added}))
}

// ── changing ────────────────────────────────────────────────────────────────────────────────────

fn set_snap_zone(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setSnapZone", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = indexes(&p, &r.list)?;
    let zone = zone(&p, font, p.str("zone")?)?;
    all_of(&p, &r.list, &ix, "Snap", |k| matches!(k, TtKind::Snap { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        let Some(t) = list.get_mut(i) else { continue };
        if matches!(zone, ZoneRef::Index(_)) && t.direction == Direction::Horizontal {
            return Err(p.bad(format!("instruction {i} is horizontal: zones are vertical, so a named zone needs a vertical Snap instruction")));
        }
        t.kind = TtKind::Snap { zone: zone.clone() };
    }
    write(s, &p, "Set Snap Zone", &r, &list, json!({"instructions": ix}))
}

fn set_stem_reference(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setStemReference", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = indexes(&p, &r.list)?;
    let text = p.str("stem")?;
    let (stem_ref, horizontal) = stem(&p, font, text)?;
    all_of(&p, &r.list, &ix, "Stem", |k| matches!(k, TtKind::Stem { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        let Some(t) = list.get_mut(i) else { continue };
        stem_suits(&p, horizontal, t.direction, text)?;
        if let TtKind::Stem { stem, .. } = &mut t.kind {
            *stem = stem_ref.clone();
        }
    }
    write(s, &p, "Set Stem Reference", &r, &list, json!({"instructions": ix}))
}

fn set_rounding(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setRounding", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = indexes(&p, &r.list)?;
    let rounding = rounding_named(&p, p.str("rounding")?)?;
    all_of(&p, &r.list, &ix, "Snap, Stem, Shift or Interpolate", |k| !matches!(k, TtKind::Delta { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        if let Some(t) = list.get_mut(i) {
            t.rounding = rounding;
        }
    }
    write(s, &p, "Set Instruction Rounding", &r, &list, json!({"instructions": ix}))
}

/// The triple group of each Stem instruction, by list position.
pub(super) fn triple_of(t: &TtInstruction) -> Option<u32> {
    match t.kind {
        TtKind::Stem { triple, .. } => triple,
        _ => None,
    }
}

fn reverse(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.reverse", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = indexes(&p, &r.list)?;
    all_of(&p, &r.list, &ix, "Stem", |k| matches!(k, TtKind::Stem { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    let groups: BTreeSet<u32> = ix.iter().filter_map(|&i| list.get(i).and_then(triple_of)).collect();
    let mut dissolved = Vec::new();
    for (i, t) in list.iter_mut().enumerate() {
        if let TtKind::Stem { triple, .. } = &mut t.kind
            && triple.is_some_and(|g| groups.contains(&g))
        {
            *triple = None;
            dissolved.push(i);
        }
    }
    for &i in &ix {
        let Some(t) = list.get_mut(i) else { continue };
        let origin = t.origin.ok_or_else(|| p.bad(format!("Stem instruction {i} has no origin")))?;
        t.origin = Some(t.target);
        t.target = origin;
    }
    write(s, &p, "Reverse", &r, &list, json!({"instructions": ix, "dissolved": dissolved}))
}

fn make_triple(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.makeTripleHint", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let mut ix = indexes(&p, &r.list)?;
    if ix.len() != 3 {
        return Err(p.bad(format!("`instructions` names {} different instructions: a triple hint links three", ix.len())));
    }
    all_of(&p, &r.list, &ix, "Stem", |k| matches!(k, TtKind::Stem { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        let Some(t) = list.get_mut(i) else { continue };
        if t.direction != Direction::Vertical {
            return Err(p.bad(format!(
                "instruction {i} is horizontal: a triple hint links Stem instructions on three horizontal stems, which are vertical instructions"
            )));
        }
        let TtKind::Stem { triple, .. } = &mut t.kind else { continue };
        if triple.is_some() {
            return Err(p.bad(format!("instruction {i} belongs to a triple hint already")));
        }
        *triple = Some(u32::MAX);
    }
    // The stored form keeps only a flag, and the flagged instructions form groups by threes in
    // list order; the new three must come out as one of those groups.
    ix.sort_unstable();
    let flagged: Vec<usize> = list.iter().enumerate().filter(|(_, t)| triple_of(t).is_some()).map(|(i, _)| i).collect();
    if !flagged.chunks(3).any(|c| c == ix.as_slice()) {
        return Err(
            p.bad("the three instructions interleave with another triple hint in list order: the stored form could not tell the two groups apart")
        );
    }
    write(s, &p, "Make Triple Hint", &r, &list, json!({"instructions": ix}))
}
