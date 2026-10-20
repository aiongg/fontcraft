//! `tables.importTtx`: tables compiled from TTX dumps into a compiled font file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-import-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn font(dir: &Path) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({ "destination": ttf, "timestamp": 0 }));
    (s, ttf)
}

fn table(path: &Path, tag: &[u8; 4]) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).unwrap();
    fontcraft_tables::read(&bytes, 0).unwrap().get(*tag).map(<[u8]>::to_vec)
}

#[test]
fn tables_import_ttx_replaces_the_dumped_tables_and_keeps_the_rest() {
    let dir = temp("edit");
    let (mut s, ttf) = font(&dir);
    let original = std::fs::read(&ttf).unwrap();
    let dumped = run(&mut s, "font.dumpTables", json!({ "font": ttf, "tables": ["name", "OS/2"] }));
    let text = dumped["text"].as_str().unwrap();
    let edited_text = text.replace("<usWeightClass value=\"400\"/>", "<usWeightClass value=\"700\"/>");
    assert_ne!(edited_text, text, "the dump holds the weight class");
    let dump = dir.join("patch.ttx");
    std::fs::write(&dump, &edited_text).unwrap();

    let r = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [dump] }));
    let out = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(out, dir.join("Font.edited.ttf"));
    assert!(r["replaced"].as_array().unwrap().contains(&json!("OS/2")), "{r}");
    assert_eq!(r["added"], json!([]), "{r}");
    let os2 = table(&out, b"OS/2").unwrap();
    assert_eq!(u16::from_be_bytes([os2[4], os2[5]]), 700);
    assert_eq!(table(&out, b"glyf"), table(&ttf, b"glyf"), "tables the dump does not hold keep their bytes");
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    assert!(run(&mut s, "tables.checksum", json!({ "font": out }))["ok"].as_bool().unwrap());
}

#[test]
fn tables_import_ttx_refuses_a_bad_dump_and_writes_nothing() {
    let dir = temp("refuse");
    let (mut s, ttf) = font(&dir);
    let dump = dir.join("bad.ttx");
    std::fs::write(&dump, "<?xml version=\"1.0\"?>\n<ttFont><OS_2><version value=\"9\"/></OS_2></ttFont>\n").unwrap();
    assert!(s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [dump] })).is_err());
    assert!(!dir.join("Font.edited.ttf").exists(), "a failed import writes no font");
    std::fs::write(&dump, "not xml").unwrap();
    assert!(s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [dump] })).is_err());
}

#[test]
fn tables_import_ttx_refuses_split_dump_parts_outside_its_folder() {
    let dir = temp("split");
    let (mut s, ttf) = font(&dir);
    std::fs::create_dir_all(dir.join("dump")).unwrap();
    let os2 = run(&mut s, "font.dumpTables", json!({ "font": ttf, "tables": ["OS/2"] }))["text"].as_str().unwrap().to_string();
    std::fs::write(dir.join("os2.ttx"), &os2).unwrap();
    std::fs::write(dir.join("dump").join("os2.ttx"), &os2).unwrap();
    let main = dir.join("dump").join("Font.ttx");
    for src in ["../os2.ttx", dir.join("os2.ttx").to_str().unwrap()] {
        std::fs::write(&main, format!("<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"{src}\"/></ttFont>\n")).unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main] })).unwrap_err();
        assert!(err.to_string().contains("parts must be beside it"), "{src}: {err}");
    }
    std::fs::write(&main, "<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"os2.ttx\"/></ttFont>\n").unwrap();
    let r = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [main] }));
    assert_eq!(r["added"], json!([]), "a part beside the main file is read: {r}");

    // A link beside the main file that leads out of its folder is refused, with or without
    // automation roots; one that stays inside is read.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("os2.ttx"), dir.join("dump").join("out.ttx")).unwrap();
        std::os::unix::fs::symlink(dir.join("dump").join("os2.ttx"), dir.join("dump").join("in.ttx")).unwrap();
        std::fs::write(&main, "<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"out.ttx\"/></ttFont>\n").unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main] })).unwrap_err();
        assert!(err.to_string().contains("outside the dump's folder"), "{err}");
        std::fs::write(&main, "<?xml version=\"1.0\"?>\n<ttFont><OS_2 src=\"in.ttx\"/></ttFont>\n").unwrap();
        run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [main] }));
    }
}

#[test]
fn tables_import_ttx_reads_a_dump_split_by_glyph() {
    let dir = temp("glyphs");
    let (mut s, ttf) = font(&dir);
    let out = dir.join("dump");
    let r = run(&mut s, "font.dumpTables", json!({ "font": ttf, "split": "glyphs", "outputDir": out }));
    let paths: Vec<&str> = r["paths"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    let glyph = out.join("Font._g_l_y_f.A_.ttx");
    assert!(paths.contains(&glyph.to_str().unwrap()), "{r}");
    let main = out.join("Font.ttx");
    assert!(std::fs::read_to_string(out.join("Font._g_l_y_f.ttx")).unwrap().contains("<TTGlyph src=\"Font._g_l_y_f.A_.ttx\"/>"));

    // Read back unchanged, the outlines are the font's own.
    let back = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [main], "output": dir.join("Same.ttf") }));
    assert_eq!(table(Path::new(back["output"].as_str().unwrap()), b"glyf"), table(&ttf, b"glyf"), "{back}");

    // A point moved in the glyph's own file reaches the font.
    let text = std::fs::read_to_string(&glyph).unwrap();
    // The leading space keeps `xMax="560"` out of the match.
    let moved = text.replacen(" x=\"560\"", " x=\"570\"", 1);
    assert_ne!(moved, text);
    std::fs::write(&glyph, moved).unwrap();
    let edited = run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [main], "output": dir.join("Moved.ttf") }));
    assert!(edited["replaced"].as_array().unwrap().contains(&json!("glyf")), "{edited}");

    // A glyph file outside the dump's folder is refused.
    let part = out.join("Font._g_l_y_f.ttx");
    let glyf = std::fs::read_to_string(&part).unwrap().replace("src=\"Font._g_l_y_f.A_.ttx\"", "src=\"../A.ttx\"");
    std::fs::write(&part, glyf).unwrap();
    let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main], "output": dir.join("Bad.ttf") })).unwrap_err();
    assert!(err.to_string().contains("parts must be beside it"), "{err}");

    // So is a glyph file that is a link to a file outside it.
    #[cfg(unix)]
    {
        std::fs::copy(&glyph, dir.join("A.ttx")).unwrap();
        std::os::unix::fs::symlink(dir.join("A.ttx"), out.join("link.ttx")).unwrap();
        let glyf = std::fs::read_to_string(&part).unwrap().replace("src=\"../A.ttx\"", "src=\"link.ttx\"");
        std::fs::write(&part, glyf).unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main], "output": dir.join("Bad.ttf") })).unwrap_err();
        assert!(err.to_string().contains("outside the dump's folder"), "{err}");
    }
}

/// A font of three bitmap strikes (bit depths 2, 4 and 1; image formats 1, 2, 5, 6 and 7) that
/// fontTools compiles as the tables crate does.
const BITMAP_FONT: &str = include_str!("../../../tables/src/ttx/read_bitmap/testdata/forms_row.ttx");

fn bitmap_font(dir: &Path) -> PathBuf {
    use fontcraft_tables::ttx::{ImportOptions, import};
    let font = import(fontcraft_tables::SfntFont::new(fontcraft_tables::Flavour::TrueType), BITMAP_FONT, &ImportOptions::default()).unwrap();
    let path = dir.join("Bitmap.ttf");
    std::fs::write(&path, fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default()).unwrap()).unwrap();
    path
}

/// The dump of `ttf`'s bitmap tables in `form`, written to `dir/Bitmap.ttx`.
fn dump_bitmaps(s: &mut Session, ttf: &Path, dir: &Path, form: &str) -> (PathBuf, Value) {
    let out = dir.join("Bitmap.ttx");
    let r = run(s, "font.dumpTables", json!({ "font": ttf, "tables": ["EBDT", "EBLC"], "bitmapFormat": form, "output": out }));
    (out, r)
}

#[test]
fn tables_import_ttx_reads_back_bitmap_images_in_every_form() {
    let dir = temp("bitmap-forms");
    let ttf = bitmap_font(&dir);
    let mut s = Session::new();
    let ebdt = table(&ttf, b"EBDT").unwrap();
    for form in ["raw", "row", "bitwise", "extfile"] {
        let (dump, r) = dump_bitmaps(&mut s, &ttf, &dir.join(form), form);
        let text = std::fs::read_to_string(&dump).unwrap();
        let element = if form == "raw" { "rawimagedata".to_string() } else { format!("<{form}imagedata") };
        assert!(text.contains(&element), "{form}: {text}");
        if form == "extfile" {
            assert_eq!(r["images"], json!(8), "{r}");
            assert_eq!(std::fs::read(dir.join(form).join("bitmaps/strike1/B.bin")).unwrap(), [0xab, 0xcd, 0xef]);
        }
        let back = dir.join(format!("{form}.ttf"));
        run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [dump], "output": back }));
        // The bitwise form keeps one bit a pixel, so the deeper strikes lose pixels, as in ttx.
        if form != "bitwise" {
            assert_eq!(table(&back, b"EBDT").unwrap(), ebdt, "{form}");
        }
    }

    // ttx names each file from the folder it ran in: a dump read from elsewhere finds its images
    // by the last three parts of the path, below its own folder.
    let ext = dir.join("extfile");
    let main = ext.join("Bitmap.ttx");
    let text = std::fs::read_to_string(&main).unwrap();
    let moved = ext.join("Moved.ttx");
    for prefix in ["build/out/", "/somewhere/else/", "../up/"] {
        std::fs::write(&moved, text.replace("value=\"bitmaps/", &format!("value=\"{prefix}bitmaps/"))).unwrap();
        let back = dir.join("moved.ttf");
        run(&mut s, "tables.importTtx", json!({ "font": ttf, "dumps": [moved], "output": back }));
        assert_eq!(table(&back, b"EBDT").unwrap(), ebdt, "{prefix}");
    }

    // Paths that leave the dump's folder, links out of it and files too large are refused.
    std::fs::write(dir.join("secret.bin"), [1, 2, 3]).unwrap();
    for value in ["../secret.bin", "/etc/passwd", "bitmaps/../../secret.bin", ""] {
        std::fs::write(&moved, text.replacen("value=\"bitmaps/strike0/A.bin\"", &format!("value=\"{value}\""), 1)).unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [moved] })).unwrap_err();
        assert!(err.to_string().contains("outside the dump's folder"), "{value}: {err}");
    }
    let a = ext.join("bitmaps/strike0/A.bin");
    #[cfg(unix)]
    {
        std::fs::remove_file(&a).unwrap();
        std::os::unix::fs::symlink(dir.join("secret.bin"), &a).unwrap();
        let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main] })).unwrap_err();
        assert!(err.to_string().contains("outside the dump's folder"), "{err}");
    }
    let _ = std::fs::remove_file(&a);
    // Sparse: the size is checked, and at most one byte past the limit is read.
    std::fs::File::create(&a).unwrap().set_len((16 << 20) + 1).unwrap();
    let err = s.execute("tables.importTtx", &json!({ "font": ttf, "dumps": [main] })).unwrap_err();
    assert!(err.to_string().contains("the most an image file may hold"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn font_dump_tables_writes_image_files_only_with_the_dump() {
    let dir = temp("bitmap-out");
    let ttf = bitmap_font(&dir);
    let mut s = Session::new();
    let err = s.execute("font.dumpTables", &json!({ "font": ttf, "bitmapFormat": "extfile" })).unwrap_err();
    assert!(err.to_string().contains("give `output` or `outputDir`"), "{err}");
    let err = s.execute("font.dumpTables", &json!({ "font": ttf, "bitmapFormat": "pixels" })).unwrap_err();
    assert!(err.to_string().contains("`bitmapFormat` must be"), "{err}");
    // A split dump writes the images beside its parts.
    let r = run(&mut s, "font.dumpTables", json!({ "font": ttf, "bitmapFormat": "extfile", "split": "tables", "outputDir": dir.join("split") }));
    assert_eq!(r["images"], json!(8), "{r}");
    assert!(dir.join("split/bitmaps/strike2/.notdef.bin").is_file());
    // When one file cannot be written, none is: a folder stands where an image would go.
    let out = dir.join("blocked");
    std::fs::create_dir_all(out.join("bitmaps/strike1/C.bin")).unwrap();
    let err = s.execute("font.dumpTables", &json!({ "font": ttf, "bitmapFormat": "extfile", "output": out.join("Bitmap.ttx") })).unwrap_err();
    assert!(err.to_string().contains("a folder has that name"), "{err}");
    assert!(!out.join("Bitmap.ttx").exists());
    assert!(!out.join("bitmaps/strike0").exists(), "the folders made for the images are removed");
    let _ = std::fs::remove_dir_all(&dir);
}
