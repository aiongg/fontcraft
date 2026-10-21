//! The file checks a command passes before it runs while automation roots are in force.
//!
//! What a command reads and writes comes from its declarations ([`CommandSpec::files`],
//! [`CommandSpec::writes_files`]) and, for the places no parameter names, from
//! [`implicit_place`]. Every check fails closed: a value that cannot be read as a path, a path
//! that cannot be resolved, a place that cannot be seen is refused, never run.

use std::path::Path;

use serde_json::Value;

use super::{Access, AutomationRoots};
use crate::cmd::{FileAccess, FileForm, FileParam};
use crate::{CommandSpec, EngineError, Result, Session};

/// Most paths checked for one command (one step of a batch).
pub const MAX_PATHS: usize = 100_000;

/// Commands that run other commands or open files no parameter names (macro files, recent
/// documents, URLs). They are refused while roots are in force.
pub const INDIRECT_COMMANDS: &[&str] =
    &["script.run", "script.rerunLast", "script.runActionSet", "script.repeatLastAction", "file.openRecent", "url.open", "url.show"];

/// Where a command writes, or what it reads, without a parameter naming the place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Implicit {
    /// The open documents' own files, or the folders that hold them.
    Documents,
    /// The open projects' own files.
    ProjectFiles,
    /// The open projects' export destinations and the font files they link.
    ProjectExports,
    /// The session's snippet folder, read or written.
    SnippetFolder(Access),
    /// A place that cannot be checked; the reason, after the command id.
    Hidden(&'static str),
}

/// The implicit place `id` writes to with these parameters, if any.
fn implicit_place(id: &str, params: &Value) -> Option<Implicit> {
    let given = |key: &str| params.get(key).is_some_and(|v| !v.is_null());
    match id {
        "file.save" if !given("path") => Some(Implicit::Documents),
        "file.exportWithProfiles" if given("bySource") && params.get("bySource") != Some(&Value::Bool(false)) => Some(Implicit::Documents),
        "project.save" if !given("path") => Some(Implicit::ProjectFiles),
        "project.export" => Some(Implicit::ProjectExports),
        "font.features.listSnippets" | "font.features.insertSnippet" if !given("folder") => Some(Implicit::SnippetFolder(Access::Read)),
        "font.features.showSnippetFolder" if !given("folder") => Some(Implicit::SnippetFolder(Access::Write)),
        "script.saveActionSet" if !given("path") => Some(Implicit::Hidden("writes into the Macros folder without `path`; give `path`")),
        "script.recordMacro" if given("name") => Some(Implicit::Hidden(
            "writes the recording into the Macros folder when `name` is given; stop without `name` and save the steps with script.saveActionSet and `path`",
        )),
        _ => None,
    }
}

/// Checks command `spec` with `params` against `roots`. Returns the parameters to run it with:
/// each file parameter replaced by the absolute path that was checked, so the command opens
/// exactly that file.
pub(crate) fn check(s: &Session, spec: &CommandSpec, params: &Value, roots: &AutomationRoots) -> Result<Value> {
    let id = spec.id;
    let refuse = |message: String| EngineError::BadParams { command: id.to_string(), message };
    if INDIRECT_COMMANDS.contains(&id) {
        return Err(refuse(
            "runs steps or opens files that cannot be checked against the automation roots; run the steps one by one, or with command.batch".into(),
        ));
    }
    if let Some(place) = implicit_place(id, params) {
        check_implicit(s, place, roots).map_err(refuse)?;
    }
    check_files(s, id, spec.files, params, roots).map_err(refuse)
}

/// Checks the parameters `files` declares for command `id` against `roots`, and the files each
/// file it reads reads in turn. Returns the parameters with each path replaced by the absolute
/// path that was checked. For surfaces with commands of their own (the desktop app's shell).
pub fn check_files(s: &Session, id: &str, files: &[FileParam], params: &Value, roots: &AutomationRoots) -> std::result::Result<Value, String> {
    let mut params = params.clone();
    if let Value::Object(map) = &mut params {
        let open: Vec<&str> =
            s.documents().iter().flat_map(|d| [Some(d.title.as_str()), d.path.as_deref().and_then(Path::to_str)]).flatten().collect();
        let folder =
            s.active().and_then(|d| d.path.as_deref()).and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.parent().map(Path::to_path_buf));
        let mut checker = Checker { roots, open: &open, left: MAX_PATHS, access: Access::Read, base: None, reads: Vec::new() };
        for f in files {
            if let Some(value) = map.get_mut(f.name) {
                checker.access = match f.access {
                    FileAccess::Read => Access::Read,
                    FileAccess::Write => Access::Write,
                };
                checker.base = folder.as_deref().filter(|_| f.from_document);
                checker.value(f, value)?;
            }
        }
        // One source named many times is opened once.
        checker.reads.sort();
        checker.reads.dedup();
        for path in &checker.reads {
            check_references(path, roots)?;
            if id == "project.open" || (id == "file.open" && crate::project::is_project_file(path)) {
                check_project_link(path, roots)?;
            }
        }
    }
    Ok(params)
}

/// Checks a place a command writes to, or reads, without a parameter naming it. Every open
/// document or project is checked, since a step of a batch can change which one is active.
fn check_implicit(s: &Session, place: Implicit, roots: &AutomationRoots) -> std::result::Result<(), String> {
    match place {
        Implicit::Hidden(why) => Err(format!("{why}: automation file access is confined")),
        Implicit::Documents => {
            for path in s.documents().iter().filter_map(|d| d.path.as_deref()) {
                implicit(roots, path, Access::Write, "writes to or beside the open document's file")?;
            }
            Ok(())
        }
        Implicit::ProjectFiles => {
            for path in s.projects().iter().filter_map(|p| p.path.as_deref()) {
                implicit(roots, path, Access::Write, "writes the project's file")?;
            }
            Ok(())
        }
        Implicit::ProjectExports => {
            for project in s.projects() {
                if let Some(dest) = project.data.destination.as_deref() {
                    implicit(roots, dest, Access::Write, "writes into the project's export destination")?;
                }
                if let Some(font) = project.data.font.as_deref() {
                    implicit(roots, font, Access::Read, "reads the project's linked font")?;
                    check_references(font, roots)?;
                }
            }
            Ok(())
        }
        Implicit::SnippetFolder(access) => match s.snippets_dir() {
            Some(dir) => implicit(roots, dir, access, "uses the session's snippet folder (give `folder`)"),
            None => Ok(()),
        },
    }
}

fn implicit(roots: &AutomationRoots, path: &Path, access: Access, what: &str) -> std::result::Result<(), String> {
    let text = path.to_str().ok_or_else(|| format!("{what}, whose path is not UTF-8"))?;
    roots.check(text, access).map_err(|e| format!("{what}: {e}"))
}

/// Checks the files that reading the source at `path` reads in turn (a designspace's UFOs, the
/// Import Font and Import Master files) against the read root, before any of them is read.
#[cfg(not(target_arch = "wasm32"))]
fn check_references(path: &Path, roots: &AutomationRoots) -> std::result::Result<(), String> {
    let reader = path.display().to_string();
    crate::io::check_references(path, &mut |p| implicit(roots, p, Access::Read, &format!("reads a file through {reader}")))
}

#[cfg(target_arch = "wasm32")]
fn check_references(_: &Path, _: &AutomationRoots) -> std::result::Result<(), String> {
    Ok(())
}

/// Checks the font a project file links, which `project.open` reads, and the files that font
/// reads in turn.
#[cfg(not(target_arch = "wasm32"))]
fn check_project_link(project: &Path, roots: &AutomationRoots) -> std::result::Result<(), String> {
    // A project file that cannot be read links nothing; project.open then fails on its own.
    let Ok(Some(font)) = crate::project::linked_font(project) else { return Ok(()) };
    implicit(roots, &font, Access::Read, "reads the project's linked font")?;
    check_references(&font, roots)
}

#[cfg(target_arch = "wasm32")]
fn check_project_link(_: &Path, _: &AutomationRoots) -> std::result::Result<(), String> {
    Ok(())
}

/// Checks the values of one command's file parameters.
struct Checker<'a> {
    roots: &'a AutomationRoots,
    /// The titles and paths of the open documents, which a document parameter may name.
    open: &'a [&'a str],
    /// Paths still allowed before the command is refused ([`MAX_PATHS`]).
    left: usize,
    /// The access the parameter being checked declares.
    access: Access,
    /// The folder a relative path of the parameter being checked starts from, when not the
    /// working directory ([`FileParam::from_document`]).
    base: Option<&'a Path>,
    /// The resolved paths of the files the command reads.
    reads: Vec<std::path::PathBuf>,
}

impl Checker<'_> {
    /// A parameter's value: one item or a list of items.
    fn value(&mut self, f: &FileParam, value: &mut Value) -> std::result::Result<(), String> {
        match value {
            Value::Null => Ok(()),
            Value::Array(items) => items.iter_mut().try_for_each(|item| self.item(f, item)),
            other => self.item(f, other),
        }
    }

    fn item(&mut self, f: &FileParam, item: &mut Value) -> std::result::Result<(), String> {
        let key = f.name;
        match (f.form, item) {
            (_, Value::String(text)) if f.keywords.contains(&text.as_str()) => Ok(()),
            (FileForm::Path, Value::String(text)) => {
                *text = self.path(key, text)?;
                Ok(())
            }
            (FileForm::Document, Value::String(text)) => {
                if !self.open.contains(&text.as_str()) {
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
                    Some(_) => Err(format!("`{key}`: `from` holds a value that cannot be checked as a path")),
                }
            }
            _ => Err(format!("`{key}` holds a value that cannot be checked as a path")),
        }
    }

    fn optional_path(&mut self, key: &str, value: &mut Value) -> std::result::Result<(), String> {
        match value {
            Value::Null => Ok(()),
            Value::String(text) => {
                *text = self.path(key, text)?;
                Ok(())
            }
            _ => Err(format!("`{key}` holds a value that cannot be checked as a path")),
        }
    }

    /// `"TAG"`, `"TAG=FILE"`, or several of them separated by commas, as the table commands
    /// read them.
    fn table_list(&mut self, key: &str, text: &str) -> std::result::Result<String, String> {
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

    fn path(&mut self, key: &str, text: &str) -> std::result::Result<String, String> {
        self.left = self.left.checked_sub(1).ok_or_else(|| format!("more than {MAX_PATHS} paths in one command"))?;
        let joined = match self.base {
            Some(base) if Path::new(text).is_relative() => Some(base.join(text)),
            _ => None,
        };
        let text = match &joined {
            Some(joined) => joined.to_str().ok_or_else(|| format!("`{key}`: {text} in the document's folder is not a UTF-8 path"))?,
            None => text,
        };
        let resolved = self.roots.resolved(text, self.access).map_err(|e| format!("`{key}`: {e}"))?;
        if self.access == Access::Read {
            self.reads.push(resolved.clone());
        }
        resolved.to_str().map(str::to_string).ok_or_else(|| format!("`{key}`: {text} does not resolve to a UTF-8 path"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use serde_json::json;

    /// `in` is both roots; `outside` is not.
    fn roots(name: &str) -> (std::path::PathBuf, std::sync::Arc<AutomationRoots>) {
        let base = std::env::temp_dir().join(format!("fontcraft-access-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("in")).unwrap();
        let inside = std::fs::canonicalize(base.join("in")).unwrap();
        let roots = AutomationRoots::new(Some(&inside), Some(&inside)).unwrap().unwrap();
        (inside, roots)
    }

    fn check(roots: &AutomationRoots, open: &[&str], f: &FileParam, mut value: Value) -> std::result::Result<Value, String> {
        let access = if f.access == FileAccess::Read { Access::Read } else { Access::Write };
        let mut checker = Checker { roots, open, left: MAX_PATHS, access, base: None, reads: Vec::new() };
        checker.value(f, &mut value).map(|()| value)
    }

    #[test]
    fn table_items_keywords_and_open_documents() {
        let (inside, roots) = roots("forms");
        let file = |n: &str| inside.join(n).to_str().unwrap().to_string();
        let tables = FileParam::write("tables").tables();
        for refused in [
            json!(["head=/outside/x"]),
            json!("head=/outside/x"),
            json!(["GPOS,head=../x"]),
            json!([{"tag": "head", "file": "/outside"}]),
            json!([{"tag": "head", "from": {"font": "/outside.ttf"}}]),
            json!([7]),
            json!([["nested"]]),
        ] {
            assert!(check(&roots, &[], &tables, refused.clone()).is_err(), "{refused}");
        }
        let given = json!(["GPOS", format!("head={},cmap", file("h.bin")), {"tag": "name", "file": file("n.bin")}]);
        let checked = check(&roots, &[], &tables, given.clone()).unwrap();
        assert_eq!(checked, given, "paths inside the root are kept, absolute");

        let names = FileParam::read("namelist").keywords(&["nice"]);
        assert_eq!(check(&roots, &[], &names, json!("nice")).unwrap(), json!("nice"));
        assert!(check(&roots, &[], &names, json!("/etc/names")).unwrap_err().contains("outside the read root"));

        let doc = FileParam::read("a").document();
        let open = ["Untitled", "/elsewhere/A.glyphs"];
        for name in open {
            assert_eq!(check(&roots, &open, &doc, json!(name)).unwrap(), json!(name), "an open document is no file");
        }
        assert!(check(&roots, &open, &doc, json!("/elsewhere/B.glyphs")).is_err());
        assert!(check(&roots, &[], &FileParam::read("index"), json!(3)).unwrap_err().contains("cannot be checked as a path"));
        let _ = std::fs::remove_dir_all(inside.parent().unwrap());
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
        assert_eq!(implicit_place("font.features.listSnippets", &json!({})), Some(Implicit::SnippetFolder(Access::Read)));
        assert_eq!(implicit_place("font.features.showSnippetFolder", &json!({})), Some(Implicit::SnippetFolder(Access::Write)));
        assert_eq!(implicit_place("font.features.insertSnippet", &json!({"folder": "/x"})), None);
    }
}
