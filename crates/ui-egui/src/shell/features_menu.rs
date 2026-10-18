//! The Features menu of Edit View, at the left of the bottom bar (rows `editView.featuresMenu`,
//! `features.menuNeedsCompile`, `features.buttonShowsActiveTags`), and its shell commands:
//! `editView.toggleFeature`, `editView.clearFeatures`, `editView.setLocalizedForms` and the query
//! `editView.getFeatures`.
//!
//! The menu lists the features of the document's last feature compile (its feature state), not
//! of the code as it is now: a feature written since appears after the next compile. Its entries,
//! top to bottom: the dash, which turns every feature off; one checkable entry per compiled
//! feature; Localized Forms; the script and language pairs the compiled `locl` feature is
//! registered for. Typing while the menu is open moves the highlight to the first entry whose
//! name starts with what was typed (the dash too); Return picks it.
//!
//! The active features are the tab's view state (`Tab::features`): no undo step, the document
//! stays unmodified. The button shows the active tags and takes the accent colour while any
//! feature or Localized Forms is on. Edit View shapes the tab's text with them
//! (`crate::canvas::view::text_layout`).

use std::sync::Arc;

use egui::{Popup, PopupCloseBehavior, Sense, StrokeKind};
use fontcraft_engine::Session;
use fontcraft_engine::fea::LayoutTable;
use fontcraft_engine::feature_state::FeatureState;
use fontcraft_textlayout::shaping::Tag;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::{Pending, widgets};

/// Most features a tab keeps active.
pub const MAX_ACTIVE: usize = 256;
/// How long, in seconds, typed characters build one type-ahead search.
const TYPE_AHEAD_SECONDS: f64 = 1.0;
/// The `locl` tag.
const LOCL: &str = "locl";

/// The features a tab previews.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct TabFeatures {
    /// The active feature tags, in the order they were turned on.
    pub active: Vec<String>,
    /// Localized Forms is on.
    pub localized: bool,
    /// The OpenType script and language tags Localized Forms evaluates `locl` for.
    pub script: Option<String>,
    pub language: Option<String>,
}

impl TabFeatures {
    /// Whether the tab's text is shaped at all.
    pub fn any(&self) -> bool {
        !self.active.is_empty() || self.localized
    }

    /// The active tags, as tags.
    pub fn active_tags(&self) -> Vec<Tag> {
        self.active.iter().filter_map(|t| tag(t)).collect()
    }

    /// Localized Forms' script and language tags, when it is on (a missing one is the default).
    pub fn localized_tags(&self) -> Option<(Tag, Tag)> {
        self.localized.then(|| {
            (self.script.as_deref().and_then(tag).unwrap_or(Tag::new(b"DFLT")), self.language.as_deref().and_then(tag).unwrap_or(Tag::new(b"dflt")))
        })
    }
}

/// A tag from text: one to four printable ASCII characters, padded with spaces.
pub fn tag(text: &str) -> Option<Tag> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 || !bytes.iter().all(|b| (0x20..=0x7e).contains(b)) || bytes.first() == Some(&b' ') {
        return None;
    }
    let mut out = [b' '; 4];
    for (slot, b) in out.iter_mut().zip(bytes) {
        *slot = *b;
    }
    Some(Tag::new(&out))
}

/// A tag as text, without its padding.
fn tag_text(tag: Tag) -> String {
    tag.to_string().trim_end().to_string()
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "editView.toggleFeature",
        "Toggle Feature",
        &[],
        "",
        "{tab?: id, tag: string, on?: bool} turn one OpenType feature on or off for the tab's text (omitted: the other state); several can be on together. A tag the last feature compile does not have is kept and applies once a compile has it",
    ),
    ("editView.clearFeatures", "Deactivate All Features", &[], "", "{tab?: id} turn off every preview feature of the tab, and Localized Forms"),
    (
        "editView.setLocalizedForms",
        "Localized Forms",
        &[],
        "",
        "{tab?: id, on?: bool, script?: tag, language?: tag} turn the tab's language-specific forms on or off (omitted: on when a script or language is given, else the other state) and pick the OpenType script and language tags `locl` is evaluated for; turned on without them, the first pair the compiled locl feature has",
    ),
    (
        "editView.getFeatures",
        "Get Preview Features",
        &[],
        "",
        "{tab?: id} the Features menu of the tab → {tab, compiled: bool (the document has a feature compile), current: bool (the font has not changed since), features: [{tag, table: \"GSUB\"|\"GPOS\", active: bool}], active: [tag], localizedForms: bool, script, language, languageSystems: [[script, language]] (the pairs Localized Forms offers), shapingError?: string}",
    ),
];

/// The document's last feature compile.
fn state(session: &Session) -> Option<Arc<FeatureState>> {
    session.feature_state()
}

/// The script and language pairs Localized Forms offers: the compiled `locl` feature's, without
/// the default language, each once.
fn localized_pairs(state: Option<&FeatureState>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(state) = state else { return out };
    for f in state.features.iter().filter(|f| tag_text(f.tag) == LOCL) {
        for (script, language) in &f.language_systems {
            let pair = (tag_text(*script), tag_text(*language));
            if pair.1 != "dflt" && !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

/// The compiled features, each tag once, in feature-list order.
fn compiled_features(state: Option<&FeatureState>) -> Vec<(String, LayoutTable)> {
    let mut out: Vec<(String, LayoutTable)> = Vec::new();
    for f in state.map(|s| s.features.as_slice()).unwrap_or_default() {
        let t = tag_text(f.tag);
        if !out.iter().any(|(o, _)| *o == t) {
            out.push((t, f.table));
        }
    }
    out
}

fn table_name(table: LayoutTable) -> &'static str {
    match table {
        LayoutTable::Gsub => "GSUB",
        LayoutTable::Gpos => "GPOS",
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let index = super::view::target_tab(app, args)?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    // Every change is worked out on a copy, so a command that fails changes nothing.
    let mut next = tab.features.clone();
    match id {
        "editView.getFeatures" => return Ok(reply(app, index)),
        "editView.toggleFeature" => {
            let text = args.string("tag")?.ok_or_else(|| args.err("`tag` (a feature tag such as \"smcp\") is required"))?;
            let t = tag(text).ok_or_else(|| args.err(format!("`tag` must be one to four printable ASCII characters, got \"{text}\"")))?;
            let text = tag_text(t);
            let on = match args.flag("on")? {
                Some(on) => on,
                None => !next.active.contains(&text),
            };
            next.active.retain(|a| *a != text);
            if on {
                if next.active.len() >= MAX_ACTIVE {
                    return Err(args.err(format!("a tab keeps at most {MAX_ACTIVE} active features")));
                }
                next.active.push(text);
            }
        }
        "editView.clearFeatures" => {
            next.active.clear();
            next.localized = false;
        }
        "editView.setLocalizedForms" => {
            let parse = |key: &str| -> Result<Option<String>, String> {
                match args.string(key)? {
                    None => Ok(None),
                    Some(text) => tag(text)
                        .map(|t| Some(tag_text(t)))
                        .ok_or_else(|| args.err(format!("`{key}` must be one to four printable ASCII characters, got \"{text}\""))),
                }
            };
            let (script, language) = (parse("script")?, parse("language")?);
            let on = match args.flag("on")? {
                Some(on) => on,
                None => script.is_some() || language.is_some() || !next.localized,
            };
            next.localized = on;
            if script.is_some() || language.is_some() {
                next.script = script.or(next.script);
                next.language = language.or(next.language);
            } else if on && next.script.is_none() && next.language.is_none() {
                let state = state(&app.session);
                if let Some((s, l)) = localized_pairs(state.as_deref()).into_iter().next() {
                    next.script = Some(s);
                    next.language = Some(l);
                }
            }
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    if let Some(tab) = app.state.tabs.get_mut(index) {
        tab.features = next;
    }
    Ok(reply(app, index))
}

/// `editView.getFeatures`' reply for the tab at `index`.
pub(crate) fn reply(app: &FontcraftApp, index: usize) -> Value {
    let Some(tab) = app.state.tabs.get(index) else { return Value::Null };
    let state = state(&app.session);
    let current = match (state.as_deref(), app.session.doc()) {
        (Some(s), Ok(doc)) => s.is_current(doc),
        _ => false,
    };
    let features: Vec<Value> = compiled_features(state.as_deref())
        .into_iter()
        .map(|(t, table)| json!({"tag": t, "table": table_name(table), "active": tab.features.active.contains(&t)}))
        .collect();
    let pairs: Vec<Value> = localized_pairs(state.as_deref()).into_iter().map(|(s, l)| json!([s, l])).collect();
    let mut out = json!({
        "tab": tab.id,
        "compiled": state.is_some(),
        "current": current,
        "features": features,
        "active": tab.features.active,
        "localizedForms": tab.features.localized,
        "script": tab.features.script,
        "language": tab.features.language,
        "languageSystems": pairs,
    });
    let error = crate::canvas::view::text_layout(&app.session, &app.layouts, tab).and_then(|l| l.shaping_error.clone());
    if let (Some(error), Some(obj)) = (error, out.as_object_mut()) {
        obj.insert("shapingError".into(), Value::String(error));
    }
    out
}

// ── The menu ────────────────────────────────────────────────────────────────────────────────────

/// One entry of the menu.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Clear,
    Feature(String),
    Localized,
    Pair(String, String),
}

impl Entry {
    /// The name type-ahead matches.
    fn name(&self) -> String {
        match self {
            Entry::Clear => "-".into(),
            Entry::Feature(t) => t.clone(),
            Entry::Localized => "Localized Forms".into(),
            Entry::Pair(s, l) => format!("{s} {l}"),
        }
    }

    /// The command the entry runs on `tab`.
    fn command(&self, tab: u64) -> (&'static str, Value) {
        match self {
            Entry::Clear => ("editView.clearFeatures", json!({ "tab": tab })),
            Entry::Feature(t) => ("editView.toggleFeature", json!({"tab": tab, "tag": t})),
            Entry::Localized => ("editView.setLocalizedForms", json!({ "tab": tab })),
            Entry::Pair(s, l) => ("editView.setLocalizedForms", json!({"tab": tab, "on": true, "script": s, "language": l})),
        }
    }
}

/// The menu's entries, top to bottom.
fn entries(state: Option<&FeatureState>) -> Vec<Entry> {
    let mut out = vec![Entry::Clear];
    out.extend(compiled_features(state).into_iter().map(|(t, _)| Entry::Feature(t)));
    out.push(Entry::Localized);
    out.extend(localized_pairs(state).into_iter().map(|(s, l)| Entry::Pair(s, l)));
    out
}

/// The entry type-ahead picks for `typed`: the first whose name starts with it, ignoring case.
fn type_ahead(entries: &[Entry], typed: &str) -> Option<usize> {
    let typed = typed.to_lowercase();
    (!typed.is_empty()).then(|| entries.iter().position(|e| e.name().to_lowercase().starts_with(&typed))).flatten()
}

/// The type-ahead state of the open menu: what was typed and when the last character came.
#[derive(Clone, Debug, Default)]
struct Typed {
    text: String,
    at: f64,
}

/// The Features button and its menu, at the left of the bottom bar.
pub fn bottom_bar_button(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    let (tab_id, features) = (tab.id, &tab.features);
    let state = state(&app.session);
    let label = if features.active.is_empty() { "Features".to_string() } else { features.active.join(" ") };
    let on = features.any();
    let id = widgets::id("bottomBar.features");

    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_color = if on { app.tokens.selected_text } else { ui.visuals().text_color() };
    let galley = ui.painter().layout_no_wrap(label, font, text_color);
    let side = ui.spacing().interact_size.y;
    let size = (galley.size() + 2.0 * ui.spacing().button_padding).max(egui::vec2(side, side));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let fill = if on { app.tokens.accent } else { visuals.weak_bg_fill };
        ui.painter().rect(rect, visuals.corner_radius, fill, visuals.bg_stroke, StrokeKind::Inside);
        ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, text_color);
    }
    let response = response.on_hover_text("Features to preview in this tab");

    let list = entries(state.as_deref());
    let memory = id.with("typed");
    let mut picked: Option<Entry> = None;
    let open = Popup::menu(&response).id(id.with("menu")).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let now = ui.input(|i| i.time);
        let mut typed: Typed = ui.ctx().data(|d| d.get_temp(memory)).unwrap_or_default();
        let (text, enter) = ui.input(|i| {
            let text: String = i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None }).collect();
            (text, i.key_pressed(egui::Key::Enter))
        });
        if !text.is_empty() {
            if now - typed.at > TYPE_AHEAD_SECONDS {
                typed.text.clear();
            }
            typed.text.extend(text.chars().take(64));
            typed.at = now;
        }
        let highlighted = type_ahead(&list, &typed.text);
        ui.ctx().data_mut(|d| d.insert_temp(memory, typed));
        if enter && let Some(entry) = highlighted.and_then(|i| list.get(i)) {
            picked = Some(entry.clone());
        }
        if state.is_none() {
            ui.weak("Compile the features in Font Info to list them here.");
        }
        for (i, entry) in list.iter().enumerate() {
            if matches!(entry, Entry::Localized) {
                ui.separator();
            }
            let checked = match entry {
                Entry::Clear => false,
                Entry::Feature(t) => features.active.contains(t),
                Entry::Localized => features.localized,
                Entry::Pair(s, l) => features.script.as_deref() == Some(s.as_str()) && features.language.as_deref() == Some(l.as_str()),
            };
            let label = match entry {
                Entry::Clear => "-".to_string(),
                Entry::Pair(s, l) => format!("    {s} / {l}"),
                other => other.name(),
            };
            let enabled = !matches!(entry, Entry::Pair(..)) || features.localized;
            let row = ui.add_enabled(enabled, egui::Button::selectable(checked || highlighted == Some(i), label));
            if highlighted == Some(i) {
                row.scroll_to_me(None);
            }
            if row.clicked() {
                picked = Some(entry.clone());
            }
        }
    });
    if open.is_none() {
        ui.ctx().data_mut(|d| d.remove::<Typed>(memory));
    }
    if let Some(entry) = picked {
        pending.push(entry.command(tab_id));
        ui.ctx().data_mut(|d| d.remove::<Typed>(memory));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_ahead_finds_the_first_entry_by_name_and_the_dash() {
        let list = vec![Entry::Clear, Entry::Feature("liga".into()), Entry::Feature("smcp".into()), Entry::Localized];
        assert_eq!(type_ahead(&list, "sm"), Some(2));
        assert_eq!(type_ahead(&list, "-"), Some(0));
        assert_eq!(type_ahead(&list, "LOC"), Some(3));
        assert_eq!(type_ahead(&list, "x"), None);
        assert_eq!(type_ahead(&list, ""), None);
    }

    #[test]
    fn tags_are_one_to_four_printable_characters() {
        assert_eq!(tag("ss1").map(tag_text), Some("ss1".to_string()));
        assert!(tag("").is_none() && tag("toolong").is_none() && tag(" ab").is_none() && tag("é").is_none());
    }
}
