//! Compiling feature code, its diagnostics and the editor's queries. The result is kept in
//! [`crate::feature_state`] (M8.15).
//!
//! Commands: `font.features.compile`, `listErrors`, `applyFix`, `jumpToError`, `showErrorDetails`,
//! `evaluateToken`, `complete`, `previewName`.
//!
//! Compile assembles the entries for one master (tokens expanded, extensions lowered) and runs
//! the feature compiler over the whole code, substitutions and positioning, without the
//! generated kerning and marks; with no errors it builds the preview font. The result, errors
//! mapped to the entries, is the document's feature state. Compiling the same input again (the
//! same assembled files, glyph order, master and include folder) reuses the state instead of
//! running the compiler, so the Features tab may compile after each pause in typing.
//!
//! A diagnostic's id is its index in the last compile. `listErrors`, `jumpToError` and
//! `showErrorDetails` return the data the Features tab draws (inline markers, sidebar icons,
//! the expanded message); `applyFix` changes the font, one undo step. Ranges are byte ranges of
//! the entry's stored code; lines and columns count from 1, columns in characters.
//!
//! The editor's queries read the active document: `complete` (`fontInfo.features.autocomplete`),
//! `evaluateToken` (Option-click on a token), `previewName` (Alt-click on a glyph or class name).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_compile::preview::{self, PreviewFont, PreviewKind};
use fontcraft_fea::{CompileRequest, Diagnostic, GlyphOrder, LayoutTable, MAX_DIAGNOSTICS, MAX_GLYPHS, Severity};
use fontcraft_features::edit::{self, CompletionKind, CompletionNames, Definitions, Fix, MAX_FIXES};
use fontcraft_features::tokens::{self, TokenValue};
use fontcraft_features::{EntryDiagnostic, EntryKind, EntryRef, FeatureError, MAX_ENTRY_BYTES, MasterValues, Target, TargetValues, assemble};
use fontcraft_model::{Features, Font, Snippet};
use serde_json::{Value, json};

use super::*;
use crate::feature_state::FeatureState;
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most bytes of entry code read for the class and lookup names completion offers.
const MAX_NAME_SCAN_BYTES: usize = 4 << 20;
/// Most characters of a code line quoted in an error's details.
const MAX_QUOTED_CHARS: usize = 300;
/// How deep class entries that name other classes are followed for a preview.
const MAX_CLASS_DEPTH: usize = 64;
/// Most names a class preview lists as not in the font.
const MAX_UNKNOWN: usize = 1_000;
/// Most included files whose modification times key a reused compile; code that includes more
/// is compiled every time.
const MAX_KEYED_INCLUDES: usize = 256;
/// Most bytes of included files read to find the files they include in turn, for the same key.
const MAX_KEYED_INCLUDE_BYTES: usize = 4 << 20;
/// Most bytes after `include` searched for the closing parenthesis of the file name.
const MAX_INCLUDE_NAME_BYTES: usize = 1_024;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.features.compile",
            "Compile",
            [],
            None,
            "{master?: id|name (default: the first master; the Features tab passes the master of the active Edit View tab), force?: bool (compile even when the code, glyph order and master are those of the last compile; default false)} compiles the feature code of one master so Edit View can preview it, and keeps the result as the document's feature state (no undo step, never saved). Feature code with errors is not a failure: the errors come back → {master, ok: bool (no errors), cached: bool (the last compile's result, reused), errors: n, warnings: n, features: [{tag, table: \"GSUB\"|\"GPOS\", languageSystems: [[script, language]], lookups: n}], diagnostics: [as font.features.listErrors], preview: bool (the preview font was built), previewMessage?: string}",
            has_doc,
            compile
        ),
        cmd!(
            query "font.features.listErrors",
            "List Feature Errors",
            [],
            None,
            "{kind?: \"prefix\"|\"class\"|\"feature\", name?: string, index?: position in the group (with kind and name: that entry's only)} the errors and warnings of the last compile → {compiled: bool, current: bool (the font has not changed since), master?, errors: n, warnings: n, diagnostics: [{id, severity: \"error\"|\"warning\", message, entry: {kind, name, index}|null (null: about no entry), range: [start, end]|null (bytes of the entry's code), line, column (from 1, columns in characters), fixes: [label] (the wrench; empty when the font changed since)}]}",
            has_doc,
            list_errors
        ),
        cmd!(
            "font.features.applyFix",
            "Fix",
            [],
            None,
            "{error: id (from font.features.listErrors), fix?: index into its `fixes` (default 0)} makes the suggested correction: the font's spelling of a glyph or class name written in another case, a missing semicolon, or a class entry moved before the class that uses it. Refused when the font changed since the compile, when the error has no fix, or when the code is generated automatically → {label, entry: {kind, name, index}, code?: the entry's new code}",
            has_doc,
            apply_fix
        ),
        cmd!(
            query "font.features.jumpToError",
            "Jump to Error",
            [],
            None,
            "{kind: \"prefix\"|\"class\"|\"feature\", name: string, index?: position in the group, error?: id (default: the entry's first error, else its first warning)} where the editor goes when the error icon of an entry is clicked in the sidebar → {id, severity, message, entry: {kind, name, index}, range: [start, end], line, column, fixes}",
            has_doc,
            jump_to_error
        ),
        cmd!(
            query "font.features.showErrorDetails",
            "Show Error Details",
            [],
            None,
            "{error: id, kind?, name?, index? (checked against the error's entry when given)} the expanded description of an error, shown when its icon in the editor is clicked → {id, severity, message, entry, range, line, column, fixes, details: the message with its place and the line of code marked}",
            has_doc,
            show_error_details
        ),
        cmd!(
            query "font.features.evaluateToken",
            "Evaluate Token",
            [],
            None,
            "{token: string (`$name`, `${…}`, `$[…]`), master?: id|name (default: the first master; the Features tab passes the master of the active Edit View tab), instance?: id (not available yet)} what one feature-code token evaluates to → {token, master, kind: \"number\", value: integer} or {token, master, kind: \"glyphs\", glyphs: [name] (in the font's order)}",
            has_doc,
            evaluate_token
        ),
        cmd!(
            query "font.features.complete",
            "Complete Feature Code",
            [],
            None,
            "{kind: \"prefix\"|\"class\"|\"feature\", name: string, index?: position in the group, position: byte offset in the code (on a character boundary), code?: string (the editor's text, when it differs from the stored code)} the names that fit at a position: glyphs in rules, classes after @, lookups after `lookup`, number values after $ and inside ${…}, feature tags after `feature`, keywords at the start of a statement; narrowed by the partial word before the position, at most 1,000 → {completions: [{label, kind: \"glyph\"|\"class\"|\"lookup\"|\"keyword\"|\"tag\"|\"number\", replace: [start, end] (the bytes the label replaces)}]}",
            has_doc,
            complete
        ),
        cmd!(
            query "font.features.previewName",
            "Preview Glyph or Class",
            [],
            None,
            "{name: string (a glyph name, or a class name with or without @), master?: id|name (for tokens in class code; default: the first master)} what the popover of an Alt-clicked name shows → {kind: \"glyph\", name, unicodes: [hex], export: bool} or {kind: \"class\", name, glyphs: [name], unknown: [name] (members the font does not have)}",
            has_doc,
            preview_name
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

fn parse_kind(p: &Params, value: &str) -> Result<EntryKind> {
    match value {
        "prefix" => Ok(EntryKind::Prefix),
        "class" => Ok(EntryKind::Class),
        "feature" => Ok(EntryKind::Feature),
        other => Err(p.bad(format!("`kind` must be \"prefix\", \"class\" or \"feature\", got \"{other}\""))),
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

fn snippet_of(f: &Features, entry: EntryRef) -> Option<&Snippet> {
    group(f, entry.kind).get(entry.index)
}

/// The entry the parameters `kind`, `name` and `index?` name; `None` when `kind` is absent and
/// `required` is false.
fn entry_param(p: &Params, f: &Features, required: bool) -> Result<Option<EntryRef>> {
    let kind = match p.opt_str("kind")? {
        Some(k) => parse_kind(p, k)?,
        None if required => return Err(p.bad("`kind` is required")),
        None => {
            if p.get("name").is_some() || p.get("index").is_some() {
                return Err(p.bad("`name` and `index` need `kind`"));
            }
            return Ok(None);
        }
    };
    let name = p.str("name")?;
    let name = if kind == EntryKind::Class { name.strip_prefix('@').unwrap_or(name) } else { name };
    let what = kind_name(kind);
    let entries = group(f, kind);
    let index = match p.opt_int("index", 0, fontcraft_features::MAX_ENTRIES as i64)? {
        Some(i) => {
            let i = i as usize;
            let at = entries.get(i).ok_or_else(|| p.bad(format!("the font has no {what} at index {i}")))?;
            if at.name != name {
                return Err(p.bad(format!("the {what} at index {i} is `{}`, not `{name}`", at.name)));
            }
            i
        }
        None => entries.iter().position(|e| e.name == name).ok_or_else(|| p.bad(format!("the font has no {what} `{name}`")))?,
    };
    Ok(Some(EntryRef { kind, index }))
}

fn entry_json(f: &Features, entry: EntryRef) -> Value {
    let name = snippet_of(f, entry).map(|e| e.name.clone()).unwrap_or_default();
    json!({ "kind": kind_name(entry.kind), "name": name, "index": entry.index })
}

fn master_id(p: &Params, font: &Font, value: Option<&str>) -> Result<String> {
    match value {
        None => Ok(font.default_master()?.id.clone()),
        Some(v) => font
            .masters
            .iter()
            .find(|m| m.id == v)
            .or_else(|| font.masters.iter().find(|m| m.name == v))
            .map(|m| m.id.clone())
            .ok_or_else(|| p.bad(format!("`master`: the font has no master `{v}`"))),
    }
}

fn feature_error(p: &Params, e: FeatureError) -> EngineError {
    match e {
        FeatureError::NotYetImplemented(what) => EngineError::Unsupported(what.to_string()),
        FeatureError::Unsupported(what) => EngineError::Unsupported(what),
        other => p.bad(other.to_string()),
    }
}

/// The preview font of `master`, or `None` with the reason in `message`.
fn build_preview(font: &Font, master: &str, message: &mut Option<String>) -> Option<Arc<PreviewFont>> {
    match preview::layout_font(font, master, PreviewKind::GsubOnly) {
        Ok(built) => Some(Arc::new(built)),
        Err(e) => {
            *message = Some(e.to_string());
            None
        }
    }
}

/// The file names `text` includes (`include(name)`), unchecked: a match in a comment only adds a
/// name to the cache key. A name ends at `)` and cannot span a line; the search goes on after the
/// scanned name, so each byte of `text` is scanned a bounded number of times.
fn include_names(text: &str) -> impl Iterator<Item = &str> {
    const WORD: &str = "include";
    let mut from = 0usize;
    std::iter::from_fn(move || {
        loop {
            let at = from + text.get(from..)?.find(WORD)?;
            let after = at + WORD.len();
            from = after;
            let tail = text.get(after..)?;
            let Some(rest) = tail.trim_start().strip_prefix('(') else {
                continue;
            };
            let start = text.len() - rest.len();
            let mut end = None;
            let mut scanned = 0;
            for (i, c) in rest.char_indices().take(MAX_INCLUDE_NAME_BYTES) {
                scanned = i + c.len_utf8();
                if c == ')' {
                    end = Some(i);
                    break;
                }
                if c == '\n' {
                    break;
                }
            }
            from = start + scanned;
            if let Some(name) = end.and_then(|end| rest.get(..end)).map(str::trim).filter(|name| !name.is_empty()) {
                return Some(name);
            }
        }
    })
}

/// Adds the modification time and length of each file the code includes from `dir` to
/// `hasher`, following the includes of included files. An include is looked up both in `dir`
/// and next to the file that names it; a file outside `dir` is skipped, as the compiler refuses
/// it. Returns `false` when there were more files or bytes than [`MAX_KEYED_INCLUDES`] and
/// [`MAX_KEYED_INCLUDE_BYTES`] allow, so the compile is not reused.
fn hash_included_files(files: &fontcraft_fea::VirtualFiles, dir: &std::path::Path, hasher: &mut DefaultHasher) -> bool {
    use std::io::Read;
    let Ok(root) = dir.canonicalize() else {
        // No folder, nothing to include: the compile fails the same way each time.
        return true;
    };
    let parent = |p: &std::path::Path| p.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| root.clone());
    let mut pending: Vec<(PathBuf, Arc<str>)> = files.iter().map(|(path, text)| (parent(&root.join(path)), Arc::from(text))).collect();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut read_bytes = 0usize;
    while let Some((from_dir, text)) = pending.pop() {
        for name in include_names(&text) {
            for candidate in [root.join(name), from_dir.join(name)] {
                if !seen.insert(candidate.clone()) {
                    continue;
                }
                if seen.len() > MAX_KEYED_INCLUDES {
                    return false;
                }
                candidate.hash(hasher);
                let Some(real) = candidate.canonicalize().ok().filter(|real| real.starts_with(&root)) else {
                    false.hash(hasher);
                    continue;
                };
                let Ok(meta) = std::fs::metadata(&real) else {
                    false.hash(hasher);
                    continue;
                };
                meta.len().hash(hasher);
                meta.modified().ok().hash(hasher);
                if !meta.is_file() {
                    continue;
                }
                // The length on record can be wrong (special files report zero), so the read
                // itself is capped at what the budget has left.
                let remaining = MAX_KEYED_INCLUDE_BYTES.saturating_sub(read_bytes);
                let limit = u64::try_from(remaining).unwrap_or(u64::MAX).saturating_add(1);
                let mut bytes = Vec::new();
                let Ok(file) = std::fs::File::open(&real) else {
                    continue;
                };
                // A file that cannot be read fails the compile the same way each time.
                let readable = file.take(limit).read_to_end(&mut bytes).is_ok();
                read_bytes = read_bytes.saturating_add(bytes.len());
                if read_bytes > MAX_KEYED_INCLUDE_BYTES {
                    return false;
                }
                if let Some(included) = readable.then(|| String::from_utf8(bytes).ok()).flatten() {
                    pending.push((parent(&real), Arc::from(included)));
                }
            }
        }
    }
    true
}

// ── Compile ─────────────────────────────────────────────────────────────────────────────────────

/// The compiled font's glyphs: `.notdef` first, then the exporting glyphs in the font's order.
fn glyph_order(font: &Font) -> Vec<String> {
    let mut names = vec![".notdef".to_string()];
    names.extend(font.glyphs.as_arcs().iter().filter(|g| g.export && g.name != ".notdef").map(|g| g.name.clone()));
    names
}

/// A diagnostic about no entry.
fn request_diagnostic(message: impl Into<String>) -> EntryDiagnostic {
    EntryDiagnostic { place: None, diagnostic: Diagnostic::request(Severity::Error, message) }
}

fn compile(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.compile", raw)?;
    let force = p.bool_or("force", false)?;
    let doc = s.doc()?;
    let (doc_id, revision) = (doc.id, doc.revision);
    let include_dir: Option<PathBuf> = doc.path.as_ref().and_then(|path| path.parent()).map(|d| d.to_path_buf());
    let data = s.glyph_data()?;
    let font = s.font()?;
    let master = master_id(&p, font, p.opt_str("master")?)?;
    let values = MasterValues::new(font, &data, &master).map_err(|e| feature_error(&p, e))?;
    let previous = s.feature_state_of(doc_id);

    let mut cached = false;
    let mut preview_message = None;
    let state = match assemble(font, &font.features, &Target::Master(master.clone()), &values) {
        Err(FeatureError::Code(diagnostics)) => {
            FeatureState { revision, master: master.clone(), preview: None, features: Vec::new(), diagnostics, source_hash: 0 }
        }
        Err(e @ (FeatureError::NotYetImplemented(_) | FeatureError::Unsupported(_))) => return Err(feature_error(&p, e)),
        Err(other) => FeatureState {
            revision,
            master: master.clone(),
            preview: None,
            features: Vec::new(),
            diagnostics: vec![request_diagnostic(other.to_string())],
            source_hash: 0,
        },
        Ok(assembly) => {
            let names = glyph_order(font);
            let mut files = assembly.files;
            if let Some(dir) = &include_dir {
                files = files.with_include_dir(dir.clone());
            }
            let mut hasher = DefaultHasher::new();
            master.hash(&mut hasher);
            include_dir.hash(&mut hasher);
            for (path, text) in files.iter() {
                path.hash(&mut hasher);
                text.hash(&mut hasher);
            }
            for g in font.glyphs.as_arcs().iter().filter(|g| g.export) {
                g.name.hash(&mut hasher);
                g.unicodes.hash(&mut hasher);
            }
            let includes_keyed = include_dir.as_deref().is_none_or(|dir| hash_included_files(&files, dir, &mut hasher));
            // Zero marks a state that compiled nothing.
            let hash = hasher.finish().max(1);
            match previous.filter(|prev| !force && includes_keyed && prev.source_hash == hash && prev.master == master) {
                Some(prev) => {
                    cached = true;
                    // The stored code may differ where the assembled code does not (a token
                    // replaced by its value), so the places are mapped again.
                    let diagnostics: Vec<EntryDiagnostic> = prev.diagnostics.iter().map(|d| assembly.map.map(d.diagnostic.clone())).collect();
                    // The compiled features depend only on the hashed code and glyph order; the
                    // preview font also holds outlines and widths, so it is reused only from the
                    // same revision of the document.
                    let preview = if diagnostics.iter().any(|d| d.diagnostic.is_error()) {
                        None
                    } else if prev.revision == revision {
                        prev.preview.clone()
                    } else {
                        build_preview(font, &master, &mut preview_message)
                    };
                    FeatureState { revision, diagnostics, preview, ..prev.as_ref().clone() }
                }
                None => {
                    let (features, diagnostics) = match GlyphOrder::new(names) {
                        Err(d) => (Vec::new(), vec![EntryDiagnostic { place: None, diagnostic: d }]),
                        Ok(order) => {
                            let request = CompileRequest::new(&files, &order);
                            match fontcraft_fea::compile(&request) {
                                Ok(compiled) => (compiled.features, compiled.warnings.into_iter().map(|d| assembly.map.map(d)).collect()),
                                Err(found) => (Vec::new(), found.into_iter().map(|d| assembly.map.map(d)).collect()),
                            }
                        }
                    };
                    let errors = diagnostics.iter().any(|d: &EntryDiagnostic| d.diagnostic.is_error());
                    let preview = if errors { None } else { build_preview(font, &master, &mut preview_message) };
                    FeatureState { revision, master: master.clone(), preview, features, diagnostics, source_hash: hash }
                }
            }
        }
    };

    let fixes = fix_labels(font, &state, true);
    let diagnostics: Vec<Value> = diagnostics_json(font, &state, None, &fixes);
    let features: Vec<Value> = state
        .features
        .iter()
        .map(|f| {
            json!({
                "tag": f.tag.to_string(),
                "table": match f.table { LayoutTable::Gsub => "GSUB", LayoutTable::Gpos => "GPOS" },
                "languageSystems": f.language_systems.iter().map(|(s, l)| json!([s.to_string(), l.to_string()])).collect::<Vec<_>>(),
                "lookups": f.lookups,
            })
        })
        .collect();
    let (errors, warnings) = counts(&state);
    let mut out = json!({
        "master": master,
        "ok": errors == 0,
        "cached": cached,
        "errors": errors,
        "warnings": warnings,
        "features": features,
        "diagnostics": diagnostics,
        "preview": state.preview.is_some(),
    });
    if let (Some(message), Some(obj)) = (preview_message, out.as_object_mut()) {
        obj.insert("previewMessage".into(), Value::String(message));
    }
    drop(values);
    s.set_feature_state(doc_id, state);
    Ok(out)
}

fn counts(state: &FeatureState) -> (usize, usize) {
    let errors = state.diagnostics.iter().filter(|d| d.diagnostic.is_error()).count();
    (errors, state.diagnostics.len() - errors)
}

// ── Diagnostics as data ─────────────────────────────────────────────────────────────────────────

/// (line, column) from 1 of each of `offsets` in `code`, columns in characters, in one pass.
fn positions(code: &str, offsets: &[usize]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..offsets.len()).collect();
    order.sort_by_key(|&k| offsets.get(k).copied().unwrap_or(0));
    let mut out = vec![(1usize, 1usize); offsets.len()];
    let (mut line, mut column) = (1usize, 1usize);
    let mut chars = code.char_indices().peekable();
    for k in order {
        let target = offsets.get(k).copied().unwrap_or(0);
        while let Some(&(at, c)) = chars.peek() {
            if at >= target {
                break;
            }
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            chars.next();
        }
        if let Some(slot) = out.get_mut(k) {
            *slot = (line, column);
        }
    }
    out
}

/// The line and column of the start of each diagnostic that has a place, by id.
fn diagnostic_positions(font: &Font, state: &FeatureState, only: Option<&[usize]>) -> BTreeMap<usize, (usize, usize)> {
    let mut by_entry: BTreeMap<EntryRef, Vec<(usize, usize)>> = BTreeMap::new();
    for (id, d) in state.diagnostics.iter().enumerate() {
        if only.is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        if let Some(place) = &d.place {
            by_entry.entry(place.entry).or_default().push((id, place.range.start));
        }
    }
    let mut out = BTreeMap::new();
    for (entry, items) in by_entry {
        let Some(snippet) = snippet_of(&font.features, entry) else { continue };
        let offsets: Vec<usize> = items.iter().map(|(_, at)| *at).collect();
        for ((id, _), position) in items.iter().zip(positions(&snippet.code, &offsets)) {
            out.insert(*id, position);
        }
    }
    out
}

/// The names fixes and completions are drawn from: every glyph, the class entries and the
/// classes and lookups the code defines (reading at most [`MAX_NAME_SCAN_BYTES`] of code), the
/// number values. `replace` stands in for the stored code of one entry.
struct Names {
    glyphs: Vec<String>,
    definitions: Definitions,
    numbers: Vec<String>,
}

impl Names {
    fn of(font: &Font, replace: Option<(EntryRef, &str)>) -> Self {
        let glyphs = font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
        let mut definitions = Definitions::default();
        for class in &font.features.classes {
            definitions.add_class(&class.name);
        }
        let mut budget = MAX_NAME_SCAN_BYTES;
        let f = &font.features;
        for (kind, list) in [(EntryKind::Prefix, &f.prefixes), (EntryKind::Feature, &f.features)] {
            for (index, e) in list.iter().enumerate() {
                let code = match replace {
                    Some((entry, text)) if entry == (EntryRef { kind, index }) => text,
                    _ => e.code.as_str(),
                };
                if code.is_empty() || code.len() > budget {
                    continue;
                }
                budget -= code.len();
                edit::definitions(code, &mut definitions);
            }
        }
        let numbers = font.numbers.iter().map(|n| n.name.clone()).collect();
        Self { glyphs, definitions, numbers }
    }

    fn completion(&self) -> CompletionNames<'_> {
        CompletionNames { glyphs: &self.glyphs, classes: self.definitions.classes(), lookups: self.definitions.lookups(), numbers: &self.numbers }
    }
}

/// A correction for one diagnostic.
enum Remedy {
    /// Edits to the code of the diagnostic's entry.
    Code(EntryRef, Fix),
    /// The class entry at `from` moved to `to` (before the class that uses it).
    MoveClass { from: usize, to: usize, label: String },
}

impl Remedy {
    fn label(&self) -> &str {
        match self {
            Remedy::Code(_, fix) => &fix.label,
            Remedy::MoveClass { label, .. } => label,
        }
    }
}

/// The corrections offered for `d`.
fn remedies(font: &Font, d: &EntryDiagnostic, names: &Names) -> Vec<Remedy> {
    let Some(place) = &d.place else { return Vec::new() };
    let Some(snippet) = snippet_of(&font.features, place.entry) else { return Vec::new() };
    let mut out: Vec<Remedy> = edit::fixes(&snippet.code, d, &names.completion()).into_iter().map(|f| Remedy::Code(place.entry, f)).collect();
    // A class used by a class entry above its own: classes are compiled in sidebar order.
    if place.entry.kind == EntryKind::Class && d.diagnostic.is_error() && d.message().to_ascii_lowercase().contains("undefined glyph class") {
        let used = snippet.code.get(place.range.clone()).unwrap_or("").trim_start_matches('@');
        let later = font.features.classes.iter().enumerate().find(|(i, c)| *i > place.entry.index && c.name == used && !c.disabled);
        if let Some((from, class)) = later {
            out.push(Remedy::MoveClass { from, to: place.entry.index, label: format!("Move the class @{} before @{}", class.name, snippet.name) });
        }
    }
    out.truncate(MAX_FIXES);
    out
}

/// The fix labels of each diagnostic that has fixes, by id; none when the font changed since
/// the compile (`current` false), as the places no longer fit the code.
fn fix_labels(font: &Font, state: &FeatureState, current: bool) -> BTreeMap<usize, Vec<String>> {
    let mut out = BTreeMap::new();
    if !current || !state.diagnostics.iter().any(|d| d.diagnostic.is_error() && d.place.is_some()) {
        return out;
    }
    let names = Names::of(font, None);
    for (id, d) in state.diagnostics.iter().enumerate() {
        let labels: Vec<String> = remedies(font, d, &names).iter().map(|r| r.label().to_string()).collect();
        if !labels.is_empty() {
            out.insert(id, labels);
        }
    }
    out
}

fn diagnostic_json(font: &Font, id: usize, d: &EntryDiagnostic, position: Option<(usize, usize)>, fixes: &[String]) -> Value {
    let severity = if d.diagnostic.is_error() { "error" } else { "warning" };
    let (entry, range) = match &d.place {
        Some(place) => (entry_json(&font.features, place.entry), json!([place.range.start, place.range.end])),
        None => (Value::Null, Value::Null),
    };
    let (line, column) = match position {
        Some((l, c)) => (json!(l), json!(c)),
        None => (Value::Null, Value::Null),
    };
    json!({
        "id": id,
        "severity": severity,
        "message": d.message(),
        "entry": entry,
        "range": range,
        "line": line,
        "column": column,
        "fixes": fixes,
    })
}

/// The diagnostics of `state` as data, those of `entry` only when given.
fn diagnostics_json(font: &Font, state: &FeatureState, entry: Option<EntryRef>, fixes: &BTreeMap<usize, Vec<String>>) -> Vec<Value> {
    let ids: Vec<usize> = state
        .diagnostics
        .iter()
        .enumerate()
        .filter(|(_, d)| entry.is_none() || d.place.as_ref().map(|p| p.entry) == entry)
        .map(|(id, _)| id)
        .collect();
    let positions = diagnostic_positions(font, state, Some(&ids));
    ids.iter()
        .filter_map(|&id| {
            let d = state.diagnostics.get(id)?;
            Some(diagnostic_json(font, id, d, positions.get(&id).copied(), fixes.get(&id).map(Vec::as_slice).unwrap_or(&[])))
        })
        .collect()
}

/// The last compile, and whether the font is as it read it.
fn state_of(s: &Session, p: &Params) -> Result<(Arc<FeatureState>, bool)> {
    let doc = s.doc()?;
    let state = s.feature_state_of(doc.id).ok_or_else(|| p.bad("the feature code has not been compiled yet"))?;
    let current = state.is_current(doc);
    Ok((state, current))
}

fn error_param(p: &Params, state: &FeatureState) -> Result<usize> {
    let id = p.opt_int("error", 0, MAX_DIAGNOSTICS as i64)?.ok_or_else(|| p.bad("`error` is required"))? as usize;
    if id >= state.diagnostics.len() {
        return Err(p.bad(format!("the last compile has no error {id}")));
    }
    Ok(id)
}

fn list_errors(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.listErrors", raw)?;
    let font = s.font()?;
    let entry = entry_param(&p, &font.features, false)?;
    let Some(state) = s.feature_state() else {
        return Ok(json!({ "compiled": false, "current": false, "errors": 0, "warnings": 0, "diagnostics": [] }));
    };
    let current = state.is_current(s.doc()?);
    let fixes = fix_labels(font, &state, current);
    let diagnostics = diagnostics_json(font, &state, entry, &fixes);
    let (errors, warnings) = counts(&state);
    Ok(json!({
        "compiled": true,
        "current": current,
        "master": state.master,
        "errors": errors,
        "warnings": warnings,
        "diagnostics": diagnostics,
    }))
}

/// One diagnostic as data, with its position and fixes.
fn one_json(font: &Font, state: &FeatureState, id: usize, current: bool) -> Value {
    let Some(d) = state.diagnostics.get(id) else { return Value::Null };
    let position = diagnostic_positions(font, state, Some(std::slice::from_ref(&id))).get(&id).copied();
    let labels: Vec<String> =
        if current { remedies(font, d, &Names::of(font, None)).iter().map(|r| r.label().to_string()).collect() } else { Vec::new() };
    diagnostic_json(font, id, d, position, &labels)
}

fn jump_to_error(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.jumpToError", raw)?;
    let (state, current) = state_of(s, &p)?;
    if !current {
        return Err(p.bad("the font changed since the last compile: compile again to find the errors"));
    }
    let font = s.font()?;
    let Some(entry) = entry_param(&p, &font.features, true)? else { return Err(p.bad("`kind` is required")) };
    let in_entry = |d: &EntryDiagnostic| d.place.as_ref().is_some_and(|place| place.entry == entry);
    let id = match p.get("error") {
        Some(_) => {
            let id = error_param(&p, &state)?;
            if !state.diagnostics.get(id).is_some_and(in_entry) {
                return Err(p.bad(format!("error {id} is not in this {}", kind_name(entry.kind))));
            }
            id
        }
        None => state
            .diagnostics
            .iter()
            .position(|d| in_entry(d) && d.diagnostic.is_error())
            .or_else(|| state.diagnostics.iter().position(in_entry))
            .ok_or_else(|| p.bad(format!("the last compile found nothing in this {}", kind_name(entry.kind))))?,
    };
    Ok(one_json(font, &state, id, current))
}

/// The message with its place and the line of code marked under it.
fn details(font: &Font, d: &EntryDiagnostic, position: Option<(usize, usize)>) -> String {
    let severity = if d.diagnostic.is_error() { "Error" } else { "Warning" };
    let Some(place) = &d.place else {
        return match d.diagnostic.file.as_str() {
            "" => format!("{severity}: {}", d.message()),
            file => format!("{severity} in {file}: {}", d.message()),
        };
    };
    let Some(snippet) = snippet_of(&font.features, place.entry) else { return format!("{severity}: {}", d.message()) };
    let what = match place.entry.kind {
        EntryKind::Prefix => format!("prefix `{}`", snippet.name),
        EntryKind::Class => format!("class @{}", snippet.name),
        EntryKind::Feature => format!("feature `{}`", snippet.name),
    };
    let (line, column) = position.unwrap_or((1, 1));
    let mut text = format!("{severity} in {what}, line {line}, column {column}: {}", d.message());
    let code = snippet.code.as_str();
    let start = place.range.start.min(code.len());
    let line_start = code.get(..start).and_then(|b| b.rfind('\n')).map_or(0, |i| i + 1);
    let line_end = code.get(start..).and_then(|a| a.find('\n')).map_or(code.len(), |i| start + i);
    if let Some(quoted) = code.get(line_start..line_end) {
        let shown: String = quoted.chars().take(MAX_QUOTED_CHARS).collect();
        let lead = column.saturating_sub(1).min(MAX_QUOTED_CHARS);
        let width = code.get(start..place.range.end.min(line_end).max(start)).map_or(1, |s| s.chars().count().max(1));
        let width = width.min(MAX_QUOTED_CHARS.saturating_sub(lead).max(1));
        text.push_str(&format!("\n\n{shown}\n{}{}", " ".repeat(lead), "^".repeat(width)));
    }
    text
}

fn show_error_details(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.showErrorDetails", raw)?;
    let (state, current) = state_of(s, &p)?;
    let id = error_param(&p, &state)?;
    let font = s.font()?;
    let d = state.diagnostics.get(id).ok_or_else(|| p.bad(format!("the last compile has no error {id}")))?;
    if let Some(entry) = entry_param(&p, &font.features, false)?
        && d.place.as_ref().map(|place| place.entry) != Some(entry)
    {
        return Err(p.bad(format!("error {id} is not in this {}", kind_name(entry.kind))));
    }
    let mut out = one_json(font, &state, id, current);
    let position = if current { diagnostic_positions(font, &state, Some(std::slice::from_ref(&id))).get(&id).copied() } else { None };
    let text = details(font, d, position);
    if let Some(obj) = out.as_object_mut() {
        obj.insert("details".into(), Value::String(text));
        obj.insert("current".into(), Value::Bool(current));
    }
    Ok(out)
}

fn apply_fix(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.applyFix", raw)?;
    let (state, current) = state_of(s, &p)?;
    let id = error_param(&p, &state)?;
    if !current {
        return Err(p.bad("the font changed since the last compile: compile again before fixing"));
    }
    let which = p.opt_int("fix", 0, MAX_FIXES as i64)?.unwrap_or(0) as usize;
    let font = s.font()?;
    let d = state.diagnostics.get(id).ok_or_else(|| p.bad(format!("the last compile has no error {id}")))?;
    let mut offered = remedies(font, d, &Names::of(font, None));
    if offered.is_empty() {
        return Err(p.bad(format!("error {id} has no fix")));
    }
    if which >= offered.len() {
        return Err(p.bad(format!("error {id} has {} fixes; there is no fix {which}", offered.len())));
    }
    let remedy = offered.swap_remove(which);
    let label = remedy.label().to_string();
    match remedy {
        Remedy::Code(entry, fix) => {
            let snippet = snippet_of(&font.features, entry).ok_or_else(|| p.bad("the entry is gone"))?;
            if snippet.automatic {
                return Err(p.bad(format!(
                    "the {} `{}` is generated automatically: turn off Generate Feature Automatically to edit its code",
                    kind_name(entry.kind),
                    snippet.name
                )));
            }
            let code = fix.apply(&snippet.code).ok_or_else(|| p.bad("the fix no longer fits the code"))?;
            if code.len() > MAX_ENTRY_BYTES {
                return Err(p.bad(format!("the fixed code would be longer than the {MAX_ENTRY_BYTES} bytes one entry may hold")));
            }
            let new_code = code.clone();
            let command = p.command();
            s.edit("Fix Feature Code", |font| {
                let target = group_mut(Arc::make_mut(&mut font.features), entry.kind)
                    .get_mut(entry.index)
                    .ok_or_else(|| EngineError::BadParams { command: command.into(), message: "the entry is gone".into() })?;
                target.code = new_code;
                Ok(())
            })?;
            Ok(json!({ "label": label, "entry": entry_json(&s.font()?.features, entry), "code": code }))
        }
        Remedy::MoveClass { from, to, .. } => {
            let count = font.features.classes.len();
            if from >= count || to >= count || to > from {
                return Err(p.bad("the fix no longer fits the classes"));
            }
            let command = p.command();
            s.edit("Fix Feature Code", |font| {
                let classes = &mut Arc::make_mut(&mut font.features).classes;
                if from >= classes.len() || to > from {
                    return Err(EngineError::BadParams { command: command.into(), message: "the fix no longer fits the classes".into() });
                }
                let moved = classes.remove(from);
                classes.insert(to, moved);
                Ok(())
            })?;
            Ok(json!({ "label": label, "entry": entry_json(&s.font()?.features, EntryRef { kind: EntryKind::Class, index: to }) }))
        }
    }
}

// ── Editor queries ──────────────────────────────────────────────────────────────────────────────

fn evaluate_token(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.evaluateToken", raw)?;
    if p.get("instance").is_some() {
        return Err(EngineError::Unsupported("evaluating a token for an instance".into()));
    }
    let token = p.str("token")?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let master = master_id(&p, font, p.opt_str("master")?)?;
    let values = MasterValues::new(font, &data, &master).map_err(|e| feature_error(&p, e))?;
    match tokens::evaluate(token, &values).map_err(|message| p.bad(message))? {
        TokenValue::Number(value) => Ok(json!({ "token": token.trim(), "master": master, "kind": "number", "value": value })),
        TokenValue::Glyphs(glyphs) => Ok(json!({ "token": token.trim(), "master": master, "kind": "glyphs", "glyphs": glyphs })),
    }
}

fn completion_kind(kind: CompletionKind) -> &'static str {
    match kind {
        CompletionKind::Glyph => "glyph",
        CompletionKind::Class => "class",
        CompletionKind::Lookup => "lookup",
        CompletionKind::Keyword => "keyword",
        CompletionKind::Tag => "tag",
        CompletionKind::Number => "number",
    }
}

fn complete(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.complete", raw)?;
    let font = s.font()?;
    let Some(entry) = entry_param(&p, &font.features, true)? else { return Err(p.bad("`kind` is required")) };
    let stored = snippet_of(&font.features, entry).map(|e| e.code.as_str()).unwrap_or("");
    let code = p.opt_str("code")?.unwrap_or(stored);
    if code.len() > MAX_ENTRY_BYTES {
        return Err(p.bad(format!("the code has {} bytes, more than the {MAX_ENTRY_BYTES} one entry may hold", code.len())));
    }
    let position = p.opt_int("position", 0, MAX_ENTRY_BYTES as i64)?.ok_or_else(|| p.bad("`position` is required"))? as usize;
    if code.get(..position).is_none() {
        return Err(p.bad(format!("`position` {position} is not a character boundary of the code ({} bytes)", code.len())));
    }
    let names = Names::of(font, Some((entry, code)));
    let completions: Vec<Value> = edit::complete(code, position, &names.completion())
        .into_iter()
        .map(|c| json!({ "label": c.label, "kind": completion_kind(c.kind), "replace": [c.replace.start, c.replace.end] }))
        .collect();
    Ok(json!({ "completions": completions }))
}

/// Adds the members of class entry `name` to `out`, following classes it names.
struct ClassWalk<'a> {
    /// The class entries by name; the first entry wins where names repeat.
    classes: &'a HashMap<&'a str, &'a Snippet>,
    values: &'a dyn TargetValues,
    glyphs: &'a HashSet<&'a str>,
    seen_classes: HashSet<String>,
    seen_glyphs: HashSet<String>,
    out: Vec<String>,
    unknown: Vec<String>,
}

impl ClassWalk<'_> {
    fn walk(&mut self, name: &str, depth: usize) {
        if depth > MAX_CLASS_DEPTH || !self.seen_classes.insert(name.to_string()) {
            return;
        }
        let Some(entry) = self.classes.get(name) else {
            if self.unknown.len() < MAX_UNKNOWN {
                self.unknown.push(format!("@{name}"));
            }
            return;
        };
        let expanded = tokens::expand(&entry.code, self.values, EntryKind::Class).map(|(text, _)| text).unwrap_or_else(|_| entry.code.clone());
        for word in expanded.split(|c: char| c.is_whitespace() || c == '[' || c == ']') {
            if word.is_empty() || word.starts_with('#') {
                continue;
            }
            if let Some(class) = word.strip_prefix('@') {
                self.walk(class, depth + 1);
            } else {
                let glyph = word.strip_prefix('\\').unwrap_or(word);
                if self.glyphs.contains(glyph) {
                    if self.out.len() < MAX_GLYPHS && self.seen_glyphs.insert(glyph.to_string()) {
                        self.out.push(glyph.to_string());
                    }
                } else if self.unknown.len() < MAX_UNKNOWN {
                    self.unknown.push(glyph.to_string());
                }
            }
        }
    }
}

fn preview_name(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.previewName", raw)?;
    let name = p.str("name")?.trim();
    let data = s.glyph_data()?;
    let font = s.font()?;
    let mut classes: HashMap<&str, &Snippet> = HashMap::new();
    for c in &font.features.classes {
        classes.entry(c.name.as_str()).or_insert(c);
    }
    let is_class_entry = |n: &str| classes.contains_key(n);
    let class = match name.strip_prefix('@') {
        Some(rest) => Some(rest),
        None if font.glyphs.get(name).is_none() && is_class_entry(name) => Some(name),
        None => None,
    };
    let Some(class) = class else {
        let glyph = name.strip_prefix('\\').unwrap_or(name);
        let g = font.glyphs.get(glyph).ok_or_else(|| p.bad(format!("the font has no glyph or class `{name}`")))?;
        let unicodes: Vec<String> = g.unicodes.iter().map(|u| format!("{u:04X}")).collect();
        return Ok(json!({ "kind": "glyph", "name": g.name, "unicodes": unicodes, "export": g.export }));
    };
    if !is_class_entry(class) {
        return Err(p.bad(format!("the font has no class `@{class}`")));
    }
    let master = master_id(&p, font, p.opt_str("master")?)?;
    let values = MasterValues::new(font, &data, &master).map_err(|e| feature_error(&p, e))?;
    let glyphs: HashSet<&str> = font.glyphs.as_arcs().iter().map(|g| g.name.as_str()).collect();
    let mut walk = ClassWalk {
        classes: &classes,
        values: &values,
        glyphs: &glyphs,
        seen_classes: HashSet::new(),
        seen_glyphs: HashSet::new(),
        out: Vec::new(),
        unknown: Vec::new(),
    };
    walk.walk(class, 0);
    Ok(json!({ "kind": "class", "name": class, "glyphs": walk.out, "unknown": walk.unknown }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_names_end_at_a_parenthesis_on_the_same_line_and_scan_in_linear_time() {
        let names: Vec<&str> = include_names("include(a.fea); include ( b.fea );\ninclude(c\n.fea);").collect();
        assert_eq!(names, ["a.fea", "b.fea"]);
        let hostile = "include(".repeat(2_000_000);
        let started = std::time::Instant::now();
        assert_eq!(include_names(&hostile).count(), 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }
}
