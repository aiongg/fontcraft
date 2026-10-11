//! Whole-font outline commands on sources and compiled fonts (M12.28, M12.29; `fontcraft_tables::outlines`, `fontcraft_tables::curves`). On a source each runs the document command on every glyph (one implementation per action).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};
use fontcraft_tables::outlines::{self, OutlineAction, OutlineOptions, OutlineReport, Winding};

/// Most inputs one `outlines.decompose` call takes.
const MAX_INPUTS: usize = 100_000;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = repair_specs();
    v.extend(curve_specs());
    v
}

/// `outlines.removeOverlaps`, `.correctDirection`, `.addExtremes`, `.roundCoordinates`, `.decompose` (M12.28).
fn repair_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "outlines.removeOverlaps",
            "Remove Overlaps (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], keepHinting?: bool = false, ignoreErrors?: bool = false, keepUnusedSubroutines?: bool, minArea?: number = 25, keepContourOrder?: bool = true, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. Merges overlapping contours of every selected glyph and writes the result; the input is never changed. A TrueType font: composites whose components overlap are decomposed first, a changed glyph loses its instructions, and without keepHinting every processed glyph does; other tables are copied. A source: path.removeOverlapForAllMasters on the glyphs → {input, output, kind, changed: [names], failed: [{glyph, reason}]}",
            always,
            remove_overlaps
        ),
        cmd!(
            query "outlines.correctDirection",
            "Correct Contour Direction (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], flavour?: \"postscript\" | \"truetype\" | \"reverse\" (default the input's convention: truetype for a TrueType font, postscript for a source), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. path.correctPathDirection on every selected glyph, the TrueType convention reversing the result; reverse only reverses every contour → {input, output, kind, changed, failed}",
            always,
            correct_direction
        ),
        cmd!(
            query "outlines.addExtremes",
            "Add Extremes (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], minSegment?: 3 | 0 (3, the editor's: skip an extreme closer than 3 units to a point; 0: add every extreme), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. path.addExtremes (path.forceExtremes for 0) on every selected glyph → {input, output, kind, changed, failed, pointsAdded: {glyph: count}}",
            always,
            add_extremes
        ),
        cmd!(
            query "outlines.roundCoordinates",
            "Round Coordinates (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], grid?: units (default 1, at most 1000), tolerance?: units (compiled fonts only: round only values this close to the grid), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. A compiled font: point coordinates, component offsets and advance widths to the grid. A source: path.roundCoordinatesForAllMasters with the grid → {input, output, kind, changed, failed, valuesChanged}",
            always,
            round_coordinates
        ),
        cmd!(
            query "outlines.decompose",
            "Decompose Components (Whole Font)",
            [],
            None,
            "{inputs: [path] | input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], nestedOnly?: bool, transformedOnly?: bool, ignoreErrors?: bool, output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool}. A TrueType font: every selected composite becomes contours without instructions; nestedOnly flattens components of components instead; transformedOnly decomposes only glyphs with a scaled, rotated or slanted component. A source: glyph.decomposeGlyphForAllMasters → {results: [{input, output, kind, changed, failed}]}",
            always,
            decompose
        ),
    ]
}

/// `outlines.convertCurves`, `outlines.componentize` (M12.29).
fn curve_specs() -> Vec<CommandSpec> {
    // M12.29
    Vec::new()
}

// ── M12.28 ──────────────────────────────────────────────────────────────────────────────────────

fn remove_overlaps(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.removeOverlaps", v)?;
    let mut o = common_options(&p, OutlineAction::RemoveOverlaps)?;
    o.keep_hinting = p.bool_or("keepHinting", false)?;
    p.bool_or("keepUnusedSubroutines", false)?;
    p.bool_or("keepContourOrder", true)?;
    if let Some(area) = p.opt_f64("minArea")? {
        o.min_area = area;
    }
    let input = p.str("input")?;
    run_one(&p, Path::new(input), &o)
}

fn correct_direction(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.correctDirection", v)?;
    let mut o = common_options(&p, OutlineAction::CorrectDirection)?;
    o.winding = match p.opt_str("flavour")? {
        None => Winding::Auto,
        Some("postscript") => Winding::PostScript,
        Some("truetype") => Winding::TrueType,
        Some("reverse") => Winding::Reverse,
        Some(other) => return Err(p.bad(format!("unknown flavour `{other}`: give postscript, truetype or reverse"))),
    };
    let input = p.str("input")?;
    run_one(&p, Path::new(input), &o)
}

fn add_extremes(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.addExtremes", v)?;
    let mut o = common_options(&p, OutlineAction::AddExtremes)?;
    if let Some(min) = p.opt_f64("minSegment")? {
        o.min_segment = min;
    }
    let input = p.str("input")?;
    run_one(&p, Path::new(input), &o)
}

fn round_coordinates(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.roundCoordinates", v)?;
    let mut o = common_options(&p, OutlineAction::Round)?;
    if let Some(grid) = p.opt_f64("grid")? {
        o.grid = grid;
    }
    o.tolerance = p.opt_f64("tolerance")?;
    let input = p.str("input")?;
    run_one(&p, Path::new(input), &o)
}

fn decompose(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.decompose", v)?;
    let mut o = common_options(&p, OutlineAction::Decompose)?;
    o.nested_only = p.bool_or("nestedOnly", false)?;
    o.transformed_only = p.bool_or("transformedOnly", false)?;
    let inputs: Vec<&str> = match (p.opt_strings("inputs")?, p.opt_str("input")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `inputs` or `input`, not both")),
        (Some(list), None) => list,
        (None, Some(one)) => vec![one],
        (None, None) => return Err(p.bad("`inputs` is required")),
    };
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(p.bad(format!("give 1 to {MAX_INPUTS} inputs")));
    }
    if inputs.len() > 1 && p.get("output").is_some() {
        return Err(p.bad("`output` names one file: give `outputDir` or `inPlace` for several inputs"));
    }
    // Every input is read and repaired before anything is written, so a failure leaves nothing.
    let mut done = Vec::with_capacity(inputs.len());
    for input in &inputs {
        done.push(prepare(&p, Path::new(input), &o)?);
    }
    let mut results = Vec::with_capacity(done.len());
    for d in done {
        results.push(d.write()?);
    }
    Ok(json!({ "results": results }))
}

/// The options every action reads.
fn common_options(p: &Params, action: OutlineAction) -> Result<OutlineOptions> {
    for key in ["layer", "writeLayer"] {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("`{key}`: choosing a UFO layer in whole-font outline commands")));
        }
    }
    let mut o = OutlineOptions::new(action);
    o.glyphs = names(p, "glyphs")?;
    o.exclude_glyphs = names(p, "excludeGlyphs")?;
    o.ignore_errors = p.bool_or("ignoreErrors", false)?;
    Ok(o)
}

/// A glyph list: an array of names or one comma-separated string.
fn names(p: &Params, key: &str) -> Result<Vec<String>> {
    if let Some(Value::String(s)) = p.get(key) {
        return Ok(s.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect());
    }
    let list = p.opt_strings(key)?.unwrap_or_default();
    if list.len() > fontcraft_tables::limits::MAX_GLYPHS {
        return Err(p.bad(format!("`{key}` names more than {} glyphs", fontcraft_tables::limits::MAX_GLYPHS)));
    }
    Ok(list.into_iter().map(str::to_string).collect())
}

fn run_one(p: &Params, input: &Path, o: &OutlineOptions) -> Result<Value> {
    prepare(p, input, o)?.write()
}

/// A repaired input waiting to be written.
struct Done {
    input: PathBuf,
    output: PathBuf,
    request: OutputRequest,
    output_bytes: Option<Vec<u8>>,
    /// A source and the name of the format it is saved in.
    output_source: Option<(fontcraft_model::Font, &'static str)>,
    summary: Value,
}

impl Done {
    fn write(self) -> Result<Value> {
        if let Some(bytes) = &self.output_bytes {
            file_io::write_output(&self.output, bytes, &self.request)?;
        }
        if let Some((font, format)) = &self.output_source {
            save_source(font, &self.output, format, &self.request)?;
        }
        let mut summary = self.summary;
        if let Some(map) = summary.as_object_mut() {
            map.insert("input".into(), json!(self.input.display().to_string()));
            map.insert("output".into(), json!(self.output.display().to_string()));
        }
        Ok(summary)
    }
}

fn prepare(p: &Params, input: &Path, o: &OutlineOptions) -> Result<Done> {
    let request = OutputRequest::from_params(p)?;
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    if input.is_dir() {
        return prepare_source(p, input, o, request, face);
    }
    let read = file_io::read_input(input)?;
    match read.kind {
        InputKind::TrueType | InputKind::Cff | InputKind::Collection | InputKind::Woff | InputKind::Woff2 => {
            let font = fontcraft_tables::read(&read.bytes, face.unwrap_or(0))?;
            let report = outlines::repair_report(&font, o)?;
            // A WOFF or WOFF2 input comes out as a plain font: the containers are written by `woff`.
            let extension = match read.kind {
                InputKind::Woff | InputKind::Woff2 | InputKind::Collection => Some(if font.is_cff() { "otf" } else { "ttf" }),
                _ => None,
            };
            let output = file_io::output_path(input, &request, "", extension)?;
            let bytes = fontcraft_tables::write(&report.result.font, &fontcraft_tables::WriteOptions::default())?;
            Ok(Done {
                input: input.to_path_buf(),
                output,
                request,
                output_bytes: Some(bytes),
                output_source: None,
                summary: summary(read.kind.name(), &report),
            })
        }
        InputKind::Ttx => Err(EngineError::Unsupported("outline repairs on TTX files: compile the TTX first".into())),
        InputKind::Other => prepare_source(p, input, o, request, face),
    }
}

fn summary(kind: &str, r: &OutlineReport) -> Value {
    let failed: Vec<Value> = r.failed.iter().map(|f| json!({"glyph": f.glyph, "reason": f.reason})).collect();
    let mut out = json!({"kind": kind, "changed": r.result.changed, "failed": failed});
    if let Some(map) = out.as_object_mut() {
        if !r.points_added.is_empty() {
            map.insert("pointsAdded".into(), json!(r.points_added));
        }
        if r.values_changed > 0 {
            map.insert("valuesChanged".into(), json!(r.values_changed));
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn save_source(font: &fontcraft_model::Font, path: &Path, format: &str, request: &OutputRequest) -> Result<()> {
    let format = crate::io::SaveFormat::parse(format).ok_or_else(|| EngineError::Internal(format!("unknown save format `{format}`")))?;
    if request.no_clobber && !request.in_place && path.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
    }
    crate::io::save(font, path, format).map(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn save_source(_: &fontcraft_model::Font, _: &Path, _: &str, _: &OutputRequest) -> Result<()> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

/// A source: open it, run the document command on the glyphs in a session of its own, keep the
/// result for writing.
#[cfg(not(target_arch = "wasm32"))]
fn prepare_source(p: &Params, input: &Path, o: &OutlineOptions, request: OutputRequest, face: Option<u32>) -> Result<Done> {
    use crate::io::SaveFormat;
    let (mut font, _) = crate::io::open(input, face)?;
    if font.source.format == fontcraft_model::SourceFormat::Binary {
        return Err(EngineError::Unsupported("this kind of compiled font".into()));
    }
    let output = file_io::output_path(input, &request, "", None)?;
    let format = SaveFormat::of_path(&output)
        .or_else(|| SaveFormat::of_source(font.source.format))
        .ok_or_else(|| EngineError::Io(format!("cannot tell which format to save {} in", output.display())))?;
    let mut glyphs: Vec<String> = if o.glyphs.is_empty() { font.glyphs.iter().map(|g| g.name.clone()).collect() } else { o.glyphs.clone() };
    for name in &glyphs {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("the font has no glyph named `{name}`")));
        }
    }
    glyphs.retain(|g| !o.exclude_glyphs.contains(g));
    let settings = font.info.settings.clone();
    let mut steps: Vec<(&str, Value)> = Vec::new();
    match o.action {
        OutlineAction::RemoveOverlaps => steps.push(("path.removeOverlapForAllMasters", json!({"glyphs": glyphs}))),
        OutlineAction::CorrectDirection => match o.winding {
            Winding::Auto | Winding::PostScript => steps.push(("path.correctPathDirectionForAllMasters", json!({"glyphs": glyphs}))),
            Winding::TrueType => {
                steps.push(("path.correctPathDirectionForAllMasters", json!({"glyphs": glyphs})));
                steps.push(("path.reverseContoursForAllMasters", json!({"glyphs": glyphs})));
            }
            Winding::Reverse => steps.push(("path.reverseContoursForAllMasters", json!({"glyphs": glyphs}))),
        },
        OutlineAction::AddExtremes => {
            let id = if o.min_segment == 0.0 { "path.forceExtremes" } else { "path.addExtremes" };
            let layers: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
            steps.push((id, json!({"glyphs": glyphs, "layers": layers})));
        }
        OutlineAction::Round => {
            if o.tolerance.is_some() {
                return Err(EngineError::Unsupported("`tolerance` on a source".into()));
            }
            // The document command rounds to the font's grid: the grid asked for stands in for it
            // while the command runs, and the font keeps its own.
            let info = std::sync::Arc::make_mut(&mut font.info);
            info.settings.grid_length = Some(o.grid);
            info.settings.grid_sub_division = Some(1.0);
            steps.push(("path.roundCoordinatesForAllMasters", json!({"glyphs": glyphs})));
        }
        OutlineAction::Decompose => {
            if o.nested_only || o.transformed_only {
                return Err(EngineError::Unsupported("`nestedOnly` and `transformedOnly` on a source".into()));
            }
            let composites: Vec<&String> = glyphs
                .iter()
                .filter(|g| {
                    font.glyph(g).is_some_and(|g| g.layers.iter().any(|l| l.shapes.iter().any(|s| matches!(s, fontcraft_model::Shape::Component(_)))))
                })
                .collect();
            steps.push(("glyph.decomposeGlyphForAllMasters", json!({"glyphs": composites})));
        }
    }
    let before = font.clone();
    let mut session = Session::new();
    session.push_document(font, None);
    let mut failed = Vec::new();
    for (id, params) in &steps {
        let names_empty = params.get("glyphs").and_then(Value::as_array).is_none_or(Vec::is_empty);
        if names_empty {
            continue;
        }
        let result = session.execute(id, params)?;
        if let Some(skipped) = result.get("skipped").and_then(Value::as_array) {
            for s in skipped {
                failed.push(
                    json!({"glyph": s.get("glyph").cloned().unwrap_or(Value::Null), "reason": s.get("reason").cloned().unwrap_or(Value::Null)}),
                );
            }
        }
    }
    if !failed.is_empty() && !o.ignore_errors {
        let first = failed.first().map(Value::to_string).unwrap_or_default();
        return Err(EngineError::BadParams {
            command: p.command().into(),
            message: format!("{} glyphs could not be done, the first {first}; pass ignoreErrors to write the rest", failed.len()),
        });
    }
    let mut after = session.font()?.clone();
    std::sync::Arc::make_mut(&mut after.info).settings = settings;
    let changed: Vec<&str> =
        after.glyphs.iter().filter(|g| before.glyph(&g.name).is_none_or(|old| old.layers != g.layers)).map(|g| g.name.as_str()).collect();
    let summary = json!({"kind": "source", "changed": changed, "failed": failed});
    Ok(Done { input: input.to_path_buf(), output, request, output_bytes: None, output_source: Some((after, format.name())), summary })
}

#[cfg(target_arch = "wasm32")]
fn prepare_source(_: &Params, _: &Path, _: &OutlineOptions, _: OutputRequest, _: Option<u32>) -> Result<Done> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "outlines_file_tests.rs"]
mod tests;
