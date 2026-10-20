//! `mort` and `morx` against `ttx`'s compiler (M12.7d), with the inputs and checks of
//! [`crate::ttx::read_bitmap::tests`]. `morx.ttx` and `mort.ttx` hold one subtable of each kind
//! (rearrangement, contextual, ligature, noncontextual, insertion), compiled by fontTools 4.66.1
//! and dumped by its `ttx`; `morph_crafted.ttx` is the dump of the hand-built tables of
//! `misc/morph_tests.rs` after fontTools compiled them again.

use crate::TablesError;
use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::read_bitmap::tests::{assert_reads_as_ttx, survives_missing_parts};
use crate::ttx::var::tests::assert_truncations_survive;
use crate::ttx::{ImportOptions, import};

const MORX: (&str, &str) = (include_str!("testdata/morx.ttx"), include_str!("testdata/morx.digest"));
const MORT: (&str, &str) = (include_str!("testdata/mort.ttx"), include_str!("testdata/mort.digest"));
const CRAFTED: (&str, &str) = (include_str!("testdata/morph_crafted.ttx"), include_str!("testdata/morph_crafted.digest"));

#[test]
fn every_morx_subtable_kind_compiles_as_ttx_compiles_and_dumps_back() {
    let font = assert_reads_as_ttx(MORX);
    assert_truncations_survive(&font, &[*b"morx"]);
}

#[test]
fn every_mort_subtable_kind_compiles_as_ttx_compiles_and_dumps_back() {
    let font = assert_reads_as_ttx(MORT);
    assert_truncations_survive(&font, &[*b"mort"]);
}

#[test]
fn rebased_components_and_reserved_coverage_bits_compile_as_ttx_compiles() {
    assert_reads_as_ttx(CRAFTED);
}

#[test]
fn morph_readers_survive_missing_parts() {
    for (text, _) in [MORX, MORT, CRAFTED] {
        survives_missing_parts(text);
    }
}

#[test]
fn a_long_insertion_run_is_refused() {
    // Every run of a glyph sequence is indexed, so the work grows with the cube of its length.
    let run: String = (0..400).map(|_| "<CurrentInsertionAction glyph=\"C\"/>").collect();
    let (text, _) = MORX;
    let text = text.replacen("<CurrentInsertionAction glyph=\"C\"/>", &run, 1);
    let err = import(SfntFont::new(Flavour::TrueType), &text, &ImportOptions::default()).unwrap_err();
    assert!(matches!(err, TablesError::TooLarge { .. }), "{err}");
}

#[test]
fn a_transition_missing_for_a_glyph_class_is_an_error() {
    let (text, _) = MORX;
    let cut = text.replacen("<Transition onGlyphClass=\"3\">", "<Transition onGlyphClass=\"30\">", 1);
    assert!(import(SfntFont::new(Flavour::TrueType), &cut, &ImportOptions::default()).is_err());
}
