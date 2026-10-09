//! The same source and timestamp give the same bytes: two independent sessions open one source
//! and export it with TrueType and CFF outlines, plain, WOFF and WOFF2, and every file matches
//! byte for byte. Each build starts from its own reading, so state that varies between runs
//! (hash order, a clock other than the given timestamp) would show up here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::json;

/// Two masters with an instance at each; a curved glyph (converted to quadratics for TrueType),
/// a composite with an offset component, and vertical metrics.
const SOURCE: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
customParameters = (
{
name = vheaVertAscender;
value = 500;
},
{
name = vheaVertDescender;
value = -500;
}
);
familyName = "Same Bytes";
fontMaster = (
{
axesValues = (
400
);
id = m01;
name = Regular;
},
{
axesValues = (
700
);
id = m02;
name = Bold;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(500,0,l),
(500,700,l),
(100,700,l)
);
}
);
width = 600;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(80,0,l),
(620,0,l),
(620,700,l),
(80,700,l)
);
}
);
width = 700;
}
);
unicode = 65;
},
{
glyphname = O;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(465,0,o),
(600,135,o),
(600,350,c),
(600,565,o),
(465,700,o),
(300,700,c),
(135,700,o),
(0,565,o),
(0,350,c),
(0,135,o),
(135,0,o),
(300,0,c)
);
}
);
width = 640;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(520,0,o),
(680,135,o),
(680,350,c),
(680,565,o),
(520,700,o),
(340,700,c),
(160,700,o),
(0,565,o),
(0,350,c),
(0,135,o),
(160,0,o),
(340,0,c)
);
}
);
width = 720;
}
);
unicode = 79;
},
{
glyphname = acutecomb;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(0,0,l),
(80,0,l),
(140,120,l),
(60,120,l)
);
}
);
width = 0;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(0,0,l),
(100,0,l),
(170,120,l),
(70,120,l)
);
}
);
width = 0;
}
);
unicode = 769;
},
{
glyphname = Aacute;
layers = (
{
layerId = m01;
shapes = (
{
ref = A;
},
{
pos = (240,740);
ref = acutecomb;
}
);
width = 600;
},
{
layerId = m02;
shapes = (
{
ref = A;
},
{
pos = (280,740);
ref = acutecomb;
}
);
width = 700;
}
);
unicode = 193;
}
);
instances = (
{
axesValues = (
400
);
name = Regular;
},
{
axesValues = (
700
);
name = Bold;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

fn walk(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
}

/// Opens the source in a new session, exports it into `out` and returns each file, by its path
/// below `out`, with its bytes.
fn build(source: &Path, out: &Path) -> Vec<(String, Vec<u8>)> {
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": source })).unwrap();
    let params = json!({
        "outlines": ["truetype", "cff"],
        "containers": ["plain", "woff", "woff2"],
        "destination": out,
        "timestamp": 1_700_000_000,
    });
    s.execute("file.exportAll", &params).unwrap();
    let mut files: Vec<(String, Vec<u8>)> =
        walk(out).into_iter().map(|p| (p.strip_prefix(out).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap())).collect();
    files.sort();
    files
}

#[test]
fn the_same_source_and_timestamp_give_the_same_bytes() {
    let dir = std::env::temp_dir().join(format!("fontcraft-determinism-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("SameBytes.glyphs");
    std::fs::write(&source, SOURCE).unwrap();

    let first = build(&source, &dir.join("first"));
    let second = build(&source, &dir.join("second"));
    let names: Vec<&str> = first.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "otf/SameBytes-Bold.otf",
            "otf/SameBytes-Bold.woff",
            "otf/SameBytes-Bold.woff2",
            "otf/SameBytes-Regular.otf",
            "otf/SameBytes-Regular.woff",
            "otf/SameBytes-Regular.woff2",
            "ttf/SameBytes-Bold.ttf",
            "ttf/SameBytes-Bold.woff",
            "ttf/SameBytes-Bold.woff2",
            "ttf/SameBytes-Regular.ttf",
            "ttf/SameBytes-Regular.woff",
            "ttf/SameBytes-Regular.woff2",
        ]
    );
    assert_eq!(second.len(), first.len());
    for ((name, a), (other, b)) in first.iter().zip(&second) {
        assert_eq!(name, other);
        assert!(a == b, "{name} differs between two builds");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
