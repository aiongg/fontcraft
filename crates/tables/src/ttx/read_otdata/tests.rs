//! The structured, variation and colour tables against `ttx`'s compiler. Each input
//! (`testdata/<name>.ttx`) is made of the table elements the writer tests store, under a glyph
//! order; `testdata/<name>.digest` holds the length and SHA-256 of each table fontTools 4.66.1
//! compiles from it (`testdata/oracle.py`). On a mismatch the test prints our bytes as hex.
#![allow(clippy::string_slice)]

use crate::sfnt::{Flavour, SfntFont};
use crate::ttx::oracle_digest::{assert_table, expected};
use crate::ttx::{DumpOptions, ImportOptions, dump, import};

const LAYOUT: (&str, &str) = (include_str!("testdata/layout.ttx"), include_str!("testdata/layout.digest"));
const VARIABLE: (&str, &str) = (include_str!("testdata/variable.ttx"), include_str!("testdata/variable.digest"));
const VAR: (&str, &str) = (include_str!("testdata/var.ttx"), include_str!("testdata/var.digest"));
const COLOR: (&str, &str) = (include_str!("testdata/color.ttx"), include_str!("testdata/color.digest"));
const GVAR: (&str, &str) = (include_str!("testdata/gvar.ttx"), include_str!("testdata/gvar.digest"));
const KERN: (&str, &str) = (include_str!("testdata/kern.ttx"), include_str!("testdata/kern.digest"));
const APPLE_KERN: (&str, &str) = (include_str!("testdata/apple_kern.ttx"), include_str!("testdata/apple_kern.digest"));

fn read(text: &str) -> SfntFont {
    import(SfntFont::new(Flavour::TrueType), text, &ImportOptions::default()).unwrap()
}

fn assert_compiles_as_ttx((text, digest): (&str, &str)) {
    let font = read(text);
    let want = expected(digest);
    for (tag, len, sha) in &want {
        let got = font.get(*tag).unwrap_or_else(|| panic!("no '{}' table", String::from_utf8_lossy(tag)));
        assert_table(tag, got, *len, sha);
    }
    assert_eq!(font.tables.len(), want.len(), "no other tables");
}

#[test]
fn layout_tables_compile_as_ttx_compiles() {
    assert_compiles_as_ttx(LAYOUT);
}

#[test]
fn variable_layout_tables_compile_as_ttx_compiles() {
    assert_compiles_as_ttx(VARIABLE);
}

#[test]
fn variation_stores_compile_as_ttx_compiles() {
    assert_compiles_as_ttx(VAR);
}

#[test]
fn colour_tables_compile_as_ttx_compiles() {
    assert_compiles_as_ttx(COLOR);
}

#[test]
fn glyph_and_control_value_variations_compile_as_ttx_compiles() {
    // The outlines and control values are there for their counts; only the variations count.
    let (text, digest) = GVAR;
    let font = read(text);
    for (tag, len, sha) in &expected(digest) {
        assert_table(tag, font.get(*tag).unwrap(), *len, sha);
    }
}

#[test]
fn kerning_tables_compile_as_ttx_compiles() {
    assert_compiles_as_ttx(KERN);
    assert_compiles_as_ttx(APPLE_KERN);
}

#[test]
fn our_layout_dump_reads_back_to_the_same_tables() {
    for (text, _) in [LAYOUT, VARIABLE, VAR, GVAR, COLOR, KERN, APPLE_KERN] {
        let font = read(text);
        let dumped = dump(&font, &DumpOptions::default()).unwrap();
        let again = read(&dumped);
        for (tag, data) in &font.tables {
            assert_eq!(again.get(*tag), Some(&data[..]), "table '{}'", String::from_utf8_lossy(tag));
        }
    }
}

/// A GSUB with one lookup of type `lookup_type` holding a ligature subtable.
fn gsub(lookup_type: u8, extra: &str) -> String {
    format!(
        r#"<ttFont>
  <GlyphOrder><GlyphID id="0" name=".notdef"/><GlyphID id="1" name="f"/><GlyphID id="2" name="i"/><GlyphID id="3" name="f_i"/></GlyphOrder>
  <GSUB>
    <Version value="0x00010000"/>
    <ScriptList/>
    <FeatureList/>
    <LookupList>
      <Lookup index="0">
        <LookupType value="{lookup_type}"/>
        <LookupFlag value="0"/>
        <LigatureSubst index="0" Format="1">
          <LigatureSet glyph="f"><Ligature components="i" glyph="f_i"/></LigatureSet>
        </LigatureSubst>
        {extra}
      </Lookup>
    </LookupList>
  </GSUB>
</ttFont>"#
    )
}

#[test]
fn a_ligature_lookup_compiles_as_ttx_compiles() {
    let font = read(&gsub(4, ""));
    let hex: String = font.get(*b"GSUB").unwrap().iter().map(|b| format!("{b:02x}")).collect();
    // As fontTools compiles it: the empty script and feature lists are not shared, and the
    // ligature subtable's coverage comes after its ligature set.
    assert_eq!(hex, "00010000000a000c000e00000000000100040004000000010008000100120001000800010004000300020002000100010001");
}

#[test]
fn a_subtable_of_another_lookup_type_is_refused() {
    let err = import(SfntFont::new(Flavour::TrueType), &gsub(1, ""), &ImportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("LookupType"), "{err}");
}

#[test]
fn an_unknown_field_is_refused() {
    let err = import(SfntFont::new(Flavour::TrueType), &gsub(4, "<Bogus value=\"1\"/>"), &ImportOptions::default()).unwrap_err();
    assert!(err.to_string().contains("Bogus"), "{err}");
}
