//! The variable font compile (M9.5, with M9.6 to M9.8 and M9.13).
//!
//! [`compile_variable`] builds one variable font for a variable font setting, or for the whole
//! designspace when the font has none (`export.variableUsesSettingsOrDefault`):
//!
//! | Stage | Where | Package |
//! |---|---|---|
//! | Designspace and preflight | `fontcraft_interp::Designspace::for_setting`, the compatibility check (`compatibility.exportBlocked`) | M9.5 |
//! | Glyph order | the static order plus the alternate-layer glyphs ([`featvar::with_bracket_glyphs`]) | M9.5, M9.8 |
//! | Decomposition | [`decompose`]: inconsistent component transforms, the decomposition parameters | M9.5 |
//! | Outlines | [`glyf`]: cubic to quadratic across sources, `glyf` from the origin; [`gvar`] with phantom points, IUP and shared tuples | M9.5 |
//! | Interpolation curves | [`hoi`]: sources along the spans of glyphs whose nodes keep curves | M9.33 |
//! | Axes and names | [`fvar`], [`avar`], [`names`] | M9.6 |
//! | Metrics variations | [`hvar`] (`HVAR`, `VVAR`), [`mvar`] | M9.6, M9.38 |
//! | Style attributes | [`stat`] | M9.7 |
//! | Feature variations | [`featvar`]: alternate layers as `GSUB` feature variations, merged into the feature code's `GSUB` by the layout stage | M9.8 |
//! | Variable layout | `fontcraft_features::var_layout`: kerning and marks with deltas | M9.9 |
//! | CFF2 | [`cff2`] | M9.13 |
//! | Interpolatable masters | [`interpolatable`]: the outline stage's result written as one font per master | M2.9 |
//! | Hinting variations | [`cvar`]: the masters' TrueType control values, once the table stage knows the `cvt ` | M10.19 |
//!
//! Contract: each table builder takes plain data ([`VarAxes`], per-glyph source outlines, per
//! master values as [`VarValue`]s, a name allocator), not the font, so it can be tested alone.
//! Building from the font is this module's job. Variable fonts always have TrueType outlines
//! unless the command line or `file.exportVariable` asks for CFF2. Instances outside the axis
//! ranges are left out of `fvar`. Ported code (fontc's `stat.rs`, its `gvar`, `HVAR`, `MVAR` and
//! `avar` assembly, fontTools' feature-variation overlay) carries its credit in the file header
//! and in `NOTICE`.

pub mod avar;
pub mod cff2;
pub mod cvar;
pub mod decompose;
pub mod featvar;
pub mod fvar;
pub mod glyf;
pub mod gvar;
pub mod hoi;
pub mod hvar;
pub mod interpolatable;
pub mod mvar;
pub mod names;
pub mod stat;

use std::collections::{HashMap, HashSet};

use fontcraft_geom::BezPath;
use fontcraft_model::{Font, ResolvedGlyph, Target};
use write_fonts::tables::glyf::SimpleGlyph;
use write_fonts::types::{NameId, Tag};

pub use fontcraft_features::var_layout::VarValue;
use fontcraft_features::var_layout::{VarLayout, VarMaster};
pub use fontcraft_geom::variation::{Region, RegionAxis};

use crate::outline::Prepared;
use crate::{CompileError, Compiled, Flavour, Options, ot_round};

/// Most glyphs one composite reaches through its components when its intermediate sources are
/// gathered.
const MAX_COMPONENT_CLOSURE: usize = 1024;

/// Which outlines a variable font gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VarFlavour {
    /// `glyf` and `gvar` (`variable.alwaysTrueTypeFlavour`): the export dialog's only choice.
    #[default]
    TrueType,
    /// `CFF2`, from the command line and `file.exportVariable` (M9.13).
    Cff2,
}

/// Options of [`compile_variable`] beyond the static compile's [`Options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableOptions {
    pub flavour: VarFlavour,
    /// Optimize `gvar` deltas with IUP; `None` follows the font's Optimize Variable Deltas
    /// parameter (on by default).
    pub optimize_deltas: Option<bool>,
}

/// One axis of a variable font, as `fvar` and `avar` hold it.
#[derive(Clone, Debug, PartialEq)]
pub struct VarAxis {
    pub tag: Tag,
    pub name: String,
    /// Minimum, default and maximum in external (user) coordinates.
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub hidden: bool,
    /// The `avar` segment map: `(normalized external, normalized internal)` pairs including
    /// (−1, −1), (0, 0) and (1, 1); empty for identity.
    pub avar: Vec<(f64, f64)>,
}

/// The axes of a variable font, in `fvar` order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VarAxes {
    pub axes: Vec<VarAxis>,
}

impl VarAxes {
    /// The axes of `designspace`: tags, names, external ranges and normalized maps.
    pub fn from_designspace(designspace: &fontcraft_interp::Designspace) -> Result<VarAxes, CompileError> {
        // M9.6
        let axes = designspace
            .axes()
            .iter()
            .enumerate()
            .map(|(k, a)| {
                let tag = Tag::new_checked(a.tag.as_bytes()).map_err(|_| fontcraft_interp::InterpError::BadAxisTag(a.tag.clone()))?;
                let (min, default, max) = (a.external.minimum, a.external.default, a.external.maximum);
                if !(min.is_finite() && default.is_finite() && max.is_finite() && min <= default && default <= max) {
                    return Err(CompileError::Table { table: "fvar", message: format!("axis `{}` has no usable range", a.tag) });
                }
                Ok(VarAxis { tag, name: a.name.clone(), min, default, max, hidden: a.hidden, avar: avar::segment_map(designspace, k) })
            })
            .collect::<Result<Vec<_>, CompileError>>()?;
        Ok(VarAxes { axes })
    }
}

/// A named instance of a variable font (`fvar`).
#[derive(Clone, Debug, PartialEq)]
pub struct VarInstance {
    /// The subfamily name.
    pub name: String,
    pub postscript_name: Option<String>,
    /// External coordinates, parallel to [`VarAxes::axes`].
    pub coordinates: Vec<f64>,
}

/// A built variable font.
#[derive(Clone, Debug, PartialEq)]
pub struct VariableFont {
    pub compiled: Compiled,
    /// The variable font setting built (an index into `Font::instances`); `None` for the default
    /// font of the whole designspace.
    pub setting: Option<usize>,
    /// The file name without extension (Variable Font File Name, or the reference's default).
    pub file_stem: String,
}

/// The variable fonts `font` exports: each variable font setting, or one default font when there
/// is none.
pub fn variable_targets(font: &Font) -> Vec<Option<usize>> {
    let settings = fontcraft_interp::designspace::variable_settings(font);
    if settings.is_empty() { vec![None] } else { settings.into_iter().map(Some).collect() }
}

/// Most distinct intermediate-layer locations one variable font is built from; each is drawn as a
/// whole font interpolated there.
pub const MAX_INTERMEDIATE_LOCATIONS: usize = 64;
/// Most incompatible glyphs a refusal names.
const MAX_NAMED_GLYPHS: usize = 10;

/// Parameters of a variable font setting that the setting itself uses rather than passing on to
/// the font.
const SETTING_ONLY: &[&str] = &[
    fontcraft_interp::params::DISABLE_MASTERS,
    fontcraft_interp::params::AXIS_MAPPINGS,
    names::VARIABLE_FAMILY_NAME,
    names::VARIABLE_FILE_NAME,
    names::VARIABLE_STYLE_NAME,
    names::VARIABLE_SUBFAMILY_NAME,
    names::VARIABLE_POSTSCRIPT_NAME,
    names::VARIATIONS_POSTSCRIPT_PREFIX,
    names::FILE_NAME,
];

/// `font` with the parameters of the variable font setting at `setting` in place of the font's
/// own of the same name (`variableFontSetting.wholeFontParameters`).
pub(crate) fn setting_font(font: &Font, setting: Option<usize>) -> std::borrow::Cow<'_, Font> {
    let Some(inst) = setting.and_then(|s| font.instances.get(s)) else { return std::borrow::Cow::Borrowed(font) };
    let passed: Vec<&fontcraft_model::Param> = inst.params.0.iter().filter(|p| !p.disabled && !SETTING_ONLY.contains(&p.name.as_str())).collect();
    if passed.is_empty() {
        return std::borrow::Cow::Borrowed(font);
    }
    let mut out = font.clone();
    let info = std::sync::Arc::make_mut(&mut out.info);
    info.params.0.retain(|p| !passed.iter().any(|q| q.name == p.name));
    info.params.0.extend(passed.into_iter().cloned());
    std::borrow::Cow::Owned(out)
}

/// One source of the variable font other than the origin: a master, or a location where some
/// glyph has an intermediate layer.
struct Source {
    /// The master, for a master source.
    master: Option<String>,
    /// Internal coordinates.
    location: fontcraft_interp::Location,
    normalized: Vec<f64>,
    glyphs: Vec<ResolvedGlyph>,
    index: HashMap<String, usize>,
    /// The global metrics there (`MVAR` for a master source, the vertical metrics of glyphs that
    /// state none).
    metrics: crate::info::GlobalMetrics,
}

impl Source {
    fn glyph(&self, name: &str) -> Option<(usize, &ResolvedGlyph)> {
        let &i = self.index.get(name)?;
        Some((i, self.glyphs.get(i)?))
    }
}

/// The glyphs of `prepared` at `target` after every stage before the outlines, with the global
/// metrics there.
fn source_glyphs(
    font: &Font,
    prepared: &Font,
    target: &Target,
    flavour: Flavour,
    options: &Options,
) -> Result<(Vec<ResolvedGlyph>, crate::info::GlobalMetrics), CompileError> {
    let mut resolved = crate::drawing::resolve(prepared, target)?;
    let glyphs = crate::glyph_stages(Some(font), &mut resolved, flavour, options, &mut Vec::new())?;
    Ok((glyphs, crate::info::derive(&resolved, &options.style_name, &mut Vec::new()).metrics))
}

/// Reads one global metric.
type Metric = fn(&crate::info::GlobalMetrics) -> f64;

/// The `MVAR` value tags and the metric each one varies.
const MVAR_TAGS: [(&[u8; 4], Metric); 27] = [
    (b"cpht", |m| m.cap_height),
    (b"hasc", |m| m.typo_ascender),
    (b"hcla", |m| m.win_ascent),
    (b"hcld", |m| m.win_descent),
    (b"hcof", |m| m.caret_offset),
    (b"hcrn", |m| m.caret_slope_run),
    (b"hcrs", |m| m.caret_slope_rise),
    (b"hdsc", |m| m.typo_descender),
    (b"hlgp", |m| m.typo_line_gap),
    (b"sbxo", |m| m.subscript_x_offset),
    (b"sbxs", |m| m.subscript_x_size),
    (b"sbyo", |m| m.subscript_y_offset),
    (b"sbys", |m| m.subscript_y_size),
    (b"spxo", |m| m.superscript_x_offset),
    (b"spxs", |m| m.superscript_x_size),
    (b"spyo", |m| m.superscript_y_offset),
    (b"spys", |m| m.superscript_y_size),
    (b"stro", |m| m.strikeout_position),
    (b"strs", |m| m.strikeout_size),
    (b"undo", |m| m.underline_position),
    (b"unds", |m| m.underline_thickness),
    (b"vasc", |m| m.vhea_ascender),
    (b"vcof", |m| m.vhea_caret_offset),
    (b"vcrn", |m| m.vhea_caret_slope_run),
    (b"vcrs", |m| m.vhea_caret_slope_rise),
    (b"vdsc", |m| m.vhea_descender),
    (b"xhgt", |m| m.x_height),
];

/// Keeps the invisible open corners of the glyphs whose sources do not all have the same ones.
fn keep_uneven_corners(
    font: &Font,
    origin: &str,
    sources: &HashMap<String, fontcraft_interp::GlyphSources>,
    options: &mut Options,
    notes: &mut Vec<String>,
) {
    let kept = crate::components_opt::uneven_open_corners(font, origin, sources.values(), &options.components);
    if !kept.is_empty() {
        let mut names: Vec<&str> = kept.iter().map(String::as_str).collect();
        names.sort_unstable();
        names.truncate(MAX_NAMED_GLYPHS);
        notes.push(format!("open corners were kept in glyphs whose masters do not all have the same ones: {}", names.join(", ")));
    }
    options.components.keep_open_corners = kept;
}

/// The `MVAR` values of the masters: the origin's metrics first, then each master source's.
fn mvar_values(origin: &crate::info::GlobalMetrics, sources: &[Source]) -> Result<Vec<(Tag, VarValue)>, CompileError> {
    // Masters that share a location (or the origin's) give one value.
    let mut masters: Vec<(&[f64], &crate::info::GlobalMetrics)> = Vec::new();
    for s in sources {
        if s.master.is_none() {
            continue;
        }
        if s.normalized.iter().any(|v| *v != 0.0) && !masters.iter().any(|(l, _)| *l == s.normalized.as_slice()) {
            masters.push((s.normalized.as_slice(), &s.metrics));
        }
    }
    let mut locations = vec![vec![0.0; masters.first().map_or(0, |m| m.0.len())]];
    locations.extend(masters.iter().map(|m| m.0.to_vec()));
    let mut models = gvar::Models::default();
    let bad = |e: String| CompileError::Table { table: "MVAR", message: e };
    let model = models.get(&locations).map_err(bad)?;
    let mut out = Vec::with_capacity(MVAR_TAGS.len());
    for (tag, get) in MVAR_TAGS {
        let mut values = vec![vec![ot_round(get(origin))]];
        values.extend(masters.iter().map(|m| vec![ot_round(get(m.1))]));
        let mut deltas = Vec::new();
        for (region, d) in gvar::region_deltas(model, &values).map_err(bad)? {
            let d = d.first().copied().unwrap_or(0.0);
            if !gvar::fits_i16(d) {
                return Err(bad(format!("a delta of {d} units for `{}` does not fit the table", String::from_utf8_lossy(tag))));
            }
            if d != 0.0 {
                deltas.push((region, d as i32));
            }
        }
        out.push((Tag::new(tag), VarValue { default: ot_round(get(origin)) as i32, deltas }));
    }
    Ok(out)
}

/// `font` interpolated at `location` (internal coordinates of `designspace`), as a font with one
/// master there.
fn font_at(font: &Font, designspace: &fontcraft_interp::Designspace, location: &fontcraft_interp::Location) -> Result<Font, CompileError> {
    let origin =
        designspace.origin().and_then(|o| font.masters.get(o.index)).ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
    let mut at = origin.location.clone();
    for (axis, v) in designspace.axes().iter().zip(&location.0) {
        if at.len() <= axis.font_axis {
            at.resize(axis.font_axis + 1, 0.0);
        }
        if let Some(slot) = at.get_mut(axis.font_axis) {
            *slot = *v;
        }
    }
    let mut probe = font.clone();
    std::sync::Arc::make_mut(&mut probe.instances).push(fontcraft_model::Instance {
        name: "intermediate".into(),
        location: at,
        exports: false,
        ..Default::default()
    });
    let index = probe.instances.len() - 1;
    let options = fontcraft_interp::instance::options_for(&probe, index)?;
    Ok(fontcraft_interp::instance::instance_font(&probe, index, &options)?)
}

/// A glyph's vertical phantom points at one source: the y of its vertical origin and of the
/// bottom of its vertical advance, both 0 in a font without vertical metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Vertical {
    top: f64,
    bottom: f64,
}

impl Vertical {
    /// From a layer's own vertical width and origin, each falling back to `metrics`, with the
    /// values `vmtx` would write (`compile.verticalMetrics`).
    fn new(width: Option<f64>, origin: Option<f64>, metrics: &crate::info::GlobalMetrics) -> Vertical {
        let height = crate::u16_of(width.unwrap_or(metrics.typo_ascender - metrics.typo_descender));
        let top = crate::i16_of(origin.map_or(metrics.typo_ascender, |o| metrics.typo_ascender - o));
        Vertical { top: f64::from(top), bottom: f64::from(top) - f64::from(height) }
    }
}

/// The phantom points of a glyph of horizontal advance `advance`: left and right, top and bottom.
fn phantoms(advance: f64, vertical: Vertical) -> [(f64, f64); gvar::PHANTOM_POINTS] {
    [(0.0, 0.0), (ot_round(advance), 0.0), (0.0, vertical.top), (0.0, vertical.bottom)]
}

/// The phantom points at the end of a glyph's point list (zeros when it is too short).
fn phantoms_of(points: &[(f64, f64)]) -> [(f64, f64); gvar::PHANTOM_POINTS] {
    points.last_chunk().copied().unwrap_or_default()
}

/// The y of the vertical origin, which a CFF2 font keeps in `VORG` and `VVAR`.
fn vertical_origin(points: &[(f64, f64)]) -> f64 {
    let [_, _, top, _] = phantoms_of(points);
    top.1
}

fn vertical_advance(points: &[(f64, f64)]) -> f64 {
    let [_, _, top, bottom] = phantoms_of(points);
    top.1 - bottom.1
}

fn horizontal_advance(points: &[(f64, f64)]) -> f64 {
    let [_, right, _, _] = phantoms_of(points);
    right.0
}

/// The origin master's id, and the axes and masters feature code is compiled with, in the variable font of the whole
/// designspace: for checking the code on its own. Refused where [`compile_variable`] refuses the designspace.
pub fn feature_layout(font: &Font) -> Result<(String, VarLayout), CompileError> {
    let designspace = fontcraft_interp::Designspace::new(font)?;
    if !designspace.axes().iter().any(|a| a.external.minimum < a.external.maximum) || designspace.masters().len() < 2 {
        return Err(CompileError::Table { table: "fvar", message: "a variable font needs two masters and an axis with a range".into() });
    }
    designspace.check_variable_origin()?;
    let origin = designspace.origin().cloned().ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
    let axes: Vec<Tag> = VarAxes::from_designspace(&designspace)?.axes.into_iter().map(|a| a.tag).collect();
    let mut masters = vec![VarMaster { id: origin.id.clone(), location: vec![0.0; axes.len()] }];
    for m in designspace.masters().iter().filter(|m| m.id != origin.id) {
        masters.push(VarMaster { id: m.id.clone(), location: designspace.to_normalized(&m.location)?.0 });
    }
    let layout = VarLayout::new(axes, masters).map_err(|e| CompileError::Table { table: "GPOS", message: e.to_string() })?;
    Ok((origin.id, layout))
}

/// Compiles the variable font of `setting` (an index into `Font::instances`; `None` for the
/// whole designspace). Refuses incompatible glyphs, a designspace without a usable origin, and
/// everything the static compile refuses.
pub fn compile_variable(font: &Font, setting: Option<usize>, options: &Options, var: &VariableOptions) -> Result<VariableFont, CompileError> {
    font.check()?;
    let flavour = if var.flavour == VarFlavour::Cff2 { Flavour::Cff } else { Flavour::TrueType };
    let mut notes: Vec<String> = Vec::new();
    let font = setting_font(font, setting);
    let font: &Font = &font;
    let designspace = variable_designspace(font, setting)?;
    notes.extend(designspace.warnings().iter().cloned());
    // Layers left out of interpolation are no sources of their glyph; where another glyph draws
    // it at that master, its interpolated drawing stands in.
    let linear = fontcraft_interp::InterpOptions { hoi: false, ..fontcraft_interp::InterpOptions::default() };
    let drawn = fontcraft_interp::sources::with_inactive_interpolated(font, &designspace, &linear);
    let font: &Font = drawn.as_ref().unwrap_or(font);
    let origin = designspace.origin().cloned().ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
    let axis_count = u16::try_from(designspace.axes().len()).map_err(|_| CompileError::Table { table: "fvar", message: "too many axes".into() })?;

    refuse_incompatible(font, &designspace)?;

    let mut options = options.clone();
    options.remove_overlap = false;
    if options.glyph_data.0.is_none()
        && let Ok(data) = fontcraft_model::glyphdata::GlyphData::new()
    {
        options.glyph_data = crate::DataRef(Some(std::sync::Arc::new(data)));
    }
    let vnames = names::variable_names(font, setting)?;
    options.style_name.clone_from(&vnames.style);

    // Alternate layers: their glyphs join the font as glyphs of their own, and their ranges
    // become feature variations of the layout stage.
    let aligned = featvar::align_alternate_layers(font, &mut notes)?;
    let font: &Font = aligned.as_ref().unwrap_or(font);
    let alternates = featvar::AlternateRules::of(font, &designspace, &mut notes)?;
    let with_brackets = featvar::with_bracket_glyphs(font, &designspace)?;
    let font: &Font = with_brackets.as_ref().unwrap_or(font);

    // Colour layer glyphs become glyphs of the font here, so they vary as any glyph does.
    let switches = fontcraft_color::export::read_switches(font, &Target::Master(origin.id.clone())).unwrap_or_default();
    let split = crate::color::variable::split_layer_glyphs(font, &designspace, &switches, &mut notes)?;
    let prepared = crate::prepare_source(split.as_ref().unwrap_or(font), &mut notes);
    let prepared: &Font = &prepared;
    let mut glyph_sources: HashMap<String, fontcraft_interp::GlyphSources> = HashMap::new();
    for g in prepared.glyphs.iter() {
        crate::check_stop()?;
        glyph_sources.insert(g.name.clone(), fontcraft_interp::GlyphSources::of(prepared, &designspace, &g.name)?);
    }

    keep_uneven_corners(prepared, &origin.id, &glyph_sources, &mut options, &mut notes);

    // The sources besides the origin: the other masters, then each intermediate location.
    let mut sources: Vec<Source> = Vec::new();
    let mut add = |master: Option<String>,
                   location: fontcraft_interp::Location,
                   (glyphs, metrics): (Vec<ResolvedGlyph>, crate::info::GlobalMetrics)|
     -> Result<(), CompileError> {
        let normalized = designspace.to_normalized(&location)?.0;
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        sources.push(Source { master, location, normalized, glyphs, index, metrics });
        Ok(())
    };
    for m in designspace.masters().iter().filter(|m| m.id != origin.id) {
        crate::check_stop()?;
        add(Some(m.id.clone()), m.location.clone(), source_glyphs(font, prepared, &Target::Master(m.id.clone()), flavour, &options)?)?;
    }
    let samples = hoi::Samples::plan(prepared, &designspace, &glyph_sources, &mut notes)?;
    let mut intermediate: Vec<fontcraft_interp::Location> = Vec::new();
    for s in glyph_sources.values().flat_map(|g| g.main.iter()).filter(|s| !s.is_master) {
        if !intermediate.contains(&s.location) {
            if intermediate.len() >= MAX_INTERMEDIATE_LOCATIONS {
                return Err(
                    fontcraft_interp::InterpError::LimitExceeded { what: "intermediate layer locations", limit: MAX_INTERMEDIATE_LOCATIONS }.into()
                );
            }
            intermediate.push(s.location.clone());
        }
    }
    intermediate.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut drawn: Vec<fontcraft_interp::Location> = Vec::with_capacity(intermediate.len());
    for location in intermediate {
        crate::check_stop()?;
        // An intermediate layer beyond the axes of this font (a setting that disables masters
        // can narrow them) is no source of it.
        if designspace.to_normalized(&location)?.0.iter().any(|v| !(-1.0..=1.0).contains(v)) {
            notes.push(format!("intermediate layers at {:?} lie outside the axes of this variable font and were left out", location.0));
            continue;
        }
        let mut at = font_at(prepared, &designspace, &location)?;
        samples.place(&mut at, &location);
        let target = at.default_target()?;
        drawn.push(location.clone());
        add(None, location, source_glyphs(&at, &at, &target, flavour, &options)?)?;
    }
    // Where only curves need a source, only the glyphs they reach are drawn.
    let curved: Vec<fontcraft_interp::Location> = samples.locations.iter().filter(|l| !drawn.contains(*l)).cloned().collect();
    if !curved.is_empty() {
        let reduced = samples.reduced(prepared);
        for location in curved {
            let mut at = font_at(&reduced, &designspace, &location)?;
            samples.place(&mut at, &location);
            let target = at.default_target()?;
            add(None, location, source_glyphs(&at, &at, &target, flavour, &options)?)?;
        }
    }

    // The masters the generated kerning and mark positions vary with: the origin, then each
    // other master source.
    let layout_axes: Vec<Tag> = VarAxes::from_designspace(&designspace)?.axes.into_iter().map(|a| a.tag).collect();
    let mut layout_masters = vec![VarMaster { id: origin.id.clone(), location: vec![0.0; layout_axes.len()] }];
    layout_masters.extend(sources.iter().filter_map(|s| Some(VarMaster { id: s.master.clone()?, location: s.normalized.clone() })));
    let var_layout = VarLayout::new(layout_axes, layout_masters).map_err(|e| CompileError::Table { table: "GPOS", message: e.to_string() })?;

    let variable_layout = crate::layout::VariableLayout {
        layout: &var_layout,
        masters: sources
            .iter()
            .filter_map(|s| {
                Some(crate::layout::OtherMaster { id: s.master.as_deref()?, glyphs: s.glyphs.iter().map(|g| (g.name.as_str(), g)).collect() })
            })
            .collect(),
        alternates: alternates.as_ref(),
    };

    let mut resolved = crate::drawing::resolve(prepared, &Target::Master(origin.id.clone()))?;
    resolved.family_name.clone_from(&vnames.family);
    let gvar_options = gvar::GvarOptions { optimize: var.optimize_deltas.unwrap_or_else(|| optimize_deltas(font)) };
    let static_names = static_names(&resolved, &options.style_name);
    // The masters `cvar` varies with: each master source and its normalized location.
    let cvar_masters: Vec<(String, Vec<f64>)> = sources.iter().filter_map(|s| Some((s.master.clone()?, s.normalized.clone()))).collect();
    let ctx = Outlines {
        designspace: &designspace,
        prepared,
        origin: &origin.id,
        sources: &sources,
        glyph_sources: &glyph_sources,
        samples: &samples,
        flavour,
        keep_implied: false,
    };
    let mut stage = |origin_resolved: &fontcraft_model::Resolved,
                     glyphs: &[ResolvedGlyph],
                     settings: &crate::outline::Settings,
                     warnings: &mut Vec<String>|
     -> Result<(Vec<Prepared>, crate::Extra), CompileError> {
        let programs = flavour == Flavour::TrueType && crate::tt_hints::compiles_glyph_programs(prepared, origin_resolved, &options);
        let info = crate::info::derive(origin_resolved, &options.style_name, &mut Vec::new());
        let vertical = info.vertical.then_some(&info.metrics);
        let (outlines, per_glyph, cubic) = ctx.build(glyphs, settings, programs, vertical, warnings)?;
        let mut extra = crate::Extra::default();
        if flavour == Flavour::Cff {
            let masters = cff2::master_privates(prepared, origin_resolved, &sources, warnings)?;
            let ids: Vec<String> = designspace.masters().iter().map(|m| m.id.clone()).collect();
            let hints = cff2::HintContext::new(prepared, &ids, &options.hinting, warnings)?;
            let cff2_font = cff2::cff2_font(axis_count, &cubic, &masters, origin_resolved.units_per_em, hints.as_ref(), warnings)?;
            extra.cff2 = Some(cff2::write(&cff2_font, warnings)?);
        } else {
            let gvar = gvar::build_gvar(axis_count, &per_glyph, &gvar_options)?;
            extra.cvar = Some(cvar::CvarMasters { axis_count, origin: origin_resolved.master.clone(), masters: cvar_masters.clone() });
            extra.tables.push((
                Tag::new(b"gvar"),
                write_fonts::dump_table(&gvar).map_err(|e| CompileError::Table { table: "gvar", message: e.to_string() })?,
            ));
        }
        let advances = metric_values(&per_glyph, "HVAR", horizontal_advance)?;
        let axes = VarAxes::from_designspace(&designspace)?;
        let NameTables { fvar, stat, names, .. } = name_tables(font, &designspace, &axes, &vnames, static_names.clone())?;
        let dump = |tag: &'static str, r: Result<Vec<u8>, write_fonts::error::Error>| {
            r.map_err(|e| CompileError::Table { table: tag, message: e.to_string() })
        };
        extra.tables.push((Tag::new(b"fvar"), dump("fvar", write_fonts::dump_table(&fvar))?));
        if let Some(avar) = avar::build_avar(&axes)? {
            extra.tables.push((Tag::new(b"avar"), dump("avar", write_fonts::dump_table(&avar))?));
        }
        if advances.iter().any(|a| !a.deltas.is_empty()) {
            extra.tables.push((Tag::new(b"HVAR"), dump("HVAR", write_fonts::dump_table(&hvar::build_hvar(axis_count, &advances)?))?));
        }
        if vertical.is_some() {
            let heights = metric_values(&per_glyph, "VVAR", vertical_advance)?;
            let origins = (flavour == Flavour::Cff).then(|| metric_values(&per_glyph, "VVAR", vertical_origin)).transpose()?;
            let vvar = hvar::build_vvar(axis_count, &heights, origins.as_deref())?;
            extra.tables.push((Tag::new(b"VVAR"), dump("VVAR", write_fonts::dump_table(&vvar))?));
        }
        if let Some(mvar) = mvar::build_mvar(axis_count, &mvar_values(&info.metrics, &sources)?)? {
            extra.tables.push((Tag::new(b"MVAR"), dump("MVAR", write_fonts::dump_table(&mvar))?));
        }
        if let Some(stat) = stat {
            extra.tables.push((Tag::new(b"STAT"), dump("STAT", write_fonts::dump_table(&stat))?));
        }
        extra.names = names;
        Ok((outlines, extra))
    };
    let mut compiled = crate::compile_stages_with(Some(font), resolved, flavour, &options, Some(&mut stage), Some(&variable_layout))?;
    crate::extras::finish(Some(font), &mut compiled, &options)?;
    notes.append(&mut compiled.warnings);
    compiled.warnings = notes;
    Ok(VariableFont { compiled, setting, file_stem: vnames.file_stem })
}

/// Preflight (`compatibility.exportBlocked`): every exported glyph is compatible in every master
/// of `designspace`, or the compile is refused, naming the glyphs.
fn refuse_incompatible(font: &Font, designspace: &fontcraft_interp::Designspace) -> Result<(), CompileError> {
    let mut incompatible: Vec<String> = incompatible(font, designspace)?.into_iter().map(|(glyph, _)| glyph).collect();
    if incompatible.is_empty() {
        return Ok(());
    }
    let count = incompatible.len();
    incompatible.truncate(MAX_NAMED_GLYPHS);
    let more = if count > MAX_NAMED_GLYPHS { format!(" and {} more", count - MAX_NAMED_GLYPHS) } else { String::new() };
    Err(CompileError::Incompatible(format!("{}{more}", incompatible.join(", "))))
}

/// The exported glyphs that are not compatible in every master of `designspace`, each with the
/// differences found first.
fn incompatible(font: &Font, designspace: &fontcraft_interp::Designspace) -> Result<Vec<(String, Vec<fontcraft_interp::Mismatch>)>, CompileError> {
    let scope = fontcraft_interp::MasterPairs { masters: designspace.masters().iter().map(|m| m.id.clone()).collect(), indicators: true };
    let mut out = Vec::new();
    for g in font.glyphs.iter().filter(|g| g.export) {
        let sources = fontcraft_interp::GlyphSources::of(font, designspace, &g.name)?;
        let checked = fontcraft_interp::compat::check_glyph(font, &sources, &scope);
        if !checked.ok {
            out.push((g.name.clone(), checked.first_mismatch));
        }
    }
    Ok(out)
}

/// The exported glyphs whose layers keep the variable font of `setting` (an index into
/// `Font::instances`; `None` for the whole designspace) from being made, each with the differences
/// found first: what [`compile_variable`] refuses the font for.
pub fn incompatible_glyphs(font: &Font, setting: Option<usize>) -> Result<Vec<(String, Vec<fontcraft_interp::Mismatch>)>, CompileError> {
    font.check()?;
    let font = setting_font(font, setting);
    let font: &Font = &font;
    let designspace = match setting {
        Some(s) => fontcraft_interp::Designspace::for_setting(font, s)?,
        None => fontcraft_interp::Designspace::new(font)?,
    };
    let linear = fontcraft_interp::InterpOptions { hoi: false, ..fontcraft_interp::InterpOptions::default() };
    let drawn = fontcraft_interp::sources::with_inactive_interpolated(font, &designspace, &linear);
    incompatible(drawn.as_ref().unwrap_or(font), &designspace)
}

/// The designspace of the variable font of `setting`, refused when no variable font can be built from it.
pub(crate) fn variable_designspace(font: &Font, setting: Option<usize>) -> Result<fontcraft_interp::Designspace, CompileError> {
    let designspace = match setting {
        Some(s) => fontcraft_interp::Designspace::for_setting(font, s)?,
        None => fontcraft_interp::Designspace::new(font)?,
    };
    if !designspace.axes().iter().any(|a| a.external.minimum < a.external.maximum) || designspace.masters().len() < 2 {
        return Err(CompileError::Table { table: "fvar", message: "a variable font needs two masters and an axis with a range".into() });
    }
    designspace.check_variable_origin()?;
    Ok(designspace)
}

/// The English Windows names of the static tables of `resolved`, its tokens as written: `fvar` and `STAT` reuse their IDs.
pub(crate) fn static_names(resolved: &fontcraft_model::Resolved, style: &str) -> Vec<(NameId, String)> {
    crate::info::derive(resolved, style, &mut Vec::new())
        .names
        .into_iter()
        .filter(|r| r.platform == 3 && r.language == 0x409)
        .map(|r| (NameId::new(r.id), r.text))
        .collect()
}

/// The tables of a variable font that name its instances and axis values, and what they were built from.
pub(crate) struct NameTables {
    pub fvar: write_fonts::tables::fvar::Fvar,
    pub stat: Option<write_fonts::tables::stat::Stat>,
    pub instances: Vec<VarInstance>,
    pub stat_input: Option<stat::StatInput>,
    /// The records they add to the `name` table ([`crate::info::add_names`]), with the font's own PostScript names.
    pub names: Vec<(u16, String)>,
}

pub(crate) fn name_tables(
    font: &Font,
    designspace: &fontcraft_interp::Designspace,
    axes: &VarAxes,
    vnames: &names::VariableNames,
    static_names: Vec<(NameId, String)>,
) -> Result<NameTables, CompileError> {
    let instances = fvar::named_instances(font, designspace)?;
    let mut allocator = names::NameAllocator::with_static(static_names);
    let fvar = fvar::build_fvar(axes, &instances, &mut allocator)?;
    let stat_input = stat::stat_input(font, designspace)?;
    let stat = stat_input.as_ref().map(|input| stat::build_stat(input, &mut allocator)).transpose()?;
    let mut names: Vec<(u16, String)> = allocator.records().iter().map(|(id, text)| (id.to_u16(), text.clone())).collect();
    if let Some(ps) = &vnames.postscript_name {
        names.push((6, ps.clone()));
    }
    if let Some(prefix) = &vnames.postscript_prefix {
        names.push((25, prefix.clone()));
    }
    Ok(NameTables { fvar, stat, instances, stat_input, names })
}

/// Optimize Variable Deltas, or its second name; on unless one is off.
fn optimize_deltas(font: &Font) -> bool {
    const NAMES: [&str; 2] = ["Optimize Variable Deltas", "Variable Font Optimize Deltas"];
    font.info
        .params
        .0
        .iter()
        .filter(|p| !p.disabled && NAMES.contains(&p.name.as_str()))
        .filter_map(|p| fontcraft_model::param_flag(&p.value))
        .all(|on| on)
}

/// The outlines to write, each glyph's points at its sources, and (CFF2 only) its cubic
/// sources.
type BuiltOutlines = (Vec<Prepared>, Vec<gvar::SourceOutlines>, Vec<cff2::CubicSources>);

/// What the outline stage of a variable compile reads.
struct Outlines<'a> {
    designspace: &'a fontcraft_interp::Designspace,
    prepared: &'a Font,
    origin: &'a str,
    sources: &'a [Source],
    glyph_sources: &'a HashMap<String, fontcraft_interp::GlyphSources>,
    samples: &'a hoi::Samples,
    /// `Cff` for a CFF2 font: every composite decomposed, cubic outlines kept.
    flavour: Flavour,
    /// Keep the on-curve points of the source that could be implied ([`interpolatable`]).
    keep_implied: bool,
}

impl Outlines<'_> {
    /// The layer source `slot` of glyph `name` is drawn from (0 the origin, k + 1 source k), for
    /// its hints.
    fn source_layer(&self, name: &str, slot: usize) -> Option<cff2::SourceLayer> {
        let Some(k) = slot.checked_sub(1) else { return Some(cff2::SourceLayer { master: self.origin.to_string(), intermediate: None }) };
        let s = self.sources.get(k)?;
        if let Some(m) = &s.master {
            return Some(cff2::SourceLayer { master: m.clone(), intermediate: None });
        }
        let found = self.glyph_sources.get(name)?.main.iter().find(|x| !x.is_master && x.location == s.location)?;
        Some(cff2::SourceLayer { master: found.master.clone(), intermediate: Some(found.layer_id.clone()) })
    }

    /// The global metrics of master `id` (`origin`'s are given).
    fn master_metrics<'m>(&'m self, id: &str, origin: &'m crate::info::GlobalMetrics) -> Option<&'m crate::info::GlobalMetrics> {
        if id == self.origin {
            return Some(origin);
        }
        self.sources.iter().find(|s| s.master.as_deref() == Some(id)).map(|s| &s.metrics)
    }

    /// The vertical phantom points of glyph `r` at source `slot` (0 the origin, whose metrics are
    /// `origin`; k + 1 source k). A layer without a vertical width or origin takes its master's
    /// metrics; so does an intermediate layer, as both oracles read it (an interpolated value
    /// would not be the layer's).
    fn vertical(&self, r: &ResolvedGlyph, slot: usize, origin: Option<&crate::info::GlobalMetrics>) -> Vertical {
        let Some(origin) = origin else { return Vertical::default() };
        let Some(source) = slot.checked_sub(1).and_then(|k| self.sources.get(k)) else {
            return Vertical::new(r.vert_width, r.vert_origin, origin);
        };
        if source.master.is_none()
            && let Some(cff2::SourceLayer { master, intermediate: Some(id) }) = self.source_layer(&r.name, slot)
            && let Some(layer) = self.prepared.glyph(&r.name).and_then(|g| g.layers.iter().find(|l| l.id == id))
            && let Some(metrics) = self.master_metrics(&master, origin)
        {
            return Vertical::new(layer.info.vert_width, layer.info.vert_origin, metrics);
        }
        Vertical::new(r.vert_width, r.vert_origin, &source.metrics)
    }

    /// The sources besides the origin that `name` has a layer for (indices into `sources`).
    fn own_sources(&self, name: &str, warnings: &mut Vec<String>) -> Vec<usize> {
        let Some(gs) = self.glyph_sources.get(name) else {
            // The `.notdef` the compile draws follows each master's vertical metrics, as fontc
            // and fontmake vary it.
            if name != ".notdef" {
                return Vec::new();
            }
            // Masters that share a location (or the origin's) give one source.
            let mut kept: Vec<usize> = Vec::new();
            for (k, o) in self.sources.iter().enumerate() {
                let distinct = o.normalized.iter().any(|v| *v != 0.0)
                    && !kept.iter().any(|&j| self.sources.get(j).is_some_and(|p| p.normalized == o.normalized));
                if o.master.is_some() && o.index.contains_key(name) && distinct {
                    kept.push(k);
                }
            }
            return kept;
        };
        if gs.missing_masters.iter().any(|m| m == self.origin) {
            if gs.main.iter().any(|s| s.is_master) {
                warnings.push(format!("glyph `{name}` has no layer for the origin master, so it does not vary"));
            }
            return Vec::new();
        }
        gs.main
            .iter()
            .filter_map(|s| {
                if s.is_master && s.master == self.origin {
                    return None;
                }
                self.sources.iter().position(|o| {
                    if s.is_master { o.master.as_deref() == Some(s.master.as_str()) } else { o.master.is_none() && o.location == s.location }
                })
            })
            .filter(|&k| self.sources.get(k).is_some_and(|o| o.index.contains_key(name)))
            .collect()
    }

    /// The intermediate sources (indices into `sources`) at the locations of the intermediate
    /// layers of the glyphs `g` reaches through its components, nested ones included.
    fn component_sources<'a>(&self, g: &'a ResolvedGlyph, glyphs: &'a [ResolvedGlyph], index: &HashMap<&str, usize>) -> Vec<usize> {
        // Each name enters `todo` once, so the walk is linear in the components it reads.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut todo: Vec<&str> = Vec::new();
        let visit = |components: &'a [fontcraft_model::ResolvedComponent], seen: &mut HashSet<&'a str>, todo: &mut Vec<&'a str>| {
            for c in components {
                if seen.len() >= MAX_COMPONENT_CLOSURE {
                    return;
                }
                if seen.insert(c.base.as_str()) {
                    todo.push(c.base.as_str());
                }
            }
        };
        visit(&g.components, &mut seen, &mut todo);
        let mut out = Vec::new();
        while let Some(name) = todo.pop() {
            if let Some(gs) = self.glyph_sources.get(name) {
                for s in gs.main.iter().filter(|s| !s.is_master) {
                    if let Some(k) = self.sources.iter().position(|o| o.master.is_none() && o.location == s.location)
                        && !out.contains(&k)
                    {
                        out.push(k);
                    }
                }
            }
            if let Some(r) = index.get(name).and_then(|&j| glyphs.get(j)) {
                visit(&r.components, &mut seen, &mut todo);
            }
        }
        out.sort_unstable();
        out
    }

    /// The sources (indices into `sources`) that sample the interpolation curves `g` is drawn
    /// with: its own, and those of the glyphs its outline holds as contours. `decomposed`: its
    /// components are written as contours; a component that is not has its own sources.
    fn sample_sources(&self, g: &ResolvedGlyph, decomposed: bool) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        if self.samples.is_empty() {
            return out;
        }
        let referenced = |name: &str| !decomposed && g.components.iter().any(|c| c.base == name);
        // Each name enters `todo` once, so the walk is linear in the components it reads.
        let mut seen: HashSet<&str> = HashSet::from([g.name.as_str()]);
        let mut todo: Vec<&str> = vec![g.name.as_str()];
        while let Some(name) = todo.pop() {
            for location in self.samples.of(name) {
                if let Some(k) = self.sources.iter().position(|o| o.master.is_none() && o.location == *location)
                    && !out.contains(&k)
                {
                    out.push(k);
                }
            }
            let Some(layer) = self.prepared.glyph(name).and_then(|p| p.layer(self.origin)) else { continue };
            for shape in &layer.shapes {
                let fontcraft_model::Shape::Component(c) = shape else { continue };
                if !referenced(&c.glyph) && seen.len() < MAX_COMPONENT_CLOSURE && seen.insert(c.glyph.as_str()) {
                    todo.push(c.glyph.as_str());
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// The outline of each glyph of `glyphs` (the origin's final order) and its points at each of
    /// its sources; for CFF2 the points are the advances' phantoms only and the cubic outlines of
    /// each source come third. `programs`: glyph programs are compiled for the font. `vertical`:
    /// the origin's metrics, when the font has vertical metrics.
    fn build(
        &self,
        glyphs: &[ResolvedGlyph],
        settings: &crate::outline::Settings,
        programs: bool,
        vertical: Option<&crate::info::GlobalMetrics>,
        warnings: &mut Vec<String>,
    ) -> Result<BuiltOutlines, CompileError> {
        let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
        let own: Vec<Vec<usize>> = glyphs.iter().map(|g| self.own_sources(&g.name, warnings)).collect();
        let mut decompose = vec![false; glyphs.len()];
        for (i, g) in glyphs.iter().enumerate() {
            if g.components.is_empty() {
                continue;
            }
            let mut drawn: Vec<&ResolvedGlyph> = vec![g];
            drawn.extend(own.get(i).into_iter().flatten().filter_map(|&k| self.sources.get(k)?.glyph(&g.name).map(|(_, r)| r)));
            let model_reason = self.glyph_sources.get(&g.name).and_then(|gs| decompose::decompose_reason(self.prepared, self.designspace, gs));
            if let Some(slot) = decompose.get_mut(i) {
                *slot = drawn.iter().any(|r| crate::outline::must_decompose(r, self.flavour))
                    || decompose::inconsistent_transform(&drawn).is_some()
                    || model_reason.is_some();
            }
        }
        // A CFF2 composite is decomposed, so it varies with its components' intermediate layers
        // too, as the TrueType composite does through its components.
        let mut own = own;
        if self.flavour == Flavour::Cff {
            for (i, g) in glyphs.iter().enumerate() {
                if !decompose.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let extra = self.component_sources(g, glyphs, &index);
                if let Some(list) = own.get_mut(i) {
                    for k in extra {
                        if !list.contains(&k) && self.sources.get(k).is_some_and(|o| o.index.contains_key(&g.name)) {
                            list.push(k);
                        }
                    }
                }
            }
        }
        // Each source's flags, by name, for nested decomposition.
        let source_flags: Vec<Vec<bool>> = self
            .sources
            .iter()
            .map(|s| {
                s.glyphs
                    .iter()
                    .map(|g| match index.get(g.name.as_str()) {
                        Some(&i) => decompose.get(i).copied().unwrap_or(false),
                        None => crate::outline::must_decompose(g, self.flavour),
                    })
                    .collect()
            })
            .collect();
        let source_index: Vec<HashMap<&str, usize>> =
            self.sources.iter().map(|s| s.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect()).collect();
        let mut memo: Vec<HashMap<usize, Vec<BezPath>>> = vec![HashMap::new(); self.sources.len() + 1];
        let mut expansion = crate::Expansion::new(settings.max_expanded);
        let origin_location = vec![0.0; self.designspace.axes().len()];

        let mut outlines = Vec::with_capacity(glyphs.len());
        let mut per_glyph = Vec::with_capacity(glyphs.len());
        let mut cubic = Vec::new();
        for (i, g) in glyphs.iter().enumerate() {
            let decomposed = decompose.get(i).copied().unwrap_or(false);
            let mut ks: Vec<usize> = own.get(i).cloned().unwrap_or_default();
            ks.retain(|&k| self.sources.get(k).is_some_and(|s| s.index.contains_key(&g.name)));
            let own_count = ks.len();
            if own_count > 0 {
                for k in self.sample_sources(g, decomposed) {
                    if !ks.contains(&k) && self.sources.get(k).is_some_and(|s| s.index.contains_key(&g.name)) {
                        ks.push(k);
                    }
                }
            }
            let samples = ks.len().saturating_sub(own_count);
            let mut locations = vec![origin_location.clone()];
            let mut drawn: Vec<(usize, &ResolvedGlyph)> = vec![(i, g)];
            for &k in &ks {
                let Some(s) = self.sources.get(k) else { continue };
                if let Some(found) = s.glyph(&g.name) {
                    locations.push(s.normalized.clone());
                    drawn.push(found);
                }
            }
            // Per entry of `drawn`, which source it is: 0 the origin, k + 1 source k.
            let slots: Vec<usize> = std::iter::once(0).chain(ks.iter().map(|k| k + 1)).collect();
            let ends: Vec<[(f64, f64); gvar::PHANTOM_POINTS]> =
                slots.iter().zip(&drawn).map(|(&slot, (_, r))| phantoms(r.width, self.vertical(r, slot, vertical))).collect();
            if !decomposed && !g.components.is_empty() {
                outlines.push(crate::outline::composite(g)?);
                let points = drawn
                    .iter()
                    .zip(&ends)
                    .map(|((_, r), end)| {
                        let mut ps: Vec<(f64, f64)> = r
                            .components
                            .iter()
                            .map(|c| {
                                let [.., x, y] = c.transform.as_coeffs();
                                (ot_round(x), ot_round(y))
                            })
                            .collect();
                        ps.extend(end);
                        ps
                    })
                    .collect();
                per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends: Vec::new(), samples });
                continue;
            }
            let mut contours: Vec<Vec<BezPath>> = Vec::with_capacity(drawn.len());
            for (&slot, &(j, r)) in slots.iter().zip(&drawn) {
                let list = if !decomposed {
                    r.contours.clone()
                } else if slot == 0 {
                    let m = memo.get_mut(0).ok_or(CompileError::NotYetImplemented("a decomposition memo"))?;
                    crate::outline::decomposed(glyphs, &index, &decompose, j, 0, m, &mut expansion)?
                } else {
                    let k = slot - 1;
                    let (Some(s), Some(idx), Some(flags), Some(m)) =
                        (self.sources.get(k), source_index.get(k), source_flags.get(k), memo.get_mut(slot))
                    else {
                        continue;
                    };
                    crate::outline::decomposed(&s.glyphs, idx, flags, j, 0, m, &mut expansion)?
                };
                contours.push(list);
            }
            if contours.len() != drawn.len() {
                return Err(CompileError::Glyph { glyph: g.name.clone(), message: "a source of the glyph could not be drawn".into() });
            }
            if self.flavour == Flavour::Cff {
                let points = ends.iter().map(|end| end.to_vec()).collect();
                let origin_path = contours.first().map(|list| {
                    let mut path = BezPath::new();
                    list.iter().for_each(|c| path.extend(c.clone()));
                    path
                });
                outlines.push(match origin_path {
                    Some(p) if !p.elements().is_empty() => Prepared::Simple(p),
                    _ => Prepared::Empty,
                });
                per_glyph.push(gvar::SourceOutlines {
                    glyph: g.name.clone(),
                    locations: locations.clone(),
                    points,
                    contour_ends: Vec::new(),
                    samples,
                });
                let layers = slots.iter().map(|&slot| self.source_layer(&g.name, slot)).collect();
                cubic.push(cff2::CubicSources { glyph: g.name.clone(), locations, paths: contours, layers, samples });
                continue;
            }
            if contours.iter().all(|c| c.iter().all(|p| p.elements().is_empty())) {
                outlines.push(Prepared::Empty);
                let points = ends.iter().map(|end| end.to_vec()).collect();
                per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends: Vec::new(), samples });
                continue;
            }
            let far = |p: &fontcraft_geom::Point| !(p.x.abs() <= crate::MAX_TT_COORDINATE && p.y.abs() <= crate::MAX_TT_COORDINATE);
            if contours.iter().flatten().any(|c| crate::outline::points(c).any(|p| far(&p))) {
                return Err(CompileError::Glyph {
                    glyph: g.name.clone(),
                    message: format!(
                        "a point lies more than {} units from the origin, further than a TrueType outline can store",
                        crate::MAX_TT_COORDINATE
                    ),
                });
            }
            if settings.curves == crate::outline::CurveMode::KeepQuad {
                crate::outline::refuse_cubic(&g.name, contours.iter().flatten())?;
            }
            let quadratic = glyf::quadratic_sources(&g.name, &contours, settings.tolerance)?;
            let paths: Vec<BezPath> = quadratic
                .into_iter()
                .map(|list| {
                    let mut path = BezPath::new();
                    list.into_iter().for_each(|c| path.extend(c));
                    if settings.keep_direction { path } else { path.reverse_subpaths() }
                })
                .collect();
            // A glyph whose TrueType instructions are compiled keeps the on-curve points its
            // nodes give, so the instructions find them ([`crate::points`]).
            let keep = if self.keep_implied {
                contours.iter().take(1).flatten().flat_map(crate::outline::on_curve_nodes).collect()
            } else if programs {
                crate::points::kept_nodes(self.prepared, &g.name, self.origin)
            } else {
                Vec::new()
            };
            let mut simples = match Some(&keep).filter(|k| !k.is_empty()).and_then(|k| crate::points::interpolatable_glyphs(&paths, k)) {
                Some(s) => s,
                None => SimpleGlyph::interpolatable_glyphs_from_bezpaths(&paths).map_err(|e| CompileError::Glyph {
                    glyph: g.name.clone(),
                    message: format!("its sources cannot be written with the same points: {e:?}"),
                })?,
            };
            let count: usize = simples.first().map_or(0, |s| s.contours.iter().map(|c| c.len()).sum());
            if count > crate::MAX_TT_POINTS {
                return Err(CompileError::Glyph {
                    glyph: g.name.clone(),
                    message: format!("the outline has {count} points; a TrueType glyph holds at most {}", crate::MAX_TT_POINTS),
                });
            }
            let mut contour_ends = Vec::new();
            if let Some(first) = simples.first() {
                let mut end = 0usize;
                for c in &first.contours {
                    end += c.len();
                    contour_ends.push(end.saturating_sub(1));
                }
            }
            let points = simples
                .iter()
                .zip(&ends)
                .map(|(s, end)| {
                    let mut ps: Vec<(f64, f64)> = s.contours.iter().flat_map(|c| c.iter().map(|p| (f64::from(p.x), f64::from(p.y)))).collect();
                    ps.extend(end);
                    ps
                })
                .collect();
            let mut origin = if simples.is_empty() { SimpleGlyph::default() } else { simples.swap_remove(0) };
            origin.overlaps = true;
            origin.recompute_bounding_box();
            outlines.push(Prepared::Points(origin));
            per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends, samples });
        }
        Ok((outlines, per_glyph, cubic))
    }
}

/// Each glyph's value `get` reads from the phantom points of its sources, with its deltas. The
/// sources that only sample the outline take no part.
fn metric_values(glyphs: &[gvar::SourceOutlines], table: &'static str, get: fn(&[(f64, f64)]) -> f64) -> Result<Vec<VarValue>, CompileError> {
    let mut models = gvar::Models::default();
    glyphs
        .iter()
        .map(|g| {
            let own = g.locations.len().saturating_sub(g.samples);
            let locations = g.locations.get(..own).unwrap_or(&g.locations);
            let values: Vec<Vec<f64>> = g.points.iter().take(own).map(|ps| vec![get(ps)]).collect();
            let default = values.first().and_then(|v| v.first()).copied().unwrap_or(0.0);
            let default = i32::try_from(default as i64).unwrap_or(0);
            if locations.len() < 2 {
                return Ok(VarValue { default, deltas: Vec::new() });
            }
            let bad = |e: String| CompileError::Table { table, message: format!("glyph `{}`: {e}", g.glyph) };
            let model = models.get(locations).map_err(bad)?;
            let mut deltas = Vec::new();
            for (region, d) in gvar::region_deltas(model, &values).map_err(bad)? {
                let d = d.first().copied().unwrap_or(0.0);
                if !gvar::fits_i16(d) {
                    return Err(bad(format!("a delta of {d} units does not fit the table")));
                }
                if d != 0.0 {
                    deltas.push((region, d as i32));
                }
            }
            Ok(VarValue { default, deltas })
        })
        .collect()
}

#[cfg(test)]
mod tests;
