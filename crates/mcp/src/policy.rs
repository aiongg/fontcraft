//! What an MCP client may do: `--read-only`, `--allow` and `--deny` patterns over command ids, and
//! `--root` folders for file parameters.
//!
//! Every check fails closed: a command the policy cannot place (an unknown id under
//! `--read-only`, a macro whose steps it cannot see, a path it cannot resolve) is refused, never
//! run.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Most `--allow` or `--deny` patterns.
pub const MAX_PATTERNS: usize = 256;
/// Longest pattern, in bytes.
pub const MAX_PATTERN_LEN: usize = 256;
/// Most `--root` folders.
pub const MAX_ROOTS: usize = 64;
/// Deepest nesting of a command's parameters the path check walks; deeper parameters are refused.
pub const MAX_PARAM_DEPTH: usize = 128;
/// Deepest nesting of `command.batch` inside `command.batch`.
pub const MAX_BATCH_NESTING: usize = 8;

/// Parameter names whose text values (or lists of texts) are file or folder paths.
pub const FILE_KEYS: &[&str] =
    &["path", "paths", "file", "files", "folder", "folders", "destination", "directory", "font", "fonts", "input", "inputs", "output", "outputDir"];

/// Commands that run other commands or open files the server cannot see in their parameters
/// (macro files, recent-document lists, URLs). They are refused whenever the policy restricts
/// anything.
pub const INDIRECT_COMMANDS: &[&str] =
    &["script.run", "script.rerunLast", "script.runActionSet", "script.repeatLastAction", "file.openRecent", "url.open", "url.show"];

/// Commands that write to the active document's own file when no path parameter is given.
pub const IMPLICIT_FILE_COMMANDS: &[&str] = &["file.save"];

/// The batch command: its steps are checked one by one.
pub const BATCH_COMMAND: &str = "command.batch";

#[derive(Clone, Debug, Default)]
pub struct Policy {
    read_only: bool,
    allow: Vec<String>,
    deny: Vec<String>,
    /// Canonical folders; `None` admits every path.
    roots: Option<Vec<PathBuf>>,
    /// Where relative paths resolve; the working directory when unset.
    base: Option<PathBuf>,
}

impl Policy {
    /// Admits every command and every path.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Admits only commands that declare themselves queries.
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

    /// Adds a `--root` folder. File parameters must then lie inside one of the roots.
    pub fn root(mut self, dir: &Path) -> Result<Self, String> {
        let roots = self.roots.get_or_insert_with(Vec::new);
        if roots.len() >= MAX_ROOTS {
            return Err(format!("at most {MAX_ROOTS} roots"));
        }
        let canonical = std::fs::canonicalize(dir).map_err(|e| format!("root `{}`: {e}", dir.display()))?;
        if !canonical.is_dir() {
            return Err(format!("root `{}` is not a folder", dir.display()));
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

    /// Whether anything at all is restricted.
    pub fn is_restricted(&self) -> bool {
        self.restricts_commands() || self.roots.is_some()
    }

    /// Whether the command check needs to know which commands are queries.
    pub(crate) fn needs_catalogue(&self) -> bool {
        self.read_only
    }

    /// One paragraph for `initialize`'s instructions, or `None` when nothing is restricted.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.read_only {
            parts.push("only commands that read (queries) run".to_string());
        }
        if !self.allow.is_empty() {
            parts.push(format!("only commands matching {} run", self.allow.join(", ")));
        }
        if !self.deny.is_empty() {
            parts.push(format!("commands matching {} are refused", self.deny.join(", ")));
        }
        if let Some(roots) = &self.roots {
            let list: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
            parts.push(format!("file parameters must lie inside {}", list.join(", ")));
        }
        if self.is_restricted() {
            parts.push("macros and other commands that run steps this server cannot see are refused".to_string());
        }
        (!parts.is_empty()).then(|| format!("This server is restricted: {}.", parts.join("; ")))
    }

    /// Decides one command by id. `query` is the catalogue's flag for it (`None`: not in the
    /// catalogue, or the catalogue was not read).
    pub fn admit(&self, id: &str, query: Option<bool>) -> Result<(), String> {
        if let Some(p) = self.deny.iter().find(|p| glob(p, id)) {
            return Err(format!("`{id}` is refused by this server (--deny {p})"));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|p| glob(p, id)) {
            return Err(format!("`{id}` is not among the commands this server allows (--allow {})", self.allow.join(", ")));
        }
        if self.read_only && query != Some(true) {
            return Err(match query {
                Some(_) => format!("`{id}` changes documents, files or settings, and this server is read-only"),
                None => format!("`{id}` is not a known query, and this server is read-only"),
            });
        }
        if self.is_restricted() && INDIRECT_COMMANDS.contains(&id) {
            return Err(format!(
                "`{id}` runs steps or opens files this server cannot check against its restrictions; run the steps one by one, or with command.batch"
            ));
        }
        Ok(())
    }

    /// Checks every file parameter against the roots and replaces each with its resolved absolute
    /// path, so the command runs on exactly the path that was checked (and the app, whose working
    /// directory may differ, reads the same file).
    pub fn check_paths(&self, params: &mut Value) -> Result<(), String> {
        let Some(roots) = &self.roots else { return Ok(()) };
        let base = self.base()?;
        walk(params, 0, &mut |key, text| {
            let resolved = resolve(&base, text).map_err(|e| format!("`{key}`: {e}"))?;
            if !roots.iter().any(|r| resolved.starts_with(r)) {
                return Err(format!("`{key}`: {text} lies outside the folders this server may use (--root)"));
            }
            resolved.to_str().map(str::to_string).ok_or_else(|| format!("`{key}`: {text} does not resolve to a UTF-8 path"))
        })
    }

    /// Checks one path the command writes without naming it (the active document's file).
    pub fn check_implicit_path(&self, path: &str) -> Result<(), String> {
        let Some(roots) = &self.roots else { return Ok(()) };
        let resolved = resolve(&self.base()?, path)?;
        if roots.iter().any(|r| resolved.starts_with(r)) {
            Ok(())
        } else {
            Err(format!("the document's file {path} lies outside the folders this server may use (--root); give `path`"))
        }
    }

    fn base(&self) -> Result<PathBuf, String> {
        match &self.base {
            Some(b) => Ok(b.clone()),
            None => std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}")),
        }
    }

    pub(crate) fn has_roots(&self) -> bool {
        self.roots.is_some()
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

/// Calls `check` on every text under a file key, replacing it with what `check` returns.
fn walk(value: &mut Value, depth: usize, check: &mut dyn FnMut(&str, &str) -> Result<String, String>) -> Result<(), String> {
    if depth > MAX_PARAM_DEPTH {
        return Err(format!("parameters nest deeper than {MAX_PARAM_DEPTH} levels"));
    }
    match value {
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if FILE_KEYS.contains(&key.as_str()) {
                    match v {
                        Value::String(text) => *text = check(key.as_str(), text.as_str())?,
                        Value::Array(items) => {
                            for item in items.iter_mut() {
                                match item {
                                    Value::String(text) => *text = check(key.as_str(), text.as_str())?,
                                    other => walk(other, depth + 1, check)?,
                                }
                            }
                        }
                        other => walk(other, depth + 1, check)?,
                    }
                } else {
                    walk(v, depth + 1, check)?;
                }
            }
            Ok(())
        }
        Value::Array(items) => items.iter_mut().try_for_each(|v| walk(v, depth + 1, check)),
        _ => Ok(()),
    }
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
    fn read_only_admits_only_known_queries_and_restrictions_refuse_macros() {
        let p = Policy::unrestricted().read_only(true);
        assert!(p.admit("glyph.getInfo", Some(true)).is_ok());
        assert!(p.admit("glyph.newGlyph", Some(false)).is_err());
        assert!(p.admit("made.up", None).is_err());
        assert!(Policy::unrestricted().admit("script.run", None).is_ok());
        assert!(Policy::unrestricted().deny("x").unwrap().admit("script.run", None).is_err());
    }

    #[test]
    fn paths_outside_the_roots_are_refused_and_inside_ones_resolved() {
        let base = std::env::temp_dir().join(format!("fontcraft-mcp-policy-{}", std::process::id()));
        let inside = base.join("work");
        std::fs::create_dir_all(&inside).unwrap();
        let p = Policy::unrestricted().root(&inside).unwrap().base_dir(&inside);
        let root = std::fs::canonicalize(&inside).unwrap();

        let mut params = json!({"destination": "out", "steps": [{"params": {"files": ["a.svg", "sub/b.svg"]}}]});
        p.check_paths(&mut params).unwrap();
        assert_eq!(params["destination"], json!(root.join("out").to_str().unwrap()));
        assert_eq!(params["steps"][0]["params"]["files"][1], json!(root.join("sub/b.svg").to_str().unwrap()));

        for outside in ["../x.ttf", "/etc/passwd", "missing/../../x.ttf"] {
            let mut params = json!({"path": outside});
            assert!(p.check_paths(&mut params).is_err(), "{outside}");
        }
        // Numbers under a file key are not paths (path indexes in outline commands).
        assert!(p.check_paths(&mut json!({"path": 3})).is_ok());
        assert!(p.check_implicit_path("/etc/fonts/x.glyphs").is_err());
        assert!(p.check_implicit_path(root.join("x.glyphs").to_str().unwrap()).is_ok());
        std::fs::remove_dir_all(&base).ok();
    }
}
