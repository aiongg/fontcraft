//! GLIF, the UFO's file per glyph per layer: reading into a model [`Layer`] and writing back.
//!
//! The writer follows fontTools' `glifLib` (MIT; see `NOTICE`): elements in the order advance,
//! unicode, note, image, guideline, anchor, outline, lib; attributes in its order (`height` before
//! `width`; `x`, `y`, `type`, `smooth`, then `name` and `identifier` on a point; a transform only
//! where it is not the identity); numbers as Python writes them; the file pretty-printed by lxml.
//! The indentation, the XML declaration and whether whole numbers are spelled as floats (`300.0`)
//! are a [`Style`], read from the file.
//!
//! The reader builds, for every element, the start tag this writer would write for what it read.
//! Where the file's tag differs (another attribute order or quote, `447.0` in an integer file,
//! lowercase hexadecimal), the file's tag is kept, and the writer uses it as long as it still
//! means what the model holds; an edit writes the canonical tag.
//!
//! What the model has no field for goes to the object's [`Extra`] under [`Family::Ufo`]:
//!
//! | Key | On | Holds |
//! |---|---|---|
//! | `@name` | any object | an attribute the model has no field for (a point's `name` and `identifier`, a contour's `identifier`, an anchor's `color`…), in source order |
//! | `<` | path, node, component, anchor | the file's start tag, where it differs from the writer's |
//! | `<glyph`, `<advance`, `<outline`, `<lib` | layer | the same, for elements without an object of their own |
//! | `<unicode` | layer | the file's `unicode` start tags, `""` where they equal the writer's |
//! | `noteXml` | layer | the `<note>` element as XML text, where it differs from the writer's for the note it holds |
//! | `+image`, `+guideline`, `+…` | layer | an element the model does not read (an image or guideline whose writer's tag would mean something else, a second `<note>`…), as XML text, in source order |
//! | `order` | layer | the names of the glyph's child elements, when they were not in the writer's order |
//! | `lib` | layer | the glyph lib as read, a property-list dictionary (its keys other than [`TYPED_LIB`] are also the layer's user data) |
//! | `outline` | layer | the text inside an empty `<outline>` (fontTools writes a line break and two spaces); an empty outline is written only where a file had one |
//! | `text` | path | the text inside an empty `<contour>` |
//! | `noName` | anchor | the anchor has no `name` attribute |
//! | `<point`, `anchorAt` | anchor, layer | format 1: the file's tag of the point that stands for an anchor, and where in the outline the anchors were when not after the shapes |
//! | `libGaps` | layer | the whitespace around the lib's dictionary, when not the pretty printer's |
//!
//! A lib that is not a well-formed property list is kept as an unknown element (`+lib`).
//!
//! Model fields: the advance height is the layer's `vert_width`; `<guideline>` elements are its
//! guides, read and written by glyphsLib's guideline rules ([`crate::design`]); `<image>` is its
//! image (`fileName` the path, the six transformation attributes a matrix, `color` kept as
//! `@color`); the lib's keys other than [`TYPED_LIB`] are its user data. The writer patches the
//! kept lib: a key whose value still means the same keeps its spelling. GLIF format 1 has no
//! guidelines or images; they are not written there.
//!
//! The note ([`Glif::note`]) is read as fontTools reads it (each line stripped, blank lines left
//! out) and written as it writes it (the note between line breaks). The mark colour stays in the
//! lib; [`write`] sets it from a [`MarkColor`], and [`mark_color`] and [`mark_color_text`]
//! convert between its text and a [`ColorLabel`] as glyphsLib 6.15.0 does (`builder/glyph.py`,
//! the label colours of `builder/constants.py`; Apache-2.0, see `NOTICE`).
//!
//! Contours the model cannot hold as a [`Path`] (a `curve` after one or three off-curve points, an
//! open contour ending in off-curve points) and unknown elements inside `<outline>` are kept as a
//! [`Shape::Raw`] holding their XML text. A shape another format's reader kept that way is left
//! out of the file.

use fontcraft_model::{
    Anchor, ColorLabel, Component, Extra, Family, Guide, Image, Layer, MAX_COORDINATE, MAX_LAYER_GUIDES, Node, NodeKind, Path, RawShape, Shape,
    Transform, UserData,
};
use fontcraft_plist::xml::{self, Element, Node as XNode};
use fontcraft_plist::{Array, Dict, Layout, Number, Value, xmlplist};

use crate::{Result, UfoError, bad};

/// How a set of GLIF files is laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// The text between `<?` and `?>` of the XML declaration; empty for a file without one.
    pub declaration: String,
    pub indent: String,
    pub final_newline: bool,
    /// Whole numbers are written as floats (`300.0`).
    pub float_numbers: bool,
}

impl Style {
    /// What fontTools writes today.
    pub fn fonttools() -> Self {
        Self { declaration: xmlplist::DECLARATION.into(), indent: "  ".into(), final_newline: true, float_numbers: false }
    }

    pub fn to_value(&self) -> Value {
        let mut d = Dict::new();
        d.insert("declaration", Value::String(self.declaration.as_str().into()));
        d.insert("finalNewline", Value::Bool(self.final_newline));
        d.insert("floatNumbers", Value::Bool(self.float_numbers));
        d.insert("indent", Value::String(self.indent.as_str().into()));
        Value::Dict(d)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let d = v.as_dict()?;
        Some(Self {
            declaration: d.get("declaration")?.as_str()?.to_string(),
            indent: d.get("indent")?.as_str()?.to_string(),
            final_newline: d.get("finalNewline")?.as_bool()?,
            float_numbers: d.get("floatNumbers")?.as_bool()?,
        })
    }
}

/// What one GLIF file holds.
pub struct Glif {
    /// The `name` attribute (the UFO's `contents.plist` decides the glyph's name).
    pub name: String,
    pub unicodes: Vec<u32>,
    /// The note, as fontTools reads it.
    pub note: Option<String>,
    /// The GLIF format version: 1 in UFO 2, 2 in UFO 3.
    pub format: u32,
    pub layer: Layer,
    pub style: Style,
}

const ELEMENT_ORDER: [&str; 8] = ["advance", "unicode", "note", "image", "guideline", "anchor", "outline", "lib"];
/// The transform attributes in the order fontTools writes them, with their defaults.
const TRANSFORM: [(&str, f64); 6] = [("xScale", 1.0), ("xyScale", 0.0), ("yxScale", 0.0), ("yScale", 1.0), ("xOffset", 0.0), ("yOffset", 0.0)];

/// Where a child element goes in the writer's order; unknown elements go between the outline
/// and the lib.
fn rank(name: &str) -> usize {
    match ELEMENT_ORDER.iter().position(|e| *e == name) {
        Some(7) => 8,
        Some(i) => i,
        None => 7,
    }
}

fn ufo(x: &mut Extra, key: &str, v: Value) {
    x.push(Family::Ufo, key, v);
}

fn s(text: &str) -> Value {
    Value::String(text.into())
}

/// The mark colour's key in a glyph lib.
pub const MARK_COLOR: &str = "public.markColor";
/// The vertical origin's key in a glyph lib.
pub const VERTICAL_ORIGIN: &str = "public.verticalOrigin";
/// Glyph lib keys that are fields of their own rather than the layer's user data.
pub const TYPED_LIB: [&str; 2] = [MARK_COLOR, VERTICAL_ORIGIN];

/// The reference's label colours as glyphsLib writes them to `public.markColor`, by index.
const LABEL_COLORS: [&str; 12] = [
    "0.85,0.26,0.06,1",
    "0.99,0.62,0.11,1",
    "0.65,0.48,0.2,1",
    "0.97,1,0,1",
    "0.67,0.95,0.38,1",
    "0.04,0.57,0.04,1",
    "0,0.67,0.91,1",
    "0.18,0.16,0.78,1",
    "0.5,0.09,0.79,1",
    "0.98,0.36,0.67,1",
    "0.75,0.75,0.75,1",
    "0.25,0.25,0.25,1",
];

/// A `public.markColor` value as a colour label: a label colour's text is its index; other text
/// is four numbers from 0 to 1, separated by commas.
pub fn mark_color(text: &str) -> Option<ColorLabel> {
    if let Some(i) = LABEL_COLORS.iter().position(|c| *c == text) {
        return u8::try_from(i).ok().map(ColorLabel::Index);
    }
    let parts: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let [r, g, b, a] = parts.as_slice() else { return None };
    Some(ColorLabel::Rgba([*r, *g, *b, *a]))
}

/// A colour label as `public.markColor` text; `None` for an index the label colours do not have.
/// A component that is a whole number of 255ths is written with three decimals (`0` and `1`
/// bare), as glyphsLib writes 8-bit components; any other in its shortest form.
pub fn mark_color_text(c: &ColorLabel) -> Option<String> {
    match c {
        ColorLabel::Index(i) => LABEL_COLORS.get(usize::from(*i)).map(|t| (*t).to_string()),
        ColorLabel::Rgba(parts) => {
            let one = |v: f64| {
                // In range for a valid colour, so the conversion is exact.
                #[allow(clippy::cast_possible_truncation)]
                let byte = (v * 255.0).round() as i64;
                #[allow(clippy::cast_precision_loss)]
                let whole = (byte as f64) / 255.0 == v;
                if v == 0.0 {
                    "0".to_string()
                } else if v == 1.0 {
                    "1".to_string()
                } else if whole {
                    format!("{v:.3}")
                } else {
                    format!("{v}")
                }
            };
            Some(parts.iter().map(|v| one(*v)).collect::<Vec<_>>().join(","))
        }
    }
}

/// What [`write`] does with the lib's mark colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MarkColor<'a> {
    /// Leaves the lib as it is.
    Keep,
    /// Sets this text, unless the lib's value already means the same colour.
    Set(&'a str),
    /// Removes a mark colour the lib has (one that does not read as a colour stays).
    Remove,
}

fn patch_mark_color(lib: &mut Dict, want: MarkColor<'_>) {
    let at = lib.entries.iter().position(|(k, _)| k.as_str() == MARK_COLOR);
    let kept = at.and_then(|i| lib.entries.get(i)).and_then(|(_, v)| v.as_str()).and_then(mark_color);
    match (want, at) {
        (MarkColor::Keep, _) => {}
        (MarkColor::Set(text), _) if kept.is_some() && kept == mark_color(text) => {}
        (MarkColor::Set(text), Some(i)) => {
            if let Some((_, v)) = lib.entries.get_mut(i) {
                *v = s(text);
            }
        }
        (MarkColor::Set(text), None) => {
            let i = lib.entries.iter().position(|(k, _)| k.as_str() > MARK_COLOR).unwrap_or(lib.entries.len());
            lib.insert_at(i, MARK_COLOR, s(text));
        }
        (MarkColor::Remove, Some(i)) if kept.is_some() => {
            lib.remove_at(i);
        }
        (MarkColor::Remove, _) => {}
    }
}

/// A note as fontTools reads it: each line stripped of white space, blank lines left out.
fn read_note(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The `<note>` element fontTools writes for a note.
fn note_el(note: &str) -> Element {
    Element::new("note").with_text(format!("\n{}\n", note.trim()))
}

/// Whether writing `note` and reading it back gives the same note.
pub fn note_survives(note: &str) -> bool {
    note.is_empty() || read_note(&note_el(note).text()).as_deref() == Some(note)
}

fn hint<'a>(x: &'a Extra, key: &str) -> Option<&'a str> {
    x.get(Family::Ufo, key).and_then(Value::as_str)
}

/// The file's start tag of `src` when it differs from the writer's tag `ours` for the same
/// content.
fn tag_if_different(src: &Element, ours: &Element) -> Option<String> {
    let theirs = src.start_tag_text();
    let mine = ours.canonical_start_tag(theirs.ends_with("/>"));
    (theirs != mine).then_some(theirs)
}

/// Keeps the file's start tag of `src` in `x` under `key` when it differs from `ours`.
fn keep_tag(x: &mut Extra, key: &str, src: &Element, ours: &Element) {
    if let Some(t) = tag_if_different(src, ours) {
        ufo(x, key, s(&t));
    }
}

/// Uses a kept start tag for `el` when it means the same as what was built.
fn apply_source(el: &mut Element, source: Option<&str>) {
    if let Some(src) = source
        && !src.is_empty()
        && xml::parse_start_tag(src).is_ok_and(|parsed| xml::same_attributes(&parsed, el))
    {
        el.set_start_source(src);
    }
}

fn apply_tag(el: &mut Element, x: &Extra, key: &str) {
    apply_source(el, hint(x, key));
}

/// Attributes of `el` other than `known`, into `x` as `@name` entries.
fn keep_other_attrs(x: &mut Extra, el: &Element, known: &[&str]) {
    for a in el.attrs() {
        if !known.contains(&a.name.as_str()) {
            ufo(x, &format!("@{}", a.name), s(&a.value));
        }
    }
}

/// The `@name` entries of `x`, as attributes of `el`.
fn put_other_attrs(el: &mut Element, x: &Extra) {
    for e in x.of(Family::Ufo) {
        if let (Some(name), Some(v)) = (e.key.as_str().strip_prefix('@'), e.value.as_str()) {
            el.set_attr(name, v);
        }
    }
}

fn number(el: &Element, attr: &str, at: &str) -> Result<Option<f64>> {
    let Some(v) = el.attr(attr) else { return Ok(None) };
    match v.trim().parse::<f64>() {
        Ok(f) if f.is_finite() => Ok(Some(f)),
        _ => Err(bad(at, format!("`{attr}=\"{v}\"` is not a number"))),
    }
}

fn required(el: &Element, attr: &str, at: &str) -> Result<f64> {
    number(el, attr, at)?.ok_or_else(|| bad(at, format!("`<{}>` has no `{attr}`", el.name())))
}

/// A number as glifLib writes it: Python's `repr` of an int or a float. Whole numbers are ints
/// unless the style spells them as floats.
pub fn fmt_number(v: f64, float_numbers: bool) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        let i = v as i64;
        if float_numbers { format!("{i}.0") } else { i.to_string() }
    } else {
        xmlplist::py_float_repr(v)
    }
}

/// Counts whole numbers spelled with and without a decimal point, over the attributes that
/// hold numbers.
fn count_spellings(el: &Element, with_dot: &mut usize, without: &mut usize) {
    for a in el.attrs() {
        if matches!(a.name.as_str(), "x" | "y" | "width" | "height" | "xOffset" | "yOffset" | "xScale" | "yScale" | "xyScale" | "yxScale" | "angle") {
            let v = a.value.trim();
            if v.parse::<f64>().is_ok_and(|f| f.fract() == 0.0) {
                if v.contains('.') { *with_dot += 1 } else { *without += 1 }
            }
        }
    }
    for c in el.elements() {
        count_spellings(c, with_dot, without);
    }
}

/// The indentation of a document's first nested line, or two spaces when nothing is nested.
fn detect_indent(root: &Element) -> String {
    for n in &root.children {
        if let XNode::Text(t) = n
            && t.as_str().contains('\n')
            && let Some(line) = t.as_str().rsplit('\n').next()
        {
            return line.to_string();
        }
    }
    "  ".into()
}

// ── the writer's elements, shared by the reader to tell which tags to keep ─────────────────────

fn glyph_el(name: &str, format: u32, x: &Extra) -> Element {
    let mut el = Element::new("glyph").with_attr("name", name).with_attr("format", format.to_string());
    put_other_attrs(&mut el, x);
    el
}

fn advance_el(width: f64, height: Option<f64>, style: &Style) -> Option<Element> {
    let height = height.filter(|h| *h != 0.0);
    if width == 0.0 && height.is_none() {
        return None;
    }
    let mut el = Element::new("advance");
    if let Some(h) = height {
        el.set_attr("height", fmt_number(h, style.float_numbers));
    }
    if width != 0.0 {
        el.set_attr("width", fmt_number(width, style.float_numbers));
    }
    Some(el)
}

/// A guide as a `<guideline>`, by glyphsLib's guideline rules, attributes in glifLib's order.
fn guideline_el(g: &Guide, style: &Style) -> Option<Element> {
    let v = crate::design::guideline(g)?;
    let d = v.as_dict()?;
    let mut el = Element::new("guideline");
    for key in ["x", "y", "angle"] {
        if let Some(n) = d.get(key).and_then(Value::as_num) {
            el.set_attr(key, fmt_number(n.as_f64(), style.float_numbers));
        }
    }
    for key in ["name", "color", "identifier"] {
        if let Some(t) = d.get(key).and_then(Value::as_str) {
            el.set_attr(key, t);
        }
    }
    Some(el)
}

/// A `<guideline>` as a guide, when writing the guide gives an element that means the same.
fn read_guideline(el: &Element, style: &Style) -> Option<Guide> {
    let mut d = fontcraft_plist::Dict::new();
    for a in el.attrs() {
        match a.name.as_str() {
            "x" | "y" | "angle" => {
                d.insert(a.name.as_str(), Value::Number(plist_number(a.value.trim().parse::<f64>().ok().filter(|f| f.is_finite())?)))
            }
            "name" | "color" | "identifier" => d.insert(a.name.as_str(), s(&a.value)),
            _ => return None,
        }
    }
    let mut g = crate::design::guide_of(&Value::Dict(d))?;
    if !(g.x.abs() <= MAX_COORDINATE && g.y.abs() <= MAX_COORDINATE) {
        return None;
    }
    let ours = guideline_el(&g, style)?;
    if !xml::same_attributes(&ours, el) {
        return None;
    }
    keep_tag(&mut g.extra, "<", el, &ours);
    Some(g)
}

/// An image as an `<image>`: the file name, the transformation attributes that are not the
/// identity's, and the attributes kept as `@name`.
fn image_el(i: &Image, style: &Style) -> Option<Element> {
    let mut el = Element::new("image").with_attr("fileName", i.path.as_deref()?);
    for ((name, default), v) in TRANSFORM.iter().zip(i.transform.affine().as_coeffs()) {
        if !v.is_finite() {
            return None;
        }
        if v != *default {
            el.set_attr(*name, fmt_number(v, style.float_numbers));
        }
    }
    put_other_attrs(&mut el, &i.extra);
    Some(el)
}

/// An `<image>` as an image, when writing it gives an element that means the same.
fn read_image(el: &Element, style: &Style) -> Option<Image> {
    let path = el.attr("fileName")?.to_string();
    let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    for (slot, (name, default)) in m.iter_mut().zip(TRANSFORM) {
        *slot = match el.attr(name) {
            Some(v) => v.trim().parse::<f64>().ok().filter(|f| f.is_finite())?,
            None => default,
        };
    }
    let mut i = Image { path: Some(path), transform: Transform::Matrix(m), ..Default::default() };
    let known = ["fileName", "xScale", "xyScale", "yxScale", "yScale", "xOffset", "yOffset"];
    keep_other_attrs(&mut i.extra, el, &known);
    let ours = image_el(&i, style)?;
    if !xml::same_attributes(&ours, el) {
        return None;
    }
    keep_tag(&mut i.extra, "<", el, &ours);
    Some(i)
}

/// Sets the lib's keys from the layer's user data: a key the user data no longer has goes, a new
/// one goes in sorted position, and a kept value that means the same stays as written. The keys
/// of [`TYPED_LIB`] are left to their fields.
fn patch_user_data(lib: &mut Dict, user: &UserData) {
    let stale: Vec<String> =
        lib.entries.iter().map(|(k, _)| k.as_str()).filter(|k| !TYPED_LIB.contains(k) && user.get(k).is_none()).map(str::to_string).collect();
    for k in stale {
        crate::ufo::patch(lib, &k, None);
    }
    for (k, v) in user.0.entries.iter().filter(|(k, _)| !TYPED_LIB.contains(&k.as_str())) {
        crate::ufo::patch(lib, k.as_str(), Some(v.without_spellings()));
    }
}

fn unicode_el(u: u32) -> Element {
    Element::new("unicode").with_attr("hex", format!("{u:04X}"))
}

fn anchor_el(a: &Anchor, style: &Style) -> Element {
    let mut el = Element::new("anchor").with_attr("x", fmt_number(a.x, style.float_numbers)).with_attr("y", fmt_number(a.y, style.float_numbers));
    if !a.name.is_empty() || a.extra.get(Family::Ufo, "noName").and_then(Value::as_bool) != Some(true) {
        el.set_attr("name", &a.name);
    }
    put_other_attrs(&mut el, &a.extra);
    el
}

/// A format 1 anchor: the point of a one-point contour.
fn anchor_point_el(a: &Anchor, style: &Style) -> Element {
    let num = |v: f64| fmt_number(v, style.float_numbers);
    Element::new("point").with_attr("x", num(a.x)).with_attr("y", num(a.y)).with_attr("type", "move").with_attr("name", &a.name)
}

fn point_el(n: &Node, starts_open_path: bool, style: &Style) -> Element {
    let mut el = Element::new("point").with_attr("x", fmt_number(n.x, style.float_numbers)).with_attr("y", fmt_number(n.y, style.float_numbers));
    let kind = match n.kind {
        _ if starts_open_path => Some("move"),
        NodeKind::Line => Some("line"),
        NodeKind::Curve => Some("curve"),
        NodeKind::QCurve => Some("qcurve"),
        NodeKind::OffCurve => None,
    };
    if let Some(k) = kind {
        el.set_attr("type", k);
    }
    if n.smooth {
        el.set_attr("smooth", "yes");
    }
    put_other_attrs(&mut el, &n.extra);
    el
}

fn contour_el(p: &Path) -> Element {
    let mut el = Element::new("contour");
    put_other_attrs(&mut el, &p.extra);
    el
}

fn component_el(c: &Component, style: &Style) -> Element {
    let mut el = Element::new("component").with_attr("base", &c.glyph);
    for ((name, default), v) in TRANSFORM.iter().zip(c.affine().as_coeffs()) {
        if v != *default {
            el.set_attr(*name, fmt_number(v, style.float_numbers));
        }
    }
    put_other_attrs(&mut el, &c.extra);
    el
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

/// Reads a GLIF file into a layer with the given id and master.
pub fn read(text: &str, at: &str, layer_id: &str, master: &str) -> Result<Glif> {
    let doc = xml::parse(text).map_err(|e| UfoError::Syntax { file: at.to_string(), error: e })?;
    let root = &doc.root;
    if root.name() != "glyph" {
        return Err(bad(at, "the root element is not `<glyph>`"));
    }
    let (mut with_dot, mut without) = (0, 0);
    count_spellings(root, &mut with_dot, &mut without);
    let style = Style {
        declaration: doc.declaration().unwrap_or("").to_string(),
        indent: detect_indent(root),
        final_newline: doc.epilog.iter().any(|n| matches!(n, XNode::Text(t) if t.as_str().contains('\n'))),
        float_numbers: with_dot > without,
    };
    let format = match root.attr("format").map(str::trim) {
        Some("1") => 1,
        Some("2") => 2,
        Some(v) => return Err(UfoError::Unsupported(format!("{at}: GLIF format {v}"))),
        None => return Err(bad(at, "`<glyph>` has no format")),
    };
    let name = root.attr("name").unwrap_or("").to_string();
    let mut layer = Layer::new(master, 0.0);
    layer.id = layer_id.to_string();
    keep_other_attrs(&mut layer.extra, root, &["name", "format"]);
    let ours = glyph_el(&name, format, &layer.extra);
    keep_tag(&mut layer.extra, "<glyph", root, &ours);

    let mut unicodes = Vec::new();
    let mut note = None;
    let mut seen_note = false;
    let mut unicode_tags: Vec<Value> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for el in root.elements() {
        names.push(el.name());
        let at = format!("{at}: <{}>", el.name());
        match el.name() {
            "advance" => {
                layer.width = number(el, "width", &at)?.unwrap_or(0.0);
                let height = number(el, "height", &at)?;
                match height {
                    // Beyond the model's limit: kept as written.
                    Some(h) if h.abs() > MAX_COORDINATE => ufo(&mut layer.extra, "height", Value::Number(plist_number(h))),
                    Some(_) => layer.info.vert_width = height,
                    None => {}
                }
                match advance_el(layer.width, height, &style) {
                    Some(ours) => keep_tag(&mut layer.extra, "<advance", el, &ours),
                    // The writer leaves out a zero advance; this file wrote one.
                    None => ufo(&mut layer.extra, "<advance", s(&el.start_tag_text())),
                }
            }
            "unicode" => {
                let hex = el.attr("hex").ok_or_else(|| bad(&at, "no `hex`"))?;
                let u = u32::from_str_radix(hex.trim(), 16).map_err(|_| bad(&at, format!("`{hex}` is not hexadecimal")))?;
                unicodes.push(u);
                unicode_tags.push(s(&tag_if_different(el, &unicode_el(u)).unwrap_or_default()));
            }
            "note" if !seen_note => {
                seen_note = true;
                note = read_note(&el.text());
                let (mut theirs, mut ours) = (String::new(), String::new());
                el.write(&mut theirs);
                if let Some(n) = &note {
                    note_el(n).write(&mut ours);
                }
                if theirs != ours {
                    ufo(&mut layer.extra, "noteXml", s(&theirs));
                }
            }
            "anchor" => {
                let a = read_anchor(el, &at, &style)?;
                layer.anchors.push(a);
            }
            "guideline" => match (format >= 2 && layer.info.guides.len() < MAX_LAYER_GUIDES).then(|| read_guideline(el, &style)).flatten() {
                Some(g) => layer.info.guides.push(g),
                None => keep_element(&mut layer.extra, el),
            },
            "image" => match (format >= 2 && layer.info.image.is_none()).then(|| read_image(el, &style)).flatten() {
                Some(i) => layer.info.image = Some(i),
                None => keep_element(&mut layer.extra, el),
            },
            "outline" => {
                keep_tag(&mut layer.extra, "<outline", el, &Element::new("outline"));
                // An empty outline is written only where the file had one.
                if el.elements().next().is_none() {
                    ufo(&mut layer.extra, "outline", s(&el.text()));
                }
                let mut anchor_at = Vec::new();
                for (i, child) in el.elements().enumerate() {
                    match read_outline_child(child, format, &at, &style)? {
                        OutlineItem::Shape(s) => layer.shapes.push(s),
                        OutlineItem::Anchor(a) => {
                            anchor_at.push(i);
                            layer.anchors.push(a);
                        }
                    }
                }
                // The writer puts format 1 anchors after the shapes, as fontTools does.
                let shapes = layer.shapes.len();
                if anchor_at.iter().enumerate().any(|(k, i)| *i != shapes + k) {
                    let items = anchor_at.iter().map(|i| Value::Number(Number::int(i64::try_from(*i).unwrap_or(i64::MAX)))).collect();
                    ufo(&mut layer.extra, "anchorAt", Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }));
                }
            }
            "lib" => {
                let mut irregular = 0;
                let value = match el.elements().collect::<Vec<_>>().as_slice() {
                    [dict] => xmlplist::read_value(dict, &style.indent, 2, &mut irregular).ok().filter(|v| v.as_dict().is_some()),
                    _ => None,
                };
                match value {
                    Some(value) => {
                        keep_tag(&mut layer.extra, "<lib", el, &Element::new("lib"));
                        let gaps = lib_gaps(el);
                        if gaps != [format!("\n{}", style.indent.repeat(2)), format!("\n{}", style.indent)] {
                            let items = gaps.iter().map(|g| s(g)).collect();
                            ufo(&mut layer.extra, "libGaps", Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }));
                        }
                        if let Some(d) = value.as_dict() {
                            let mut user = Dict::new();
                            for (k, v) in d.entries.iter().filter(|(k, _)| !TYPED_LIB.contains(&k.as_str())) {
                                user.insert(k.as_str(), v.without_spellings());
                            }
                            if !user.is_empty() {
                                layer.info.user_data = UserData(user);
                            }
                        }
                        ufo(&mut layer.extra, "lib", value);
                    }
                    // A lib that is not a well-formed property list (some old tools wrote keys
                    // without values) is kept as it is.
                    None => {
                        let mut text = String::new();
                        el.write(&mut text);
                        ufo(&mut layer.extra, "+lib", s(&text));
                    }
                }
            }
            _ => keep_element(&mut layer.extra, el),
        }
    }
    if unicode_tags.iter().any(|t| t.as_str() != Some("")) {
        ufo(&mut layer.extra, "<unicode", Value::Array(Array { items: unicode_tags, layout: Layout::Compact, ..Default::default() }));
    }
    let mut sorted = names.clone();
    sorted.sort_by_key(|n| rank(n));
    if sorted != names {
        let items = names.iter().map(|n| s(n)).collect();
        ufo(&mut layer.extra, "order", Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }));
    }
    Ok(Glif { name, unicodes, note, format, layer, style })
}

/// An element the model does not read, kept as text under `+<name>`.
fn keep_element(x: &mut Extra, el: &Element) {
    let mut text = String::new();
    el.write(&mut text);
    ufo(x, &format!("+{}", el.name()), s(&text));
}

/// The whitespace inside `<lib>` before and after its dictionary.
fn lib_gaps(el: &Element) -> [String; 2] {
    let mut gaps = [String::new(), String::new()];
    let mut after = false;
    for n in &el.children {
        match n {
            XNode::Element(_) => after = true,
            XNode::Text(t) => {
                if let Some(g) = gaps.get_mut(usize::from(after)) {
                    g.push_str(t.raw().unwrap_or(t.as_str()));
                }
            }
            _ => {}
        }
    }
    gaps
}

fn plist_number(f: f64) -> Number {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        // In range, so the conversion is exact.
        #[allow(clippy::cast_possible_truncation)]
        Number::int(f as i64)
    } else {
        Number::float(f).unwrap_or_else(|| Number::int(0))
    }
}

enum OutlineItem {
    Shape(Shape),
    /// A format 1 anchor: a contour of one named `move` point.
    Anchor(Anchor),
}

fn raw_shape(el: &Element) -> Shape {
    let mut text = String::new();
    el.write(&mut text);
    Shape::Raw(RawShape { family: Family::Ufo, value: s(&text) })
}

fn read_outline_child(el: &Element, format: u32, at: &str, style: &Style) -> Result<OutlineItem> {
    match el.name() {
        "contour" => {
            let points: Vec<&Element> = el.elements().collect();
            if format == 1
                && let [p] = points.as_slice()
                && p.attr("type") == Some("move")
                && let Some(name) = p.attr("name")
                && p.attrs().len() == 4
            {
                let mut a = Anchor::new(name, required(p, "x", at)?, required(p, "y", at)?);
                if let Some(t) = tag_if_different(p, &anchor_point_el(&a, style)) {
                    ufo(&mut a.extra, "<point", s(&t));
                }
                return Ok(OutlineItem::Anchor(a));
            }
            Ok(OutlineItem::Shape(read_contour(el, at, style)?.map_or_else(|| raw_shape(el), Shape::Path)))
        }
        "component" => {
            let base = el.attr("base").ok_or_else(|| bad(at, "a component without `base`"))?;
            let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            for (slot, (name, default)) in m.iter_mut().zip(TRANSFORM) {
                *slot = number(el, name, at)?.unwrap_or(default);
            }
            let mut c = Component { glyph: base.to_string(), transform: Transform::Matrix(m), extra: Extra::new() };
            keep_other_attrs(&mut c.extra, el, &["base", "xScale", "xyScale", "yxScale", "yScale", "xOffset", "yOffset"]);
            if let Some(t) = tag_if_different(el, &component_el(&c, style)) {
                ufo(&mut c.extra, "<", s(&t));
            }
            Ok(OutlineItem::Shape(Shape::Component(c)))
        }
        _ => Ok(OutlineItem::Shape(raw_shape(el))),
    }
}

/// A contour as a path, or `None` when the model cannot hold it (it is then kept raw).
fn read_contour(el: &Element, at: &str, style: &Style) -> Result<Option<Path>> {
    if el.has_text() || el.elements().any(|p| p.name() != "point") {
        return Ok(None);
    }
    let mut path = Path::new(true, Vec::new());
    for (i, p) in el.elements().enumerate() {
        let (x, y) = (required(p, "x", at)?, required(p, "y", at)?);
        let kind = match p.attr("type") {
            None | Some("offcurve") => NodeKind::OffCurve,
            Some("line") => NodeKind::Line,
            Some("curve") => NodeKind::Curve,
            Some("qcurve") => NodeKind::QCurve,
            Some("move") if i == 0 => {
                path.closed = false;
                NodeKind::Line
            }
            Some(_) => return Ok(None),
        };
        let smooth = match p.attr("smooth") {
            None | Some("no") => false,
            Some("yes") => true,
            Some(_) => return Ok(None),
        };
        let mut node = Node::new(x, y, kind);
        node.smooth = smooth;
        keep_other_attrs(&mut node.extra, p, &["x", "y", "type", "smooth"]);
        if let Some(t) = tag_if_different(p, &point_el(&node, i == 0 && !path.closed, style)) {
            ufo(&mut node.extra, "<", s(&t));
        }
        path.nodes.push(node);
    }
    // The model's node lists are stricter than GLIF's.
    if path.to_bez().is_err() || path.nodes.iter().any(|n| n.kind == NodeKind::OffCurve && n.smooth) {
        return Ok(None);
    }
    keep_other_attrs(&mut path.extra, el, &[]);
    if let Some(t) = tag_if_different(el, &contour_el(&path)) {
        ufo(&mut path.extra, "<", s(&t));
    }
    if path.nodes.is_empty() {
        let inside = el.text();
        if inside != "\n  " {
            ufo(&mut path.extra, "text", s(&inside));
        }
    }
    Ok(Some(path))
}

fn read_anchor(el: &Element, at: &str, style: &Style) -> Result<Anchor> {
    let mut a = Anchor::new(el.attr("name").unwrap_or(""), required(el, "x", at)?, required(el, "y", at)?);
    if el.attr("name").is_none() {
        ufo(&mut a.extra, "noName", Value::Bool(true));
    }
    keep_other_attrs(&mut a.extra, el, &["x", "y", "name"]);
    if let Some(t) = tag_if_different(el, &anchor_el(&a, style)) {
        ufo(&mut a.extra, "<", s(&t));
    }
    Ok(a)
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// What a GLIF file holds besides the layer.
#[derive(Clone, Copy, Debug)]
pub struct Head<'a> {
    pub name: &'a str,
    pub unicodes: &'a [u32],
    /// The note to write (empty: none).
    pub note: &'a str,
    pub mark_color: MarkColor<'a>,
    /// The lib's `public.verticalOrigin`: `None` leaves the lib as it is, `Some(None)` removes
    /// the key, `Some(Some(y))` sets it (a kept value equal to `y` stays as written).
    pub vertical_origin: Option<Option<f64>>,
    /// The advance height.
    pub height: Option<f64>,
}

impl<'a> Head<'a> {
    /// A file that holds what was read: the note as read, the lib as it is.
    pub fn of(g: &'a Glif) -> Self {
        let kept = g.layer.extra.get(Family::Ufo, "height").and_then(Value::as_num).map(|n| n.as_f64());
        Head {
            name: &g.name,
            unicodes: &g.unicodes,
            note: g.note.as_deref().unwrap_or(""),
            mark_color: MarkColor::Keep,
            vertical_origin: None,
            height: g.layer.info.vert_width.or(kept),
        }
    }
}

/// Writes a layer as a GLIF file of `format` (1 or 2).
pub fn write(head: Head<'_>, layer: &Layer, format: u32, style: &Style) -> Result<String> {
    let Head { name, unicodes, note, mark_color, vertical_origin, height } = head;
    let x = &layer.extra;
    let mut root = glyph_el(name, format, x);
    apply_tag(&mut root, x, "<glyph");
    let mut children: Vec<Element> = Vec::new();

    match advance_el(layer.width, height, style) {
        Some(mut adv) => {
            apply_tag(&mut adv, x, "<advance");
            children.push(adv);
        }
        None => {
            // A zero advance the file wrote explicitly.
            if let Some(mut adv) = hint(x, "<advance").and_then(|h| xml::parse_start_tag(h).ok())
                && adv.attrs().iter().all(|a| a.value.trim().parse::<f64>().is_ok_and(|v| v == 0.0))
            {
                let src = adv.start_tag_text();
                adv.set_start_source(hint(x, "<advance").unwrap_or(&src));
                children.push(adv);
            }
        }
    }
    let unicode_tags = x.get(Family::Ufo, "<unicode").and_then(Value::as_array);
    let mut seen = Vec::new();
    for (i, u) in unicodes.iter().enumerate() {
        if seen.contains(u) {
            continue;
        }
        seen.push(*u);
        let mut el = unicode_el(*u);
        apply_source(&mut el, unicode_tags.and_then(|t| t.items.get(i)).and_then(Value::as_str));
        children.push(el);
    }
    // The note: as read while it says the same, else as fontTools writes it.
    let kept_note = hint(x, "noteXml")
        .map(|t| xml::parse_element(t).map_err(|err| UfoError::Syntax { file: format!("glyph `{name}`, kept <note>"), error: err }))
        .transpose()?;
    match kept_note {
        Some(el) if read_note(&el.text()).as_deref().unwrap_or("") == note => children.push(el),
        _ if !note.is_empty() => children.push(note_el(note)),
        _ => {}
    }
    if format >= 2 {
        if let Some(i) = &layer.info.image {
            let mut el = image_el(i, style).ok_or_else(|| bad(name, "an image without a file name or with a transform that is not a number"))?;
            apply_tag(&mut el, &i.extra, "<");
            children.push(el);
        }
        for g in &layer.info.guides {
            let mut el = guideline_el(g, style).ok_or_else(|| bad(name, "a guide value is not a number"))?;
            apply_tag(&mut el, &g.extra, "<");
            children.push(el);
        }
    }
    // Elements the model does not read, kept as text.
    for e in x.of(Family::Ufo) {
        if let (Some(el_name), Some(text)) = (e.key.as_str().strip_prefix('+'), e.value.as_str()) {
            // Written as read: it sits at the depth it was read at.
            let el = xml::parse_element(text).map_err(|err| UfoError::Syntax { file: format!("glyph `{name}`, kept <{el_name}>"), error: err })?;
            children.push(el);
        }
    }
    let mut outline = Element::new("outline");
    // A shape another format's reader kept as written has no GLIF form; the engine's loss report
    // lists it.
    for shape in layer.shapes.iter().filter(|s| !matches!(s, Shape::Raw(r) if r.family != Family::Ufo)) {
        outline.children.push(XNode::Element(write_shape(shape, style, name)?));
    }
    let anchor_at: Vec<usize> = x
        .get(Family::Ufo, "anchorAt")
        .and_then(Value::as_array)
        .map(|a| a.items.iter().filter_map(|v| v.as_num().and_then(|n| usize::try_from(n.as_f64() as i64).ok())).collect())
        .filter(|v: &Vec<usize>| v.len() == layer.anchors.len())
        .unwrap_or_default();
    for (k, a) in layer.anchors.iter().enumerate() {
        if format == 1 {
            // UFO 2 has no anchor element: a contour of one named move point stands in.
            let mut p = anchor_point_el(a, style);
            apply_tag(&mut p, &a.extra, "<point");
            let contour = XNode::Element(Element::new("contour").with_child(p));
            match anchor_at.get(k) {
                Some(&i) if i <= outline.children.len() => outline.children.insert(i, contour),
                _ => outline.children.push(contour),
            }
        } else {
            let mut el = anchor_el(a, style);
            apply_tag(&mut el, &a.extra, "<");
            children.push(el);
        }
    }
    let had_outline = x.get(Family::Ufo, "outline");
    if !outline.children.is_empty() || had_outline.is_some() {
        if outline.children.is_empty() {
            let inside = had_outline.and_then(Value::as_str).unwrap_or("\n  ");
            outline.children.push(XNode::Text(xml::Text::new(inside)));
        }
        apply_tag(&mut outline, x, "<outline");
        children.push(outline);
    }
    let mut lib = x.get(Family::Ufo, "lib").and_then(Value::as_dict).cloned().unwrap_or_default();
    if x.get(Family::Ufo, "+lib").is_none() {
        patch_mark_color(&mut lib, mark_color);
        match vertical_origin {
            None => {}
            Some(None) => crate::ufo::patch(&mut lib, VERTICAL_ORIGIN, None),
            Some(Some(y)) => crate::ufo::patch(&mut lib, VERTICAL_ORIGIN, Some(Value::Number(plist_number(y)))),
        }
        patch_user_data(&mut lib, &layer.info.user_data);
    }
    if !lib.is_empty() {
        let lib = Value::Dict(lib);
        let mut text = String::new();
        xmlplist::write_value(&mut text, &lib, &style.indent, 2);
        let dict = xml::parse_element(&text).map_err(|e| UfoError::Syntax { file: format!("glyph `{name}`, lib"), error: e })?;
        let mut el = Element::new("lib").with_child(dict);
        if let Some(gaps) = x.get(Family::Ufo, "libGaps").and_then(Value::as_array)
            && let [Some(before), Some(after)] = [gaps.items.first().and_then(Value::as_str), gaps.items.get(1).and_then(Value::as_str)]
        {
            el.children.insert(0, XNode::Text(xml::Text::new(before)));
            el.children.push(XNode::Text(xml::Text::new(after)));
        }
        apply_tag(&mut el, x, "<lib");
        children.push(el);
    }
    order_children(&mut children, x);
    root.children = children.into_iter().map(XNode::Element).collect();
    let mut doc = xml::Document::new(&style.declaration, root);
    if !style.final_newline {
        doc.epilog.clear();
    }
    if style.declaration.is_empty() {
        doc.prolog.clear();
    }
    Ok(doc.write_pretty(&style.indent))
}

/// Puts the children in the writer's order, or in the kept order when it lists the same
/// elements.
fn order_children(children: &mut Vec<Element>, x: &Extra) {
    children.sort_by_key(|e| rank(e.name()));
    let Some(order) = x.get(Family::Ufo, "order").and_then(Value::as_array) else { return };
    let wanted: Vec<&str> = order.items.iter().filter_map(Value::as_str).collect();
    let mut have: Vec<&str> = children.iter().map(Element::name).collect();
    let mut sorted_wanted = wanted.clone();
    have.sort_unstable();
    sorted_wanted.sort_unstable();
    if have != sorted_wanted {
        return;
    }
    let mut pool: Vec<Option<Element>> = std::mem::take(children).into_iter().map(Some).collect();
    for w in wanted {
        if let Some(slot) = pool.iter_mut().find(|p| p.as_ref().is_some_and(|e| e.name() == w))
            && let Some(item) = slot.take()
        {
            children.push(item);
        }
    }
}

fn write_shape(shape: &Shape, style: &Style, glyph: &str) -> Result<Element> {
    match shape {
        Shape::Path(p) => {
            let mut contour = contour_el(p);
            for (i, n) in p.nodes.iter().enumerate() {
                let mut el = point_el(n, i == 0 && !p.closed, style);
                apply_tag(&mut el, &n.extra, "<");
                contour.children.push(XNode::Element(el));
            }
            if p.nodes.is_empty() {
                contour.children.push(XNode::Text(xml::Text::new(hint(&p.extra, "text").unwrap_or("\n  "))));
            }
            apply_tag(&mut contour, &p.extra, "<");
            Ok(contour)
        }
        Shape::Component(c) => {
            let mut el = component_el(c, style);
            apply_tag(&mut el, &c.extra, "<");
            Ok(el)
        }
        Shape::Raw(r) => {
            let text = match (&r.family, &r.value) {
                (Family::Ufo, Value::String(t)) => t.as_str(),
                _ => return Err(UfoError::Unsupported(format!("glyph `{glyph}`: a shape from another format cannot be written to a UFO yet"))),
            };
            xml::parse_element(text).map_err(|e| UfoError::Syntax { file: format!("glyph `{glyph}`, kept shape"), error: e })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "<?xml version='1.0' encoding='UTF-8'?>
<glyph name=\"a\" format=\"2\">
  <advance width=\"600\"/>
  <unicode hex=\"0061\"/>
  <anchor x=\"300\" y=\"500\" name=\"top\"/>
  <outline>
    <contour>
      <point x=\"447\" y=\"434\" type=\"line\"/>
      <point x=\"250\" y=\"520\"/>
      <point x=\"300\" y=\"520\"/>
      <point x=\"214\" y=\"504\" type=\"curve\" smooth=\"yes\" name=\"p\"/>
    </contour>
    <component base=\"acute\" xOffset=\"100\"/>
  </outline>
  <lib>
    <dict>
      <key>public.markColor</key>
      <string>1,0,0,1</string>
    </dict>
  </lib>
</glyph>
";

    fn round_trip(text: &str) -> String {
        let g = read(text, "test.glif", "m", "m").unwrap();
        write(Head::of(&g), &g.layer, g.format, &g.style).unwrap()
    }

    fn hints(g: &Glif) -> usize {
        let mut n = g.layer.extra.iter().filter(|e| e.key.as_str().starts_with('<')).count();
        for s in &g.layer.shapes {
            if let Shape::Path(p) = s {
                n += p.nodes.iter().filter(|n| n.extra.get(Family::Ufo, "<").is_some()).count();
            }
        }
        n
    }

    #[test]
    fn a_fonttools_glif_round_trips_without_hints() {
        let g = read(A, "a.glif", "m01", "m01").unwrap();
        assert_eq!(g.unicodes, [0x61]);
        assert_eq!(g.layer.width, 600.0);
        assert_eq!(g.layer.shapes.len(), 2);
        assert_eq!(g.layer.anchors.len(), 1);
        let Shape::Path(p) = &g.layer.shapes[0] else { panic!("{:?}", g.layer.shapes[0]) };
        assert!(p.closed);
        assert_eq!(p.nodes[3].extra.get(Family::Ufo, "@name").and_then(Value::as_str), Some("p"));
        assert_eq!(hints(&g), 0, "nothing in a fontTools file needs a kept tag");
        assert_eq!(round_trip(A), A);
    }

    #[test]
    fn spellings_survive_until_the_value_changes() {
        let text = A.replace("x=\"447\"", "x=\"447.0\"").replace("<point x=\"250\" y=\"520\"/>", "<point y='520' x=\"250\"/>");
        assert_eq!(round_trip(&text), text);
        let mut g = read(&text, "a.glif", "m", "m").unwrap();
        assert_eq!(hints(&g), 2);
        let Shape::Path(p) = &mut g.layer.shapes[0] else { panic!() };
        p.nodes[0].x = 448.0;
        let out = write(Head::of(&g), &g.layer, 2, &g.style).unwrap();
        assert!(out.contains("<point x=\"448\" y=\"434\" type=\"line\"/>"), "{out}");
        assert!(out.contains("<point y='520' x=\"250\"/>"), "an untouched point keeps its spelling");
    }

    #[test]
    fn float_styled_files_keep_their_style() {
        let mut text = A.to_string();
        for n in ["600", "300", "500", "447", "434", "250", "520", "214", "504", "100"] {
            text = text.replace(&format!("\"{n}\""), &format!("\"{n}.0\""));
        }
        let g = read(&text, "a.glif", "m", "m").unwrap();
        assert!(g.style.float_numbers);
        assert_eq!(hints(&g), 0);
        assert_eq!(round_trip(&text), text);
    }

    #[test]
    fn odd_elements_and_contours_are_kept() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<glyph name=\"b\" format=\"2\">
\t<note>
hello
</note>
\t<guideline x=\"10\" angle=\"90\"/>
\t<outline>
\t\t<contour>
\t\t\t<point x=\"0\" y=\"0\" type=\"curve\"/>
\t\t\t<point x=\"10\" y=\"10\"/>
\t\t\t<point x=\"20\" y=\"0\" type=\"curve\"/>
\t\t</contour>
\t\t<contour>
\t\t\t<point x=\"0\" y=\"0\" type=\"move\"/>
\t\t\t<point x=\"5\" y=\"5\" type=\"line\"/>
\t\t</contour>
\t</outline>
\t<future attr=\"1\"/>
</glyph>
";
        let g = read(text, "b.glif", "m", "m").unwrap();
        assert!(matches!(g.layer.shapes[0], Shape::Raw(_)), "a curve after one off-curve point is kept raw");
        assert!(matches!(&g.layer.shapes[1], Shape::Path(p) if !p.closed));
        assert_eq!(g.style.indent, "\t");
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn format_1_anchors_are_move_contours() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<glyph name=\"a\" format=\"1\">
  <advance width=\"500\"/>
  <outline>
    <contour>
      <point x=\"250\" y=\"700\" type=\"move\" name=\"top\"/>
    </contour>
  </outline>
</glyph>
";
        let g = read(text, "a.glif", "m", "m").unwrap();
        assert_eq!(g.layer.anchors.len(), 1);
        assert!(g.layer.shapes.is_empty());
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn empty_and_minimal_glyphs() {
        let empty = "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\".notdef\" format=\"2\">\n  <advance width=\"600\"/>\n  <outline>\n  </outline>\n</glyph>\n";
        assert_eq!(round_trip(empty), empty);
        let bare = "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"plus\" format=\"2\">\n  <advance width=\"557\"/>\n  <unicode hex=\"002B\"/>\n</glyph>\n";
        assert_eq!(round_trip(bare), bare);
        let old_order = "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"plus\" format=\"2\">\n  <unicode hex=\"002b\"/>\n  <advance width=\"557\"/>\n</glyph>\n";
        assert_eq!(round_trip(old_order), old_order);
    }

    #[test]
    fn malformed_glifs_are_errors() {
        for bad in [
            "<glyph/>",
            "<glyph name=\"a\" format=\"3\"/>",
            "<nope/>",
            "<glyph name=\"a\" format=\"2\"><advance width=\"x\"/></glyph>",
            "<glyph name=\"a\" format=\"2\"><unicode hex=\"zz\"/></glyph>",
            "<glyph name=\"a\" format=\"2\"><outline><contour><point y=\"1\"/></contour></outline></glyph>",
            "<glyph name=\"a\" format=\"2\"><outline><component/></outline></glyph>",
            "<glyph name=\"a\" format=\"2\"><advance width=\"inf\"/></glyph>",
        ] {
            assert!(read(bad, "bad.glif", "m", "m").is_err(), "{bad}");
        }
        // A lib that is not a dictionary is kept as it is rather than refused.
        let g = read("<glyph name=\"a\" format=\"2\"><lib><string>x</string></lib></glyph>", "lib.glif", "m", "m").unwrap();
        assert!(g.layer.extra.get(Family::Ufo, "+lib").is_some());
    }

    #[test]
    fn guidelines_images_heights_and_libs_are_layer_fields() {
        let text = "<?xml version='1.0' encoding='UTF-8'?>
<glyph name=\"a\" format=\"2\">
  <advance height=\"1000\" width=\"600\"/>
  <image fileName=\"a.png\" xScale=\"0.5\" yScale=\"0.5\" xOffset=\"10\" color=\"1,0,0,1\"/>
  <guideline x=\"10\" name=\"stem [locked]\"/>
  <guideline y=\"500\" color=\"0,0,1,1\" identifier=\"g1\"/>
  <guideline x=\"1\" y=\"2\" angle=\"45\"/>
  <lib>
    <dict>
      <key>a.key</key>
      <real>1.0</real>
      <key>public.markColor</key>
      <string>1,0,0,1</string>
    </dict>
  </lib>
</glyph>
";
        let g = read(text, "a.glif", "m", "m").unwrap();
        let i = &*g.layer.info;
        assert_eq!(i.vert_width, Some(1000.0));
        assert_eq!(i.guides.len(), 3);
        assert_eq!((i.guides[0].x, i.guides[0].angle, i.guides[0].name.as_deref(), i.guides[0].locked), (10.0, 90.0, Some("stem"), true));
        assert_eq!((i.guides[1].y, i.guides[1].color.as_deref(), i.guides[1].identifier.as_deref()), (500.0, Some("0,0,1,1"), Some("g1")));
        let image = i.image.as_ref().unwrap();
        assert_eq!(image.path.as_deref(), Some("a.png"));
        assert_eq!(image.transform, Transform::Matrix([0.5, 0.0, 0.0, 0.5, 10.0, 0.0]));
        assert_eq!(i.user_data.0.len(), 1, "the mark colour is the glyph's, not user data");
        assert!(i.user_data.get("a.key").is_some());
        assert_eq!(round_trip(text), text, "written back as read");

        // Edits are written; the kept spellings stay where the value does.
        let mut l = g.layer.clone();
        l.info.guides.remove(0);
        l.info.vert_width = Some(900.0);
        l.info.user_data.0.insert("b.key", Value::Bool(true));
        let head = Head { height: l.info.vert_width, ..Head::of(&g) };
        let out = write(head, &l, 2, &g.style).unwrap();
        assert!(out.contains("<advance height=\"900\" width=\"600\"/>"), "{out}");
        assert!(!out.contains("stem"));
        assert!(out.contains("<real>1.0</real>") && out.contains("<key>b.key</key>\n      <true/>"), "{out}");
        l.info.user_data = UserData::default();
        l.info.image = None;
        let out = write(Head::of(&g), &l, 2, &g.style).unwrap();
        assert!(!out.contains("a.key") && !out.contains("<image") && out.contains("public.markColor"), "{out}");
        // Format 1 has no guidelines or images.
        let out = write(Head::of(&g), &g.layer, 1, &g.style).unwrap();
        assert!(!out.contains("<guideline") && !out.contains("<image"), "{out}");
    }

    #[test]
    fn values_beyond_the_models_limits_stay_as_written() {
        let text = "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"a\" format=\"2\">\n  <advance height=\"9000000\" width=\"600\"/>\n  <guideline x=\"2000000\"/>\n</glyph>\n";
        let g = read(text, "a.glif", "m", "m").unwrap();
        assert!(g.layer.info.is_default(), "{:?}", *g.layer.info);
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn a_shape_another_format_kept_is_left_out() {
        let mut l = Layer::new("m", 500.0);
        l.shapes.push(Shape::Raw(RawShape { family: Family::Glyphs, value: s("{closed = 1;}") }));
        let head = Head { name: "a", unicodes: &[], note: "", mark_color: MarkColor::Keep, vertical_origin: None, height: None };
        let out = write(head, &l, 2, &Style::fonttools()).unwrap();
        assert!(!out.contains("<outline"), "{out}");
    }

    #[test]
    fn guidelines_and_images_that_would_change_stay_as_written() {
        // glyphsLib would write `y` 0 alone as a horizontal line; an unknown attribute has no
        // field; an image needs a file name.
        let text = "<?xml version='1.0' encoding='UTF-8'?>
<glyph name=\"a\" format=\"2\">
  <image xScale=\"2\"/>
  <guideline x=\"0\" y=\"0\" angle=\"90\"/>
  <guideline x=\"5\" odd=\"1\"/>
</glyph>
";
        let g = read(text, "a.glif", "m", "m").unwrap();
        assert!(g.layer.info.guides.is_empty() && g.layer.info.image.is_none());
        assert_eq!(g.layer.extra.len(), 3, "kept as elements");
        assert_eq!(round_trip(text), text);
        // A guideline's own spelling stays while it means the same.
        let spelled = "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"a\" format=\"2\">\n  <guideline name=\"n\" x=\"10.0\"/>\n</glyph>\n";
        let g = read(spelled, "a.glif", "m", "m").unwrap();
        assert_eq!(g.layer.info.guides.len(), 1);
        assert_eq!(round_trip(spelled), spelled);
    }
}
