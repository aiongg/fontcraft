//! The export plan (M11.6): the instance switches resolved into the colour tables one export
//! writes. Every table builder in `fontcraft-compile` and `color.describeExport` read this one
//! plan.
//!
//! Rows: `color.export.perInstance`, `color.export.defaultPalette`,
//! `color.export.colorPaletteForCpal`, `color.export.exportColrTableOff`,
//! `color.export.sbixAutomatic`, `color.export.exportSbixTableOff`,
//! `color.export.exportSvgTable`, `color.export.anyFlavour`, `color.layered.toColr`, the eight
//! parameter rows of `docs/spec/15-color-fonts.md` § 9, and `color.export.cbdtNotInReference`.
//!
//! Contract (`docs/plans/M11.md` § 3 › Instance switches):
//! - Defaults: CPAL and COLR whenever palette or full-colour layers exist, sbix whenever iColor
//!   layers exist, the SVG table whenever svg layers exist or a conversion asks for it. Each
//!   `Export … Table` parameter set off removes its table. CBDT only with the export option
//!   [`Switches::cbdt`], never from a parameter.
//! - CPAL holds every palette, the one `Color Palette for CPAL` names moved to position 0, and is
//!   written only when COLR or SVG uses it.
//! - COLR version 0 when every colour glyph is a stack of solid layers, version 1 as soon as one
//!   needs a gradient.
//! - A master's own `Color Palettes` wins over the font's.
//! - An instance between masters with colour layers is refused with a [`ProblemKind::Refusal`]
//!   naming the instance, and the plan holds no tables (until M11.19).

use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Glyph, ModelError, Params, Target, param_flag};

use crate::layers::{ColorLayerKind, ICOLOR_SIZES, color_layers, export_kind};
use crate::paint::{ColorRef, Paint, ShapeRef, drawn_shapes, glyph_paint};
use crate::palette::{self, FOREGROUND_INDEX};
use crate::strike::{self, Strike};
use crate::{ColorError, Palettes, Rgba};

/// Instance parameter: vector colour layers become SVG documents.
pub const COLOR_LAYERS_TO_SVG: &str = "Color Layers to SVG";
/// Instance parameter: layered masters with a Master Color become CPAL and COLR.
pub const COLOR_LAYERS_TO_COLR: &str = "Color Layers to COLR";
/// Instance parameter: the palette moved to the front of CPAL, from 0.
pub const COLOR_PALETTE_FOR_CPAL: &str = "Color Palette for CPAL";
/// Instance parameter: the palette baked into SVG documents, from 0.
pub const COLOR_PALETTE_FOR_SVG: &str = "Color Palette for SVG";
/// Instance parameter: off removes CPAL and COLR.
pub const EXPORT_COLR_TABLE: &str = "Export COLR Table";
/// Instance parameter: off removes sbix.
pub const EXPORT_SBIX_TABLE: &str = "Export sbix Table";
/// Instance parameter: off removes the SVG table.
pub const EXPORT_SVG_TABLE: &str = "Export SVG Table";
/// Instance parameter: the iColor size also written into the SVG table.
pub const SBIX_TO_SVG: &str = "SBIX to SVG";

/// The colour switches of one export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switches {
    pub color_layers_to_svg: bool,
    pub color_layers_to_colr: bool,
    pub palette_for_cpal: usize,
    pub palette_for_svg: usize,
    pub export_colr: bool,
    pub export_sbix: bool,
    pub export_svg: bool,
    /// Pixels per em of the iColor layers copied into the SVG table.
    pub sbix_to_svg: Option<u16>,
    /// Write CBDT and CBLC from the iColor layers: an export option (dialog checkbox, CLI flag),
    /// not a parameter.
    pub cbdt: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            color_layers_to_svg: false,
            color_layers_to_colr: false,
            palette_for_cpal: 0,
            palette_for_svg: 0,
            export_colr: true,
            export_sbix: true,
            export_svg: true,
            sbix_to_svg: None,
            cbdt: false,
        }
    }
}

impl Switches {
    /// The switches the enabled parameters in `params` set ([`fontcraft_model::Resolved::params`]
    /// holds the font's, the master's and the instance's merged). [`Switches::cbdt`] stays off.
    pub fn from_params(params: &Params) -> Switches {
        let mut out = Switches::default();
        out.apply(params);
        out
    }

    /// Overlays the switches `params` set; a parameter that is absent or cannot be read keeps the
    /// value already there.
    fn apply(&mut self, params: &Params) {
        let flag = |name: &str| params.get(name).and_then(param_flag);
        if let Some(on) = flag(COLOR_LAYERS_TO_SVG) {
            self.color_layers_to_svg = on;
        }
        if let Some(on) = flag(COLOR_LAYERS_TO_COLR) {
            self.color_layers_to_colr = on;
        }
        if let Some(on) = flag(EXPORT_COLR_TABLE) {
            self.export_colr = on;
        }
        if let Some(on) = flag(EXPORT_SBIX_TABLE) {
            self.export_sbix = on;
        }
        if let Some(on) = flag(EXPORT_SVG_TABLE) {
            self.export_svg = on;
        }
        if let Some(n) = params.get(COLOR_PALETTE_FOR_CPAL).and_then(whole_number) {
            self.palette_for_cpal = usize::try_from(n).unwrap_or(usize::MAX);
        }
        if let Some(n) = params.get(COLOR_PALETTE_FOR_SVG).and_then(whole_number) {
            self.palette_for_svg = usize::try_from(n).unwrap_or(usize::MAX);
        }
        if let Some(n) = params.get(SBIX_TO_SVG).and_then(whole_number) {
            // 0 or a size no iColor layer can have switches the conversion off.
            self.sbix_to_svg = u16::try_from(n).ok().filter(|s| ICOLOR_SIZES.contains(s));
        }
    }
}

/// A parameter value that is a whole number of at least 0, stored as a number or as text.
fn whole_number(v: &Value) -> Option<u64> {
    let n = match v {
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        other => other.as_num()?.as_f64(),
    };
    // Bounded first, so the conversion below is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX)).then_some(n as u64)
}

/// The switches of an export of `target`: the font's parameters, then its master's, then the
/// instance's, the later winning (`color.export.perInstance`).
pub fn read_switches(font: &Font, target: &Target) -> Result<Switches, ColorError> {
    let mut out = Switches::default();
    out.apply(&font.info.params);
    match target {
        Target::Master(id) => {
            let master = font.masters.iter().find(|m| &m.id == id).ok_or_else(|| ColorError::MissingMaster(id.clone()))?;
            out.apply(&master.params);
        }
        Target::Instance(i) => {
            let inst = font.instances.get(*i).ok_or(ColorError::Model(ModelError::NoSuchInstance(*i)))?;
            if let Some(master) = font.master_at(&inst.location) {
                out.apply(&master.params);
            }
            out.apply(&inst.params);
        }
    }
    Ok(out)
}

/// Something an export should tell the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    /// The glyph it concerns, when one.
    pub glyph: Option<String>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// The export goes ahead.
    Warning,
    /// The colour tables are not written; the compile reports it as an error.
    Refusal,
}

/// The colour tables one export writes, and why not others.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportPlan {
    pub cpal: Option<CpalPlan>,
    pub colr: Option<ColrPlan>,
    pub sbix: Option<SbixPlan>,
    pub svg: Option<SvgPlan>,
    pub cbdt: Option<CbdtPlan>,
    pub problems: Vec<Problem>,
}

impl ExportPlan {
    /// The tags of the tables the plan writes, sorted as the table directory is.
    pub fn tables(&self) -> Vec<&'static str> {
        let mut tags = Vec::new();
        if self.cbdt.is_some() {
            tags.extend(["CBDT", "CBLC"]);
        }
        if self.colr.is_some() {
            tags.push("COLR");
        }
        if self.cpal.is_some() {
            tags.push("CPAL");
        }
        if self.svg.is_some() {
            tags.push("SVG ");
        }
        if self.sbix.is_some() {
            tags.push("sbix");
        }
        tags
    }

    /// Whether a problem stops the colour export.
    pub fn refused(&self) -> bool {
        self.problems.iter().any(|p| p.kind == ProblemKind::Refusal)
    }
}

/// CPAL: every palette, the chosen one first. Colours that are not palette references are
/// added as rows by the CPAL writer when it numbers the COLR colours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpalPlan {
    pub palettes: Palettes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColrVersion {
    #[default]
    V0,
    V1,
}

/// COLR: the base glyphs and their paints, in glyph order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColrPlan {
    pub version: ColrVersion,
    pub glyphs: Vec<ColorGlyph>,
}

/// A base glyph and the paint graph that draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGlyph {
    pub glyph: String,
    pub paint: Paint,
}

/// sbix: the strikes of the exported master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SbixPlan {
    pub strikes: Vec<Strike>,
}

/// CBDT and CBLC: the same strikes as sbix would hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CbdtPlan {
    pub strikes: Vec<Strike>,
}

/// The SVG table: one document source per glyph, in glyph order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgPlan {
    /// The palette baked into documents made from paints.
    pub palette: Vec<Rgba>,
    pub documents: Vec<SvgDocument>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgDocument {
    pub glyph: String,
    pub source: SvgSource,
}

/// What a glyph's SVG document is made from. An svg layer wins over a conversion for its glyph.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgSource {
    /// Vector colour layers or layered masters, through their paint graph.
    Paint(Paint),
    /// The SVG file of an svg layer, with the layer's id and the stored path.
    SvgLayer { layer: String, path: String },
    /// The image of the iColor layer of this size (SBIX to SVG).
    Bitmap { layer: String, ppem: u16 },
}

/// Master parameter: the colour a master is drawn in; in a layered font, the colour its layer
/// takes when Color Layers to COLR or Color Layers to SVG converts it.
pub const MASTER_COLOR: &str = "Master Color";

/// The plan of an export of `target`: its switches read from the instance, the master's layers.
pub fn plan(font: &Font, target: &Target) -> ExportPlan {
    let switches = match read_switches(font, target) {
        Ok(s) => s,
        Err(e) => return ExportPlan { problems: vec![refusal(e.to_string())], ..ExportPlan::default() },
    };
    match target {
        Target::Master(id) => plan_master(font, id, &switches),
        Target::Instance(i) => {
            let Some(inst) = font.instances.get(*i) else { return ExportPlan::default() };
            match font.master_at(&inst.location) {
                Some(master) => plan_master(font, &master.id, &switches),
                None => {
                    let label = if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() };
                    plan_between_masters(font, &label, &switches)
                }
            }
        }
    }
}

/// The plan of an instance interpolated from the masters (`instance` names it): refused when
/// `switches` would write a colour table from the font's colour layers, because colour layers
/// do not interpolate yet; empty otherwise.
pub fn plan_between_masters(font: &Font, instance: &str, switches: &Switches) -> ExportPlan {
    if !writes_color(font, switches) {
        return ExportPlan::default();
    }
    ExportPlan {
        problems: vec![refusal(format!(
            "instance `{instance}` is interpolated from the masters, and colour layers cannot be exported for such an instance yet; export it with Export COLR Table, Export sbix Table and Export SVG Table off, or from a master"
        ))],
        ..ExportPlan::default()
    }
}

/// Whether any exported glyph has a colour layer, or a master layer marked full colour, on any
/// master.
pub fn has_color_layers(font: &Font) -> bool {
    font.glyphs.as_arcs().iter().any(|g| g.export && g.layers.iter().any(|l| export_kind(l).is_some()))
}

/// Whether `switches` would write a table from the font's colour layers or layered masters.
fn writes_color(font: &Font, switches: &Switches) -> bool {
    let layered = ((switches.color_layers_to_colr && switches.export_colr) || (switches.color_layers_to_svg && switches.export_svg))
        && font.masters.iter().any(|m| m.params.get(MASTER_COLOR).is_some());
    layered
        || font.glyphs.as_arcs().iter().filter(|g| g.export).flat_map(|g| g.layers.iter()).filter_map(export_kind).any(|k| match k {
            ColorLayerKind::Palette(_) | ColorLayerKind::FullColor => switches.export_colr || (switches.color_layers_to_svg && switches.export_svg),
            ColorLayerKind::IColor(_) => switches.export_sbix || switches.cbdt || (switches.sbix_to_svg.is_some() && switches.export_svg),
            ColorLayerKind::Svg => switches.export_svg,
        })
}

/// The plan of an export of the layers of `master_id` with `switches`, which is what the
/// compiler runs on a resolved font.
///
/// - COLR and CPAL from the glyphs' palette and full-colour layers, and with Color Layers to
///   COLR from the masters with a Master Color for glyphs that have no such layers. CPAL goes
///   with COLR and never alone, as fontmake writes it.
/// - The SVG table: a glyph's svg layer, else with Color Layers to SVG its vector paint, else
///   with SBIX to SVG the image of its iColor layer of the size [`strike::pick_size`] picks.
/// - sbix and CBDT from the master's strikes.
pub fn plan_master(font: &Font, master_id: &str, switches: &Switches) -> ExportPlan {
    let mut plan = ExportPlan::default();
    let palettes = match palette::read(font, Some(master_id)) {
        Ok(p) => Some(p),
        Err(e) => {
            plan.problems.push(warning(None, format!("{e}; the colour layers that use the palettes are not exported")));
            None
        }
    };
    let layered = if switches.color_layers_to_colr || switches.color_layers_to_svg { layered_masters(font, &mut plan.problems) } else { Vec::new() };
    if switches.color_layers_to_colr && switches.export_colr && layered.is_empty() {
        plan.problems.push(warning(None, format!("{COLOR_LAYERS_TO_COLR} is on, but no master has a {MASTER_COLOR}")));
    }

    let mut colr_glyphs = Vec::new();
    let mut documents = Vec::new();
    for glyph in fontcraft_model::glyph_order(font).into_iter().filter_map(|i| font.glyphs.as_arcs().get(i)) {
        if !glyph.export {
            continue;
        }
        let own = match &palettes {
            Some(p) => glyph_paint(font, glyph, master_id, p).unwrap_or_else(|e| {
                plan.problems.push(warning(Some(&glyph.name), format!("its colour layers are not exported: {e}")));
                None
            }),
            None => None,
        };
        // Layered masters convert only glyphs without vector colour layers of their own.
        let layered_paint = if own.is_none() { layered_paint(glyph, &layered) } else { None };
        if switches.export_colr {
            let paint = own.clone().or_else(|| layered_paint.clone().filter(|_| switches.color_layers_to_colr));
            if let Some(paint) = paint {
                colr_glyphs.push(ColorGlyph { glyph: glyph.name.clone(), paint });
            }
        }
        if switches.export_svg {
            let vector = if switches.color_layers_to_svg { own.or(layered_paint) } else { None };
            if let Some(source) = svg_source(glyph, master_id, switches, vector, &mut plan.problems) {
                documents.push(SvgDocument { glyph: glyph.name.clone(), source });
            }
        }
    }

    let palettes = palettes.unwrap_or_default();
    if !colr_glyphs.is_empty() {
        for g in &colr_glyphs {
            check_rows(&g.glyph, &g.paint, palettes.rows(), &mut plan.problems);
        }
        let version = if colr_glyphs.iter().all(|g| crate::paint::is_solid_stack(&g.paint)) { ColrVersion::V0 } else { ColrVersion::V1 };
        plan.cpal = Some(CpalPlan { palettes: chosen_first(&palettes, switches.palette_for_cpal, &mut plan.problems) });
        plan.colr = Some(ColrPlan { version, glyphs: colr_glyphs });
    }
    if !documents.is_empty() {
        let palette = match palettes.palette(switches.palette_for_svg) {
            Some(p) => p.to_vec(),
            None => {
                if !palettes.is_empty() {
                    plan.problems.push(warning(
                        None,
                        format!(
                            "{COLOR_PALETTE_FOR_SVG} names palette {}, but the font has {}; palette 0 is used",
                            switches.palette_for_svg,
                            palettes.palettes().len()
                        ),
                    ));
                }
                palettes.palette(0).map(<[Rgba]>::to_vec).unwrap_or_default()
            }
        };
        plan.svg = Some(SvgPlan { palette, documents });
    }

    if switches.export_sbix || switches.cbdt {
        let strikes = strike::strikes(font, master_id);
        if !strikes.is_empty() {
            if switches.export_sbix {
                plan.sbix = Some(SbixPlan { strikes: strikes.clone() });
                plan.problems.extend(strike::strike_problems(font, master_id));
            }
            if switches.cbdt {
                plan.cbdt = Some(CbdtPlan { strikes });
            }
        }
    }
    plan
}

fn warning(glyph: Option<&str>, message: String) -> Problem {
    Problem { kind: ProblemKind::Warning, glyph: glyph.map(str::to_string), message }
}

fn refusal(message: String) -> Problem {
    Problem { kind: ProblemKind::Refusal, glyph: None, message }
}

/// The masters with a readable Master Color, in master order, each with its colour.
fn layered_masters(font: &Font, problems: &mut Vec<Problem>) -> Vec<(String, Rgba)> {
    let mut out = Vec::new();
    for m in font.masters.iter() {
        let Some(value) = m.params.get(MASTER_COLOR) else { continue };
        match master_color(value) {
            Some(color) => out.push((m.id.clone(), color)),
            None => problems.push(warning(None, format!("the {MASTER_COLOR} of master `{}` is not a colour; the master is left out", m.name))),
        }
    }
    out
}

/// A Master Color value: the text `R,G,B,A` or `R,G,B` with components from 0 to 255, a list of
/// such numbers, or `#RRGGBB` / `#RRGGBBAA`.
pub fn master_color(value: &Value) -> Option<Rgba> {
    let parts: Vec<f64> = match value {
        Value::String(s) => {
            let text = s.as_str().trim();
            if text.starts_with('#') {
                return Rgba::parse_hex(text);
            }
            if text.split(',').count() > 4 {
                return None;
            }
            text.split(',').map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?
        }
        Value::Array(a) if a.items.len() <= 4 => a.items.iter().map(|v| v.as_num().map(|n| n.as_f64())).collect::<Option<_>>()?,
        _ => return None,
    };
    let byte = |v: &f64| -> Option<u8> {
        // Range-checked first, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (v.is_finite() && (0.0..=255.0).contains(v)).then(|| v.round() as u8)
    };
    match parts.as_slice() {
        [r, g, b] => Some(Rgba::new(byte(r)?, byte(g)?, byte(b)?, 255)),
        [r, g, b, a] => Some(Rgba::new(byte(r)?, byte(g)?, byte(b)?, byte(a)?)),
        _ => None,
    }
}

/// The paint of `glyph` in a layered font: one layer per master with a Master Color whose
/// layer of the glyph has outlines, the first master on top (`color.layered.stackingOrder`).
fn layered_paint(glyph: &Glyph, layered: &[(String, Rgba)]) -> Option<Paint> {
    let layers: Vec<Paint> = layered
        .iter()
        .rev()
        .filter_map(|(master, color)| {
            let layer = glyph.layer(master)?;
            let shapes = drawn_shapes(&layer.shapes);
            (!shapes.is_empty()).then(|| Paint::Glyph {
                shape: ShapeRef { layer: layer.id.clone(), shapes, stroke: None },
                paint: Box::new(Paint::Solid(ColorRef::Rgba(color.to_f64()))),
            })
        })
        .collect();
    (!layers.is_empty()).then_some(Paint::Layers(layers))
}

/// Where `glyph`'s SVG document comes from, if it has one: its svg layer wins over a conversion
/// (`color.fourKinds`); then `vector`, the paint Color Layers to SVG converts; then SBIX to SVG.
fn svg_source(glyph: &Glyph, master_id: &str, switches: &Switches, vector: Option<Paint>, problems: &mut Vec<Problem>) -> Option<SvgSource> {
    let layers = color_layers(glyph, master_id);
    let static_layers = || layers.iter().filter(|(l, _)| l.info.attrs.coordinates.is_empty());
    if let Some((layer, _)) = static_layers().find(|(_, k)| *k == ColorLayerKind::Svg) {
        match layer.info.image.as_ref().and_then(|i| i.path.clone()) {
            Some(path) => return Some(SvgSource::SvgLayer { layer: layer.id.clone(), path }),
            None => problems.push(warning(Some(&glyph.name), "its svg layer has no SVG file, so it has no SVG document".to_string())),
        }
    }
    if let Some(paint) = vector {
        return Some(SvgSource::Paint(paint));
    }
    let wanted = switches.sbix_to_svg?;
    let sized: Vec<(u16, &str)> = static_layers()
        .filter(|(l, _)| l.info.image.as_ref().is_some_and(|i| i.path.is_some()))
        .filter_map(|(l, k)| match k {
            ColorLayerKind::IColor(size) => Some((*size, l.id.as_str())),
            _ => None,
        })
        .collect();
    let sizes: Vec<u16> = sized.iter().map(|(s, _)| *s).collect();
    let ppem = strike::pick_size(&sizes, wanted)?;
    let layer = sized.iter().find(|(s, _)| *s == ppem)?.1;
    Some(SvgSource::Bitmap { layer: layer.to_string(), ppem })
}

/// A warning for each palette row `paint` uses that the palettes do not have.
fn check_rows(glyph: &str, paint: &Paint, rows: usize, problems: &mut Vec<Problem>) {
    let mut used = Vec::new();
    palette_rows(paint, 0, &mut used);
    used.sort_unstable();
    used.dedup();
    for index in used.into_iter().filter(|&i| usize::from(i) >= rows) {
        problems.push(warning(Some(glyph), format!("it uses colour {index}, but the palettes have {rows} colours")));
    }
}

fn palette_rows(paint: &Paint, depth: usize, out: &mut Vec<u16>) {
    if depth > crate::MAX_PAINT_DEPTH {
        return;
    }
    let row = |c: &ColorRef| match c {
        ColorRef::Palette { index, .. } if *index != FOREGROUND_INDEX => Some(*index),
        _ => None,
    };
    match paint {
        Paint::Layers(list) => list.iter().for_each(|p| palette_rows(p, depth + 1, out)),
        Paint::Glyph { paint, .. } => palette_rows(paint, depth + 1, out),
        Paint::Solid(c) => out.extend(row(c)),
        Paint::Linear { line, .. } | Paint::Radial { line, .. } | Paint::Sweep { line, .. } => {
            out.extend(line.stops.iter().filter_map(|s| row(&s.color)))
        }
    }
}

/// `palettes` with palette `chosen` moved to the front (`color.export.colorPaletteForCpal`); a
/// palette the font does not have leaves the order alone, with a warning.
fn chosen_first(palettes: &Palettes, chosen: usize, problems: &mut Vec<Problem>) -> Palettes {
    if chosen == 0 || palettes.is_empty() {
        return palettes.clone();
    }
    let all = palettes.palettes();
    let Some(first) = all.get(chosen) else {
        problems
            .push(warning(None, format!("{COLOR_PALETTE_FOR_CPAL} names palette {chosen}, but the font has {}; palette 0 comes first", all.len())));
        return palettes.clone();
    };
    let mut order = vec![first.clone()];
    order.extend(all.iter().enumerate().filter(|(i, _)| *i != chosen).map(|(_, p)| p.clone()));
    Palettes::new(order).unwrap_or_else(|_| palettes.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Number, Str, Value};
    use fontcraft_model::{Font, Instance, Layer, LayerAttrs, Master, PaletteEntry, Param, Path, Shape, Target};

    use super::*;
    use crate::layers::with_kind;
    use crate::palette::to_param;

    const RED: Rgba = Rgba::new(255, 0, 0, 255);
    const BLUE: Rgba = Rgba::new(0, 0, 255, 255);
    const GREEN: Rgba = Rgba::new(0, 255, 0, 255);
    const GRAY: Rgba = Rgba::new(9, 9, 9, 255);

    fn colour_layer(id: &str, entry: PaletteEntry, drawn: bool) -> Layer {
        let mut l = Layer::new("m01", 600.0);
        l.id = id.to_string();
        l.info.attrs = with_kind(&LayerAttrs::default(), Some(ColorLayerKind::Palette(entry)));
        if drawn {
            l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            l.shapes.push(Shape::Path(Path::rect(200.0, 0.0, 300.0, 100.0)));
        }
        l
    }

    /// Two palettes of two colours; `A` has a layer in colour 1, one in the text colour and an
    /// empty one; `B` has none.
    fn palette_font() -> Font {
        let mut f = Font::new("Paint Sans");
        Arc::make_mut(&mut f.info).params.0.push(to_param(&Palettes::new(vec![vec![RED, BLUE], vec![GREEN, GRAY]]).unwrap()));
        let mut a = f.glyph_mut("A").unwrap();
        a.layers.push(colour_layer("A-c0", PaletteEntry::Index(1), true));
        a.layers.push(colour_layer("A-c1", PaletteEntry::Foreground, true));
        a.layers.push(colour_layer("A-empty", PaletteEntry::Index(0), false));
        drop(a);
        f
    }

    fn int(i: i64) -> Value {
        Value::Number(Number::int(i))
    }

    fn instance(name: &str, params: Vec<(&str, Value)>) -> Instance {
        let mut inst = Instance { name: name.into(), ..Default::default() };
        inst.params.0.extend(params.into_iter().map(|(n, v)| Param::new(n, v)));
        inst
    }

    fn solid(layer: &str, shapes: Vec<usize>, color: ColorRef) -> Paint {
        Paint::Glyph { shape: ShapeRef { layer: layer.into(), shapes, stroke: None }, paint: Box::new(Paint::Solid(color)) }
    }

    #[test]
    fn palette_layers_plan_cpal_and_colr_version_0() {
        let f = palette_font();
        let plan = plan_master(&f, "m01", &Switches::default());
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(plan.tables(), ["COLR", "CPAL"], "no SVG table without a conversion");
        let colr = plan.colr.unwrap();
        assert_eq!(colr.version, ColrVersion::V0);
        assert_eq!(colr.glyphs.len(), 1);
        assert_eq!(colr.glyphs[0].glyph, "A");
        // File order, the first layer at the bottom; the empty layer left out; `*` is the text
        // colour.
        let want = Paint::Layers(vec![
            solid("A-c0", vec![0, 1], ColorRef::Palette { index: 1, alpha: 1.0 }),
            solid("A-c1", vec![0, 1], ColorRef::Foreground { alpha: 1.0 }),
        ]);
        assert_eq!(colr.glyphs[0].paint, want);
        assert_eq!(crate::paint::shape_refs(&want).len(), 2);
        assert_eq!(plan.cpal.unwrap().palettes.palettes(), [vec![RED, BLUE], vec![GREEN, GRAY]]);

        // A master's own palettes win.
        let mut own = palette_font();
        Arc::make_mut(&mut own.masters)[0].params.0.push(to_param(&Palettes::new(vec![vec![GRAY, GRAY]]).unwrap()));
        assert_eq!(plan_master(&own, "m01", &Switches::default()).cpal.unwrap().palettes.palettes(), [vec![GRAY, GRAY]]);

        // A row the palettes do not have is exported, with a warning naming the glyph.
        let mut short = palette_font();
        Arc::make_mut(&mut short.info).params.0.retain(|p| p.name != palette::COLOR_PALETTES_PARAM);
        Arc::make_mut(&mut short.info).params.0.push(to_param(&Palettes::new(vec![vec![RED]]).unwrap()));
        let plan = plan_master(&short, "m01", &Switches::default());
        assert!(plan.colr.is_some());
        assert_eq!(plan.problems.iter().map(|p| p.glyph.as_deref()).collect::<Vec<_>>(), [Some("A")]);
    }

    #[test]
    fn instance_switches_choose_the_tables_and_the_palettes() {
        let mut f = palette_font();
        Arc::make_mut(&mut f.instances).extend([
            instance("Plain", vec![]),
            instance("Second", vec![(COLOR_PALETTE_FOR_CPAL, int(1))]),
            instance("No COLR", vec![(EXPORT_COLR_TABLE, Value::Bool(false))]),
            instance("SVG", vec![(COLOR_LAYERS_TO_SVG, Value::String(Str::new("1"))), (COLOR_PALETTE_FOR_SVG, int(1))]),
            instance("Missing", vec![(COLOR_PALETTE_FOR_CPAL, int(5))]),
        ]);
        let plain = plan(&f, &Target::Instance(0));
        assert_eq!(plain.cpal.unwrap().palettes.palettes(), [vec![RED, BLUE], vec![GREEN, GRAY]]);

        let second = plan(&f, &Target::Instance(1));
        assert_eq!(second.cpal.unwrap().palettes.palettes(), [vec![GREEN, GRAY], vec![RED, BLUE]], "every palette, the chosen first");

        let off = plan(&f, &Target::Instance(2));
        assert!(off.tables().is_empty(), "Export COLR Table off removes CPAL and COLR: {:?}", off.tables());

        let svg = plan(&f, &Target::Instance(3));
        assert_eq!(svg.tables(), ["COLR", "CPAL", "SVG "]);
        let svg = svg.svg.unwrap();
        assert_eq!(svg.palette, [GREEN, GRAY], "Color Palette for SVG bakes in palette 1");
        assert_eq!(svg.documents.len(), 1);
        assert!(matches!(&svg.documents[0].source, SvgSource::Paint(Paint::Layers(l)) if l.len() == 2));

        let missing = plan(&f, &Target::Instance(4));
        assert_eq!(missing.cpal.unwrap().palettes.palettes(), [vec![RED, BLUE], vec![GREEN, GRAY]]);
        assert_eq!(missing.problems.len(), 1, "{:?}", missing.problems);
        assert_eq!(missing.problems[0].kind, ProblemKind::Warning);

        // The instance's value wins over the font's.
        Arc::make_mut(&mut f.info).params.0.push(Param::new(EXPORT_COLR_TABLE, Value::Bool(false)));
        let mut on = instance("On", vec![(EXPORT_COLR_TABLE, int(1))]);
        on.params.0.push(Param::new(SBIX_TO_SVG, int(0)));
        Arc::make_mut(&mut f.instances).push(on);
        let switches = read_switches(&f, &Target::Instance(5)).unwrap();
        assert!(switches.export_colr);
        assert_eq!(switches.sbix_to_svg, None, "0 pixels per em converts nothing");
        assert!(!read_switches(&f, &Target::Instance(0)).unwrap().export_colr);
        assert!(read_switches(&f, &Target::Instance(99)).is_err());
    }

    #[test]
    fn an_instance_between_masters_with_colour_layers_is_refused() {
        let mut f = palette_font();
        Arc::make_mut(&mut f.instances).push(Instance { name: "Between".into(), location: vec![500.0], ..Default::default() });
        let refused = plan(&f, &Target::Instance(0));
        assert!(refused.refused() && refused.tables().is_empty());
        assert!(refused.problems[0].message.contains("Between"), "{}", refused.problems[0].message);

        // Nothing to refuse when no colour table would be written.
        Arc::make_mut(&mut f.instances)[0].params.0.push(Param::new(EXPORT_COLR_TABLE, Value::Bool(false)));
        assert_eq!(plan(&f, &Target::Instance(0)), ExportPlan::default());
        assert!(has_color_layers(&f) && !has_color_layers(&Font::new("Plain")));

        // A master layer marked full colour is a colour layer for the export.
        let mut full = Font::new("Full");
        full.glyph_mut("A").unwrap().layers[0].info.attrs.full_color = true;
        Arc::make_mut(&mut full.instances).push(Instance { name: "Between".into(), location: vec![500.0], ..Default::default() });
        assert!(has_color_layers(&full));
        assert!(plan(&full, &Target::Instance(0)).refused());
    }

    #[test]
    fn layered_masters_with_a_master_color_become_colr_layers_only_when_asked() {
        let mut f = Font::new("Layers");
        let masters = Arc::make_mut(&mut f.masters);
        masters[0].params.0.push(Param::new(MASTER_COLOR, Value::String(Str::new("255,0,0,255"))));
        masters.push(Master { id: "m02".into(), name: "Shade".into(), ..Default::default() });
        masters[1].params.0.push(Param::new(MASTER_COLOR, Value::String(Str::new("0,0,255,128"))));
        let mut a = f.glyph_mut("A").unwrap();
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        let mut shade = Layer::new("m02", 600.0);
        shade.shapes.push(Shape::Path(Path::rect(10.0, -10.0, 110.0, 90.0)));
        a.layers.push(shade);
        drop(a);

        // By default a layered font exports one plain font per instance.
        assert!(plan_master(&f, "m01", &Switches::default()).tables().is_empty());

        let switches = Switches { color_layers_to_colr: true, ..Switches::default() };
        let plan = plan_master(&f, "m01", &switches);
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        let colr = plan.colr.unwrap();
        assert_eq!(colr.version, ColrVersion::V0);
        let names: Vec<&str> = colr.glyphs.iter().map(|g| g.glyph.as_str()).collect();
        assert_eq!(names, [".notdef", "A"], "glyphs without outlines in any colour master have no colour");
        // The first master on top: painted last.
        let want = Paint::Layers(vec![
            solid("m02", vec![0], ColorRef::Rgba(Rgba::new(0, 0, 255, 128).to_f64())),
            solid("m01", vec![0], ColorRef::Rgba(RED.to_f64())),
        ]);
        assert_eq!(colr.glyphs[1].paint, want);
        assert!(plan.cpal.unwrap().palettes.is_empty(), "the master colours are added by the CPAL writer");
    }

    #[test]
    fn master_colors_read_in_every_stored_form() {
        let s = |t: &str| Value::String(Str::new(t));
        assert_eq!(master_color(&s("255,0,0,255")), Some(RED));
        assert_eq!(master_color(&s(" 0, 0, 255 ")), Some(BLUE));
        assert_eq!(master_color(&s("#00FF00")), Some(GREEN));
        assert_eq!(
            master_color(&Value::Array(fontcraft_model::plist::Array { items: vec![int(9), int(9), int(9), int(255)], ..Default::default() })),
            Some(GRAY)
        );
        for bad in ["", "red", "256,0,0", "1,2", "1,2,3,4,5", "nan,0,0"] {
            assert_eq!(master_color(&s(bad)), None, "{bad}");
        }
    }
}
