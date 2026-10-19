//! `font.diff` (M12.32; `fontcraft_tables::diff`): two compiled fonts, or two folders of fonts,
//! compared table by table.
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web. This one is a query: it writes nothing unless
//! `outputDir` asks for the compared texts, and then writes all of them or none.
//!
//! The result of one pair (the Table Viewer's Compare With… reads it):
//! `{tables: {NAME: {tag, a, b, equal, identical, status, sizeA?, sizeB?, firstOffset?,
//! similarity?, oversized?, lines?, truncated?}}, identical, differing: [NAME], files?: [path]}`,
//! where NAME is the tag without trailing spaces or a pseudo-table's name (`GPOS=normalized`),
//! `equal` is byte equality and `identical` the verdict after the chosen normalisation. With a
//! folder: `{pairs: [{name, a, b, …the result of the pair}], onlyA: [name], onlyB: [name],
//! identical}`.

use std::path::{Path, PathBuf};

use fontcraft_tables::diff::{DiffMode, DiffOptions, DiffStatus, TableDiff};
use fontcraft_tables::sfnt::{SfntFont, parse_tag};
use serde_json::{Map, Value, json};

use super::file_io::read_input;
use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most font files paired when A or B is a folder.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_FOLDER_FILES: usize = 10_000;
/// Most characters of a PostScript name given as `faceA` or `faceB`.
const MAX_FACE_NAME: usize = 256;

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "font.diff",
        "Compare Fonts",
        [],
        None,
        "{a: path, b: path, faceA?: int|name, faceB?: int|name, detail?: \"tables\"|\"offset\"|\"hex\"|\"fields\"|\"verbose\" (default fields), tables?: [tag], excludeTables?: [tag], compareIdentical?: bool, context?: int (default 3), normalize?: \"none\"|\"dump\"|\"compiler\", hints?: bool, version?: bool, unwrapExtensions?: bool, offByOneBudget?: number (percent, default 0.1), sizeThreshold?: number (percent, default 10), similarity?: bool, all?: bool, outputDir?: path} two compiled fonts compared table by table: presence on each side, byte equality, and per detail the first differing offset, a unified diff of hex dumps or of the decoded tables (verbose: with every line); normalize dump compares the normalised dumps, compiler hides what two compilers may do differently without changing behaviour and adds the pseudo-tables GPOS=normalized and GDEF=normalized; hints and version false leave hinting and version data out of a normalised comparison; similarity leaves the diff lines out and keeps the ratio; all lifts the limit of 1000 lines (64 KiB of hex) per table; outputDir writes the compared texts as A.TABLE.txt and B.TABLE.txt; a and b may be two folders, or a font and a folder, paired by file name → {tables: {name: {tag, a, b, equal, identical, status, sizeA?, sizeB?, firstOffset?, similarity?, oversized?, lines?, truncated?}}, identical, differing: [name], files?: [path]} | with a folder {pairs: [{name, a, b, tables, identical, differing}], onlyA: [name], onlyB: [name], identical}",
        always,
        diff
    )
    .files(const { &[FileParam::read("a"), FileParam::read("b"), FileParam::write("outputDir")] })
    .writes_files()]
}

/// The parsed parameters.
struct Request {
    options: DiffOptions,
    face_a: Option<Value>,
    face_b: Option<Value>,
    lines: bool,
    output_dir: Option<PathBuf>,
}

fn diff(_: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("font.diff", p)?;
    let a = PathBuf::from(p.str("a")?);
    let b = PathBuf::from(p.str("b")?);
    let request = request(&p)?;
    match (a.is_dir(), b.is_dir()) {
        (true, true) => folders(&p, &a, &b, &request),
        (false, true) => pair(&p, &a, &in_folder(&b, &a)?, &request),
        (true, false) => pair(&p, &in_folder(&a, &b)?, &b, &request),
        (false, false) => pair(&p, &a, &b, &request),
    }
}

fn request(p: &Params<'_>) -> Result<Request> {
    if p.get("tool").is_some() {
        return Err(p.bad("`tool` is not supported: write the texts with `outputDir` and run the program on them"));
    }
    let detail = p.opt_str("detail")?.unwrap_or("fields");
    let normalize = p.opt_str("normalize")?.unwrap_or("none");
    let mode = match (detail, normalize) {
        (_, n) if !matches!(n, "none" | "dump" | "compiler") => {
            return Err(p.bad(format!("`normalize` must be \"none\", \"dump\" or \"compiler\", got \"{n}\"")));
        }
        ("offset" | "hex", "dump" | "compiler") => {
            return Err(p.bad(format!("`detail: {detail}` compares bytes: it cannot be combined with `normalize: {normalize}`")));
        }
        ("tables" | "fields" | "verbose", "compiler") => DiffMode::NormalizeCompiler,
        ("tables" | "fields" | "verbose", "dump") => DiffMode::NormalizeDump,
        ("tables", _) => DiffMode::Tables,
        ("offset", _) => DiffMode::Bytes,
        ("hex", _) => DiffMode::Hex,
        ("fields" | "verbose", _) => DiffMode::Text,
        (other, _) => {
            return Err(p.bad(format!("`detail` must be \"tables\", \"offset\", \"hex\", \"fields\" or \"verbose\", got \"{other}\"")));
        }
    };
    let tags = |key: &str| -> Result<Vec<[u8; 4]>> {
        let names = p.opt_strings(key)?.unwrap_or_default();
        if names.len() > fontcraft_tables::limits::MAX_TABLES {
            return Err(p.bad(format!("`{key}` names more than {} tables", fontcraft_tables::limits::MAX_TABLES)));
        }
        names.into_iter().map(|t| parse_tag(t).map_err(EngineError::from)).collect()
    };
    let tables = tags("tables")?;
    let skip = tags("excludeTables")?;
    if !tables.is_empty() && !skip.is_empty() {
        return Err(p.bad("give `tables` or `excludeTables`, not both"));
    }
    let context = match detail {
        "verbose" => usize::MAX,
        _ => p.opt_int("context", 0, 1_000_000)?.and_then(|n| usize::try_from(n).ok()).unwrap_or(3),
    };
    let percent = |key: &str, default: f64, max: f64| -> Result<f64> {
        let v = p.opt_f64(key)?.unwrap_or(default);
        if !(0.0..=max).contains(&v) {
            return Err(p.bad(format!("`{key}` must be a percentage from 0 to {max}, got {v}")));
        }
        Ok(v)
    };
    let output_dir = p.opt_str("outputDir")?.map(PathBuf::from);
    let options = DiffOptions {
        mode,
        tables,
        skip,
        all: p.bool_or("all", false)?,
        context,
        compare_identical: p.bool_or("compareIdentical", false)?,
        no_hints: !p.bool_or("hints", true)?,
        no_version: !p.bool_or("version", true)?,
        unwrap_extensions: match p.get("unwrapExtensions") {
            None => None,
            Some(_) => Some(p.bool_or("unwrapExtensions", false)?),
        },
        off_by_one_budget: percent("offByOneBudget", 0.1, 100.0)?,
        size_threshold: percent("sizeThreshold", 10.0, 1.0e9)?,
        keep_texts: output_dir.is_some(),
    };
    Ok(Request {
        options,
        face_a: p.get("faceA").cloned(),
        face_b: p.get("faceB").cloned(),
        lines: detail != "tables" && !p.bool_or("similarity", false)?,
        output_dir,
    })
}

/// `folder`'s file with the name of `file`.
fn in_folder(folder: &Path, file: &Path) -> Result<PathBuf> {
    let name = file.file_name().ok_or_else(|| EngineError::Io(format!("{} names no file", file.display())))?;
    Ok(folder.join(name))
}

/// One font of a pair, by face number or PostScript name.
fn open(p: &Params<'_>, path: &Path, face: Option<&Value>, key: &str) -> Result<SfntFont> {
    let input = read_input(path)?;
    if !input.kind.is_font() {
        return Err(EngineError::Io(format!("{} is not a compiled font ({})", path.display(), input.kind.name())));
    }
    let face = match face {
        None => 0,
        Some(Value::String(name)) => face_by_name(p, &input.bytes, name, key)?,
        Some(v) => {
            v.as_u64().and_then(|f| u32::try_from(f).ok()).ok_or_else(|| p.bad(format!("`{key}` must be a face number or a PostScript name")))?
        }
    };
    Ok(fontcraft_tables::read(&input.bytes, face)?)
}

/// The face of a collection whose PostScript name (name ID 6) is `name`.
fn face_by_name(p: &Params<'_>, bytes: &[u8], name: &str, key: &str) -> Result<u32> {
    if let Ok(index) = name.parse::<u32>() {
        return Ok(index);
    }
    if name.len() > MAX_FACE_NAME {
        return Err(p.bad(format!("`{key}` is longer than {MAX_FACE_NAME} characters")));
    }
    use skrifa::raw::{FontData, FontRead};
    let count = fontcraft_tables::sfnt::face_count(bytes)?;
    for face in 0..count {
        let font = fontcraft_tables::read(bytes, face)?;
        let Some(table) = font.get(*b"name") else { continue };
        let Ok(names) = skrifa::raw::tables::name::Name::read(FontData::new(table)) else { continue };
        let data = names.string_data();
        if names.name_record().iter().any(|r| r.name_id().to_u16() == 6 && r.string(data).is_ok_and(|s| s.chars().eq(name.chars()))) {
            return Ok(face);
        }
    }
    Err(p.bad(format!("no face of the file has the PostScript name \"{name}\"")))
}

/// Compares the fonts at `a` and `b`; writes the texts when asked.
fn pair(p: &Params<'_>, a: &Path, b: &Path, request: &Request) -> Result<Value> {
    let (mut result, texts) = compare(p, a, b, request, "")?;
    if let Some(dir) = &request.output_dir {
        let files = write_texts(dir, &texts)?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("files".into(), json!(files.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>()));
        }
    }
    Ok(result)
}

/// The result of one pair, and the texts to write (file name, text), each name prefixed with
/// `prefix`.
fn compare(p: &Params<'_>, a: &Path, b: &Path, request: &Request, prefix: &str) -> Result<(Value, Vec<(String, String)>)> {
    let fa = open(p, a, request.face_a.as_ref(), "faceA")?;
    let fb = open(p, b, request.face_b.as_ref(), "faceB")?;
    let diffs = fontcraft_tables::diff::diff(&fa, &fb, &request.options)?;
    let mut tables = Map::new();
    let mut texts = Vec::new();
    for d in &diffs {
        tables.insert(d.name.clone(), table_json(d, request.lines));
        if let Some((ta, tb)) = &d.texts {
            let file = d.name.replace(['/', '='], ".").replace(' ', "_");
            texts.push((format!("{prefix}A.{file}.txt"), ta.clone()));
            texts.push((format!("{prefix}B.{file}.txt"), tb.clone()));
        }
    }
    let differing: Vec<&str> = diffs.iter().filter(|d| d.status != DiffStatus::Same).map(|d| d.name.as_str()).collect();
    let result = json!({
        "tables": tables,
        "identical": differing.is_empty(),
        "differing": differing,
    });
    Ok((result, texts))
}

fn table_json(d: &TableDiff, with_lines: bool) -> Value {
    let status = match d.status {
        DiffStatus::Same => "same",
        DiffStatus::Different => "different",
        DiffStatus::OnlyFirst => "onlyA",
        DiffStatus::OnlySecond => "onlyB",
    };
    let pseudo = d.size_a.is_none() && d.size_b.is_none();
    let mut o = Map::new();
    o.insert("tag".into(), json!(fontcraft_tables::sfnt::tag_str(d.tag)));
    o.insert("a".into(), json!(d.status != DiffStatus::OnlySecond && (pseudo || d.size_a.is_some())));
    o.insert("b".into(), json!(d.status != DiffStatus::OnlyFirst && (pseudo || d.size_b.is_some())));
    o.insert("equal".into(), json!(d.bytes_equal));
    o.insert("identical".into(), json!(d.status == DiffStatus::Same));
    o.insert("status".into(), json!(status));
    if let Some(s) = d.size_a {
        o.insert("sizeA".into(), json!(s));
    }
    if let Some(s) = d.size_b {
        o.insert("sizeB".into(), json!(s));
    }
    if let Some(offset) = d.first_offset {
        o.insert("firstOffset".into(), json!(offset));
    }
    if let Some(ratio) = d.similarity {
        o.insert("similarity".into(), json!(ratio));
    }
    if d.oversized {
        o.insert("oversized".into(), json!(true));
    }
    if with_lines && !d.lines.is_empty() {
        o.insert("lines".into(), json!(d.lines));
        o.insert("truncated".into(), json!(d.truncated));
    }
    Value::Object(o)
}

/// Every font file name found in both folders compared, the others listed.
fn folders(p: &Params<'_>, a: &Path, b: &Path, request: &Request) -> Result<Value> {
    let names_a = font_names(a)?;
    let names_b = font_names(b)?;
    let mut pairs = Vec::new();
    let mut texts = Vec::new();
    let mut all_identical = true;
    for name in names_a.iter().filter(|n| names_b.contains(*n)) {
        let (pa, pb) = (a.join(name), b.join(name));
        let (mut result, t) = compare(p, &pa, &pb, request, &format!("{name}."))?;
        all_identical &= result.get("identical").and_then(Value::as_bool).unwrap_or(false);
        if let Some(obj) = result.as_object_mut() {
            obj.insert("name".into(), json!(name));
            obj.insert("a".into(), json!(pa.to_string_lossy()));
            obj.insert("b".into(), json!(pb.to_string_lossy()));
        }
        pairs.push(result);
        texts.extend(t);
    }
    let only_a: Vec<&String> = names_a.iter().filter(|n| !names_b.contains(*n)).collect();
    let only_b: Vec<&String> = names_b.iter().filter(|n| !names_a.contains(*n)).collect();
    all_identical &= only_a.is_empty() && only_b.is_empty();
    let mut result = json!({ "pairs": pairs, "onlyA": only_a, "onlyB": only_b, "identical": all_identical });
    if let Some(dir) = &request.output_dir {
        let files = write_texts(dir, &texts)?;
        if let Some(obj) = result.as_object_mut() {
            obj.insert("files".into(), json!(files.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>()));
        }
    }
    Ok(result)
}

/// The names of the regular files in `dir` whose content is a compiled font, sorted.
#[cfg(not(target_arch = "wasm32"))]
fn font_names(dir: &Path) -> Result<std::collections::BTreeSet<String>> {
    use std::io::Read as _;
    let entries = std::fs::read_dir(dir).map_err(|e| EngineError::Io(format!("cannot read the folder {}: {e}", dir.display())))?;
    let mut names = std::collections::BTreeSet::new();
    for (seen, entry) in entries.flatten().enumerate() {
        if seen >= MAX_FOLDER_FILES {
            return Err(EngineError::Io(format!("{} holds more than {MAX_FOLDER_FILES} entries", dir.display())));
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let mut head = Vec::new();
        if std::fs::File::open(&path).and_then(|f| f.take(1024).read_to_end(&mut head)).is_err() {
            continue;
        }
        if super::file_io::detect(&head).is_font() {
            names.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

#[cfg(target_arch = "wasm32")]
fn font_names(dir: &Path) -> Result<std::collections::BTreeSet<String>> {
    Err(EngineError::Io(format!("cannot read the folder {}: there are no files on the web", dir.display())))
}

/// Writes every text into `dir`, all or none: on a failure the files written and the folders
/// created are removed again.
#[cfg(not(target_arch = "wasm32"))]
fn write_texts(dir: &Path, texts: &[(String, String)]) -> Result<Vec<PathBuf>> {
    let request = super::file_io::OutputRequest { output_dir: Some(dir.to_path_buf()), ..Default::default() };
    let created = super::file_io::create_output_dir(&request)?;
    let mut written = Vec::with_capacity(texts.len());
    for (name, text) in texts {
        let path = dir.join(name);
        let result =
            super::file_io::check_write(&path).and_then(|()| fontcraft_format::write_atomic(&path, text.as_bytes()).map_err(EngineError::from));
        if let Err(e) = result {
            for f in &written {
                let _ = std::fs::remove_file(f);
            }
            super::file_io::remove_created(&created);
            return Err(e);
        }
        written.push(path);
    }
    Ok(written)
}

#[cfg(target_arch = "wasm32")]
fn write_texts(dir: &Path, _: &[(String, String)]) -> Result<Vec<PathBuf>> {
    Err(EngineError::Io(format!("cannot write into {}: there are no files on the web", dir.display())))
}
