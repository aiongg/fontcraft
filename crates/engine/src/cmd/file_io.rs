//! File plumbing for the whole-file commands of M12 (row `cli.global.sourceOrBinary`): reading
//! an input path, recognising its kind by content, deciding the output path and writing it
//! atomically.
//!
//! Whole-file commands take paths (`{font: path, output?: path, outputDir?: path, inPlace?: bool,
//! noClobber?: bool, …}`), never change their input, and leave no output behind when they fail:
//! the result is built in memory and written in one atomic step at the end. On the web there are
//! no paths: every function here refuses with a message.

use std::path::{Path, PathBuf};

use crate::params::Params;
use crate::{EngineError, Result};

/// Largest input file read, in bytes.
pub const MAX_INPUT: usize = fontcraft_tables::limits::MAX_SFNT;

/// What an input file holds, recognised by its first bytes, never by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// A TrueType-flavoured sfnt.
    TrueType,
    /// A CFF-flavoured sfnt (`OTTO`).
    Cff,
    /// A font collection (`ttcf`).
    Collection,
    Woff,
    Woff2,
    /// TTX text: XML whose root element is `ttFont`.
    Ttx,
    /// Anything else (a source, feature code, a GOADB…); the command decides.
    Other,
}

impl InputKind {
    /// A compiled font the `tables` crate reads.
    pub fn is_font(self) -> bool {
        matches!(self, InputKind::TrueType | InputKind::Cff | InputKind::Collection | InputKind::Woff | InputKind::Woff2)
    }

    /// The name results and messages use.
    pub fn name(self) -> &'static str {
        match self {
            InputKind::TrueType => "ttf",
            InputKind::Cff => "otf",
            InputKind::Collection => "collection",
            InputKind::Woff => "woff",
            InputKind::Woff2 => "woff2",
            InputKind::Ttx => "ttx",
            InputKind::Other => "other",
        }
    }
}

/// The kind of `bytes`.
pub fn detect(bytes: &[u8]) -> InputKind {
    use fontcraft_container::Kind;
    match fontcraft_container::sniff(bytes) {
        Some(Kind::TrueType) => return InputKind::TrueType,
        Some(Kind::Cff) => return InputKind::Cff,
        Some(Kind::Collection) => return InputKind::Collection,
        Some(Kind::Woff) => return InputKind::Woff,
        Some(Kind::Woff2) => return InputKind::Woff2,
        None => {}
    }
    // The root element comes after the XML declaration and comments, within the first few
    // hundred bytes of any file `ttx` writes.
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or_default();
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('<') && text.contains("<ttFont") { InputKind::Ttx } else { InputKind::Other }
}

/// An input file read into memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub kind: InputKind,
}

/// Reads a regular file of at most [`MAX_INPUT`] bytes and recognises its kind.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_input(path: &Path) -> Result<Input> {
    use std::io::Read as _;
    let meta = std::fs::metadata(path).map_err(|e| EngineError::Io(format!("cannot open {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let mut bytes = Vec::new();
    let limit = MAX_INPUT as u64;
    // At most one byte past the limit, whatever the file's size claims to be.
    std::fs::File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|e| EngineError::Io(format!("cannot read {}: {e}", path.display())))?;
    if bytes.len() as u64 > limit {
        return Err(EngineError::Io(format!("{} is larger than {MAX_INPUT} bytes", path.display())));
    }
    let kind = detect(&bytes);
    Ok(Input { path: path.to_path_buf(), bytes, kind })
}

#[cfg(target_arch = "wasm32")]
pub fn read_input(_: &Path) -> Result<Input> {
    Err(refused())
}

/// Where a whole-file command writes, from its parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputRequest {
    /// `output`: the exact output path.
    pub output: Option<PathBuf>,
    /// `outputDir`: the folder the derived name goes into.
    pub output_dir: Option<PathBuf>,
    /// `inPlace`: replace the input (atomically).
    pub in_place: bool,
    /// `noClobber`: refuse when the output exists.
    pub no_clobber: bool,
}

impl OutputRequest {
    /// Reads `output`, `outputDir`, `inPlace` and `noClobber`. `inPlace` with either of the
    /// others is refused.
    pub fn from_params(p: &Params<'_>) -> Result<Self> {
        let req = OutputRequest {
            output: p.opt_str("output")?.map(PathBuf::from),
            output_dir: p.opt_str("outputDir")?.map(PathBuf::from),
            in_place: p.bool_or("inPlace", false)?,
            no_clobber: p.bool_or("noClobber", false)?,
        };
        if req.in_place && (req.output.is_some() || req.output_dir.is_some()) {
            return Err(p.bad("`inPlace` cannot be combined with `output` or `outputDir`"));
        }
        if req.output.is_some() && req.output_dir.is_some() {
            return Err(p.bad("give `output` or `outputDir`, not both"));
        }
        Ok(req)
    }
}

/// The output path for `input`. Without `output` the name is the input's stem, then `suffix`
/// (`.subset`), then `extension` (or the input's own): `A.ttf` with `.subset` gives
/// `A.subset.ttf`, in `outputDir` or beside the input. A derived path equal to the input is
/// refused unless `inPlace` is set.
pub fn output_path(input: &Path, req: &OutputRequest, suffix: &str, extension: Option<&str>) -> Result<PathBuf> {
    if req.in_place {
        return Ok(input.to_path_buf());
    }
    if let Some(out) = &req.output {
        if out == input {
            return Err(EngineError::Io(format!("the output {} is the input: pass inPlace to replace it", out.display())));
        }
        return Ok(out.clone());
    }
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    let ext = extension.map(str::to_string).or_else(|| input.extension().map(|e| e.to_string_lossy().into_owned()));
    let name = match ext {
        Some(ext) if !ext.is_empty() => format!("{stem}{suffix}.{ext}"),
        _ => format!("{stem}{suffix}"),
    };
    let dir = req.output_dir.clone().or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
    let path = dir.join(name);
    if path == input {
        return Err(EngineError::Io(format!("the output {} would replace the input: give output, outputDir or inPlace", path.display())));
    }
    Ok(path)
}

/// Writes `bytes` to `path` atomically, creating `outputDir` when it is missing. With
/// `noClobber`, an existing file is refused and left alone.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_output(path: &Path, bytes: &[u8], req: &OutputRequest) -> Result<()> {
    if req.no_clobber && !req.in_place && path.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
    }
    if let Some(dir) = &req.output_dir
        && !req.in_place
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    fontcraft_format::write_atomic(path, bytes)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn write_output(_: &Path, _: &[u8], _: &OutputRequest) -> Result<()> {
    Err(refused())
}

#[cfg(target_arch = "wasm32")]
fn refused() -> EngineError {
    EngineError::Unsupported("reading and writing files by path in the browser".into())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn output_names_and_no_clobber() {
        let input = Path::new("/fonts/A.ttf");
        let plain = OutputRequest::default();
        assert_eq!(output_path(input, &plain, ".subset", None).unwrap(), PathBuf::from("/fonts/A.subset.ttf"));
        assert_eq!(output_path(input, &plain, "", Some("woff2")).unwrap(), PathBuf::from("/fonts/A.woff2"));
        let dir = OutputRequest { output_dir: Some("/out".into()), ..OutputRequest::default() };
        assert_eq!(output_path(input, &dir, "", None).unwrap(), PathBuf::from("/out/A.ttf"));
        assert!(output_path(input, &plain, "", None).is_err(), "a derived name equal to the input is refused");
        let in_place = OutputRequest { in_place: true, ..OutputRequest::default() };
        assert_eq!(output_path(input, &in_place, ".subset", None).unwrap(), input);

        let tmp = std::env::temp_dir().join(format!("fontcraft-file-io-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let target = tmp.join("B.ttf");
        std::fs::write(&target, b"old").unwrap();
        let keep = OutputRequest { no_clobber: true, ..OutputRequest::default() };
        assert!(write_output(&target, b"new", &keep).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old", "a refused write changes nothing");
        write_output(&target, b"new", &OutputRequest::default()).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn kinds_come_from_content() {
        assert_eq!(detect(b"OTTO\0\0"), InputKind::Cff);
        assert_eq!(detect(b"<?xml version=\"1.0\"?>\n<ttFont sfntVersion=\"OTTO\">"), InputKind::Ttx);
        assert_eq!(detect(b"languagesystem DFLT dflt;"), InputKind::Other);
    }
}
