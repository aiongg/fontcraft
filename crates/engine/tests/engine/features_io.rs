//! Feature files (`file.importFeatures`, `file.exportFeatures`): a file splits into entries that
//! go after the font's, clashing names get suffixes, includes are inlined from inside the file's
//! folder, the UFO writer's form keeps its flags, an import exported again gives the same
//! entries, kerning exports as a `kern` feature, and hostile files fail with messages.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Features, Font, Snippet};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-features-io-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn snippet(name: &str, code: &str) -> Snippet {
    Snippet { name: name.into(), code: code.into(), ..Default::default() }
}

fn session(font: Font) -> Session {
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn features(s: &Session) -> Features {
    Features::clone(&s.font().unwrap().features)
}

fn names(list: &[Snippet]) -> Vec<&str> {
    list.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn import_features_adds_entries_after_the_fonts_and_renames_clashing_classes() {
    let mut f = Font::new("Features Test");
    {
        let fx = Arc::make_mut(&mut f.features);
        fx.prefixes.push(snippet("Prefix", "languagesystem DFLT dflt;"));
        fx.classes.push(snippet("Upper", "A B"));
        fx.features.push(snippet("liga", "sub f i by fi;"));
    }
    let mut s = session(f);
    let steps = s.doc().unwrap().history.undo.len();
    let text = "# Prefix: Lookups\nlookup L { sub a by b; } L;\n\n@Upper = [A B C];\n@Lower = [a b];\n\nfeature liga {\n  sub f l by fl;\n} liga;\n\nfeature smcp {\n  sub @Lower by @Upper; # @Upper\n} smcp;\n\n# closing remark\n";
    let r = run(&mut s, "file.importFeatures", json!({"text": text}));
    assert_eq!(r["source"], "text");
    assert_eq!(r["prefixes"], json!(["Lookups", "Prefix2"]));
    assert_eq!(r["classes"], json!(["Upper2", "Lower"]));
    assert_eq!(r["features"], json!(["liga", "smcp"]));
    assert_eq!(r["counts"], json!({"prefixes": 2, "classes": 2, "features": 2}));
    assert_eq!(r["renamed"], json!([{"kind": "class", "from": "Upper", "to": "Upper2"}, {"kind": "prefix", "from": "Prefix", "to": "Prefix2"}]));
    assert_eq!(r["sharedTags"], json!(["liga"]));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");

    let fx = features(&s);
    assert_eq!(names(&fx.prefixes), ["Prefix", "Lookups", "Prefix2"]);
    assert_eq!(fx.prefixes[1].code, "lookup L { sub a by b; } L;");
    assert_eq!(fx.prefixes[2].code, "# closing remark");
    assert_eq!(names(&fx.classes), ["Upper", "Upper2", "Lower"]);
    assert_eq!(fx.classes[1].code, "A B C");
    assert_eq!(names(&fx.features), ["liga", "liga", "smcp"], "a feature keeps its tag");
    assert_eq!(fx.features[1].code, "  sub f l by fl;");
    assert_eq!(fx.features[2].code, "  sub @Lower by @Upper2; # @Upper", "references follow the renamed class; comments do not");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&features(&s).features), ["liga"]);
}

#[test]
fn import_features_keeps_the_flags_of_the_ufo_writers_form() {
    let mut s = session(Font::new("Features Test"));
    let text = "@Foo = [ a b\n];\n\n# Prefix: Languagesystems\n# automatic\nlanguagesystem DFLT dflt;\n\nfeature liga {\n# disabled\n#sub a b by c;\n} liga;\n";
    run(&mut s, "file.importFeatures", json!({"text": text}));
    let fx = features(&s);
    assert_eq!(names(&fx.classes), ["Foo"]);
    assert_eq!(fx.classes[0].code, "a b");
    assert!(fx.prefixes[0].automatic && fx.prefixes[0].name == "Languagesystems");
    assert!(fx.features[0].disabled);
    assert_eq!(fx.features[0].code, "sub a b by c;");
}

#[test]
fn import_features_then_export_then_import_gives_the_same_entries() {
    let text = "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\n\n@Upper = [A B C];\n\nfeature liga {\n  sub f i by f_i;\n} liga;\n\nfeature ss01 {\n  sub a by a.ss01;\n} ss01;\n";
    let mut first = session(Font::new("Features Test"));
    run(&mut first, "file.importFeatures", json!({"text": text}));
    let exported = run(&mut first, "file.exportFeatures", json!({"format": "fea"}));
    let written = exported["text"].as_str().unwrap();
    assert!(written.contains("@Upper") && written.contains("feature liga") && written.contains("languagesystem latn dflt;"), "{written}");
    assert_eq!(exported["entries"], 4);

    let mut second = session(Font::new("Features Test"));
    run(&mut second, "file.importFeatures", json!({"text": written}));
    assert_eq!(features(&second), features(&first), "the exported file imports to the same entries");
}

#[test]
fn import_features_inlines_includes_from_inside_the_files_folder() {
    let dir = temp("includes");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("classes.fea"), "@Upper = [A B];\n").unwrap();
    std::fs::write(dir.join("sub/liga.fea"), "sub f i by f_i;\ninclude(more.fea);\n").unwrap();
    std::fs::write(dir.join("sub/more.fea"), "sub f l by f_l;").unwrap();
    std::fs::write(dir.join("main.fea"), "include(classes.fea);\nfeature liga {\n  include(sub/liga.fea)\n} liga;\n# include(nothing.fea);\n")
        .unwrap();
    let mut s = session(Font::new("Features Test"));
    let r = run(&mut s, "file.importFeatures", json!({"file": dir.join("main.fea")}));
    assert_eq!(r["source"], "file");
    assert_eq!(
        r["includes"],
        json!(["classes.fea", "sub/liga.fea", "sub/more.fea"]),
        "an include is tried in the top file's folder, then the including file's"
    );
    let fx = features(&s);
    assert_eq!(fx.classes[0].code, "A B");
    assert_eq!(fx.features[0].code, "  sub f i by f_i;\nsub f l by f_l;");

    let r = run(&mut s, "file.importFeatures", json!({"text": "include(classes.fea);\nfeature kern { pos A V -10; } kern;\n"}));
    assert_eq!(r["unresolvedIncludes"], json!(["classes.fea"]), "text keeps its includes as written");

    let inner = dir.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("escape.fea"), "include(../classes.fea);\n").unwrap();
    let e = refused(&mut s, "file.importFeatures", json!({"file": inner.join("escape.fea")}));
    assert!(e.contains("outside the folder"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_features_refuses_hostile_files_with_messages() {
    let mut s = session(Font::new("Features Test"));
    for (text, expected) in [
        ("feature liga {\n  sub a by b;\n", "never closed"),
        ("} liga;", "closes no block"),
        ("feature liga { sub a by \"b; } liga;", "never closed"),
        ("anon TEST {\n stuff\n", "never closed"),
        ("", "no feature code"),
        (" \n\t\n", "no feature code"),
        ("include(a.fea", "no closing parenthesis"),
    ] {
        let e = refused(&mut s, "file.importFeatures", json!({"text": text}));
        assert!(e.contains(expected), "{text:?}: {e}");
    }
    let deep = format!("table X {}{} X;", "{".repeat(65), "}".repeat(65));
    let e = refused(&mut s, "file.importFeatures", json!({"text": deep}));
    assert!(e.contains("nested more than 64"), "{e}");
    let big = "#".repeat((16 << 20) + 1);
    let e = refused(&mut s, "file.importFeatures", json!({"text": big}));
    assert!(e.contains("at most"), "{e}");
    let e = refused(&mut s, "file.importFeatures", json!({"text": "x;", "file": "x.fea"}));
    assert!(e.contains("not both"), "{e}");
    let e = refused(&mut s, "file.importFeatures", json!({}));
    assert!(e.contains("give `file` or `text`"), "{e}");

    let dir = temp("hostile");
    std::fs::write(dir.join("a.fea"), "include(b.fea);\n").unwrap();
    std::fs::write(dir.join("b.fea"), "include(a.fea);\n").unwrap();
    std::fs::write(dir.join("missing.fea"), "include(nowhere.fea);\n").unwrap();
    std::fs::write(dir.join("binary.fea"), [0xff_u8, 0xfe, 0x00, 0x80]).unwrap();
    for (file, expected) in
        [("a.fea", "nested more than 50"), ("missing.fea", "does not exist"), ("binary.fea", "not UTF-8"), ("absent.fea", "cannot open")]
    {
        let e = refused(&mut s, "file.importFeatures", json!({"file": dir.join(file)}));
        assert!(e.contains(expected), "{file}: {e}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Groups: right T (T, Tcaron), left o (o, oacute). Master m01's pairs, left to right in stored
/// order: @T–@o −60, T–o −20.4, A–@o −15, A–V −80.5, A–missing 10; right to left: beh-ar–alef-ar
/// −30.
fn kerned_font() -> Font {
    let mut f = Font::new("Features Test");
    for name in ["T", "Tcaron", "o", "oacute", "A", "V", "alef-ar", "beh-ar"] {
        if f.glyph(name).is_none() {
            f.add_glyph(name, vec![], 600.0).unwrap();
        }
    }
    for (glyph, right, left) in [("T", Some("T"), None), ("Tcaron", Some("T"), None), ("o", None, Some("o")), ("oacute", None, Some("o"))] {
        let mut g = f.glyph_mut(glyph).unwrap();
        g.kern_groups.right = right.map(str::to_string);
        g.kern_groups.left = left.map(str::to_string);
    }
    let master = f.masters[0].id.clone();
    let k = Arc::make_mut(&mut f.kerning);
    k.ltr.set(&master, "@MMK_L_T", "@MMK_R_o", -60.0);
    k.ltr.set(&master, "T", "o", -20.4);
    k.ltr.set(&master, "A", "@MMK_R_o", -15.0);
    k.ltr.set(&master, "A", "V", -80.5);
    k.ltr.set(&master, "A", "missing", 10.0);
    k.rtl.set(&master, "beh-ar", "alef-ar", -30.0);
    f
}

#[test]
fn export_features_writes_kerning_as_a_kern_feature() {
    let mut s = session(kerned_font());
    let r = run(&mut s, "file.exportFeatures", json!({"format": "fea-kern"}));
    let expected = "@MMK_L_T = [T Tcaron];\n@MMK_R_o = [o oacute];\n\nfeature kern {\nlookup kern_ltr {\n    pos T o -20;\n    pos A V -80;\n    enum pos A @MMK_R_o -15;\n    pos @MMK_L_T @MMK_R_o -60;\n} kern_ltr;\n\nlookup kern_rtl {\n    pos beh-ar alef-ar <-30 0 -30 0>;\n} kern_rtl;\n} kern;\n";
    assert_eq!(r["text"], expected);
    assert_eq!(r["pairs"], 5);
    assert_eq!(r["classes"], 2);
    assert_eq!(r["warnings"], json!(["1 kerning pairs were left out: they name glyphs the font does not have"]));

    let dir = temp("kern");
    let r = run(&mut s, "file.exportFeatures", json!({"format": "fea-kern", "destination": dir}));
    let path = PathBuf::from(r["path"].as_str().unwrap());
    assert!(path.file_name().unwrap().to_string_lossy().ends_with("-kern.fea"), "{path:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), expected, "the file holds what the text gives");
    let _ = std::fs::remove_dir_all(&dir);

    let mut bare = session(Font::new("Features Test"));
    let e = bare.execute("file.exportFeatures", &json!({"format": "fea-kern"})).unwrap_err().to_string();
    assert!(e.contains("no kerning pairs"), "{e}");
    let e = bare.execute("file.exportFeatures", &json!({"format": "afm"})).unwrap_err().to_string();
    assert!(e.contains("fea or fea-kern"), "{e}");
}
