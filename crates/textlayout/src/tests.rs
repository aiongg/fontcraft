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
fn bidi_keeps_latin_words_in_order_in_right_to_left_text() {
    let mut f = font();
    f.add_glyph("alef", vec![0x5D0], 500.0).unwrap();
    f.add_glyph("bet", vec![0x5D1], 450.0).unwrap();
    f.add_glyph("one", vec![0x31], 550.0).unwrap();
    f.add_glyph("two", vec![0x32], 550.0).unwrap();
    kern(&mut f, Direction::Ltr, "f", "o", -40.0);
    kern(&mut f, Direction::Rtl, "f", "o", -99.0);
    kern(&mut f, Direction::Rtl, "alef", "space", -30.0);
    let t = text(&f, "\u{5d0} brown fox \u{5d1} 12");
    let x = |l: &Layout, i: usize| l.placed[i].x;

    // Without bidi every letter runs right to left, the Latin ones reversed.
    let plain = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, ..Options::default() }).unwrap();
    assert!(x(&plain, 3) < x(&plain, 2));

    let l = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, bidi: true, ..Options::default() }).unwrap();
    // alef ␣ b r o w n ␣ f o x ␣ bet ␣ 1 2: the runs right to left, letters and digits in order.
    let latin: Vec<f64> = (2..7).chain(8..11).map(|i| x(&l, i)).collect();
    assert!(latin.windows(2).all(|p| p[0] < p[1]), "brown fox reads left to right: {latin:?}");
    assert_eq!(x(&l, 0), -500.0, "alef starts the line at the right");
    assert_eq!(x(&l, 1), -500.0 - 250.0 + 30.0, "the space after it kerns from the right-to-left table");
    assert_eq!(x(&l, 10) + 600.0, x(&l, 1), "the Latin run ends where the space begins");
    assert_eq!(x(&l, 9) - x(&l, 8), 600.0 - 40.0, "f o kerns from the left-to-right table");
    assert_eq!(l.placed[9].kern, -40.0);
    assert!(x(&l, 12) < x(&l, 2), "bet stands left of the Latin run");
    assert!(x(&l, 14) < x(&l, 15) && x(&l, 15) < x(&l, 12), "the number reads left to right, left of bet");
    assert_eq!(x(&l, 14), -l.lines[0].length, "the line ends at its length");

    // An embedding gives a range its own direction: a left-to-right range holding a Hebrew word
    // and a Latin word sets them left to right.
    let t = text(&f, "\u{5d0}\u{5d1} fox");
    let embeddings = [Embedding { items: 0..6, rtl: false }];
    let l = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, bidi: true, embeddings: &embeddings, ..Options::default() }).unwrap();
    assert!(x(&l, 1) < x(&l, 0), "the Hebrew letters still read right to left");
    assert!(x(&l, 0) < x(&l, 3), "the Hebrew word stands left of the Latin one");
    // Ranges are taken in order of their start, whatever their order in the list.
    let listed_later = [Embedding { items: 4..6, rtl: true }, Embedding { items: 0..6, rtl: false }];
    let later = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, bidi: true, embeddings: &listed_later, ..Options::default() }).unwrap();
    assert_eq!(later.placed, l.placed);
    let l = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, bidi: true, ..Options::default() }).unwrap();
    assert!(x(&l, 3) < x(&l, 1), "without it, the Latin word stands left");
}

/// The test font with `alef` (500), `bet` (450), `gimel` (400), `one` and `two` (550) and
/// `acutecomb` (0, a combining mark).
fn bidi_font() -> Font {
    let mut f = font();
    f.add_glyph("alef", vec![0x5D0], 500.0).unwrap();
    f.add_glyph("bet", vec![0x5D1], 450.0).unwrap();
    f.add_glyph("gimel", vec![0x5D2], 400.0).unwrap();
    f.add_glyph("one", vec![0x31], 550.0).unwrap();
    f.add_glyph("two", vec![0x32], 550.0).unwrap();
    f.add_glyph("acutecomb", vec![0x301], 0.0).unwrap();
    kern(&mut f, Direction::Ltr, "f", "o", -40.0);
    kern(&mut f, Direction::Rtl, "f", "o", -99.0);
    f
}

#[test]
fn bidi_lines_start_at_their_edge_and_a_trailing_space_kerns_with_nothing() {
    let mut f = bidi_font();
    kern(&mut f, Direction::Ltr, "x", "space", -100.0);
    // alef ␣ b r o w n ␣ f o x ␣ | j u m p s ⏎ | bet ␣ f o x ´: the first line wraps before
    // "jumps", so the space after "fox" trails it and takes the paragraph's level.
    let t = text(&f, "\u{5d0} brown fox jumps\n\u{5d1} fox\u{301}");
    let opts = Options { direction: Direction::Rtl, bidi: true, line_width: 7000.0, line_height: Some(1000.0), ..Options::default() };
    let l = layout(&f, "m01", &t, &opts).unwrap();
    let x = |i: usize| l.placed[i].x;
    let ranges: Vec<_> = l.lines.iter().map(|line| line.items.clone()).collect();
    assert_eq!(ranges, [0..12, 12..18, 18..24]);

    assert_eq!(x(0), -500.0, "alef ends the first line at x = 0");
    let latin: Vec<f64> = (2..7).chain(8..11).map(x).collect();
    assert!(latin.windows(2).all(|p| p[0] < p[1]), "brown fox reads left to right: {latin:?}");
    assert_eq!(l.placed[11].kern, 0.0, "the trailing space stands apart from x and does not kern with it");
    assert_eq!(x(10) + 600.0, -750.0, "the Latin run ends where the space after alef begins");
    assert_eq!(l.lines[0].length, 6050.0 - 40.0);
    assert_eq!(x(11), -l.lines[0].length, "the trailing space stands at the far left");

    assert_eq!(x(16) + 600.0, 0.0, "a line of Latin letters ends at x = 0");
    let jumps: Vec<f64> = (12..17).map(x).collect();
    assert!(jumps.windows(2).all(|p| p[0] < p[1]), "jumps reads left to right: {jumps:?}");
    assert_eq!(l.placed[12].y, -1000.0);

    assert_eq!(x(18), -450.0, "bet ends the third line at x = 0");
    assert_eq!(l.placed[18].y, -2000.0);
    assert_eq!(x(20), -l.lines[2].length, "fox starts the line at its length");
    assert_eq!(x(21) - x(20), 600.0 - 40.0, "f o kerns from the left-to-right table");
    assert_eq!(x(23), x(22) + 600.0, "the combining mark follows x, not the Hebrew letter");
}

#[test]
fn bidi_trailing_space_before_a_break_and_blank_lines_measure_their_own_items() {
    let mut f = bidi_font();
    kern(&mut f, Direction::Ltr, "x", "space", -100.0);
    // alef ␣ f o x ␣ ⏎ | ⏎ | ␣ ␣ ␣ ⏎ | bet
    let t = text(&f, "\u{5d0} fox \n\n   \n\u{5d1}");
    let opts = Options { direction: Direction::Rtl, bidi: true, line_height: Some(1000.0), ..Options::default() };
    let l = layout(&f, "m01", &t, &opts).unwrap();
    let ranges: Vec<_> = l.lines.iter().map(|line| line.items.clone()).collect();
    assert_eq!(ranges, [0..7, 7..8, 8..12, 12..13]);
    assert_eq!(l.placed[5].kern, 0.0, "the space before the break does not kern with x");
    assert_eq!(l.lines[1].length, 0.0, "an empty line has no length");
    assert_eq!(l.lines[2].length, 3.0 * 250.0, "a line of blanks is as long as its spaces");
}

#[test]
fn bidi_reverses_a_right_to_left_run_in_left_to_right_text_and_off_changes_nothing() {
    let f = bidi_font();
    let t = text(&f, "ab \u{5d0}\u{5d1}\u{5d2} cd");
    let l = layout(&f, "m01", &t, &Options { bidi: true, ..Options::default() }).unwrap();
    let xs: Vec<f64> = l.placed.iter().map(|p| p.x).collect();
    // a b ␣ alef bet gimel ␣ c d: the Hebrew run reads right to left.
    assert_eq!(xs, [0.0, 600.0, 1200.0, 2300.0, 1850.0, 1450.0, 2800.0, 3050.0, 3650.0]);

    // Without bidi every item follows the direction in text order, as in Edit View.
    let t = text(&f, "\u{5d0} fox\u{301} 12");
    let ltr = layout(&f, "m01", &t, &Options::default()).unwrap();
    let xs: Vec<f64> = ltr.placed.iter().map(|p| p.x).collect();
    assert_eq!(xs, [0.0, 500.0, 750.0, 1310.0, 1910.0, 2510.0, 2510.0, 2760.0, 3310.0]);
    let rtl = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, ..Options::default() }).unwrap();
    let xs: Vec<f64> = rtl.placed.iter().map(|p| p.x).collect();
    assert_eq!(xs, [-500.0, -750.0, -1350.0, -1851.0, -2451.0, -2451.0, -2701.0, -3251.0, -3801.0]);
}

#[test]
fn bidi_orders_numbers_and_embeddings_as_the_unicode_algorithm_does() {
    let mut f = bidi_font();
    f.add_glyph("onearabic", vec![0x661], 500.0).unwrap();
    f.add_glyph("twoarabic", vec![0x662], 500.0).unwrap();
    let rtl = |t: &Text, embeddings: &[Embedding]| {
        layout(&f, "m01", t, &Options { direction: Direction::Rtl, bidi: true, embeddings, ..Options::default() }).unwrap()
    };
    // alef ␣ 1 2 ␣ 2 1 in a right-to-left paragraph reads, from the left: 21 12 alef.
    let t = text(&f, "\u{5d0} 12 21");
    let l = rtl(&t, &[]);
    let x = |i: usize| l.placed[i].x;
    assert!(x(5) < x(6) && x(6) < x(2) && x(2) < x(3) && x(3) < x(0), "{:?}", l.placed.iter().map(|p| p.x).collect::<Vec<_>>());
    assert_eq!(l.levels.as_deref(), Some(&[1, 1, 2, 2, 1, 2, 2][..]));

    // Arabic-Indic digits after a Latin word in left-to-right text stay in order, one level up.
    let t = text(&f, "ab \u{661}\u{662}");
    let l = layout(&f, "m01", &t, &Options { bidi: true, ..Options::default() }).unwrap();
    assert_eq!(l.levels.as_deref(), Some(&[0, 0, 0, 2, 2][..]));
    assert!(l.placed[3].x < l.placed[4].x);

    // An embedding reaching over a line break is closed before it and opened again after it: in
    // both lines alef stands left of c, which it does not without the embedding.
    let t = text(&f, "\u{5d0} c\n\u{5d0} c");
    let l = rtl(&t, &[Embedding { items: 0..7, rtl: false }]);
    assert!(l.placed[0].x < l.placed[2].x && l.placed[4].x < l.placed[6].x);
    assert_eq!(l.levels.as_deref(), Some(&[3, 2, 2, 1, 3, 2, 2][..]));
    let l = rtl(&t, &[]);
    assert!(l.placed[2].x < l.placed[0].x && l.placed[6].x < l.placed[4].x);
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

#[test]
fn a_picked_layer_gives_its_glyph_its_width() {
    let mut f = font();
    f.glyph_mut("A").unwrap().layers.push(fontcraft_model::Layer::new("alt", 900.0));
    let t = text(&f, "AB");
    let picked = std::collections::BTreeMap::from([("A".to_string(), "alt".to_string()), ("B".to_string(), "none".to_string())]);
    let l = layout(&f, "m01", &t, &Options { layers: Some(&picked), ..Options::default() }).unwrap();
    assert_eq!((l.placed[0].advance, l.placed[1].advance), (900.0, 600.0), "a layer the glyph lacks is ignored");
}

#[test]
fn a_prefix_reads_the_first_items_only() {
    let f = font();
    let map = Charmap::new(&f);
    let bad = format!("/{}", "n".repeat(300));
    assert_eq!(parse_prefix(&map, &format!("AB{bad}"), 2).unwrap().0.len(), 2);
    assert!(parse_prefix(&map, &format!("AB{bad}"), 2).unwrap().1);
    assert!(!parse_prefix(&map, "AB", 2).unwrap().1);
    assert!(parse_prefix(&map, &format!("A{bad}"), 2).is_err());
}

/// In right-to-left text, cursive attachment puts each glyph's `entry` anchor on the `exit`
/// anchor of the glyph before it, replacing the kerning between them, and the rises add up
/// along the chain; left-to-right text and layouts without the option are not attached.
#[test]
fn cursive_attachment_joins_entry_to_exit_in_right_to_left_text() {
    let mut f = font();
    kern(&mut f, Direction::Rtl, "a", "b", -40.0);
    for (glyph, anchors) in
        [("a", vec![("exit", 50.0, 100.0)]), ("b", vec![("entry", 550.0, 300.0), ("exit", 20.0, 120.0)]), ("c", vec![("entry", 580.0, 100.0)])]
    {
        let mut g = f.glyphs.get_mut(glyph).unwrap();
        for (name, x, y) in anchors {
            g.layers[0].anchors.push(fontcraft_model::Anchor::new(name, x, y));
        }
    }
    let t = text(&f, "abc");
    let options = Options { direction: Direction::Rtl, cursive: true, ..Options::default() };
    let l = layout(&f, "m01", &t, &options).unwrap();
    let (a, b, c) = (&l.placed[0], &l.placed[1], &l.placed[2]);
    // Each entry anchor lands on the exit anchor before it.
    assert_eq!((a.x + 50.0, a.y + 100.0), (b.x + 550.0, b.y + 300.0));
    assert_eq!((b.x + 20.0, b.y + 120.0), (c.x + 580.0, c.y + 100.0));
    assert!(b.cursive && c.cursive && !a.cursive);
    assert_eq!(b.y, -200.0, "the rise follows the anchors");
    assert_eq!(c.y, -180.0, "rises add up along the chain");

    let plain = layout(&f, "m01", &t, &Options { direction: Direction::Rtl, ..Options::default() }).unwrap();
    assert_eq!(plain.placed[1].kern, -40.0, "without the option the pair is kerned");
    let ltr = layout(&f, "m01", &t, &Options { cursive: true, ..Options::default() }).unwrap();
    assert!(ltr.placed.iter().all(|p| !p.cursive && p.y == 0.0), "left-to-right text is not attached");
}
