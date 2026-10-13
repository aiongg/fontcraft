//! Zoom, scrolling and the View menu's display toggles.
//!
//! The zoom value agents see is the size in screen points of 1000 font units (`zoom.valueMeaning`):
//! 1000 is actual size. Positions are in the edited glyph's font units.

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::state::{DisplayOptions, MAX_SCROLL, TabView, ZOOM_STEP};

pub const COMMANDS: &[UiCommand] = &[
    ("view.zoomIn", "Zoom In", &["View"], "Cmd+Plus", "{tab?: id} zoom in by one step (×√2)"),
    ("view.zoomOut", "Zoom Out", &["View"], "Cmd+Minus", "{tab?: id} zoom out by one step (÷√2)"),
    (
        "view.zoomToActiveLayer",
        "Zoom to Active Layer",
        &["View"],
        "Cmd+0",
        "{tab?: id} fit the descender-to-ascender span of the edited glyph in the window",
    ),
    ("view.zoomToActualSize", "Zoom to Actual Size", &["View"], "Cmd+Alt+0", "{tab?: id} one font unit per screen point (zoom 1000)"),
    (
        "view.setZoom",
        "Set Zoom",
        &[],
        "",
        "{tab?: id, size: number, center?: [x, y]} set the zoom value (screen points per 1000 units, 50 to 64000); `center` is the font point to show at the window's centre",
    ),
    (
        "view.zoomToRect",
        "Zoom to Rectangle",
        &[],
        "",
        "{tab?: id, rect: [x, y, width, height]} zoom and scroll so this rectangle in font units fills the window",
    ),
    (
        "view.scrollTo",
        "Scroll Canvas",
        &[],
        "",
        "{tab?: id, x?: number, y?: number, dx?: number, dy?: number} show font point (x, y) at the window's centre, then move the content by (dx, dy) screen points (right and down)",
    ),
    ("view.showNodes", "Show Nodes", &["View"], "", "{on?: bool} show nodes and handles of the edited layer (omitted: toggle)"),
    (
        "view.showNodesInForeground",
        "In Foreground",
        &["View", "Show Nodes"],
        "Cmd+Shift+N",
        "{on?: bool} show nodes and handles of the edited layer (omitted: toggle)",
    ),
    (
        "view.showNodesInBackground",
        "In Background",
        &["View", "Show Nodes"],
        "",
        "{on?: bool} show the background layer's nodes and handles (omitted: toggle)",
    ),
    ("view.showExtraNodes", "Extra Nodes", &["View", "Show Nodes"], "", "{on?: bool} show Extra Nodes (omitted: toggle)"),
    ("view.showMetrics", "Show Metrics", &["View"], "Cmd+Shift+M", "{on?: bool} show metric lines and alignment zones (omitted: toggle)"),
    ("view.showMetricsNames", "Show Metrics Names", &["View"], "", "{on?: bool} label the metric lines (omitted: toggle)"),
    ("view.showInfo", "Show Info", &["View"], "Cmd+Shift+I", "{on?: bool} show the Info box and the Palette's contextual controls (omitted: toggle)"),
    (
        "view.showBackground",
        "Show Background",
        &["View"],
        "Cmd+Shift+B",
        "{on?: bool} draw the other side of the layer as an outline behind it (omitted: toggle)",
    ),
    ("view.showGuides", "Show Guides", &["View"], "Cmd+Shift+L", "{on?: bool} show local and global guides (omitted: toggle)"),
    (
        "view.showBoundingBox",
        "Show Bounding Box",
        &["View"],
        "Cmd+Alt+Shift+B",
        "{on?: bool} show the selection's bounding box with its knobs (omitted: toggle)",
    ),
    ("view.showAnnotations", "Show Annotations", &["View"], "", "{on?: bool} show annotations (omitted: toggle)"),
    ("view.showAnchors", "Show Anchors", &["View"], "", "{on?: bool} show the edited layer's anchors (omitted: toggle)"),
    (
        "view.fillPreview",
        "Fill Preview",
        &["View"],
        "",
        "{on?: bool} fill closed paths; the edited glyph only while the Text, Hand or Zoom tool is active (omitted: toggle)",
    ),
    (
        "view.getDisplayOptions",
        "Get Display Options",
        &[],
        "",
        "{tab?: id} every display toggle, the zoom value, the font point at the window's centre, the writing direction and the preview features of a tab",
    ),
];

/// The display toggle a command flips.
fn flag<'a>(display: &'a mut DisplayOptions, id: &str) -> Option<&'a mut bool> {
    Some(match id {
        "view.showNodes" | "view.showNodesInForeground" => &mut display.nodes,
        "view.showNodesInBackground" => &mut display.nodes_in_background,
        "view.showExtraNodes" => &mut display.extra_nodes,
        "view.showMetrics" => &mut display.metrics,
        "view.showMetricsNames" => &mut display.metrics_names,
        "view.showInfo" => &mut display.info,
        "view.showBackground" => &mut display.background,
        "view.showGuides" => &mut display.guides,
        "view.showBoundingBox" => &mut display.bounding_box,
        "view.showAnnotations" => &mut display.annotations,
        "view.showAnchors" => &mut display.anchors,
        "view.fillPreview" => &mut display.fill_preview,
        _ => return None,
    })
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if let Some(current) = flag(&mut app.state.display, id).map(|f| *f) {
        let on = args.toggle(current)?;
        if let Some(f) = flag(&mut app.state.display, id) {
            *f = on;
        }
        return Ok(json!({"on": on}));
    }
    let index = target_tab(app, args)?;
    if id == "view.getDisplayOptions" {
        return display_options(app, index);
    }
    app.session.font().map_err(|e| e.to_string())?;
    let canvas = app.state.canvas_size;
    let margin = f64::from(app.tokens.fit_margin);
    let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?;
    let [offset, offset_y] = view::edited_origin(&app.session, &app.layouts, tab);
    let mut v = tab.view;
    match id {
        "view.zoomIn" => v.scale = TabView::clamped_scale(v.scale * ZOOM_STEP),
        "view.zoomOut" => v.scale = TabView::clamped_scale(v.scale / ZOOM_STEP),
        "view.zoomToActualSize" => v.scale = 1.0,
        "view.zoomToActiveLayer" => v = view::fit(&app.session, &app.layouts, tab, canvas, margin),
        "view.setZoom" => {
            let size = args.number("size")?.ok_or_else(|| args.err("`size` (screen points per 1000 units) is required"))?;
            if size <= 0.0 {
                return Err(args.err("`size` must be more than 0"));
            }
            v.scale = TabView::clamped_scale(size / 1000.0);
            if let Some([x, y]) = args.numbers::<2>("center")? {
                v.center = [scroll_limit(offset + x), scroll_limit(offset_y + y)];
            }
        }
        "view.zoomToRect" => {
            let [x, y, w, h] = args.numbers::<4>("rect")?.ok_or_else(|| args.err("`rect` ([x, y, width, height]) is required"))?;
            if w <= 0.0 || h <= 0.0 {
                return Err(args.err("`rect` must have a width and a height of more than 0"));
            }
            v = view::zoom_to_rect([offset + x, offset_y + y, w, h], canvas, margin);
            v.center = [scroll_limit(v.center[0]), scroll_limit(v.center[1])];
        }
        "view.scrollTo" => {
            let (x, y, dx, dy) = (args.number("x")?, args.number("y")?, args.number("dx")?, args.number("dy")?);
            if x.is_none() && y.is_none() && dx.is_none() && dy.is_none() {
                return Err(args.err("give `x` and `y`, or `dx` and `dy`"));
            }
            let s = TabView::clamped_scale(v.scale);
            let cx = x.map_or(v.center[0], |x| offset + x) - dx.unwrap_or(0.0) / s;
            let cy = y.map_or(v.center[1], |y| offset_y + y) + dy.unwrap_or(0.0) / s;
            v.center = [scroll_limit(cx), scroll_limit(cy)];
        }
        other => return Err(format!("unknown command `{other}`")),
    }
    if let Some(tab) = app.state.tabs.get_mut(index) {
        tab.view = v;
    }
    Ok(view_reply(app, index))
}

fn scroll_limit(v: f64) -> f64 {
    v.clamp(-MAX_SCROLL, MAX_SCROLL)
}

/// The tab a view command acts on: `tab` by id, else the showing tab.
pub(crate) fn target_tab(app: &FontcraftApp, args: &Args) -> Result<usize, String> {
    match args.index("tab")? {
        Some(id) => app.state.tab_index(id).ok_or_else(|| args.err(format!("no tab has id {id}"))),
        None => app.state.active_tab.ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)")),
    }
}

/// A tab's camera as commands return it: zoom value and centre in the edited glyph's units.
fn view_reply(app: &FontcraftApp, index: usize) -> Value {
    let Some(tab) = app.state.tabs.get(index) else { return json!({}) };
    let [x, y] = view::edited_origin(&app.session, &app.layouts, tab);
    json!({
        "tab": tab.id,
        "zoom": tab.view.scale * 1000.0,
        "center": [tab.view.center[0] - x, tab.view.center[1] - y],
    })
}

fn display_options(app: &FontcraftApp, index: usize) -> Result<Value, String> {
    let mut out = serde_json::to_value(app.state.display).map_err(|e| e.to_string())?;
    if let (Some(map), Value::Object(view)) = (out.as_object_mut(), view_reply(app, index)) {
        map.extend(view);
        map.insert("palette".into(), json!(app.state.show_palette));
        let direction = app.state.tabs.get(index).map_or("ltr", |t| t.direction.name());
        map.insert("direction".into(), json!(direction));
        map.insert("previewFeatures".into(), json!([]));
    }
    Ok(out)
}
