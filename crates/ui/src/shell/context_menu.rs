//! The context menus and the clipboard in the window.
//!
//! | Command | Does |
//! |---|---|
//! | `contextMenu.open {x, y, space?, alt?}` | opens the canvas menu at a point, as a secondary click does (`app::context_menu`) |
//! | `contextMenu.invoke {command, value?}` | runs an item of the open canvas menu, as a click on it does; `value` picks a submenu item's choice |
//! | `contextMenu.close` | closes the open menus |
//! | `fontView.openGlyphMenu {glyphs, x?, y?, alt?}` | opens Font View's glyph menu on these glyphs; with `alt` its swatches colour the layer |
//! | `fontView.showTags` | shows the sidebar, whose glyph inspector holds the Tags field (the glyph menu's More Tags…) |
//!
//! [`show`] runs every pass from the window's body: it hands clipboard text to the window, takes the clipboard's text
//! for a waiting paste, and shows the open menus. A pick becomes the item's messages.

use craft::ui::contract::BodyCx;
use craft::ui::runtime::commands::{ClipItem, CommandRef, Commands as _};
use egui::Ui;
use fontcraft_geom::Point;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use super::dock::{self, PanelId};
use crate::app::clipboard;
use crate::app::context_menu::glyph_menu::{self, GlyphAction};
use crate::app::context_menu::{self as menus, Space};
use crate::app::{FontCraftApp, Msg};
use crate::panels::context_menu::{self as views, Pick};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The glyph menu's More Tags….
pub const SHOW_TAGS: &str = "fontView.showTags";

/// Passes a clipboard request waits for its text before it gives up.
const PASTE_WAIT: u64 = 3;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "contextMenu.open",
        label: "Open Context Menu",
        menu: &[],
        shortcut: None,
        params: "{x: number, y: number, space?: \"screen\" | \"font\" (default screen), alt?: bool} open the canvas context menu at this point as a secondary click does: an object there that is not selected becomes the selection first. Nothing opens while a tool drags or a dialog is open",
    },
    UiCommand {
        id: "contextMenu.invoke",
        label: "Run Context Menu Item",
        menu: &[],
        shortcut: None,
        params: "{command: id, value?: string (a submenu item's choice, as `ui.contextMenu.list` lists it under `choices`)} run an item of the open canvas context menu as a click on it does, and close the menu. A disabled or missing item, a submenu item without one of its values, or a value for a plain item is refused and the menu stays open",
    },
    UiCommand { id: "contextMenu.close", label: "Close Context Menu", menu: &[], shortcut: None, params: "{} close the open context menus" },
    UiCommand {
        id: "fontView.openGlyphMenu",
        label: "Open Glyph Menu",
        menu: &[],
        shortcut: None,
        params: "{glyphs: [names], x?: number, y?: number, alt?: bool (Alt held at the click: the swatches set the layer colour instead of the glyph colour)} open Font View's glyph context menu on these glyphs at this window point",
    },
    UiCommand {
        id: SHOW_TAGS,
        label: "Show Tags",
        menu: &[],
        shortcut: None,
        params: "{} show the sidebar, whose glyph inspector holds the Tags field of the selected glyphs",
    },
];

fn number(params: &Value, key: &str) -> Result<f64, String> {
    params.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| format!("`{key}` must be a finite number"))
}

fn optional_number(params: &Value, key: &str) -> Result<f64, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(_) => number(params, key),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "contextMenu.open" => {
            let space = match p.get("space").and_then(Value::as_str) {
                None => Space::Screen,
                Some(name) => Space::parse(name).ok_or("`space` is \"screen\" or \"font\"")?,
            };
            menus::open_canvas(app, Point::new(number(p, "x")?, number(p, "y")?), space)?;
            if let Some(menu) = app.context_menus.canvas.as_mut() {
                menu.alt = p.get("alt").and_then(Value::as_bool).unwrap_or(false);
            }
            Ok(())
        }
        "contextMenu.invoke" => {
            let id = p.get("command").and_then(Value::as_str).ok_or("`command` is required")?;
            let value = p.get("value").and_then(Value::as_str);
            menus::invoke_choice(app, id, value, FontCraftApp::apply_msg_reply).map(drop)
        }
        "contextMenu.close" => {
            app.context_menus = menus::ContextMenus::default();
            Ok(())
        }
        "fontView.openGlyphMenu" => {
            let list = p.get("glyphs").and_then(Value::as_array).ok_or("`glyphs` must be a list of glyph names")?;
            let glyphs: Vec<String> = list.iter().take(glyph_menu::MAX_MENU_GLYPHS).filter_map(Value::as_str).map(str::to_string).collect();
            let at = [optional_number(p, "x")?, optional_number(p, "y")?];
            if at.iter().any(|v| v.abs() > menus::MAX_COORDINATE) {
                return Err(format!("`x` and `y` are at most {} either way", menus::MAX_COORDINATE));
            }
            let mut menu = glyph_menu::open(app, &glyphs, [at[0] as f32, at[1] as f32]);
            menu.alt = p.get("alt").and_then(Value::as_bool).unwrap_or(false);
            if menu.glyphs.is_empty() {
                return Err("none of these glyphs is in the font".into());
            }
            app.context_menus.glyph = Some(menu);
            Ok(())
        }
        SHOW_TAGS => {
            if app.view.workspace.layout.panel_state(&PanelId::Sidebar) == craft::behavior::dock::PanelState::Hidden {
                dock::toggle(&mut app.view.workspace, PanelId::Sidebar);
            }
            Ok(())
        }
        other => Err(format!("no context menu command `{other}`")),
    }
}

/// The window's part: clipboard text out, the clipboard's text in for a waiting paste, and the open menus.
pub fn show(ui: &mut Ui, cx: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    if let Some(text) = app.clipboard.out.take() {
        cx.clipboard().put(&[ClipItem::Text(text.into())]);
    }
    take_paste(ui, app);
    let opener = ui.response();
    if let Some(menu) = app.context_menus.canvas.clone() {
        let still = menus::still(app, &menu) && app.dialogs.open.is_none();
        let answer = if still { views::canvas_menu(ui, &opener, &menu, menu.alt) } else { views::Answer { picked: None, open: false } };
        // `contextMenu.invoke` closes the menu after it checked the item against it.
        if !answer.open && !matches!(answer.picked, Some(Pick::Item(_) | Pick::Choice(..))) {
            app.context_menus.canvas = None;
        }
        match answer.picked {
            Some(Pick::Item(id)) => app.sent.push(Msg::Shell(CommandRef::with_params("contextMenu.invoke", serde_json::json!({ "command": id })))),
            Some(Pick::Choice(id, value)) => {
                app.sent.push(Msg::Shell(CommandRef::with_params("contextMenu.invoke", serde_json::json!({ "command": id, "value": value }))));
            }
            Some(Pick::Color(color)) => {
                let steps = menus::color_steps(app, &menu, color, menu.alt);
                app.sent.extend(steps);
            }
            Some(Pick::CopyNames(format)) => {
                let target = glyph_menu::GlyphMenu { glyphs: vec![menu.glyph.clone()], ..Default::default() };
                app.sent.extend(glyph_menu::steps(&target, &GlyphAction::CopyNames(format)));
            }
            _ => {}
        }
    }
    if let Some(menu) = app.context_menus.glyph.clone() {
        let answer = if app.dialogs.open.is_none() { views::glyph_menu(ui, &opener, &menu) } else { views::Answer { picked: None, open: false } };
        if !answer.open {
            app.context_menus.glyph = None;
        }
        if let Some(action) = answer.picked.and_then(|pick| glyph_action(&menu, pick)) {
            app.sent.extend(glyph_menu::steps(&menu, &action));
        }
    }
}

/// What a pick in the glyph menu does: a swatch colours the glyphs, or their layer when Alt was held at the click.
pub(crate) fn glyph_action(menu: &glyph_menu::GlyphMenu, pick: Pick) -> Option<GlyphAction> {
    match pick {
        Pick::Color(c) if menu.alt => Some(GlyphAction::LayerColor(c)),
        Pick::Color(c) => Some(GlyphAction::Color(c)),
        Pick::Export => Some(GlyphAction::Export),
        Pick::Tag(tag) => Some(GlyphAction::Tag(tag)),
        Pick::MoreTags => Some(GlyphAction::MoreTags),
        Pick::CopyNames(format) => Some(GlyphAction::CopyNames(format)),
        Pick::Status(code) => Some(GlyphAction::Status(code)),
        Pick::Item(_) | Pick::Choice(..) => None,
    }
}

/// Takes the clipboard's text for a paste that waits for it: from the system clipboard where the app has a reader
/// ([`clipboard::read_waiting`]); else asks the window for it once, takes the paste event that brings it, and gives up
/// when none comes.
fn take_paste(ui: &Ui, app: &mut FontCraftApp) {
    if clipboard::read_waiting(app) {
        return;
    }
    let Some(kind) = app.clipboard.request else { return };
    let pasted = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(text) => Some(text.clone()),
            _ => None,
        })
    });
    let pass = ui.ctx().cumulative_pass_nr();
    match (pasted, app.clipboard.asked) {
        (Some(text), _) => {
            app.clipboard.request = None;
            app.clipboard.asked = None;
            clipboard::paste(app, &text, kind);
        }
        (None, None) => {
            app.clipboard.asked = Some(pass);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ui.ctx().request_repaint();
        }
        (None, Some(asked)) if pass.saturating_sub(asked) > PASTE_WAIT => {
            app.clipboard.request = None;
            app.clipboard.asked = None;
            app.status = "Paste: the clipboard holds no text".into();
        }
        (None, Some(_)) => ui.ctx().request_repaint(),
    }
}
