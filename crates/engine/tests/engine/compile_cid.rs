//! `font.compile` of a CID-keyed font (M12.41): a CID-keyed CFF whose cmap comes from the
//! Unicode CMap file the request names; a CMap file that cannot be read writes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use serde_json::{Value, json};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, MetadataProvider};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-compile-cid-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A UTF-32 CMap of the made-up `Test-Identity` collection: A is CID 2 and a is CID 28, as the
/// Identity conversion numbers them (`.notdef`, space, then A to Z, a to z).
const CMAP: &str = "/CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Identity) def /Supplement 0 def end def\n\
/CMapName /UniTest-UTF32-H def\n1 begincidchar\n<00000041> 2\nendcidchar\n1 begincidrange\n<00000061> <00000062> 28\nendcidrange\n";

/// The first operator of the CFF table's Top DICT, as one byte or 1200 + the escaped byte.
fn first_top_operator(cff: &[u8]) -> u16 {
    let index_end = |at: usize| -> (usize, usize) {
        let count = usize::from(u16::from_be_bytes([cff[at], cff[at + 1]]));
        let size = usize::from(cff[at + 2]);
        let off = |i: usize| cff[at + 3 + i * size..at + 3 + (i + 1) * size].iter().fold(0usize, |a, b| a << 8 | usize::from(*b));
        let base = at + 2 + (count + 1) * size;
        (base + off(0), base + off(count))
    };
    let (_, names_end) = index_end(usize::from(cff[2]));
    let (top, _) = index_end(names_end);
    let mut i = top;
    loop {
        match cff[i] {
            12 => return 1200 + u16::from(cff[i + 1]),
            b @ 0..=21 => return u16::from(b),
            28 => i += 3,
            29 => i += 5,
            32..=246 => i += 1,
            _ => i += 2,
        }
    }
}

#[test]
fn a_cid_keyed_font_compiles_with_its_cmap_file_and_a_bad_cmap_file_writes_nothing() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Cid Sans"}));
    run(&mut s, "cid.convertToCid", json!({"registry": "Test", "ordering": "Identity", "supplement": 0}));
    let dir = temp("ok");
    let cmap = dir.join("UniTest-UTF32-H");
    std::fs::write(&cmap, CMAP).unwrap();
    let out = dir.join("CidSans.otf");
    let r = run(
        &mut s,
        "font.compile",
        json!({"formats": ["otf"], "destination": out.to_str().unwrap(), "cidCmap": cmap.to_str().unwrap(), "versionStamp": false}),
    );
    assert_eq!(r["fonts"].as_array().unwrap().len(), 1, "{r}");
    let bytes = std::fs::read(&out).unwrap();
    let font = FontRef::new(&bytes).unwrap();
    let cff = font.table_data(skrifa::raw::types::Tag::new(b"CFF ")).unwrap();
    assert_eq!(first_top_operator(cff.as_bytes()), 1230, "a CID-keyed Top DICT starts with ROS");
    let charmap = font.charmap();
    assert_eq!(charmap.map('A').map(|g| g.to_u32()), Some(2));
    assert_eq!(charmap.map('b').map(|g| g.to_u32()), Some(29));
    assert_eq!(charmap.map('B'), None, "only the CMap's code points are encoded");
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_3_0);

    let bad_dir = temp("bad");
    let bad = bad_dir.join("UniBad-UTF32-H");
    std::fs::write(&bad, "/CMapName /UniBad-UTF32-H def 1 begincidrange <41> 2 endcidrange").unwrap();
    let dest = bad_dir.join("out");
    let e = s
        .execute("font.compile", &json!({"formats": ["otf"], "destination": dest.to_str().unwrap(), "cidCmap": bad.to_str().unwrap()}))
        .map(|r| r.to_string())
        .unwrap_or_else(|e| e.to_string());
    assert!(e.contains("Unicode CMap cannot be read"), "{e}");
    assert!(!dest.exists() || std::fs::read_dir(&dest).unwrap().next().is_none(), "a failing compile writes nothing");
    let e = s
        .execute("font.compile", &json!({"formats": ["otf"], "destination": dest.to_str().unwrap(), "cidMacScript": 40000}))
        .expect_err("a script id beyond 32767")
        .to_string();
    assert!(e.contains("cidMacScript"), "{e}");
}
