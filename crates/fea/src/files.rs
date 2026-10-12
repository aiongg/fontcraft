//! The files one compile reads: a closed set of texts by path, and optionally a folder that
//! include statements may also read from.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Diagnostic, MAX_INCLUDE_FILES, MAX_PATH_BYTES, Severity};

/// The root file and the files it may include, by path. Paths are `/`-separated and relative;
/// an include statement names them as written here (`classes/Uppercase.fea`). The assembler in
/// `fontcraft-features` makes one file per prefix, class and feature and a root file that
/// includes them in order.
///
/// Includes that name no file here are read from [`VirtualFiles::include_dir`] when it is set
/// (native targets only): each path is normalised against that folder and may climb out of it,
/// as the reference compilers read it; each file is read once, must be a regular file, must pass
/// the [`IncludeCheck`] when one is set, and counts towards [`crate::Limits::include_files`] and
/// [`crate::Limits::text_bytes`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VirtualFiles {
    root: String,
    files: BTreeMap<String, Arc<str>>,
    include_dir: Option<PathBuf>,
    include_check: Option<IncludeCheck>,
}

/// Decides whether an included file on disk may be read: given its path with links followed, an
/// `Err` refuses it with that message. Two are equal when they are the same check.
#[derive(Clone)]
pub struct IncludeCheck(pub Arc<IncludeCheckFn>);

/// The function of an [`IncludeCheck`].
pub type IncludeCheckFn = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

impl IncludeCheck {
    pub(crate) fn check(&self, path: &Path) -> Result<(), String> {
        (self.0)(path)
    }
}

impl PartialEq for IncludeCheck {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for IncludeCheck {}

impl std::fmt::Debug for IncludeCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IncludeCheck")
    }
}

impl VirtualFiles {
    /// The files of one compile, starting with the root file. Refused: an empty path or one
    /// longer than [`MAX_PATH_BYTES`].
    pub fn new(root_path: &str, root_text: &str) -> Result<Self, Diagnostic> {
        check_path(root_path)?;
        let mut files = BTreeMap::new();
        files.insert(root_path.to_string(), Arc::from(root_text));
        Ok(Self { root: root_path.to_string(), files, include_dir: None, include_check: None })
    }

    /// Adds or replaces a file. Refused: a bad path (as for [`VirtualFiles::new`]) or more than
    /// [`MAX_INCLUDE_FILES`] files. The total size is checked by the compile, against its limits.
    pub fn insert(&mut self, path: &str, text: &str) -> Result<(), Diagnostic> {
        check_path(path)?;
        if !self.files.contains_key(path) && self.files.len() >= MAX_INCLUDE_FILES {
            return Err(Diagnostic::request(Severity::Error, format!("feature code may use at most {MAX_INCLUDE_FILES} files")));
        }
        self.files.insert(path.to_string(), Arc::from(text));
        Ok(())
    }

    /// Lets include statements read files from `dir` (the source's folder) that are not in the
    /// set.
    pub fn with_include_dir(mut self, dir: PathBuf) -> Self {
        self.include_dir = Some(dir);
        self
    }

    /// Reads an included file on disk only when `check` allows it.
    pub fn with_include_check(mut self, check: IncludeCheck) -> Self {
        self.include_check = Some(check);
        self
    }

    pub fn include_check(&self) -> Option<&IncludeCheck> {
        self.include_check.as_ref()
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|t| &**t)
    }

    /// The root file's text.
    pub fn root_text(&self) -> &str {
        self.get(&self.root).unwrap_or_default()
    }

    /// Every file, by path, in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(p, t)| (p.as_str(), &**t))
    }

    pub fn include_dir(&self) -> Option<&std::path::Path> {
        self.include_dir.as_deref()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The bytes of all files together, saturating.
    pub fn total_bytes(&self) -> usize {
        self.files.values().fold(0usize, |n, t| n.saturating_add(t.len()))
    }
}

fn check_path(path: &str) -> Result<(), Diagnostic> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(Diagnostic::request(Severity::Error, format!("a feature file path must have 1 to {MAX_PATH_BYTES} bytes")));
    }
    Ok(())
}
