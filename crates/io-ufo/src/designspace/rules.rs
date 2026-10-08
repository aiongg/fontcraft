//! Alternate layers in a designspace: separate glyphs `<glyph>.BRACKET.varAlt<nn>` in every
//! master's UFO, and `<rules>` that substitute them where the layers apply, as glyphsLib writes
//! them; and the way back.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0, see `NOTICE`): `builder/bracket_layers.py` (the
//! glyph names and their numbering, one rule per alternate layer, the master layers that stand in
//! where a master has no alternate layer for a region, component renaming, the kerning the new
//! glyphs inherit), `builder/transformations/align_alternate_layers.py` (a composite gets the
//! alternate layers of its components), `classes.py` (`GSLayer._bracket_info`) and
//! `builder/builders.py` (`to_glyphs`: bracket glyphs back into layers). And from fontTools 4
//! (MIT): `varLib/featureVars.py` (`overlayFeatureVariations`, `overlayBox`, `cleanupBox`), which
//! splits overlapping regions and merges the rules of one region.
//!
//! Writing ([`lower`]) makes a copy of the font in which the alternate layers are glyphs, so the
//! UFO writer needs to know nothing of them. Reading ([`raise`]) turns the glyphs the rules name
//! back into alternate layers of their glyphs; the designspace reader keeps the result only when
//! writing it gives the same files as writing the plain reading.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::{AxisRule, Family, Font, Glyph, GlyphTable, KernFirst, Layer, MasterKerning, Shape};
use fontcraft_plist::xml::{Element, Node as XNode};
use fontcraft_plist::{Str, Value};

use super::{MAX_ITEMS, int_or_float, number};
use crate::{Result, UfoError, ufo};

/// The GLIF lib key that names the layer a bracket glyph was made from; empty for a master
/// layer standing in (glyphsLib's "implicit" bracket layer).
pub(crate) const ORIGINAL_LAYER_NAME: &str = "com.schriftgestaltung.Glyphs._originalLayerName";
/// The font parameter that names the feature the substitutions go to.
pub(crate) const FEATURE_PARAM: &str = "Feature for Feature Variations";
/// The document lib key for that feature when it is neither `rvrn` nor `rclt` (fontTools'
/// `FEAVAR_FEATURETAG_LIB_KEY`).
pub(crate) const FEATURE_TAG_KEY: &str = "com.github.fonttools.varLib.featureVarsFeatureTag";
/// What a bracket glyph's name adds to its glyph's.
const INFIX: &str = ".BRACKET.";
/// A marker on the glyphs [`lower`] makes, so that the glyph order leaves them out.
pub(crate) const MADE: &str = "bracketGlyph";
/// Most regions the overlay may make.
pub const MAX_REGIONS: usize = 10_000;
/// Most passes of [`aligned`], which adds a pass per level of nested components.
const MAX_ALIGN_PASSES: usize = 64;

/// One `<rule>`: its condition sets and substitutions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rule {
    pub name: Option<String>,
    pub sets: Vec<Vec<Condition>>,
    pub subs: Vec<(String, String)>,
    /// Conditions written directly in the rule (designspace format 3), not in a set.
    pub legacy: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Condition {
    pub name: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// The `<rules>` element.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rules {
    pub rules: Vec<Rule>,
    /// `processing="last"`: the substitutions go to `rclt` rather than `rvrn`.
    pub last: bool,
}

fn opt_number(el: &Element, attr: &str) -> Result<Option<f64>> {
    number(el, attr)
}

fn read_conditions(parent: &Element) -> Result<Vec<Condition>> {
    let mut out = Vec::new();
    for c in parent.elements().filter(|e| e.name() == "condition") {
        if out.len() >= MAX_ITEMS {
            return Err(crate::bad(super::DOC, format!("a rule has more than {MAX_ITEMS} conditions")));
        }
        out.push(Condition { name: c.attr("name").unwrap_or("").to_string(), min: opt_number(c, "minimum")?, max: opt_number(c, "maximum")? });
    }
    Ok(out)
}

/// The document's `<rules>`, as designspaceLib reads them; `None` when it has none.
pub(crate) fn read_rules(root: &Element) -> Result<Option<Rules>> {
    let Some(el) = root.child("rules") else { return Ok(None) };
    let mut rules = Vec::new();
    for r in el.elements().filter(|e| e.name() == "rule") {
        if rules.len() >= MAX_ITEMS {
            return Err(crate::bad(super::DOC, format!("more than {MAX_ITEMS} rules")));
        }
        let mut sets = Vec::new();
        for s in r.elements().filter(|e| e.name() == "conditionset").take(MAX_ITEMS) {
            sets.push(read_conditions(s)?);
        }
        let direct = read_conditions(r)?;
        let legacy = !direct.is_empty();
        if legacy {
            sets.push(direct);
        }
        let subs = r
            .elements()
            .filter(|e| e.name() == "sub")
            .take(MAX_ITEMS)
            .map(|s| (s.attr("name").unwrap_or("").to_string(), s.attr("with").unwrap_or("").to_string()))
            .collect();
        rules.push(Rule { name: r.attr("name").map(str::to_string), sets, subs, legacy });
    }
    Ok(Some(Rules { rules, last: el.attr("processing") == Some("last") }))
}

/// The rules as designspaceLib writes them (`_addRule`).
pub(crate) fn rule_element(r: &Rule) -> Element {
    let mut el = Element::new("rule");
    if let Some(n) = &r.name {
        el.set_attr("name", n);
    }
    for set in &r.sets {
        let mut s = Element::new("conditionset");
        for c in set.iter().filter(|c| c.min.is_some() || c.max.is_some()) {
            let mut e = Element::new("condition").with_attr("name", &c.name);
            if let Some(v) = c.min {
                e.set_attr("minimum", int_or_float(v));
            }
            if let Some(v) = c.max {
                e.set_attr("maximum", int_or_float(v));
            }
            s.children.push(XNode::Element(e));
        }
        el.children.push(XNode::Element(s));
    }
    for (name, with) in &r.subs {
        el.children.push(XNode::Element(Element::new("sub").with_attr("name", name).with_attr("with", with)));
    }
    el
}

pub(crate) fn rules_element(r: &Rules) -> Element {
    let mut el = Element::new("rules");
    if r.last {
        el.set_attr("processing", "last");
    }
    for rule in &r.rules {
        el.children.push(XNode::Element(rule_element(rule)));
    }
    el
}

// ── the overlay (fontTools' `overlayFeatureVariations`) ──────────────────────────────────────

type Range = (f64, f64);

/// A box: axis name → range, in the order the keys were set; equality ignores the order, as for
/// a Python dictionary.
#[derive(Clone, Debug, Default)]
struct Region(Vec<(String, Range)>);

impl Region {
    fn get(&self, k: &str) -> Option<Range> {
        self.0.iter().find(|(n, _)| n == k).map(|(_, r)| *r)
    }

    fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(n, _)| n == k)
    }

    fn set(&mut self, k: &str, v: Range) {
        match self.0.iter_mut().find(|(n, _)| n == k) {
            Some(slot) => slot.1 = v,
            None => self.0.push((k.to_string(), v)),
        }
    }

    fn same(&self, o: &Region) -> bool {
        self.0.len() == o.0.len() && self.0.iter().all(|(k, v)| o.get(k) == Some(*v))
    }

    /// The items sorted, as `tuple(sorted(d.items()))`.
    fn sorted(&self) -> Vec<(String, Range)> {
        let mut v = self.0.clone();
        v.sort_by(item_order);
        v
    }
}

fn item_order(a: &(String, Range), b: &(String, Range)) -> std::cmp::Ordering {
    a.0.cmp(&b.0)
        .then(a.1.0.partial_cmp(&b.1.0).unwrap_or(std::cmp::Ordering::Equal))
        .then(a.1.1.partial_cmp(&b.1.1).unwrap_or(std::cmp::Ordering::Equal))
}

fn region_order(a: &Region, b: &Region) -> std::cmp::Ordering {
    let (x, y) = (a.sorted(), b.sorted());
    for (p, q) in x.iter().zip(&y) {
        let o = item_order(p, q);
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    x.len().cmp(&y.len())
}

/// Substitutions: glyph → glyph, in insertion order; equality ignores the order.
type Subst = Vec<(String, String)>;

fn same_subst(a: &Subst, b: &Subst) -> bool {
    a.len() == b.len() && a.iter().all(|p| b.contains(p))
}

/// `a.update(b)`.
fn update(a: &mut Subst, b: &Subst) {
    for (k, v) in b {
        match a.iter_mut().find(|(x, _)| x == k) {
            Some(slot) => slot.1 = v.clone(),
            None => a.push((k.clone(), v.clone())),
        }
    }
}

/// A set of indices, as the integer bit set fontTools uses for a rank.
#[derive(Clone, Debug, Default, PartialEq)]
struct Rank(Vec<u64>);

impl Rank {
    fn bit(i: usize) -> Self {
        let mut v = vec![0u64; i / 64 + 1];
        if let Some(w) = v.get_mut(i / 64) {
            *w |= 1u64 << (i % 64);
        }
        Rank(v)
    }

    fn or(&self, o: &Rank) -> Rank {
        let n = self.0.len().max(o.0.len());
        Rank((0..n).map(|i| self.0.get(i).copied().unwrap_or(0) | o.0.get(i).copied().unwrap_or(0)).collect())
    }

    fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }

    fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(i, w)| (0..64).filter(move |b| w & (1u64 << b) != 0).map(move |b| i * 64 + b))
    }
}

/// Python's `max(a, b)` and `min(a, b)`: the first one unless the second is strictly beyond it.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `overlayBox(top, bot)`: the intersection of the two boxes, if they meet, and the part of
/// `bot` outside `top` (a box that may hold more than that part, when it is no box).
fn overlay_box(top: &Region, bot: &Region) -> (Option<Region>, Option<Region>) {
    let mut inter = top.clone();
    for (k, v) in &bot.0 {
        inter.set(k, *v);
    }
    for (k, (min1, max1)) in &top.0 {
        let Some((min2, max2)) = bot.get(k) else { continue };
        let (lo, hi) = (py_max(*min1, min2), py_min(*max1, max2));
        if lo.partial_cmp(&hi) != Some(std::cmp::Ordering::Less) {
            return (None, Some(bot.clone()));
        }
        inter.set(k, (lo, hi));
    }
    let mut rem = bot.clone();
    let mut extruding = top.0.iter().any(|(k, _)| !bot.has(k));
    let mut inside = !extruding;
    for (k, (min2, max2)) in &bot.0 {
        if !top.has(k) {
            continue;
        }
        let Some((min1, max1)) = inter.get(k) else { continue };
        if min1 <= *min2 && *max2 <= max1 {
            continue;
        }
        if extruding {
            return (Some(inter), Some(bot.clone()));
        }
        extruding = true;
        inside = false;
        let cut = if min1 <= *min2 {
            (py_max(max1, *min2), *max2)
        } else if *max2 <= max1 {
            (*min2, py_min(min1, *max2))
        } else {
            return (Some(inter), Some(bot.clone()));
        };
        rem.set(k, cut);
    }
    if inside { (Some(inter), None) } else { (Some(inter), Some(rem)) }
}

/// `cleanupBox`: the box without ranges of (-1, 1).
fn cleanup(r: &Region) -> Region {
    Region(r.0.iter().filter(|(_, v)| *v != (-1.0, 1.0)).cloned().collect())
}

fn add(map: &mut Vec<(Region, Rank)>, region: Region, rank: Rank) {
    match map.iter_mut().find(|(r, _)| r.same(&region)) {
        Some((_, k)) => *k = k.or(&rank),
        None => map.push((region, rank)),
    }
}

/// `overlayFeatureVariations`: rules of the same substitutions merged, then rules of the same
/// regions, then the regions overlaid so that each part of the space where a set of rules apply
/// is one box, the most specific first.
fn overlay(subs: Vec<(Vec<Region>, Subst)>) -> Result<Vec<(Region, Vec<Subst>)>> {
    let mut merged: Vec<(Vec<Region>, Subst)> = Vec::new();
    for (regions, s) in subs {
        match merged.iter_mut().find(|(_, t)| same_subst(t, &s)) {
            Some((r, _)) => r.extend(regions),
            None => merged.push((regions, s)),
        }
    }
    let mut by_region: Vec<(Vec<Region>, Subst)> = Vec::new();
    for (regions, s) in merged.into_iter().rev() {
        let mut key: Vec<Region> = regions.iter().map(cleanup).collect();
        key.sort_by(region_order);
        match by_region.iter_mut().find(|(k, _)| k.len() == key.len() && k.iter().zip(&key).all(|(a, b)| a.same(b))) {
            Some((_, t)) => update(t, &s),
            None => by_region.push((key, s)),
        }
    }
    by_region.reverse();
    let mut map: Vec<(Region, Rank)> = vec![(Region::default(), Rank::default())];
    for (i, (regions, _)) in by_region.iter().enumerate() {
        let bit = Rank::bit(i);
        let mut next: Vec<(Region, Rank)> = vec![(Region::default(), Rank::default())];
        for (bx, rank) in &map {
            for cur in regions {
                let (inter, rem) = overlay_box(cur, bx);
                if let Some(inter) = inter {
                    add(&mut next, inter, rank.or(&bit));
                }
                if let Some(rem) = rem {
                    add(&mut next, rem, rank.clone());
                }
            }
            if next.len() > MAX_REGIONS {
                return Err(UfoError::Unsupported(format!("alternate layers that make more than {MAX_REGIONS} regions")));
            }
        }
        map = next;
    }
    map.sort_by_key(|(_, rank)| std::cmp::Reverse(rank.count()));
    Ok(map
        .into_iter()
        .filter(|(_, rank)| rank.count() > 0)
        .map(|(region, rank)| {
            let substs = rank.ones().filter_map(|i| by_region.get(i).map(|(_, s)| s.clone())).collect();
            (region, substs)
        })
        .collect())
}

// ── writing ───────────────────────────────────────────────────────────────────────────────────

/// An axis as the written designspace states it: its name and its range in design coordinates
/// (glyphsLib's `designspace_min_max`).
pub(crate) struct DsAxis {
    pub name: String,
    pub min: f64,
    pub max: f64,
}

pub(crate) fn ds_axes(font: &Font) -> Vec<DsAxis> {
    (0..font.axes.len())
        .filter_map(|i| {
            let a = font.axes.get(i)?;
            let (map, range) = super::axes::map_and_range(font, i);
            let (min, max) = if map.is_empty() {
                (range.minimum, range.maximum)
            } else {
                map.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, d)| (lo.min(*d), hi.max(*d)))
            };
            Some(DsAxis { name: a.name.clone(), min, max })
        })
        .collect()
}

/// A box by axis index: the axes where an alternate layer's rules narrow the axis, with the
/// range (`GSLayer._bracket_info`: an open end is the axis's end).
type BoxOf = Vec<(usize, Range)>;

/// A glyph's regions, in order of first appearance, each with its alternate layers (indices).
type GlyphBoxes = (usize, Vec<(BoxOf, Vec<usize>)>);

/// A glyph and master that need copies of master layers, with the rules of each.
type Problem<'a> = ((usize, &'a str), Vec<Vec<AxisRule>>);

/// A rule as [`recover`] reads it: its region by axis index and its substitutions.
type Parsed<'a> = (BoxOf, &'a [(String, String)]);

/// The rules with one condition set, each condition on an axis with both ends.
fn parse<'a>(axes: &[DsAxis], rules: &'a [Rule]) -> Vec<Parsed<'a>> {
    rules
        .iter()
        .filter_map(|r| {
            let set = r.sets.first()?;
            let region = set.iter().filter_map(|c| Some((axes.iter().position(|a| a.name == c.name)?, (c.min?, c.max?)))).collect();
            Some((region, r.subs.as_slice()))
        })
        .collect()
}

fn box_of(rules: &[AxisRule], axes: &[DsAxis]) -> BoxOf {
    axes.iter()
        .zip(rules)
        .enumerate()
        .filter_map(|(i, (a, r))| {
            let range = (r.min.unwrap_or(a.min), r.max.unwrap_or(a.max));
            (range != (a.min, a.max)).then_some((i, range))
        })
        .collect()
}

/// A number as Python's `str` writes it in a rule's name: whole numbers without a fraction.
fn py_number(v: f64) -> String {
    match fontcraft_model::axes::whole(v) {
        Some(i) => i.to_string(),
        None => format!("{v}"),
    }
}

/// Whether a layer is an alternate layer glyphsLib turns into a bracket glyph.
fn is_alternate(l: &Layer, masters: &[&str]) -> bool {
    !l.is_master_layer()
        && masters.contains(&l.master.as_str())
        && !l.info.attrs.axis_rules.is_empty()
        && !l.info.name.as_deref().is_some_and(|n| n.contains(".background"))
}

/// The masters a UFO of their own holds (not the layers of another UFO).
fn real_masters(font: &Font) -> Vec<&str> {
    font.masters.iter().filter(|m| m.extra.get(Family::Designspace, "layer").is_none()).map(|m| m.id.as_str()).collect()
}

/// The alternate layers glyphsLib adds to composites so they switch with their components
/// (`align_alternate_layers`): for each glyph, the master and rules of each copy of a master
/// layer it gets, in the order they are added.
fn aligned(font: &Font, masters: &[&str]) -> Vec<Vec<(String, Vec<AxisRule>)>> {
    let mut out: Vec<Vec<(String, Vec<AxisRule>)>> = vec![Vec::new(); font.glyphs.len()];
    // Masters in the order their layers first appear, each with its glyphs' master layers.
    let mut order: Vec<&str> = Vec::new();
    let mut alternates: HashMap<(&str, &str), Vec<Vec<AxisRule>>> = HashMap::new();
    for g in font.glyphs.iter() {
        for l in &g.layers {
            if l.is_master_layer() && masters.contains(&l.id.as_str()) {
                if !order.contains(&l.id.as_str()) {
                    order.push(l.id.as_str());
                }
            } else if !l.is_master_layer() && masters.contains(&l.master.as_str()) && !l.info.attrs.axis_rules.is_empty() {
                alternates.entry((l.master.as_str(), g.name.as_str())).or_default().push(l.info.attrs.axis_rules.clone());
            }
        }
    }
    let set_eq = |a: &[Vec<AxisRule>], b: &[Vec<AxisRule>]| a.iter().all(|x| b.contains(x)) && b.iter().all(|x| a.contains(x));
    for _ in 0..MAX_ALIGN_PASSES {
        let mut problems: Vec<Problem<'_>> = Vec::new();
        for m in &order {
            for (gi, g) in font.glyphs.iter().enumerate() {
                let Some(l) = g.layer(m) else { continue };
                let none = Vec::new();
                let mine = alternates.get(&(*m, g.name.as_str())).unwrap_or(&none);
                for s in &l.shapes {
                    let Shape::Component(c) = s else { continue };
                    let theirs = alternates.get(&(*m, c.glyph.as_str())).unwrap_or(&none);
                    if set_eq(mine, theirs) {
                        continue;
                    }
                    let needed: Vec<&Vec<AxisRule>> = theirs.iter().filter(|r| !mine.contains(r)).collect();
                    if needed.is_empty() {
                        continue;
                    }
                    let slot = match problems.iter().position(|(k, _)| *k == (gi, *m)) {
                        Some(p) => p,
                        None => {
                            problems.push(((gi, *m), Vec::new()));
                            problems.len() - 1
                        }
                    };
                    if let Some((_, list)) = problems.get_mut(slot) {
                        for r in needed {
                            if !list.contains(r) {
                                list.push(r.clone());
                            }
                        }
                    }
                }
            }
        }
        if problems.is_empty() {
            break;
        }
        for ((gi, m), mut needed) in problems {
            let key = |rules: &Vec<AxisRule>| -> Vec<(f64, f64)> {
                rules.iter().map(|r| (r.min.unwrap_or(f64::NEG_INFINITY), r.max.unwrap_or(f64::INFINITY))).collect()
            };
            needed.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
            let Some(g) = font.glyphs.as_arcs().get(gi) else { continue };
            for rules in needed {
                alternates.entry((m, g.name.as_str())).or_default().push(rules.clone());
                if let Some(list) = out.get_mut(gi) {
                    list.push((m.to_string(), rules));
                }
            }
        }
    }
    out
}

/// The font written as glyphsLib writes alternate layers: a copy in which they are glyphs of
/// their own, with the rules that substitute them.
pub(crate) struct Lowered<'a> {
    pub font: Cow<'a, Font>,
    pub rules: Rules,
    /// Whether the font had alternate layers (the feature tag then goes to the document's lib).
    pub any: bool,
    /// The feature the substitutions go to, when neither `rvrn` nor `rclt`.
    pub feature_tag: Option<String>,
    /// Each bracket glyph: its name, its glyph's and its region.
    pub boxes: Vec<(String, String, BoxOf)>,
    /// Each alternate layer of the font (not the copies align adds): its glyph, its id and the
    /// bracket glyph it goes to.
    pub named: Vec<(String, String, String)>,
}

/// One alternate layer to turn into a glyph's layer.
struct Alt<'a> {
    glyph: usize,
    layer: &'a Layer,
    master: String,
    rules: Vec<AxisRule>,
    /// A copy of the master layer that align adds (written as a stand-in, see [`lower`]).
    copy: bool,
}

/// The UFO-family details of a layer that tie it to its own file, which a copy for a bracket
/// glyph must not take.
const FILE_KEYS: [&str; 7] = ["fileName", "glifName", "unicodes", "unicodesRead", "note", "noteRead", "markColorRead"];

/// Writes the alternate layers as glyphsLib's `to_designspace_bracket_layers` does: each
/// alternate layer's region (its rules, an open end at the axis's end) gets a glyph
/// `<glyph>.BRACKET.varAlt<nn>` (numbered per glyph in order of first appearance) in every
/// master's UFO, from the master's alternate layer for that region or else its master layer;
/// components of glyphs that have a bracket glyph for the same region point at it; the new glyphs
/// join their glyph's kerning groups and pairs; and one rule per alternate layer of an exported
/// glyph, overlaid, substitutes them. Composites first get the alternate layers of their
/// components (`align_alternate_layers`).
///
/// glyphsLib names a master layer standing in with `""` in [`ORIGINAL_LAYER_NAME`]; the copies
/// align adds are written so too (glyphsLib gives them the master layer's name, which is empty in
/// format version 3), so they do not come back as layers of their own.
pub(crate) fn lower(font: &Font) -> Result<Lowered<'_>> {
    let masters = real_masters(font);
    let has_any = font.glyphs.iter().any(|g| g.layers.iter().any(|l| is_alternate(l, &masters)));
    if !has_any || font.axes.is_empty() {
        return Ok(Lowered {
            font: Cow::Borrowed(font),
            rules: Rules::default(),
            any: false,
            feature_tag: None,
            boxes: Vec::new(),
            named: Vec::new(),
        });
    }
    let axes = ds_axes(font);
    let synth = aligned(font, &masters);
    let mut alts: Vec<Alt<'_>> = Vec::new();
    for (gi, g) in font.glyphs.iter().enumerate() {
        for l in g.layers.iter().filter(|l| is_alternate(l, &masters)) {
            alts.push(Alt { glyph: gi, layer: l, master: l.master.clone(), rules: l.info.attrs.axis_rules.clone(), copy: false });
        }
        for (m, rules) in synth.get(gi).into_iter().flatten() {
            if let Some(base) = g.layer(m) {
                alts.push(Alt { glyph: gi, layer: base, master: m.clone(), rules: rules.clone(), copy: true });
            }
        }
    }
    if alts.len() > MAX_ITEMS {
        return Err(UfoError::Unsupported(format!("more than {MAX_ITEMS} alternate layers")));
    }
    // Regions by glyph, in order of first appearance, with their layers; bracket glyph names.
    let mut map: Vec<GlyphBoxes> = Vec::new();
    let mut subs: Vec<(Vec<Region>, Subst)> = Vec::new();
    for (k, alt) in alts.iter().enumerate() {
        let bx = box_of(&alt.rules, &axes);
        let at = match map.iter().position(|(gi, _)| *gi == alt.glyph) {
            Some(p) => p,
            None => {
                map.push((alt.glyph, Vec::new()));
                map.len() - 1
            }
        };
        let Some((_, boxes)) = map.get_mut(at) else { continue };
        let n = match boxes.iter().position(|(b, _)| *b == bx) {
            Some(p) => p,
            None => {
                boxes.push((bx.clone(), Vec::new()));
                boxes.len() - 1
            }
        };
        if let Some((_, list)) = boxes.get_mut(n) {
            list.push(k);
        }
        let Some(g) = font.glyphs.as_arcs().get(alt.glyph) else { continue };
        if !g.export {
            continue;
        }
        let region = Region(bx.iter().filter_map(|(i, r)| Some((axes.get(*i)?.name.clone(), *r))).collect());
        subs.push((vec![region], vec![(g.name.clone(), bracket_name(&g.name, n))]));
    }
    let mut rules = Vec::new();
    for (region, substs) in overlay(subs)? {
        let mut mapping: Subst = Vec::new();
        for s in &substs {
            update(&mut mapping, s);
        }
        let name = region.sorted().iter().map(|(n, (lo, hi))| format!("{n}_{}_{}", py_number(*lo), py_number(*hi))).collect::<Vec<_>>().join(".");
        let conditions = region.0.iter().map(|(n, (lo, hi))| Condition { name: n.clone(), min: Some(*lo), max: Some(*hi) }).collect();
        rules.push(Rule { name: Some(format!("BRACKET.{name}")), sets: vec![conditions], subs: mapping, legacy: false });
    }
    let feature = font.info.params.get(FEATURE_PARAM).and_then(Value::as_str).map(str::to_string);
    let last = feature.as_deref() == Some("rclt");
    let feature_tag = feature.filter(|f| !f.is_empty() && f != "rvrn" && f != "rclt");

    // The bracket glyphs. A component goes to its glyph's bracket glyph of the same region.
    let renamed = |glyph: &str, bx: &BoxOf| -> Option<String> {
        let gi = font.glyphs.position(glyph)?;
        let (_, boxes) = map.iter().find(|(g, _)| *g == gi)?;
        boxes.iter().position(|(b, _)| b == bx).map(|n| bracket_name(glyph, n))
    };
    let mut made: Vec<(String, String, Glyph)> = Vec::new();
    let mut boxes_out: Vec<(String, String, BoxOf)> = Vec::new();
    let mut named: Vec<(String, String, String)> = Vec::new();
    for (gi, boxes) in &map {
        let Some(g) = font.glyphs.as_arcs().get(*gi) else { continue };
        for (n, (bx, ks)) in boxes.iter().enumerate() {
            let name = bracket_name(&g.name, n);
            boxes_out.push((name.clone(), g.name.clone(), bx.clone()));
            for a in ks.iter().filter_map(|k| alts.get(*k)).filter(|a| !a.copy) {
                named.push((g.name.clone(), a.layer.id.clone(), name.clone()));
            }
            if font.glyphs.get(&name).is_some() {
                return Err(UfoError::Unsupported(format!("the glyph `{name}` exists, and an alternate layer of `{}` needs its name", g.name)));
            }
            let mut layers: Vec<Layer> = Vec::new();
            for m in &masters {
                let explicit = ks.iter().filter_map(|k| alts.get(*k)).find(|a| a.master == *m);
                let (src, original, copy) = match explicit {
                    Some(a) if a.copy => (a.layer, String::new(), true),
                    Some(a) => (a.layer, a.layer.info.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| unnamed(&a.rules)), false),
                    None => match g.layer(m) {
                        Some(l) => (l, String::new(), true),
                        None => continue,
                    },
                };
                let mut l = src.clone();
                l.id = (*m).to_string();
                l.master = (*m).to_string();
                if copy {
                    for k in FILE_KEYS {
                        l.extra.remove(Family::Ufo, k);
                    }
                }
                // glyphsLib writes the background of an alternate layer to `<layer name>.background`.
                if !copy && !original.is_empty() && l.info.background.is_some() {
                    l.extra.insert(
                        Family::Designspace,
                        ufo::BACKGROUND_LAYER,
                        Value::String(Str::new(format!("{original}{}", ufo::BACKGROUND_SUFFIX))),
                    );
                }
                {
                    let info = &mut *l.info;
                    info.name = None;
                    info.attrs.axis_rules.clear();
                    info.user_data.0.insert(ORIGINAL_LAYER_NAME, Value::String(Str::new(&original)));
                }
                l.info.tidy();
                for s in &mut l.shapes {
                    if let Shape::Component(c) = s
                        && let Some(to) = renamed(&c.glyph, bx)
                    {
                        c.glyph = to;
                    }
                }
                layers.push(l);
            }
            let mut b = Glyph::new(name.clone());
            b.export = g.export;
            b.kern_groups = g.kern_groups.clone();
            b.info = g.info.clone();
            {
                let info = &mut *b.info;
                info.user_data = Default::default();
                // A production name gets the bracket glyph's suffix too.
                if let Some(p) = &info.production
                    && !p.contains(INFIX)
                {
                    let suffix = name.get(g.name.len()..).unwrap_or_default();
                    info.production = Some(format!("{p}{suffix}"));
                }
            }
            // A font that states `GDEF` classes gets one for the new glyph by glyphsLib's rule.
            if g.info.opentype_category.is_some() {
                let origin = fontcraft_model::axes::origin(font).map(|m| m.id.as_str());
                let l = layers.iter().find(|l| Some(l.master.as_str()) == origin).or(layers.first());
                b.info.opentype_category = l.and_then(|l| opentype_category(g, l));
            }
            b.info.tidy();
            b.layers = layers;
            b.extra.push(Family::Designspace, MADE, Value::String(Str::new(&g.name)));
            made.push((g.name.clone(), name, b));
        }
    }
    let mut out = font.clone();
    // The base glyphs without their alternate layers.
    let mut glyphs: Vec<Arc<Glyph>> = Vec::with_capacity(font.glyphs.len() + made.len());
    for g in font.glyphs.iter() {
        if g.layers.iter().any(|l| is_alternate(l, &masters)) {
            let mut c = Glyph::clone(g);
            c.layers.retain(|l| !is_alternate(l, &masters));
            glyphs.push(Arc::new(c));
        } else {
            glyphs.push(Arc::clone(g));
        }
    }
    // Kerning: each bracket glyph takes its glyph's pairs (`_expand_kerning_to_brackets`).
    let mut kerning = (*font.kerning).clone();
    for mk in &mut kerning.ltr.0 {
        for (base, name, _) in &made {
            expand_kerning(mk, base, name);
        }
    }
    glyphs.extend(made.into_iter().map(|(_, _, g)| Arc::new(g)));
    out.glyphs = GlyphTable::from_glyphs(glyphs);
    out.kerning = Arc::new(kerning);
    Ok(Lowered { font: Cow::Owned(out), rules: Rules { rules, last }, any: true, feature_tag, boxes: boxes_out, named })
}

/// The `GDEF` class glyphsLib's `_build_public_opentype_categories` gives a glyph made of a
/// layer: a ligature or a base when the layer has an anchor to attach to, a mark by the glyph's
/// category.
fn opentype_category(g: &Glyph, l: &Layer) -> Option<String> {
    let attaching = l.anchors.iter().any(|a| !a.name.is_empty() && !a.name.starts_with('_'));
    let (category, sub) = (g.info.category.as_deref(), g.info.sub_category.as_deref());
    let class = if sub == Some("Ligature") && attaching {
        "ligature"
    } else if category == Some("Mark") && matches!(sub, Some("Nonspacing" | "Spacing Combining")) {
        "mark"
    } else if attaching {
        "base"
    } else {
        return None;
    };
    Some(class.to_string())
}

/// The name an alternate layer without one is written with, so that it does not read back as a
/// master layer standing in: its rules as a version 2 name says them (`[300]`, `]600]`) when it
/// can, else `[]`.
pub(crate) fn unnamed(rules: &[AxisRule]) -> String {
    match rules {
        [AxisRule { min: Some(v), max: None }] => format!("[{}]", py_number(*v)),
        [AxisRule { min: None, max: Some(v) }] => format!("]{}]", py_number(*v)),
        _ => "[]".to_string(),
    }
}

/// `<glyph>.BRACKET.varAlt<nn>` for the `n`-th region (from 0) of a glyph.
fn bracket_name(glyph: &str, n: usize) -> String {
    format!("{glyph}{INFIX}varAlt{:02}", n + 1)
}

/// Gives `name` the pairs of `base`: `(base, x)` → `(name, x)`, `(x, base)` → `(x, name)`, and
/// `(base, base)` → `(name, name)`, each set after its model in the table.
fn expand_kerning(mk: &mut MasterKerning, base: &str, name: &str) {
    let mut add: Vec<(String, String, f64)> = Vec::new();
    for f in &mk.firsts {
        for (s, v) in &f.seconds {
            match (f.first == base, s == base) {
                (true, true) => add.push((name.to_string(), name.to_string(), *v)),
                (true, false) => add.push((name.to_string(), s.clone(), *v)),
                (false, true) => add.push((f.first.clone(), name.to_string(), *v)),
                (false, false) => {}
            }
        }
    }
    // A bracket glyph's entries go after its glyph's (and after earlier bracket glyphs').
    let follows = |x: &str| x == base || x.strip_prefix(base).is_some_and(|r| r.starts_with(INFIX));
    for (first, second, v) in add {
        let at = match mk.firsts.iter().position(|f| f.first == first) {
            Some(p) => p,
            None => {
                let p = mk.firsts.iter().rposition(|f| follows(&f.first)).map_or(mk.firsts.len(), |p| p + 1);
                mk.firsts.insert(p, KernFirst { first, seconds: Vec::new() });
                p
            }
        };
        let Some(f) = mk.firsts.get_mut(at) else { continue };
        match f.seconds.iter_mut().find(|(s, _)| *s == second) {
            Some(slot) => slot.1 = v,
            None => {
                let p =
                    if second == name { f.seconds.iter().rposition(|(s, _)| follows(s)).map_or(f.seconds.len(), |p| p + 1) } else { f.seconds.len() };
                f.seconds.insert(p, (second, v));
            }
        }
    }
}

/// What a designspace changes of a font's alternate layers, each a count of layers.
#[derive(Default)]
pub(crate) struct AlternateLosses {
    /// Layers whose rules come back otherwise: a later alternate layer of the glyph overlaps
    /// them, and the rules do not tell where they ended.
    pub ranges: usize,
    /// Layers that later alternate layers of their glyph cover entirely, so that no rule names
    /// their bracket glyph: the reader then takes the document as it is written.
    pub covered: usize,
    /// Glyphs whose alternate layers come back in another place among their layers: after the
    /// others, by region and master.
    pub order: usize,
    /// Layers of glyphs that are not exported, which come back as glyphs of their own.
    pub not_exported: usize,
}

pub(crate) fn alternate_losses(font: &Font) -> AlternateLosses {
    let mut out = AlternateLosses::default();
    let Ok(lowered) = lower(font) else { return out };
    if !lowered.any {
        return out;
    }
    let axes = ds_axes(font);
    let parsed = parse(&axes, &lowered.rules.rules);
    let order: Vec<(String, String)> = lowered.boxes.iter().map(|(w, b, _)| (w.clone(), b.clone())).collect();
    let back = recover(&axes, &parsed, &order);
    let as_ranges = |bx: &BoxOf| -> Ranges { (0..axes.len()).map(|i| bx.iter().find(|(j, _)| *j == i).map(|(_, r)| *r)).collect() };
    let normal = |r: Ranges| -> Ranges { r.into_iter().zip(&axes).map(|(r, a)| r.filter(|v| *v != (a.min, a.max))).collect() };
    let exported = |name: &str| font.glyphs.get(name).is_some_and(|g| g.export);
    let mut lost: Vec<(&str, bool)> = Vec::new();
    for ((w, base, bx), r) in lowered.boxes.iter().zip(back) {
        match r {
            _ if !exported(base) => {}
            None => lost.push((w.as_str(), true)),
            Some(r) if normal(r.clone()) != as_ranges(bx) => lost.push((w.as_str(), false)),
            Some(_) => {}
        }
    }
    for (glyph, _, bracket) in &lowered.named {
        if !exported(glyph) {
            out.not_exported += 1;
        } else if let Some((_, covered)) = lost.iter().find(|(w, _)| w == bracket) {
            if *covered {
                out.covered += 1;
            } else {
                out.ranges += 1;
            }
        }
    }
    // The order the reader gives: the other layers, then the alternate layers by region and
    // master.
    let masters: Vec<&str> = font.masters.iter().map(|m| m.id.as_str()).collect();
    for g in font.glyphs.iter().filter(|g| g.export) {
        let mine: Vec<(&str, &str)> = lowered.named.iter().filter(|(n, _, _)| *n == g.name).map(|(_, id, b)| (id.as_str(), b.as_str())).collect();
        if mine.is_empty() {
            continue;
        }
        let is_alt = |l: &Layer| mine.iter().any(|(id, _)| *id == l.id);
        let first_alt = g.layers.iter().position(is_alt);
        let after = first_alt.is_some_and(|f| g.layers.iter().skip(f).any(|l| !is_alt(l)));
        let key = |l: &&Layer| {
            let b = mine.iter().find(|(id, _)| *id == l.id).map(|(_, b)| *b).unwrap_or("");
            (b.get(g.name.len() + INFIX.len()..).and_then(suffix_number), masters.iter().position(|m| *m == l.master))
        };
        let alts: Vec<&Layer> = g.layers.iter().filter(|l| is_alt(l)).collect();
        let mut sorted = alts.clone();
        sorted.sort_by_key(key);
        if after || alts.iter().zip(&sorted).any(|(a, b)| a.id != b.id) {
            out.order += 1;
        }
    }
    out
}

/// The ids a reader gives the alternate layers, made of their master and their bracket glyph's
/// name: glyph, layer id, id read back.
pub(crate) fn read_back_ids(font: &Font) -> Vec<(String, String, String)> {
    match lower(font) {
        Ok(l) => l
            .named
            .into_iter()
            .filter_map(|(g, id, bracket)| {
                let master = font.glyphs.get(&g)?.layers.iter().find(|l| l.id == id)?.master.clone();
                Some((g, id, crate::glyphkeys::synthetic_id(&master, &bracket)))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ── reading ───────────────────────────────────────────────────────────────────────────────────

/// The number of a bracket glyph's name (`varAlt07` → 7), for putting the layers back in order.
fn suffix_number(suffix: &str) -> Option<u64> {
    suffix.strip_prefix("varAlt").and_then(|n| n.parse().ok())
}

/// Most cells of the space [`recover`] looks at.
const MAX_CELLS: usize = 100_000;
/// Most ranges [`recover`] tries for one bracket glyph.
const MAX_CANDIDATES: usize = 4096;
/// Most cell checks [`recover`] makes in one step before it settles for the boxes around the
/// rules' regions.
const MAX_WORK: usize = 20_000_000;

/// A range on each axis; `None` for an axis the range does not narrow.
type Ranges = Vec<Option<Range>>;

/// The model's rules for a range: an end at the axis's end is open; trailing axes the range does
/// not narrow are left out.
fn axis_rules(axes: &[DsAxis], r: &Ranges) -> Vec<AxisRule> {
    let mut out: Vec<AxisRule> = axes
        .iter()
        .zip(r)
        .map(|(a, r)| match r {
            Some((lo, hi)) => AxisRule { min: (*lo != a.min).then_some(*lo), max: (*hi != a.max).then_some(*hi) },
            None => AxisRule::default(),
        })
        .collect();
    // An alternate layer for the whole space still has a rule.
    while out.len() > 1 && out.last().is_some_and(|r| *r == AxisRule::default()) {
        out.pop();
    }
    out.truncate(out.len().max(1));
    if out.is_empty() {
        out.push(AxisRule::default());
    }
    out
}

fn inside(p: &[f64], r: &[Option<Range>]) -> bool {
    p.iter().zip(r).all(|(v, r)| r.is_none_or(|(lo, hi)| lo <= *v && *v <= hi))
}

/// The range each bracket glyph (`(bracket glyph, glyph)`, a glyph's in order of number) was
/// written from, read back from the rules (region on each axis, substitutions).
///
/// A bracket glyph's rules name it in the parts of its range where no later bracket glyph of
/// its glyph takes over, so the box around those parts can be short of the range. Each end of
/// the box may also have run on to the axis's end: the range is the box with as many ends at the
/// axis's ends as stay clear of every part of the space where the rules give the glyph no bracket
/// glyph or an earlier one (judged at the middle of each cell between the rules' bounds). `None`
/// for a bracket glyph no rule names.
fn recover(axes: &[DsAxis], rules: &[Parsed<'_>], brackets: &[(String, String)]) -> Vec<Option<Ranges>> {
    let n = axes.len();
    // Each rule's substitutions, by glyph.
    let sub_of: Vec<HashMap<&str, &str>> = rules.iter().map(|(_, subs)| subs.iter().map(|(b, x)| (b.as_str(), x.as_str())).collect()).collect();
    // The box around the regions of the rules that name each bracket glyph.
    let tight: Vec<Option<Ranges>> = brackets
        .iter()
        .map(|(w, base)| {
            let regions: Vec<&BoxOf> =
                rules.iter().zip(&sub_of).filter(|(_, subs)| subs.get(base.as_str()) == Some(&w.as_str())).map(|((r, _), _)| r).collect();
            if regions.is_empty() {
                return None;
            }
            Some(
                (0..n)
                    .map(|i| {
                        let ranges: Option<Vec<Range>> = regions.iter().map(|r| r.iter().find(|(j, _)| *j == i).map(|(_, v)| *v)).collect();
                        ranges.map(|list| {
                            (list.iter().map(|r| r.0).fold(f64::INFINITY, f64::min), list.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max))
                        })
                    })
                    .collect(),
            )
        })
        .collect();
    // The cells between the bounds the rules and the axes have, by their middles.
    let mut bounds: Vec<Vec<f64>> = axes.iter().map(|a| vec![a.min, a.max]).collect();
    for (region, _) in rules {
        for (i, (lo, hi)) in region {
            if let Some(b) = bounds.get_mut(*i) {
                b.extend([*lo, *hi]);
            }
        }
    }
    let mut middles: Vec<Vec<f64>> = Vec::new();
    for b in &mut bounds {
        b.retain(|v| v.is_finite());
        b.sort_by(f64::total_cmp);
        b.dedup();
        middles.push(if b.len() < 2 { b.clone() } else { b.windows(2).filter_map(|w| Some((w.first()? + w.get(1)?) / 2.0)).collect() });
    }
    let cells: usize = middles.iter().map(|m| m.len().max(1)).try_fold(1usize, |acc, k| acc.checked_mul(k)).unwrap_or(usize::MAX);
    let mut bases: Vec<&str> = brackets.iter().map(|(_, b)| b.as_str()).collect();
    bases.dedup();
    if cells > MAX_CELLS || cells.saturating_mul(rules.len().max(bases.len())) > MAX_WORK {
        return tight;
    }
    let mut points: Vec<Vec<f64>> = vec![Vec::new()];
    for m in &middles {
        let mut next = Vec::with_capacity(points.len().saturating_mul(m.len().max(1)));
        for p in &points {
            for v in m {
                let mut q = p.clone();
                q.push(*v);
                next.push(q);
            }
        }
        points = next;
    }
    // The rule that applies at each cell: the first whose region holds it.
    let first: Vec<Option<usize>> = points
        .iter()
        .map(|p| rules.iter().position(|(region, _)| region.iter().all(|(i, (lo, hi))| p.get(*i).is_some_and(|v| lo <= v && v <= hi))))
        .collect();
    // For each glyph, the bracket glyph it gets at each cell.
    let mut got: HashMap<&str, Vec<Option<&str>>> = HashMap::new();
    for base in &bases {
        let at: Vec<Option<&str>> = first.iter().map(|r| r.and_then(|r| sub_of.get(r)).and_then(|subs| subs.get(base).copied())).collect();
        got.insert(base, at);
    }
    brackets
        .iter()
        .enumerate()
        .map(|(k, (w, base))| {
            let t = tight.get(k)?.clone()?;
            let siblings: Vec<&str> = brackets.iter().filter(|(_, b)| b == base).map(|(x, _)| x.as_str()).collect();
            let rank = siblings.iter().position(|x| x == w)?;
            // Cells where the glyph gets no bracket glyph or an earlier one.
            let at = got.get(base.as_str())?;
            let forbidden: Vec<&Vec<f64>> = points
                .iter()
                .zip(at)
                .filter(|(_, x)| match x {
                    None => true,
                    Some(x) => siblings.iter().position(|s| s == x).is_some_and(|i| i < rank),
                })
                .map(|(p, _)| p)
                .collect();
            // Each narrowed axis: each end as the box has it or at the axis's end.
            let mut options: Vec<(Ranges, usize)> = vec![(Vec::new(), 0)];
            for (a, r) in axes.iter().zip(&t) {
                let choices: Vec<(Option<Range>, usize)> = match r {
                    None => vec![(None, 0)],
                    Some((lo, hi)) => {
                        let mut c = Vec::new();
                        // An end runs on only outwards.
                        let los = if a.min < *lo && lo <= hi { vec![(a.min, 1), (*lo, 0)] } else { vec![(*lo, 0)] };
                        let his = if a.max > *hi && lo <= hi { vec![(a.max, 1), (*hi, 0)] } else { vec![(*hi, 0)] };
                        for (l, ol) in los.iter().copied() {
                            for (h, oh) in his.iter().copied() {
                                let v = (Some((l, h)), ol + oh);
                                if !c.iter().any(|(x, _)| *x == v.0) {
                                    c.push(v);
                                }
                            }
                        }
                        c
                    }
                };
                let mut next = Vec::new();
                for (o, open) in &options {
                    for (c, more) in &choices {
                        let mut v = o.clone();
                        v.push(*c);
                        next.push((v, open + more));
                    }
                }
                if next.len() > MAX_CANDIDATES || next.len().saturating_mul(forbidden.len()) > MAX_WORK {
                    return Some(t);
                }
                options = next;
            }
            options.sort_by_key(|(_, open)| std::cmp::Reverse(*open));
            let fits = |r: &Ranges| !forbidden.iter().any(|p| inside(p, r));
            Some(options.into_iter().map(|(r, _)| r).find(|r| fits(r)).unwrap_or(t))
        })
        .collect()
}

/// The font with the glyphs a document's rules substitute turned back into alternate layers of
/// their glyphs, when the rules are glyphsLib's: every rule named `BRACKET.…` with one condition
/// set of conditions on the axes with both ends, substituting glyphs `<glyph>.BRACKET.<suffix>`
/// the font has. A bracket glyph's region comes from the rules ([`recover`]); each master's
/// layer of it with a non-empty [`ORIGINAL_LAYER_NAME`] becomes an alternate layer of that name,
/// with the region as its rules (an end at the axis's end left open), its components pointing
/// back at their glyphs, and the GLIF of the UFO layer `<name>.background` as its background; the
/// others stood in for a master layer and go. The bracket glyphs' kerning goes with them. `None`
/// when the document is not in that form, or a bracket glyph has other layers.
pub(crate) fn raise(font: &Font, rules: &Rules) -> Option<Font> {
    if rules.rules.is_empty() {
        return None;
    }
    let axes = ds_axes(font);
    // Bracket glyph → (base glyph, regions).
    let mut found: Vec<(String, String, Vec<BoxOf>)> = Vec::new();
    for r in &rules.rules {
        if r.legacy || !r.name.as_deref().is_some_and(|n| n.starts_with("BRACKET.")) || r.subs.is_empty() {
            return None;
        }
        let [set] = r.sets.as_slice() else { return None };
        let mut region: Vec<(usize, Range)> = Vec::new();
        for c in set {
            let i = axes.iter().position(|a| a.name == c.name)?;
            if region.iter().any(|(j, _)| *j == i) {
                return None;
            }
            region.push((i, (c.min?, c.max?)));
        }
        for (name, with) in &r.subs {
            with.strip_prefix(name.as_str()).and_then(|s| s.strip_prefix(INFIX)).filter(|s| !s.is_empty())?;
            if font.glyphs.get(name).is_none() || font.glyphs.get(with).is_none() {
                return None;
            }
            match found.iter_mut().find(|(w, _, _)| w == with) {
                Some((_, base, regions)) => {
                    if base != name {
                        return None;
                    }
                    regions.push(region.clone());
                }
                None => found.push((with.clone(), name.clone(), vec![region.clone()])),
            }
        }
    }
    // The bracket glyphs of each glyph, by number, and their ranges.
    let mut order: Vec<(String, String)> = found.iter().map(|(w, b, _)| (w.clone(), b.clone())).collect();
    order.sort_by(|a, b| {
        let pos = |n: &str| font.glyphs.position(n).unwrap_or(usize::MAX);
        let num = |w: &str, base: &str| w.get(base.len() + INFIX.len()..).and_then(suffix_number);
        pos(&a.1).cmp(&pos(&b.1)).then(num(&a.0, &a.1).cmp(&num(&b.0, &b.1))).then(a.0.cmp(&b.0))
    });
    let parsed = parse(&axes, &rules.rules);
    let ranges = recover(&axes, &parsed, &order);
    let mut boxes: Vec<(String, String, Vec<AxisRule>)> = Vec::new();
    for ((w, b), r) in order.iter().zip(ranges) {
        boxes.push((w.clone(), b.clone(), axis_rules(&axes, &r?)));
    }
    let rules_for = |name: &str| boxes.iter().find(|(w, _, _)| w == name).map(|(_, b, r)| (b.as_str(), r));
    let order = &boxes;
    let mut out = font.clone();
    let masters: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    for (with, base, axis_rules) in order {
        let g = font.glyphs.get(with)?;
        // Other layers: only the backgrounds glyphsLib writes to `<layer name>.background`.
        let mut used = vec![false; g.layers.len()];
        let mut layers: Vec<Layer> = Vec::new();
        for m in &masters {
            let Some(l) = g.layer(m) else { continue };
            let original = match l.info.user_data.get(ORIGINAL_LAYER_NAME) {
                Some(Value::String(s)) => s.as_str().to_string(),
                _ => return None,
            };
            let background = format!("{original}{}", ufo::BACKGROUND_SUFFIX);
            let other = g.layers.iter().position(|b| !b.is_master_layer() && b.master == *m && b.info.name.as_deref() == Some(background.as_str()));
            if original.is_empty() {
                continue;
            }
            let mut l = l.clone();
            if let Some(k) = other {
                if l.info.background.is_some() {
                    return None;
                }
                if let Some(slot) = used.get_mut(k) {
                    *slot = true;
                }
                l.info.background = Some(ufo::background_of(g.layers.get(k)?.clone()));
            }
            l.id = crate::glyphkeys::synthetic_id(m, with);
            {
                let info = &mut *l.info;
                info.name = Some(original);
                info.attrs.axis_rules = axis_rules.clone();
                if let Some(i) = info.user_data.0.entries.iter().position(|(k, _)| k.as_str() == ORIGINAL_LAYER_NAME) {
                    info.user_data.0.remove_at(i);
                }
            }
            if let Some(f) = &g.file_name
                && l.extra.get(Family::Ufo, "fileName").is_none()
            {
                l.extra.push(Family::Ufo, "fileName", Value::String(Str::new(f)));
            }
            for s in &mut l.shapes {
                if let Shape::Component(c) = s
                    && let Some((b, r)) = rules_for(&c.glyph)
                {
                    if r != axis_rules {
                        return None;
                    }
                    c.glyph = b.to_string();
                }
            }
            layers.push(l);
        }
        if g.layers.iter().zip(&used).any(|(l, u)| !l.is_master_layer() && !u) {
            return None;
        }
        let mut target = out.glyphs.get_mut(base)?;
        target.layers.extend(layers);
    }
    let gone: Vec<&str> = boxes.iter().map(|(w, _, _)| w.as_str()).collect();
    // Another glyph that uses a bracket glyph as a component keeps it a glyph.
    let uses =
        |g: &Glyph| g.layers.iter().flat_map(|l| l.shapes.iter()).any(|s| matches!(s, Shape::Component(c) if gone.contains(&c.glyph.as_str())));
    if font.glyphs.iter().any(|g| !gone.contains(&g.name.as_str()) && uses(g)) {
        return None;
    }
    out.glyphs.retain(|g| !gone.contains(&g.name.as_str()));
    let mut kerning = (*out.kerning).clone();
    for mk in &mut kerning.ltr.0 {
        mk.firsts.retain(|f| !gone.contains(&f.first.as_str()));
        for f in &mut mk.firsts {
            f.seconds.retain(|(s, _)| !gone.contains(&s.as_str()));
        }
    }
    out.kerning = Arc::new(kerning);
    Some(out)
}

/// The region an alternate layer's rules cover in a designspace of `font`, axis by axis, in
/// design coordinates: an open end at the axis's end, `None` for an axis it does not narrow.
pub(crate) fn region(font: &Font, rules: &[AxisRule]) -> Vec<Option<(f64, f64)>> {
    let axes = ds_axes(font);
    let bx = box_of(rules, &axes);
    (0..axes.len()).map(|i| bx.iter().find(|(j, _)| *j == i).map(|(_, r)| *r)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(items: &[(&str, f64, f64)]) -> Region {
        Region(items.iter().map(|(n, a, b)| (n.to_string(), (*a, *b))).collect())
    }

    fn subst(pairs: &[(&str, &str)]) -> Subst {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    /// The example of fontTools' `overlayFeatureVariations` docstring.
    #[test]
    fn the_overlay_matches_fonttools_example() {
        let cs = vec![
            (vec![region(&[("wght", 0.5, 1.0)])], subst(&[("dollar", "dollar.rvrn")])),
            (vec![region(&[("wght", 0.5, 1.0)])], subst(&[("dollar", "dollar.rvrn")])),
            (vec![region(&[("wdth", 0.5, 1.0)])], subst(&[("cent", "cent.rvrn")])),
            (vec![region(&[("wght", 0.5, 1.0), ("wdth", -1.0, 1.0)])], subst(&[("dollar", "dollar.rvrn")])),
        ];
        let out = overlay(cs).unwrap();
        type Shown = (Vec<(String, Range)>, Vec<Subst>);
        let shown: Vec<Shown> = out.into_iter().map(|(r, s)| (r.sorted(), s)).collect();
        assert_eq!(
            shown,
            vec![
                (
                    region(&[("wdth", 0.5, 1.0), ("wght", 0.5, 1.0)]).sorted(),
                    vec![subst(&[("dollar", "dollar.rvrn")]), subst(&[("cent", "cent.rvrn")])]
                ),
                (region(&[("wdth", 0.5, 1.0)]).sorted(), vec![subst(&[("cent", "cent.rvrn")])]),
                (region(&[("wght", 0.5, 1.0)]).sorted(), vec![subst(&[("dollar", "dollar.rvrn")])]),
            ]
        );
    }

    #[test]
    fn boxes_overlay_as_fonttools_overlays_them() {
        // Nested ranges on one axis: the inner range, then what is left of the outer one.
        let (inter, rem) = overlay_box(&region(&[("w", 0.5, 1.0)]), &region(&[("w", 0.0, 1.0)]));
        assert!(inter.unwrap().same(&region(&[("w", 0.5, 1.0)])));
        assert!(rem.unwrap().same(&region(&[("w", 0.0, 0.5)])));
        // Disjoint ranges do not meet.
        let (inter, rem) = overlay_box(&region(&[("w", 0.0, 0.2)]), &region(&[("w", 0.5, 1.0)]));
        assert!(inter.is_none() && rem.unwrap().same(&region(&[("w", 0.5, 1.0)])));
        // A box inside the other leaves nothing.
        let (_, rem) = overlay_box(&region(&[("w", 0.0, 1.0)]), &region(&[("w", 0.2, 0.4)]));
        assert!(rem.is_none());
    }

    #[test]
    fn ranks_count_and_list_their_bits() {
        let r = Rank::bit(3).or(&Rank::bit(70));
        assert_eq!((r.count(), r.ones().collect::<Vec<_>>()), (2, vec![3, 70]));
    }

    #[test]
    fn rule_names_use_python_numbers() {
        assert_eq!(py_number(150.0), "150");
        assert_eq!(py_number(137.5), "137.5");
    }
}
