//! The first pass over an SVG document: checks it against the limits and writes the subset that
//! carries geometry as a new document for usvg. Everything else is left out and counted in
//! [`Dropped`].
//!
//! The rewrite is what keeps usvg's work bounded and side-effect free: without `image`, `style`,
//! `filter`, `pattern`, `marker` or `clipPath` elements and without presentation attributes, usvg
//! has nothing to load, decode or multiply except the shapes we counted.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use usvg::roxmltree::{self, Document, Node, NodeId, ParsingOptions};

use crate::{Dropped, MAX_SVG_DEPTH, MAX_SVG_ELEMENTS, MAX_SVG_EXPANDED_ELEMENTS, MAX_SVG_NUMBER, MAX_SVG_NUMBERS, SvgError};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
/// XML nodes of every kind (elements, text, comments) the parser may build.
const MAX_XML_NODES: u32 = 400_000;

/// Elements that carry geometry or structure. Everything else is left out with its content.
const GEOMETRY_ELEMENTS: &[&str] =
    &["svg", "g", "defs", "symbol", "use", "a", "switch", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon"];
/// Elements left out without being reported: they never draw anything.
const SILENT_ELEMENTS: &[&str] = &["title", "desc", "metadata"];
/// Attributes kept on the elements above: geometry, placement, references and the conditions
/// `switch` reads. `display` is kept so that hidden parts stay hidden.
const GEOMETRY_ATTRIBUTES: &[&str] = &[
    "id",
    "transform",
    "x",
    "y",
    "width",
    "height",
    "viewBox",
    "preserveAspectRatio",
    "rx",
    "ry",
    "cx",
    "cy",
    "r",
    "x1",
    "y1",
    "x2",
    "y2",
    "points",
    "d",
    "href",
    "display",
    "systemLanguage",
    "requiredFeatures",
    "requiredExtensions",
];
/// The kept attributes whose numbers are counted and range-checked.
const NUMERIC_ATTRIBUTES: &[&str] =
    &["transform", "x", "y", "width", "height", "viewBox", "rx", "ry", "cx", "cy", "r", "x1", "y1", "x2", "y2", "points", "d"];
/// Attributes left out without being reported: they describe the file, not the drawing.
const SILENT_ATTRIBUTES: &[&str] = &["version", "baseProfile"];

pub(crate) struct Sanitized {
    /// The geometry-only document. Its root has no `viewBox`, so usvg's coordinates are the
    /// source's user units.
    pub text: String,
    /// The root element's `viewBox` as `[x, y, width, height]`, when it has a usable one.
    pub view_box: Option<[f64; 4]>,
    pub dropped: Dropped,
}

pub(crate) fn sanitize(text: &str) -> Result<Sanitized, SvgError> {
    // The XML parser recurses once per open element, so nesting is checked before it runs.
    let nesting = markup_nesting(text.as_bytes());
    if nesting.body > MAX_SVG_DEPTH || nesting.literals > MAX_SVG_DEPTH {
        return Err(SvgError::TooDeep);
    }
    let options = ParsingOptions { allow_dtd: true, nodes_limit: MAX_XML_NODES, ..ParsingOptions::default() };
    let doc = Document::parse_with_options(text, options).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => SvgError::TooManyElements,
        e => SvgError::Parse(e.to_string()),
    })?;
    let root = doc.root_element();
    if !(is_svg_element(root) && root.tag_name().name() == "svg") {
        return Err(SvgError::NoSvgRoot);
    }
    let survey = survey(&doc)?;
    let mut ctx = Measure { kept: &survey.kept, ids: &survey.ids, memo: HashMap::new() };
    measure(root, 0, &mut ctx)?;

    let view_box = root.attribute("viewBox").and_then(parse_view_box);
    let mut out = String::with_capacity(text.len().min(1 << 20));
    write_root(root, view_box, &survey.kept, &mut out);
    Ok(Sanitized { text: out, view_box, dropped: survey.dropped })
}

/// The deepest element nesting in the markup (`body`) and inside any quoted literal of the
/// document type declaration (`literals`), where entity values live. The parser expands an
/// entity by parsing its value as content, at most ten entities deep, so the two bounds together
/// bound its recursion.
#[derive(Debug, Default, PartialEq)]
struct Nesting {
    body: usize,
    literals: usize,
}

/// A linear scan that follows the XML markup closely enough to count open elements: comments,
/// CDATA sections, processing instructions and declarations are skipped, and quotes are honoured
/// inside tags. An attribute value cannot contain `<`, so every `<` outside those constructs is
/// a tag. Malformed markup may be miscounted; the parser rejects it at that point.
fn markup_nesting(b: &[u8]) -> Nesting {
    let mut out = Nesting::default();
    out.body = scan_nesting(b, &mut out.literals, false);
    out
}

fn scan_nesting(b: &[u8], literals: &mut usize, in_literal: bool) -> usize {
    let at = |i: usize, s: &[u8]| b.get(i..).is_some_and(|t| t.starts_with(s));
    let find = |from: usize, s: &[u8]| b.get(from..).and_then(|t| t.windows(s.len()).position(|w| w == s)).map_or(b.len(), |p| from + p + s.len());
    let (mut i, mut depth, mut max) = (0usize, 0usize, 0usize);
    while let Some(p) = b.get(i..).and_then(|t| t.iter().position(|&c| c == b'<')) {
        i += p;
        if at(i, b"<!--") {
            i = find(i + 4, b"-->");
        } else if at(i, b"<![CDATA[") {
            i = find(i + 9, b"]]>");
        } else if at(i, b"<?") {
            i = find(i + 2, b"?>");
        } else if at(i, b"<!") {
            // A declaration, with an internal subset in brackets. Its quoted literals are
            // scanned as markup, once: a literal's own literals are not entity values.
            i += 2;
            let mut brackets = 0usize;
            while let Some(&c) = b.get(i) {
                if at(i, b"<!--") {
                    i = find(i + 4, b"-->");
                    continue;
                }
                if at(i, b"<?") {
                    i = find(i + 2, b"?>");
                    continue;
                }
                i += 1;
                match c {
                    b'"' | b'\'' => {
                        let end = b.get(i..).and_then(|t| t.iter().position(|&q| q == c)).map_or(b.len(), |q| i + q);
                        if !in_literal {
                            let inner = scan_nesting(b.get(i..end).unwrap_or_default(), literals, true);
                            *literals = (*literals).max(inner);
                        }
                        i = end.saturating_add(1);
                    }
                    b'[' => brackets += 1,
                    b']' => brackets = brackets.saturating_sub(1),
                    b'>' if brackets == 0 => break,
                    _ => {}
                }
            }
        } else if at(i, b"</") {
            depth = depth.saturating_sub(1);
            i = find(i + 2, b">");
        } else {
            i += 1;
            let mut quote = None;
            let mut empty = false;
            while let Some(&c) = b.get(i) {
                i += 1;
                match (quote, c) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, b'"' | b'\'') => quote = Some(c),
                    (None, b'>') => break,
                    (None, b'/') if b.get(i) == Some(&b'>') => {
                        empty = true;
                        i += 1;
                        break;
                    }
                    (None, b'<') => {
                        i -= 1;
                        break;
                    }
                    _ => {}
                }
            }
            if !empty {
                depth += 1;
                max = max.max(depth);
            }
        }
    }
    max
}

fn is_svg_element(node: Node) -> bool {
    node.is_element() && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
}

struct Survey<'a, 'input> {
    kept: HashSet<NodeId>,
    /// Kept elements by `id`; a list, because a file may repeat an id.
    ids: HashMap<&'a str, Vec<Node<'a, 'input>>>,
    dropped: Dropped,
}

/// One pass over every element, without recursion: counts and depth against the limits, which
/// elements are kept, their ids, and what is left out.
fn survey<'a, 'input>(doc: &'a Document<'input>) -> Result<Survey<'a, 'input>, SvgError> {
    let mut s = Survey { kept: HashSet::new(), ids: HashMap::new(), dropped: Dropped::default() };
    let mut elements = 0usize;
    let mut stack: Vec<(Node<'a, 'input>, usize, bool)> = vec![(doc.root_element(), 1, true)];
    while let Some((node, depth, parent_kept)) = stack.pop() {
        elements += 1;
        if elements > MAX_SVG_ELEMENTS {
            return Err(SvgError::TooManyElements);
        }
        if depth > MAX_SVG_DEPTH {
            return Err(SvgError::TooDeep);
        }
        let name = node.tag_name().name();
        let keep = parent_kept && is_svg_element(node) && GEOMETRY_ELEMENTS.contains(&name);
        if keep {
            s.kept.insert(node.id());
            if let Some(id) = node.attribute("id") {
                s.ids.entry(id).or_default().push(node);
            }
            for a in node.attributes() {
                let n = a.name();
                let silent = a.namespace().is_some() || SILENT_ATTRIBUTES.contains(&n) || n.starts_with("data-");
                if !(silent || GEOMETRY_ATTRIBUTES.contains(&n)) {
                    *s.dropped.attributes.entry(n.to_string()).or_insert(0) += 1;
                }
            }
        } else if parent_kept && is_svg_element(node) && !SILENT_ELEMENTS.contains(&name) {
            // Reported once, at the top of the part that is left out.
            *s.dropped.elements.entry(name.to_string()).or_insert(0) += 1;
        }
        for child in node.children().filter(Node::is_element) {
            stack.push((child, depth + 1, keep));
        }
    }
    Ok(s)
}

/// What one element amounts to once its `use` references are expanded.
#[derive(Clone, Copy, Default)]
struct Weight {
    elements: usize,
    numbers: usize,
    height: usize,
}

impl Weight {
    fn absorb(&mut self, below: Weight) -> Result<(), SvgError> {
        self.elements = self.elements.saturating_add(below.elements);
        self.numbers = self.numbers.saturating_add(below.numbers);
        self.height = self.height.max(below.height);
        if self.elements > MAX_SVG_EXPANDED_ELEMENTS {
            return Err(SvgError::TooManyExpanded);
        }
        if self.numbers > MAX_SVG_NUMBERS {
            return Err(SvgError::TooMuchData);
        }
        Ok(())
    }

    fn larger(self, other: Weight) -> Weight {
        Weight { elements: self.elements.max(other.elements), numbers: self.numbers.max(other.numbers), height: self.height.max(other.height) }
    }
}

struct Measure<'s, 'a, 'input> {
    kept: &'s HashSet<NodeId>,
    ids: &'s HashMap<&'a str, Vec<Node<'a, 'input>>>,
    memo: HashMap<NodeId, Weight>,
}

/// The size of `node` with every `use` reference expanded, as usvg will expand it: elements,
/// numbers of geometry, and height. Each element is measured once (`memo`), so a document that
/// doubles itself through `use` costs linear work here. `level` bounds the recursion; a `use`
/// that refers to its own ancestor runs into it.
fn measure<'a, 'input>(node: Node<'a, 'input>, level: usize, ctx: &mut Measure<'_, 'a, 'input>) -> Result<Weight, SvgError> {
    if level > MAX_SVG_DEPTH {
        return Err(SvgError::TooDeep);
    }
    if let Some(w) = ctx.memo.get(&node.id()) {
        return Ok(*w);
    }
    let mut w = Weight { elements: 1, numbers: own_numbers(node)?, height: 0 };
    if node.tag_name().name() == "use" {
        let targets: Vec<Node<'a, 'input>> = href(node).and_then(|id| ctx.ids.get(id)).cloned().unwrap_or_default();
        let mut largest = Weight::default();
        for target in targets {
            largest = largest.larger(measure(target, level + 1, ctx)?);
        }
        w.absorb(largest)?;
    } else {
        for child in node.children() {
            if ctx.kept.contains(&child.id()) {
                let below = measure(child, level + 1, ctx)?;
                w.absorb(below)?;
            }
        }
    }
    w.height += 1;
    if w.height > MAX_SVG_DEPTH {
        return Err(SvgError::TooDeep);
    }
    ctx.memo.insert(node.id(), w);
    Ok(w)
}

/// The id a `use` element refers to (`#id` in `href` or `xlink:href`).
fn href<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute((XLINK_NS, "href")).or_else(|| node.attribute("href")).map(str::trim).and_then(|s| s.strip_prefix('#'))
}

/// How many numbers the element's geometry attributes hold; an error for one out of range.
fn own_numbers(node: Node) -> Result<usize, SvgError> {
    let mut count = 0usize;
    let mut bad = false;
    for a in node.attributes() {
        if a.namespace().is_some() || !NUMERIC_ATTRIBUTES.contains(&a.name()) {
            continue;
        }
        for_each_number(a.value(), |v| {
            count += 1;
            bad = !(v.is_finite() && v.abs() <= MAX_SVG_NUMBER);
            !bad
        });
        if bad {
            return Err(SvgError::NumberOutOfRange);
        }
    }
    Ok(count)
}

/// Calls `f` on each number in `s` (SVG number syntax: sign, digits, point, exponent), in order,
/// until `f` returns false. A lone sign or point is skipped. Units and letters between numbers
/// are ignored, so this reads path data, point lists, transforms and lengths alike.
pub(crate) fn for_each_number(s: &str, mut f: impl FnMut(f64) -> bool) {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        let starts = c.is_ascii_digit() || c == b'.' || ((c == b'-' || c == b'+') && (digit(i + 1) || b.get(i + 1) == Some(&b'.')));
        if !starts {
            i += 1;
            continue;
        }
        let start = i;
        if c == b'-' || c == b'+' {
            i += 1;
        }
        while digit(i) {
            i += 1;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            while digit(i) {
                i += 1;
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(b.get(j), Some(b'-' | b'+')) {
                j += 1;
            }
            if digit(j) {
                i = j;
                while digit(i) {
                    i += 1;
                }
            }
        }
        if i == start {
            i += 1;
            continue;
        }
        if let Some(Ok(v)) = s.get(start..i).map(str::parse::<f64>)
            && !f(v)
        {
            return;
        }
    }
}

/// `x y width height`, separated by spaces or commas, with a positive size.
fn parse_view_box(s: &str) -> Option<[f64; 4]> {
    let mut v = Vec::with_capacity(4);
    for t in s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
        if v.len() == 4 {
            return None;
        }
        v.push(t.parse::<f64>().ok()?);
    }
    let [x, y, w, h] = v.as_slice() else { return None };
    let all_finite = [x, y, w, h].iter().all(|n| n.is_finite());
    (all_finite && *w > 0.0 && *h > 0.0).then_some([*x, *y, *w, *h])
}

/// Does this length start with a positive number?
fn positive_length(s: &str) -> bool {
    let mut first = None;
    for_each_number(s, |v| {
        first = Some(v);
        false
    });
    first.is_some_and(|v| v > 0.0)
}

/// The root `<svg>`: no `viewBox`, so that usvg places nothing and its coordinates are the
/// source's user units; with a `viewBox`, its size as width and height, so that percentages
/// resolve as in the source.
fn write_root(root: Node, view_box: Option<[f64; 4]>, kept: &HashSet<NodeId>, out: &mut String) {
    out.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink""#);
    match view_box {
        Some([_, _, w, h]) => {
            let _ = write!(out, r#" width="{w}" height="{h}""#);
        }
        None => {
            for name in ["width", "height"] {
                if let Some(v) = root.attribute(name).filter(|v| positive_length(v)) {
                    write_attribute(out, name, v);
                }
            }
        }
    }
    const PLACEMENT: &[&str] = &["viewBox", "preserveAspectRatio", "width", "height", "x", "y"];
    for a in root.attributes() {
        if a.namespace().is_none() && GEOMETRY_ATTRIBUTES.contains(&a.name()) && !PLACEMENT.contains(&a.name()) {
            write_attribute(out, a.name(), a.value());
        }
    }
    out.push('>');
    write_children(root, kept, out);
    out.push_str("</svg>");
}

/// The kept children of `node`. Recursion depth is bounded by [`MAX_SVG_DEPTH`], which the
/// survey checked for every element.
fn write_children(node: Node, kept: &HashSet<NodeId>, out: &mut String) {
    for child in node.children() {
        if kept.contains(&child.id()) {
            write_element(child, kept, out);
        }
    }
}

fn write_element(node: Node, kept: &HashSet<NodeId>, out: &mut String) {
    let name = node.tag_name().name();
    out.push('<');
    out.push_str(name);
    // One link, the one `href` reads (and so the one `measure` counted).
    if let Some(link) = node.attribute((XLINK_NS, "href")).or_else(|| node.attribute("href")) {
        write_attribute(out, "xlink:href", link);
    }
    for a in node.attributes() {
        if a.namespace().is_none() && a.name() != "href" && GEOMETRY_ATTRIBUTES.contains(&a.name()) {
            write_attribute(out, a.name(), a.value());
        }
    }
    if node.children().any(|c| kept.contains(&c.id())) {
        out.push('>');
        write_children(node, kept, out);
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    } else {
        out.push_str("/>");
    }
}

fn write_attribute(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out.push('"');
}
