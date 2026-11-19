//! The Filter menu: one `filter.*` command for each filter of
//! [`fontcraft_pathops::filters::registry`], `filter.apply` (any filter by name, with the
//! arguments of its export string or its command's parameters), `filter.list` and
//! `filter.repeatLastFilter` (`docs/spec/11-filters.md`).
//!
//! A filter changes the paths of whole layers: one `glyph` or a `glyphs` list (Font View), the
//! first master's layer unless `layer` or `layers` names others. Components and locked paths stay
//! as they are. With `glyphs`, a locked glyph or one without the named layer is skipped and listed
//! in `skipped`; with one `glyph` it is an error. A run over many glyphs is one undo step.
//!
//! Three operations the reference documents as filters are Path menu commands: Transformations,
//! Add Extremes and Remove Overlap. They are in the same registry under their export names, and
//! `filter.apply` runs them through their own commands.
//!
//! [`parse_filter_string`] reads an export-filter string (the `Filter` and `PreFilter` custom
//! parameters); export runs it with [`run_filter_string`].

use std::collections::{HashMap, HashSet};

use fontcraft_geom::Point;
use fontcraft_model::{Font, Layer, Node, NodeKind, Path, Shape};
use fontcraft_pathops::filters::{
    self, Args, Context, Filter, FilterString, Input, MAX_ITEMS, MAX_SCOPE_NAMES, MasterMetrics, Param, ParamKind, Slot, Value as Arg,
};
use fontcraft_pathops::{Contour, Node as PNode, NodeKind as PKind};
use serde_json::{Map, Value, json};

use super::common::{each_layer, grid, in_range, master_metrics, scope};
use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind, Targets};

/// The parameters that say where a filter runs; everything else configures it.
const TARGET_KEYS: [&str; 12] =
    ["glyph", "glyphs", "layer", "layers", "side", "points", "segments", "segment", "shapes", "anchors", "guides", "annotations"];

const LAYER_TARGETS: &[TargetKind] = &[TargetKind::Glyphs, TargetKind::Layer];
const SELECTION_TARGETS: &[TargetKind] = &[TargetKind::Glyphs, TargetKind::Layer, TargetKind::Points, TargetKind::Segments];

/// Filters that are Path menu commands: export name, command, label, menu.
const COMMAND_FILTERS: [(&str, &str, &str, &[&str]); 3] = [
    ("Transformations", "path.transformations", "Transformations", &["Path"]),
    ("AddExtremes", "path.addExtremes", "Add Extremes", &["Path"]),
    ("RemoveOverlap", "path.removeOverlap", "Remove Overlap", &["Path"]),
];

/// A command's `run` is a plain function, so the command of the filter at registry position `I`
/// runs `slot::<I>`. The registry may hold at most this many filters.
const SLOTS: [Run; 16] = [
    slot::<0>, slot::<1>, slot::<2>, slot::<3>, slot::<4>, slot::<5>, slot::<6>, slot::<7>, slot::<8>, slot::<9>, slot::<10>, slot::<11>, slot::<12>,
    slot::<13>, slot::<14>, slot::<15>,
];

fn slot<const I: usize>(s: &mut Session, v: &Value) -> Result<Value> {
    let f = filters::registry().get(I).copied().ok_or_else(|| EngineError::Internal(format!("no filter at registry position {I}")))?;
    run_filter(s, v, f)
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v: Vec<CommandSpec> = filters::registry()
        .iter()
        .zip(SLOTS)
        .map(|(f, run)| CommandSpec {
            id: f.id(),
            label: f.label(),
            menu: &["Filter"],
            shortcut: None,
            params: f.doc(),
            enabled: has_doc,
            run,
            journal: true,
            targets: if f.takes_selection() { SELECTION_TARGETS } else { LAYER_TARGETS },
            when: None,
            context: &[],
            files: &[],
            writes_files: false,
        })
        .collect();
    v.extend([
        cmd!(
            "filter.apply",
            "Apply Filter",
            [],
            None,
            "{filter: name (the export name, e.g. \"RoundCorner\", or the command id, e.g. \"filter.roundCorners\"), args?: [items] (the export string's items after the name: \"20\", \"Key: value\") | {key: value} (the filter command's parameters), string?: export-filter string in place of filter and args (e.g. \"RoundCorner; 20; 1; exclude: a,b\"), include?: [names], exclude?: [names], glyph?: name | glyphs?: [names], layer?: id | layers?: [ids], points?, segments? (with glyph, for filters that take a selection)} runs a filter by name, as an export-time filter string does. Without glyph or glyphs, every glyph of the font; include or exclude narrow that list. Transformations, AddExtremes and RemoveOverlap run their Path menu commands. See filter.list for the names and parameters → the filter command's result with {filter: name, command: id, leftOut?: [names] (the glyphs include or exclude left out, when at most 1000)}",
            has_doc,
            apply
        )
        .targets(SELECTION_TARGETS),
        cmd!(
            query "filter.list",
            "List Filters",
            [],
            None,
            "{} every filter → {filters: [{name (export name), command, label, menu, origin: \"built-in\", selection: bool (takes points and segments), params: [{key, label, kind: \"number\" | \"integer\" | \"bool\" | \"choice\", min?, max?, choices?, default (null with sameAs: key when it takes that parameter's value), sameAs?, required, slot: {position: n} | {key: name} | null (not in the export string), doc}], string?: an export string with the defaults (absent when the filter has no string form), doc}]}",
            always,
            list
        ),
        cmd!(
            "filter.repeatLastFilter",
            "Repeat Last Filter",
            [],
            Some("Cmd+R"),
            "{glyph: name | glyphs: [names], layer?: id | layers?: [ids], points?, segments?} runs the last filter of the session again (the last filter.* command, or filter.apply), with the same settings, on these targets → that filter's result with {repeated: {command, params}}",
            can_repeat,
            repeat
        )
        .targets(SELECTION_TARGETS),
    ]);
    v
}

// ── Filter commands ─────────────────────────────────────────────────────────────────────────────

fn run_filter(s: &mut Session, v: &Value, f: &'static dyn Filter) -> Result<Value> {
    let p = Params::new(f.id(), v)?;
    let mut given: Vec<(&str, Input)> = Vec::new();
    for param in f.params() {
        if let Some(value) = p.get(param.key) {
            given.push((param.key, input(&p, param.key, value)?));
        }
    }
    let args = Args::new(f.params(), given).map_err(|e| p.bad(e))?;
    let font = s.font()?;
    let scope = scope(&p, font)?;
    for key in ["side", "shapes", "anchors", "guides", "annotations"] {
        if p.get(key).is_some() {
            return Err(p.bad(format!("`{key}` is not taken: a filter changes the paths of whole layers")));
        }
    }
    let selected = if ["points", "segments", "segment"].iter().any(|k| p.get(k).is_some()) {
        if !f.takes_selection() {
            return Err(p.bad("this filter changes whole layers: `points` and `segments` are not taken"));
        }
        if !scope.single || p.get("layers").is_some() {
            return Err(p.bad("`points` and `segments` name nodes in one layer: give `glyph` and at most one `layer`"));
        }
        let t = Targets::parse(&p)?;
        t.resolve(font)?;
        Some(t)
    } else {
        None
    };
    let masters = master_data(font);
    let step = grid(font);
    let out = s.edit(f.label(), |font| {
        each_layer(font, &scope, f.id(), |_, layer| {
            let data = masters.get(&layer.master);
            let cx = Context {
                grid: step,
                vertical_stems: data.map(|d| d.vertical.as_slice()).unwrap_or_default(),
                horizontal_stems: data.map(|d| d.horizontal.as_slice()).unwrap_or_default(),
                metrics: data.map(|d| d.metrics).unwrap_or_default(),
                ..Context::default()
            };
            filter_layer(&p, layer, f, &args, cx, selected.as_ref())
        })
    })?;
    let mut v = out.to_json();
    if f.has_string_form()
        && let Some(o) = v.as_object_mut()
    {
        o.insert("string".into(), json!(FilterString::write(f, &args, &[], &[])));
    }
    Ok(v)
}

/// A JSON parameter as a filter argument.
fn input<'a>(p: &Params, key: &str, v: &'a Value) -> Result<Input<'a>> {
    match v {
        Value::Bool(b) => Ok(Input::Bool(*b)),
        Value::Number(n) => n.as_f64().map(Input::Number).ok_or_else(|| p.bad(format!("`{key}` is not a number a filter can take"))),
        Value::String(t) => Ok(Input::Text(t)),
        other => Err(p.bad(format!("`{key}` must be a number, a bool or a word, got {}", crate::params::kind(other)))),
    }
}

/// What a filter reads of a master.
struct MasterData {
    vertical: Vec<f64>,
    horizontal: Vec<f64>,
    metrics: MasterMetrics,
}

fn master_data(font: &Font) -> HashMap<String, MasterData> {
    font.masters
        .iter()
        .map(|m| {
            let stems = |horizontal: bool| -> Vec<f64> {
                font.stems
                    .iter()
                    .zip(&m.stem_values)
                    .filter(|(def, value)| def.horizontal == horizontal && value.is_finite())
                    .map(|(_, value)| *value)
                    .collect()
            };
            let mm = master_metrics(font, m);
            let metrics = MasterMetrics {
                units_per_em: f64::from(font.units_per_em),
                ascender: mm.ascender,
                descender: mm.descender,
                cap_height: mm.cap_height,
                x_height: mm.x_height,
                italic_angle: mm.italic_angle,
            };
            (m.id.clone(), MasterData { vertical: stems(false), horizontal: stems(true), metrics })
        })
        .collect()
}

/// Runs `f` on the unlocked paths of `layer`. `selected`: the node selection of a single-layer
/// run.
fn filter_layer(p: &Params, layer: &mut Layer, f: &dyn Filter, args: &Args, cx: Context, selected: Option<&Targets>) -> Result<Option<String>> {
    let (contours, slots, sources) = layer_contours(layer);
    if contours.is_empty() {
        return Ok(None);
    }
    let picked = selected.map(|t| selection(layer, t, &slots)).unwrap_or_default();
    let cx = Context { selection: &picked, ..cx };
    let out = f.apply(&contours, args, &cx).map_err(|e| p.bad(e.to_string()))?;
    if out != contours {
        write_back(p, layer, &slots, &sources, out)?;
    }
    Ok(None)
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

/// The selected on-curve nodes as (contour, node): selected nodes, and both end nodes of selected
/// segments. Handles select no corner.
fn selection(layer: &Layer, t: &Targets, slots: &[usize]) -> Vec<(usize, usize)> {
    let path = |s: usize| match layer.shapes.get(s) {
        Some(Shape::Path(p)) => Some(p),
        _ => None,
    };
    let mut nodes: Vec<(usize, usize)> = Vec::new();
    for &(s, n) in &t.points {
        if path(s).and_then(|p| p.nodes.get(n)).is_some_and(Node::on_curve) {
            nodes.push((s, n));
        }
    }
    for &(s, n) in &t.segments {
        if let Some(seg) = path(s).and_then(|p| p.segment_ending_at(n)) {
            nodes.push((s, seg.start));
            nodes.push((s, seg.end));
        }
    }
    let mut seen = HashSet::new();
    nodes.into_iter().filter_map(|(s, n)| slots.iter().position(|x| *x == s).map(|c| (c, n))).filter(|a| seen.insert(*a)).collect()
}

/// Replaces the filtered paths of `layer` with `out`. With as many results as inputs, each takes
/// its input's place; otherwise all of them take the place of the first input. A path keeps the
/// lock, attributes and identifier of the input its first surviving node came from (the first
/// such result only), and a surviving node keeps its name and user data.
fn write_back(p: &Params, layer: &mut Layer, slots: &[usize], sources: &[(usize, usize)], out: Vec<Contour>) -> Result<()> {
    let old = |s: usize| match layer.shapes.get(s) {
        Some(Shape::Path(p)) => Some(p),
        _ => None,
    };
    let mut claimed = HashSet::new();
    let mut paths = Vec::with_capacity(out.len());
    for c in out {
        let first = c.nodes.iter().find_map(|n| n.tag).and_then(|t| sources.get(t)).map(|(s, _)| *s);
        let mut path = Path::new(c.closed, Vec::with_capacity(c.nodes.len()));
        if let Some(src) = first.filter(|s| claimed.insert(*s)).and_then(old) {
            path.info = src.info.clone();
            path.extra = src.extra.clone();
        }
        for n in &c.nodes {
            let (x, y) = (n.point.x, n.point.y);
            if !(in_range(x) && in_range(y)) {
                return Err(p.bad(format!("the result has a point ({x}, {y}) outside the coordinate range")));
            }
            let source = n.tag.and_then(|t| sources.get(t)).and_then(|&(s, i)| old(s).and_then(|p| p.nodes.get(i)));
            let mut node = source.cloned().unwrap_or_else(|| Node::new(x, y, mkind(n.kind)));
            node.x = x;
            node.y = y;
            node.kind = mkind(n.kind);
            node.smooth = n.smooth && n.on_curve();
            if !node.smooth {
                node.tangent = false;
            }
            path.nodes.push(node);
        }
        paths.push(Shape::Path(path));
    }
    if paths.len() == slots.len() {
        for (&s, path) in slots.iter().zip(paths) {
            if let Some(shape) = layer.shapes.get_mut(s) {
                *shape = path;
            }
        }
        return Ok(());
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

// ── filter.apply ────────────────────────────────────────────────────────────────────────────────

/// A filter in the registry.
#[derive(Clone, Copy)]
enum Entry {
    Contour(&'static dyn Filter),
    /// A Path menu command: export name, command id.
    Command(&'static str, &'static str),
}

/// The filter named `name` (export name, ASCII case ignored, or command id).
fn find(name: &str) -> Option<Entry> {
    if let Some(f) = filters::find(name) {
        return Some(Entry::Contour(f));
    }
    COMMAND_FILTERS
        .iter()
        .find(|(n, id, ..)| (n.eq_ignore_ascii_case(name) || *id == name) && find_command(id).is_some())
        .map(|(n, id, ..)| Entry::Command(n, id))
}

fn names() -> Vec<&'static str> {
    let mut out: Vec<&str> = filters::registry().iter().map(|f| f.name()).collect();
    out.extend(COMMAND_FILTERS.iter().filter(|(_, id, ..)| find_command(id).is_some()).map(|(n, ..)| *n));
    out
}

/// Reads an export-filter string (`Name; item; …; include: a b`). Export (M7) and
/// `filter.apply` share it.
pub fn parse_filter_string(text: &str) -> std::result::Result<FilterString, String> {
    let fs = FilterString::parse(text)?;
    match find(&fs.name) {
        None => Err(format!("unknown filter `{}`; the filters are {}", fs.name, names().join(", "))),
        Some(Entry::Contour(f)) if !f.has_string_form() => Err(no_string_form(f)),
        Some(_) => Ok(fs),
    }
}

fn no_string_form(f: &dyn Filter) -> String {
    format!("{} has no export string form; run {} or give `args` as an object of its parameters", f.name(), f.id())
}

/// Runs the filter string `text` on every glyph it includes, on the layers `layers` names (the
/// first master's when empty), as one command: what export does with a `Filter` custom parameter.
pub fn run_filter_string(s: &mut Session, text: &str, layers: &[String]) -> Result<Value> {
    let mut params = json!({"string": text});
    if !layers.is_empty()
        && let Some(o) = params.as_object_mut()
    {
        o.insert("layers".into(), json!(layers));
    }
    s.execute("filter.apply", &params)
}

fn apply(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.apply", v)?;
    // The filter string, from `string` or from `filter` and `args`.
    let mut fs = match (p.opt_str("string")?, p.get("filter")) {
        (Some(_), Some(_)) => return Err(p.bad("give `string`, or `filter` with `args`, not both")),
        (Some(_), None) if p.get("args").is_some() => return Err(p.bad("`string` holds the arguments: give it or `args`, not both")),
        (Some(text), None) => parse_filter_string(text).map_err(|e| p.bad(format!("`string`: {e}")))?,
        (None, _) => {
            let name = p.str("filter")?;
            let mut fs = FilterString { name: name.to_string(), ..Default::default() };
            if let Some(Value::Array(items)) = p.get("args") {
                if items.len() >= MAX_ITEMS {
                    return Err(p.bad(format!("`args` holds {} items; at most {} are allowed", items.len(), MAX_ITEMS - 1)));
                }
                let mut text = name.to_string();
                for item in items {
                    let item = match item {
                        Value::String(t) if !t.contains(';') => t.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => u8::from(*b).to_string(),
                        other => {
                            return Err(
                                p.bad(format!("`args` must hold strings without semicolons or numbers, found {}", crate::params::kind(other)))
                            );
                        }
                    };
                    text.push(';');
                    text.push_str(&item);
                }
                fs = parse_filter_string(&text).map_err(|e| p.bad(format!("`args`: {e}")))?;
            }
            fs
        }
    };
    let object = match p.get("args") {
        Some(Value::Object(m)) => Some(m),
        Some(Value::Array(_)) | None => None,
        Some(other) => return Err(p.bad(format!("`args` must be a list of items or an object of parameters, got {}", crate::params::kind(other)))),
    };
    let entry = find(&fs.name).ok_or_else(|| p.bad(format!("unknown filter `{}`; the filters are {}", fs.name, names().join(", "))))?;
    for (key, list) in [("include", &mut fs.include), ("exclude", &mut fs.exclude)] {
        let Some(names) = p.opt_strings(key)? else { continue };
        if !list.is_empty() {
            return Err(p.bad(format!("`{key}` is given in the filter string and as a parameter")));
        }
        if names.len() > MAX_SCOPE_NAMES {
            return Err(p.bad(format!("`{key}` names more than {MAX_SCOPE_NAMES} glyphs")));
        }
        *list = names.into_iter().map(str::to_string).collect();
    }
    if !fs.include.is_empty() && !fs.exclude.is_empty() {
        return Err(p.bad("give `include` or `exclude`, not both"));
    }

    // The command and its settings.
    let (command, mut params) = match (entry, object) {
        (Entry::Contour(f), Some(object)) => {
            let mut given = Vec::with_capacity(object.len());
            for (key, value) in object {
                if !value.is_null() {
                    given.push((key.as_str(), input(&p, key, value)?));
                }
            }
            let args = Args::new(f.params(), given).map_err(|e| p.bad(format!("`args`: {e}")))?;
            (f.id(), args_json(&args))
        }
        (Entry::Contour(f), _) if !f.has_string_form() => return Err(p.bad(no_string_form(f))),
        (Entry::Contour(f), _) => {
            let args = f.read_items(&fs.items).map_err(|e| p.bad(format!("{}: {e}", fs.name)))?;
            (f.id(), args_json(&args))
        }
        (Entry::Command(name, id), Some(object)) => {
            if let Some(key) = object.keys().find(|k| TARGET_KEYS.contains(&k.as_str()) || k.as_str() == "string") {
                return Err(p.bad(format!("`args` holds the settings of {name}; give `{key}` outside it")));
            }
            (id, object.clone())
        }
        (Entry::Command(name, id), _) if name == "Transformations" => {
            let text = FilterString { name: name.to_string(), items: fs.items.clone(), ..Default::default() }.to_string();
            let mut m = Map::new();
            m.insert("string".into(), json!(text));
            (id, m)
        }
        (Entry::Command(name, id), _) => {
            if let Some(item) = fs.items.first() {
                return Err(p.bad(format!("{name} takes no arguments, got \"{item}\"")));
            }
            (id, Map::new())
        }
    };

    // The targets: every glyph unless `glyph` or `glyphs` names some, narrowed by include and
    // exclude.
    for key in TARGET_KEYS {
        if let Some(value) = p.get(key) {
            params.insert(key.into(), value.clone());
        }
    }
    let font = s.font()?;
    let mut left_out = Vec::new();
    if let Some(glyph) = p.opt_str("glyph")? {
        if !fs.includes(glyph) {
            left_out.push(glyph.to_string());
        }
    } else {
        let all = match p.opt_glyphs("glyphs", font)? {
            Some(g) => g,
            None => font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect(),
        };
        let (kept, out): (Vec<String>, Vec<String>) = all.into_iter().partition(|g| fs.includes(g));
        left_out = out;
        params.insert("glyphs".into(), json!(kept));
    }
    let nothing = params.get("glyphs").and_then(Value::as_array).is_some_and(Vec::is_empty) || (params.contains_key("glyph") && !left_out.is_empty());
    let mut v = if nothing { json!({"glyphs": [], "skipped": []}) } else { s.execute(command, &Value::Object(params))? };
    if let Some(o) = v.as_object_mut() {
        o.insert("filter".into(), json!(fs.name));
        o.insert("command".into(), json!(command));
        if !left_out.is_empty() && left_out.len() <= MAX_REPORTED {
            o.insert("leftOut".into(), json!(left_out));
        }
    }
    Ok(v)
}

/// Most glyph names `filter.apply` lists as left out by `include` or `exclude`.
const MAX_REPORTED: usize = 1_000;

fn args_json(args: &Args) -> Map<String, Value> {
    args.iter()
        .map(|(k, v)| {
            let v = match v {
                Arg::Number(n) => json!(n),
                Arg::Bool(b) => json!(b),
                Arg::Word(w) => json!(w),
                // Checked arguments never hold it.
                Arg::SameAs(_) => Value::Null,
            };
            (k.to_string(), v)
        })
        .collect()
}

// ── filter.list ─────────────────────────────────────────────────────────────────────────────────

fn list(_: &mut Session, _: &Value) -> Result<Value> {
    let mut out: Vec<Value> = filters::registry()
        .iter()
        .map(|f| {
            let mut entry = json!({
                "name": f.name(),
                "command": f.id(),
                "label": f.label(),
                "menu": ["Filter"],
                "origin": "built-in",
                "selection": f.takes_selection(),
                "params": f.params().iter().map(param_json).collect::<Vec<_>>(),
                "doc": f.doc(),
            });
            if f.has_string_form()
                && let Some(o) = entry.as_object_mut()
            {
                o.insert("string".into(), json!(FilterString::write(*f, &Args::defaults(f.params()), &[], &[])));
            }
            entry
        })
        .collect();
    for (name, id, label, menu) in COMMAND_FILTERS {
        let Some(spec) = super::registry::find(id) else { continue };
        out.push(json!({
            "name": name,
            "command": id,
            "label": label,
            "menu": menu,
            "origin": "built-in",
            "selection": spec.targets.contains(&TargetKind::Points),
            "params": [],
            "doc": spec.params,
        }));
    }
    Ok(json!({"filters": out}))
}

fn param_json(p: &Param) -> Value {
    let mut m = Map::new();
    m.insert("key".into(), json!(p.key));
    m.insert("label".into(), json!(p.label));
    match p.kind {
        ParamKind::Number { min, max } => {
            m.insert("kind".into(), json!("number"));
            m.insert("min".into(), json!(min));
            m.insert("max".into(), json!(max));
        }
        ParamKind::Integer { min, max } => {
            m.insert("kind".into(), json!("integer"));
            m.insert("min".into(), json!(min));
            m.insert("max".into(), json!(max));
        }
        ParamKind::Bool => {
            m.insert("kind".into(), json!("bool"));
        }
        ParamKind::Choice(words) => {
            m.insert("kind".into(), json!("choice"));
            m.insert("choices".into(), json!(words));
        }
    }
    let default = match p.default {
        Arg::Number(n) => json!(n),
        Arg::Bool(b) => json!(b),
        Arg::Word(w) => json!(w),
        Arg::SameAs(other) => {
            m.insert("sameAs".into(), json!(other));
            Value::Null
        }
    };
    m.insert("default".into(), default);
    m.insert("required".into(), json!(p.required));
    let slot = match p.slot {
        Slot::Position(n) => json!({"position": n}),
        Slot::Key(k) => json!({"key": k}),
        Slot::None => Value::Null,
    };
    m.insert("slot".into(), slot);
    m.insert("doc".into(), json!(p.doc));
    Value::Object(m)
}

// ── filter.repeatLastFilter ─────────────────────────────────────────────────────────────────────

/// The last filter run in this session, from the journal: a `filter.*` command of the registry,
/// or `filter.apply`.
fn last_filter(s: &Session) -> Option<&(String, Value)> {
    s.journal().iter().rev().find(|(id, _)| id == "filter.apply" || filters::registry().iter().any(|f| f.id() == id))
}

fn can_repeat(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    last_filter(s).map(|_| ()).ok_or_else(|| "no filter has run yet".into())
}

fn repeat(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("filter.repeatLastFilter", v)?;
    if p.get("glyph").is_none() && p.get("glyphs").is_none() {
        return Err(p.bad("`glyph` or `glyphs` is required"));
    }
    let (id, last) = last_filter(s).cloned().ok_or_else(|| p.bad("no filter has run yet"))?;
    let mut params = match last {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    for key in TARGET_KEYS {
        params.remove(key);
        if let Some(value) = p.get(key) {
            params.insert(key.into(), value.clone());
        }
    }
    let params = Value::Object(params);
    let mut out = s.execute(&id, &params)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("repeated".into(), json!({"command": id, "params": params}));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filter past the last slot would get no command.
    #[test]
    fn every_registered_filter_has_a_command_slot() {
        assert!(filters::registry().len() <= SLOTS.len(), "add slots to SLOTS");
        for f in filters::registry() {
            assert!(find_command(f.id()).is_some(), "{} has no command", f.id());
        }
    }
}
