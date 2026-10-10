//! The pointer tools and the Edit View selection: `tool.*`, `edit.selectAll` and its relatives,
//! `selection.*`, and path focus.
//!
//! The selection belongs to the showing tab and to its edited glyph's layer in the active master
//! (decision 0004: the engine never stores it). Every command here that changes it returns the
//! new one in the shape command targets take.

use fontcraft_geom::Point;
use fontcraft_tools::geometry::rect_corners;
use fontcraft_tools::selection::{in_region, invert, select_all, step};
use fontcraft_tools::{MAX_LASSO_POINTS, Scene, SelectMode, Selection};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::tools_glue;

pub const COMMANDS: &[UiCommand] = &[
    ("tool.select", "Select Tool", &[], "", "{tool: id} make a pointer tool active; a gesture in progress is cancelled → {tool, previous}"),
    (
        "tool.list",
        "List Tools",
        &[],
        "",
        "{} the pointer tools with their keys and toolbar groups, and the active one → {active, tools: [{id, label, shortcut, group}]}",
    ),
    (
        "edit.selectAll",
        "Select All",
        &["Edit"],
        "Cmd+A",
        "{} Edit View: every point of the edited layer; when they are all selected already, its anchors and components too → {selection}. Font View: every visible glyph → {glyphs}",
    ),
    (
        "edit.deselectAll",
        "Deselect All",
        &["Edit"],
        "Cmd+Alt+A",
        "{} empty the Edit View selection (in Font View, the glyph selection) → {selection}",
    ),
    (
        "edit.invertSelection",
        "Invert Selection",
        &["Edit"],
        "Cmd+Alt+Shift+I",
        "{} select the points and anchors of the edited layer that are not selected, and nothing else → {selection}. Font View: the visible glyphs that are not selected → {glyphs}",
    ),
    (
        "selection.selectNext",
        "Select Next",
        &[],
        "Tab",
        "{} with exactly one point, anchor or guide selected: select the next one (points in path order, then anchors, then local and global guides), wrapping around → {selection}",
    ),
    (
        "selection.selectPrevious",
        "Select Previous",
        &[],
        "Shift+Tab",
        "{} with exactly one point, anchor or guide selected: select the one before it, wrapping around → {selection}",
    ),
    (
        "selection.set",
        "Set Selection",
        &[],
        "",
        "{glyph?: name (the edited glyph), layer?: id, mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace), points?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index]} change the Edit View selection; an address that does not resolve is an error → {selection}",
    ),
    (
        "selection.selectInRegion",
        "Select in Region",
        &[],
        "",
        "{glyph?: name (the edited glyph), layer?: id, rect?: [x, y, width, height], angle?: degrees (turns the rectangle about (x, y)), polygon?: [[x, y]] (3 or more points), nodesOnly?: bool, mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace)} the points and anchors inside the rectangle (a marquee) or the polygon (a lasso); nodesOnly leaves the handles out → {selection}",
    ),
    (
        "path.focusOnSelectedPath",
        "Focus on Selected Path",
        &[],
        "",
        "{glyph?: name (the edited glyph), layer?: id, shapes?: [index] (default: the paths with a selected point)} show and select only these paths' nodes, until the focus is cleared or another glyph is edited → {shapes}",
    ),
    ("path.clearFocusOnPaths", "Clear Focus on Paths", &[], "", "{} show and select the nodes of every path again → {}"),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "tool.select" => {
            let tool = args.string("tool")?.ok_or_else(|| args.err("`tool` (a tool id, see tool.list) is required"))?;
            let switch = app.tools.toolbox.select(tool).map_err(|e| args.err(e))?;
            tools_glue::apply(app, switch.output);
            tools_glue::sync(app);
            Ok(json!({"tool": switch.tool, "previous": switch.previous}))
        }
        "tool.list" => Ok(app.tools.toolbox.list()),
        "edit.selectAll" | "edit.deselectAll" | "edit.invertSelection" if app.state.tab().is_none() => Ok(super::font_view::select(app, id)),
        "path.clearFocusOnPaths" => {
            if let Some(tab) = app.state.tab_mut() {
                tab.focus = None;
            }
            Ok(json!({}))
        }
        _ => edit(app, id, args),
    }
}

/// What a command does to the showing tab.
enum Change {
    Select(Selection),
    Focus(Vec<usize>),
}

fn edit(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    tools_glue::sync(app);
    let (change, reply) = {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let tab = app.state.tab().ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)"))?;
        let scene = tools_glue::shown_scene(&app.session, font, tab).ok_or_else(|| args.err("the edited glyph has no layer in the active master"))?;
        check_target(args, &scene)?;
        let change = change(id, args, &scene, &tab.selection)?;
        let reply = match &change {
            Change::Select(s) => json!({"selection": Value::Object(s.targets(&scene))}),
            Change::Focus(shapes) => json!({"shapes": shapes}),
        };
        (change, reply)
    };
    if let Some(tab) = app.state.tab_mut() {
        match change {
            Change::Select(s) => tab.selection = s,
            Change::Focus(shapes) => tab.focus = Some(shapes),
        }
    }
    Ok(reply)
}

/// `glyph` and `layer`, when given, must name what Edit View edits.
fn check_target(args: &Args, scene: &Scene) -> Result<(), String> {
    if let Some(glyph) = args.string("glyph")?
        && glyph != scene.glyph
    {
        return Err(args.err(format!("`{glyph}` is not the glyph being edited (`{}`); open it first", scene.glyph)));
    }
    if let Some(layer) = args.string("layer")?
        && layer != scene.layer_id
    {
        return Err(args.err(format!("layer `{layer}` is not the layer being edited (`{}`)", scene.layer_id)));
    }
    Ok(())
}

fn mode(args: &Args) -> Result<SelectMode, String> {
    match args.get("mode") {
        None => Ok(SelectMode::Replace),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| args.err("`mode` must be \"replace\", \"add\", \"remove\" or \"toggle\"")),
    }
}

fn change(id: &str, args: &Args, scene: &Scene, current: &Selection) -> Result<Change, String> {
    let applied = |items: Selection| -> Result<Change, String> {
        let mut s = current.clone();
        s.apply(mode(args)?, items.items());
        Ok(Change::Select(s))
    };
    match id {
        "edit.selectAll" => Ok(Change::Select(select_all(scene, current))),
        "edit.deselectAll" => Ok(Change::Select(Selection::new())),
        "edit.invertSelection" => Ok(Change::Select(invert(scene, current))),
        "selection.selectNext" | "selection.selectPrevious" => step(scene, current, id == "selection.selectNext")
            .map(Change::Select)
            .ok_or_else(|| args.err("select exactly one point, anchor or guide first")),
        "selection.set" => {
            let given = Selection::from_value(&args.value()).map_err(|e| args.err(e))?;
            let mut valid = given.clone();
            valid.retain_valid(scene);
            if valid != given {
                let missing: Vec<String> = given.items().into_iter().filter(|i| !valid.contains(i)).take(10).map(|i| format!("{i:?}")).collect();
                return Err(args.err(format!("these addresses are not in the edited layer: {}", missing.join(", "))));
            }
            applied(given)
        }
        "selection.selectInRegion" => {
            let polygon = region(args)?;
            let nodes_only = args.flag("nodesOnly")?.unwrap_or(false);
            applied(in_region(scene, &polygon, nodes_only))
        }
        "path.focusOnSelectedPath" => {
            let shapes: Vec<usize> = match args.get("shapes") {
                Some(list) => {
                    let list = list.as_array().ok_or_else(|| args.err("`shapes` must be a list of shape indices"))?;
                    let count = scene.layer.shapes.len();
                    let mut out = Vec::new();
                    for v in list.iter().take(count.saturating_add(1)) {
                        let i =
                            v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| args.err("`shapes` must be a list of shape indices"))?;
                        if scene.path(i).is_none() {
                            return Err(args.err(format!("shape {i} is not a path of the edited layer")));
                        }
                        out.push(i);
                    }
                    out
                }
                None => current.points.iter().map(|&(s, _)| s).filter(|&s| scene.path(s).is_some()).collect(),
            };
            let mut shapes = shapes;
            shapes.sort_unstable();
            shapes.dedup();
            if shapes.is_empty() {
                return Err(args.err("select a point of the paths to focus on first"));
            }
            Ok(Change::Focus(shapes))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The marquee rectangle or lasso polygon of `selection.selectInRegion`, in font units.
fn region(args: &Args) -> Result<Vec<Point>, String> {
    match (args.numbers::<4>("rect")?, args.get("polygon")) {
        (Some([x, y, w, h]), None) => {
            let angle = args.number("angle")?.unwrap_or(0.0).to_radians();
            Ok(rect_corners(Point::new(x, y), angle, w, h).to_vec())
        }
        (None, Some(list)) => {
            let bad = || args.err("`polygon` must be a list of [x, y] points");
            let list = list.as_array().ok_or_else(bad)?;
            if list.len() > MAX_LASSO_POINTS {
                return Err(args.err(format!("`polygon` holds at most {MAX_LASSO_POINTS} points")));
            }
            let points = list
                .iter()
                .map(|p| match p.as_array().map(Vec::as_slice) {
                    Some([x, y]) => match (x.as_f64(), y.as_f64()) {
                        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Ok(Point::new(x, y)),
                        _ => Err(bad()),
                    },
                    _ => Err(bad()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            if points.len() < 3 {
                return Err(args.err("`polygon` needs 3 or more points"));
            }
            Ok(points)
        }
        (Some(_), Some(_)) => Err(args.err("give `rect` or `polygon`, not both")),
        (None, None) => Err(args.err("`rect` ([x, y, width, height]) or `polygon` ([[x, y]…]) is required")),
    }
}
