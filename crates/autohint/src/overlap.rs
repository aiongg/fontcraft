//! Hinting with overlaps removed: segments come from the overlap-free outline, matched back to
//! the elements of the original.
//!
//! Ported from otfautohint's `overlap.py` (AFDKO 5.0.1, Apache-2.0, Copyright 2021 Adobe; see
//! `NOTICE`). The original removes overlaps itself with booleanOperations; this crate sits below
//! the path operations, so the caller passes the overlap-free contours
//! ([`crate::HintInput::without_overlap`]) and puts the result of [`associate`] in
//! [`crate::state::GlyphHintState::overlap_removed`] before segments are generated (see
//! [`crate::segments`]).

use fontcraft_geom::BezPath;

use crate::AutohintError;
use crate::glyph::GlyphData;

/// The overlap-free glyph, its elements associated with those of `glyph` (`removeOverlap`):
/// `None` when `without_overlap` draws the same outline as `glyph`. Its coordinates are not
/// rounded, as in the original.
pub fn associate(glyph: &GlyphData, without_overlap: &[BezPath], loose: bool) -> Result<Option<GlyphData>, AutohintError> {
    let mut free = GlyphData::from_contours(&glyph.name, without_overlap, false)?;
    if same_outline(glyph, &free) {
        return Ok(None);
    }
    free.associate_path(glyph, loose);
    Ok(Some(free))
}

/// The two glyphs have the same elements with the same on-curve points, whatever the start
/// point and order of their contours. Coordinates are compared rounded to whole units, since
/// `glyph` may have been rounded and the overlap-free outline is not.
///
/// The original decides by whether removing overlaps dropped any point of the flattened
/// outline; the caller passes no overlap-free outline when nothing overlaps, so this only
/// catches an outline returned unchanged.
fn same_outline(glyph: &GlyphData, free: &GlyphData) -> bool {
    let ends = |g: &GlyphData| {
        let mut v: Vec<(f64, f64, bool)> =
            g.subpaths.iter().flatten().map(|pe| (pe.e.x.round_ties_even(), pe.e.y.round_ties_even(), pe.is_line)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        v
    };
    ends(glyph) == ends(free)
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::BezPath;

    use super::*;
    use crate::glyph::{Dim, PeRef};
    use crate::segments::tests::{assert_segs, contour, hinter, segments};
    use crate::state::OverlapState;
    use crate::{Options, hinter::GlyphHinter};

    fn tee() -> Vec<BezPath> {
        vec![
            contour((0.0, 400.0), &[&[(500.0, 400.0)], &[(500.0, 500.0)], &[(0.0, 500.0)]]),
            contour((200.0, 0.0), &[&[(300.0, 0.0)], &[(300.0, 450.0)], &[(200.0, 450.0)]]),
        ]
    }

    fn tee_union() -> Vec<BezPath> {
        vec![contour(
            (200.0, 0.0),
            &[&[(300.0, 0.0)], &[(300.0, 400.0)], &[(500.0, 400.0)], &[(500.0, 500.0)], &[(0.0, 500.0)], &[(0.0, 400.0)], &[(200.0, 400.0)]],
        )]
    }

    fn overlapping(gh: &mut GlyphHinter) {
        let free = associate(&gh.glyphs.main, &tee_union(), false).unwrap();
        let state = free.map_or(OverlapState::NoOverlap, |g| OverlapState::Removed(Box::new(g)));
        gh.h.hs.overlap_removed = state.clone();
        gh.v.hs.overlap_removed = state;
    }

    // Expected values: tests/oracle/segments.py (otfautohint 5.0.1).
    #[test]
    fn overlap_free_elements_map_back_to_the_original() {
        let gh = hinter("tee", &tee(), Options::default());
        let free = associate(&gh.glyphs.main, &tee_union(), false).unwrap().unwrap();
        let got: Vec<Option<PeRef>> = free.elements().map(|r| free.element(r).and_then(|e| e.association).flatten()).collect();
        let want = [(1, 0), (1, 1), (0, 0), (0, 1), (0, 2), (0, 3), (0, 0), (1, 3)].map(|(s, o)| Some(PeRef::new(s, o)));
        assert_eq!(got, want);
    }

    #[test]
    fn segments_of_an_overlapping_glyph_come_from_its_union() {
        let mut gh = hinter("tee", &tee(), Options { overlap_forcing: Some(true), ..Options::default() });
        overlapping(&mut gh);
        let (inc, dec) = segments(&mut gh, Dim::H);
        assert_segs(&inc, &[(0.0, 199.0, 301.0, 1), (400.0, 299.0, 501.0, 1), (400.0, -1.0, 201.0, 1)]);
        assert_segs(&dec, &[(500.0, -1.0, 501.0, 1)]);
        let (inc, dec) = segments(&mut gh, Dim::V);
        assert_segs(&inc, &[(300.0, -1.0, 401.0, 1), (500.0, 399.0, 501.0, 1)]);
        assert_segs(&dec, &[(0.0, 399.0, 501.0, 1), (200.0, -1.0, 401.0, 1)]);
        // Every segment names an element of the original glyph.
        let hs = &gh.h.hs;
        for seg in hs.increasing_segs.iter().chain(&hs.decreasing_segs).filter_map(|id| hs.seg(*id)) {
            let pe = seg.pe.unwrap();
            assert!(gh.glyphs.main.element(pe).is_some(), "{pe:?}");
        }
    }

    #[test]
    fn an_unchanged_outline_needs_no_association() {
        let gh = hinter("stem", &[contour((100.0, 0.0), &[&[(180.0, 0.0)], &[(180.0, 700.0)], &[(100.0, 700.0)]])], Options::default());
        let same = [contour((180.0, 700.0), &[&[(100.0, 700.0)], &[(100.0, 0.0)], &[(180.0, 0.0)]])];
        assert_eq!(associate(&gh.glyphs.main, &same, false).unwrap(), None);
    }
}
