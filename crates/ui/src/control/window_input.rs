//! Window input and window state: `ui.click`, `ui.drag`, `ui.move`, `ui.wheel`, `ui.resize`, `ui.focus` and
//! `ui.set`.
//!
//! - Clicks, drags, moves and wheel turns are queued on the window host at window points, one step per pass, so
//!   they reach every widget a mouse reaches. A point is in the main window's points; the desktop around the main
//!   window has the same origin and unit, so a point over a panel floating in an OS window of its own reaches that
//!   window ([`land`]). A press gives its window the keyboard, as a click on the desktop does. The modifiers a request names are held for the steps it queues. A
//!   request is checked whole before anything of it is queued. In the desktop app the reply comes once the frames
//!   have carried the input.
//! - `ui.click {id}` presses a control of the open dialog: it runs `ui.dialog.click`, which the dialogs package
//!   adds.
//! - `ui.set` turns switches on or off with their commands, so the result is the menu's: the panels by their id,
//!   and the switches packages add to [`SWITCHES`].

use craft::behavior::PointerButton;
use craft::keymap::Mods;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::windows::{Released, WindowAction, WindowHost};
use egui::{Pos2, ViewportId, pos2, vec2};
use fontcraft_tools::Modifiers;
use serde_json::{Map, Value, json};

use super::Method;
use super::inspect::shown;
use crate::app::FontCraftApp;
use crate::shell::dock::PanelId;

pub const METHODS: &[(&str, Method)] =
    &[("ui.click", click), ("ui.drag", drag), ("ui.move", move_to), ("ui.wheel", wheel), ("ui.resize", resize), ("ui.focus", focus), ("ui.set", set)];

/// Most moves between the press and the release of one `ui.drag`.
pub const MAX_DRAG_STEPS: u64 = 256;
/// Most clicks one `ui.click` or `ui.drag` makes.
pub const MAX_CLICK_COUNT: u64 = 16;
/// Largest wheel turn, in notches or points.
pub const MAX_WHEEL: f64 = 100.0;
/// Largest window coordinate, either sign.
pub const MAX_COORDINATE: f64 = 100_000.0;
/// The smallest and largest window size `ui.resize` asks for, in points.
pub const MIN_WINDOW_WIDTH: f64 = 640.0;
pub const MIN_WINDOW_HEIGHT: f64 = 400.0;
pub const MAX_WINDOW: f64 = 16_384.0;
/// Most keys one `ui.set` changes.
const MAX_SET_KEYS: usize = 64;
const DEFAULT_DRAG_STEPS: u64 = 8;
/// Points per wheel notch: egui's line scroll speed, which a mouse wheel's line steps are multiplied by.
const LINE_POINTS: f32 = 40.0;

/// The window host for window input, or the error for a host without frames.
fn window(app: &FontCraftApp, method: &str) -> Result<WindowHost, String> {
    app.control.window.clone().ok_or_else(|| format!("{method}: no window: window input needs the desktop app or a test frame"))
}

/// The reply of a request that queued `steps` window input steps in `viewport`.
fn queued(app: &mut FontCraftApp, steps: usize, viewport: ViewportId) -> Value {
    app.control.queued += steps;
    json!({ "queued": steps, "window": window_name(viewport) })
}

/// How replies and `ui.inspect` name a window: `main`, else the viewport id in hex.
pub(super) fn window_name(viewport: ViewportId) -> String {
    if viewport == ViewportId::ROOT { "main".into() } else { format!("{:016x}", viewport.0.value()) }
}

/// The window the main-window point `at` lands in, and the point in that window's own points. A point inside a
/// panel floating in an OS window of its own lands there: floating panels are palettes above the main window. Where
/// floating windows overlap, the order is [`WindowHost::viewports_at`]'s. Every other point, and every point while
/// the main window has not reported where it is, lands in the main window unchanged.
fn land(host: &WindowHost, at: Pos2) -> (ViewportId, Pos2) {
    let Some(main) = host.facts(ViewportId::ROOT) else { return (ViewportId::ROOT, at) };
    let desktop = main.inner.min + at.to_vec2();
    host.viewports_at(desktop)
        .into_iter()
        .filter(|viewport| *viewport != ViewportId::ROOT)
        .find_map(|viewport| {
            let facts = host.facts(viewport).filter(|facts| facts.inner.contains(desktop))?;
            Some((viewport, desktop - facts.inner.min.to_vec2()))
        })
        .unwrap_or((ViewportId::ROOT, at))
}

/// A press in `viewport` gives its window the keyboard, as a click on the desktop does, so `ui.text` and `ui.key`
/// reach the field the click focused. Nothing is asked of the platform when the window has the keyboard already, or
/// for the main window while no window of the app has it ([`WindowHost::key_window`] is then the main window).
fn focus_on_press(host: &WindowHost, viewport: ViewportId) {
    if viewport != host.key_window() {
        host.act(viewport, WindowAction::Focus);
    }
}

/// `ui.inspect` key `windows`: the app's OS windows, the main one first, each `{window, rect: [x0, y0, x1, y1],
/// focused}` with its inner rect in main-window points, the points `ui.click` and the other window input take.
/// Absent without frames, and while the main window has not reported where it is.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let (Some(host), Some(ctx)) = (app.control.window.as_ref(), app.window.as_ref()) else { return };
    let Some(main) = host.facts(ViewportId::ROOT) else { return };
    let origin = main.inner.min.to_vec2();
    let mut viewports: Vec<ViewportId> = ctx.input(|input| input.raw.viewports.keys().copied().collect());
    viewports.sort_by_key(|viewport| *viewport != ViewportId::ROOT);
    let windows: Vec<Value> = viewports
        .into_iter()
        .filter_map(|viewport| {
            let facts = host.facts(viewport)?;
            let rect = facts.inner.translate(-origin);
            Some(json!({
                "window": window_name(viewport),
                "rect": [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                "focused": facts.focused,
            }))
        })
        .collect();
    state.insert("windows".into(), Value::Array(windows));
}

/// A window coordinate `key` of `params`.
fn coordinate(method: &str, params: &Value, key: &str) -> Result<f32, String> {
    let v = params.get(key).and_then(Value::as_f64).ok_or_else(|| format!("{method}: `{key}` (a window coordinate in points) is required"))?;
    if !v.is_finite() || v.abs() > MAX_COORDINATE {
        return Err(format!("{method}: `{key}` must be between -{MAX_COORDINATE} and {MAX_COORDINATE}"));
    }
    Ok(v as f32)
}

/// The point `(x, y)` of `params`.
fn point(method: &str, params: &Value, x: &str, y: &str) -> Result<Pos2, String> {
    Ok(pos2(coordinate(method, params, x)?, coordinate(method, params, y)?))
}

/// A count `key` of `params`: a non-negative integer up to `max`; zero acts as one.
fn count(method: &str, params: &Value, key: &str, default: u64, max: u64) -> Result<u32, String> {
    let n = match params.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v.as_u64().ok_or_else(|| format!("{method}: `{key}` must be a non-negative integer"))?,
    };
    if n > max {
        return Err(format!("{method}: `{key}` must be at most {max}"));
    }
    u32::try_from(n.max(1)).map_err(|_| format!("{method}: `{key}` is out of range"))
}

fn button(method: &str, params: &Value) -> Result<PointerButton, String> {
    match params.get("button") {
        None | Some(Value::Null) => Ok(PointerButton::Primary),
        Some(Value::String(b)) if matches!(b.as_str(), "left" | "primary") => Ok(PointerButton::Primary),
        Some(Value::String(b)) if matches!(b.as_str(), "right" | "secondary") => Ok(PointerButton::Secondary),
        Some(Value::String(b)) if b == "middle" => Ok(PointerButton::Middle),
        Some(_) => Err(format!("{method}: `button` must be left, right or middle")),
    }
}

/// The modifiers of a request: its `mods` (a list of names, `["shift", "alt"]`, or `{shift, alt, cmd, ctrl}`) plus
/// the flags `shift`, `alt`, `cmd` and `ctrl` set to true.
pub(super) fn modifiers(method: &str, params: &Value) -> Result<Modifiers, String> {
    let bad = || format!("{method}: `mods` must be a list of modifier names (shift, alt, cmd, ctrl)");
    let mut mods = match params.get("mods") {
        None | Some(Value::Null) => Modifiers::NONE,
        Some(Value::Array(list)) => {
            if list.len() > 8 {
                return Err(bad());
            }
            let names: Vec<&str> = list.iter().map(|v| v.as_str().ok_or_else(bad)).collect::<Result<_, _>>()?;
            Modifiers::from_names(names).map_err(|e| format!("{method}: {e}"))?
        }
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).map_err(|_| bad())?,
        Some(_) => return Err(bad()),
    };
    for (key, slot) in [("shift", &mut mods.shift), ("alt", &mut mods.alt), ("cmd", &mut mods.cmd), ("ctrl", &mut mods.ctrl)] {
        match params.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => *slot |= *b,
            Some(_) => return Err(format!("{method}: `{key}` must be true or false")),
        }
    }
    Ok(mods)
}

/// The window host's modifiers for a request's: `cmd` is the command key (Ctrl off the Mac), `ctrl` the Control
/// key, which off the Mac is the same key.
fn host_mods(m: Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    mods.cmd = m.cmd;
    mods.ctrl = m.ctrl;
    mods.alt = m.alt;
    mods.shift = m.shift;
    mods
}

/// `ui.click {x, y, button?, count?, mods?}` clicks at a window point ([`land`]); `ui.click {id}` presses a control of the
/// open dialog (`ui.dialog.click`).
fn click(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.click";
    let has_point = params.get("x").is_some() || params.get("y").is_some();
    match params.get("id").filter(|v| !v.is_null()) {
        Some(Value::String(_)) if has_point => Err(format!("{M}: give `id` or `x` and `y`, not both")),
        Some(Value::String(_)) => super::dialogs::click(app, params).map_err(|e| format!("{M}: {e}")),
        Some(_) => Err(format!("{M}: `id` must be a control id")),
        None if !has_point => Err(format!("{M}: `id` (a dialog control) or `x` and `y` (a window point) is required")),
        None => {
            let clicks = count(M, params, "count", 1, MAX_CLICK_COUNT)?;
            let button = button(M, params)?;
            let mods = host_mods(modifiers(M, params)?);
            let at = point(M, params, "x", "y")?;
            let host = window(app, M)?;
            let (viewport, at) = land(&host, at);
            focus_on_press(&host, viewport);
            host.inject_click(viewport, at, button, clicks, mods, false);
            Ok(queued(app, 1 + 2 * clicks as usize, viewport))
        }
    }
}

/// `ui.drag {x, y, toX, toY, steps?, count?, button?, mods?}`: a press at (x, y), the moves of the drag in
/// `steps`, the release at (toX, toY), then the further clicks of `count` there. The whole drag goes to the window
/// the press lands in, as a held mouse button keeps its window; the end point is in that window's points as the
/// pointer's position there.
fn drag(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.drag";
    let steps = count(M, params, "steps", DEFAULT_DRAG_STEPS, MAX_DRAG_STEPS)?;
    let clicks = count(M, params, "count", 1, MAX_CLICK_COUNT)?;
    let button = button(M, params)?;
    let mods = host_mods(modifiers(M, params)?);
    let from = point(M, params, "x", "y")?;
    let to = point(M, params, "toX", "toY")?;
    let host = window(app, M)?;
    let (viewport, local) = land(&host, from);
    let (from, to) = (local, to + (local - from));
    focus_on_press(&host, viewport);
    host.inject_click(viewport, from, button, 1, mods, true);
    for i in 1..=steps {
        host.inject_move(viewport, from + (to - from) * (i as f32 / steps as f32), mods, 0.0);
    }
    host.inject_release(viewport, Some(to), Released::Button(button));
    if clicks > 1 {
        host.inject_click(viewport, to, button, clicks - 1, mods, false);
    }
    Ok(queued(app, 2 + steps as usize + 2 * (clicks as usize - 1), viewport))
}

/// `ui.move {x, y}`: the pointer moves to a window point (hover).
fn move_to(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.move";
    let at = point(M, params, "x", "y")?;
    let mods = host_mods(modifiers(M, params)?);
    let host = window(app, M)?;
    let (viewport, at) = land(&host, at);
    host.inject_move(viewport, at, mods, 0.0);
    Ok(queued(app, 1, viewport))
}

/// `ui.wheel {x, y, dx?, dy?, unit?: line | point, mods?}`: a wheel turn over a window point. `dy` positive turns
/// up (the content moves down), `dx` positive moves the content right.
fn wheel(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.wheel";
    let at = point(M, params, "x", "y")?;
    let amount = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && n.abs() <= MAX_WHEEL => Ok(n as f32),
            _ => Err(format!("{M}: `{key}` must be a number between -{MAX_WHEEL} and {MAX_WHEEL}")),
        },
    };
    let delta = vec2(amount("dx")?, amount("dy")?);
    let scale = match params.get("unit") {
        None | Some(Value::Null) => LINE_POINTS,
        Some(Value::String(u)) if u == "line" => LINE_POINTS,
        Some(Value::String(u)) if u == "point" => 1.0,
        Some(_) => return Err(format!("{M}: `unit` must be line or point")),
    };
    let mods = host_mods(modifiers(M, params)?);
    let host = window(app, M)?;
    let (viewport, at) = land(&host, at);
    host.inject_wheel(viewport, at, delta * scale, mods);
    Ok(queued(app, 2, viewport))
}

/// `ui.resize {width, height}`: the window's inner size in points, at the next frame. A size larger than the
/// graphics device can draw is clamped; the reply has the size asked of the window.
fn resize(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.resize";
    let size = |key: &str, min: f64| match params.get(key).and_then(Value::as_f64) {
        Some(v) if (min..=MAX_WINDOW).contains(&v) => Ok(v as f32),
        _ => Err(format!("{M}: `{key}` must be a number of points from {min} to {MAX_WINDOW}")),
    };
    let wanted = vec2(size("width", MIN_WINDOW_WIDTH)?, size("height", MIN_WINDOW_HEIGHT)?);
    let host = window(app, M)?;
    let largest = host
        .screenshot_facts(ViewportId::ROOT)
        .map(|facts| {
            let ppp = if facts.pixels_per_point.is_finite() && facts.pixels_per_point > 0.0 { facts.pixels_per_point } else { 1.0 };
            facts.max_texture_side as f32 / ppp
        })
        .filter(|side| side.is_finite() && *side > 0.0)
        .unwrap_or(MAX_WINDOW as f32);
    let size = wanted.min(vec2(largest, largest));
    if !host.act(ViewportId::ROOT, WindowAction::SetInnerSize(size)) {
        return Err(format!("{M}: the window cannot be resized"));
    }
    Ok(json!({"width": size.x, "height": size.y, "clamped": size != wanted}))
}

/// `ui.focus {}`: brings the window to the front with the keyboard.
fn focus(app: &mut FontCraftApp, _: &Value) -> Result<Value, String> {
    let host = window(app, "ui.focus")?;
    if !host.act(ViewportId::ROOT, WindowAction::Focus) {
        return Err("ui.focus: the window cannot take the focus".into());
    }
    Ok(json!({"focus": true}))
}

/// A switch a package adds to `ui.set`: the command that turns it over (run without parameters) and its state.
/// Its keys are the command id and the short key [`set_key`] makes of it.
pub struct Switch {
    pub command: &'static str,
    pub on: fn(&FontCraftApp) -> bool,
}

/// Every package's switches, one line per package.
pub const SWITCHES: &[&[Switch]] = &[];

/// The `ui.set` key of a switch command: the part after the area, without a leading `show`, `set` or `toggle`
/// (`view.showBoundingBox` → `boundingBox`).
pub fn set_key(id: &str) -> String {
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

/// What a `ui.set` key turns.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Panel(PanelId),
    Switch(&'static Switch),
}

impl Target {
    fn of(key: &str) -> Option<Target> {
        if let Some(panel) = PanelId::from_id(key) {
            return Some(Target::Panel(panel));
        }
        SWITCHES.iter().flat_map(|s| s.iter()).find(|s| s.command == key || set_key(s.command) == key).map(Target::Switch)
    }

    fn on(self, app: &FontCraftApp) -> bool {
        match self {
            Target::Panel(panel) => shown(app, panel),
            Target::Switch(switch) => (switch.on)(app),
        }
    }

    fn toggle(self) -> CommandRef {
        match self {
            Target::Panel(panel) => CommandRef::with_params("window.togglePanel", json!({ "panel": panel.id() })),
            Target::Switch(switch) => CommandRef::new(switch.command),
        }
    }
}

impl PartialEq for Switch {
    fn eq(&self, other: &Switch) -> bool {
        self.command == other.command
    }
}

/// `ui.set {key: bool, …}`: turns panels (by their id) and switches on or off with their commands. Every key is
/// checked before the first one moves; when a command fails, the ones already moved are put back. Returns each
/// key's value afterwards.
fn set(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.set";
    let keys = params
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| format!("{M}: params must be an object of switches, such as {{\"layers\": true}}"))?;
    if keys.len() > MAX_SET_KEYS {
        return Err(format!("{M}: at most {MAX_SET_KEYS} keys"));
    }
    let mut plan: Vec<(&str, Target, bool)> = Vec::with_capacity(keys.len());
    for (key, value) in keys {
        let target = Target::of(key).ok_or_else(|| {
            let mut known: Vec<String> = PanelId::ALL.iter().map(|p| p.id().to_string()).collect();
            known.extend(SWITCHES.iter().flat_map(|s| s.iter()).map(|s| set_key(s.command)));
            format!("{M}: unknown key `{key}`; the keys are {}", known.join(", "))
        })?;
        let want = value.as_bool().ok_or_else(|| format!("{M}: `{key}` must be true or false"))?;
        if let Some((other, ..)) = plan.iter().find(|(_, t, w)| *t == target && *w != want) {
            return Err(format!("{M}: `{key}` and `{other}` ask for opposite states of one switch"));
        }
        plan.push((key.as_str(), target, want));
    }
    let mut moved: Vec<(Target, bool)> = Vec::new();
    let mut failure = None;
    for (key, target, want) in &plan {
        let was = target.on(app);
        if was == *want {
            continue;
        }
        if let Err(e) = app.run_now(FontCraftApp::route(target.toggle()), false) {
            failure = Some(format!("{M}: `{key}`: {e}"));
            break;
        }
        moved.push((*target, was));
        if target.on(app) != *want {
            failure = Some(format!("{M}: `{key}` cannot be turned {} now", if *want { "on" } else { "off" }));
            break;
        }
    }
    if let Some(e) = failure {
        for (target, was) in moved.into_iter().rev() {
            if target.on(app) != was {
                let _ = app.run_now(FontCraftApp::route(target.toggle()), false);
            }
        }
        return Err(e);
    }
    let mut result = Map::new();
    for (key, target, _) in &plan {
        result.insert((*key).to_string(), json!(target.on(app)));
    }
    Ok(Value::Object(result))
}
