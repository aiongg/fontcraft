//! Audit fixes on compiled fonts: the `tables.*` fix options `patchTable`, `dropTables`, `cleanNameRecords`, `cleanInstances`, `rebuildCmap`, `recompileCff`, `recompileLayout` (M12.61; `fontcraft_tables::patch`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! `audit.fix` with a file target ([`fix_file`]) runs the checks on the file, builds one call per
//! fix in the order of the file-target table of `docs/spec/21-qa.md` § 5 › Fix calls, and runs
//! the calls one after the other on a temporary copy next to the output, each reading what the
//! previous one wrote. The copy replaces the output (or the input with `inPlace`) only when every
//! call succeeded; otherwise it is removed and nothing is written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_tables::TableOrder;
use fontcraft_tables::patch::{self, NameOptions, PatchValue};
use fontcraft_tables::sfnt::parse_tag;
use serde_json::{Map, Value, json};

use super::audit_fix_recalc::{self, canonical};
use super::file_io::{self, OutputRequest};
use super::tables_file::{load, write_font};
use super::*;
use crate::EngineError;
use crate::params::Params;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "tables.recalculate",
            "Recalculate Table Fields",
            [],
            None,
            "{font: path, face?: number|name, fields?: [\"table.field\"], values?: {\"table.field\": number|text}, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} recomputes fields derived from the font's other data and writes the font (default output: FONT.fixed.EXT). Without fields: glyf.bounds, head.bbox, hhea.advanceWidthMax, hhea.extents, OS/2.xAvgCharWidth, OS/2.charIndex, OS/2.ulUnicodeRange and OS/2.ulCodePageRange. By name also: sfntVersion, head.flags (force integer sizes when the font has instructions), hhea.caretSlope (from post.italicAngle), post.underlineThickness (5% of the em when 0), post.isFixedPitch and OS/2.panose (from the advances), OS/2.version (raised to 4), OS/2.fsSelection (USE_TYPO_METRICS, version 4), the ten OS/2 strikeout and script fields (their defaults), OS/2.usWeightClass (the wght default), gasp (one range for all sizes) and prep (smart dropout control); one field of a group (head.xMin) stands for the group. values writes fields as given, after the recomputed ones. Every other byte stays as it was → {output, changed: [{field, old, new}]}",
            always,
            tables_recalculate
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "tables.cleanNameRecords",
            "Clean Name Records",
            [],
            None,
            "{font: path, face?: number|name, dropMac?: bool = true, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} trims name records and turns each run of white space and line breaks into one space, removes empty and repeated records and (with dropMac) the Macintosh records, and gives every PostScript name record (name ID 6) the CFF font name, else the Windows English one; records in an encoding that cannot be written again keep their bytes (default output: FONT.fixed.EXT) → {output, removed: count, changed: count}",
            always,
            tables_clean_names
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "tables.rebuildCmap",
            "Rebuild cmap",
            [],
            None,
            "{font: path, face?: number|name, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} rebuilds the cmap from the union of its Unicode subtables (platform 3 wins a conflict): format 4 for (0, 3) and (3, 1), format 12 for (0, 4) and (3, 10) when a code point lies beyond the Basic Multilingual Plane, format 14 copied; other subtables are left out (default output: FONT.fixed.EXT) → {output, mappings: count, conflicts: [{codepoint, kept, dropped}]}",
            always,
            tables_rebuild_cmap
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "tables.recompileLayout",
            "Recompile Contextual Positioning",
            [],
            None,
            "{font: path, face?: number|name, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} rewrites each contextual positioning lookup (GPOS type 7, also inside an extension lookup) as a chained contextual lookup (type 8) with empty backtrack and lookahead; lookup indices, flags and every other table stay as they were (default output: FONT.fixed.EXT) → {output, converted: [lookup index]}",
            always,
            tables_recompile_layout
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "tables.cleanInstances",
            "Clean Named Instances",
            [],
            None,
            "{font: path, face?: number|name, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} removes repeated named instances, sorts them (by the other axes, then weight), removes the PostScript name field from every record when some records lack a name, and names the instance at the default location with name ID 17 (else 2) and name ID 6 (default output: FONT.fixed.EXT) → {output, removed: count, reordered: bool, droppedPostScriptNames: bool}",
            always,
            tables_clean_instances
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
        cmd!(
            query "tables.dropAxis",
            "Drop Axis",
            [],
            None,
            "{font: path, face?: number|name, axis: tag, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} removes an axis whose minimum equals its maximum by pinning it: its fvar record and coordinates, avar map, STAT records and its column in every variation region; an axis with a range is refused; the only axis leaves a static font (default output: FONT.fixed.EXT) → {output, axis}",
            always,
            tables_drop_axis
        )
        .files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
    ]
}

const SUFFIX: &str = ".fixed";

fn tables_err(e: fontcraft_tables::TablesError) -> EngineError {
    EngineError::Tables(e)
}

fn tables_recalculate(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.recalculate", v)?;
    let loaded = load(&p, "font", "face")?;
    let fields = match p.opt_strings("fields")? {
        None => None,
        Some(names) => {
            let mut out: Vec<&'static str> = Vec::new();
            for name in names {
                let field =
                    canonical(name).ok_or_else(|| p.bad(format!("`{name}` is not a field this command recomputes; give its value in `values`")))?;
                if !out.contains(&field) {
                    out.push(field);
                }
            }
            Some(out)
        }
    };
    let mut values = Vec::new();
    if let Some(map) = p.opt_object("values")? {
        for (name, value) in map {
            let value = match value {
                Value::Number(n) => PatchValue::Number(n.as_f64().ok_or_else(|| p.bad(format!("`values.{name}` is not a number")))?),
                Value::String(s) => PatchValue::Text(s.clone()),
                _ => return Err(p.bad(format!("`values.{name}` must be a number or text"))),
            };
            values.push((name.clone(), value));
        }
    }
    let (font, changed) = audit_fix_recalc::recalculate(&loaded.font, fields.as_deref(), &values)?;
    let output = write_font(&p, &loaded, &font, TableOrder::Keep, SUFFIX)?;
    Ok(json!({"output": output, "changed": changed}))
}

fn tables_clean_names(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.cleanNameRecords", v)?;
    let loaded = load(&p, "font", "face")?;
    let r = patch::clean_name_records(&loaded.font, &NameOptions { drop_mac: p.bool_or("dropMac", true)? }).map_err(tables_err)?;
    let output = write_font(&p, &loaded, &r.font, TableOrder::Keep, SUFFIX)?;
    Ok(json!({"output": output, "removed": r.removed, "changed": r.changed}))
}

fn tables_rebuild_cmap(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.rebuildCmap", v)?;
    let loaded = load(&p, "font", "face")?;
    let r = patch::rebuild_cmap(&loaded.font).map_err(tables_err)?;
    let output = write_font(&p, &loaded, &r.font, TableOrder::Keep, SUFFIX)?;
    let conflicts: Vec<Value> = r.conflicts.iter().map(|c| json!({"codepoint": c.codepoint, "kept": c.kept, "dropped": c.dropped})).collect();
    Ok(json!({"output": output, "mappings": r.mappings, "conflicts": conflicts}))
}

fn tables_recompile_layout(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.recompileLayout", v)?;
    let loaded = load(&p, "font", "face")?;
    let (font, converted) = patch::recompile_layout(&loaded.font).map_err(tables_err)?;
    let output = write_font(&p, &loaded, &font, TableOrder::Keep, SUFFIX)?;
    Ok(json!({"output": output, "converted": converted}))
}

fn tables_clean_instances(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.cleanInstances", v)?;
    let loaded = load(&p, "font", "face")?;
    let r = patch::clean_instances(&loaded.font).map_err(tables_err)?;
    let output = write_font(&p, &loaded, &r.font, TableOrder::Keep, SUFFIX)?;
    Ok(json!({"output": output, "removed": r.removed, "reordered": r.reordered, "droppedPostScriptNames": r.dropped_postscript_names}))
}

fn tables_drop_axis(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("tables.dropAxis", v)?;
    let axis = p.str("axis")?;
    let tag = parse_tag(axis).map_err(|e| p.bad(e.to_string()))?;
    let loaded = load(&p, "font", "face")?;
    let font = patch::drop_axis(&loaded.font, tag).map_err(tables_err)?;
    let output = write_font(&p, &loaded, &font, TableOrder::Keep, SUFFIX)?;
    Ok(json!({"output": output, "axis": axis}))
}

// ── audit.fix on a file ─────────────────────────────────────────────────────────────────────────

/// The table repairs, in the order they run, with the command each one calls.
const FILE_FIXES: [(&str, &str); 10] = [
    ("dropTables", "tables.delete"),
    ("recompileCff", "tables.recompileCff"),
    ("recompileLayout", "tables.recompileLayout"),
    ("rebuildCmap", "tables.rebuildCmap"),
    ("cleanNameRecords", "tables.cleanNameRecords"),
    ("cleanInstances", "tables.cleanInstances"),
    ("dropAxis", "tables.dropAxis"),
    ("recompile", "variations.buildHvar"),
    ("removeOverlap", "outlines.removeOverlaps"),
    ("patchTable", "tables.recalculate"),
];

/// The command that carries out the fix `id` on a compiled font file, when it is registered.
pub(super) fn file_fix_command(id: &str) -> Option<&'static str> {
    FILE_FIXES.iter().find(|(f, _)| *f == id).map(|(_, c)| *c).filter(|c| find_command(c).is_some())
}

/// The `audit.run` parameters `audit.fix` passes through on a file.
const RUN_KEYS: [&str; 7] = ["profile", "checks", "exclude", "groups", "glyphs", "minSeverity", "config"];
/// `audit.fix` parameters that only make sense on the open document.
const DOCUMENT_KEYS: [&str; 4] = ["results", "masters", "layers", "keepOriginal"];
/// Results read from one run, whatever `maxResults` the caller gave.
const ALL_RESULTS: i64 = 1_000_000;

/// `audit.fix` names a file rather than the open document.
pub(super) fn is_file_target(p: &Params) -> bool {
    match p.get("target") {
        Some(Value::String(s)) => s != "document",
        Some(Value::Array(_)) => true,
        _ => false,
    }
}

/// One call `audit.fix` makes on a file.
struct Call {
    fix: &'static str,
    command: &'static str,
    /// The call's own parameters, without the font and the output.
    params: Map<String, Value>,
    checks: Vec<String>,
}

fn text<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

/// The `fields` key `tables.recalculate` reads for a result's location, or why there is none.
fn recalc_field(location: &Value) -> std::result::Result<&'static str, String> {
    let table = text(location, "table").map(str::trim_end);
    let field = text(location, "field");
    let name = match (table, field) {
        (Some(t), Some(f)) => format!("{t}.{f}"),
        (Some(t), None) => t.to_string(),
        (None, Some(f)) => f.to_string(),
        (None, None) => return Err("the result names no table field".into()),
    };
    canonical(&name).ok_or_else(|| format!("recomputing {name} in a compiled font is not built yet"))
}

/// Report of a file fix.
#[derive(Default)]
struct FileReport {
    applied: Vec<Value>,
    skipped: Vec<Value>,
}

impl FileReport {
    fn skip(&mut self, check: &str, fix: Option<&str>, reason: impl Into<String>) {
        let mut m = Map::new();
        m.insert("check".into(), json!(check));
        if let Some(fix) = fix {
            m.insert("fix".into(), json!(fix));
        }
        m.insert("reason".into(), json!(reason.into()));
        self.skipped.push(Value::Object(m));
    }
}

/// The calls the results ask for, in the order they run.
/// `truetype` says whether the file is an uncompressed TrueType font, the only kind
/// `variations.buildHvar` reads.
fn plan(results: &[Value], safe_only: bool, truetype: bool, report: &mut FileReport) -> Vec<Call> {
    // (fix order, face) → call
    let mut calls: BTreeMap<(usize, u64), Call> = BTreeMap::new();
    for r in results {
        let check = text(r, "check").unwrap_or_default();
        let Some(fix) = r.get("fix").filter(|f| f.get("id").is_some()) else { continue };
        let id = text(fix, "id").unwrap_or_default();
        let Some(order) = FILE_FIXES.iter().position(|(f, _)| *f == id) else {
            report.skip(check, Some(id), "the fix edits glyph data or Font Info in a source; it does not apply to a compiled font");
            continue;
        };
        let Some(&(fix_id, command)) = FILE_FIXES.get(order) else { continue };
        if safe_only && fix.get("safe").and_then(Value::as_bool) != Some(true) {
            report.skip(check, Some(fix_id), "the fix changes the design: give safeOnly: false");
            continue;
        }
        if find_command(command).is_none() {
            report.skip(check, Some(fix_id), format!("{command}, which carries out this fix, is not built yet"));
            continue;
        }
        if fix_id == "recompile" && !truetype {
            report.skip(check, Some(fix_id), "HVAR is built from the gvar deltas of an uncompressed TrueType font; convert the file first");
            continue;
        }
        let location = r.get("location").cloned().unwrap_or(Value::Null);
        let face = text(&location, "font").and_then(|f| f.strip_prefix('f')).and_then(|i| i.parse::<u64>().ok()).unwrap_or(0);
        // Variations are rebuilt for the whole file.
        let face = if fix_id == "recompile" { 0 } else { face };
        let call = calls.entry((order, face)).or_insert_with(|| {
            let mut params = Map::new();
            if fix_id != "recompile" {
                params.insert("face".into(), json!(face));
            }
            Call { fix: fix_id, command, params, checks: Vec::new() }
        });
        let mut list = |key: &str, item: Value| {
            if let Value::Array(a) = call.params.entry(key).or_insert_with(|| json!([]))
                && !a.contains(&item)
            {
                a.push(item);
            }
        };
        let usable = match fix_id {
            "dropTables" => text(&location, "table").map(|t| list("tables", json!(t))).ok_or("the result names no table"),
            "removeOverlap" => {
                text(&location, "glyph").filter(|g| !g.is_empty()).map(|g| list("glyphs", json!(g))).ok_or("the result names no glyph")
            }
            "dropAxis" => text(&location, "axis").map(|a| list("axes", json!(a))).ok_or("the result names no axis"),
            "patchTable" => {
                let fs_type = text(&location, "table") == Some("OS/2") && text(&location, "field") == Some("fsType");
                if fs_type {
                    match r.get("data").and_then(|d| d.get("expected")).and_then(Value::as_f64) {
                        Some(n) => {
                            let values = call.params.entry("values").or_insert_with(|| json!({}));
                            if let Some(o) = values.as_object_mut() {
                                o.insert("OS/2.fsType".into(), json!(n));
                            }
                            Ok(())
                        }
                        None => Err("the result does not give the project's fsType"),
                    }
                } else {
                    match recalc_field(&location) {
                        Ok(field) => {
                            list("fields", json!(field));
                            Ok(())
                        }
                        Err(why) => {
                            report.skip(check, Some(fix_id), why);
                            continue;
                        }
                    }
                }
            }
            _ => Ok(()),
        };
        match usable {
            Ok(()) => {
                if !call.checks.iter().any(|c| c == check) {
                    call.checks.push(check.to_string());
                }
            }
            Err(why) => report.skip(check, Some(fix_id), why),
        }
    }
    // One dropAxis call per axis; a call that collected nothing to do is left out.
    let mut out = Vec::new();
    for (_, mut call) in calls {
        if call.checks.is_empty() {
            continue;
        }
        if call.fix == "patchTable" && !call.params.contains_key("fields") {
            call.params.insert("fields".into(), json!([]));
        }
        if call.fix == "dropAxis" {
            let axes = call.params.remove("axes").and_then(|a| a.as_array().cloned()).unwrap_or_default();
            for axis in axes {
                let mut params = call.params.clone();
                params.insert("axis".into(), axis);
                out.push(Call { fix: call.fix, command: call.command, params, checks: call.checks.clone() });
            }
            continue;
        }
        out.push(call);
    }
    out
}

/// The parameters of `call` on the font at `font`, replaced in place.
fn call_params(call: &Call, font: &Path, in_place: bool) -> Value {
    let mut params = call.params.clone();
    let key = if call.command == "outlines.removeOverlaps" { "input" } else { "font" };
    params.insert(key.into(), json!(font));
    if in_place {
        params.insert("inPlace".into(), json!(true));
    }
    Value::Object(params)
}

/// `audit.fix` on a compiled font file.
pub(super) fn fix_file(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.fix", v)?;
    let target = match p.get("target") {
        Some(Value::String(t)) => t.clone(),
        _ => return Err(p.bad("`target` names one compiled font file; fix the fonts of a family one file at a time")),
    };
    if let Some(key) = DOCUMENT_KEYS.iter().find(|k| p.get(k).is_some()) {
        return Err(p.bad(format!("`{key}` applies to the open document; on a file, select results with `checks` and the other filters")));
    }
    let safe_only = p.bool_or("safeOnly", true)?;
    let dry = p.bool_or("dryRun", false)?;
    let req = OutputRequest::from_params(&p)?;
    let input = PathBuf::from(&target);
    let output = file_io::output_path(&input, &req, SUFFIX, None)?;
    let source = file_io::read_input(&input)?;
    if !source.kind.is_font() {
        return Err(p.bad(format!("{target} is not a compiled font; open a source and fix the document")));
    }

    let mut run = Map::new();
    for key in RUN_KEYS {
        if let Some(value) = p.get(key) {
            run.insert(key.into(), value.clone());
        }
    }
    run.insert("target".into(), json!(target));
    run.insert("maxResults".into(), json!(ALL_RESULTS));
    let before = s.execute("audit.run", &Value::Object(run.clone()))?;
    let results = before.get("results").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut report = FileReport::default();
    let calls = plan(&results, safe_only, matches!(source.kind, file_io::InputKind::TrueType), &mut report);
    if dry {
        let planned: Vec<Value> =
            calls.iter().map(|c| json!({"fix": c.fix, "command": c.command, "params": call_params(c, &input, false), "checks": c.checks})).collect();
        return Ok(json!({"output": output, "applied": [], "skipped": report.skipped, "planned": planned, "remaining": before, "dryRun": true}));
    }
    if calls.is_empty() {
        return Ok(json!({"output": Value::Null, "applied": [], "skipped": report.skipped, "remaining": before, "dryRun": false}));
    }

    file_io::check_write(&output)?;
    // The calls work on a new temporary copy beside the file the output replaces (the file a
    // link points to, so the link stays), with that file's permissions; stage checks both
    // paths against the write roots before it creates the copy.
    let staged = super::instance::stage(&output, &source.bytes, 0)?;
    let discard = |staged: &super::instance::Staged| super::instance::discard(std::slice::from_ref(staged));
    let tmp = staged.temp().to_path_buf();
    for call in &calls {
        match s.execute(call.command, &call_params(call, &tmp, true)) {
            Ok(result) => report.applied.push(json!({
                "fix": call.fix,
                "command": call.command,
                "params": call_params(call, &output, false),
                "checks": call.checks,
                "result": result,
            })),
            Err(e) => {
                discard(&staged);
                return Err(EngineError::Io(format!("the fix {} ({}) failed, so nothing was written: {e}", call.fix, call.command)));
            }
        }
    }
    if req.no_clobber && !req.in_place && output.exists() {
        discard(&staged);
        return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
    }
    super::instance::commit(std::slice::from_ref(&staged))?;
    // The fixed file is in place, so a check run that fails now is reported, not an error.
    run.insert("target".into(), json!(output));
    let (remaining, warnings) = match s.execute("audit.run", &Value::Object(run)) {
        Ok(remaining) => (remaining, None),
        Err(e) => (Value::Null, Some(json!([format!("the fixed file was written, but checking it again failed: {e}")]))),
    };
    let mut result = json!({"output": output, "applied": report.applied, "skipped": report.skipped, "remaining": remaining, "dryRun": false});
    if let (Some(warnings), Some(obj)) = (warnings, result.as_object_mut()) {
        obj.insert("warnings".into(), warnings);
    }
    Ok(result)
}
