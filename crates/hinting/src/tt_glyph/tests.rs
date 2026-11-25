//! Glyph programs run through skrifa's TrueType interpreter on fonts built in the test. Points
//! sit at the same value in x and y, so one table of expectations serves both directions;
//! horizontal runs use the monochrome target, since the smooth targets ignore moves along x in
//! backward-compatibility mode.

use super::*;
use crate::tt_testfont::{Target, font, gdi, grayscale, hinted, mono};
use crate::{Deltas, FontPrograms, NodeRef, TtStem, TtZone, VecPointMap, font_programs, settings::DEFAULT_MINIMUM_DISTANCE};

const COLUMN: &str = "m01";
const BOTH: [Direction; 2] = [Direction::Vertical, Direction::Horizontal];
/// Half a 26.6 unit either side of a scaled value, plus its own rounding.
const EPS: f32 = 1.5 / 64.0;

fn settings(zones: Vec<TtZone>, stems: Vec<TtStem>) -> HintSettings {
    HintSettings {
        master: COLUMN.to_string(),
        tt_zones: zones,
        tt_stems: stems,
        tt_blue_fuzz: 1.0,
        tt_zone_rounding_threshold: 0.0,
        tt_minimum_distance: DEFAULT_MINIMUM_DISTANCE,
        ..HintSettings::default()
    }
}

fn zone(name: &str, position: f64, size: f64) -> TtZone {
    TtZone { name: name.to_string(), position, size, ..TtZone::default() }
}

/// A TT stem for instructions in direction `d`.
fn stem(size: f64, d: Direction) -> TtStem {
    TtStem { name: format!("stem {size}"), horizontal: d == Direction::Vertical, size, ..TtStem::default() }
}

fn p(k: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(0, k))
}

fn with(kind: TtKind, d: Direction, rounding: Rounding, target: usize, refs: &[usize]) -> TtInstruction {
    let mut t = TtInstruction::new(kind, d, p(target));
    t.rounding = rounding;
    t.origin = refs.first().map(|&k| p(k));
    t.other = refs.get(1).map(|&k| p(k));
    t
}

fn snap(k: usize, d: Direction, rounding: Rounding) -> TtInstruction {
    with(TtKind::Snap { zone: ZoneRef::Auto }, d, rounding, k, &[])
}

fn stem_from(o: usize, t: usize, d: Direction, rounding: Rounding) -> TtInstruction {
    with(TtKind::Stem { stem: StemRef::Auto, triple: None }, d, rounding, t, &[o])
}

fn diagonal_points(points: &[(i16, i16)]) -> Vec<(i16, i16)> {
    points.iter().map(|&(v, _)| (v, v)).collect()
}

/// The points of one contour through `points` (font units), mapped one to one.
fn outline(points: &[(i16, i16)]) -> (Vec<OutlinePoint>, [u16; 1], VecPointMap) {
    let o = points.iter().map(|&(x, y)| OutlinePoint { x: f64::from(x), y: f64::from(y), on_curve: true }).collect();
    let n = u16::try_from(points.len()).unwrap();
    (o, [n - 1], VecPointMap { paths: vec![(0..n).map(Some).collect()], count: n })
}

fn compile(s: &HintSettings, points: &[(i16, i16)], list: &[TtInstruction]) -> (FontPrograms, Result<GlyphProgram, HintingError>) {
    let programs = font_programs(s, &[], COLUMN).unwrap();
    let functions = FunctionTable::from_fpgm(&programs.fpgm).unwrap();
    let (o, ends, map) = outline(points);
    let cx = GlyphContext {
        glyph: "a",
        instructions: list,
        points: &map,
        outline: GlyphOutline { points: &o, contour_ends: &ends, advance: 1000.0 },
        layout: &programs.layout,
        functions: &functions,
        settings: s,
        zones: None,
        stems: None,
        delta_column: Some(COLUMN),
        auto_stems: false,
    };
    let g = glyph_program(&cx);
    (programs, g)
}

fn bytes(s: &HintSettings, points: &[(i16, i16)], list: &[TtInstruction]) -> Vec<u8> {
    compile(s, points, list).1.unwrap().bytes
}

/// Each point's coordinate in direction `d` after hinting at `ppem` (pixels).
fn run(s: &HintSettings, points: &[(i16, i16)], list: &[TtInstruction], d: Direction, ppem: u16, target: Target) -> Vec<f32> {
    let (programs, g) = compile(s, points, list);
    let f = font(&programs, points, &g.unwrap().bytes);
    hinted(&f, ppem, target).into_iter().map(|(x, y)| if d == Direction::Vertical { y } else { x }).collect()
}

fn target_for(d: Direction) -> Target {
    if d == Direction::Vertical { grayscale() } else { mono() }
}

fn px(units: i16, ppem: u16) -> f32 {
    f32::from(units) * f32::from(ppem) / 1000.0
}

#[test]
fn a_snap_rounds_by_its_mode_and_lands_on_its_zones_row() {
    let s = settings(vec![zone("xHeight", 520.0, 20.0)], vec![]);
    // p0 lies in the zone (an overshoot); p1 to p4 lie outside it, one per rounding mode.
    let points = diagonal_points(&[(536, 0), (553, 0), (553, 0), (553, 0), (553, 0), (0, 0)]);
    for d in BOTH {
        let list = [
            snap(0, d, Rounding::Round),
            snap(1, d, Rounding::Round),
            snap(2, d, Rounding::Up),
            snap(3, d, Rounding::Down),
            snap(4, d, Rounding::NoRounding),
        ];
        for ppem in [10, 16, 25, 40] {
            let v = run(&s, &points, &list, d, ppem, target_for(d));
            let in_zone = if d == Direction::Vertical { px(520, ppem).round() } else { px(536, ppem).round() };
            assert_eq!(v[0], in_zone, "{d:?} at {ppem} PPM: the zone's row");
            let at = px(553, ppem);
            assert_eq!([v[1], v[2], v[3]], [at.round(), at.ceil(), at.floor()], "{d:?} at {ppem} PPM");
            assert!((v[4] - at).abs() < EPS, "{d:?} at {ppem} PPM: No rounding keeps {at}, got {}", v[4]);
        }
    }
}

#[test]
fn a_named_zone_keeps_the_point_at_a_rounded_distance() {
    let mut s = settings(vec![zone("xHeight", 520.0, 20.0)], vec![]);
    s.tt_zone_rounding_threshold = 0.4;
    // At 10 PPM the zone sits at 5.2 px and rounds to 6 with the threshold; 600 is 0.8 px above.
    let points = diagonal_points(&[(600, 0), (536, 0), (0, 0)]);
    let named = |k| with(TtKind::Snap { zone: ZoneRef::Index(0) }, Direction::Vertical, Rounding::Round, k, &[]);
    let v = run(&s, &points, &[named(0), named(1)], Direction::Vertical, 10, mono());
    assert_eq!(v[..2], [7.0, 6.0], "one pixel above the zone's row, and in it");
}

#[test]
fn a_stem_keeps_its_rounded_width_and_rounds_up_and_down_away_from_and_towards_its_origin() {
    // Up: 100 → 230; down: 410 → 280. Both 130 units.
    let points = diagonal_points(&[(100, 0), (230, 0), (410, 0), (280, 0), (0, 0)]);
    for d in BOTH {
        let s = settings(vec![], vec![stem(130.0, d), stem(400.0, d)]);
        let list = |r| [stem_from(0, 1, d, r), stem_from(2, 3, d, r)];
        for ppem in [10, 12, 20, 30] {
            let v = run(&s, &points, &list(Rounding::Round), d, ppem, target_for(d));
            let width = px(130, ppem).round();
            assert_eq!(v[0], px(100, ppem).round(), "{d:?}: the untouched origin is rounded");
            assert_eq!((v[1] - v[0], v[2] - v[3]), (width, width), "{d:?} at {ppem} PPM");
        }
        // 1.3 px at 10 PPM: Up gives 2 px, Down 1 px, No rounding the stem's own 1.3 px.
        let up = run(&s, &points, &list(Rounding::Up), d, 10, target_for(d));
        assert_eq!((up[1], up[3]), (1.0 + 2.0, 4.0 - 2.0), "{d:?}: farther from the origin");
        let down = run(&s, &points, &list(Rounding::Down), d, 10, target_for(d));
        assert_eq!((down[1], down[3]), (1.0 + 1.0, 4.0 - 1.0), "{d:?}: nearer the origin");
        let none = run(&s, &points, &list(Rounding::NoRounding), d, 10, target_for(d));
        assert!((none[1] - 2.3).abs() < EPS && (none[3] - 2.7).abs() < EPS, "{d:?}: {none:?}");
    }
}

#[test]
fn a_stem_starting_in_a_zone_snaps_its_origin_to_the_zone() {
    let s = settings(vec![zone("baseline", 0.0, -20.0)], vec![stem(130.0, Direction::Vertical)]);
    let points = diagonal_points(&[(-14, 0), (116, 0), (0, 0)]);
    let v = run(&s, &points, &[stem_from(0, 1, Direction::Vertical, Rounding::Round)], Direction::Vertical, 40, mono());
    assert_eq!(v[..2], [0.0, 5.0], "the overshoot flattened onto the baseline, the stem 5.2 px → 5");
}

#[test]
fn a_shift_copies_the_movement_of_its_origin_then_rounds() {
    let points = diagonal_points(&[(525, 0), (350, 0), (350, 0), (350, 0), (350, 0), (0, 0)]);
    let s = settings(vec![], vec![]);
    for d in BOTH {
        let shift = |t, r| with(TtKind::Shift, d, r, t, &[0]);
        let list = [
            snap(0, d, Rounding::Round),
            shift(1, Rounding::NoRounding),
            shift(2, Rounding::Round),
            shift(3, Rounding::Up),
            shift(4, Rounding::Down),
        ];
        for ppem in [10, 14, 20] {
            let v = run(&s, &points, &list, d, ppem, target_for(d));
            let moved = v[0] - px(525, ppem);
            assert!((v[1] - px(350, ppem) - moved).abs() < EPS, "{d:?} at {ppem} PPM: {v:?}");
        }
        // At 10 PPM: 5.25 → 5, so 3.5 → 3.25.
        let v = run(&s, &points, &list, d, 10, target_for(d));
        assert_eq!(v[1..5], [3.25, 3.0, 4.0, 3.0], "{d:?}");
    }
}

#[test]
fn an_interpolate_keeps_the_ratio_between_its_references() {
    let points = diagonal_points(&[(0, 0), (525, 0), (375, 0), (375, 0), (375, 0), (375, 0), (1000, 0)]);
    let s = settings(vec![], vec![]);
    for d in BOTH {
        let ip = |t, r| with(TtKind::Interpolate, d, r, t, &[0, 1]);
        let list = [
            snap(0, d, Rounding::Round),
            snap(1, d, Rounding::Round),
            ip(2, Rounding::NoRounding),
            ip(3, Rounding::Round),
            ip(4, Rounding::Up),
            ip(5, Rounding::Down),
        ];
        for ppem in [10, 16, 23] {
            let v = run(&s, &points, &list, d, ppem, target_for(d));
            let ratio = (v[2] - v[0]) / (v[1] - v[0]);
            assert!((ratio - 375.0 / 525.0).abs() < 2.0 / 64.0, "{d:?} at {ppem} PPM: {v:?}");
        }
        // At 10 PPM the references land on 0 and 5, so the target on 3.57.
        let v = run(&s, &points, &list, d, 10, target_for(d));
        assert_eq!(v[3..6], [4.0, 4.0, 3.0], "{d:?}");
    }
}

#[test]
fn a_delta_moves_its_point_one_pixel_at_its_size_only() {
    let points = diagonal_points(&[(525, 0), (300, 0), (0, 0)]);
    let s = settings(vec![], vec![]);
    let v = Direction::Vertical;
    let delta = |k, ppm, dir| {
        let cells: Deltas = [(COLUMN.to_string(), [(ppm, dir)].into())].into();
        with(TtKind::Delta { cells }, v, Rounding::NoRounding, k, &[])
    };
    let plain = [snap(0, v, Rounding::Round), snap(1, v, Rounding::Round)];
    let mut list = plain.to_vec();
    list.extend([delta(0, 10, DeltaDir::Up), delta(1, 12, DeltaDir::Down)]);
    for target in [mono(), grayscale(), gdi()] {
        for ppem in 8..=14 {
            let a = run(&s, &points, &plain, v, ppem, target);
            let b = run(&s, &points, &list, v, ppem, target);
            let want = [if ppem == 10 { 1.0 } else { 0.0 }, if ppem == 12 { -1.0 } else { 0.0 }];
            assert_eq!([b[0] - a[0], b[1] - a[1]], want, "at {ppem} PPM");
        }
    }
    // A horizontal Delta is left out with a warning.
    let mut h = delta(0, 10, DeltaDir::Up);
    h.direction = Direction::Horizontal;
    let g = compile(&s, &points, &[plain[0].clone(), h]).1.unwrap();
    assert_eq!(g.warnings.len(), 1, "{:?}", g.warnings);
    assert_eq!(g.bytes, bytes(&s, &points, &plain[..1]));
}

#[test]
fn the_gdi_only_mode_rounds_under_gdi_cleartype_only() {
    let s = settings(vec![zone("xHeight", 520.0, 20.0)], vec![]);
    let points = diagonal_points(&[(553, 0), (536, 0), (0, 0)]);
    let v = Direction::Vertical;
    let list = [snap(0, v, Rounding::GdiOnly), snap(1, v, Rounding::GdiOnly)];
    assert_eq!(run(&s, &points, &list, v, 10, gdi())[..2], [6.0, 5.0], "rounded, and on the zone's row");
    for target in [grayscale(), mono()] {
        let y = run(&s, &points, &list, v, 10, target);
        assert!((y[0] - 5.53).abs() < EPS && (y[1] - 5.36).abs() < EPS, "{y:?}");
    }
}

#[test]
fn a_triple_hint_keeps_a_pixel_between_its_stems() {
    // Three 80-unit stems at 0, 120 and 320: at 9 PPM the lower two touch once rounded.
    let points = diagonal_points(&[(0, 0), (80, 0), (120, 0), (200, 0), (320, 0), (400, 0), (1000, 0)]);
    for d in BOTH {
        let s = settings(vec![], vec![stem(80.0, d)]);
        let member = |o, t, triple| with(TtKind::Stem { stem: StemRef::Auto, triple }, d, Rounding::Round, t, &[o]);
        let plain = [member(0, 1, None), member(2, 3, None), member(4, 5, None)];
        let v = run(&s, &points, &plain, d, 9, target_for(d));
        assert_eq!(v[2] - v[1], 0.0, "{d:?}: without the triple hint the lower stems touch: {v:?}");
        // Listed out of order: the program sorts the stems by position.
        let triple = [member(4, 5, Some(0)), member(0, 1, Some(0)), member(2, 3, Some(0))];
        for ppem in 8..=24 {
            let v = run(&s, &points, &triple, d, ppem, target_for(d));
            assert!(v[2] - v[1] >= 1.0 && v[4] - v[3] >= 1.0, "{d:?} at {ppem} PPM: {v:?}");
            assert_eq!((v[1] - v[0], v[3] - v[2], v[5] - v[4]), widths(ppem), "{d:?} at {ppem} PPM: the widths stay");
        }
    }
}

/// The rounded width of the 80-unit stems, at least the 1 px a sub-pixel stem rounds up to.
fn widths(ppem: u16) -> (f32, f32, f32) {
    let w = px(80, ppem).round().max(1.0);
    (w, w, w)
}

#[test]
fn ignore_vertical_hints_leaves_out_horizontal_stems_on_vertical_tt_stems() {
    let points = diagonal_points(&[(100, 0), (230, 0), (0, 0)]);
    let mut s = settings(vec![], vec![stem(130.0, Direction::Vertical), stem(130.0, Direction::Horizontal)]);
    s.ignore_vertical_hints = true;
    let h = stem_from(0, 1, Direction::Horizontal, Rounding::Round);
    let v = stem_from(0, 1, Direction::Vertical, Rounding::Round);
    assert_eq!(bytes(&s, &points, &[h.clone(), v.clone()]), bytes(&s, &points, std::slice::from_ref(&v)));
    let mut none = h.clone();
    none.kind = TtKind::Stem { stem: StemRef::NoStem, triple: None };
    assert_ne!(bytes(&s, &points, &[none, v.clone()]), bytes(&s, &points, std::slice::from_ref(&v)), "a Stem without a TT stem stays");
    s.ignore_vertical_hints = false;
    assert_ne!(bytes(&s, &points, &[h, v.clone()]), bytes(&s, &points, &[v]));
}

#[test]
fn a_point_on_a_diagonal_moves_along_it_unless_diagonals_are_not_preserved() {
    // The foot of a slanted stem at the origin, a crossbar corner on it at (150, 333).
    let points = [(0, 0), (150, 333), (500, 333), (500, 0)];
    let v = Direction::Vertical;
    let list = [snap(0, v, Rounding::Round), snap(1, v, Rounding::Round)];
    let mut s = settings(vec![], vec![]);
    let hinted_at = |s: &HintSettings| {
        let (programs, g) = compile(s, &points, &list);
        hinted(&font(&programs, &points, &g.unwrap().bytes), 10, mono())
    };
    let kept = hinted_at(&s);
    assert_eq!(kept[1].1, 3.0);
    assert!((kept[1].0 - 3.0 * 150.0 / 333.0).abs() < 2.0 / 64.0, "along the slant: {kept:?}");
    s.tt_dont_preserve_diagonals = true;
    let plain = hinted_at(&s);
    assert_eq!(plain[1], (1.5, 3.0), "straight down");
    assert!(bytes(&s, &points, &list).len() < compile(&settings(vec![], vec![]), &points, &list).1.unwrap().bytes.len());
}

#[test]
fn a_point_missing_from_the_outline_is_left_out_with_a_warning() {
    let points = diagonal_points(&[(525, 0), (300, 0), (0, 0)]);
    let s = settings(vec![], vec![]);
    let v = Direction::Vertical;
    let mut dangling = snap(0, v, Rounding::Round);
    dangling.target = TtPoint::Node(NodeRef::new(3, 0));
    let mut shift = with(TtKind::Shift, v, Rounding::NoRounding, 1, &[0]);
    shift.origin = Some(TtPoint::Intersection { a: NodeRef::new(0, 0), b: NodeRef::new(0, 1) });
    let g = compile(&s, &points, &[dangling, snap(1, v, Rounding::Round), shift]).1.unwrap();
    assert_eq!(g.warnings.len(), 2, "{:?}", g.warnings);
    assert!(g.warnings[0].contains("point [3, 0]"), "{:?}", g.warnings);
    assert_eq!(g.bytes, bytes(&s, &points, &[snap(1, v, Rounding::Round)]));
}

#[test]
fn a_program_past_the_glyf_limit_or_a_mismatched_outline_is_refused() {
    let points = diagonal_points(&[(525, 0), (300, 0), (0, 0)]);
    let s = settings(vec![], vec![]);
    let cells: Deltas = [(COLUMN.to_string(), (1..=MAX_DELTA_PPM).map(|ppm| (ppm, DeltaDir::Up)).collect())].into();
    let delta = with(TtKind::Delta { cells }, Direction::Vertical, Rounding::NoRounding, 0, &[]);
    let list = vec![delta; 100];
    match compile(&s, &points, &list).1 {
        Err(HintingError::Glyph { message, .. }) => assert!(message.contains("65535"), "{message}"),
        other => panic!("{other:?}"),
    }
    let programs = font_programs(&s, &[], COLUMN).unwrap();
    let functions = FunctionTable::from_fpgm(&programs.fpgm).unwrap();
    let (o, _, map) = outline(&points);
    let cx = GlyphContext {
        glyph: "a",
        instructions: &list[..1],
        points: &map,
        outline: GlyphOutline { points: &o, contour_ends: &[1], advance: 1000.0 },
        layout: &programs.layout,
        functions: &functions,
        settings: &s,
        zones: None,
        stems: None,
        delta_column: Some(COLUMN),
        auto_stems: false,
    };
    assert!(matches!(glyph_program(&cx), Err(HintingError::Glyph { .. })));
}

#[test]
fn every_instruction_runs_within_its_analysed_limits_at_every_size_and_intent() {
    let points = diagonal_points(&[(0, 0), (80, 0), (120, 0), (200, 0), (320, 0), (400, 0), (536, 0), (553, 0), (250, 0), (1000, 0)]);
    let mut list = Vec::new();
    for d in BOTH {
        let member = |o, t| with(TtKind::Stem { stem: StemRef::Auto, triple: Some(axis_bit(d).into()) }, d, Rounding::GdiOnly, t, &[o]);
        list.extend([member(0, 1), member(2, 3), member(4, 5), snap(6, d, Rounding::GdiOnly), snap(7, d, Rounding::Up)]);
        list.push(with(TtKind::Shift, d, Rounding::Down, 8, &[7]));
        list.push(with(TtKind::Interpolate, d, Rounding::Round, 9, &[0, 7]));
    }
    let cells: Deltas = [(COLUMN.to_string(), [(9, DeltaDir::Up), (20, DeltaDir::Down)].into())].into();
    list.push(with(TtKind::Delta { cells }, Direction::Vertical, Rounding::NoRounding, 6, &[]));
    let mut s = settings(vec![zone("xHeight", 520.0, 20.0)], vec![stem(80.0, Direction::Vertical), stem(80.0, Direction::Horizontal)]);
    s.tt_overshoot_suppression_below_ppm = Some(20);
    let (programs, g) = compile(&s, &points, &list);
    let g = g.unwrap();
    assert!(g.warnings.is_empty() && !g.limits.uncertain, "{g:?}");
    let f = font(&programs, &points, &g.bytes);
    for ppem in 8..=72 {
        for target in [mono(), grayscale(), gdi()] {
            assert!(hinted(&f, ppem, target).iter().all(|(x, y)| x.is_finite() && y.is_finite()));
        }
    }
}
