//! A UFO source's JPEG and TIFF glyph images are saved as PNG files with the same pixels, since a
//! UFO's `images/` folder holds PNG files only (`image-jpeg`, `image-tiff`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::io::{SaveFormat, save};
use fontcraft_io_ufo::Files;
use fontcraft_testkit::temp_dir;

fn plist(body: &str) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
    .into_bytes()
}

fn glif(name: &str, image: &str) -> Vec<u8> {
    format!("<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\">\n  <advance width=\"600\"/>\n  <image fileName=\"{image}\"/>\n</glyph>\n")
        .into_bytes()
}

/// A grey baseline JPEG of 8 × 8 pixels whose one block has no coefficients: one quantisation
/// table of ones, and DC and AC tables holding one code of one bit each.
fn jpeg() -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8];
    b.extend([0xFF, 0xDB, 0x00, 0x43, 0x00]);
    b.extend([1u8; 64]);
    b.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00]);
    for class in [0x00u8, 0x10] {
        b.extend([0xFF, 0xC4, 0x00, 0x14, class, 0x01]);
        b.extend([0u8; 15]);
        b.push(0x00);
    }
    b.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    // The DC difference and the end of block, one zero bit each, padded with ones.
    b.push(0x3F);
    b.extend([0xFF, 0xD9]);
    b
}

/// An uncompressed little-endian RGBA TIFF of 2 × 1 pixels.
fn tiff() -> Vec<u8> {
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    let entries: [(u16, u16, u32, u32); 11] = [
        (256, SHORT, 1, 2),
        (257, SHORT, 1, 1),
        (258, SHORT, 4, 146),
        (259, SHORT, 1, 1),
        (262, SHORT, 1, 2),
        (273, LONG, 1, 154),
        (277, SHORT, 1, 4),
        (278, SHORT, 1, 1),
        (279, LONG, 1, 8),
        (284, SHORT, 1, 1),
        (338, SHORT, 1, 2),
    ];
    let mut b = b"II*\0".to_vec();
    b.extend(8u32.to_le_bytes());
    b.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
    for (tag, kind, count, value) in entries {
        b.extend(tag.to_le_bytes());
        b.extend(kind.to_le_bytes());
        b.extend(count.to_le_bytes());
        b.extend(value.to_le_bytes());
    }
    b.extend(0u32.to_le_bytes());
    assert_eq!(b.len(), 146);
    for _ in 0..4 {
        b.extend(8u16.to_le_bytes());
    }
    b.extend([255, 0, 0, 255, 0, 128, 255, 64]);
    b
}

fn ufo() -> Files {
    let mut files = Files::new();
    files.insert(
        "metainfo.plist".into(),
        plist("<dict>\n<key>creator</key>\n<string>org.example</string>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>"),
    );
    files.insert(
        "fontinfo.plist".into(),
        plist("<dict>\n<key>familyName</key>\n<string>Pictures</string>\n<key>unitsPerEm</key>\n<integer>1000</integer>\n</dict>"),
    );
    files.insert(
        "glyphs/contents.plist".into(),
        plist("<dict>\n<key>a</key>\n<string>a.glif</string>\n<key>b</key>\n<string>b.glif</string>\n</dict>"),
    );
    files.insert("glyphs/a.glif".into(), glif("a", "photo.jpg"));
    files.insert("glyphs/b.glif".into(), glif("b", "scan.tif"));
    files.insert("images/photo.jpg".into(), jpeg());
    files.insert("images/scan.tif".into(), tiff());
    files
}

#[test]
fn jpeg_and_tiff_images_are_saved_as_png_files_with_the_same_pixels() {
    let font = fontcraft_io_ufo::ufo::read(&ufo()).unwrap();
    let dir = temp_dir("ufo-images").join("Pictures.ufo");
    let saved = save(&font, &dir, SaveFormat::Ufo).unwrap();
    let losses = serde_json::to_string(&saved.losses).unwrap();
    assert!(!losses.contains("image files"), "{losses}");

    let images: Vec<String> = std::fs::read_dir(dir.join("images")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    let mut images = images;
    images.sort();
    assert_eq!(images, ["photo.png", "scan.png"]);
    for (glif, png, original) in [("a.glif", "photo.png", jpeg()), ("b.glif", "scan.png", tiff())] {
        let text = std::fs::read_to_string(dir.join("glyphs").join(glif)).unwrap();
        assert!(text.contains(&format!("fileName=\"{png}\"")), "{text}");
        let bytes = std::fs::read(dir.join("images").join(png)).unwrap();
        assert_eq!(fontcraft_imagery::sniff(&bytes).unwrap().format, fontcraft_imagery::ImageFormat::Png);
        let want = fontcraft_imagery::decode_rgba(&original, fontcraft_imagery::MAX_PIXELS).unwrap();
        let got = fontcraft_imagery::decode_rgba(&bytes, fontcraft_imagery::MAX_PIXELS).unwrap();
        assert_eq!((got.width, got.height, got.pixels), (want.width, want.height, want.pixels), "{png}");
    }
    let back = fontcraft_engine::io::open(&dir, None).unwrap().0;
    let path = back.glyph("a").unwrap().layers[0].info.image.as_ref().unwrap().path.clone();
    assert_eq!(path.as_deref(), Some("photo.png"));
}
