//! Node references in `.glyphs` hints count a closed path's nodes in the file's order, start node
//! last; the model keeps the start node first. Typed hints read from a file resolve to the nodes
//! the file names, and writing them back stores the file's indexes again.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_hinting::{Edge, NodeRef, PsHint, TtInstruction, TtPoint, ps_hints, set_ps_hints, set_tt_instructions, tt_instructions};
use fontcraft_model::{Font, Layer, Target};
use skrifa::FontRef;
use skrifa::GlyphId;
use skrifa::raw::ps::cff::CffFontRef;
use skrifa::raw::ps::cs::CommandSink;
use skrifa::raw::types::{Fixed, Tag};

/// The format 3 sample's `A` outline, a two-node open path, a horizontal stem from (10, 66) to
/// (10, 608), a vertical stem on the open path and a Snap on (10, 608).
const V3: &str = "{
.formatVersion = 3;
fontMaster = (
{
id = m01;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
hints = (
{
horizontal = 1;
origin = (0,0);
target = (0,7);
type = Stem;
},
{
origin = (1,0);
target = (1,1);
type = Stem;
},
{
horizontal = 1;
origin = (0,7);
type = TTSnap;
}
);
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(10,66,l),
(92,-21,o),
(338,-35,o),
(439,66,cs),
(578,205,o),
(522,462,o),
(439,608,c),
(10,608,l)
);
},
{
closed = 0;
nodes = (
(100,0,l),
(200,0,l)
);
}
);
width = 459;
}
);
}
);
unitsPerEm = 1000;
}
";

/// The same outline and stems in format 2.
const V2: &str = "{
.appVersion = \"1192\";
familyName = Old;
fontMaster = (
{
id = m01;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
hints = (
{
horizontal = 1;
origin = \"{0, 0}\";
target = \"{0, 7}\";
type = Stem;
},
{
origin = \"{1, 0}\";
target = \"{1, 1}\";
type = Stem;
}
);
layerId = m01;
paths = (
{
closed = 1;
nodes = (
\"10 66 LINE\",
\"92 -21 OFFCURVE\",
\"338 -35 OFFCURVE\",
\"439 66 CURVE SMOOTH\",
\"578 205 OFFCURVE\",
\"522 462 OFFCURVE\",
\"439 608 CURVE\",
\"10 608 LINE\"
);
},
{
closed = 0;
nodes = (
\"100 0 LINE\",
\"200 0 LINE\"
);
}
);
width = 459;
}
);
}
);
unitsPerEm = 1000;
}
";

fn layer_a(font: &Font) -> Layer {
    font.glyph("A").unwrap().layers[0].clone()
}

/// The `hints = (…);` block of a written file.
fn hints_block(text: &str) -> &str {
    let (_, rest) = text.split_once("hints = (").unwrap();
    rest.split_once("\n);\n").unwrap().0
}

/// Reads `source`, checks where its stems resolve, then writes it back as read and with every
/// hint written afresh from its typed form: both store the file's indexes.
fn check(source: &str) {
    let font = fontcraft_io_glyphs::read(source).unwrap();
    let layer = layer_a(&font);
    let ps: Vec<PsHint> = ps_hints(&layer).unwrap().into_iter().map(|(_, h)| h).collect();
    assert_eq!((ps[0].origin, ps[0].other), (Edge::Node(NodeRef::new(0, 1)), Some(Edge::Node(NodeRef::new(0, 0)))));
    assert_eq!(ps[0].edges(&layer).unwrap(), (66.0, 608.0));
    assert_eq!(ps[1].origin, Edge::Node(NodeRef::new(1, 0)), "an open path's indexes agree");
    assert_eq!(ps[1].edges(&layer).unwrap(), (100.0, 200.0));
    let tt: Vec<TtInstruction> = tt_instructions(&layer).unwrap().into_iter().map(|(_, t)| t).collect();
    if let Some(snap) = tt.first() {
        assert_eq!(snap.target, TtPoint::Node(NodeRef::new(0, 0)));
    }

    let written = fontcraft_io_glyphs::write(&font).unwrap();
    assert_eq!(hints_block(&written), hints_block(source));

    let mut fresh = font.clone();
    {
        let mut g = fresh.glyph_mut("A").unwrap();
        let layer = &mut g.layers[0];
        set_ps_hints(layer, &ps.iter().map(|h| PsHint { source: None, ..h.clone() }).collect::<Vec<_>>()).unwrap();
        set_tt_instructions(layer, &tt.iter().map(|t| TtInstruction { source: None, ..t.clone() }).collect::<Vec<_>>()).unwrap();
    }
    assert_eq!(layer_a(&fresh).info.hints, layer.info.hints, "written from the typed fields alone, the stored values come back");
    let written = fontcraft_io_glyphs::write(&fresh).unwrap();
    assert_eq!(hints_block(&written), hints_block(source));
}

#[test]
fn format3_hint_references_resolve_in_file_order_and_write_back() {
    check(V3);
}

#[test]
fn format2_hint_references_resolve_in_file_order_and_write_back() {
    check(V2);
}

#[derive(Default)]
struct Stems {
    h: Vec<(i32, i32)>,
}

impl CommandSink for Stems {
    fn move_to(&mut self, _: Fixed, _: Fixed) {}
    fn line_to(&mut self, _: Fixed, _: Fixed) {}
    fn curve_to(&mut self, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed, _: Fixed) {}
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.h.push((y.to_i32(), dy.to_i32()));
    }
}

/// The format 3 sample's `A` exports its stem from (10, 66) to (10, 608). Skipped when the
/// corpus is not fetched (`cargo xtask corpus --fetch`).
#[test]
fn the_format3_sample_exports_its_hint_stem_on_the_nodes_the_file_names() {
    let corpus = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    let path = corpus.join("glyphssdk/GlyphsFileFormat/GlyphsFileFormatv3.glyphs");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    };
    let mut font = fontcraft_io_glyphs::read(&text).unwrap();
    // The sample's classes name glyphs it does not hold; the outlines are what this test needs.
    font.features = Default::default();
    let out = compile(&font, &Target::Master("m01".into()), Flavour::Cff, &Options::default()).unwrap();
    let otf = FontRef::new(&out.bytes).unwrap();
    let gid = GlyphId::new(out.glyph_order.iter().position(|n| n == "A").unwrap() as u32);
    let table = otf.table_data(Tag::new(b"CFF ")).unwrap();
    let cff = CffFontRef::new_cff(table.as_bytes(), 0, None).unwrap();
    let subfont = cff.subfont(0, &[]).unwrap();
    let mut sink = Stems::default();
    cff.evaluate_charstring(&subfont, gid, &[], &mut sink).unwrap();
    assert_eq!(sink.h, vec![(66, 608)]);
}
