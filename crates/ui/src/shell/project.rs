//! The Project panel's view state and command: which open project it shows and which of its instances are selected.
//! A project that opens or is created shows in the panel ([`after_run`]). Row: `window.project`.

use std::collections::BTreeMap;

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most selected entries kept per project.
const MAX_SELECTED: usize = 4096;

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "window.project.select",
    label: "Select Project Instances",
    menu: &[],
    shortcut: None,
    params: "{project: id, instances?: [index]} show the open project `project` in the Project panel and select these entries of its instance list (omitted: keep the selection); the fields at the right edit every selected entry, and an empty list selects the first",
}];

/// What the Project panel shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ProjectWindowView {
    /// The project shown; `None` or a closed one shows the last open project.
    pub shown: Option<u64>,
    /// The selected entries of each project, as indices into its instances.
    pub selected: BTreeMap<u64, Vec<usize>>,
}

/// The parameters of `window.project.select`.
pub fn select(project: u64, instances: &[usize]) -> CommandRef {
    CommandRef::with_params("window.project.select", json!({ "project": project, "instances": instances }))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
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
    let workspace = &mut app.view.workspace;
    if workspace.layout.panel_state(&PanelId::Project) == PanelState::Hidden {
        // A refused move (a protected panel) leaves the layout as it was.
        let _ = workspace.set_visible(PanelId::Project, true, None, false, |p| p.permissions());
    }
    app.view_changed();
}
