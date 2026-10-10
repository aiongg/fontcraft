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

use std::collections::{HashMap, HashSet};

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

/// A font's glyphs, ready to be sorted into sidebar entries. Each glyph's effective properties
/// are computed at most once for everything asked of one catalog.
pub struct Catalog<'a> {
    env: Env<'a>,
    subjects: Vec<Subject<'a>>,
    filters: &'a [Filter],
    languages: &'a [LanguageSet],
    groups: Option<&'a Groups>,
    visible_scripts: Vec<String>,
    codes: HashSet<u32>,
}

/// A label of a category's children for a glyph: its case group, then its subcategory.
fn child_labels(sub: Option<&str>, case: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((label, _)) = CASE_GROUPS.iter().find(|(_, c)| Some(*c) == case) {
        out.push((*label).to_string());
    }
    if let Some(sub) = sub
        && !out.iter().any(|l| l == sub)
    {
        out.push(sub.to_string());
    }
    out
}

impl<'a> Catalog<'a> {
    pub fn new(env: Env<'a>) -> Self {
        let font = env.font;
        let subjects: Vec<Subject<'a>> =
            glyph_order(font).into_iter().filter_map(|i| font.glyphs.as_arcs().get(i)).map(|g| Subject::new(g.as_ref())).collect();
        let codes = font.glyphs.iter().flat_map(|g| g.unicodes.iter().copied()).collect();
        Catalog {
            env,
            subjects,
            filters: &[],
            languages: &[],
            groups: None,
            visible_scripts: DEFAULT_VISIBLE_SCRIPTS.iter().map(|s| (*s).to_string()).collect(),
            codes,
        }
    }

    pub fn with_filters(mut self, filters: &'a [Filter]) -> Self {
        self.filters = filters;
        self
    }

    pub fn with_languages(mut self, languages: &'a [LanguageSet]) -> Result<Self, QueryError> {
        if languages.len() > MAX_LANGUAGES {
            return fail(format!("at most {MAX_LANGUAGES} language entries are allowed"));
        }
        if let Some(l) = languages.iter().find(|l| l.characters.len() > MAX_LANGUAGE_CHARACTERS) {
            return fail(format!("the language entry `{}` needs more than {MAX_LANGUAGE_CHARACTERS} characters", short(&l.id)));
        }
        self.languages = languages;
        Ok(self)
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
        self.env.font.glyph(&e.name).is_some() || e.code_point.is_some_and(|c| self.codes.contains(&c))
    }

    fn count(&self, m: &Member) -> usize {
        self.subjects.iter().filter(|s| m.contains(s, &self.env)).count()
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
                let Some(set) = self.languages.iter().find(|l| &l.script == script && &l.id == id) else {
                    return fail(format!("there is no language `{}` under `{}`", short(id), short(script)));
                };
                let expected = self.language_expected(&[set]);
                let codes = set.characters.iter().copied().collect();
                let names = expected.iter().map(|e| e.name.clone()).collect();
                Ok((Member::Codes { codes, names }, Some(expected)))
            }
            _ => fail(format!("there is no language entry `{}`", short(&rest.join("/")))),
        }
    }

    /// The characters of `sets` as glyphs, each code point once.
    fn language_expected(&self, sets: &[&LanguageSet]) -> Vec<Expected> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for set in sets {
            for &cp in &set.characters {
                if seen.insert(cp) {
                    out.push(Expected { name: self.env.data.nice_name(cp), code_point: Some(cp) });
                }
            }
        }
        out
    }

    fn script_expected(&self, script: &str) -> Option<Vec<Expected>> {
        let sets: Vec<&LanguageSet> = self.languages.iter().filter(|l| l.script == script).collect();
        if sets.is_empty() { None } else { Some(self.language_expected(&sets)) }
    }

    /// Every character the languages of the listed scripts need, with the category, subcategory
    /// and case the glyph data gives its glyph.
    fn expected_pool(&self) -> Vec<(Expected, Option<String>, Option<String>, Option<String>)> {
        let shown = self.shown_scripts();
        let sets: Vec<&LanguageSet> = self.languages.iter().filter(|l| shown.contains(&l.script)).collect();
        self.language_expected(&sets)
            .into_iter()
            .map(|e| {
                let p = data_props(&e.name, &e.code_point.into_iter().collect::<Vec<_>>(), self.env.data, false);
                (e, p.category.value, p.sub_category.value, p.case.value)
            })
            .collect()
    }

    /// A category entry's expected glyphs: those the languages of the listed scripts need that
    /// belong to it. `None` without language data.
    fn category_expected(&self, cat: &str, child: Option<&str>) -> Option<Vec<Expected>> {
        let out: Vec<Expected> = self
            .expected_pool()
            .into_iter()
            .filter(|(_, c, sub, case)| {
                c.as_deref() == Some(cat) && child.is_none_or(|ch| child_labels(sub.as_deref(), case.as_deref()).iter().any(|l| l == ch))
            })
            .map(|(e, ..)| e)
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

    fn badge(&self, item: &mut SidebarItem, expected: Option<&[Expected]>) {
        if let Some(list) = expected.filter(|l| !l.is_empty()) {
            item.expected = Some(list.len());
            item.present = Some(list.iter().filter(|e| self.present(e)).count());
        }
    }

    fn category_items(&self) -> Vec<SidebarItem> {
        let mut count: HashMap<String, usize> = HashMap::new();
        let mut child_count: HashMap<(String, String), usize> = HashMap::new();
        for s in &self.subjects {
            let p = s.props(&self.env);
            let Some(cat) = p.category.as_deref() else { continue };
            *count.entry(cat.to_string()).or_default() += 1;
            for label in child_labels(p.sub_category.as_deref(), p.case.as_deref()) {
                *child_count.entry((cat.to_string(), label)).or_default() += 1;
            }
        }
        let pool = self.expected_pool();
        let mut expected: HashMap<String, Vec<Expected>> = HashMap::new();
        let mut child_expected: HashMap<(String, String), Vec<Expected>> = HashMap::new();
        for (e, cat, sub, case) in &pool {
            let Some(cat) = cat else { continue };
            expected.entry(cat.clone()).or_default().push(e.clone());
            for label in child_labels(sub.as_deref(), case.as_deref()) {
                child_expected.entry((cat.clone(), label)).or_default().push(e.clone());
            }
        }
        let labels: Vec<&str> = CASE_GROUPS
            .iter()
            .map(|(l, _)| *l)
            .chain(SubCategory::ALL.iter().map(|s| s.as_str()).filter(|s| !CASE_GROUPS.iter().any(|(l, _)| l == s)))
            .collect();
        let mut out = Vec::new();
        for cat in Category::ALL.iter().map(|c| c.as_str()) {
            let n = count.get(cat).copied().unwrap_or(0);
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
            self.badge(&mut it, exp.map(Vec::as_slice));
            for label in &labels {
                let key = (cat.to_string(), (*label).to_string());
                let n = child_count.get(&key).copied().unwrap_or(0);
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
                self.badge(&mut child, exp.map(Vec::as_slice));
                it.children.push(child);
            }
            out.push(it);
        }
        out
    }

    fn script_items(&self) -> Vec<SidebarItem> {
        let mut count: HashMap<String, usize> = HashMap::new();
        for s in &self.subjects {
            if let Some(script) = s.props(&self.env).script.as_deref() {
                *count.entry(script.to_string()).or_default() += 1;
            }
        }
        self.shown_scripts()
            .into_iter()
            .map(|script| {
                let mut it = SidebarItem {
                    path: vec!["languages".into(), script.clone()],
                    label: script.clone(),
                    kind: "script",
                    count: count.get(&script).copied().unwrap_or(0),
                    expected: None,
                    present: None,
                    children: Vec::new(),
                    problem: None,
                };
                self.badge(&mut it, self.script_expected(&script).as_deref());
                for set in self.languages.iter().filter(|l| l.script == script) {
                    let expected = self.language_expected(&[set]);
                    let member =
                        Member::Codes { codes: set.characters.iter().copied().collect(), names: expected.iter().map(|e| e.name.clone()).collect() };
                    let mut child = SidebarItem {
                        path: vec!["languages".into(), script.clone(), set.id.clone()],
                        label: set.name.clone(),
                        kind: "language",
                        count: self.count(&member),
                        expected: None,
                        present: None,
                        children: Vec::new(),
                        problem: None,
                    };
                    self.badge(&mut child, Some(expected.as_slice()));
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
        self.badge(&mut it, expected.as_deref());
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
                self.badge(&mut it, Some(expected.as_slice()));
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
