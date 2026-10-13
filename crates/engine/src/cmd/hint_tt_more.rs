//! TrueType Shift, Interpolate and Delta instructions, deleting, removing and listing
//! instructions, validated against the touched state (`fontcraft_hinting::TouchedState`).
//!
//! Rows: `hint.tt.shiftPoints`, `hint.tt.interpolate`, `hint.tt.addDelta`, `hint.tt.setDeltas`,
//! `hint.tt.deleteInstructions`, `hint.tt.removeHints`, `hint.tt.listInstructions`,
//! `hinting.tt.shift.*`, `hinting.tt.interpolate.*`, `hinting.tt.delta.restrictions`.
//!
//! Parameters follow `hint_tt`: `glyph`, `layer` (the main master's layer when absent), points as
//! `[path, node]`, `[p1, n1, p2, n2]` or `"lsb"` / `"rsb"`, instructions by list position. Delta
//! instructions are vertical and belong to the main master: a horizontal direction, or a layer of
//! another master, is refused. Their cells are keyed by column: an instance or master, named by
//! name or id, becomes the stored key for its location ([`delta_column_key`]); a stored key in
//! braces, as `hint.tt.listInstructions` reports it, is taken as given.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_hinting::settings::main_master;
use fontcraft_hinting::tt::delta_column_key;
use fontcraft_hinting::{
    DeltaDir, Deltas, Direction, MAX_DELTA_COLUMNS, MAX_DELTA_PPM, Rounding, StemRef, TouchedState, TtInstruction, TtKind, TtPoint, ZoneRef,
    set_tt_instructions, unmet_needs,
};
use fontcraft_model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::hint_tt::{Read, all_of, append, direction, indexes, point, points, points_in, read, rounding_named, settings, triple_of, write};
use super::*;
use crate::params::{Params, TargetKind, Targets};

const ONE_LAYER: &[TargetKind] = &[TargetKind::Layer];
const GLYPHS: &[TargetKind] = &[TargetKind::Glyphs];

/// Longest stored column key (`{…}`) accepted as given.
const MAX_COLUMN_KEY: usize = 512;
/// Most cells one `cells` parameter may name: every PPM of every column.
const MAX_CELL_EDITS: usize = MAX_DELTA_COLUMNS * MAX_DELTA_PPM as usize;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "hint.tt.shiftPoints",
            "Shift Points",
            [],
            Some("F"),
            "{glyph: name, layer?: id (default: the main master's layer), nodes?: [point] (exactly one touched point, the origin, and untouched targets), origin?: point, targets?: [point] (instead of `nodes`), direction?: \"vertical\" | \"horizontal\" (default vertical), rounding?: \"round\" | \"up\" | \"down\" | \"none\" | \"gdiOnly\" (default none)} add one Shift instruction per target, from the origin, at the end of the list. The origin must be touched and every target untouched in that direction by the instructions before it; otherwise nothing is added. Points as for Snap Point → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            shift_points
        )
        .targets(ONE_LAYER)
        .when("tool:trueTypeInstructor"),
        cmd!(
            "hint.tt.interpolate",
            "Interpolate",
            [],
            Some("G"),
            "{glyph: name, layer?: id (default: the main master's layer), nodes?: [point, point, point] (two touched references and one untouched target, in any order), references?: [point, point], target?: point (instead of `nodes`), direction?: \"vertical\" | \"horizontal\" (default vertical), rounding?: \"round\" | \"up\" | \"down\" | \"none\" | \"gdiOnly\" (default none)} add one Interpolate instruction at the end of the list: the target keeps its relative position between the two references → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            interpolate
        )
        .targets(ONE_LAYER)
        .when("tool:trueTypeInstructor"),
        cmd!(
            "hint.tt.addDelta",
            "Delta",
            [],
            Some("E"),
            "{glyph: name, layer?: id (default: the main master's layer; a layer of another master is refused), nodes: [point] (touched or not), direction?: \"vertical\" (the only direction Delta instructions have), cells?: {column: {ppm: \"up\" | \"down\" | \"none\"}}} add one Delta instruction per point, at the end of the list. A column is an instance or master, by name or id, or a stored column key in braces; a PPM is a size from 1 to 255 or a range \"a-b\" → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            add_delta
        )
        .targets(ONE_LAYER)
        .when("tool:trueTypeInstructor"),
        cmd!(
            "hint.tt.setDeltas",
            "Set Delta Cells",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer; a layer of another master is refused), instructions: [index], cells: {column: {ppm: \"up\" | \"down\" | \"none\"}}} set cells of Delta instructions: \"up\" or \"down\" moves the point one pixel at that size in that column, \"none\" clears the cell; cells not named stay as they are. Columns and PPMs as for Delta → {instructions: [index], selection: {glyph, layer}}",
            has_doc,
            set_deltas
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.deleteInstructions",
            "Delete Instructions",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer), instructions: [index]} delete instructions by their position in the list. Deleting a member of a triple hint dissolves that triple hint. Later instructions whose needs the deleted ones met (a Shift origin, Interpolate references, an untouched point) stay and are reported → {deleted: [index], dissolved: [index], unmet: [{instruction: index, reason}], selection: {glyph, layer}}; `dissolved` and `unmet` count positions in the new list",
            has_doc,
            delete_instructions
        )
        .targets(ONE_LAYER),
        cmd!(
            "hint.tt.removeHints",
            "Remove Hints",
            [],
            None,
            "{glyphs: [name] (or glyph: name), layer?: id (default: the main master's layer)} delete every TrueType instruction of that layer of each glyph, in both directions; PostScript hints stay → {glyphs: [name], removed: count}; `glyphs` lists the glyphs that had instructions",
            has_doc,
            remove_hints
        )
        .targets(GLYPHS),
        cmd!(
            query "hint.tt.listInstructions",
            "List TrueType Instructions",
            [],
            None,
            "{glyph: name, layer?: id (default: the main master's layer)} the layer's instructions in execution order → {glyph, layer, mainMaster: bool, instructions: [{index, type: \"snap\" | \"stem\" | \"shift\" | \"interpolate\" | \"delta\", direction, target: point, origin?: point, references?: [point, point], rounding?, zone?, stem?, triple?: group, deltas?: [{column, names: [name], ppm, shift: \"up\" | \"down\"}]}], unmet: [{instruction: index, reason}]}; zones and stems by name, or \"auto\" / \"none\"; `names` lists the instances and masters at a column's location",
            has_doc,
            list_instructions
        )
        .targets(ONE_LAYER),
    ]
}

// ── Shift and Interpolate ───────────────────────────────────────────────────────────────────────

/// A refusal for parameters that give a selection both as `nodes` and in the explicit form.
fn one_form(p: &Params, explicit: &[&str]) -> Result<bool> {
    let has_explicit = explicit.iter().any(|k| p.get(k).is_some());
    if has_explicit && p.get("nodes").is_some() {
        return Err(p.bad(format!("give `nodes` or `{}`, not both", explicit.join("` and `"))));
    }
    Ok(has_explicit)
}

/// One point under `key`, which is required.
fn one_point(p: &Params, key: &str, layer: &Layer) -> Result<TtPoint> {
    let v = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    point(p, key, v, layer)
}

fn opt_rounding(p: &Params) -> Result<Option<Rounding>> {
    p.opt_str("rounding")?.map(|text| rounding_named(p, text)).transpose()
}

fn shift_points(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.shiftPoints", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let dir = direction(&p)?;
    let rounding = opt_rounding(&p)?;
    let state = TouchedState::replay(&r.list, usize::MAX);
    let (origin, targets) = if one_form(&p, &["origin", "targets"])? {
        (one_point(&p, "origin", layer)?, points_in(&p, "targets", layer)?)
    } else {
        let nodes = points(&p, layer)?;
        let (touched, untouched): (Vec<TtPoint>, Vec<TtPoint>) = nodes.into_iter().partition(|n| state.is_touched(n, dir));
        match touched.as_slice() {
            [origin] if !untouched.is_empty() => (*origin, untouched),
            [_] => return Err(p.bad("`nodes` holds no untouched point: Shift needs one touched origin and at least one untouched target")),
            _ => {
                return Err(p.bad(format!(
                    "`nodes` holds {} touched points: Shift needs exactly one touched point, the origin, and untouched targets",
                    touched.len()
                )));
            }
        }
    };
    let new = targets
        .into_iter()
        .map(|target| {
            let mut t = TtInstruction::new(TtKind::Shift, dir, target);
            t.origin = Some(origin);
            t.rounding = rounding.unwrap_or(t.rounding);
            t
        })
        .collect();
    let mut list = std::mem::take(&mut r.list);
    let added = append(&p, &mut list, new)?;
    write(s, &p, "Shift Points", &r, &list, json!({"instructions": added}))
}

fn interpolate(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.interpolate", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let dir = direction(&p)?;
    let rounding = opt_rounding(&p)?;
    let state = TouchedState::replay(&r.list, usize::MAX);
    let (a, b, target) = if one_form(&p, &["references", "target"])? {
        let refs = points_in(&p, "references", layer)?;
        let [a, b] = refs.as_slice() else {
            return Err(p.bad(format!("`references` lists {} points: Interpolate needs two", refs.len())));
        };
        (*a, *b, one_point(&p, "target", layer)?)
    } else {
        let nodes = points(&p, layer)?;
        if nodes.len() != 3 {
            return Err(p.bad(format!("`nodes` lists {} points: Interpolate needs two touched references and one untouched target", nodes.len())));
        }
        let (touched, untouched): (Vec<TtPoint>, Vec<TtPoint>) = nodes.into_iter().partition(|n| state.is_touched(n, dir));
        let ([a, b], [target]) = (touched.as_slice(), untouched.as_slice()) else {
            return Err(
                p.bad(format!("`nodes` holds {} touched points: Interpolate needs two touched references and one untouched target", touched.len()))
            );
        };
        (*a, *b, *target)
    };
    let mut t = TtInstruction::new(TtKind::Interpolate, dir, target);
    t.origin = Some(a);
    t.other = Some(b);
    t.rounding = rounding.unwrap_or(t.rounding);
    let mut list = std::mem::take(&mut r.list);
    let added = append(&p, &mut list, vec![t])?;
    write(s, &p, "Interpolate", &r, &list, json!({"instructions": added}))
}

// ── Delta ───────────────────────────────────────────────────────────────────────────────────────

/// Cell changes: column key → PPM → a direction, or `None` to clear the cell.
type CellEdits = BTreeMap<String, BTreeMap<u16, Option<DeltaDir>>>;

/// Refuses unless the addressed layer belongs to the main master, the only one whose Delta
/// instructions count.
fn main_master_only(p: &Params, font: &Font, r: &Read) -> Result<()> {
    let layer = r.target.resolve(font)?.layer;
    let main = main_master(font);
    if layer.master != main {
        return Err(p.bad(format!("Delta instructions belong to the main master `{main}`: layer `{}` is on master `{}`", layer.id, layer.master)));
    }
    Ok(())
}

/// The stored column key `key` names.
fn column(p: &Params, font: &Font, key: &str) -> Result<String> {
    if key.starts_with('{') {
        if key.len() > MAX_COLUMN_KEY || !key.ends_with('}') {
            return Err(p.bad(format!("`cells`: a column key in braces is at most {MAX_COLUMN_KEY} bytes and ends with `}}`")));
        }
        return Ok(key.to_string());
    }
    if let Some(i) = font.instances.iter().find(|i| i.name == key || i.id.as_deref() == Some(key)) {
        return Ok(delta_column_key(&i.location));
    }
    if let Some(m) = font.masters.iter().find(|m| m.id == key || m.name == key) {
        return Ok(delta_column_key(&m.location));
    }
    Err(p.bad(format!("`cells`: the font has no instance or master `{key}`")))
}

/// A PPM key: one size, or a range `a-b`, each from 1 to [`MAX_DELTA_PPM`].
fn ppms(p: &Params, key: &str) -> Result<std::ops::RangeInclusive<u16>> {
    let size = |text: &str| {
        text.trim()
            .parse::<u16>()
            .ok()
            .filter(|n| (1..=MAX_DELTA_PPM).contains(n))
            .ok_or_else(|| p.bad(format!("`cells`: `{key}` is not a PPM from 1 to {MAX_DELTA_PPM} or a range of them such as \"12-14\"")))
    };
    let (a, b) = match key.split_once('-') {
        Some((a, b)) => (size(a)?, size(b)?),
        None => {
            let n = size(key)?;
            (n, n)
        }
    };
    if a > b {
        return Err(p.bad(format!("`cells`: the range `{key}` runs backwards")));
    }
    Ok(a..=b)
}

/// The `cells` parameter, every column resolved and every PPM checked.
fn cell_edits(p: &Params, font: &Font) -> Result<CellEdits> {
    let mut out = CellEdits::new();
    let Some(columns) = p.opt_object("cells")? else { return Ok(out) };
    if columns.len() > MAX_DELTA_COLUMNS {
        return Err(p.bad(format!("`cells` names more than {MAX_DELTA_COLUMNS} columns")));
    }
    let mut count = 0usize;
    for (key, rows) in columns {
        let col = column(p, font, key)?;
        let rows = rows.as_object().ok_or_else(|| p.bad(format!("`cells`: column `{key}` must map PPMs to \"up\", \"down\" or \"none\"")))?;
        if rows.len() > usize::from(MAX_DELTA_PPM) {
            return Err(p.bad(format!("`cells`: column `{key}` names more than {MAX_DELTA_PPM} rows")));
        }
        let edits = out.entry(col).or_default();
        for (ppm, value) in rows {
            let dir = match value.as_str() {
                Some("up") => Some(DeltaDir::Up),
                Some("down") => Some(DeltaDir::Down),
                Some("none") => None,
                _ => return Err(p.bad(format!("`cells`: column `{key}`, PPM `{ppm}` must be \"up\", \"down\" or \"none\", found {value}"))),
            };
            for size in ppms(p, ppm)? {
                count = count.saturating_add(1);
                if count > MAX_CELL_EDITS {
                    return Err(p.bad(format!("`cells` names more than {MAX_CELL_EDITS} cells")));
                }
                edits.insert(size, dir);
            }
        }
    }
    Ok(out)
}

/// Applies `edits` to `cells`, dropping emptied columns; more than [`MAX_DELTA_COLUMNS`] columns
/// in the result is refused.
fn apply_cells(p: &Params, cells: &mut Deltas, edits: &CellEdits) -> Result<()> {
    for (col, rows) in edits {
        let column = cells.entry(col.clone()).or_default();
        for (&ppm, dir) in rows {
            match dir {
                Some(d) => {
                    column.insert(ppm, *d);
                }
                None => {
                    column.remove(&ppm);
                }
            }
        }
        if column.is_empty() {
            cells.remove(col);
        }
    }
    if cells.len() > MAX_DELTA_COLUMNS {
        return Err(p.bad(format!("a Delta instruction may have cells in at most {MAX_DELTA_COLUMNS} columns")));
    }
    Ok(())
}

fn add_delta(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.addDelta", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    if direction(&p)? == Direction::Horizontal {
        return Err(p.bad("Delta instructions exist only in the vertical direction"));
    }
    main_master_only(&p, font, &r)?;
    let layer = r.target.resolve(font)?.layer;
    let nodes = points(&p, layer)?;
    let edits = cell_edits(&p, font)?;
    let mut cells = Deltas::new();
    apply_cells(&p, &mut cells, &edits)?;
    let new = nodes.into_iter().map(|n| TtInstruction::new(TtKind::Delta { cells: cells.clone() }, Direction::Vertical, n)).collect();
    let mut list = std::mem::take(&mut r.list);
    let added = append(&p, &mut list, new)?;
    write(s, &p, "Delta", &r, &list, json!({"instructions": added}))
}

fn set_deltas(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setDeltas", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    main_master_only(&p, font, &r)?;
    let ix = indexes(&p, &r.list)?;
    if p.get("cells").is_none() {
        return Err(p.bad("`cells` is required"));
    }
    let edits = cell_edits(&p, font)?;
    all_of(&p, &r.list, &ix, "Delta", |k| matches!(k, TtKind::Delta { .. }))?;
    let mut list = std::mem::take(&mut r.list);
    for &i in &ix {
        if let Some(TtInstruction { kind: TtKind::Delta { cells }, .. }) = list.get_mut(i) {
            apply_cells(&p, cells, &edits)?;
        }
    }
    write(s, &p, "Set Delta Cells", &r, &list, json!({"instructions": ix}))
}

// ── deleting ────────────────────────────────────────────────────────────────────────────────────

fn delete_instructions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.deleteInstructions", v)?;
    let font = s.font()?;
    let mut r = read(&p, font)?;
    let ix = indexes(&p, &r.list)?;
    let gone: BTreeSet<usize> = ix.iter().copied().collect();
    let old = std::mem::take(&mut r.list);
    let unmet_before: BTreeSet<usize> = unmet_needs(&old).into_iter().map(|(i, _)| i).collect();
    // The stored form groups flagged Stem instructions by threes in list order, so the other
    // members of a deleted member's triple hint must lose their flag too.
    let groups: BTreeSet<u32> = gone.iter().filter_map(|&i| old.get(i).and_then(triple_of)).collect();
    let mut list = Vec::with_capacity(old.len());
    let mut was = Vec::with_capacity(old.len());
    let mut dissolved = Vec::new();
    for (i, mut t) in old.into_iter().enumerate() {
        if gone.contains(&i) {
            continue;
        }
        if let TtKind::Stem { triple, .. } = &mut t.kind
            && triple.is_some_and(|g| groups.contains(&g))
        {
            *triple = None;
            dissolved.push(list.len());
        }
        was.push(i);
        list.push(t);
    }
    let unmet: Vec<Value> = unmet_needs(&list)
        .into_iter()
        .filter(|(i, _)| was.get(*i).is_some_and(|w| !unmet_before.contains(w)))
        .map(|(i, reason)| json!({"instruction": i, "reason": reason}))
        .collect();
    let deleted: Vec<usize> = gone.into_iter().collect();
    write(s, &p, "Delete Instructions", &r, &list, json!({"deleted": deleted, "dissolved": dissolved, "unmet": unmet}))
}

fn remove_hints(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.removeHints", v)?;
    let font = s.font()?;
    let names = match (p.opt_glyphs("glyphs", font)?, p.opt_str("glyph")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `glyphs` or `glyph`, not both")),
        (Some(names), None) => names,
        (None, Some(name)) => vec![name.to_string()],
        (None, None) => return Err(p.bad("`glyphs` is required")),
    };
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => main_master(font),
    };
    let mut targets = Vec::new();
    let mut removed = 0usize;
    for name in names {
        let target = Targets::new(p.command(), name.clone(), Some(layer.clone()));
        let count = fontcraft_hinting::tt_instructions(target.resolve(font)?.layer).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?.len();
        if count > 0 {
            removed = removed.saturating_add(count);
            targets.push((name, target));
        }
    }
    if !targets.is_empty() {
        s.edit("Remove Hints", |font| {
            for (_, target) in &targets {
                target.edit_layer(font, |l| set_tt_instructions(l, &[]).map_err(|e| p.bad(e.to_string())))?;
            }
            Ok(())
        })?;
    }
    let glyphs: Vec<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();
    Ok(json!({"glyphs": glyphs, "removed": removed}))
}

// ── listing ─────────────────────────────────────────────────────────────────────────────────────

/// A point in the form the commands take it.
fn point_json(p: &TtPoint) -> Value {
    match p {
        TtPoint::Node(r) => json!([r.path, r.node]),
        TtPoint::Intersection { a, b } => json!([a.path, a.node, b.path, b.node]),
        TtPoint::Lsb => json!("lsb"),
        TtPoint::Rsb => json!("rsb"),
    }
}

fn rounding_json(r: Rounding) -> &'static str {
    match r {
        Rounding::Round => "round",
        Rounding::Up => "up",
        Rounding::Down => "down",
        Rounding::NoRounding => "none",
        Rounding::GdiOnly => "gdiOnly",
    }
}

/// `auto`, `none`, or the name of entry `i` of `names` (its index when there is no such entry).
fn reference_json(auto: bool, index: Option<usize>, names: &[String]) -> Value {
    match index {
        Some(i) => names.get(i).map_or_else(|| json!(i), |n| json!(n)),
        None if auto => json!("auto"),
        None => json!("none"),
    }
}

/// The names of the instances and masters at each location key.
fn column_names(font: &Font) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for i in font.instances.iter() {
        out.entry(delta_column_key(&i.location)).or_default().push(i.name.clone());
    }
    for m in font.masters.iter() {
        out.entry(delta_column_key(&m.location)).or_default().push(m.name.clone());
    }
    out
}

fn list_instructions(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.listInstructions", v)?;
    let font = s.font()?;
    let r = read(&p, font)?;
    let layer = r.target.resolve(font)?.layer;
    let hs = settings(&p, font)?;
    let zones: Vec<String> = hs.tt_zones.iter().map(|z| z.name.clone()).collect();
    let stems: Vec<String> = hs.tt_stems.iter().map(|z| z.name.clone()).collect();
    let names = column_names(font);
    let items: Vec<Value> = r
        .list
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut m = Map::new();
            m.insert("index".into(), json!(i));
            let kind = match &t.kind {
                TtKind::Snap { zone } => {
                    let index = if let ZoneRef::Index(z) = zone { Some(*z) } else { None };
                    m.insert("zone".into(), reference_json(*zone == ZoneRef::Auto, index, &zones));
                    "snap"
                }
                TtKind::Stem { stem, triple } => {
                    let index = if let StemRef::Index(z) = stem { Some(*z) } else { None };
                    m.insert("stem".into(), reference_json(*stem == StemRef::Auto, index, &stems));
                    if let Some(g) = triple {
                        m.insert("triple".into(), json!(g));
                    }
                    "stem"
                }
                TtKind::Shift => "shift",
                TtKind::Interpolate => "interpolate",
                TtKind::Delta { cells } => {
                    let deltas: Vec<Value> = cells
                        .iter()
                        .flat_map(|(col, rows)| {
                            let at = names.get(col).cloned().unwrap_or_default();
                            rows.iter().map(move |(ppm, d)| {
                                json!({"column": col, "names": at, "ppm": ppm, "shift": if *d == DeltaDir::Up { "up" } else { "down" }})
                            })
                        })
                        .collect();
                    m.insert("deltas".into(), Value::Array(deltas));
                    "delta"
                }
            };
            m.insert("type".into(), json!(kind));
            m.insert("direction".into(), json!(if t.direction == Direction::Vertical { "vertical" } else { "horizontal" }));
            m.insert("target".into(), point_json(&t.target));
            match (&t.kind, t.origin, t.other) {
                (TtKind::Interpolate, Some(a), Some(b)) => {
                    m.insert("references".into(), json!([point_json(&a), point_json(&b)]));
                }
                (_, Some(o), _) => {
                    m.insert("origin".into(), point_json(&o));
                }
                _ => {}
            }
            if !matches!(t.kind, TtKind::Delta { .. }) {
                m.insert("rounding".into(), json!(rounding_json(t.rounding)));
            }
            Value::Object(m)
        })
        .collect();
    let unmet: Vec<Value> = unmet_needs(&r.list).into_iter().map(|(i, reason)| json!({"instruction": i, "reason": reason})).collect();
    Ok(json!({
        "glyph": p.str("glyph")?,
        "layer": layer.id,
        "mainMaster": layer.master == main_master(font),
        "instructions": items,
        "unmet": unmet,
    }))
}
