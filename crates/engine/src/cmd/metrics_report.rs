//! `font.reportMetrics` (M12.16; `fontcraft_tables::metrics`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! The query reads a compiled font (TrueType, CFF, CFF2, WOFF, WOFF2 or one face of a
//! collection) and returns the report as JSON. With `format` `text`, `csv` or `afm` the result
//! also holds that text; with `output` the text (or the JSON) is written to that file instead of
//! being returned in full. With `location` a variable font is first instantiated there.

use std::path::Path;

use serde_json::{Map, Value, json};

use fontcraft_tables::instance::{AxisLimit, InstanceOptions};
use fontcraft_tables::metrics::{self, MetricsOptions, MetricsReport, TextOptions};
use fontcraft_tables::sfnt::{self, SfntFont};

use super::file_io::{self, OutputRequest};
use super::*;
use crate::EngineError;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.reportMetrics",
        "Report Metrics",
        [],
        None,
        "{font: path, face?: int|name, format?: \"text\"|\"json\"|\"csv\"|\"afm\", precision?: \"integer\"|\"real\", aggregateBbox?: bool, sidebearings?: bool, units?: \"font\"|1000, kerning?: bool, statistics?: bool, controlBox?: bool, fontOnly?: bool, glyphs?: selector, excludeGlyphs?: selector, fd?: [int], location?: {axis: number}, output?: path, noClobber?: bool} glyph metrics of a compiled font measured from its outlines: advance, exact bounding box, side bearings, the font bounding box with the glyphs that set its edges, flattened kerning pairs and shape statistics (area, centre of mass, variance, covariance, correlation, slant; area is negative for clockwise outer contours); a selector is a glyph-list string or a list of them → {unitsPerEm, units, glyphs: [{gid, name, unicode?, advance, bounds?, lsb?, rsb?, statistics?}], aggregate?, kerning?, fontStatistics?, warnings, text?, written?}",
        always,
        report_metrics
    ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files()]
}

fn report_metrics(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.reportMetrics", p)?;
    let path = p.str("font")?;
    let format = p.opt_str("format")?.unwrap_or("text");
    if !matches!(format, "text" | "json" | "csv" | "afm") {
        return Err(p.bad(format!("`format` must be \"text\", \"json\", \"csv\" or \"afm\", got \"{format}\"")));
    }
    let real = match p.opt_str("precision")?.unwrap_or("integer") {
        "integer" => false,
        "real" => true,
        other => return Err(p.bad(format!("`precision` must be \"integer\" or \"real\", got \"{other}\""))),
    };
    let per_1000 = match p.get("units") {
        None => false,
        Some(Value::String(s)) if s == "font" => false,
        Some(Value::String(s)) if s == "1000" => true,
        Some(v) if v.as_f64() == Some(1000.0) => true,
        Some(_) => return Err(p.bad("`units` must be \"font\" or 1000")),
    };
    let text = TextOptions {
        real,
        sidebearings: p.bool_or("sidebearings", false)?,
        aggregate_bbox: p.bool_or("aggregateBbox", false)?,
        font_only: p.bool_or("fontOnly", false)?,
    };
    let statistics = p.bool_or("statistics", false)?;
    let control_box = p.bool_or("controlBox", false)?;
    if (control_box || text.font_only) && !statistics {
        return Err(p.bad("`controlBox` and `fontOnly` need `statistics`"));
    }
    let fd = match p.opt_array("fd")? {
        None => Vec::new(),
        Some(items) => items
            .iter()
            .map(|v| v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(|| p.bad("`fd` must be a list of font dictionary numbers")))
            .collect::<Result<Vec<_>>>()?,
    };
    let options = MetricsOptions {
        glyphs: selector(&p, "glyphs")?,
        exclude_glyphs: selector(&p, "excludeGlyphs")?,
        fd,
        kerning: p.bool_or("kerning", false)?,
        statistics,
        control_box,
    };
    let limits = location(&p)?;
    let output = OutputRequest::from_params(&p)?;
    if output.in_place || output.output_dir.is_some() {
        return Err(p.bad("the report is written with `output`; `inPlace` and `outputDir` do not apply"));
    }

    let input = file_io::read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return Err(p.bad(format!("{path} is not a compiled font (a {} file); compile a source before measuring it", input.kind.name())));
    }
    let font = read_face(&p, &input.bytes)?;
    let font = if limits.is_empty() {
        font
    } else {
        fontcraft_tables::instance::instantiate(&font, &InstanceOptions { limits, ..InstanceOptions::default() })?
    };
    let measured = metrics::report(&font, &options)?;
    let measured = if per_1000 { measured.rescaled(1000.0) } else { measured };

    let mut result = to_json(&measured, &text);
    let rendered = match format {
        "text" => Some(metrics::to_text(&measured, &text)),
        "csv" => Some(metrics::to_csv(&measured, &text)),
        "afm" => Some(metrics::to_afm(&measured, &text)),
        _ => None,
    };
    if let Some(out) = &output.output {
        let bytes = match &rendered {
            Some(t) => t.clone().into_bytes(),
            None => serde_json::to_vec_pretty(&result).map_err(|e| EngineError::Io(e.to_string()))?,
        };
        file_io::write_output(out, &bytes, &output)?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("written".into(), json!(out.display().to_string()));
        }
    } else if let (Some(t), Some(obj)) = (rendered, result.as_object_mut()) {
        obj.insert("text".into(), json!(t));
    }
    Ok(result)
}

/// A glyph selector: one glyph-list string, or a list of them.
fn selector(p: &Params<'_>, key: &str) -> Result<Vec<String>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(_)) => Ok(p.strings(key)?.into_iter().map(str::to_string).collect()),
        Some(_) => Err(p.bad(format!("`{key}` must be a glyph list: a string or a list of strings"))),
    }
}

/// `location` as pinned axes, by four-letter tag.
fn location(p: &Params<'_>) -> Result<Vec<(fontcraft_tables::Tag, AxisLimit)>> {
    let Some(map) = p.opt_object("location")? else { return Ok(Vec::new()) };
    if map.len() > fontcraft_tables::limits::MAX_AXES {
        return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
    }
    map.iter()
        .map(|(axis, v)| {
            let value = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("`location.{axis}` must be a finite number")))?;
            let tag = sfnt::parse_tag(axis).map_err(|_| p.bad(format!("`{axis}` is not an axis tag")))?;
            Ok((tag, AxisLimit::Pin(value)))
        })
        .collect()
}

/// The face `face` names: its index, or its PostScript name.
fn read_face(p: &Params<'_>, bytes: &[u8]) -> Result<SfntFont> {
    match p.get("face") {
        None | Some(Value::Null) => Ok(sfnt::read(bytes, 0)?),
        Some(Value::String(name)) => {
            let probe = MetricsOptions { glyphs: vec!["0".into()], ..MetricsOptions::default() };
            for face in 0..sfnt::face_count(bytes)? {
                let font = sfnt::read(bytes, face)?;
                if metrics::report(&font, &probe).is_ok_and(|r| &r.info.postscript_name == name) {
                    return Ok(font);
                }
            }
            Err(p.bad(format!("the file has no face whose PostScript name is `{name}`")))
        }
        Some(v) => {
            let face = v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a face number or a PostScript name"))?;
            Ok(sfnt::read(bytes, face)?)
        }
    }
}

fn to_json(r: &MetricsReport, o: &TextOptions) -> Value {
    let mut out = Map::new();
    out.insert("unitsPerEm".into(), json!(r.units_per_em));
    out.insert("units".into(), json!(r.units));
    if !o.font_only {
        let glyphs: Vec<Value> = r
            .glyphs
            .iter()
            .map(|g| {
                let mut row = Map::new();
                row.insert("gid".into(), json!(g.gid));
                row.insert("name".into(), json!(g.name));
                if let Some(u) = g.unicode {
                    row.insert("unicode".into(), json!(format!("U+{u:04X}")));
                }
                row.insert("advance".into(), json!(g.advance));
                if let Some(b) = g.bounds {
                    row.insert("bounds".into(), json!(b));
                }
                if o.sidebearings {
                    row.insert("lsb".into(), json!(g.left_side_bearing));
                    row.insert("rsb".into(), json!(g.right_side_bearing));
                }
                if let Some(s) = g.statistics {
                    row.insert(
                        "statistics".into(),
                        json!({ "area": s.area, "mean": s.mean, "variance": s.variance, "stddev": s.stddev, "covariance": s.covariance, "correlation": s.correlation, "slant": s.slant }),
                    );
                }
                Value::Object(row)
            })
            .collect();
        out.insert("glyphs".into(), Value::Array(glyphs));
    }
    if o.aggregate_bbox
        && let Some(fb) = &r.font_bounds
    {
        out.insert("aggregate".into(), json!({ "bounds": fb.bounds, "gids": fb.gids, "glyphs": fb.names }));
    }
    if !r.kerning.is_empty() {
        let pairs: Vec<Value> = r.kerning.iter().map(|k| json!({ "left": k.left, "right": k.right, "value": k.value })).collect();
        out.insert("kerning".into(), Value::Array(pairs));
    }
    if let Some(f) = &r.font_statistics {
        out.insert(
            "fontStatistics".into(),
            json!({
                "weight": f.weight,
                "weightPerceptual": f.weight_perceptual,
                "width": f.width,
                "slant": f.slant,
                "slantAngle": f.slant_angle,
                "slantPerceptual": f.slant_perceptual,
                "slantPerceptualAngle": f.slant_perceptual_angle,
            }),
        );
    }
    out.insert("warnings".into(), json!(r.warnings));
    Value::Object(out)
}
