//! The sidebar of Font View (`docs/spec/05-font-view.md` § 6.2): its entries, what each one
//! contains, how a selection of entries combines with rules and search, and the glyphs an entry
//! expects that the font lacks.
//!
//! An entry is addressed by a path whose first segment names the section: `all`,
//! `categories/Letter`, `categories/Letter/Uppercase`, `languages/latin`, `languages/latin/de`,
//! `filters/Folder/My filter`. Entries of a `Groups*.plist` file follow the built-in ones in
//! Categories and Languages, by name.
//!
//! Combination: entries of Categories add up (union), and so do entries of Languages; entries of
//! Filters narrow each other (intersection); sections narrow each other; a folder is the union of
//! what it holds. Rules and search narrow the result further. Results follow the glyph order,
//! except that a selected list filter shows its glyphs in list order.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::filters::{Filter, filter_at};
use crate::predicate::{Attr, Expr, Literal, Op, Test, parse_predicate};
use crate::rules::{CASE_GROUPS, category_expr};
use crate::search::Matcher;
use crate::{
    Env, MAX_ENTRIES, MAX_FILTER_NAME_BYTES, MAX_GROUP_ENTRIES, MAX_GROUP_NESTING, MAX_GROUPS_BYTES, MAX_LANGUAGE_CHARACTERS, MAX_LANGUAGES,
    MAX_PATH_SEGMENTS, QueryError, Subject, fail, short,
};
use fontcraft_glyphdata::{Category, GlyphData, SubCategory};
use fontcraft_model::{data_props, glyph_order};
use fontcraft_plist::Value as Plist;

/// The scripts the Languages section lists until the user chooses others. Scripts the font has
/// glyphs for are listed as well.
pub const DEFAULT_VISIBLE_SCRIPTS: &[&str] = &["latin", "greek", "cyrillic"];

/// One entry of the Languages section below a script: a language or subset and the characters
/// it needs. The data comes from the caller (the language tables of the glyph database).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSet {
    pub id: String,
    pub name: String,
    /// The script id it is listed under (`latin`).
    pub script: String,
    pub characters: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    All,
    Categories,
    Languages,
    Filters,
}

/// The `categories` and `languages` entries of `Groups*.plist` files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Groups {
    pub categories: Vec<GroupEntry>,
    pub languages: Vec<GroupEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupEntry {
    pub name: String,
    pub icon: Option<String>,
    pub rule: GroupRule,
    pub children: Vec<GroupEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GroupRule {
    /// Only a container: the union of its children.
    None,
    Script(String),
    Predicate(Expr),
    /// Glyph names, in order.
    List(Vec<String>),
}

/// A glyph an entry expects: its name, and the code point it stands for when the entry knows
/// one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expected {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_point: Option<u32>,
}

/// A resolved sidebar entry.
pub struct Entry {
    pub path: Vec<String>,
    pub section: Section,
    member: Member,
    expected: Option<Vec<Expected>>,
}

impl Entry {
    /// Whether the entry knows its full glyph set (and so can list missing glyphs).
    pub fn knows_set(&self) -> bool {
        self.expected.is_some()
    }

    /// Every glyph the entry expects, present or not, when it knows its full set.
    pub fn expected(&self) -> Option<&[Expected]> {
        self.expected.as_deref()
    }
}

/// What an entry contains.
#[derive(Clone, Debug)]
enum Member {
    Everything,
    Pred(Expr),
    Names { order: Vec<String>, set: HashSet<String> },
    Codes { codes: HashSet<u32>, names: HashSet<String> },
    Union(Vec<Member>),
    Intersect(Vec<Member>),
}

impl Member {
    fn names(order: &[String]) -> Member {
        Member::Names { order: order.to_vec(), set: order.iter().cloned().collect() }
    }

    fn contains(&self, s: &Subject, env: &Env) -> bool {
        match self {
            Member::Everything => true,
            Member::Pred(e) => e.matches(s, env),
            Member::Names { set, .. } => set.contains(&s.glyph.name),
            Member::Codes { codes, names } => s.glyph.unicodes.iter().any(|c| codes.contains(c)) || names.contains(&s.glyph.name),
            Member::Union(v) => v.iter().any(|m| m.contains(s, env)),
            Member::Intersect(v) => v.iter().all(|m| m.contains(s, env)),
        }
    }
}

/// What a query selects: sidebar entries, ad-hoc rules (compiled), and a search term.
#[derive(Default)]
pub struct Selection {
    pub entries: Vec<Vec<String>>,
    pub rules: Option<Expr>,
    pub search: Option<Matcher>,
}

/// One row of the sidebar with its glyph count, and its badge numbers when it knows its full set.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarItem {
    pub path: Vec<String>,
    pub label: String,
    /// `all`, `section`, `category`, `subcategory`, `case`, `script`, `language`, `group`,
    /// `smart`, `list` or `folder`.
    pub kind: &'static str,
    pub count: usize,
    /// Glyphs the entry expects, when it knows its full set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<usize>,
    /// How many of those the font has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SidebarItem>,
    /// Why the entry cannot be evaluated (a stored rule that no longer reads).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// A sidebar path from JSON: `"filters/Folder/Name"` or `["filters", "Folder", "Name"]` (the
/// list form for names that contain `/`).
pub fn parse_path(v: &Value) -> Result<Vec<String>, QueryError> {
    let segments: Vec<String> = match v {
        Value::String(s) => s.split('/').map(str::to_string).collect(),
        Value::Array(items) if items.len() <= MAX_PATH_SEGMENTS => items
            .iter()
            .map(|i| i.as_str().map(str::to_string).ok_or_else(|| QueryError("a path is a list of names".into())))
            .collect::<Result<_, _>>()?,
        Value::Array(_) => return fail(format!("a path has at most {MAX_PATH_SEGMENTS} parts")),
        _ => return fail("a path is text such as `categories/Letter` or a list of names"),
    };
    if segments.is_empty() || segments.len() > MAX_PATH_SEGMENTS {
        return fail(format!("a path has from 1 to {MAX_PATH_SEGMENTS} parts"));
    }
    for s in &segments {
        if s.is_empty() {
            return fail("a path has an empty part");
        }
        if s.len() > MAX_FILTER_NAME_BYTES {
            return fail(format!("a path part is at most {MAX_FILTER_NAME_BYTES} bytes long"));
        }
    }
    Ok(segments)
}

/// A usable script id: lowercase letters, digits, `_` or `-`, at most 64 bytes.
pub fn check_script_id(id: &str) -> Result<String, QueryError> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return fail(format!("`{}` is not a script id (lowercase letters such as latin)", short(id)));
    }
    Ok(id.to_string())
}

// ── Groups*.plist ───────────────────────────────────────────────────────────────────────────────

/// Reads a `Groups*.plist` file (an old-style or XML property list with `categories` and
/// `languages` lists). Entries that cannot be used are left out and described in the second
/// value; a file that cannot be read at all is an error.
pub fn parse_groups(text: &str, data: &GlyphData) -> Result<(Groups, Vec<String>), QueryError> {
    if text.len() > MAX_GROUPS_BYTES {
        return fail(format!("the groups file is larger than {MAX_GROUPS_BYTES} bytes"));
    }
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let root = if trimmed.starts_with('<') {
        fontcraft_plist::xmlplist::parse(text).map(|d| d.root)
    } else {
        fontcraft_plist::openstep::parse(text).map(|d| d.root)
    }
    .map_err(|e| QueryError(format!("the groups file cannot be read: {e}")))?;
    let Plist::Dict(dict) = root else { return fail("a groups file holds a dictionary with `categories` and `languages` lists") };
    let mut problems = Vec::new();
    let mut count = 0;
    let mut section = |key: &str, problems: &mut Vec<String>| -> Vec<GroupEntry> {
        match dict.get(key) {
            Some(Plist::Array(a)) => group_entries(&a.items, 1, &mut count, data, problems, key),
            Some(_) => {
                problems.push(format!("`{key}` is not a list"));
                Vec::new()
            }
            None => Vec::new(),
        }
    };
    let categories = section("categories", &mut problems);
    let languages = section("languages", &mut problems);
    Ok((Groups { categories, languages }, problems))
}

fn plist_str(v: Option<&Plist>) -> Option<&str> {
    match v {
        Some(Plist::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn group_entries(items: &[Plist], depth: usize, count: &mut usize, data: &GlyphData, problems: &mut Vec<String>, place: &str) -> Vec<GroupEntry> {
    if depth > MAX_GROUP_NESTING {
        problems.push(format!("{place}: groups are nested more than {MAX_GROUP_NESTING} levels deep"));
        return Vec::new();
    }
    let mut out = Vec::new();
    for item in items {
        *count += 1;
        if *count > MAX_GROUP_ENTRIES {
            problems.push(format!("only the first {MAX_GROUP_ENTRIES} group entries are read"));
            break;
        }
        let Plist::Dict(d) = item else {
            problems.push(format!("{place}: an entry is not a dictionary"));
            continue;
        };
        let Some(name) = plist_str(d.get("name")).map(str::trim).filter(|n| !n.is_empty() && n.len() <= MAX_FILTER_NAME_BYTES) else {
            problems.push(format!("{place}: an entry has no usable `name`"));
            continue;
        };
        let here = format!("{place}/{}", short(name));
        let rule = if let Some(script) = plist_str(d.get("script")) {
            GroupRule::Script(script.to_string())
        } else if let Some(predicate) = plist_str(d.get("predicate")) {
            match parse_predicate(predicate) {
                Ok(e) => GroupRule::Predicate(e),
                Err(e) => {
                    problems.push(format!("{here}: {e}"));
                    continue;
                }
            }
        } else if let Some(Plist::Array(list)) = d.get("list") {
            let mut names = Vec::new();
            let mut seen = HashSet::new();
            for v in &list.items {
                let Some(text) = plist_str(Some(v)) else { continue };
                let mut chars = text.chars();
                let name = match (chars.next(), chars.next()) {
                    (Some(c), None) => data.nice_name(c as u32),
                    _ => text.to_string(),
                };
                if names.len() < crate::MAX_LIST_NAMES && !name.is_empty() && seen.insert(name.clone()) {
                    names.push(name);
                }
            }
            GroupRule::List(names)
        } else {
            GroupRule::None
        };
        let children = match d.get("subGroup") {
            Some(Plist::Array(a)) => group_entries(&a.items, depth + 1, count, data, problems, &here),
            _ => Vec::new(),
        };
        out.push(GroupEntry { name: name.to_string(), icon: plist_str(d.get("icon")).map(str::to_string), rule, children });
    }
    out
}

// ── The catalog ─────────────────────────────────────────────────────────────────────────────────

/// A category, how many glyphs it has, and how many each of its child labels has.
type CategoryCount<'s> = (&'s str, usize, Vec<(&'s str, usize)>);

/// The category, subcategory and case the glyph data gives a character's glyph.
type DataClass = (Option<String>, Option<String>, Option<String>);

/// The language sets named through one glyph data: the glyphs each set and each script expects,
/// and the category, subcategory and case of each of those glyphs. None of it depends on the
/// font, so one index serves every catalog built on the same sets and glyph data. Names and
/// classes are worked out when first asked for, so scripts the sidebar does not list cost nothing.
pub struct LanguageIndex {
    /// Held so the addresses [`LanguageIndex::is_for`] compares stay unique.
    languages: Arc<Vec<LanguageSet>>,
    data: Arc<GlyphData>,
    named: Named,
}

impl LanguageIndex {
    /// Fails like [`Catalog::with_languages`] on too many sets or characters.
    pub fn new(languages: Arc<Vec<LanguageSet>>, data: Arc<GlyphData>) -> Result<LanguageIndex, QueryError> {
        check_languages(&languages)?;
        let named = Named::new(&languages);
        Ok(LanguageIndex { languages, data, named })
    }

    /// Whether the index was built from these very sets and glyph data.
    pub fn is_for(&self, languages: &Arc<Vec<LanguageSet>>, data: &GlyphData) -> bool {
        Arc::ptr_eq(&self.languages, languages) && std::ptr::eq(Arc::as_ptr(&self.data), data)
    }
}

/// The characters of the language sets, each code point once. An expected glyph is named by its
/// position in `cps`.
struct Named {
    cps: Vec<u32>,
    names: Vec<OnceLock<String>>,
    class: Vec<OnceLock<DataClass>>,
    /// Each set's characters, each once, in the order of the sets.
    sets: Vec<Vec<usize>>,
    /// Each script's characters over all its sets, each once, in the order of the sets.
    scripts: HashMap<String, Vec<usize>>,
}

impl Named {
    fn new(languages: &[LanguageSet]) -> Named {
        let mut ids: HashMap<u32, usize> = HashMap::new();
        let mut cps = Vec::new();
        let mut sets = Vec::with_capacity(languages.len());
        let mut scripts: HashMap<String, (HashSet<usize>, Vec<usize>)> = HashMap::new();
        for set in languages {
            let mut seen = HashSet::new();
            let mut list = Vec::new();
            for &cp in &set.characters {
                let id = *ids.entry(cp).or_insert_with(|| {
                    cps.push(cp);
                    cps.len() - 1
                });
                if seen.insert(id) {
                    list.push(id);
                }
            }
            let (seen, all) = scripts.entry(set.script.clone()).or_default();
            all.extend(list.iter().filter(|&&id| seen.insert(id)));
            sets.push(list);
        }
        Named {
            names: cps.iter().map(|_| OnceLock::new()).collect(),
            class: cps.iter().map(|_| OnceLock::new()).collect(),
            cps,
            sets,
            scripts: scripts.into_iter().map(|(k, (_, v))| (k, v)).collect(),
        }
    }

    fn cp(&self, id: usize) -> Option<u32> {
        self.cps.get(id).copied()
    }

    fn name(&self, id: usize, data: &GlyphData) -> &str {
        match (self.names.get(id), self.cp(id)) {
            (Some(cell), Some(cp)) => cell.get_or_init(|| data.nice_name(cp)),
            _ => "",
        }
    }

    fn class(&self, id: usize, data: &GlyphData) -> Option<&DataClass> {
        let (cell, cp) = (self.class.get(id)?, self.cp(id)?);
        Some(cell.get_or_init(|| {
            let p = data_props(self.name(id, data), &[cp], data, false);
            (p.category.value, p.sub_category.value, p.case.value)
        }))
    }

    fn expected(&self, id: usize, data: &GlyphData) -> Expected {
        Expected { name: self.name(id, data).to_string(), code_point: self.cp(id) }
    }
}

/// What a catalog's font has of one expected character: whether a glyph has its name or code
/// point, and the glyphs that do.
struct Hit {
    present: bool,
    glyphs: Vec<usize>,
}

fn check_languages(languages: &[LanguageSet]) -> Result<(), QueryError> {
    if languages.len() > MAX_LANGUAGES {
        return fail(format!("at most {MAX_LANGUAGES} language entries are allowed"));
    }
    if let Some(l) = languages.iter().find(|l| l.characters.len() > MAX_LANGUAGE_CHARACTERS) {
        return fail(format!("the language entry `{}` needs more than {MAX_LANGUAGE_CHARACTERS} characters", short(&l.id)));
    }
    Ok(())
}

/// A font's glyphs, ready to be sorted into sidebar entries. Each glyph's effective properties
/// are computed at most once for everything asked of one catalog.
pub struct Catalog<'a> {
    env: Env<'a>,
    subjects: Vec<Subject<'a>>,
    filters: &'a [Filter],
    languages: &'a [LanguageSet],
    /// The languages named through the glyph data: shared, or built when first needed.
    index: Option<&'a LanguageIndex>,
    own_index: OnceCell<Named>,
    /// By character of the index, when first asked for.
    hits: OnceCell<Vec<OnceCell<Hit>>>,
    groups: Option<&'a Groups>,
    visible_scripts: Vec<String>,
    /// The positions in `font.glyphs` of the glyphs with each code point. With the font's index
    /// by name, a language entry is counted from its own characters rather than by a pass over
    /// every glyph.
    by_code: HashMap<u32, Vec<usize>>,
}

/// The labels of a category's children for a glyph: its case group, then its subcategory.
fn child_labels<'s>(sub: Option<&'s str>, case: Option<&str>) -> impl Iterator<Item = &'s str> {
    let case_label: Option<&'static str> = CASE_GROUPS.iter().find(|(_, c)| Some(*c) == case).map(|(l, _)| *l);
    case_label.into_iter().chain(sub.filter(|s| Some(*s) != case_label))
}

impl<'a> Catalog<'a> {
    pub fn new(env: Env<'a>) -> Self {
        let font = env.font;
        let subjects: Vec<Subject<'a>> =
            glyph_order(font).into_iter().filter_map(|i| font.glyphs.as_arcs().get(i)).map(|g| Subject::new(g.as_ref())).collect();
        let mut by_code: HashMap<u32, Vec<usize>> = HashMap::with_capacity(subjects.len());
        for (i, g) in font.glyphs.iter().enumerate() {
            for &c in &g.unicodes {
                by_code.entry(c).or_default().push(i);
            }
        }
        Catalog {
            env,
            subjects,
            filters: &[],
            languages: &[],
            index: None,
            own_index: OnceCell::new(),
            hits: OnceCell::new(),
            groups: None,
            visible_scripts: DEFAULT_VISIBLE_SCRIPTS.iter().map(|s| (*s).to_string()).collect(),
            by_code,
        }
    }

    pub fn with_filters(mut self, filters: &'a [Filter]) -> Self {
        self.filters = filters;
        self
    }

    pub fn with_languages(mut self, languages: &'a [LanguageSet]) -> Result<Self, QueryError> {
        check_languages(languages)?;
        self.languages = languages;
        self.index = None;
        self.own_index = OnceCell::new();
        self.hits = OnceCell::new();
        Ok(self)
    }

    /// The index's language sets, named as the index names them. An index built from other
    /// glyph data than the catalog's gives only its sets; their names are worked out again.
    pub fn with_language_index(mut self, index: &'a LanguageIndex) -> Self {
        self.languages = index.languages.as_slice();
        self.own_index = OnceCell::new();
        self.hits = OnceCell::new();
        self.index = std::ptr::eq(Arc::as_ptr(&index.data), self.env.data).then_some(index);
        self
    }

    fn named(&self) -> &Named {
        match self.index {
            Some(index) => &index.named,
            None => self.own_index.get_or_init(|| Named::new(self.languages)),
        }
    }

    /// What the font has of the index's character `id`.
    fn hit(&self, id: usize) -> Option<&Hit> {
        let named = self.named();
        let cell = self.hits.get_or_init(|| named.cps.iter().map(|_| OnceCell::new()).collect()).get(id)?;
        Some(cell.get_or_init(|| {
            let name = named.name(id, self.env.data);
            let mut glyphs: Vec<usize> = named.cp(id).and_then(|cp| self.by_code.get(&cp)).cloned().unwrap_or_default();
            if let Some(i) = self.env.font.glyphs.position(name)
                && !glyphs.contains(&i)
            {
                glyphs.push(i);
            }
            let present = !glyphs.is_empty();
            Hit { present, glyphs }
        }))
    }

    /// How many glyphs one of the index's characters `ids` counts.
    fn count_ids(&self, ids: &[usize]) -> usize {
        let mut hit: Vec<usize> = ids.iter().filter_map(|&id| self.hit(id)).flat_map(|h| h.glyphs.iter().copied()).collect();
        hit.sort_unstable();
        hit.dedup();
        hit.len()
    }

    /// Sets the badge numbers of an entry expecting the index's characters `ids`.
    fn badge_ids(&self, item: &mut SidebarItem, ids: &[usize]) {
        if !ids.is_empty() {
            item.expected = Some(ids.len());
            item.present = Some(ids.iter().filter(|&&id| self.hit(id).is_some_and(|h| h.present)).count());
        }
    }

    pub fn with_groups(mut self, groups: &'a Groups) -> Self {
        self.groups = Some(groups);
        self
    }

    pub fn with_visible_scripts(mut self, scripts: &[String]) -> Self {
        self.visible_scripts = scripts.to_vec();
        self
    }

    pub fn env(&self) -> &Env<'a> {
        &self.env
    }

    /// The scripts the Languages section lists: the chosen ones, then the others the font has
    /// glyphs for, alphabetically.
    pub fn shown_scripts(&self) -> Vec<String> {
        let mut out = self.visible_scripts.clone();
        let mut more: Vec<String> = Vec::new();
        for s in &self.subjects {
            if let Some(script) = s.props(&self.env).script.as_deref()
                && !out.iter().any(|o| o == script)
                && !more.iter().any(|o| o == script)
            {
                more.push(script.to_string());
            }
        }
        more.sort();
        out.extend(more);
        out
    }

    fn present(&self, e: &Expected) -> bool {
        self.env.font.glyph(&e.name).is_some() || e.code_point.is_some_and(|c| self.by_code.contains_key(&c))
    }

    fn count(&self, m: &Member) -> usize {
        if let Member::Codes { codes, names } = m {
            return self.count_codes(codes.iter().copied(), names.iter().map(String::as_str));
        }
        self.subjects.iter().filter(|s| m.contains(s, &self.env)).count()
    }

    /// How many glyphs have one of `codes` or one of `names`.
    fn count_codes<'n>(&self, codes: impl Iterator<Item = u32>, names: impl Iterator<Item = &'n str>) -> usize {
        let mut hit: HashSet<usize> = HashSet::new();
        for c in codes {
            if let Some(list) = self.by_code.get(&c) {
                hit.extend(list.iter().copied());
            }
        }
        hit.extend(names.filter_map(|n| self.env.font.glyphs.position(n)));
        hit.len()
    }

    /// The glyphs an entry expects that the font lacks; `None` when the entry does not know its
    /// full set.
    pub fn missing(&self, entry: &Entry) -> Option<Vec<Expected>> {
        entry.expected.as_ref().map(|v| v.iter().filter(|e| !self.present(e)).cloned().collect())
    }

    /// The names of the glyphs an entry contains, in glyph order.
    pub fn members(&self, entry: &Entry) -> Vec<String> {
        self.subjects.iter().filter(|s| entry.member.contains(s, &self.env)).map(|s| s.glyph.name.clone()).collect()
    }

    // ── Resolving paths ──

    pub fn resolve(&self, path: &[String]) -> Result<Entry, QueryError> {
        let Some((section, rest)) = path.split_first() else { return fail("the sidebar path is empty") };
        let entry = |section, member, expected| Entry { path: path.to_vec(), section, member, expected };
        match section.as_str() {
            "all" if rest.is_empty() => Ok(entry(Section::All, Member::Everything, None)),
            "categories" => {
                let (member, expected) = self.category(rest)?;
                Ok(entry(Section::Categories, member, expected))
            }
            "languages" => {
                let (member, expected) = self.language(rest)?;
                Ok(entry(Section::Languages, member, expected))
            }
            "filters" => {
                let f = filter_at(self.filters, rest)?;
                let expected = match f {
                    Filter::List { glyphs, .. } => Some(glyphs.iter().map(|n| Expected { name: n.clone(), code_point: None }).collect()),
                    _ => None,
                };
                Ok(entry(Section::Filters, filter_member(f)?, expected))
            }
            _ => fail(format!("`{}`: a sidebar path starts with all, categories, languages or filters", short(&path.join("/")))),
        }
    }

    fn category(&self, rest: &[String]) -> Result<(Member, Option<Vec<Expected>>), QueryError> {
        let (cat, child) = match rest {
            [cat] => (cat.as_str(), None),
            [cat, child] => (cat.as_str(), Some(child.as_str())),
            _ => ("", None),
        };
        if Category::parse(cat).is_some() && rest.len() <= 2 {
            if let Some(child) = child
                && SubCategory::parse(child).is_none()
                && !CASE_GROUPS.iter().any(|(l, _)| *l == child)
            {
                return fail(format!("`{}` is not a subcategory or case group of `{cat}`", short(child)));
            }
            let value = match child {
                Some(c) => format!("{cat}/{c}"),
                None => cat.to_string(),
            };
            return Ok((Member::Pred(category_expr(&value)?), self.category_expected(cat, child)));
        }
        match self.groups {
            Some(g) => group_lookup(&g.categories, rest),
            None => fail(format!("there is no category `{}`", short(&rest.join("/")))),
        }
    }

    fn language(&self, rest: &[String]) -> Result<(Member, Option<Vec<Expected>>), QueryError> {
        if let Some(g) = self.groups
            && rest.first().is_some_and(|n| g.languages.iter().any(|e| &e.name == n))
        {
            return group_lookup(&g.languages, rest);
        }
        match rest {
            [script] => {
                let script = check_script_id(script)?;
                let member = Member::Pred(Expr::Test(Test::new(Attr::Script, Op::Eq, Literal::Str(script.clone()))?));
                Ok((member, self.script_expected(&script)))
            }
            [script, id] => {
                let found = self.languages.iter().enumerate().find(|(_, l)| &l.script == script && &l.id == id);
                let Some((i, set)) = found else {
                    return fail(format!("there is no language `{}` under `{}`", short(id), short(script)));
                };
                let named = self.named();
                let expected: Vec<Expected> =
                    named.sets.get(i).map(|ids| ids.iter().map(|&id| named.expected(id, self.env.data)).collect()).unwrap_or_default();
                let codes = set.characters.iter().copied().collect();
                let names = expected.iter().map(|e| e.name.clone()).collect();
                Ok((Member::Codes { codes, names }, Some(expected)))
            }
            _ => fail(format!("there is no language entry `{}`", short(&rest.join("/")))),
        }
    }

    fn script_expected(&self, script: &str) -> Option<Vec<Expected>> {
        let named = self.named();
        named.scripts.get(script).map(|ids| ids.iter().map(|&id| named.expected(id, self.env.data)).collect())
    }

    /// Every character the languages of the listed scripts need, each once, as the index's
    /// characters, with the category, subcategory and case the glyph data gives its glyph.
    fn expected_pool(&self) -> Vec<(usize, &DataClass)> {
        let shown = self.shown_scripts();
        let named = self.named();
        let mut seen = vec![false; named.cps.len()];
        let mut out = Vec::new();
        for (set, ids) in self.languages.iter().zip(&named.sets) {
            if !shown.contains(&set.script) {
                continue;
            }
            for &id in ids {
                if let Some(s) = seen.get_mut(id)
                    && !*s
                {
                    *s = true;
                    if let Some(class) = named.class(id, self.env.data) {
                        out.push((id, class));
                    }
                }
            }
        }
        out
    }

    /// A category entry's expected glyphs: those the languages of the listed scripts need that
    /// belong to it. `None` without language data.
    fn category_expected(&self, cat: &str, child: Option<&str>) -> Option<Vec<Expected>> {
        let named = self.named();
        let out: Vec<Expected> = self
            .expected_pool()
            .into_iter()
            .filter(|(_, (c, sub, case))| {
                c.as_deref() == Some(cat) && child.is_none_or(|ch| child_labels(sub.as_deref(), case.as_deref()).any(|l| l == ch))
            })
            .map(|(id, _)| named.expected(id, self.env.data))
            .collect();
        if out.is_empty() { None } else { Some(out) }
    }

    // ── Selecting ──

    /// The names of the glyphs a selection shows, in glyph order, or in list order when a list
    /// filter is selected.
    pub fn select(&self, selection: &Selection) -> Result<Vec<String>, QueryError> {
        if selection.entries.len() > MAX_ENTRIES {
            return fail(format!("at most {MAX_ENTRIES} sidebar entries can be selected at once"));
        }
        let (mut categories, mut languages, mut filters) = (Vec::new(), Vec::new(), Vec::new());
        let mut order: Option<Vec<String>> = None;
        for path in &selection.entries {
            let entry = self.resolve(path)?;
            match entry.section {
                Section::All => {}
                Section::Categories => categories.push(entry.member),
                Section::Languages => languages.push(entry.member),
                Section::Filters => {
                    if order.is_none()
                        && let Member::Names { order: list, .. } = &entry.member
                    {
                        order = Some(list.clone());
                    }
                    filters.push(entry.member);
                }
            }
        }
        let mut parts = Vec::new();
        if !categories.is_empty() {
            parts.push(Member::Union(categories));
        }
        if !languages.is_empty() {
            parts.push(Member::Union(languages));
        }
        if !filters.is_empty() {
            parts.push(Member::Intersect(filters));
        }
        if let Some(rules) = &selection.rules {
            parts.push(Member::Pred(rules.clone()));
        }
        let all = Member::Intersect(parts);
        let shown: Vec<&Subject> =
            self.subjects.iter().filter(|s| all.contains(s, &self.env) && selection.search.as_ref().is_none_or(|m| m.matches(s.glyph))).collect();
        Ok(match order {
            Some(list) => {
                let names: HashSet<&str> = shown.iter().map(|s| s.glyph.name.as_str()).collect();
                list.into_iter().filter(|n| names.contains(n.as_str())).collect()
            }
            None => shown.iter().map(|s| s.glyph.name.clone()).collect(),
        })
    }

    // ── Listing ──

    /// Every sidebar entry with its count and badge numbers: All, then the Categories, Languages
    /// and Filters sections.
    pub fn sidebar(&self) -> Vec<SidebarItem> {
        let item = |path: Vec<String>, label: String, kind: &'static str, count: usize| SidebarItem {
            path,
            label,
            kind,
            count,
            expected: None,
            present: None,
            children: Vec::new(),
            problem: None,
        };
        let total = self.subjects.len();
        let mut categories = item(vec!["categories".into()], "Categories".into(), "section", total);
        categories.children = self.category_items();
        if let Some(g) = self.groups {
            categories.children.extend(g.categories.iter().map(|e| self.group_item(e, vec!["categories".into()])));
        }
        let mut languages = item(vec!["languages".into()], "Languages".into(), "section", total);
        languages.children = self.script_items();
        if let Some(g) = self.groups {
            languages.children.extend(g.languages.iter().map(|e| self.group_item(e, vec!["languages".into()])));
        }
        let mut filters = item(vec!["filters".into()], "Filters".into(), "section", total);
        filters.children = self.filters.iter().map(|f| self.filter_item(f, vec!["filters".into()])).collect();
        vec![item(vec!["all".into()], "All".into(), "all", total), categories, languages, filters]
    }

    fn badge<'e>(&self, item: &mut SidebarItem, expected: impl IntoIterator<Item = &'e Expected>) {
        let (mut all, mut present) = (0, 0);
        for e in expected {
            all += 1;
            if self.present(e) {
                present += 1;
            }
        }
        if all > 0 {
            item.expected = Some(all);
            item.present = Some(present);
        }
    }

    fn category_items(&self) -> Vec<SidebarItem> {
        // A font has a handful of categories and labels: short lists beat hashing every glyph's.
        let mut counts: Vec<CategoryCount> = Vec::new();
        for s in &self.subjects {
            let p = s.props(&self.env);
            let Some(cat) = p.category.as_deref() else { continue };
            let at = match counts.iter().position(|(c, ..)| *c == cat) {
                Some(at) => at,
                None => {
                    counts.push((cat, 0, Vec::new()));
                    counts.len() - 1
                }
            };
            let Some((_, n, children)) = counts.get_mut(at) else { continue };
            *n += 1;
            for label in child_labels(p.sub_category.as_deref(), p.case.as_deref()) {
                match children.iter_mut().find(|(l, _)| *l == label) {
                    Some((_, n)) => *n += 1,
                    None => children.push((label, 1)),
                }
            }
        }
        let count_of = |cat: &str, label: Option<&str>| -> usize {
            let Some((_, n, children)) = counts.iter().find(|(c, ..)| *c == cat) else { return 0 };
            match label {
                None => *n,
                Some(label) => children.iter().find(|(l, _)| *l == label).map_or(0, |(_, n)| *n),
            }
        };
        let pool = self.expected_pool();
        let mut expected: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut child_expected: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
        for (id, (cat, sub, case)) in pool {
            let Some(cat) = cat.as_deref() else { continue };
            expected.entry(cat).or_default().push(id);
            for label in child_labels(sub.as_deref(), case.as_deref()) {
                child_expected.entry((cat, label)).or_default().push(id);
            }
        }
        let labels: Vec<&str> = CASE_GROUPS
            .iter()
            .map(|(l, _)| *l)
            .chain(SubCategory::ALL.iter().map(|s| s.as_str()).filter(|s| !CASE_GROUPS.iter().any(|(l, _)| l == s)))
            .collect();
        let mut out = Vec::new();
        for cat in Category::ALL.iter().map(|c| c.as_str()) {
            let n = count_of(cat, None);
            let exp = expected.get(cat);
            if n == 0 && exp.is_none() {
                continue;
            }
            let mut it = SidebarItem {
                path: vec!["categories".into(), cat.into()],
                label: cat.into(),
                kind: "category",
                count: n,
                expected: None,
                present: None,
                children: Vec::new(),
                problem: None,
            };
            self.badge_ids(&mut it, exp.map_or(&[], Vec::as_slice));
            for label in &labels {
                let key = (cat, *label);
                let n = count_of(cat, Some(*label));
                let exp = child_expected.get(&key);
                if n == 0 && exp.is_none() {
                    continue;
                }
                let kind = if CASE_GROUPS.iter().any(|(l, _)| l == label) { "case" } else { "subcategory" };
                let mut child = SidebarItem {
                    path: vec!["categories".into(), cat.into(), (*label).into()],
                    label: (*label).into(),
                    kind,
                    count: n,
                    expected: None,
                    present: None,
                    children: Vec::new(),
                    problem: None,
                };
                self.badge_ids(&mut child, exp.map_or(&[], Vec::as_slice));
                it.children.push(child);
            }
            out.push(it);
        }
        out
    }

    fn script_items(&self) -> Vec<SidebarItem> {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for s in &self.subjects {
            if let Some(script) = s.props(&self.env).script.as_deref() {
                *count.entry(script).or_default() += 1;
            }
        }
        self.shown_scripts()
            .into_iter()
            .map(|script| {
                let mut it = SidebarItem {
                    path: vec!["languages".into(), script.clone()],
                    label: self.env.data.script_display_name(&script),
                    kind: "script",
                    count: count.get(script.as_str()).copied().unwrap_or(0),
                    expected: None,
                    present: None,
                    children: Vec::new(),
                    problem: None,
                };
                let named = self.named();
                self.badge_ids(&mut it, named.scripts.get(&script).map_or(&[], Vec::as_slice));
                for (set, ids) in self.languages.iter().zip(&named.sets).filter(|(l, _)| l.script == script) {
                    let mut child = SidebarItem {
                        path: vec!["languages".into(), script.clone(), set.id.clone()],
                        label: set.name.clone(),
                        kind: "language",
                        count: self.count_ids(ids),
                        expected: None,
                        present: None,
                        children: Vec::new(),
                        problem: None,
                    };
                    self.badge_ids(&mut child, ids);
                    it.children.push(child);
                }
                it
            })
            .collect()
    }

    fn group_item(&self, e: &GroupEntry, parent: Vec<String>) -> SidebarItem {
        let mut path = parent;
        path.push(e.name.clone());
        let (member, expected) = group_member(e);
        let mut it = SidebarItem {
            path: path.clone(),
            label: e.name.clone(),
            kind: "group",
            count: self.count(&member),
            expected: None,
            present: None,
            children: Vec::new(),
            problem: None,
        };
        self.badge(&mut it, expected.iter().flatten());
        it.children = e.children.iter().map(|c| self.group_item(c, path.clone())).collect();
        it
    }

    fn filter_item(&self, f: &Filter, parent: Vec<String>) -> SidebarItem {
        let mut path = parent;
        path.push(f.name().to_string());
        let mut it = SidebarItem {
            path: path.clone(),
            label: f.name().to_string(),
            kind: f.kind(),
            count: 0,
            expected: None,
            present: None,
            children: Vec::new(),
            problem: None,
        };
        match filter_member(f) {
            Ok(m) => it.count = self.count(&m),
            Err(e) => it.problem = Some(e.0),
        }
        match f {
            Filter::List { glyphs, .. } => {
                let expected: Vec<Expected> = glyphs.iter().map(|n| Expected { name: n.clone(), code_point: None }).collect();
                self.badge(&mut it, &expected);
            }
            Filter::Folder { children, .. } => it.children = children.iter().map(|c| self.filter_item(c, path.clone())).collect(),
            Filter::Smart { .. } => {}
        }
        it
    }
}

fn filter_member(f: &Filter) -> Result<Member, QueryError> {
    Ok(match f {
        Filter::Smart { rules, .. } => Member::Pred(rules.compile()?),
        Filter::List { glyphs, .. } => Member::names(glyphs),
        Filter::Folder { children, .. } => Member::Union(children.iter().map(filter_member).collect::<Result<_, _>>()?),
    })
}

fn group_member(e: &GroupEntry) -> (Member, Option<Vec<Expected>>) {
    match &e.rule {
        GroupRule::Script(s) => match Test::new(Attr::Script, Op::Eq, Literal::Str(s.clone())) {
            Ok(t) => (Member::Pred(Expr::Test(t)), None),
            Err(_) => (Member::Pred(Expr::Any(Vec::new())), None),
        },
        GroupRule::Predicate(p) => (Member::Pred(p.clone()), None),
        GroupRule::List(names) => (Member::names(names), Some(names.iter().map(|n| Expected { name: n.clone(), code_point: None }).collect())),
        GroupRule::None => (Member::Union(e.children.iter().map(|c| group_member(c).0).collect()), None),
    }
}

fn group_lookup(entries: &[GroupEntry], path: &[String]) -> Result<(Member, Option<Vec<Expected>>), QueryError> {
    let mut level = entries;
    let mut found = None;
    for segment in path {
        let Some(e) = level.iter().find(|e| &e.name == segment) else {
            return fail(format!("there is no sidebar entry `{}`", short(&path.join("/"))));
        };
        level = &e.children;
        found = Some(e);
    }
    match found {
        Some(e) => Ok(group_member(e)),
        None => fail("the sidebar path is empty"),
    }
}
