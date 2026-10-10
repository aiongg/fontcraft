//! Font View's filtering, headless (`docs/spec/05-font-view.md` § 6): which glyphs the sidebar
//! entries, smart-filter rules and search show (`fontView.queryGlyphs`), the Filters section and
//! the Languages list (kept in the user settings, so they change no document and make no undo
//! step), and the glyphs a sidebar entry expects that the font lacks.
//!
//! The evaluation is `fontcraft_glyphquery`'s. It is given the built-in language sets (CLDR
//! exemplar characters grouped by script, [`crate::glyph_data::language_sets`]) and the entries of
//! the user's `Groups*.plist` files ([`crate::Session::user_groups`]).

use std::sync::Arc;

use fontcraft_glyphquery::{self as gq, Catalog, Env, Expected, Filter, Groups, LanguageIndex, Matcher, RuleTree, Search, Selection};
use fontcraft_model::{Glyph, Layer, MAX_GLYPHS, data_props, insert_by_sort_key};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;
use crate::settings::Reserved;

/// The advance width of a glyph made from a missing-glyph list, as `glyph.newGlyph` gives.
const NEW_GLYPH_WIDTH: f64 = 600.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "fontView.queryGlyphs",
            "Query Glyphs",
            [],
            None,
            "{entries?: [path], rules?: ruleTree, search?: {query, fields?, matchCase?, regex?}, master?: id, sidebar?: bool} the glyphs that sidebar entries, ad-hoc smart-filter rules and a search term show together. A path is text such as categories/Letter/Uppercase, languages/latin or filters/Folder/Name, or a list of names. Categories add up, scripts add up, filters narrow each other; sections, rules and search narrow each other. A rule tree is {all|any|none: [rule or group]} with rule {property, op, value}; the Custom rule takes predicate text. `search` may also be plain text; fields are name, unicode and note. Layer rules read `master` (default: the first master); the count rules always read the first master. sidebar: true adds every entry with its count and its present/expected badge → {glyphs: [names] in glyph order (list order while a list filter is selected), count, total, sidebar?}",
            has_doc,
            query_glyphs
        ),
        cmd!(
            "fontView.addSmartFilter",
            "Add Smart Filter",
            [],
            None,
            "{name: string, rules?: ruleTree, folder?: path} add a smart filter to the Filters section (in `folder`, a path inside the section such as `Work`, or the sidebar's `filters/Work`; else at the top). Without rules it has one rule every glyph matches. Kept in the user settings → {filter: path}",
            always,
            add_smart_filter
        ),
        cmd!(
            "fontView.addListFilter",
            "Add List Filter",
            [],
            None,
            "{name: string, glyphs?: [names], folder?: path} add a list filter: glyph names in the order the filter shows them (each string may hold several names separated by spaces or line breaks; names the font lacks are allowed). Kept in the user settings → {filter: path}",
            always,
            add_list_filter
        ),
        cmd!(
            "fontView.addFilterFolder",
            "Add Folder",
            [],
            None,
            "{name: string, parent?: path} add a folder to the Filters section; selecting it shows the union of what it holds → {filter: path}",
            always,
            add_filter_folder
        ),
        cmd!(
            "fontView.editFilter",
            "Edit Filter",
            [],
            None,
            "{filter: path, name?: string, rules?: ruleTree, glyphs?: [names]} change a filter: its name, a smart filter's rules or a list filter's names → {filter: path}",
            always,
            edit_filter
        ),
        cmd!(
            "fontView.renameFilter",
            "Rename Filter",
            [],
            Some("Return"),
            "{filter: path, name: string} rename a filter or folder; names are unique within their folder → {filter: path}",
            always,
            rename_filter
        )
        .when("panel:fontView.sidebar.filters"),
        cmd!(
            "fontView.removeFilter",
            "Remove Filter",
            [],
            None,
            "{filter: path} delete a filter, or a folder with everything inside it → {removed: path, kind}",
            always,
            remove_filter
        ),
        cmd!(
            "fontView.moveFilter",
            "Move Filter",
            [],
            None,
            "{filter: path, parent?: path, index: number} move a filter or folder into the folder `parent` (the top level without it), at `index` among the entries there → {filter: path}",
            always,
            move_filter
        ),
        cmd!(
            "fontView.setVisibleScripts",
            "Edit Languages List",
            [],
            None,
            "{show?: [script], hide?: [script]} choose the scripts the Languages section lists (ids such as latin); scripts the font has glyphs for are listed anyway. Kept in the user settings → {scripts: [ids]}",
            always,
            set_visible_scripts
        ),
        cmd!(
            query "fontView.listMissingGlyphs",
            "Show Missing Glyphs",
            [],
            None,
            "{entry: path} the glyphs a sidebar entry expects that the font lacks, for entries that know their full set (list filters, scripts and languages with language data) → {entry: path, missing: [names], present, expected}",
            has_doc,
            list_missing_glyphs
        ),
        cmd!(
            "fontView.generateMissingGlyphs",
            "Generate",
            [],
            None,
            "{entry: path, glyphs?: [names]} add the missing glyphs of a sidebar entry (all of them, or the ones named), placed by the default sort order, in one undo step → {added: [names]}",
            has_doc,
            generate_missing_glyphs
        ),
    ]
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

fn bad(p: &Params, key: &str, e: gq::QueryError) -> EngineError {
    p.bad(format!("`{key}`: {e}"))
}

fn opt_path(p: &Params, key: &str) -> Result<Option<Vec<String>>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => gq::parse_path(v).map(Some).map_err(|e| bad(p, key, e)),
    }
}

fn path(p: &Params, key: &str) -> Result<Vec<String>> {
    opt_path(p, key)?.ok_or_else(|| p.bad(format!("`{key}` is required")))
}

/// The section's name at the head of a sidebar path (`filters/Folder/Name`).
const FILTERS_SECTION: &str = "filters";

/// A path inside the Filters section. The sidebar's form, which names the section first, is
/// taken too, so the paths `fontView.state` lists work here unchanged.
fn opt_filter_path(p: &Params, key: &str) -> Result<Option<Vec<String>>> {
    Ok(opt_path(p, key)?.map(|mut path| {
        if path.len() > 1 && path.first().is_some_and(|s| s == FILTERS_SECTION) {
            path.remove(0);
        }
        path
    }))
}

fn filter_path(p: &Params, key: &str) -> Result<Vec<String>> {
    opt_filter_path(p, key)?.ok_or_else(|| p.bad(format!("`{key}` is required")))
}

fn opt_rules(p: &Params) -> Result<Option<RuleTree>> {
    match p.get("rules") {
        None => Ok(None),
        Some(v) => RuleTree::from_json(v).map(Some).map_err(|e| bad(p, "rules", e)),
    }
}

fn opt_search(p: &Params) -> Result<Option<Matcher>> {
    let search = match p.get("search") {
        None => return Ok(None),
        Some(Value::String(query)) => Search { query: query.clone(), ..Search::default() },
        Some(v) => serde_json::from_value::<Search>(v.clone()).map_err(|e| p.bad(format!("`search`: {e}")))?,
    };
    Matcher::new(&search).map(Some).map_err(|e| bad(p, "search", e))
}

fn filter_name(p: &Params) -> Result<String> {
    gq::check_filter_name(p.str("name")?).map_err(|e| bad(p, "name", e))
}

fn opt_list_names(p: &Params) -> Result<Option<Vec<String>>> {
    match p.opt_strings("glyphs")? {
        None => Ok(None),
        Some(texts) => gq::list_names(texts).map(Some).map_err(|e| bad(p, "glyphs", e)),
    }
}

// ── The stored filters and scripts ──────────────────────────────────────────────────────────────

fn stored_filters(s: &Session) -> Result<Vec<Filter>> {
    Ok(s.settings().reserved::<Vec<Filter>>(Reserved::Filters)?.unwrap_or_default())
}

/// Checks the whole tree and stores it; a refused tree changes nothing.
fn store_filters(s: &mut Session, p: &Params, items: &[Filter]) -> Result<()> {
    gq::validate_filters(items).map_err(|e| p.bad(e.to_string()))?;
    s.settings_mut().set_reserved(Reserved::Filters, items)?;
    Ok(())
}

fn visible_scripts(s: &Session) -> Result<Vec<String>> {
    Ok(s.settings()
        .reserved::<Vec<String>>(Reserved::VisibleScripts)?
        .unwrap_or_else(|| gq::DEFAULT_VISIBLE_SCRIPTS.iter().map(|x| (*x).to_string()).collect()))
}

/// What a catalog is built from besides the font: the stored filters, the listed scripts, the
/// language sets and the user's groups.
struct Sources {
    filters: Vec<Filter>,
    scripts: Vec<String>,
    languages: Arc<LanguageIndex>,
    groups: Option<Arc<Groups>>,
}

impl Sources {
    fn read(s: &mut Session, with_filters: bool) -> Result<Sources> {
        Ok(Sources {
            filters: if with_filters { stored_filters(s)? } else { Vec::new() },
            scripts: visible_scripts(s)?,
            languages: s.language_index()?,
            groups: s.user_groups(),
        })
    }

    fn catalog<'a>(&'a self, env: Env<'a>) -> Result<Catalog<'a>> {
        let catalog = Catalog::new(env).with_filters(&self.filters).with_visible_scripts(&self.scripts).with_language_index(&self.languages);
        Ok(match self.groups.as_deref() {
            Some(g) => catalog.with_groups(g),
            None => catalog,
        })
    }
}

// ── Queries ─────────────────────────────────────────────────────────────────────────────────────

fn query_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.queryGlyphs", v)?;
    let entries = match p.opt_array("entries")? {
        None => Vec::new(),
        Some(list) if list.len() > gq::MAX_ENTRIES => {
            return Err(p.bad(format!("`entries` holds {} paths; at most {} are allowed", list.len(), gq::MAX_ENTRIES)));
        }
        Some(list) => list.iter().map(|e| gq::parse_path(e).map_err(|e| bad(&p, "entries", e))).collect::<Result<Vec<_>>>()?,
    };
    let rules = match opt_rules(&p)? {
        Some(tree) => Some(tree.compile().map_err(|e| bad(&p, "rules", e))?),
        None => None,
    };
    let search = opt_search(&p)?;
    let master = p.opt_str("master")?;
    let with_sidebar = p.bool_or("sidebar", false)?;
    let wants_filters = with_sidebar || entries.iter().any(|e| e.first().is_some_and(|s| s == "filters"));
    let sources = Sources::read(s, wants_filters)?;
    let props = s.effective_props()?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let env = Env::new(font, &data, master).map_err(|e| bad(&p, "master", e))?.with_props(&props);
    let catalog = sources.catalog(env)?;
    let glyphs = catalog.select(&Selection { entries, rules, search }).map_err(|e| p.bad(e.to_string()))?;
    let mut out = Map::new();
    out.insert("count".into(), json!(glyphs.len()));
    out.insert("glyphs".into(), json!(glyphs));
    out.insert("total".into(), json!(font.glyphs.len()));
    if with_sidebar {
        let sidebar = serde_json::to_value(catalog.sidebar()).map_err(|e| EngineError::Internal(e.to_string()))?;
        out.insert("sidebar".into(), sidebar);
    }
    Ok(Value::Object(out))
}

/// The glyphs an entry expects that the font lacks, and how many it expects in all.
fn missing_of(s: &mut Session, p: &Params, entry: &[String]) -> Result<(Vec<Expected>, usize)> {
    let sources = Sources::read(s, entry.first().is_some_and(|s| s == "filters"))?;
    let props = s.effective_props()?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let env = Env::new(font, &data, None).map_err(|e| p.bad(e.to_string()))?.with_props(&props);
    let catalog = sources.catalog(env)?;
    let resolved = catalog.resolve(entry).map_err(|e| bad(p, "entry", e))?;
    let expected = resolved.expected().map_or(0, <[Expected]>::len);
    match catalog.missing(&resolved) {
        Some(missing) => Ok((missing, expected)),
        None => Err(p.bad(format!("`{}` does not know its full glyph set", entry.join("/")))),
    }
}

fn list_missing_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.listMissingGlyphs", v)?;
    let entry = path(&p, "entry")?;
    let (missing, expected) = missing_of(s, &p, &entry)?;
    let names: Vec<&str> = missing.iter().map(|e| e.name.as_str()).collect();
    Ok(json!({ "entry": entry, "missing": names, "present": expected.saturating_sub(missing.len()), "expected": expected }))
}

fn generate_missing_glyphs(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.generateMissingGlyphs", v)?;
    let entry = path(&p, "entry")?;
    let (missing, _) = missing_of(s, &p, &entry)?;
    let chosen: Vec<Expected> = match p.opt_strings("glyphs")? {
        None => missing,
        Some(names) => {
            let mut out: Vec<Expected> = Vec::new();
            for name in names {
                let Some(e) = missing.iter().find(|e| e.name == name) else {
                    return Err(p.bad(format!("`glyphs`: `{name}` is not missing from `{}`", entry.join("/"))));
                };
                if !out.iter().any(|o| o.name == name) {
                    out.push(e.clone());
                }
            }
            out
        }
    };
    if chosen.len() > MAX_GLYPHS {
        return Err(p.bad(format!("{} glyphs are missing; at most {MAX_GLYPHS} can be added at once", chosen.len())));
    }
    if chosen.is_empty() {
        return Ok(json!({ "added": [] }));
    }
    let data = s.glyph_data()?;
    let items: Vec<(String, Vec<u32>)> = chosen
        .iter()
        .map(|e| {
            let codes = match e.code_point {
                Some(cp) => vec![cp],
                None => data_props(&e.name, &[], &data, false).data_unicode.into_iter().collect(),
            };
            (e.name.clone(), codes)
        })
        .collect();
    s.edit("Generate Missing Glyphs", |font| {
        let glyphs = items
            .iter()
            .map(|(name, codes)| {
                let mut g = Glyph::new(name.as_str());
                g.unicodes = codes.clone();
                g.layers = font.masters.iter().map(|m| Layer::new(&m.id, NEW_GLYPH_WIDTH)).collect();
                g
            })
            .collect();
        Ok(insert_by_sort_key(font, &data, glyphs)?)
    })?;
    let added: Vec<&str> = items.iter().map(|(n, _)| n.as_str()).collect();
    Ok(json!({ "added": added }))
}

// ── The Filters section ─────────────────────────────────────────────────────────────────────────

fn add(s: &mut Session, p: &Params, parent_key: &str, filter: Filter) -> Result<Value> {
    let parent = opt_filter_path(p, parent_key)?.unwrap_or_default();
    let mut items = stored_filters(s)?;
    let path = gq::add_filter(&mut items, &parent, filter).map_err(|e| p.bad(e.to_string()))?;
    store_filters(s, p, &items)?;
    Ok(json!({ "filter": path }))
}

fn add_smart_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.addSmartFilter", v)?;
    let name = filter_name(&p)?;
    let rules = opt_rules(&p)?.unwrap_or_else(RuleTree::starter);
    add(s, &p, "folder", Filter::Smart { name, rules })
}

fn add_list_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.addListFilter", v)?;
    let name = filter_name(&p)?;
    let glyphs = opt_list_names(&p)?.unwrap_or_default();
    add(s, &p, "folder", Filter::List { name, glyphs })
}

fn add_filter_folder(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.addFilterFolder", v)?;
    let name = filter_name(&p)?;
    add(s, &p, "parent", Filter::Folder { name, children: Vec::new() })
}

fn edit_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.editFilter", v)?;
    let at = filter_path(&p, "filter")?;
    let name = match p.opt_str("name")? {
        Some(n) => Some(gq::check_filter_name(n).map_err(|e| bad(&p, "name", e))?),
        None => None,
    };
    let rules = opt_rules(&p)?;
    let glyphs = opt_list_names(&p)?;
    if name.is_none() && rules.is_none() && glyphs.is_none() {
        return Err(p.bad("give `name`, `rules` or `glyphs`"));
    }
    let mut items = stored_filters(s)?;
    let current = gq::filter_at(&items, &at).map_err(|e| bad(&p, "filter", e))?.clone();
    let name = name.unwrap_or_else(|| current.name().to_string());
    let changed = match current {
        Filter::Smart { rules: old, .. } => {
            if glyphs.is_some() {
                return Err(p.bad("`glyphs` belongs to list filters; this is a smart filter"));
            }
            Filter::Smart { name, rules: rules.unwrap_or(old) }
        }
        Filter::List { glyphs: old, .. } => {
            if rules.is_some() {
                return Err(p.bad("`rules` belongs to smart filters; this is a list filter"));
            }
            Filter::List { name, glyphs: glyphs.unwrap_or(old) }
        }
        Filter::Folder { children, .. } => {
            if rules.is_some() || glyphs.is_some() {
                return Err(p.bad("a folder has only a name"));
            }
            Filter::Folder { name, children }
        }
    };
    let new_path = gq::replace_filter(&mut items, &at, changed).map_err(|e| p.bad(e.to_string()))?;
    store_filters(s, &p, &items)?;
    Ok(json!({ "filter": new_path }))
}

fn rename_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.renameFilter", v)?;
    let at = filter_path(&p, "filter")?;
    let name = p.str("name")?;
    let mut items = stored_filters(s)?;
    let new_path = gq::rename_filter(&mut items, &at, name).map_err(|e| p.bad(e.to_string()))?;
    store_filters(s, &p, &items)?;
    Ok(json!({ "filter": new_path }))
}

fn remove_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.removeFilter", v)?;
    let at = filter_path(&p, "filter")?;
    let mut items = stored_filters(s)?;
    let removed = gq::remove_filter(&mut items, &at).map_err(|e| p.bad(e.to_string()))?;
    store_filters(s, &p, &items)?;
    Ok(json!({ "removed": at, "kind": removed.kind() }))
}

fn move_filter(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.moveFilter", v)?;
    let at = filter_path(&p, "filter")?;
    let parent = opt_filter_path(&p, "parent")?.unwrap_or_default();
    let index = p.opt_int("index", 0, gq::MAX_FILTERS as i64)?.ok_or_else(|| p.bad("`index` is required"))?;
    let index = usize::try_from(index).unwrap_or(0);
    let mut items = stored_filters(s)?;
    let new_path = gq::move_filter(&mut items, &at, &parent, index).map_err(|e| p.bad(e.to_string()))?;
    store_filters(s, &p, &items)?;
    Ok(json!({ "filter": new_path }))
}

fn set_visible_scripts(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("fontView.setVisibleScripts", v)?;
    let show = p.opt_strings("show")?;
    let hide = p.opt_strings("hide")?;
    if show.is_none() && hide.is_none() {
        return Err(p.bad("give `show` or `hide`"));
    }
    let mut scripts = visible_scripts(s)?;
    for id in show.unwrap_or_default() {
        let id = gq::check_script_id(id).map_err(|e| bad(&p, "show", e))?;
        if !scripts.contains(&id) {
            if scripts.len() >= gq::MAX_VISIBLE_SCRIPTS {
                return Err(p.bad(format!("the Languages section lists at most {} scripts by choice", gq::MAX_VISIBLE_SCRIPTS)));
            }
            scripts.push(id);
        }
    }
    for id in hide.unwrap_or_default() {
        let id = gq::check_script_id(id).map_err(|e| bad(&p, "hide", e))?;
        scripts.retain(|s| *s != id);
    }
    s.settings_mut().set_reserved(Reserved::VisibleScripts, &scripts)?;
    Ok(json!({ "scripts": scripts }))
}
