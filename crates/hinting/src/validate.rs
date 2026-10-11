//! The font-hint checks: the zone and stem rules of the handbook and the grid compensation check.
//!
//! Rows: `hinting.ps.alignmentZones.limits`, `hinting.ps.standardStems`, `hint.ps.validateFontHints`
//! (the engine command lists these findings). M10.2 fills the body.

use fontcraft_model::Font;

use crate::HintingError;

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// One broken rule.
#[derive(Clone, Debug, PartialEq)]
pub struct HintFinding {
    pub severity: Severity,
    /// A stable id for the rule (`zones.overlap`, `zones.tooMany`, `stems.tooMany`…).
    pub rule: &'static str,
    pub message: String,
}

/// The findings for the hint settings of `master`: overlapping zones, too many zones or stems
/// for the CFF limits, zones closer than `2 × BlueFuzz + 1`, a zone taller than BlueScale
/// allows, stems that are not positive… An empty list means the settings pass.
pub fn validate_font_hints(font: &Font, master: &str) -> Result<Vec<HintFinding>, HintingError> {
    let _settings = crate::HintSettings::for_master(font, master)?;
    // M10.2: the rules.
    Ok(Vec::new())
}
