//! Window input for the control channel (`docs/control-protocol.md`).
//!
//! - `ui.click {x, y}`, `ui.drag`, `ui.wheel` and `ui.move` become egui events at window points,
//!   so they reach every widget a mouse reaches: Font View cells, panel fields, lists, the canvas
//!   (a right-click there opens its context menu). The events wait in [`SyntheticInput`] and the
//!   host hands them to egui one per drawn frame ([`FontcraftApp::raw_input_hook`]), so egui sees
//!   a press, the moves of a drag and the release as separate input, as it does from a real mouse.
//! - The modifiers a request names are held for the frames its input spans: a click's press and
//!   release, a drag from the press through every move to the release. The keyboard's own
//!   modifiers come back after.
//! - The person's own mouse press or key press in the window drops the input still waiting, and
//!   so does the question about unsaved changes when it opens other than from that input.
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
/// The smallest window `ui.resize` takes, in points: the window's own minimum size.
pub const MIN_WINDOW_WIDTH: f64 = 640.0;
pub const MIN_WINDOW_HEIGHT: f64 = 400.0;
/// The largest size `ui.resize` takes, in points. Larger sizes are refused; sizes the window
/// cannot take are clamped ([`MAX_SURFACE`]).
pub const MAX_WINDOW: f64 = 16_384.0;
/// The largest window side `ui.resize` asks for, in points: the texture size the renderer asks
/// the graphics device for. A surface larger than the device's textures aborts the process.
pub const MAX_SURFACE: f32 = 8_192.0;
/// egui's texture side before the host reports the device's.
const DEFAULT_TEXTURE_SIDE: f32 = 2_048.0;
/// Most `ui.resize` and `ui.focus` requests waiting for the next frame.
const MAX_VIEWPORT_COMMANDS: usize = 16;
/// Most keys one `ui.set` takes.
const MAX_SET_KEYS: usize = 64;
/// Steps of a `ui.drag` that names none.
const DEFAULT_DRAG_STEPS: u64 = 8;

/// The sizes the window can take, from the last drawn frame.
#[derive(Clone, Copy, Debug)]
struct WindowLimits {
    /// The graphics device's largest texture side, in pixels.
    texture_side: f32,
    /// Pixels per point: the larger of the monitor's and egui's (which includes the zoom).
    pixels_per_point: f32,
    /// The monitor's size in points, when the host knows it.
    monitor: Option<egui::Vec2>,
}

impl Default for WindowLimits {
    fn default() -> Self {
        Self { texture_side: DEFAULT_TEXTURE_SIDE, pixels_per_point: 1.0, monitor: None }
    }
}

impl WindowLimits {
    fn of(ctx: &egui::Context) -> Self {
        ctx.input(|i| Self {
            texture_side: i.max_texture_side as f32,
            pixels_per_point: i.pixels_per_point.max(i.viewport().native_pixels_per_point.unwrap_or(0.0)),
            monitor: i.viewport().monitor_size,
        })
    }

    /// The largest inner size the window can take, in points.
    fn largest(&self) -> egui::Vec2 {
        let ppp = if self.pixels_per_point.is_finite() && self.pixels_per_point > 0.0 { self.pixels_per_point } else { 1.0 };
        let side = if self.texture_side.is_finite() && self.texture_side > 0.0 { self.texture_side / ppp } else { DEFAULT_TEXTURE_SIDE };
        let side = side.min(MAX_SURFACE);
        let mut size = egui::vec2(side, side);
        if let Some(monitor) = self.monitor.filter(|m| m.x.is_finite() && m.y.is_finite() && m.x > 0.0 && m.y > 0.0) {
            size = size.min(monitor);
        }
        size
    }

    fn clamp(&self, command: egui::ViewportCommand) -> egui::ViewportCommand {
        match command {
            egui::ViewportCommand::InnerSize(size) => egui::ViewportCommand::InnerSize(size.min(self.largest())),
            other => other,
        }
    }
}

/// Control-channel input waiting for the next frames.
#[derive(Default)]
pub struct SyntheticInput {
    events: VecDeque<egui::Event>,
    viewport: Vec<egui::ViewportCommand>,
    /// The modifiers the keyboard holds, from the host's input, given back after synthetic input.
    host_modifiers: egui::Modifiers,
    /// Synthetic modifiers are in force in egui.
    holding: bool,
    /// What the hook added to egui's input for the event in flight: handed out, not yet drawn by
    /// [`FontcraftApp::ui_with_control_input`]. Empty when no event is in flight. While the window
    /// is hidden the host runs passes without a ui and carries their input over to the next
    /// drawn pass, so these events can come back to the hook.
    in_flight: Vec<egui::Event>,
    limits: WindowLimits,
    /// The question about unsaved changes was open at the last look.
    prompt_open: bool,
    /// Whether each drawn pass ran confined, for the tests.
    #[cfg(test)]
    confined: Vec<bool>,
}

impl SyntheticInput {
    /// How many events wait to be drawn: those queued and the one handed to egui whose frame has
    /// not been drawn yet.
    pub fn pending(&self) -> usize {
        self.events.len() + usize::from(!self.in_flight.is_empty())
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

    /// Drops the waiting input when the question about unsaved changes has opened since the last
    /// look, unless `ours` (it opened in a frame drawing the channel's input).
    fn watch_close_prompt(&mut self, open: bool, ours: bool) {
        if open && !self.prompt_open && !ours {
            self.events.clear();
        }
        self.prompt_open = open;
    }
}

impl FontcraftApp {
    /// Hands the next waiting control-channel event to egui. The host calls it before every pass
    /// (eframe's `raw_input_hook`); headless hosts call it before `ui`. Pointer and wheel events go
    /// one per drawn frame, each button press or release after a move to its position, so hover,
    /// clicks, double clicks and drags register as they do from a mouse. No event is handed out
    /// while the one before it waits for [`Self::ui_with_control_input`] to draw it: a hidden
    /// window's passes run no ui and never collect a backlog.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        let prompt_open = self.close_prompt.is_open();
        let input = &mut self.input;
        let mut ours = input.in_flight.clone();
        let mut person_acts = false;
        for event in &raw.events {
            if let Some(at) = ours.iter().position(|e| e == event) {
                ours.swap_remove(at);
                continue;
            }
            match event {
                egui::Event::ModifiersChanged(m) => input.host_modifiers = *m,
                egui::Event::WindowFocused(false) => input.host_modifiers = egui::Modifiers::NONE,
                egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { pressed: true, .. } => person_acts = true,
                _ => {}
            }
        }
        // The person takes over: their press must not run inside the agent's frame.
        if person_acts {
            input.events.clear();
        }
        input.watch_close_prompt(prompt_open, false);
        if !input.in_flight.is_empty() {
            return;
        }
        let Some(event) = input.events.pop_front() else {
            if std::mem::take(&mut input.holding) {
                // First, so a modifier change of the host's in this input wins.
                raw.events.insert(0, egui::Event::ModifiersChanged(input.host_modifiers));
            }
            return;
        };
        let mut added = Vec::with_capacity(3);
        if let egui::Event::PointerButton { pos, .. } = &event {
            added.push(egui::Event::PointerMoved(*pos));
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
                added.push(egui::Event::ModifiersChanged(m));
                input.holding = true;
            }
            None => {
                if std::mem::take(&mut input.holding) {
                    added.push(egui::Event::ModifiersChanged(input.host_modifiers));
                }
            }
        }
        added.push(event);
        raw.events.extend(added.iter().cloned());
        input.in_flight = added;
    }

    /// Draws one frame as [`FontcraftApp::ui`] does, for a host with a control channel: it sends
    /// the window requests waiting (`ui.resize`, clamped to what the window can take, and
    /// `ui.focus`), and a frame that draws control-channel input runs as a control request does,
    /// inside the automation roots and with exports kept inside their destination. A bug in
    /// drawing returns its message.
    pub fn ui_with_control_input(&mut self, ui: &mut egui::Ui) -> Result<(), String> {
        let ctx = ui.ctx().clone();
        self.input.limits = WindowLimits::of(&ctx);
        for command in std::mem::take(&mut self.input.viewport) {
            ctx.send_viewport_cmd(self.input.limits.clamp(command));
        }
        // The question may have opened in the host's logic (a close request) or in a request.
        let open = self.close_prompt.is_open();
        self.input.watch_close_prompt(open, false);
        let synthetic = !std::mem::take(&mut self.input.in_flight).is_empty();
        let result = if synthetic {
            let roots = self.automation_roots.clone();
            let outside = self.session.export_outside();
            self.session.set_export_outside(false);
            let result = file_access::confine(roots.as_ref(), || {
                #[cfg(test)]
                self.input.confined.push(file_access::current().is_some());
                guard::catch_panic(|| self.ui(ui))
            });
            self.session.set_export_outside(outside);
            result
        } else {
            #[cfg(test)]
            self.input.confined.push(file_access::current().is_some());
            guard::catch_panic(|| self.ui(ui))
        };
        let open = self.close_prompt.is_open();
        self.input.watch_close_prompt(open, synthetic);
        if !self.input.events.is_empty() {
            ctx.request_repaint();
        }
        result
    }
}

/// The egui modifiers for control-channel modifiers, as the windowing layer reports the keys: on
/// the Mac `cmd` is Command and `ctrl` is Control; elsewhere both are the Control key, which is
/// also the command key.
fn egui_modifiers(m: Modifiers) -> egui::Modifiers {
    if cfg!(target_os = "macos") {
        egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: m.cmd, command: m.cmd }
    } else {
        let control = m.ctrl || m.cmd;
        egui::Modifiers { alt: m.alt, ctrl: control, shift: m.shift, mac_cmd: false, command: control }
    }
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
    let unit = match params.get("unit") {
        None | Some(Value::Null) => egui::MouseWheelUnit::Line,
        Some(Value::String(u)) if u == "line" => egui::MouseWheelUnit::Line,
        Some(Value::String(u)) if u == "point" => egui::MouseWheelUnit::Point,
        Some(_) => return Err(format!("{M}: `unit` must be line or point")),
    };
    let modifiers = egui_modifiers(crate::control::modifiers_and_flags(M, params)?);
    app.input.reserve(2)?;
    app.input.events.push_back(egui::Event::PointerMoved(at));
    app.input.events.push_back(egui::Event::MouseWheel { unit, delta, phase: egui::TouchPhase::Move, modifiers });
    Ok(queued(app, 2))
}

/// `ui.resize {width, height}`: the window's inner size in points, at the next frame. A size
/// larger than the monitor or than the graphics device can draw is clamped; the reply has the
/// size asked of the window.
pub fn resize(app: &mut FontcraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.resize";
    let size = |key: &str, min: f64| match params.get(key).and_then(Value::as_f64) {
        Some(v) if (min..=MAX_WINDOW).contains(&v) => Ok(v as f32),
        _ => Err(format!("{M}: `{key}` must be a number of points from {min} to {MAX_WINDOW}")),
    };
    let wanted = egui::vec2(size("width", MIN_WINDOW_WIDTH)?, size("height", MIN_WINDOW_HEIGHT)?);
    let size = wanted.min(app.input.limits.largest());
    app.input.queue_viewport(egui::ViewportCommand::InnerSize(size)).map_err(|e| format!("{M}: {e}"))?;
    Ok(json!({"width": size.x, "height": size.y, "clamped": size != wanted}))
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
    let mut plan: Vec<(&str, &'static str, bool)> = Vec::with_capacity(keys.len());
    for (key, value) in keys {
        let (id, inverted) = switch_for(key).ok_or_else(|| {
            let known: Vec<String> = menus::SWITCHES.iter().map(|id| set_key(id)).collect();
            format!("{M}: unknown key `{key}`; the keys are outline, {}", known.join(", "))
        })?;
        let on = value.as_bool().ok_or_else(|| format!("{M}: `{key}` must be true or false"))?;
        if app.dialog.is_some() && !is_ui_command(id) {
            return Err(format!("{M}: `{key}` cannot change while a dialog is open"));
        }
        let want = on != inverted;
        if let Some((other, ..)) = plan.iter().find(|(_, other_id, other_want)| *other_id == id && *other_want != want) {
            return Err(format!("{M}: `{key}` and `{other}` ask for opposite states of one switch"));
        }
        plan.push((key.as_str(), id, want));
    }
    // Show Indexes turns all its labels on or off; put back the ones that were on.
    let indexes = app.state.panels.edit_aids.indexes;
    let mut moved: Vec<(&'static str, Option<bool>)> = Vec::new();
    let mut failure = None;
    for (key, id, want) in &plan {
        let was = menus::checked(&app.session, &app.state, id);
        if was == Some(*want) {
            continue;
        }
        if let Err(e) = app.run_menu_command(id, json!({})) {
            failure = Some(format!("{M}: `{key}`: {e}"));
            break;
        }
        moved.push((*id, was));
        if menus::checked(&app.session, &app.state, id) != Some(*want) {
            failure = Some(format!("{M}: `{key}` cannot be turned {} now", if *want { "on" } else { "off" }));
            break;
        }
    }
    if let Some(e) = failure {
        for (id, was) in moved.into_iter().rev() {
            if menus::checked(&app.session, &app.state, id) != was {
                let _ = app.run_menu_command(id, json!({}));
            }
        }
        app.state.panels.edit_aids.indexes = indexes;
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
        if app.tools.toolbox.busy() || app.dialog.is_some() {
            return Err(format!("{M}: the menu does not open while a tool is dragging or a dialog is open"));
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

        fn raw() -> egui::RawInput {
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() }
        }

        fn frame(&self, app: &mut FontcraftApp) -> egui::FullOutput {
            let mut raw = Self::raw();
            app.raw_input_hook(&mut raw);
            self.draw(app, raw)
        }

        /// A drawn pass over `raw`, which has been through the hook.
        fn draw(&self, app: &mut FontcraftApp, raw: egui::RawInput) -> egui::FullOutput {
            let mut output = self.ctx.run_ui(raw, |ui| {
                let _ = app.ui_with_control_input(ui);
            });
            output.textures_delta.clear();
            output
        }

        /// Frames until `pendingInput` is 0, as a client polls it, then `extra` more.
        fn settle(&self, app: &mut FontcraftApp, extra: usize) {
            let mut frames = 0;
            while app.input.pending() > 0 {
                assert!(frames < 4 * MAX_SYNTHETIC_EVENTS, "the input never drained");
                self.frame(app);
                frames += 1;
            }
            for _ in 0..extra {
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
        w.settle(&mut app, 0);
        assert_eq!(selected(&mut app), ["C"], "once pendingInput is 0 the click has been drawn");
        let at = cell(&app, "E");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y, "mods": ["cmd"]})));
        w.settle(&mut app, 0);
        assert_eq!(selected(&mut app), ["C", "E"], "Cmd held through the click adds to the selection");
        // The modifiers are given back once the click is over.
        let at = cell(&app, "A");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y})));
        w.settle(&mut app, 0);
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
        w.settle(&mut app, 0);
        let menu = ok(call(&mut app, "ui.contextMenu.list", json!({})));
        assert_eq!(menu["open"], true);
        let contexts: Vec<&str> = menu["sections"].as_array().unwrap().iter().map(|s| s["context"].as_str().unwrap()).collect();
        assert_eq!(contexts.first(), Some(&context_menu::NODE), "{contexts:?}");
        let selection = serde_json::to_value(&app.state.tab().unwrap().selection).unwrap();
        assert_eq!(selection, json!({"points": [[0, 2]]}), "the right-click selected the node first");
        let r = call(&mut app, "ui.contextMenu.invoke", json!({"command": "no.such"}));
        assert_eq!(r["ok"], false);
        assert_eq!(ok(call(&mut app, "ui.contextMenu.list", json!({})))["open"], true, "a refused item leaves the menu open");
    }

    #[test]
    fn a_context_menu_item_is_checked_again_when_it_is_invoked() {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
        app.state.tab_mut().unwrap().selection = Selection::new();
        Window::new().settle(&mut app, 2);
        let open = |app: &mut FontcraftApp| ok(call(app, "ui.contextMenu.list", json!({"x": 300, "y": 300})));
        const FOCUS: &str = "path.focusOnSelectedPath";
        let menu = open(&mut app);
        let item = menu["sections"].as_array().unwrap().iter().flat_map(|s| s["items"].as_array().unwrap()).find(|i| i["command"] == FOCUS).cloned();
        assert_eq!(item.map(|i| i["enabled"].clone()), Some(json!(true)), "{menu}");
        // The selection the right-click made is gone: the item is disabled now.
        app.state.tab_mut().unwrap().selection = Selection::new();
        let r = call(&mut app, "ui.contextMenu.invoke", json!({"command": FOCUS}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(app.state.tab().unwrap().focus.is_none());
        assert_eq!(ok(call(&mut app, "ui.contextMenu.list", json!({})))["open"], true);
        // Another glyph in the tab: the menu belonged to A and closes.
        open(&mut app);
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["B"]})).unwrap();
        let r = call(&mut app, "ui.contextMenu.invoke", json!({"command": FOCUS}));
        assert!(r["error"].as_str().unwrap().contains("changed"), "{r}");
        assert_eq!(ok(call(&mut app, "ui.contextMenu.list", json!({}))), json!({"open": false}));
    }

    #[test]
    fn an_event_waits_for_the_frame_that_draws_it_and_that_frame_is_confined() {
        let mut app = FontcraftApp::default();
        app.automation_roots = file_access::AutomationRoots::new(Some(std::env::temp_dir().as_path()), None).unwrap();
        let w = Window::new();
        w.frame(&mut app);
        let at = cell(&app, "C");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y})));
        // A hidden window: the host runs passes without a ui and carries their input over.
        let mut raw = Window::raw();
        app.raw_input_hook(&mut raw);
        let handed = raw.events.clone();
        assert_eq!(handed, [egui::Event::PointerMoved(at)]);
        assert_eq!(ok(call(&mut app, "ui.inspect", json!({})))["pendingInput"], 3, "the move counts until a frame draws it");
        for _ in 0..3 {
            app.raw_input_hook(&mut raw);
        }
        assert_eq!(raw.events, handed, "a pass without a ui takes nothing more");
        assert_eq!(app.input.pending(), 3);
        // The window shows again and draws the carried input, confined as its request.
        app.input.confined.clear();
        w.draw(&mut app, raw);
        assert_eq!(app.input.confined.first(), Some(&true));
        assert_eq!(app.input.pending(), 2);
        w.settle(&mut app, 0);
        assert_eq!(selected(&mut app), ["C"]);
        assert_eq!(app.input.confined.iter().filter(|c| **c).count(), 3, "one confined frame per event");
        app.input.confined.clear();
        w.frame(&mut app);
        assert!(app.input.confined.iter().all(|c| !c), "a frame without channel input is the person's");
    }

    #[test]
    fn the_persons_own_press_drops_the_waiting_input() {
        let mut app = FontcraftApp::default();
        app.automation_roots = file_access::AutomationRoots::new(Some(std::env::temp_dir().as_path()), None).unwrap();
        let w = Window::new();
        w.frame(&mut app);
        let at = cell(&app, "C");
        ok(call(&mut app, "ui.click", json!({"x": at.x, "y": at.y, "count": 2})));
        w.frame(&mut app);
        let mut raw = Window::raw();
        raw.events.push(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        app.raw_input_hook(&mut raw);
        assert_eq!(app.input.pending(), 0);
        assert_eq!(raw.events.len(), 1, "only the person's key");
        app.input.confined.clear();
        w.draw(&mut app, raw);
        assert!(app.input.confined.iter().all(|c| !c));
    }

    #[test]
    fn synthetic_cmd_and_ctrl_match_the_keys_the_window_reports() {
        let cmd = egui_modifiers(Modifiers { cmd: true, ..Modifiers::NONE });
        let ctrl = egui_modifiers(Modifiers { ctrl: true, ..Modifiers::NONE });
        if cfg!(target_os = "macos") {
            assert_eq!(cmd, egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND);
            assert_eq!(ctrl, egui::Modifiers::CTRL);
        } else {
            // The Control key is both, whichever name the request uses.
            assert_eq!(cmd, egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
            assert_eq!(ctrl, cmd);
        }
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
        // The chord that holds the Measurement tool on the keyboard the frames report.
        let keys = match crate::tools_glue::keyboard(w.ctx.os() == egui::os::OperatingSystem::Mac) {
            fontcraft_tools::toolbox::Keyboard::Mac => ["cmd", "alt", "ctrl"],
            fontcraft_tools::toolbox::Keyboard::Other => ["ctrl", "alt", "shift"],
        };
        ok(call(&mut app, "ui.click", json!({"x": p.x, "y": p.y, "mods": keys})));
        // The move, the press and the release: the chord holds the Measurement tool.
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
            ("ui.resize", json!({"width": 800, "height": 300})),
            ("ui.wheel", json!({"x": 1, "y": 1, "dy": 1, "unit": 5})),
            ("ui.set", json!({"fillPreview": true, "outline": true})),
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

    #[test]
    fn a_resize_larger_than_the_window_can_take_is_clamped_and_says_so() {
        // Before the first frame: egui's own texture side.
        let mut app = FontcraftApp::default();
        let r = ok(call(&mut app, "ui.resize", json!({"width": 9000, "height": 1000})));
        assert_eq!(r, json!({"width": 2048.0, "height": 1000.0, "clamped": true}));
        // A 3000 × 2000 point monitor at 2× on a device with 8192-pixel textures.
        let w = Window::new();
        let mut raw = Window::raw();
        raw.max_texture_side = Some(8192);
        let info = egui::ViewportInfo { native_pixels_per_point: Some(2.0), monitor_size: Some(egui::vec2(3000.0, 2000.0)), ..Default::default() };
        raw.viewports.insert(egui::ViewportId::ROOT, info.clone());
        app.raw_input_hook(&mut raw);
        w.draw(&mut app, raw);
        let r = ok(call(&mut app, "ui.resize", json!({"width": 16384, "height": 16384})));
        assert_eq!(r, json!({"width": 3000.0, "height": 2000.0, "clamped": true}));
        assert_eq!(ok(call(&mut app, "ui.resize", json!({"width": 1000, "height": 700})))["clamped"], false);
        let mut raw = Window::raw();
        raw.max_texture_side = Some(8192);
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        app.raw_input_hook(&mut raw);
        let output = w.draw(&mut app, raw);
        let commands = output.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        let sizes: Vec<egui::Vec2> =
            commands.iter().filter_map(|c| if let egui::ViewportCommand::InnerSize(s) = c { Some(*s) } else { None }).collect();
        assert_eq!(sizes, [egui::vec2(3000.0, 2000.0), egui::vec2(1000.0, 700.0)]);
    }
}
