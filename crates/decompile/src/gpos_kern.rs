//! Pair kerning recovered from a compiled font's GPOS `kern` feature (`open.kerningMostlyPreserved`).
//!
//! The value a shaper gives every pair the feature's pair lookups cover is worked out first:
//! lookups apply in lookup-list order and their values add up; inside one lookup the first
//! subtable that applies to a pair decides it. A class subtable (format 2) applies to every pair
//! whose first glyph it covers, class 0 included; a glyph-pair subtable (format 1) only to the
//! pairs it lists. Extension lookups are read through. Contextual kerning is not read.
//!
//! The document then gets kerning that gives the same value for every covered pair, as
//! `kerning.getPair` reads it: each class becomes a kerning group named after its first glyph in
//! glyph order, class pairs become group pairs, and every covered pair whose value the group pairs
//! do not give is stored as a glyph pair. A glyph keeps the first class it is found in on each
//! side; a later class with other members is spelled out as glyph pairs. Feature records that
//! only right-to-left scripts use fill the right-to-left table, in logical order, as the GPOS
//! stores it; a pair two feature records of one direction cover takes the first record's value.
//!
//! Input is untrusted: at most [`MAX_KERN_PAIRS`] covered pairs are read, a class subtable with
//! more than [`MAX_CLASS_RECORDS`] class pairs is skipped, and all work is capped; each cut is a
//! warning, never a failure.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use fontcraft_model::{KernFirst, KernGroups, Kerning, MasterKerning};
use fontcraft_spacing::kerning::{Direction, GroupSide, MAX_GROUP_NAME_LEN, Position, Side, check_group_name, facing};
use skrifa::raw::tables::gpos::{Gpos, PairPos, PairPosFormat1, PairPosFormat2, PositionSubtables, ValueRecord};
use skrifa::raw::types::{GlyphId16, Tag};
use skrifa::raw::{FontRef, TableProvider};

/// Most covered pairs read from one font.
pub const MAX_KERN_PAIRS: usize = 2_000_000;
/// Most class pairs (class-1 count × class-2 count) in a class subtable that is read.
pub const MAX_CLASS_RECORDS: usize = 1_000_000;
/// Most units of work (records, glyphs, offsets looked at) spent on one font's kerning.
const MAX_STEPS: usize = 16 * MAX_KERN_PAIRS;

/// Scripts written right to left, by OpenType script tag.
const RTL_SCRIPTS: [&[u8; 4]; 26] = [
    b"arab", b"hebr", b"syrc", b"thaa", b"nko ", b"adlm", b"rohg", b"mand", b"samr", b"mend", b"yezi", b"ougr", b"sogd", b"sogo", b"chrs", b"elym",
    b"hatr", b"nbat", b"palm", b"phnx", b"phli", b"phlp", b"prti", b"sarb", b"narb", b"avst",
];

/// The kerning read from a font.
#[derive(Default)]
pub(crate) struct Recovered {
    pub kerning: Kerning,
    /// Each glyph's kerning groups, by glyph id.
    pub groups: Vec<KernGroups>,
}

/// Reads the `kern` feature's pair kerning into kerning for `master`. `names` are the glyph names
/// by glyph id. A font without that feature gives empty kerning and no warning.
pub(crate) fn read(font: &FontRef, names: &[String], master: &str, warnings: &mut Vec<String>) -> Recovered {
    let Ok(gpos) = font.gpos() else { return Recovered::default() };
    let mut reader = Reader::new(names.len());
    let records = kern_records(&gpos, &mut reader.budget);
    if records.is_empty() {
        if reader.budget.exhausted {
            warnings.push("the kerning feature was not read: its script and feature lists are too large".into());
        }
        return Recovered::default();
    }
    let Ok(lookup_list) = gpos.lookup_list() else {
        warnings.push("the kerning lookups could not be read".into());
        return Recovered::default();
    };
    let lookups = lookup_list.lookups();
    let mut cache: HashMap<u16, LookupPairs> = HashMap::new();
    for rec in &records {
        for &li in &rec.lookups {
            if reader.budget.exhausted {
                break;
            }
            if let Entry::Vacant(slot) = cache.entry(li) {
                let pairs = match lookups.get(usize::from(li)) {
                    Ok(lookup) => reader.lookup(&lookup),
                    Err(_) => {
                        reader.unreadable += 1;
                        LookupPairs::default()
                    }
                };
                slot.insert(pairs);
            }
        }
    }

    let mut groups = Groups::new(names);
    let mut kerning = Kerning::default();
    for direction in [Direction::Ltr, Direction::Rtl] {
        let mine: Vec<&KernRecord> = records.iter().filter(|r| if direction == Direction::Rtl { r.rtl } else { r.ltr }).collect();
        if mine.is_empty() {
            continue;
        }
        let table = direction_table(direction, &mine, &cache, &mut groups, &mut reader.budget, names);
        if !table.is_empty() {
            direction.table_mut(&mut kerning).0.push(MasterKerning { master: master.to_string(), firsts: table });
        }
    }

    if reader.contextual > 0 {
        warnings.push(format!("{} kerning lookups that are not pair lookups (contextual kerning) were not read", reader.contextual));
    }
    if reader.skipped_classes > 0 {
        warnings.push(format!("{} kerning class tables with more than {MAX_CLASS_RECORDS} class pairs were skipped", reader.skipped_classes));
    }
    if reader.unreadable > 0 {
        warnings.push(format!("{} damaged kerning lookups or subtables were skipped", reader.unreadable));
    }
    if reader.budget.exhausted {
        warnings.push(format!("kerning was read only in part: the font covers more than {MAX_KERN_PAIRS} pairs"));
    }
    Recovered { kerning, groups: groups.into_kern_groups() }
}

/// A `kern` feature record that some script uses: its lookups in the order they apply, and the
/// directions of the scripts that use it.
struct KernRecord {
    lookups: Vec<u16>,
    ltr: bool,
    rtl: bool,
}

/// The `kern` feature records, in feature-list order.
fn kern_records(gpos: &Gpos, budget: &mut Budget) -> Vec<KernRecord> {
    // Which kinds of script use each feature record.
    let mut used: HashMap<u16, (bool, bool, bool)> = HashMap::new();
    if let Ok(scripts) = gpos.script_list() {
        for rec in scripts.script_records() {
            // Records may share one Script and one LangSys, so every record is charged, not only
            // the feature indices it leads to.
            if !budget.step(1) {
                return Vec::new();
            }
            let tag = rec.script_tag();
            let rtl = RTL_SCRIPTS.iter().any(|t| Tag::new(t) == tag);
            let dflt = tag == Tag::new(b"DFLT");
            let Ok(script) = rec.script(scripts.offset_data()) else { continue };
            let defaults = script.default_lang_sys().into_iter().flatten();
            let others = script.lang_sys_records().iter().filter_map(|l| l.lang_sys(script.offset_data()).ok());
            for lang in defaults.chain(others) {
                if !budget.step(1) {
                    return Vec::new();
                }
                let required = Some(lang.required_feature_index()).filter(|&i| i != 0xFFFF);
                for fi in lang.feature_indices().iter().map(|i| i.get()).chain(required) {
                    if !budget.step(1) {
                        return Vec::new();
                    }
                    let u = used.entry(fi).or_default();
                    if dflt {
                        u.2 = true;
                    } else if rtl {
                        u.1 = true;
                    } else {
                        u.0 = true;
                    }
                }
            }
        }
    }
    let Ok(features) = gpos.feature_list() else { return Vec::new() };
    let mut out = Vec::new();
    for (fi, rec) in features.feature_records().iter().enumerate() {
        if rec.feature_tag() != Tag::new(b"kern") {
            continue;
        }
        // A record no script uses is never applied.
        let Some(&(ltr, rtl, dflt)) = u16::try_from(fi).ok().and_then(|fi| used.get(&fi)) else { continue };
        let Ok(feature) = rec.feature(features.offset_data()) else { continue };
        let indices = feature.lookup_list_indices();
        if !budget.step(indices.len()) {
            return Vec::new();
        }
        // Shapers apply a feature's lookups in lookup-list order, whatever order the record lists.
        let mut lookups: Vec<u16> = indices.iter().map(|i| i.get()).collect();
        lookups.sort_unstable();
        lookups.dedup();
        out.push(KernRecord { lookups, ltr: ltr || (dflt && !rtl), rtl });
    }
    out
}

/// What one lookup gives.
#[derive(Default)]
struct LookupPairs {
    /// Every pair the lookup applies to with its value, by [`key`].
    values: HashMap<u32, i32>,
    /// Its class subtables, in order.
    classes: Vec<ClassSub>,
}

/// A class subtable as the lookup applies it.
struct ClassSub {
    /// By first class: the covered glyphs this subtable decides for (earlier subtables of the
    /// lookup took the others), in glyph order.
    firsts: Vec<Vec<u16>>,
    /// By second class: its glyphs, in glyph order. Class 0 is left empty: it is every other glyph.
    seconds: Vec<Vec<u16>>,
    /// The non-zero values of class pairs with a second class other than 0.
    values: Vec<(u16, u16, i32)>,
}

fn key(first: u16, second: u16) -> u32 {
    (u32::from(first) << 16) | u32::from(second)
}

fn split(key: u32) -> (u16, u16) {
    ((key >> 16) as u16, (key & 0xFFFF) as u16)
}

fn advance(v: &ValueRecord) -> i32 {
    v.x_advance().map_or(0, i32::from)
}

struct Budget {
    steps: usize,
    pairs: usize,
    exhausted: bool,
}

impl Budget {
    /// Spends `n` steps; false (and exhausted from then on) when they are not left.
    fn step(&mut self, n: usize) -> bool {
        if self.exhausted || self.steps < n {
            self.exhausted = true;
            return false;
        }
        self.steps -= n;
        true
    }

    fn pair(&mut self) -> bool {
        if self.exhausted || self.pairs == 0 {
            self.exhausted = true;
            return false;
        }
        self.pairs -= 1;
        true
    }
}

struct Reader {
    glyph_count: usize,
    budget: Budget,
    /// Per glyph: the stamp of the lookup in which a class subtable took it.
    claimed: Vec<u32>,
    lookup_stamp: u32,
    /// Per glyph: the stamp of the subtable whose second class definition lists it.
    listed: Vec<u32>,
    subtable_stamp: u32,
    contextual: usize,
    skipped_classes: usize,
    unreadable: usize,
}

impl Reader {
    fn new(glyph_count: usize) -> Reader {
        Reader {
            glyph_count,
            budget: Budget { steps: MAX_STEPS, pairs: MAX_KERN_PAIRS, exhausted: false },
            claimed: vec![0; glyph_count],
            lookup_stamp: 0,
            listed: vec![0; glyph_count],
            subtable_stamp: 0,
            contextual: 0,
            skipped_classes: 0,
            unreadable: 0,
        }
    }

    fn glyph(&self, g: GlyphId16) -> Option<u16> {
        let g = g.to_u16();
        (usize::from(g) < self.glyph_count).then_some(g)
    }

    fn is_claimed(&self, g: u16) -> bool {
        self.claimed.get(usize::from(g)) == Some(&self.lookup_stamp)
    }

    fn lookup(&mut self, lookup: &skrifa::raw::tables::gpos::PositionLookup) -> LookupPairs {
        let mut out = LookupPairs::default();
        let subtables = match lookup.subtables() {
            Ok(PositionSubtables::Pair(s)) => s,
            Ok(PositionSubtables::EmptyExtension) => return out,
            Ok(_) => {
                self.contextual += 1;
                return out;
            }
            Err(_) => {
                self.unreadable += 1;
                return out;
            }
        };
        self.lookup_stamp = self.lookup_stamp.wrapping_add(1);
        for sub in subtables.iter() {
            if !self.budget.step(1) {
                break;
            }
            match sub {
                Ok(PairPos::Format1(t)) => self.format1(&t, &mut out),
                Ok(PairPos::Format2(t)) => self.format2(&t, &mut out),
                Err(_) => self.unreadable += 1,
            }
            if self.budget.exhausted {
                break;
            }
        }
        out
    }

    fn format1(&mut self, t: &PairPosFormat1, out: &mut LookupPairs) {
        let Ok(coverage) = t.coverage() else {
            self.unreadable += 1;
            return;
        };
        let sets = t.pair_sets();
        for g1 in coverage.iter() {
            if !self.budget.step(1) {
                return;
            }
            let Some(first) = self.glyph(g1).filter(|&g| !self.is_claimed(g)) else { continue };
            let Some(Ok(set)) = coverage.get(g1).map(|i| sets.get(usize::from(i))) else { continue };
            for record in set.pair_value_records().iter() {
                if !self.budget.step(1) {
                    return;
                }
                let Ok(record) = record else { break };
                let Some(second) = self.glyph(record.second_glyph()) else { continue };
                // A pair listed twice: the first record counts.
                if let Entry::Vacant(slot) = out.values.entry(key(first, second)) {
                    if !self.budget.pair() {
                        return;
                    }
                    slot.insert(advance(record.value_record1()));
                }
            }
        }
    }

    fn format2(&mut self, t: &PairPosFormat2, out: &mut LookupPairs) {
        let (Ok(coverage), Ok(class1), Ok(class2)) = (t.coverage(), t.class_def1(), t.class_def2()) else {
            self.unreadable += 1;
            return;
        };
        let (count1, count2) = (usize::from(t.class1_count()), usize::from(t.class2_count()));
        if count1.saturating_mul(count2) > MAX_CLASS_RECORDS {
            self.skipped_classes += 1;
            // A shaper still stops at this subtable for the glyphs it covers.
            for g in coverage.iter() {
                if !self.budget.step(1) {
                    return;
                }
                if let Some(slot) = self.glyph(g).and_then(|g| self.claimed.get_mut(usize::from(g))) {
                    *slot = self.lookup_stamp;
                }
            }
            return;
        }
        if !self.budget.step(count1 + count2) {
            return;
        }
        let mut firsts = vec![Vec::new(); count1];
        for g in coverage.iter() {
            if !self.budget.step(1) {
                return;
            }
            let Some(first) = self.glyph(g).filter(|&g| !self.is_claimed(g)) else { continue };
            // A class past the class count makes a shaper go on to the next subtable.
            let Some(members) = firsts.get_mut(usize::from(class1.get(g))) else { continue };
            members.push(first);
            if let Some(slot) = self.claimed.get_mut(usize::from(first)) {
                *slot = self.lookup_stamp;
            }
        }
        self.subtable_stamp = self.subtable_stamp.wrapping_add(1);
        let mut seconds = vec![Vec::new(); count2];
        for (g, class) in class2.iter() {
            if !self.budget.step(1) {
                return;
            }
            let Some(second) = self.glyph(g) else { continue };
            // Where ranges overlap, the class the table gives the glyph counts, once.
            if class == 0 || class2.get(g) != class || self.listed.get(usize::from(second)) == Some(&self.subtable_stamp) {
                continue;
            }
            let Some(members) = seconds.get_mut(usize::from(class)) else { continue };
            members.push(second);
            if let Some(slot) = self.listed.get_mut(usize::from(second)) {
                *slot = self.subtable_stamp;
            }
        }
        for members in firsts.iter_mut().chain(seconds.iter_mut()) {
            members.sort_unstable();
        }

        let records = t.class_value_records();
        let mut unlisted: Option<Vec<u16>> = None;
        let mut values = Vec::new();
        for (c1, members1) in firsts.iter().enumerate() {
            if members1.is_empty() {
                continue;
            }
            let Ok(c1) = u16::try_from(c1) else { break };
            for c2 in 0..t.class2_count() {
                if !self.budget.step(1) {
                    return;
                }
                let value = records.get(c1, c2).map_or(0, |[v, _]| advance(&v));
                if value == 0 {
                    continue;
                }
                let members2: &[u16] = if c2 == 0 {
                    if unlisted.is_none() {
                        if !self.budget.step(self.glyph_count) {
                            return;
                        }
                        let stamp = self.subtable_stamp;
                        unlisted = Some(
                            (0..self.glyph_count).filter(|&g| self.listed.get(g) != Some(&stamp)).filter_map(|g| u16::try_from(g).ok()).collect(),
                        );
                    }
                    unlisted.as_deref().unwrap_or_default()
                } else {
                    values.push((c1, c2, value));
                    seconds.get(usize::from(c2)).map_or(&[], |m| m.as_slice())
                };
                for &first in members1 {
                    for &second in members2 {
                        if !self.budget.step(1) {
                            return;
                        }
                        if let Entry::Vacant(slot) = out.values.entry(key(first, second)) {
                            if !self.budget.pair() {
                                return;
                            }
                            slot.insert(value);
                        }
                    }
                }
            }
        }
        out.classes.push(ClassSub { firsts, seconds, values });
    }
}

/// The groups made so far on one side.
#[derive(Default)]
struct SideGroups {
    /// By glyph id.
    of: Vec<Option<u32>>,
    names: Vec<String>,
    sizes: Vec<usize>,
    used: HashSet<String>,
    next_suffix: HashMap<String, u32>,
}

struct Groups<'a> {
    glyph_names: &'a [String],
    left: SideGroups,
    right: SideGroups,
}

impl<'a> Groups<'a> {
    fn new(glyph_names: &'a [String]) -> Groups<'a> {
        let side = || SideGroups { of: vec![None; glyph_names.len()], ..Default::default() };
        Groups { glyph_names, left: side(), right: side() }
    }

    fn side(&mut self, side: GroupSide) -> Option<&mut SideGroups> {
        match side {
            GroupSide::Left => Some(&mut self.left),
            GroupSide::Right => Some(&mut self.right),
            GroupSide::Top | GroupSide::Bottom => None,
        }
    }

    fn of(&self, side: GroupSide, glyph: u16) -> Option<u32> {
        let groups = match side {
            GroupSide::Left => &self.left,
            GroupSide::Right => &self.right,
            GroupSide::Top | GroupSide::Bottom => return None,
        };
        groups.of.get(usize::from(glyph)).copied().flatten()
    }

    fn name(&self, side: GroupSide, group: u32) -> Option<&str> {
        let groups = match side {
            GroupSide::Left => &self.left,
            GroupSide::Right => &self.right,
            GroupSide::Top | GroupSide::Bottom => return None,
        };
        groups.names.get(group as usize).map(String::as_str)
    }

    /// The group that stands for a class with these members (in glyph order) on `side`: a new
    /// group of the members that have none on that side yet, else the group that has exactly
    /// these members, else none (the class's pairs are then stored glyph by glyph).
    fn represent(&mut self, side: GroupSide, members: &[u16]) -> Option<u32> {
        let names = self.glyph_names;
        let groups = self.side(side)?;
        let unassigned: Vec<u16> = members.iter().copied().filter(|&g| groups.of.get(usize::from(g)) == Some(&None)).collect();
        if let Some(&first) = unassigned.first() {
            let base = group_name(names.get(usize::from(first)).map_or("group", String::as_str));
            let name = unique(groups, base);
            let id = u32::try_from(groups.names.len()).ok()?;
            groups.used.insert(name.clone());
            groups.names.push(name);
            groups.sizes.push(unassigned.len());
            for g in unassigned {
                if let Some(slot) = groups.of.get_mut(usize::from(g)) {
                    *slot = Some(id);
                }
            }
            return Some(id);
        }
        let id = groups.of.get(usize::from(*members.first()?)).copied().flatten()?;
        let same = members.iter().all(|&g| groups.of.get(usize::from(g)) == Some(&Some(id)));
        (same && groups.sizes.get(id as usize) == Some(&members.len())).then_some(id)
    }

    fn into_kern_groups(self) -> Vec<KernGroups> {
        let name = |side: &SideGroups, g: usize| side.of.get(g).copied().flatten().and_then(|id| side.names.get(id as usize).cloned());
        (0..self.glyph_names.len()).map(|g| KernGroups { left: name(&self.left, g), right: name(&self.right, g), top: None, bottom: None }).collect()
    }
}

/// A group name from a glyph name: characters a group name may not hold become `_`.
fn group_name(glyph: &str) -> String {
    let mut out = String::new();
    for c in glyph.chars() {
        if out.len() >= MAX_GROUP_NAME_LEN {
            break;
        }
        out.push(if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' });
    }
    if check_group_name(&out).is_err() { "group".into() } else { out }
}

/// `base`, or `base.1`, `base.2`… when a group on this side already has that name.
fn unique(groups: &mut SideGroups, base: String) -> String {
    if !groups.used.contains(&base) {
        return base;
    }
    // Room for the suffix within the name limit.
    let mut stem = base.clone();
    while stem.len() > MAX_GROUP_NAME_LEN - 11 {
        stem.pop();
    }
    let next = groups.next_suffix.entry(base).or_insert(1);
    loop {
        let candidate = format!("{stem}.{next}");
        *next = next.wrapping_add(1);
        if !groups.used.contains(&candidate) {
            return candidate;
        }
    }
}

/// One direction's kerning: group pairs for the class pairs, then a glyph pair for every covered
/// pair whose value the group pairs do not give.
fn direction_table(
    direction: Direction,
    records: &[&KernRecord],
    cache: &HashMap<u16, LookupPairs>,
    groups: &mut Groups,
    budget: &mut Budget,
    names: &[String],
) -> Vec<KernFirst> {
    let (side1, side2) = (facing(direction, Position::First), facing(direction, Position::Second));

    // The value of every covered pair: the first record that covers it, its lookups added up.
    let mut effective: HashMap<u32, (i32, usize)> = HashMap::new();
    for (ri, rec) in records.iter().enumerate() {
        for li in &rec.lookups {
            let Some(pairs) = cache.get(li) else { continue };
            if !budget.step(pairs.values.len()) {
                break;
            }
            for (&k, &v) in &pairs.values {
                match effective.entry(k) {
                    Entry::Occupied(mut slot) if slot.get().1 == ri => slot.get_mut().0 = slot.get().0.saturating_add(v),
                    Entry::Occupied(_) => {}
                    Entry::Vacant(slot) => {
                        slot.insert((v, ri));
                    }
                }
            }
        }
    }

    // Group pairs, the first for each pair of groups.
    let mut group_pairs: HashMap<(u32, u32), i32> = HashMap::new();
    let mut order: Vec<(u32, u32, i32)> = Vec::new();
    let mut seen = HashSet::new();
    for rec in records {
        for li in &rec.lookups {
            if !seen.insert(*li) {
                continue;
            }
            let Some(pairs) = cache.get(li) else { continue };
            for sub in &pairs.classes {
                let reps1: Vec<Option<u32>> = sub.firsts.iter().map(|m| groups.represent(side1, m)).collect();
                let reps2: Vec<Option<u32>> = sub.seconds.iter().map(|m| groups.represent(side2, m)).collect();
                for &(c1, c2, value) in &sub.values {
                    let (Some(Some(a)), Some(Some(b))) = (reps1.get(usize::from(c1)), reps2.get(usize::from(c2))) else { continue };
                    if let Entry::Vacant(slot) = group_pairs.entry((*a, *b)) {
                        slot.insert(value);
                        order.push((*a, *b, value));
                    }
                }
            }
        }
    }

    // Glyph pairs where the group pairs give another value.
    let mut exceptions: Vec<(u32, i32)> = effective
        .into_iter()
        .filter(|&(k, (v, _))| {
            let (g1, g2) = split(k);
            let grouped = groups.of(side1, g1).zip(groups.of(side2, g2)).and_then(|p| group_pairs.get(&p)).copied().unwrap_or(0);
            grouped != v
        })
        .map(|(k, (v, _))| (k, v))
        .collect();
    exceptions.sort_unstable_by_key(|&(k, _)| k);

    let mut firsts: Vec<KernFirst> = Vec::new();
    let mut at: HashMap<u32, usize> = HashMap::new();
    for (a, b, value) in order {
        let (Some(n1), Some(n2)) = (groups.name(side1, a), groups.name(side2, b)) else { continue };
        let second = Side::Group(n2.to_string()).stored(direction, Position::Second);
        let i = *at.entry(a).or_insert_with(|| {
            firsts.push(KernFirst { first: Side::Group(n1.to_string()).stored(direction, Position::First), seconds: Vec::new() });
            firsts.len() - 1
        });
        if let Some(f) = firsts.get_mut(i) {
            f.seconds.push((second, f64::from(value)));
        }
    }
    let mut current: Option<u16> = None;
    for (k, value) in exceptions {
        let (g1, g2) = split(k);
        let (Some(n1), Some(n2)) = (names.get(usize::from(g1)), names.get(usize::from(g2))) else { continue };
        if current != Some(g1) {
            firsts.push(KernFirst { first: n1.clone(), seconds: Vec::new() });
            current = Some(g1);
        }
        if let Some(f) = firsts.last_mut() {
            f.seconds.push((n2.clone(), f64::from(value)));
        }
    }
    firsts
}
