//! Typing an anchor's name on the canvas (`anchor.add`, `anchor.rename`).
//!
//! Add Anchor in the canvas's context menu opens a field at the new anchor, its name selected ([`after_add`]). A
//! double-click on an anchor, and Return with one anchor selected alone, run `anchor.editName`, which opens the same
//! field. Return or a click outside commits the name through `anchor.rename`; a name that is empty, taken or refused
//! keeps the field open with the reason under it. Escape keeps the name the anchor had. Adding an anchor and naming it
//! is one undo step: committing undoes the bare add and adds the anchor again under its name.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::popover::Popover;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::text::TextField;
use egui::{Pos2, Ui};
use fontcraft_engine::model::{Anchor, LayerSide};
use serde_json::{Map, Value, json};

use super::overlays::OverlayWords;
use crate::app::FontCraftApp;
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params::{quoted, string};

/// Most characters the field takes; the engine's limit is in bytes.
const MAX_NAME_CHARS: usize = 255;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "anchor.editName",
    label: "Edit Anchor Name",
    menu: &[],
    shortcut: None,
    params: "{anchor?: name (default the only selected anchor)} open the field that renames an anchor of the edited layer at the anchor's place on the canvas. Return or a click outside renames it (anchor.rename), Escape keeps its name; text mode and a locked glyph are errors",
}];

/// Why a committed name was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    Empty,
    Taken,
    /// The engine's message.
    Other(String),
}

/// The anchor name being typed.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchorEdit {
    /// The document the anchor is in.
    pub doc: Option<u64>,
    pub glyph: String,
    pub layer: String,
    pub side: LayerSide,
    /// The anchor's name now.
    pub anchor: String,
    pub text: String,
    /// The document and its revision right after Add Anchor made the anchor: adding and naming become one undo step
    /// while it holds. `None` for an anchor that was there.
    pub added_at: Option<(u64, u64)>,
    /// Why the last commit was refused.
    pub problem: Option<Problem>,
    /// The field is open; the popover writes `false` when it closes.
    pub open: bool,
    /// The field closed or was submitted: `Some(true)` commits, `Some(false)` cancels.
    pub done: Option<bool>,
}

impl AnchorEdit {
    /// The parameters that name the edited side of the layer, with `extra`.
    fn params(&self, extra: &[(&str, Value)]) -> Value {
        let mut m = Map::new();
        m.insert("glyph".into(), json!(self.glyph));
        m.insert("layer".into(), json!(self.layer));
        if self.side == LayerSide::Background {
            m.insert("side".into(), json!("background"));
        }
        for (key, value) in extra {
            m.insert((*key).to_string(), value.clone());
        }
        Value::Object(m)
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "anchor.editName" => {
            let name = match string(&command.params, "anchor")? {
                Some(name) => name.to_string(),
                None => {
                    let mut anchors = app.tools.selection.anchors.iter();
                    match (anchors.next(), anchors.next()) {
                        (Some(only), None) => only.clone(),
                        _ => return Err("select exactly one anchor, or give `anchor`".into()),
                    }
                }
            };
            open(app, &name)
        }
        other => Err(format!("no anchor command `{other}`")),
    }
}

/// The anchor `name` of the edited side of the edited layer.
fn anchor(app: &FontCraftApp, edit: (&str, &str, LayerSide), name: &str) -> Option<Anchor> {
    let (glyph, layer, side) = edit;
    let layer = app.session.font().ok()?.glyph(glyph)?.layers.iter().find(|l| l.id == layer)?;
    layer.parts(side).anchors.iter().find(|a| a.name == name).cloned()
}

fn open(app: &mut FontCraftApp, name: &str) -> Result<(), String> {
    if super::text::text_mode(&app.view) {
        return Err("the text is being edited; edit the glyph first".into());
    }
    let target = super::tools::target(&app.session, &app.view).ok_or("no glyph is being edited")?;
    if app.session.font().ok().and_then(|f| f.glyph(&target.glyph)).is_some_and(|g| g.info.locked) {
        return Err(format!("glyph `{}` is locked: unlock it to rename its anchors", quoted(&target.glyph)));
    }
    if anchor(app, (&target.glyph, &target.layer, target.side), name).is_none() {
        return Err(format!("the edited layer has no anchor `{}`", quoted(name)));
    }
    app.overlays.anchor = Some(AnchorEdit {
        doc: app.session.active().map(|d| d.id),
        glyph: target.glyph,
        layer: target.layer,
        side: target.side,
        anchor: name.to_string(),
        text: name.to_string(),
        added_at: None,
        problem: None,
        open: true,
        done: None,
    });
    Ok(())
}

/// After the context menu's Add Anchor ran with the reply `result`: opens the field on the new anchor.
pub fn after_add(app: &mut FontCraftApp, result: &Value) {
    let Some(name) = result.get("anchor").and_then(Value::as_str).map(str::to_string) else { return };
    let revision = app.session.active().map(|d| (d.id, d.revision));
    if open(app, &name).is_ok()
        && let Some(edit) = app.overlays.anchor.as_mut()
    {
        edit.added_at = revision;
    }
}

/// Is the anchor of `edit` there, in the document, glyph, layer and side being edited?
fn there(app: &FontCraftApp, edit: &AnchorEdit) -> bool {
    let here = super::tools::target(&app.session, &app.view).is_some_and(|t| t.glyph == edit.glyph && t.layer == edit.layer && t.side == edit.side);
    app.session.active().map(|d| d.id) == edit.doc
        && here
        && !super::text::text_mode(&app.view)
        && anchor(app, (&edit.glyph, &edit.layer, edit.side), &edit.anchor).is_some()
}

/// Drops an edit whose anchor is no longer there. Runs in `drain`, after the messages.
pub fn drop_stale(app: &mut FontCraftApp) {
    if app.overlays.anchor.as_ref().is_some_and(|edit| !there(app, edit)) {
        app.overlays.anchor = None;
    }
}

/// Finishes the edit whose field closed. Runs in `drain`, before the messages: a click outside can also change what is
/// edited, and its name goes to the anchor it was typed for.
pub fn finish(app: &mut FontCraftApp) {
    let Some(commit) = app.overlays.anchor.as_ref().and_then(|edit| edit.done) else { return };
    let Some(mut edit) = app.overlays.anchor.take() else { return };
    if !commit || edit.text.trim() == edit.anchor || !there(app, &edit) {
        return;
    }
    if let Err(problem) = rename(app, &edit) {
        edit.problem = Some(problem);
        edit.open = true;
        edit.done = None;
        app.overlays.anchor = Some(edit);
    }
}

/// Renames the anchor of `edit` to its text.
fn rename(app: &mut FontCraftApp, edit: &AnchorEdit) -> Result<(), Problem> {
    let name = edit.text.trim();
    if name.is_empty() {
        return Err(Problem::Empty);
    }
    let at = (edit.glyph.as_str(), edit.layer.as_str(), edit.side);
    if anchor(app, at, name).is_some() {
        return Err(Problem::Taken);
    }
    let added = anchor(app, at, &edit.anchor);
    let revision = app.session.active().map(|d| (d.id, d.revision));
    app.run_command("anchor.rename", edit.params(&[("anchor", json!(edit.anchor)), ("name", json!(name))])).map_err(Problem::Other)?;
    // The name is good. Nothing else was edited since the add: the add and the rename become one add under the name.
    if let Some(added) = added.filter(|_| edit.added_at.is_some() && edit.added_at == revision) {
        let place = [("name", json!(name)), ("x", json!(added.x)), ("y", json!(added.y))];
        let mut undone = 0usize;
        let mut result = Ok(());
        while result.is_ok() && undone < 2 {
            result = app.run_command("edit.undo", json!({})).map(drop);
            undone += usize::from(result.is_ok());
        }
        if result.is_ok() {
            result = app.run_command("anchor.add", edit.params(&place)).map(drop);
        }
        // The add and the rename stay two steps.
        if let Err(error) = result {
            for _ in 0..undone {
                if app.run_command("edit.redo", json!({})).is_err() {
                    break;
                }
            }
            app.status = error;
        }
    }
    Ok(())
}

/// What the field reads and changes.
pub struct EditorProps<'a> {
    pub edit: &'a mut AnchorEdit,
    /// The anchor on screen.
    pub at: Pos2,
}

/// Shows the field at the anchor.
pub fn show(ui: &mut Ui, props: EditorProps<'_>) {
    let words = Env::of(ui).words::<OverlayWords>();
    let edit = props.edit;
    let problem = edit.problem.as_ref().map(|p| match p {
        Problem::Empty => words.anchor_empty.as_str().to_string(),
        Problem::Taken => words.anchor_taken.as_str().to_string(),
        Problem::Other(message) => message.clone(),
    });
    let (text, open) = (&mut edit.text, &mut edit.open);
    let out = Popover::new(words.anchor.clone(), Bind::bound(open)).at_point(props.at).show(ui, |ui, _| {
        let field = TextField::new(words.anchor.clone(), Bind::bound(&mut *text)).select_on_focus().limit(MAX_NAME_CHARS).submits().show(ui);
        if let Some(problem) = &problem {
            Text::new(problem.clone()).shade(Shade::Muted).show(ui);
        }
        if field.escaped {
            Some(false)
        } else if field.submitted {
            Some(true)
        } else {
            None
        }
    });
    let done = out.inner.flatten().or(match out.output.closed {
        Some(CloseReason::Escape) => Some(false),
        Some(_) => Some(true),
        None => None,
    });
    if done.is_some() {
        edit.done = done;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point;
    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use egui::{Pos2, Rect, vec2};
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::Problem;
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    fn app() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), Point::new(400.0, 400.0), 1.0);
        app
    }

    fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
        crate::control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    fn names(app: &FontCraftApp) -> Vec<String> {
        let font = app.session.font().unwrap();
        font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().anchors.iter().map(|a| a.name.clone()).collect()
    }

    fn undo_steps(app: &FontCraftApp) -> usize {
        app.session.inspect()["documents"][0]["undo"].as_array().map_or(0, Vec::len)
    }

    fn type_name(app: &mut FontCraftApp, name: &str, commit: bool) {
        let edit = app.overlays.anchor.as_mut().expect("the name field is open");
        edit.text = name.into();
        edit.done = Some(commit);
        app.drain();
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
    }

    fn add_at(app: &mut FontCraftApp, x: f64, y: f64) {
        call(app, "ui.contextMenu.list", json!({"x": x, "y": y}));
        call(app, "ui.contextMenu.invoke", json!({"command": "anchor.add"}));
    }

    /// Add Anchor, a typed name and Return give one anchor in one undo step; an empty or taken name keeps the field open
    /// and changes nothing; Escape leaves the new anchor its first name.
    #[test]
    fn a_new_anchor_is_named_in_one_undo_step() {
        let mut app = app();
        let steps = undo_steps(&app);
        add_at(&mut app, 300.0, 500.0);
        assert_eq!(app.overlays.anchor.as_ref().map(|e| e.text.as_str()), Some("new anchor"));
        type_name(&mut app, " top ", true);
        assert!(app.overlays.anchor.is_none(), "{:?}", app.overlays.anchor);
        assert_eq!((names(&app), undo_steps(&app)), (vec!["top".to_string()], steps + 1));
        assert!(app.tools.selection.anchors.contains("top"), "the anchor is selected under its name");

        add_at(&mut app, 300.0, 0.0);
        for (text, problem) in [("top", Problem::Taken), ("  ", Problem::Empty)] {
            let before = (names(&app), undo_steps(&app));
            type_name(&mut app, text, true);
            let edit = app.overlays.anchor.as_ref().expect("the field stays open");
            assert_eq!((edit.open, edit.problem.as_ref()), (true, Some(&problem)));
            assert_eq!((names(&app), undo_steps(&app)), before);
        }
        type_name(&mut app, "bottom", false);
        assert!(app.overlays.anchor.is_none());
        assert_eq!(names(&app), ["top", "new anchor"], "Escape keeps the name");
    }

    /// The window's menu, which posts its choice, opens the field too.
    #[test]
    fn the_windows_add_anchor_opens_the_name_field() {
        let mut app = app();
        shell(&mut app, "contextMenu.open", json!({"x": 300, "y": 500, "space": "font"}));
        shell(&mut app, "contextMenu.invoke", json!({"command": "anchor.add"}));
        assert_eq!(app.overlays.anchor.as_ref().map(|e| e.text.as_str()), Some("new anchor"), "{}", app.status);
        type_name(&mut app, "top", true);
        assert_eq!(names(&app), ["top"]);
    }

    /// A click outside that also changes what is edited commits the name to the glyph and the document it was typed in;
    /// an open field closes when another document comes to the front.
    #[test]
    fn a_name_goes_to_the_glyph_and_document_it_was_typed_in() {
        let mut app = app();
        let anchors = |app: &FontCraftApp, doc: usize| -> Vec<String> {
            let font = &app.session.documents()[doc].font;
            font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().anchors.iter().map(|a| a.name.clone()).collect()
        };
        let open = |app: &mut FontCraftApp, name: &str| {
            app.sent.extend([Msg::View(ViewMsg::SelectGlyph("A".into())), Msg::View(ViewMsg::SelectTool("select"))]);
            app.drain();
            shell(app, "anchor.editName", json!({ "anchor": name }));
            assert!(app.overlays.anchor.is_some(), "{}", app.status);
        };
        let commit_with = |app: &mut FontCraftApp, name: &str, click: Msg| {
            let edit = app.overlays.anchor.as_mut().unwrap();
            edit.text = name.into();
            edit.done = Some(true);
            app.sent.push(click);
            app.drain();
            assert!(app.overlays.anchor.is_none());
        };
        app.run_now(Msg::run("anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 500})), false).unwrap();
        open(&mut app, "top");
        commit_with(&mut app, "ogonek", Msg::View(ViewMsg::SelectGlyph("B".into())));
        assert_eq!(anchors(&app, 0), ["ogonek"]);

        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        app.run_now(Msg::run("anchor.add", json!({"glyph": "A", "name": "ogonek", "x": 300, "y": 500})), false).unwrap();
        app.run_now(Msg::run("document.activate", json!({"index": 0})), false).unwrap();
        open(&mut app, "ogonek");
        commit_with(&mut app, "exit", Msg::run("document.activate", json!({"index": 1})));
        assert_eq!((anchors(&app, 0), anchors(&app, 1)), (vec!["exit".to_string()], vec!["ogonek".to_string()]));

        open(&mut app, "ogonek");
        app.sent.push(Msg::run("document.activate", json!({"index": 0})));
        app.drain();
        assert!(app.overlays.anchor.is_none());
    }

    /// A double-click, Return with the anchor selected alone, and the Info box name the anchor; Return with a node selected
    /// too does not open the field.
    #[test]
    fn an_anchor_is_renamed_by_double_click_return_and_the_info_box() {
        let mut app = app();
        app.sent.push(Msg::run("anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 500})));
        app.drain();
        let click = json!([{"kind": "down", "x": 300, "y": 500}, {"kind": "up", "x": 300, "y": 500}]);
        call(&mut app, "ui.pointer", json!({ "events": click }));
        assert_eq!(app.models.palette.info_box.selection.as_ref().and_then(|s| s.anchor.as_deref()), Some("top"));
        call(&mut app, "ui.pointer", json!({"events": [{"kind": "doubleclick", "x": 300, "y": 500}]}));
        let steps = undo_steps(&app);
        type_name(&mut app, "ogonek", true);
        assert_eq!((names(&app), undo_steps(&app)), (vec!["ogonek".to_string()], steps + 1));

        call(&mut app, "ui.key", json!({"key": "Return"}));
        assert_eq!(app.overlays.anchor.as_ref().map(|e| e.anchor.as_str()), Some("ogonek"));
        type_name(&mut app, "bottom", true);
        assert_eq!(names(&app), ["bottom"]);

        let target = app.models.palette.info_box.selection.as_ref().unwrap().rename.clone();
        app.sent.push(Msg::run("anchor.rename", target.with(json!({"anchor": "bottom", "name": "exit"}))));
        app.drain();
        assert_eq!(names(&app), ["exit"]);
        app.drain();
        assert_eq!(app.models.palette.info_box.selection.as_ref().and_then(|s| s.anchor.as_deref()), Some("exit"));

        app.sent.push(Msg::run("path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})));
        app.drain();
        app.tools.selection =
            fontcraft_tools::Selection::from_items([fontcraft_tools::Item::Point(0, 0), fontcraft_tools::Item::Anchor("exit".into())]);
        call(&mut app, "ui.key", json!({"key": "Return"}));
        assert!(app.overlays.anchor.is_none(), "with a node selected Return toggles its type");
    }

    /// Dragging a copy of an anchor, then of the copy, appends underscores to the name.
    #[test]
    fn option_drag_names_copies_with_underscores() {
        let mut app = app();
        app.sent.push(Msg::run("anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 500})));
        app.drain();
        for (x, y) in [(300.0, 500.0), (380.0, 530.0)] {
            let events = json!([
                {"kind": "down", "x": x, "y": y, "mods": ["alt"]},
                {"kind": "drag", "x": x + 80.0, "y": y + 30.0, "mods": ["alt"]},
                {"kind": "up", "x": x + 80.0, "y": y + 30.0, "mods": ["alt"]},
            ]);
            call(&mut app, "ui.pointer", json!({ "events": events }));
        }
        assert_eq!(names(&app), ["top", "top_", "top__"]);
    }
}
