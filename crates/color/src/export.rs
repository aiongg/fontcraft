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

use fontcraft_model::{Font, Params, Target};

use crate::paint::Paint;
use crate::strike::Strike;
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
        // M11.6
        let _ = params;
        Switches::default()
    }
}

/// The switches of an export of `target`.
pub fn read_switches(font: &Font, target: &Target) -> Result<Switches, ColorError> {
    // M11.6
    let _ = (font, target);
    Ok(Switches::default())
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

/// The plan of an export of `target`: its switches read from the instance, the master's layers.
pub fn plan(font: &Font, target: &Target) -> ExportPlan {
    // M11.6
    let _ = (font, target);
    ExportPlan::default()
}

/// The plan of an export of the layers of `master_id` with `switches`, which is what the
/// compiler runs on a resolved font.
pub fn plan_master(font: &Font, master_id: &str, switches: &Switches) -> ExportPlan {
    // M11.6
    let _ = (font, master_id, switches);
    ExportPlan::default()
}
