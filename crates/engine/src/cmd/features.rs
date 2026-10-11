//! Feature entries: prefixes, classes and features of Font Info › Features, and Update (M8.14).
//!
//! Commands: `font.features.list`, `get`, `listAvailable`, `addPrefix`, `addClass`, `addFeature`,
//! `remove`, `rename`, `reorder`, `setCode`, `setAutomatic`, `setNotes`, `update`, `updateEntry`,
//! `setLabel`, `removeLabel`, `openSpec`, `expandCode`. Entry data and Update logic live in
//! `fontcraft_features` (`auto`, `assemble`, `registry`).
//!
//! An entry is named by its group (`kind`), its name (a feature's tag, a class name without `@`)
//! and, when a name occurs more than once, its `index` in the group as `font.features.list`
//! reports it. The sidebar groups are always Prefix, Classes, Features, in that order
//! (`fontInfo.features.sidebarGroups`); automatic entries carry the update button
//! (`fontInfo.features.automaticIndicator`, the `automatic` flag of each listed entry).

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_features::auto::{self, AutoContext};
use fontcraft_features::{EntryKind, EntryRef, FeatureError, MAX_ENTRIES, MAX_ENTRY_BYTES, MasterValues, Target, assemble, registry};
use fontcraft_model::{DEFAULT_LANGUAGE, Extra, Features, Font, Localized, MAX_NAME_LEN, Snippet, language_id};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most bytes of the notes kept with one entry.
const MAX_NOTES_BYTES: usize = 1 << 20;
/// Most localized names of one stylistic set.
const MAX_LABELS: usize = 1_000;
/// Most characters of one localized stylistic-set name.
const MAX_LABEL_CHARS: usize = 8_000;
/// Most characters of a class name (the feature file specification's limit).
const MAX_CLASS_NAME: usize = 63;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "font.features.list",
            "List Feature Entries",
            [],
            None,
            "{kind?: \"prefix\"|\"class\"|\"feature\" (default: every group)} the sidebar entries in order: the prefixes, then the classes, then the features, each group in its stored order → {entries: [{kind, name, index: position in its group, automatic: bool (shown with an update button), disabled: bool, errors: bool (the last compile, if the font has not changed since, found errors in it)}]}",
            has_doc,
            list
        ),
        cmd!(
            query "font.features.get",
            "Get Feature Entry",
            [],
            None,
            "{kind: \"prefix\"|\"class\"|\"feature\", name: string (a feature's tag, a class name with or without @), index?: position in the group (default: the first entry of that name)} one entry → {kind, name, index, code, automatic, disabled, notes, errors: bool, labels?: [{language, name}] (stylistic sets ss01–ss20)}",
            has_doc,
            get
        ),
        cmd!(
            query "font.features.listAvailable",
            "Search Feature Tags",
            [],
            None,
            "{query?: string (matched against the tag and the name, ignoring case; default: every feature)} the registered feature tags, as the Add Feature picker lists them → {features: [{tag, name, url: the specification's section, inFont: bool, canGenerate: bool (Update could write its code for this font)}]}",
            has_doc,
            list_available
        ),
        cmd!(
            "font.features.addPrefix",
            "Add Prefix",
            [],
            None,
            "{name?: string (default: \"Prefix\", numbered when taken), code?: string, automatic?: bool (default: false; true asks Update's generators for the code and is refused when none writes this prefix)} add a prefix at the end of the Prefix group. A name another prefix has is refused → {kind: \"prefix\", name, index}",
            has_doc,
            add_prefix
        ),
        cmd!(
            "font.features.addClass",
            "Add Class",
            [],
            None,
            "{name: string (letters, digits, `_`, `.`, `-`, at most 63, not starting with a digit or `-`; a leading @ is dropped), code?: string, glyphs?: [glyph names] (written as the class body, one space apart; not with `code`), automatic?: bool (default: false; true asks Update's generators for the code, as for the All class)} add a class at the end of the Classes group. A name another class has is refused → {kind: \"class\", name, index}",
            has_doc,
            add_class
        ),
        cmd!(
            "font.features.addFeature",
            "Add Feature",
            [],
            None,
            "{tag: string (four letters, digits or `_`), code?: string, automatic?: bool (default: false; true asks Update's generators for the code and is refused when none writes this feature, as for calt)} add a feature at the end of the Features group; an implicit feature such as kern can be added to extend it by hand. A tag another feature has is refused → {kind: \"feature\", name: tag, index}",
            has_doc,
            add_feature
        ),
        cmd!(
            "font.features.remove",
            "Remove Feature Entries",
            [],
            None,
            "{entries: [{kind, name, index?}]} delete the entries, as the minus button does; every entry must exist → {removed: [{kind, name}]}",
            has_doc,
            remove
        ),
        cmd!(
            "font.features.rename",
            "Rename Feature Entry",
            [],
            None,
            "{kind, name, index?, newName: string} change a prefix's or class's name or a feature's tag; the new name follows the rules of addPrefix, addClass or addFeature and may not be another entry's in the group → {kind, name: new name, index}",
            has_doc,
            rename
        ),
        cmd!(
            "font.features.reorder",
            "Reorder Entries",
            [],
            None,
            "{kind, order: [name|index]} put the entries of one group in this order: every entry of the group exactly once, by name, or by its present index where a name occurs more than once. The order of features is the order of their blocks in the compiled code → {kind, order: [names]}",
            has_doc,
            reorder
        ),
        cmd!(
            "font.features.setCode",
            "Set Feature Code",
            [],
            None,
            "{kind, name, index?, code: string (at most 4 MB)} replace the code of one entry. An automatic entry is refused: turn off Generate Feature Automatically (font.features.setAutomatic) first → {kind, name, index}",
            has_doc,
            set_code
        ),
        cmd!(
            "font.features.setAutomatic",
            "Generate Feature Automatically",
            [],
            None,
            "{kind, name, index?, automatic: bool} turn automatic code on or off for one entry. Off keeps the code as manual text that Update leaves alone; on regenerates it at once and is refused for an entry no generator writes (such as calt) → {kind, name, index, automatic, code}",
            has_doc,
            set_automatic
        ),
        cmd!(
            "font.features.setNotes",
            "Set Feature Notes",
            [],
            None,
            "{kind, name, index?, notes: string (\"\" clears)} the free-text note kept with one entry → {kind, name, index, notes}",
            has_doc,
            set_notes
        ),
        cmd!(
            "font.features.update",
            "Update",
            [],
            None,
            "{} regenerate every automatic prefix, class and feature and add the entries the glyph names now call for, in one undo step; manual entries stay as they are, and optional entries (the All class, cpsp) are only refreshed when present → {added: [{kind, name}], updated: [{kind, name}], notGenerated: [{name, reason}]}",
            has_doc,
            update
        ),
        cmd!(
            "font.features.updateEntry",
            "Update Entry",
            [],
            None,
            "{kind, name, index?} regenerate the code of one automatic entry, as its update button does; a manual entry is refused → {kind, name, index, code}",
            has_doc,
            update_entry
        ),
        cmd!(
            "font.features.setLabel",
            "Set Stylistic Set Name",
            [],
            None,
            "{tag: \"ss01\"…\"ss20\", index?, language?: OpenType language tag (default: \"dflt\", the fallback name), name: string} add or change one localized name of a stylistic set → {tag, labels: [{language, name}]}",
            has_doc,
            set_label
        ),
        cmd!(
            "font.features.removeLabel",
            "Remove Stylistic Set Name",
            [],
            None,
            "{tag: \"ss01\"…\"ss20\", index?, language: string} remove one localized name of a stylistic set; a language without a name is refused → {tag, labels: [{language, name}]}",
            has_doc,
            remove_label
        ),
        cmd!(
            query "font.features.openSpec",
            "Spec",
            [],
            None,
            "{tag: string} the section of the OpenType specification that describes a registered feature; the Features tab opens `url` in the browser. An unregistered tag is refused → {tag, name, url}",
            always,
            open_spec
        ),
        cmd!(
            query "font.features.expandCode",
            "Expand Feature Code",
            [],
            None,
            "{target?: {master?: id|name} (default: the first master), includeImplicit?: bool (default: false)} the complete standard feature file for one master after tokens, conditional blocks and syntax extensions are resolved, every included entry written in place. Instances, the variable font and the implicit features are not available yet → {target: {master}, code}",
            has_doc,
            expand_code
        ),
    ]
}

// ── Entries ─────────────────────────────────────────────────────────────────────────────────────

fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Prefix => "prefix",
        EntryKind::Class => "class",
        EntryKind::Feature => "feature",
    }
}

fn kind_param(p: &Params, key: &str) -> Result<EntryKind> {
    match p.str(key)? {
        "prefix" => Ok(EntryKind::Prefix),
        "class" => Ok(EntryKind::Class),
        "feature" => Ok(EntryKind::Feature),
        other => Err(p.bad(format!("`{key}` must be \"prefix\", \"class\" or \"feature\", got \"{other}\""))),
    }
}

fn group(f: &Features, kind: EntryKind) -> &Vec<Snippet> {
    match kind {
        EntryKind::Prefix => &f.prefixes,
        EntryKind::Class => &f.classes,
        EntryKind::Feature => &f.features,
    }
}

fn group_mut(f: &mut Features, kind: EntryKind) -> &mut Vec<Snippet> {
    match kind {
        EntryKind::Prefix => &mut f.prefixes,
        EntryKind::Class => &mut f.classes,
        EntryKind::Feature => &mut f.features,
    }
}

fn entry_count(f: &Features) -> usize {
    f.prefixes.len() + f.classes.len() + f.features.len()
}

/// A name as the caller may spell it: a class name with its `@`.
fn bare(kind: EntryKind, name: &str) -> &str {
    match kind {
        EntryKind::Class => name.strip_prefix('@').unwrap_or(name),
        _ => name,
    }
}

/// The entry `name` (at `index` when given) of the group, from an object holding `kind`, `name`
/// and `index?`.
fn find(p: &Params, f: &Features, kind: EntryKind, name: &str, index: Option<usize>) -> Result<EntryRef> {
    let name = bare(kind, name);
    let entries = group(f, kind);
    let what = kind_name(kind);
    let index = match index {
        Some(i) => {
            let at = entries.get(i).ok_or_else(|| p.bad(format!("the font has no {what} at index {i}")))?;
            if at.name != name {
                return Err(p.bad(format!("the {what} at index {i} is `{}`, not `{name}`", at.name)));
            }
            i
        }
        None => entries.iter().position(|e| e.name == name).ok_or_else(|| p.bad(format!("the font has no {what} `{name}`")))?,
    };
    Ok(EntryRef { kind, index })
}

/// The entry the parameters `kind`, `name` and `index?` name.
fn entry_param(p: &Params, f: &Features) -> Result<EntryRef> {
    let kind = kind_param(p, "kind")?;
    let name = p.str("name")?;
    let index = p.opt_int("index", 0, MAX_ENTRIES as i64)?.map(|i| i as usize);
    find(p, f, kind, name, index)
}

/// The feature `tag` (at `index` when given), for the stylistic-set name commands.
fn stylistic_set(p: &Params, f: &Features) -> Result<EntryRef> {
    let tag = p.str("tag")?;
    if !is_stylistic_set(tag) {
        return Err(p.bad(format!("only stylistic sets (ss01 to ss20) have names, got `{tag}`")));
    }
    let index = p.opt_int("index", 0, MAX_ENTRIES as i64)?.map(|i| i as usize);
    find(p, f, EntryKind::Feature, tag, index)
}

fn is_stylistic_set(tag: &str) -> bool {
    tag.strip_prefix("ss").filter(|n| n.len() == 2).and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=20).contains(&n))
}

fn snippet<'a>(p: &Params, f: &'a Features, entry: EntryRef) -> Result<&'a Snippet> {
    group(f, entry.kind).get(entry.index).ok_or_else(|| p.bad("the entry is gone"))
}

fn snippet_mut<'a>(f: &'a mut Features, entry: EntryRef, command: &str) -> Result<&'a mut Snippet> {
    group_mut(f, entry.kind)
        .get_mut(entry.index)
        .ok_or_else(|| EngineError::BadParams { command: command.into(), message: "the entry is gone".into() })
}

/// Why `name` cannot name an entry of `kind`, if it cannot.
fn name_problem(kind: EntryKind, name: &str) -> Option<String> {
    match kind {
        EntryKind::Feature => {
            let ok = name.len() == 4 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            (!ok).then(|| format!("a feature tag is four letters, digits or `_`, got `{name}`"))
        }
        EntryKind::Class => {
            let first_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '.');
            let rest_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
            if name.is_empty() || name.chars().count() > MAX_CLASS_NAME || !first_ok || !rest_ok {
                Some(format!(
                    "a class name is 1 to {MAX_CLASS_NAME} letters, digits, `_`, `.` or `-`, not starting with a digit or `-`, got `{name}`"
                ))
            } else {
                None
            }
        }
        EntryKind::Prefix => {
            if name.trim().is_empty() {
                Some("a prefix name may not be empty".into())
            } else if name.chars().count() > MAX_NAME_LEN {
                Some(format!("a prefix name has at most {MAX_NAME_LEN} characters"))
            } else if name.chars().any(char::is_control) {
                Some("a prefix name may not hold control characters".into())
            } else {
                None
            }
        }
    }
}

/// `name` checked for a new or renamed entry of `kind`: valid, and not the name of another entry
/// of the group (`except` is the entry being renamed).
fn check_new_name(p: &Params, f: &Features, kind: EntryKind, name: &str, except: Option<usize>) -> Result<()> {
    if let Some(problem) = name_problem(kind, name) {
        return Err(p.bad(problem));
    }
    let taken = group(f, kind).iter().enumerate().any(|(i, e)| Some(i) != except && e.name == name);
    if taken {
        return Err(p.bad(format!("the font already has a {} `{name}`", kind_name(kind))));
    }
    Ok(())
}

fn check_code(p: &Params, code: &str) -> Result<()> {
    if code.len() > MAX_ENTRY_BYTES {
        return Err(p.bad(format!("the code has {} bytes, more than the {MAX_ENTRY_BYTES} one entry may hold", code.len())));
    }
    Ok(())
}

fn feature_error(p: &Params, e: FeatureError) -> EngineError {
    match e {
        FeatureError::NotYetImplemented(what) => EngineError::Unsupported(what.to_string()),
        other => p.bad(other.to_string()),
    }
}

/// The generators' view of the active font.
fn auto_context(s: &Session, p: &Params) -> Result<AutoContext> {
    let data = s.glyph_data()?;
    AutoContext::from_font(s.font()?, &data, None).map_err(|e| feature_error(p, e))
}

/// Some generator may write an entry named `name`.
fn generatable(name: &str) -> bool {
    auto::generators().iter().any(|g| g.names().contains(&name))
}

/// The entries of the active document's last compile that have errors, when it read the font
/// as it is now.
fn entries_with_errors(s: &Session) -> HashSet<EntryRef> {
    let (Some(state), Ok(doc)) = (s.feature_state(), s.doc()) else { return HashSet::new() };
    if !state.is_current(doc) {
        return HashSet::new();
    }
    state.diagnostics.iter().filter(|d| d.diagnostic.is_error()).filter_map(|d| d.place.as_ref().map(|r| r.entry)).collect()
}

fn labels_json(e: &Snippet) -> Value {
    Value::Array(e.labels.iter().map(|l| json!({"language": l.language, "name": l.value})).collect())
}

fn entry_json(kind: EntryKind, index: usize, e: &Snippet, errors: bool) -> Value {
    json!({
        "kind": kind_name(kind),
        "name": e.name,
        "index": index,
        "automatic": e.automatic,
        "disabled": e.disabled,
        "errors": errors,
    })
}

const GROUPS: [EntryKind; 3] = [EntryKind::Prefix, EntryKind::Class, EntryKind::Feature];

fn list(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.list", raw)?;
    let only = match p.get("kind") {
        None => None,
        Some(_) => Some(kind_param(&p, "kind")?),
    };
    let errors = entries_with_errors(s);
    let f = &s.font()?.features;
    let mut entries = Vec::new();
    for kind in GROUPS.into_iter().filter(|k| only.is_none_or(|o| o == *k)) {
        for (index, e) in group(f, kind).iter().enumerate() {
            entries.push(entry_json(kind, index, e, errors.contains(&EntryRef { kind, index })));
        }
    }
    Ok(json!({ "entries": entries }))
}

fn get(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.get", raw)?;
    let errors = entries_with_errors(s);
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let e = snippet(&p, f, entry)?;
    let mut out = entry_json(entry.kind, entry.index, e, errors.contains(&entry));
    if let Value::Object(map) = &mut out {
        map.insert("code".into(), json!(e.code));
        map.insert("notes".into(), json!(e.notes));
        if entry.kind == EntryKind::Feature && is_stylistic_set(&e.name) {
            map.insert("labels".into(), labels_json(e));
        }
    }
    Ok(out)
}

fn list_available(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.listAvailable", raw)?;
    let query = p.opt_str("query")?.unwrap_or("");
    let cx = auto_context(s, &p)?;
    // A generator that fails on this font cannot write its features here.
    let can: HashSet<String> =
        auto::generators().iter().filter_map(|g| g.generate(&cx).ok()).flatten().filter(|e| e.kind == EntryKind::Feature).map(|e| e.name).collect();
    let f = &s.font()?.features;
    let features: Vec<Value> = registry::search(query)
        .map(|t| {
            json!({
                "tag": t.tag,
                "name": t.name,
                "url": t.spec_url(),
                "inFont": f.features.iter().any(|e| e.name == t.tag),
                "canGenerate": can.contains(t.tag),
            })
        })
        .collect();
    Ok(json!({ "features": features }))
}

// ── Adding, removing, renaming, reordering ──────────────────────────────────────────────────────

/// Adds an entry at the end of its group, with its code generated when `automatic`.
fn add(s: &mut Session, p: &Params, label: &str, kind: EntryKind, name: String, code: Option<String>, automatic: bool) -> Result<Value> {
    let f = &s.font()?.features;
    check_new_name(p, f, kind, &name, None)?;
    if entry_count(f) >= MAX_ENTRIES {
        return Err(p.bad(format!("the font already has {MAX_ENTRIES} prefixes, classes and features")));
    }
    if automatic && code.is_some() {
        return Err(p.bad("an automatic entry gets its code from Update: give `code` or `automatic: true`, not both"));
    }
    if automatic && !generatable(&name) {
        return Err(p.bad(format!("no automatic code exists for the {} `{name}`: add it without `automatic`", kind_name(kind))));
    }
    if let Some(code) = &code {
        check_code(p, code)?;
    }
    let cx = if automatic { Some(auto_context(s, p)?) } else { None };
    let command = p.command();
    let index = s.edit(label, |font| {
        let features = Arc::make_mut(&mut font.features);
        let entries = group_mut(features, kind);
        entries.push(Snippet { name: name.clone(), code: code.unwrap_or_default(), automatic, ..Snippet::default() });
        let index = entries.len() - 1;
        if let Some(cx) = &cx {
            let updated = auto::update_entry(features, EntryRef { kind, index }, cx)
                .map_err(|e| EngineError::BadParams { command: command.into(), message: e.to_string() })?;
            *features = updated;
        }
        Ok(index)
    })?;
    Ok(json!({ "kind": kind_name(kind), "name": name, "index": index }))
}

fn add_prefix(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.addPrefix", raw)?;
    let name = match p.opt_str("name")? {
        Some(n) => n.to_string(),
        None => {
            let taken: HashSet<&str> = s.font()?.features.prefixes.iter().map(|e| e.name.as_str()).collect();
            let mut n = 1usize;
            loop {
                let candidate = if n == 1 { "Prefix".to_string() } else { format!("Prefix {n}") };
                // At most MAX_ENTRIES names are taken, so this ends within that many steps.
                if !taken.contains(candidate.as_str()) || n > MAX_ENTRIES {
                    break candidate;
                }
                n += 1;
            }
        }
    };
    let code = p.opt_str("code")?.map(str::to_string);
    let automatic = p.bool_or("automatic", false)?;
    add(s, &p, "Add Prefix", EntryKind::Prefix, name, code, automatic)
}

fn add_class(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.addClass", raw)?;
    let name = bare(EntryKind::Class, p.str("name")?).to_string();
    let glyphs = p.opt_glyphs("glyphs", s.font()?)?;
    let code = match (p.opt_str("code")?, glyphs) {
        (Some(_), Some(_)) => return Err(p.bad("give the class body as `code` or as `glyphs`, not both")),
        (Some(code), None) => Some(code.to_string()),
        (None, Some(glyphs)) => Some(glyphs.join(" ")),
        (None, None) => None,
    };
    let automatic = p.bool_or("automatic", false)?;
    add(s, &p, "Add Class", EntryKind::Class, name, code, automatic)
}

fn add_feature(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.addFeature", raw)?;
    let tag = p.str("tag")?.to_string();
    let code = p.opt_str("code")?.map(str::to_string);
    let automatic = p.bool_or("automatic", false)?;
    add(s, &p, "Add Feature", EntryKind::Feature, tag, code, automatic)
}

fn remove(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.remove", raw)?;
    let list = p.array("entries")?;
    if list.is_empty() {
        return Err(p.bad("`entries` is empty"));
    }
    let f = &s.font()?.features;
    if list.len() > entry_count(f) {
        return Err(p.bad(format!("`entries` names {} entries, more than the font has", list.len())));
    }
    let mut refs = Vec::new();
    for item in list {
        let ip = Params::new("font.features.remove", item)?;
        let entry = entry_param(&ip, f).map_err(|e| p.bad(format!("`entries`: {}", message(&e))))?;
        if !refs.contains(&entry) {
            refs.push(entry);
        }
    }
    let removed: Vec<Value> =
        refs.iter().filter_map(|r| group(f, r.kind).get(r.index).map(|e| json!({"kind": kind_name(r.kind), "name": e.name}))).collect();
    // Highest index first, so the indices still to remove stay valid.
    refs.sort_by(|a, b| b.cmp(a));
    s.edit("Remove Feature Entries", |font| {
        let features = Arc::make_mut(&mut font.features);
        for r in &refs {
            let entries = group_mut(features, r.kind);
            if r.index < entries.len() {
                entries.remove(r.index);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "removed": removed }))
}

/// The message of a parameter error, without the command name.
fn message(e: &EngineError) -> String {
    match e {
        EngineError::BadParams { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn rename(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.rename", raw)?;
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let new_name = bare(entry.kind, p.str("newName")?).to_string();
    check_new_name(&p, f, entry.kind, &new_name, Some(entry.index))?;
    let command = p.command();
    s.edit("Rename Feature Entry", |font| {
        snippet_mut(Arc::make_mut(&mut font.features), entry, command)?.name = new_name.clone();
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(entry.kind), "name": new_name, "index": entry.index }))
}

fn reorder(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.reorder", raw)?;
    let kind = kind_param(&p, "kind")?;
    let order = p.array("order")?;
    let entries = group(&s.font()?.features, kind);
    if order.len() != entries.len() {
        return Err(p.bad(format!("`order` has {} entries; the group has {}, and each must be listed once", order.len(), entries.len())));
    }
    let mut used = vec![false; entries.len()];
    let mut picked = Vec::with_capacity(order.len());
    for item in order {
        let index = match item {
            Value::String(name) => {
                let name = bare(kind, name);
                // The first entry of that name not listed yet.
                entries
                    .iter()
                    .enumerate()
                    .position(|(i, e)| e.name == name && !used.get(i).copied().unwrap_or(true))
                    .ok_or_else(|| p.bad(format!("`order`: no {} `{name}` left to place", kind_name(kind))))?
            }
            Value::Number(n) => {
                let i = n.as_u64().map(|i| i as usize).filter(|i| *i < entries.len());
                i.ok_or_else(|| p.bad(format!("`order`: {n} is not the index of a {}", kind_name(kind))))?
            }
            other => return Err(p.bad(format!("`order` holds names or indices, found {}", crate::params::kind(other)))),
        };
        match used.get_mut(index) {
            Some(slot) if !*slot => *slot = true,
            _ => return Err(p.bad(format!("`order` lists the {} at index {index} twice", kind_name(kind)))),
        }
        picked.push(index);
    }
    let names: Vec<String> = picked.iter().filter_map(|&i| entries.get(i).map(|e| e.name.clone())).collect();
    s.edit("Reorder Entries", |font| {
        let features = Arc::make_mut(&mut font.features);
        let old = std::mem::take(group_mut(features, kind));
        let mut slots: Vec<Option<Snippet>> = old.into_iter().map(Some).collect();
        let reordered: Vec<Snippet> = picked.iter().filter_map(|&i| slots.get_mut(i).and_then(Option::take)).collect();
        *group_mut(features, kind) = reordered;
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(kind), "order": names }))
}

// ── Code, automatic flag, notes, Update ─────────────────────────────────────────────────────────

fn set_code(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.setCode", raw)?;
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let e = snippet(&p, f, entry)?;
    if e.automatic {
        return Err(p.bad(format!(
            "the {} `{}` is generated automatically: turn off Generate Feature Automatically to edit its code",
            kind_name(entry.kind),
            e.name
        )));
    }
    let name = e.name.clone();
    let code = p.str("code")?.to_string();
    check_code(&p, &code)?;
    let command = p.command();
    s.edit("Set Feature Code", |font| {
        snippet_mut(Arc::make_mut(&mut font.features), entry, command)?.code = code;
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(entry.kind), "name": name, "index": entry.index }))
}

fn set_automatic(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.setAutomatic", raw)?;
    let automatic = match p.get("automatic") {
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`automatic` must be true or false, got {}", crate::params::kind(other)))),
        None => return Err(p.bad("`automatic` is required")),
    };
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let name = snippet(&p, f, entry)?.name.clone();
    if automatic && !generatable(&name) {
        return Err(p.bad(format!("no automatic code exists for the {} `{name}`: it is always written by hand", kind_name(entry.kind))));
    }
    let cx = if automatic { Some(auto_context(s, &p)?) } else { None };
    let command = p.command();
    let code = s.edit("Generate Feature Automatically", |font| {
        let features = Arc::make_mut(&mut font.features);
        snippet_mut(features, entry, command)?.automatic = automatic;
        if let Some(cx) = &cx {
            *features =
                auto::update_entry(features, entry, cx).map_err(|e| EngineError::BadParams { command: command.into(), message: e.to_string() })?;
        }
        Ok(snippet_mut(features, entry, command)?.code.clone())
    })?;
    Ok(json!({ "kind": kind_name(entry.kind), "name": name, "index": entry.index, "automatic": automatic, "code": code }))
}

fn set_notes(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.setNotes", raw)?;
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let name = snippet(&p, f, entry)?.name.clone();
    let notes = match p.get("notes") {
        None => return Err(p.bad("`notes` is required (\"\" clears)")),
        Some(_) => p.str("notes")?.to_string(),
    };
    if notes.len() > MAX_NOTES_BYTES {
        return Err(p.bad(format!("the notes have more than {MAX_NOTES_BYTES} bytes")));
    }
    let command = p.command();
    s.edit("Set Feature Notes", |font| {
        snippet_mut(Arc::make_mut(&mut font.features), entry, command)?.notes = notes.clone();
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(entry.kind), "name": name, "index": entry.index, "notes": notes }))
}

fn pairs_json(list: &[(EntryKind, String)]) -> Value {
    Value::Array(list.iter().map(|(k, n)| json!({"kind": kind_name(*k), "name": n})).collect())
}

fn update(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.update", raw)?;
    let cx = auto_context(s, &p)?;
    let report = auto::update(&s.font()?.features, &cx).map_err(|e| feature_error(&p, e))?;
    if entry_count(&report.features) > MAX_ENTRIES {
        return Err(p.bad(format!("Update would leave more than {MAX_ENTRIES} prefixes, classes and features")));
    }
    let features = report.features;
    s.edit("Update", |font| {
        if *font.features != features {
            font.features = Arc::new(features);
        }
        Ok(())
    })?;
    let not_generated: Vec<Value> = report.not_generated.iter().map(|(name, reason)| json!({"name": name, "reason": reason})).collect();
    Ok(json!({
        "added": pairs_json(&report.added),
        "updated": pairs_json(&report.updated),
        "notGenerated": not_generated,
    }))
}

fn update_entry(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.updateEntry", raw)?;
    let f = &s.font()?.features;
    let entry = entry_param(&p, f)?;
    let e = snippet(&p, f, entry)?;
    if !e.automatic {
        return Err(p.bad(format!("the {} `{}` is written by hand; Update leaves it alone", kind_name(entry.kind), e.name)));
    }
    let name = e.name.clone();
    let cx = auto_context(s, &p)?;
    let updated = auto::update_entry(&s.font()?.features, entry, &cx).map_err(|e| feature_error(&p, e))?;
    let code = group(&updated, entry.kind).get(entry.index).map(|e| e.code.clone()).unwrap_or_default();
    s.edit("Update Entry", |font| {
        if *font.features != updated {
            font.features = Arc::new(updated);
        }
        Ok(())
    })?;
    Ok(json!({ "kind": kind_name(entry.kind), "name": name, "index": entry.index, "code": code }))
}

// ── Stylistic set names ─────────────────────────────────────────────────────────────────────────

fn language_param(p: &Params, required: bool) -> Result<String> {
    let language = match p.opt_str("language")? {
        Some(l) => l,
        None if required => return Err(p.bad("`language` is required")),
        None => DEFAULT_LANGUAGE,
    };
    if language != DEFAULT_LANGUAGE && language_id(language).is_none() {
        return Err(p.bad(format!("`language` must be \"{DEFAULT_LANGUAGE}\" or an OpenType language tag such as \"DEU\", got \"{language}\"")));
    }
    Ok(language.to_string())
}

fn set_label(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.setLabel", raw)?;
    let f = &s.font()?.features;
    let entry = stylistic_set(&p, f)?;
    let language = language_param(&p, false)?;
    let name = p.str("name")?.to_string();
    if name.trim().is_empty() {
        return Err(p.bad("the name is empty: remove the row with font.features.removeLabel instead"));
    }
    if name.chars().count() > MAX_LABEL_CHARS {
        return Err(p.bad(format!("the name has more than {MAX_LABEL_CHARS} characters")));
    }
    let e = snippet(&p, f, entry)?;
    if !e.labels.iter().any(|l| l.language == language) && e.labels.len() >= MAX_LABELS {
        return Err(p.bad(format!("the stylistic set already has {MAX_LABELS} names")));
    }
    let tag = e.name.clone();
    let command = p.command();
    let labels = s.edit("Set Stylistic Set Name", |font| {
        let e = snippet_mut(Arc::make_mut(&mut font.features), entry, command)?;
        match e.labels.iter_mut().find(|l| l.language == language) {
            Some(l) => l.value = name.clone(),
            None => e.labels.push(Localized { language: language.clone(), value: name.clone(), extra: Extra::default() }),
        }
        Ok(labels_json(e))
    })?;
    Ok(json!({ "tag": tag, "labels": labels }))
}

fn remove_label(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.removeLabel", raw)?;
    let f = &s.font()?.features;
    let entry = stylistic_set(&p, f)?;
    let language = language_param(&p, true)?;
    let e = snippet(&p, f, entry)?;
    if !e.labels.iter().any(|l| l.language == language) {
        return Err(p.bad(format!("the stylistic set `{}` has no name in \"{language}\"", e.name)));
    }
    let tag = e.name.clone();
    let command = p.command();
    let labels = s.edit("Remove Stylistic Set Name", |font| {
        let e = snippet_mut(Arc::make_mut(&mut font.features), entry, command)?;
        e.labels.retain(|l| l.language != language);
        Ok(labels_json(e))
    })?;
    Ok(json!({ "tag": tag, "labels": labels }))
}

// ── Queries: the specification, the expanded code ───────────────────────────────────────────────

fn open_spec(_: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.openSpec", raw)?;
    let tag = p.str("tag")?;
    let t = registry::lookup(tag).ok_or_else(|| p.bad(format!("`{tag}` is not a registered feature tag")))?;
    Ok(json!({ "tag": t.tag, "name": t.name, "url": t.spec_url() }))
}

fn master_id(p: &Params, font: &Font, key: &str, value: Option<&str>) -> Result<String> {
    match value {
        None => Ok(font.default_master()?.id.clone()),
        Some(v) => font
            .masters
            .iter()
            .find(|m| m.id == v)
            .or_else(|| font.masters.iter().find(|m| m.name == v))
            .map(|m| m.id.clone())
            .ok_or_else(|| p.bad(format!("`{key}`: the font has no master `{v}`"))),
    }
}

fn expand_code(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.expandCode", raw)?;
    if p.bool_or("includeImplicit", false)? {
        return Err(EngineError::Unsupported("writing the implicit features (kern, mark, mkmk, curs, dist) as feature code".into()));
    }
    let master = match p.opt_object("target")? {
        None => None,
        Some(target) => {
            if target.contains_key("instance") || target.contains_key("variable") {
                return Err(EngineError::Unsupported("expanding feature code for an instance or the variable font".into()));
            }
            if let Some(key) = target.keys().find(|k| k.as_str() != "master") {
                return Err(p.bad(format!("`target` takes `master`, found `{key}`")));
            }
            match target.get("master") {
                None | Some(Value::Null) => None,
                Some(Value::String(m)) => Some(m.as_str()),
                Some(other) => return Err(p.bad(format!("`target.master` must be a string, got {}", crate::params::kind(other)))),
            }
        }
    };
    let data = s.glyph_data()?;
    let font = s.font()?;
    let master = master_id(&p, font, "target.master", master)?;
    let values = MasterValues::new(font, &data, &master).map_err(|e| feature_error(&p, e))?;
    let assembly = assemble(font, &font.features, &Target::Master(master.clone()), &values).map_err(|e| feature_error(&p, e))?;
    Ok(json!({ "target": {"master": master}, "code": assembly.expanded_text() }))
}
