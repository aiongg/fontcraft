//! Font › Set Math Data…: the `MATH` table's data for the first master, as text, previewed as
//! `font.setMathData` runs it.
//!
//! The constants box holds one `Name value` line per constant and opens on the font's values.
//! The glyph fields act on the glyph named in Glyph and open on its data when `dialog.open` names
//! one (`{glyph}`). An empty glyph field leaves that value as it is; Extended Shape has an
//! Unchanged choice for the same reason. Variants are glyph names separated by spaces; an
//! assembly has one part per line (`glyph flags start end`, flags 1 for an extender); kerning has
//! one point per line (`topRight kern height`). A line that does not read is sent as text, so the
//! command's message says what is wrong.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::math_base::{AssemblyPart, GlyphMath, glyph_math, math_constants};
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Dialog, Fields, Shaped};

pub const DIALOG: Dialog = Dialog::new("dialog.mathTable", "Math Table", "font.setMathData", Fields::Shaped(Shaped { fields, given, params, link }));

/// Most characters of one text box.
const MAX_TEXT: usize = 64 * 1024;

fn fields(session: &Session) -> Vec<Field> {
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
    super::form::format_number(v)
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
fn given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
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

/// A number, else the text itself so the command names it.
fn number_or_text(word: &str) -> Value {
    word.parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| json!(word), |n| json!(n))
}

fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|w| !w.is_empty())
}

fn parts(text: &str) -> Value {
    Value::Array(
        lines(text).map(|w| Value::Array(w.iter().enumerate().map(|(i, x)| if i == 0 { json!(x) } else { number_or_text(x) }).collect())).collect(),
    )
}

/// The command's parameters from the field values.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
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

fn link(_: &mut Map<String, Value>, _: &str) {}
