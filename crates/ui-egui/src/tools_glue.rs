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

use egui::{Event, PointerButton, Pos2, Rect};
use fontcraft_engine::params::TargetKind;
use fontcraft_geom::Point;
use fontcraft_model::Font;
use fontcraft_tools::{
    Action, Context, Key, KeyEvent, Modifiers, Output, PointerEvent, PointerKind, Scene, Selection, ToolSettings, Toolbox, View, ViewChange,
};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::{Tab, TabView};
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
}

/// The interaction a tool began, and what to put back if it is cancelled.
#[derive(Clone, Debug)]
struct Running {
    tab: u64,
    glyph: String,
    before: Selection,
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
    /// The tab and glyph the tools last worked on.
    target: Option<(u64, String)>,
    /// The document revision the selection was last checked against.
    checked: Option<(u64, u64)>,
    /// A text field had the keyboard at the last frame.
    pub(crate) text_focus: bool,
    /// Text from `ui.text` for the focused text field, typed at the next frame.
    pending_text: String,
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

/// The scene of a tab's edited glyph: its layer in the active master, with the tab's path focus.
pub fn scene<'a>(font: &'a Font, tab: &Tab) -> Option<Scene<'a>> {
    let glyph = tab.edited_glyph()?;
    let master = view::active_master(font)?;
    let layer = font.glyph(glyph)?.layer(&master.id)?;
    let mut scene = Scene::from_font(font, glyph, Some(&layer.id))?;
    scene.focus = tab.focus.clone();
    Some(scene)
}

/// The edited glyph's transform on the canvas now.
pub fn current_view(app: &FontcraftApp) -> Option<View> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    Some(view::tools_view(&tab.view, view::edited_offset(font, tab), app.tools.center(app.state.canvas_size)))
}

/// Runs `f` with the active tool and the context of the showing tab.
fn with_tool<R>(app: &mut FontcraftApp, f: impl FnOnce(&mut Toolbox, &Context<'_>) -> R) -> Result<R, String> {
    sync(app);
    let center = app.tools.center(app.state.canvas_size);
    let FontcraftApp { session, state, tools, tokens, .. } = app;
    let font = session.font().map_err(|e| e.to_string())?;
    let tab = state.tab().ok_or("no Edit View tab is showing (open a glyph first)")?;
    let scene = scene(font, tab).ok_or("the edited glyph has no layer in the active master")?;
    let view = view::tools_view(&tab.view, view::edited_offset(font, tab), center);
    let cx = Context { scene: &scene, view, selection: &tab.selection, settings: settings(tokens, font) };
    Ok(f(&mut tools.toolbox, &cx))
}

/// Brings the tools in line with the app before they see an event or a frame.
pub fn sync(app: &mut FontcraftApp) {
    if let Some(tab) = app.state.tab_mut() {
        tab.sync_selection();
    }
    let target = app.state.tab().and_then(|t| Some((t.id, t.edited_glyph()?.to_string())));
    if target != app.tools.target {
        // Another tab or glyph: a gesture in progress ends without touching the new one.
        let _ = app.tools.toolbox.cancel();
        app.tools.pressed = false;
        app.tools.target = target.clone();
    }
    if let Some(r) = &app.tools.running {
        let same = target.as_ref().is_some_and(|(id, glyph)| *id == r.tab && *glyph == r.glyph);
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
    }
    app.preview_tool = matches!(app.tools.toolbox.active_id(), "hand" | "zoom" | "text");
}

/// One pointer event for the active tool. Returns the errors of the actions it asked for (they
/// also go to the status bar).
pub fn pointer(app: &mut FontcraftApp, ev: RawPointer) -> Vec<String> {
    app.tools.mods = ev.mods;
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
pub fn key(app: &mut FontcraftApp, ev: KeyEvent) -> Option<Vec<String>> {
    app.state.tab()?;
    let out = with_tool(app, |toolbox, cx| toolbox.key(cx, &ev)).ok()?;
    out.handled.then(|| apply(app, out))
}

/// Carries out a tool's actions in order. Returns the errors; each also goes to the status bar.
pub fn apply(app: &mut FontcraftApp, out: Output) -> Vec<String> {
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
            Action::Run { invocation } => app.run(&invocation.id, &invocation.params).map(|value| adopt(app, &value)),
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
    let Some((tab, Some(glyph), before)) = app.state.tab().map(|t| (t.id, t.edited_glyph().map(str::to_string), t.selection.clone())) else {
        return Err("no glyph is being edited".into());
    };
    app.session.begin(label).map_err(|e| e.to_string())?;
    app.tools.running = Some(Running { tab, glyph, before });
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
    if let Some(tab) = app.state.tab_index(r.tab).and_then(|i| app.state.tabs.get_mut(i))
        && tab.edited_glyph() == Some(r.glyph.as_str())
    {
        tab.selection = r.before;
    }
}

/// Takes the `selection` a command returned, when it is one of the edited glyph's foreground.
fn adopt(app: &mut FontcraftApp, value: &Value) {
    let Some(sel) = value.get("selection") else { return };
    let Some(tab) = app.state.tab_mut() else { return };
    if sel.get("glyph").and_then(Value::as_str) != tab.edited_glyph() || sel.get("side").is_some_and(|s| s != "foreground") {
        return;
    }
    if let Ok(s) = Selection::from_value(sel) {
        tab.selection = s;
    }
}

fn view_change(app: &mut FontcraftApp, change: ViewChange) -> Result<(), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab().ok_or("no Edit View tab is showing")?;
    let id = tab.id;
    let (command, params) = match change {
        ViewChange::Pan { dx, dy } => ("view.scrollTo", json!({"tab": id, "dx": dx, "dy": dy})),
        ViewChange::Zoom { factor, at } => {
            // Keep `at` where it is on screen: the centre moves towards it as the scale grows.
            let offset = view::edited_offset(font, tab);
            let scale = TabView::clamped_scale(tab.view.scale);
            let next = TabView::clamped_scale(scale * factor);
            let k = scale / next;
            let [cx, cy] = tab.view.center;
            let x = at.x + offset - (at.x + offset - cx) * k - offset;
            let y = at.y - (at.y - cy) * k;
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
/// once the frame is drawn. Only the primary button: the middle button scrolls, the secondary
/// one is for context menus.
pub fn collect(tools: &mut ToolsState, ui: &egui::Ui, response: &egui::Response, rect: Rect) {
    tools.canvas = Some(rect);
    let over = response.contains_pointer();
    let (events, mods, double) = ui.input(|i| {
        let double = i.pointer.button_double_clicked(PointerButton::Primary).then(|| i.pointer.interact_pos()).flatten();
        (i.events.clone(), i.modifiers, double)
    });
    let to_point = |p: Pos2| Point::new(f64::from(p.x), f64::from(p.y));
    let queue = |tools: &mut ToolsState, kind: PointerKind, at: Pos2, m: egui::Modifiers| {
        if tools.queued.len() < MAX_POINTER_EVENTS {
            tools.queued.push(Queued::Pointer(RawPointer { kind, at: to_point(at), space: Space::Screen, mods: pointer_mods(m) }));
        }
    };
    for event in events {
        match event {
            Event::PointerMoved(at) if tools.pressed => queue(tools, PointerKind::Drag, at, mods),
            Event::PointerMoved(at) if over && rect.contains(at) => queue(tools, PointerKind::Move, at, mods),
            Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }
                if over && rect.contains(pos) && !tools.pressed =>
            {
                tools.pressed = true;
                queue(tools, PointerKind::Down, pos, modifiers);
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
        }
    }
    any
}

/// The keys of this frame that the tools take, before the menu shortcuts see them. Also types
/// the text `ui.text` left for a focused text field.
pub fn keys(app: &mut FontcraftApp, ctx: &egui::Context) {
    app.tools.text_focus = ctx.egui_wants_keyboard_input();
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
        let Event::Key { key, pressed, modifiers, .. } = event else { continue };
        let Some(k) = Key::parse(key.name()) else { continue };
        if self::key(app, KeyEvent { key: k, mods: key_mods(*modifiers), pressed: *pressed }).is_some() {
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

/// The parameters for a command run from a menu or a shortcut: its targets filled from the Edit
/// View selection when the command takes targets and was given no parameters.
pub fn with_targets(app: &FontcraftApp, id: &str, params: Value) -> Value {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    let Some(spec) = fontcraft_engine::find_command(id) else { return params };
    if !empty || spec.targets.is_empty() {
        return params;
    }
    let (Ok(font), Some(tab)) = (app.session.font(), app.state.tab()) else { return params };
    let Some(scene) = scene(font, tab) else { return params };
    let all = tab.selection.targets(&scene);
    let mut m = Map::new();
    let mut copy = |key: &str| {
        if let Some(v) = all.get(key) {
            m.insert(key.into(), v.clone());
        }
    };
    for kind in spec.targets {
        match kind {
            TargetKind::Layer => {
                copy("glyph");
                copy("layer");
            }
            TargetKind::Points => copy("points"),
            TargetKind::Shapes => copy("shapes"),
            TargetKind::Anchors => copy("anchors"),
            TargetKind::Guides => copy("guides"),
            TargetKind::Annotations => copy("annotations"),
            TargetKind::Glyphs | TargetKind::Segments => {}
        }
    }
    if spec.targets.contains(&TargetKind::Glyphs) && !m.contains_key("glyph") {
        m.insert("glyphs".into(), json!([scene.glyph]));
    }
    Value::Object(m)
}
