//! The pair that applies to two glyphs, and the index that answers it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use fontcraft_model::{Font, Glyph, Kerning, Master, side_group};

use super::sides::{GroupSide, Side, facing};
use super::{Direction, Position};
use crate::link::metrics_source;

/// The kind of stored pair that supplied a value, in lookup order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    GlyphGlyph,
    GlyphGroup,
    GroupGlyph,
    GroupGroup,
}

impl Step {
    /// `glyphGlyph`, `glyphGroup`, `groupGlyph`, `groupGroup`.
    pub fn name(self) -> &'static str {
        match self {
            Step::GlyphGlyph => "glyphGlyph",
            Step::GlyphGroup => "glyphGroup",
            Step::GroupGlyph => "groupGlyph",
            Step::GroupGroup => "groupGroup",
        }
    }

    pub(crate) fn names_glyph(self, position: Position) -> bool {
        match position {
            Position::First => matches!(self, Step::GlyphGlyph | Step::GlyphGroup),
            Position::Second => matches!(self, Step::GlyphGlyph | Step::GroupGlyph),
        }
    }
}

/// The stored pair that applies: its value, the step that found it, its sides as stored and the
/// master whose table holds it (for a linked master, the master it follows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Applied<'a> {
    pub value: f64,
    pub step: Step,
    pub first: &'a str,
    pub second: &'a str,
    pub master: &'a str,
}

/// A pair as one master's table stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StoredPair<'a> {
    pub first: &'a str,
    pub second: &'a str,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum KernError {
    #[error("the font has no master `{0}`")]
    UnknownMaster(String),
    #[error("the font has no glyph `{0}`")]
    UnknownGlyph(String),
    #[error("the kerning amount must be a finite number")]
    NotFinite,
}

/// A multiply-rotate hasher for the index's integer keys. The keys are dense positions, not
/// values taken from a file, so a keyed hash buys nothing here and costs most of a lookup.
#[derive(Default, Clone, Copy)]
struct IdHasher(u64);

const HASH_K: u64 = 0x517c_c1b7_2722_0a95;

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(HASH_K);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(HASH_K);
    }
}

type IdMap<V> = HashMap<u64, V, BuildHasherDefault<IdHasher>>;

/// Set on a side key that is a group id; glyph ids are below it.
const GROUP_BIT: u32 = 1 << 31;

fn pair_key(first: u32, second: u32) -> u64 {
    (u64::from(first) << 32) | u64::from(second)
}

struct Entry {
    value: f64,
    first: String,
    second: String,
}

struct MasterPairs {
    direction: Direction,
    master: String,
    pairs: IdMap<Entry>,
}

/// Kerning prepared for lookups: glyph and group ids, each glyph's groups, the pairs of every
/// direction and master keyed by ids, and the members of every group. Build it once per font
/// revision; it holds no reference to the font.
pub struct KernIndex {
    kerning: Arc<Kerning>,
    masters: Arc<Vec<Master>>,
    glyphs: Vec<Arc<Glyph>>,
    glyph_ids: HashMap<String, u32>,
    group_ids: HashMap<String, u32>,
    group_names: Vec<String>,
    /// Per glyph id, its group id on each [`GroupSide`] (by `GroupSide::slot`).
    glyph_groups: Vec<[Option<u32>; 4]>,
    /// Master id and the id of the master whose kerning it shows.
    sources: Vec<(String, String)>,
    tables: Vec<MasterPairs>,
    members: HashMap<(GroupSide, u32), Vec<u32>>,
}

impl KernIndex {
    pub fn new(font: &Font) -> KernIndex {
        let glyphs: Vec<Arc<Glyph>> = font.glyphs.as_arcs().to_vec();
        let mut glyph_ids = HashMap::with_capacity(glyphs.len());
        for (i, g) in glyphs.iter().enumerate() {
            if let Ok(id) = u32::try_from(i)
                && id < GROUP_BIT
            {
                glyph_ids.entry(g.name.clone()).or_insert(id);
            }
        }
        let mut group_ids: HashMap<String, u32> = HashMap::new();
        let mut group_names = Vec::new();
        let mut glyph_groups = Vec::with_capacity(glyphs.len());
        let mut members: HashMap<(GroupSide, u32), Vec<u32>> = HashMap::new();
        for (i, g) in glyphs.iter().enumerate() {
            let mut slots = [None; 4];
            for side in GroupSide::ALL {
                let Some(name) = side.of(&g.kern_groups) else { continue };
                let id = match group_ids.get(name) {
                    Some(id) => *id,
                    None => {
                        let Ok(id) = u32::try_from(group_names.len()) else { continue };
                        if id >= GROUP_BIT {
                            continue;
                        }
                        group_ids.insert(name.to_string(), id);
                        group_names.push(name.to_string());
                        id
                    }
                };
                if let Some(slot) = slots.get_mut(side.slot()) {
                    *slot = Some(id);
                }
                if let Ok(gi) = u32::try_from(i) {
                    members.entry((side, id)).or_default().push(gi);
                }
            }
            glyph_groups.push(slots);
        }
        let side_key = |stored: &str| -> Option<u32> {
            match side_group(stored) {
                Some(g) => group_ids.get(g).map(|id| id | GROUP_BIT),
                None => glyph_ids.get(stored).copied(),
            }
        };
        let mut tables: Vec<MasterPairs> = Vec::new();
        for direction in Direction::ALL {
            for m in &direction.table(&font.kerning).0 {
                let at = match tables.iter().position(|t| t.direction == direction && t.master == m.master) {
                    Some(at) => at,
                    None => {
                        tables.push(MasterPairs { direction, master: m.master.clone(), pairs: IdMap::default() });
                        tables.len().saturating_sub(1)
                    }
                };
                let Some(table) = tables.get_mut(at) else { continue };
                for f in &m.firsts {
                    // A side naming no glyph or no group of the font can never apply.
                    let Some(first) = side_key(&f.first) else { continue };
                    for (second, value) in &f.seconds {
                        let Some(second_key) = side_key(second) else { continue };
                        if !value.is_finite() {
                            continue;
                        }
                        table.pairs.entry(pair_key(first, second_key)).or_insert_with(|| Entry {
                            value: *value,
                            first: f.first.clone(),
                            second: second.clone(),
                        });
                    }
                }
            }
        }
        let sources =
            font.masters.iter().map(|m| (m.id.clone(), metrics_source(font, &m.id).map_or_else(|| m.id.clone(), |s| s.id.clone()))).collect();
        KernIndex {
            kerning: font.kerning.clone(),
            masters: font.masters.clone(),
            glyphs,
            glyph_ids,
            group_ids,
            group_names,
            glyph_groups,
            sources,
            tables,
            members,
        }
    }

    /// Whether this index still describes `font`: the same kerning and masters, and the same
    /// glyphs with the same groups (an outline edit keeps an index current).
    pub fn is_current(&self, font: &Font) -> bool {
        Arc::ptr_eq(&self.kerning, &font.kerning)
            && Arc::ptr_eq(&self.masters, &font.masters)
            && self.glyphs.len() == font.glyphs.len()
            && self.glyphs.iter().zip(font.glyphs.as_arcs()).all(|(a, b)| Arc::ptr_eq(a, b) || (a.name == b.name && a.kern_groups == b.kern_groups))
    }

    /// The pairs of `direction` that `master` shows.
    pub fn table(&self, direction: Direction, master: &str) -> Result<Table<'_>, KernError> {
        let (_, source) = self.sources.iter().find(|(m, _)| m == master).ok_or_else(|| KernError::UnknownMaster(master.to_string()))?;
        let pairs = self.tables.iter().find(|t| t.direction == direction && &t.master == source).map(|t| &t.pairs);
        Ok(Table { index: self, direction, master: source, pairs })
    }

    /// The stored pair that applies between the glyphs `first` and `second` (logical order) in
    /// `master` and `direction`; `None` when no pair applies, so the kerning is 0.
    pub fn lookup(&self, direction: Direction, master: &str, first: &str, second: &str) -> Result<Option<Applied<'_>>, KernError> {
        let table = self.table(direction, master)?;
        let f = self.glyph_id(first).ok_or_else(|| KernError::UnknownGlyph(first.to_string()))?;
        let s = self.glyph_id(second).ok_or_else(|| KernError::UnknownGlyph(second.to_string()))?;
        Ok(table.lookup(f, s))
    }

    /// A glyph's id: its position in the font's glyph order when the index was built.
    pub fn glyph_id(&self, name: &str) -> Option<usize> {
        self.glyph_ids.get(name).map(|id| *id as usize)
    }

    /// The glyph's group on `side`.
    pub fn group(&self, glyph: &str, side: GroupSide) -> Option<&str> {
        let id = self.glyph_id(glyph)?;
        self.group_at(id, side).and_then(|g| self.group_names.get(g as usize)).map(String::as_str)
    }

    fn group_at(&self, glyph: usize, side: GroupSide) -> Option<u32> {
        self.glyph_groups.get(glyph).and_then(|slots| slots.get(side.slot()).copied().flatten())
    }

    /// The glyphs whose group on `side` is `group`, in glyph order.
    pub fn members(&self, side: GroupSide, group: &str) -> impl Iterator<Item = &str> + '_ {
        self.group_ids
            .get(group)
            .and_then(|id| self.members.get(&(side, *id)))
            .into_iter()
            .flatten()
            .filter_map(|g| self.glyphs.get(*g as usize).map(|g| g.name.as_str()))
    }
}

/// The pairs of one direction as one master shows them; lookups by glyph id.
#[derive(Clone, Copy)]
pub struct Table<'a> {
    index: &'a KernIndex,
    direction: Direction,
    master: &'a str,
    pairs: Option<&'a IdMap<Entry>>,
}

impl<'a> Table<'a> {
    /// The master whose stored pairs these are.
    pub fn master(&self) -> &'a str {
        self.master
    }

    /// The stored pair that applies between the glyphs with ids `first` and `second`.
    pub fn lookup(&self, first: usize, second: usize) -> Option<Applied<'a>> {
        let pairs = self.pairs?;
        let f = u32::try_from(first).ok().filter(|id| *id < GROUP_BIT)?;
        let s = u32::try_from(second).ok().filter(|id| *id < GROUP_BIT)?;
        let fg = self.index.group_at(first, facing(self.direction, Position::First)).map(|g| g | GROUP_BIT);
        let sg = self.index.group_at(second, facing(self.direction, Position::Second)).map(|g| g | GROUP_BIT);
        let steps =
            [(Some(f), Some(s), Step::GlyphGlyph), (Some(f), sg, Step::GlyphGroup), (fg, Some(s), Step::GroupGlyph), (fg, sg, Step::GroupGroup)];
        steps.into_iter().find_map(|(a, b, step)| {
            let e = pairs.get(&pair_key(a?, b?))?;
            Some(Applied { value: e.value, step, first: &e.first, second: &e.second, master: self.master })
        })
    }
}

fn matches(stored: &str, side: &Side) -> bool {
    match side {
        Side::Glyph(n) => side_group(stored).is_none() && stored == n,
        Side::Group(n) => side_group(stored) == Some(n.as_str()),
    }
}

/// The pair with these sides in `master`'s own table, in its stored spelling: what an edit of the
/// pair overwrites or removes. Linked masters are not followed.
pub fn stored_pair<'a>(font: &'a Font, direction: Direction, master: &str, first: &Side, second: &Side) -> Option<StoredPair<'a>> {
    direction
        .table(&font.kerning)
        .0
        .iter()
        .filter(|m| m.master == master)
        .flat_map(|m| &m.firsts)
        .filter(|f| matches(&f.first, first))
        .find_map(|f| f.seconds.iter().find(|(s, _)| matches(s, second)).map(|(s, v)| StoredPair { first: &f.first, second: s, value: *v }))
}
