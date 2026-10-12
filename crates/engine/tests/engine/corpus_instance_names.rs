//! Five corpus sources whose instances carry their own names (family names, PostScript names,
//! style linking), compiled at an instance that sits at a master. The expected records and
//! classes are those fontmake 3.12.1 (glyphsLib 6.15.0) writes for the same instance, read from
//! its output (M7 plan § 5, the name measure). Skipped when the corpus is not fetched
//! (`cargo xtask corpus --fetch`); `FONTCRAFT_CORPUS` names another corpus folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::Target;
use skrifa::FontRef;
use skrifa::raw::TableProvider;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

struct Case {
    source: &'static str,
    /// The instance's name and family (several instances share a name).
    instance: (&'static str, &'static str),
    /// Windows English records by id; absent ones must be absent.
    names: &'static [(u16, &'static str)],
    weight: u16,
    width: u16,
    /// `fsSelection` bits 0 (italic), 5 (bold) and 6 (regular).
    selection: u16,
}

const CASES: &[Case] = &[
    Case {
        source: "inter/src/Inter-Roman.glyphspackage",
        instance: ("Thin", "Inter Display"),
        names: &[
            (1, "Inter Display Thin"),
            (2, "Regular"),
            (4, "Inter Display Thin"),
            (6, "InterDisplay-Thin"),
            (16, "Inter Display"),
            (17, "Thin"),
            (21, "Inter Display"),
            (22, "Thin"),
        ],
        weight: 100,
        width: 5,
        selection: 64,
    },
    Case {
        source: "inter/src/Inter-Italic.glyphspackage",
        instance: ("Italic", "Inter Display"),
        names: &[(1, "Inter Display"), (2, "Italic"), (4, "Inter Display Italic"), (6, "InterDisplay-Italic"), (21, "Inter Display"), (22, "Italic")],
        weight: 400,
        width: 5,
        selection: 1,
    },
    Case {
        source: "fontc/resources/testdata/glyphs3/InstancePostscript.glyphs",
        instance: ("Bold", ""),
        names: &[(1, "WghtVar"), (2, "Bold"), (4, "WghtVar Bold"), (6, "PostBold")],
        weight: 700,
        width: 5,
        selection: 32,
    },
    Case {
        source: "fontc/resources/testdata/glyphs3/WghtVar_Avar.glyphs",
        instance: ("Bold", ""),
        names: &[(1, "WghtVar Bold"), (2, "Regular"), (4, "WghtVar Bold"), (6, "WghtVar-Bold"), (16, "WghtVar"), (17, "Bold")],
        weight: 700,
        width: 5,
        selection: 64,
    },
    Case {
        source: "fontc/resources/testdata/glyphs2/BracketTestFontKerning.glyphs",
        instance: ("Bold Condensed", ""),
        names: &[
            (1, "New Font Condensed"),
            (2, "Bold"),
            (4, "New Font Bold Condensed"),
            (6, "NewFont-BoldCondensed"),
            (16, "New Font"),
            (17, "Bold Condensed"),
        ],
        weight: 700,
        width: 3,
        selection: 32,
    },
];

#[test]
fn instances_with_their_own_names_compile_to_fontmake_s_names() {
    let root = corpus();
    for case in CASES {
        let path = root.join(case.source);
        if !path.exists() {
            eprintln!("skipped: {} is not in the corpus", case.source);
            continue;
        }
        let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
        let (name, family) = case.instance;
        let index = font
            .instances
            .iter()
            .position(|i| i.name == name && (family.is_empty() || i.properties.get("familyNames").and_then(|p| p.default_value()) == Some(family)))
            .unwrap_or_else(|| panic!("{}: no instance {name} {family}", case.source));
        let options = Options { skip_features: true, remove_overlap: false, ..Options::default() };
        let out = compile(&font, &Target::Instance(index), Flavour::TrueType, &options).unwrap();
        let built = FontRef::new(&out.bytes).unwrap();
        let table = built.name().unwrap();
        let data = table.string_data();
        let records: Vec<(u16, String)> = table
            .name_record()
            .iter()
            .filter(|r| r.platform_id() == 3 && r.language_id() == 0x409 && matches!(r.name_id().to_u16(), 1 | 2 | 4 | 6 | 16 | 17 | 21 | 22))
            .map(|r| (r.name_id().to_u16(), r.string(data).unwrap().chars().collect()))
            .collect();
        let expected: Vec<(u16, String)> = case.names.iter().map(|(id, t)| (*id, (*t).to_string())).collect();
        assert_eq!(records, expected, "{} {name}", case.source);
        let os2 = built.os2().unwrap();
        assert_eq!((os2.us_weight_class(), os2.us_width_class()), (case.weight, case.width), "{} {name}", case.source);
        assert_eq!(os2.fs_selection().bits() & 0x61, case.selection, "{} {name}", case.source);
    }
}
