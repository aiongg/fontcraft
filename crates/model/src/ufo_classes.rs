//! The `GDEF` classes of a UFO's `public.openTypeCategories` (`docs/spec/30-source-formats.md`
//! § Glyph classification): the rules that class a font read from another format, and the
//! effective stated class of a font read from a UFO.
//!
//! The rules are glyphsLib's `_build_public_opentype_categories` (`builder/features.py`,
//! glyphsLib 6.15.0, Apache-2.0, see `NOTICE`), on each glyph's effective category and
//! subcategory (stored, else the glyph database's) and on the anchors of its layer for the
//! master:
//!
//! | Class | When |
//! |---|---|
//! | `ligature` | subcategory `Ligature` and an attaching anchor |
//! | `mark` | category `Mark`, subcategory `Nonspacing` or `Spacing Combining` |
//! | `base` | any other glyph with an attaching anchor |
//!
//! An attaching anchor is one whose name does not start with `_`. glyphsLib classifies after
//! propagating anchors into composites; here a layer without anchors of its own counts the
//! attaching anchors of its components' glyphs, to [`MAX_DEPTH`] levels. A glyph none of the rules
//! picks gets no entry.
//!
//! Reading a UFO stores a class from its map on the glyph only where it differs from the rules'
//! class, so a font written from another format and read back states no classes it did not
//! state before. The map stays in the master's kept `lib.plist`. A glyph's effective class
//! ([`stated`]) is its stored class, else the rules' class where a kept map says that class: what
//! a UFO written from the font states.

use std::collections::HashMap;

use crate::glyphdata::GlyphData;
use crate::plist::{Dict, Value};
use crate::{Family, Font, Glyph, Shape};

/// The key of the UFO lib's class map.
pub const OPENTYPE_CATEGORIES: &str = "public.openTypeCategories";
/// The kept UFO file that holds the map, in a master's [`crate::Extra`].
const LIB: &str = "lib.plist";
/// How deep components are followed in search of an attaching anchor.
const MAX_DEPTH: usize = 16;

/// The class of each glyph the rules pick, for the master `master`.
pub fn rule_classes(font: &Font, master: &str, data: &GlyphData) -> HashMap<String, &'static str> {
    let mut out = HashMap::new();
    let mut memo = HashMap::new();
    for g in font.glyphs.iter() {
        let stated = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
        let (mut category, mut sub_category) = (stated(&g.info.category), stated(&g.info.sub_category));
        if category.is_none() || sub_category.is_none() {
            let info = data.lookup(&g.name, &g.unicodes);
            category = category.or_else(|| info.category.map(|c| c.value.as_str().to_string()));
            sub_category = sub_category.or_else(|| info.sub_category.map(|c| c.value.as_str().to_string()));
        }
        let attaching = has_attaching_anchor(font, g, master, 0, &mut memo);
        let class = match (category.as_deref(), sub_category.as_deref()) {
            (_, Some("Ligature")) if attaching => "ligature",
            (Some("Mark"), Some("Nonspacing" | "Spacing Combining")) => "mark",
            _ if attaching => "base",
            _ => continue,
        };
        out.insert(g.name.clone(), class);
    }
    out
}

/// The class map of a kept UFO lib, from a master's [`crate::Extra`].
pub fn kept_map(extra: &crate::Extra) -> Option<&Dict> {
    extra.get(Family::Ufo, LIB).and_then(Value::as_dict).and_then(|lib| lib.get(OPENTYPE_CATEGORIES)).and_then(Value::as_dict)
}

/// The effective stated class of each glyph of a font read from a UFO, for the master `master`:
/// the glyph's stored class, else the rules' class (`rules`, from [`rule_classes`]) where the
/// master's kept map or the default master's says that class.
pub fn stated(font: &Font, master: &str, rules: &HashMap<String, &'static str>) -> HashMap<String, String> {
    let maps: Vec<&Dict> = [font.master(master).ok(), font.default_master().ok()].into_iter().flatten().filter_map(|m| kept_map(&m.extra)).collect();
    let mut out = HashMap::new();
    for g in font.glyphs.iter() {
        let class = match g.info.opentype_category.as_deref() {
            Some(c) => Some(c),
            None => rules.get(&g.name).copied().filter(|class| maps.iter().any(|m| m.get(&g.name).and_then(Value::as_str) == Some(*class))),
        };
        if let Some(class) = class {
            out.insert(g.name.clone(), class.to_string());
        }
    }
    out
}

/// Whether the glyph's layer for `master` has an attaching anchor, or (without anchors of its own)
/// one of its components' glyphs has. `memo` holds the answers found so far, so shared components
/// are looked at once.
fn has_attaching_anchor(font: &Font, g: &Glyph, master: &str, depth: usize, memo: &mut HashMap<String, bool>) -> bool {
    if let Some(&known) = memo.get(&g.name) {
        return known;
    }
    let Some(layer) = g.layer(master) else { return false };
    if !layer.anchors.is_empty() {
        return layer.anchors.iter().any(|a| !a.name.starts_with('_'));
    }
    // Marked as not attaching while it is looked at, so a component cycle ends.
    memo.insert(g.name.clone(), false);
    let mut found = false;
    if depth < MAX_DEPTH {
        for s in &layer.shapes {
            if let Shape::Component(c) = s
                && let Some(base) = font.glyph(&c.glyph)
                && has_attaching_anchor(font, base, master, depth + 1, memo)
            {
                found = true;
                break;
            }
        }
    }
    memo.insert(g.name.clone(), found);
    found
}
