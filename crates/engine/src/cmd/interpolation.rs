//! Interpolation queries and axis mapping commands (M9.17).
//!
//! Rows: `interpolation.interpolateGlyph`, `interpolation.describeDesignspace`,
//! `interpolation.checkCompatibility`, `axisMappings.addPoint`, `axisMappings.setPoint`,
//! `axisMappings.nudgePoint`, `axisMappings.removePoint`, `axisMappings.convertCoordinate`.
//!
//! Coordinates are internal unless a parameter says otherwise. The mapping commands edit the
//! font's Axis Mappings parameter, or a variable font setting's when `variableFontSetting` names
//! one; the checks and the refusals are `fontcraft_interp::mapping`'s, so an edit that breaks the
//! increasing order changes nothing.

use std::borrow::Cow;
use std::sync::Arc;

use fontcraft_fontinfo::target;
use fontcraft_interp::compat::{self, MismatchKind};
use fontcraft_interp::designspace::{Designspace, Location, Placement, Space, UserLocation};
use fontcraft_interp::instance::{InterpOptions, ModelKind, interpolate_layer};
use fontcraft_interp::mapping::{self, MappingEdit};
use fontcraft_interp::params::{self as iparams, AXIS_MAPPINGS};
use fontcraft_interp::{GlyphSources, InterpError, MAX_INSTANCES, Mismatch};
use fontcraft_model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most mismatches one refusal message lists; the rest are counted.
const MAX_LISTED_MISMATCHES: usize = 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "interpolation.interpolateGlyph",
            "Interpolate Glyph at Location",
            [],
            None,
            "{glyph: name, location: {tag: number}, coordinates?: \"internal\"|\"external\" = \"internal\", model?: \"default\"|\"variable\"} the glyph's layer at the location, interpolated from its masters and intermediate layers, with alternate layers switched by their ranges as an export does. Axes the location leaves out take the origin master's value; `external` values go through Axis Location and Axis Mappings first; `model` defaults to the font's Font Type setting. Locations beyond the masters extrapolate. Incompatible layers are refused with the differences → {glyph, location: {tag: internal}, external: {tag: number}, placement: \"inside\"|\"outside\", layer: {width, shapes, anchors}}",
            has_doc,
            interpolate_glyph
        ),
        cmd!(
            query "interpolation.describeDesignspace",
            "Describe Designspace",
            [],
            None,
            "{variableFontSetting?: id|name|index} the designspace of the font, or of one variable font setting (its disabled masters left out) → {setting: {index, name}|null, axes: [{tag, name, hidden, internal: {minimum, default, maximum}, external: {minimum, default, maximum}, mapping: [{external, internal}]}], origin: {id, name}|null, masters: [{id, name, index, location, external}], virtualMasters: [{name, location}], instances: [{index, name, exports, location, external, placement: \"inside\"|\"extrapolated\"}], settings: [{index, name}], intermediateLayers: [glyph], alternateLayers: [glyph], warnings: [string]}",
            has_doc,
            describe_designspace
        ),
        cmd!(
            query "interpolation.checkCompatibility",
            "Check Master Compatibility",
            [],
            None,
            "{glyphs?: [name], masters?: [id|name|index], includeSpecialLayers?: bool = true, enforceAll?: bool = false} check whether each glyph's layers can interpolate: master layers among themselves, intermediate layers against the masters, alternate layers among themselves. The masters checked are those some instance or variable font setting needs; `enforceAll` takes every master, `masters` exactly those named. Without `glyphs`, every glyph → {masters: [id], indicators: bool, checked: number, incompatible: [glyph], glyphs: [{glyph, ok, mismatches: [{layerA, layerB, kind, message, …}], warnings: [{layerA, layerB, shape, node, degrees}]}]}; `glyphs` lists the glyphs with a mismatch or a warning",
            has_doc,
            check_compatibility
        ),
        cmd!(
            "axisMappings.addPoint",
            "Add Mapping Point",
            [],
            None,
            "{axis: tag, internal: number, external?: number, variableFontSetting?: id|name|index} add a point to the axis's mapping; without `external` the point lies on the current mapping line, so the mapping is unchanged until the point moves. Points must rise on the external coordinate and rise or fall throughout on the internal one → {axis, index, points: [{external, internal}]}",
            has_doc,
            add_point
        ),
        cmd!(
            "axisMappings.setPoint",
            "Edit Mapping Point",
            [],
            None,
            "{axis: tag, index: integer, internal?: number, external?: number, variableFontSetting?: id|name|index} move one point of the axis's mapping (points count from 0 in external order); an edit that breaks the order is refused → {axis, index, points: [{external, internal}]}",
            has_doc,
            set_point
        ),
        cmd!(
            "axisMappings.nudgePoint",
            "Nudge Mapping Point",
            [],
            None,
            "{axis: tag, index: integer, dInternal?: number, dExternal?: number, variableFontSetting?: id|name|index} move one point of the axis's mapping by the given amounts; an edit that breaks the order is refused → {axis, index, points: [{external, internal}]}",
            has_doc,
            nudge_point
        ),
        cmd!(
            "axisMappings.removePoint",
            "Remove Mapping Point",
            [],
            None,
            "{axis: tag, index: integer, variableFontSetting?: id|name|index} delete one point of the axis's mapping; removing the last point leaves the axis without a mapping → {axis, points: [{external, internal}]}",
            has_doc,
            remove_point
        ),
        cmd!(
            query "axisMappings.convertCoordinate",
            "Convert Axis Coordinate",
            [],
            None,
            "{axis: tag, value: number, from: \"internal\"|\"external\", variableFontSetting?: id|name|index} convert a coordinate between the internal and external systems through Axis Location and Axis Mappings → {axis, from, to, value, result}",
            has_doc,
            convert_coordinate
        ),
    ]
}

// ── Shared reading ──────────────────────────────────────────────────────────────────────────────

fn interp_err(p: &Params, e: InterpError) -> EngineError {
    match e {
        InterpError::Incompatible { glyph, mismatches } => p.bad(incompatible_message(&glyph, &mismatches)),
        other => p.bad(other.to_string()),
    }
}

/// The compatibility report of a refused interpolation: every difference, up to a limit.
fn incompatible_message(glyph: &str, mismatches: &[Mismatch]) -> String {
    let listed: Vec<String> = mismatches.iter().take(MAX_LISTED_MISMATCHES).map(ToString::to_string).collect();
    let more = mismatches.len().saturating_sub(MAX_LISTED_MISMATCHES);
    let tail = if more > 0 { format!("; and {more} more") } else { String::new() };
    if listed.is_empty() {
        format!("`{glyph}` has layers that are not compatible")
    } else {
        format!("`{glyph}` has layers that are not compatible: {}{tail}", listed.join("; "))
    }
}

/// The variable font setting `variableFontSetting` names, as an index into `Font::instances`.
fn opt_setting(p: &Params, font: &Font) -> Result<Option<usize>> {
    let Some(raw) = p.get("variableFontSetting") else { return Ok(None) };
    let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
    let i = target::instance(font, &r).map_err(|e| p.bad(e.to_string()))?;
    let inst = font.instances.get(i).ok_or_else(|| p.bad(format!("there is no instance number {i}")))?;
    if inst.kind.as_deref() != Some("variable") {
        return Err(p.bad(format!("`{}` is a static instance, not a variable font setting", target::instance_name(inst))));
    }
    Ok(Some(i))
}

fn designspace_for(p: &Params, font: &Font, setting: Option<usize>) -> Result<Designspace> {
    match setting {
        Some(i) => Designspace::for_setting(font, i),
        None => Designspace::new(font),
    }
    .map_err(|e| interp_err(p, e))
}

fn space(p: &Params, key: &str, default: Option<Space>) -> Result<Space> {
    match (p.opt_str(key)?, default) {
        (Some("internal"), _) => Ok(Space::Internal),
        (Some("external"), _) => Ok(Space::External),
        (Some(other), _) => Err(p.bad(format!("`{key}` is \"internal\" or \"external\", not `{other}`"))),
        (None, Some(d)) => Ok(d),
        (None, None) => Err(p.bad(format!("`{key}` is required"))),
    }
}

fn space_name(s: Space) -> &'static str {
    match s {
        Space::Internal => "internal",
        Space::External => "external",
    }
}

/// `{tag: value}` over the designspace's axes.
fn location_json(ds: &Designspace, values: &[f64]) -> Value {
    let mut out = Map::new();
    for (a, v) in ds.axes().iter().zip(values) {
        out.insert(a.tag.clone(), json!(v));
    }
    Value::Object(out)
}

fn external_json(ds: &Designspace, location: &Location) -> Value {
    match ds.to_external(location) {
        Ok(UserLocation(values)) => location_json(ds, &values),
        Err(_) => Value::Null,
    }
}

fn range_json(r: &fontcraft_model::AxisRange) -> Value {
    json!({"minimum": r.minimum, "default": r.default, "maximum": r.maximum})
}

fn points_json(points: &[(f64, f64)]) -> Value {
    Value::Array(points.iter().map(|(e, i)| json!({"external": e, "internal": i})).collect())
}

fn layer_json(layer: &Layer) -> Result<Value> {
    let shapes = serde_json::to_value(&layer.shapes).map_err(|e| EngineError::Internal(format!("could not describe the shapes: {e}")))?;
    let anchors = serde_json::to_value(&layer.anchors).map_err(|e| EngineError::Internal(format!("could not describe the anchors: {e}")))?;
    Ok(json!({"width": layer.width, "shapes": shapes, "anchors": anchors}))
}

// ── interpolation.interpolateGlyph ──────────────────────────────────────────────────────────────

fn interpolate_glyph(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("interpolation.interpolateGlyph", p)?;
    let font = s.font()?;
    let glyph = p.str("glyph")?;
    if font.glyph(glyph).is_none() {
        return Err(p.bad(format!("there is no glyph called `{glyph}`")));
    }
    let given = p.opt_object("location")?.ok_or_else(|| p.bad("`location` is required"))?;
    let from = space(&p, "coordinates", Some(Space::Internal))?;
    let model = match p.opt_str("model")? {
        Some("default") => ModelKind::Default,
        Some("variable") => ModelKind::Variable,
        Some(other) => return Err(p.bad(format!("`model` is \"default\" or \"variable\", not `{other}`"))),
        None => match font.info.settings.font_type.as_deref() {
            Some("variable") => ModelKind::Variable,
            _ => ModelKind::Default,
        },
    };

    let ds = designspace_for(&p, font, None)?;
    if given.len() > ds.axes().len() {
        return Err(p.bad(format!("`location` names {} axes; the font has {}", given.len(), ds.axes().len())));
    }
    for tag in given.keys() {
        if ds.axis(tag).is_none() {
            return Err(p.bad(format!("the font has no axis `{tag}`")));
        }
    }
    let origin = ds.origin().map(|m| m.location.0.clone()).ok_or_else(|| p.bad("the font has no masters"))?;
    let mut values = Vec::with_capacity(ds.axes().len());
    for (k, a) in ds.axes().iter().enumerate() {
        let value = match given.get(&a.tag) {
            Some(v) => {
                let v = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| p.bad(format!("the `{}` value must be a finite number", a.tag)))?;
                match from {
                    Space::Internal => v,
                    Space::External => mapping::convert(&ds, k, v, Space::External).map_err(|e| interp_err(&p, e))?,
                }
            }
            None => origin.get(k).copied().unwrap_or(0.0),
        };
        values.push(value);
    }
    let location = Location(values);
    // Refuses a value that is not finite or beyond the coordinate limit before any model call.
    ds.to_normalized(&location).map_err(|e| interp_err(&p, e))?;

    let sources = GlyphSources::of(font, &ds, glyph).map_err(|e| interp_err(&p, e))?;
    let options = InterpOptions { extrapolate: true, model, weight_y: None };
    let layer = interpolate_layer(font, &sources, &location, &options).map_err(|e| interp_err(&p, e))?;
    let placement = match ds.classify(&location) {
        Placement::Inside => "inside",
        Placement::Outside => "outside",
    };
    Ok(json!({
        "glyph": glyph,
        "location": location_json(&ds, &location.0),
        "external": external_json(&ds, &location),
        "placement": placement,
        "layer": layer_json(&layer)?,
    }))
}

// ── interpolation.describeDesignspace ───────────────────────────────────────────────────────────

/// Whether a glyph has a layer that could be an intermediate or alternate layer, so the full
/// source reading runs only for those.
fn has_special_layers(g: &fontcraft_model::Glyph) -> bool {
    g.layers.iter().any(|l| !l.info.attrs.axis_rules.is_empty() || (!l.is_master_layer() && !l.info.attrs.coordinates.is_empty()))
}

fn describe_designspace(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("interpolation.describeDesignspace", p)?;
    let font = s.font()?;
    let setting = opt_setting(&p, font)?;
    let ds = designspace_for(&p, font, setting)?;

    let axes: Vec<Value> = ds
        .axes()
        .iter()
        .map(|a| {
            json!({
                "tag": a.tag,
                "name": a.name,
                "hidden": a.hidden,
                "internal": range_json(&a.internal),
                "external": range_json(&a.external),
                "mapping": points_json(a.map.pairs()),
            })
        })
        .collect();
    let masters: Vec<Value> = ds
        .masters()
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "name": m.name,
                "index": m.index,
                "location": location_json(&ds, &m.location.0),
                "external": external_json(&ds, &m.location),
            })
        })
        .collect();
    let virtual_masters: Vec<Value> =
        ds.virtual_masters().iter().map(|v| json!({"name": v.name, "location": location_json(&ds, &v.location.0)})).collect();

    let mut warnings: Vec<String> = ds.warnings().to_vec();
    let mut instances = Vec::new();
    let mut settings = Vec::new();
    for (i, inst) in font.instances.iter().enumerate().take(MAX_INSTANCES) {
        let name = target::instance_name(inst);
        if inst.kind.as_deref() == Some("variable") {
            settings.push(json!({"index": i, "name": name}));
            continue;
        }
        match ds.instance_location(font, i) {
            Ok(location) => {
                let placement = match ds.classify(&location) {
                    Placement::Inside => "inside",
                    Placement::Outside => "extrapolated",
                };
                instances.push(json!({
                    "index": i,
                    "name": name,
                    "exports": inst.exports,
                    "location": location_json(&ds, &location.0),
                    "external": external_json(&ds, &location),
                    "placement": placement,
                }));
            }
            Err(e) => warnings.push(format!("instance `{name}`: {e}")),
        }
    }

    let mut intermediate = Vec::new();
    let mut alternate = Vec::new();
    for g in font.glyphs.iter().filter(|g| has_special_layers(g)) {
        match GlyphSources::of(font, &ds, &g.name) {
            Ok(sources) => {
                if sources.has_intermediates() {
                    intermediate.push(g.name.clone());
                }
                if !sources.alternates.is_empty() {
                    alternate.push(g.name.clone());
                }
            }
            Err(e) => warnings.push(format!("glyph `{}`: {e}", g.name)),
        }
    }

    let setting_json = match setting.and_then(|i| font.instances.get(i).map(|inst| (i, inst))) {
        Some((i, inst)) => json!({"index": i, "name": target::instance_name(inst)}),
        None => Value::Null,
    };
    let origin = match ds.origin() {
        Some(m) => json!({"id": m.id, "name": m.name}),
        None => Value::Null,
    };
    Ok(json!({
        "setting": setting_json,
        "axes": axes,
        "origin": origin,
        "masters": masters,
        "virtualMasters": virtual_masters,
        "instances": instances,
        "settings": settings,
        "intermediateLayers": intermediate,
        "alternateLayers": alternate,
        "warnings": warnings,
    }))
}

// ── interpolation.checkCompatibility ────────────────────────────────────────────────────────────

fn mismatch_json(m: &Mismatch) -> Value {
    let mut v = match &m.kind {
        MismatchKind::ShapeCount { a, b } => json!({"kind": "shapeCount", "a": a, "b": b}),
        MismatchKind::ShapeKind { shape } => json!({"kind": "shapeKind", "shape": shape}),
        MismatchKind::NodeCount { shape, a, b } => json!({"kind": "nodeCount", "shape": shape, "a": a, "b": b}),
        MismatchKind::SegmentType { shape, node } => json!({"kind": "segmentType", "shape": shape, "node": node}),
        MismatchKind::ComponentGlyph { shape, a, b } => json!({"kind": "componentGlyph", "shape": shape, "a": a, "b": b}),
        MismatchKind::AnchorCount { a, b } => json!({"kind": "anchorCount", "a": a, "b": b}),
        MismatchKind::AnchorName { name } => json!({"kind": "anchorName", "name": name}),
    };
    if let Some(o) = v.as_object_mut() {
        o.insert("layerA".into(), json!(m.layer_a));
        o.insert("layerB".into(), json!(m.layer_b));
        o.insert("message".into(), json!(m.to_string()));
    }
    v
}

fn check_compatibility(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("interpolation.checkCompatibility", p)?;
    let font = s.font()?;
    let include_special = p.bool_or("includeSpecialLayers", true)?;
    let enforce_all = p.bool_or("enforceAll", false)?;
    let ds = designspace_for(&p, font, None)?;
    let mut scope = compat::scope(font, &ds);
    if let Some(raw) = p.get("masters") {
        if enforce_all {
            return Err(p.bad("give `masters` or `enforceAll`, not both"));
        }
        if raw.as_array().is_some_and(|a| a.len() > ds.masters().len()) {
            return Err(p.bad(format!("`masters` names more masters than the font has ({})", ds.masters().len())));
        }
        let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
        let indices = target::masters(font, &refs).map_err(|e| p.bad(e.to_string()))?;
        let named: Vec<&str> = indices.iter().filter_map(|&i| font.masters.get(i)).map(|m| m.id.as_str()).collect();
        scope.masters = ds.masters().iter().filter(|m| named.contains(&m.id.as_str())).map(|m| m.id.clone()).collect();
    } else if enforce_all {
        scope.masters = ds.masters().iter().map(|m| m.id.clone()).collect();
    }

    let names: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(names) => names,
        None => font.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let mut incompatible = Vec::new();
    let mut reports = Vec::new();
    for name in &names {
        let (ok, mismatches, warnings) = match GlyphSources::of(font, &ds, name) {
            Ok(mut sources) => {
                if !include_special {
                    sources.main.retain(|s| s.is_master);
                    sources.alternates.clear();
                }
                let c = compat::check_glyph(font, &sources, &scope);
                let warnings: Vec<Value> = c
                    .warnings
                    .iter()
                    .map(|w| json!({"layerA": w.layer_a, "layerB": w.layer_b, "shape": w.shape, "node": w.node, "degrees": w.degrees}))
                    .collect();
                (c.ok, c.first_mismatch.iter().map(mismatch_json).collect::<Vec<_>>(), warnings)
            }
            // A glyph whose sources cannot be read cannot interpolate either.
            Err(e) => (false, vec![json!({"kind": "sources", "message": e.to_string()})], Vec::new()),
        };
        if !ok {
            incompatible.push(name.clone());
        }
        if !ok || !mismatches.is_empty() || !warnings.is_empty() {
            reports.push(json!({"glyph": name, "ok": ok, "mismatches": mismatches, "warnings": warnings}));
        }
    }
    Ok(json!({
        "masters": scope.masters,
        "indicators": scope.indicators,
        "checked": names.len(),
        "incompatible": incompatible,
        "glyphs": reports,
    }))
}

// ── Axis mappings ───────────────────────────────────────────────────────────────────────────────

/// The font as the mapping edit sees it: for a variable font setting with an Axis Mappings
/// parameter of its own, the font with that parameter in place of the font's.
fn mapping_base(font: &Font, setting: Option<usize>) -> Cow<'_, Font> {
    let own = setting.and_then(|i| font.instances.get(i)).and_then(|inst| inst.params.get(AXIS_MAPPINGS));
    match own {
        Some(value) => {
            let mut lite = font.clone();
            Arc::make_mut(&mut lite.info).params.set(AXIS_MAPPINGS, value.clone());
            Cow::Owned(lite)
        }
        None => Cow::Borrowed(font),
    }
}

/// Writes axis `tag`'s new points into the font's Axis Mappings, or the setting's. A setting
/// without its own parameter starts from the font's, so the other axes keep their mappings.
fn write_points(font: &mut Font, setting: Option<usize>, tag: &str, points: &[(f64, f64)]) -> std::result::Result<(), InterpError> {
    match setting {
        None => {
            let info = Arc::make_mut(&mut font.info);
            info.params = iparams::set_axis_mappings(&info.params, tag, points)?;
        }
        Some(i) => {
            let inherited = font.info.params.get(AXIS_MAPPINGS).cloned();
            let inst = Arc::make_mut(&mut font.instances).get_mut(i).ok_or(InterpError::UnknownInstance(i))?;
            let mut start = inst.params.clone();
            if start.get(AXIS_MAPPINGS).is_none()
                && let Some(value) = inherited
            {
                start.set(AXIS_MAPPINGS, value);
            }
            inst.params = iparams::set_axis_mappings(&start, tag, points)?;
        }
    }
    Ok(())
}

/// Reads `axis` and `variableFontSetting`, works out the edit from the axis's current points,
/// and writes the result in one undo step.
fn edit_mapping(
    s: &mut Session,
    p: &Params,
    label: &str,
    make: impl FnOnce(&Params, &Font, &str, usize) -> Result<MappingEdit>,
) -> Result<(String, Vec<(f64, f64)>)> {
    let font = s.font()?;
    let tag = p.str("axis")?.to_string();
    let axis = font.axes.iter().position(|a| a.tag == tag).ok_or_else(|| p.bad(format!("the font has no axis `{tag}`")))?;
    let setting = opt_setting(p, font)?;
    let base = mapping_base(font, setting);
    let edit = make(p, &base, &tag, axis)?;
    let points = mapping::edited_points(&base, &tag, &edit).map_err(|e| interp_err(p, e))?;
    s.edit(label, |f| write_points(f, setting, &tag, &points).map_err(|e| interp_err(p, e)))?;
    Ok((tag, points))
}

fn index_param(p: &Params) -> Result<usize> {
    let max = i64::try_from(fontcraft_interp::MAX_MAPPING_POINTS).unwrap_or(i64::MAX);
    let i = p.opt_int("index", 0, max)?.ok_or_else(|| p.bad("`index` is required"))?;
    usize::try_from(i).map_err(|_| p.bad("`index` must be a whole number from 0"))
}

/// The current points of the axis: a zero move of point `index`, which also checks the index.
fn current_points(p: &Params, base: &Font, tag: &str, index: usize) -> Result<Vec<(f64, f64)>> {
    mapping::edited_points(base, tag, &MappingEdit::Nudge { index, external: 0.0, internal: 0.0 }).map_err(|e| interp_err(p, e))
}

fn add_point(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("axisMappings.addPoint", p)?;
    let internal = p.f64("internal")?;
    let external = p.opt_f64("external")?;
    let (tag, points) = edit_mapping(s, &p, "Add Mapping Point", |p, base, _, axis| {
        let external = match external {
            Some(e) => e,
            // On the line the axis maps by now, so adding the point changes no location.
            None => fontcraft_model::axes::mapping(base, axis).to_user(internal),
        };
        if !external.is_finite() {
            return Err(p.bad("the point has no external value on the current mapping"));
        }
        Ok(MappingEdit::Add { external, internal })
    })?;
    let index = points.iter().position(|q| q.1 == internal && external.is_none_or(|e| q.0 == e)).unwrap_or(0);
    Ok(json!({"axis": tag, "index": index, "points": points_json(&points)}))
}

fn set_point(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("axisMappings.setPoint", p)?;
    let index = index_param(&p)?;
    let internal = p.opt_f64("internal")?;
    let external = p.opt_f64("external")?;
    if internal.is_none() && external.is_none() {
        return Err(p.bad("give `internal`, `external` or both"));
    }
    let (tag, points) = edit_mapping(s, &p, "Edit Mapping Point", |p, base, tag, _| {
        let (e, i) = match (external, internal) {
            (Some(e), Some(i)) => (e, i),
            _ => {
                let now = current_points(p, base, tag, index)?;
                let (e0, i0) = now.get(index).copied().ok_or_else(|| p.bad(format!("there is no point number {}", index + 1)))?;
                (external.unwrap_or(e0), internal.unwrap_or(i0))
            }
        };
        Ok(MappingEdit::Set { index, external: e, internal: i })
    })?;
    Ok(json!({"axis": tag, "index": index, "points": points_json(&points)}))
}

fn nudge_point(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("axisMappings.nudgePoint", p)?;
    let index = index_param(&p)?;
    let d_internal = p.opt_f64("dInternal")?;
    let d_external = p.opt_f64("dExternal")?;
    if d_internal.is_none() && d_external.is_none() {
        return Err(p.bad("give `dInternal`, `dExternal` or both"));
    }
    let edit = MappingEdit::Nudge { index, external: d_external.unwrap_or(0.0), internal: d_internal.unwrap_or(0.0) };
    let (tag, points) = edit_mapping(s, &p, "Nudge Mapping Point", |_, _, _, _| Ok(edit))?;
    Ok(json!({"axis": tag, "index": index, "points": points_json(&points)}))
}

fn remove_point(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("axisMappings.removePoint", p)?;
    let index = index_param(&p)?;
    let (tag, points) = edit_mapping(s, &p, "Remove Mapping Point", |_, _, _, _| Ok(MappingEdit::Remove { index }))?;
    Ok(json!({"axis": tag, "points": points_json(&points)}))
}

// ── axisMappings.convertCoordinate ──────────────────────────────────────────────────────────────

fn convert_coordinate(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("axisMappings.convertCoordinate", p)?;
    let font = s.font()?;
    let tag = p.str("axis")?;
    let value = p.f64("value")?;
    let from = space(&p, "from", None)?;
    let setting = opt_setting(&p, font)?;
    if !font.axes.iter().any(|a| a.tag == tag) {
        return Err(p.bad(format!("the font has no axis `{tag}`")));
    }
    let ds = designspace_for(&p, font, setting)?;
    let (k, _) = ds.axis(tag).ok_or_else(|| p.bad(format!("the variable font setting leaves out the axis `{tag}`")))?;
    let result = mapping::convert(&ds, k, value, from).map_err(|e| interp_err(&p, e))?;
    let to = match from {
        Space::Internal => Space::External,
        Space::External => Space::Internal,
    };
    Ok(json!({"axis": tag, "from": space_name(from), "to": space_name(to), "value": value, "result": result}))
}
