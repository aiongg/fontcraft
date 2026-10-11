//! The TrueType autohinter: Snap and Stem instructions from the PostScript autohinter's zone
//! and stem analysis, compiled by the manual-instruction compiler (owner decision M10 § 7,
//! question 2 (a)). When TTFAutohint binary path names a ttfautohint program (desktop only), the
//! compiler runs that program on the finished font instead; that path is the compiler's.
//!
//! The TTFAutohint options that have a meaning here apply (hinting limit, x-height increase and
//! exceptions, fallback stem width, Windows compatibility, dehint, symbol, info, TTFA table); the
//! others are reported as ignored. Rows: `autohint.tt.run`, `hint.tt.autohintGlyph`,
//! `export.autohintTrueType`, `hinting.tt.export.autohintIgnoresManual`,
//! `hinting.tt.param.ttfAutohintOptions`, `autohint.tt.option.*`. M10.21 fills the bodies
//! marked `// M10.21:`.

use fontcraft_model::Font;

use crate::control::ControlInstructions;
use crate::{HintSettings, HintingError, TtInstruction};

/// How stems are rounded in one rendering mode (`--stem-width-mode` letters n, q, s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StemWidthMode {
    Natural,
    #[default]
    Quantized,
    Strong,
}

/// The TTFAutohint options, with ttfautohint 1.8's defaults where the handbook gives none.
#[derive(Clone, Debug, PartialEq)]
pub struct TtAutohintOptions {
    pub hinting_range_min: u16,
    pub hinting_range_max: u16,
    /// 0: no limit.
    pub hinting_limit: u16,
    /// 0: off.
    pub increase_x_height: u16,
    pub x_height_snapping_exceptions: String,
    pub fallback_stem_width: Option<u16>,
    pub windows_compatibility: bool,
    pub adjust_subglyphs: bool,
    pub hint_composites: bool,
    pub symbol: bool,
    pub dehint: bool,
    pub no_info: bool,
    pub ttfa_table: bool,
    pub default_script: String,
    pub fallback_script: String,
    pub fallback_scaling: bool,
    /// Grayscale, GDI ClearType, DirectWrite ClearType: `qsq` by default.
    pub stem_width_mode: [StemWidthMode; 3],
}

impl Default for TtAutohintOptions {
    fn default() -> Self {
        Self {
            hinting_range_min: 8,
            hinting_range_max: 50,
            hinting_limit: 200,
            increase_x_height: 14,
            x_height_snapping_exceptions: String::new(),
            fallback_stem_width: None,
            windows_compatibility: false,
            adjust_subglyphs: false,
            hint_composites: false,
            symbol: false,
            dehint: false,
            no_info: false,
            ttfa_table: false,
            default_script: "latn".into(),
            fallback_script: "none".into(),
            fallback_scaling: false,
            stem_width_mode: [StemWidthMode::Quantized, StemWidthMode::Strong, StemWidthMode::Quantized],
        }
    }
}

impl TtAutohintOptions {
    /// Reads the TTFAutohint options parameter (long options, as stored); returns the options
    /// and the names of options given but ignored by our autohinter.
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), HintingError> {
        // M10.21: the option syntax.
        let _ = text;
        Ok((Self::default(), Vec::new()))
    }

    /// The options as long options, which [`TtAutohintOptions::parse`] reads back.
    pub fn to_text(&self) -> String {
        // M10.21.
        String::new()
    }
}

/// TrueType instructions for one glyph's main-master layer, replacing its manual ones.
pub fn autohint_tt(
    font: &Font,
    glyph: &str,
    settings: &HintSettings,
    options: &TtAutohintOptions,
    control: &ControlInstructions,
) -> Result<Vec<TtInstruction>, HintingError> {
    // M10.21: the PostScript analysis (fontcraft-autohint) turned into Snap and Stem.
    let _ = (settings, options, control);
    font.glyph(glyph).ok_or_else(|| HintingError::MissingGlyph(glyph.to_string()))?;
    Err(HintingError::NotYetImplemented("the TrueType autohinter"))
}
