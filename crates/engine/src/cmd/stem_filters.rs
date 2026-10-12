//! The stem filters (M13.24), on `fontcraft_pathops::stems`: `filter.adjustStems`,
//! `filter.changeWeight`, `filter.condenseExtend`, `filter.changeXHeight`
//! (`fontcraft_pathops::stem_filters` has what each one moves).
//!
//! Like the other filters they change the unlocked paths of whole layers: one `glyph` or a
//! `glyphs` list, the first master's layer unless `layer` or `layers` names others. With
//! `glyphs`, a locked glyph, a missing layer or a layer the filter leaves as it is goes to
//! `skipped`; with one `glyph`, a locked glyph or a missing layer is an error. Components stay.
//! A run over many glyphs is one undo step, and moved points land on the font's grid.
//!
//! The metrics a filter reads (zones, x-height, ascender, italic angle) are those of the layer's
//! master.

use std::collections::{HashMap, HashSet};

use fontcraft_geom::Point;
use fontcraft_model::{Layer, Metrics, Node, NodeKind, Path, Shape};
use fontcraft_pathops::stem_filters::{self as sf, AdjustStems, ChangeWeight, ChangeXHeight, CondenseExtend, CounterMode, StemRule, WeightMethod};
use fontcraft_pathops::{Contour, Node as PNode, NodeKind as PKind};
use serde_json::{Map, json};

use super::common::{each_layer, grid, in_range, master_metrics, scope};
use super::*;
use crate::EngineError;
use crate::params::Params;

const LAYER_TARGETS: &[TargetKind] = &[TargetKind::Glyphs, TargetKind::Layer];

/// Most stem-detection work (segment visits of ink tests) one filter command does across all its
/// glyphs and layers. Past it the command fails and changes nothing.
pub const MAX_STEM_FILTER_WORK: usize = 1_000_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "filter.adjustStems",
            "Adjust Stems",
            ["Filter"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], vertical?: [{from: number, to: number, set?: number, offset?: number}], horizontal?: [{…the same}]} changes the width of each vertical stem (width measured across) and horizontal stem (width measured up) whose width lies from `from` to `to`: to `set` units, or by `offset` units (one of the two). The first rule of the stem's direction that holds its width applies; at most 64 rules each. Both edges move by half the change, except that a horizontal stem's edge on the baseline, x-height, cap height, ascender or descender stays and the other edge takes the whole change. Diagonal stems stay → {glyphs: [changed], skipped: [{glyph, layer?, reason}], stems: number of stems changed}",
            has_doc,
            adjust
        )
        .targets(LAYER_TARGETS),
        cmd!(
            "filter.changeWeight",
            "Change Weight…",
            ["Filter"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], amount: units (how much thicker every stem gets; negative thins, within ±1000), method?: \"auto\" (default: cjk for CJK, Hangul and kana characters, else lcg) | \"lcg\" (keep the zones: letter heights stay) | \"cjk\" (only expand) | \"custom\" (lcg with the given zones), zones?: {bottom: y, top: y, serifHeight?: units} (default: the baseline, the x-height for lowercase and the cap height for the rest, serif height 0), counters?: \"auto\" (default: retain with lcg, squish with cjk) | \"squish\" (counters shrink, the width stays) | \"retain\" (counters keep their widths, the glyph gets wider), cleanup?: bool (remove overlaps after, default false)} thickens or thins the outline → {glyphs: [changed], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            change_weight
        )
        .targets(LAYER_TARGETS),
        cmd!(
            "filter.condenseExtend",
            "Condense / Extend…",
            ["Filter"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], counters?: {scale?: percent (default 100), add?: units (default 0)}, sidebearings?: {scale?: percent, add?: units}} narrows or widens the outline while the vertical stems keep their widths: the spaces between and beside the stems scale, each counter between two stems also takes `add`, and both sidebearings scale and take their own `add`; the advance width follows. A slanted master is unslanted by its italic angle first and reslanted after. Scales above 0% and at most 1000% → {glyphs: [changed], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            condense_extend
        )
        .targets(LAYER_TARGETS),
        cmd!(
            "filter.changeXHeight",
            "Change x-Height…",
            ["Filter"],
            None,
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], from?: y (default: the master's x-height), to: y, serifHeight?: units (default 0)} moves the x-height from `from` to `to` by stretching the outline between the baseline and the x-height. Within the serif height of the baseline nothing moves; within it of the x-height the outline shifts rigidly; between the x-height and the master's ascender the outline stretches so the ascender stays → {glyphs: [changed], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            change_x_height
        )
        .targets(LAYER_TARGETS),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

/// What a stem filter reads of a layer.
struct Cx<'a> {
    glyph: &'a str,
    metrics: Metrics,
    unicode: Option<u32>,
    width: f64,
    grid: f64,
}

/// What a stem filter made of a layer's paths.
struct Change {
    contours: Vec<Contour>,
    width_change: f64,
    /// The node lists differ from the input's (overlaps were removed).
    rebuilt: bool,
    stems: usize,
}

impl Change {
    fn moved(contours: Vec<Contour>) -> Self {
        Self { contours, width_change: 0.0, rebuilt: false, stems: 0 }
    }
}

/// Runs `op` on the unlocked paths of every layer the parameters name, as one edit. `op` returns
/// `None` when it leaves a layer as it is. All layers share one work budget, which `op` draws
/// from; when it runs out the command fails, naming where it stopped, and changes nothing.
fn run_layers(s: &mut Session, p: &Params, label: &str, mut op: impl FnMut(&Cx, &[Contour], &mut usize) -> Result<Option<Change>>) -> Result<Value> {
    let font = s.font()?;
    let scope = scope(p, font)?;
    let metrics: HashMap<String, Metrics> = font.masters.iter().map(|m| (m.id.clone(), master_metrics(font, m))).collect();
    let fallback = font.masters.first().map(|m| master_metrics(font, m)).unwrap_or_default();
    let unicodes: HashMap<String, Option<u32>> =
        scope.glyphs.iter().map(|g| (g.clone(), font.glyph(g).and_then(|g| g.unicodes.first().copied()))).collect();
    let step = grid(font);
    let mut stems = 0usize;
    let mut work = MAX_STEM_FILTER_WORK;
    let (mut done, mut current) = (0usize, String::new());
    let out = s.edit(label, |font| {
        each_layer(font, &scope, p.command(), |name, layer| {
            if current != name {
                if !current.is_empty() {
                    done += 1;
                }
                name.clone_into(&mut current);
            }
            let cx = Cx {
                glyph: name,
                metrics: metrics.get(&layer.master).copied().unwrap_or(fallback),
                unicode: unicodes.get(name).copied().flatten(),
                width: layer.width,
                grid: step,
            };
            let (contours, slots, sources) = layer_contours(layer);
            if contours.is_empty() {
                return Ok(Some("the layer has no unlocked paths".into()));
            }
            let change = op(&cx, &contours, &mut work)?;
            if work == 0 {
                return Err(p.bad(format!(
                    "the filter reached its work limit at glyph `{name}` (layer `{}`) after {done} whole glyphs; nothing was changed: run it on fewer glyphs at a time",
                    layer.id
                )));
            }
            let Some(change) = change else { return Ok(Some("no stem matched".into())) };
            if change.contours == contours && change.width_change == 0.0 {
                return Ok(Some("nothing changed".into()));
            }
            write(p, layer, &slots, &sources, &change)?;
            stems += change.stems;
            Ok(None)
        })
    })?;
    let mut v = out.to_json();
    if p.command() == "filter.adjustStems"
        && let Some(o) = v.as_object_mut()
    {
        o.insert("stems".into(), json!(stems));
    }
    Ok(v)
}

/// The unlocked paths of `layer` as contours, the shape index of each, and the (shape, node)
/// address of every node; a node's tag is its index in that list.
fn layer_contours(layer: &Layer) -> (Vec<Contour>, Vec<usize>, Vec<(usize, usize)>) {
    let (mut contours, mut slots, mut sources) = (Vec::new(), Vec::new(), Vec::new());
    for (si, shape) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        if path.info.locked {
            continue;
        }
        let nodes = path
            .nodes
            .iter()
            .enumerate()
            .map(|(ni, n)| {
                sources.push((si, ni));
                PNode { point: Point::new(n.x, n.y), kind: pkind(n.kind), smooth: n.smooth, tag: Some(sources.len() - 1) }
            })
            .collect();
        contours.push(Contour::new(path.closed, nodes));
        slots.push(si);
    }
    (contours, slots, sources)
}

/// Writes `change` into `layer`: node by node, or, for rebuilt contours, as new paths in place of
/// the old ones (each keeping the attributes of the path its first surviving node came from, and
/// surviving nodes their names and user data).
fn write(p: &Params, layer: &mut Layer, slots: &[usize], sources: &[(usize, usize)], change: &Change) -> Result<()> {
    let width = layer.width + change.width_change;
    if !(in_range(width) && width >= 0.0) {
        return Err(p.bad(format!("glyph layer `{}` would get the advance width {width}", layer.id)));
    }
    for n in change.contours.iter().flat_map(|c| &c.nodes) {
        let (x, y) = (n.point.x, n.point.y);
        if !(in_range(x) && in_range(y)) {
            return Err(p.bad(format!("the result has a point ({x}, {y}) outside the coordinate range")));
        }
    }
    layer.width = width;
    if !change.rebuilt {
        for (&slot, contour) in slots.iter().zip(&change.contours) {
            let Some(Shape::Path(path)) = layer.shapes.get_mut(slot) else {
                return Err(EngineError::Internal("a filtered path is gone".into()));
            };
            if path.nodes.len() != contour.nodes.len() {
                return Err(EngineError::Internal("a stem filter changed a path's node count".into()));
            }
            for (node, new) in path.nodes.iter_mut().zip(&contour.nodes) {
                node.x = new.point.x;
                node.y = new.point.y;
            }
        }
        return Ok(());
    }
    let old = |shapes: &[Shape], s: usize| match shapes.get(s) {
        Some(Shape::Path(p)) => Some(p.clone()),
        _ => None,
    };
    let mut claimed = HashSet::new();
    let mut paths = Vec::with_capacity(change.contours.len());
    for c in &change.contours {
        let first = c.nodes.iter().find_map(|n| n.tag).and_then(|t| sources.get(t)).map(|(s, _)| *s);
        let mut path = Path::new(c.closed, Vec::with_capacity(c.nodes.len()));
        if let Some(src) = first.filter(|s| claimed.insert(*s)).and_then(|s| old(&layer.shapes, s)) {
            path.info = src.info.clone();
            path.extra = src.extra.clone();
        }
        for n in &c.nodes {
            let source = n.tag.and_then(|t| sources.get(t)).and_then(|&(s, i)| match layer.shapes.get(s) {
                Some(Shape::Path(p)) => p.nodes.get(i).cloned(),
                _ => None,
            });
            let mut node = source.unwrap_or_else(|| Node::new(n.point.x, n.point.y, mkind(n.kind)));
            node.x = n.point.x;
            node.y = n.point.y;
            node.kind = mkind(n.kind);
            node.smooth = n.smooth && n.on_curve();
            if !node.smooth {
                node.tangent = false;
            }
            path.nodes.push(node);
        }
        paths.push(Shape::Path(path));
    }
    let first = slots.first().copied().unwrap_or(layer.shapes.len());
    let replaced: HashSet<usize> = slots.iter().copied().collect();
    let old_shapes = std::mem::take(&mut layer.shapes);
    let mut paths = Some(paths);
    for (i, shape) in old_shapes.into_iter().enumerate() {
        if i == first {
            layer.shapes.extend(paths.take().unwrap_or_default());
        }
        if !replaced.contains(&i) {
            layer.shapes.push(shape);
        }
    }
    layer.shapes.extend(paths.take().unwrap_or_default());
    Ok(())
}

fn pkind(k: NodeKind) -> PKind {
    match k {
        NodeKind::Line => PKind::Line,
        NodeKind::Curve => PKind::Curve,
        NodeKind::QCurve => PKind::QCurve,
        NodeKind::OffCurve => PKind::OffCurve,
    }
}

fn mkind(k: PKind) -> NodeKind {
    match k {
        PKind::Line => NodeKind::Line,
        PKind::Curve => NodeKind::Curve,
        PKind::QCurve => NodeKind::QCurve,
        PKind::OffCurve => NodeKind::OffCurve,
    }
}

/// A finite number of the object `m` (a part of parameter `key`), if present.
fn number(p: &Params, key: &str, m: &Map<String, Value>, field: &str) -> Result<Option<f64>> {
    match m.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| p.bad(format!("`{key}.{field}` must be a finite number"))),
    }
}

/// Refuses keys of the object `m` (parameter `key`) other than `allowed`.
fn only(p: &Params, key: &str, m: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    match m.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(p.bad(format!("`{key}` takes {}, not `{k}`", allowed.join(", ")))),
        None => Ok(()),
    }
}

/// The checks a filter makes on its options before it touches a layer, with no contours.
fn validate<T>(p: &Params, r: std::result::Result<T, fontcraft_pathops::PathOpError>) -> Result<()> {
    r.map(|_| ()).map_err(|e| p.bad(e.to_string()))
}

// ── filter.adjustStems ──────────────────────────────────────────────────────────────────────────

fn rules(p: &Params, key: &str) -> Result<Vec<StemRule>> {
    let Some(list) = p.opt_array(key)? else { return Ok(Vec::new()) };
    if list.len() > sf::MAX_RULES {
        return Err(p.bad(format!("`{key}` holds {} rules; at most {} are allowed", list.len(), sf::MAX_RULES)));
    }
    list.iter()
        .map(|r| {
            let Value::Object(m) = r else {
                return Err(p.bad(format!("each `{key}` rule must be an object {{from, to, set?, offset?}}")));
            };
            only(p, key, m, &["from", "to", "set", "offset"])?;
            let need = |field: &str| number(p, key, m, field)?.ok_or_else(|| p.bad(format!("each `{key}` rule needs `{field}`")));
            Ok(StemRule { from: need("from")?, to: need("to")?, set: number(p, key, m, "set")?, offset: number(p, key, m, "offset")? })
        })
        .collect()
}

fn adjust(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.adjustStems", v)?;
    let vertical = rules(&p, "vertical")?;
    let horizontal = rules(&p, "horizontal")?;
    if vertical.is_empty() && horizontal.is_empty() {
        return Err(p.bad("give `vertical` or `horizontal` rules"));
    }
    let base = AdjustStems { vertical, horizontal, ..AdjustStems::default() };
    validate(&p, sf::adjust_stems(&[], &base))?;
    run_layers(s, &p, "Adjust Stems", |cx, contours, work| {
        let m = &cx.metrics;
        let options = AdjustStems { fixed_heights: vec![0.0, m.x_height, m.cap_height, m.ascender, m.descender], grid: cx.grid, ..base.clone() };
        let out = sf::adjust_stems_within(contours, &options, work).map_err(|e| p.bad(format!("glyph `{}`: {e}", cx.glyph)))?;
        Ok((out.stems > 0).then(|| Change { stems: out.stems, ..Change::moved(out.contours) }))
    })
}

// ── filter.changeWeight ─────────────────────────────────────────────────────────────────────────

/// CJK ideographs, Hangul and kana: the characters the `auto` method only expands.
fn is_cjk(u: u32) -> bool {
    matches!(u, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Whether the glyph is lowercase: by its character, else by the first letter of its name.
fn is_lowercase(cx: &Cx) -> bool {
    match cx.unicode.and_then(char::from_u32) {
        Some(c) => c.is_lowercase(),
        None => cx.glyph.chars().next().is_some_and(|c| c.is_lowercase()),
    }
}

fn change_weight(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.changeWeight", v)?;
    let amount = p.f64("amount")?;
    let method = p.opt_str("method")?.unwrap_or("auto");
    if !["auto", "lcg", "cjk", "custom"].contains(&method) {
        return Err(p.bad(format!("`method` must be auto, lcg, cjk or custom, got `{method}`")));
    }
    let zones = match p.opt_object("zones")? {
        Some(m) => {
            only(&p, "zones", m, &["bottom", "top", "serifHeight"])?;
            let need = |field: &str| number(&p, "zones", m, field)?.ok_or_else(|| p.bad(format!("`zones` needs `{field}`")));
            Some((need("bottom")?, need("top")?, number(&p, "zones", m, "serifHeight")?.unwrap_or(0.0)))
        }
        None => None,
    };
    if method == "custom" && zones.is_none() {
        return Err(p.bad("the custom method needs `zones`"));
    }
    let counters = p.opt_str("counters")?.unwrap_or("auto");
    if !["auto", "squish", "retain"].contains(&counters) {
        return Err(p.bad(format!("`counters` must be auto, squish or retain, got `{counters}`")));
    }
    let cleanup = p.bool_or("cleanup", false)?;
    let options = |cx: &Cx| {
        let method = match method {
            "cjk" => WeightMethod::Cjk,
            "auto" if cx.unicode.is_some_and(is_cjk) => WeightMethod::Cjk,
            _ => WeightMethod::Lcg,
        };
        let m = &cx.metrics;
        let (bottom, top, serif_height) = zones.unwrap_or((0.0, if is_lowercase(cx) { m.x_height } else { m.cap_height }, 0.0));
        let counters = match (counters, method) {
            ("squish", _) | ("auto", WeightMethod::Cjk) => CounterMode::Squish,
            _ => CounterMode::Retain,
        };
        ChangeWeight { amount, method, bottom, top, serif_height, counters, cleanup, grid: cx.grid }
    };
    let first = Cx { glyph: "", metrics: Metrics { x_height: 500.0, cap_height: 700.0, ..Metrics::default() }, unicode: None, width: 0.0, grid: 0.0 };
    validate(&p, sf::change_weight(&[], &options(&first)))?;
    run_layers(s, &p, "Change Weight", |cx, contours, work| {
        let out = sf::change_weight_within(contours, &options(cx), work).map_err(|e| p.bad(format!("glyph `{}`: {e}", cx.glyph)))?;
        Ok(Some(Change { contours: out.contours, width_change: out.width_change, rebuilt: out.rebuilt, stems: 0 }))
    })
}

// ── filter.condenseExtend ───────────────────────────────────────────────────────────────────────

/// `{scale?: percent, add?: units}` of the parameter `key`, as (factor, add).
fn scale_add(p: &Params, key: &str) -> Result<Option<(f64, f64)>> {
    let Some(m) = p.opt_object(key)? else { return Ok(None) };
    only(p, key, m, &["scale", "add"])?;
    let scale = number(p, key, m, "scale")?.unwrap_or(100.0);
    Ok(Some((scale / 100.0, number(p, key, m, "add")?.unwrap_or(0.0))))
}

fn condense_extend(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.condenseExtend", v)?;
    let counters = scale_add(&p, "counters")?;
    let sidebearings = scale_add(&p, "sidebearings")?;
    if counters.is_none() && sidebearings.is_none() {
        return Err(p.bad("give `counters` or `sidebearings`"));
    }
    let (counter_scale, counter_add) = counters.unwrap_or((1.0, 0.0));
    let (sidebearing_scale, sidebearing_add) = sidebearings.unwrap_or((1.0, 0.0));
    let options = |cx: &Cx| CondenseExtend {
        counter_scale,
        counter_add,
        sidebearing_scale,
        sidebearing_add,
        italic_angle: cx.metrics.italic_angle,
        width: cx.width,
        grid: cx.grid,
    };
    let first = Cx { glyph: "", metrics: Metrics::default(), unicode: None, width: 0.0, grid: 0.0 };
    validate(&p, sf::condense_extend(&[], &options(&first)))?;
    run_layers(s, &p, "Condense / Extend", |cx, contours, work| {
        let out = sf::condense_extend_within(contours, &options(cx), work).map_err(|e| p.bad(format!("glyph `{}`: {e}", cx.glyph)))?;
        Ok(Some(Change { contours: out.contours, width_change: out.width_change, rebuilt: out.rebuilt, stems: 0 }))
    })
}

// ── filter.changeXHeight ────────────────────────────────────────────────────────────────────────

fn change_x_height(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.changeXHeight", v)?;
    let to = p.f64("to")?;
    let from = p.opt_f64("from")?;
    let serif_height = p.opt_f64("serifHeight")?.unwrap_or(0.0);
    let options =
        |cx: &Cx| ChangeXHeight { from: from.unwrap_or(cx.metrics.x_height), to, serif_height, ascender: cx.metrics.ascender, grid: cx.grid };
    let font = s.font()?;
    let metrics = font.masters.first().map(|m| master_metrics(font, m)).unwrap_or_default();
    let first = Cx { glyph: "", metrics, unicode: None, width: 0.0, grid: 0.0 };
    validate(&p, sf::change_x_height(&[], &options(&first)))?;
    run_layers(s, &p, "Change x-Height", |cx, contours, _| {
        let out = sf::change_x_height(contours, &options(cx)).map_err(|e| p.bad(format!("glyph `{}`: {e}", cx.glyph)))?;
        Ok(Some(Change::moved(out)))
    })
}
