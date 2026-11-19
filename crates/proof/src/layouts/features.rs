//! The feature sheet (rows `proof`, `font.proof`): the rules of `GSUB` and `GPOS` features drawn
//! as their input and their result, as the reference table dumper's feature proofs.
//!
//! Each feature of each table starts a page with its tag; each of its lookups gets a heading with
//! its index, its type and the language systems (script/language) that use it. Under it:
//!
//! - single and multiple substitutions: the glyph, an arrow, the result;
//! - alternates: the glyph, an arrow, every alternate;
//! - ligatures: the components joined by `+`, an arrow, the ligature;
//! - single adjustments: the glyph, an arrow, the glyph moved by the placement, with its value;
//! - pair adjustments: the two glyphs set with the adjustment applied, its value under them.
//!
//! Contextual, chained, reverse, cursive and mark lookups are named in their heading but not
//! drawn. A class pair subtable is drawn as every pair of glyphs its classes hold, without class 0
//! of the second glyph. Only the first glyph's advance (or vertical advance) of a pair is read;
//! device tables and variation deltas are left out.
//!
//! Rules come in glyph-id order of their first glyph inside each lookup, or in the font's own order
//! when asked; a selection of glyphs keeps the rules whose first glyph is selected.
//!
//! Input is untrusted: at most [`MAX_RULES`] rules are drawn and [`MAX_WORK`] glyphs and pairs
//! read; a lookup cut short says so in its heading.

use std::collections::{BTreeMap, HashSet};

use skrifa::raw::TableProvider;
use skrifa::raw::tables::gpos::{PairPos, PositionLookup, PositionSubtables, SinglePos, ValueRecord};
use skrifa::raw::tables::gsub::{FeatureList, ScriptList, SingleSubst, SubstitutionLookup, SubstitutionSubtables};

use super::Display;
use super::spot::{Flow, Glyphs, draw_glyph, glyph_label};
use crate::font::Face;
use crate::page::{Document, Ink, Item};
use crate::text::num;
use crate::{ProofError, ProofOptions};

/// Most rules one feature sheet draws.
pub(crate) const MAX_RULES: usize = 50_000;
/// Most glyphs, pairs and records read from the layout tables for one sheet.
pub(crate) const MAX_WORK: usize = 4_000_000;
/// Most scripts, language systems and feature records looked at in one table.
const MAX_RECORDS: usize = 4_096;
/// Most characters of a lookup heading.
const MAX_HEADING: usize = 160;

/// What the feature sheet shows.
pub(crate) struct Request<'r> {
    /// Feature tags in the order to show them; empty shows every feature.
    pub tags: &'r [[u8; 4]],
    pub font_order: bool,
    /// Glyph size in points.
    pub size: f64,
}

/// One rule of a lookup, by glyph id.
#[derive(Clone, Debug, PartialEq)]
enum Rule {
    /// `input` becomes `output`; for an alternate substitution `output` lists the choices.
    Sub { input: Vec<u32>, output: Vec<u32> },
    /// A single adjustment: x and y placement, x and y advance.
    Single { glyph: u32, value: [f64; 4] },
    /// A pair adjustment of the first glyph's advance.
    Pair { first: u32, second: u32, value: f64 },
}

impl Rule {
    fn first(&self) -> u32 {
        match self {
            Rule::Sub { input, .. } => input.first().copied().unwrap_or(0),
            Rule::Single { glyph, .. } => *glyph,
            Rule::Pair { first, .. } => *first,
        }
    }
}

/// A lookup as the sheet shows it.
struct Shown {
    index: u16,
    kind: &'static str,
    drawn: bool,
    users: Vec<String>,
    rules: Vec<Rule>,
    /// Rules were left out for the limits.
    cut: bool,
}

/// One feature of one table.
struct Section {
    table: &'static str,
    tag: [u8; 4],
    lookups: Vec<Shown>,
}

/// A budget of glyphs and pairs to read.
struct Work(usize);

impl Work {
    fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

pub(crate) fn sheet(
    face: &Face<'_>,
    doc: &mut Document,
    options: &ProofOptions,
    d: &Display,
    gids: &[u32],
    request: Request<'_>,
) -> Result<(), ProofError> {
    let mut work = Work(MAX_WORK);
    let mut sections: Vec<Section> = Vec::new();
    let wanted = |tag: &[u8; 4]| request.tags.is_empty() || request.tags.contains(tag);
    let gsub = face.font.gsub().ok().and_then(|t| Some((t.script_list().ok()?, t.feature_list().ok()?, t.lookup_list().ok()?)));
    if let Some((scripts, features, list)) = gsub {
        for (tag, lookups) in features_of(&scripts, &features).into_iter().filter(|(t, _)| wanted(t)) {
            let mut shown = Vec::new();
            for (index, users) in lookups {
                let (kind, drawn, rules) = match list.lookups().get(usize::from(index)) {
                    Ok(lookup) => gsub_rules(&lookup, &mut work),
                    Err(_) => ("unreadable lookup", false, Vec::new()),
                };
                shown.push(Shown { index, kind, drawn, users, rules, cut: work.0 == 0 });
            }
            sections.push(Section { table: "GSUB", tag, lookups: shown });
        }
    }
    let gpos = face.font.gpos().ok().and_then(|t| Some((t.script_list().ok()?, t.feature_list().ok()?, t.lookup_list().ok()?)));
    if let Some((scripts, features, list)) = gpos {
        for (tag, lookups) in features_of(&scripts, &features).into_iter().filter(|(t, _)| wanted(t)) {
            let mut shown = Vec::new();
            for (index, users) in lookups {
                let (kind, drawn, rules) = match list.lookups().get(usize::from(index)) {
                    Ok(lookup) => gpos_rules(&lookup, &mut work),
                    Err(_) => ("unreadable lookup", false, Vec::new()),
                };
                shown.push(Shown { index, kind, drawn, users, rules, cut: work.0 == 0 });
            }
            sections.push(Section { table: "GPOS", tag, lookups: shown });
        }
    }
    for tag in request.tags {
        if !sections.iter().any(|s| s.tag == *tag) {
            return Err(ProofError::BadOptions(format!("the font has no feature `{}` in GSUB or GPOS", tag_text(tag))));
        }
    }
    if sections.is_empty() {
        return Err(ProofError::BadOptions("the font has no GSUB or GPOS features".into()));
    }
    if !request.tags.is_empty() {
        sections.sort_by_key(|s| request.tags.iter().position(|t| *t == s.tag).unwrap_or(usize::MAX));
    }
    // The glyphs to show filter the rules; every glyph selected keeps every rule.
    let every = options.glyphs.is_empty();
    let selected: HashSet<u32> = gids.iter().copied().collect();
    let mut left = MAX_RULES;
    for lookup in sections.iter_mut().flat_map(|s| s.lookups.iter_mut()) {
        if !every {
            lookup.rules.retain(|r| selected.contains(&r.first()));
        }
        if !request.font_order {
            lookup.rules.sort_by_key(Rule::first);
        }
        if lookup.rules.len() > left {
            lookup.rules.truncate(left);
            lookup.cut = true;
        }
        left -= lookup.rules.len();
    }

    let mut glyphs = Glyphs::new(doc);
    let mut flow = Flow::new(face, options.page_size, request.size, true);
    for section in &sections {
        flow.page_break(doc)?;
        flow.heading(doc, format!("{} feature {}", section.table, tag_text(&section.tag)), 12.0, Ink::Text)?;
        for lookup in &section.lookups {
            let mut heading = format!("lookup {}: {}", lookup.index, lookup.kind);
            if !lookup.users.is_empty() {
                heading.push_str(&format!("; used by {}", lookup.users.join(", ")));
            }
            if !lookup.drawn {
                heading.push_str("; rules not drawn");
            } else if lookup.rules.is_empty() {
                heading.push_str("; no rules for the glyphs to show");
            }
            if lookup.cut {
                heading.push_str("; cut short at the proof's limits");
            }
            flow.heading(doc, crate::page::fit_text(&heading, MAX_HEADING), 8.0, Ink::Faint)?;
            for rule in &lookup.rules {
                draw_rule(face, doc, d, &mut flow, &mut glyphs, rule)?;
            }
        }
    }
    flow.finish(doc)
}

fn tag_text(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).trim_end().to_string()
}

/// For each feature tag of a table, in the feature list's order, its lookups in lookup-list
/// order with the language systems (`script/language`) that use each one.
fn features_of(scripts: &ScriptList<'_>, features: &FeatureList<'_>) -> Vec<([u8; 4], BTreeMap<u16, Vec<String>>)> {
    let records = features.feature_records();
    let mut users: Vec<Vec<String>> = vec![Vec::new(); records.len().min(MAX_RECORDS)];
    let mut add = |index: u16, who: &str| {
        if let Some(list) = users.get_mut(usize::from(index)).filter(|l| !l.iter().any(|w| w == who)) {
            list.push(who.to_string());
        }
    };
    for record in scripts.script_records().iter().take(MAX_RECORDS) {
        let Ok(script) = record.script(scripts.offset_data()) else { continue };
        let tag = record.script_tag();
        let mut systems = Vec::new();
        if let Some(Ok(default)) = script.default_lang_sys() {
            systems.push(("dflt".to_string(), default));
        }
        for lang in script.lang_sys_records().iter().take(MAX_RECORDS) {
            if let Ok(sys) = lang.lang_sys(script.offset_data()) {
                systems.push((lang.lang_sys_tag().to_string(), sys));
            }
        }
        for (lang, sys) in systems {
            let who = format!("{tag}/{}", lang.trim_end());
            if sys.required_feature_index() != 0xFFFF {
                add(sys.required_feature_index(), &who);
            }
            for index in sys.feature_indices().iter().take(MAX_RECORDS) {
                add(index.get(), &who);
            }
        }
    }
    let mut out: Vec<([u8; 4], BTreeMap<u16, Vec<String>>)> = Vec::new();
    for (i, record) in records.iter().enumerate().take(MAX_RECORDS) {
        let Ok(feature) = record.feature(features.offset_data()) else { continue };
        let tag = record.feature_tag().to_be_bytes();
        let slot = match out.iter().position(|(t, _)| *t == tag) {
            Some(p) => p,
            None => {
                out.push((tag, BTreeMap::new()));
                out.len() - 1
            }
        };
        let Some((_, lookups)) = out.get_mut(slot) else { continue };
        let who = users.get(i).cloned().unwrap_or_default();
        for index in feature.lookup_list_indices().iter().take(MAX_RECORDS) {
            let entry = lookups.entry(index.get()).or_default();
            for w in &who {
                if !entry.contains(w) {
                    entry.push(w.clone());
                }
            }
        }
    }
    out
}

/// The kind of a `GSUB` lookup, whether the sheet draws it, and its rules.
fn gsub_rules(lookup: &SubstitutionLookup<'_>, work: &mut Work) -> (&'static str, bool, Vec<Rule>) {
    let mut rules = Vec::new();
    let push = |input: Vec<u32>, output: Vec<u32>, rules: &mut Vec<Rule>| {
        if rules.len() < MAX_RULES {
            rules.push(Rule::Sub { input, output });
        }
    };
    let ids = |list: &[skrifa::raw::types::BigEndian<skrifa::raw::types::GlyphId16>]| -> Vec<u32> { list.iter().map(|g| g.get().to_u32()).collect() };
    let kind = match lookup.subtables() {
        Ok(SubstitutionSubtables::Single(subtables)) => {
            for subtable in subtables.iter().filter_map(Result::ok) {
                match subtable {
                    SingleSubst::Format1(t) => {
                        let Ok(coverage) = t.coverage() else { continue };
                        let delta = t.delta_glyph_id();
                        for g in coverage.iter() {
                            if !work.take() {
                                break;
                            }
                            push(vec![g.to_u32()], vec![u32::from(g.to_u16().wrapping_add_signed(delta))], &mut rules);
                        }
                    }
                    SingleSubst::Format2(t) => {
                        let Ok(coverage) = t.coverage() else { continue };
                        for (g, out) in coverage.iter().zip(t.substitute_glyph_ids()) {
                            if !work.take() {
                                break;
                            }
                            push(vec![g.to_u32()], vec![out.get().to_u32()], &mut rules);
                        }
                    }
                }
            }
            ("single substitution", true)
        }
        Ok(SubstitutionSubtables::Multiple(subtables)) => {
            for t in subtables.iter().filter_map(Result::ok) {
                let Ok(coverage) = t.coverage() else { continue };
                let sequences = t.sequences();
                for (i, g) in coverage.iter().enumerate() {
                    let Ok(sequence) = sequences.get(i) else { continue };
                    if !work.take() {
                        break;
                    }
                    push(vec![g.to_u32()], ids(sequence.substitute_glyph_ids()), &mut rules);
                }
            }
            ("multiple substitution", true)
        }
        Ok(SubstitutionSubtables::Alternate(subtables)) => {
            for t in subtables.iter().filter_map(Result::ok) {
                let Ok(coverage) = t.coverage() else { continue };
                let sets = t.alternate_sets();
                for (i, g) in coverage.iter().enumerate() {
                    let Ok(set) = sets.get(i) else { continue };
                    if !work.take() {
                        break;
                    }
                    push(vec![g.to_u32()], ids(set.alternate_glyph_ids()), &mut rules);
                }
            }
            ("alternate substitution", true)
        }
        Ok(SubstitutionSubtables::Ligature(subtables)) => {
            for t in subtables.iter().filter_map(Result::ok) {
                let Ok(coverage) = t.coverage() else { continue };
                let sets = t.ligature_sets();
                for (i, g) in coverage.iter().enumerate() {
                    let Ok(set) = sets.get(i) else { continue };
                    for ligature in set.ligatures().iter().filter_map(Result::ok) {
                        if !work.take() {
                            break;
                        }
                        let mut input = vec![g.to_u32()];
                        input.extend(ids(ligature.component_glyph_ids()));
                        push(input, vec![ligature.ligature_glyph().to_u32()], &mut rules);
                    }
                }
            }
            ("ligature substitution", true)
        }
        Ok(SubstitutionSubtables::Contextual(_)) => ("contextual substitution", false),
        Ok(SubstitutionSubtables::ChainContextual(_)) => ("chained contextual substitution", false),
        Ok(SubstitutionSubtables::Reverse(_)) => ("reverse chained substitution", false),
        Ok(SubstitutionSubtables::EmptyExtension) => ("empty extension lookup", false),
        Err(_) => ("unreadable lookup", false),
    };
    (kind.0, kind.1, rules)
}

/// The kind of a `GPOS` lookup, whether the sheet draws it, and its rules.
fn gpos_rules(lookup: &PositionLookup<'_>, work: &mut Work) -> (&'static str, bool, Vec<Rule>) {
    let mut rules = Vec::new();
    let kind = match lookup.subtables() {
        Ok(PositionSubtables::Single(subtables)) => {
            for subtable in subtables.iter().filter_map(Result::ok) {
                match subtable {
                    SinglePos::Format1(t) => {
                        let Ok(coverage) = t.coverage() else { continue };
                        let value = values(&t.value_record());
                        for g in coverage.iter() {
                            if !work.take() || rules.len() >= MAX_RULES {
                                break;
                            }
                            rules.push(Rule::Single { glyph: g.to_u32(), value });
                        }
                    }
                    SinglePos::Format2(t) => {
                        let Ok(coverage) = t.coverage() else { continue };
                        let records = t.value_records();
                        for (i, g) in coverage.iter().enumerate() {
                            let Ok(record) = records.get(i) else { continue };
                            if !work.take() || rules.len() >= MAX_RULES {
                                break;
                            }
                            rules.push(Rule::Single { glyph: g.to_u32(), value: values(&record) });
                        }
                    }
                }
            }
            ("single adjustment", true)
        }
        Ok(PositionSubtables::Pair(subtables)) => {
            for subtable in subtables.iter().filter_map(Result::ok) {
                pair_rules(&subtable, work, &mut rules);
            }
            ("pair adjustment", true)
        }
        Ok(PositionSubtables::Cursive(_)) => ("cursive attachment", false),
        Ok(PositionSubtables::MarkToBase(_)) => ("mark-to-base attachment", false),
        Ok(PositionSubtables::MarkToLig(_)) => ("mark-to-ligature attachment", false),
        Ok(PositionSubtables::MarkToMark(_)) => ("mark-to-mark attachment", false),
        Ok(PositionSubtables::Contextual(_)) => ("contextual positioning", false),
        Ok(PositionSubtables::ChainContextual(_)) => ("chained contextual positioning", false),
        Ok(PositionSubtables::EmptyExtension) => ("empty extension lookup", false),
        Err(_) => ("unreadable lookup", false),
    };
    (kind.0, kind.1, rules)
}

fn pair_rules(subtable: &PairPos<'_>, work: &mut Work, rules: &mut Vec<Rule>) {
    match subtable {
        PairPos::Format1(t) => {
            let Ok(coverage) = t.coverage() else { return };
            let sets = t.pair_sets();
            for (i, first) in coverage.iter().enumerate() {
                let Ok(set) = sets.get(i) else { continue };
                for record in set.pair_value_records().iter().filter_map(Result::ok) {
                    if !work.take() || rules.len() >= MAX_RULES {
                        return;
                    }
                    let value = advance(record.value_record1());
                    rules.push(Rule::Pair { first: first.to_u32(), second: record.second_glyph().to_u32(), value });
                }
            }
        }
        PairPos::Format2(t) => {
            let (Ok(coverage), Ok(class1), Ok(class2)) = (t.coverage(), t.class_def1(), t.class_def2()) else { return };
            let records = t.class_value_records();
            for first in coverage.iter() {
                let c1 = class1.get(first);
                if c1 >= t.class1_count() {
                    continue;
                }
                for (second, c2) in class2.iter() {
                    if !work.take() || rules.len() >= MAX_RULES {
                        return;
                    }
                    if c2 == 0 || c2 >= t.class2_count() {
                        continue;
                    }
                    let Some([v, _]) = records.get(c1, c2) else { continue };
                    let value = advance(&v);
                    if value != 0.0 {
                        rules.push(Rule::Pair { first: first.to_u32(), second: second.to_u32(), value });
                    }
                }
            }
        }
    }
}

/// The advance a value record changes: x, else y (vertical features).
fn advance(v: &ValueRecord<'_>) -> f64 {
    v.x_advance().or(v.y_advance()).map_or(0.0, f64::from)
}

/// x and y placement, x and y advance.
fn values(v: &ValueRecord<'_>) -> [f64; 4] {
    [v.x_placement(), v.y_placement(), v.x_advance(), v.y_advance()].map(|p| p.map_or(0.0, f64::from))
}

/// A single adjustment as feature code writes it: the advance alone, or all four values.
fn value_text(v: [f64; 4]) -> String {
    let [a, b, c, e] = v;
    if a == 0.0 && b == 0.0 && e == 0.0 { num(c) } else { format!("<{} {} {} {}>", num(a), num(b), num(c), num(e)) }
}

/// One rule on the current line of `flow`.
fn draw_rule(face: &Face<'_>, doc: &mut Document, d: &Display, flow: &mut Flow, glyphs: &mut Glyphs, rule: &Rule) -> Result<(), ProofError> {
    let s = flow.size;
    let (plus_w, arrow_w) = (s * 0.5, s * 0.9);
    let colour = d.meta_colour;
    match rule {
        Rule::Sub { input, output } => {
            let mut ins = Vec::with_capacity(input.len());
            for g in input {
                ins.push(glyphs.get(face, doc, *g)?);
            }
            let mut outs = Vec::with_capacity(output.len());
            for g in output {
                outs.push(glyphs.get(face, doc, *g)?);
            }
            let cells: Vec<f64> = ins.iter().chain(&outs).map(|i| flow.cell(doc.glyphs.get(*i).map_or(0.0, |g| g.outline.advance))).collect();
            let width = cells.iter().sum::<f64>() + plus_w * ins.len().saturating_sub(1) as f64 + arrow_w;
            let origin = flow.place(doc, width)?;
            let (mut x, y) = (origin[0], origin[1]);
            let mut cells = cells.into_iter();
            for (k, i) in ins.iter().enumerate() {
                if k > 0 {
                    flow.items.push(Item::Label {
                        x: x + s * 0.1,
                        y: y - s * 0.15,
                        size: s * 0.5,
                        text: "+".into(),
                        anchor: crate::page::Anchor::Start,
                        ink: Ink::Rgb(colour),
                    });
                    x += plus_w;
                }
                glyph_at(face, doc, d, flow, *i, [x, y]);
                x += cells.next().unwrap_or(0.0);
            }
            arrow(flow, [x, y], colour);
            x += arrow_w;
            for i in &outs {
                glyph_at(face, doc, d, flow, *i, [x, y]);
                x += cells.next().unwrap_or(0.0);
            }
        }
        Rule::Single { glyph, value } => {
            let i = glyphs.get(face, doc, *glyph)?;
            let a = doc.glyphs.get(i).map_or(0.0, |g| g.outline.advance);
            let [xp, yp, xa, _] = *value;
            let width = flow.cell(a) + arrow_w + flow.cell(a + xa);
            let origin = flow.place(doc, width)?;
            let (mut x, y) = (origin[0], origin[1]);
            glyph_at(face, doc, d, flow, i, [x, y]);
            x += flow.cell(a);
            arrow(flow, [x, y], colour);
            x += arrow_w;
            glyph_at(face, doc, d, flow, i, [x + xp * flow.scale, y - yp * flow.scale]);
            if d.x_advance && xa != 0.0 {
                let ax = x + (a + xa) * flow.scale;
                flow.items.push(Item::Line { from: [ax, y - s * 0.08], to: [ax, y + s * 0.08], ink: Ink::Rgb(d.x_advance_colour) });
            }
            if d.numeric_labels {
                flow.value([x, y], value_text(*value), colour);
            }
        }
        Rule::Pair { first, second, value } => {
            let (i1, i2) = (glyphs.get(face, doc, *first)?, glyphs.get(face, doc, *second)?);
            let a1 = doc.glyphs.get(i1).map_or(0.0, |g| g.outline.advance);
            let a2 = doc.glyphs.get(i2).map_or(0.0, |g| g.outline.advance);
            let step = (a1 + value).max(0.0) * flow.scale;
            let origin = flow.place(doc, step + flow.cell(a2))?;
            let (x, y) = (origin[0], origin[1]);
            glyph_at(face, doc, d, flow, i1, [x, y]);
            glyph_at(face, doc, d, flow, i2, [x + step, y]);
            if d.numeric_labels {
                flow.value([x + step, y], num(*value), colour);
            }
        }
    }
    Ok(())
}

/// A glyph of a rule with its label.
fn glyph_at(face: &Face<'_>, doc: &Document, d: &Display, flow: &mut Flow, index: usize, origin: [f64; 2]) {
    draw_glyph(face, doc, d, index, origin, flow.scale, &mut flow.items);
    if let Some(text) = doc.glyphs.get(index).and_then(|g| glyph_label(d, g, None)) {
        flow.label(origin, text, d.meta_colour);
    }
}

/// An arrow from the input of a rule to its result, starting at `at` on the baseline.
fn arrow(flow: &mut Flow, at: [f64; 2], colour: [u8; 3]) {
    let s = flow.size;
    let (x, y) = (at[0], at[1] - s * 0.3);
    let ink = Ink::Rgb(colour);
    flow.items.push(Item::Line { from: [x + s * 0.15, y], to: [x + s * 0.75, y], ink });
    flow.items.push(Item::Line { from: [x + s * 0.6, y - s * 0.1], to: [x + s * 0.75, y], ink });
    flow.items.push(Item::Line { from: [x + s * 0.6, y + s * 0.1], to: [x + s * 0.75, y], ink });
}
