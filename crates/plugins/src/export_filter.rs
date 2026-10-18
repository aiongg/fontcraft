//! The `Filter` and `PreFilter` export parameter string, and one filter module run on one layer.
//!
//! Form: `Name; key:value; bare value; …; include:A,B` or `exclude:A,B`, parts separated by `;`
//! with surrounding spaces trimmed and empty parts ignored. The first part names the filter: a
//! built-in filter of the engine, or an installed module by id or name. `include:` and `exclude:`
//! take glyph names separated by spaces or commas and are applied by FontCraft, not passed to the
//! module (a guess recorded on `filter.customParameter.thirdParty`). `key:value` parts become
//! parameters by name (exact, else ASCII case ignored); bare values become parameters by
//! position: to the parameters named `0`, `1`, … when the manifest declares such names, else to
//! the declared parameters in order. Values are converted to the parameter's type (booleans as
//! `1`/`0`, `true`/`false` or `yes`/`no`) and then checked like any module parameters.
//!
//! [`run_on_layer`] is the one way a filter module changes a layer: `plugin.runFilter` and the
//! export filter step both use it, so a module gives the same outline from the menu and at
//! export. Rows: `filter.customParameter.thirdParty`, `filter.menu.pluginFiltersListed` (the
//! export half).

use fontcraft_model::{Font, Glyph, Layer};
use serde_json::{Map, Value, json};

pub use crate::error::{MAX_FILTER_PARTS, MAX_FILTER_STRING_BYTES};
use crate::layer_json::{FilterOutput, LayerInput};
use crate::manifest::{Kind, Manifest, ParamSpec};
use crate::runtime::Plugin;
use crate::{Error, Result};

/// Which glyphs an export filter runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GlyphScope {
    /// Every glyph.
    #[default]
    All,
    /// Only these glyphs (`include:`).
    Include(Vec<String>),
    /// Every glyph but these (`exclude:`).
    Exclude(Vec<String>),
}

/// One parsed filter string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterCall {
    /// The filter or module name, as written.
    pub name: String,
    /// `key:value` parts, in order.
    pub named: Vec<(String, String)>,
    /// Bare values, in order.
    pub positional: Vec<String>,
    pub scope: GlyphScope,
}

/// Parses a filter string of at most [`MAX_FILTER_STRING_BYTES`] and [`MAX_FILTER_PARTS`] parts.
/// An empty name, both `include:` and `exclude:`, or a part over the limits is [`Error::Syntax`].
pub fn parse_filter_string(text: &str) -> Result<FilterCall> {
    if text.len() > MAX_FILTER_STRING_BYTES {
        return Err(Error::Syntax(format!("filter string is {} bytes (limit {MAX_FILTER_STRING_BYTES})", text.len())));
    }
    let mut parts = text.split(';').map(str::trim).filter(|s| !s.is_empty());
    let name = parts.next().ok_or_else(|| Error::Syntax("the filter string is empty".into()))?;
    if name.contains(':') {
        return Err(Error::Syntax(format!("the filter string must start with a filter or module name, not \"{name}\"")));
    }
    let mut call = FilterCall { name: name.to_string(), ..FilterCall::default() };
    let (mut include, mut exclude): (Option<Vec<String>>, Option<Vec<String>>) = (None, None);
    for (count, part) in parts.enumerate() {
        // The name is one part too.
        if count.saturating_add(2) > MAX_FILTER_PARTS {
            return Err(Error::Syntax(format!("the filter string holds more than {MAX_FILTER_PARTS} parts")));
        }
        let Some((key, value)) = part.split_once(':') else {
            call.positional.push(part.to_string());
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() {
            return Err(Error::Syntax(format!("\"{part}\" has no key before its colon")));
        }
        let scope = if key.eq_ignore_ascii_case("include") {
            &mut include
        } else if key.eq_ignore_ascii_case("exclude") {
            &mut exclude
        } else {
            if call.named.iter().any(|(k, _)| k == key) {
                return Err(Error::Syntax(format!("`{key}` is given twice")));
            }
            call.named.push((key.to_string(), value.to_string()));
            continue;
        };
        if scope.is_some() {
            return Err(Error::Syntax(format!("`{key}` is given twice")));
        }
        let names: Vec<String> = value.split([' ', ',']).filter(|n| !n.is_empty()).map(str::to_string).collect();
        if names.is_empty() {
            return Err(Error::Syntax(format!("`{key}` names no glyph")));
        }
        *scope = Some(names);
    }
    call.scope = match (include, exclude) {
        (Some(_), Some(_)) => return Err(Error::Syntax("give `include` or `exclude`, not both".into())),
        (Some(names), None) => GlyphScope::Include(names),
        (None, Some(names)) => GlyphScope::Exclude(names),
        (None, None) => GlyphScope::All,
    };
    Ok(call)
}

impl FilterCall {
    /// The parameter object for `manifest`: named values by key, bare values by position (see
    /// the module docs), each converted to the parameter's type. Then checked by
    /// [`Manifest::validate_params`], so missing parameters take their defaults and numbers are
    /// clamped into range. A key the manifest does not declare, a value given twice, more bare
    /// values than parameters or a value of the wrong type is an [`Error::Params`].
    pub fn params_for(&self, manifest: &Manifest) -> Result<Map<String, Value>> {
        let declared = || manifest.params.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ");
        let mut given = Map::new();
        for (key, text) in &self.named {
            let (name, spec) = manifest
                .params
                .iter()
                .find(|(k, _)| k == key)
                .or_else(|| manifest.params.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
                .ok_or_else(|| Error::Params(format!("module {:?} has no parameter `{key}` (its parameters: {})", manifest.id, declared())))?;
            put(&mut given, name, spec, text)?;
        }
        let numbered = manifest.params.iter().any(|(k, _)| k.bytes().all(|b| b.is_ascii_digit()));
        for (i, text) in self.positional.iter().enumerate() {
            let entry = if numbered {
                let index = i.to_string();
                manifest.params.iter().find(|(k, _)| *k == index)
            } else {
                manifest.params.get(i)
            };
            let (name, spec) =
                entry.ok_or_else(|| Error::Params(format!("module {:?} takes no value at position {i}; \"{text}\" is one too many", manifest.id)))?;
            put(&mut given, name, spec, text)?;
        }
        manifest.validate_params(&Value::Object(given))
    }

    /// Whether the filter runs on `glyph`.
    pub fn applies_to(&self, glyph: &str) -> bool {
        match &self.scope {
            GlyphScope::All => true,
            GlyphScope::Include(names) => names.iter().any(|n| n == glyph),
            GlyphScope::Exclude(names) => !names.iter().any(|n| n == glyph),
        }
    }
}

/// Adds `text` as the value of parameter `key`; a parameter given twice is refused.
fn put(given: &mut Map<String, Value>, key: &str, spec: &ParamSpec, text: &str) -> Result<()> {
    if given.contains_key(key) {
        return Err(Error::Params(format!("`{key}` is given twice")));
    }
    let wrong = |what: &str| Error::Params(format!("`{key}` must be {what}, not \"{text}\""));
    let value = match spec {
        ParamSpec::Number { .. } | ParamSpec::Int { .. } => {
            json!(text.parse::<f64>().ok().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?)
        }
        ParamSpec::Bool { .. } => match text.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => json!(true),
            "0" | "false" | "no" => json!(false),
            _ => return Err(wrong("1 or 0")),
        },
        // `validate_params` checks it is one of the options.
        ParamSpec::Choice { .. } => json!(text),
    };
    given.insert(key.to_string(), value);
    Ok(())
}

/// Runs the filter module `plugin` once on `layer` of `glyph` in `font` with `params` (from
/// [`FilterCall::params_for`] or [`Manifest::validate_params`]). Returns the layer as the module
/// left it, every value checked against the model's limits, and the module's log lines. Nothing
/// is changed here; the caller applies the layers of one run in one edit.
pub fn run_on_layer(plugin: &Plugin, font: &Font, glyph: &Glyph, layer: &Layer, params: &Map<String, Value>) -> Result<(Layer, Vec<String>)> {
    if plugin.kind() != Kind::Filter {
        return Err(Error::Params(format!("module {:?} is a {} module, not a filter", plugin.id(), plugin.kind().name())));
    }
    let input = LayerInput::from_layer(font, glyph, layer)?.to_json()?;
    let params = serde_json::to_vec(params).map_err(|e| Error::Params(e.to_string()))?;
    let output = FilterOutput::parse(&plugin.call(&input, &params)?)?;
    let new = output.apply(layer)?;
    Ok((new, output.log))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use fontcraft_model::{Node, NodeKind, Path, Shape};

    use super::*;
    use crate::runtime::Limits;

    fn manifest(params: &str) -> Manifest {
        Manifest::parse(crate::wat::manifest("org.test.m", "filter", params).as_bytes()).unwrap()
    }

    #[test]
    fn a_filter_string_gives_the_name_named_and_bare_values_and_the_glyph_scope() {
        let call = parse_filter_string(" Shift ; dx:10 ;5; include: A,B C ;").unwrap();
        assert_eq!(call.name, "Shift");
        assert_eq!(call.named, vec![("dx".to_string(), "10".to_string())]);
        assert_eq!(call.positional, vec!["5".to_string()]);
        assert_eq!(call.scope, GlyphScope::Include(vec!["A".into(), "B".into(), "C".into()]));
        assert!(call.applies_to("B") && !call.applies_to("O"));
        let call = parse_filter_string("org.example.round; EXCLUDE:a").unwrap();
        assert!(!call.applies_to("a") && call.applies_to("b"));

        for bad in ["", " ; ", "dx:1; Shift", "Shift; include:a; exclude:b", "Shift; :3", "Shift; include: , ", "Shift; dx:1; dx:2"] {
            assert!(matches!(parse_filter_string(bad), Err(Error::Syntax(_))), "{bad:?}");
        }
        let long = format!("Shift;{}", "1;".repeat(MAX_FILTER_PARTS));
        assert!(matches!(parse_filter_string(&long), Err(Error::Syntax(m)) if m.contains("parts")));
        let huge = format!("Shift; include:{}", "a,".repeat(MAX_FILTER_STRING_BYTES));
        assert!(matches!(parse_filter_string(&huge), Err(Error::Syntax(m)) if m.contains("bytes")));
    }

    #[test]
    fn filter_custom_parameter_third_party_values_are_typed_by_name_and_by_position() {
        let m = manifest(
            r#"{"amount":{"type":"number","min":0,"max":100,"default":50},"steps":{"type":"int","default":2},
                "mono":{"type":"bool"},"mode":{"type":"choice","options":["soft","hard"]}}"#,
        );
        // Bare values fill the declared parameters in order; keys ignore ASCII case.
        let p = parse_filter_string("M; 12.5; 3.6; MODE:hard").unwrap().params_for(&m).unwrap();
        assert_eq!(Value::Object(p), json!({"amount": 12.5, "steps": 4, "mono": false, "mode": "hard"}));
        let p = parse_filter_string("M; mono:1; amount:500").unwrap().params_for(&m).unwrap();
        assert_eq!((p["mono"].clone(), p["amount"].clone()), (json!(true), json!(100.0)), "numbers are clamped");

        for (text, says) in [
            ("M; size:3", "no parameter `size`"),
            ("M; 1; amount:2", "given twice"),
            ("M; 1; 2; 0; soft; extra", "one too many"),
            ("M; amount:lots", "must be a number"),
            ("M; amount:NaN", "must be a number"),
            ("M; mono:maybe", "must be 1 or 0"),
            ("M; mode:medium", "one of soft, hard"),
        ] {
            let e = parse_filter_string(text).unwrap().params_for(&m).unwrap_err();
            assert!(e.to_string().contains(says), "{text}: {e}");
        }

        // A manifest that names its parameters `0`, `1` takes bare values by those names.
        let numbered = manifest(r#"{"1":{"type":"int"},"0":{"type":"int"}}"#);
        let p = parse_filter_string("M; 7; 9").unwrap().params_for(&numbered).unwrap();
        assert_eq!(Value::Object(p), json!({"0": 7, "1": 9}));
    }

    #[test]
    fn a_module_named_by_a_filter_string_shifts_the_layer_by_its_parameters() {
        let plugin = Plugin::load(&::wat::parse_str(crate::wat::shift_filter("Shift")).unwrap(), Limits::default()).unwrap();
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes = vec![Shape::Path(Path::new(true, vec![n(100.0, 0.0), n(500.0, 0.0), n(500.0, 700.0)]))];
        let mut glyph = Glyph::new("A");
        glyph.layers = vec![layer.clone()];
        let font = Font::new("Test");

        let call = parse_filter_string("Shift; dx:10; include:A,B").unwrap();
        let params = call.params_for(plugin.manifest()).unwrap();
        let (moved, _) = run_on_layer(&plugin, &font, &glyph, &layer, &params).unwrap();
        let Some(Shape::Path(p)) = moved.shapes.first() else { panic!("no path") };
        let xs: Vec<f64> = p.nodes.iter().map(|n| n.x).collect();
        assert_eq!(xs, vec![110.0, 510.0, 510.0]);
        assert_eq!(moved.width, 600.0);

        // A bare value goes to the first declared parameter, `dx`.
        let params = parse_filter_string("Shift; -20").unwrap().params_for(plugin.manifest()).unwrap();
        let (moved, _) = run_on_layer(&plugin, &font, &glyph, &layer, &params).unwrap();
        let Some(Shape::Path(p)) = moved.shapes.first() else { panic!("no path") };
        assert_eq!(p.nodes.first().map(|n| n.x), Some(80.0));
    }
}
