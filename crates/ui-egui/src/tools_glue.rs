//! The pointer tools in the shell: canvas input in, interactions, selection and view changes out.
//!
//! The tools crate turns events into [`Action`]s; this module feeds it and carries the actions
//! out. Events come from the canvas (egui pointer and key events) and from the control channel
//! (`ui.pointer`, `ui.key`), and both take the same path:
//!
//! 1. [`sync`] drops what no longer applies: a selection of another glyph, addresses an edit
//!    removed, a gesture whose tab or glyph changed.
//! 2. The event is put into the edited glyph's font units with the current view, so a pan earlier
//!    in the same frame counts.
//! 3. [`apply`] runs the actions: a drag is `Session::begin`, `preview`s, then `commit` (one undo
//!    step) or `cancel` (the font as it was, the selection as it was).
//!
//! Commands that return a `selection` for the edited glyph replace the tab's selection with it
//! (decision 0004, "What is the selection after an edit?").

use std::collections::BTreeSet;

use egui::{Event, PointerButton, Pos2, Rect};
use fontcraft_engine::params::TargetKind;
use fontcraft_engine::{CommandSpec, Session};
use fontcraft_geom::Point;
use fontcraft_model::{Font, LayerSide};
use fontcraft_tools::{
    Action, Context, HitResult, Key, KeyEvent, Modifiers, Output, PointerEvent, PointerKind, Scene, Selection, ToolSettings, Toolbox, View,
    ViewChange, hit_test,
};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::dialogs;
use crate::shell::context_menu::{self, ContextMenu};
use crate::state::{Tab, TabView, UiState};
use crate::theme::Tokens;

/// Most pointer events one `ui.pointer` request, or one frame, may carry.
pub const MAX_POINTER_EVENTS: usize = 10_000;

/// Where a pointer position is measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// Font units of the edited glyph.
    Font,
    /// Window points, as the mouse reports them.
    Screen,
}

/// A pointer event before it is put into font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawPointer {
    pub kind: PointerKind,
    pub at: Point,
    pub space: Space,
    pub mods: Modifiers,
}

#[derive(Clone, Copy, Debug)]
enum Queued {
    Pointer(RawPointer),
    Modifiers(Modifiers),
    /// A right-click (or a Mac Ctrl+click) at this window point: open the context menu.
    Context(Point),
}

/// The interaction a tool began, and what to put back if it is cancelled.
#[derive(Clone, Debug)]
struct Running {
    target: Target,
    before: Selection,
}

/// What the tools work on: a tab, its edited glyph, the active layer's id and the edited side.
type Target = (u64, String, String, LayerSide);

/// The showing tab's [`Target`].
fn target(app: &FontcraftApp) -> Option<Target> {
    let tab = app.state.tab()?;
    let layer = app.session.font().ok().and_then(|f| view::active_layer(f, tab)).map(|l| l.id.clone()).unwrap_or_default();
    Some((tab.id, tab.edited_glyph()?.to_string(), layer, tab.side()))
}

/// The tools' side of the shell. Not part of [`crate::state::UiState`]: tools are state
/// machines, not data.
#[derive(Debug, Default)]
pub struct ToolsState {
    pub toolbox: Toolbox,
    /// The canvas at the last frame, in window points.
    canvas: Option<Rect>,
    /// The primary button went down on the canvas and has not come up.
    pressed: bool,
    /// The modifiers the tools last saw.
    mods: Modifiers,
    running: Option<Running>,
    queued: Vec<Queued>,
    /// The tab, glyph and layer the tools last worked on.
    target: Option<Target>,
    /// The document revision the selection was last checked against.
    checked: Option<(u64, u64)>,
    /// A text field has the keyboard this frame.
    pub(crate) text_focus: bool,
    /// A text field had the keyboard when the last frame ended. A plain Escape takes the focus
    /// away before the frame starts, so without this the shortcuts would take the field's
    /// Escape.
    pub(crate) text_focus_at_end: bool,
    /// A text edit, not just any focusable widget, had the keyboard when the last frame ended:
    /// the menu shortcuts stay off for it alone.
    pub(crate) text_edit_at_end: bool,
    /// Text from `ui.text` for the focused text field, typed at the next frame.
    pending_text: String,
    /// A Mac Ctrl+press on the canvas, held back until it turns out to be a click (the context
    /// menu) or a drag (the tools).
    ctrl_press: Option<(Pos2, egui::Modifiers)>,
    /// The open canvas context menu.
    pub(crate) context_menu: Option<ContextMenu>,
    /// A press on the text in text mode, while the button is down.
    pub(crate) text_drag: crate::canvas::text::TextDrag,
}

impl ToolsState {
    /// The canvas centre in window points (before the first frame: the centre of a canvas of
    /// `size` at the window's corner).
    fn center(&self, size: [f64; 2]) -> Point {
        match self.canvas {
            Some(r) => Point::new(f64::from(r.center().x), f64::from(r.center().y)),
            None => Point::new(size[0] / 2.0, size[1] / 2.0),
        }
    }
}

/// The canvas centre in window points, as the last frame drew it.
pub(crate) fn canvas_center(app: &FontcraftApp) -> Point {
    app.tools.center(app.state.canvas_size)
}

/// The settings the tools work with: distances from the tokens, arrow steps from the font.
pub fn settings(tokens: &Tokens, font: &Font) -> ToolSettings {
    ToolSettings {
        hit_tolerance: f64::from(tokens.hit_tolerance),
        snap_distance: f64::from(tokens.snap_distance),
        drag_threshold: f64::from(tokens.drag_threshold),
        ..ToolSettings::default()
    }
    .with_font(font)
}

/// The scene of a tab's edited glyph: the edited side of its active layer, with the tab's path
/// focus.
pub fn scene<'a>(font: &'a Font, tab: &Tab) -> Option<Scene<'a>> {
    let glyph = tab.edited_glyph()?;
    let layer = view::active_layer(font, tab)?;
    let mut scene = Scene::from_font_side(font, glyph, Some(&layer.id), tab.side())?;
    scene.focus = tab.focus.clone();
    (scene.smart_handles, scene.smart_sizes) = crate::canvas::components::smart(font, &scene.layer, &tab.selection);
    Some(scene)
}

/// [`scene`] without the global guides whose filter leaves out the edited glyph, as the canvas
/// draws it: what pointer tools and selection commands may reach.
pub fn shown_scene<'a>(session: &Session, font: &'a Font, tab: &Tab) -> Option<Scene<'a>> {
    let mut scene = scene(font, tab)?;
    scene.hidden_global_guides = crate::canvas::guides::hidden_global_guides(session, font, tab);
    Some(scene)
}

/// The edited glyph's transform on the canvas now.
pub fn current_view(app: &FontcraftApp) -> Option<View> {
    app.session.font().ok()?;
    let tab = app.state.tab()?;
    Some(view::tools_view(&tab.view, view::edited_origin(&app.session, &app.layouts, tab), app.tools.center(app.state.canvas_size)))
}

/// Runs `f` with the active tool and the context of the showing tab.
fn with_tool<R>(app: &mut FontcraftApp, f: impl FnOnce(&mut Toolbox, &Context<'_>) -> R) -> Result<R, String> {
    sync(app);
    let center = app.tools.center(app.state.canvas_size);
    let FontcraftApp { session, state, tools, tokens, layouts, .. } = app;
    let hints = crate::canvas::hints_ps::marks(session, state, tokens);
    let font = session.font().map_err(|e| e.to_string())?;
    let tab = state.tab().ok_or("no Edit View tab is showing (open a glyph first)")?;
    let mut scene = shown_scene(session, font, tab).ok_or("the edited glyph has no layer in the active master")?;
    scene.ps_hints = hints;
    scene.background_shown = state.display.background;
    scene.guides_shown = state.display.guides;
    scene.suggest = crate::canvas::suggestions::suggest(session, font, tab, state.panels.edit_aids.suggest);
    let view = view::tools_view(&tab.view, view::edited_origin(session, layouts, tab), center);
    let settings = ToolSettings {
        bounding_box: state.display.bounding_box,
        reference: state.transform.reference(),
        tunni_lines: state.panels.edit_aids.tunni_lines,
        curve_tension: view::active_master(font, tab)
            .map_or(fontcraft_tools::rapid::DEFAULT_TENSION, |m| fontcraft_engine::cmd::tension::master_tension(font, &m.id)),
        tt_direction: state.tt_instructor.direction,
        ..settings(tokens, font)
    }
    .with_drag_aids(|id| session.settings().get(id).ok());
    let cx = Context { scene: &scene, view, selection: &tab.selection, settings };
    Ok(f(&mut tools.toolbox, &cx))
}

/// Brings the tools in line with the app before they see an event or a frame.
pub fn sync(app: &mut FontcraftApp) {
    crate::canvas::compatibility::carry_selection(app);
    let target = target(app);
    if let Some(tab) = app.state.tab_mut() {
        tab.sync_selection(target.as_ref().map(|t| t.2.as_str()));
    }
    if target != app.tools.target {
        // Another tab, glyph, layer or side: a gesture in progress ends without touching the new one.
        let _ = app.tools.toolbox.cancel();
        app.tools.pressed = false;
        app.tools.target = target.clone();
    }
    if let Some(r) = &app.tools.running {
        let same = target.as_ref() == Some(&r.target);
        if !same || app.session.interaction().is_none() {
            let _ = app.tools.toolbox.cancel();
            cancel(app);
        }
    }
    // After an edit, undo or redo, drop the addresses that no longer resolve.
    let revision = app.session.active().map(|d| (d.id, d.revision));
    if revision != app.tools.checked {
        app.tools.checked = revision;
        let valid = match (app.session.font(), app.state.tab()) {
            (Ok(font), Some(tab)) => scene(font, tab).map(|scene| {
                let mut s = tab.selection.clone();
                s.retain_valid(&scene);
                s
            }),
            _ => None,
        };
        if let (Some(valid), Some(tab)) = (valid, app.state.tab_mut()) {
            tab.selection = valid;
        }
        crate::canvas::hints_ps::retain(app);
    }
    // A tab shown in text mode has the Text tool, and the Text tool needs a tab in text mode.
    crate::shell::text::tool_follows_tab(app);
    app.preview_tool = matches!(app.tools.toolbox.active_id(), "hand" | "zoom" | "text");
}

/// One pointer event for the active tool. Returns the errors of the actions it asked for (they
/// also go to the status bar).
pub fn pointer(app: &mut FontcraftApp, ev: RawPointer) -> Vec<String> {
    app.tools.mods = ev.mods;
    if let Some(errors) = crate::canvas::text::pointer(app, ev) {
        return errors;
    }
    if let Some(errors) = crate::canvas::images::pointer(app, ev) {
        return errors;
    }
    let out = with_tool(app, |toolbox, cx| {
        let pos = match ev.space {
            Space::Font => ev.at,
            Space::Screen => cx.view.to_font(ev.at),
        };
        toolbox.pointer(cx, &PointerEvent { kind: ev.kind, pos, mods: ev.mods })
    });
    match out {
        Ok(out) => apply(app, out),
        Err(e) => vec![e],
    }
}

/// The modifier keys changed while the pointer stood still.
pub fn modifiers(app: &mut FontcraftApp, mods: Modifiers) -> Vec<String> {
    if app.tools.mods == mods {
        return Vec::new();
    }
    app.tools.mods = mods;
    if !app.tools.toolbox.busy() {
        return Vec::new();
    }
    match with_tool(app, |toolbox, cx| toolbox.modifiers(cx, mods)) {
        Ok(out) => apply(app, out),
        Err(e) => vec![e],
    }
}

/// A key for the tools: their own keys, the tool letters and the Edit View keys (decision 0004,
/// D1). `None` when the key is not theirs, so it goes on to the menu shortcuts.
/// In text mode the Text tool's keys come first, and a character key types itself.
pub fn key(app: &mut FontcraftApp, ev: KeyEvent) -> Option<Vec<String>> {
    key_typing(app, ev, true)
}

/// [`key`]; `types` false for a frame's keys, whose text events do the typing.
fn key_typing(app: &mut FontcraftApp, ev: KeyEvent, types: bool) -> Option<Vec<String>> {
    app.state.tab()?;
    if let Some(errors) = crate::shell::text::key(app, &ev, types) {
        return Some(errors);
    }
    let out = with_tool(app, |toolbox, cx| toolbox.key(cx, &ev)).ok();
    match out {
        Some(out) if out.handled => Some(apply(app, out)),
        _ => crate::shell::text::escape_to_text(app, &ev),
    }
}

/// Carries out a tool's actions in order. Returns the errors; each also goes to the status bar.
pub fn apply(app: &mut FontcraftApp, out: Output) -> Vec<String> {
    let out = crate::shell::edit_aids::unless_locked(app, out);
    let mut errors = Vec::new();
    for action in out.actions {
        let result = match action {
            Action::Begin { label } => {
                if let Err(e) = begin(app, &label) {
                    // The tool cannot drag without an interaction: end its gesture and skip the
                    // previews that would follow.
                    let _ = app.tools.toolbox.cancel();
                    app.state.status = e.clone();
                    errors.push(e);
                    break;
                }
                Ok(())
            }
            Action::Preview { invocation } => match app.session.preview(&invocation.id, &invocation.params) {
                Ok(value) => {
                    adopt(app, &value);
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            Action::Commit => commit(app),
            Action::Cancel => {
                cancel(app);
                Ok(())
            }
            Action::Run { invocation } if invocation.id == dialogs::primitive_measurements::CLICK => {
                dialogs::primitive_measurements::for_click(&invocation.params)
                    .ok_or_else(|| format!("{}: the click names no glyph", invocation.id))
                    .and_then(|(dialog, given)| dialogs::open(app, &dialog, &given).map(drop))
            }
            Action::Run { invocation } => app.run_from_shell(&invocation.id, &invocation.params).map(|value| adopt(app, &value)),
            Action::Select { selection } => {
                if let Some(tab) = app.state.tab_mut() {
                    tab.selection = selection;
                }
                Ok(())
            }
            Action::View { change } => view_change(app, change),
        };
        if let Err(e) = result {
            app.state.status = e.clone();
            errors.push(e);
        }
    }
    errors
}

fn begin(app: &mut FontcraftApp, label: &str) -> Result<(), String> {
    let (Some(target), Some(before)) = (target(app), app.state.tab().map(|t| t.selection.clone())) else {
        return Err("no glyph is being edited".into());
    };
    app.session.begin(label).map_err(|e| e.to_string())?;
    app.tools.running = Some(Running { target, before });
    Ok(())
}

fn commit(app: &mut FontcraftApp) -> Result<(), String> {
    app.tools.running = None;
    let label = app.session.interaction().unwrap_or_default().to_string();
    let changed = app.session.commit().map_err(|e| e.to_string())?;
    if changed {
        app.state.status = format!("{label}: done");
    }
    Ok(())
}

/// Ends the running interaction: the font and the selection as they were when it began.
fn cancel(app: &mut FontcraftApp) {
    let Some(r) = app.tools.running.take() else { return };
    if app.session.interaction().is_some() {
        let _ = app.session.cancel();
    }
    if let Some(tab) = app.state.tab_index(r.target.0).and_then(|i| app.state.tabs.get_mut(i))
        && tab.edited_glyph() == Some(r.target.1.as_str())
    {
        tab.selection = r.before;
    }
}

/// Takes the `selection` a command returned, when it is one of the edited glyph's edited side.
fn adopt(app: &mut FontcraftApp, value: &Value) {
    let Some(sel) = value.get("selection") else { return };
    let Some(tab) = app.state.tab_mut() else { return };
    let side = sel.get("side").and_then(Value::as_str).unwrap_or("foreground");
    if sel.get("glyph").and_then(Value::as_str) != tab.edited_glyph() || side != tab.side().as_str() {
        return;
    }
    if let Ok(s) = Selection::from_value(sel) {
        tab.selection = s;
    }
}

fn view_change(app: &mut FontcraftApp, change: ViewChange) -> Result<(), String> {
    app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab().ok_or("no Edit View tab is showing")?;
    let id = tab.id;
    let (command, params) = match change {
        ViewChange::Pan { dx, dy } => ("view.scrollTo", json!({"tab": id, "dx": dx, "dy": dy})),
        ViewChange::Zoom { factor, at } => {
            // Keep `at` where it is on screen: the centre moves towards it as the scale grows.
            let [ox, oy] = view::edited_origin(&app.session, &app.layouts, tab);
            let scale = TabView::clamped_scale(tab.view.scale);
            let next = TabView::clamped_scale(scale * factor);
            let k = scale / next;
            let [cx, cy] = tab.view.center;
            let x = at.x + ox - (at.x + ox - cx) * k - ox;
            let y = at.y + oy - (at.y + oy - cy) * k - oy;
            ("view.setZoom", json!({"tab": id, "size": next * 1000.0, "center": [x, y]}))
        }
        ViewChange::ZoomToRect { x, y, width, height } => ("view.zoomToRect", json!({"tab": id, "rect": [x, y, width, height]})),
    };
    app.run(command, &params).map(|_| ())
}

/// The modifiers of a pointer event. egui reports Control as `ctrl` on every system (and also
/// as `command` off the Mac), so `Ctrl`+drag suspends snapping everywhere.
pub fn pointer_mods(m: egui::Modifiers) -> Modifiers {
    Modifiers { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl }
}

/// The modifiers of a key event. Off the Mac, Control is the command key, so it counts as `cmd`
/// only: `Ctrl+Right` then steps by the largest increment, as `Cmd+Right` does on the Mac.
pub fn key_mods(m: egui::Modifiers) -> Modifiers {
    Modifiers { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl && (m.mac_cmd || !m.command) }
}

/// Collects the canvas's pointer input of this frame. [`dispatch_queued`] hands it to the tools
/// once the frame is drawn. The primary button goes to the tools, the middle button scrolls, and
/// the secondary button opens the context menu. On the Mac a Ctrl+click opens it too, while a
/// Ctrl+drag still reaches the tools.
pub fn collect(tools: &mut ToolsState, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
    tools.canvas = Some(rect);
    let over = response.hovered();
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    let (events, mods, double, dragging) = ui.input(|i| {
        let double = i.pointer.button_double_clicked(PointerButton::Primary).then(|| i.pointer.interact_pos()).flatten();
        (i.events.clone(), i.modifiers, double, i.pointer.is_decidedly_dragging())
    });
    let to_point = |p: Pos2| Point::new(f64::from(p.x), f64::from(p.y));
    let queue = |tools: &mut ToolsState, kind: PointerKind, at: Pos2, m: egui::Modifiers| {
        if tools.queued.len() < MAX_POINTER_EVENTS {
            tools.queued.push(Queued::Pointer(RawPointer { kind, at: to_point(at), space: Space::Screen, mods: pointer_mods(m) }));
        }
    };
    for event in events {
        match event {
            Event::PointerMoved(at) if tools.ctrl_press.is_some() => {
                if dragging && let Some((from, m)) = tools.ctrl_press.take() {
                    tools.pressed = true;
                    queue(tools, PointerKind::Down, from, m);
                    queue(tools, PointerKind::Drag, at, mods);
                }
            }
            Event::PointerMoved(at) if tools.pressed => queue(tools, PointerKind::Drag, at, mods),
            Event::PointerMoved(at) if over && rect.contains(at) => queue(tools, PointerKind::Move, at, mods),
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }
                if over && rect.contains(pos) && !tools.pressed && tools.ctrl_press.is_none() =>
            {
                if mac && modifiers.ctrl && !modifiers.mac_cmd && !tools.toolbox.busy() {
                    tools.ctrl_press = Some((pos, modifiers));
                } else {
                    tools.pressed = true;
                    queue(tools, PointerKind::Down, pos, modifiers);
                }
            }
            Event::PointerButton { button: PointerButton::Primary, pressed: false, .. } if tools.ctrl_press.is_some() => {
                if let Some((from, _)) = tools.ctrl_press.take()
                    && tools.queued.len() < MAX_POINTER_EVENTS
                {
                    tools.queued.push(Queued::Context(to_point(from)));
                }
            }
            Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, .. } if over && rect.contains(pos) => {
                if tools.queued.len() < MAX_POINTER_EVENTS {
                    tools.queued.push(Queued::Context(to_point(pos)));
                }
            }
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } if tools.pressed => {
                tools.pressed = false;
                queue(tools, PointerKind::Up, pos, modifiers);
            }
            _ => {}
        }
    }
    if let Some(at) = double
        && over
        && rect.contains(at)
    {
        queue(tools, PointerKind::DoubleClick, at, mods);
    }
    let m = pointer_mods(mods);
    if m != tools.mods && tools.toolbox.busy() {
        tools.queued.push(Queued::Modifiers(m));
    }
}

/// Hands the pointer input collected during the frame to the tools. `true` when there was any.
pub fn dispatch_queued(app: &mut FontcraftApp) -> bool {
    let queued = std::mem::take(&mut app.tools.queued);
    if app.dialog.is_some() {
        // The canvas takes no pointer input while a dialog is open.
        return false;
    }
    let any = !queued.is_empty();
    for q in queued {
        match q {
            Queued::Pointer(p) => {
                pointer(app, p);
            }
            Queued::Modifiers(m) => {
                modifiers(app, m);
            }
            Queued::Context(at) => {
                if let Err(e) = context_menu::open(app, at, Space::Screen) {
                    app.state.status = e;
                }
            }
        }
    }
    any
}

/// The keys of this frame that the tools take, before the menu shortcuts see them. Also types
/// the text `ui.text` left for a focused text field.
pub fn keys(app: &mut FontcraftApp, ctx: &egui::Context) {
    app.tools.text_focus = ctx.egui_wants_keyboard_input() || app.tools.text_focus_at_end;
    if !app.tools.pending_text.is_empty() {
        let text = std::mem::take(&mut app.tools.pending_text);
        ctx.input_mut(|i| i.events.push(Event::Text(text)));
    }
    if app.state.tab().is_none() || app.tools.text_focus || app.dialog.is_some() {
        return;
    }
    let events = ctx.input(|i| i.events.clone());
    let mut used = Vec::new();
    for (index, event) in events.iter().enumerate() {
        if let Event::Text(text) = event {
            // Text mode types what the keyboard gives; a Space held to pan types nothing.
            if crate::shell::text::in_text_mode(app) {
                if !crate::shell::text::panning(app)
                    && let Err(e) = crate::shell::text::type_chars(app, text)
                {
                    app.state.status = e;
                }
                used.push(index);
            }
            continue;
        }
        let Event::Key { key, pressed, modifiers, .. } = event else { continue };
        let Some(k) = Key::parse(key.name()) else { continue };
        if key_typing(app, KeyEvent { key: k, mods: key_mods(*modifiers), pressed: *pressed }, false).is_some() {
            used.push(index);
        }
    }
    if !used.is_empty() {
        ctx.input_mut(|i| {
            let mut index = 0;
            i.events.retain(|_| {
                let keep = !used.contains(&index);
                index += 1;
                keep
            });
        });
    }
}

/// `ui.text`: text for the focused text field, typed at the next frame. `false` when no text
/// field has the keyboard.
pub fn queue_text(app: &mut FontcraftApp, text: &str) -> bool {
    if !app.tools.text_focus {
        return false;
    }
    app.tools.pending_text.push_str(text);
    true
}

/// Whether `spec`'s params doc names `param` among its parameters (not in its result, after `→`).
pub fn takes_param(spec: &CommandSpec, param: &str) -> bool {
    spec.params.split('→').next().is_some_and(|given| given.contains(param))
}

/// The parameters for a command run from a menu or a shortcut: its targets filled from the Edit
/// View selection when the command takes targets and was given no parameters.
pub fn with_targets(app: &FontcraftApp, id: &str, params: Value) -> Value {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    if empty && let Some(hint) = crate::canvas::hints_ps::targets(app, id) {
        return hint;
    }
    let Some(spec) = fontcraft_engine::find_command(id) else { return params };
    if !empty || spec.targets.is_empty() {
        return params;
    }
    if app.state.tab().is_none() {
        return crate::shell::font_view::batch_targets(app, spec).map_or(params, Value::Object);
    }
    match selection_targets(&app.session, &app.state, spec) {
        Some(m) => Value::Object(m),
        None => params,
    }
}

/// The target parameters of `spec` that the showing tab's selection supplies: `glyph`, `layer`,
/// `side` while the background is edited (when the command's params doc names `side`), and each
/// list the command takes that the selection fills. `None` when no glyph is being edited.
///
/// A command whose targets include `glyphs` but not `layer` gets the edited glyph as `glyphs`.
/// A command that takes `segments` but not `points` gets the selected segments
/// ([`selected_segments`]); one that takes `shapes` but not `points` also gets the paths whose
/// on-curve nodes are all selected, which is what a selected path is. In text mode a command
/// that takes `glyphs` gets the glyphs of the text selection and, when it takes a layer, the
/// tab's master as `layer`.
pub fn selection_targets(session: &Session, state: &UiState, spec: &CommandSpec) -> Option<Map<String, Value>> {
    let font = session.font().ok()?;
    let tab = state.tab()?;
    // In text mode a command that takes glyphs acts on every glyph of the text selection, in the
    // tab's master; with nothing selected, on the current glyph as in edit mode.
    if tab.text_mode && spec.targets.contains(&TargetKind::Glyphs) {
        let mut seen = std::collections::HashSet::new();
        let glyphs: Vec<&str> = tab.text.selection().filter_map(|i| tab.item_glyph(i)).filter(|g| seen.insert(*g)).collect();
        if !glyphs.is_empty() {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(glyphs));
            if (spec.targets.contains(&TargetKind::Layer) || takes_param(spec, "layer?"))
                && let Some(master) = view::tab_master(font, tab)
            {
                m.insert("layer".into(), json!(master.id));
            }
            return Some(m);
        }
    }
    let scene = scene(font, tab)?;
    let selection = &tab.selection;
    let all = selection.targets(&scene);
    let takes_points = spec.targets.contains(&TargetKind::Points);
    let mut m = Map::new();
    let copy = |m: &mut Map<String, Value>, key: &str| {
        if let Some(v) = all.get(key) {
            m.insert(key.into(), v.clone());
        }
    };
    for kind in spec.targets {
        match kind {
            TargetKind::Layer => {
                copy(&mut m, "glyph");
                copy(&mut m, "layer");
                if takes_param(spec, "side?") {
                    copy(&mut m, "side");
                }
            }
            TargetKind::Points => copy(&mut m, "points"),
            TargetKind::Segments if !takes_points => {
                let segments = selected_segments(&scene, selection);
                if !segments.is_empty() {
                    m.insert("segments".into(), Value::Array(segments.iter().map(|&(s, n)| json!([s, n])).collect()));
                }
            }
            TargetKind::Shapes => {
                let mut shapes: BTreeSet<usize> = selection.shapes.clone();
                if !takes_points {
                    shapes.extend(selected_paths(&scene, selection));
                }
                if !shapes.is_empty() {
                    m.insert("shapes".into(), Value::Array(shapes.iter().map(|&s| json!(s)).collect()));
                }
            }
            TargetKind::Anchors => copy(&mut m, "anchors"),
            TargetKind::Guides => copy(&mut m, "guides"),
            TargetKind::Annotations => copy(&mut m, "annotations"),
            TargetKind::Glyphs | TargetKind::Segments => {}
        }
    }
    if spec.targets.contains(&TargetKind::Glyphs) && !m.contains_key("glyph") {
        m.insert("glyphs".into(), json!([scene.glyph]));
    }
    Some(m)
}

/// The paths of `scene` whose on-curve nodes are all in `selection`.
pub fn selected_paths(scene: &Scene, selection: &Selection) -> Vec<usize> {
    let touched: BTreeSet<usize> = selection.points.iter().map(|&(s, _)| s).collect();
    touched
        .into_iter()
        .filter(|&shape| {
            scene.path(shape).is_some_and(|path| {
                let mut on_curve = path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).peekable();
                on_curve.peek().is_some() && on_curve.all(|(i, _)| selection.points.contains(&(shape, i)))
            })
        })
        .collect()
}

/// The selected segments of `scene`, by the node that ends them: those whose two end nodes or
/// one of whose handles are in `selection`.
pub fn selected_segments(scene: &Scene, selection: &Selection) -> Vec<(usize, usize)> {
    let touched: BTreeSet<usize> = selection.points.iter().map(|&(s, _)| s).collect();
    let mut out = Vec::new();
    for shape in touched {
        let Some(path) = scene.path(shape) else { continue };
        let has = |node: usize| selection.points.contains(&(shape, node));
        for seg in fontcraft_tools::segment::segments(shape, path) {
            if (has(seg.start) && has(seg.end)) || seg.handles.iter().any(|&h| has(h)) {
                out.push((shape, seg.end));
            }
        }
    }
    out
}

/// Commands that take selection targets but act on the whole layer when given none (their
/// params docs say so), so they stay enabled without a selection.
const WHOLE_LAYER_WITHOUT_TARGETS: &[&str] = &["edit.copy", "edit.cut", "path.expandOutline", "path.expandOutlineForAllMasters"];

/// The target lists a selection can fill, their parameter keys, and what to select for them.
const OBJECT_TARGETS: &[(TargetKind, &str, &str)] = &[
    (TargetKind::Points, "points", "nodes"),
    (TargetKind::Segments, "segments", "segments"),
    (TargetKind::Shapes, "shapes", "paths or components"),
    (TargetKind::Anchors, "anchors", "anchors"),
    (TargetKind::Guides, "guides", "guides"),
    (TargetKind::Annotations, "annotations", "annotations"),
];

/// Whether the showing tab's selection gives `spec` what it acts on; the error says what to
/// select. A command that takes no target lists, or that takes `glyphs` (it then acts on whole
/// glyphs), needs no selection. Menus grey out an item for which this fails.
pub fn selection_supplies(session: &Session, state: &UiState, spec: &CommandSpec) -> Result<(), String> {
    let wanted: Vec<&(TargetKind, &str, &str)> = OBJECT_TARGETS.iter().filter(|(k, _, _)| spec.targets.contains(k)).collect();
    if wanted.is_empty() || spec.targets.contains(&TargetKind::Glyphs) || WHOLE_LAYER_WITHOUT_TARGETS.contains(&spec.id) {
        return Ok(());
    }
    let what = match wanted.as_slice() {
        [one] => one.2.to_string(),
        [a, b] => format!("{} or {}", a.2, b.2),
        _ => "something".to_string(),
    };
    let Some(m) = selection_targets(session, state, spec) else {
        return Err(format!("edit a glyph and select {what} first"));
    };
    if !wanted.iter().any(|(_, key, _)| m.contains_key(*key)) {
        return Err(format!("select {what} first"));
    }
    match (session.font().ok(), state.tab()) {
        (Some(font), Some(tab)) => scene(font, tab).map_or(Ok(()), |scene| crate::shell::item_state::ready(&scene, &tab.selection, spec.id)),
        _ => Ok(()),
    }
}

/// The edited glyph's point under the window or font point `at`, and the object there.
pub(crate) fn hit_at(app: &mut FontcraftApp, at: Point, space: Space) -> Result<(Point, Option<HitResult>), String> {
    with_tool(app, |_, cx| {
        let pos = match space {
            Space::Font => at,
            Space::Screen => cx.view.to_font(at),
        };
        (pos, hit_test(cx.scene, &cx.view, pos, cx.settings.hit_tolerance))
    })
}
