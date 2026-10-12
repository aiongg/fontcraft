//! Include statements, resolved before fea-rs runs.
//!
//! fea-rs loads every included file before it checks for cycles and depth, and it copies a
//! file's tree once for each statement that includes it, so a file included twice by a file
//! included twice, fifty levels down, grows without bound. This pass therefore follows the
//! include statements itself: it reads each file once, refuses cycles, chains deeper than the
//! limit, and code whose expanded size (each file counted once per inclusion) is over the
//! limit, and checks every file's nesting depth. fea-rs then reads only the files this pass
//! loaded.
//!
//! A relative path is looked up against the root file's folder first, then against the
//! including file's folder; the first that names a file wins. Paths are `/`-separated; an
//! absolute path, or one that climbs above the top of the file set, is refused.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fea_rs::parse::{SourceLoadError, SourceResolver};

use crate::{Diagnostic, Limits, MAX_PATH_BYTES, Severity, VirtualFiles};

/// The files one compile reads, by normalised path.
pub(crate) type Loaded = Arc<BTreeMap<String, Arc<str>>>;

/// Follows the include statements from the root file and loads every file they reach.
pub(crate) fn load(files: &VirtualFiles, limits: &Limits) -> Result<(String, Loaded), Vec<Diagnostic>> {
    let root = normalize("", files.root())
        .ok_or_else(|| vec![Diagnostic::request(Severity::Error, format!("the root feature file path `{}` cannot be used", files.root()))])?;
    let mut loader = Loader {
        files,
        limits,
        loaded: BTreeMap::new(),
        expanded: HashMap::new(),
        disk_bytes: 0,
        stack: Vec::new(),
        root_dir: parent(&root).to_string(),
    };
    loader.loaded.insert(root.clone(), Arc::from(files.root_text()));
    let text = Arc::from(files.root_text());
    loader.visit(&root, &text).map_err(|d| vec![d])?;
    Ok((root, Arc::new(loader.loaded)))
}

struct Loader<'a> {
    files: &'a VirtualFiles,
    limits: &'a Limits,
    loaded: BTreeMap<String, Arc<str>>,
    /// The expanded size of each file already followed.
    expanded: HashMap<String, usize>,
    disk_bytes: usize,
    /// The include chain being followed, for cycles and depth.
    stack: Vec<String>,
    root_dir: String,
}

impl Loader<'_> {
    fn visit(&mut self, path: &str, text: &Arc<str>) -> Result<usize, Diagnostic> {
        if let Some(&size) = self.expanded.get(path) {
            return Ok(size);
        }
        check_nesting(path, text, self.limits.nesting_depth.min(crate::MAX_NESTING_DEPTH))?;
        self.stack.push(path.to_string());
        let mut size = text.len();
        for (raw, span) in include_statements(text) {
            let Some(target) = resolve(&self.root_dir, path, raw, |p| self.exists(p)) else {
                // fea-rs reports the missing file at the statement.
                continue;
            };
            if self.stack.contains(&target) {
                return Err(Diagnostic::at(path, span, Severity::Error, format!("`{target}` includes itself through this statement")));
            }
            let depth = self.limits.include_depth.min(crate::MAX_INCLUDE_DEPTH);
            if self.stack.len() >= depth {
                return Err(Diagnostic::at(path, span, Severity::Error, format!("includes may be nested at most {depth} deep")));
            }
            let child = match self.loaded.get(&target) {
                Some(text) => text.clone(),
                None => {
                    let text = self.read(&target).map_err(|m| Diagnostic::at(path, span.clone(), Severity::Error, m))?;
                    self.loaded.insert(target.clone(), text.clone());
                    text
                }
            };
            size = size.saturating_add(self.visit(&target, &child)?);
            if size > self.limits.text_bytes {
                return Err(Diagnostic::at(
                    path,
                    span,
                    Severity::Error,
                    format!("the feature code with its includes comes to more than {} bytes", self.limits.text_bytes),
                ));
            }
        }
        self.stack.pop();
        self.expanded.insert(path.to_string(), size);
        Ok(size)
    }

    fn exists(&self, path: &str) -> bool {
        self.files.get(path).is_some() || self.disk_path(path).is_some_and(|p| p.is_file())
    }

    fn read(&mut self, path: &str) -> Result<Arc<str>, String> {
        let limit = self.limits.include_files.min(crate::MAX_INCLUDE_FILES);
        if self.loaded.len() >= limit {
            return Err(format!("feature code may read at most {limit} files"));
        }
        if let Some(text) = self.files.get(path) {
            return Ok(Arc::from(text));
        }
        let Some(file) = self.disk_path(path) else {
            return Err(format!("`{path}` was not found"));
        };
        let dir = self.files.include_dir().and_then(|d| d.canonicalize().ok());
        let real = file.canonicalize().map_err(|e| format!("`{path}` could not be read: {e}"))?;
        if !dir.is_some_and(|d| real.starts_with(d)) {
            return Err(format!("`{path}` is outside the source's folder"));
        }
        let len = real.metadata().map(|m| m.len()).map_err(|e| format!("`{path}` could not be read: {e}"))?;
        let budget = self.limits.text_bytes.saturating_sub(self.files.total_bytes()).saturating_sub(self.disk_bytes);
        if usize::try_from(len).map_or(true, |len| len > budget) {
            return Err(format!("the feature code with its includes comes to more than {} bytes", self.limits.text_bytes));
        }
        let bytes = std::fs::read(&real).map_err(|e| format!("`{path}` could not be read: {e}"))?;
        self.disk_bytes = self.disk_bytes.saturating_add(bytes.len());
        let text = String::from_utf8(bytes).map_err(|_| format!("`{path}` is not UTF-8 text"))?;
        Ok(Arc::from(text))
    }

    fn disk_path(&self, path: &str) -> Option<PathBuf> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        self.files.include_dir().map(|d| d.join(path))
    }
}

/// The first of the candidate paths for `raw`, included from `from`, that `exists`; `None`
/// when the path is refused or names no file.
fn resolve(root_dir: &str, from: &str, raw: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let candidates = [normalize(root_dir, raw), normalize(parent(from), raw)];
    candidates.into_iter().flatten().find(|p| exists(p))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `raw` joined to `dir`, with `.` and `..` resolved. `None` for an absolute path, one that
/// climbs above the top, an empty one or one longer than [`MAX_PATH_BYTES`].
pub(crate) fn normalize(dir: &str, raw: &str) -> Option<String> {
    let absolute = raw.starts_with('/') || raw.starts_with('\\') || raw.as_bytes().get(1) == Some(&b':');
    if absolute || raw.len() > MAX_PATH_BYTES {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in dir.split('/').chain(raw.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    let path = parts.join("/");
    (!path.is_empty() && path.len() <= MAX_PATH_BYTES).then_some(path)
}

/// The include statements of `text`: the path with surrounding whitespace trimmed, and the
/// statement's byte range. Comments and strings are skipped. `include` counts whatever byte
/// precedes it: fea-rs's error recovery follows `-include(…)`, `1include(…)` and the like, and
/// every statement it follows must be counted here. Statements inside anonymous blocks or
/// identifiers are found too, which only makes the checks stricter.
fn include_statements(text: &str) -> Vec<(&str, std::ops::Range<usize>)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => i = skip_comment(bytes, i),
            b'"' => i = skip_string(bytes, i),
            b'i' if bytes.get(i..i + 7) == Some(b"include") => {
                let mut j = i + 7;
                while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                    j += 1;
                }
                if bytes.get(j) != Some(&b'(') {
                    i += 7;
                    continue;
                }
                let open = j + 1;
                let Some(len) = bytes.get(open..).and_then(|rest| rest.iter().position(|&c| c == b')')) else {
                    break;
                };
                if let Some(path) = text.get(open..open + len) {
                    let trimmed = path.trim_matches(|c: char| c.is_ascii_whitespace());
                    out.push((if trimmed.is_empty() { path } else { trimmed }, i..open + len + 1));
                }
                i = open + len + 1;
            }
            _ => i += 1,
        }
    }
    out
}

fn skip_comment(bytes: &[u8], i: usize) -> usize {
    bytes.get(i..).and_then(|rest| rest.iter().position(|&c| c == b'\n' || c == b'\r')).map_or(bytes.len(), |n| i + n)
}

fn skip_string(bytes: &[u8], i: usize) -> usize {
    bytes.get(i + 1..).and_then(|rest| rest.iter().position(|&c| c == b'"')).map_or(bytes.len(), |n| i + n + 2)
}

/// Refuses braces, brackets and parentheses nested deeper than `limit`, outside comments and
/// strings. fea-rs's parser recurses on them.
pub(crate) fn check_nesting(path: &str, text: &str, limit: usize) -> Result<(), Diagnostic> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => {
                i = skip_comment(bytes, i);
                continue;
            }
            b'"' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'{' | b'[' | b'(' => {
                depth += 1;
                if depth > limit {
                    return Err(Diagnostic::at(path, i..i + 1, Severity::Error, format!("braces and brackets may be nested at most {limit} deep")));
                }
            }
            b'}' | b']' | b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// What fea-rs reads through: the files [`load`] found, nothing else.
pub(crate) struct MapResolver {
    pub files: Loaded,
    pub root_dir: String,
}

impl SourceResolver for MapResolver {
    fn get_contents(&self, path: &Path) -> Result<Arc<str>, SourceLoadError> {
        let key = path.to_string_lossy();
        self.files.get(key.as_ref()).cloned().ok_or_else(|| SourceLoadError::new(path.to_path_buf(), "file not found"))
    }

    fn resolve_raw_path(&self, path: &Path, included_from: Option<&Path>) -> PathBuf {
        let raw = path.to_string_lossy();
        let from = included_from.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if included_from.is_none() {
            return path.to_path_buf();
        }
        let found = resolve(&self.root_dir, &from, &raw, |p| self.files.contains_key(p));
        PathBuf::from(found.unwrap_or_else(|| raw.into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_normalised_and_kept_inside() {
        assert_eq!(normalize("a/b", "../c.fea").as_deref(), Some("a/c.fea"));
        assert_eq!(normalize("", "./x/./y.fea").as_deref(), Some("x/y.fea"));
        assert_eq!(normalize("", "../c.fea"), None);
        assert_eq!(normalize("", "/etc/passwd"), None);
        assert_eq!(normalize("", "C:/x.fea"), None);
    }

    #[test]
    fn include_statements_skip_comments_and_strings() {
        let text = "# include(a.fea)\ninclude(b.fea);\nname \"include(c)\"; -include(d); include ( e f.fea )";
        let paths: Vec<&str> = include_statements(text).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, ["b.fea", "d", "e f.fea"]);
    }
}
