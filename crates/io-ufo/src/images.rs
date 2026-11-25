//! JPEG and TIFF images of a UFO source as PNG files, the only kind a UFO's `images/` folder
//! holds (`image-jpeg`, `image-tiff`).
//!
//! The writers copy a UFO source's other files as they are ([`Font::files`]); a glyph image
//! naming a JPEG or TIFF file there would be a file the UFO specification does not allow. Before
//! saving, the caller hands each such file to its decoder through [`to_png`], which this crate
//! does not have, and saves the font it gives back: the PNG file under the old name with a `png`
//! extension (`_1`, `_2`… after the name when that file exists), the images naming it, and the
//! old file gone. A file the decoder refuses stays as it is, and the loss report names it
//! (`ufo::LAYER_LOSSES`, the image files that are not PNG files).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use fontcraft_model::{Family, Font, Image};
use fontcraft_plist::Value;

use crate::ufo::FilesFrom;

/// Most image files converted in one save.
pub const MAX_CONVERTED: usize = 10_000;
/// Most numbered names tried for one converted file.
const MAX_SUFFIX: usize = 1_000;

/// Whether `path` names a JPEG or TIFF file by its extension.
fn convertible(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["jpg", "jpeg", "tif", "tiff"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Where the writer `files` names finds master `master`'s files in [`Font::files`]; `None` when
/// it finds none (a document that is not a UFO source, or a designspace master without a file).
fn prefix(font: &Font, files: FilesFrom, master: &str) -> Option<String> {
    if !fontcraft_model::is_ufo_family(font.source.format) {
        return None;
    }
    let file = font.masters.iter().find(|m| m.id == master).and_then(|m| m.extra.get(Family::Designspace, "filename")).and_then(Value::as_str);
    match (files, file) {
        (FilesFrom::Single, _) | (FilesFrom::PerMaster, None) => Some(String::new()),
        (FilesFrom::PerMaster | FilesFrom::Designspace, Some(f)) => Some(format!("{f}/")),
        (FilesFrom::Designspace, None) => None,
    }
}

/// The images of a layer: its own and its background's.
fn images(l: &fontcraft_model::Layer) -> impl Iterator<Item = &Image> {
    l.info.image.iter().chain(l.info.background.iter().flat_map(|b| b.image.iter()))
}

/// `font` with the JPEG and TIFF files its glyph images name in their UFO's `images/` folder as
/// PNG files `convert` makes (the decoded pixels, encoded as PNG), for the writer `files` names;
/// `None` when there is nothing it converts. At most [`MAX_CONVERTED`] files are converted.
pub fn to_png(font: &Font, files: FilesFrom, convert: &dyn Fn(&[u8]) -> Option<Vec<u8>>) -> Option<Font> {
    // The file's key in `Font::files` → (prefix, the image path, the new image path).
    let mut plan: BTreeMap<String, (String, String, Option<String>)> = BTreeMap::new();
    let mut prefixes: BTreeMap<&str, Option<String>> = BTreeMap::new();
    for l in font.glyphs.iter().flat_map(|g| g.layers.iter()) {
        let p = prefixes.entry(l.master.as_str()).or_insert_with(|| prefix(font, files, &l.master));
        let Some(p) = p.clone() else { continue };
        for path in images(l).filter_map(|i| i.path.as_deref()).filter(|path| convertible(path)) {
            let key = format!("{p}images/{path}");
            if plan.len() < MAX_CONVERTED && font.files.0.contains_key(&key) {
                plan.entry(key).or_insert_with(|| (p.clone(), path.to_string(), None));
            }
        }
    }
    let mut made: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for (key, (p, path, new)) in &mut plan {
        let Some(bytes) = font.files.0.get(key.as_str()) else { continue };
        let Some(png) = convert(bytes) else { continue };
        let stem = std::path::Path::new(path.as_str()).with_extension("");
        let stem = stem.to_string_lossy();
        let free = |name: &str| !font.files.0.contains_key(&format!("{p}images/{name}")) && !taken.contains(&format!("{p}images/{name}"));
        let Some(name) = std::iter::once(format!("{stem}.png")).chain((1..=MAX_SUFFIX).map(|n| format!("{stem}_{n}.png"))).find(|n| free(n)) else {
            continue;
        };
        taken.insert(format!("{p}images/{name}"));
        made.insert(format!("{p}images/{name}"), png);
        *new = Some(name);
    }
    if made.is_empty() {
        return None;
    }
    let mut out = font.clone();
    {
        let store = Arc::make_mut(&mut out.files);
        for (key, (_, _, new)) in &plan {
            if new.is_some() {
                store.0.remove(key);
            }
        }
        for (key, png) in made {
            store.0.insert(key, Arc::from(png));
        }
    }
    let renamed = |master: &str, path: &str| -> Option<String> {
        let p = prefixes.get(master).cloned().flatten()?;
        plan.get(&format!("{p}images/{path}")).and_then(|(_, _, new)| new.clone())
    };
    let count = out.glyphs.as_arcs().len();
    for i in 0..count {
        let touches =
            out.glyphs.as_arcs().get(i).is_some_and(|g| {
                g.layers.iter().any(|l| images(l).filter_map(|im| im.path.as_deref()).any(|path| renamed(&l.master, path).is_some()))
            });
        if !touches {
            continue;
        }
        let Some(mut g) = out.glyphs.get_mut_at(i) else { continue };
        for l in &mut g.layers {
            let master = l.master.clone();
            if let Some(im) = l.info.image.as_mut()
                && let Some(new) = im.path.as_deref().and_then(|path| renamed(&master, path))
            {
                im.path = Some(new);
            }
            let background =
                l.info.background.as_ref().and_then(|b| b.image.as_ref()).and_then(|im| im.path.as_deref()).and_then(|path| renamed(&master, path));
            if let Some(new) = background
                && let Some(im) = l.info.background.as_mut().and_then(|b| b.image.as_mut())
            {
                im.path = Some(new);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use fontcraft_model::SourceFormat;

    use super::*;

    fn font_with(files: &[(&str, &[u8])], images: &[(&str, &str)]) -> Font {
        let mut font = Font::new("Images");
        font.glyphs.retain(|_| false);
        font.source.format = SourceFormat::Ufo;
        let mut store = fontcraft_model::FileStore::default();
        for (k, v) in files {
            store.0.insert((*k).to_string(), Arc::from(*v));
        }
        font.files = Arc::new(store);
        let master = font.masters.first().map(|m| m.id.clone()).unwrap();
        for (glyph, path) in images {
            let mut g = fontcraft_model::Glyph::new(*glyph);
            let mut l = fontcraft_model::Layer::new(&master, 500.0);
            l.info.image = Some(Image { path: Some((*path).to_string()), ..Default::default() });
            g.layers.push(l);
            font.glyphs.push(g);
        }
        font
    }

    fn path_of(font: &Font, glyph: &str) -> Option<String> {
        font.glyph(glyph)?.layers.first()?.info.image.as_ref()?.path.clone()
    }

    /// Stands in for the decoder: "decodes" anything but `bad`.
    fn fake(bytes: &[u8]) -> Option<Vec<u8>> {
        (bytes != b"bad").then(|| [b"PNG:".as_slice(), bytes].concat())
    }

    #[test]
    fn jpeg_and_tiff_images_become_png_files_beside_the_others() {
        let font = font_with(
            &[
                ("images/a.jpg", b"jay"),
                ("images/b.TIFF", b"tee"),
                ("images/c.png", b"png"),
                ("images/d.png", b"other"),
                ("images/d.jpeg", b"dee"),
                ("images/e.jpg", b"bad"),
            ],
            &[("a", "a.jpg"), ("a2", "a.jpg"), ("b", "b.TIFF"), ("c", "c.png"), ("d", "d.jpeg"), ("e", "e.jpg")],
        );
        let out = to_png(&font, FilesFrom::Single, &fake).unwrap();
        let file = |k: &str| out.files.0.get(k).map(|v| v.to_vec());
        assert_eq!(file("images/a.png"), Some(b"PNG:jay".to_vec()));
        assert_eq!(file("images/b.png"), Some(b"PNG:tee".to_vec()));
        assert_eq!(file("images/d_1.png"), Some(b"PNG:dee".to_vec()), "d.png is another file");
        assert_eq!(file("images/d.png"), Some(b"other".to_vec()));
        assert_eq!((file("images/a.jpg"), file("images/b.TIFF"), file("images/d.jpeg")), (None, None, None), "the old files go");
        assert_eq!(file("images/e.jpg"), Some(b"bad".to_vec()), "a file the decoder refuses stays");
        let paths: Vec<Option<String>> = ["a", "a2", "b", "c", "d", "e"].iter().map(|g| path_of(&out, g)).collect();
        let want = ["a.png", "a.png", "b.png", "c.png", "d_1.png", "e.jpg"].map(|p| Some(p.to_string()));
        assert_eq!(paths, want);

        // Nothing to convert, or not a UFO source: no new font.
        assert!(to_png(&font_with(&[("images/c.png", b"png")], &[("c", "c.png")]), FilesFrom::Single, &fake).is_none());
        let mut glyphs = font.clone();
        glyphs.source.format = SourceFormat::Glyphs;
        assert!(to_png(&glyphs, FilesFrom::Single, &fake).is_none());
    }
}
