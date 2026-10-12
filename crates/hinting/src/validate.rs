//! The font-hint checks: the zone and stem rules of the handbook and the grid compensation check.
//!
//! Rows: `hinting.ps.alignmentZones.limits`, `hinting.ps.alignmentZones.gridCompensation`,
//! `hinting.ps.standardStems`, `hinting.ps.blueFuzz`, `hint.ps.validateFontHints` (the engine
//! command lists these findings).
//!
//! | Rule | Severity | Breaks when |
//! |---|---|---|
//! | `zones.overlap` | error | two zones share a value |
//! | `zones.tooClose` | warning | the gap between two zones is under 2 × BlueFuzz + 1 units (at least 1) |
//! | `zones.tooManyTop` | warning | more than 6 top zones besides the baseline zone |
//! | `zones.tooManyBottom` | warning | more than 5 bottom zones besides the baseline zone |
//! | `zones.noBaseline` | warning | there are zones but none at position 0 |
//! | `zones.tooTall` | warning | a zone is larger than 25 units |
//! | `zones.blueScale` | warning | a zone is larger than 240 ÷ (240 × BlueScale − 0.98) |
//! | `zones.gridCompensation` | warning | the font uses a coarser grid than 1 unit and a zone's size is a whole number of grid steps, so it was not widened by one unit at each end |
//! | `blueScale.notPositive` | error | BlueScale is 0 or less |
//! | `stems.notPositive` | error | a standard stem is 0 or less |
//! | `stems.tooMany` | warning | more than 12 stems in one direction |
//! | `ttZones.duplicateName`, `ttStems.duplicateName` | warning | two TT zones or two TT stems share a name |
//!
//! The severities are our choice (guess on `hinting.ps.alignmentZones.limits`), as is the grid
//! check (guess on `hinting.ps.alignmentZones.gridCompensation`). The export writes what it can
//! either way: the first entries within the CFF limits, with a warning (guesses on
//! `hinting.ps.alignmentZones.blueValuesMapping` and `hinting.ps.standardStems.privateDictMapping`).

use fontcraft_model::Font;

use crate::settings::PsZone;
use crate::{HintSettings, HintingError, MAX_PS_STEMS_WRITTEN};

/// Top zones a master may have besides the baseline zone.
pub const MAX_TOP_ZONES: usize = 6;
/// Bottom zones a master may have besides the baseline zone.
pub const MAX_BOTTOM_ZONES: usize = 5;
/// Largest zone size the handbook allows, in units.
pub const MAX_ZONE_SIZE: f64 = 25.0;

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
    let s = HintSettings::for_master(font, master)?;
    let mut out = Vec::new();
    zone_rules(&s, &mut out);
    grid_rule(font, &s.zones, &mut out);
    stem_rules(&s, &mut out);
    name_rules(&s, &mut out);
    Ok(out)
}

fn finding(out: &mut Vec<HintFinding>, severity: Severity, rule: &'static str, message: String) {
    out.push(HintFinding { severity, rule, message });
}

/// The zone at `position` with `size`, as a person reads it.
fn zone_text(z: &PsZone) -> String {
    format!("the zone at {} (size {})", z.position, z.size)
}

/// The baseline zone is the zone at 0; the others are top or bottom zones by the sign of their
/// size, as the Private dict mapping sorts them.
fn zone_rules(s: &HintSettings, out: &mut Vec<HintFinding>) {
    let zones = &s.zones;
    if zones.is_empty() {
        return;
    }
    let top = zones.iter().filter(|z| z.position != 0.0 && z.size >= 0.0).count();
    let bottom = zones.iter().filter(|z| z.position != 0.0 && z.size < 0.0).count();
    if top > MAX_TOP_ZONES {
        finding(
            out,
            Severity::Warning,
            "zones.tooManyTop",
            format!("{top} top zones; at most {MAX_TOP_ZONES} are written besides the baseline zone"),
        );
    }
    if bottom > MAX_BOTTOM_ZONES {
        finding(
            out,
            Severity::Warning,
            "zones.tooManyBottom",
            format!("{bottom} bottom zones; at most {MAX_BOTTOM_ZONES} are written besides the baseline zone"),
        );
    }
    if !zones.iter().any(|z| z.position == 0.0) {
        finding(out, Severity::Warning, "zones.noBaseline", "no zone sits at position 0 (the baseline zone)".to_string());
    }

    let scale = s.effective_blue_scale();
    if scale <= 0.0 {
        finding(out, Severity::Error, "blueScale.notPositive", format!("blueScale is {scale}; it must be more than 0"));
    }
    let denominator = 240.0 * scale - 0.98;
    let scale_limit = (denominator > 0.0).then(|| 240.0 / denominator);
    for z in zones {
        let size = z.size.abs();
        if size > MAX_ZONE_SIZE {
            finding(out, Severity::Warning, "zones.tooTall", format!("{} is larger than {MAX_ZONE_SIZE} units", zone_text(z)));
        }
        if let Some(limit) = scale_limit.filter(|l| size > *l) {
            finding(
                out,
                Severity::Warning,
                "zones.blueScale",
                format!("{} is larger than {limit:.2} units, the most blueScale {scale} allows", zone_text(z)),
            );
        }
    }

    // Zones as (low, high) intervals in ascending order.
    let mut spans: Vec<(f64, f64, &PsZone)> = zones
        .iter()
        .map(|z| {
            let (a, b) = (z.position, z.position + z.size);
            if a <= b { (a, b, z) } else { (b, a, z) }
        })
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let min_gap = (2.0 * s.effective_blue_fuzz() + 1.0).max(1.0);
    for pair in spans.windows(2) {
        let [(_, high, z1), (low, _, z2)] = pair else { continue };
        let gap = low - high;
        if gap <= 0.0 {
            finding(out, Severity::Error, "zones.overlap", format!("{} and {} overlap", zone_text(z1), zone_text(z2)));
        } else if gap < min_gap {
            finding(
                out,
                Severity::Warning,
                "zones.tooClose",
                format!("{} and {} are {gap} units apart; keep at least {min_gap} (2 × blueFuzz + 1)", zone_text(z1), zone_text(z2)),
            );
        }
    }
}

/// With a grid coarser than one unit, every zone should reach one unit further at each end (the
/// baseline zone one unit further down only), so its size is no longer a whole number of grid
/// steps. A finer or fractional grid cannot be checked this way and is not reported.
fn grid_rule(font: &Font, zones: &[PsZone], out: &mut Vec<HintFinding>) {
    let length = font.info.settings.grid_length.unwrap_or(1.0);
    let sub = font.info.settings.grid_sub_division.filter(|s| s.is_finite() && *s >= 1.0).unwrap_or(1.0);
    let step = length / sub;
    if !step.is_finite() || step <= 1.0 || step.fract() != 0.0 {
        return;
    }
    for z in zones {
        if z.size != 0.0 && (z.size.abs() % step) == 0.0 {
            finding(
                out,
                Severity::Warning,
                "zones.gridCompensation",
                format!("{} matches the {step}-unit grid exactly; widen it by one unit at each end to catch rounded nodes", zone_text(z)),
            );
        }
    }
}

fn stem_rules(s: &HintSettings, out: &mut Vec<HintFinding>) {
    for (dir, stems) in [("horizontal", &s.stems_h), ("vertical", &s.stems_v)] {
        if stems.len() > MAX_PS_STEMS_WRITTEN {
            finding(out, Severity::Warning, "stems.tooMany", format!("{} {dir} stems; the first {MAX_PS_STEMS_WRITTEN} are written", stems.len()));
        }
        for v in stems.iter().filter(|v| **v <= 0.0) {
            finding(out, Severity::Error, "stems.notPositive", format!("a {dir} stem is {v}; stems must be more than 0"));
        }
    }
}

fn name_rules(s: &HintSettings, out: &mut Vec<HintFinding>) {
    let duplicates = |names: Vec<&str>| -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut dup = std::collections::BTreeSet::new();
        for n in names {
            if !seen.insert(n) {
                dup.insert(n.to_string());
            }
        }
        dup.into_iter().collect()
    };
    for name in duplicates(s.tt_zones.iter().map(|z| z.name.as_str()).collect()) {
        finding(out, Severity::Warning, "ttZones.duplicateName", format!("two TT zones are called `{name}`"));
    }
    for name in duplicates(s.tt_stems.iter().map(|z| z.name.as_str()).collect()) {
        finding(out, Severity::Warning, "ttStems.duplicateName", format!("two TT stems are called `{name}`"));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{AlignmentZone, Font, Master, Param, StemDef};

    use super::*;
    use crate::settings::{BLUE_FUZZ, BLUE_SCALE, TTF_STEMS};

    fn rules(font: &Font) -> Vec<&'static str> {
        validate_font_hints(font, "m01").unwrap().into_iter().map(|f| f.rule).collect()
    }

    fn master(font: &mut Font) -> &mut Master {
        &mut Arc::make_mut(&mut font.masters)[0]
    }

    fn with_zones(zones: &[(f64, f64)]) -> Font {
        let mut font = Font::new("T");
        let m = master(&mut font);
        m.metric_values.iter_mut().for_each(|v| v.over = 0.0);
        m.zones = zones.iter().map(|&(pos, size)| AlignmentZone { pos, size }).collect();
        font
    }

    #[test]
    fn the_new_font_passes() {
        assert!(rules(&Font::new("T")).is_empty());
    }

    #[test]
    fn each_zone_rule_reports_its_case() {
        assert_eq!(rules(&with_zones(&[(0.0, -16.0), (500.0, 16.0), (510.0, 16.0)])), ["zones.overlap"]);
        assert_eq!(rules(&with_zones(&[(0.0, -16.0), (500.0, 16.0), (518.0, 16.0)])), ["zones.tooClose"], "2 units apart, 3 needed");
        let mut font = with_zones(&[(0.0, -16.0), (500.0, 16.0), (518.0, 16.0)]);
        Arc::make_mut(&mut font.info).params.set(BLUE_FUZZ, fontcraft_model::plist::Value::Number(fontcraft_model::plist::Number::int(0)));
        assert!(rules(&font).is_empty(), "blueFuzz 0 needs one unit");
        let tops: Vec<(f64, f64)> = std::iter::once((0.0, -10.0)).chain((1..=7).map(|i| (f64::from(i) * 100.0, 10.0))).collect();
        assert_eq!(rules(&with_zones(&tops)), ["zones.tooManyTop"]);
        let bottoms: Vec<(f64, f64)> = std::iter::once((0.0, -10.0)).chain((1..=6).map(|i| (f64::from(i) * -100.0, -10.0))).collect();
        assert_eq!(rules(&with_zones(&bottoms)), ["zones.tooManyBottom"]);
        assert_eq!(rules(&with_zones(&[(500.0, 16.0)])), ["zones.noBaseline"]);
        assert_eq!(rules(&with_zones(&[(0.0, -16.0), (500.0, 26.0)])), ["zones.tooTall"]);
        let mut font = with_zones(&[(0.0, -16.0), (500.0, 20.0)]);
        Arc::make_mut(&mut font.info).params.set(BLUE_SCALE, fontcraft_model::plist::openstep::parse_value("0.06").unwrap());
        assert_eq!(rules(&font), ["zones.blueScale"], "240 / (240 × 0.06 − 0.98) is about 17.9");
        Arc::make_mut(&mut font.info).params.set(BLUE_SCALE, fontcraft_model::plist::openstep::parse_value("0").unwrap());
        assert_eq!(rules(&font), ["blueScale.notPositive"]);
    }

    #[test]
    fn a_coarse_grid_asks_for_widened_zones() {
        let mut font = with_zones(&[(0.0, -10.0), (500.0, 11.0)]);
        Arc::make_mut(&mut font.info).settings.grid_length = Some(10.0);
        assert_eq!(rules(&font), ["zones.gridCompensation"], "only the zone of a whole number of grid steps");
        Arc::make_mut(&mut font.info).settings.grid_sub_division = Some(10.0);
        assert!(rules(&font).is_empty(), "a one-unit grid is the default");
    }

    #[test]
    fn stem_rules_report_their_cases() {
        let mut font = Font::new("T");
        Arc::make_mut(&mut font.stems).extend((0..14).map(|i| StemDef { name: format!("s{i}"), horizontal: false, ..StemDef::default() }));
        master(&mut font).stem_values = (0..14).map(|i| if i == 3 { 0.0 } else { 80.0 }).collect();
        assert_eq!(rules(&font), ["stems.tooMany", "stems.notPositive"]);
        let mut font = Font::new("T");
        master(&mut font)
            .params
            .0
            .push(Param::new(TTF_STEMS, fontcraft_model::plist::openstep::parse_value("({name = a; width = 1;}, {name = a; width = 2;})").unwrap()));
        assert_eq!(rules(&font), ["ttStems.duplicateName"]);
    }
}
