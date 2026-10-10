//! Languages List (`dialog.languagesList`): one checkbox per script, ticked for the scripts the
//! sidebar's Languages section lists, over `fontView.setVisibleScripts`. It opens from the plus
//! button beside the Languages heading. Nothing runs until OK, which shows the ticked scripts and
//! hides the others; the list lives in the user settings, so Cancel leaves it as it was.
//!
//! The scripts offered are those the language data lists, with the default ones. A script the
//! font has glyphs for is listed in the sidebar whether it is ticked or not.

use serde_json::{Map, Value, json};

use super::form::{Field, Kind};
use super::{Dialog, Fields, Mode, Shaped};
use crate::shell::font_view::sidebar::DEFAULT_SCRIPTS;

pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    ..Dialog::new("dialog.languagesList", "Languages", "fontView.setVisibleScripts", Fields::Shaped(Shaped { fields, given, params, link }))
};

/// The scripts offered, alphabetically: every script of the language data and the default ones.
pub fn scripts() -> Vec<String> {
    let mut out: Vec<String> =
        fontcraft_engine::glyph_data::language_sets().map(|sets| sets.iter().map(|l| l.script.clone()).collect::<Vec<String>>()).unwrap_or_default();
    out.extend(DEFAULT_SCRIPTS.iter().map(|s| (*s).to_string()));
    out.sort();
    out.dedup();
    out.truncate(super::form::MAX_CHOICES);
    out
}

fn fields(s: &fontcraft_engine::Session) -> Vec<Field> {
    let choices = scripts();
    let shown: Vec<String> = s
        .settings()
        .get("fontView.visibleScripts")
        .ok()
        .and_then(|v| v.as_array().map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()))
        .unwrap_or_else(|| DEFAULT_SCRIPTS.iter().map(|x| (*x).to_string()).collect());
    let ticked: Vec<&String> = choices.iter().filter(|c| shown.contains(*c)).collect();
    let data = s.glyph_data().ok();
    let label = |id: &str| data.as_ref().map_or_else(|| id.to_string(), |d| d.script_display_name(id));
    vec![Field {
        key: "scripts".into(),
        label: "Scripts".into(),
        default: json!(ticked),
        kind: Kind::Multi { labels: choices.iter().map(|c| label(c)).collect(), choices: choices.clone() },
        doc: "The scripts the Languages section lists".into(),
        group: None,
    }]
}

/// The ticked scripts, given as `scripts`.
fn given(_: &fontcraft_engine::Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

/// `{show, hide}`: the ticked scripts and the other scripts offered.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let ticked: Vec<String> =
        v.get("scripts").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let hide: Vec<String> = scripts().into_iter().filter(|s| !ticked.contains(s)).collect();
    let mut out = Map::new();
    out.insert("show".into(), json!(ticked));
    out.insert("hide".into(), json!(hide));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}
