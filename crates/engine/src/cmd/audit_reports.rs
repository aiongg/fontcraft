//! Audit reports and export: `audit.export`, `audit.report.menuNames`, `.metrics`, `.panose`, `.featureLanguages`, `audit.validatorKnownIssues` (M12.60).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! `audit.export` writes a result document (`docs/spec/21-qa.md` § 4) as JSON, or renders it as
//! text, Markdown or HTML with `fontcraft_audit::export`. The four reports read compiled fonts
//! by path and return the tables of the comparison tool's `-rn`, `-rm`, `-rp` and `-rf` reports
//! as data. `audit.validatorKnownIssues` is a behaviour with no command of its own: the importer
//! of an external validator's messages filters them with
//! `fontcraft_audit::export::known_validator_issue`.

use std::path::{Path, PathBuf};

use fontcraft_audit::Severity;
use fontcraft_audit::export::{self, CheckRow, ExportFormat, FaceInfo, ReportDoc, ReportFont, ReportSummary, ResultRow};
use fontcraft_tables::limits::MAX_FAMILY_FONTS;
use serde_json::{Map, json};

use super::file_io::{self, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most directory entries looked at when a report reads a folder.
const MAX_DIRECTORY_ENTRIES: usize = 100_000;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "audit.export",
            "Export Audit Report",
            [],
            None,
            "{path: path, format?: \"json\" | \"text\" | \"markdown\" | \"html\" (default from the extension of path: .json, .txt, .md, .html; else json), target?: \"document\" | path (default \"document\"), results?: a result document (written instead of the target's last results), minSeverity?: \"info\" | \"warn\" | \"fail\" = \"info\", noClobber?: bool} writes the last results of the target (those audit.results returns) to a file: the JSON result document, a plain-text log with one line per result (SEVERITY check-id [font] place: message) and a summary line, a Markdown report grouped by check with each check's description, or a self-contained HTML page with the same sections. Results below minSeverity are left out and the summary counts follow. The file is written in one step; nothing is written when the call fails → {path, format, results: count written, bytes}",
            always,
            export_report
        ),
        cmd!(
            query "audit.report.menuNames",
            "Menu Name Report",
            [],
            None,
            "{fonts?: [path], directory?: path} the menu names of each font, per typographic family: preferredMenu (typographic family/subfamily), macCompatibleMenu (the compatible full name), windowsCompatibleMenu (family/subfamily), and the family, subfamily, full, typographic family and subfamily, compatible full and PostScript names. fonts and directory: see audit.report.metrics → {families: [{family, fonts: [{path, postScriptName, preferredMenu, macCompatibleMenu, windowsCompatibleMenu, familyName, subfamilyName, fullName, typographicFamily, typographicSubfamily, compatibleFullName}]}], skipped?: [{path, reason}]}",
            always,
            menu_names
        ),
        cmd!(
            query "audit.report.metrics",
            "Font Metrics Report",
            [],
            None,
            "{fonts?: [path], directory?: path} the global values of each typographic family: glyph count, typo ascender, descender, line gap, cap height and x-height, italic angle, the bold and italic bits of head.macStyle, font revision, embedding permissions, bounding box, weight and width class, underline position and thickness, vendor ID, and the copyright and trademark of the first face. A value every face shares is given once (same: true, value, label); otherwise each face is listed, sorted as the field says (glyph count rising; italic angle rising; bold or italic faces first; revision, weight, width and underline values falling; the rest in family order). The bounding boxes are always listed. A field whose table the first face lacks has skipped. fonts and directory: the fonts to read, as the four reports take them: compiled font files (TrueType, CFF, WOFF, WOFF2; every face of a collection) and the font files directly in a folder, in file-name order; other files in the folder are passed over and fonts that cannot be read are listed in skipped. Faces are ordered by Windows family name, then italic and bold, and grouped by typographic family → {families: [{family, values: [{field, source, same, value?, label?, fonts?: [{postScriptName, value, label?}], skipped?}], firstFace: {postScriptName, copyright, trademark}}], skipped?: [{path, reason}]}",
            always,
            metrics
        ),
        cmd!(
            query "audit.report.panose",
            "PANOSE Report",
            [],
            None,
            "{fonts?: [path], directory?: path} the ten PANOSE digits of each font with the names their family kind gives them, per typographic family. The base face (weight class 400, width class 5, not italic, preferably subfamily Regular; else the first face) comes first, and each digit of the other faces says whether it differs from the base. fonts and directory: see audit.report.metrics → {families: [{family, base, fonts: [{postScriptName, digits, kind, fields: [{name, value, label, differs}]}], skipped?}], skipped?: [{path, reason}]}",
            always,
            panose
        ),
        cmd!(
            query "audit.report.featureLanguages",
            "Feature Coverage Report",
            [],
            None,
            "{fonts?: [path], directory?: path} for GPOS and GSUB of each font: the scripts and their language systems (dflt for the default one), the sets of lookups features map to (numbered from 1 in the order the script list first reaches them), and for each feature the lookup set in each script and language column (null where absent) with same when every column agrees. Fonts whose tables agree are grouped; groups are sorted by their first font's PostScript name. fonts and directory: see audit.report.metrics → {same, groups: [{fonts: [postScriptName], tables: [{table, scripts: [{script, languages}], columns: [{script, language}], lookupSets: [{id, lookups}], features: [{feature, same, sets}], truncated?, error?}]}], skipped?: [{path, reason}]}",
            always,
            feature_languages
        ),
    ]
}

// ── audit.export ────────────────────────────────────────────────────────────────────────────────

fn export_report(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.export", v)?;
    let path = PathBuf::from(p.str("path")?);
    let format = match p.opt_str("format")? {
        Some(f) => {
            ExportFormat::parse(f).ok_or_else(|| p.bad(format!("`format` must be \"json\", \"text\", \"markdown\" or \"html\", got \"{f}\"")))?
        }
        None => path.extension().and_then(|e| e.to_str()).and_then(ExportFormat::from_extension).unwrap_or(ExportFormat::Json),
    };
    let min = match p.opt_str("minSeverity")? {
        None => Severity::Info,
        Some(x) => Severity::parse(x).ok_or_else(|| p.bad(format!("`minSeverity` must be \"info\", \"warn\" or \"fail\", got \"{x}\"")))?,
    };
    let req = OutputRequest { no_clobber: p.bool_or("noClobber", false)?, ..OutputRequest::default() };
    let mut doc = match (p.get("results"), p.get("target")) {
        (Some(_), Some(_)) => return Err(p.bad("give `results` or `target`, not both")),
        (Some(d @ Value::Object(_)), None) => d.clone(),
        (Some(_), None) => return Err(p.bad("`results` must be a result document (an object)")),
        (None, target) => {
            let mut q = Map::new();
            if let Some(t) = target {
                q.insert("target".into(), t.clone());
            }
            s.execute("audit.results", &Value::Object(q))?
        }
    };
    let written = keep_at_least(&p, &mut doc, min)?;
    let bytes = match format {
        ExportFormat::Json => {
            let mut text = serde_json::to_string_pretty(&doc).map_err(|e| EngineError::Internal(format!("cannot write the result document: {e}")))?;
            text.push('\n');
            text.into_bytes()
        }
        other => export::render(&report_doc(&doc), other).into_bytes(),
    };
    file_io::write_output(&path, &bytes, &req)?;
    Ok(json!({"path": path.to_string_lossy(), "format": format.as_str(), "results": written, "bytes": bytes.len()}))
}

/// Drops the results below `min` and recounts the summary; returns how many results are left.
fn keep_at_least(p: &Params, doc: &mut Value, min: Severity) -> Result<usize> {
    let Some(obj) = doc.as_object_mut() else { return Err(p.bad("the result document is not an object")) };
    let results = match obj.get_mut("results") {
        None | Some(Value::Null) => return Ok(0),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(p.bad("the result document's `results` is not a list")),
    };
    if results.len() > export::MAX_EXPORT_RESULTS {
        return Err(p.bad(format!("the result document holds {} results; at most {} can be exported", results.len(), export::MAX_EXPORT_RESULTS)));
    }
    results.retain(|r| severity_of(r) <= min);
    let mut counts = [0usize; 3];
    for r in results.iter() {
        if let Some(n) = counts.get_mut(severity_of(r) as usize) {
            *n += 1;
        }
    }
    let written = results.len();
    if let Some(Value::Object(summary)) = obj.get_mut("summary") {
        let [fail, warn, info] = counts;
        summary.insert("fail".into(), json!(fail));
        summary.insert("warn".into(), json!(warn));
        summary.insert("info".into(), json!(info));
    }
    Ok(written)
}

fn severity_of(r: &Value) -> Severity {
    r.get("severity").and_then(Value::as_str).and_then(Severity::parse).unwrap_or(Severity::Info)
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

fn count(v: Option<&Value>, key: &str) -> usize {
    v.and_then(|v| v.get(key)).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The parts of a result document the text reports show. Missing or mistyped keys read as empty:
/// a document passed in by the caller is untrusted.
fn report_doc(v: &Value) -> ReportDoc {
    let summary = v.get("summary");
    let fonts = v.pointer("/target/fonts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    ReportDoc {
        tool_version: text(v.pointer("/tool/version")).unwrap_or_default(),
        profile: text(v.get("profile")).unwrap_or_default(),
        target_kind: text(v.pointer("/target/kind")).unwrap_or_default(),
        target_path: text(v.pointer("/target/path")),
        fonts: fonts
            .iter()
            .map(|f| ReportFont { id: text(f.get("id")).unwrap_or_default(), name: text(f.get("name")).unwrap_or_default() })
            .collect(),
        summary: ReportSummary {
            fail: count(summary, "fail"),
            warn: count(summary, "warn"),
            info: count(summary, "info"),
            passed: count(summary, "passed"),
            skipped: count(summary, "skipped"),
            ignored: count(summary, "ignored"),
            errors: count(summary, "errors"),
            stale: count(summary, "stale"),
        },
        checks: list(v, "checks")
            .iter()
            .map(|c| CheckRow {
                id: text(c.get("id")).unwrap_or_default(),
                status: text(c.get("status")).unwrap_or_default(),
                results: count(Some(c), "results"),
                reason: text(c.get("reason")).or_else(|| text(c.get("message"))),
            })
            .collect(),
        results: list(v, "results")
            .iter()
            .map(|r| {
                let loc = r.get("location");
                let at = |key: &str| text(loc.and_then(|l| l.get(key)));
                ResultRow {
                    id: text(r.get("id")).unwrap_or_default(),
                    check: text(r.get("check")).unwrap_or_default(),
                    code: text(r.get("code")).unwrap_or_default(),
                    severity: severity_of(r),
                    message: text(r.get("message")).unwrap_or_default(),
                    font: at("font"),
                    glyph: at("glyph").unwrap_or_default(),
                    master: at("master"),
                    layer: at("layer").unwrap_or_default(),
                    table: at("table"),
                    field: at("field"),
                    fonts: loc
                        .and_then(|l| l.get("fonts"))
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                        .unwrap_or_default(),
                    stale: r.get("stale").and_then(Value::as_bool).unwrap_or(false),
                }
            })
            .collect(),
    }
}

// ── The four reports ────────────────────────────────────────────────────────────────────────────

fn menu_names(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.report.menuNames", v)?;
    let (faces, skipped) = load_faces(&p, false)?;
    with_skipped(json!({ "families": to_json(&export::menu_name_report(&faces))? }), skipped)
}

fn metrics(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.report.metrics", v)?;
    let (faces, skipped) = load_faces(&p, false)?;
    with_skipped(json!({ "families": to_json(&export::metrics_report(&faces))? }), skipped)
}

fn panose(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.report.panose", v)?;
    let (faces, skipped) = load_faces(&p, false)?;
    with_skipped(json!({ "families": to_json(&export::panose_report(&faces))? }), skipped)
}

fn feature_languages(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.report.featureLanguages", v)?;
    let (faces, skipped) = load_faces(&p, true)?;
    with_skipped(to_json(&export::feature_report(&faces))?, skipped)
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| EngineError::Internal(format!("cannot write the report: {e}")))
}

fn with_skipped(mut out: Value, skipped: Vec<Value>) -> Result<Value> {
    if !skipped.is_empty()
        && let Some(o) = out.as_object_mut()
    {
        o.insert("skipped".into(), Value::Array(skipped));
    }
    Ok(out)
}

/// The faces of the `fonts` and `directory` parameters, and the files of the folder that could
/// not be read.
fn load_faces(p: &Params, with_layout: bool) -> Result<(Vec<FaceInfo>, Vec<Value>)> {
    let fonts = p.opt_strings("fonts")?;
    let directory = p.opt_str("directory")?;
    if fonts.is_none() && directory.is_none() {
        return Err(p.bad("give `fonts` (a list of font paths) or `directory`"));
    }
    let mut faces = Vec::new();
    let mut skipped = Vec::new();
    if let Some(list) = fonts {
        if list.len() > MAX_FAMILY_FONTS {
            return Err(p.bad(format!("`fonts` lists {} files; at most {MAX_FAMILY_FONTS} are allowed", list.len())));
        }
        for path in list {
            add_file(p, Path::new(path), true, with_layout, &mut faces, &mut skipped)?;
        }
    }
    if let Some(dir) = directory {
        for path in list_directory(Path::new(dir))? {
            add_file(p, &path, false, with_layout, &mut faces, &mut skipped)?;
        }
    }
    if faces.is_empty() {
        return Err(p.bad("no compiled font found in the given files"));
    }
    Ok((faces, skipped))
}

/// The regular files directly in `dir`, in file-name order.
fn list_directory(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|e| EngineError::Io(format!("cannot list {}: {e}", dir.display())))?;
    let mut files = Vec::new();
    for (i, entry) in entries.enumerate() {
        if i >= MAX_DIRECTORY_ENTRIES {
            return Err(EngineError::Io(format!("{} holds more than {MAX_DIRECTORY_ENTRIES} entries", dir.display())));
        }
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            files.push(path);
        }
    }
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(files)
}

/// Reads every face of one file. A file named in `fonts` (`explicit`) must be a readable font;
/// a file found in a folder is passed over when it is not a font, and listed in `skipped` when it
/// is one that cannot be read.
fn add_file(p: &Params, path: &Path, explicit: bool, with_layout: bool, faces: &mut Vec<FaceInfo>, skipped: &mut Vec<Value>) -> Result<()> {
    let shown = path.display().to_string();
    let mut fail = |reason: String| -> Result<()> {
        if explicit {
            return Err(EngineError::Io(format!("{shown}: {reason}")));
        }
        skipped.push(json!({"path": shown, "reason": reason}));
        Ok(())
    };
    let input = match file_io::read_input(path) {
        Ok(i) => i,
        Err(e) if explicit => return Err(e),
        Err(e) => return fail(e.to_string()),
    };
    if !input.kind.is_font() {
        if explicit {
            return Err(EngineError::Io(format!("{shown} is not a compiled font")));
        }
        return Ok(());
    }
    let sfnt = match fontcraft_container::to_sfnt(&input.bytes) {
        Ok(b) => b,
        Err(e) => return fail(e.to_string()),
    };
    let count = match export::face_count(&sfnt) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    for face in 0..count {
        if faces.len() >= MAX_FAMILY_FONTS {
            return Err(p.bad(format!("more than {MAX_FAMILY_FONTS} fonts to report on")));
        }
        let label = if count > 1 { format!("{shown}#{face}") } else { shown.clone() };
        match FaceInfo::read(&label, &sfnt, face, with_layout) {
            Ok(f) => faces.push(f),
            Err(e) if explicit => return Err(EngineError::Io(format!("{label}: {e}"))),
            Err(e) => skipped.push(json!({"path": label, "reason": e})),
        }
    }
    Ok(())
}
