//! Project documents: export instances linked to one font file and kept in a file of their own
//! (`project.separateFromFont`). They sit beside the font documents of a [`Session`]; the
//! commands in `cmd/project.rs` act on them.
//!
//! In memory a project holds absolute paths. When it is written, the font link and the export
//! destination are stored relative to the project file's folder where the two share a folder
//! above the file-system root, else absolute ([`stored_path`]); reading resolves them back
//! ([`resolved_path`]).
//!
//! The linked font is never written. It is read from disk when it is linked, when the project
//! opens, when the instances are reverted to the font's and at every export, so it need not be
//! open (`project.linkedFontNeedNotBeOpen`). The copy read last is kept for the axes and masters
//! that new and edited instances need.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use fontcraft_format::project::Project as StoredProject;
use fontcraft_model::{Font, Instance};
use serde_json::{Map, Value, json};

use crate::{EngineError, Result, Session};

/// Most projects open at once.
pub const MAX_OPEN_PROJECTS: usize = 256;
/// Most `..` segments in a stored relative path; a longer climb is stored absolute.
const MAX_CLIMB: usize = 32;

/// A project's contents as edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectData {
    /// The linked font file (absolute). `None`: no font linked yet.
    pub font: Option<PathBuf>,
    /// The export folder (absolute). `None`: not chosen yet.
    pub destination: Option<PathBuf>,
    /// The project's own instances, in list order.
    pub instances: Arc<Vec<Instance>>,
    /// Keys of the file this version does not know, written back as read.
    pub(crate) extra: Arc<Map<String, Value>>,
}

/// One open project.
#[derive(Clone, Debug)]
pub struct ProjectDoc {
    /// Stable for the session.
    pub id: u64,
    pub title: String,
    /// Where the project is saved (`None`: never saved).
    pub path: Option<PathBuf>,
    pub data: Arc<ProjectData>,
    /// The contents as on disk (for a new project: as it was created).
    pub saved: Arc<ProjectData>,
    /// Bumped by every change.
    pub revision: u64,
    /// The linked font as last read from disk; `None` when no font is linked or it could not be
    /// read.
    pub font: Option<Arc<Font>>,
}

impl ProjectDoc {
    pub fn modified(&self) -> bool {
        !Arc::ptr_eq(&self.data, &self.saved) && *self.data != *self.saved
    }

    /// What the project commands return about a project.
    pub fn summary(&self) -> Value {
        let instances: Vec<Value> = self
            .data
            .instances
            .iter()
            .enumerate()
            .map(|(i, inst)| {
                json!({
                    "index": i,
                    "name": fontcraft_fontinfo::target::instance_name(inst),
                    "type": if inst.kind.as_deref() == Some("variable") { "variable" } else { "static" },
                    "active": inst.exports,
                })
            })
            .collect();
        json!({
            "id": self.id,
            "title": self.title,
            "path": self.path.as_ref().map(|p| p.to_string_lossy()),
            "font": self.data.font.as_ref().map(|p| p.to_string_lossy()),
            "fontRead": self.font.is_some(),
            "destination": self.data.destination.as_ref().map(|p| p.to_string_lossy()),
            "instances": instances,
            "modified": self.modified(),
        })
    }
}

/// The open projects of a session.
#[derive(Clone, Debug, Default)]
pub(crate) struct Projects {
    docs: Vec<ProjectDoc>,
    active: Option<usize>,
    untitled: u32,
    next_id: u64,
}

impl Session {
    /// The open projects, in the order they were opened.
    pub fn projects(&self) -> &[ProjectDoc] {
        &self.projects.docs
    }

    /// The project the project commands act on when they name none: the one opened, created or
    /// named last.
    pub fn active_project(&self) -> Option<&ProjectDoc> {
        self.projects.active.and_then(|i| self.projects.docs.get(i))
    }

    pub(crate) fn project_at(&self, i: usize) -> Result<&ProjectDoc> {
        self.projects.docs.get(i).ok_or_else(|| EngineError::Disabled("project".into(), "no project open".into()))
    }

    /// Opens a project as a new, unmodified document and makes it the active one.
    pub(crate) fn push_project(&mut self, data: ProjectData, path: Option<PathBuf>, font: Option<Arc<Font>>) -> Result<usize> {
        if self.projects.docs.len() >= MAX_OPEN_PROJECTS {
            return Err(EngineError::Unsupported(format!("more than {MAX_OPEN_PROJECTS} open projects: close one first")));
        }
        let title = match &path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|t| !t.is_empty()).unwrap_or_else(|| "Project".into()),
            None => {
                self.projects.untitled += 1;
                if self.projects.untitled == 1 { "Untitled Project".to_string() } else { format!("Untitled Project {}", self.projects.untitled) }
            }
        };
        self.projects.next_id += 1;
        let data = Arc::new(data);
        self.projects.docs.push(ProjectDoc { id: self.projects.next_id, title, path, saved: data.clone(), data, revision: 0, font });
        let i = self.projects.docs.len() - 1;
        self.projects.active = Some(i);
        Ok(i)
    }

    /// Replaces project `i`'s contents (and, when given, its copy of the linked font), and makes
    /// it the active project.
    pub(crate) fn update_project(&mut self, i: usize, data: ProjectData, font: Option<Option<Arc<Font>>>) -> Result<()> {
        let doc = self.projects.docs.get_mut(i).ok_or_else(|| EngineError::Disabled("project".into(), "no project open".into()))?;
        if *doc.data != data {
            doc.data = Arc::new(data);
            doc.revision += 1;
        }
        if let Some(font) = font {
            doc.font = font;
        }
        self.projects.active = Some(i);
        Ok(())
    }

    /// Marks project `i` as saved to `path`.
    pub(crate) fn project_saved(&mut self, i: usize, path: PathBuf) -> Result<()> {
        let doc = self.projects.docs.get_mut(i).ok_or_else(|| EngineError::Disabled("project".into(), "no project open".into()))?;
        if let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|t| !t.is_empty()) {
            doc.title = stem;
        }
        doc.path = Some(path);
        doc.saved = doc.data.clone();
        Ok(())
    }

    pub(crate) fn close_project(&mut self, i: usize) -> Result<ProjectDoc> {
        if i >= self.projects.docs.len() {
            return Err(EngineError::Disabled("project".into(), "no project open".into()));
        }
        let doc = self.projects.docs.remove(i);
        let n = self.projects.docs.len();
        self.projects.active = match self.projects.active {
            _ if n == 0 => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.min(n - 1)),
            other => other,
        };
        Ok(doc)
    }

    /// The project a command names: `None` is the active project; a number is a project id; a
    /// text is the path a project was opened from or saved to, or its title.
    pub(crate) fn find_project(&self, reference: Option<&Value>) -> std::result::Result<usize, String> {
        let docs = &self.projects.docs;
        match reference {
            None | Some(Value::Null) => self.projects.active.filter(|i| *i < docs.len()).ok_or_else(|| "no project open".to_string()),
            Some(Value::Number(n)) => {
                let id = n.as_u64().ok_or_else(|| format!("`project` is a project id, a path or a title, not {n}"))?;
                docs.iter().position(|d| d.id == id).ok_or_else(|| format!("no open project has the id {id}"))
            }
            Some(Value::String(name)) => {
                let wanted = Path::new(name);
                let want = canonical(wanted);
                docs.iter()
                    .position(|d| d.path.as_deref().is_some_and(|p| p == wanted || (want.is_some() && canonical(p) == want)))
                    .or_else(|| docs.iter().position(|d| d.title == *name))
                    .ok_or_else(|| format!("no open project is called or saved as `{name}`"))
            }
            Some(other) => Err(format!("`project` is a project id, a path or a title, not {other}")),
        }
    }

    /// The glyph data for exporting a font read from `path`: the built-in database with the
    /// user's files and the files beside the font on top, as for an open document.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn glyph_data_beside(
        &self,
        font: &Font,
        path: &Path,
        warnings: &mut Vec<String>,
    ) -> Result<Arc<fontcraft_model::glyphdata::GlyphData>> {
        let base = || self.user_glyph_data.merged.clone().map_or_else(crate::glyph_data::builtin, Ok);
        if crate::glyph_data::ignores_source_files(font) {
            return base();
        }
        let files = crate::glyph_data::source_files(path, warnings);
        if files.is_empty() {
            return base();
        }
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let user = self.user_glyph_data.files.clone();
        let own = Arc::new(crate::glyph_data::parse_files(&refs, warnings));
        match crate::glyph_data::merge(&[&user, &own], warnings) {
            Some(merged) => Ok(merged),
            None => base(),
        }
    }
}

fn canonical(path: &Path) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::canonicalize(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

// ---------- paths ----------

/// `path` made absolute against the working folder, with `.` and `..` worked out by name.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    let full = if path.is_absolute() { path.to_path_buf() } else { std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()) };
    normalized(&full)
}

/// `path` with `.` dropped and `..` taking off the segment before it (none above the root).
fn normalized(path: &Path) -> PathBuf {
    let mut out: Vec<Component<'_>> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(c),
            },
            other => out.push(other),
        }
    }
    out.iter().collect()
}

/// How a path is written into the project file at `project`: relative to the project's folder
/// (segments joined by `/`) when the two share a folder below the root, else absolute.
pub fn stored_path(project: Option<&Path>, target: &Path) -> String {
    let target = absolute(target);
    let relative = project.and_then(|p| absolute(p).parent().map(Path::to_path_buf)).and_then(|dir| relative_to(&dir, &target));
    match relative {
        Some(segments) => segments.join("/"),
        None => target.to_string_lossy().into_owned(),
    }
}

/// The segments leading from folder `dir` to `target`, both absolute and normalized; `None` when
/// they share no folder below the root or the climb is too long.
fn relative_to(dir: &Path, target: &Path) -> Option<Vec<String>> {
    let a: Vec<Component<'_>> = dir.components().collect();
    let b: Vec<Component<'_>> = target.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if !a.iter().take(common).any(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let climb = a.len().saturating_sub(common);
    if climb > MAX_CLIMB {
        return None;
    }
    let mut out: Vec<String> = std::iter::repeat_n("..".to_string(), climb).collect();
    out.extend(b.iter().skip(common).map(|c| c.as_os_str().to_string_lossy().into_owned()));
    if out.is_empty() {
        out.push(".".into());
    }
    Some(out)
}

/// The absolute path a stored path names, for a project saved at `project`.
pub fn resolved_path(project: Option<&Path>, stored: &str) -> PathBuf {
    let path = Path::new(stored);
    if path.is_absolute() {
        return normalized(path);
    }
    match project.and_then(|p| absolute(p).parent().map(Path::to_path_buf)) {
        Some(dir) => normalized(&dir.join(path)),
        None => absolute(path),
    }
}

/// The contents as written to a project file at `path`.
pub fn to_stored(data: &ProjectData, path: Option<&Path>) -> StoredProject {
    StoredProject {
        font: data.font.as_deref().map(|f| stored_path(path, f)),
        destination: data.destination.as_deref().map(|d| stored_path(path, d)),
        instances: data.instances.as_ref().clone(),
        extra: data.extra.as_ref().clone(),
    }
}

/// The contents of a project file read from `path`.
pub fn from_stored(stored: StoredProject, path: Option<&Path>) -> ProjectData {
    ProjectData {
        font: stored.font.as_deref().map(|f| resolved_path(path, f)),
        destination: stored.destination.as_deref().map(|d| resolved_path(path, d)),
        instances: Arc::new(stored.instances),
        extra: Arc::new(stored.extra),
    }
}

// ---------- disk ----------

/// Reads the linked font at `path` with its linked sources resolved, as File › Open reads it.
/// A missing, moved or unreadable file is refused with a message that asks for the link again
/// (`project.exportPreconditions`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_font(path: &Path) -> Result<(Font, Vec<String>)> {
    let broken = |e: EngineError| EngineError::Io(format!("the linked font {} cannot be read ({e}); choose the font again", path.display()));
    let (font, mut warnings) = crate::io::open(path, None).map_err(broken)?;
    let font = crate::linked::resolve(&font, Some(path), &mut warnings).map_err(broken)?.unwrap_or(font);
    Ok((font, warnings))
}

/// Reads a project file, at most [`fontcraft_format::project::MAX_PROJECT_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file(path: &Path) -> Result<StoredProject> {
    use std::io::Read as _;
    let io = |what: &str, e: std::io::Error| EngineError::Io(format!("cannot {what} {}: {e}", path.display()));
    let meta = std::fs::metadata(path).map_err(|e| io("open", e))?;
    if !meta.is_file() {
        return Err(EngineError::Io(format!("cannot open {}: it is not a regular file", path.display())));
    }
    let limit = fontcraft_format::project::MAX_PROJECT_BYTES;
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| io("open", e))?.take(limit as u64 + 1).read_to_end(&mut bytes).map_err(|e| io("read", e))?;
    Ok(fontcraft_format::project::load_project(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn links_are_stored_relative_within_a_shared_folder_and_absolute_otherwise() {
        let project = Path::new("/work/family/projects/web.fontcraftproject");
        assert_eq!(stored_path(Some(project), Path::new("/work/family/Family.glyphs")), "../Family.glyphs");
        assert_eq!(stored_path(Some(project), Path::new("/work/family/projects/out")), "out");
        assert_eq!(stored_path(Some(project), Path::new("/other/Family.glyphs")), "/other/Family.glyphs");
        assert_eq!(stored_path(None, Path::new("/work/Family.glyphs")), "/work/Family.glyphs");
        assert_eq!(resolved_path(Some(project), "../Family.glyphs"), PathBuf::from("/work/family/Family.glyphs"));
        assert_eq!(resolved_path(Some(project), "/other/./a/../Family.glyphs"), PathBuf::from("/other/Family.glyphs"));
        assert_eq!(resolved_path(Some(project), "../../../../../../x"), PathBuf::from("/x"));
    }
}
