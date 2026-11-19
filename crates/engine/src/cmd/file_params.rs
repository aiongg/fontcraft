//! What a command declares about the files it touches: the parameters that hold paths, and
//! whether it writes to disk. The automation roots (`crate::file_access`) and the MCP server's
//! `--read-only` read these declarations, so every parameter a command reads a path from must be
//! declared here.

use serde::Serialize;

/// Whether a command reads or writes the file a parameter names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileAccess {
    Read,
    Write,
}

/// How a parameter's value holds its paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileForm {
    /// A path, or a list of paths.
    Path,
    /// Table items: `"TAG"` (no file), `"TAG=FILE"` (several comma-separated in one string), or
    /// `{tag, file?, from?: {font}}`, where `file` and `from.font` are paths. A list or one item.
    Tables,
    /// An open document's title or path; anything else is a file read from disk. A list or one.
    Document,
}

/// One parameter that holds a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct FileParam {
    pub name: &'static str,
    pub access: FileAccess,
    pub form: FileForm,
    /// Values of the parameter that are not paths (`"nice"` for a list of names).
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub keywords: &'static [&'static str],
    /// A relative path is relative to the active document's folder (to the working directory
    /// for a document never saved), as the command reads it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_document: bool,
}

impl FileParam {
    /// A parameter naming a file or folder the command reads.
    pub const fn read(name: &'static str) -> Self {
        Self { name, access: FileAccess::Read, form: FileForm::Path, keywords: &[], from_document: false }
    }

    /// A parameter naming a file or folder the command writes (or stores as a place to write).
    pub const fn write(name: &'static str) -> Self {
        Self { name, access: FileAccess::Write, form: FileForm::Path, keywords: &[], from_document: false }
    }

    /// Sets [`FileParam::form`] to [`FileForm::Tables`].
    pub const fn tables(mut self) -> Self {
        self.form = FileForm::Tables;
        self
    }

    /// Sets [`FileParam::form`] to [`FileForm::Document`].
    pub const fn document(mut self) -> Self {
        self.form = FileForm::Document;
        self
    }

    /// Sets [`FileParam::from_document`].
    pub const fn from_document(mut self) -> Self {
        self.from_document = true;
        self
    }

    /// Sets [`FileParam::keywords`].
    pub const fn keywords(mut self, keywords: &'static [&'static str]) -> Self {
        self.keywords = keywords;
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::cmd::command_specs;

    /// Parameter names that usually hold a path.
    const PATH_NAMES: &[&str] = &[
        "path",
        "paths",
        "file",
        "files",
        "folder",
        "folders",
        "destination",
        "directory",
        "dir",
        "output",
        "outputs",
        "outputdir",
        "instancedir",
        "input",
        "inputs",
        "font",
        "fonts",
    ];

    /// Parameters whose documentation looks like a path but which name no file the command
    /// reads or writes. `*` at the end of an id matches any rest.
    const NOT_PATHS: &[(&str, &[&str], &str)] = &[
        ("path.nameContour", &["path"], "an outline path's index"),
        ("hint.tt.renderPreview", &["output"], "'outline' or 'bitmap'"),
        ("tables.show", &["path"], "a record path inside a table"),
        ("tables.add", &["from"], "inside a `tables` item, checked with the `tables` parameter"),
        (
            "font.merge",
            &["path", "map"],
            "inside a `fonts` item: the `fonts` parameter is checked, and an item given as an object is refused under automation roots",
        ),
        ("audit.export", &["target"], "only \"document\": files are not audited, so no file is read"),
        ("audit.clearCache", &["target"], "a file path clears nothing and reads no file"),
        ("fontView.*", &["entries", "entry", "folder", "parent", "filter"], "a sidebar entry's path, not a file"),
        ("compareFonts.*", &["left", "right", "document"], "an open document's position, title or path; no file is opened"),
        ("compareFamily.report", &["documents"], "open documents only; no file is opened"),
        ("project.*", &["project"], "an open project's id, path or title; no file is opened"),
        ("document.activate", &["path"], "an open document's path; no file is opened"),
        ("file.exportWithProfiles", &["fonts"], "open documents' titles or paths; no file is opened"),
        ("file.reviewExternalChanges", &["font"], "an open document, whose own file is read again"),
        ("file.saveExportProfile", &["folder", "settings"], "a subfolder of a later export's destination, never climbing out of it"),
        ("font.master.addFromOtherFont", &["sources", "font"], "an open document's position, title or path; no file is opened"),
        ("font.overlayFonts", &["font", "layers"], "inside a `layers` item: an open document's position, title or path; no file is opened"),
        ("font.overlayFonts", &["output"], "'colorLayers' or 'masters'"),
        ("hint.ps.*", &["nodes", "node"], "[outline path index, node index] pairs"),
        ("path.replaceOutlinePattern", &["replacement"], "outline paths given inline"),
        ("font.setCrossAxisMappings", &["input", "output"], "axis locations by tag"),
        (
            "font.compile",
            &["interpolateBinaryLayout"],
            "true or a folder relative to the source's folder; each master file read from it is checked against the automation roots when opened",
        ),
        (
            "font.instantiate",
            &["interpolateBinaryLayout"],
            "true or a folder relative to the source's folder; each master file read from it is checked against the automation roots when opened",
        ),
        ("plugin.installFromFile", &["dataBase64"], "the file's bytes; `path` is the declared one"),
        // Only a macro the session already lists in its Macros folder matches, and its file
        // moves to the Removed folder beside it; a path naming anything else is refused.
        ("pluginManager.remove", &["id"], "an installed module's id or a known macro's path or title"),
        // These run steps or open files the declarations cannot describe; a restricted MCP
        // server refuses them outright (its INDIRECT_COMMANDS).
        ("script.run", &["macro"], "refused whenever file access is restricted"),
        ("script.runActionSet", &["set"], "refused whenever file access is restricted"),
        ("url.open", &["files", "path"], "refused whenever file access is restricted"),
    ];

    fn not_a_path(id: &str, key: &str) -> bool {
        NOT_PATHS.iter().any(|(pattern, keys, _)| {
            let id_matches = match pattern.strip_suffix('*') {
                Some(prefix) => id.starts_with(prefix),
                None => id == *pattern,
            };
            id_matches && keys.contains(&key)
        })
    }

    /// The `name: type` pairs of the parameter part of a params doc (before `→`), nested ones
    /// included.
    fn documented_params(doc: &str) -> Vec<(String, String)> {
        let part = doc.split('→').next().unwrap_or(doc);
        let chars: Vec<char> = part.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{' || chars[i] == ',' {
                let mut j = i + 1;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                let start = j;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[start..j].iter().collect();
                if j < chars.len() && chars[j] == '?' {
                    j += 1;
                }
                let starts_with_letter = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                if starts_with_letter && j < chars.len() && chars[j] == ':' {
                    let rest: String = chars[j + 1..].iter().collect();
                    let ty = rest.trim_start().split([',', '}', '(']).next().unwrap_or("").trim().to_string();
                    out.push((name, ty));
                }
            }
            i += 1;
        }
        out
    }

    fn looks_like_a_path(name: &str, ty: &str) -> bool {
        let ty = ty.to_lowercase();
        let starts = ty.trim_start_matches('[');
        PATH_NAMES.contains(&name.to_lowercase().as_str())
            || ["path", "file", "folder", "director"].iter().any(|w| starts.starts_with(w))
            || ty.split(|c: char| matches!(c, '|' | '[' | ']') || c.is_whitespace()).any(|w| w == "path")
            || ty.contains("tag=file")
    }

    #[test]
    fn every_documented_path_parameter_is_declared() {
        let mut missing = Vec::new();
        for c in command_specs() {
            for (name, ty) in documented_params(c.params) {
                if name == "inPlace" {
                    if !c.writes_files {
                        missing.push(format!("{}: takes inPlace but does not declare writes_files", c.id));
                    }
                    continue;
                }
                if looks_like_a_path(&name, &ty) && !c.files.iter().any(|f| f.name == name) && !not_a_path(c.id, &name) {
                    missing.push(format!("{}: `{name}: {ty}`", c.id));
                }
            }
        }
        assert!(missing.is_empty(), "declare these as file parameters (.files) or list them in NOT_PATHS with the reason:\n{}", missing.join("\n"));
    }

    #[test]
    fn queries_that_write_files_say_so() {
        let wrong: Vec<&str> = command_specs()
            .iter()
            .filter(|c| !c.journal && !c.writes_files && c.files.iter().any(|f| f.access == FileAccess::Write))
            .map(|c| c.id)
            .collect();
        assert!(wrong.is_empty(), "queries with a parameter they write to must set writes_files: {wrong:?}");
    }

    #[test]
    fn the_catalogue_carries_the_declarations() {
        let s = crate::Session::new();
        let spec = command_specs().iter().find(|c| c.id == "tables.extract").unwrap();
        let info = serde_json::to_value(spec.info(&s)).unwrap();
        assert_eq!(info["writes_files"], true);
        let files = info["file_params"].as_array().unwrap();
        assert!(files.iter().any(|f| f["name"] == "tables" && f["access"] == "write" && f["form"] == "tables"));
        let plain = serde_json::to_value(command_specs().iter().find(|c| c.id == "glyph.newGlyph").unwrap().info(&s)).unwrap();
        assert!(plain.get("file_params").is_none() && plain.get("writes_files").is_none());
    }
}
