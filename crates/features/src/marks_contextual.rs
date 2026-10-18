//! Contextual mark attachment (M8.21): anchors whose name starts with `*` replace the plain anchor
//! of the same base name wherever the glyph occurs in the context stored with the anchor.
//!
//! Rows: `anchors.contextual`, `anchors.contextSyntax`.
//!
//! Ported from ufo2ft (https://github.com/googlefonts/ufo2ft), MIT License, version 3.9.1: the
//! contextual part of `Lib/ufo2ft/featureWriters/markFeatureWriter.py` (the asterisk rule of
//! `parseAnchorName`, `_makeContextualAttachments`, `_makeAllContextualMarkLookups`,
//! `_makeContextualMarkLookup`, and the lookup order of `_makeMarkFeature`, `_makeMkmkFeature`
//! and `_makeAbvmOrBlwmFeature`).
//!
//! Contract:
//! - The context of an anchor is its user data under [`CONTEXT_KEY`] ([`anchor_context`]). The
//!   compiler collects them per compiled glyph ([`contexts_of`]) and runs [`for_master`].
//! - A context is feature code: one context per line; an optional `lookupflag …;` before the
//!   pattern; `*` stands for the glyph holding the anchor and `&` for the mark (after `*` when
//!   left out). Tokens are expanded first, with the master's values.
//! - When at least one contextual rule is written, the mark writer's features (`abvm`, `blwm`,
//!   `mark`, `mkmk`) are written as feature code ([`ContextualMarks::code`]): the plain lookups
//!   of [`super::generate_marks`] first, then the lookups the contexts call, then one dispatch
//!   lookup per distinct `lookupflag` prefix, so a contextual attachment is applied after the
//!   plain one and overrides it. The compiler appends that code to the feature file
//!   ([`with_code`]) and turns those features off in the generated lookups
//!   ([`ContextualMarks::tags_left`]); `curs`, the kerning and `GDEF` stay generated.
//! - Static targets only: a variable font keeps the plain anchors, with a warning.
//!
//! What the writer does, as ufo2ft does:
//! 1. An anchor is contextual when its name starts with `*` and it has a context. Its key is the
//!    name without the `*` and without everything from the first `.` (`*bottom.reh` attaches by
//!    `bottom`); a trailing `_2` makes it the anchor of a ligature's second component.
//! 2. It takes part when the glyph takes part in mark attachment and some mark glyph has the
//!    anchor `_key`. On a mark glyph it goes to `mkmk`; a numbered one on a ligature (or on any
//!    glyph without `GDEF` classes) to `mark` as mark-to-ligature; an unnumbered one on a base or
//!    ligature to `abvm` or `blwm` (by its key, as the plain lookups split) when the glyph is one
//!    of the Indic glyphs, else to `mark`.
//! 3. Contexts are taken longest first. Each context and key gets a lookup holding the
//!    attachments of every glyph with that context, and a rule in the dispatch lookup of its
//!    `lookupflag` prefix: `*` becomes the glyphs, `&` the marks of the key marked for that
//!    lookup.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_fea::{Diagnostic, VirtualFiles};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Anchor as ModelAnchor, Font};
use write_fonts::tables::gdef::GlyphClassDef;
use write_fonts::types::{GlyphId16, Tag};

use super::{
    ABVM, AnchorKind, BLWM, Budget, MARK, MAX_LIGATURE_COMPONENTS, MKMK, MarkLookup, MarkRequest, MarkRules, MarkTags, NAMED_IN_WARNING, Point16,
    gdef, generate_marks, is_below_mark, listed, read_anchors, split_abvm, tag_of,
};
use crate::properties::{DFLT_LANG, DFLT_SCRIPT};
use crate::source_map::EntryKind;
use crate::target::MasterValues;
use crate::{FeatureError, MAX_ENTRY_BYTES, TargetValues};

/// The user-data key of an anchor's context, where the reference's files and ufo2ft keep it.
pub const CONTEXT_KEY: &str = "GPOS_Context";
/// Most bytes of one anchor's context.
pub const MAX_CONTEXT_BYTES: usize = 16_384;
/// Most contexts (lines) of one compile; the rest are left out with a warning.
pub const MAX_CONTEXTS: usize = 10_000;
/// Most bytes of the code [`for_master`] writes.
pub const MAX_CODE_BYTES: usize = MAX_ENTRY_BYTES;
/// The virtual file the code is compiled from, included at the end of the root file.
pub const CONTEXTUAL_FILE: &str = "contextual_marks.fea";
/// Characters a context may not hold: they would end the generated lookup or rule.
const REFUSED: [char; 4] = ['{', '}', '#', '"'];

/// The context code of `anchor`, when it has one.
pub fn anchor_context(anchor: &ModelAnchor) -> Option<&str> {
    anchor.info.user_data.get(CONTEXT_KEY).and_then(|v| v.as_str())
}

/// The contexts of contextual anchors, by (compiled glyph name, anchor name).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnchorContexts(pub BTreeMap<(String, String), String>);

impl AnchorContexts {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The contexts of the `*` anchors of `glyphs`, each a compiled glyph name with the source glyph
/// it comes from: those of the glyph's layer for `master`, then those only another master layer
/// has (an instance has no layer of its own). At most [`MAX_CONTEXTS`] anchors.
pub fn contexts_of(font: &Font, master: &str, glyphs: &[(&str, &str)]) -> AnchorContexts {
    let mut out = BTreeMap::new();
    for (compiled, source) in glyphs {
        let Some(glyph) = font.glyph(source) else { continue };
        let own = glyph.layer(master);
        let others = glyph.layers.iter().filter(|l| l.is_master_layer() && l.id != master);
        for layer in own.into_iter().chain(others) {
            for anchor in &layer.anchors {
                if !anchor.name.starts_with('*') {
                    continue;
                }
                let Some(context) = anchor_context(anchor) else { continue };
                if out.len() >= MAX_CONTEXTS {
                    return AnchorContexts(out);
                }
                out.entry((compiled.to_string(), anchor.name.clone())).or_insert_with(|| context.to_string());
            }
        }
    }
    AnchorContexts(out)
}

/// What [`contextual_marks`] writes from.
#[derive(Clone, Copy)]
pub struct ContextualRequest<'a> {
    /// The mark writer's request for the master; its tags say which features may be written.
    pub marks: &'a MarkRequest<'a>,
    pub contexts: &'a AnchorContexts,
    /// Evaluates the tokens of the contexts; `None` leaves a context with a token out.
    pub values: Option<&'a dyn TargetValues>,
}

/// The mark writer's features as feature code, with the contextual lookups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextualMarks {
    /// Mark classes, lookups and feature blocks, to be compiled after the font's code.
    pub code: String,
    /// The features the code has a block for, in the order written.
    pub features: Vec<Tag>,
    /// How many contextual rules the code has.
    pub rules: usize,
    /// Contexts left out and why.
    pub warnings: Vec<String>,
}

impl ContextualMarks {
    /// `tags` without the features [`ContextualMarks::code`] writes, for the generated lookups.
    pub fn tags_left(&self, tags: MarkTags) -> MarkTags {
        MarkTags { mark: false, mkmk: false, abvm: false, blwm: false, ..tags }
    }
}

/// The compiler's entry: the contextual code of `marks.master` for a compile of `glyphs`
/// (compiled name, source name), or `None` when no contextual rule comes of the font's anchors.
/// A variable font, or a writer that fails, gives `None` with a warning.
pub fn for_master(
    font: &Font,
    data: Option<&GlyphData>,
    marks: &MarkRequest,
    glyphs: &[(&str, &str)],
    variable: bool,
    warnings: &mut Vec<String>,
) -> Option<ContextualMarks> {
    let contexts = contexts_of(font, marks.master, glyphs);
    if contexts.is_empty() {
        return None;
    }
    if variable {
        warnings.push(format!(
            "{} contextual anchors were left out: contextual mark attachment is written to static fonts only, so the variable font uses the plain anchors",
            contexts.0.len()
        ));
        return None;
    }
    let values = data.and_then(|data| MasterValues::new(font, data, marks.master).ok());
    let req = ContextualRequest { marks, contexts: &contexts, values: values.as_ref().map(|v| v as &dyn TargetValues) };
    match contextual_marks(&req) {
        Ok(found) => {
            if let Some(found) = &found {
                warnings.extend(found.warnings.iter().cloned());
            }
            found
        }
        Err(e) => {
            warnings.push(format!("contextual mark attachment was left out, the plain anchors are used: {e}"));
            None
        }
    }
}

/// `files` with [`CONTEXTUAL_FILE`] holding `code`, included at the end of the root file.
pub fn with_code(files: &VirtualFiles, code: &str) -> Result<VirtualFiles, Diagnostic> {
    let root = format!("{}\ninclude({CONTEXTUAL_FILE});\n", files.root_text());
    let mut out = VirtualFiles::new(files.root(), &root)?;
    for (path, text) in files.iter().filter(|(p, _)| *p != files.root()) {
        out.insert(path, text)?;
    }
    out.insert(CONTEXTUAL_FILE, code)?;
    if let Some(dir) = files.include_dir() {
        out = out.with_include_dir(dir.to_path_buf());
    }
    if let Some(check) = files.include_check() {
        out = out.with_include_check(check.clone());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Reading the anchors
// ---------------------------------------------------------------------------------------------

/// What a contextual anchor name says: the key it attaches by and its ligature component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextualName {
    pub(crate) key: String,
    pub(crate) number: Option<usize>,
}

/// Reads a `*` anchor name as ufo2ft does. `Ok(None)`: not a contextual name, or one ufo2ft
/// ignores (a key that does not start with a letter). `Err`: a name ufo2ft refuses.
pub(crate) fn contextual_name(name: &str) -> Result<Option<ContextualName>, &'static str> {
    let Some(rest) = name.strip_prefix('*') else { return Ok(None) };
    let rest = rest.split('.').next().unwrap_or_default();
    let digits = rest.len().saturating_sub(rest.trim_end_matches(|c: char| c.is_ascii_digit()).len());
    let (key, number) = match rest.split_at_checked(rest.len().saturating_sub(digits)) {
        Some((head, tail)) if !tail.is_empty() => match head.strip_suffix('_') {
            Some(key) => (key, Some(tail.parse::<usize>().map_err(|_| "the component number is too large")?)),
            None => (rest, None),
        },
        _ => (rest, None),
    };
    if rest.starts_with('_') && !key.is_empty() {
        return Err("a contextual anchor cannot be a mark anchor");
    }
    if !key.chars().next().is_some_and(char::is_alphabetic) {
        return Ok(None);
    }
    match number {
        Some(0) => Err("a ligature component number must not be 0"),
        Some(n) if n > MAX_LIGATURE_COMPONENTS => Err("the component number is too large"),
        _ => Ok(Some(ContextualName { key: key.to_string(), number })),
    }
}

/// The anchor name a parsed kind came from, for `*` names (which read as base or ligature
/// anchors).
fn kind_name(kind: &AnchorKind) -> Option<String> {
    match kind {
        AnchorKind::Base(g) => Some(g.clone()),
        AnchorKind::Ligature { group, index } => Some(format!("{group}_{index}")),
        _ => None,
    }
}

/// One contextual anchor that takes part.
#[derive(Clone, Debug)]
struct Contextual {
    glyph: GlyphId16,
    key: String,
    number: Option<usize>,
    point: Point16,
    /// How many components the glyph's ligature anchors give it (for a numbered anchor).
    components: usize,
}

/// Where a contextual anchor's rules go.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Base,
    Ligature,
    Mark,
    Abvm,
}

/// Contexts in the order first seen, each with its anchors (indices into the anchor list).
#[derive(Default)]
struct ByContext {
    order: Vec<(String, Vec<usize>)>,
    at: HashMap<String, usize>,
}

impl ByContext {
    fn push(&mut self, context: &str, anchor: usize) {
        match self.at.get(context) {
            Some(&k) => {
                if let Some((_, list)) = self.order.get_mut(k) {
                    list.push(anchor);
                }
            }
            None => {
                self.at.insert(context.to_string(), self.order.len());
                self.order.push((context.to_string(), vec![anchor]));
            }
        }
    }

    /// The contexts longest first (by characters, as ufo2ft counts), the order kept for equal
    /// lengths.
    fn longest_first(&self) -> Vec<(String, Vec<usize>)> {
        let mut out = self.order.clone();
        out.sort_by_key(|(c, _)| Reverse(c.chars().count()));
        out
    }
}

/// The lines of a context after token expansion: trimmed, without trailing `;`, empty lines
/// dropped. `Err`: why the anchor's context is left out.
fn context_lines(text: &str, values: Option<&dyn TargetValues>) -> Result<Vec<String>, String> {
    if text.len() > MAX_CONTEXT_BYTES {
        return Err(format!("its context is longer than {MAX_CONTEXT_BYTES} bytes"));
    }
    let expanded = if text.contains('$') {
        let Some(values) = values else { return Err("its context has a token, and no values were given to evaluate it".into()) };
        match crate::tokens::expand(text, values, EntryKind::Feature) {
            Ok((code, _)) => code,
            Err(found) => {
                let first = found.first().map_or_else(String::new, |d| d.message.clone());
                return Err(format!("a token of its context could not be evaluated: {first}"));
            }
        }
    } else {
        text.to_string()
    };
    if let Some(c) = expanded.chars().find(|c| REFUSED.contains(c) || (c.is_control() && *c != '\n' && *c != '\r' && *c != '\t')) {
        return Err(format!("its context holds {c:?}, which a context may not hold"));
    }
    let lines: Vec<String> = expanded.split(['\n', '\r']).map(|l| l.trim().trim_end_matches(';').to_string()).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return Err("its context is empty".into());
    }
    if let Some(l) = lines.iter().find(|l| !l.contains('*')) {
        return Err(format!("its context `{l}` has no `*` for the glyph"));
    }
    Ok(lines)
}

// ---------------------------------------------------------------------------------------------
// Writing the code
// ---------------------------------------------------------------------------------------------

/// One attachment of a lookup a context calls.
#[derive(Clone, Debug)]
enum Attach {
    Base(GlyphId16, Point16),
    Mark(GlyphId16, Point16),
    Ligature(GlyphId16, Vec<Option<Point16>>),
}

impl Attach {
    fn glyph(&self) -> GlyphId16 {
        match self {
            Attach::Base(g, _) | Attach::Mark(g, _) | Attach::Ligature(g, _) => *g,
        }
    }
}

/// A dispatch lookup: its `lookupflag` prefix, name and statements.
struct Dispatch {
    before: String,
    name: String,
    statements: Vec<String>,
}

/// The contextual lookups of one feature: those the rules call, then the dispatch lookups.
#[derive(Default)]
struct ContextLookups {
    called: Vec<(String, Vec<String>)>,
    dispatch: Vec<Dispatch>,
}

/// The code being written: definitions (mark classes, filtering classes) and the lookups and
/// feature blocks, within [`MAX_CODE_BYTES`] together.
struct Writer<'a> {
    names: Vec<&'a str>,
    defs: String,
    body: String,
    mark_classes: HashMap<(String, Vec<(GlyphId16, Point16)>), String>,
    taken: HashSet<String>,
}

fn too_long() -> FeatureError {
    FeatureError::Limit { what: "bytes of contextual mark code", limit: MAX_CODE_BYTES }
}

/// `text` as a name feature code accepts: letters, digits, `_` and `.`, at most 56 bytes (with
/// room for a numeric suffix under the 63 the specification allows), starting with a letter or `_`.
fn fea_name(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' }).take(56).collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert(0, '_');
    }
    out
}

impl<'a> Writer<'a> {
    fn new(names: Vec<&'a str>) -> Self {
        Writer { names, defs: String::new(), body: String::new(), mark_classes: HashMap::new(), taken: HashSet::new() }
    }

    fn room(&self, more: usize) -> Result<(), FeatureError> {
        let total = self.defs.len().saturating_add(self.body.len()).saturating_add(more);
        if total > MAX_CODE_BYTES { Err(too_long()) } else { Ok(()) }
    }

    fn def(&mut self, line: &str) -> Result<(), FeatureError> {
        self.room(line.len().saturating_add(1))?;
        self.defs.push_str(line);
        self.defs.push('\n');
        Ok(())
    }

    fn line(&mut self, line: &str) -> Result<(), FeatureError> {
        self.room(line.len().saturating_add(1))?;
        self.body.push_str(line);
        self.body.push('\n');
        Ok(())
    }

    /// A name not taken yet, from `base`.
    fn unique(&mut self, base: &str) -> String {
        let base = fea_name(base);
        let mut name = base.clone();
        let mut n = 1usize;
        while self.taken.contains(&name) {
            name = format!("{base}_{n}");
            n = n.saturating_add(1);
        }
        self.taken.insert(name.clone());
        name
    }

    /// A glyph as feature code names it, escaped so no glyph name reads as a keyword.
    fn glyph(&self, gid: GlyphId16) -> Result<String, FeatureError> {
        let name = self.names.get(usize::from(gid.to_u16())).ok_or_else(|| FeatureError::Unsupported(format!("no glyph has id {}", gid.to_u16())))?;
        Ok(format!("\\{name}"))
    }

    fn glyph_list(&self, gids: impl IntoIterator<Item = GlyphId16>) -> Result<String, FeatureError> {
        let names: Vec<String> = gids.into_iter().map(|g| self.glyph(g)).collect::<Result<_, _>>()?;
        Ok(format!("[{}]", names.join(" ")))
    }

    /// The mark class of `key` holding `marks`, defined on first use.
    fn mark_class(&mut self, key: &str, marks: &[(GlyphId16, Point16)]) -> Result<String, FeatureError> {
        let id = (key.to_string(), marks.to_vec());
        if let Some(name) = self.mark_classes.get(&id) {
            return Ok(name.clone());
        }
        let name = format!("@{}", self.unique(&format!("gen_mark_{key}")));
        for (gid, (x, y)) in marks {
            let line = format!("markClass {} <anchor {x} {y}> {name};", self.glyph(*gid)?);
            self.def(&line)?;
        }
        self.mark_classes.insert(id, name.clone());
        Ok(name)
    }

    fn lookup(&mut self, name: &str, statements: &[String]) -> Result<(), FeatureError> {
        self.line(&format!("lookup {name} {{"))?;
        for s in statements {
            self.line(&format!("    {s}"))?;
        }
        self.line(&format!("}} {name};"))?;
        self.line("")
    }

    fn anchor((x, y): Point16) -> String {
        format!("<anchor {x} {y}>")
    }

    /// The statements of one attachment, its marks in `class`.
    fn attach(&self, a: &Attach, class: &str) -> Result<String, FeatureError> {
        Ok(match a {
            Attach::Base(g, p) => format!("pos base {} {} mark {class};", self.glyph(*g)?, Self::anchor(*p)),
            Attach::Mark(g, p) => format!("pos mark {} {} mark {class};", self.glyph(*g)?, Self::anchor(*p)),
            Attach::Ligature(g, slots) => {
                let parts: Vec<String> =
                    slots.iter().map(|s| s.map_or_else(|| "<anchor NULL>".to_string(), |p| format!("{} mark {class}", Self::anchor(p)))).collect();
                format!("pos ligature {} {};", self.glyph(*g)?, parts.join(" ligComponent "))
            }
        })
    }

    /// One of the writer's plain lookups as code; returns its name. `None` for a cursive lookup.
    fn plain(&mut self, tag: &str, l: &MarkLookup) -> Result<Option<String>, FeatureError> {
        let (kind, attachments, marks): (&str, Vec<Attach>, &[(GlyphId16, Point16)]) = match &l.rules {
            MarkRules::MarkToBase { marks, bases } => ("mark2base", bases.iter().map(|(g, p)| Attach::Base(*g, *p)).collect(), marks.as_slice()),
            MarkRules::MarkToMark { marks, bases } => ("mark2mark", bases.iter().map(|(g, p)| Attach::Mark(*g, *p)).collect(), marks.as_slice()),
            MarkRules::MarkToLigature { marks, ligatures } => {
                ("mark2liga", ligatures.iter().map(|(g, slots)| Attach::Ligature(*g, slots.clone())).collect(), marks.as_slice())
            }
            MarkRules::Cursive { .. } => return Ok(None),
        };
        let class = self.mark_class(&l.class, marks)?;
        let name = self.unique(&format!("gen_{tag}_{kind}_{}", l.class));
        let mut statements = Vec::with_capacity(attachments.len().saturating_add(1));
        if let Some(set) = &l.filter_set {
            let set_name = format!("@{}", self.unique(&format!("gen_MFS_{name}")));
            let members = self.glyph_list(set.iter().copied())?;
            self.def(&format!("{set_name} = {members};"))?;
            statements.push(format!("lookupflag UseMarkFilteringSet {set_name};"));
        }
        for a in &attachments {
            statements.push(self.attach(a, &class)?);
        }
        self.lookup(&name, &statements)?;
        Ok(Some(name))
    }

    /// ufo2ft's `_makeContextualMarkLookup`: for each key of one context, a lookup with its
    /// attachments and a rule in the dispatch lookup of the context's `lookupflag` prefix.
    fn context(
        &mut self,
        prefix: &str,
        context: &str,
        attachments: &[(String, Vec<Attach>)],
        classes: &BTreeMap<String, Vec<(GlyphId16, Point16)>>,
        out: &mut ContextLookups,
    ) -> Result<usize, FeatureError> {
        // Several statements before the pattern are kept together.
        let (before, after) = match context.rsplit_once(';') {
            Some((before, after)) => (before.trim().to_string(), after.trim()),
            None => (String::new(), context.trim()),
        };
        let mut rules = 0usize;
        for (key, list) in attachments {
            let Some(marks) = classes.get(key) else { continue };
            let class = self.mark_class(key, marks)?;
            let k = match out.dispatch.iter().position(|d| d.before == before) {
                Some(k) => k,
                None => {
                    let name = self.unique(&format!("gen_{prefix}Dispatch_{}", out.dispatch.len()));
                    let statements = if before.is_empty() { Vec::new() } else { vec![format!("{before};")] };
                    out.dispatch.push(Dispatch { before: before.clone(), name, statements });
                    out.dispatch.len() - 1
                }
            };
            let called = self.unique(&format!("gen_{prefix}_{}", out.called.len()));
            let bases = self.glyph_list(list.iter().map(Attach::glyph))?;
            let mark_glyphs = self.glyph_list(marks.iter().map(|(g, _)| *g))?;
            let pattern = if after.contains('&') { after.to_string() } else { after.replace('*', "* &") };
            let mut rule = String::with_capacity(pattern.len().saturating_add(bases.len()).saturating_add(mark_glyphs.len()));
            for c in pattern.chars() {
                match c {
                    '*' => rule.push_str(&bases),
                    '&' => {
                        rule.push_str(&mark_glyphs);
                        rule.push_str("' lookup ");
                        rule.push_str(&called);
                    }
                    other => rule.push(other),
                }
                self.room(rule.len())?;
            }
            let Some(d) = out.dispatch.get_mut(k) else { continue };
            d.statements.push(format!("# {after}"));
            d.statements.push(format!("pos {rule};"));
            let statements = list.iter().map(|a| self.attach(a, &class)).collect::<Result<Vec<_>, _>>()?;
            out.called.push((called, statements));
            rules = rules.saturating_add(1);
        }
        Ok(rules)
    }
}

/// The attachments of one context, grouped by key in the order first seen; a glyph listed twice
/// for a key keeps its first place and its last anchor, a ligature's components merged.
fn grouped(anchors: &[Contextual], list: &[usize], mark: bool) -> Vec<(String, Vec<Attach>)> {
    let mut out: Vec<(String, Vec<Attach>)> = Vec::new();
    for a in list.iter().filter_map(|i| anchors.get(*i)) {
        let attach = match a.number {
            Some(n) => {
                let mut slots = vec![None; a.components.max(n)];
                if let Some(slot) = n.checked_sub(1).and_then(|k| slots.get_mut(k)) {
                    *slot = Some(a.point);
                }
                Attach::Ligature(a.glyph, slots)
            }
            None if mark => Attach::Mark(a.glyph, a.point),
            None => Attach::Base(a.glyph, a.point),
        };
        let group = match out.iter().position(|(k, _)| *k == a.key) {
            Some(k) => k,
            None => {
                out.push((a.key.clone(), Vec::new()));
                out.len() - 1
            }
        };
        let Some((_, items)) = out.get_mut(group) else { continue };
        match items.iter_mut().find(|i| i.glyph() == a.glyph) {
            Some(Attach::Ligature(_, have)) => {
                if let Attach::Ligature(_, new) = attach {
                    if new.len() > have.len() {
                        have.resize(new.len(), None);
                    }
                    for (slot, p) in have.iter_mut().zip(new) {
                        if p.is_some() {
                            *slot = p;
                        }
                    }
                }
            }
            Some(existing) => *existing = attach,
            None => items.push(attach),
        }
    }
    out
}

/// The contextual code of the request's master, or `None` when no contextual rule comes of it.
pub fn contextual_marks(req: &ContextualRequest) -> Result<Option<ContextualMarks>, FeatureError> {
    let m = req.marks;
    let tags = m.tags;
    if req.contexts.is_empty() || !(tags.mark || tags.mkmk || tags.abvm || tags.blwm) {
        return Ok(None);
    }
    let mut warnings = Vec::new();
    // The plain writer's run reports these.
    let mut quiet = Vec::new();
    let mut budget = Budget::new(m.limits.work);
    let glyph_count = m.input.glyphs.len().min(usize::from(u16::MAX));
    let all = read_anchors(m, &mut budget, &mut quiet)?;
    let classes = gdef::glyph_classes(m.input, &all, &mut quiet);
    let include: HashSet<GlyphId16> =
        classes.iter().filter(|(_, c)| matches!(c, GlyphClassDef::Base | GlyphClassDef::Mark | GlyphClassDef::Ligature)).map(|(g, _)| *g).collect();
    let takes_part = |gid: &GlyphId16| include.is_empty() || include.contains(gid);
    let class_is = |gid: GlyphId16, c: GlyphClassDef| classes.is_empty() || classes.get(&gid) == Some(&c);
    let name_of = |gid: GlyphId16| m.input.glyphs.get(usize::from(gid.to_u16())).map(|g| g.name.as_str());

    // The contextual anchors with a context, and the keys every base-side anchor uses.
    let mut found: Vec<(GlyphId16, ContextualName, Point16, &str)> = Vec::new();
    let mut base_keys: HashSet<String> = HashSet::new();
    let mut mark_keys: HashSet<&str> = HashSet::new();
    let mut refused: Vec<String> = Vec::new();
    for (gid, anchors) in all.iter().filter(|(g, _)| takes_part(g)) {
        budget.spend(anchors.len())?;
        let Some(glyph) = name_of(*gid) else { continue };
        for a in anchors {
            match &a.kind {
                AnchorKind::Mark(k) => {
                    mark_keys.insert(k.as_str());
                }
                AnchorKind::Base(g) | AnchorKind::Ligature { group: g, .. } if !g.starts_with('*') => {
                    base_keys.insert(g.clone());
                }
                _ => {}
            }
            let Some(name) = kind_name(&a.kind).filter(|n| n.starts_with('*')) else { continue };
            let Some(context) = req.contexts.0.get(&(glyph.to_string(), name.clone())) else { continue };
            match contextual_name(&name) {
                Ok(Some(parsed)) => {
                    base_keys.insert(parsed.key.clone());
                    found.push((*gid, parsed, a.point(), context.as_str()));
                }
                Ok(None) => {}
                Err(why) => refused.push(format!("`{name}` of `{glyph}` ({why})")),
            }
        }
    }
    if !refused.is_empty() {
        let shown: Vec<String> = refused.iter().take(NAMED_IN_WARNING).cloned().collect();
        warnings.push(format!("{} contextual anchors were left out, their names are not valid: {}", refused.len(), listed(&shown, refused.len())));
    }

    // The mark classes: the marks of each key some base-side anchor uses.
    let mut mark_classes: BTreeMap<String, Vec<(GlyphId16, Point16)>> = BTreeMap::new();
    for (gid, anchors) in all.iter().filter(|(g, _)| takes_part(g) && class_is(**g, GlyphClassDef::Mark)) {
        for a in anchors {
            if let AnchorKind::Mark(k) = &a.kind
                && base_keys.contains(k)
            {
                mark_classes.entry(k.clone()).or_default().push((*gid, a.point()));
            }
        }
    }
    let mark_glyphs: BTreeSet<GlyphId16> = mark_classes.values().flatten().map(|(g, _)| *g).collect();

    // The Indic glyphs, as the plain writer splits them.
    let mut char_map: HashMap<u32, GlyphId16> = HashMap::new();
    for (i, g) in m.input.glyphs.iter().enumerate().take(glyph_count) {
        let Ok(id) = u16::try_from(i) else { break };
        budget.spend(g.codepoints.len())?;
        for cp in &g.codepoints {
            char_map.entry(*cp).or_insert(GlyphId16::new(id));
        }
    }
    let mut language_systems: Vec<(Tag, Tag)> = m.input.language_systems.iter().filter_map(|(s, l)| Some((tag_of(s)?, tag_of(l)?))).collect();
    language_systems.sort_unstable();
    language_systems.dedup();
    if language_systems.is_empty() {
        language_systems.push((DFLT_SCRIPT, DFLT_LANG));
    }
    let gsub: Option<read_fonts::tables::gsub::Gsub> = m.gsub.and_then(|bytes| read_fonts::FontRead::read(read_fonts::FontData::new(bytes)).ok());
    let (abvm_glyphs, _) = split_abvm(&char_map, glyph_count, &language_systems, gsub.as_ref());

    // Each glyph's component count: its highest ligature number among the anchors that attach.
    let components = |gid: GlyphId16| -> usize {
        let plain = all.get(&gid).into_iter().flatten().filter_map(|a| match &a.kind {
            AnchorKind::Ligature { group, index } if mark_keys.contains(group.as_str()) => Some(*index),
            _ => None,
        });
        let contextual = found.iter().filter(|(g, n, _, _)| *g == gid && mark_keys.contains(n.key.as_str())).filter_map(|(_, n, _, _)| n.number);
        plain.chain(contextual).max().unwrap_or(0)
    };

    // Sorted by glyph name, then in the glyph's anchor order.
    let mut order: Vec<usize> = (0..found.len()).collect();
    order.sort_by(|a, b| {
        let name = |i: &usize| found.get(*i).and_then(|(g, ..)| name_of(*g)).unwrap_or_default();
        name(a).cmp(name(b))
    });
    let mut anchors: Vec<Contextual> = Vec::new();
    let (mut base, mut liga, mut mkmk, mut abvm) = (ByContext::default(), ByContext::default(), ByContext::default(), ByContext::default());
    let mut lines_left = MAX_CONTEXTS;
    let mut skipped: Vec<String> = Vec::new();
    for i in order {
        let Some((gid, parsed, point, text)) = found.get(i) else { continue };
        budget.spend(1)?;
        // A key without marks (or without a mark class) attaches nothing.
        if !mark_keys.contains(parsed.key.as_str()) || !mark_classes.contains_key(&parsed.key) {
            continue;
        }
        let glyph = name_of(*gid).unwrap_or_default();
        let dest = if mark_glyphs.contains(gid) {
            if parsed.number.is_some() {
                skipped.push(format!("`*{}_{}` of `{glyph}` (a ligature anchor on a mark glyph)", parsed.key, parsed.number.unwrap_or(0)));
                continue;
            }
            Dest::Mark
        } else if parsed.number.is_some() && class_is(*gid, GlyphClassDef::Ligature) {
            Dest::Ligature
        } else if parsed.number.is_none() && (class_is(*gid, GlyphClassDef::Base) || classes.get(gid) == Some(&GlyphClassDef::Ligature)) {
            if abvm_glyphs.contains(*gid) { Dest::Abvm } else { Dest::Base }
        } else {
            continue;
        };
        let lines = match context_lines(text, req.values) {
            Ok(lines) => lines,
            Err(why) => {
                skipped.push(format!("a contextual anchor of `{glyph}`: {why}"));
                continue;
            }
        };
        let index = anchors.len();
        anchors.push(Contextual {
            glyph: *gid,
            key: parsed.key.clone(),
            number: parsed.number,
            point: *point,
            components: if parsed.number.is_some() { components(*gid) } else { 0 },
        });
        for line in lines {
            if lines_left == 0 {
                warnings.push(format!("only the first {MAX_CONTEXTS} anchor contexts were written"));
                break;
            }
            lines_left -= 1;
            budget.spend(line.len())?;
            let into = match dest {
                Dest::Base => &mut base,
                Dest::Ligature => &mut liga,
                Dest::Mark => &mut mkmk,
                Dest::Abvm => &mut abvm,
            };
            into.push(&line, index);
        }
        if lines_left == 0 {
            break;
        }
    }
    if !skipped.is_empty() {
        let shown: Vec<String> = skipped.iter().take(NAMED_IN_WARNING).cloned().collect();
        warnings.push(format!("{} contextual anchors were left out: {}", skipped.len(), listed(&shown, skipped.len())));
    }

    // The plain lookups and the contextual ones, feature by feature, in ufo2ft's order: the
    // plain lookups, the lookups the contexts call, the dispatch lookups, then the mark-to-mark
    // lookups `abvm` and `blwm` hold.
    let generated = generate_marks(m, &mut quiet)?;
    let names: Vec<&str> = m.input.glyphs.iter().take(glyph_count).map(|g| g.name.as_str()).collect();
    let mut w = Writer::new(names);
    let mut blocks: Vec<(Tag, Vec<String>)> = Vec::new();
    let mut rules = 0usize;
    let below = |anchor: &usize| anchors.get(*anchor).is_some_and(|a| is_below_mark(&a.key));
    let abvm_on = !abvm_glyphs.is_empty();
    for (tag, on) in [(ABVM, tags.abvm && abvm_on), (BLWM, tags.blwm && abvm_on), (MARK, tags.mark), (MKMK, tags.mkmk)] {
        let label = tag.to_string();
        let nested = |l: &&MarkLookup| (tag == ABVM || tag == BLWM) && matches!(l.rules, MarkRules::MarkToMark { .. });
        let mut used = Vec::new();
        for l in generated.lookups.iter().filter(|l| l.feature == tag).filter(|l| !nested(l)) {
            budget.spend(1)?;
            used.extend(w.plain(&label, l)?);
        }
        let mut ctx = ContextLookups::default();
        if on {
            let (prefix, sources, mark): (&str, Vec<(String, Vec<usize>)>, bool) = if tag == MARK {
                ("ContextualMark", base.longest_first().into_iter().chain(liga.longest_first()).collect(), false)
            } else if tag == MKMK {
                ("ContextualMarkToMark", mkmk.longest_first(), true)
            } else {
                let want_below = tag == BLWM;
                let prefix = if want_below { "ContextualBlwm" } else { "ContextualAbvm" };
                let kept: Vec<(String, Vec<usize>)> =
                    abvm.longest_first().into_iter().map(|(c, l)| (c, l.into_iter().filter(|a| below(a) == want_below).collect())).collect();
                (prefix, kept, false)
            };
            for (context, list) in sources {
                if list.is_empty() {
                    continue;
                }
                budget.spend(list.len())?;
                let attachments = grouped(&anchors, &list, mark);
                rules = rules.saturating_add(w.context(prefix, &context, &attachments, &mark_classes, &mut ctx)?);
            }
        }
        for (name, statements) in &ctx.called {
            w.lookup(name, statements)?;
        }
        for d in &ctx.dispatch {
            w.lookup(&d.name, &d.statements)?;
        }
        used.extend(ctx.dispatch.iter().map(|d| d.name.clone()));
        for l in generated.lookups.iter().filter(|l| l.feature == tag).filter(nested) {
            budget.spend(1)?;
            used.extend(w.plain(&label, l)?);
        }
        if !used.is_empty() {
            blocks.push((tag, used));
        }
    }
    if rules == 0 {
        return Ok(None);
    }
    let mut features = Vec::with_capacity(blocks.len());
    for (tag, used) in &blocks {
        w.line(&format!("feature {tag} {{"))?;
        for name in used {
            w.line(&format!("    lookup {name};"))?;
        }
        w.line(&format!("}} {tag};"))?;
        features.push(*tag);
    }
    let mut code = String::from("# Mark attachment with contextual anchors, generated from the anchors and their contexts.\n");
    code.push_str(&w.defs);
    code.push('\n');
    code.push_str(&w.body);
    if code.len() > MAX_CODE_BYTES {
        return Err(too_long());
    }
    Ok(Some(ContextualMarks { code, features, rules, warnings }))
}

#[cfg(test)]
#[path = "marks_contextual_tests.rs"]
mod tests;
