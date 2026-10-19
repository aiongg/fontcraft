//! Subroutines in static CFF exports (row `compile.cffSubroutinisation`), on fonts built here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_cff::subr::{CharstringSet, charstrings_of_table, desubroutinize};
use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Param, Path, Shape};
use skrifa::FontRef;
use skrifa::raw::types::Tag;

use crate::{Flavour, Options, compile};

/// A–Z drawn as the same two bars, so their charstrings repeat.
fn font() -> Font {
    let mut f = Font::new("Subr Sans");
    for c in 'A'..='Z' {
        let name = c.to_string();
        for (x0, x1) in [(50.0, 250.0), (300.0, 500.0)] {
            f.glyph_mut(&name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x0, 0.0, x1, 700.0)));
        }
    }
    f
}

fn cff(options: &Options, f: &Font) -> CharstringSet {
    let out = compile(f, &f.default_target().unwrap(), Flavour::Cff, options).unwrap();
    let font = FontRef::new(&out.bytes).unwrap();
    let data = font.table_data(Tag::new(b"CFF ")).unwrap();
    charstrings_of_table(data.as_bytes(), false).unwrap()
}

fn subrs(set: &CharstringSet) -> usize {
    set.global_subrs.len() + set.local_subrs.iter().map(Vec::len).sum::<usize>()
}

#[test]
fn a_cff_export_is_subroutinised_unless_the_source_or_the_options_say_no() {
    let f = font();
    let on = cff(&Options::default(), &f);
    let off = cff(&Options { subroutinize: Some(false), ..Options::default() }, &f);
    assert!(subrs(&on) > 0, "the repeated bars went into subroutines");
    assert_eq!(subrs(&off), 0);
    assert!(on.index_bytes() < off.index_bytes(), "{} >= {}", on.index_bytes(), off.index_bytes());
    assert_eq!(desubroutinize(&on).unwrap().charstrings, off.charstrings, "expanded, the charstrings are the plain ones");

    let mut disabled = font();
    Arc::make_mut(&mut disabled.info).params.0.push(Param::new("Disable Subroutines", Value::Bool(true)));
    assert_eq!(subrs(&cff(&Options::default(), &disabled)), 0, "Disable Subroutines turns them off");
    assert!(subrs(&cff(&Options { subroutinize: Some(true), ..Options::default() }, &disabled)) > 0, "the option wins over the parameter");
}
