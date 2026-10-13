//! The variable font compile (M9.5, with M9.6 to M9.8 and M9.13).
//!
//! [`compile_variable`] builds one variable font for a variable font setting, or for the whole
//! designspace when the font has none (`export.variableUsesSettingsOrDefault`):
//!
//! | Stage | Where | Package |
//! |---|---|---|
//! | Designspace and preflight | `fontcraft_interp::Designspace::for_setting`, the compatibility check (`compatibility.exportBlocked`) | M9.5 |
//! | Glyph order | the static order plus the alternate-layer glyphs ([`crate::order::MadeGlyph`]) | M9.5, M9.8 |
//! | Decomposition | [`decompose`]: inconsistent component transforms, the decomposition parameters | M9.5 |
//! | Outlines | [`glyf`]: cubic to quadratic across sources, `glyf` from the origin; [`gvar`] with phantom points, IUP and shared tuples | M9.5 |
//! | Axes and names | [`fvar`], [`avar`], [`names`] | M9.6 |
//! | Metrics variations | [`hvar`] (`HVAR`, `VVAR`), [`mvar`] | M9.6 |
//! | Style attributes | [`stat`] | M9.7 |
//! | Feature variations | [`featvar`]: alternate layers as `GSUB` feature variations | M9.8 |
//! | Variable layout | `fontcraft_features::var_layout`: kerning and marks with deltas | M9.9 |
//! | CFF2 | [`cff2`] | M9.13 |
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
pub mod decompose;
pub mod featvar;
pub mod fvar;
pub mod glyf;
pub mod gvar;
pub mod hvar;
pub mod mvar;
pub mod names;
pub mod stat;

use std::collections::HashMap;

use fontcraft_geom::BezPath;
use fontcraft_model::{Font, ResolvedGlyph, Target};
use write_fonts::tables::glyf::SimpleGlyph;
use write_fonts::types::{NameId, Tag};

pub use fontcraft_features::var_layout::VarValue;
pub use fontcraft_geom::variation::{Region, RegionAxis};

use crate::outline::Prepared;
use crate::{CompileError, Compiled, Flavour, Options, ot_round};

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
fn setting_font(font: &Font, setting: Option<usize>) -> std::borrow::Cow<'_, Font> {
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
}

impl Source {
    fn glyph(&self, name: &str) -> Option<(usize, &ResolvedGlyph)> {
        let &i = self.index.get(name)?;
        Some((i, self.glyphs.get(i)?))
    }
}

/// The glyphs of `prepared` at `target` after every stage before the outlines.
fn source_glyphs(font: &Font, prepared: &Font, target: &Target, options: &Options) -> Result<Vec<ResolvedGlyph>, CompileError> {
    let mut resolved = crate::drawing::resolve(prepared, target)?;
    crate::glyph_stages(Some(font), &mut resolved, Flavour::TrueType, options, &mut Vec::new())
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

fn phantoms(advance: f64) -> [(f64, f64); gvar::PHANTOM_POINTS] {
    [(0.0, 0.0), (ot_round(advance), 0.0), (0.0, 0.0), (0.0, 0.0)]
}

/// Compiles the variable font of `setting` (an index into `Font::instances`; `None` for the
/// whole designspace). Refuses incompatible glyphs, a designspace without a usable origin, and
/// everything the static compile refuses.
pub fn compile_variable(font: &Font, setting: Option<usize>, options: &Options, var: &VariableOptions) -> Result<VariableFont, CompileError> {
    if var.flavour == VarFlavour::Cff2 {
        return Err(CompileError::NotYetImplemented("CFF2 variable fonts"));
    }
    font.check()?;
    let mut notes: Vec<String> = Vec::new();
    let font = setting_font(font, setting);
    let font: &Font = &font;
    let designspace = match setting {
        Some(s) => fontcraft_interp::Designspace::for_setting(font, s)?,
        None => fontcraft_interp::Designspace::new(font)?,
    };
    notes.extend(designspace.warnings().iter().cloned());
    if !designspace.axes().iter().any(|a| a.external.minimum < a.external.maximum) || designspace.masters().len() < 2 {
        return Err(CompileError::Table { table: "fvar", message: "a variable font needs two masters and an axis with a range".into() });
    }
    designspace.check_variable_origin()?;
    let origin = designspace.origin().cloned().ok_or(CompileError::NotYetImplemented("a designspace without an origin"))?;
    let axis_count = u16::try_from(designspace.axes().len()).map_err(|_| CompileError::Table { table: "fvar", message: "too many axes".into() })?;

    // Preflight (`compatibility.exportBlocked`): every exported glyph compatible in every master.
    let scope = fontcraft_interp::MasterPairs { masters: designspace.masters().iter().map(|m| m.id.clone()).collect(), indicators: true };
    let mut incompatible: Vec<String> = Vec::new();
    for g in font.glyphs.iter().filter(|g| g.export) {
        let sources = fontcraft_interp::GlyphSources::of(font, &designspace, &g.name)?;
        if !fontcraft_interp::compat::check_glyph(font, &sources, &scope).ok {
            incompatible.push(g.name.clone());
        }
    }
    if !incompatible.is_empty() {
        let count = incompatible.len();
        incompatible.truncate(MAX_NAMED_GLYPHS);
        let more = if count > MAX_NAMED_GLYPHS { format!(" and {} more", count - MAX_NAMED_GLYPHS) } else { String::new() };
        return Err(CompileError::Incompatible(format!("{}{more}", incompatible.join(", "))));
    }

    let mut options = options.clone();
    options.remove_overlap = false;
    if options.glyph_data.0.is_none()
        && let Ok(data) = fontcraft_model::glyphdata::GlyphData::new()
    {
        options.glyph_data = crate::DataRef(Some(std::sync::Arc::new(data)));
    }
    let vnames = names::variable_names(font, setting)?;
    options.style_name.clone_from(&vnames.style);

    let prepared = crate::prepare_source(font, &mut notes);
    let prepared: &Font = &prepared;
    let mut glyph_sources: HashMap<String, fontcraft_interp::GlyphSources> = HashMap::new();
    for g in prepared.glyphs.iter() {
        glyph_sources.insert(g.name.clone(), fontcraft_interp::GlyphSources::of(prepared, &designspace, &g.name)?);
    }
    if glyph_sources.values().any(|s| !s.alternates.is_empty()) {
        notes.push("alternate layers are not in variable fonts yet; their glyphs keep the master layers".into());
    }

    // The sources besides the origin: the other masters, then each intermediate location.
    let mut sources: Vec<Source> = Vec::new();
    let mut add = |master: Option<String>, location: fontcraft_interp::Location, glyphs: Vec<ResolvedGlyph>| -> Result<(), CompileError> {
        let normalized = designspace.to_normalized(&location)?.0;
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        sources.push(Source { master, location, normalized, glyphs, index });
        Ok(())
    };
    for m in designspace.masters().iter().filter(|m| m.id != origin.id) {
        add(Some(m.id.clone()), m.location.clone(), source_glyphs(font, prepared, &Target::Master(m.id.clone()), &options)?)?;
    }
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
    for location in intermediate {
        // An intermediate layer beyond the axes of this font (a setting that disables masters
        // can narrow them) is no source of it.
        if designspace.to_normalized(&location)?.0.iter().any(|v| !(-1.0..=1.0).contains(v)) {
            notes.push(format!("intermediate layers at {:?} lie outside the axes of this variable font and were left out", location.0));
            continue;
        }
        let at = font_at(prepared, &designspace, &location)?;
        let target = at.default_target()?;
        add(None, location, source_glyphs(&at, &at, &target, &options)?)?;
    }

    let mut resolved = crate::drawing::resolve(prepared, &Target::Master(origin.id.clone()))?;
    resolved.family_name.clone_from(&vnames.family);
    let gvar_options = gvar::GvarOptions { optimize: var.optimize_deltas.unwrap_or_else(|| optimize_deltas(font)) };
    let static_names = {
        let mut probe = resolved.clone();
        crate::fill_glyph_data(&mut probe, &options, &mut Vec::new());
        crate::info::derive(&probe, &options.style_name, &mut Vec::new())
            .names
            .into_iter()
            .filter(|r| r.platform == 3 && r.language == 0x409)
            .map(|r| (NameId::new(r.id), r.text))
            .collect::<Vec<_>>()
    };
    let ctx = Outlines { designspace: &designspace, prepared, origin: &origin.id, sources: &sources, glyph_sources: &glyph_sources };
    let mut stage = |_: &fontcraft_model::Resolved,
                     glyphs: &[ResolvedGlyph],
                     settings: &crate::outline::Settings,
                     warnings: &mut Vec<String>|
     -> Result<(Vec<Prepared>, crate::Extra), CompileError> {
        let (outlines, per_glyph) = ctx.build(glyphs, settings, warnings)?;
        let mut extra = crate::Extra::default();
        let gvar = gvar::build_gvar(axis_count, &per_glyph, &gvar_options)?;
        let advances = ctx.advances(&per_glyph)?;
        let axes = VarAxes::from_designspace(&designspace)?;
        let instances = fvar::named_instances(font, &designspace)?;
        let mut allocator = names::NameAllocator::with_static(static_names.clone());
        let fvar = fvar::build_fvar(&axes, &instances, &mut allocator)?;
        let stat = stat::stat_input(font, &designspace)?.map(|input| stat::build_stat(&input, &mut allocator)).transpose()?;
        let dump = |tag: &'static str, r: Result<Vec<u8>, write_fonts::error::Error>| {
            r.map_err(|e| CompileError::Table { table: tag, message: e.to_string() })
        };
        extra.tables.push((Tag::new(b"fvar"), dump("fvar", write_fonts::dump_table(&fvar))?));
        extra.tables.push((Tag::new(b"gvar"), dump("gvar", write_fonts::dump_table(&gvar))?));
        if let Some(avar) = avar::build_avar(&axes)? {
            extra.tables.push((Tag::new(b"avar"), dump("avar", write_fonts::dump_table(&avar))?));
        }
        if advances.iter().any(|a| !a.deltas.is_empty()) {
            extra.tables.push((Tag::new(b"HVAR"), dump("HVAR", write_fonts::dump_table(&hvar::build_hvar(axis_count, &advances)?))?));
        }
        if let Some(stat) = stat {
            extra.tables.push((Tag::new(b"STAT"), dump("STAT", write_fonts::dump_table(&stat))?));
        }
        extra.names = allocator.records().iter().map(|(id, text)| (id.to_u16(), text.clone())).collect();
        if let Some(ps) = &vnames.postscript_name {
            extra.names.push((6, ps.clone()));
        }
        if let Some(prefix) = &vnames.postscript_prefix {
            extra.names.push((25, prefix.clone()));
        }
        Ok((outlines, extra))
    };
    let mut compiled = crate::compile_stages_with(Some(font), resolved, Flavour::TrueType, &options, Some(&mut stage))?;
    notes.append(&mut compiled.warnings);
    compiled.warnings = notes;
    Ok(VariableFont { compiled, setting, file_stem: vnames.file_stem })
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

/// What the outline stage of a variable compile reads.
struct Outlines<'a> {
    designspace: &'a fontcraft_interp::Designspace,
    prepared: &'a Font,
    origin: &'a str,
    sources: &'a [Source],
    glyph_sources: &'a HashMap<String, fontcraft_interp::GlyphSources>,
}

impl Outlines<'_> {
    /// The sources besides the origin that `name` has a layer for (indices into `sources`).
    fn own_sources(&self, name: &str, warnings: &mut Vec<String>) -> Vec<usize> {
        let Some(gs) = self.glyph_sources.get(name) else { return Vec::new() };
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

    /// The outline of each glyph of `glyphs` (the origin's final order) and its points at each of
    /// its sources.
    fn build(
        &self,
        glyphs: &[ResolvedGlyph],
        settings: &crate::outline::Settings,
        warnings: &mut Vec<String>,
    ) -> Result<(Vec<Prepared>, Vec<gvar::SourceOutlines>), CompileError> {
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
                *slot = drawn.iter().any(|r| crate::outline::must_decompose(r, Flavour::TrueType))
                    || decompose::inconsistent_transform(&drawn).is_some()
                    || model_reason.is_some();
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
                        None => crate::outline::must_decompose(g, Flavour::TrueType),
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
        for (i, g) in glyphs.iter().enumerate() {
            let ks: &[usize] = own.get(i).map_or(&[], Vec::as_slice);
            let mut locations = vec![origin_location.clone()];
            let mut drawn: Vec<(usize, &ResolvedGlyph)> = vec![(i, g)];
            for &k in ks {
                let Some(s) = self.sources.get(k) else { continue };
                if let Some(found) = s.glyph(&g.name) {
                    locations.push(s.normalized.clone());
                    drawn.push(found);
                }
            }
            // Per entry of `drawn`, which source it is: 0 the origin, k + 1 source k.
            let slots: Vec<usize> = std::iter::once(0)
                .chain(ks.iter().filter(|&&k| self.sources.get(k).is_some_and(|s| s.index.contains_key(&g.name))).map(|k| k + 1))
                .collect();
            let decomposed = decompose.get(i).copied().unwrap_or(false);
            if !decomposed && !g.components.is_empty() {
                outlines.push(crate::outline::composite(g)?);
                let points = drawn
                    .iter()
                    .map(|(_, r)| {
                        let mut ps: Vec<(f64, f64)> = r
                            .components
                            .iter()
                            .map(|c| {
                                let [.., x, y] = c.transform.as_coeffs();
                                (ot_round(x), ot_round(y))
                            })
                            .collect();
                        ps.extend(phantoms(r.width));
                        ps
                    })
                    .collect();
                per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends: Vec::new() });
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
            let advances: Vec<f64> = drawn.iter().map(|(_, r)| r.width).collect();
            if contours.iter().all(|c| c.iter().all(|p| p.elements().is_empty())) {
                outlines.push(Prepared::Empty);
                let points = advances.iter().map(|&a| phantoms(a).to_vec()).collect();
                per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends: Vec::new() });
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
            let quadratic = glyf::quadratic_sources(&g.name, &contours, settings.tolerance)?;
            let paths: Vec<BezPath> = quadratic
                .into_iter()
                .map(|list| {
                    let mut path = BezPath::new();
                    list.into_iter().for_each(|c| path.extend(c));
                    if settings.keep_direction { path } else { path.reverse_subpaths() }
                })
                .collect();
            let mut simples = SimpleGlyph::interpolatable_glyphs_from_bezpaths(&paths).map_err(|e| CompileError::Glyph {
                glyph: g.name.clone(),
                message: format!("its sources cannot be written with the same points: {e:?}"),
            })?;
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
                .zip(&advances)
                .map(|(s, &a)| {
                    let mut ps: Vec<(f64, f64)> = s.contours.iter().flat_map(|c| c.iter().map(|p| (f64::from(p.x), f64::from(p.y)))).collect();
                    ps.extend(phantoms(a));
                    ps
                })
                .collect();
            let mut origin = if simples.is_empty() { SimpleGlyph::default() } else { simples.swap_remove(0) };
            origin.overlaps = true;
            origin.recompute_bounding_box();
            outlines.push(Prepared::Points(origin));
            per_glyph.push(gvar::SourceOutlines { glyph: g.name.clone(), locations, points, contour_ends });
        }
        Ok((outlines, per_glyph))
    }

    /// Each glyph's advance with its deltas, from the second phantom point of its sources.
    fn advances(&self, glyphs: &[gvar::SourceOutlines]) -> Result<Vec<VarValue>, CompileError> {
        let mut models = gvar::Models::default();
        glyphs
            .iter()
            .map(|g| {
                let advance = |ps: &Vec<(f64, f64)>| ps.len().checked_sub(3).and_then(|i| ps.get(i)).map_or(0.0, |p| p.0);
                let values: Vec<Vec<f64>> = g.points.iter().map(|ps| vec![advance(ps)]).collect();
                let default = values.first().and_then(|v| v.first()).copied().unwrap_or(0.0);
                let default = i32::try_from(default as i64).unwrap_or(0);
                if g.locations.len() < 2 {
                    return Ok(VarValue { default, deltas: Vec::new() });
                }
                let bad = |e: String| CompileError::Table { table: "HVAR", message: format!("glyph `{}`: {e}", g.glyph) };
                let model = models.get(&g.locations).map_err(bad)?;
                let deltas = gvar::region_deltas(model, &values)
                    .map_err(bad)?
                    .into_iter()
                    .filter_map(|(region, d)| {
                        let d = d.first().copied().unwrap_or(0.0);
                        (d != 0.0).then_some((region, d as i32))
                    })
                    .collect();
                Ok(VarValue { default, deltas })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
