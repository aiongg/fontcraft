//! The Filters section of the sidebar: smart filters, list filters and folders
//! (`docs/spec/05-font-view.md` § 6.3–6.5). The tree lives in the user settings; these functions
//! read and change it and keep it within its limits.
//!
//! A filter is addressed by its path: the names of the folders that hold it, then its own name.
//! Names are unique among siblings.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::rules::{GroupKind, Property, Rule, RuleNode, RuleOp, RuleTree, RuleValue};
use crate::{MAX_FILTER_NAME_BYTES, MAX_FILTERS, MAX_FOLDER_DEPTH, MAX_LIST_NAMES, QueryError, fail, short};
use fontcraft_glyphdata::MAX_NAME_BYTES;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Filter {
    /// Rules evaluated against the current font.
    Smart { name: String, rules: RuleTree },
    /// Glyph names in the order Font View shows them; names the font lacks count as missing.
    List { name: String, glyphs: Vec<String> },
    /// Shows the union of everything inside it.
    Folder {
        name: String,
        #[serde(default)]
        children: Vec<Filter>,
    },
}

impl Filter {
    pub fn name(&self) -> &str {
        match self {
            Filter::Smart { name, .. } | Filter::List { name, .. } | Filter::Folder { name, .. } => name,
        }
    }

    fn name_mut(&mut self) -> &mut String {
        match self {
            Filter::Smart { name, .. } | Filter::List { name, .. } | Filter::Folder { name, .. } => name,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Filter::Smart { .. } => "smart",
            Filter::List { .. } => "list",
            Filter::Folder { .. } => "folder",
        }
    }
}

/// The name of the built-in smart filter that lists glyphs whose masters cannot interpolate
/// (`compatibility.smartFilterIncompatibleMasters`).
pub const INCOMPATIBLE_MASTERS_FILTER: &str = "Incompatible masters";

/// The Filters section before the user changes it: the built-in Incompatible masters filter.
pub fn default_filters() -> Vec<Filter> {
    let rule = Rule { property: Property::MasterCompatible, op: RuleOp::Is, value: RuleValue::Bool(false) };
    vec![Filter::Smart { name: INCOMPATIBLE_MASTERS_FILTER.to_string(), rules: RuleTree { kind: GroupKind::All, rules: vec![RuleNode::Rule(rule)] } }]
}

/// A usable filter or folder name: trimmed, not empty, no control characters.
pub fn check_filter_name(name: &str) -> Result<String, QueryError> {
    let name = name.trim();
    if name.is_empty() {
        return fail("a filter name cannot be empty");
    }
    if name.len() > MAX_FILTER_NAME_BYTES {
        return fail(format!("a filter name is at most {MAX_FILTER_NAME_BYTES} bytes long"));
    }
    if name.chars().any(char::is_control) {
        return fail("a filter name cannot contain control characters");
    }
    Ok(name.to_string())
}

/// The names of a list filter, from strings that may each hold several names separated by
/// spaces, tabs or line breaks. Repeats are dropped; the first place counts.
pub fn list_names<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<Vec<String>, QueryError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for text in texts {
        for name in text.split_whitespace() {
            if name.len() > MAX_NAME_BYTES {
                return fail(format!("the glyph name `{}` is longer than {MAX_NAME_BYTES} bytes", short(name)));
            }
            if name.chars().any(char::is_control) {
                return fail("a glyph name cannot contain control characters");
            }
            if seen.insert(name) {
                if out.len() >= MAX_LIST_NAMES {
                    return fail(format!("a list filter holds at most {MAX_LIST_NAMES} names"));
                }
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

/// Checks a whole tree: names, unique siblings, nesting, counts and list sizes. Rule trees were
/// checked when they were read.
pub fn validate_filters(items: &[Filter]) -> Result<(), QueryError> {
    let mut count = 0;
    validate_level(items, 1, &mut count)
}

fn validate_level(items: &[Filter], depth: usize, count: &mut usize) -> Result<(), QueryError> {
    if depth > MAX_FOLDER_DEPTH && !items.is_empty() {
        return fail(format!("filter folders are nested more than {MAX_FOLDER_DEPTH} levels deep"));
    }
    let mut names = HashSet::new();
    for f in items {
        *count += 1;
        if *count > MAX_FILTERS {
            return fail(format!("the Filters section holds at most {MAX_FILTERS} filters and folders"));
        }
        let name = check_filter_name(f.name())?;
        if name != f.name() {
            return fail(format!("the filter name `{}` has spaces around it", short(f.name())));
        }
        if !names.insert(f.name()) {
            return fail(format!("two filters in one place are both named `{}`", short(f.name())));
        }
        match f {
            Filter::List { glyphs, .. } => {
                let clean = list_names(glyphs.iter().map(String::as_str))?;
                if clean.len() != glyphs.len() {
                    return fail(format!("the list filter `{}` holds a repeated, empty or spaced name", short(f.name())));
                }
            }
            Filter::Folder { children, .. } => validate_level(children, depth + 1, count)?,
            Filter::Smart { .. } => {}
        }
    }
    Ok(())
}

fn path_text(path: &[String]) -> String {
    short(&path.join("/"))
}

/// The positions of each segment of `path`, from the top.
fn indices(items: &[Filter], path: &[String]) -> Result<Vec<usize>, QueryError> {
    if path.is_empty() {
        return fail("the filter path is empty");
    }
    let mut level = items;
    let mut out = Vec::with_capacity(path.len());
    for (depth, segment) in path.iter().enumerate() {
        let Some(i) = level.iter().position(|f| f.name() == segment.as_str()) else {
            return fail(format!("there is no filter `{}`", path_text(path)));
        };
        out.push(i);
        if depth + 1 < path.len() {
            match level.get(i) {
                Some(Filter::Folder { children, .. }) => level = children,
                _ => return fail(format!("`{}` is not a folder", path_text(path.get(..=depth).unwrap_or_default()))),
            }
        }
    }
    Ok(out)
}

/// The filter at `path`.
pub fn filter_at<'a>(items: &'a [Filter], path: &[String]) -> Result<&'a Filter, QueryError> {
    let at = indices(items, path)?;
    let mut level = items;
    let mut found = None;
    for i in at {
        let f = level.get(i).ok_or_else(|| QueryError(format!("there is no filter `{}`", path_text(path))))?;
        if let Filter::Folder { children, .. } = f {
            level = children;
        }
        found = Some(f);
    }
    found.ok_or_else(|| QueryError(format!("there is no filter `{}`", path_text(path))))
}

/// The list that holds the folder at `parent` (the top level when `parent` is empty).
fn level_mut<'a>(items: &'a mut Vec<Filter>, parent: &[String]) -> Result<&'a mut Vec<Filter>, QueryError> {
    if parent.is_empty() {
        return Ok(items);
    }
    if !matches!(filter_at(items, parent)?, Filter::Folder { .. }) {
        return fail(format!("`{}` is not a folder", path_text(parent)));
    }
    let at = indices(items, parent)?;
    let mut level = items;
    for i in at {
        let next = { level }.get_mut(i);
        match next {
            Some(Filter::Folder { children, .. }) => level = children,
            _ => return fail(format!("`{}` is not a folder", path_text(parent))),
        }
    }
    Ok(level)
}

fn depth_of(f: &Filter) -> usize {
    match f {
        Filter::Folder { children, .. } => 1 + children.iter().map(depth_of).max().unwrap_or(0),
        _ => 1,
    }
}

/// Adds `filter` at the end of the folder at `parent` (the top level when empty) and returns its
/// path.
pub fn add_filter(items: &mut Vec<Filter>, parent: &[String], filter: Filter) -> Result<Vec<String>, QueryError> {
    let name = check_filter_name(filter.name())?;
    let level = level_mut(items, parent)?;
    if level.iter().any(|f| f.name() == name.as_str()) {
        return fail(format!("there is already a filter named `{}` there", short(&name)));
    }
    let mut filter = filter;
    *filter.name_mut() = name.clone();
    level.push(filter);
    if let Err(e) = validate_filters(items) {
        if let Ok(level) = level_mut(items, parent) {
            level.pop();
        }
        return Err(e);
    }
    let mut path = parent.to_vec();
    path.push(name);
    Ok(path)
}

/// Removes the filter or folder at `path`, with everything inside it.
pub fn remove_filter(items: &mut Vec<Filter>, path: &[String]) -> Result<Filter, QueryError> {
    let (name, parent) = path.split_last().ok_or_else(|| QueryError("the filter path is empty".into()))?;
    filter_at(items, path)?;
    let level = level_mut(items, parent)?;
    let i = level.iter().position(|f| f.name() == name.as_str()).ok_or_else(|| QueryError(format!("there is no filter `{}`", path_text(path))))?;
    Ok(level.remove(i))
}

/// Renames the filter or folder at `path` and returns its new path.
pub fn rename_filter(items: &mut Vec<Filter>, path: &[String], name: &str) -> Result<Vec<String>, QueryError> {
    let new_name = check_filter_name(name)?;
    let (old, parent) = path.split_last().ok_or_else(|| QueryError("the filter path is empty".into()))?;
    filter_at(items, path)?;
    let level = level_mut(items, parent)?;
    if old.as_str() != new_name && level.iter().any(|f| f.name() == new_name.as_str()) {
        return fail(format!("there is already a filter named `{}` there", short(&new_name)));
    }
    if let Some(f) = level.iter_mut().find(|f| f.name() == old.as_str()) {
        *f.name_mut() = new_name.clone();
    }
    let mut out = parent.to_vec();
    out.push(new_name);
    Ok(out)
}

/// Replaces the filter at `path` with `filter`, which keeps its place (and must keep its kind;
/// the caller checks). Returns the new path.
pub fn replace_filter(items: &mut Vec<Filter>, path: &[String], filter: Filter) -> Result<Vec<String>, QueryError> {
    let renamed = rename_filter(items, path, filter.name())?;
    let (name, parent) = renamed.split_last().ok_or_else(|| QueryError("the filter path is empty".into()))?;
    let level = level_mut(items, parent)?;
    if let Some(slot) = level.iter_mut().find(|f| f.name() == name.as_str()) {
        let mut filter = filter;
        *filter.name_mut() = name.clone();
        *slot = filter;
    }
    validate_filters(items)?;
    Ok(renamed)
}

/// Moves the filter at `path` into the folder at `parent` (the top level when empty), at
/// `index` among the entries there once it has left its old place. Returns its new path.
pub fn move_filter(items: &mut Vec<Filter>, path: &[String], parent: &[String], index: usize) -> Result<Vec<String>, QueryError> {
    let moving = filter_at(items, path)?;
    if parent.len() >= path.len() && parent.get(..path.len()) == Some(path) {
        return fail(format!("`{}` cannot be moved into itself", path_text(path)));
    }
    if parent.len() + depth_of(moving) > MAX_FOLDER_DEPTH {
        return fail(format!("filter folders are nested at most {MAX_FOLDER_DEPTH} levels deep"));
    }
    let name = moving.name().to_string();
    let (_, old_parent) = path.split_last().ok_or_else(|| QueryError("the filter path is empty".into()))?;
    if old_parent != parent && level_mut(items, parent)?.iter().any(|f| f.name() == name.as_str()) {
        return fail(format!("there is already a filter named `{}` there", short(&name)));
    }
    // Check the destination exists before taking the filter out, so a refusal changes nothing.
    let room = level_mut(items, parent)?.len().saturating_sub(usize::from(old_parent == parent));
    if index > room {
        return fail(format!("`index` must be from 0 to {room}, got {index}"));
    }
    let taken = remove_filter(items, path)?;
    let level = level_mut(items, parent)?;
    level.insert(index.min(level.len()), taken);
    let mut out = parent.to_vec();
    out.push(name);
    Ok(out)
}
