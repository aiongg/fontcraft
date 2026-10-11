//! Our TrueType font programs: the `cvt ` layout (zones, then stems), the `fpgm` function
//! library and the `prep` program.
//!
//! Contract (`docs/plans/M10.md` § 3 › Our TrueType programs): `fpgm` holds functions for zone
//! rounding (with the rounding threshold, doubled for `xHeight`), stem widths per rendering
//! intent, the five rounding modes (the GDI-only mode through GETINFO), triple hints, the minimum
//! distance and diagonal preservation; `prep` aligns zones, applies per-PPM zone and stem deltas,
//! suppresses overshoots below their PPM and sets up instruction control. Written from the
//! TrueType instruction set specification; nothing is taken from the reference's output.
//! Programs are assembled with `fontcraft-ttasm`, whose [`fontcraft_ttasm::analyse`] gives the
//! `maxp` limits. Rows: `hinting.tt.export.generatedPrograms`, `hinting.tt.param.ttfBlueFuzz`,
//! `hinting.tt.param.zoneRoundingThreshold`, `hinting.tt.param.overshootSuppressionBelowPpm`,
//! `hinting.tt.param.minimumDistance`, `hinting.tt.zones.align`, `hinting.tt.zones.deltas`,
//! `hinting.tt.stems.deltas`, `TTFFamilyZonesThreshold`, the `cvt `, `fpgm`, `prep` table rows.
//! M10.17 fills the bodies marked `// M10.17:`.

use fontcraft_ttasm::ProgramLimits;

use crate::{HintSettings, HintingError, MAX_CVT_ENTRIES};

/// Where each zone and stem sits in `cvt `.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CvtLayout {
    /// Per TT zone (in [`HintSettings::tt_zones`] order): the entries of its flat edge and of its
    /// overshoot edge.
    pub zones: Vec<(u16, u16)>,
    /// Per TT stem (in [`HintSettings::tt_stems`] order): its width entry.
    pub stems: Vec<u16>,
    /// Entries used in all, at most [`MAX_CVT_ENTRIES`].
    pub len: u16,
}

impl CvtLayout {
    /// The layout for `settings`: zones first, then stems.
    pub fn new(settings: &HintSettings) -> Result<Self, HintingError> {
        let n = settings.tt_zones.len() * 2 + settings.tt_stems.len();
        if n > MAX_CVT_ENTRIES {
            return Err(HintingError::LimitExceeded { what: "cvt entries", limit: MAX_CVT_ENTRIES });
        }
        // M10.17: the layout (and any fixed entries the fpgm functions need).
        Ok(Self::default())
    }

    pub fn zone(&self, index: usize) -> Option<(u16, u16)> {
        self.zones.get(index).copied()
    }

    pub fn stem(&self, index: usize) -> Option<u16> {
        self.stems.get(index).copied()
    }
}

/// The font-wide TrueType programs of one master or instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontPrograms {
    pub cvt: Vec<i16>,
    pub fpgm: Vec<u8>,
    pub prep: Vec<u8>,
    /// What `fpgm` and `prep` need of `maxp`; the compiler merges the glyph programs' limits.
    pub limits: ProgramLimits,
    pub layout: CvtLayout,
}

/// The programs for `settings` at `units_per_em`. Imported programs (cvt Table, prep and fpgm
/// Table Assembly) are the compiler's business, not this function's.
pub fn font_programs(settings: &HintSettings, units_per_em: u16) -> Result<FontPrograms, HintingError> {
    let layout = CvtLayout::new(settings)?;
    // M10.17: cvt values, fpgm library, prep.
    let _ = units_per_em;
    Ok(FontPrograms { layout, ..FontPrograms::default() })
}
