//! The Features tab's view model: the sidebar groups, the selection, the shown entry with its stylistic-set names and
//! the last compile's diagnostics, and the snippets menu.
//!
//! It is projected only while the Features tab shows: `font.features.listErrors`, `font.features.openSpec` and
//! `font.features.listSnippets` (which reads the snippet folder) run then and not otherwise.

use std::collections::HashSet;
use std::sync::Arc;

use craft::ui::contract::PlainData;
use fontcraft_engine::features::registry;
use fontcraft_engine::model::{Features, Font, Snippet};
use serde_json::{Value, json};

use super::ProjectCx;
use super::font_view::{CellArt, flatten};
use crate::shell::feature_work::expand_target;
use crate::shell::font_info_tabs::{EntryKey, EntryKind, ExpandTarget, ExpandedView, FindState, PreviewAsk};

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
    /// The expanded-code view, while it shows in place of the entry.
    pub expanded: Option<ExpandedModel>,
    /// The text of the sidebar's filter field.
    pub filter: String,
    /// The height of the notes field in points, once its divider was dragged.
    pub notes_height: Option<f32>,
    /// The compile the Compile button asked for is under way.
    pub compiling: bool,
    /// The code changed since the last compile, or was never compiled while there is some.
    pub stale: bool,
    /// What the last compile found outside the entries: in generated features, the glyph order or the preview font, or
    /// in code an instance's parameter writes.
    pub unplaced: Vec<DiagnosticModel>,
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

/// The expanded code of one build (`font.features.expandCode`).
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct ExpandedModel {
    pub state: ExpandedView,
    /// The build shown, and every build with its name.
    pub target: ExpandTarget,
    pub targets: Vec<(ExpandTarget, String)>,
    /// The whole code, and the part of it the field shows.
    pub code: Arc<String>,
    pub shown: Arc<String>,
    /// The code of the build and the font as they are is still to come.
    pub busy: bool,
    pub refused: Vec<String>,
    pub problem: Option<String>,
    /// The matches of the find field's text in the part shown, as byte ranges; at most [`MAX_FOUND`].
    pub found: Vec<(usize, usize)>,
}

/// What the preview of a name or token Alt-clicked in the code shows: a token's value for the current master and in every
/// build or the glyphs it matches (`font.features.evaluateToken`); a glyph, a class's members (with the anchors of
/// `markClass` statements), a lookup's first rules (`font.features.previewName`); or why there is nothing to show.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PreviewModel {
    /// The name as clicked, and its characters in the code as shown.
    pub name: String,
    pub start: usize,
    pub end: usize,
    /// It is a token.
    pub token: bool,
    /// What the preview was made from: the document's id and revision, and the master.
    pub made_from: (u64, u64, String),
    /// `number` or `glyphs` for a token, else `glyph`, `class` or `lookup`; empty when `problem` says why there is no preview.
    pub kind: String,
    /// A glyph's Unicode values, as hexadecimal.
    pub unicodes: Vec<String>,
    /// A glyph that does not export.
    pub hidden: bool,
    /// A number token's value for the current master.
    pub value: i64,
    /// Its value in every master and exported instance, as (name, value, why it has none).
    pub values: Vec<(String, Option<i64>, String)>,
    /// The glyph, or the first glyphs of a class (nested classes and tokens resolved) or of a predicate token, and how
    /// many there are.
    pub glyphs: Vec<String>,
    pub total: usize,
    /// The drawings of `glyphs` in the current master.
    pub art: Vec<Arc<CellArt>>,
    pub units_per_em: f32,
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
    /// Its id in `font.features.listErrors`; meaningless without fixes.
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

/// `before` is the preview the model showed, kept while what it was made from stays the same.
pub fn project(cx: &mut ProjectCx<'_>, before: Option<&PreviewModel>) -> FeaturesModel {
    let errors = cx.session.query("font.features.listErrors", &json!({})).ok();
    let snippets = snippets(cx.session.query("font.features.listSnippets", &json!({})));
    let Ok(font) = cx.session.font() else { return FeaturesModel::default() };
    let f = &font.features;
    let compiled = errors.as_ref().and_then(|r| r.get("current")).and_then(Value::as_bool).unwrap_or(false);
    let diagnostics = diagnostics(errors.as_ref());
    let unplaced = diagnostics.iter().filter(|(entry, _)| entry.is_none()).map(|(_, d)| d.clone()).take(MAX_SHOWN_ERRORS).collect();
    let stale = !compiled && !f.is_empty();
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
    let preview = entry.as_ref().and_then(|e| preview(cx, e, master.as_deref(), before));
    let expanded = expanded(cx);
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
    FeaturesModel {
        groups,
        selected,
        entry,
        expanded,
        filter: view.filter.clone(),
        notes_height: view.notes_height,
        compiling: view.compiling,
        stale,
        unplaced,
        snippets,
    }
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

/// The diagnostics of a `font.features.listErrors` result with the entry each is in, those of the other targets after
/// the compiled master's; none once the font changed since the compile.
fn diagnostics(result: Option<&Value>) -> Vec<(Option<EntryKey>, DiagnosticModel)> {
    let Some(r) = result else { return Vec::new() };
    if !r.get("current").and_then(Value::as_bool).unwrap_or(false) {
        return Vec::new();
    }
    let mut out = diagnostic_list(r, "diagnostics");
    out.extend(diagnostic_list(r, "targetDiagnostics"));
    if let Some(message) = r.get("previewMessage").and_then(Value::as_str) {
        out.push((None, DiagnosticModel { id: u64::MAX, error: true, message: message.to_string(), ..DiagnosticModel::default() }));
    }
    out
}

/// Most target names a message lists.
const MAX_NAMED_TARGETS: usize = 5;

/// A diagnostic's message, with the names of the targets it is limited to.
fn message(d: &Value) -> String {
    let text = d.get("message").and_then(Value::as_str).unwrap_or_default();
    let names = strings(d.get("targets"));
    if names.is_empty() {
        return text.to_string();
    }
    let mut shown = names.iter().take(MAX_NAMED_TARGETS).map(String::as_str).collect::<Vec<_>>().join(", ");
    if names.len() > MAX_NAMED_TARGETS {
        shown.push_str(", …");
    }
    format!("{text} ({shown})")
}

/// The diagnostics of a `font.features.check` result that are in the checked entry, `key`.
pub fn checked(result: &Value, key: &EntryKey) -> Vec<DiagnosticModel> {
    diagnostic_list(result, "diagnostics").into_iter().filter(|(k, _)| k.as_ref() == Some(key)).map(|(_, d)| d).take(MAX_SHOWN_ERRORS).collect()
}

/// The diagnostics under `key` of a `listErrors` or `check` result with the entry each is in.
fn diagnostic_list(r: &Value, key: &str) -> Vec<(Option<EntryKey>, DiagnosticModel)> {
    let list = r.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .take(MAX_DIAGNOSTICS)
        .map(|d| {
            let model = DiagnosticModel {
                id: d.get("id").and_then(Value::as_u64).unwrap_or(u64::MAX),
                error: d.get("severity").and_then(Value::as_str) == Some("error"),
                message: message(d),
                line: d.get("line").and_then(Value::as_u64).unwrap_or(0),
                range: byte_range(d.get("range")),
                fixes: strings(d.get("fixes")).into_iter().take(MAX_FIXES).collect(),
            };
            (d.get("entry").and_then(EntryKey::from_json), model)
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

/// Most entries of each list a preview shows, and most glyphs it draws.
pub const MAX_PREVIEW_ITEMS: usize = 500;
/// Most builds a token's preview lists.
const MAX_PREVIEW_TARGETS: usize = 256;

/// The preview of the name or token the code editor asked about, when it asks about the shown entry.
fn preview(cx: &mut ProjectCx<'_>, e: &EntryPane, master: Option<&str>, before: Option<&PreviewModel>) -> Option<PreviewModel> {
    let ask = cx.view.font_info.features.preview.clone().filter(|a| a.key == e.key)?;
    let doc = cx.session.active()?;
    let made_from = (doc.id, doc.revision, master.unwrap_or_default().to_string());
    if let Some(kept) = before
        .filter(|p| p.made_from == made_from && (p.name.as_str(), p.start, p.end, p.token) == (ask.name.as_str(), ask.start, ask.end, ask.token))
    {
        return Some(kept.clone());
    }
    let mut model = match if ask.token { token_preview(cx, &ask, master) } else { name_preview(cx, &ask, master) } {
        Ok(model) => model,
        Err(problem) => PreviewModel { problem: Some(problem), ..PreviewModel::default() },
    };
    if let (Ok(font), Some(master)) = (cx.session.font(), master) {
        model.units_per_em = f32::from(font.units_per_em.max(1));
        model.art = model.glyphs.iter().map(|g| Arc::new(art_of(font, g, master).unwrap_or_default())).collect();
    }
    Some(PreviewModel { name: ask.name, start: ask.start, end: ask.end, token: ask.token, made_from, ..model })
}

/// The outline of `glyph` in `master`, flattened.
fn art_of(font: &Font, glyph: &str, master: &str) -> Option<CellArt> {
    let g = font.glyph(glyph)?;
    let width = g.layer(master).map_or(0.0, |l| l.width) as f32;
    let contours = font.outline_of(g, master).map(|p| flatten(&p)).unwrap_or_default();
    Some(CellArt { width: if width.is_finite() { width } else { 0.0 }, contours, color: None })
}

/// What `font.features.evaluateToken` gives for the token asked about.
fn token_preview(cx: &mut ProjectCx<'_>, ask: &PreviewAsk, master: Option<&str>) -> Result<PreviewModel, String> {
    let mut params = serde_json::Map::new();
    params.insert("token".into(), json!(ask.name));
    params.insert("targets".into(), json!("all"));
    if let Some(m) = master {
        params.insert("master".into(), json!(m));
    }
    let r = cx.session.query("font.features.evaluateToken", &Value::Object(params)).map_err(|e| e.to_string())?;
    let rows = r.get("targets").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let values = rows.iter().take(MAX_PREVIEW_TARGETS).map(|row| {
        let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        (text("name"), row.get("value").and_then(Value::as_i64), text("problem"))
    });
    let mut glyphs = strings(r.get("glyphs"));
    glyphs.truncate(MAX_PREVIEW_ITEMS);
    Ok(PreviewModel {
        kind: r.get("kind").and_then(Value::as_str).unwrap_or_default().to_string(),
        value: r.get("value").and_then(Value::as_i64).unwrap_or_default(),
        values: values.collect(),
        total: r.get("glyphs").and_then(Value::as_array).map_or(0, Vec::len),
        glyphs,
        ..PreviewModel::default()
    })
}

/// What `font.features.previewName` gives for the name asked about.
fn name_preview(cx: &mut ProjectCx<'_>, ask: &PreviewAsk, master: Option<&str>) -> Result<PreviewModel, String> {
    let mut params = serde_json::Map::new();
    params.insert("name".into(), json!(ask.name));
    if let Some(m) = master {
        params.insert("master".into(), json!(m));
    }
    let r = cx.session.query("font.features.previewName", &Value::Object(params)).map_err(|e| e.to_string())?;
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
    let kind = r.get("kind").and_then(Value::as_str).unwrap_or_default().to_string();
    let glyphs = if kind == "glyph" { r.get("name").and_then(Value::as_str).map(str::to_string).into_iter().collect() } else { list("glyphs") };
    Ok(PreviewModel {
        unicodes: list("unicodes"),
        hidden: r.get("export").and_then(Value::as_bool) == Some(false),
        total: r.get("glyphs").and_then(Value::as_array).map_or(glyphs.len(), Vec::len),
        glyphs,
        kind,
        unknown: list("unknown"),
        anchors,
        rules: list("rules"),
        more: r.get("more").and_then(Value::as_bool).unwrap_or(false),
        ..PreviewModel::default()
    })
}

/// Most bytes of expanded code the field shows.
pub const MAX_SHOWN_CODE: usize = 1 << 20;
/// Most matches found in it.
pub const MAX_FOUND: usize = 10_000;
/// Most instances listed as builds.
const MAX_LISTED_INSTANCES: usize = 1_000;

/// The expanded-code view, while it is open.
fn expanded(cx: &mut ProjectCx<'_>) -> Option<ExpandedModel> {
    let view = &cx.view.font_info.features;
    let state = view.expanded.clone()?;
    let doc = cx.session.active()?;
    let font = &doc.font;
    let target = expand_target(font, &cx.view.layers, state.target.as_ref())?;
    let masters = font.masters.iter().map(|m| {
        let name = if m.name.is_empty() { &m.id } else { &m.name };
        (ExpandTarget::Master(m.id.clone()), name.clone())
    });
    let instances = font.instances.iter().enumerate().filter(|(_, i)| i.kind.is_none()).take(MAX_LISTED_INSTANCES).map(|(at, i)| {
        let name = if i.name.is_empty() { format!("#{at}") } else { i.name.clone() };
        (ExpandTarget::Instance(at), name)
    });
    let mut targets: Vec<(ExpandTarget, String)> = masters.chain(instances).collect();
    if !font.axes.is_empty() && font.masters.len() > 1 {
        targets.push((ExpandTarget::Variable, String::new()));
    }
    let same = |e: &&crate::shell::font_info_tabs::Expansion| (e.key.0, &e.key.2, e.key.3) == (doc.id, &target, state.generated);
    let expansion = view.expansion.as_ref().filter(same);
    let code = expansion.map(|e| Arc::clone(&e.code)).unwrap_or_default();
    let shown = shown_code(&code);
    let found = find_in(&shown, &state.query);
    Some(ExpandedModel {
        busy: expansion.is_none_or(|e| e.key.1 != doc.revision),
        refused: expansion.map(|e| e.refused.iter().take(MAX_SHOWN_ERRORS).cloned().collect()).unwrap_or_default(),
        problem: expansion.and_then(|e| e.problem.clone()),
        state,
        target,
        targets,
        code,
        shown,
        found,
    })
}

/// The first [`MAX_SHOWN_CODE`] bytes of `code`, whole lines.
fn shown_code(code: &Arc<String>) -> Arc<String> {
    if code.len() <= MAX_SHOWN_CODE {
        return Arc::clone(code);
    }
    let end = (0..=MAX_SHOWN_CODE).rev().find(|at| code.as_bytes().get(*at) == Some(&b'\n')).unwrap_or(0);
    Arc::new(code.get(..end).unwrap_or_default().to_string())
}

/// The byte ranges of `query` in `text`, ignoring the case of ASCII letters, at most [`MAX_FOUND`].
pub fn find_in(text: &str, query: &str) -> Vec<(usize, usize)> {
    let (text, query) = (text.as_bytes(), query.as_bytes());
    let Some(first) = query.first() else { return Vec::new() };
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(b) = text.get(at) {
        let end = at.saturating_add(query.len());
        if b.eq_ignore_ascii_case(first) && text.get(at..end).is_some_and(|t| t.eq_ignore_ascii_case(query)) {
            out.push((at, end));
            if out.len() >= MAX_FOUND {
                break;
            }
            at = end;
        } else {
            at += 1;
        }
    }
    out
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

pub fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().take(MAX_MENU_SNIPPETS).filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}
