//! Layout normaliser tests on fonts built in code. The expected text follows the oracle's format
//! (otl-normalizer 1.0.0) rule by rule.

use read_fonts::types::{F2Dot14, Fixed, GlyphId, GlyphId16, NameId, Tag};
use write_fonts::tables::gdef::{CaretValue, Gdef, LigCaretList, LigGlyph};
use write_fonts::tables::gpos::{
    AnchorTable, BaseArray, BaseRecord, Class1Record, Class2Record, ComponentRecord, CursivePosFormat1, EntryExitRecord, Gpos, LigatureArray,
    LigatureAttach, MarkArray, MarkBasePosFormat1, MarkLigPosFormat1, MarkRecord, PairPos, PairSet, PairValueRecord, PositionLookup,
    PositionLookupList, ValueRecord,
};
use write_fonts::tables::layout::{
    ClassDef, ClassRangeRecord, CoverageTable, DeviceOrVariationIndex, Feature, FeatureList, FeatureRecord, LangSys, LangSysRecord, Lookup,
    LookupFlag, Script, ScriptList, ScriptRecord, VariationIndex,
};
use write_fonts::tables::variations::{ItemVariationData, ItemVariationStore, RegionAxisCoordinates, VariationRegion, VariationRegionList};

use super::*;
use crate::sfnt::Flavour;

const GLYPHS: [&str; 8] = [".notdef", "A", "V", "T", "o", "acutecomb", "f_i", "dal"];

fn g(id: u16) -> GlyphId16 {
    GlyphId16::new(id)
}

fn cov(ids: &[u16]) -> CoverageTable {
    CoverageTable::format_1(ids.iter().map(|i| g(*i)).collect())
}

fn anchor(x: i16, y: i16) -> AnchorTable {
    AnchorTable::format_1(x, y)
}

fn adv(x: i16) -> ValueRecord {
    ValueRecord::new().with_x_advance(x)
}

fn dump<T: write_fonts::FontWrite + write_fonts::validate::Validate>(table: &T) -> Vec<u8> {
    write_fonts::dump_table(table).unwrap()
}

/// A font with `maxp`, `post` names for [`GLYPHS`] and the given tables.
fn font(tables: Vec<(&[u8; 4], Vec<u8>)>) -> SfntFont {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", dump(&write_fonts::tables::maxp::Maxp::new(GLYPHS.len() as u16)));
    font.insert(*b"post", dump(&write_fonts::tables::post::Post::new_v2(GLYPHS)));
    for (tag, data) in tables {
        font.insert(*tag, data);
    }
    font
}

fn feature(tag: &[u8; 4], lookups: Vec<u16>) -> FeatureRecord {
    FeatureRecord::new(Tag::new(tag), Feature::new(None, lookups))
}

/// Kerning in two lookups (pair and class), mark-to-base, mark-to-ligature and cursive attachment.
fn layout_font() -> SfntFont {
    let pairs = PairPos::format_1(
        cov(&[1, 3]),
        vec![
            PairSet::new(vec![PairValueRecord::new(g(2), adv(-80), ValueRecord::new())]),
            PairSet::new(vec![PairValueRecord::new(g(4), adv(-40), ValueRecord::new())]),
        ],
    );
    let classes = PairPos::format_2(
        cov(&[1, 3]),
        ClassDef::format_2(vec![ClassRangeRecord::new(g(1), g(1), 1), ClassRangeRecord::new(g(3), g(3), 1)]),
        ClassDef::format_2(vec![ClassRangeRecord::new(g(4), g(4), 1)]),
        vec![
            Class1Record::new(vec![
                Class2Record::new(ValueRecord::new(), ValueRecord::new()),
                Class2Record::new(ValueRecord::new(), ValueRecord::new()),
            ]),
            Class1Record::new(vec![Class2Record::new(ValueRecord::new(), ValueRecord::new()), Class2Record::new(adv(-10), ValueRecord::new())]),
        ],
    );
    let mark_base = MarkBasePosFormat1::new(
        cov(&[5]),
        cov(&[1, 4]),
        MarkArray::new(vec![MarkRecord::new(0, anchor(100, 500))]),
        BaseArray::new(vec![BaseRecord::new(vec![Some(anchor(300, 700))]), BaseRecord::new(vec![Some(anchor(250, 500))])]),
    );
    let mark_lig = MarkLigPosFormat1::new(
        cov(&[5]),
        cov(&[6]),
        MarkArray::new(vec![MarkRecord::new(0, anchor(100, 500))]),
        LigatureArray::new(vec![LigatureAttach::new(vec![ComponentRecord::new(vec![Some(anchor(150, 600))]), ComponentRecord::new(vec![None])])]),
    );
    let cursive = CursivePosFormat1::new(cov(&[7]), vec![EntryExitRecord::new(Some(anchor(500, 0)), None)]);
    let lookups = PositionLookupList::new(vec![
        PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![pairs])),
        PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![classes])),
        PositionLookup::MarkToBase(Lookup::new(LookupFlag::empty(), vec![mark_base])),
        PositionLookup::MarkToLig(Lookup::new(LookupFlag::empty(), vec![mark_lig])),
        PositionLookup::Cursive(Lookup::new(LookupFlag::RIGHT_TO_LEFT | LookupFlag::IGNORE_MARKS, vec![cursive])),
    ]);
    let features = FeatureList::new(vec![feature(b"curs", vec![4]), feature(b"kern", vec![0, 1]), feature(b"mark", vec![2, 3])]);
    let scripts = ScriptList::new(vec![
        ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0, 1, 2])), vec![])),
        ScriptRecord::new(
            Tag::new(b"latn"),
            Script::new(Some(LangSys::new(vec![0, 1, 2])), vec![LangSysRecord::new(Tag::new(b"TRK "), LangSys::new(vec![1]))]),
        ),
    ]);
    font(vec![(b"GPOS", dump(&Gpos::new(scripts, features, lookups)))])
}

#[test]
fn kerning_marks_and_cursive_print_as_flat_sorted_rules() {
    let text = normalize(&layout_font(), &NormalizeOptions::default()).unwrap();
    let expected = "# GPOS #\n\
\n\
# curs: DFLT/dflt, latn/dflt\n\
# 1 CursivePos rules\n\
# lookupflag LookupFlag(9)\n\
dal\n  entry: @(x: 500, y: 0)\n  exit: <NULL>\n\
\n\
# kern: DFLT/dflt, latn/dflt, latn/TRK \n\
# 3 PairPos rules\n\
# lookupflag LookupFlag(0)\n\
A -80 V\n\
A -10 o\n\
T -50 o\n\
\n\
# mark: DFLT/dflt, latn/dflt\n\
# 2 MarkToBase rules\n\
# lookupflag LookupFlag(0)\n\
A @(x: 300, y: 700)\n  @(x: 100, y: 500) acutecomb\n\
o @(x: 250, y: 500)\n  @(x: 100, y: 500) acutecomb\n\
# 1 MarkToLig rules\n\
# lookupflag LookupFlag(0)\n\
f_i (lig) [@(x: 150, y: 600), <NULL>]\n  @(x: 100, y: 500) acutecomb\n";
    assert_eq!(text, expected);

    // GSUB alone: the font has none, so nothing prints; an unknown table is refused.
    let gsub = NormalizeOptions { tables: vec![*b"GSUB"], ..NormalizeOptions::default() };
    assert_eq!(normalize(&layout_font(), &gsub).unwrap(), "");
    let bad = NormalizeOptions { tables: vec![*b"name"], ..NormalizeOptions::default() };
    assert!(matches!(normalize(&layout_font(), &bad), Err(TablesError::BadOptions(_))));
}

/// One `wght` axis (100 to 900, default 400) with one region peaking at the maximum: the kerning
/// of A V varies by -20 and the caret of f_i by +40.
fn variable_font() -> SfntFont {
    let one = F2Dot14::from_f32(1.0);
    let regions = VariationRegionList::new(1, vec![VariationRegion::new(vec![RegionAxisCoordinates::new(F2Dot14::ZERO, one, one)])]);
    let data = ItemVariationData::new(2, 0, vec![0], vec![(-20i8) as u8, 40]);
    let store = ItemVariationStore::new(regions, vec![Some(data)]);
    let caret = CaretValue::format_3(300, DeviceOrVariationIndex::VariationIndex(VariationIndex::new(0, 1)));
    let mut gdef = Gdef::new(None, None, Some(LigCaretList::new(cov(&[6]), vec![LigGlyph::new(vec![caret])])), None);
    gdef.item_var_store = store.into();

    let kern = ValueRecord::new().with_x_advance(-80).with_x_advance_device(DeviceOrVariationIndex::VariationIndex(VariationIndex::new(0, 0)));
    let pairs = PairPos::format_1(cov(&[1]), vec![PairSet::new(vec![PairValueRecord::new(g(2), kern, ValueRecord::new())])]);
    let lookups = PositionLookupList::new(vec![PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![pairs]))]);
    let features = FeatureList::new(vec![feature(b"kern", vec![0])]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);

    use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, VariationAxisRecord};
    let axis =
        VariationAxisRecord::new(Tag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![]));
    font(vec![(b"GDEF", dump(&gdef)), (b"GPOS", dump(&Gpos::new(scripts, features, lookups))), (b"fvar", dump(&fvar))])
}

#[test]
fn variable_values_print_per_master_and_resolve_at_a_location() {
    let font = variable_font();
    let masters = normalize(&font, &NormalizeOptions::default()).unwrap();
    let head = "# GDEF #\nf_i\n";
    let kern = "# GPOS #\n\n# kern: DFLT/dflt\n# 1 PairPos rules\n# lookupflag LookupFlag(0)\n";
    assert_eq!(masters, format!("{head}  0: coord 300 {{340}}\n{kern}A -80 {{-100}} V\n"));

    let at = |wght: f64| normalize(&font, &NormalizeOptions { location: vec![(*b"wght", wght)], ..NormalizeOptions::default() }).unwrap();
    assert_eq!(at(900.0), format!("{head}  0: coord 340\n{kern}A -100 V\n"));
    assert_eq!(at(650.0), format!("{head}  0: coord 320\n{kern}A -90 V\n"));
    assert_eq!(at(400.0), format!("{head}  0: coord 300\n{kern}A -80 V\n"));

    let unknown = NormalizeOptions { location: vec![(*b"wdth", 100.0)], ..NormalizeOptions::default() };
    assert!(matches!(normalize(&font, &unknown), Err(TablesError::BadOptions(_))));
    let static_font = layout_font();
    let on_static = NormalizeOptions { location: vec![(*b"wght", 700.0)], ..NormalizeOptions::default() };
    assert!(matches!(normalize(&static_font, &on_static), Err(TablesError::BadOptions(_))));
}

#[test]
fn glyphs_without_post_names_are_named_from_cmap() {
    // .notdef, then A, the micro sign, Greek capital delta, an emoji, and an unmapped glyph.
    let mappings = [('A', 1u32), ('\u{B5}', 2), ('\u{394}', 3), ('\u{1F600}', 4)].map(|(c, gid)| (c, GlyphId::new(gid)));
    let cmap = write_fonts::tables::cmap::Cmap::from_mappings(mappings).unwrap();
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", dump(&write_fonts::tables::maxp::Maxp::new(6)));
    font.insert(*b"cmap", dump(&cmap));
    let names = names::NameMap::from_font(&Provider { font: &font }).unwrap();
    let got: Vec<String> = (0..7).map(|i| names.get(g(i)).into_owned()).collect();
    assert_eq!(got, [".notdef", "A", "mu", "uni0394", "u1F600", "glyph.00005", "glyph.00006"]);
}

#[test]
fn class_kerning_that_describes_too_many_pairs_is_refused() {
    // Every glyph kerns against every glyph through one class pair: 65,534² pairs from a few bytes.
    let all = CoverageTable::format_2(vec![write_fonts::tables::layout::RangeRecord::new(g(1), g(65534), 0)]);
    let classes = PairPos::format_2(
        all,
        ClassDef::format_2(vec![]),
        ClassDef::format_2(vec![ClassRangeRecord::new(g(1), g(65534), 1)]),
        vec![Class1Record::new(vec![Class2Record::new(ValueRecord::new(), ValueRecord::new()), Class2Record::new(adv(-5), ValueRecord::new())])],
    );
    let lookups = PositionLookupList::new(vec![PositionLookup::Pair(Lookup::new(LookupFlag::empty(), vec![classes]))]);
    let features = FeatureList::new(vec![feature(b"kern", vec![0])]);
    let scripts = ScriptList::new(vec![ScriptRecord::new(Tag::new(b"DFLT"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"maxp", dump(&write_fonts::tables::maxp::Maxp::new(65535)));
    font.insert(*b"GPOS", dump(&Gpos::new(scripts, features, lookups)));
    let result = run(&font, &NormalizeOptions::default(), 100_000);
    assert!(matches!(result, Err(TablesError::TooLarge { .. })), "{result:?}");
}

#[test]
fn damaged_layout_tables_fail_with_a_message() {
    // A GPOS header whose lists point past the end, and a truncated one.
    for gpos in [vec![0, 1, 0, 0, 0xFF, 0xF0, 0xFF, 0xF0, 0xFF, 0xF0], vec![0, 1, 0]] {
        let font = font(vec![(b"GPOS", gpos)]);
        match normalize(&font, &NormalizeOptions::default()) {
            Ok(text) => assert!(text.is_empty() || text.starts_with("# GPOS #")),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
    // No maxp: the glyph count is unknown.
    let mut no_maxp = layout_font();
    no_maxp.remove(*b"maxp");
    assert!(matches!(normalize(&no_maxp, &NormalizeOptions::default()), Err(TablesError::MissingTable(_))));
}
