//! The Apple tables, `DSIG`, `FFTM`, `LTSH`, `VDMX`, `COLR` version 1 and `cmap` format 2
//! against `ttx`'s compiler (M12.7c),
//! with the inputs and checks of [`crate::ttx::read_bitmap::tests`].

use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::read_bitmap::tests::{assert_reads_as_ttx, survives_missing_parts};
use crate::ttx::{DumpOptions, dump};

const APPLE: (&str, &str) = (include_str!("testdata/apple.ttx"), include_str!("testdata/apple.digest"));
const APPLE2: (&str, &str) = (include_str!("testdata/apple2.ttx"), include_str!("testdata/apple2.digest"));
const COLR1: (&str, &str) = (include_str!("testdata/colr1.ttx"), include_str!("testdata/colr1.digest"));
const MISC: (&str, &str) = (include_str!("testdata/misc.ttx"), include_str!("testdata/misc.digest"));
const CMAP2: (&str, &str) = (include_str!("testdata/cmap2.ttx"), include_str!("testdata/cmap2.digest"));

#[test]
fn apple_tables_compile_as_ttx_compiles_and_dump_back() {
    assert_reads_as_ttx(APPLE);
    assert_reads_as_ttx(APPLE2);
}

#[test]
fn signature_timestamp_hinting_and_device_tables_compile_as_ttx_compiles_and_dump_back() {
    assert_reads_as_ttx(MISC);
}

#[test]
fn colr_version_1_compiles_as_ttx_compiles_and_dumps_back() {
    assert_reads_as_ttx(COLR1);
}

#[test]
fn cmap_format_2_compiles_as_ttx_compiles_and_dumps_back() {
    assert_reads_as_ttx(CMAP2);
}

#[test]
fn apple_and_colour_readers_survive_missing_parts() {
    for (text, _) in [APPLE, APPLE2, MISC, COLR1, CMAP2] {
        survives_missing_parts(text);
    }
}

#[test]
fn prop_format_0_is_written_without_a_glyph_order() {
    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"prop", vec![0, 3, 0, 0, 0, 0, 0, 3]);
    let text = dump(&font, &DumpOptions { tables: vec![*b"prop"], strict: true, ..DumpOptions::default() }).unwrap();
    assert!(text.contains("<GlyphProperties Format=\"0\">\n      <DefaultProperties value=\"3\"/>\n    </GlyphProperties>"), "{text}");
}
