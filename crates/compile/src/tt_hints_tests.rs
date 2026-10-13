//! TrueType hinting at export, run in skrifa's interpreter.

use std::collections::BTreeMap;

use fontcraft_hinting::tt::delta_column_key;
use fontcraft_hinting::{DeltaDir, Direction, NodeRef, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, set_tt_instructions};
use fontcraft_model::plist::{Array, Number, Str, Value};
use fontcraft_model::{Font, Node, NodeKind, Param, Path, Shape, Target};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target as Intent};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::Glyph as RawGlyph;
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::{Compiled, Options, compile, compile_ttf};

fn node(path: usize, node: usize) -> TtPoint {
    TtPoint::Node(NodeRef::new(path, node))
}

fn instruction(kind: TtKind, direction: Direction, target: TtPoint, origin: Option<TtPoint>, other: Option<TtPoint>) -> TtInstruction {
    let rounding = TtInstruction::default_rounding(&kind);
    TtInstruction { kind, direction, rounding, target, origin, other, source: None }
}

/// An `o` of four cubic curves and an `H`-like bar, each with instructions; the `o`'s Interpolate
/// moves an off-curve node, which only exists in the source as a cubic handle.
fn hinted_font() -> Font {
    let mut f = Font::new("Hint Sans");
    let n = Node::new;
    let (cx, cy, rx, ry, k) = (300.0, 250.0, 250.0, 260.0, 0.5523);
    {
        let mut o = f.glyph_mut("o").unwrap();
        o.layers[0].width = 600.0;
        o.layers[0].shapes.push(Shape::Path(Path::new(
            true,
            vec![
                n(cx + rx, cy, NodeKind::Curve),
                n(cx + rx, cy + ry * k, NodeKind::OffCurve),
                n(cx + rx * k, cy + ry, NodeKind::OffCurve),
                n(cx, cy + ry, NodeKind::Curve),
                n(cx - rx * k, cy + ry, NodeKind::OffCurve),
                n(cx - rx, cy + ry * k, NodeKind::OffCurve),
                n(cx - rx, cy, NodeKind::Curve),
                n(cx - rx, cy - ry * k, NodeKind::OffCurve),
                n(cx - rx * k, cy - ry, NodeKind::OffCurve),
                n(cx, cy - ry, NodeKind::Curve),
                n(cx + rx * k, cy - ry, NodeKind::OffCurve),
                n(cx + rx, cy - ry * k, NodeKind::OffCurve),
            ],
        )));
        let column = delta_column_key(&[]);
        let cells = BTreeMap::from([(column, BTreeMap::from([(12u16, DeltaDir::Up)]))]);
        let list = vec![
            instruction(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, node(0, 9), None, None),
            instruction(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, node(0, 3), None, None),
            instruction(TtKind::Interpolate, Direction::Vertical, node(0, 0), Some(node(0, 9)), Some(node(0, 3))),
            instruction(TtKind::Interpolate, Direction::Vertical, node(0, 1), Some(node(0, 9)), Some(node(0, 3))),
            instruction(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Horizontal, node(0, 0), Some(node(0, 6)), None),
            instruction(TtKind::Delta { cells }, Direction::Vertical, node(0, 6), None, None),
        ];
        set_tt_instructions(&mut o.layers[0], &list).unwrap();
    }
    {
        let mut h = f.glyph_mut("H").unwrap();
        h.layers[0].width = 600.0;
        h.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0)));
        h.layers[0].shapes.push(Shape::Path(Path::rect(450.0, 0.0, 550.0, 700.0)));
        h.layers[0].shapes.push(Shape::Path(Path::rect(100.0, 300.0, 500.0, 380.0)));
        let list = vec![
            instruction(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, node(0, 0), None, None),
            instruction(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, node(0, 2), None, None),
            instruction(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, node(2, 2), Some(node(2, 0)), None),
            instruction(TtKind::Shift, Direction::Vertical, node(1, 0), Some(node(0, 0)), None),
        ];
        set_tt_instructions(&mut h.layers[0], &list).unwrap();
    }
    f
}

struct Sink;
impl OutlinePen for Sink {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

/// Runs every glyph program of `bytes` at 8 to 72 PPM in the three rendering intents.
fn runs_everywhere(bytes: &[u8]) {
    let font = FontRef::new(bytes).unwrap();
    let outlines = font.outline_glyphs();
    let intents = [
        Intent::Mono,
        Intent::Smooth { mode: SmoothMode::Normal, symmetric_rendering: true, preserve_linear_metrics: false },
        Intent::Smooth { mode: SmoothMode::Lcd, symmetric_rendering: false, preserve_linear_metrics: false },
    ];
    let count = font.maxp().unwrap().num_glyphs();
    for target in intents {
        for ppem in 8..=72u16 {
            let options = HintingOptions { engine: Engine::Interpreter, target };
            let instance = HintingInstance::new(&outlines, Size::new(f32::from(ppem)), LocationRef::default(), options)
                .unwrap_or_else(|e| panic!("fpgm or prep fails at {ppem} PPM: {e}"));
            for gid in 0..count {
                let glyph = outlines.get(GlyphId::new(u32::from(gid))).unwrap();
                glyph.draw(DrawSettings::hinted(&instance, true), &mut Sink).unwrap_or_else(|e| panic!("glyph {gid} fails at {ppem} PPM: {e}"));
            }
        }
    }
}

fn gid(out: &Compiled, name: &str) -> GlyphId {
    GlyphId::new(u32::try_from(out.glyph_order.iter().position(|n| n == name).unwrap()).unwrap())
}

fn program<'a>(font: &FontRef<'a>, g: GlyphId) -> &'a [u8] {
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    match loca.get_glyf(g, &glyf).unwrap() {
        Some(RawGlyph::Simple(s)) => s.instructions(),
        _ => &[],
    }
}

#[test]
fn hinted_glyphs_export_with_their_programs_and_maxp_covers_them() {
    let out = compile_ttf(&hinted_font(), &Options::default()).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("left out") || w.contains("problem")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    for tag in [b"cvt ", b"fpgm", b"prep", b"gasp"] {
        assert!(font.table_data(skrifa::Tag::new(tag)).is_some(), "{}", String::from_utf8_lossy(tag));
    }
    assert_eq!(font.head().unwrap().flags().bits() & 0x000C, 0x000C, "programs depend on whole-pixel sizes");
    let fpgm = font.table_data(skrifa::Tag::new(b"fpgm")).unwrap();
    let prep = font.table_data(skrifa::Tag::new(b"prep")).unwrap();
    let functions = fontcraft_ttasm::FunctionTable::from_fpgm(fpgm.as_bytes()).unwrap();
    let mut need = fontcraft_ttasm::analyse(fpgm.as_bytes(), &functions).unwrap();
    need.merge(&fontcraft_ttasm::analyse(prep.as_bytes(), &functions).unwrap());
    // `maxSizeOfInstructions` counts glyph programs only.
    need.max_size_of_instructions = 0;
    for name in ["o", "H"] {
        let p = program(&font, gid(&out, name));
        assert!(!p.is_empty(), "{name} has a program");
        need.merge(&fontcraft_ttasm::analyse(p, &functions).unwrap());
    }
    assert!(program(&font, gid(&out, ".notdef")).is_empty());
    let maxp = font.maxp().unwrap();
    assert!(maxp.max_stack_elements().unwrap() >= need.max_stack_elements);
    assert!(maxp.max_function_defs().unwrap() >= need.max_function_defs);
    assert!(maxp.max_storage().unwrap() >= need.max_storage);
    assert!(maxp.max_twilight_points().unwrap() >= need.max_twilight_points);
    assert!(maxp.max_zones().unwrap() >= need.max_zones);
    assert!(maxp.max_size_of_instructions().unwrap() >= need.max_size_of_instructions);
    runs_everywhere(&out.bytes);
}

#[test]
fn point_indexes_name_the_exported_points() {
    let f = hinted_font();
    let out = compile_ttf(&f, &Options::default()).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    let Some(RawGlyph::Simple(o)) = loca.get_glyf(gid(&out, "o"), &glyf).unwrap() else { panic!("o is simple") };
    let points: Vec<_> = o.points().collect();
    let map = crate::points::point_indexes(&f, &Target::Master("m01".into()), "o", &Options::default()).unwrap().unwrap();
    assert_eq!(usize::from(map.count), points.len());
    let layer = &f.glyph("o").unwrap().layers[0];
    let Shape::Path(path) = &layer.shapes[0] else { panic!() };
    for (i, n) in path.nodes.iter().enumerate() {
        let at = points[usize::from(map.paths[0][i].unwrap())];
        assert_eq!(at.on_curve, n.on_curve(), "node {i}");
        if n.on_curve() {
            assert_eq!((f64::from(at.x), f64::from(at.y)), (n.x, n.y), "on-curve node {i} keeps its place");
        }
    }
}

#[test]
fn hints_are_left_out_when_stripped_or_switched_off() {
    let mut f = hinted_font();
    let stripped = compile_ttf(&f, &Options { hinting: crate::HintingOptions { strip: true, ..Default::default() }, ..Options::default() }).unwrap();
    assert!(FontRef::new(&stripped.bytes).unwrap().table_data(skrifa::Tag::new(b"fpgm")).is_none());
    std::sync::Arc::make_mut(&mut f.info).params.0.push(Param::new("Export TrueType Hints", Value::Number(Number::int(0))));
    let off = compile_ttf(&f, &Options::default()).unwrap();
    let font = FontRef::new(&off.bytes).unwrap();
    assert!(font.table_data(skrifa::Tag::new(b"prep")).is_none());
    assert!(program(&font, gid(&off, "o")).is_empty());
}

#[test]
fn imported_tables_are_written_as_they_are() {
    let mut f = hinted_font();
    let params = &mut std::sync::Arc::make_mut(&mut f.masters)[0].params.0;
    params.push(Param::new(
        "cvt Table",
        Value::Array(Array { items: vec![Value::Number(Number::int(10)), Value::Number(Number::int(-20))], ..Default::default() }),
    ));
    params.push(Param::new("fpgm Table Assembly", Value::String(Str::new("PUSHB[ ] 0\nFDEF[ ]\nPOP[ ]\nENDF[ ]"))));
    params.push(Param::new("prep Table Assembly", Value::String(Str::new("PUSHB[ ] 1\nPUSHB[ ] 0\nCALL[ ]"))));
    let out = compile_ttf(&f, &Options::default()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("imported TrueType tables")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(font.table_data(skrifa::Tag::new(b"cvt ")).unwrap().as_bytes(), [0, 10, 0xFF, 0xEC]);
    assert_eq!(
        font.table_data(skrifa::Tag::new(b"fpgm")).unwrap().as_bytes(),
        fontcraft_ttasm::assemble("PUSHB[ ] 0\nFDEF[ ]\nPOP[ ]\nENDF[ ]").unwrap()
    );
    assert!(program(&font, gid(&out, "o")).is_empty(), "manual instructions need our fpgm");
    assert!(font.maxp().unwrap().max_function_defs().unwrap() >= 1);
    runs_everywhere(&out.bytes);
}

/// [`hinted_font`] with a second master at weight 200 whose layers carry no hints, and an
/// instance `Medium` at 150.
fn two_masters() -> Font {
    let mut f = hinted_font();
    f.axes = std::sync::Arc::new(vec![fontcraft_model::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let masters = std::sync::Arc::make_mut(&mut f.masters);
    masters[0].location = vec![100.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    bold.location = vec![200.0];
    masters.push(bold);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let mut layer = g.layers[0].clone();
        layer.id = "m02".into();
        layer.master = "m02".into();
        layer.info.hints.clear();
        g.layers.push(layer);
    }
    let inst = fontcraft_model::Instance { name: "Medium".into(), location: vec![150.0], ..Default::default() };
    f.instances = std::sync::Arc::new(vec![inst]);
    f
}

#[test]
fn an_instance_between_masters_exports_its_programs() {
    let f = two_masters();
    let out = compile(&f, &Target::Instance(0), crate::Flavour::TrueType, &Options::default()).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(!program(&font, gid(&out, "o")).is_empty(), "the main master's instructions apply to the instance: {:?}", out.warnings);
    runs_everywhere(&out.bytes);
}

#[test]
fn a_variable_font_takes_the_main_master_s_programs_without_deltas() {
    let f = two_masters();
    let var = crate::var::compile_variable(&f, None, &Options::default(), &crate::var::VariableOptions::default()).unwrap();
    let out = &var.compiled;
    assert!(out.warnings.iter().any(|w| w.contains("cvar")), "{:?}", out.warnings);
    assert!(out.warnings.iter().any(|w| w.contains("Delta instructions are not written")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(font.table_data(skrifa::Tag::new(b"gvar")).is_some());
    assert!(!program(&font, gid(out, "o")).is_empty(), "{:?}", out.warnings);
    runs_everywhere(&out.bytes);
}

#[test]
fn an_unhinted_variable_export_of_an_instructed_source_writes_the_plain_points() {
    let glyf = |f: &Font, options: &Options| -> Vec<u8> {
        let var = crate::var::compile_variable(f, None, options, &crate::var::VariableOptions::default()).unwrap();
        FontRef::new(&var.compiled.bytes).unwrap().table_data(skrifa::Tag::new(b"glyf")).unwrap().as_bytes().to_vec()
    };
    let mut plain = two_masters();
    for i in 0..plain.glyphs.len() {
        plain.glyphs.get_mut_at(i).unwrap().layers.iter_mut().for_each(|l| l.info.hints.clear());
    }
    let expected = glyf(&plain, &Options::default());
    let f = two_masters();
    assert_ne!(glyf(&f, &Options::default()), expected, "instructed glyphs keep the on-curve points of their nodes");
    let strip = Options { hinting: crate::HintingOptions { strip: true, ..Default::default() }, ..Options::default() };
    assert_eq!(glyf(&f, &strip), expected, "stripped");
    let mut off = two_masters();
    std::sync::Arc::make_mut(&mut off.info).params.0.push(Param::new("Export TrueType Hints", Value::Number(Number::int(0))));
    assert_eq!(glyf(&off, &Options::default()), expected, "Export TrueType Hints off");
}
