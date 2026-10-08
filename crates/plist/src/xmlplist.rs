//! XML property lists, as UFO and designspace files hold them.
//!
//! Canonical spelling is what fontTools' `plistlib` writes with lxml's pretty printer, which is
//! what most UFOs on disk look like (written from the fontTools source, MIT; see `NOTICE`):
//!
//! - A document is the declaration, the Apple document type and a `<plist version="1.0">`
//!   element whose value sits one level in. The declaration and document type are kept as read.
//! - Containers put each child on its own line, indented one unit deeper than the container, and
//!   the end tag on a line at the container's level. An empty container is `<dict/>` or
//!   `<array/>`. A container written on one line is [`Layout::Compact`] (an empty one is then
//!   `<dict></dict>`).
//! - `<integer>` is the decimal value; `<real>` is Python's `repr` of the float (`1.0`, `0.5`,
//!   `1e-05`). [`Num::Int`] is written as an integer and [`Num::Float`] as a real, so the two
//!   element types survive a round trip.
//! - `<string>` and `<key>` escape `&`, `<` and `>`; an empty string is `<string></string>`.
//! - `<data>` is base64 broken into lines of at most 76 characters, counting an indentation of two
//!   spaces per level of the element (the closing tag stays on that indentation, as fontTools
//!   writes it).
//!
//! A scalar whose source differs keeps the whole element's source text as its spelling. Spellings
//! here are XML spellings (see the crate documentation).
//!
//! The indentation unit is the document's own: two spaces in files fontTools writes today, a tab
//! in many older ones. [`Document::indent`] records it and [`write`] uses it.

use crate::xml::{self, Element, Node};
use crate::{Array, Data, Dict, Error, Layout, MAX_DEPTH, Num, Number, Spacing, Str, Value};

/// The declaration fontTools writes, as the text between `<?` and `?>`.
pub const DECLARATION: &str = "xml version='1.0' encoding='UTF-8'";
/// The Apple document type, as the text between `<!DOCTYPE` and `>`.
pub const DOCTYPE: &str = " plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"";

/// A parsed property-list file.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub bom: bool,
    /// Declaration, document type and the whitespace between them, as read.
    pub prolog: Vec<Node>,
    /// The `<plist>` element itself (its attributes), without children.
    pub plist: Element,
    /// What the file has between `<plist>` and the value, and between the value and `</plist>`.
    pub before: String,
    pub after: String,
    pub root: Value,
    pub epilog: Vec<Node>,
    /// The indentation unit, taken from the line the value starts on.
    pub indent: String,
    /// Containers whose whitespace was neither the canonical layout nor compact (comments inside
    /// count). Their gaps are kept ([`crate::Spacing`]) and written back while the container has
    /// the same number of children.
    pub irregular: usize,
}

impl Document {
    /// A new file as fontTools writes it, with two-space indentation.
    pub fn new(root: Value) -> Self {
        Self::with_indent(root, "  ")
    }

    pub fn with_indent(root: Value, indent: &str) -> Self {
        let text = xml::Text::new("\n");
        Self {
            bom: false,
            prolog: vec![Node::Pi(DECLARATION.into()), Node::Text(text.clone()), Node::Doctype(DOCTYPE.into()), Node::Text(text.clone())],
            plist: Element::new("plist").with_attr("version", "1.0"),
            before: format!("\n{indent}"),
            after: "\n".into(),
            root,
            epilog: vec![Node::Text(text)],
            indent: indent.to_string(),
            irregular: 0,
        }
    }
}

/// Parses a whole property-list file.
pub fn parse(text: &str) -> Result<Document, Error> {
    let doc = xml::parse(text)?;
    let err = |m: &str| Error::at(text, 0, m);
    let plist = doc.root;
    if plist.name() != "plist" {
        return Err(err("the root element is not `<plist>`"));
    }
    let mut before = String::new();
    let mut after = String::new();
    let mut value: Option<&Element> = None;
    for n in &plist.children {
        match n {
            Node::Element(e) if value.is_none() => value = Some(e),
            Node::Element(_) => return Err(err("`<plist>` holds more than one value")),
            Node::Text(t) if t.is_whitespace() => {
                let mut s = String::new();
                write_text(&mut s, t);
                if value.is_none() { before.push_str(&s) } else { after.push_str(&s) }
            }
            _ => return Err(err("`<plist>` holds something other than one value")),
        }
    }
    let Some(value) = value else { return Err(err("`<plist>` is empty")) };
    let indent = before.rsplit('\n').next().unwrap_or("").to_string();
    let mut irregular = 0;
    let root = read_value(value, &indent, 1, &mut irregular)?;
    let mut shell = plist.clone();
    shell.children.clear();
    Ok(Document { bom: doc.bom, prolog: doc.prolog, plist: shell, before, after, root, epilog: doc.epilog, indent, irregular })
}

fn write_text(out: &mut String, t: &xml::Text) {
    match t.raw() {
        Some(r) => out.push_str(r),
        None => xml::escape_text(out, t.as_str()),
    }
}

/// Writes a document. For a document [`parse`] returned unchanged (and without irregular
/// containers), the result is the input.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    if doc.bom {
        out.push('\u{feff}');
    }
    for n in &doc.prolog {
        xml::write_node(&mut out, n);
    }
    let mut plist = doc.plist.clone();
    plist.set_self_closing(false);
    let _ = plist.write_start(&mut out);
    out.push_str(&doc.before);
    write_value(&mut out, &doc.root, &doc.indent, 1);
    out.push_str(&doc.after);
    plist.write_end(&mut out);
    for n in &doc.epilog {
        xml::write_node(&mut out, n);
    }
    out
}

/// Reads the value element `el`, which sits at `level` (the depth of the element in its document)
/// in a file indented by `indent` per level.
pub fn read_value(el: &Element, indent: &str, level: usize, irregular: &mut usize) -> Result<Value, Error> {
    if level > MAX_DEPTH {
        return Err(Error { line: 1, column: 1, message: format!("values nest deeper than {MAX_DEPTH}") });
    }
    let bad = |m: String| Error { line: 1, column: 1, message: m };
    let source = || {
        let mut s = String::new();
        el.write(&mut s);
        s
    };
    let value = match el.name() {
        "dict" => {
            let (children, layout, spacing) = container_children(el, indent, level, irregular)?;
            let mut d = Dict { entries: Vec::with_capacity(children.len() / 2), layout, spacing };
            let mut it = children.into_iter();
            while let Some(k) = it.next() {
                if k.name() != "key" {
                    return Err(bad(format!("a `<dict>` holds `<{}>` where a `<key>` belongs", k.name())));
                }
                let Some(v) = it.next() else { return Err(bad("a `<dict>` key has no value".into())) };
                let key = scalar_text(k)?;
                let canonical = canonical_scalar("key", &key);
                let mut src = String::new();
                k.write(&mut src);
                d.entries.push((Str::with_spelling(key, (canonical != src).then(|| src.into())), read_value(v, indent, level + 1, irregular)?));
            }
            return Ok(Value::Dict(d));
        }
        "array" => {
            let (children, layout, spacing) = container_children(el, indent, level, irregular)?;
            let items = children.into_iter().map(|c| read_value(c, indent, level + 1, irregular)).collect::<Result<_, _>>()?;
            return Ok(Value::Array(Array { items, layout, spacing }));
        }
        "string" => Value::String(Str::new(scalar_text(el)?)),
        "date" => Value::Date(Str::new(scalar_text(el)?)),
        "integer" => {
            let t = scalar_text(el)?;
            let t = t.trim();
            match t.parse::<i64>() {
                Ok(i) => Value::Number(Number::int(i)),
                Err(_) => match t.parse::<f64>().ok().filter(|f| f.is_finite() && f.fract() == 0.0) {
                    // Beyond i64 (plistlib allows up to 2^64): kept as a float that remembers its text.
                    Some(f) => Value::Number(Number::with_spelling(Num::Float(f), Some(source().into()))),
                    None => return Err(bad(format!("`<integer>{t}</integer>` is not a whole number"))),
                },
            }
        }
        "real" => {
            let t = scalar_text(el)?;
            match t.trim().parse::<f64>().ok().and_then(Number::float) {
                Some(n) => Value::Number(n),
                None => return Err(bad(format!("`<real>{t}</real>` is not a finite number"))),
            }
        }
        "true" | "false" => {
            if !el.children.is_empty() {
                return Err(bad(format!("`<{}>` has content", el.name())));
            }
            Value::Bool(el.name() == "true")
        }
        "data" => {
            let t = scalar_text(el)?;
            Value::Data(Data::new(base64_decode(&t).ok_or_else(|| bad("`<data>` is not base64".into()))?))
        }
        other => return Err(bad(format!("`<{other}>` is not a property-list value"))),
    };
    // A scalar keeps its source when the canonical writer would spell it differently.
    let mut canonical = String::new();
    write_value(&mut canonical, &value, indent, level);
    let src = source();
    Ok(if canonical == src { value } else { with_spelling(value, src) })
}

fn with_spelling(v: Value, spelling: String) -> Value {
    let sp = Some(spelling.into_boxed_str());
    match v {
        Value::String(s) => Value::String(Str::with_spelling(s.as_str().to_string(), sp)),
        Value::Date(s) => Value::Date(Str::with_spelling(s.as_str().to_string(), sp)),
        Value::Number(n) => Value::Number(Number::with_spelling(n.value(), sp)),
        Value::Data(d) => Value::Data(Data::with_spelling(d.bytes().to_vec(), sp)),
        // Booleans have one spelling worth keeping; containers keep a layout instead.
        other => other,
    }
}

/// The text of a scalar element, which may hold only text and CDATA.
fn scalar_text(el: &Element) -> Result<String, Error> {
    if el.children.iter().any(|n| !matches!(n, Node::Text(_) | Node::CData(_))) {
        return Err(Error { line: 1, column: 1, message: format!("`<{}>` holds markup", el.name()) });
    }
    Ok(el.text())
}

/// The child elements of a container, its layout, and its whitespace when that is irregular.
fn container_children<'a>(el: &'a Element, indent: &str, level: usize, irregular: &mut usize) -> Result<(Vec<&'a Element>, Layout, Spacing), Error> {
    let children: Vec<&Element> = el.elements().collect();
    let mut texts = Vec::new();
    for n in &el.children {
        match n {
            Node::Element(_) => {}
            Node::Text(t) if t.is_whitespace() => texts.push(t.as_str()),
            Node::Comment(_) | Node::Pi(_) => texts.push("<!---->"),
            _ => return Err(Error { line: 1, column: 1, message: format!("`<{}>` holds text", el.name()) }),
        }
    }
    let compact = texts.is_empty() && (!children.is_empty() || !el.self_closing());
    if compact {
        return Ok((children, Layout::Compact, Spacing::default()));
    }
    // Canonical: whitespace before each child and before the end tag, nothing else.
    let inner = format!("\n{}", indent.repeat(level + 1));
    let outer = format!("\n{}", indent.repeat(level));
    let regular = if children.is_empty() {
        el.self_closing()
    } else {
        let mut expect: Vec<Option<&str>> = Vec::new();
        for n in &el.children {
            match n {
                Node::Element(_) => expect.push(None),
                Node::Text(t) => expect.push(Some(t.as_str())),
                _ => {}
            }
        }
        // Text, element, text, element, …, text.
        expect.len() == children.len() * 2 + 1
            && expect.iter().enumerate().all(|(i, t)| match (i % 2, t) {
                (1, None) => true,
                (0, Some(t)) => *t == if i + 1 == expect.len() { outer.as_str() } else { inner.as_str() },
                _ => false,
            })
    };
    let markup = el.children.iter().any(|n| matches!(n, Node::Comment(_) | Node::Pi(_)));
    if regular && !markup {
        return Ok((children, Layout::Multiline, Spacing::default()));
    }
    *irregular += 1;
    // Keep the whitespace before each child element and before the end tag.
    let mut gaps = vec![String::new()];
    for n in &el.children {
        match n {
            Node::Element(_) => gaps.push(String::new()),
            // Comments and processing instructions travel with the whitespace around them.
            other => {
                if let Some(g) = gaps.last_mut() {
                    xml::write_node(g, other);
                }
            }
        }
    }
    Ok((children, Layout::Multiline, Spacing::new(gaps)))
}

/// Writes `value` as an element at `level` of a document indented by `indent` per level.
pub fn write_value(out: &mut String, value: &Value, indent: &str, level: usize) {
    match value {
        Value::Dict(d) => {
            let multiline = d.layout == Layout::Multiline;
            if d.entries.is_empty() && d.spacing.gaps(0).is_none() {
                out.push_str(if multiline { "<dict/>" } else { "<dict></dict>" });
                return;
            }
            out.push_str("<dict>");
            let gaps = d.spacing.gaps(d.entries.len() * 2);
            for (i, (k, v)) in d.entries.iter().enumerate() {
                gap(out, gaps, i * 2, multiline, indent, level + 1);
                match k.spelling() {
                    Some(sp) => out.push_str(sp),
                    None => out.push_str(&canonical_scalar("key", k.as_str())),
                }
                gap(out, gaps, i * 2 + 1, multiline, indent, level + 1);
                write_value(out, v, indent, level + 1);
            }
            gap(out, gaps, d.entries.len() * 2, multiline, indent, level);
            out.push_str("</dict>");
        }
        Value::Array(a) => {
            let multiline = a.layout == Layout::Multiline;
            if a.items.is_empty() && a.spacing.gaps(0).is_none() {
                out.push_str(if multiline { "<array/>" } else { "<array></array>" });
                return;
            }
            out.push_str("<array>");
            let gaps = a.spacing.gaps(a.items.len());
            for (i, v) in a.items.iter().enumerate() {
                gap(out, gaps, i, multiline, indent, level + 1);
                write_value(out, v, indent, level + 1);
            }
            gap(out, gaps, a.items.len(), multiline, indent, level);
            out.push_str("</array>");
        }
        Value::String(s) => match s.spelling() {
            Some(sp) => out.push_str(sp),
            None => out.push_str(&canonical_scalar("string", s.as_str())),
        },
        Value::Date(s) => match s.spelling() {
            Some(sp) => out.push_str(sp),
            None => out.push_str(&canonical_scalar("date", s.as_str())),
        },
        Value::Number(n) => match n.spelling() {
            Some(sp) => out.push_str(sp),
            None => match n.value() {
                Num::Int(i) => {
                    out.push_str("<integer>");
                    out.push_str(&i.to_string());
                    out.push_str("</integer>");
                }
                Num::Float(f) => {
                    out.push_str("<real>");
                    out.push_str(&py_float_repr(f));
                    out.push_str("</real>");
                }
            },
        },
        Value::Bool(b) => out.push_str(if *b { "<true/>" } else { "<false/>" }),
        Value::Data(d) => match d.spelling() {
            Some(sp) => out.push_str(sp),
            None => {
                out.push_str("<data>");
                out.push_str(&wrapped_base64(d.bytes(), level));
                out.push_str("</data>");
            }
        },
    }
}

/// The whitespace before a child (or the end tag): the kept gap, or the canonical line break.
fn gap(out: &mut String, gaps: Option<&[Box<str>]>, i: usize, multiline: bool, indent: &str, level: usize) {
    match gaps.and_then(|g| g.get(i)) {
        Some(g) => out.push_str(g),
        None if multiline => newline(out, indent, level),
        None => {}
    }
}

fn newline(out: &mut String, indent: &str, level: usize) {
    out.push('\n');
    for _ in 0..level {
        out.push_str(indent);
    }
}

fn canonical_scalar(tag: &str, text: &str) -> String {
    let mut s = String::with_capacity(text.len() + 2 * tag.len() + 5);
    s.push('<');
    s.push_str(tag);
    s.push('>');
    xml::escape_text(&mut s, text);
    s.push_str("</");
    s.push_str(tag);
    s.push('>');
    s
}

/// Writes a value as a one-line fragment (compact containers, no indentation), for keeping a
/// value inside other text. [`parse_fragment`] reads it back with the same spellings.
pub fn write_fragment(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, "", 0);
    out
}

/// Reads a fragment written by [`write_fragment`] (or any single value element).
pub fn parse_fragment(text: &str) -> Result<Value, Error> {
    let el = xml::parse_element(text)?;
    let mut irregular = 0;
    read_value(&el, "", 0, &mut irregular)
}

/// Python's `repr` of a float, which is how fontTools writes reals and GLIF coordinates: the
/// shortest digits that read back to the same value, in positional notation when the decimal
/// exponent is from -4 to 15 (always with a fractional part, `1.0`), otherwise in scientific
/// notation with a signed exponent of at least two digits (`1e-05`, `1.5e+16`).
pub fn py_float_repr(f: f64) -> String {
    if !f.is_finite() {
        return if f.is_nan() {
            "nan".into()
        } else if f > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    // Rust's `{:e}` gives the shortest round-trip digits: `-1.2345e-7`.
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = usize::try_from(exp).unwrap_or(0) + 1;
            let int_part: String = digits.chars().chain(std::iter::repeat('0')).take(int_len).collect();
            let frac: String = digits.chars().skip(int_len).collect();
            out.push_str(&int_part);
            out.push('.');
            out.push_str(if frac.is_empty() { "0" } else { &frac });
        } else {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
    } else {
        let mut chars = digits.chars();
        out.extend(chars.next());
        let rest: String = chars.collect();
        if !rest.is_empty() {
            out.push('.');
            out.push_str(&rest);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.unsigned_abs()));
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                let idx = usize::try_from((n >> shift) & 63).unwrap_or(0);
                out.push(char::from(B64.get(idx).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut padding = false;
    for c in text.bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding = true;
            continue;
        }
        if padding {
            return None;
        }
        let v = u32::try_from(B64.iter().position(|&b| b == c)?).ok()?;
        acc = ((acc << 6) | v) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// Base64 as fontTools writes it inside a `<data>` element at `level`: lines of at most 76
/// characters including an indentation of two spaces per level, each on its own line, and the
/// closing tag on the same indentation.
fn wrapped_base64(bytes: &[u8], level: usize) -> String {
    let data = base64_encode(bytes);
    if data.is_empty() {
        return data;
    }
    let indent = format!("\n{}", "  ".repeat(level));
    let max = 76usize.saturating_sub(indent.len()).max(16);
    let mut out = String::new();
    let mut rest = data.as_str();
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(max.min(rest.len()));
        out.push_str(&indent);
        out.push_str(line);
        rest = tail;
    }
    out.push_str(&indent);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n  <dict>\n    <key>creator</key>\n    <string>com.github.fonttools.ufoLib</string>\n    <key>formatVersion</key>\n    <integer>3</integer>\n  </dict>\n</plist>\n";

    #[test]
    fn a_fonttools_file_round_trips_without_spellings() {
        let doc = parse(META).unwrap();
        assert_eq!(doc.indent, "  ");
        assert_eq!(doc.root.spelling_count(), 0);
        assert_eq!(doc.irregular, 0);
        assert_eq!(write(&doc), META);
        let d = doc.root.as_dict().unwrap();
        assert_eq!(d.get("formatVersion").and_then(Value::as_num), Some(Num::Int(3)));
    }

    #[test]
    fn a_new_document_is_written_as_fonttools_writes_it() {
        let mut d = Dict::new();
        d.insert("creator", Value::String("com.github.fonttools.ufoLib".into()));
        d.insert("formatVersion", Value::Number(Number::int(3)));
        assert_eq!(write(&Document::new(Value::Dict(d))), META);
    }

    #[test]
    fn tab_indented_files_keep_their_tabs() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n\t<dict>\n\t\t<key>a</key>\n\t\t<array>\n\t\t\t<real>0.5</real>\n\t\t\t<true/>\n\t\t</array>\n\t\t<key>b</key>\n\t\t<dict/>\n\t</dict>\n</plist>\n";
        let doc = parse(text).unwrap();
        assert_eq!(doc.indent, "\t");
        assert_eq!((doc.root.spelling_count(), doc.irregular), (0, 0));
        assert_eq!(write(&doc), text);
    }

    #[test]
    fn integers_and_reals_stay_apart() {
        let v = parse_fragment("<array><integer>1</integer><real>1.0</real><real>1e-05</real></array>").unwrap();
        let items = &v.as_array().unwrap().items;
        assert_eq!(items[0].as_num(), Some(Num::Int(1)));
        assert_eq!(items[1].as_num(), Some(Num::Float(1.0)));
        assert_eq!(v.spelling_count(), 0, "these are the canonical spellings");
        assert_eq!(write_fragment(&v), "<array><integer>1</integer><real>1.0</real><real>1e-05</real></array>");
    }

    #[test]
    fn odd_spellings_are_kept() {
        let text = "<dict><key>a&#46;b</key><string/><key>n</key><integer> 7 </integer><key>r</key><real>2.50</real></dict>";
        let v = parse_fragment(text).unwrap();
        assert_eq!(v.spelling_count(), 4);
        assert_eq!(write_fragment(&v), text);
        assert_eq!(
            write_fragment(&v.without_spellings()),
            "<dict><key>a.b</key><string></string><key>n</key><integer>7</integer><key>r</key><real>2.5</real></dict>"
        );
    }

    #[test]
    fn data_is_wrapped_by_level() {
        let v = Value::Data(Data::new((0u8..=99).collect()));
        let mut out = String::new();
        write_value(&mut out, &v, "  ", 2);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "<data>");
        assert!(lines[1].starts_with("    ") && lines[1].len() == 4 + 71, "{:?}", lines[1]);
        assert_eq!(*lines.last().unwrap(), "    </data>");
        let back = parse_fragment(&out).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn python_float_repr() {
        for (f, s) in [
            (1.0, "1.0"),
            (0.5, "0.5"),
            (-12.25, "-12.25"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-7, "1.5e-07"),
            (123456789012345.6, "123456789012345.6"),
            (1e16, "1e+16"),
            (1.2345e20, "1.2345e+20"),
            (100.0, "100.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(py_float_repr(f), s, "{f}");
        }
    }

    #[test]
    fn base64_round_trips() {
        for n in 0..10 {
            let bytes: Vec<u8> = (0..n).map(|i: u8| i.wrapping_mul(37)).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert!(base64_decode("TW=E").is_none());
    }

    #[test]
    fn malformed_values_are_errors() {
        for bad in [
            "<dict><string>x</string></dict>",
            "<dict><key>a</key></dict>",
            "<integer>x</integer>",
            "<real>nan</real>",
            "<true>1</true>",
            "<data>!!</data>",
            "<string><b/></string>",
            "<wat/>",
        ] {
            assert!(parse_fragment(bad).is_err(), "{bad}");
        }
        assert!(parse("<a/>").is_err());
        assert!(parse("<plist/>").is_err());
        assert!(parse("<plist><dict/><dict/></plist>").is_err());
    }
}
