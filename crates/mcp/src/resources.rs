//! Resources (`resources/list`, `resources/templates/list`, `resources/read`) and argument
//! completion (`completion/complete`): an agent reads one glyph, command, custom parameter or
//! feature instead of the whole font, and completes names as it types them.
//!
//! Every read goes through query commands, so the server's policy applies to resources as it
//! does to `run_command`.

use serde_json::{Value, json};

pub const SCHEME: &str = "fontcraft://";
/// Longest resource URI or completion reference, in bytes.
pub const MAX_URI_LEN: usize = 4096;
/// Most values in one completion (the protocol's own limit).
pub const MAX_COMPLETION_VALUES: usize = 100;
/// Longest text being completed, in bytes.
pub const MAX_COMPLETION_INPUT: usize = 1024;
/// Most master layers a glyph resource includes.
pub const MAX_GLYPH_LAYERS: usize = 64;

const JSON: &str = "application/json";

/// Where completion values come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    GlyphNames,
    CommandIds,
    ParameterNames,
    FeatureTags,
    MasterNames,
}

/// What resources and completions read through. The server answers each call under its policy.
pub(crate) trait Reader {
    /// Runs a query command; `Err` is the reason (refused, failed).
    fn query(&mut self, command: &str, params: Value) -> Result<Value, String>;
    /// The open documents (`document.inspect`).
    fn document(&mut self) -> Result<Value, String>;
    /// The catalogue entries of the commands the policy admits.
    fn admitted_commands(&mut self) -> Result<Vec<Value>, String>;
}

/// A resource the server can read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resource {
    Document,
    Glyph(String),
    Command(String),
    Parameter(String),
    Feature(String),
}

struct Template {
    prefix: &'static str,
    uri_template: &'static str,
    variable: &'static str,
    name: &'static str,
    description: &'static str,
    source: Source,
}

const TEMPLATES: &[Template] = &[
    Template {
        prefix: "glyph/",
        uri_template: "fontcraft://glyph/{name}",
        variable: "name",
        name: "glyph",
        description: "One glyph of the active font: its properties and, for each master, its layer with the address of every shape, node and anchor.",
        source: Source::GlyphNames,
    },
    Template {
        prefix: "command/",
        uri_template: "fontcraft://command/{id}",
        variable: "id",
        name: "command",
        description: "One command: label, menu path, shortcut, its parameters and result, and whether it is a query.",
        source: Source::CommandIds,
    },
    Template {
        prefix: "parameter/",
        uri_template: "fontcraft://parameter/{name}",
        variable: "name",
        name: "custom parameter",
        description: "One known custom parameter: its kind, where it may be set, what it does and other spellings.",
        source: Source::ParameterNames,
    },
    Template {
        prefix: "feature/",
        uri_template: "fontcraft://feature/{tag}",
        variable: "tag",
        name: "feature",
        description: "One feature of the active font: its code, whether it is generated automatically, its notes and labels.",
        source: Source::FeatureTags,
    },
];

/// The `resources/list` result: the fixed resources. Glyphs, commands, parameters and features are
/// templates, so a large font does not make a long list.
pub(crate) fn list() -> Value {
    json!({"resources": [{
        "uri": "fontcraft://document",
        "name": "document",
        "title": "Open documents",
        "description": "The open fonts: title, file path, unsaved changes, revision, family name, glyph count, masters, and the undo and redo steps.",
        "mimeType": JSON,
    }]})
}

/// The `resources/templates/list` result.
pub(crate) fn templates() -> Value {
    let list: Vec<Value> =
        TEMPLATES.iter().map(|t| json!({"uriTemplate": t.uri_template, "name": t.name, "description": t.description, "mimeType": JSON})).collect();
    json!({"resourceTemplates": list})
}

pub(crate) fn parse_uri(uri: &str) -> Result<Resource, String> {
    if uri.len() > MAX_URI_LEN {
        return Err(format!("a resource URI may be at most {MAX_URI_LEN} bytes"));
    }
    let Some(rest) = uri.strip_prefix(SCHEME) else {
        return Err(format!("unknown resource {uri}: FontCraft resources start with {SCHEME}"));
    };
    if rest == "document" {
        return Ok(Resource::Document);
    }
    let unknown = || format!("unknown resource {uri}");
    let template = TEMPLATES.iter().find(|t| rest.starts_with(t.prefix)).ok_or_else(unknown)?;
    let value = percent_decode(rest.get(template.prefix.len()..).unwrap_or_default())?;
    if value.is_empty() {
        return Err(unknown());
    }
    Ok(match template.source {
        Source::GlyphNames => Resource::Glyph(value),
        Source::CommandIds => Resource::Command(value),
        Source::ParameterNames => Resource::Parameter(value),
        Source::FeatureTags => Resource::Feature(value),
        Source::MasterNames => return Err(unknown()),
    })
}

/// The `resources/read` result. `Err` means the resource does not exist or cannot be read now.
pub(crate) fn read(uri: &str, resource: &Resource, r: &mut dyn Reader) -> Result<Value, String> {
    let body = match resource {
        Resource::Document => r.document()?,
        Resource::Glyph(name) => glyph(name, r)?,
        Resource::Command(id) => r
            .admitted_commands()?
            .into_iter()
            .find(|c| c.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| format!("no command `{id}` (or this server does not offer it)"))?,
        Resource::Parameter(name) => {
            let found = r.query("font.customParameters.describe", json!({"query": name}))?;
            let wanted = name.to_lowercase();
            let named = |p: &&Value| {
                p.get("name").and_then(Value::as_str).is_some_and(|n| n.to_lowercase() == wanted)
                    || p.get("aliases")
                        .and_then(Value::as_array)
                        .is_some_and(|a| a.iter().any(|x| x.as_str().is_some_and(|n| n.to_lowercase() == wanted)))
            };
            found
                .get("parameters")
                .and_then(Value::as_array)
                .and_then(|list| list.iter().find(named))
                .cloned()
                .ok_or_else(|| format!("no custom parameter named `{name}`"))?
        }
        Resource::Feature(tag) => r.query("font.features.get", json!({"kind": "feature", "name": tag}))?,
    };
    Ok(json!({"contents": [{"uri": uri, "mimeType": JSON, "text": body.to_string()}]}))
}

fn glyph(name: &str, r: &mut dyn Reader) -> Result<Value, String> {
    let info = r.query("glyph.getInfo", json!({"glyphs": [name]}))?;
    let Some(info) = info.get("glyphs").and_then(Value::as_array).and_then(|g| g.first()).cloned() else {
        return Err(format!("no glyph named `{name}`"));
    };
    let masters = r.query("font.master.list", json!({}))?;
    let mut layers = Vec::new();
    for master in masters.get("masters").and_then(Value::as_array).into_iter().flatten().take(MAX_GLYPH_LAYERS) {
        let Some(id) = master.get("id").and_then(Value::as_str) else { continue };
        layers.push(match r.query("layer.inspect", json!({"glyph": name, "layer": id})) {
            Ok(layer) => layer,
            Err(e) => json!({"master": id, "error": e}),
        });
    }
    Ok(json!({"glyph": info, "layers": layers}))
}

/// The source a resource template's variable completes from.
pub(crate) fn template_source(uri_template: &str, argument: &str) -> Option<Source> {
    TEMPLATES.iter().find(|t| t.uri_template == uri_template && t.variable == argument).map(|t| t.source)
}

/// The `completion/complete` result for `value` from `source`. A value with spaces completes its
/// last word (prompt arguments that take several names).
pub(crate) fn complete(source: Source, value: &str, r: &mut dyn Reader) -> Result<Value, String> {
    if value.len() > MAX_COMPLETION_INPUT {
        return Err(format!("the value to complete may be at most {MAX_COMPLETION_INPUT} bytes"));
    }
    let split = value.rfind(char::is_whitespace).map_or(0, |i| i + value.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8));
    let (head, word) = value.split_at_checked(split).unwrap_or(("", value));
    let candidates = names(source, r)?;
    let wanted = word.to_lowercase();
    let matches: Vec<&String> = candidates.iter().filter(|c| c.to_lowercase().starts_with(&wanted)).collect();
    let values: Vec<String> = matches.iter().take(MAX_COMPLETION_VALUES).map(|c| format!("{head}{c}")).collect();
    Ok(json!({"completion": {"values": values, "total": matches.len(), "hasMore": matches.len() > MAX_COMPLETION_VALUES}}))
}

fn names(source: Source, r: &mut dyn Reader) -> Result<Vec<String>, String> {
    let pick = |v: &Value, list: &str, key: &str| -> Vec<String> {
        v.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(|x| x.get(key).and_then(Value::as_str)).map(str::to_string).collect()
    };
    Ok(match source {
        Source::GlyphNames => pick(&r.query("font.listGlyphs", json!({}))?, "glyphs", "name"),
        Source::CommandIds => r.admitted_commands()?.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).map(str::to_string).collect(),
        Source::ParameterNames => pick(&r.query("font.customParameters.describe", json!({}))?, "parameters", "name"),
        Source::FeatureTags => pick(&r.query("font.features.list", json!({"kind": "feature"}))?, "entries", "name"),
        Source::MasterNames => pick(&r.query("font.master.list", json!({}))?, "masters", "name"),
    })
}

/// Decodes `%XX` escapes; the result must be UTF-8.
fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            let Some(decoded) = hex else {
                return Err(format!("`{text}` has a % that is not followed by two hex digits"));
            };
            out.push(decoded);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("`{text}` does not decode to UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_parse_and_decode() {
        assert_eq!(parse_uri("fontcraft://document"), Ok(Resource::Document));
        assert_eq!(parse_uri("fontcraft://glyph/A.alt"), Ok(Resource::Glyph("A.alt".into())));
        assert_eq!(parse_uri("fontcraft://glyph/%C3%9C"), Ok(Resource::Glyph("Ü".into())));
        assert_eq!(parse_uri("fontcraft://feature/liga"), Ok(Resource::Feature("liga".into())));
        for bad in ["http://x/glyph/A", "fontcraft://glyph/", "fontcraft://nope/A", "fontcraft://glyph/%G1", "fontcraft://glyph/%FF"] {
            assert!(parse_uri(bad).is_err(), "{bad}");
        }
        assert!(parse_uri(&format!("fontcraft://glyph/{}", "a".repeat(MAX_URI_LEN))).is_err());
    }
}
