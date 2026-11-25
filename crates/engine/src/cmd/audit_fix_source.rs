//! `audit.fix` on a source file (M12.84): the fixes of the open document, made on the source in a
//! session of its own and written through the save path ([`crate::io::save`], each file
//! atomically). The open documents do not change, and nothing is written unless every step of
//! the call went through and changed the source.
// The browser reads and writes no files by path.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::path::Path;

use serde_json::{Map, Value, json};

use super::file_io::{self, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// The parameters of the call that choose and make the fixes, passed to the source's session.
const FIX_KEYS: [&str; 12] =
    ["profile", "checks", "exclude", "groups", "glyphs", "masters", "layers", "minSeverity", "config", "safeOnly", "keepOriginal", "dryRun"];
/// The parameters of the call the check runs on the file take.
const RUN_KEYS: [&str; 9] = ["profile", "checks", "exclude", "groups", "glyphs", "masters", "layers", "minSeverity", "config"];
const SUFFIX: &str = ".fixed";

fn passed(p: &Params, keys: &[&str]) -> Map<String, Value> {
    keys.iter().filter_map(|k| Some(((*k).to_string(), p.get(k)?.clone()))).collect()
}

/// `audit.fix` on the source at `target`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn fix_source(s: &mut Session, p: &Params, target: &str) -> Result<Value> {
    use crate::io::SaveFormat;
    if p.get("results").is_some() {
        return Err(p.bad("`results` applies to the open document; on a file, select results with `checks` and the other filters"));
    }
    let dry = p.bool_or("dryRun", false)?;
    let req = OutputRequest::from_params(p)?;
    let input = Path::new(target);
    let (font, mut problems) = crate::io::open(input, None)?;
    if font.source.format == fontcraft_model::SourceFormat::Binary {
        return Err(EngineError::Unsupported("fixing this kind of compiled font; open it and save it as a source first".into()));
    }
    let output = file_io::output_path(input, &req, SUFFIX, None)?;
    if !req.in_place && file_io::resolved(&output) == file_io::resolved(input) {
        return Err(EngineError::Io(format!("the output {} is the input: pass inPlace to replace it", output.display())));
    }
    let format = if req.in_place { SaveFormat::of_source(font.source.format) } else { SaveFormat::of_path(&output) }.ok_or_else(|| {
        p.bad(format!(
            "{} is not in a format a source is saved in: give an `output` ending in .fontcraft, .glyphs, .glyphspackage, .ufo or .ufoz",
            output.display()
        ))
    })?;
    if format == SaveFormat::Designspace && !req.in_place {
        return Err(p.bad("a designspace names its UFOs, so a copy of it would write over them: give `inPlace`, or an `output` in another format"));
    }
    let read = crate::io::document_stamp(input, &font, font.source.format);

    let mut own = s.without_documents();
    own.push_document(font, Some(input.to_path_buf()));
    let data = crate::glyph_data::source_files(input, &mut problems);
    if !data.is_empty() {
        let files: Vec<(&str, &[u8])> = data.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        problems.extend(own.set_source_glyph_data(&files)?);
    }
    let mut result = match own.execute("audit.fix", &Value::Object(passed(p, &FIX_KEYS)))? {
        Value::Object(o) => o,
        other => return Err(EngineError::Internal(format!("audit.fix answered {other}"))),
    };
    let doc = own.doc()?;
    let changed = !dry && doc.modified() && !doc.font.same_as(&doc.saved);
    if changed {
        crate::work::check()?;
        file_io::check_write(&output)?;
        if req.no_clobber && !req.in_place && output.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
        }
        if req.in_place && crate::io::document_stamp(input, &doc.saved, doc.saved.source.format) != read {
            return Err(EngineError::Io(format!("{target} changed on disk while it was fixed, so nothing was written")));
        }
        let mut font = doc.font.as_ref().clone();
        if !req.in_place {
            crate::assets::rebase_image_paths(&mut font, Some(input), &output);
        }
        let created = file_io::create_output_dir(&req)?;
        let saved = match crate::io::save(&font, &output, format) {
            Ok(saved) => saved,
            Err(e) => {
                file_io::remove_created(&created);
                return Err(e);
            }
        };
        if !saved.losses.is_empty() {
            result.insert("losses".into(), Value::Array(saved.losses));
        }
    }

    // What the file holds now: a check run that fails after the write is reported, not an error.
    let mut run = passed(p, &RUN_KEYS);
    run.insert("target".into(), if changed { json!(output) } else { json!(target) });
    if let Some(max) = p.get("maxResults") {
        run.insert("maxResults".into(), max.clone());
    }
    let remaining = match super::audit::check_files(s, &Value::Object(run)) {
        Ok(report) => report,
        Err(e) if changed => {
            problems.push(format!("the fixed file was written, but checking it again failed: {e}"));
            Value::Null
        }
        Err(e) => return Err(e),
    };
    result.insert("remaining".into(), remaining);
    result.insert("output".into(), if changed || dry { json!(output) } else { Value::Null });
    if !problems.is_empty() {
        result.insert("warnings".into(), json!(problems));
    }
    Ok(Value::Object(result))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn fix_source(_: &mut Session, _: &Params, _: &str) -> Result<Value> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}
