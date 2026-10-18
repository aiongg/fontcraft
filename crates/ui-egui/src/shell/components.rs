//! Moving between composites, their base glyphs and corner glyphs in Edit View:
//!
//! | Command | Reached from | Does |
//! |---|---|---|
//! | `component.editBaseGlyph` | the Select tool's double-click on a component, the arrow button of the component Info box | the base glyph joins the tab text before the edited glyph and is edited |
//! | `cornerComponent.editGlyph` | the arrow button of the corner Info box | the corner glyph joins the tab text before the edited glyph and is edited |
//! | `glyph.showGlyphsUsingAsComponent` | the canvas context menu | a new tab with every glyph that has a component of the edited glyph |
//!
//! After Component from Selection the new glyph joins the tab text after the edited glyph and is
//! edited ([`after_run`]).

use serde_json::{Value, json};

use fontcraft_engine::model::corner::corner_hint;

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::MAX_TAB_GLYPHS;

pub const COMMANDS: &[UiCommand] = &[
    (
        "component.editBaseGlyph",
        "Edit Base Glyph",
        &[],
        "",
        "{tab?: id, component?: index, glyph?: name} put a base glyph into the tab text just before the edited glyph and edit it: `glyph` names it, else it is the base of component `component` of the edited layer, else of the only selected component → {glyph}",
    ),
    (
        "cornerComponent.editGlyph",
        "Edit Corner Glyph",
        &[],
        "",
        "{tab?: id, glyph?: name (default the edited glyph), layer?: id (default the edited layer), corner: index (in the layer's hint list)} put the glyph of that corner component into the tab text just before the edited glyph and edit it → {glyph}",
    ),
    (
        "glyph.showGlyphsUsingAsComponent",
        "Show all glyphs that use this glyph as a component",
        &[],
        "",
        "{glyph?: name (default the edited glyph)} open a new Edit View tab with every glyph that has a component of the glyph, in font order (glyph.listComposites); a glyph that no other glyph uses is an error → {glyph, glyphs: [names], tab}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "component.editBaseGlyph" => edit_base_glyph(app, args),
        "cornerComponent.editGlyph" => edit_corner_glyph(app, args),
        "glyph.showGlyphsUsingAsComponent" => show_users(app, args),
        _ => Err(args.err("unknown command")),
    }
}

fn edit_base_glyph(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let index = super::view::target_tab(app, args)?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let base = match args.string("glyph")? {
        Some(name) => name.to_string(),
        None => {
            let layer = view::edited_layer(font, tab).ok_or_else(|| args.err("no glyph is being edited"))?;
            let component = |i: usize| match layer.shapes.get(i) {
                Some(fontcraft_engine::model::Shape::Component(c)) => Some(c.glyph.clone()),
                _ => None,
            };
            match args.index("component")? {
                Some(i) => {
                    let i = usize::try_from(i).map_err(|_| args.err("`component` is too large"))?;
                    component(i).ok_or_else(|| args.err(format!("shape {i} is not a component of the edited layer")))?
                }
                None => {
                    let selected: Vec<String> = tab.selection.shapes.iter().filter_map(|&i| component(i)).take(2).collect();
                    match selected.as_slice() {
                        [only] => only.clone(),
                        _ => return Err(args.err("select exactly one component, or give `component` or `glyph`")),
                    }
                }
            }
        }
    };
    if font.glyph(&base).is_none() {
        return Err(args.err(format!("no glyph is named `{base}`")));
    }
    let at = tab.edited_index().unwrap_or(0);
    insert_and_edit(app, index, &base, at)?;
    Ok(json!({"glyph": base}))
}

/// Puts `glyph` into the text of the tab at `index` at position `at` and edits it there. A
/// failure puts the tab back as it was.
fn insert_and_edit(app: &mut FontcraftApp, index: usize, glyph: &str, at: usize) -> Result<(), String> {
    let tab = app.state.tabs.get(index).ok_or("no such tab")?;
    let tab_id = tab.id;
    let before = tab.clone();
    let done = app
        .run("editView.insertText", &json!({"tab": tab_id, "glyphs": [glyph], "at": at}))
        .and_then(|_| app.run("editView.setCursor", &json!({"tab": tab_id, "index": at})));
    if let Err(e) = done {
        // A command that fails changes nothing: the inserted glyph goes again.
        if let Some(tab) = app.state.tabs.get_mut(index) {
            *tab = before;
        }
        if let Some(tab) = app.state.tabs.get(index) {
            app.layouts.prepare(&mut app.session, tab);
        }
        return Err(e);
    }
    Ok(())
}

fn edit_corner_glyph(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let index = super::view::target_tab(app, args)?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let corner = args.index("corner")?.ok_or_else(|| args.err("give `corner`, the corner's index in the layer's hint list"))?;
    let corner = usize::try_from(corner).map_err(|_| args.err("`corner` is too large"))?;
    let glyph = match args.string("glyph")? {
        Some(g) => g.to_string(),
        None => tab.edited_glyph().map(str::to_string).ok_or_else(|| args.err("no glyph is being edited: give `glyph`"))?,
    };
    let g = font.glyph(&glyph).ok_or_else(|| args.err(format!("no glyph is named `{glyph}`")))?;
    let layer = match args.string("layer")? {
        Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer `{id}`")))?,
        None => match view::active_layer(font, tab).filter(|_| tab.edited_glyph() == Some(glyph.as_str())) {
            Some(l) => l,
            None => font.masters.first().and_then(|m| g.layer(&m.id)).ok_or_else(|| args.err(format!("glyph `{glyph}` has no master layer")))?,
        },
    };
    let hint = layer.info.hints.get(corner).ok_or_else(|| args.err(format!("the layer has no hint {corner}")))?;
    let name = match corner_hint(hint) {
        Ok(Some(c)) => c.name,
        Ok(None) => return Err(args.err(format!("hint {corner} is not a corner component"))),
        Err(e) => return Err(args.err(e)),
    };
    if font.glyph(&name).is_none() {
        return Err(args.err(format!("the corner glyph `{name}` does not exist")));
    }
    let at = tab.edited_index().unwrap_or(0);
    insert_and_edit(app, index, &name, at)?;
    Ok(json!({"glyph": name}))
}

fn show_users(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let glyph = match args.string("glyph")? {
        Some(g) => g.to_string(),
        None => {
            app.state.tab().and_then(|t| t.edited_glyph()).map(str::to_string).ok_or_else(|| args.err("no glyph is being edited: give `glyph`"))?
        }
    };
    let listed = app.session.execute("glyph.listComposites", &json!({ "glyph": glyph })).map_err(|e| args.err(e))?;
    let glyphs: Vec<String> = listed
        .get("composites")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if glyphs.is_empty() {
        return Err(args.err(format!("no glyph uses `{glyph}` as a component")));
    }
    let shown: Vec<String> = glyphs.iter().take(MAX_TAB_GLYPHS).cloned().collect();
    app.run("view.newTab", &json!({ "glyphs": shown }))?;
    let tab = app.state.tab().map(|t| t.id);
    Ok(json!({"glyph": glyph, "glyphs": glyphs, "tab": tab}))
}

/// After Component from Selection succeeds while its glyph is edited in the showing tab, the
/// new glyph joins the text right after it and is edited, as the person would continue.
pub fn after_run(app: &mut FontcraftApp, id: &str, result: &Result<Value, String>) {
    if id != "glyph.componentFromSelection" {
        return;
    }
    let Ok(r) = result else { return };
    let (Some(name), Some(glyph)) = (r.get("name").and_then(Value::as_str), r.get("glyph").and_then(Value::as_str)) else { return };
    let Some(index) = app.state.active_tab else { return };
    let Some(tab) = app.state.tabs.get(index) else { return };
    if tab.edited_glyph() != Some(glyph) {
        return;
    }
    let at = tab.edited_index().map_or(0, |i| i + 1);
    if let Err(e) = insert_and_edit(app, index, name, at) {
        app.state.status = e;
    }
}
