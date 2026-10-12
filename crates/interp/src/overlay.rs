//! The overlay of alternate-layer boxes into non-overlapping feature-variation records (M9.8).
//!
//! Rows: `compile.featureVariations`, `alternateLayer.variableExport`.
//!
//! Ported from fontTools 4.66.1 (MIT, see `NOTICE`): `varLib/featureVars.py`
//! (`overlayFeatureVariations`, `overlayBox`, `cleanupBox`). fontc's port
//! (`fontir/src/feature_variations.rs`, Apache-2.0) behaves the same.
//!
//! Contract: the input is a list of rules, each a set of boxes (any of which applies) and a
//! payload; the output is a list of records whose boxes do not overlap except in the order given,
//! each with the payloads of every rule that applies there, in input order. The most specific
//! region comes first. The work is bounded by a budget ([`crate::MAX_OVERLAY_WORK`]) and the
//! output by [`crate::MAX_OVERLAY_RECORDS`]; past either the overlay stops with
//! [`InterpError::LimitExceeded`].
//!
//! [`overlay`] is the overlay alone. [`overlay_substitutions`] is all of
//! `overlayFeatureVariations`: rules with the same substitutions merged, then rules with the same
//! region merged (the earlier rule winning a glyph both substitute), then the overlay.
//!
//! One difference from fontTools: boxes are keyed by axis index, not by axis tag, so the boxes of
//! one region are sorted by axis index. That order only matters for a region of several boxes,
//! which alternate layers never make (each rule's substitution is distinct).

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::InterpError;
use crate::error::{MAX_COORDINATE, MAX_OVERLAY_RECORDS};

/// The range of one axis in a [`RegionBox`], in normalized coordinates, both ends included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisSpan {
    /// Index into the designspace's axes.
    pub axis: usize,
    pub min: f64,
    pub max: f64,
}

/// A box in normalized coordinates: the axes it constrains, sorted by axis index; an axis not
/// listed is unconstrained.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionBox(pub Vec<AxisSpan>);

/// One rule of the overlay's input: where it applies and what it carries (for feature variations,
/// the substitutions of one alternate set).
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayRule<T> {
    /// Any of these boxes; empty applies nowhere.
    pub boxes: Vec<RegionBox>,
    pub payload: T,
}

/// One record of the overlay's output.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayRecord<T> {
    pub region: RegionBox,
    /// The payloads of every rule that applies in `region`, in input order.
    pub payloads: Vec<T>,
}

/// Substitutions: (glyph, replacement) name pairs. Two lists with the same pairs in another order
/// are the same substitutions.
pub type Substitutions = Vec<(String, String)>;

impl RegionBox {
    fn span(&self, axis: usize) -> Option<&AxisSpan> {
        self.0.iter().find(|s| s.axis == axis)
    }

    fn has(&self, axis: usize) -> bool {
        self.span(axis).is_some()
    }

    /// Sets the range of `axis`, keeping the spans sorted by axis.
    fn set(&mut self, axis: usize, min: f64, max: f64) {
        match self.0.binary_search_by(|s| s.axis.cmp(&axis)) {
            Ok(i) => {
                if let Some(s) = self.0.get_mut(i) {
                    s.min = min;
                    s.max = max;
                }
            }
            Err(i) => self.0.insert(i, AxisSpan { axis, min, max }),
        }
    }

    /// The box with its spans sorted by axis (a later span of one axis replacing an earlier one,
    /// as in a dictionary) and without the spans of (−1, 1), which constrain nothing
    /// (`cleanupBox`). Refuses values that are not usable numbers and spans whose minimum is above
    /// their maximum.
    fn canonical(&self) -> Result<RegionBox, InterpError> {
        let mut out = RegionBox::default();
        for s in &self.0 {
            if !usable(s.min) || !usable(s.max) {
                return Err(InterpError::BadNumber { what: "a bound of an alternate range".to_string() });
            }
            if s.min > s.max {
                return Err(InterpError::BadNumber { what: "an alternate range whose minimum is above its maximum".to_string() });
            }
            out.set(s.axis, s.min, s.max);
        }
        out.0.retain(|s| (s.min, s.max) != (-1.0, 1.0));
        Ok(out)
    }

    /// A hashable key: equal for boxes with the same spans (−0 and 0 alike, as in Python).
    fn key(&self) -> Vec<(usize, u64, u64)> {
        self.0.iter().map(|s| (s.axis, (s.min + 0.0).to_bits(), (s.max + 0.0).to_bits())).collect()
    }

    /// The order of `tuple(sorted(box.items()))`.
    fn order(&self, other: &RegionBox) -> Ordering {
        for (a, b) in self.0.iter().zip(&other.0) {
            let o = a.axis.cmp(&b.axis).then(a.min.total_cmp(&b.min)).then(a.max.total_cmp(&b.max));
            if o != Ordering::Equal {
                return o;
            }
        }
        self.0.len().cmp(&other.0.len())
    }
}

fn usable(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// Python's `max(a, b)` and `min(a, b)` on finite numbers.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `overlayBox(top, bot)`: the intersection of the two boxes, if they meet, and the part of `bot`
/// outside `top`. The remainder may be larger than that part (all of `bot`) when the part is not
/// a box; it is `None` when `bot` lies inside `top`.
fn overlay_box(top: &RegionBox, bot: &RegionBox) -> (Option<RegionBox>, Option<RegionBox>) {
    let mut inter = top.clone();
    for s in &bot.0 {
        inter.set(s.axis, s.min, s.max);
    }
    for t in &top.0 {
        let Some(b) = bot.span(t.axis) else { continue };
        let (lo, hi) = (py_max(t.min, b.min), py_min(t.max, b.max));
        // Boxes that only touch do not intersect.
        if lo >= hi {
            return (None, Some(bot.clone()));
        }
        inter.set(t.axis, lo, hi);
    }

    let mut rem = bot.clone();
    let mut extruding = top.0.iter().any(|t| !bot.has(t.axis));
    let mut inside = !extruding;
    for b in &bot.0 {
        if !top.has(b.axis) {
            continue;
        }
        let Some(i) = inter.span(b.axis) else { continue };
        let (min1, max1, min2, max2) = (i.min, i.max, b.min, b.max);
        if min1 <= min2 && max2 <= max1 {
            continue;
        }
        // A second axis that sticks out: the remainder is no box, so all of `bot` stays.
        if extruding {
            return (Some(inter), Some(bot.clone()));
        }
        extruding = true;
        inside = false;
        let (lo, hi) = if min1 <= min2 {
            (py_max(max1, min2), max2)
        } else if max2 <= max1 {
            (min2, py_min(min1, max2))
        } else {
            return (Some(inter), Some(bot.clone()));
        };
        rem.set(b.axis, lo, hi);
    }
    if inside { (Some(inter), None) } else { (Some(inter), Some(rem)) }
}

/// A set of rule indices, the integer bit set fontTools uses for a rank.
#[derive(Clone, Debug, Default, PartialEq)]
struct Rank(Vec<u64>);

impl Rank {
    fn bit(i: usize) -> Rank {
        let mut words = vec![0u64; i / 64 + 1];
        if let Some(w) = words.get_mut(i / 64) {
            *w |= 1u64 << (i % 64);
        }
        Rank(words)
    }

    fn or_assign(&mut self, other: &Rank) {
        if self.0.len() < other.0.len() {
            self.0.resize(other.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= *b;
        }
    }

    fn count(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }

    fn ones(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(i, w)| (0..64usize).filter(move |b| w & (1u64 << b) != 0).map(move |b| i * 64 + b))
    }

    fn words(&self) -> usize {
        self.0.len()
    }
}

/// The boxes met so far with the rules that apply in each, in the order first met.
struct BoxMap {
    entries: Vec<(RegionBox, Rank)>,
    index: HashMap<Vec<(usize, u64, u64)>, usize>,
}

impl BoxMap {
    /// The map that holds the whole space with no rule (`initMapInit`).
    fn new() -> BoxMap {
        let mut map = BoxMap { entries: Vec::new(), index: HashMap::new() };
        map.add(RegionBox::default(), &Rank::default());
        map
    }

    fn add(&mut self, region: RegionBox, rank: &Rank) {
        let key = region.key();
        match self.index.get(&key).and_then(|&i| self.entries.get_mut(i)) {
            Some((_, r)) => r.or_assign(rank),
            None => {
                self.index.insert(key, self.entries.len());
                self.entries.push((region, rank.clone()));
            }
        }
    }
}

/// Counts work against the budget.
struct Budget {
    used: usize,
    limit: usize,
}

impl Budget {
    fn charge(&mut self, steps: usize) -> Result<(), InterpError> {
        self.used = self.used.saturating_add(steps);
        if self.used > self.limit {
            return Err(InterpError::LimitExceeded { what: "steps of work in the overlay of alternate ranges", limit: self.limit });
        }
        Ok(())
    }
}

/// The non-overlapping records of `rules`, most specific first, within `budget` steps of work.
pub fn overlay<T: Clone>(rules: &[OverlayRule<T>], budget: usize) -> Result<Vec<OverlayRecord<T>>, InterpError> {
    let mut budget = Budget { used: 0, limit: budget };
    let mut regions: Vec<Vec<RegionBox>> = Vec::with_capacity(rules.len().min(MAX_OVERLAY_RECORDS));
    for rule in rules {
        budget.charge(1usize.saturating_add(rule.boxes.len()))?;
        regions.push(rule.boxes.iter().map(RegionBox::canonical).collect::<Result<_, _>>()?);
    }

    let mut map = BoxMap::new();
    for (i, region) in regions.iter().enumerate() {
        let current = Rank::bit(i);
        let mut next = BoxMap::new();
        for (bx, rank) in &map.entries {
            for top in region {
                budget.charge(
                    1usize.saturating_add(top.0.len()).saturating_add(bx.0.len()).saturating_add(rank.words()).saturating_add(current.words()),
                )?;
                let (inter, rem) = overlay_box(top, bx);
                if let Some(inter) = inter {
                    let mut both = rank.clone();
                    both.or_assign(&current);
                    next.add(inter, &both);
                }
                if let Some(rem) = rem {
                    next.add(rem, rank);
                }
            }
            // Only the whole space carries no rule, so every other entry becomes a record.
            if next.entries.len() > MAX_OVERLAY_RECORDS.saturating_add(1) {
                return Err(InterpError::LimitExceeded { what: "feature-variation records", limit: MAX_OVERLAY_RECORDS });
            }
        }
        map = next;
    }

    let mut entries = map.entries;
    // Stable, so boxes with as many rules keep the order they were met in.
    entries.sort_by_key(|(_, rank)| std::cmp::Reverse(rank.count()));
    let mut out = Vec::new();
    for (region, rank) in entries {
        if rank.count() == 0 {
            continue;
        }
        budget.charge(rank.words())?;
        let payloads = rank.ones().filter_map(|i| rules.get(i).map(|r| r.payload.clone())).collect();
        out.push(OverlayRecord { region, payloads });
    }
    if out.len() > MAX_OVERLAY_RECORDS {
        return Err(InterpError::LimitExceeded { what: "feature-variation records", limit: MAX_OVERLAY_RECORDS });
    }
    Ok(out)
}

fn same_substitutions(a: &Substitutions, b: &Substitutions) -> bool {
    a.len() == b.len() && a.iter().all(|p| b.contains(p))
}

/// `a.update(b)`: `b`'s pairs replace `a`'s for the same glyph; new glyphs are appended.
fn update(a: &mut Substitutions, b: &Substitutions) {
    for (k, v) in b {
        match a.iter_mut().find(|(x, _)| x == k) {
            Some(slot) => slot.1.clone_from(v),
            None => a.push((k.clone(), v.clone())),
        }
    }
}

/// A region box as a hashable key: per axis, the axis index and the bits of its two bounds.
type BoxKey = Vec<(usize, u64, u64)>;

/// `overlayFeatureVariations`: the rules with the same substitutions merged into one rule over
/// all their boxes, the rules with the same region merged into one rule (the earlier rule's
/// replacement winning for a glyph both substitute), then [`overlay`].
pub fn overlay_substitutions(rules: &[OverlayRule<Substitutions>], budget: usize) -> Result<Vec<OverlayRecord<Substitutions>>, InterpError> {
    let mut work = Budget { used: 0, limit: budget };
    let mut merged: Vec<OverlayRule<Substitutions>> = Vec::new();
    for rule in rules {
        work.charge(1usize.saturating_add(merged.len()))?;
        match merged.iter_mut().find(|m| same_substitutions(&m.payload, &rule.payload)) {
            Some(m) => m.boxes.extend(rule.boxes.iter().cloned()),
            None => merged.push(rule.clone()),
        }
    }

    // Reversed, so that the earlier of two rules for one region has the last word.
    let mut by_region: Vec<(Vec<RegionBox>, Substitutions)> = Vec::new();
    let mut index: HashMap<Vec<BoxKey>, usize> = HashMap::new();
    for rule in merged.into_iter().rev() {
        work.charge(1usize.saturating_add(rule.boxes.len()))?;
        let mut boxes = rule.boxes.iter().map(RegionBox::canonical).collect::<Result<Vec<_>, _>>()?;
        boxes.sort_by(RegionBox::order);
        let key: Vec<_> = boxes.iter().map(RegionBox::key).collect();
        match index.get(&key).and_then(|&i| by_region.get_mut(i)) {
            Some((_, subs)) => update(subs, &rule.payload),
            None => {
                index.insert(key, by_region.len());
                by_region.push((boxes, rule.payload));
            }
        }
    }
    by_region.reverse();

    let rules: Vec<OverlayRule<Substitutions>> = by_region.into_iter().map(|(boxes, payload)| OverlayRule { boxes, payload }).collect();
    overlay(&rules, budget.saturating_sub(work.used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_OVERLAY_WORK;

    const WGHT: usize = 0;
    const WDTH: usize = 1;

    fn bx(spans: &[(usize, f64, f64)]) -> RegionBox {
        RegionBox(spans.iter().map(|&(axis, min, max)| AxisSpan { axis, min, max }).collect())
    }

    fn rule(boxes: &[&[(usize, f64, f64)]], subs: &[(&str, &str)]) -> OverlayRule<Substitutions> {
        OverlayRule { boxes: boxes.iter().map(|b| bx(b)).collect(), payload: subs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect() }
    }

    fn subs(pairs: &[(&str, &str)]) -> Substitutions {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    /// The example in fontTools' `overlayFeatureVariations` docstring, with its expected output.
    #[test]
    fn overlay_matches_the_reference_example() {
        let rules = vec![
            rule(&[&[(WGHT, 0.5, 1.0)]], &[("dollar", "dollar.rvrn")]),
            rule(&[&[(WGHT, 0.5, 1.0)]], &[("dollar", "dollar.rvrn")]),
            rule(&[&[(WDTH, 0.5, 1.0)]], &[("cent", "cent.rvrn")]),
            rule(&[&[(WGHT, 0.5, 1.0), (WDTH, -1.0, 1.0)]], &[("dollar", "dollar.rvrn")]),
        ];
        let out = overlay_substitutions(&rules, MAX_OVERLAY_WORK).unwrap();
        let expected = vec![
            OverlayRecord {
                region: bx(&[(WGHT, 0.5, 1.0), (WDTH, 0.5, 1.0)]),
                payloads: vec![subs(&[("dollar", "dollar.rvrn")]), subs(&[("cent", "cent.rvrn")])],
            },
            OverlayRecord { region: bx(&[(WDTH, 0.5, 1.0)]), payloads: vec![subs(&[("cent", "cent.rvrn")])] },
            OverlayRecord { region: bx(&[(WGHT, 0.5, 1.0)]), payloads: vec![subs(&[("dollar", "dollar.rvrn")])] },
        ];
        assert_eq!(out, expected);
    }

    /// Two nested ranges on one axis (a glyph with alternates from 0.5 and from 0.8): the inner
    /// range comes first with both substitutions, the outer range's remainder after it.
    #[test]
    fn nested_ranges_put_the_most_specific_first() {
        let rules = vec![rule(&[&[(WGHT, 0.5, 1.0)]], &[("a", "a.BRACKET.varAlt01")]), rule(&[&[(WGHT, 0.8, 1.0)]], &[("a", "a.BRACKET.varAlt02")])];
        let out = overlay_substitutions(&rules, MAX_OVERLAY_WORK).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].region, bx(&[(WGHT, 0.8, 1.0)]));
        assert_eq!(out[0].payloads, vec![subs(&[("a", "a.BRACKET.varAlt01")]), subs(&[("a", "a.BRACKET.varAlt02")])]);
        assert_eq!(out[1].region, bx(&[(WGHT, 0.5, 0.8)]));
        assert_eq!(out[1].payloads, vec![subs(&[("a", "a.BRACKET.varAlt01")])]);
    }

    /// Ranges that only touch do not overlap; a whole-axis span constrains nothing. Records with
    /// as many rules keep the order they were made in, so the later range comes first here.
    #[test]
    fn touching_ranges_stay_apart_and_full_spans_vanish() {
        let rules = vec![
            rule(&[&[(WGHT, -1.0, 0.0)]], &[("a", "a.BRACKET.varAlt01")]),
            rule(&[&[(WGHT, 0.0, 1.0)]], &[("b", "b.BRACKET.varAlt01")]),
            rule(&[&[(WDTH, -1.0, 1.0)]], &[("c", "c.BRACKET.varAlt01")]),
        ];
        let out = overlay_substitutions(&rules, MAX_OVERLAY_WORK).unwrap();
        let regions: Vec<_> = out.iter().map(|r| (r.region.clone(), r.payloads.len())).collect();
        assert_eq!(regions, vec![(bx(&[(WGHT, 0.0, 1.0)]), 2), (bx(&[(WGHT, -1.0, 0.0)]), 2), (bx(&[]), 1)]);
    }

    /// Two rules for one region become one; the earlier rule wins a glyph both substitute.
    #[test]
    fn rules_of_one_region_merge_with_the_earlier_winning() {
        let rules = vec![rule(&[&[(WGHT, 0.5, 1.0)]], &[("a", "a.one")]), rule(&[&[(WGHT, 0.5, 1.0)]], &[("a", "a.two"), ("b", "b.two")])];
        let out = overlay_substitutions(&rules, MAX_OVERLAY_WORK).unwrap();
        assert_eq!(out.len(), 1);
        let mut payload = out[0].payloads[0].clone();
        payload.sort();
        assert_eq!(payload, subs(&[("a", "a.one"), ("b", "b.two")]));
    }

    /// Many crossing boxes on two axes stop at the budget instead of growing without end.
    #[test]
    fn the_budget_stops_the_overlay() {
        let rules: Vec<_> = (0..200)
            .map(|i| {
                let t = f64::from(i) / 200.0;
                OverlayRule { boxes: vec![bx(&[(WGHT, t, 1.0), (WDTH, -1.0, 1.0 - t)])], payload: i }
            })
            .collect();
        let err = overlay(&rules, 10_000).unwrap_err();
        assert!(matches!(err, InterpError::LimitExceeded { .. }), "{err:?}");
    }

    #[test]
    fn unusable_bounds_are_refused() {
        let rules = vec![OverlayRule { boxes: vec![bx(&[(WGHT, f64::NAN, 1.0)])], payload: () }];
        assert!(matches!(overlay(&rules, MAX_OVERLAY_WORK), Err(InterpError::BadNumber { .. })));
        let rules = vec![OverlayRule { boxes: vec![bx(&[(WGHT, 0.6, 0.2)])], payload: () }];
        assert!(matches!(overlay(&rules, MAX_OVERLAY_WORK), Err(InterpError::BadNumber { .. })));
    }
}
