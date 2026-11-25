//! Checking a draft of one entry's feature code while it is typed (M8.25): `font.features.check`.
//!
//! The check assembles the font's code for one master with the stored code of one entry replaced by the draft and runs
//! the feature compiler over it, the code on its own: no generated kerning or marks, no preview font. It returns the
//! diagnostics as `font.features.listErrors` does, places in the draft, and changes nothing: not the font, not the
//! document's feature state.
//!
//! [`check_font`] runs on a font and a [`CheckContext`] without the session, so the Features tab runs it on a worker
//! thread with the font of the revision the draft was typed against.
//!
//! [`compile_instance`] answers `font.features.compile` with `instance` (M8.31) the same way: the font's code with the
//! instance's Add, Replace and Remove parameters applied, as export applies them, compiled on its own and stored
//! nowhere, each diagnostic in code a parameter added or replaced naming that parameter.
//!
//! Limits: a draft holds at most [`MAX_DRAFT_BYTES`], the most one entry may hold; the compiler caps the rules it
//! generates and the diagnostics it reports (`fontcraft_fea` limits), so the work grows at most with the font's code
//! plus one capped entry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_fea::{CompileRequest, GlyphOrder};
use fontcraft_features::EntryKind;
use fontcraft_features::params::{ADD_CLASS, ADD_FEATURE, ADD_PREFIX, REPLACE_CLASS, REPLACE_FEATURE, REPLACE_PREFIX, apply_instance_params};
use fontcraft_features::{EntryDiagnostic, FeatureError, MAX_ENTRY_BYTES, MasterValues};
use fontcraft_fontinfo::target;
use fontcraft_model::Font;
use fontcraft_model::glyphdata::GlyphData;
use serde_json::{Value, json};

use super::features_compile::{
    Prepared, counts, diagnostics_json, entry_param, feature_error, glyph_order, group, group_mut, master_id, prepare, request_diagnostic,
};
use super::*;
use crate::feature_state::FeatureState;
use crate::params::Params;
use crate::{Result, Session};

/// Most bytes of draft code a check reads: the most one entry may hold.
pub const MAX_DRAFT_BYTES: usize = MAX_ENTRY_BYTES;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.features.check",
        "Check Feature Code",
        [],
        None,
        "{kind: \"prefix\"|\"class\"|\"feature\", name: string, index?: position in the group, code: string (the draft that stands in for the entry's stored code; at most 1 MB), master?: id|name (default: the first master; the Features tab passes the master of the active Edit View tab)} compiles the font's feature code with the entry's code replaced by the draft, without generated kerning or marks, and changes nothing → {master, ok: bool (no errors), errors: n, warnings: n, diagnostics: [{id, severity, message, entry: {kind, name, index}|null, range: [start, end]|null (bytes of the draft for the checked entry, of the stored code for the others), line, column, fixes: [] }]}",
        has_doc,
        check
    )]
}

/// What a check reads besides the font: the glyph data and the folder included files are read from.
#[derive(Clone)]
pub struct CheckContext {
    pub glyph_data: Arc<GlyphData>,
    pub include_dir: Option<PathBuf>,
}

/// The context of the active document, as `font.features.compile` reads it.
pub fn check_context(s: &Session) -> Result<CheckContext> {
    let doc = s.doc()?;
    Ok(CheckContext { glyph_data: s.glyph_data_of(doc)?, include_dir: doc.path.as_deref().and_then(super::shared::source_feature_dir) })
}

fn check(s: &mut Session, raw: &Value) -> Result<Value> {
    let cx = check_context(s)?;
    check_font(s.font()?, &cx, raw)
}

/// `font.features.check` on `font`: the same parameters and answer as the command.
pub fn check_font(font: &Font, cx: &CheckContext, raw: &Value) -> Result<Value> {
    let p = Params::new("font.features.check", raw)?;
    let code = p.str("code")?;
    if code.len() > MAX_DRAFT_BYTES {
        return Err(p.bad(format!("`code` holds more than the {} MB one entry may hold", MAX_DRAFT_BYTES >> 20)));
    }
    let entry = entry_param(&p, &font.features, true)?.ok_or_else(|| p.bad("`kind` is required"))?;
    let master = master_id(&p, font, p.opt_str("master")?)?;
    let values = MasterValues::new(font, &cx.glyph_data, &master).map_err(|e| feature_error(&p, e))?;
    let mut features = font.features.as_ref().clone();
    let Some(snippet) = group_mut(&mut features, entry.kind).get_mut(entry.index) else {
        return Err(p.bad("the entry is gone"));
    };
    snippet.code = code.to_string();
    let diagnostics: Vec<EntryDiagnostic> = match prepare(font, &features, &master, cx.include_dir.as_deref(), &values) {
        Err(FeatureError::Code(found)) => found,
        Err(e @ (FeatureError::NotYetImplemented(_) | FeatureError::Unsupported(_))) => return Err(feature_error(&p, e)),
        Err(other) => vec![request_diagnostic(other.to_string())],
        Ok(Prepared { map, files, .. }) => match GlyphOrder::new(glyph_order(font)) {
            Err(d) => vec![EntryDiagnostic { place: None, diagnostic: d }],
            Ok(order) => match fontcraft_fea::compile(&CompileRequest::new(&files, &order)) {
                Ok(compiled) => compiled.warnings.into_iter().map(|d| map.map(d)).collect(),
                Err(found) => found.into_iter().map(|d| map.map(d)).collect(),
            },
        },
    };
    let state = FeatureState { revision: 0, master: master.clone(), preview: None, features: Vec::new(), diagnostics, source_hash: 0, code_hash: 0 };
    let (errors, warnings) = counts(&state);
    Ok(json!({
        "master": master,
        "ok": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "diagnostics": diagnostics_json(&features, &state, None, &BTreeMap::new()),
    }))
}

/// The parameters that put code of their own in the instance's entries, and the group each acts on.
const CODE_PARAMS: [(&str, EntryKind); 6] = [
    (ADD_CLASS, EntryKind::Class),
    (ADD_PREFIX, EntryKind::Prefix),
    (ADD_FEATURE, EntryKind::Feature),
    (REPLACE_CLASS, EntryKind::Class),
    (REPLACE_PREFIX, EntryKind::Prefix),
    (REPLACE_FEATURE, EntryKind::Feature),
];

/// The compiled diagnostics of `features` (the font's entries as one export would hold them) for `master`.
pub(super) fn compile_entries(
    font: &Font,
    features: &fontcraft_model::Features,
    master: &str,
    cx: &CheckContext,
    values: &MasterValues,
    p: &Params,
) -> Result<Vec<EntryDiagnostic>> {
    Ok(match prepare(font, features, master, cx.include_dir.as_deref(), values) {
        Err(FeatureError::Code(found)) => found,
        Err(e @ (FeatureError::NotYetImplemented(_) | FeatureError::Unsupported(_))) => return Err(feature_error(p, e)),
        Err(other) => vec![request_diagnostic(other.to_string())],
        Ok(Prepared { map, files, .. }) => match GlyphOrder::new(glyph_order(font)) {
            Err(d) => vec![EntryDiagnostic { place: None, diagnostic: d }],
            Ok(order) => match fontcraft_fea::compile(&CompileRequest::new(&files, &order)) {
                Ok(compiled) => compiled.warnings.into_iter().map(|d| map.map(d)).collect(),
                Err(found) => found.into_iter().map(|d| map.map(d)).collect(),
            },
        },
    })
}

/// `font.features.compile` with `instance`: the font's feature code with the instance's parameters applied one by one,
/// as export applies them, compiled for `master` without generated kerning or marks, stored nowhere. A parameter whose
/// value cannot be applied is left out and reported; a diagnostic in an entry an Add or Replace parameter wrote gets
/// `param: {name, index}` (the index counts the instance's parameters of that name), its line counted in the
/// parameter's code.
/// Runs on `font` and a [`CheckContext`] without the session, as [`check_font`] does, so the Exports tab can call it.
pub fn compile_instance(font: &Font, cx: &CheckContext, raw: &Value) -> Result<Value> {
    let p = &Params::new("font.features.compile", raw)?;
    let raw = p.get("instance").ok_or_else(|| p.bad("`instance` is required"))?;
    let at = target::parse(raw).and_then(|r| target::instance(font, &r)).map_err(|e| p.bad(format!("`instance`: {e}")))?;
    let instance = font.instances.get(at).ok_or_else(|| p.bad("the instance is gone"))?;
    let master = master_id(p, font, p.opt_str("master")?)?;
    let values = MasterValues::new(font, &cx.glyph_data, &master).map_err(|e| feature_error(p, e))?;
    let mut features = font.features.as_ref().clone();
    let mut refused = Vec::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    // The Add and Replace parameters applied: (parameter name, index among the parameters of that name, entry name,
    // code).
    let mut written: Vec<(&str, usize, &str, &str)> = Vec::new();
    for param in &instance.params.0 {
        let occurrence = seen.entry(param.name.as_str()).or_insert(0);
        let index = *occurrence;
        *occurrence += 1;
        if param.disabled {
            continue;
        }
        match apply_instance_params(&features, &fontcraft_model::Params(vec![param.clone()])) {
            Ok(next) => features = next,
            Err(e) => {
                refused.push(json!({ "param": { "name": param.name, "index": index }, "message": e.to_string() }));
                continue;
            }
        }
        if CODE_PARAMS.iter().any(|(name, _)| *name == param.name)
            && let Some((name, code)) = param.value.as_str().and_then(|t| t.split_once(';'))
        {
            written.push((param.name.as_str(), index, name.trim(), code.trim_start()));
        }
    }
    let mut origins: BTreeMap<(&'static str, usize), (String, usize)> = BTreeMap::new();
    for (param, index, name, code) in &written {
        let Some((_, kind)) = CODE_PARAMS.iter().find(|(n, _)| n == param) else { continue };
        let bare = |n: &str| n.strip_prefix('@').unwrap_or(n).to_string();
        let found = group(&features, *kind).iter().rposition(|e| bare(&e.name) == bare(name) && e.code == *code);
        if let Some(at) = found {
            origins.insert((kind_label(*kind), at), ((*param).to_string(), *index));
        }
    }
    let diagnostics = compile_entries(font, &features, &master, cx, &values, p)?;
    let state = FeatureState { revision: 0, master: master.clone(), preview: None, features: Vec::new(), diagnostics, source_hash: 0, code_hash: 0 };
    let (errors, warnings) = counts(&state);
    let mut list = diagnostics_json(&features, &state, None, &BTreeMap::new());
    for d in &mut list {
        let entry = d.get("entry").and_then(|e| Some((e.get("kind")?.as_str()?.to_string(), usize::try_from(e.get("index")?.as_u64()?).ok()?)));
        let origin = entry.and_then(|(kind, at)| origins.iter().find(|((k, i), _)| *k == kind && *i == at).map(|(_, o)| o.clone()));
        if let (Some((name, index)), Some(object)) = (origin, d.as_object_mut()) {
            object.insert("param".into(), json!({ "name": name, "index": index }));
        }
    }
    let refused_count = refused.len();
    Ok(json!({
        "master": master,
        "instance": instance.name,
        "ok": errors == 0 && refused_count == 0,
        "errors": errors + refused_count,
        "warnings": warnings,
        "diagnostics": list,
        "refused": refused,
    }))
}

/// The name `diagnostics_json` gives a group.
fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Prefix => "prefix",
        EntryKind::Class => "class",
        EntryKind::Feature => "feature",
    }
}
