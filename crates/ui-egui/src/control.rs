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
use fontcraft_engine::guard;
use fontcraft_render::{Image, LineOptions};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::shell::menus;

/// The methods [`handle`] answers.
pub const METHODS: &[(&str, &str)] = &[
    ("engine.execute", "{command, params?} run any engine or UI command"),
    ("engine.commands", "{} every command with label, menu, shortcut, params doc, enablement"),
    ("document.inspect", "{} the open documents: title, path, modified, revision, glyph count, history"),
    ("ui.inspect", "{} the UI state: selection, the glyph being edited, zoom, visible aids, status text"),
    ("ui.menu.list", "{} the menu tree with enablement"),
    ("ui.menu.invoke", "{command, params?} run a menu item (the same as engine.execute; an id that is in no menu is an error)"),
    ("ui.screenshot", "{path?} capture the window as PNG: written to `path`, else returned as base64 `png`; returns {width, height, pixelsPerPoint}"),
    (
        "ui.render",
        "{path?, glyphs?, text?, layer?, size?, scale?} draw glyphs filled, black on white: written to `path` as PNG, else returned as base64 `png`; returns {width, height, glyphs}",
    ),
    ("control.methods", "{} this list"),
];

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
    if method == "ui.screenshot" {
        return match params.get("path") {
            None | Some(Value::Null) => Handled::Screenshot(ScreenshotRequest { id, path: None }),
            Some(Value::String(p)) if !p.is_empty() => Handled::Screenshot(ScreenshotRequest { id, path: Some(p.clone()) }),
            Some(_) => Handled::Reply(reply(&id, Err("ui.screenshot: `path` must be a file path".into()))),
        };
    }
    Handled::Reply(match guard::catch_panic(|| dispatch(app, method, &params)) {
        Ok(result) => reply(&id, result),
        Err(panic) => reply(&id, Err(format!("internal error (please report it): `{method}` failed unexpectedly: {panic}"))),
    })
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
    let options = LineOptions { size: number("size")?, scale: number("scale")?, ..Default::default() };
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
            Ok(state)
        }
        "ui.menu.list" => serde_json::to_value(menus::menu_tree(&app.session)).map_err(|e| e.to_string()),
        "ui.menu.invoke" => {
            let id = command()?;
            if !menus::menu_tree(&app.session).iter().any(|m| m.items.iter().any(|i| i.command == id)) {
                return Err(format!("{method}: `{id}` is not a menu item"));
            }
            app.run(id, &inner())
        }
        "ui.render" => render(app, params),
        "control.methods" => Ok(json!(METHODS.iter().map(|(m, doc)| json!({"method": m, "params": doc})).collect::<Vec<_>>())),
        other => Err(format!("unknown method `{other}` (see control.methods)")),
    }
}
