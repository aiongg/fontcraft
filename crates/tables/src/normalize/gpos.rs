//! GPOS: pair kerning, mark-to-base, mark-to-mark, mark-to-ligature and cursive attachment as flat
//! rules per feature and language-system group.
//!
//! Ported from fontc's `otl-normalizer` (MIT OR Apache-2.0).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use read_fonts::tables::gdef::{Gdef, MarkGlyphSets};
use read_fonts::tables::gpos::{
    AnchorTable, CursivePosFormat1, Gpos, MarkBasePosFormat1, MarkLigPosFormat1, MarkMarkPosFormat1, PairPos, PairPosFormat1, PairPosFormat2,
    PositionLookupList, PositionSubtables, ValueRecord,
};
use read_fonts::tables::layout::LookupFlag;
use read_fonts::types::{F2Dot14, GlyphId16};

use super::deltas::{DeltaComputer, ResolvedValue, Resolver};
use super::names::NameMap;
use super::rules::{self, GlyphSet, Lookup, PrintNames, SingleRule};
use super::{Budget, Out, read_err};
use crate::Result;

/// Prints the normalised GPOS rules. `coords`: resolve variable values at this location.
pub(super) fn print(
    out: &mut Out,
    table: &Gpos<'_>,
    gdef: Option<&Gdef<'_>>,
    names: &NameMap,
    coords: Option<&[F2Dot14]>,
    budget: &mut Budget,
) -> Result<()> {
    let ivs = match gdef.and_then(|g| g.item_var_store()) {
        Some(ivs) => Some(ivs.map_err(read_err("GDEF's item variation store"))?),
        None => None,
    };
    let resolver = match (ivs, coords) {
        (None, _) => Resolver::None,
        (Some(ivs), Some(coords)) => Resolver::At(ivs, coords.to_vec()),
        (Some(ivs), None) => Resolver::Masters(DeltaComputer::new(ivs, None, budget)?),
    };
    let mark_glyph_sets = match gdef.and_then(|g| g.mark_glyph_sets_def()) {
        Some(sets) => Some(sets.map_err(read_err("GDEF's mark glyph sets"))?),
        None => None,
    };

    let script_list = table.script_list().map_err(read_err("GPOS's script list"))?;
    let feature_list = table.feature_list().map_err(read_err("GPOS's feature list"))?;
    let lang_systems = rules::get_lang_systems(&script_list, &feature_list, budget)?;
    let lookup_rules = get_lookup_rules(&table.lookup_list().map_err(read_err("GPOS's lookup list"))?, &resolver, budget)?;

    for sys in &lang_systems {
        out.put(format_args!("\n"))?;
        sys.print_header(out)?;
        let pairpos = lookup_rules.pairpos_rules(&sys.lookups, budget)?;
        let markmark = normalize_mark_lookups(&lookup_rules.markmark, &sys.lookups, budget)?;
        let markbase = normalize_mark_lookups(&lookup_rules.markbase, &sys.lookups, budget)?;
        let markliga = normalize_mark_lookups(&lookup_rules.markliga, &sys.lookups, budget)?;
        let cursive = lookup_rules.cursive_rules(&sys.lookups, budget)?;

        let sets = mark_glyph_sets.as_ref();
        print_rules(out, "PairPos", &pairpos, names, sets)?;
        print_rules(out, "MarkToBase", &markbase, names, sets)?;
        print_rules(out, "MarkToMark", &markmark, names, sets)?;
        print_rules(out, "MarkToLig", &markliga, names, sets)?;
        print_rules(out, "CursivePos", &cursive, names, sets)?;
    }
    Ok(())
}

fn print_rules<T: PrintNames + Clone>(
    out: &mut Out,
    type_name: &str,
    rules: &[SingleRule<'_, T>],
    names: &NameMap,
    mark_glyph_sets: Option<&MarkGlyphSets<'_>>,
) -> Result<()> {
    if rules.is_empty() {
        return Ok(());
    }
    out.put(format_args!("# {} {type_name} rules\n", rules.len()))?;
    let mut last_flag = None;
    let mut last_filter_set = None;
    for rule in rules {
        let (flags, filter_set_id) = rule.lookup_flags();
        if last_flag != Some(flags) {
            out.put(format_args!("# lookupflag {flags:?}\n"))?;
            last_flag = Some(flags);
        }
        if filter_set_id != last_filter_set
            && let (Some(filter_id), Some(sets)) = (filter_set_id, mark_glyph_sets)
        {
            let coverage = sets.coverages().get(usize::from(filter_id)).map_err(read_err("a mark glyph set"))?;
            let glyphs: GlyphSet = coverage.iter().collect();
            out.put(format_args!("# filter glyphs: {}\n", glyphs.printer(names)))?;
        }
        last_filter_set = filter_set_id;
        out.put(format_args!("{}\n", rule.printer(names)))?;
    }
    Ok(())
}

/// A value record with its device tables resolved.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ResolvedValueRecord {
    x_advance: ResolvedValue,
    y_advance: ResolvedValue,
    x_placement: ResolvedValue,
    y_placement: ResolvedValue,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ResolvedAnchor {
    x: ResolvedValue,
    y: ResolvedValue,
}

impl ResolvedValueRecord {
    fn new(record: &ValueRecord<'_>, resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Self> {
        Ok(ResolvedValueRecord {
            x_advance: ResolvedValue::new(record.x_advance(), record.x_advance_device(), resolver, budget)?,
            y_advance: ResolvedValue::new(record.y_advance(), record.y_advance_device(), resolver, budget)?,
            x_placement: ResolvedValue::new(record.x_placement(), record.x_placement_device(), resolver, budget)?,
            y_placement: ResolvedValue::new(record.y_placement(), record.y_placement_device(), resolver, budget)?,
        })
    }

    /// The x advance, when nothing else is set.
    fn maybe_just_adv(&self) -> Option<&ResolvedValue> {
        (self.y_advance.is_zero() && self.y_placement.is_zero() && self.x_placement.is_zero()).then_some(&self.x_advance)
    }

    fn is_zero(&self) -> bool {
        self.y_advance.is_zero() && self.x_advance.is_zero() && self.y_placement.is_zero() && self.x_placement.is_zero()
    }

    fn add_in_place(&mut self, other: &Self) {
        self.x_advance.add_in_place(&other.x_advance);
        self.y_advance.add_in_place(&other.y_advance);
        self.x_placement.add_in_place(&other.x_placement);
        self.y_placement.add_in_place(&other.y_placement);
    }
}

impl ResolvedAnchor {
    fn new(anchor: &AnchorTable<'_>, resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Self> {
        match anchor {
            AnchorTable::Format1(_) | AnchorTable::Format2(_) => {
                Ok(ResolvedAnchor { x: ResolvedValue::plain(anchor.x_coordinate()), y: ResolvedValue::plain(anchor.y_coordinate()) })
            }
            AnchorTable::Format3(table) => Ok(ResolvedAnchor {
                x: ResolvedValue::new(Some(table.x_coordinate()), table.x_device(), resolver, budget)?,
                y: ResolvedValue::new(Some(table.y_coordinate()), table.y_device(), resolver, budget)?,
            }),
        }
    }
}

impl fmt::Display for ResolvedValueRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.maybe_just_adv() {
            Some(xadv) if xadv.is_zero() => Ok(()),
            Some(xadv) => write!(f, "{xadv}"),
            None => write!(f, "<{} {} {} {}>", self.x_placement, self.y_placement, self.x_advance, self.y_advance),
        }
    }
}

impl fmt::Display for ResolvedAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@(x: {}, y: {})", self.x, self.y)
    }
}

// ── the decomposed lookups ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct LookupRules {
    pairpos: Vec<Lookup<PairPosRule>>,
    markbase: Vec<Lookup<MarkAttachmentRule>>,
    markmark: Vec<Lookup<MarkAttachmentRule>>,
    markliga: Vec<Lookup<MarkAttachmentRule>>,
    cursive: Vec<Lookup<CursivePosRule>>,
}

type PairKey = (GlyphId16, GlyphSet, (LookupFlag, Option<u16>));
type GroupKey = (GlyphId16, (LookupFlag, Option<u16>), ResolvedValueRecord, ResolvedValueRecord);

impl LookupRules {
    /// The pair rules of `lookups`: values for the same pair add up across lookups, then pairs
    /// with the same first glyph and values are printed as one rule.
    fn pairpos_rules<'a>(&'a self, lookups: &[u16], budget: &mut Budget) -> Result<Vec<SingleRule<'a, PairPosRule>>> {
        // Every `second` here is a single glyph, so the key's ordering agrees with its equality.
        let mut pairmap: BTreeMap<PairKey, SingleRule<'a, PairPosRule>> = BTreeMap::new();
        for rule in self.pairpos.iter().filter(|l| lookups.contains(&l.lookup_id)).flat_map(Lookup::iter) {
            budget.charge(1)?;
            match pairmap.entry((rule.rule().first, rule.rule().second.clone(), rule.lookup_flags())) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(rule);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => entry.get_mut().rule_mut().merge(rule.rule()),
            }
        }
        let mut seen: BTreeMap<GroupKey, SingleRule<'a, PairPosRule>> = BTreeMap::new();
        for rule in pairmap.into_values() {
            budget.charge(1)?;
            let key = (rule.rule().first, rule.lookup_flags(), rule.rule().record1.clone(), rule.rule().record2.clone());
            match seen.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(rule);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => entry.get_mut().rule_mut().second.combine(&rule.rule().second),
            }
        }
        let mut result: Vec<_> = seen.into_values().collect();
        result.sort();
        Ok(result)
    }

    /// The cursive rules of `lookups`: a later lookup's entry and exit for a glyph replace an
    /// earlier one's.
    fn cursive_rules<'a>(&'a self, lookups: &[u16], budget: &mut Budget) -> Result<Vec<SingleRule<'a, CursivePosRule>>> {
        let mut rules = BTreeMap::new();
        for rule in self.cursive.iter().filter(|l| lookups.contains(&l.lookup_id)).flat_map(Lookup::iter) {
            budget.charge(1)?;
            rules.insert((rule.rule().glyph, rule.lookup_flags()), rule);
        }
        Ok(rules.into_values().collect())
    }
}

/// The mark rules of the lookups among `ids`: a (base, mark) pair is kept only from the last
/// lookup that has it.
fn normalize_mark_lookups<'a>(
    all: &'a [Lookup<MarkAttachmentRule>],
    ids: &[u16],
    budget: &mut Budget,
) -> Result<Vec<SingleRule<'a, MarkAttachmentRule>>> {
    let lookups: Vec<&Lookup<MarkAttachmentRule>> = all.iter().filter(|l| ids.contains(&l.lookup_id)).collect();
    let mut last_seen: HashMap<(GlyphId16, GlyphId16), usize> = HashMap::new();
    for (i, lookup) in lookups.iter().enumerate() {
        for rule in lookup.iter() {
            for pair in rule.rule().iter_base_mark_pairs() {
                budget.charge(1)?;
                last_seen.insert(pair, i);
            }
        }
    }
    let mut result = Vec::new();
    for (i, lookup) in lookups.iter().enumerate() {
        for mut rule in lookup.iter() {
            let to_remove: Vec<GlyphId16> =
                rule.rule().iter_base_mark_pairs().filter(|pair| last_seen.get(pair) != Some(&i)).map(|pair| pair.1).collect();
            budget.charge(to_remove.len().saturating_add(1))?;
            if to_remove.is_empty() || rule.rule_mut().remove_marks(&to_remove) {
                result.push(rule);
            }
        }
    }
    result.sort();
    Ok(result)
}

fn get_lookup_rules(lookups: &PositionLookupList<'_>, resolver: &Resolver<'_>, budget: &mut Budget) -> Result<LookupRules> {
    let mut result = LookupRules::default();
    for (id, lookup) in lookups.lookups().iter().enumerate() {
        let lookup = lookup.map_err(read_err("a GPOS lookup"))?;
        let id = u16::try_from(id).unwrap_or(u16::MAX);
        let flag = lookup.lookup_flag();
        let mark_filter_id = if flag.contains(LookupFlag::USE_MARK_FILTERING_SET) { lookup.mark_filtering_set() } else { None };
        match lookup.subtables().map_err(read_err("a GPOS lookup's subtables"))? {
            PositionSubtables::Pair(subs) => {
                let subs: Vec<_> = subs.iter().filter_map(|s| s.ok()).collect();
                // The oracle keeps no mark filtering set for pair lookups.
                result.pairpos.push(Lookup::new(id, get_pairpos_rules(&subs, resolver, budget)?, flag, None));
            }
            PositionSubtables::MarkToBase(subs) => {
                let subs: Vec<_> = subs.iter().filter_map(|s| s.ok()).collect();
                result.markbase.push(Lookup::new(id, get_mark_base_rules(&subs, resolver, budget)?, flag, mark_filter_id));
            }
            PositionSubtables::MarkToMark(subs) => {
                let subs: Vec<_> = subs.iter().filter_map(|s| s.ok()).collect();
                result.markmark.push(Lookup::new(id, get_mark_mark_rules(&subs, resolver, budget)?, flag, mark_filter_id));
            }
            PositionSubtables::MarkToLig(subs) => {
                let subs: Vec<_> = subs.iter().filter_map(|s| s.ok()).collect();
                result.markliga.push(Lookup::new(id, get_mark_liga_rules(&subs, resolver, budget)?, flag, mark_filter_id));
            }
            PositionSubtables::Cursive(subs) => {
                let subs: Vec<_> = subs.iter().filter_map(|s| s.ok()).collect();
                result.cursive.push(Lookup::new(id, get_cursive_rules(&subs, resolver, budget)?, flag, mark_filter_id));
            }
            _ => {}
        }
    }
    Ok(result)
}

// ── pair positioning ────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PairPosRule {
    first: GlyphId16,
    second: GlyphSet,
    record1: ResolvedValueRecord,
    record2: ResolvedValueRecord,
}

impl PairPosRule {
    fn merge(&mut self, other: &Self) {
        self.record1.add_in_place(&other.record1);
        self.record2.add_in_place(&other.record2);
    }
}

impl PrintNames for PairPosRule {
    fn fmt_names(&self, f: &mut fmt::Formatter<'_>, names: &NameMap) -> fmt::Result {
        write!(f, "{} {} {}", names.get(self.first), self.record1, self.second.printer(names))?;
        if self.record2.is_zero() { Ok(()) } else { write!(f, " {}", self.record2) }
    }
}

/// The pairs of a pair lookup; within it, the first subtable that has a pair wins.
fn get_pairpos_rules(subtables: &[PairPos<'_>], resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Vec<PairPosRule>> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for sub in subtables {
        match sub {
            PairPos::Format1(sub) => append_pairpos_f1_rules(sub, resolver, &mut seen, &mut result, budget)?,
            PairPos::Format2(sub) => append_pairpos_f2_rules(sub, resolver, &mut seen, &mut result, budget)?,
        }
    }
    Ok(result)
}

fn append_pairpos_f1_rules(
    subtable: &PairPosFormat1<'_>,
    resolver: &Resolver<'_>,
    seen: &mut HashSet<(GlyphId16, GlyphId16)>,
    result: &mut Vec<PairPosRule>,
    budget: &mut Budget,
) -> Result<()> {
    let err = read_err("a pair positioning subtable");
    let coverage = subtable.coverage().map_err(&err)?;
    for (gid1, pairset) in coverage.iter().zip(subtable.pair_sets().iter()) {
        let pairset = pairset.map_err(&err)?;
        for record in pairset.pair_value_records().iter() {
            let record = record.map_err(&err)?;
            budget.charge(1)?;
            let gid2 = record.second_glyph();
            if !seen.insert((gid1, gid2)) {
                continue;
            }
            let record1 = ResolvedValueRecord::new(record.value_record1(), resolver, budget)?;
            let record2 = ResolvedValueRecord::new(record.value_record2(), resolver, budget)?;
            result.push(PairPosRule { first: gid1, second: gid2.into(), record1, record2 });
        }
    }
    Ok(())
}

fn is_noop(record: &ValueRecord<'_>) -> bool {
    record.x_placement().unwrap_or(0) == 0
        && record.x_placement_device().is_none()
        && record.y_placement().unwrap_or(0) == 0
        && record.y_placement_device().is_none()
        && record.x_advance().unwrap_or(0) == 0
        && record.x_advance_device().is_none()
        && record.y_advance().unwrap_or(0) == 0
        && record.y_advance_device().is_none()
}

fn append_pairpos_f2_rules(
    subtable: &PairPosFormat2<'_>,
    resolver: &Resolver<'_>,
    seen: &mut HashSet<(GlyphId16, GlyphId16)>,
    result: &mut Vec<PairPosRule>,
    budget: &mut Budget,
) -> Result<()> {
    let err = read_err("a class pair positioning subtable");
    let coverage = subtable.coverage().map_err(&err)?;
    let class1 = subtable.class_def1().map_err(&err)?;
    let class2 = subtable.class_def2().map_err(&err)?;
    let mut reverse_class2: HashMap<u16, Vec<GlyphId16>> = HashMap::new();
    for (gid, class) in class2.iter() {
        budget.charge(1)?;
        reverse_class2.entry(class).or_default().push(gid);
    }
    let class1_records = subtable.class1_records();
    for gid1 in coverage.iter() {
        let class1_record = class1_records.get(usize::from(class1.get(gid1))).map_err(&err)?;
        for (c2, class2_record) in class1_record.class2_records().iter().enumerate() {
            let class2_record = class2_record.map_err(&err)?;
            budget.charge(1)?;
            let (raw1, raw2) = (class2_record.value_record1(), class2_record.value_record2());
            if is_noop(raw1) && is_noop(raw2) {
                continue;
            }
            let Some(glyphs) = u16::try_from(c2).ok().and_then(|c2| reverse_class2.get(&c2)) else { continue };
            let mut resolved: Option<(ResolvedValueRecord, ResolvedValueRecord)> = None;
            for &gid2 in glyphs {
                budget.charge(1)?;
                if !seen.insert((gid1, gid2)) {
                    continue;
                }
                let (record1, record2) = match &resolved {
                    Some(r) => r.clone(),
                    None => {
                        let r = (ResolvedValueRecord::new(raw1, resolver, budget)?, ResolvedValueRecord::new(raw2, resolver, budget)?);
                        resolved = Some(r.clone());
                        r
                    }
                };
                result.push(PairPosRule { first: gid1, second: gid2.into(), record1, record2 });
            }
        }
    }
    Ok(())
}

// ── mark attachment ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BaseAnchors {
    Base(ResolvedAnchor),
    Liga(Vec<Option<ResolvedAnchor>>),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct MarkAttachmentRule {
    base: GlyphId16,
    base_anchor: BaseAnchors,
    marks: BTreeMap<ResolvedAnchor, GlyphSet>,
}

impl PrintNames for MarkAttachmentRule {
    fn fmt_names(&self, f: &mut fmt::Formatter<'_>, names: &NameMap) -> fmt::Result {
        let base_name = names.get(self.base);
        match &self.base_anchor {
            BaseAnchors::Base(anchor) => writeln!(f, "{base_name} {anchor}")?,
            BaseAnchors::Liga(anchors) => {
                write!(f, "{base_name} (lig) [")?;
                for (i, anchor) in anchors.iter().enumerate() {
                    if i != 0 {
                        f.write_str(", ")?;
                    }
                    match anchor {
                        Some(a) => write!(f, "{a}")?,
                        None => f.write_str("<NULL>")?,
                    }
                }
                writeln!(f, "]")?;
            }
        }
        for (i, (anchor, glyphs)) in self.marks.iter().enumerate() {
            if i != 0 {
                writeln!(f)?;
            }
            write!(f, "  {anchor} {}", glyphs.printer(names))?;
        }
        Ok(())
    }
}

impl MarkAttachmentRule {
    fn iter_base_mark_pairs(&self) -> impl Iterator<Item = (GlyphId16, GlyphId16)> + '_ {
        self.marks.values().flat_map(GlyphSet::iter).map(|mark| (self.base, mark))
    }

    /// Removes `marks`; true when some remain.
    fn remove_marks(&mut self, marks: &[GlyphId16]) -> bool {
        self.marks.retain(|_, v| {
            v.remove_glyphs(marks);
            !v.is_empty()
        });
        !self.marks.is_empty()
    }
}

/// Mark coverage index to glyph.
fn mark_glyphs(coverage: read_fonts::tables::layout::CoverageTable<'_>) -> Vec<GlyphId16> {
    coverage.iter().collect()
}

fn get_mark_base_rules(subtables: &[MarkBasePosFormat1<'_>], resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Vec<MarkAttachmentRule>> {
    let err = read_err("a mark-to-base subtable");
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for sub in subtables {
        let base_array = sub.base_array().map_err(&err)?;
        let base_records = base_array.base_records();
        let mark_array = sub.mark_array().map_err(&err)?;
        let mark_records = mark_array.mark_records();
        let marks_by_index = mark_glyphs(sub.mark_coverage().map_err(&err)?);
        for (base_ix, base_glyph) in sub.base_coverage().map_err(&err)?.iter().enumerate() {
            let base_record = base_records.get(base_ix).map_err(&err)?;
            for (anchor_ix, base_anchor) in base_record.base_anchors(base_array.offset_data()).iter().enumerate() {
                let Some(base_anchor) = base_anchor else { continue };
                let base_anchor = ResolvedAnchor::new(&base_anchor.map_err(&err)?, resolver, budget)?;
                let mut marks: BTreeMap<ResolvedAnchor, GlyphSet> = BTreeMap::new();
                for (mark_ix, mark_record) in mark_records.iter().enumerate() {
                    budget.charge(1)?;
                    if usize::from(mark_record.mark_class()) != anchor_ix {
                        continue;
                    }
                    let Some(&mark_glyph) = marks_by_index.get(mark_ix) else { continue };
                    if !seen.insert((base_glyph, mark_glyph)) {
                        continue;
                    }
                    let mark_anchor = ResolvedAnchor::new(&mark_record.mark_anchor(mark_array.offset_data()).map_err(&err)?, resolver, budget)?;
                    marks.entry(mark_anchor).or_insert_with(|| GlyphSet::from(mark_glyph)).add(mark_glyph);
                }
                result.push(MarkAttachmentRule { base: base_glyph, base_anchor: BaseAnchors::Base(base_anchor), marks });
            }
        }
    }
    Ok(result)
}

fn get_mark_mark_rules(subtables: &[MarkMarkPosFormat1<'_>], resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Vec<MarkAttachmentRule>> {
    let err = read_err("a mark-to-mark subtable");
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for sub in subtables {
        let base_array = sub.mark2_array().map_err(&err)?;
        let base_records = base_array.mark2_records();
        let mark_array = sub.mark1_array().map_err(&err)?;
        let mark_records = mark_array.mark_records();
        let marks_by_index = mark_glyphs(sub.mark1_coverage().map_err(&err)?);
        for (base_ix, base_glyph) in sub.mark2_coverage().map_err(&err)?.iter().enumerate() {
            let base_record = base_records.get(base_ix).map_err(&err)?;
            for (anchor_ix, base_anchor) in base_record.mark2_anchors(base_array.offset_data()).iter().enumerate() {
                let Some(base_anchor) = base_anchor else { continue };
                let base_anchor = ResolvedAnchor::new(&base_anchor.map_err(&err)?, resolver, budget)?;
                let mut marks: BTreeMap<ResolvedAnchor, GlyphSet> = BTreeMap::new();
                for (mark_ix, mark_record) in mark_records.iter().enumerate() {
                    budget.charge(1)?;
                    if usize::from(mark_record.mark_class()) != anchor_ix {
                        continue;
                    }
                    let Some(&mark_glyph) = marks_by_index.get(mark_ix) else { continue };
                    if !seen.insert((base_glyph, mark_glyph)) {
                        continue;
                    }
                    let mark_anchor = ResolvedAnchor::new(&mark_record.mark_anchor(mark_array.offset_data()).map_err(&err)?, resolver, budget)?;
                    marks.entry(mark_anchor).or_insert_with(|| GlyphSet::from(mark_glyph)).add(mark_glyph);
                }
                result.push(MarkAttachmentRule { base: base_glyph, base_anchor: BaseAnchors::Base(base_anchor), marks });
            }
        }
    }
    Ok(result)
}

/// Mark-to-ligature rules: one per ligature and mark class. Unlike the other mark lookups, later
/// subtables are not checked against earlier ones, as in the oracle.
fn get_mark_liga_rules(subtables: &[MarkLigPosFormat1<'_>], resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Vec<MarkAttachmentRule>> {
    let err = read_err("a mark-to-ligature subtable");
    let mut result = Vec::new();
    for sub in subtables {
        let lig_array = sub.ligature_array().map_err(&err)?;
        let lig_tables = lig_array.ligature_attaches();
        let mark_array = sub.mark_array().map_err(&err)?;
        let mark_records = mark_array.mark_records();
        let mark_count = usize::from(sub.mark_class_count());
        let marks_by_index = mark_glyphs(sub.mark_coverage().map_err(&err)?);
        for (lig_ix, lig_glyph) in sub.ligature_coverage().map_err(&err)?.iter().enumerate() {
            let lig_attach = lig_tables.get(lig_ix).map_err(&err)?;
            for anchor_ix in 0..mark_count {
                let mut comp_anchors = Vec::new();
                for comp in lig_attach.component_records().iter() {
                    budget.charge(1)?;
                    let comp = comp.map_err(&err)?;
                    let anchor = match comp.ligature_anchors(lig_attach.offset_data()).get(anchor_ix) {
                        None => None,
                        Some(anchor) => Some(ResolvedAnchor::new(&anchor.map_err(&err)?, resolver, budget)?),
                    };
                    comp_anchors.push(anchor);
                }
                let mut marks: BTreeMap<ResolvedAnchor, GlyphSet> = BTreeMap::new();
                for (mark_ix, mark_record) in mark_records.iter().enumerate() {
                    budget.charge(1)?;
                    if usize::from(mark_record.mark_class()) != anchor_ix {
                        continue;
                    }
                    let Some(&mark_glyph) = marks_by_index.get(mark_ix) else { continue };
                    let mark_anchor = ResolvedAnchor::new(&mark_record.mark_anchor(mark_array.offset_data()).map_err(&err)?, resolver, budget)?;
                    marks.entry(mark_anchor).or_insert_with(|| GlyphSet::from(mark_glyph)).add(mark_glyph);
                }
                result.push(MarkAttachmentRule { base: lig_glyph, base_anchor: BaseAnchors::Liga(comp_anchors), marks });
            }
        }
    }
    Ok(result)
}

// ── cursive attachment ──────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CursivePosRule {
    glyph: GlyphId16,
    entry: Option<ResolvedAnchor>,
    exit: Option<ResolvedAnchor>,
}

impl PrintNames for CursivePosRule {
    fn fmt_names(&self, f: &mut fmt::Formatter<'_>, names: &NameMap) -> fmt::Result {
        writeln!(f, "{}", names.get(self.glyph))?;
        f.write_str("  entry: ")?;
        match &self.entry {
            Some(anchor) => writeln!(f, "{anchor}")?,
            None => writeln!(f, "<NULL>")?,
        }
        f.write_str("  exit: ")?;
        match &self.exit {
            Some(anchor) => write!(f, "{anchor}"),
            None => f.write_str("<NULL>"),
        }
    }
}

fn get_cursive_rules(subtables: &[CursivePosFormat1<'_>], resolver: &Resolver<'_>, budget: &mut Budget) -> Result<Vec<CursivePosRule>> {
    let err = read_err("a cursive attachment subtable");
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for sub in subtables {
        let coverage = sub.coverage().map_err(&err)?;
        let data = sub.offset_data();
        for (gid, entry_exit) in coverage.iter().zip(sub.entry_exit_record()) {
            budget.charge(1)?;
            if !seen.insert(gid) {
                continue;
            }
            let entry = match entry_exit.entry_anchor(data) {
                None => None,
                Some(anchor) => Some(ResolvedAnchor::new(&anchor.map_err(&err)?, resolver, budget)?),
            };
            let exit = match entry_exit.exit_anchor(data) {
                None => None,
                Some(anchor) => Some(ResolvedAnchor::new(&anchor.map_err(&err)?, resolver, budget)?),
            };
            result.push(CursivePosRule { glyph: gid, entry, exit });
        }
    }
    Ok(result)
}
