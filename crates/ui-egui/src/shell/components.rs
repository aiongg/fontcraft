//! Moving from a composite to its base glyph in Edit View (`component.editBaseGlyph`): the Select
//! tool's double-click on a component, and the arrow button of the component Info box.

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;

pub const COMMANDS: &[UiCommand] = &[(
    "component.editBaseGlyph",
    "Edit Base Glyph",
    &[],
    "",
    "{tab?: id, component?: index, glyph?: name} put a base glyph into the tab text just before the edited glyph and edit it: `glyph` names it, else it is the base of component `component` of the edited layer, else of the only selected component → {glyph}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "component.editBaseGlyph" {
        return Err(args.err("unknown command"));
    }
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
    let tab_id = tab.id;
    app.run("editView.insertText", &json!({"tab": tab_id, "glyphs": [base], "at": at}))?;
    app.run("editView.setCursor", &json!({"tab": tab_id, "index": at}))?;
    Ok(json!({"glyph": base}))
}
