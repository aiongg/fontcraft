//! PostScript hints of an opened CFF font: its stems become numeric layer hints and its Private
//! dict becomes zones, stems and the blue parameters (`docs/plans/M10.md` § 3 › Imported
//! hinting).
//!
//! Rows: `open.postscriptHintsPreserved`.
//!
//! The `CFF ` table is a CFF font program, so it is read as a bare CFF font is
//! ([`crate::bare_cff::read_cff`]) and its hints are taken over as a Type 1 font's are
//! ([`crate::type1`]):
//!
//! | In the font | In the document |
//! |---|---|
//! | the stems of a glyph's charstring (`hstem`, `vstem`, their `hm` forms, the stems a `hintmask` implies) | the glyph's distinct stems as numeric hints on its layer, in first-seen order; a width of −20 or −21 as a top or bottom ghost hint. Hint replacement and counter hints are not kept: export builds the masks again |
//! | BlueValues | the first zone that holds the baseline as the baseline's overshoot; a zone starting at the ascender, cap height or x-height as that metric's overshoot; any other as an alignment zone of the master |
//! | OtherBlues | a zone ending at the descender as its overshoot; any other as an alignment zone |
//! | StemSnapH, StemSnapV (else StdHW, StdVW) | the font's stems, `hStem0`… and `vStem0`…, with the master's widths |
//! | BlueScale, BlueShift, BlueFuzz | the blueScale, blueShift and blueFuzz parameters of the master, when the dict sets them |
//!
//! The hints are numbers, not linked to nodes, as the handbook says of imported hints. A font
//! whose `CFF ` table cannot be read this way keeps its outlines without hints, with a warning.

use std::sync::Arc;

use fontcraft_model::plist::{Number, Value};
use fontcraft_model::{AlignmentZone, Font, MAX_COORDINATE, StemDef, metric};
use skrifa::raw::FontRef;
use skrifa::raw::types::Tag;

use crate::DecompileError;
use crate::type1::{PrivateDict, layer_hints};

/// Most stem hints read from one charstring (the Type 2 limit).
pub const MAX_IMPORTED_STEMS: usize = 96;

/// Adds the CFF hints of `sfnt` to `font` (one master, glyphs in the font's order). A font
/// without a `CFF ` table, or without hints, is left alone. What cannot be read goes to
/// `warnings`; only a model error fails.
pub fn import(sfnt: &FontRef, font: &mut Font, warnings: &mut Vec<String>) -> Result<(), DecompileError> {
    let Some(data) = sfnt.table_data(Tag::new(b"CFF ")) else { return Ok(()) };
    // The outlines were read already; what this second reading finds wrong with them is not
    // news to the user.
    let mut ignored = Vec::new();
    let ps = match crate::bare_cff::read_cff(data.as_bytes(), &mut ignored) {
        Ok(ps) => ps,
        Err(e) => {
            warnings.push(format!("the PostScript hints were not kept: {e}"));
            return Ok(());
        }
    };

    if ps.glyphs.len() == font.glyphs.as_arcs().len() {
        let mut cut = 0usize;
        for (gid, g) in ps.glyphs.iter().enumerate() {
            let mut hints = layer_hints(&g.ops);
            if hints.is_empty() {
                continue;
            }
            if hints.len() > MAX_IMPORTED_STEMS {
                hints.truncate(MAX_IMPORTED_STEMS);
                cut += 1;
            }
            let Some(mut glyph) = font.glyphs.get_mut_at(gid) else { continue };
            if let Some(layer) = glyph.layers.first_mut() {
                layer.info.hints = hints;
            }
        }
        if cut > 0 {
            warnings.push(format!("{cut} glyphs have more than {MAX_IMPORTED_STEMS} stem hints; the first {MAX_IMPORTED_STEMS} of each were kept"));
        }
    } else {
        warnings.push("the CFF table and the font disagree on the number of glyphs, so the glyphs' PostScript hints were not kept".into());
    }

    private_dict(&ps.private, font, warnings);
    font.check()?;
    Ok(())
}

fn usable(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::Number(Number::int(v as i64))
    } else {
        Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
    }
}

/// The Private dict as zones, stems and blue parameters of the font's first master.
fn private_dict(p: &PrivateDict, font: &mut Font, warnings: &mut Vec<String>) {
    let slot = |kind: &str| font.metrics.iter().position(|d| d.kind == kind && d.name.is_none() && d.filter.is_none());
    let [baseline, descender] = [slot(metric::BASELINE), slot(metric::DESCENDER)];
    let tops: Vec<usize> = [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT].into_iter().filter_map(slot).collect();

    let mut stems = Vec::new();
    let mut widths = Vec::new();
    for (horizontal, snaps, std) in [(true, &p.stem_snap_h, p.std_hw), (false, &p.stem_snap_v, p.std_vw)] {
        let list: Vec<f64> = if snaps.is_empty() { std.into_iter().collect() } else { snaps.clone() };
        for (i, w) in list.into_iter().filter(|w| usable(*w)).enumerate() {
            stems.push(StemDef { name: format!("{}Stem{i}", if horizontal { 'h' } else { 'v' }), horizontal, ..Default::default() });
            widths.push(w);
        }
    }

    let masters = Arc::make_mut(&mut font.masters);
    let Some(master) = masters.first_mut() else { return };
    let pairs = |v: &[f64]| -> Vec<(f64, f64)> {
        v.chunks_exact(2).filter_map(|c| Some((*c.first()?, *c.get(1)?))).filter(|(a, b)| usable(*a) && usable(*b)).collect()
    };
    let mut zones = Vec::new();
    let mut baseline_taken = false;
    for (lo, hi) in pairs(&p.blue_values) {
        if !baseline_taken && lo <= 0.0 && hi >= 0.0 {
            baseline_taken = true;
            if let Some(v) = baseline.and_then(|i| master.metric_values.get_mut(i)).filter(|v| v.over == 0.0) {
                v.over = lo;
                continue;
            }
        }
        let at = tops.iter().copied().find(|i| master.metric_values.get(*i).is_some_and(|v| v.pos == lo && v.over == 0.0));
        match at.and_then(|i| master.metric_values.get_mut(i)) {
            Some(v) => v.over = hi - lo,
            None => zones.push(AlignmentZone { pos: lo, size: hi - lo }),
        }
    }
    for (lo, hi) in pairs(&p.other_blues) {
        match descender.and_then(|i| master.metric_values.get_mut(i)).filter(|v| v.pos == hi && v.over == 0.0) {
            Some(v) => v.over = lo - hi,
            None => zones.push(AlignmentZone { pos: hi, size: lo - hi }),
        }
    }
    master.zones.extend(zones);
    if !stems.is_empty() && font.stems.is_empty() {
        master.stem_values = widths;
        font.stems = Arc::new(stems);
    }
    for (key, v) in [("blueScale", p.blue_scale), ("blueShift", p.blue_shift), ("blueFuzz", p.blue_fuzz)] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            master.params.set(key, number(v));
        }
    }
    if !p.family_blues.is_empty() || !p.family_other_blues.is_empty() {
        warnings.push("FamilyBlues and FamilyOtherBlues were not read".into());
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_compile::{Flavour, Options, compile};
    use fontcraft_hinting::{Edge, PsHint, PsHintKind, ps_hints, set_ps_hints};
    use fontcraft_model::{Path, Shape, Target};

    use super::*;

    /// `l`, a stem from x 100 to 180 and 0 to 700 high, hinted with a vertical stem, a
    /// horizontal stem and a top ghost; the font has the default zones (16 units at every
    /// metric) and a horizontal stem of 40 and a vertical one of 80.
    fn hinted() -> Font {
        let mut f = Font::new("Hint Back");
        {
            let mut l = f.glyph_mut("l").unwrap();
            l.layers[0].shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 180.0, 700.0))];
            let hints = [
                PsHint::stem(false, Edge::Value(100.0), Edge::Value(180.0)),
                PsHint::stem(true, Edge::Value(0.0), Edge::Value(40.0)),
                PsHint::ghost(true, Edge::Value(700.0)),
            ];
            set_ps_hints(&mut l.layers[0], &hints).unwrap();
        }
        f.stems = Arc::new(vec![
            StemDef { name: "bar".into(), horizontal: true, ..Default::default() },
            StemDef { name: "stem".into(), horizontal: false, ..Default::default() },
        ]);
        Arc::make_mut(&mut f.masters)[0].stem_values = vec![40.0, 80.0];
        f
    }

    #[test]
    fn an_opened_cff_font_keeps_its_stem_hints_zones_and_stems() {
        let source = hinted();
        let out = compile(&source, &Target::Master("m01".into()), Flavour::Cff, &Options::default()).unwrap();
        let d = crate::decompile(&out.bytes, None).unwrap();
        let f = d.font;

        // Stem hints, as numbers.
        let layer = f.glyph("l").unwrap().layers[0].clone();
        let got: Vec<(bool, PsHintKind, (f64, f64))> =
            ps_hints(&layer).unwrap().into_iter().map(|(_, h)| (h.horizontal, h.kind, h.edges(&layer).unwrap())).collect();
        let want = [(false, PsHintKind::Stem, (100.0, 180.0)), (true, PsHintKind::Stem, (0.0, 40.0)), (true, PsHintKind::GhostTop, (700.0, 700.0))];
        assert_eq!(got.len(), want.len(), "{got:?}");
        assert!(want.iter().all(|w| got.contains(w)), "{got:?}");
        assert!(ps_hints(&layer).unwrap().iter().all(|(_, h)| !matches!(h.origin, Edge::Node(_))), "imported hints are not linked to nodes");
        assert!(f.glyph("o").unwrap().layers[0].info.hints.is_empty(), "an unhinted glyph stays unhinted");

        // Zones: each metric gets its overshoot back.
        let m = f.default_master().unwrap();
        for kind in [metric::ASCENDER, metric::CAP_HEIGHT, metric::X_HEIGHT, metric::BASELINE, metric::DESCENDER] {
            let want = if matches!(kind, metric::BASELINE | metric::DESCENDER) { -16.0 } else { 16.0 };
            assert_eq!(f.metric(m, kind).map(|v| v.over), Some(want), "{kind}");
        }
        assert!(m.zones.is_empty(), "{:?}", m.zones);

        // Stems.
        let stems: Vec<(bool, f64)> = f.stems.iter().zip(&m.stem_values).map(|(s, w)| (s.horizontal, *w)).collect();
        assert_eq!(stems, vec![(true, 40.0), (false, 80.0)]);
        assert!(!d.warnings.iter().any(|w| w.contains("hints were not kept")), "{:?}", d.warnings);
    }

    #[test]
    fn a_truetype_font_has_no_cff_hints_to_read() {
        let out = fontcraft_compile::compile_ttf(&hinted(), &Options::default()).unwrap();
        let d = crate::decompile(&out.bytes, None).unwrap();
        assert!(d.font.glyph("l").unwrap().layers[0].info.hints.is_empty());
        assert!(d.font.stems.is_empty());
    }
}
