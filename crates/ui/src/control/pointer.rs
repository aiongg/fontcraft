//! Driving the canvas tools from the control channel: `ui.pointer`, `ui.key`, `ui.tool.select` and `ui.tool.list`.
//!
//! The events take the path the canvas's own input takes (`canvas::tools::apply::input`), so a script and the mouse
//! do the same thing. Each method answers with the tools' state after it. While Font View shows, `ui.key` gives Font
//! View's keys to the window, where the grid or the list that has the keyboard takes them, and none to the tools.
//! While a context menu is open, every key goes to the window, where the menu takes it as it takes the keyboard's, and
//! so does every key while a text field has the keyboard (a panel's field, the macro editor) or a modal dialog is open
//! (About, Settings, …), whose Escape and Return are the dialog's.

use craft::keymap::{Key as WindowKey, Mods};
use craft::ui::runtime::commands::{Binding, CommandRef};
use egui::ViewportId;
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

/// The engine or shell command whose shortcut is `chord` and works while Font View shows (`font_view`) with `tool`
/// current (`catalog::shortcut_works`), then the numbered shortcuts with their parameters.
fn shortcut_command(chord: &Binding, font_view: bool, tool: &str) -> Option<CommandRef> {
    let engine = fontcraft_engine::cmd::command_specs()
        .iter()
        .filter(|spec| crate::app::catalog::shortcut_works(spec.id, font_view, tool))
        .filter_map(|spec| Some((spec.id, spec.shortcut?)));
    let shell = crate::shell::commands::MODULES.iter().flat_map(|m| m.commands.iter()).filter_map(|c| Some((c.id, c.shortcut?)));
    let matches = |text: &str| Binding::parse(text).ok().as_ref() == Some(chord);
    if let Some((id, _)) = engine.chain(shell).find(|(_, text)| matches(text)) {
        return Some(CommandRef::new(id));
    }
    crate::shell::numbered_keys::all().into_iter().find(|key| matches(&key.shortcut)).map(|key| key.command)
}

/// `{key, shift?, alt?, cmd?, ctrl?, mods?, action?: tap | press | release}`: a key as the keyboard gives it, the
/// tools' keys first, then the menu shortcuts that work where the window is. While Font View shows, its own keys and
/// the characters no shortcut there takes go to the window, for the grid or list that has the keyboard.
fn key(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.key";
    let name =
        params.get("key").and_then(Value::as_str).ok_or_else(|| format!("{M}: `key` (a key name such as `v`, `ArrowLeft`, `Escape`) is required"))?;
    if app.dialogs.open.is_some() {
        let reply = super::dialogs::key(app, params).map_err(|e| format!("{M}: {e}"))?;
        if reply.get("handled").and_then(Value::as_bool) == Some(true) {
            return Ok(reply);
        }
    }
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
    // An open context menu takes the window's keys before anything under it, so the key goes to the window as the
    // keyboard sends it.
    let menu_open = app.context_menus.canvas.is_some() || app.context_menus.glyph.is_some();
    if menu_open && let Some(steps) = to_window(app, name, mods, press, release) {
        return Ok(json!({"queued": steps, "handled": true, "contextMenu": true}));
    }
    // A text field that has the keyboard takes the keys before the tools and Font View, as it takes the keyboard's.
    if app.control.window.as_ref().is_some_and(craft::ui::runtime::windows::WindowHost::typing) {
        return field_key(app, name, mods, press, release);
    }
    // A modal dialog takes the window's keys: Escape cancels it and Return is its OK, as on the keyboard.
    if app.dialogs.modal
        && let Some(steps) = to_window(app, name, mods, press, release)
    {
        return Ok(json!({"queued": steps, "handled": true, "dialog": true}));
    }
    let tool_key = Key::parse(name);
    // While Font View shows, its own keys go to the window as the keyboard sends them, and no key reaches the edit
    // view's tools.
    let font_view = app.font_view_shows();
    if font_view && let Some(taken) = tool_key.and_then(|k| crate::panels::font_view::keys::takes(k, mods)) {
        return font_view_key(app, taken, mods, press, release);
    }
    let tool_key = tool_key.filter(|_| !font_view);
    let mut errors = Vec::new();
    let mut handled = false;
    let mut command = None;
    if press {
        if let Some(k) = tool_key
            && let Some(errs) = apply::key(app, KeyEvent::press(k, mods))
        {
            handled = true;
            errors.extend(errs);
        } else if let Some(fired) =
            Binding::parse(&chord_text(name, mods)).ok().as_ref().and_then(|chord| shortcut_command(chord, font_view, app.view.tool))
        {
            handled = true;
            command = Some(fired.id.clone());
            let msgs = match crate::canvas::text::edit::fired(app, &fired) {
                Some(Ok(msgs)) => msgs,
                Some(Err(e)) => {
                    errors.push(e);
                    Vec::new()
                }
                None => vec![match FontCraftApp::route(fired) {
                    Msg::Run(c) => Msg::Run(app.with_targets(c)),
                    other => other,
                }],
            };
            for msg in msgs {
                if let Err(e) = app.apply_msg(msg) {
                    errors.push(e);
                }
            }
        }
    }
    if font_view
        && !handled
        && let Some(typed) = typed_char(name, mods)
    {
        return font_view_typed(app, typed, mods, press, release);
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

/// Queues the key `name` on the main window as the keyboard sends it → the steps queued; `None` without a window or for
/// a key the window has no name for.
fn to_window(app: &mut FontCraftApp, name: &str, mods: Modifiers, press: bool, release: bool) -> Option<usize> {
    let host = app.control.window.clone()?;
    let key = WindowKey::parse(name)?;
    let held = Mods { cmd: mods.cmd, ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift };
    if press {
        host.inject_key(ViewportId::ROOT, key, held, !release);
    } else if release {
        host.inject_let_go(ViewportId::ROOT, key);
    }
    let steps = usize::from(press) + usize::from(release);
    app.control.queued += steps;
    Some(steps)
}

/// A key for the text field that has the keyboard, queued on its window as the keyboard sends it; a character with no
/// Cmd, Ctrl or Alt is typed too → `{queued, handled, field: true}`, `handled` false for a key the window has no name for.
fn field_key(app: &mut FontCraftApp, name: &str, mods: Modifiers, press: bool, release: bool) -> Result<Value, String> {
    let host = app.control.window.clone().ok_or("ui.key: a text field's keys need the window")?;
    let viewport = host.key_window();
    let key = WindowKey::parse(name);
    let typed = typed_char(name, mods);
    let held = Mods { cmd: mods.cmd, ctrl: mods.ctrl, alt: mods.alt, shift: mods.shift };
    let mut steps = 0;
    if press {
        if let Some(key) = key {
            host.inject_key(viewport, key, held, !release);
            steps += 1;
        }
        if let Some(text) = &typed {
            host.inject_text(viewport, text);
            steps += 1;
        }
    } else if release && let Some(key) = key {
        host.inject_let_go(viewport, key);
        steps += 1;
    }
    app.control.queued += steps;
    Ok(json!({"queued": steps, "handled": steps > 0, "field": true}))
}

/// A key Font View takes (`keys::takes`), queued on the window for the grid or the list that has the keyboard →
/// `{queued, handled: true, fontView: true}`.
fn font_view_key(app: &mut FontCraftApp, key: WindowKey, mods: Modifiers, press: bool, release: bool) -> Result<Value, String> {
    let host = app.control.window.clone().ok_or("ui.key: Font View's keys need the window: the desktop app or a test frame")?;
    let mut held = Mods::NONE;
    held.shift = mods.shift;
    if press {
        host.inject_key(ViewportId::ROOT, key, held, !release);
    } else if release {
        host.inject_let_go(ViewportId::ROOT, key);
    }
    let steps = usize::from(press) + usize::from(release);
    app.control.queued += steps;
    Ok(json!({"queued": steps, "handled": true, "fontView": true}))
}

/// The character the key `name` types with `mods` held: a one-character name with no Cmd, Ctrl or Alt held, as
/// Shift gives it.
fn typed_char(name: &str, mods: Modifiers) -> Option<String> {
    let mut chars = name.chars();
    let c = chars.next().filter(|c| !c.is_control() && chars.next().is_none() && !(mods.cmd || mods.ctrl || mods.alt))?;
    Some(if mods.shift { c.to_uppercase().collect() } else { c.to_lowercase().collect() })
}

/// A character typed while Font View shows that no shortcut there takes, queued on the window as the keyboard types
/// it: the grid or list that has the keyboard selects by name (type-ahead) → `{queued, handled: true, fontView: true}`.
fn font_view_typed(app: &mut FontCraftApp, typed: String, mods: Modifiers, press: bool, release: bool) -> Result<Value, String> {
    let host = app.control.window.clone().ok_or("ui.key: Font View's keys need the window: the desktop app or a test frame")?;
    let key = WindowKey::parse(&typed);
    let mut held = Mods::NONE;
    held.shift = mods.shift;
    let mut steps = 0;
    if press {
        if let Some(key) = key {
            host.inject_key(ViewportId::ROOT, key, held, !release);
            steps += 1;
        }
        host.inject_text(ViewportId::ROOT, &typed);
        steps += 1;
    } else if release && let Some(key) = key {
        host.inject_let_go(ViewportId::ROOT, key);
        steps += 1;
    }
    app.control.queued += steps;
    Ok(json!({"queued": steps, "handled": true, "fontView": true}))
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
