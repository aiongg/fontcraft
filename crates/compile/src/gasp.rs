//! The `gasp` table: rendering behaviour by size.
//!
//! Source order: the gasp Table parameter (instance, then font) or a UFO's
//! `openTypeGaspRangeRecords`. A TrueType font with instructions and no such source gets the
//! handbook's thresholds 8 and 20 with DOGRAY + SYMMETRIC_SMOOTHING up to 8 PPM, GRIDFIT +
//! SYMMETRIC_GRIDFIT up to 20 and all four bits above (a guess on the flags); an unhinted font
//! without a source gets none, as fontc and fontmake do. A key above 65535 is dropped with a
//! warning, as fontc does. Version 1 table. Rows: the `gasp` table row,
//! `hinting.tt.param.gaspTable`, `hinting.tt.gasp.ranges`, `hinting.tt.gasp.flagMapping`,
//! `gasp Table`.
//!
//! The parameter is a dictionary from the largest size of each range, in PPM, to its flag bits
//! (1 grid-fit, 2 grayscale, 4 symmetric grid-fit, 8 symmetric smoothing). Reading it follows
//! fontc: a size given twice keeps its last value, bits above the four the table defines are
//! dropped, and a range left with no bit is dropped from a `.glyphs` source but kept from a UFO
//! (fontc and ufo2ft both keep it there). The UFO reader stores `openTypeGaspRangeRecords` as
//! this parameter. A CFF font gets no `gasp`: the table only steers TrueType rasterizers, and
//! ufo2ft writes it for TrueType output only.

use std::collections::BTreeMap;

use fontcraft_model::plist::Value;
use fontcraft_model::{Resolved, is_ufo_family};
use write_fonts::tables::gasp::{Gasp, GaspRange, GaspRangeBehavior};

use crate::CompileError;
use crate::info::int;
use crate::outline::Flavour;

/// Most ranges read from a source.
pub const MAX_GASP_RANGES: usize = 256;

/// The parameter's two spellings, both in use in sources; the UFO reader writes the second.
const PARAM_NAMES: [&str; 2] = ["gasp Table", "GASP Table"];

/// The four behaviour bits the version 1 table defines.
const DEFINED_BITS: i64 = 0xF;

/// The largest size a range can end at; the table's last range should end here.
const LAST_PPEM: u16 = u16::MAX;

/// The default thresholds for a hinted font without a source.
const DEFAULT_THRESHOLDS: [u16; 2] = [8, 20];

/// The `gasp` table for `font`, or `None` when it gets none. `hinted`: the font carries TrueType
/// instructions.
pub fn gasp_table(font: &Resolved, flavour: Flavour, hinted: bool, warnings: &mut Vec<String>) -> Result<Option<Gasp>, CompileError> {
    if flavour != Flavour::TrueType {
        return Ok(None);
    }
    let source = PARAM_NAMES.iter().find_map(|name| font.params.get(name).map(|value| (*name, value)));
    let mut ranges = match source {
        Some((name, value)) => source_ranges(name, value, is_ufo_family(font.format), warnings),
        None => Vec::new(),
    };
    if ranges.is_empty() && hinted {
        ranges = default_ranges();
    }
    let Some(last) = ranges.last() else {
        return Ok(None);
    };
    if last.range_max_ppem != LAST_PPEM {
        warnings.push(format!(
            "the gasp table's last range ends at {} PPM, not {LAST_PPEM}, so larger sizes get no rendering recommendation",
            last.range_max_ppem
        ));
    }
    // At most MAX_GASP_RANGES ranges, so the count fits.
    let count = u16::try_from(ranges.len()).unwrap_or(u16::MAX);
    Ok(Some(Gasp::new(1, count, ranges)))
}

/// The ranges `value` (the parameter `name`) gives, sorted by size; the entries that cannot be
/// written are reported and left out.
fn source_ranges(name: &str, value: &Value, ufo: bool, warnings: &mut Vec<String>) -> Vec<GaspRange> {
    let Some(dict) = value.as_dict() else {
        warnings.push(format!("the {name} parameter is not a dictionary of sizes to flags, so it was not used"));
        return Vec::new();
    };
    if dict.entries.len() > MAX_GASP_RANGES {
        warnings.push(format!("the {name} parameter has {} ranges; only the first {MAX_GASP_RANGES} were read", dict.entries.len()));
    }
    let mut by_size: BTreeMap<i64, i64> = BTreeMap::new();
    for (key, flags) in dict.entries.iter().take(MAX_GASP_RANGES) {
        match (key.as_str().trim().parse::<i64>(), int(flags)) {
            (Ok(size), Some(flags)) => {
                by_size.insert(size, flags);
            }
            _ => warnings.push(format!("the {name} entry `{}` is not a size with flag bits, so it was left out", key.as_str())),
        }
    }
    let mut ranges = Vec::with_capacity(by_size.len());
    for (size, flags) in by_size {
        let Ok(max_ppem) = u16::try_from(size) else {
            warnings.push(format!("the {name} range up to {size} PPM is outside 0 to {LAST_PPEM}, so it was left out"));
            continue;
        };
        if flags & !DEFINED_BITS != 0 {
            warnings.push(format!("the {name} range up to {size} PPM has flag bits above 8, which the table does not define; they were dropped"));
        }
        let bits = u16::try_from(flags & DEFINED_BITS).unwrap_or(0);
        if bits == 0 && !ufo {
            warnings.push(format!("the {name} range up to {size} PPM sets no flag, so it was left out"));
            continue;
        }
        ranges.push(GaspRange::new(max_ppem, GaspRangeBehavior::from_bits_truncate(bits)));
    }
    ranges
}

/// The ranges of a hinted font without a source: no grid-fitting with smoothing up to the first
/// threshold, grid-fitting without grayscale up to the second, everything above.
fn default_ranges() -> Vec<GaspRange> {
    let [small, medium] = DEFAULT_THRESHOLDS;
    vec![
        GaspRange::new(small, GaspRangeBehavior::GASP_DOGRAY | GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING),
        GaspRange::new(medium, GaspRangeBehavior::GASP_GRIDFIT | GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT),
        GaspRange::new(
            LAST_PPEM,
            GaspRangeBehavior::GASP_GRIDFIT
                | GaspRangeBehavior::GASP_DOGRAY
                | GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT
                | GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Dict, Number};
    use fontcraft_model::{Font, Instance, Param, SourceFormat, Target};
    use skrifa::FontRef;
    use skrifa::raw::TableProvider;

    use super::*;
    use crate::{Options, compile};

    fn table(entries: &[(&str, i64)]) -> Value {
        let mut d = Dict::new();
        for (k, v) in entries {
            d.entries.push(((*k).into(), Value::Number(Number::int(*v))));
        }
        Value::Dict(d)
    }

    fn font_with(format: SourceFormat, name: &str, value: Value) -> Font {
        let mut f = Font::new("Gasp Sans");
        f.source.format = format;
        Arc::make_mut(&mut f.info).params.0.push(Param::new(name, value));
        f
    }

    /// The `gasp` records of `f` compiled at `target` as TrueType, and the warnings.
    fn compiled(f: &Font, target: &Target) -> (Option<(u16, Vec<(u16, u16)>)>, Vec<String>) {
        let out = compile(f, target, Flavour::TrueType, &Options::default()).unwrap();
        let font = FontRef::new(&out.bytes).unwrap();
        let gasp = font.gasp().ok().map(|g| {
            let ranges = g.gasp_ranges().iter().map(|r| (r.range_max_ppem(), r.range_gasp_behavior().bits())).collect();
            (g.version(), ranges)
        });
        (gasp, out.warnings)
    }

    fn records(f: &Font, flavour: Flavour, hinted: bool) -> Option<Vec<(u16, u16)>> {
        let resolved = f.resolve(&f.default_target().unwrap()).unwrap();
        let gasp = gasp_table(&resolved, flavour, hinted, &mut Vec::new()).unwrap()?;
        assert_eq!(usize::from(gasp.num_ranges), gasp.gasp_ranges.len());
        Some(gasp.gasp_ranges.iter().map(|r| (r.range_max_ppem, r.range_gasp_behavior.bits())).collect())
    }

    #[test]
    fn the_gasp_table_parameter_is_written_sorted_with_a_size_above_65535_dropped() {
        // fontc's WghtVarGasp sources, in either spelling of the parameter.
        let source = [("20", 7), ("65535", 15), ("8", 10), ("65536", 1)];
        for name in PARAM_NAMES {
            let f = font_with(SourceFormat::Glyphs, name, table(&source));
            let (gasp, warnings) = compiled(&f, &f.default_target().unwrap());
            assert_eq!(gasp, Some((1, vec![(8, 10), (20, 7), (65535, 15)])), "{name}");
            assert!(warnings.iter().any(|w| w.contains("65536")), "{name}: {warnings:?}");
        }
    }

    #[test]
    fn a_range_without_flags_is_dropped_from_a_glyphs_source_and_kept_from_a_ufo() {
        // A UFO's records reach the model as the parameter (`io-ufo`), sizes as text.
        let source = [("65535", 15), ("7", 0x1A), ("7", 10), ("12", 0)];
        let f = font_with(SourceFormat::Ufo, "GASP Table", table(&source));
        let (gasp, warnings) = compiled(&f, &f.default_target().unwrap());
        // The last value of a repeated size wins; the bit above 8 in the first is dropped.
        assert_eq!(gasp, Some((1, vec![(7, 10), (12, 0), (65535, 15)])));
        assert!(!warnings.iter().any(|w| w.contains("sets no flag")), "{warnings:?}");

        let f = font_with(SourceFormat::Glyphs, "gasp Table", table(&source));
        let (gasp, warnings) = compiled(&f, &f.default_target().unwrap());
        assert_eq!(gasp, Some((1, vec![(7, 10), (65535, 15)])));
        assert!(warnings.iter().any(|w| w.contains("up to 12 PPM sets no flag")), "{warnings:?}");
    }

    #[test]
    fn an_instance_s_gasp_table_replaces_the_font_s() {
        let mut f = font_with(SourceFormat::Glyphs, "gasp Table", table(&[("65535", 15)]));
        let mut instance = Instance { name: "Regular".into(), ..Default::default() };
        instance.params.0.push(Param::new("gasp Table", table(&[("9", 2), ("65535", 3)])));
        Arc::make_mut(&mut f.instances).push(instance);
        let (gasp, _) = compiled(&f, &Target::Instance(0));
        assert_eq!(gasp, Some((1, vec![(9, 2), (65535, 3)])));
        let (gasp, _) = compiled(&f, &f.default_target().unwrap());
        assert_eq!(gasp, Some((1, vec![(65535, 15)])));
    }

    #[test]
    fn a_hinted_font_without_a_source_gets_the_default_and_an_unhinted_one_none() {
        let f = Font::new("Gasp Sans");
        assert_eq!(records(&f, Flavour::TrueType, true), Some(vec![(8, 10), (20, 5), (65535, 15)]));
        assert_eq!(records(&f, Flavour::TrueType, false), None);
        let (gasp, _) = compiled(&f, &f.default_target().unwrap());
        assert_eq!(gasp, None, "a font without instructions");
        // A parameter with nothing usable counts as no source.
        let f = font_with(SourceFormat::Glyphs, "gasp Table", Value::String("8".into()));
        assert_eq!(records(&f, Flavour::TrueType, true), Some(vec![(8, 10), (20, 5), (65535, 15)]));
        // CFF fonts get no gasp, with or without a source.
        let f = font_with(SourceFormat::Glyphs, "gasp Table", table(&[("65535", 15)]));
        assert_eq!(records(&f, Flavour::Cff, true), None);
        assert_eq!(records(&f, Flavour::TrueType, false), Some(vec![(65535, 15)]));
    }
}
