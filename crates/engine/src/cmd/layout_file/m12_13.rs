//! `layout.convertSyntax` (M12.13): a VOLT project or FontDame layout text to feature code
//! (`fontcraft_tables::volt`, `fontcraft_tables::fontdame`), or to the TTX text of the tables
//! that code compiles to against a font's glyph names.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::super::file_io::{OutputRequest, read_input, write_output};
use crate::params::Params;
use crate::{EngineError, Result, Session};
use fontcraft_tables::fontdame::{self, FontDameOptions};
use fontcraft_tables::sfnt::{SfntFont, Tag, parse_tag};
use fontcraft_tables::volt::{self, VoltOptions};

/// Most compiler errors quoted when the converted code does not compile.
const MAX_QUOTED_ERRORS: usize = 10;

/// The layout tables the source syntaxes describe.
const LAYOUT_TABLES: [&str; 3] = ["GDEF", "GSUB", "GPOS"];

enum Source {
    /// A project and, when it came from a font, that font's glyph order.
    Volt(String, Option<Vec<String>>),
    FontDame(Vec<String>),
}

/// A font's tables and glyph names.
fn open_font(p: &Params, path: &str) -> Result<(SfntFont, Vec<String>)> {
    let face = p.opt_int("face", 0, i64::from(u32::MAX))?.and_then(|f| u32::try_from(f).ok()).unwrap_or(0);
    let input = read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return Err(p.bad(format!("{path} is not a compiled font (it holds {})", input.kind.name())));
    }
    let font = fontcraft_tables::read(&input.bytes, face)?;
    let order = fontcraft_tables::ttx::glyph_order(&font)?;
    Ok((font, order))
}

fn read_sources(p: &Params, inputs: &[String], output: Option<&Path>) -> Result<Source> {
    let mut volt_sources: Vec<(String, Option<Vec<String>>)> = Vec::new();
    let mut fontdame_texts = Vec::new();
    for path in inputs {
        if output == Some(Path::new(path)) {
            return Err(p.bad(format!("`output` is the input {path}: give another path")));
        }
        let input = read_input(Path::new(path))?;
        if input.kind.is_font() {
            let font = fontcraft_tables::read(&input.bytes, 0)?;
            let tsiv = font.get(*b"TSIV").ok_or_else(|| EngineError::Io(format!("{path} is a font without a VOLT project (no TSIV table)")))?;
            let text = std::str::from_utf8(tsiv).map_err(|_| EngineError::Io(format!("the TSIV table of {path} is not UTF-8 text")))?;
            let order = fontcraft_tables::ttx::glyph_order(&font)?;
            volt_sources.push((text.trim_end_matches('\0').to_string(), Some(order)));
            continue;
        }
        let text = String::from_utf8(input.bytes).map_err(|_| EngineError::Io(format!("{path} is not UTF-8 text")))?;
        if volt::looks_like_volt(&text) {
            volt_sources.push((text, None));
        } else if fontdame::looks_like_fontdame(&text) {
            fontdame_texts.push(text);
        } else {
            return Err(EngineError::Io(format!("{path} is neither a VOLT project nor FontDame layout text")));
        }
    }
    match (volt_sources.len(), fontdame_texts.is_empty()) {
        (0, _) => Ok(Source::FontDame(fontdame_texts)),
        (1, true) => {
            let (text, order) = volt_sources.into_iter().next().unwrap_or_default();
            Ok(Source::Volt(text, order))
        }
        _ => Err(p.bad("a VOLT project is converted on its own: give one project and no other input")),
    }
}

/// The feature code compiled against the font's glyph names, its layout tables put into the
/// font, and those tables as TTX text.
fn compile_to_ttx(fea: &str, mut font: SfntFont, order: Vec<String>, tables: &[Tag]) -> Result<String> {
    let files = fontcraft_fea::VirtualFiles::new("converted.fea", fea).map_err(|d| EngineError::Interaction(d.message))?;
    let glyph_order = fontcraft_fea::GlyphOrder::new(order).map_err(|d| EngineError::Interaction(d.message))?;
    let compiled = fontcraft_fea::compile(&fontcraft_fea::CompileRequest::new(&files, &glyph_order)).map_err(|diagnostics| {
        let errors: Vec<String> =
            diagnostics.iter().filter(|d| d.severity == fontcraft_fea::Severity::Error).take(MAX_QUOTED_ERRORS).map(|d| d.message.clone()).collect();
        EngineError::Interaction(format!("the converted feature code does not compile against the font: {}", errors.join("; ")))
    })?;
    let mut written = Vec::new();
    for (tag, bytes) in [(*b"GDEF", compiled.gdef), (*b"GSUB", compiled.gsub), (*b"GPOS", compiled.gpos)] {
        font.remove(tag);
        if let Some(bytes) = bytes
            && (tables.is_empty() || tables.contains(&tag))
        {
            font.insert(tag, bytes);
            written.push(tag);
        }
    }
    if written.is_empty() {
        return Ok(String::new());
    }
    let options = fontcraft_tables::ttx::DumpOptions { tables: written, ..Default::default() };
    Ok(fontcraft_tables::ttx::dump(&font, &options)?)
}

pub(super) fn convert_syntax(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layout.convertSyntax", p)?;
    let inputs: Vec<String> = match p.get("input") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(_)) => p.strings("input")?.into_iter().map(str::to_string).collect(),
        _ => return Err(p.bad("`input` must be a path or a list of paths")),
    };
    if inputs.is_empty() || inputs.len() > fontdame::MAX_FILES {
        return Err(p.bad(format!("`input` takes 1 to {} files", fontdame::MAX_FILES)));
    }
    let to = p.opt_str("to")?.unwrap_or("fea");
    if !matches!(to, "fea" | "ttx") {
        return Err(p.bad(format!("`to` must be \"fea\" or \"ttx\", got \"{to}\"")));
    }
    let mut tables: Vec<Tag> = Vec::new();
    for t in p.opt_strings("tables")?.unwrap_or_default() {
        if !LAYOUT_TABLES.contains(&t) {
            return Err(p.bad(format!("`tables` takes GDEF, GSUB and GPOS, got \"{t}\"")));
        }
        tables.push(parse_tag(t)?);
    }
    let output = p.opt_str("output")?.map(PathBuf::from);
    let no_clobber = p.bool_or("noClobber", false)?;
    let font = p.opt_str("font")?.map(|path| open_font(&p, path)).transpose()?;
    if to == "ttx" && font.is_none() {
        return Err(p.bad("`to: \"ttx\"` needs `font`: the tables are compiled against its glyph names"));
    }

    let (syntax, fea, warnings) = match read_sources(&p, &inputs, output.as_deref())? {
        Source::Volt(text, order) => {
            let (fea, warnings) = volt::convert(&text, &VoltOptions { tables: tables.clone(), glyph_order: order })?;
            ("volt", fea, warnings)
        }
        Source::FontDame(texts) => {
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            // One file and one table: the table of a file whose first line names none.
            let table = if refs.len() == 1 && tables.len() == 1 { tables.first().copied() } else { None };
            let glyph_order = font.as_ref().map(|(_, order)| order.clone()).unwrap_or_default();
            let (fea, warnings) = fontdame::convert(&refs, &FontDameOptions { table, tables: tables.clone(), glyph_order })?;
            ("fontdame", fea, warnings)
        }
    };
    let text = match (to, font) {
        ("ttx", Some((font, order))) => compile_to_ttx(&fea, font, order, &tables)?,
        _ => fea,
    };
    match output {
        None => Ok(json!({ "text": text, "syntax": syntax, "warnings": warnings })),
        Some(out) => {
            let request = OutputRequest { output: Some(out.clone()), no_clobber, ..Default::default() };
            write_output(&out, text.as_bytes(), &request)?;
            Ok(json!({ "output": out.to_string_lossy(), "syntax": syntax, "warnings": warnings }))
        }
    }
}
