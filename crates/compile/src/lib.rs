//! FontCraft compiler (L3): source model → OpenType binary.
//!
//! Compiling runs in stages, each a function of its own that reads only what the stages before it
//! produced (`docs/spec/40-compile.md` § 1, `compile.pipelineStages`):
//!
//! | Stage | Where | What it does |
//! |---|---|---|
//! | Source preparation | [`prepare_source`] | corner components fitted into their paths, then smart components drawn as outlines, as both reference compilers do when they load a source |
//! | Resolve | [`drawing::resolve`] | one master, or an instance at a master, as the compiler reads it: glyphs, contours, components, metrics, names; strokes expanded and masks applied |
//! | Preflight | [`preflight`] | component cycles refused, missing components dropped, non-exported glyphs inlined and removed, code point conflicts refused |
//! | Glyph data | [`fill_glyph_data`] | production names and categories a glyph does not store, from the glyph database; non-spacing marks get no advance |
//! | Export parameters | [`export_params`] | Export, Keep, Remove, Rename and Reencode Glyphs and the Arabic presentation forms change the resolved glyphs; the renames and removals go to the kerning writers |
//! | Glyph order | [`order`] | the export order, `.notdef` first and drawn when the font has none, at most 65,535 glyphs |
//! | Component options | [`components_opt`] | decomposition and flattening the options, parameters and filter list ask for |
//! | Layout | [`layout`] (with `kern` and `marks`) | the feature code for the target (automatic entries regenerated, instance parameters, tokens) compiled with the generated kerning and mark attachment: `GSUB`, `GPOS`, `GDEF`; a compile error stops the export |
//! | Outline preparation | [`outline`] | decomposition (mixed glyphs, transforms beyond ±2, everything for CFF), overlap removal, cubic to quadratic, contour direction |
//! | Hinting (M10) | [`cff_hints`], [`tt_hints`], [`points`], [`gasp`] | CFF stem hints, masks and flex; `cvt `, `fpgm`, `prep` and glyph programs against the point map; `gasp` |
//! | Table builders and assembly | `build`, [`cff`], [`names`], [`kern_table`] | `glyf`/`loca` with composites, or `CFF ` (cubic outlines in their drawn direction, decomposed); `hmtx`, `hhea`, `maxp`, `head`, `cmap`, `OS/2`, `name`, `post` with production names, the layout tables, then the font file |
//!
//! The output is a **static font** of one master, with TrueType outlines ([`Flavour::TrueType`],
//! as fontc builds it) or CFF outlines ([`Flavour::Cff`], as fontmake builds it: `maxp` 0.5, `post`
//! 3.0, glyph names in the CFF charset, metrics from the charstrings' bounds). It is
//! deterministic: the same font and options give the same bytes. Rounding is half up everywhere
//! (OpenType rounding, `compile.rounding`): point coordinates when the glyph is written,
//! component offsets, advances.
//!
//! Variable fonts are built by [`var::compile_variable`], which runs the same stages for every
//! source and adds the variation tables.
//!
//! Everything handed to the table writer is validated first. `write-fonts` panics or overflows on
//! points more than 32,767 units apart, on more than 65,277 glyph names outside the standard set and
//! on oversized name strings; each has a limit and a test here. Work that grows with what
//! components multiply (decomposition, composite bounds) runs under a budget of
//! [`fontcraft_model::MAX_OUTLINE_ELEMENTS`] per glyph.
//!
//! What each table must hold at parity, and which reference compilers define it, is specified in
//! `docs/spec/40-compile.md` and `spec/inventory/40-tables.toml`. The plan for the rest of the
//! pipeline (layout, variations, CFF, hinting) is in `docs/decisions/0002-compile-pipeline.md`.
#![forbid(unsafe_code)]

pub mod arabic_forms;
mod build;
pub mod cff;
pub mod cff_hints;
pub mod color;
pub mod components_opt;
pub mod drawing;
pub mod export_params;
pub mod gasp;
pub mod info;
mod kern;
pub mod kern_table;
pub mod layout;
mod marks;
pub mod names;
pub mod order;
pub mod outline;
pub mod plan;
pub mod points;
pub mod preflight;
pub mod preview;
mod ranges;
pub mod thickness;
pub mod tt_hints;
pub mod var;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, ModelError, Resolved, Target};

pub use outline::Flavour;

/// Largest coordinate, in font units, an outline point may have in a TrueType outline. `glyf`
/// stores each point as a 16-bit difference from the one before, so two points may be at most
/// 32,767 units apart. (`write-fonts` 0.54 overflows past that; found by this crate's tests.)
/// Component offsets are held to the same limit.
pub const MAX_TT_COORDINATE: f64 = 16_383.0;
/// Most points one TrueType glyph may have (`maxp.maxPoints` and contour end indices are 16-bit).
pub const MAX_TT_POINTS: usize = 65_535;
/// Most glyphs a font may have, `.notdef` included (glyph ids are 16-bit).
pub const MAX_FONT_GLYPHS: usize = 65_535;
/// Most components one composite glyph may have (`maxp.maxComponentElements` is 16-bit).
pub const MAX_COMPONENTS: usize = 65_535;
/// Above this many glyphs the `post` table is written without names (format 3): format 2 can
/// index at most 65,277 names outside the standard set.
pub const MAX_POST_NAMES: usize = 65_000;

/// Most path elements and components that decomposition and inlining may create in one compile,
/// over all glyphs together. Components multiply what they reference, so without this a small
/// hostile source could ask for more memory than the machine has (an abort the panic guard cannot
/// catch). Ten million segments is several times what a fully decomposed CJK font needs.
pub const MAX_EXPANDED_ELEMENTS: usize = 10_000_000;

/// What decomposition and inlining may still create ([`MAX_EXPANDED_ELEMENTS`]).
#[derive(Debug)]
pub(crate) struct Expansion {
    left: usize,
}

impl Expansion {
    pub(crate) fn new(limit: usize) -> Self {
        Self { left: limit }
    }

    /// Takes `n` from what is left; an error naming `glyph` when that is not enough.
    pub(crate) fn take(&mut self, n: usize, glyph: &str) -> Result<(), CompileError> {
        self.left = self.left.checked_sub(n).ok_or_else(|| CompileError::Glyph {
            glyph: glyph.to_string(),
            message: format!("its components would take the font past {MAX_EXPANDED_ELEMENTS} decomposed outline segments"),
        })?;
        Ok(())
    }
}

/// The maximum error of the cubic to quadratic conversion, in em (`compile.cu2quTolerance`): 0.001,
/// as fontTools' cu2qu, fontmake and fontc use it.
pub const DEFAULT_CONVERSION_ERROR: f64 = 0.001;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("glyph `{glyph}`: {message}")]
    Glyph { glyph: String, message: String },
    #[error("U+{codepoint:04X} is mapped to both `{first}` and `{second}`")]
    DuplicateCodepoint { codepoint: u32, first: String, second: String },
    #[error("the font would have {0} glyphs, `.notdef` included; a font holds at most 65,535")]
    TooManyGlyphs(usize),
    #[error("cannot build the `{table}` table: {message}")]
    Table { table: &'static str, message: String },
    #[error("{0} outlines are not written yet")]
    Unsupported(&'static str),
    /// The feature code has errors, mapped to the entries that hold them; never empty.
    #[error("{}", fontcraft_features::source_map::summary(.0))]
    Features(Vec<fontcraft_features::EntryDiagnostic>),
    /// The designspace, the glyph sources or the compatibility check refused the font.
    #[error(transparent)]
    Interp(#[from] fontcraft_interp::InterpError),
    #[error(transparent)]
    Color(#[from] fontcraft_color::ColorError),
    /// The colour export of this target is refused (an instance between masters with colour
    /// layers, until variable colour lands); the message names the instance.
    #[error("{0}")]
    ColorRefused(String),
    /// A stage or table builder whose package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}

/// Options of [`compile`].
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Creation and modification time, in seconds since the Unix epoch. The compiler never reads
    /// the clock; callers that want "now" pass it (the engine does, and honours `SOURCE_DATE_EPOCH`).
    pub timestamp: i64,
    /// The most a quadratic approximation may be from the cubic curve it replaces, in em (the
    /// distance is this times the units per em). Not finite or not positive: the default.
    pub conversion_error: f64,
    /// Keep contours in the source's direction instead of reversing them for TrueType
    /// (`compile.reverseContourDirection`).
    pub keep_direction: bool,
    /// Style name written to the `name` table. Empty: the resolved master's or instance's name.
    pub style_name: String,
    /// Merge each glyph's overlapping contours into one outline (`compile.removeOverlapsStatic`).
    pub remove_overlap: bool,
    /// How long overlap removal may take for one glyph before the glyph keeps its overlaps, with
    /// a warning. Not used in the browser, where there are no worker threads.
    pub overlap_time_limit: Duration,
    /// The glyph database that gives production names and categories. `None`: the built-in one.
    pub glyph_data: DataRef,
    /// Decomposition and flattening beyond what the source asks ([`components_opt`]).
    pub components: components_opt::ComponentOptions,
    /// Hinting at export ([`HintingOptions`]).
    pub hinting: HintingOptions,
    /// Where image and SVG files of colour layers are read from ([`color`]). `None`: no files,
    /// so a glyph whose image is needed is left out of its table, with a warning.
    pub assets: AssetsRef,
    /// Also write `CBDT` and `CBLC` from the iColor layers (`color.export.cbdtNotInReference`).
    pub cbdt: bool,
    /// Leave the layout tables out: no feature code, kerning or mark attachment, for fast
    /// outline-only builds (`compile.skipFeatures`).
    pub skip_features: bool,
    /// The folder the feature code's `include` statements read from: the source's folder.
    /// `None`: an include of a file on disk is an error.
    pub feature_dir: Option<std::path::PathBuf>,
    /// Which included files on disk may be read; `None` reads any (see
    /// [`fontcraft_fea::VirtualFiles`]).
    pub include_check: Option<fontcraft_fea::IncludeCheck>,
    /// Append `;FontCraft <version>` to a generated version string (name ID 5), as the reference
    /// app appends its own name; a versionString parameter is written as it is
    /// (`fontInfo.version.headAndNameId5`). Exports turn it on; the default leaves it off.
    pub version_stamp: bool,
}

/// The asset store a compile reads ([`Options::assets`]). Two are equal when they are the same
/// store.
#[derive(Clone, Default)]
pub struct AssetsRef(pub Option<Arc<dyn fontcraft_color::AssetStore>>);

impl PartialEq for AssetsRef {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for AssetsRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "AssetsRef(store)" } else { "AssetsRef(none)" })
    }
}

/// What the compiler does about hints (`docs/plans/M10.md` § 3). The source's own settings
/// (Autohint, Export PostScript Hints, Export TrueType Hints…) are read from the font by
/// `fontcraft_hinting::HintSettings`; these options are the export dialog's and the command's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintingOptions {
    /// Autohint: PostScript glyphs without manual hints, every TrueType glyph (the export
    /// option; an instance's Autohint parameter overrides it).
    pub autohint: bool,
    /// Write no hints at all, whatever the source holds.
    pub strip: bool,
}

/// The glyph database a compile reads ([`Options::glyph_data`]). Two are equal when they are the
/// same database.
#[derive(Clone, Default)]
pub struct DataRef(pub Option<Arc<GlyphData>>);

impl PartialEq for DataRef {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for DataRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "DataRef(session)" } else { "DataRef(built-in)" })
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timestamp: 0,
            conversion_error: DEFAULT_CONVERSION_ERROR,
            keep_direction: false,
            style_name: String::new(),
            remove_overlap: true,
            overlap_time_limit: fontcraft_pathops::TIME_LIMIT,
            glyph_data: DataRef::default(),
            components: components_opt::ComponentOptions::default(),
            hinting: HintingOptions::default(),
            assets: AssetsRef::default(),
            cbdt: false,
            skip_features: false,
            feature_dir: None,
            include_check: None,
            version_stamp: false,
        }
    }
}

/// What a compile produced, besides the bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Compiled {
    pub bytes: Vec<u8>,
    /// Final glyph order (`.notdef` first), by source glyph name.
    pub glyph_order: Vec<String>,
    /// Things the user should know that did not stop the compile.
    pub warnings: Vec<String>,
}

/// Compiles `font` at `target` (a master, or an instance at a master) to a static font.
pub fn compile(font: &Font, target: &Target, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    font.check()?;
    let mut notes = Vec::new();
    let prepared = prepare_source(font, &mut notes);
    let resolved = drawing::resolve(&prepared, target)?;
    let mut compiled = compile_resolved_from(font, resolved, flavour, options)?;
    notes.append(&mut compiled.warnings);
    compiled.warnings = notes;
    Ok(compiled)
}

/// `font` as the compile stages read it: the corner components of every layer fitted into its
/// paths ([`fontcraft_model::corner::insert_all_corner_components`]), then every smart component
/// drawn as outlines ([`fontcraft_model::smart::draw_smart_components`]), the order in which
/// both reference compilers apply them when they load a source. Corners go first because a
/// smart glyph's pole layers can hold corners. A layer whose corners cannot be fitted, and a
/// smart component that cannot be drawn, stay as they are, with a warning. A font with neither
/// is borrowed, not copied. Callers of [`compile_resolved_from`] resolve the prepared font and
/// pass the source as it is stored.
pub fn prepare_source<'a>(font: &'a Font, warnings: &mut Vec<String>) -> Cow<'a, Font> {
    let mut out = Cow::Borrowed(font);
    let has_corners = font.glyphs.iter().any(|g| g.layers.iter().any(|l| l.info.hints.iter().any(fontcraft_model::corner::is_corner)));
    if has_corners {
        let (fitted, report) = fontcraft_model::corner::insert_all_corner_components(&out);
        warnings.extend(report.failed.into_iter().map(|e| format!("corner components were not fitted, so the paths were written without them: {e}")));
        out = Cow::Owned(fitted);
    }
    if out.glyphs.iter().any(|g| !g.info.parts.is_empty()) {
        let (drawn, report) = fontcraft_model::smart::draw_smart_components(&out);
        warnings.extend(report.kept.into_iter().map(|e| format!("a smart component was written as a plain component: {e}")));
        out = Cow::Owned(drawn);
    }
    out
}

/// Compiles the first master of `font` to a static TrueType font.
pub fn compile_ttf(font: &Font, options: &Options) -> Result<Compiled, CompileError> {
    compile(font, &font.default_target()?, Flavour::TrueType, options)
}

/// The stages after resolving, on a resolved font, without kerning: the resolved font holds none.
pub fn compile_resolved(resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    compile_stages(None, resolved, flavour, options)
}

/// The stages after resolving, on `resolved`, which was resolved from `font` after
/// [`prepare_source`]. `font` is the source as stored: it gives the kerning, the feature settings
/// and the anchors that propagation passes on, which a smart component passes on only while it is
/// still a component.
pub fn compile_resolved_from(font: &Font, resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    compile_stages(Some(font), resolved, flavour, options)
}

fn compile_stages(font: Option<&Font>, mut resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    fill_glyph_data(&mut resolved, options, &mut warnings);
    info::expand_tokens(font, &mut resolved, &mut warnings);
    let edits = export_params::apply(font, &mut resolved, &mut warnings)?;
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let synthesised_notdef = !glyphs.iter().any(|g| g.name == ".notdef");
    let glyphs = order::final_order(glyphs, &resolved, &mut warnings)?;
    preflight::check_cmap(&glyphs)?;
    let glyph_data = font.and_then(|_| compile_glyph_data(options));
    let color_plan = color::plan(font, &resolved, options);
    let (glyphs, made) = color::append_layer_glyphs(font, &color_plan, &resolved, glyphs, &mut warnings)?;
    let layout = layout_tables(font, glyph_data.as_deref(), &resolved, &glyphs, &edits, options, &mut warnings)?;
    let upm = f64::from(resolved.units_per_em);
    let error =
        if options.conversion_error.is_finite() && options.conversion_error > 0.0 { options.conversion_error } else { DEFAULT_CONVERSION_ERROR };
    let mut expansion = Expansion::new(MAX_EXPANDED_ELEMENTS);
    let glyphs = components_opt::apply(font, &resolved, glyphs, flavour, &options.components, &mut expansion, &mut warnings)?;
    let settings = outline::Settings {
        flavour,
        tolerance: upm * error,
        keep_direction: options.keep_direction,
        max_expanded: expansion.left,
        remove_overlap: options.remove_overlap,
        overlap_time_limit: options.overlap_time_limit,
    };
    let outlines = outline::prepare(&glyphs, &settings, &mut warnings)?;
    let kern_table = font.and_then(|font| kern_table::build(font, &resolved, &glyphs, flavour, &edits, &mut warnings));
    let color_tables = color::build(font, &color_plan, &color::GlyphOrder::new(&glyphs, &made)?, options, &mut warnings)?;
    let bytes = build::assemble(
        &resolved,
        &glyphs,
        &outlines,
        flavour,
        synthesised_notdef,
        &layout.tables(),
        kern_table.as_deref(),
        &color_tables.tables,
        options,
        &mut warnings,
    )?;
    Ok(Compiled { bytes, glyph_order: glyphs.into_iter().map(|g| g.name).collect(), warnings })
}

/// The layout stage of a static build ([`layout::build`]); nothing when the options skip features.
pub(crate) fn layout_tables(
    font: Option<&Font>,
    data: Option<&GlyphData>,
    resolved: &Resolved,
    order: &[fontcraft_model::ResolvedGlyph],
    edits: &export_params::ExportEdits,
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<layout::LayoutTables, CompileError> {
    if options.skip_features {
        return Ok(layout::LayoutTables::default());
    }
    let input = layout::LayoutInput {
        font,
        data,
        resolved,
        order,
        edits,
        include_dir: options.feature_dir.as_deref(),
        include_check: options.include_check.as_ref(),
    };
    layout::build(&input, &layout::LayoutOptions::default(), warnings)
}

/// Fills what the glyphs do not store from the glyph database ([`Resolved::apply_glyph_data`])
/// and gives non-spacing marks no advance (`compile.nonSpacingMarkWidth`), as fontc and glyphsLib
/// write them for the reference's format; a UFO states its advances as they are. A built-in
/// database that cannot be read leaves the stored values alone, with a warning.
pub fn fill_glyph_data(resolved: &mut Resolved, options: &Options, warnings: &mut Vec<String>) {
    if fontcraft_model::is_ufo_family(resolved.format) {
        return;
    }
    match &options.glyph_data.0 {
        Some(data) => resolved.apply_glyph_data(data),
        None => match GlyphData::new() {
            Ok(data) => resolved.apply_glyph_data(&data),
            Err(e) => warnings.push(format!("production names and categories come only from the glyphs: {e}")),
        },
    }
    for g in &mut resolved.glyphs {
        if g.category.as_deref() == Some("Mark") && g.sub_category.as_deref() == Some("Nonspacing") {
            g.width = 0.0;
        }
    }
}

/// The glyph database the compile reads categories from: the options' (a document's custom glyph
/// data on top of the built-in one), else the built-in one; `None` when neither can be read.
pub(crate) fn compile_glyph_data(options: &Options) -> Option<Arc<GlyphData>> {
    options.glyph_data.0.clone().or_else(|| GlyphData::new().ok().map(Arc::new))
}

/// OpenType rounding: half up (`floor(v + 0.5)`), not half away from zero. A value that is not
/// finite is 0.
pub fn ot_round(v: f64) -> f64 {
    if v.is_finite() { (v + 0.5).floor() } else { 0.0 }
}

/// [`ot_round`] into the 16-bit signed range the tables store, saturating.
pub(crate) fn i16_of(v: f64) -> i16 {
    ot_round(v).clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// [`ot_round`] into the 16-bit unsigned range the tables store, saturating.
pub(crate) fn u16_of(v: f64) -> u16 {
    ot_round(v).clamp(0.0, f64::from(u16::MAX)) as u16
}

#[cfg(test)]
mod instance_names_tests;
#[cfg(test)]
mod tests;
