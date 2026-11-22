//! The Project panel's view state and commands: which open project it shows and which of its instances are selected,
//! Copy and Paste of instances, and the Add Custom Parameter picker's request. A project that opens or is created
//! shows in the panel ([`after_run`]). Row: `window.project`.
//!
//! Copy (`window.project.copy`) puts the selected entries on the clipboard as `fontcraft-instances` JSON and keeps
//! what it copied. Paste (`window.project.paste`) asks the window for the clipboard's text and, when it is still the
//! text Copy wrote, duplicates the copied entries after the selection with `project.duplicateInstances`. A copied
//! entry that changed or went since is left out; other text, or entries copied in another project, paste nothing.

use std::collections::BTreeMap;

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::FontCraftApp;
use crate::app::clipboard::PasteKind;
use crate::app::models::font_exports::shown_selection;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most selected entries kept per project.
const MAX_SELECTED: usize = 4096;
/// The `format` of the clipboard text Copy writes for project instances.
pub const INSTANCES_FORMAT: &str = "fontcraft-instances";
pub const COPY: &str = "window.project.copy";
pub const PASTE: &str = "window.project.paste";
pub const ADD_PARAMETER: &str = "window.project.addCustomParameter";

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.project.select",
        label: "Select Project Instances",
        menu: &[],
        shortcut: None,
        params: "{project: id, instances?: [index]} show the open project `project` in the Project panel and select these entries of its instance list (omitted: keep the selection); the fields at the right edit every selected entry, and an empty list selects the first",
    },
    UiCommand {
        id: COPY,
        label: "Copy Project Instances",
        menu: &[],
        shortcut: None,
        params: "{project?: id (default: the project the panel shows)} put the selected entries of the instance list (the first with none selected) on the clipboard as fontcraft-instances JSON",
    },
    UiCommand {
        id: PASTE,
        label: "Paste Project Instances",
        menu: &[],
        shortcut: None,
        params: "{project?: id (default: the project the panel shows), text?: the clipboard's text (omitted: the window's clipboard)} when the text is the one Copy wrote in this project, duplicate the copied entries after the selection (at the end with none selected) and select the copies",
    },
    UiCommand {
        id: ADD_PARAMETER,
        label: "Add Custom Parameter to Project Instances",
        menu: &[],
        shortcut: None,
        params: "{project?: id (default: the project the panel shows)} open the Add Custom Parameter picker for the selected entries of the instance list; Add adds each picked parameter to each of them with project.editInstance",
    },
];

/// What Copy took: the project, the text it wrote, and each copied entry as JSON, as it was.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CopiedInstances {
    pub project: u64,
    pub text: String,
    pub instances: Vec<String>,
}

/// What the Project panel shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ProjectWindowView {
    /// The project shown; `None` or a closed one shows the last open project.
    pub shown: Option<u64>,
    /// The selected entries of each project, as indices into its instances.
    pub selected: BTreeMap<u64, Vec<usize>>,
    /// What the last Copy took.
    pub copied: Option<CopiedInstances>,
    /// The Add Custom Parameter picker asked for: the project and the entries it adds to.
    pub picker: Option<(u64, Vec<usize>)>,
}

/// The parameters of `window.project.select`.
pub fn select(project: u64, instances: &[usize]) -> CommandRef {
    CommandRef::with_params("window.project.select", json!({ "project": project, "instances": instances }))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        COPY => return copy(app, &command.params),
        PASTE => return paste(app, &command.params),
        ADD_PARAMETER => {
            let (project, count) = target_project(app, &command.params)?;
            let wanted = app.view.project.selected.get(&project).cloned().unwrap_or_default();
            app.view.project.picker = Some((project, shown_selection(&wanted, count)));
            return Ok(());
        }
        _ => {}
    }
    let params = &command.params;
    let project = params.get("project").and_then(Value::as_u64).ok_or("`project` is the id of an open project")?;
    let count = app
        .session
        .projects()
        .iter()
        .find(|d| d.id == project)
        .map(|d| d.data.instances.len())
        .ok_or_else(|| format!("no open project {project}"))?;
    let instances = match params.get("instances") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            let mut picked: Vec<usize> = list
                .iter()
                .take(MAX_SELECTED)
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|n| *n < count)
                        .ok_or_else(|| format!("{v} is not an entry of the project"))
                })
                .collect::<Result<_, _>>()?;
            picked.sort_unstable();
            picked.dedup();
            Some(picked)
        }
        Some(_) => return Err("`instances` is a list of entry positions".into()),
    };
    let view = &mut app.view.project;
    view.shown = Some(project);
    if let Some(picked) = instances {
        view.selected.insert(project, picked);
    }
    view.selected.retain(|id, _| app.session.projects().iter().any(|d| d.id == *id));
    Ok(())
}

/// After `file.newProject` or `project.open`: the project shows in the Project panel, which opens when hidden.
pub fn after_run(app: &mut FontCraftApp, command: &str, reply: &Value) {
    if command != "file.newProject" && command != "project.open" {
        return;
    }
    let Some(id) = reply.get("project").and_then(|p| p.get("id")).and_then(Value::as_u64) else { return };
    app.view.project.shown = Some(id);
    if app.view.workspace.layout.panel_state(&PanelId::Project) == PanelState::Hidden {
        crate::shell::dock::show_window(&mut app.view.workspace, PanelId::Project, true, app.view.canvas.rect);
    }
    app.view_changed();
}

/// The project a Copy, Paste or picker request acts on (`project`, else the one the panel shows) and its entry count.
fn target_project(app: &FontCraftApp, params: &Value) -> Result<(u64, usize), String> {
    let docs = app.session.projects();
    let doc = match params.get("project") {
        None | Some(Value::Null) => app.view.project.shown.and_then(|id| docs.iter().find(|d| d.id == id)).or_else(|| docs.last()),
        Some(v) => {
            let id = v.as_u64().ok_or("`project` is the id of an open project")?;
            Some(docs.iter().find(|d| d.id == id).ok_or_else(|| format!("no open project {id}"))?)
        }
    };
    let doc = doc.ok_or("no project is open")?;
    Ok((doc.id, doc.data.instances.len()))
}

/// Puts the selected entries (the first with none selected) on the clipboard and keeps what was copied.
fn copy(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let (project, count) = target_project(app, params)?;
    let wanted = app.view.project.selected.get(&project).cloned().unwrap_or_default();
    let picked = shown_selection(&wanted, count);
    let doc = app.session.projects().iter().find(|d| d.id == project).ok_or_else(|| format!("no open project {project}"))?;
    let copied: Vec<&fontcraft_engine::model::Instance> = picked.iter().filter_map(|k| doc.data.instances.get(*k)).collect();
    if copied.is_empty() {
        return Err("Copy: the project has no instances".into());
    }
    let text = serde_json::to_string(&json!({ "format": INSTANCES_FORMAT, "instances": copied })).map_err(|e| format!("Copy: {e}"))?;
    let instances = copied.iter().map(serde_json::to_string).collect::<Result<Vec<_>, _>>().map_err(|e| format!("Copy: {e}"))?;
    app.status = format!("Copied {} instances", instances.len());
    app.clipboard.out = Some(text.clone());
    app.view.project.copied = Some(CopiedInstances { project, text, instances });
    Ok(())
}

/// Without `text`, asks the window for the clipboard's text, which comes back as `text`. With it, duplicates the
/// copied entries that are still in the project after the selection, when `text` is what Copy wrote there.
fn paste(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let (project, count) = target_project(app, params)?;
    let Some(text) = params.get("text").and_then(Value::as_str) else {
        app.clipboard.request = Some(PasteKind::ProjectInstances(project));
        return Ok(());
    };
    let copied = app.view.project.copied.as_ref().filter(|c| c.project == project && c.text == text);
    let copied = copied.ok_or("Paste: the clipboard holds no instances copied in this project")?;
    let doc = app.session.projects().iter().find(|d| d.id == project).ok_or_else(|| format!("no open project {project}"))?;
    let list: Vec<String> = doc.data.instances.iter().map(|i| serde_json::to_string(i).unwrap_or_default()).collect();
    let mut used = vec![false; list.len()];
    let mut originals = Vec::new();
    for json in &copied.instances {
        let found = list.iter().enumerate().position(|(k, inst)| inst == json && !used.get(k).copied().unwrap_or(true));
        if let Some(k) = found {
            if let Some(u) = used.get_mut(k) {
                *u = true;
            }
            originals.push(k);
        }
    }
    if originals.is_empty() {
        return Err("Paste: the copied instances changed or were removed".into());
    }
    let selected = app.view.project.selected.get(&project).cloned().unwrap_or_default();
    let to = selected.iter().copied().filter(|i| *i < count).max().map_or(count, |i| i.saturating_add(1));
    let n = originals.len();
    app.run_command("project.duplicateInstances", json!({ "project": project, "instances": originals, "to": to }))?;
    app.view.project.selected.insert(project, (to..to.saturating_add(n)).collect());
    app.view.project.shown = Some(project);
    app.view_changed();
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::Msg;
    use crate::dialogs::custom_parameter_picker::project_adds;

    fn project_app(names: &[&str]) -> (FontCraftApp, u64) {
        let mut app = FontCraftApp::with_edit_tab(Session::new(), Arc::new(Host::builder(crate::app::APP).build()));
        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        app.run_now(Msg::run("file.newProject", json!({})), false).unwrap();
        for name in names {
            app.run_now(Msg::run("project.addInstance", json!({ "styleName": name })), false).unwrap();
        }
        let id = app.session.projects().last().unwrap().id;
        (app, id)
    }

    fn names(app: &FontCraftApp) -> Vec<String> {
        app.session.projects().last().unwrap().data.instances.iter().map(|i| i.name.clone()).collect()
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
    }

    /// Copy puts the selected instances on the clipboard; Paste of that text duplicates them after the selection and
    /// selects the copies; other text pastes nothing. Paste without text asks the window for the clipboard.
    #[test]
    fn project_copy_and_paste_duplicate_the_selected_instances_after_the_selection() {
        let (mut app, project) = project_app(&["Light", "Regular", "Bold"]);
        shell(&mut app, "window.project.select", json!({ "project": project, "instances": [0, 1] })).unwrap();
        shell(&mut app, COPY, json!({})).unwrap();
        let text = app.clipboard.out.clone().expect("Copy wrote the entries for the clipboard");
        assert!(text.contains(INSTANCES_FORMAT), "{text}");
        assert_eq!(names(&app).len(), 3, "Copy changes nothing");

        shell(&mut app, PASTE, json!({})).unwrap();
        assert_eq!(app.clipboard.request, Some(PasteKind::ProjectInstances(project)), "Paste asks for the clipboard's text");
        app.clipboard.request = None;

        shell(&mut app, "window.project.select", json!({ "project": project, "instances": [2] })).unwrap();
        shell(&mut app, PASTE, json!({ "text": text })).unwrap();
        assert_eq!(names(&app), ["Light", "Regular", "Bold", "Light", "Regular"]);
        assert_eq!(app.view.project.selected.get(&project), Some(&vec![3, 4]), "the copies are selected");

        assert!(shell(&mut app, PASTE, json!({ "text": "Light" })).is_err(), "other text pastes nothing");
        assert_eq!(names(&app).len(), 5);
    }

    /// The plus beside Custom Parameters opens the picker on the selected instances; its Add edits each of them.
    #[test]
    fn project_parameter_picker_adds_to_every_selected_instance() {
        let (mut app, project) = project_app(&["Light", "Regular"]);
        shell(&mut app, "window.project.select", json!({ "project": project, "instances": [0, 1] })).unwrap();
        shell(&mut app, ADD_PARAMETER, json!({})).unwrap();
        assert_eq!(app.view.project.picker, Some((project, vec![0, 1])));
        for msg in project_adds(project, &[0, 1], &["Instance Preview".to_string()]) {
            app.run_now(msg, false).unwrap();
        }
        let doc = app.session.projects().last().unwrap();
        for instance in doc.data.instances.iter() {
            assert!(instance.params.0.iter().any(|p| p.name == "Instance Preview"), "{:?}", instance.params);
        }
    }
}
