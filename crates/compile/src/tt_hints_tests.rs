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
    assert!(out.warnings.iter().any(|w| w.contains("Delta instructions are not written")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(font.table_data(skrifa::Tag::new(b"gvar")).is_some());
    assert!(font.table_data(skrifa::Tag::new(b"cvar")).is_none(), "the masters' control values are the same");
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

/// The lowest and highest y a hinted glyph draws, in pixels.
#[derive(Default)]
struct Extent(Option<(f32, f32)>);
impl OutlinePen for Extent {
    fn move_to(&mut self, _: f32, y: f32) {
        self.add(y);
    }
    fn line_to(&mut self, _: f32, y: f32) {
        self.add(y);
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, y: f32) {
        self.add(y);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, y: f32) {
        self.add(y);
    }
    fn close(&mut self) {}
}
impl Extent {
    fn add(&mut self, y: f32) {
        self.0 = Some(self.0.map_or((y, y), |(lo, hi)| (lo.min(y), hi.max(y))));
    }
}

#[test]
fn autohint_instructs_unhinted_glyphs_and_puts_zone_extremes_on_pixel_rows() {
    let mut f = hinted_font();
    for name in ["o", "H"] {
        set_tt_instructions(&mut f.glyph_mut(name).unwrap().layers[0], &[]).unwrap();
    }
    let unhinted = compile_ttf(&f, &Options::default()).unwrap();
    assert!(FontRef::new(&unhinted.bytes).unwrap().table_data(skrifa::Tag::new(b"fpgm")).is_none());

    let options = Options { hinting: crate::HintingOptions { autohint: true, ..Default::default() }, ..Options::default() };
    let out = compile_ttf(&f, &options).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("autohint")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    for name in ["o", "H"] {
        assert!(!program(&font, gid(&out, name)).is_empty(), "{name} has a program");
    }
    runs_everywhere(&out.bytes);
    let outlines = font.outline_glyphs();
    let target = Intent::Smooth { mode: SmoothMode::Normal, symmetric_rendering: true, preserve_linear_metrics: false };
    for ppem in 9..=30u16 {
        let instance = HintingInstance::new(
            &outlines,
            Size::new(f32::from(ppem)),
            LocationRef::default(),
            HintingOptions { engine: Engine::Interpreter, target },
        )
        .unwrap();
        for name in ["o", "H"] {
            let mut e = Extent::default();
            outlines.get(gid(&out, name)).unwrap().draw(DrawSettings::hinted(&instance, true), &mut e).unwrap();
            let (lo, hi) = e.0.unwrap();
            for y in [lo, hi] {
                assert!((y - y.round()).abs() < 0.02, "{name} at {ppem} PPM: an extreme at {y} is off the pixel grid");
            }
        }
    }

    // Manual instructions give way to the autohinter.
    let manual = compile_ttf(&hinted_font(), &options).unwrap();
    assert!(manual.warnings.iter().any(|w| w.contains("replaced the manual instructions")), "{:?}", manual.warnings);
}

/// [`hinted_font`] with glyphs `I` and `J`, plain rectangles holding their own programs.
fn with_own_programs(i: &str, j: &str) -> Font {
    let mut f = hinted_font();
    for (name, text) in [("I", i), ("J", j)] {
        let mut g = f.glyph_mut(name).unwrap();
        g.layers[0].width = 300.0;
        g.layers[0].shapes.push(Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)));
        fontcraft_hinting::imported::set_imported_program(&mut g.layers[0], text.into());
    }
    f
}

#[test]
fn own_programs_are_kept_when_the_instructor_tables_cannot_be_built() {
    let mut f = with_own_programs("SVTCA[0]", "SVTCA[1]");
    std::sync::Arc::make_mut(&mut f.info).params.0.push(Param::new(fontcraft_hinting::settings::TTF_ZONES, Value::String(Str::new("bad"))));
    let out = compile_ttf(&f, &Options::default()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("family TT zones cannot be read")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert_eq!(program(&font, gid(&out, "I")), [0x00]);
    assert_eq!(program(&font, gid(&out, "J")), [0x01]);
    assert!(program(&font, gid(&out, "o")).is_empty());
}

#[test]
fn own_programs_that_need_the_original_font_programs_are_left_out() {
    let f = with_own_programs("PUSHB[ ] 0\nCALL[ ]", "SVTCA[0]");
    let out = compile_ttf(&f, &Options::default()).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("`I`") && w.contains("left out")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(program(&font, gid(&out, "I")).is_empty(), "CALL 0 would reach our function 0");
    assert_eq!(program(&font, gid(&out, "J")), [0x00]);
    assert!(!program(&font, gid(&out, "o")).is_empty(), "the Instructor hints are still compiled");
    runs_everywhere(&out.bytes);
}

/// `x` (0 to 500) and `H` (0 to 700) in a new font, whose `xHeight` zone at 500 has an overshoot
/// of 16, with `options` and `control` as its TTFAutohint options and control instructions.
fn x_and_h(options: &str, control: &str) -> Font {
    let mut f = Font::new("Scale Sans");
    for (name, right, top) in [("x", 450.0, 500.0), ("H", 550.0, 700.0)] {
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, right, top)));
    }
    let params = &mut std::sync::Arc::make_mut(&mut f.info).params.0;
    for (name, text) in
        [(fontcraft_hinting::settings::TTF_AUTOHINT_OPTIONS, options), (fontcraft_hinting::settings::TTF_AUTOHINT_CONTROL_INSTRUCTIONS, control)]
    {
        if !text.is_empty() {
            params.push(Param::new(name, Value::String(Str::new(text))));
        }
    }
    f
}

fn autohinted() -> Options {
    Options { hinting: crate::HintingOptions { autohint: true, ..Default::default() }, ..Options::default() }
}

/// The lowest and highest y of each named glyph, hinted at `ppem` by the interpreter.
fn hinted_extents(out: &Compiled, names: &[&str], ppem: u16, target: Intent) -> Vec<(f32, f32)> {
    let font = FontRef::new(&out.bytes).unwrap();
    let outlines = font.outline_glyphs();
    let instance =
        HintingInstance::new(&outlines, Size::new(f32::from(ppem)), LocationRef::default(), HintingOptions { engine: Engine::Interpreter, target })
            .unwrap();
    names
        .iter()
        .map(|name| {
            let mut e = Extent::default();
            outlines.get(gid(out, name)).unwrap().draw(DrawSettings::hinted(&instance, true), &mut e).unwrap();
            e.0.unwrap()
        })
        .collect()
}

const SMOOTH: Intent = Intent::Smooth { mode: SmoothMode::Normal, symmetric_rendering: true, preserve_linear_metrics: false };

#[test]
fn autohint_scales_the_zones_so_the_x_height_lands_on_the_grid_as_ttfautohint_does() {
    // The tops of `x` and `H` at 6 to 30 PPM in ttfautohint 1.8.4's output for this font (an `o`
    // and an `O` added to give it the same zones), with its default options, with `-X -` and
    // with `-x 0`, rendered by FreeType's TrueType interpreter.
    type Case = (&'static str, Option<u16>, bool, [i32; 25], [i32; 25]);
    let cases: [Case; 3] = [
        (
            "",
            Some(14),
            true,
            [3, 4, 4, 5, 5, 6, 7, 7, 8, 8, 8, 9, 9, 10, 10, 11, 11, 12, 13, 13, 14, 14, 15, 15, 15],
            [4, 5, 5, 7, 7, 8, 9, 9, 11, 11, 11, 12, 12, 14, 14, 15, 15, 16, 18, 18, 19, 19, 20, 20, 22],
        ),
        (
            "--x-height-snapping-exceptions=-",
            Some(14),
            false,
            [3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15],
            [4, 5, 6, 6, 7, 8, 8, 9, 10, 11, 11, 12, 13, 13, 14, 15, 15, 16, 17, 18, 18, 19, 20, 20, 21],
        ),
        (
            "-x 0",
            None,
            true,
            [3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 13, 13, 14, 14, 15, 15, 15],
            [4, 5, 5, 7, 7, 8, 8, 9, 10, 11, 11, 12, 12, 14, 14, 15, 15, 16, 18, 18, 19, 19, 20, 20, 22],
        ),
    ];
    for (options, limit, snapping, xs, hs) in cases {
        let out = compile_ttf(&x_and_h(options, ""), &autohinted()).unwrap();
        assert!(!out.warnings.iter().any(|w| w.contains("TTFAutohint")), "{:?}", out.warnings);
        runs_everywhere(&out.bytes);
        let mut checked = 0;
        for (i, ppem) in (6..=30u16).enumerate() {
            // Sizes where a value lies within 0.03 px of a rounding boundary are left out: the
            // interpreters' fixed-point steps can tip them either way.
            let s = 516.0 * f64::from(ppem) / 1000.0;
            let up = if limit.is_some_and(|l| ppem <= l) { 13.0 / 16.0 } else { 5.0 / 8.0 };
            if snapping && !(0.03..=0.97).contains(&(s + up).fract()) {
                continue;
            }
            let factor = if snapping { (s + up).floor() / s } else { 1.0 };
            let tops = hinted_extents(&out, &["x", "H"], ppem, SMOOTH);
            for ((units, want), (bottom, top)) in [(500.0, xs[i]), (700.0, hs[i])].into_iter().zip(tops) {
                assert_eq!(bottom, 0.0, "`{options}` at {ppem} PPM");
                let v: f64 = units * f64::from(ppem) / 1000.0 * factor;
                if (v.fract() - 0.5).abs() < 0.03 {
                    continue;
                }
                assert_eq!(top, want as f32, "`{options}` at {ppem} PPM: the row of {units}");
                checked += 1;
            }
        }
        assert!(checked >= 35, "`{options}`: {checked} rows compared");
    }
}

#[test]
fn autohint_hints_a_glyph_with_components_as_its_outline_and_leaves_composites_alone() {
    let mut f = x_and_h("", "");
    f.add_glyph("xdot", Vec::new(), 600.0).unwrap();
    f.add_glyph("xx", Vec::new(), 600.0).unwrap();
    {
        let mut g = f.glyph_mut("xdot").unwrap();
        g.layers[0].shapes.push(Shape::Component(fontcraft_model::Component::new("x", 0.0, 0.0)));
        g.layers[0].shapes.push(Shape::Path(Path::rect(200.0, 600.0, 300.0, 700.0)));
    }
    f.glyph_mut("xx").unwrap().layers[0].shapes.push(Shape::Component(fontcraft_model::Component::new("x", 0.0, 0.0)));
    let out = compile_ttf(&f, &autohinted()).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(!program(&font, gid(&out, "xdot")).is_empty(), "the mixed glyph is hinted as its outline");
    assert!(program(&font, gid(&out, "xx")).is_empty(), "a composite gets no program of its own");
    runs_everywhere(&out.bytes);
    for ppem in 9..=30u16 {
        let e = hinted_extents(&out, &["x", "H", "xdot", "xx"], ppem, SMOOTH);
        let (x, h, xdot, xx) = (e[0], e[1], e[2], e[3]);
        assert_eq!(xdot, (0.0, h.1), "{ppem} PPM: the dot's top on the cap-height row, the base on the baseline");
        assert_eq!(xx, x, "{ppem} PPM: the composite draws its hinted component");
    }
}

#[test]
fn control_instruction_point_shifts_move_points_after_iup() {
    let shifted = compile_ttf(&x_and_h("", "x point 0-3 yshift 1 @ 12"), &autohinted()).unwrap();
    assert!(!shifted.warnings.iter().any(|w| w.contains("control")), "{:?}", shifted.warnings);
    let plain = compile_ttf(&x_and_h("", ""), &autohinted()).unwrap();
    runs_everywhere(&shifted.bytes);
    let font = FontRef::new(&shifted.bytes).unwrap();
    let text = fontcraft_hinting::ttasm::disassemble(program(&font, gid(&shifted, "x"))).unwrap();
    let iup = text.rfind("IUP").unwrap();
    assert!(text.get(iup..).unwrap().contains("SVTCA[0]"), "the shifts follow IUP: {text}");
    for ppem in [11u16, 12, 13] {
        let (a, b) = (hinted_extents(&shifted, &["x"], ppem, Intent::Mono)[0], hinted_extents(&plain, &["x"], ppem, Intent::Mono)[0]);
        let lift = if ppem == 12 { 1.0 } else { 0.0 };
        assert_eq!(a, (b.0 + lift, b.1 + lift), "{ppem} PPM");
    }
}

/// Gives `f` an `X` of two crossing bars on every layer. Its main-master layer snaps two extra
/// nodes, where bar 0's bottom edge (ending at node 1) meets bar 1's right edge (ending at node
/// 2) at (200, 100) and where bar 0's top edge meets bar 1's left edge at (100, 200), and one
/// regular node.
fn add_crossing(f: &mut Font) {
    let mut x = f.glyph_mut("X").unwrap();
    for layer in x.layers.iter_mut() {
        layer.width = 400.0;
        layer.shapes.push(Shape::Path(Path::rect(0.0, 100.0, 300.0, 200.0)));
        layer.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 200.0, 300.0)));
    }
    let crossing = |p1, n1, p2, n2| TtPoint::Intersection { a: NodeRef::new(p1, n1), b: NodeRef::new(p2, n2) };
    let snap = |p| instruction(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, p, None, None);
    let list = vec![snap(crossing(0, 1, 1, 2)), snap(crossing(0, 3, 1, 0)), snap(node(1, 2))];
    set_tt_instructions(&mut x.layers[0], &list).unwrap();
}

#[test]
fn a_static_export_moves_an_instruction_on_an_extra_node_to_the_crossing_point() {
    use fontcraft_hinting::PointMap;
    let mut f = hinted_font();
    add_crossing(&mut f);
    let out = compile_ttf(&f, &Options::default()).unwrap();
    assert!(!out.warnings.iter().any(|w| w.contains("left out")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(!program(&font, gid(&out, "X")).is_empty());
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    let Some(RawGlyph::Simple(x)) = loca.get_glyf(gid(&out, "X"), &glyf).unwrap() else { panic!("X is simple") };
    let points: Vec<_> = x.points().collect();
    let map = crate::points::point_indexes(&f, &Target::Master("m01".into()), "X", &Options::default()).unwrap().unwrap();
    for (a, b, at) in [(NodeRef::new(0, 1), NodeRef::new(1, 2), (200, 100)), (NodeRef::new(0, 3), NodeRef::new(1, 0), (100, 200))] {
        let p = points[usize::from(map.intersection(a, b).unwrap())];
        assert_eq!((p.x, p.y, p.on_curve), (at.0, at.1, true), "the union's corner at the crossing");
    }
    runs_everywhere(&out.bytes);
}

#[test]
fn a_variable_export_drops_instructions_on_extra_nodes_with_one_warning_per_glyph() {
    let mut f = two_masters();
    add_crossing(&mut f);
    let var = crate::var::compile_variable(&f, None, &Options::default(), &crate::var::VariableOptions::default()).unwrap();
    let out = &var.compiled;
    let about_x: Vec<&String> = out.warnings.iter().filter(|w| w.contains("`X`")).collect();
    assert_eq!(about_x.len(), 1, "{:?}", out.warnings);
    assert!(about_x[0].contains("2 TrueType instructions on extra nodes"), "{about_x:?}");
    let font = FontRef::new(&out.bytes).unwrap();
    assert!(!program(&font, gid(out, "X")).is_empty(), "the Snap on a regular node is kept: {:?}", out.warnings);
    runs_everywhere(&out.bytes);
}
