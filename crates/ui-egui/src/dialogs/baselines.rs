//! Font › Set Baselines…: the feature code's `BASE` table as text, one group per direction,
//! previewed as `font.setBaselines` runs it. It opens on the font's block.
//!
//! Tags are baseline tags separated by spaces. Scripts have one line per script: the script tag,
//! its default baseline, then one coordinate per tag in the order the tags are typed. Extents
//! have one line per override: script, language, minimum, maximum. A direction whose tag field
//! is empty is removed. A line that does not read is sent as text, so the command's message says
//! what is wrong. A block the command cannot edit opens empty, and OK shows the reason.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::math_base::{BaseAxis, baselines};
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog = Dialog::new("dialog.baselines", "Baselines", "font.setBaselines", Fields::Shaped(Shaped { fields, given, params, link }));

/// Most characters of one text box.
const MAX_TEXT: usize = 64 * 1024;

/// The two directions: the parameter, the field prefix and the group heading.
const AXES: [(&str, &str, &str); 2] = [("horizontal", "h", "Horizontal"), ("vertical", "v", "Vertical")];

fn axis_text(axis: Option<&BaseAxis>) -> [String; 3] {
    let Some(a) = axis else { return Default::default() };
    let scripts = a.scripts.iter().map(|s| {
        let values: Vec<String> = s.values.iter().map(i32::to_string).collect();
        format!("{} {} {}", s.script, s.baseline, values.join(" "))
    });
    let extents = a.extents.iter().map(|e| format!("{} {} {} {}", e.script, e.language, e.min, e.max));
    [a.tags.join(" "), scripts.collect::<Vec<_>>().join("\n"), extents.collect::<Vec<_>>().join("\n")]
}

fn fields(session: &Session) -> Vec<Field> {
    let current = session.font().ok().and_then(|f| baselines(f).ok().flatten()).map(|(_, b)| b).unwrap_or_default();
    let mut out = Vec::new();
    for (key, prefix, group) in AXES {
        let axis = if key == "horizontal" { current.horizontal.as_ref() } else { current.vertical.as_ref() };
        let [tags, scripts, extents] = axis_text(axis);
        out.push(
            Field {
                default: json!(tags),
                ..Field::text(
                    &format!("{prefix}Tags"),
                    "Tags",
                    MAX_TEXT,
                    "Baseline tags separated by spaces (romn, ideo, hang…); empty removes this direction",
                )
            }
            .in_group(group),
        );
        out.push(
            Field {
                default: json!(scripts),
                ..Field::text_area(
                    &format!("{prefix}Scripts"),
                    "Scripts",
                    MAX_TEXT,
                    6,
                    "One line per script: script tag, default baseline tag, one coordinate per tag",
                )
            }
            .in_group(group),
        );
        out.push(
            Field {
                default: json!(extents),
                ..Field::text_area(
                    &format!("{prefix}Extents"),
                    "Extents",
                    MAX_TEXT,
                    4,
                    "One line per override: script tag, language tag, minimum, maximum",
                )
            }
            .in_group(group),
        );
    }
    out
}

fn given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

/// A whole number, else the text itself so the command names it.
fn number_or_text(word: &str) -> Value {
    word.parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| json!(word), |n| json!(n))
}

fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|w| !w.is_empty())
}

/// The command's parameters from the field values: both directions, each an axis or null.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let text = |key: String| v.get(&key).and_then(Value::as_str).unwrap_or("").to_string();
    let mut out = Map::new();
    for (key, prefix, _) in AXES {
        let tags: Vec<String> = text(format!("{prefix}Tags")).split_whitespace().map(str::to_string).collect();
        if tags.is_empty() {
            out.insert(key.into(), Value::Null);
            continue;
        }
        let scripts: Vec<Value> = lines(&text(format!("{prefix}Scripts")))
            .map(|w| {
                let (script, baseline) = (w.first().copied().unwrap_or(""), w.get(1).copied().unwrap_or(""));
                let values: Vec<Value> = w.iter().skip(2).map(|x| number_or_text(x)).collect();
                json!({"script": script, "baseline": baseline, "values": values})
            })
            .collect();
        let extents: Vec<Value> = lines(&text(format!("{prefix}Extents")))
            .map(|w| {
                let at = |i: usize| w.get(i).copied().unwrap_or("");
                json!({"script": at(0), "language": at(1), "min": number_or_text(at(2)), "max": number_or_text(at(3))})
            })
            .collect();
        out.insert(key.into(), json!({"tags": tags, "scripts": scripts, "extents": extents}));
    }
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}
