//! Hint removal on fonts built here. The CFF expectations are the charstrings pyftsubset
//! (`--no-hinting --desubroutinize`) wrote for the same programs; the TrueType ones follow the
//! `glyf` and `maxp` rules of the same tool.

use std::collections::BTreeSet;

use super::cff::build::{charstring, index, private_ops, programs};
use super::*;
use crate::sfnt::Flavour;

fn dict_int(v: i32) -> Vec<u8> {
    let mut b = vec![29];
    b.extend_from_slice(&v.to_be_bytes());
    b
}

/// A name-keyed CFF table with stems, hint masks, flex, one local and one global subroutine,
/// and a Private DICT with BlueValues, StdVW and defaultWidthX.
fn cff_table() -> Vec<u8> {
    cff_table_with_subr("0 50 hstemhm return")
}

fn cff_table_with_subr(local_subr: &str) -> Vec<u8> {
    let glyphs: Vec<Vec<u8>> = [
        "300 endchar",
        "600 0 100 hstem 50 80 vstem 10 20 rmoveto hintmask <c0> 100 0 rlineto 0 100 rlineto -100 0 rlineto endchar",
        "-107 callsubr 30 40 hintmask <c0> 0 0 rmoveto 10 0 10 10 10 0 10 0 10 -10 10 0 50 flex -107 callgsubr endchar",
        "700 50 hmoveto 20 30 10 20 30 20 30 hflex 0 -10 rlineto endchar",
    ]
    .iter()
    .map(|p| charstring(p))
    .collect();
    let gsubrs = index(&[charstring("0 100 rlineto return")], false);
    let lsubrs = index(&[charstring(local_subr)], false);
    let names = index(&[b"Test".to_vec()], false);
    let strings = index(&[], false);
    let charstrings = index(&glyphs, false);

    let mut private = Vec::new();
    for v in [-10, 10, 490, 10] {
        private.extend(dict_int(v));
    }
    private.push(6);
    private.extend(dict_int(80));
    private.push(11);
    private.extend(dict_int(500));
    private.push(20);
    let subrs_at = private.len() + 6;
    private.extend(dict_int(subrs_at as i32));
    private.push(19);

    let top_len = 17;
    let top_index_len = 2 + 1 + 2 + top_len;
    let cs_at = 4 + names.len() + top_index_len + strings.len() + gsubrs.len();
    let private_at = cs_at + charstrings.len();
    let mut top = dict_int(cs_at as i32);
    top.push(17);
    top.extend(dict_int(private.len() as i32));
    top.extend(dict_int(private_at as i32));
    top.push(18);
    assert_eq!(top.len(), top_len);

    let mut t = vec![1, 0, 4, 4];
    t.extend(names);
    t.extend(index(&[top], false));
    t.extend(strings);
    t.extend(gsubrs);
    assert_eq!(t.len(), cs_at);
    t.extend(charstrings);
    t.extend(private);
    t.extend(lsubrs);
    t
}

fn cff_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", cff_table());
    font
}

#[test]
fn cff_hints_go_like_the_oracle_and_flex_becomes_curves() {
    let font = cff_font();
    assert_eq!(
        programs(font.get(*b"CFF ").unwrap())[2],
        "-107 callsubr 30 40 hintmask <c0> 0 0 rmoveto 10 0 10 10 10 0 10 0 10 -10 10 0 50 flex -107 callgsubr endchar",
        "the test assembler and disassembler agree"
    );

    let kept_flex = DehintOptions { keep_flex: true, ..DehintOptions::default() };
    let (out, report) = dehint_with_report(&font, &kept_flex).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    assert_eq!(
        programs(table),
        [
            "300 endchar",
            "600 10 20 rmoveto 100 0 rlineto 0 100 rlineto -100 0 rlineto endchar",
            "0 0 rmoveto 10 0 10 10 10 0 10 0 10 -10 10 0 50 flex 0 100 rlineto endchar",
            "700 50 hmoveto 20 30 10 20 30 20 30 hflex 0 -10 rlineto endchar",
        ],
        "pyftsubset --no-hinting --desubroutinize gives these programs"
    );
    assert_eq!(private_ops(table), vec![vec![20]], "only defaultWidthX stays in the Private DICT");
    assert_eq!(report.glyphs_changed, 2);
    assert_eq!(report.tables_changed, vec![*b"CFF "]);

    let (out, report) = dehint_with_report(&font, &DehintOptions::default()).unwrap();
    let programs = programs(out.get(*b"CFF ").unwrap());
    assert_eq!(programs[2], "0 0 rmoveto 10 0 10 10 10 0 10 0 10 -10 10 0 rrcurveto 0 100 rlineto endchar");
    assert_eq!(programs[3], "700 50 hmoveto 20 0 30 10 20 0 30 0 20 -10 30 0 rrcurveto 0 -10 rlineto endchar");
    assert_eq!(report.glyphs_changed, 3, "flex removal counts as a change");
}

#[test]
fn cff_selection_and_kept_font_hints() {
    let font = cff_font();
    let options = DehintOptions { glyphs: Some(BTreeSet::from([1])), ..DehintOptions::default() };
    let out = dehint(&font, &options).unwrap();
    let table = out.get(*b"CFF ").unwrap();
    let p = programs(table);
    assert_eq!(p[1], "600 10 20 rmoveto 100 0 rlineto 0 100 rlineto -100 0 rlineto endchar");
    assert_eq!(
        p[2], "0 50 hstemhm 30 40 hintmask <c0> 0 0 rmoveto 10 0 10 10 10 0 10 0 10 -10 10 0 50 flex 0 100 rlineto endchar",
        "an unselected glyph keeps its hints, written without subroutines"
    );
    assert_eq!(private_ops(table), vec![vec![6, 11, 20]], "a selection keeps the Private DICT values");

    let keep = DehintOptions { keep_font_hints: true, ..DehintOptions::default() };
    let out = dehint(&font, &keep).unwrap();
    assert_eq!(private_ops(out.get(*b"CFF ").unwrap()), vec![vec![6, 11, 20]]);
}

#[test]
fn hostile_cff_is_refused() {
    let mut table = cff_table();
    // The CharStrings offset (first Top DICT operand) pointing past the end.
    let at = 4 + index(&[b"Test".to_vec()], false).len() + 5;
    table[at..at + 5].copy_from_slice(&dict_int(0x7FFF_FFF0));
    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", table);
    assert!(dehint(&font, &DehintOptions::default()).is_err());

    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", cff_table_with_subr("-107 callsubr return"));
    let e = dehint(&font, &DehintOptions::default()).unwrap_err();
    assert!(e.to_string().contains("nest"), "a subroutine that calls itself: {e}");

    let mut font = SfntFont::new(Flavour::Cff);
    font.insert(*b"CFF ", vec![1, 0, 4]);
    assert!(dehint(&font, &DehintOptions::default()).is_err(), "a cut header");
}

#[test]
fn font_wide_cff_budgets_refuse_before_the_table_is_built() {
    // A local subroutine of 2,000 path operators, called by one glyph.
    let subr = format!("0 50 hstemhm{} return", " 0 0 rlineto".repeat(2_000));
    let table = cff_table_with_subr(&subr);
    let options = DehintOptions::default();
    assert!(cff::dehint_within(&table, false, &options, 1 << 20, 1 << 20).unwrap().is_some());

    let e = cff::dehint_within(&table, false, &options, 5_000, 1 << 20).unwrap_err();
    assert!(e.to_string().contains("operators all glyphs run"), "{e}");
    let e = cff::dehint_within(&table, false, &options, 1 << 20, 4_000).unwrap_err();
    assert!(e.to_string().contains("memory the charstrings of one table take"), "{e}");
}

#[test]
fn nested_subroutine_expansion_counts_against_the_charstring_memory() {
    // Glyph 2 calls a local subroutine that calls the global `0 100 rlineto` 3,000 times, a
    // program of about 6 KB that writes about 9 KB once flat, and then overflows the stack.
    let subr = format!("0 50 hstemhm{}{} return", " -107 callgsubr".repeat(3_000), " 1".repeat(60));
    let table = cff_table_with_subr(&subr);
    let options = DehintOptions::default();
    let e = cff::dehint_within(&table, false, &options, 1 << 20, 1 << 20).unwrap_err();
    assert!(e.to_string().contains("operands on the stack"), "within a larger budget the glyph fails at its end: {e}");

    // Charged only after a glyph is done, the budget would never see this one: the glyph fails
    // first. Charged per written operator, it is refused while the glyph is written.
    let e = cff::dehint_within(&table, false, &options, 1 << 20, 8_000).unwrap_err();
    assert!(e.to_string().contains("memory the charstrings of one table take"), "{e}");
}

#[test]
fn shared_gpos_subtables_are_refused_before_they_are_copied() {
    use crate::layout_budget::tests::{MAX_SHARED, shared_gpos};
    let mut font = cff_font();
    font.insert(*b"GPOS", shared_gpos(2, 2, 3));
    let (out, _) = dehint_with_report(&font, &DehintOptions::default()).unwrap();
    assert_eq!(out.get(*b"GPOS"), font.get(*b"GPOS"), "a GPOS table without device tables stays as it is");

    font.insert(*b"GPOS", shared_gpos(MAX_SHARED, MAX_SHARED, 3));
    let e = dehint(&font, &DehintOptions::default()).unwrap_err();
    assert!(e.to_string().contains("too large"), "{e}");
}

// ---------- TrueType ----------

fn be16(v: i32) -> [u8; 2] {
    (v as i16).to_be_bytes()
}

/// A simple glyph: one contour of three on-curve points, with a two-byte program.
fn simple_glyph() -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1, 0, 0, 100, 100, 2] {
        g.extend(be16(v));
    }
    g.extend(be16(2));
    g.extend([0xB0, 0x01]);
    g.extend([1, 1, 1]);
    for v in [0, 100, 0, 0, 0, 100] {
        g.extend(be16(v));
    }
    g
}

/// A composite of glyph 1 with a three-byte program.
fn composite_glyph() -> Vec<u8> {
    let mut g = Vec::new();
    for v in [-1, 0, 0, 100, 100] {
        g.extend(be16(v));
    }
    g.extend(be16(0x0102));
    g.extend(be16(1));
    g.extend([10, 0]);
    g.extend(be16(3));
    g.extend([1, 2, 3]);
    g
}

fn tt_font() -> SfntFont {
    tt_font_of([Vec::new(), simple_glyph(), composite_glyph()])
}

/// A short-loca TrueType font of three glyphs, each padded to an even length.
fn tt_font_of(glyphs: [Vec<u8>; 3]) -> SfntFont {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in &glyphs {
        loca.extend(be16((glyf.len() / 2) as i32));
        glyf.extend(g);
        if glyf.len() % 2 == 1 {
            glyf.push(0);
        }
    }
    loca.extend(be16((glyf.len() / 2) as i32));
    let mut head = vec![0u8; 54];
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[16..18].copy_from_slice(&0x001Bu16.to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&3u16.to_be_bytes());
    for (at, v) in [(14, 2u16), (16, 4), (18, 8), (20, 10), (22, 1), (24, 200), (26, 3)] {
        maxp[at..at + 2].copy_from_slice(&v.to_be_bytes());
    }
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp);
    font.insert(*b"loca", loca);
    font.insert(*b"glyf", glyf);
    for tag in [*b"fpgm", *b"prep", *b"cvt ", *b"hdmx", *b"VDMX", *b"LTSH", *b"name"] {
        font.insert(tag, vec![1, 2, 3, 4]);
    }
    font.insert(*b"gasp", vec![0, 1, 0, 1, 0, 8, 0, 2]);
    font
}

fn glyph(font: &SfntFont, gid: usize) -> Vec<u8> {
    let loca = font.get(*b"loca").unwrap();
    let long = font.get(*b"head").unwrap()[51] == 1;
    let offsets = read_loca(loca, long, 3).unwrap();
    font.get(*b"glyf").unwrap()[offsets[gid]..offsets[gid + 1]].to_vec()
}

#[test]
fn truetype_instructions_and_hinting_tables_go() {
    let font = tt_font();
    let (out, report) = dehint_with_report(&font, &DehintOptions::default()).unwrap();

    let simple = glyph(&out, 1);
    let original = simple_glyph();
    assert_eq!(&simple[..12], &original[..12], "header and end points unchanged");
    assert_eq!(&simple[12..14], &[0, 0], "no instructions");
    assert_eq!(&simple[14..29], &original[16..31], "flags and coordinates unchanged");

    let composite = glyph(&out, 2);
    assert_eq!(&composite[..16], &[0xFF, 0xFF, 0, 0, 0, 0, 0, 100, 0, 100, 0x00, 0x02, 0, 1, 10, 0]);
    assert!(composite[16..].iter().all(|b| *b == 0), "only padding after the component");
    assert!(glyph(&out, 0).is_empty());

    for tag in [*b"fpgm", *b"prep", *b"cvt ", *b"hdmx", *b"VDMX", *b"LTSH"] {
        assert!(out.get(tag).is_none(), "{} is deleted", crate::sfnt::tag_str(tag));
    }
    assert_eq!(out.get(*b"name"), font.get(*b"name"), "untouched tables pass through");
    assert_eq!(out.get(*b"gasp"), font.get(*b"gasp"), "gasp is kept by default");
    let maxp = out.get(*b"maxp").unwrap();
    assert_eq!(&maxp[14..28], &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], "maxp limits reset as pyftsubset resets them");
    assert_eq!(&maxp[28..32], &font.get(*b"maxp").unwrap()[28..32]);
    assert_eq!(&out.get(*b"head").unwrap()[16..18], &0x000Bu16.to_be_bytes(), "head flags 2 and 4 cleared");
    assert_eq!(report.glyphs_changed, 2);
    assert_eq!(report.tables_removed, vec![*b"LTSH", *b"VDMX", *b"cvt ", *b"fpgm", *b"hdmx", *b"prep"]);
    assert_eq!(report.tables_changed, vec![*b"glyf", *b"head", *b"loca", *b"maxp"]);

    let again = dehint(&out, &DehintOptions::default()).unwrap();
    assert_eq!(again, out, "a second run changes nothing");
}

#[test]
fn truetype_selection_keep_and_gasp() {
    let font = tt_font();
    let only_one = DehintOptions { glyphs: Some(BTreeSet::from([1])), ..DehintOptions::default() };
    let out = dehint(&font, &only_one).unwrap();
    assert_eq!(glyph(&out, 2), glyph(&font, 2), "an unselected glyph keeps its program");
    assert_eq!(&glyph(&out, 1)[12..14], &[0, 0]);
    assert_eq!(out.get(*b"fpgm"), font.get(*b"fpgm"), "a selection keeps font-wide data");
    assert_eq!(out.get(*b"maxp"), font.get(*b"maxp"));

    let keep = DehintOptions { keep_font_hints: true, ..DehintOptions::default() };
    let out = dehint(&font, &keep).unwrap();
    for tag in [*b"fpgm", *b"prep", *b"cvt "] {
        assert!(out.get(tag).is_some());
    }
    assert!(out.get(*b"hdmx").is_none() && out.get(*b"LTSH").is_none());
    assert_eq!(out.get(*b"maxp"), font.get(*b"maxp"), "the maxp limits stay with the font programs");

    let tables = DehintOptions { hinting_tables: vec![*b"VDMX"], ..DehintOptions::default() };
    let out = dehint(&font, &tables).unwrap();
    assert!(out.get(*b"VDMX").is_none() && out.get(*b"fpgm").is_some());

    let smooth = DehintOptions { gasp: GaspMode::Smooth, ..DehintOptions::default() };
    assert_eq!(dehint(&font, &smooth).unwrap().get(*b"gasp"), Some([0u8, 1, 0, 1, 0xFF, 0xFF, 0, 0x0F].as_slice()));
    let remove = DehintOptions { gasp: GaspMode::Remove, ..DehintOptions::default() };
    assert!(dehint(&font, &remove).unwrap().get(*b"gasp").is_none());
    assert!(GaspMode::parse("bogus").is_err());
}

#[test]
fn hostile_glyf_is_refused() {
    let mut font = tt_font();
    let mut glyf = font.get(*b"glyf").unwrap().to_vec();
    // The simple glyph claims a program longer than the glyph.
    glyf[12..14].copy_from_slice(&be16(4000));
    font.insert(*b"glyf", glyf);
    assert!(dehint(&font, &DehintOptions::default()).is_err());

    let mut font = tt_font();
    font.insert(*b"loca", vec![0, 0, 0, 40]);
    assert!(dehint(&font, &DehintOptions::default()).is_err(), "loca shorter than numGlyphs");
}

#[test]
fn input_padding_after_a_glyph_is_not_copied() {
    // Long-loca inputs pad each glyph to four bytes; the merge left 4 to 6 such bytes.
    let mut simple = simple_glyph();
    simple.extend([0; 5]);
    let mut composite = composite_glyph();
    composite.extend([0; 6]);
    let font = tt_font_of([Vec::new(), simple, composite]);
    let out = dehint(&font, &DehintOptions::default()).unwrap();

    // Data ends: the simple glyph at 29 bytes (31 less its two-byte program), the composite
    // after its one component (16 bytes). Only the table's own two-byte alignment follows.
    let simple = glyph(&out, 1);
    assert_eq!(simple.len(), 30);
    assert_eq!(&simple[14..29], &simple_glyph()[16..31]);
    assert_eq!(simple[29], 0);
    let composite = glyph(&out, 2);
    assert_eq!(composite.len(), 16);
    assert_eq!(composite, glyph(&dehint(&tt_font(), &DehintOptions::default()).unwrap(), 2));
    assert_eq!(out.get(*b"glyf").unwrap().len(), 46);
}

#[test]
fn truncated_glyph_data_is_refused() {
    let mut simple = simple_glyph();
    simple.truncate(simple.len() - 4);
    let font = tt_font_of([Vec::new(), simple, composite_glyph()]);
    assert!(dehint(&font, &DehintOptions::default()).is_err(), "coordinates cut short");

    let mut simple = simple_glyph();
    // The last end point claims 201 points; the flags run out first.
    simple[10..12].copy_from_slice(&be16(200));
    let font = tt_font_of([Vec::new(), simple, composite_glyph()]);
    assert!(dehint(&font, &DehintOptions::default()).is_err(), "flags cut short");

    let mut composite = composite_glyph();
    composite.truncate(composite.len() - 2);
    let font = tt_font_of([Vec::new(), simple_glyph(), composite]);
    assert!(dehint(&font, &DehintOptions::default()).is_err(), "program cut short");
}
