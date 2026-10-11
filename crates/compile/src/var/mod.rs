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

use fontcraft_model::Font;
use write_fonts::types::Tag;

pub use fontcraft_features::var_layout::VarValue;
pub use fontcraft_geom::variation::{Region, RegionAxis};

use crate::{CompileError, Compiled, Options};

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
        let _ = designspace;
        Err(CompileError::NotYetImplemented("variable font axes"))
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

/// Compiles the variable font of `setting` (an index into `Font::instances`; `None` for the
/// whole designspace). Refuses incompatible glyphs, a designspace without a usable origin, and
/// everything the static compile refuses.
pub fn compile_variable(font: &Font, setting: Option<usize>, options: &Options, var: &VariableOptions) -> Result<VariableFont, CompileError> {
    // M9.5
    let _ = (font, setting, options, var);
    Err(CompileError::NotYetImplemented("variable fonts"))
}
