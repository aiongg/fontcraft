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
    ).files(const { &[FileParam::read("font"), FileParam::write("output")] }).writes_files()]
}

/// Most characters of a face name given as `face`.
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
        Some(v) => v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad("`face` must be a face number or a face name"))?,
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

/// The face of a collection whose full name (name ID 4) or PostScript name (name ID 6) is
/// `name`; a number given as text is the face number.
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
        let found =
            names.name_record().iter().any(|r| matches!(r.name_id().to_u16(), 4 | 6) && r.string(data).is_ok_and(|s| s.chars().eq(name.chars())));
        if found {
            return Ok(face);
        }
    }
    Err(p.bad(format!("no face of the file has the full or PostScript name \"{name}\"")))
}

/// `layout.listFeatures` (M12.12).
fn list_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        query "layout.listFeatures",
        "List Layout Features",
        [],
        None,
        "{font: path, face?: number|name (a face of a collection by number, full name or PostScript name), table?: \"GSUB\"|\"GPOS\", lookups?: bool, by?: \"feature\"|\"script\"} the scripts, language systems and features of a compiled font's GSUB and GPOS; with lookups, the lookups each feature names per language system with their type (an extension lookup's type read through) → by feature (default): {features: [{table, feature, systems: [{script, language, lookups?: [{index, type, extension}]}]}]}; by script: {systems: [{table, script, language, features: [{feature, lookups?}]}]}",
        super::always,
        m12_12::list_features
    ).files(const { &[FileParam::read("font")] })]
}

/// `layout.decompileFeatures` (M12.12).
fn decompile_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        "layout.decompileFeatures",
        "Decompile Layout to Feature Code",
        [],
        None,
        "{font: path, face?: number|name, tables?: [\"GSUB\"|\"GPOS\"|\"GDEF\"|\"BASE\"] (default all the font has), features?: [tag] (only these features and the lookups they reach), showExtensions?: bool, names?: \"final\"|\"source\" (source: development names from goadb), goadb?: path, classes?: \"auto\"|\"inline\" (auto names glyph sets used more than once), output?: path, outputDir?: path, noClobber?: bool} the font's layout tables as feature code; without output or outputDir the text is returned; constructs without feature syntax are written as comments and listed → {text?, output?, unsupported: [string]}",
        super::always,
        m12_12::decompile_features
    ).files(const { &[FileParam::read("font"), FileParam::read("goadb"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files()]
}

/// The runners of M12.12.
mod m12_12 {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde_json::{Value, json};

    use super::super::file_io::{OutputRequest, output_path, read_input, write_output};
    use crate::params::Params;
    use crate::{Result, Session};
    use fontcraft_tables::fea_write::{DecompileOptions, decompile_report};
    use fontcraft_tables::features::{FeatureEntry, list_features as list, lookup_kinds};
    use fontcraft_tables::sfnt::{SfntFont, Tag, parse_tag, tag_str};

    fn tag_text(t: Tag) -> String {
        tag_str(t).trim_end().to_string()
    }

    /// The font at `font`, face `face`: a number (0 by default) or a face's full or PostScript
    /// name.
    fn open(p: &Params) -> Result<(String, SfntFont)> {
        let path = p.str("font")?;
        let input = read_input(Path::new(path))?;
        if !input.kind.is_font() {
            return Err(p.bad(format!("{path} is not a compiled font (it holds {})", input.kind.name())));
        }
        let face = match p.get("face") {
            Some(Value::String(name)) => super::face_by_name(p, &input.bytes, name)?,
            _ => p.opt_int("face", 0, i64::from(u32::MAX))?.and_then(|f| u32::try_from(f).ok()).unwrap_or(0),
        };
        let font = fontcraft_tables::read(&input.bytes, face)?;
        Ok((path.to_string(), font))
    }

    fn tags(p: &Params, key: &str, allowed: Option<&[&str]>) -> Result<Vec<Tag>> {
        let mut out = Vec::new();
        for s in p.opt_strings(key)?.unwrap_or_default() {
            if let Some(allowed) = allowed
                && !allowed.contains(&s)
            {
                return Err(p.bad(format!("`{key}` takes {}, got \"{s}\"", allowed.join(", "))));
            }
            out.push(parse_tag(s)?);
        }
        Ok(out)
    }

    pub(super) fn list_features(_: &mut Session, p: &Value) -> Result<Value> {
        let p = Params::new("layout.listFeatures", p)?;
        let table = match p.opt_str("table")? {
            None => None,
            Some("GSUB") => Some(*b"GSUB"),
            Some("GPOS") => Some(*b"GPOS"),
            Some(other) => return Err(p.bad(format!("`table` must be \"GSUB\" or \"GPOS\", got \"{other}\""))),
        };
        let with_lookups = p.bool_or("lookups", false)?;
        let by = p.opt_str("by")?.unwrap_or("feature");
        if !matches!(by, "feature" | "script") {
            return Err(p.bad(format!("`by` must be \"feature\" or \"script\", got \"{by}\"")));
        }
        let (_, font) = open(&p)?;
        let entries: Vec<FeatureEntry> = list(&font)?.into_iter().filter(|e| table.is_none_or(|t| e.table == t)).collect();
        let mut kinds = BTreeMap::new();
        if with_lookups {
            for t in [*b"GSUB", *b"GPOS"] {
                kinds.insert(t, lookup_kinds(&font, t)?);
            }
        }
        let lookups = |e: &FeatureEntry| -> Value {
            let k = kinds.get(&e.table);
            Value::Array(
                e.lookups
                    .iter()
                    .map(|i| {
                        let kind = k.and_then(|k| k.get(usize::from(*i)));
                        json!({ "index": i, "type": kind.map(|k| k.lookup_type), "extension": kind.map(|k| k.extension) })
                    })
                    .collect(),
            )
        };
        if by == "script" {
            let mut systems: Vec<Value> = Vec::new();
            let mut last: Option<(Tag, Tag, Tag)> = None;
            for e in &entries {
                let key = (e.table, e.script, e.language);
                if last != Some(key) {
                    systems
                        .push(json!({ "table": tag_text(e.table), "script": tag_text(e.script), "language": tag_text(e.language), "features": [] }));
                    last = Some(key);
                }
                let mut f = json!({ "feature": tag_text(e.feature) });
                if let (true, Some(o)) = (with_lookups, f.as_object_mut()) {
                    o.insert("lookups".into(), lookups(e));
                }
                if let Some(Value::Array(list)) = systems.last_mut().and_then(|s| s.get_mut("features")) {
                    list.push(f);
                }
            }
            return Ok(json!({ "systems": systems }));
        }
        let mut grouped: BTreeMap<(Tag, Tag), Vec<Value>> = BTreeMap::new();
        for e in &entries {
            let mut s = json!({ "script": tag_text(e.script), "language": tag_text(e.language) });
            if let (true, Some(o)) = (with_lookups, s.as_object_mut()) {
                o.insert("lookups".into(), lookups(e));
            }
            grouped.entry((e.table, e.feature)).or_default().push(s);
        }
        let features: Vec<Value> =
            grouped.into_iter().map(|((t, f), systems)| json!({ "table": tag_text(t), "feature": tag_text(f), "systems": systems })).collect();
        Ok(json!({ "features": features }))
    }

    pub(super) fn decompile_features(_: &mut Session, p: &Value) -> Result<Value> {
        let p = Params::new("layout.decompileFeatures", p)?;
        let mut options = DecompileOptions {
            tables: tags(&p, "tables", Some(&["GSUB", "GPOS", "GDEF", "BASE"]))?,
            features: tags(&p, "features", None)?,
            show_extensions: p.bool_or("showExtensions", false)?,
            ..DecompileOptions::default()
        };
        options.named_classes = match p.opt_str("classes")?.unwrap_or("auto") {
            "auto" => true,
            "inline" => false,
            other => return Err(p.bad(format!("`classes` must be \"auto\" or \"inline\", got \"{other}\""))),
        };
        match p.opt_str("names")?.unwrap_or("final") {
            "final" => {}
            "source" => {
                let goadb =
                    p.opt_str("goadb")?.ok_or_else(|| p.bad("`names: \"source\"` needs `goadb`, the file that maps final names to source names"))?;
                let bytes = read_input(Path::new(goadb))?.bytes;
                let text = String::from_utf8(bytes).map_err(|_| p.bad(format!("{goadb} is not UTF-8 text")))?;
                for entry in fontcraft_tables::goadb::parse(&text)?.entries {
                    options.rename.insert(entry.final_name, entry.source_name);
                }
            }
            other => return Err(p.bad(format!("`names` must be \"final\" or \"source\", got \"{other}\""))),
        }
        let req = OutputRequest::from_params(&p)?;
        if req.in_place {
            return Err(p.bad("`inPlace` would replace the font with feature code: give `output` or `outputDir`"));
        }
        let (path, font) = open(&p)?;
        let d = decompile_report(&font, &options)?;
        if req.output.is_none() && req.output_dir.is_none() {
            return Ok(json!({ "text": d.text, "unsupported": d.unsupported }));
        }
        let out = output_path(Path::new(&path), &req, "", Some("fea"))?;
        write_output(&out, d.text.as_bytes(), &req)?;
        Ok(json!({ "output": out, "unsupported": d.unsupported }))
    }
}

mod m12_13;

/// `layout.convertSyntax` (M12.13).
fn convert_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        "layout.convertSyntax",
        "Convert Layout Source Syntax",
        [],
        None,
        "{input: path | [path] (a VOLT project, a font holding one in its TSIV table, or FontDame layout text, one file per table), to?: \"fea\"|\"ttx\" (default fea), tables?: [\"GDEF\"|\"GSUB\"|\"GPOS\"] (default all; with one FontDame file whose first line names no table, a single entry says which table it is), font?: path (its glyph names by glyph id, for FontDame `# N` references and class 0; required for ttx), face?: number (of font), output?: path, noClobber?: bool} translates the layout source to feature code, or with ttx to the TTX text of the tables that code compiles to against the font; without output the text is returned; what feature code cannot hold is listed in warnings → {text, syntax: \"volt\"|\"fontdame\", warnings: [string]} | {output, syntax, warnings}",
        super::always,
        m12_13::convert_syntax
    ).files(const { &[FileParam::read("input"), FileParam::read("font"), FileParam::write("output")] }).writes_files()]
}

/// `layout.compileFeatures` (M12.14).
fn compile_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        "layout.compileFeatures",
        "Compile Features into Font",
        [],
        None,
        "{source: path | [path] (feature code; FontDame text, one file per table; a VOLT project, or a font holding one in its TSIV table), font: path, face?: number|name, syntax?: \"fea\"|\"mti\"|\"volt\" (default: told from the first line), tables?: [\"BASE\"|\"GDEF\"|\"GPOS\"|\"GSUB\"|\"OS/2\"|\"head\"|\"hhea\"|\"name\"|\"vhea\"|\"STAT\"] (default all: a layout table the code leaves empty is removed), includeDir?: path (default: the source's folder), goadb?: path (the code names glyphs by the source names this file maps to the font's), emitFea?: path (FontDame or VOLT input: also write the feature code it became), ship?: bool (remove the VOLT tables TSIV, TSIS, TSIP, TSID), to?: \"font\"|\"ttx\" (ttx: the built tables as TTX text), debugLookups?: bool (also write a Debg table: for each lookup the file, line and column of its first rule, its name and a script, language and feature that use it, as JSON in the reference compiler's form; a table not built records no lookups), output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} compiles the code against the font's glyph names and writes the font with the tables it builds, leaving the others as they are; a table block changes only the fields it sets; without output the file is the font's name with .features; code with errors writes nothing and fails with the errors and their places → {output, syntax, tables: [tag] (written), removed: [tag], features: [{tag, table: \"GSUB\"|\"GPOS\", languageSystems: [[script, language]], lookups: n}], diagnostics: [{severity, message, file, line, column}] (the compiler's warnings), warnings: [string], emitFea?: path}",
        super::always,
        m12_14::compile_features
    )
    .files(const {
        &[
            FileParam::read("source"),
            FileParam::read("font"),
            FileParam::read("includeDir"),
            FileParam::read("goadb"),
            FileParam::write("emitFea"),
            FileParam::write("output"),
            FileParam::write("outputDir"),
        ]
    })
    .writes_files()]
}

/// `layout.lintFeatures` (M12.14).
fn lint_specs() -> Vec<CommandSpec> {
    vec![super::cmd!(
        query "layout.lintFeatures",
        "Check Feature Code",
        [],
        None,
        "{source: path, font?: path, face?: number|name, glyphOrder?: path (a glyph list or GOADB: the first word of each line), glyphNames?: [name], includeDir?: path (default: the source's folder), warnings?: \"default\"|\"all\"} checks feature code as the compiler reads it and writes nothing; with a glyph set (font, glyphOrder or glyphNames) glyph names are checked and the tables are built in memory, so lookups too large for their offsets are found too; without one glyph names are not checked; all adds notes on usually harmless conditions (a glyph one attachment lookup uses both as a mark and as a base) → {errors: n, warnings: n, notes: n, glyphsChecked: bool, diagnostics: [{severity: \"error\"|\"warning\"|\"note\", message, file, line, column (from 1, in characters; null without a place)}]}",
        super::always,
        m12_14::lint_features
    )
    .files(const { &[FileParam::read("source"), FileParam::read("font"), FileParam::read("glyphOrder"), FileParam::read("includeDir")] })]
}

/// The runners of M12.14.
mod m12_14;
