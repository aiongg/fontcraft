//! The app side of the tools: input in, the toolbox's answer carried out in order.
//!
//! 1. [`sync`] drops what no longer applies: a selection of another glyph, layer or side, addresses an edit removed,
//!    a gesture whose interaction ended elsewhere (an undo).
//! 2. The toolbox runs with the edited glyph's scene, the selection and the canvas's view as it is now.
//! 3. [`apply`] carries out the actions: a drag is `Preview::Begin`, `Preview::Update`s, then `Preview::Commit` (one
//!    undo step) or `Preview::Cancel` (the font and the selection as they were). Commands go out as `Msg::Run`, or as
//!    the shell or tool message their id names.

use craft::behavior::kurbo::Point as KPoint;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::Font;
use fontcraft_geom::Point;
use fontcraft_tools::{Action, Context, Output, Selection, ToolSettings, Toolbox, View, ViewChange};
use serde_json::Value;

use super::tokens::ToolTokens;
use super::{ToolInput, scene};
use crate::app::{FontCraftApp, Msg, Preview};
use crate::canvas::camera::Camera;
use crate::shell::view::ZOOM_RANGE;

/// Most pointer events one control request may carry.
pub const MAX_POINTER_EVENTS: usize = 10_000;

/// The camera of the canvas as it is now: the canvas view the last pass left, the edited glyph's ascender at the top.
pub fn camera(app: &FontCraftApp) -> Camera {
    let top = app.models.canvas.glyph.as_ref().map_or(0.0, |g| g.ascender);
    Camera::new(app.view.canvas, top)
}

/// The tools' view of `camera`: font units to screen points, for distances measured on screen.
pub fn tools_view(camera: &Camera) -> View {
    let origin = camera.to_screen(KPoint::ZERO);
    View::new(camera.scale(), Point::new(f64::from(origin.x), f64::from(origin.y)))
}

/// The settings the tools work with: distances from the tokens, arrow steps and curve tension from the font, drag
/// aids from the user settings.
fn settings(session: &Session, font: &Font, app: &FontCraftApp) -> ToolSettings {
    let t = ToolTokens::standard();
    let curve_tension =
        font.masters.first().map_or(fontcraft_tools::rapid::DEFAULT_TENSION, |m| fontcraft_engine::cmd::tension::master_tension(font, &m.id));
    ToolSettings {
        hit_tolerance: f64::from(t.hit_tolerance),
        snap_distance: f64::from(t.snap_distance),
        drag_threshold: f64::from(t.drag_threshold),
        bounding_box: app.view.display.bounding_box,
        tunni_lines: app.overlays.switches.tunni_lines,
        curve_tension,
        tt_direction: app.view.hinting.instructor.direction,
        ..ToolSettings::default()
    }
    .with_font(font)
    .with_drag_aids(|id| session.settings().get(id).ok())
}

/// Runs `f` with the toolbox and the context of the edited glyph.
fn with_tool<R>(app: &mut FontCraftApp, f: impl FnOnce(&mut Toolbox, &Context<'_>) -> R) -> Result<R, String> {
    sync(app);
    let view = tools_view(&camera(app));
    let marks = crate::canvas::hinting::ps_marks(&mut app.session, &app.view);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut scene = scene(font, &app.view, app.tools.focus.as_ref()).ok_or("no glyph is being edited")?;
    scene.ps_hints = marks;
    let settings = settings(&app.session, font, app);
    let cx = Context { scene: &scene, view, selection: &app.tools.selection, settings };
    Ok(f(&mut app.tools.toolbox, &cx))
}

/// Brings the tools in line with the app before they see an event.
pub fn sync(app: &mut FontCraftApp) {
    let target = super::target(&app.session, &app.view);
    if target != app.tools.target {
        // Another glyph, layer or side: a gesture in progress ends without touching the new one.
        let _ = app.tools.toolbox.cancel();
        if app.tools.before.take().is_some() && app.session.interaction().is_some() {
            let _ = app.session.cancel();
        }
        app.tools.selection = Selection::new();
        app.tools.focus = None;
        app.tools.target = target;
    }
    if app.tools.before.is_some() && app.session.interaction().is_none() {
        // The interaction ended elsewhere: the gesture ends, and the selection is what it was.
        let _ = app.tools.toolbox.cancel();
        if let Some(before) = app.tools.before.take() {
            app.tools.selection = before;
        }
    }
    let revision = app.session.active().map(|d| (d.id, d.revision));
    if revision != app.tools.checked {
        app.tools.checked = revision;
        // After an edit, undo or redo, drop the addresses that no longer resolve.
        if let Ok(font) = app.session.font()
            && let Some(scene) = scene(font, &app.view, None)
        {
            app.tools.selection.retain_valid(&scene);
            if let Some(focus) = app.tools.focus.as_mut() {
                focus.retain(|&shape| scene.path(shape).is_some());
            }
        }
    }
    if app.tools.toolbox.base_id() != app.view.tool
        && let Ok(switch) = app.tools.toolbox.select(app.view.tool)
    {
        let _ = apply(app, switch.output);
    }
    if !app.tools.selection.hints.is_empty() {
        // Hidden hints cannot be acted on; hints an edit removed are no longer selected.
        let marks = crate::canvas::hinting::ps_marks(&mut app.session, &app.view);
        if marks.shown {
            app.tools.selection.hints.retain(|&i| marks.get(i).is_some());
        } else {
            app.tools.selection.hints.clear();
        }
    }
    let base = app.tools.toolbox.base_id();
    app.active_tool.set_tool(base);
}

/// Hands `input` to the toolbox and carries out its answer. Returns the errors, in order.
pub fn input(app: &mut FontCraftApp, input: ToolInput) -> Vec<String> {
    if crate::canvas::text::text_mode(&app.view) {
        return crate::canvas::text::input::input(app, input);
    }
    if let ToolInput::Key(ev) = input {
        let out = match with_tool(app, |toolbox, cx| toolbox.key(cx, &ev)) {
            Ok(out) => out,
            Err(e) => return vec![e],
        };
        if !out.handled
            && let Some(result) = crate::canvas::text::input::escape_to_text(app, &ev)
        {
            return result.err().into_iter().collect();
        }
        return apply(app, out);
    }
    let out = match input {
        ToolInput::Pointer(ev) => {
            if let Some(errors) = crate::canvas::color::knobs::pointer(app, ev) {
                return errors;
            }
            app.tools.mods = ev.mods;
            if let Some(errors) = crate::canvas::hinting::badges::pointer(app, &ev) {
                return errors;
            }
            with_tool(app, |toolbox, cx| toolbox.pointer(cx, &ev))
        }
        ToolInput::Modifiers(mods) => {
            if app.tools.mods == mods {
                return Vec::new();
            }
            app.tools.mods = mods;
            // Building the context costs a scene; most modifier changes need none.
            if !app.tools.toolbox.reacts_to(mods) {
                return Vec::new();
            }
            with_tool(app, |toolbox, cx| toolbox.modifiers(cx, mods))
        }
        ToolInput::Key(ev) => {
            if let Some(errors) = crate::canvas::hinting::badges::key(app, &ev) {
                return errors;
            }
            with_tool(app, |toolbox, cx| toolbox.key(cx, &ev))
        }
    };
    let errors = match out {
        Ok(out) => apply(app, out),
        Err(e) => vec![e],
    };
    crate::canvas::overlays::after_input(app, &input);
    errors
}

/// Whether the toolbox takes the key `ev` now, and what that did: `None` when the key is not the tools'.
pub fn key(app: &mut FontCraftApp, ev: fontcraft_tools::KeyEvent) -> Option<Vec<String>> {
    if crate::canvas::text::text_mode(&app.view) {
        return crate::canvas::text::input::takes_key(&ev).then(|| crate::canvas::text::input::input(app, ToolInput::Key(ev)));
    }
    let out = with_tool(app, |toolbox, cx| toolbox.key(cx, &ev)).ok()?;
    if !out.handled {
        return crate::canvas::text::input::escape_to_text(app, &ev).map(|result| result.err().into_iter().collect());
    }
    Some(apply(app, out))
}

/// Carries out a tool's actions in order. Returns the errors.
pub fn apply(app: &mut FontCraftApp, out: Output) -> Vec<String> {
    let out = crate::canvas::overlays::unless_locked(app, out);
    let mut errors = Vec::new();
    for action in out.actions {
        let ran = match &action {
            Action::Run { invocation } => Some((invocation.id.clone(), invocation.params.clone())),
            _ => None,
        };
        let msg = match action {
            Action::Begin { label } => Msg::Preview(Preview::Begin(label)),
            Action::Preview { invocation } => Msg::Preview(Preview::Update(CommandRef::with_params(invocation.id, invocation.params))),
            Action::Commit => Msg::Preview(Preview::Commit),
            Action::Cancel => Msg::Preview(Preview::Cancel),
            Action::Run { invocation } => FontCraftApp::route(CommandRef::with_params(invocation.id, invocation.params)),
            Action::Select { selection } => {
                app.tools.selection = selection;
                continue;
            }
            Action::View { change } => {
                view_change(app, change);
                continue;
            }
        };
        let begin = matches!(msg, Msg::Preview(Preview::Begin(_)));
        match app.apply_msg(msg) {
            Err(e) => {
                errors.push(e);
                if begin {
                    // The tool cannot drag without an interaction: end its gesture and skip the previews that follow.
                    let _ = app.tools.toolbox.cancel();
                    break;
                }
            }
            Ok(()) => {
                if let Some((id, params)) = ran {
                    crate::canvas::annotation_editor::after_tool_run(app, &id, &params);
                }
            }
        }
    }
    errors
}

/// Carries out one step of an interaction.
pub fn preview(app: &mut FontCraftApp, step: Preview) -> Result<(), String> {
    match step {
        Preview::Begin(label) => {
            if super::target(&app.session, &app.view).is_none() {
                return Err("no glyph is being edited".into());
            }
            app.session.begin(&label).map_err(|e| e.to_string())?;
            app.tools.before = Some(app.tools.selection.clone());
            Ok(())
        }
        Preview::Update(command) => {
            let params = if command.params.is_null() { serde_json::json!({}) } else { command.params.clone() };
            let value = app.session.preview(&command.id, &params).map_err(|e| format!("{}: {e}", command.id))?;
            adopt(app, &value);
            Ok(())
        }
        Preview::Commit => {
            app.tools.before = None;
            app.session.commit().map(drop).map_err(|e| e.to_string())
        }
        Preview::Cancel => {
            let before = app.tools.before.take();
            if app.session.interaction().is_some() {
                app.session.cancel().map_err(|e| e.to_string())?;
            }
            if let Some(before) = before {
                app.tools.selection = before;
            }
            Ok(())
        }
    }
}

/// Makes `tool` the toolbox's tool; a gesture in progress is cancelled.
pub fn select_tool(app: &mut FontCraftApp, tool: &'static str) -> Result<(), String> {
    let switch = app.tools.toolbox.select(tool)?;
    apply(app, switch.output).pop().map_or(Ok(()), Err)
}

/// Takes the `selection` a command returned, when it belongs to the edited glyph's edited side.
pub fn adopt(app: &mut FontCraftApp, value: &Value) {
    let Some(sel) = value.get("selection") else { return };
    let Some(target) = super::target(&app.session, &app.view) else { return };
    let side = sel.get("side").and_then(Value::as_str).unwrap_or("foreground");
    if sel.get("glyph").and_then(Value::as_str) != Some(target.glyph.as_str()) || side != target.side.as_str() {
        return;
    }
    if let Ok(selection) = Selection::from_value(sel) {
        app.tools.selection = selection;
    }
}

/// Moves the canvas view as a tool asked.
fn view_change(app: &mut FontCraftApp, change: ViewChange) {
    let camera = camera(app);
    let view = &mut app.view.canvas;
    match change {
        ViewChange::Pan { dx, dy } => {
            if dx.is_finite() && dy.is_finite() {
                view.center -= view.unmap_vec(egui::vec2(dx as f32, dy as f32));
            }
        }
        ViewChange::Zoom { factor, at } => {
            let zoom = view.zoom * factor;
            if !zoom.is_finite() {
                return;
            }
            // Keep `at` where it is on screen.
            let doc = camera.font_to_doc() * KPoint::new(at.x, at.y);
            let before = view.to_screen(doc);
            view.zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            let after = view.to_screen(doc);
            view.center += view.unmap_vec(after - before);
        }
        ViewChange::ZoomToRect { x, y, width, height } => {
            let size = view.rect.size();
            if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) || size.x <= 0.0 || size.y <= 0.0 {
                return;
            }
            let zoom = (f64::from(size.x) / width).min(f64::from(size.y) / height);
            view.zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            view.center = camera.font_to_doc() * KPoint::new(x + width / 2.0, y + height / 2.0);
        }
    }
}
