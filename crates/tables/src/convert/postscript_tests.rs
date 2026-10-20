//! Type 1 and bare CFF inputs, on fonts built here. The expected dumps are AFDKO 5.0.1's
//! `tx -dump -0` and `tx -dump -6` (encoding column left out) of `tx -cff` run on the same
//! Type 1 font, built byte for byte the same way.

use read_fonts::tables::glyf::Glyph;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

use super::super::cff_table::{test_charstring, test_table};
use super::super::{CFF, ConvertOptions, ConvertTarget};
use super::*;
use crate::sfnt::WriteOptions;
use fontcraft_decompile::bare_cff::read_cff;

/// Charstring programs of the test font: `subr N:` or `glyph NAME:`, Type 1 operators after
/// their operands.
const SPEC: &str = "\
subr 0: 3 0 callothersubr pop pop setcurrentpoint return
subr 1: 0 1 callothersubr return
subr 2: 0 2 callothersubr return
subr 3: return
subr 4: 300 40 hstem 30 60 vstem return
subr 5: 100 0 rlineto return
glyph .notdef: 50 500 hsbw 0 0 rmoveto 400 0 rlineto 0 700 rlineto -400 0 rlineto closepath endchar
glyph space: 0 250 hsbw endchar
glyph A: 20 600 hsbw 0 50 hstem 30 60 vstem 0 0 rmoveto 5 callsubr 0 50 rlineto 4 1 3 callothersubr pop callsubr 200 0 rlineto 0 600 rlineto closepath endchar
glyph O: 40 700 hsbw 0 0 rmoveto 0 300 rlineto 1 callsubr 100 0 rmoveto 2 callsubr -50 0 rmoveto 2 callsubr 50 20 rmoveto 2 callsubr 50 0 rmoveto 2 callsubr 50 0 rmoveto 2 callsubr 50 -20 rmoveto 2 callsubr -50 0 rmoveto 2 callsubr 50 240 300 0 callsubr 0 -300 rlineto closepath endchar
glyph acute: 30 300 hsbw 0 600 rmoveto 100 0 rlineto 50 100 rlineto -100 0 rlineto closepath endchar
glyph Aacute: 20 600 hsbw 30 150 150 65 194 seac
glyph E: 50 550 hsbw 0 50 300 40 650 50 hstem3 0 40 200 40 400 40 vstem3 700 -20 hstem 0 -21 hstem 0 0 rmoveto 400 hlineto 700 vlineto -400 hlineto closepath 50 -600 rmoveto 1000 4 div 0 rlineto 50 0 50 50 hvcurveto 50 -50 50 -50 vhcurveto closepath endchar
glyph one: 40 0 500 0 sbw 0 0 rmoveto 100 hlineto 0 700 rlineto -100 hlineto closepath dotsection endchar
glyph zzz: 10 300 hsbw 0 0 rmoveto 100 3 div 0 rlineto 0 100 rlineto closepath endchar
glyph B: 10 300 hsbw 0 0 rmoveto 100 0 rlineto 0 100 rlineto -100 0 rlineto 0 -100 rlineto closepath endchar
";

const CLEAR: &str = "%!PS-AdobeFont-1.0: PSTest-Regular 001.002\n\
11 dict begin\n\
/FontInfo 9 dict dup begin\n\
/version (001.002) readonly def\n\
/Notice (Test font for FontCraft.) readonly def\n\
/FullName (PS Test) readonly def\n\
/FamilyName (PS Test) readonly def\n\
/Weight (Regular) readonly def\n\
/ItalicAngle -12 def\n\
/isFixedPitch false def\n\
/UnderlinePosition -100 def\n\
/UnderlineThickness 50 def\n\
end readonly def\n\
/FontName /PSTest-Regular def\n\
/PaintType 0 def\n\
/FontType 1 def\n\
/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
/Encoding StandardEncoding def\n\
/FontBBox {0 -21 700 750} readonly def\n\
currentdict end\n\
currentfile eexec\n";

const PRIVATE_HEAD: &str = "dup /Private 17 dict dup begin\n\
/RD{string currentfile exch readstring pop}executeonly def\n\
/ND{noaccess def}executeonly def\n\
/NP{noaccess put}executeonly def\n\
/BlueValues [-10 0 700 710] def\n\
/OtherBlues [-250 -240] def\n\
/BlueScale 0.039625 def\n\
/BlueShift 8 def\n\
/BlueFuzz 0 def\n\
/StdHW [50] def\n\
/StdVW [60] def\n\
/StemSnapH [40 50] def\n\
/StemSnapV [40 60] def\n\
/ForceBold false def\n\
/password 5839 def\n\
/MinFeature {16 16} def\n\
/lenIV 4 def\n";

/// `tx -dump -0` of tx's CFF, from `## Top Dict` on.
const TX_DICTS: &str = "## Top Dict
version             \"001.002\"
Notice              \"Test font for FontCraft.\"
FullName            \"PS Test\"
FamilyName          \"PS Test\"
Weight              \"Regular\"
ItalicAngle         -12
FontBBox            {10,0,450,700}
sup.srcFontType     CFF (name-keyed)
sup.nGlyphs         10
## FontDict[0]
FontName            \"PSTest-Regular\"
## Private
BlueValues          {-10,0,700,710}
OtherBlues          {-250,-240}
BlueShift           8
BlueFuzz            0
StdHW               50
StdVW               60
StemSnapH           {40,50}
StemSnapV           {40,60}
";

/// `tx -dump -6` of tx's CFF, the encoding column left out.
const TX_GLYPHS: &str = "## glyph[tag] {name,encoding,path}
glyph[0] {.notdef,
  500 width
  50 0 move
  450 0 line
  450 700 line
  50 700 line
  endchar}
glyph[1] {space,
  250 width
  endchar}
glyph[2] {one,
  500 width
  40 0 move
  140 0 line
  140 700 line
  40 700 line
  endchar}
glyph[3] {A,
  600 width
  0 50 hstem
  50 110 vstem
  20 0 move
  120 0 line
  120 50 line
  newhints
  300 340 hstem
  50 110 vstem
  320 50 line
  320 650 line
  endchar}
glyph[4] {B,
  300 width
  10 0 move
  110 0 line
  110 100 line
  10 100 line
  10 0 line
  endchar}
glyph[5] {E,
  550 width
  -21 bottomedge
  0 50 hstem3
  300 340 hstem3
  650 700 hstem3
  700 topedge
  50 90 vstem3
  250 290 vstem3
  450 490 vstem3
  50 0 move
  450 0 line
  450 700 line
  50 700 line
  100 100 move
  350 100 line
  400 100 400 150 400 200 curve
  400 250 350 300 300 300 curve
  endchar}
glyph[6] {O,
  700 width
  40 0 move
  40 300 line
  90 300 140 320 190 320 240 320 290 300 240 300 50 flex
  240 0 line
  endchar}
glyph[7] {acute,
  300 width
  30 600 move
  130 600 line
  180 700 line
  80 700 line
  endchar}
glyph[8] {Aacute,
  600 width
  140 150 65 194 seac
  endchar}
glyph[9] {zzz,
  300 width
  10 0 move
  43.33 0 line
  43.33 100 line
  endchar}
";

fn t1_op(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "hstem" => &[1],
        "vstem" => &[3],
        "rlineto" => &[5],
        "hlineto" => &[6],
        "vlineto" => &[7],
        "closepath" => &[9],
        "callsubr" => &[10],
        "return" => &[11],
        "hsbw" => &[13],
        "endchar" => &[14],
        "rmoveto" => &[21],
        "vhcurveto" => &[30],
        "hvcurveto" => &[31],
        "dotsection" => &[12, 0],
        "vstem3" => &[12, 1],
        "hstem3" => &[12, 2],
        "seac" => &[12, 6],
        "sbw" => &[12, 7],
        "div" => &[12, 12],
        "callothersubr" => &[12, 16],
        "pop" => &[12, 17],
        "setcurrentpoint" => &[12, 33],
        _ => return None,
    })
}

fn num(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => {
            let v = v - 108;
            vec![((v >> 8) + 247) as u8, (v & 0xFF) as u8]
        }
        -1131..=-108 => {
            let v = -v - 108;
            vec![((v >> 8) + 251) as u8, (v & 0xFF) as u8]
        }
        _ => [vec![255], v.to_be_bytes().to_vec()].concat(),
    }
}

fn assemble(program: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for t in program.split_whitespace() {
        match t1_op(t) {
            Some(op) => out.extend_from_slice(op),
            None => out.extend(num(t.parse().unwrap())),
        }
    }
    out
}

fn encrypt(data: &[u8], mut r: u16) -> Vec<u8> {
    data.iter()
        .map(|&b| {
            let c = b ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
            c
        })
        .collect()
}

/// The clear text, the encrypted part and the trailer of the test font.
fn type1_parts() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let cs = |program: &str| encrypt(&[vec![0; 4], assemble(program)].concat(), 4330);
    let (mut subrs, mut glyphs) = (Vec::new(), Vec::new());
    for line in SPEC.lines() {
        let (head, program) = line.split_once(':').unwrap();
        let (kind, name) = head.split_once(' ').unwrap();
        if kind == "subr" { subrs.push(program) } else { glyphs.push((name, program)) }
    }
    let mut private = PRIVATE_HEAD.as_bytes().to_vec();
    private.extend(format!("/Subrs {} array\n", subrs.len()).bytes());
    for (i, program) in subrs.iter().enumerate() {
        let c = cs(program);
        private.extend(format!("dup {i} {} RD ", c.len()).bytes());
        private.extend(c);
        private.extend(b" NP\n");
    }
    private.extend(format!("ND\n2 index /CharStrings {} dict dup begin\n", glyphs.len()).bytes());
    for (name, program) in &glyphs {
        let c = cs(program);
        private.extend(format!("/{name} {} RD ", c.len()).bytes());
        private.extend(c);
        private.extend(b" ND\n");
    }
    private.extend(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
    let trailer = format!("{}cleartomark\n", format!("{}\n", "0".repeat(64)).repeat(8));
    (CLEAR.as_bytes().to_vec(), encrypt(&[vec![0; 4], private].concat(), 55665), trailer.into_bytes())
}

fn pfa_from((clear, enc, trailer): (Vec<u8>, Vec<u8>, Vec<u8>)) -> Vec<u8> {
    let hex: String = enc.iter().map(|b| format!("{b:02X}")).collect();
    let lines: Vec<String> = hex.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    [clear, lines.join("\n").into_bytes(), b"\n".to_vec(), trailer].concat()
}

fn pfb_from((clear, enc, trailer): (Vec<u8>, Vec<u8>, Vec<u8>)) -> Vec<u8> {
    let seg = |t: u8, d: &[u8]| [vec![0x80, t], (d.len() as u32).to_le_bytes().to_vec(), d.to_vec()].concat();
    [seg(1, &clear), seg(2, &enc), seg(1, &trailer), vec![0x80, 3]].concat()
}

/// A tx dump with the encoding column of each glyph line left out.
fn without_encoding(dump: &str) -> String {
    dump.lines()
        .map(|l| match (l.starts_with("glyph["), l.split_once(',')) {
            (true, Some((head, _))) => format!("{head},"),
            _ => l.to_string(),
        })
        .map(|l| l + "\n")
        .collect()
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

#[test]
fn a_type1_font_becomes_the_cff_table_tx_writes() {
    for (form, bytes) in [("pfa", pfa_from(type1_parts())), ("pfb", pfb_from(type1_parts()))] {
        assert_eq!(PsSource::sniff(&bytes), Some(PsSource::Type1), "{form}");
        let input = read_postscript(&bytes).unwrap_or_else(|e| panic!("{form}: {e}"));
        assert!(!input.notes.iter().any(|n| n.contains("hints") || n.contains("accented")), "{form}: {:?}", input.notes);
        let cff = input.font.get(CFF).unwrap();
        let ps = read_cff(cff, &mut Vec::new()).unwrap();
        assert_eq!(ps.dump_dicts(), TX_DICTS, "{form}: dictionaries");
        assert_eq!(without_encoding(&ps.dump_glyphs()), TX_GLYPHS, "{form}: glyphs, hints and seac");

        // The sfnt around it: metrics, cmap from the names, post 3.
        let f = &input.font;
        assert_eq!(f.flavour, Flavour::Cff);
        assert_eq!(u16_at(f.get(*b"head").unwrap(), 18), 1000);
        assert_eq!(u16_at(f.get(*b"maxp").unwrap(), 4), 10);
        let hmtx = f.get(*b"hmtx").unwrap();
        // A (glyph 3): advance 600, left side bearing 20; Aacute (glyph 8) reaches the accent.
        assert_eq!((u16_at(hmtx, 12), u16_at(hmtx, 14)), (600, 20));
        assert_eq!(u16_at(f.get(*b"head").unwrap(), 42) as i16, 850, "{form}: yMax includes the accent of Aacute");
        assert_eq!(&f.get(*b"post").unwrap()[..4], &[0, 3, 0, 0]);
        let bytes = crate::write(f, &WriteOptions::default()).unwrap();
        let font = FontRef::new(&bytes).unwrap();
        let cmap = font.cmap().unwrap();
        assert_eq!(cmap.map_codepoint('A'), Some(GlyphId::new(3)));
        assert_eq!(cmap.map_codepoint('\u{c1}'), Some(GlyphId::new(8)), "Aacute");
    }
}

#[test]
fn a_type1_font_converts_to_every_target() {
    let input = read_postscript(&pfa_from(type1_parts())).unwrap();
    // TrueType: the seac glyph becomes a composite of A and acute.
    let ttf = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 })).unwrap();
    assert_eq!(ttf.glyphs, 10);
    let font = FontRef::new(&ttf.bytes).unwrap();
    let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
    match loca.get_glyf(GlyphId::new(8), &glyf).unwrap() {
        Some(Glyph::Composite(c)) => assert_eq!(c.components().map(|c| u32::from(c.glyph.to_u16())).collect::<Vec<_>>(), vec![3, 7]),
        _ => panic!("Aacute is not a composite"),
    }
    assert!(matches!(loca.get_glyf(GlyphId::new(6), &glyf).unwrap(), Some(Glyph::Simple(_))), "O, drawn from its flex");

    // OpenType CFF: subroutinised by default, the same outlines.
    let otf = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::Cff)).unwrap();
    let out = otf.font.unwrap();
    let flat = fontcraft_cff::subr::desubroutinize_table(out.get(CFF).unwrap(), false).unwrap();
    assert_eq!(without_encoding(&read_cff(&flat, &mut Vec::new()).unwrap().dump_glyphs()), TX_GLYPHS);

    // Bare CFF as tx writes it; bare CFF2 with the seac glyph drawn out.
    let cff = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::BareCff)).unwrap();
    assert_eq!(without_encoding(&read_cff(&cff.bytes, &mut Vec::new()).unwrap().dump_glyphs()), TX_GLYPHS);
    let cff2 = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::BareCff2)).unwrap();
    assert_eq!(&cff2.bytes[..2], &[2, 0]);
    assert_eq!(cff2.glyphs, 10);
}

#[test]
fn a_bare_cff_table_is_kept_and_wrapped() {
    // .notdef (empty), A (a square, advance 600) and B (a triangle, advance 500).
    let charstrings = vec![
        test_charstring(&[(&[0.0], 14)]),
        test_charstring(&[(&[600.0, 50.0, 0.0], 21), (&[500.0, 700.0, -500.0], 6), (&[], 14)]),
        test_charstring(&[(&[500.0, 0.0, 0.0], 21), (&[400.0, 0.0, -200.0, 300.0], 5), (&[], 14)]),
    ];
    let table = test_table(&[".notdef", "A", "B"], charstrings, &[]);
    assert_eq!(PsSource::sniff(&table), Some(PsSource::BareCff));
    let input = read_postscript(&table).unwrap();
    assert_eq!(input.font.get(CFF), Some(table.as_slice()), "the table is kept as it is");
    let hmtx = input.font.get(*b"hmtx").unwrap();
    assert_eq!([u16_at(hmtx, 4), u16_at(hmtx, 6), u16_at(hmtx, 8), u16_at(hmtx, 10)], [600, 50, 500, 0]);

    let ttf = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::TrueType { max_error: 1.0 })).unwrap();
    let font = FontRef::new(&ttf.bytes).unwrap();
    assert_eq!(font.maxp().unwrap().num_glyphs(), 3);
    assert_eq!(font.hmtx().unwrap().advance(GlyphId::new(1)), Some(600));
    assert_eq!(font.cmap().unwrap().map_codepoint('B'), Some(GlyphId::new(2)));
    let bare = convert_postscript(&input, &ConvertOptions::new(ConvertTarget::BareCff)).unwrap();
    let ps = read_cff(&bare.bytes, &mut Vec::new()).unwrap();
    assert_eq!(ps.glyphs.iter().map(|g| (g.name.as_str(), g.width)).collect::<Vec<_>>(), vec![(".notdef", 0.0), ("A", 600.0), ("B", 500.0)]);
}

#[test]
fn damaged_font_programs_are_refused_without_a_crash() {
    let parts = type1_parts();
    let pfb = pfb_from(parts.clone());
    // A segment cut short, and one that declares far more bytes than the file holds.
    assert!(read_postscript(&pfb[..pfb.len() / 2]).is_err());
    let mut huge = pfb.clone();
    huge[2..6].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_postscript(&huge).is_err());
    assert!(read_postscript(&[0x80, 0x01, 0xFF]).is_err());
    // An eexec section that does not decrypt to a private dictionary.
    let (clear, enc, trailer) = parts.clone();
    let garbage: Vec<u8> = enc.iter().map(|b| b.wrapping_mul(31).wrapping_add(7)).collect();
    assert!(read_postscript(&pfa_from((clear.clone(), garbage, trailer.clone()))).is_err());
    assert!(read_postscript(&[clear, b"zz".to_vec()].concat()).is_err());
    // Every prefix of the font is read or refused, never a crash.
    let pfa = pfa_from(parts);
    for end in (0..pfa.len()).step_by(41) {
        let _ = read_postscript(&pfa[..end]);
    }
    // A CFF font set of two fonts.
    let mut two = test_table(&[".notdef"], vec![test_charstring(&[(&[0.0], 14)])], &[]);
    two[4..6].copy_from_slice(&2u16.to_be_bytes());
    assert!(matches!(read_postscript(&two), Err(TablesError::Unsupported(_))));
}
