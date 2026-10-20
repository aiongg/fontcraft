//! Pair adjustment subtables, merged as fontTools' `varLib.merger` merges them (MIT License,
//! Copyright (c) 2017 Google Inc. and the fontTools authors): `PairPos`, `PairSet`, the
//! subtable joining of `_Lookup_PairPos_subtables_canonicalize`, and the class realignment of
//! `_PairPosFormat2_align_matrices` with `misc.classifyTools.Classifier`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use write_fonts::tables::gpos::{Class1Record, Class2Record, PairPos, PairSet, PairValueRecord};
use write_fonts::types::GlyphId16;

use super::{Ctx, Vr, bad, class_map, coverage, merge_orders, write_class_def, write_coverage};
use crate::Result;
use crate::limits::MAX_LAYOUT_ITEMS;

/// The two value records of a pair.
type Values = (Option<Vr>, Option<Vr>);

#[derive(Clone, Debug)]
struct Pvr {
    second: u16,
    v: Values,
}

#[derive(Clone, Debug)]
enum Pair {
    F1 {
        vf: (u16, u16),
        glyphs: Vec<u16>,
        sets: Vec<Vec<Pvr>>,
        /// (first, second) → values, for the effective-value search.
        index: HashMap<(u16, u16), Values>,
    },
    F2 {
        vf: (u16, u16),
        glyphs: Vec<u16>,
        covered: std::collections::HashSet<u16>,
        cd1: HashMap<u16, u16>,
        cd2: HashMap<u16, u16>,
        class2_count: usize,
        /// `None` only for the transparent rows of a subtable being joined.
        rows: Vec<Vec<Option<Values>>>,
    },
}

fn f1(vf: (u16, u16), glyphs: Vec<u16>, sets: Vec<Vec<Pvr>>) -> Pair {
    let index = glyphs.iter().zip(&sets).flat_map(|(g, set)| set.iter().map(move |r| ((*g, r.second), r.v))).collect();
    Pair::F1 { vf, glyphs, sets, index }
}

fn f2(vf: (u16, u16), glyphs: Vec<u16>, cd1: HashMap<u16, u16>, cd2: HashMap<u16, u16>, class2_count: usize, rows: Vec<Vec<Option<Values>>>) -> Pair {
    let covered = glyphs.iter().copied().collect();
    Pair::F2 { vf, glyphs, covered, cd1, cd2, class2_count, rows }
}

impl Pair {
    fn read(ctx: &Ctx, m: usize, p: &PairPos) -> Result<Pair> {
        let vr = Vr::read;
        match p {
            PairPos::Format1(t) => {
                let glyphs = coverage(ctx, m, &t.coverage)?;
                if glyphs.len() != t.pair_sets.len() {
                    return Err(bad(format!("master {m}: a pair adjustment subtable's coverage and pair sets differ in length")));
                }
                let mut vf = (0, 0);
                let mut sets = Vec::with_capacity(t.pair_sets.len());
                for set in &t.pair_sets {
                    let mut recs = Vec::with_capacity(set.pair_value_records.len());
                    for r in &set.pair_value_records {
                        vf = (r.value_record1.format().bits(), r.value_record2.format().bits());
                        recs.push(Pvr { second: ctx.gid(m, r.second_glyph)?, v: (vr(&r.value_record1)?, vr(&r.value_record2)?) });
                    }
                    sets.push(recs);
                }
                Ok(f1(vf, glyphs, sets))
            }
            PairPos::Format2(t) => {
                let glyphs = coverage(ctx, m, &t.coverage)?;
                let cd1 = class_map(ctx, m, &t.class_def1)?;
                let cd2 = class_map(ctx, m, &t.class_def2)?;
                let mut vf = (0, 0);
                let mut rows = Vec::with_capacity(t.class1_records.len());
                for row in &t.class1_records {
                    let mut cells = Vec::with_capacity(row.class2_records.len());
                    for c in &row.class2_records {
                        vf = (c.value_record1.format().bits(), c.value_record2.format().bits());
                        cells.push(Some((vr(&c.value_record1)?, vr(&c.value_record2)?)));
                    }
                    rows.push(cells);
                }
                let class2_count = rows.first().map_or_else(|| cd2.values().map(|c| usize::from(*c) + 1).max().unwrap_or(1), Vec::len);
                Ok(f2(vf, glyphs, cd1, cd2, class2_count, rows))
            }
        }
    }

    fn vf(&self) -> (u16, u16) {
        match self {
            Pair::F1 { vf, .. } | Pair::F2 { vf, .. } => *vf,
        }
    }

    fn glyphs(&self) -> &[u16] {
        match self {
            Pair::F1 { glyphs, .. } | Pair::F2 { glyphs, .. } => glyphs,
        }
    }

    fn has_records(&self) -> bool {
        match self {
            Pair::F1 { sets, .. } => !sets.is_empty(),
            Pair::F2 { rows, .. } => !rows.is_empty(),
        }
    }

    fn write(&self) -> PairPos {
        let cov = write_coverage(self.glyphs());
        match self {
            Pair::F1 { vf, sets, .. } => {
                let sets = sets
                    .iter()
                    .map(|set| {
                        PairSet::new(
                            set.iter()
                                .map(|r| {
                                    PairValueRecord::new(GlyphId16::new(r.second), Vr::write(r.v.0.as_ref(), vf.0), Vr::write(r.v.1.as_ref(), vf.1))
                                })
                                .collect(),
                        )
                    })
                    .collect();
                PairPos::format_1(cov, sets)
            }
            Pair::F2 { vf, cd1, cd2, rows, .. } => {
                let sorted = |m: &HashMap<u16, u16>| m.iter().map(|(g, c)| (*g, *c)).collect::<BTreeMap<u16, u16>>();
                let records = rows
                    .iter()
                    .map(|row| {
                        Class1Record::new(
                            row.iter()
                                .map(|c| {
                                    let (a, b) = c.unwrap_or_default();
                                    Class2Record::new(Vr::write(a.as_ref(), vf.0), Vr::write(b.as_ref(), vf.1))
                                })
                                .collect(),
                        )
                    })
                    .collect();
                PairPos::format_2(cov, write_class_def(&sorted(cd1)), write_class_def(&sorted(cd2)), records)
            }
        }
    }
}

/// `_Lookup_PairPos_get_effective_value_pair`: the values the lookup's subtables give the pair.
fn effective(subtables: &[Pair], first: u16, second: u16) -> Result<Option<Values>> {
    for s in subtables {
        match s {
            Pair::F1 { glyphs, index, .. } => {
                if glyphs.contains(&first)
                    && let Some(v) = index.get(&(first, second))
                {
                    return Ok(Some(*v));
                }
            }
            Pair::F2 { covered, cd1, cd2, rows, .. } => {
                if covered.contains(&first) {
                    let c1 = usize::from(cd1.get(&first).copied().unwrap_or(0));
                    let c2 = usize::from(cd2.get(&second).copied().unwrap_or(0));
                    let cell = rows.get(c1).and_then(|r| r.get(c2)).ok_or_else(|| bad("a pair class is out of range"))?;
                    return Ok(Some(cell.unwrap_or_default()));
                }
            }
        }
    }
    Ok(None)
}

fn check_second_formats(lst: &[&Pair]) -> Result<()> {
    let zero: Vec<bool> = lst.iter().filter(|p| p.has_records()).map(|p| p.vf().1 == 0).collect();
    if zero.windows(2).any(|w| matches!(w, [a, b] if a != b)) {
        return Err(bad("pair adjustments of the masters differ in whether the second glyph is adjusted"));
    }
    Ok(())
}

fn union_vf(lst: &[&Pair]) -> (u16, u16) {
    lst.iter().fold((0, 0), |(a, b), p| (a | p.vf().0, b | p.vf().1))
}

/// A record of `format`, all fields zero; `None` for format 0.
fn blank(format: u16) -> Option<Vr> {
    (format != 0).then(|| Vr::with_format(format, None))
}

// ---------- class realignment ----------

/// `misc.classifyTools.Classifier`: the coarsest partition refining every set added.
#[derive(Default)]
struct Classifier {
    sets: Vec<BTreeSet<u16>>,
    mapping: HashMap<u16, usize>,
}

impl Classifier {
    fn add(&mut self, s: &BTreeSet<u16>) {
        let (mut inter, diff): (BTreeSet<u16>, BTreeSet<u16>) = s.iter().partition(|g| self.mapping.contains_key(g));
        if !diff.is_empty() {
            let at = self.sets.len();
            for g in &diff {
                self.mapping.insert(*g, at);
            }
            self.sets.push(diff);
        }
        while let Some(g) = inter.iter().next().copied() {
            let Some(old) = self.mapping.get(&g).copied() else { break };
            let Some(old_set) = self.sets.get_mut(old) else { break };
            let split: BTreeSet<u16> = old_set.intersection(&inter).copied().collect();
            for x in &split {
                old_set.remove(x);
                inter.remove(x);
            }
            let at = self.sets.len();
            for x in &split {
                self.mapping.insert(*x, at);
            }
            self.sets.push(split);
        }
    }

    /// The classes, largest first, ties in the order of their sorted glyph names.
    fn classes(self, ctx: &Ctx) -> Vec<BTreeSet<u16>> {
        let mut keyed: Vec<(usize, Vec<&str>, BTreeSet<u16>)> = self
            .sets
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(|s| {
                let mut names: Vec<&str> = s.iter().map(|g| ctx.name(*g)).collect();
                names.sort_unstable();
                (s.len(), names, s)
            })
            .collect();
        keyed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        keyed.into_iter().map(|(_, _, s)| s).collect()
    }
}

/// `_ClassDef_invert`: the glyphs of each class; with `all`, class 0 is the glyphs of `all` in
/// no other class and the others are limited to `all`; without, class 0 is left out.
fn invert(cd: &HashMap<u16, u16>, all: Option<&[u16]>) -> Vec<BTreeSet<u16>> {
    let top = cd.values().copied().max().unwrap_or(0);
    let mut sets = vec![BTreeSet::new(); usize::from(top) + 1];
    for (g, c) in cd {
        if let Some(s) = sets.get_mut(usize::from(*c)) {
            s.insert(*g);
        }
    }
    match all {
        Some(all) => {
            let mut class0: BTreeSet<u16> = all.iter().copied().collect();
            for s in sets.iter_mut().skip(1) {
                s.retain(|g| class0.contains(g));
                for g in s.iter() {
                    class0.remove(g);
                }
            }
            if let Some(first) = sets.first_mut() {
                *first = class0;
            }
            sets
        }
        None => sets.into_iter().skip(1).collect(),
    }
}

struct Aligned {
    cd1: HashMap<u16, u16>,
    cd2: HashMap<u16, u16>,
    class2_count: usize,
    matrices: Vec<Vec<Vec<Option<Values>>>>,
}

/// `_PairPosFormat2_align_matrices`: every subtable's class matrix over the classes that refine
/// all of theirs. A first class a subtable does not cover gets a row of zero records of format
/// `vf`, or of nothing when `transparent`.
fn align(ctx: &Ctx, vf: (u16, u16), lst: &[&Pair], transparent: bool) -> Result<Aligned> {
    let mut parts = Vec::with_capacity(lst.len());
    for p in lst {
        if let Pair::F2 { glyphs, covered, cd1, cd2, class2_count, rows, .. } = p {
            parts.push((glyphs, covered, cd1, cd2, *class2_count, rows));
        } else {
            return Err(bad("pair adjustment subtables of different formats cannot be interpolated"));
        }
    }
    let mut c1 = Classifier::default();
    for (glyphs, _, cd1, ..) in &parts {
        for s in invert(cd1, Some(glyphs)) {
            c1.add(&s);
        }
    }
    let classes1 = c1.classes(ctx);
    let mut c2 = Classifier::default();
    for (_, _, _, cd2, ..) in &parts {
        for s in invert(cd2, None) {
            c2.add(&s);
        }
    }
    let mut classes2 = c2.classes(ctx);
    classes2.insert(0, BTreeSet::new());
    let cells = classes1.len().saturating_mul(classes2.len()).saturating_mul(lst.len().max(1));
    if cells > MAX_LAYOUT_ITEMS {
        return Err(bad(format!("the aligned pair classes need more than {MAX_LAYOUT_ITEMS} records")));
    }
    let numbered = |classes: &[BTreeSet<u16>]| -> HashMap<u16, u16> {
        classes.iter().enumerate().skip(1).flat_map(|(i, s)| s.iter().map(move |g| (*g, u16::try_from(i).unwrap_or(u16::MAX)))).collect()
    };
    let cd1 = numbered(&classes1);
    let cd2 = numbered(&classes2);
    if classes1.len() > usize::from(u16::MAX) || classes2.len() > usize::from(u16::MAX) {
        return Err(bad("the aligned pair classes are too many"));
    }
    let out_of_range = || bad("a pair class is out of range");
    let mut matrices = Vec::with_capacity(parts.len());
    for (_, covered, own_cd1, own_cd2, class2_count, rows) in &parts {
        let mut first = Vec::with_capacity(classes1.len());
        for set in &classes1 {
            let Some(ex) = set.iter().next() else { continue };
            if covered.contains(ex) {
                let k = usize::from(own_cd1.get(ex).copied().unwrap_or(0));
                first.push(rows.get(k).cloned().ok_or_else(out_of_range)?);
            } else if transparent {
                first.push(vec![None; *class2_count]);
            } else {
                first.push(vec![Some((blank(vf.0), blank(vf.1))); *class2_count]);
            }
        }
        let mut matrix = Vec::with_capacity(first.len());
        for old in &first {
            let mut row = Vec::with_capacity(classes2.len());
            for set in &classes2 {
                let k = set.iter().next().map_or(0, |ex| usize::from(own_cd2.get(ex).copied().unwrap_or(0)));
                row.push(*old.get(k).ok_or_else(out_of_range)?);
            }
            matrix.push(row);
        }
        matrices.push(matrix);
    }
    Ok(Aligned { cd1, cd2, class2_count: classes2.len(), matrices })
}

// ---------- joining subtables ----------

/// `_Lookup_PairPosFormat1_subtables_flatten`: format 1 subtables as one; a pair in several keeps
/// the first subtable's values.
fn flatten_f1(lst: &[&Pair]) -> Result<Pair> {
    check_second_formats(lst)?;
    let vf = union_vf(lst);
    let lists: Vec<&[u16]> = lst.iter().map(|p| p.glyphs()).collect();
    let (glyphs, padded) = merge_orders(&lists)?;
    let mut sets = Vec::with_capacity(glyphs.len());
    for j in 0..glyphs.len() {
        let present: Vec<&Vec<Pvr>> = lst
            .iter()
            .zip(&padded)
            .filter_map(|(p, at)| match (p, at.get(j).copied().flatten()) {
                (Pair::F1 { sets, .. }, Some(i)) => sets.get(i),
                _ => None,
            })
            .collect();
        let seconds: Vec<Vec<u16>> = present.iter().map(|s| s.iter().map(|r| r.second).collect()).collect();
        let refs: Vec<&[u16]> = seconds.iter().map(Vec::as_slice).collect();
        let (union, at) = merge_orders(&refs)?;
        let mut recs = Vec::with_capacity(union.len());
        for t in 0..union.len() {
            let rec = present.iter().zip(&at).find_map(|(s, at)| at.get(t).copied().flatten().and_then(|i| s.get(i)));
            recs.push(rec.cloned().ok_or_else(|| bad("a pair is missing from every subtable"))?);
        }
        sets.push(recs);
    }
    Ok(f1(vf, glyphs, sets))
}

/// `_Lookup_PairPosFormat2_subtables_flatten`: format 2 subtables as one; each class pair keeps
/// the first subtable's values that covers it.
fn flatten_f2(ctx: &Ctx, lst: &[&Pair]) -> Result<Pair> {
    check_second_formats(lst)?;
    let vf = union_vf(lst);
    let lists: Vec<&[u16]> = lst.iter().map(|p| p.glyphs()).collect();
    let (glyphs, _) = merge_orders(&lists)?;
    let a = align(ctx, vf, lst, true)?;
    let n1 = a.matrices.first().map_or(0, Vec::len);
    let mut rows = Vec::with_capacity(n1);
    for r in 0..n1 {
        let mut row = Vec::with_capacity(a.class2_count);
        for c in 0..a.class2_count {
            let cell = a.matrices.iter().find_map(|m| m.get(r).and_then(|row| row.get(c)).copied().flatten());
            row.push(Some(cell.ok_or_else(|| bad("a pair class is in no subtable"))?));
        }
        rows.push(row);
    }
    Ok(f2(vf, glyphs, a.cd1, a.cd2, a.class2_count, rows))
}

/// `_Lookup_PairPos_subtables_canonicalize`: the leading format 1 subtables joined into one and
/// the trailing format 2 subtables joined into one (either possibly empty).
fn canonical(ctx: &Ctx, list: &[Pair]) -> Result<Vec<Pair>> {
    let lead = list.iter().take_while(|p| matches!(p, Pair::F1 { .. })).count();
    let head: Vec<&Pair> = list.iter().take(lead).collect();
    let mut out = vec![flatten_f1(&head)?];
    out.extend(list.iter().skip(lead).cloned());
    let trail = out.iter().rev().take_while(|p| matches!(p, Pair::F2 { .. })).count();
    let tail = out.split_off(out.len().saturating_sub(trail));
    let refs: Vec<&Pair> = tail.iter().collect();
    out.push(flatten_f2(ctx, &refs)?);
    Ok(out)
}

// ---------- merging ----------

/// One subtable: the base master's `own` merged with the masters' `lst` (subtable `k` of their
/// lookups `lookups`).
fn merge_one(ctx: &Ctx, own: &Pair, lst: &[&Pair], lookups: &[Vec<Pair>], k: usize) -> Result<Pair> {
    let vf = union_vf(lst);
    if lst.iter().any(|p| std::mem::discriminant(*p) != std::mem::discriminant(own)) {
        return Err(bad("pair adjustment subtables of different formats cannot be interpolated"));
    }
    check_second_formats(lst)?;
    let lists: Vec<&[u16]> = lst.iter().map(|p| p.glyphs()).collect();
    let (glyphs, padded) = merge_orders(&lists)?;
    match own {
        Pair::F1 { .. } => {
            let mut sets = Vec::with_capacity(glyphs.len());
            let mut out_vf = (0, 0);
            for (j, g) in glyphs.iter().enumerate() {
                let own_sets: Vec<&[Pvr]> = lst
                    .iter()
                    .zip(&padded)
                    .map(|(p, at)| match (p, at.get(j).copied().flatten()) {
                        (Pair::F1 { sets, .. }, Some(i)) => sets.get(i).map_or(&[][..], Vec::as_slice),
                        _ => &[][..],
                    })
                    .collect();
                let seconds: Vec<Vec<u16>> = own_sets.iter().map(|s| s.iter().map(|r| r.second).collect()).collect();
                let refs: Vec<&[u16]> = seconds.iter().map(Vec::as_slice).collect();
                let (union, at) = merge_orders(&refs)?;
                let mut recs = Vec::with_capacity(union.len());
                for (t, s) in union.iter().enumerate() {
                    let mut v1 = Vec::with_capacity(lst.len());
                    let mut v2 = Vec::with_capacity(lst.len());
                    for ((set, at), subs) in own_sets.iter().zip(&at).zip(lookups) {
                        let pair = match at.get(t).copied().flatten().and_then(|i| set.get(i)) {
                            Some(r) => Some(r.v),
                            None => effective(subs, *g, *s)?,
                        };
                        let (a, b) = pair.unwrap_or_default();
                        v1.push((vf.0 != 0).then(|| Vr::with_format(vf.0, a.as_ref())));
                        v2.push((vf.1 != 0).then(|| Vr::with_format(vf.1, b.as_ref())));
                    }
                    let a = ctx.value_record(blank(vf.0).as_ref(), &v1)?;
                    let b = ctx.value_record(blank(vf.1).as_ref(), &v2)?;
                    out_vf = (out_vf.0 | a.map_or(0, |v| v.mask), out_vf.1 | b.map_or(0, |v| v.mask));
                    recs.push(Pvr { second: *s, v: (a, b) });
                }
                sets.push(recs);
            }
            Ok(f1(out_vf, glyphs, sets))
        }
        Pair::F2 { .. } => {
            // Rows added for glyphs a master's subtable lacks are not looked up in its later
            // subtables, so such a subtable must be its lookup's last, as fontTools requires.
            for (p, subs) in lst.iter().zip(lookups) {
                if p.glyphs() != glyphs.as_slice() && k.saturating_add(1) != subs.len() {
                    return Err(bad("a class pair adjustment subtable that covers fewer glyphs than other masters' must be its lookup's last"));
                }
            }
            let a = align(ctx, vf, lst, false)?;
            let first = a.matrices.first().ok_or_else(|| bad("no masters"))?;
            let mut rows = Vec::with_capacity(first.len());
            let mut out_vf = (0, 0);
            for (r, row) in first.iter().enumerate() {
                let mut cells = Vec::with_capacity(row.len());
                for (c, cell) in row.iter().enumerate() {
                    let own = cell.ok_or_else(|| bad("a pair class is in no subtable"))?;
                    let mut v1 = Vec::with_capacity(lst.len());
                    let mut v2 = Vec::with_capacity(lst.len());
                    for m in &a.matrices {
                        let (x, y) = m.get(r).and_then(|row| row.get(c)).copied().flatten().ok_or_else(|| bad("a pair class is in no subtable"))?;
                        v1.push(x);
                        v2.push(y);
                    }
                    let x = ctx.value_record(own.0.as_ref(), &v1)?;
                    let y = ctx.value_record(own.1.as_ref(), &v2)?;
                    out_vf = (out_vf.0 | x.map_or(0, |v| v.mask), out_vf.1 | y.map_or(0, |v| v.mask));
                    cells.push(Some((x, y)));
                }
                rows.push(cells);
            }
            Ok(f2(out_vf, glyphs, a.cd1, a.cd2, a.class2_count, rows))
        }
    }
}

/// A pair adjustment lookup's subtables: the base master's `own` merged with each master's.
pub(super) fn merge(ctx: &Ctx, own: &[&PairPos], masters: &[Vec<&PairPos>]) -> Result<Vec<PairPos>> {
    if own.is_empty() {
        if masters.iter().any(|l| !l.is_empty()) {
            return Err(bad("the masters' lookups have different subtable counts"));
        }
        return Ok(Vec::new());
    }
    let own: Vec<Pair> = own.iter().map(|p| Pair::read(ctx, ctx.base, p)).collect::<Result<_>>()?;
    let own = canonical(ctx, &own)?;
    let mut lookups = Vec::with_capacity(masters.len());
    for (m, l) in masters.iter().enumerate() {
        let read: Vec<Pair> = l.iter().map(|p| Pair::read(ctx, m, p)).collect::<Result<_>>()?;
        let c = canonical(ctx, &read)?;
        if c.len() != own.len() {
            return Err(bad("the masters' lookups have different subtable counts"));
        }
        lookups.push(c);
    }
    let mut out = Vec::with_capacity(own.len());
    for (k, o) in own.iter().enumerate() {
        let lst: Vec<&Pair> = lookups.iter().filter_map(|l| l.get(k)).collect();
        out.push(merge_one(ctx, o, &lst, &lookups, k)?);
    }
    if out.first().is_some_and(|p| matches!(p, Pair::F1 { .. }) && p.glyphs().is_empty()) {
        out.remove(0);
    }
    if out.last().is_some_and(|p| matches!(p, Pair::F2 { .. }) && p.glyphs().is_empty()) {
        out.pop();
    }
    Ok(out.iter().map(Pair::write).collect())
}

#[cfg(test)]
pub(super) fn classify_for_test(ctx: &Ctx, sets: &[&[u16]]) -> Vec<Vec<u16>> {
    let mut c = Classifier::default();
    for s in sets {
        c.add(&s.iter().copied().collect());
    }
    c.classes(ctx).into_iter().map(|s| s.into_iter().collect()).collect()
}
