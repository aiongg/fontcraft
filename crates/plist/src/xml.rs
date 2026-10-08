//! An XML tree that writes back the bytes it was read from.
//!
//! UFO and designspace files are XML. Their readers need a tree; their writers need to put back
//! what a file said in the way it said it, so that saving an unchanged document changes no byte
//! (`roundtrip.ufoXmlFormatting`, `roundtrip.designspaceVerbatim`). This module keeps:
//!
//! - every node: elements, text (whitespace included), comments, CDATA sections, processing
//!   instructions, the document type declaration, and what surrounds the root element;
//! - every spelling: a start tag, end tag or text whose source differs from the canonical
//!   rendering keeps its source text, and the writer uses it until the node is changed.
//!
//! The canonical rendering is the one lxml (libxml2) produces, which is what fontTools writes:
//! attributes in double quotes with `&`, `<`, `>`, `"`, tab, line feed and carriage return escaped;
//! text with `&`, `<`, `>` and carriage return escaped; an element without children written
//! `<name/>`. [`Element::write_pretty`] adds the indentation lxml's pretty printer adds.
//!
//! Not supported, and an error: entities declared in a document type (only the five predefined
//! entities and character references are known) and nesting deeper than [`MAX_DEPTH`].
//! All input is untrusted; parsing never panics and is bounded by [`MAX_INPUT`].

use crate::{Error, MAX_DEPTH, MAX_INPUT};

/// Most attributes one element may have.
pub const MAX_ATTRIBUTES: usize = 4096;

/// A parsed file.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// The file began with a UTF-8 byte-order mark.
    pub bom: bool,
    /// What comes before the root element: the XML declaration (a processing instruction), the
    /// document type, comments and the whitespace between them.
    pub prolog: Vec<Node>,
    pub root: Element,
    /// What comes after the root element: normally one line break.
    pub epilog: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(Text),
    /// The text between `<!--` and `-->`.
    Comment(String),
    /// The text between `<![CDATA[` and `]]>`.
    CData(String),
    /// The text between `<?` and `?>`, the XML declaration included.
    Pi(String),
    /// The text between `<!DOCTYPE` and the closing `>`.
    Doctype(String),
}

/// Character data, decoded, with its source text when that is not the canonical escaping.
#[derive(Clone, Debug, Default)]
pub struct Text {
    text: String,
    raw: Option<Box<str>>,
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Text {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), raw: None }
    }

    /// Character data as it appears in a file: decoded, keeping the source when it is not the
    /// canonical escaping.
    fn from_source(raw: &str) -> Result<Self, String> {
        let text = decode(raw, false)?;
        let mut canonical = String::new();
        escape_text(&mut canonical, &text);
        let raw = (canonical != raw).then(|| raw.into());
        Ok(Self { text, raw })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The source text, when it differs from the canonical escaping.
    pub fn raw(&self) -> Option<&str> {
        self.raw.as_deref()
    }

    pub fn is_whitespace(&self) -> bool {
        self.text.chars().all(is_xml_space)
    }
}

/// An attribute: its name and decoded value.
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

/// An element. Its start and end tags keep their source text when it is not canonical (other
/// quotes, extra spaces, a different order of nothing); changing the attributes drops it.
#[derive(Clone, Debug)]
pub struct Element {
    name: String,
    attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// Written `<name/>` when it has no children (as opposed to `<name></name>`).
    self_closing: bool,
    start_raw: Option<Box<str>>,
    end_raw: Option<Box<str>>,
}

impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.attrs == other.attrs && self.children == other.children
    }
}

impl Element {
    /// A new element; with no children it is written `<name/>`.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), attrs: Vec::new(), children: Vec::new(), self_closing: true, start_raw: None, end_raw: None }
    }

    /// Adds an attribute (builder style).
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_attr(name, value);
        self
    }

    /// Adds a child element (builder style).
    pub fn with_child(mut self, child: Element) -> Self {
        self.children.push(Node::Element(child));
        self
    }

    /// Sets the text content (builder style), replacing every child.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.children = vec![Node::Text(Text::new(text))];
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn attrs(&self) -> &[Attr] {
        &self.attrs
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.name == name).map(|a| a.value.as_str())
    }

    /// Sets an attribute in place, or appends it.
    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let (name, value) = (name.into(), value.into());
        match self.attrs.iter_mut().find(|a| a.name == name) {
            Some(a) if a.value == value => return,
            Some(a) => a.value = value,
            None => self.attrs.push(Attr { name, value }),
        }
        self.start_raw = None;
    }

    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        let i = self.attrs.iter().position(|a| a.name == name)?;
        self.start_raw = None;
        Some(self.attrs.remove(i).value)
    }

    /// The source text of the start tag, when it was not the canonical rendering.
    pub fn start_source(&self) -> Option<&str> {
        self.start_raw.as_deref()
    }

    /// Makes the element write `source` as its start tag. The caller vouches that `source` is a
    /// start tag of this element with the same meaning (see [`same_attributes`]); it applies only
    /// while the attributes stay as they are.
    pub fn set_start_source(&mut self, source: &str) {
        self.start_raw = Some(source.into());
    }

    /// Whether an element without children is written `<name/>` rather than `<name></name>`.
    pub fn self_closing(&self) -> bool {
        self.self_closing
    }

    pub fn set_self_closing(&mut self, yes: bool) {
        if self.self_closing != yes {
            self.self_closing = yes;
            self.start_raw = None;
            self.end_raw = None;
        }
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// The first child element called `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// The concatenated text and CDATA of the direct children.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) => out.push_str(&t.text),
                Node::CData(c) => out.push_str(c),
                _ => {}
            }
        }
        out
    }

    /// Whether the children include text that is not whitespace (or a CDATA section).
    pub fn has_text(&self) -> bool {
        self.children.iter().any(|n| matches!(n, Node::CData(_)) || matches!(n, Node::Text(t) if !t.is_whitespace()))
    }

    /// Removes the whitespace a pretty printer put between child elements, here and below, so
    /// [`Element::write_pretty`] can indent the element at another depth. Elements whose content
    /// includes text are left alone.
    pub fn strip_layout(&mut self) {
        if !self.has_text() && self.children.iter().any(|n| matches!(n, Node::Element(_) | Node::Comment(_) | Node::Pi(_))) {
            self.children.retain(|n| !matches!(n, Node::Text(_)));
        }
        for e in self.elements_mut() {
            e.strip_layout();
        }
    }

    /// Writes the element as it was read (or canonically, where it was built or changed).
    pub fn write(&self, out: &mut String) {
        if self.write_start(out) {
            return;
        }
        for n in &self.children {
            write_node(out, n);
        }
        self.write_end(out);
    }

    /// Writes the element the way lxml's pretty printer does: an element whose children are all
    /// elements (and comments) puts each child on its own line, indented by `unit` per level,
    /// and its end tag on a line at its own level; an element with any text child is written as
    /// it is. Whitespace text already among the children is written as it is too, so call this on
    /// a built tree or after [`Element::strip_layout`].
    pub fn write_pretty(&self, out: &mut String, unit: &str, level: usize) {
        if self.write_start(out) {
            return;
        }
        let format = !self.children.is_empty() && !self.children.iter().any(|n| matches!(n, Node::Text(_) | Node::CData(_)));
        for n in &self.children {
            if format {
                newline(out, unit, level + 1);
            }
            match n {
                Node::Element(e) if format => e.write_pretty(out, unit, level + 1),
                Node::Element(e) => e.write(out),
                n => write_node(out, n),
            }
        }
        if format {
            newline(out, unit, level);
        }
        self.write_end(out);
    }

    /// Writes the start tag; `true` when it closed the element (`<name/>`).
    pub(crate) fn write_start(&self, out: &mut String) -> bool {
        // A source tag that closed itself does not fit once children were added.
        if let Some(raw) = &self.start_raw
            && !(raw.ends_with("/>") && !self.children.is_empty())
        {
            out.push_str(raw);
            return raw.ends_with("/>");
        }
        let empty = self.children.is_empty() && self.self_closing;
        out.push_str(&self.canonical_start_tag(empty));
        empty
    }

    /// The start tag as this module writes it canonically, closing the element when `empty`.
    pub fn canonical_start_tag(&self, empty: bool) -> String {
        let mut out = String::new();
        out.push('<');
        out.push_str(&self.name);
        for a in &self.attrs {
            out.push(' ');
            out.push_str(&a.name);
            out.push_str("=\"");
            escape_attr(&mut out, &a.value);
            out.push('"');
        }
        out.push_str(if empty { "/>" } else { ">" });
        out
    }

    /// The start tag as it will be written: the kept source, or the canonical tag.
    pub fn start_tag_text(&self) -> String {
        let mut out = String::new();
        self.write_start(&mut out);
        out
    }

    /// Whether the element is written closed (`<name/>`).
    pub fn writes_empty(&self) -> bool {
        self.start_tag_text().ends_with("/>")
    }

    pub(crate) fn write_end(&self, out: &mut String) {
        match &self.end_raw {
            Some(raw) => out.push_str(raw),
            None => {
                out.push_str("</");
                out.push_str(&self.name);
                out.push('>');
            }
        }
    }

    /// The canonical start tag, for telling whether a source tag needs to be kept.
    fn canonical_start(&self) -> String {
        self.canonical_start_tag(self.children.is_empty() && self.self_closing)
    }
}

fn newline(out: &mut String, unit: &str, level: usize) {
    out.push('\n');
    for _ in 0..level {
        out.push_str(unit);
    }
}

pub(crate) fn write_node(out: &mut String, n: &Node) {
    match n {
        Node::Element(e) => e.write(out),
        Node::Text(t) => match &t.raw {
            Some(raw) => out.push_str(raw),
            None => escape_text(out, &t.text),
        },
        Node::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        Node::CData(c) => {
            out.push_str("<![CDATA[");
            out.push_str(c);
            out.push_str("]]>");
        }
        Node::Pi(p) => {
            out.push_str("<?");
            out.push_str(p);
            out.push_str("?>");
        }
        Node::Doctype(d) => {
            out.push_str("<!DOCTYPE");
            out.push_str(d);
            out.push('>');
        }
    }
}

/// Escapes character data as libxml2 does.
pub fn escape_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// Escapes an attribute value as libxml2 does.
pub fn escape_attr(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
}

impl Document {
    /// A document with an XML declaration spelled as `declaration` (the text between `<?` and
    /// `?>`), a line break, `root`, and a final line break.
    pub fn new(declaration: &str, root: Element) -> Self {
        Self {
            bom: false,
            prolog: vec![Node::Pi(declaration.to_string()), Node::Text(Text::new("\n"))],
            root,
            epilog: vec![Node::Text(Text::new("\n"))],
        }
    }

    /// Writes the document. For a document [`parse`] returned unchanged, the result is the input.
    pub fn write(&self) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        for n in &self.prolog {
            write_node(&mut out, n);
        }
        self.root.write(&mut out);
        for n in &self.epilog {
            write_node(&mut out, n);
        }
        out
    }

    /// Writes the document with the root element pretty-printed (see [`Element::write_pretty`]).
    pub fn write_pretty(&self, unit: &str) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        for n in &self.prolog {
            write_node(&mut out, n);
        }
        self.root.write_pretty(&mut out, unit, 0);
        for n in &self.epilog {
            write_node(&mut out, n);
        }
        out
    }

    /// The text of the XML declaration (between `<?` and `?>`), if the document has one.
    pub fn declaration(&self) -> Option<&str> {
        match self.prolog.first() {
            Some(Node::Pi(p)) if p.starts_with("xml") && p.get(3..4).is_none_or(|c| c.chars().all(is_xml_space)) => Some(p),
            _ => None,
        }
    }
}

pub(crate) fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == ':' || (!c.is_ascii() && !c.is_whitespace())
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

/// Parses a whole document.
pub fn parse(text: &str) -> Result<Document, Error> {
    if text.len() > MAX_INPUT {
        return Err(Error { line: 1, column: 1, message: format!("input is larger than {MAX_INPUT} bytes") });
    }
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut p = Parser { text: body, pos: 0 };
    let mut prolog = Vec::new();
    let root = loop {
        match p.misc()? {
            Some(n) => prolog.push(n),
            None => {
                if p.rest().starts_with('<') {
                    break p.element(0)?;
                }
                return Err(p.err(if p.rest().is_empty() { "no root element" } else { "text before the root element" }));
            }
        }
    };
    let mut epilog = Vec::new();
    while let Some(n) = p.misc()? {
        if matches!(n, Node::Doctype(_)) {
            return Err(p.err("a document type after the root element"));
        }
        epilog.push(n);
    }
    if !p.rest().is_empty() {
        return Err(p.err("unexpected content after the root element"));
    }
    Ok(Document { bom, prolog, root, epilog })
}

/// Parses a start tag on its own (`<a b="1">` or `<a b="1"/>`), as kept by
/// [`Element::start_source`]. The element has no children.
pub fn parse_start_tag(text: &str) -> Result<Element, Error> {
    let mut p = Parser { text, pos: 0 };
    let name_end = {
        p.expect("<")?;
        p.name()?;
        p.pos
    };
    let name = text.get(1..name_end).unwrap_or("");
    let closed = if text.trim_end().ends_with("/>") { text.to_string() } else { format!("{text}</{name}>") };
    let mut el = parse_element(&closed)?;
    el.children.clear();
    el.end_raw = None;
    Ok(el)
}

/// Whether two elements have the same name and the same attributes in any order, where values
/// that both read as numbers count as the same when the numbers are equal (`100` and `100.0`),
/// and `hex` values when they are the same hexadecimal number (`00e9` and `00E9`).
pub fn same_attributes(a: &Element, b: &Element) -> bool {
    if a.name != b.name || a.attrs.len() != b.attrs.len() {
        return false;
    }
    a.attrs.iter().all(|x| {
        b.attr(&x.name).is_some_and(|v| {
            v == x.value
                || matches!((v.trim().parse::<f64>(), x.value.trim().parse::<f64>()), (Ok(p), Ok(q)) if p == q)
                || x.name == "hex" && matches!((u32::from_str_radix(v, 16), u32::from_str_radix(&x.value, 16)), (Ok(p), Ok(q)) if p == q)
        })
    })
}

/// Parses a fragment that is one element, with nothing around it but whitespace. For elements
/// kept as text inside other data.
pub fn parse_element(text: &str) -> Result<Element, Error> {
    let doc = parse(text)?;
    if doc.prolog.iter().chain(&doc.epilog).any(|n| !matches!(n, Node::Text(t) if t.is_whitespace())) {
        return Err(Error { line: 1, column: 1, message: "expected a single element".into() });
    }
    Ok(doc.root)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.pos..).unwrap_or("")
    }

    fn err(&self, message: impl Into<String>) -> Error {
        Error::at(self.text, self.pos, message)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<(), Error> {
        if self.eat(s) { Ok(()) } else { Err(self.err(format!("expected `{s}`"))) }
    }

    /// Everything up to `end`, which is consumed.
    fn until(&mut self, end: &str, what: &str) -> Result<&'a str, Error> {
        let rest = self.rest();
        let Some(i) = rest.find(end) else { return Err(self.err(format!("{what} is not closed"))) };
        let s = rest.get(..i).unwrap_or("");
        self.pos += i + end.len();
        Ok(s)
    }

    fn skip_space(&mut self) -> &'a str {
        let rest = self.rest();
        let n = rest.len() - rest.trim_start_matches(is_xml_space).len();
        self.pos += n;
        rest.get(..n).unwrap_or("")
    }

    fn name(&mut self) -> Result<&'a str, Error> {
        let rest = self.rest();
        let mut chars = rest.char_indices();
        match chars.next() {
            Some((_, c)) if is_name_start(c) => {}
            _ => return Err(self.err("expected a name")),
        }
        let end = chars.find(|(_, c)| !is_name_char(*c)).map_or(rest.len(), |(i, _)| i);
        self.pos += end;
        Ok(rest.get(..end).unwrap_or(""))
    }

    /// A node allowed outside the root element (whitespace, comment, processing instruction,
    /// document type), or `None` at an element or the end.
    fn misc(&mut self) -> Result<Option<Node>, Error> {
        let rest = self.rest();
        if rest.is_empty() {
            return Ok(None);
        }
        if rest.starts_with(is_xml_space) {
            let raw = self.skip_space();
            return Ok(Some(Node::Text(Text::from_source(raw).map_err(|m| self.err(m))?)));
        }
        if self.eat("<!--") {
            return self.comment().map(Some);
        }
        if self.eat("<?") {
            return self.pi().map(Some);
        }
        if self.eat("<!DOCTYPE") {
            return self.doctype().map(Some);
        }
        Ok(None)
    }

    fn comment(&mut self) -> Result<Node, Error> {
        let c = self.until("-->", "a comment")?;
        Ok(Node::Comment(c.to_string()))
    }

    fn pi(&mut self) -> Result<Node, Error> {
        let p = self.until("?>", "a processing instruction")?;
        Ok(Node::Pi(p.to_string()))
    }

    fn doctype(&mut self) -> Result<Node, Error> {
        let start = self.pos;
        let mut in_subset = false;
        let mut quote: Option<char> = None;
        for (i, c) in self.rest().char_indices() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '[') => in_subset = true,
                (None, ']') => in_subset = false,
                (None, '>') if !in_subset => {
                    let d = self.text.get(start..start + i).unwrap_or("");
                    if d.contains("<!ENTITY") {
                        return Err(self.err("entity declarations are not supported"));
                    }
                    self.pos = start + i + 1;
                    return Ok(Node::Doctype(d.to_string()));
                }
                _ => {}
            }
        }
        Err(self.err("the document type is not closed"))
    }

    fn element(&mut self, depth: usize) -> Result<Element, Error> {
        if depth >= MAX_DEPTH {
            return Err(self.err(format!("elements nest deeper than {MAX_DEPTH}")));
        }
        let tag_start = self.pos;
        self.expect("<")?;
        let name = self.name()?.to_string();
        let mut attrs: Vec<Attr> = Vec::new();
        let self_closing = loop {
            let space = self.skip_space();
            if self.eat("/>") {
                break true;
            }
            if self.eat(">") {
                break false;
            }
            if space.is_empty() {
                return Err(self.err("expected a space, `>` or `/>`"));
            }
            let attr_name = self.name()?.to_string();
            self.skip_space();
            self.expect("=")?;
            self.skip_space();
            let quote = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => return Err(self.err("expected a quoted attribute value")),
            };
            self.pos += 1;
            let raw = self.until(if quote == '"' { "\"" } else { "'" }, "an attribute value")?;
            if raw.contains('<') {
                return Err(self.err("`<` in an attribute value"));
            }
            let value = decode(raw, true).map_err(|m| self.err(m))?;
            if attrs.iter().any(|a| a.name == attr_name) {
                return Err(self.err(format!("the attribute `{attr_name}` is repeated")));
            }
            if attrs.len() >= MAX_ATTRIBUTES {
                return Err(self.err(format!("more than {MAX_ATTRIBUTES} attributes")));
            }
            attrs.push(Attr { name: attr_name, value });
        };
        let start_src = self.text.get(tag_start..self.pos).unwrap_or("").to_string();
        let mut el = Element { name, attrs, children: Vec::new(), self_closing, start_raw: None, end_raw: None };
        if !self_closing {
            el.children = self.content(depth)?;
            let end_start = self.pos;
            self.expect("</")?;
            let end_name = self.name()?;
            if end_name != el.name {
                return Err(self.err(format!("`</{end_name}>` closes `<{}>`", el.name)));
            }
            self.skip_space();
            self.expect(">")?;
            let end_src = self.text.get(end_start..self.pos).unwrap_or("");
            if end_src.len() != el.name.len() + 3 {
                el.end_raw = Some(end_src.into());
            }
        }
        if el.canonical_start() != start_src {
            el.start_raw = Some(start_src.into());
        }
        Ok(el)
    }

    fn content(&mut self, depth: usize) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        loop {
            let rest = self.rest();
            if rest.is_empty() {
                return Err(self.err("the document ends inside an element"));
            }
            if rest.starts_with("</") {
                return Ok(out);
            }
            if self.eat("<!--") {
                out.push(self.comment()?);
            } else if self.eat("<![CDATA[") {
                out.push(Node::CData(self.until("]]>", "a CDATA section")?.to_string()));
            } else if self.eat("<?") {
                out.push(self.pi()?);
            } else if rest.starts_with("<!") {
                return Err(self.err("unexpected markup declaration"));
            } else if rest.starts_with('<') {
                out.push(Node::Element(self.element(depth + 1)?));
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                let raw = rest.get(..end).unwrap_or("");
                self.pos += end;
                out.push(Node::Text(Text::from_source(raw).map_err(|m| self.err(m))?));
            }
        }
    }
}

/// Decodes entity and character references, and normalises line ends (and, in attribute values,
/// whitespace characters) as an XML processor must.
fn decode(raw: &str, attribute: bool) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find(['&', '\r', '\n', '\t']) {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        if let Some(after_amp) = after.strip_prefix('&') {
            let Some(semi) = after_amp.find(';') else { return Err("`&` without `;`".into()) };
            let entity = after_amp.get(..semi).unwrap_or("");
            let c = match entity {
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
                        return Err(format!("unknown entity `&{entity};`"));
                    };
                    match code.and_then(char::from_u32) {
                        Some(c) if c != '\0' => c,
                        _ => return Err(format!("`&{entity};` is not a character")),
                    }
                }
            };
            out.push(c);
            rest = after_amp.get(semi + 1..).unwrap_or("");
        } else {
            let (c, len) = match after.as_bytes() {
                [b'\r', b'\n', ..] => ('\n', 2),
                [b'\r', ..] => ('\n', 1),
                [b, ..] => (char::from(*b), 1),
                [] => break,
            };
            out.push(if attribute { ' ' } else { c });
            rest = after.get(len..).unwrap_or("");
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(text: &str) {
        let doc = parse(text).unwrap();
        assert_eq!(doc.write(), text);
    }

    #[test]
    fn documents_write_back_byte_for_byte() {
        same("<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"a\" format=\"2\">\n  <advance width=\"600\"/>\n</glyph>\n");
        same("<?xml version=\"1.0\"?>\r\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"x.dtd\">\r\n<a  b = 'c' ></a >");
        same("\u{feff}<a><!-- note --><![CDATA[x < y]]><?pi data?>t&#233;xt &amp; &#x3c;</a>\n\n");
        same("<a b=\"&#10;x&quot;\" c=\"'\"><b/><c></c>\t</a>");
        same("<a>\r\n line\r</a>");
    }

    #[test]
    fn values_are_decoded() {
        let doc = parse("<a x=\"1&amp;2\" y='a\nb' z=\"&#10;\">&lt;&#65;&#x42;</a>").unwrap();
        assert_eq!(doc.root.attr("x"), Some("1&2"));
        assert_eq!(doc.root.attr("y"), Some("a b"), "a literal line break in an attribute is a space");
        assert_eq!(doc.root.attr("z"), Some("\n"));
        assert_eq!(doc.root.text(), "<AB");
    }

    #[test]
    fn spellings_are_kept_only_where_needed() {
        let doc = parse("<a b=\"1\"><c d='2'/>x&#65;</a>").unwrap();
        assert!(doc.root.start_raw.is_none());
        let c = doc.root.child("c").unwrap();
        assert_eq!(c.start_raw.as_deref(), Some("<c d='2'/>"));
        let Some(Node::Text(t)) = doc.root.children.get(1) else { panic!() };
        assert_eq!(t.raw(), Some("x&#65;"));
    }

    #[test]
    fn a_changed_attribute_drops_the_source_spelling() {
        let mut doc = parse("<a b='1'/>").unwrap();
        doc.root.set_attr("b", "1");
        assert_eq!(doc.write(), "<a b='1'/>", "setting the same value changes nothing");
        doc.root.set_attr("b", "2");
        assert_eq!(doc.write(), "<a b=\"2\"/>");
    }

    #[test]
    fn pretty_printing_follows_lxml() {
        let root = Element::new("glyph")
            .with_attr("name", "a")
            .with_child(Element::new("advance").with_attr("width", "600"))
            .with_child(Element::new("outline").with_child(Element::new("contour").with_child(Element::new("point").with_attr("x", "1"))))
            .with_child(Element::new("note").with_text("\nhi\n"));
        let doc = Document::new("xml version='1.0' encoding='UTF-8'", root);
        assert_eq!(
            doc.write_pretty("  "),
            "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"a\">\n  <advance width=\"600\"/>\n  <outline>\n    <contour>\n      <point x=\"1\"/>\n    </contour>\n  </outline>\n  <note>\nhi\n</note>\n</glyph>\n"
        );
    }

    #[test]
    fn strip_layout_allows_reindenting() {
        let mut el = parse_element("<a>\n\t<b>\n\t\t<c/>\n\t</b>\n\t<d>text</d>\n</a>").unwrap();
        el.strip_layout();
        let mut out = String::new();
        el.write_pretty(&mut out, "  ", 1);
        assert_eq!(out, "<a>\n    <b>\n      <c/>\n    </b>\n    <d>text</d>\n  </a>");
    }

    #[test]
    fn start_tags_parse_and_compare_by_meaning() {
        let a = parse_start_tag("<point y='5' x=\"100.0\" type=\"line\"/>").unwrap();
        let b = Element::new("point").with_attr("x", "100").with_attr("y", "5").with_attr("type", "line");
        assert!(same_attributes(&a, &b));
        assert!(!same_attributes(&a, &b.clone().with_attr("x", "101")));
        let u = parse_start_tag("<unicode hex=\"00e9\">").unwrap();
        assert!(same_attributes(&u, &Element::new("unicode").with_attr("hex", "00E9")));
        assert!(parse_start_tag("<a b=>").is_err());
    }

    #[test]
    fn malformed_input_is_an_error() {
        for bad in [
            "",
            "text",
            "<a>",
            "<a></b>",
            "<a b=1/>",
            "<a b='1' b='2'/>",
            "<a>&nope;</a>",
            "<a>&#0;</a>",
            "<a>&#xFFFFFFFF;</a>",
            "<a/><b/>",
            "<a b='<'/>",
            "<!DOCTYPE a [<!ENTITY e 'x'>]><a/>",
            "<a><!-- open</a>",
            "<a",
            "<1/>",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        let text = "<a>".repeat(MAX_DEPTH + 1) + &"</a>".repeat(MAX_DEPTH + 1);
        assert!(parse(&text).is_err());
    }
}
