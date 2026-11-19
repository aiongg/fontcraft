//! `audit.revealIssue`, `audit.nextIssue` and `audit.previousIssue`: open what a result is about.

use std::sync::Arc;

use craft::ui::runtime::commands::CommandRef;
use fontcraft_model::{Layer, Shape};
use serde_json::{Map, Value, json};

use super::run::{Item, Run};
use super::{check_catalogue, current_glyph, quoted, string};
use crate::app::FontCraftApp;

/// Smallest side of the area a reveal zooms to, in font units.
const MIN_REVEAL_SPAN: f64 = 200.0;
/// The area revealed around a result's bounds, as a multiple of their size.
const REVEAL_MARGIN: f64 = 1.5;

/// Runs a shell or engine command at once, as a menu item would.
fn run(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
    app.apply_msg(FontCraftApp::route(CommandRef::with_params(id, params)))
}

/// The panel's run: its own, else the document's last `audit.run`, which becomes the panel's.
fn ensure_run(app: &mut FontCraftApp) -> Result<Arc<Run>, String> {
    if let Some(run) = &app.view.font_audit.run {
        return Ok(Arc::clone(run));
    }
    let doc = app.session.active().map(|d| (d.id, d.revision)).ok_or("no font is open")?;
    let report = app
        .session
        .query("audit.results", &json!({"includeIgnored": true}))
        .map_err(|_| "no audit has run on this font: press Run in the Font Audit panel, or run audit.run".to_string())?;
    let mut params = Map::new();
    if let Some(p) = report.get("profile").and_then(Value::as_str) {
        params.insert("profile".into(), json!(p));
    }
    params.insert("includeIgnored".into(), json!(true));
    let checks = check_catalogue(app);
    let run = Arc::new(Run::read(report, doc, "font".into(), Value::Object(params), true, checks));
    app.view.font_audit.run = Some(Arc::clone(&run));
    Ok(run)
}

/// The Font Info tab that holds `field` (`master.…`, `instance.…`, `font.…`).
fn font_info_tab(field: &str) -> &'static str {
    match field.split('.').next().unwrap_or("") {
        "master" | "masters" => "masters",
        "instance" | "instances" => "exports",
        "feature" | "features" => "features",
        _ => "font",
    }
}

/// The area `[x, y, width, height]` that shows a result's spot with room around it.
fn reveal_rect(item: &Item) -> Option<[f64; 4]> {
    let [x0, y0, x1, y1] = item.bounds()?;
    let w = ((x1 - x0).abs() * REVEAL_MARGIN).max(MIN_REVEAL_SPAN);
    let h = ((y1 - y0).abs() * REVEAL_MARGIN).max(MIN_REVEAL_SPAN);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let rect = [cx - w / 2.0, cy - h / 2.0, w, h];
    rect.iter().all(|v| v.is_finite()).then_some(rect)
}

/// The `selection.set` addresses of what a result reports on `layer`: its nodes, the end node of its segments, and
/// its shapes (a path's every node when the result names no node). `None` when nothing resolves.
fn selection(layer: &Layer, item: &Item) -> Option<(Vec<[usize; 2]>, Vec<usize>)> {
    let nodes = |s: usize| match layer.shapes.get(s) {
        Some(Shape::Path(p)) => Some(p.nodes.len()),
        _ => None,
    };
    let named: Vec<[usize; 2]> = item.addresses("points").into_iter().chain(item.addresses("segments")).collect();
    let mut points: Vec<[usize; 2]> = named.iter().copied().filter(|&[s, n]| nodes(s).is_some_and(|len| n < len)).collect();
    let mut shapes = Vec::new();
    for s in item.shapes() {
        match (layer.shapes.get(s), nodes(s)) {
            (Some(Shape::Path(_)), Some(len)) if named.is_empty() => points.extend((0..len).map(|n| [s, n])),
            (Some(Shape::Path(_)), _) | (None, _) => {}
            (Some(_), _) => shapes.push(s),
        }
    }
    points.sort_unstable();
    points.dedup();
    (!points.is_empty() || !shapes.is_empty()).then_some((points, shapes))
}

fn reveal_glyph(app: &mut FontCraftApp, index: usize, item: &Item, glyph: &str) -> Result<Value, String> {
    let doc_id = app.session.documents().get(index).map(|d| d.id).ok_or("the font of this result is no longer open")?;
    if app.session.active().map(|d| d.id) != Some(doc_id) {
        run(app, "document.activate", json!({"index": index}))?;
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let g = font.glyph(glyph).ok_or_else(|| format!("the font has no glyph `{}` any more", quoted(glyph)))?;
    let layer = item.layer.as_ref().and_then(|l| g.layers.iter().find(|x| &x.id == l)).or_else(|| {
        let master = item.master.as_ref()?;
        g.layers.iter().find(|x| x.is_master_layer() && &x.master == master)
    });
    let layer_id = layer.map(|l| l.id.clone());
    let picked = layer.filter(|_| !item.stale).and_then(|l| selection(l, item));
    run(app, "view.editSelectedGlyphs", json!({"glyphs": [glyph]}))?;
    if let Some(layer) = &layer_id {
        run(app, "layers.select", json!({"glyphs": [glyph], "layer": layer}))?;
        if let Some((points, shapes)) = &picked {
            run(app, "selection.set", json!({"glyph": glyph, "layer": layer, "points": points, "shapes": shapes}))?;
        }
        if !item.stale
            && let Some(rect) = reveal_rect(item)
        {
            run(app, "view.zoomToRect", json!({"rect": rect}))?;
        }
    }
    Ok(json!({"revealed": "glyph", "glyph": glyph, "layer": layer_id}))
}

/// Reveals result `id` of the panel's run.
pub fn reveal(app: &mut FontCraftApp, p: &Map<String, Value>, id: &str) -> Result<(), String> {
    let run = ensure_run(app)?;
    if let Some(asked) = string(p, "runId")?
        && run.run_id.as_deref() != Some(asked.as_str())
    {
        let listed = run.run_id.as_deref().unwrap_or("none");
        return Err(format!("the Font Audit panel lists run `{}`, not `{}`", quoted(listed), quoted(&asked)));
    }
    let item = run.item(id).cloned().ok_or_else(|| format!("the Font Audit panel's last run has no result `{}`", quoted(id)))?;
    let index = app.session.documents().iter().position(|d| d.id == run.doc);
    let family = !item.fonts.is_empty() || !run.kept;
    match (&item.glyph, &item.source_field) {
        (Some(glyph), _) if !family => {
            let index = index.ok_or("the font of this result is no longer open")?;
            reveal_glyph(app, index, &item, glyph)?;
        }
        (_, Some(field)) if !family => {
            let index = index.ok_or("the font of this result is no longer open")?;
            if app.session.active().map(|d| d.id) != Some(run.doc) {
                run_index(app, index)?;
            }
            run_cmd(app, "font.info", json!({"tab": font_info_tab(field)}))?;
        }
        _ if family => {
            let ids: Vec<&String> = if item.fonts.is_empty() { item.font.iter().collect() } else { item.fonts.iter().collect() };
            let paths: Vec<&str> = ids.iter().filter_map(|f| run.fonts.iter().find(|(id, _)| id == *f).and_then(|(_, p)| p.as_deref())).collect();
            let first = app
                .session
                .documents()
                .iter()
                .position(|d| d.path.as_ref().is_some_and(|p| paths.iter().any(|q| std::path::Path::new(q) == p.as_path())))
                .ok_or("none of the fonts this result names is open")?;
            run_index(app, first)?;
        }
        _ => return Err("the result names no glyph and no Font Info field: its value is made when the font is compiled".into()),
    }
    let st = &mut app.view.font_audit;
    st.current = Some(id.to_string());
    st.selected = vec![id.to_string()];
    Ok(())
}

fn run_index(app: &mut FontCraftApp, index: usize) -> Result<(), String> {
    run(app, "document.activate", json!({"index": index}))
}

fn run_cmd(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
    run(app, id, params)
}

/// Reveals the listed result after (`forward`) or before the current one, wrapping.
pub fn step(app: &mut FontCraftApp, p: &Map<String, Value>, forward: bool) -> Result<(), String> {
    let glyph_scope = match string(p, "scope")?.as_deref() {
        None | Some("font") => false,
        Some("glyph") => true,
        Some(other) => return Err(format!("`scope` must be \"font\" or \"glyph\", got \"{}\"", quoted(other))),
    };
    let run = ensure_run(app)?;
    let glyph = current_glyph(app);
    let listed = super::run::listing(&run, &app.view.font_audit.filter, glyph.as_deref());
    let current = app.view.font_audit.current.clone();
    let scope_glyph = if glyph_scope { glyph.or_else(|| current.as_deref().and_then(|c| run.item(c)).and_then(|i| i.glyph.clone())) } else { None };
    let order: Vec<&Item> =
        listed.order().filter_map(|i| run.items.get(i)).filter(|it| !glyph_scope || (scope_glyph.is_some() && it.glyph == scope_glyph)).collect();
    let n = order.len();
    if n == 0 {
        return Err("no listed audit result to step to".into());
    }
    let at = current.as_deref().and_then(|c| order.iter().position(|it| it.id == c));
    let next = match (at, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    let id = order.get(next).map(|it| it.id.clone()).ok_or("no listed audit result to step to")?;
    reveal(app, p, &id)
}
