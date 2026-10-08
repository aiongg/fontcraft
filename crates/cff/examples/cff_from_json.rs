//! Build `CFF ` tables from JSON font descriptions: the Rust half of the cross-check against the
//! reference writer (`crates/cff/crosscheck/crosscheck.py`, which writes the JSON and compares).
//!
//! Usage: `cargo run -p fontcraft-cff --example cff_from_json -- FILE.json…`. Each `FILE.json`
//! gives `FILE.ours.cff`, or `FILE.ours.err` with the error. The JSON keys are the CFF DICT names;
//! a JSON integer becomes an integer operand and a JSON real a real one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_cff::{Font, Glyph, Number, PathOp, PrivateDict, TopDict, Widths, build};
use serde_json::Value;

fn number(v: &Value) -> Number {
    match v.as_i64() {
        Some(i) => Number::Int(i),
        None => Number::Real(v.as_f64().expect("a number")),
    }
}

fn numbers(v: Option<&Value>) -> Vec<Number> {
    v.and_then(Value::as_array).map(|a| a.iter().map(number).collect()).unwrap_or_default()
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn font(v: &Value) -> Font {
    let t = &v["top"];
    let mut top = TopDict {
        version: string(t, "version"),
        notice: string(t, "Notice"),
        copyright: string(t, "Copyright"),
        full_name: string(t, "FullName"),
        family_name: string(t, "FamilyName"),
        weight: string(t, "Weight"),
        ..TopDict::default()
    };
    for (key, slot) in [
        ("isFixedPitch", &mut top.is_fixed_pitch),
        ("ItalicAngle", &mut top.italic_angle),
        ("UnderlinePosition", &mut top.underline_position),
        ("UnderlineThickness", &mut top.underline_thickness),
        ("PaintType", &mut top.paint_type),
        ("StrokeWidth", &mut top.stroke_width),
    ] {
        if let Some(n) = t.get(key) {
            *slot = number(n);
        }
    }
    if let Some(m) = t.get("FontMatrix").and_then(Value::as_array) {
        for (slot, n) in top.font_matrix.iter_mut().zip(m) {
            *slot = number(n);
        }
    }
    let p = &v["private"];
    let mut private = PrivateDict {
        blue_values: numbers(p.get("BlueValues")),
        other_blues: numbers(p.get("OtherBlues")),
        family_blues: numbers(p.get("FamilyBlues")),
        family_other_blues: numbers(p.get("FamilyOtherBlues")),
        stem_snap_h: numbers(p.get("StemSnapH")),
        stem_snap_v: numbers(p.get("StemSnapV")),
        std_hw: p.get("StdHW").map(number),
        std_vw: p.get("StdVW").map(number),
        ..PrivateDict::default()
    };
    for (key, slot) in [
        ("BlueScale", &mut private.blue_scale),
        ("BlueShift", &mut private.blue_shift),
        ("BlueFuzz", &mut private.blue_fuzz),
        ("ForceBold", &mut private.force_bold),
        ("LanguageGroup", &mut private.language_group),
    ] {
        if let Some(n) = p.get(key) {
            *slot = number(n);
        }
    }
    let widths = match v.get("widths").and_then(Value::as_array) {
        Some(w) => Widths::Fixed { default_width_x: w[0].as_i64().unwrap(), nominal_width_x: w[1].as_i64().unwrap() },
        None => Widths::Optimise,
    };
    let glyphs = v["glyphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            let path = g["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| {
                    let op = op.as_array().unwrap();
                    let n = |i: usize| op[i].as_f64().unwrap();
                    match op[0].as_str().unwrap() {
                        "M" => PathOp::MoveTo((n(1), n(2))),
                        "L" => PathOp::LineTo((n(1), n(2))),
                        "C" => PathOp::CurveTo((n(1), n(2)), (n(3), n(4)), (n(5), n(6))),
                        other => panic!("unknown path operator {other}"),
                    }
                })
                .collect();
            Glyph { name: g["name"].as_str().unwrap().to_string(), advance: g["advance"].as_f64().unwrap(), path }
        })
        .collect();
    Font {
        name: v["name"].as_str().unwrap().to_string(),
        top,
        private,
        glyphs,
        widths,
        round_tolerance: v["round_tolerance"].as_f64().unwrap_or(0.5),
        specialise: v["specialise"].as_bool().unwrap_or(true),
    }
}

fn main() {
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        let stem = path.strip_suffix(".json").unwrap_or(&path);
        match build(&font(&v)) {
            Ok(built) => std::fs::write(format!("{stem}.ours.cff"), built.table).unwrap(),
            Err(e) => std::fs::write(format!("{stem}.ours.err"), e.to_string()).unwrap(),
        }
    }
}
