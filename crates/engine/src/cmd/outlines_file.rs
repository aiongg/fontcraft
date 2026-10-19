//! Whole-font outline commands on sources and compiled fonts (M12.28, M12.29; `fontcraft_tables::outlines`, `fontcraft_tables::curves`). On a source each runs the document command on every glyph (one implementation per action).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::collection::resolved;
use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
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
        ).files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "outlines.correctDirection",
            "Correct Contour Direction (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], flavour?: \"postscript\" | \"truetype\" | \"reverse\" (default the input's convention: truetype for a TrueType font, postscript for a source), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. path.correctPathDirection on every selected glyph, the TrueType convention reversing the result; reverse only reverses every contour → {input, output, kind, changed, failed}",
            always,
            correct_direction
        ).files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "outlines.addExtremes",
            "Add Extremes (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], minSegment?: 3 | 0 (3, the editor's: skip an extreme closer than 3 units to a point; 0: add every extreme), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. path.addExtremes (path.forceExtremes for 0) on every selected glyph → {input, output, kind, changed, failed, pointsAdded: {glyph: count}}",
            always,
            add_extremes
        ).files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "outlines.roundCoordinates",
            "Round Coordinates (Whole Font)",
            [],
            None,
            "{input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], grid?: units (default 1, at most 1000), tolerance?: units (compiled fonts only: round only values this close to the grid), ignoreErrors?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. A compiled font: point coordinates, component offsets and advance widths to the grid. A source: path.roundCoordinatesForAllMasters with the grid → {input, output, kind, changed, failed, valuesChanged}",
            always,
            round_coordinates
        ).files(const { &[FileParam::read("input"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "outlines.decompose",
            "Decompose Components (Whole Font)",
            [],
            None,
            "{inputs: [path] | input: path, face?: int, glyphs?: [names], excludeGlyphs?: [names], nestedOnly?: bool, transformedOnly?: bool, ignoreErrors?: bool, output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool}. A TrueType font: every selected composite becomes contours without instructions; nestedOnly flattens components of components instead; transformedOnly decomposes only glyphs with a scaled, rotated or slanted component. A source: glyph.decomposeGlyphForAllMasters → {results: [{input, output, kind, changed, failed}]}",
            always,
            decompose
        ).files(const { &[FileParam::read("input"), FileParam::read("inputs"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
    ]
}

/// `outlines.convertCurves`, `outlines.componentize` (M12.29).
fn curve_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "outlines.convertCurves",
            "Convert Curve Type",
            [],
            None,
            "{inputs: [path] | input: path, to: \"quadratic\" | \"cubic\", maxError?: em fraction (default 0.001, above 0 and at most 1), mixed?: bool (quadratic, compiled fonts: keep a cubic curve that one quadratic curve cannot replace), allCubic?: bool (cubic: every curve, not only where it saves points), interpolatable?: bool (one input whose masters convert together; a source's masters always do), keepDirection?: bool (quadratic, sources: do not reverse the contours), face?: int, glyphs?: [names], excludeGlyphs?: [names], ignoreErrors?: bool, output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool}. A TrueType font: runs of quadratic curves become cubic curves within the error where that saves points, written as cubic glyf points (glyphDataFormat 1), or cubic glyf curves become quadratic splines; changed glyphs lose their instructions; composites and other tables stay. A source: path.convertToQuadratic (masters together, new points not rounded) then path.reverseContoursForAllMasters, or path.convertToCubic. Without output the result goes beside the input, named with .quadratic or .cubic before the extension → {results: [{input, output, kind, changed: [names], failed: [{glyph, reason}], segments, maxError: units}]}",
            always,
            convert_curves
        ).files(const { &[FileParam::read("input"), FileParam::read("inputs"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "outlines.componentize",
            "Componentize Glyphs",
            [],
            None,
            "{font: path, reference?: path (a source; default the first .ufo in the font's folder), goadb?: path (source names to the font's names; default the source's production names, else a GlyphOrderAndAliasDB beside the source or up to 3 folders above), detect?: bool (find glyphs made of other glyphs moved by whole units instead), allowTransforms?: bool (reference only: also scaled, rotated or flipped components), face?: int, glyphs?: [names], excludeGlyphs?: [names], output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. A TrueType font: each glyph the reference builds only from components (only shifted, unless allowTransforms) becomes that composite when the components give its contours; the first component supplies the metrics when the advances agree; the composite has no instructions. Without output the result goes beside the input, named with .components before the extension → {input, output, kind, changed: [names], skipped: [{glyph, reason}], reference?: path}",
            always,
            componentize
        ).files(const { &[FileParam::read("font"), FileParam::read("reference"), FileParam::read("goadb"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
    ]
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
    // Two inputs written to one file, or an output that replaces another input, would lose a
    // result: refused before anything is written.
    let mut sources: HashMap<PathBuf, usize> = HashMap::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        sources.entry(resolved(Path::new(input))).or_insert(i);
    }
    let mut targets: HashMap<PathBuf, usize> = HashMap::with_capacity(done.len());
    for (i, d) in done.iter().enumerate() {
        let target = resolved(&d.output);
        let name = |k: usize| inputs.get(k).copied().unwrap_or_default();
        if let Some(&other) = targets.get(&target) {
            return Err(p.bad(format!("{} and {} would both be written to {}", name(other), name(i), d.output.display())));
        }
        if let Some(&other) = sources.get(&target)
            && resolved(&d.input) != target
        {
            return Err(p.bad(format!("the output of {} would replace the input {}", name(i), name(other))));
        }
        targets.insert(target, i);
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

// ── M12.29 ──────────────────────────────────────────────────────────────────────────────────────

/// The reference tools' default conversion error, as a fraction of the em.
const DEFAULT_MAX_ERROR_EM: f64 = 0.001;
/// Folders above a reference source searched for a GOADB (ttfcomponentizer's three).
#[cfg(not(target_arch = "wasm32"))]
const GOADB_LEVELS_UP: usize = 3;

/// What `outlines.convertCurves` asks for, read once for every input.
struct CurveRequest {
    to_cubic: bool,
    max_error_em: f64,
    mixed: bool,
    all_cubic: bool,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    keep_direction: bool,
    glyphs: Vec<String>,
    exclude_glyphs: Vec<String>,
    ignore_errors: bool,
}

fn convert_curves(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("outlines.convertCurves", v)?;
    let to_cubic = match p.str("to")? {
        "quadratic" => false,
        "cubic" => true,
        other => return Err(p.bad(format!("unknown target `{other}`: give quadratic or cubic"))),
    };
    let max_error_em = p.opt_f64("maxError")?.unwrap_or(DEFAULT_MAX_ERROR_EM);
    if !(max_error_em.is_finite() && max_error_em > 0.0 && max_error_em <= 1.0) {
        return Err(p.bad("`maxError` is a fraction of the em, above 0 and at most 1"));
    }
    let r = CurveRequest {
        to_cubic,
        max_error_em,
        mixed: p.bool_or("mixed", false)?,
        all_cubic: p.bool_or("allCubic", false)?,
        keep_direction: p.bool_or("keepDirection", false)?,
        glyphs: names(&p, "glyphs")?,
        exclude_glyphs: names(&p, "excludeGlyphs")?,
        ignore_errors: p.bool_or("ignoreErrors", false)?,
    };
    if r.mixed && to_cubic {
        return Err(p.bad("`mixed` belongs to the conversion to quadratic curves"));
    }
    if r.all_cubic && !to_cubic {
        return Err(p.bad("`allCubic` belongs to the conversion to cubic curves"));
    }
    let interpolatable = p.bool_or("interpolatable", false)?;
    let inputs: Vec<&str> = match (p.opt_strings("inputs")?, p.opt_str("input")?) {
        (Some(_), Some(_)) => return Err(p.bad("give `inputs` or `input`, not both")),
        (Some(list), None) => list,
        (None, Some(one)) => vec![one],
        (None, None) => return Err(p.bad("`inputs` is required")),
    };
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(p.bad(format!("give 1 to {MAX_INPUTS} inputs")));
    }
    if interpolatable && inputs.len() > 1 {
        return Err(EngineError::Unsupported(
            "converting separate files as one compatible set: open the masters as one source (a designspace or a multi-master file), whose masters convert together".into(),
        ));
    }
    if inputs.len() > 1 && p.get("output").is_some() {
        return Err(p.bad("`output` names one file: give `outputDir` or `inPlace` for several inputs"));
    }
    let mut done = Vec::with_capacity(inputs.len());
    for input in &inputs {
        done.push(prepare_curves(&p, Path::new(input), &r)?);
    }
    check_targets(&p, &inputs, &done)?;
    let mut results = Vec::with_capacity(done.len());
    for d in done {
        results.push(d.write()?);
    }
    Ok(json!({ "results": results }))
}

/// Refuses two inputs written to one file, and an output that replaces another input.
fn check_targets(p: &Params, inputs: &[&str], done: &[Done]) -> Result<()> {
    let mut sources: HashMap<PathBuf, usize> = HashMap::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        sources.entry(resolved(Path::new(input))).or_insert(i);
    }
    let mut targets: HashMap<PathBuf, usize> = HashMap::with_capacity(done.len());
    let name = |k: usize| inputs.get(k).copied().unwrap_or_default();
    for (i, d) in done.iter().enumerate() {
        let target = resolved(&d.output);
        if let Some(&other) = targets.get(&target) {
            return Err(p.bad(format!("{} and {} would both be written to {}", name(other), name(i), d.output.display())));
        }
        if let Some(&other) = sources.get(&target)
            && resolved(&d.input) != target
        {
            return Err(p.bad(format!("the output of {} would replace the input {}", name(i), name(other))));
        }
        targets.insert(target, i);
    }
    Ok(())
}

/// A compiled font's units per em, as `head` gives them.
fn units_per_em(font: &fontcraft_tables::sfnt::SfntFont) -> Result<f64> {
    let head = font.get(*b"head").ok_or_else(|| EngineError::Io("the font has no head table".into()))?;
    let upm = head.get(18..20).map(|b| u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)])).unwrap_or(0);
    if !(16..=16_384).contains(&upm) {
        return Err(EngineError::Io(format!("the font's units per em ({upm}) are outside 16 to 16384")));
    }
    Ok(f64::from(upm))
}

fn prepare_curves(p: &Params, input: &Path, r: &CurveRequest) -> Result<Done> {
    use fontcraft_tables::curves::{self, CurveOptions, CurveTarget};
    let request = OutputRequest::from_params(p)?;
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    let suffix = if r.to_cubic { ".cubic" } else { ".quadratic" };
    if input.is_dir() {
        return prepare_curves_source(p, input, r, request, face, suffix);
    }
    let read = file_io::read_input(input)?;
    match read.kind {
        InputKind::TrueType | InputKind::Cff | InputKind::Collection | InputKind::Woff | InputKind::Woff2 => {
            let font = fontcraft_tables::read(&read.bytes, face.unwrap_or(0))?;
            let max_error = r.max_error_em * units_per_em(&font)?;
            let target = if r.to_cubic { CurveTarget::Cubic { max_error } } else { CurveTarget::Quadratic { max_error, reverse_direction: false } };
            let mut o = CurveOptions::new(target);
            o.all_cubic = r.all_cubic;
            o.mixed = r.mixed;
            o.glyphs = r.glyphs.clone();
            o.exclude_glyphs = r.exclude_glyphs.clone();
            o.ignore_errors = r.ignore_errors;
            let report = curves::convert_curves_report(&font, &o)?;
            let extension = match read.kind {
                InputKind::Woff | InputKind::Woff2 | InputKind::Collection => Some("ttf"),
                _ => None,
            };
            let output = file_io::output_path(input, &request, suffix, extension)?;
            let bytes = fontcraft_tables::write(&report.result.font, &fontcraft_tables::WriteOptions::default())?;
            let failed: Vec<Value> = report.failed.iter().map(|f| json!({"glyph": f.glyph, "reason": f.reason})).collect();
            let summary = json!({"kind": read.kind.name(), "changed": report.result.changed, "failed": failed, "segments": report.segments, "maxError": max_error});
            Ok(Done { input: input.to_path_buf(), output, request, output_bytes: Some(bytes), output_source: None, summary })
        }
        InputKind::Ttx => Err(EngineError::Unsupported("curve conversion of TTX files: compile the TTX first".into())),
        InputKind::Other => prepare_curves_source(p, input, r, request, face, suffix),
    }
}

/// A source: the document commands on the selected glyphs of every master, the masters
/// converted together, new points not rounded.
#[cfg(not(target_arch = "wasm32"))]
fn prepare_curves_source(p: &Params, input: &Path, r: &CurveRequest, request: OutputRequest, face: Option<u32>, suffix: &str) -> Result<Done> {
    use crate::io::SaveFormat;
    if r.mixed {
        return Err(EngineError::Unsupported("`mixed` on a source: the editor's conversion makes every curve quadratic".into()));
    }
    let (mut font, _) = crate::io::open(input, face)?;
    if font.source.format == fontcraft_model::SourceFormat::Binary {
        return Err(EngineError::Unsupported("this kind of compiled font".into()));
    }
    let output = file_io::output_path(input, &request, suffix, None)?;
    let format = SaveFormat::of_path(&output)
        .or_else(|| SaveFormat::of_source(font.source.format))
        .ok_or_else(|| EngineError::Io(format!("cannot tell which format to save {} in", output.display())))?;
    let mut glyphs: Vec<String> = if r.glyphs.is_empty() { font.glyphs.iter().map(|g| g.name.clone()).collect() } else { r.glyphs.clone() };
    for name in &glyphs {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("the font has no glyph named `{name}`")));
        }
    }
    glyphs.retain(|g| !r.exclude_glyphs.contains(g));
    let max_error = r.max_error_em * f64::from(font.units_per_em);
    let settings = font.info.settings.clone();
    // The reference converter leaves new points where they fall: no grid while the command runs.
    std::sync::Arc::make_mut(&mut font.info).settings.grid_length = Some(0.0);
    let mut steps: Vec<(&str, Value)> = Vec::new();
    if r.to_cubic {
        steps.push(("path.convertToCubic", json!({"glyphs": glyphs, "allMasters": true})));
    } else {
        steps.push(("path.convertToQuadratic", json!({"glyphs": glyphs, "allMasters": true, "tolerance": max_error})));
        if !r.keep_direction {
            steps.push(("path.reverseContoursForAllMasters", json!({"glyphs": glyphs})));
        }
    }
    let before = font.clone();
    let mut session = Session::new();
    session.push_document(font, None);
    let mut segments = 0u64;
    if !glyphs.is_empty() {
        for (id, params) in &steps {
            let result = session.execute(id, params)?;
            segments = segments.saturating_add(result.get("segments").and_then(Value::as_u64).unwrap_or(0));
        }
    }
    let mut after = session.font()?.clone();
    std::sync::Arc::make_mut(&mut after.info).settings = settings;
    let changed: Vec<&str> =
        after.glyphs.iter().filter(|g| before.glyph(&g.name).is_none_or(|old| old.layers != g.layers)).map(|g| g.name.as_str()).collect();
    let summary = json!({"kind": "source", "changed": changed, "failed": [], "segments": segments, "maxError": max_error});
    Ok(Done { input: input.to_path_buf(), output, request, output_bytes: None, output_source: Some((after, format.name())), summary })
}

#[cfg(target_arch = "wasm32")]
fn prepare_curves_source(_: &Params, _: &Path, _: &CurveRequest, _: OutputRequest, _: Option<u32>, _: &str) -> Result<Done> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

fn componentize(_: &mut Session, v: &Value) -> Result<Value> {
    use fontcraft_tables::curves::{self, ComponentizeOptions};
    let p = Params::new("outlines.componentize", v)?;
    let input = Path::new(p.str("font")?);
    let detect = p.bool_or("detect", false)?;
    let allow_transforms = p.bool_or("allowTransforms", false)?;
    if detect && (p.get("reference").is_some() || p.get("goadb").is_some()) {
        return Err(p.bad("`detect` finds components without a source: leave out `reference` and `goadb`"));
    }
    let request = OutputRequest::from_params(&p)?;
    let face = p.opt_int("face", 0, i64::from(u16::MAX))?.map(|f| f as u32);
    let read = file_io::read_input(input)?;
    let extension = match read.kind {
        InputKind::TrueType => None,
        InputKind::Woff | InputKind::Woff2 | InputKind::Collection => Some("ttf"),
        InputKind::Cff => return Err(EngineError::Unsupported("componentizing CFF outlines; TrueType (`glyf`) outlines only".into())),
        _ => return Err(p.bad(format!("{} is not a TrueType font", input.display()))),
    };
    let font = fontcraft_tables::read(&read.bytes, face.unwrap_or(0))?;
    let mut options = ComponentizeOptions {
        allow_transforms,
        glyphs: names(&p, "glyphs")?,
        exclude_glyphs: names(&p, "excludeGlyphs")?,
        ..ComponentizeOptions::default()
    };
    let mut reference_path = None;
    if !detect {
        let path = match p.opt_str("reference")? {
            Some(path) => PathBuf::from(path),
            None => source_beside(input)?,
        };
        let goadb = p.opt_str("goadb")?.map(Path::new);
        options.reference = Some(reference_glyphs(&path, goadb)?);
        reference_path = Some(path);
    }
    let report = curves::componentize_report(&font, &options)?;
    let output = file_io::output_path(input, &request, ".components", extension)?;
    let bytes = fontcraft_tables::write(&report.result.font, &fontcraft_tables::WriteOptions::default())?;
    let skipped: Vec<Value> = report.skipped.iter().map(|f| json!({"glyph": f.glyph, "reason": f.reason})).collect();
    let mut summary = json!({"kind": read.kind.name(), "changed": report.result.changed, "skipped": skipped});
    if let (Some(path), Some(map)) = (&reference_path, summary.as_object_mut()) {
        map.insert("reference".into(), json!(path.display().to_string()));
    }
    let done = Done { input: input.to_path_buf(), output, request, output_bytes: Some(bytes), output_source: None, summary };
    done.write()
}

/// The first `.ufo` in the font's folder, by name, as ttfcomponentizer looks for it.
#[cfg(not(target_arch = "wasm32"))]
fn source_beside(font: &Path) -> Result<PathBuf> {
    let folder = font.parent().filter(|f| !f.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let entries = std::fs::read_dir(folder).map_err(|e| EngineError::Io(format!("cannot list {}: {e}", folder.display())))?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ufo")))
        .take(MAX_INPUTS)
        .collect();
    found.sort();
    found
        .into_iter()
        .next()
        .ok_or_else(|| EngineError::Io(format!("no .ufo source beside {}: give `reference`, or `detect` to work without one", font.display())))
}

#[cfg(target_arch = "wasm32")]
fn source_beside(_: &Path) -> Result<PathBuf> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

/// A GlyphOrderAndAliasDB in the source's folder or up to three folders above it.
#[cfg(not(target_arch = "wasm32"))]
fn goadb_near(source: &Path) -> Option<PathBuf> {
    let mut folder = source.parent();
    for _ in 0..=GOADB_LEVELS_UP {
        let dir = folder?;
        let candidate = dir.join("GlyphOrderAndAliasDB");
        if candidate.is_file() {
            return Some(candidate);
        }
        folder = dir.parent();
    }
    None
}

/// How the reference source builds its composites, by the compiled font's glyph names: the
/// default master's layer of each glyph that has components.
#[cfg(not(target_arch = "wasm32"))]
fn reference_glyphs(path: &Path, goadb: Option<&Path>) -> Result<std::collections::BTreeMap<String, fontcraft_tables::curves::ReferenceGlyph>> {
    use fontcraft_model::Shape;
    use fontcraft_tables::curves::{ReferenceComponent, ReferenceGlyph};
    let (font, _) = crate::io::open(path, None)?;
    let master = font.default_master().map_err(|e| EngineError::Io(format!("{}: {e}", path.display())))?.id.clone();
    // Source names to the font's: an explicit GOADB, else the production names, else a GOADB
    // found near the source.
    let goadb = match goadb {
        Some(g) => Some(g.to_path_buf()),
        None if font.glyphs.iter().any(|g| g.info.production.is_some()) => None,
        None => goadb_near(path),
    };
    let table = match &goadb {
        Some(g) => {
            let read = file_io::read_input(g)?;
            let text = String::from_utf8(read.bytes).map_err(|_| EngineError::Io(format!("{} is not UTF-8 text", g.display())))?;
            Some(fontcraft_tables::goadb::parse(&text)?)
        }
        None => None,
    };
    let final_name = |name: &str| -> String {
        match &table {
            Some(t) => t.final_name(name).unwrap_or(name).to_string(),
            None => font.glyph(name).and_then(|g| g.info.production.clone()).unwrap_or_else(|| name.to_string()),
        }
    };
    let width = |name: &str| font.glyph(name).and_then(|g| g.layer(&master)).map(|l| l.width);
    let mut out = std::collections::BTreeMap::new();
    for g in font.glyphs.iter() {
        let Some(layer) = g.layer(&master) else { continue };
        let components: Vec<ReferenceComponent> = layer
            .shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Component(c) => Some(ReferenceComponent { glyph: final_name(&c.glyph), transform: c.transform.affine().as_coeffs() }),
                _ => None,
            })
            .collect();
        if components.is_empty() {
            continue;
        }
        let first = layer.shapes.iter().find_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.as_str()),
            _ => None,
        });
        let same_advance_as_first = first.and_then(width) == Some(layer.width);
        let has_contours = layer.shapes.iter().any(|s| matches!(s, Shape::Path(_)));
        out.insert(final_name(&g.name), ReferenceGlyph { components, has_contours, same_advance_as_first });
    }
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn reference_glyphs(_: &Path, _: Option<&Path>) -> Result<std::collections::BTreeMap<String, fontcraft_tables::curves::ReferenceGlyph>> {
    Err(EngineError::Unsupported("reading and writing files by path in the browser".into()))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
#[path = "outlines_file_tests.rs"]
mod tests;

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
#[path = "outlines_file_curves_tests.rs"]
mod curve_tests;
