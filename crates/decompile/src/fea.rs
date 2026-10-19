//! Feature code rebuilt from a compiled font's layout tables when it is opened
//! (`open.reconstructsFeatures`, M8.23).
//!
//! `GSUB`, `GPOS`, `GDEF` and `BASE` are read into feature code ([`write`]) and laid out as the
//! document's entries, so that exporting the opened font compiles layout tables equivalent to the
//! ones it was opened from:
//!
//! - **Classes**: every glyph set of two or more glyphs the code uses more than once, named
//!   after its first and last glyph (`a_z`).
//! - **Prefix `Languagesystems`**: the `languagesystem` statements of the scripts and languages
//!   the features use, `DFLT dflt` first.
//! - **Prefix `Prefix`**: the `markClass` statements, the `table GDEF` and `table BASE` blocks,
//!   and every lookup that cannot live in one feature: a lookup used by more than one feature
//!   (the handbook's rule), one a contextual rule calls, one no feature uses, and the lookups
//!   that have to come first so the lookup list keeps its order.
//! - **Features**, one entry per tag: each lookup only that feature uses is written inside it,
//!   where the feature first uses it, the others by `lookup name;`. A feature registered for
//!   every language system with one list of lookups has no `script` or `language` statements.
//!   `aalt` holds the rules of its own lookups, which the compiler builds `aalt` from.
//!   A stylistic set's name becomes the entry's label.
//!
//! Lookup order matters to shaping: of the lookups of a table, the ones written inside features
//! are a final stretch of the lookup list in which each feature's lookups follow one another,
//! and the entries are ordered by their first lookup; everything before that stretch goes into
//! `Prefix`. Compiling the entries in the oracles' order (classes, prefixes, features) gives the
//! lookup list back in the order the font had it.
//!
//! Pair kerning is not written as code when the document's kerning (`gpos_kern`) gives it back:
//! the `kern` feature's pair lookups are left out, and when the feature holds other lookups
//! (contextual kerning), or later lookups follow it, a `kern` entry keeps them with an
//! `# Automatic Code` line where the pair lookups were, so the kerning generated at export lands
//! there. When the kerning cannot give every pair lookup back (values other than the first
//! glyph's advance, device tables, other lookup flags, kerning read only in part), the document's
//! kerning is cleared and the `kern` feature stays code, so export does not kern twice.
//!
//! Whatever has no feature syntax is written as a `# not written:` comment and listed among the
//! warnings. Glyph names feature syntax cannot hold, and glyph ids past the font's glyphs, leave
//! every entry disabled, with a warning, so the font still exports; the kerning read then stays. A font whose layout tables cannot be read opens without feature code
//! and a warning; opening never fails because of them.
//!
//! Input is untrusted: the reader caps its work ([`write::MAX_LAYOUT_WORK`]), the text at
//! [`MAX_TEXT`] bytes and lookup nesting at [`MAX_NESTING`]; the layout moves at most 16 features
//! into the prefix one at a time before it puts every lookup there.

mod write;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{DEFAULT_LANGUAGE, Extra, Features, Localized, Snippet};
use skrifa::raw::types::Tag;
use skrifa::raw::{FileRef, FontRef, TableProvider};
use skrifa::string::StringId;

use crate::{DecompileError, Decompiled, MAX_CLASS_RECORDS};
use write::{FeaError, GPOS, GSUB, Layout, Line, LookupOut, Piece, SysKey, Systems, T4, Writer, fea_tag};

/// Most bytes of feature code written for one font.
pub const MAX_TEXT: usize = 16 * 1024 * 1024;
/// Deepest chain of contextual lookups calling lookups that is followed.
pub const MAX_NESTING: usize = 64;
/// The prefix that holds shared lookups (the handbook's name).
pub const PREFIX: &str = "Prefix";
/// The prefix that holds the `languagesystem` statements.
pub const LANGUAGESYSTEMS: &str = "Languagesystems";
/// The end of the note on a glyph id past the font's glyphs ([`write::Writer`]).
const BEYOND: &str = "beyond the font's glyph count";
/// The marker where generated kerning goes in a `kern` entry.
const AUTOMATIC_CODE: &str = "# Automatic Code";

const KERN: T4 = *b"kern";
const AALT: T4 = *b"aalt";
const SIZE: T4 = *b"size";
const LAYOUT_TABLES: [&[u8; 4]; 4] = [b"GSUB", b"GPOS", b"GDEF", b"BASE"];
/// Most features moved into the prefix one by one to keep the lookup order; past it, every
/// lookup goes into the prefix.
const MAX_PLAN_ROUNDS: usize = 16;
/// Most constructs named in the warning about what was not written.
const MAX_LISTED: usize = 10;

/// The kerning reader's warnings that mean the document's kerning does not give back every pair.
const PARTIAL_KERNING: [&str; 3] = ["kerning was read only in part", "kerning class tables with more than", "damaged kerning lookups"];
/// The kerning reader's note on contextual kerning, which the `kern` feature's code keeps.
const CONTEXTUAL_NOT_READ: &str = "(contextual kerning) were not read";
/// The start of the readers' list of tables left out.
const NOT_READ: &str = "tables not read: ";

/// Rebuilds the feature code of the opened font `d` from `font`'s layout tables (the hook of
/// [`crate::decompile`]). Failures become warnings.
pub(crate) fn import(font: &FontRef, d: &mut Decompiled) {
    if !LAYOUT_TABLES.iter().any(|t| font.table_data(Tag::new(t)).is_some()) {
        return;
    }
    let names: Vec<String> = d.font.glyphs.iter().map(|g| g.name.clone()).collect();
    let count = font.maxp().map(|m| usize::from(m.num_glyphs())).unwrap_or(0);
    if names.len() != count {
        d.warnings.push("the layout tables were not turned into feature code: the glyphs read do not match the font's glyph count".into());
        return;
    }
    let kerning_read = !d.font.kerning.is_empty() && !d.warnings.iter().any(|w| PARTIAL_KERNING.iter().any(|p| w.contains(p)));
    let rebuilt = match rebuild(font, &names, kerning_read) {
        Ok(r) => r,
        Err(FeaError(e)) => {
            d.warnings.push(format!("the layout tables were not turned into feature code: {e}"));
            return;
        }
    };
    let turned_off = !rebuilt.bad_names.is_empty() || !rebuilt.beyond.is_empty();
    // Turned-off entries export nothing, so the kerning read stays and export still kerns.
    if rebuilt.kerning_as_code && !turned_off && !d.font.kerning.is_empty() {
        d.font.kerning = Arc::default();
        for name in &names {
            if d.font.glyphs.get(name).is_some_and(|g| !g.kern_groups.is_empty())
                && let Ok(mut g) = d.font.glyph_mut(name)
            {
                g.kern_groups = Default::default();
            }
        }
        d.warnings.push("the kerning stays feature code in the kern feature: its lookups hold values the kerning cannot give back".into());
    }
    d.warnings.retain(|w| !w.contains(CONTEXTUAL_NOT_READ));
    let present: Vec<String> =
        LAYOUT_TABLES.iter().filter(|t| font.table_data(Tag::new(t)).is_some()).map(|t| String::from_utf8_lossy(t.as_slice()).into_owned()).collect();
    for w in &mut d.warnings {
        let Some(list) = w.strip_prefix(NOT_READ) else { continue };
        let left: Vec<&str> = list
            .split(", ")
            .filter(|t| !present.iter().any(|p| *t == p.as_str() || t.strip_prefix(p.as_str()).is_some_and(|r| r.starts_with(" ("))))
            .collect();
        *w = if left.is_empty() { String::new() } else { format!("{NOT_READ}{}", left.join(", ")) };
    }
    d.warnings.retain(|w| !w.is_empty());
    let mut unsupported = rebuilt.unsupported;
    if !rebuilt.beyond.is_empty() {
        unsupported.retain(|u| !u.ends_with(BEYOND));
        let shown: Vec<String> = rebuilt.beyond.iter().take(MAX_LISTED).map(u16::to_string).collect();
        d.warnings.push(format!(
            "the feature code made from the layout tables is turned off: its rules use glyph ids the font does not have ({}), which feature code cannot name",
            shown.join(", ")
        ));
    }
    if !unsupported.is_empty() {
        let shown: Vec<&str> = unsupported.iter().take(MAX_LISTED).map(String::as_str).collect();
        let more = unsupported.len().saturating_sub(MAX_LISTED);
        let more = if more > 0 { format!(" and {more} more") } else { String::new() };
        d.warnings.push(format!("the feature code made from the layout tables leaves out: {}{more}", shown.join("; ")));
    }
    if !rebuilt.bad_names.is_empty() {
        let shown: Vec<&str> = rebuilt.bad_names.iter().take(MAX_LISTED).map(String::as_str).collect();
        d.warnings.push(format!(
            "the feature code made from the layout tables is turned off: feature code cannot name the glyphs {}; rename them and turn the entries on",
            shown.join(", ")
        ));
    }
    d.font.features = Arc::new(rebuilt.features);
}

/// The entries and what [`import`] reports.
struct Rebuilt {
    features: Features,
    /// The `kern` feature's pair lookups stay code; the document's kerning must go.
    kerning_as_code: bool,
    unsupported: Vec<String>,
    bad_names: Vec<String>,
    /// Glyph ids the rules use that the font does not have.
    beyond: Vec<u16>,
}

type Id = (T4, u16);

/// One statement of a feature block, in the order the block writes it.
enum Step {
    Script(T4),
    Language(String),
    Lookup(Id),
}

/// The statements of a feature registered for `systems`; `all` is every language system of the
/// font. A feature with one list of lookups for every language system needs no `script` or
/// `language` statements.
fn walk(systems: &Systems, all: &BTreeSet<SysKey>) -> Vec<Step> {
    let first = systems.values().next();
    if let Some((false, ids)) = first
        && systems.len() == all.len()
        && all.iter().all(|k| systems.contains_key(k))
        && systems.values().all(|(required, list)| !required && list == ids)
    {
        return ids.iter().map(|id| Step::Lookup(*id)).collect();
    }
    type LangEntry<'a> = (T4, bool, &'a Vec<Id>);
    let mut scripts: BTreeMap<(bool, T4), Vec<LangEntry<'_>>> = BTreeMap::new();
    for ((nd, script, _, lang), (required, ids)) in systems {
        scripts.entry((*nd, *script)).or_default().push((*lang, *required, ids));
    }
    let mut steps = Vec::new();
    for ((_, script), langs) in scripts {
        steps.push(Step::Script(script));
        let dflt: Vec<Id> = langs.iter().find(|(l, _, _)| *l == write::DFLT_LANG).map(|(_, _, ids)| (*ids).clone()).unwrap_or_default();
        let dflt_set: HashSet<Id> = dflt.iter().copied().collect();
        for (lang, required, ids) in &langs {
            let req = if *required { " required" } else { "" };
            let list: Vec<Id> = if *lang == write::DFLT_LANG {
                if *required {
                    steps.push(Step::Language("language dflt required;".into()));
                }
                (*ids).clone()
            } else {
                let has: HashSet<Id> = ids.iter().copied().collect();
                if !dflt.is_empty() && dflt.iter().all(|d| has.contains(d)) {
                    steps.push(Step::Language(format!("language {}{req};", fea_tag(*lang))));
                    ids.iter().filter(|i| !dflt_set.contains(*i)).copied().collect()
                } else {
                    let ex = if dflt.is_empty() { "" } else { " exclude_dflt" };
                    steps.push(Step::Language(format!("language {}{ex}{req};", fea_tag(*lang))));
                    (*ids).clone()
                }
            };
            steps.extend(list.into_iter().map(Step::Lookup));
        }
    }
    steps
}

/// The length of the part of `seq` that goes into the prefix: what follows is a run of lookups
/// per feature, each feature's run in one piece.
fn split_point(seq: &[Id], owner: &dyn Fn(&Id) -> Option<T4>) -> usize {
    let mut k = seq.len();
    let mut seen: HashSet<T4> = HashSet::new();
    let mut current: Option<T4> = None;
    while k > 0 {
        let Some(id) = seq.get(k - 1) else { break };
        match owner(id) {
            Some(tag) if current == Some(tag) => {}
            Some(tag) if !seen.contains(&tag) => {
                seen.insert(tag);
                current = Some(tag);
            }
            _ => break,
        }
        k -= 1;
    }
    k
}

/// How the lookups are laid out.
struct Plan {
    /// Lookups written inside a feature, with that feature.
    inline: HashMap<Id, T4>,
    /// Per table (`GSUB`, `GPOS`), the lookups in the order they are defined.
    seqs: Vec<Vec<Id>>,
    /// Per table, how many of `seqs` go into the prefix.
    splits: Vec<usize>,
}

fn rebuild(font: &FontRef, names: &[String], kerning_read: bool) -> Result<Rebuilt, FeaError> {
    let mut w = Writer::new(names);
    let mut layout: Layout = w.read(font, &[])?;
    let variable = font.fvar().is_ok();
    let kinds: HashMap<Id, u16> = layout.lookups.iter().map(|l| ((l.table, l.index), l.kind)).collect();

    // The kern feature's pair lookups, and whether the kerning gives them back.
    let kern_pairs: BTreeSet<Id> = layout
        .features
        .get(&KERN)
        .map(|s| s.values().flat_map(|(_, ids)| ids.iter().copied()).filter(|id| id.0 == GPOS && kinds.get(id) == Some(&2)).collect())
        .unwrap_or_default();
    let replace = kerning_read && !kern_pairs.is_empty() && kern_pairs.iter().all(|id| write::plain_pair(font, id.1, variable, MAX_CLASS_RECORDS));
    let kerning_as_code = !replace && !kern_pairs.is_empty();
    let dropped: HashSet<Id> = if replace { kern_pairs.iter().copied().collect() } else { HashSet::new() };

    // Who uses each lookup.
    let mut users: HashMap<Id, BTreeSet<T4>> = HashMap::new();
    for (tag, systems) in &layout.features {
        for (_, ids) in systems.values() {
            for id in ids {
                if *tag == KERN && dropped.contains(id) {
                    continue;
                }
                users.entry(*id).or_default().insert(*tag);
            }
        }
    }
    let callers: HashSet<Id> = layout.lookups.iter().flat_map(|l| l.refs.iter().map(move |r| (l.table, *r))).collect();
    let only = |id: &Id| users.get(id).filter(|u| u.len() == 1).and_then(|u| u.iter().next().copied());
    let aalt_rules: HashSet<Id> = layout
        .lookups
        .iter()
        .filter(|l| l.table == GSUB && matches!(l.kind, 1 | 3) && l.flag.is_none())
        .map(|l| (l.table, l.index))
        .filter(|id| only(id) == Some(AALT) && !callers.contains(id))
        .collect();
    let omitted: HashSet<Id> = dropped.iter().copied().filter(|id| !users.contains_key(id) && !callers.contains(id)).collect();
    let base_owner = |id: &Id| -> Option<T4> {
        if callers.contains(id) {
            return None;
        }
        only(id).filter(|t| *t != AALT && *t != SIZE)
    };

    let seqs: Vec<Vec<Id>> = [GSUB, GPOS]
        .iter()
        .map(|t| {
            layout
                .lookups
                .iter()
                .filter(|l| l.table == *t)
                .map(|l| (l.table, l.index))
                .filter(|id| !omitted.contains(id) && !aalt_rules.contains(id))
                .collect()
        })
        .collect();
    let positions: HashMap<Id, usize> = seqs.iter().flat_map(|s| s.iter().enumerate().map(|(i, id)| (*id, i))).collect();

    // Features whose lookups go into the prefix to keep the lookup list in order.
    let walks: Vec<(T4, Vec<Step>)> = layout.features.iter().map(|(tag, systems)| (*tag, walk(systems, &layout.systems))).collect();
    let mut forced: HashSet<T4> = HashSet::new();
    let plan = loop {
        if forced.len() > MAX_PLAN_ROUNDS {
            // Everything in the prefix keeps the order whatever the features do.
            break Plan { inline: HashMap::new(), seqs: seqs.clone(), splits: seqs.iter().map(Vec::len).collect() };
        }
        let owner = |id: &Id| base_owner(id).filter(|t| !forced.contains(t));
        let mut inline: HashMap<Id, T4> = HashMap::new();
        let mut splits = Vec::new();
        for seq in &seqs {
            let k = split_point(seq, &owner);
            splits.push(k);
            for id in seq.iter().skip(k) {
                if let Some(t) = owner(id) {
                    inline.insert(*id, t);
                }
            }
        }
        let mut tables: HashMap<T4, BTreeSet<T4>> = HashMap::new();
        for (id, tag) in &inline {
            tables.entry(*tag).or_default().insert(id.0);
        }
        // A feature with lookups in both tables could need two orders at once.
        let mut bad: Option<T4> = tables.iter().filter(|(_, t)| t.len() > 1).map(|(tag, _)| *tag).min();
        if bad.is_none() {
            for (tag, steps) in &walks {
                let mut seen: HashSet<Id> = HashSet::new();
                let mut last: Option<usize> = None;
                let mut in_order = true;
                for step in steps {
                    let Step::Lookup(id) = step else { continue };
                    if inline.get(id) != Some(tag) || !seen.insert(*id) {
                        continue;
                    }
                    let p = positions.get(id).copied();
                    if last.is_some() && p <= last {
                        in_order = false;
                        break;
                    }
                    last = p;
                }
                if !in_order {
                    bad = Some(*tag);
                    break;
                }
            }
        }
        match bad {
            Some(tag) if forced.insert(tag) => {}
            _ => break Plan { inline, seqs: seqs.clone(), splits },
        }
    };

    // The lines of each entry.
    let mut lookups: HashMap<Id, LookupOut> = HashMap::new();
    let mut marks: Vec<Line> = Vec::new();
    for mut l in std::mem::take(&mut layout.lookups) {
        let id = (l.table, l.index);
        if omitted.contains(&id) {
            continue;
        }
        marks.append(&mut l.pre);
        lookups.insert(id, l);
    }
    let mut defined: HashSet<Id> = HashSet::new();
    let mut languages: Vec<Line> = Vec::new();
    for (_, script, _, lang) in &layout.systems {
        languages.push(Line::text(0, format!("languagesystem {} {};", fea_tag(*script), fea_tag(*lang))));
    }
    let mut prefix: Vec<Line> = std::mem::take(&mut layout.head);
    for block in [marks, std::mem::take(&mut layout.gdef), std::mem::take(&mut layout.base)] {
        if block.is_empty() {
            continue;
        }
        if !prefix.is_empty() {
            prefix.push(Line::text(0, String::new()));
        }
        prefix.extend(block);
    }
    for (seq, k) in plan.seqs.iter().zip(&plan.splits) {
        for id in seq.iter().take(*k) {
            if let Some(l) = lookups.remove(id) {
                if !prefix.is_empty() {
                    prefix.push(Line::text(0, String::new()));
                }
                prefix.extend(lookup_block(l, &mut defined, &mut w));
            }
        }
    }

    // Features, by where their lookups start.
    let min_dropped = dropped.iter().map(|id| id.1).min();
    let gpos_seq = plan.seqs.get(1).cloned().unwrap_or_default();
    let kern_entry_needed = min_dropped.is_some_and(|m| gpos_seq.iter().any(|id| id.1 > m));
    let mut order: Vec<((usize, usize, u8), T4)> = Vec::new();
    for tag in layout.features.keys() {
        let first = plan.inline.iter().filter(|(_, t)| *t == tag).filter_map(|(id, _)| Some((usize::from(id.0 == GPOS), *positions.get(id)?))).min();
        let key = match first {
            Some((t, p)) => (t, p, 1),
            None if *tag == KERN && kern_entry_needed => {
                let p = min_dropped.and_then(|m| gpos_seq.iter().position(|id| id.1 > m)).unwrap_or(gpos_seq.len());
                (1, p, 0)
            }
            None => (2, 0, 1),
        };
        order.push((key, *tag));
    }
    order.sort();
    let mut chunks: Vec<Vec<Line>> = vec![languages, prefix];
    let mut tags: Vec<T4> = Vec::new();
    for (_, tag) in order {
        let Some(systems) = layout.features.get(&tag) else { continue };
        let mut lines: Vec<Line> = layout.params.remove(&tag).unwrap_or_default();
        if tag == AALT {
            let mut seen = HashSet::new();
            for step in walk(systems, &layout.systems) {
                let Step::Lookup(id) = step else { continue };
                if !seen.insert(id) {
                    continue;
                }
                if aalt_rules.contains(&id) {
                    if let Some(l) = lookups.remove(&id) {
                        for mut line in l.body.into_iter().filter(|l| !l.is_subtable_break()) {
                            line.indent = 0;
                            lines.push(line);
                        }
                    }
                } else if defined.contains(&id) {
                    lines.push(Line::text(0, format!("lookup {};", w.lookup_name(id.0, id.1))));
                }
            }
        } else {
            let steps = walk(systems, &layout.systems);
            let kept = steps.iter().any(|s| matches!(s, Step::Lookup(id) if !dropped.contains(id) || tag != KERN));
            if tag == KERN && !kept {
                if kern_entry_needed {
                    lines.push(Line::text(0, AUTOMATIC_CODE.into()));
                }
            } else {
                let mut marker = false;
                for step in steps {
                    match step {
                        Step::Script(s) => lines.push(Line::text(0, format!("script {};", fea_tag(s)))),
                        Step::Language(l) => lines.push(Line::text(0, l)),
                        Step::Lookup(id) if tag == KERN && dropped.contains(&id) => {
                            if !marker {
                                lines.push(Line::text(0, AUTOMATIC_CODE.into()));
                                marker = true;
                            }
                        }
                        Step::Lookup(id) => {
                            if plan.inline.get(&id) == Some(&tag)
                                && let Some(l) = lookups.remove(&id)
                            {
                                lines.extend(lookup_block(l, &mut defined, &mut w));
                            } else if defined.contains(&id) {
                                lines.push(Line::text(0, format!("lookup {};", w.lookup_name(id.0, id.1))));
                            }
                        }
                    }
                }
            }
        }
        if lines.is_empty() && !layout.set_names.contains_key(&tag) {
            continue;
        }
        chunks.push(lines);
        tags.push(tag);
    }

    let rendered = w.render(&chunks, true)?;
    let disabled = !rendered.bad_names.is_empty() || !rendered.beyond.is_empty();
    let snippet = |name: String, code: &str| Snippet { name, code: code.trim_end().to_string(), disabled, ..Default::default() };
    let mut features = Features::default();
    for (name, members) in &rendered.classes {
        features.classes.push(snippet(name.clone(), members));
    }
    let mut texts = rendered.texts.iter();
    for name in [LANGUAGESYSTEMS, PREFIX] {
        if let Some(text) = texts.next()
            && !text.trim().is_empty()
        {
            features.prefixes.push(snippet(name.to_string(), text));
        }
    }
    for (tag, text) in tags.iter().zip(texts) {
        let mut entry = snippet(fea_tag(*tag), text);
        if let Some(id) = layout.set_names.get(tag)
            && let Some(name) = crate::english(font, &[StringId::new(*id)])
        {
            entry.labels.push(Localized { language: DEFAULT_LANGUAGE.into(), value: name, extra: Extra::new() });
        }
        features.features.push(entry);
    }
    Ok(Rebuilt { features, kerning_as_code, unsupported: rendered.unsupported, bad_names: rendered.bad_names, beyond: rendered.beyond })
}

/// A lookup block. A rule calling a lookup not defined before it (a cycle) is commented out.
fn lookup_block(l: LookupOut, defined: &mut HashSet<Id>, w: &mut Writer) -> Vec<Line> {
    let mut lines = vec![Line::text(0, format!("lookup {} {{", l.name))];
    if let Some(flag) = l.flag {
        lines.push(flag);
    }
    for mut line in l.body {
        if let Some(missing) = line.refs.iter().find(|r| **r == l.index || !defined.contains(&(l.table, **r))) {
            let name = w.lookup_name(l.table, *missing);
            let mut note = w.note(&format!("a rule of lookup {} calls lookup {name}, which cannot be defined before it", l.name));
            note.indent = 1;
            lines.push(note);
            line.pieces.insert(0, Piece::Text("# ".into()));
            line.refs.clear();
        }
        lines.push(line);
    }
    lines.push(Line::text(0, format!("}} {};", l.name)));
    defined.insert((l.table, l.index));
    lines
}

// ── comparing layouts ───────────────────────────────────────────────────────────────────────

/// A font's layout in a form two fonts can be compared by, glyphs by name: each lookup's rules
/// as sorted lines, the features as lists of lookup indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutSummary {
    /// `GSUB`, then `GPOS`, when the font has them.
    pub tables: Vec<TableSummary>,
    /// The `table GDEF` block's lines.
    pub gdef: Vec<String>,
    /// One sentence per construct without feature syntax.
    pub unsupported: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableSummary {
    pub tag: String,
    /// By lookup index.
    pub lookups: BTreeMap<u16, LookupSummary>,
    pub features: Vec<FeatureSummary>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LookupSummary {
    /// The lookup type, extension lookups read through.
    pub kind: u16,
    /// The `lookupflag` statement, `markClass` statements and rules, sorted, without
    /// `subtable;` breaks; the lookup's own name is `SELF` and the lookups rules call are `#`.
    pub lines: Vec<String>,
    /// The lookups its rules call, sorted.
    pub calls: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureSummary {
    pub tag: String,
    pub script: String,
    pub language: String,
    pub required: bool,
    pub lookups: Vec<u16>,
}

/// The layout tables of the sfnt `bytes` as a [`LayoutSummary`], glyphs named by `names` (by
/// glyph id), leaving out the features `exclude` names and the lookups only they reach.
pub fn layout_summary(bytes: &[u8], names: &[String], exclude: &[&str]) -> Result<LayoutSummary, DecompileError> {
    let font = match FileRef::new(bytes).map_err(|e| DecompileError::Unreadable(e.to_string()))? {
        FileRef::Font(f) => f,
        FileRef::Collection(c) => c.get(0).map_err(|e| DecompileError::Unreadable(e.to_string()))?,
    };
    let exclude: Vec<T4> = exclude
        .iter()
        .map(|t| {
            let mut tag = [b' '; 4];
            for (slot, b) in tag.iter_mut().zip(t.bytes()) {
                *slot = b;
            }
            tag
        })
        .collect();
    let mut w = Writer::new(names);
    let mut layout = w.read(&font, &exclude).map_err(|FeaError(e)| DecompileError::Unreadable(e))?;
    let mut chunks: Vec<Vec<Line>> = Vec::new();
    let mut meta: Vec<(T4, u16, u16, Vec<u16>)> = Vec::new();
    for l in std::mem::take(&mut layout.lookups) {
        let mut lines: Vec<Line> = l.pre;
        lines.extend(l.flag);
        for mut line in l.body.into_iter().filter(|line| !line.is_subtable_break()) {
            for piece in &mut line.pieces {
                if let Piece::Text(t) = piece
                    && t.starts_with(" lookup ")
                {
                    *t = " lookup #".into();
                }
            }
            line.indent = 0;
            lines.push(line);
        }
        let mut calls: Vec<u16> = l.refs.iter().copied().collect();
        calls.sort_unstable();
        meta.push((l.table, l.index, l.kind, calls));
        chunks.push(lines);
    }
    let names_of: Vec<String> = meta.iter().map(|(t, i, _, _)| w.lookup_name(*t, *i)).collect();
    chunks.push(std::mem::take(&mut layout.gdef));
    let rendered = w.render(&chunks, false).map_err(|FeaError(e)| DecompileError::Unreadable(e))?;
    let mut summary = LayoutSummary { unsupported: rendered.unsupported, ..Default::default() };
    let mut texts = rendered.texts.into_iter();
    let mut tables: BTreeMap<T4, TableSummary> = BTreeMap::new();
    for ((table, index, kind, calls), name) in meta.into_iter().zip(names_of) {
        let Some(text) = texts.next() else { break };
        let mut lines: Vec<String> = text.lines().map(|line| line.replace(&name, "SELF")).collect();
        lines.sort();
        tables
            .entry(table)
            .or_insert_with(|| TableSummary { tag: fea_tag(table), ..Default::default() })
            .lookups
            .insert(index, LookupSummary { kind, lines, calls });
    }
    summary.gdef = texts.next().map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default();
    for (tag, systems) in &layout.features {
        for ((_, script, _, language), (required, ids)) in systems {
            for table in [GSUB, GPOS] {
                let lookups: Vec<u16> = ids.iter().filter(|id| id.0 == table).map(|id| id.1).collect();
                if lookups.is_empty() {
                    continue;
                }
                tables.entry(table).or_insert_with(|| TableSummary { tag: fea_tag(table), ..Default::default() }).features.push(FeatureSummary {
                    tag: fea_tag(*tag),
                    script: fea_tag(*script),
                    language: fea_tag(*language),
                    required: *required,
                    lookups,
                });
            }
        }
    }
    summary.tables = [GSUB, GPOS].iter().filter_map(|t| tables.remove(t)).collect();
    Ok(summary)
}

#[cfg(test)]
mod tests;
