//! TTX writer for `mort` and `morx`, the Apple glyph metamorphosis tables (M12.7d).
//!
//! Each table is decoded into chains, features and subtables, and each subtable's state table
//! into the [`StateTable`] model that fontTools builds (`AATStateTable`): glyph classes by glyph
//! name, states of transitions by glyph class, per-glyph lookups, ligature components and
//! ligatures. `morx` state tables are the extended form (32-bit offsets, 16-bit entry indices);
//! `mort` state tables are the original form (16-bit offsets, 8-bit entry indices, a class array
//! instead of a lookup), turned into the same model, as fontTools turns them. The model is
//! written as `ttx` writes it, and [`crate::ttx::read_misc`] compiles it back. Ported from
//! fontTools' `ttLib.tables` (MIT): the `mort` and `morx` layouts of `otData`, the
//! `MortSubtableConverter`, `MorxSubtableConverter`, `STXHeader`, `StateHeader` and `AATLookup`
//! converters of `otConverters`, and the AAT actions of `otTables`.
//!
//! Limits: a table with more than [`MAX_MORPH_CHAINS`] chains, a state table with more than
//! [`MAX_STATES`] states or [`MAX_GLYPH_CLASSES`] glyph classes, and a table whose transitions,
//! actions and lookup entries together pass [`MAX_MORPH_ITEMS`] are refused (and written as hex
//! by [`crate::ttx::dump`]); `ttx` writes them in full.

use std::collections::{BTreeMap, BTreeSet};

use crate::limits::MAX_LAYOUT_ITEMS;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

use super::super::XmlWriter;
use super::super::core::glyph_name;

/// The most chains read from one table.
pub(crate) const MAX_MORPH_CHAINS: usize = 65_536;
/// The most glyph classes of one state table: class values are 16-bit, so more are unreachable.
pub(crate) const MAX_GLYPH_CLASSES: i64 = 65_536;
/// The most states of one state table: a transition's next state is 16-bit.
pub(crate) const MAX_STATES: i64 = 65_536;
/// The most transitions, ligature actions, inserted glyphs and lookup entries one table may hold.
pub(crate) const MAX_MORPH_ITEMS: usize = MAX_LAYOUT_ITEMS;

/// The kind of a state-table subtable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Rearrangement,
    Contextual,
    Ligature,
    Insertion,
}

impl Kind {
    /// The flags a transition of this kind names, with their bits (fontTools' `_FLAGS`).
    pub(crate) fn flags(self) -> &'static [(&'static str, u16)] {
        match self {
            Kind::Rearrangement => &[("MarkFirst", 0x8000), ("DontAdvance", 0x4000), ("MarkLast", 0x2000)],
            Kind::Contextual => &[("SetMark", 0x8000), ("DontAdvance", 0x4000)],
            Kind::Ligature => &[("SetComponent", 0x8000), ("DontAdvance", 0x4000)],
            Kind::Insertion => &[
                ("SetMark", 0x8000),
                ("DontAdvance", 0x4000),
                ("CurrentIsKashidaLike", 0x2000),
                ("MarkedIsKashidaLike", 0x1000),
                ("CurrentInsertBefore", 0x0800),
                ("MarkedInsertBefore", 0x0400),
            ],
        }
    }

    /// The element of a subtable of this kind.
    pub(crate) fn element(self) -> &'static str {
        match self {
            Kind::Rearrangement => "RearrangementMorph",
            Kind::Contextual => "ContextualMorph",
            Kind::Ligature => "LigatureMorph",
            Kind::Insertion => "InsertionMorph",
        }
    }

    /// The subtable type number.
    pub(crate) fn morph_type(self) -> u8 {
        match self {
            Kind::Rearrangement => 0,
            Kind::Contextual => 1,
            Kind::Ligature => 2,
            Kind::Insertion => 5,
        }
    }

    /// The size of an extended entry (fontTools' `staticSize`).
    pub(crate) fn entry_size(self) -> usize {
        match self {
            Kind::Rearrangement => 4,
            Kind::Ligature => 6,
            Kind::Contextual | Kind::Insertion => 8,
        }
    }
}

/// One transition (fontTools' `RearrangementMorphAction`, `ContextualMorphAction`,
/// `LigatureMorphAction` or `InsertionMorphAction`); the fields of the other kinds stay at their
/// defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Transition {
    pub(crate) new_state: i64,
    /// The named flags set, as bits.
    pub(crate) flags: u16,
    pub(crate) reserved: i64,
    pub(crate) verb: i64,
    pub(crate) mark_index: i64,
    pub(crate) current_index: i64,
    /// Ligature actions: (glyph index delta, store).
    pub(crate) actions: Vec<(i64, bool)>,
    pub(crate) current_insertion: Vec<String>,
    pub(crate) marked_insertion: Vec<String>,
}

impl Default for Transition {
    fn default() -> Self {
        Transition {
            new_state: 0,
            flags: 0,
            reserved: 0,
            verb: 0,
            mark_index: 0xFFFF,
            current_index: 0xFFFF,
            actions: Vec::new(),
            current_insertion: Vec::new(),
            marked_insertion: Vec::new(),
        }
    }
}

/// A state table (fontTools' `AATStateTable`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StateTable {
    pub(crate) class_count: i64,
    pub(crate) classes: BTreeMap<String, i64>,
    pub(crate) states: Vec<BTreeMap<i64, Transition>>,
    pub(crate) per_glyph: Vec<BTreeMap<String, String>>,
    pub(crate) lig_components: Option<Vec<i64>>,
    /// `mort` only: the components that hold the ligature table's offset.
    pub(crate) rebase: Option<BTreeSet<i64>>,
    pub(crate) ligatures: Option<Vec<String>>,
}

/// The work done on one table, against [`MAX_MORPH_ITEMS`].
#[derive(Default)]
pub(crate) struct Budget(usize);

impl Budget {
    pub(crate) fn charge(&mut self, n: usize) -> Result<()> {
        self.0 = self.0.saturating_add(n);
        if self.0 > MAX_MORPH_ITEMS {
            return Err(TablesError::TooLarge { what: "the items of a glyph metamorphosis table", limit: MAX_MORPH_ITEMS });
        }
        Ok(())
    }
}

/// The verb comments of a rearrangement transition.
const VERBS: [&str; 16] = [
    "no change",
    "Ax ⇒ xA",
    "xD ⇒ Dx",
    "AxD ⇒ DxA",
    "ABx ⇒ xAB",
    "ABx ⇒ xBA",
    "xCD ⇒ CDx",
    "xCD ⇒ DCx",
    "AxCD ⇒ CDxA",
    "AxCD ⇒ DCxA",
    "ABxD ⇒ DxAB",
    "ABxD ⇒ DxBA",
    "ABxCD ⇒ CDxAB",
    "ABxCD ⇒ CDxBA",
    "ABxCD ⇒ DCxAB",
    "ABxCD ⇒ DCxBA",
];

// ── Reading bytes ──────────────────────────────────────────────────────────────────────────────

fn invalid(what: impl std::fmt::Display) -> TablesError {
    TablesError::invalid(format!("glyph metamorphosis table: {what}"))
}

fn bytes_at(d: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    at.checked_add(n).and_then(|end| d.get(at..end)).ok_or_else(|| invalid("the data runs past the end of the table"))
}

fn u8_at(d: &[u8], at: usize) -> Result<u8> {
    d.get(at).copied().ok_or_else(|| invalid("the data runs past the end of the table"))
}

fn u16_at(d: &[u8], at: usize) -> Result<u16> {
    let b = bytes_at(d, at, 2)?;
    Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
}

fn i16_at(d: &[u8], at: usize) -> Result<i16> {
    Ok(u16_at(d, at)? as i16)
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    Ok(bytes_at(d, at, 4)?.iter().fold(0u32, |v, x| (v << 8) | u32::from(*x)))
}

/// `at + n`, saturating: a position past the end fails when read.
fn add(at: usize, n: u64) -> usize {
    at.saturating_add(usize::try_from(n).unwrap_or(usize::MAX))
}

/// Python's `readArray` of 16-bit numbers: cut short where the data ends, an error only when the
/// end splits a number; nothing for a count below one.
fn u16s(d: &[u8], at: usize, count: i64) -> Result<Vec<u16>> {
    if count <= 0 {
        return Ok(Vec::new());
    }
    let end = add(at, u64::try_from(count).unwrap_or(u64::MAX).saturating_mul(2)).min(d.len());
    let bytes = d.get(at.min(d.len())..end).unwrap_or_default();
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid("an array ends part-way through a number"));
    }
    Ok(bytes.chunks(2).map(|c| u16::from_be_bytes([c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0)])).collect())
}

/// `AATLookup.read` of 16-bit values at `at`: (glyph id, value) in the order read.
fn aat_lookup(d: &[u8], at: usize, glyphs: usize, budget: &mut Budget) -> Result<Vec<(u32, u16)>> {
    let format = u16_at(d, at)?;
    let mut out = Vec::new();
    match format {
        0 => {
            budget.charge(glyphs)?;
            for (gid, v) in u16s(d, add(at, 2), i64::try_from(glyphs).unwrap_or(i64::MAX))?.into_iter().enumerate() {
                out.push((u32::try_from(gid).unwrap_or(u32::MAX), v));
            }
        }
        2 | 4 | 6 => {
            let unit = usize::from(u16_at(d, add(at, 2))?);
            let units = u16_at(d, add(at, 4))?;
            let least = if format == 6 { 4 } else { 6 };
            if unit < least {
                return Err(invalid(format!("an AAT lookup's unit size {unit} is too small")));
            }
            for i in 0..usize::from(units) {
                budget.charge(1)?;
                let seg = add(at, (i as u64).saturating_mul(unit as u64).saturating_add(12));
                if format == 6 {
                    let gid = u16_at(d, seg)?;
                    let value = u16_at(d, add(seg, 2))?;
                    if gid != 0xFFFF {
                        out.push((u32::from(gid), value));
                    }
                    continue;
                }
                let last = u16_at(d, seg)?;
                let first = u16_at(d, add(seg, 2))?;
                let value = u16_at(d, add(seg, 4))?;
                if last == 0xFFFF {
                    continue;
                }
                if format == 2 {
                    budget.charge(usize::from(last.saturating_sub(first)).saturating_add(1))?;
                    for gid in u32::from(first)..=u32::from(last) {
                        out.push((gid, value));
                    }
                } else {
                    let count = i64::from(last) - i64::from(first) + 1;
                    let values = u16s(d, add(at, u64::from(value)), count)?;
                    budget.charge(values.len())?;
                    for (k, v) in values.into_iter().enumerate() {
                        out.push((u32::from(first).saturating_add(u32::try_from(k).unwrap_or(u32::MAX)), v));
                    }
                }
            }
        }
        8 => {
            let first = u32::from(u16_at(d, add(at, 2))?);
            let count = u16_at(d, add(at, 4))?;
            let values = u16s(d, add(at, 6), i64::from(count))?;
            budget.charge(values.len())?;
            for (k, v) in values.into_iter().enumerate() {
                out.push((first.saturating_add(u32::try_from(k).unwrap_or(u32::MAX)), v));
            }
        }
        _ => return Err(invalid(format!("unsupported lookup format: {format}"))),
    }
    Ok(out)
}

fn name_of(order: &[String], gid: impl Into<u32>) -> String {
    glyph_name(order, gid.into())
}

/// A lookup of glyph ids as names to names, a later entry replacing an earlier one.
fn glyph_lookup(d: &[u8], at: usize, order: &[String], budget: &mut Budget) -> Result<BTreeMap<String, String>> {
    Ok(aat_lookup(d, at, order.len(), budget)?.into_iter().map(|(g, v)| (name_of(order, g), name_of(order, v))).collect())
}

// ── Extended state tables (`morx`) ─────────────────────────────────────────────────────────────

/// `STXHeader.read`: the state table at the start of a subtable body `d`.
fn read_extended(d: &[u8], kind: Kind, order: &[String], budget: &mut Budget) -> Result<StateTable> {
    let class_count = u32_at(d, 0)?;
    let class_off = u32_at(d, 4)?;
    let state_off = u32_at(d, 8)?;
    let entry_off = u32_at(d, 12)?;
    let mut table = StateTable { class_count: i64::from(class_count), ..StateTable::default() };
    let (mut per_glyph_off, mut action_off) = (0u32, 0u32);
    match kind {
        Kind::Contextual => per_glyph_off = u32_at(d, 16)?,
        Kind::Ligature => {
            action_off = u32_at(d, 16)?;
            let comp_off = u32_at(d, 20)?;
            let lig_off = u32_at(d, 24)?;
            if lig_off < comp_off {
                return Err(invalid("the ligature components end before they start"));
            }
            let components = u16s(d, comp_off as usize, i64::from((lig_off - comp_off) / 2))?;
            table.lig_components = Some(components.into_iter().map(i64::from).collect());
            let count = (d.len() as i64 - i64::from(lig_off)).div_euclid(2);
            let ligatures = u16s(d, lig_off as usize, count)?;
            budget.charge(ligatures.len())?;
            table.ligatures = Some(ligatures.into_iter().map(|g| name_of(order, g)).collect());
        }
        Kind::Insertion => action_off = u32_at(d, 16)?,
        Kind::Rearrangement => {}
    }
    for (g, v) in aat_lookup(d, class_off as usize, order.len(), budget)? {
        table.classes.insert(name_of(order, g), i64::from(v));
    }
    if class_count == 0 {
        return Err(invalid("a state table has no glyph classes"));
    }
    let states = (i64::from(entry_off) - i64::from(state_off)) / (i64::from(class_count) * 2);
    if states > 0 && i64::from(class_count) > MAX_GLYPH_CLASSES {
        return Err(TablesError::TooLarge { what: "the glyph classes of a state table", limit: MAX_GLYPH_CLASSES as usize });
    }
    if states > MAX_STATES {
        return Err(TablesError::TooLarge { what: "the states of a state table", limit: MAX_STATES as usize });
    }
    let mut at = state_off as usize;
    for _ in 0..states.max(0) {
        let mut row = BTreeMap::new();
        for class in 0..i64::from(class_count) {
            budget.charge(1)?;
            let entry = u16_at(d, at)?;
            at = add(at, 2);
            let pos = add(entry_off as usize, u64::from(entry).saturating_mul(kind.entry_size() as u64));
            row.insert(class, read_extended_transition(d, kind, pos, action_off as usize, order, budget)?);
        }
        table.states.push(row);
    }
    if kind == Kind::Contextual {
        for i in 0..per_glyph_count(&table) {
            let offset = u32_at(d, add(per_glyph_off as usize, u64::from(i) * 4))?;
            let lookup = glyph_lookup(d, add(per_glyph_off as usize, u64::from(offset)), order, budget)?;
            table.per_glyph.push(lookup);
        }
    }
    Ok(table)
}

/// `_countPerGlyphLookups`: one more than the highest mark or current index of a contextual
/// transition.
pub(crate) fn per_glyph_count(table: &StateTable) -> u32 {
    let mut count = 0i64;
    for t in table.states.iter().flat_map(|s| s.values()) {
        for index in [t.mark_index, t.current_index] {
            if index != 0xFFFF {
                count = count.max(index.saturating_add(1));
            }
        }
    }
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn set_flags(t: &mut Transition, kind: Kind, raw: u16) {
    t.flags = kind.flags().iter().fold(0, |f, (_, bit)| f | (raw & bit));
}

fn read_extended_transition(d: &[u8], kind: Kind, at: usize, action_off: usize, order: &[String], budget: &mut Budget) -> Result<Transition> {
    let mut t = Transition { new_state: i64::from(u16_at(d, at)?), ..Transition::default() };
    let raw = u16_at(d, add(at, 2))?;
    set_flags(&mut t, kind, raw);
    match kind {
        Kind::Rearrangement => {
            t.verb = i64::from(raw & 0xF);
            t.reserved = i64::from(raw & 0x1FF0);
        }
        Kind::Contextual => {
            t.reserved = i64::from(raw & 0x3FFF);
            t.mark_index = i64::from(u16_at(d, add(at, 4))?);
            t.current_index = i64::from(u16_at(d, add(at, 6))?);
        }
        Kind::Ligature => {
            t.reserved = i64::from(raw & 0x1FFF);
            let index = u16_at(d, add(at, 4))?;
            if raw & 0x2000 != 0 {
                t.actions = read_ligature_actions(d, add(action_off, u64::from(index) * 4), budget)?;
            }
        }
        Kind::Insertion => {
            let current = u16_at(d, add(at, 4))?;
            t.current_insertion = insertion_glyphs(d, action_off, current, (raw & 0x03E0) >> 5, order, budget)?;
            let marked = u16_at(d, add(at, 6))?;
            t.marked_insertion = insertion_glyphs(d, action_off, marked, raw & 0x001F, order, budget)?;
        }
    }
    Ok(t)
}

fn read_ligature_actions(d: &[u8], mut at: usize, budget: &mut Budget) -> Result<Vec<(i64, bool)>> {
    let mut actions = Vec::new();
    loop {
        budget.charge(1)?;
        let value = u32_at(d, at)?;
        at = add(at, 4);
        let mut delta = i64::from(value & 0x3FFF_FFFF);
        if delta >= 0x2000_0000 {
            delta -= 0x4000_0000;
        }
        actions.push((delta, value & 0x4000_0000 != 0));
        if value & 0x8000_0000 != 0 {
            return Ok(actions);
        }
    }
}

fn insertion_glyphs(d: &[u8], action_off: usize, index: u16, count: u16, order: &[String], budget: &mut Budget) -> Result<Vec<String>> {
    if index == 0xFFFF || count == 0 {
        return Ok(Vec::new());
    }
    let glyphs = u16s(d, add(action_off, u64::from(index) * 2), i64::from(count))?;
    budget.charge(glyphs.len())?;
    Ok(glyphs.into_iter().map(|g| name_of(order, g)).collect())
}

// ── Original state tables (`mort`) ─────────────────────────────────────────────────────────────

/// What a `mort` entry holds besides the transition: the raw offsets a contextual entry gives.
struct LegacyEntry {
    transition: Transition,
    mark_offset: i16,
    current_offset: i16,
}

/// `StateHeader.read`: the original state table at the start of a subtable body `d`, turned into
/// the extended model.
fn read_legacy(d: &[u8], kind: Kind, order: &[String], budget: &mut Budget) -> Result<StateTable> {
    let class_count = u16_at(d, 0)?;
    let class_off = usize::from(u16_at(d, 2)?);
    let state_off = u16_at(d, 4)?;
    let entry_off = u16_at(d, 6)?;
    if class_count == 0 {
        return Err(invalid("a mort state table has no glyph classes"));
    }
    let mut table = StateTable { class_count: i64::from(class_count), ..StateTable::default() };
    let (mut substitution_off, mut action_off, mut comp_off, mut lig_off) = (None, 0usize, 0u16, 0u16);
    let mut rebase = BTreeSet::new();
    match kind {
        Kind::Contextual => substitution_off = Some(u16_at(d, 8)?),
        Kind::Ligature => {
            action_off = usize::from(u16_at(d, 8)?);
            comp_off = u16_at(d, 10)?;
            lig_off = u16_at(d, 12)?;
            if comp_off > lig_off || !(lig_off - comp_off).is_multiple_of(2) {
                return Err(invalid("invalid mort ligature component table"));
            }
            let raw = u16s(d, usize::from(comp_off), i64::from((lig_off - comp_off) / 2))?;
            if raw.iter().any(|c| !c.is_multiple_of(2)) {
                return Err(invalid("invalid mort ligature component value"));
            }
            table.lig_components = Some(raw.into_iter().map(|c| i64::from(c / 2)).collect());
        }
        Kind::Insertion => action_off = usize::from(u16_at(d, 8)?),
        Kind::Rearrangement => {}
    }
    let first = u16_at(d, class_off)?;
    let count = u16_at(d, add(class_off, 2))?;
    let start = add(class_off, 4).min(d.len());
    let classes = d.get(start..add(start, u64::from(count)).min(d.len())).unwrap_or_default();
    budget.charge(classes.len())?;
    for (i, class) in classes.iter().enumerate() {
        table.classes.insert(name_of(order, u32::from(first).saturating_add(i as u32)), i64::from(*class));
    }
    if entry_off < state_off {
        return Err(invalid("mort state-array offset follows its entry table"));
    }
    let states = usize::from((entry_off - state_off) / class_count);
    let components = table.lig_components.clone().unwrap_or_default();
    let glyph_count = order.len() as i64;
    let mut entries: BTreeMap<u8, LegacyEntry> = BTreeMap::new();
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(states);
    let mut at = usize::from(state_off);
    for _ in 0..states {
        let mut row = Vec::with_capacity(usize::from(class_count));
        for _ in 0..class_count {
            budget.charge(1)?;
            let index = u8_at(d, at)?;
            at = add(at, 1);
            if !entries.contains_key(&index) {
                let mut e = LegacyEntry { transition: Transition::default(), mark_offset: 0, current_offset: 0 };
                let size = if kind == Kind::Ligature { 4 } else { kind.entry_size() };
                let pos = add(usize::from(entry_off), u64::from(index) * size as u64);
                let t = &mut e.transition;
                match kind {
                    Kind::Contextual => {
                        t.new_state = i64::from(u16_at(d, pos)?);
                        let raw = u16_at(d, add(pos, 2))?;
                        set_flags(t, kind, raw);
                        t.reserved = i64::from(raw & 0x3FFF);
                        e.mark_offset = i16_at(d, add(pos, 4))?;
                        e.current_offset = i16_at(d, add(pos, 6))?;
                    }
                    Kind::Ligature => {
                        t.new_state = i64::from(u16_at(d, pos)?);
                        let raw = u16_at(d, add(pos, 2))?;
                        set_flags(t, kind, raw);
                        let actions_at = raw & 0x3FFF;
                        if actions_at != 0 {
                            let context = LegacyLigature { comp_off, components: components.len() as i64, glyph_count };
                            t.actions = legacy_ligature_actions(d, usize::from(actions_at), &context, &mut rebase, budget)?;
                        }
                    }
                    Kind::Rearrangement | Kind::Insertion => *t = read_extended_transition(d, kind, pos, action_off, order, budget)?,
                }
                let offset = t.new_state - i64::from(state_off);
                if offset < 0 || offset % i64::from(class_count) != 0 {
                    return Err(invalid("invalid mort new-state offset"));
                }
                t.new_state = offset / i64::from(class_count);
                entries.insert(index, e);
            }
            row.push(index);
        }
        rows.push(row);
    }
    if let Some(substitution_off) = substitution_off {
        let offsets: BTreeSet<i16> = entries.values().flat_map(|e| [e.mark_offset, e.current_offset]).filter(|o| *o != 0).collect();
        let available = d.len() as i64;
        for offset in &offsets {
            let offset = i64::from(*offset);
            let first = 0i64.max((i64::from(substitution_off) + 1) / 2 - offset);
            let last = (glyph_count - 1).min((available - 2).div_euclid(2) - offset);
            let mut lookup = BTreeMap::new();
            if first <= last {
                let at = usize::try_from(2 * (offset + first)).unwrap_or(usize::MAX);
                let replacements = u16s(d, at, last - first + 1)?;
                budget.charge(replacements.len())?;
                for (i, r) in replacements.into_iter().enumerate() {
                    if r != 0 {
                        let glyph = usize::try_from(first).ok().and_then(|f| order.get(f.saturating_add(i))).cloned().unwrap_or_default();
                        lookup.insert(glyph, name_of(order, r));
                    }
                }
            }
            table.per_glyph.push(lookup);
        }
        let index_of = |o: i16| if o == 0 { 0xFFFF } else { offsets.iter().position(|x| *x == o).map_or(0xFFFF, |p| p as i64) };
        for e in entries.values_mut() {
            e.transition.mark_index = index_of(e.mark_offset);
            e.transition.current_index = index_of(e.current_offset);
        }
    } else if kind == Kind::Ligature {
        let base = i64::from(lig_off / 2);
        let mut components = components;
        rebase.retain(|i| usize::try_from(*i).ok().and_then(|i| components.get(i)).is_some_and(|c| *c >= base));
        for i in &rebase {
            if let Some(c) = usize::try_from(*i).ok().and_then(|i| components.get_mut(i)) {
                *c -= base;
            }
        }
        let count = count_legacy_ligatures(entries.values().map(|e| &e.transition), &components, glyph_count, budget)?;
        let ligatures = u16s(d, usize::from(lig_off), count)?;
        budget.charge(ligatures.len())?;
        table.ligatures = Some(ligatures.into_iter().map(|g| name_of(order, g)).collect());
        table.lig_components = Some(components);
        table.rebase = Some(rebase);
    }
    for row in rows {
        let mut state = BTreeMap::new();
        for (class, index) in row.into_iter().enumerate() {
            if let Some(e) = entries.get(&index) {
                state.insert(class as i64, e.transition.clone());
            }
        }
        table.states.push(state);
    }
    Ok(table)
}

/// What reading the actions of a `mort` ligature entry needs.
struct LegacyLigature {
    comp_off: u16,
    components: i64,
    glyph_count: i64,
}

/// `_readLigatureActions`: the actions at `at`, as component indices; the components the first
/// storing (or last) action can reach are marked for rebasing.
fn legacy_ligature_actions(d: &[u8], mut at: usize, c: &LegacyLigature, rebase: &mut BTreeSet<i64>, budget: &mut Budget) -> Result<Vec<(i64, bool)>> {
    let mut actions = Vec::new();
    let mut rebased = false;
    loop {
        budget.charge(1)?;
        let value = u32_at(d, at)?;
        at = add(at, 4);
        let last = value & 0x8000_0000 != 0;
        let store = value & 0x4000_0000 != 0;
        let mut word = i64::from(value & 0x3FFF_FFFF);
        if word & 0x2000_0000 != 0 {
            word -= 0x4000_0000;
        }
        let index = word - i64::from(c.comp_off / 2);
        if index >= c.components || index + c.glyph_count <= 0 {
            return Err(invalid("mort ligature component offset is out of bounds"));
        }
        if (store || last) && !rebased {
            let (start, limit) = (index.max(0), c.components.min(index + c.glyph_count));
            budget.charge(usize::try_from(limit - start).unwrap_or(0))?;
            rebase.extend(start..limit);
            rebased = true;
        }
        actions.push((index, store));
        if last {
            return Ok(actions);
        }
    }
}

/// `_countLegacyLigatures`: one more than the highest ligature index an action can reach.
pub(crate) fn count_legacy_ligatures<'a>(
    transitions: impl Iterator<Item = &'a Transition>,
    components: &[i64],
    glyph_count: i64,
    budget: &mut Budget,
) -> Result<i64> {
    let len = components.len() as i64;
    let mut count = 0i64;
    for t in transitions {
        let mut cumulative = 0i64;
        for (i, (delta, store)) in t.actions.iter().enumerate() {
            let start = (*delta).max(0);
            let limit = len.min(delta.saturating_add(glyph_count));
            let window = if start < limit { components.get(start as usize..limit as usize).unwrap_or_default() } else { &[] };
            budget.charge(window.len())?;
            cumulative = cumulative.saturating_add(window.iter().copied().max().unwrap_or(0));
            if *store || i + 1 == t.actions.len() {
                count = count.max(cumulative.saturating_add(1));
            }
        }
    }
    Ok(count)
}

// ── Chains and subtables ───────────────────────────────────────────────────────────────────────

/// Writes `mort` (`legacy`) or `morx` (bytes `data`, from `font`) into `w`.
pub(crate) fn write(data: &[u8], font: &SfntFont, w: &mut XmlWriter, legacy: bool) -> Result<()> {
    let order = w.glyph_order(font)?;
    let order = order.as_slice();
    let mut budget = Budget::default();
    let chain_count = if legacy {
        let version = u32_at(data, 0)? as i32;
        let text = if version < 0 { format!("0x-{:07x}", version.unsigned_abs()) } else { format!("0x{version:08x}") };
        w.simple("Version", &[("value", &text)]);
        u32_at(data, 4)?
    } else {
        w.simple("Version", &[("value", &u16_at(data, 0)?.to_string())]);
        w.simple("Reserved", &[("value", &u16_at(data, 2)?.to_string())]);
        u32_at(data, 4)?
    };
    w.comment(&format!("MorphChainCount={chain_count}"));
    let mut at = 8usize;
    for i in 0..chain_count {
        if i as usize >= MAX_MORPH_CHAINS {
            return Err(TablesError::TooLarge { what: "the chains of a glyph metamorphosis table", limit: MAX_MORPH_CHAINS });
        }
        let length = u32_at(data, add(at, 4))?;
        chain(data, at, legacy, order, w, i, &mut budget)?;
        at = add(at, u64::from(length));
        w.check()?;
    }
    Ok(())
}

fn flags32(v: u32) -> String {
    format!("0x{v:08X}")
}

fn chain(d: &[u8], start: usize, legacy: bool, order: &[String], w: &mut XmlWriter, index: u32, budget: &mut Budget) -> Result<()> {
    let default_flags = u32_at(d, start)?;
    let length = u32_at(d, add(start, 4))?;
    let (features, subtables, mut at) = if legacy {
        (u32::from(u16_at(d, add(start, 8))?), u32::from(u16_at(d, add(start, 10))?), add(start, 12))
    } else {
        (u32_at(d, add(start, 8))?, u32_at(d, add(start, 12))?, add(start, 16))
    };
    w.open("MorphChain", &[("index", &index.to_string())]);
    w.simple("DefaultFlags", &[("value", &flags32(default_flags))]);
    w.comment(&format!("StructLength={length}"));
    w.comment(&format!("MorphFeatureCount={features}"));
    w.comment(&format!("MorphSubtableCount={subtables}"));
    for i in 0..features {
        budget.charge(1)?;
        let feature = bytes_at(d, at, 12)?;
        w.open("MorphFeature", &[("index", &i.to_string())]);
        w.simple("FeatureType", &[("value", &u16_at(feature, 0)?.to_string())]);
        w.simple("FeatureSetting", &[("value", &u16_at(feature, 2)?.to_string())]);
        w.simple("EnableFlags", &[("value", &flags32(u32_at(feature, 4)?))]);
        w.simple("DisableFlags", &[("value", &flags32(u32_at(feature, 8)?))]);
        w.close("MorphFeature");
        at = add(at, 12);
        w.check()?;
    }
    for i in 0..subtables {
        budget.charge(1)?;
        at = subtable(d, at, legacy, order, w, i, budget)?;
        w.check()?;
    }
    w.close("MorphChain");
    Ok(())
}

/// Writes the subtable at `at`; returns where the next one starts.
fn subtable(d: &[u8], at: usize, legacy: bool, order: &[String], w: &mut XmlWriter, index: u32, budget: &mut Budget) -> Result<usize> {
    let (length, header, morph_type) =
        if legacy { (u64::from(u16_at(d, at)?), 8u64, u8_at(d, add(at, 3))?) } else { (u64::from(u32_at(d, at)?), 12u64, u8_at(d, add(at, 7))?) };
    let body_len = length.checked_sub(header).ok_or_else(|| invalid("a subtable is shorter than its header"))?;
    let body = bytes_at(d, add(at, header), usize::try_from(body_len).unwrap_or(usize::MAX))?;
    w.open("MorphSubtable", &[("index", &index.to_string())]);
    w.comment(&format!("StructLength={length}"));
    if legacy {
        w.simple("CoverageFlags", &[("value", &u8_at(d, add(at, 2))?.to_string())]);
    } else {
        let flags = u8_at(d, add(at, 4))?;
        let reserved = u32::from(u16_at(d, add(at, 5))?) | (u32::from(flags & 0xF) << 16);
        let direction = if flags & 0x20 != 0 {
            "Any"
        } else if flags & 0x80 != 0 {
            "Vertical"
        } else {
            "Horizontal"
        };
        let order_name = match (flags & 0x40 != 0, flags & 0x10 != 0) {
            (false, false) => "LayoutOrder",
            (true, false) => "ReversedLayoutOrder",
            (false, true) => "LogicalOrder",
            (true, true) => "ReversedLogicalOrder",
        };
        w.simple("TextDirection", &[("value", direction)]);
        w.simple("ProcessingOrder", &[("value", order_name)]);
        if reserved != 0 {
            w.simple("Reserved", &[("value", &format!("0x{reserved:04x}"))]);
        }
    }
    w.comment(&format!("MorphType={morph_type}"));
    let sub_feature_flags = u32_at(d, add(at, if legacy { 4 } else { 8 }))?;
    w.simple("SubFeatureFlags", &[("value", &format!("0x{sub_feature_flags:08x}"))]);
    let kind = match morph_type {
        0 => Kind::Rearrangement,
        1 => Kind::Contextual,
        2 => Kind::Ligature,
        5 => Kind::Insertion,
        4 => {
            let lookup = glyph_lookup(body, 0, order, budget)?;
            w.open("NoncontextualMorph", &[]);
            write_lookup(w, "Substitution", &lookup)?;
            w.close("NoncontextualMorph");
            w.close("MorphSubtable");
            return Ok(add(at, length));
        }
        _ => return Err(invalid(format!("unsupported '{}' lookup type {morph_type}", if legacy { "mort" } else { "morx" }))),
    };
    let table = if legacy { read_legacy(body, kind, order, budget)? } else { read_extended(body, kind, order, budget)? };
    w.open(kind.element(), &[]);
    write_state_table(w, &table, kind)?;
    w.close(kind.element());
    w.close("MorphSubtable");
    Ok(add(at, length))
}

fn write_lookup(w: &mut XmlWriter, element: &str, lookup: &BTreeMap<String, String>) -> Result<()> {
    w.open(element, &[]);
    for (glyph, value) in lookup {
        w.simple("Lookup", &[("glyph", glyph), ("value", value)]);
        w.check()?;
    }
    w.close(element);
    Ok(())
}

/// `STXHeader.xmlWrite`.
pub(crate) fn write_state_table(w: &mut XmlWriter, table: &StateTable, kind: Kind) -> Result<()> {
    w.open("StateTable", &[]);
    w.comment(&format!("GlyphClassCount={}", table.class_count));
    for (glyph, class) in &table.classes {
        w.simple("GlyphClass", &[("glyph", glyph), ("value", &class.to_string())]);
    }
    for (i, state) in table.states.iter().enumerate() {
        w.open("State", &[("index", &i.to_string())]);
        for (class, t) in state {
            write_transition(w, kind, *class, t);
        }
        w.close("State");
        w.check()?;
    }
    for (i, lookup) in table.per_glyph.iter().enumerate() {
        w.open("PerGlyphLookup", &[("index", &i.to_string())]);
        for (glyph, value) in lookup {
            w.simple("Lookup", &[("glyph", glyph), ("value", value)]);
        }
        w.close("PerGlyphLookup");
        w.check()?;
    }
    if let Some(components) = &table.lig_components {
        w.open("LigComponents", &[]);
        for (i, v) in components.iter().enumerate() {
            w.simple("LigComponent", &[("index", &i.to_string()), ("value", &v.to_string())]);
        }
        w.close("LigComponents");
    }
    if let Some(rebase) = &table.rebase {
        w.open("MortLigatureRebase", &[]);
        for i in rebase {
            w.simple("Component", &[("index", &i.to_string())]);
        }
        w.close("MortLigatureRebase");
    }
    if let Some(ligatures) = &table.ligatures {
        w.open("Ligatures", &[]);
        for (i, glyph) in ligatures.iter().enumerate() {
            w.simple("Ligature", &[("glyph", glyph), ("index", &i.to_string())]);
        }
        w.close("Ligatures");
    }
    w.close("StateTable");
    w.check()
}

fn write_transition(w: &mut XmlWriter, kind: Kind, class: i64, t: &Transition) {
    w.open("Transition", &[("onGlyphClass", &class.to_string())]);
    w.simple("NewState", &[("value", &t.new_state.to_string())]);
    let names: Vec<&str> = kind.flags().iter().filter(|(_, bit)| t.flags & bit != 0).map(|(name, _)| *name).collect();
    if !names.is_empty() {
        w.simple("Flags", &[("value", &names.join(","))]);
    }
    if t.reserved != 0 {
        w.simple("ReservedFlags", &[("value", &format!("0x{:04X}", t.reserved))]);
    }
    match kind {
        Kind::Rearrangement => {
            let verb = t.verb.to_string();
            match usize::try_from(t.verb).ok().and_then(|v| VERBS.get(v)) {
                Some(comment) => w.simple_comment("Verb", &[("value", &verb)], comment),
                None => w.simple("Verb", &[("value", &verb)]),
            }
        }
        Kind::Contextual => {
            w.simple("MarkIndex", &[("value", &t.mark_index.to_string())]);
            w.simple("CurrentIndex", &[("value", &t.current_index.to_string())]);
        }
        Kind::Ligature => {
            for (delta, store) in &t.actions {
                let delta = delta.to_string();
                if *store {
                    w.simple("Action", &[("GlyphIndexDelta", &delta), ("Flags", "Store")]);
                } else {
                    w.simple("Action", &[("GlyphIndexDelta", &delta)]);
                }
            }
        }
        Kind::Insertion => {
            for g in &t.current_insertion {
                w.simple("CurrentInsertionAction", &[("glyph", g)]);
            }
            for g in &t.marked_insertion {
                w.simple("MarkedInsertionAction", &[("glyph", g)]);
            }
        }
    }
    w.close("Transition");
}
