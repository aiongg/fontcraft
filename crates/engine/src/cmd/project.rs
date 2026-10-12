//! Projects: `file.newProject` and the `project.*` commands, on the project documents of
//! `crate::project`, exported through the same build as File › Export.
//!
//! Rows: `file.newProject`, `project.linkFont`, `project.addInstance`, `project.removeInstances`,
//! `project.revertInstances`, `project.moveInstances`, `project.duplicateInstances`,
//! `project.editInstance`, `project.setExportDestination`, `project.export`, `project.open`,
//! `project.save`, `project.close`, `project.list`; `project.separateFromFont`,
//! `project.startsFromFontInstances`, `project.linkedFontNeedNotBeOpen`,
//! `project.exportPreconditions`, `project.instanceExportFolder`.
//!
//! Instance edits run the export entry commands (`font.instance.*`, `font.customParameters.*`) on
//! a private session holding a copy of the linked font with the project's instances in place of
//! its own, so a project instance is checked and changed exactly as an export of the font is.
//! Every command works out the project's new contents first and stores them last, so one that
//! fails changes nothing. The linked font is only ever read.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_fontinfo::{MAX_PARAMS_PER_SCOPE, MAX_PROJECT_INSTANCES, target};
use fontcraft_model::{Font, Instance};
use serde_json::{Map, json};

use super::*;
use crate::EngineError;
use crate::params::Params;
use crate::project::{ProjectData, ProjectDoc};

/// The keys of `project.editInstance`'s `changes` that `font.instance.set` takes.
const INSTANCE_KEYS: [&str; 8] = ["active", "styleName", "weightClass", "widthClass", "axes", "isBold", "isItalic", "linkStyle"];
/// The keys of `project.addInstance` that `font.instance.add` takes.
const ADD_KEYS: [&str; 4] = ["styleName", "axes", "weightClass", "widthClass"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.newProject",
            "New Project",
            ["File"],
            None,
            "{font?: path} a new, untitled project: export instances linked to a font file and kept apart from it, so other versions of the font can be exported without changing the font. With font, the font is linked at once as project.linkFont links it. The project becomes the active project; save it with project.save → {project: {id, title, path, font, fontRead, destination, instances: [{index, name, type, active}], modified}, copied, warnings}",
            always,
            new_project
        ).files(const { &[FileParam::read("font")] }),
        cmd!(
            "project.linkFont",
            "Choose",
            [],
            None,
            "{project?: id|path|title, font: path} link the project to a font file (default: the active project). The font is read from disk, so it need not be open, and is never written. A project without instances takes the font's instances as its starting list (copied counts them); one with instances keeps them → {project, copied, warnings}",
            has_project,
            link_font
        ).files(const { &[FileParam::read("font")] }),
        cmd!(
            "project.addInstance",
            "Add Instance",
            [],
            None,
            "{project?: id|path|title, kind?: \"instance\"|\"variableFontSetting\", at?: index, styleName?: string, axes?: {tag: number}, weightClass?: 1–1000, widthClass?: 1–9} add a static instance with the defaults of font.instance.add (Regular, at the linked font's first master, classes 400 and 5) at position at (default: the end). Variable font settings are not supported yet → {project, index}",
            has_project,
            add_instance
        ),
        cmd!(
            "project.removeInstances",
            "Remove Instances",
            [],
            None,
            "{project?: id|path|title, instances: [id|name|index]} delete the named instances from the project; the linked font is not touched → {project, removed}",
            has_project,
            remove_instances
        ),
        cmd!(
            "project.revertInstances",
            "Revert to Font Instances",
            [],
            None,
            "{project?: id|path|title} replace the project's instances with the ones the linked font file holds now, read from disk → {project, instances: count, warnings}",
            has_project,
            revert_instances
        ),
        cmd!(
            "project.moveInstances",
            "Move Instances",
            [],
            None,
            "{project?: id|path|title, instances: [id|name|index], to: index} move the named instances, in list order, so that the first of them sits at position to (from 0) of the new list → {project, indexes: [index]}",
            has_project,
            move_instances
        ),
        cmd!(
            "project.duplicateInstances",
            "Duplicate Instances",
            [],
            None,
            "{project?: id|path|title, instances: [id|name|index], to?: index} copy the named instances: each copy keeps its original's name and settings and goes right after its original, or, with to, the copies go together at position to (from 0) of the list as it was → {project, indexes: [index of each copy]}",
            has_project,
            duplicate_instances
        ),
        cmd!(
            "project.editInstance",
            "Edit Instance",
            [],
            None,
            "{project?: id|path|title, instance: id|name|index, changes: {active?, styleName?, weightClass?, widthClass?, axes?, isBold?, isItalic?, linkStyle?, customParameters?: [{action: \"add\"|\"set\"|\"remove\", name: string, index?: integer, value?: any, enabled?: bool}]}} change one project instance as Font Info › Exports changes an export: the property keys are those of font.instance.set, and each custom parameter edit is font.customParameters.add, set or remove on this instance, in order. All of it is checked before anything changes → {project, changed: [keys], warnings}",
            has_project,
            edit_instance
        ),
        cmd!(
            "project.setExportDestination",
            "Export Destination",
            [],
            None,
            "{project?: id|path|title, folder: path} set the folder the project exports into; it need not exist yet, export creates it → {project}",
            has_project,
            set_destination
        ).files(const { &[FileParam::write("folder")] }),
        cmd!(
            "project.export",
            "Export",
            [],
            None,
            "{project?: id|path|title, outlines?: [\"truetype\"|\"cff\"], containers?: [\"plain\"|\"woff\"|\"woff2\"], removeOverlap?: bool, timestamp?: number|\"now\", versionStamp?: bool} build every active static instance of the project from the linked font file, read from disk (it need not be open), into the export destination, through the same build as File › Export; an instance's Export Folder parameter puts its files in that sub-folder. Needs a saved project without unsaved changes, a font link that resolves and a destination. Default outlines truetype, container plain; with both outline formats the files go into ttf and otf subfolders. Instances between masters are skipped and listed. A failing export writes nothing → {destination, written, fonts: [{instance, source, format, outlines, glyphs, warnings, files}], skipped: [{instance, source, reason}], warnings}",
            has_project,
            export
        ).writes_files(),
        cmd!(
            "project.open",
            "Open Project",
            [],
            None,
            "{path: path} open a project file as the active project; one already open is made active. A font link that no longer resolves is kept and reported in warnings: choose the font again before exporting → {project, alreadyOpen, warnings}",
            always,
            open_project
        ).files(const { &[FileParam::read("path")] }),
        cmd!(
            "project.save",
            "Save Project",
            [],
            None,
            "{project?: id|path|title, path?: path} write the project to its file, or to path (ending in .fontcraftproject), which then becomes its file. The font link and the destination are stored relative to the project's folder where the two share a folder, else absolute. The file is written atomically; the linked font is not written → {project, path, bytes}",
            has_project,
            save_project
        ).files(const { &[FileParam::write("path")] }).writes_files(),
        cmd!(
            "project.close",
            "Close Project",
            [],
            None,
            "{project?: id|path|title, discard?: bool} close a project; one with unsaved changes needs discard: true → {closed: title}",
            has_project,
            close_project
        ),
        cmd!(
            query "project.list",
            "List Projects",
            [],
            None,
            "{} the open projects, in the order they were opened, and the active one → {active: id|null, projects: [{id, title, path, font, fontRead, destination, instances: [{index, name, type, active}], modified}]}",
            always,
            list_projects
        ),
    ]
}

fn has_project(s: &Session) -> std::result::Result<(), String> {
    s.active_project().map(|_| ()).ok_or_else(|| "no project open".into())
}

// ---------- shared ----------

/// The project the parameters name (`project`, default the active one).
fn project_index(s: &Session, p: &Params) -> Result<usize> {
    s.find_project(p.get("project")).map_err(|m| p.bad(m))
}

/// A file-path parameter, made absolute.
fn path_param(p: &Params, key: &str) -> Result<PathBuf> {
    let text = p.str(key)?;
    fontcraft_format::project::check_path(&format!("`{key}`"), text).map_err(|m| p.bad(m))?;
    Ok(crate::project::absolute(Path::new(text)))
}

/// The linked font's copy, or a new font when there is none, holding the project's instances in
/// place of its own: what the instance commands run on.
fn carrier(doc: &ProjectDoc) -> Font {
    let mut font = match &doc.font {
        Some(f) => Font::clone(f),
        None => Font::new("Untitled"),
    };
    font.instances = doc.data.instances.clone();
    font
}

/// Runs `steps` on a private session holding the carrier of `doc`, and returns its instances
/// afterwards with what the steps returned. An error of a step is reported as `command`'s.
fn on_carrier(command: &'static str, doc: &ProjectDoc, steps: impl FnOnce(&mut Session) -> Result<Value>) -> Result<(Arc<Vec<Instance>>, Value)> {
    let mut private = Session::new();
    // The linked font passed the model's checks when it was read; a new font is valid.
    private.push_document(carrier(doc), None);
    let out = steps(&mut private).map_err(|e| match e {
        EngineError::BadParams { message, .. } => EngineError::BadParams { command: command.into(), message },
        EngineError::Disabled(_, why) => EngineError::BadParams { command: command.into(), message: why },
        other => other,
    })?;
    let instances = private.font()?.instances.clone();
    Ok((instances, out))
}

/// The indexes of the instances `key` names, in list order, each once.
fn picked(p: &Params, doc: &ProjectDoc, key: &str) -> Result<Vec<usize>> {
    let value = p.get(key).ok_or_else(|| p.bad(format!("`{key}` is required")))?;
    let refs = target::parse_list(value).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    let font = carrier(doc);
    let mut out = target::instances(&font, &refs).map_err(|e| p.bad(format!("`{key}`: {e}")))?;
    out.sort_unstable();
    Ok(out)
}

/// A position parameter within `0..=max`.
fn position(p: &Params, key: &str, max: usize) -> Result<Option<usize>> {
    let max = i64::try_from(max).unwrap_or(i64::MAX);
    Ok(p.opt_int(key, 0, max)?.and_then(|v| usize::try_from(v).ok()))
}

/// `instances` with the ones at `moved` (sorted) taken out and put back together at `to` of the
/// list that remains.
fn moved_to(instances: &[Instance], moved: &[usize], to: usize) -> Option<Vec<Instance>> {
    let mut is_moved = vec![false; instances.len()];
    for &i in moved {
        *is_moved.get_mut(i)? = true;
    }
    let taken: Vec<Instance> = instances.iter().zip(&is_moved).filter(|(_, m)| **m).map(|(inst, _)| inst.clone()).collect();
    let mut rest: Vec<Instance> = instances.iter().zip(&is_moved).filter(|(_, m)| !**m).map(|(inst, _)| inst.clone()).collect();
    if to > rest.len() {
        return None;
    }
    rest.splice(to..to, taken);
    Some(rest)
}

fn with_project(s: &Session, i: usize, mut result: Map<String, Value>) -> Result<Value> {
    result.insert("project".into(), s.project_at(i)?.summary());
    Ok(Value::Object(result))
}

/// Links `data` to the font at `path`: reads it and, when the project has no instances, takes
/// the font's. Returns the font read, how many instances were copied, and the reader's warnings.
fn link(p: &Params, data: &mut ProjectData, path: PathBuf) -> Result<(Arc<Font>, usize, Vec<String>)> {
    let (font, warnings) = read_font(&path)?;
    if font.instances.len() > MAX_PROJECT_INSTANCES {
        return Err(p.bad(format!("the font has {} instances; a project holds at most {MAX_PROJECT_INSTANCES}", font.instances.len())));
    }
    data.font = Some(path);
    let copied = if data.instances.is_empty() {
        data.instances = font.instances.clone();
        data.instances.len()
    } else {
        0
    };
    Ok((Arc::new(font), copied, warnings))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_font(path: &Path) -> Result<(Font, Vec<String>)> {
    crate::project::read_font(path)
}

#[cfg(target_arch = "wasm32")]
fn read_font(_: &Path) -> Result<(Font, Vec<String>)> {
    Err(EngineError::Unsupported("reading a linked font file in the browser".into()))
}

// ---------- commands ----------

fn new_project(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.newProject", v)?;
    let mut data = ProjectData::default();
    let (font, copied, warnings) = match p.get("font") {
        Some(_) => {
            let path = path_param(&p, "font")?;
            let (font, copied, warnings) = link(&p, &mut data, path)?;
            (Some(font), copied, warnings)
        }
        None => (None, 0, Vec::new()),
    };
    let i = s.push_project(data, None, font)?;
    let mut out = Map::new();
    out.insert("copied".into(), json!(copied));
    out.insert("warnings".into(), json!(warnings));
    with_project(s, i, out)
}

fn link_font(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.linkFont", v)?;
    let i = project_index(s, &p)?;
    let path = path_param(&p, "font")?;
    let mut data = ProjectData::clone(&s.project_at(i)?.data);
    let (font, copied, warnings) = link(&p, &mut data, path)?;
    s.update_project(i, data, Some(Some(font)))?;
    let mut out = Map::new();
    out.insert("copied".into(), json!(copied));
    out.insert("warnings".into(), json!(warnings));
    with_project(s, i, out)
}

fn add_instance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.addInstance", v)?;
    let i = project_index(s, &p)?;
    match p.opt_str("kind")? {
        None | Some("instance") => {}
        Some("variableFontSetting") => return Err(EngineError::Unsupported("variable font settings in a project (ROADMAP.md, M9)".into())),
        Some(other) => return Err(p.bad(format!("`kind` is \"instance\" or \"variableFontSetting\", not `{other}`"))),
    }
    let doc = s.project_at(i)?;
    let count = doc.data.instances.len();
    let at = position(&p, "at", count)?.unwrap_or(count);
    let mut forward = Map::new();
    for key in ADD_KEYS {
        if let Some(value) = p.get(key) {
            forward.insert(key.into(), value.clone());
        }
    }
    let (instances, _) = on_carrier("project.addInstance", doc, |t| t.execute("font.instance.add", &Value::Object(forward)))?;
    let last = instances.len().checked_sub(1).ok_or_else(|| EngineError::Internal("the new instance is missing".into()))?;
    let list = moved_to(&instances, &[last], at).ok_or_else(|| p.bad("`at` is out of range"))?;
    let mut data = ProjectData::clone(&doc.data);
    data.instances = Arc::new(list);
    s.update_project(i, data, None)?;
    let mut out = Map::new();
    out.insert("index".into(), json!(at));
    with_project(s, i, out)
}

fn remove_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.removeInstances", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let gone = picked(&p, doc, "instances")?;
    let mut data = ProjectData::clone(&doc.data);
    let kept: Vec<Instance> =
        data.instances.iter().enumerate().filter(|(k, _)| gone.binary_search(k).is_err()).map(|(_, inst)| inst.clone()).collect();
    data.instances = Arc::new(kept);
    s.update_project(i, data, None)?;
    let mut out = Map::new();
    out.insert("removed".into(), json!(gone.len()));
    with_project(s, i, out)
}

fn revert_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.revertInstances", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let path = doc.data.font.clone().ok_or_else(|| p.bad("the project is linked to no font: choose one first"))?;
    let mut data = ProjectData::clone(&doc.data);
    data.instances = Arc::default();
    let (font, copied, warnings) = link(&p, &mut data, path)?;
    s.update_project(i, data, Some(Some(font)))?;
    let mut out = Map::new();
    out.insert("instances".into(), json!(copied));
    out.insert("warnings".into(), json!(warnings));
    with_project(s, i, out)
}

fn move_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.moveInstances", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let moved = picked(&p, doc, "instances")?;
    let room = doc.data.instances.len().saturating_sub(moved.len());
    let to = position(&p, "to", room)?.ok_or_else(|| p.bad(format!("`to` is required: a position from 0 to {room}")))?;
    let list = moved_to(&doc.data.instances, &moved, to).ok_or_else(|| p.bad("`to` is out of range"))?;
    let mut data = ProjectData::clone(&doc.data);
    data.instances = Arc::new(list);
    s.update_project(i, data, None)?;
    let mut out = Map::new();
    out.insert("indexes".into(), json!((to..to + moved.len()).collect::<Vec<_>>()));
    with_project(s, i, out)
}

fn duplicate_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.duplicateInstances", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let originals = picked(&p, doc, "instances")?;
    let count = doc.data.instances.len();
    if count.saturating_add(originals.len()) > MAX_PROJECT_INSTANCES {
        return Err(p.bad(format!("a project holds at most {MAX_PROJECT_INSTANCES} instances")));
    }
    let to = position(&p, "to", count)?;
    // A copy has no id of its own: ids name one entry each.
    let copy = |k: usize| doc.data.instances.get(k).map(|inst| Instance { id: None, ..inst.clone() });
    let (list, indexes) = match to {
        Some(to) => {
            let copies: Vec<Instance> = originals.iter().filter_map(|&k| copy(k)).collect();
            let n = copies.len();
            let mut list = doc.data.instances.as_ref().clone();
            list.splice(to..to, copies);
            (list, (to..to + n).collect::<Vec<_>>())
        }
        None => {
            let mut list = Vec::with_capacity(count + originals.len());
            let mut indexes = Vec::with_capacity(originals.len());
            for (k, inst) in doc.data.instances.iter().enumerate() {
                list.push(inst.clone());
                if originals.binary_search(&k).is_ok() {
                    indexes.push(list.len());
                    list.push(Instance { id: None, ..inst.clone() });
                }
            }
            (list, indexes)
        }
    };
    let mut data = ProjectData::clone(&doc.data);
    data.instances = Arc::new(list);
    s.update_project(i, data, None)?;
    let mut out = Map::new();
    out.insert("indexes".into(), json!(indexes));
    with_project(s, i, out)
}

fn edit_instance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.editInstance", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let reference = p.get("instance").ok_or_else(|| p.bad("`instance` is required"))?;
    let reference = target::parse(reference).map_err(|e| p.bad(format!("`instance`: {e}")))?;
    let k = target::instance(&carrier(doc), &reference).map_err(|e| p.bad(format!("`instance`: {e}")))?;
    let changes = p.opt_object("changes")?.ok_or_else(|| p.bad("`changes` is required"))?;
    if changes.is_empty() {
        return Err(p.bad("`changes` names nothing to change"));
    }
    let mut set = Map::new();
    let mut edits: &[Value] = &[];
    for (key, value) in changes {
        if INSTANCE_KEYS.contains(&key.as_str()) {
            set.insert(key.clone(), value.clone());
        } else if key == "customParameters" {
            edits = value.as_array().ok_or_else(|| p.bad("`changes.customParameters` is a list of edits"))?;
            if edits.len() > MAX_PARAMS_PER_SCOPE {
                return Err(p.bad(format!("`changes.customParameters` holds more than {MAX_PARAMS_PER_SCOPE} edits")));
            }
        } else {
            return Err(p.bad(format!("`changes` has no key `{key}`: it takes {} and customParameters", INSTANCE_KEYS.join(", "))));
        }
    }
    let mut steps: Vec<(&'static str, Value)> = Vec::with_capacity(edits.len() + 1);
    if !set.is_empty() {
        set.insert("instances".into(), json!([k]));
        steps.push(("font.instance.set", Value::Object(set)));
    }
    for (n, edit) in edits.iter().enumerate() {
        let Value::Object(edit) = edit else { return Err(p.bad(format!("custom parameter edit #{n} is not an object"))) };
        let id = match edit.get("action").and_then(Value::as_str) {
            Some("add") => "font.customParameters.add",
            Some("set") => "font.customParameters.set",
            Some("remove") => "font.customParameters.remove",
            _ => return Err(p.bad(format!("custom parameter edit #{n}: `action` is \"add\", \"set\" or \"remove\""))),
        };
        let mut forward: Map<String, Value> = edit.iter().filter(|(key, _)| *key != "action").map(|(key, v)| (key.clone(), v.clone())).collect();
        forward.insert("scope".into(), json!("instance"));
        forward.insert("targets".into(), json!([k]));
        steps.push((id, Value::Object(forward)));
    }
    let (instances, outputs) = on_carrier("project.editInstance", doc, |t| {
        let mut outputs = Vec::with_capacity(steps.len());
        for (id, params) in &steps {
            outputs.push(t.execute(id, params)?);
        }
        Ok(Value::Array(outputs))
    })?;
    let mut changed: Vec<String> = changes.keys().filter(|key| *key != "customParameters").cloned().collect();
    if !edits.is_empty() {
        changed.push("customParameters".into());
    }
    let warnings: Vec<Value> =
        outputs.as_array().into_iter().flatten().filter_map(|o| o.get("warnings").and_then(Value::as_array)).flatten().cloned().collect();
    let mut data = ProjectData::clone(&doc.data);
    data.instances = instances;
    s.update_project(i, data, None)?;
    let mut out = Map::new();
    out.insert("changed".into(), json!(changed));
    out.insert("warnings".into(), json!(warnings));
    with_project(s, i, out)
}

fn set_destination(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.setExportDestination", v)?;
    let i = project_index(s, &p)?;
    let folder = path_param(&p, "folder")?;
    let mut data = ProjectData::clone(&s.project_at(i)?.data);
    data.destination = Some(folder);
    s.update_project(i, data, None)?;
    with_project(s, i, Map::new())
}

#[cfg(not(target_arch = "wasm32"))]
fn export(s: &mut Session, v: &Value) -> Result<Value> {
    use super::compile::{self, Dest, Pick, Report, Request, Sink};
    use fontcraft_compile::{Flavour, Options};

    let p = Params::new("project.export", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    if doc.path.is_none() {
        return Err(p.bad("save the project first: a project that was never saved cannot be exported"));
    }
    if doc.modified() {
        return Err(p.bad("the project has unsaved changes: save it first"));
    }
    let font_path = doc.data.font.clone().ok_or_else(|| p.bad("the project is linked to no font: choose one first"))?;
    let destination = doc.data.destination.clone().ok_or_else(|| p.bad("the project has no export destination: set one first"))?;
    let instances = doc.data.instances.clone();
    let mut flavours = Vec::new();
    for name in p.opt_strings("outlines")?.unwrap_or_else(|| vec!["truetype"]) {
        let f = match name {
            "truetype" => Flavour::TrueType,
            "cff" => Flavour::Cff,
            other => return Err(p.bad(format!("`outlines` holds \"truetype\" or \"cff\", got \"{other}\""))),
        };
        if !flavours.contains(&f) {
            flavours.push(f);
        }
    }
    if flavours.is_empty() {
        return Err(p.bad("`outlines` names no outline format"));
    }
    let dest = Dest::Folder(destination.clone());
    let containers = compile::containers(&p, "containers", &dest)?;
    let options = Options {
        timestamp: compile::timestamp(&p)?,
        remove_overlap: p.bool_or("removeOverlap", true)?,
        version_stamp: p.bool_or("versionStamp", true)?,
        ..Options::default()
    };
    let req = Request { flavours, containers, options, dest, first_only: false };

    let (mut font, mut warnings) = crate::project::read_font(&font_path)?;
    font.instances = instances;
    compile::check_glyph_names(&font)?;
    // Only the project's own instances: a project without one exports nothing, not a master.
    let jobs: Vec<compile::Job> = compile::jobs(&font, &Pick::Exportable, false, &p)?.into_iter().filter(|j| j.source == "instance").collect();
    if jobs.is_empty() {
        return Err(p.bad("the project has no active static instance to export"));
    }
    compile::check_size(&req, jobs.len())?;
    let data = s.glyph_data_beside(&font, &font_path, &mut warnings)?;
    let title = font_path.file_stem().map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
    let mut report = Report { warnings, ..Report::default() };
    let mut sink = Sink::new(&req.dest, false, s.export_outside())?;
    compile::build(&font, &title, Some(&font_path), data, &jobs, &req, &mut sink, &mut report)?;
    if report.fonts.is_empty() {
        return Err(compile::nothing_built(&report));
    }
    // A compiled font in the destination may be the linked font itself; the staged files are
    // dropped unwritten.
    if let Some(target) = report.fonts.iter().flat_map(|f| &f.files).filter_map(|f| f.path.as_deref()).find(|t| same_file(t, &font_path)) {
        return Err(p.bad(format!(
            "the export would replace the linked font {}: choose another destination, or give the instance another file name",
            target.display()
        )));
    }
    sink.commit()?;
    Ok(json!({
        "destination": destination.to_string_lossy(),
        "written": report.file_count(),
        "fonts": report.fonts_json(false),
        "skipped": report.skipped,
        "warnings": report.warnings,
    }))
}

/// Whether `a` and `b` name the same existing file, through links and letter case on file
/// systems that ignore it.
#[cfg(not(target_arch = "wasm32"))]
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (a.canonicalize(), b.canonicalize()) else { return false };
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let (Ok(ma), Ok(mb)) = (a.metadata(), b.metadata()) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    false
}

#[cfg(target_arch = "wasm32")]
fn export(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("exporting a project in the browser".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn open_project(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.open", v)?;
    let path = path_param(&p, "path")?;
    if let Ok(i) = s.find_project(Some(&json!(path.to_string_lossy()))) {
        let data = ProjectData::clone(&s.project_at(i)?.data);
        s.update_project(i, data, None)?;
        let mut out = Map::new();
        out.insert("alreadyOpen".into(), json!(true));
        out.insert("warnings".into(), json!([]));
        return with_project(s, i, out);
    }
    let stored = crate::project::read_file(&path)?;
    let data = crate::project::from_stored(stored, Some(&path));
    let mut warnings = Vec::new();
    let font = match &data.font {
        Some(font_path) => match crate::project::read_font(font_path) {
            Ok((font, mut w)) => {
                warnings.append(&mut w);
                Some(Arc::new(font))
            }
            Err(e) => {
                warnings.push(e.to_string());
                None
            }
        },
        None => None,
    };
    let i = s.push_project(data, Some(path), font)?;
    let mut out = Map::new();
    out.insert("alreadyOpen".into(), json!(false));
    out.insert("warnings".into(), json!(warnings));
    with_project(s, i, out)
}

#[cfg(target_arch = "wasm32")]
fn open_project(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("opening a project file in the browser".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_project(s: &mut Session, v: &Value) -> Result<Value> {
    use fontcraft_format::project::PROJECT_EXTENSION;

    let p = Params::new("project.save", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    let path = match p.get("path") {
        Some(_) => path_param(&p, "path")?,
        None => doc.path.clone().ok_or_else(|| p.bad("the project was never saved: give a `path`"))?,
    };
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "glyphsproject" {
        return Err(EngineError::Unsupported(format!("writing the reference app's project format; save as .{PROJECT_EXTENSION}")));
    }
    if ext != PROJECT_EXTENSION {
        return Err(p.bad(format!("a project file ends in .{PROJECT_EXTENSION}")));
    }
    if doc.data.font.as_deref() == Some(path.as_path()) {
        return Err(p.bad("`path` is the linked font: a project never writes its font"));
    }
    let bytes = fontcraft_format::project::save_project(&crate::project::to_stored(&doc.data, Some(&path)))?;
    fontcraft_format::write_atomic(&path, &bytes)?;
    s.project_saved(i, path.clone())?;
    let mut out = Map::new();
    out.insert("path".into(), json!(path.to_string_lossy()));
    out.insert("bytes".into(), json!(bytes.len()));
    with_project(s, i, out)
}

#[cfg(target_arch = "wasm32")]
fn save_project(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Unsupported("saving a project file in the browser".into()))
}

fn close_project(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("project.close", v)?;
    let i = project_index(s, &p)?;
    let doc = s.project_at(i)?;
    if doc.modified() && !p.bool_or("discard", false)? {
        return Err(p.bad(format!("`{}` has unsaved changes: save it first, or pass discard: true", doc.title)));
    }
    let closed = s.close_project(i)?;
    Ok(json!({ "closed": closed.title }))
}

fn list_projects(s: &mut Session, v: &Value) -> Result<Value> {
    Params::new("project.list", v)?;
    let projects: Vec<Value> = s.projects().iter().map(ProjectDoc::summary).collect();
    Ok(json!({ "active": s.active_project().map(|d| d.id), "projects": projects }))
}
