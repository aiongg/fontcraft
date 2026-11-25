//! What the values of custom parameters name in the font: the glyphs of a glyph list, and the names the font lacks
//! (`fontInfo.rows.valueEditors`, M7.33). Nothing here refuses a value: a list may name glyphs the font does not have
//! yet, so the editors only point them out.

use fontcraft_model::Font;
use fontcraft_model::plist::Value;

use crate::catalog::{self, Names};
use crate::value::lines;

/// Most names of one value looked up.
const MAX_NAMES: usize = 10_000;
/// Most unknown names listed; the rest are counted.
pub const MAX_LISTED: usize = 8;

/// What the value of the parameter `name` names; a spelling of `Variable Font Origin` older files use names a master.
pub fn names_of(name: &str) -> Names {
    match catalog::lookup(name) {
        Some(spec) => spec.names,
        None if name == "Variation Font Origin" => Names::Master,
        None => Names::Nothing,
    }
}

/// The glyph names one line of a glyph parameter's value mentions: the old name of a Rename Glyphs pair, the glyph of a
/// Reencode Glyphs entry, the glyphs after `include:` of Local Interpolation, else the line itself. A line with a
/// wildcard, a `key=value` query or a class names no glyph.
pub fn glyph_names<'a>(param: &str, line: &'a str) -> Vec<&'a str> {
    let names: Vec<&str> = match param {
        "Rename Glyphs" => line.split(',').filter_map(|pair| pair.split_once('=')).map(|(old, _)| old).collect(),
        "Reencode Glyphs" => line.split_once('=').map(|(glyph, _)| glyph).into_iter().collect(),
        "Local Interpolation" => line.split_once("include:").map(|(_, list)| list.split(',').collect()).unwrap_or_default(),
        _ if line.contains(['*', '=', '@']) => Vec::new(),
        _ => line.split(',').collect(),
    };
    names.into_iter().map(str::trim).filter(|n| !n.is_empty()).collect()
}

/// The lines of a value that holds text: a list's items, a text's lines.
fn text_lines(value: &Value) -> Vec<&str> {
    lines(value).unwrap_or_default()
}

/// The names in `value`, of the parameter `param`, that `font` lacks, each once: at most [`MAX_LISTED`], and how many
/// there are.
pub fn unknown(font: &Font, param: &str, value: &Value) -> (Vec<String>, usize) {
    let known = |name: &str| match names_of(param) {
        Names::Nothing => true,
        Names::Axis => font.axes.iter().any(|a| a.tag == name),
        Names::Master => font.masters.iter().any(|m| m.name == name || m.id == name),
        Names::Glyph => font.glyph(name).is_some(),
    };
    let list = text_lines(value);
    let mentioned: Vec<&str> = match names_of(param) {
        Names::Nothing => return (Vec::new(), 0),
        Names::Glyph => list.iter().flat_map(|line| glyph_names(param, line)).take(MAX_NAMES).collect(),
        // A text of several masters separates them with commas.
        Names::Master if matches!(value, Value::String(_)) && catalog::lookup(param).is_some_and(|s| s.kind == catalog::ParamKind::List) => {
            list.iter().flat_map(|line| line.split(',')).map(str::trim).filter(|n| !n.is_empty()).take(MAX_NAMES).collect()
        }
        _ => list.iter().map(|line| line.trim()).filter(|n| !n.is_empty()).take(MAX_NAMES).collect(),
    };
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let missing: Vec<&str> = mentioned.into_iter().filter(|name| !known(name) && seen.insert(*name)).collect();
    (missing.iter().take(MAX_LISTED).map(|s| (*s).to_string()).collect(), missing.len())
}

/// Whether `text`, the value of the glyph parameter `param`, mentions `glyph`.
pub fn mentions(param: &str, text: &str, glyph: &str) -> bool {
    text.lines().any(|line| glyph_names(param, line).contains(&glyph))
}

/// `text` with `glyph` added or removed, as the picker of the glyph parameter `param` writes it. A pair starts as
/// `glyph=`; a line that only mentions the glyph among others (a Rename Glyphs line of several pairs) loses that part.
pub fn with_glyph(param: &str, text: &str, glyph: &str, on: bool) -> String {
    if mentions(param, text, glyph) == on {
        return text.to_string();
    }
    let single = catalog::lookup(param).is_some_and(|s| s.kind == catalog::ParamKind::String) && param != "Local Interpolation";
    if param == "Local Interpolation" {
        let (head, list) = text.split_once("include:").unwrap_or((text.trim_end().trim_end_matches(';'), ""));
        let mut names: Vec<&str> = list.split(',').map(str::trim).filter(|n| !n.is_empty() && *n != glyph).collect();
        if on {
            names.push(glyph);
        }
        let head = head.trim().trim_end_matches(';').trim_end();
        return format!("{head}; include: {}", names.join(", "));
    }
    if single {
        return if on { glyph.to_string() } else { String::new() };
    }
    if on {
        let entry = if matches!(param, "Rename Glyphs" | "Reencode Glyphs") { format!("{glyph}=") } else { glyph.to_string() };
        let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines.push(&entry);
        return lines.join("\n");
    }
    let keep = |line: &str| -> Option<String> {
        if !glyph_names(param, line).contains(&glyph) {
            return Some(line.to_string());
        }
        if param == "Reencode Glyphs" {
            return None;
        }
        let parts: Vec<&str> = line
            .split(',')
            .filter(|part| match param {
                "Rename Glyphs" => part.split_once('=').is_none_or(|(old, _)| old.trim() != glyph),
                _ => part.trim() != glyph,
            })
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    };
    text.lines().filter_map(keep).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use fontcraft_model::plist::Str;

    use super::*;

    #[test]
    fn unknown_names_are_found_in_each_form() {
        let mut font = Font::new("Names");
        for g in ["e", "g", "g.alt"] {
            if font.glyph(g).is_none() {
                font.add_glyph(g, Vec::new(), 500.0).unwrap();
            }
        }
        let text = |s: &str| Value::String(Str::new(s));
        let of = |param: &str, s: &str| unknown(&font, param, &text(s)).0;
        assert_eq!(of("Remove Glyphs", "e\nq.sc\n*.sc\ncategory=Mark"), ["q.sc"]);
        assert_eq!(of("Rename Glyphs", "g.alt=g, zz=e\nq=new"), ["zz", "q"], "the new name may be new");
        assert_eq!(of("Reencode Glyphs", "space.x=0020,00A0\ne="), ["space.x"]);
        assert_eq!(of("Local Interpolation", "95; include: e, o.x, g"), ["o.x"]);
        assert_eq!(of("Style Name as STAT entry", "wgth"), ["wgth"]);
        assert_eq!(of("Disable Masters", "Regular, Nowhere"), ["Nowhere"]);
        assert!(of("Filter", "Nothing;1").is_empty());
        let long: String = (0..20).map(|i| format!("x{i}\n")).collect();
        let (listed, count) = unknown(&font, "Keep Glyphs", &text(&long));
        assert_eq!((listed.len(), count), (MAX_LISTED, 20));
    }

    #[test]
    fn the_picker_adds_and_removes_a_glyph_in_each_form() {
        for (param, before, on, after) in [
            ("Keep Glyphs", "a\n*.sc", true, "a\n*.sc\ne"),
            ("Keep Glyphs", "a\ne\n*.sc", false, "a\n*.sc"),
            ("Rename Glyphs", "a=b", true, "a=b\ne="),
            ("Rename Glyphs", "a=b, e=f", false, "a=b"),
            ("Reencode Glyphs", "e=0020,00A0\na=0041", false, "a=0041"),
            ("Local Interpolation", "95; include: o", true, "95; include: o, e"),
            ("Local Interpolation", "95", true, "95; include: e"),
            ("Local Interpolation", "95; include: e, o", false, "95; include: o"),
            ("Master Icon Glyph Name", "a", true, "e"),
        ] {
            assert_eq!(with_glyph(param, before, "e", on), after, "{param} {before}");
            assert_eq!(mentions(param, after, "e"), on, "{param} {after}");
        }
    }
}
