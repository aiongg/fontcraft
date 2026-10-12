//! Decompiling layout tables to feature code (M12.12; rows `features decompile`,
//! `layout.decompileFeatures`). The text compiled back by feaLib gives tables that normalise equal
//! to the original.
//!
//! Layout of the text (`docs/spec/20-cli.md` › features decompile):
//!
//! 1. `languagesystem` statements, `DFLT dflt` first.
//! 2. Named glyph classes: glyph sets used more than once, named from their first and last
//!    member (`@a_z`), with members in between added when two names would collide.
//! 3. `table GDEF` and `table BASE` blocks.
//! 4. Every lookup as a top-level `lookup` block, `GSUB` first, in lookup-list order, except that
//!    a lookup a contextual rule calls is written before the rule that calls it (the compiler
//!    resolves names as it reads). A lookup a feature uses is named
//!    `feature_script_language_index` after the first language system that uses it; the others
//!    `GSUB_index` or `GPOS_index`. Mark-attachment lookups are preceded by their `markClass`
//!    statements, one class name per lookup, subtable and class.
//! 5. One `feature` block per feature tag: for each script, `script`, the default language
//!    system's lookups, then each language with `exclude_dflt` unless it uses all of the default
//!    lookups.
//!
//! Rules are written so the compiler builds the same kind of subtable: glyph pairs as glyph
//! pairs, class pairs with bracketed classes, original subtable boundaries as `subtable;`.
//! Whatever has no feature syntax (variation indices, feature variations, most feature
//! parameters, lookups that call themselves) is written as a comment starting `# not written:`
//! and listed in [`Decompiled::unsupported`], so nothing is dropped silently.
//!
//! Input is untrusted: every record, glyph and rule read is charged to a work budget
//! ([`crate::features::MAX_LAYOUT_WORK`]), the text is capped at
//! [`crate::limits::MAX_FEATURE_SOURCE`] bytes, and lookup nesting is followed at most
//! [`crate::limits::MAX_LOOKUP_NESTING`] deep.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use read_fonts::tables::gdef::{CaretValue, Gdef};
use read_fonts::tables::gpos::{AnchorTable, PositionLookup, PositionSubtables, SinglePos, ValueRecord};
use read_fonts::tables::gsub::{SingleSubst, SubstitutionLookup, SubstitutionSubtables};
use read_fonts::tables::layout::{
    ChainedSequenceContext, ClassDef, CoverageTable, DeviceOrVariationIndex, FeatureParams, SequenceContext, SequenceLookupRecord,
};
use read_fonts::{ReadError, TableProvider};

use crate::features::{
    BASE, Budget, DFLT_LANG, GDEF, GPOS, GSUB, Provider, feature_at, glyph_count, glyph_names, language_systems, layout_lists, read_err,
};
use crate::limits::{MAX_FEATURE_SOURCE, MAX_LOOKUP_NESTING};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// What [`decompile`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecompileOptions {
    /// `GSUB`, `GPOS`, `GDEF`; empty decompiles every layout table the font has.
    pub tables: Vec<Tag>,
    /// Write glyph classes as named classes when a set of glyphs repeats.
    pub named_classes: bool,
    /// Only these features and the lookups they reach; empty writes every feature and lookup.
    pub features: Vec<Tag>,
    /// Write `useExtension` on lookups the font stores through Extension lookups.
    pub show_extensions: bool,
    /// Glyph names to write in place of the font's: font name → written name.
    pub rename: BTreeMap<String, String>,
}

/// The text and what it could not express.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decompiled {
    pub text: String,
    /// One sentence per construct without feature syntax; each is also a comment in `text`.
    pub unsupported: Vec<String>,
}

/// The font's layout as feature code.
pub fn decompile(font: &SfntFont, options: &DecompileOptions) -> Result<String> {
    decompile_report(font, options).map(|d| d.text)
}

/// The font's layout as feature code, with the list of constructs written as comments.
pub fn decompile_report(font: &SfntFont, options: &DecompileOptions) -> Result<Decompiled> {
    let all = [GSUB, GPOS, GDEF, BASE];
    for t in &options.tables {
        if !all.contains(t) {
            return Err(TablesError::BadOptions(format!("`{}` is not a layout table: give GSUB, GPOS, GDEF or BASE", tag_str(*t))));
        }
    }
    let wanted = |t: Tag| (options.tables.is_empty() || options.tables.contains(&t)) && font.get(t).is_some();
    let provider = Provider(font);
    let mut budget = Budget::new();
    let raw = glyph_names(&provider, &mut budget)?;
    let raw: Vec<String> = raw.into_iter().map(|n| options.rename.get(&n).cloned().unwrap_or(n)).collect();
    let mut w = Writer::new(&raw, budget, glyph_count(&provider));
    let gdef = match font.get(GDEF) {
        Some(_) => Some(provider.gdef().map_err(|e| read_err("GDEF", e))?),
        None => None,
    };
    w.gdef = gdef.clone();

    let mut head: Vec<Line> = Vec::new();
    let mut systems_seen: BTreeSet<(bool, Tag, bool, Tag)> = BTreeSet::new();
    // feature tag → (script, language) → (required, lookups by table)
    type SysFeatures = (bool, Vec<(Tag, u16)>);
    let mut feats: BTreeMap<Tag, BTreeMap<SysKey, SysFeatures>> = BTreeMap::new();
    let mut params: BTreeMap<Tag, Vec<Line>> = BTreeMap::new();
    let mut lookups: Vec<LookupOut> = Vec::new();

    for table in [GSUB, GPOS] {
        if !wanted(table) {
            continue;
        }
        let Some((scripts, features)) = layout_lists(&provider, table)? else { continue };
        if has_feature_variations(&provider, table) {
            let line = w.note(&format!("the {} feature variations (alternate lookups chosen by axis location)", tag_str(table)));
            head.push(line);
        }
        let systems = language_systems(&scripts, &mut w.budget)?;
        let count = lookup_count(&provider, table)?;
        let mut names: Vec<Option<String>> = vec![None; count];
        let mut roots: Vec<u16> = Vec::new();
        for sys in &systems {
            let indices: Vec<(u16, bool)> = sys.required.map(|r| (r, true)).into_iter().chain(sys.features.iter().map(|f| (*f, false))).collect();
            for (index, required) in indices {
                let (tag, ids) = feature_at(&features, index, &mut w.budget)?;
                if !options.features.is_empty() && !options.features.contains(&tag) {
                    continue;
                }
                if let Some(Ok(p)) = features
                    .feature_records()
                    .get(usize::from(index))
                    .and_then(|r| r.feature(features.offset_data()).ok())
                    .and_then(|f| f.feature_params())
                    && let std::collections::btree_map::Entry::Vacant(slot) = params.entry(tag)
                {
                    let lines = w.feature_params(tag, &p);
                    slot.insert(lines);
                }
                systems_seen.insert(sys_key(sys.script, sys.language));
                let entry = feats.entry(tag).or_default().entry(sys_key(sys.script, sys.language)).or_insert((false, Vec::new()));
                entry.0 |= required;
                for li in ids {
                    let Some(slot) = names.get_mut(usize::from(li)) else {
                        let line =
                            w.note(&format!("feature '{}' names {} lookup {li}, which the lookup list does not have", tag_str(tag), tag_str(table)));
                        head.push(line);
                        continue;
                    };
                    if slot.is_none() {
                        *slot = Some(format!("{}_{}_{}_{li}", sanitize_tag(tag), sanitize_tag(sys.script), sanitize_tag(sys.language)));
                    }
                    roots.push(li);
                    entry.1.push((table, li));
                }
            }
        }
        let names: Vec<String> = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| n.unwrap_or_else(|| format!("{}_{i}", tag_str(table).trim_end())))
            .map(|n| w.unique_lookup_name(n))
            .collect();
        w.lookup_names.insert(table, names);
        let candidates: Vec<u16> = if options.features.is_empty() { (0..count).filter_map(|i| u16::try_from(i).ok()).collect() } else { roots };
        let mut outs: HashMap<u16, LookupOut> = HashMap::new();
        let mut queue = candidates;
        while let Some(li) = queue.pop() {
            if outs.contains_key(&li) {
                continue;
            }
            let out = w.lookup(&provider, table, li)?;
            queue.extend(out.refs.iter().copied());
            outs.insert(li, out);
        }
        let mut order = Vec::new();
        let mut state: HashMap<u16, bool> = HashMap::new();
        let mut sorted: Vec<u16> = outs.keys().copied().collect();
        sorted.sort_unstable();
        for li in sorted {
            visit(li, &outs, &mut state, &mut order, 0);
        }
        for li in order {
            if let Some(out) = outs.remove(&li) {
                lookups.push(out);
            }
        }
    }

    // GDEF and BASE.
    let mut blocks: Vec<Line> = Vec::new();
    if wanted(GDEF)
        && let Some(g) = &gdef
    {
        blocks.extend(w.gdef_block(g)?);
    }
    if wanted(BASE) {
        let base = provider.base().map_err(|e| read_err("BASE", e))?;
        blocks.extend(w.base_block(&base)?);
    }

    // Lookups, commenting out calls to lookups not yet defined.
    let mut body: Vec<Line> = Vec::new();
    let mut defined: HashSet<(Tag, u16)> = HashSet::new();
    for out in lookups {
        body.extend(out.pre);
        let ext = if out.extension && options.show_extensions { " useExtension" } else { "" };
        body.push(Line::text(0, format!("lookup {}{ext} {{", out.name)));
        if let Some(flag) = out.flag {
            body.push(flag);
        }
        for mut line in out.body {
            if let Some(missing) = line.refs.iter().find(|r| **r == out.index || !defined.contains(&(out.table, **r))) {
                let name = w.lookup_name(out.table, *missing);
                let note = w.note(&format!("a rule of lookup {} calls lookup {name}, which cannot be defined before it", out.name));
                body.push(note);
                line.pieces.insert(0, Piece::Text("# ".into()));
                line.refs.clear();
            }
            body.push(line);
        }
        body.push(Line::text(0, format!("}} {};", out.name)));
        body.push(Line::text(0, String::new()));
        defined.insert((out.table, out.index));
    }

    // Feature blocks.
    for (tag, systems) in &feats {
        let name = tag_str(*tag);
        body.push(Line::text(0, format!("feature {} {{", name.trim_end())));
        if let Some(lines) = params.remove(tag) {
            body.extend(lines);
        }
        let special = tag == b"aalt" || tag == b"size";
        if special {
            if tag == b"aalt" {
                body.push(Line::text(1, "# the compiler rebuilds this feature from the lookups it names".into()));
            }
            let mut seen = HashSet::new();
            for (_, ids) in systems.values() {
                for id in ids {
                    if seen.insert(*id) {
                        body.push(Line::text(1, format!("lookup {};", w.lookup_name(id.0, id.1))));
                    }
                }
            }
        } else {
            type LangEntry<'a> = (Tag, bool, &'a Vec<(Tag, u16)>);
            let mut scripts: BTreeMap<(bool, Tag), Vec<LangEntry<'_>>> = BTreeMap::new();
            for ((nd, script, _, lang), (required, ids)) in systems {
                scripts.entry((*nd, *script)).or_default().push((*lang, *required, ids));
            }
            for ((_, script), langs) in scripts {
                body.push(Line::text(1, format!("script {};", tag_str(script))));
                let dflt: Vec<(Tag, u16)> = langs.iter().find(|(l, _, _)| *l == DFLT_LANG).map(|(_, _, ids)| (*ids).clone()).unwrap_or_default();
                for (lang, required, ids) in &langs {
                    let req = if *required { " required" } else { "" };
                    let list: Vec<(Tag, u16)> = if *lang == DFLT_LANG {
                        if *required {
                            body.push(Line::text(2, "language dflt required;".into()));
                        }
                        (*ids).clone()
                    } else if !dflt.is_empty() && dflt.iter().all(|d| ids.contains(d)) {
                        body.push(Line::text(2, format!("language {}{req};", tag_str(*lang))));
                        ids.iter().filter(|i| !dflt.contains(*i)).copied().collect()
                    } else {
                        let ex = if dflt.is_empty() { "" } else { " exclude_dflt" };
                        body.push(Line::text(2, format!("language {}{ex}{req};", tag_str(*lang))));
                        (*ids).clone()
                    };
                    for (t, li) in list {
                        body.push(Line::text(3, format!("lookup {};", w.lookup_name(t, li))));
                    }
                }
            }
        }
        body.push(Line::text(0, format!("}} {};", name.trim_end())));
        body.push(Line::text(0, String::new()));
    }
    // Features with parameters but no lookups (`size`).
    for (tag, lines) in params {
        let name = tag_str(tag);
        body.push(Line::text(0, format!("feature {} {{", name.trim_end())));
        body.extend(lines);
        body.push(Line::text(0, format!("}} {};", name.trim_end())));
        body.push(Line::text(0, String::new()));
    }

    let mut lines: Vec<Line> = vec![Line::text(0, "# Feature code decompiled from the font's layout tables.".into()), Line::text(0, String::new())];
    for (_, script, _, lang) in &systems_seen {
        lines.push(Line::text(0, format!("languagesystem {} {};", tag_str(*script), tag_str(*lang))));
    }
    if !systems_seen.is_empty() {
        lines.push(Line::text(0, String::new()));
    }
    lines.extend(head);
    let rest: Vec<Line> = blocks.into_iter().chain(body).collect();
    w.render(lines, rest, options.named_classes)
}

/// (not DFLT, script, not dflt, language): sorts `DFLT` and `dflt` first.
type SysKey = (bool, Tag, bool, Tag);

fn sys_key(script: Tag, language: Tag) -> SysKey {
    (&script != b"DFLT", script, language != DFLT_LANG, language)
}

/// Depth-first: the lookups `li` calls come before it. A call back into a lookup being written
/// (a cycle), or nesting past the limit, leaves the order as it is; the rule is commented out
/// when it is written.
fn visit(li: u16, outs: &HashMap<u16, LookupOut>, state: &mut HashMap<u16, bool>, order: &mut Vec<u16>, depth: usize) {
    if state.contains_key(&li) {
        return;
    }
    state.insert(li, false);
    if depth < MAX_LOOKUP_NESTING
        && let Some(out) = outs.get(&li)
    {
        for r in &out.refs {
            if *r != li {
                visit(*r, outs, state, order, depth + 1);
            }
        }
    }
    state.insert(li, true);
    order.push(li);
}

fn has_feature_variations(p: &Provider, table: Tag) -> bool {
    if table == GSUB {
        p.gsub().ok().and_then(|t| t.feature_variations()).is_some()
    } else {
        p.gpos().ok().and_then(|t| t.feature_variations()).is_some()
    }
}

fn lookup_count(p: &Provider, table: Tag) -> Result<usize> {
    let n = if table == GSUB {
        p.gsub().and_then(|t| t.lookup_list()).map(|l| l.lookup_count())
    } else {
        p.gpos().and_then(|t| t.lookup_list()).map(|l| l.lookup_count())
    };
    n.map(usize::from).map_err(|e| read_err(&format!("{} lookup list", tag_str(table)), e))
}

/// A tag as part of a name: trailing spaces dropped, other characters a name cannot hold as `_`.
fn sanitize_tag(tag: Tag) -> String {
    sanitize(tag_str(tag).trim_end())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect()
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

enum Piece {
    Text(String),
    Glyph(u16),
    /// A glyph set by id; `true` writes a single glyph in brackets (a class).
    Set(usize, bool),
}

struct Line {
    indent: usize,
    pieces: Vec<Piece>,
    /// Lookups (of the lookup's own table) this rule calls.
    refs: Vec<u16>,
}

impl Line {
    fn text(indent: usize, s: String) -> Self {
        Self { indent, pieces: vec![Piece::Text(s)], refs: Vec::new() }
    }

    fn new(indent: usize, pieces: Vec<Piece>) -> Self {
        Self { indent, pieces, refs: Vec::new() }
    }
}

struct LookupOut {
    table: Tag,
    index: u16,
    name: String,
    extension: bool,
    flag: Option<Line>,
    /// `markClass` statements written before the lookup.
    pre: Vec<Line>,
    body: Vec<Line>,
    refs: BTreeSet<u16>,
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

struct Writer<'a> {
    raw: &'a [String],
    names: Vec<String>,
    budget: Budget,
    count: usize,
    sets: Vec<Vec<u16>>,
    set_ids: HashMap<Vec<u16>, usize>,
    unsupported: Vec<String>,
    gdef: Option<Gdef<'a>>,
    lookup_names: HashMap<Tag, Vec<String>>,
    used_lookup_names: HashSet<String>,
    mark_class_names: HashSet<String>,
    bad_names_noted: bool,
}

impl<'a> Writer<'a> {
    fn new(raw: &'a [String], budget: Budget, count: usize) -> Self {
        Self {
            raw,
            names: raw.iter().map(|n| fea_glyph_name(n).0).collect(),
            budget,
            count,
            sets: Vec::new(),
            set_ids: HashMap::new(),
            unsupported: Vec::new(),
            gdef: None,
            lookup_names: HashMap::new(),
            used_lookup_names: HashSet::new(),
            mark_class_names: HashSet::new(),
            bad_names_noted: false,
        }
    }

    /// Records a construct without feature syntax and returns its comment line.
    fn note(&mut self, what: &str) -> Line {
        self.unsupported.push(what.to_string());
        Line::text(1, format!("# not written: {what}"))
    }

    fn unique_lookup_name(&mut self, name: String) -> String {
        let mut candidate = name.clone();
        let mut n = 2;
        while !self.used_lookup_names.insert(candidate.clone()) {
            candidate = format!("{name}_{n}");
            n += 1;
        }
        candidate
    }

    fn lookup_name(&self, table: Tag, li: u16) -> String {
        self.lookup_names.get(&table).and_then(|v| v.get(usize::from(li))).cloned().unwrap_or_else(|| format!("{}_{li}", tag_str(table).trim_end()))
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

    fn coverage(&mut self, cov: std::result::Result<CoverageTable, ReadError>, what: &str) -> Result<Vec<u16>> {
        let cov = cov.map_err(|e| read_err(what, e))?;
        let mut out = Vec::new();
        for g in cov.iter() {
            self.budget.charge(1)?;
            out.push(g.to_u16());
        }
        Ok(out)
    }

    fn classes(&mut self, cd: std::result::Result<ClassDef, ReadError>, what: &str) -> Result<Classes> {
        let cd = cd.map_err(|e| read_err(what, e))?;
        let mut seen: HashSet<u16> = HashSet::new();
        let mut by: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
        for (g, class) in cd.iter() {
            self.budget.charge(1)?;
            let g = g.to_u16();
            if class != 0 && seen.insert(g) {
                by.entry(class).or_default().push(g);
            }
        }
        self.budget.charge(self.count)?;
        let zero = (0..self.count).filter_map(|g| u16::try_from(g).ok()).filter(|g| !seen.contains(g)).collect();
        Ok(Classes { by, zero })
    }

    // ── lookups ──────────────────────────────────────────────────────────────────────────────

    fn lookup(&mut self, p: &Provider<'a>, table: Tag, li: u16) -> Result<LookupOut> {
        let name = self.lookup_name(table, li);
        let mut out = LookupOut { table, index: li, name, extension: false, flag: None, pre: Vec::new(), body: Vec::new(), refs: BTreeSet::new() };
        let what = format!("{} lookup {li}", tag_str(table));
        if table == GSUB {
            let list = p.gsub().and_then(|t| t.lookup_list()).map_err(|e| read_err("GSUB lookup list", e))?;
            let lookup: SubstitutionLookup = list.lookups().get(usize::from(li)).map_err(|e| read_err(&what, e))?;
            out.flag = self.flag_line(lookup.lookup_flag().to_bits(), lookup.mark_filtering_set());
            let subtables = lookup.subtables().map_err(|e| read_err(&what, e))?;
            out.extension = crate::features::gsub_kind(&subtables).1;
            self.gsub_body(&subtables, &mut out)?;
        } else {
            let list = p.gpos().and_then(|t| t.lookup_list()).map_err(|e| read_err("GPOS lookup list", e))?;
            let lookup: PositionLookup = list.lookups().get(usize::from(li)).map_err(|e| read_err(&what, e))?;
            out.flag = self.flag_line(lookup.lookup_flag().to_bits(), lookup.mark_filtering_set());
            let subtables = lookup.subtables().map_err(|e| read_err(&what, e))?;
            out.extension = crate::features::gpos_kind(&subtables).1;
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
                .and_then(|c| c.ok())
                .map(|cd| cd.iter().filter(|(_, c)| *c == attach).map(|(g, _)| g.to_u16()).collect::<Vec<u16>>());
            match glyphs.filter(|g| !g.is_empty() && self.budget.charge(g.len()).is_ok()) {
                Some(glyphs) => {
                    pieces.push(Piece::Text(" MarkAttachmentType ".into()));
                    let piece = self.set_piece(glyphs, true);
                    pieces.push(piece);
                }
                None => {
                    self.unsupported.push(format!("mark attachment type {attach}, which the GDEF table does not define"));
                }
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

    fn subtable_break(&mut self, out: &mut LookupOut, i: usize) {
        if i > 0 {
            out.body.push(Line::text(1, "subtable;".into()));
        }
    }

    fn gsub_body(&mut self, subtables: &SubstitutionSubtables<'a>, out: &mut LookupOut) -> Result<()> {
        let what = format!("{} subtable", out.name);
        match subtables {
            SubstitutionSubtables::Single(s) => {
                for (i, st) in s.iter().enumerate() {
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let cov = self.coverage(st.coverage(), &what)?;
                    for (g, set) in cov.into_iter().zip(st.ligature_sets().iter()) {
                        let set = set.map_err(|e| read_err(&what, e))?;
                        for lig in set.ligatures().iter() {
                            let lig = lig.map_err(|e| read_err(&what, e))?;
                            let comps = lig.component_glyph_ids();
                            self.budget.charge(comps.len().saturating_add(1))?;
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
                    self.subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.sequence_rules(&st, &what)?;
                    self.ctx_lines("sub", rules, out)?;
                }
            }
            SubstitutionSubtables::ChainContextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    self.subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.chain_rules(&st, &what)?;
                    self.ctx_lines("sub", rules, out)?;
                }
            }
            SubstitutionSubtables::Reverse(s) => {
                for (i, st) in s.iter().enumerate() {
                    self.subtable_break(out, i);
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
                let line = self.note(&format!("lookup {} is an extension lookup without subtables", out.name));
                out.body.push(line);
            }
        }
        Ok(())
    }

    fn gpos_body(&mut self, subtables: &PositionSubtables<'a>, out: &mut LookupOut) -> Result<()> {
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
                    self.subtable_break(out, i);
                    match st.map_err(|e| read_err(&what, e))? {
                        read_fonts::tables::gpos::PairPos::Format1(f) => {
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
                        read_fonts::tables::gpos::PairPos::Format2(f) => {
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
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
                    self.subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.sequence_rules(&st, &what)?;
                    self.ctx_lines("pos", rules, out)?;
                }
            }
            PositionSubtables::ChainContextual(s) => {
                for (i, st) in s.iter().enumerate() {
                    self.subtable_break(out, i);
                    let st = st.map_err(|e| read_err(&what, e))?;
                    let rules = self.chain_rules(&st, &what)?;
                    self.ctx_lines("pos", rules, out)?;
                }
            }
            PositionSubtables::EmptyExtension => {
                let line = self.note(&format!("lookup {} is an extension lookup without subtables", out.name));
                out.body.push(line);
            }
        }
        Ok(())
    }

    fn pair_line(&mut self, a: Piece, b: Piece, v1: &ValueRecord, v2: &ValueRecord) -> Line {
        let second_empty = v2.format().is_empty();
        let mut pieces = vec![Piece::Text("pos ".into()), a];
        if second_empty {
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

    fn device(&mut self, d: Option<std::result::Result<DeviceOrVariationIndex, ReadError>>) -> String {
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

    fn anchor(&mut self, a: Option<std::result::Result<AnchorTable, ReadError>>) -> String {
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
        cov: std::result::Result<CoverageTable, ReadError>,
        array: std::result::Result<read_fonts::tables::gpos::MarkArray, ReadError>,
        what: &str,
    ) -> Result<Vec<String>> {
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
    fn attach_parts<'b>(
        &mut self,
        classes: &[String],
        anchor_of: impl Fn(usize) -> Option<std::result::Result<AnchorTable<'b>, ReadError>>,
    ) -> Result<Vec<String>> {
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
        for (g, parts) in rows {
            match groups.iter_mut().find(|(p, _)| *p == parts) {
                Some((_, glyphs)) => glyphs.push(g),
                None => groups.push((parts, vec![g])),
            }
        }
        for (parts, glyphs) in groups {
            let set = self.set_piece(glyphs, false);
            out.body.push(Line::new(1, vec![Piece::Text(keyword.into()), set, Piece::Text(format!(" {};", parts.join(" ")))]));
        }
    }

    // ── contextual rules ─────────────────────────────────────────────────────────────────────

    fn records(&mut self, recs: &[SequenceLookupRecord]) -> Result<Vec<(u16, u16)>> {
        self.budget.charge(recs.len())?;
        Ok(recs.iter().map(|r| (r.sequence_index(), r.lookup_list_index())).collect())
    }

    fn sequence_rules(&mut self, st: &SequenceContext<'a>, what: &str) -> Result<Vec<CtxRule>> {
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

    fn chain_rules(&mut self, st: &ChainedSequenceContext<'a>, what: &str) -> Result<Vec<CtxRule>> {
        let mut rules = Vec::new();
        let singles =
            |ids: &[read_fonts::types::BigEndian<read_fonts::types::GlyphId16>]| ids.iter().map(|x| vec![x.get().to_u16()]).collect::<Vec<_>>();
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
                        self.budget.charge(n.saturating_mul(self.count.max(1)))?;
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
    fn ctx_lines(&mut self, kw: &str, rules: Vec<CtxRule>, out: &mut LookupOut) -> Result<()> {
        let count = self.lookup_names.get(&out.table).map_or(0, Vec::len);
        for rule in rules {
            if rule.input.is_empty() || rule.back.iter().chain(&rule.input).chain(&rule.look).any(Vec::is_empty) {
                continue;
            }
            let mut calls: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
            for (pos, li) in &rule.records {
                if usize::from(*pos) >= rule.input.len() || usize::from(*li) >= count {
                    let line = self.note(&format!("a rule of lookup {} calls lookup {li} at position {pos}, which does not exist", out.name));
                    out.body.push(line);
                    continue;
                }
                calls.entry(*pos).or_default().push(*li);
            }
            let ignore = rule.records.is_empty();
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

    fn gdef_block(&mut self, g: &Gdef<'a>) -> Result<Vec<Line>> {
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
        lines.push(Line::text(0, String::new()));
        Ok(lines)
    }

    fn base_block(&mut self, base: &read_fonts::tables::base::Base<'a>) -> Result<Vec<Line>> {
        let mut lines = vec![Line::text(0, "table BASE {".into())];
        for (prefix, axis) in [("HorizAxis", base.horiz_axis()), ("VertAxis", base.vert_axis())] {
            let Some(axis) = axis else { continue };
            let axis = axis.map_err(|e| read_err("BASE axis", e))?;
            let Some(Ok(tags)) = axis.base_tag_list() else {
                let line = self.note(&format!("the BASE {prefix} script list without a baseline tag list"));
                lines.push(line);
                continue;
            };
            let tags: Vec<Tag> = tags.baseline_tags().iter().map(|t| t.get().into_bytes()).collect();
            self.budget.charge(tags.len())?;
            let tag_text: Vec<String> = tags.iter().map(|t| tag_str(*t)).collect();
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
                        records.push(format!("{} {} {}", tag_str(script), tag_str(default), coords.join(" ")));
                    }
                    _ => self.unsupported.push(format!("BASE script '{}' without baseline values", tag_str(script))),
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
        lines.push(Line::text(0, String::new()));
        Ok(lines)
    }

    fn min_max(&mut self, prefix: &str, script: Tag, lang: Tag, mm: &read_fonts::tables::base::MinMax) -> Option<Line> {
        if mm.feat_min_max_count() > 0 {
            self.unsupported.push(format!("BASE feature extents of '{}'", tag_str(script)));
        }
        match (mm.min_coord(), mm.max_coord()) {
            (Some(Ok(min)), Some(Ok(max))) => {
                Some(Line::text(1, format!("{prefix}.MinMax {} {} {}, {};", tag_str(script), tag_str(lang), min.coordinate(), max.coordinate())))
            }
            _ => None,
        }
    }

    fn feature_params(&mut self, tag: Tag, p: &FeatureParams) -> Vec<Line> {
        match p {
            FeatureParams::Size(s) if &tag == b"size" => {
                let pt = |v: u16| format!("{:.1}", f64::from(v) / 10.0);
                let mut line = format!("parameters {} {}", pt(s.design_size()), s.identifier());
                if s.range_start() != 0 || s.range_end() != 0 {
                    line.push_str(&format!(" {} {}", pt(s.range_start()), pt(s.range_end())));
                }
                line.push(';');
                let mut lines = vec![Line::text(1, line)];
                if s.name_entry() != 0 {
                    lines.push(self.note("the size feature's menu name"));
                }
                lines
            }
            _ => vec![self.note(&format!("the parameters of feature '{}' (names it shows in menus)", tag_str(tag)))],
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

    fn render(mut self, head: Vec<Line>, rest: Vec<Line>, named: bool) -> Result<Decompiled> {
        // Count set uses, then name the sets of two or more glyphs used more than once.
        let mut uses = vec![0usize; self.sets.len()];
        for line in head.iter().chain(&rest) {
            for piece in &line.pieces {
                if let Piece::Set(id, _) = piece
                    && let Some(u) = uses.get_mut(*id)
                {
                    *u += 1;
                }
            }
        }
        let mut taken: HashSet<String> = self.mark_class_names.clone();
        let mut class_names: HashMap<usize, String> = HashMap::new();
        let mut defs: Vec<String> = Vec::new();
        if named {
            for (id, glyphs) in self.sets.iter().enumerate() {
                if glyphs.len() < 2 || uses.get(id).copied().unwrap_or(0) < 2 {
                    continue;
                }
                let name = class_name(glyphs, self.raw, &mut taken);
                class_names.insert(id, name);
            }
        }
        let mut text = String::new();
        let mut bad: BTreeSet<String> = BTreeSet::new();
        let mut used_glyphs: BTreeSet<u16> = BTreeSet::new();
        let push_line = |w: &mut Self, line: &Line, text: &mut String, used: &mut BTreeSet<u16>| -> Result<()> {
            for _ in 0..line.indent {
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
            if text.len() > MAX_FEATURE_SOURCE {
                return Err(TablesError::TooLarge { what: "the feature code", limit: MAX_FEATURE_SOURCE });
            }
            Ok(())
        };
        for line in &head {
            push_line(&mut self, line, &mut text, &mut used_glyphs)?;
        }
        let mut sorted: Vec<(&usize, &String)> = class_names.iter().collect();
        sorted.sort_by_key(|(id, _)| **id);
        for (id, name) in sorted {
            let glyphs = self.sets.get(*id).cloned().unwrap_or_default();
            used_glyphs.extend(glyphs.iter().copied());
            let members: Vec<String> = glyphs.iter().map(|g| self.glyph_text(*g)).collect();
            defs.push(format!("@{name} = [{}];", members.join(" ")));
        }
        for d in &defs {
            text.push_str(d);
            text.push('\n');
        }
        if !defs.is_empty() {
            text.push('\n');
        }
        for line in &rest {
            push_line(&mut self, line, &mut text, &mut used_glyphs)?;
        }
        for g in used_glyphs {
            if let Some(raw) = self.raw.get(usize::from(g))
                && !fea_glyph_name(raw).1
            {
                bad.insert(raw.clone());
            }
        }
        if !bad.is_empty() && !self.bad_names_noted {
            self.bad_names_noted = true;
            let list: Vec<String> = bad.into_iter().take(20).collect();
            self.unsupported.push(format!("glyph names feature syntax cannot hold: {}", list.join(", ")));
        }
        let mut unsupported = Vec::new();
        for u in self.unsupported {
            if !unsupported.contains(&u) {
                unsupported.push(u);
            }
        }
        Ok(Decompiled { text, unsupported })
    }
}

/// `@first_last`, members added from the second one on while the name is taken, then a number.
fn class_name(glyphs: &[u16], raw: &[String], taken: &mut HashSet<String>) -> String {
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
    let base = candidate.clone();
    let mut n = 2;
    while !taken.insert(candidate.clone()) {
        candidate = format!("{base}_{n}");
        n += 1;
    }
    candidate
}

fn is_zero(vr: &ValueRecord) -> bool {
    let devices = [vr.x_placement_device(), vr.y_placement_device(), vr.x_advance_device(), vr.y_advance_device()];
    [vr.x_placement(), vr.y_placement(), vr.x_advance(), vr.y_advance()].iter().all(|v| v.unwrap_or(0) == 0) && devices.iter().all(Option::is_none)
}

#[cfg(test)]
mod tests;
