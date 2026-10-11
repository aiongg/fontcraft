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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_model::side_group;
use fontcraft_spacing::kerning::{Direction, GroupSide};
use read_fonts::collections::IntSet;
use write_fonts::tables::gpos::builders::ValueRecordBuilder;
use write_fonts::types::GlyphId16;

use crate::{Budget, FeatureError, KernRequest, MAX_KERN_RULES};

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

    // The master's left-to-right pairs, then its right-to-left pairs in left-to-right spelling.
    let mut kerns: BTreeMap<IrPair, f64> = BTreeMap::new();
    for (direction, flip) in [(Direction::Ltr, false), (Direction::Rtl, true)] {
        let table = req.index.table(direction, req.master).map_err(|e| FeatureError::Kerning(e.to_string()))?;
        for (first, second, value) in table.pairs() {
            budget.spend(1)?;
            let (first, second) = if flip { (flip_class_side(first), flip_class_side(second)) } else { (first.to_string(), second.to_string()) };
            let side1 = kern_participant(&first, first_side, &groups, &gids);
            let side2 = kern_participant(&second, second_side, &groups, &gids);
            if let (Some(side1), Some(side2)) = (side1, side2) {
                kerns.insert((side1, side2), value);
            }
        }
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

    let mut adjustments: BTreeMap<IrPair, f64> = BTreeMap::new();
    for key in kerns.keys() {
        if let (IrSide::Glyph(first), IrSide::Group(second_group)) = key {
            for member in groups.get(second_group).into_iter().flatten() {
                budget.spend(1)?;
                let pair = (IrSide::Glyph(first.clone()), IrSide::Glyph(member.clone()));
                let value = lookup_kerning_value(&pair, &kerns, &side1_of, &side2_of);
                adjustments.insert(pair, value);
            }
        } else {
            budget.spend(1)?;
            let value = lookup_kerning_value(key, &kerns, &side1_of, &side2_of);
            if matches!(key, (IrSide::Group(_), IrSide::Group(_))) && value == 0.0 {
                continue;
            }
            adjustments.insert(key.clone(), value);
        }
        if adjustments.len() > MAX_KERN_RULES {
            return Err(FeatureError::TooMany { what: "kerning rules", limit: MAX_KERN_RULES });
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

    let mut out = Vec::with_capacity(adjustments.len());
    let mut clamped = 0usize;
    for ((first, second), value) in &adjustments {
        let (Some(side1), Some(side2)) = (side(first), side(second)) else { continue };
        let rounded = if value.is_finite() { (value + 0.5).floor() } else { 0.0 };
        let x_advance = rounded.clamp(f64::from(i16::MIN), f64::from(i16::MAX));
        if x_advance != rounded {
            clamped = clamped.saturating_add(1);
        }
        out.push(KernPair { side1, side2, value: ValueRecordBuilder::new().with_x_advance(x_advance as i16) });
    }
    if clamped > 0 {
        warnings.push(format!("{clamped} kerning values do not fit the font (−32,768 to 32,767) and were written as the nearest that does"));
    }
    Ok(out)
}
