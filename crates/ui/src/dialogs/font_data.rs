//! Font data dialogs: tables edited as text, and the scripts Font View lists.
//!
//! - Baselines (`dialog.baselines`, Font › Set Baselines…): the feature code's `BASE` table, one group per direction,
//!   run by `font.setBaselines` on OK. It opens on the font's block. Tags are baseline tags separated by spaces.
//!   Scripts have one line per script: the script tag, its default baseline, then one coordinate per tag in the order
//!   the tags are typed. Extents have one line per override: script, language, minimum, maximum. A direction whose tag
//!   field is empty is removed. A line that does not read is sent as text, so the command's message says what is
//!   wrong; a block the command cannot edit opens empty, and OK shows the reason.
//! - Math Table (`dialog.mathTable`, Font › Set Math Data…): the `MATH` data of the default master, run by
//!   `font.setMathData` on OK. The constants box holds one `Name value` line per constant and opens on the font's
//!   values. The glyph fields act on the glyph named in Glyph and open on its data when `dialog.open` names one
//!   (`{glyph}`). An empty glyph field leaves that value as it is; Extended Shape has an Unchanged choice for the same
//!   reason. Variants are glyph names separated by spaces; an assembly has one part per line (`glyph flags start end`,
//!   flags 1 for an extender); kerning has one point per line (`topRight kern height`).
//! - Languages (`dialog.languagesList`): one checkbox per script, ticked for the scripts Font View's Languages section
//!   lists, over `fontView.setVisibleScripts`. OK shows the ticked scripts and hides the others; the list lives in the
//!   user settings, so Cancel leaves it as it was. The scripts offered are those of the language data and the default
//!   ones.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::math_base::{AssemblyPart, BaseAxis, GlyphMath, baselines, glyph_math, math_constants};
use serde_json::{Map, Value, json};

use super::fields::{Field, Kind};
use super::spec::{DialogSpec, Fields, Shaped};

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new(
        "dialog.baselines",
        "Baselines",
        "font.setBaselines",
        Fields::Shaped(Shaped { fields: base_fields, given: pass, params: base_params, link: no_link }),
    )
    .run_on_ok(),
    DialogSpec::new(
        "dialog.mathTable",
        "Math Table",
        "font.setMathData",
        Fields::Shaped(Shaped { fields: math_fields, given: math_given, params: math_params, link: no_link }),
    )
    .run_on_ok(),
    DialogSpec::new(
        "dialog.languagesList",
        "Languages",
        "fontView.setVisibleScripts",
        Fields::Shaped(Shaped { fields: script_fields, given: pass, params: script_params, link: no_link }),
    )
    .run_on_ok(),
];

/// The scripts Font View's Languages section lists until the person picks others.
pub const DEFAULT_SCRIPTS: [&str; 3] = ["latin", "greek", "cyrillic"];

/// Most characters of one text box.
const MAX_TEXT: usize = 64 * 1024;

fn no_link(_: &mut Map<String, Value>, _: &str) {}

fn pass(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

/// A number, else the text itself so the command names it.
fn number_or_text(word: &str) -> Value {
    word.parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| json!(word), |n| json!(n))
}

fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|w| !w.is_empty())
}

// ── Baselines ───────────────────────────────────────────────────────────────────────────────────────────────────────

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

fn base_fields(session: &Session) -> Vec<Field> {
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

/// The command's parameters from the field values: both directions, each an axis or null.
fn base_params(v: &Map<String, Value>) -> Map<String, Value> {
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

// ── Math Table ──────────────────────────────────────────────────────────────────────────────────────────────────────

fn math_fields(session: &Session) -> Vec<Field> {
    let constants = session
        .font()
        .ok()
        .and_then(|f| Some(math_constants(f, &f.default_master().ok()?.id)))
        .unwrap_or_default()
        .into_iter()
        .map(|(name, v)| format!("{name} {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let glyph = |f: Field| f.in_group("Glyph");
    vec![
        Field {
            default: json!(constants),
            ..Field::text_area(
                "constants",
                "Constants",
                MAX_TEXT,
                12,
                "One constant per line: its name as the MATH specification writes it, then its value",
            )
        }
        .in_group("Constants"),
        glyph(Field::text("glyph", "Glyph", 255, "The glyph the fields below change; empty changes no glyph")),
        glyph(Field::text("italicCorrection", "Italic correction", 32, "From the advance width; empty leaves it as it is")),
        glyph(Field::text("topAccent", "Top accent", 32, "The accent attachment position from the origin; empty leaves it as it is")),
        glyph(Field::choice(
            "extendedShape",
            "Extended shape",
            &[("keep", "Unchanged"), ("yes", "Yes"), ("no", "No")],
            "keep",
            "Whether the glyph is an extended shape, such as a large operator",
        )),
        glyph(Field::text("vVariants", "Vertical variants", MAX_TEXT, "Glyph names, smallest first, separated by spaces")),
        glyph(Field::text_area("vAssembly", "Vertical assembly", MAX_TEXT, 4, "One part per line, bottom first: glyph flags start end")),
        glyph(Field::text("hVariants", "Horizontal variants", MAX_TEXT, "Glyph names, smallest first, separated by spaces")),
        glyph(Field::text_area("hAssembly", "Horizontal assembly", MAX_TEXT, 4, "One part per line, left first: glyph flags start end")),
        glyph(Field::text_area(
            "kerning",
            "Math kerning",
            MAX_TEXT,
            4,
            "One point per line: topRight, topLeft, bottomRight or bottomLeft, the kern value, the height",
        )),
    ]
}

/// A number as the fields show it.
fn shown(v: f64) -> String {
    super::fields::format_number(v)
}

/// The glyph fields for `m`.
fn glyph_values(m: &GlyphMath) -> Map<String, Value> {
    let parts = |parts: &[AssemblyPart]| {
        parts.iter().map(|p| format!("{} {} {} {}", p.glyph, p.flags, shown(p.start), shown(p.end))).collect::<Vec<_>>().join("\n")
    };
    let kerning: Vec<String> =
        m.kerning.iter().flat_map(|(side, points)| points.iter().map(move |(k, h)| format!("{} {} {}", side.key(), shown(*k), shown(*h)))).collect();
    let mut out = Map::new();
    out.insert("italicCorrection".into(), json!(m.italic_correction.map(shown).unwrap_or_default()));
    out.insert("topAccent".into(), json!(m.top_accent.map(shown).unwrap_or_default()));
    out.insert("extendedShape".into(), json!(if m.extended_shape { "yes" } else { "no" }));
    out.insert("vVariants".into(), json!(m.variants.v_variants.join(" ")));
    out.insert("hVariants".into(), json!(m.variants.h_variants.join(" ")));
    out.insert("vAssembly".into(), json!(parts(&m.variants.v_assembly)));
    out.insert("hAssembly".into(), json!(parts(&m.variants.h_assembly)));
    out.insert("kerning".into(), json!(kerning.join("\n")));
    out
}

/// A glyph named in `dialog.open`'s parameters opens with its data.
fn math_given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = p.clone();
    if let Some(name) = p.get("glyph").and_then(Value::as_str)
        && let Ok(font) = session.font()
        && let Ok(master) = font.default_master()
        && let Some(m) = glyph_math(font, &master.id, name)
    {
        for (key, value) in glyph_values(&m) {
            out.entry(key).or_insert(value);
        }
    }
    Ok(out)
}

fn parts(text: &str) -> Value {
    Value::Array(
        lines(text).map(|w| Value::Array(w.iter().enumerate().map(|(i, x)| if i == 0 { json!(x) } else { number_or_text(x) }).collect())).collect(),
    )
}

/// The command's parameters from the field values.
fn math_params(v: &Map<String, Value>) -> Map<String, Value> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let mut out = Map::new();
    let mut constants = Map::new();
    for words in lines(&text("constants")) {
        if let [name, value, ..] = words.as_slice() {
            constants.insert((*name).to_string(), number_or_text(value));
        } else if let [name] = words.as_slice() {
            constants.insert((*name).to_string(), json!(""));
        }
    }
    if !constants.is_empty() {
        out.insert("constants".into(), Value::Object(constants));
    }
    let glyph = text("glyph");
    if glyph.is_empty() {
        return out;
    }
    let mut g = Map::new();
    for key in ["italicCorrection", "topAccent"] {
        let t = text(key);
        if !t.is_empty() {
            g.insert(key.into(), number_or_text(&t));
        }
    }
    match v.get("extendedShape").and_then(Value::as_str) {
        Some("yes") => {
            g.insert("extendedShape".into(), json!(true));
        }
        Some("no") => {
            g.insert("extendedShape".into(), json!(false));
        }
        _ => {}
    }
    for key in ["vVariants", "hVariants"] {
        let t = text(key);
        if !t.is_empty() {
            g.insert(key.into(), json!(t.split_whitespace().collect::<Vec<_>>()));
        }
    }
    for key in ["vAssembly", "hAssembly"] {
        let t = text(key);
        if !t.is_empty() {
            g.insert(key.into(), parts(&t));
        }
    }
    let k = text("kerning");
    if !k.is_empty() {
        let mut sides: Map<String, Value> = Map::new();
        for words in lines(&k) {
            let (side, rest) = words.split_first().map_or(("", &[][..]), |(s, r)| (*s, r));
            let point: Vec<Value> = rest.iter().map(|x| number_or_text(x)).collect();
            if let Value::Array(list) = sides.entry(side.to_string()).or_insert_with(|| json!([])) {
                list.push(Value::Array(point));
            }
        }
        g.insert("kerning".into(), Value::Object(sides));
    }
    let mut glyphs = Map::new();
    glyphs.insert(glyph, Value::Object(g));
    out.insert("glyphs".into(), Value::Object(glyphs));
    out
}

// ── Languages ───────────────────────────────────────────────────────────────────────────────────────────────────────

/// The scripts offered, alphabetically: every script of the language data and the default ones.
pub fn scripts() -> Vec<String> {
    let mut out: Vec<String> =
        fontcraft_engine::glyph_data::language_sets().map(|sets| sets.iter().map(|l| l.script.clone()).collect::<Vec<String>>()).unwrap_or_default();
    out.extend(DEFAULT_SCRIPTS.iter().map(|s| (*s).to_string()));
    out.sort();
    out.dedup();
    out.truncate(super::fields::MAX_CHOICES);
    out
}

fn script_fields(s: &fontcraft_engine::Session) -> Vec<Field> {
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
        same_as: None,
    }]
}

/// `{show, hide}`: the ticked scripts and the other scripts offered.
fn script_params(v: &Map<String, Value>) -> Map<String, Value> {
    let ticked: Vec<String> =
        v.get("scripts").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let hide: Vec<String> = scripts().into_iter().filter(|s| !ticked.contains(s)).collect();
    let mut out = Map::new();
    out.insert("show".into(), json!(ticked));
    out.insert("hide".into(), json!(hide));
    out
}

#[cfg(test)]
#[path = "font_data_tests.rs"]
mod tests;
