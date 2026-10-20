//! The `gposCompaction` step of `font.optimize`: each class-kerning subtable (pair adjustment
//! format 2) split into smaller, denser subtables when that saves bytes. A port of fontTools'
//! `otlLib.optimize.gpos` (`compact`, `compact_lookup`, `compact_ext_lookup`,
//! `compact_pair_pos`, `compact_class_pairs`, `is_really_zero`, `_classDef_bytes`, `Cluster`
//! and `cluster_pairs_by_class2_coverage_custom_cost`) and of `otlLib.builder`'s
//! `buildPairPosClassesSubtable` and `ClassDefBuilder` (fontTools 4.66.1, MIT License), run as
//! `fonttools otlLib.optimize FONT --gpos-compression-level LEVEL` runs them.
//!
//! Every subtable is handled on its own. Its non-zero class pairs become a matrix with one line
//! per first-glyph class, lines in the order of their sorted glyph names. Lines start as one
//! cluster each and are merged greedily, the merge that changes the estimated size least first
//! (the first such pair of clusters on a tie). Merges that make the estimate smaller always
//! happen. After that, merging stops once there are at most `-log2(after / before) × LEVEL + 1`
//! clusters, `before` being the estimate for one subtable and `after` the clusters' total; level
//! 9 stops as soon as no merge saves bytes. Each cluster becomes a subtable, its classes
//! numbered by size and then by glyph names, the largest first-glyph class being class 0.
//!
//! As in fontTools, a first-glyph class whose values are all zero leaves the coverage, and a
//! glyph pair that used to match it may then reach a later subtable of the same lookup. Where
//! fontTools fails, this step keeps the subtable as it was: a subtable with a non-zero value in
//! the class-0 column (every second glyph not in a class). A class with no glyphs can never
//! match, so its values are dropped.
//!
//! `GPOS` is written again by write-fonts, whose packer moves lookups into extension subtables
//! and splits pair subtables when offsets overflow. A table it cannot pack is kept, with a note.
//!
//! Limits: [`MAX_CLASSES`] first-glyph classes with a value in one subtable; [`MAX_WORK`] for
//! the clustering of the whole table. A subtable past either keeps its layout, with a note.

use std::collections::{BTreeSet, HashMap};

use read_fonts::{FontData, FontRead};
use write_fonts::OffsetMarker;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{
    Class1Record, Class2Record, ExtensionPosFormat1, ExtensionSubtable, Gpos, PairPos, PairPosFormat2, PositionLookup, ValueFormat, ValueRecord,
};
use write_fonts::tables::layout::{ClassDef, CoverageTable};
use write_fonts::types::GlyphId16;

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most first-glyph classes with a non-zero value in one subtable; the clustering keeps a
/// matrix of this size squared.
pub const MAX_CLASSES: usize = 2_048;
/// Most work (clusters compared, lines, ranges and column words visited) for one table.
pub const MAX_WORK: u64 = 1 << 32;
/// The highest level; it splits as long as splitting saves bytes.
pub const MAX_LEVEL: u8 = 9;

/// The result of compacting `GPOS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compacted {
    /// The table written again; `None` when no subtable could be compacted.
    pub table: Option<Vec<u8>>,
    /// Class-kerning subtables before and the subtables they became.
    pub split: usize,
    pub produced: usize,
    /// Subtables kept as they were, one sentence each.
    pub notes: Vec<String>,
}

/// Compacts the class kerning of `font`'s `GPOS` at `level` (1 to 9). `None` when the font has
/// no `GPOS` or no class-kerning subtable.
pub fn run(font: &SfntFont, level: u8) -> Result<Option<Compacted>> {
    if !(1..=MAX_LEVEL).contains(&level) {
        return Err(TablesError::BadOptions(format!("the GPOS compaction level must be 1 to {MAX_LEVEL}, not {level}")));
    }
    let Some(data) = font.get(*b"GPOS") else { return Ok(None) };
    crate::layout_budget::check_gpos(data)?;
    let read = read_fonts::tables::gpos::Gpos::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the GPOS table: {e}")))?;
    let mut gpos: Gpos = read.to_owned_table();
    crate::layout_budget::restore_empty_pair_records(&mut gpos)?;
    let names = glyph_names(font);
    let mut job = Job { names: &names, level, work: 0, split: 0, produced: 0, notes: Vec::new() };
    for (index, lookup) in gpos.lookup_list.lookups.iter_mut().enumerate() {
        match &mut **lookup {
            PositionLookup::Pair(l) => {
                let old = std::mem::take(&mut l.subtables);
                for sub in old {
                    for new in job.subtable(index, sub.into_inner()) {
                        l.subtables.push(OffsetMarker::new(new));
                    }
                }
            }
            PositionLookup::Extension(l) if l.subtables.iter().all(|s| matches!(&**s, ExtensionSubtable::Pair(_))) => {
                let old = std::mem::take(&mut l.subtables);
                for sub in old {
                    let ExtensionSubtable::Pair(ext) = sub.into_inner() else { continue };
                    for new in job.subtable(index, ext.extension.into_inner()) {
                        l.subtables.push(OffsetMarker::new(ExtensionSubtable::Pair(ExtensionPosFormat1::new(2, new))));
                    }
                }
            }
            _ => {}
        }
    }
    if job.split == 0 {
        if job.notes.is_empty() {
            return Ok(None);
        }
        return Ok(Some(Compacted { table: None, split: 0, produced: 0, notes: job.notes }));
    }
    check_class_counts(&gpos)?;
    let table = write_fonts::dump_table(&gpos).map_err(|e| TablesError::invalid(format!("GPOS could not be written again: {e}")))?;
    let lookups = read_fonts::tables::gpos::Gpos::read(FontData::new(&table))
        .and_then(|t| t.lookup_list())
        .map(|l| l.lookup_offsets().len())
        .map_err(|e| TablesError::invalid(format!("the compacted GPOS does not read back: {e}")))?;
    if lookups != gpos.lookup_list.lookups.len() {
        return Err(TablesError::invalid("the compacted GPOS reads back with a different number of lookups"));
    }
    Ok(Some(Compacted { table: Some(table), split: job.split, produced: job.produced, notes: job.notes }))
}

/// fontTools' glyph names, which order the classes; a glyph without one gets `glyphNNNNN`.
fn glyph_names(font: &SfntFont) -> Vec<String> {
    crate::ttx::glyph_order(font).unwrap_or_default()
}

fn name_of(names: &[String], gid: u16) -> std::borrow::Cow<'_, str> {
    match names.get(usize::from(gid)) {
        Some(n) => std::borrow::Cow::Borrowed(n.as_str()),
        None => std::borrow::Cow::Owned(format!("glyph{gid:05}")),
    }
}

/// write-fonts writes a class count as a 16-bit number of distinct classes, class 0 included.
fn check_class_counts(gpos: &Gpos) -> Result<()> {
    let fits = |cd: &ClassDef| cd.iter().map(|(_, c)| c).chain(std::iter::once(0)).collect::<BTreeSet<u16>>().len() <= usize::from(u16::MAX);
    let pair_fits = |p: &PairPos| match p {
        PairPos::Format1(_) => true,
        PairPos::Format2(f) => fits(&f.class_def1) && fits(&f.class_def2),
    };
    for lookup in &gpos.lookup_list.lookups {
        let ok = match &**lookup {
            PositionLookup::Pair(l) => l.subtables.iter().all(|s| pair_fits(s)),
            PositionLookup::Extension(l) => l.subtables.iter().all(|s| match &**s {
                ExtensionSubtable::Pair(e) => pair_fits(&e.extension),
                _ => true,
            }),
            _ => true,
        };
        if !ok {
            return Err(TablesError::invalid("a GPOS class definition holds more than 65,535 classes"));
        }
    }
    Ok(())
}

struct Job<'a> {
    names: &'a [String],
    level: u8,
    work: u64,
    split: usize,
    produced: usize,
    notes: Vec<String>,
}

impl Job<'_> {
    /// The subtables `sub` becomes: itself when it is not class kerning or cannot be compacted.
    fn subtable(&mut self, lookup: usize, sub: PairPos) -> Vec<PairPos> {
        let f = match sub {
            PairPos::Format2(f) => f,
            other => return vec![other],
        };
        match self.compact(&f) {
            Ok(new) => {
                self.split += 1;
                self.produced += new.len();
                new.into_iter().map(PairPos::Format2).collect()
            }
            Err(why) => {
                self.notes.push(format!("GPOS lookup {lookup}: a class-kerning subtable was kept as it was: {why}"));
                vec![PairPos::Format2(f)]
            }
        }
    }

    fn compact(&mut self, f: &PairPosFormat2) -> std::result::Result<Vec<PairPosFormat2>, String> {
        let pairs = Pairs::read(f, self.names)?;
        if pairs.lines.len() > MAX_CLASSES {
            return Err(format!("it has {} first-glyph classes with values, more than {MAX_CLASSES}", pairs.lines.len()));
        }
        let groups = cluster(&pairs, self.level, &mut self.work)?;
        Ok(groups.iter().map(|g| pairs.build(g, self.names)).collect())
    }
}

/// One subtable's non-zero class pairs. Lines are first-glyph classes, columns second-glyph
/// classes, each sorted by its glyph names.
struct Pairs {
    /// Each line's glyphs, sorted by name.
    lines: Vec<Vec<u16>>,
    columns: Vec<Vec<u16>>,
    /// Each line's columns with a value, ascending.
    line_columns: Vec<Vec<usize>>,
    /// (line, column) → the two value records.
    values: HashMap<(usize, usize), (ValueRecord, ValueRecord)>,
    format1: ValueFormat,
    format2: ValueFormat,
    /// Bytes of one class record: the value fields any pair sets, counted as fontTools'
    /// `getEffectiveFormat` counts them.
    record_bytes: u64,
}

/// The fields of `v` that are set: non-zero numbers and device offsets.
fn effective_format(v: &ValueRecord) -> u16 {
    let num = |x: Option<i16>, bit: u16| if x.is_some_and(|x| x != 0) { bit } else { 0 };
    let dev = |set: bool, bit: u16| if set { bit } else { 0 };
    num(v.x_placement, 0x01)
        | num(v.y_placement, 0x02)
        | num(v.x_advance, 0x04)
        | num(v.y_advance, 0x08)
        | dev(v.x_placement_device.is_some(), 0x10)
        | dev(v.y_placement_device.is_some(), 0x20)
        | dev(v.x_advance_device.is_some(), 0x40)
        | dev(v.y_advance_device.is_some(), 0x80)
}

/// `a` before `b` when its glyph names sort first (Python's order of name tuples).
fn by_names(names: &[String], a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    a.iter().map(|&g| name_of(names, g)).cmp(b.iter().map(|&g| name_of(names, g))).then_with(|| a.cmp(b))
}

fn sorted_by_name(names: &[String], mut glyphs: Vec<u16>) -> Vec<u16> {
    glyphs.sort_by(|&a, &b| name_of(names, a).cmp(&name_of(names, b)).then(a.cmp(&b)));
    glyphs
}

impl Pairs {
    fn read(f: &PairPosFormat2, names: &[String]) -> std::result::Result<Self, String> {
        let mut class1: HashMap<u16, Vec<u16>> = HashMap::new();
        for g in f.coverage.iter() {
            class1.entry(f.class_def1.get(g)).or_default().push(g.to_u16());
        }
        let mut class2: HashMap<u16, Vec<u16>> = HashMap::new();
        let classified: BTreeSet<GlyphId16> = f.class_def2.iter().map(|(g, _)| g).collect();
        for g in classified {
            let c = f.class_def2.get(g);
            if c != 0 {
                class2.entry(c).or_default().push(g.to_u16());
            }
        }
        let format_of = |r: Option<&Class2Record>, second: bool| r.map(|r| if second { r.value_record2.format() } else { r.value_record1.format() });
        let first = f.class1_records.first().and_then(|r| r.class2_records.first());
        let format1 = format_of(first, false).unwrap_or(ValueFormat::empty());
        let format2 = format_of(first, true).unwrap_or(ValueFormat::empty());

        let mut raw: Vec<(u16, u16, &Class2Record)> = Vec::new();
        let (mut effective1, mut effective2) = (0u16, 0u16);
        for (i, row) in f.class1_records.iter().enumerate() {
            let Ok(i) = u16::try_from(i) else { break };
            for (j, rec) in row.class2_records.iter().enumerate() {
                let Ok(j) = u16::try_from(j) else { break };
                let (e1, e2) = (effective_format(&rec.value_record1), effective_format(&rec.value_record2));
                if e1 == 0 && e2 == 0 {
                    continue;
                }
                if j == 0 {
                    return Err("it has a non-zero value for second glyphs outside every class".into());
                }
                if !class1.contains_key(&i) || !class2.contains_key(&j) {
                    continue;
                }
                effective1 |= e1;
                effective2 |= e2;
                raw.push((i, j, rec));
            }
        }
        let order = |map: &HashMap<u16, Vec<u16>>, used: BTreeSet<u16>| -> Vec<(u16, Vec<u16>)> {
            let mut v: Vec<(u16, Vec<u16>)> = used.into_iter().filter_map(|c| map.get(&c).map(|g| (c, sorted_by_name(names, g.clone())))).collect();
            v.sort_by(|a, b| by_names(names, &a.1, &b.1));
            v
        };
        let lines = order(&class1, raw.iter().map(|r| r.0).collect());
        let columns = order(&class2, raw.iter().map(|r| r.1).collect());
        let line_of: HashMap<u16, usize> = lines.iter().enumerate().map(|(k, (c, _))| (*c, k)).collect();
        let column_of: HashMap<u16, usize> = columns.iter().enumerate().map(|(k, (c, _))| (*c, k)).collect();
        let mut values = HashMap::new();
        let mut line_columns = vec![Vec::new(); lines.len()];
        for (i, j, rec) in raw {
            if let (Some(&l), Some(&c)) = (line_of.get(&i), column_of.get(&j)) {
                values.insert((l, c), (rec.value_record1.clone(), rec.value_record2.clone()));
                if let Some(cols) = line_columns.get_mut(l) {
                    cols.push(c);
                }
            }
        }
        for cols in &mut line_columns {
            cols.sort_unstable();
        }
        let record_bytes = 2 * u64::from(effective1.count_ones() + effective2.count_ones());
        Ok(Pairs {
            lines: lines.into_iter().map(|(_, g)| g).collect(),
            columns: columns.into_iter().map(|(_, g)| g).collect(),
            line_columns,
            values,
            format1,
            format2,
            record_bytes,
        })
    }

    /// The subtable of the pairs whose first-glyph class is one of `lines`, as
    /// `buildPairPosClassesSubtable` builds it.
    fn build(&self, lines: &[usize], names: &[String]) -> PairPosFormat2 {
        let used: BTreeSet<usize> = lines.iter().filter_map(|&l| self.line_columns.get(l)).flatten().copied().collect();
        let by_size = |a: &(usize, &Vec<u16>), b: &(usize, &Vec<u16>)| b.1.len().cmp(&a.1.len()).then_with(|| by_names(names, a.1, b.1));
        let mut ones: Vec<(usize, &Vec<u16>)> = lines.iter().filter_map(|&l| self.lines.get(l).map(|g| (l, g))).collect();
        ones.sort_by(by_size);
        let mut twos: Vec<(usize, &Vec<u16>)> = used.iter().filter_map(|&c| self.columns.get(c).map(|g| (c, g))).collect();
        twos.sort_by(by_size);

        let coverage: CoverageTable = ones.iter().flat_map(|(_, g)| g.iter().map(|&g| GlyphId16::new(g))).collect::<Vec<_>>().into();
        let class_def1: ClassDef = ones
            .iter()
            .enumerate()
            .skip(1)
            .flat_map(|(id, (_, g))| g.iter().map(move |&g| (GlyphId16::new(g), u16::try_from(id).unwrap_or(u16::MAX))))
            .collect();
        let class_def2: ClassDef = twos
            .iter()
            .enumerate()
            .flat_map(|(id, (_, g))| g.iter().map(move |&g| (GlyphId16::new(g), u16::try_from(id + 1).unwrap_or(u16::MAX))))
            .collect();
        let empty = |format: ValueFormat| ValueRecord::new().with_explicit_value_format(format);
        let row = |line: Option<usize>| {
            let mut records = vec![Class2Record::new(empty(self.format1), empty(self.format2))];
            for (c, _) in &twos {
                let rec = match line.and_then(|l| self.values.get(&(l, *c))) {
                    Some((v1, v2)) => Class2Record::new(v1.clone(), v2.clone()),
                    None => Class2Record::new(empty(self.format1), empty(self.format2)),
                };
                records.push(rec);
            }
            Class1Record::new(records)
        };
        // An empty subtable still has class 0, which write-fonts counts.
        let records = if ones.is_empty() { vec![row(None)] } else { ones.iter().map(|(l, _)| row(Some(*l))).collect() };
        PairPosFormat2::new(coverage, class_def1, class_def2, records)
    }
}

/// A glyph set's runs of consecutive ids, its smallest and its largest id.
fn ranges(glyphs: &[u16]) -> (Vec<(u32, u32)>, u32, u32) {
    let mut ids: Vec<u32> = glyphs.iter().map(|&g| u32::from(g)).collect();
    ids.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for g in &ids {
        match out.last_mut() {
            Some((_, end)) if *end + 1 == *g => *end = *g,
            _ => out.push((*g, *g)),
        }
    }
    (out, ids.first().copied().unwrap_or(0), ids.last().copied().unwrap_or(0))
}

/// What the cost estimate needs of the lines and columns.
struct Ctx {
    /// Each line's columns as a bit set.
    bits: Vec<Vec<u64>>,
    words: usize,
    line_len: Vec<u64>,
    line_ranges: Vec<Vec<(u32, u32)>>,
    line_span: Vec<(u32, u32)>,
    column_ranges: Vec<u64>,
    column_span: Vec<(u32, u32)>,
    record_bytes: u64,
}

impl Ctx {
    fn new(p: &Pairs) -> Self {
        let words = p.columns.len().div_ceil(64);
        let mut bits = vec![vec![0u64; words]; p.lines.len()];
        for (b, cols) in bits.iter_mut().zip(&p.line_columns) {
            for &c in cols {
                if let Some(w) = b.get_mut(c / 64) {
                    *w |= 1 << (c % 64);
                }
            }
        }
        let lines: Vec<_> = p.lines.iter().map(|g| ranges(g)).collect();
        let columns: Vec<_> = p.columns.iter().map(|g| ranges(g)).collect();
        Ctx {
            bits,
            words,
            line_len: p.lines.iter().map(|g| g.len() as u64).collect(),
            line_span: lines.iter().map(|r| (r.1, r.2)).collect(),
            line_ranges: lines.into_iter().map(|r| r.0).collect(),
            column_ranges: columns.iter().map(|r| r.0.len() as u64).collect(),
            column_span: columns.iter().map(|r| (r.1, r.2)).collect(),
            record_bytes: p.record_bytes,
        }
    }

    /// `_classDef_bytes`: the smaller of the two class definition formats for these classes.
    fn class_def_bytes(items: impl Iterator<Item = (u64, (u32, u32))>) -> u64 {
        let mut any = false;
        let (mut count, mut lo, mut hi) = (0u64, u32::MAX, 0u32);
        for (n, (a, b)) in items {
            any = true;
            count += n;
            lo = lo.min(a);
            hi = hi.max(b);
        }
        if !any {
            return 0;
        }
        let format1 = 6 + 2 * (u64::from(hi.saturating_sub(lo)) + 1);
        let format2 = 4 + 6 * count;
        format1.min(format2)
    }

    /// A cluster of `lines` (ascending) with its cost.
    fn cluster(&self, lines: Vec<usize>, work: &mut u64) -> Cluster {
        let mut columns = vec![0u64; self.words];
        let mut glyphs = 0;
        let mut coverage: Vec<(u32, u32)> = Vec::new();
        for &l in &lines {
            if let Some(b) = self.bits.get(l) {
                for (w, x) in columns.iter_mut().zip(b) {
                    *w |= x;
                }
            }
            glyphs += self.line_len.get(l).copied().unwrap_or(0);
            coverage.extend(self.line_ranges.get(l).into_iter().flatten());
        }
        coverage.sort_unstable();
        *work += (lines.len() * (self.words + 1) + coverage.len()) as u64;
        let mut c = Cluster { lines, columns, coverage, glyphs, cost: 0 };
        c.cost = self.cost(&c, work);
        c
    }

    /// Two clusters merged.
    fn merge(&self, a: &Cluster, b: &Cluster, work: &mut u64) -> Cluster {
        let mut lines = Vec::with_capacity(a.lines.len() + b.lines.len());
        let (mut i, mut j) = (a.lines.iter().peekable(), b.lines.iter().peekable());
        while let (Some(&&x), Some(&&y)) = (i.peek(), j.peek()) {
            if x < y {
                lines.push(x);
                i.next();
            } else {
                lines.push(y);
                j.next();
            }
        }
        lines.extend(i);
        lines.extend(j);
        let columns = a.columns.iter().zip(&b.columns).map(|(x, y)| x | y).collect();
        let mut coverage = Vec::with_capacity(a.coverage.len() + b.coverage.len());
        coverage.extend_from_slice(&a.coverage);
        coverage.extend_from_slice(&b.coverage);
        coverage.sort_unstable();
        *work += (lines.len() + self.words + coverage.len()) as u64;
        let mut c = Cluster { lines, columns, coverage, glyphs: a.glyphs + b.glyphs, cost: 0 };
        c.cost = self.cost(&c, work);
        c
    }

    /// `Cluster.cost`: the estimated bytes of the cluster's subtable, its lookup offset included.
    fn cost(&self, c: &Cluster, work: &mut u64) -> i64 {
        let format1 = 4 + c.glyphs * 2;
        // fontTools counts the breaks between ranges, one fewer than the ranges.
        let breaks = c.coverage.windows(2).filter(|w| matches!(w, [(_, end), (start, _)] if *start != end + 1)).count() as u64;
        let coverage = format1.min(4 + breaks * 6);
        // The largest first-glyph class (the first of equals) becomes class 0 and is not listed.
        let mut biggest: Option<(usize, u64)> = None;
        for &l in &c.lines {
            let n = self.line_len.get(l).copied().unwrap_or(0);
            if biggest.is_none_or(|(_, m)| n > m) {
                biggest = Some((l, n));
            }
        }
        let skip = biggest.map(|(l, _)| l);
        let class_def1 = Self::class_def_bytes(
            c.lines
                .iter()
                .filter(|&&l| Some(l) != skip)
                .map(|&l| (self.line_ranges.get(l).map_or(0, |r| r.len() as u64), self.line_span.get(l).copied().unwrap_or((0, 0)))),
        );
        let mut width = 1u64;
        let set = c.columns.iter().enumerate().flat_map(|(w, &x)| {
            let mut x = x;
            std::iter::from_fn(move || {
                let b = (x != 0).then_some(x.trailing_zeros() as usize)?;
                x &= x - 1;
                Some(w * 64 + b)
            })
        });
        let class_def2 = Self::class_def_bytes(
            set.inspect(|_| width += 1)
                .map(|col| (self.column_ranges.get(col).copied().unwrap_or(0), self.column_span.get(col).copied().unwrap_or((0, 0)))),
        );
        *work += (c.lines.len() + c.coverage.len() + self.words) as u64 + width;
        let records = self.record_bytes * c.lines.len() as u64 * width;
        i64::try_from(18 + coverage + class_def1 + class_def2 + records).unwrap_or(i64::MAX)
    }
}

struct Cluster {
    lines: Vec<usize>,
    columns: Vec<u64>,
    coverage: Vec<(u32, u32)>,
    glyphs: u64,
    cost: i64,
}

/// The clustering of `cluster_pairs_by_class2_coverage_custom_cost`: groups of lines, each
/// one subtable, in fontTools' order.
fn cluster(p: &Pairs, level: u8, work: &mut u64) -> std::result::Result<Vec<Vec<usize>>, String> {
    let n = p.lines.len();
    if n == 0 {
        return Ok(vec![Vec::new()]);
    }
    let over = || format!("compacting it would take more than {MAX_WORK} units of work");
    if *work > MAX_WORK {
        return Err(over());
    }
    let ctx = Ctx::new(p);
    let before = ctx.cluster((0..n).collect(), work).cost;
    let mut slots: Vec<Option<Cluster>> = (0..n).map(|l| Some(ctx.cluster(vec![l], work))).collect();
    // `change[i * n + j]` (i < j): the cost change of merging slots i and j. Merging keeps the
    // lower slot, so the live slots stay in fontTools' cluster order.
    let mut change = vec![0i64; n * n];
    let delta = |slots: &[Option<Cluster>], i: usize, j: usize, work: &mut u64| -> Option<i64> {
        let (Some(Some(a)), Some(Some(b))) = (slots.get(i), slots.get(j)) else { return None };
        Some(ctx.merge(a, b, work).cost - a.cost - b.cost)
    };
    for i in 0..n {
        for j in i + 1..n {
            if let (Some(d), Some(cell)) = (delta(&slots, i, j, work), change.get_mut(i * n + j)) {
                *cell = d;
            }
        }
        if *work > MAX_WORK {
            return Err(over());
        }
    }
    // Each live row's smallest change and the first column holding it.
    let row_min = |slots: &[Option<Cluster>], change: &[i64], i: usize, work: &mut u64| -> Option<(i64, usize)> {
        *work += (n - i) as u64;
        let mut best: Option<(i64, usize)> = None;
        for j in i + 1..n {
            if matches!(slots.get(j), Some(Some(_)))
                && let Some(&d) = change.get(i * n + j)
                && best.is_none_or(|(m, _)| d < m)
            {
                best = Some((d, j));
            }
        }
        best
    };
    let mut mins: Vec<Option<(i64, usize)>> = (0..n).map(|i| row_min(&slots, &change, i, work)).collect();
    let mut live = n;
    while live > 1 {
        *work += n as u64;
        if *work > MAX_WORK {
            return Err(over());
        }
        let mut best: Option<(i64, usize, usize)> = None;
        for (i, m) in mins.iter().enumerate() {
            if let (Some(Some(_)), Some((d, j))) = (slots.get(i), m)
                && best.is_none_or(|(b, _, _)| *d < b)
            {
                best = Some((*d, i, *j));
            }
        }
        let Some((lowest, bi, bj)) = best else { break };
        if lowest > 0 {
            let after: i64 = slots.iter().flatten().map(|c| c.cost).sum();
            let size_reduction = 1.0 - after as f64 / before as f64;
            let mut max_new = -(1.0 - size_reduction).log2() * f64::from(level);
            if level == MAX_LEVEL {
                max_new = live as f64;
            }
            if live as f64 <= max_new + 1.0 {
                break;
            }
        }
        let (Some(Some(a)), Some(Some(b))) = (slots.get(bi), slots.get(bj)) else { break };
        let merged = ctx.merge(a, b, work);
        if let Some(s) = slots.get_mut(bi) {
            *s = Some(merged);
        }
        if let Some(s) = slots.get_mut(bj) {
            *s = None;
        }
        live -= 1;
        // Changes against the merged slot.
        for i in 0..n {
            if i == bi || !matches!(slots.get(i), Some(Some(_))) {
                continue;
            }
            let (lo, hi) = if i < bi { (i, bi) } else { (bi, i) };
            if let (Some(d), Some(cell)) = (delta(&slots, lo, hi, work), change.get_mut(lo * n + hi)) {
                *cell = d;
            }
        }
        // Rows that pointed at either slot are searched again; rows above the merged slot may
        // find their new smallest change in it.
        for i in 0..n {
            if !matches!(slots.get(i), Some(Some(_))) {
                continue;
            }
            let current = mins.get(i).copied().flatten();
            let again = i == bi || current.is_some_and(|(_, j)| j == bi || j == bj);
            let next = if again {
                row_min(&slots, &change, i, work)
            } else if i < bi {
                let d = change.get(i * n + bi).copied().unwrap_or(i64::MAX);
                match current {
                    Some((m, j)) if d > m || (d == m && j < bi) => current,
                    _ => Some((d, bi)),
                }
            } else {
                current
            };
            if let Some(m) = mins.get_mut(i) {
                *m = next;
            }
        }
    }
    Ok(slots.into_iter().flatten().map(|c| c.lines).collect())
}

#[cfg(test)]
#[path = "gpos_tests.rs"]
mod tests;
