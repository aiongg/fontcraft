//! The compiler normalisation of [`super::diff`] (`--normalize compiler`): each table decoded to
//! its TTX element tree, the differences that two compilers may make without changing behaviour
//! removed from both sides, and the trees written back as text for a line comparison.
//!
//! Ported from fontc's `ttx_diff` (`ttx_diff/src/ttx_diff/core.py`, Apache-2.0, Copyright the
//! fontc authors). What is ported, applied to both fonts alike:
//!
//! - Extension lookup wrappers stripped (when asked; the default in this mode);
//! - lookup indices inside a feature and feature indices inside a language system sorted;
//! - name IDs above 255 that other tables point at (axis, instance, `STAT` value and feature UI
//!   names) replaced by their English Windows strings, then the name records' user IDs blanked
//!   and the records sorted;
//! - a vendor ID of `NONE`, spaces or NULs, in `OS/2` and in the unique ID, written as spaces;
//!   Macintosh Roman English name records dropped; a compiler's own version stamp (`;fontc 1.2`,
//!   `;FontCraft 1.2`) removed from the version string; name ID 25 dropped when only one font has it;
//! - `head.checkSumAdjustment` blanked; `STAT` 1.2 written as 1.1;
//! - kerning, mark and cursive lookups emptied in `GPOS`, and the ligature carets and variation
//!   store removed from `GDEF`: the layout normaliser compares them instead
//!   ([`crate::normalize`]), as the pseudo-tables `GPOS=normalized` and `GDEF=normalized`;
//! - `GDEF` mark glyph sets sorted by their glyphs, and the mark filtering sets that point at
//!   them renumbered;
//! - the on-curve flag of one-point contours ignored;
//! - a budget of `glyf` coordinates and `gvar` deltas that differ by at most one unit made equal.
//!
//! Not ported yet: filling in the `gvar` deltas a compiler left to interpolation, matching
//! contour order and start points, and renumbering the classes of class-based contextual rules.
//! With `no_hints`, glyph instructions and the hinting fields of `maxp` are left out too; with
//! `no_version`, `head.fontRevision` and name IDs 3 and 5.

use std::collections::HashMap;

use crate::limits::MAX_TTX_DEPTH;
use crate::sfnt::{SfntFont, Tag};
use crate::ttx::{self, Element, XmlWriter, tag_to_xml};
use crate::{Result, TablesError};

/// Deepest element nesting walked; the TTX writers nest far less.
const MAX_TREE_DEPTH: usize = 4 * MAX_TTX_DEPTH;

/// Lookup types of `GPOS` the layout normaliser compares: pair, cursive, mark-to-base,
/// mark-to-ligature and mark-to-mark.
const NORMALIZED_GPOS_TYPES: [i64; 5] = [2, 3, 4, 5, 6];

/// The `maxp` fields that only hinting uses.
const MAXP_HINT_FIELDS: [&str; 7] =
    ["maxZones", "maxTwilightPoints", "maxStorage", "maxFunctionDefs", "maxInstructionDefs", "maxStackElements", "maxSizeOfInstructions"];

/// What one font contributes to the normalisation of its tables.
#[derive(Debug, Default)]
struct Side {
    /// English Windows name strings by name ID.
    names: HashMap<String, String>,
    /// New index of each `GDEF` mark glyph set, by old index.
    mark_sets: HashMap<String, String>,
    has_name_25: bool,
}

/// The settings and the per-font data of one comparison.
#[derive(Debug)]
pub(super) struct Context {
    a: Side,
    b: Side,
    unwrap_extensions: bool,
    no_hints: bool,
    no_version: bool,
    /// Share of `glyf` coordinates and `gvar` deltas, in percent, that may differ by one unit.
    off_by_one_budget: f64,
}

impl Context {
    pub(super) fn new(a: &SfntFont, b: &SfntFont, unwrap_extensions: bool, no_hints: bool, no_version: bool, off_by_one_budget: f64) -> Result<Self> {
        Ok(Context { a: side(a)?, b: side(b)?, unwrap_extensions, no_hints, no_version, off_by_one_budget })
    }

    /// The normalised text of table `tag` of each font; both fonts have it.
    pub(super) fn texts(&self, tag: Tag, a: &SfntFont, b: &SfntFont) -> Result<(String, String)> {
        let mut ta = table_tree(a, tag)?;
        let mut tb = table_tree(b, tag)?;
        self.normalize(&mut ta, tag, &self.a, self.b.has_name_25)?;
        self.normalize(&mut tb, tag, &self.b, self.a.has_name_25)?;
        match &tag {
            b"glyf" => off_by_ones(&mut ta, &mut tb, "TTGlyph", "name", "pt", self.off_by_one_budget)?,
            b"gvar" => off_by_ones(&mut ta, &mut tb, "glyphVariations", "glyph", "delta", self.off_by_one_budget)?,
            _ => {}
        }
        Ok((write_tree(&ta)?, write_tree(&tb)?))
    }

    /// Applies every rule that concerns table `tag` to its tree `t`.
    fn normalize(&self, t: &mut Element, tag: Tag, side: &Side, other_has_name_25: bool) -> Result<()> {
        match &tag {
            b"GPOS" | b"GSUB" => {
                if self.unwrap_extensions {
                    let (wrapper, kind) = if tag == *b"GPOS" { ("ExtensionPos", "9") } else { ("ExtensionSubst", "7") };
                    each_named(t, "Lookup", 0, &mut |lookup| unwrap_extension(lookup, wrapper, kind))?;
                }
                sort_indices(t, "Feature", "LookupListIndex")?;
                sort_indices(t, "LangSys", "FeatureIndex")?;
                sort_indices(t, "DefaultLangSys", "FeatureIndex")?;
                resolve_name_ids(t, "UINameID", Some("value"), &side.names)?;
                if tag == *b"GSUB" {
                    for element in ["FeatUILabelNameID", "FeatUITooltipTextNameID", "SampleTextNameID", "FirstParamUILabelNameID"] {
                        resolve_name_ids(t, element, Some("value"), &side.names)?;
                    }
                } else {
                    each_named(t, "Lookup", 0, &mut empty_normalized_lookup)?;
                }
            }
            b"fvar" => {
                resolve_name_ids(t, "NamedInstance", Some("subfamilyNameID"), &side.names)?;
                resolve_name_ids(t, "NamedInstance", Some("postscriptNameID"), &side.names)?;
                resolve_name_ids(t, "AxisNameID", None, &side.names)?;
            }
            b"STAT" => {
                for element in ["AxisNameID", "ValueNameID", "ElidedFallbackNameID"] {
                    resolve_name_ids(t, element, Some("value"), &side.names)?;
                }
                if let Some(version) = t.children.iter_mut().find(|c| c.name == "Version")
                    && version.attribute("value") == Some("0x00010002")
                {
                    set_attribute(version, "value", "0x00010001");
                }
            }
            b"OS/2" => {
                each_named(t, "achVendID", 0, &mut |e| {
                    if e.attribute("value").is_some_and(blank_vendor) {
                        set_attribute(e, "value", "    ");
                    }
                })?;
            }
            b"name" => self.normalize_names(t, side.has_name_25 && !other_has_name_25),
            b"head" => {
                for c in &mut t.children {
                    if c.name == "checkSumAdjustment" || (self.no_version && c.name == "fontRevision") {
                        c.attributes.retain(|(k, _)| k != "value");
                    }
                }
            }
            b"GDEF" => {
                t.children.retain(|c| c.name != "LigCaretList" && c.name != "VarStore");
                if let Some(sets) = t.children.iter_mut().find(|c| c.name == "MarkGlyphSetsDef") {
                    sort_mark_sets(sets);
                }
            }
            b"glyf" => {
                each_named(t, "contour", 0, &mut |contour| {
                    if contour.children.iter().filter(|c| c.name == "pt").count() == 1
                        && let Some(pt) = contour.children.iter_mut().find(|c| c.name == "pt")
                    {
                        set_attribute(pt, "on", "any");
                    }
                })?;
                if self.no_hints {
                    each_named(t, "TTGlyph", 0, &mut |g| g.children.retain(|c| c.name != "instructions"))?;
                }
            }
            b"maxp" if self.no_hints => t.children.retain(|c| !MAXP_HINT_FIELDS.contains(&c.name.as_str())),
            _ => {}
        }
        if !side.mark_sets.is_empty() {
            each_named(t, "MarkFilteringSet", 0, &mut |e| {
                if let Some(new) = e.attribute("value").and_then(|old| side.mark_sets.get(old)).cloned() {
                    set_attribute(e, "value", &new);
                }
            })?;
        }
        Ok(())
    }

    fn normalize_names(&self, t: &mut Element, drop_name_25: bool) {
        let attr = |e: &Element, k: &str| e.attribute(k).unwrap_or_default().to_string();
        t.children.retain(|r| {
            if r.name != "namerecord" {
                return true;
            }
            let id = attr(r, "nameID");
            let mac_roman_english = attr(r, "platformID") == "1" && attr(r, "platEncID") == "0" && attr(r, "langID") == "0x0";
            !(mac_roman_english || (drop_name_25 && id == "25") || (self.no_version && (id == "3" || id == "5")))
        });
        for r in t.children.iter_mut().filter(|r| r.name == "namerecord") {
            match r.attribute("nameID") {
                Some("3") => r.text = blank_unique_id_vendor(&r.text),
                Some("5") => r.text = strip_version_stamps(&r.text),
                Some(id) if id.parse::<u32>().is_ok_and(|n| n > 255) => set_attribute(r, "nameID", "USER_ID"),
                _ => {}
            }
        }
        let key = |r: &Element| (attr(r, "platformID"), attr(r, "platEncID"), attr(r, "langID"), attr(r, "nameID"), r.text.clone());
        let (mut records, others): (Vec<Element>, Vec<Element>) = std::mem::take(&mut t.children).into_iter().partition(|c| c.name == "namerecord");
        records.sort_by_cached_key(key);
        t.children = others;
        t.children.extend(records);
    }
}

/// The name strings, mark glyph set order and name ID 25 presence of one font.
fn side(font: &SfntFont) -> Result<Side> {
    let mut side = Side::default();
    if font.get(*b"name").is_some() {
        let names = table_tree(font, *b"name")?;
        for r in names.children.iter().filter(|c| c.name == "namerecord") {
            let Some(id) = r.attribute("nameID") else { continue };
            if id == "25" {
                side.has_name_25 = true;
            }
            if r.attribute("platformID") == Some("3") && r.attribute("platEncID") == Some("1") && r.attribute("langID") == Some("0x409") {
                side.names.insert(id.to_string(), r.text.trim().to_string());
            }
        }
    }
    if font.get(*b"GDEF").is_some() {
        let gdef = table_tree(font, *b"GDEF")?;
        if let Some(sets) = gdef.children.iter().find(|c| c.name == "MarkGlyphSetsDef") {
            let mut sorted: Vec<&Element> = sets.children.iter().filter(|c| c.name == "Coverage").collect();
            sorted.sort_by_cached_key(|c| coverage_glyphs(c));
            for (new, c) in sorted.iter().enumerate() {
                if let Some(old) = c.attribute("index") {
                    side.mark_sets.insert(old.to_string(), new.to_string());
                }
            }
        }
    }
    Ok(side)
}

/// Table `tag` of `font` as an element tree named as `ttx` names it. A table no TTX writer reads
/// is a `raw="True"` element holding its hex, as in a dump.
pub(super) fn table_tree(font: &SfntFont, tag: Tag) -> Result<Element> {
    type TableWriter = fn(Tag, &[u8], &SfntFont, &mut XmlWriter) -> Result<bool>;
    const WRITERS: [TableWriter; 6] = [
        ttx::core::write_table,
        ttx::cff::write_table,
        ttx::otdata::write_table,
        ttx::var::write_table,
        ttx::color::write_table,
        ttx::misc::write_table,
    ];
    let data = font.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag)))?;
    let name = tag_to_xml(tag);
    let mut w = XmlWriter::new();
    let mut known = Ok(false);
    for write in WRITERS {
        known = write(tag, data, font, &mut w);
        if !matches!(known, Ok(false)) {
            break;
        }
    }
    w.check()?;
    if matches!(known, Ok(true)) {
        return Ok(Element { name, children: w.tree().to_vec(), ..Element::default() });
    }
    let mut hex = XmlWriter::new();
    hex.open("hexdata", &[]);
    hex.hex(data);
    hex.close("hexdata");
    hex.check()?;
    Ok(Element { name, attributes: vec![("raw".into(), "True".into())], children: hex.tree().to_vec(), ..Element::default() })
}

/// Calls `f` on `e` and on every element below it named `name`, outermost first.
fn each_named(e: &mut Element, name: &str, depth: usize, f: &mut dyn FnMut(&mut Element)) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(TablesError::TooLarge { what: "the element nesting", limit: MAX_TREE_DEPTH });
    }
    if e.name == name {
        f(e);
    }
    for c in &mut e.children {
        each_named(c, name, depth + 1, f)?;
    }
    Ok(())
}

/// Every element below `e` (and `e` itself) named `name`, in document order.
fn collect_named<'a>(e: &'a mut Element, name: &str, depth: usize, out: &mut Vec<&'a mut Element>) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(TablesError::TooLarge { what: "the element nesting", limit: MAX_TREE_DEPTH });
    }
    if e.name == name {
        out.push(e);
        return Ok(());
    }
    for c in &mut e.children {
        collect_named(c, name, depth + 1, out)?;
    }
    Ok(())
}

fn set_attribute(e: &mut Element, name: &str, value: &str) {
    match e.attributes.iter_mut().find(|(k, _)| k == name) {
        Some((_, v)) => *v = value.to_string(),
        None => e.attributes.push((name.to_string(), value.to_string())),
    }
}

/// Sorts, inside each `container` element, the `value` of the `index` elements below it.
fn sort_indices(t: &mut Element, container: &str, index: &str) -> Result<()> {
    let mut failed = None;
    each_named(t, container, 0, &mut |c| {
        let mut items = Vec::new();
        for child in &mut c.children {
            if let Err(e) = collect_named(child, index, 1, &mut items) {
                failed = Some(e);
                return;
            }
        }
        let Some(mut values) = items.iter().map(|e| e.attribute("value").and_then(|v| v.parse::<i64>().ok())).collect::<Option<Vec<i64>>>() else {
            return;
        };
        values.sort_unstable();
        for (e, v) in items.into_iter().zip(values) {
            set_attribute(e, "value", &v.to_string());
        }
    })?;
    failed.map_or(Ok(()), Err)
}

/// Replaces each name ID above 255 in `element` (its `attribute`, or its text) by the string it
/// names; IDs up to 255 have fixed meanings and stay.
fn resolve_name_ids(t: &mut Element, element: &str, attribute: Option<&str>, names: &HashMap<String, String>) -> Result<()> {
    let lookup = |id: &str| -> Option<String> {
        let n = id.trim().parse::<u32>().ok()?;
        (n > 255).then(|| names.get(&n.to_string()).cloned().unwrap_or_else(|| format!("NonExistingNameID {n}")))
    };
    each_named(t, element, 0, &mut |e| match attribute {
        Some(a) => {
            if let Some(name) = e.attribute(a).and_then(&lookup) {
                set_attribute(e, a, &name);
            }
        }
        None => {
            if let Some(name) = lookup(&e.text) {
                e.text = name;
            }
        }
    })
}

/// A lookup's type, read through an extension wrapper.
fn lookup_type(lookup: &Element, wrapper: &str) -> Option<(i64, bool)> {
    let t = lookup.child("LookupType")?.attribute("value")?.parse::<i64>().ok()?;
    let extension = (wrapper == "ExtensionPos" && t == 9) || (wrapper == "ExtensionSubst" && t == 7);
    if !extension {
        return Some((t, false));
    }
    let inner = lookup.child(wrapper)?.child("ExtensionLookupType")?.attribute("value")?.parse::<i64>().ok()?;
    Some((inner, true))
}

/// Leaves only the type of a `GPOS` lookup the layout normaliser compares.
fn empty_normalized_lookup(lookup: &mut Element) {
    let Some((t, extension)) = lookup_type(lookup, "ExtensionPos") else { return };
    if !NORMALIZED_GPOS_TYPES.contains(&t) {
        return;
    }
    lookup.children.retain(|c| c.name == "LookupType");
    if extension && let Some(lt) = lookup.children.iter_mut().find(|c| c.name == "LookupType") {
        set_attribute(lt, "value", &t.to_string());
    }
}

/// Replaces each extension subtable of `lookup` by the subtable it wraps, and the lookup's type
/// by the wrapped type.
fn unwrap_extension(lookup: &mut Element, wrapper: &str, kind: &str) {
    if lookup.child("LookupType").and_then(|e| e.attribute("value")) != Some(kind) {
        return;
    }
    let Some(inner_type) = lookup.child(wrapper).and_then(|e| e.child("ExtensionLookupType")).and_then(|e| e.attribute("value")).map(str::to_string)
    else {
        return;
    };
    if let Some(lt) = lookup.children.iter_mut().find(|c| c.name == "LookupType") {
        set_attribute(lt, "value", &inner_type);
    }
    for child in &mut lookup.children {
        if child.name != wrapper {
            continue;
        }
        let Some(pos) = child.children.iter().position(|c| c.name != "ExtensionLookupType") else { continue };
        let index = child.attribute("index").map(str::to_string);
        let mut inner = child.children.swap_remove(pos);
        if let Some(index) = index {
            inner.attributes.retain(|(k, _)| k != "index");
            inner.attributes.insert(0, ("index".into(), index));
        }
        *child = inner;
    }
}

/// The glyph names of a mark glyph set's coverage, its sort key.
fn coverage_glyphs(c: &Element) -> Vec<String> {
    c.children.iter().filter_map(|g| g.attribute("value")).map(str::to_string).collect()
}

/// Sorts the coverages of `MarkGlyphSetsDef` by their glyphs and numbers them again.
fn sort_mark_sets(sets: &mut Element) {
    let (mut coverages, others): (Vec<Element>, Vec<Element>) = std::mem::take(&mut sets.children).into_iter().partition(|c| c.name == "Coverage");
    coverages.sort_by_cached_key(coverage_glyphs);
    for (i, c) in coverages.iter_mut().enumerate() {
        set_attribute(c, "index", &i.to_string());
    }
    sets.children = others;
    sets.children.extend(coverages);
}

/// Whether a vendor ID means "no vendor": `NONE`, or only spaces and NULs.
fn blank_vendor(v: &str) -> bool {
    v == "NONE" || v.chars().all(|c| c == ' ' || c == '\0')
}

/// The unique ID (`version;vendor;name`) with its first vendor field that is `NONE` or blank
/// written as four spaces.
fn blank_unique_id_vendor(text: &str) -> String {
    let parts: Vec<&str> = text.split(';').collect();
    let last = parts.len().saturating_sub(1);
    let mut done = false;
    let mut out = Vec::with_capacity(parts.len());
    for (i, p) in parts.iter().enumerate() {
        if !done && i > 0 && i < last && (*p == "NONE" || p.chars().all(|c| c == ' ')) {
            out.push("    ");
            done = true;
        } else {
            out.push(p);
        }
    }
    out.join(";")
}

/// The version string without compiler stamps: `;fontc ` or `;FontCraft ` followed by a digit,
/// up to the next `;` or white space.
fn strip_version_stamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while !rest.is_empty() {
        for stamp in [";fontc ", ";FontCraft "] {
            if let Some(after) = rest.strip_prefix(stamp)
                && after.starts_with(|c: char| c.is_ascii_digit())
            {
                let end = after.find(|c: char| c == ';' || c.is_whitespace()).unwrap_or(after.len());
                rest = after.get(end..).unwrap_or_default();
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// Makes equal, up to a budget, the `x` and `y` of `coord` elements that differ by at most one
/// unit between the same-named `container` elements (glyphs) of the two tables, and marks each
/// such element with `diff_adjusted="1"` on both sides. The budget is `percent` of the second
/// table's `coord` elements; glyphs whose element counts differ are left alone.
fn off_by_ones(a: &mut Element, b: &mut Element, container: &str, name_attr: &str, coord: &str, percent: f64) -> Result<()> {
    let mut count = 0usize;
    for c in b.children.iter_mut().filter(|c| c.name == container) {
        let mut items = Vec::new();
        collect_named(c, coord, 1, &mut items)?;
        count = count.saturating_add(items.len());
    }
    let budget = (percent / 100.0 * count as f64).floor();
    if budget.is_nan() || budget < 1.0 {
        return Ok(());
    }
    let budget = budget as usize;
    let by_name: HashMap<String, usize> = a
        .children
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name == container)
        .filter_map(|(i, c)| Some((c.attribute(name_attr)?.to_string(), i)))
        .collect();
    let mut spent = 0usize;
    for cb in b.children.iter_mut().filter(|c| c.name == container) {
        let Some(&ia) = cb.attribute(name_attr).and_then(|n| by_name.get(n)) else { continue };
        let Some(ca) = a.children.get_mut(ia) else { continue };
        let (mut ea, mut eb) = (Vec::new(), Vec::new());
        collect_named(ca, coord, 1, &mut ea)?;
        collect_named(cb, coord, 1, &mut eb)?;
        if ea.len() != eb.len() {
            continue;
        }
        for (pa, pb) in ea.into_iter().zip(eb) {
            for axis in ["x", "y"] {
                let (Some(va), Some(vb)) = (pa.attribute(axis), pb.attribute(axis)) else { continue };
                let (Ok(fa), Ok(fb)) = (va.parse::<f64>(), vb.parse::<f64>()) else { continue };
                let d = (fa - fb).abs();
                if d > 0.0 && d <= 1.0 {
                    let value = vb.to_string();
                    set_attribute(pa, axis, &value);
                    set_attribute(pa, "diff_adjusted", "1");
                    set_attribute(pb, "diff_adjusted", "1");
                    spent += 1;
                }
                if spent >= budget {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// The tree as indented XML text: two spaces per level, attributes in their order, text lines
/// one level deeper than their element.
pub(super) fn write_tree(e: &Element) -> Result<String> {
    let mut out = String::new();
    write_element(&mut out, e, 0)?;
    Ok(out)
}

fn write_element(out: &mut String, e: &Element, depth: usize) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(TablesError::TooLarge { what: "the element nesting", limit: MAX_TREE_DEPTH });
    }
    let indent = |out: &mut String, d: usize| (0..d).for_each(|_| out.push_str("  "));
    indent(out, depth);
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attributes {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape(out, v);
        out.push('"');
    }
    if e.children.is_empty() && e.text.is_empty() {
        out.push_str("/>\n");
        return Ok(());
    }
    out.push_str(">\n");
    for line in e.text.lines() {
        indent(out, depth + 1);
        escape(out, line);
        out.push('\n');
    }
    for c in &e.children {
        write_element(out, c, depth + 1)?;
    }
    indent(out, depth);
    out.push_str("</");
    out.push_str(&e.name);
    out.push_str(">\n");
    Ok(())
}

fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c < ' ' && c != '\t' => out.push_str(&format!("&#{};", u32::from(c))),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
pub(super) mod testing {
    //! Entry points for the tests: the rules of one table on a tree built by hand.
    use super::*;

    pub(in crate::diff) fn normalize_tree(t: &mut Element, tag: Tag, names: &[(&str, &str)], unwrap: bool) {
        let side = Side { names: names.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), ..Side::default() };
        let ctx =
            Context { a: Side::default(), b: Side::default(), unwrap_extensions: unwrap, no_hints: false, no_version: false, off_by_one_budget: 0.1 };
        ctx.normalize(t, tag, &side, false).unwrap();
    }

    pub(in crate::diff) fn strip(text: &str) -> String {
        strip_version_stamps(text)
    }

    pub(in crate::diff) fn unique_id(text: &str) -> String {
        blank_unique_id_vendor(text)
    }
}
