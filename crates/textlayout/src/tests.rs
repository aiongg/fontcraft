#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_spacing::kerning::GroupSide;

use super::*;

/// The default test font (`A`–`Z`, `a`–`z` at 600, `space` at 250) with `dieresiscomb` (no
/// character), `odieresis`, `exclamdown`, `exclam` and `slash`.
fn font() -> Font {
    let mut f = Font::new("Text");
    f.add_glyph("dieresiscomb", vec![], 0.0).unwrap();
    f.add_glyph("odieresis", vec![0xF6], 600.0).unwrap();
    f.add_glyph("exclamdown", vec![0xA1], 300.0).unwrap();
    f.add_glyph("exclam", vec![0x21], 300.0).unwrap();
    f.add_glyph("slash", vec![0x2F], 400.0).unwrap();
    f
}

fn kern(f: &mut Font, direction: Direction, first: &str, second: &str, value: f64) {
    direction.table_mut(Arc::make_mut(&mut f.kerning)).set("m01", first, second, value);
}

fn names(layout: &Layout) -> Vec<Option<&str>> {
    layout.placed.iter().map(|p| p.glyph.as_deref()).collect()
}

fn text(f: &Font, s: &str) -> Text {
    Text::from_display_string(f, s).unwrap()
}

#[test]
fn display_strings_read_name_escapes_and_write_back() {
    let f = font();
    let t = text(&f, "o/dieresiscomb ö");
    assert_eq!(
        t.items(),
        [Item::Glyph { name: "o".into(), typed: Some('o') }, Item::glyph("dieresiscomb"), Item::Glyph { name: "odieresis".into(), typed: Some('ö') },],
        "a space after a name ends it and is dropped"
    );
    assert_eq!(t.to_display_string(&f), "o/dieresiscomb ö");

    let t = text(&f, "/exclamdown Hola /Placeholder/exclam\nsecond line");
    assert_eq!(t.len(), 1 + 5 + 2 + 1 + 11);
    assert_eq!(t.items()[6], Item::Placeholder);
    assert_eq!(t.items()[7], Item::glyph("exclam"));
    assert_eq!(t.items()[8], Item::LineBreak);
    assert_eq!(t.to_display_string(&f), "/exclamdown Hola /Placeholder/exclam\nsecond line");

    // A bare slash is the slash character; an unneeded space after a name is not written back.
    let t = text(&f, "a/ b//A /B");
    assert_eq!(t.items()[1], Item::Glyph { name: "slash".into(), typed: Some('/') });
    assert_eq!(t.items()[2], Item::Glyph { name: "b".into(), typed: Some('b') });
    assert_eq!(t.items()[3], Item::Glyph { name: "slash".into(), typed: Some('/') });
    assert_eq!(t.to_display_string(&f), "a/ b//A/B");
    assert_eq!(text(&f, &t.to_display_string(&f)).items(), t.items());

    // Names the font lacks and characters no glyph encodes are kept.
    let t = text(&f, "/nosuch é\r\n");
    assert_eq!(t.items(), [Item::glyph("nosuch"), Item::Missing('é'), Item::LineBreak]);
    assert_eq!(t.to_display_string(&f), "/nosuch é\n");
    let l = layout(&f, "m01", &t, &Options::default()).unwrap();
    assert_eq!(names(&l), [None, None, None]);
    assert_eq!(l.placed[1].advance, 500.0, "a missing character takes half the em");

    assert_eq!(text(&f, "A/dieresiscomb").to_plain_text(&f), "A", "a glyph without a character has no plain text");
    let long = format!("/{}", "x".repeat(300));
    assert!(matches!(Text::from_display_string(&f, &long), Err(TextError::NameTooLong { .. })));
}

#[test]
fn placeholders_show_the_glyph_after_the_cursor() {
    let f = font();
    let mut t = text(&f, "n/Placeholder n");
    assert_eq!(t.current_glyph(), Some("n"));
    t.insert(3, vec![Item::Glyph { name: "o".into(), typed: Some('o') }]).unwrap();
    t.set_cursor(3);
    assert_eq!(t.current_glyph(), Some("o"));
    let l = layout(&f, "m01", &t, &Options::default()).unwrap();
    assert_eq!(names(&l), [Some("n"), Some("o"), Some("n"), Some("o")]);
    // Next to a placeholder, the current glyph stays; at the end, it is the glyph before.
    t.set_cursor(1);
    assert_eq!(t.current_glyph(), Some("o"));
    assert_eq!(t.to_plain_text(&f), "nono");
    t.set_cursor(4);
    assert_eq!(t.current_glyph(), Some("o"));
    t.set_cursor(2);
    assert_eq!(t.current_glyph(), Some("n"));
    assert_eq!(t.to_plain_text(&f), "nnno");
}

#[test]
fn right_to_left_reverses_each_line_and_kerns_from_its_own_table() {
    let mut f = font();
    kern(&mut f, Direction::Ltr, "A", "V", -50.0);
    kern(&mut f, Direction::Rtl, "A", "V", -77.0);
    let t = text(&f, "AV\nA");
    let rtl = Options { direction: Direction::Rtl, ..Options::default() };
    let l = layout(&f, "m01", &t, &rtl).unwrap();
    let (a, v) = (&l.placed[0], &l.placed[1]);
    assert_eq!((a.x, a.y), (-600.0, 0.0), "the first glyph ends at x = 0");
    assert_eq!(v.x, -600.0 - 600.0 + 77.0, "the second glyph stands to its left, kerned");
    assert_eq!(v.kern, -77.0);
    assert_eq!(l.placed[3].x, -600.0, "each line starts again at the right");
    assert!(l.placed[3].y < 0.0);
    assert_eq!(l.hit(-100.0, 100.0), 0, "the right half of the first glyph is before it");
    assert_eq!(l.hit(-500.0, 100.0), 1);
    assert_eq!(l.caret(1), (-523.0, 0.0), "the cursor stands at the start of the kerned glyph");
}

#[test]
fn vertical_text_uses_vertical_widths_and_origins() {
    let mut f = font();
    {
        let mut g = f.glyph_mut("A").unwrap();
        g.layers[0].info.vert_width = Some(700.0);
        g.layers[0].info.vert_origin = Some(50.0);
    }
    let t = text(&f, "AB");
    let l = layout(&f, "m01", &t, &Options { direction: Direction::Vertical, ..Options::default() }).unwrap();
    let (a, b) = (&l.placed[0], &l.placed[1]);
    assert_eq!(a.advance, 700.0);
    assert_eq!(b.advance, 1000.0, "without a vertical width the ascender to the descender");
    // The top of A's advance box (ascender 800 less the origin 50) sits at y = 0.
    assert_eq!((a.x, a.y), (-300.0, -750.0));
    assert_eq!(b.y, -700.0 - 800.0);
    assert_eq!(l.hit(0.0, -1000.0), 1);
    assert_eq!(l.caret(2), (0.0, -1700.0));
}

#[test]
fn lines_wrap_after_the_last_space_or_before_the_overflowing_glyph() {
    let f = font();
    let t = text(&f, "AA AA\nAAAA");
    let opts = Options { line_width: 2000.0, line_height: Some(1000.0), ..Options::default() };
    let l = layout(&f, "m01", &t, &opts).unwrap();
    let ranges: Vec<_> = l.lines.iter().map(|line| line.items.clone()).collect();
    assert_eq!(ranges, [0..3, 3..6, 6..9, 9..10]);
    assert_eq!(l.placed[3].offset, 0.0);
    assert_eq!(l.placed[3].y, -1000.0);
    assert_eq!(l.lines[0].length, 1450.0, "a space hangs at the end of its line");
    assert_eq!(l.hit(5000.0, -1000.0), 5, "past the end of a line is before its line break");
    assert_eq!(l.hit(650.0, -2000.0), 7);

    let one = layout(&f, "m01", &t, &Options { single_line: true, line_width: 1.0, ..opts }).unwrap();
    assert_eq!(one.lines.len(), 1, "the preview line neither wraps nor breaks");
    assert_eq!(one.placed[9].x, 600.0 * 7.0 + 250.0);
}

#[test]
fn kerning_modes() {
    let mut f = font();
    f.glyph_mut("V").unwrap().kern_groups.left = Some("V".into());
    f.glyph_mut("W").unwrap().kern_groups.left = Some("V".into());
    kern(&mut f, Direction::Ltr, "A", "@MMK_R_V", -50.0);
    let t = text(&f, "AWA");
    let x_of = |mode: KerningMode| {
        let l = layout(&f, "m01", &t, &Options { kerning: mode, ..Options::default() }).unwrap();
        (l.placed[1].x, l.placed[1].kern, l.placed[2].kern)
    };
    assert_eq!(x_of(KerningMode::None), (600.0, 0.0, 0.0));
    assert_eq!(x_of(KerningMode::Kerning), (550.0, -50.0, 0.0));
    assert_eq!(x_of(KerningMode::Locked), (550.0, -50.0, 0.0));
    let index = fontcraft_spacing::kerning::KernIndex::new(&f);
    let l = layout(&f, "m01", &t, &Options { index: Some(&index), ..Options::default() }).unwrap();
    assert_eq!(l.placed[1].kern, -50.0);
    assert_eq!(GroupSide::Left.name(), "left");
    assert_eq!(KerningMode::from_name("locked"), Some(KerningMode::Locked));
}

#[test]
fn edits_stay_within_the_item_limit() {
    let f = font();
    let full = "a".repeat(MAX_ITEMS);
    let mut t = text(&f, &full);
    assert_eq!(t.len(), MAX_ITEMS);
    assert!(matches!(Text::from_display_string(&f, &format!("{full}a")), Err(TextError::TooManyItems { limit: MAX_ITEMS })));
    let before = t.clone();
    assert!(t.insert(5, vec![Item::Placeholder]).is_err());
    assert!(t.replace(5..20, vec![Item::Placeholder; 16]).is_err());
    assert!(t.delete(5..MAX_ITEMS + 1).is_err());
    assert_eq!(t, before, "a failed edit changes nothing");
    t.replace(5..20, vec![Item::Placeholder; 15]).unwrap();
    assert_eq!((t.len(), t.cursor()), (MAX_ITEMS, 20));
    t.select(2, 0);
    assert_eq!(t.selection(), 0..2);
    t.delete(t.selection()).unwrap();
    assert_eq!(t.len(), MAX_ITEMS - 2);
    assert!(Text::with_limit(MAX_PREVIEW_ITEMS + 1).is_err());
    assert!(layout(&f, "m01", &t, &Options { line_width: f64::NAN, ..Options::default() }).is_err());
    assert!(layout(&f, "nope", &t, &Options::default()).is_err());
}
