//! Reader for the flat `GlyphData.xml` form: a `glyphData` root holding empty `glyph` elements.
//!
//! Custom glyph data files are untrusted. The reader accepts only that shape: an optional XML
//! declaration, comments, a document type declaration whose internal subset is skipped (entity
//! declarations and parameter-entity references are refused, so nothing expands), the root and
//! its `glyph` children. Everything is bounded by the limits below; a file that breaks one fails
//! with an error naming the line.

use crate::Error;
use crate::record::Record;
use crate::types::{Case, Category, Direction, SubCategory};

/// Largest file accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 16 << 20;
/// Most `glyph` elements in one file.
pub const MAX_RECORDS: usize = 200_000;
/// Longest attribute value, in bytes as written.
pub const MAX_ATTRIBUTE_BYTES: usize = 1024;
/// Most attributes on one element.
pub const MAX_ATTRIBUTES: usize = 32;
/// Most alternative names on one record.
pub const MAX_ALT_NAMES: usize = 64;
/// Most warnings kept for one file.
pub const MAX_WARNINGS: usize = 100;
/// Deepest element nesting: the root and its `glyph` children.
pub const MAX_DEPTH: usize = 2;

/// The records of one file, in file order, and what the reader skipped.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub records: Vec<Record>,
    /// Attributes the format does not define, and similar things that do not fail the file.
    pub warnings: Vec<String>,
}

/// Reads a `GlyphData.xml` file.
pub fn parse(bytes: &[u8]) -> Result<Parsed, Error> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge { what: "glyph data file", size: bytes.len(), limit: MAX_FILE_BYTES });
    }
    let text = std::str::from_utf8(bytes).map_err(|e| Error::Xml { line: Lines::new(bytes).line(e.valid_up_to()), message: "not UTF-8".into() })?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    Reader { s: text.as_bytes(), pos: 0, lines: Lines::new(text.as_bytes()) }.document()
}

/// Line numbers for byte positions, from one scan of the input.
struct Lines(Vec<usize>);

impl Lines {
    fn new(bytes: &[u8]) -> Lines {
        Lines(bytes.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i).collect())
    }

    fn line(&self, pos: usize) -> usize {
        1 + self.0.partition_point(|nl| *nl < pos)
    }
}

struct Reader<'a> {
    s: &'a [u8],
    pos: usize,
    lines: Lines,
}

impl<'a> Reader<'a> {
    fn err<T>(&self, message: impl Into<String>) -> Result<T, Error> {
        Err(Error::Xml { line: self.lines.line(self.pos), message: message.into() })
    }

    fn rest(&self) -> &'a [u8] {
        self.s.get(self.pos..).unwrap_or(&[])
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.rest().starts_with(prefix.as_bytes())
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// Moves past the next occurrence of `end`.
    fn skip_past(&mut self, end: &str, what: &str) -> Result<(), Error> {
        match find(self.rest(), end.as_bytes()) {
            Some(i) => {
                self.pos += i + end.len();
                Ok(())
            }
            None => self.err(format!("unterminated {what}")),
        }
    }

    fn document(mut self) -> Result<Parsed, Error> {
        // Prolog: declaration, comments, processing instructions, one document type declaration.
        loop {
            self.skip_space();
            if self.starts_with("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else if self.starts_with("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.starts_with("<!DOCTYPE") {
                self.doctype()?;
            } else {
                break;
            }
        }
        let (name, attrs, empty) = self.start_tag()?;
        if name != "glyphData" {
            return self.err(format!("the root element is `{name}`, not `glyphData`"));
        }
        drop(attrs);
        let mut out = Parsed::default();
        if !empty {
            self.children(&mut out)?;
        }
        loop {
            self.skip_space();
            if self.starts_with("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.starts_with("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else {
                break;
            }
        }
        if self.pos < self.s.len() {
            return self.err("content after the root element");
        }
        Ok(out)
    }

    /// Skips a document type declaration. Its internal subset may declare elements and
    /// attributes; an entity declaration or a parameter-entity reference fails the file.
    fn doctype(&mut self) -> Result<(), Error> {
        self.pos += "<!DOCTYPE".len();
        loop {
            match self.peek() {
                None => return self.err("unterminated document type declaration"),
                Some(b'>') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b'[') => {
                    self.pos += 1;
                    self.internal_subset()?;
                }
                Some(q @ (b'"' | b'\'')) => {
                    self.pos += 1;
                    self.skip_past(if q == b'"' { "\"" } else { "'" }, "quoted text")?;
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn internal_subset(&mut self) -> Result<(), Error> {
        loop {
            self.skip_space();
            if self.starts_with("]") {
                self.pos += 1;
                return Ok(());
            } else if self.starts_with("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.starts_with("<!ENTITY") {
                return self.err("entity declarations are not accepted");
            } else if self.starts_with("<!") || self.starts_with("<?") {
                // A markup declaration: skip to its `>`, stepping over quoted text.
                self.pos += 2;
                loop {
                    match self.peek() {
                        None => return self.err("unterminated markup declaration"),
                        Some(b'>') => {
                            self.pos += 1;
                            break;
                        }
                        Some(q @ (b'"' | b'\'')) => {
                            self.pos += 1;
                            self.skip_past(if q == b'"' { "\"" } else { "'" }, "quoted text")?;
                        }
                        Some(_) => self.pos += 1,
                    }
                }
            } else if self.starts_with("%") {
                return self.err("parameter-entity references are not accepted");
            } else if self.peek().is_none() {
                return self.err("unterminated document type declaration");
            } else {
                return self.err("unexpected text in the document type declaration");
            }
        }
    }

    fn name(&mut self) -> Result<&'a str, Error> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')) {
            self.pos += 1;
        }
        if self.pos == start || self.pos - start > 64 {
            return self.err("expected a name");
        }
        // The bytes are ASCII, checked above.
        std::str::from_utf8(self.s.get(start..self.pos).unwrap_or(&[])).or_else(|_| self.err("expected a name"))
    }

    /// Reads `<name attr="value" …>` or `<name … />`. Returns the attributes with their values
    /// decoded, and whether the element is empty.
    #[allow(clippy::type_complexity)]
    fn start_tag(&mut self) -> Result<(&'a str, Vec<(&'a str, String, usize)>, bool), Error> {
        if self.peek() != Some(b'<') {
            return self.err("expected an element");
        }
        self.pos += 1;
        let name = self.name()?;
        let mut attrs = Vec::new();
        loop {
            let before = self.pos;
            self.skip_space();
            match self.peek() {
                Some(b'/') if self.starts_with("/>") => {
                    self.pos += 2;
                    return Ok((name, attrs, true));
                }
                Some(b'>') => {
                    self.pos += 1;
                    return Ok((name, attrs, false));
                }
                None => return self.err(format!("unterminated `{name}` tag")),
                Some(_) if self.pos == before => return self.err(format!("expected a space before an attribute of `{name}`")),
                Some(_) => {}
            }
            if attrs.len() >= MAX_ATTRIBUTES {
                return self.err(format!("more than {MAX_ATTRIBUTES} attributes on one element"));
            }
            let line = self.lines.line(self.pos);
            let key = self.name()?;
            self.skip_space();
            if self.peek() != Some(b'=') {
                return self.err(format!("expected `=` after `{key}`"));
            }
            self.pos += 1;
            self.skip_space();
            let quote = match self.peek() {
                Some(q @ (b'"' | b'\'')) => q,
                _ => return self.err(format!("expected a quoted value for `{key}`")),
            };
            self.pos += 1;
            let start = self.pos;
            let Some(len) = self.rest().iter().position(|b| *b == quote) else {
                return self.err(format!("unterminated value of `{key}`"));
            };
            if len > MAX_ATTRIBUTE_BYTES {
                return self.err(format!("the value of `{key}` is longer than {MAX_ATTRIBUTE_BYTES} bytes"));
            }
            let raw = self.s.get(start..start + len).unwrap_or(&[]);
            self.pos += len + 1;
            if attrs.iter().any(|(k, _, _)| *k == key) {
                return self.err(format!("`{key}` is given twice"));
            }
            let value = decode(raw).map_err(|message| Error::Xml { line, message: format!("in `{key}`: {message}") })?;
            attrs.push((key, value, line));
        }
    }

    /// The children of the root: `glyph` elements, comments and white space.
    fn children(&mut self, out: &mut Parsed) -> Result<(), Error> {
        loop {
            self.skip_space();
            if self.starts_with("<!--") {
                self.skip_past("-->", "comment")?;
            } else if self.starts_with("<?") {
                self.skip_past("?>", "processing instruction")?;
            } else if self.starts_with("</") {
                self.pos += 2;
                let name = self.name()?;
                self.skip_space();
                if name != "glyphData" || self.peek() != Some(b'>') {
                    return self.err("expected `</glyphData>`");
                }
                self.pos += 1;
                return Ok(());
            } else if self.starts_with("<!") {
                return self.err("unexpected markup declaration");
            } else if self.peek() == Some(b'<') {
                let line = self.lines.line(self.pos);
                let (name, attrs, empty) = self.start_tag()?;
                if name != "glyph" {
                    return self.err(format!("unexpected element `{name}` (only `glyph` may appear in `glyphData`)"));
                }
                if !empty {
                    // `<glyph …></glyph>` is fine; anything inside it is nesting the format does
                    // not have.
                    self.skip_space();
                    if !self.starts_with("</glyph") {
                        return self.err(format!("elements nest deeper than {MAX_DEPTH} levels"));
                    }
                    self.pos += "</glyph".len();
                    self.skip_space();
                    if self.peek() != Some(b'>') {
                        return self.err("expected `</glyph>`");
                    }
                    self.pos += 1;
                }
                if out.records.len() >= MAX_RECORDS {
                    return self.err(format!("more than {MAX_RECORDS} records"));
                }
                out.records.push(record(attrs, line, &mut out.warnings)?);
            } else if self.peek().is_none() {
                return self.err("missing `</glyphData>`");
            } else {
                return self.err("unexpected text");
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Decodes the predefined entities and character references of an attribute value and
/// normalises white space characters to spaces, as XML does for attribute values.
fn decode(raw: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(raw).map_err(|_| "not UTF-8".to_string())?;
    if text.contains('<') {
        return Err("`<` in an attribute value".into());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['&', '\t', '\n', '\r']) {
        out.push_str(rest.get(..i).unwrap_or(""));
        let tail = rest.get(i..).unwrap_or("");
        if !tail.starts_with('&') {
            out.push(' ');
            rest = tail.get(1..).unwrap_or("");
            continue;
        }
        let Some(end) = tail.find(';') else {
            return Err("`&` without `;`".into());
        };
        let entity = tail.get(1..end).unwrap_or("");
        let ch = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok()
                } else {
                    return Err(format!("undeclared entity `&{entity};`"));
                };
                code.and_then(char::from_u32).filter(|c| *c != '\0').ok_or_else(|| format!("bad character reference `&{entity};`"))?
            }
        };
        out.push(ch);
        rest = tail.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(out)
}

fn hex_code_point(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 6 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(text, 16).ok().filter(|cp| char::from_u32(*cp).is_some())
}

fn record(attrs: Vec<(&str, String, usize)>, line: usize, warnings: &mut Vec<String>) -> Result<Record, Error> {
    let bad = |line: usize, message: String| Error::Xml { line, message };
    let mut r = Record::default();
    let mut has_name = false;
    for (key, value, at) in attrs {
        let text = || (!value.is_empty()).then(|| value.clone());
        match key {
            "name" => {
                r.name = value.trim().to_string();
                has_name = true;
            }
            "category" => r.category = Some(Category::parse(value.trim()).ok_or_else(|| bad(at, format!("unknown category `{value}`")))?),
            "subCategory" => {
                r.sub_category = Some(SubCategory::parse(value.trim()).ok_or_else(|| bad(at, format!("unknown subcategory `{value}`")))?)
            }
            "unicode" => r.unicode = Some(hex_code_point(value.trim()).ok_or_else(|| bad(at, format!("bad code point `{value}`")))?),
            "unicodeLegacy" => {
                r.unicode_legacy = Some(hex_code_point(value.trim()).ok_or_else(|| bad(at, format!("bad legacy code point `{value}`")))?)
            }
            "case" => {
                r.case = match value.trim() {
                    "" | "noCase" => None,
                    v => Some(Case::parse(v).ok_or_else(|| bad(at, format!("unknown case `{value}`")))?),
                }
            }
            "direction" => r.direction = Some(Direction::parse(value.trim()).ok_or_else(|| bad(at, format!("unknown direction `{value}`")))?),
            "script" => r.script = text(),
            "description" => r.description = text(),
            "production" => r.production = text(),
            "altNames" => {
                let names: Vec<String> = value.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect();
                if names.len() > MAX_ALT_NAMES {
                    return Err(bad(at, format!("more than {MAX_ALT_NAMES} alternative names")));
                }
                r.alt_names = names;
            }
            "sortName" => r.sort_name = text(),
            "sortNameKeep" => r.sort_name_keep = text(),
            "decompose" => r.decompose = text(),
            "anchors" => r.anchors = text(),
            // `accents` is the version 2 spelling of `marks`.
            "marks" | "accents" => r.marks = text(),
            other => {
                if warnings.len() < MAX_WARNINGS {
                    warnings.push(format!("line {at}: unknown attribute `{other}` skipped"));
                }
            }
        }
    }
    if !has_name || r.name.is_empty() {
        return Err(bad(line, "a `glyph` without a `name`".into()));
    }
    if r.category.is_none() {
        return Err(bad(line, format!("`{}` has no `category`", r.name)));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<!DOCTYPE glyphData [
<!ELEMENT glyphData (glyph)+>
<!ATTLIST glyph
	name CDATA #REQUIRED
	category CDATA #REQUIRED>
]>
<glyphData format="2">
	<!-- a comment -->
	<glyph unicode="0041" name="A" category="Letter" case="upper" script="latin" altNames="Alpha-latin, Aaa" description="LATIN &amp; CAPITAL" />
	<glyph name="myGlyph" category="Mark" subCategory="Spacing Combining" production="uniE000" sortName="zzz"></glyph>
</glyphData>
"#;

    #[test]
    fn reads_the_flat_form() {
        let p = parse(SMALL.as_bytes()).unwrap();
        assert_eq!(p.records.len(), 2);
        let a = &p.records[0];
        assert_eq!((a.name.as_str(), a.unicode, a.case), ("A", Some(0x41), Some(Case::Upper)));
        assert_eq!(a.alt_names, ["Alpha-latin", "Aaa"]);
        assert_eq!(a.description.as_deref(), Some("LATIN & CAPITAL"));
        let b = &p.records[1];
        assert_eq!(b.sub_category, Some(SubCategory::SpacingCombining));
        assert_eq!(b.sort_name.as_deref(), Some("zzz"));
    }

    #[test]
    fn hostile_files_fail_with_a_message() {
        let entity =
            r#"<!DOCTYPE glyphData [<!ENTITY a "aaaaaaaa"><!ENTITY b "&a;&a;&a;">]><glyphData><glyph name="&b;" category="Letter"/></glyphData>"#;
        let deep = format!("<glyphData>{}</glyphData>", "<glyph name=\"a\" category=\"Letter\">".repeat(10_000));
        let long = format!("<glyphData><glyph name=\"{}\" category=\"Letter\"/></glyphData>", "a".repeat(1 << 20));
        let parameter = "<!DOCTYPE glyphData [ %p; ]><glyphData/>";
        let undeclared = r#"<glyphData><glyph name="&x;" category="Letter"/></glyphData>"#;
        let many = format!("<glyphData>{}</glyphData>", "<glyph name=\"a\" category=\"Letter\"/>".repeat(MAX_RECORDS + 1));
        for (what, text) in [
            ("entity", entity.to_string()),
            ("deep", deep),
            ("long attribute", long),
            ("parameter entity", parameter.to_string()),
            ("undeclared entity", undeclared.to_string()),
            ("too many records", many),
            ("no root", String::new()),
            ("unterminated", "<glyphData><glyph name=\"a".into()),
            ("other root", "<plist/>".into()),
            ("missing category", "<glyphData><glyph name=\"a\"/></glyphData>".into()),
            ("bad category", "<glyphData><glyph name=\"a\" category=\"Lettre\"/></glyphData>".into()),
            ("bad code point", "<glyphData><glyph name=\"a\" category=\"Letter\" unicode=\"D800\"/></glyphData>".into()),
        ] {
            let e = parse(text.as_bytes());
            assert!(e.is_err(), "{what} was accepted");
            assert!(!e.unwrap_err().to_string().is_empty());
        }
        let huge = vec![b' '; MAX_FILE_BYTES + 1];
        assert!(matches!(parse(&huge), Err(Error::TooLarge { .. })));
    }

    #[test]
    fn unknown_attributes_are_warnings() {
        let p = parse(br#"<glyphData><glyph name="a" category="Letter" colour="red"/></glyphData>"#).unwrap();
        assert_eq!(p.records.len(), 1);
        assert_eq!(p.warnings.len(), 1);
    }
}
