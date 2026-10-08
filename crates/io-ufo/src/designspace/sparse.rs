//! Intermediate layers as sparse designspace sources: a `<source>` with a `layer` attribute that
//! points at the UFO layer holding the intermediate layers of one location, as glyphsLib writes
//! it; and the way back.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0, see `NOTICE`): `builder/sources.py`
//! (`_to_designspace_source_layer`: one source per UFO layer name and master, at the layer's
//! coordinates with the master's value for an axis the coordinates leave out, named
//! `<master source> <layer>`, after its master's source, sorted by location).

use fontcraft_model::{Family, Font, Layer};
use fontcraft_plist::xml::{Element, Node as XNode};

use crate::ufo;

/// One sparse source: the master whose UFO holds the layer, the UFO layer and its location.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Brace {
    pub master: String,
    pub layer: String,
    pub location: Vec<f64>,
}

/// Whether an intermediate layer becomes a sparse source: a layer of a master with a UFO of its
/// own, with coordinates, that is neither an alternate layer (those become glyphs) nor a colour
/// palette layer (whose UFO layer is `color.<n>`).
pub(crate) fn comes_back(font: &Font, l: &Layer) -> bool {
    let a = &l.info.attrs;
    !l.is_master_layer()
        && !a.coordinates.is_empty()
        && a.axis_rules.is_empty()
        && a.color_palette.is_none()
        && font.masters.iter().any(|m| m.id == l.master && m.extra.get(Family::Designspace, "layer").is_none())
        && ufo::ufo_layer_name(font, l).is_some()
}

/// The location of an intermediate layer: its coordinates, the master's value where it has none
/// (format version 4's `-`) or where it states fewer than there are axes.
fn location(font: &Font, l: &Layer) -> Vec<f64> {
    let master = font.masters.iter().find(|m| m.id == l.master);
    (0..font.axes.len())
        .map(|i| {
            l.info
                .attrs
                .coordinates
                .get(i)
                .copied()
                .flatten()
                .unwrap_or_else(|| master.map_or(0.0, |m| fontcraft_model::axes::design_value(&m.location, i)))
        })
        .collect()
}

/// The sparse sources of `font`, master by master in the masters' order, each master's sorted by
/// location.
pub(crate) fn braces(font: &Font) -> Vec<Brace> {
    // (UFO layer, location) → the masters with such a layer, in order of first appearance.
    let mut keys: Vec<(String, Vec<f64>, Vec<&str>)> = Vec::new();
    for g in font.glyphs.iter() {
        for l in g.layers.iter().filter(|l| comes_back(font, l)) {
            let Some(name) = ufo::ufo_layer_name(font, l) else { continue };
            let loc = location(font, l);
            match keys.iter_mut().find(|(n, p, _)| *n == name && *p == loc) {
                Some((_, _, masters)) => {
                    if !masters.contains(&l.master.as_str()) {
                        masters.push(&l.master);
                    }
                }
                None => keys.push((name, loc, vec![&l.master])),
            }
        }
    }
    let mut out = Vec::new();
    for m in font.masters.iter() {
        let mut mine: Vec<Brace> = keys
            .iter()
            .filter(|(_, _, masters)| masters.contains(&m.id.as_str()))
            .map(|(n, p, _)| Brace { master: m.id.clone(), layer: n.clone(), location: p.clone() })
            .collect();
        mine.sort_by(|a, b| a.location.partial_cmp(&b.location).unwrap_or(std::cmp::Ordering::Equal));
        out.extend(mine);
    }
    out
}

/// The name glyphsLib gives a sparse source: its master source's name and the layer's.
pub(crate) fn source_name(master_source: &str, layer: &str) -> String {
    format!("{master_source} {layer}")
}

/// A sparse source element, as designspaceLib writes glyphsLib's.
pub(crate) fn element(b: &Brace, filename: &str, names: &[String]) -> Element {
    let mut el =
        Element::new("source").with_attr("filename", filename).with_attr("name", source_name(&b.master, &b.layer)).with_attr("layer", &b.layer);
    if !names.is_empty() {
        el.children.push(XNode::Element(super::location_element(names, &b.location)));
    }
    el
}

/// How many intermediate layers that become sparse sources take a master's value on an axis
/// (format version 4's `-`), which comes back as the value.
pub(crate) fn master_values(font: &Font) -> usize {
    font.glyphs.iter().flat_map(|g| g.layers.iter()).filter(|l| comes_back(font, l) && l.info.attrs.coordinates.iter().any(Option::is_none)).count()
}
