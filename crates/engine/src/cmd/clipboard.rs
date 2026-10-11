//! Edit menu: Cut, Copy and Paste; File › Import › Outlines.
//!
//! The engine never touches the system clipboard. Copy and Cut return the clipboard text in two
//! flavours (our JSON fragment and an SVG document for other apps), Paste takes clipboard text in
//! its parameters, and the desktop shell moves the text to and from the system clipboard.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use fontcraft_geom::{Point, Rect, Segment, bounds, round_to_grid};
use fontcraft_io_svg::clipboard::{self, Fragment, FragmentGuide, FragmentShape, GuideScope as FragmentScope};
use fontcraft_io_svg::{Placement, glyph_name_for_file, looks_like_svg, read_svg, shapes_to_svg};
use fontcraft_model::{Component, Font, MAX_COORDINATE, MAX_LAYER_GUIDES, MAX_NAME_LEN, NodeKind, Path, Shape, Transform, metric};
use serde_json::{Value, json};

use super::common::{grid, in_range};
use super::*;
use crate::EngineError;
use crate::params::{GuideRef, GuideScope, Params, TargetKind, Targets};

/// Most files one `file.importOutlines` call reads.
const MAX_IMPORT_FILES: usize = 4096;
/// Most nodes one `file.importOutlines` call adds, over all its files.
const MAX_IMPORT_NODES: usize = 2_000_000;

const COPY_TARGETS: &[TargetKind] =
    &[TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes, TargetKind::Anchors, TargetKind::Guides];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.cut",
            "Cut",
            ["Edit"],
            Some("Cmd+X"),
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: local|global, index}]} copies the targets as edit.copy does, then deletes them with edit.delete, as one undo step. Without a target list it cuts what edit.copy copies. A locked glyph refuses it, and so does `corners`: corner components are copied, not cut → {json: string, svg: string, selection}",
            has_doc,
            cut
        )
        .targets(COPY_TARGETS),
        cmd!(
            query "edit.copy",
            "Copy",
            ["Edit"],
            Some("Cmd+C"),
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: local|global, index}]} the targets as clipboard text, at their own coordinates. A path is copied whole when it is in `shapes` or every on-curve node of it is in `points`; otherwise each run of its selected segments (both end nodes in `points`, or the segment in `segments`) becomes an open path. Without a target list the whole side of the layer is copied with its local guides. Images, annotations and shapes the model keeps unread are not copied. With a Point To Unit Ratio parameter on the master (else the font), the SVG is written in points, that many font units to the point, with its size in pt. With `glyphs: [names]` (and `master?: id`, the master shown) it copies those glyphs whole instead, for edit.paste and edit.pasteSpecial; with `corners: [hint index]` it copies those corner components without their nodes, for edit.paste onto other nodes → {json: our clipboard fragment, svg: an SVG document for other apps, skipped?: shapes left out} | {json: the copied glyphs, glyphs: [names]} | {json: the copied corners, corners: count}",
            has_doc,
            copy
        )
        .targets(COPY_TARGETS),
        cmd!(
            "edit.paste",
            "Paste",
            ["Edit"],
            Some("Cmd+V"),
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", json?: string, svg?: string, text?: string, at?: [x, y], points?: [[shape, node]]} adds clipboard contents to the layer. `json` is our fragment, `svg` a drawing read at one font unit per SVG unit with SVG y = 0 on the baseline, `text` either one, told apart by its content; `json` wins, and `svg` is used when `json` cannot be read. Pasted things keep their coordinates, or with `at` the bottom left of their box lands there. Coordinates are rounded to the font's grid unless its spacing is 0. A pasted anchor replaces the layer's anchor of the same name, global guides go to the layer's master. `outOfBounds` is true when the pasted box lies more than half an em outside the glyph box, and `correction` is the move that puts its bottom left at the origin. A locked glyph refuses it. When `json` or `text` holds copied glyphs (edit.copy with `glyphs`), each is added as a new glyph at its place in the default sort order, with the first free counter suffix (A.001) and no code points when its name is taken, and no `glyph` is needed. Copied corner components (edit.copy with `corners`) go onto the nodes in `points`: one corner onto every node, several onto as many nodes in order; any other selection is refused → {pasted: count, flavour: \"json\" | \"svg\" | \"glyphs\" | \"corners\", glyphs?: [names], corners?: [hint index], renamed?: [{from, to}], outOfBounds: bool, bounds?: [xMin, yMin, xMax, yMax], correction?: [dx, dy], notes?: what the SVG reader left out, selection}",
            has_doc,
            paste
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "file.importOutlines",
            "Outlines…",
            ["File", "Import"],
            None,
            "{files: [paths], glyphs?: [names], layer?: id (default: the first master's layer), onMissingGlyph?: \"add\" | \"skip\" (default skip), scale?: \"unscaled\" | \"em\" (default unscaled)} imports SVG files as outlines, added to the glyph's layer after its shapes, as one undo step. Each file goes to the glyph named by its base name (A.svg into A), or to the glyph at the same position in `glyphs`. A file whose glyph does not exist is listed in `unmatched`, or with add the glyph is created. unscaled reads one font unit per SVG unit with SVG y = 0 at the master's ascender; em scales the drawing's box to the em with its top edge at the ascender. Coordinates are rounded to the grid; styling is dropped and named in `notes`. A file that cannot be read is listed in `failed`; when no file is imported and one failed, the command fails. At most 4096 files and 2,000,000 nodes → {imported: [{file, glyph, paths, notes?}], added: [names], unmatched: [files], failed: [{file, error}]}",
            has_doc,
            import_outlines
        ),
    ]
}

// ── Copy and cut ────────────────────────────────────────────────────────────────────────────────

/// What a copy takes from one layer.
struct Copied {
    /// The copied things as a target object: what a cut deletes.
    targets: Targets,
    /// Shapes left out: images and shapes the model keeps unread.
    skipped: usize,
    json: String,
    svg: String,
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("edit.copy", p)?;
    if p.get("glyphs").is_some() {
        return super::glyph_set::copy_glyphs(s, &p);
    }
    if p.get("corners").is_some() {
        return super::corner::copy_corners(s, &p);
    }
    let c = gather(s.font()?, &p)?;
    let mut out = json!({"json": c.json, "svg": c.svg});
    if c.skipped > 0
        && let Some(o) = out.as_object_mut()
    {
        o.insert("skipped".into(), json!(c.skipped));
    }
    Ok(out)
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("edit.cut", p)?;
    // Without this, `corners` would be ignored and the cut would take the whole layer.
    if p.get("corners").is_some() {
        return Err(p.bad("corner components cannot be cut: copy them with edit.copy, then remove them from their nodes"));
    }
    let c = gather(s.font()?, &p)?;
    if let Ok(r) = c.targets.resolve(s.font()?) {
        crate::check_unlocked(r.glyph)?;
    }
    let revision = s.doc()?.revision;
    let deleted = s.execute("edit.delete", &c.targets.to_json())?;
    // edit.delete made the undo step; it is this command's, so it carries this command's name.
    if s.batch_is_idle()
        && s.doc()?.revision != revision
        && let Some(step) = s.doc_mut()?.history.undo.last_mut()
    {
        step.label = "Cut".into();
    }
    let selection =
        deleted.get("selection").cloned().unwrap_or_else(|| Targets::new("edit.cut", c.targets.glyph.clone(), c.targets.layer.clone()).to_json());
    Ok(json!({"json": c.json, "svg": c.svg, "selection": selection}))
}

/// The targets of `p` as a clipboard fragment and its two flavours.
fn gather(font: &Font, p: &Params) -> Result<Copied> {
    let t = Targets::parse(p)?;
    let r = t.resolve(font)?;
    let parts = r.parts;
    let mut copied = Targets::new(p.command(), t.glyph.clone(), t.layer.clone());
    copied.side = t.side;
    let mut fragment = Fragment::default();
    let mut skipped = 0usize;
    if !t.has_lists() {
        for (i, shape) in parts.shapes.iter().enumerate() {
            match FragmentShape::from_shape(shape) {
                Some(f) => {
                    fragment.shapes.push(f);
                    copied.shapes.push(i);
                }
                None => skipped += 1,
            }
        }
        fragment.anchors = parts.anchors.to_vec();
        copied.anchors = parts.anchors.iter().map(|a| a.name.clone()).collect();
        fragment.guides = parts.guides.iter().map(|g| FragmentGuide { scope: FragmentScope::Local, guide: g.clone() }).collect();
        copied.guides = (0..parts.guides.len()).map(|index| GuideRef { scope: GuideScope::Local, index }).collect();
    } else {
        let whole: BTreeSet<usize> = t.shapes.iter().copied().collect();
        let mut nodes: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &(shape, node) in t.points.iter().filter(|(shape, _)| !whole.contains(shape)) {
            nodes.entry(shape).or_default().insert(node);
        }
        let mut ends: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &(shape, node) in t.segments.iter().filter(|(shape, _)| !whole.contains(shape)) {
            ends.entry(shape).or_default().insert(node);
        }
        let partial: BTreeSet<usize> = nodes.keys().chain(ends.keys()).copied().collect();
        let none = BTreeSet::new();
        for &i in whole.union(&partial) {
            let Some(shape) = parts.shapes.get(i) else { continue };
            if whole.contains(&i) {
                match FragmentShape::from_shape(shape) {
                    Some(f) => fragment.shapes.push(f),
                    None => skipped += 1,
                }
            } else if let Shape::Path(path) = shape {
                let pieces = selected_pieces(path, nodes.get(&i).unwrap_or(&none), ends.get(&i).unwrap_or(&none));
                fragment.shapes.extend(pieces.into_iter().map(FragmentShape::Path));
            }
        }
        let names: HashSet<&str> = t.anchors.iter().map(String::as_str).collect();
        fragment.anchors = parts.anchors.iter().filter(|a| names.contains(a.name.as_str())).cloned().collect();
        let mut guides = t.guides.clone();
        guides.sort_by_key(|g| (g.scope == GuideScope::Global, g.index));
        for g in &guides {
            let (scope, guide) = match g.scope {
                GuideScope::Local => (FragmentScope::Local, parts.guides.get(g.index)),
                GuideScope::Global => (FragmentScope::Global, r.master.guides.get(g.index)),
            };
            if let Some(guide) = guide {
                fragment.guides.push(FragmentGuide { scope, guide: guide.clone() });
            }
        }
        copied.points = t.points.iter().copied().filter(|(shape, _)| !whole.contains(shape)).collect();
        copied.segments = t.segments.iter().copied().filter(|(shape, _)| !whole.contains(shape)).collect();
        copied.shapes = whole.iter().copied().filter(|i| matches!(parts.shapes.get(*i), Some(Shape::Path(_) | Shape::Component(_)))).collect();
        copied.anchors = t.anchors.clone();
        copied.guides = t.guides.clone();
    }
    if fragment.is_empty() {
        return Err(p.bad("nothing to copy: the targets hold no shape, complete segment, anchor or guide"));
    }
    let json = clipboard::encode(&fragment).map_err(|e| p.bad(e.to_string()))?;
    let shapes: Vec<Shape> = fragment.shapes.iter().cloned().map(FragmentShape::into_shape).collect();
    let master = r.layer.master.as_str();
    let svg = shapes_to_svg(&shapes, &|c: &Component| font.component_outline(c, master).ok(), None).map_err(|e| p.bad(e.to_string()))?;
    let svg = super::clipboard_scale::scale_copy(font, master, &shapes, svg).map_err(|e| p.bad(e.to_string()))?;
    Ok(Copied { targets: copied, skipped, json, svg })
}

/// The selected part of `path`: the whole path when every on-curve node is in `nodes` or every
/// segment is selected, else one open path per run of selected segments. A segment is selected
/// when both its end nodes are in `nodes`, or its end node is in `ends`.
fn selected_pieces(path: &Path, nodes: &BTreeSet<usize>, ends: &BTreeSet<usize>) -> Vec<Path> {
    let mut on_curve = path.nodes.iter().enumerate().filter(|(_, n)| n.on_curve()).map(|(i, _)| i).peekable();
    if on_curve.peek().is_some() && on_curve.all(|i| nodes.contains(&i)) {
        return vec![path.clone()];
    }
    let segs = path.segments();
    let picked: Vec<bool> = segs.iter().map(|s| ends.contains(&s.end) || (nodes.contains(&s.start) && nodes.contains(&s.end))).collect();
    if !picked.is_empty() && picked.iter().all(|b| *b) {
        return vec![path.clone()];
    }
    let count = segs.len();
    // In a closed path, start after an unselected segment so that no run is split at the seam.
    let first = if path.closed { picked.iter().position(|b| !*b).map_or(0, |u| u + 1) } else { 0 };
    let mut out = Vec::new();
    let mut run: Vec<&Segment> = Vec::new();
    for k in 0..count {
        let at = (first + k) % count;
        let (Some(seg), Some(&on)) = (segs.get(at), picked.get(at)) else { continue };
        let joins = run.last().is_some_and(|last| last.end == seg.start);
        if !(on && joins) {
            out.extend(open_path(path, &run));
            run.clear();
        }
        if on {
            run.push(seg);
        }
    }
    out.extend(open_path(path, &run));
    out
}

/// A run of consecutive segments of `path` as an open path with the path's attributes.
fn open_path(path: &Path, run: &[&Segment]) -> Option<Path> {
    let first = run.first()?;
    let mut start = path.nodes.get(first.start)?.clone();
    start.kind = NodeKind::Line;
    start.smooth = false;
    let mut nodes = vec![start];
    for seg in run {
        for &h in &seg.handles {
            nodes.push(path.nodes.get(h)?.clone());
        }
        nodes.push(path.nodes.get(seg.end)?.clone());
    }
    if let Some(last) = nodes.last_mut() {
        last.smooth = false;
    }
    Some(Path { closed: false, nodes, info: path.info.clone(), extra: path.extra.clone() })
}

// ── Paste ───────────────────────────────────────────────────────────────────────────────────────

/// The pasted contents, the flavour they came in, and what the SVG reader left out.
type Incoming = (Fragment, &'static str, Option<String>);

/// Reads the clipboard text in `json`, `svg` or `text`.
fn incoming(p: &Params) -> Result<Incoming> {
    let from_json = |text: &str| -> Result<Incoming> {
        let f = clipboard::decode(text).map_err(|e| p.bad(format!("the clipboard fragment cannot be pasted: {e}")))?;
        Ok((f, "json", None))
    };
    let from_svg = |text: &str| -> Result<Incoming> {
        let read = read_svg(text.as_bytes(), Placement::Unscaled { top: 0.0 }).map_err(|e| p.bad(format!("the SVG cannot be pasted: {e}")))?;
        let shapes = read.paths.into_iter().map(FragmentShape::Path).collect();
        Ok((Fragment { shapes, ..Fragment::default() }, "svg", read.dropped.summary()))
    };
    match (p.opt_str("json")?, p.opt_str("svg")?, p.opt_str("text")?) {
        (Some(j), svg, _) => match (from_json(j), svg) {
            (Err(_), Some(svg)) => from_svg(svg),
            (r, _) => r,
        },
        (None, Some(svg), _) => from_svg(svg),
        (None, None, Some(text)) if clipboard::looks_like_fragment(text) => from_json(text),
        (None, None, Some(text)) if looks_like_svg(text) => from_svg(text),
        (None, None, Some(_)) => Err(p.bad("the text is neither a clipboard fragment nor an SVG document")),
        (None, None, None) => Err(p.bad("give the clipboard text as `json`, `svg` or `text`")),
    }
}

/// An `[x, y]` parameter: finite numbers within the model's coordinate range.
fn opt_point(p: &Params, key: &str) -> Result<Option<Point>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let bad = || p.bad(format!("`{key}` must be [x, y], two numbers within ±{MAX_COORDINATE}"));
    let Value::Array(a) = v else { return Err(bad()) };
    let [x, y] = a.as_slice() else { return Err(bad()) };
    let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else { return Err(bad()) };
    if !(in_range(x) && in_range(y)) {
        return Err(bad());
    }
    Ok(Some(Point::new(x, y)))
}

/// The box around what a fragment places: nodes, component origins and anchors, or the guides'
/// positions when it holds nothing else.
fn content_bounds(f: &Fragment) -> Option<Rect> {
    let mut points = Vec::new();
    for shape in &f.shapes {
        match shape {
            FragmentShape::Path(path) => points.extend(path.nodes.iter().map(|n| n.point())),
            FragmentShape::Component(c) => {
                let [.., dx, dy] = c.affine().as_coeffs();
                points.push(Point::new(dx, dy));
            }
        }
    }
    points.extend(f.anchors.iter().map(|a| Point::new(a.x, a.y)));
    if points.is_empty() {
        points.extend(f.guides.iter().map(|g| Point::new(g.guide.x, g.guide.y)));
    }
    bounds(points)
}

/// Moves everything in `f` by `(dx, dy)` and rounds it to the grid.
fn place(f: &mut Fragment, dx: f64, dy: f64, step: f64) {
    let r = |v: f64, d: f64| round_to_grid(v + d, step);
    for shape in &mut f.shapes {
        match shape {
            FragmentShape::Path(path) => {
                for n in &mut path.nodes {
                    n.x = r(n.x, dx);
                    n.y = r(n.y, dy);
                }
            }
            FragmentShape::Component(c) => match &mut c.transform {
                Transform::Decomposed { pos, .. } => *pos = (r(pos.0, dx), r(pos.1, dy)),
                Transform::Matrix([.., x, y]) => {
                    *x = r(*x, dx);
                    *y = r(*y, dy);
                }
            },
        }
    }
    for a in &mut f.anchors {
        a.x = r(a.x, dx);
        a.y = r(a.y, dy);
    }
    for g in &mut f.guides {
        g.guide.x = r(g.guide.x, dx);
        g.guide.y = r(g.guide.y, dy);
    }
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("edit.paste", p)?;
    if let Some(text) = super::glyph_set::glyphs_text(&p)? {
        return super::glyph_set::paste_glyphs(s, &p, text);
    }
    if let Some(pasted) = super::corner::paste_onto_nodes(s, &p)? {
        return Ok(pasted);
    }
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it adds to the layer"));
    }
    let at = opt_point(&p, "at")?;
    let (mut fragment, flavour, notes) = incoming(&p)?;
    if fragment.is_empty() {
        return Err(p.bad("nothing to paste: the clipboard text holds no outline, anchor or guide"));
    }
    if fragment.shapes.iter().any(|f| matches!(f, FragmentShape::Component(c) if c.glyph == t.glyph)) {
        return Err(p.bad(format!("a component of `{0}` cannot be pasted into `{0}` itself", t.glyph)));
    }

    let font = s.font()?;
    let r = t.resolve(font)?;
    crate::check_unlocked(r.glyph)?;
    let (layer_id, master_id, width) = (r.layer.id.clone(), r.layer.master.clone(), r.layer.width);
    let em = f64::from(font.units_per_em);
    let ascender = font.metric_pos(r.master, metric::ASCENDER, em * 0.8);
    let descender = font.metric_pos(r.master, metric::DESCENDER, -em * 0.2);
    let step = grid(font);

    let (dx, dy) = match (at, content_bounds(&fragment)) {
        (Some(at), Some(b)) => (at.x - b.min_x(), at.y - b.min_y()),
        _ => (0.0, 0.0),
    };
    place(&mut fragment, dx, dy, step);
    let placed = content_bounds(&fragment);
    let margin = em / 2.0;
    let glyph_box = Rect::new(-margin, descender.min(ascender) - margin, width.max(0.0) + margin, ascender.max(descender) + margin);
    let out_of_bounds = placed.is_some_and(|b| {
        b.min_x() > glyph_box.max_x() || b.max_x() < glyph_box.min_x() || b.min_y() > glyph_box.max_y() || b.max_y() < glyph_box.min_y()
    });

    let shape_count = fragment.shapes.len();
    let anchor_names: Vec<String> = fragment.anchors.iter().map(|a| a.name.clone()).collect();
    let pasted = shape_count + fragment.anchors.len() + fragment.guides.len();
    let (local, global): (Vec<FragmentGuide>, Vec<FragmentGuide>) = fragment.guides.into_iter().partition(|g| g.scope == FragmentScope::Local);
    let (shapes, anchors) = (fragment.shapes, fragment.anchors);
    t.layer = Some(layer_id);
    let side = t.side;
    let (first_shape, guide_refs) = s.edit("Paste", |font| {
        let (first, mut refs) = t.edit_layer(font, |layer| {
            let parts = layer.parts_mut(side);
            let first = parts.shapes.len();
            parts.shapes.extend(shapes.into_iter().map(FragmentShape::into_shape));
            for a in anchors {
                match parts.anchors.iter_mut().find(|x| x.name == a.name) {
                    Some(existing) => *existing = a,
                    None => parts.anchors.push(a),
                }
            }
            if parts.guides.len().saturating_add(local.len()) > MAX_LAYER_GUIDES {
                return Err(EngineError::BadParams {
                    command: "edit.paste".into(),
                    message: format!("the layer would have more than {MAX_LAYER_GUIDES} guides"),
                });
            }
            let refs: Vec<GuideRef> =
                (parts.guides.len()..parts.guides.len() + local.len()).map(|index| GuideRef { scope: GuideScope::Local, index }).collect();
            parts.guides.extend(local.into_iter().map(|g| g.guide));
            Ok((first, refs))
        })?;
        if !global.is_empty() {
            let masters = Arc::make_mut(&mut font.masters);
            let master =
                masters.iter_mut().find(|m| m.id == master_id).ok_or_else(|| EngineError::Internal(format!("master `{master_id}` vanished")))?;
            if master.guides.len().saturating_add(global.len()) > MAX_LAYER_GUIDES {
                return Err(EngineError::BadParams {
                    command: "edit.paste".into(),
                    message: format!("the master would have more than {MAX_LAYER_GUIDES} guides"),
                });
            }
            let start = master.guides.len();
            refs.extend((start..start + global.len()).map(|index| GuideRef { scope: GuideScope::Global, index }));
            master.guides.extend(global.into_iter().map(|g| g.guide));
        }
        Ok((first, refs))
    })?;

    t.shapes = (first_shape..first_shape + shape_count).collect();
    t.anchors = anchor_names;
    t.guides = guide_refs;
    let mut out = json!({"pasted": pasted, "flavour": flavour, "outOfBounds": out_of_bounds, "selection": t.to_json()});
    if let Some(o) = out.as_object_mut() {
        if let Some(b) = placed {
            o.insert("bounds".into(), json!([b.min_x(), b.min_y(), b.max_x(), b.max_y()]));
            if out_of_bounds {
                o.insert("correction".into(), json!([round_to_grid(-b.min_x(), step), round_to_grid(-b.min_y(), step)]));
            }
        }
        if let Some(n) = notes {
            o.insert("notes".into(), json!(n));
        }
    }
    Ok(out)
}

// ── Import outlines ─────────────────────────────────────────────────────────────────────────────

/// One file read for import, before the font changes.
struct ImportJob {
    file: String,
    glyph: String,
    /// The glyph does not exist yet and is created.
    new: bool,
    paths: Vec<Path>,
    notes: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bytes(path: &str) -> Result<Vec<u8>> {
    crate::io::read_file(std::path::Path::new(path))
}

#[cfg(target_arch = "wasm32")]
fn read_bytes(_: &str) -> Result<Vec<u8>> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

/// Why `name` cannot be a new glyph's name, by the model's rule.
fn name_problem(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("is empty")
    } else if name.len() > MAX_NAME_LEN {
        Some("is longer than 255 bytes")
    } else if name.chars().any(char::is_control) {
        Some("contains a control character")
    } else {
        None
    }
}

fn import_outlines(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("file.importOutlines", p)?;
    let files = p.strings("files")?;
    if files.is_empty() {
        return Err(p.bad("`files` is empty"));
    }
    if files.len() > MAX_IMPORT_FILES {
        return Err(p.bad(format!("`files` holds {} paths; at most {MAX_IMPORT_FILES} are allowed", files.len())));
    }
    let names = p.opt_strings("glyphs")?;
    if let Some(n) = &names
        && n.len() != files.len()
    {
        return Err(p.bad(format!("`glyphs` names {} glyphs for {} files: give one glyph per file", n.len(), files.len())));
    }
    let add = match p.opt_str("onMissingGlyph")? {
        None | Some("skip") => false,
        Some("add") => true,
        Some(other) => return Err(p.bad(format!("`onMissingGlyph` must be \"add\" or \"skip\", got \"{other}\""))),
    };
    let fit_em = match p.opt_str("scale")? {
        None | Some("unscaled") => false,
        Some("em") => true,
        Some(other) => return Err(p.bad(format!("`scale` must be \"unscaled\" or \"em\", got \"{other}\""))),
    };
    let font = s.font()?;
    let layer_id = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let step = grid(font);
    let em = f64::from(font.units_per_em);

    let mut jobs: Vec<ImportJob> = Vec::new();
    let mut unmatched: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut total_nodes = 0usize;
    for (i, &file) in files.iter().enumerate() {
        let name = match &names {
            Some(n) => n.get(i).copied().unwrap_or_default(),
            None => glyph_name_for_file(file),
        };
        let (master_id, new) = match font.glyph(name) {
            Some(g) if g.info.locked => {
                failed.push((file.into(), format!("glyph `{name}` is locked")));
                continue;
            }
            Some(g) => match g.layers.iter().find(|l| l.id == layer_id) {
                Some(l) => (l.master.clone(), false),
                None => {
                    failed.push((file.into(), format!("glyph `{name}` has no layer `{layer_id}`")));
                    continue;
                }
            },
            None if !add => {
                unmatched.push(file.into());
                continue;
            }
            None => {
                if let Some(problem) = name_problem(name) {
                    failed.push((file.into(), format!("a glyph cannot be added for it: its name {problem}")));
                    continue;
                }
                match font.master(&layer_id) {
                    Ok(m) => (m.id.clone(), true),
                    Err(_) => {
                        failed.push((
                            file.into(),
                            format!("glyph `{name}` would be new, and a new glyph has master layers only; `{layer_id}` is not a master"),
                        ));
                        continue;
                    }
                }
            }
        };
        let master = font.master(&master_id)?;
        let ascender = font.metric_pos(master, metric::ASCENDER, em * 0.8);
        let placement = if fit_em { Placement::FitEm { units_per_em: em, ascender } } else { Placement::Unscaled { top: ascender } };
        let read = match read_bytes(file).and_then(|bytes| read_svg(&bytes, placement).map_err(|e| EngineError::Io(e.to_string()))) {
            Ok(read) => read,
            Err(e) => {
                failed.push((file.into(), e.to_string()));
                continue;
            }
        };
        if read.paths.is_empty() {
            failed.push((file.into(), "the file has no outlines".into()));
            continue;
        }
        total_nodes = read.paths.iter().fold(total_nodes, |sum, path| sum.saturating_add(path.nodes.len()));
        if total_nodes > MAX_IMPORT_NODES {
            return Err(p.bad(format!("the files hold more than {MAX_IMPORT_NODES} nodes together: import fewer at a time")));
        }
        let mut paths = read.paths;
        for n in paths.iter_mut().flat_map(|path| path.nodes.iter_mut()) {
            n.x = round_to_grid(n.x, step);
            n.y = round_to_grid(n.y, step);
        }
        jobs.push(ImportJob { file: file.into(), glyph: name.into(), new, paths, notes: read.dropped.summary() });
    }

    let failed_json: Vec<Value> = failed.iter().map(|(file, error)| json!({"file": file, "error": error})).collect();
    if jobs.is_empty() {
        if let Some((file, error)) = failed.first() {
            let more = if failed.len() > 1 { format!(" ({} more files failed)", failed.len() - 1) } else { String::new() };
            return Err(p.bad(format!("nothing was imported: {file}: {error}{more}")));
        }
        return Ok(json!({"imported": [], "added": [], "unmatched": unmatched, "failed": failed_json}));
    }

    let mut added: Vec<String> = Vec::new();
    let data = s.glyph_data().ok();
    let imported = s.edit("Import Outlines", |font| {
        let mut out = Vec::with_capacity(jobs.len());
        for job in jobs {
            if job.new && font.glyph(&job.glyph).is_none() {
                let layers = super::widths::new_glyph_layers(font, data.as_deref(), &job.glyph, &[]);
                font.add_glyph(&job.glyph, Vec::new(), super::widths::BUILT_IN_LAYER_WIDTH)?;
                font.glyph_mut(&job.glyph)?.layers = layers;
                added.push(job.glyph.clone());
            }
            let count = job.paths.len();
            let t = Targets::new("file.importOutlines", job.glyph.clone(), Some(layer_id.clone()));
            t.edit_layer(font, |layer| {
                layer.shapes.extend(job.paths.into_iter().map(Shape::Path));
                Ok(())
            })?;
            let mut entry = json!({"file": job.file, "glyph": job.glyph, "paths": count});
            if let (Some(n), Some(o)) = (job.notes, entry.as_object_mut()) {
                o.insert("notes".into(), json!(n));
            }
            out.push(entry);
        }
        Ok(out)
    })?;
    Ok(json!({"imported": imported, "added": added, "unmatched": unmatched, "failed": failed_json}))
}
