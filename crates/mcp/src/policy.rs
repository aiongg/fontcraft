//! What an MCP client may do: `--read-only`, `--allow` and `--deny` patterns over command ids, and
//! `--root` folders for file parameters.
//!
//! Which parameters name files, and whether a command writes to disk, come from the command
//! catalogue (`file_params`, `writes_files`): the engine declares them for every command.
//!
//! Every check fails closed: a command the policy cannot place (an unknown id under
//! `--read-only` or `--root`, a macro whose steps it cannot see, a path it cannot resolve, a
//! parameter value it cannot read as a path, a place it cannot see) is refused, never run.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Most `--allow` or `--deny` patterns.
pub const MAX_PATTERNS: usize = 256;
/// Longest pattern, in bytes.
pub const MAX_PATTERN_LEN: usize = 256;
/// Most `--root` folders.
pub const MAX_ROOTS: usize = 64;
/// Deepest nesting of `command.batch` inside `command.batch`.
pub const MAX_BATCH_NESTING: usize = 8;
/// Most paths checked for one command (one step of a batch).
pub const MAX_PATHS: usize = 100_000;

/// Commands that run other commands or open files the server cannot see in their parameters
/// (macro files, recent-document lists, URLs). They are refused whenever the policy restricts
/// anything.
pub const INDIRECT_COMMANDS: &[&str] =
    &["script.run", "script.rerunLast", "script.runActionSet", "script.repeatLastAction", "file.openRecent", "url.open", "url.show"];

/// The batch command: its steps are checked one by one.
pub const BATCH_COMMAND: &str = "command.batch";

/// How a file parameter holds its paths: the catalogue's `form`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileForm {
    /// A path, or a list of paths.
    Path,
    /// Table items: `"TAG"`, `"TAG=FILE"` (several comma-separated in one string) or
    /// `{tag, file?, from?: {font}}`.
    Tables,
    /// An open document's title or path; anything else is a file read from disk.
    Document,
    /// A form this server does not know: any value but null is refused.
    Unknown,
}

/// One parameter that names files, from the catalogue's `file_params`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileParam {
    pub name: String,
    pub write: bool,
    pub form: FileForm,
    /// Values that are not paths.
    pub keywords: Vec<String>,
}

impl FileParam {
    fn from_entry(entry: &Value) -> Option<Self> {
        let name = entry.get("name")?.as_str()?.to_string();
        // An access this server does not know counts as a write.
        let write = entry.get("access").and_then(Value::as_str) != Some("read");
        let form = match entry.get("form").and_then(Value::as_str) {
            Some("path") => FileForm::Path,
            Some("tables") => FileForm::Tables,
            Some("document") => FileForm::Document,
            _ => FileForm::Unknown,
        };
        let keywords = match entry.get("keywords") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list.iter().map(|k| k.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?,
            Some(_) => return None,
        };
        Some(Self { name, write, form, keywords })
    }
}

/// What the policy knows about one command, from its catalogue entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandFacts {
    /// Reads and changes nothing in the session (it may still write files).
    pub query: bool,
    /// Writes to disk.
    pub writes_files: bool,
    /// The parameters that name files.
    pub files: Vec<FileParam>,
    /// The entry's file declarations could not be read; the command is refused whenever file
    /// access is restricted.
    pub unreadable: bool,
}

impl CommandFacts {
    /// Reads one entry of the command catalogue; `None` when it has no id.
    pub fn from_entry(entry: &Value) -> Option<(String, Self)> {
        let id = entry.get("id")?.as_str()?.to_string();
        let query = entry.get("query").and_then(Value::as_bool).unwrap_or(false);
        let writes_files = entry.get("writes_files").and_then(Value::as_bool).unwrap_or(false);
        let (files, unreadable) = match entry.get("file_params") {
            None | Some(Value::Null) => (Vec::new(), false),
            Some(Value::Array(list)) => match list.iter().map(FileParam::from_entry).collect::<Option<Vec<_>>>() {
                Some(files) => (files, false),
                None => (Vec::new(), true),
            },
            Some(_) => (Vec::new(), true),
        };
        Some((id, Self { query, writes_files, files, unreadable }))
    }

    /// Whether the command may write to disk.
    pub fn writes(&self) -> bool {
        self.writes_files || self.unreadable || self.files.iter().any(|f| f.write)
    }
}

/// Where a command writes, or what it reads, without a parameter naming the place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Implicit {
    /// The open documents' own files, or the folders that hold them.
    Documents,
    /// The open projects' own files.
    ProjectFiles,
    /// The open projects' export destinations and the font files they link.
    ProjectExports,
    /// A place this server cannot see; the reason, after the command id.
    Hidden(&'static str),
}

/// The implicit place `id` writes to with these parameters, if any.
pub(crate) fn implicit_place(id: &str, params: &Value) -> Option<Implicit> {
    let given = |key: &str| params.get(key).is_some_and(|v| !v.is_null());
    match id {
        "file.save" if !given("path") => Some(Implicit::Documents),
        "file.exportWithProfiles" if given("bySource") && params.get("bySource") != Some(&Value::Bool(false)) => Some(Implicit::Documents),
        "project.save" if !given("path") => Some(Implicit::ProjectFiles),
        "project.export" => Some(Implicit::ProjectExports),
        "script.saveActionSet" if !given("path") => Some(Implicit::Hidden("writes into the Macros folder without `path`; give `path`")),
        "script.recordMacro" if given("name") => Some(Implicit::Hidden(
            "writes the recording into the Macros folder when `name` is given; stop without `name` and save the steps with script.saveActionSet and `path`",
        )),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Policy {
    read_only: bool,
    allow: Vec<String>,
    deny: Vec<String>,
    /// Canonical folders; `None` admits every path.
    roots: Option<Vec<PathBuf>>,
    /// One of the roots is the file system's root: every path is admitted.
    every_path: bool,
    /// Where relative paths resolve; the working directory when unset.
    base: Option<PathBuf>,
}

impl Policy {
    /// Admits every command and every path.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Admits only commands that declare themselves queries and write no files.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Adds an `--allow` pattern: once there is one, a command must match one of them.
    pub fn allow(mut self, pattern: &str) -> Result<Self, String> {
        check_pattern(pattern, self.allow.len())?;
        self.allow.push(pattern.to_string());
        Ok(self)
    }

    /// Adds a `--deny` pattern: a command that matches one is refused, even when an allow pattern
    /// matches it too.
    pub fn deny(mut self, pattern: &str) -> Result<Self, String> {
        check_pattern(pattern, self.deny.len())?;
        self.deny.push(pattern.to_string());
        Ok(self)
    }

    /// Adds a `--root` folder. File parameters must then lie inside one of the roots. The file
    /// system's root admits every path, which leaves file access unrestricted.
    pub fn root(mut self, dir: &Path) -> Result<Self, String> {
        let roots = self.roots.get_or_insert_with(Vec::new);
        if roots.len() >= MAX_ROOTS {
            return Err(format!("at most {MAX_ROOTS} roots"));
        }
        let canonical = std::fs::canonicalize(dir).map_err(|e| format!("root `{}`: {e}", dir.display()))?;
        if !canonical.is_dir() {
            return Err(format!("root `{}` is not a folder", dir.display()));
        }
        if canonical.parent().is_none() {
            self.every_path = true;
        }
        roots.push(canonical);
        Ok(self)
    }

    /// The folder relative paths resolve against (default: the working directory).
    pub fn base_dir(mut self, dir: &Path) -> Self {
        self.base = Some(dir.to_path_buf());
        self
    }

    /// Whether any command could be refused by id.
    pub fn restricts_commands(&self) -> bool {
        self.read_only || !self.allow.is_empty() || !self.deny.is_empty()
    }

    /// Whether file parameters must lie inside the roots.
    pub fn restricts_paths(&self) -> bool {
        self.roots.is_some() && !self.every_path
    }

    /// Whether anything at all is restricted.
    pub fn is_restricted(&self) -> bool {
        self.restricts_commands() || self.restricts_paths()
    }

    /// Whether the checks need the command catalogue (queries, file declarations).
    pub(crate) fn needs_catalogue(&self) -> bool {
        self.read_only || self.restricts_paths()
    }

    /// One paragraph for `initialize`'s instructions, or `None` when nothing is restricted.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.read_only {
            parts.push("only commands that read (queries) and write no files run".to_string());
        }
        if !self.allow.is_empty() {
            parts.push(format!("only commands matching {} run", self.allow.join(", ")));
        }
        if !self.deny.is_empty() {
            parts.push(format!("commands matching {} are refused", self.deny.join(", ")));
        }
        if let Some(roots) = self.path_roots() {
            let list: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
            parts.push(format!("file parameters must lie inside {}", list.join(", ")));
        }
        if self.is_restricted() {
            parts.push("macros and other commands that run steps this server cannot see are refused".to_string());
        }
        (!parts.is_empty()).then(|| format!("This server is restricted: {}.", parts.join("; ")))
    }

    /// Decides one command by id. `facts` is its catalogue entry (`None`: not in the catalogue,
    /// or the catalogue was not read because no check needs it).
    pub fn admit(&self, id: &str, facts: Option<&CommandFacts>) -> Result<(), String> {
        if let Some(p) = self.deny.iter().find(|p| glob(p, id)) {
            return Err(format!("`{id}` is refused by this server (--deny {p})"));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|p| glob(p, id)) {
            return Err(format!("`{id}` is not among the commands this server allows (--allow {})", self.allow.join(", ")));
        }
        if self.read_only {
            match facts {
                None => return Err(format!("`{id}` is not a known query, and this server is read-only")),
                Some(f) if !f.query => return Err(format!("`{id}` changes documents, files or settings, and this server is read-only")),
                Some(f) if f.writes() => return Err(format!("`{id}` writes files, and this server is read-only")),
                Some(_) => {}
            }
        }
        if self.is_restricted() && INDIRECT_COMMANDS.contains(&id) {
            return Err(format!(
                "`{id}` runs steps or opens files this server cannot check against its restrictions; run the steps one by one, or with command.batch"
            ));
        }
        if self.restricts_paths() {
            match facts {
                None => return Err(format!("`{id}` is not in the command catalogue, so its file parameters cannot be checked (--root)")),
                Some(f) if f.unreadable => {
                    return Err(format!("the catalogue's file declarations for `{id}` cannot be read, so its paths cannot be checked (--root)"));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Checks every file parameter `facts` declares against the roots and replaces each path
    /// with its resolved absolute form, so the command runs on exactly the path that was checked
    /// (and the app, whose working directory may differ, reads the same file). `open` lists the
    /// titles and paths of the open documents, which a document parameter may name instead.
    pub fn check_params(&self, facts: &CommandFacts, params: &mut Value, open: &[String]) -> Result<(), String> {
        self.check_params_reads(facts, params, open).map(|_| ())
    }

    /// As [`Policy::check_params`], answering the resolved paths of the files the command reads,
    /// whose own references ([`Policy::check_references`]) the caller checks next.
    pub fn check_params_reads(&self, facts: &CommandFacts, params: &mut Value, open: &[String]) -> Result<Vec<PathBuf>, String> {
        let Some(roots) = self.path_roots() else { return Ok(Vec::new()) };
        let Value::Object(map) = params else { return Ok(Vec::new()) };
        let mut checker = Checker { roots, base: self.base()?, open, left: MAX_PATHS, reading: false, reads: Vec::new() };
        for f in &facts.files {
            if let Some(value) = map.get_mut(&f.name) {
                checker.reading = !f.write;
                checker.value(f, value)?;
            }
        }
        Ok(checker.reads)
    }

    /// Checks the files that reading the source at `path` reads in turn (a designspace's UFOs, the
    /// Import Font and Import Master files) against the roots, before any of them is read. `what`
    /// names the command for the refusal.
    pub fn check_references(&self, path: &Path, what: &str) -> Result<(), String> {
        if self.path_roots().is_none() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let reader = path.display().to_string();
            fontcraft_engine::io::check_references(path, &mut |p| {
                let text = p.to_str().ok_or_else(|| format!("{what}: {reader} refers to a path that is not UTF-8"))?;
                self.check_implicit_path(text, &format!("{what} reads, through {reader},"))
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, what);
            Ok(())
        }
    }

    /// Checks one path a command writes or reads without naming it; `what` comes before the
    /// path in the refusal ("`file.save` writes the open document's file").
    pub fn check_implicit_path(&self, path: &str, what: &str) -> Result<(), String> {
        let Some(roots) = self.path_roots() else { return Ok(()) };
        let resolved = resolve(&self.base()?, path)?;
        if roots.iter().any(|r| resolved.starts_with(r)) {
            Ok(())
        } else {
            Err(format!("{what} {path}, which lies outside the folders this server may use (--root)"))
        }
    }

    fn path_roots(&self) -> Option<&[PathBuf]> {
        if self.every_path { None } else { self.roots.as_deref() }
    }

    fn base(&self) -> Result<PathBuf, String> {
        match &self.base {
            Some(b) => Ok(b.clone()),
            None => std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}")),
        }
    }
}

/// Checks the values of one command's file parameters.
struct Checker<'a> {
    roots: &'a [PathBuf],
    base: PathBuf,
    open: &'a [String],
    /// Paths still allowed before the command is refused ([`MAX_PATHS`]).
    left: usize,
    /// The parameter being checked names files the command reads.
    reading: bool,
    /// The resolved paths of the files the command reads.
    reads: Vec<PathBuf>,
}

impl Checker<'_> {
    /// A parameter's value: one item or a list of items.
    fn value(&mut self, f: &FileParam, value: &mut Value) -> Result<(), String> {
        match value {
            Value::Null => Ok(()),
            Value::Array(items) => items.iter_mut().try_for_each(|item| self.item(f, item)),
            other => self.item(f, other),
        }
    }

    fn item(&mut self, f: &FileParam, item: &mut Value) -> Result<(), String> {
        let key = f.name.as_str();
        match (f.form, item) {
            (_, Value::String(text)) if f.keywords.iter().any(|k| k == text) => Ok(()),
            (FileForm::Path, Value::String(text)) => {
                *text = self.path(key, text)?;
                Ok(())
            }
            (FileForm::Document, Value::String(text)) => {
                if !self.open.iter().any(|o| o == text) {
                    *text = self.path(key, text)?;
                }
                Ok(())
            }
            (FileForm::Tables, Value::String(text)) => {
                *text = self.table_list(key, text)?;
                Ok(())
            }
            (FileForm::Tables, Value::Object(entry)) => {
                if let Some(file) = entry.get_mut("file") {
                    self.optional_path(key, file)?;
                }
                match entry.get_mut("from") {
                    None | Some(Value::Null) => Ok(()),
                    Some(Value::Object(from)) => match from.get_mut("font") {
                        Some(font) => self.optional_path(key, font),
                        None => Ok(()),
                    },
                    Some(_) => Err(format!("`{key}`: `from` holds a value this server cannot check as a path")),
                }
            }
            _ => Err(format!("`{key}` holds a value this server cannot check as a path")),
        }
    }

    fn optional_path(&mut self, key: &str, value: &mut Value) -> Result<(), String> {
        match value {
            Value::Null => Ok(()),
            Value::String(text) => {
                *text = self.path(key, text)?;
                Ok(())
            }
            _ => Err(format!("`{key}` holds a value this server cannot check as a path")),
        }
    }

    /// `"TAG"`, `"TAG=FILE"`, or several of them separated by commas, as the table commands
    /// read them.
    fn table_list(&mut self, key: &str, text: &str) -> Result<String, String> {
        let mut parts = Vec::new();
        for part in text.split(',').filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((tag, file)) => {
                    let resolved = self.path(key, file)?;
                    if resolved.contains(',') {
                        return Err(format!(
                            "`{key}`: {file} resolves to a path with a comma, which a TAG=FILE list cannot hold; give {{tag, file}}"
                        ));
                    }
                    parts.push(format!("{tag}={resolved}"));
                }
                None => parts.push(part.to_string()),
            }
        }
        Ok(parts.join(","))
    }

    fn path(&mut self, key: &str, text: &str) -> Result<String, String> {
        self.left = self.left.checked_sub(1).ok_or_else(|| format!("more than {MAX_PATHS} paths in one command"))?;
        let resolved = resolve(&self.base, text).map_err(|e| format!("`{key}`: {e}"))?;
        if !self.roots.iter().any(|r| resolved.starts_with(r)) {
            return Err(format!("`{key}`: {text} lies outside the folders this server may use (--root)"));
        }
        if self.reading {
            self.reads.push(resolved.clone());
        }
        resolved.to_str().map(str::to_string).ok_or_else(|| format!("`{key}`: {text} does not resolve to a UTF-8 path"))
    }
}

fn check_pattern(pattern: &str, count: usize) -> Result<(), String> {
    if count >= MAX_PATTERNS {
        return Err(format!("at most {MAX_PATTERNS} patterns"));
    }
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_LEN {
        return Err(format!("a pattern has 1 to {MAX_PATTERN_LEN} bytes"));
    }
    Ok(())
}

/// Shell-style match over a whole command id: `*` is any run of characters, `?` one character,
/// everything else itself. Linear backtracking over the last `*`, so the work is at most the
/// product of the two lengths.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(c) if *c == '?' || Some(c) == t.get(ti) => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p.get(pi..).unwrap_or_default().iter().all(|c| *c == '*')
}

/// The absolute path `text` names, symbolic links resolved. The longest part that exists is
/// resolved by the operating system; the rest must be plain names (no `..`), since nothing on
/// disk says where `..` of a missing folder leads.
fn resolve(base: &Path, text: &str) -> Result<PathBuf, String> {
    if text.contains('\0') {
        return Err("a path cannot contain a NUL character".into());
    }
    let joined = base.join(text);
    let mut existing = joined.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(existing) {
            let mut out = canonical;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        let Some(name) = existing.file_name() else {
            return Err(format!("cannot resolve {text}"));
        };
        // `file_name` is `None` for a path ending in `..`, so only plain names are kept.
        rest.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| format!("cannot resolve {text}"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(query: bool, writes_files: bool, files: &[(&str, bool, FileForm)]) -> CommandFacts {
        let files =
            files.iter().map(|(name, write, form)| FileParam { name: name.to_string(), write: *write, form: *form, keywords: Vec::new() }).collect();
        CommandFacts { query, writes_files, files, unreadable: false }
    }

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
    /// A fresh folder with `work` inside; the policy's only root is `work`.
    fn rooted(name: &str) -> (Dir, PathBuf, Policy) {
        let base = std::env::temp_dir().join(format!("fontcraft-mcp-policy-{name}-{}", std::process::id()));
        let inside = base.join("work");
        std::fs::create_dir_all(&inside).unwrap();
        let p = Policy::unrestricted().root(&inside).unwrap().base_dir(&inside);
        let root = std::fs::canonicalize(&inside).unwrap();
        (Dir(base), root, p)
    }

    #[test]
    fn glob_matches_whole_ids() {
        assert!(glob("glyph.*", "glyph.newGlyph"));
        assert!(!glob("glyph.*", "font.glyph.x"));
        assert!(glob("*.list*", "font.master.list"));
        assert!(glob("kerning.?etPair", "kerning.setPair"));
        assert!(!glob("file.save", "file.saveAs"));
        assert!(glob("*", ""));
        assert!(!glob("a*b*c", &"ab".repeat(200)));
    }

    #[test]
    fn deny_beats_allow() {
        let p = Policy::unrestricted().allow("glyph.*").unwrap().deny("glyph.remove*").unwrap();
        assert!(p.admit("glyph.newGlyph", None).is_ok());
        assert!(p.admit("glyph.removeGlyph", None).unwrap_err().contains("--deny"));
        assert!(p.admit("font.info.set", None).unwrap_err().contains("--allow"));
    }

    #[test]
    fn read_only_admits_only_queries_that_write_nothing_and_restrictions_refuse_macros() {
        let p = Policy::unrestricted().read_only(true);
        assert!(p.admit("glyph.getInfo", Some(&facts(true, false, &[]))).is_ok());
        assert!(p.admit("glyph.newGlyph", Some(&facts(false, false, &[]))).is_err());
        assert!(p.admit("made.up", None).is_err());
        // A query that writes files: flagged, or with a parameter it writes to.
        assert!(p.admit("tables.delete", Some(&facts(true, true, &[("font", false, FileForm::Path)]))).unwrap_err().contains("writes files"));
        assert!(p.admit("font.dumpTables", Some(&facts(true, false, &[("output", true, FileForm::Path)]))).is_err());
        assert!(p.admit("tables.list", Some(&facts(true, false, &[("font", false, FileForm::Path)]))).is_ok());
        assert!(Policy::unrestricted().admit("script.run", None).is_ok());
        assert!(Policy::unrestricted().deny("x").unwrap().admit("script.run", None).is_err());
    }

    #[test]
    fn the_file_system_root_leaves_everything_unrestricted() {
        let p = Policy::unrestricted().root(Path::new("/")).unwrap();
        assert!(!p.is_restricted() && p.describe().is_none());
        assert!(p.admit("script.run", None).is_ok());
        let mut params = json!({"path": "/etc/passwd"});
        p.check_params(&facts(false, true, &[("path", true, FileForm::Path)]), &mut params, &[]).unwrap();
        assert_eq!(params["path"], "/etc/passwd");
        // With another restriction, macros are refused again.
        assert!(Policy::unrestricted().root(Path::new("/")).unwrap().read_only(true).admit("script.run", None).is_err());
    }

    #[test]
    fn roots_need_the_catalogue_entry() {
        let (_dir, _, p) = rooted("entry");
        assert!(p.admit("made.up", None).unwrap_err().contains("catalogue"));
        let unreadable = CommandFacts { unreadable: true, ..CommandFacts::default() };
        assert!(p.admit("x.y", Some(&unreadable)).is_err());
        let (_, read) =
            CommandFacts::from_entry(&json!({"id": "a.b", "query": true, "file_params": [{"name": "font", "access": "read", "form": "path"}]}))
                .unwrap();
        assert_eq!(read, facts(true, false, &[("font", false, FileForm::Path)]));
        let (_, odd) = CommandFacts::from_entry(&json!({"id": "a.b", "file_params": [{"name": "x", "access": "?", "form": "?"}]})).unwrap();
        assert!(odd.files[0].write && odd.files[0].form == FileForm::Unknown);
        assert!(CommandFacts::from_entry(&json!({"id": "a.b", "file_params": "x"})).unwrap().1.unreadable);
    }

    #[test]
    fn declared_paths_outside_the_roots_are_refused_and_inside_ones_resolved() {
        let (_dir, root, p) = rooted("paths");
        let f = facts(false, true, &[("destination", true, FileForm::Path), ("files", false, FileForm::Path), ("index", false, FileForm::Path)]);
        let mut params = json!({"destination": "out", "files": ["a.svg", "sub/b.svg"], "other": "/etc/passwd"});
        p.check_params(&f, &mut params, &[]).unwrap();
        assert_eq!(params["destination"], json!(root.join("out").to_str().unwrap()));
        assert_eq!(params["files"][1], json!(root.join("sub/b.svg").to_str().unwrap()));
        assert_eq!(params["other"], "/etc/passwd", "undeclared parameters are not paths");
        for outside in ["../x.ttf", "/etc/passwd", "missing/../../x.ttf"] {
            assert!(p.check_params(&f, &mut json!({"destination": outside}), &[]).is_err(), "{outside}");
            assert!(p.check_params(&f, &mut json!({"files": ["a", outside]}), &[]).is_err(), "{outside}");
        }
        // A declared parameter holding something that is not a path is refused.
        assert!(p.check_params(&f, &mut json!({"index": 3}), &[]).is_err());
        assert!(p.check_params(&f, &mut json!({"files": [["nested"]]}), &[]).is_err());
        assert!(p.check_implicit_path("/etc/fonts/x.glyphs", "the document's file").is_err());
        assert!(p.check_implicit_path(root.join("x.glyphs").to_str().unwrap(), "the document's file").is_ok());
    }

    #[test]
    fn table_items_name_files_after_the_tag() {
        let (_dir, root, p) = rooted("tables");
        let f = facts(true, true, &[("tables", true, FileForm::Tables)]);
        for refused in [
            json!(["head=/outside/x"]),
            json!("head=/outside/x"),
            json!(["GPOS,head=../x"]),
            json!([{"tag": "head", "file": "/outside"}]),
            json!([{"tag": "head", "from": {"font": "/outside.ttf"}}]),
            json!([7]),
        ] {
            assert!(p.check_params(&f, &mut json!({"tables": refused.clone()}), &[]).is_err(), "{refused}");
        }
        let mut params = json!({"tables": ["GPOS", "head=h.bin,cmap", {"tag": "name", "file": "n.bin"}], "font": "/elsewhere"});
        p.check_params(&f, &mut params, &[]).unwrap();
        assert_eq!(params["tables"][0], "GPOS");
        assert_eq!(params["tables"][1], json!(format!("head={},cmap", root.join("h.bin").display())));
        assert_eq!(params["tables"][2]["file"], json!(root.join("n.bin").to_str().unwrap()));
    }

    #[test]
    fn keywords_and_open_documents_are_not_paths() {
        let (_dir, root, p) = rooted("keywords");
        let mut namelist = facts(false, false, &[("namelist", false, FileForm::Path)]);
        namelist.files[0].keywords = vec!["nice".into()];
        let mut params = json!({"namelist": "nice"});
        p.check_params(&namelist, &mut params, &[]).unwrap();
        assert_eq!(params["namelist"], "nice");
        assert!(p.check_params(&namelist, &mut json!({"namelist": "/etc/names"}), &[]).is_err());
        let doc = facts(true, false, &[("a", false, FileForm::Document)]);
        let open = ["Untitled".to_string(), "/elsewhere/A.glyphs".to_string()];
        for name in ["Untitled", "/elsewhere/A.glyphs"] {
            let mut params = json!({"a": name});
            p.check_params(&doc, &mut params, &open).unwrap();
            assert_eq!(params["a"], name);
        }
        assert!(p.check_params(&doc, &mut json!({"a": "/elsewhere/B.glyphs"}), &open).is_err());
        let mut params = json!({"a": "B.glyphs"});
        p.check_params(&doc, &mut params, &open).unwrap();
        assert_eq!(params["a"], json!(root.join("B.glyphs").to_str().unwrap()));
    }

    #[test]
    fn hidden_places_are_named() {
        assert_eq!(implicit_place("file.save", &json!({})), Some(Implicit::Documents));
        assert_eq!(implicit_place("file.save", &json!({"path": "x"})), None);
        assert_eq!(implicit_place("project.save", &json!({"path": null})), Some(Implicit::ProjectFiles));
        assert_eq!(implicit_place("project.export", &json!({})), Some(Implicit::ProjectExports));
        assert_eq!(implicit_place("file.exportWithProfiles", &json!({"bySource": true})), Some(Implicit::Documents));
        assert_eq!(implicit_place("file.exportWithProfiles", &json!({"bySource": false})), None);
        assert!(matches!(implicit_place("script.saveActionSet", &json!({"name": "m"})), Some(Implicit::Hidden(_))));
        assert!(matches!(implicit_place("script.recordMacro", &json!({"action": "stop", "name": "m"})), Some(Implicit::Hidden(_))));
        assert_eq!(implicit_place("script.recordMacro", &json!({"action": "stop"})), None);
    }
}
