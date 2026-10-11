//! Kerning maintenance: Clean Up (obsolete pairs), Compress (glyph sides lifted to their groups)
//! and group renaming. Behaviour notes: `docs/spec/08-spacing-kerning.md` § 3.8 and its open
//! questions 15 and 16; rows `kerning.cleanUp`, `kerning.compress*`, `kerning.renameGroup`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{Font, KernFirst, KernGroups, MasterKerning, side_group};

use super::groups::{GroupNameError, check_group_name};
use super::sides::{GroupSide, Side, facing};
use super::{Direction, NamedPair, Position};
use crate::link::metrics_source;

/// Most Compress runs one call makes.
pub const MAX_COMPRESS_RUNS: usize = 8;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MaintainError {
    #[error("the font has no master `{0}`")]
    UnknownMaster(String),
    #[error("master `{master}` shows the kerning of master `{follows}`; edit that master instead")]
    Linked { master: String, follows: String },
    #[error(transparent)]
    GroupName(#[from] GroupNameError),
    #[error("the old and the new group name are the same")]
    SameName,
    #[error("no glyph and no pair uses the {side} group `{name}`")]
    NoGroup { side: &'static str, name: String },
    #[error("the {side} group `{name}` exists already; renaming onto it would merge two groups")]
    GroupExists { side: &'static str, name: String },
    #[error("master `{master}` has no pair {first} {second}")]
    NoPair { master: String, first: String, second: String },
    #[error("the {column} side of the pair is `{found}`, not `@{name}`")]
    NotThatGroup { column: &'static str, found: String, name: String },
    #[error("master `{master}` already has the pair {first} {second}")]
    PairExists { master: String, first: String, second: String },
}

/// A stored pair with its sides in command spelling.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub first: Side,
    pub second: Side,
    pub value: f64,
}

/// Why Clean Up removed a pair: the first side, in pair order, that names nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Obsolete {
    /// A glyph side names a glyph the font does not have.
    MissingGlyph(String),
    /// A group side names a group no glyph belongs to on the side that faces the pair.
    EmptyGroup(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Removed {
    pub pair: Pair,
    pub reason: Obsolete,
}

/// A pair one run rewrote with one glyph side replaced by that glyph's group.
#[derive(Clone, Debug, PartialEq)]
pub struct Lift {
    /// The run that made it, from 1.
    pub run: usize,
    pub from: Pair,
    pub to: (Side, Side),
}

/// A pair removed because the group pair that covers it has the same value.
#[derive(Clone, Debug, PartialEq)]
pub struct Absorbed {
    pub run: usize,
    pub pair: Pair,
    pub into: (Side, Side),
}

/// A pair removed with the earlier pair of the same sides (in another spelling, or in another
/// entry for the master) when that pair was lifted or absorbed. It never applied: the earlier
/// pair shadowed it, and left alone it would apply in its place.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadowed {
    pub run: usize,
    pub pair: Pair,
    /// The earlier pair, as it was before the run.
    pub by: Pair,
}

/// Why Compress left a liftable pair alone.
#[derive(Clone, Debug, PartialEq)]
pub enum KeptWhy {
    /// A real exception: the group pair that covers it has another value.
    DifferentValue(Pair),
    /// A glyph–group pair whose lifted form would lose to group–glyph pairs stored for members
    /// of its group, changing what applies to those glyphs.
    MemberPairs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    pub pair: Pair,
    pub why: KeptWhy,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompressReport {
    /// Runs that changed something.
    pub runs: usize,
    pub lifted: Vec<Lift>,
    pub absorbed: Vec<Absorbed>,
    pub shadowed: Vec<Shadowed>,
    /// The pairs the last run left alone although a side could be lifted.
    pub kept: Vec<Kept>,
    pub pairs_before: usize,
    pub pairs_after: usize,
}

impl CompressReport {
    pub fn changed(&self) -> bool {
        self.runs > 0
    }
}

/// What a group rename changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenameReport {
    /// Glyphs whose group on the renamed side changed.
    pub glyphs: Vec<String>,
    /// The renamed pairs, in their new stored spelling.
    pub pairs: Vec<NamedPair>,
}

/// Where a rename applies.
#[derive(Clone, Debug, PartialEq)]
pub enum RenameScope {
    /// One pair of one master (command spelling); the glyphs keep their group.
    Pair { master: String, first: Side, second: Side },
    /// Every glyph and every pair of every master and direction that uses the group on that side.
    Font,
}

/// Refuses a master that shows another master's kerning.
pub fn check_editable(font: &Font, master: &str) -> Result<(), MaintainError> {
    let source = metrics_source(font, master).ok_or_else(|| MaintainError::UnknownMaster(master.to_string()))?;
    if source.id != master {
        return Err(MaintainError::Linked { master: master.to_string(), follows: source.id.clone() });
    }
    Ok(())
}

/// One stored pair of a table being worked on.
#[derive(Clone)]
struct Entry {
    first_stored: String,
    second_stored: String,
    value: f64,
    first: Side,
    second: Side,
    alive: bool,
}

impl Entry {
    fn pair(&self) -> Pair {
        Pair { first: self.first.clone(), second: self.second.clone(), value: self.value }
    }
}

/// The stored pairs of `master`'s own table, in stored order.
fn master_entries(font: &Font, direction: Direction, master: &str) -> Vec<Entry> {
    direction
        .table(&font.kerning)
        .0
        .iter()
        .filter(|m| m.master == master)
        .flat_map(|m| &m.firsts)
        .flat_map(|f| {
            f.seconds.iter().map(|(s, v)| Entry {
                first_stored: f.first.clone(),
                second_stored: s.clone(),
                value: *v,
                first: Side::from_stored(&f.first),
                second: Side::from_stored(s),
                alive: true,
            })
        })
        .collect()
}

/// Writes `entries` back as `master`'s table of `direction`, first sides in the order they first
/// appear; an empty table is removed.
fn write_back(font: &mut Font, direction: Direction, master: &str, entries: &[Entry]) {
    let mut firsts: Vec<KernFirst> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for e in entries.iter().filter(|e| e.alive) {
        let i = *at.entry(e.first_stored.as_str()).or_insert_with(|| {
            firsts.push(KernFirst { first: e.first_stored.clone(), seconds: Vec::new() });
            firsts.len() - 1
        });
        if let Some(f) = firsts.get_mut(i) {
            f.seconds.push((e.second_stored.clone(), e.value));
        }
    }
    let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
    match table.0.iter().position(|m| m.master == master) {
        Some(i) if firsts.is_empty() => {
            table.0.remove(i);
        }
        Some(i) => {
            if let Some(m) = table.0.get_mut(i) {
                m.firsts = firsts;
            }
        }
        None if firsts.is_empty() => {}
        None => table.0.push(MasterKerning { master: master.to_string(), firsts }),
    }
}

/// The group names that have at least one member, per side.
fn used_groups(font: &Font) -> HashMap<GroupSide, HashSet<&str>> {
    let mut out: HashMap<GroupSide, HashSet<&str>> = HashMap::new();
    for g in font.glyphs.iter() {
        for side in GroupSide::ALL {
            if let Some(name) = side.of(&g.kern_groups) {
                out.entry(side).or_default().insert(name);
            }
        }
    }
    out
}

fn obsolete(font: &Font, used: &HashMap<GroupSide, HashSet<&str>>, direction: Direction, e: &Entry) -> Option<Obsolete> {
    let reason = |side: &Side, position: Position| match side {
        Side::Glyph(g) => font.glyph(g).is_none().then(|| Obsolete::MissingGlyph(g.clone())),
        Side::Group(g) => {
            let alive = used.get(&facing(direction, position)).is_some_and(|set| set.contains(g.as_str()));
            (!alive).then(|| Obsolete::EmptyGroup(g.clone()))
        }
    };
    reason(&e.first, Position::First).or_else(|| reason(&e.second, Position::Second))
}

/// The pairs of `master`'s own table in `direction` that name a glyph the font lacks or a group
/// without members on the side facing the pair, in stored order.
pub fn obsolete_pairs(font: &Font, direction: Direction, master: &str) -> Vec<Removed> {
    let used = used_groups(font);
    master_entries(font, direction, master)
        .iter()
        .filter_map(|e| obsolete(font, &used, direction, e).map(|reason| Removed { pair: e.pair(), reason }))
        .collect()
}

/// Clean Up: removes the [`obsolete_pairs`] of one master and direction. A linked master is
/// refused.
pub fn clean_up(font: &mut Font, direction: Direction, master: &str) -> Result<Vec<Removed>, MaintainError> {
    check_editable(font, master)?;
    let used = used_groups(font);
    let mut list = master_entries(font, direction, master);
    let mut removed = Vec::new();
    for e in &mut list {
        if let Some(reason) = obsolete(font, &used, direction, e) {
            e.alive = false;
            removed.push(Removed { pair: e.pair(), reason });
        }
    }
    drop(used);
    if !removed.is_empty() {
        write_back(font, direction, master, &list);
    }
    Ok(removed)
}

type Key = (Side, Side);

/// The table Compress works on: the entries, the live ones by sides, and per group pair (first
/// group, second group) how many group–glyph pairs name a member of the second group.
struct Work {
    list: Vec<Entry>,
    by_sides: HashMap<Key, usize>,
    /// Per sides, the later live pairs with the same sides, which the first one shadows.
    shadows: HashMap<Key, Vec<usize>>,
    member_pairs: HashMap<(String, String), usize>,
}

impl Work {
    fn live(&self, key: &Key) -> Option<&Entry> {
        self.by_sides.get(key).and_then(|i| self.list.get(*i)).filter(|e| e.alive)
    }

    /// Removes the pairs `first` shadowed under `key`, now that it leaves those sides.
    fn drop_shadows(&mut self, key: &Key, first: &Entry, run: usize, group_of_second: &dyn Fn(&Side) -> Option<String>, report: &mut CompressReport) {
        for j in self.shadows.remove(key).unwrap_or_default() {
            let Some(slot) = self.list.get_mut(j).filter(|e| e.alive) else { continue };
            slot.alive = false;
            if let (Side::Group(an), Some(b)) = (&slot.first, group_of_second(&slot.second)) {
                decrement(&mut self.member_pairs, (an.clone(), b));
            }
            report.shadowed.push(Shadowed { run, pair: slot.pair(), by: first.pair() });
        }
    }
}

/// Compress on one master and direction: up to `runs` runs (at most [`MAX_COMPRESS_RUNS`]),
/// stopping early when a run changes nothing. A linked master is refused.
///
/// One run takes the pairs in stored order and looks at each once. For a glyph side whose glyph
/// has a group on the side facing the pair, the group pairs that would cover the pair are,
/// in lookup order, glyph–group, group–glyph and group–group. When one of them is stored, the
/// first one decides: the same value absorbs the pair (it is removed, and what applies to its
/// glyphs does not change), another value keeps it as an exception. When none is stored, the run
/// lifts one side: the first side when it can, else the second. Later pairs of the same run see
/// the earlier lifts, so of two pairs that lift to one group pair the first is lifted and the
/// second meets it. A later pair with the same sides as an earlier one never applies; it is
/// removed with the earlier pair when that one is lifted or absorbed ([`Shadowed`]).
pub fn compress(font: &mut Font, direction: Direction, master: &str, runs: usize) -> Result<CompressReport, MaintainError> {
    check_editable(font, master)?;
    let fg = facing(direction, Position::First);
    let sg = facing(direction, Position::Second);
    let group = |side: &Side, at: GroupSide| -> Option<String> {
        match side {
            Side::Glyph(g) => font.glyph(g).and_then(|g| at.of(&g.kern_groups)).map(str::to_string),
            Side::Group(_) => None,
        }
    };
    let list = master_entries(font, direction, master);
    let mut work = Work { by_sides: HashMap::new(), shadows: HashMap::new(), member_pairs: HashMap::new(), list };
    for (i, e) in work.list.iter().enumerate() {
        let key = (e.first.clone(), e.second.clone());
        match work.by_sides.entry(key) {
            std::collections::hash_map::Entry::Occupied(slot) => work.shadows.entry(slot.key().clone()).or_default().push(i),
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(i);
            }
        }
        if let (Side::Group(a), Some(b)) = (&e.first, group(&e.second, sg)) {
            *work.member_pairs.entry((a.clone(), b)).or_default() += 1;
        }
    }
    let mut report = CompressReport { pairs_before: work.list.len(), ..CompressReport::default() };
    for run in 1..=runs.min(MAX_COMPRESS_RUNS) {
        let mut changed = false;
        let mut kept = Vec::new();
        for i in 0..work.list.len() {
            let Some(e) = work.list.get(i).filter(|e| e.alive).cloned() else { continue };
            // Later duplicates of a pair's sides are inert, as in lookup.
            if work.by_sides.get(&(e.first.clone(), e.second.clone())) != Some(&i) {
                continue;
            }
            let a = group(&e.first, fg);
            let b = group(&e.second, sg);
            let ga = a.clone().map(Side::Group);
            let gb = b.clone().map(Side::Group);
            let candidates: Vec<Key> = match (&e.first, &e.second) {
                (Side::Glyph(_), Side::Glyph(_)) => {
                    [gb.clone().map(|gb| (e.first.clone(), gb)), ga.clone().map(|ga| (ga, e.second.clone())), ga.clone().zip(gb.clone())]
                        .into_iter()
                        .flatten()
                        .collect()
                }
                (Side::Glyph(_), Side::Group(_)) => ga.clone().map(|ga| (ga, e.second.clone())).into_iter().collect(),
                (Side::Group(_), Side::Glyph(_)) => gb.clone().map(|gb| (e.first.clone(), gb)).into_iter().collect(),
                (Side::Group(_), Side::Group(_)) => Vec::new(),
            };
            if candidates.is_empty() {
                continue;
            }
            if let (Some(a), Side::Group(bn)) = (&a, &e.second)
                && work.member_pairs.get(&(a.clone(), bn.clone())).is_some_and(|n| *n > 0)
            {
                kept.push(Kept { pair: e.pair(), why: KeptWhy::MemberPairs });
                continue;
            }
            if let Some((key, cover)) = candidates.iter().find_map(|k| work.live(k).map(|c| (k.clone(), c.pair()))) {
                if cover.value == e.value {
                    if let Some(slot) = work.list.get_mut(i) {
                        slot.alive = false;
                    }
                    let own = (e.first.clone(), e.second.clone());
                    work.by_sides.remove(&own);
                    if let (Side::Group(an), Some(b)) = (&e.first, &b) {
                        decrement(&mut work.member_pairs, (an.clone(), b.clone()));
                    }
                    work.drop_shadows(&own, &e, run, &|side| group(side, sg), &mut report);
                    report.absorbed.push(Absorbed { run, pair: e.pair(), into: key });
                    changed = true;
                } else {
                    kept.push(Kept { pair: e.pair(), why: KeptWhy::DifferentValue(cover) });
                }
                continue;
            }
            let (first, second) = match (&e.first, ga, gb) {
                (Side::Glyph(_), Some(ga), _) => (ga, e.second.clone()),
                (_, _, Some(gb)) => (e.first.clone(), gb),
                _ => continue,
            };
            let own = (e.first.clone(), e.second.clone());
            work.by_sides.remove(&own);
            if let (Side::Group(an), Some(b)) = (&e.first, &b) {
                decrement(&mut work.member_pairs, (an.clone(), b.clone()));
            }
            work.drop_shadows(&own, &e, run, &|side| group(side, sg), &mut report);
            if let (Side::Group(an), Some(b)) = (&first, &b)
                && !second.is_group()
            {
                *work.member_pairs.entry((an.clone(), b.clone())).or_default() += 1;
            }
            work.by_sides.insert((first.clone(), second.clone()), i);
            if let Some(slot) = work.list.get_mut(i) {
                if first != e.first {
                    slot.first_stored = first.stored(direction, Position::First);
                }
                if second != e.second {
                    slot.second_stored = second.stored(direction, Position::Second);
                }
                slot.first = first.clone();
                slot.second = second.clone();
            }
            report.lifted.push(Lift { run, from: e.pair(), to: (first, second) });
            changed = true;
        }
        report.kept = kept;
        if !changed {
            break;
        }
        report.runs = run;
    }
    report.pairs_after = work.list.iter().filter(|e| e.alive).count();
    if report.changed() {
        write_back(font, direction, master, &work.list);
    }
    Ok(report)
}

fn decrement(map: &mut HashMap<(String, String), usize>, key: (String, String)) {
    if let Some(n) = map.get_mut(&key) {
        *n = n.saturating_sub(1);
    }
}

fn slot_mut(groups: &mut KernGroups, side: GroupSide) -> &mut Option<String> {
    match side {
        GroupSide::Left => &mut groups.left,
        GroupSide::Right => &mut groups.right,
        GroupSide::Top => &mut groups.top,
        GroupSide::Bottom => &mut groups.bottom,
    }
}

/// The stored side with its group renamed, keeping the stored prefix.
fn renamed_side(stored: &str, from: &str, to: &str, direction: Direction, position: Position) -> String {
    match stored.strip_suffix(from) {
        Some(prefix) if side_group(stored) == Some(from) => format!("{prefix}{to}"),
        _ => Side::Group(to.to_string()).stored(direction, position),
    }
}

fn column(position: Position) -> &'static str {
    match position {
        Position::First => "left",
        Position::Second => "right",
    }
}

/// Renames the group `from` to `to` on the glyph side that faces the pair at `position` in
/// `direction`. Groups of the other side are separate and never change, even with the same name.
///
/// With [`RenameScope::Font`] every glyph in the group moves to the new name, and every pair in
/// every master and direction that names the group on that glyph side follows; `to` may not be a
/// group of that side already. With [`RenameScope::Pair`] only that pair's side changes and the
/// glyphs keep their group.
pub fn rename_group(
    font: &mut Font,
    direction: Direction,
    position: Position,
    from: &str,
    to: &str,
    scope: &RenameScope,
) -> Result<RenameReport, MaintainError> {
    let from = from.strip_prefix('@').unwrap_or(from);
    let to = to.strip_prefix('@').unwrap_or(to);
    check_group_name(from)?;
    check_group_name(to)?;
    if from == to {
        return Err(MaintainError::SameName);
    }
    match scope {
        RenameScope::Font => rename_in_font(font, facing(direction, position), from, to),
        RenameScope::Pair { master, first, second } => rename_in_pair(font, direction, position, from, to, master, first, second),
    }
}

/// The (direction, position) places whose group sides name a group of `side`.
fn places(side: GroupSide) -> Vec<(Direction, Position)> {
    Direction::ALL.into_iter().flat_map(|d| [(d, Position::First), (d, Position::Second)]).filter(|(d, p)| facing(*d, *p) == side).collect()
}

fn names_group_at(font: &Font, side: GroupSide, name: &str) -> bool {
    places(side).into_iter().any(|(d, p)| {
        d.table(&font.kerning).0.iter().flat_map(|m| &m.firsts).any(|f| match p {
            Position::First => side_group(&f.first) == Some(name),
            Position::Second => f.seconds.iter().any(|(s, _)| side_group(s) == Some(name)),
        })
    })
}

fn rename_in_font(font: &mut Font, side: GroupSide, from: &str, to: &str) -> Result<RenameReport, MaintainError> {
    let glyphs: Vec<String> = font.glyphs.iter().filter(|g| side.of(&g.kern_groups) == Some(from)).map(|g| g.name.clone()).collect();
    if glyphs.is_empty() && !names_group_at(font, side, from) {
        return Err(MaintainError::NoGroup { side: side.name(), name: from.to_string() });
    }
    if font.glyphs.iter().any(|g| side.of(&g.kern_groups) == Some(to)) || names_group_at(font, side, to) {
        return Err(MaintainError::GroupExists { side: side.name(), name: to.to_string() });
    }
    for name in &glyphs {
        let mut g = font.glyph_mut(name).map_err(|_| MaintainError::NoGroup { side: side.name(), name: from.to_string() })?;
        *slot_mut(&mut g.kern_groups, side) = Some(to.to_string());
    }
    let mut pairs = Vec::new();
    let kerning = Arc::make_mut(&mut font.kerning);
    for (direction, position) in places(side) {
        for m in &mut direction.table_mut(kerning).0 {
            for f in &mut m.firsts {
                if position == Position::First && side_group(&f.first) == Some(from) {
                    f.first = renamed_side(&f.first, from, to, direction, position);
                    for (s, _) in &f.seconds {
                        pairs.push(NamedPair { direction, master: m.master.clone(), first: f.first.clone(), second: s.clone() });
                    }
                }
                if position == Position::Second {
                    for (s, _) in &mut f.seconds {
                        if side_group(s) == Some(from) {
                            *s = renamed_side(s, from, to, direction, position);
                            pairs.push(NamedPair { direction, master: m.master.clone(), first: f.first.clone(), second: s.clone() });
                        }
                    }
                }
            }
        }
    }
    Ok(RenameReport { glyphs, pairs })
}

#[allow(clippy::too_many_arguments)]
fn rename_in_pair(
    font: &mut Font,
    direction: Direction,
    position: Position,
    from: &str,
    to: &str,
    master: &str,
    first: &Side,
    second: &Side,
) -> Result<RenameReport, MaintainError> {
    check_editable(font, master)?;
    let named = match position {
        Position::First => first,
        Position::Second => second,
    };
    if named != &Side::Group(from.to_string()) {
        return Err(MaintainError::NotThatGroup { column: column(position), found: named.command(), name: from.to_string() });
    }
    let renamed = Side::Group(to.to_string());
    let (new_first, new_second) = match position {
        Position::First => (&renamed, second),
        Position::Second => (first, &renamed),
    };
    let found = super::stored_pair(font, direction, master, first, second)
        .map(|p| (p.first.to_string(), p.second.to_string()))
        .ok_or_else(|| MaintainError::NoPair { master: master.to_string(), first: first.command(), second: second.command() })?;
    if super::stored_pair(font, direction, master, new_first, new_second).is_some() {
        return Err(MaintainError::PairExists { master: master.to_string(), first: new_first.command(), second: new_second.command() });
    }
    let mut list = master_entries(font, direction, master);
    let mut out = None;
    if let Some(e) = list.iter_mut().find(|e| e.first_stored == found.0 && e.second_stored == found.1) {
        match position {
            Position::First => {
                e.first_stored = renamed_side(&e.first_stored, from, to, direction, position);
                e.first = renamed.clone();
            }
            Position::Second => {
                e.second_stored = renamed_side(&e.second_stored, from, to, direction, position);
                e.second = renamed.clone();
            }
        }
        out = Some(NamedPair { direction, master: master.to_string(), first: e.first_stored.clone(), second: e.second_stored.clone() });
    }
    write_back(font, direction, master, &list);
    Ok(RenameReport { glyphs: Vec::new(), pairs: out.into_iter().collect() })
}
