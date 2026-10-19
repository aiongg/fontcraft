//! `font.convert` (M12.30; `fontcraft_tables::convert`).
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.
//!
//! Compiled fonts (TrueType, OpenType CFF or CFF2, one face of a collection, WOFF, WOFF2) are
//! converted between outline flavours and to bare CFF or CFF2 tables, and a result can be
//! wrapped in WOFF or WOFF2. Each output is written to a temporary file beside its target as
//! soon as its input is converted, so memory holds one font at a time; only when all of them
//! are there are they moved into place, so a failing conversion writes nothing.

use std::path::{Path, PathBuf};

use fontcraft_tables::convert::{ConvertOptions, ConvertTarget, PostFormat, QuadToCubic, convert_with};
use serde_json::{Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::{CommandSpec, FileParam, always, cmd};
use crate::params::Params;
use crate::{EngineError, Result, Session};

/// Most inputs one call converts.
const MAX_INPUTS: usize = 100_000;
/// Seconds between 1904-01-01 (the `head` epoch) and 1970-01-01.
const EPOCH_1904: i64 = 2_082_844_800;

/// Parameters of the row this version does not act on yet; naming one is refused, so a call
/// never silently does less than it asks.
const NOT_YET: [&str; 19] = [
    "removeOverlaps",
    "ignoreOverlapErrors",
    "type1",
    "newline",
    "cid",
    "glyphs",
    "excludeGlyphs",
    "fdSelect",
    "excludeFd",
    "goadb",
    "location",
    "clamp",
    "ufo",
    "svg",
    "recalcBounds",
    "rebuildTables",
    "optimizeFor",
    "validateSources",
    "smartComponents",
];

/// The commands of this module.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.convert",
            "Convert Font Format",
            [],
            None,
            "{inputs: [path] | input: path, to: \"ttf\"|\"otf\"|\"otf-cff2\"|\"cff\"|\"cff2\", output?: path (one input), outputDir?: path, inPlace?: bool, noClobber?: bool, face?: int|name, container?: \"plain\"|\"woff\"|\"woff2\", nameBy?: \"filename\", curves?: {maxError?: em fraction (default one font unit), conversion?: \"optimized\"|\"exact\", keepDirection?: bool}, postFormat?: 2|3, cff?: {subroutinize?: bool, maxSubrs?: int}, hints?: bool, timestamp?: \"keep\"|\"now\"|int (seconds since 1970)}. Compiled fonts only: CFF or static CFF2 to TrueType (cubic to quadratic within maxError, contours reversed), TrueType to CFF or CFF2 (components decomposed, instructions dropped), CFF to CFF2 and back with hints kept, or the bare outline table (cff, cff2). Without output the result goes beside the input with the new extension → {results: [{input, output, bytes, glyphs, leftBehind: [text]}]}",
            always,
            convert_fonts
        )
        .files(const { &[FileParam::read("input"), FileParam::read("inputs"), FileParam::write("output"), FileParam::write("outputDir")] })
        .writes_files(),
    ]
}

/// How a result is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Plain,
    Woff,
    Woff2,
}

/// One converted input.
struct Converted {
    bytes: Vec<u8>,
    glyphs: usize,
    left_behind: Vec<String>,
}

fn convert_fonts(_: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.convert", v)?;
    for key in NOT_YET {
        if p.get(key).is_some() {
            return Err(EngineError::Unsupported(format!("`{key}` in font.convert")));
        }
    }
    if let Some(name_by) = p.opt_str("nameBy")?
        && name_by != "filename"
    {
        return Err(EngineError::Unsupported(format!("nameBy `{name_by}`: outputs are named after their input files")));
    }
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
    let to = p.str("to")?;
    let container = match p.opt_str("container")?.unwrap_or("plain") {
        "plain" => Container::Plain,
        "woff" => Container::Woff,
        "woff2" => Container::Woff2,
        other => return Err(p.bad(format!("`container` must be plain, woff or woff2, not `{other}`"))),
    };
    let request = OutputRequest::from_params(&p)?;
    let settings = Settings::read(&p, to)?;
    if container != Container::Plain && matches!(to, "cff" | "cff2") {
        return Err(p.bad("a bare CFF or CFF2 table cannot be wrapped in WOFF"));
    }

    let mut outputs = Vec::with_capacity(inputs.len());
    for input in &inputs {
        outputs.push(file_io::output_path(Path::new(input), &request, "", Some(settings.extension(container))).map_err(|e| in_input(e, input))?);
    }
    // Two inputs written to one file, or an output that replaces another input, would lose a
    // result: refused before anything is written.
    let resolved = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let sources: Vec<PathBuf> = inputs.iter().map(|i| resolved(Path::new(i))).collect();
    let mut targets: Vec<PathBuf> = Vec::with_capacity(outputs.len());
    for (input, output) in inputs.iter().zip(&outputs) {
        let target = resolved(output);
        if targets.contains(&target) {
            return Err(p.bad(format!("two inputs would both be written to {}", output.display())));
        }
        if sources.contains(&target) && resolved(Path::new(input)) != target {
            return Err(p.bad(format!("the output {} would replace another input", output.display())));
        }
        targets.push(target);
    }
    for output in &outputs {
        file_io::check_write(output)?;
        if request.no_clobber && !request.in_place && output.exists() {
            return Err(EngineError::Io(format!("{} exists and noClobber is set", output.display())));
        }
    }
    let mut staging = Staging::open(&request)?;
    let mut results = Vec::with_capacity(inputs.len());
    for (n, (input, output)) in inputs.iter().zip(&outputs).enumerate() {
        let c = convert_one(&p, Path::new(input), &settings, container).map_err(|e| in_input(e, input))?;
        staging.stage(output, &c.bytes, n)?;
        results.push(json!({
            "input": input,
            "output": output.display().to_string(),
            "bytes": c.bytes.len(),
            "glyphs": c.glyphs,
            "leftBehind": c.left_behind,
        }));
    }
    staging.commit()?;
    Ok(json!({ "results": results }))
}

fn in_input(e: EngineError, input: &str) -> EngineError {
    match e {
        EngineError::Io(m) => EngineError::Io(m),
        EngineError::Unsupported(m) => EngineError::Unsupported(format!("{input}: {m}")),
        other => other,
    }
}

/// The conversion parameters, read once for every input.
struct Settings {
    to: &'static str,
    /// `None`: one font unit, the reference converters' bound.
    max_error_em: Option<f64>,
    quad_to_cubic: QuadToCubic,
    reverse_direction: bool,
    post_format: PostFormat,
    subroutinize: Option<bool>,
    max_subrs: Option<usize>,
    hints: bool,
    timestamp: Timestamp,
}

enum Timestamp {
    Keep,
    Now,
    At(i64),
}

impl Settings {
    fn read(p: &Params<'_>, to: &str) -> Result<Self> {
        let to: &'static str = match to {
            "ttf" => "ttf",
            "otf" => "otf",
            "otf-cff2" => "otf-cff2",
            "cff" => "cff",
            "cff2" => "cff2",
            "ttx" => return Err(EngineError::Unsupported("`to: ttx`: write TTX with font.dumpTables".into())),
            "pfa" | "pfb" | "t1-text" => return Err(EngineError::Unsupported(format!("`to: {to}`: Type 1 fonts are read, not written"))),
            "ufo" | "ufoz" | "designspace" | "glyphs" | "native" => {
                return Err(EngineError::Unsupported(format!("`to: {to}` from a compiled font: open it (file.open) and save it with file.saveAs")));
            }
            "svg-font" => return Err(EngineError::Unsupported("`to: svg-font`".into())),
            other => return Err(p.bad(format!("`to` must be ttf, otf, otf-cff2, cff or cff2, not `{other}`"))),
        };
        let curves = p.opt_object("curves")?;
        let curve = |key: &str| curves.and_then(|c| c.get(key));
        for key in curves.map(|c| c.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            if !matches!(key.as_str(), "maxError" | "conversion" | "keepDirection") {
                return Err(p.bad(format!("`curves.{key}` is not a curve setting")));
            }
        }
        let max_error_em = match curve("maxError") {
            None => None,
            Some(v) => Some(
                v.as_f64()
                    .filter(|e| e.is_finite() && *e > 0.0 && *e <= 1.0)
                    .ok_or_else(|| p.bad("`curves.maxError` must be above 0 and at most 1 (em)"))?,
            ),
        };
        let quad_to_cubic = match curve("conversion").map(|v| v.as_str()) {
            None | Some(Some("optimized")) => QuadToCubic::Optimized,
            Some(Some("exact")) => QuadToCubic::Exact,
            _ => return Err(p.bad("`curves.conversion` must be optimized or exact")),
        };
        let keep_direction = match curve("keepDirection") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| p.bad("`curves.keepDirection` must be true or false"))?,
        };
        let post_format = match p.opt_int("postFormat", 2, 3)? {
            None | Some(2) => PostFormat::Names,
            Some(_) => PostFormat::NoNames,
        };
        let cff = p.opt_object("cff")?;
        let mut subroutinize = None;
        let mut max_subrs = None;
        if let Some(cff) = cff {
            for (key, value) in cff {
                match key.as_str() {
                    "subroutinize" => subroutinize = Some(value.as_bool().ok_or_else(|| p.bad("`cff.subroutinize` must be true or false"))?),
                    "maxSubrs" => {
                        let n = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad("`cff.maxSubrs` must be a whole number"))?;
                        max_subrs = Some(n);
                    }
                    other => return Err(EngineError::Unsupported(format!("`cff.{other}` in font.convert"))),
                }
            }
        }
        let timestamp = match p.get("timestamp") {
            None => Timestamp::Keep,
            Some(Value::String(s)) if s == "keep" => Timestamp::Keep,
            Some(Value::String(s)) if s == "now" => Timestamp::Now,
            Some(Value::Number(n)) => Timestamp::At(n.as_i64().ok_or_else(|| p.bad("`timestamp` must be keep, now or whole seconds"))?),
            Some(_) => return Err(p.bad("`timestamp` must be keep, now or whole seconds since 1970")),
        };
        Ok(Settings {
            to,
            max_error_em,
            quad_to_cubic,
            reverse_direction: !keep_direction,
            post_format,
            subroutinize,
            max_subrs,
            hints: p.bool_or("hints", true)?,
            timestamp,
        })
    }

    fn options(&self, units_per_em: f64) -> Result<ConvertOptions> {
        let max_error = self.max_error_em.map_or(1.0, |e| e * units_per_em);
        let target = match self.to {
            "ttf" => ConvertTarget::TrueType { max_error },
            "otf" => ConvertTarget::Cff,
            "otf-cff2" => ConvertTarget::Cff2,
            "cff" => ConvertTarget::BareCff,
            _ => ConvertTarget::BareCff2,
        };
        let mut o = ConvertOptions::new(target);
        o.reverse_direction = self.reverse_direction;
        o.quad_to_cubic = self.quad_to_cubic;
        o.cubic_error = max_error;
        o.post_format = self.post_format;
        o.subroutinize = self.subroutinize;
        o.max_subrs = self.max_subrs;
        o.hints = self.hints;
        o.modified = match self.timestamp {
            Timestamp::Keep => None,
            Timestamp::At(unix) => Some(unix.saturating_add(EPOCH_1904)),
            Timestamp::Now => Some(now()?.saturating_add(EPOCH_1904)),
        };
        Ok(o)
    }

    fn extension(&self, container: Container) -> &'static str {
        match (container, self.to) {
            (Container::Woff, _) => "woff",
            (Container::Woff2, _) => "woff2",
            (_, "ttf") => "ttf",
            (_, "cff") => "cff",
            (_, "cff2") => "cff2",
            _ => "otf",
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> Result<i64> {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| EngineError::Io(format!("the clock: {e}")))?;
    i64::try_from(since.as_secs()).map_err(|_| EngineError::Io("the clock is out of range".into()))
}

#[cfg(target_arch = "wasm32")]
fn now() -> Result<i64> {
    Err(EngineError::Unsupported("the current time in the browser".into()))
}

fn convert_one(p: &Params<'_>, input: &Path, settings: &Settings, container: Container) -> Result<Converted> {
    let file = file_io::read_input(input)?;
    match file.kind {
        k if k.is_font() => {}
        InputKind::Ttx => return Err(EngineError::Unsupported("converting TTX: compile it with tables import first".into())),
        _ => {
            return Err(EngineError::Unsupported(
                "font.convert takes compiled fonts; a source is converted with file.saveAs or compiled with file.exportStatic".into(),
            ));
        }
    }
    let face = super::dehint::face_index(p, &file.bytes)?;
    let font = fontcraft_tables::read(&file.bytes, face)?;
    let upem =
        font.get(*b"head").and_then(|h| h.get(18..20)).and_then(|b| <[u8; 2]>::try_from(b).ok()).map_or(1000.0, |b| f64::from(u16::from_be_bytes(b)));
    let converted = convert_with(&font, &settings.options(upem)?)?;
    let bytes = match container {
        Container::Plain => converted.bytes,
        Container::Woff => fontcraft_container::woff::encode(&converted.bytes, &fontcraft_container::Extras::default())
            .map_err(|e| EngineError::Io(format!("WOFF: {e}")))?,
        Container::Woff2 => fontcraft_container::woff2::encode(&converted.bytes, &fontcraft_container::woff2::Options::default())
            .map_err(|e| EngineError::Io(format!("WOFF2: {e}")))?,
    };
    Ok(Converted { bytes, glyphs: converted.glyphs, left_behind: converted.left_behind })
}

/// The temporary files of one run. [`Staging::commit`] moves them into place; dropped before
/// that, the run removes them and the output folder it created.
struct Staging {
    /// Each output's temporary file, its target, and whether the target existed before.
    #[cfg(not(target_arch = "wasm32"))]
    staged: Vec<(PathBuf, PathBuf, bool)>,
    #[cfg(not(target_arch = "wasm32"))]
    created: Vec<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Staging {
    /// Creates the request's `outputDir` when it is missing.
    fn open(request: &OutputRequest) -> Result<Self> {
        Ok(Staging { staged: Vec::new(), created: file_io::create_output_dir(request)? })
    }

    /// Writes output `n` to its temporary file.
    fn stage(&mut self, out: &Path, bytes: &[u8], n: usize) -> Result<()> {
        let (temp, target) = stage(out, bytes, n)?;
        let existed = target.exists();
        self.staged.push((temp, target, existed));
        Ok(())
    }

    /// Moves every temporary file into place. When a move fails, the outputs already moved that
    /// replaced no file are removed again; a file already replaced keeps its new content, and
    /// the message says how many there are.
    fn commit(mut self) -> Result<()> {
        for (i, (temp, target, _)) in self.staged.iter().enumerate() {
            if let Err(e) = std::fs::rename(temp, target) {
                let mut replaced = 0;
                for (_, moved, existed) in self.staged.iter().take(i) {
                    if *existed {
                        replaced += 1;
                    } else {
                        let _ = std::fs::remove_file(moved);
                    }
                }
                let kept = if replaced == 0 {
                    "nothing was written".to_string()
                } else {
                    format!("{replaced} existing files before it were already replaced; the new files were removed")
                };
                return Err(EngineError::Io(format!("cannot write {}: {e}; {kept}", target.display())));
            }
        }
        self.staged.clear();
        self.created.clear();
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Staging {
    fn drop(&mut self) {
        for (temp, _, _) in &self.staged {
            let _ = std::fs::remove_file(temp);
        }
        file_io::remove_created(&self.created);
    }
}

#[cfg(target_arch = "wasm32")]
impl Staging {
    fn open(_: &OutputRequest) -> Result<Self> {
        Ok(Staging {})
    }

    fn stage(&mut self, _: &Path, _: &[u8], _: usize) -> Result<()> {
        Err(EngineError::Unsupported("writing files in the browser".into()))
    }

    fn commit(self) -> Result<()> {
        Ok(())
    }
}

/// Writes `bytes` to a new temporary file beside `out` (beside the file it links to, for a
/// link), with the permissions of the file it will replace.
#[cfg(not(target_arch = "wasm32"))]
fn stage(out: &Path, bytes: &[u8], n: usize) -> Result<(PathBuf, PathBuf)> {
    use std::io::Write as _;
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = if out.is_symlink() { std::fs::canonicalize(out).map_err(|e| io(out, e))? } else { out.to_path_buf() };
    if target.is_dir() {
        return Err(EngineError::Io(format!("cannot write {}: it is a folder", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io(&temp, e))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(io(&temp, e));
    }
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    Ok((temp, target))
}
