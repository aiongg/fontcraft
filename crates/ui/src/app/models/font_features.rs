//! The Features tab's view model: the sidebar groups, the selection, the shown entry with its stylistic-set names and
//! the last compile's diagnostics, and the snippets menu.
//!
//! It is projected only while the Features tab shows: `font.features.listErrors`, `font.features.openSpec` and
//! `font.features.listSnippets` (which reads the snippet folder) run then and not otherwise.

use std::collections::HashSet;

use craft::ui::contract::PlainData;
use fontcraft_engine::features::registry;
use fontcraft_engine::model::{Features, Font, Snippet};
use serde_json::{Value, json};

use super::ProjectCx;
use crate::shell::font_info_tabs::{EntryKey, EntryKind, FindState};

/// Most entries listed per sidebar group.
pub const MAX_SHOWN_ENTRIES: usize = 5_000;
/// Most diagnostics read from the last compile, and most listed under the code.
const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_SHOWN_ERRORS: usize = 50;
/// Most stylistic-set names shown.
const MAX_SHOWN_LABELS: usize = 100;
/// Most snippets kept per menu group, and most groups.
const MAX_MENU_SNIPPETS: usize = 1_000;
/// Most fix labels kept per diagnostic.
const MAX_FIXES: usize = 16;

/// What the Features tab shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FeaturesModel {
    /// The groups in sidebar order: prefixes, classes, features.
    pub groups: Vec<GroupModel>,
    /// The selected entries, resolved against the font; the last shows in the editor.
    pub selected: Vec<EntryKey>,
    pub entry: Option<EntryPane>,
    /// The text of the sidebar's filter field.
    pub filter: String,
    /// The height of the notes field in points, once its divider was dragged.
    pub notes_height: Option<f32>,
    /// The compile the Compile button asked for is under way.
    pub compiling: bool,
    pub snippets: SnippetsModel,
}

/// One sidebar group.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GroupModel {
    pub kind: EntryKind,
    pub entries: Vec<EntryRow>,
    /// Entries past [`MAX_SHOWN_ENTRIES`].
    pub hidden: usize,
}

/// One sidebar entry.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct EntryRow {
    pub name: String,
    pub automatic: bool,
    pub disabled: bool,
    /// The last compile, still current, found an error in it.
    pub error: bool,
    /// A registered feature's name; empty for the others.
    pub title: String,
    /// The filter leaves it out of the sidebar.
    pub filtered: bool,
}

/// The entry the editor shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct EntryPane {
    pub key: EntryKey,
    pub code: String,
    pub notes: String,
    pub automatic: bool,
    /// A feature ss01 to ss20, whose names the tab edits.
    pub stylistic_set: bool,
    /// The set's names as (language, name).
    pub labels: Vec<(String, String)>,
    /// The feature's section of the OpenType specification.
    pub spec_url: Option<String>,
    pub diagnostics: Vec<DiagnosticModel>,
    /// The last compile describes the font as it is (`font.features.listErrors`'s `current`): `diagnostics` hold for
    /// the stored code.
    pub compiled: bool,
    /// The code as typed that the app holds for this entry (`FeaturesView::draft`).
    pub draft: Option<String>,
    /// The check of a draft of this entry (`font.features.check`): the code it checked and its diagnostics.
    pub check: Option<(String, Vec<DiagnosticModel>)>,
    /// The answer to the code editor's completion question about this entry.
    pub completion: Option<CompletionModel>,
    /// Where the code editor puts the caret, in characters (after a snippet went in).
    pub caret: Option<usize>,
    /// The place the code editor selects and scrolls to, in characters (an error jumped to).
    pub jump: Option<(usize, usize)>,
    /// The preview of the name Alt-clicked in this entry's code.
    pub preview: Option<PreviewModel>,
    /// The find bar, while it is open, with its matches.
    pub find: Option<FindModel>,
}

/// The find bar and what `font.features.find` found for it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FindModel {
    pub state: FindState,
    /// The matches in the shown entry's text as shown (the draft, else the stored code), as byte ranges.
    pub here: Vec<(usize, usize)>,
    /// Matches in the scope.
    pub total: usize,
    /// More matches exist than were listed.
    pub truncated: bool,
    /// In the all-entries scope, the entries with matches in sidebar order.
    pub entries: Vec<EntryKey>,
    /// Why the query cannot run (an invalid regular expression).
    pub problem: Option<String>,
}

/// What `font.features.previewName` gives for a name Alt-clicked in the code: a glyph, a class's members (with the
/// anchors of `markClass` statements), a lookup's first rules, or why there is nothing to show.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PreviewModel {
    /// The name as clicked, and its characters in the code as shown.
    pub name: String,
    pub start: usize,
    pub end: usize,
    /// `glyph`, `class` or `lookup`; empty when `problem` says why there is no preview.
    pub kind: String,
    /// A glyph's Unicode values, as hexadecimal.
    pub unicodes: Vec<String>,
    /// A glyph that does not export.
    pub hidden: bool,
    /// A class's member glyphs, nested classes and tokens resolved.
    pub glyphs: Vec<String>,
    /// Names in a class that are neither glyphs nor classes.
    pub unknown: Vec<String>,
    /// Each `markClass` statement of a class: its anchor and its glyphs, space-separated.
    pub anchors: Vec<(String, String)>,
    /// A lookup's first rules, and whether it has more.
    pub rules: Vec<String>,
    pub more: bool,
    pub problem: Option<String>,
}

/// The names that fit at a place in the code (`font.features.complete`), for the code as typed and the caret's byte
/// offset they answer.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompletionModel {
    pub code: String,
    pub position: usize,
    pub items: Vec<CompletionItem>,
}

/// One name that fits, and the bytes of the code it replaces.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub start: usize,
    pub end: usize,
}

/// One error or warning of the last compile in the shown entry.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DiagnosticModel {
    pub id: u64,
    pub error: bool,
    pub message: String,
    pub line: u64,
    /// The bytes of the entry's code the diagnostic is about, which the code editor underlines.
    pub range: Option<(usize, usize)>,
    /// Labels of the suggested fixes.
    pub fixes: Vec<String>,
}

/// The snippets menu: built-in titles, then one group of titles per user file.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SnippetsModel {
    pub built_in: Vec<String>,
    pub groups: Vec<(String, Vec<String>)>,
    pub folder: Option<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

pub fn project(cx: &mut ProjectCx<'_>) -> FeaturesModel {
    let errors = cx.session.query("font.features.listErrors", &json!({})).ok();
    let snippets = snippets(cx.session.query("font.features.listSnippets", &json!({})));
    let Ok(font) = cx.session.font() else { return FeaturesModel::default() };
    let f = &font.features;
    let compiled = errors.as_ref().and_then(|r| r.get("current")).and_then(Value::as_bool).unwrap_or(false);
    let diagnostics = diagnostics(errors.as_ref());
    let with_errors: HashSet<(EntryKind, usize)> =
        diagnostics.iter().filter(|(_, d)| d.error).filter_map(|(e, _)| e.as_ref().map(|e| (e.kind, e.index))).collect();
    let filter = cx.view.font_info.features.filter.trim().to_lowercase();
    let groups = EntryKind::ALL
        .into_iter()
        .map(|kind| {
            let group = kind.group(f);
            GroupModel {
                kind,
                entries: group
                    .iter()
                    .take(MAX_SHOWN_ENTRIES)
                    .enumerate()
                    .map(|(index, e)| EntryRow {
                        name: e.name.clone(),
                        automatic: e.automatic,
                        disabled: e.disabled,
                        error: with_errors.contains(&(kind, index)),
                        title: if kind == EntryKind::Feature {
                            registry::lookup(&e.name).map(|t| t.name.to_string()).unwrap_or_default()
                        } else {
                            String::new()
                        },
                        filtered: !passes(&filter, kind, e),
                    })
                    .collect(),
                hidden: group.len().saturating_sub(MAX_SHOWN_ENTRIES),
            }
        })
        .collect();
    let selected = selection(f, &cx.view.font_info.features.selected);
    let master = crate::shell::masters::current_master(font, &cx.view.layers).map(|m| m.id.clone());
    let entry = selected.last().and_then(|key| pane(font, key, &diagnostics));
    let spec_url = match &entry {
        Some(e) if e.key.kind == EntryKind::Feature => cx
            .session
            .query("font.features.openSpec", &json!({ "tag": e.key.name }))
            .ok()
            .and_then(|r| r.get("url").and_then(Value::as_str).map(str::to_string)),
        _ => None,
    };
    let completion = entry.as_ref().and_then(|e| completion(cx, e));
    let preview = entry.as_ref().and_then(|e| preview(cx, e, master.as_deref()));
    let find = entry.as_ref().and_then(|e| find(cx, e));
    let caret = entry.as_ref().and_then(|e| cx.view.font_info.features.caret.as_ref().filter(|(k, _)| *k == e.key).map(|(_, c)| *c));
    let jump =
        entry.as_ref().and_then(|e| cx.view.font_info.features.jump.as_ref().filter(|(k, _, _)| *k == e.key).map(|(_, start, end)| (*start, *end)));
    let view = &cx.view.font_info.features;
    let entry = entry.map(|e| {
        let draft = view.draft.as_ref().filter(|d| d.key == e.key).map(|d| d.code.clone());
        let check = view.checked.as_ref().filter(|c| c.draft.key == e.key).map(|c| (c.draft.code.clone(), c.diagnostics.clone()));
        EntryPane { spec_url, completion, caret, jump, compiled, draft, check, preview, find, ..e }
    });
    FeaturesModel { groups, selected, entry, filter: view.filter.clone(), notes_height: view.notes_height, compiling: view.compiling, snippets }
}

/// Whether the sidebar lists the entry `e` under the filter text `filter`, in lower case: its name as shown or its code
/// holds that text. The empty filter passes every entry.
fn passes(filter: &str, kind: EntryKind, e: &Snippet) -> bool {
    if filter.is_empty() {
        return true;
    }
    let name = if kind == EntryKind::Class { format!("@{}", e.name.trim_start_matches('@')) } else { e.name.clone() };
    name.to_lowercase().contains(filter) || e.code.to_lowercase().contains(filter)
}

/// The selection resolved against the font, without repeats; the first entry when nothing is selected.
fn selection(f: &Features, wanted: &[EntryKey]) -> Vec<EntryKey> {
    let mut seen = HashSet::new();
    let out: Vec<EntryKey> = wanted.iter().filter_map(|k| k.resolve(f)).filter(|k| seen.insert(k.clone())).collect();
    if !out.is_empty() {
        return out;
    }
    EntryKind::ALL.into_iter().find_map(|kind| kind.group(f).first().map(|e| EntryKey { kind, name: e.name.clone(), index: 0 })).into_iter().collect()
}

/// Whether `tag` is one of ss01 to ss20.
pub fn is_stylistic_set(tag: &str) -> bool {
    tag.strip_prefix("ss").filter(|n| n.len() == 2).and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=20).contains(&n))
}

fn pane(font: &Font, key: &EntryKey, diagnostics: &[(Option<EntryKey>, DiagnosticModel)]) -> Option<EntryPane> {
    let e = key.snippet(&font.features)?;
    let stylistic_set = key.kind == EntryKind::Feature && is_stylistic_set(&e.name);
    Some(EntryPane {
        key: key.clone(),
        code: e.code.clone(),
        notes: e.notes.clone(),
        automatic: e.automatic,
        stylistic_set,
        labels: if stylistic_set {
            e.labels.iter().take(MAX_SHOWN_LABELS).map(|l| (l.language.clone(), l.value.clone())).collect()
        } else {
            Vec::new()
        },
        spec_url: None,
        completion: None,
        caret: None,
        jump: None,
        preview: None,
        find: None,
        compiled: false,
        draft: None,
        check: None,
        diagnostics: diagnostics.iter().filter(|(k, _)| k.as_ref() == Some(key)).map(|(_, d)| d.clone()).take(MAX_SHOWN_ERRORS).collect(),
    })
}

/// The diagnostics of a `font.features.listErrors` result with the entry each is in; none once the font changed since
/// the compile.
fn diagnostics(result: Option<&Value>) -> Vec<(Option<EntryKey>, DiagnosticModel)> {
    let Some(r) = result else { return Vec::new() };
    if !r.get("current").and_then(Value::as_bool).unwrap_or(false) {
        return Vec::new();
    }
    diagnostic_list(r)
}

/// The diagnostics of a `font.features.check` result that are in the checked entry, `key`.
pub fn checked(result: &Value, key: &EntryKey) -> Vec<DiagnosticModel> {
    diagnostic_list(result).into_iter().filter(|(k, _)| k.as_ref() == Some(key)).map(|(_, d)| d).take(MAX_SHOWN_ERRORS).collect()
}

/// The `diagnostics` of a `listErrors` or `check` result with the entry each is in.
fn diagnostic_list(r: &Value) -> Vec<(Option<EntryKey>, DiagnosticModel)> {
    let list = r.get("diagnostics").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .take(MAX_DIAGNOSTICS)
        .filter_map(|d| {
            let model = DiagnosticModel {
                id: d.get("id")?.as_u64()?,
                error: d.get("severity").and_then(Value::as_str) == Some("error"),
                message: d.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
                line: d.get("line").and_then(Value::as_u64).unwrap_or(0),
                range: byte_range(d.get("range")),
                fixes: strings(d.get("fixes")).into_iter().take(MAX_FIXES).collect(),
            };
            Some((d.get("entry").and_then(EntryKey::from_json), model))
        })
        .collect()
}

/// The answer to the code editor's completion question, when it asks about the shown entry.
fn completion(cx: &mut ProjectCx<'_>, e: &EntryPane) -> Option<CompletionModel> {
    let ask = cx.view.font_info.features.completion.as_ref().filter(|a| a.key == e.key)?;
    let params = ask.key.with(&[("code", json!(ask.code)), ("position", json!(ask.position))]);
    let result = cx.session.query("font.features.complete", &params).ok()?;
    let list = result.get("completions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let items = list
        .iter()
        .take(craft_ext::ui::text::MAX_COMPLETIONS)
        .filter_map(|c| {
            let (start, end) = byte_range(c.get("replace"))?;
            Some(CompletionItem { label: c.get("label")?.as_str()?.to_string(), start, end })
        })
        .collect();
    Some(CompletionModel { code: ask.code.clone(), position: ask.position, items })
}

/// The find bar's matches in the shown entry `e`, and in the all-entries scope the entries with matches.
fn find(cx: &mut ProjectCx<'_>, e: &EntryPane) -> Option<FindModel> {
    let view = &cx.view.font_info.features;
    let state = view.find.clone()?;
    let shown = view.draft.as_ref().filter(|d| d.key == e.key).map_or_else(|| e.code.clone(), |d| d.code.clone());
    let mut model = FindModel { state: state.clone(), ..FindModel::default() };
    if state.query.is_empty() {
        return Some(model);
    }
    let mut here = state.params();
    here.extend(e.key.params());
    here.insert("code".into(), json!(shown));
    let r = match cx.session.query("font.features.find", &Value::Object(here)) {
        Ok(r) => r,
        Err(err) => return Some(FindModel { problem: Some(err.to_string()), ..model }),
    };
    let listed = r.get("matches").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    model.here = listed
        .iter()
        .filter_map(|m| Some((usize::try_from(m.get("start")?.as_u64()?).ok()?, usize::try_from(m.get("end")?.as_u64()?).ok()?)))
        .collect();
    model.total = model.here.len();
    model.truncated = r.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    if state.all_entries
        && let Ok(r) = cx.session.query("font.features.find", &Value::Object(state.params()))
    {
        let listed = r.get("matches").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for k in listed.iter().filter_map(EntryKey::from_json) {
            if model.entries.last() != Some(&k) {
                model.entries.push(k.clone());
            }
            if k != e.key {
                model.total += 1;
            }
        }
        model.truncated |= r.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    }
    Some(model)
}

/// Most entries of each list a preview shows.
const MAX_PREVIEW_ITEMS: usize = 500;

/// The preview of the name the code editor asked about, when it asks about the shown entry.
fn preview(cx: &mut ProjectCx<'_>, e: &EntryPane, master: Option<&str>) -> Option<PreviewModel> {
    let ask = cx.view.font_info.features.preview.as_ref().filter(|a| a.key == e.key)?;
    let mut params = serde_json::Map::new();
    params.insert("name".into(), json!(ask.name));
    if let Some(m) = master {
        params.insert("master".into(), json!(m));
    }
    let base = PreviewModel { name: ask.name.clone(), start: ask.start, end: ask.end, ..PreviewModel::default() };
    let r = match cx.session.query("font.features.previewName", &Value::Object(params)) {
        Ok(r) => r,
        Err(err) => return Some(PreviewModel { problem: Some(err.to_string()), ..base }),
    };
    let list = |key: &str| strings(r.get(key)).into_iter().take(MAX_PREVIEW_ITEMS).collect::<Vec<_>>();
    let anchors = r
        .get("anchors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(MAX_PREVIEW_ITEMS)
        .map(|a| {
            let anchor = match a.get("anchor") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            (anchor, strings(a.get("glyphs")).join(" "))
        })
        .collect();
    Some(PreviewModel {
        kind: r.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
        unicodes: list("unicodes"),
        hidden: r.get("export").and_then(Value::as_bool) == Some(false),
        glyphs: list("glyphs"),
        unknown: list("unknown"),
        anchors,
        rules: list("rules"),
        more: r.get("more").and_then(Value::as_bool).unwrap_or(false),
        ..base
    })
}

/// A diagnostic's `range`, `[start, end]` in bytes; `None` when it is missing or ends before it starts.
fn byte_range(v: Option<&Value>) -> Option<(usize, usize)> {
    let r = v?.as_array()?;
    let start = usize::try_from(r.first()?.as_u64()?).ok()?;
    let end = usize::try_from(r.get(1)?.as_u64()?).ok()?;
    (start <= end).then_some((start, end))
}

fn snippets(result: Result<Value, fontcraft_engine::EngineError>) -> SnippetsModel {
    let r = match result {
        Ok(r) => r,
        Err(e) => return SnippetsModel { error: Some(e.to_string()), ..SnippetsModel::default() },
    };
    let groups = r
        .get("groups")
        .and_then(Value::as_array)
        .map(|g| g.iter().take(MAX_MENU_SNIPPETS).filter_map(|g| Some((g.get("name")?.as_str()?.to_string(), titles(g.get("snippets"))))).collect())
        .unwrap_or_default();
    SnippetsModel {
        built_in: titles(r.get("builtIn")),
        groups,
        folder: r.get("folder").and_then(Value::as_str).map(str::to_string),
        warnings: strings(r.get("warnings")),
        error: None,
    }
}

fn titles(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_MENU_SNIPPETS).filter_map(|s| s.get("title")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().take(MAX_MENU_SNIPPETS).filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}
