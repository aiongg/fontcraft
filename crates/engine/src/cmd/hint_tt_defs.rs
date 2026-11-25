//! TT zone and stem definitions (TTFZones, TTFStems): Get PS zones and stems, add, remove, set.
//!
//! Rows: `hint.tt.getPsZones`, `hint.tt.addZone`, `hint.tt.removeZone`, `hint.tt.setZone`,
//! `hint.tt.getPsStems`, `hint.tt.addStemDefinition`, `hint.tt.removeStemDefinition`,
//! `hint.tt.setStemDefinition`, `hinting.tt.zones.*`, `hinting.tt.stems.*`,
//! `hinting.tt.param.ttfZones`, `hinting.tt.param.ttfStems`.
//!
//! Every command names a master and an entry by name, and is one undo step.
//!
//! | What changes | Where |
//! |---|---|
//! | add, remove, rename; a stem's orientation | every master: each master's parameter is written with the list it uses now, then changed |
//! | a zone's position, size and alignment; a stem's size | the named master |
//! | deltas and filters | the main master only; refused on any other |
//!
//! The lists. A master's list is the one hinting reads ([`HintSettings::tt_zones`],
//! [`HintSettings::tt_stems`]): its TTFZones / TTFStems parameter, else its alignment zones or
//! standard stems, laid out on the main master's list by name (`fontcraft_hinting::settings`).
//! Names are unique within a list; `auto` and `none` are refused as names because the instruction
//! commands use them for the automatic choice and for no zone or stem.
//!
//! Instructions. Snap and Stem instructions name a zone or stem by its index in the main master's
//! list. Removing an entry turns the instructions that name it to Auto and moves later indexes
//! down. A rename leaves them as they are. Zone alignments follow a rename, and a removal clears
//! them. Turning a stem's orientation turns the Stem instructions it no longer suits to Auto. Get
//! PS zones and Get PS stems on the main master keep each instruction's entry by name, else turn
//! it to Auto.
//!
//! Stored form (guess on `hinting.tt.param.ttfZones`): zones `{align, deltas, filter, name,
//! position, size}`, stems `{deltas, filter, horizontal, name, width}` as in the corpus, keys left
//! out when empty; `deltas` = {column: {PPM: 1 | -1}}, a column being a master id or an instance
//! name. Rewriting an entry changes only the keys whose meaning changed, so unknown keys and the
//! source's spellings (`pos`, `size` for a stem's width, a delta of 8) survive.
//!
//! Family zones. With `family: true` in place of `master` the zone commands edit the TTFZones
//! parameter of the font (`hinting.tt.param.ttfFamilyZonesThreshold`): a list of its own, which no
//! master shares and no instruction names. It takes no deltas and no filters.
//!
//! For the TTFZones and TTFStems editors: read a master's list with
//! `HintSettings::for_master(font, master)?.tt_zones` (or `.tt_stems`); show the delta and filter
//! buttons when the master is `fontcraft_hinting::settings::main_master(font)`. Every command
//! returns the master's list after the edit as `{master, zones | stems, instructions}`: a zone is
//! `{name, position, size, align, deltas, filter}`, a stem `{name, orientation: "x" | "y", size,
//! deltas, filter}`, `deltas` is `{column: {PPM: "up" | "down"}}`, and `instructions` counts the
//! instructions whose zone or stem changed.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use fontcraft_hinting::settings::{TTF_STEMS, TTF_ZONES, derived_tt_stems, derived_tt_zones, family_tt_zones, main_master};
use fontcraft_hinting::{
    DeltaDir, Deltas, Direction, HintSettings, MAX_DELTA_COLUMNS, MAX_DELTA_PPM, MAX_TT_STEMS, MAX_TT_ZONES, StemRef, TtInstruction, TtKind, TtStem,
    TtZone, ZoneRef, set_tt_instructions, tt_instructions,
};
use fontcraft_model::axes::number_value;
use fontcraft_model::plist::{Array, Dict, Number, Str, Value as PValue};
use fontcraft_model::{Font, MAX_COORDINATE, MAX_NAME_LEN, Master};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "hint.tt.getPsZones",
            "Get PS zones",
            [],
            None,
            "{master: id | family: true} replace the master's TT zones with its alignment zones (family: the font's TTFZones, the family zones, with the main master's alignment zones), named after their metrics (xHeight, capHeight, ascender, baseline, descender, a named metric's name; other zones zone 1, zone 2…). On the main master, instructions keep their zone by name, else turn to Auto → {master (family: true for the family zones), zones: [zone], instructions: count}",
            has_doc,
            get_ps_zones
        ),
        cmd!(
            "hint.tt.addZone",
            "Add Zone",
            [],
            None,
            "{master: id | family: true, name?: string (default: `zone N`, N one past the list's length, or the next free number), position: number, size: number (positive: top zone; negative: bottom zone), align?: TT zone name} add a TT zone at the end of every master's list, with the same values (family: of the font's TTFZones only); names are unique, and `auto` and `none` are refused → {master (family: true for the family zones), zones: [zone], instructions: count}",
            has_doc,
            add_zone
        ),
        cmd!(
            "hint.tt.removeZone",
            "Remove Zone",
            [],
            None,
            "{master: id | family: true, zone: TT zone name} remove the zone from every master (family: from the font's TTFZones only, no instruction changes). Snap instructions that name it turn to Auto, and zones aligned to it lose their alignment → {master (family: true for the family zones), zones: [zone], instructions: count}",
            has_doc,
            remove_zone
        ),
        cmd!(
            "hint.tt.setZone",
            "Edit Zone",
            [],
            None,
            "{master: id | family: true (the font's TTFZones, without deltas and filter), zone: TT zone name, name?: string, position?: number, size?: number, align?: TT zone name | null, deltas?: {column: {ppm: \"up\" | \"down\" | \"none\"}}, filter?: glyph predicate | null} change a TT zone. A new name applies in every master and must be unique; position, size and alignment belong to `master`; an alignment must not lead back to the zone. Deltas (a column is a master id or an instance name, PPM 1 to 255) and the filter are accepted on the main master only → {master (family: true for the family zones), zones: [zone], instructions: count}",
            has_doc,
            set_zone
        ),
        cmd!(
            "hint.tt.getPsStems",
            "Get PS stems",
            [],
            None,
            "{master: id} replace the master's TT stems with its standard stems, keeping their names (stem 1, stem 2… for unnamed ones); horizontal stems become Y stems and vertical ones X stems. On the main master, instructions keep their stem by name, else turn to Auto → {master, stems: [stem], instructions: count}",
            has_doc,
            get_ps_stems
        ),
        cmd!(
            "hint.tt.addStemDefinition",
            "Add Stem",
            [],
            None,
            "{master: id, name?: string (default: `stem N`, N one past the list's length, or the next free number), orientation: \"x\" (measures a vertical stroke) | \"y\" (a horizontal stroke), size: number above 0} add a TT stem at the end of every master's list, with the same values; names are unique, and `auto` and `none` are refused → {master, stems: [stem], instructions: count}",
            has_doc,
            add_stem
        ),
        cmd!(
            "hint.tt.removeStemDefinition",
            "Remove Stem",
            [],
            None,
            "{master: id, stem: TT stem name} remove the stem from every master. Stem instructions that name it turn to Auto → {master, stems: [stem], instructions: count}",
            has_doc,
            remove_stem
        ),
        cmd!(
            "hint.tt.setStemDefinition",
            "Edit Stem",
            [],
            None,
            "{master: id, stem: TT stem name, name?: string, orientation?: \"x\" | \"y\", size?: number above 0, deltas?: {column: {ppm: \"up\" | \"down\" | \"none\"}}, filter?: glyph predicate | null} change a TT stem. A new name and the orientation apply in every master; Stem instructions the new orientation does not suit turn to Auto. The size belongs to `master`. Deltas (a column is a master id or an instance name, PPM 1 to 255) and the filter are accepted on the main master only → {master, stems: [stem], instructions: count}",
            has_doc,
            set_stem
        ),
    ]
}

// ── zones and stems behind one interface ────────────────────────────────────────────────────────

/// What the commands need from a TT zone or a TT stem.
trait Def: Clone + Default {
    /// The master parameter that holds the list.
    const PARAM: &'static str;
    /// The entry in messages and default names.
    const WHAT: &'static str;
    /// The key of the list in a command's result.
    const KEY: &'static str;
    const LIMIT: usize;

    fn name(&self) -> &str;
    fn name_mut(&mut self) -> &mut String;
    fn deltas_mut(&mut self) -> &mut Deltas;
    fn filter_mut(&mut self) -> &mut Option<String>;
    fn listed(s: &HintSettings) -> &[Self];
    fn derived(font: &Font, m: &Master) -> Vec<Self>;
    /// The stored keys: the spellings read (the first is written) and the value, `None` for none.
    fn fields(&self) -> Vec<(&'static [&'static str], Option<PValue>)>;
    fn json(&self) -> Value;
    /// Re-points the zone or stem `kind` names through `map` (`None`: Auto); whether it changed.
    fn repoint(kind: &mut TtKind, direction: Direction, map: &dyn Fn(usize, Direction) -> Option<usize>) -> bool;
    /// Follows a rename of another entry (zone alignments).
    fn renamed(&mut self, _old: &str, _new: &str) {}
    /// Forgets a removed entry (zone alignments).
    fn removed(&mut self, _name: &str) {}
}

impl Def for TtZone {
    const PARAM: &'static str = TTF_ZONES;
    const WHAT: &'static str = "zone";
    const KEY: &'static str = "zones";
    const LIMIT: usize = MAX_TT_ZONES;

    fn name(&self) -> &str {
        &self.name
    }
    fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }
    fn deltas_mut(&mut self) -> &mut Deltas {
        &mut self.deltas
    }
    fn filter_mut(&mut self) -> &mut Option<String> {
        &mut self.filter
    }
    fn listed(s: &HintSettings) -> &[Self] {
        &s.tt_zones
    }
    fn derived(font: &Font, m: &Master) -> Vec<Self> {
        derived_tt_zones(font, m)
    }
    fn fields(&self) -> Vec<(&'static [&'static str], Option<PValue>)> {
        vec![
            field(&["align"], self.align.as_deref().map(text)),
            field(&["deltas"], deltas_value(&self.deltas)),
            field(&["filter"], self.filter.as_deref().map(text)),
            field(&["name"], Some(text(&self.name))),
            field(&["position", "pos"], Some(number(self.position))),
            field(&["size"], Some(number(self.size))),
        ]
    }
    fn json(&self) -> Value {
        json!({"name": self.name, "position": self.position, "size": self.size, "align": self.align, "deltas": deltas_json(&self.deltas), "filter": self.filter})
    }
    fn repoint(kind: &mut TtKind, direction: Direction, map: &dyn Fn(usize, Direction) -> Option<usize>) -> bool {
        let TtKind::Snap { zone } = kind else { return false };
        let ZoneRef::Index(i) = zone else { return false };
        let new = map(*i, direction).map_or(ZoneRef::Auto, ZoneRef::Index);
        let changed = new != *zone;
        *zone = new;
        changed
    }
    fn renamed(&mut self, old: &str, new: &str) {
        if self.align.as_deref() == Some(old) {
            self.align = Some(new.to_string());
        }
    }
    fn removed(&mut self, name: &str) {
        if self.align.as_deref() == Some(name) {
            self.align = None;
        }
    }
}

impl Def for TtStem {
    const PARAM: &'static str = TTF_STEMS;
    const WHAT: &'static str = "stem";
    const KEY: &'static str = "stems";
    const LIMIT: usize = MAX_TT_STEMS;

    fn name(&self) -> &str {
        &self.name
    }
    fn name_mut(&mut self) -> &mut String {
        &mut self.name
    }
    fn deltas_mut(&mut self) -> &mut Deltas {
        &mut self.deltas
    }
    fn filter_mut(&mut self) -> &mut Option<String> {
        &mut self.filter
    }
    fn listed(s: &HintSettings) -> &[Self] {
        &s.tt_stems
    }
    fn derived(font: &Font, m: &Master) -> Vec<Self> {
        derived_tt_stems(font, m)
    }
    fn fields(&self) -> Vec<(&'static [&'static str], Option<PValue>)> {
        vec![
            field(&["deltas"], deltas_value(&self.deltas)),
            field(&["filter"], self.filter.as_deref().map(text)),
            field(&["horizontal"], self.horizontal.then(|| PValue::Number(Number::int(1)))),
            field(&["name"], Some(text(&self.name))),
            field(&["width", "size"], Some(number(self.size))),
        ]
    }
    fn json(&self) -> Value {
        let orientation = if self.horizontal { "y" } else { "x" };
        json!({"name": self.name, "orientation": orientation, "size": self.size, "deltas": deltas_json(&self.deltas), "filter": self.filter})
    }
    fn repoint(kind: &mut TtKind, direction: Direction, map: &dyn Fn(usize, Direction) -> Option<usize>) -> bool {
        let TtKind::Stem { stem, .. } = kind else { return false };
        let StemRef::Index(i) = stem else { return false };
        let new = map(*i, direction).map_or(StemRef::Auto, StemRef::Index);
        let changed = new != *stem;
        *stem = new;
        changed
    }
}

fn field(keys: &'static [&'static str], value: Option<PValue>) -> (&'static [&'static str], Option<PValue>) {
    (keys, value)
}

fn text(s: &str) -> PValue {
    PValue::String(Str::new(s))
}

/// Values are checked finite before they get here; 0 stands in for anything else.
fn number(v: f64) -> PValue {
    number_value(v).unwrap_or(PValue::Number(Number::int(0)))
}

fn deltas_value(deltas: &Deltas) -> Option<PValue> {
    let mut table = Dict::new();
    for (column, rows) in deltas.iter().filter(|(_, rows)| !rows.is_empty()) {
        let mut out = Dict::new();
        for (ppm, dir) in rows {
            out.entries.push((Str::new(ppm.to_string()), PValue::Number(Number::int(if *dir == DeltaDir::Up { 1 } else { -1 }))));
        }
        table.entries.push((Str::new(column.clone()), PValue::Dict(out)));
    }
    (!table.is_empty()).then_some(PValue::Dict(table))
}

/// Stored deltas read as directions, to tell whether a rewrite changes their meaning.
fn stored_deltas(v: &PValue) -> Option<Deltas> {
    let mut out = Deltas::new();
    for (column, rows) in &v.as_dict()?.entries {
        let mut cells = BTreeMap::new();
        for (ppm, shift) in &rows.as_dict()?.entries {
            let ppm = ppm.as_str().trim().parse::<u16>().ok()?;
            let shift = match shift {
                PValue::Number(n) => n.value().as_f64(),
                PValue::String(s) => s.as_str().trim().parse::<f64>().ok()?,
                _ => return None,
            };
            if shift != 0.0 {
                cells.insert(ppm, if shift > 0.0 { DeltaDir::Up } else { DeltaDir::Down });
            }
        }
        if !cells.is_empty() {
            out.insert(column.as_str().to_string(), cells);
        }
    }
    Some(out)
}

fn deltas_json(deltas: &Deltas) -> Value {
    let mut out = Map::new();
    for (column, rows) in deltas.iter().filter(|(_, rows)| !rows.is_empty()) {
        let cells: Map<String, Value> =
            rows.iter().map(|(ppm, dir)| (ppm.to_string(), json!(if *dir == DeltaDir::Up { "up" } else { "down" }))).collect();
        out.insert(column.clone(), Value::Object(cells));
    }
    Value::Object(out)
}

/// Whether the stored value of `key` already says what `fresh` says.
fn keeps(key: &str, stored: &PValue, fresh: Option<&PValue>) -> bool {
    if fresh == Some(stored) {
        return true;
    }
    match (key, fresh) {
        ("deltas", _) => stored_deltas(stored).is_some_and(|d| Some(d) == fresh.map_or(Some(Deltas::new()), stored_deltas)),
        ("horizontal", None) => fontcraft_model::param_flag(stored) == Some(false),
        ("align" | "filter", None) => stored.as_str() == Some(""),
        (_, Some(PValue::Number(_))) => {
            stored.as_str().and_then(|s| s.trim().parse::<f64>().ok()).zip(fresh.and_then(PValue::as_num)).is_some_and(|(a, b)| a == b.as_f64())
        }
        _ => false,
    }
}

/// `stored` with `fields` written over it: changed keys replaced in place, new ones appended,
/// empty ones removed; every other key kept.
fn merge(stored: Option<Dict>, fields: Vec<(&'static [&'static str], Option<PValue>)>) -> Dict {
    let mut d = stored.unwrap_or_default();
    for (keys, value) in fields {
        let at = d.entries.iter().position(|(k, _)| keys.contains(&k.as_str()));
        match at {
            Some(i) => {
                let Some((key, old)) = d.entries.get(i) else { continue };
                if keeps(key.as_str(), old, value.as_ref()) {
                    continue;
                }
                match value {
                    Some(v) => {
                        if let Some(slot) = d.entries.get_mut(i) {
                            slot.1 = v;
                        }
                    }
                    None => {
                        d.remove_at(i);
                    }
                }
            }
            None => {
                if let (Some(v), Some(key)) = (value, keys.first()) {
                    d.insert(*key, v);
                }
            }
        }
    }
    d
}

// ── one master's list, read and written ─────────────────────────────────────────────────────────

/// An entry and the stored dictionary it was read from.
struct Slot<T> {
    stored: Option<Dict>,
    def: T,
}

/// One master's list as hinting reads it, with what the parameter stores.
struct List<T> {
    master: String,
    main: bool,
    slots: Vec<Slot<T>>,
    /// Stored entries the list does not use (names the main master lacks); written back after it.
    extras: Vec<(String, PValue)>,
    /// The stored list, for its layout.
    array: Option<Array>,
}

impl<T: Def> List<T> {
    fn position(&self, name: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.def.name() == name)
    }

    fn names(&self) -> Vec<String> {
        self.slots.iter().map(|s| s.def.name().to_string()).collect()
    }
}

fn no_master(p: &Params, master: &str) -> EngineError {
    p.bad(format!("the font has no master `{master}`"))
}

fn hint_settings(p: &Params, font: &Font, master: &str) -> Result<HintSettings> {
    HintSettings::for_master(font, master).map_err(|e| p.bad(e.to_string()))
}

/// The name an entry is read under (the hinting crate's rule).
fn stored_name(d: &Dict, index: usize, what: &str) -> String {
    match d.get("name") {
        Some(v) => v.as_str().map(|n| n.chars().take(MAX_NAME_LEN).collect()).unwrap_or_default(),
        None => format!("{what} {}", index.saturating_add(1)),
    }
}

fn load<T: Def>(p: &Params, font: &Font, master: &str) -> Result<List<T>> {
    let m = font.master(master).map_err(|_| no_master(p, master))?;
    let settings = hint_settings(p, font, master)?;
    let main = settings.main_master == master;
    let array = m.params.get(T::PARAM).and_then(PValue::as_array).cloned();
    let mut stored: Vec<Option<(String, Dict)>> =
        array.iter().flat_map(|a| a.items.iter()).enumerate().map(|(i, v)| v.as_dict().map(|d| (stored_name(d, i, T::WHAT), d.clone()))).collect();
    let mut slots = Vec::new();
    for (i, def) in T::listed(&settings).iter().enumerate() {
        // The main master's list is its parameter in order; another master's is matched by name.
        let found = if main && array.is_some() {
            stored.get_mut(i).and_then(Option::take)
        } else {
            stored.iter_mut().find(|s| s.as_ref().is_some_and(|(n, _)| n == def.name())).and_then(Option::take)
        };
        slots.push(Slot { stored: found.map(|(_, d)| d), def: def.clone() });
    }
    let extras = stored.into_iter().flatten().map(|(n, d)| (n, PValue::Dict(d))).collect();
    Ok(List { master: master.to_string(), main, slots, extras, array })
}

/// Every master's list, read before any is written.
fn load_all<T: Def>(p: &Params, font: &Font, master: &str) -> Result<Vec<List<T>>> {
    font.master(master).map_err(|_| no_master(p, master))?;
    font.masters.iter().map(|m| load(p, font, &m.id)).collect()
}

fn store<T: Def>(p: &Params, font: &mut Font, list: List<T>) -> Result<()> {
    let names: HashSet<String> = list.names().into_iter().collect();
    let main = list.main;
    let mut items: Vec<PValue> = list
        .slots
        .into_iter()
        .map(|s| {
            let mut def = s.def;
            if !main {
                def.deltas_mut().clear();
                *def.filter_mut() = None;
            }
            PValue::Dict(merge(s.stored, def.fields()))
        })
        .collect();
    items.extend(list.extras.into_iter().filter(|(n, _)| !names.contains(n)).map(|(_, v)| v));
    let mut array = list.array.unwrap_or_default();
    array.items = items;
    let m = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == list.master).ok_or_else(|| no_master(p, &list.master))?;
    m.params.set(T::PARAM, PValue::Array(array));
    Ok(())
}

fn store_all<T: Def>(p: &Params, font: &mut Font, lists: Vec<List<T>>) -> Result<()> {
    lists.into_iter().try_for_each(|l| store(p, font, l))
}

/// The list of `master` in `lists`.
fn of<'a, T: Def>(p: &Params, lists: &'a mut [List<T>], master: &str) -> Result<&'a mut List<T>> {
    lists.iter_mut().find(|l| l.master == master).ok_or_else(|| no_master(p, master))
}

/// Re-points the Snap (zones) or Stem (stems) instructions of every layer through `map`; returns
/// how many changed. Layers whose instructions cannot be read are left alone.
fn repoint<T: Def>(p: &Params, font: &mut Font, map: &dyn Fn(usize, Direction) -> Option<usize>) -> Result<usize> {
    let mut changes: Vec<(usize, usize, Vec<TtInstruction>)> = Vec::new();
    let mut count = 0usize;
    for (g, glyph) in font.glyphs.iter().enumerate() {
        for (l, layer) in glyph.layers.iter().enumerate() {
            if layer.info.hints.is_empty() {
                continue;
            }
            let Ok(read) = tt_instructions(layer) else { continue };
            let mut list: Vec<TtInstruction> = read.into_iter().map(|(_, t)| t).collect();
            let mut changed = 0usize;
            for t in &mut list {
                if T::repoint(&mut t.kind, t.direction, map) {
                    changed = changed.saturating_add(1);
                }
            }
            if changed > 0 {
                count = count.saturating_add(changed);
                changes.push((g, l, list));
            }
        }
    }
    for (g, l, list) in changes {
        let mut glyph = font.glyphs.get_mut_at(g).ok_or_else(|| p.bad("a glyph went missing while instructions were updated"))?;
        let name = glyph.name.clone();
        let layer = glyph.layers.get_mut(l).ok_or_else(|| p.bad("a layer went missing while instructions were updated"))?;
        set_tt_instructions(layer, &list).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
    }
    Ok(count)
}

/// The command's result: the master's list after the edit.
fn result<T: Def>(p: &Params, font: &Font, master: &str, instructions: usize) -> Result<Value> {
    let s = hint_settings(p, font, master)?;
    let mut out = Map::new();
    out.insert("master".into(), json!(master));
    out.insert(T::KEY.into(), Value::Array(T::listed(&s).iter().map(T::json).collect()));
    out.insert("instructions".into(), json!(instructions));
    Ok(Value::Object(out))
}

// ── parameters ──────────────────────────────────────────────────────────────────────────────────

fn checked_name(p: &Params, key: &str, text: &str) -> Result<String> {
    let name = text.trim();
    if name.is_empty() {
        return Err(p.bad(format!("`{key}` is empty")));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(p.bad(format!("`{key}` is longer than {MAX_NAME_LEN} characters")));
    }
    if name == "auto" || name == "none" {
        return Err(p.bad(format!("`{name}` cannot be a name: instructions use it for the automatic choice or for none")));
    }
    Ok(name.to_string())
}

/// `name` for a new entry: the one given, or the first free `zone N` / `stem N`.
fn new_name<T: Def>(p: &Params, list: &List<T>) -> Result<String> {
    let taken: HashSet<String> = list.names().into_iter().collect();
    let name = match p.opt_str("name")? {
        Some(text) => checked_name(p, "name", text)?,
        None => {
            // At most `len` names are taken, so one of these `len + 1` is free.
            let start = list.slots.len().saturating_add(1);
            (start..=start.saturating_add(list.slots.len())).map(|n| format!("{} {n}", T::WHAT)).find(|n| !taken.contains(n)).unwrap_or_default()
        }
    };
    if taken.contains(&name) {
        return Err(p.bad(format!("there is already a TT {} named `{name}`; names are unique", T::WHAT)));
    }
    Ok(name)
}

fn coordinate(p: &Params, key: &str) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        Some(v) if v.abs() > MAX_COORDINATE => Err(p.bad(format!("`{key}` is beyond ±{MAX_COORDINATE}"))),
        other => Ok(other),
    }
}

fn stem_size(p: &Params) -> Result<Option<f64>> {
    match coordinate(p, "size")? {
        Some(v) if v <= 0.0 => Err(p.bad("`size` must be above 0")),
        other => Ok(other),
    }
}

/// `orientation`: whether the stem is horizontal (measured along y).
fn orientation(p: &Params) -> Result<Option<bool>> {
    match p.opt_str("orientation")? {
        None => Ok(None),
        Some("x") => Ok(Some(false)),
        Some("y") => Ok(Some(true)),
        Some(other) => Err(p.bad(format!("`orientation` must be \"x\" or \"y\", got \"{other}\""))),
    }
}

/// Whether `key` is given as `null` (or as an empty string), which clears it.
fn cleared(v: &Value, key: &str) -> bool {
    v.get(key).is_some_and(|x| x.is_null() || x.as_str().is_some_and(|s| s.trim().is_empty()))
}

/// Refuses deltas and filters outside the main master.
fn main_only(p: &Params, v: &Value, font: &Font, master: &str) -> Result<bool> {
    let wanted = v.get("deltas").is_some_and(|d| !d.is_null()) || v.get("filter").is_some();
    let main = main_master(font);
    if wanted && main != master {
        return Err(p.bad(format!("deltas and filters belong to the main master (`{main}`), not to `{master}`")));
    }
    Ok(wanted)
}

/// Applies the `deltas` cells to `deltas`.
fn apply_deltas(p: &Params, font: &Font, deltas: &mut Deltas) -> Result<()> {
    let Some(table) = p.opt_object("deltas")? else { return Ok(()) };
    if table.len() > MAX_DELTA_COLUMNS {
        return Err(p.bad(format!("`deltas` has more than {MAX_DELTA_COLUMNS} columns")));
    }
    for (column, rows) in table {
        if !font.masters.iter().any(|m| &m.id == column) && !font.instances.iter().any(|i| &i.name == column) {
            return Err(p.bad(format!("`deltas`: the column `{column}` is neither a master id nor an instance name")));
        }
        let rows = rows.as_object().ok_or_else(|| p.bad(format!("`deltas`: the column `{column}` is not an object of PPM sizes")))?;
        if rows.len() > usize::from(MAX_DELTA_PPM) {
            return Err(p.bad(format!("`deltas`: the column `{column}` has more than {MAX_DELTA_PPM} sizes")));
        }
        let cells = deltas.entry(column.clone()).or_default();
        for (ppm, dir) in rows {
            let size = ppm
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|s| (1..=MAX_DELTA_PPM).contains(s))
                .ok_or_else(|| p.bad(format!("`deltas`: `{ppm}` is not a PPM size from 1 to {MAX_DELTA_PPM}")))?;
            match dir.as_str() {
                Some("up") => {
                    cells.insert(size, DeltaDir::Up);
                }
                Some("down") => {
                    cells.insert(size, DeltaDir::Down);
                }
                Some("none") => {
                    cells.remove(&size);
                }
                _ => return Err(p.bad(format!("`deltas`: the cell at {ppm} PPM must be \"up\", \"down\" or \"none\", got {dir}"))),
            }
        }
        if cells.is_empty() {
            deltas.remove(column);
        }
    }
    if deltas.len() > MAX_DELTA_COLUMNS {
        return Err(p.bad(format!("the deltas would have more than {MAX_DELTA_COLUMNS} columns")));
    }
    Ok(())
}

/// The `filter` to set: `Some(None)` clears it; the predicate must parse.
fn filter(p: &Params, v: &Value) -> Result<Option<Option<String>>> {
    if cleared(v, "filter") {
        return Ok(Some(None));
    }
    let Some(text) = p.opt_str("filter")? else { return Ok(None) };
    fontcraft_glyphquery::parse_predicate(text).map_err(|e| p.bad(format!("`filter`: {e}")))?;
    Ok(Some(Some(text.to_string())))
}

/// Applies `deltas` and `filter` to the entry, on the main master.
fn apply_main_only<T: Def>(p: &Params, v: &Value, font: &Font, def: &mut T) -> Result<()> {
    let mut deltas = def.deltas_mut().clone();
    apply_deltas(p, font, &mut deltas)?;
    *def.deltas_mut() = deltas;
    if let Some(f) = filter(p, v)? {
        *def.filter_mut() = f;
    }
    Ok(())
}

// ── shared command bodies ───────────────────────────────────────────────────────────────────────

fn get_ps<T: Def>(s: &mut Session, v: &Value, id: &'static str, label: &str) -> Result<Value> {
    let p = Params::new(id, v)?;
    let master = p.str("master")?;
    s.edit(label, |font| {
        let m = font.master(master).map_err(|_| no_master(&p, master))?;
        let fresh = T::derived(font, m);
        if fresh.len() > T::LIMIT {
            return Err(p.bad(format!("the master has more than {} PS {}s", T::LIMIT, T::WHAT)));
        }
        // On the main master instructions follow their entry by name; an unreadable old list
        // leaves them as they are.
        let main = main_master(font) == master;
        let old: Option<Vec<String>> = if main {
            HintSettings::for_master(font, master).ok().map(|s| T::listed(&s).iter().map(|d| d.name().to_string()).collect())
        } else {
            None
        };
        let new: Vec<String> = fresh.iter().map(|d| d.name().to_string()).collect();
        let list = List {
            master: master.to_string(),
            main,
            slots: fresh.into_iter().map(|def| Slot { stored: None, def }).collect(),
            extras: Vec::new(),
            array: None,
        };
        store(&p, font, list)?;
        let changed = match old {
            Some(old) => repoint::<T>(&p, font, &|i, _| old.get(i).and_then(|n| new.iter().position(|x| x == n)))?,
            None => 0,
        };
        result::<T>(&p, font, master, changed)
    })
}

fn remove<T: Def>(s: &mut Session, v: &Value, id: &'static str, label: &str) -> Result<Value> {
    let p = Params::new(id, v)?;
    let master = p.str("master")?;
    let name = p.str(T::WHAT)?;
    s.edit(label, |font| {
        let mut lists = load_all::<T>(&p, font, master)?;
        let k = of(&p, &mut lists, master)?.position(name).ok_or_else(|| p.bad(format!("the master has no TT {} named `{name}`", T::WHAT)))?;
        for list in &mut lists {
            if let Some(i) = list.position(name) {
                list.slots.remove(i);
            }
            for slot in &mut list.slots {
                slot.def.removed(name);
            }
        }
        store_all(&p, font, lists)?;
        let changed = repoint::<T>(&p, font, &|i, _| match i.cmp(&k) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i.saturating_sub(1)),
        })?;
        result::<T>(&p, font, master, changed)
    })
}

/// Adds `def` (named by the `name` parameter) at the end of every master's list.
fn add<T: Def>(p: &Params, font: &mut Font, master: &str, def: T, check: impl Fn(&List<T>) -> Result<()>) -> Result<Value> {
    let mut lists = load_all::<T>(p, font, master)?;
    let target = of(p, &mut lists, master)?;
    if target.slots.len() >= T::LIMIT {
        return Err(p.bad(format!("a master has at most {} TT {}s", T::LIMIT, T::WHAT)));
    }
    let name = new_name(p, target)?;
    check(target)?;
    for list in &mut lists {
        let mut def = def.clone();
        *def.name_mut() = name.clone();
        list.slots.push(Slot { stored: None, def });
    }
    store_all(p, font, lists)?;
    result::<T>(p, font, master, 0)
}

/// Renames entry `old` of every list to the `name` parameter, when it differs; returns the
/// entry's name afterwards.
fn rename<T: Def>(p: &Params, lists: &mut [List<T>], master: &str, old: &str) -> Result<String> {
    let Some(text) = p.opt_str("name")? else { return Ok(old.to_string()) };
    let new = checked_name(p, "name", text)?;
    if new == old {
        return Ok(new);
    }
    if of(p, lists, master)?.position(&new).is_some() {
        return Err(p.bad(format!("there is already a TT {} named `{new}`; names are unique", T::WHAT)));
    }
    for list in lists.iter_mut() {
        for slot in &mut list.slots {
            if slot.def.name() == old {
                *slot.def.name_mut() = new.clone();
            } else {
                slot.def.renamed(old, &new);
            }
        }
    }
    Ok(new)
}

// ── the commands ────────────────────────────────────────────────────────────────────────────────

fn get_ps_zones(s: &mut Session, v: &Value) -> Result<Value> {
    if is_family(&Params::new("hint.tt.getPsZones", v)?)? {
        return family_get_ps(s, v);
    }
    get_ps::<TtZone>(s, v, "hint.tt.getPsZones", "Get PS zones")
}

fn get_ps_stems(s: &mut Session, v: &Value) -> Result<Value> {
    get_ps::<TtStem>(s, v, "hint.tt.getPsStems", "Get PS stems")
}

fn remove_zone(s: &mut Session, v: &Value) -> Result<Value> {
    if is_family(&Params::new("hint.tt.removeZone", v)?)? {
        return family_remove(s, v);
    }
    remove::<TtZone>(s, v, "hint.tt.removeZone", "Remove Zone")
}

fn remove_stem(s: &mut Session, v: &Value) -> Result<Value> {
    remove::<TtStem>(s, v, "hint.tt.removeStemDefinition", "Remove Stem")
}

fn add_zone(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.addZone", v)?;
    let family = is_family(&p)?;
    let master = if family { "" } else { p.str("master")? };
    let position = coordinate(&p, "position")?.ok_or_else(|| p.bad("`position` is required"))?;
    let size = coordinate(&p, "size")?.ok_or_else(|| p.bad("`size` is required"))?;
    let align = p.opt_str("align")?.map(str::trim).filter(|a| !a.is_empty());
    let def = TtZone { position, size, align: align.map(str::to_string), ..TtZone::default() };
    let check = |list: &List<TtZone>| match align {
        Some(a) if list.position(a).is_none() => Err(p.bad(format!("`align`: there is no TT zone named `{a}`"))),
        _ => Ok(()),
    };
    s.edit("Add Zone", |font| {
        if !family {
            return add(&p, font, master, def, check);
        }
        let mut list = family_load(&p, font)?;
        if list.slots.len() >= MAX_TT_ZONES {
            return Err(p.bad(format!("a font has at most {MAX_TT_ZONES} family TT zones")));
        }
        let name = new_name(&p, &list)?;
        check(&list)?;
        list.slots.push(Slot { stored: None, def: TtZone { name, ..def } });
        family_store(&p, font, list)
    })
}

fn add_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.addStemDefinition", v)?;
    let master = p.str("master")?;
    let horizontal = orientation(&p)?.ok_or_else(|| p.bad("`orientation` is required"))?;
    let size = stem_size(&p)?.ok_or_else(|| p.bad("`size` is required"))?;
    let def = TtStem { horizontal, size, ..TtStem::default() };
    s.edit("Add Stem", |font| add(&p, font, master, def, |_| Ok(())))
}

// ── family zones ────────────────────────────────────────────────────────────────────────────────

/// Whether the command edits the family zones (`family`) rather than a master's.
fn is_family(p: &Params) -> Result<bool> {
    let family = p.bool_or("family", false)?;
    if family && p.get("master").is_some_and(|m| !m.is_null()) {
        return Err(p.bad("give `master` or `family`, not both"));
    }
    Ok(family)
}

/// The font's TTFZones as a list of no master; its stored deltas and filters are kept.
fn family_load(p: &Params, font: &Font) -> Result<List<TtZone>> {
    let zones = family_tt_zones(font).map_err(|e| p.bad(e.to_string()))?;
    let array = font.info.params.get(TTF_ZONES).and_then(PValue::as_array).cloned();
    let stored = array.iter().flat_map(|a| a.items.iter()).map(|v| v.as_dict().cloned()).chain(std::iter::repeat(None));
    let slots = zones.into_iter().zip(stored).map(|(def, stored)| Slot { stored, def }).collect();
    Ok(List { master: String::new(), main: true, slots, extras: Vec::new(), array })
}

/// Writes the font's TTFZones and gives the command's result.
fn family_store(p: &Params, font: &mut Font, list: List<TtZone>) -> Result<Value> {
    let mut array = list.array.unwrap_or_default();
    array.items = list.slots.into_iter().map(|s| PValue::Dict(merge(s.stored, s.def.fields()))).collect();
    Arc::make_mut(&mut font.info).params.set(TTF_ZONES, PValue::Array(array));
    let zones = family_tt_zones(font).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({"family": true, "zones": zones.iter().map(Def::json).collect::<Vec<_>>(), "instructions": 0}))
}

fn family_get_ps(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.getPsZones", v)?;
    s.edit("Get PS zones", |font| {
        let main = main_master(font);
        let zones = derived_tt_zones(font, font.master(&main).map_err(|_| no_master(&p, &main))?);
        if zones.len() > MAX_TT_ZONES {
            return Err(p.bad(format!("the main master has more than {MAX_TT_ZONES} PS zones")));
        }
        let slots = zones.into_iter().map(|z| Slot { stored: None, def: TtZone { filter: None, ..z } }).collect();
        family_store(&p, font, List { master: String::new(), main: true, slots, extras: Vec::new(), array: None })
    })
}

fn family_remove(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.removeZone", v)?;
    let name = p.str("zone")?;
    s.edit("Remove Zone", |font| {
        let mut list = family_load(&p, font)?;
        let k = list.position(name).ok_or_else(|| p.bad(format!("there is no family TT zone named `{name}`")))?;
        list.slots.remove(k);
        for slot in &mut list.slots {
            slot.def.removed(name);
        }
        family_store(&p, font, list)
    })
}

/// Refuses an alignment of zone `k` that leads back to it.
fn check_alignment(p: &Params, list: &List<TtZone>, k: usize) -> Result<()> {
    let Some(start) = list.slots.get(k).map(|s| s.def.name.clone()) else { return Ok(()) };
    let mut at = list.slots.get(k).and_then(|s| s.def.align.clone());
    // Each step visits another zone, so the walk ends within the list's length.
    for _ in 0..=list.slots.len() {
        let Some(name) = at else { return Ok(()) };
        if name == start {
            return Err(p.bad(format!("`align`: zone `{start}` would be aligned to itself through its alignments")));
        }
        at = list.slots.iter().find(|s| s.def.name == name).and_then(|s| s.def.align.clone());
    }
    Ok(())
}

fn set_zone(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setZone", v)?;
    let family = is_family(&p)?;
    let master = if family { "" } else { p.str("master")? };
    let zone = p.str("zone")?;
    let position = coordinate(&p, "position")?;
    let size = coordinate(&p, "size")?;
    let align = if cleared(v, "align") { Some(None) } else { p.opt_str("align")?.map(|a| Some(a.trim().to_string())) };
    let renaming = p.opt_str("name")?.is_some();
    s.edit("Edit Zone", |font| {
        let main_only_wanted = !family && main_only(&p, v, font, master)?;
        if family && (p.get("deltas").is_some() || p.get("filter").is_some()) {
            return Err(p.bad("family TT zones have no deltas and no filter"));
        }
        if !renaming && position.is_none() && size.is_none() && align.is_none() && !main_only_wanted {
            return Err(p.bad("give at least one of `name`, `position`, `size`, `align`, `deltas` and `filter`"));
        }
        let mut lists = match (family, renaming) {
            (true, _) => vec![family_load(&p, font)?],
            (false, true) => load_all::<TtZone>(&p, font, master)?,
            (false, false) => vec![load::<TtZone>(&p, font, master)?],
        };
        of(&p, &mut lists, master)?.position(zone).ok_or_else(|| p.bad(format!("there is no TT zone named `{zone}`")))?;
        let name = rename(&p, &mut lists, master, zone)?;
        let target = of(&p, &mut lists, master)?;
        let k = target.position(&name).ok_or_else(|| p.bad(format!("there is no TT zone named `{name}`")))?;
        if let Some(Some(a)) = &align
            && (a == &name || target.position(a).is_none())
        {
            return Err(p.bad(format!("`align`: `{a}` is not another TT zone of the list")));
        }
        let slot = target.slots.get_mut(k).ok_or_else(|| p.bad("the zone went missing"))?;
        if let Some(x) = position {
            slot.def.position = x;
        }
        if let Some(x) = size {
            slot.def.size = x;
        }
        if let Some(a) = align {
            slot.def.align = a;
        }
        apply_main_only(&p, v, font, &mut slot.def)?;
        check_alignment(&p, target, k)?;
        if family {
            let list = lists.pop().ok_or_else(|| p.bad("the family zones went missing"))?;
            return family_store(&p, font, list);
        }
        store_all(&p, font, lists)?;
        result::<TtZone>(&p, font, master, 0)
    })
}

fn set_stem(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.setStemDefinition", v)?;
    let master = p.str("master")?;
    let stem = p.str("stem")?;
    let size = stem_size(&p)?;
    let horizontal = orientation(&p)?;
    let renaming = p.opt_str("name")?.is_some();
    s.edit("Edit Stem", |font| {
        let main_only_wanted = main_only(&p, v, font, master)?;
        if !renaming && size.is_none() && horizontal.is_none() && !main_only_wanted {
            return Err(p.bad("give at least one of `name`, `orientation`, `size`, `deltas` and `filter`"));
        }
        let every = renaming || horizontal.is_some();
        let mut lists = if every { load_all::<TtStem>(&p, font, master)? } else { vec![load::<TtStem>(&p, font, master)?] };
        of(&p, &mut lists, master)?.position(stem).ok_or_else(|| p.bad(format!("the master has no TT stem named `{stem}`")))?;
        let name = rename(&p, &mut lists, master, stem)?;
        let k = of(&p, &mut lists, master)?.position(&name).ok_or_else(|| p.bad(format!("the master has no TT stem named `{name}`")))?;
        let turned = match horizontal {
            Some(h) => {
                let mut turned = false;
                for list in &mut lists {
                    if let Some(slot) = list.slots.get_mut(k) {
                        turned |= slot.def.horizontal != h;
                        slot.def.horizontal = h;
                    }
                }
                turned.then_some(h)
            }
            None => None,
        };
        let slot = of(&p, &mut lists, master)?.slots.get_mut(k).ok_or_else(|| p.bad("the stem went missing"))?;
        if let Some(x) = size {
            slot.def.size = x;
        }
        apply_main_only(&p, v, font, &mut slot.def)?;
        store_all(&p, font, lists)?;
        // A vertical instruction suits a horizontal stem, a horizontal instruction a vertical one.
        let changed = match turned {
            Some(h) => repoint::<TtStem>(&p, font, &|i, dir| (i != k || h == (dir == Direction::Vertical)).then_some(i))?,
            None => 0,
        };
        result::<TtStem>(&p, font, master, changed)
    })
}
