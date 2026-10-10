//! The glyph names written to the font (`compile.productionNames`,
//! `compile.productionNameSanitising`).
//!
//! With production names on, each glyph is written under its production name (the one it stores,
//! else the glyph database's, [`fontcraft_model::Resolved::apply_glyph_data`]), made valid as
//! fontc makes it: every character outside `A-Z`, `a-z`, `0-9`, period and underscore is removed,
//! and a name already written gets `.1`, `.2`… appended. Ported from fontc's
//! `fontbe/src/post.rs` (Apache-2.0, see `NOTICE`). A name that still breaks the Adobe Glyph List
//! rules (a leading digit or period, more than 63 characters) is written with a warning, as fontc
//! writes it.
//!
//! Production names are off when the *Don't use Production Names* parameter or the font's *Use
//! Custom Naming* setting is on, and for a UFO that maps no glyph to a production name (fontc and
//! ufo2ft rename a UFO's glyphs only through its `public.postscriptNames`). The source names are
//! then written; a name the `post` table cannot hold becomes `glyphNNNNN`, with a warning.

use std::collections::HashMap;

use fontcraft_model::{Resolved, ResolvedGlyph, is_ufo_family};

/// Longest name the `post` table can store (a Pascal string).
const MAX_STORED_NAME: usize = 255;
/// Longest name the Adobe Glyph List rules allow.
const MAX_AGL_NAME: usize = 63;
/// Most renamed glyphs a warning lists by name.
const MAX_LISTED: usize = 5;

/// Whether `order`'s glyphs are written under their production names.
pub fn uses_production_names(font: &Resolved, order: &[ResolvedGlyph]) -> bool {
    font.production_names && (!is_ufo_family(font.format) || order.iter().any(|g| g.production_name.is_some()))
}

/// The name each glyph of `order` is written under, in order.
pub fn final_names(font: &Resolved, order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> Vec<String> {
    if uses_production_names(font, order) { production_names(order, warnings) } else { source_names(order, warnings) }
}

/// May `name` be written to the `post` table as it is? (At most 63 bytes of letters, digits,
/// period, underscore and hyphen, as glyph names are conventionally restricted.)
fn is_post_name(name: &str) -> bool {
    (1..=MAX_AGL_NAME).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The glyphs' own names, `glyphNNNNN` by glyph id where `post` cannot hold the name.
fn source_names(order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> Vec<String> {
    order
        .iter()
        .enumerate()
        .map(|(gid, g)| {
            if is_post_name(&g.name) {
                g.name.clone()
            } else {
                warnings.push(format!(
                    "glyph `{}` is written to the font as `glyph{gid:05}`: its name is not a valid glyph name there",
                    g.name.escape_debug()
                ));
                format!("glyph{gid:05}")
            }
        })
        .collect()
}

/// Production names, made valid and unique.
fn production_names(order: &[ResolvedGlyph], warnings: &mut Vec<String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(order.len());
    let mut changed: Vec<(String, String)> = Vec::new();
    let mut breaks_agl: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(order.len());
    for (gid, g) in order.iter().enumerate() {
        let wanted = g.production_name.as_deref().unwrap_or(&g.name);
        let mut name: String = wanted.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_')).collect();
        if name.is_empty() || name.len() > MAX_STORED_NAME {
            name = format!("glyph{gid:05}");
        }
        if let Some(&first) = seen.get(&name) {
            let mut n = first;
            while seen.contains_key(&format!("{name}.{n}")) {
                n += 1;
            }
            seen.insert(name.clone(), n + 1);
            name = format!("{name}.{n}");
        }
        seen.insert(name.clone(), 1);
        if name != wanted {
            changed.push((wanted.to_string(), name.clone()));
        }
        if name != ".notdef" && (name.len() > MAX_AGL_NAME || name.starts_with(|c: char| c.is_ascii_digit() || c == '.')) {
            breaks_agl.push(name.clone());
        }
        out.push(name);
    }
    if !changed.is_empty() {
        let shown: Vec<String> = changed.iter().take(MAX_LISTED).map(|(a, b)| format!("`{}` → `{b}`", a.escape_debug())).collect();
        warnings.push(format!(
            "{} glyph names were changed to be valid and unique in the font: {}{}",
            changed.len(),
            shown.join(", "),
            if changed.len() > MAX_LISTED { ", …" } else { "" }
        ));
    }
    if !breaks_agl.is_empty() {
        let shown: Vec<&str> = breaks_agl.iter().take(MAX_LISTED).map(String::as_str).collect();
        warnings.push(format!(
            "{} glyph names start with a digit or a period or are longer than {MAX_AGL_NAME} characters, which some software does not accept: {}{}",
            breaks_agl.len(),
            shown.join(", "),
            if breaks_agl.len() > MAX_LISTED { ", …" } else { "" }
        ));
    }
    out
}
