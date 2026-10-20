//! Behaviour tests of the subroutiniser: outlines read back with read-fonts, expansion back to
//! the input, the format limits, and hostile input.

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::types::{Fixed, GlyphId};

use super::*;
use crate::{Font, Glyph, PathOp, build};

/// Drawing commands with their coordinates in 16.16.
type Drawing = Vec<(char, Vec<i32>)>;

#[derive(Default)]
struct Pen(Drawing);

impl CommandSink for Pen {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('M', vec![x.to_bits(), y.to_bits()]));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.0.push(('L', vec![x.to_bits(), y.to_bits()]));
    }
    fn curve_to(&mut self, a: Fixed, b: Fixed, c: Fixed, d: Fixed, e: Fixed, f: Fixed) {
        self.0.push(('C', [a, b, c, d, e, f].iter().map(|v| v.to_bits()).collect()));
    }
    fn close(&mut self) {
        self.0.push(('Z', Vec::new()));
    }
}

/// Every glyph's outline and width, as read-fonts draws them from a `CFF ` table.
fn outlines(table: &[u8]) -> Vec<(Drawing, Option<i32>)> {
    let cff = CffFontRef::new_cff(table, 0, None).unwrap();
    (0..cff.num_glyphs())
        .map(|gid| {
            let gid = GlyphId::new(gid);
            let subfont = cff.subfont(cff.subfont_index(gid).unwrap_or(0), &[]).unwrap();
            let mut pen = Pen::default();
            let width = cff.evaluate_charstring(&subfont, gid, &[], &mut pen).unwrap();
            (pen.0, width.map(Fixed::to_bits))
        })
        .collect()
}

/// A contour of one of four kinds at `(x, y)`; the kinds repeat across glyphs as serifs and
/// bowls do.
fn shape(kind: usize, x: f64, y: f64) -> Vec<PathOp> {
    let c = |a: (f64, f64), b: (f64, f64), e: (f64, f64)| PathOp::CurveTo((x + a.0, y + a.1), (x + b.0, y + b.1), (x + e.0, y + e.1));
    let l = |a: f64, b: f64| PathOp::LineTo((x + a, y + b));
    let mut p = vec![PathOp::MoveTo((x, y))];
    match kind % 4 {
        0 => p.extend([l(80.0, 0.0), l(80.0, 600.0), l(130.0, 640.0), l(-50.0, 640.0), l(0.0, 600.0)]),
        1 => p.extend([
            c((55.0, 0.0), (100.0, 45.0), (100.0, 100.0)),
            c((100.0, 155.0), (55.0, 200.0), (0.0, 200.0)),
            c((-55.0, 200.0), (-100.0, 155.0), (-100.0, 100.0)),
            c((-100.0, 45.0), (-55.0, 0.0), (0.0, 0.0)),
        ]),
        2 => p.extend([l(300.0, 0.0), l(310.0, 25.0), l(320.0, 60.0), c((280.0, 70.0), (200.0, 71.0), (150.0, 72.0)), l(0.0, 72.0)]),
        _ => p.extend([l(40.0, 0.0), c((60.0, 120.0), (90.0, 300.0), (95.0, 500.0)), l(55.0, 500.0), c((50.0, 300.0), (20.0, 120.0), (0.0, 0.0))]),
    }
    p
}

fn sample_font() -> Font {
    let mut glyphs = vec![Glyph { name: ".notdef".into(), advance: 500.0, ..Glyph::default() }];
    for g in 0..40usize {
        let mut path = Vec::new();
        for part in 0..(2 + g % 3) {
            let kind = (g * 7 + part * 3) % 4;
            path.extend(shape(kind, 50.0 + 211.0 * part as f64 + (g % 5) as f64, (g % 3) as f64 * 10.0));
        }
        glyphs.push(Glyph { name: format!("g{g}"), advance: 600.0 + (g % 4) as f64 * 50.0, path, hints: None });
    }
    Font { name: "Subr-Test".into(), glyphs, ..Font::default() }
}

/// Assembles a charstring from operator names and integers; `mask:HEX` is a hint mask's bytes.
fn cs(program: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for word in program.split_whitespace() {
        if let Some(hex) = word.strip_prefix("mask:") {
            out.extend((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap(), 16).unwrap()));
            continue;
        }
        if let Ok(v) = word.parse::<i64>() {
            push_int(v, &mut out).unwrap();
            continue;
        }
        let op: &[u8] = match word {
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
            "vsindex" => &[15],
            "blend" => &[16],
            "hstemhm" => &[18],
            "hintmask" => &[19],
            "rmoveto" => &[21],
            "hmoveto" => &[22],
            "callgsubr" => &[29],
            "add" => &[12, 10],
            _ => panic!("unknown operator {word}"),
        };
        out.extend_from_slice(op);
    }
    out
}

/// The operators of a charstring without hint masks, as bytes, numbers skipped.
fn operators(code: &[u8]) -> Vec<u8> {
    let mut ops = Vec::new();
    let mut i = 0;
    while i < code.len() {
        let b = code[i];
        i += match b {
            28 => 3,
            32..=246 => 1,
            247..=254 => 2,
            255 => 5,
            12 => {
                ops.push(12);
                2
            }
            _ => {
                ops.push(b);
                1
            }
        };
    }
    ops
}

/// The deepest call chain of a set (a glyph alone is depth 0), following calls as `flat` does.
fn max_depth(set: &CharstringSet) -> usize {
    fn walk(set: &CharstringSet, local: &[Vec<u8>], code: &[u8], depth: usize) -> usize {
        let mut deepest = depth;
        let mut last = 0i64;
        let mut i = 0;
        while i < code.len() {
            let b = code[i];
            if b == 28 || b >= 32 {
                let (len, v) = flat_number(code, i);
                last = v;
                i += len;
                continue;
            }
            if b == 10 || b == 29 {
                let subrs = if b == 10 { local } else { &set.global_subrs[..] };
                let subr = &subrs[(last + bias(subrs.len())) as usize];
                deepest = deepest.max(walk(set, local, subr, depth + 1));
            }
            i += if b == 12 { 2 } else { 1 };
        }
        deepest
    }
    (0..set.charstrings.len()).map(|g| walk(set, &set.local_subrs[set.fd_of(g)], &set.charstrings[g], 0)).max().unwrap_or(0)
}

fn flat_number(code: &[u8], i: usize) -> (usize, i64) {
    let b0 = i64::from(code[i]);
    match b0 {
        28 => (3, i64::from(i16::from_be_bytes([code[i + 1], code[i + 2]]))),
        32..=246 => (1, b0 - 139),
        247..=250 => (2, (b0 - 247) * 256 + i64::from(code[i + 1]) + 108),
        251..=254 => (2, -(b0 - 251) * 256 - i64::from(code[i + 1]) - 108),
        _ => (5, 0),
    }
}

#[test]
fn a_subroutinised_font_draws_the_same_outlines_in_fewer_bytes() {
    let table = build(&sample_font()).unwrap().table;
    let small = subroutinize_table(&table, false, &SubrOptions::default()).unwrap();
    assert!(small.len() < table.len(), "{} bytes, not fewer than {}", small.len(), table.len());
    assert_eq!(outlines(&small), outlines(&table));
    let set = charstrings_of_table(&small, false).unwrap();
    assert!(!set.global_subrs.is_empty() || !set.local_subrs[0].is_empty());
    // Expanding gives the original charstrings back, byte for byte.
    let flat = desubroutinize_table(&small, false).unwrap();
    assert_eq!(charstrings_of_table(&flat, false).unwrap().charstrings, charstrings_of_table(&table, false).unwrap().charstrings);
    // A subroutinised table subroutinises again to the same outlines.
    let again = subroutinize_table(&small, false, &SubrOptions::default()).unwrap();
    assert_eq!(outlines(&again), outlines(&table));
    // Deterministic.
    assert_eq!(subroutinize_table(&table, false, &SubrOptions::default()).unwrap(), small);
}

#[test]
fn hint_masks_stay_with_their_bytes() {
    // Four stems, so one mask byte; the masks differ between glyphs while the paths repeat.
    let body = "100 50 hlineto 20 30 -20 30 rlineto 5 6 7 8 9 10 rrcurveto 30 -40 rlineto 1 2 3 4 5 6 rrcurveto";
    let glyphs: Vec<Vec<u8>> = (0..12)
        .map(|g| {
            let mask = if g % 2 == 0 { "mask:A0" } else { "mask:50" };
            cs(&format!("0 20 100 20 hstemhm 10 30 200 30 hintmask {mask} {} {} rmoveto {body} hintmask mask:F0 {body} endchar", g * 3, g))
        })
        .collect();
    let set = CharstringSet { charstrings: glyphs.clone(), ..CharstringSet::default() };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert!(sub.index_bytes() < set.index_bytes());
    assert_eq!(desubroutinize(&sub).unwrap().charstrings, glyphs);
}

#[test]
fn calls_never_overflow_the_operand_stack() {
    // 48 operands, then a long tail that every glyph shares: a call cannot start right after the
    // operands, where the subroutine number would be the 49th.
    let tail = "rrcurveto 10 20 rlineto 30 40 rlineto 50 60 rlineto 70 80 rlineto 90 100 rlineto 1 2 3 4 5 6 rrcurveto endchar";
    let glyphs: Vec<Vec<u8>> = (0..10)
        .map(|g| {
            let args: Vec<String> = (0..48).map(|k| ((k * 7 + g * 13) % 200).to_string()).collect();
            cs(&format!("{} 0 rmoveto {} {tail}", g * 10, args.join(" ")))
        })
        .collect();
    let set = CharstringSet { charstrings: glyphs.clone(), ..CharstringSet::default() };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert!(sub.index_bytes() < set.index_bytes());
    // Expansion runs with the 48-entry stack and refuses an overflow.
    assert_eq!(desubroutinize(&sub).unwrap().charstrings, glyphs);
}

#[test]
fn subroutines_nest_at_most_ten_deep() {
    // Glyph k is the first k + 1 segments of one long path, three times over, so each prefix is
    // a candidate that could call the one before it.
    let seg = |k: i64| format!("{} {} rlineto", k % 90 + 3, (k * 37) % 90 + 3);
    let glyphs: Vec<Vec<u8>> = (0..40)
        .map(|k| {
            let path: Vec<String> = (0..=k).map(seg).collect();
            let path = path.join(" ");
            cs(&format!("0 0 rmoveto {path} 5 hmoveto {path} 5 hmoveto {path} endchar"))
        })
        .collect();
    let set = CharstringSet { charstrings: glyphs.clone(), ..CharstringSet::default() };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert!(max_depth(&sub) <= MAX_CALL_DEPTH);
    assert_eq!(desubroutinize(&sub).unwrap().charstrings, glyphs);
}

#[test]
fn with_several_font_dicts_global_subroutines_call_no_local_ones() {
    let shared = "10 20 rlineto 30 40 rlineto 50 60 rlineto 1 2 3 4 5 6 rrcurveto 7 8 9 10 11 12 rrcurveto";
    let own = ["100 0 rlineto 0 100 rlineto -100 0 rlineto", "200 0 rlineto 0 50 rlineto 9 9 9 9 9 9 rrcurveto"];
    let glyphs: Vec<Vec<u8>> = (0..16).map(|g| cs(&format!("{g} 0 rmoveto {shared} {} {} {shared} endchar", own[g % 2], own[g % 2]))).collect();
    let set = CharstringSet {
        charstrings: glyphs.clone(),
        local_subrs: vec![Vec::new(), Vec::new()],
        fd_select: (0..16).map(|g| (g % 2) as u16).collect(),
        ..CharstringSet::default()
    };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert!(sub.index_bytes() < set.index_bytes());
    for g in &sub.global_subrs {
        assert!(!operators(g).contains(&10), "a global subroutine calls a local one");
    }
    assert_eq!(desubroutinize(&sub).unwrap().charstrings, glyphs);
}

#[test]
fn cff2_blends_keep_their_operands_and_need_no_return() {
    // Two regions: a blended value is the default, two deltas and the count.
    let body = "10 1 2 20 3 4 2 blend rlineto 30 40 rlineto 50 60 rlineto 5 0 0 1 blend 6 7 8 9 10 rrcurveto";
    let glyphs: Vec<Vec<u8>> = (0..12).map(|g| cs(&format!("{g} 0 rmoveto {body} 15 15 rlineto {body}"))).collect();
    let set = CharstringSet { cff2: true, charstrings: glyphs.clone(), regions: vec![2], max_stack: CFF2_DEFAULT_STACK, ..CharstringSet::default() };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert!(sub.index_bytes() < set.index_bytes());
    for s in sub.global_subrs.iter().chain(sub.local_subrs.iter().flatten()) {
        assert!(!operators(s).contains(&11) && !operators(s).contains(&14), "CFF2 has no return or endchar");
    }
    assert_eq!(desubroutinize(&sub).unwrap().charstrings, glyphs);
}

#[test]
fn hostile_charstrings_fail_with_a_message() {
    let with = |glyph: &str, global: Vec<&str>| CharstringSet {
        charstrings: vec![cs(glyph)],
        global_subrs: global.into_iter().map(cs).collect(),
        ..CharstringSet::default()
    };
    // A subroutine that calls itself.
    let looping = with("-107 callgsubr endchar", vec!["-107 callgsubr return"]);
    assert!(matches!(desubroutinize(&looping), Err(SubrError::Invalid(_))));
    assert!(matches!(subroutinize(&looping, &SubrOptions::default()), Err(SubrError::Invalid(_))));
    // A call to a subroutine that does not exist, a call with nothing on the stack.
    assert!(matches!(desubroutinize(&with("5 callgsubr endchar", vec![])), Err(SubrError::Invalid(_))));
    assert!(matches!(desubroutinize(&with("callsubr endchar", vec![])), Err(SubrError::Invalid(_))));
    // Arithmetic operators, a hint mask cut short, a reserved operator, a stack overflow.
    assert!(matches!(desubroutinize(&with("1 2 add rmoveto endchar", vec![])), Err(SubrError::Unsupported(_))));
    assert!(matches!(desubroutinize(&with("1 2 3 4 hstemhm hintmask", vec![])), Err(SubrError::Invalid(_))));
    assert!(matches!(desubroutinize(&CharstringSet { charstrings: vec![vec![2]], ..CharstringSet::default() }), Err(SubrError::Invalid(_))));
    let many: Vec<String> = (0..49).map(|k| k.to_string()).collect();
    assert!(matches!(desubroutinize(&with(&format!("{} rlineto endchar", many.join(" ")), vec![])), Err(SubrError::Invalid(_))));
    // Bad set shapes.
    let bad_fd = CharstringSet { charstrings: vec![cs("endchar")], fd_select: vec![3], ..CharstringSet::default() };
    assert!(matches!(subroutinize(&bad_fd, &SubrOptions::default()), Err(SubrError::Invalid(_))));
    let bad_stack = CharstringSet { charstrings: vec![cs("endchar")], max_stack: 10_000, ..CharstringSet::default() };
    assert!(matches!(subroutinize(&bad_stack, &SubrOptions::default()), Err(SubrError::Invalid(_))));
}

#[test]
fn hostile_tables_fail_with_a_message() {
    let table = build(&sample_font()).unwrap().table;
    for cut in [0, 1, 3, 10, table.len() / 2, table.len() - 1] {
        assert!(subroutinize_table(&table[..cut], false, &SubrOptions::default()).is_err(), "cut at {cut}");
    }
    assert!(matches!(subroutinize_table(&table, true, &SubrOptions::default()), Err(SubrError::Unsupported(_))));
    // Garbage after a valid header: refused or accepted, but never a panic.
    let mut junk = table[..4].to_vec();
    junk.extend((0..2000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8));
    let _ = subroutinize_table(&junk, false, &SubrOptions::default());
}

#[test]
fn an_empty_or_unrepetitive_set_comes_back_unchanged() {
    let set = CharstringSet { charstrings: vec![cs("endchar"), cs("10 20 rmoveto 30 hlineto endchar")], ..CharstringSet::default() };
    let sub = subroutinize(&set, &SubrOptions::default()).unwrap();
    assert_eq!(sub.charstrings, set.charstrings);
    assert!(sub.global_subrs.is_empty() && sub.local_subrs.iter().all(Vec::is_empty));
    let none = CharstringSet::default();
    assert_eq!(subroutinize(&none, &SubrOptions::default()).unwrap().charstrings, Vec::<Vec<u8>>::new());
}

#[test]
fn work_beyond_the_memory_budget_is_refused_with_a_message() {
    let table = build(&sample_font()).unwrap().table;
    let set = charstrings_of_table(&table, false).unwrap();
    let options = SubrOptions::default();
    let (result, stats) = subroutinize_measured(&set, &options, MAX_SUBR_MEMORY).unwrap();
    assert!(stats.tokens > set.charstrings.len() && stats.candidates > 0 && stats.peak_memory > 0, "{stats:?}");
    assert_eq!(result, subroutinize(&set, &options).unwrap());
    // The budget is charged the same way every time: its peak is enough, one byte less is not.
    assert_eq!(subroutinize_measured(&set, &options, stats.peak_memory).unwrap(), (result, stats));
    let refused = subroutinize_measured(&set, &options, stats.peak_memory - 1);
    assert!(matches!(refused, Err(SubrError::TooLarge { .. })), "{refused:?}");
}

/// Subroutinises the `CFF ` table of the font that `FONTCRAFT_CJK_FONT` names (a full CJK font,
/// read where it is) and reports what it took. Run with `--ignored --nocapture`.
#[test]
#[ignore = "needs a large CFF font named by FONTCRAFT_CJK_FONT"]
fn a_full_cjk_font_subroutinises() {
    let Some(path) = std::env::var_os("FONTCRAFT_CJK_FONT") else {
        eprintln!("skipped: set FONTCRAFT_CJK_FONT to the path of an OpenType font with a CFF table");
        return;
    };
    let data = std::fs::read(&path).unwrap();
    let font = read_fonts::FontRef::new(&data).unwrap();
    let table = font.table_data(read_fonts::types::Tag::new(b"CFF ")).expect("the font has no CFF table");
    let set = charstrings_of_table(table.as_bytes(), false).unwrap();
    let plain = desubroutinize(&set).unwrap();
    let started = std::time::Instant::now();
    let (result, stats) = subroutinize_measured(&set, &SubrOptions::default(), MAX_SUBR_MEMORY).unwrap();
    let took = started.elapsed();
    assert_eq!(desubroutinize(&result).unwrap().charstrings, plain.charstrings, "the result expands to the input");
    let (before, after, input) = (plain.index_bytes(), result.index_bytes(), set.index_bytes());
    let mb = |b: usize| b as f64 / 1e6;
    eprintln!(
        "{}: {} glyphs, {} Font DICTs, {} tokens, {} candidates; {:.1} s; charstrings and subroutines {:.2} MB, unsubroutinised {:.2} MB ({:.3}), input {:.2} MB; peak memory {:.0} MB of {:.0} MB ({:.1} bytes per token)",
        std::path::Path::new(&path).display(),
        set.charstrings.len(),
        set.fd_count(),
        stats.tokens,
        stats.candidates,
        took.as_secs_f64(),
        mb(after),
        mb(before),
        after as f64 / before as f64,
        mb(input),
        mb(stats.peak_memory),
        mb(MAX_SUBR_MEMORY),
        stats.peak_memory as f64 / stats.tokens as f64,
    );
    assert!(after < before);
}
