//! Variation utilities (M12.33, M12.34; `fontcraft_tables::variations`, `fontcraft_tables::variations_model`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use std::path::Path;

use fontcraft_tables::sfnt::{SfntFont, Tag, parse_tag, tag_str};
use fontcraft_tables::variations::{self, Space};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::*;
use crate::params::Params;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = map_specs();
    v.extend(model_specs());
    v
}

/// `variations.mapCoordinates`, `variations.buildAvar` (M12.33).
fn map_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "variations.mapCoordinates",
            "Map Variation Coordinates",
            [],
            None,
            "{font: path, location: {axis: number}, from?: \"user\"|\"normalized\", to?: \"normalized\"|\"user\", allAxes?: bool, face?: number} converts a location of a variable font through fvar normalisation and the avar table (cross-axis mappings of avar version 2 included); axes not given are at their default; `to` defaults to the space other than `from`; `location` lists the axes given (with avar version 2, the axes that end away from 0), every axis with allAxes → {axes: [tag], to, location: {axis: number}, user: {axis: number}, normalized: {axis: number}, mapped: {axis: number}}",
            always,
            map_coordinates
        ),
        cmd!(
            "variations.buildAvar",
            "Build avar Table",
            [],
            None,
            "{font: path, designspace?: path, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} writes the avar table of a TrueType or CFF variable font from the axis maps and cross-axis mappings of a designspace, or rebuilt from the font's own avar table without one; every map the identity and no cross-axis mapping writes no avar table; the output defaults to <name>.avar.<ext> beside the input → {output, avar: bool, version?: 1|2, segments: {axis: [[from, to]]}}",
            always,
            build_avar
        ),
    ]
}

/// `variations.buildHvar`, `.describeModel`, `.plot`, `.planAvar` (M12.34).
fn model_specs() -> Vec<CommandSpec> {
    // M12.34
    Vec::new()
}

fn read_font(p: &Params<'_>, path: &str, face: u32) -> Result<(file_io::Input, SfntFont)> {
    let input = file_io::read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return Err(p.bad(format!("{path} is not a compiled font")));
    }
    let font = fontcraft_tables::read(&input.bytes, face)?;
    Ok((input, font))
}

fn space(p: &Params<'_>, key: &str) -> Result<Option<Space>> {
    match p.opt_str(key)? {
        None => Ok(None),
        Some("user") => Ok(Some(Space::User)),
        Some("normalized") => Ok(Some(Space::Normalized)),
        Some(other) => Err(p.bad(format!("`{key}` must be \"user\" or \"normalized\", got \"{other}\""))),
    }
}

fn tagged(values: &[(Tag, f64)]) -> Value {
    Value::Object(values.iter().map(|(t, v)| (tag_str(*t).trim_end().to_string(), json!(v))).collect())
}

fn map_coordinates(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.mapCoordinates", params)?;
    let path = p.str("font")?;
    let face = p.opt_int("face", 0, i64::from(u32::MAX))?.and_then(|f| u32::try_from(f).ok()).unwrap_or(0);
    let from = space(&p, "from")?.unwrap_or(Space::User);
    let to = space(&p, "to")?.unwrap_or(if from == Space::User { Space::Normalized } else { Space::User });
    let all_axes = p.bool_or("allAxes", false)?;
    let given = p.opt_object("location")?.ok_or_else(|| p.bad("`location` is required"))?;
    if given.len() > fontcraft_tables::limits::MAX_AXES {
        return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
    }
    let mut location: Vec<(Tag, f64)> = Vec::with_capacity(given.len());
    for (axis, v) in given {
        let tag = parse_tag(axis).map_err(|_| p.bad(format!("`{axis}` is not an axis tag")))?;
        let v = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("the value of `{axis}` must be a finite number")))?;
        location.push((tag, v));
    }
    let (_, font) = read_font(&p, path, face)?;
    let axes = variations::fvar_axes(&font)?;
    let result = variations::map_coordinates(&font, std::slice::from_ref(&location), from)?;
    let Some(m) = result.first() else { return Err(p.bad("no location was mapped")) };
    // The axes fontTools prints: with avar version 2 those away from 0, otherwise those given.
    let cross_axis = variations::avar_version(&font)? == Some(2);
    let mut shown = Map::new();
    for (axis, &(tag, v)) in axes.iter().zip(&m.mapped) {
        let given = location.iter().any(|(t, _)| *t == tag);
        let show = all_axes || if cross_axis { v != 0.0 } else { given };
        if show {
            let value = if to == Space::User { variations::denormalize_value(v, axis) } else { v };
            shown.insert(tag_str(tag).trim_end().to_string(), json!(value));
        }
    }
    Ok(json!({
        "axes": axes.iter().map(|a| tag_str(a.tag).trim_end().to_string()).collect::<Vec<_>>(),
        "to": if to == Space::User { "user" } else { "normalized" },
        "location": shown,
        "user": tagged(&m.user),
        "normalized": tagged(&m.normalized),
        "mapped": tagged(&m.mapped),
    }))
}

fn build_avar(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.buildAvar", params)?;
    let path = p.str("font")?;
    let req = OutputRequest::from_params(&p)?;
    let (input, font) = read_font(&p, path, 0)?;
    if !matches!(input.kind, InputKind::TrueType | InputKind::Cff) {
        return Err(p.bad(format!("{path} is a {} file: give a TrueType or CFF font (unpack collections and WOFF files first)", input.kind.name())));
    }
    let options = match p.opt_str("designspace")? {
        Some(ds) => {
            let doc = file_io::read_input(Path::new(ds))?;
            let text = String::from_utf8(doc.bytes).map_err(|_| p.bad(format!("{ds} is not UTF-8 text")))?;
            variations::avar_options_from_designspace(&text)?
        }
        None => variations::avar_options_from_font(&font)?,
    };
    let built = variations::build_avar(&font, &options)?;
    let bytes = fontcraft_tables::write(&built, &WriteOptions { order: TableOrder::Recommended })?;
    let out = file_io::output_path(&input.path, &req, ".avar", None)?;
    file_io::write_output(&out, &bytes, &req)?;
    let version = variations::avar_version(&built)?;
    let mut segments = Map::new();
    for (tag, pairs) in variations::segment_maps(&built)? {
        segments.insert(tag_str(tag).trim_end().to_string(), json!(pairs.iter().map(|(a, b)| [*a, *b]).collect::<Vec<_>>()));
    }
    let mut result = json!({ "output": out.display().to_string(), "avar": version.is_some(), "segments": segments });
    if let (Some(v), Some(obj)) = (version, result.as_object_mut()) {
        obj.insert("version".into(), json!(v));
    }
    Ok(result)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use fontcraft_tables::sfnt::Flavour;

    /// A TrueType font with only `head` and an `fvar` of wght 100–400–900 and wdth 75–100–125.
    fn variable_font() -> Vec<u8> {
        let mut fvar: Vec<u8> = Vec::new();
        for v in [1u16, 0, 16, 2, 2, 20, 0, 12] {
            fvar.extend_from_slice(&v.to_be_bytes());
        }
        for (tag, lo, de, hi) in [(b"wght", 100i32, 400i32, 900i32), (b"wdth", 75, 100, 125)] {
            fvar.extend_from_slice(tag);
            for v in [lo, de, hi] {
                fvar.extend_from_slice(&(v << 16).to_be_bytes());
            }
            fvar.extend_from_slice(&[0, 0, 1, 0]);
        }
        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(*b"head", head);
        font.insert(*b"fvar", fvar);
        fontcraft_tables::write(&font, &WriteOptions::default()).unwrap()
    }

    const DESIGNSPACE: &str = r#"<designspace format="5.0"><axes>
  <axis tag="wght" name="Weight" minimum="100" default="400" maximum="900">
    <map input="100" output="20"/><map input="400" output="80"/><map input="700" output="150"/><map input="900" output="200"/>
  </axis>
  <axis tag="wdth" name="Width" minimum="75" default="100" maximum="125"/>
</axes></designspace>"#;

    #[test]
    fn build_avar_then_map_coordinates_match_fonttools() {
        let dir = std::env::temp_dir().join(format!("fontcraft-variations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = dir.join("V.ttf");
        let ds = dir.join("V.designspace");
        std::fs::write(&font, variable_font()).unwrap();
        std::fs::write(&ds, DESIGNSPACE).unwrap();
        let before = std::fs::read(&font).unwrap();
        let mut s = Session::new();

        let r = s.execute("variations.buildAvar", &json!({ "font": font, "designspace": ds })).unwrap();
        let out = dir.join("V.avar.ttf");
        assert_eq!(r["output"], json!(out.display().to_string()), "the default output name");
        assert_eq!(r["version"], json!(1));
        assert_eq!(r["segments"]["wght"][2], json!([9830.0 / 16384.0, 9557.0 / 16384.0]), "fontTools writes 0.6 → 0.58331");
        assert_eq!(std::fs::read(&font).unwrap(), before, "the input is not changed");

        let m = s.execute("variations.mapCoordinates", &json!({ "font": out, "location": { "wght": 700 } })).unwrap();
        assert_eq!(m["to"], json!("normalized"));
        assert!((m["location"]["wght"].as_f64().unwrap() - 0.5833384192859322).abs() < 1e-12);
        assert!(m["location"].get("wdth").is_none(), "an axis not given is not listed");
        let m = s
            .execute("variations.mapCoordinates", &json!({ "font": out, "location": { "wght": 0.5 }, "from": "normalized", "allAxes": true }))
            .unwrap();
        assert_eq!(m["to"], json!("user"));
        assert_eq!(m["location"]["wdth"], json!(100.0));
        let expected = 400.0 + 0.5 * (9557.0 / 16384.0) / (9830.0 / 16384.0) * 500.0;
        assert!((m["location"]["wght"].as_f64().unwrap() - expected).abs() < 1e-9, "user space is the mapped value denormalised");

        let refused = s.execute("variations.buildAvar", &json!({ "font": font, "designspace": ds, "noClobber": true }));
        assert!(refused.is_err(), "the output exists and noClobber is set");
        assert!(s.execute("variations.mapCoordinates", &json!({ "font": out, "location": { "opsz": 12 } })).is_err());
        assert!(s.execute("variations.mapCoordinates", &json!({ "font": ds, "location": {} })).is_err(), "a designspace is not a compiled font");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
