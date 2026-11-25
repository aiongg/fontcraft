//! Glyph sources (M9.37): leaving one source of a glyph out of interpolation, and how much each
//! source contributes at a location.
//!
//! Rows: `glyph.setSourceActive`, `view.showSourceInfluence`; `palette.glyphSources` reads
//! [`influence`].
//!
//! A source is a master, intermediate or alternate layer (`fontcraft_interp::sources`). The
//! inactive mark is the layer's user data `com.fontcraft.sourceInactive`; instances, previews and
//! exports skip a marked layer. The origin master's layer always takes part.

use fontcraft_interp::instance::source_weights;
use fontcraft_interp::labels::special_label;
use fontcraft_interp::sources::{is_inactive, set_inactive};
use fontcraft_interp::{Designspace, GlyphSources, InterpOptions, Location, ModelKind, Source};
use fontcraft_model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.setSourceActive",
            "Use Source in Interpolation",
            [],
            None,
            "{glyph: name, source: layer id, active: bool} leave one of the glyph's sources (a master, intermediate or alternate layer) out of interpolation, or include it again, without touching its drawing. Instances, previews and exports skip an inactive source of that glyph; where another glyph draws the glyph at an inactive master layer, its interpolated drawing stands in. The origin master's layer always takes part and is refused, as is including a source at a location another one already holds → {glyph, source, active, changed: bool}",
            has_doc,
            set_source_active
        ),
        cmd!(
            query "view.showSourceInfluence",
            "Show Source Influence",
            [],
            None,
            "{glyph: name, location?: {tag: number}} how much each of the glyph's sources contributes at an internal location (an axis left out takes the origin master's value). The weights sum to 1; a source that is inactive, empty or replaced by an alternate layer there weighs 0 → {glyph, location: {tag: number}, sources: [{layer, master, name, kind: \"master\" | \"intermediate\" | \"alternate\", location: {tag: number}, active: bool, origin: bool, weight}], missing: [{master, name}]}; `missing` lists the masters the glyph has no layer for",
            has_doc,
            show_source_influence
        ),
    ]
}

/// What kind of source a layer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Master,
    Intermediate,
    Alternate,
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Master => "master",
            SourceKind::Intermediate => "intermediate",
            SourceKind::Alternate => "alternate",
        }
    }
}

/// One source of a glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceRow {
    /// The layer's id.
    pub layer: String,
    pub master: String,
    /// The master's name, with the brace or bracket label of a special layer.
    pub name: String,
    pub kind: SourceKind,
    /// Internal coordinates, one per designspace axis.
    pub location: Location,
    pub active: bool,
    /// The origin master's layer, which cannot be left out.
    pub origin: bool,
    /// Its share of the interpolated glyph at the location asked for.
    pub weight: f64,
}

/// A glyph's sources and their weights at a location.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Influence {
    /// Master layers in master order, then intermediate, alternate and inactive ones.
    pub sources: Vec<SourceRow>,
    /// Ids of the masters the glyph has no layer for.
    pub missing: Vec<String>,
}

/// The sources of `glyph` and how much each contributes at `location`.
pub fn influence(font: &Font, designspace: &Designspace, glyph: &str, location: &Location) -> std::result::Result<Influence, String> {
    let g = font.glyph(glyph).ok_or_else(|| format!("there is no glyph `{glyph}`"))?;
    let sources = GlyphSources::of(font, designspace, glyph).map_err(|e| e.to_string())?;
    let model = if font.info.settings.font_type.as_deref() == Some("variable") { ModelKind::Variable } else { ModelKind::Default };
    let options = InterpOptions { model, ..InterpOptions::default() };
    let weights = if sources.main.is_empty() { Vec::new() } else { source_weights(font, &sources, location, &options).map_err(|e| e.to_string())? };
    let origin = designspace.origin().map(|m| m.id.as_str());
    let mut out = Influence { missing: sources.missing_masters.clone(), ..Influence::default() };
    let alternates = sources.alternates.iter().flat_map(|set| &set.sources);
    for (source, active) in sources.main.iter().chain(alternates).map(|s| (s, true)).chain(sources.inactive.iter().map(|s| (s, false))) {
        if out.sources.iter().any(|r| r.layer == source.layer_id) {
            continue;
        }
        let Some(layer) = g.layers.iter().find(|l| l.id == source.layer_id) else { continue };
        let main = sources.main.iter().any(|s| s.layer_id == source.layer_id);
        out.sources.push(SourceRow {
            layer: source.layer_id.clone(),
            master: source.master.clone(),
            name: source_name(font, source, layer),
            kind: kind(layer, source, main),
            location: source.location.clone(),
            active,
            origin: main && source.is_master && Some(source.master.as_str()) == origin,
            weight: weights.iter().find(|(id, _)| *id == source.layer_id).map_or(0.0, |(_, w)| *w),
        });
    }
    Ok(out)
}

fn kind(layer: &Layer, source: &Source, main: bool) -> SourceKind {
    if !source.is_master {
        SourceKind::Intermediate
    } else if main || layer.is_master_layer() {
        SourceKind::Master
    } else {
        SourceKind::Alternate
    }
}

fn source_name(font: &Font, source: &Source, layer: &Layer) -> String {
    let master = font.masters.iter().find(|m| m.id == source.master).map_or(source.master.as_str(), |m| m.name.as_str());
    match special_label(font, layer) {
        Some(label) => format!("{master} {label}"),
        None => master.to_string(),
    }
}

/// Whether `layer` can be a source: it is a master layer or a special layer that draws an outline.
fn can_be_source(layer: &Layer) -> bool {
    let a = &layer.info.attrs;
    let special = !a.coordinates.is_empty() || !a.axis_rules.is_empty();
    let outline = a.color_palette.is_none() && !a.full_color && !a.svg && a.sbix_size.is_none() && a.smart_poles.is_empty();
    layer.is_master_layer() || (special && outline)
}

fn set_source_active(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("glyph.setSourceActive", value)?;
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("source")?.to_string();
    let active = match p.get("active") {
        Some(Value::Bool(on)) => *on,
        _ => return Err(p.bad("`active` is required: true or false")),
    };
    let font = s.font()?;
    let g = font.glyph(&glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
    crate::check_unlocked(g)?;
    let layer = g.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
    let designspace = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    if !can_be_source(layer) || designspace.master(&layer.master).is_none() {
        return Err(p.bad(format!("layer `{id}` is no source of glyph `{glyph}`: only master, intermediate and alternate layers interpolate")));
    }
    let reply = |changed: bool| json!({"glyph": glyph, "source": id, "active": active, "changed": changed});
    if active != is_inactive(layer) {
        return Ok(reply(false));
    }
    let readable_before = GlyphSources::of(font, &designspace, &glyph).is_ok();
    s.edit("Use Source in Interpolation", |font| {
        {
            let mut g = font.glyph_mut(&glyph)?;
            let layer = g.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
            set_inactive(layer, !active);
        }
        match GlyphSources::of(font, &designspace, &glyph) {
            Ok(after) if !active && after.main.iter().any(|m| m.layer_id == id) => {
                Err(p.bad("the origin master's layer always takes part in interpolation"))
            }
            Err(reason) if readable_before => Err(p.bad(format!("refused: glyph `{glyph}` could no longer interpolate: {reason}"))),
            _ => Ok(()),
        }
    })?;
    Ok(reply(true))
}

fn show_source_influence(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("view.showSourceInfluence", value)?;
    let glyph = p.str("glyph")?;
    let font = s.font()?;
    let ds = Designspace::new(font).map_err(|e| p.bad(format!("the font's axes and masters cannot interpolate: {e}")))?;
    let mut location = ds.origin().map(|m| m.location.clone()).ok_or_else(|| p.bad("the font has no masters"))?;
    if let Some(given) = p.opt_object("location")? {
        if given.len() > ds.axes().len() {
            return Err(p.bad(format!("`location` names {} axes; the font has {}", given.len(), ds.axes().len())));
        }
        for (tag, v) in given {
            let (i, _) = ds.axis(tag).ok_or_else(|| p.bad(format!("the font has no axis `{tag}`")))?;
            let v = v.as_f64().filter(|v| in_range(*v)).ok_or_else(|| p.bad(format!("the `{tag}` value must be a usable number")))?;
            if let Some(slot) = location.0.get_mut(i) {
                *slot = v;
            }
        }
    }
    let found = influence(font, &ds, glyph, &location).map_err(|e| p.bad(e))?;
    let at = |l: &Location| -> Value {
        let mut out = Map::new();
        for (a, v) in ds.axes().iter().zip(&l.0) {
            out.insert(a.tag.clone(), json!(v));
        }
        Value::Object(out)
    };
    let sources: Vec<Value> = found
        .sources
        .iter()
        .map(|r| {
            json!({
                "layer": r.layer,
                "master": r.master,
                "name": r.name,
                "kind": r.kind.name(),
                "location": at(&r.location),
                "active": r.active,
                "origin": r.origin,
                "weight": r.weight,
            })
        })
        .collect();
    let missing: Vec<Value> =
        found.missing.iter().map(|id| json!({"master": id, "name": font.masters.iter().find(|m| m.id == *id).map(|m| m.name.as_str())})).collect();
    Ok(json!({"glyph": glyph, "location": at(&location), "sources": sources, "missing": missing}))
}
