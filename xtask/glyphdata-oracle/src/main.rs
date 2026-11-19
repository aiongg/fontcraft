//! Reads glyph names from stdin, one per line, and writes fontc's answer for each as one JSON
//! line: `name`, `category`, `subCategory`, `production` and `unicode` (null when fontc has
//! none). A query that panics answers `<panic>` in every field.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

const FIELDS: &[&str] = &["category", "subCategory", "production", "unicode"];

fn answer(data: &glyphs_reader::glyphdata::GlyphData, name: &str) -> Value {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| data.query(name, None)));
    let mut out = serde_json::Map::new();
    out.insert("name".into(), json!(name));
    match result {
        Err(_) => {
            for f in FIELDS {
                out.insert((*f).into(), json!("<panic>"));
            }
        }
        Ok(r) => {
            out.insert("category".into(), json!(r.as_ref().map(|r| r.category.to_string())));
            out.insert("subCategory".into(), json!(r.as_ref().and_then(|r| r.subcategory.map(|s| s.to_string()))));
            // fontc exports a glyph without a production name under its own name.
            let production = r.as_ref().and_then(|r| r.production_name.as_ref().map(|p| p.to_string())).unwrap_or_else(|| name.to_string());
            out.insert("production".into(), json!(production));
            out.insert("unicode".into(), json!(r.as_ref().and_then(|r| r.codepoint.map(|c| format!("{c:04X}")))));
        }
    }
    Value::Object(out)
}

fn main() -> Result<(), String> {
    let data = glyphs_reader::glyphdata::GlyphData::new(None);
    std::panic::set_hook(Box::new(|_| {}));
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in std::io::stdin().lock().lines() {
        let name = line.map_err(|e| e.to_string())?;
        writeln!(out, "{}", answer(&data, &name)).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}
