//! Kerning: command sides and stored sides, the pair that applies to two glyphs, the locks of a
//! pair, and kerning groups. Behaviour notes: `docs/spec/08-spacing-kerning.md` § 3; rows
//! `kerning.*` in `spec/inventory/08-spacing-kerning.toml`.
//!
//! # Sides ([`sides`])
//! Commands spell a side as a glyph name or `@group`, in logical order: `left` is the first glyph
//! of the pair, `right` the second (for vertical kerning: top and bottom). A group side names the
//! group on the glyph side that faces the pair ([`facing`]): in left-to-right text the first
//! glyph's right group and the second glyph's left group; in right-to-left text the first glyph's
//! left group and the second glyph's right group; in vertical text the first glyph's bottom
//! group and the second glyph's top group. [`Side::stored`] gives the stored spelling
//! (`@MMK_L_`, `@MMK_R_`, `@MMK_T_`, `@MMK_B_` by the visual position in the pair, [`prefix`]);
//! [`Side::from_stored`] reads any stored spelling, the UFO's included.
//!
//! # Lookup ([`lookup`])
//! [`KernIndex`] is built once per font revision ([`KernIndex::is_current`]). For two glyphs in
//! a master and direction it gives the stored pair that applies and the [`Step`] that supplied
//! it, in this order: glyph–glyph, glyph–group, group–glyph, group–group, else no pair (0). A
//! master linked to another (`metrics_source`) shows that master's kerning. When two stored
//! pairs name the same sides in different spellings, the first in stored order counts.
//! [`stored_pair`] finds the stored spelling of a pair in one master's own table, for edits.
//!
//! # Locks ([`mod@locks`])
//! Lock states are derived from the stored pairs, never stored. A side is open when the pair
//! that applies names that glyph itself while the glyph has a group on the facing side; a glyph
//! without that group is [`Lock::Ungrouped`] and is always written as itself. [`locked_pair`] is
//! the pair a lock state names; [`shortcut_edit`] the pair and value a kerning shortcut writes;
//! [`open_locks`] the exception that opening locks stores (at the value that applied, so nothing
//! moves); [`close_locks`] the exception that closing locks deletes, and the pair that keeps the
//! other side open when one side of a glyph–glyph exception closes.
//!
//! # Groups ([`groups`])
//! [`check_group_name`] accepts `A–Z a–z 0–9 _ . -`. [`members`] lists a group's glyphs on one
//! side in glyph order, [`group_list`] every group of a side.
//!
//! # Maintenance ([`maintain`])
//! [`clean_up`] removes the [`obsolete_pairs`] of one master and direction: pairs that name a
//! glyph the font lacks or a group without members on the side facing the pair. [`compress`]
//! lifts glyph sides to their groups, one side per pair and run, the first side first; a pair
//! whose covering group pair has the same value is absorbed, one with another value is kept as
//! an exception. [`rename_group`] renames a group on one glyph side, in one pair or in the whole
//! font; groups of the other side never change. All three refuse a linked master
//! ([`check_editable`]); a font-wide rename is not tied to a master.
//!
//! # Pair list ([`pairs`])
//! The Kerning window's list: [`pairs::stored_pairs`] in stored order, [`pairs::filter`] with
//! the window's search options ([`pairs::Filter`]), [`pairs::sort`] by a column with ties in
//! stored order. [`pairs::plan_paste`] sorts pasted pairs into new, conflicting and unchanged
//! ones; [`pairs::apply_paste`] writes them; [`pairs::delete_pairs`] removes pairs by their sides.
//!
//! The public API:
//! ```text
//! Direction::{Ltr, Rtl, Vertical}: name, from_name, table(&Kerning) -> &KernTable
//! Position::{First, Second};  GroupSide::{Left, Right, Top, Bottom}: name, of(&KernGroups)
//! facing(direction: Direction, position: Position) -> GroupSide
//! prefix(direction: Direction, position: Position) -> &'static str
//! Side::{Glyph(String), Group(String)}: parse(text: &str) -> Result<Side, SideError>
//!     command(&self) -> String;  stored(&self, Direction, Position) -> String
//!     from_stored(stored: &str) -> Side;  name(&self) -> &str;  is_group(&self) -> bool
//! KernIndex::new(font: &Font) -> KernIndex;  KernIndex::is_current(&self, font: &Font) -> bool
//! KernIndex::lookup(&self, Direction, master: &str, first: &str, second: &str) -> Result<Option<Applied<'_>>, KernError>
//! KernIndex::table(&self, Direction, master: &str) -> Result<Table<'_>, KernError>
//! KernIndex::glyph_id(&self, name: &str) -> Option<usize>;  KernIndex::group(&self, glyph: &str, GroupSide) -> Option<&str>
//! KernIndex::members(&self, GroupSide, group: &str) -> impl Iterator<Item = &str>
//! Table::lookup(&self, first: usize, second: usize) -> Option<Applied<'_>>
//! Applied { value: f64, step: Step, first: &str, second: &str, master: &str }
//! stored_pair<'a>(font: &'a Font, Direction, master: &str, first: &Side, second: &Side) -> Option<StoredPair<'a>>
//! locks(index: &KernIndex, Direction, master: &str, first: &str, second: &str) -> Result<Locks, KernError>
//! locked_pair(index: &KernIndex, Direction, first: &str, second: &str, locks: Locks) -> (Side, Side)
//! shortcut_edit(index: &KernIndex, Direction, master: &str, first: &str, second: &str, amount: f64) -> Result<Edit, KernError>
//! open_locks(index: &KernIndex, Direction, master: &str, first: &str, second: &str, which: Which) -> Result<Option<Edit>, KernError>
//! close_locks<'a>(index: &'a KernIndex, Direction, master: &str, first: &str, second: &str, which: Which) -> Result<Option<Close<'a>>, KernError>
//! Close { removed: Applied, replacement: Option<Edit> };  Table::at_step(&self, first: usize, second: usize, Step) -> Option<Applied<'_>>
//! check_group_name(name: &str) -> Result<(), GroupNameError>
//! members<'a>(font: &'a Font, side: GroupSide, group: &str) -> Vec<&'a str>
//! group_list(font: &Font, side: GroupSide) -> Vec<(&str, Vec<&str>)>
//! pairs_naming(font: &Font, glyphs: &[&str]) -> Vec<NamedPair>
//! check_editable(font: &Font, master: &str) -> Result<(), MaintainError>
//! obsolete_pairs(font: &Font, Direction, master: &str) -> Vec<Removed>
//! clean_up(font: &mut Font, Direction, master: &str) -> Result<Vec<Removed>, MaintainError>
//! compress(font: &mut Font, Direction, master: &str, runs: usize) -> Result<CompressReport, MaintainError>
//! rename_group(font: &mut Font, Direction, Position, from: &str, to: &str, &RenameScope) -> Result<RenameReport, MaintainError>
//! Pair { first: Side, second: Side, value: f64 };  Removed { pair, reason: Obsolete::{MissingGlyph, EmptyGroup}(String) }
//! CompressReport { runs, lifted: Vec<Lift { run, from, to }>, absorbed: Vec<Absorbed { run, pair, into }>, kept: Vec<Kept { pair, why: KeptWhy::{DifferentValue(Pair), MemberPairs} }>, pairs_before, pairs_after }
//! RenameScope::{Pair { master, first: Side, second: Side }, Font};  RenameReport { glyphs: Vec<String>, pairs: Vec<NamedPair> }
//! pairs::stored_pairs<'a>(font: &'a Font, Direction, master: &str) -> Vec<ListedPair<'a>>
//! pairs::filter<'a>(font: &Font, Direction, pairs: Vec<ListedPair<'a>>, &Filter) -> Vec<ListedPair<'a>>
//! pairs::sort(pairs: &mut [ListedPair], Column, descending: bool)
//! pairs::list<'a>(font: &'a Font, Direction, master: &str, &Filter, order: Option<(Column, bool)>) -> Vec<ListedPair<'a>>
//! pairs::plan_paste(font: &Font, Direction, master: &str, pairs: &[NewPair]) -> PastePlan
//! pairs::apply_paste(table: &mut KernTable, Direction, master: &str, pairs: &[NewPair], &PastePlan, overwrite: bool)
//! pairs::delete_pairs(table: &mut KernTable, master: &str, pairs: &[(Side, Side)]) -> Vec<bool>
//! ```

pub mod groups;
pub mod locks;
pub mod lookup;
pub mod maintain;
pub mod pairs;
pub mod sides;

#[cfg(test)]
mod tests;

pub use groups::{GroupNameError, MAX_GROUP_NAME_LEN, check_group_name, group_list, members};
pub use locks::{Close, Edit, Lock, Locks, Which, close_locks, locked_pair, locks, open_locks, shortcut_edit};
pub use lookup::{Applied, KernError, KernIndex, Step, StoredPair, Table, stored_pair};
pub use maintain::{
    Absorbed, CompressReport, Kept, KeptWhy, Lift, MAX_COMPRESS_RUNS, MaintainError, Obsolete, Pair, Removed, RenameReport, RenameScope,
    check_editable, clean_up, compress, obsolete_pairs, rename_group,
};
pub use sides::{GroupSide, Side, SideError, facing, prefix};

use std::collections::HashSet;

use fontcraft_model::{Font, KernTable, Kerning};

/// A kerning direction: each has its own pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    Ltr,
    Rtl,
    Vertical,
}

impl Direction {
    pub const ALL: [Direction; 3] = [Direction::Ltr, Direction::Rtl, Direction::Vertical];

    /// The name used in command parameters: `ltr`, `rtl`, `vertical`.
    pub fn name(self) -> &'static str {
        match self {
            Direction::Ltr => "ltr",
            Direction::Rtl => "rtl",
            Direction::Vertical => "vertical",
        }
    }

    pub fn from_name(name: &str) -> Option<Direction> {
        Direction::ALL.into_iter().find(|d| d.name() == name)
    }

    /// This direction's pairs.
    pub fn table(self, kerning: &Kerning) -> &KernTable {
        match self {
            Direction::Ltr => &kerning.ltr,
            Direction::Rtl => &kerning.rtl,
            Direction::Vertical => &kerning.vertical,
        }
    }

    pub fn table_mut(self, kerning: &mut Kerning) -> &mut KernTable {
        match self {
            Direction::Ltr => &mut kerning.ltr,
            Direction::Rtl => &mut kerning.rtl,
            Direction::Vertical => &mut kerning.vertical,
        }
    }
}

/// A side's place in a pair, in logical order (the `left` and `right` of a command).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Position {
    First,
    Second,
}

/// A stored pair, in the spelling of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedPair {
    pub direction: Direction,
    pub master: String,
    pub first: String,
    pub second: String,
}

/// Every stored pair, in any direction and master, that names one of `glyphs` as a glyph side.
/// Group sides are not matched. In stored order, directions in [`Direction::ALL`] order.
pub fn pairs_naming(font: &Font, glyphs: &[&str]) -> Vec<NamedPair> {
    let names: HashSet<&str> = glyphs.iter().copied().collect();
    let names_glyph = |side: &str| fontcraft_model::side_group(side).is_none() && names.contains(side);
    let mut out = Vec::new();
    for direction in Direction::ALL {
        for m in &direction.table(&font.kerning).0 {
            for f in &m.firsts {
                let first = names_glyph(&f.first);
                for (second, _) in &f.seconds {
                    if first || names_glyph(second) {
                        out.push(NamedPair { direction, master: m.master.clone(), first: f.first.clone(), second: second.clone() });
                    }
                }
            }
        }
    }
    out
}
