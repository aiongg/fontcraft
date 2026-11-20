//! The canvas overlays' switches and the Annotation tool's shell commands: the View menu's display aids (Tunni lines,
//! curve tension, indexes, curve length, details across glyphs, Lock Outline, the metrics beam), the snapping
//! suggestions, the Measure items, the audit switches, Select All for annotations and the annotation sub-tools.
//! The drawing is `canvas::overlays`.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::MAX_COORDINATE;
use fontcraft_tools::Selection;
use fontcraft_tools::annotate::{SUB_TOOLS, sub_tool_id};
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::canvas::overlays::{Indexes, Switches};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most documents whose beam height is kept; past it, the oldest document's height is dropped.
const KEPT_BEAM_HEIGHTS: usize = 256;

const fn command(id: &'static str, label: &'static str, menu: &'static [&'static str], params: &'static str) -> UiCommand {
    UiCommand { id, label, menu, shortcut: None, params }
}

const COMMANDS: &[UiCommand] = &[
    command(
        "view.showTunniLines",
        "Tunni Lines",
        &["View"],
        "{on?: bool} draw, for every curve segment of the edited layer, a dashed line joining the ends of its two handles and a dot where the lines through its handles meet (omitted: toggle)",
    ),
    command(
        "view.showCurveTension",
        "Show Curve Tension",
        &["View", "Show"],
        "{on?: bool} print beside each handle of every curve segment its tension: the handle's length as a percentage of the distance from its node to the point where the lines through both handles meet (omitted: toggle)",
    ),
    command(
        "view.showIndexes",
        "Show Indexes",
        &["View"],
        "{points?: bool, segments?: bool, contours?: bool, components?: bool, anchors?: bool} label nodes and segments with their index in their path, paths and components with their shape index, and anchors with their index in the layer. The kinds given are set and the others kept; with none given, all five turn off when any is on, else all turn on",
    ),
    command(
        "view.showCurveLength",
        "Show Curve Length",
        &["View"],
        "{on?: bool} print the arc length of every curve segment beside it, in font units (omitted: toggle)",
    ),
    command(
        "view.showDetailsAcrossGlyphs",
        "Details Across Glyphs",
        &["View"],
        "{on?: bool} draw nodes, handles and the display aids that are on for every glyph on the edited glyph's line, not only for the edited glyph (omitted: toggle)",
    ),
    command(
        "view.lockOutline",
        "Lock Outline",
        &["View", "Lock"],
        "{on?: bool} refuse pointer and key edits of paths, nodes and components on the canvas while guides, anchors and metrics stay editable, with a badge on the canvas; commands run by name still edit the outline (omitted: toggle)",
    ),
    command(
        "view.showMetricsBeam",
        "Metrics Beam",
        &["View"],
        "{on?: bool, y?: number} show a horizontal line at height `y` (font units, kept per document; by default half the x-height) with the sidebearings measured where it meets the edited glyph's outline; giving `y` without `on` also shows it (omitted: toggle)",
    ),
    command(
        "view.suggestHandles",
        "Suggest Handles",
        &["View", "Suggest"],
        "{on?: bool} while one handle is dragged, snap it flat, upright, parallel to the segment's other handle or square to it (omitted: toggle)",
    ),
    command(
        "view.suggestLines",
        "Suggest Lines",
        &["View", "Suggest"],
        "{on?: bool} while nodes are dragged, snap to the continuations of nearby straight segments and handles, perpendiculars of diagonal guides, and centrelines between facing edges (omitted: toggle)",
    ),
    command(
        "view.suggestDistance",
        "Suggest Distance",
        &["View", "Suggest"],
        "{on?: bool} while nodes are dragged, snap to outlines parallel to the other segments at the master's suggestDistance (omitted: toggle)",
    ),
    command(
        "view.suggestStems",
        "Suggest Stems",
        &["View", "Suggest"],
        "{on?: bool} while a straight edge is dragged, snap it one of the master's declared stems away from a facing edge (omitted: toggle)",
    ),
    command(
        "view.showStemWidths",
        "Show Stem Widths",
        &["View", "Measure"],
        "{on?: bool} label each stem of the edited layer from 20 to 250 units wide with its width, coloured by how it compares with the master's stem values: one of them, near one (with a dot beside the label), or neither (omitted: toggle)",
    ),
    command(
        "view.showCounterWidths",
        "Show Counter Widths",
        &["View", "Measure"],
        "{on?: bool} label the white space between neighbouring stems of the edited layer with its width or height (omitted: toggle)",
    ),
    command(
        "view.showCornerAngles",
        "Show Corner Angles",
        &["View", "Measure"],
        "{on?: bool} show the angle at each sharp node of the edited layer's paths once the view is zoomed in far enough for the labels (omitted: toggle)",
    ),
    command(
        "view.showQuickMeasurement",
        "Quick Measurement",
        &["View", "Measure"],
        "{on?: bool} while on, hovering over the edited layer shows the distance from the nearest segment through the pointer to the segment opposite; Shift measures horizontally or vertically (omitted: toggle)",
    ),
    command(
        "audit.toggleLiveAudit",
        "Live Audit",
        &["View", "Audit"],
        "{on?: bool} re-run the outlines profile on the edited layer after each change to the document while it is on (omitted: toggle)",
    ),
    command(
        "audit.toggleMarkers",
        "Show Audit Markers",
        &[],
        "{on?: bool} mark the nodes, segments and paths of the current audit results of the edited layer on the canvas (omitted: toggle)",
    ),
    command(
        "annotation.selectAll",
        "Select All Annotations",
        &[],
        "{glyph?: name (the edited glyph), layer?: id} select every annotation of the edited layer and nothing else; the Annotation tool runs it for Cmd+A",
    ),
    command(
        "annotation.setTool",
        "Choose Annotation Sub-tool",
        &[],
        "{tool: \"select\" | \"text\" | \"arrow\" | \"circle\" | \"plus\" | \"minus\"} make that annotation sub-tool the active tool; a gesture in progress is cancelled",
    ),
];

/// The parameters of one command, with errors that name it.
struct Args<'a> {
    id: &'a str,
    params: &'a Value,
}

impl Args<'_> {
    fn err(&self, message: impl std::fmt::Display) -> String {
        format!("{}: {message}", self.id)
    }

    fn flag(&self, key: &str) -> Result<Option<bool>, String> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(on)) => Ok(Some(*on)),
            Some(_) => Err(self.err(format!("`{key}` must be true or false"))),
        }
    }

    /// `on` when given, else the opposite of `current`.
    fn toggle(&self, current: bool) -> Result<bool, String> {
        Ok(self.flag("on")?.unwrap_or(!current))
    }

    fn string(&self, key: &str) -> Result<Option<&str>, String> {
        match self.params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(self.err(format!("`{key}` must be a string"))),
        }
    }
}

/// The plain switch a command flips.
fn switch<'a>(sw: &'a mut Switches, id: &str) -> Option<&'a mut bool> {
    if let Some(flag) = sw.suggest.get_mut(id) {
        return Some(flag);
    }
    Some(match id {
        "view.showTunniLines" => &mut sw.tunni_lines,
        "view.showCurveTension" => &mut sw.curve_tension,
        "view.showCurveLength" => &mut sw.curve_length,
        "view.showDetailsAcrossGlyphs" => &mut sw.details_across_glyphs,
        "view.lockOutline" => &mut sw.lock_outline,
        "view.showStemWidths" => &mut sw.stem_widths,
        "view.showCounterWidths" => &mut sw.counter_widths,
        "view.showCornerAngles" => &mut sw.corner_angles,
        "view.showQuickMeasurement" => &mut sw.quick_measurement,
        "audit.toggleLiveAudit" => &mut sw.live_audit,
        "audit.toggleMarkers" => &mut sw.audit_markers,
        _ => return None,
    })
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let args = Args { id: &command.id, params: &command.params };
    if let Some(current) = switch(&mut app.overlays.switches, &command.id).map(|f| *f) {
        let on = args.toggle(current)?;
        if let Some(flag) = switch(&mut app.overlays.switches, &command.id) {
            *flag = on;
        }
        return Ok(());
    }
    match command.id.as_str() {
        "view.showIndexes" => indexes(&args, &mut app.overlays.switches.indexes),
        "view.showMetricsBeam" => beam(app, &args),
        "annotation.setTool" => {
            let names: Vec<&str> = SUB_TOOLS.iter().map(|(n, _, _)| *n).collect();
            let name = args.string("tool")?.ok_or_else(|| args.err(format!("`tool` is required: one of {}", names.join(", "))))?;
            let tool = sub_tool_id(name).ok_or_else(|| args.err(format!("unknown sub-tool `{name}`; the sub-tools are {}", names.join(", "))))?;
            app.view.tool = tool;
            crate::canvas::tools::apply::select_tool(app, tool)
        }
        "annotation.selectAll" => {
            crate::canvas::tools::apply::sync(app);
            let target = crate::canvas::tools::target(&app.session, &app.view).ok_or_else(|| args.err("no glyph is being edited"))?;
            if let Some(glyph) = args.string("glyph")?
                && glyph != target.glyph
            {
                return Err(args.err(format!("`{glyph}` is not the glyph being edited (`{}`); open it first", target.glyph)));
            }
            if let Some(layer) = args.string("layer")?
                && layer != target.layer
            {
                return Err(args.err(format!("layer `{layer}` is not the layer being edited (`{}`)", target.layer)));
            }
            let font = app.session.font().map_err(|e| e.to_string())?;
            let layer = font.glyph(&target.glyph).and_then(|g| g.layer(&target.layer)).ok_or_else(|| args.err("the edited layer is missing"))?;
            let mut selection = Selection::new();
            selection.annotations.extend(0..layer.parts(target.side).annotations.len());
            app.tools.selection = selection;
            Ok(())
        }
        other => Err(format!("no overlay command `{other}`")),
    }
}

fn indexes(args: &Args<'_>, shown: &mut Indexes) -> Result<(), String> {
    let mut next = *shown;
    let mut given = false;
    for (key, slot) in [
        ("points", &mut next.points),
        ("segments", &mut next.segments),
        ("contours", &mut next.contours),
        ("components", &mut next.components),
        ("anchors", &mut next.anchors),
    ] {
        if let Some(on) = args.flag(key)? {
            *slot = on;
            given = true;
        }
    }
    *shown = if given { next } else { Indexes::all(!shown.any()) };
    Ok(())
}

fn beam(app: &mut FontCraftApp, args: &Args<'_>) -> Result<(), String> {
    let y = match args.params.get("y") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| args.err("`y` must be a number"))?),
    };
    if y.is_some_and(|y| y.abs() > MAX_COORDINATE) {
        return Err(args.err(format!("`y` must lie within ±{MAX_COORDINATE}")));
    }
    let doc = app.session.active().map(|d| d.id);
    if y.is_some() && doc.is_none() {
        return Err(args.err("no font is open"));
    }
    let sw = &mut app.overlays.switches;
    let on = if args.params.get("on").is_none() && y.is_some() { true } else { args.toggle(sw.beam)? };
    if let (Some(y), Some(doc)) = (y, doc) {
        if sw.beam_heights.len() >= KEPT_BEAM_HEIGHTS && !sw.beam_heights.contains_key(&doc) {
            sw.beam_heights.pop_first();
        }
        sw.beam_heights.insert(doc, y);
    }
    sw.beam = on;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod live_audit_tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::menu::Node;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};

    /// View › Audit › Live Audit fires `audit.toggleLiveAudit`, which switches live audit on and off.
    #[test]
    fn the_view_menu_has_the_live_audit_switch() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let view = app.catalog.menu.menus.iter().find(|m| m.title == "View").expect("the View menu");
        let audit = view
            .children
            .iter()
            .find_map(|n| match n {
                Node::Submenu { label, children, .. } if label == "Audit" => Some(children),
                _ => None,
            })
            .expect("the Audit submenu");
        let row = audit
            .iter()
            .find_map(|n| match n {
                Node::Item(item) if item.command.id == "audit.toggleLiveAudit" => Some(item.label.clone()),
                _ => None,
            })
            .expect("the Live Audit row");
        assert_eq!(row, "Live Audit");

        app.fire(CommandRef::new("audit.toggleLiveAudit"));
        app.drain();
        assert!(app.overlays.switches.live_audit, "the row toggles live audit on");
        app.sent.push(Msg::Shell(CommandRef::with_params("audit.toggleLiveAudit", json!({"on": false}))));
        app.drain();
        assert!(!app.overlays.switches.live_audit);
    }
}
