//! TTX reader for `mort` and `morx`, the Apple glyph metamorphosis tables (M12.7d).
//!
//! The text is read into the state-table model of the writer (`misc::morph`) and compiled as
//! fontTools compiles it: `morx` state tables in the extended form, with equal entries stored
//! once, ligature action sequences and insertion glyph runs shared with the longer sequences
//! they end or contain, and per-glyph lookups stored once each; `mort` state tables in the
//! original form, with a class array, 8-bit entry indices and per-glyph substitution arrays. AAT
//! lookups take the smallest of formats 0, 2, 6 and 8. Ported from fontTools' `ttLib.tables`
//! (MIT): the `MortSubtableConverter`, `MorxSubtableConverter`, `STXHeader`, `StateHeader` and
//! `AATLookup` converters of `otConverters`, and the AAT actions of `otTables`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{Result, TablesError};

use super::super::Element;
use super::super::misc::morph::{Budget, Kind, MAX_GLYPH_CLASSES, MAX_STATES, StateTable, Transition, per_glyph_count};
use super::super::read::{Out, ReadContext, int_lit};
use super::super::read_otdata::build::version_value;

fn bad(what: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("glyph metamorphosis table: {what}"))
}

fn attr<'a>(e: &'a Element, name: &str) -> Result<&'a str> {
    e.attribute(name).ok_or_else(|| bad(format!("the element '{}' has no '{name}'", e.name)))
}

fn value(e: &Element) -> Result<i64> {
    int_lit(attr(e, "value")?)
}

fn missing(what: &str) -> TablesError {
    bad(format!("no {what}"))
}

fn len_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `mort` (`legacy`) or `morx` from its table element.
pub(super) fn compile(e: &Element, ctx: &ReadContext, legacy: bool) -> Result<Vec<u8>> {
    let tag = if legacy { "mort" } else { "morx" };
    let (mut version, mut reserved, mut chains) = (None, None, Vec::new());
    for c in &e.children {
        match c.name.as_str() {
            "Version" => version = Some(attr(c, "value")?),
            "Reserved" if !legacy => reserved = Some(value(c)?),
            "MorphChainCount" => {}
            "MorphChain" => chains.push(c),
            other => return Err(bad(format!("'{tag}' has no field '{other}'"))),
        }
    }
    let version = version.ok_or_else(|| missing("Version"))?;
    let mut o = Out::default();
    if legacy {
        o.i32(version_value(version)?, "the mort version")?;
    } else {
        o.u16(int_lit(version)?, "the morx version")?;
        o.u16(reserved.ok_or_else(|| missing("Reserved"))?, "Reserved")?;
    }
    o.u32(len_i64(chains.len()), "the number of chains")?;
    let mut budget = Budget::default();
    for c in chains {
        o.bytes(&chain(c, ctx, legacy, &mut budget)?);
    }
    Ok(o.0)
}

fn chain(e: &Element, ctx: &ReadContext, legacy: bool, budget: &mut Budget) -> Result<Vec<u8>> {
    let (mut default_flags, mut features, mut subtables) = (None, Vec::new(), Vec::new());
    for c in &e.children {
        match c.name.as_str() {
            "DefaultFlags" => default_flags = Some(value(c)?),
            "StructLength" | "MorphFeatureCount" | "MorphSubtableCount" => {}
            "MorphFeature" => features.push(c),
            "MorphSubtable" => subtables.push(c),
            other => return Err(bad(format!("a chain has no field '{other}'"))),
        }
    }
    let mut body = Out::default();
    for f in &features {
        budget.charge(1)?;
        body.bytes(&feature(f)?);
    }
    for s in &subtables {
        budget.charge(1)?;
        body.bytes(&if legacy { mort_subtable(s, ctx, budget)? } else { morx_subtable(s, ctx, budget)? });
    }
    let header = if legacy { 12 } else { 16 };
    let mut o = Out::default();
    o.u32(default_flags.ok_or_else(|| missing("DefaultFlags"))?, "DefaultFlags")?;
    o.u32(len_i64(body.0.len().saturating_add(header)), "the length of a chain")?;
    if legacy {
        o.u16(len_i64(features.len()), "the number of features")?;
        o.u16(len_i64(subtables.len()), "the number of subtables")?;
    } else {
        o.u32(len_i64(features.len()), "the number of features")?;
        o.u32(len_i64(subtables.len()), "the number of subtables")?;
    }
    o.bytes(&body.0);
    Ok(o.0)
}

fn feature(e: &Element) -> Result<Vec<u8>> {
    let mut fields = [None; 4];
    const NAMES: [&str; 4] = ["FeatureType", "FeatureSetting", "EnableFlags", "DisableFlags"];
    for c in &e.children {
        let slot = NAMES.iter().position(|n| *n == c.name).and_then(|i| fields.get_mut(i));
        let Some(slot) = slot else {
            return Err(bad(format!("a feature has no field '{}'", c.name)));
        };
        *slot = Some(value(c)?);
    }
    let mut o = Out::default();
    for (i, (name, v)) in NAMES.iter().zip(fields).enumerate() {
        let v = v.ok_or_else(|| missing(name))?;
        if i < 2 { o.u16(v, name)? } else { o.u32(v, name)? }
    }
    Ok(o.0)
}

const ORDERS: [&str; 4] = ["LayoutOrder", "ReversedLayoutOrder", "LogicalOrder", "ReversedLogicalOrder"];

/// The processing order coverage flags give (bits 0x40 and 0x10).
fn order_of(flags: i64) -> &'static str {
    let index = usize::from(flags & 0x40 != 0) + 2 * usize::from(flags & 0x10 != 0);
    ORDERS.get(index).copied().unwrap_or("LayoutOrder")
}

fn direction_of(flags: i64) -> &'static str {
    if flags & 0x20 != 0 {
        "Any"
    } else if flags & 0x80 != 0 {
        "Vertical"
    } else {
        "Horizontal"
    }
}

/// `MorxSubtableConverter.xmlRead` and `write`.
fn morx_subtable(e: &Element, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    let (mut coverage, mut reserved, mut direction, mut order, mut sub_flags, mut body) = (0i64, 0i64, None, None, None, None);
    for c in &e.children {
        match c.name.as_str() {
            "CoverageFlags" => {
                coverage = value(c)?;
                order = Some(order_of(coverage));
                direction = Some(direction_of(coverage));
            }
            "ProcessingOrder" => {
                let v = attr(c, "value")?;
                order = Some(ORDERS.iter().find(|o| **o == v).copied().ok_or_else(|| bad(format!("unknown ProcessingOrder: {v}")))?);
            }
            "TextDirection" => {
                let v = attr(c, "value")?;
                direction =
                    Some(["Horizontal", "Vertical", "Any"].into_iter().find(|d| *d == v).ok_or_else(|| bad(format!("unknown TextDirection {v}")))?);
            }
            "Reserved" => reserved = value(c)?,
            "SubFeatureFlags" => sub_flags = Some(value(c)?),
            name if name.ends_with("Morph") => body = Some(c),
            other => return Err(bad(format!("a subtable has no field '{other}'"))),
        }
    }
    let reserved = ((coverage & 0xF) << 16) | reserved;
    let direction = direction.ok_or_else(|| missing("TextDirection"))?;
    let order = order.ok_or_else(|| missing("ProcessingOrder"))?;
    let position = ORDERS.iter().position(|o| *o == order).unwrap_or(0);
    let mut flags = (reserved & 0x000F_0000) >> 16;
    if direction == "Vertical" {
        flags |= 0x80;
    }
    if position % 2 == 1 {
        flags |= 0x40;
    }
    if direction == "Any" {
        flags |= 0x20;
    }
    if position >= 2 {
        flags |= 0x10;
    }
    let (morph_type, data) = morph_body(body.ok_or_else(|| missing("subtable body"))?, ctx, false, budget)?;
    let mut o = Out::default();
    o.u32(len_i64(data.len().saturating_add(12)), "the length of a subtable")?;
    o.u8(flags, "CoverageFlags")?;
    o.u16(reserved & 0xFFFF, "Reserved")?;
    o.u8(i64::from(morph_type), "MorphType")?;
    o.u32(sub_flags.ok_or_else(|| missing("SubFeatureFlags"))?, "SubFeatureFlags")?;
    o.bytes(&data);
    Ok(o.0)
}

/// `MortSubtableConverter.xmlRead` and `write`.
fn mort_subtable(e: &Element, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    let (mut coverage, mut sub_flags, mut body) = (0i64, None, None);
    for c in &e.children {
        match c.name.as_str() {
            "CoverageFlags" => coverage = value(c)?,
            "SubFeatureFlags" => sub_flags = Some(value(c)?),
            name if name.ends_with("Morph") => body = Some(c),
            other => return Err(bad(format!("a subtable has no field '{other}'"))),
        }
    }
    let (morph_type, data) = morph_body(body.ok_or_else(|| missing("subtable body"))?, ctx, true, budget)?;
    let length = data.len().saturating_add(8);
    if length > 0xFFFF {
        return Err(bad("mort subtable exceeds 65535 bytes"));
    }
    let mut o = Out::default();
    o.u16(len_i64(length), "the length of a subtable")?;
    o.u8(coverage, "CoverageFlags")?;
    o.u8(i64::from(morph_type), "MorphType")?;
    o.u32(sub_flags.ok_or_else(|| missing("SubFeatureFlags"))?, "SubFeatureFlags")?;
    o.bytes(&data);
    Ok(o.0)
}

/// The subtable type and the compiled body of a `…Morph` element.
fn morph_body(e: &Element, ctx: &ReadContext, legacy: bool, budget: &mut Budget) -> Result<(u8, Vec<u8>)> {
    let kind = match e.name.as_str() {
        "RearrangementMorph" => Kind::Rearrangement,
        "ContextualMorph" => Kind::Contextual,
        "LigatureMorph" => Kind::Ligature,
        "InsertionMorph" => Kind::Insertion,
        "NoncontextualMorph" => {
            let mut lookup = None;
            for c in &e.children {
                if c.name != "Substitution" {
                    return Err(bad(format!("'NoncontextualMorph' has no field '{}'", c.name)));
                }
                lookup = Some(glyph_map(c)?);
            }
            let lookup = lookup.ok_or_else(|| missing("Substitution"))?;
            return Ok((4, glyph_lookup_bytes(&lookup, ctx, budget)?));
        }
        other => return Err(bad(format!("unknown subtable '{other}'"))),
    };
    let mut table = None;
    for c in &e.children {
        if c.name != "StateTable" {
            return Err(bad(format!("'{}' has no field '{}'", e.name, c.name)));
        }
        table = Some(state_table(c, kind, budget)?);
    }
    let table = table.ok_or_else(|| missing("StateTable"))?;
    let data = if legacy { compile_legacy(&table, kind, ctx, budget)? } else { compile_extended(&table, kind, ctx, budget)? };
    Ok((kind.morph_type(), data))
}

// ── The state-table model from text ────────────────────────────────────────────────────────────

/// `AATLookup(GlyphID).xmlRead`: glyph to glyph, a later entry replacing an earlier one.
fn glyph_map(e: &Element) -> Result<BTreeMap<String, String>> {
    let mut map = BTreeMap::new();
    for c in e.children.iter().filter(|c| c.name == "Lookup") {
        map.insert(attr(c, "glyph")?.to_string(), attr(c, "value")?.to_string());
    }
    Ok(map)
}

/// `STXHeader.xmlRead`.
fn state_table(e: &Element, kind: Kind, budget: &mut Budget) -> Result<StateTable> {
    let mut t = StateTable::default();
    for c in &e.children {
        match c.name.as_str() {
            "GlyphClass" => {
                t.classes.insert(attr(c, "glyph")?.to_string(), value(c)?);
            }
            "State" => {
                if len_i64(t.states.len()) >= MAX_STATES {
                    return Err(TablesError::TooLarge { what: "the states of a state table", limit: MAX_STATES as usize });
                }
                let mut state = BTreeMap::new();
                for tr in c.children.iter().filter(|t| t.name == "Transition") {
                    budget.charge(1)?;
                    state.insert(int_lit(attr(tr, "onGlyphClass")?)?, transition(tr, kind)?);
                }
                t.states.push(state);
            }
            "PerGlyphLookup" => {
                if kind != Kind::Contextual {
                    return Err(bad("only contextual subtables have per-glyph lookups"));
                }
                t.per_glyph.push(glyph_map(c)?);
            }
            "LigComponents" => {
                t.lig_components = Some(c.children.iter().filter(|x| x.name == "LigComponent").map(value).collect::<Result<_>>()?);
            }
            "MortLigatureRebase" => {
                let indices = c.children.iter().filter(|x| x.name == "Component").map(|x| int_lit(attr(x, "index")?));
                t.rebase = Some(indices.collect::<Result<_>>()?);
            }
            "Ligatures" => {
                let glyphs = c.children.iter().filter(|x| x.name == "Ligature").map(|x| attr(x, "glyph").map(str::to_string));
                t.ligatures = Some(glyphs.collect::<Result<_>>()?);
            }
            _ => {}
        }
    }
    let highest = t.classes.values().copied().chain(t.states.iter().flat_map(|s| s.keys().copied())).max().unwrap_or(-1);
    t.class_count = highest.saturating_add(1);
    Ok(t)
}

/// The `fromXML` of the transition of each kind.
fn transition(e: &Element, kind: Kind) -> Result<Transition> {
    let mut t = Transition::default();
    for c in &e.children {
        match (kind, c.name.as_str()) {
            (_, "NewState") => t.new_state = value(c)?,
            (_, "Flags") => {
                for flag in attr(c, "value")?.split(',') {
                    let flag = flag.trim();
                    let bit =
                        kind.flags().iter().find(|(n, _)| *n == flag).map(|(_, b)| *b).ok_or_else(|| bad(format!("unsupported flag {flag}")))?;
                    t.flags |= bit;
                }
            }
            (Kind::Rearrangement | Kind::Contextual | Kind::Ligature, "ReservedFlags") => t.reserved = value(c)?,
            (Kind::Rearrangement, "Verb") => t.verb = value(c)?,
            (Kind::Contextual, "MarkIndex") => t.mark_index = value(c)?,
            (Kind::Contextual, "CurrentIndex") => t.current_index = value(c)?,
            (Kind::Ligature, "Action") => {
                let delta = int_lit(attr(c, "GlyphIndexDelta")?)?;
                let store = c.attribute("Flags").unwrap_or("").split(',').any(|f| f.trim() == "Store");
                t.actions.push((delta, store));
            }
            (Kind::Insertion, "CurrentInsertionAction") => t.current_insertion.push(attr(c, "glyph")?.to_string()),
            (Kind::Insertion, "MarkedInsertionAction") => t.marked_insertion.push(attr(c, "glyph")?.to_string()),
            (Kind::Insertion, other) => return Err(bad(format!("an insertion transition has no field '{other}'"))),
            _ => {}
        }
    }
    Ok(t)
}

// ── AAT lookups ────────────────────────────────────────────────────────────────────────────────

/// fontTools' `getSearchRange`.
fn search_range(n: usize, item_size: usize) -> [usize; 3] {
    let exponent = (usize::BITS - n.leading_zeros()).saturating_sub(1) as usize;
    let range = (1usize << exponent.min(30)).saturating_mul(item_size);
    [range, exponent, n.saturating_mul(item_size).saturating_sub(range)]
}

/// `AATLookup.write` of 16-bit values: `entries` are (glyph id, the value as fontTools compares
/// it, the value's bytes), sorted here; formats 0, 2, 6 and 8 are sized and the smallest is
/// written (the lowest format on a tie).
fn lookup_bytes<K: Ord>(mut entries: Vec<(u32, K, u16)>, glyphs: usize) -> Result<Vec<u8>> {
    entries.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let (Some(first), Some(last)) = (entries.first(), entries.last()) else {
        return Err(bad("an AAT lookup with no entry"));
    };
    let (first_gid, last_gid) = (first.0, last.0);
    let mut segments: Vec<(u32, u32, usize)> = Vec::new();
    for (i, (gid, key, _)) in entries.iter().enumerate() {
        let extends = segments.last().is_some_and(|(_, end, at)| *gid == end.saturating_add(1) && entries.get(*at).is_some_and(|e| e.1 == *key));
        match segments.last_mut() {
            Some(segment) if extends => segment.1 = *gid,
            _ => segments.push((*gid, *gid, i)),
        }
    }
    let n = entries.len();
    let mut sizes: Vec<(usize, u16)> = vec![(12 + (segments.len() + 1) * 6, 2), (12 + (n + 1) * 4, 6)];
    if n == glyphs {
        sizes.push((2 + n * 2, 0));
    }
    if u64::try_from(n).unwrap_or(u64::MAX) == u64::from(last_gid) - u64::from(first_gid) + 1 {
        sizes.push((6 + n * 2, 8));
    }
    sizes.sort_unstable();
    let format = sizes.first().map_or(6, |s| s.1);
    let mut o = Out::default();
    o.u16(i64::from(format), "an AAT lookup format")?;
    let header = |o: &mut Out, units: usize, unit: usize| -> Result<()> {
        o.u16(len_i64(unit), "an AAT lookup unit size")?;
        o.u16(len_i64(units), "the number of AAT lookup units")?;
        for v in search_range(units, unit) {
            o.u16(len_i64(v), "an AAT lookup search range")?;
        }
        Ok(())
    };
    match format {
        0 => {
            for (_, _, v) in &entries {
                o.u16(i64::from(*v), "an AAT lookup value")?;
            }
        }
        2 => {
            header(&mut o, segments.len(), 6)?;
            for (start, end, at) in &segments {
                o.u16(i64::from(*end), "a glyph id")?;
                o.u16(i64::from(*start), "a glyph id")?;
                o.u16(i64::from(entries.get(*at).map_or(0, |e| e.2)), "an AAT lookup value")?;
            }
            o.bytes(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
        }
        8 => {
            o.u16(i64::from(first_gid), "a glyph id")?;
            o.u16(len_i64(n), "the number of glyphs")?;
            for (_, _, v) in &entries {
                o.u16(i64::from(*v), "an AAT lookup value")?;
            }
        }
        _ => {
            header(&mut o, n, 4)?;
            for (gid, _, v) in &entries {
                o.u16(i64::from(*gid), "a glyph id")?;
                o.u16(i64::from(*v), "an AAT lookup value")?;
            }
            o.bytes(&[0xFF, 0xFF, 0, 0]);
        }
    }
    Ok(o.0)
}

fn gid16(ctx: &ReadContext, name: &str) -> Result<u16> {
    let id = ctx.glyph_id(name)?;
    u16::try_from(id).map_err(|_| bad(format!("the glyph id {id} does not fit 16 bits")))
}

fn glyph_lookup_bytes(lookup: &BTreeMap<String, String>, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    budget.charge(lookup.len())?;
    let entries = lookup.iter().map(|(g, v)| Ok((ctx.glyph_id(g)?, v.as_str(), gid16(ctx, v)?))).collect::<Result<Vec<_>>>()?;
    lookup_bytes(entries, ctx.order()?.len())
}

fn pad(data: &mut Vec<u8>, size: usize) {
    while !data.len().is_multiple_of(size) {
        data.push(0);
    }
}

fn aligned(v: usize, alignment: usize) -> usize {
    v.saturating_add((alignment - v % alignment) % alignment)
}

// ── Shared actions ─────────────────────────────────────────────────────────────────────────────

/// The action data of a state table and where each sequence starts (fontTools'
/// `compileActions`): ligature action sequences by their bytes, in 32-bit units; insertion glyph
/// runs by their glyphs, in 16-bit units.
#[derive(Default)]
struct Actions {
    data: Vec<u8>,
    ligatures: HashMap<Vec<u8>, i64>,
    insertions: HashMap<Vec<u32>, i64>,
    /// Glyph names numbered in sorted order, so runs compare as fontTools compares them.
    ranks: BTreeMap<String, u32>,
}

impl Actions {
    fn run(&self, glyphs: &[String]) -> Vec<u32> {
        glyphs.iter().map(|g| self.ranks.get(g).copied().unwrap_or(0)).collect()
    }
}

/// `LigatureMorphAction.compileLigActions`.
fn ligature_action_bytes(actions: &[(i64, bool)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(actions.len() * 4);
    for (i, (delta, store)) in actions.iter().enumerate() {
        let mut v = (*delta & 0x3FFF_FFFF) as u32;
        if i + 1 == actions.len() {
            v |= 0x8000_0000;
        }
        if *store {
            v |= 0x4000_0000;
        }
        out.extend_from_slice(&v.to_be_bytes());
    }
    out
}

fn compile_actions(kind: Kind, states: &[BTreeMap<i64, Transition>], ctx: &ReadContext, budget: &mut Budget) -> Result<Actions> {
    let mut a = Actions::default();
    let transitions = || states.iter().flat_map(|s| s.values());
    match kind {
        Kind::Ligature => {
            let set: BTreeSet<Vec<u8>> = transitions().map(|t| ligature_action_bytes(&t.actions)).collect();
            let mut sequences: Vec<Vec<u8>> = set.into_iter().collect();
            sequences.sort_by(|x, y| y.len().cmp(&x.len()).then_with(|| x.cmp(y)));
            for s in sequences {
                if a.ligatures.contains_key(&s) {
                    continue;
                }
                for i in (0..s.len()).step_by(4) {
                    budget.charge((s.len() - i) / 4)?;
                    let at = len_i64((a.data.len() + i) / 4);
                    a.ligatures.entry(s.get(i..).unwrap_or_default().to_vec()).or_insert(at);
                }
                a.data.extend_from_slice(&s);
            }
        }
        Kind::Insertion => {
            for t in transitions() {
                for g in t.current_insertion.iter().chain(&t.marked_insertion) {
                    a.ranks.entry(g.clone()).or_insert(0);
                }
            }
            for (i, rank) in a.ranks.values_mut().enumerate() {
                *rank = u32::try_from(i).unwrap_or(u32::MAX);
            }
            let names: Vec<&String> = a.ranks.keys().collect();
            let set: BTreeSet<Vec<u32>> = transitions().flat_map(|t| [a.run(&t.current_insertion), a.run(&t.marked_insertion)]).collect();
            let mut runs: Vec<Vec<u32>> = set.into_iter().collect();
            runs.sort_by(|x, y| y.len().cmp(&x.len()).then_with(|| x.cmp(y)));
            for run in runs {
                if a.insertions.contains_key(&run) {
                    continue;
                }
                for start in 0..run.len() {
                    let at = len_i64(a.data.len() / 2 + start);
                    for limit in start..run.len() {
                        budget.charge(limit - start + 1)?;
                        a.insertions.entry(run.get(start..=limit).unwrap_or_default().to_vec()).or_insert(at);
                    }
                }
                for rank in &run {
                    let name = names.get(*rank as usize).map_or("", |n| n.as_str());
                    a.data.extend_from_slice(&gid16(ctx, name)?.to_be_bytes());
                }
            }
        }
        Kind::Rearrangement | Kind::Contextual => {}
    }
    Ok(a)
}

/// The compile of a transition into its extended entry (fontTools' `compile` of each action).
fn extended_entry(t: &Transition, kind: Kind, actions: &Actions) -> Result<Vec<u8>> {
    let mut o = Out::default();
    o.u16(t.new_state, "NewState")?;
    let named = i64::from(t.flags);
    match kind {
        Kind::Rearrangement => {
            if !(0..=15).contains(&t.verb) {
                return Err(bad(format!("the verb {} is not between 0 and 15", t.verb)));
            }
            o.u16(t.verb | t.reserved | named, "the flags of a transition")?;
        }
        Kind::Contextual => {
            o.u16(t.reserved | named, "the flags of a transition")?;
            o.u16(t.mark_index, "MarkIndex")?;
            o.u16(t.current_index, "CurrentIndex")?;
        }
        Kind::Ligature => {
            let perform = if t.actions.is_empty() { 0 } else { 0x2000 };
            o.u16(t.reserved | named | perform, "the flags of a transition")?;
            let index = if t.actions.is_empty() {
                0
            } else {
                actions.ligatures.get(&ligature_action_bytes(&t.actions)).copied().ok_or_else(|| bad("a ligature action without an index"))?
            };
            o.u16(index, "a ligature action index")?;
        }
        Kind::Insertion => {
            let counts = (len_i64(t.current_insertion.len()) << 5) | len_i64(t.marked_insertion.len());
            o.u16(t.reserved | named | counts, "the flags of a transition")?;
            for glyphs in [&t.current_insertion, &t.marked_insertion] {
                let index = if glyphs.is_empty() {
                    0xFFFF
                } else {
                    actions.insertions.get(&actions.run(glyphs)).copied().ok_or_else(|| bad("an insertion without an index"))?
                };
                o.u16(index, "an insertion action index")?;
            }
        }
    }
    Ok(o.0)
}

/// The classes of a state table as (glyph id, class) entries.
fn class_entries(t: &StateTable, ctx: &ReadContext) -> Result<Vec<(u32, i64)>> {
    t.classes.iter().map(|(g, c)| Ok((ctx.glyph_id(g)?, *c))).collect()
}

fn transition_at(state: &BTreeMap<i64, Transition>, class: i64) -> Result<&Transition> {
    state.get(&class).ok_or_else(|| bad(format!("a state has no transition on glyph class {class}")))
}

// ── Extended state tables (`morx`) ─────────────────────────────────────────────────────────────

/// `STXHeader.write`.
fn compile_extended(t: &StateTable, kind: Kind, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    let glyphs = ctx.order()?.len();
    let classes = class_entries(t, ctx)?;
    budget.charge(classes.len())?;
    let class_lookup = classes
        .iter()
        .map(|(g, c)| Ok((*g, *c, u16::try_from(*c).map_err(|_| bad(format!("the glyph class {c} does not fit 16 bits")))?)))
        .collect::<Result<Vec<_>>>()?;
    let mut class_data = lookup_bytes(class_lookup, glyphs)?;
    pad(&mut class_data, 2);
    let class_count = classes.iter().map(|(_, c)| *c).max().unwrap_or(0).saturating_add(1);
    if class_count > MAX_GLYPH_CLASSES {
        return Err(TablesError::TooLarge { what: "the glyph classes of a state table", limit: MAX_GLYPH_CLASSES as usize });
    }
    let class_table_offset = 16
        + match kind {
            Kind::Contextual => 4,
            Kind::Ligature => 12,
            Kind::Insertion => 4,
            Kind::Rearrangement => 0,
        };
    let actions = compile_actions(kind, &t.states, ctx, budget)?;
    let mut state_data = Out::default();
    let (mut entry_data, mut entry_ids) = (Vec::new(), HashMap::new());
    for state in &t.states {
        for class in 0..class_count {
            budget.charge(1)?;
            let entry = extended_entry(transition_at(state, class)?, kind, &actions)?;
            let next = entry_ids.len();
            let id = *entry_ids.entry(entry.clone()).or_insert_with(|| {
                entry_data.extend_from_slice(&entry);
                next
            });
            state_data.u16(len_i64(id), "an entry index")?;
        }
    }
    let mut state_data = state_data.0;
    pad(&mut state_data, 4);
    pad(&mut entry_data, 4);
    let state_offset = class_table_offset + class_data.len();
    let entry_offset = state_offset + state_data.len();
    let per_glyph_offset = entry_offset + entry_data.len();
    let mut per_glyph = if kind == Kind::Contextual { per_glyph_lookups(t, ctx, budget)? } else { Vec::new() };
    pad(&mut per_glyph, 4);
    let action_offset = matches!(kind, Kind::Ligature | Kind::Insertion).then_some(entry_offset + entry_data.len());
    let components = match &t.lig_components {
        Some(c) => {
            let mut o = Out::default();
            for v in c {
                o.u16(*v, "a ligature component")?;
            }
            Some(o.0)
        }
        None => None,
    };
    let ligatures = match &t.ligatures {
        Some(l) => {
            let mut o = Out::default();
            for g in l {
                o.u16(i64::from(gid16(ctx, g)?), "a ligature glyph")?;
            }
            Some(o.0)
        }
        None => None,
    };
    let mut o = Out::default();
    for v in [class_count, len_i64(class_table_offset), len_i64(state_offset), len_i64(entry_offset)] {
        o.u32(v, "a state table header")?;
    }
    if kind == Kind::Contextual {
        o.u32(len_i64(per_glyph_offset), "the per-glyph table offset")?;
    }
    if let Some(at) = action_offset {
        o.u32(len_i64(at), "the action offset")?;
    }
    if let Some(c) = &components {
        if !per_glyph.is_empty() {
            return Err(bad("a contextual subtable with ligature components"));
        }
        let at = action_offset.ok_or_else(|| bad("ligature components in a subtable without actions"))?;
        let component_offset = at + actions.data.len();
        o.u32(len_i64(component_offset), "the component offset")?;
        o.u32(len_i64(component_offset + c.len()), "the ligature offset")?;
    }
    o.bytes(&class_data);
    o.bytes(&state_data);
    o.bytes(&entry_data);
    o.bytes(&per_glyph);
    if action_offset.is_some() {
        o.bytes(&actions.data);
    }
    o.bytes(&components.unwrap_or_default());
    o.bytes(&ligatures.unwrap_or_default());
    Ok(o.0)
}

/// `_compilePerGlyphLookups`: a 32-bit offset per lookup, then each different lookup once, in
/// the order of its last use (fontTools' table packing).
fn per_glyph_lookups(t: &StateTable, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    let count = per_glyph_count(t);
    if u32::try_from(t.per_glyph.len()).ok() != Some(count) {
        return Err(bad(format!("the actions refer to {count} per-glyph lookups, but the table has {}", t.per_glyph.len())));
    }
    let mut unique: Vec<Vec<u8>> = Vec::new();
    let mut ids = Vec::with_capacity(t.per_glyph.len());
    for lookup in &t.per_glyph {
        let bytes = glyph_lookup_bytes(lookup, ctx, budget)?;
        let id = match unique.iter().position(|u| *u == bytes) {
            Some(id) => id,
            None => {
                unique.push(bytes);
                unique.len() - 1
            }
        };
        ids.push(id);
    }
    let mut last_use: Vec<(usize, usize)> = (0..unique.len()).map(|u| (ids.iter().rposition(|i| *i == u).unwrap_or(0), u)).collect();
    last_use.sort_unstable();
    let mut position = vec![0usize; unique.len()];
    let mut at = ids.len() * 4;
    for (_, u) in &last_use {
        if let Some(p) = position.get_mut(*u) {
            *p = at;
        }
        at += unique.get(*u).map_or(0, Vec::len);
    }
    let mut o = Out::default();
    for id in &ids {
        o.u32(len_i64(position.get(*id).copied().unwrap_or(0)), "a per-glyph lookup offset")?;
    }
    for (_, u) in &last_use {
        o.bytes(unique.get(*u).map_or(&[][..], Vec::as_slice));
    }
    Ok(o.0)
}

// ── Original state tables (`mort`) ─────────────────────────────────────────────────────────────

/// Python's list indexing: a negative index counts from the end.
fn py_get<T: Copy>(list: &[T], index: i64) -> Result<T> {
    let i = if index < 0 { index + len_i64(list.len()) } else { index };
    usize::try_from(i).ok().and_then(|i| list.get(i)).copied().ok_or_else(|| bad("list index out of range"))
}

/// `StateHeader.write`.
fn compile_legacy(t: &StateTable, kind: Kind, ctx: &ReadContext, budget: &mut Budget) -> Result<Vec<u8>> {
    let class_count = t.class_count;
    if !(1..=256).contains(&class_count) {
        return Err(bad("mort state tables support 1 to 256 glyph classes"));
    }
    let mut classes = class_entries(t, ctx)?;
    classes.sort_unstable();
    let mut class_data = Out::default();
    match (classes.first(), classes.last()) {
        (Some((first, _)), Some((last, _))) => {
            let span = u64::from(*last) - u64::from(*first) + 1;
            if span > 0xFFFF {
                return Err(bad("a mort class table covers more than 65535 glyphs"));
            }
            budget.charge(span as usize)?;
            let values: BTreeMap<u32, i64> = classes.iter().copied().collect();
            class_data.u16(i64::from(*first), "the first glyph of a class table")?;
            class_data.u16(span as i64, "the length of a class table")?;
            for gid in *first..=*last {
                let class = values.get(&gid).copied().unwrap_or(1);
                class_data.u8(class, "a glyph class")?;
            }
        }
        _ => class_data.bytes(&[0, 0, 0, 0]),
    }
    let class_data = class_data.0;
    let actions = compile_actions(kind, &t.states, ctx, budget)?;
    let (mut entries, mut entry_ids, mut rows): (Vec<&Transition>, HashMap<Vec<u8>, usize>, Vec<u8>) = (Vec::new(), HashMap::new(), Vec::new());
    for state in &t.states {
        for class in 0..class_count {
            budget.charge(1)?;
            let transition = transition_at(state, class)?;
            let key = extended_entry(transition, kind, &actions)?;
            let id = match entry_ids.get(&key) {
                Some(id) => *id,
                None => {
                    let id = entries.len();
                    if id > 0xFF {
                        return Err(bad("mort state tables support at most 256 entries"));
                    }
                    entry_ids.insert(key, id);
                    entries.push(transition);
                    id
                }
            };
            rows.push(id as u8);
        }
    }
    let extra_size = match kind {
        Kind::Contextual | Kind::Insertion => 2,
        Kind::Ligature => 6,
        Kind::Rearrangement => 0,
    };
    let class_table_offset = 8 + extra_size;
    let state_offset = aligned(class_table_offset + class_data.len(), 2);
    let entry_offset = aligned(state_offset + rows.len(), 2);
    let entry_size = if kind == Kind::Ligature { 4 } else { kind.entry_size() };
    let tail_offset = entry_offset.saturating_add(entries.len().saturating_mul(entry_size));
    let legacy = Legacy { class_count, state_offset, actions: &actions };
    let (extra, tail, context, sequences) = match kind {
        Kind::Contextual => {
            let (extra, tail, offsets) = contextual_lookups(t, ctx, tail_offset, budget)?;
            (extra, tail, offsets, HashMap::new())
        }
        Kind::Ligature => {
            let (extra, tail, sequences) = ligature_data(t, ctx, &entries, tail_offset, budget)?;
            (extra, tail, Vec::new(), sequences)
        }
        Kind::Insertion => {
            if tail_offset > 0xFFFF {
                return Err(bad("mort insertion-action offset exceeds 16 bits"));
            }
            (u16::try_from(tail_offset).unwrap_or(0).to_be_bytes().to_vec(), actions.data.clone(), Vec::new(), HashMap::new())
        }
        Kind::Rearrangement => (Vec::new(), Vec::new(), Vec::new(), HashMap::new()),
    };
    let mut entry_data = Vec::new();
    for transition in &entries {
        entry_data.extend_from_slice(&legacy.entry(transition, kind, &context, &sequences)?);
    }
    let mut o = Out::default();
    for v in [class_count, len_i64(class_table_offset), len_i64(state_offset), len_i64(entry_offset)] {
        o.u16(v, "a mort state table header")?;
    }
    o.bytes(&extra);
    o.bytes(&class_data);
    let mut data = o.0;
    pad(&mut data, 2);
    data.extend_from_slice(&rows);
    pad(&mut data, 2);
    data.extend_from_slice(&entry_data);
    data.extend_from_slice(&tail);
    pad(&mut data, 4);
    if data.len() > 0xFFFF {
        return Err(bad("mort state table exceeds 65535 bytes"));
    }
    Ok(data)
}

/// What compiling a `mort` entry needs.
struct Legacy<'a> {
    class_count: i64,
    state_offset: usize,
    actions: &'a Actions,
}

impl Legacy<'_> {
    /// `_compileTransition`.
    fn entry(&self, t: &Transition, kind: Kind, context: &[i64], sequences: &Sequences) -> Result<Vec<u8>> {
        let new_state = t.new_state.checked_mul(self.class_count).and_then(|v| v.checked_add(len_i64(self.state_offset)));
        let new_state = new_state.filter(|v| (0..=0xFFFF).contains(v)).ok_or_else(|| bad("mort new-state offset exceeds 16 bits"))?;
        let mut o = Out::default();
        o.u16(new_state, "a new state")?;
        let named = i64::from(t.flags);
        match kind {
            Kind::Contextual => {
                o.u16(t.reserved | named, "the flags of a transition")?;
                for index in [t.mark_index, t.current_index] {
                    let offset = if index == 0xFFFF { 0 } else { py_get(context, index)? };
                    o.i16(offset, "a substitution offset")?;
                }
            }
            Kind::Ligature => {
                if t.reserved != 0 {
                    return Err(bad("mort ligature entries cannot encode reserved flags"));
                }
                o.u16(named | sequences.get(&t.actions).copied().unwrap_or(0), "the flags of a transition")?;
            }
            Kind::Rearrangement | Kind::Insertion => {
                let entry = extended_entry(t, kind, self.actions)?;
                o.bytes(entry.get(2..).unwrap_or_default());
            }
        }
        Ok(o.0)
    }
}

/// `_compileContextualLookups`: a substitution array over the whole glyph order per lookup.
fn contextual_lookups(t: &StateTable, ctx: &ReadContext, tail_offset: usize, budget: &mut Budget) -> Result<(Vec<u8>, Vec<u8>, Vec<i64>)> {
    let count = per_glyph_count(t);
    if u32::try_from(t.per_glyph.len()).ok() != Some(count) {
        return Err(bad(format!("mort contextual actions refer to {count} lookups, but the table has {}", t.per_glyph.len())));
    }
    let order = ctx.order()?;
    let (mut offsets, mut data) = (Vec::new(), Out::default());
    for lookup in &t.per_glyph {
        let offset = (tail_offset.saturating_add(data.0.len())) / 2;
        if offset > 0x7FFF {
            return Err(bad("mort substitution offset exceeds signed 16 bits"));
        }
        offsets.push(len_i64(offset));
        budget.charge(order.len())?;
        for glyph in order {
            let replacement = match lookup.get(glyph) {
                Some(r) => i64::from(gid16(ctx, r)?),
                None => 0,
            };
            data.u16(replacement, "a substitution")?;
        }
    }
    if tail_offset > 0xFFFF {
        return Err(bad("mort substitution-table offset exceeds 16 bits"));
    }
    Ok((u16::try_from(tail_offset).unwrap_or(0).to_be_bytes().to_vec(), data.0, offsets))
}

/// `_compileLigatureData`: the action, component and ligature tables after the entries, and
/// where each action sequence starts.
/// Where each `mort` ligature action sequence starts.
type Sequences = HashMap<Vec<(i64, bool)>, i64>;

fn ligature_data(
    t: &StateTable,
    ctx: &ReadContext,
    entries: &[&Transition],
    tail_offset: usize,
    budget: &mut Budget,
) -> Result<(Vec<u8>, Vec<u8>, Sequences)> {
    let components = t.lig_components.as_ref().ok_or_else(|| missing("LigComponents"))?;
    let ligatures = t.ligatures.as_ref().ok_or_else(|| missing("Ligatures"))?;
    let mut sequences: Vec<&Vec<(i64, bool)>> = Vec::new();
    for e in entries {
        if !e.actions.is_empty() && !sequences.contains(&&e.actions) {
            sequences.push(&e.actions);
        }
    }
    let action_offset = aligned(tail_offset, 4);
    let action_count: usize = sequences.iter().map(|s| s.len()).sum();
    let component_offset = action_offset.saturating_add(action_count.saturating_mul(4));
    let ligature_offset = component_offset.saturating_add(components.len().saturating_mul(2));
    if action_offset.max(component_offset).max(ligature_offset) > 0xFFFF {
        return Err(bad("mort ligature-table offset exceeds 16 bits"));
    }
    let glyph_count = len_i64(ctx.order()?.len());
    let mut rebase: BTreeSet<i64> = t.rebase.clone().unwrap_or_default();
    let (mut offsets, mut action_data) = (HashMap::new(), Vec::new());
    for sequence in sequences {
        let sequence_offset = len_i64(action_offset + action_data.len());
        if sequence_offset > 0x3FFF {
            return Err(bad("mort ligature-action offset exceeds 14 bits"));
        }
        offsets.insert(sequence.clone(), sequence_offset);
        let mut rebased = false;
        for (i, (delta, store)) in sequence.iter().enumerate() {
            budget.charge(1)?;
            let value = len_i64(component_offset / 2).saturating_add(*delta);
            if !(-(1i64 << 29)..(1i64 << 29)).contains(&value) {
                return Err(bad("mort ligature component offset exceeds 30 bits"));
            }
            let last = i + 1 == sequence.len();
            let mut word = (value & 0x3FFF_FFFF) as u32;
            if *store {
                word |= 0x4000_0000;
            }
            if last {
                word |= 0x8000_0000;
            }
            action_data.extend_from_slice(&word.to_be_bytes());
            if (*store || last) && !rebased && t.rebase.is_none() {
                let (start, limit) = ((*delta).max(0), len_i64(components.len()).min(delta.saturating_add(glyph_count)));
                if start < limit {
                    budget.charge(usize::try_from(limit - start).unwrap_or(usize::MAX))?;
                    rebase.extend(start..limit);
                }
            }
            rebased |= *store || last;
        }
    }
    let mut o = Out::default();
    for (i, c) in components.iter().enumerate() {
        let mut v = c.saturating_mul(2);
        if rebase.contains(&len_i64(i)) {
            v = v.saturating_add(len_i64(ligature_offset));
        }
        o.u16(v, "mort ligature component value exceeds 16 bits")?;
    }
    for g in ligatures {
        o.u16(i64::from(gid16(ctx, g)?), "a ligature glyph")?;
    }
    let mut extra = Out::default();
    for v in [action_offset, component_offset, ligature_offset] {
        extra.u16(len_i64(v), "a mort ligature offset")?;
    }
    let mut tail = vec![0u8; action_offset - tail_offset];
    tail.extend_from_slice(&action_data);
    tail.extend_from_slice(&o.0);
    Ok((extra.0, tail, offsets))
}
