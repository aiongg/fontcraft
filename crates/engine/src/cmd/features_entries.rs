//! Disabling, copying and pasting feature entries (Font Info › Features sidebar, M8.29).
//!
//! Commands: `font.features.setDisabled`, `font.features.copyEntries`,
//! `font.features.pasteEntries`. Rows: `spec/inventory/06-font-info.toml`.
//!
//! The engine never touches the system clipboard: Copy returns text and Paste takes it, as
//! `edit.copy` and `edit.paste` do. The text is feature code with one marker line before each
//! entry, so it still reads as a feature file in another app:
//!
//! ```text
//! # entry: {"kind":"class","name":"Upper","notes":"capitals"}
//! @Upper = [
//! A B C
//! ];
//!
//! # entry: {"kind":"feature","name":"liga","automatic":true}
//! feature liga {
//! sub f i by f_i;
//! } liga;
//! ```
//!
//! The marker is `# entry: ` and a one-line JSON object: `kind` and `name` always; `automatic`,
//! `disabled`, `notes` and `labels` (`[{language, name}]`) only when set. A prefix's code follows
//! as it is; a class's sits between `@Name = [` and `];` lines, a feature's between
//! `feature tag {` and `} tag;` lines. A disabled entry's code lines are commented out (`# ` before
//! each line, `#` for an empty one), so the copied text compiles as the font does. Entries are
//! separated by one blank line; the text ends with a line break. Copy writes classes first, then
//! prefixes, then features, the order export assembles them in.
//!
//! Paste reads the marker form only when writing what it read gives the text back byte for byte
//! (line breaks normalised to `\n`). Any other text is read as `file.importFeatures` reads text:
//! the UFO writer's form with its flags, else a split at the top level. The entries are added
//! after the font's, as one undo step, with the import's naming: a class or prefix whose name is
//! taken gets the first free numeric suffix (`Upper2`), and pasted references to a renamed class
//! follow it; a feature keeps its tag.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_features::{EntryKind, EntryRef, MAX_ENTRIES, MAX_ENTRY_BYTES};
use fontcraft_model::{Extra, Features, Localized, Snippet};
use serde::Serialize;
use serde_json::{Value, json};

use super::features::{
    MAX_LABEL_CHARS, MAX_LABELS, MAX_NOTES_BYTES, entry_count, entry_param, group, group_mut, kind_name, message, name_problem, snippet_mut,
};
use super::features_io::{MAX_TEXT_BYTES, add_entries, entries_of};
use super::*;
use crate::params::Params;
use crate::{Result, Session};

/// The start of the line before each copied entry.
const MARKER: &str = "# entry: ";
/// Most bytes of a pasted stylistic-set name's language tag.
const MAX_LANGUAGE_BYTES: usize = 32;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.features.setDisabled",
            "Disable Feature Entries",
            [],
            None,
            "{entries: [{kind: \"prefix\"|\"class\"|\"feature\", name, index?}], disabled: bool} disable or enable the entries, as the sidebar's Disable and Enable do. A disabled entry stays in the font and in the sidebar but is left out of the compiled code and of exported feature files; every entry must exist. One undo step; entries already in that state are left as they are → {entries: [{kind, name, index, disabled}], changed: number of entries whose state changed}",
            has_doc,
            set_disabled
        ),
        cmd!(
            query "font.features.copyEntries",
            "Copy Feature Entries",
            [],
            None,
            "{entries: [{kind, name, index?}]} the entries as clipboard text: feature code with a `# entry:` line before each entry that keeps its kind, name, automatic and disabled flags, notes and stylistic-set names; classes first, then prefixes, then features, each group in its order; a disabled entry's code is commented out. The shell puts `text` on the clipboard. More than 16 MB of text is refused → {text, count}",
            has_doc,
            copy_entries
        ),
        cmd!(
            "font.features.pasteEntries",
            "Paste Feature Entries",
            [],
            None,
            "{text: string (at most 16 MB)} add the entries of clipboard text after the font's, as one undo step. Text copied with font.features.copyEntries gives back each entry with its code, flags, notes and names; other text is read as File › Import › Features reads text. A class or prefix whose name the font already has gets a numeric suffix (Upper2) and the pasted references to a renamed class follow it; a feature keeps its tag. Refused: text with no feature code, an invalid name, more than 4 MB of code in one entry, more entries than a font may hold → {entries: [{kind, name, index}] (the pasted entries, in sidebar order), counts: {prefixes, classes, features}, renamed: [{kind, from, to}], sharedTags}",
            has_doc,
            paste_entries
        ),
    ]
}

/// The entries the parameter `entries` names, each once, in the order given.
fn entry_list(p: &Params, f: &Features) -> Result<Vec<EntryRef>> {
    let list = p.array("entries")?;
    if list.is_empty() {
        return Err(p.bad("`entries` is empty"));
    }
    if list.len() > entry_count(f) {
        return Err(p.bad(format!("`entries` names {} entries, more than the font has", list.len())));
    }
    let mut seen = HashSet::new();
    let mut refs = Vec::new();
    for item in list {
        let ip = Params::new(p.command(), item)?;
        let entry = entry_param(&ip, f).map_err(|e| p.bad(format!("`entries`: {}", message(&e))))?;
        if seen.insert(entry) {
            refs.push(entry);
        }
    }
    Ok(refs)
}

// ── font.features.setDisabled ─────────────────────────────────────────────────────────────────

fn set_disabled(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.setDisabled", raw)?;
    let disabled = match p.get("disabled") {
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`disabled` must be true or false, got {}", crate::params::kind(other)))),
        None => return Err(p.bad("`disabled` is required")),
    };
    let f = &s.font()?.features;
    let refs = entry_list(&p, f)?;
    let mut entries = Vec::with_capacity(refs.len());
    let mut changed = Vec::new();
    for r in &refs {
        if let Some(e) = group(f, r.kind).get(r.index) {
            entries.push(json!({"kind": kind_name(r.kind), "name": e.name, "index": r.index, "disabled": disabled}));
            if e.disabled != disabled {
                changed.push(*r);
            }
        }
    }
    if !changed.is_empty() {
        let command = p.command();
        let label = if disabled { "Disable Feature Entries" } else { "Enable Feature Entries" };
        s.edit(label, |font| {
            let features = Arc::make_mut(&mut font.features);
            for r in &changed {
                snippet_mut(features, *r, command)?.disabled = disabled;
            }
            Ok(())
        })?;
    }
    Ok(json!({ "entries": entries, "changed": changed.len() }))
}

// ── The clipboard text ────────────────────────────────────────────────────────────────────────

/// The marker line's JSON object; serialised in field order, defaults left out.
#[derive(Serialize)]
struct Header<'a> {
    kind: &'static str,
    name: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    automatic: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<Label<'a>>>,
}

#[derive(Serialize)]
struct Label<'a> {
    language: &'a str,
    name: &'a str,
}

/// The marker line's JSON object for one entry. Compact JSON escapes line breaks inside
/// strings, so the object stays on one line.
fn header(kind: EntryKind, e: &Snippet) -> String {
    let labels = e.labels.iter().map(|l| Label { language: &l.language, name: &l.value }).collect::<Vec<_>>();
    let h = Header {
        kind: kind_name(kind),
        name: &e.name,
        automatic: e.automatic,
        disabled: e.disabled,
        notes: (!e.notes.is_empty()).then_some(e.notes.as_str()),
        labels: (!labels.is_empty()).then_some(labels),
    };
    // Strings and booleans always serialise.
    serde_json::to_string(&h).unwrap_or_default()
}

fn commented(code: &str) -> String {
    code.split('\n').map(|l| if l.is_empty() { "#".to_string() } else { format!("# {l}") }).collect::<Vec<_>>().join("\n")
}

/// `code` with the comment marks [`commented`] adds taken off again; `None` when a line lacks them.
fn uncommented(code: &str) -> Option<String> {
    let lines: Option<Vec<&str>> = code.split('\n').map(|l| if l == "#" { Some("") } else { l.strip_prefix("# ") }).collect();
    Some(lines?.join("\n"))
}

/// The clipboard text of the entries, in the order given (see the module doc).
fn write_entries(list: &[(EntryKind, &Snippet)]) -> String {
    let mut out = String::new();
    for (i, (kind, e)) in list.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(MARKER);
        out.push_str(&header(*kind, e));
        out.push('\n');
        let code = if e.disabled { commented(&e.code) } else { e.code.clone() };
        match kind {
            EntryKind::Prefix => out.push_str(&code),
            EntryKind::Class => out.push_str(&format!("@{} = [\n{code}\n];", e.name)),
            EntryKind::Feature => out.push_str(&format!("feature {0} {{\n{code}\n}} {0};", e.name)),
        }
        out.push('\n');
    }
    out
}

/// The kind and the flags, notes and names of a marker line's object; `None` when it is not one
/// [`header`] could have written.
fn parse_header(line: &str) -> Option<(EntryKind, Snippet)> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(line) else { return None };
    let kind = match map.get("kind")?.as_str()? {
        "prefix" => EntryKind::Prefix,
        "class" => EntryKind::Class,
        "feature" => EntryKind::Feature,
        _ => return None,
    };
    let name = map.get("name")?.as_str()?.to_string();
    let flag = |key: &str| match map.get(key) {
        None => Some(false),
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => None,
    };
    let automatic = flag("automatic")?;
    let disabled = flag("disabled")?;
    let notes = match map.get("notes") {
        None => String::new(),
        Some(Value::String(n)) => n.clone(),
        Some(_) => return None,
    };
    let labels = match map.get("labels") {
        None => Vec::new(),
        Some(Value::Array(list)) if list.len() <= MAX_LABELS => list
            .iter()
            .map(|l| {
                Some(Localized {
                    language: l.get("language")?.as_str()?.to_string(),
                    value: l.get("name")?.as_str()?.to_string(),
                    extra: Extra::default(),
                })
            })
            .collect::<Option<Vec<_>>>()?,
        Some(_) => return None,
    };
    Some((kind, Snippet { name, automatic, disabled, notes, labels, ..Snippet::default() }))
}

/// One entry's code from the text after its marker line.
fn parse_body(kind: EntryKind, e: &Snippet, body: &str) -> Option<String> {
    let code = match kind {
        EntryKind::Prefix => body,
        EntryKind::Class => body.strip_prefix(&format!("@{} = [\n", e.name))?.strip_suffix("\n];")?,
        EntryKind::Feature => body.strip_prefix(&format!("feature {} {{\n", e.name))?.strip_suffix(&format!("\n}} {};", e.name))?,
    };
    if e.disabled { uncommented(code) } else { Some(code.to_string()) }
}

/// The entries of text in the marker form, in text order; `None` when the text is not exactly
/// what [`write_entries`] writes. `text` has `\n` line breaks and ends with one.
fn read_entries(text: &str) -> Option<Vec<(EntryKind, Snippet)>> {
    let rest = text.strip_prefix(MARKER)?.strip_suffix('\n')?;
    let mut entries = Vec::new();
    for chunk in rest.split("\n\n# entry: ") {
        if entries.len() >= MAX_ENTRIES {
            return None;
        }
        // An empty entry's line break is the separator's first.
        let (line, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        let (kind, mut e) = parse_header(line)?;
        e.code = parse_body(kind, &e, body)?;
        entries.push((kind, e));
    }
    let exact = {
        let list: Vec<(EntryKind, &Snippet)> = entries.iter().map(|(k, e)| (*k, e)).collect();
        write_entries(&list) == text
    };
    exact.then_some(entries)
}

/// Why a pasted entry cannot go into a font, if it cannot.
fn entry_problem(kind: EntryKind, e: &Snippet) -> Option<String> {
    let what = kind_name(kind);
    if let Some(problem) = name_problem(kind, &e.name) {
        return Some(problem);
    }
    if e.code.len() > MAX_ENTRY_BYTES {
        return Some(format!("the {what} `{}` holds more than the {} MB of code one entry may hold", e.name, MAX_ENTRY_BYTES >> 20));
    }
    if e.notes.len() > MAX_NOTES_BYTES {
        return Some(format!("the notes of the {what} `{}` have more than {MAX_NOTES_BYTES} bytes", e.name));
    }
    let bad_label = e.labels.iter().find(|l| {
        l.language.is_empty()
            || l.language.len() > MAX_LANGUAGE_BYTES
            || l.language.chars().any(char::is_whitespace)
            || l.value.chars().count() > MAX_LABEL_CHARS
    });
    bad_label.map(|l| format!("the {what} `{}` has a name that cannot be kept (language \"{}\")", e.name, l.language))
}

// ── font.features.copyEntries ─────────────────────────────────────────────────────────────────

/// The place of a group in the copied text.
fn copy_rank(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Class => 0,
        EntryKind::Prefix => 1,
        EntryKind::Feature => 2,
    }
}

fn copy_entries(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.copyEntries", raw)?;
    let f = &s.font()?.features;
    let mut refs = entry_list(&p, f)?;
    refs.sort_by_key(|r| (copy_rank(r.kind), r.index));
    let list: Vec<(EntryKind, &Snippet)> = refs.iter().filter_map(|r| group(f, r.kind).get(r.index).map(|e| (r.kind, e))).collect();
    let too_big = || p.bad(format!("the entries hold more than the {} MB of text Paste reads; copy fewer entries", MAX_TEXT_BYTES >> 20));
    // Checked before the text is built, which can only be longer.
    let bytes: usize = list.iter().map(|(_, e)| e.code.len().saturating_add(e.notes.len())).fold(0, usize::saturating_add);
    if bytes > MAX_TEXT_BYTES {
        return Err(too_big());
    }
    let text = write_entries(&list);
    if text.len() > MAX_TEXT_BYTES {
        return Err(too_big());
    }
    Ok(json!({ "text": text, "count": list.len() }))
}

// ── font.features.pasteEntries ────────────────────────────────────────────────────────────────

fn paste_entries(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.pasteEntries", raw)?;
    s.font()?;
    let text = p.str("text")?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(p.bad(format!("`text` is {} bytes long; at most {MAX_TEXT_BYTES} are allowed", text.len())));
    }
    let mut text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let new = match read_entries(&text) {
        Some(entries) => {
            let mut new = Features::default();
            for (kind, e) in entries {
                if let Some(problem) = entry_problem(kind, &e) {
                    return Err(p.bad(problem));
                }
                group_mut(&mut new, kind).push(e);
            }
            new
        }
        None => entries_of(&text).map_err(|e| p.bad(e))?,
    };
    if new.is_empty() {
        return Err(p.bad("the text holds no feature code"));
    }
    if let Some(big) = new.prefixes.iter().chain(&new.classes).chain(&new.features).find(|e| e.code.len() > MAX_ENTRY_BYTES) {
        return Err(p.bad(format!("`{}` holds more than the {} MB of code one entry may hold", big.name, MAX_ENTRY_BYTES >> 20)));
    }
    let count = entry_count(&new);
    let (added, entries) = s.edit("Paste Feature Entries", |font| {
        let features = Arc::make_mut(&mut font.features);
        if entry_count(features).saturating_add(count) > MAX_ENTRIES {
            return Err(p.bad(format!("the font would hold more than {MAX_ENTRIES} prefixes, classes and features")));
        }
        let before = [features.prefixes.len(), features.classes.len(), features.features.len()];
        let added = add_entries(features, new);
        let mut entries = Vec::with_capacity(count);
        for (kind, start) in [EntryKind::Prefix, EntryKind::Class, EntryKind::Feature].into_iter().zip(before) {
            for (index, e) in group(features, kind).iter().enumerate().skip(start) {
                entries.push(json!({"kind": kind_name(kind), "name": e.name, "index": index}));
            }
        }
        Ok((added, entries))
    })?;
    let [prefixes, classes, features] = added.counts;
    Ok(json!({
        "entries": entries,
        "counts": {"prefixes": prefixes, "classes": classes, "features": features},
        "renamed": added.renamed,
        "sharedTags": added.shared_tags,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn snippet(name: &str, code: &str) -> Snippet {
        Snippet { name: name.into(), code: code.into(), ..Snippet::default() }
    }

    /// The text Copy writes reads back as the same entries, odd code included; edited text does not.
    #[test]
    fn copied_text_reads_back_exactly_or_not_at_all() {
        let class = Snippet { notes: "two\nlines".into(), ..snippet("Upper", "A B\nC") };
        let prefix = Snippet { disabled: true, ..snippet("Lookups", "lookup L {\n\n  sub a by b;\n} L;\n") };
        let empty = snippet("Empty", "");
        let feature = Snippet {
            automatic: true,
            labels: vec![Localized { language: "dflt".into(), value: "Alt \"one\"".into(), extra: Extra::default() }],
            ..snippet("ss01", "sub a by a.ss01;")
        };
        let off = Snippet { disabled: true, ..snippet("smcp", "") };
        let list = [
            (EntryKind::Class, &class),
            (EntryKind::Prefix, &prefix),
            (EntryKind::Prefix, &empty),
            (EntryKind::Feature, &feature),
            (EntryKind::Feature, &off),
        ];
        let text = write_entries(&list);
        let read = read_entries(&text).unwrap();
        let expected: Vec<(EntryKind, Snippet)> = list.iter().map(|(k, e)| (*k, (*e).clone())).collect();
        assert_eq!(read, expected);
        assert!(text.contains("# entry: {") && text.contains("feature ss01 {\nsub a by a.ss01;\n} ss01;"), "{text}");
        assert!(text.contains("# lookup L {\n#\n#   sub a by b;"), "disabled code is commented out: {text}");

        assert!(read_entries(&text.replace("sub a by a.ss01;", "sub a by a.ss01;\n# more")).is_some(), "code is free text");
        assert!(read_entries(&text.replace("} ss01;", "} ss02;")).is_none());
        assert!(read_entries(&text.replace("\"automatic\":true", "\"automatic\":false")).is_none());
        assert!(read_entries(&text.replace("#   sub a by b;", "  sub a by b;")).is_none());
        assert!(read_entries("@Upper = [A B];\n").is_none());
    }
}
