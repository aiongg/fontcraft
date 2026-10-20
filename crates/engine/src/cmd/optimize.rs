//! `font.optimize` (M12.26; `fontcraft_tables::optimize`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. A WOFF, WOFF2 or collection input is written back
//! as one, with the other members of a collection unchanged.
//!
//! Without a step named, every built lossless step runs: CFF widths and subroutines, implied
//! on-curve points, `gvar` delta optimisation, the `GDEF` variation store and `COLR` layer reuse.
//! Naming steps runs only those. GPOS compaction runs only when a level above 0 is given, as in
//! the reference tool. Steps whose packages have not landed (charstring rewriting, the speed
//! preset) are refused when named.

use std::path::PathBuf;

use fontcraft_tables::optimize::gpos::MAX_LEVEL;
use fontcraft_tables::optimize::varstore::MAX_QUANTUM;
use fontcraft_tables::optimize::{DeltaMode, OptimizeOptions, optimize};
use fontcraft_tables::sfnt::tag_str;
use fontcraft_tables::{SfntFont, TableOrder};
use serde_json::{Value, json};

use super::file_io::{self, OutputRequest};
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
        "{font: path, face?: int|name, preset?: \"size\", subroutinize?: bool, maxSubrs?: int, desubroutinize?: bool, cffWidths?: bool|\"exhaustive\", gposCompaction?: 0..9, varstore?: bool|{quantum: int}, gvarIup?: bool, dropImpliedOncurves?: bool, colrLayerReuse?: bool, canonicalOrder?: bool, report?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool}. Re-encodes a compiled font more compactly without changing what it draws or how it shapes. Without a step named it chooses the CFF default and nominal widths again, subroutinises CFF and CFF2 charstrings, drops implied on-curve points, optimises gvar deltas, merges the GDEF variation store and shares COLR layers; naming steps runs only those. maxSubrs caps the subroutines of one INDEX (1 to 65,535) and implies subroutinize. desubroutinize expands every subroutine. gvarIup true keeps only the gvar deltas interpolation cannot infer, false writes every delta. varstore {quantum} rounds the store's deltas to multiples of quantum (1 to 32,767) first, which is lossy above 1. dropImpliedOncurves removes on-curve points halfway between their off-curve neighbours at every location, except in glyphs with instructions. colrLayerReuse builds a version 1 COLR table again, sharing runs of layers. gposCompaction 1 to 8 splits each class-kerning subtable into denser ones, accepting up to that many extra subtables per halving of its size; 9 splits as long as that saves bytes; 0 (the default, also in a run without steps named) leaves class kerning alone. A step that would not make a table smaller is not applied, and a step that fails keeps its tables, with a note. canonicalOrder writes the tables in the recommended order (default: the input's). A WOFF, WOFF2 or collection is written as one, the other members of a collection unchanged. report measures without writing. Not built yet, refused when asked for: preset \"speed\", charstrings → {output: path|null, written, bytes, bytesBefore, bytesAfter, steps: [{step, table, before, after, applied}], tables: [{table, before, after}], notes}",
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

    // The font is written back in its container (WOFF, WOFF2, or the collection with its other
    // members), so `inPlace` never drops what the file held besides the optimised face.
    let loaded = super::tables_file::load(&p, "font", "face")?;
    let font = &loaded.font;
    let result = optimize(font, &options)?;
    let bytes = super::tables_file::save(&loaded.wrap, &result.font, order)?;

    let output = if report_only {
        None
    } else {
        let path = file_io::output_path(&input, &request, ".optimized", None)?;
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
        "tables": table_sizes(font, &result.font),
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
    if named("charstrings") {
        return Err(not_built("rewriting charstrings"));
    }
    let gpos_compaction = p.opt_int("gposCompaction", 0, i64::from(MAX_LEVEL))?.and_then(|n| u8::try_from(n).ok()).unwrap_or(0);
    // `gvarIup: false` names a step too: writing every delta.
    let gvar = match p.get("gvarIup") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(true)) => Some(DeltaMode::Optimize),
        Some(Value::Bool(false)) => Some(DeltaMode::All),
        Some(_) => return Err(p.bad("`gvarIup` must be true or false")),
    };
    let quantum = match p.get("varstore") {
        Some(Value::Object(o)) => match o.get("quantum") {
            None | Some(Value::Null) => 1,
            Some(q) => q
                .as_u64()
                .and_then(|q| u32::try_from(q).ok())
                .filter(|q| (1..=MAX_QUANTUM).contains(q))
                .ok_or_else(|| p.bad(format!("`varstore.quantum` must be a whole number from 1 to {MAX_QUANTUM}")))?,
        },
        None | Some(Value::Null) | Some(Value::Bool(_)) => 1,
        Some(_) => return Err(p.bad("`varstore` must be true, false or {quantum}")),
    };
    let max_subrs = p.opt_int("maxSubrs", 1, 65_535)?.and_then(|n| usize::try_from(n).ok());
    let cff_widths = match p.get("cffWidths") {
        None | Some(Value::Null) | Some(Value::Bool(_)) => p.bool_or("cffWidths", false)?,
        // The fast search already finds the smallest pair.
        Some(Value::String(s)) if s == "exhaustive" => true,
        Some(_) => return Err(p.bad("`cffWidths` must be true, false or \"exhaustive\"")),
    };
    let desubroutinize = p.bool_or("desubroutinize", false)?;
    let drop_implied_oncurves = p.bool_or("dropImpliedOncurves", false)?;
    let colr_layer_reuse = p.bool_or("colrLayerReuse", false)?;
    let any = STEPS.iter().any(|k| named(k)) || gvar.is_some();
    if !any {
        // Every built step, less the ones turned off by name.
        let off = |key: &str| matches!(p.get(key), Some(Value::Bool(false)));
        let d = OptimizeOptions::default();
        return Ok(OptimizeOptions {
            subroutines: !off("subroutinize"),
            cff_widths: !off("cffWidths"),
            variation_store: !off("varstore"),
            drop_implied_oncurves: !off("dropImpliedOncurves"),
            colr_layer_reuse: !off("colrLayerReuse"),
            gpos_compaction,
            ..d
        });
    }
    let subroutines = p.bool_or("subroutinize", false)? || (max_subrs.is_some() && !desubroutinize);
    Ok(OptimizeOptions {
        cff_widths,
        subroutines,
        variation_store: named("varstore"),
        quantum,
        desubroutinize,
        max_subrs,
        gvar,
        drop_implied_oncurves,
        colr_layer_reuse,
        gpos_compaction,
    })
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

        for bad in [
            json!({"font": font, "gposCompaction": 10}),
            json!({"font": font, "preset": "speed"}),
            json!({"font": font, "cffWidths": 2}),
            json!({"font": font, "varstore": {"quantum": 0}}),
            json!({"font": font, "gvarIup": "yes"}),
        ] {
            assert!(session.execute("font.optimize", &bad).is_err(), "{bad}");
        }
        // A font without class kerning: the step runs alone and changes nothing.
        let kern = session.execute("font.optimize", &json!({"font": font, "gposCompaction": 3, "report": true})).unwrap();
        assert!(applied(&kern).is_empty(), "{kern}");
        let both = session.execute("font.optimize", &json!({"font": font, "subroutinize": true, "desubroutinize": true}));
        assert!(both.is_err());
        let widths_only = session.execute("font.optimize", &json!({"font": font, "subroutinize": false, "report": true})).unwrap();
        assert!(!applied(&widths_only).contains(&"subroutinize"), "{widths_only}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn font_optimize_in_place_keeps_the_container() {
        use fontcraft_tables::collection::{PackOptions, pack, unpack};
        let dir = std::env::temp_dir().join(format!("fontcraft-optimize-wrap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let otf = plain_otf();
        let member = fontcraft_tables::read(&otf, 0).unwrap();
        let mut session = Session::new();

        // Optimising one member of a collection in place keeps the other.
        let ttc = dir.join("Fam.ttc");
        std::fs::write(&ttc, pack(&[member.clone(), member.clone()], &PackOptions::default()).unwrap()).unwrap();
        session.execute("font.optimize", &json!({"font": ttc.display().to_string(), "face": 1, "inPlace": true})).unwrap();
        let bytes = std::fs::read(&ttc).unwrap();
        assert_eq!(file_io::detect(&bytes), file_io::InputKind::Collection);
        let fonts = unpack(&bytes).unwrap();
        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].get(*b"CFF "), member.get(*b"CFF "), "the other member is unchanged");
        assert!(fonts[1].get(*b"CFF ").unwrap().len() < member.get(*b"CFF ").unwrap().len(), "the chosen member is optimised");

        // A WOFF stays a WOFF, in place and under its default name.
        let woff = dir.join("A.woff");
        std::fs::write(&woff, fontcraft_container::woff::encode(&otf, &fontcraft_container::Extras::default()).unwrap()).unwrap();
        let derived = session.execute("font.optimize", &json!({"font": woff.display().to_string()})).unwrap();
        let out = dir.join("A.optimized.woff");
        assert_eq!(derived["output"], json!(out.display().to_string()));
        assert_eq!(file_io::detect(&std::fs::read(&out).unwrap()), file_io::InputKind::Woff);
        session.execute("font.optimize", &json!({"font": woff.display().to_string(), "inPlace": true})).unwrap();
        assert_eq!(file_io::detect(&std::fs::read(&woff).unwrap()), file_io::InputKind::Woff);

        // A file holding one font takes that font's PostScript name as `face`, and no other.
        let plain = dir.join("Plain.otf");
        std::fs::write(&plain, &otf).unwrap();
        let ps = super::super::tables_file::postscript_name(&member).unwrap();
        let font = plain.display().to_string();
        session.execute("font.optimize", &json!({"font": font, "face": ps, "report": true})).unwrap();
        session.execute("font.optimize", &json!({"font": font, "face": 0, "report": true})).unwrap();
        assert!(session.execute("font.optimize", &json!({"font": font, "face": "Other-Name", "report": true})).is_err());
        assert!(session.execute("font.optimize", &json!({"font": font, "face": 1, "report": true})).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
