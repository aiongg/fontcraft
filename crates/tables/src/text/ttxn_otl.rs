//! `GSUB` and `GPOS` in the normalised dump: the layout as the feature-file-like text AFDKO
//! `ttxn` writes inside the table element.
//!
//! Ported from `ttxn.py` of AFDKO 5.0.1 (`OTLConverter` and the rule functions; Apache-2.0, Copyright
//! 2014 Adobe; see `NOTICE`). The text follows the oracle line for line, including its quirks,
//! because the normalised dump is only useful when it compares equal to the oracle's:
//!
//! - `languagesystem` lines: per script, the default language system goes to the front of the
//!   list, so the last script's comes first. Without a `DFLT` script, `DFLT dflt` is added when
//!   some feature is used by as many language systems as there are scripts.
//! - Glyph classes are named from their sorted members (`@first_count_last`); classes of one
//!   glyph are written inline as `[glyph]`. Mark classes are named the same way with `_mark`.
//! - Features are written in tag order; inside one, the default language system's lookups come
//!   first, then each other language system's lookups that the default does not use. A lookup is
//!   written in full the first time and referred to by name after that.
//! - Rules of a subtable are sorted as text. Contextual rules print their context only (the
//!   oracle never expands the nested lookups), with the oracle's own punctuation per format.
//!
//! Where the oracle stops with a Python error (format-3 anchors, `GPOS` chaining contexts of
//! format 2, class indices without glyphs), this writer goes on: format-3 anchors print their
//! coordinates, the chaining rule prints as the `GSUB` one does, and an empty class prints as
//! `@empty`.

use std::collections::{BTreeMap, HashMap, HashSet};

use read_fonts::tables::gpos::{AnchorTable, PairPos, PositionSubtables, SinglePos, ValueRecord};
use read_fonts::tables::gsub::{SingleSubst, SubstitutionSubtables};
use read_fonts::tables::layout::{ChainedSequenceContext, ClassDef, CoverageTable, SequenceContext};
use read_fonts::{ReadError, TableProvider};

use super::Out;
use crate::features::{Budget, DFLT_LANG, GSUB, Provider, feature_at, language_systems, layout_lists, read_err};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

const INDENT: &str = "  ";
const WIDTH: usize = 80;

/// Writes the body of the `GSUB` or `GPOS` element (indented for its place in the document).
pub(crate) fn write(font: &SfntFont, table: Tag, show_extensions: bool, out: &mut Out) -> Result<()> {
    let names = crate::ttx::glyph_order(font)?;
    let provider = Provider(font);
    let mut budget = Budget::new();
    let Some(model) = read_table(&provider, table, &names, &mut budget)? else { return Ok(()) };
    let mut conv = Converter::new(&model, table == GSUB, show_extensions, &names);
    conv.run(out, &mut budget)
}

// ── the table, read ─────────────────────────────────────────────────────────────────────────────

struct Model {
    /// (script, language, required feature or 0xFFFF, feature indices), per script the default
    /// language system first, as stored.
    systems: Vec<(String, String, u16, Vec<u16>)>,
    scripts: usize,
    /// Tag and lookup indices of each feature record.
    features: Vec<(String, Vec<u16>)>,
    lookups: Vec<Lookup>,
}

struct Lookup {
    /// The type as stored (the Extension type for extension lookups).
    stored: u16,
    flag: u16,
    subtables: Vec<Sub>,
}

type Classes = Vec<(String, u16)>;

enum Sub {
    Single(Vec<(String, String)>),
    Multiple(Vec<(String, Vec<String>)>),
    Alternate(Vec<(String, Vec<String>)>),
    Ligature(Vec<(String, Vec<String>, String)>),
    Reverse {
        back: Vec<Vec<String>>,
        cov: Vec<String>,
        subs: Vec<String>,
        look: Vec<Vec<String>>,
    },
    Ctx(Ctx),
    Chain(Chain),
    SinglePos1 {
        cov: Vec<String>,
        value: String,
    },
    SinglePos2(Vec<(String, String)>),
    /// First glyph, second glyph, first value, second value (`None` when the format is 0).
    Pair1(Vec<(String, String, Option<String>, Option<String>)>),
    Pair2 {
        cov: Vec<String>,
        cd1: Classes,
        cd2: Classes,
        values: Vec<Vec<(Option<String>, Option<String>)>>,
    },
    Cursive(Vec<(String, String, String)>),
    /// Marks (glyph, class, anchor) and the bases with their anchors by class index.
    MarkBase {
        marks: Vec<(String, u16, String)>,
        bases: Vec<(String, Vec<String>)>,
    },
    MarkLig {
        marks: Vec<(String, u16, String)>,
        ligs: Vec<(String, Vec<Vec<String>>)>,
    },
    MarkMark {
        marks: Vec<(String, u16, String)>,
        bases: Vec<(String, Vec<String>)>,
    },
}

enum Ctx {
    F1(Vec<(String, Vec<Vec<String>>)>),
    F2 { cd: Classes, sets: Vec<Option<Vec<Vec<u16>>>> },
    F3(Vec<Vec<String>>),
}

type ChainRule<T> = (Vec<T>, Vec<T>, Vec<T>);

enum Chain {
    F1(Vec<(String, Vec<ChainRule<String>>)>),
    F2 { back: Option<Classes>, input: Option<Classes>, look: Option<Classes>, sets: Vec<Option<Vec<ChainRule<u16>>>> },
    F3 { back: Vec<Vec<String>>, input: Vec<Vec<String>>, look: Vec<Vec<String>> },
}

struct Reader<'n, 'b> {
    names: &'n [String],
    budget: &'b mut Budget,
    what: String,
}

impl Reader<'_, '_> {
    fn name(&self, g: u16) -> String {
        self.names.get(usize::from(g)).cloned().unwrap_or_else(|| format!("glyph{g:05}"))
    }

    fn err(&self, e: ReadError) -> TablesError {
        read_err(&self.what, e)
    }

    fn coverage(&mut self, cov: std::result::Result<CoverageTable, ReadError>) -> Result<Vec<String>> {
        let cov = cov.map_err(|e| self.err(e))?;
        let mut out = Vec::new();
        for g in cov.iter() {
            self.budget.charge(1)?;
            out.push(self.name(g.to_u16()));
        }
        Ok(out)
    }

    fn classes(&mut self, cd: std::result::Result<ClassDef, ReadError>) -> Result<Classes> {
        let cd = cd.map_err(|e| self.err(e))?;
        let mut out = Vec::new();
        for (g, class) in cd.iter() {
            self.budget.charge(1)?;
            if class != 0 {
                out.push((self.name(g.to_u16()), class));
            }
        }
        Ok(out)
    }

    fn ids(&mut self, ids: &[read_fonts::types::BigEndian<read_fonts::types::GlyphId16>]) -> Result<Vec<String>> {
        self.budget.charge(ids.len())?;
        Ok(ids.iter().map(|g| self.name(g.get().to_u16())).collect())
    }
}

fn anchor(a: Option<std::result::Result<AnchorTable, ReadError>>) -> String {
    match a {
        Some(Ok(AnchorTable::Format1(f))) => format!("<anchor {} {}>", f.x_coordinate(), f.y_coordinate()),
        Some(Ok(AnchorTable::Format2(f))) => format!("<anchor {} {} {}>", f.x_coordinate(), f.y_coordinate(), f.anchor_point()),
        Some(Ok(AnchorTable::Format3(f))) => format!("<anchor {} {}>", f.x_coordinate(), f.y_coordinate()),
        _ => "<anchor NULL>".into(),
    }
}

/// `getValRec`: the value as the oracle prints it under the bits of `format`.
fn value(format: u16, v: &ValueRecord) -> String {
    let (xp, yp, xa, ya) = (v.x_placement().unwrap_or(0), v.y_placement().unwrap_or(0), v.x_advance().unwrap_or(0), v.y_advance().unwrap_or(0));
    if format == 4 {
        return xa.to_string();
    }
    let pick = |bit: u16, n: i16| if format & bit != 0 { n } else { 0 };
    let (xp, yp, xa, ya) = (pick(1, xp), pick(2, yp), pick(4, xa), pick(8, ya));
    if xp == 0 && yp == 0 && xa == 0 && ya == 0 {
        return "0".into();
    }
    format!("<{xp} {yp} {xa} {ya}>")
}

fn tag_text(tag: Tag) -> String {
    String::from_utf8_lossy(&tag).into_owned()
}

fn read_table(p: &Provider, table: Tag, names: &[String], budget: &mut Budget) -> Result<Option<Model>> {
    let Some((scripts, features)) = layout_lists(p, table)? else { return Ok(None) };
    let systems: Vec<(String, String, u16, Vec<u16>)> = language_systems(&scripts, budget)?
        .into_iter()
        .map(|s| (tag_text(s.script), tag_text(s.language), s.required.unwrap_or(0xFFFF), s.features))
        .collect();
    let script_count = scripts.script_records().len();
    let mut feats = Vec::new();
    for i in 0..features.feature_records().len() {
        let (tag, lookups) = feature_at(&features, u16::try_from(i).unwrap_or(u16::MAX), budget)?;
        feats.push((tag_text(tag), lookups));
    }
    let mut lookups = Vec::new();
    let mut r = Reader { names, budget, what: String::new() };
    if table == GSUB {
        let list = p.gsub().and_then(|t| t.lookup_list()).map_err(|e| read_err("GSUB lookup list", e))?;
        for (i, lookup) in list.lookups().iter().enumerate() {
            r.what = format!("GSUB lookup {i}");
            let lookup = lookup.map_err(|e| r.err(e))?;
            let subtables = lookup.subtables().map_err(|e| r.err(e))?;
            lookups.push(Lookup {
                stored: lookup.lookup_type(),
                flag: lookup.lookup_flag().to_bits(),
                subtables: gsub_subtables(&mut r, &subtables)?,
            });
        }
    } else {
        let list = p.gpos().and_then(|t| t.lookup_list()).map_err(|e| read_err("GPOS lookup list", e))?;
        for (i, lookup) in list.lookups().iter().enumerate() {
            r.what = format!("GPOS lookup {i}");
            let lookup = lookup.map_err(|e| r.err(e))?;
            let subtables = lookup.subtables().map_err(|e| r.err(e))?;
            lookups.push(Lookup {
                stored: lookup.lookup_type(),
                flag: lookup.lookup_flag().to_bits(),
                subtables: gpos_subtables(&mut r, &subtables)?,
            });
        }
    }
    Ok(Some(Model { systems, scripts: script_count, features: feats, lookups }))
}

fn gsub_subtables(r: &mut Reader, subtables: &SubstitutionSubtables) -> Result<Vec<Sub>> {
    let mut out = Vec::new();
    match subtables {
        SubstitutionSubtables::Single(s) => {
            for st in s.iter() {
                let pairs = match st.map_err(|e| r.err(e))? {
                    SingleSubst::Format1(f) => {
                        let delta = i32::from(f.delta_glyph_id());
                        let cov = f.coverage().map_err(|e| r.err(e))?;
                        let mut v = Vec::new();
                        for g in cov.iter() {
                            r.budget.charge(1)?;
                            let to = u16::try_from((i32::from(g.to_u16()) + delta).rem_euclid(65_536)).unwrap_or(0);
                            v.push((r.name(g.to_u16()), r.name(to)));
                        }
                        v
                    }
                    SingleSubst::Format2(f) => {
                        let cov = r.coverage(f.coverage())?;
                        let subs = r.ids(f.substitute_glyph_ids())?;
                        cov.into_iter().zip(subs).collect()
                    }
                };
                out.push(Sub::Single(pairs));
            }
        }
        SubstitutionSubtables::Multiple(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let cov = r.coverage(st.coverage())?;
                let mut v = Vec::new();
                for (g, seq) in cov.into_iter().zip(st.sequences().iter()) {
                    let seq = seq.map_err(|e| r.err(e))?;
                    v.push((g, r.ids(seq.substitute_glyph_ids())?));
                }
                out.push(Sub::Multiple(v));
            }
        }
        SubstitutionSubtables::Alternate(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let cov = r.coverage(st.coverage())?;
                let mut v = Vec::new();
                for (g, set) in cov.into_iter().zip(st.alternate_sets().iter()) {
                    let set = set.map_err(|e| r.err(e))?;
                    v.push((g, r.ids(set.alternate_glyph_ids())?));
                }
                out.push(Sub::Alternate(v));
            }
        }
        SubstitutionSubtables::Ligature(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let cov = r.coverage(st.coverage())?;
                let mut v = Vec::new();
                for (g, set) in cov.into_iter().zip(st.ligature_sets().iter()) {
                    let set = set.map_err(|e| r.err(e))?;
                    for lig in set.ligatures().iter() {
                        let lig = lig.map_err(|e| r.err(e))?;
                        let comps = r.ids(lig.component_glyph_ids())?;
                        v.push((g.clone(), comps, r.name(lig.ligature_glyph().to_u16())));
                    }
                }
                out.push(Sub::Ligature(v));
            }
        }
        SubstitutionSubtables::Contextual(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                out.push(Sub::Ctx(context(r, &st)?));
            }
        }
        SubstitutionSubtables::ChainContextual(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                out.push(Sub::Chain(chain(r, &st)?));
            }
        }
        SubstitutionSubtables::Reverse(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let cov = r.coverage(st.coverage())?;
                let subs = r.ids(st.substitute_glyph_ids())?;
                let mut back = Vec::new();
                for c in st.backtrack_coverages().iter() {
                    back.push(r.coverage(c)?);
                }
                let mut look = Vec::new();
                for c in st.lookahead_coverages().iter() {
                    look.push(r.coverage(c)?);
                }
                out.push(Sub::Reverse { back, cov, subs, look });
            }
        }
        SubstitutionSubtables::EmptyExtension => {}
    }
    Ok(out)
}

fn context(r: &mut Reader, st: &SequenceContext) -> Result<Ctx> {
    Ok(match st {
        SequenceContext::Format1(f) => {
            let cov = r.coverage(f.coverage())?;
            let mut v = Vec::new();
            for (g, set) in cov.into_iter().zip(f.seq_rule_sets().iter()) {
                let mut rules = Vec::new();
                if let Some(set) = set {
                    for rule in set.map_err(|e| r.err(e))?.seq_rules().iter() {
                        let rule = rule.map_err(|e| r.err(e))?;
                        rules.push(r.ids(rule.input_sequence())?);
                    }
                }
                v.push((g, rules));
            }
            Ctx::F1(v)
        }
        SequenceContext::Format2(f) => {
            let cd = r.classes(f.class_def())?;
            let mut sets = Vec::new();
            for set in f.class_seq_rule_sets().iter() {
                let Some(set) = set else {
                    sets.push(None);
                    continue;
                };
                let mut rules = Vec::new();
                for rule in set.map_err(|e| r.err(e))?.class_seq_rules().iter() {
                    let rule = rule.map_err(|e| r.err(e))?;
                    let seq = rule.input_sequence();
                    r.budget.charge(seq.len())?;
                    rules.push(seq.iter().map(|k| k.get()).collect());
                }
                sets.push(Some(rules));
            }
            Ctx::F2 { cd, sets }
        }
        SequenceContext::Format3(f) => {
            let mut input = Vec::new();
            for c in f.coverages().iter() {
                input.push(r.coverage(c)?);
            }
            Ctx::F3(input)
        }
    })
}

fn chain(r: &mut Reader, st: &ChainedSequenceContext) -> Result<Chain> {
    Ok(match st {
        ChainedSequenceContext::Format1(f) => {
            let cov = r.coverage(f.coverage())?;
            let mut v = Vec::new();
            for (g, set) in cov.into_iter().zip(f.chained_seq_rule_sets().iter()) {
                let mut rules = Vec::new();
                if let Some(set) = set {
                    for rule in set.map_err(|e| r.err(e))?.chained_seq_rules().iter() {
                        let rule = rule.map_err(|e| r.err(e))?;
                        let back = r.ids(rule.backtrack_sequence())?;
                        let input = r.ids(rule.input_sequence())?;
                        let look = r.ids(rule.lookahead_sequence())?;
                        rules.push((back, input, look));
                    }
                }
                v.push((g, rules));
            }
            Chain::F1(v)
        }
        ChainedSequenceContext::Format2(f) => {
            let back = r.classes(f.backtrack_class_def()).ok();
            let input = r.classes(f.input_class_def()).ok();
            let look = r.classes(f.lookahead_class_def()).ok();
            let mut sets = Vec::new();
            for set in f.chained_class_seq_rule_sets().iter() {
                let Some(set) = set else {
                    sets.push(None);
                    continue;
                };
                let mut rules = Vec::new();
                for rule in set.map_err(|e| r.err(e))?.chained_class_seq_rules().iter() {
                    let rule = rule.map_err(|e| r.err(e))?;
                    let (b, i, l) = (rule.backtrack_sequence(), rule.input_sequence(), rule.lookahead_sequence());
                    r.budget.charge(b.len().saturating_add(i.len()).saturating_add(l.len()))?;
                    rules.push((b.iter().map(|k| k.get()).collect(), i.iter().map(|k| k.get()).collect(), l.iter().map(|k| k.get()).collect()));
                }
                sets.push(Some(rules));
            }
            Chain::F2 { back, input, look, sets }
        }
        ChainedSequenceContext::Format3(f) => {
            let mut back = Vec::new();
            for c in f.backtrack_coverages().iter() {
                back.push(r.coverage(c)?);
            }
            let mut input = Vec::new();
            for c in f.input_coverages().iter() {
                input.push(r.coverage(c)?);
            }
            let mut look = Vec::new();
            for c in f.lookahead_coverages().iter() {
                look.push(r.coverage(c)?);
            }
            Chain::F3 { back, input, look }
        }
    })
}

fn mark_records(
    r: &mut Reader,
    cov: std::result::Result<CoverageTable, ReadError>,
    array: std::result::Result<read_fonts::tables::gpos::MarkArray, ReadError>,
) -> Result<Vec<(String, u16, String)>> {
    let marks = r.coverage(cov)?;
    let array = array.map_err(|e| r.err(e))?;
    let data = array.offset_data();
    let mut out = Vec::new();
    for (g, rec) in marks.into_iter().zip(array.mark_records().iter()) {
        r.budget.charge(1)?;
        out.push((g, rec.mark_class(), anchor(Some(rec.mark_anchor(data)))));
    }
    Ok(out)
}

fn gpos_subtables(r: &mut Reader, subtables: &PositionSubtables) -> Result<Vec<Sub>> {
    let mut out = Vec::new();
    match subtables {
        PositionSubtables::Single(s) => {
            for st in s.iter() {
                match st.map_err(|e| r.err(e))? {
                    SinglePos::Format1(f) => {
                        let cov = r.coverage(f.coverage())?;
                        let v = value(f.value_format().bits(), &f.value_record());
                        out.push(Sub::SinglePos1 { cov, value: v });
                    }
                    SinglePos::Format2(f) => {
                        let format = f.value_format().bits();
                        let cov = r.coverage(f.coverage())?;
                        let mut v = Vec::new();
                        for (g, vr) in cov.into_iter().zip(f.value_records().iter()) {
                            let vr = vr.map_err(|e| r.err(e))?;
                            v.push((g, value(format, &vr)));
                        }
                        out.push(Sub::SinglePos2(v));
                    }
                }
            }
        }
        PositionSubtables::Pair(s) => {
            for st in s.iter() {
                match st.map_err(|e| r.err(e))? {
                    PairPos::Format1(f) => {
                        let (f1, f2) = (f.value_format1().bits(), f.value_format2().bits());
                        let cov = r.coverage(f.coverage())?;
                        let mut v = Vec::new();
                        for (g, set) in cov.into_iter().zip(f.pair_sets().iter()) {
                            let set = set.map_err(|e| r.err(e))?;
                            for rec in set.pair_value_records().iter() {
                                r.budget.charge(1)?;
                                let rec = rec.map_err(|e| r.err(e))?;
                                let v1 = (f1 != 0).then(|| value(f1, rec.value_record1()));
                                let v2 = (f2 != 0).then(|| value(f1, rec.value_record2()));
                                v.push((g.clone(), r.name(rec.second_glyph().to_u16()), v1, v2));
                            }
                        }
                        out.push(Sub::Pair1(v));
                    }
                    PairPos::Format2(f) => {
                        let (f1, f2) = (f.value_format1().bits(), f.value_format2().bits());
                        let cov = r.coverage(f.coverage())?;
                        let cd1 = r.classes(f.class_def1())?;
                        let cd2 = r.classes(f.class_def2())?;
                        let (n1, n2) = (usize::from(f.class1_count()), usize::from(f.class2_count()));
                        r.budget.charge(n1.saturating_mul(n2))?;
                        let mut values = Vec::new();
                        for rec1 in f.class1_records().iter().take(n1) {
                            let rec1 = rec1.map_err(|e| r.err(e))?;
                            let mut row = Vec::new();
                            for rec2 in rec1.class2_records().iter().take(n2) {
                                let rec2 = rec2.map_err(|e| r.err(e))?;
                                let v1 = (f1 != 0).then(|| value(f1, rec2.value_record1()));
                                let v2 = (f2 != 0).then(|| value(f1, rec2.value_record2()));
                                row.push((v1, v2));
                            }
                            values.push(row);
                        }
                        out.push(Sub::Pair2 { cov, cd1, cd2, values });
                    }
                }
            }
        }
        PositionSubtables::Cursive(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let cov = r.coverage(st.coverage())?;
                let data = st.offset_data();
                let mut v = Vec::new();
                for (g, rec) in cov.into_iter().zip(st.entry_exit_record().iter()) {
                    v.push((g, anchor(rec.entry_anchor(data)), anchor(rec.exit_anchor(data))));
                }
                out.push(Sub::Cursive(v));
            }
        }
        PositionSubtables::MarkToBase(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let marks = mark_records(r, st.mark_coverage(), st.mark_array())?;
                let classes = usize::from(st.mark_class_count());
                let bases = r.coverage(st.base_coverage())?;
                let array = st.base_array().map_err(|e| r.err(e))?;
                let data = array.offset_data();
                let mut v = Vec::new();
                for (g, rec) in bases.into_iter().zip(array.base_records().iter()) {
                    let rec = rec.map_err(|e| r.err(e))?;
                    r.budget.charge(classes)?;
                    let anchors = rec.base_anchors(data);
                    v.push((g, (0..classes).map(|c| anchor(anchors.get(c))).collect()));
                }
                out.push(Sub::MarkBase { marks, bases: v });
            }
        }
        PositionSubtables::MarkToMark(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let marks = mark_records(r, st.mark1_coverage(), st.mark1_array())?;
                let classes = usize::from(st.mark_class_count());
                let bases = r.coverage(st.mark2_coverage())?;
                let array = st.mark2_array().map_err(|e| r.err(e))?;
                let data = array.offset_data();
                let mut v = Vec::new();
                for (g, rec) in bases.into_iter().zip(array.mark2_records().iter()) {
                    let rec = rec.map_err(|e| r.err(e))?;
                    r.budget.charge(classes)?;
                    let anchors = rec.mark2_anchors(data);
                    v.push((g, (0..classes).map(|c| anchor(anchors.get(c))).collect()));
                }
                out.push(Sub::MarkMark { marks, bases: v });
            }
        }
        PositionSubtables::MarkToLig(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                let marks = mark_records(r, st.mark_coverage(), st.mark_array())?;
                let classes = usize::from(st.mark_class_count());
                let ligs = r.coverage(st.ligature_coverage())?;
                let array = st.ligature_array().map_err(|e| r.err(e))?;
                let mut v = Vec::new();
                for (g, attach) in ligs.into_iter().zip(array.ligature_attaches().iter()) {
                    let attach = attach.map_err(|e| r.err(e))?;
                    let data = attach.offset_data();
                    let mut comps = Vec::new();
                    for rec in attach.component_records().iter() {
                        let rec = rec.map_err(|e| r.err(e))?;
                        r.budget.charge(classes)?;
                        let anchors = rec.ligature_anchors(data);
                        comps.push((0..classes).map(|c| anchor(anchors.get(c))).collect());
                    }
                    v.push((g, comps));
                }
                out.push(Sub::MarkLig { marks, ligs: v });
            }
        }
        PositionSubtables::Contextual(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                out.push(Sub::Ctx(context(r, &st)?));
            }
        }
        PositionSubtables::ChainContextual(s) => {
            for st in s.iter() {
                let st = st.map_err(|e| r.err(e))?;
                out.push(Sub::Chain(chain(r, &st)?));
            }
        }
        PositionSubtables::EmptyExtension => {}
    }
    Ok(out)
}

// ── text wrapping (Python's textwrap with width 80 and a four-space subsequent indent) ─────────

/// The lines `textwrap.TextWrapper(width=80, subsequent_indent="    ").wrap(text)` returns.
pub(crate) fn wrap(text: &str) -> Vec<String> {
    let sub_indent = INDENT.repeat(2);
    // Whitespace runs and words, as the wrapper splits them (whitespace becomes spaces).
    let mut chunks: Vec<String> = Vec::new();
    for c in text.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        match chunks.last_mut() {
            Some(last) if last.starts_with(' ') == (c == ' ') => last.push(c),
            _ => chunks.push(c.to_string()),
        }
    }
    chunks.reverse();
    let mut lines: Vec<String> = Vec::new();
    while !chunks.is_empty() {
        let indent = if lines.is_empty() { "" } else { sub_indent.as_str() };
        let width = WIDTH.saturating_sub(indent.len()).max(1);
        if !lines.is_empty() && chunks.last().is_some_and(|c| c.trim().is_empty()) {
            chunks.pop();
        }
        let mut cur: Vec<String> = Vec::new();
        let mut len = 0usize;
        while let Some(c) = chunks.last() {
            let l = c.chars().count();
            if len + l <= width {
                len += l;
                cur.extend(chunks.pop());
            } else {
                break;
            }
        }
        if let Some(c) = chunks.last()
            && c.chars().count() > width
        {
            let space = if width > len { width - len } else { 1 };
            if let Some(word) = chunks.pop() {
                let head: String = word.chars().take(space).collect();
                let tail: String = word.chars().skip(space).collect();
                cur.push(head);
                if !tail.is_empty() {
                    chunks.push(tail);
                }
            }
        }
        if cur.last().is_some_and(|c| c.trim().is_empty()) {
            cur.pop();
        }
        if !cur.is_empty() {
            lines.push(format!("{indent}{}", cur.concat()));
        }
    }
    lines
}

fn wrap_statement(text: &str) -> String {
    wrap(text).join(&format!("\n{}", INDENT.repeat(4)))
}

// ── the converter ───────────────────────────────────────────────────────────────────────────────

/// A class record's key: lookup, subtable, class, side.
type ClassKey = (usize, usize, u16, &'static str);

const LEFT: &str = "Left";
const RIGHT: &str = "Right";
const BACKTRACK: &str = "BacktrackClassDef";
const INPUT: &str = "InputClassDef";
const LOOKAHEAD: &str = "LookAheadClassDef";
const CONTEXT: &str = "";
const MARK: &str = "MarkClass";
const MARK1: &str = "MarkMarkClass1";

struct Converter<'m> {
    model: &'m Model,
    gsub: bool,
    show_extensions: bool,
    names: &'m [String],
    classes_by_names: BTreeMap<Vec<String>, Vec<ClassKey>>,
    classes_by_lookup: HashMap<ClassKey, String>,
    classes_by_class_name: BTreeMap<String, Vec<String>>,
    marks_by_defs: BTreeMap<Vec<(Vec<String>, String)>, Vec<ClassKey>>,
    marks_by_lookup: HashMap<ClassKey, String>,
    marks_by_class_name: BTreeMap<String, Vec<(Vec<String>, String)>>,
    seen: HashMap<usize, String>,
    level: usize,
}

impl<'m> Converter<'m> {
    fn new(model: &'m Model, gsub: bool, show_extensions: bool, names: &'m [String]) -> Self {
        Self {
            model,
            gsub,
            show_extensions,
            names,
            classes_by_names: BTreeMap::new(),
            classes_by_lookup: HashMap::new(),
            classes_by_class_name: BTreeMap::new(),
            marks_by_defs: BTreeMap::new(),
            marks_by_lookup: HashMap::new(),
            marks_by_class_name: BTreeMap::new(),
            seen: HashMap::new(),
            level: 2,
        }
    }

    fn line(&self, out: &mut Out, text: &str) -> Result<()> {
        if text.is_empty() {
            return out.line("");
        }
        out.push(&INDENT.repeat(self.level))?;
        out.line(text)
    }

    fn run(&mut self, out: &mut Out, budget: &mut Budget) -> Result<()> {
        let (systems, by_tag) = self.lang_systems();
        for (script, lang, req) in &systems {
            let rf = if *req != 0xFFFF { format!("  Required Feature Index {req}") } else { String::new() };
            self.line(out, &format!("languagesystem {script} {lang}{rf};"))?;
        }
        self.line(out, "")?;
        self.build_classes(budget)?;
        self.write_classes(out)?;
        self.write_mark_classes(out)?;
        self.features(out, &by_tag, budget)
    }

    /// `buildLangSys`: the `languagesystem` list and, per feature tag, the feature indices of
    /// each language system.
    #[allow(clippy::type_complexity)]
    fn lang_systems(&self) -> (Vec<(String, String, u16)>, BTreeMap<String, BTreeMap<(String, String), Vec<u16>>>) {
        let mut list: Vec<(String, String, u16)> = Vec::new();
        let mut by_tag: BTreeMap<String, BTreeMap<(String, String), Vec<u16>>> = BTreeMap::new();
        let mut by_index: BTreeMap<u16, usize> = BTreeMap::new();
        let dflt = tag_text(DFLT_LANG);
        let mut have_dflt = false;
        for (script, lang, req, feats) in &self.model.systems {
            if script == "DFLT" || script == "dflt" {
                have_dflt = true;
            }
            if *lang == dflt {
                list.insert(0, (script.clone(), lang.clone(), *req));
            } else {
                list.push((script.clone(), lang.clone(), *req));
            }
            for fi in feats {
                let Some((tag, _)) = self.model.features.get(usize::from(*fi)) else { continue };
                *by_index.entry(*fi).or_default() += 1;
                by_tag.entry(tag.clone()).or_default().entry((script.clone(), lang.clone())).or_default().push(*fi);
            }
        }
        if !have_dflt && by_index.values().any(|n| *n == self.model.scripts) {
            list.insert(0, ("DFLT".into(), "dflt".into(), 0xFFFF));
        }
        (list, by_tag)
    }

    fn name(&self, g: usize) -> String {
        self.names.get(g).cloned().unwrap_or_else(|| format!("glyph{g:05}"))
    }

    /// `addClassDef`.
    fn add_class_def(
        &mut self,
        key: (usize, usize),
        classes: &Classes,
        coverage: Option<&[String]>,
        side: &'static str,
        budget: &mut Budget,
    ) -> Result<()> {
        let mut by_class: BTreeMap<u16, Vec<String>> = BTreeMap::new();
        let mut taken: HashSet<&str> = HashSet::new();
        for (name, class) in classes {
            taken.insert(name.as_str());
            by_class.entry(*class).or_default().push(name.clone());
        }
        let mut zero = Vec::new();
        if let Some(cov) = coverage
            && !by_class.contains_key(&0)
        {
            budget.charge(self.names.len())?;
            let cov: HashSet<&str> = cov.iter().map(String::as_str).collect();
            let mut rest: Vec<String> =
                (0..self.names.len()).map(|g| self.name(g)).filter(|n| !taken.contains(n.as_str()) && cov.contains(n.as_str())).collect();
            rest.sort();
            rest.dedup();
            zero = rest;
        }
        by_class.insert(0, zero);
        for (class, mut names) in by_class {
            names.sort();
            self.classes_by_names.entry(names).or_default().push((key.0, key.1, class, side));
        }
        Ok(())
    }

    /// `AddMarkClassDef`.
    fn add_mark_class_def(&mut self, key: (usize, usize), marks: &[(String, u16, String)], side: &'static str) {
        let mut by_class: Vec<(u16, Vec<(String, String)>)> = Vec::new();
        for (g, class, a) in marks {
            match by_class.iter_mut().find(|(c, _)| c == class) {
                Some((_, v)) => v.push((a.clone(), g.clone())),
                None => by_class.push((*class, vec![(a.clone(), g.clone())])),
            }
        }
        for (class, values) in by_class {
            let mut by_anchor: Vec<(String, Vec<String>)> = Vec::new();
            for (a, g) in values {
                match by_anchor.iter_mut().find(|(k, _)| *k == a) {
                    Some((_, v)) => v.push(g),
                    None => by_anchor.push((a, vec![g])),
                }
            }
            let mut defs: Vec<(Vec<String>, String)> = by_anchor
                .into_iter()
                .map(|(a, mut gs)| {
                    gs.sort();
                    (gs, a)
                })
                .collect();
            defs.sort();
            self.marks_by_defs.entry(defs).or_default().push((key.0, key.1, class, side));
        }
    }

    fn build_classes(&mut self, budget: &mut Budget) -> Result<()> {
        let model = self.model;
        for (li, lookup) in model.lookups.iter().enumerate() {
            for (si, sub) in lookup.subtables.iter().enumerate() {
                let key = (li, si);
                match sub {
                    Sub::Pair2 { cov, cd1, cd2, .. } => {
                        self.add_class_def(key, cd1, Some(cov), LEFT, budget)?;
                        self.add_class_def(key, cd2, None, RIGHT, budget)?;
                    }
                    Sub::MarkBase { marks, .. } | Sub::MarkLig { marks, .. } => self.add_mark_class_def(key, marks, MARK),
                    Sub::MarkMark { marks, .. } => self.add_mark_class_def(key, marks, MARK1),
                    Sub::Ctx(Ctx::F2 { cd, .. }) => self.add_class_def(key, cd, None, CONTEXT, budget)?,
                    Sub::Chain(Chain::F2 { back, input, look, .. }) => {
                        for (cd, side) in [(back, BACKTRACK), (input, INPUT), (look, LOOKAHEAD)] {
                            if let Some(cd) = cd {
                                self.add_class_def(key, cd, None, side, budget)?;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let lists: Vec<(Vec<String>, Vec<ClassKey>)> = std::mem::take(&mut self.classes_by_names).into_iter().collect();
        for (names, recs) in lists {
            let name = match names.as_slice() {
                [] => "@empty".to_string(),
                [a] => format!("[{a}]"),
                [a, b] => format!("@{a}_{b}"),
                [a, b, .., z] => {
                    let n = format!("@{a}_{}_{z}", names.len());
                    if self.classes_by_class_name.contains_key(&n) { format!("@{a}_{b}_{}_{z}", names.len()) } else { n }
                }
            };
            for rec in recs {
                self.classes_by_lookup.insert(rec, name.clone());
            }
            self.classes_by_class_name.insert(name, names);
        }
        #[allow(clippy::type_complexity)]
        let defs: Vec<(Vec<(Vec<String>, String)>, Vec<ClassKey>)> = std::mem::take(&mut self.marks_by_defs).into_iter().collect();
        for (def, recs) in defs {
            let all: Vec<&String> = def.iter().flat_map(|(gs, _)| gs.iter()).collect();
            let name = match all.as_slice() {
                [] => "@empty_mark".to_string(),
                [a] => format!("{a}_mark"),
                [a, b] => format!("@{a}_{b}_mark"),
                [a, b, .., z] => {
                    let n = format!("@{a}_{}_{z}_mark", all.len());
                    if self.classes_by_class_name.contains_key(&n) { format!("@{a}_{b}_{}_{z}_mark", all.len()) } else { n }
                }
            };
            for rec in recs {
                self.marks_by_lookup.insert(rec, name.clone());
            }
            self.marks_by_class_name.insert(name, def);
        }
        Ok(())
    }

    fn write_classes(&self, out: &mut Out) -> Result<()> {
        if !self.classes_by_class_name.is_empty() {
            self.line(out, "")?;
            self.line(out, "# Class definitions *********************************")?;
        }
        for (name, glyphs) in &self.classes_by_class_name {
            if name.starts_with('[') {
                continue;
            }
            for l in wrap(&format!("{name} = [ {} ];", glyphs.join(" "))) {
                self.line(out, &l)?;
            }
        }
        self.line(out, "")
    }

    fn write_mark_classes(&self, out: &mut Out) -> Result<()> {
        if !self.marks_by_class_name.is_empty() {
            self.line(out, "")?;
            self.line(out, "# Mark Class definitions *********************************")?;
        }
        let pad = INDENT.repeat(self.level);
        for (name, defs) in &self.marks_by_class_name {
            for (glyphs, a) in defs {
                let lines = wrap(&glyphs.join(" "));
                let body = lines.join(&format!("\n{pad}"));
                self.line(out, &format!("mark [{body}] {a} {name};"))?;
            }
        }
        self.line(out, "")
    }

    fn class(&self, li: usize, si: usize, class: u16, side: &'static str) -> String {
        self.classes_by_lookup.get(&(li, si, class, side)).cloned().unwrap_or_else(|| "@empty".into())
    }

    fn mark_class(&self, li: usize, si: usize, class: u16, side: &'static str) -> String {
        self.marks_by_lookup.get(&(li, si, class, side)).cloned().unwrap_or_else(|| "[]".into())
    }

    fn features(&mut self, out: &mut Out, by_tag: &BTreeMap<String, BTreeMap<(String, String), Vec<u16>>>, budget: &mut Budget) -> Result<()> {
        let dflt_keys = [("DFLT".to_string(), "dflt".to_string()), ("dflt".to_string(), "dflt".to_string())];
        for (tag, systems) in by_tag {
            self.line(out, &format!("feature {tag} {{"))?;
            self.level += 1;
            let dflt = dflt_keys.iter().find(|k| systems.contains_key(*k)).cloned();
            let mut dflt_lookups: Option<BTreeMap<u16, ()>> = None;
            if let Some(key) = &dflt
                && let Some(indices) = systems.get(key)
                && !indices.is_empty()
            {
                dflt_lookups = Some(self.dflt_feature(out, key, indices, budget)?);
            }
            let mut prev = dflt.clone().unwrap_or_else(|| ("DFLT".to_string(), "dflt".to_string()));
            for (key, indices) in systems {
                if Some(key) == dflt.as_ref() {
                    continue;
                }
                self.system_feature(out, key, &prev, indices, dflt_lookups.as_ref(), budget)?;
                prev = key.clone();
            }
            self.level -= 1;
            self.line(out, &format!("}} {tag};"))?;
            self.line(out, "")?;
        }
        Ok(())
    }

    /// The lookups of a set of feature indices, and the tag of the last feature.
    fn lookups_of(&self, indices: &[u16]) -> (BTreeMap<u16, ()>, String) {
        let mut lookups = BTreeMap::new();
        let mut tag = String::new();
        for fi in indices {
            if let Some((t, ls)) = self.model.features.get(usize::from(*fi)) {
                tag.clone_from(t);
                for l in ls {
                    lookups.insert(*l, ());
                }
            }
        }
        (lookups, tag)
    }

    fn dflt_feature(&mut self, out: &mut Out, key: &(String, String), indices: &[u16], budget: &mut Budget) -> Result<BTreeMap<u16, ()>> {
        let (lookups, tag) = self.lookups_of(indices);
        let mut name_index = 0usize;
        for li in lookups.keys() {
            let li = usize::from(*li);
            if let Some(name) = self.seen.get(&li) {
                let text = format!("lookup {name};");
                self.line(out, &text)?;
            } else {
                let name = format!("{tag}_{}_{}_{name_index}", key.0, key.1);
                self.lookup_block(out, li, name, budget)?;
                name_index += 1;
            }
        }
        Ok(lookups)
    }

    fn system_feature(
        &mut self,
        out: &mut Out,
        key: &(String, String),
        prev: &(String, String),
        indices: &[u16],
        dflt: Option<&BTreeMap<u16, ()>>,
        budget: &mut Budget,
    ) -> Result<()> {
        let (mut lookups, tag) = self.lookups_of(indices);
        let mut exclude = false;
        if let Some(d) = dflt.filter(|d| !d.is_empty()) {
            let before = lookups.len();
            lookups.retain(|l, _| !d.contains_key(l));
            exclude = lookups.len() == before;
        }
        if lookups.is_empty() {
            return Ok(());
        }
        let mut exclude_text = if exclude { " excludeDFLT" } else { "" };
        if prev.0 != key.0 {
            self.line(out, &format!("script {};", key.0))?;
            self.line(out, &format!("language {};", key.1))?;
        } else if prev.1 != key.1 {
            self.line(out, &format!("language {};", key.1))?;
        }
        let mut name_index = 0usize;
        for li in lookups.keys() {
            let li = usize::from(*li);
            if let Some(name) = self.seen.get(&li) {
                let text = format!("lookup {name}{exclude_text};");
                self.line(out, &text)?;
            } else {
                let name = format!("{tag}_{}_{}_{name_index}", key.0.trim(), key.1.trim());
                self.lookup_block(out, li, name, budget)?;
                name_index += 1;
            }
            exclude_text = "";
        }
        Ok(())
    }

    fn lookup_block(&mut self, out: &mut Out, li: usize, name: String, budget: &mut Budget) -> Result<()> {
        let model = self.model;
        let Some(lookup) = model.lookups.get(li) else {
            return Err(TablesError::invalid(format!("a feature names lookup {li}, which the lookup list does not have")));
        };
        self.seen.insert(li, name.clone());
        let ext_type = if self.gsub { 7 } else { 9 };
        let ext = if self.show_extensions && lookup.stored == ext_type { " useExtension" } else { "" };
        self.line(out, &format!("lookup {name}{ext} {{"))?;
        self.level += 1;
        let flag = flag_text(lookup.flag);
        if !flag.is_empty() {
            self.line(out, &format!("lookupflag {flag};"))?;
        }
        for (si, sub) in lookup.subtables.iter().enumerate() {
            for rule in self.rules(li, si, sub, budget)? {
                self.line(out, &rule)?;
            }
        }
        self.level -= 1;
        self.line(out, &format!("}} {name};"))?;
        self.line(out, "")
    }

    fn rules(&self, li: usize, si: usize, sub: &Sub, budget: &mut Budget) -> Result<Vec<String>> {
        let mut rules: Vec<String> = Vec::new();
        let kw = if self.gsub { "sub" } else { "pos" };
        match sub {
            Sub::Single(pairs) => rules.extend(pairs.iter().map(|(a, b)| format!("sub {a} by {b};"))),
            Sub::Multiple(v) => rules.extend(v.iter().map(|(a, bs)| format!("sub {a} by [{}];", bs.join(" ")))),
            Sub::Alternate(v) => rules.extend(v.iter().map(|(a, alts)| {
                let mut alts = alts.clone();
                alts.sort();
                format!("sub {a} from [{}];", alts.join(" "))
            })),
            Sub::Ligature(v) => rules.extend(v.iter().map(|(first, comps, lig)| {
                let mut tokens = vec![first.as_str()];
                tokens.extend(comps.iter().map(String::as_str));
                format!("sub {} by {lig};", tokens.join(" "))
            })),
            Sub::SinglePos1 { cov, value } => {
                if value != "0" {
                    rules.extend(cov.iter().map(|g| format!("pos {g} {value};")));
                }
            }
            Sub::SinglePos2(v) => rules.extend(v.iter().filter(|(_, value)| value != "0").map(|(g, value)| format!("pos {g} {value};"))),
            Sub::Pair1(v) => {
                for (a, b, v1, v2) in v {
                    budget.charge(1)?;
                    let mut rule = None;
                    if let Some(s) = v1.as_ref().filter(|s| *s != "0") {
                        rule = Some(format!("pos {a} {b} {s};"));
                    }
                    if let Some(s) = v2.as_ref().filter(|s| *s != "0") {
                        rule = Some(format!("pos {a} {b} {s}; # Warning: non-zero pos value record for second glyph"));
                    }
                    rules.extend(rule);
                }
            }
            Sub::Pair2 { values, .. } => {
                for (i, row) in values.iter().enumerate() {
                    let g1 = self.class(li, si, u16::try_from(i).unwrap_or(u16::MAX), LEFT);
                    for (j, (v1, v2)) in row.iter().enumerate() {
                        budget.charge(1)?;
                        let g2 = self.class(li, si, u16::try_from(j).unwrap_or(u16::MAX), RIGHT);
                        let mut rule = None;
                        if let Some(s) = v1.as_ref().filter(|s| *s != "0") {
                            rule = Some(format!("pos {g1} {g2} {s};"));
                        }
                        if let Some(s) = v2.as_ref().filter(|s| *s != "0") {
                            rule = Some(format!("pos {g1} {g2} {s}; # Warning: non-zero pos value record for second glyph"));
                        }
                        rules.extend(rule);
                    }
                }
            }
            Sub::Cursive(v) => rules.extend(v.iter().map(|(g, a, b)| format!("pos cursive {g} {a} {b};"))),
            Sub::MarkBase { marks, bases } => rules.extend(self.attach_rules(li, si, marks, bases, MARK, true)),
            Sub::MarkMark { marks, bases } => rules.extend(self.attach_rules(li, si, marks, bases, MARK1, false)),
            Sub::MarkLig { marks, ligs } => {
                let order = class_order(marks);
                let names: Vec<String> = order.iter().map(|c| self.mark_class(li, si, *c, MARK)).collect();
                let count = order.len();
                let sub_indent = INDENT;
                for (lig, comps) in ligs {
                    let mut text = format!("pos ligature {lig}");
                    for (ci, anchors) in comps.iter().enumerate() {
                        if ci > 0 {
                            text.push_str(&format!("\n{}ligComponent", sub_indent.repeat(5)));
                        }
                        for cl in 0..count {
                            let a = anchors.get(cl).cloned().unwrap_or_else(|| "<anchor NULL>".into());
                            text.push_str(&format!(" {a} mark {}", names.get(cl).cloned().unwrap_or_default()));
                            if cl + 1 < count {
                                text.push_str(&format!("\n{sub_indent}"));
                            }
                        }
                    }
                    text.push(';');
                    rules.push(text);
                }
            }
            Sub::Ctx(ctx) => return Ok(self.context_rules(li, si, kw, ctx)),
            Sub::Chain(chain) => return Ok(self.chain_rules(li, si, chain)),
            Sub::Reverse { back, cov, subs, look } => {
                let set = |g: &Vec<String>| {
                    let mut g = g.clone();
                    g.sort();
                    g.dedup();
                    g.join(" ")
                };
                let mut back: Vec<String> = back.iter().map(|b| format!("[ {} ] ", set(b))).collect();
                back.reverse();
                let mut pairs: Vec<(&String, usize)> = cov.iter().zip(0..).collect();
                pairs.sort();
                let input = format!("[ {} ]", pairs.iter().map(|(g, _)| g.as_str()).collect::<Vec<_>>().join(" "));
                let substitutes: Vec<&str> = pairs.iter().filter_map(|(_, i)| subs.get(*i).map(String::as_str)).collect();
                let look: Vec<String> = look.iter().map(|l| format!(" [ {} ]", set(l))).collect();
                let repl = if subs.is_empty() { String::new() } else { format!(" by [ {} ]", substitutes.join(" ")) };
                return Ok(vec![format!("sub {}{input}'{}{repl};", back.join(" "), look.join(" "))]);
            }
        }
        rules.sort();
        Ok(rules)
    }

    /// Mark-to-base (`base`) and mark-to-mark rules: bases grouped by their anchors.
    fn attach_rules(
        &self,
        li: usize,
        si: usize,
        marks: &[(String, u16, String)],
        bases: &[(String, Vec<String>)],
        side: &'static str,
        base: bool,
    ) -> Vec<String> {
        let order = class_order(marks);
        let names: Vec<String> = order.iter().map(|c| self.mark_class(li, si, *c, side)).collect();
        let count = order.len();
        let mut groups: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        for (g, anchors) in bases {
            let key: Vec<String> = (0..count).map(|c| anchors.get(c).cloned().unwrap_or_else(|| "<anchor NULL>".into())).collect();
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, gs)) => gs.push(g.clone()),
                None => groups.push((key, vec![g.clone()])),
            }
        }
        let mut rules = Vec::new();
        for (key, mut glyphs) in groups {
            glyphs.sort();
            let text = glyphs.join(" ");
            let mut rule =
                if base { format!("pos base [{}]", wrap(&text).join(&format!("\n{}", INDENT.repeat(4)))) } else { format!("pos mark [{text}]") };
            for cl in 0..count {
                rule.push_str(&format!(" {} mark {}", key.get(cl).cloned().unwrap_or_default(), names.get(cl).cloned().unwrap_or_default()));
                if cl + 1 < count {
                    rule.push_str(&format!("\n{INDENT}"));
                }
            }
            rule.push(';');
            rules.push(rule);
        }
        rules.sort();
        rules
    }

    fn context_rules(&self, li: usize, si: usize, kw: &str, ctx: &Ctx) -> Vec<String> {
        let mut rules: Vec<String> = Vec::new();
        match ctx {
            Ctx::F1(sets) => {
                for (first, seqs) in sets {
                    for seq in seqs {
                        let mut input = vec![first.as_str()];
                        input.extend(seq.iter().map(String::as_str));
                        rules.push(format!("{kw} {}'", input.join("' ")));
                    }
                }
            }
            Ctx::F2 { sets, .. } => {
                for (i, set) in sets.iter().enumerate() {
                    let Some(seqs) = set else { continue };
                    for seq in seqs {
                        let mut input = vec![self.class(li, si, u16::try_from(i).unwrap_or(u16::MAX), CONTEXT)];
                        input.extend(seq.iter().map(|c| self.class(li, si, *c, CONTEXT)));
                        rules.push(format!("{kw} {}';", input.join("' ")));
                    }
                }
            }
            Ctx::F3(input) => {
                let close = if self.gsub { "]'" } else { "]" };
                let input: Vec<String> = input.iter().map(|g| format!("[{}{close}", sorted_set(g))).collect();
                rules.push(format!("{kw} {}';", input.join("' ")));
            }
        }
        rules.sort();
        rules
    }

    fn chain_rules(&self, li: usize, si: usize, chain: &Chain) -> Vec<String> {
        let mut rules: Vec<String> = Vec::new();
        match chain {
            Chain::F1(sets) => {
                for (first, seqs) in sets {
                    for (back, input, look) in seqs {
                        let mut back = back.clone();
                        back.reverse();
                        let mut inp = vec![first.clone()];
                        inp.extend(input.iter().cloned());
                        let text = if self.gsub { format!("{}'", inp.join(" ")) } else { format!("{}'", inp.join("' ")) };
                        let tail = if self.gsub { " by " } else { "" };
                        rules.push(format!("sub {} {text} {}{tail}", back.join(" "), look.join(" ")));
                    }
                }
            }
            Chain::F2 { sets, .. } => {
                for (i, set) in sets.iter().enumerate() {
                    let Some(seqs) = set else { continue };
                    for (back, input, look) in seqs {
                        let back: Vec<String> = back.iter().map(|c| self.class(li, si, *c, BACKTRACK)).collect();
                        let mut inp = vec![self.class(li, si, u16::try_from(i).unwrap_or(u16::MAX), INPUT)];
                        inp.extend(input.iter().map(|c| self.class(li, si, *c, INPUT)));
                        let look: Vec<String> = look.iter().map(|c| self.class(li, si, *c, LOOKAHEAD)).collect();
                        let rule = if self.gsub {
                            format!("sub {} {}' {};", back.join(" "), inp.join(" "), look.join(" "))
                        } else {
                            format!("sub {} {}' {};", back.join(" "), inp.join("' "), look.join(" "))
                        };
                        rules.push(rule);
                    }
                }
            }
            Chain::F3 { back, input, look } => {
                let mut back: Vec<String> = back.iter().map(|g| format!("[{}]", sorted_set(g))).collect();
                back.reverse();
                let look: Vec<String> = look.iter().map(|g| format!("[{}]", sorted_set(g))).collect();
                if self.gsub {
                    let input: Vec<String> = input.iter().map(|g| format!("[{}]'", sorted_set(g))).collect();
                    rules.push(wrap_statement(&format!("sub {} {} {};", back.join(" "), input.join(" "), look.join(" "))));
                } else {
                    let input: Vec<String> = input.iter().map(|g| format!("[{}]", sorted_set(g))).collect();
                    rules.push(format!("pos {} {}' {};", back.join(" "), input.join("' "), look.join(" ")));
                }
            }
        }
        rules.sort();
        rules
    }
}

/// Mark classes in the order they first appear in the mark coverage.
fn class_order(marks: &[(String, u16, String)]) -> Vec<u16> {
    let mut order = Vec::new();
    for (_, c, _) in marks {
        if !order.contains(c) {
            order.push(*c);
        }
    }
    order
}

fn sorted_set(glyphs: &[String]) -> String {
    let mut g = glyphs.to_vec();
    g.sort();
    g.dedup();
    g.join(" ")
}

/// `getLookupFlagTag`.
fn flag_text(flag: u16) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (bit, name) in [(1u16, "RightToLeft"), (2, "IgnoreBaseGlyphs"), (4, "IgnoreLigatures"), (8, "IgnoreMarks"), (0xFF00, "MarkAttachmentTypedef")]
    {
        if bit & flag != 0 {
            parts.push(name.into());
            if bit == 0xFF00 {
                parts.push(((bit & flag) >> 8).to_string());
            }
        }
    }
    parts.join(" ")
}
