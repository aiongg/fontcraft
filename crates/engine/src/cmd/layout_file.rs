//! Layout commands on compiled fonts and feature files: `layout.normalize`, `.listFeatures`, `.decompileFeatures`, `.convertSyntax`, `.compileFeatures`, `.lintFeatures` (M12.11–M12.14).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = normalize_specs();
    v.extend(list_specs());
    v.extend(decompile_specs());
    v.extend(convert_specs());
    v.extend(compile_specs());
    v.extend(lint_specs());
    v
}

/// `layout.normalize` (M12.11).
fn normalize_specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layout.normalize",
        "Normalise Layout Rules",
        [],
        None,
        "{font: path, face?: number|name, table?: \"gpos\"|\"gsub\"|\"gdef\"|\"all\", location?: {axis: number}, output?: path, noClobber?: bool} the font's kerning, mark and cursive attachment and ligature carets as canonical sorted text, one flat rule per line, grouped by feature and the language systems that share its lookups; a variable value prints as the default and its value at each master location ({…}), or as the one value at `location` (user coordinates); GSUB prints no rules; with `output` the text is written there instead of returned → {text} | {output, bytes}",
        always,
        normalize
    )]
}

/// Most characters of a PostScript name given as `face`.
const MAX_FACE_NAME: usize = 256;

fn normalize(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layout.normalize", p)?;
    let path = std::path::PathBuf::from(p.str("font")?);
    let tables = match p.opt_str("table")?.map(str::to_ascii_lowercase).as_deref() {
        None | Some("all") => Vec::new(),
        Some("gpos") => vec![*b"GPOS"],
        Some("gsub") => vec![*b"GSUB"],
        Some("gdef") => vec![*b"GDEF"],
        Some(other) => return Err(p.bad(format!("`table` must be \"gpos\", \"gsub\", \"gdef\" or \"all\", got \"{other}\""))),
    };
    let mut location = Vec::new();
    if let Some(axes) = p.opt_object("location")? {
        if axes.len() > fontcraft_tables::limits::MAX_AXES {
            return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
        }
        for (axis, value) in axes {
            let tag = fontcraft_tables::sfnt::parse_tag(axis)?;
            let value = value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("`location.{axis}` must be a number")))?;
            location.push((tag, value));
        }
    }
    let output = p.opt_str("output")?.map(std::path::PathBuf::from);
    let no_clobber = p.bool_or("noClobber", false)?;
    if output.as_deref() == Some(path.as_path()) {
        return Err(p.bad("`output` is the input font: give another path"));
    }
    let input = super::file_io::read_input(&path)?;
    if !input.kind.is_font() {
        return Err(EngineError::Io(format!("{} is not a compiled font ({})", path.display(), input.kind.name())));
    }
    let face = match p.get("face") {
        None => 0,
        Some(Value::String(name)) => face_by_name(&p, &input.bytes, name)?,
        Some(v) => v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a face number or a PostScript name"))?,
    };
    let font = fontcraft_tables::read(&input.bytes, face)?;
    let options = fontcraft_tables::normalize::NormalizeOptions { tables, location };
    let text = fontcraft_tables::normalize::normalize(&font, &options)?;
    match output {
        None => Ok(json!({ "text": text })),
        Some(out) => {
            let request = super::file_io::OutputRequest { output: Some(out.clone()), no_clobber, ..Default::default() };
            super::file_io::write_output(&out, text.as_bytes(), &request)?;
            Ok(json!({ "output": out.to_string_lossy(), "bytes": text.len() }))
        }
    }
}

/// The face of a collection whose PostScript name (name ID 6) is `name`.
fn face_by_name(p: &Params<'_>, bytes: &[u8], name: &str) -> Result<u32> {
    if let Ok(index) = name.parse::<u32>() {
        return Ok(index);
    }
    if name.len() > MAX_FACE_NAME {
        return Err(p.bad(format!("`face` is longer than {MAX_FACE_NAME} characters")));
    }
    use skrifa::raw::{FontData, FontRead};
    let count = fontcraft_tables::sfnt::face_count(bytes)?;
    for face in 0..count {
        let font = fontcraft_tables::read(bytes, face)?;
        let Some(table) = font.get(*b"name") else { continue };
        let Ok(names) = skrifa::raw::tables::name::Name::read(FontData::new(table)) else { continue };
        let data = names.string_data();
        let found = names.name_record().iter().any(|r| r.name_id().to_u16() == 6 && r.string(data).is_ok_and(|s| s.chars().eq(name.chars())));
        if found {
            return Ok(face);
        }
    }
    Err(p.bad(format!("no face of the file has the PostScript name \"{name}\"")))
}

/// `layout.listFeatures` (M12.12).
fn list_specs() -> Vec<CommandSpec> {
    // M12.12
    Vec::new()
}

/// `layout.decompileFeatures` (M12.12).
fn decompile_specs() -> Vec<CommandSpec> {
    // M12.12
    Vec::new()
}

/// `layout.convertSyntax` (M12.13).
fn convert_specs() -> Vec<CommandSpec> {
    // M12.13
    Vec::new()
}

/// `layout.compileFeatures` (M12.14).
fn compile_specs() -> Vec<CommandSpec> {
    // M12.14
    Vec::new()
}

/// `layout.lintFeatures` (M12.14).
fn lint_specs() -> Vec<CommandSpec> {
    // M12.14
    Vec::new()
}
