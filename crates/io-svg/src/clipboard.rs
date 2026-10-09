//! The clipboard fragment: our own JSON for copied shapes, anchors and guides (the `clipboard`
//! format row). `edit.copy` puts it on the clipboard next to the SVG flavour; `edit.paste`
//! prefers it, because it keeps every attribute the model has.
//!
//! The document is one JSON object:
//!
//! ```json
//! {"format": "fontcraft-clipboard", "version": 1,
//!  "shapes": [{"path": {…}}, {"component": {…}}],
//!  "anchors": [{"name": "top", "x": 250, "y": 700}],
//!  "guides": [{"scope": "local", "guide": {"x": 0, "y": 500}}]}
//! ```
//!
//! Shapes, anchors and guides are written as the native file writes them. Empty lists are left
//! out. A reader takes the versions up to its own and refuses newer ones; fields it does not know
//! are ignored. Pasted text is untrusted: [`decode`] caps its size and its counts and checks every
//! number before the engine sees it.

use fontcraft_model::{Anchor, Component, Guide, MAX_COORDINATE, MAX_LAYER_GUIDES, MAX_NAME_LEN, MAX_OUTLINE_ELEMENTS, Path, Shape};
use serde::{Deserialize, Serialize};

/// The `format` value that marks our fragment.
pub const FRAGMENT_FORMAT: &str = "fontcraft-clipboard";
/// The fragment version this build writes, and the newest it reads.
pub const FRAGMENT_VERSION: u32 = 1;
/// Largest fragment, in bytes.
pub const MAX_FRAGMENT_BYTES: usize = 16 << 20;
/// Most shapes one fragment may hold.
pub const MAX_FRAGMENT_SHAPES: usize = 10_000;
/// Most nodes all the fragment's paths may hold together.
pub const MAX_FRAGMENT_NODES: usize = MAX_OUTLINE_ELEMENTS;
/// Most anchors one fragment may hold.
pub const MAX_FRAGMENT_ANCHORS: usize = 4096;
/// Most guides one fragment may hold.
pub const MAX_FRAGMENT_GUIDES: usize = MAX_LAYER_GUIDES;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ClipboardError {
    #[error("the clipboard text is {size} bytes; the limit is {MAX_FRAGMENT_BYTES}")]
    TooLarge { size: usize },
    #[error("the clipboard text is not JSON: {0}")]
    NotJson(String),
    #[error("the clipboard text is not a FontCraft fragment")]
    NotAFragment,
    #[error("the clipboard fragment has version {0}; this version reads up to {FRAGMENT_VERSION}")]
    TooNew(u32),
    #[error("the clipboard fragment holds more than {limit} {what}")]
    TooMany { what: &'static str, limit: usize },
    #[error("the clipboard fragment is not valid: {0}")]
    Invalid(String),
    #[error("the fragment could not be written: {0}")]
    Encode(String),
}

/// A shape on the clipboard. Its JSON is the model's ([`Shape`]) for the same kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FragmentShape {
    Path(Path),
    Component(Component),
}

impl FragmentShape {
    /// The clipboard form of a shape; `None` for images and raw shapes, which are not copied.
    pub fn from_shape(shape: &Shape) -> Option<Self> {
        match shape {
            Shape::Path(p) => Some(FragmentShape::Path(p.clone())),
            Shape::Component(c) => Some(FragmentShape::Component(c.clone())),
            Shape::Image(_) | Shape::Raw(_) => None,
        }
    }

    pub fn into_shape(self) -> Shape {
        match self {
            FragmentShape::Path(p) => Shape::Path(p),
            FragmentShape::Component(c) => Shape::Component(c),
        }
    }
}

/// Whether a copied guide belonged to the layer or to the master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideScope {
    Local,
    Global,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FragmentGuide {
    pub scope: GuideScope,
    pub guide: Guide,
}

/// What a copy holds, in glyph coordinates as they were in the source layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fragment {
    pub shapes: Vec<FragmentShape>,
    pub anchors: Vec<Anchor>,
    pub guides: Vec<FragmentGuide>,
}

impl Fragment {
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty() && self.anchors.is_empty() && self.guides.is_empty()
    }
}

fn empty<T>(v: &&[T]) -> bool {
    v.is_empty()
}

#[derive(Serialize)]
struct WireOut<'a> {
    format: &'static str,
    version: u32,
    #[serde(skip_serializing_if = "empty")]
    shapes: &'a [FragmentShape],
    #[serde(skip_serializing_if = "empty")]
    anchors: &'a [Anchor],
    #[serde(skip_serializing_if = "empty")]
    guides: &'a [FragmentGuide],
}

/// Read first, so that text that is not ours gets a plain answer instead of a parse error about
/// a field deep inside.
#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    version: Option<u32>,
}

#[derive(Deserialize)]
struct WireIn {
    #[serde(default)]
    shapes: Vec<FragmentShape>,
    #[serde(default)]
    anchors: Vec<Anchor>,
    #[serde(default)]
    guides: Vec<FragmentGuide>,
}

/// The fragment as clipboard text. Fails when the text would be larger than a paste accepts.
pub fn encode(fragment: &Fragment) -> Result<String, ClipboardError> {
    let wire = WireOut {
        format: FRAGMENT_FORMAT,
        version: FRAGMENT_VERSION,
        shapes: &fragment.shapes,
        anchors: &fragment.anchors,
        guides: &fragment.guides,
    };
    let text = serde_json::to_string(&wire).map_err(|e| ClipboardError::Encode(e.to_string()))?;
    if text.len() > MAX_FRAGMENT_BYTES {
        return Err(ClipboardError::TooLarge { size: text.len() });
    }
    Ok(text)
}

/// A fragment from clipboard text, checked: sizes and counts within the limits, every path a
/// well-formed node list, every coordinate finite and within the model's range, every name
/// usable.
pub fn decode(text: &str) -> Result<Fragment, ClipboardError> {
    if text.len() > MAX_FRAGMENT_BYTES {
        return Err(ClipboardError::TooLarge { size: text.len() });
    }
    let header: Header = serde_json::from_str(text).map_err(|e| match e.classify() {
        serde_json::error::Category::Data => ClipboardError::NotAFragment,
        _ => ClipboardError::NotJson(e.to_string()),
    })?;
    if header.format.as_deref() != Some(FRAGMENT_FORMAT) {
        return Err(ClipboardError::NotAFragment);
    }
    match header.version {
        None | Some(0) => return Err(ClipboardError::NotAFragment),
        Some(v) if v > FRAGMENT_VERSION => return Err(ClipboardError::TooNew(v)),
        Some(_) => {}
    }
    let wire: WireIn = serde_json::from_str(text).map_err(|e| ClipboardError::Invalid(e.to_string()))?;
    let fragment = Fragment { shapes: wire.shapes, anchors: wire.anchors, guides: wire.guides };
    check(&fragment)?;
    Ok(fragment)
}

/// Does `text` look like our fragment (rather than SVG or plain text)? A quick test for choosing a
/// flavour; [`decode`] decides.
pub fn looks_like_fragment(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(FRAGMENT_FORMAT)
}

fn check(f: &Fragment) -> Result<(), ClipboardError> {
    if f.shapes.len() > MAX_FRAGMENT_SHAPES {
        return Err(too_many("shapes", MAX_FRAGMENT_SHAPES));
    }
    if f.anchors.len() > MAX_FRAGMENT_ANCHORS {
        return Err(too_many("anchors", MAX_FRAGMENT_ANCHORS));
    }
    if f.guides.len() > MAX_FRAGMENT_GUIDES {
        return Err(too_many("guides", MAX_FRAGMENT_GUIDES));
    }
    let mut nodes = 0usize;
    for shape in &f.shapes {
        match shape {
            FragmentShape::Path(p) => {
                nodes = nodes.saturating_add(p.nodes.len());
                if nodes > MAX_FRAGMENT_NODES {
                    return Err(too_many("nodes", MAX_FRAGMENT_NODES));
                }
                p.to_bez().map_err(|m| ClipboardError::Invalid(format!("a path: {m}")))?;
            }
            FragmentShape::Component(c) => {
                check_name("component glyph", &c.glyph)?;
                if !c.transform.numbers().iter().all(|v| v.is_finite()) {
                    return Err(ClipboardError::Invalid(format!("the transform of the component of `{}` is not a number", c.glyph)));
                }
            }
        }
    }
    for a in &f.anchors {
        check_name("anchor", &a.name)?;
        if !(in_range(a.x) && in_range(a.y)) {
            return Err(ClipboardError::Invalid(format!("anchor `{}` lies beyond ±{MAX_COORDINATE} or is not a number", a.name)));
        }
    }
    for g in &f.guides {
        if !(in_range(g.guide.x) && in_range(g.guide.y) && g.guide.angle.is_finite()) {
            return Err(ClipboardError::Invalid(format!("a guide's position or angle is not a number or lies beyond ±{MAX_COORDINATE}")));
        }
    }
    Ok(())
}

fn too_many(what: &'static str, limit: usize) -> ClipboardError {
    ClipboardError::TooMany { what, limit }
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

/// The model's rule for names: not empty, at most [`MAX_NAME_LEN`] bytes, no control characters.
fn check_name(what: &str, name: &str) -> Result<(), ClipboardError> {
    let problem = if name.trim().is_empty() {
        "is empty"
    } else if name.len() > MAX_NAME_LEN {
        "is too long"
    } else if name.chars().any(char::is_control) {
        "contains a control character"
    } else {
        return Ok(());
    };
    Err(ClipboardError::Invalid(format!("a {what} name {problem}")))
}
