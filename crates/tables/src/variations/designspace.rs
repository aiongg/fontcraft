//! The axes of a designspace document as [`AvarOptions`]: each continuous axis's `<map>`
//! elements (user to design coordinates) and the cross-axis `<mappings>` of designspace 5, as
//! fontTools' `designspaceLib` (MIT) lays them out. Sources, instances and rules are not read,
//! so the document needs no source files.
//!
//! The reader is a small tag scanner, not a full XML parser: it follows elements and attributes,
//! decodes the predefined and numeric character references, and skips comments, processing
//! instructions, CDATA and the document type declaration.

use super::{AvarOptions, CrossAxisMapping, MAX_CROSS_AXIS_MAPPINGS, MAX_DESIGNSPACE, MAX_MAP_POINTS};
use crate::limits::MAX_AXES;
use crate::sfnt::{Tag, parse_tag};
use crate::{Result, TablesError};

/// Deepest element nesting followed.
const MAX_DEPTH: usize = 64;
/// Most tags scanned in one document.
const MAX_TAGS: usize = 10_000_000;

fn bad(message: impl Into<String>) -> TablesError {
    TablesError::Invalid(format!("the designspace: {}", message.into()))
}

enum Event<'a> {
    Start { name: &'a str, attrs: Vec<(&'a str, String)>, empty: bool },
    End(&'a str),
}

/// The text up to the `>` that ends a tag, quoted `>` skipped, and the text after it.
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

/// `text` with its character references decoded.
fn decode(text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let (name, tail) = after.split_once(';').ok_or_else(|| bad("a character reference without `;`"))?;
        let c = match name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    name.strip_prefix('#').and_then(|d| d.parse::<u32>().ok())
                };
                code.and_then(char::from_u32).ok_or_else(|| bad(format!("the character reference `&{name};` is not supported")))?
            }
        };
        out.push(c);
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

fn attributes(text: &str) -> Result<Vec<(&str, String)>> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (name, after) =
            rest.split_once('=').ok_or_else(|| bad(format!("an attribute without a value near `{}`", rest.chars().take(40).collect::<String>())))?;
        let name = name.trim();
        let after = after.trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'').ok_or_else(|| bad(format!("the attribute `{name}` is not quoted")))?;
        let body = after.get(1..).unwrap_or_default();
        let (value, tail) = body.split_once(quote).ok_or_else(|| bad(format!("the attribute `{name}` is not closed")))?;
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(bad(format!("a malformed attribute name `{name}`")));
        }
        out.push((name, decode(value)?));
        rest = tail.trim_start();
    }
    Ok(out)
}

fn events(text: &str) -> Result<Vec<Event<'_>>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('<') {
        if out.len() >= MAX_TAGS {
            return Err(TablesError::TooLarge { what: "the tags of the designspace", limit: MAX_TAGS });
        }
        let skip = |body: &'_ str, end: &str, what: &str| -> Result<()> {
            if body.contains(end) { Ok(()) } else { Err(bad(format!("an unterminated {what}"))) }
        };
        if let Some(body) = after.strip_prefix("!--") {
            skip(body, "-->", "comment")?;
            rest = body.split_once("-->").map_or("", |(_, r)| r);
            continue;
        }
        if let Some(body) = after.strip_prefix("![CDATA[") {
            skip(body, "]]>", "CDATA section")?;
            rest = body.split_once("]]>").map_or("", |(_, r)| r);
            continue;
        }
        if let Some(body) = after.strip_prefix('?') {
            skip(body, "?>", "processing instruction")?;
            rest = body.split_once("?>").map_or("", |(_, r)| r);
            continue;
        }
        let (tag, tail) = split_tag(after).ok_or_else(|| bad("a tag without `>`"))?;
        rest = tail;
        if tag.starts_with('!') {
            if tag.contains('[') {
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
        if name.is_empty() {
            return Err(bad("a tag without a name"));
        }
        out.push(Event::Start { name, attrs: attributes(attrs)?, empty });
    }
    Ok(out)
}

fn attr<'a>(attrs: &'a [(&str, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
}

fn number(attrs: &[(&str, String)], name: &str, element: &str) -> Result<f64> {
    let text = attr(attrs, name).ok_or_else(|| bad(format!("a `<{element}>` without `{name}`")))?;
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| bad(format!("`{name}=\"{text}\"` of a `<{element}>` is not a number")))
}

struct Axis {
    tag: Tag,
    name: String,
    map: Vec<(f64, f64)>,
}

#[derive(Default)]
struct Mapping {
    input: Vec<(String, f64)>,
    output: Vec<(String, f64)>,
}

/// The `avar` build options a designspace document gives: each continuous axis's map and the
/// cross-axis mappings, with axes named by tag. Discrete axes are left out: they are not in a
/// variable font's `fvar` table.
pub fn avar_options_from_designspace(text: &str) -> Result<AvarOptions> {
    if text.len() > MAX_DESIGNSPACE {
        return Err(TablesError::TooLarge { what: "the designspace", limit: MAX_DESIGNSPACE });
    }
    let mut stack: Vec<&str> = Vec::new();
    let mut axes: Vec<Axis> = Vec::new();
    // The last `<axis>` read was continuous.
    let mut in_axis = false;
    let mut mappings: Vec<Mapping> = Vec::new();
    let mut root_seen = false;
    for event in events(text)? {
        match event {
            Event::Start { name, attrs, empty } => {
                if !root_seen {
                    if name != "designspace" {
                        return Err(bad("the root element is not `<designspace>`"));
                    }
                    root_seen = true;
                }
                let parent = stack.last().copied();
                let grandparent = stack.len().checked_sub(2).and_then(|i| stack.get(i)).copied();
                match name {
                    "axis" if parent == Some("axes") => {
                        in_axis = attr(&attrs, "values").is_none();
                        if in_axis {
                            if axes.len() >= MAX_AXES {
                                return Err(TablesError::TooLarge { what: "the axes of the designspace", limit: MAX_AXES });
                            }
                            let tag = parse_tag(attr(&attrs, "tag").ok_or_else(|| bad("an `<axis>` without `tag`"))?)?;
                            let name = attr(&attrs, "name").unwrap_or_default().to_string();
                            axes.push(Axis { tag, name, map: Vec::new() });
                        }
                    }
                    "map" if parent == Some("axis") && grandparent == Some("axes") && in_axis => {
                        let point = (number(&attrs, "input", "map")?, number(&attrs, "output", "map")?);
                        let axis = axes.last_mut().ok_or_else(|| bad("a `<map>` outside an axis"))?;
                        if axis.map.len() >= MAX_MAP_POINTS {
                            return Err(TablesError::TooLarge { what: "the points of one axis map", limit: MAX_MAP_POINTS });
                        }
                        axis.map.push(point);
                    }
                    "mapping" if parent == Some("mappings") => {
                        if mappings.len() >= MAX_CROSS_AXIS_MAPPINGS {
                            return Err(TablesError::TooLarge { what: "the cross-axis mapping count", limit: MAX_CROSS_AXIS_MAPPINGS });
                        }
                        mappings.push(Mapping::default());
                    }
                    "dimension" if grandparent == Some("mapping") && matches!(parent, Some("input" | "output")) => {
                        let axis = attr(&attrs, "name").ok_or_else(|| bad("a mapping `<dimension>` without `name`"))?.to_string();
                        let value = number(&attrs, "xvalue", "dimension")?;
                        let mapping = mappings.last_mut().ok_or_else(|| bad("a `<dimension>` outside a mapping"))?;
                        let side = if parent == Some("input") { &mut mapping.input } else { &mut mapping.output };
                        if side.len() >= MAX_AXES {
                            return Err(TablesError::TooLarge { what: "the axes of one mapping", limit: MAX_AXES });
                        }
                        side.push((axis, value));
                    }
                    _ => {}
                }
                if !empty {
                    if stack.len() >= MAX_DEPTH {
                        return Err(TablesError::TooLarge { what: "the element nesting of the designspace", limit: MAX_DEPTH });
                    }
                    stack.push(name);
                }
            }
            Event::End(name) => {
                if stack.pop() != Some(name) {
                    return Err(bad(format!("the end tag `</{name}>` does not match")));
                }
            }
        }
    }
    if !root_seen {
        return Err(bad("there is no `<designspace>` element"));
    }
    if !stack.is_empty() {
        return Err(bad("an element is not closed"));
    }
    let tag_of = |name: &str| -> Result<Tag> {
        axes.iter()
            .find(|a| a.name == name)
            .map(|a| a.tag)
            .ok_or_else(|| bad(format!("a mapping names the axis `{name}`, which the designspace does not define")))
    };
    let mut cross = Vec::with_capacity(mappings.len());
    for m in &mappings {
        let side = |dims: &[(String, f64)]| dims.iter().map(|(n, v)| Ok((tag_of(n)?, *v))).collect::<Result<Vec<_>>>();
        cross.push(CrossAxisMapping { input: side(&m.input)?, output: side(&m.output)? });
    }
    Ok(AvarOptions { maps: axes.into_iter().filter(|a| !a.map.is_empty()).map(|a| (a.tag, a.map)).collect(), version2: false, mappings: cross })
}
