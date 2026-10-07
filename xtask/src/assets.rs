//! `cargo xtask assets`: every asset is attributed, and no font binary is in the repository.
//!
//! * Every file under `assets/` or under a `tests/data/` folder, and every image anywhere, must
//!   have a row in `ASSETS.md`: a table row whose first cell is the file's path (or the path of a
//!   folder that holds it) in backticks.
//! * Compiled fonts are never committed, anywhere: tests build the fonts they need in code, and
//!   real fonts come from the pinned corpus (`cargo xtask corpus --fetch`, git-ignored). A font is
//!   recognised by its extension and by its first bytes, whatever it is called.
//! * Font *sources* (`.glyphs`, `.ufo`, `.designspace`…) may be committed only as small fixtures
//!   written for FontCraft, under `tests/data/`, with an `ASSETS.md` row.

use std::path::Path;

const FONT_BINARIES: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2", "pfa", "pfb", "dfont", "eot", "vfb", "vfc"];
const FONT_SOURCES: &[&str] = &["glyphs", "glyphspackage", "ufo", "ufoz", "designspace", "vfj", "sfd", "fontra"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "svg", "ico", "icns", "psd", "pdf"];
/// The first four bytes of the compiled font formats.
const FONT_MAGIC: &[&[u8; 4]] = &[b"\x00\x01\x00\x00", b"OTTO", b"true", b"typ1", b"ttcf", b"wOFF", b"wOF2"];

fn extension(path: &str) -> String {
    path.rsplit('/').next().and_then(|name| name.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Is `path` inside a font source bundle (`X.ufo/…`, `X.glyphspackage/…`) or a source file itself?
fn is_font_source(path: &str) -> bool {
    FONT_SOURCES.contains(&extension(path).as_str())
        || path.split('/').any(|part| FONT_SOURCES.iter().any(|e| part.to_ascii_lowercase().ends_with(&format!(".{e}"))))
}

fn is_fixture(path: &str) -> bool {
    path.starts_with("tests/data/") || path.contains("/tests/data/")
}

/// The paths listed in `ASSETS.md`: the first cell of each table row, when it is a path in backticks.
pub fn listed_paths(assets_md: &str) -> Vec<String> {
    assets_md
        .lines()
        .filter_map(|line| line.trim().strip_prefix('|'))
        .filter_map(|rest| rest.split('|').next())
        .filter_map(|cell| cell.trim().strip_prefix('`')?.strip_suffix('`'))
        .filter(|path| !path.is_empty())
        .map(|path| path.trim_end_matches('/').to_string())
        .collect()
}

/// `starts_with_font_magic(path)` says whether a file begins like a compiled font.
pub fn problems(files: &[String], assets_md: &str, starts_with_font_magic: &dyn Fn(&str) -> bool) -> Vec<String> {
    let listed = listed_paths(assets_md);
    let is_listed = |path: &str| listed.iter().any(|l| l == path || path.strip_prefix(l.as_str()).is_some_and(|rest| rest.starts_with('/')));
    let mut out = Vec::new();
    for path in files {
        if FONT_BINARIES.contains(&extension(path).as_str()) || starts_with_font_magic(path) {
            out.push(format!("{path}: compiled fonts are never committed (build test fonts in code; real fonts come from the pinned corpus)"));
            continue;
        }
        if is_font_source(path) && !is_fixture(path) {
            out.push(format!("{path}: font sources may be committed only as fixtures under a tests/data/ folder"));
            continue;
        }
        let needs_row = path.starts_with("assets/") || is_fixture(path) || IMAGES.contains(&extension(path).as_str());
        if needs_row && !is_listed(path) {
            out.push(format!("{path}: no row in ASSETS.md (path, what, author, source, licence)"));
        }
    }
    for l in &listed {
        if !files.iter().any(|f| f == l || f.strip_prefix(l.as_str()).is_some_and(|rest| rest.starts_with('/'))) {
            out.push(format!("ASSETS.md lists `{l}`, which is not in the repository"));
        }
    }
    out
}

pub fn run(root: &Path) -> Result<(), String> {
    use std::io::Read as _;
    let files = crate::repo_files(root)?;
    let assets_md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let magic = |path: &str| {
        let mut head = [0u8; 4];
        std::fs::File::open(root.join(path)).and_then(|mut f| f.read_exact(&mut head)).is_ok() && FONT_MAGIC.contains(&&head)
    };
    let found = problems(&files, &assets_md, &magic);
    if found.is_empty() {
        println!(
            "assets: {} listed in ASSETS.md, all present; every asset attributed; no font binaries in the repository",
            listed_paths(&assets_md).len()
        );
        return Ok(());
    }
    for p in &found {
        eprintln!("  {p}");
    }
    Err(format!("{} asset problem(s)", found.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let files: Vec<String> = [
            "assets/icons/app.svg",
            "assets/icons/new.svg",
            "crates/x/tests/data/Tiny.ufo/fontinfo.plist",
            "crates/x/src/lib.rs",
            "Demo.ttf",
            "examples/Family.glyphs",
            "crates/y/tests/data/unlisted.fea",
            "tests/data/top.fea",
            "packaging/icon.png",
            "docs/renamed.bin",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // Prose that mentions `assets` in backticks must not attribute the whole folder.
        let md = "Files under `assets` need rows.\n\n| Path | What |\n|---|---|\n| `assets/icons/app.svg` | … |\n| `crates/x/tests/data/Tiny.ufo` | … |\n| `gone/old.svg` | … |\n| *(none yet)* | |";
        let found = problems(&files, md, &|path| path == "docs/renamed.bin");
        let expect = [
            "assets/icons/new.svg: no row",
            "Demo.ttf: compiled fonts",
            "examples/Family.glyphs: font sources",
            "crates/y/tests/data/unlisted.fea: no row",
            "tests/data/top.fea: no row",
            "packaging/icon.png: no row",
            "docs/renamed.bin: compiled fonts",
            "ASSETS.md lists `gone/old.svg`",
        ];
        assert_eq!(found.len(), expect.len(), "{found:?}");
        for e in expect {
            assert!(found.iter().any(|f| f.starts_with(e)), "missing: {e} in {found:?}");
        }
        assert_eq!(listed_paths(md), ["assets/icons/app.svg", "crates/x/tests/data/Tiny.ufo", "gone/old.svg"]);
    }
}
