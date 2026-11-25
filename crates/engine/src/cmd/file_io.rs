//! File plumbing for the whole-file commands of M12 (row `cli.global.sourceOrBinary`): reading
//! an input path, recognising its kind by content, deciding the output path and writing it
//! atomically.
//!
//! Whole-file commands take paths (`{font: path, output?: path, outputDir?: path, inPlace?: bool,
//! noClobber?: bool, …}`), never change their input, and leave no output behind when they fail:
//! the result is built in memory and written in one atomic step at the end. On the web there are
//! no paths: every function here refuses with a message.

use std::path::{Component, Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use fontcraft_format::output;

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
pub fn read_input(path: &Path) -> Result<Input> {
    let input = read_input_up_to(path, MAX_INPUT)?;
    if input.bytes.len() > MAX_INPUT {
        return Err(EngineError::Io(format!("{} is larger than {MAX_INPUT} bytes", path.display())));
    }
    Ok(input)
}

/// Reads at most `limit` + 1 bytes of a regular file (`limit` capped at [`MAX_INPUT`]), so the
/// caller can refuse a file over its own, smaller limit without reading all of it.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_input_up_to(path: &Path, limit: usize) -> Result<Input> {
    use std::io::Read as _;
    if let Some(mut bytes) = output::held(path) {
        bytes.truncate(limit.min(MAX_INPUT).saturating_add(1));
        let kind = detect(&bytes);
        return Ok(Input { path: path.to_path_buf(), bytes, kind });
    }
    let meta = std::fs::metadata(path).map_err(|e| EngineError::Io(format!("cannot open {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let mut bytes = Vec::new();
    let limit = limit.min(MAX_INPUT) as u64;
    // At most one byte past the limit, whatever the file's size claims to be.
    std::fs::File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|e| EngineError::Io(format!("cannot read {}: {e}", path.display())))?;
    let kind = detect(&bytes);
    Ok(Input { path: path.to_path_buf(), bytes, kind })
}

#[cfg(target_arch = "wasm32")]
pub fn read_input_up_to(_: &Path, _: usize) -> Result<Input> {
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
/// `noClobber`, an existing file is refused and left alone. When the write fails, the folders
/// this call created are removed again. Under automation roots `path` must lie in the write
/// root: a derived output beside the input, or the input itself with `inPlace`, is not a
/// parameter the command checks before it runs.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_output(path: &Path, bytes: &[u8], req: &OutputRequest) -> Result<()> {
    check_write(path)?;
    if req.no_clobber && !req.in_place && path.exists() {
        return Err(EngineError::Io(format!("{} exists and noClobber is set", path.display())));
    }
    let created = create_output_dir(req)?;
    let written = fontcraft_format::write_atomic(path, bytes);
    if written.is_err() {
        remove_created(&created);
    }
    written?;
    Ok(())
}

/// Creates the request's `outputDir` when it is missing. Answers the folders this call created,
/// deepest first, for [`remove_created`].
#[cfg(not(target_arch = "wasm32"))]
pub fn create_output_dir(req: &OutputRequest) -> Result<Vec<PathBuf>> {
    let Some(dir) = &req.output_dir else { return Ok(Vec::new()) };
    if req.in_place || dir.as_os_str().is_empty() {
        return Ok(Vec::new());
    }
    check_write(dir)?;
    let missing: Vec<PathBuf> = dir.ancestors().take_while(|a| !a.as_os_str().is_empty() && !a.exists()).map(Path::to_path_buf).collect();
    if let Err(e) = output::create_dir_all(dir) {
        remove_created(&missing);
        return Err(EngineError::Io(format!("cannot create {}: {e}", dir.display())));
    }
    Ok(missing)
}

/// `path` made absolute, with its longest existing part canonicalized (links followed) and the
/// rest applied component by component, so that two spellings of one place compare equal.
pub fn resolved(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let parts: Vec<Component<'_>> = abs.components().collect();
    for k in (1..=parts.len()).rev() {
        let head: PathBuf = parts.iter().take(k).collect();
        if let Ok(mut out) = std::fs::canonicalize(&head) {
            for c in parts.iter().skip(k) {
                match c {
                    Component::ParentDir => {
                        out.pop();
                    }
                    Component::CurDir => {}
                    other => out.push(other),
                }
            }
            return out;
        }
    }
    abs
}

/// Removes the folders [`create_output_dir`] created, deepest first, stopping at the first that
/// is not empty or cannot be removed.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove_created(dirs: &[PathBuf]) {
    for dir in dirs {
        if output::remove_dir(dir).is_err() {
            break;
        }
    }
}

/// [`crate::file_access::check_write`] as an engine error.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_write(path: &Path) -> Result<()> {
    crate::file_access::check_write(path).map_err(|e| EngineError::Io(format!("refusing to write {}: {e}", path.display())))
}

#[cfg(target_arch = "wasm32")]
pub fn check_write(_: &Path) -> Result<()> {
    Err(refused())
}

#[cfg(target_arch = "wasm32")]
pub fn write_output(_: &Path, _: &[u8], _: &OutputRequest) -> Result<()> {
    Err(refused())
}

#[cfg(target_arch = "wasm32")]
pub fn create_output_dir(_: &OutputRequest) -> Result<Vec<PathBuf>> {
    Err(refused())
}

#[cfg(target_arch = "wasm32")]
pub fn remove_created(_: &[PathBuf]) {}

#[cfg(target_arch = "wasm32")]
fn refused() -> EngineError {
    EngineError::Unsupported("reading and writing files by path in the browser".into())
}

/// An output written to a temporary file beside its target.
// The browser writes no files, so nothing builds one there.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct Staged {
    temp: PathBuf,
    /// The file the temporary replaces: the output, or the file it links to.
    target: PathBuf,
    /// A folder (a UFO), which replaces an existing one by being swapped in for it.
    folder: bool,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl Staged {
    /// The temporary file or folder.
    pub fn temp(&self) -> &Path {
        &self.temp
    }
}

/// Writes `bytes` to a new temporary file beside `out` (beside the file it links to, for a
/// link), with the permissions of the file it will replace. Both the file replaced and the
/// temporary must pass the write check before anything is created.
#[cfg(not(target_arch = "wasm32"))]
pub fn stage(out: &Path, bytes: &[u8], n: usize) -> Result<Staged> {
    use std::io::Write as _;
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = if out.is_symlink() { std::fs::canonicalize(out).map_err(|e| io(out, e))? } else { out.to_path_buf() };
    if target.is_dir() {
        return Err(EngineError::Io(format!("cannot write {}: it is a folder", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    // `out` was checked by the caller; a link may lead elsewhere, so check where the write lands.
    check_write(&target)?;
    check_write(&temp)?;
    if output::diverted(&target) {
        output::plan_write(&target, bytes)?;
        return Ok(Staged { temp, target, folder: false });
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| io(&temp, e))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(io(&temp, e));
    }
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    Ok(Staged { temp, target, folder: false })
}

/// Most files one staged folder holds.
#[cfg(not(target_arch = "wasm32"))]
const MAX_FOLDER_FILES: usize = crate::io::MAX_PACKAGE_FILES;

/// Writes `files` (paths relative to the folder, `/`-separated) into a new temporary folder
/// beside `out` (beside the folder it links to, for a link). An existing `out` must be a folder
/// that `is_replaceable` accepts. Both the folder replaced and the temporary must pass the write
/// check before anything is created.
#[cfg(not(target_arch = "wasm32"))]
pub fn stage_folder(
    out: &Path,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
    n: usize,
    is_replaceable: impl Fn(&Path) -> bool,
) -> Result<Staged> {
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    let target = if out.is_symlink() { std::fs::canonicalize(out).map_err(|e| io(out, e))? } else { out.to_path_buf() };
    if target.exists() && !(target.is_dir() && is_replaceable(&target)) {
        return Err(EngineError::Io(format!("cannot write {}: something that is not a UFO has that name", target.display())));
    }
    if files.len() > MAX_FOLDER_FILES {
        return Err(EngineError::Io(format!("cannot write {}: more than {MAX_FOLDER_FILES} files", target.display())));
    }
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let temp = target.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()));
    check_write(&target)?;
    check_write(&temp)?;
    if output::diverted(&target) {
        output::plan_whole(&target, Some(files.values().map(Vec::len).sum()))?;
        return Ok(Staged { temp, target, folder: true });
    }
    std::fs::create_dir(&temp).map_err(|e| io(&temp, e))?;
    let staged = Staged { temp, target, folder: true };
    let written = write_tree(&staged.temp, &staged.target, files);
    match written {
        Ok(()) => Ok(staged),
        Err(e) => {
            discard(std::slice::from_ref(&staged));
            Err(e)
        }
    }
}

/// Writes `files` under `root`, a new folder that stands in for `target` in messages.
#[cfg(not(target_arch = "wasm32"))]
fn write_tree(root: &Path, target: &Path, files: &std::collections::BTreeMap<String, Vec<u8>>) -> Result<()> {
    let io = |path: &Path, e: std::io::Error| EngineError::Io(format!("cannot write {}: {e}", path.display()));
    for (rel, bytes) in files {
        let safe = !rel.is_empty() && !rel.starts_with('/') && !rel.contains('\\') && rel.split('/').all(|s| !s.is_empty() && s != "." && s != "..");
        if !safe {
            return Err(EngineError::Io(format!("cannot write {}: `{rel}` is not a path inside it", target.display())));
        }
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            output::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::write(&path, bytes).map_err(|e| io(&path, e))?;
    }
    Ok(())
}

/// Moves every staged output into place. A staged folder is swapped in: the folder it replaces
/// is moved aside first, then removed. If one cannot be moved, the ones after it are discarded
/// and the error says which were written.
#[cfg(not(target_arch = "wasm32"))]
pub fn commit(staged: &[Staged]) -> Result<()> {
    if let Some(e) = staged.iter().find_map(|s| output::check_clobber(&s.target).err()) {
        discard(staged);
        return Err(e.into());
    }
    for (i, s) in staged.iter().enumerate() {
        if output::diverted(&s.target) {
            continue;
        }
        let moved = if s.folder { swap_folder(s) } else { std::fs::rename(&s.temp, &s.target) };
        if let Err(e) = moved {
            discard(staged.get(i..).unwrap_or_default());
            let before = if i == 0 { String::new() } else { format!("; the {i} outputs before it were written") };
            return Err(EngineError::Io(format!("cannot write {}: {e}{before}", s.target.display())));
        }
        fontcraft_format::written::record(&s.target);
    }
    Ok(())
}

/// Puts a staged folder in place of its target, which is set aside and removed once the new
/// folder is there; when the move fails, the old folder is put back.
#[cfg(not(target_arch = "wasm32"))]
fn swap_folder(s: &Staged) -> std::io::Result<()> {
    if std::fs::symlink_metadata(&s.target).is_err() {
        return std::fs::rename(&s.temp, &s.target);
    }
    let name = s.target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "instance".into());
    let aside = s.target.with_file_name(format!(".{name}.{}.old", std::process::id()));
    std::fs::rename(&s.target, &aside)?;
    if let Err(e) = std::fs::rename(&s.temp, &s.target) {
        let _ = std::fs::rename(&aside, &s.target);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
}

/// Removes the temporaries that are still there.
#[cfg(not(target_arch = "wasm32"))]
pub fn discard(staged: &[Staged]) {
    for s in staged {
        if output::diverted(&s.target) {
            output::unplan(&s.target);
        } else if s.folder {
            let _ = std::fs::remove_dir_all(&s.temp);
        } else {
            let _ = std::fs::remove_file(&s.temp);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn stage(out: &Path, _: &[u8], _: usize) -> Result<Staged> {
    Err(EngineError::Unsupported(format!("writing {} in the browser", out.display())))
}

#[cfg(target_arch = "wasm32")]
pub fn commit(_: &[Staged]) -> Result<()> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn discard(_: &[Staged]) {}

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

        // A write that fails removes the output folders it created, and only those.
        let made = tmp.join("made");
        let req = OutputRequest { output_dir: Some(made.join("deeper")), ..OutputRequest::default() };
        let blocked = made.join("deeper").join("missing").join("C.ttf");
        assert!(write_output(&blocked, b"x", &req).is_err());
        assert!(!made.exists(), "the folders the failed write created are gone");
        assert!(tmp.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn kinds_come_from_content() {
        assert_eq!(detect(b"OTTO\0\0"), InputKind::Cff);
        assert_eq!(detect(b"<?xml version=\"1.0\"?>\n<ttFont sfntVersion=\"OTTO\">"), InputKind::Ttx);
        assert_eq!(detect(b"languagesystem DFLT dflt;"), InputKind::Other);
    }
}
