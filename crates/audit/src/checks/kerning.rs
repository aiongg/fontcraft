//! Kerning checks (M12.56). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The binary checks follow FontBakery 1.1.0 (Apache-2.0): `opentype/kern_table`,
//! `gpos_kerning_info` and `tabular_kerning`; the source check of groups between masters follows
//! its `designspace_has_consistent_groups`. The other source checks are our own kerning clean-up
//! conventions.
//!
//! Pair values in a compiled font are worked out as a shaper applies the `kern` feature's pair
//! lookups: lookups apply in lookup-list order and their values add up; inside one lookup the
//! first subtable that applies decides. A class subtable (format 2) applies to every pair whose
//! first glyph it covers; a glyph-pair subtable (format 1) only to the pairs it lists. Device and
//! variation deltas, contextual kerning and lookup flags are not read.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::components::Names;
use fontcraft_model::plist::Value;
use fontcraft_model::{Family, Font, Glyph, KernGroups, KernTable, Master, side_group};
use read_fonts::tables::cmap::{CmapIterLimits, CmapSubtable};
use read_fonts::tables::gpos::{PairPos, PositionSubtables, ValueRecord};
use read_fonts::tables::gsub::{SingleSubst, SubstitutionSubtables};
use read_fonts::tables::kern::SubtableKind;
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};

use crate::{BinaryCtx, Hit, Location, Params, Sink, SourceCtx, num};

/// The key a UFO source's groups file is kept under in a master's extra data.
const UFO_GROUPS: &str = "groups.plist";
/// UFO prefixes of first-side and second-side kerning groups.
const KERN1: &str = "public.kern1.";
const KERN2: &str = "public.kern2.";

/// Most (code point, glyph) mappings read from all `cmap` subtables together.
pub(crate) const MAX_CMAP_MAPPINGS: usize = 2 * 1_114_112;
/// Most `kern` subtables read.
const MAX_KERN_SUBTABLES: usize = 1_024;
/// Most pair subtables of the `kern` feature read.
const MAX_PAIR_SUBTABLES: usize = 4_096;
/// Most subtable probes the tabular check spends on one font.
const MAX_KERN_PROBES: usize = 50_000_000;
/// Most glyph names one message lists.
const MAX_NAMES_IN_MESSAGE: usize = 20;

// ---------------------------------------------------------------------------------------------
// Source checks
// ---------------------------------------------------------------------------------------------

/// A kerning table's direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Ltr,
    Rtl,
    Vertical,
}

impl Dir {
    fn name(self) -> &'static str {
        match self {
            Dir::Ltr => "left-to-right",
            Dir::Rtl => "right-to-left",
            Dir::Vertical => "vertical",
        }
    }

    /// The direction as a finding's `direction` data: 0 left-to-right, 1 right-to-left,
    /// 2 vertical, so a fix can name the table the pair is stored in.
    fn code(self) -> f64 {
        match self {
            Dir::Ltr => 0.0,
            Dir::Rtl => 1.0,
            Dir::Vertical => 2.0,
        }
    }
}

fn tables(font: &Font) -> [(Dir, &KernTable); 3] {
    [(Dir::Ltr, &font.kerning.ltr), (Dir::Rtl, &font.kerning.rtl), (Dir::Vertical, &font.kerning.vertical)]
}

/// The side of a glyph's kerning groups ([`KernGroups`]) that faces a pair.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Slot {
    Left,
    Right,
    Top,
    Bottom,
}

impl Slot {
    fn index(self) -> usize {
        match self {
            Slot::Left => 0,
            Slot::Right => 1,
            Slot::Top => 2,
            Slot::Bottom => 3,
        }
    }

    fn of(self, groups: &KernGroups) -> Option<&str> {
        match self {
            Slot::Left => groups.left.as_deref(),
            Slot::Right => groups.right.as_deref(),
            Slot::Top => groups.top.as_deref(),
            Slot::Bottom => groups.bottom.as_deref(),
        }
    }
}

/// The group side of a glyph at the first or second place of a pair. Right-to-left pairs are in
/// logical order, so their first glyph stands on the right and faces the pair with its left side.
fn facing(dir: Dir, first: bool) -> Slot {
    match (dir, first) {
        (Dir::Ltr, true) | (Dir::Rtl, false) => Slot::Right,
        (Dir::Ltr, false) | (Dir::Rtl, true) => Slot::Left,
        (Dir::Vertical, true) => Slot::Bottom,
        (Dir::Vertical, false) => Slot::Top,
    }
}

/// A pair side: a glyph, or a group by its name without the prefix.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key<'a> {
    Glyph(&'a str),
    Group(&'a str),
}

fn key(side: &str) -> Key<'_> {
    side_group(side).map_or(Key::Glyph(side), Key::Group)
}

/// The glyphs of every kerning group, per group side.
struct Members<'a> {
    slots: [HashMap<&'a str, Vec<&'a Glyph>>; 4],
}

impl<'a> Members<'a> {
    fn new(font: &'a Font) -> Self {
        let mut slots: [HashMap<&'a str, Vec<&'a Glyph>>; 4] = Default::default();
        for g in font.glyphs.as_arcs() {
            let g: &Glyph = g;
            for slot in [Slot::Left, Slot::Right, Slot::Top, Slot::Bottom] {
                if let (Some(group), Some(map)) = (slot.of(&g.kern_groups), slots.get_mut(slot.index())) {
                    map.entry(group).or_default().push(g);
                }
            }
        }
        Self { slots }
    }

    fn of(&self, slot: Slot, group: &str) -> &[&'a Glyph] {
        self.slots.get(slot.index()).and_then(|m| m.get(group)).map_or(&[], Vec::as_slice)
    }
}

fn pair_place(master: &str, first: &str, second: &str) -> Location {
    Location { master: Some(master.to_string()), pair: Some([first.to_string(), second.to_string()]), ..Location::default() }
}

/// `check.kerning.missingGlyphReference`: pairs that name a glyph the font does not have, or a
/// group no glyph belongs to on the side facing the pair (the glyphs of the group were removed).
/// These are the pairs `kerning.cleanUp` deletes.
pub(crate) fn missing_glyph_reference(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let members = Members::new(font);
    for (dir, table) in tables(font) {
        for mk in &table.0 {
            for f in &mk.firsts {
                for (second, _) in &f.seconds {
                    for (side, first) in [(f.first.as_str(), true), (second.as_str(), false)] {
                        let place = || pair_place(&mk.master, &f.first, second);
                        match key(side) {
                            Key::Glyph(name) if font.glyph(name).is_none() => sink.push(
                                Hit::new(
                                    "missing-glyph",
                                    format!(
                                        "The {} pair `{}` `{second}` of master `{}` names the glyph `{name}`, which is not in the font.",
                                        dir.name(),
                                        f.first,
                                        mk.master
                                    ),
                                )
                                .data("direction", dir.code())
                                .place(place()),
                            ),
                            Key::Group(group) if members.of(facing(dir, first), group).is_empty() => sink.push(
                                Hit::new(
                                    "empty-group",
                                    format!(
                                        "The {} pair `{}` `{second}` of master `{}` names the kerning group `{group}`, which has no glyphs on that side.",
                                        dir.name(),
                                        f.first,
                                        mk.master
                                    ),
                                )
                                .data("direction", dir.code())
                                .place(place()),
                            ),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

/// The kerning groups of a master's kept UFO groups file: group name with its prefix → members.
/// `None` when the master was not read from a UFO of its own.
fn kept_groups(master: &Master) -> Option<BTreeMap<&str, BTreeSet<&str>>> {
    let dict = master.extra.get(Family::Ufo, UFO_GROUPS)?.as_dict()?;
    let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (name, members) in &dict.entries {
        let name = name.as_str();
        if !(name.starts_with(KERN1) || name.starts_with(KERN2)) {
            continue;
        }
        let Some(list) = members.as_array() else { continue };
        out.entry(name).or_default().extend(list.items.iter().filter_map(Value::as_str));
    }
    Some(out)
}

/// The kerning groups the glyphs hold, spelled as a UFO groups file spells them.
fn model_groups(font: &Font) -> BTreeMap<String, BTreeSet<&str>> {
    let mut out: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for g in font.glyphs.as_arcs() {
        if let Some(r) = &g.kern_groups.right {
            out.entry(format!("{KERN1}{r}")).or_default().insert(g.name.as_str());
        }
        if let Some(l) = &g.kern_groups.left {
            out.entry(format!("{KERN2}{l}")).or_default().insert(g.name.as_str());
        }
    }
    out
}

/// `check.kerning.groupsDifferBetweenMasters`: a master read from its own UFO keeps a groups file
/// whose kerning groups differ from the default master's. In a source whose kerning groups live
/// on the glyphs (one set for every master) nothing can differ. When the glyphs' groups no longer
/// match the default master's file (they were edited), saving writes every master's groups from
/// the glyphs, so nothing is reported either.
pub(crate) fn groups_differ_between_masters(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if font.masters.len() < 2 {
        return;
    }
    let Ok(default) = font.default_master() else { return };
    let Some(reference) = kept_groups(default) else { return };
    // Members the reader could not place (glyphs the font lacks) are left out of the comparison
    // with the glyphs.
    let placed: BTreeMap<&str, BTreeSet<&str>> = reference
        .iter()
        .map(|(k, v)| (*k, v.iter().copied().filter(|g| font.glyph(g).is_some()).collect::<BTreeSet<&str>>()))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let from_glyphs = model_groups(font);
    let same = placed.len() == from_glyphs.len() && placed.iter().all(|(k, v)| from_glyphs.get(*k) == Some(v));
    if !same {
        return;
    }
    for master in font.masters.iter().filter(|m| m.id != default.id) {
        let Some(mine) = kept_groups(master) else { continue };
        let names: BTreeSet<&str> = reference.keys().chain(mine.keys()).copied().collect();
        for name in names {
            let place = Location { master: Some(master.id.clone()), ..Location::default() };
            let message = match (reference.get(name), mine.get(name)) {
                (Some(a), Some(b)) if a == b => continue,
                (Some(a), Some(b)) => {
                    let only_here = b.difference(a).count();
                    let only_default = a.difference(b).count();
                    format!(
                        "The kerning group `{name}` of master `{}` differs from the default master's: {only_here} glyphs only here, {only_default} only in the default master.",
                        master.name
                    )
                }
                (Some(_), None) => format!("Master `{}` has no kerning group `{name}`, which the default master has.", master.name),
                (None, Some(_)) => format!("Master `{}` has the kerning group `{name}`, which the default master does not have.", master.name),
                (None, None) => continue,
            };
            sink.push(Hit::new("groups-differ", message).place(place));
        }
    }
}

/// `check.kerning.glyphInSeveralGroups`: a UFO groups file lists a glyph in two kerning groups of
/// the same side. The glyph keeps the first; the other membership is lost on reading. A source
/// that keeps kerning groups on the glyphs cannot have this.
pub(crate) fn glyph_in_several_groups(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let mut reported: HashSet<(usize, &str, &str, &str)> = HashSet::new();
    for master in font.masters.iter() {
        let Some(dict) = master.extra.get(Family::Ufo, UFO_GROUPS).and_then(Value::as_dict) else { continue };
        let mut seen: [HashMap<&str, &str>; 2] = Default::default();
        for (name, members) in &dict.entries {
            let name = name.as_str();
            let (side, short) = match (name.strip_prefix(KERN1), name.strip_prefix(KERN2)) {
                (Some(s), _) => (0usize, s),
                (_, Some(s)) => (1usize, s),
                _ => continue,
            };
            let Some(list) = members.as_array() else { continue };
            let Some(seen) = seen.get_mut(side) else { continue };
            for member in list.items.iter().filter_map(Value::as_str) {
                let earlier = seen.get(member).copied();
                match earlier {
                    None => {
                        seen.insert(member, short);
                    }
                    Some(kept) if kept != short => {
                        if !reported.insert((side, member, kept, short)) {
                            continue;
                        }
                        let which = if side == 0 { "first-side" } else { "second-side" };
                        let place = Location { glyph: member.to_string(), master: Some(master.id.clone()), ..Location::default() };
                        sink.push(
                            Hit::new(
                                "several-groups",
                                format!("`{member}` is in two {which} kerning groups, `{kept}` and `{short}`; only `{kept}` is used."),
                            )
                            .place(place),
                        );
                    }
                    Some(_) => {}
                }
            }
        }
    }
}

/// `check.kerning.redundantPairs`: a pair whose value is the one the font gives without it. A
/// pair's fallback is the next pair that applies when it is gone, in the order glyph–glyph,
/// glyph–group, group–glyph, group–group; with no fallback the pair is redundant when it is
/// zero, otherwise when it repeats the fallback's value.
pub(crate) fn redundant_pairs(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let group_of = |name: &str, slot: Slot| font.glyph(name).and_then(|g| slot.of(&g.kern_groups).map(str::to_string));
    for (dir, table) in tables(font) {
        for mk in &table.0 {
            let mut index: HashMap<(Key<'_>, Key<'_>), f64> = HashMap::new();
            for f in &mk.firsts {
                for (second, value) in &f.seconds {
                    index.entry((key(&f.first), key(second))).or_insert(*value);
                }
            }
            for f in &mk.firsts {
                for (second, value) in &f.seconds {
                    if !value.is_finite() {
                        continue;
                    }
                    let (a, b) = (key(&f.first), key(second));
                    let ga = match a {
                        Key::Glyph(n) => group_of(n, facing(dir, true)),
                        Key::Group(_) => None,
                    };
                    let gb = match b {
                        Key::Glyph(n) => group_of(n, facing(dir, false)),
                        Key::Group(_) => None,
                    };
                    let mut candidates: Vec<(Key<'_>, Key<'_>)> = Vec::with_capacity(3);
                    match (a, b) {
                        (Key::Glyph(_), Key::Glyph(_)) => {
                            if let Some(gb) = gb.as_deref() {
                                candidates.push((a, Key::Group(gb)));
                            }
                            if let Some(ga) = ga.as_deref() {
                                candidates.push((Key::Group(ga), b));
                            }
                            if let (Some(ga), Some(gb)) = (ga.as_deref(), gb.as_deref()) {
                                candidates.push((Key::Group(ga), Key::Group(gb)));
                            }
                        }
                        (Key::Glyph(_), Key::Group(_)) => {
                            if let Some(ga) = ga.as_deref() {
                                candidates.push((Key::Group(ga), b));
                            }
                        }
                        (Key::Group(_), Key::Glyph(_)) => {
                            if let Some(gb) = gb.as_deref() {
                                candidates.push((a, Key::Group(gb)));
                            }
                        }
                        (Key::Group(_), Key::Group(_)) => {}
                    }
                    let fallback = candidates.iter().find_map(|k| index.get(k).copied());
                    let place = pair_place(&mk.master, &f.first, second);
                    let hit = match fallback {
                        None if *value == 0.0 => Hit::new(
                            "zero-pair",
                            format!("The {} pair `{}` `{second}` of master `{}` is zero and overrides nothing.", dir.name(), f.first, mk.master),
                        ),
                        Some(v) if (v - value).abs() < 1e-9 => Hit::new(
                            "repeats-group-value",
                            format!(
                                "The {} pair `{}` `{second}` of master `{}` repeats the value {} that its group pair already gives.",
                                dir.name(),
                                f.first,
                                mk.master,
                                num(*value)
                            ),
                        ),
                        _ => continue,
                    };
                    sink.push(hit.data("value", *value).data("direction", dir.code()).place(place));
                }
            }
        }
    }
}

/// `check.kerning.excessivePair`: a negative horizontal pair larger than the advance width of one
/// of its glyphs in that master. A group side stands for its narrowest glyph. Glyphs without
/// width (marks) are left out.
pub(crate) fn excessive_pair(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let members = Members::new(font);
    let width = |g: &Glyph, master: &str| g.layer(master).map(|l| l.width).filter(|w| w.is_finite() && *w > 0.0);
    for (dir, table) in tables(font) {
        if dir == Dir::Vertical {
            continue;
        }
        for mk in &table.0 {
            let mut narrowest: HashMap<(Slot, String), Option<(f64, String)>> = HashMap::new();
            let mut side_width = |side: &str, slot: Slot| -> Option<(f64, String)> {
                match key(side) {
                    Key::Glyph(n) => font.glyph(n).and_then(|g| width(g, &mk.master)).map(|w| (w, n.to_string())),
                    Key::Group(group) => narrowest
                        .entry((slot, group.to_string()))
                        .or_insert_with(|| {
                            members
                                .of(slot, group)
                                .iter()
                                .filter_map(|g| width(g, &mk.master).map(|w| (w, g.name.clone())))
                                .min_by(|x, y| x.0.total_cmp(&y.0))
                        })
                        .clone(),
                }
            };
            for f in &mk.firsts {
                for (second, value) in &f.seconds {
                    if !value.is_finite() || *value >= 0.0 {
                        continue;
                    }
                    let a = side_width(&f.first, facing(dir, true));
                    let b = side_width(second, facing(dir, false));
                    let Some((w, glyph)) = [a, b].into_iter().flatten().min_by(|x, y| x.0.total_cmp(&y.0)) else { continue };
                    if -value <= w {
                        continue;
                    }
                    sink.push(
                        Hit::new(
                            "exceeds-width",
                            format!(
                                "The {} pair `{}` `{second}` of master `{}` is {}, more than the advance width {} of `{glyph}`.",
                                dir.name(),
                                f.first,
                                mk.master,
                                num(*value),
                                num(w)
                            ),
                        )
                        .data("value", *value)
                        .data("width", w)
                        .place(pair_place(&mk.master, &f.first, second)),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Binary checks
// ---------------------------------------------------------------------------------------------

/// The name a finding gives a glyph of a compiled font: the source glyph when the font was
/// compiled in this run, else the font's own glyph name (or a made-up `gidN`).
pub(crate) fn glyph_label(ctx: &BinaryCtx<'_>, names: &Names, gid: u32) -> String {
    super::components::binary_glyph_name(ctx, names, gid)
}

/// Some names from a list for a message, with the count of the rest.
fn name_list(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(MAX_NAMES_IN_MESSAGE).map(String::as_str).collect();
    let mut out = shown.join(", ");
    if names.len() > MAX_NAMES_IN_MESSAGE {
        out.push_str(&format!(" and {} more", names.len() - MAX_NAMES_IN_MESSAGE));
    }
    out
}

/// (code point, glyph id) mappings of the font's `cmap`: of every subtable, or of the one a
/// shaper uses. At most [`MAX_CMAP_MAPPINGS`]; unmapped entries (glyph 0) are left out.
pub(crate) fn cmap_mappings(font: &FontRef<'_>, best_only: bool) -> Vec<(u32, u32)> {
    let Ok(cmap) = font.cmap() else { return Vec::new() };
    let limits = CmapIterLimits::default_for_font(font);
    let subtables: Vec<CmapSubtable<'_>> = if best_only {
        cmap.best_subtable().map(|(_, _, s)| s).into_iter().collect()
    } else {
        cmap.encoding_records().iter().filter_map(|r| r.subtable(cmap.offset_data()).ok()).collect()
    };
    let mut out = Vec::new();
    for subtable in subtables {
        if let CmapSubtable::Format0(t) = &subtable {
            for c in 0u32..256 {
                if let Some(g) = t.map_codepoint(c).filter(|g| g.to_u32() != 0) {
                    out.push((c, g.to_u32()));
                }
            }
            continue;
        }
        for (c, g) in subtable.iter_with_limits(limits) {
            if out.len() >= MAX_CMAP_MAPPINGS {
                return out;
            }
            if g.to_u32() != 0 {
                out.push((c, g.to_u32()));
            }
        }
    }
    out
}

fn table_place(tag: &str) -> Location {
    Location { table: Some(tag.to_string()), ..Location::default() }
}

/// `check.kerning.kernTable`: a legacy `kern` table that some programs cannot use: no format 0
/// subtable, or format 0 pairs with glyphs no `cmap` subtable maps.
pub(crate) fn kern_table(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if font.table_data(Tag::new(b"kern")).is_none() {
        return;
    }
    let kern = match font.kern() {
        Ok(k) => k,
        Err(e) => {
            sink.push(Hit::new("kern-unreadable", format!("The kern table cannot be read: {e}.")).place(table_place("kern")));
            return;
        }
    };
    let encoded: HashSet<u32> = cmap_mappings(font, false).into_iter().map(|(_, g)| g).collect();
    let mut format0 = false;
    let mut stray: BTreeSet<u32> = BTreeSet::new();
    for subtable in kern.subtables().take(MAX_KERN_SUBTABLES) {
        let Ok(subtable) = subtable else { break };
        let Ok(SubtableKind::Format0(t)) = subtable.kind() else { continue };
        format0 = true;
        for p in t.pairs() {
            for g in [p.left().to_u32(), p.right().to_u32()] {
                if !encoded.contains(&g) {
                    stray.insert(g);
                }
            }
        }
    }
    if !format0 {
        sink.push(
            Hit::new("kern-unknown-format", "The kern table has no format 0 subtable, which the programs that read this table need.")
                .place(table_place("kern")),
        );
    } else if !stray.is_empty() {
        let names = Names::new(font);
        let listed: Vec<String> = stray.iter().map(|g| glyph_label(ctx, &names, *g)).collect();
        sink.push(
            Hit::new(
                "kern-non-character-glyphs",
                format!(
                    "The kern table kerns glyphs that the cmap table does not map, which the programs that read this table reject: {}.",
                    name_list(&listed)
                ),
            )
            .data("glyphs", stray.len() as f64)
            .place(table_place("kern")),
        );
    } else {
        sink.push(Hit::new("kern-found", "The font has a usable kern table; only a few programs read it.").place(table_place("kern")).without_fix());
    }
}

/// `check.kerning.gposKerningMissing`: a font that is not marked fixed-pitch has no pair
/// adjustment lookup in GPOS (directly or through an extension lookup).
pub(crate) fn gpos_kerning_missing(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if font.post().is_ok_and(|p| p.is_fixed_pitch() != 0) {
        return;
    }
    let has_pairs = font.gpos().ok().and_then(|g| g.lookup_list().ok()).is_some_and(|list| {
        list.lookups().iter().take(usize::from(u16::MAX)).any(|l| matches!(l.and_then(|l| l.subtables()), Ok(PositionSubtables::Pair(_))))
    });
    if !has_pairs {
        sink.push(Hit::new("lacks-kern-info", "GPOS has no pair kerning.").place(table_place("GPOS")));
    }
}

/// Code points whose glyphs take part in kerning on purpose: Arabic number signs, the fraction
/// slash and the division slash.
const TABULAR_EXCLUDE: [u32; 12] = [0x0600, 0x0601, 0x0602, 0x0603, 0x0604, 0x06DD, 0x0890, 0x0891, 0x0605, 0x08E2, 0x2044, 0x2215];

/// The pair subtables of the GPOS `kern` feature, per lookup in lookup-list order.
fn kern_pair_lookups<'a>(font: &FontRef<'a>) -> Vec<Vec<PairPos<'a>>> {
    let Ok(gpos) = font.gpos() else { return Vec::new() };
    let (Ok(features), Ok(list)) = (gpos.feature_list(), gpos.lookup_list()) else { return Vec::new() };
    let mut indices: BTreeSet<u16> = BTreeSet::new();
    for rec in features.feature_records() {
        if rec.feature_tag() != Tag::new(b"kern") {
            continue;
        }
        if let Ok(feature) = rec.feature(features.offset_data()) {
            indices.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
        }
    }
    let mut out = Vec::new();
    let mut total = 0usize;
    for i in indices {
        let Ok(lookup) = list.lookups().get(usize::from(i)) else { continue };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        let subs: Vec<PairPos<'a>> = subtables.iter().filter_map(Result::ok).take(MAX_PAIR_SUBTABLES.saturating_sub(total)).collect();
        total += subs.len();
        out.push(subs);
    }
    out
}

fn advance(v: &ValueRecord<'_>) -> i32 {
    v.x_advance().map_or(0, i32::from)
}

/// The value one pair subtable gives `(a, b)`, or `None` when it does not apply.
fn pair_value(sub: &PairPos<'_>, a: u32, b: u32) -> Option<i32> {
    match sub {
        PairPos::Format1(t) => {
            let index = t.coverage().ok()?.get(GlyphId::new(a))?;
            let set = t.pair_sets().get(usize::from(index)).ok()?;
            let records = set.pair_value_records();
            let (mut lo, mut hi) = (0usize, records.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let r = records.get(mid).ok()?;
                match r.second_glyph().to_u32().cmp(&b) {
                    std::cmp::Ordering::Less => lo = mid + 1,
                    std::cmp::Ordering::Greater => hi = mid,
                    std::cmp::Ordering::Equal => return Some(advance(r.value_record1()) + advance(r.value_record2())),
                }
            }
            None
        }
        PairPos::Format2(t) => {
            t.coverage().ok()?.get(GlyphId::new(a))?;
            let c1 = t.class_def1().map_or(0, |cd| cd.get(GlyphId::new(a)));
            let c2 = t.class_def2().map_or(0, |cd| cd.get(GlyphId::new(b)));
            let record = t.class1_records().get(usize::from(c1)).ok()?;
            let values = record.class2_records().get(usize::from(c2)).ok()?;
            Some(advance(values.value_record1()) + advance(values.value_record2()))
        }
    }
}

/// The kerning a shaper gives `(a, b)`, counting subtable probes against `probes`. `None` when
/// the probe budget is spent.
fn kern_value(lookups: &[Vec<PairPos<'_>>], a: u32, b: u32, probes: &mut usize) -> Option<i32> {
    let mut total = 0i32;
    for lookup in lookups {
        for sub in lookup {
            *probes = probes.checked_sub(1)?;
            if let Some(v) = pair_value(sub, a, b) {
                total = total.saturating_add(v);
                break;
            }
        }
    }
    Some(total)
}

/// Single substitutions of the GSUB `tnum` feature, or `None` when the font has no such feature.
fn tnum_substitutions(font: &FontRef<'_>) -> Option<BTreeMap<u32, u32>> {
    let gsub = font.gsub().ok()?;
    let features = gsub.feature_list().ok()?;
    let mut indices: BTreeSet<u16> = BTreeSet::new();
    let mut found = false;
    for rec in features.feature_records() {
        if rec.feature_tag() != Tag::new(b"tnum") {
            continue;
        }
        found = true;
        if let Ok(feature) = rec.feature(features.offset_data()) {
            indices.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
        }
    }
    if !found {
        return None;
    }
    let mut map = BTreeMap::new();
    let Ok(list) = gsub.lookup_list() else { return Some(map) };
    for i in indices {
        let Ok(lookup) = list.lookups().get(usize::from(i)) else { continue };
        let Ok(SubstitutionSubtables::Single(subtables)) = lookup.subtables() else { continue };
        for sub in subtables.iter().filter_map(Result::ok).take(MAX_PAIR_SUBTABLES) {
            match sub {
                SingleSubst::Format1(t) => {
                    let Ok(coverage) = t.coverage() else { continue };
                    let delta = i32::from(t.delta_glyph_id());
                    for g in coverage.iter() {
                        let out = (i32::from(g.to_u16()) + delta).rem_euclid(65_536);
                        map.entry(g.to_u32()).or_insert(out as u32);
                    }
                }
                SingleSubst::Format2(t) => {
                    let Ok(coverage) = t.coverage() else { continue };
                    for (g, s) in coverage.iter().zip(t.substitute_glyph_ids()) {
                        map.entry(g.to_u32()).or_insert(s.get().to_u32());
                    }
                }
            }
        }
    }
    Some(map)
}

/// `check.kerning.tabularKerning`: the `kern` feature kerns a tabular figure against another
/// glyph. With a `tnum` feature the tabular figures are the default figures after its
/// substitutions, and every glyph it produces is checked against them; without one, the default
/// figures are tabular when they share one advance width, and every glyph except marks is checked
/// against them.
pub(crate) fn tabular_kerning(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let mappings = cmap_mappings(font, true);
    let by_code: HashMap<u32, u32> = mappings.iter().map(|(c, g)| (*c, *g)).collect();
    let mut figures = Vec::with_capacity(10);
    for c in 0x30u32..=0x39 {
        let Some(g) = by_code.get(&c) else {
            sink.skip("the font has no glyphs for the figures 0 to 9");
            return;
        };
        figures.push(*g);
    }
    let lookups = kern_pair_lookups(font);
    if lookups.iter().all(Vec::is_empty) {
        return;
    }
    let glyph_count = font.maxp().map_or(0, |m| u32::from(m.num_glyphs()));
    let names = Names::new(font);
    let marks: HashSet<u32> = font
        .gdef()
        .ok()
        .and_then(|g| g.glyph_class_def())
        .and_then(Result::ok)
        .map(|cd| cd.iter().filter(|(_, class)| *class == 3).map(|(g, _)| g.to_u32()).collect())
        .unwrap_or_default();
    let (numerals, tabular_glyphs): (Vec<u32>, HashSet<u32>) = match tnum_substitutions(font) {
        Some(map) => (figures.iter().map(|g| map.get(g).copied().unwrap_or(*g)).collect(), map.values().copied().collect()),
        None => {
            let Ok(hmtx) = font.hmtx() else {
                sink.skip("the font has no hmtx table");
                return;
            };
            let widths: HashSet<Option<u16>> = figures.iter().map(|g| hmtx.advance(GlyphId::new(*g))).collect();
            if widths.len() != 1 {
                sink.skip("the font has no tabular figures");
                return;
            }
            (figures.clone(), HashSet::new())
        }
    };
    let excluded: HashSet<u32> = mappings.iter().filter(|(c, _)| TABULAR_EXCLUDE.contains(c)).map(|(_, g)| *g).collect();
    let numeral_set: HashSet<u32> = numerals.iter().copied().collect();
    let partner = |g: u32| -> bool {
        g != 0
            && !excluded.contains(&g)
            && (!marks.contains(&g) || tabular_glyphs.contains(&g) || numeral_set.contains(&g))
            && names.get(g).is_none_or(|n| n != "NULL")
    };
    let mut probes = MAX_KERN_PROBES;
    let mut done: HashSet<u32> = HashSet::new();
    for &n in &numerals {
        if !partner(n) || !done.insert(n) {
            continue;
        }
        for other in (1..glyph_count).filter(|g| partner(*g)) {
            // A pair of two figures is met from both sides: take it from its first figure's.
            let orders: &[(u32, u32)] = if numeral_set.contains(&other) { &[(n, other)] } else { &[(n, other), (other, n)] };
            for &(a, b) in orders {
                let Some(value) = kern_value(&lookups, a, b, &mut probes) else {
                    sink.skip("the font's kerning is too large to check");
                    return;
                };
                if value == 0 {
                    continue;
                }
                let (la, lb) = (glyph_label(ctx, &names, a), glyph_label(ctx, &names, b));
                let place = Location {
                    table: Some("GPOS".into()),
                    feature: Some("kern".into()),
                    pair: Some([la.clone(), lb.clone()]),
                    ..Location::default()
                };
                sink.push(
                    Hit::new("has-tabular-kerning", format!("`{la}` `{lb}` is kerned by {value}; tabular figures must not be kerned."))
                        .data("value", f64::from(value))
                        .place(place),
                );
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_font {
    //! Small compiled fonts built byte by byte, for the binary checks' tests.

    /// Big-endian bytes.
    #[derive(Default)]
    pub(crate) struct W(pub Vec<u8>);

    impl W {
        pub(crate) fn u16(&mut self, v: u16) -> &mut Self {
            self.0.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub(crate) fn i16(&mut self, v: i16) -> &mut Self {
            self.0.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub(crate) fn u32(&mut self, v: u32) -> &mut Self {
            self.0.extend_from_slice(&v.to_be_bytes());
            self
        }
        pub(crate) fn tag(&mut self, t: &[u8; 4]) -> &mut Self {
            self.0.extend_from_slice(t);
            self
        }
        pub(crate) fn bytes(&mut self, b: &[u8]) -> &mut Self {
            self.0.extend_from_slice(b);
            self
        }
    }

    /// An sfnt with these tables (checksums are not computed; nothing here reads them).
    pub(crate) fn sfnt(mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
        tables.sort_by_key(|(t, _)| **t);
        let n = tables.len() as u16;
        let mut w = W::default();
        w.u32(0x0001_0000).u16(n).u16(0).u16(0).u16(0);
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            w.tag(tag).u32(0).u32(offset as u32).u32(data.len() as u32);
            let mut padded = data.clone();
            while padded.len() % 4 != 0 {
                padded.push(0);
            }
            offset += padded.len();
            body.extend(padded);
        }
        w.bytes(&body);
        w.0
    }

    /// `maxp` version 0.5.
    pub(crate) fn maxp(glyphs: u16) -> Vec<u8> {
        let mut w = W::default();
        w.u32(0x0000_5000).u16(glyphs);
        w.0
    }

    /// `post` version 3 (no glyph names).
    pub(crate) fn post(fixed_pitch: bool) -> Vec<u8> {
        let mut w = W::default();
        w.u32(0x0003_0000).u32(0).i16(0).i16(0).u32(u32::from(fixed_pitch)).u32(0).u32(0).u32(0).u32(0);
        w.0
    }

    /// `cmap` with one format 12 subtable (platform 3, encoding 10) mapping each code point to its
    /// glyph, in code point order.
    pub(crate) fn cmap(map: &[(u32, u16)]) -> Vec<u8> {
        let mut w = W::default();
        w.u16(0).u16(1).u16(3).u16(10).u32(12);
        w.u16(12).u16(0).u32(16 + 12 * map.len() as u32).u32(0).u32(map.len() as u32);
        for (c, g) in map {
            w.u32(*c).u32(*c).u32(u32::from(*g));
        }
        w.0
    }

    /// `hhea` and `hmtx` with these advance widths.
    pub(crate) fn metrics(widths: &[u16]) -> (Vec<u8>, Vec<u8>) {
        let mut h = W::default();
        h.u32(0x0001_0000).i16(800).i16(-200).i16(0).u16(1000);
        for _ in 0..11 {
            h.i16(0);
        }
        h.u16(widths.len() as u16);
        let mut m = W::default();
        for w in widths {
            m.u16(*w).i16(0);
        }
        (h.0, m.0)
    }

    /// GPOS with one `kern` feature (no scripts) whose one lookup holds one format 1 pair
    /// subtable with these (first, second, x advance) pairs, sorted by first glyph.
    pub(crate) fn gpos_pairs(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut firsts: Vec<u16> = pairs.iter().map(|p| p.0).collect();
        firsts.sort_unstable();
        firsts.dedup();
        // Pair subtable: header, then pair sets, then coverage.
        let header_len = 10 + 2 * firsts.len();
        let mut sets = W::default();
        let mut set_offsets = Vec::new();
        for f in &firsts {
            set_offsets.push((header_len + sets.0.len()) as u16);
            let mut seconds: Vec<(u16, i16)> = pairs.iter().filter(|p| p.0 == *f).map(|p| (p.1, p.2)).collect();
            seconds.sort_unstable();
            sets.u16(seconds.len() as u16);
            for (s, v) in seconds {
                sets.u16(s).i16(v);
            }
        }
        let coverage_offset = (header_len + sets.0.len()) as u16;
        let mut sub = W::default();
        sub.u16(1).u16(coverage_offset).u16(0x0004).u16(0).u16(firsts.len() as u16);
        for o in set_offsets {
            sub.u16(o);
        }
        sub.bytes(&sets.0);
        sub.u16(1).u16(firsts.len() as u16);
        for f in &firsts {
            sub.u16(*f);
        }
        // Lookup list → lookup → subtable.
        let mut lookups = W::default();
        lookups.u16(1).u16(4);
        lookups.u16(2).u16(0).u16(1).u16(8);
        lookups.bytes(&sub.0);
        // Script list (empty), feature list, lookup list.
        let scripts = [0u8, 0];
        let mut features = W::default();
        features.u16(1).tag(b"kern").u16(8).u16(0).u16(1).u16(0);
        let mut w = W::default();
        let script_offset = 10u16;
        let feature_offset = script_offset + scripts.len() as u16;
        let lookup_offset = feature_offset + features.0.len() as u16;
        w.u16(1).u16(0).u16(script_offset).u16(feature_offset).u16(lookup_offset);
        w.bytes(&scripts).bytes(&features.0).bytes(&lookups.0);
        w.0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Array, Dict, Str};
    use fontcraft_model::{Font, Layer, Master};

    use super::test_font::*;
    use crate::{AuditContext, BinaryCtx, Config, Finding, NoServices, SourceCtx, audit_binary, audit_source, find};

    fn run_source(font: &Font, id: &str) -> Vec<Finding> {
        let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
        let audit = AuditContext::new(font);
        let ctx = SourceCtx { audit: &audit, services: &NoServices };
        let r = audit_source(&ctx, &[spec], &Config::default());
        assert!(r.skipped.is_empty(), "{id}: {:?}", r.skipped);
        for f in &r.findings {
            assert!(spec.codes.contains(&f.code), "{id} emits unlisted `{}`", f.code);
        }
        r.findings
    }

    fn run_binary(bytes: &[u8], id: &str) -> crate::LayerReport {
        let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
        let ctx = BinaryCtx::new("f0", bytes, 0, &NoServices).unwrap();
        let r = audit_binary(&ctx, &[spec], &Config::default());
        for f in &r.findings {
            assert!(spec.codes.contains(&f.code), "{id} emits unlisted `{}`", f.code);
            assert_eq!(f.location.font.as_deref(), Some("f0"));
        }
        r
    }

    fn codes(findings: &[Finding]) -> Vec<&'static str> {
        findings.iter().map(|f| f.code).collect()
    }

    /// The default font with a period (200 units) and narrower V (580) and T (560): A's right
    /// side and V's and O's left sides are in kerning groups named after them.
    fn kerned_font() -> Font {
        let mut font = Font::new("Test");
        font.add_glyph("period", vec![0x2E], 200.0).unwrap();
        for (name, width) in [("V", 580.0), ("O", 640.0), ("T", 560.0)] {
            font.glyph_mut(name).unwrap().layers[0].width = width;
        }
        font.glyph_mut("A").unwrap().kern_groups.right = Some("A".into());
        font.glyph_mut("V").unwrap().kern_groups.left = Some("V".into());
        font.glyph_mut("O").unwrap().kern_groups.left = Some("O".into());
        font
    }

    fn set(font: &mut Font, first: &str, second: &str, value: f64) {
        Arc::make_mut(&mut font.kerning).ltr.set("m01", first, second, value);
    }

    #[test]
    fn kerning_names_missing_glyphs_and_empty_groups() {
        let mut font = kerned_font();
        set(&mut font, "@MMK_L_A", "@MMK_R_V", -60.0);
        set(&mut font, "T", "O", -30.0);
        assert!(run_source(&font, "check.kerning.missingGlyphReference").is_empty(), "a clean font passes");

        set(&mut font, "T", "Wgone", -40.0);
        set(&mut font, "@MMK_L_A", "@MMK_R_nobody", -20.0);
        let found = run_source(&font, "check.kerning.missingGlyphReference");
        assert_eq!(codes(&found), ["empty-group", "missing-glyph"]);
        let missing = found.iter().find(|f| f.code == "missing-glyph").unwrap();
        assert_eq!(missing.location.pair, Some(["T".to_string(), "Wgone".to_string()]));
        assert_eq!(missing.location.master.as_deref(), Some("m01"));
        assert_eq!(missing.fix.and_then(|f| f.command), Some("kerning.cleanUp"));

        // A group on the wrong side is empty on the side that faces the pair: V's group is on its
        // left side, so `@MMK_L_V` (V's right side) names nobody.
        let mut font = kerned_font();
        set(&mut font, "@MMK_L_V", "A", -10.0);
        assert_eq!(codes(&run_source(&font, "check.kerning.missingGlyphReference")), ["empty-group"]);
    }

    #[test]
    fn redundant_pairs_are_zero_or_repeat_their_group_value() {
        let mut font = kerned_font();
        set(&mut font, "@MMK_L_A", "@MMK_R_V", -60.0);
        set(&mut font, "A", "V", -50.0); // an exception: kept
        set(&mut font, "A", "@MMK_R_O", 0.0); // zero, but no group pair to override: redundant
        set(&mut font, "T", "O", -30.0);
        assert!(
            run_source(&font, "check.kerning.redundantPairs").iter().all(|f| f
                .location
                .pair
                .as_ref()
                .is_some_and(|p| p[0] == "A" && p[1] == "@MMK_R_O"))
        );

        set(&mut font, "A", "@MMK_R_V", -60.0); // repeats the group pair
        set(&mut font, "@MMK_L_A", "O", 0.0); // zero over nothing (O's group pair does not exist)
        let found = run_source(&font, "check.kerning.redundantPairs");
        let mut got: Vec<(String, String, &str)> =
            found.iter().map(|f| (f.location.pair.clone().unwrap()[0].clone(), f.location.pair.clone().unwrap()[1].clone(), f.code)).collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("@MMK_L_A".to_string(), "O".to_string(), "zero-pair"),
                ("A".to_string(), "@MMK_R_O".to_string(), "zero-pair"),
                ("A".to_string(), "@MMK_R_V".to_string(), "repeats-group-value"),
            ]
        );

        // A zero exception that cancels a group pair is not redundant.
        let mut font = kerned_font();
        set(&mut font, "@MMK_L_A", "@MMK_R_V", -60.0);
        set(&mut font, "A", "V", 0.0);
        assert!(run_source(&font, "check.kerning.redundantPairs").is_empty());
    }

    #[test]
    fn excessive_pairs_exceed_the_narrowest_advance() {
        let mut font = kerned_font();
        set(&mut font, "T", "period", -150.0);
        set(&mut font, "@MMK_L_A", "@MMK_R_V", -100.0);
        assert!(run_source(&font, "check.kerning.excessivePair").is_empty());

        set(&mut font, "T", "period", -201.0);
        let found = run_source(&font, "check.kerning.excessivePair");
        assert_eq!(codes(&found), ["exceeds-width"]);
        assert_eq!(found[0].data.get("width"), Some(&200.0));
        assert!(found[0].message.contains("`period`"), "{}", found[0].message);

        let mut font = kerned_font();
        set(&mut font, "@MMK_L_A", "@MMK_R_V", -590.0);
        let found = run_source(&font, "check.kerning.excessivePair");
        assert_eq!(found.len(), 1, "V (580 units) is the narrowest glyph of the pair");
        assert!(found[0].message.contains("`V`"));
    }

    fn groups_file(groups: &[(&str, &[&str])]) -> Value {
        let mut d = Dict::new();
        for (name, members) in groups {
            let items = members.iter().map(|m| Value::String(Str::new(*m))).collect();
            d.insert(*name, Value::Array(Array { items, ..Array::default() }));
        }
        Value::Dict(d)
    }

    use fontcraft_model::plist::Value;

    fn two_master_font() -> Font {
        let mut font = kerned_font();
        Arc::make_mut(&mut font.masters).push(Master { id: "m02".into(), name: "Bold".into(), ..Master::default() });
        let names: Vec<String> = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
        for n in names {
            font.glyph_mut(&n).unwrap().layers.push(Layer::new("m02", 600.0));
        }
        font
    }

    #[test]
    fn kerning_groups_are_compared_with_the_default_master() {
        let reference: &[(&str, &[&str])] = &[("public.kern1.A", &["A"]), ("public.kern2.V", &["V"]), ("public.kern2.O", &["O"]), ("ss", &["A"])];
        let mut font = two_master_font();
        Arc::make_mut(&mut font.masters)[0].extra.push(fontcraft_model::Family::Ufo, "groups.plist", groups_file(reference));
        Arc::make_mut(&mut font.masters)[1].extra.push(fontcraft_model::Family::Ufo, "groups.plist", groups_file(reference));
        assert!(run_source(&font, "check.kerning.groupsDifferBetweenMasters").is_empty());

        let bold: &[(&str, &[&str])] = &[("public.kern1.A", &["A", "T"]), ("public.kern2.V", &["V"])];
        Arc::make_mut(&mut font.masters)[1].extra = Default::default();
        Arc::make_mut(&mut font.masters)[1].extra.push(fontcraft_model::Family::Ufo, "groups.plist", groups_file(bold));
        let found = run_source(&font, "check.kerning.groupsDifferBetweenMasters");
        assert_eq!(codes(&found), ["groups-differ", "groups-differ"], "kern1.A differs and kern2.O is missing; `ss` is not a kerning group");
        assert!(found.iter().all(|f| f.location.master.as_deref() == Some("m02")));

        // Groups edited on the glyphs since reading: saving rewrites every master, nothing to report.
        font.glyph_mut("T").unwrap().kern_groups.right = Some("A".into());
        assert!(run_source(&font, "check.kerning.groupsDifferBetweenMasters").is_empty());
    }

    #[test]
    fn a_glyph_in_two_groups_of_one_side_is_reported() {
        let mut font = kerned_font();
        let groups: &[(&str, &[&str])] = &[("public.kern1.A", &["A", "Aacute"]), ("public.kern1.Alt", &["Aacute"]), ("public.kern2.A", &["Aacute"])];
        Arc::make_mut(&mut font.masters)[0].extra.push(fontcraft_model::Family::Ufo, "groups.plist", groups_file(groups));
        let found = run_source(&font, "check.kerning.glyphInSeveralGroups");
        assert_eq!(codes(&found), ["several-groups"], "one first side twice; the second side is another side");
        assert_eq!(found[0].location.glyph, "Aacute");
        assert!(found[0].message.contains("`A` and `Alt`"));
    }

    /// Glyphs: .notdef, zero to nine (1–10), A (11), mark (12).
    fn figures_font(pairs: &[(u16, u16, i16)], widths: &[u16]) -> Vec<u8> {
        let mut map: Vec<(u32, u16)> = (0..10).map(|i| (0x30 + i, 1 + i as u16)).collect();
        map.push((0x41, 11));
        let (hhea, hmtx) = metrics(widths);
        sfnt(vec![(b"cmap", cmap(&map)), (b"maxp", maxp(13)), (b"post", post(false)), (b"hhea", hhea), (b"hmtx", hmtx), (b"GPOS", gpos_pairs(pairs))])
    }

    #[test]
    fn tabular_figures_must_not_be_kerned() {
        let tabular = [500u16; 13];
        let clean = figures_font(&[(11, 12, -30)], &tabular);
        let r = run_binary(&clean, "check.kerning.tabularKerning");
        assert!(r.findings.is_empty() && r.skipped.is_empty(), "{r:?}");

        let kerned = figures_font(&[(2, 11, -40), (11, 12, -30)], &tabular);
        let r = run_binary(&kerned, "check.kerning.tabularKerning");
        assert_eq!(r.findings.len(), 1, "{r:?}");
        let f = &r.findings[0];
        assert_eq!(f.code, "has-tabular-kerning");
        assert_eq!(f.data.get("value"), Some(&-40.0));
        assert_eq!(f.location.pair, Some(["gid2".to_string(), "gid11".to_string()]));

        // Proportional figures may be kerned.
        let mut proportional = tabular;
        proportional[2] = 300;
        let r = run_binary(&figures_font(&[(2, 11, -40)], &proportional), "check.kerning.tabularKerning");
        assert!(r.findings.is_empty());
        assert_eq!(r.skipped.len(), 1, "no tabular figures: skipped with the reason");
    }

    #[test]
    fn gpos_without_pair_kerning_is_reported_unless_monospaced() {
        let with = sfnt(vec![(b"maxp", maxp(13)), (b"post", post(false)), (b"GPOS", gpos_pairs(&[(1, 2, -10)]))]);
        assert!(run_binary(&with, "check.kerning.gposKerningMissing").findings.is_empty());
        let without = sfnt(vec![(b"maxp", maxp(13)), (b"post", post(false))]);
        assert_eq!(codes(&run_binary(&without, "check.kerning.gposKerningMissing").findings), ["lacks-kern-info"]);
        let mono = sfnt(vec![(b"maxp", maxp(13)), (b"post", post(true))]);
        assert!(run_binary(&mono, "check.kerning.gposKerningMissing").findings.is_empty());
    }

    /// A version 0 `kern` table with one subtable of this format and these pairs.
    fn kern(format: u8, pairs: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut w = W::default();
        w.u16(0).u16(1);
        let len = 6 + 8 + 6 * pairs.len();
        w.u16(0).u16(len as u16).u16((u16::from(format) << 8) | 1);
        w.u16(pairs.len() as u16).u16(0).u16(0).u16(0);
        for (l, r, v) in pairs {
            w.u16(*l).u16(*r).i16(*v);
        }
        w.0
    }

    #[test]
    fn legacy_kern_tables_are_graded() {
        let base = |kern_table: Vec<u8>| {
            sfnt(vec![(b"cmap", cmap(&[(0x41, 1), (0x56, 2)])), (b"maxp", maxp(4)), (b"post", post(false)), (b"kern", kern_table)])
        };
        let ok = run_binary(&base(kern(0, &[(1, 2, -50)])), "check.kerning.kernTable");
        assert_eq!(codes(&ok.findings), ["kern-found"]);
        assert_eq!(ok.findings[0].severity, crate::Severity::Info);
        assert!(ok.findings[0].fix.is_none());

        let stray = run_binary(&base(kern(0, &[(1, 3, -50)])), "check.kerning.kernTable");
        assert_eq!(codes(&stray.findings), ["kern-non-character-glyphs"]);
        assert_eq!(stray.findings[0].severity, crate::Severity::Fail);
        assert!(stray.findings[0].message.contains("gid3"), "{}", stray.findings[0].message);
        assert_eq!(stray.findings[0].location.table.as_deref(), Some("kern"));

        let other = run_binary(&base(kern(2, &[])), "check.kerning.kernTable");
        assert_eq!(codes(&other.findings), ["kern-unknown-format"]);
        assert_eq!(other.findings[0].severity, crate::Severity::Warn);

        let none = sfnt(vec![(b"maxp", maxp(4))]);
        assert!(run_binary(&none, "check.kerning.kernTable").findings.is_empty());
    }

    #[test]
    fn kerning_and_colour_checks_sit_in_their_profiles() {
        use crate::{Profile, select};
        let ids = |p, pattern| select(Some(p), &[pattern], &[]).iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(Profile::Opentype, "check.kerning.*"), ["check.kerning.kernTable"]);
        assert_eq!(ids(Profile::Universal, "check.kerning.*").len(), 6);
        assert_eq!(ids(Profile::Strict, "check.kerning.*").len(), 8);
        assert_eq!(ids(Profile::Universal, "check.color.*").len(), 2);
        assert_eq!(ids(Profile::Strict, "check.color.*").len(), 3);
    }

    #[test]
    fn hostile_kerning_tables_do_not_panic() {
        let truncated_gpos = {
            let mut g = gpos_pairs(&[(1, 2, -10)]);
            g.truncate(g.len() - 7);
            g
        };
        let fonts = [
            sfnt(vec![(b"maxp", maxp(13)), (b"kern", vec![0, 0, 0, 9, 1])]),
            sfnt(vec![(b"maxp", maxp(13)), (b"kern", kern(0, &[]).into_iter().take(9).collect())]),
            sfnt(vec![
                (b"maxp", maxp(13)),
                (b"GPOS", truncated_gpos.clone()),
                (b"cmap", cmap(&(0..10).map(|i| (0x30 + i, 1 + i as u16)).collect::<Vec<_>>())),
            ]),
            sfnt(vec![(b"maxp", maxp(0)), (b"GPOS", vec![0, 1, 0, 0, 0, 200, 0, 200, 0, 200])]),
            sfnt(vec![(b"cmap", vec![0, 0, 0, 200, 0, 3])]),
        ];
        for bytes in &fonts {
            for id in ["check.kerning.kernTable", "check.kerning.gposKerningMissing", "check.kerning.tabularKerning"] {
                let _ = run_binary(bytes, id);
            }
        }
    }
}
