//! Instance and master commands (M9.18).
//!
//! Rows: `font.instance.addForEachMaster`, `font.instance.toMaster`,
//! `font.instance.addVariableFontSetting`, `font.master.addFromOtherFont`,
//! `file.generateInstances`, `open.fontsAsMasters`, `switching.renameGlyphsAtExport` (static
//! half). Generate Instances creates at most `fontcraft_interp::MAX_GENERATED_INSTANCES`
//! documents per run.
//!
//! Instance as Master and Generate Instances interpolate through
//! [`fontcraft_interp::instance::instance_font_with_notes`], the function static export uses, so
//! a master made from an instance holds exactly what that instance exports. Rename Glyphs at
//! export is applied by the compiler's export filters (`fontcraft_compile::export_params`); a
//! generated document keeps its instance's parameters, so it exports the same way.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_fontinfo::{MAX_FIELD_BYTES, MAX_INSTANCES, MAX_MASTERS, target};
use fontcraft_interp::instance::{instance_font_with_notes, options_for};
use fontcraft_interp::{Designspace, GlyphSources, MAX_GENERATED_INSTANCES};
use fontcraft_model::axes::design_value;
use fontcraft_model::{Font, Instance, Kerning, Layer, Master, MetricValue};
use serde_json::json;

use super::*;
use crate::EngineError;
use crate::params::Params;

/// The kind a variable font setting has in `Instance::kind`.
const VARIABLE: &str = "variable";
/// The name a new variable font setting gets.
const DEFAULT_SETTING_NAME: &str = "Regular";
/// Instance parameters that only steer interpolation; a generated document has interpolated
/// already, so its instance drops them.
const INTERPOLATION_PARAMS: [&str; 2] = ["InterpolationWeightY", "Local Interpolation"];
/// Font parameters that name a master of the source font, which a generated document lacks.
const MASTER_NAMING_FONT_PARAMS: [&str; 1] = ["Variable Font Origin"];
/// Two coordinates this close are one location.
const SAME_LOCATION: f64 = 1e-9;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.instance.addForEachMaster",
            "Add Instance for Each Master",
            [],
            None,
            "{masters?: [id|name|index]} add one active static instance per master (default: every master, in master order), named after the master and at the master's axis coordinates; an instance is added even when one already sits there → {instances: [{index, name, master}], warnings: [string]}",
            has_doc,
            add_for_each_master
        ),
        cmd!(
            "font.instance.toMaster",
            "Instance as Master",
            [],
            None,
            "{instance: id|name|index} add a master at the end of the list, named after the static instance and at its coordinates, holding the instance's interpolated outlines, metrics, stems, numbers and kerning. Instances on either side then interpolate with the new master and the instance itself uses it unchanged. An intermediate layer at the same location is replaced by the new master's layer. Refused for a variable font setting, and when a master already sits at the instance's location → {master: {index, id, name}, removedLayers: [{glyph, layer}], warnings: [string]}",
            has_doc,
            instance_to_master
        ),
        cmd!(
            "font.instance.addVariableFontSetting",
            "Add Variable Font Setting",
            [],
            None,
            "{name?: string = \"Regular\"} add a variable font setting at the end of the export list; each setting exports one variable font with its own properties and custom parameters, so settings need different names → {index, name, warnings: [string]}",
            has_doc,
            add_variable_font_setting
        ),
        cmd!(
            "font.master.addFromOtherFont",
            "Add Other Font",
            [],
            None,
            "{sources: [{font: index|title|path, master: id|name|index}]} copy masters of open documents (font: the document's position in the session, its title or the path it was opened from) into the active document, each added at the end of the master list as one undo step. A copy keeps its name, metrics (matched by kind and name; a metric the active font lacks is left out, one the other font lacks takes the active font's first master's value), stems and numbers (matched by name), custom parameters, the master layer of every glyph both fonts have and its kerning. Glyphs only the other font has are not added; glyphs it lacks get an empty layer. Coordinates are matched by axis tag, 0 for an axis the other font lacks → {masters: [{index, id, name, from: {document, master}}], warnings: [string]}",
            has_doc,
            add_from_other_font
        ),
        cmd!(
            "file.generateInstances",
            "Generate Instances",
            ["File"],
            None,
            "{instances?: [id|name|index], open?: bool = true, destination?: folder} interpolate static instances of the active document (default: every active one) and make a document of each: one master, named after the instance, with the instance's outlines, metrics, kerning and features and no special layers, and the instance itself as the only export. open: open the documents in the session (the last one becomes active). destination: also save each document into this folder (created when missing), named Family-Style with the extension of the active document's format (.fontcraft for a document that has none; a UFO for a designspace), replacing a file of that name. Every instance is interpolated before anything is opened or written. At most 256 instances per run → {documents: [{instance, title?, index?, path?, warnings}]}",
            has_doc,
            generate_instances
        )
        .files(const { &[FileParam::write("destination")] })
        .writes_files(),
    ]
}

// ── Shared ──────────────────────────────────────────────────────────────────────────────────────

fn interp_error(p: &Params, e: fontcraft_interp::InterpError) -> EngineError {
    p.bad(e.to_string())
}

fn is_variable_setting(inst: &Instance) -> bool {
    inst.kind.as_deref() == Some(VARIABLE)
}

fn same_location(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= SAME_LOCATION)
}

/// Every id a new master or layer must avoid: the masters' ids and every layer id of every glyph.
fn taken_ids(font: &Font) -> HashSet<String> {
    let mut out: HashSet<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    for g in font.glyphs.iter() {
        out.extend(g.layers.iter().map(|l| l.id.clone()));
    }
    out
}

/// The first free id of the form `m01`, `m02`…, the form a new master id takes.
fn new_master_id(p: &Params, taken: &HashSet<String>) -> Result<String> {
    (1..=10_000u32).map(|n| format!("m{n:02}")).find(|id| !taken.contains(id)).ok_or_else(|| p.bad("could not find a free master id"))
}

/// Copies the kerning `from` has under master `from_master` into `to` under `to_master`, in every
/// direction; a context `to` also has gets the value too.
fn copy_kerning(from: &Kerning, from_master: &str, to: &mut Kerning, to_master: &str) {
    for (src, dst) in [(&from.ltr, &mut to.ltr), (&from.rtl, &mut to.rtl), (&from.vertical, &mut to.vertical)] {
        if let Some(entry) = src.0.iter().find(|m| m.master == from_master) {
            let mut e = entry.clone();
            e.master = to_master.to_string();
            dst.0.retain(|m| m.master != to_master);
            dst.0.push(e);
        }
    }
    for c in &mut to.context {
        let value =
            from.context.iter().find(|o| o.context == c.context).and_then(|o| o.values.iter().find(|(m, _)| m == from_master)).map(|(_, v)| *v);
        if let Some(v) = value {
            c.values.retain(|(m, _)| m != to_master);
            c.values.push((to_master.to_string(), v));
        }
    }
}

/// `font` with every alternate layer removed and the ranges on master layers cleared, so an
/// instance of it holds each glyph's main design wherever it lies; and the glyphs that had any.
fn main_design_only(font: &Font) -> (Font, Vec<String>) {
    let mut out = font.clone();
    let mut touched = Vec::new();
    for gi in 0..out.glyphs.len() {
        let has = out.glyphs.as_arcs().get(gi).is_some_and(|g| g.layers.iter().any(|l| !l.info.attrs.axis_rules.is_empty()));
        if !has {
            continue;
        }
        let Some(mut g) = out.glyphs.get_mut_at(gi) else { continue };
        touched.push(g.name.clone());
        g.layers.retain(|l| l.is_master_layer() || l.info.attrs.axis_rules.is_empty());
        for l in g.layers.iter_mut() {
            l.info.attrs.axis_rules.clear();
        }
    }
    (out, touched)
}

// ── Add Instance for Each Master ────────────────────────────────────────────────────────────────

fn add_for_each_master(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.addForEachMaster", v)?;
    let font = s.font()?;
    let indices: Vec<usize> = match p.get("masters") {
        None => (0..font.masters.len()).collect(),
        Some(raw) => {
            if let Value::Array(a) = raw
                && a.len() > MAX_MASTERS
            {
                return Err(p.bad(format!("`masters` holds {} targets; a font has at most {MAX_MASTERS} masters", a.len())));
            }
            let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
            target::masters(font, &refs).map_err(|e| p.bad(e.to_string()))?
        }
    };
    if font.instances.len().saturating_add(indices.len()) > MAX_INSTANCES {
        return Err(p.bad(format!("a font holds at most {MAX_INSTANCES} export entries")));
    }
    let axis_count = font.axes.len();
    let mut made = Vec::with_capacity(indices.len());
    let mut added = Vec::with_capacity(indices.len());
    let mut warnings = Vec::new();
    for (n, &i) in indices.iter().enumerate() {
        let m = font.masters.get(i).ok_or_else(|| p.bad(format!("there is no master #{i}")))?;
        let name = if m.name.trim().is_empty() { m.id.clone() } else { m.name.clone() };
        let location: Vec<f64> = (0..axis_count).map(|a| design_value(&m.location, a)).collect();
        if font.instances.iter().any(|inst| !is_variable_setting(inst) && target::instance_name(inst) == name) {
            warnings.push(format!("another instance is already called `{name}`"));
        }
        made.push(json!({"index": font.instances.len() + n, "name": name, "master": m.id}));
        added.push(Instance { name, location, ..Instance::default() });
    }
    s.edit("Add Instance for Each Master", |font| {
        Arc::make_mut(&mut font.instances).extend(added);
        Ok(())
    })?;
    Ok(json!({"instances": made, "warnings": warnings}))
}

// ── Instance as Master ──────────────────────────────────────────────────────────────────────────

fn instance_to_master(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.toMaster", v)?;
    let font = s.font()?;
    let raw = p.get("instance").ok_or_else(|| p.bad("`instance` is required"))?;
    let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
    let index = target::instance(font, &r).map_err(|e| p.bad(e.to_string()))?;
    let inst = font.instances.get(index).ok_or_else(|| p.bad(format!("there is no instance #{index}")))?;
    if is_variable_setting(inst) {
        return Err(p.bad("a variable font setting has no outlines of its own: name a static instance"));
    }
    if font.masters.len() >= MAX_MASTERS {
        return Err(p.bad(format!("a font has at most {MAX_MASTERS} masters")));
    }
    let name = target::instance_name(inst).to_string();
    let designspace = Designspace::new(font).map_err(|e| interp_error(&p, e))?;
    let location = designspace.instance_location(font, index).map_err(|e| interp_error(&p, e))?;
    if let Some(m) = designspace.masters().iter().find(|m| same_location(&m.location.0, &location.0)) {
        return Err(p.bad(format!("master `{}` already sits at the location of instance `{name}`", m.name)));
    }

    // Intermediate layers at the instance's location would stand where the new master stands.
    let mut replaced: Vec<(String, String)> = Vec::new();
    for g in font.glyphs.iter() {
        let Ok(sources) = GlyphSources::of(font, &designspace, &g.name) else { continue };
        for src in sources.main.iter().filter(|src| !src.is_master && same_location(&src.location.0, &location.0)) {
            replaced.push((g.name.clone(), src.layer_id.clone()));
        }
    }

    let (main, alternates) = main_design_only(font);
    let options = options_for(&main, index).map_err(|e| interp_error(&p, e))?;
    let (instance_font, notes) = instance_font_with_notes(&main, index, &options).map_err(|e| interp_error(&p, e))?;
    let interpolated = instance_font.masters.first().ok_or_else(|| p.bad("the instance has no master"))?;
    let id = new_master_id(&p, &taken_ids(font))?;
    let mut master = interpolated.clone();
    master.id = id.clone();
    master.name = name.clone();
    master.location = location.0.clone();
    master.icon = None;
    crate::linked::unmark(&mut master.user_data);
    let origin_id = interpolated.id.clone();

    let mut warnings = notes;
    warnings.extend(alternates.iter().map(|g| format!("glyph `{g}` has alternate layers; the new master has none of its own")));
    let first_id = font.default_master()?.id.clone();
    let master_index = font.masters.len();
    let removed: Vec<Value> = replaced.iter().map(|(g, l)| json!({"glyph": g, "layer": l})).collect();
    s.edit("Instance as Master", |font| {
        Arc::make_mut(&mut font.masters).push(master);
        for gi in 0..font.glyphs.len() {
            let Some(mut g) = font.glyphs.get_mut_at(gi) else { continue };
            let layer = match instance_font.glyph(&g.name).and_then(|ig| ig.layers.iter().find(|l| l.master == origin_id && l.is_master_layer())) {
                Some(l) => {
                    let mut l = l.clone();
                    l.id = id.clone();
                    l.master = id.clone();
                    l
                }
                None => {
                    let width = g.layers.iter().find(|l| l.id == first_id).map_or(0.0, |l| l.width);
                    Layer::new(id.as_str(), width)
                }
            };
            let name = g.name.clone();
            g.layers.retain(|l| !replaced.iter().any(|(rg, rl)| *rg == name && *rl == l.id));
            g.layers.push(layer);
        }
        copy_kerning(&instance_font.kerning, &origin_id, Arc::make_mut(&mut font.kerning), &id);
        Ok(())
    })?;
    Ok(json!({"master": {"index": master_index, "id": id, "name": name}, "removedLayers": removed, "warnings": warnings}))
}

// ── Add Variable Font Setting ───────────────────────────────────────────────────────────────────

fn add_variable_font_setting(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.instance.addVariableFontSetting", v)?;
    let name = match p.opt_str("name")? {
        None => DEFAULT_SETTING_NAME.to_string(),
        Some(n) if n.trim().is_empty() => return Err(p.bad("`name` cannot be empty")),
        Some(n) if n.len() > MAX_FIELD_BYTES => return Err(p.bad(format!("`name` is longer than {MAX_FIELD_BYTES} bytes"))),
        Some(n) => n.to_string(),
    };
    let font = s.font()?;
    if font.instances.len() >= MAX_INSTANCES {
        return Err(p.bad(format!("a font holds at most {MAX_INSTANCES} export entries")));
    }
    let mut warnings = Vec::new();
    if font.instances.iter().any(|inst| is_variable_setting(inst) && target::instance_name(inst) == name) {
        warnings.push(format!(
            "another variable font setting is called `{name}`: give each a different name so their exports do not overwrite each other"
        ));
    }
    let index = font.instances.len();
    let setting = Instance { name: name.clone(), kind: Some(VARIABLE.to_string()), ..Instance::default() };
    s.edit("Add Variable Font Setting", |font| {
        Arc::make_mut(&mut font.instances).push(setting);
        Ok(())
    })?;
    Ok(json!({"index": index, "name": name, "warnings": warnings}))
}

// ── Add Other Font ──────────────────────────────────────────────────────────────────────────────

/// An open document named by its position, its title or the path it was opened from.
fn document_index(s: &Session, p: &Params, what: &str, v: &Value) -> Result<usize> {
    let docs = s.documents();
    match v {
        Value::Number(n) => n
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < docs.len())
            .ok_or_else(|| p.bad(format!("`{what}`: there is no open document {n}"))),
        Value::String(name) => docs
            .iter()
            .position(|d| d.title == *name || d.path.as_deref().is_some_and(|path| path == std::path::Path::new(name)))
            .or_else(|| super::shared::find_open(s, std::path::Path::new(name)))
            .ok_or_else(|| p.bad(format!("`{what}`: no open document is called or saved as `{name}`"))),
        _ => Err(p.bad(format!("`{what}` must be an open document's position, title or path"))),
    }
}

/// A master of `other` made into a master of `font` with id `id`.
fn imported_master(font: &Font, other: &Font, m: &Master, id: &str) -> Result<Master> {
    let first = font.default_master()?;
    let metric_values: Vec<MetricValue> = font
        .metrics
        .iter()
        .enumerate()
        .map(|(i, def)| {
            let theirs = other.metrics.iter().position(|o| o.kind == def.kind && o.name == def.name && o.filter == def.filter);
            match theirs {
                Some(j) => m.metric_values.get(j).copied().unwrap_or_default(),
                None => first.metric_values.get(i).copied().unwrap_or_default(),
            }
        })
        .collect();
    let stem_values: Vec<f64> = font
        .stems
        .iter()
        .enumerate()
        .map(|(i, def)| {
            let theirs = (!def.name.is_empty()).then(|| other.stems.iter().position(|o| o.name == def.name)).flatten();
            match theirs {
                Some(j) => m.stem_values.get(j).copied().unwrap_or(0.0),
                None => first.stem_values.get(i).copied().unwrap_or(0.0),
            }
        })
        .collect();
    let number_values: Vec<f64> = font
        .numbers
        .iter()
        .enumerate()
        .map(|(i, def)| match other.numbers.iter().position(|o| o.name == def.name) {
            Some(j) => m.number_values.get(j).copied().unwrap_or(0.0),
            None => first.number_values.get(i).copied().unwrap_or(0.0),
        })
        .collect();
    let location: Vec<f64> = font
        .axes
        .iter()
        .map(|a| match other.axes.iter().position(|o| o.tag == a.tag) {
            Some(j) => design_value(&m.location, j),
            None => 0.0,
        })
        .collect();
    let mut out = m.clone();
    out.id = id.to_string();
    out.location = location;
    out.metric_values = metric_values;
    out.stem_values = stem_values;
    out.number_values = number_values;
    crate::linked::unmark(&mut out.user_data);
    Ok(out)
}

fn add_from_other_font(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.master.addFromOtherFont", v)?;
    let list = p.array("sources")?;
    if list.is_empty() {
        return Err(p.bad("`sources` is empty"));
    }
    let font = s.font()?;
    if font.masters.len().saturating_add(list.len()) > MAX_MASTERS {
        return Err(p.bad(format!("a font has at most {MAX_MASTERS} masters")));
    }
    let mut taken = taken_ids(font);
    // (other font, master id there, document index, new master)
    let mut plan: Vec<(Arc<Font>, String, usize, Master)> = Vec::with_capacity(list.len());
    let mut warnings = Vec::new();
    for (n, entry) in list.iter().enumerate() {
        let obj = entry.as_object().ok_or_else(|| p.bad(format!("`sources[{n}]` must be an object {{font, master}}")))?;
        let doc_ref = obj.get("font").ok_or_else(|| p.bad(format!("`sources[{n}].font` is required")))?;
        let doc = document_index(s, &p, &format!("sources[{n}].font"), doc_ref)?;
        let other = s.documents().get(doc).map(|d| Arc::clone(&d.font)).ok_or(EngineError::NoDocument)?;
        let master_ref = obj.get("master").ok_or_else(|| p.bad(format!("`sources[{n}].master` is required")))?;
        let r = target::parse(master_ref).map_err(|e| p.bad(format!("`sources[{n}].master`: {e}")))?;
        let mi = target::master(&other, &r).map_err(|e| p.bad(format!("`sources[{n}].master`: {e}")))?;
        let m = other.masters.get(mi).ok_or_else(|| p.bad(format!("`sources[{n}].master`: there is no master #{mi}")))?;
        let font = s.font()?;
        let id = new_master_id(&p, &taken)?;
        taken.insert(id.clone());
        let master = imported_master(font, &other, m, &id)?;
        if other.units_per_em != font.units_per_em {
            warnings.push(format!(
                "master `{}` comes from a font with {} units per em; this font has {}, and nothing was scaled",
                m.name, other.units_per_em, font.units_per_em
            ));
        }
        let missing = font.glyphs.iter().filter(|g| other.glyph(&g.name).is_none()).count();
        if missing > 0 {
            warnings.push(format!("master `{}`: {missing} glyphs are missing in its font and got an empty layer", m.name));
        }
        let extra = other.glyphs.iter().filter(|g| font.glyph(&g.name).is_none()).count();
        if extra > 0 {
            warnings.push(format!("master `{}`: {extra} glyphs only its font has were not added", m.name));
        }
        let other_id = m.id.clone();
        plan.push((other, other_id, doc, master));
    }
    let first_id = s.font()?.default_master()?.id.clone();
    let start = s.font()?.masters.len();
    let made: Vec<Value> = plan
        .iter()
        .enumerate()
        .map(|(n, (_, from, doc, m))| json!({"index": start + n, "id": m.id, "name": m.name, "from": {"document": doc, "master": from}}))
        .collect();
    s.edit("Add Other Font", |font| {
        for (other, from, _, master) in plan {
            let id = master.id.clone();
            Arc::make_mut(&mut font.masters).push(master);
            for gi in 0..font.glyphs.len() {
                let Some(mut g) = font.glyphs.get_mut_at(gi) else { continue };
                let layer = match other.glyph(&g.name).and_then(|og| og.layers.iter().find(|l| l.master == from && l.is_master_layer())) {
                    Some(l) => {
                        let mut l = l.clone();
                        l.id = id.clone();
                        l.master = id.clone();
                        l
                    }
                    None => {
                        let width = g.layers.iter().find(|l| l.id == first_id).map_or(0.0, |l| l.width);
                        Layer::new(id.as_str(), width)
                    }
                };
                g.layers.push(layer);
            }
            copy_kerning(&other.kerning, &from, Arc::make_mut(&mut font.kerning), &id);
        }
        Ok(())
    })?;
    Ok(json!({"masters": made, "warnings": warnings}))
}

// ── Generate Instances ──────────────────────────────────────────────────────────────────────────

/// The family name an instance exports with: its own familyNames field, else the font's.
fn family_of<'a>(font: &'a Font, inst: &'a Instance) -> &'a str {
    inst.properties.value("familyNames").or_else(|| inst.properties.value("familyName")).unwrap_or(font.family_name.as_str())
}

/// The document made from instance `index` of `font`: its interpolation, with one master named
/// after the instance and the instance as its only export.
pub(super) fn generated_font(p: &Params, font: &Font, index: usize) -> Result<(Font, Vec<String>)> {
    let inst = font.instances.get(index).ok_or_else(|| p.bad(format!("there is no instance #{index}")))?;
    let options = options_for(font, index).map_err(|e| interp_error(p, e))?;
    let (mut out, notes) = instance_font_with_notes(font, index, &options).map_err(|e| interp_error(p, e))?;
    let name = target::instance_name(inst).to_string();
    let location = out.masters.first().map(|m| m.location.clone()).unwrap_or_default();
    for m in Arc::make_mut(&mut out.masters).iter_mut() {
        if !name.is_empty() {
            m.name = name.clone();
        }
        crate::linked::unmark(&mut m.user_data);
    }
    let family = family_of(font, inst).to_string();
    if !family.is_empty() {
        out.family_name = family;
    }
    let mut kept = inst.clone();
    kept.location = location;
    kept.manual_interpolation = false;
    kept.interpolation.clear();
    kept.params.0.retain(|prm| !INTERPOLATION_PARAMS.contains(&prm.name.as_str()));
    out.instances = Arc::new(vec![kept]);
    if inst.style_map_stated.is_some() {
        Arc::make_mut(&mut out.info).style_map_stated = inst.style_map_stated;
    }
    if font.info.params.0.iter().any(|prm| MASTER_NAMING_FONT_PARAMS.contains(&prm.name.as_str())) {
        Arc::make_mut(&mut out.info).params.0.retain(|prm| !MASTER_NAMING_FONT_PARAMS.contains(&prm.name.as_str()));
    }
    out.check()?;
    Ok((out, notes))
}

/// A file name for a generated document: `Family-Style`, letters, digits, `-` and `_` only, made
/// unique among `taken` with `-2`, `-3`….
#[cfg(not(target_arch = "wasm32"))]
fn file_stem(font: &Font, style: &str, taken: &mut HashSet<String>) -> String {
    let clean = |s: &str| s.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').collect::<String>();
    let family = match clean(&font.family_name) {
        f if f.is_empty() => "Font".to_string(),
        f => f,
    };
    let style = match clean(style) {
        st if st.is_empty() => "Instance".to_string(),
        st => st,
    };
    let base = format!("{family}-{style}");
    let mut name = base.clone();
    let mut n = 2u32;
    while taken.contains(&name.to_lowercase()) {
        name = format!("{base}-{n}");
        n = n.saturating_add(1);
    }
    taken.insert(name.to_lowercase());
    name
}

fn generate_instances(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("file.generateInstances", v)?;
    let open = p.bool_or("open", true)?;
    let destination = p.opt_str("destination")?.map(std::path::PathBuf::from);
    if !open && destination.is_none() {
        return Err(p.bad("with `open: false`, give a `destination` folder: otherwise nothing is made"));
    }
    let font = Arc::clone(&s.doc()?.font);
    let indices: Vec<usize> = match p.get("instances") {
        None => (0..font.instances.len()).filter(|&i| font.instances.get(i).is_some_and(|inst| inst.exports && !is_variable_setting(inst))).collect(),
        Some(raw) => {
            let refs = target::parse_list(raw).map_err(|e| p.bad(e.to_string()))?;
            let found = target::instances(&font, &refs).map_err(|e| p.bad(e.to_string()))?;
            if let Some(inst) = found.iter().filter_map(|&i| font.instances.get(i)).find(|inst| is_variable_setting(inst)) {
                return Err(p.bad(format!("`{}` is a variable font setting: name static instances", target::instance_name(inst))));
            }
            found
        }
    };
    if indices.is_empty() {
        return Err(p.bad("the font has no active static instance to generate"));
    }
    if indices.len() > MAX_GENERATED_INSTANCES {
        return Err(p.bad(format!("{} instances asked for; at most {MAX_GENERATED_INSTANCES} per run", indices.len())));
    }

    // Interpolate everything first, so a failing instance opens and writes nothing.
    let mut made: Vec<(String, Font, Vec<String>)> = Vec::with_capacity(indices.len());
    for &i in &indices {
        let name = font.instances.get(i).map(|inst| target::instance_name(inst).to_string()).unwrap_or_default();
        let (f, notes) = generated_font(&p, &font, i).map_err(|e| match e {
            EngineError::BadParams { message, .. } => p.bad(format!("instance `{name}`: {message}")),
            other => other,
        })?;
        made.push((name, f, notes));
    }

    #[cfg(not(target_arch = "wasm32"))]
    let format = destination.as_ref().map(|_| generated_format(&font));
    let paths = match &destination {
        Some(dir) => save_all(&p, &font, dir, &made)?,
        None => vec![None; made.len()],
    };

    let mut documents = Vec::with_capacity(made.len());
    for ((name, f, notes), path) in made.into_iter().zip(paths) {
        let mut entry = json!({"instance": name, "warnings": notes});
        if let (Some(path), Some(o)) = (&path, entry.as_object_mut()) {
            o.insert("path".into(), json!(path.to_string_lossy()));
        }
        if open {
            s.push_document(f, path);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(format) = format {
                s.doc_mut()?.set_file_format(format.source());
            }
            let doc = s.doc()?;
            let index = s.documents().len().saturating_sub(1);
            if let Some(o) = entry.as_object_mut() {
                o.insert("title".into(), json!(doc.title));
                o.insert("index".into(), json!(index));
            }
        }
        documents.push(entry);
    }
    Ok(json!({"documents": documents}))
}

/// The format generated documents of `source` are written in.
#[cfg(not(target_arch = "wasm32"))]
fn generated_format(source: &Font) -> crate::io::SaveFormat {
    use crate::io::SaveFormat;
    match SaveFormat::of_source(source.source.format) {
        Some(SaveFormat::Designspace) => SaveFormat::Ufo,
        Some(f) => f,
        None => SaveFormat::Native,
    }
}

/// Writes every generated document into `dir` ([`generated_format`]).
#[cfg(not(target_arch = "wasm32"))]
fn save_all(p: &Params, source: &Font, dir: &std::path::Path, made: &[(String, Font, Vec<String>)]) -> Result<Vec<Option<std::path::PathBuf>>> {
    if dir.exists() && !dir.is_dir() {
        return Err(p.bad(format!("{} is a file: give a folder", dir.display())));
    }
    let format = generated_format(source);
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Io(format!("cannot create {}: {e}", dir.display())))?;
    let mut taken = HashSet::new();
    let mut out = Vec::with_capacity(made.len());
    for (name, f, _) in made {
        let stem = file_stem(f, name, &mut taken);
        let path = dir.join(format!("{stem}.{}", format.name()));
        crate::io::save(f, &path, format)?;
        out.push(Some(path));
    }
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
fn save_all(_: &Params, _: &Font, _: &std::path::Path, _: &[(String, Font, Vec<String>)]) -> Result<Vec<Option<std::path::PathBuf>>> {
    Err(EngineError::Unsupported("saving generated documents to a folder in the browser".into()))
}
