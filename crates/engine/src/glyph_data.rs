//! The glyph data a document's commands read (`docs/spec/05-font-view.md` § 4.6).
//!
//! Layers, later ones winning per record: the built-in database; the user's custom files
//! (`GlyphData*.xml` in the settings folder, which the app hands to
//! [`crate::Session::set_user_glyph_data`]); the files beside the document's source, unless the
//! font's *Ignore custom GlyphData file* parameter is on. A malformed file is skipped and
//! reported; the others still apply.

use std::fmt;
use std::sync::{Arc, OnceLock};

use fontcraft_model::glyphdata::{GlyphData, MAX_CUSTOM_FILES, Record, xml};
use fontcraft_model::{Font, param_flag};

use crate::{EngineError, Result};

/// The parameter that turns off the files beside the source.
pub const IGNORE_PARAM: &str = "Ignore custom GlyphData file";
/// Most custom files of one kind (the user's, or one document's); together they stay within
/// what the database merges.
pub const MAX_FILES: usize = MAX_CUSTOM_FILES / 2;

/// The built-in database, shared.
pub(crate) fn builtin() -> Result<Arc<GlyphData>> {
    static BUILTIN: OnceLock<std::result::Result<Arc<GlyphData>, String>> = OnceLock::new();
    match BUILTIN.get_or_init(|| GlyphData::new().map(Arc::new).map_err(|e| e.to_string())) {
        Ok(d) => Ok(d.clone()),
        Err(e) => Err(EngineError::Internal(e.clone())),
    }
}

/// Parsed custom files and, when there are any, the database merged with them.
#[derive(Clone, Default)]
pub(crate) struct CustomData {
    pub(crate) files: Arc<Vec<Vec<Record>>>,
    pub(crate) merged: Option<Arc<GlyphData>>,
}

impl fmt::Debug for CustomData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomData").field("files", &self.files.len()).field("merged", &self.merged.is_some()).finish()
    }
}

/// Parses `files` (name, bytes), skipping and reporting the malformed ones and those past
/// [`MAX_FILES`].
pub(crate) fn parse_files(files: &[(&str, &[u8])], problems: &mut Vec<String>) -> Vec<Vec<Record>> {
    let mut out = Vec::new();
    for (i, (name, bytes)) in files.iter().enumerate() {
        if i >= MAX_FILES {
            problems.push(format!("glyph data file {name}: skipped, more than {MAX_FILES} files"));
            continue;
        }
        match xml::parse(bytes) {
            Ok(parsed) => {
                problems.extend(parsed.warnings.into_iter().map(|w| format!("glyph data file {name}: {w}")));
                out.push(parsed.records);
            }
            Err(e) => problems.push(format!("glyph data file {name} is ignored: {e}")),
        }
    }
    out
}

/// The database with `layers` on top; `None` without layers or when merging fails (reported).
pub(crate) fn merge(layers: &[&[Vec<Record>]], problems: &mut Vec<String>) -> Option<Arc<GlyphData>> {
    let all: Vec<Vec<Record>> = layers.iter().flat_map(|l| l.iter().cloned()).collect();
    if all.is_empty() {
        return None;
    }
    match GlyphData::with_custom(all) {
        Ok(d) => Some(Arc::new(d)),
        Err(e) => {
            problems.push(format!("custom glyph data is ignored: {e}"));
            None
        }
    }
}

/// Whether the font turns off the glyph data files beside its source.
pub fn ignores_source_files(font: &Font) -> bool {
    font.info.params.get(IGNORE_PARAM).and_then(param_flag) == Some(true)
}

/// The `GlyphData*.xml` files beside a source and in the `Info` folder beside it, by name, in
/// that order. Unreadable files are reported, not fatal.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn source_files(path: &std::path::Path, problems: &mut Vec<String>) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let Some(dir) = path.parent() else { return Vec::new() };
    let mut out = Vec::new();
    for folder in [dir.to_path_buf(), dir.join("Info")] {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("GlyphData") && n.ends_with(".xml"))
            .take(MAX_FILES + 1)
            .collect();
        names.sort();
        for name in names {
            let file = folder.join(&name);
            if !std::fs::metadata(&file).is_ok_and(|m| m.is_file()) {
                continue;
            }
            let mut bytes = Vec::new();
            let limit = xml::MAX_FILE_BYTES as u64;
            match std::fs::File::open(&file).and_then(|f| f.take(limit + 1).read_to_end(&mut bytes)) {
                Ok(_) => out.push((file.display().to_string(), bytes)),
                Err(e) => problems.push(format!("glyph data file {} cannot be read: {e}", file.display())),
            }
        }
    }
    out
}
