//! The font-wide colour layer list of a UFO: `com.github.googlei18n.ufo2ft.colorLayers` in a
//! master's lib, each base glyph's colour layers as `[layer glyph, palette index]` pairs whose
//! layer glyphs are glyphs of the font. Key and form as ufo2ft (`outlineCompiler.py`,
//! `constants.py`) and fontc (`ufo2fontir/src/source.rs`) read them for a version 0 `COLR`.
//!
//! | Lib | Model |
//! |---|---|
//! | `base: [[layer glyph, index], …]` in a master's UFO | per pair, in order, a colour palette layer of `base` in that master holding the layer glyph as one unmoved component; index 65535 is the text colour |
//!
//! The compiler writes such a layer as a layer record of the layer glyph itself when the glyph
//! is named `<base>.color<i>`, the name it would give a new layer glyph (fontcraft-compile's
//! `color::layer_glyphs`).
//!
//! **Reading is checked by writing**, as in [`crate::libkeys`]: the list becomes colour layers
//! only when the writer gives the same list back (in any key order); otherwise it stays user
//! data, written as it was. The layers made from it carry a mark in their UFO data, so the writer
//! puts them back in the list rather than in UFO layers, as long as each still holds one
//! unmoved component of a glyph of the font.

use fontcraft_model::{Component, Family, Font, Layer, PaletteEntry, Shape};
use fontcraft_plist::{Dict, Value};

use crate::glyphkeys::{array, int, string, synthetic_id};
use crate::libkeys::sorted_keys;

pub(crate) const COLOR_LAYERS: &str = "com.github.googlei18n.ufo2ft.colorLayers";
/// The mark in a layer's UFO data.
const MARK: &str = "colorLayers";
/// Most colour layers read for one base glyph.
const MAX_LIST_LAYERS: usize = 1024;

/// The layer glyph and palette entry of `l` when it is a colour layer of the font-wide list: read
/// from it, and still nothing but one unmoved component of a glyph of the font.
pub(crate) fn listed<'a>(font: &Font, l: &'a Layer) -> Option<(&'a str, PaletteEntry)> {
    let entry = l.info.attrs.color_palette?;
    if l.is_master_layer() || l.extra.get(Family::Ufo, MARK).is_none() || !l.info.attrs.coordinates.is_empty() || !l.anchors.is_empty() {
        return None;
    }
    let [Shape::Component(c)] = l.shapes.as_slice() else { return None };
    (c.transform.is_identity() && font.glyph(&c.glyph).is_some()).then_some((c.glyph.as_str(), entry))
}

/// The list the writer gives master `master`: its listed layers by base glyph, in the font's
/// glyph order; `None` when it has none.
pub(crate) fn value(font: &Font, master: &str) -> Option<Value> {
    let mut d = Dict::new();
    for g in font.glyphs.iter() {
        let items: Vec<Value> = g
            .layers
            .iter()
            .filter(|l| l.master == master)
            .filter_map(|l| listed(font, l))
            .map(|(name, entry)| {
                let index = match entry {
                    PaletteEntry::Index(i) => i,
                    PaletteEntry::Foreground => u16::MAX,
                };
                array(vec![string(name), int(i64::from(index))])
            })
            .collect();
        if !items.is_empty() {
            d.insert(g.name.as_str(), array(items));
        }
    }
    (!d.entries.is_empty()).then_some(Value::Dict(d))
}

/// Turns each master's list into colour layers, where the writer gives the list back.
pub(crate) fn read(font: &mut Font) {
    let lists: Vec<(String, Value)> = font.masters.iter().filter_map(|m| m.user_data.get(COLOR_LAYERS).map(|v| (m.id.clone(), v.clone()))).collect();
    for (master, list) in lists {
        let mut f = font.clone();
        if add_layers(&mut f, &master, &list).is_none() {
            continue;
        }
        if value(&f, &master).map(sorted_keys) != Some(sorted_keys(list)) {
            continue;
        }
        if let Some(m) = std::sync::Arc::make_mut(&mut f.masters).iter_mut().find(|m| m.id == master) {
            m.user_data.0.entries.retain(|(k, _)| k.as_str() != COLOR_LAYERS);
        }
        *font = f;
    }
}

/// Adds the colour layers `list` describes to the glyphs of `master`; `None`, part done, when
/// it names a glyph the font lacks, a base glyph as its own layer, or a glyph with colour layers
/// of that master already.
fn add_layers(font: &mut Font, master: &str, list: &Value) -> Option<()> {
    let d = list.as_dict()?;
    if d.entries.len() > font.glyphs.len() {
        return None;
    }
    for (base, layers) in &d.entries {
        let items = &layers.as_array()?.items;
        if items.is_empty() || items.len() > MAX_LIST_LAYERS {
            return None;
        }
        let pairs: Vec<(String, PaletteEntry)> = crate::glyphkeys::palette_entries(layers)?;
        if pairs.iter().any(|(name, _)| name == base.as_str() || font.glyph(name).is_none()) {
            return None;
        }
        let mut g = font.glyph_mut(base.as_str()).ok()?;
        let width = g.layer(master)?.width;
        if g.layers.iter().any(|l| l.master == master && l.info.attrs.color_palette.is_some()) {
            return None;
        }
        for (k, (name, entry)) in pairs.into_iter().enumerate() {
            let mut l = Layer::new(master, width);
            let mut id = synthetic_id(master, &format!("{MARK}.{k}"));
            while g.layers.iter().any(|x| x.id == id) {
                id.push('#');
            }
            l.id = id;
            l.shapes = vec![Shape::Component(Component::new(name, 0.0, 0.0))];
            l.info.name = Some(match entry {
                PaletteEntry::Index(i) => format!("Color {i}"),
                PaletteEntry::Foreground => "Color *".to_string(),
            });
            l.info.attrs.color_palette = Some(entry);
            l.extra.insert(Family::Ufo, MARK, Value::Bool(true));
            g.layers.push(l);
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Files;

    fn plist(body: &str) -> Vec<u8> {
        format!(
            "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
        )
        .into_bytes()
    }

    fn glif(name: &str, x: i32) -> Vec<u8> {
        format!(
            "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\">\n  <advance width=\"600\"/>\n  <outline>\n    <contour>\n      <point x=\"{x}\" y=\"0\" type=\"line\"/>\n      <point x=\"{}\" y=\"0\" type=\"line\"/>\n      <point x=\"{}\" y=\"700\" type=\"line\"/>\n      <point x=\"{x}\" y=\"700\" type=\"line\"/>\n    </contour>\n  </outline>\n</glyph>\n",
            x + 200,
            x + 200
        )
        .into_bytes()
    }

    /// A UFO whose `a` is drawn in two palette colours by `a.color0` and `a.color1`.
    pub(crate) fn listed_ufo() -> Files {
        let mut files = Files::new();
        files.insert(
            "metainfo.plist".into(),
            plist("<dict>\n<key>creator</key>\n<string>org.example</string>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>"),
        );
        files.insert(
            "fontinfo.plist".into(),
            plist("<dict>\n<key>familyName</key>\n<string>Listed</string>\n<key>unitsPerEm</key>\n<integer>1000</integer>\n</dict>"),
        );
        files.insert(
            "layercontents.plist".into(),
            plist("<array>\n<array>\n<string>public.default</string>\n<string>glyphs</string>\n</array>\n</array>"),
        );
        files.insert(
            "glyphs/contents.plist".into(),
            plist("<dict>\n<key>a</key>\n<string>a.glif</string>\n<key>a.color0</key>\n<string>a.color0.glif</string>\n<key>a.color1</key>\n<string>a.color1.glif</string>\n</dict>"),
        );
        files.insert("glyphs/a.glif".into(), glif("a", 100));
        files.insert("glyphs/a.color0.glif".into(), glif("a.color0", 100));
        files.insert("glyphs/a.color1.glif".into(), glif("a.color1", 300));
        files.insert(
            "lib.plist".into(),
            plist(concat!(
                "<dict>\n<key>com.github.googlei18n.ufo2ft.colorLayers</key>\n<dict>\n<key>a</key>\n<array>\n",
                "<array>\n<string>a.color0</string>\n<integer>0</integer>\n</array>\n",
                "<array>\n<string>a.color1</string>\n<integer>1</integer>\n</array>\n</array>\n</dict>\n",
                "<key>com.github.googlei18n.ufo2ft.colorPalettes</key>\n<array>\n<array>\n",
                "<array>\n<real>1.0</real>\n<real>0.0</real>\n<real>0.0</real>\n<real>1.0</real>\n</array>\n",
                "<array>\n<real>0.0</real>\n<real>0.0</real>\n<real>1.0</real>\n<real>1.0</real>\n</array>\n</array>\n</array>\n",
                "<key>public.glyphOrder</key>\n<array>\n<string>a</string>\n<string>a.color0</string>\n<string>a.color1</string>\n</array>\n</dict>"
            )),
        );
        files
    }

    /// The colour layers of `a`: component glyph and palette entry.
    fn colour_layers(font: &Font) -> Vec<(String, Option<PaletteEntry>)> {
        let a = font.glyph("a").unwrap();
        a.layers
            .iter()
            .filter(|l| !l.is_master_layer())
            .map(|l| {
                let name = match l.shapes.as_slice() {
                    [Shape::Component(c)] => c.glyph.clone(),
                    other => format!("{other:?}"),
                };
                (name, l.info.attrs.color_palette)
            })
            .collect()
    }

    #[test]
    fn a_font_wide_colour_layer_list_opens_as_palette_layers_and_is_written_back() {
        let font = crate::ufo::read(&listed_ufo()).unwrap();
        let want = vec![("a.color0".to_string(), Some(PaletteEntry::Index(0))), ("a.color1".to_string(), Some(PaletteEntry::Index(1)))];
        assert_eq!(colour_layers(&font), want);
        assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_none(), "the list is not user data");

        let out = crate::ufo::write(&font).unwrap();
        let text = String::from_utf8(out["lib.plist"].clone()).unwrap();
        assert!(text.contains(COLOR_LAYERS), "{text}");
        assert!(!out.keys().any(|k| k.starts_with("glyphs.color")), "no UFO layer for the listed layers: {:?}", out.keys());
        let back = crate::ufo::read(&out).unwrap();
        assert_eq!(colour_layers(&back), want);
    }

    #[test]
    fn a_list_naming_a_missing_glyph_stays_user_data() {
        let mut files = listed_ufo();
        files.remove("glyphs/a.color1.glif");
        files.insert(
            "glyphs/contents.plist".into(),
            plist("<dict>\n<key>a</key>\n<string>a.glif</string>\n<key>a.color0</key>\n<string>a.color0.glif</string>\n</dict>"),
        );
        let font = crate::ufo::read(&files).unwrap();
        assert!(colour_layers(&font).is_empty());
        assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_some());
    }
}
