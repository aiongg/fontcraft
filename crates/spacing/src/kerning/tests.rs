#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Param};

use super::*;
use crate::LINK_WITH_FIRST_MASTER;

fn group(f: &mut Font, glyph: &str, side: GroupSide, name: &str) {
    if f.glyph(glyph).is_none() {
        f.add_glyph(glyph, vec![], 500.0).unwrap();
    }
    let mut g = f.glyph_mut(glyph).unwrap();
    let slot = match side {
        GroupSide::Left => &mut g.kern_groups.left,
        GroupSide::Right => &mut g.kern_groups.right,
        GroupSide::Top => &mut g.kern_groups.top,
        GroupSide::Bottom => &mut g.kern_groups.bottom,
    };
    *slot = Some(name.to_string());
}

fn set(f: &mut Font, direction: Direction, first: &str, second: &str, value: f64) {
    direction.table_mut(Arc::make_mut(&mut f.kerning)).set("m01", first, second, value);
}

/// The oracle test font of `docs/plans/M5.md` § 2: A–@V −30, @A–V −40, @A–@V −50, T–@o −60 with
/// the exception T–oacute 0, and the right-to-left @alef–@bet −77. A second master follows the
/// first.
fn oracle_font() -> Font {
    let mut f = Font::new("Kern");
    group(&mut f, "A", GroupSide::Right, "A");
    group(&mut f, "Aacute", GroupSide::Right, "A");
    group(&mut f, "V", GroupSide::Left, "V");
    group(&mut f, "W", GroupSide::Left, "V");
    group(&mut f, "o", GroupSide::Left, "o");
    group(&mut f, "oacute", GroupSide::Left, "o");
    group(&mut f, "alef", GroupSide::Left, "alef");
    group(&mut f, "bet", GroupSide::Right, "bet");
    set(&mut f, Direction::Ltr, "A", "@MMK_R_V", -30.0);
    set(&mut f, Direction::Ltr, "@MMK_L_A", "V", -40.0);
    set(&mut f, Direction::Ltr, "@MMK_L_A", "@MMK_R_V", -50.0);
    set(&mut f, Direction::Ltr, "T", "@MMK_R_o", -60.0);
    set(&mut f, Direction::Ltr, "T", "oacute", 0.0);
    let rtl =
        (Side::Group("alef".into()).stored(Direction::Rtl, Position::First), Side::Group("bet".into()).stored(Direction::Rtl, Position::Second));
    assert_eq!(rtl, ("@MMK_R_alef".to_string(), "@MMK_L_bet".to_string()), "right-to-left sides are written by the side they stand on");
    set(&mut f, Direction::Rtl, &rtl.0, &rtl.1, -77.0);
    let mut linked = f.masters[0].clone();
    linked.id = "m02".into();
    linked.name = "Linked".into();
    linked.params.0.push(Param::new(LINK_WITH_FIRST_MASTER, Value::Bool(true)));
    Arc::make_mut(&mut f.masters).push(linked);
    f
}

fn applied(index: &KernIndex, direction: Direction, master: &str, first: &str, second: &str) -> Option<(f64, Step)> {
    index.lookup(direction, master, first, second).unwrap().map(|a| (a.value, a.step))
}

#[test]
fn lookup_follows_the_oracles_order() {
    let f = oracle_font();
    let index = KernIndex::new(&f);
    let ltr = |a, b| applied(&index, Direction::Ltr, "m01", a, b);
    assert_eq!(ltr("A", "V"), Some((-30.0, Step::GlyphGroup)), "glyph–group wins over group–glyph");
    assert_eq!(ltr("A", "W"), Some((-30.0, Step::GlyphGroup)));
    assert_eq!(ltr("Aacute", "V"), Some((-40.0, Step::GroupGlyph)));
    assert_eq!(ltr("Aacute", "W"), Some((-50.0, Step::GroupGroup)));
    assert_eq!(ltr("T", "oacute"), Some((0.0, Step::GlyphGlyph)), "a zero exception is kept");
    assert_eq!(ltr("T", "o"), Some((-60.0, Step::GlyphGroup)));
    assert_eq!(ltr("V", "A"), None);
    assert_eq!(ltr("alef", "bet"), None, "directions are stored separately");
    assert_eq!(applied(&index, Direction::Rtl, "m01", "alef", "bet"), Some((-77.0, Step::GroupGroup)));
    assert_eq!(applied(&index, Direction::Rtl, "m01", "bet", "alef"), None, "right-to-left pairs are in logical order");

    let linked = index.lookup(Direction::Ltr, "m02", "Aacute", "W").unwrap().unwrap();
    assert_eq!(
        (linked.value, linked.master, linked.first, linked.second),
        (-50.0, "m01", "@MMK_L_A", "@MMK_R_V"),
        "a linked master shows its source's kerning"
    );
    assert_eq!(index.lookup(Direction::Ltr, "m09", "A", "V"), Err(KernError::UnknownMaster("m09".into())));
    assert_eq!(index.lookup(Direction::Ltr, "m01", "A", "nope"), Err(KernError::UnknownGlyph("nope".into())));

    assert_eq!(index.members(GroupSide::Right, "A").collect::<Vec<_>>(), ["A", "Aacute"]);
    assert_eq!(members(&f, GroupSide::Left, "V"), ["V", "W"]);
    assert!(members(&f, GroupSide::Right, "V").is_empty(), "left and right groups are separate");
    let left: Vec<&str> = group_list(&f, GroupSide::Left).into_iter().map(|(g, _)| g).collect();
    assert_eq!(left, ["V", "o", "alef"], "groups in the order of their first member");

    let named: Vec<(String, String)> = pairs_naming(&f, &["T", "V"]).into_iter().map(|p| (p.first, p.second)).collect();
    assert_eq!(named, [("@MMK_L_A", "V"), ("T", "@MMK_R_o"), ("T", "oacute")].map(|(a, b)| (a.to_string(), b.to_string())));

    let found = stored_pair(&f, Direction::Ltr, "m01", &Side::parse("@A").unwrap(), &Side::parse("V").unwrap()).unwrap();
    assert_eq!((found.first, found.second, found.value), ("@MMK_L_A", "V", -40.0));
    assert!(stored_pair(&f, Direction::Ltr, "m01", &Side::parse("A").unwrap(), &Side::parse("V").unwrap()).is_none());
}

#[test]
fn sides_convert_per_direction_and_position() {
    let g = Side::parse("@round").unwrap();
    let cases = [
        (Direction::Ltr, Position::First, "@MMK_L_round", GroupSide::Right),
        (Direction::Ltr, Position::Second, "@MMK_R_round", GroupSide::Left),
        (Direction::Rtl, Position::First, "@MMK_R_round", GroupSide::Left),
        (Direction::Rtl, Position::Second, "@MMK_L_round", GroupSide::Right),
        (Direction::Vertical, Position::First, "@MMK_T_round", GroupSide::Bottom),
        (Direction::Vertical, Position::Second, "@MMK_B_round", GroupSide::Top),
    ];
    for (d, p, stored, side) in cases {
        assert_eq!(g.stored(d, p), stored, "{d:?} {p:?}");
        assert_eq!(facing(d, p), side, "{d:?} {p:?}");
        assert_eq!(Side::from_stored(stored).command(), "@round");
    }
    assert_eq!(Side::from_stored("public.kern2.O"), Side::Group("O".into()));
    assert_eq!(Side::parse("A").unwrap().stored(Direction::Rtl, Position::First), "A");
    assert_eq!(Side::parse("@"), Err(SideError::Group(GroupNameError::Empty)));
    assert_eq!(Side::parse(""), Err(SideError::Empty));
    assert_eq!(check_group_name("o.sc-1_x"), Ok(()));
    assert_eq!(check_group_name("ab c"), Err(GroupNameError::Character { ch: ' ', position: 3 }));
    assert_eq!(check_group_name("é"), Err(GroupNameError::Character { ch: 'é', position: 1 }));
    assert_eq!(check_group_name(&"a".repeat(MAX_GROUP_NAME_LEN + 1)), Err(GroupNameError::TooLong));
}

#[test]
fn vertical_pairs_use_bottom_and_top_groups() {
    let mut f = Font::new("Vert");
    group(&mut f, "b", GroupSide::Bottom, "B1");
    group(&mut f, "b", GroupSide::Right, "R1");
    group(&mut f, "c", GroupSide::Top, "C1");
    group(&mut f, "d", GroupSide::Top, "C1");
    set(&mut f, Direction::Vertical, "@MMK_T_B1", "@MMK_B_C1", 30.0);
    set(&mut f, Direction::Ltr, "@MMK_L_R1", "c", -5.0);
    let index = KernIndex::new(&f);
    assert_eq!(applied(&index, Direction::Vertical, "m01", "b", "d"), Some((30.0, Step::GroupGroup)));
    assert_eq!(applied(&index, Direction::Vertical, "m01", "d", "b"), None);
    assert_eq!(applied(&index, Direction::Ltr, "m01", "b", "c"), Some((-5.0, Step::GroupGlyph)), "horizontal groups do not reach vertical pairs");
    assert_eq!(applied(&index, Direction::Vertical, "m01", "b", "c").map(|a| a.0), Some(30.0));
    let l = locks(&index, Direction::Vertical, "m01", "b", "c").unwrap();
    assert_eq!(l, Locks { first: Lock::Closed, second: Lock::Closed });
}

/// T and Y share the right group T; o and odieresis the left group o; x has no group.
fn lock_font(pairs: &[(&str, &str, f64)]) -> Font {
    let mut f = Font::new("Locks");
    group(&mut f, "T", GroupSide::Right, "T");
    group(&mut f, "Y", GroupSide::Right, "T");
    group(&mut f, "o", GroupSide::Left, "o");
    group(&mut f, "odieresis", GroupSide::Left, "o");
    for (a, b, v) in pairs {
        set(&mut f, Direction::Ltr, a, b, *v);
    }
    f
}

fn pair(e: &Edit) -> (String, String, f64) {
    (e.first.command(), e.second.command(), e.value)
}

#[test]
fn locks_follow_the_pair_that_applies() {
    use Lock::{Closed, Open, Ungrouped};
    // The rows of the note's table § 3.6: the stored pair, the locks it gives and the pair a
    // shortcut of +1 writes.
    let rows = [
        (("@MMK_L_T", "@MMK_R_o"), (Closed, Closed), ("@T", "@o")),
        (("@MMK_L_T", "odieresis"), (Closed, Open), ("@T", "odieresis")),
        (("T", "@MMK_R_o"), (Open, Closed), ("T", "@o")),
        (("T", "odieresis"), (Open, Open), ("T", "odieresis")),
    ];
    for ((a, b), (l, r), (wa, wb)) in rows {
        let f = lock_font(&[("@MMK_L_T", "@MMK_R_o", -60.0), (a, b, -40.0)]);
        let index = KernIndex::new(&f);
        assert_eq!(locks(&index, Direction::Ltr, "m01", "T", "odieresis").unwrap(), Locks { first: l, second: r }, "{a} {b}");
        let e = shortcut_edit(&index, Direction::Ltr, "m01", "T", "odieresis", 1.0).unwrap();
        assert_eq!(pair(&e), (wa.to_string(), wb.to_string(), -39.0), "{a} {b}");
        assert_eq!(locked_pair(&index, Direction::Ltr, "Y", "o", Locks { first: l, second: r }).0.is_group(), l == Closed);
    }

    // No pair, no group on the left: the glyph is written as itself.
    let f = lock_font(&[]);
    let index = KernIndex::new(&f);
    assert_eq!(locks(&index, Direction::Ltr, "m01", "x", "o").unwrap(), Locks { first: Ungrouped, second: Closed });
    assert_eq!(pair(&shortcut_edit(&index, Direction::Ltr, "m01", "x", "o", -10.0).unwrap()), ("x".into(), "@o".into(), -10.0));
    assert_eq!(open_locks(&index, Direction::Ltr, "m01", "x", "o", Which::First).unwrap(), None, "an ungrouped side has no lock to open");
    assert_eq!(shortcut_edit(&index, Direction::Ltr, "m01", "x", "o", f64::NAN), Err(KernError::NotFinite));

    // Opening the right lock stores the exception at the value that applies; closing removes it.
    let mut f = lock_font(&[("@MMK_L_T", "@MMK_R_o", -60.0)]);
    let index = KernIndex::new(&f);
    let e = open_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::Second).unwrap().unwrap();
    assert_eq!(pair(&e), ("@T".into(), "odieresis".into(), -60.0));
    assert_eq!(close_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::Both).unwrap(), None, "nothing is open yet");
    set(&mut f, Direction::Ltr, &e.first.stored(Direction::Ltr, Position::First), &e.second.stored(Direction::Ltr, Position::Second), e.value);
    assert!(!index.is_current(&f), "a kerning change makes the index stale");
    let index = KernIndex::new(&f);
    assert_eq!(locks(&index, Direction::Ltr, "m01", "T", "odieresis").unwrap(), Locks { first: Closed, second: Open });
    assert_eq!(applied(&index, Direction::Ltr, "m01", "T", "odieresis"), Some((-60.0, Step::GroupGlyph)), "opening a lock moves nothing");
    assert_eq!(open_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::Second).unwrap(), None);
    assert_eq!(close_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::First).unwrap(), None, "the left lock is closed");
    let gone = close_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::Second).unwrap().unwrap();
    assert_eq!((gone.first, gone.second), ("@MMK_L_T", "odieresis"));
    let both = open_locks(&index, Direction::Ltr, "m01", "T", "odieresis", Which::Both).unwrap().unwrap();
    assert_eq!(pair(&both), ("T".into(), "odieresis".into(), -60.0));

    // An outline edit keeps the index; a group edit does not.
    let mut g = f.clone();
    g.glyph_mut("T").unwrap().layers[0].width += 10.0;
    assert!(index.is_current(&g));
    group(&mut g, "T", GroupSide::Right, "Y");
    assert!(!index.is_current(&g));
}
