//! Window input for the control channel (`docs/control-protocol.md`).
//!
//! - `ui.click {x, y}`, `ui.drag`, `ui.wheel` and `ui.move` become egui events at window points,
//!   so they reach every widget a mouse reaches: Font View cells, panel fields, lists, the canvas
//!   (a right-click there opens its context menu). The events wait in [`SyntheticInput`] and the
//!   host hands them to egui one per frame ([`FontcraftApp::raw_input_hook`]), so egui sees a press,
//!   the moves of a drag and the release as separate input, as it does from a real mouse.
//! - The modifiers a request names are held for the frames its input spans: a click's press and
//!   release, a drag from the press through every move to the release. The keyboard's own
//!   modifiers come back after.
//! - `ui.contextMenu.list` and `ui.contextMenu.invoke` read and run the canvas context menu.
//! - `ui.set` turns view switches on or off by running their menu commands, so the result is the
//!   menu's. `ui.resize` and `ui.focus` ask the window through egui's viewport commands.
//!
//! The limits are the sibling Crafting Apps': at most [`MAX_DRAG_STEPS`] steps, [`MAX_CLICK_COUNT`]
//! clicks and [`MAX_SYNTHETIC_EVENTS`] events waiting. A request that does not fit is refused whole,
//! before any of its events is queued.

use std::collections::VecDeque;

use fontcraft_engine::{file_access, guard};
use fontcraft_geom::Point;
use fontcraft_tools::Modifiers;
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::shell::{context_menu, is_ui_command, menus};
use crate::tools_glue::Space;

/// Most moves between the press and the release of one `ui.drag`.
pub const MAX_DRAG_STEPS: u64 = 256;
/// Most clicks one `ui.click` or `ui.drag` makes.
pub const MAX_CLICK_COUNT: u64 = 16;
/// Most events waiting to be handed to egui, across all requests.
pub const MAX_SYNTHETIC_EVENTS: usize = 1024;
/// Most wheel notches (or points, with `unit: "point"`) one `ui.wheel` turns along each axis.
pub const MAX_WHEEL: f64 = 100.0;
/// Largest window coordinate a request may name, in points from the window's top left.
pub const MAX_COORDINATE: f64 = 100_000.0;
/// Largest font-unit coordinate `ui.contextMenu.list` takes.
const MAX_FONT_COORDINATE: f64 = 10_000_000.0;
/// The window sizes `ui.resize` takes, in points.
pub const MIN_WINDOW: f64 = 200.0;
pub const MAX_WINDOW: f64 = 16_384.0;
/// Most `ui.resize` and `ui.focus` requests waiting for the next frame.
const MAX_VIEWPORT_COMMANDS: usize = 16;
/// Most keys one `ui.set` takes.
const MAX_SET_KEYS: usize = 64;
/// Steps of a `ui.drag` that names none.
const DEFAULT_DRAG_STEPS: u64 = 8;

/// Control-channel input waiting for the next frames.
#[derive(Default)]
pub struct SyntheticInput {
    events: VecDeque<egui::Event>,
    viewport: Vec<egui::ViewportCommand>,
    /// The modifiers the keyboard holds, from the host's input, given back after synthetic input.
    host_modifiers: egui::Modifiers,
    /// Synthetic modifiers are in force in egui.
    holding: bool,
    /// The frame being drawn carries synthetic input.
    frame: bool,
}

impl SyntheticInput {
    /// How many events wait to be handed to egui.
    pub fn pending(&self) -> usize {
        self.events.len()
    }

    /// Room for `additional` more events, or the error that refuses the whole request.
    fn reserve(&mut self, additional: usize) -> Result<(), String> {
        if additional > MAX_SYNTHETIC_EVENTS.saturating_sub(self.events.len()) {
            return Err(format!("the input queue holds at most {MAX_SYNTHETIC_EVENTS} events; retry once the waiting input has been handed out"));
        }
        self.events.try_reserve(additional).map_err(|e| format!("cannot queue the input: {e}"))
    }

    fn queue_viewport(&mut self, command: egui::ViewportCommand) -> Result<(), String> {
        if self.viewport.len() >= MAX_VIEWPORT_COMMANDS {
            return Err("too many window requests are waiting for a frame; retry after the next frame".into());
        }
        self.viewport.push(command);
        Ok(())
    }
}

impl FontcraftApp {
    /// Hands the next waiting control-channel event to egui. The host calls it before every frame
    /// (eframe's `raw_input_hook`); headless hosts call it before `ui`. Pointer and wheel events go
    /// one per frame, each button press or release after a move to its position, so hover, clicks,
    /// double clicks and drags register as they do from a mouse.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        let input = &mut self.input;
        for event in &raw.events {
            match event {
                egui::Event::ModifiersChanged(m) => input.host_modifiers = *m,
                egui::Event::WindowFocused(false) => input.host_modifiers = egui::Modifiers::NONE,
                _ => {}
            }
        }
        input.frame = !input.events.is_empty();
        let Some(event) = input.events.pop_front() else {
            if std::mem::take(&mut input.holding) {
                raw.events.push(egui::Event::ModifiersChanged(input.host_modifiers));
            }
            return;
        };
        if let egui::Event::PointerButton { pos, .. } = &event {
            raw.events.push(egui::Event::PointerMoved(*pos));
        }
        // This frame's button or wheel modifiers, else those of a drag still holding its button.
        let held = match &event {
            egui::Event::PointerButton { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. } => Some(*modifiers),
            _ => match input.events.iter().find(|e| matches!(e, egui::Event::PointerButton { .. })) {
                Some(egui::Event::PointerButton { pressed: false, modifiers, .. }) => Some(*modifiers),
                _ => None,
            },
        };
        match held {
            Some(m) => {
                raw.events.push(egui::Event::ModifiersChanged(m));
                input.holding = true;
            }
            None => {
                if std::mem::take(&mut input.holding) {
                    raw.events.push(egui::Event::ModifiersChanged(input.host_modifiers));
                }
            }
        }
        raw.events.push(event);
    }

    /// Draws one frame as [`FontcraftApp::ui`] does, for a host with a control channel: it sends
    /// the window requests waiting (`ui.resize`, `ui.focus`), and a frame that carries
    /// control-channel input runs as a control request does, inside the automation roots and with
    /// exports kept inside their destination. A bug in drawing returns its message.
    pub fn ui_with_control_input(&mut self, ui: &mut egui::Ui) -> Result<(), String> {
        let ctx = ui.ctx().clone();
        for command in std::mem::take(&mut self.input.viewport) {
            ctx.send_viewport_cmd(command);
        }
        let result = if self.input.frame {
            let roots = self.automation_roots.clone();
            let outside = self.session.export_outside();
            self.session.set_export_outside(false);
            let result = file_access::confine(roots.as_ref(), || guard::catch_panic(|| self.ui(ui)));
            self.session.set_export_outside(outside);
            result
        } else {
            guard::catch_panic(|| self.ui(ui))
        };
        if !self.input.events.is_empty() {
            ctx.request_repaint();
        }
        result
    }
}

/// The egui modifiers for control-channel modifiers: `cmd` is Command on the Mac and the command
/// key (Control) elsewhere; `ctrl` is the Control key itself.
fn egui_modifiers(m: Modifiers) -> egui::Modifiers {
    let mac = cfg!(target_os = "macos");
    egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: mac && m.cmd, command: m.cmd }
}

/// A window coordinate `key` of `params`.
fn coordinate(method: &str, params: &Value, key: &str) -> Result<f32, String> {
    let v = params.get(key).and_then(Value::as_f64).ok_or_else(|| format!("{method}: `{key}` (a window coordinate in points) is required"))?;
    if !v.is_finite() || v.abs() > MAX_COORDINATE {
        return Err(format!("{method}: `{key}` must be between -{MAX_COORDINATE} and {MAX_COORDINATE}"));
    }
    Ok(v as f32)
}

/// A count `key` of `params`: a non-negative integer up to `max`; zero acts as one.
fn count(method: &str, params: &Value, key: &str, default: u64, max: u64) -> Result<usize, String> {
    let n = match params.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v.as_u64().ok_or_else(|| format!("{method}: `{key}` must be a non-negative integer"))?,
    };
    if n > max {
        return Err(format!("{method}: `{key}` must be at most {max}"));
    }
    usize::try_from(n.max(1)).map_err(|_| format!("{method}: `{key}` is out of range"))
}

fn button(method: &str, params: &Value) -> Result<egui::PointerButton, String> {
    match params.get("button") {
        None | Some(Value::Null) => Ok(egui::PointerButton::Primary),
        Some(Value::String(b)) => match b.as_str() {
            "left" | "primary" => Ok(egui::PointerButton::Primary),
            "right" | "secondary" => Ok(egui::PointerButton::Secondary),
            "middle" => Ok(egui::PointerButton::Middle),
            _ => Err(format!("{method}: `button` must be left, right or middle")),
        },
        Some(_) => Err(format!("{method}: `button` must be left, right or middle")),
    }
}

fn queued(app: &FontcraftApp, events: usize) -> Value {
    json!({"queued": events, "pending": app.input.pending()})
}

/// `ui.click {x, y, button?, count?, mods?}` and `ui.drag {x, y, toX, toY, steps?, count?, button?,
/// mods?}`: a move to (x, y), the press, the moves of a drag, the release, then the further clicks
/// of `count` at the end point.
pub fn click_or_drag(app: &mut FontcraftApp, method: &str, params: &Value) -> Result<Value, String> {
    let drag = method == "ui.drag";
    let steps = if drag { count(method, params, "steps", DEFAULT_DRAG_STEPS, MAX_DRAG_STEPS)? } else { 0 };
    let clicks = count(method, params, "count", 1, MAX_CLICK_COUNT)?;
    let button = button(method, params)?;
    let modifiers = egui_modifiers(crate::control::modifiers_and_flags(method, params)?);
    let from = egui::pos2(coordinate(method, params, "x")?, coordinate(method, params, "y")?);
    let to = if drag { egui::pos2(coordinate(method, params, "toX")?, coordinate(method, params, "toY")?) } else { from };
    let events = clicks
        .checked_mul(2)
        .and_then(|n| n.checked_add(steps))
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| format!("{method}: too many events"))?;
    let input = &mut app.input;
    input.reserve(events)?;
    input.events.push_back(egui::Event::PointerMoved(from));
    input.events.push_back(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers });
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        input.events.push_back(egui::Event::PointerMoved(from + (to - from) * t));
    }
    input.events.push_back(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers });
    for _ in 1..clicks {
        input.events.push_back(egui::Event::PointerButton { pos: to, button, pressed: true, modifiers });
        input.events.push_back(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers });
    }
    Ok(queued(app, events))
}

/// `ui.move {x, y}`: the pointer moves to a window point (hover).
pub fn move_pointer(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.move";
    let at = egui::pos2(coordinate(M, params, "x")?, coordinate(M, params, "y")?);
    app.input.reserve(1)?;
    app.input.events.push_back(egui::Event::PointerMoved(at));
    Ok(queued(app, 1))
}

/// `ui.wheel {x, y, dx?, dy?, unit?, mods?}`: a wheel turn over a window point. `dy` positive turns
/// up (the content moves down), `dx` positive moves the content right.
pub fn wheel(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.wheel";
    let at = egui::pos2(coordinate(M, params, "x")?, coordinate(M, params, "y")?);
    let amount = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && n.abs() <= MAX_WHEEL => Ok(n as f32),
            _ => Err(format!("{M}: `{key}` must be a number between -{MAX_WHEEL} and {MAX_WHEEL}")),
        },
    };
    let delta = egui::vec2(amount("dx")?, amount("dy")?);
    let unit = match params.get("unit").and_then(Value::as_str) {
        None | Some("line") => egui::MouseWheelUnit::Line,
        Some("point") => egui::MouseWheelUnit::Point,
        Some(_) => return Err(format!("{M}: `unit` must be line or point")),
    };
    let modifiers = egui_modifiers(crate::control::modifiers_and_flags(M, params)?);
    app.input.reserve(2)?;
    app.input.events.push_back(egui::Event::PointerMoved(at));
    app.input.events.push_back(egui::Event::MouseWheel { unit, delta, phase: egui::TouchPhase::Move, modifiers });
    Ok(queued(app, 2))
}

/// `ui.resize {width, height}`: the window's inner size in points, at the next frame.
pub fn resize(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.resize";
    let size = |key: &str| match params.get(key).and_then(Value::as_f64) {
        Some(v) if (MIN_WINDOW..=MAX_WINDOW).contains(&v) => Ok(v as f32),
        _ => Err(format!("{M}: `{key}` must be a number of points from {MIN_WINDOW} to {MAX_WINDOW}")),
    };
    let (width, height) = (size("width")?, size("height")?);
    app.input.queue_viewport(egui::ViewportCommand::InnerSize(egui::vec2(width, height))).map_err(|e| format!("{M}: {e}"))?;
    Ok(json!({"width": width, "height": height}))
}

/// `ui.focus {}`: brings the window to the front and gives it the keyboard, at the next frame.
pub fn focus(app: &mut FontcraftApp) -> Result<Value, String> {
    app.input.queue_viewport(egui::ViewportCommand::Focus).map_err(|e| format!("ui.focus: {e}"))?;
    Ok(json!({"focus": true}))
}

/// The `ui.set` key of a switch command: the part after the area, without a leading `show`,
/// `set` or `toggle` (`view.showBoundingBox` → `boundingBox`, `window.palette` → `palette`).
pub fn set_key(id: &str) -> String {
    if id == "audit.showPanel" {
        return "auditPanel".into();
    }
    let tail = id.split_once('.').map_or(id, |(_, t)| t);
    for prefix in ["show", "set", "toggle"] {
        if let Some(rest) = tail.strip_prefix(prefix)
            && let Some(first) = rest.chars().next()
            && first.is_ascii_uppercase()
        {
            return format!("{}{}", first.to_ascii_lowercase(), rest.get(first.len_utf8()..).unwrap_or(""));
        }
    }
    tail.to_string()
}

/// The switch a `ui.set` key turns, and whether the key means its opposite (`outline` is Fill
/// Preview off). A switch's command id is a key too.
fn switch_for(key: &str) -> Option<(&'static str, bool)> {
    if key == "outline" {
        return Some(("view.fillPreview", true));
    }
    menus::SWITCHES.iter().copied().find(|id| *id == key || set_key(id) == key).map(|id| (id, false))
}

/// `ui.set {key: bool, …}`: turns view switches on or off with their menu commands. Every key is
/// checked before the first switch moves; when a command fails, the switches already moved are
/// put back. Returns each key's value afterwards.
pub fn set(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.set";
    let keys = params
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| format!("{M}: params must be an object of switches, such as {{\"boundingBox\": true}}"))?;
    if keys.len() > MAX_SET_KEYS {
        return Err(format!("{M}: at most {MAX_SET_KEYS} keys"));
    }
    let mut plan = Vec::with_capacity(keys.len());
    for (key, value) in keys {
        let (id, inverted) = switch_for(key).ok_or_else(|| {
            let known: Vec<String> = menus::SWITCHES.iter().map(|id| set_key(id)).collect();
            format!("{M}: unknown key `{key}`; the keys are outline, {}", known.join(", "))
        })?;
        let on = value.as_bool().ok_or_else(|| format!("{M}: `{key}` must be true or false"))?;
        if app.dialog.is_some() && !is_ui_command(id) {
            return Err(format!("{M}: `{key}` cannot change while a dialog is open"));
        }
        plan.push((key.as_str(), id, on != inverted));
    }
    let mut moved: Vec<&'static str> = Vec::new();
    let mut failure = None;
    for (key, id, want) in &plan {
        if menus::checked(&app.session, &app.state, id) == Some(*want) {
            continue;
        }
        if let Err(e) = app.run_menu_command(id, json!({})) {
            failure = Some(format!("{M}: `{key}`: {e}"));
            break;
        }
        moved.push(*id);
        if menus::checked(&app.session, &app.state, id) != Some(*want) {
            failure = Some(format!("{M}: `{key}` cannot be turned {} now", if *want { "on" } else { "off" }));
            break;
        }
    }
    if let Some(e) = failure {
        for id in moved.into_iter().rev() {
            let _ = app.run_menu_command(id, json!({}));
        }
        return Err(e);
    }
    let mut result = Map::new();
    for (key, id, _) in &plan {
        let on = menus::checked(&app.session, &app.state, id).unwrap_or(false);
        let inverted = switch_for(key).is_some_and(|(_, inv)| inv);
        result.insert((*key).to_string(), json!(on != inverted));
    }
    Ok(Value::Object(result))
}

/// `ui.contextMenu.list {x?, y?, space?}`: the open canvas context menu, `{open: false}` when none
/// is. With a position it first opens the menu there, as a right-click does.
pub fn context_menu_list(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.contextMenu.list";
    if params.get("x").is_some() || params.get("y").is_some() {
        let number = |key: &str| match params.get(key).and_then(Value::as_f64) {
            Some(v) if v.is_finite() && v.abs() <= MAX_FONT_COORDINATE => Ok(v),
            _ => Err(format!("{M}: `{key}` must be a number between -{MAX_FONT_COORDINATE} and {MAX_FONT_COORDINATE}")),
        };
        let at = Point::new(number("x")?, number("y")?);
        let space = match params.get("space").and_then(Value::as_str) {
            None | Some("font") => Space::Font,
            Some("screen") => Space::Screen,
            Some(_) => return Err(format!("{M}: `space` must be font or screen")),
        };
        if app.state.tab().is_none() {
            return Err(format!("{M}: no Edit View tab is showing (open a glyph first)"));
        }
        context_menu::open(app, at, space).map_err(|e| format!("{M}: {e}"))?;
    }
    let Some(menu) = &app.tools.context_menu else { return Ok(json!({"open": false})) };
    let mut value = serde_json::to_value(menu).map_err(|e| format!("{M}: {e}"))?;
    if let Some(map) = value.as_object_mut() {
        map.insert("open".into(), json!(true));
    }
    Ok(value)
}

/// `ui.contextMenu.invoke {command}`: runs an item of the open canvas context menu as a click on
/// it does, and closes the menu.
pub fn context_menu_invoke(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.contextMenu.invoke";
    let id = params.get("command").and_then(Value::as_str).ok_or_else(|| format!("{M}: `command` (an item's command id) is required"))?;
    context_menu::invoke(app, id).map_err(|e| format!("{M}: {e}"))
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect};
    use fontcraft_tools::Selection;
    use serde_json::{Value, json};

    use super::*;
    use crate::control;

    /// A headless window that hands control input to egui as the desktop host does.
    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn new() -> Self {
            Self { ctx: egui::Context::default() }
        }

        fn frame(&self, app: &mut FontcraftApp) -> egui::FullOutput {
            let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            app.raw_input_hook(&mut raw);
            let mut output = self.ctx.run_ui(raw, |ui| {
                let _ = app.ui_with_control_input(ui);
            });
            output.textures_delta.clear();
            output
        }

        /// Frames until the waiting input is handed out, and `extra` more.
        fn settle(&self, app: &mut FontcraftApp, extra: usize) {
            let frames = app.input.pending() + extra;
            for _ in 0..frames {
                self.frame(app);
            }
        }
    }

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        control::handle(app, &json!({"id": 1, "method": method, "params": params}))
    }

    fn ok(reply: Value) -> Value {
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    }

    fn selected(app: &mut FontcraftApp) -> Vec<String> {
        let state = app.run("fontView.getState", &json!({})).unwrap();
        let mut names: Vec<String> = state["selected"].as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect();
        names.sort();
        names
    }

    fn cell(app: &FontcraftApp, glyph: &str) -> Pos2 {
        app.font_view.laid_out.iter().find(|(n, _)| n == glyph).unwrap_or_else(|| panic!("{glyph} was not laid out")).1.center()
    }

    #[test]
    fn a_click_at_a_window_point_selects_a_font_view_cell_and_holds_its_modifiers() {
        let mut app = FontcraftApp::default();
        let w = Window::new();
        w.frame(&mut app);
        let at = cell(&app, "C");
        let r = ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y})));
        assert_eq!(r["queued"], 3, "a move, the press and the release");
        assert_eq!(ok(call(&mut app, "ui.inspect", json!({})))["pendingInput"], 3);
        w.settle(&mut app, 1);
        assert_eq!(selected(&mut app), ["C"]);
        let at = cell(&app, "E");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y, "mods": ["cmd"]})));
        w.settle(&mut app, 1);
        assert_eq!(selected(&mut app), ["C", "E"], "Cmd held through the click adds to the selection");
        // The modifiers are given back once the click is over.
        let at = cell(&app, "A");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y})));
        w.settle(&mut app, 1);
        assert_eq!(selected(&mut app), ["A"]);
        // The dialog form of ui.click is still there.
        let r = call(&mut app, "ui.click", json!({"id": "ok"}));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().starts_with("ui.click:"), "{r}");
    }

    #[test]
    fn a_wheel_turn_over_font_view_scrolls_the_grid() {
        let mut app = FontcraftApp::default();
        app.session
            .edit("many", |font| {
                for i in 0..2_000 {
                    font.add_glyph(&format!("g{i:04}"), vec![], 500.0)?;
                }
                Ok(())
            })
            .unwrap();
        let w = Window::new();
        w.frame(&mut app);
        let first = app.font_view.laid_out.first().unwrap().clone();
        let at = first.1.center();
        ok(call(&mut app, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": -10})));
        // egui spreads a wheel turn over a few frames.
        w.settle(&mut app, 30);
        let now = app.font_view.laid_out.first().unwrap().clone();
        assert_ne!(now.0, first.0, "the grid scrolled down: the first row in view is another one");
        let r = call(&mut app, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 101}));
        assert_eq!(r["ok"], false, "a turn is at most {MAX_WHEEL} notches");
    }

    #[test]
    fn ui_set_moves_a_switch_as_its_menu_command_does() {
        let mut by_set = FontcraftApp::default();
        let mut by_menu = FontcraftApp::default();
        let r = ok(call(&mut by_set, "ui.set", json!({"boundingBox": true, "palette": false, "outline": true, "view.showHints": true})));
        assert_eq!(r, json!({"boundingBox": true, "palette": false, "outline": true, "view.showHints": true}));
        // Each of these switches starts in the other state, so its menu item flips it once.
        for id in ["view.showBoundingBox", "window.palette", "view.fillPreview", "view.showHints"] {
            by_menu.run_menu_command(id, json!({})).unwrap();
        }
        let state = |app: &FontcraftApp| (serde_json::to_value(app.state.display).unwrap(), app.state.show_palette);
        assert_eq!(state(&by_set), state(&by_menu));
        // Setting what is already set changes nothing.
        ok(call(&mut by_set, "ui.set", json!({"boundingBox": true})));
        assert_eq!(state(&by_set), state(&by_menu));
        // A bad key or value refuses the whole request.
        let before = state(&by_set);
        for bad in [json!({"guides": false, "grid": true}), json!({"guides": false, "metrics": 1}), json!({})] {
            assert_eq!(call(&mut by_set, "ui.set", bad)["ok"], false);
        }
        assert_eq!(state(&by_set), before);
    }

    #[test]
    fn every_switch_has_its_own_set_key() {
        let mut keys: Vec<String> = menus::SWITCHES.iter().map(|id| set_key(id)).collect();
        keys.push("outline".into());
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count, "two switches share a ui.set key");
        assert_eq!(set_key("view.showBoundingBox"), "boundingBox");
        assert_eq!(set_key("path.togglePowerNudge"), "powerNudge");
    }

    #[test]
    fn a_right_click_on_the_canvas_opens_the_context_menu_the_channel_lists() {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
        app.state.tab_mut().unwrap().selection = Selection::new();
        let w = Window::new();
        w.settle(&mut app, 2);
        assert_eq!(ok(call(&mut app, "ui.contextMenu.list", json!({}))), json!({"open": false}));
        let p = crate::tools_glue::current_view(&app).unwrap().to_screen(Point::new(300.0, 300.0));
        ok(call(&mut app, "ui.click", json!({"x": p.x, "y": p.y, "button": "right"})));
        w.settle(&mut app, 1);
        let menu = ok(call(&mut app, "ui.contextMenu.list", json!({})));
        assert_eq!(menu["open"], true);
        let contexts: Vec<&str> = menu["sections"].as_array().unwrap().iter().map(|s| s["context"].as_str().unwrap()).collect();
        assert_eq!(contexts.first(), Some(&context_menu::NODE), "{contexts:?}");
        let selection = serde_json::to_value(&app.state.tab().unwrap().selection).unwrap();
        assert_eq!(selection, json!({"points": [[0, 2]]}), "the right-click selected the node first");
        let r = call(&mut app, "ui.contextMenu.invoke", json!({"command": "no.such"}));
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn modifiers_held_through_a_click_hold_a_tool_only_until_they_are_given_back() {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        let w = Window::new();
        w.settle(&mut app, 2);
        assert_eq!(app.tools.toolbox.active_id(), "select");
        let p = crate::tools_glue::current_view(&app).unwrap().to_screen(Point::new(500.0, 500.0));
        ok(call(&mut app, "ui.click", json!({"x": p.x, "y": p.y, "mods": ["cmd", "alt", "ctrl"]})));
        // The move, the press and the release: Cmd+Alt+Ctrl holds the Measurement tool.
        for _ in 0..3 {
            w.frame(&mut app);
        }
        assert_eq!(app.tools.toolbox.active_id(), "measurement");
        // The next frame gives back the window's own modifiers, and the tool goes with them.
        w.settle(&mut app, 2);
        assert_eq!(app.tools.toolbox.active_id(), "select");
    }

    #[test]
    fn input_over_the_limits_is_refused_before_anything_is_queued() {
        let mut app = FontcraftApp::default();
        for (method, params) in [
            ("ui.click", json!({"x": 1, "y": 1, "count": 17})),
            ("ui.drag", json!({"x": 1, "y": 1, "toX": 2, "toY": 2, "steps": 257})),
            ("ui.drag", json!({"x": 1, "y": 1, "toX": 2, "toY": 2, "steps": -1})),
            ("ui.click", json!({"x": 1})),
            ("ui.click", json!({"x": 1, "y": 1e9})),
            ("ui.click", json!({"x": 1, "y": 1, "button": "fourth"})),
            ("ui.move", json!({"x": "1", "y": 1})),
            ("ui.resize", json!({"width": 100, "height": 600})),
            ("ui.resize", json!({"width": 800})),
        ] {
            let r = call(&mut app, method, params.clone());
            assert_eq!(r["ok"], false, "{method} {params}");
            assert_eq!(app.input.pending(), 0, "{method} {params} queued input");
        }
        // 289 events each: the fourth does not fit and leaves the queue as it was.
        let big = json!({"x": 1, "y": 1, "toX": 300, "toY": 300, "steps": 256, "count": 16});
        for _ in 0..3 {
            ok(call(&mut app, "ui.drag", big.clone()));
        }
        assert_eq!(app.input.pending(), 867);
        assert_eq!(call(&mut app, "ui.drag", big)["ok"], false);
        assert_eq!(app.input.pending(), 867);
        ok(call(&mut app, "ui.move", json!({"x": 5, "y": 5})));
        assert_eq!(app.input.pending(), 868);
    }

    #[test]
    fn resize_and_focus_reach_the_window_at_the_next_frame() {
        let mut app = FontcraftApp::default();
        ok(call(&mut app, "ui.resize", json!({"width": 900, "height": 700})));
        ok(call(&mut app, "ui.focus", json!({})));
        let output = Window::new().frame(&mut app);
        let commands = output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        assert!(commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(s) if *s == egui::vec2(900.0, 700.0))), "{commands:?}");
        assert!(commands.iter().any(|c| matches!(c, egui::ViewportCommand::Focus)), "{commands:?}");
    }
}
