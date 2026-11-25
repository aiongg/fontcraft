//! `cargo xtask roundtrip --mode stable`: what the writer wrote, read and written again, is the
//! same bytes (`roundtrip.noOpSaveIsByteIdentical` for files FontCraft saved).
//!
//! Every sample is read into a `Font` and written, and that output is read and written once
//! more, in memory. A designspace is written with its UFOs; one whose masters are not UFOs in the
//! corpus has no codec. The original's own bytes play no part: `model` and `native` compare with
//! those.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use fontcraft_io_ufo::{designspace, ufo};

use crate::roundtrip::{Kind, Outcome, Sample, first_difference, read_tree, tree_difference};

pub const MODE: &str = "stable";

type Tree = BTreeMap<String, Vec<u8>>;
/// The first and the second output.
type Pair = (Tree, Tree);

/// The name a single file has in its tree.
const FILE: &str = "";

fn file(text: String) -> Tree {
    Tree::from([(FILE.to_string(), text.into_bytes())])
}

pub fn check(corpus: &Path, sample: &Sample) -> Outcome {
    let path = corpus.join(&sample.path);
    let pair = catch_unwind(AssertUnwindSafe(|| match sample.kind {
        Kind::Glyphs => glyphs(&path).map(Some),
        Kind::GlyphsPackage => package(&path).map(Some),
        Kind::Ufo => ufo_tree(&path).map(Some),
        Kind::Designspace => designspace_tree(&path),
        Kind::Ufoz => Ok(None),
    }));
    match pair {
        Err(_) => Outcome::Failed("the codec panicked".to_string()),
        Ok(Err(e)) => Outcome::Failed(e),
        Ok(Ok(None)) => Outcome::NoCodec,
        Ok(Ok(Some(pair))) => compare(&pair),
    }
}

fn compare((first, second): &Pair) -> Outcome {
    let difference = match (first.get(FILE), second.get(FILE)) {
        (Some(a), Some(b)) if first.len() == 1 && second.len() == 1 => first_difference(a, b),
        _ => tree_difference(first, second),
    };
    difference.map_or(Outcome::Identical, Outcome::Different)
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|e| format!("{}: not UTF-8: {e}", path.display()))
}

fn glyphs(path: &Path) -> Result<Pair, String> {
    let once = |text: &str, what: &str| -> Result<String, String> {
        let font = fontcraft_io_glyphs::read(text).map_err(|e| format!("reading {what}: {e}"))?;
        fontcraft_io_glyphs::write(&font).map_err(|e| format!("writing {what}: {e}"))
    };
    let first = once(&read_text(path)?, "the original")?;
    let second = once(&first, "our output")?;
    Ok((file(first), file(second)))
}

fn package(path: &Path) -> Result<Pair, String> {
    let once = |files: &Tree, what: &str| -> Result<Tree, String> {
        let font = fontcraft_io_glyphs::read_package(files).map_err(|e| format!("reading {what}: {e}"))?;
        fontcraft_io_glyphs::write_package(&font).map_err(|e| format!("writing {what}: {e}"))
    };
    let first = once(&read_tree(path)?, "the original")?;
    let second = once(&first, "our output")?;
    Ok((first, second))
}

fn ufo_tree(path: &Path) -> Result<Pair, String> {
    let once = |files: &Tree, what: &str| -> Result<Tree, String> {
        let font = ufo::read(files).map_err(|e| format!("reading {what}: {e}"))?;
        ufo::write(&font).map_err(|e| format!("writing {what}: {e}"))
    };
    let first = once(&read_tree(path)?, "the original")?;
    let second = once(&first, "our output")?;
    Ok((first, second))
}

/// The document and its UFOs' files; `None` when a master is not a UFO in the corpus.
fn designspace_tree(path: &Path) -> Result<Option<Pair>, String> {
    let text = read_text(path)?;
    let dir = path.parent().ok_or("a designspace without a folder")?;
    let mut ufos = BTreeMap::new();
    for rel in designspace::ufo_paths(&text).map_err(|e| e.to_string())? {
        let source = dir.join(&rel);
        if !source.join("metainfo.plist").is_file() {
            return Ok(None);
        }
        ufos.insert(rel, read_tree(&source)?);
    }
    let once = |text: &str, ufos: &BTreeMap<String, Tree>, what: &str| -> Result<designspace::Written, String> {
        let font = designspace::read(text, ufos).map_err(|e| format!("reading {what}: {e}"))?;
        designspace::write(&font).map_err(|e| format!("writing {what}: {e}"))
    };
    let first = once(&text, &ufos, "the original")?;
    let second = once(&first.document, &first.ufos, "our output")?;
    Ok(Some((all_files(first), all_files(second))))
}

fn all_files(written: designspace::Written) -> Tree {
    let mut out = file(written.document);
    for (name, files) in written.ufos {
        out.extend(files.into_iter().map(|(path, bytes)| (format!("{name}/{path}"), bytes)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_output_is_compared_with_the_first() {
        let tree = |text: &str| file(text.to_string());
        assert_eq!(compare(&(tree("a\n"), tree("a\n"))), Outcome::Identical);
        assert!(matches!(compare(&(tree("a\nb\n"), tree("a\nc\n"))), Outcome::Different(d) if d.starts_with("line 2")));
        let mut more = tree("a\n");
        more.insert("x.ufo/lib.plist".into(), Vec::new());
        assert!(matches!(compare(&(more, tree("a\n"))), Outcome::Different(d) if d.contains("not written")));
    }
}
