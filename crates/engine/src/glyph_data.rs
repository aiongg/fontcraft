//! The glyph data a document's commands read (`docs/spec/05-font-view.md` § 4.6).
//!
//! Layers, later ones winning per record: the built-in database; the user's custom files
//! (`GlyphData*.xml` in the `Info` folder of the settings folder, which the desktop app has read by
//! [`crate::Session::load_user_info`]); the files beside the document's source, unless the
//! font's *Ignore custom GlyphData file* parameter is on. A malformed file is skipped and
//! reported; the others still apply.
//!
//! Also here: the built-in language sets that Font View's Languages section lists
//! ([`language_sets`]), and the user's `Groups*.plist` files with extra Categories and Languages
//! entries ([`crate::Session::set_user_groups`]).

use std::fmt;
use std::sync::{Arc, OnceLock};

use fontcraft_glyphquery::{self as gq, GroupEntry, Groups, LanguageIndex, LanguageSet};
use fontcraft_model::glyphdata::{GlyphData, MAX_CUSTOM_FILES, Record, sets, xml};
use fontcraft_model::{Font, param_flag};

use crate::{EngineError, Result, Session};

/// The parameter that turns off the files beside the source.
pub const IGNORE_PARAM: &str = "Ignore custom GlyphData file";
/// Most custom files of one kind (the user's, or one document's); together they stay within
/// what the database merges.
pub const MAX_FILES: usize = MAX_CUSTOM_FILES / 2;

/// Largest `Groups*.plist` file read, in bytes.
pub const MAX_GROUPS_FILE_BYTES: usize = gq::MAX_GROUPS_BYTES;

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
    let Some(dir) = path.parent() else { return Vec::new() };
    let mut out = Vec::new();
    for folder in [dir.to_path_buf(), dir.join(INFO_FOLDER)] {
        out.extend(named_files(&folder, "GlyphData", ".xml", xml::MAX_FILE_BYTES, true, problems));
    }
    out
}

/// The folder of custom glyph data and groups files, beside a source and in the settings folder.
#[cfg(not(target_arch = "wasm32"))]
const INFO_FOLDER: &str = "Info";

/// The files of `folder` whose name starts with `prefix` and ends with `suffix`, by name, each read
/// up to a byte past `limit`, so one that is too large is seen to be. `checked`: the automation
/// read root decides which may be read.
#[cfg(not(target_arch = "wasm32"))]
fn named_files(
    folder: &std::path::Path,
    prefix: &str,
    suffix: &str,
    limit: usize,
    checked: bool,
    problems: &mut Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else { return out };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .take(MAX_FILES + 1)
        .collect();
    names.sort();
    for name in names {
        let file = folder.join(&name);
        if checked && let Err(e) = crate::file_access::check_read(&file) {
            problems.push(format!("{} is not read: {e}", file.display()));
            continue;
        }
        if !std::fs::metadata(&file).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let mut bytes = Vec::new();
        let most = (limit as u64).saturating_add(1);
        match std::fs::File::open(&file).and_then(|f| f.take(most).read_to_end(&mut bytes)) {
            Ok(_) => out.push((file.display().to_string(), bytes)),
            Err(e) => problems.push(format!("{} cannot be read: {e}", file.display())),
        }
    }
    out
}

// ── Languages and groups ────────────────────────────────────────────────────────────────────────

/// The built-in language sets (CLDR exemplar characters grouped by script), as the sidebar takes
/// them; decoded once and shared.
pub fn language_sets() -> Result<Arc<Vec<LanguageSet>>> {
    static LANGUAGES: OnceLock<std::result::Result<Arc<Vec<LanguageSet>>, String>> = OnceLock::new();
    let built = LANGUAGES.get_or_init(|| {
        sets::builtin()
            .map(|s| {
                Arc::new(
                    s.languages
                        .iter()
                        .map(|l| LanguageSet { id: l.id.clone(), name: l.name.clone(), script: l.script.clone(), characters: l.characters.clone() })
                        .collect(),
                )
            })
            .map_err(|e| e.to_string())
    });
    match built {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(EngineError::Internal(e.clone())),
    }
}

/// How many entries `entries` define at all levels.
fn count_entries(entries: &[GroupEntry]) -> usize {
    let mut n = 0usize;
    let mut stack: Vec<&GroupEntry> = entries.iter().collect();
    while let Some(e) = stack.pop() {
        n = n.saturating_add(1);
        stack.extend(e.children.iter());
    }
    n
}

/// Reads `Groups*.plist` files (name, text) into one set of entries, in file order. A file that
/// cannot be read, one past [`MAX_FILES`], and one that would take the entries of all files past
/// the limit of one file are skipped and reported; so are unusable entries.
pub(crate) fn parse_groups_files(files: &[(&str, &str)], data: &GlyphData, problems: &mut Vec<String>) -> Option<Arc<Groups>> {
    let mut all = Groups::default();
    let mut total = 0usize;
    for (i, (name, text)) in files.iter().enumerate() {
        if i >= MAX_FILES {
            problems.push(format!("groups file {name}: skipped, more than {MAX_FILES} files"));
            continue;
        }
        match gq::parse_groups(text, data) {
            Ok((groups, found)) => {
                problems.extend(found.into_iter().map(|p| format!("groups file {name}: {p}")));
                let n = count_entries(&groups.categories).saturating_add(count_entries(&groups.languages));
                if total.saturating_add(n) > gq::MAX_GROUP_ENTRIES {
                    problems
                        .push(format!("groups file {name} is ignored: the groups files together define more than {} entries", gq::MAX_GROUP_ENTRIES));
                    continue;
                }
                total += n;
                all.categories.extend(groups.categories);
                all.languages.extend(groups.languages);
            }
            Err(e) => problems.push(format!("groups file {name} is ignored: {e}")),
        }
    }
    if all.categories.is_empty() && all.languages.is_empty() { None } else { Some(Arc::new(all)) }
}

impl Session {
    /// The glyph data of a document that has no source folder (a new one): the built-in database
    /// with the user's custom files on top.
    pub fn glyph_data_without_document(&self) -> Result<Arc<GlyphData>> {
        self.user_glyph_data.merged.clone().map_or_else(builtin, Ok)
    }

    /// The built-in language sets the Languages section lists and its badges count.
    pub fn language_sets(&self) -> Result<Arc<Vec<LanguageSet>>> {
        language_sets()
    }

    /// The language sets named through the active document's glyph data, for the sidebar's
    /// Languages section and badges. Built once for each glyph data.
    pub fn language_index(&mut self) -> Result<Arc<LanguageIndex>> {
        let data = self.glyph_data()?;
        let languages = self.language_sets()?;
        if let Some(index) = self.language_index.as_ref().filter(|i| i.is_for(&languages, &data)) {
            return Ok(index.clone());
        }
        let index = Arc::new(LanguageIndex::new(languages, data).map_err(|e| EngineError::Internal(e.to_string()))?);
        self.language_index = Some(index.clone());
        Ok(index)
    }

    /// Sets the user's `Groups*.plist` files (name, text), in order: the app reads them from the
    /// `Info` folder of its settings folder. A character in a `list` is named through the built-in
    /// glyph data and the user's glyph data files, so the app sets those first
    /// ([`Session::set_user_glyph_data`]). Files that cannot be used are skipped; the problems are
    /// returned.
    pub fn set_user_groups(&mut self, files: &[(&str, &str)]) -> Vec<String> {
        let mut problems = Vec::new();
        self.user_groups = None;
        self.glyph_data_revision += 1;
        let data = match self.glyph_data_without_document() {
            Ok(d) => d,
            Err(e) => {
                problems.push(format!("groups files are ignored: {e}"));
                return problems;
            }
        };
        self.user_groups = parse_groups_files(files, &data, &mut problems);
        problems
    }

    /// Reads the user's glyph data (`GlyphData*.xml`) and groups (`Groups*.plist`) from the `Info`
    /// folder of the app's settings folder `settings` and sets them ([`Session::set_user_glyph_data`],
    /// [`Session::set_user_groups`]). Files that cannot be used are skipped; the problems are returned.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_user_info(&mut self, settings: &std::path::Path) -> Vec<String> {
        let folder = settings.join(INFO_FOLDER);
        let mut problems = Vec::new();
        let data = named_files(&folder, "GlyphData", ".xml", xml::MAX_FILE_BYTES, false, &mut problems);
        if !data.is_empty() {
            let files: Vec<(&str, &[u8])> = data.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
            problems.extend(self.set_user_glyph_data(&files));
        }
        let groups = named_files(&folder, "Groups", ".plist", MAX_GROUPS_FILE_BYTES, false, &mut problems);
        let mut texts: Vec<(&str, &str)> = Vec::new();
        for (name, bytes) in &groups {
            match std::str::from_utf8(bytes) {
                Ok(text) => texts.push((name, text)),
                Err(_) => problems.push(format!("groups file {name} is ignored: it is not UTF-8 text")),
            }
        }
        if !texts.is_empty() {
            problems.extend(self.set_user_groups(&texts));
        }
        problems
    }

    /// The Categories and Languages entries of the user's `Groups*.plist` files, if there are any.
    pub fn user_groups(&self) -> Option<Arc<Groups>> {
        self.user_groups.clone()
    }

    /// Goes up each time the user's glyph data, a document's source glyph data or the user's
    /// groups are set, so a view built from them knows to read them again.
    pub fn glyph_data_revision(&self) -> u64 {
        self.glyph_data_revision
    }
}
