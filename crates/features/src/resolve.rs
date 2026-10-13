//! The kerning of one master as the kern writer takes it: the stored pairs of both directions in
//! one table, the kerning groups by pair side, and every rule resolved to its value.
//!
//! Ported from fontc (https://github.com/googlefonts/fontc), Apache-2.0, at commit
//! 4504578c3f74c2739f7e55cae133bbfe773f6d8c: `glyphs2fontir/src/source.rs` (`derive_kern_groups`,
//! `get_glyphs_with_rtl_kerning`, `kerning_at_location`, `kern_participant`) and
//! `fontbe/src/features/kern.rs` (`GatherIrKerningWork`, `build_variable_kern_adjustments` for one
//! source, `lookup_kerning_value`, `KerningFragmentWork`). fontc follows glyphsLib
//! (`builder/kerning.py`, `builder/groups.py`) and fontTools' `ufoLib/kerning.py`.
//!
//! Right-to-left pairs join the left-to-right ones in logical order. A glyph that takes part in
//! right-to-left kerning of any master, by name or through a group, is given by its left group
//! when it is the first glyph of a pair and by its right group when it is the second; every other
//! glyph the other way round.
//!
//! Joining the tables as fontc does has two effects on left-to-right kerning, which the writer
//! reports with counts: a right-to-left pair replaces a left-to-right pair of the same sides, and
//! a glyph with right-to-left kerning has its groups swapped in left-to-right pairs too.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_model::side_group;
use fontcraft_spacing::kerning::{Direction, GroupSide};
use read_fonts::collections::IntSet;
use write_fonts::tables::gpos::builders::ValueRecordBuilder;
use write_fonts::types::GlyphId16;

use crate::{Budget, FeatureError, KernRequest};

/// A kerning group by the pair side it serves: `Side1` for the first glyph of a pair, `Side2`
/// for the second.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum KernGroup {
    Side1(String),
    Side2(String),
}

impl KernGroup {
    fn flip(self) -> KernGroup {
        match self {
            KernGroup::Side1(name) => KernGroup::Side2(name),
            KernGroup::Side2(name) => KernGroup::Side1(name),
        }
    }
}

/// One side of a stored pair, by name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum IrSide {
    Glyph(String),
    Group(KernGroup),
}

type IrPair = (IrSide, IrSide);

/// One side of a rule, by glyph id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum KernSide {
    Glyph(GlyphId16),
    Group(IntSet<GlyphId16>),
}

impl KernSide {
    pub(crate) const fn empty() -> Self {
        Self::Group(IntSet::empty())
    }

    pub(crate) fn is_empty(&self) -> bool {
        matches!(self, KernSide::Group(items) if items.is_empty())
    }

    pub(crate) fn is_group(&self) -> bool {
        matches!(self, KernSide::Group(_))
    }

    /// The glyphs of this side.
    pub(crate) fn iter(&self) -> impl Iterator<Item = GlyphId16> + '_ {
        let (first, second) = match self {
            Self::Glyph(gid) => (Some(*gid), None),
            Self::Group(group) => (None, Some(group)),
        };
        first.into_iter().chain(second.into_iter().flat_map(|set| set.iter()))
    }

    /// How many glyphs the side has.
    pub(crate) fn len(&self) -> u64 {
        match self {
            Self::Glyph(_) => 1,
            Self::Group(group) => group.len(),
        }
    }
}

/// A rule with its value, by glyph ids.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KernPair {
    pub(crate) side1: KernSide,
    pub(crate) side2: KernSide,
    pub(crate) value: ValueRecordBuilder,
}

impl KernPair {
    /// A right-to-left rule moves the glyph as well as its advance.
    pub(crate) fn make_rtl_compatible(&mut self) {
        self.value.make_rtl_compatible()
    }

    /// Whether no glyph of the rule is in `glyphs`.
    pub(crate) fn glyphs_are_disjoint(&self, glyphs: &IntSet<GlyphId16>) -> bool {
        self.side1.iter().chain(self.side2.iter()).all(|gid| !glyphs.contains(gid))
    }

    /// The rule with other glyphs on its sides; a glyph side stays a glyph side.
    pub(crate) fn with_new_glyphs(&self, side1: IntSet<GlyphId16>, side2: IntSet<GlyphId16>) -> Option<Self> {
        let (side1, side2) = match (&self.side1, &self.side2) {
            (KernSide::Glyph(_), KernSide::Glyph(_)) => (self.side1.clone(), self.side2.clone()),
            (KernSide::Glyph(_), KernSide::Group(_)) => (self.side1.clone(), KernSide::Group(side2)),
            (KernSide::Group(_), KernSide::Glyph(_)) => (KernSide::Group(side1), self.side2.clone()),
            _ => (KernSide::Group(side1), KernSide::Group(side2)),
        };
        if side1.is_empty() || side2.is_empty() {
            return None;
        }
        Some(KernPair { side1, side2, value: self.value.clone() })
    }
}

/// The group a stored side names, by the pair side its prefix gives it.
fn parse_kern_group(name: &str) -> Option<KernGroup> {
    let group = side_group(name)?.to_string();
    if name.starts_with("@MMK_L_") || name.starts_with("public.kern1.") {
        Some(KernGroup::Side1(group))
    } else if name.starts_with("@MMK_R_") || name.starts_with("public.kern2.") {
        Some(KernGroup::Side2(group))
    } else {
        None
    }
}

/// A stored side as the writer reads it; `None` drops the pair: a group of the wrong side, a
/// group without members or a glyph the font does not export.
fn kern_participant(
    side: &str,
    expect: fn(&str) -> Option<KernGroup>,
    groups: &BTreeMap<KernGroup, BTreeSet<String>>,
    gids: &HashMap<&str, GlyphId16>,
) -> Option<IrSide> {
    if side_group(side).is_some() {
        let group = expect(side)?;
        return groups.contains_key(&group).then_some(IrSide::Group(group));
    }
    gids.contains_key(side).then(|| IrSide::Glyph(side.to_string()))
}

fn first_side(side: &str) -> Option<KernGroup> {
    parse_kern_group(side).filter(|g| matches!(g, KernGroup::Side1(_)))
}

fn second_side(side: &str) -> Option<KernGroup> {
    parse_kern_group(side).filter(|g| matches!(g, KernGroup::Side2(_)))
}

/// The other pair side's spelling of a stored group side: `@MMK_L_` and `@MMK_R_` swapped.
fn flip_class_side(side: &str) -> String {
    if let Some(ident) = side.strip_prefix("@MMK_L_") {
        format!("@MMK_R_{ident}")
    } else if let Some(ident) = side.strip_prefix("@MMK_R_") {
        format!("@MMK_L_{ident}")
    } else {
        side.to_string()
    }
}

/// The glyphs that take part in right-to-left kerning in any master: named in a pair, or members
/// of a group a pair names (`@MMK_L_` a right group, `@MMK_R_` a left group).
fn glyphs_with_rtl_kerning(req: &KernRequest, budget: &mut Budget) -> Result<HashSet<String>, FeatureError> {
    let mut glyphs = HashSet::new();
    let mut groups = HashSet::new();
    for master in &req.font.kerning.rtl.0 {
        for first in &master.firsts {
            budget.spend(first.seconds.len().saturating_add(1))?;
            for side in std::iter::once(first.first.as_str()).chain(first.seconds.iter().map(|(s, _)| s.as_str())) {
                match parse_kern_group(side) {
                    Some(group) => {
                        groups.insert(group);
                    }
                    None => {
                        glyphs.insert(side.to_string());
                    }
                }
            }
        }
    }
    if groups.is_empty() {
        return Ok(glyphs);
    }
    for g in req.glyphs {
        let right = req.index.group(g.name, GroupSide::Right).map(|n| KernGroup::Side1(n.to_string()));
        let left = req.index.group(g.name, GroupSide::Left).map(|n| KernGroup::Side2(n.to_string()));
        if right.is_some_and(|r| groups.contains(&r)) || left.is_some_and(|l| groups.contains(&l)) {
            glyphs.insert(g.name.to_string());
        }
    }
    Ok(glyphs)
}

/// How many glyphs with right-to-left kerning have a group that a left-to-right pair of the
/// master names (`ltr_groups`, as stored): those pairs see the glyph by its other group.
fn flipped_ltr_glyphs(req: &KernRequest, rtl_glyphs: &HashSet<String>, ltr_groups: &HashSet<KernGroup>) -> usize {
    if rtl_glyphs.is_empty() || ltr_groups.is_empty() {
        return 0;
    }
    req.glyphs
        .iter()
        .filter(|g| rtl_glyphs.contains(g.name))
        .filter(|g| {
            let right = req.index.group(g.name, GroupSide::Right).map(|n| KernGroup::Side1(n.to_string()));
            let left = req.index.group(g.name, GroupSide::Left).map(|n| KernGroup::Side2(n.to_string()));
            [right, left].into_iter().flatten().any(|group| ltr_groups.contains(&group))
        })
        .count()
}

/// The kerning groups by pair side, with their members.
fn derive_kern_groups(req: &KernRequest, rtl_glyphs: &HashSet<String>) -> BTreeMap<KernGroup, BTreeSet<String>> {
    let mut groups: BTreeMap<KernGroup, BTreeSet<String>> = BTreeMap::new();
    for g in req.glyphs {
        let right = req.index.group(g.name, GroupSide::Right).map(|n| KernGroup::Side1(n.to_string()));
        let left = req.index.group(g.name, GroupSide::Left).map(|n| KernGroup::Side2(n.to_string()));
        let (side1, side2) = if rtl_glyphs.contains(g.name) { (right.map(KernGroup::flip), left.map(KernGroup::flip)) } else { (right, left) };
        for group in [side1, side2].into_iter().flatten() {
            groups.entry(group).or_default().insert(g.name.to_string());
        }
    }
    groups
}

/// The value that applies to a pair: the pair itself, then each glyph side tried as itself and
/// as its group, then 0 (fontTools' `ufoLib.kerning.lookupKerningValue`).
fn lookup_kerning_value(
    pair: &IrPair,
    kerning: &BTreeMap<IrPair, f64>,
    side1_glyphs: &HashMap<&str, &KernGroup>,
    side2_glyphs: &HashMap<&str, &KernGroup>,
) -> f64 {
    if let Some(value) = kerning.get(pair) {
        return *value;
    }
    fn get_group_if_glyph(side: &IrSide, map: &HashMap<&str, &KernGroup>) -> Option<IrSide> {
        match side {
            IrSide::Glyph(glyph) => map.get(glyph.as_str()).map(|group| IrSide::Group((*group).clone())),
            IrSide::Group(_) => Some(side.clone()),
        }
    }
    let (first, second) = pair;
    let first_group = get_group_if_glyph(first, side1_glyphs);
    let second_group = get_group_if_glyph(second, side2_glyphs);
    let first = Some(first).filter(|side| matches!(side, IrSide::Glyph(_)));
    let second = Some(second).filter(|side| matches!(side, IrSide::Glyph(_)));
    for (first, second) in [(first.cloned(), second_group.clone()), (first_group.clone(), second.cloned()), (first_group, second_group)] {
        if let Some(pair) = first.zip(second)
            && let Some(value) = kerning.get(&pair)
        {
            return *value;
        }
    }
    0.0
}

/// Every rule of the master's kerning, by glyph ids, with its value rounded half up. Glyph–group
/// pairs become one glyph pair per member, each with the value that applies to it; group–group
/// pairs of 0 are left out; zero exceptions stay.
pub(crate) fn kern_pairs(req: &KernRequest, budget: &mut Budget, warnings: &mut Vec<String>) -> Result<Vec<KernPair>, FeatureError> {
    let mut gids: HashMap<&str, GlyphId16> = HashMap::with_capacity(req.glyphs.len());
    for (i, g) in req.glyphs.iter().enumerate() {
        let id = u16::try_from(i).map_err(|_| FeatureError::TooMany { what: "glyphs", limit: usize::from(u16::MAX) })?;
        gids.entry(g.name).or_insert(GlyphId16::new(id));
    }

    let rtl_glyphs = glyphs_with_rtl_kerning(req, budget)?;
    let groups = derive_kern_groups(req, &rtl_glyphs);

    // Each master's left-to-right pairs, then its right-to-left pairs in left-to-right spelling.
    // A static font has the one master compiled; a variable font every master, the origin first.
    let masters: Vec<&str> = match req.variable {
        Some(v) => v.masters().iter().map(|m| m.id.as_str()).collect(),
        None => vec![req.master],
    };
    let mut per_master: Vec<BTreeMap<IrPair, f64>> = Vec::with_capacity(masters.len());
    let mut has_pairs: Vec<bool> = Vec::with_capacity(masters.len());
    let mut ltr_groups: HashSet<KernGroup> = HashSet::new();
    let mut replaced = 0usize;
    for master in &masters {
        let mut kerns: BTreeMap<IrPair, f64> = BTreeMap::new();
        let mut any = false;
        for (direction, flip) in [(Direction::Ltr, false), (Direction::Rtl, true)] {
            let table = req.index.table(direction, master).map_err(|e| FeatureError::Kerning(e.to_string()))?;
            for (first, second, value) in table.pairs() {
                budget.spend(1)?;
                any = true;
                if !flip {
                    ltr_groups.extend([first, second].into_iter().filter_map(parse_kern_group));
                }
                let (first, second) = if flip { (flip_class_side(first), flip_class_side(second)) } else { (first.to_string(), second.to_string()) };
                let side1 = kern_participant(&first, first_side, &groups, &gids);
                let side2 = kern_participant(&second, second_side, &groups, &gids);
                if let (Some(side1), Some(side2)) = (side1, side2)
                    && let Some(old) = kerns.insert((side1, side2), value)
                    && flip
                    && old != value
                {
                    replaced = replaced.saturating_add(1);
                }
            }
        }
        per_master.push(kerns);
        has_pairs.push(any);
    }
    // A master without any kerning has no kerning value, as both reference compilers read it:
    // the others' values interpolate there. The origin always has one.
    let kept: Vec<usize> = (0..per_master.len()).filter(|&m| m == 0 || has_pairs.get(m).copied().unwrap_or(false)).collect();
    let subset;
    let layout = match req.variable {
        Some(v) if kept.len() < per_master.len() => {
            subset = v.subset(&kept)?;
            per_master = per_master.into_iter().enumerate().filter(|(m, _)| kept.contains(m)).map(|(_, k)| k).collect();
            Some(&subset)
        }
        other => other,
    };
    // The rules of every master; each master's value of a rule is what its own pairs give it.
    let mut kerns: BTreeSet<&IrPair> = BTreeSet::new();
    for m in &per_master {
        kerns.extend(m.keys());
    }
    if replaced > 0 {
        warnings.push(format!(
            "{replaced} right-to-left kerning pairs have the same sides as a left-to-right pair with another value: the right-to-left value is written for both"
        ));
    }
    let flipped = flipped_ltr_glyphs(req, &rtl_glyphs, &ltr_groups);
    if flipped > 0 {
        warnings.push(format!(
            "{flipped} glyphs with right-to-left kerning are also in left-to-right group kerning: their groups serve the other side of the pair there too"
        ));
    }
    if kerns.is_empty() {
        return Ok(Vec::new());
    }

    let mut side1_of: HashMap<&str, &KernGroup> = HashMap::new();
    let mut side2_of: HashMap<&str, &KernGroup> = HashMap::new();
    for (group, members) in &groups {
        let map = match group {
            KernGroup::Side1(_) => &mut side1_of,
            KernGroup::Side2(_) => &mut side2_of,
        };
        for member in members {
            map.insert(member.as_str(), group);
        }
    }

    let values_of = |pair: &IrPair| -> Vec<f64> { per_master.iter().map(|k| lookup_kerning_value(pair, k, &side1_of, &side2_of)).collect() };
    let masters_len = per_master.len();
    let mut adjustments: BTreeMap<IrPair, Vec<f64>> = BTreeMap::new();
    for key in kerns {
        if let (IrSide::Glyph(first), IrSide::Group(second_group)) = key {
            for member in groups.get(second_group).into_iter().flatten() {
                budget.spend(masters_len)?;
                let pair = (IrSide::Glyph(first.clone()), IrSide::Glyph(member.clone()));
                let values = values_of(&pair);
                adjustments.insert(pair, values);
            }
        } else {
            budget.spend(masters_len)?;
            let values = values_of(key);
            if matches!(key, (IrSide::Group(_), IrSide::Group(_))) && values.iter().all(|v| *v == 0.0) {
                continue;
            }
            adjustments.insert(key.clone(), values);
        }
        if adjustments.len() > req.limits.rules {
            return Err(FeatureError::TooMany { what: "kerning rules", limit: req.limits.rules });
        }
    }

    // Groups as glyph sets; a group whose members are all left out is left out with its pairs.
    let mut sets: BTreeMap<&KernGroup, IntSet<GlyphId16>> = BTreeMap::new();
    for (group, members) in &groups {
        let set: IntSet<GlyphId16> = members.iter().filter_map(|m| gids.get(m.as_str()).copied()).collect();
        if !set.is_empty() {
            sets.insert(group, set);
        }
    }
    let side = |side: &IrSide| -> Option<KernSide> {
        match side {
            IrSide::Glyph(name) => gids.get(name.as_str()).copied().map(KernSide::Glyph),
            IrSide::Group(group) => sets.get(group).cloned().map(KernSide::Group),
        }
    };

    // A variable font's values as the origin's plus deltas, one column per rule.
    let variable = match layout {
        Some(v) => {
            let columns: Vec<Vec<f64>> =
                (0..per_master.len()).map(|m| adjustments.values().map(|values| values.get(m).copied().unwrap_or(0.0)).collect()).collect();
            Some((v, v.resolve(&columns)?))
        }
        None => None,
    };
    let mut out = Vec::with_capacity(adjustments.len());
    let mut clamped = 0usize;
    for (i, ((first, second), values)) in adjustments.iter().enumerate() {
        let value = values.first().copied().unwrap_or(0.0);
        // Each group side is copied: its members count as work.
        let group_len = |s: &IrSide| match s {
            IrSide::Glyph(_) => 0,
            IrSide::Group(group) => sets.get(group).map_or(0, IntSet::len),
        };
        budget.spend_glyphs(group_len(first).saturating_add(group_len(second)))?;
        let (Some(side1), Some(side2)) = (side(first), side(second)) else { continue };
        let rounded = if value.is_finite() { (value + 0.5).floor() } else { 0.0 };
        let x_advance = rounded.clamp(f64::from(i16::MIN), f64::from(i16::MAX));
        if x_advance != rounded {
            clamped = clamped.saturating_add(1);
        }
        let mut record = ValueRecordBuilder::new().with_x_advance(x_advance as i16);
        if let Some(device) = variable.as_ref().and_then(|(v, resolved)| v.device(resolved.get(i)?)) {
            record = record.with_x_advance_device(device);
        }
        out.push(KernPair { side1, side2, value: record });
    }
    if clamped > 0 {
        warnings.push(format!("{clamped} kerning values do not fit the font (−32,768 to 32,767) and were written as the nearest that does"));
    }
    Ok(out)
}
