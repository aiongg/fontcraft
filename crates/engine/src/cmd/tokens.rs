//! `font.expandTextTokens`: a text with its `{{{token}}}` tokens expanded in a font, master or instance context, through `fontcraft_fontinfo::tokens`.
//!
//! Package M7.13 (`docs/plans/M7.md` § 4).

use fontcraft_fontinfo::tokens::{self, TokenContext};
use fontcraft_fontinfo::{MAX_FIELD_BYTES, target};
use fontcraft_model::Font;
use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.expandTextTokens",
        "Expand Text Tokens",
        [],
        None,
        "{font?: path, text: string (at most 65,535 bytes), master?: id, name or index, instance?: id, name or index} `text` with its {{{token}}} tokens replaced as an export would replace them: versionString and buildNumber anywhere; glyph:path (width, LSB, RSB, bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height, anchors.<name>.x, anchors.<name>.y) on the master's layer, else the first master's; with `instance` the instance's names and classes, then the font's fields; with `master` alone its number values; otherwise the font's fields (familyName, versionMajor, versionMinor, unitsPerEm, date, copyright, designer, manufacturer, license, description, vendorID…). Reads the active document, or the font file at `font` without opening it. A token that names nothing is kept as written → {text, unresolved: [token], context: \"font\" | \"master\" | \"instance\"}",
        always,
        expand
    ).files(const { &[FileParam::read("font")] })]
}

fn expand(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.expandTextTokens", p)?;
    let text = p.str("text")?;
    if text.len() > MAX_FIELD_BYTES {
        return Err(p.bad(format!("`text` is longer than {MAX_FIELD_BYTES} bytes")));
    }
    let from_file;
    let font = match p.opt_str("font")? {
        Some(path) => {
            from_file = open_file(path)?;
            &from_file
        }
        None => s.font()?,
    };
    let master = match p.get("master") {
        Some(v) => {
            let i = target::parse(v).and_then(|t| target::master(font, &t)).map_err(|e| p.bad(e.to_string()))?;
            Some(font.masters.get(i).ok_or_else(|| p.bad("no such master"))?)
        }
        None => None,
    };
    let instance = match p.get("instance") {
        Some(v) => {
            let i = target::parse(v).and_then(|t| target::instance(font, &t)).map_err(|e| p.bad(e.to_string()))?;
            Some(font.instances.get(i).ok_or_else(|| p.bad("no such instance"))?)
        }
        None => None,
    };
    let width = |glyph: &str, master: &str| font.glyph(glyph).and_then(|g| g.layer(master)).map(|l| l.width);
    let ctx = TokenContext { font, master, instance, glyph_width: &width };
    let out = tokens::expand(text, &ctx);
    let context = if instance.is_some() {
        "instance"
    } else if master.is_some() {
        "master"
    } else {
        "font"
    };
    Ok(json!({ "text": out.text, "unresolved": out.unresolved, "context": context }))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file(path: &str) -> Result<Font> {
    crate::io::open(std::path::Path::new(path), None).map(|(font, _)| font)
}

#[cfg(target_arch = "wasm32")]
fn open_file(_: &str) -> Result<Font> {
    Err(crate::EngineError::Unsupported("reading a file by path in the browser".into()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Instance, NumberDef, Path, Shape};

    use super::*;

    fn session() -> Session {
        let mut f = Font::new("Token Sans");
        let info = Arc::make_mut(&mut f.info);
        info.properties.set("designers", "Ada", true);
        info.properties.set("descriptions", "{{{familyName}}} by {{{designer}}}", true);
        Arc::make_mut(&mut f.numbers).push(NumberDef { name: "gap".into(), ..Default::default() });
        if let Some(m) = Arc::make_mut(&mut f.masters).first_mut() {
            m.number_values = vec![40.0];
        }
        if let Ok(mut g) = f.glyph_mut("H")
            && let Some(l) = g.layer_mut("m01")
        {
            l.shapes.push(Shape::Path(Path::rect(60.0, 0.0, 540.0, 700.0)));
        }
        Arc::make_mut(&mut f.instances).push(Instance { name: "Bold".into(), id: Some("i1".into()), ..Default::default() });
        let mut s = Session::new();
        s.push_document(f, None);
        s
    }

    fn run(s: &mut Session, p: Value) -> Value {
        s.execute("font.expandTextTokens", &p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    #[test]
    fn expand_text_tokens_in_each_context() {
        let mut s = session();
        let before = s.font().cloned().ok();
        let out = run(&mut s, json!({"text": "{{{familyName}}} {{{H:LSB}}} {{{H:width}}} {{{gap}}} {{{nonsense}}}"}));
        assert_eq!(out["text"], "Token Sans 60 600 {{{gap}}} {{{nonsense}}}");
        assert_eq!(out["unresolved"], json!(["gap", "nonsense"]));
        assert_eq!(out["context"], "font");

        let out = run(&mut s, json!({"text": "{{{gap}}} {{{H:RSB}}}", "master": "m01"}));
        assert_eq!((out["text"].as_str(), out["context"].as_str()), (Some("40 60"), Some("master")));

        let out = run(&mut s, json!({"text": "{{{fullName}}}, {{{designer}}}, {{{versionString}}}", "instance": "i1"}));
        assert_eq!(out["text"], format!("Token Sans Bold, Ada, {}", tokens::APP_VERSION));
        assert_eq!(out["context"], "instance");

        // A field's own tokens are expanded once, not again inside a value.
        let out = run(&mut s, json!({"text": "{{{description}}}"}));
        assert_eq!(out["text"], "{{{familyName}}} by {{{designer}}}");

        // A query: the font is the same object as before.
        assert_eq!(s.font().cloned().ok(), before);
    }

    #[test]
    fn expand_text_tokens_refuses_bad_input() {
        let mut s = session();
        for p in [
            json!({}),
            json!({"text": 5}),
            json!({"text": "x".repeat(MAX_FIELD_BYTES + 1)}),
            json!({"text": "{{{gap}}}", "master": "nope"}),
            json!({"text": "{{{name}}}", "instance": "nope"}),
        ] {
            assert!(s.execute("font.expandTextTokens", &p).is_err(), "{p}");
        }
        assert!(Session::new().execute("font.expandTextTokens", &json!({"text": "a"})).is_err());
    }
}
