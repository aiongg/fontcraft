//! Copy and paste of cap components, segment components and brushes (`cap.copyPasteOntoSegments`,
//! `segment.copyPasteOntoSegments`, `brush.copyPasteOntoSegments`).
//!
//! `edit.copy` with `caps`, `segmentComponents` or `brushes` (hint indices, one kind per call)
//! writes the attachments as clipboard text without the segments they sat on: the glyph, and for
//! a cap its scale, alignment and Fit. `edit.paste` of that text puts them onto segments: those
//! whose two end nodes are both in `points`, and those in `segments` (each by the node that ends
//! it). A cap goes onto straight segments only. One copied attachment goes onto every target
//! segment; several go onto as many segments, in path order. A selection that holds no fitting
//! segment is refused, and so is `edit.cut`: attachments are copied, then removed with Delete.

use fontcraft_geom::Vec2;
use fontcraft_model::attach::{self, Attachment, CapHint, MAX_CAP_SCALE, SegmentHint};
use fontcraft_model::{Font, Hint, Layer, LayerSide, MAX_LAYER_HINTS, MAX_NAME_LEN, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::corner::{file_index, mode_name, parse_mode};
use super::*;
use crate::params::{Params, Targets};

/// The `format` of the clipboard text.
pub const ATTACHMENTS_FORMAT: &str = "fontcraft-attachments";
/// The newest version of that text this version writes and reads.
const ATTACHMENTS_VERSION: u32 = 1;
/// Longest clipboard text read as copied attachments, in bytes.
const MAX_ATTACHMENTS_TEXT: usize = 1 << 20;

/// The three kinds the clipboard carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Cap,
    Segment,
    Brush,
}

const KINDS: [Kind; 3] = [Kind::Cap, Kind::Segment, Kind::Brush];

impl Kind {
    /// The parameter and result key listing hint indices, also the clipboard's `kind`.
    fn key(self) -> &'static str {
        match self {
            Kind::Cap => "caps",
            Kind::Segment => "segmentComponents",
            Kind::Brush => "brushes",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Cap => "cap components",
            Kind::Segment => "segment components",
            Kind::Brush => "brushes",
        }
    }

    fn is_name(self, name: &str) -> bool {
        match self {
            Kind::Cap => attach::is_cap_name(name),
            Kind::Segment => attach::is_segment_name(name),
            Kind::Brush => attach::is_brush_name(name),
        }
    }

    /// The attachment of this kind that `hint` stores, if it stores one.
    fn read(self, hint: &Hint) -> Option<Attachment> {
        match (self, attach::attachment(hint).ok().flatten()?) {
            (Kind::Cap, a @ Attachment::Cap(_)) | (Kind::Segment, a @ Attachment::Segment(_)) | (Kind::Brush, a @ Attachment::Brush(_)) => Some(a),
            _ => None,
        }
    }
}

/// Copied attachments as clipboard text.
#[derive(Serialize, Deserialize)]
struct Clip {
    format: String,
    version: u32,
    /// The key of the kind: `caps`, `segmentComponents` or `brushes`.
    kind: String,
    items: Vec<ClipItem>,
}

/// One copied attachment: what it draws, without the segment it sat on.
#[derive(Serialize, Deserialize)]
struct ClipItem {
    name: String,
    /// A cap's horizontal and vertical scale as factors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scale: Option<[f64; 2]>,
    /// A cap's alignment: left, right, both or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alignment: Option<String>,
    /// A cap's Fit.
    #[serde(default)]
    fit: bool,
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
}

/// The kind whose key `p` holds; an error when it holds more than one.
fn given_kind(p: &Params) -> Result<Option<Kind>> {
    let given: Vec<Kind> = KINDS.into_iter().filter(|k| p.get(k.key()).is_some()).collect();
    match given.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        _ => Err(p.bad("give one of `caps`, `segmentComponents` and `brushes`: the clipboard holds one kind at a time")),
    }
}

/// The foreground target object without target lists other than `points` and `segments` (when
/// `lists_allowed`).
fn foreground(p: &Params, lists_allowed: bool, why: &str) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if t.side != LayerSide::Foreground {
        return Err(p.bad("cap components, segment components and brushes sit on the foreground only"));
    }
    let others = !(t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty());
    if others || (!lists_allowed && !(t.points.is_empty() && t.segments.is_empty())) {
        return Err(p.bad(why));
    }
    Ok(t)
}

/// `edit.cut` refuses copied attachments rather than cutting the whole layer.
pub(crate) fn refuse_cut(p: &Params) -> Result<()> {
    match given_kind(p)? {
        Some(kind) => Err(p.bad(format!("{} cannot be cut: copy them with edit.copy, then remove them with edit.delete", kind.noun()))),
        None => Ok(()),
    }
}

/// `edit.copy` with `caps`, `segmentComponents` or `brushes`: those attachments as clipboard text,
/// without their segments; `None` when `p` names none of them.
pub(crate) fn copy(s: &Session, p: &Params) -> Result<Option<Value>> {
    let Some(kind) = given_kind(p)? else { return Ok(None) };
    let key = kind.key();
    let t = foreground(p, false, &format!("addresses {} with `{key}` and takes no target lists", kind.noun()))?;
    let list = p.array(key)?;
    if list.is_empty() || list.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("`{key}` must name 1 to {MAX_LAYER_HINTS} hints, it names {}", list.len())));
    }
    let r = t.resolve(s.font()?)?;
    let hints = &r.layer.info.hints;
    let mut items = Vec::with_capacity(list.len());
    let mut seen = Vec::with_capacity(list.len());
    for v in list {
        let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| p.bad(format!("`{key}` must hold hint indices, found {v}")))?;
        if seen.contains(&i) {
            continue;
        }
        seen.push(i);
        let hint = hints.get(i).ok_or_else(|| p.bad(format!("`{key}`: {i} does not resolve: the layer has {} hints", hints.len())))?;
        let item = match kind.read(hint) {
            Some(Attachment::Cap(c)) => {
                ClipItem { name: c.name, scale: Some([c.scale.x, c.scale.y]), alignment: Some(mode_name(c.alignment).to_string()), fit: c.fit }
            }
            Some(Attachment::Segment(h) | Attachment::Brush(h)) => ClipItem { name: h.name, scale: None, alignment: None, fit: false },
            None => return Err(p.bad(format!("`{key}`: hint {i} is not one of the {}", kind.noun()))),
        };
        items.push(item);
    }
    let count = items.len();
    let clip = Clip { format: ATTACHMENTS_FORMAT.into(), version: ATTACHMENTS_VERSION, kind: key.into(), items };
    let text = serde_json::to_string(&clip).map_err(|e| p.bad(e.to_string()))?;
    let mut out = json!({ "json": text });
    if let Some(o) = out.as_object_mut() {
        o.insert(key.into(), json!(count));
    }
    Ok(Some(out))
}

/// The copied-attachments text in `json` or `text`, if either holds some.
fn clip_text<'a>(p: &Params<'a>) -> Result<Option<&'a str>> {
    for key in ["json", "text"] {
        if let Some(text) = p.opt_str(key)?
            && text.trim_start().starts_with('{')
            && text.contains(ATTACHMENTS_FORMAT)
            && serde_json::from_str::<Header>(text).is_ok_and(|h| h.format.as_deref() == Some(ATTACHMENTS_FORMAT))
        {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

/// One attachment to paste, checked.
struct Copied {
    name: String,
    scale: Vec2,
    alignment: fontcraft_model::corner::CornerAlignment,
    fit: bool,
}

/// The kind and the attachments of clipboard text, checked: pasted text is untrusted.
fn decode(p: &Params, text: &str) -> Result<(Kind, Vec<Copied>)> {
    if text.len() > MAX_ATTACHMENTS_TEXT {
        return Err(p.bad(format!("the copied attachments are {} bytes; the limit is {MAX_ATTACHMENTS_TEXT}", text.len())));
    }
    let clip: Clip = serde_json::from_str(text).map_err(|e| p.bad(format!("the copied attachments cannot be read: {e}")))?;
    if clip.version > ATTACHMENTS_VERSION {
        return Err(p.bad(format!("the copied attachments have version {}; this version reads up to {ATTACHMENTS_VERSION}", clip.version)));
    }
    let kind = KINDS
        .into_iter()
        .find(|k| k.key() == clip.kind)
        .ok_or_else(|| p.bad(format!("the copied attachments are of an unknown kind `{}`", clip.kind)))?;
    if clip.items.is_empty() || clip.items.len() > MAX_LAYER_HINTS {
        return Err(p.bad(format!("the clipboard must hold 1 to {MAX_LAYER_HINTS} {}, it holds {}", kind.noun(), clip.items.len())));
    }
    let mut out = Vec::with_capacity(clip.items.len());
    for item in clip.items {
        let [x, y] = item.scale.unwrap_or([1.0, 1.0]);
        for f in [x, y] {
            if !f.is_finite() || f == 0.0 || f.abs() > MAX_CAP_SCALE {
                return Err(p.bad(format!("a copied scale must not be 0 and lie within ±{} percent, got {}", MAX_CAP_SCALE * 100.0, f * 100.0)));
            }
        }
        let alignment = parse_mode(p, item.alignment.as_deref().unwrap_or("left"), true)?;
        out.push(Copied { name: item.name, scale: Vec2::new(x, y), alignment, fit: item.fit });
    }
    Ok((kind, out))
}

/// Refuses `name` unless it is a glyph of `kind` in `font` other than `host`.
fn check_glyph(p: &Params, font: &Font, kind: Kind, name: &str, host: &str) -> Result<()> {
    if !kind.is_name(name) || name.len() > MAX_NAME_LEN {
        return Err(p.bad(format!("the copied `{name}` is not a glyph of the {}", kind.noun())));
    }
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    if name == host {
        return Err(p.bad(format!("`{name}` cannot be pasted onto `{name}` itself")));
    }
    Ok(())
}

/// The segments of `layer` that take a pasted attachment of `kind`, in path order, as (path, file-order
/// node the hint stores): those whose two end nodes are in `points` and those in `segments`. A cap
/// stores the segment's first node and takes straight segments only; the others store its end node.
fn target_segments(p: &Params, layer: &Layer, kind: Kind, points: &[(usize, usize)], segments: &[(usize, usize)]) -> Result<Vec<(usize, usize)>> {
    let mut shapes: Vec<usize> = points.iter().chain(segments).map(|&(s, _)| s).collect();
    shapes.sort_unstable();
    shapes.dedup();
    let mut out = Vec::new();
    for shape in shapes {
        let path = match layer.shapes.get(shape) {
            Some(Shape::Path(path)) => path,
            Some(_) => return Err(p.bad(format!("shape {shape} is not a path"))),
            None => return Err(p.bad(format!("shape {shape} does not resolve: the layer has {} shapes", layer.shapes.len()))),
        };
        let len = path.nodes.len();
        for seg in path.segments() {
            let chosen = (points.contains(&(shape, seg.start)) && points.contains(&(shape, seg.end))) || segments.contains(&(shape, seg.end));
            if !chosen || (kind == Kind::Cap && !seg.handles.is_empty()) {
                continue;
            }
            let node = if kind == Kind::Cap { seg.start } else { seg.end };
            let target = (shape, file_index(path.closed, len, node));
            if !out.contains(&target) {
                out.push(target);
            }
        }
    }
    Ok(out)
}

/// `edit.paste` of copied caps, segment components or brushes onto the selected segments; `None`
/// when the clipboard text holds none.
pub(crate) fn paste(s: &mut Session, p: &Params) -> Result<Option<Value>> {
    let Some(text) = clip_text(p)? else { return Ok(None) };
    let (kind, copied) = decode(p, text)?;
    let noun = kind.noun();
    let mut t =
        foreground(p, true, &format!("the clipboard holds {noun}: they paste onto segments, given by their nodes in `points` or as `segments`"))?;
    if t.points.is_empty() && t.segments.is_empty() {
        return Err(p.bad(format!("the clipboard holds {noun}: select the segments to paste them onto (`points` or `segments`)")));
    }
    let font = s.font()?;
    for (k, c) in copied.iter().enumerate() {
        if copied.iter().take(k).all(|o| o.name != c.name) {
            check_glyph(p, font, kind, &c.name, &t.glyph)?;
        }
    }
    let layer = t.layer_id(font)?;
    t.layer = Some(layer.clone());
    let (points, segments) = (t.points.clone(), t.segments.clone());
    let indices = s.edit("Paste", |font| {
        t.edit_layer(font, |l| {
            let targets = target_segments(p, l, kind, &points, &segments)?;
            if targets.is_empty() {
                let what = if kind == Kind::Cap { "the two nodes of a straight segment" } else { "both end nodes of a segment" };
                return Err(p.bad(format!("the clipboard holds {noun}: the selection holds no segment to paste them onto; select {what}")));
            }
            if copied.len() > 1 && copied.len() != targets.len() {
                return Err(p.bad(format!(
                    "the clipboard holds {} {noun} and the selection {} segments; select one segment for each, or copy one",
                    copied.len(),
                    targets.len()
                )));
            }
            let mut indices = Vec::with_capacity(targets.len());
            for (k, &(shape, node)) in targets.iter().enumerate() {
                let Some(c) = copied.get(k).or(copied.first()) else { continue };
                let hint = match kind {
                    Kind::Cap => CapHint { name: c.name.clone(), shape, node, scale: c.scale, alignment: c.alignment, fit: c.fit }.to_hint(),
                    Kind::Segment => Attachment::Segment(SegmentHint::new(c.name.clone(), shape, node)).to_hint(),
                    Kind::Brush => Attachment::Brush(SegmentHint::new(c.name.clone(), shape, node)).to_hint(),
                };
                let existing = l.info.hints.iter().position(|h| kind.read(h).is_some_and(|a| a.origin() == (shape, node)));
                if let Some(i) = existing
                    && let Some(slot) = l.info.hints.get_mut(i)
                {
                    *slot = hint;
                    indices.push(i);
                    continue;
                }
                if l.info.hints.len() >= MAX_LAYER_HINTS {
                    return Err(p.bad(format!("the layer already holds {MAX_LAYER_HINTS} hints, the most allowed")));
                }
                l.info.hints.push(hint);
                indices.push(l.info.hints.len() - 1);
            }
            Ok(indices)
        })
    })?;
    let key = kind.key();
    let mut selection = Targets::new(p.command(), t.glyph.clone(), Some(layer.clone())).to_json();
    if let Some(o) = selection.as_object_mut() {
        o.insert(key.into(), json!(indices));
    }
    let mut out = json!({"glyph": t.glyph, "layer": layer, "pasted": indices.len(), "flavour": key, "selection": selection});
    if let Some(o) = out.as_object_mut() {
        o.insert(key.into(), json!(indices));
    }
    Ok(Some(out))
}
