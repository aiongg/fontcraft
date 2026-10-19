//! `font.optimize` (M12.26; `fontcraft_tables::optimize`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! Without a step named, every built lossless step runs: CFF widths and subroutines. Naming steps
//! runs only those. Steps whose packages have not landed (GPOS compaction, variation store
//! merging, gvar delta optimisation, implied on-curve points, COLR layer reuse, charstring
//! rewriting, the speed preset) are refused when named.

use std::path::PathBuf;

use fontcraft_tables::optimize::{OptimizeOptions, optimize};
use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::{SfntFont, TableOrder, WriteOptions};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// The parameters that name a step.
const STEPS: &[&str] = &[
    "subroutinize",
    "maxSubrs",
    "desubroutinize",
    "cffWidths",
    "charstrings",
    "gposCompaction",
    "varstore",
    "gvarIup",
    "dropImpliedOncurves",
    "colrLayerReuse",
];

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.optimize",
        "Optimize Font",
        [],
        None,
        "{font: path, face?: int|name, preset?: \"size\", subroutinize?: bool, maxSubrs?: int, desubroutinize?: bool, cffWidths?: bool|\"exhaustive\", gposCompaction?: 0, varstore?: false, canonicalOrder?: bool, report?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. Re-encodes a compiled font more compactly without changing what it draws or how it shapes. Without a step named it chooses the CFF default and nominal widths again and subroutinises CFF and CFF2 charstrings; naming steps runs only those. maxSubrs caps the subroutines of one INDEX (1 to 65,535) and implies subroutinize. desubroutinize expands every subroutine. A step that would not make a table smaller is not applied, and a table the subroutiniser refuses is kept, with a note. canonicalOrder writes the tables in the recommended order (default: the input's). report measures without writing. Not built yet, refused when asked for: preset \"speed\", charstrings, gposCompaction above 0, varstore, gvarIup, dropImpliedOncurves, colrLayerReuse → {output: path|null, written, bytes, bytesBefore, bytesAfter, steps: [{step, table, before, after, applied}], tables: [{table, before, after}], notes}",
        always,
        optimize_font
    ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files()]
}

fn optimize_font(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.optimize", v)?;
    let input = PathBuf::from(p.str("font")?);
    let request = OutputRequest::from_params(&p)?;
    let options = options(&p)?;
    let order = if p.bool_or("canonicalOrder", false)? { TableOrder::Recommended } else { TableOrder::Keep };
    let report_only = p.bool_or("report", false)?;

    let file = file_io::read_input(&input)?;
    if !file.kind.is_font() {
        return Err(EngineError::Unsupported(format!("optimizing {}: give a compiled font, not a source", input.display())));
    }
    let face = super::dehint::face_index(&p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    let result = optimize(&font, &options)?;
    let bytes = fontcraft_tables::write(&result.font, &WriteOptions { order })?;

    let output = if report_only {
        None
    } else {
        let extension = match file.kind {
            InputKind::TrueType | InputKind::Cff => None,
            _ if result.font.is_cff() => Some("otf"),
            _ => Some("ttf"),
        };
        let path = file_io::output_path(&input, &request, ".optimized", extension)?;
        file_io::write_output(&path, &bytes, &request)?;
        Some(path.display().to_string())
    };
    let steps: Vec<Value> = result
        .steps
        .iter()
        .map(|s| json!({"step": s.step, "table": tag_str(s.table), "before": s.before, "after": s.after, "applied": s.applied}))
        .collect();
    Ok(json!({
        "output": output,
        "written": !report_only,
        "bytes": bytes.len(),
        "bytesBefore": result.bytes_before,
        "bytesAfter": result.bytes_after,
        "steps": steps,
        "tables": table_sizes(&font, &result.font),
        "notes": result.notes,
    }))
}

/// The steps the parameters ask for.
fn options(p: &Params<'_>) -> Result<OptimizeOptions> {
    let named = |key: &str| match p.get(key) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(_) => true,
    };
    match p.opt_str("preset")? {
        None | Some("size") => {}
        Some("speed") => return Err(not_built("the speed preset")),
        Some(other) => return Err(p.bad(format!("`preset` must be \"size\" or \"speed\", got \"{other}\""))),
    }
    for (key, what) in [
        ("charstrings", "rewriting charstrings"),
        ("gposCompaction", "GPOS compaction"),
        ("varstore", "variation store merging"),
        ("dropImpliedOncurves", "dropping implied on-curve points"),
        ("colrLayerReuse", "COLR layer reuse"),
    ] {
        if named(key) {
            return Err(not_built(what));
        }
    }
    if p.get("gvarIup").is_some_and(|v| !v.is_null()) {
        return Err(not_built("gvar delta optimisation"));
    }
    let max_subrs = p.opt_int("maxSubrs", 1, 65_535)?.and_then(|n| usize::try_from(n).ok());
    let cff_widths = match p.get("cffWidths") {
        None | Some(Value::Null) | Some(Value::Bool(_)) => p.bool_or("cffWidths", false)?,
        // The fast search already finds the smallest pair.
        Some(Value::String(s)) if s == "exhaustive" => true,
        Some(_) => return Err(p.bad("`cffWidths` must be true, false or \"exhaustive\"")),
    };
    let desubroutinize = p.bool_or("desubroutinize", false)?;
    let any = STEPS.iter().any(|k| named(k));
    if !any {
        // Every built step, less the ones turned off by name.
        let off = |key: &str| matches!(p.get(key), Some(Value::Bool(false)));
        return Ok(OptimizeOptions { subroutines: !off("subroutinize"), cff_widths: !off("cffWidths"), ..OptimizeOptions::default() });
    }
    let subroutines = p.bool_or("subroutinize", false)? || (max_subrs.is_some() && !desubroutinize);
    Ok(OptimizeOptions { gpos: false, cff_widths, subroutines, variation_store: false, desubroutinize, max_subrs })
}

fn not_built(what: &str) -> EngineError {
    EngineError::Unsupported(format!("{what} is not built yet"))
}

/// Every table's size in the input and the output, in the input's order.
fn table_sizes(before: &SfntFont, after: &SfntFont) -> Vec<Value> {
    let mut tags = before.tags();
    tags.extend(after.tags().into_iter().filter(|t| before.get(*t).is_none()));
    tags.iter().map(|&t| json!({"table": tag_str(t), "before": before.get(t).map(<[u8]>::len), "after": after.get(t).map(<[u8]>::len)})).collect()
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use fontcraft_model::{Font, Path, Shape};

    /// A CFF font whose capitals are the same two bars, compiled without subroutines.
    fn plain_otf() -> Vec<u8> {
        let mut f = Font::new("Optimize Sans");
        for c in 'A'..='Z' {
            let name = c.to_string();
            for (x0, x1) in [(50.0, 250.0), (300.0, 500.0)] {
                f.glyph_mut(&name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x0, 0.0, x1, 700.0)));
            }
        }
        let options = fontcraft_compile::Options { subroutinize: Some(false), ..fontcraft_compile::Options::default() };
        fontcraft_compile::compile(&f, &f.default_target().unwrap(), fontcraft_compile::Flavour::Cff, &options).unwrap().bytes
    }

    /// The `CFF ` table's size before and after, from a result.
    fn cff_sizes(result: &Value) -> (u64, u64) {
        let row = result["tables"].as_array().unwrap().iter().find(|t| t["table"] == json!("CFF ")).unwrap();
        (row["before"].as_u64().unwrap(), row["after"].as_u64().unwrap())
    }

    fn applied(result: &Value) -> Vec<&str> {
        result["steps"].as_array().unwrap().iter().filter(|s| s["applied"] == json!(true)).filter_map(|s| s["step"].as_str()).collect()
    }

    #[test]
    fn font_optimize_subroutinises_a_cff_font_and_leaves_the_input() {
        let dir = std::env::temp_dir().join(format!("fontcraft-optimize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("A.otf");
        let original = plain_otf();
        std::fs::write(&input, &original).unwrap();
        let font = input.display().to_string();

        let mut session = Session::new();
        let report = session.execute("font.optimize", &json!({"font": font, "report": true})).unwrap();
        assert_eq!(report["output"], Value::Null);
        assert!(!dir.join("A.optimized.otf").exists(), "report writes nothing");

        let result = session.execute("font.optimize", &json!({"font": font})).unwrap();
        let output = dir.join("A.optimized.otf");
        assert_eq!(result["output"], json!(output.display().to_string()));
        assert_eq!(result["bytes"], report["bytes"]);
        assert!(result["bytesAfter"].as_u64().unwrap() < result["bytesBefore"].as_u64().unwrap(), "{result}");
        assert!(applied(&result).contains(&"subroutinize"), "{result}");
        let (before, after) = cff_sizes(&result);
        assert!(after < before, "{result}");
        assert_eq!(std::fs::read(&input).unwrap(), original, "the input is never changed");

        let flat = dir.join("flat.otf");
        let expanded = session
            .execute("font.optimize", &json!({"font": output.display().to_string(), "desubroutinize": true, "output": flat.display().to_string()}))
            .unwrap();
        assert_eq!(applied(&expanded), ["desubroutinize"], "{expanded}");
        assert!(flat.exists());

        for bad in [json!({"font": font, "gposCompaction": 3}), json!({"font": font, "preset": "speed"}), json!({"font": font, "cffWidths": 2})] {
            assert!(session.execute("font.optimize", &bad).is_err(), "{bad}");
        }
        let both = session.execute("font.optimize", &json!({"font": font, "subroutinize": true, "desubroutinize": true}));
        assert!(both.is_err());
        let widths_only = session.execute("font.optimize", &json!({"font": font, "subroutinize": false, "report": true})).unwrap();
        assert!(!applied(&widths_only).contains(&"subroutinize"), "{widths_only}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
