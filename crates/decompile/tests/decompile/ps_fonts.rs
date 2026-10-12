//! Type 1 and bare CFF reading (formats `type1`, `cff`), compared with AFDKO tx.
//!
//! The test fonts are built here from the charstring programs in `data/ps_type1.txt` and
//! `data/ps_cff.txt`. The expected dumps in `data/*.dicts.txt`, `data/*.glyphs.txt` and
//! `data/corpus_*.txt` are the output of `tx -dump -0` (without its `## Filename` line) and
//! `tx -dump -6` (AFDKO 5.0.1) on the same fonts, built byte for byte the same way, and on the
//! corpus files.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use std::path::Path;

use fontcraft_decompile::bare_cff::read_cff;
use fontcraft_decompile::decompile;
use fontcraft_decompile::type1::read_type1;
use fontcraft_model::{NodeKind, Shape, Transform};

const T1_SPEC: &str = include_str!("data/ps_type1.txt");
const CFF_SPEC: &str = include_str!("data/ps_cff.txt");

fn t1_op(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "hstem" => &[1],
        "vstem" => &[3],
        "vmoveto" => &[4],
        "rlineto" => &[5],
        "hlineto" => &[6],
        "vlineto" => &[7],
        "rrcurveto" => &[8],
        "closepath" => &[9],
        "callsubr" => &[10],
        "return" => &[11],
        "hsbw" => &[13],
        "endchar" => &[14],
        "rmoveto" => &[21],
        "hmoveto" => &[22],
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

fn t2_op(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "hstem" => &[1],
        "vstem" => &[3],
        "vmoveto" => &[4],
        "rlineto" => &[5],
        "hlineto" => &[6],
        "vlineto" => &[7],
        "rrcurveto" => &[8],
        "callsubr" => &[10],
        "return" => &[11],
        "endchar" => &[14],
        "hstemhm" => &[18],
        "hintmask" => &[19],
        "cntrmask" => &[20],
        "rmoveto" => &[21],
        "hmoveto" => &[22],
        "vstemhm" => &[23],
        "rcurveline" => &[24],
        "rlinecurve" => &[25],
        "vvcurveto" => &[26],
        "hhcurveto" => &[27],
        "callgsubr" => &[29],
        "vhcurveto" => &[30],
        "hvcurveto" => &[31],
        "hflex" => &[12, 34],
        "flex" => &[12, 35],
        "hflex1" => &[12, 36],
        "flex1" => &[12, 37],
        "mul" => &[12, 24],
        _ => return None,
    })
}

fn num(v: i32, type2: bool) -> Vec<u8> {
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
        _ if type2 => [vec![28], (v as i16).to_be_bytes().to_vec()].concat(),
        _ => [vec![255], v.to_be_bytes().to_vec()].concat(),
    }
}

fn assemble(tokens: &[&str], type2: bool) -> Vec<u8> {
    let op = if type2 { t2_op } else { t1_op };
    let mut out = Vec::new();
    for t in tokens {
        if let Some((name, mask)) = t.split_once(':') {
            out.extend_from_slice(op(name).unwrap());
            out.extend((0..mask.len()).step_by(2).map(|i| u8::from_str_radix(&mask[i..i + 2], 16).unwrap()));
        } else if let Some(bytes) = op(t) {
            out.extend_from_slice(bytes);
        } else {
            out.extend(num(t.parse().unwrap(), type2));
        }
    }
    out
}

struct Spec<'a> {
    subrs: Vec<Vec<&'a str>>,
    gsubrs: Vec<Vec<&'a str>>,
    glyphs: Vec<(&'a str, Vec<&'a str>)>,
}

fn spec(text: &str) -> Spec<'_> {
    let mut s = Spec { subrs: Vec::new(), gsubrs: Vec::new(), glyphs: Vec::new() };
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (head, prog) = line.split_once(':').unwrap();
        let (kind, name) = head.split_once(' ').unwrap();
        let toks: Vec<&str> = prog.split_whitespace().collect();
        match kind {
            // Subroutines are listed in order, from 0.
            "subr" => s.subrs.push(toks),
            "gsubr" => s.gsubrs.push(toks),
            _ => s.glyphs.push((name, toks)),
        }
    }
    s
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

/// The clear text, the encrypted part and the trailer of the Type 1 test font.
fn type1_parts() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    type1_parts_with("ND\n2 index /CharStrings")
}

/// [`type1_parts`] with `intro` between the last subroutine and the CharStrings count.
fn type1_parts_with(intro: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let s = spec(T1_SPEC);
    let cs = |toks: &[&str]| encrypt(&[vec![0; 4], assemble(toks, false)].concat(), 4330);
    let mut private = PRIVATE_HEAD.as_bytes().to_vec();
    private.extend(format!("/Subrs {} array\n", s.subrs.len()).bytes());
    for (i, toks) in s.subrs.iter().enumerate() {
        let c = cs(toks);
        private.extend(format!("dup {i} {} RD ", c.len()).bytes());
        private.extend(c);
        private.extend(b" NP\n");
    }
    private.extend(format!("{intro} {} dict dup begin\n", s.glyphs.len()).bytes());
    for (name, toks) in &s.glyphs {
        let c = cs(toks);
        private.extend(format!("/{name} {} RD ", c.len()).bytes());
        private.extend(c);
        private.extend(b" ND\n");
    }
    private.extend(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
    let trailer = format!("{}cleartomark\n", format!("{}\n", "0".repeat(64)).repeat(8));
    (CLEAR.as_bytes().to_vec(), encrypt(&[vec![0; 4], private].concat(), 55665), trailer.into_bytes())
}

fn pfa() -> Vec<u8> {
    let (clear, enc, trailer) = type1_parts();
    let hex: String = enc.iter().map(|b| format!("{b:02X}")).collect();
    let lines: Vec<&str> = (0..hex.len()).step_by(64).map(|i| &hex[i..(i + 64).min(hex.len())]).collect();
    [clear, lines.join("\n").into_bytes(), b"\n".to_vec(), trailer].concat()
}

fn pfb() -> Vec<u8> {
    pfb_from(type1_parts())
}

fn pfb_from((clear, enc, trailer): (Vec<u8>, Vec<u8>, Vec<u8>)) -> Vec<u8> {
    let seg = |t: u8, d: &[u8]| [vec![0x80, t], (d.len() as u32).to_le_bytes().to_vec(), d.to_vec()].concat();
    [seg(1, &clear), seg(2, &enc), seg(1, &trailer), vec![0x80, 3]].concat()
}

fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    if items.is_empty() {
        return vec![0, 0];
    }
    let mut offs = vec![1usize];
    for it in items {
        offs.push(offs.last().unwrap() + it.len());
    }
    let last = *offs.last().unwrap();
    let size = if last < 0x100 {
        1
    } else if last < 0x10000 {
        2
    } else {
        4
    };
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    out.push(size as u8);
    for o in offs {
        out.extend_from_slice(&(o as u32).to_be_bytes()[4 - size..]);
    }
    for it in items {
        out.extend_from_slice(it);
    }
    out
}

fn dint(v: i32) -> Vec<u8> {
    match v {
        -1131..=1131 => num(v, true),
        _ => [vec![28], (v as i16).to_be_bytes().to_vec()].concat(),
    }
}

fn dint5(v: i32) -> Vec<u8> {
    [vec![29], v.to_be_bytes().to_vec()].concat()
}

/// A real in the DICT's nibble form.
fn dreal(text: &str) -> Vec<u8> {
    let mut nib: Vec<u8> = text
        .chars()
        .map(|c| match c {
            '.' => 0xA,
            '-' => 0xE,
            d => d.to_digit(10).unwrap() as u8,
        })
        .collect();
    nib.push(0xF);
    if nib.len() % 2 == 1 {
        nib.push(0xF);
    }
    [vec![30], nib.chunks(2).map(|p| p[0] << 4 | p[1]).collect()].concat()
}

fn cff() -> Vec<u8> {
    cff_from(CFF_SPEC)
}

fn cff_from(text: &str) -> Vec<u8> {
    let s = spec(text);
    let mut strings: Vec<String> = ["001.002", "Test font for FontCraft.", "PS Test", "PS Test", "Regular"].map(String::from).to_vec();
    let sid = |i: usize| 391 + i as i32;
    let names: Vec<&str> = s.glyphs.iter().skip(1).map(|(n, _)| *n).collect();
    let name_sids: Vec<i32> = (0..names.len()).map(|i| sid(strings.len() + i)).collect();
    strings.extend(names.iter().map(|n| n.to_string()));
    let mut charset = vec![0u8];
    for v in &name_sids {
        charset.extend((*v as u16).to_be_bytes());
    }
    let charstrings = cff_index(&s.glyphs.iter().map(|(_, t)| assemble(t, true)).collect::<Vec<_>>());
    let local = cff_index(&s.subrs.iter().map(|t| assemble(t, true)).collect::<Vec<_>>());
    let mut private = [
        dint(-10),
        dint(10),
        dint(700),
        dint(10),
        vec![6],
        dint(-250),
        dint(10),
        vec![7],
        dreal("0.039625"),
        vec![12, 9],
        dint(8),
        vec![12, 10],
        dint(0),
        vec![12, 11],
        dint(50),
        vec![10],
        dint(60),
        vec![11],
        dint(40),
        dint(10),
        vec![12, 12],
        dint(40),
        dint(20),
        vec![12, 13],
        dint(250),
        vec![20],
        dint(400),
        vec![21],
    ]
    .concat();
    let subrs_at = private.len() as i32 + 6;
    private.extend(dint5(subrs_at));
    private.push(19);
    let top = |charset_at: i32, cs_at: i32, private_at: i32| {
        [
            dint(sid(0)),
            vec![0],
            dint(sid(1)),
            vec![1],
            dint(sid(2)),
            vec![2],
            dint(sid(3)),
            vec![3],
            dint(sid(4)),
            vec![4],
            dint(-12),
            vec![12, 2],
            dint(-100),
            vec![12, 3],
            dint(50),
            vec![12, 4],
            dint(0),
            dint(-21),
            dint(700),
            dint(750),
            vec![5],
            dint5(charset_at),
            vec![15],
            dint5(cs_at),
            vec![17],
            dint5(private.len() as i32),
            dint5(private_at),
            vec![18],
        ]
        .concat()
    };
    let header = vec![1, 0, 4, 4];
    let name_index = cff_index(&[b"PSTest-Regular".to_vec()]);
    let string_index = cff_index(&strings.iter().map(|s| s.as_bytes().to_vec()).collect::<Vec<_>>());
    let gsubr_index = cff_index(&s.gsubrs.iter().map(|t| assemble(t, true)).collect::<Vec<_>>());
    let top_len = cff_index(&[top(0, 0, 0)]).len();
    let charset_at = header.len() + name_index.len() + top_len + string_index.len() + gsubr_index.len();
    let cs_at = charset_at + charset.len();
    let private_at = cs_at + charstrings.len();
    let top_index = cff_index(&[top(charset_at as i32, cs_at as i32, private_at as i32)]);
    assert_eq!(top_index.len(), top_len);
    [header, name_index, top_index, string_index, gsubr_index, charset, charstrings, private, local].concat()
}

fn expected(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap()
}

#[test]
fn type1_pfa_and_pfb_read_as_tx_dumps_them() {
    for (form, bytes) in [("pfa", pfa()), ("pfb", pfb())] {
        let mut warnings = Vec::new();
        let ps = read_type1(&bytes, &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{form}: {warnings:?}");
        assert_eq!(ps.dump_dicts(), expected("ps_type1.dicts.txt"), "{form}: dictionaries");
        assert_eq!(ps.dump_glyphs(), expected("ps_type1.glyphs.txt"), "{form}: glyphs");
    }
}

#[test]
fn bare_cff_reads_as_tx_dumps_it() {
    let mut warnings = Vec::new();
    let ps = read_cff(&cff(), &mut warnings).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(ps.dump_dicts(), expected("ps_cff.dicts.txt"));
    assert_eq!(ps.dump_glyphs(), expected("ps_cff.glyphs.txt"));
}

#[test]
fn type1_and_bare_cff_open_as_one_master_documents() {
    for (form, bytes) in [("pfb", pfb()), ("cff", cff())] {
        let d = decompile(&bytes, None).unwrap();
        let font = &d.font;
        assert_eq!(font.family_name, "PS Test", "{form}");
        assert_eq!((font.version_major, font.version_minor), (1, 2), "{form}");
        assert_eq!(font.glyphs.len(), 8, "{form}");
        let master = font.masters.first().unwrap();
        assert_eq!(master.name, "Regular");
        // StemSnapH 40 50, StemSnapV 40 60.
        assert_eq!(master.stem_values, vec![40.0, 50.0, 40.0, 60.0], "{form}");
        assert_eq!(font.stems.iter().filter(|s| s.horizontal).count(), 2, "{form}");
        // The bottom zone -10..0 is the baseline's overshoot; 700..710 meets no metric.
        // Metrics in order: ascender, cap height, x-height, baseline, descender.
        assert_eq!(master.metric_values.get(3).map(|v| v.over), Some(-10.0), "{form}");
        assert!(master.zones.iter().any(|z| z.pos == 700.0 && z.size == 10.0), "{form}: {:?}", master.zones);
        assert!(master.params.get("blueShift").is_some(), "{form}");

        let a = font.glyph("A").unwrap();
        assert_eq!(a.unicodes, vec![0x41], "{form}");
        let layer = a.layers.first().unwrap();
        assert_eq!(layer.width, 600.0, "{form}");
        // Hint replacement keeps every distinct stem once: two horizontal, one vertical.
        assert_eq!(layer.info.hints.len(), 3, "{form}");

        let o = font.glyph("O").unwrap().layers.first().unwrap();
        let Some(Shape::Path(path)) = o.shapes.first() else { panic!("{form}: O has no path") };
        assert_eq!(path.nodes.iter().filter(|n| n.kind == NodeKind::Curve).count(), 2, "{form}: flex is two curves");

        let accented = font.glyph("Aacute").unwrap().layers.first().unwrap();
        let components: Vec<(&str, [f64; 6])> = accented
            .shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Component(c) => match c.transform {
                    Transform::Matrix(m) => Some((c.glyph.as_str(), m)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // The Type 1 accent offset includes the sidebearing difference (20 - 30).
        let adx = if form == "cff" { 150.0 } else { 140.0 };
        assert_eq!(components, vec![("A", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), ("acute", [1.0, 0.0, 0.0, 1.0, adx, 150.0])], "{form}");
    }
}

#[test]
fn corpus_type1_and_cff_files_read_as_tx_dumps_them() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut differ = Vec::new();
    for file in ["TestT1-Regular.pfa", "TestT1-Regular.pfb", "TestT1-weird-zeros.pfa", "TestT1-ellipsis-hinted.pfa", "TestSupplementEncoding.cff"] {
        let dir = if file.ends_with(".cff") { "cffLib/data" } else { "t1Lib/data" };
        let bytes = std::fs::read(corpus.join(dir).join(file)).unwrap();
        let mut warnings = Vec::new();
        let ps = if file.ends_with(".cff") { read_cff(&bytes, &mut warnings) } else { read_type1(&bytes, &mut warnings) }.unwrap();
        let ours = format!("{}{}", ps.dump_dicts(), ps.dump_glyphs());
        if ours != expected(&format!("corpus_{file}.txt")) {
            differ.push(file);
        }
        decompile(&bytes, None).unwrap();
    }
    assert!(differ.is_empty(), "dumps differ from tx: {differ:?}");
}

#[test]
fn damaged_type1_and_cff_fonts_fail_with_a_message() {
    let good = pfb();
    for cut in [2, 6, 40, good.len() / 2] {
        let _ = decompile(&good[..cut], None);
    }
    let cff = cff();
    for cut in [4, 10, 60, cff.len() / 2, cff.len() - 3] {
        let _ = decompile(&cff[..cut], None);
    }
    // A PFB segment claiming 4 GB.
    let huge = [vec![0x80, 1], u32::MAX.to_le_bytes().to_vec(), b"%!".to_vec()].concat();
    assert!(decompile(&huge, None).is_err());
    // A subroutine that calls itself stops at the nesting limit, and the glyph is reported.
    let looping = CLEAR.to_string();
    let mut private = PRIVATE_HEAD.as_bytes().to_vec();
    let sub = encrypt(&[vec![0; 4], assemble(&["0", "callsubr", "return"], false)].concat(), 4330);
    private.extend(format!("/Subrs 1 array\ndup 0 {} RD ", sub.len()).bytes());
    private.extend(sub);
    let glyph = encrypt(&[vec![0; 4], assemble(&["0", "500", "hsbw", "0", "callsubr", "endchar"], false)].concat(), 4330);
    private.extend(format!(" NP\nND\n2 index /CharStrings 1 dict dup begin\n/.notdef {} RD ", glyph.len()).bytes());
    private.extend(glyph);
    private.extend(b" ND\nend\n");
    let font = [looping.into_bytes(), encrypt(&[vec![0; 4], private].concat(), 55665)].concat();
    let mut warnings = Vec::new();
    read_type1(&font, &mut warnings).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn charstring_arithmetic_cannot_put_infinite_points_in_the_model() {
    // 30000^68 * 5000 is about 1.3e308, still finite; two of them added overflow to infinity.
    let huge = format!("30000{} 5000 mul", " 30000 mul".repeat(67));
    let text = format!("{CFF_SPEC}\nglyph huge: 0 0 rmoveto {huge} 0 rlineto {huge} 0 rlineto 0 {huge} rlineto endchar\n");
    let d = decompile(&cff_from(&text), None).unwrap();
    for glyph in d.font.glyphs.iter() {
        for layer in &glyph.layers {
            for shape in &layer.shapes {
                let Shape::Path(path) = shape else { continue };
                for node in &path.nodes {
                    assert!(node.x.is_finite() && node.y.is_finite() && node.x.abs() <= 1e6 && node.y.abs() <= 1e6, "{}: {node:?}", glyph.name);
                }
            }
        }
    }
    assert!(d.font.glyph("huge").is_some(), "the glyph stays, without the refused outline");
}

#[test]
fn charstrings_after_dup_are_read() {
    // Some writers end the subroutines with `def put` and open the glyphs with `dup /CharStrings`.
    let expected = read_type1(&pfb(), &mut Vec::new()).unwrap().glyphs.len();
    let font = read_type1(&pfb_from(type1_parts_with("def\nput\ndup /CharStrings")), &mut Vec::new()).unwrap();
    assert_eq!(font.glyphs.len(), expected);
}
