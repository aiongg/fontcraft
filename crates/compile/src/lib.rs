//! FontCraft compiler (L3): source model → OpenType binary.
//!
//! Compiling runs in stages, each a function of its own that reads only what the stages before it
//! produced (`docs/spec/40-compile.md` § 1, `compile.pipelineStages`):
//!
//! | Stage | Where | What it does |
//! |---|---|---|
//! | Resolve | [`drawing::resolve`] | one master, or an instance at a master, as the compiler reads it: glyphs, contours, components, metrics, names; strokes expanded and masks applied |
//! | Preflight | [`preflight`] | component cycles refused, missing components dropped, non-exported glyphs inlined and removed, code point conflicts refused |
//! | Glyph order | [`order`] | the font's order, `.notdef` first and drawn when the font has none, at most 65,535 glyphs |
//! | Outline preparation | [`outline`] | decomposition (mixed glyphs, transforms beyond ±2, everything for CFF), overlap removal, cubic to quadratic, contour direction |
//! | Table builders and assembly | `build`, [`cff`] | `glyf`/`loca` with composites, or `CFF ` (cubic outlines in their drawn direction, decomposed); `hmtx`, `hhea`, `maxp`, `head`, `cmap`, `OS/2`, `name`, `post`, then the font file |
//!
//! The output is a **static font** of one master, with TrueType outlines ([`Flavour::TrueType`],
//! as fontc builds it) or CFF outlines ([`Flavour::Cff`], as fontmake builds it: `maxp` 0.5, `post`
//! 3.0, glyph names in the CFF charset, metrics from the charstrings' bounds). It is
//! deterministic: the same font and options give the same bytes. Rounding is half up everywhere
//! (OpenType rounding, `compile.rounding`): point coordinates when the glyph is written,
//! component offsets, advances.
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

mod build;
pub mod cff;
pub mod drawing;
pub mod info;
pub mod order;
pub mod outline;
pub mod plan;
pub mod preflight;
mod ranges;

use std::time::Duration;

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
    let resolved = drawing::resolve(font, target)?;
    compile_resolved(resolved, flavour, options)
}

/// Compiles the first master of `font` to a static TrueType font.
pub fn compile_ttf(font: &Font, options: &Options) -> Result<Compiled, CompileError> {
    compile(font, &font.default_target()?, Flavour::TrueType, options)
}

/// The stages after resolving, on a resolved font.
pub fn compile_resolved(mut resolved: Resolved, flavour: Flavour, options: &Options) -> Result<Compiled, CompileError> {
    let mut warnings = std::mem::take(&mut resolved.notes);
    // Non-spacing marks have no advance in a compiled font (`compile.nonSpacingMarkWidth`), as
    // fontc and glyphsLib write them for `.glyphs` sources; a UFO states its advances as they
    // are. Only the categories glyphs state are known until the glyph database (M4).
    if !fontcraft_model::is_ufo_family(resolved.format) {
        for g in &mut resolved.glyphs {
            if g.category.as_deref() == Some("Mark") && g.sub_category.as_deref() == Some("Nonspacing") {
                g.width = 0.0;
            }
        }
    }
    let glyphs = preflight::run(std::mem::take(&mut resolved.glyphs), &mut warnings)?;
    let synthesised_notdef = !glyphs.iter().any(|g| g.name == ".notdef");
    let glyphs = order::final_order(glyphs, &resolved, &mut warnings)?;
    preflight::check_cmap(&glyphs)?;
    let upm = f64::from(resolved.units_per_em);
    let error =
        if options.conversion_error.is_finite() && options.conversion_error > 0.0 { options.conversion_error } else { DEFAULT_CONVERSION_ERROR };
    let settings = outline::Settings {
        flavour,
        tolerance: upm * error,
        keep_direction: options.keep_direction,
        max_expanded: MAX_EXPANDED_ELEMENTS,
        remove_overlap: options.remove_overlap,
        overlap_time_limit: options.overlap_time_limit,
    };
    let outlines = outline::prepare(&glyphs, &settings, &mut warnings)?;
    let bytes = build::assemble(&resolved, &glyphs, &outlines, flavour, synthesised_notdef, options, &mut warnings)?;
    Ok(Compiled { bytes, glyph_order: glyphs.into_iter().map(|g| g.name).collect(), warnings })
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
mod tests;
