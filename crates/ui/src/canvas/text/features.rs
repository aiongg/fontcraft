//! The edit view's preview features (rows `editView.featuresMenu`, `editView.toggleFeature`, `editView.clearFeatures`,
//! `editView.setLocalizedForms`, `text.shaping`): the OpenType features the text is shaped with, through the preview
//! font of the document's last feature compile.
//!
//! The active features are view state ([`Features`] in `EditText::features`): no undo step, the document stays
//! unmodified. The Features menu (`panels::features_menu`) lists the features of the last compile, not of the code as
//! it is now: a feature written since appears after the next compile. `ui.inspect`'s `editText.features` reports the
//! menu as agents read it.

use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::fea::LayoutTable;
use fontcraft_engine::feature_state::FeatureState;
use fontcraft_textlayout::shaping::Tag;
use serde_json::{Map, Value, json};

use crate::app::FontCraftApp;
use crate::app::models::text::FeaturesModel;
use crate::shell::commands::{Module, UiCommand};

/// Most features the text keeps active.
pub const MAX_ACTIVE: usize = 256;
/// Most entries the menu lists for the compiled features and for the language systems, each.
pub const MAX_LISTED: usize = 512;
/// The `locl` tag.
const LOCL: &str = "locl";

/// The features the text is shaped with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// The active feature tags, in the order they were turned on.
    pub active: Vec<String>,
    /// Localized Forms is on.
    pub localized: bool,
    /// The OpenType script and language tags Localized Forms evaluates `locl` for.
    pub script: Option<String>,
    pub language: Option<String>,
}

impl Features {
    /// Whether the text is shaped at all.
    pub fn any(&self) -> bool {
        !self.active.is_empty() || self.localized
    }

    pub fn active_tags(&self) -> Vec<Tag> {
        self.active.iter().filter_map(|t| tag(t)).collect()
    }

    /// Localized Forms' script and language tags while it is on (a missing one is the default).
    pub fn localized_tags(&self) -> Option<(Tag, Tag)> {
        self.localized.then(|| {
            let script = self.script.as_deref().and_then(tag).unwrap_or(Tag::new(b"DFLT"));
            let language = self.language.as_deref().and_then(tag).unwrap_or(Tag::new(b"dflt"));
            (script, language)
        })
    }
}

/// A tag from text: one to four printable ASCII characters, not starting with a space, padded with spaces.
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
pub fn tag_text(tag: Tag) -> String {
    tag.to_string().trim_end().to_string()
}

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "editView.toggleFeature",
        label: "Toggle Feature",
        menu: &[],
        shortcut: None,
        params: "{tag: string, on?: bool} turn one OpenType feature on or off for the edit view's text (omitted: the other state); several can be on together. A tag the last feature compile does not have is kept and applies once a compile has it",
    },
    UiCommand {
        id: "editView.clearFeatures",
        label: "Deactivate All Features",
        menu: &[],
        shortcut: None,
        params: "{} turn off every preview feature of the edit view's text, and Localized Forms",
    },
    UiCommand {
        id: "editView.setLocalizedForms",
        label: "Localized Forms",
        menu: &[],
        shortcut: None,
        params: "{on?: bool, script?: tag, language?: tag} turn the language-specific forms of the edit view's text on or off (omitted: on when a script or language is given, else the other state) and pick the OpenType script and language tags `locl` is evaluated for; turned on without them, the first pair the compiled locl feature has",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let empty = Map::new();
    let p = command.params.as_object().unwrap_or(&empty);
    // Every change is worked out on a copy, so a command that fails changes nothing.
    let mut next = app.view.text.features.clone();
    match command.id.as_str() {
        "editView.toggleFeature" => {
            let text = string(p, "tag")?.ok_or("`tag` (a feature tag such as \"smcp\") is required")?;
            let t = tag(text).ok_or_else(|| format!("`tag` must be one to four printable ASCII characters, got \"{}\"", quoted(text)))?;
            let text = tag_text(t);
            let on = flag(p, "on")?.unwrap_or(!next.active.contains(&text));
            next.active.retain(|a| *a != text);
            if on {
                if next.active.len() >= MAX_ACTIVE {
                    return Err(format!("the edit view keeps at most {MAX_ACTIVE} active features"));
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
                match string(p, key)? {
                    None => Ok(None),
                    Some(text) => tag(text)
                        .map(|t| Some(tag_text(t)))
                        .ok_or_else(|| format!("`{key}` must be one to four printable ASCII characters, got \"{}\"", quoted(text))),
                }
            };
            let (script, language) = (parse("script")?, parse("language")?);
            let given = script.is_some() || language.is_some();
            let on = flag(p, "on")?.unwrap_or(given || !next.localized);
            next.localized = on;
            if given {
                next.script = script.or(next.script);
                next.language = language.or(next.language);
            } else if on && next.script.is_none() && next.language.is_none() {
                let state = app.session.feature_state();
                if let Some((s, l)) = localized_pairs(state.as_deref()).into_iter().next() {
                    next.script = Some(s);
                    next.language = Some(l);
                }
            }
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    let text = &mut app.view.text;
    text.features = next;
    text.touch();
    app.view_changed();
    Ok(())
}

/// The script and language pairs Localized Forms offers: the compiled `locl` feature's, without the default
/// language, each once.
pub fn localized_pairs(state: Option<&FeatureState>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let Some(state) = state else { return out };
    for f in state.features.iter().filter(|f| tag_text(f.tag) == LOCL) {
        for (script, language) in &f.language_systems {
            let pair = (tag_text(*script), tag_text(*language));
            if out.len() < MAX_LISTED && pair.1 != "dflt" && !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out
}

/// The compiled features, each tag once, in feature-list order, with the table each is in.
pub fn compiled_features(state: Option<&FeatureState>) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for f in state.map(|s| s.features.as_slice()).unwrap_or_default() {
        let t = tag_text(f.tag);
        if out.len() < MAX_LISTED && !out.iter().any(|(o, _)| *o == t) {
            let table = match f.table {
                LayoutTable::Gsub => "GSUB",
                LayoutTable::Gpos => "GPOS",
            };
            out.push((t, table));
        }
    }
    out
}

/// What the Features menu shows: the compiled features and language systems with the text's active ones.
pub fn menu(session: &Session, features: &Features, shaping_error: Option<String>) -> FeaturesModel {
    let state = session.feature_state();
    let current = match (state.as_deref(), session.doc()) {
        (Some(s), Ok(doc)) => s.is_current(doc),
        _ => false,
    };
    FeaturesModel {
        compiled: state.is_some(),
        current,
        features: compiled_features(state.as_deref()).into_iter().map(|(t, table)| (t, table.to_string())).collect(),
        language_systems: localized_pairs(state.as_deref()),
        active: features.active.clone(),
        localized: features.localized,
        script: features.script.clone(),
        language: features.language.clone(),
        shaping_error,
    }
}

/// `ui.inspect`'s `editText.features`: `{compiled, current, features: [{tag, table, active}], active, localizedForms,
/// script, language, languageSystems, shapingError?}`.
pub fn reply(model: &FeaturesModel) -> Value {
    let features: Vec<Value> =
        model.features.iter().map(|(t, table)| json!({"tag": t, "table": table, "active": model.active.contains(t)})).collect();
    let pairs: Vec<Value> = model.language_systems.iter().map(|(s, l)| json!([s, l])).collect();
    let mut out = json!({
        "compiled": model.compiled,
        "current": model.current,
        "features": features,
        "active": model.active,
        "localizedForms": model.localized,
        "script": model.script,
        "language": model.language,
        "languageSystems": pairs,
    });
    if let (Some(error), Some(obj)) = (&model.shaping_error, out.as_object_mut()) {
        obj.insert("shapingError".into(), json!(error));
    }
    out
}

/// The identity of the document's last feature compile: a new compile gives a new one.
pub fn compile_stamp(session: &Session) -> Option<(u64, u64, bool)> {
    session.feature_state().map(|s| (s.revision, s.source_hash, s.preview.is_some()))
}

/// The preview font a layout of `features` shapes with, when any feature is on and a compile made one.
pub fn preview(session: &Session, features: &Features) -> Option<Arc<fontcraft_engine::compile::preview::PreviewFont>> {
    if !features.any() {
        return None;
    }
    session.feature_state().and_then(|s| s.preview.clone())
}

fn quoted(s: &str) -> String {
    s.chars().take(16).collect()
}

fn string<'a>(p: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}
