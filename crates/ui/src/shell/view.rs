//! Zoom, scrolling, the View menu's display toggles and the edited side of the layer.
//!
//! The zoom value agents see is the size in screen points of 1000 font units (`zoom.valueMeaning`): 1000 is actual
//! size. Positions are in the edited glyph's font units. The canvas's own wheel, pinch and middle-button pan move the
//! same view (`ViewState::canvas`).

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use craft::ui::runtime::commands::CommandRef;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::canvas::Display;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// One zoom step: ×√2.
pub const ZOOM_STEP: f64 = std::f64::consts::SQRT_2;
/// The zoom range in screen points per font unit: zoom values 50 to 64000.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.05..=64.0;
/// Farthest a view centre may lie from the origin, in font units.
pub const MAX_SCROLL: f64 = 10_000_000.0;

const fn command(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
) -> UiCommand {
    UiCommand { id, label, menu, shortcut, params }
}

const TOGGLE: &str = "{on?: bool} (omitted: toggle)";

const COMMANDS: &[UiCommand] = &[
    command("view.zoomIn", "Zoom In", &["View"], Some("Cmd+Plus"), "{} zoom in by one step (×√2)"),
    command("view.zoomOut", "Zoom Out", &["View"], Some("Cmd+Minus"), "{} zoom out by one step (÷√2)"),
    command(
        "view.zoomToActiveLayer",
        "Zoom to Active Layer",
        &["View"],
        Some("Cmd+0"),
        "{} fit the descender-to-ascender span of the edited glyph in the window",
    ),
    command("view.zoomToActualSize", "Zoom to Actual Size", &["View"], Some("Cmd+Alt+0"), "{} one font unit per screen point (zoom 1000)"),
    command(
        "view.setZoom",
        "Set Zoom",
        &[],
        None,
        "{size: number, center?: [x, y]} set the zoom value (screen points per 1000 units, 50 to 64000); `center` is the font point to show at the window's centre",
    ),
    command(
        "view.zoomToRect",
        "Zoom to Rectangle",
        &[],
        None,
        "{rect: [x, y, width, height]} zoom and scroll so this rectangle in font units fills the window",
    ),
    command(
        "view.scrollTo",
        "Scroll Canvas",
        &[],
        None,
        "{x?: number, y?: number, dx?: number, dy?: number} show font point (x, y) at the window's centre, then move the content by (dx, dy) screen points (right and down)",
    ),
    command("view.showNodes", "Show Nodes", &["View"], None, TOGGLE),
    command("view.showNodesInForeground", "In Foreground", &["View", "Show Nodes"], Some("Cmd+Shift+N"), TOGGLE),
    command("view.showNodesInBackground", "In Background", &["View", "Show Nodes"], None, TOGGLE),
    command("view.showExtraNodes", "Extra Nodes", &["View", "Show Nodes"], None, TOGGLE),
    command("view.showMetrics", "Show Metrics", &["View"], Some("Cmd+Shift+M"), TOGGLE),
    command("view.showMetricsNames", "Show Metrics Names", &["View"], None, TOGGLE),
    command("view.showInfo", "Show Info", &["View"], Some("Cmd+Shift+I"), TOGGLE),
    command("view.showBackground", "Show Background", &["View"], Some("Cmd+Shift+B"), TOGGLE),
    command("view.showGuides", "Show Guides", &["View"], Some("Cmd+Shift+L"), TOGGLE),
    command("view.showBoundingBox", "Show Bounding Box", &["View"], Some("Cmd+Alt+Shift+B"), TOGGLE),
    command("view.showAnnotations", "Show Annotations", &["View"], None, TOGGLE),
    command("view.showAnchors", "Show Anchors", &["View"], None, TOGGLE),
    command("view.fillPreview", "Fill Preview", &["View"], None, TOGGLE),
    command(
        "view.showImage",
        "Show Image",
        &["View"],
        None,
        "{on?: bool} draw the images placed on glyph layers; a layer side with no paths and no components shows its image regardless (omitted: toggle)",
    ),
    command(
        "view.showOutlineAtLocation",
        "Show Outline at Location",
        &["View"],
        None,
        "{on?: bool} draw the edited glyph as it interpolates at the location the previews show, behind the edited layer (omitted: toggle)",
    ),
    command(
        "path.editBackground",
        "Edit Background",
        &["Path"],
        Some("Cmd+B"),
        "{on?: bool} edit the background of the layer instead of its foreground (omitted: switch sides)",
    ),
];

/// The display toggle a command flips.
pub(crate) fn flag<'a>(display: &'a mut Display, id: &str) -> Option<&'a mut bool> {
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
        "view.showImage" => &mut display.image,
        "view.showOutlineAtLocation" => &mut display.outline_at_location,
        _ => return None,
    })
}

/// The parameters of one command, with errors that name it.
struct Args<'a> {
    id: &'a str,
    params: &'a Value,
}

impl Args<'_> {
    fn err(&self, message: impl std::fmt::Display) -> String {
        format!("{}: {message}", self.id)
    }

    /// `on` when given (a boolean), else the opposite of `current`.
    fn toggle(&self, current: bool) -> Result<bool, String> {
        match self.params.get("on") {
            None | Some(Value::Null) => Ok(!current),
            Some(Value::Bool(on)) => Ok(*on),
            Some(_) => Err(self.err("`on` must be true or false")),
        }
    }

    fn number(&self, key: &str) -> Result<Option<f64>, String> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| self.err(format!("`{key}` must be a number"))),
        }
    }

    fn numbers<const N: usize>(&self, key: &str) -> Result<Option<[f64; N]>, String> {
        let Some(v) = self.params.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
        let bad = || self.err(format!("`{key}` must be a list of {N} numbers"));
        let list = v.as_array().filter(|a| a.len() == N).ok_or_else(bad)?;
        let mut out = [0.0; N];
        for (slot, item) in out.iter_mut().zip(list) {
            *slot = item.as_f64().filter(|n| n.is_finite()).ok_or_else(bad)?;
        }
        Ok(Some(out))
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let args = Args { id: &command.id, params: &command.params };
    if let Some(current) = flag(&mut app.view.display, &command.id).map(|f| *f) {
        let on = args.toggle(current)?;
        if let Some(f) = flag(&mut app.view.display, &command.id) {
            *f = on;
        }
        return Ok(());
    }
    if command.id == "path.editBackground" {
        let on = args.toggle(app.view.edit_background)?;
        if on != app.view.edit_background {
            app.view.edit_background = on;
            app.view_changed();
        }
        return Ok(());
    }
    let glyph = app.models.canvas.glyph.as_ref().ok_or_else(|| args.err("no glyph is open in the canvas"))?;
    let (top, span, width) = (glyph.ascender, (glyph.descender, glyph.ascender), glyph.width);
    let margin = app.models.canvas_tokens.fit_margin;
    let view = &mut app.view.canvas;
    // Font point `(x, y)` is document point `(x, top - y)` (`canvas::camera`).
    let doc = |x: f64, y: f64| Point::new(scroll_limit(x), scroll_limit(top - y));
    match command.id.as_str() {
        "view.zoomIn" => view.zoom = clamped(view.zoom * ZOOM_STEP),
        "view.zoomOut" => view.zoom = clamped(view.zoom / ZOOM_STEP),
        "view.zoomToActualSize" => view.zoom = 1.0,
        "view.zoomToActiveLayer" => fit(view, [0.0, span.0, width.max(1.0), span.1 - span.0], top, margin),
        "view.setZoom" => {
            let size = args.number("size")?.ok_or_else(|| args.err("`size` (screen points per 1000 units) is required"))?;
            if size <= 0.0 {
                return Err(args.err("`size` must be more than 0"));
            }
            view.zoom = clamped(size / 1000.0);
            if let Some([x, y]) = args.numbers::<2>("center")? {
                view.center = doc(x, y);
            }
        }
        "view.zoomToRect" => {
            let rect = args.numbers::<4>("rect")?.ok_or_else(|| args.err("`rect` ([x, y, width, height]) is required"))?;
            if rect[2] <= 0.0 || rect[3] <= 0.0 {
                return Err(args.err("`rect` must have a width and a height of more than 0"));
            }
            fit(view, rect, top, margin);
        }
        "view.scrollTo" => {
            let (x, y, dx, dy) = (args.number("x")?, args.number("y")?, args.number("dx")?, args.number("dy")?);
            if x.is_none() && y.is_none() && dx.is_none() && dy.is_none() {
                return Err(args.err("give `x` and `y`, or `dx` and `dy`"));
            }
            let z = clamped(view.zoom);
            let cx = x.unwrap_or(view.center.x) - dx.unwrap_or(0.0) / z;
            let cy = y.map_or(view.center.y, |y| top - y) - dy.unwrap_or(0.0) / z;
            view.center = Point::new(scroll_limit(cx), scroll_limit(cy));
        }
        other => return Err(format!("no view command `{other}`")),
    }
    Ok(())
}

fn clamped(zoom: f64) -> f64 {
    if zoom.is_finite() { zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end()) } else { 1.0 }
}

fn scroll_limit(v: f64) -> f64 {
    v.clamp(-MAX_SCROLL, MAX_SCROLL)
}

/// Zooms and scrolls `view` so the font rectangle `[x, y, width, height]` fills its rect, `margin` (a share) free on
/// every side. A view never shown has no rect yet: it keeps its zoom and only centres the rectangle.
pub(crate) fn fit(view: &mut CanvasView, [x, y, w, h]: [f64; 4], top: f64, margin: f64) {
    let room = 1.0 - 2.0 * margin.clamp(0.0, 0.45);
    let (rw, rh) = (f64::from(view.rect.width()), f64::from(view.rect.height()));
    if rw.is_finite() && rh.is_finite() && rw > 0.0 && rh > 0.0 {
        view.zoom = clamped((rw * room / w).min(rh * room / h));
    }
    view.center = Point::new(scroll_limit(x + w / 2.0), scroll_limit(top - (y + h / 2.0)));
}
