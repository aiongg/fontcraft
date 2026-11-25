//! The dialogs that make documents by interpolation. Each runs its command once, on OK.
//!
//! | Dialog | Command |
//! |---|---|
//! | `dialog.interpolateFonts` | `font.interpolateFonts`: two open fonts of one master and the percentage between them |
//! | `dialog.generateInstances` | `file.generateInstances`: the static instances, whether their documents open, and a folder to save them in |

use fontcraft_engine::Session;
use serde_json::{Map, Value, json};

use super::super::fields::{Field, Kind, MAX_CHOICES};
use super::super::spec::{DialogSpec, Fields, Shaped};
use super::{OpenFonts, no_link};

/// The percentage the command takes, either side of 0.
const MAX_AMOUNT: f64 = 100_000.0;

pub const INTERPOLATE_FONTS: DialogSpec = DialogSpec::new(
    "dialog.interpolateFonts",
    "Interpolate Fonts",
    "font.interpolateFonts",
    Fields::Shaped(Shaped { fields: blend_fields, given: blend_given, params: blend_params, link: no_link }),
)
.run_on_ok()
.buttons("Interpolate", "");

pub const GENERATE_INSTANCES: DialogSpec = DialogSpec::new(
    "dialog.generateInstances",
    "Generate Instances",
    "file.generateInstances",
    Fields::Shaped(Shaped { fields: generate_fields, given: generate_given, params: generate_params, link: no_link }),
)
.run_on_ok()
.buttons("Generate", "");

fn single_master_fonts(session: &Session) -> OpenFonts {
    OpenFonts::of(session, |d| d.font.masters.len() == 1)
}

fn blend_fields(session: &Session) -> Vec<Field> {
    let fonts = single_master_fonts(session);
    let (choices, labels) = (&fonts.choices, &fonts.labels);
    let font = |key: &str, label: &str, at: usize, doc: &str| Field {
        key: key.into(),
        label: label.into(),
        default: choices.get(at).or(choices.first()).map_or(Value::Null, |c| json!(c)),
        kind: Kind::Choice { choices: choices.clone(), labels: labels.clone() },
        doc: format!("{doc}{}", fonts.note()),
        group: None,
        same_as: None,
    };
    vec![
        font("a", "From", 0, "The font at 0 percent: the new font copies everything that does not interpolate from it"),
        font("b", "To", 1, "The font at 100 percent: the same glyphs, each with the same shapes in the same order"),
        Field::number(
            "amount",
            "Percentage",
            -MAX_AMOUNT,
            MAX_AMOUNT,
            50.0,
            "0 gives the first font, 100 the second; below 0 or above 100 extrapolates",
        ),
    ]
}

fn blend_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let fonts = single_master_fonts(session);
    if fonts.choices.len() < 2 {
        return Err(format!("Interpolate Fonts: open two fonts with one master each{}", fonts.note()));
    }
    Ok(given.clone())
}

fn blend_params(values: &Map<String, Value>) -> Map<String, Value> {
    ["a", "b", "amount"].into_iter().filter_map(|key| Some((key.to_string(), values.get(key)?.clone()))).collect()
}

/// The static instances as (index, label).
fn statics(session: &Session) -> Vec<(String, String, bool)> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.instances
        .iter()
        .enumerate()
        .filter(|(_, i)| i.kind.is_none())
        .take(MAX_CHOICES)
        .map(|(at, i)| (at.to_string(), if i.name.is_empty() { format!("#{at}") } else { i.name.clone() }, i.exports))
        .collect()
}

fn generate_fields(session: &Session) -> Vec<Field> {
    let list = statics(session);
    let active: Vec<&String> = list.iter().filter(|(_, _, active)| *active).map(|(at, _, _)| at).collect();
    vec![
        Field {
            key: "instances".into(),
            label: "Instances".into(),
            default: json!(active),
            kind: Kind::Multi {
                choices: list.iter().map(|(at, _, _)| at.clone()).collect(),
                labels: list.iter().map(|(_, name, _)| name.clone()).collect(),
            },
            doc: "The static instances to make documents of".into(),
            group: None,
            same_as: None,
        },
        Field::bool("open", "Open the Documents", true, "Open each new document; the last one becomes active"),
        Field::folder("destination", "Save In", "Also save each document into this folder; empty saves nothing"),
    ]
}

fn generate_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    session.font().map_err(|e| e.to_string())?;
    if statics(session).is_empty() {
        return Err("Generate Instances: the font has no static instance".into());
    }
    Ok(given.clone())
}

fn generate_params(values: &Map<String, Value>) -> Map<String, Value> {
    let picked: Vec<u64> = values
        .get("instances")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_default();
    let mut m = Map::new();
    m.insert("instances".into(), json!(picked));
    m.insert("open".into(), json!(values.get("open").and_then(Value::as_bool).unwrap_or(true)));
    if let Some(folder) = values.get("destination").and_then(Value::as_str).map(str::trim).filter(|f| !f.is_empty()) {
        m.insert("destination".into(), json!(folder));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(fields: Vec<Field>) -> Map<String, Value> {
        fields.into_iter().map(|f| (f.key, f.default)).collect()
    }

    /// A font the command would take for another, of the same title, is no choice; saved ones are told apart by path.
    #[test]
    fn fonts_of_one_title_are_not_confused() {
        use fontcraft_engine::model::Font;
        let mut s = Session::new();
        s.add_document(Font::new("One"), Some("/fonts/a/Sans.glyphs".into())).unwrap();
        s.add_document(Font::new("Two"), Some("/fonts/b/Sans.glyphs".into())).unwrap();
        s.add_document(Font::new("Three"), None).unwrap();
        let untitled = s.doc().unwrap().title.clone();
        s.add_document(Font::new("Four"), Some(format!("/fonts/{untitled}.glyphs").into())).unwrap();
        let fonts = single_master_fonts(&s);
        assert_eq!(fonts.choices.len(), 4, "{fonts:?}");
        assert_eq!(fonts.labels[..2], fonts.choices[..2], "told apart by path");
        assert!(!fonts.left_out);

        let mut s = Session::new();
        s.add_document(Font::new("Five"), Some(format!("/fonts/{untitled}.glyphs").into())).unwrap();
        s.add_document(Font::new("Six"), None).unwrap();
        assert_eq!(s.doc().unwrap().title, untitled);
        let fonts = single_master_fonts(&s);
        assert_eq!((fonts.choices.len(), fonts.left_out), (1, true), "the title names the first font: {fonts:?}");
        assert!(blend_given(&s, &Map::new()).is_err_and(|e| e.contains("save it")));
    }

    /// Each dialog opens on parameters its command takes.
    #[test]
    fn the_defaults_run_the_command() {
        let mut s = Session::with_document();
        assert!(blend_given(&s, &Map::new()).is_err(), "one font is not enough");
        assert!(generate_given(&s, &Map::new()).is_ok() != statics(&s).is_empty());
        s.execute("font.instance.add", &json!({"styleName": "One"})).unwrap();
        s.execute("font.instance.add", &json!({"styleName": "Two"})).unwrap();
        s.execute("font.instance.setActive", &json!({"instances": ["Two"], "active": false})).unwrap();
        let params = generate_params(&defaults(generate_fields(&s)));
        let before = s.documents().len();
        let made = s.execute("file.generateInstances", &Value::Object(params.clone())).unwrap();
        let picked = params["instances"].as_array().map_or(0, Vec::len);
        assert_eq!(made["documents"].as_array().map_or(0, Vec::len), picked);
        assert_eq!(s.documents().len(), before + picked);
        let params = blend_params(&defaults(blend_fields(&s)));
        assert_eq!(params["amount"], 50.0);
        assert_ne!(params["a"], params["b"]);
        s.execute("font.interpolateFonts", &Value::Object(params)).unwrap();
    }
}
