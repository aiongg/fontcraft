//! The canvas selection and path focus: `edit.selectAll` and its relatives, `selection.*`, `edit.recallSelection`,
//! `path.focusOnSelectedPath`, `path.clearFocusOnPaths`, the Primitives tool's two modes (`tool.drawRectangle`,
//! `tool.drawCircle`), and `tool.select {tool}`, which makes a named pointer tool current.
//!
//! `tool.select` is also the Select tool's own command: fired without `tool`, from its toolbar button or key, it makes
//! the Select tool current. `FontCraftApp::route` runs it as a tool command whenever `tool` names a tool; only a
//! `tool` that names none reaches [`run`], which refuses it with the list of tools.
//!
//! While Font View shows, Select All, Deselect All and Invert Selection act on its visible glyphs
//! (`FontCraftApp::select_in_font_view`). The other selection commands refuse there, unless they name the glyph whose
//! selection they change, so the selection of the tab behind Font View stays as it is.
//!
//! The selection belongs to the edited glyph's layer (decision 0004: the engine never stores it), so these are shell
//! commands. Each takes the edited glyph as given; `glyph` and `layer`, when given, must name it.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_geom::Point;
use fontcraft_tools::geometry::rect_corners;
use fontcraft_tools::selection::{in_region, invert, select_all, step};
use fontcraft_tools::{MAX_LASSO_POINTS, Scene, SelectMode, Selection};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::{FontCraftApp, Msg, ViewMsg};
use crate::canvas::tools::{apply, scene};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const fn command(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
) -> UiCommand {
    UiCommand { id, label, menu, shortcut, params }
}

const COMMANDS: &[UiCommand] = &[
    command("tool.drawRectangle", "Draw Rectangle", &[], None, "{} make the Primitives tool current in rectangle mode"),
    command("tool.drawCircle", "Draw Circle", &[], None, "{} make the Primitives tool current in ellipse mode"),
    command(
        "tool.select",
        "Select Tool",
        &[],
        None,
        "{tool?: id (tool.list names them)} → {tool, previous}: make the named pointer tool current, as its toolbar button or key does; without `tool`, the Select tool",
    ),
    command(
        "edit.selectAll",
        "Select All",
        &["Edit"],
        Some("Cmd+A"),
        "{} every point of the edited layer; when they are all selected already, its anchors and components too. While Font View shows: every glyph it lists",
    ),
    command("edit.deselectAll", "Deselect All", &["Edit"], Some("Cmd+Alt+A"), "{} empty the selection; while Font View shows, its glyph selection"),
    command(
        "edit.invertSelection",
        "Invert Selection",
        &["Edit"],
        Some("Cmd+Alt+Shift+I"),
        "{} select the points and anchors of the edited layer that are not selected, and nothing else. While Font View shows: the glyphs it lists that are not selected",
    ),
    command(
        "selection.selectNext",
        "Select Next",
        &[],
        Some("Tab"),
        "{} with exactly one point, component, anchor or guide selected: select the next one (points in path order, then components, then anchors, then local and global guides), wrapping around",
    ),
    command(
        "selection.selectPrevious",
        "Select Previous",
        &[],
        Some("Shift+Tab"),
        "{} with exactly one point, component, anchor or guide selected: select the one before it, wrapping around",
    ),
    command(
        "selection.set",
        "Set Selection",
        &[],
        None,
        "{glyph?: name (the edited glyph), layer?: id, mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace), points?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index]} change the selection; an address that does not resolve is an error",
    ),
    command(
        "selection.selectInRegion",
        "Select in Region",
        &[],
        None,
        "{glyph?: name (the edited glyph), layer?: id, rect?: [x, y, width, height], angle?: degrees (turns the rectangle about (x, y)), polygon?: [[x, y]] (3 or more points), nodesOnly?: bool, components?: bool, mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace)} the points and anchors inside the rectangle (a marquee) or the polygon (a lasso); nodesOnly leaves the handles out; components also takes each unlocked component with an on-curve point of its outline inside",
    ),
    command(
        "path.focusOnSelectedPath",
        "Focus on Selected Path",
        &[],
        None,
        "{glyph?: name (the edited glyph), layer?: id, shapes?: [index] (default: the paths with a selected point)} show and select only these paths' nodes, until the focus is cleared or another glyph is edited",
    ),
    command("path.clearFocusOnPaths", "Clear Focus on Paths", &[], None, "{} show and select the nodes of every path again"),
    command(
        "edit.recallSelection",
        "Recall Selection",
        &["Edit", "Selection"],
        None,
        "{name: string, glyph?: name, layer?: id (both must name the edited glyph and layer)} replace the selection with the stored selection `name` of the edited glyph, mapped onto the edited layer: the nodes with the same indexes, the components at the same indexes, the anchors with the same names. What does not resolve there is left out; when nothing resolves the command is refused",
    ),
];

fn label(id: &str) -> &str {
    COMMANDS.iter().find(|c| c.id == id).map_or(id, |c| c.label)
}

/// What a command does to the tools' state.
enum Change {
    Select(Selection),
    Focus(Vec<usize>),
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    match command.id.as_str() {
        "tool.drawRectangle" | "tool.drawCircle" => {
            let tool = if command.id == "tool.drawCircle" { "ellipse" } else { "rectangle" };
            app.apply_msg(Msg::View(ViewMsg::SelectTool(tool)))
        }
        crate::app::catalog::SELECT_TOOL => {
            let tool = crate::app::catalog::chosen_tool("select", command)?;
            app.apply_msg(Msg::View(ViewMsg::SelectTool(tool)))
        }
        "edit.selectAll" | "edit.deselectAll" | "edit.invertSelection" if app.font_view_shows() => app.select_in_font_view(&command.id),
        _ if app.font_view_shows() && params.get("glyph").is_none_or(Value::is_null) => {
            Err(format!("{}: open a glyph first; Font View is in front", label(&command.id)))
        }
        "path.clearFocusOnPaths" => {
            app.tools.focus = None;
            Ok(())
        }
        "edit.recallSelection" => {
            app.tools.selection = recall(app, params)?;
            Ok(())
        }
        id => {
            apply::sync(app);
            let change = {
                let font = app.session.font().map_err(|e| e.to_string())?;
                let scene = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref())
                    .ok_or("no glyph is being edited (open a glyph first)")?;
                check_target(params, &scene)?;
                change(id, params, &scene, &app.tools.selection)?
            };
            match change {
                Change::Select(s) => app.tools.selection = s,
                Change::Focus(shapes) => app.tools.focus = Some(shapes),
            }
            Ok(())
        }
    }
}

/// The stored selection `name` of the edited glyph, as the engine maps it onto the edited layer
/// (`edit.getStoredSelections`).
fn recall(app: &mut FontCraftApp, params: &Value) -> Result<Selection, String> {
    let name = string(params, "name")?.ok_or("`name` is required")?;
    apply::sync(app);
    let query = {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let scene = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref())
            .ok_or("no glyph is being edited (open a glyph first)")?;
        check_target(params, &scene)?;
        serde_json::json!({"glyph": scene.glyph, "layer": scene.layer_id, "side": scene.side.as_str(), "name": name})
    };
    let found = app.session.execute("edit.getStoredSelections", &query).map_err(|e| e.to_string())?;
    let entry =
        found.get("selections").and_then(Value::as_array).and_then(|list| list.first()).ok_or_else(|| format!("no stored selection `{name}`"))?;
    let mut lists = serde_json::Map::new();
    for key in ["points", "shapes", "anchors"] {
        if let Some(v) = entry.get(key).filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
            lists.insert(key.into(), v.clone());
        }
    }
    if lists.is_empty() {
        return Err(format!("nothing of `{name}` resolves on the edited layer"));
    }
    Selection::from_value(&Value::Object(lists))
}

fn string<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| format!("`{key}` must be a string")),
    }
}

fn flag(params: &Value, key: &str) -> Result<bool, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| format!("`{key}` must be true or false")),
    }
}

/// `glyph` and `layer`, when given, must name what the canvas edits.
fn check_target(params: &Value, scene: &Scene) -> Result<(), String> {
    if let Some(glyph) = string(params, "glyph")?
        && glyph != scene.glyph
    {
        return Err(format!("`{glyph}` is not the glyph being edited (`{}`); open it first", scene.glyph));
    }
    if let Some(layer) = string(params, "layer")?
        && layer != scene.layer_id
    {
        return Err(format!("layer `{layer}` is not the layer being edited (`{}`)", scene.layer_id));
    }
    Ok(())
}

fn mode(params: &Value) -> Result<SelectMode, String> {
    match params.get("mode") {
        None | Some(Value::Null) => Ok(SelectMode::Replace),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| "`mode` must be \"replace\", \"add\", \"remove\" or \"toggle\"".to_string()),
    }
}

fn change(id: &str, params: &Value, scene: &Scene, current: &Selection) -> Result<Change, String> {
    let applied = |items: Selection| -> Result<Change, String> {
        let mut s = current.clone();
        s.apply(mode(params)?, items.items());
        Ok(Change::Select(s))
    };
    match id {
        "edit.selectAll" => Ok(Change::Select(select_all(scene, current))),
        "edit.deselectAll" => Ok(Change::Select(Selection::new())),
        "edit.invertSelection" => Ok(Change::Select(invert(scene, current))),
        "selection.selectNext" | "selection.selectPrevious" => step(scene, current, id == "selection.selectNext")
            .map(Change::Select)
            .ok_or_else(|| "select exactly one point, component, anchor or guide first".to_string()),
        "selection.set" => {
            let given = Selection::from_value(params)?;
            let mut valid = given.clone();
            valid.retain_valid(scene);
            if valid != given {
                let missing: Vec<String> = given.items().into_iter().filter(|i| !valid.contains(i)).take(10).map(|i| format!("{i:?}")).collect();
                return Err(format!("these addresses are not in the edited layer: {}", missing.join(", ")));
            }
            applied(given)
        }
        "selection.selectInRegion" => {
            let polygon = region(params)?;
            applied(in_region(scene, &polygon, flag(params, "nodesOnly")?, flag(params, "components")?))
        }
        "path.focusOnSelectedPath" => {
            let mut shapes: Vec<usize> = match params.get("shapes").filter(|v| !v.is_null()) {
                Some(list) => {
                    let bad = || "`shapes` must be a list of shape indices".to_string();
                    let list = list.as_array().ok_or_else(bad)?;
                    let mut out = Vec::new();
                    for v in list.iter().take(scene.layer.shapes.len().saturating_add(1)) {
                        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(bad)?;
                        if scene.path(i).is_none() {
                            return Err(format!("shape {i} is not a path of the edited layer"));
                        }
                        out.push(i);
                    }
                    out
                }
                None => current.points.iter().map(|&(s, _)| s).filter(|&s| scene.path(s).is_some()).collect(),
            };
            shapes.sort_unstable();
            shapes.dedup();
            if shapes.is_empty() {
                return Err("select a point of the paths to focus on first".into());
            }
            Ok(Change::Focus(shapes))
        }
        other => Err(format!("no selection command `{other}`")),
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}

/// The marquee rectangle or lasso polygon of `selection.selectInRegion`, in font units.
fn region(params: &Value) -> Result<Vec<Point>, String> {
    let rect = params.get("rect").filter(|v| !v.is_null());
    let polygon = params.get("polygon").filter(|v| !v.is_null());
    match (rect, polygon) {
        (Some(rect), None) => {
            let bad = || "`rect` must be [x, y, width, height]".to_string();
            let [x, y, w, h] = match rect.as_array().map(Vec::as_slice) {
                Some([x, y, w, h]) => [number(x), number(y), number(w), number(h)],
                _ => return Err(bad()),
            };
            let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else { return Err(bad()) };
            let angle = match params.get("angle") {
                None | Some(Value::Null) => 0.0,
                Some(v) => number(v).ok_or("`angle` must be a number")?,
            };
            Ok(rect_corners(Point::new(x, y), angle.to_radians(), w, h).to_vec())
        }
        (None, Some(list)) => {
            let bad = || "`polygon` must be a list of [x, y] points".to_string();
            let list = list.as_array().ok_or_else(bad)?;
            if list.len() > MAX_LASSO_POINTS {
                return Err(format!("`polygon` holds at most {MAX_LASSO_POINTS} points"));
            }
            let points = list
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([x, y]) => match (number(x), number(y)) {
                        (Some(x), Some(y)) => Ok(Point::new(x, y)),
                        _ => Err(bad()),
                    },
                    _ => Err(bad()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if points.len() < 3 {
                return Err("`polygon` needs 3 or more points".into());
            }
            Ok(points)
        }
        (Some(_), Some(_)) => Err("give `rect` or `polygon`, not both".into()),
        (None, None) => Err("`rect` ([x, y, width, height]) or `polygon` ([[x, y]…]) is required".into()),
    }
}
