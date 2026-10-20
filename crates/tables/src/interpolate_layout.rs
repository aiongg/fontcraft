//! The `GPOS` of an instance, interpolated from the compiled masters of a designspace: a port of
//! fontTools' `varLib.interpolate_layout` and of the `InstancerMerger` of `varLib.merger` (MIT
//! License, Copyright (c) 2017 Google Inc. and the fontTools authors).
//!
//! The masters' `GPOS` tables are walked in parallel. Every value record, anchor and the parts
//! that hold them are aligned across the masters the way the variation merger aligns them for a
//! variable font, and each number becomes `round(Σ value × weight)`, the weights being each
//! master's scalar at the instance's location. Everything else (scripts, features, lookup types
//! and flags, contextual lookups) must be the same in every master, or the run fails.
//!
//! As in fontTools:
//! - only `GPOS` is interpolated; the instance keeps the base master's other tables, `GDEF` and
//!   `GSUB` included;
//! - extension lookups are unwrapped; pair adjustment subtables are first joined (the leading
//!   format 1 subtables into one, the trailing format 2 subtables into one), and single
//!   adjustment lookups whose masters have different subtable counts are joined into one and
//!   split again by value;
//! - a glyph one master's subtable lacks takes the value the master's lookup gives it through
//!   its other subtables;
//! - device tables in value records are refused; anchors become format 1.
//!
//! Glyphs are matched by name, so masters may number their glyphs differently, except in
//! contextual lookups, which must then be identical glyph by glyph.

use std::collections::{BTreeMap, HashMap};

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{
    AnchorTable, BaseArray, BaseRecord, ComponentRecord, CursivePosFormat1, EntryExitRecord, ExtensionSubtable, Gpos, LigatureArray, LigatureAttach,
    Mark2Array, Mark2Record, MarkArray, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1, MarkRecord, PositionLookup, PositionLookupList,
    SinglePos, ValueFormat, ValueRecord,
};
use write_fonts::tables::layout::{ClassDef, ClassRangeRecord, CoverageTable, Lookup, LookupFlag};
use write_fonts::types::GlyphId16;

use crate::limits::MAX_INTERPOLATED_LAYOUT_ITEMS;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

mod pair;
#[cfg(test)]
mod tests;

/// The most masters one interpolation reads.
pub const MAX_LAYOUT_MASTERS: usize = 256;

const GPOS: crate::sfnt::Tag = *b"GPOS";

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what)
}

/// The base master `masters[base]` with its `GPOS` replaced by the one interpolated at the
/// location where master `i` weighs `scalars[i]` (fontTools' `interpolate_layout`).
pub fn interpolate_layout(masters: &[SfntFont], base: usize, scalars: &[f64]) -> Result<SfntFont> {
    let mut font = masters.get(base).cloned().ok_or_else(|| bad("the base master is not one of the masters"))?;
    if let Some(gpos) = interpolate_gpos(masters, base, scalars)? {
        font.insert(GPOS, gpos);
    }
    Ok(font)
}

/// The `GPOS` table interpolated from `masters` (see the module notes), or `None` when the base
/// master has none. `scalars` holds each master's weight at the instance's location.
pub fn interpolate_gpos(masters: &[SfntFont], base: usize, scalars: &[f64]) -> Result<Option<Vec<u8>>> {
    if masters.len() > MAX_LAYOUT_MASTERS {
        return Err(TablesError::TooLarge { what: "the master count", limit: MAX_LAYOUT_MASTERS });
    }
    if scalars.len() != masters.len() || scalars.iter().any(|s| !s.is_finite()) {
        return Err(bad("each master needs one finite weight"));
    }
    let base_font = masters.get(base).ok_or_else(|| bad("the base master is not one of the masters"))?;
    if base_font.get(GPOS).is_none() {
        return Ok(None);
    }
    let names = crate::ttx::glyph_order(base_font)?;
    let index: HashMap<&str, u16> = names.iter().enumerate().filter_map(|(i, n)| Some((n.as_str(), u16::try_from(i).ok()?))).collect();
    // Every master's table is held in owned form at once, so their sizes are charged together
    // before any is converted.
    crate::layout_budget::check_gpos_together(masters.iter().filter_map(|m| m.get(GPOS)), MAX_INTERPOLATED_LAYOUT_ITEMS)?;
    let mut tables = Vec::with_capacity(masters.len());
    let mut remap = Vec::with_capacity(masters.len());
    for (i, m) in masters.iter().enumerate() {
        let data = m.get(GPOS).ok_or_else(|| bad(format!("master {i} has no GPOS table and the base master has one")))?;
        crate::layout_budget::check_gpos(data)?;
        let read =
            read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| bad(format!("master {i}: the GPOS table cannot be read: {e}")))?;
        let gpos: Gpos = read.to_owned_table();
        tables.push(gpos);
        let own = crate::ttx::glyph_order(m)?;
        remap.push(if own == names { None } else { Some(own.iter().map(|n| index.get(n.as_str()).copied()).collect::<Vec<_>>()) });
    }
    let ctx = Ctx { scalars: scalars.to_vec(), remap, names, base };
    let gpos = merge_gpos(&ctx, base, &tables)?;
    let bytes = write_fonts::dump_table(&gpos).map_err(|e| bad(format!("the interpolated GPOS table cannot be written: {e}")))?;
    Ok(Some(bytes))
}

/// What every merge needs: the weights, how each master's glyph ids map to the base master's,
/// and the base master's glyph names.
pub(crate) struct Ctx {
    scalars: Vec<f64>,
    /// For each master, its glyphs' ids in the base master (`None`: the same glyph order).
    remap: Vec<Option<Vec<Option<u16>>>>,
    names: Vec<String>,
    /// The index of the base master, whose tables the merged ones start from.
    base: usize,
}

impl Ctx {
    /// Master `m`'s glyph `g` as a base-master glyph id.
    fn gid(&self, m: usize, g: GlyphId16) -> Result<u16> {
        match self.remap.get(m) {
            Some(Some(map)) => map
                .get(usize::from(g.to_u16()))
                .copied()
                .flatten()
                .ok_or_else(|| bad(format!("master {m}: glyph {} is not in the base master", g.to_u16()))),
            _ => Ok(g.to_u16()),
        }
    }

    fn same_glyph_orders(&self) -> bool {
        self.remap.iter().all(Option::is_none)
    }

    fn name(&self, g: u16) -> &str {
        self.names.get(usize::from(g)).map_or("", String::as_str)
    }

    /// `round(Σ value × weight)`, summed in master order and skipping zero weights, as fontTools
    /// interpolates.
    fn value(&self, values: impl IntoIterator<Item = i32>) -> Result<i16> {
        let mut sum: Option<f64> = None;
        for (v, s) in values.into_iter().zip(&self.scalars) {
            if *s == 0.0 {
                continue;
            }
            let c = f64::from(v) * s;
            sum = Some(sum.map_or(c, |t| t + c));
        }
        let r = (sum.unwrap_or(0.0) + 0.5).floor();
        if r.is_finite() && r >= f64::from(i16::MIN) && r <= f64::from(i16::MAX) {
            Ok(r as i16)
        } else {
            Err(bad("an interpolated value does not fit in 16 bits"))
        }
    }

    fn anchor(&self, anchors: &[Option<Anchor>]) -> Result<Option<Anchor>> {
        if anchors.iter().all(Option::is_none) {
            return Ok(None);
        }
        let all: Vec<Anchor> = anchors.iter().map(|a| a.ok_or_else(|| bad("an anchor is missing in some masters"))).collect::<Result<_>>()?;
        Ok(Some(Anchor { x: self.value(all.iter().map(|a| i32::from(a.x)))?, y: self.value(all.iter().map(|a| i32::from(a.y)))?, format: 1 }))
    }
}

// ---------- value records and anchors ----------

/// The value-format bits of the four device offsets.
const DEVICE_BITS: u16 = 0x00F0;

/// A value record: which of XPlacement, YPlacement, XAdvance and YAdvance it holds (format bits
/// 1, 2, 4, 8), and their values (0 when not held).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Vr {
    mask: u16,
    v: [i16; 4],
}

const FIELD_BITS: [u16; 4] = [1, 2, 4, 8];

impl Vr {
    /// A master's record; `None` for a record of format 0, as fontTools reads one.
    fn read(r: &ValueRecord) -> Result<Option<Vr>> {
        let format = r.format().bits();
        if format & DEVICE_BITS != 0 {
            return Err(bad("value records with device tables cannot be interpolated"));
        }
        if format == 0 {
            return Ok(None);
        }
        let v = [r.x_placement.unwrap_or(0), r.y_placement.unwrap_or(0), r.x_advance.unwrap_or(0), r.y_advance.unwrap_or(0)];
        Ok(Some(Vr { mask: format, v }))
    }

    /// fontTools' `ValueRecord(format, src)`: every field of `format`, valued from `src` where it
    /// holds the field.
    fn with_format(format: u16, src: Option<&Vr>) -> Vr {
        let mut v = [0i16; 4];
        for ((slot, bit), from) in v.iter_mut().zip(FIELD_BITS).zip(src.map_or([0; 4], |s| s.v)) {
            if format & bit != 0 && src.is_some_and(|s| s.mask & bit != 0) {
                *slot = from;
            }
        }
        Vr { mask: format & 0x000F, v }
    }

    /// The format bits of the fields that are not zero.
    fn effective(&self) -> u16 {
        self.v.iter().zip(FIELD_BITS).filter(|(v, _)| **v != 0).fold(0, |f, (_, b)| f | b)
    }

    fn field(&self, i: usize) -> i16 {
        self.v.get(i).copied().unwrap_or(0)
    }

    /// The record written under the subtable's `format`.
    fn write(v: Option<&Vr>, format: u16) -> ValueRecord {
        let get = |i: usize| v.map_or(0, |v| v.field(i));
        let mut r = ValueRecord::new();
        if format & 1 != 0 {
            r.x_placement = Some(get(0));
        }
        if format & 2 != 0 {
            r.y_placement = Some(get(1));
        }
        if format & 4 != 0 {
            r.x_advance = Some(get(2));
        }
        if format & 8 != 0 {
            r.y_advance = Some(get(3));
        }
        r.with_explicit_value_format(ValueFormat::from_bits_truncate(format))
    }
}

impl Ctx {
    /// The instancer's value-record merge: each field `out` holds, interpolated from the
    /// masters' values of it (0 where a master's record does not hold it).
    fn value_record(&self, out: Option<&Vr>, masters: &[Option<Vr>]) -> Result<Option<Vr>> {
        let Some(out) = out else {
            if masters.iter().any(Option::is_some) {
                return Err(bad("value records of different formats cannot be interpolated"));
            }
            return Ok(None);
        };
        if masters.iter().any(Option::is_none) {
            return Err(bad("value records of different formats cannot be interpolated"));
        }
        let mut r = *out;
        for (i, bit) in FIELD_BITS.iter().enumerate() {
            if r.mask & bit != 0
                && let Some(slot) = r.v.get_mut(i)
            {
                *slot = self.value(masters.iter().map(|m| m.map_or(0, |m| i32::from(m.field(i)))))?;
            }
        }
        Ok(Some(r))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    x: i16,
    y: i16,
    format: u8,
}

impl Anchor {
    fn read(a: &AnchorTable) -> Anchor {
        match a {
            AnchorTable::Format1(t) => Anchor { x: t.x_coordinate, y: t.y_coordinate, format: 1 },
            AnchorTable::Format2(t) => Anchor { x: t.x_coordinate, y: t.y_coordinate, format: 2 },
            AnchorTable::Format3(t) => Anchor { x: t.x_coordinate, y: t.y_coordinate, format: 3 },
        }
    }

    fn write(self) -> AnchorTable {
        AnchorTable::format_1(self.x, self.y)
    }
}

fn read_anchor(a: Option<&AnchorTable>) -> Option<Anchor> {
    a.map(Anchor::read)
}

// ---------- glyph lists ----------

fn coverage(ctx: &Ctx, m: usize, c: &CoverageTable) -> Result<Vec<u16>> {
    c.iter().map(|g| ctx.gid(m, g)).collect()
}

fn write_coverage(glyphs: &[u16]) -> CoverageTable {
    CoverageTable::format_1(glyphs.iter().map(|g| GlyphId16::new(*g)).collect())
}

fn class_map(ctx: &Ctx, m: usize, c: &ClassDef) -> Result<HashMap<u16, u16>> {
    c.iter().map(|(g, k)| Ok((ctx.gid(m, g)?, k))).collect()
}

/// A class definition holding `items` (glyph → class, class 0 left out), in the smaller format.
fn write_class_def(items: &BTreeMap<u16, u16>) -> ClassDef {
    let items: BTreeMap<u16, u16> = items.iter().filter(|(_, k)| **k != 0).map(|(g, k)| (*g, *k)).collect();
    let mut ranges: Vec<(u16, u16, u16)> = Vec::new();
    for (g, k) in &items {
        match ranges.last_mut() {
            Some((_, end, class)) if *class == *k && end.checked_add(1) == Some(*g) => *end = *g,
            _ => ranges.push((*g, *g, *k)),
        }
    }
    let (Some(first), Some(last)) = (items.keys().next().copied(), items.keys().next_back().copied()) else {
        return ClassDef::format_2(Vec::new());
    };
    let format1 = 6 + 2 * (usize::from(last) - usize::from(first) + 1);
    let format2 = 4 + 6 * ranges.len();
    if format1 < format2 {
        let values = (first..=last).map(|g| items.get(&g).copied().unwrap_or(0)).collect();
        ClassDef::format_1(GlyphId16::new(first), values)
    } else {
        ClassDef::format_2(ranges.into_iter().map(|(s, e, k)| ClassRangeRecord::new(GlyphId16::new(s), GlyphId16::new(e), k)).collect())
    }
}

/// For each list, the index in it of each glyph of the union (`None` where the list lacks it).
type Positions = Vec<Vec<Option<usize>>>;

/// fontTools' `_merge_GlyphOrders`: the union of `lists` in glyph order, and for each list the
/// index of each union glyph in it. Every list must already be in glyph order.
fn merge_orders(lists: &[&[u16]]) -> Result<(Vec<u16>, Positions)> {
    if lists.iter().any(|l| l.windows(2).any(|w| matches!(w, [a, b] if a > b))) {
        return Err(bad("a master lists glyphs out of the base master's glyph order"));
    }
    let mut union: Vec<u16> = lists.iter().flat_map(|l| l.iter().copied()).collect();
    union.sort_unstable();
    union.dedup();
    let padded = lists
        .iter()
        .map(|l| {
            let at: HashMap<u16, usize> = l.iter().enumerate().map(|(i, g)| (*g, i)).collect();
            union.iter().map(|g| at.get(g).copied()).collect()
        })
        .collect();
    Ok((union, padded))
}

// ---------- the table ----------

fn merge_gpos(ctx: &Ctx, base: usize, tables: &[Gpos]) -> Result<Gpos> {
    let b = tables.get(base).ok_or_else(|| bad("the base master is not one of the masters"))?;
    for (i, t) in tables.iter().enumerate() {
        if *t.script_list != *b.script_list || *t.feature_list != *b.feature_list || t.feature_variations != b.feature_variations {
            return Err(bad(format!("master {i}: the GPOS scripts, features or feature variations differ from the base master's")));
        }
        if t.lookup_list.lookups.len() != b.lookup_list.lookups.len() {
            return Err(bad(format!(
                "master {i}: the GPOS table has {} lookups and the base master's {}",
                t.lookup_list.lookups.len(),
                b.lookup_list.lookups.len()
            )));
        }
    }
    let mut lookups = Vec::with_capacity(b.lookup_list.lookups.len());
    for (k, own) in b.lookup_list.lookups.iter().enumerate() {
        let masters: Vec<&PositionLookup> = tables.iter().filter_map(|t| t.lookup_list.lookups.get(k).map(|l| &**l)).collect();
        let merged = merge_lookup(ctx, own, &masters).map_err(|e| bad(format!("GPOS lookup {k}: {}", message(&e))))?;
        lookups.push(merged);
    }
    let mut out = b.clone();
    out.lookup_list = PositionLookupList::new(lookups).into();
    Ok(out)
}

fn message(e: &TablesError) -> String {
    match e {
        TablesError::Invalid(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A lookup with its extension subtables unwrapped.
enum Kind<'a> {
    Single(Vec<&'a SinglePos>),
    Pair(Vec<&'a write_fonts::tables::gpos::PairPos>),
    Cursive(Vec<&'a CursivePosFormat1>),
    MarkBase(Vec<&'a MarkBasePosFormat1>),
    MarkLig(Vec<&'a MarkLigPosFormat1>),
    MarkMark(Vec<&'a MarkMarkPosFormat1>),
    /// Contextual and chained contextual lookups, and empty extension lookups: kept from the base
    /// master when every master's is the same.
    Other,
}

impl Kind<'_> {
    fn type_id(&self) -> u8 {
        match self {
            Kind::Single(_) => 1,
            Kind::Pair(_) => 2,
            Kind::Cursive(_) => 3,
            Kind::MarkBase(_) => 4,
            Kind::MarkLig(_) => 5,
            Kind::MarkMark(_) => 6,
            Kind::Other => 0,
        }
    }
}

fn subs<T>(l: &Lookup<T>) -> Vec<&T> {
    l.subtables.iter().map(|s| &**s).collect()
}

fn kind(l: &PositionLookup) -> Result<(Kind<'_>, LookupFlag, Option<u16>)> {
    let meta = |f: LookupFlag, m: Option<u16>| (f, m);
    let (k, (flag, mark)) = match l {
        PositionLookup::Single(l) => (Kind::Single(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::Pair(l) => (Kind::Pair(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::Cursive(l) => (Kind::Cursive(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::MarkToBase(l) => (Kind::MarkBase(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::MarkToLig(l) => (Kind::MarkLig(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::MarkToMark(l) => (Kind::MarkMark(subs(l)), meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::Contextual(l) => (Kind::Other, meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::ChainContextual(l) => (Kind::Other, meta(l.lookup_flag, l.mark_filtering_set)),
        PositionLookup::Extension(l) => (unwrap_extension(l)?, meta(l.lookup_flag, l.mark_filtering_set)),
    };
    Ok((k, flag, mark))
}

fn unwrap_extension(l: &Lookup<ExtensionSubtable>) -> Result<Kind<'_>> {
    macro_rules! all {
        ($variant:ident, $kind:ident) => {{
            let mut out = Vec::new();
            for s in &l.subtables {
                match &**s {
                    ExtensionSubtable::$variant(e) => out.push(&*e.extension),
                    _ => return Err(bad("an extension lookup mixes subtable types")),
                }
            }
            Kind::$kind(out)
        }};
    }
    Ok(match l.subtables.first().map(|s| &**s) {
        None | Some(ExtensionSubtable::Contextual(_)) | Some(ExtensionSubtable::ChainContextual(_)) => Kind::Other,
        Some(ExtensionSubtable::Single(_)) => all!(Single, Single),
        Some(ExtensionSubtable::Pair(_)) => all!(Pair, Pair),
        Some(ExtensionSubtable::Cursive(_)) => all!(Cursive, Cursive),
        Some(ExtensionSubtable::MarkToBase(_)) => all!(MarkToBase, MarkBase),
        Some(ExtensionSubtable::MarkToLig(_)) => all!(MarkToLig, MarkLig),
        Some(ExtensionSubtable::MarkToMark(_)) => all!(MarkToMark, MarkMark),
    })
}

fn lookup<T: Default>(flag: LookupFlag, mark: Option<u16>, subtables: Vec<T>) -> Lookup<T> {
    let mut l = Lookup::new(flag, subtables);
    l.mark_filtering_set = mark;
    l
}

fn merge_lookup(ctx: &Ctx, own: &PositionLookup, masters: &[&PositionLookup]) -> Result<PositionLookup> {
    let (base, flag, mark) = kind(own)?;
    let mut kinds = Vec::with_capacity(masters.len());
    for (i, m) in masters.iter().enumerate() {
        let (k, f, s) = kind(m)?;
        if k.type_id() != base.type_id() || f != flag || s != mark {
            return Err(bad(format!("master {i} has a lookup of another type, flags or mark filtering set")));
        }
        kinds.push(k);
    }
    macro_rules! per_master {
        ($kind:ident) => {{
            let mut out = Vec::with_capacity(kinds.len());
            for k in &kinds {
                if let Kind::$kind(s) = k {
                    out.push(s.clone());
                }
            }
            out
        }};
    }
    Ok(match base {
        Kind::Single(own) => PositionLookup::Single(lookup(flag, mark, single::merge(ctx, &own, &per_master!(Single))?)),
        Kind::Pair(own) => PositionLookup::Pair(lookup(flag, mark, pair::merge(ctx, &own, &per_master!(Pair))?)),
        Kind::Cursive(own) => {
            let lists = per_master!(Cursive);
            same_count(own.len(), &lists)?;
            let mut out = Vec::with_capacity(own.len());
            for k in 0..own.len() {
                let lst: Vec<&CursivePosFormat1> = lists.iter().filter_map(|l| l.get(k).copied()).collect();
                out.push(cursive(ctx, &lst)?);
            }
            PositionLookup::Cursive(lookup(flag, mark, out))
        }
        Kind::MarkBase(own) => {
            let lists = per_master!(MarkBase);
            same_count(own.len(), &lists)?;
            let mut out = Vec::with_capacity(own.len());
            for k in 0..own.len() {
                let lst: Vec<Attach> =
                    lists.iter().enumerate().filter_map(|(m, l)| l.get(k).map(|s| Attach::mark_base(ctx, m, s))).collect::<Result<_>>()?;
                let a = attach(ctx, &lst)?;
                let bases = BaseArray::new(a.base_anchors.into_iter().map(BaseRecord::new).collect());
                out.push(MarkBasePosFormat1::new(write_coverage(&a.marks), write_coverage(&a.bases), a.mark_array, bases));
            }
            PositionLookup::MarkToBase(lookup(flag, mark, out))
        }
        Kind::MarkMark(own) => {
            let lists = per_master!(MarkMark);
            same_count(own.len(), &lists)?;
            let mut out = Vec::with_capacity(own.len());
            for k in 0..own.len() {
                let lst: Vec<Attach> =
                    lists.iter().enumerate().filter_map(|(m, l)| l.get(k).map(|s| Attach::mark_mark(ctx, m, s))).collect::<Result<_>>()?;
                let a = attach(ctx, &lst)?;
                let bases = Mark2Array::new(a.base_anchors.into_iter().map(Mark2Record::new).collect());
                out.push(MarkMarkPosFormat1::new(write_coverage(&a.marks), write_coverage(&a.bases), a.mark_array, bases));
            }
            PositionLookup::MarkToMark(lookup(flag, mark, out))
        }
        Kind::MarkLig(own) => {
            let lists = per_master!(MarkLig);
            same_count(own.len(), &lists)?;
            let mut out = Vec::with_capacity(own.len());
            for k in 0..own.len() {
                let lst: Vec<Lig> = lists.iter().enumerate().filter_map(|(m, l)| l.get(k).map(|s| Lig::read(ctx, m, s))).collect::<Result<_>>()?;
                out.push(mark_lig(ctx, &lst)?);
            }
            PositionLookup::MarkToLig(lookup(flag, mark, out))
        }
        Kind::Other => {
            if !ctx.same_glyph_orders() {
                return Err(bad("a contextual lookup cannot be matched across masters whose glyph orders differ"));
            }
            if masters.iter().any(|m| *m != own) {
                return Err(bad("the masters' contextual lookups differ"));
            }
            own.clone()
        }
    })
}

fn same_count<T>(n: usize, lists: &[Vec<T>]) -> Result<()> {
    if lists.iter().any(|l| l.len() != n) {
        return Err(bad("the masters' lookups have different subtable counts"));
    }
    Ok(())
}

// ---------- cursive attachment ----------

fn cursive(ctx: &Ctx, lst: &[&CursivePosFormat1]) -> Result<CursivePosFormat1> {
    let mut glyphs = Vec::with_capacity(lst.len());
    for (m, s) in lst.iter().enumerate() {
        if s.coverage.iter().count() != s.entry_exit_record.len() {
            return Err(bad(format!("master {m}: a cursive subtable's coverage and records differ in length")));
        }
        glyphs.push(coverage(ctx, m, &s.coverage)?);
    }
    let lists: Vec<&[u16]> = glyphs.iter().map(Vec::as_slice).collect();
    let (union, padded) = merge_orders(&lists)?;
    let mut records = Vec::with_capacity(union.len());
    for j in 0..union.len() {
        let mut entries = Vec::with_capacity(lst.len());
        let mut exits = Vec::with_capacity(lst.len());
        for (s, at) in lst.iter().zip(&padded) {
            let rec = at.get(j).copied().flatten().and_then(|i| s.entry_exit_record.get(i));
            let rec = rec.ok_or_else(|| bad("a cursive glyph is missing in some masters"))?;
            entries.push(read_anchor(rec.entry_anchor.as_ref()));
            exits.push(read_anchor(rec.exit_anchor.as_ref()));
        }
        records.push(EntryExitRecord::new(ctx.anchor(&entries)?.map(Anchor::write), ctx.anchor(&exits)?.map(Anchor::write)));
    }
    Ok(CursivePosFormat1::new(write_coverage(&union), records))
}

// ---------- mark attachment ----------

/// A mark-to-base or mark-to-mark subtable: marks (glyph, class, anchor) and bases (glyph, one
/// anchor per class).
pub(crate) struct Attach {
    marks: Vec<(u16, u16, Option<Anchor>)>,
    bases: Vec<(u16, Vec<Option<Anchor>>)>,
    class_count: usize,
}

impl Attach {
    fn new(ctx: &Ctx, m: usize, cov: (&CoverageTable, &CoverageTable), marks: &MarkArray, bases: Vec<Vec<Option<Anchor>>>) -> Result<Attach> {
        let mark_glyphs = coverage(ctx, m, cov.0)?;
        let base_glyphs = coverage(ctx, m, cov.1)?;
        if mark_glyphs.len() != marks.mark_records.len() || base_glyphs.len() != bases.len() {
            return Err(bad(format!("master {m}: a mark attachment subtable's coverage and records differ in length")));
        }
        let marks: Vec<(u16, u16, Option<Anchor>)> =
            mark_glyphs.into_iter().zip(&marks.mark_records).map(|(g, r)| (g, r.mark_class, Some(Anchor::read(&r.mark_anchor)))).collect();
        let used = marks.iter().map(|m| usize::from(m.1) + 1).max().unwrap_or(0);
        let class_count = bases.first().map_or(used, |b| b.len().max(used));
        Ok(Attach { marks, bases: base_glyphs.into_iter().zip(bases).collect(), class_count })
    }

    fn mark_base(ctx: &Ctx, m: usize, s: &MarkBasePosFormat1) -> Result<Attach> {
        let bases = s.base_array.base_records.iter().map(|r| r.base_anchors.iter().map(|a| read_anchor(a.as_ref())).collect()).collect();
        Attach::new(ctx, m, (&s.mark_coverage, &s.base_coverage), &s.mark_array, bases)
    }

    fn mark_mark(ctx: &Ctx, m: usize, s: &MarkMarkPosFormat1) -> Result<Attach> {
        let bases = s.mark2_array.mark2_records.iter().map(|r| r.mark2_anchors.iter().map(|a| read_anchor(a.as_ref())).collect()).collect();
        Attach::new(ctx, m, (&s.mark1_coverage, &s.mark2_coverage), &s.mark1_array, bases)
    }
}

/// A merged mark attachment subtable.
struct Attached {
    marks: Vec<u16>,
    mark_array: MarkArray,
    bases: Vec<u16>,
    /// One anchor per class for each base.
    base_anchors: Vec<Vec<Option<AnchorTable>>>,
}

/// fontTools' `_MarkBasePosFormat1_merge`.
fn attach(ctx: &Ctx, lst: &[Attach]) -> Result<Attached> {
    let class_count = lst.iter().map(|a| a.class_count).max().unwrap_or(0);
    let mark_lists: Vec<Vec<u16>> = lst.iter().map(|a| a.marks.iter().map(|m| m.0).collect()).collect();
    let refs: Vec<&[u16]> = mark_lists.iter().map(Vec::as_slice).collect();
    let (marks, mark_at) = merge_orders(&refs)?;
    let mut records = Vec::with_capacity(marks.len());
    let mut classes = std::collections::BTreeSet::new();
    for j in 0..marks.len() {
        let recs: Vec<Option<&(u16, u16, Option<Anchor>)>> =
            lst.iter().zip(&mark_at).map(|(a, at)| at.get(j).copied().flatten().and_then(|i| a.marks.get(i))).collect();
        let class = recs.iter().flatten().map(|r| r.1).next().unwrap_or(0);
        if recs.iter().flatten().any(|r| r.1 != class) {
            return Err(bad("a mark has different classes in different masters"));
        }
        let anchors: Vec<Option<Anchor>> = recs.iter().map(|r| r.and_then(|r| r.2)).collect();
        let anchor = ctx.anchor(&anchors)?.ok_or_else(|| bad("a mark has no anchor"))?;
        classes.insert(class);
        records.push(MarkRecord::new(class, anchor.write()));
    }
    // write-fonts counts the classes the marks use, so they must be every class up to the count.
    if classes.len() != class_count || classes.iter().any(|c| usize::from(*c) >= class_count) {
        return Err(bad("the mark classes are not numbered 0 to the class count"));
    }
    let base_lists: Vec<Vec<u16>> = lst.iter().map(|a| a.bases.iter().map(|b| b.0).collect()).collect();
    let refs: Vec<&[u16]> = base_lists.iter().map(Vec::as_slice).collect();
    let (bases, base_at) = merge_orders(&refs)?;
    let mut base_records = Vec::with_capacity(bases.len());
    for j in 0..bases.len() {
        let recs: Vec<Option<&Vec<Option<Anchor>>>> =
            lst.iter().zip(&base_at).map(|(a, at)| at.get(j).copied().flatten().and_then(|i| a.bases.get(i).map(|b| &b.1))).collect();
        let mut anchors = Vec::with_capacity(class_count);
        for c in 0..class_count {
            let per: Vec<Option<Anchor>> = recs.iter().map(|r| r.and_then(|r| r.get(c).copied().flatten())).collect();
            anchors.push(ctx.anchor(&per)?.map(Anchor::write));
        }
        base_records.push(anchors);
    }
    Ok(Attached { marks, mark_array: MarkArray::new(records), bases, base_anchors: base_records })
}

/// A mark-to-ligature subtable. fontTools merges it field by field: everything must match
/// except the anchors, which interpolate, and the base master's anchors must be format 1.
pub(crate) struct Lig {
    marks: Vec<(u16, u16, Anchor)>,
    ligatures: Vec<(u16, Vec<Vec<Option<Anchor>>>)>,
}

impl Lig {
    fn read(ctx: &Ctx, m: usize, s: &MarkLigPosFormat1) -> Result<Lig> {
        let mark_glyphs = coverage(ctx, m, &s.mark_coverage)?;
        let lig_glyphs = coverage(ctx, m, &s.ligature_coverage)?;
        if mark_glyphs.len() != s.mark_array.mark_records.len() || lig_glyphs.len() != s.ligature_array.ligature_attaches.len() {
            return Err(bad(format!("master {m}: a mark-to-ligature subtable's coverage and records differ in length")));
        }
        let marks = mark_glyphs.into_iter().zip(&s.mark_array.mark_records).map(|(g, r)| (g, r.mark_class, Anchor::read(&r.mark_anchor))).collect();
        let ligatures = lig_glyphs
            .into_iter()
            .zip(&s.ligature_array.ligature_attaches)
            .map(|(g, a)| (g, a.component_records.iter().map(|c| c.ligature_anchors.iter().map(|x| read_anchor(x.as_ref())).collect()).collect()))
            .collect();
        Ok(Lig { marks, ligatures })
    }
}

fn mark_lig(ctx: &Ctx, lst: &[Lig]) -> Result<MarkLigPosFormat1> {
    let first = lst.first().ok_or_else(|| bad("no masters"))?;
    let differ = || bad("the masters' mark-to-ligature subtables differ");
    let not_format_1 = || bad("a mark-to-ligature anchor of the base master is not format 1");
    let mut records = Vec::with_capacity(first.marks.len());
    for (j, (g, class, own)) in first.marks.iter().enumerate() {
        let mut anchors = Vec::with_capacity(lst.len());
        for l in lst {
            match l.marks.get(j) {
                Some((g2, c2, a)) if g2 == g && c2 == class => anchors.push(Some(*a)),
                _ => return Err(differ()),
            }
        }
        if own.format != 1 {
            return Err(not_format_1());
        }
        let a = ctx.anchor(&anchors)?.ok_or_else(differ)?;
        records.push(MarkRecord::new(*class, a.write()));
    }
    let mut attaches = Vec::with_capacity(first.ligatures.len());
    for (j, (g, comps)) in first.ligatures.iter().enumerate() {
        let mut components = Vec::with_capacity(comps.len());
        for (c, own) in comps.iter().enumerate() {
            let mut anchors = Vec::with_capacity(own.len());
            for (k, own_anchor) in own.iter().enumerate() {
                let mut per = Vec::with_capacity(lst.len());
                for l in lst {
                    let Some((g2, comps2)) = l.ligatures.get(j) else { return Err(differ()) };
                    let row = comps2.get(c).filter(|r| r.len() == own.len() && comps2.len() == comps.len());
                    match (g2 == g, row) {
                        (true, Some(row)) => per.push(row.get(k).copied().flatten()),
                        _ => return Err(differ()),
                    }
                }
                if own_anchor.is_some_and(|a| a.format != 1) {
                    return Err(not_format_1());
                }
                anchors.push(ctx.anchor(&per)?.map(Anchor::write));
            }
            components.push(ComponentRecord::new(anchors));
        }
        attaches.push(LigatureAttach::new(components));
    }
    for l in lst {
        if l.marks.len() != first.marks.len() || l.ligatures.len() != first.ligatures.len() {
            return Err(differ());
        }
    }
    let marks: Vec<u16> = first.marks.iter().map(|m| m.0).collect();
    let ligs: Vec<u16> = first.ligatures.iter().map(|l| l.0).collect();
    Ok(MarkLigPosFormat1::new(write_coverage(&marks), write_coverage(&ligs), MarkArray::new(records), LigatureArray::new(attaches)))
}

// ---------- single adjustment ----------

mod single {
    use super::*;

    /// A single adjustment subtable: format, value format, glyphs, and one value (format 1) or
    /// one per glyph (format 2); `None` is a record of format 0.
    #[derive(Clone, Debug)]
    pub(super) struct Single {
        format: u8,
        vf: u16,
        glyphs: Vec<u16>,
        values: Vec<Option<Vr>>,
    }

    impl Single {
        fn read(ctx: &Ctx, m: usize, s: &SinglePos) -> Result<Single> {
            match s {
                SinglePos::Format1(t) => Ok(Single {
                    format: 1,
                    vf: t.value_record.format().bits(),
                    glyphs: coverage(ctx, m, &t.coverage)?,
                    values: vec![Vr::read(&t.value_record)?],
                }),
                SinglePos::Format2(t) => {
                    let glyphs = coverage(ctx, m, &t.coverage)?;
                    if glyphs.len() != t.value_records.len() {
                        return Err(bad(format!("master {m}: a single adjustment subtable's coverage and records differ in length")));
                    }
                    let vf = t.value_records.first().map_or(0, |r| r.format().bits());
                    Ok(Single { format: 2, vf, glyphs, values: t.value_records.iter().map(Vr::read).collect::<Result<_>>()? })
                }
            }
        }

        fn upgraded(&self) -> Single {
            if self.format == 2 {
                return self.clone();
            }
            let v = self.values.first().copied().flatten();
            Single { format: 2, vf: self.vf, glyphs: self.glyphs.clone(), values: vec![v; self.glyphs.len()] }
        }

        fn write(&self) -> SinglePos {
            let cov = write_coverage(&self.glyphs);
            if self.format == 1 {
                SinglePos::format_1(cov, Vr::write(self.values.first().copied().flatten().as_ref(), self.vf))
            } else {
                SinglePos::format_2(cov, self.values.iter().map(|v| Vr::write(v.as_ref(), self.vf)).collect())
            }
        }
    }

    /// `_Lookup_SinglePos_get_effective_value` for every glyph the lookup's subtables cover: the
    /// value of the first subtable that covers it, at the glyph's first entry there.
    fn effective_values(subtables: &[Single]) -> HashMap<u16, Option<Vr>> {
        let mut out = HashMap::new();
        for s in subtables {
            for (i, g) in s.glyphs.iter().enumerate() {
                out.entry(*g).or_insert_with(|| if s.format == 1 { s.values.first().copied().flatten() } else { s.values.get(i).copied().flatten() });
            }
        }
        out
    }

    /// `_Lookup_SinglePos_subtables_flatten`: one format 2 subtable of value format `vf`; a glyph
    /// in several subtables keeps its value from the last.
    fn flatten(lst: &[Single], vf: u16) -> Result<Single> {
        let lists: Vec<&[u16]> = lst.iter().map(|s| s.glyphs.as_slice()).collect();
        let (glyphs, _) = merge_orders(&lists)?;
        let at: HashMap<u16, usize> = glyphs.iter().enumerate().map(|(i, g)| (*g, i)).collect();
        let mut values = vec![None; glyphs.len()];
        for s in lst {
            for (j, g) in s.glyphs.iter().enumerate() {
                let v = if s.format == 1 { s.values.first() } else { s.values.get(j) };
                if let (Some(i), Some(v)) = (at.get(g), v)
                    && let Some(slot) = values.get_mut(*i)
                {
                    *slot = *v;
                }
            }
        }
        Ok(Single { format: 2, vf, glyphs, values })
    }

    /// One subtable: the base master's `own` merged with the masters' `lst`, whose lookups give
    /// the values `effective` (see [`effective_values`]).
    fn merge_one(ctx: &Ctx, own: &Single, lst: &[Single], effective: &[HashMap<u16, Option<Vr>>]) -> Result<Single> {
        let vf = lst.iter().fold(0, |f, s| f | s.vf);
        if lst.len() != 1 && vf & !0x000F != 0 {
            return Err(bad("single adjustments with device tables cannot be interpolated"));
        }
        if own.format == 1 && lst.iter().all(|s| s.format == 1 && s.glyphs == own.glyphs) {
            let value = if vf != 0 {
                let out = Vr::with_format(vf, own.values.first().copied().flatten().as_ref());
                let masters: Vec<Option<Vr>> = lst.iter().map(|s| Some(s.values.first().copied().flatten().unwrap_or_default())).collect();
                ctx.value_record(Some(&out), &masters)?
            } else {
                None
            };
            return Ok(Single { format: 1, vf: value.map_or(0, |v| v.mask), glyphs: own.glyphs.clone(), values: vec![value] });
        }
        let lst: Vec<Single> = lst.iter().map(Single::upgraded).collect();
        let lists: Vec<&[u16]> = lst.iter().map(|s| s.glyphs.as_slice()).collect();
        let (glyphs, padded) = merge_orders(&lists)?;
        let mut per_master: Vec<Vec<Vr>> = Vec::with_capacity(lst.len());
        for ((s, at), subs) in lst.iter().zip(&padded).zip(effective) {
            let mut vals = Vec::with_capacity(glyphs.len());
            for (j, g) in glyphs.iter().enumerate() {
                let own = at.get(j).copied().flatten().and_then(|i| s.values.get(i).copied().flatten());
                vals.push(own.or_else(|| subs.get(g).copied().flatten()).unwrap_or_else(|| Vr::with_format(vf, None)));
            }
            per_master.push(vals);
        }
        let mut values = Vec::with_capacity(glyphs.len());
        for j in 0..glyphs.len() {
            let masters: Vec<Option<Vr>> = per_master.iter().map(|v| v.get(j).copied()).collect();
            values.push(ctx.value_record(Some(&Vr::with_format(vf, None)), &masters)?);
        }
        let format = values.iter().flatten().fold(0, |f, v| f | v.effective());
        Ok(Single { format: 2, vf: format, glyphs, values })
    }

    pub(super) fn merge(ctx: &Ctx, own: &[&SinglePos], masters: &[Vec<&SinglePos>]) -> Result<Vec<SinglePos>> {
        let mut own: Vec<Single> = own.iter().map(|s| Single::read(ctx, ctx.base, s)).collect::<Result<_>>()?;
        let mut lookups: Vec<Vec<Single>> =
            masters.iter().enumerate().map(|(m, l)| l.iter().map(|s| Single::read(ctx, m, s)).collect::<Result<Vec<_>>>()).collect::<Result<_>>()?;
        let counts_differ = lookups.iter().any(|l| l.len() != lookups.first().map_or(0, Vec::len));
        if counts_differ {
            let vf = lookups.iter().flatten().fold(0, |f, s| f | s.vf);
            own = vec![flatten(&own, vf)?];
            lookups = lookups.iter().map(|l| flatten(l, vf).map(|s| vec![s])).collect::<Result<_>>()?;
        }
        if lookups.iter().any(|l| l.len() != own.len()) {
            return Err(bad("the masters' lookups have different subtable counts"));
        }
        let effective: Vec<HashMap<u16, Option<Vr>>> = lookups.iter().map(|l| effective_values(l)).collect();
        let mut out = Vec::with_capacity(own.len());
        for (k, o) in own.iter().enumerate() {
            let lst: Vec<Single> = lookups.iter().filter_map(|l| l.get(k).cloned()).collect();
            out.push(merge_one(ctx, o, &lst, &effective)?);
        }
        if counts_differ {
            let merged = out.first().cloned().ok_or_else(|| bad("no subtable"))?;
            return Ok(rebuild(&merged).iter().map(Single::write).collect());
        }
        Ok(out.iter().map(Single::write).collect())
    }

    /// otlLib's `buildSinglePos` on the merged values: one format 1 subtable per value shared by
    /// enough glyphs, one format 2 subtable for the rest, each glyph left alone in a subtable of
    /// its own; largest first. (fontTools keeps only the last glyph of a lone leftover value;
    /// every glyph is kept here.)
    fn rebuild(merged: &Single) -> Vec<Single> {
        let mut keys: Vec<Vr> = Vec::new();
        let mut by_key: HashMap<Vr, Vec<u16>> = HashMap::new();
        for (g, v) in merged.glyphs.iter().zip(&merged.values) {
            let v = v.unwrap_or_default();
            if !by_key.contains_key(&v) {
                keys.push(v);
            }
            by_key.entry(v).or_default().push(*g);
        }
        let mut result = Vec::new();
        let mut handled = std::collections::HashSet::new();
        for k in &keys {
            let glyphs = by_key.get(k).cloned().unwrap_or_default();
            if glyphs.len() * k.mask.count_ones() as usize > 5 {
                result.push(subtable(&glyphs.iter().map(|g| (*g, *k)).collect::<Vec<_>>()));
                handled.insert(*k);
            }
        }
        // Every key shares the one value format of the merged subtable.
        let rest: Vec<Vr> = keys.iter().filter(|k| !handled.contains(*k)).copied().collect();
        if rest.len() > 1 {
            let mut items: Vec<(u16, Vr)> = rest.iter().flat_map(|k| by_key.get(k).into_iter().flatten().map(|g| (*g, *k))).collect();
            items.sort_by_key(|(g, _)| *g);
            result.push(subtable(&items));
        } else {
            for k in &rest {
                for g in by_key.get(k).into_iter().flatten() {
                    result.push(subtable(&[(*g, *k)]));
                }
            }
        }
        result.sort_by_key(|s| (std::cmp::Reverse(s.glyphs.len()), s.glyphs.first().copied().unwrap_or(0)));
        result
    }

    /// `buildSinglePosSubtable`.
    fn subtable(items: &[(u16, Vr)]) -> Single {
        let vf = items.iter().fold(0, |f, (_, v)| f | v.mask);
        let glyphs: Vec<u16> = items.iter().map(|(g, _)| *g).collect();
        let values: Vec<Vr> = items.iter().map(|(_, v)| Vr::with_format(vf, Some(v))).collect();
        if values.windows(2).all(|w| matches!(w, [a, b] if a == b)) {
            let v = values.first().copied().filter(|_| vf != 0);
            Single { format: 1, vf, glyphs, values: vec![v] }
        } else {
            Single { format: 2, vf, glyphs, values: values.into_iter().map(Some).collect() }
        }
    }
}
