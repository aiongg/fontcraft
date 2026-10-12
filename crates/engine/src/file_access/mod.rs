//! The folders automation may read and write.
//!
//! `fontcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>`, and the desktop
//! app's control channel started with the same flags, confine the files an agent's commands reach.
//! [`AutomationRoots`] holds the two folders; [`confine`] puts them in force on the calling thread
//! while it runs a request. While they are in force:
//!
//! - every command's declared file parameters are checked before it runs ([`command`]): the files
//!   it reads against the read root, the files it writes against the write root, and the places
//!   it writes or reads without a parameter (the open documents' files, the open projects' files
//!   and export destinations) as well. Commands that run steps or open files no parameter names
//!   (macros, recent documents, URLs) are refused.
//! - the files a document links to are read only inside the read root ([`check_read`]): Import
//!   Font and Import Master sources, images, glyph data files and feature includes. A link that
//!   points outside it is treated as a missing file.
//!
//! Paths are compared as the file system sees them: made absolute against the working directory,
//! links followed (for a file that doesn't exist yet, the links of its nearest existing folder,
//! the rest of the path being plain names, so `..` below a missing folder is refused, and so is a
//! link that leads nowhere), then compared with the root by whole folder names (`/work2` is not
//! inside `/work`), ignoring case on Windows. Read and write access are separate: a root left out
//! grants none of its access. Without roots nothing is checked.
//!
//! A path is checked, then opened: another program that swaps in a link between the two can still
//! win that race. The roots keep an agent to the files it was given; they don't guard against other
//! programs on the computer.

pub mod command;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Read access or write access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

impl Access {
    fn word(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// The folders automation may read and write; a root left out grants no access of its kind.
#[derive(Debug)]
pub struct AutomationRoots {
    read: Option<Root>,
    write: Option<Root>,
}

#[derive(Debug)]
struct Root {
    /// The folder made absolute, links followed: what paths are compared with.
    resolved: PathBuf,
    /// The same without Windows' `\\?\` prefix, for people.
    shown: PathBuf,
}

impl Root {
    /// The folder at `path`, which must exist.
    fn open(path: &Path, access: Access) -> Result<Self, String> {
        let kind = access.word();
        let resolved = std::fs::canonicalize(path)
            .and_then(|p| if p.is_dir() { Ok(p) } else { Err(std::io::ErrorKind::NotADirectory.into()) })
            .map_err(|e| format!("cannot open the automation {kind} root `{}`: {e}", path.display()))?;
        Ok(Self { shown: plain(&resolved), resolved })
    }
}

impl AutomationRoots {
    /// The roots `read` and `write`, which must be existing folders; `None` when neither is given
    /// (nothing is confined then).
    pub fn new(read: Option<&Path>, write: Option<&Path>) -> Result<Option<Arc<Self>>, String> {
        if read.is_none() && write.is_none() {
            return Ok(None);
        }
        let read = read.map(|p| Root::open(p, Access::Read)).transpose()?;
        let write = write.map(|p| Root::open(p, Access::Write)).transpose()?;
        Ok(Some(Arc::new(Self { read, write })))
    }

    /// The read root, absolute with its links followed.
    pub fn read_root(&self) -> Option<&Path> {
        self.read.as_ref().map(|r| r.shown.as_path())
    }

    /// The write root, absolute with its links followed.
    pub fn write_root(&self) -> Option<&Path> {
        self.write.as_ref().map(|r| r.shown.as_path())
    }

    /// One sentence for people and agents: which folders may be read and written.
    pub fn describe(&self) -> String {
        let show = |root: Option<&Path>| root.map_or_else(|| "none".to_string(), |r| r.display().to_string());
        format!("files are confined: read root {}, write root {}", show(self.read_root()), show(self.write_root()))
    }

    /// May the file or folder at `path` be read (or written)? `Err` says why not.
    pub fn check(&self, path: &str, access: Access) -> Result<(), String> {
        self.resolved(path, access).map(|_| ())
    }

    /// `path` absolute with its links followed, when it lies inside the root of `access`.
    pub fn resolved(&self, path: &str, access: Access) -> Result<PathBuf, String> {
        let root = match access {
            Access::Read => self.read.as_ref(),
            Access::Write => self.write.as_ref(),
        };
        let Some(Root { resolved: folder, shown }) = root else {
            return Err(format!("automation filesystem access is not granted: {} authority is absent: {path}", access.word()));
        };
        let resolved = resolve(path)?;
        if inside(&resolved, folder) {
            return Ok(resolved);
        }
        Err(format!("automation path rejected: outside the {} root {}: {path}", access.word(), shown.display()))
    }
}

thread_local! {
    /// The roots in force on this thread ([`confine`]).
    static SCOPE: RefCell<Option<Arc<AutomationRoots>>> = const { RefCell::new(None) };
}

/// Puts the roots it replaced back when dropped, also when the confined work panics.
struct Restore(Option<Arc<AutomationRoots>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        SCOPE.with(|s| *s.borrow_mut() = previous);
    }
}

/// Runs `f` with `roots` in force on this thread; `None` runs it as things stand.
pub fn confine<T>(roots: Option<&Arc<AutomationRoots>>, f: impl FnOnce() -> T) -> T {
    match roots {
        Some(r) => {
            let _restore = Restore(SCOPE.with(|s| s.replace(Some(Arc::clone(r)))));
            f()
        }
        None => f(),
    }
}

/// The roots in force on this thread, if any.
pub fn current() -> Option<Arc<AutomationRoots>> {
    SCOPE.with(|s| s.borrow().clone())
}

/// May `path` be read here? Always, without roots in force.
pub fn check_read(path: &Path) -> Result<(), String> {
    check(path, Access::Read)
}

/// The check for the files feature code includes: each must pass [`check_read`] against the
/// roots in force now. `None` without roots.
pub fn include_check() -> Option<fontcraft_fea::IncludeCheck> {
    let roots = current()?;
    Some(fontcraft_fea::IncludeCheck(Arc::new(move |path: &Path| {
        let text = path.to_str().ok_or_else(|| format!("automation path rejected: not a UTF-8 path: {}", path.display()))?;
        roots.check(text, Access::Read)
    })))
}

/// May `path` be written here? Always, without roots in force.
pub fn check_write(path: &Path) -> Result<(), String> {
    check(path, Access::Write)
}

fn check(path: &Path, access: Access) -> Result<(), String> {
    let Some(roots) = current() else { return Ok(()) };
    let text = path.to_str().ok_or_else(|| format!("automation path rejected: not a UTF-8 path: {}", path.display()))?;
    roots.check(text, access)
}

/// `path` made absolute with its links followed; for a path that doesn't exist (yet), its nearest
/// existing folder's links followed and the rest appended.
fn resolve(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("automation path rejected: path is empty".into());
    }
    if path.contains('\0') {
        return Err(format!("automation path rejected: a path cannot contain a NUL character: {}", path.replace('\0', "\\0")));
    }
    // Checked as given: making `…\NUL` absolute gives the device path `\\.\NUL` on Windows.
    if cfg!(windows) && Path::new(path).components().any(|c| matches!(c, Component::Normal(n) if is_device_name(&n.to_string_lossy()))) {
        return Err(format!("automation path rejected: reserved device names are not allowed: {path}"));
    }
    let absolute = std::path::absolute(path).map_err(|e| format!("automation path rejected: cannot resolve {path}: {e}"))?;
    // The names below the nearest existing folder, last first.
    let mut missing: Vec<OsString> = vec![];
    let mut at = absolute.as_path();
    loop {
        match std::fs::canonicalize(at) {
            Ok(mut found) => {
                found.extend(missing.iter().rev());
                return Ok(found);
            }
            Err(e) => match std::fs::symlink_metadata(at) {
                Ok(m) if m.file_type().is_symlink() => return Err(format!("automation path rejected: a link that leads nowhere: {path}")),
                // There, yet its real path can't be had (no permission): nothing to compare.
                Ok(_) => return Err(format!("automation path rejected: cannot resolve {path}: {e}")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("automation path rejected: cannot resolve {path}: {e}")),
            },
        }
        match (at.components().next_back(), at.parent()) {
            (Some(Component::Normal(name)), Some(parent)) => {
                missing.push(name.to_os_string());
                at = parent;
            }
            (Some(Component::ParentDir), _) => {
                return Err(format!("automation path rejected: parent traversal below a folder that doesn't exist: {path}"));
            }
            // Down to the root (or drive) and it doesn't exist either.
            _ => {
                let e = std::io::Error::from(std::io::ErrorKind::NotFound);
                return Err(format!("automation path rejected: cannot resolve {path}: {e}"));
            }
        }
    }
}

/// Is `path` (resolved) `root` or inside it, component by component (without regard to case on
/// Windows, whose file systems ignore it)?
fn inside(path: &Path, root: &Path) -> bool {
    let mut path = path.components();
    root.components().all(|r| path.next().is_some_and(|p| same_component(p, r)))
}

fn same_component(a: Component, b: Component) -> bool {
    if a == b {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    match (a.as_os_str().to_str(), b.as_os_str().to_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => false,
    }
}

/// A Windows device name (`CON`, `NUL`, `COM1`… also with an extension), which names no file.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

/// `path` without Windows' verbatim prefix: `\\?\C:\x` is `C:\x`, `\\?\UNC\server\share` is
/// `\\server\share`.
fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}
