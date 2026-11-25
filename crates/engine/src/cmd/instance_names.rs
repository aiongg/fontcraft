//! `font.instance.names` (M7.32): what the export of each entry of Font Info › Exports writes about the font besides its
//! glyphs, through the compiler's own rules ([`fontcraft_compile::exported`]), without building a font.
//!
//! [`names_of`] runs on a font without the session, so the Exports tab runs it on a worker thread with the font of the
//! revision shown then.
//!
//! Limits: at most [`MAX_ENTRIES`] entries are computed, fewer when their texts together pass [`MAX_TEXT_BYTES`]; the
//! reply says which were left out.

use std::collections::HashMap;

use fontcraft_compile::exported::{ExportedNames, TextScope, TokenProblem};
use fontcraft_compile::info::NameRecord;
use fontcraft_compile::var::stat::StatInput;
use fontcraft_compile::{CompileError, Options};
use fontcraft_fontinfo::target;
use fontcraft_model::{Font, Instance, LANGUAGES};
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

/// Most entries whose names are computed.
pub const MAX_ENTRIES: usize = 1_000;
/// Most bytes of text the computed entries hold together.
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
/// Most other entries one conflict lists.
const MAX_CONFLICT_ENTRIES: usize = 100;
/// The bits of `fsSelection` by name, from bit 0.
const SELECTION_NAMES: [&str; 10] =
    ["italic", "underscore", "negative", "outlined", "strikeout", "bold", "regular", "useTypoMetrics", "wws", "oblique"];
/// The Windows language of the names every application reads.
const ENGLISH: u16 = 0x409;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.instance.names",
        "Exported Names",
        [],
        None,
        "{instances?: [id|name|index] (default all)} what the export of each entry writes: its file name, its name records with their text tokens expanded, its OS/2 classes and style flags, the style-linked family and slot it takes, and for a variable font setting the named instances and axis values. tokens lists the texts whose tokens name nothing (they are written as they stand), conflicts the active entries of the same kind that take the same file name, PostScript name or style-linked slot. The names that feature code, colour palettes and autohinting add are not listed, and tokens read glyphs before filters. An entry that cannot be exported has its error instead → {instances: [{index, id?, name, type: \"static\"|\"variable\", active, error?, folder?, fileName, postscriptName?, names: [{id, platform, encoding, language, tag?, text}], weightClass, widthClass, fsSelection: [string], styleGroup?: {family, style, entries: [index]}, axes?: [tag], namedInstances?: [{name, postscriptName?, coordinates: [number]}], stat?: {axes: [{tag, name, values: [{name, value, min?, max?, linkedValue?, elidable}]}], elidedFallbackName?}, tokens: [{scope: \"font\"|\"master\"|\"instance\", master?, param: bool, key, index, tokens: [string]}], conflicts: [{field: \"fileName\"|\"postscriptName\"|\"styleLinking\", value, entries: [index]}], warnings: [string]}], leftOut: n}",
        has_doc,
        names
    )]
}

fn names(s: &mut Session, raw: &Value) -> Result<Value> {
    let title = s.doc()?.title.clone();
    names_of(s.font()?, &title, raw)
}

/// `font.instance.names` on `font`, whose document is called `title`: the same parameters and answer as the command.
pub fn names_of(font: &Font, title: &str, raw: &Value) -> Result<Value> {
    let p = Params::new("font.instance.names", raw)?;
    let asked: Option<Vec<usize>> = match p.get("instances") {
        Some(raw) => {
            let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
            Some(target::instances(font, &refs).map_err(|e| p.bad(e.to_string()))?)
        }
        None => None,
    };
    // Every entry is computed whichever are asked for: an entry's conflicts are with any of them.
    let compiler = ExportedNames::new(font);
    let options = Options { version_stamp: true, ..Options::default() };
    let mut entries: Vec<Entry> = Vec::new();
    let mut bytes = 0usize;
    for (index, instance) in font.instances.iter().enumerate().take(MAX_ENTRIES) {
        if bytes > MAX_TEXT_BYTES {
            break;
        }
        let entry = entry(&compiler, title, index, instance, &options);
        bytes = bytes.saturating_add(entry.names.iter().map(|n| n.text.len()).fold(0, usize::saturating_add));
        entries.push(entry);
    }
    let conflicts = conflicts(&entries);
    let groups = style_groups(&entries);
    let left_out = font.instances.len().saturating_sub(entries.len());
    let out: Vec<Value> = entries
        .iter()
        .filter(|e| asked.as_ref().is_none_or(|a| a.contains(&e.index)))
        .map(|e| entry_json(e, conflicts.get(&e.index).map(Vec::as_slice).unwrap_or_default(), &groups))
        .collect();
    Ok(json!({ "instances": out, "leftOut": left_out }))
}

/// What one entry writes.
struct Entry {
    index: usize,
    id: Option<String>,
    name: String,
    variable: bool,
    active: bool,
    error: Option<String>,
    folder: Option<String>,
    file_name: String,
    postscript_name: Option<String>,
    names: Vec<NameRecord>,
    /// Name IDs 1 and 2, which link the styles of a static family.
    slot: Option<(String, String)>,
    classes: Option<(u16, u16)>,
    selection: u16,
    /// The fields only a variable font has.
    variable_fields: Vec<(&'static str, Value)>,
    tokens: Vec<TokenProblem>,
    warnings: Vec<String>,
}

fn entry(compiler: &ExportedNames, title: &str, index: usize, instance: &Instance, options: &Options) -> Entry {
    let variable = instance.kind.as_deref() == Some("variable");
    let mut out = Entry {
        index,
        id: instance.id.clone(),
        name: instance.name.clone(),
        variable,
        active: instance.exports,
        error: None,
        folder: None,
        file_name: String::new(),
        postscript_name: None,
        names: Vec::new(),
        slot: None,
        classes: None,
        selection: 0,
        variable_fields: Vec::new(),
        tokens: Vec::new(),
        warnings: Vec::new(),
    };
    let filled = if variable { variable_entry(compiler, &mut out, options) } else { static_entry(compiler, title, &mut out, options) };
    if let Err(e) = filled {
        out.error = Some(e.to_string());
    }
    out
}

fn static_entry(compiler: &ExportedNames, title: &str, out: &mut Entry, options: &Options) -> std::result::Result<(), CompileError> {
    let made = compiler.static_instance(out.index, options)?;
    out.file_name = super::compile::stem(&made.source, &made.resolved, options, title, false);
    match crate::export_steps::export_folder(&made.resolved, options) {
        Ok(folder) => out.folder = folder.map(|f| f.to_string_lossy().into_owned()),
        Err(e) => out.warnings.push(e.to_string()),
    }
    out.postscript_name = made.info.name(6);
    out.slot = made.info.name(1).zip(made.info.name(2));
    out.classes = Some((made.info.weight_class, made.info.width_class));
    out.selection = made.info.selection.bits();
    out.names = made.info.names;
    out.tokens = made.tokens;
    out.warnings.extend(made.warnings);
    Ok(())
}

fn variable_entry(compiler: &ExportedNames, out: &mut Entry, options: &Options) -> std::result::Result<(), CompileError> {
    let made = compiler.variable_setting(out.index, options)?;
    out.file_name = made.file_stem;
    out.postscript_name = made.info.name(6);
    out.classes = Some((made.info.weight_class, made.info.width_class));
    out.selection = made.info.selection.bits();
    out.names = made.info.names;
    let named: Vec<Value> = made
        .instances
        .iter()
        .map(|i| {
            let mut v = json!({ "name": i.name, "coordinates": i.coordinates });
            if let (Some(ps), Some(m)) = (&i.postscript_name, v.as_object_mut()) {
                m.insert("postscriptName".into(), json!(ps));
            }
            v
        })
        .collect();
    out.variable_fields.push(("axes", json!(made.axes)));
    out.variable_fields.push(("namedInstances", Value::Array(named)));
    if let Some(stat) = &made.stat {
        out.variable_fields.push(("stat", stat_json(stat)));
    }
    out.tokens = made.tokens;
    out.warnings.extend(made.warnings);
    Ok(())
}

fn stat_json(stat: &StatInput) -> Value {
    let axes: Vec<Value> = stat
        .axes
        .iter()
        .map(|a| {
            let values: Vec<Value> = a
                .values
                .iter()
                .map(|v| {
                    let mut out = json!({ "name": v.name, "value": v.value, "elidable": v.elidable });
                    if let Some(m) = out.as_object_mut() {
                        if let Some((min, max)) = v.range {
                            m.insert("min".into(), json!(min));
                            m.insert("max".into(), json!(max));
                        }
                        if let Some(linked) = v.linked_value {
                            m.insert("linkedValue".into(), json!(linked));
                        }
                    }
                    out
                })
                .collect();
            json!({ "tag": a.tag.to_string(), "name": a.name, "values": values })
        })
        .collect();
    let mut out = json!({ "axes": axes });
    if let (Some(name), Some(m)) = (&stat.elided_fallback_name, out.as_object_mut()) {
        m.insert("elidedFallbackName".into(), json!(name));
    }
    out
}

/// The static entries of each style-linked family and the slot each takes.
fn style_groups(entries: &[Entry]) -> HashMap<&str, Vec<usize>> {
    let mut out: HashMap<&str, Vec<usize>> = HashMap::new();
    for e in entries.iter().filter(|e| e.active) {
        if let Some((family, _)) = &e.slot {
            out.entry(family.as_str()).or_default().push(e.index);
        }
    }
    out
}

/// A field two exported files may not share, and the entries that share its value.
struct Conflict {
    field: &'static str,
    value: String,
    entries: Vec<usize>,
}

/// The conflicts of each active entry. Files of one kind land in one folder, so their names may not differ in case
/// alone; a variable font may share its PostScript name with a static one.
fn conflicts(entries: &[Entry]) -> HashMap<usize, Vec<Conflict>> {
    let mut taken: HashMap<(&'static str, bool, String), Vec<(usize, String)>> = HashMap::new();
    for e in entries.iter().filter(|e| e.active && e.error.is_none()) {
        let folder = e.folder.as_deref().unwrap_or_default();
        let file = if folder.is_empty() { e.file_name.clone() } else { format!("{folder}/{}", e.file_name) };
        taken.entry(("fileName", e.variable, file.to_lowercase())).or_default().push((e.index, file));
        if let Some(ps) = &e.postscript_name {
            taken.entry(("postscriptName", e.variable, ps.clone())).or_default().push((e.index, ps.clone()));
        }
        if let Some((family, style)) = &e.slot {
            taken.entry(("styleLinking", e.variable, format!("{family}\n{style}"))).or_default().push((e.index, format!("{family} {style}")));
        }
    }
    let mut out: HashMap<usize, Vec<Conflict>> = HashMap::new();
    for ((field, _, _), holders) in taken {
        if holders.len() < 2 {
            continue;
        }
        for (index, value) in &holders {
            let entries = holders.iter().map(|(i, _)| *i).filter(|i| i != index).take(MAX_CONFLICT_ENTRIES).collect();
            out.entry(*index).or_default().push(Conflict { field, value: value.clone(), entries });
        }
    }
    for list in out.values_mut() {
        list.sort_by_key(|c| c.field);
    }
    out
}

fn entry_json(e: &Entry, conflicts: &[Conflict], groups: &HashMap<&str, Vec<usize>>) -> Value {
    let mut out = json!({
        "index": e.index,
        "name": e.name,
        "type": if e.variable { "variable" } else { "static" },
        "active": e.active,
        "tokens": e.tokens.iter().map(token_json).collect::<Vec<_>>(),
        "conflicts": conflicts.iter().map(|c| json!({ "field": c.field, "value": c.value, "entries": c.entries })).collect::<Vec<_>>(),
        "warnings": e.warnings,
    });
    let Some(m) = out.as_object_mut() else { return out };
    if let Some(id) = &e.id {
        m.insert("id".into(), json!(id));
    }
    if let Some(error) = &e.error {
        m.insert("error".into(), json!(error));
        return out;
    }
    if let Some(folder) = &e.folder {
        m.insert("folder".into(), json!(folder));
    }
    m.insert("fileName".into(), json!(e.file_name));
    if let Some(ps) = &e.postscript_name {
        m.insert("postscriptName".into(), json!(ps));
    }
    m.insert("names".into(), e.names.iter().map(name_json).collect());
    if let Some((weight, width)) = e.classes {
        m.insert("weightClass".into(), json!(weight));
        m.insert("widthClass".into(), json!(width));
    }
    let flags: Vec<&str> = SELECTION_NAMES.iter().enumerate().filter(|(bit, _)| e.selection & (1 << bit) != 0).map(|(_, name)| *name).collect();
    m.insert("fsSelection".into(), json!(flags));
    if let Some((family, style)) = &e.slot {
        let members = groups.get(family.as_str()).cloned().unwrap_or_default();
        m.insert("styleGroup".into(), json!({ "family": family, "style": style, "entries": members }));
    }
    for (key, value) in &e.variable_fields {
        m.insert((*key).to_string(), value.clone());
    }
    out
}

fn name_json(r: &NameRecord) -> Value {
    let mut out = json!({ "id": r.id, "platform": r.platform, "encoding": r.encoding, "language": r.language, "text": r.text });
    let tag = (r.platform == 3 && r.language != ENGLISH).then(|| LANGUAGES.iter().find(|(_, id)| *id == r.language)).flatten();
    if let (Some((tag, _)), Some(m)) = (tag, out.as_object_mut()) {
        m.insert("tag".into(), json!(tag));
    }
    out
}

fn token_json(t: &TokenProblem) -> Value {
    let (scope, master) = match &t.scope {
        TextScope::Font => ("font", None),
        TextScope::Master(id) => ("master", Some(id)),
        TextScope::Instance => ("instance", None),
    };
    let mut out = json!({ "scope": scope, "param": t.param, "key": t.key, "index": t.index, "tokens": t.tokens });
    if let (Some(id), Some(m)) = (master, out.as_object_mut()) {
        m.insert("master".into(), json!(id));
    }
    out
}
