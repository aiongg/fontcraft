//! The Project panel's view model: the open projects, and for the one shown its font link, destination, instance
//! list, the selected entries' fields and custom parameters, and why Export is not yet possible.
//!
//! Project edits do not move the documents' revisions, so this model is projected outside the `Stamp` gate, keyed by
//! each project's revision and the panel's view state ([`ProjectModels::project`]).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_engine::fontinfo::rows::RowKind;
use fontcraft_engine::model::Font;
use fontcraft_engine::project::ProjectDoc;

use super::font_exports::{ExportEntry, ExportFields, fields, is_variable, shown_selection};
use super::font_info::{MAX_SHOWN, RowModel, rows};
use super::{ProjectCx, set};
use crate::shell::font_info::Target;

/// Why Export is disabled.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    NoFont,
    NoDestination,
    NeverSaved,
    Unsaved,
}

/// One open project, for the choice of the project shown.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ProjectChoice {
    pub id: u64,
    pub title: String,
}

/// What the Project panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ProjectModel {
    pub projects: Vec<ProjectChoice>,
    /// The project shown; `None` while no project is open.
    pub shown: Option<ShownProject>,
}

/// The project the panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ShownProject {
    pub id: u64,
    pub title: String,
    pub modified: bool,
    /// The linked font's path.
    pub font: Option<String>,
    /// A font is linked but could not be read.
    pub font_unread: bool,
    pub destination: Option<String>,
    /// Where the project is saved.
    pub path: Option<String>,
    /// Why Export is disabled; `None`: it can export.
    pub blocked: Option<Blocked>,
    pub entries: Vec<ExportEntry>,
    /// The selected entries, in list order; the first entry when none is selected.
    pub selected: Vec<usize>,
    /// The selected entries' built-in fields; `None` without entries.
    pub fields: Option<ExportFields>,
    /// The selected entries' custom parameters.
    pub params: Vec<RowModel>,
}

/// The model and what it was last projected for.
#[derive(Default)]
pub struct ProjectModels {
    pub model: Tracked<ProjectModel>,
    seen: Option<u64>,
}

impl ProjectModels {
    /// Projects the model when a project or the panel's view state changed.
    pub fn project(&mut self, cx: &mut ProjectCx<'_>) {
        let mut h = DefaultHasher::new();
        for doc in cx.session.projects() {
            (doc.id, doc.revision, doc.path.is_some(), doc.font.is_some(), doc.modified()).hash(&mut h);
        }
        cx.view.project.hash(&mut h);
        let key = h.finish();
        if self.seen == Some(key) {
            return;
        }
        self.seen = Some(key);
        set(&mut self.model, project(cx));
    }
}

/// Why `doc` cannot export now, or `None` when it can.
pub fn export_blocked(doc: &ProjectDoc) -> Option<Blocked> {
    if doc.data.font.is_none() {
        Some(Blocked::NoFont)
    } else if doc.data.destination.is_none() {
        Some(Blocked::NoDestination)
    } else if doc.path.is_none() {
        Some(Blocked::NeverSaved)
    } else if doc.modified() {
        Some(Blocked::Unsaved)
    } else {
        None
    }
}

fn project(cx: &mut ProjectCx<'_>) -> ProjectModel {
    let docs = cx.session.projects();
    let projects = docs.iter().map(|d| ProjectChoice { id: d.id, title: d.title.clone() }).collect();
    let view = &cx.view.project;
    let doc = view.shown.and_then(|id| docs.iter().find(|d| d.id == id)).or_else(|| docs.last());
    let Some(doc) = doc else { return ProjectModel { projects, shown: None } };
    let instances = &doc.data.instances;
    let wanted = view.selected.get(&doc.id).map(Vec::as_slice).unwrap_or_default();
    let selected = shown_selection(wanted, instances.len());
    let entries = instances
        .iter()
        .take(MAX_SHOWN)
        .map(|i| ExportEntry { name: i.name.trim().to_string(), active: i.exports, variable: is_variable(i) })
        .collect();
    // The shared field and row builders read a font: the linked font's axes and masters with the project's instances.
    let mut scratch = doc.font.as_deref().cloned().unwrap_or_else(|| Font::new(""));
    scratch.instances = Arc::clone(instances);
    let (fields, params) = if selected.is_empty() {
        (None, Vec::new())
    } else {
        let target = Target { scope: Scope::Instance, indices: selected.clone() };
        (fields(&scratch, &selected, &serde_json::Value::Null), rows(&scratch, &target, RowKind::Param))
    };
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let shown = ShownProject {
        id: doc.id,
        title: doc.title.clone(),
        modified: doc.modified(),
        font: doc.data.font.as_deref().map(path),
        font_unread: doc.data.font.is_some() && doc.font.is_none(),
        destination: doc.data.destination.as_deref().map(path),
        path: doc.path.as_deref().map(path),
        blocked: export_blocked(doc),
        entries,
        selected,
        fields,
        params,
    };
    ProjectModel { projects, shown: Some(shown) }
}
