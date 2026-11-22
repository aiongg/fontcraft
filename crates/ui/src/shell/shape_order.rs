//! The Shape Order panel's shell command and view state.
//!
//! | Command | Does |
//! |---|---|
//! | `window.shapeOrder {open?, glyph?}` | shows or hides the panel, on a named glyph or on the glyph being edited |
//!
//! What the panel lists is its projection's (`app::models::shape_order`); `ui.inspect` reports it under
//! `shapeOrder`. A drop in the panel runs `glyph.setShapeOrder`, an engine command.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;
use crate::app::models::shape_order::ShapeKind;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "window.shapeOrder",
    label: "Shape Order",
    menu: &["Window"],
    shortcut: None,
    params: "{open?: bool (default true), glyph?: name (default: the glyph being edited, followed as it changes)} show or hide the Shape Order panel: every path and component of every master layer and special layer of the glyph as numbered thumbnails in stacking order, one list per layer. Dropping a shape on another place in its list reorders that layer with glyph.setShapeOrder {glyph, layer, order}. ui.inspect reports {open, glyph, layers: [{layer, name, shapes: [{index, kind, glyph?}]}]} under shapeOrder",
}];

/// The panel's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeOrderView {
    /// The glyph the panel was opened on; `None` follows the glyph being edited.
    pub glyph: Option<String>,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != "window.shapeOrder" {
        return Err(format!("no Shape Order command `{}`", command.id));
    }
    let p = &command.params;
    let open = flag(p, "open")?.unwrap_or(true);
    let named = string(p, "glyph")?.map(str::to_string);
    if open {
        let font = app.session.font().map_err(|e| e.to_string())?;
        match &named {
            Some(name) if font.glyph(name).is_none() => return Err(format!("the font has no glyph `{}`", quoted(name))),
            Some(_) => {}
            None if crate::shell::masters::shown_glyph(font, &app.view).is_none() => {
                return Err("open a glyph first, or name one with `glyph`".into());
            }
            None => {}
        }
        app.view.shape_order.glyph = named;
    }
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::ShapeOrder, open, app.view.canvas.rect);
    Ok(())
}

/// `ui.inspect`'s `shapeOrder`: whether the panel shows, its glyph and each layer's shapes in order.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let open = app.view.workspace.layout.panel_state(&PanelId::ShapeOrder) != craft::behavior::dock::PanelState::Hidden;
    let model = &app.models.shape_order;
    let layers: Vec<Value> = model
        .columns
        .iter()
        .map(|c| {
            let shapes: Vec<Value> = c
                .shapes
                .iter()
                .enumerate()
                .map(|(index, s)| match &s.kind {
                    ShapeKind::Component { glyph } => json!({"index": index, "kind": "component", "glyph": glyph}),
                    kind => json!({"index": index, "kind": kind.word()}),
                })
                .collect();
            json!({"layer": c.layer, "name": c.title, "shapes": shapes, "total": c.total})
        })
        .collect();
    state.insert("shapeOrder".into(), json!({"open": open, "glyph": model.glyph, "layers": layers}));
}
