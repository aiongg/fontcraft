//! The axes and source locations of a designspace document, normalised as fontTools'
//! `DesignSpaceDocument.normalize` does (MIT): each source's design coordinates normalised over
//! the axis range mapped to design space, axes the source leaves out at 0. The source files are
//! not read.
//!
//! The reader scans start tags only: `<axis>`, its `<map>` children, `<source>` and the
//! `<dimension>` elements of its `<location>`. Comments, CDATA sections and processing
//! instructions are skipped; a document type declaration with an internal subset is refused.

use super::piecewise_linear_map;
use crate::limits::MAX_AXES;
use crate::sfnt::{Tag, parse_tag};
use crate::variations::{MAX_DESIGNSPACE, MAX_MAP_POINTS};
use crate::{Result, TablesError};

/// Most sources read from one document.
pub const MAX_SOURCES: usize = super::MAX_MODEL_MASTERS;
/// Most tags scanned in one document.
const MAX_TAGS: usize = 10_000_000;

/// What [`designspace_locations`] reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignspaceLocations {
    /// Tag and name of each axis, sorted by name (the order fontTools' model sees them in).
    pub axes: Vec<(Tag, String)>,
    /// Each source's name (empty when it has none) and normalised location, one value per axis
    /// of [`Self::axes`], in document order.
    pub sources: Vec<(String, Vec<(Tag, f64)>)>,
}

fn bad(message: impl Into<String>) -> TablesError {
    TablesError::Invalid(format!("the designspace: {}", message.into()))
}

struct Start<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, String)>,
    empty: bool,
}

enum Event<'a> {
    Start(Start<'a>),
    End(&'a str),
}

fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let Some((name, tail)) = after.split_once(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let c = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(name);
                out.push(';');
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn attributes(text: &str) -> Result<Vec<(&str, String)>> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (name, after) = rest.split_once('=').ok_or_else(|| bad("an attribute without a value"))?;
        let after = after.trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'').ok_or_else(|| bad("an attribute value without quotes"))?;
        let (value, tail) = after.get(1..).unwrap_or_default().split_once(quote).ok_or_else(|| bad("an attribute value that is not closed"))?;
        out.push((name.trim(), decode(value)));
        rest = tail.trim_start();
    }
    Ok(out)
}

/// The text up to the `>` that ends a tag (a quoted `>` does not), and the text after it.
fn split_tag(text: &str) -> Option<(&str, &str)> {
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some((text.get(..i)?, text.get(i + 1..)?)),
            _ => {}
        }
    }
    None
}

fn events(text: &str) -> Result<Vec<Event<'_>>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('<') {
        if out.len() >= MAX_TAGS {
            return Err(TablesError::TooLarge { what: "the tags of the designspace", limit: MAX_TAGS });
        }
        let skipped =
            [("!--", "-->"), ("![CDATA[", "]]>"), ("?", "?>")].iter().find_map(|(open, close)| after.strip_prefix(open).map(|b| (b, *close)));
        if let Some((body, close)) = skipped {
            rest = body.split_once(close).ok_or_else(|| bad("a comment, CDATA section or processing instruction that is not closed"))?.1;
            continue;
        }
        let (tag, tail) = split_tag(after).ok_or_else(|| bad("a tag without `>`"))?;
        rest = tail;
        if let Some(decl) = tag.strip_prefix('!') {
            if decl.contains('[') {
                return Err(TablesError::Unsupported("a designspace with an internal document type subset".into()));
            }
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            out.push(Event::End(name.trim()));
            continue;
        }
        let (tag, empty) = match tag.strip_suffix('/') {
            Some(t) => (t, true),
            None => (tag, false),
        };
        let (name, attrs) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        out.push(Event::Start(Start { name, attrs: attributes(attrs)?, empty }));
    }
    Ok(out)
}

fn attr<'a>(s: &'a Start<'_>, name: &str) -> Option<&'a str> {
    s.attrs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
}

fn number(s: &Start<'_>, name: &str) -> Result<Option<f64>> {
    match attr(s, name) {
        None => Ok(None),
        Some(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(Some)
            .ok_or_else(|| bad(format!("`{name}=\"{text}\"` of a `<{}>` is not a number", s.name))),
    }
}

struct Axis {
    tag: Tag,
    name: String,
    minimum: f64,
    default: f64,
    maximum: f64,
    map: Vec<(f64, f64)>,
}

impl Axis {
    fn map_forward(&self, v: f64) -> f64 {
        if self.map.is_empty() { v } else { piecewise_linear_map(v, &self.map) }
    }

    /// `v` (design) normalised over the axis range in design space, clamped to it.
    fn normalize(&self, v: f64) -> Result<f64> {
        let (lo, de, hi) = (self.map_forward(self.minimum), self.map_forward(self.default), self.map_forward(self.maximum));
        fontcraft_geom::variation::normalize_value(v, lo, de, hi, false)
            .ok_or_else(|| bad(format!("the axis `{}` has a minimum, default and maximum out of order", self.name)))
    }
}

/// The tag of an axis without a `tag` attribute: the registered tag of its name.
fn registered_tag(name: &str) -> Option<Tag> {
    Some(match name {
        "weight" => *b"wght",
        "width" => *b"wdth",
        "optical" => *b"opsz",
        "slant" => *b"slnt",
        "italic" => *b"ital",
        _ => return None,
    })
}

fn read_axis(s: &Start<'_>) -> Result<Axis> {
    let name = attr(s, "name").ok_or_else(|| bad("an `<axis>` without a name"))?.to_string();
    let tag = match attr(s, "tag") {
        Some(t) => parse_tag(t)?,
        None => registered_tag(&name).ok_or_else(|| bad(format!("the axis `{name}` has no tag")))?,
    };
    // A discrete axis lists its values instead of a range.
    let values: Vec<f64> =
        attr(s, "values").map(|v| v.split_whitespace().filter_map(|x| x.parse::<f64>().ok()).filter(|x| x.is_finite()).collect()).unwrap_or_default();
    let lowest = values.iter().copied().reduce(f64::min);
    let highest = values.iter().copied().reduce(f64::max);
    let minimum = number(s, "minimum")?.or(lowest).ok_or_else(|| bad(format!("the axis `{name}` has no minimum")))?;
    let maximum = number(s, "maximum")?.or(highest).ok_or_else(|| bad(format!("the axis `{name}` has no maximum")))?;
    let default = number(s, "default")?.ok_or_else(|| bad(format!("the axis `{name}` has no default")))?;
    Ok(Axis { tag, name, minimum, default, maximum, map: Vec::new() })
}

/// The axes and normalised source locations of the designspace `text`.
pub fn designspace_locations(text: &str) -> Result<DesignspaceLocations> {
    if text.len() > MAX_DESIGNSPACE {
        return Err(TablesError::TooLarge { what: "the designspace document", limit: MAX_DESIGNSPACE });
    }
    let mut axes: Vec<Axis> = Vec::new();
    // Each source's name and its dimensions: axis name, design value, user value.
    type Dimension = (String, Option<f64>, Option<f64>);
    let mut sources: Vec<(String, Vec<Dimension>)> = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    for event in events(text)? {
        match event {
            Event::End(name) => {
                if let Some(at) = stack.iter().rposition(|n| *n == name) {
                    stack.truncate(at);
                }
            }
            Event::Start(s) => {
                let parent = stack.last().copied();
                let grandparent = stack.len().checked_sub(2).and_then(|i| stack.get(i)).copied();
                match (s.name, parent) {
                    ("axis", Some("axes")) => {
                        if axes.len() >= MAX_AXES {
                            return Err(TablesError::TooLarge { what: "the axes of the designspace", limit: MAX_AXES });
                        }
                        axes.push(read_axis(&s)?);
                    }
                    ("map", Some("axis")) => {
                        let axis = axes.last_mut().ok_or_else(|| bad("a `<map>` outside an axis"))?;
                        if axis.map.len() >= MAX_MAP_POINTS {
                            return Err(TablesError::TooLarge { what: "the points of one axis map", limit: MAX_MAP_POINTS });
                        }
                        let input = number(&s, "input")?.ok_or_else(|| bad("a `<map>` without `input`"))?;
                        let output = number(&s, "output")?.ok_or_else(|| bad("a `<map>` without `output`"))?;
                        match axis.map.iter_mut().find(|(i, _)| *i == input) {
                            Some(slot) => slot.1 = output,
                            None => axis.map.push((input, output)),
                        }
                    }
                    ("source", Some("sources")) => {
                        if sources.len() >= MAX_SOURCES {
                            return Err(TablesError::TooLarge { what: "the sources of the designspace", limit: MAX_SOURCES });
                        }
                        sources.push((attr(&s, "name").unwrap_or_default().to_string(), Vec::new()));
                    }
                    ("dimension", Some("location")) if grandparent == Some("source") => {
                        let source = sources.last_mut().ok_or_else(|| bad("a `<dimension>` outside a source"))?;
                        if source.1.len() >= MAX_AXES {
                            return Err(TablesError::TooLarge { what: "the dimensions of one source", limit: MAX_AXES });
                        }
                        let name = attr(&s, "name").ok_or_else(|| bad("a `<dimension>` without a name"))?.to_string();
                        source.1.push((name, number(&s, "xvalue")?, number(&s, "uservalue")?));
                    }
                    _ => {}
                }
                if !s.empty {
                    if stack.len() >= 64 {
                        return Err(bad("elements nest deeper than 64 levels"));
                    }
                    stack.push(s.name);
                }
            }
        }
    }
    if sources.is_empty() {
        return Err(bad("it has no sources"));
    }
    axes.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = DesignspaceLocations { axes: axes.iter().map(|a| (a.tag, a.name.clone())).collect(), sources: Vec::with_capacity(sources.len()) };
    for (name, dims) in sources {
        let mut location = Vec::with_capacity(axes.len());
        for axis in &axes {
            let value = match dims.iter().find(|(n, _, _)| *n == axis.name) {
                Some((_, Some(design), _)) => axis.normalize(*design)?,
                Some((_, None, Some(user))) => axis.normalize(axis.map_forward(*user))?,
                _ => 0.0,
            };
            location.push((axis.tag, value));
        }
        out.sources.push((name, location));
    }
    Ok(out)
}
