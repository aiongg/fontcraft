//! Layout tables read into lines of feature code (the reader behind [`super`]).
//!
//! Ported from `fontcraft_tables::fea_write` (M12.12), which `tables` builds on top of this
//! crate and so cannot share: the same rules and statements, read through `skrifa`'s tables,
//! with the document's glyph names. The lines are kept apart per lookup, per feature and per
//! block, so [`super`] can lay them out as entries; [`Writer::render`] turns them into text and
//! names the glyph sets that repeat.
//!
//! Input is untrusted: every record, glyph and rule read is charged to [`MAX_LAYOUT_WORK`],
//! the text is capped at [`super::MAX_TEXT`] bytes, and lookup nesting is followed at most
//! [`super::MAX_NESTING`] deep.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use skrifa::raw::tables::base::{Base, MinMax};
use skrifa::raw::tables::gdef::{CaretValue, Gdef};
use skrifa::raw::tables::gpos::{AnchorTable, MarkArray, PairPos, PositionLookup, PositionSubtables, SinglePos, ValueFormat, ValueRecord};
use skrifa::raw::tables::gsub::{SingleSubst, SubstitutionLookup, SubstitutionSubtables};
use skrifa::raw::tables::layout::{
    ChainedSequenceContext, ClassDef, CoverageTable, DeviceOrVariationIndex, FeatureList, FeatureParams, LangSys, ScriptList, SequenceContext,
    SequenceLookupRecord,
};
use skrifa::raw::types::{BigEndian, GlyphId16, Tag};
use skrifa::raw::{FontRef, ReadError, TableProvider};

use super::{MAX_NESTING, MAX_TEXT};

/// Most units of work (records, glyphs, rules) spent reading one font's layout tables.
pub const MAX_LAYOUT_WORK: usize = 50_000_000;

/// A table or feature tag as bytes.
pub(crate) type T4 = [u8; 4];

pub(crate) const GSUB: T4 = *b"GSUB";
pub(crate) const GPOS: T4 = *b"GPOS";
pub(crate) const DFLT_LANG: T4 = *b"dflt";

/// Why the layout could not be read.
#[derive(Debug)]
pub(crate) struct FeaError(pub String);

pub(crate) type R<T> = Result<T, FeaError>;

fn read_err(what: &str, e: ReadError) -> FeaError {
    FeaError(format!("the {what} cannot be read: {e}"))
}

/// A tag as feature code writes it: trailing spaces dropped (`TRK ` is `TRK`).
pub(crate) fn fea_tag(tag: T4) -> String {
    String::from_utf8_lossy(&tag).trim_end().to_string()
}

/// A tag as part of a name: other characters than a name can hold become `_`.
fn sanitize_tag(tag: T4) -> String {
    sanitize(&fea_tag(tag))
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect()
}

/// The work left for one run; every record, glyph and rule read is charged before it is used.
pub(crate) struct Budget {
    left: usize,
}

impl Budget {
    fn new() -> Self {
        Self { left: MAX_LAYOUT_WORK }
    }

    fn charge(&mut self, units: usize) -> R<()> {
        match self.left.checked_sub(units) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                self.left = 0;
                Err(FeaError(format!("the layout tables need more than {MAX_LAYOUT_WORK} units of work to read")))
            }
        }
    }
}

/// Names the compiler reads as keywords; a glyph with one of these names is written escaped.
const KEYWORDS: &[&str] = &[
    "anchor",
    "anchorDef",
    "anon",
    "anonymous",
    "base",
    "by",
    "contour",
    "contourpoint",
    "cursive",
    "cvParameters",
    "device",
    "enum",
    "enumerate",
    "exclude_dflt",
    "excludeDFLT",
    "feature",
    "featureNames",
    "from",
    "ignore",
    "IgnoreBaseGlyphs",
    "IgnoreLigatures",
    "IgnoreMarks",
    "include",
    "include_dflt",
    "includeDFLT",
    "language",
    "languagesystem",
    "ligature",
    "ligComponent",
    "lookup",
    "lookupflag",
    "mark",
    "MarkAttachmentType",
    "markClass",
    "nameid",
    "NULL",
    "parameters",
    "pos",
    "position",
    "required",
    "reversesub",
    "RightToLeft",
    "rsub",
    "script",
    "sizemenuname",
    "sub",
    "substitute",
    "subtable",
    "table",
    "useExtension",
    "UseMarkFilteringSet",
    "valueRecordDef",
];

/// The glyph name as feature code writes it, and whether feature syntax can hold it at all.
fn fea_glyph_name(name: &str) -> (String, bool) {
    let start = |c: char| c.is_ascii_alphabetic() || "_+*:.^~!".contains(c);
    let cont = |c: char| c.is_ascii_alphanumeric() || "_.+*:^~!/-".contains(c);
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(start) && chars.all(cont);
    if KEYWORDS.contains(&name) { (format!("\\{name}"), valid) } else { (name.to_string(), valid) }
}

// ── the text being built ─────────────────────────────────────────────────────────────────────

pub(crate) enum Piece {
    Text(String),
    Glyph(u16),
    /// A glyph set by id; `true` writes a single glyph in brackets (a class).
    Set(usize, bool),
}

pub(crate) struct Line {
    pub indent: usize,
    pub pieces: Vec<Piece>,
    /// Lookups (of the lookup's own table) this rule calls.
    pub refs: Vec<u16>,
}

impl Line {
    pub fn text(indent: usize, s: String) -> Self {
        Self { indent, pieces: vec![Piece::Text(s)], refs: Vec::new() }
    }

    fn new(indent: usize, pieces: Vec<Piece>) -> Self {
        Self { indent, pieces, refs: Vec::new() }
    }

    /// The `subtable;` break between two subtables.
    pub fn is_subtable_break(&self) -> bool {
        matches!(self.pieces.as_slice(), [Piece::Text(t)] if t == "subtable;")
    }

    /// A rule, not a comment or a subtable break.
    fn is_rule(&self) -> bool {
        !self.is_subtable_break() && !matches!(self.pieces.first(), Some(Piece::Text(t)) if t.starts_with('#'))
    }
}

/// Removes the calls of contextual rules to the lookups of `empty` (lookups without rules). Such
/// a call does nothing, and the feature compiler cannot give the called lookup a type. A rule
/// left without calls still stops the lookup where it matches, which an `ignore` rule does too.
fn drop_empty_calls(lookups: &mut HashMap<u16, LookupOut>, empty: &BTreeSet<u16>, calls: &HashSet<String>) {
    for l in lookups.values_mut() {
        for line in &mut l.body {
            if !line.refs.iter().any(|r| empty.contains(r)) {
                continue;
            }
            line.refs.retain(|r| !empty.contains(r));
            line.pieces.retain(|p| !matches!(p, Piece::Text(t) if calls.contains(t)));
            if line.refs.is_empty()
                && let Some(Piece::Text(kw)) = line.pieces.first_mut()
                && !kw.starts_with("ignore ")
            {
                *kw = format!("ignore {kw}");
            }
        }
    }
}

/// One lookup read into lines.
pub(crate) struct LookupOut {
    pub table: T4,
    pub index: u16,
    pub name: String,
    /// The lookup type (extension lookups read through).
    pub kind: u16,
    pub flag: Option<Line>,
    /// `markClass` statements the lookup's rules need.
    pub pre: Vec<Line>,
    pub body: Vec<Line>,
    /// Lookups its contextual rules call.
    pub refs: BTreeSet<u16>,
}

/// (not DFLT, script, not dflt, language): sorts `DFLT` and `dflt` first.
pub(crate) type SysKey = (bool, T4, bool, T4);

fn sys_key(script: T4, language: T4) -> SysKey {
    (&script != b"DFLT", script, language != DFLT_LANG, language)
}

/// A feature's lookups per language system: (required, lookups by table).
pub(crate) type Systems = BTreeMap<SysKey, (bool, Vec<(T4, u16)>)>;

/// Everything read from the layout tables, before it is laid out.
#[derive(Default)]
pub(crate) struct Layout {
    /// The language systems the features use, `DFLT dflt` first.
    pub systems: BTreeSet<SysKey>,
    /// Notes on what the tables hold that has no feature syntax.
    pub head: Vec<Line>,
    pub features: BTreeMap<T4, Systems>,
    /// Feature parameters written as statements (`size`), or notes.
    pub params: BTreeMap<T4, Vec<Line>>,
    /// The name ID of a stylistic set's name.
    pub set_names: BTreeMap<T4, u16>,
    /// Every lookup, `GSUB` first, each table in lookup-list order except that a lookup a
    /// contextual rule calls comes before the rule.
    pub lookups: Vec<LookupOut>,
    pub gdef: Vec<Line>,
    pub base: Vec<Line>,
}

/// One class definition read into sets.
struct Classes {
    by: BTreeMap<u16, Vec<u16>>,
    /// Every glyph of the font no class holds (class 0).
    zero: Vec<u16>,
}

impl Classes {
    fn glyphs(&self, class: u16) -> Vec<u16> {
        if class == 0 { self.zero.clone() } else { self.by.get(&class).cloned().unwrap_or_default() }
    }
}

/// A contextual rule with its glyph sets in logical order.
struct CtxRule {
    back: Vec<Vec<u16>>,
    input: Vec<Vec<u16>>,
    look: Vec<Vec<u16>>,
    records: Vec<(u16, u16)>,
}

/// What [`Writer::render`] gives.
pub(crate) struct Rendered {
    /// Named glyph classes: name without `@`, members separated by spaces.
    pub classes: Vec<(String, String)>,
    /// The text of each chunk, in the order given.
    pub texts: Vec<String>,
    /// One sentence per construct without feature syntax.
    pub unsupported: Vec<String>,
    /// Glyph names used in the text that feature syntax cannot hold.
    pub bad_names: Vec<String>,
    /// Glyph ids used in the text that the font does not have (no name can refer to them).
    pub beyond: Vec<u16>,
}

pub(crate) struct Writer<'a> {
    raw: &'a [String],
    names: Vec<String>,
    budget: Budget,
    count: usize,
    sets: Vec<Vec<u16>>,
    set_ids: HashMap<Vec<u16>, usize>,
    unsupported: Vec<String>,
    gdef: Option<Gdef<'a>>,
    lookup_names: HashMap<T4, Vec<String>>,
    used_lookup_names: Taken,
    mark_class_names: HashSet<String>,
}

impl<'a> Writer<'a> {
    /// A writer for a font whose glyphs are named `names`, by glyph id.
    pub fn new(names: &'a [String]) -> Self {
        Self {
            raw: names,
            names: names.iter().map(|n| fea_glyph_name(n).0).collect(),
            budget: Budget::new(),
            count: names.len(),
            sets: Vec::new(),
            set_ids: HashMap::new(),
            unsupported: Vec::new(),
            gdef: None,
            lookup_names: HashMap::new(),
            used_lookup_names: Taken::new(HashSet::new()),
            mark_class_names: HashSet::new(),
        }
    }

    /// Records a construct without feature syntax and returns its comment line.
    pub fn note(&mut self, what: &str) -> Line {
        self.unsupported.push(what.to_string());
        Line::text(0, format!("# not written: {what}"))
    }

    pub fn lookup_name(&self, table: T4, li: u16) -> String {
        self.lookup_names.get(&table).and_then(|v| v.get(usize::from(li))).cloned().unwrap_or_else(|| format!("{}_{li}", fea_tag(table)))
    }

    /// Reads `GSUB`, `GPOS`, `GDEF` and `BASE`. With `exclude`, those features and the lookups
    /// only they reach are left out.
    pub fn read(&mut self, font: &FontRef<'a>, exclude: &[T4]) -> R<Layout> {
        let mut out = Layout::default();
        self.gdef = font.gdef().ok();
        for table in [GSUB, GPOS] {
            let Some((scripts, features)) = layout_lists(font, table)? else { continue };
            if has_feature_variations(font, table) {
                let line = self.note(&format!("the {} feature variations (alternate lookups chosen by axis location)", fea_tag(table)));
                out.head.push(line);
            }
            let systems = language_systems(&scripts, &mut self.budget)?;
            let count = lookup_count(font, table)?;
            let mut names: Vec<Option<String>> = vec![None; count];
            let mut roots: Vec<u16> = Vec::new();
            for (script, language, required, indices) in &systems {
                let indices: Vec<(u16, bool)> = required.map(|r| (r, true)).into_iter().chain(indices.iter().map(|f| (*f, false))).collect();
                for (index, required) in indices {
                    let (tag, ids) = feature_at(&features, index, &mut self.budget)?;
                    if exclude.contains(&tag) {
                        continue;
                    }
                    let params = features
                        .feature_records()
                        .get(usize::from(index))
                        .and_then(|r| r.feature(features.offset_data()).ok())
                        .and_then(|f| f.feature_params())
                        .and_then(Result::ok);
                    if let Some(p) = params
                        && !out.params.contains_key(&tag)
                        && !out.set_names.contains_key(&tag)
                    {
                        match &p {
                            FeatureParams::StylisticSet(s) if tag.starts_with(b"ss") => {
                                out.set_names.insert(tag, s.ui_name_id().to_u16());
                            }
                            _ => {
                                let lines = self.feature_params(tag, &p);
                                out.params.insert(tag, lines);
                            }
                        }
                    }
                    out.systems.insert(sys_key(*script, *language));
                    let entry = out.features.entry(tag).or_default().entry(sys_key(*script, *language)).or_insert((false, Vec::new()));
                    entry.0 |= required;
                    for li in ids {
                        let Some(slot) = names.get_mut(usize::from(li)) else {
                            let line = self.note(&format!(
                                "feature '{}' names {} lookup {li}, which the lookup list does not have",
                                fea_tag(tag),
                                fea_tag(table)
                            ));
                            out.head.push(line);
                            continue;
                        };
                        if slot.is_none() {
                            *slot = Some(format!("{}_{}_{}_{li}", sanitize_tag(tag), sanitize_tag(*script), sanitize_tag(*language)));
                        }
                        roots.push(li);
                        entry.1.push((table, li));
                    }
                }
            }
            let names: Vec<String> = names
                .into_iter()
                .enumerate()
                .map(|(i, n)| n.unwrap_or_else(|| format!("{}_{i}", fea_tag(table))))
                .map(|n| self.used_lookup_names.claim(n))
                .collect();
            self.lookup_names.insert(table, names);
            let candidates: Vec<u16> = if exclude.is_empty() { (0..count).filter_map(|i| u16::try_from(i).ok()).collect() } else { roots };
            let mut outs: HashMap<u16, LookupOut> = HashMap::new();
            let mut queue = candidates;
            while let Some(li) = queue.pop() {
                if outs.contains_key(&li) {
                    continue;
                }
                let lookup = self.lookup(font, table, li)?;
                queue.extend(lookup.refs.iter().copied().filter(|r| usize::from(*r) < count));
                outs.insert(li, lookup);
            }
            let empty: BTreeSet<u16> = outs.iter().filter(|(_, l)| !l.body.iter().any(Line::is_rule)).map(|(li, _)| *li).collect();
            if !empty.is_empty() {
                let calls: HashSet<String> = empty.iter().map(|li| format!(" lookup {}", self.lookup_name(table, *li))).collect();
                drop_empty_calls(&mut outs, &empty, &calls);
            }
            let mut order = Vec::new();
            let mut state: HashMap<u16, bool> = HashMap::new();
            let mut sorted: Vec<u16> = outs.keys().copied().collect();
            sorted.sort_unstable();
            for li in sorted {
                visit(li, &outs, &mut state, &mut order, 0);
            }
            for li in order {
                if let Some(lookup) = outs.remove(&li) {
                    out.lookups.push(lookup);
                }
            }
        }
        if let Some(g) = self.gdef.clone() {
            out.gdef = self.gdef_block(&g)?;
        }
        if let Ok(base) = font.base() {
            out.base = self.base_block(&base)?;
        }
        Ok(out)
    }

    fn set(&mut self, mut glyphs: Vec<u16>) -> usize {
        glyphs.sort_unstable();
        glyphs.dedup();
        if let Some(id) = self.set_ids.get(&glyphs) {
            return *id;
        }
        let id = self.sets.len();
        self.sets.push(glyphs.clone());
        self.set_ids.insert(glyphs, id);
        id
    }

    /// A glyph set as one piece: a single glyph plain unless `class` asks for brackets.
    fn set_piece(&mut self, glyphs: Vec<u16>, class: bool) -> Piece {
        if !class
            && glyphs.len() == 1
            && let Some(g) = glyphs.first()
        {
            return Piece::Glyph(*g);
        }
        Piece::Set(self.set(glyphs), class)
    }

    fn coverage(&mut self, cov: Result<CoverageTable, ReadError>, what: &str) -> R<Vec<u16>> {
        let cov = cov.map_err(|e| read_err(what, e))?;
        let mut out = Vec::new();
        for g in cov.iter() {
            self.budget.charge(1)?;
            out.push(g.to_u16());
        }
        Ok(out)
    }

    fn classes(&mut self, cd: Result<ClassDef, ReadError>, what: &str) -> R<Classes> {
        let cd = cd.map_err(|e| read_err(what, e))?;
        let mut seen: HashSet<u16> = HashSet::new();
        let mut by: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
        for (g, class) in cd.iter() {
            self.budget.charge(1)?;
            let g = g.to_u16();
            // A class can list glyph ids past the font's glyphs; no glyph run holds them.
            if class != 0 && usize::from(g) < self.count && seen.insert(g) {
                by.entry(class).or_default().push(g);
            }
        }
        self.budget.charge(self.count)?;
        let zero = (0..self.count).filter_map(|g| u16::try_from(g).ok()).filter(|g| !seen.contains(g)).collect();
        Ok(Classes { by, zero })
    }

    // ── lookups ──────────────────────────────────────────────────────────────────────────────

    fn lookup(&mut self, font: &FontRef<'a>, table: T4, li: u16) -> R<LookupOut> {
        let name = self.lookup_name(table, li);
        let mut out = LookupOut { table, index: li, name, kind: 0, flag: None, pre: Vec::new(), body: Vec::new(), refs: BTreeSet::new() };
        let what = format!("{} lookup {li}", fea_tag(table));
        if table == GSUB {
            let list = font.gsub().and_then(|t| t.lookup_list()).map_err(|e| read_err("GSUB lookup list", e))?;
            let lookup: SubstitutionLookup = list.lookups().get(usize::from(li)).map_err(|e| read_err(&what, e))?;
            out.flag = self.flag_line(lookup.lookup_flag().to_bits(), lookup.mark_filtering_set());
            let subtables = lookup.subtables().map_err(|e| read_err(&what, e))?;
            out.kind = gsub_kind(&subtables);
            self.gsub_body(&subtables, &mut out)?;
        } else {
            let list = font.gpos().and_then(|t| t.lookup_list()).map_err(|e| read_err("GPOS lookup list", e))?;
            let lookup: PositionLookup = list.lookups().get(usize::from(li)).map_err(|e| read_err(&what, e))?;
            out.flag = self.flag_line(lookup.lookup_flag().to_bits(), lookup.mark_filtering_set());
            let subtables = lookup.subtables().map_err(|e| read_err(&what, e))?;
            out.kind = gpos_kind(&subtables);
            self.gpos_body(&subtables, &mut out)?;
        }
        for line in &out.body {
            out.refs.extend(line.refs.iter().copied());
        }
        Ok(out)
    }

    fn flag_line(&mut self, flag: u16, filter: Option<u16>) -> Option<Line> {
        if flag == 0 {
            return None;
        }
        let mut pieces = vec![Piece::Text("lookupflag".into())];
        for (bit, name) in [(1, "RightToLeft"), (2, "IgnoreBaseGlyphs"), (4, "IgnoreLigatures"), (8, "IgnoreMarks")] {
            if flag & bit != 0 {
                pieces.push(Piece::Text(format!(" {name}")));
            }
        }
        let attach = flag >> 8;
        if attach != 0 {
            let glyphs = self
                .gdef
                .as_ref()
                .and_then(|g| g.mark_attach_class_def())
                .and_then(Result::ok)
                .map(|cd| cd.iter().filter(|(_, c)| *c == attach).map(|(g, _)| g.to_u16()).collect::<Vec<u16>>());
            match glyphs.filter(|g| !g.is_empty() && self.budget.charge(g.len()).is_ok()) {
                Some(glyphs) => {
                    pieces.push(Piece::Text(" MarkAttachmentType ".into()));
                    let piece = self.set_piece(glyphs, true);
                    pieces.push(piece);
                }
                None => self.unsupported.push(format!("mark attachment type {attach}, which the GDEF table does not define")),
            }
        }
        if flag & 0x10 != 0 {
            let set = filter.and_then(|i| {
                let sets = self.gdef.as_ref()?.mark_glyph_sets_def()?.ok()?;
                let cov = sets.coverages().get(usize::from(i)).ok()?;
                Some(cov.iter().map(|g| g.to_u16()).collect::<Vec<u16>>())
            });
            match set.filter(|g| !g.is_empty() && self.budget.charge(g.len()).is_ok()) {
                Some(glyphs) => {
                    pieces.push(Piece::Text(" UseMarkFilteringSet ".into()));
                    let piece = self.set_piece(glyphs, true);
                    pieces.push(piece);
                }
                None => self.unsupported.push("a mark filtering set the GDEF table does not define".into()),
            }
        }
        if flag & 0x00E0 != 0 {
            self.unsupported.push(format!("reserved lookup flag bits 0x{:04X}", flag & 0x00E0));
        }
        if pieces.len() == 1 {
            pieces.push(Piece::Text(" 0".into()));
        }
        pieces.push(Piece::Text(";".into()));
        Some(Line::new(1, pieces))
    }

    fn subtable_break(out: &mut LookupOut, i: usize) {
        if i > 0 {
            out.body.push(Line::text(1, "subtable;".into()));
        }
    }

    fn gsub_body(&mut self, subtables: &SubstitutionSubtables<'a>, out: &mut LookupOut) -> R<()> {
        let what = format!("{} subtable", out.name);
        match subtables {
            SubstitutionSubtables::Single(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let pairs: Vec<(u16, u16)> = match st.map_err(|e| read_err(&what, e))? {
                        SingleSubst::Format1(f) => {
                            let delta = i32::from(f.delta_glyph_id());
                            let cov = self.coverage(f.coverage(), &what)?;
                            cov.into_iter().map(|g| (g, u16::try_from((i32::from(g) + delta).rem_euclid(65_536)).unwrap_or(0))).collect()
                        }
                        SingleSubst::Format2(f) => {
                            let cov = self.coverage(f.coverage(), &what)?;
                            cov.into_iter().zip(f.substitute_glyph_ids().iter().map(|g| g.get().to_u16())).collect()
                        }
                    };
                    for (from, to) in pairs {
                        out.body.push(Line::new(
                            1,
                            vec![
                                Piece::Text("sub ".into()),
                                Piece::Glyph(from),
                                Piece::Text(" by ".into()),
                                Piece::Glyph(to),
                                Piece::Text(";".into()),
                            ],
                        ));
                    }
                }
            }
            SubstitutionSubtables::Multiple(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    for (g, seq) in cov.into_iter().zip(st.sequences().iter()) {
                        let seq = seq.map_err(|e| read_err(&what, e))?;
                        let ids = seq.substitute_glyph_ids();
                        self.budget.charge(ids.len())?;
                        let mut pieces = vec![Piece::Text("sub ".into()), Piece::Glyph(g), Piece::Text(" by".into())];
                        if ids.is_empty() {
                            pieces.push(Piece::Text(" NULL".into()));
                        }
                        for id in ids {
                            pieces.push(Piece::Text(" ".into()));
                            pieces.push(Piece::Glyph(id.get().to_u16()));
                        }
                        pieces.push(Piece::Text(";".into()));
                        out.body.push(Line::new(1, pieces));
                    }
                }
            }
            SubstitutionSubtables::Alternate(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    for (g, set) in cov.into_iter().zip(st.alternate_sets().iter()) {
                        let set = set.map_err(|e| read_err(&what, e))?;
                        let ids = set.alternate_glyph_ids();
                        self.budget.charge(ids.len())?;
                        if ids.is_empty() {
                            continue;
                        }
                        // The order of alternates matters, so they are written as given.
                        let mut pieces = vec![Piece::Text("sub ".into()), Piece::Glyph(g), Piece::Text(" from [".into())];
                        for (k, id) in ids.iter().enumerate() {
                            if k > 0 {
                                pieces.push(Piece::Text(" ".into()));
                            }
                            pieces.push(Piece::Glyph(id.get().to_u16()));
                        }
                        pieces.push(Piece::Text("];".into()));
                        out.body.push(Line::new(1, pieces));
                    }
                }
            }
            SubstitutionSubtables::Ligature(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    for (g, set) in cov.into_iter().zip(st.ligature_sets().iter()) {
                        let set = set.map_err(|e| read_err(&what, e))?;
                        // A shaper takes the first ligature of a set that matches, so a later one
                        // with the same components never applies; the feature compiler refuses it.
                        let mut seen: HashSet<Vec<u16>> = HashSet::new();
                        for lig in set.ligatures().iter() {
                            let lig = lig.map_err(|e| read_err(&what, e))?;
                            let comps = lig.component_glyph_ids();
                            self.budget.charge(comps.len().saturating_add(1))?;
                            if !seen.insert(comps.iter().map(|c| c.get().to_u16()).collect()) {
                                continue;
                            }
                            let mut pieces = vec![Piece::Text("sub ".into()), Piece::Glyph(g)];
                            for c in comps {
                                pieces.push(Piece::Text(" ".into()));
                                pieces.push(Piece::Glyph(c.get().to_u16()));
                            }
                            pieces.extend([Piece::Text(" by ".into()), Piece::Glyph(lig.ligature_glyph().to_u16()), Piece::Text(";".into())]);
                            out.body.push(Line::new(1, pieces));
                        }
                    }
                }
            }
            SubstitutionSubtables::Contextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.sequence_rules(&st, &what)?;
                    self.ctx_lines("sub", rules, out)?;
                }
            }
            SubstitutionSubtables::ChainContextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.chain_rules(&st, &what)?;
                    self.ctx_lines("sub", rules, out)?;
                }
            }
            SubstitutionSubtables::Reverse(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    let subs: Vec<u16> = st.substitute_glyph_ids().iter().map(|g| g.get().to_u16()).collect();
                    let mut back = Vec::new();
                    for c in st.backtrack_coverages().iter() {
                        back.push(self.coverage(c, &what)?);
                    }
                    back.reverse();
                    let mut look = Vec::new();
                    for c in st.lookahead_coverages().iter() {
                        look.push(self.coverage(c, &what)?);
                    }
                    if cov.is_empty() || back.iter().chain(&look).any(Vec::is_empty) {
                        continue;
                    }
                    let mut pieces = vec![Piece::Text("rsub".into())];
                    for set in back {
                        pieces.push(Piece::Text(" ".into()));
                        pieces.push(self.set_piece(set, false));
                    }
                    // Input and replacement in coverage order, pairwise.
                    let pairs: Vec<(u16, u16)> = cov.iter().copied().zip(subs.iter().copied()).collect();
                    pieces.push(Piece::Text(" [".into()));
                    for (k, (g, _)) in pairs.iter().enumerate() {
                        if k > 0 {
                            pieces.push(Piece::Text(" ".into()));
                        }
                        pieces.push(Piece::Glyph(*g));
                    }
                    pieces.push(Piece::Text("]'".into()));
                    for set in look {
                        pieces.push(Piece::Text(" ".into()));
                        pieces.push(self.set_piece(set, false));
                    }
                    pieces.push(Piece::Text(" by [".into()));
                    for (k, (_, s)) in pairs.iter().enumerate() {
                        if k > 0 {
                            pieces.push(Piece::Text(" ".into()));
                        }
                        pieces.push(Piece::Glyph(*s));
                    }
                    pieces.push(Piece::Text("];".into()));
                    out.body.push(Line::new(1, pieces));
                }
            }
            SubstitutionSubtables::EmptyExtension => {
                let mut line = self.note(&format!("lookup {} is an extension lookup without subtables", out.name));
                line.indent = 1;
                out.body.push(line);
            }
        }
        Ok(())
    }

    fn gpos_body(&mut self, subtables: &PositionSubtables<'a>, out: &mut LookupOut) -> R<()> {
        let what = format!("{} subtable", out.name);
        match subtables {
            PositionSubtables::Single(s) => {
                for st in s.iter() {
                    match st.map_err(|e| read_err(&what, e))? {
                        SinglePos::Format1(f) => {
                            let cov = self.coverage(f.coverage(), &what)?;
                            if cov.is_empty() {
                                continue;
                            }
                            let v = self.value(&f.value_record(), true);
                            let set = self.set_piece(cov, false);
                            out.body.push(Line::new(1, vec![Piece::Text("pos ".into()), set, Piece::Text(format!(" {v};"))]));
                        }
                        SinglePos::Format2(f) => {
                            let cov = self.coverage(f.coverage(), &what)?;
                            for (g, vr) in cov.into_iter().zip(f.value_records().iter()) {
                                let vr = vr.map_err(|e| read_err(&what, e))?;
                                let v = self.value(&vr, true);
                                out.body.push(Line::new(1, vec![Piece::Text("pos ".into()), Piece::Glyph(g), Piece::Text(format!(" {v};"))]));
                            }
                        }
                    }
                }
            }
            PositionSubtables::Pair(s) => {
                let many = s.len() > 1;
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    match st.map_err(|e| read_err(&what, e))? {
                        PairPos::Format1(f) => {
                            let cov = self.coverage(f.coverage(), &what)?;
                            for (g, set) in cov.into_iter().zip(f.pair_sets().iter()) {
                                let set = set.map_err(|e| read_err(&what, e))?;
                                for rec in set.pair_value_records().iter() {
                                    self.budget.charge(1)?;
                                    let rec = rec.map_err(|e| read_err(&what, e))?;
                                    let line = self.pair_line(
                                        Piece::Glyph(g),
                                        Piece::Glyph(rec.second_glyph().to_u16()),
                                        rec.value_record1(),
                                        rec.value_record2(),
                                    );
                                    out.body.push(line);
                                }
                            }
                        }
                        PairPos::Format2(f) => {
                            let cov = self.coverage(f.coverage(), &what)?;
                            let cd1 = self.classes(f.class_def1(), &what)?;
                            let cd2 = self.classes(f.class_def2(), &what)?;
                            let n1 = usize::from(f.class1_count());
                            let n2 = usize::from(f.class2_count());
                            self.budget.charge(n1.saturating_mul(n2))?;
                            let covered: HashSet<u16> = cov.iter().copied().collect();
                            for (c1, rec1) in f.class1_records().iter().enumerate().take(n1) {
                                let rec1 = rec1.map_err(|e| read_err(&what, e))?;
                                let first: Vec<u16> =
                                    cd1.glyphs(u16::try_from(c1).unwrap_or(u16::MAX)).into_iter().filter(|g| covered.contains(g)).collect();
                                if first.is_empty() {
                                    continue;
                                }
                                let mut wrote = false;
                                for (c2, rec2) in rec1.class2_records().iter().enumerate().take(n2) {
                                    let rec2 = rec2.map_err(|e| read_err(&what, e))?;
                                    if is_zero(rec2.value_record1()) && is_zero(rec2.value_record2()) {
                                        continue;
                                    }
                                    let second = cd2.glyphs(u16::try_from(c2).unwrap_or(u16::MAX));
                                    if second.is_empty() {
                                        continue;
                                    }
                                    let a = self.set_piece(first.clone(), true);
                                    let b = self.set_piece(second, true);
                                    let line = self.pair_line(a, b, rec2.value_record1(), rec2.value_record2());
                                    out.body.push(line);
                                    wrote = true;
                                }
                                // A covered first glyph stops later subtables even with a zero value.
                                if !wrote
                                    && many
                                    && let Some(second) = (1..n2).filter_map(|c| u16::try_from(c).ok()).map(|c| cd2.glyphs(c)).find(|g| !g.is_empty())
                                {
                                    let a = self.set_piece(first.clone(), true);
                                    let b = self.set_piece(second, true);
                                    out.body.push(Line::new(
                                        1,
                                        vec![Piece::Text("pos ".into()), a, Piece::Text(" ".into()), b, Piece::Text(" 0;".into())],
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            PositionSubtables::Cursive(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    let data = st.offset_data();
                    for (g, rec) in cov.into_iter().zip(st.entry_exit_record().iter()) {
                        let entry = self.anchor(rec.entry_anchor(data));
                        let exit = self.anchor(rec.exit_anchor(data));
                        out.body
                            .push(Line::new(1, vec![Piece::Text("pos cursive ".into()), Piece::Glyph(g), Piece::Text(format!(" {entry} {exit};"))]));
                    }
                }
            }
            PositionSubtables::MarkToBase(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let classes = self.mark_classes(out, i, st.mark_coverage(), st.mark_array(), &what)?;
                    let bases = self.coverage(st.base_coverage(), &what)?;
                    let array = st.base_array().map_err(|e| read_err(&what, e))?;
                    let data = array.offset_data();
                    let mut rows: Vec<(u16, Vec<String>)> = Vec::new();
                    for (g, rec) in bases.into_iter().zip(array.base_records().iter()) {
                        let rec = rec.map_err(|e| read_err(&what, e))?;
                        let anchors = rec.base_anchors(data);
                        let parts = self.attach_parts(&classes, |c| anchors.get(c))?;
                        if !parts.is_empty() {
                            rows.push((g, parts));
                        }
                    }
                    self.group_attach(out, "pos base ", rows);
                }
            }
            PositionSubtables::MarkToMark(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let classes = self.mark_classes(out, i, st.mark1_coverage(), st.mark1_array(), &what)?;
                    let bases = self.coverage(st.mark2_coverage(), &what)?;
                    let array = st.mark2_array().map_err(|e| read_err(&what, e))?;
                    let data = array.offset_data();
                    let mut rows: Vec<(u16, Vec<String>)> = Vec::new();
                    for (g, rec) in bases.into_iter().zip(array.mark2_records().iter()) {
                        let rec = rec.map_err(|e| read_err(&what, e))?;
                        let anchors = rec.mark2_anchors(data);
                        let parts = self.attach_parts(&classes, |c| anchors.get(c))?;
                        if !parts.is_empty() {
                            rows.push((g, parts));
                        }
                    }
                    self.group_attach(out, "pos mark ", rows);
                }
            }
            PositionSubtables::MarkToLig(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let classes = self.mark_classes(out, i, st.mark_coverage(), st.mark_array(), &what)?;
                    let ligs = self.coverage(st.ligature_coverage(), &what)?;
                    let array = st.ligature_array().map_err(|e| read_err(&what, e))?;
                    for (g, attach) in ligs.into_iter().zip(array.ligature_attaches().iter()) {
                        let attach = attach.map_err(|e| read_err(&what, e))?;
                        let data = attach.offset_data();
                        let mut comps = Vec::new();
                        for rec in attach.component_records().iter() {
                            let rec = rec.map_err(|e| read_err(&what, e))?;
                            let anchors = rec.ligature_anchors(data);
                            let parts = self.attach_parts(&classes, |c| anchors.get(c))?;
                            comps.push(if parts.is_empty() { "<anchor NULL>".to_string() } else { parts.join(" ") });
                        }
                        if comps.is_empty() {
                            continue;
                        }
                        out.body.push(Line::new(
                            1,
                            vec![Piece::Text("pos ligature ".into()), Piece::Glyph(g), Piece::Text(format!(" {};", comps.join(" ligComponent ")))],
                        ));
                    }
                }
            }
            PositionSubtables::Contextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.sequence_rules(&st, &what)?;
                    self.ctx_lines("pos", rules, out)?;
                }
            }
            PositionSubtables::ChainContextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    Self::subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.chain_rules(&st, &what)?;
                    self.ctx_lines("pos", rules, out)?;
                }
            }
            PositionSubtables::EmptyExtension => {
                let mut line = self.note(&format!("lookup {} is an extension lookup without subtables", out.name));
                line.indent = 1;
                out.body.push(line);
            }
        }
        Ok(())
    }

    fn pair_line(&mut self, a: Piece, b: Piece, v1: &ValueRecord, v2: &ValueRecord) -> Line {
        let mut pieces = vec![Piece::Text("pos ".into()), a];
        if v2.format().is_empty() {
            let v = self.value(v1, true);
            pieces.extend([Piece::Text(" ".into()), b, Piece::Text(format!(" {v};"))]);
        } else {
            let first = if v1.format().is_empty() { "<NULL>".to_string() } else { self.value(v1, false) };
            let second = self.value(v2, false);
            pieces.extend([Piece::Text(format!(" {first} ")), b, Piece::Text(format!(" {second};"))]);
        }
        Line::new(1, pieces)
    }

    /// A value record: a bare number when it holds only an x advance and `short` allows it.
    fn value(&mut self, vr: &ValueRecord, short: bool) -> String {
        let (xp, yp, xa, ya) = (vr.x_placement(), vr.y_placement(), vr.x_advance(), vr.y_advance());
        let devices = [vr.x_placement_device(), vr.y_placement_device(), vr.x_advance_device(), vr.y_advance_device()];
        let has_device = devices.iter().any(Option::is_some);
        if short && !has_device && xp.is_none() && yp.is_none() && ya.is_none() {
            return xa.unwrap_or(0).to_string();
        }
        let nums = format!("{} {} {} {}", xp.unwrap_or(0), yp.unwrap_or(0), xa.unwrap_or(0), ya.unwrap_or(0));
        if !has_device {
            return format!("<{nums}>");
        }
        let devs: Vec<String> = devices.into_iter().map(|d| self.device(d)).collect();
        format!("<{nums} {}>", devs.join(" "))
    }

    fn device(&mut self, d: Option<Result<DeviceOrVariationIndex, ReadError>>) -> String {
        match d {
            None => "<device NULL>".into(),
            Some(Err(e)) => {
                self.unsupported.push(format!("an unreadable device table ({e})"));
                "<device NULL>".into()
            }
            Some(Ok(DeviceOrVariationIndex::VariationIndex(_))) => {
                self.unsupported.push("a variation index (a value that changes with the axes); the default value is written".into());
                "<device NULL>".into()
            }
            Some(Ok(DeviceOrVariationIndex::Device(dev))) => {
                let start = dev.start_size();
                let deltas: Vec<String> =
                    (start..=dev.end_size()).zip(dev.iter()).filter(|(_, d)| *d != 0).map(|(size, d)| format!("{size} {d}")).collect();
                if deltas.is_empty() { "<device NULL>".into() } else { format!("<device {}>", deltas.join(", ")) }
            }
        }
    }

    fn anchor(&mut self, a: Option<Result<AnchorTable, ReadError>>) -> String {
        match a {
            None => "<anchor NULL>".into(),
            Some(Err(e)) => {
                self.unsupported.push(format!("an unreadable anchor ({e})"));
                "<anchor NULL>".into()
            }
            Some(Ok(AnchorTable::Format1(f))) => format!("<anchor {} {}>", f.x_coordinate(), f.y_coordinate()),
            Some(Ok(AnchorTable::Format2(f))) => format!("<anchor {} {} contourpoint {}>", f.x_coordinate(), f.y_coordinate(), f.anchor_point()),
            Some(Ok(AnchorTable::Format3(f))) => {
                let (x, y) = (f.x_coordinate(), f.y_coordinate());
                let (dx, dy) = (f.x_device(), f.y_device());
                if dx.is_none() && dy.is_none() {
                    format!("<anchor {x} {y}>")
                } else {
                    let (dx, dy) = (self.device(dx), self.device(dy));
                    format!("<anchor {x} {y} {dx} {dy}>")
                }
            }
        }
    }

    /// Writes the `markClass` statements of one subtable and returns the class names by class.
    fn mark_classes(
        &mut self,
        out: &mut LookupOut,
        subtable: usize,
        cov: Result<CoverageTable, ReadError>,
        array: Result<MarkArray, ReadError>,
        what: &str,
    ) -> R<Vec<String>> {
        let marks = self.coverage(cov, what)?;
        let array = array.map_err(|e| read_err(what, e))?;
        let data = array.offset_data();
        // (class, anchor) → glyphs, in first-seen order.
        let mut groups: Vec<((u16, String), Vec<u16>)> = Vec::new();
        let mut max_class = 0u16;
        for (g, rec) in marks.into_iter().zip(array.mark_records().iter()) {
            self.budget.charge(1)?;
            let class = rec.mark_class();
            max_class = max_class.max(class);
            let anchor = self.anchor(Some(rec.mark_anchor(data)));
            match groups.iter_mut().find(|(k, _)| k.0 == class && k.1 == anchor) {
                Some((_, glyphs)) => glyphs.push(g),
                None => groups.push(((class, anchor), vec![g])),
            }
        }
        self.budget.charge(usize::from(max_class).saturating_add(1))?;
        let names: Vec<String> = (0..=max_class).map(|c| format!("@MC_{}_{subtable}_{c}", out.name)).collect();
        for n in &names {
            self.mark_class_names.insert(n.trim_start_matches('@').to_string());
        }
        for ((class, anchor), glyphs) in groups {
            let name = names.get(usize::from(class)).cloned().unwrap_or_default();
            let set = self.set_piece(glyphs, true);
            out.pre.push(Line::new(0, vec![Piece::Text("markClass ".into()), set, Piece::Text(format!(" {anchor} {name};"))]));
        }
        Ok(names)
    }

    /// `<anchor …> mark @class` for each class with an anchor.
    fn attach_parts<'b>(&mut self, classes: &[String], anchor_of: impl Fn(usize) -> Option<Result<AnchorTable<'b>, ReadError>>) -> R<Vec<String>> {
        let mut parts = Vec::new();
        for (c, name) in classes.iter().enumerate() {
            self.budget.charge(1)?;
            if let Some(a) = anchor_of(c) {
                let a = self.anchor(Some(a));
                parts.push(format!("{a} mark {name}"));
            }
        }
        Ok(parts)
    }

    /// One rule per distinct anchor list, with every glyph that has it.
    fn group_attach(&mut self, out: &mut LookupOut, keyword: &str, rows: Vec<(u16, Vec<String>)>) {
        let mut groups: Vec<(Vec<String>, Vec<u16>)> = Vec::new();
        let mut index: HashMap<Vec<String>, usize> = HashMap::new();
        for (g, parts) in rows {
            match index.get(&parts).and_then(|&i| groups.get_mut(i)) {
                Some((_, glyphs)) => glyphs.push(g),
                None => {
                    index.insert(parts.clone(), groups.len());
                    groups.push((parts, vec![g]));
                }
            }
        }
        for (parts, glyphs) in groups {
            let set = self.set_piece(glyphs, false);
            out.body.push(Line::new(1, vec![Piece::Text(keyword.into()), set, Piece::Text(format!(" {};", parts.join(" ")))]));
        }
    }

    // ── contextual rules ─────────────────────────────────────────────────────────────────────

    fn records(&mut self, recs: &[SequenceLookupRecord]) -> R<Vec<(u16, u16)>> {
        self.budget.charge(recs.len())?;
        Ok(recs.iter().map(|r| (r.sequence_index(), r.lookup_list_index())).collect())
    }

    fn sequence_rules(&mut self, st: &SequenceContext<'a>, what: &str) -> R<Vec<CtxRule>> {
        let mut rules = Vec::new();
        match st {
            SequenceContext::Format1(f) => {
                let cov = self.coverage(f.coverage(), what)?;
                for (g, set) in cov.into_iter().zip(f.seq_rule_sets().iter()) {
                    let Some(set) = set else { continue };
                    for rule in set.map_err(|e| read_err(what, e))?.seq_rules().iter() {
                        let rule = rule.map_err(|e| read_err(what, e))?;
                        let mut input = vec![vec![g]];
                        input.extend(rule.input_sequence().iter().map(|x| vec![x.get().to_u16()]));
                        self.budget.charge(input.len())?;
                        let records = self.records(rule.seq_lookup_records())?;
                        rules.push(CtxRule { back: Vec::new(), input, look: Vec::new(), records });
                    }
                }
            }
            SequenceContext::Format2(f) => {
                let cov = self.coverage(f.coverage(), what)?;
                let cd = self.classes(f.class_def(), what)?;
                for (c, set) in f.class_seq_rule_sets().iter().enumerate() {
                    let Some(set) = set else { continue };
                    let firsts: HashSet<u16> = cd.glyphs(u16::try_from(c).unwrap_or(u16::MAX)).into_iter().collect();
                    let first: Vec<u16> = cov.iter().copied().filter(|g| firsts.contains(g)).collect();
                    for rule in set.map_err(|e| read_err(what, e))?.class_seq_rules().iter() {
                        let rule = rule.map_err(|e| read_err(what, e))?;
                        self.budget.charge(first.len().max(1))?;
                        let mut input = vec![first.clone()];
                        for k in rule.input_sequence() {
                            self.budget.charge(self.count)?;
                            input.push(cd.glyphs(k.get()));
                        }
                        let records = self.records(rule.seq_lookup_records())?;
                        rules.push(CtxRule { back: Vec::new(), input, look: Vec::new(), records });
                    }
                }
            }
            SequenceContext::Format3(f) => {
                let mut input = Vec::new();
                for c in f.coverages().iter() {
                    input.push(self.coverage(c, what)?);
                }
                let records = self.records(f.seq_lookup_records())?;
                rules.push(CtxRule { back: Vec::new(), input, look: Vec::new(), records });
            }
        }
        Ok(rules)
    }

    fn chain_rules(&mut self, st: &ChainedSequenceContext<'a>, what: &str) -> R<Vec<CtxRule>> {
        let mut rules = Vec::new();
        let singles = |ids: &[BigEndian<GlyphId16>]| ids.iter().map(|x| vec![x.get().to_u16()]).collect::<Vec<_>>();
        match st {
            ChainedSequenceContext::Format1(f) => {
                let cov = self.coverage(f.coverage(), what)?;
                for (g, set) in cov.into_iter().zip(f.chained_seq_rule_sets().iter()) {
                    let Some(set) = set else { continue };
                    for rule in set.map_err(|e| read_err(what, e))?.chained_seq_rules().iter() {
                        let rule = rule.map_err(|e| read_err(what, e))?;
                        let mut back = singles(rule.backtrack_sequence());
                        back.reverse();
                        let mut input = vec![vec![g]];
                        input.extend(singles(rule.input_sequence()));
                        let look = singles(rule.lookahead_sequence());
                        self.budget.charge(back.len() + input.len() + look.len())?;
                        let records = self.records(rule.seq_lookup_records())?;
                        rules.push(CtxRule { back, input, look, records });
                    }
                }
            }
            ChainedSequenceContext::Format2(f) => {
                let cov = self.coverage(f.coverage(), what)?;
                let bcd = self.classes(f.backtrack_class_def(), what)?;
                let icd = self.classes(f.input_class_def(), what)?;
                let lcd = self.classes(f.lookahead_class_def(), what)?;
                for (c, set) in f.chained_class_seq_rule_sets().iter().enumerate() {
                    let Some(set) = set else { continue };
                    let firsts: HashSet<u16> = icd.glyphs(u16::try_from(c).unwrap_or(u16::MAX)).into_iter().collect();
                    let first: Vec<u16> = cov.iter().copied().filter(|g| firsts.contains(g)).collect();
                    for rule in set.map_err(|e| read_err(what, e))?.chained_class_seq_rules().iter() {
                        let rule = rule.map_err(|e| read_err(what, e))?;
                        let n = rule.backtrack_sequence().len() + rule.input_sequence().len() + rule.lookahead_sequence().len();
                        self.budget.charge(n.saturating_mul(self.count.max(1)).saturating_add(first.len().max(1)))?;
                        let mut back: Vec<Vec<u16>> = rule.backtrack_sequence().iter().map(|k| bcd.glyphs(k.get())).collect();
                        back.reverse();
                        let mut input = vec![first.clone()];
                        input.extend(rule.input_sequence().iter().map(|k| icd.glyphs(k.get())));
                        let look = rule.lookahead_sequence().iter().map(|k| lcd.glyphs(k.get())).collect();
                        let records = self.records(rule.seq_lookup_records())?;
                        rules.push(CtxRule { back, input, look, records });
                    }
                }
            }
            ChainedSequenceContext::Format3(f) => {
                let mut back = Vec::new();
                for c in f.backtrack_coverages().iter() {
                    back.push(self.coverage(c, what)?);
                }
                back.reverse();
                let mut input = Vec::new();
                for c in f.input_coverages().iter() {
                    input.push(self.coverage(c, what)?);
                }
                let mut look = Vec::new();
                for c in f.lookahead_coverages().iter() {
                    look.push(self.coverage(c, what)?);
                }
                let records = self.records(f.seq_lookup_records())?;
                rules.push(CtxRule { back, input, look, records });
            }
        }
        Ok(rules)
    }

    /// Writes contextual rules; a rule that can never match (an empty glyph set) is left out.
    fn ctx_lines(&mut self, kw: &str, rules: Vec<CtxRule>, out: &mut LookupOut) -> R<()> {
        let count = self.lookup_names.get(&out.table).map_or(0, Vec::len);
        for rule in rules {
            if rule.input.is_empty() || rule.back.iter().chain(&rule.input).chain(&rule.look).any(Vec::is_empty) {
                continue;
            }
            let mut calls: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
            for (pos, li) in &rule.records {
                if usize::from(*pos) >= rule.input.len() || usize::from(*li) >= count {
                    let mut line = self.note(&format!("a rule of lookup {} calls lookup {li} at position {pos}, which does not exist", out.name));
                    line.indent = 1;
                    out.body.push(line);
                    continue;
                }
                calls.entry(*pos).or_default().push(*li);
            }
            // A rule left with no call (its records named nothing that exists) does what `ignore` says.
            let ignore = calls.is_empty();
            let mut line = Line::new(1, vec![Piece::Text(if ignore { format!("ignore {kw}") } else { kw.to_string() })]);
            for set in rule.back {
                line.pieces.push(Piece::Text(" ".into()));
                let piece = self.set_piece(set, false);
                line.pieces.push(piece);
            }
            for (pos, set) in rule.input.into_iter().enumerate() {
                line.pieces.push(Piece::Text(" ".into()));
                let piece = self.set_piece(set, false);
                line.pieces.push(piece);
                line.pieces.push(Piece::Text("'".into()));
                if let Some(lis) = u16::try_from(pos).ok().and_then(|p| calls.get(&p)) {
                    for li in lis {
                        line.pieces.push(Piece::Text(format!(" lookup {}", self.lookup_name(out.table, *li))));
                        line.refs.push(*li);
                    }
                }
            }
            for set in rule.look {
                line.pieces.push(Piece::Text(" ".into()));
                let piece = self.set_piece(set, false);
                line.pieces.push(piece);
            }
            line.pieces.push(Piece::Text(";".into()));
            out.body.push(line);
        }
        Ok(())
    }

    // ── GDEF, BASE, feature parameters ───────────────────────────────────────────────────────

    fn gdef_block(&mut self, g: &Gdef<'a>) -> R<Vec<Line>> {
        let mut lines = vec![Line::text(0, "table GDEF {".into())];
        if let Some(cd) = g.glyph_class_def() {
            let classes = self.classes(cd, "GDEF glyph classes")?;
            let mut pieces = vec![Piece::Text("GlyphClassDef ".into())];
            for class in 1..=4u16 {
                if class > 1 {
                    pieces.push(Piece::Text(", ".into()));
                }
                let glyphs = classes.glyphs(class);
                if !glyphs.is_empty() {
                    pieces.push(self.set_piece(glyphs, true));
                }
            }
            pieces.push(Piece::Text(";".into()));
            lines.push(Line::new(1, pieces));
        }
        if let Some(list) = g.attach_list() {
            let list = list.map_err(|e| read_err("GDEF attachment list", e))?;
            let cov = self.coverage(list.coverage(), "GDEF attachment list")?;
            for (glyph, point) in cov.into_iter().zip(list.attach_points().iter()) {
                let point = point.map_err(|e| read_err("GDEF attachment list", e))?;
                let ids = point.point_indices();
                self.budget.charge(ids.len())?;
                if ids.is_empty() {
                    continue;
                }
                let nums: Vec<String> = ids.iter().map(|i| i.get().to_string()).collect();
                lines.push(Line::new(1, vec![Piece::Text("Attach ".into()), Piece::Glyph(glyph), Piece::Text(format!(" {};", nums.join(" ")))]));
            }
        }
        if let Some(list) = g.lig_caret_list() {
            let list = list.map_err(|e| read_err("GDEF ligature caret list", e))?;
            let cov = self.coverage(list.coverage(), "GDEF ligature caret list")?;
            for (glyph, lig) in cov.into_iter().zip(list.lig_glyphs().iter()) {
                let lig = lig.map_err(|e| read_err("GDEF ligature caret list", e))?;
                let mut by_pos = Vec::new();
                let mut by_index = Vec::new();
                for caret in lig.caret_values().iter() {
                    self.budget.charge(1)?;
                    match caret.map_err(|e| read_err("GDEF caret", e))? {
                        CaretValue::Format1(c) => by_pos.push(c.coordinate().to_string()),
                        CaretValue::Format2(c) => by_index.push(c.caret_value_point_index().to_string()),
                        CaretValue::Format3(c) => {
                            self.unsupported.push("a ligature caret's device table".into());
                            by_pos.push(c.coordinate().to_string());
                        }
                    }
                }
                if !by_pos.is_empty() {
                    lines.push(Line::new(
                        1,
                        vec![Piece::Text("LigatureCaretByPos ".into()), Piece::Glyph(glyph), Piece::Text(format!(" {};", by_pos.join(" ")))],
                    ));
                }
                if !by_index.is_empty() {
                    lines.push(Line::new(
                        1,
                        vec![Piece::Text("LigatureCaretByIndex ".into()), Piece::Glyph(glyph), Piece::Text(format!(" {};", by_index.join(" ")))],
                    ));
                }
            }
        }
        lines.push(Line::text(0, "} GDEF;".into()));
        Ok(lines)
    }

    fn base_block(&mut self, base: &Base<'a>) -> R<Vec<Line>> {
        let mut lines = vec![Line::text(0, "table BASE {".into())];
        for (prefix, axis) in [("HorizAxis", base.horiz_axis()), ("VertAxis", base.vert_axis())] {
            let Some(axis) = axis else { continue };
            let axis = axis.map_err(|e| read_err("BASE axis", e))?;
            let Some(Ok(tags)) = axis.base_tag_list() else {
                let mut line = self.note(&format!("the BASE {prefix} script list without a baseline tag list"));
                line.indent = 1;
                lines.push(line);
                continue;
            };
            let tags: Vec<T4> = tags.baseline_tags().iter().map(|t| t.get().into_bytes()).collect();
            self.budget.charge(tags.len())?;
            let tag_text: Vec<String> = tags.iter().map(|t| fea_tag(*t)).collect();
            lines.push(Line::text(1, format!("{prefix}.BaseTagList {};", tag_text.join(" "))));
            let scripts = axis.base_script_list().map_err(|e| read_err("BASE script list", e))?;
            let data = scripts.offset_data();
            let mut records = Vec::new();
            let mut minmax = Vec::new();
            for rec in scripts.base_script_records() {
                self.budget.charge(1)?;
                let script = rec.base_script_tag().into_bytes();
                let bs = rec.base_script(data).map_err(|e| read_err("BASE script", e))?;
                match bs.base_values() {
                    Some(Ok(values)) => {
                        let default = tags.get(usize::from(values.default_baseline_index())).copied().unwrap_or(*b"romn");
                        let mut coords = Vec::new();
                        for c in values.base_coords().iter() {
                            self.budget.charge(1)?;
                            coords.push(c.map(|c| c.coordinate()).unwrap_or(0).to_string());
                        }
                        coords.resize(tags.len(), "0".into());
                        records.push(format!("{} {} {}", fea_tag(script), fea_tag(default), coords.join(" ")));
                    }
                    _ => self.unsupported.push(format!("BASE script '{}' without baseline values", fea_tag(script))),
                }
                if let Some(Ok(mm)) = bs.default_min_max()
                    && let Some(line) = self.min_max(prefix, script, DFLT_LANG, &mm)
                {
                    minmax.push(line);
                }
                for lang in bs.base_lang_sys_records() {
                    self.budget.charge(1)?;
                    if let Ok(mm) = lang.min_max(bs.offset_data())
                        && let Some(line) = self.min_max(prefix, script, lang.base_lang_sys_tag().into_bytes(), &mm)
                    {
                        minmax.push(line);
                    }
                }
            }
            if !records.is_empty() {
                lines.push(Line::text(1, format!("{prefix}.BaseScriptList {};", records.join(", "))));
            }
            lines.extend(minmax);
        }
        lines.push(Line::text(0, "} BASE;".into()));
        Ok(lines)
    }

    fn min_max(&mut self, prefix: &str, script: T4, lang: T4, mm: &MinMax) -> Option<Line> {
        if mm.feat_min_max_count() > 0 {
            self.unsupported.push(format!("BASE feature extents of '{}'", fea_tag(script)));
        }
        match (mm.min_coord(), mm.max_coord()) {
            (Some(Ok(min)), Some(Ok(max))) => {
                Some(Line::text(1, format!("{prefix}.MinMax {} {} {}, {};", fea_tag(script), fea_tag(lang), min.coordinate(), max.coordinate())))
            }
            _ => None,
        }
    }

    fn feature_params(&mut self, tag: T4, p: &FeatureParams) -> Vec<Line> {
        match p {
            FeatureParams::Size(s) if &tag == b"size" => {
                let pt = |v: u16| format!("{:.1}", f64::from(v) / 10.0);
                let mut line = format!("parameters {} {}", pt(s.design_size()), s.identifier());
                if s.range_start() != 0 || s.range_end() != 0 {
                    line.push_str(&format!(" {} {}", pt(s.range_start()), pt(s.range_end())));
                }
                line.push(';');
                let mut lines = vec![Line::text(0, line)];
                if s.name_entry() != 0 {
                    lines.push(self.note("the size feature's menu name"));
                }
                lines
            }
            _ => vec![self.note(&format!("the parameters of feature '{}' (names it shows in menus)", fea_tag(tag)))],
        }
    }

    // ── rendering ────────────────────────────────────────────────────────────────────────────

    fn glyph_text(&mut self, g: u16) -> String {
        match self.names.get(usize::from(g)) {
            Some(n) => n.clone(),
            None => {
                self.unsupported.push(format!("glyph {g}, beyond the font's glyph count"));
                format!("glyph{g:05}")
            }
        }
    }

    /// Turns each chunk of lines into text. With `named`, glyph sets of two or more glyphs used
    /// more than once become named classes, listed in [`Rendered::classes`].
    pub fn render(mut self, chunks: &[Vec<Line>], named: bool) -> R<Rendered> {
        let mut uses = vec![0usize; self.sets.len()];
        for line in chunks.iter().flatten() {
            for piece in &line.pieces {
                if let Piece::Set(id, _) = piece
                    && let Some(u) = uses.get_mut(*id)
                {
                    *u += 1;
                }
            }
        }
        let mut taken = Taken::new(self.mark_class_names.clone());
        let mut class_names: HashMap<usize, String> = HashMap::new();
        if named {
            for (id, glyphs) in self.sets.iter().enumerate() {
                if glyphs.len() < 2 || uses.get(id).copied().unwrap_or(0) < 2 {
                    continue;
                }
                let name = class_name(glyphs, self.raw, &mut taken);
                class_names.insert(id, name);
            }
        }
        let mut size = 0usize;
        let mut used_glyphs: BTreeSet<u16> = BTreeSet::new();
        let push_line = |w: &mut Self, line: &Line, text: &mut String, used: &mut BTreeSet<u16>| {
            for _ in 0..line.indent.min(MAX_NESTING) {
                text.push_str("    ");
            }
            for piece in &line.pieces {
                match piece {
                    Piece::Text(s) => text.push_str(s),
                    Piece::Glyph(g) => {
                        used.insert(*g);
                        let n = w.glyph_text(*g);
                        text.push_str(&n);
                    }
                    Piece::Set(id, class) => {
                        if let Some(name) = class_names.get(id) {
                            text.push('@');
                            text.push_str(name);
                        } else {
                            let glyphs = w.sets.get(*id).cloned().unwrap_or_default();
                            used.extend(glyphs.iter().copied());
                            if glyphs.len() == 1 && !*class {
                                let n = glyphs.first().map(|g| w.glyph_text(*g)).unwrap_or_default();
                                text.push_str(&n);
                            } else {
                                text.push('[');
                                for (k, g) in glyphs.iter().enumerate() {
                                    if k > 0 {
                                        text.push(' ');
                                    }
                                    let n = w.glyph_text(*g);
                                    text.push_str(&n);
                                }
                                text.push(']');
                            }
                        }
                    }
                }
            }
            text.push('\n');
        };
        let too_large = || FeaError(format!("the feature code would be larger than {MAX_TEXT} bytes"));
        let mut sorted: Vec<(&usize, &String)> = class_names.iter().collect();
        sorted.sort_by_key(|(id, _)| **id);
        let mut classes = Vec::new();
        for (id, name) in sorted {
            let glyphs = self.sets.get(*id).cloned().unwrap_or_default();
            used_glyphs.extend(glyphs.iter().copied());
            let members: Vec<String> = glyphs.iter().map(|g| self.glyph_text(*g)).collect();
            let members = members.join(" ");
            size = size.saturating_add(members.len()).saturating_add(name.len());
            if size > MAX_TEXT {
                return Err(too_large());
            }
            classes.push((name.clone(), members));
        }
        let mut texts = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let mut text = String::new();
            for line in chunk {
                let before = text.len();
                push_line(&mut self, line, &mut text, &mut used_glyphs);
                size = size.saturating_add(text.len().saturating_sub(before));
                if size > MAX_TEXT {
                    return Err(too_large());
                }
            }
            texts.push(text);
        }
        let mut bad_names = Vec::new();
        let beyond: Vec<u16> = used_glyphs.iter().copied().filter(|g| usize::from(*g) >= self.count).collect();
        for g in used_glyphs {
            if let Some(raw) = self.raw.get(usize::from(g))
                && !fea_glyph_name(raw).1
            {
                bad_names.push(raw.clone());
            }
        }
        let mut unsupported = Vec::new();
        for u in self.unsupported {
            if !unsupported.contains(&u) {
                unsupported.push(u);
            }
        }
        Ok(Rendered { classes, texts, unsupported, bad_names, beyond })
    }
}

/// Depth-first: the lookups `li` calls come before it. A call back into a lookup being written
/// (a cycle), or nesting past the limit, leaves the order as it is; the rule is commented out
/// when it is written.
fn visit(li: u16, outs: &HashMap<u16, LookupOut>, state: &mut HashMap<u16, bool>, order: &mut Vec<u16>, depth: usize) {
    if state.contains_key(&li) {
        return;
    }
    state.insert(li, false);
    if depth < MAX_NESTING
        && let Some(out) = outs.get(&li)
    {
        for r in &out.refs {
            if *r != li {
                visit(*r, outs, state, order, depth + 1);
            }
        }
    }
    state.insert(li, true);
    if outs.contains_key(&li) {
        order.push(li);
    }
}

fn has_feature_variations(font: &FontRef, table: T4) -> bool {
    if table == GSUB {
        font.gsub().ok().and_then(|t| t.feature_variations()).is_some()
    } else {
        font.gpos().ok().and_then(|t| t.feature_variations()).is_some()
    }
}

fn lookup_count(font: &FontRef, table: T4) -> R<usize> {
    let n = if table == GSUB {
        font.gsub().and_then(|t| t.lookup_list()).map(|l| l.lookup_count())
    } else {
        font.gpos().and_then(|t| t.lookup_list()).map(|l| l.lookup_count())
    };
    n.map(usize::from).map_err(|e| read_err(&format!("{} lookup list", fea_tag(table)), e))
}

/// The script and feature lists of `GSUB` or `GPOS`; `None` when the font lacks the table.
fn layout_lists<'a>(font: &FontRef<'a>, table: T4) -> R<Option<(ScriptList<'a>, FeatureList<'a>)>> {
    let name = fea_tag(table);
    if font.table_data(Tag::new(&table)).is_none() {
        return Ok(None);
    }
    let lists = if table == GSUB {
        let t = font.gsub().map_err(|e| read_err(&name, e))?;
        (t.script_list(), t.feature_list())
    } else {
        let t = font.gpos().map_err(|e| read_err(&name, e))?;
        (t.script_list(), t.feature_list())
    };
    let scripts = lists.0.map_err(|e| read_err(&format!("{name} script list"), e))?;
    let features = lists.1.map_err(|e| read_err(&format!("{name} feature list"), e))?;
    Ok(Some((scripts, features)))
}

/// A language system: script, language, required feature, other feature indices.
type LanguageSystem = (T4, T4, Option<u16>, Vec<u16>);

/// Every language system, per script in stored order, the default one (as `dflt`) first.
fn language_systems(scripts: &ScriptList, budget: &mut Budget) -> R<Vec<LanguageSystem>> {
    let data = scripts.offset_data();
    let mut out = Vec::new();
    for record in scripts.script_records() {
        budget.charge(1)?;
        let script_tag = record.script_tag().into_bytes();
        let script = record.script(data).map_err(|e| read_err(&format!("script '{}'", fea_tag(script_tag)), e))?;
        let mut push = |language: T4, sys: LangSys, budget: &mut Budget| -> R<()> {
            let indices = sys.feature_indices();
            budget.charge(indices.len().saturating_add(1))?;
            let required = Some(sys.required_feature_index()).filter(|i| *i != 0xFFFF);
            out.push((script_tag, language, required, indices.iter().map(|i| i.get()).collect()));
            Ok(())
        };
        if let Some(sys) = script.default_lang_sys() {
            let sys = sys.map_err(|e| read_err(&format!("default language system of '{}'", fea_tag(script_tag)), e))?;
            push(DFLT_LANG, sys, budget)?;
        }
        let sdata = script.offset_data();
        for lang in script.lang_sys_records() {
            budget.charge(1)?;
            let language = lang.lang_sys_tag().into_bytes();
            let sys =
                lang.lang_sys(sdata).map_err(|e| read_err(&format!("language system '{}' of '{}'", fea_tag(language), fea_tag(script_tag)), e))?;
            push(language, sys, budget)?;
        }
    }
    Ok(out)
}

/// The tag and lookup indices of feature `index`.
fn feature_at(features: &FeatureList, index: u16, budget: &mut Budget) -> R<(T4, Vec<u16>)> {
    let record = features
        .feature_records()
        .get(usize::from(index))
        .ok_or_else(|| FeaError(format!("a language system names feature {index}, which the feature list does not have")))?;
    let tag = record.feature_tag().into_bytes();
    let feature = record.feature(features.offset_data()).map_err(|e| read_err(&format!("feature '{}'", fea_tag(tag)), e))?;
    let lookups = feature.lookup_list_indices();
    budget.charge(lookups.len().saturating_add(1))?;
    Ok((tag, lookups.iter().map(|i| i.get()).collect()))
}

fn gsub_kind(s: &SubstitutionSubtables) -> u16 {
    match s {
        SubstitutionSubtables::Single(_) => 1,
        SubstitutionSubtables::Multiple(_) => 2,
        SubstitutionSubtables::Alternate(_) => 3,
        SubstitutionSubtables::Ligature(_) => 4,
        SubstitutionSubtables::Contextual(_) => 5,
        SubstitutionSubtables::ChainContextual(_) => 6,
        SubstitutionSubtables::EmptyExtension => 7,
        SubstitutionSubtables::Reverse(_) => 8,
    }
}

fn gpos_kind(s: &PositionSubtables) -> u16 {
    match s {
        PositionSubtables::Single(_) => 1,
        PositionSubtables::Pair(_) => 2,
        PositionSubtables::Cursive(_) => 3,
        PositionSubtables::MarkToBase(_) => 4,
        PositionSubtables::MarkToLig(_) => 5,
        PositionSubtables::MarkToMark(_) => 6,
        PositionSubtables::Contextual(_) => 7,
        PositionSubtables::ChainContextual(_) => 8,
        PositionSubtables::EmptyExtension => 9,
    }
}

/// Whether GPOS lookup `li` is pair kerning the document's kerning gives back in full: pair
/// subtables whose values are the first glyph's x advance only (with variations in a variable
/// font, whose default the kerning keeps), a lookup flag of at most right-to-left and
/// ignore-marks, and class tables small enough for the kerning reader.
pub(crate) fn plain_pair(font: &FontRef, li: u16, variable: bool, max_class_records: usize) -> bool {
    let Ok(list) = font.gpos().and_then(|g| g.lookup_list()) else { return false };
    let Ok(lookup) = list.lookups().get(usize::from(li)) else { return false };
    if lookup.lookup_flag().to_bits() & !0x0009 != 0 {
        return false;
    }
    let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { return false };
    let mut allowed = ValueFormat::X_ADVANCE.bits();
    if variable {
        allowed |= ValueFormat::X_ADVANCE_DEVICE.bits();
    }
    subtables.iter().all(|st| match st {
        Ok(PairPos::Format1(t)) => t.value_format1().bits() & !allowed == 0 && t.value_format2().is_empty(),
        Ok(PairPos::Format2(t)) => {
            t.value_format1().bits() & !allowed == 0
                && t.value_format2().is_empty()
                && usize::from(t.class1_count()).saturating_mul(usize::from(t.class2_count())) <= max_class_records
        }
        Err(_) => false,
    })
}

/// Names in use, with the next number to try after each base name, so a run of names on one
/// base costs one step each instead of a scan from 2.
struct Taken {
    names: HashSet<String>,
    next: HashMap<String, usize>,
}

impl Taken {
    fn new(names: HashSet<String>) -> Self {
        Taken { names, next: HashMap::new() }
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// `base` when it is free, otherwise `base_N` with the smallest free N from 2 on; the name
    /// returned is taken from then on.
    fn claim(&mut self, base: String) -> String {
        if self.names.insert(base.clone()) {
            return base;
        }
        // Names are only ever added, so every number below the stored one is still taken.
        let mut n = self.next.get(&base).copied().unwrap_or(2);
        let mut candidate = format!("{base}_{n}");
        while !self.names.insert(candidate.clone()) {
            n += 1;
            candidate = format!("{base}_{n}");
        }
        self.next.insert(base, n + 1);
        candidate
    }
}

/// `first_last`, members added from the second one on while the name is taken, then a number.
fn class_name(glyphs: &[u16], raw: &[String], taken: &mut Taken) -> String {
    let part = |g: &u16| sanitize(raw.get(usize::from(*g)).map(String::as_str).unwrap_or("glyph"));
    let first = glyphs.first().map(part).unwrap_or_default();
    let last = glyphs.last().map(part).unwrap_or_default();
    let fix = |s: String| if s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') { s } else { format!("c_{s}") };
    let mut middle: Vec<String> = Vec::new();
    let mut candidate = fix(format!("{first}_{last}"));
    let mut k = 1;
    while (taken.contains(&candidate) || candidate.len() > 60) && k + 1 < glyphs.len() && k <= 3 {
        if let Some(g) = glyphs.get(k) {
            middle.push(part(g));
        }
        candidate = fix(format!("{first}_{}_{last}", middle.join("_")));
        k += 1;
    }
    if candidate.len() > 60 {
        candidate = "class".into();
    }
    taken.claim(candidate)
}

fn is_zero(vr: &ValueRecord) -> bool {
    let devices = [vr.x_placement_device(), vr.y_placement_device(), vr.x_advance_device(), vr.y_advance_device()];
    [vr.x_placement(), vr.y_placement(), vr.x_advance(), vr.y_advance()].iter().all(|v| v.unwrap_or(0) == 0) && devices.iter().all(Option::is_none)
}
