//! Variation utilities (M12.33, M12.34; `fontcraft_tables::variations`, `fontcraft_tables::variations_model`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.
//!
//! Whole-file commands follow [`super::file_io`]: paths in, the input never changed, the output
//! written atomically, a refusal on the web.

use std::path::Path;

use fontcraft_tables::sfnt::{SfntFont, Tag, parse_tag, tag_str};
use fontcraft_tables::variations::{self, Space};
use fontcraft_tables::variations_model::{self, AxisPlan, HvarOptions, PlanOptions};
use fontcraft_tables::{TableOrder, WriteOptions};
use serde_json::{Map, Value, json};

use super::file_io::{self, InputKind, OutputRequest};
use super::*;
use crate::params::Params;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = map_specs();
    v.extend(model_specs());
    v
}

/// `variations.mapCoordinates`, `variations.buildAvar` (M12.33).
fn map_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "variations.mapCoordinates",
            "Map Variation Coordinates",
            [],
            None,
            "{font: path, location: {axis: number}, from?: \"user\"|\"normalized\", to?: \"normalized\"|\"user\", allAxes?: bool, face?: number} converts a location of a variable font through fvar normalisation and the avar table (cross-axis mappings of avar version 2 included); axes not given are at their default; `to` defaults to the space other than `from`; `location` lists the axes given (with avar version 2, the axes that end away from 0), every axis with allAxes → {axes: [tag], to, location: {axis: number}, user: {axis: number}, normalized: {axis: number}, mapped: {axis: number}}",
            always,
            map_coordinates
        ).files(const { &[FileParam::read("font")] }),
        cmd!(
            "variations.buildAvar",
            "Build avar Table",
            [],
            None,
            "{font: path, designspace?: path, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} writes the avar table of a TrueType or CFF variable font from the axis maps and cross-axis mappings of a designspace, or rebuilt from the font's own avar table without one; every map the identity and no cross-axis mapping writes no avar table; the output defaults to <name>.avar.<ext> beside the input → {output, avar: bool, version?: 1|2, segments: {axis: [[from, to]]}}",
            always,
            build_avar
        ).files(const { &[FileParam::read("font"), FileParam::read("designspace"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
    ]
}

/// `variations.buildHvar`, `.describeModel`, `.plot`, `.planAvar` (M12.34).
fn model_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "variations.buildHvar",
            "Build HVAR Table",
            [],
            None,
            "{font: path, vvar?: bool, replace?: bool, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} writes an HVAR table into a TrueType variable font, its advance deltas read from the gvar phantom points, and a VVAR table too when the font has vmtx and vvar is not false; a table the font has is kept unless replace is set; the output defaults to <name>.hvar.<ext> beside the input → {output, hvar: \"built\"|\"kept\", vvar: \"built\"|\"kept\"|\"skipped\", variedGlyphs, regions}",
            always,
            build_hvar
        ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir")] }).writes_files(),
        cmd!(
            query "variations.describeModel",
            "Describe Variation Model",
            [],
            None,
            "{designspace?: path, locations?: [[number]] | [{axis: number}]} the variation model of master locations: the designspace's sources normalised through the axis maps (axes sorted by name), or normalised locations given directly (a list of numbers names the axes A, B, C…; one location must be all zeros) → {axes: [tag], names: [string], locations: [{axis: number}], order: [master], sorted: [{axis: number}], supports: [{axis: [lower, peak, upper]}]}; order, sorted and supports follow the model's master order",
            always,
            describe_model
        ).files(const { &[FileParam::read("designspace")] }),
        cmd!(
            "variations.plot",
            "Plot Variation Model",
            [],
            None,
            "{designspace?: path, font?: path, locations?: [[number]] | [{axis: number}], axes?: [tag], output?: path, noClobber?: bool} draws, as SVG, the scalar each master's deltas are multiplied by over one axis (a curve) or two (a wireframe), one panel per master of the model of a designspace or of locations, or per distinct region of a variable font's gvar and HVAR; axes picks one or two axes when there are more; writes the SVG to output, or returns it → {output?: path, svg?: text, panels, axes: [tag]}",
            always,
            plot
        ).files(const { &[FileParam::read("designspace"), FileParam::read("font"), FileParam::write("output")] }).writes_files(),
        cmd!(
            "variations.planAvar",
            "Plan avar Table",
            [],
            None,
            "{font: path, targets?: {wght?: [number], wdth?: [number], slnt?: [number], opsz?: [number]}, samples?: int, glyphs?: [name] | {name: frequency}, designLimits?: {axis: [min, default, max]}, pins?: {axis: [[before, after]]}, sanitize?: bool, plot?: path, output?: path, outputDir?: path, inPlace?: bool, noClobber?: bool} measures the glyphs of a variable font along its weight, width, slant and optical-size axes and writes the avar maps that place the target user values evenly (the planner's default targets when not given; 8 samples between pins; every glyph once); sanitize adds warnings when the measured limits disagree with the axis values; plot writes an SVG of the maps; the output defaults to <name>.avar.<ext> beside the input → {output, avar: bool, axes: {axis: {limits, mapping: [[user, design]], normalized: [[from, to]], measurements: [[design, value]], warnings: [string]}}, designspace: text, plot?: path}",
            always,
            plan_avar
        ).files(const { &[FileParam::read("font"), FileParam::write("output"), FileParam::write("outputDir"), FileParam::write("plot")] }).writes_files(),
    ]
}

fn read_font(p: &Params<'_>, path: &str, face: u32) -> Result<(file_io::Input, SfntFont)> {
    let input = file_io::read_input(Path::new(path))?;
    if !input.kind.is_font() {
        return Err(p.bad(format!("{path} is not a compiled font")));
    }
    let font = fontcraft_tables::read(&input.bytes, face)?;
    Ok((input, font))
}

fn space(p: &Params<'_>, key: &str) -> Result<Option<Space>> {
    match p.opt_str(key)? {
        None => Ok(None),
        Some("user") => Ok(Some(Space::User)),
        Some("normalized") => Ok(Some(Space::Normalized)),
        Some(other) => Err(p.bad(format!("`{key}` must be \"user\" or \"normalized\", got \"{other}\""))),
    }
}

fn tagged(values: &[(Tag, f64)]) -> Value {
    Value::Object(values.iter().map(|(t, v)| (tag_str(*t).trim_end().to_string(), json!(v))).collect())
}

fn map_coordinates(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.mapCoordinates", params)?;
    let path = p.str("font")?;
    let face = p.opt_int("face", 0, i64::from(u32::MAX))?.and_then(|f| u32::try_from(f).ok()).unwrap_or(0);
    let from = space(&p, "from")?.unwrap_or(Space::User);
    let to = space(&p, "to")?.unwrap_or(if from == Space::User { Space::Normalized } else { Space::User });
    let all_axes = p.bool_or("allAxes", false)?;
    let given = p.opt_object("location")?.ok_or_else(|| p.bad("`location` is required"))?;
    if given.len() > fontcraft_tables::limits::MAX_AXES {
        return Err(p.bad(format!("`location` names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
    }
    let mut location: Vec<(Tag, f64)> = Vec::with_capacity(given.len());
    for (axis, v) in given {
        let tag = parse_tag(axis).map_err(|_| p.bad(format!("`{axis}` is not an axis tag")))?;
        let v = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad(format!("the value of `{axis}` must be a finite number")))?;
        location.push((tag, v));
    }
    let (_, font) = read_font(&p, path, face)?;
    let axes = variations::fvar_axes(&font)?;
    let result = variations::map_coordinates(&font, std::slice::from_ref(&location), from)?;
    let Some(m) = result.first() else { return Err(p.bad("no location was mapped")) };
    // The axes fontTools prints: with avar version 2 those away from 0, otherwise those given.
    let cross_axis = variations::avar_version(&font)? == Some(2);
    let mut shown = Map::new();
    for (axis, &(tag, v)) in axes.iter().zip(&m.mapped) {
        let given = location.iter().any(|(t, _)| *t == tag);
        let show = all_axes || if cross_axis { v != 0.0 } else { given };
        if show {
            let value = if to == Space::User { variations::denormalize_value(v, axis) } else { v };
            shown.insert(tag_str(tag).trim_end().to_string(), json!(value));
        }
    }
    Ok(json!({
        "axes": axes.iter().map(|a| tag_str(a.tag).trim_end().to_string()).collect::<Vec<_>>(),
        "to": if to == Space::User { "user" } else { "normalized" },
        "location": shown,
        "user": tagged(&m.user),
        "normalized": tagged(&m.normalized),
        "mapped": tagged(&m.mapped),
    }))
}

fn build_avar(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.buildAvar", params)?;
    let path = p.str("font")?;
    let req = OutputRequest::from_params(&p)?;
    let (input, font) = read_font(&p, path, 0)?;
    if !matches!(input.kind, InputKind::TrueType | InputKind::Cff) {
        return Err(p.bad(format!("{path} is a {} file: give a TrueType or CFF font (unpack collections and WOFF files first)", input.kind.name())));
    }
    let options = match p.opt_str("designspace")? {
        Some(ds) => {
            let doc = file_io::read_input(Path::new(ds))?;
            let text = String::from_utf8(doc.bytes).map_err(|_| p.bad(format!("{ds} is not UTF-8 text")))?;
            variations::avar_options_from_designspace(&text)?
        }
        None => variations::avar_options_from_font(&font)?,
    };
    let built = variations::build_avar(&font, &options)?;
    let bytes = fontcraft_tables::write(&built, &WriteOptions { order: TableOrder::Recommended })?;
    let out = file_io::output_path(&input.path, &req, ".avar", None)?;
    file_io::write_output(&out, &bytes, &req)?;
    let version = variations::avar_version(&built)?;
    let mut segments = Map::new();
    for (tag, pairs) in variations::segment_maps(&built)? {
        segments.insert(tag_str(tag).trim_end().to_string(), json!(pairs.iter().map(|(a, b)| [*a, *b]).collect::<Vec<_>>()));
    }
    let mut result = json!({ "output": out.display().to_string(), "avar": version.is_some(), "segments": segments });
    if let (Some(v), Some(obj)) = (version, result.as_object_mut()) {
        obj.insert("version".into(), json!(v));
    }
    Ok(result)
}

fn tag_text(tag: Tag) -> String {
    tag_str(tag).trim_end().to_string()
}

fn pairs(v: &[(f64, f64)]) -> Value {
    json!(v.iter().map(|(a, b)| [*a, *b]).collect::<Vec<_>>())
}

fn build_hvar(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.buildHvar", params)?;
    let path = p.str("font")?;
    let req = OutputRequest::from_params(&p)?;
    let options = HvarOptions { vvar: p.bool_or("vvar", true)?, replace: p.bool_or("replace", false)? };
    let (input, font) = read_font(&p, path, 0)?;
    if !matches!(input.kind, InputKind::TrueType) {
        return Err(p.bad(format!("{path} is a {} file: give a TrueType variable font (its gvar table holds the advance deltas)", input.kind.name())));
    }
    let built = variations_model::build_hvar_with(&font, &options)?;
    let bytes = fontcraft_tables::write(&built.font, &WriteOptions { order: TableOrder::Recommended })?;
    let out = file_io::output_path(&input.path, &req, ".hvar", None)?;
    file_io::write_output(&out, &bytes, &req)?;
    Ok(json!({
        "output": out.display().to_string(),
        "hvar": built.hvar.as_str(),
        "vvar": built.vvar.as_str(),
        "variedGlyphs": built.varied_glyphs,
        "regions": built.regions,
    }))
}

/// Master locations from `locations`: lists of numbers (axes A, B, C…) or `{axis: number}`.
fn given_locations(p: &Params<'_>, list: &[Value]) -> Result<Vec<Vec<(Tag, f64)>>> {
    const LETTERS: &[u8; 26] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if list.len() > variations_model::MAX_MODEL_MASTERS {
        return Err(p.bad(format!("`locations` lists more than {} masters", variations_model::MAX_MODEL_MASTERS)));
    }
    let number = |v: &Value| v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| p.bad("a location value must be a finite number"));
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let loc = match item {
            Value::Array(values) => {
                if values.len() > LETTERS.len() {
                    return Err(p.bad(format!("a location lists more than {} values", LETTERS.len())));
                }
                values.iter().zip(LETTERS).map(|(v, c)| Ok(([*c, b' ', b' ', b' '], number(v)?))).collect::<Result<Vec<_>>>()?
            }
            Value::Object(map) => {
                if map.len() > fontcraft_tables::limits::MAX_AXES {
                    return Err(p.bad(format!("a location names more than {} axes", fontcraft_tables::limits::MAX_AXES)));
                }
                map.iter()
                    .map(|(axis, v)| Ok((parse_tag(axis).map_err(|_| p.bad(format!("`{axis}` is not an axis tag")))?, number(v)?)))
                    .collect::<Result<Vec<_>>>()?
            }
            _ => return Err(p.bad("each location is a list of numbers or an {axis: number} object")),
        };
        out.push(loc);
    }
    Ok(out)
}

/// Master locations, each a list of axis tag and coordinate.
type Locations = Vec<Vec<(Tag, f64)>>;

/// The master locations and names of `designspace` or `locations`; `None` when neither is given.
fn model_input(p: &Params<'_>) -> Result<Option<(Locations, Vec<String>)>> {
    match (p.opt_str("designspace")?, p.opt_array("locations")?) {
        (Some(_), Some(_)) => Err(p.bad("give `designspace` or `locations`, not both")),
        (Some(ds), None) => {
            let doc = file_io::read_input(Path::new(ds))?;
            let text = String::from_utf8(doc.bytes).map_err(|_| p.bad(format!("{ds} is not UTF-8 text")))?;
            let read = variations_model::designspace_locations(&text)?;
            let (names, locations) = read.sources.into_iter().unzip();
            Ok(Some((locations, names)))
        }
        (None, Some(list)) => Ok(Some((given_locations(p, list)?, Vec::new()))),
        (None, None) => Ok(None),
    }
}

fn describe_model(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.describeModel", params)?;
    let (locations, names) = model_input(&p)?.ok_or_else(|| p.bad("give `designspace` or `locations`"))?;
    let model = variations_model::describe_model(&locations)?;
    let mut axes: Vec<String> = Vec::new();
    for (t, _) in locations.iter().flatten() {
        let t = tag_text(*t);
        if !axes.contains(&t) {
            axes.push(t);
        }
    }
    let location = |l: &[(Tag, f64)]| tagged(&l.iter().filter(|(_, v)| *v != 0.0).copied().collect::<Vec<_>>());
    Ok(json!({
        "axes": axes,
        "names": names,
        "locations": locations.iter().map(|l| tagged(l)).collect::<Vec<_>>(),
        "order": model.iter().map(|s| s.master).collect::<Vec<_>>(),
        "sorted": model.iter().map(|s| location(&s.location)).collect::<Vec<_>>(),
        "supports": model
            .iter()
            .map(|s| Value::Object(s.region.iter().map(|(t, r)| (tag_text(*t), json!(r))).collect()))
            .collect::<Vec<_>>(),
    }))
}

fn plot(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.plot", params)?;
    let axes: Option<Vec<Tag>> = match p.opt_strings("axes")? {
        Some(list) => Some(list.iter().map(|a| parse_tag(a).map_err(|_| p.bad(format!("`{a}` is not an axis tag")))).collect::<Result<Vec<_>>>()?),
        None => None,
    };
    let font = p.opt_str("font")?;
    let svg = match (model_input(&p)?, font) {
        (Some(_), Some(_)) => return Err(p.bad("give one of `designspace`, `font` and `locations`")),
        (Some((locations, names)), None) => variations_model::plot_with(&locations, &names, axes.as_deref())?,
        (None, Some(path)) => {
            let (_, font) = read_font(&p, path, 0)?;
            variations_model::plot_font(&font, axes.as_deref())?
        }
        (None, None) => return Err(p.bad("give `designspace`, `font` or `locations`")),
    };
    let panels = svg.matches("class=\"panel\"").count();
    let shown: Vec<String> = match &axes {
        Some(a) => a.iter().map(|t| tag_text(*t)).collect(),
        None => Vec::new(),
    };
    match p.opt_str("output")? {
        Some(out) => {
            let req = OutputRequest { output: Some(out.into()), output_dir: None, in_place: false, no_clobber: p.bool_or("noClobber", false)? };
            file_io::write_output(Path::new(out), svg.as_bytes(), &req)?;
            Ok(json!({ "output": out, "panels": panels, "axes": shown }))
        }
        None => Ok(json!({ "svg": svg, "panels": panels, "axes": shown })),
    }
}

fn numbers(p: &Params<'_>, v: &Value, what: &str) -> Result<Vec<f64>> {
    let list = v.as_array().ok_or_else(|| p.bad(format!("{what} must be a list of numbers")))?;
    if list.len() > variations_model::MAX_PLAN_VALUES {
        return Err(p.bad(format!("{what} lists more than {} values", variations_model::MAX_PLAN_VALUES)));
    }
    list.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).ok_or_else(|| p.bad(format!("{what} must hold finite numbers")))).collect()
}

/// The per-axis options of `variations.planAvar`.
fn plan_options(p: &Params<'_>) -> Result<PlanOptions> {
    let mut options = PlanOptions { sanitize: p.bool_or("sanitize", false)?, ..PlanOptions::default() };
    if let Some(n) = p.opt_int("samples", 0, variations_model::MAX_SAMPLES as i64)? {
        options.samples = usize::try_from(n).ok();
    }
    let axis_keys = ["wght", "wdth", "slnt", "opsz"];
    let check_axes = |key: &str, map: &Map<String, Value>| -> Result<()> {
        match map.keys().find(|k| !axis_keys.contains(&k.as_str())) {
            Some(k) => Err(p.bad(format!("`{key}` names the axis `{k}`; the planner plans wght, wdth, slnt and opsz"))),
            None => Ok(()),
        }
    };
    let mut plans: [AxisPlan; 4] = Default::default();
    if let Some(targets) = p.opt_object("targets")? {
        check_axes("targets", targets)?;
        for (i, key) in axis_keys.iter().enumerate() {
            if let (Some(v), Some(plan)) = (targets.get(*key), plans.get_mut(i)) {
                plan.values = Some(numbers(p, v, &format!("targets.{key}"))?);
            }
        }
    }
    if let Some(limits) = p.opt_object("designLimits")? {
        check_axes("designLimits", limits)?;
        for (i, key) in axis_keys.iter().enumerate() {
            if let (Some(v), Some(plan)) = (limits.get(*key), plans.get_mut(i)) {
                match numbers(p, v, &format!("designLimits.{key}"))?.as_slice() {
                    [a, b, c] => plan.design_limits = Some((*a, *b, *c)),
                    _ => return Err(p.bad(format!("designLimits.{key} must be [min, default, max]"))),
                }
            }
        }
    }
    if let Some(pins) = p.opt_object("pins")? {
        check_axes("pins", pins)?;
        for (i, key) in axis_keys.iter().enumerate() {
            let (Some(v), Some(plan)) = (pins.get(*key), plans.get_mut(i)) else { continue };
            let list = v.as_array().ok_or_else(|| p.bad(format!("pins.{key} must be a list of [before, after] pairs")))?;
            if list.len() > variations_model::MAX_PLAN_VALUES {
                return Err(p.bad(format!("pins.{key} lists more than {} pins", variations_model::MAX_PLAN_VALUES)));
            }
            for pair in list {
                match numbers(p, pair, &format!("a pin of {key}"))?.as_slice() {
                    [a, b] => plan.pins.push((*a, *b)),
                    _ => return Err(p.bad(format!("each pin of {key} is a [before, after] pair"))),
                }
            }
        }
    }
    let [weight, width, slant, optical_size] = plans;
    options.weight = weight;
    options.width = width;
    options.slant = slant;
    options.optical_size = optical_size;
    options.glyphs = match p.get("glyphs") {
        None | Some(Value::Null) => None,
        Some(Value::Array(names)) => {
            if names.len() > fontcraft_tables::limits::MAX_GLYPHS {
                return Err(p.bad("`glyphs` lists too many glyphs"));
            }
            Some(
                names
                    .iter()
                    .map(|n| n.as_str().map(|n| (n.to_string(), 1.0)).ok_or_else(|| p.bad("`glyphs` holds glyph names")))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        Some(Value::Object(map)) => {
            if map.len() > fontcraft_tables::limits::MAX_GLYPHS {
                return Err(p.bad("`glyphs` lists too many glyphs"));
            }
            Some(
                map.iter()
                    .map(|(n, f)| {
                        f.as_f64()
                            .filter(|f| f.is_finite())
                            .map(|f| (n.clone(), f))
                            .ok_or_else(|| p.bad(format!("the frequency of `{n}` must be a finite number")))
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        Some(_) => return Err(p.bad("`glyphs` is a list of names or a {name: frequency} object")),
    };
    Ok(options)
}

fn plan_avar(_: &mut Session, params: &Value) -> Result<Value> {
    let p = Params::new("variations.planAvar", params)?;
    let path = p.str("font")?;
    let req = OutputRequest::from_params(&p)?;
    let options = plan_options(&p)?;
    let plot_path = p.opt_str("plot")?;
    let (input, font) = read_font(&p, path, 0)?;
    if !matches!(input.kind, InputKind::TrueType | InputKind::Cff) {
        return Err(p.bad(format!("{path} is a {} file: give a TrueType or CFF variable font", input.kind.name())));
    }
    let plan = variations_model::plan_avar_with(&font, &options)?;
    let out = file_io::output_path(&input.path, &req, ".avar", None)?;
    if let Some(plot) = plot_path {
        file_io::check_write(Path::new(plot))?;
        if req.no_clobber && Path::new(plot).exists() {
            return Err(p.bad(format!("{plot} exists and noClobber is set")));
        }
    }
    let bytes = fontcraft_tables::write(&plan.font, &WriteOptions { order: TableOrder::Recommended })?;
    file_io::write_output(&out, &bytes, &req)?;
    if let Some(plot) = plot_path {
        let plot_req = OutputRequest { output: Some(plot.into()), output_dir: None, in_place: false, no_clobber: req.no_clobber };
        file_io::write_output(Path::new(plot), variations_model::plan_plot(&plan.axes).as_bytes(), &plot_req)?;
    }
    let mut axes = Map::new();
    let mut designspace = String::new();
    for a in &plan.axes {
        designspace.push_str(&a.designspace);
        designspace.push('\n');
        axes.insert(
            tag_text(a.tag),
            json!({
                "limits": [a.limits.0, a.limits.1, a.limits.2],
                "mapping": pairs(&a.mapping),
                "normalized": pairs(&a.normalized),
                "measurements": pairs(&a.measurements),
                "warnings": a.warnings,
            }),
        );
    }
    let mut result = json!({
        "output": out.display().to_string(),
        "avar": plan.font.get(*b"avar").is_some(),
        "axes": axes,
        "designspace": designspace,
    });
    if let (Some(plot), Some(obj)) = (plot_path, result.as_object_mut()) {
        obj.insert("plot".into(), json!(plot));
    }
    Ok(result)
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use fontcraft_tables::sfnt::Flavour;

    /// A TrueType font with only `head` and an `fvar` of wght 100–400–900 and wdth 75–100–125.
    fn variable_font() -> Vec<u8> {
        let mut fvar: Vec<u8> = Vec::new();
        for v in [1u16, 0, 16, 2, 2, 20, 0, 12] {
            fvar.extend_from_slice(&v.to_be_bytes());
        }
        for (tag, lo, de, hi) in [(b"wght", 100i32, 400i32, 900i32), (b"wdth", 75, 100, 125)] {
            fvar.extend_from_slice(tag);
            for v in [lo, de, hi] {
                fvar.extend_from_slice(&(v << 16).to_be_bytes());
            }
            fvar.extend_from_slice(&[0, 0, 1, 0]);
        }
        let mut head = vec![0u8; 54];
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(*b"head", head);
        font.insert(*b"fvar", fvar);
        fontcraft_tables::write(&font, &WriteOptions::default()).unwrap()
    }

    const DESIGNSPACE: &str = r#"<designspace format="5.0"><axes>
  <axis tag="wght" name="Weight" minimum="100" default="400" maximum="900">
    <map input="100" output="20"/><map input="400" output="80"/><map input="700" output="150"/><map input="900" output="200"/>
  </axis>
  <axis tag="wdth" name="Width" minimum="75" default="100" maximum="125"/>
</axes></designspace>"#;

    #[test]
    fn build_avar_then_map_coordinates_match_fonttools() {
        let dir = std::env::temp_dir().join(format!("fontcraft-variations-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let font = dir.join("V.ttf");
        let ds = dir.join("V.designspace");
        std::fs::write(&font, variable_font()).unwrap();
        std::fs::write(&ds, DESIGNSPACE).unwrap();
        let before = std::fs::read(&font).unwrap();
        let mut s = Session::new();

        let r = s.execute("variations.buildAvar", &json!({ "font": font, "designspace": ds })).unwrap();
        let out = dir.join("V.avar.ttf");
        assert_eq!(r["output"], json!(out.display().to_string()), "the default output name");
        assert_eq!(r["version"], json!(1));
        assert_eq!(r["segments"]["wght"][2], json!([9830.0 / 16384.0, 9557.0 / 16384.0]), "fontTools writes 0.6 → 0.58331");
        assert_eq!(std::fs::read(&font).unwrap(), before, "the input is not changed");

        let m = s.execute("variations.mapCoordinates", &json!({ "font": out, "location": { "wght": 700 } })).unwrap();
        assert_eq!(m["to"], json!("normalized"));
        assert!((m["location"]["wght"].as_f64().unwrap() - 0.5833384192859322).abs() < 1e-12);
        assert!(m["location"].get("wdth").is_none(), "an axis not given is not listed");
        let m = s
            .execute("variations.mapCoordinates", &json!({ "font": out, "location": { "wght": 0.5 }, "from": "normalized", "allAxes": true }))
            .unwrap();
        assert_eq!(m["to"], json!("user"));
        assert_eq!(m["location"]["wdth"], json!(100.0));
        let expected = 400.0 + 0.5 * (9557.0 / 16384.0) / (9830.0 / 16384.0) * 500.0;
        assert!((m["location"]["wght"].as_f64().unwrap() - expected).abs() < 1e-9, "user space is the mapped value denormalised");

        let refused = s.execute("variations.buildAvar", &json!({ "font": font, "designspace": ds, "noClobber": true }));
        assert!(refused.is_err(), "the output exists and noClobber is set");
        assert!(s.execute("variations.mapCoordinates", &json!({ "font": out, "location": { "opsz": 12 } })).is_err());
        assert!(s.execute("variations.mapCoordinates", &json!({ "font": ds, "location": {} })).is_err(), "a designspace is not a compiled font");
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TWO_MASTERS: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
familyName = "Plan Sans";
fontMaster = (
{
axesValues = (
400
);
id = m01;
name = Regular;
},
{
axesValues = (
700
);
id = m02;
name = Bold;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(200,0,l),
(200,700,l),
(100,700,l)
);
}
);
width = 300;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(500,0,l),
(500,700,l),
(100,700,l)
);
}
);
width = 600;
}
);
unicode = 65;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

    /// A two-master TrueType variable font exported from a source built in code, without HVAR.
    fn exported_variable_font(dir: &Path) -> std::path::PathBuf {
        let source = dir.join("PlanSans.glyphs");
        std::fs::write(&source, TWO_MASTERS).unwrap();
        let mut s = Session::new();
        s.execute("file.open", &json!({ "path": source })).unwrap();
        let out = dir.join("out");
        s.execute("file.exportVariable", &json!({ "destination": out, "timestamp": 0 })).unwrap();
        let path = out.join("PlanSansVF.ttf");
        let mut font = fontcraft_tables::read(&std::fs::read(&path).unwrap(), 0).unwrap();
        font.remove(*b"HVAR");
        let bare = dir.join("Bare.ttf");
        std::fs::write(&bare, fontcraft_tables::write(&font, &WriteOptions::default()).unwrap()).unwrap();
        bare
    }

    #[test]
    fn build_hvar_and_plan_avar_on_an_exported_variable_font() {
        let dir = std::env::temp_dir().join(format!("fontcraft-variations-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bare = exported_variable_font(&dir);
        let before = std::fs::read(&bare).unwrap();
        let mut s = Session::new();

        let r = s.execute("variations.buildHvar", &json!({ "font": bare })).unwrap();
        let out = dir.join("Bare.hvar.ttf");
        assert_eq!(r["output"], json!(out.display().to_string()), "the default output name");
        assert_eq!((r["hvar"].as_str(), r["vvar"].as_str()), (Some("built"), Some("skipped")));
        assert_eq!(std::fs::read(&bare).unwrap(), before, "the input is not changed");
        let bytes = std::fs::read(&out).unwrap();
        let font = skrifa::FontRef::new(&bytes).unwrap();
        assert!(skrifa::raw::TableProvider::hvar(&font).is_ok());
        let gid = skrifa::MetadataProvider::charmap(&font).map('A').unwrap();
        let advance = |w: f32| {
            let location = skrifa::MetadataProvider::axes(&font).location([("wght", w)]);
            skrifa::MetadataProvider::glyph_metrics(&font, skrifa::instance::Size::unscaled(), &location).advance_width(gid)
        };
        assert_eq!((advance(400.0), advance(550.0), advance(700.0)), (Some(300.0), Some(450.0), Some(600.0)));
        assert!(
            s.execute("variations.buildHvar", &json!({ "font": out, "output": dir.join("again.ttf") })).is_err(),
            "HVAR exists and replace is off"
        );
        assert!(s.execute("variations.buildHvar", &json!({ "font": out, "replace": true, "output": dir.join("again.ttf") })).is_ok());

        let plot = dir.join("plan.svg");
        let r = s.execute("variations.planAvar", &json!({ "font": out, "plot": plot, "samples": 4 })).unwrap();
        assert_eq!(r["output"], json!(dir.join("Bare.hvar.avar.ttf").display().to_string()));
        assert_eq!(r["avar"], json!(true), "a stem that widens linearly needs a map for even weights: {r}");
        let normalized = r["axes"]["wght"]["normalized"].as_array().unwrap();
        assert_eq!(normalized.first(), Some(&json!([0.0, 0.0])), "the axis starts at its default");
        assert_eq!(normalized.last(), Some(&json!([1.0, 1.0])));
        assert!(
            r["designspace"].as_str().unwrap().starts_with("    <axis tag=\"wght\" name=\"Weight\" minimum=\"400\" default=\"400\" maximum=\"700\">")
        );
        assert!(std::fs::read_to_string(&plot).unwrap().starts_with("<svg"));
        assert!(s.execute("variations.planAvar", &json!({ "font": out, "targets": { "ital": [1] } })).is_err(), "only four axes are planned");
        assert!(s.execute("variations.planAvar", &json!({ "font": out, "glyphs": ["missing"] })).is_err());

        let r = s.execute("variations.plot", &json!({ "font": out })).unwrap();
        assert_eq!(r["panels"], json!(1), "one region: {r}");
        assert!(r["svg"].as_str().unwrap().contains("<polyline"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describe_model_and_plot_take_locations_and_designspaces() {
        let mut s = Session::new();
        let r = s.execute("variations.describeModel", &json!({ "locations": [[0, 0], [1, 0], [0, 1], [1, 1]] })).unwrap();
        assert_eq!(r["axes"], json!(["A", "B"]));
        assert_eq!(r["order"], json!([0, 1, 2, 3]));
        assert_eq!(r["supports"][3], json!({ "A": [0.0, 1.0, 1.0], "B": [0.0, 1.0, 1.0] }));
        let r = s.execute("variations.describeModel", &json!({ "locations": [{}, { "wght": -1 }, { "wght": 1 }, { "wght": 0.5 }] })).unwrap();
        assert_eq!(r["order"], json!([0, 1, 3, 2]), "fontTools sorts by distance from the default");
        assert_eq!(r["supports"][3], json!({ "wght": [0.5, 1.0, 1.0] }), "the far master is cut back at the near one");
        assert!(s.execute("variations.describeModel", &json!({ "locations": [[1]] })).is_err(), "no default master");
        assert!(s.execute("variations.describeModel", &json!({})).is_err());

        let dir = std::env::temp_dir().join(format!("fontcraft-variations-plot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ds = dir.join("V.designspace");
        std::fs::write(&ds, DESIGNSPACE.replace("</axes>", "</axes><sources><source filename=\"a.ufo\" name=\"Regular\"><location><dimension name=\"Weight\" xvalue=\"80\"/></location></source><source filename=\"b.ufo\" name=\"Black\"><location><dimension name=\"Weight\" xvalue=\"200\"/></location></source></sources>")).unwrap();
        let r = s.execute("variations.describeModel", &json!({ "designspace": ds })).unwrap();
        assert_eq!(r["names"], json!(["Regular", "Black"]));
        assert_eq!(r["supports"][1], json!({ "wght": [0.0, 1.0, 1.0] }));
        let out = dir.join("plot.svg");
        let r = s.execute("variations.plot", &json!({ "designspace": ds, "output": out })).unwrap();
        assert_eq!(r["panels"], json!(2));
        assert!(std::fs::read_to_string(&out).unwrap().contains(">Black<"));
        assert!(s.execute("variations.plot", &json!({ "designspace": ds, "output": out, "noClobber": true })).is_err());
        assert!(
            s.execute("variations.plot", &json!({ "locations": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]] })).is_err(),
            "three axes need `axes`"
        );
        let r = s.execute("variations.plot", &json!({ "locations": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]], "axes": ["A", "B"] })).unwrap();
        assert_eq!(r["panels"], json!(3));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
