//! FontCraft compiler (L3): source model → OpenType binary.
//!
//! Compiling runs in stages, each a function of its own that reads only what the stages before it
//! produced (`docs/spec/40-compile.md` § 1, `compile.pipelineStages`):
//!
//! | Stage | Where | What it does |
//! |---|---|---|
//! | Source preparation | [`prepare_source`] | corner components fitted into their paths, then smart components drawn as outlines, as both reference compilers do when they load a source |
//! | Resolve | [`drawing::resolve`] | one master, or an instance at a master (an instance elsewhere first becomes a one-master font, [`sources_at`]), as the compiler reads it: glyphs, contours, components, metrics, names; strokes expanded and masks applied |
//! | Preflight | [`preflight`] | component cycles refused, missing components dropped, non-exported glyphs inlined and removed, code point conflicts refused |
//! | Glyph data | [`fill_glyph_data`] | production names and categories a glyph does not store, from the glyph database; non-spacing marks get no advance |
//! | Export parameters | [`export_params`] | Export, Keep, Remove, Rename and Reencode Glyphs and the Arabic presentation forms change the resolved glyphs; the renames and removals go to the kerning writers |
//! | Glyph order | [`order`] | the export order, `.notdef` first and drawn when the font has none, `space` second with TrueType Keep GlyphOrder, at most 65,535 glyphs |
//! | Component options | [`components_opt`] | decomposition and flattening the options, parameters and filter list ask for |
//! | Layout | [`layout`] (with `kern` and `marks`) | the feature code for the target (automatic entries regenerated, instance parameters, tokens) compiled with the generated kerning and mark attachment: `GSUB`, `GPOS`, `GDEF`; a compile error stops the export |
//! | Outline preparation | [`outline`] | decomposition (mixed glyphs, transforms beyond ±2, everything for CFF), overlap removal, cubic to quadratic, contour direction |
//! | Hinting (M10) | [`cff_hints`], [`tt_hints`], [`points`], [`gasp`] | CFF stem hints, masks and flex; `cvt `, `fpgm`, `prep` and glyph programs against the point map; `gasp` |
//! | Table builders and assembly | `build`, [`cff`], [`names`], [`kern_table`] | `glyf`/`loca` with composites, or `CFF ` (cubic outlines in their drawn direction, decomposed); `hmtx`, `hhea`, `maxp`, `head`, `cmap`, `OS/2`, `name`, `post` with production names, the layout tables, then the font file |
//! | Extras | [`extras`] | `meta` from the meta Table parameter, `VORG` for CFF with vertical metrics; after assembly, the stored tables of an opened compiled font and a stub `DSIG` when the options ask |
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
pub mod cid;
pub mod color;
pub mod components_opt;
pub mod drawing;
pub mod export_params;
pub mod extras;
pub mod gasp;
pub mod info;
mod kern;
pub mod kern_table;
pub mod layout;
pub mod layout_tables;
pub mod makeotf;
mod marks;
pub mod math;
pub mod names;
pub mod order;
pub mod outline;
pub mod plan;
pub mod points;
pub mod preflight;
pub mod preview;
pub mod ranges;
pub mod thickness;
pub mod tt_hints;
pub mod var;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, ModelError, Resolved, ResolvedGlyph, Target};
use write_fonts::types::Tag;

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
    /// Glyphs whose layers are not compatible, so no variable font can be made
    /// (`compatibility.exportBlocked`); the text names them.
    #[error("the layers of these glyphs are not compatible, so the variable font cannot be made: {0}")]
    Incompatible(String),
    /// A stage or table builder whose package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
    /// The CID-keyed data or a CMap file cannot be used ([`cid`]); the text says why.
    #[error("{0}")]
    Cid(String),
}

/// Options of [`compile`].
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Creation and modification time, in seconds since the Unix epoch. The compiler never reads
    /// the clock; callers that want "now" pass it (the engine does, and honours `SOURCE_DATE_EPOCH`).
    pub timestamp: i64,
    /// The most a quadratic approximation may be from the cubic curve it replaces, in em (the
    /// distance is this times the units per em). `None`, or a value that is not a positive
    /// finite number: the source's TrueType Curve Error, else [`DEFAULT_CONVERSION_ERROR`].
    pub conversion_error: Option<f64>,
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
    /// For a static instance compiled from its interpolated font ([`instance_feature_axes`]):
    /// the source's axes and masters and the instance's internal location, which the variable
    /// feature syntax (`(wght:700) 14`, `condition`) reads. The interpolated font has one
    /// master, so it no longer holds them. `None` reads them from the compiled font.
    pub feature_axes: Option<(fontcraft_features::fea_variable::FeatureAxes, Vec<f64>)>,
    /// Move repeated charstring fragments of a `CFF ` or variable `CFF2` table into subroutines
    /// (`compile.cffSubroutinisation`), as fontmake does by default. `None`: on, unless the
    /// source's Disable Subroutines parameter is set. A table the subroutiniser refuses (one
    /// beyond its limits) is written without subroutines, with a warning.
    pub subroutinize: Option<bool>,
    /// What a project compile adds to the built font (`makeotf::finish`); `None` otherwise.
    pub project: Option<makeotf::Finish>,
    /// How TrueType curves are made (`compile.ttfCurveModes`): converted from cubic (the
    /// default) or written as the source draws them.
    pub curves: outline::CurveMode,
    /// Leave out of TrueType glyphs every on-curve point exactly halfway between two off-curve
    /// points (`compile.dropImpliedOnCurves`; on by default, as fontc writes them). Off: only the
    /// points the conversion and the source's quadratic splines imply are left out. A variable
    /// font always leaves out the points that are implied in every source.
    pub drop_implied_on_curves: bool,
    /// Apply the Keep Glyphs and Remove Glyphs parameters (`compile.subsetOnExport`; on by
    /// default). Off builds every glyph whose export flag is set, as the reference compiler's
    /// `--no-subset` does.
    pub subset: bool,
    /// Add an empty `DSIG` table ([`makeotf::STUB_DSIG`]), for software that looks for one.
    pub stub_dsig: bool,
    /// Copy the tables a compiled font kept when it was opened ([`extras::stored_tables`]) into
    /// the build when the glyph order is still theirs (`roundtrip.binaryPassThroughTables`).
    pub pass_through_tables: bool,
    /// The CMap files and Macintosh `cmap` ids of a CID-keyed CFF build ([`cid`]).
    pub cid: cid::CidOptions,
    /// Write an empty `GSUB` table when the layout stage gives none, for software that expects
    /// one (`--stub-gsub`). Not with the features skipped.
    pub stub_gsub: bool,
    /// How the charstrings of a static `CFF ` table are written.
    pub cff: CffOptions,
    /// Set `USE_MY_METRICS` on the component that gives a TrueType composite its advance
    /// (`compile.useMyMetrics`); false writes the flag on no component.
    pub use_my_metrics: bool,
    /// The `post` format of TrueType outlines (2 or 3), over the Post Table Type parameter.
    /// `None`: the parameter, else 2.
    pub post_format: Option<u8>,
    /// The font goes into a web container (WOFF or WOFF2), which Remove post names for webfonts
    /// and Webfont Only read ([`export_params::differs_by_container`]).
    pub web_container: bool,
}

impl Options {
    /// The cubic to quadratic tolerance of `resolved`, in font units ([`Self::conversion_error`]).
    pub(crate) fn conversion_tolerance(&self, resolved: &Resolved) -> f64 {
        let upm = f64::from(resolved.units_per_em);
        match self.conversion_error.filter(|e| e.is_finite() && *e > 0.0) {
            Some(em) => upm * em,
            None => export_params::curve_error(&resolved.params, upm).unwrap_or(upm * DEFAULT_CONVERSION_ERROR),
        }
    }
}

/// Most subroutines one CFF index can hold, the largest [`CffOptions::max_subrs`].
pub const MAX_SUBRS_PER_INDEX: usize = fontcraft_cff::subr::MAX_SUBRS_PER_INDEX;

/// How the charstrings of a static `CFF ` table are written ([`Options::cff`]). A project compile
/// keeps coordinates to two decimals whatever `round_tolerance` says.
#[derive(Clone, Debug, PartialEq)]
pub struct CffOptions {
    /// Round a coordinate to an integer only when that moves it by at most this much
    /// (`compile.cffRoundTolerance`): 0.5 rounds every coordinate, 0 none. `None`: hundredths of
    /// a unit when the source's grid is finer than a unit (`fontInfo.grid.fractionalCff`), else
    /// 0.5.
    pub round_tolerance: Option<f64>,
    /// Write the specialised operators (`hlineto`, `vhcurveto`…), not only `rlineto` and
    /// `rrcurveto`.
    pub specialise: bool,
    /// Most subroutines in one index when the table is subroutinised.
    pub max_subrs: usize,
    /// Choose `defaultWidthX` and `nominalWidthX` so the widths take the fewest bytes, before the
    /// table is subroutinised.
    pub optimize_widths: bool,
}

impl Default for CffOptions {
    fn default() -> Self {
        CffOptions { round_tolerance: None, specialise: true, max_subrs: fontcraft_cff::subr::DEFAULT_MAX_SUBRS, optimize_widths: false }
    }
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
            conversion_error: None,
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
            feature_axes: None,
            subroutinize: None,
            project: None,
            curves: outline::CurveMode::Cu2Qu,
            drop_implied_on_curves: true,
            subset: true,
            stub_dsig: false,
            pass_through_tables: false,
            cid: cid::CidOptions::default(),
            stub_gsub: false,
            cff: CffOptions::default(),
            use_my_metrics: true,
            post_format: None,
            web_container: false,
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
    /// The feature code the layout stage compiled
    /// ([`layout::LayoutTables::assembled_code`]); `None` without code or with the features
    /// skipped.
    pub feature_code: Option<String>,
}

/// Compiles `font` at `target` (a master, or a static instance anywhere) to a static font.
pub fn compile(font: &Font, target: &Target, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    font.check()?;
    color::check_instance(font, target)?;
    let mut notes = Vec::new();
    let (source, prepared) = sources_at(font, target, &mut notes)?;
    let resolved = drawing::resolve(&prepared, target)?;
    let options = with_instance_feature_axes(options, font, target);
    let mut compiled = compile_resolved_from(&source, resolved, flavour, &options)?;
    notes.append(&mut compiled.warnings);
    compiled.warnings = notes;
    Ok(compiled)
}

/// `font` as the compile stages read it: the cap components of every layer fitted into its paths
/// with the corners of those layers ([`fontcraft_model::attach::insert_all_attachments`]), the
/// corner components of the other layers ([`fontcraft_model::corner::insert_all_corner_components`]), then every smart component
/// drawn as outlines ([`fontcraft_model::smart::draw_smart_components`]), the order in which
/// both reference compilers apply them when they load a source. Corners go first because a
/// smart glyph's pole layers can hold corners. Last, components pinned to another master or layer
/// become the paths they draw ([`fontcraft_model::pinned::draw_pinned_components`]). A layer whose corners cannot be fitted, and a
/// smart component that cannot be drawn, stay as they are, with a warning. A font with none
/// is borrowed, not copied. Callers of [`compile_resolved_from`] resolve the prepared font and
/// pass the source as it is stored.
pub fn prepare_source<'a>(font: &'a Font, warnings: &mut Vec<String>) -> Cow<'a, Font> {
    let mut out = Cow::Borrowed(font);
    let has_attachments = font.glyphs.iter().any(|g| g.layers.iter().any(|l| l.info.hints.iter().any(fontcraft_model::attach::is_attachment)));
    if has_attachments {
        let (fitted, report) = fontcraft_model::attach::insert_all_attachments(&out);
        warnings.extend(
            report
                .failed
                .into_iter()
                .map(|e| format!("cap components, segment components and brushes were not fitted, so the paths were written without them: {e}")),
        );
        out = Cow::Owned(fitted);
    }
    let has_corners = out.glyphs.iter().any(|g| g.layers.iter().any(|l| l.info.hints.iter().any(fontcraft_model::corner::is_corner)));
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
    let pinned = |l: &fontcraft_model::Layer| {
        l.shapes.iter().any(|s| matches!(s, fontcraft_model::Shape::Component(c) if fontcraft_model::pinned::is_redirected(&out, c, l)))
    };
    if out.glyphs.iter().any(|g| g.layers.iter().any(pinned)) {
        let (drawn, report) = fontcraft_model::pinned::draw_pinned_components(&out);
        warnings.extend(report.kept.into_iter().map(|e| format!("a pinned component was written from its glyph's own layer: {e}")));
        out = Cow::Owned(drawn);
    }
    out
}

/// `font` as stored and as prepared ([`prepare_source`]) for a compile at `target`: for a static
/// instance between or beyond the masters, or at a master where an alternate layer applies, both
/// are the font [`fontcraft_interp::instance::instance_font`] makes at the instance, which
/// resolves at `target` like any instance at a master. The prepared font is interpolated after
/// corners and smart components are drawn into every master.
pub fn sources_at<'a>(font: &'a Font, target: &Target, warnings: &mut Vec<String>) -> Result<(Cow<'a, Font>, Cow<'a, Font>), CompileError> {
    let prepared = prepare_source(font, warnings);
    let Some(index) = needs_instance_font(font, target) else { return Ok((Cow::Borrowed(font), prepared)) };
    let source = instance_source(font, index, warnings)?;
    let prepared = match prepared {
        Cow::Borrowed(_) => source.clone(),
        Cow::Owned(p) => instance_source(&p, index, &mut Vec::new())?,
    };
    Ok((Cow::Owned(source), Cow::Owned(prepared)))
}

/// The static instance `target` names when the compile must interpolate it: it lies away from
/// every master, it sets manual weights, InterpolationWeightY or Local Interpolation, it lies
/// at a master where some glyph's alternate layer applies, a designspace rule holds there, or a
/// glyph has a source left out of interpolation.
pub fn needs_instance_font(font: &Font, target: &Target) -> Option<usize> {
    let Target::Instance(index) = *target else { return None };
    let inst = font.instances.get(index).filter(|i| i.kind.is_none())?;
    if font.master_at(&inst.location).is_none()
        || fontcraft_interp::instance::adjusts_interpolation(font, index)
        || fontcraft_interp::sources::has_inactive(font)
    {
        return Some(index);
    }
    // A designspace rule that holds at the instance swaps glyphs, which the instance font does.
    if fontcraft_model::is_ufo_family(font.source.format)
        && fontcraft_interp::doc_rules::document_rules(font)
            .is_some_and(|r| !fontcraft_interp::doc_rules::swaps_at(&r, &inst.location, |n| font.glyph(n).is_some()).is_empty())
    {
        return Some(index);
    }
    let has_alternates =
        font.glyphs.iter().any(|g| g.layers.iter().any(|l| l.info.attrs.axis_rules.iter().any(|r| r.min.is_some() || r.max.is_some())));
    if !has_alternates {
        return None;
    }
    let designspace = fontcraft_interp::Designspace::new(font).ok()?;
    let location = designspace.instance_location(font, index).ok()?;
    font.glyphs
        .iter()
        .any(|g| {
            g.layers.iter().any(|l| {
                !l.info.attrs.axis_rules.is_empty()
                    && fontcraft_interp::RangeSet::from_rules(&designspace, &l.info.attrs.axis_rules)
                        .is_ok_and(|r| r.axes.iter().any(|a| a.min.is_some() || a.max.is_some()) && r.contains(&location))
            })
        })
        .then_some(index)
}

/// [`Options::feature_axes`] for compiling `target` of `font` from its interpolated instance
/// font ([`needs_instance_font`]); `None` for any other target.
pub fn instance_feature_axes(font: &Font, target: &Target) -> Option<(fontcraft_features::fea_variable::FeatureAxes, Vec<f64>)> {
    let index = needs_instance_font(font, target)?;
    let inst = font.instances.get(index)?;
    let location = (0..font.axes.len()).map(|i| fontcraft_model::axes::design_value(&inst.location, i)).collect();
    Some((fontcraft_features::fea_variable::FeatureAxes::from_font(font), location))
}

/// `options` with [`Options::feature_axes`] set for `target` of `font` when it is compiled from
/// its interpolated instance font.
pub fn with_instance_feature_axes<'a>(options: &'a Options, font: &Font, target: &Target) -> Cow<'a, Options> {
    match instance_feature_axes(font, target) {
        Some(axes) => Cow::Owned(Options { feature_axes: Some(axes), ..options.clone() }),
        None => Cow::Borrowed(options),
    }
}

/// The instance font of the static instance at `index`, its notes added to `warnings`.
fn instance_source(font: &Font, index: usize, warnings: &mut Vec<String>) -> Result<Font, CompileError> {
    let options = fontcraft_interp::instance::options_for(font, index)?;
    let staged = tt_hints::with_tt_values(font);
    let (mut out, notes) = fontcraft_interp::instance::instance_font_with_notes(staged.as_ref().unwrap_or(font), index, &options)?;
    warnings.extend(notes);
    color::variable::interpolate_color_layers(font, index, &options, &mut out, warnings)?;
    tt_hints::finish_instance(font, &mut out);
    Ok(out)
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

/// Tables and name records a variable compile adds to its origin's static tables. A name ID
/// below 256 replaces the static records of that ID.
#[derive(Clone, Debug, Default)]
pub(crate) struct Extra {
    pub tables: Vec<(Tag, Vec<u8>)>,
    pub names: Vec<(u16, String)>,
    /// The `CFF2` table of a CFF2 variable font, which replaces the static `CFF ` table.
    pub cff2: Option<fontcraft_cff::cff2::Cff2Built>,
    /// The source's math data, written to `MATH` with the glyph boxes.
    pub math: Option<math::MathPlan>,
    /// A CID-keyed CFF build ([`cid::plan`]).
    pub cid: Option<cid::CidPlan>,
    /// The masters a variable TrueType font's `cvar` varies with, once its `cvt ` is known.
    pub cvar: Option<var::cvar::CvarMasters>,
    /// `CPAL`, built with the `name` table because its labels take IDs above the other names.
    pub cpal: Option<color::PendingCpal>,
}

/// The outline stage of a variable compile: given the origin's final glyphs, the outlines to
/// write and the variation tables. Static compiles have none.
pub(crate) type VarStage<'a> =
    &'a mut dyn FnMut(&Resolved, &[ResolvedGlyph], &outline::Settings, &mut Vec<String>) -> Result<(Vec<outline::Prepared>, Extra), CompileError>;

fn compile_stages(font: Option<&Font>, resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    let mut compiled = compile_stages_with(font, resolved, flavour, options, None, None)?;
    if let Some(finish) = &options.project {
        makeotf::finish(&mut compiled, finish)?;
    }
    Ok(compiled)
}

/// The glyphs of `resolved` as every compile stage before the outlines leaves them: glyph data,
/// tokens, export parameters, preflight, the final order and the component options.
pub(crate) fn glyph_stages(
    font: Option<&Font>,
    resolved: &mut Resolved,
    flavour: Flavour,
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Vec<ResolvedGlyph>, CompileError> {
    fill_glyph_data(resolved, options, warnings);
    info::expand_tokens(font, resolved, warnings);
    export_params::without_subset_lists(resolved, options.subset);
    export_params::apply(font, resolved, warnings)?;
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), warnings)?;
    let glyphs = order::final_order(glyphs, resolved, flavour, warnings)?;
    let mut expansion = Expansion::new(MAX_EXPANDED_ELEMENTS);
    let components = tt_hints::component_options(font, resolved, flavour, options);
    components_opt::apply(font, resolved, glyphs, flavour, &components, &mut expansion, warnings)
}

pub(crate) fn compile_stages_with(
    font: Option<&Font>,
    mut resolved: Resolved,
    flavour: Flavour,
    options: &Options,
    var: Option<VarStage<'_>>,
    var_layout: Option<&layout::VariableLayout<'_>>,
) -> Result<Compiled, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    fill_glyph_data(&mut resolved, options, &mut warnings);
    info::expand_tokens(font, &mut resolved, &mut warnings);
    export_params::without_subset_lists(&mut resolved, options.subset);
    let edits = export_params::apply(font, &mut resolved, &mut warnings)?;
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let synthesised_notdef = !glyphs.iter().any(|g| g.name == ".notdef");
    let glyphs = order::final_order(glyphs, &resolved, flavour, &mut warnings)?;
    let (glyphs, cid_plan) = match font {
        Some(font) if flavour == Flavour::Cff && var.is_none() => cid::plan(font, &resolved.params, glyphs, &options.cid, &mut warnings)?,
        _ => (glyphs, None),
    };
    preflight::check_cmap(&glyphs)?;
    let glyph_data = font.and_then(|_| compile_glyph_data(options));
    let mut color_plan = if var.is_some() {
        color::variable_plan(
            font,
            &resolved.master,
            &fontcraft_color::export::Switches { cbdt: options.cbdt, ..fontcraft_color::export::Switches::from_params(&resolved.params) },
            var_layout.map_or(&[][..], |v| v.layout.masters()),
        )
    } else {
        color::plan(font, &resolved, options)
    };
    let (glyphs, made) = color::append_layer_glyphs(font, &mut color_plan, &resolved, glyphs, &mut warnings)?;
    let layout = layout_tables(font, glyph_data.as_deref(), &resolved, &glyphs, &edits, options, var_layout, &mut warnings)?;
    let mut expansion = Expansion::new(MAX_EXPANDED_ELEMENTS);
    let components = tt_hints::component_options(font, &resolved, flavour, options);
    let glyphs = components_opt::apply(font, &resolved, glyphs, flavour, &components, &mut expansion, &mut warnings)?;
    let settings = outline::Settings {
        flavour,
        tolerance: options.conversion_tolerance(&resolved),
        keep_direction: options.keep_direction,
        curves: options.curves,
        drop_implied: options.drop_implied_on_curves,
        max_expanded: expansion.left,
        remove_overlap: options.remove_overlap,
        overlap_time_limit: options.overlap_time_limit,
        single_precision: flavour == Flavour::Cff && options.project.as_ref().is_some_and(|p| p.two_decimals),
    };
    let (outlines, mut extra, kern_table) = match var {
        Some(stage) => {
            let (outlines, extra) = stage(&resolved, &glyphs, &settings, &mut warnings)?;
            (outlines, extra, None)
        }
        None => {
            let outlines = outline::prepare(&glyphs, &settings, &mut warnings)?;
            let kern_table = font.and_then(|font| kern_table::build(font, &resolved, &glyphs, flavour, &edits, &mut warnings));
            (outlines, Extra::default(), kern_table)
        }
    };
    extra.math = math::plan(font, &resolved, &edits);
    extra.cid = cid_plan;
    let color_tables = color::build(font, &color_plan, &color::GlyphOrder::new(&glyphs, &made)?, options, &mut warnings)?;
    extra.cpal = color_tables.cpal;
    let bytes = build::assemble(
        font,
        &resolved,
        &glyphs,
        &outlines,
        flavour,
        synthesised_notdef,
        &layout,
        kern_table.as_deref(),
        &color_tables.tables,
        &extra,
        options,
        &mut warnings,
    )?;
    let mut compiled = Compiled { bytes, glyph_order: glyphs.into_iter().map(|g| g.name).collect(), warnings, feature_code: layout.code.clone() };
    if let Some(plan) = &extra.cid {
        cid::finish(&mut compiled, plan)?;
    }
    if let Some(uvs) = &options.cid.uvs {
        cid::uvs::finish(&mut compiled, uvs, extra.cid.as_ref())?;
    }
    extras::finish(font, &mut compiled, options)?;
    Ok(compiled)
}

/// The layout stage ([`layout::build`]), variable when `variable` gives the masters; nothing when
/// the options skip features.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_tables(
    font: Option<&Font>,
    data: Option<&GlyphData>,
    resolved: &Resolved,
    order: &[fontcraft_model::ResolvedGlyph],
    edits: &export_params::ExportEdits,
    options: &Options,
    variable: Option<&layout::VariableLayout<'_>>,
    warnings: &mut Vec<String>,
) -> Result<layout::LayoutTables, CompileError> {
    if options.skip_features {
        return Ok(layout::LayoutTables::default());
    }
    // The code's own name IDs start above the source's (`compile.nameFeaOverrides`).
    let first_name_id = match font {
        Some(f) if !f.features.is_empty() => info::first_free_name_id(resolved, &options.style_name),
        _ => 256,
    };
    let input = layout::LayoutInput {
        font,
        data,
        resolved,
        order,
        edits,
        first_name_id,
        include_dir: options.feature_dir.as_deref(),
        include_check: options.include_check.as_ref(),
        variable,
        feature_axes: options.feature_axes.as_ref(),
    };
    // A project compile groups rules into lookups as the reference's feature compiler does.
    let layout_options = layout::LayoutOptions { lookup_per_rule_type: options.project.is_some(), ..Default::default() };
    let mut tables = layout::build(&input, &layout_options, warnings)?;
    if options.stub_gsub && tables.compiled.gsub.is_none() {
        tables.compiled.gsub = Some(STUB_GSUB.to_vec());
    }
    Ok(tables)
}

/// An empty `GSUB` table, version 1.0: a script list, feature list and lookup list with no
/// records, each right after the header.
const STUB_GSUB: [u8; 16] = [0, 1, 0, 0, 0, 10, 0, 12, 0, 14, 0, 0, 0, 0, 0, 0];

/// Fills what the glyphs do not store from the glyph database ([`Resolved::apply_glyph_data`])
/// and gives non-spacing marks no advance (`compile.nonSpacingMarkWidth`), as fontc and glyphsLib
/// write them for the reference's format; a UFO states its advances as they are, and
/// DisableAllAutomaticBehaviour keeps them. A built-in
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
    if export_params::automatic_disabled(&resolved.params) {
        return;
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
mod subr_export_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tt_hints_tests;
