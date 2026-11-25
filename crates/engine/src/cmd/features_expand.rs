//! What feature code is for one build (M8.52): `font.features.expandCode`, the code a build compiles, and
//! `font.features.evaluateToken`, what a token of it evaluates to.
//!
//! Both read a build as `font.features.compile`'s `targets` does (`features_targets::at_target`): an instance between the
//! masters is interpolated. A number token is evaluated on a font reduced to the glyphs it can read, so its value at every
//! instance of a large font takes no more than a few glyphs' interpolation.
//!
//! [`expand_font`] runs on a font and a [`CheckContext`] without the session, so the Features tab runs it on a worker.

use std::borrow::Cow;
use std::collections::HashSet;

use fontcraft_compile::var::{VariableOptions, compile_variable};
use fontcraft_compile::{DataRef, Flavour, Options};
use fontcraft_features::tokens::{self, TokenValue};
use fontcraft_features::{FeatureError, MasterValues, assemble};
use fontcraft_fontinfo::target as lists;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Shape};
use serde_json::{Map, Value, json};

use super::features_check::{CheckContext, check_context};
use super::features_compile::{feature_error, master_id};
use super::features_targets::{CheckTarget, MAX_TARGETS, at_target, targets_param};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most components followed for the glyphs a token reads.
const MAX_CLOSURE_STEPS: usize = 100_000;

/// The build `target` names: `{master}`, `{instance}` or `{variable: true}`; the first master when it names none.
fn target_of(p: &Params, font: &Font, target: Option<&Map<String, Value>>) -> Result<CheckTarget> {
    let Some(map) = target.filter(|m| m.values().any(|v| !v.is_null())) else {
        return Ok(CheckTarget::Master(master_id(p, font, None)?));
    };
    let mut named = map.iter().filter(|(_, v)| !v.is_null());
    let (Some((key, value)), None) = (named.next(), named.next()) else {
        return Err(p.bad("`target` is one of {master}, {instance} and {variable: true}"));
    };
    match (key.as_str(), value) {
        ("master", Value::String(m)) => Ok(CheckTarget::Master(master_id(p, font, Some(m))?)),
        ("master", other) => Err(p.bad(format!("`target.master` must be a string, got {}", crate::params::kind(other)))),
        ("instance", v) => instance_of(p, font, "target.instance", v),
        ("variable", Value::Bool(true)) => Ok(CheckTarget::Variable),
        ("variable", _) => Err(p.bad("`target.variable` is true or absent")),
        (key, _) => Err(p.bad(format!("`target` takes `master`, `instance` or `variable`, found `{key}`"))),
    }
}

/// The static instance `value` names.
fn instance_of(p: &Params, font: &Font, key: &str, value: &Value) -> Result<CheckTarget> {
    let at = lists::parse(value).and_then(|r| lists::instance(font, &r)).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    if font.instances.get(at).is_some_and(|inst| inst.kind.is_some()) {
        return Err(p.bad(format!("`{key}` names a variable font setting: use {{variable: true}} for the variable font")));
    }
    Ok(CheckTarget::Instance(at))
}

/// `target` as `font.features.expandCode` takes it.
fn target_json(target: &CheckTarget) -> Value {
    match target {
        CheckTarget::Master(id) => json!({ "master": id }),
        CheckTarget::Instance(index) => json!({ "instance": index }),
        CheckTarget::Variable => json!({ "variable": true }),
    }
}

pub(super) fn expand_code(s: &mut Session, raw: &Value) -> Result<Value> {
    let cx = check_context(s)?;
    expand_font(s.font()?, &cx, raw)
}

/// `font.features.expandCode` on `font`: the same parameters and answer as the query.
pub fn expand_font(font: &Font, cx: &CheckContext, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.expandCode", raw)?;
    let generated = p.bool_or("includeImplicit", false)?;
    let target = target_of(&p, font, p.opt_object("target")?)?;
    let name = target.label(font);
    if generated {
        let code = exported_code(font, cx, &target)?;
        return Ok(json!({ "target": target_json(&target), "name": name, "code": code, "refused": [] }));
    }
    let at = at_target(font, &font.features, &cx.glyph_data, &target, false, |made| {
        assemble(made.font, made.features, &made.target, made.values).map(|a| a.expanded_text())
    });
    let code = at.read.map_err(|message| p.bad(message))?.map_err(|e| code_error(&p, e))?;
    Ok(json!({ "target": target_json(&target), "name": name, "code": code, "refused": at.refused }))
}

/// Code that cannot be expanded: the first error in it, where there is one.
fn code_error(p: &Params, e: FeatureError) -> EngineError {
    match &e {
        FeatureError::Code(found) => match found.first() {
            Some(first) => p.bad(format!("the code cannot be expanded: {}", first.diagnostic.message)),
            None => feature_error(p, e),
        },
        _ => feature_error(p, e),
    }
}

/// The feature code an export of `target` compiles: the automatic code regenerated for the glyphs it exports, and the code
/// of the generated contextual mark attachment after it. Kerning and the other mark attachment are generated as lookups,
/// not as code.
fn exported_code(font: &Font, cx: &CheckContext, target: &CheckTarget) -> Result<String> {
    let options = Options {
        glyph_data: DataRef(Some(cx.glyph_data.clone())),
        feature_dir: cx.include_dir.clone(),
        include_check: crate::file_access::include_check(),
        ..Options::default()
    };
    let static_code = |target: fontcraft_model::Target| -> Result<Option<String>> {
        let with_axes = fontcraft_compile::with_instance_feature_axes(&options, font, &target);
        Ok(fontcraft_compile::plan::plan(font, &target, Flavour::TrueType, &with_axes)?.features)
    };
    let code = match target {
        CheckTarget::Master(id) => static_code(fontcraft_model::Target::Master(id.clone()))?,
        CheckTarget::Instance(index) => static_code(fontcraft_model::Target::Instance(*index))?,
        CheckTarget::Variable => compile_variable(font, None, &options, &VariableOptions::default())?.compiled.feature_code,
    };
    Ok(code.unwrap_or_default())
}

/// `font` with the glyphs the number token `token` can read: the glyph it names and those its components reach, in any
/// layer. `None` when that takes too many steps.
fn reduced(font: &Font, token: &str) -> Option<Font> {
    let mut keep: HashSet<String> = HashSet::new();
    let mut todo: Vec<&str> = tokens::token_glyph(token).into_iter().collect();
    let mut steps = 0usize;
    while let Some(name) = todo.pop() {
        let Some(glyph) = font.glyph(name) else { continue };
        if !keep.insert(name.to_string()) {
            continue;
        }
        for shape in glyph.layers.iter().flat_map(|l| &l.shapes) {
            if let Shape::Component(c) = shape {
                steps = steps.saturating_add(1);
                if steps > MAX_CLOSURE_STEPS {
                    return None;
                }
                todo.push(c.glyph.as_str());
            }
        }
    }
    let mut out = font.clone();
    out.glyphs.retain(|g| keep.contains(&g.name));
    Some(out)
}

/// What `token` is for `target`, or why it is nothing there.
fn value_at(font: &Font, data: &GlyphData, target: &CheckTarget, token: &str) -> std::result::Result<TokenValue, String> {
    // A parameter of the instance changes no token's value, so the entries are read as stored.
    at_target(font, &font.features, data, target, true, |made| tokens::evaluate(token, made.values)).read?
}

pub(super) fn evaluate_token(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.evaluateToken", raw)?;
    let token = p.str("token")?.trim();
    let data = s.glyph_data()?;
    let font = s.font()?;
    let asked = targets_param(&p, font)?;
    if asked.as_ref().is_some_and(|(list, all)| !all && list.contains(&CheckTarget::Variable)) {
        return Err(p.bad("`targets`: a token has no single value in the variable font: font.features.expandCode shows what its code holds"));
    }
    let predicate = token.starts_with("$[");
    // Predicates read every glyph.
    let small = (!predicate).then(|| reduced(font, token)).flatten().map_or(Cow::Borrowed(font), Cow::Owned);
    let (value, place) = match p.get("instance") {
        Some(_) if p.get("master").is_some() => return Err(p.bad("give `master` or `instance`, not both")),
        Some(v) => {
            let target = instance_of(&p, font, "instance", v)?;
            (value_at(&small, &data, &target, token), target_json(&target))
        }
        None => {
            let master = master_id(&p, font, p.opt_str("master")?)?;
            let values = MasterValues::new(font, &data, &master).map_err(|e| feature_error(&p, e))?;
            (tokens::evaluate(token, &values), json!({ "master": master }))
        }
    };
    let mut reply = match place {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    reply.insert("token".into(), json!(token));
    match value.map_err(|message| p.bad(message))? {
        TokenValue::Number(value) => {
            reply.insert("kind".into(), json!("number"));
            reply.insert("value".into(), json!(value));
        }
        TokenValue::Glyphs(glyphs) => {
            reply.insert("kind".into(), json!("glyphs"));
            reply.insert("glyphs".into(), json!(glyphs));
        }
    }
    if let Some((list, _)) = asked.filter(|_| !predicate) {
        let list: Vec<CheckTarget> = list.into_iter().filter(|t| *t != CheckTarget::Variable).collect();
        reply.insert("skippedTargets".into(), json!(list.len().saturating_sub(MAX_TARGETS)));
        let rows = list.iter().take(MAX_TARGETS).map(|target| {
            let mut row = json!({ "target": target_json(target), "name": target.label(font) });
            let (key, value) = match value_at(&small, &data, target, token) {
                Ok(TokenValue::Number(value)) => ("value", json!(value)),
                Ok(TokenValue::Glyphs(glyphs)) => ("glyphs", json!(glyphs)),
                Err(message) => ("problem", json!(message)),
            };
            if let Some(row) = row.as_object_mut() {
                row.insert(key.into(), value);
            }
            row
        });
        reply.insert("targets".into(), rows.collect());
    }
    Ok(Value::Object(reply))
}
