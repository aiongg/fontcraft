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
//! Limits: a draft holds at most [`MAX_DRAFT_BYTES`], the most one entry may hold; the compiler caps the rules it
//! generates and the diagnostics it reports (`fontcraft_fea` limits), so the work grows at most with the font's code
//! plus one capped entry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_fea::{CompileRequest, GlyphOrder};
use fontcraft_features::{EntryDiagnostic, FeatureError, MAX_ENTRY_BYTES, MasterValues};
use fontcraft_model::Font;
use fontcraft_model::glyphdata::GlyphData;
use serde_json::{Value, json};

use super::features_compile::{
    Prepared, counts, diagnostics_json, entry_param, feature_error, glyph_order, group_mut, master_id, prepare, request_diagnostic,
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
