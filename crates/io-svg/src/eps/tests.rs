use fontcraft_model::{NodeKind, Path};

use super::*;
use crate::glyph_name_for_file;

const TOP_200: Placement = Placement::Unscaled { top: 200.0 };

/// A minimal EPS file with a 200-point box around `body`.
fn eps(body: &str) -> String {
    format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 200\n%%EndComments\n{body}\nshowpage\n%%EOF\n")
}

fn nodes(p: &Path) -> Vec<(f64, f64, NodeKind)> {
    p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect()
}

fn err(text: &str) -> EpsError {
    read_eps(text.as_bytes(), TOP_200).unwrap_err()
}

use NodeKind::{Curve, Line, OffCurve};

#[test]
fn eps_square_under_translate_and_scale_becomes_one_closed_path() {
    let mut text = eps("gsave 10 20 translate 2 2 scale newpath 0 0 moveto 50 0 lineto 50 50 lineto 0 50 lineto closepath 0 setgray fill grestore");
    // Nothing after the end of the file runs.
    text.push_str("undefinedoperator\n");
    let out = read_eps(text.as_bytes(), TOP_200).unwrap();
    assert_eq!(out.bounding_box, Some([0.0, 0.0, 200.0, 200.0]));
    assert_eq!(out.paths.len(), 1);
    let p = &out.paths[0];
    assert!(p.closed);
    assert_eq!(nodes(p), vec![(10.0, 20.0, Line), (110.0, 20.0, Line), (110.0, 120.0, Line), (10.0, 120.0, Line)]);
    assert_eq!(out.dropped.attributes.get("colour"), Some(&1));
}

#[test]
fn eps_rcurveto_under_rotate_gives_a_cubic_segment() {
    let out =
        read_eps(eps("100 100 translate 90 rotate 0 0 moveto 10 0 lineto 10 10 20 10 20 0 rcurveto closepath fill").as_bytes(), TOP_200).unwrap();
    assert_eq!(out.paths.len(), 1);
    assert_eq!(
        nodes(&out.paths[0]),
        vec![(100.0, 100.0, Line), (100.0, 110.0, Line), (90.0, 120.0, OffCurve), (90.0, 130.0, OffCurve), (100.0, 130.0, Curve)]
    );
}

/// The shape a vector graphics library writes: abbreviations defined in the prolog, a clip and
/// a flipped coordinate system.
const LIBRARY_STYLE: &str = "%!PS-Adobe-3.0 EPSF-3.0
%%Creator: a test
%%BoundingBox: 0 0 100 100
%%EndComments
%%BeginProlog
50 dict begin
/q { gsave } bind def
/Q { grestore } bind def
/cm { 6 array astore concat } bind def
/m { moveto } bind def
/l { lineto } bind def
/c { curveto } bind def
/h { closepath } bind def
/re { exch dup neg 3 1 roll 5 3 roll moveto 0 rlineto
      0 exch rlineto 0 rlineto closepath } bind def
/f { fill } bind def
/g { setgray } bind def
/W { clip } bind def
/n { newpath } bind def
/BDC { mark 3 1 roll /BDC pdfmark } bind def
/languagelevel where { pop languagelevel } { 1 } ifelse 2 lt { /Q { grestore } def } if
%%EndProlog
%%BeginSetup
%%EndSetup
%%Page: 1 1
%%BeginPageSetup
%%PageBoundingBox: 0 0 100 100
%%EndPageSetup
q 0 0 100 100 re W n
1 0 0 -1 0 100 cm
0 g
10 10 m 90 10 l 50 90 l h
f
Q
showpage
%%Trailer
end
%%EOF
";

#[test]
fn eps_prolog_definitions_in_the_subset_run_and_the_rest_is_skipped() {
    let out = read_eps(LIBRARY_STYLE.as_bytes(), Placement::Unscaled { top: 100.0 }).unwrap();
    assert_eq!(out.paths.len(), 1, "the clipping rectangle is not an outline");
    let p = &out.paths[0];
    assert!(p.closed);
    assert_eq!(nodes(p), vec![(10.0, 90.0, Line), (90.0, 90.0, Line), (50.0, 10.0, Line)]);
    assert_eq!(out.dropped.elements.get("clipping path"), Some(&1));
    assert_eq!(out.dropped.attributes.get("colour"), Some(&1));
    assert!(out.dropped.summary().is_some());
}

#[test]
fn eps_procedures_beyond_the_subset_are_refused_with_the_operator_name() {
    assert_eq!(err(&eps("/loop10 { 0 1 10 { pop } for } def loop10")), EpsError::Unsupported { op: "for".into() });
    assert_eq!(err(&eps("/Helvetica findfont 12 scalefont setfont")), EpsError::Unsupported { op: "findfont".into() });
    // A prolog procedure that needs a conditional is not kept, so the page's call to it fails.
    let text =
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n%%BeginProlog\n/m { dup 0 gt { moveto } if } bind def\n%%EndProlog\n1 1 m\n%%EOF\n";
    assert_eq!(err(text), EpsError::Unsupported { op: "m".into() });
}

const MARKED: &str = "%!PS-Adobe-3.0 EPSF-3.0
%%BoundingBox: 0 0 100 100
%AI5_FileFormat 3
%%EndComments
%%BeginProlog
/m { _doClip 1 eq { moveto } { pop pop } ifelse } def
%%EndProlog
%%BeginSetup
%%EndSetup
1 1 1 1 0 0 1 0 79 128 255 0 50 Lb
(Layer 1) Ln
0 A
u
0 g 0 G 1 w [] 0 d 0 J 0 j
0 0 m
100 0 L
100 100 l
60 120 40 120 0 100 c
0 50 0 0 v
f
10 10 m 20 20 L N
U
LB
%%PageTrailer
gsave annotatepage grestore showpage
%%Trailer
%%EOF
";

#[test]
fn eps_drawing_application_abbreviations_are_read_when_the_file_is_marked() {
    let out = read_eps(MARKED.as_bytes(), Placement::Unscaled { top: 100.0 }).unwrap();
    assert_eq!(out.paths.len(), 1, "the unpainted path is left out");
    let p = &out.paths[0];
    assert!(p.closed);
    assert_eq!(
        nodes(p),
        vec![
            (0.0, 0.0, Curve),
            (100.0, 0.0, Line),
            (100.0, 100.0, Line),
            (60.0, 120.0, OffCurve),
            (40.0, 120.0, OffCurve),
            (0.0, 100.0, Curve),
            (0.0, 100.0, OffCurve),
            (0.0, 50.0, OffCurve),
        ]
    );
    assert_eq!(out.dropped.elements.get("unpainted path"), Some(&1));
    assert_eq!(out.dropped.attributes.get("colour"), Some(&2));
    assert_eq!(out.dropped.attributes.get("line width"), Some(&1));
    assert_eq!(out.dropped.attributes.get("line style"), Some(&3));

    // Without the marker, the same operators are not known.
    let unmarked = MARKED.replace("%AI5_FileFormat 3\n", "");
    assert_eq!(err(&unmarked), EpsError::Unsupported { op: "Lb".into() });
}

#[test]
fn eps_a_path_painted_inside_gsave_and_again_after_grestore_is_read_once() {
    let out = read_eps(
        eps("newpath 0 0 moveto 10 0 lineto 10 10 lineto closepath gsave 0.5 setgray fill grestore 2 setlinewidth stroke").as_bytes(),
        TOP_200,
    )
    .unwrap();
    assert_eq!(out.paths.len(), 1);
}

#[test]
fn eps_fit_em_scales_the_bounding_box_and_without_one_the_drawing_bounds_are_the_frame() {
    let text = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 50\n0 0 100 50 rectfill\n%%EOF\n";
    let out = read_eps(text.as_bytes(), Placement::FitEm { units_per_em: 1000.0, ascender: 800.0 }).unwrap();
    assert_eq!(nodes(&out.paths[0]), vec![(0.0, -200.0, Line), (2000.0, -200.0, Line), (2000.0, 800.0, Line), (0.0, 800.0, Line)]);

    let out = read_eps(b"%!PS\n5 5 10 10 rectfill\n", Placement::Unscaled { top: 0.0 }).unwrap();
    assert_eq!(out.bounding_box, None);
    assert_eq!(nodes(&out.paths[0]), vec![(5.0, -10.0, Line), (15.0, -10.0, Line), (15.0, 0.0, Line), (5.0, 0.0, Line)]);
}

fn dos_eps(offset: u32, length: u32, ps: &[u8]) -> Vec<u8> {
    let mut data = vec![0xC5, 0xD0, 0xD3, 0xC6];
    data.extend_from_slice(&offset.to_le_bytes());
    data.extend_from_slice(&length.to_le_bytes());
    data.resize(30, 0);
    data.extend_from_slice(ps);
    data
}

#[test]
fn eps_dos_binary_header_is_followed_to_its_postscript_section() {
    let ps = eps("0 0 10 10 rectfill");
    let data = dos_eps(30, ps.len() as u32, ps.as_bytes());
    assert!(looks_like_eps(&data));
    assert_eq!(read_eps(&data, TOP_200).unwrap().paths.len(), 1);
    let bad = dos_eps(30, ps.len() as u32 + 1, ps.as_bytes());
    assert_eq!(read_eps(&bad, TOP_200).unwrap_err(), EpsError::BadBinaryHeader);
    let past = dos_eps(u32::MAX, u32::MAX, ps.as_bytes());
    assert_eq!(read_eps(&past, TOP_200).unwrap_err(), EpsError::BadBinaryHeader);
}

#[test]
fn eps_hostile_input_fails_with_a_message() {
    // A procedure that calls itself, and procedures that multiply the work.
    assert_eq!(err(&eps("/a { a } def a")), EpsError::TooDeep);
    let mut body = String::from("/l0 { 0 pop } def\n");
    for i in 1..10 {
        body.push_str(&format!("/l{i} {{ {} }} def\n", vec![format!("l{}", i - 1); 10].join(" ")));
    }
    body.push_str("l9\n");
    assert_eq!(err(&eps(&body)), EpsError::TooMuchWork);
    // Stack, nesting, saved states and allocation.
    assert_eq!(err(&eps(&"0 ".repeat(MAX_EPS_STACK + 1))), EpsError::StackOverflow);
    assert_eq!(err(&eps(&format!("{}{}", "{".repeat(100), "}".repeat(100)))), EpsError::TooDeep);
    assert_eq!(err(&eps(&"gsave ".repeat(MAX_EPS_GSAVE_DEPTH + 1))), EpsError::TooDeep);
    assert_eq!(err(&eps(&"/x 10000 array def ".repeat(101))), EpsError::TooMuchData);
    let mut long_path = String::from("0 0 moveto ");
    long_path.push_str(&"1 1 lineto ".repeat(MAX_EPS_PATH_SEGMENTS + 1));
    assert_eq!(err(&eps(&long_path)), EpsError::TooManyNodes);
    // Numbers and arithmetic.
    assert_eq!(err(&eps("1e400 0 moveto")), EpsError::NumberOutOfRange);
    assert_eq!(err(&eps("1e300 1e300 mul")), EpsError::NumberOutOfRange);
    assert_eq!(err(&eps("1 0 div")), EpsError::DivideByZero { op: "div".into() });
    assert_eq!(err(&eps("1e8 1e8 scale 100 100 moveto")), EpsError::NumberOutOfRange);
    // Syntax.
    assert!(matches!(err(&eps("{ 1 2")), EpsError::Syntax(_)));
    assert!(matches!(err(&eps("(not closed")), EpsError::Syntax(_)));
    assert!(matches!(err(&eps(&format!("/{}", "a".repeat(300)))), EpsError::Syntax(_)));
    // Operands.
    assert_eq!(err(&eps("10 10 lineto")), EpsError::NoCurrentPoint { op: "lineto".into() });
    assert_eq!(err(&eps("moveto")), EpsError::StackUnderflow { op: "moveto".into() });
    assert_eq!(err(&eps("(text) 1 moveto")), EpsError::TypeCheck { op: "moveto".into() });
    // Not PostScript, or too large.
    assert_eq!(err("<svg/>"), EpsError::NotPostScript);
    assert!(!looks_like_eps(b"<svg/>"));
    let huge = vec![b' '; MAX_EPS_BYTES + 1];
    assert_eq!(read_eps(&huge, TOP_200).unwrap_err(), EpsError::TooLarge { size: MAX_EPS_BYTES + 1 });
    // Bad placements.
    let square = eps("0 0 10 10 rectfill");
    assert!(matches!(read_eps(square.as_bytes(), Placement::Unscaled { top: f64::NAN }), Err(EpsError::BadPlacement(_))));
    assert!(matches!(read_eps(square.as_bytes(), Placement::FitEm { units_per_em: 0.0, ascender: 800.0 }), Err(EpsError::BadPlacement(_))));
}

#[test]
fn eps_every_truncation_of_a_file_fails_or_reads_without_panicking() {
    for text in [LIBRARY_STYLE, MARKED] {
        for n in 0..text.len() {
            let _ = read_eps(&text.as_bytes()[..n], TOP_200);
        }
    }
}

#[test]
fn eps_file_names_map_to_glyph_names() {
    assert_eq!(glyph_name_for_file("art/A.eps"), "A");
    assert_eq!(glyph_name_for_file("comma.PDF"), "comma");
}

#[test]
fn eps_arrays_inside_arrays_are_refused_so_values_stay_shallow() {
    // Each step wraps the previous array in a new one. Read to the end, the chain would be
    // dropped recursively, one stack frame per level; the reader refuses the first nesting and
    // drops a shallow interpreter.
    let mut body = String::from("[ ] ");
    body.push_str(&"[ exch ] ".repeat(200_000));
    assert_eq!(err(&eps(&body)), EpsError::TypeCheck { op: "]".into() });
    assert_eq!(err(&eps("[ ] 1 array astore")), EpsError::TypeCheck { op: "astore".into() });
    // A flat matrix still works.
    assert!(read_eps(eps("[ 1 0 0 1 10 10 ] concat 0 0 10 10 rectfill").as_bytes(), TOP_200).is_ok());
}

#[test]
fn eps_path_copies_kept_by_saved_states_count_against_the_allocation_limit() {
    // Every `gsave` shares the path, and the next `lineto` copies it; the saved states keep
    // each copy alive.
    let mut body = String::from("0 0 moveto ");
    body.push_str(&"1 1 lineto ".repeat(100_000));
    body.push_str(&"gsave 1 1 lineto ".repeat(11));
    assert_eq!(err(&eps(&body)), EpsError::TooMuchData);
}
