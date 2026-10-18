//! FontDame (MTI) layout text to feature code (M12.13; rows `features convert`,
//! `layout.convertSyntax`). The reader is a port of fontTools `mtiLib` (MIT, Copyright (c)
//! 2015 Google Inc. and others; see `NOTICE`); mtiLib builds binary tables, so the
//! feature-code writer is ours. It writes, in order:
//!
//! 0. A `languagesystem` statement for every language system a feature is written for,
//!    `DFLT` first.
//! 1. From a `GDEF` file: the mark attachment classes and mark filtering sets as named glyph
//!    classes (`@GDEF_MarkAttachClass1`, `@GDEF_MarkFilterSet1`), which the lookup flags of the
//!    GSUB and GPOS files given with it name.
//! 2. `GSUB`, then `GPOS`: one lookup block per lookup (`GSUB_<name>`, `GPOS_<name>`), each
//!    after the lookups its contextual rules call, with `subtable;` between the file's
//!    subtables; then one feature block per feature tag, with a `script` and `language`
//!    statement for every language system that lists the feature.
//! 3. The `GDEF` table block: glyph classes, attachment points, ligature carets.
//!
//! What feature code cannot say is a warning: a feature no language system lists, a class 0
//! that needs the glyph order when none is given, lookups moved so that a called lookup is
//! defined first, mark attachment classes without their `GDEF` file.
//!
//! Choices of ours, where the format leaves room: backtrack sequences are listed nearest glyph
//! first, as the binary stores them, and are reversed for feature code; class 0 of a class
//! definition is every glyph of the glyph order without another class (at the first input
//! position, the glyphs listed with class 0); `U XXXX` names a glyph `uniXXXX` (`uXXXXX` above
//! U+FFFF), where mtiLib would take the Adobe Glyph List name when there is one.

mod parse;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use parse::{AnchorPt, Gdef, Kind, Layout, Lookup, Reader, Record, Rules, Subtable, Table, Value};

use crate::limits::{MAX_FEATURE_SOURCE, MAX_LAYOUT_ITEMS, MAX_LOOKUP_NESTING};
use crate::sfnt::Tag;
use crate::volt::fea_glyph;
use crate::{Result, TablesError};

/// Most FontDame files converted together (one per table, and a few spare).
pub const MAX_FILES: usize = 16;

/// The tables FontDame text can describe as layout.
pub const FONTDAME_TABLES: [Tag; 3] = [*b"GDEF", *b"GSUB", *b"GPOS"];

/// What [`convert`] reads and writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontDameOptions {
    /// The table of a file whose first line does not name one; `None` judges from the content
    /// and reads an undecided file as `GSUB`, with a warning.
    pub table: Option<Tag>,
    /// Tables to write, of `GDEF`, `GSUB` and `GPOS`; empty writes every table given.
    pub tables: Vec<Tag>,
    /// Glyph names by glyph id: what `# 12` names, and the glyphs of a class 0.
    pub glyph_order: Vec<String>,
}

/// Feature code for FontDame text of at most [`crate::limits::MAX_FEATURE_SOURCE`] bytes.
/// `glyph_order` names the glyphs the text refers to by index.
pub fn fontdame_to_fea(text: &str, glyph_order: &[String]) -> Result<(String, Vec<String>)> {
    convert(&[text], &FontDameOptions { glyph_order: glyph_order.to_vec(), ..FontDameOptions::default() })
}

/// Feature code for several FontDame files, one table each, written as one text so that the
/// lookups of `GSUB` and `GPOS` can name the classes of `GDEF`.
pub fn convert(texts: &[&str], options: &FontDameOptions) -> Result<(String, Vec<String>)> {
    if texts.len() > MAX_FILES {
        return Err(TablesError::TooLarge { what: "the FontDame files given together", limit: MAX_FILES });
    }
    for tag in options.tables.iter().chain(options.table.iter()) {
        if !FONTDAME_TABLES.contains(tag) {
            return Err(TablesError::BadOptions(format!(
                "FontDame layout text describes GDEF, GSUB and GPOS, not '{}'",
                crate::sfnt::tag_str(*tag).trim_end()
            )));
        }
    }
    let mut warnings = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    for text in texts {
        if text.len() > MAX_FEATURE_SOURCE {
            return Err(TablesError::TooLarge { what: "a FontDame file", limit: MAX_FEATURE_SOURCE });
        }
        let mut lines = parse::tokenize(text);
        let tag = match parse::header_tag(&lines) {
            Some(named) => {
                lines.remove(0);
                let tag: Tag = match named.as_str() {
                    "GSUB" => *b"GSUB",
                    "GPOS" => *b"GPOS",
                    "GDEF" => *b"GDEF",
                    other => return Err(TablesError::Unsupported(format!("FontDame '{other}' tables are not layout and have no feature-code form"))),
                };
                if options.table.is_some_and(|t| t != tag) {
                    return Err(TablesError::BadOptions(format!("the file describes {named}, not the table asked for")));
                }
                tag
            }
            None => match options.table.or_else(|| parse::sniff(&lines)) {
                Some(t) => t,
                None => {
                    warnings.push("a FontDame file does not say which table it describes; it is read as GSUB".to_string());
                    *b"GSUB"
                }
            },
        };
        let table = Reader { cur: parse::Cursor::new(lines), order: &options.glyph_order }.read(tag)?;
        let same = |t: &Table| match t {
            Table::Gdef(_) => tag == *b"GDEF",
            Table::Layout(other, _) => *other == tag,
        };
        if tables.iter().any(same) {
            return Err(TablesError::BadOptions(format!("two FontDame files describe {}", String::from_utf8_lossy(&tag))));
        }
        tables.push(table);
    }
    let wanted = |tag: &[u8; 4]| options.tables.is_empty() || options.tables.contains(tag);
    let mut w = Writer::new(&options.glyph_order);
    w.warnings = warnings;
    let gdef = tables.iter().find_map(|t| match t {
        Table::Gdef(g) => Some(g),
        Table::Layout(..) => None,
    });
    if let Some(g) = gdef {
        w.gdef_classes(g)?;
    }
    for tag in [*b"GSUB", *b"GPOS"] {
        if !wanted(&tag) {
            continue;
        }
        for t in &tables {
            if let Table::Layout(t_tag, layout) = t
                && *t_tag == tag
            {
                w.layout(&tag, layout)?;
            }
        }
    }
    if let Some(g) = gdef
        && wanted(b"GDEF")
    {
        w.gdef_block(g)?;
    }
    for (class, _) in w.mark_attach.iter().filter(|(c, _)| !w.used_attach.contains(c)) {
        w.warnings.push(format!(
            "mark attachment class {class} is used by no lookup; feature code keeps such a class only through the lookups that use it"
        ));
    }
    for (set, _) in w.filter_sets.iter().filter(|(s, _)| !w.used_filter.contains(s)) {
        w.warnings.push(format!("mark filtering set {set} is used by no lookup; feature code keeps such a set only through the lookups that use it"));
    }
    // Language systems are declared before everything else, DFLT first, as feature code wants.
    let mut systems = std::mem::take(&mut w.systems);
    systems.sort_by_key(|(script, _)| u8::from(script != "DFLT"));
    let mut fea: String = systems.iter().map(|(s, l)| format!("languagesystem {s} {l};\n")).collect();
    if !fea.is_empty() {
        fea.push('\n');
    }
    fea.push_str(&w.out);
    Ok((fea, w.warnings))
}

/// Whether the text reads as FontDame layout text: its first line names a FontDame table or
/// starts one of the format's blocks.
pub fn looks_like_fontdame(text: &str) -> bool {
    let lines = parse::tokenize(text.get(..text.len().min(1 << 16)).unwrap_or(text));
    if parse::header_tag(&lines).is_some() {
        return true;
    }
    let Some(first) = lines.first() else { return false };
    let word = first.first().to_lowercase();
    matches!(
        word.as_str(),
        "lookup"
            | "script table begin"
            | "feature table begin"
            | "class definition begin"
            | "attachment list begin"
            | "carets begin"
            | "mark attachment class definition begin"
            | "markfilter set definition begin"
    ) || (word.starts_with("comment") && lines.get(1).is_some_and(|l| l.first().eq_ignore_ascii_case("lookup")))
}

// ── the writer ─────────────────────────────────────────────────────────────────────────────────

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).collect()
}

fn value(v: &Value) -> String {
    let [a, b, c, d] = v.map(|x| x.unwrap_or(0));
    format!("<{a} {b} {c} {d}>")
}

fn anchor(a: &AnchorPt) -> String {
    match a.point {
        Some(p) => format!("<anchor {} {} contourpoint {p}>", a.x, a.y),
        None => format!("<anchor {} {}>", a.x, a.y),
    }
}

fn anchor_or_null(a: Option<&AnchorPt>) -> String {
    a.map_or_else(|| "<anchor NULL>".to_string(), anchor)
}

/// One glyph, or several in brackets.
fn glyph_set(names: &[String]) -> String {
    match names {
        [one] => fea_glyph(one),
        _ => format!("[{}]", names.iter().map(|n| fea_glyph(n)).collect::<Vec<_>>().join(" ")),
    }
}

struct Writer<'a> {
    order: &'a [String],
    out: String,
    warnings: Vec<String>,
    mark_attach: BTreeMap<u16, String>,
    filter_sets: BTreeMap<u32, String>,
    used_attach: BTreeSet<u16>,
    used_filter: BTreeSet<u32>,
    names: HashSet<String>,
    /// The language systems the features name, in the order first written.
    systems: Vec<(String, String)>,
    work: usize,
}

/// What a lookup's body needs from the rest of its table.
struct LookupContext<'l> {
    /// Feature-code names of the table's lookups, by index.
    names: &'l [String],
    index: &'l HashMap<String, usize>,
    empty: &'l HashSet<usize>,
    keyword: &'static str,
}

impl<'a> Writer<'a> {
    fn new(order: &'a [String]) -> Self {
        Self {
            order,
            out: String::new(),
            warnings: Vec::new(),
            mark_attach: BTreeMap::new(),
            filter_sets: BTreeMap::new(),
            used_attach: BTreeSet::new(),
            used_filter: BTreeSet::new(),
            names: HashSet::new(),
            systems: Vec::new(),
            work: 0,
        }
    }

    fn spend(&mut self, n: usize) -> Result<()> {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_LAYOUT_ITEMS {
            return Err(TablesError::TooLarge { what: "the feature code the FontDame files expand to", limit: MAX_LAYOUT_ITEMS });
        }
        Ok(())
    }

    fn unique(&mut self, base: &str) -> String {
        let mut name = sanitize(base);
        while self.names.contains(&name) {
            name.push('_');
        }
        self.names.insert(name.clone());
        name
    }

    /// `@name = [glyphs];` written into `defs`, and the name.
    fn define(&mut self, defs: &mut Vec<String>, base: &str, glyphs: &[String]) -> Result<String> {
        self.spend(glyphs.len())?;
        let name = self.unique(base);
        defs.push(format!("@{name} = [{}];", glyphs.iter().map(|g| fea_glyph(g)).collect::<Vec<_>>().join(" ")));
        Ok(name)
    }

    /// Every glyph of the glyph order not in `assigned`, or `None` without a glyph order.
    fn complement(&mut self, assigned: &HashSet<&str>) -> Result<Option<Vec<String>>> {
        if self.order.is_empty() {
            return Ok(None);
        }
        let out: Vec<String> = self.order.iter().filter(|g| !assigned.contains(g.as_str())).cloned().collect();
        self.spend(out.len())?;
        Ok(Some(out))
    }

    fn gdef_classes(&mut self, g: &Gdef) -> Result<()> {
        let mut defs = Vec::new();
        if let Some(classes) = &g.mark_attach {
            let mut by_class: BTreeMap<u16, Vec<String>> = BTreeMap::new();
            for (glyph, class) in classes {
                by_class.entry(*class).or_default().push(glyph.clone());
            }
            for (class, glyphs) in by_class {
                if class == 0 {
                    continue;
                }
                let name = self.define(&mut defs, &format!("GDEF_MarkAttachClass{class}"), &glyphs)?;
                self.mark_attach.insert(class, name);
            }
        }
        for (set, glyphs) in g.filter_sets.iter().flatten() {
            let name = self.define(&mut defs, &format!("GDEF_MarkFilterSet{set}"), glyphs)?;
            self.filter_sets.insert(*set, name);
        }
        if !defs.is_empty() {
            self.out.push_str("# GDEF classes for lookup flags\n");
            for d in defs {
                self.out.push_str(&d);
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        Ok(())
    }

    fn gdef_block(&mut self, g: &Gdef) -> Result<()> {
        let mut body = Vec::new();
        if let Some(classes) = &g.glyph_classes {
            let mut slots: [Vec<String>; 4] = Default::default();
            for (glyph, class) in classes {
                match usize::from(*class).checked_sub(1).and_then(|i| slots.get_mut(i)) {
                    Some(slot) => slot.push(glyph.clone()),
                    None => {
                        self.warnings.push(format!("GDEF glyph class {class} of {glyph} is not base, ligature, mark or component; it is left out"))
                    }
                }
            }
            self.spend(classes.len())?;
            let [base, ligature, mark, component] = slots
                .map(|s| if s.is_empty() { String::new() } else { format!("[{}]", s.iter().map(|n| fea_glyph(n)).collect::<Vec<_>>().join(" ")) });
            body.push(format!("GlyphClassDef {base}, {ligature}, {mark}, {component};"));
        }
        for (glyph, points) in g.attach.iter().flatten() {
            if points.is_empty() {
                self.warnings.push(format!("{glyph} has no attachment points; it is left out"));
                continue;
            }
            body.push(format!("Attach {} {};", fea_glyph(glyph), points.iter().map(i64::to_string).collect::<Vec<_>>().join(" ")));
        }
        for (glyph, carets) in g.carets.iter().flatten() {
            if carets.is_empty() {
                continue;
            }
            body.push(format!("LigatureCaretByPos {} {};", fea_glyph(glyph), carets.iter().map(i64::to_string).collect::<Vec<_>>().join(" ")));
        }
        if body.is_empty() {
            return Ok(());
        }
        self.out.push_str("table GDEF {\n");
        for line in body {
            self.out.push_str(&format!("    {line}\n"));
        }
        self.out.push_str("} GDEF;\n");
        Ok(())
    }

    fn layout(&mut self, tag: &[u8; 4], layout: &Layout) -> Result<()> {
        let prefix = String::from_utf8_lossy(tag).into_owned();
        let lookups = &layout.lookups;
        let names: Vec<String> = lookups.iter().map(|l| self.unique(&format!("{prefix}_{}", l.name))).collect();
        let index: HashMap<String, usize> = lookups.iter().enumerate().map(|(i, l)| (l.name.clone(), i)).collect();
        let empty: HashSet<usize> = lookups.iter().enumerate().filter(|(_, l)| l.subtables.is_empty()).map(|(i, _)| i).collect();
        for i in &empty {
            if let Some(l) = lookups.get(*i) {
                self.warnings.push(format!("FontDame line {}: {prefix} lookup \"{}\" has no rules and is left out", l.line, l.name));
            }
        }
        // The lookups each lookup calls.
        let mut calls: Vec<Vec<usize>> = Vec::new();
        for l in lookups {
            let mut list = Vec::new();
            for (_, name) in lookup_records(l) {
                let callee = index.get(name).copied().ok_or_else(|| {
                    TablesError::Invalid(format!("{prefix} lookup \"{}\" calls lookup \"{name}\", which the file does not define", l.name))
                })?;
                if !list.contains(&callee) {
                    list.push(callee);
                }
            }
            calls.push(list);
        }
        let order = definition_order(lookups, &calls, &prefix)?;
        if order.iter().enumerate().any(|(i, l)| i != *l) {
            let moved: Vec<&str> =
                order.iter().enumerate().filter(|(i, l)| i != *l).filter_map(|(_, l)| lookups.get(*l)).map(|l| l.name.as_str()).collect();
            self.warnings.push(format!(
                "{prefix}: feature code defines a lookup before the rules that call it, so these lookups change places in the lookup list: {}",
                moved.join(", ")
            ));
        }
        let keyword = if tag == b"GSUB" { "sub" } else { "pos" };
        let ctx = LookupContext { names: &names, index: &index, empty: &empty, keyword };
        if !lookups.is_empty() {
            self.out.push_str(&format!("# {prefix} lookups\n"));
        }
        for i in order {
            if empty.contains(&i) {
                continue;
            }
            if let (Some(l), Some(name)) = (lookups.get(i), names.get(i)) {
                self.lookup(l, name, &ctx)?;
            }
        }
        self.features(&prefix, layout, &names, &index, &empty)
    }

    fn lookup(&mut self, l: &Lookup, name: &str, ctx: &LookupContext) -> Result<()> {
        let mut defs: Vec<String> = Vec::new();
        let mut body: Vec<String> = Vec::new();
        let mut words: Vec<String> = ["RightToLeft", "IgnoreBaseGlyphs", "IgnoreLigatures", "IgnoreMarks"]
            .iter()
            .enumerate()
            .filter(|(i, _)| l.flags & (1 << i) != 0)
            .map(|(_, w)| (*w).to_string())
            .collect();
        if l.mark_attach != 0 {
            match self.mark_attach.get(&l.mark_attach) {
                Some(class) => {
                    words.push(format!("MarkAttachmentType @{class}"));
                    self.used_attach.insert(l.mark_attach);
                }
                None => self.warnings.push(format!(
                    "lookup \"{}\": mark attachment class {} is defined in GDEF, which was not given with it; the lookup is written without it",
                    l.name, l.mark_attach
                )),
            }
        }
        if let Some(set) = l.filter_set {
            match self.filter_sets.get(&set) {
                Some(class) => {
                    words.push(format!("UseMarkFilteringSet @{class}"));
                    self.used_filter.insert(set);
                }
                None => self.warnings.push(format!(
                    "lookup \"{}\": mark filtering set {set} is defined in GDEF, which was not given with it; the lookup is written without it",
                    l.name
                )),
            }
        }
        if !words.is_empty() {
            body.push(format!("lookupflag {};", words.join(" ")));
        }
        for (si, st) in l.subtables.iter().enumerate() {
            if si > 0 {
                body.push("subtable;".to_string());
            }
            let base = format!("{name}_{}", si + 1);
            self.subtable(l, st, &base, ctx, &mut defs, &mut body)?;
        }
        for d in defs {
            self.out.push_str(&d);
            self.out.push('\n');
        }
        self.out.push_str(&format!("lookup {name} {{\n"));
        for line in body {
            self.out.push_str(&format!("    {line}\n"));
        }
        self.out.push_str(&format!("}} {name};\n\n"));
        Ok(())
    }

    fn subtable(&mut self, l: &Lookup, st: &Subtable, base: &str, ctx: &LookupContext, defs: &mut Vec<String>, body: &mut Vec<String>) -> Result<()> {
        match st {
            Subtable::SingleSub(m) => body.extend(m.iter().map(|(a, b)| format!("sub {} by {};", fea_glyph(a), fea_glyph(b)))),
            Subtable::MultipleSub(m) => {
                for (a, seq) in m {
                    let by = if seq.is_empty() { "NULL".to_string() } else { seq.iter().map(|g| fea_glyph(g)).collect::<Vec<_>>().join(" ") };
                    body.push(format!("sub {} by {by};", fea_glyph(a)));
                }
            }
            Subtable::AlternateSub(m) => {
                for (a, alts) in m {
                    if alts.is_empty() {
                        self.warnings.push(format!("lookup \"{}\": {a} has no alternates; it is left out", l.name));
                        continue;
                    }
                    body.push(format!("sub {} from [{}];", fea_glyph(a), alts.iter().map(|g| fea_glyph(g)).collect::<Vec<_>>().join(" ")));
                }
            }
            Subtable::Ligature(m) => {
                for (lig, comps) in m {
                    body.push(format!("sub {} by {};", comps.iter().map(|g| fea_glyph(g)).collect::<Vec<_>>().join(" "), fea_glyph(lig)));
                }
            }
            Subtable::SinglePos(values) => body.extend(values.iter().map(|(g, v)| format!("pos {} {};", fea_glyph(g), value(v)))),
            Subtable::PairGlyph(pairs) => {
                let right = pairs.iter().any(|p| p.3 != [None; 4]);
                for (a, b, v1, v2) in pairs {
                    body.push(if right {
                        format!("pos {} {} {} {};", fea_glyph(a), value(v1), fea_glyph(b), value(v2))
                    } else {
                        format!("pos {} {} {};", fea_glyph(a), fea_glyph(b), value(v1))
                    });
                }
            }
            Subtable::PairClass { first, second, records, line } => {
                let right = records.iter().any(|r| r.3 != [None; 4]);
                let mut first_names: HashMap<u16, Option<String>> = HashMap::new();
                let mut second_names: HashMap<u16, Option<String>> = HashMap::new();
                let mut skipped_zero = false;
                for (c1, c2, v1, v2) in records {
                    if !first_names.contains_key(c1) {
                        let glyphs: Vec<String> = first.iter().filter(|(_, c)| c == c1).map(|(g, _)| g.clone()).collect();
                        let name = if glyphs.is_empty() { None } else { Some(self.define(defs, &format!("{base}_first{c1}"), &glyphs)?) };
                        first_names.insert(*c1, name);
                    }
                    if !second_names.contains_key(c2) {
                        let glyphs = if *c2 == 0 {
                            let assigned: HashSet<&str> = second.iter().filter(|(_, c)| *c != 0).map(|(g, _)| g.as_str()).collect();
                            self.complement(&assigned)?
                        } else {
                            Some(second.iter().filter(|(_, c)| c == c2).map(|(g, _)| g.clone()).collect())
                        };
                        let name = match glyphs {
                            Some(g) if !g.is_empty() => Some(self.define(defs, &format!("{base}_second{c2}"), &g)?),
                            Some(_) => None,
                            None => {
                                skipped_zero = true;
                                None
                            }
                        };
                        second_names.insert(*c2, name);
                    }
                    let (Some(Some(a)), Some(Some(b))) = (first_names.get(c1), second_names.get(c2)) else { continue };
                    body.push(if right { format!("pos @{a} {} @{b} {};", value(v1), value(v2)) } else { format!("pos @{a} @{b} {};", value(v1)) });
                }
                if skipped_zero {
                    self.warnings.push(format!(
                        "FontDame line {line}: values for second class 0 (every glyph without a class) need the glyph order; they are left out"
                    ));
                }
            }
            Subtable::Cursive(records) => {
                for (g, entry, exit) in records {
                    body.push(format!("pos cursive {} {} {};", fea_glyph(g), anchor_or_null(entry.as_ref()), anchor_or_null(exit.as_ref())));
                }
            }
            Subtable::Mark { marks, bases, line } => self.marks(l, base, marks, bases, *line, defs, body)?,
            Subtable::Context(rules) => self.context(l, base, rules, ctx, defs, body)?,
            Subtable::Reverse { backtrack, lookahead, mapping, line } => {
                if mapping.is_empty() {
                    return Ok(());
                }
                let prefix: Vec<String> = backtrack.iter().rev().map(|c| glyph_set(c)).collect();
                let suffix: Vec<String> = lookahead.iter().map(|c| glyph_set(c)).collect();
                if backtrack.iter().chain(lookahead.iter()).any(Vec::is_empty) {
                    self.warnings.push(format!("FontDame line {line}: an empty coverage matches nothing; the rules are left out"));
                    return Ok(());
                }
                let src: Vec<String> = mapping.iter().map(|(a, _)| a.clone()).collect();
                let dst: Vec<String> = mapping.iter().map(|(_, b)| b.clone()).collect();
                let mut s = "rsub ".to_string();
                if prefix.is_empty() && suffix.is_empty() {
                    s.push_str(&glyph_set(&src));
                } else {
                    for p in &prefix {
                        s.push_str(p);
                        s.push(' ');
                    }
                    s.push_str(&glyph_set(&src));
                    s.push('\'');
                    for p in &suffix {
                        s.push(' ');
                        s.push_str(p);
                    }
                }
                s.push_str(&format!(" by {};", glyph_set(&dst)));
                body.push(s);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn marks(
        &mut self,
        l: &Lookup,
        base: &str,
        marks: &[(String, u16, AnchorPt)],
        bases: &[(String, u16, Option<(u16, u16)>, AnchorPt)],
        line: usize,
        defs: &mut Vec<String>,
        body: &mut Vec<String>,
    ) -> Result<()> {
        let mut classes: BTreeMap<u16, String> = BTreeMap::new();
        for (g, c, a) in marks {
            let name = match classes.get(c) {
                Some(n) => n.clone(),
                None => {
                    let n = self.unique(&format!("{base}_mark{c}"));
                    classes.insert(*c, n.clone());
                    n
                }
            };
            defs.push(format!("markClass {} {} @{name};", fea_glyph(g), anchor(a)));
        }
        let mut glyphs: Vec<&str> = Vec::new();
        for (g, ..) in bases {
            if !glyphs.contains(&g.as_str()) {
                glyphs.push(g);
            }
        }
        self.spend(bases.len().saturating_add(marks.len()))?;
        let mut unknown = BTreeSet::new();
        let mark_text = |entries: &mut Vec<(u16, &AnchorPt)>, unknown: &mut BTreeSet<u16>| -> String {
            entries.sort_by_key(|(c, _)| *c);
            entries
                .iter()
                .filter_map(|(c, a)| match classes.get(c) {
                    Some(n) => Some(format!("{} mark @{n}", anchor(a))),
                    None => {
                        unknown.insert(*c);
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        for g in glyphs {
            let mine: Vec<&(String, u16, Option<(u16, u16)>, AnchorPt)> = bases.iter().filter(|b| b.0 == g).collect();
            if l.kind == Kind::MarkLigature {
                let count = mine.first().and_then(|b| b.2).map_or(0, |(_, n)| n);
                if mine.iter().any(|b| b.2.map(|(_, n)| n) != Some(count)) {
                    return Err(TablesError::Invalid(format!("FontDame line {line}: ligature {g} is given different component counts")));
                }
                let mut parts = Vec::new();
                for comp in 1..=count {
                    let mut entries: Vec<(u16, &AnchorPt)> = mine.iter().filter(|b| b.2.map(|(i, _)| i) == Some(comp)).map(|b| (b.1, &b.3)).collect();
                    let text = mark_text(&mut entries, &mut unknown);
                    parts.push(if text.is_empty() { "<anchor NULL>".to_string() } else { text });
                }
                if mine.iter().any(|b| b.2.is_some_and(|(i, n)| i == 0 || i > n)) {
                    return Err(TablesError::Invalid(format!("FontDame line {line}: ligature {g} names a component outside its count")));
                }
                body.push(format!("pos ligature {} {};", fea_glyph(g), parts.join(" ligComponent ")));
            } else {
                let mut entries: Vec<(u16, &AnchorPt)> = mine.iter().map(|b| (b.1, &b.3)).collect();
                let text = mark_text(&mut entries, &mut unknown);
                if text.is_empty() {
                    continue;
                }
                let kw = if l.kind == Kind::MarkMark { "mark" } else { "base" };
                body.push(format!("pos {kw} {} {text};", fea_glyph(g)));
            }
        }
        if !unknown.is_empty() {
            let list: Vec<String> = unknown.iter().map(u16::to_string).collect();
            self.warnings.push(format!("FontDame line {line}: anchors for mark classes {} that no mark has are left out", list.join(", ")));
        }
        Ok(())
    }

    /// One contextual rule: marked input, each position followed by the lookups called there.
    #[allow(clippy::too_many_arguments)]
    fn rule(
        &mut self,
        l: &Lookup,
        ctx: &LookupContext,
        prefix: &[String],
        input: &[String],
        suffix: &[String],
        records: &[Record],
        no: usize,
    ) -> Result<String> {
        if input.is_empty() {
            return Err(TablesError::Invalid(format!("FontDame line {no}: a contextual rule without input")));
        }
        let mut calls: Vec<Vec<String>> = vec![Vec::new(); input.len()];
        let mut last = 0;
        for (seq, name) in records {
            let slot = seq.checked_sub(1).and_then(|i| calls.get_mut(i)).ok_or_else(|| {
                TablesError::Invalid(format!("FontDame line {no}: lookup record position {seq} is beyond the {} input glyphs", input.len()))
            })?;
            if *seq < last {
                self.warnings.push(format!("FontDame line {no}: feature code calls lookups in input order, not in the order the records list them"));
            }
            last = *seq;
            let Some(i) = ctx.index.get(name).copied() else { continue };
            if ctx.empty.contains(&i) {
                self.warnings.push(format!("lookup \"{}\" calls \"{name}\", which has no rules; the call is left out", l.name));
                continue;
            }
            if let Some(n) = ctx.names.get(i) {
                slot.push(format!("lookup {n}"));
            }
        }
        // A rule that calls no lookup is what an `ignore` rule compiles to: it matches and stops
        // the lookup's later rules there; feature code has no other form for it.
        let ignore = if calls.iter().all(Vec::is_empty) { "ignore " } else { "" };
        let mut s = format!("{ignore}{} ", ctx.keyword);
        for p in prefix {
            s.push_str(p);
            s.push(' ');
        }
        let marked: Vec<String> =
            input.iter().zip(calls.iter()).map(|(g, c)| if c.is_empty() { format!("{g}'") } else { format!("{g}' {}", c.join(" ")) }).collect();
        s.push_str(&marked.join(" "));
        for p in suffix {
            s.push(' ');
            s.push_str(p);
        }
        s.push(';');
        Ok(s)
    }

    fn context(&mut self, l: &Lookup, base: &str, rules: &Rules, ctx: &LookupContext, defs: &mut Vec<String>, body: &mut Vec<String>) -> Result<()> {
        match rules {
            Rules::Glyph(list) => {
                for (seqs, records, no) in list {
                    let [bt, input, la] = seqs;
                    let prefix: Vec<String> = bt.iter().rev().map(|g| fea_glyph(g)).collect();
                    let input: Vec<String> = input.iter().map(|g| fea_glyph(g)).collect();
                    let suffix: Vec<String> = la.iter().map(|g| fea_glyph(g)).collect();
                    let r = self.rule(l, ctx, &prefix, &input, &suffix, records, *no)?;
                    body.push(r);
                }
            }
            Rules::Coverage { coverages, records, line } => {
                let [bt, input, la] = coverages;
                if bt.iter().chain(input.iter()).chain(la.iter()).any(Vec::is_empty) {
                    self.warnings.push(format!("FontDame line {line}: an empty coverage matches nothing; the rule is left out"));
                    return Ok(());
                }
                let prefix: Vec<String> = bt.iter().rev().map(|c| glyph_set(c)).collect();
                let input: Vec<String> = input.iter().map(|c| glyph_set(c)).collect();
                let suffix: Vec<String> = la.iter().map(|c| glyph_set(c)).collect();
                let r = self.rule(l, ctx, &prefix, &input, &suffix, records, *line)?;
                body.push(r);
            }
            Rules::Class { defs: class_defs, rules } => {
                if class_defs.get(1).is_none_or(Option::is_none) {
                    return Err(TablesError::Invalid(format!("lookup \"{}\": class rules without an input class definition", l.name)));
                }
                // Named classes by (slot, class, first input position).
                let mut named: HashMap<(usize, u16, bool), Option<String>> = HashMap::new();
                let mut needs_order = false;
                const SLOT_NAMES: [&str; 3] = ["backtrack", "input", "lookahead"];
                for (seqs, records, no) in rules {
                    let mut sides: [Vec<String>; 3] = Default::default();
                    let mut matches_nothing = false;
                    for (slot, seq) in seqs.iter().enumerate() {
                        let mut out = Vec::new();
                        for (pos, class) in seq.iter().enumerate() {
                            let first_input = slot == 1 && pos == 0;
                            let key = (slot, *class, first_input);
                            if let std::collections::hash_map::Entry::Vacant(e) = named.entry(key) {
                                let def = class_defs.get(slot).and_then(Option::as_ref);
                                let glyphs = match (def, *class) {
                                    (Some(d), 0) if first_input => {
                                        Some(d.iter().filter(|(_, c)| *c == 0).map(|(g, _)| g.clone()).collect::<Vec<_>>())
                                    }
                                    (Some(d), 0) => {
                                        let assigned: HashSet<&str> = d.iter().filter(|(_, c)| *c != 0).map(|(g, _)| g.as_str()).collect();
                                        self.complement(&assigned)?
                                    }
                                    (None, 0) => self.complement(&HashSet::new())?,
                                    (Some(d), c) => Some(d.iter().filter(|(_, k)| *k == c).map(|(g, _)| g.clone()).collect()),
                                    (None, _) => Some(Vec::new()),
                                };
                                let name = match glyphs {
                                    Some(g) if !g.is_empty() => {
                                        let all = if *class == 0 && !first_input { "all" } else { "" };
                                        let label = SLOT_NAMES.get(slot).copied().unwrap_or("input");
                                        Some(self.define(defs, &format!("{base}_{label}{class}{all}"), &g)?)
                                    }
                                    Some(_) => None,
                                    None => {
                                        needs_order = true;
                                        None
                                    }
                                };
                                e.insert(name);
                            }
                            match named.get(&key) {
                                Some(Some(n)) => out.push(format!("@{n}")),
                                _ => matches_nothing = true,
                            }
                        }
                        if let Some(s) = sides.get_mut(slot) {
                            *s = out;
                        }
                    }
                    if matches_nothing {
                        continue;
                    }
                    let [bt, input, la] = sides;
                    let prefix: Vec<String> = bt.into_iter().rev().collect();
                    let r = self.rule(l, ctx, &prefix, &input, &la, records, *no)?;
                    body.push(r);
                }
                if needs_order {
                    self.warnings.push(format!(
                        "lookup \"{}\": rules with class 0 (every glyph without a class) need the glyph order; they are left out",
                        l.name
                    ));
                }
            }
        }
        Ok(())
    }

    fn features(&mut self, prefix: &str, layout: &Layout, names: &[String], index: &HashMap<String, usize>, empty: &HashSet<usize>) -> Result<()> {
        let features = layout.features.as_deref().unwrap_or_default();
        let Some(rows) = layout.scripts.as_deref() else {
            if !features.is_empty() {
                self.warnings.push(format!("{prefix}: the features have no script table to place them, so they are left out"));
            }
            return Ok(());
        };
        let by_name: HashMap<&str, usize> = features.iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
        let resolve = |sym: &str, line: usize| -> Result<usize> {
            match sym.parse::<usize>() {
                Ok(i) => Ok(i),
                Err(_) => {
                    by_name.get(sym).copied().ok_or_else(|| TablesError::Invalid(format!("FontDame line {line}: feature \"{sym}\" is not defined")))
                }
            }
        };
        // Language systems, sorted as the binary table sorts them: by script, the default
        // language first, then by language.
        let mut systems: Vec<(String, String, Vec<usize>, Option<usize>)> = Vec::new();
        for row in rows {
            let mut list = Vec::new();
            for sym in &row.features {
                list.push(resolve(sym, row.line)?);
            }
            let required = row.required.as_deref().map(|r| resolve(r, row.line)).transpose()?;
            let lang = if row.lang == "default" { "dflt".to_string() } else { row.lang.clone() };
            systems.push((row.script.clone(), lang, list, required));
        }
        systems.sort_by(|a, b| (a.0.as_str(), a.1 != "dflt", a.1.as_str()).cmp(&(b.0.as_str(), b.1 != "dflt", b.1.as_str())));
        for (s, l, list, req) in &systems {
            if list.iter().chain(req.iter()).any(|f| *f >= features.len()) {
                self.warnings.push(format!("{prefix}: script {s} language {l} lists a feature the feature table does not have; it is left out"));
            }
        }
        let mut lookups_of: Vec<Vec<usize>> = Vec::new();
        for f in features {
            let mut list = Vec::new();
            for sym in &f.lookups {
                let i = index.get(sym).copied().ok_or_else(|| {
                    TablesError::Invalid(format!("FontDame line {}: feature \"{}\" names lookup \"{sym}\", which is not defined", f.line, f.name))
                })?;
                if empty.contains(&i) {
                    continue;
                }
                list.push(i);
            }
            lookups_of.push(list);
        }
        let mut tags: Vec<&str> = Vec::new();
        for f in features {
            if !tags.contains(&f.tag.as_str()) {
                tags.push(&f.tag);
            }
        }
        for tag in tags {
            let aalt = tag == "aalt";
            let mut body: Vec<String> = Vec::new();
            for (fi, f) in features.iter().enumerate().filter(|(_, f)| f.tag == tag) {
                let users: Vec<&(String, String, Vec<usize>, Option<usize>)> =
                    systems.iter().filter(|(_, _, list, req)| list.contains(&fi) || *req == Some(fi)).collect();
                if users.is_empty() {
                    self.warnings.push(format!("{prefix}: feature \"{}\" ({tag}) is listed by no language system and is left out", f.name));
                    continue;
                }
                let lookups: Vec<String> =
                    lookups_of.get(fi).into_iter().flatten().filter_map(|i| names.get(*i)).map(|n| format!("lookup {n};")).collect();
                if aalt {
                    if !body.is_empty() || users.len() > 1 {
                        self.warnings.push(format!("{prefix}: feature code has no script or language statements in 'aalt'; only the first language system's lookups are written"));
                    }
                    if body.is_empty() {
                        body.extend(lookups);
                    }
                    continue;
                }
                for (script, lang, _, req) in users {
                    let system = (script.clone(), lang.clone());
                    if !self.systems.contains(&system) {
                        self.systems.push(system);
                    }
                    body.push(format!("script {script};"));
                    let exclude = if lang == "dflt" { "" } else { " exclude_dflt" };
                    let required = if *req == Some(fi) { " required" } else { "" };
                    body.push(format!("language {lang}{exclude}{required};"));
                    body.extend(lookups.iter().cloned());
                }
            }
            if body.is_empty() {
                continue;
            }
            self.out.push_str(&format!("feature {tag} {{\n"));
            for line in body {
                self.out.push_str(&format!("    {line}\n"));
            }
            self.out.push_str(&format!("}} {tag};\n\n"));
        }
        Ok(())
    }
}

/// The lookup records of every contextual rule of a lookup.
fn lookup_records(l: &Lookup) -> Vec<&Record> {
    let mut out = Vec::new();
    for st in &l.subtables {
        if let Subtable::Context(rules) = st {
            match rules {
                Rules::Glyph(list) => out.extend(list.iter().flat_map(|(_, r, _)| r.iter())),
                Rules::Class { rules, .. } => out.extend(rules.iter().flat_map(|(_, r, _)| r.iter())),
                Rules::Coverage { records, .. } => out.extend(records.iter()),
            }
        }
    }
    out
}

/// The lookups in file order, except that a lookup a contextual rule calls is moved before the
/// first lookup that calls it, since feature code names only lookups already defined.
fn definition_order(lookups: &[Lookup], calls: &[Vec<usize>], prefix: &str) -> Result<Vec<usize>> {
    fn visit(i: usize, calls: &[Vec<usize>], state: &mut [u8], out: &mut Vec<usize>, depth: usize, lookups: &[Lookup], prefix: &str) -> Result<()> {
        match state.get(i) {
            Some(2) | None => return Ok(()),
            Some(1) => {
                let name = lookups.get(i).map_or("", |l| l.name.as_str());
                return Err(TablesError::Invalid(format!(
                    "{prefix} lookup \"{name}\" is called, directly or not, by a rule of its own; feature code cannot hold that"
                )));
            }
            _ => {}
        }
        if depth > MAX_LOOKUP_NESTING {
            return Err(TablesError::TooLarge { what: "the nesting of contextual lookups", limit: MAX_LOOKUP_NESTING });
        }
        if let Some(s) = state.get_mut(i) {
            *s = 1;
        }
        for &callee in calls.get(i).map(Vec::as_slice).unwrap_or_default() {
            visit(callee, calls, state, out, depth + 1, lookups, prefix)?;
        }
        if let Some(s) = state.get_mut(i) {
            *s = 2;
        }
        out.push(i);
        Ok(())
    }
    let mut state = vec![0u8; lookups.len()];
    let mut out = Vec::new();
    for i in 0..lookups.len() {
        visit(i, calls, &mut state, &mut out, 0, lookups, prefix)?;
    }
    Ok(out)
}
