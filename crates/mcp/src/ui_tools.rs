//! The window tools: `screenshot`, `render`, `elements`, `pointer`, `click`, `set`, `key`, `text`,
//! `menu` and `dialog_*`. Each one calls a control method of the running app (`docs/control-protocol.md`),
//! so they are listed only when the server is attached ([`crate::Backend::has_app`]). Called
//! without an app, a window tool answers that it needs one. `render` is the exception: the engine draws it,
//! so it is offered headless too, and draws the same.
//!
//! The policy decides commands by id, and the mouse and keyboard can reach any command. So under
//! `--read-only`, `--allow` or `--deny`, the tools that act like the mouse and keyboard
//! (`pointer`, `click`, `set`, `key`, `text`) are not listed and are refused; `menu` and the dialog tools
//! check the command they would run before they run it.

use fontcraft_engine::cmd::preview_line::{RENDER_KEYS, request};
use serde_json::{Map, Value, json};

use crate::backend::NEEDS_APP;
use crate::{LogLevel, Server};

/// What a window tool can change, which decides how a restricted server treats it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// Reads the window or the open dialog, or puts the font back as the dialog found it.
    Safe,
    /// Runs one command the server can name before it runs: the policy checks it.
    Checked,
    /// Acts like the mouse and keyboard, which reach commands the server cannot see first.
    Input,
}

/// Every window tool, in the order `tools/list` gives them.
const TOOLS: &[(&str, Reach)] = &[
    ("screenshot", Reach::Safe),
    ("render", Reach::Safe),
    ("elements", Reach::Safe),
    ("pointer", Reach::Input),
    ("click", Reach::Input),
    ("set", Reach::Input),
    ("key", Reach::Input),
    ("text", Reach::Input),
    ("menu", Reach::Checked),
    ("dialog_inspect", Reach::Safe),
    ("dialog_set", Reach::Checked),
    ("dialog_confirm", Reach::Checked),
    ("dialog_cancel", Reach::Safe),
];

/// The arguments each tool passes on to its method; anything else is dropped. `render` passes on
/// [`RENDER_KEYS`]. `screenshot` and `render` take no `path`: the image comes back in the reply, and
/// no file is written.
const POINTER_KEYS: &[&str] = &["events"];
const ELEMENTS_KEYS: &[&str] = &["prefix", "role", "window", "offset", "limit"];
const CLICK_KEYS: &[&str] = &["id", "element", "x", "y", "button", "count", "mods", "shift", "alt", "cmd", "ctrl"];
const SET_KEYS: &[&str] = &["element", "value"];
const KEY_KEYS: &[&str] = &["key", "shift", "alt", "cmd", "ctrl", "action"];
const TEXT_KEYS: &[&str] = &["text"];
const DIALOG_SET_KEYS: &[&str] = &["field", "value"];

fn reach(name: &str) -> Option<Reach> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

/// What a headless server offers of them: the tool the engine draws.
pub(crate) fn headless() -> Value {
    definition("render")
}

/// The window tools a server offers: all of them, or without the input tools when `restricted`.
pub(crate) fn list(restricted: bool) -> Vec<Value> {
    TOOLS.iter().filter(|(_, r)| !(restricted && *r == Reach::Input)).map(|(name, _)| definition(name)).collect()
}

/// One sentence for `initialize`'s instructions when the server is attached.
pub(crate) fn describe(restricted: bool) -> String {
    let mut text = "The window tools (screenshot, render, elements, pointer, click, set, key, text, menu, dialog_inspect, dialog_set, dialog_confirm, dialog_cancel) show and drive the app's window.".to_string();
    if restricted {
        text.push_str(" Because this server is restricted, pointer, click, set, key and text are refused, and menu and the dialog tools run only commands the restrictions admit.");
    }
    text
}

/// Runs a window tool and returns its MCP content. `None`: `name` is not a window tool.
pub(crate) fn call(server: &mut Server, name: &str, args: &Value) -> Option<Result<Vec<Value>, String>> {
    let reach = reach(name)?;
    let outcome = run(server, name, reach, args);
    Some(match outcome {
        Ok(content) => {
            server.log(LogLevel::Info, json!({"tool": name, "ok": true}));
            Ok(content)
        }
        Err(Failure::Refused(reason)) => {
            server.log(LogLevel::Warning, json!({"tool": name, "refused": reason}));
            Err(reason)
        }
        Err(Failure::Failed(error)) => {
            server.log(LogLevel::Warning, json!({"tool": name, "error": error}));
            Err(error)
        }
    })
}

/// Why a window tool did nothing: the server's restrictions, or the app or the arguments.
enum Failure {
    Refused(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Failure::Failed(error)
    }
}

fn run(server: &mut Server, name: &str, reach: Reach, args: &Value) -> Result<Vec<Value>, Failure> {
    if !server.backend.has_app() && name != "render" {
        return Err(Failure::Failed(format!("`{name}` {NEEDS_APP}")));
    }
    let args = match args {
        Value::Null => Map::new(),
        Value::Object(map) => map.clone(),
        _ => return Err(Failure::Failed(format!("`{name}`: the arguments must be a JSON object"))),
    };
    if !server.backend.has_app() {
        let (command, params) = request(&args);
        server.check_command(command, &params, 0).map_err(Failure::Refused)?;
        return image(command, server.backend.execute(command, &params)?);
    }
    let restricted = server.policy.is_restricted();
    if restricted && reach == Reach::Input {
        return Err(Failure::Refused(format!(
            "`{name}` acts like the mouse and keyboard, which can run commands this server's restrictions cannot check first; use run_command, menu or the dialog tools"
        )));
    }
    let empty = json!({});
    match name {
        "screenshot" => image("ui.screenshot", server.backend.control("ui.screenshot", &empty)?),
        "render" => image("ui.render", server.backend.control("ui.render", &pick(&args, RENDER_KEYS))?),
        "elements" => text(server.backend.control("ui.elements", &pick(&args, ELEMENTS_KEYS))?),
        "pointer" => text(server.backend.control("ui.pointer", &pick(&args, POINTER_KEYS))?),
        "click" => text(server.backend.control("ui.click", &pick(&args, CLICK_KEYS))?),
        "set" => {
            if !args.get("element").is_some_and(Value::is_string) {
                return Err(Failure::Failed("`set`: `element` (the id of an element; see elements) is required".into()));
            }
            text(server.backend.control("ui.set", &pick(&args, SET_KEYS))?)
        }
        "key" => text(server.backend.control("ui.key", &pick(&args, KEY_KEYS))?),
        "text" => text(server.backend.control("ui.text", &pick(&args, TEXT_KEYS))?),
        "menu" => match args.get("command") {
            None | Some(Value::Null) => text(server.backend.control("ui.menu.list", &empty)?),
            Some(Value::String(command)) => {
                let params = args.get("params").cloned().unwrap_or(Value::Null);
                server.check_command(command, &params, 0).map_err(Failure::Refused)?;
                let mut invoke = Map::new();
                invoke.insert("command".into(), json!(command));
                if !params.is_null() {
                    invoke.insert("params".into(), params);
                }
                text(server.backend.control("ui.menu.invoke", &Value::Object(invoke))?)
            }
            Some(_) => Err(Failure::Failed("`menu`: `command` must be a command id".into())),
        },
        "dialog_inspect" => text(server.backend.control("ui.dialog.inspect", &empty)?),
        "dialog_set" => {
            if restricted {
                check_dialog(server)?;
            }
            text(server.backend.control("ui.dialog.set", &pick(&args, DIALOG_SET_KEYS))?)
        }
        "dialog_confirm" => {
            if restricted {
                check_dialog(server)?;
            }
            text(server.backend.control("ui.dialog.confirm", &empty)?)
        }
        "dialog_cancel" => text(server.backend.control("ui.dialog.cancel", &empty)?),
        other => Err(Failure::Failed(format!("`{other}` is not a window tool"))),
    }
}

/// Checks the command the open dialog previews and runs on OK against the policy. With no dialog
/// open there is nothing to check: the app answers that itself.
fn check_dialog(server: &mut Server) -> Result<(), Failure> {
    let dialog = server.backend.control("ui.dialog.inspect", &json!({}))?;
    if dialog.get("open").and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    let Some(command) = dialog.get("command").and_then(Value::as_str) else {
        return Err(Failure::Refused("the open dialog names no command this server can check, and this server is restricted".into()));
    };
    let params = dialog.get("params").cloned().unwrap_or(Value::Null);
    server.check_command(command, &params, 0).map_err(Failure::Refused)
}

/// The listed keys of `args` that are present, as a JSON object.
fn pick(args: &Map<String, Value>, keys: &[&str]) -> Value {
    Value::Object(keys.iter().filter_map(|k| args.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect())
}

fn text(reply: Value) -> Result<Vec<Value>, Failure> {
    Ok(vec![json!({"type": "text", "text": reply.to_string()})])
}

/// A reply with a base64 `png`: the image, then the other fields as text. A comparison that found nothing to draw is
/// its fields alone.
fn image(method: &str, reply: Value) -> Result<Vec<Value>, Failure> {
    let Value::Object(mut fields) = reply else {
        return Err(Failure::Failed(format!("{method}: the reply is not an object")));
    };
    let Some(Value::String(png)) = fields.remove("png") else {
        if fields.contains_key("changedCount") {
            return text(Value::Object(fields));
        }
        return Err(Failure::Failed(format!("{method}: the reply holds no PNG image")));
    };
    Ok(vec![json!({"type": "image", "data": png, "mimeType": "image/png"}), json!({"type": "text", "text": Value::Object(fields).to_string()})])
}

fn definition(name: &str) -> Value {
    let mods = json!({"type": "array", "items": {"type": "string", "enum": ["shift", "alt", "cmd", "ctrl"]}, "description": "Modifier keys held. cmd is Command on a Mac and Control elsewhere; ctrl suspends snapping."});
    let flag = |what: &str| json!({"type": "boolean", "description": format!("Hold {what}.")});
    let (description, properties, required): (&str, Value, &[&str]) = match name {
        "screenshot" => (
            "Capture the app's window as a PNG image. The text part gives width and height in pixels and pixelsPerPoint: window points, which click takes, are pixels divided by pixelsPerPoint.",
            json!({}),
            &[],
        ),
        "render" => (
            "Draw glyphs as a PNG image, without the window: filled black on white, shaped with features, at a master, an instance or a location, in colour, with their nodes, or compared with an earlier revision. Name the glyphs, or give text whose characters map to glyphs; with neither, the app draws the shown glyph and a comparison what changed. The text part gives the size, the glyphs drawn and warnings. The same request gives the same image.",
            json!({
                "glyphs": {"type": "array", "items": {"type": "string"}, "description": "Glyph names, drawn in a line (at most 1,000)."},
                "text": {"type": "string", "description": "Characters to draw; each maps to its glyph."},
                "master": {"type": "string", "description": "A master id or name. Default: the master the window draws in, else the first."},
                "layer": {"type": "string", "description": "Draw this layer (id or name) of the one glyph named."},
                "layers": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Glyph names and the layers (id or name) to draw them in."},
                "instance": {"type": ["string", "integer"], "description": "Draw a static instance, by name or index, interpolated."},
                "location": {"type": "object", "additionalProperties": {"type": "number"}, "description": "Draw at a location: axis tags and internal values. Axes not named stand at the origin master."},
                "features": {"type": "array", "items": {"type": "string"}, "description": "OpenType substitution features to shape with (default none)."},
                "script": {"type": "string", "description": "OpenType script tag for localized forms."},
                "language": {"type": "string", "description": "OpenType language tag for localized forms."},
                "direction": {"type": "string", "enum": ["ltr", "rtl"], "description": "Default: the direction of the first letter."},
                "color": {"type": "boolean", "description": "Draw colour layers, or a layered font's masters in their Master Colors."},
                "palette": {"type": "integer", "description": "The palette colour layers take their colours from (with color; default 0)."},
                "showNodes": {"type": "boolean", "description": "Mark nodes and handles."},
                "invert": {"type": "boolean", "description": "Light on dark."},
                "compare": {"type": "object", "properties": {"revision": {"type": "integer"}, "steps": {"type": "integer"}}, "description": "Draw each glyph as it was at a revision the document had (or that many undo steps back, default 1) in red under as it is in blue; unchanged drawings in grey. The text part lists the changed glyphs."},
                "size": {"type": "number", "description": "Pixels per em (default 256)."},
                "scale": {"type": "number", "description": "Pixels per font unit, instead of size."}
            }),
            &[],
        ),
        "elements" => (
            "List what the app's windows show, panel fields included: each element's id, window, role, label, value, the labelled element around it (in), its rectangle in window points, whether it shows and is enabled, its states, and what set takes for it (bool, text, choice or number). Click one with click, change one with set.",
            json!({
                "prefix": {"type": "string", "description": "Only elements whose label starts with this, in any case."},
                "role": {"type": "string", "description": "Only elements of this role, such as button, textInput, spinButton, checkBox, comboBox, slider or tab."},
                "window": {"type": "string", "description": "Only elements of this window: main, or the id of a floating panel's window."},
                "offset": {"type": "integer", "description": "How many matching elements to pass over first (default 0)."},
                "limit": {"type": "integer", "description": "How many to list (default 500, at most 2,000)."}
            }),
            &[],
        ),
        "pointer" => (
            "Drive the active pointer tool like the mouse (run_command tool.select chooses the tool). Events run in order; a drag is one undo step. Positions are font units of the edited glyph, or window points with space: screen. Returns the tool, whether a gesture is unfinished (busy), the selection, what the canvas draws for the tool, and the errors of commands that failed.",
            json!({
                "events": {"type": "array", "description": "At most 10,000 events.", "items": {"type": "object", "required": ["kind", "x", "y"], "properties": {
                    "kind": {"type": "string", "enum": ["down", "drag", "up", "move", "doubleclick"], "description": "move is hover; send doubleclick after the two clicks' down and up."},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                    "space": {"type": "string", "enum": ["font", "screen"], "description": "font (default): font units of the edited glyph, or of the laid-out text in text mode. screen: window points."},
                    "mods": mods.clone()
                }}}
            }),
            &["events"],
        ),
        "click" => (
            "Press a control of the open dialog by its id (ok, cancel, another button, a tab, an action or a field, as dialog_inspect lists them), click an element of a window (see elements), or click at a window point given as x and y in points from the top left. A window click reaches any widget and is handed to the window over the next frames: take a screenshot afterwards to see the result.",
            json!({
                "id": {"type": "string", "description": "A dialog control. Give one of id, element, or x and y."},
                "element": {"type": "string", "description": "An element id from elements; the middle of what shows of it is clicked."},
                "x": {"type": "number", "description": "Window points from the left."},
                "y": {"type": "number", "description": "Window points from the top."},
                "button": {"type": "string", "enum": ["left", "right", "middle"], "description": "Default left; right opens the canvas context menu."},
                "count": {"type": "integer", "description": "Clicks, 1 to 16; 2 is a double click."},
                "mods": mods.clone(),
                "shift": flag("Shift"),
                "alt": flag("Alt (Option)"),
                "cmd": flag("Command on a Mac, Control elsewhere"),
                "ctrl": flag("Control")
            }),
            &[],
        ),
        "set" => (
            "Give an element a value, through the mouse and the keyboard. What it takes is the element's set (see elements): bool takes true or false, text a text or a number, choice the label of an option, number a number between the element's min and max, reached as closely as a drag of the slider can. Returns changed, and for a slider the value reached.",
            json!({
                "element": {"type": "string", "description": "An element id from elements."},
                "value": {"description": "true or false, a text, a number, or the label of an option."}
            }),
            &["element", "value"],
        ),
        "key" => (
            "Press a key as the keyboard does: the active tool's keys first (V Select, P Draw, H Hand, arrows nudge, Delete, Escape), then the menu shortcuts. While a dialog is open the key goes to it. Returns whether anything took the key and, when a menu shortcut ran, its command and result.",
            json!({
                "key": {"type": "string", "description": "A character, or a name: ArrowLeft, ArrowRight, ArrowUp, ArrowDown, Return, Delete, Backspace, Tab, Escape, Space, Home, End."},
                "shift": flag("Shift"),
                "alt": flag("Alt (Option)"),
                "cmd": flag("Command on a Mac, Control elsewhere"),
                "ctrl": flag("Control"),
                "action": {"type": "string", "enum": ["tap", "press", "release"], "description": "tap (default) presses and releases; hold Space with press, send pointer events, then release."}
            }),
            &["key"],
        ),
        "text" => (
            "Type text into the focused text field (a dialog field, or the tab's text in text mode). With no field focused, each character is a key tap.",
            json!({"text": {"type": "string"}}),
            &["text"],
        ),
        "menu" => (
            "Without command: the menu tree, each item with its command id, label, shortcut and whether it is enabled. With command: run that menu item as a click on it does; a follow-up question opens as a dialog (see dialog_inspect).",
            json!({
                "command": {"type": "string", "description": "The command id of a menu item."},
                "params": {"type": "object", "description": "The command's parameters."}
            }),
            &[],
        ),
        "dialog_inspect" => (
            "Describe the open dialog: its command, its fields (id, kind, value, limits, choices), the parameters OK runs, its buttons, tabs and actions, and any error. open is false when no dialog is open. Open a dialog with run_command dialog.open.",
            json!({}),
            &[],
        ),
        "dialog_set" => (
            "Give a field of the open dialog a value; the dialog previews the result on the canvas. Returns the dialog as dialog_inspect does.",
            json!({
                "field": {"type": "string", "description": "A field id from dialog_inspect."},
                "value": {"description": "The value: a number (or its text), true or false, a string, a choice word, or a list, as the field's kind needs."}
            }),
            &["field", "value"],
        ),
        "dialog_confirm" => (
            "Press OK on the open dialog: the previewed change becomes one undo step, or, for a dialog that runs its command on OK (export), the command runs once. A failure keeps the dialog open with the message in its error.",
            json!({}),
            &[],
        ),
        "dialog_cancel" => ("Press Cancel on the open dialog: the font goes back to how it was when the dialog opened.", json!({}), &[]),
        _ => ("", json!({}), &[]),
    };
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), properties);
    if !required.is_empty() {
        schema.insert("required".into(), json!(required));
    }
    json!({"name": name, "description": description, "inputSchema": schema})
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::{Backend, Headless, Policy};

    /// A running app that answers control methods with canned replies and records each call.
    struct FakeApp {
        calls: Rc<RefCell<Vec<(String, Value)>>>,
        /// The command the open dialog runs; `None`: no dialog is open.
        dialog_command: Option<&'static str>,
    }

    impl Backend for FakeApp {
        fn execute(&mut self, command: &str, _params: &Value) -> Result<Value, String> {
            Err(format!("unknown command: {command}"))
        }
        fn commands(&mut self, _pattern: Option<&str>) -> Result<Value, String> {
            Ok(json!({"commands": [
                {"id": "glyph.newGlyph", "label": "New Glyph", "query": false},
                {"id": "view.zoomIn", "label": "Zoom In", "query": false, "ui": true}
            ]}))
        }
        fn inspect(&mut self) -> Result<Value, String> {
            Ok(json!({"documents": [], "active": null}))
        }
        fn describe(&self) -> String {
            "a fake app".into()
        }
        fn has_app(&self) -> bool {
            true
        }
        fn control(&mut self, method: &str, params: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push((method.to_string(), params.clone()));
            Ok(match method {
                "ui.screenshot" => json!({"width": 4, "height": 2, "pixelsPerPoint": 2.0, "png": "iVBORw0KGgo="}),
                "ui.render" => json!({"width": 8, "height": 8, "glyphs": ["A"], "png": "iVBORw0KGgo="}),
                "ui.dialog.inspect" => match self.dialog_command {
                    Some(command) => json!({"open": true, "command": command, "params": {}}),
                    None => json!({"open": false}),
                },
                _ => json!({"method": method}),
            })
        }
    }

    type Calls = Rc<RefCell<Vec<(String, Value)>>>;

    fn attached(policy: Policy, dialog_command: Option<&'static str>) -> (Server, Calls) {
        let calls = Calls::default();
        let app = FakeApp { calls: calls.clone(), dialog_command };
        (Server::with_policy(Box::new(app), policy), calls)
    }

    fn ask(s: &mut Server, method: &str, params: Value) -> Value {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        serde_json::from_str(&s.handle_line(&request.to_string()).unwrap()).unwrap()
    }

    fn names(s: &mut Server) -> Vec<String> {
        let list = ask(s, "tools/list", json!({}));
        list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
        ask(s, "tools/call", json!({"name": name, "arguments": arguments}))["result"].clone()
    }

    #[test]
    fn the_tool_list_depends_on_the_mode() {
        let mut headless = Server::new(Box::new(Headless::with_document()));
        assert_eq!(names(&mut headless), ["list_commands", "run_command", "inspect_document", "render"]);

        let (mut s, _) = attached(Policy::unrestricted(), None);
        let all = names(&mut s);
        assert_eq!(all.len(), 3 + TOOLS.len());
        assert!(
            ["screenshot", "render", "elements", "pointer", "click", "set", "key", "text", "menu", "dialog_confirm"]
                .iter()
                .all(|t| all.iter().any(|n| n == t)),
            "{all:?}"
        );
        for tool in ask(&mut s, "tools/list", json!({}))["result"]["tools"].as_array().unwrap() {
            assert!(!tool["description"].as_str().unwrap().is_empty(), "{tool}");
        }

        let (mut s, _) = attached(Policy::unrestricted().deny("glyph.*").unwrap(), None);
        let restricted = names(&mut s);
        assert!(restricted.iter().any(|n| n == "screenshot") && restricted.iter().any(|n| n == "menu"), "{restricted:?}");
        assert!(!restricted.iter().any(|n| ["pointer", "click", "set", "key", "text"].contains(&n.as_str())), "{restricted:?}");
    }

    #[test]
    fn headless_window_tools_say_they_need_the_app() {
        let mut s = Server::new(Box::new(Headless::with_document()));
        let r = call(&mut s, "screenshot", json!({}));
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("needs the running app"), "{r}");
        // A name that is no tool at all is still a protocol error.
        let r = ask(&mut s, "tools/call", json!({"name": "screenshots", "arguments": {}}));
        assert_eq!(r["error"]["code"], crate::INVALID_PARAMS);
    }

    #[test]
    fn screenshot_and_render_return_images_and_write_no_file() {
        let (mut s, calls) = attached(Policy::unrestricted(), None);
        let r = call(&mut s, "screenshot", json!({"path": "/tmp/x.png"}));
        assert_eq!(r["isError"], false, "{r}");
        assert_eq!(r["content"][0]["type"], "image");
        assert_eq!(r["content"][0]["mimeType"], "image/png");
        assert_eq!(r["content"][0]["data"], "iVBORw0KGgo=");
        let info: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(info, json!({"width": 4, "height": 2, "pixelsPerPoint": 2.0}));

        let r = call(&mut s, "render", json!({"glyphs": ["A"], "size": 64, "path": "out.png"}));
        assert_eq!(r["content"][0]["type"], "image", "{r}");
        let calls = calls.borrow();
        assert_eq!(calls[0], ("ui.screenshot".to_string(), json!({})));
        assert_eq!(calls[1], ("ui.render".to_string(), json!({"glyphs": ["A"], "size": 64})));
    }

    /// An agent without a window opens a font, builds an accented glyph, looks at it, compares it with the state before
    /// and exports.
    #[test]
    fn headless_render_draws_compares_and_follows_an_agents_edits() {
        let mut s = Server::new(Box::new(Headless::with_document()));
        let command = |s: &mut Server, id: &str, params: Value| {
            let r = call(s, "run_command", json!({"command": id, "params": params}));
            assert_eq!(r["isError"], false, "{id}: {r}");
        };
        let rectangle = |glyph: &str, y: i64| json!({"glyph": glyph, "x": 100, "y": y, "width": 200, "height": 100});
        command(&mut s, "path.addRectangle", rectangle("A", 0));
        command(&mut s, "glyph.newGlyph", json!({"name": "acute", "width": 400}));
        command(&mut s, "path.addRectangle", rectangle("acute", 600));
        command(&mut s, "glyph.newGlyph", json!({"name": "Aacute", "width": 600}));
        command(&mut s, "glyph.addComponent", json!({"glyph": "Aacute", "component": "A"}));
        command(&mut s, "glyph.addComponent", json!({"glyph": "Aacute", "component": "acute"}));

        let drawn = call(&mut s, "render", json!({"glyphs": ["Aacute"], "size": 64}));
        assert_eq!((drawn["isError"].clone(), drawn["content"][0]["type"].clone()), (json!(false), json!("image")), "{drawn}");
        assert_eq!(drawn["content"][0]["data"], call(&mut s, "render", json!({"glyphs": ["Aacute"], "size": 64}))["content"][0]["data"]);

        command(&mut s, "path.addRectangle", rectangle("A", 300));
        let compared = call(&mut s, "render", json!({"compare": {"steps": 1}, "size": 64}));
        assert_eq!(compared["content"][0]["type"], "image", "{compared}");
        let fields: Value = serde_json::from_str(compared["content"][1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(fields["changed"], json!(["A"]), "the glyph edited, not the composite that uses it");
        let same = call(&mut s, "render", json!({"compare": {"steps": 0}}));
        assert_eq!((same["isError"].clone(), same["content"][0]["type"].clone()), (json!(false), json!("text")), "{same}");
        assert_eq!(call(&mut s, "render", json!({"compare": {"steps": 10_000}}))["isError"], true);

        let target = std::env::temp_dir().join(format!("fontcraft-render-{}.ttf", std::process::id()));
        command(&mut s, "file.exportStatic", json!({"destination": target.to_string_lossy()}));
        assert!(std::fs::remove_file(&target).is_ok());
    }

    #[test]
    fn input_tools_forward_their_arguments() {
        let (mut s, calls) = attached(Policy::unrestricted(), None);
        assert_eq!(call(&mut s, "key", json!({"key": "V", "shift": true}))["isError"], false);
        assert_eq!(call(&mut s, "click", json!({"x": 10, "y": 20, "button": "right"}))["isError"], false);
        assert_eq!(call(&mut s, "elements", json!({"prefix": "Family", "path": "x"}))["isError"], false);
        assert_eq!(call(&mut s, "click", json!({"element": "00ff"}))["isError"], false);
        assert_eq!(call(&mut s, "set", json!({"element": "00ff", "value": "Sans", "layers": true}))["isError"], false);
        assert_eq!(call(&mut s, "set", json!({"layers": true}))["isError"], true);
        assert_eq!(call(&mut s, "menu", json!({}))["isError"], false);
        assert_eq!(call(&mut s, "menu", json!({"command": "glyph.newGlyph", "params": {"name": "a"}}))["isError"], false);
        assert_eq!(call(&mut s, "dialog_confirm", json!({}))["isError"], false);
        let r = call(&mut s, "text", json!("abc"));
        assert_eq!(r["isError"], true, "{r}");
        let calls = calls.borrow();
        let sent: Vec<(&str, &Value)> = calls.iter().map(|(m, p)| (m.as_str(), p)).collect();
        assert_eq!(
            sent,
            [
                ("ui.key", &json!({"key": "V", "shift": true})),
                ("ui.click", &json!({"x": 10, "y": 20, "button": "right"})),
                ("ui.elements", &json!({"prefix": "Family"})),
                ("ui.click", &json!({"element": "00ff"})),
                ("ui.set", &json!({"element": "00ff", "value": "Sans"})),
                ("ui.menu.list", &json!({})),
                ("ui.menu.invoke", &json!({"command": "glyph.newGlyph", "params": {"name": "a"}})),
                ("ui.dialog.confirm", &json!({})),
            ]
        );
    }

    #[test]
    fn a_restricted_server_checks_what_the_window_tools_would_run() {
        let (mut s, calls) = attached(Policy::unrestricted().deny("glyph.*").unwrap(), Some("glyph.newGlyph"));
        let r = call(&mut s, "key", json!({"key": "Delete"}));
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("mouse and keyboard"), "{r}");

        let r = call(&mut s, "menu", json!({"command": "glyph.newGlyph"}));
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("--deny"), "{r}");
        assert_eq!(call(&mut s, "menu", json!({"command": "view.zoomIn"}))["isError"], false);

        // The open dialog runs a refused command: OK is refused, Cancel still works.
        assert_eq!(call(&mut s, "dialog_confirm", json!({}))["isError"], true);
        assert_eq!(call(&mut s, "dialog_set", json!({"field": "name", "value": "b"}))["isError"], true);
        assert_eq!(call(&mut s, "dialog_cancel", json!({}))["isError"], false);
        assert_eq!(call(&mut s, "screenshot", json!({}))["isError"], false);

        let methods: Vec<String> = calls.borrow().iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(methods, ["ui.menu.invoke", "ui.dialog.inspect", "ui.dialog.inspect", "ui.dialog.cancel", "ui.screenshot"]);
    }
}
