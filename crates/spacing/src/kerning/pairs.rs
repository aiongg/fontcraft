//! The Kerning window's pair list: a master's stored pairs filtered with the window's search
//! options and sorted by a column, and the pair edits of the window (paste and delete).
//!
//! Behaviour notes: `docs/spec/08-spacing-kerning.md` § 3.8; rows `kerning.listPairs`,
//! `kerning.filterPairs`, `kerning.sortPairs`, `kerning.windowSearchOptions`,
//! `kerning.pastePairs` and `kerning.deletePairs` in `spec/inventory/08-spacing-kerning.toml`.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use fontcraft_model::{Font, KernFirst, KernTable, MasterKerning, side_group};

use super::sides::{GroupSide, Side, facing};
use super::{Direction, Position};

/// How the search term is compared with an entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Match {
    #[default]
    Contains,
    Is,
    BeginsWith,
    EndsWith,
    DoesNotContain,
}

impl Match {
    pub const ALL: [Match; 5] = [Match::Contains, Match::Is, Match::BeginsWith, Match::EndsWith, Match::DoesNotContain];

    /// `contains`, `is`, `beginsWith`, `endsWith`, `doesNotContain`.
    pub fn name(self) -> &'static str {
        match self {
            Match::Contains => "contains",
            Match::Is => "is",
            Match::BeginsWith => "beginsWith",
            Match::EndsWith => "endsWith",
            Match::DoesNotContain => "doesNotContain",
        }
    }

    pub fn from_name(name: &str) -> Option<Match> {
        Match::ALL.into_iter().find(|m| m.name() == name)
    }

    fn test(self, text: &str, term: &str) -> bool {
        match self {
            Match::Contains => text.contains(term),
            Match::Is => text == term,
            Match::BeginsWith => text.starts_with(term),
            Match::EndsWith => text.ends_with(term),
            Match::DoesNotContain => !text.contains(term),
        }
    }
}

/// Which entries the search keeps: groups or glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Group,
    NotGroup,
}

impl EntryKind {
    /// `group`, `notGroup`.
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::Group => "group",
            EntryKind::NotGroup => "notGroup",
        }
    }

    pub fn from_name(name: &str) -> Option<EntryKind> {
        [EntryKind::Group, EntryKind::NotGroup].into_iter().find(|k| k.name() == name)
    }
}

/// The side of the pair the search looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSide {
    Left,
    Right,
}

impl SearchSide {
    /// `left`, `right`.
    pub fn name(self) -> &'static str {
        match self {
            SearchSide::Left => "left",
            SearchSide::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Option<SearchSide> {
        [SearchSide::Left, SearchSide::Right].into_iter().find(|s| s.name() == name)
    }
}

/// A column of the list, to sort by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Left,
    Value,
    Right,
}

impl Column {
    /// `left`, `value`, `right`.
    pub fn name(self) -> &'static str {
        match self {
            Column::Left => "left",
            Column::Value => "value",
            Column::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Option<Column> {
        [Column::Left, Column::Value, Column::Right].into_iter().find(|c| c.name() == name)
    }
}

/// The window's search options. The default keeps every pair: an empty term, `contains`, both
/// kinds of entry, both sides, not limited to a glyph selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub term: String,
    pub matching: Match,
    pub kind: Option<EntryKind>,
    /// One side, or both when `None`.
    pub side: Option<SearchSide>,
    /// Keep only pairs whose searched side is one of these glyphs or a group one of them is in.
    pub glyphs: Option<Vec<String>>,
}

impl Filter {
    pub fn is_default(&self) -> bool {
        *self == Filter::default()
    }
}

/// One stored pair of the list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListedPair<'a> {
    /// Its place in the master's stored order.
    pub index: usize,
    /// The sides as stored (`@MMK_L_T`, `n`).
    pub first: &'a str,
    pub second: &'a str,
    pub value: f64,
}

impl ListedPair<'_> {
    pub fn left(&self) -> Side {
        Side::from_stored(self.first)
    }

    pub fn right(&self) -> Side {
        Side::from_stored(self.second)
    }
}

/// An entry as the list shows it: whether it is a group, and its name without `@`.
fn entry(stored: &str) -> (bool, &str) {
    match side_group(stored) {
        Some(g) => (true, g),
        None => (false, stored),
    }
}

/// The stored pairs of `master`'s own table in `direction`, in stored order. Linked masters are
/// not followed: the caller passes the master whose table it wants (`metrics_source`).
pub fn stored_pairs<'a>(font: &'a Font, direction: Direction, master: &str) -> Vec<ListedPair<'a>> {
    let mut out = Vec::new();
    for m in direction.table(&font.kerning).0.iter().filter(|m| m.master == master) {
        for f in &m.firsts {
            for (second, value) in &f.seconds {
                out.push(ListedPair { index: out.len(), first: &f.first, second, value: *value });
            }
        }
    }
    out
}

/// Keeps the pairs that pass `filter`. An empty term matches every entry, in every mode.
///
/// A side passes when it matches the term, has the entry kind asked for and (with `glyphs`) names
/// a selected glyph or a group one of them belongs to on the side facing the pair. A pair is kept
/// when one of the searched sides passes; with `doesNotContain`, when none of the searched sides
/// contains the term and one of them passes the other tests. A term that starts with `@` is
/// compared with a group's `@` spelling; any other term with the bare name.
pub fn filter<'a>(font: &Font, direction: Direction, pairs: Vec<ListedPair<'a>>, filter: &Filter) -> Vec<ListedPair<'a>> {
    if filter.is_default() {
        return pairs;
    }
    let selection = filter.glyphs.as_ref().map(|names| Selection::new(font, names));
    let side_passes = |stored: &str, position: Position| -> (bool, bool) {
        let (is_group, name) = entry(stored);
        let term_ok = term_matches(filter, is_group, name);
        let kind_ok = match filter.kind {
            None => true,
            Some(EntryKind::Group) => is_group,
            Some(EntryKind::NotGroup) => !is_group,
        };
        let selected = selection.as_ref().is_none_or(|s| s.contains(direction, position, is_group, name));
        (term_ok, kind_ok && selected)
    };
    let sides: &[Position] = match filter.side {
        Some(SearchSide::Left) => &[Position::First],
        Some(SearchSide::Right) => &[Position::Second],
        None => &[Position::First, Position::Second],
    };
    let mut pairs = pairs;
    pairs.retain(|p| {
        let tests = sides.iter().map(|pos| {
            let stored = match pos {
                Position::First => p.first,
                Position::Second => p.second,
            };
            side_passes(stored, *pos)
        });
        if filter.matching == Match::DoesNotContain {
            let mut all_terms = true;
            let mut any_other = false;
            for (term_ok, other_ok) in tests {
                all_terms &= term_ok;
                any_other |= other_ok;
            }
            all_terms && any_other
        } else {
            tests.into_iter().any(|(term_ok, other_ok)| term_ok && other_ok)
        }
    });
    pairs
}

fn term_matches(filter: &Filter, is_group: bool, name: &str) -> bool {
    let term = filter.term.as_str();
    if term.is_empty() {
        return true;
    }
    if is_group && term.starts_with('@') {
        let spelled = format!("@{name}");
        return filter.matching.test(&spelled, term);
    }
    filter.matching.test(name, term)
}

/// The selected glyphs and the groups they belong to.
struct Selection<'a> {
    glyphs: HashSet<&'a str>,
    groups: HashSet<(GroupSide, &'a str)>,
}

impl<'a> Selection<'a> {
    fn new(font: &'a Font, names: &'a [String]) -> Self {
        let glyphs: HashSet<&str> = names.iter().map(String::as_str).collect();
        let mut groups = HashSet::new();
        for name in &glyphs {
            let Some(g) = font.glyph(name) else { continue };
            for side in GroupSide::ALL {
                if let Some(group) = side.of(&g.kern_groups) {
                    groups.insert((side, group));
                }
            }
        }
        Selection { glyphs, groups }
    }

    fn contains(&self, direction: Direction, position: Position, is_group: bool, name: &str) -> bool {
        if is_group { self.groups.contains(&(facing(direction, position), name)) } else { self.glyphs.contains(name) }
    }
}

/// Sorts by a column; ties keep the stored order. Names compare byte by byte in their `@`
/// spelling; values compare numerically.
pub fn sort(pairs: &mut [ListedPair<'_>], column: Column, descending: bool) {
    let by_name = |a: &str, b: &str| -> Ordering {
        let (ga, na) = entry(a);
        let (gb, nb) = entry(b);
        let sa = ga.then_some(b'@').into_iter().chain(na.bytes());
        let sb = gb.then_some(b'@').into_iter().chain(nb.bytes());
        sa.cmp(sb)
    };
    pairs.sort_by(|a, b| {
        let o = match column {
            Column::Left => by_name(a.first, b.first),
            Column::Right => by_name(a.second, b.second),
            Column::Value => a.value.total_cmp(&b.value),
        };
        let o = if descending { o.reverse() } else { o };
        o.then(a.index.cmp(&b.index))
    });
}

/// [`stored_pairs`], then [`filter`], then [`sort`] when a column is given.
pub fn list<'a>(font: &'a Font, direction: Direction, master: &str, options: &Filter, order: Option<(Column, bool)>) -> Vec<ListedPair<'a>> {
    let mut pairs = filter(font, direction, stored_pairs(font, direction, master), options);
    if let Some((column, descending)) = order {
        sort(&mut pairs, column, descending);
    }
    pairs
}

/// A pair given in command spelling with its value.
#[derive(Clone, Debug, PartialEq)]
pub struct NewPair {
    pub left: Side,
    pub right: Side,
    pub value: f64,
}

/// A pasted pair that exists in the target master with a different value.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    /// Its place in the pasted list.
    pub index: usize,
    pub left: Side,
    pub right: Side,
    pub value: f64,
    pub existing: f64,
}

/// What a paste does to one master: every pasted pair falls in exactly one list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PastePlan {
    /// Pairs the master does not have (indices into the pasted list).
    pub added: Vec<usize>,
    /// Pairs the master has with a different value.
    pub conflicts: Vec<Conflict>,
    /// Pairs the master has with the same value.
    pub unchanged: Vec<usize>,
}

/// A key that ignores the stored spelling: (is group, name) for each side.
type SideKey<'a> = ((bool, &'a str), (bool, &'a str));

fn side_key(side: &Side) -> (bool, &str) {
    (side.is_group(), side.name())
}

/// For each side key of `master`'s own table, the first stored pair in stored order with it
/// (the one [`super::stored_pair`] finds): its first-side and pair positions.
fn index_master(m: &MasterKerning) -> HashMap<SideKey<'_>, (usize, usize)> {
    let mut at = HashMap::new();
    for (fi, f) in m.firsts.iter().enumerate() {
        let a = entry(&f.first);
        for (si, (second, _)) in f.seconds.iter().enumerate() {
            at.entry((a, entry(second))).or_insert((fi, si));
        }
    }
    at
}

/// Every entry `table` holds for `master`, as one, in stored order. A file can hold several;
/// lookups and the pair list read them all.
pub(crate) fn master_view<'a>(table: &'a KernTable, master: &str) -> Cow<'a, MasterKerning> {
    let mut entries = table.0.iter().filter(|m| m.master == master);
    let Some(first) = entries.next() else { return Cow::Owned(MasterKerning { master: master.to_string(), firsts: Vec::new() }) };
    let rest: Vec<&MasterKerning> = entries.collect();
    if rest.is_empty() {
        return Cow::Borrowed(first);
    }
    let mut merged = first.clone();
    merged.firsts.extend(rest.into_iter().flat_map(|m| m.firsts.iter().cloned()));
    Cow::Owned(merged)
}

/// Merges every entry `table` holds for `master` into the first, keeping stored order, and
/// returns its position, so an edit sees all of the master's pairs in one place.
pub(crate) fn merge_master(table: &mut KernTable, master: &str) -> Option<usize> {
    let first = table.0.iter().position(|m| m.master == master)?;
    let mut later = Vec::new();
    let mut i = first + 1;
    while i < table.0.len() {
        if table.0.get(i).is_some_and(|m| m.master == master) {
            later.push(table.0.remove(i));
        } else {
            i += 1;
        }
    }
    if let Some(m) = table.0.get_mut(first) {
        m.firsts.extend(later.into_iter().flat_map(|m| m.firsts));
    }
    Some(first)
}

/// Sorts the pasted pairs of `master` into new, conflicting and unchanged ones. When the pasted
/// list names a pair twice, the later value counts: earlier ones are left out of every list.
pub fn plan_paste(font: &Font, direction: Direction, master: &str, pairs: &[NewPair]) -> PastePlan {
    let view = master_view(direction.table(&font.kerning), master);
    let m: &MasterKerning = &view;
    let at = index_master(m);
    let last: HashMap<SideKey<'_>, usize> = pairs.iter().enumerate().map(|(i, p)| ((side_key(&p.left), side_key(&p.right)), i)).collect();
    let mut plan = PastePlan::default();
    for (i, p) in pairs.iter().enumerate() {
        let key = (side_key(&p.left), side_key(&p.right));
        if last.get(&key) != Some(&i) {
            continue;
        }
        let existing = at.get(&key).and_then(|(fi, si)| m.firsts.get(*fi)?.seconds.get(*si)).map(|(_, v)| *v);
        match existing {
            None => plan.added.push(i),
            Some(v) if v == p.value => plan.unchanged.push(i),
            Some(v) => plan.conflicts.push(Conflict { index: i, left: p.left.clone(), right: p.right.clone(), value: p.value, existing: v }),
        }
    }
    plan
}

/// Writes a paste into `table`: the added pairs, and the conflicting ones when `overwrite`. A new
/// pair is appended in the spelling [`Side::stored`] gives; an overwritten pair keeps its stored
/// spelling. One pass over the master's table, whatever the number of pairs.
pub fn apply_paste(table: &mut KernTable, direction: Direction, master: &str, pairs: &[NewPair], plan: &PastePlan, overwrite: bool) {
    let mi = match merge_master(table, master) {
        Some(i) => i,
        None => {
            table.0.push(MasterKerning { master: master.to_string(), firsts: Vec::new() });
            table.0.len() - 1
        }
    };
    let Some(m) = table.0.get_mut(mi) else { return };
    if overwrite {
        let writes: Vec<(usize, usize, f64)> = {
            let at = index_master(m);
            plan.conflicts.iter().filter_map(|c| at.get(&(side_key(&c.left), side_key(&c.right))).map(|(fi, si)| (*fi, *si, c.value))).collect()
        };
        for (fi, si, value) in writes {
            if let Some(slot) = m.firsts.get_mut(fi).and_then(|f| f.seconds.get_mut(si)) {
                slot.1 = value;
            }
        }
    }
    let mut firsts: HashMap<String, usize> = HashMap::new();
    for (i, f) in m.firsts.iter().enumerate() {
        firsts.entry(f.first.clone()).or_insert(i);
    }
    for p in plan.added.iter().filter_map(|i| pairs.get(*i)) {
        let first = p.left.stored(direction, Position::First);
        let second = p.right.stored(direction, Position::Second);
        let fi = match firsts.get(&first) {
            Some(i) => *i,
            None => {
                m.firsts.push(KernFirst { first: first.clone(), seconds: Vec::new() });
                firsts.insert(first, m.firsts.len() - 1);
                m.firsts.len() - 1
            }
        };
        if let Some(f) = m.firsts.get_mut(fi) {
            f.seconds.push((second, p.value));
        }
    }
}

/// Removes from `master`'s own table the stored pair each of `pairs` names (the first spelling in
/// stored order, as [`super::stored_pair`] finds it). Returns, for each pair, whether it was
/// there. Empty first sides and an empty master are removed with their last pair.
pub fn delete_pairs(table: &mut KernTable, master: &str, pairs: &[(Side, Side)]) -> Vec<bool> {
    let Some(mi) = merge_master(table, master) else { return vec![false; pairs.len()] };
    let Some(m) = table.0.get_mut(mi) else { return vec![false; pairs.len()] };
    let (found, gone): (Vec<bool>, HashSet<(usize, usize)>) = {
        let at = index_master(m);
        let mut gone = HashSet::new();
        let found = pairs
            .iter()
            .map(|(l, r)| match at.get(&(side_key(l), side_key(r))) {
                Some(pos) => {
                    gone.insert(*pos);
                    true
                }
                None => false,
            })
            .collect();
        (found, gone)
    };
    if gone.is_empty() {
        return found;
    }
    for (fi, f) in m.firsts.iter_mut().enumerate() {
        let mut si = 0;
        f.seconds.retain(|_| {
            let keep = !gone.contains(&(fi, si));
            si += 1;
            keep
        });
    }
    m.firsts.retain(|f| !f.seconds.is_empty());
    if m.firsts.is_empty() {
        table.0.remove(mi);
    }
    found
}
