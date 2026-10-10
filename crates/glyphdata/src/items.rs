//! The text of the Add Glyphs dialog (05-font-view.md § 3.1): items separated by spaces, line
//! breaks or tabs, each a glyph name, a single character, a code-point name (`uni00E4`,
//! `u1F600`), a range of two such items joined by a colon, or a glyph recipe.
//!
//! [`parse`] splits the text into [`Item`]s; [`resolve`] turns them into the glyphs to add, with
//! their names and code points from the database. The text is untrusted: its size, the number
//! of items, the width of one range and the number of glyphs it asks for are all capped, and
//! going over a cap is an [`Error::TooLarge`], never a long wait.

use std::collections::HashSet;

use crate::names::{code_point_name, parse_code_point_name, validate};
use crate::{Error, GlyphData};

/// Longest Add Glyphs text, in bytes.
pub const MAX_TEXT_BYTES: usize = 512 * 1024;
/// Most items one text may hold.
pub const MAX_ITEMS: usize = 65_535;
/// Most code points one range may span, from its first endpoint to its last, both included.
pub const MAX_RANGE_CODE_POINTS: u32 = 65_536;
/// Most glyphs one text may ask for, ranges expanded (a font holds at most 65,535 glyphs).
pub const MAX_WANTED: usize = 65_535;
/// Longest glyph name an item may give, in bytes (the font model's limit).
pub const MAX_ITEM_NAME_BYTES: usize = 255;

/// One item of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item<'t> {
    /// A glyph name, possibly a legacy or production name (`Ccedilla`, `dotlessi`).
    Name(&'t str),
    /// `uniXXXX` or `uXXXXX`, with the code point it stands for.
    CodePointName(&'t str, u32),
    /// A single character typed or pasted as it is.
    Character(char),
    /// `first:last`, both endpoints included.
    Range(&'t str, &'t str),
    /// A glyph recipe (`x+acutecomb=xacute`, `Eth=Dtail`).
    Recipe(&'t str),
}

impl Item<'_> {
    /// The item as it was written.
    pub fn text(&self) -> String {
        match self {
            Item::Name(t) | Item::CodePointName(t, _) | Item::Recipe(t) => (*t).to_string(),
            Item::Character(c) => c.to_string(),
            Item::Range(a, b) => format!("{a}:{b}"),
        }
    }
}

/// Splits the Add Glyphs text into items.
pub fn parse(text: &str) -> Result<Vec<Item<'_>>, Error> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(Error::TooLarge { what: "Add Glyphs text (bytes)", size: text.len(), limit: MAX_TEXT_BYTES });
    }
    let mut items = Vec::new();
    for word in text.split_whitespace() {
        if items.len() == MAX_ITEMS {
            let count = text.split_whitespace().count();
            return Err(Error::TooLarge { what: "Add Glyphs items", size: count, limit: MAX_ITEMS });
        }
        items.push(item(word));
    }
    Ok(items)
}

fn item(word: &str) -> Item<'_> {
    let mut chars = word.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Item::Character(c);
    }
    if word.contains(['+', '=']) {
        return Item::Recipe(word);
    }
    if let Some((first, last)) = word.split_once(':') {
        return Item::Range(first, last);
    }
    match parse_code_point_name(word) {
        Some(cp) => Item::CodePointName(word, cp),
        None => Item::Name(word),
    }
}

/// A glyph the text asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub name: String,
    /// The code points the new glyph gets (none for a name the database gives none).
    pub code_points: Vec<u32>,
}

/// An item that gives no glyph, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub item: String,
    pub reason: String,
}

/// What a text asks for: the glyphs in the order the items name them (each name once), and the
/// items that give none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub glyphs: Vec<Wanted>,
    pub skipped: Vec<Skipped>,
}

/// Turns items into glyphs. A name item keeps its spelling unless the database knows it under
/// another nice name (`dotlessi` → `idotless`, `uni00E4` → `adieresis`); a character and every
/// code point of a range become the nice name of their code point. With `keep_names` (the
/// font's Use Custom Naming), typed names are kept as typed and code points become code-point
/// names. Ranges skip code points Unicode does not assign and surrogates.
pub fn resolve(data: &GlyphData, items: &[Item<'_>], keep_names: bool) -> Result<Resolved, Error> {
    let mut out = Resolved::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |out: &mut Resolved, w: Wanted| -> Result<(), Error> {
        if seen.contains(&w.name) {
            return Ok(());
        }
        if out.glyphs.len() == MAX_WANTED {
            return Err(Error::TooLarge { what: "glyphs asked for", size: MAX_WANTED + 1, limit: MAX_WANTED });
        }
        seen.insert(w.name.clone());
        out.glyphs.push(w);
        Ok(())
    };
    for item in items {
        let skip = |out: &mut Resolved, reason: String| out.skipped.push(Skipped { item: item.text(), reason });
        match item {
            Item::Character(c) => push(&mut out, for_code_point(data, u32::from(*c), keep_names, true))?,
            Item::CodePointName(text, cp) => {
                let w =
                    if keep_names { Wanted { name: (*text).to_string(), code_points: vec![*cp] } } else { for_code_point(data, *cp, false, false) };
                push(&mut out, w)?;
            }
            Item::Name(text) => match for_name(data, text, keep_names) {
                Ok(w) => push(&mut out, w)?,
                Err(reason) => skip(&mut out, reason),
            },
            Item::Recipe(_) => skip(&mut out, "glyph recipes are not supported yet".into()),
            Item::Range(first, last) => {
                let (a, b) = match (endpoint(data, first), endpoint(data, last)) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(reason), _) | (_, Err(reason)) => {
                        skip(&mut out, reason);
                        continue;
                    }
                };
                if a > b {
                    skip(&mut out, format!("the range runs backwards, from U+{a:04X} down to U+{b:04X}"));
                    continue;
                }
                let span = b - a + 1;
                if span > MAX_RANGE_CODE_POINTS {
                    return Err(Error::TooLarge { what: "code points in one range", size: span as usize, limit: MAX_RANGE_CODE_POINTS as usize });
                }
                for cp in a..=b {
                    if char::from_u32(cp).is_some() && data.is_assigned(cp) {
                        push(&mut out, for_code_point(data, cp, keep_names, false))?;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The glyph for a code point: the record's nice name, else the code-point name. A typed
/// character that is only a record's legacy code point (U+FB04) gives that record's glyph, with
/// the record's own code point or none.
fn for_code_point(data: &GlyphData, cp: u32, keep_names: bool, typed: bool) -> Wanted {
    if keep_names {
        return Wanted { name: code_point_name(cp), code_points: vec![cp] };
    }
    if let Some(r) = data.by_code_point(cp) {
        return Wanted { name: r.name, code_points: vec![cp] };
    }
    if typed && let Some(r) = data.by_legacy_code_point(cp) {
        return Wanted { name: r.name, code_points: r.unicode.into_iter().collect() };
    }
    Wanted { name: code_point_name(cp), code_points: vec![cp] }
}

/// The glyph for a name item, or why it gives none.
fn for_name(data: &GlyphData, text: &str, keep_names: bool) -> Result<Wanted, String> {
    if text.len() > MAX_ITEM_NAME_BYTES {
        return Err(format!("the name is longer than {MAX_ITEM_NAME_BYTES} bytes"));
    }
    let name = if keep_names { text.to_string() } else { data.canonical_name(text).unwrap_or_else(|| text.to_string()) };
    validate(&name, true).map_err(|e| format!("`{name}` is not a valid glyph name: {e}"))?;
    let code_points = data.lookup(&name, &[]).unicode.map(|u| u.value).into_iter().collect();
    Ok(Wanted { name, code_points })
}

/// The code point a range endpoint stands for.
fn endpoint(data: &GlyphData, text: &str) -> Result<u32, String> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (None, _) => return Err("a range needs an item on both sides of the colon".into()),
        (Some(c), None) => return Ok(u32::from(c)),
        _ => {}
    }
    if let Some(cp) = parse_code_point_name(text) {
        return Ok(cp);
    }
    if text.len() > MAX_ITEM_NAME_BYTES {
        return Err(format!("a range endpoint is longer than {MAX_ITEM_NAME_BYTES} bytes"));
    }
    let name = data.canonical_name(text).unwrap_or_else(|| text.to_string());
    data.lookup(&name, &[]).unicode.map(|u| u.value).ok_or_else(|| format!("`{text}` has no code point, so it cannot end a range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> GlyphData {
        GlyphData::new().unwrap()
    }

    fn names(r: &Resolved) -> Vec<&str> {
        r.glyphs.iter().map(|w| w.name.as_str()).collect()
    }

    #[test]
    fn items_split_at_spaces_tabs_and_line_breaks() {
        let items = parse("A  ä\tuni00E4\nu1F600 a:z x+acutecomb=xacute beh-ar.medi").unwrap();
        assert_eq!(
            items,
            [
                Item::Character('A'),
                Item::Character('ä'),
                Item::CodePointName("uni00E4", 0xE4),
                Item::CodePointName("u1F600", 0x1F600),
                Item::Range("a", "z"),
                Item::Recipe("x+acutecomb=xacute"),
                Item::Name("beh-ar.medi"),
            ]
        );
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn every_item_kind_resolves() {
        let d = data();
        let r = resolve(&d, &parse("Ccedilla ä uni00E4 dotlessi uni0218:uni021B myGlyph x+y=z 1abc").unwrap(), false).unwrap();
        assert_eq!(names(&r), ["Ccedilla", "adieresis", "idotless", "Scommaaccent", "scommaaccent", "Tcommaaccent", "tcommaaccent", "myGlyph"]);
        assert_eq!(r.glyphs.first().map(|w| w.code_points.clone()), Some(vec![0xC7]));
        assert!(r.glyphs.iter().any(|w| w.name == "myGlyph" && w.code_points.is_empty()));
        let reasons: Vec<&str> = r.skipped.iter().map(|s| s.item.as_str()).collect();
        assert_eq!(reasons, ["x+y=z", "1abc"]);
        // Named endpoints: the code points between alpha and omega, unassigned ones left out.
        let greek = resolve(&d, &parse("alpha:omega").unwrap(), false).unwrap();
        assert_eq!(greek.glyphs.first().map(|w| w.name.as_str()), Some("alpha"));
        assert_eq!(greek.glyphs.last().map(|w| w.name.as_str()), Some("omega"));
        assert_eq!(greek.glyphs.len(), 25, "U+03B1 to U+03C9 with final sigma");
    }

    #[test]
    fn custom_naming_keeps_typed_names() {
        let d = data();
        let r = resolve(&d, &parse("uni00E4 dotlessi ä").unwrap(), true).unwrap();
        assert_eq!(names(&r), ["uni00E4", "dotlessi"], "ä gives uni00E4 again, added once");
        assert_eq!(r.glyphs.first().map(|w| w.code_points.clone()), Some(vec![0xE4]));
    }

    #[test]
    fn ranges_are_capped() {
        let d = data();
        // Plane 14 spans exactly the cap; only its assigned code points become glyphs.
        let r = resolve(&d, &parse("uE0000:uEFFFF").unwrap(), false).unwrap();
        assert!(!r.glyphs.is_empty() && r.glyphs.len() < 1000, "{}", r.glyphs.len());
        assert!(matches!(resolve(&d, &parse("uE0000:uF0000").unwrap(), false), Err(Error::TooLarge { .. })));
        assert!(matches!(resolve(&d, &parse("uni0000:u10FFFF").unwrap(), false), Err(Error::TooLarge { .. })));
        let back = resolve(&d, &parse("z:a").unwrap(), false).unwrap();
        assert!(back.glyphs.is_empty() && back.skipped.len() == 1);
        let open = resolve(&d, &parse("a: :b myGlyph:z").unwrap(), false).unwrap();
        assert_eq!(open.skipped.len(), 3, "{:?}", open.skipped);
    }

    #[test]
    fn hostile_text_is_an_error() {
        assert!(matches!(parse(&"a ".repeat(500_000)), Err(Error::TooLarge { .. })));
        assert!(matches!(parse(&"a ".repeat(MAX_ITEMS + 1)), Err(Error::TooLarge { .. })));
        assert_eq!(parse(&"a ".repeat(MAX_ITEMS)).map(|i| i.len()), Ok(MAX_ITEMS));
        let d = data();
        let long = "x".repeat(10_000);
        let r = resolve(&d, &parse(&long).unwrap(), false).unwrap();
        assert!(r.glyphs.is_empty() && r.skipped.len() == 1);
    }
}
