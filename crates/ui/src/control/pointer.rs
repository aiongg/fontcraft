//! Driving the canvas tools from the control channel: `ui.pointer`, `ui.key`, `ui.tool.select` and `ui.tool.list`.
//!
//! The events take the path the canvas's own input takes (`canvas::tools::apply::input`), so a script and the mouse
//! do the same thing. Each method answers with the tools' state after it.

use craft::ui::runtime::commands::{Binding, CommandRef};
use fontcraft_geom::Point;
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind, TOOLS};
use serde_json::{Value, json};

use super::Method;
use crate::app::{FontCraftApp, Msg, ViewMsg};
use crate::canvas::tools::{ToolInput, apply};

pub const METHODS: &[(&str, Method)] = &[("ui.pointer", pointer), ("ui.key", key), ("ui.tool.select", tool_select), ("ui.tool.list", tool_list)];

/// Most modifier names one event may list.
const MAX_MODIFIER_NAMES: usize = 8;

/// The tools' state after a method, with the errors it met; the last error also goes to the status line.
fn finish(app: &mut FontCraftApp, errors: Vec<String>) -> Value {
    if let Some(last) = errors.last() {
        app.status = last.clone();
    }
    apply::sync(app);
    app.tools.refresh();
    app.project();
    json!({
        "tool": app.tools.toolbox.active_id(),
        "busy": app.tools.toolbox.busy(),
        "interaction": app.session.interaction(),
        "selection": serde_json::to_value(&app.tools.selection).unwrap_or(Value::Null),
        "overlays": serde_json::to_value(&app.tools.view.overlays).unwrap_or(Value::Null),
        "errors": errors,
    })
}

/// Modifiers given as a list of names (`["shift", "alt"]`) or as `{shift, alt, cmd, ctrl}`.
fn modifiers(method: &str, value: Option<&Value>) -> Result<Modifiers, String> {
    let bad = || format!("{method}: `mods` must be a list of modifier names (shift, alt, cmd, ctrl)");
    match value {
        None | Some(Value::Null) => Ok(Modifiers::NONE),
        Some(Value::Array(list)) => {
            if list.len() > MAX_MODIFIER_NAMES {
                return Err(bad());
            }
            let names: Vec<&str> = list.iter().map(|v| v.as_str().ok_or_else(bad)).collect::<Result<_, _>>()?;
            Modifiers::from_names(names).map_err(|e| format!("{method}: {e}"))
        }
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).map_err(|_| bad()),
        Some(_) => Err(bad()),
    }
}

/// `{events: [{kind: down | drag | up | move | doubleclick, x, y, space?: font | screen, mods?}]}`: drive the active
/// tool like the mouse. Positions are the edited glyph's font units unless `space` is `screen` (window points).
fn pointer(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.pointer";
    let events = params.get("events").and_then(Value::as_array).ok_or_else(|| format!("{M}: `events` (a list of pointer events) is required"))?;
    if events.len() > apply::MAX_POINTER_EVENTS {
        return Err(format!("{M}: at most {} events", apply::MAX_POINTER_EVENTS));
    }
    let camera = apply::camera(app);
    let mut parsed = Vec::with_capacity(events.len());
    for (i, e) in events.iter().enumerate() {
        let kind: PointerKind = e
            .get("kind")
            .and_then(|k| serde_json::from_value(k.clone()).ok())
            .ok_or_else(|| format!("{M}: event {i}: `kind` must be down, drag, up, move or doubleclick"))?;
        let coord = |key: &str| {
            e.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| format!("{M}: event {i}: `{key}` must be a number"))
        };
        let (x, y) = (coord("x")?, coord("y")?);
        let pos = match e.get("space").and_then(Value::as_str) {
            None | Some("font") => Point::new(x, y),
            Some("screen") => camera.to_font(egui::pos2(x as f32, y as f32)),
            Some(_) => return Err(format!("{M}: event {i}: `space` must be font or screen")),
        };
        let mods = modifiers(M, e.get("mods"))?;
        parsed.push(PointerEvent { kind, pos, mods });
    }
    if crate::canvas::tools::target(&app.session, &app.view).is_none() {
        return Err(format!("{M}: no glyph is being edited (open a glyph first)"));
    }
    let mut errors = Vec::new();
    for ev in parsed {
        errors.extend(apply::input(app, ToolInput::Pointer(ev)));
    }
    Ok(finish(app, errors))
}

/// The binding text of `name` held with `mods`, in the engine's shortcut notation (`Cmd+Alt+Shift+A`).
fn chord_text(name: &str, mods: Modifiers) -> String {
    let mut text = String::new();
    for (on, word) in [(mods.cmd, "Cmd+"), (mods.ctrl, "Ctrl+"), (mods.alt, "Alt+"), (mods.shift, "Shift+")] {
        if on {
            text.push_str(word);
        }
    }
    text.push_str(name);
    text
}

/// The engine or shell command whose shortcut is `chord`.
fn shortcut_command(chord: &Binding) -> Option<&'static str> {
    let engine = fontcraft_engine::cmd::command_specs().iter().filter_map(|spec| Some((spec.id, spec.shortcut?)));
    let shell = crate::shell::commands::MODULES.iter().flat_map(|m| m.commands.iter()).filter_map(|c| Some((c.id, c.shortcut?)));
    engine.chain(shell).find(|(_, text)| Binding::parse(text).ok().as_ref() == Some(chord)).map(|(id, _)| id)
}

/// `{key, shift?, alt?, cmd?, ctrl?, mods?, action?: tap | press | release}`: a key as the keyboard gives it, the
/// tools' keys first, then the menu shortcuts.
fn key(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.key";
    let name =
        params.get("key").and_then(Value::as_str).ok_or_else(|| format!("{M}: `key` (a key name such as `v`, `ArrowLeft`, `Escape`) is required"))?;
    let mut mods = modifiers(M, params.get("mods"))?;
    for (flag, slot) in [("shift", &mut mods.shift), ("alt", &mut mods.alt), ("cmd", &mut mods.cmd), ("ctrl", &mut mods.ctrl)] {
        match params.get(flag) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => *slot |= *b,
            Some(_) => return Err(format!("{M}: `{flag}` must be true or false")),
        }
    }
    let (press, release) = match params.get("action").and_then(Value::as_str) {
        None | Some("tap") => (true, true),
        Some("press") => (true, false),
        Some("release") => (false, true),
        Some(_) => return Err(format!("{M}: `action` must be tap, press or release")),
    };
    let tool_key = Key::parse(name);
    let mut errors = Vec::new();
    let mut handled = false;
    let mut command = None;
    if press {
        if let Some(k) = tool_key
            && let Some(errs) = apply::key(app, KeyEvent::press(k, mods))
        {
            handled = true;
            errors.extend(errs);
        } else if let Some(id) = Binding::parse(&chord_text(name, mods)).ok().as_ref().and_then(shortcut_command) {
            handled = true;
            command = Some(id);
            let msg = match FontCraftApp::route(CommandRef::new(id)) {
                Msg::Run(c) => Msg::Run(app.with_targets(c)),
                other => other,
            };
            if let Err(e) = app.apply_msg(msg) {
                errors.push(e);
            }
        }
    }
    if release && let Some(k) = tool_key {
        errors.extend(apply::key(app, KeyEvent::release(k, mods)).unwrap_or_default());
    }
    let mut reply = finish(app, errors);
    if let Some(object) = reply.as_object_mut() {
        object.insert("handled".into(), json!(handled));
        if let Some(id) = command {
            object.insert("command".into(), json!(id));
        }
    }
    Ok(reply)
}

/// `{tool}`: make a pointer tool current → `{tool, previous}`.
fn tool_select(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    let id = params.get("tool").and_then(Value::as_str).ok_or("ui.tool.select: `tool` (a tool id, see ui.tool.list) is required")?;
    let tool = TOOLS.iter().find(|t| t.id == id).ok_or_else(|| {
        let known: Vec<&str> = TOOLS.iter().map(|t| t.id).collect();
        format!("ui.tool.select: unknown tool `{id}`; the tools are {}", known.join(", "))
    })?;
    let previous = app.view.tool;
    let result = app.apply_msg(Msg::View(ViewMsg::SelectTool(tool.id)));
    finish(app, result.err().into_iter().collect());
    Ok(json!({"tool": app.view.tool, "previous": previous}))
}

/// `{}` → `{active, tools: [{id, label, shortcut, group}]}`.
fn tool_list(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    Ok(app.tools.toolbox.list())
}
