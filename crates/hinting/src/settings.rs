//! The hint settings of a master, or of an instance at a master: zones, stems, TT zones and
//! stems, the blue numbers, the TT numbers and the main master, each read with its scope (font,
//! master, instance) and its default.
//!
//! Defaults follow the handbook (owner decision M10 § 7, question 6): blueScale 0.037 and
//! blueFuzz 1 when the source sets nothing, blueShift 7; TTFBlueFuzz 1, TTFZoneRoundingThreshold
//! 0.09375, TTFMinimumDistance 0.25 px. TT zones default to the master's alignment zones and TT
//! stems to its standard stems when TTFZones / TTFStems are absent. Rows:
//! `hinting.ps.standardStems*`, `hinting.ps.alignmentZones*`, `hinting.ps.blueScale`,
//! `hinting.ps.blueShift`, `hinting.ps.blueFuzz`, `hinting.ps.familyAlignmentZones`,
//! `hinting.ps.masters.*`, `hinting.tt.mainMasterOnly`, `hinting.tt.param.*`,
//! `hinting.tt.zones.defaultFromAlignmentZones`, `hinting.tt.stems.defaultFromStandardStems`,
//! `fontInfo.masters.firstMasterHints`, `fontInfo.stems.autohinterInput` and the M10 parameter
//! rows of `spec/inventory/18-custom-parameters.toml`. M10.2 fills the bodies marked `// M10.2:`.

use std::collections::BTreeMap;

use fontcraft_model::Font;

use crate::HintingError;

// Parameter names, as the reference spells them.
pub const GET_HINTS_FROM_MASTER: &str = "Get Hints From Master";
pub const FAMILY_ALIGNMENT_ZONES: &str = "Family Alignment Zones";
pub const BLUE_SCALE: &str = "blueScale";
pub const BLUE_SHIFT: &str = "blueShift";
pub const BLUE_FUZZ: &str = "blueFuzz";
pub const AUTOHINT: &str = "Autohint";
pub const DISABLE_AUTOHINTING_FOR_GLYPHS: &str = "Disable autohinting for glyphs";
pub const EXPORT_POSTSCRIPT_HINTS: &str = "Export PostScript Hints";
pub const EXPORT_TRUETYPE_HINTS: &str = "Export TrueType Hints";
pub const TTF_ZONES: &str = "TTFZones";
pub const TTF_STEMS: &str = "TTFStems";
pub const TTF_BLUE_FUZZ: &str = "TTFBlueFuzz";
pub const TTF_OVERSHOOT_SUPPRESSION_BELOW_PPM: &str = "TTFOvershootSuppressionBelowPPM";
pub const TTF_ZONE_ROUNDING_THRESHOLD: &str = "TTFZoneRoundingThreshold";
pub const TTF_MINIMUM_DISTANCE: &str = "TTFMinimumDistance";
pub const TTF_FAMILY_ZONES_THRESHOLD: &str = "TTFFamilyZonesThreshold";
pub const TTF_DONT_PRESERVE_DIAGONALS: &str = "TTFDontPreserveDiagonals";
pub const IGNORE_VERTICAL_HINTS: &str = "Ignore Vertical Hints";
pub const TTF_AUTOHINT_OPTIONS: &str = "TTFAutohint options";
pub const TTF_AUTOHINT_CONTROL_INSTRUCTIONS: &str = "TTFAutohint control instructions";
pub const TTF_AUTOHINT_BINARY_PATH: &str = "TTFAutohint binary path";
pub const CVT_TABLE: &str = "cvt Table";
pub const PREP_TABLE_ASSEMBLY: &str = "prep Table Assembly";
pub const FPGM_TABLE_ASSEMBLY: &str = "fpgm Table Assembly";
pub const GASP_TABLE: &str = "gasp Table";

/// Layer user-data key of an imported glyph program (assembly text), written when a hinted
/// TrueType font is opened and written back at export while the outline is unchanged.
pub const IMPORTED_GLYPH_PROGRAM_KEY: &str = "com.fontcraft.ttGlyphProgram";
/// Layer user-data key of the outline fingerprint the imported program belongs to.
pub const IMPORTED_OUTLINE_KEY: &str = "com.fontcraft.ttGlyphProgramOutline";

pub const DEFAULT_BLUE_SCALE: f64 = 0.037;
pub const DEFAULT_BLUE_SHIFT: f64 = 7.0;
pub const DEFAULT_BLUE_FUZZ: f64 = 1.0;
pub const DEFAULT_TT_BLUE_FUZZ: f64 = 1.0;
pub const DEFAULT_ZONE_ROUNDING_THRESHOLD: f64 = 0.09375;
pub const DEFAULT_MINIMUM_DISTANCE: f64 = 0.25;

/// A delta cell's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeltaDir {
    Up,
    Down,
}

/// Deltas: master or instance id → PPM (1 to [`crate::MAX_DELTA_PPM`]) → direction. At most
/// [`crate::MAX_DELTA_COLUMNS`] ids.
pub type Deltas = BTreeMap<String, BTreeMap<u16, DeltaDir>>;

/// A PostScript alignment zone: flat edge and signed size (positive: top zone).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PsZone {
    pub position: f64,
    pub size: f64,
}

/// A TT zone (TTFZones entry, or one derived from an alignment zone).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtZone {
    /// Unique among the master's TT zones. `xHeight` gets the rounding threshold twice.
    pub name: String,
    pub position: f64,
    /// Positive: top zone; negative: bottom zone.
    pub size: f64,
    /// The zone this one is aligned to, by name.
    pub align: Option<String>,
    /// Main master only.
    pub deltas: Deltas,
    /// Main master only: the glyphs the zone applies to, as a glyph predicate.
    pub filter: Option<String>,
}

/// A TT stem (TTFStems entry, or one derived from a standard stem).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TtStem {
    pub name: String,
    /// Measured vertically (the thickness of a horizontal stroke; the editor's Y direction).
    pub horizontal: bool,
    pub size: f64,
    /// Main master only.
    pub deltas: Deltas,
    /// Main master only.
    pub filter: Option<String>,
}

/// Everything hinting reads from a font for one master, or for an instance at a master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintSettings {
    /// The master whose hints count (Get Hints From Master, else the first master).
    pub main_master: String,
    /// The master these settings are for.
    pub master: String,
    /// The instance, when the settings are for one (its index in `Font::instances`).
    pub instance: Option<usize>,

    // PostScript
    /// Alignment zones (metric overshoots plus the master's extra zones), as written to the
    /// Private dict before the CFF limits are applied.
    pub zones: Vec<PsZone>,
    pub family_zones: Vec<PsZone>,
    /// Standard stems, by direction, in source order.
    pub stems_h: Vec<f64>,
    pub stems_v: Vec<f64>,
    /// `None`: the source sets none; export writes [`DEFAULT_BLUE_SCALE`] and the others.
    pub blue_scale: Option<f64>,
    pub blue_shift: Option<f64>,
    pub blue_fuzz: Option<f64>,
    /// Autohint at export (instance parameter, else the export option).
    pub autohint: Option<bool>,
    pub disable_autohint_glyphs: Vec<String>,
    pub export_ps_hints: bool,

    // TrueType
    pub tt_zones: Vec<TtZone>,
    pub tt_stems: Vec<TtStem>,
    pub tt_blue_fuzz: f64,
    pub tt_overshoot_suppression_below_ppm: Option<u16>,
    pub tt_zone_rounding_threshold: f64,
    pub tt_minimum_distance: f64,
    pub tt_family_zones_threshold: Option<u16>,
    pub tt_dont_preserve_diagonals: bool,
    pub ignore_vertical_hints: bool,
    pub export_tt_hints: bool,
    /// TTFAutohint options and control instructions as stored (parsed by [`crate::tt_auto`] and
    /// [`crate::control`]); the binary path (desktop only).
    pub tt_autohint_options: Option<String>,
    pub tt_autohint_control: Option<String>,
    pub tt_autohint_binary: Option<String>,
    /// Imported hinting kept on the master (cvt Table, prep and fpgm Table Assembly).
    pub cvt: Option<Vec<i16>>,
    pub prep_assembly: Option<String>,
    pub fpgm_assembly: Option<String>,
    /// gasp Table: (maximum PPM, flags), sorted by PPM.
    pub gasp: Option<Vec<(u16, u16)>>,
}

impl HintSettings {
    /// The settings of master `master`.
    pub fn for_master(font: &Font, master: &str) -> Result<Self, HintingError> {
        font.master(master).map_err(|_| HintingError::MissingMaster(master.to_string()))?;
        // M10.2: every parameter with its scope and default.
        Ok(Self {
            main_master: font.masters.first().map(|m| m.id.clone()).unwrap_or_default(),
            master: master.to_string(),
            export_ps_hints: true,
            export_tt_hints: true,
            tt_blue_fuzz: DEFAULT_TT_BLUE_FUZZ,
            tt_zone_rounding_threshold: DEFAULT_ZONE_ROUNDING_THRESHOLD,
            tt_minimum_distance: DEFAULT_MINIMUM_DISTANCE,
            ..Self::default()
        })
    }

    /// The settings of instance `instance` built at master `master` (instance parameters
    /// override font and master ones).
    pub fn for_instance(font: &Font, instance: usize, master: &str) -> Result<Self, HintingError> {
        // M10.2: instance scopes.
        let mut s = Self::for_master(font, master)?;
        s.instance = Some(instance);
        Ok(s)
    }

    /// The TT zones that apply to `glyph` (filters evaluated with `fontcraft-glyphquery`).
    pub fn tt_zones_for(&self, font: &Font, glyph: &str) -> Vec<usize> {
        // M10.2: filters.
        let _ = (font, glyph);
        (0..self.tt_zones.len()).collect()
    }

    /// The TT stems that apply to `glyph`.
    pub fn tt_stems_for(&self, font: &Font, glyph: &str) -> Vec<usize> {
        // M10.2: filters.
        let _ = (font, glyph);
        (0..self.tt_stems.len()).collect()
    }
}
