//! The control protocol: how agents drive the running app (`docs/control-protocol.md`).
//!
//! One JSON request in, one JSON reply out: `{"id": 1, "method": "engine.execute", "params":
//! {"command": "glyph.newGlyph", "params": {}}}` → `{"id": 1, "ok": true, "result": {…}}` or
//! `{"id": 1, "ok": false, "error": "…"}`. The transport (a loopback TCP port, one JSON object per
//! line) lives in the desktop app; this module is the dispatch.
//!
//! `ui.screenshot` needs the window, which only the host has: [`handle_request`] hands it back as
//! a [`ScreenshotRequest`], and the host answers it with [`ScreenshotRequest::finish`] once the
//! frame is captured. [`handle`] answers it with an error, for hosts without a window.

use base64::Engine as _;
use fontcraft_engine::{file_access, guard};
use fontcraft_geom::Point;
use fontcraft_render::{Image, LineOptions};
use fontcraft_tools::{Key, KeyEvent, Modifiers, PointerKind};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::dialogs::{self, DialogKey};
use crate::shell::{keymap, menus};
use crate::tools_glue::{self, MAX_POINTER_EVENTS, RawPointer, Space};

/// The methods [`handle`] answers.
pub const METHODS: &[(&str, &str)] = &[
    ("engine.execute", "{command, params?} run any engine or UI command"),
    ("engine.commands", "{} every command with label, menu, shortcut, params doc, enablement"),
    ("document.inspect", "{} the open documents: title, path, modified, revision, glyph count, history"),
    ("ui.inspect", "{} the UI state: selection, the glyph being edited, zoom, visible aids, status text"),
    ("ui.menu.list", "{} the menu tree with enablement"),
    (
        "ui.menu.invoke",
        "{command, params?} run a menu item as a click on it does: engine.execute, then the follow-up question its result asks opens as a dialog (an id that is in no menu is an error)",
    ),
    ("ui.screenshot", "{path?} capture the window as PNG: written to `path`, else returned as base64 `png`; returns {width, height, pixelsPerPoint}"),
    (
        "ui.render",
        "{path?, glyphs?, text?, master?: id (default: the showing tab's master, else the first), layer?, size?, scale?} draw glyphs filled, black on white: written to `path` as PNG, else returned as base64 `png`; returns {width, height, glyphs, master}",
    ),
    ("ui.tool.select", "{tool} make a pointer tool active (the command tool.select); returns {tool, previous}"),
    ("ui.tool.list", "{} the pointer tools and the active one (the command tool.list)"),
    (
        "ui.pointer",
        "{events: [{kind: down | drag | up | move | doubleclick, x, y, space?: font | screen (default font: the edited glyph's units; in text mode the text's), mods?: [shift, alt, cmd, ctrl]}]} drive the active tool like the mouse; returns {tool, busy, interaction, selection, overlays, errors}",
    ),
    (
        "ui.key",
        "{key, shift?, alt?, cmd?, ctrl?, action?: tap | press | release (default tap)} a key as the keyboard gives it: the tools' keys first, then the menu shortcuts; returns {handled, command?, result?, tool, busy, interaction, selection, overlays, errors}",
    ),
    ("ui.text", "{text} type into the focused text field; with none focused, each character is a ui.key tap; returns {field} or {keys, handled}"),
    (
        "ui.dialog.inspect",
        "{} the open dialog: {open, dialog, title, command, mode, message, tabs?: [{id, tab, label, active}], fields: [{id, label, kind, value, focused, min?, max?, choices?, labels?, default, doc}], editing, params (what OK runs), error, actions?: [{id, label, command}], buttons: [ids, ok first], captions: {id: caption}, canChooseFolder}; {open: false} when none is",
    ),
    (
        "ui.dialog.set",
        "{field, value} give a field of the open dialog a value (a number field also takes text) and preview it; returns ui.dialog.inspect's reply",
    ),
    ("ui.dialog.confirm", "{} OK: keep the preview as one undo step; returns {confirmed, changed, command, params}"),
    ("ui.dialog.cancel", "{} Cancel: the font as it was when the dialog opened; returns {cancelled}"),
    (
        "ui.click",
        "{id} press a control of the open dialog: `ok`, `cancel`, a third button by the id `buttons` lists (`replace`, `decompose`), a tab by its dialog id (shows it), an action of the \"…\" menu by its id (runs it: {action, command, text, result}, the text also goes to the system clipboard), or a field by its parameter's key (a checkbox toggles, a number field takes the keyboard for ui.text and ui.key, a folder field opens the folder chooser or, without one, takes the keyboard)",
    ),
    ("control.methods", "{} this list"),
];

/// Most characters one `ui.text` request may type.
pub const MAX_TEXT: usize = 10_000;

/// What [`handle_request`] did with a request.
#[derive(Debug)]
pub enum Handled {
    /// The reply, ready to send.
    Reply(Value),
    /// A screenshot: the host captures the window and answers with [`ScreenshotRequest::finish`].
    Screenshot(ScreenshotRequest),
}

/// A `ui.screenshot` request waiting for the window to be captured.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenshotRequest {
    /// The request's `id`, for the reply.
    pub id: Value,
    /// Where to write the PNG (`None`: return it as base64).
    pub path: Option<String>,
}

impl ScreenshotRequest {
    /// The reply for a captured window: `pixels` are premultiplied RGBA, rows top to bottom.
    pub fn finish(&self, width: u32, height: u32, pixels: Vec<u8>, pixels_per_point: f32) -> Value {
        let result = Image::from_rgba_premultiplied(width, height, pixels)
            .map_err(|e| format!("ui.screenshot: {e}"))
            .and_then(|image| deliver("ui.screenshot", &image, self.path.as_deref()))
            .map(|mut r| {
                set(&mut r, "pixelsPerPoint", json!(pixels_per_point));
                r
            });
        reply(&self.id, result)
    }

    /// The reply when the window cannot be captured.
    pub fn fail(&self, why: &str) -> Value {
        reply(&self.id, Err(format!("ui.screenshot: {why}")))
    }
}

fn reply(id: &Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    }
}

/// Answers one request. Never panics: a bug becomes an error reply and the app keeps running.
/// `ui.screenshot` gets an error reply: this entry point is for hosts without a window.
pub fn handle(app: &mut FontcraftApp, request: &Value) -> Value {
    match handle_request(app, request) {
        Handled::Reply(value) => value,
        Handled::Screenshot(shot) => shot.fail("this app has no window to capture (ui.render draws glyphs without one)"),
    }
}

/// Answers one request, or hands a screenshot back to the host. Never panics.
pub fn handle_request(app: &mut FontcraftApp, request: &Value) -> Handled {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Handled::Reply(reply(&id, Err("a request is a JSON object with a string `method`".into())));
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let roots = app.automation_roots.clone();
    file_access::confine(roots.as_ref(), || handle_confined(app, id, method, &params))
}

/// [`handle_request`] with the automation roots in force.
fn handle_confined(app: &mut FontcraftApp, id: Value, method: &str, params: &Value) -> Handled {
    if method == "ui.screenshot" {
        return match params.get("path") {
            None | Some(Value::Null) => Handled::Screenshot(ScreenshotRequest { id, path: None }),
            Some(Value::String(p)) if !p.is_empty() => match screenshot_path(p) {
                Ok(path) => Handled::Screenshot(ScreenshotRequest { id, path: Some(path) }),
                Err(e) => Handled::Reply(reply(&id, Err(format!("ui.screenshot: {e}")))),
            },
            Some(_) => Handled::Reply(reply(&id, Err("ui.screenshot: `path` must be a file path".into()))),
        };
    }
    // A client of the control channel exports inside the destination it names, whatever the app
    // allows the user.
    let outside = app.session.export_outside();
    app.session.set_export_outside(false);
    let result = guard::catch_panic(|| dispatch(app, method, params));
    app.session.set_export_outside(outside);
    Handled::Reply(match result {
        Ok(result) => reply(&id, result),
        Err(panic) => reply(&id, Err(format!("internal error (please report it): `{method}` failed unexpectedly: {panic}"))),
    })
}

/// Where `ui.screenshot` writes `path`. The frame is written later, after the roots are out of
/// force, so while they are in force this is the absolute path checked against the write root;
/// without them, `path` as given.
fn screenshot_path(path: &str) -> Result<String, String> {
    let Some(roots) = file_access::current() else { return Ok(path.to_string()) };
    let resolved = roots.resolved(path, file_access::Access::Write)?;
    resolved.to_str().map(str::to_string).ok_or_else(|| format!("{path} does not resolve to a UTF-8 path"))
}

/// Sets `key` on a JSON object (and does nothing to anything else).
fn set(value: &mut Value, key: &str, item: Value) {
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_string(), item);
    }
}

/// Writes `image` as PNG to `path`, or returns it as base64 when there is no path.
fn deliver(method: &str, image: &Image, path: Option<&str>) -> Result<Value, String> {
    let png = image.png().map_err(|e| format!("{method}: {e}"))?;
    let mut result = json!({"width": image.width, "height": image.height});
    match path {
        Some(path) => {
            file_access::check_write(std::path::Path::new(path)).map_err(|e| format!("{method}: {e}"))?;
            std::fs::write(path, &png).map_err(|e| format!("{method}: cannot write `{path}`: {e}"))?;
            set(&mut result, "path", json!(path));
        }
        None => set(&mut result, "png", json!(base64::engine::general_purpose::STANDARD.encode(&png))),
    }
    Ok(result)
}

/// `ui.render`: glyphs filled by the nonzero rule, through the `render` crate.
fn render(app: &FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.render";
    let font = app.session.font().map_err(|e| e.to_string())?;
    let obj = match params {
        Value::Null => serde_json::Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(format!("{M}: params must be a JSON object")),
    };
    let get = |key: &str| obj.get(key).filter(|v| !v.is_null());
    let number = |key: &'static str| match get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{M}: `{key}` must be a number")),
    };
    let path = match get("path") {
        None => None,
        Some(Value::String(p)) if !p.is_empty() => Some(p.as_str()),
        Some(_) => return Err(format!("{M}: `path` must be a file path")),
    };
    let glyphs: Vec<String> = match (get("glyphs"), get("text")) {
        (Some(_), Some(_)) => return Err(format!("{M}: give `glyphs` or `text`, not both")),
        (Some(list), None) => {
            let list = list.as_array().ok_or_else(|| format!("{M}: `glyphs` must be a list of glyph names"))?;
            if list.len() > fontcraft_render::MAX_LINE_GLYPHS {
                return Err(format!("{M}: at most {} glyphs", fontcraft_render::MAX_LINE_GLYPHS));
            }
            list.iter()
                .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{M}: `glyphs` must be a list of glyph names")))
                .collect::<Result<_, _>>()?
        }
        (None, Some(text)) => crate::state::glyphs_for_text(font, text.as_str().ok_or_else(|| format!("{M}: `text` must be a string"))?)
            .map_err(|e| format!("{M}: {e}"))?,
        (None, None) => app.default_render_glyphs(),
    };
    if glyphs.is_empty() {
        return Err(format!("{M}: no glyphs: give `glyphs` or `text`, open a glyph or select one in Font View"));
    }
    let master = match get("master") {
        None => app.render_master(),
        Some(Value::String(id)) if font.master(id).is_ok() => Some(id.clone()),
        Some(Value::String(id)) => return Err(format!("{M}: no master has id `{id}`")),
        Some(_) => return Err(format!("{M}: `master` must be a master id")),
    };
    let options = LineOptions { master: master.clone(), size: number("size")?, scale: number("scale")?, ..Default::default() };
    let image = match get("layer") {
        None => {
            let names: Vec<&str> = glyphs.iter().map(String::as_str).collect();
            fontcraft_render::render_line(font, &names, &options)
        }
        Some(Value::String(layer)) => {
            let [name] = glyphs.as_slice() else { return Err(format!("{M}: `layer` needs exactly one glyph")) };
            let glyph = font.glyph(name).ok_or_else(|| format!("{M}: no glyph named `{name}`"))?;
            fontcraft_render::render_layer(font, glyph, Some(layer), &options)
        }
        Some(_) => return Err(format!("{M}: `layer` must be a layer id")),
    }
    .map_err(|e| format!("{M}: {e}"))?;
    let mut result = deliver(M, &image, path)?;
    set(&mut result, "glyphs", json!(glyphs));
    if get("layer").is_none() {
        set(&mut result, "master", json!(master.or_else(|| font.masters.first().map(|m| m.id.clone()))));
    }
    Ok(result)
}

fn dispatch(app: &mut FontcraftApp, method: &str, params: &Value) -> Result<Value, String> {
    let command = || params.get("command").and_then(Value::as_str).ok_or_else(|| format!("{method}: `command` (a command id) is required"));
    let inner = || params.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "engine.execute" => app.run(command()?, &inner()),
        "engine.commands" => Ok(app.commands()),
        "document.inspect" => Ok(app.session.inspect()),
        "ui.inspect" => {
            let mut state = serde_json::to_value(&app.state).map_err(|e| e.to_string())?;
            set(&mut state, "editing", json!(app.state.editing()));
            set(&mut state, "tool", json!(app.tools.toolbox.active_id()));
            set(&mut state, "interaction", json!(app.session.interaction()));
            set(&mut state, "dialog", json!(app.dialog.as_ref().map(|d| d.id.as_str())));
            Ok(state)
        }
        "ui.menu.list" => serde_json::to_value(menus::menu_tree_for(&app.session, &app.state)).map_err(|e| e.to_string()),
        "ui.menu.invoke" => {
            let id = command()?;
            if !menus::menu_tree_for(&app.session, &app.state).iter().any(|m| m.items.iter().any(|i| i.command == id)) {
                return Err(format!("{method}: `{id}` is not a menu item"));
            }
            app.run_menu_command(id, inner())
        }
        "ui.render" => render(app, params),
        "ui.tool.select" => app.run("tool.select", params),
        "ui.tool.list" => app.run("tool.list", &Value::Null),
        "ui.pointer" => pointer(app, params),
        "ui.key" => {
            let name =
                params.get("key").and_then(Value::as_str).ok_or("ui.key: `key` (a key name such as `v`, `ArrowLeft`, `Escape`) is required")?;
            let mut mods = modifiers("ui.key", params.get("mods"))?;
            for (key, slot) in [("shift", &mut mods.shift), ("alt", &mut mods.alt), ("cmd", &mut mods.cmd), ("ctrl", &mut mods.ctrl)] {
                match params.get(key) {
                    None | Some(Value::Null) => {}
                    Some(Value::Bool(b)) => *slot |= *b,
                    Some(_) => return Err(format!("ui.key: `{key}` must be true or false")),
                }
            }
            let action = match params.get("action").and_then(Value::as_str) {
                None => KeyAction::Tap,
                Some("tap") => KeyAction::Tap,
                Some("press") => KeyAction::Press,
                Some("release") => KeyAction::Release,
                Some(_) => return Err("ui.key: `action` must be tap, press or release".into()),
            };
            Ok(key(app, name, mods, action))
        }
        "ui.text" => {
            let text = params.get("text").and_then(Value::as_str).ok_or("ui.text: `text` (a string) is required")?;
            if text.chars().count() > MAX_TEXT {
                return Err(format!("ui.text: at most {MAX_TEXT} characters"));
            }
            if dialogs::type_text(app, text) {
                return Ok(json!({"field": true}));
            }
            if tools_glue::queue_text(app, text) {
                return Ok(json!({"field": true}));
            }
            let mut handled = 0;
            for c in text.chars() {
                let mods = if c.is_uppercase() { Modifiers::SHIFT } else { Modifiers::NONE };
                if key(app, &c.to_string(), mods, KeyAction::Tap).get("handled") == Some(&Value::Bool(true)) {
                    handled += 1;
                }
            }
            Ok(json!({"keys": text.chars().count(), "handled": handled}))
        }
        "ui.dialog.inspect" => Ok(dialogs::inspect(app)),
        "ui.dialog.set" => {
            let field = params.get("field").and_then(Value::as_str).ok_or("ui.dialog.set: `field` (a field id) is required")?;
            let value = params.get("value").ok_or("ui.dialog.set: `value` is required")?;
            dialogs::set(app, field, value).map_err(|e| format!("ui.dialog.set: {e}"))
        }
        "ui.dialog.confirm" => dialogs::confirm(app).map_err(|e| format!("ui.dialog.confirm: {e}")),
        "ui.dialog.cancel" => dialogs::cancel(app).map_err(|e| format!("ui.dialog.cancel: {e}")),
        "ui.click" => {
            let id = params.get("id").and_then(Value::as_str).ok_or("ui.click: `id` (a control id) is required")?;
            dialogs::click(app, id).map_err(|e| format!("ui.click: {e}"))
        }
        "control.methods" => Ok(json!(METHODS.iter().map(|(m, doc)| json!({"method": m, "params": doc})).collect::<Vec<_>>())),
        other => Err(format!("unknown method `{other}` (see control.methods)")),
    }
}

/// Modifiers given as a list of names (`["shift", "alt"]`) or as `{shift, alt, cmd, ctrl}`.
fn modifiers(method: &str, value: Option<&Value>) -> Result<Modifiers, String> {
    let bad = || format!("{method}: `mods` must be a list of modifier names (shift, alt, cmd, ctrl)");
    match value {
        None | Some(Value::Null) => Ok(Modifiers::NONE),
        Some(Value::Array(list)) => {
            if list.len() > 8 {
                return Err(bad());
            }
            let names: Vec<&str> = list.iter().map(|v| v.as_str().ok_or_else(bad)).collect::<Result<_, _>>()?;
            Modifiers::from_names(names).map_err(|e| format!("{method}: {e}"))
        }
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).map_err(|_| bad()),
        Some(_) => Err(bad()),
    }
}

/// `ui.pointer`: every event is checked before the first one runs.
fn pointer(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.pointer";
    let events = params.get("events").and_then(Value::as_array).ok_or_else(|| format!("{M}: `events` (a list of pointer events) is required"))?;
    if events.len() > MAX_POINTER_EVENTS {
        return Err(format!("{M}: at most {MAX_POINTER_EVENTS} events"));
    }
    let mut parsed = Vec::with_capacity(events.len());
    for (i, e) in events.iter().enumerate() {
        let kind: PointerKind = e
            .get("kind")
            .and_then(|k| serde_json::from_value(k.clone()).ok())
            .ok_or_else(|| format!("{M}: event {i}: `kind` must be down, drag, up, move or doubleclick"))?;
        let coord = |key: &str| {
            e.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| format!("{M}: event {i}: `{key}` must be a number"))
        };
        let at = Point::new(coord("x")?, coord("y")?);
        let space = match e.get("space").and_then(Value::as_str) {
            None | Some("font") => Space::Font,
            Some("screen") => Space::Screen,
            Some(_) => return Err(format!("{M}: event {i}: `space` must be font or screen")),
        };
        let mods = modifiers(M, e.get("mods"))?;
        parsed.push(RawPointer { kind, at, space, mods });
    }
    if app.state.tab().is_none() {
        return Err(format!("{M}: no Edit View tab is showing (open a glyph first)"));
    }
    let mut errors = Vec::new();
    for ev in parsed {
        errors.extend(tools_glue::pointer(app, ev));
    }
    Ok(tools_state(app, errors))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Tap,
    Press,
    Release,
}

/// A key for `ui.key` and `ui.text`: an open dialog's keys first, then the tools' keys, then
/// the menu shortcuts. While a dialog is open nothing else takes a key.
fn key(app: &mut FontcraftApp, name: &str, mods: Modifiers, action: KeyAction) -> Value {
    if app.dialog.is_some() {
        let result = match DialogKey::parse(name) {
            Some(k) if action != KeyAction::Release && !mods.cmd && !mods.ctrl && !mods.alt => dialogs::key(app, k, mods.shift),
            _ => None,
        };
        let mut reply = dialogs::inspect(app);
        set(&mut reply, "handled", json!(result.is_some()));
        if let Some(Err(e)) = result {
            set(&mut reply, "error", json!(e));
        }
        return reply;
    }
    let tool_key = Key::parse(name).filter(|_| app.state.tab().is_some());
    let mut errors = Vec::new();
    let mut handled = false;
    let mut command = None;
    if action != KeyAction::Release {
        if let Some(k) = tool_key
            && let Some(e) = tools_glue::key(app, KeyEvent::press(k, mods))
        {
            handled = true;
            errors = e;
        }
        let egui_key = egui::Key::from_name(name).or_else(|| egui::Key::from_name(&name.to_uppercase()));
        if !handled && let Some(k) = egui_key {
            let m = egui::Modifiers { alt: mods.alt, ctrl: mods.ctrl, shift: mods.shift, mac_cmd: false, command: mods.cmd };
            if let Some((id, params)) = keymap::command_for(k, m, keymap::focused_panel(app)) {
                handled = true;
                command = Some((id, app.run_menu_command(id, params)));
            }
        }
    }
    if action != KeyAction::Press
        && let Some(k) = tool_key
        && let Some(e) = tools_glue::key(app, KeyEvent::release(k, mods))
    {
        errors.extend(e);
    }
    let mut reply = tools_state(app, errors);
    set(&mut reply, "handled", json!(handled));
    if let Some((id, result)) = command {
        set(&mut reply, "command", json!(id));
        match result {
            Ok(r) => set(&mut reply, "result", r),
            Err(e) => set(&mut reply, "error", json!(e)),
        }
    }
    reply
}

/// What the tools show after a control request: the tool, a gesture or interaction in progress,
/// the selection, the overlays and the errors of the actions carried out.
fn tools_state(app: &FontcraftApp, errors: Vec<String>) -> Value {
    json!({
        "tool": app.tools.toolbox.active_id(),
        "busy": app.tools.toolbox.busy(),
        "interaction": app.session.interaction(),
        "selection": app.state.tab().map(|t| serde_json::to_value(&t.selection).unwrap_or(Value::Null)),
        "overlays": app.tools.toolbox.overlays(),
        "errors": errors,
    })
}
