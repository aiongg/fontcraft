//! Paths, nodes, components, anchors and version 4 image shapes, as format versions 3 and 4 write
//! them, and the parts version 2 shares ([`crate::v2`] writes nodes and transforms its own way).
//!
//! | Object | Model field | Versions 3 and 4 | Version 2 |
//! |---|---|---|---|
//! | node | `kind`, `smooth`, `tangent`, `orientation`, `locked` | the flags `l`/`c`/`q`/`o`, then `s` or `t`, then `R` or `C`, then `X` | `LINE`, `CURVE`, `QCURVE`, `OFFCURVE`, ` SMOOTH` |
//! | node | `info.name`, `info.user_data` | the fourth item, `{name, userData}` | a dictionary after the type, `{name = …;}` |
//! | node | `info.hoi` | version 4: `hoi` in the fourth item ([`fontcraft_model::hoi`]); version 3: none | none |
//! | path | `info.locked`, `info.attrs` | `locked = 1`, `attr` | none |
//! | component | `info.alignment` | `alignment` (−1, 1, 3) | the same |
//! | component | `info.anchor`, `info.smart_values`, `info.master_id`, `info.locked`, `info.orientation` | `anchor`, `piece`, `masterId`, `locked = 1`, `orientation` (1, 2) | `anchor`, `piece`, `locked = 1` |
//! | component | `info.attrs`, `info.user_data` | `attr`; `userData` (version 4: `attr.userData`) | `userData` |
//! | anchor | `info.locked`, `info.orientation`, `info.user_data` | `locked = 1`, `orientation` (`center`, `right`); `userData` (version 4: `attr.userData`) | `userData` |
//! | image (version 4) | [`Shape::Image`] | `{imagePath, pos, scale, angle, slant, crop, alpha, locked}` | none |
//!
//! Drawing attributes ([`ShapeAttrs`]): `strokeWidth`, `strokeHeight` and `strokePos` (a number,
//! or text), `lineCapStart`, `lineCapEnd`, `lineJoin`, `fill`, `fillColor` and `strokeColor`
//! (red, green, blue, alpha), `color` (a label), `mask`, `reversePaths`, `hidden`, `opacity`.
//! Gradients, shadows, compositing, shape-group membership and unknown keys stay as stored values.
//!
//! As elsewhere, a key becomes a field only when writing the field back gives the same text in the
//! file's version; a key at its default (`alignment = 0`, `locked = 0`) stays in `Extra` and is
//! written from there. Shape groups (version 4) stay [`Shape::Raw`].
//!
//! Written from the GlyphsSDK format documents (Apache-2.0).

use fontcraft_model::{
    Alignment, Anchor, AnchorInfo, Component, ComponentInfo, Dimension, Family, Hoi, MAX_PART_PROPERTIES, Node, NodeInfo, NodeKind, Orientation,
    Path, PathInfo, Rare, Shape, ShapeAttrs, SmartValue, Transform, UserData,
};
use fontcraft_plist::{Array, Dict, Layout, Num, Str, Value};

use self::paint::Form;
use crate::layer::{number, same_text, take_flag, take_same, take_text};
use crate::{GlyphsError, Result, as_items, as_pair, bad, finish, int, into_extra, list, num, pair, raw, string, take};

type Entries = Vec<(Str, Value)>;

// ── nodes ──────────────────────────────────────────────────────────────────────────────────────

/// What a node's flags say, after its type.
pub(crate) struct Flags<'a> {
    pub smooth: bool,
    pub tangent: bool,
    pub orientation: Orientation,
    pub locked: bool,
    /// The flags after the join as written, when the writer would spell them otherwise; empty
    /// when it would not.
    pub rest: &'a str,
}

/// The flags after the type: `s` (or from version 4 `t`), then `R` or `C`, then `X`. `R`, `C`
/// and `X` are read in any order; flags the writer would not spell the same way (another order,
/// a repeated or unknown letter) are also kept as written in `rest`.
pub(crate) fn read_flags(s: &str, version: u32) -> Flags<'_> {
    let (smooth, tangent, after) = match s.as_bytes().first() {
        Some(b's') => (true, false, s.get(1..).unwrap_or("")),
        Some(b't') if version >= 4 => (true, true, s.get(1..).unwrap_or("")),
        _ => (false, false, s),
    };
    let (orientation, locked) = orientation_and_lock(after);
    let mut written = String::new();
    push_orientation_and_lock(&mut written, orientation, locked);
    let rest = if written == after { "" } else { after };
    Flags { smooth, tangent, orientation, locked, rest }
}

/// The orientation and lock that `R`, `C` and `X` give, in any order; of `R` and `C` the first
/// counts.
fn orientation_and_lock(flags: &str) -> (Orientation, bool) {
    let orientation = flags
        .chars()
        .find_map(|c| match c {
            'R' => Some(Orientation::Right),
            'C' => Some(Orientation::Center),
            _ => None,
        })
        .unwrap_or(Orientation::Left);
    (orientation, flags.contains('X'))
}

fn push_orientation_and_lock(out: &mut String, orientation: Orientation, locked: bool) {
    match orientation {
        Orientation::Left => {}
        Orientation::Right => out.push('R'),
        Orientation::Center => out.push('C'),
    }
    if locked {
        out.push('X');
    }
}

/// The flags after the type, as the writer spells them (a tangent join is `t` from version 4 on).
pub(crate) fn write_flags(out: &mut String, n: &Node, version: u32) {
    if n.smooth {
        out.push(if n.tangent && version >= 4 { 't' } else { 's' });
    }
    // Flags kept as written stand for the orientation and lock while they still give the model's.
    // Once an edit changes either, the fields are written, followed by the kept letters the
    // model does not read.
    let kept = n.info.extra.get(Family::Glyphs, "flags").and_then(Value::as_str);
    if let Some(rest) = kept
        && orientation_and_lock(rest) == (n.orientation, n.locked)
    {
        out.push_str(rest);
        return;
    }
    push_orientation_and_lock(out, n.orientation, n.locked);
    if let Some(rest) = kept {
        out.extend(rest.chars().filter(|c| !matches!(c, 'R' | 'C' | 'X')));
    }
}

/// A node's attributes (the fourth item of a node, a version 2 node's dictionary): the name, the
/// user data and (version 4) the higher-order interpolation data as fields; other keys stay under
/// `attr`. `None` when the dictionary is empty or would not be written back the same; the caller
/// then keeps it whole.
pub(crate) fn read_node_attr(v: &Value, version: u32) -> Option<NodeInfo> {
    node_attr_fields(v, version).filter(|info| write_node_attr(info, version).is_some_and(|w| same_text(&w, v, version)))
}

/// [`read_node_attr`] without the check that it writes back the same (version 2 checks the whole
/// node string instead).
pub(crate) fn node_attr_fields(v: &Value, version: u32) -> Option<NodeInfo> {
    let mut d = v.as_dict()?.clone();
    if d.is_empty() {
        return None;
    }
    let mut info = NodeInfo { name: take_text(&mut d, "name", version), ..Default::default() };
    info.user_data = crate::info::take_user_data(&mut d);
    if version >= 4 {
        info.hoi = take_same(&mut d, "hoi", version, Hoi::from_value, Hoi::to_value).unwrap_or_default();
    }
    if !d.is_empty() {
        info.extra.push(Family::Glyphs, "attr", Value::Dict(d));
    }
    Some(info)
}

/// The node's attribute dictionary, or `None` when it has nothing to write. Versions before 4
/// leave out the higher-order interpolation data ([`losses`] counts it).
pub(crate) fn write_node_attr(info: &NodeInfo, version: u32) -> Option<Value> {
    let kept = info.extra.get(Family::Glyphs, "attr");
    let mut e: Entries = Vec::new();
    if let Some(n) = &info.name {
        e.push(("name".into(), string(n)));
    }
    crate::info::push_user_data(&mut e, &info.user_data);
    if version >= 4
        && !info.hoi.is_empty()
        && let Some(hoi) = info.hoi.to_value()
    {
        e.push(("hoi".into(), hoi));
    }
    if e.is_empty() {
        // Nothing typed: the dictionary as the file had it.
        return kept.cloned();
    }
    let rest = kept.and_then(Value::as_dict).map(|d| into_extra(d.clone())).unwrap_or_default();
    Some(Value::Dict(finish(e, &rest)))
}

/// A node tuple `(x, y, flags[, attr])`.
fn read_node(v: &Value, version: u32) -> Option<Node> {
    let items = &v.as_array()?.items;
    let (x, y, flags, attr) = match items.as_slice() {
        [x, y, f] => (x, y, f, None),
        [x, y, f, a] => (x, y, f, Some(a)),
        _ => return None,
    };
    let flags = flags.as_str()?;
    let kind = match flags.as_bytes().first()? {
        b'l' => NodeKind::Line,
        b'c' => NodeKind::Curve,
        b'q' => NodeKind::QCurve,
        b'o' => NodeKind::OffCurve,
        _ => return None,
    };
    let f = read_flags(flags.get(1..).unwrap_or(""), version);
    let mut node = Node::new(x.as_num()?.as_f64(), y.as_num()?.as_f64(), kind);
    (node.smooth, node.tangent, node.orientation, node.locked) = (f.smooth, f.tangent, f.orientation, f.locked);
    let typed = attr.and_then(|a| read_node_attr(a, version));
    if !f.rest.is_empty() || attr.is_some() {
        let mut info = typed.unwrap_or_default();
        if !f.rest.is_empty() {
            info.extra.push(Family::Glyphs, "flags", Value::String(f.rest.into()));
        }
        let untyped = info.name.is_none() && info.user_data.is_empty() && info.hoi.is_empty() && info.extra.get(Family::Glyphs, "attr").is_none();
        if let (Some(a), true) = (attr, untyped) {
            info.extra.push(Family::Glyphs, "attr", a.clone());
        }
        node.info = Rare::new(info);
    }
    Some(node)
}

fn write_node(n: &Node, at: &str, version: u32) -> Result<Value> {
    let mut flags = String::from(match n.kind {
        NodeKind::Line => "l",
        NodeKind::Curve => "c",
        NodeKind::QCurve => "q",
        NodeKind::OffCurve => "o",
    });
    write_flags(&mut flags, n, version);
    let mut items = vec![num(n.x, at)?, num(n.y, at)?, string(&flags)];
    if let Some(attr) = write_node_attr(&n.info, version) {
        items.push(attr);
    }
    Ok(Value::Array(Array { items, layout: Layout::Compact, ..Default::default() }))
}

// ── drawing attributes ─────────────────────────────────────────────────────────────────────────

fn read_dimension(v: &Value) -> Option<Dimension> {
    match v {
        Value::String(s) => Some(Dimension::Expression(s.as_str().to_string())),
        _ => number(v).map(Dimension::Number),
    }
}

fn write_dimension(d: &Dimension) -> Option<Value> {
    match d {
        Dimension::Number(f) => num(*f, "attr").ok(),
        Dimension::Expression(s) => Some(string(s)),
    }
}

fn read_byte(v: &Value) -> Option<u8> {
    match v.as_num()? {
        Num::Int(i) => u8::try_from(i).ok(),
        Num::Float(_) => None,
    }
}

/// A shape's `attr` dictionary: the drawing attributes, and with `user_data` (version 4 components
/// and anchors) the user data. `None` when the dictionary would not be written back the same.
fn read_attrs(v: &Value, version: u32, user_data: bool) -> Option<(ShapeAttrs, UserData)> {
    let mut d = v.as_dict()?.clone();
    if d.is_empty() {
        return None;
    }
    let gray = paint::gray_colors(&d);
    let f = Form { version, gray: &gray };
    let mut a = ShapeAttrs::default();
    let dim = |d: &mut Dict, k: &str| take_same(d, k, version, read_dimension, write_dimension);
    a.stroke_width = dim(&mut d, "strokeWidth");
    a.stroke_height = dim(&mut d, "strokeHeight");
    a.stroke_pos = take_same(&mut d, "strokePos", version, |v| paint::read_stroke_pos(v, version), |p| paint::write_stroke_pos(p, version));
    let byte = |d: &mut Dict, k: &str| take_same(d, k, version, read_byte, |b| Some(int(*b)));
    a.line_cap_start = byte(&mut d, "lineCapStart");
    a.line_cap_end = byte(&mut d, "lineCapEnd");
    a.line_join = byte(&mut d, "lineJoin");
    a.mask = byte(&mut d, "mask");
    a.fill = take_flag(&mut d, "fill", version);
    a.reverse_paths = take_flag(&mut d, "reversePaths", version);
    a.hidden = take_flag(&mut d, "hidden", version);
    let color = |d: &mut Dict, k: &str| take_same(d, k, version, |v| paint::read_color(v, version), |c| paint::write_color(c, f, k));
    a.fill_color = color(&mut d, "fillColor");
    a.stroke_color = color(&mut d, "strokeColor");
    a.gradient = take_same(&mut d, "gradient", version, |v| paint::read_gradient(v, version), |g| paint::write_gradient(g, f));
    let shadow = |d: &mut Dict, k: &str| take_same(d, k, version, |v| paint::read_shadow(v, version), |s| paint::write_shadow(s, f, k));
    a.shadow = shadow(&mut d, "shadow");
    a.shadow_in = shadow(&mut d, "shadowIn");
    a.color = take_same(&mut d, "color", version, |v| crate::glyph::read_color(v, version), |c| Some(crate::glyph::write_color(c, version)));
    a.opacity = take_same(&mut d, "opacity", version, number, |f| num(*f, "opacity").ok());
    let user = if user_data { crate::info::take_user_data(&mut d) } else { UserData::default() };
    a.extra = into_extra(d);
    if !gray.is_empty() {
        a.extra.push(Family::Native, paint::GRAY_KEY, paint::gray_value(&gray));
    }
    write_attrs(&a, &user, version).filter(|w| same_text(w, v, version)).map(|_| (a, user))
}

/// The `attr` dictionary, or `None` when there is nothing to write.
fn write_attrs(a: &ShapeAttrs, user: &UserData, version: u32) -> Option<Value> {
    let mut e: Entries = Vec::new();
    let mut dim = |k: &str, d: &Option<Dimension>| -> Option<()> {
        if let Some(d) = d {
            e.push((k.into(), write_dimension(d)?));
        }
        Some(())
    };
    dim("strokeWidth", &a.stroke_width)?;
    dim("strokeHeight", &a.stroke_height)?;
    if let Some(p) = &a.stroke_pos {
        e.push(("strokePos".into(), paint::write_stroke_pos(p, version)?));
    }
    for (k, b) in [("lineCapStart", a.line_cap_start), ("lineCapEnd", a.line_cap_end), ("lineJoin", a.line_join), ("mask", a.mask)] {
        if let Some(b) = b {
            e.push((k.into(), int(b)));
        }
    }
    for (k, on) in [("fill", a.fill), ("reversePaths", a.reverse_paths), ("hidden", a.hidden)] {
        if on {
            e.push((k.into(), int(1)));
        }
    }
    let gray = paint::stored_gray(a.extra.get(Family::Native, paint::GRAY_KEY));
    let f = Form { version, gray: &gray };
    for (k, c) in [("fillColor", &a.fill_color), ("strokeColor", &a.stroke_color)] {
        if let Some(c) = c {
            e.push((k.into(), paint::write_color(c, f, k)?));
        }
    }
    // Version 3 has no conic gradient; `losses` reports it.
    if let Some(v) = a.gradient.as_ref().and_then(|g| paint::write_gradient(g, f)) {
        e.push(("gradient".into(), v));
    }
    for (k, s) in [("shadow", &a.shadow), ("shadowIn", &a.shadow_in)] {
        if let Some(s) = s {
            e.push((k.into(), paint::write_shadow(s, f, k)?));
        }
    }
    if let Some(c) = &a.color {
        e.push(("color".into(), crate::glyph::write_color(c, version)));
    }
    if let Some(o) = a.opacity {
        e.push(("opacity".into(), num(o, "opacity").ok()?));
    }
    crate::info::push_user_data(&mut e, user);
    if e.is_empty() && a.extra.of(Family::Glyphs).next().is_none() {
        return None;
    }
    Some(Value::Dict(finish(e, &a.extra)))
}

fn attrs_error(at: &str) -> GlyphsError {
    bad(at, "a drawing attribute is not a finite number")
}

// ── paths ──────────────────────────────────────────────────────────────────────────────────────

fn read_path(mut d: Dict, version: u32) -> Option<Shape> {
    let closed = crate::as_bool01(&take(&mut d, "closed")?, "closed").ok()?;
    let mut nodes = as_items(take(&mut d, "nodes")?, "nodes").ok()?.iter().map(|n| read_node(n, version)).collect::<Option<Vec<_>>>()?;
    // The format writes a closed path's start node last; the model keeps it first.
    if closed && !nodes.is_empty() {
        nodes.rotate_right(1);
    }
    // Most paths have no other key.
    let info = if d.is_empty() {
        Rare::default()
    } else {
        let mut info = PathInfo { locked: take_flag(&mut d, "locked", version), ..Default::default() };
        if let Some((attrs, _)) = take_same(&mut d, "attr", version, |v| read_attrs(v, version, false), |(a, u)| write_attrs(a, u, version)) {
            info.attrs = attrs;
        }
        Rare::new(info)
    };
    let path = Path { closed, nodes, info, extra: into_extra(d) };
    // A node list that does not make an outline is kept as written, not rejected with the file.
    path.to_bez().ok()?;
    Some(Shape::Path(path))
}

/// A path's lock and drawing attributes as entries (versions 3 and 4).
fn push_path_info(e: &mut Entries, i: &PathInfo, version: u32, at: &str) -> Result<()> {
    if i.locked {
        e.push(("locked".into(), int(1)));
    }
    if !i.attrs.is_empty() {
        match write_attrs(&i.attrs, &UserData::default(), version) {
            Some(v) => e.push(("attr".into(), v)),
            // Only a gradient that version 3 cannot write; `losses` reports it.
            None if only_unwritable_gradient(&i.attrs, version) => {}
            None => return Err(attrs_error(at)),
        }
    }
    Ok(())
}

fn write_path(p: &Path, at: &str, version: u32) -> Result<Value> {
    let mut nodes: Vec<Value> = p.nodes.iter().map(|n| write_node(n, at, version)).collect::<Result<_>>()?;
    if p.closed && !nodes.is_empty() {
        nodes.rotate_left(1);
    }
    let mut e = vec![("closed".into(), int(i64::from(p.closed))), ("nodes".into(), list(nodes))];
    push_path_info(&mut e, &p.info, version, at)?;
    Ok(Value::Dict(finish(e, &p.extra)))
}

// ── components ─────────────────────────────────────────────────────────────────────────────────

fn read_alignment(v: &Value) -> Option<Alignment> {
    match v.as_num()? {
        Num::Int(-1) => Some(Alignment::Disabled),
        Num::Int(1) => Some(Alignment::Forced),
        Num::Int(3) => Some(Alignment::Horizontal),
        _ => None,
    }
}

fn alignment_value(a: Alignment) -> Option<i64> {
    match a {
        Alignment::Default => None,
        Alignment::Disabled => Some(-1),
        Alignment::Forced => Some(1),
        Alignment::Horizontal => Some(3),
    }
}

fn read_smart_values(v: &Value) -> Option<Vec<SmartValue>> {
    let d = v.as_dict()?;
    if d.is_empty() || d.len() > MAX_PART_PROPERTIES {
        return None;
    }
    d.entries.iter().map(|(k, v)| Some(SmartValue { property: k.as_str().to_string(), value: number(v)? })).collect()
}

fn write_smart_values(values: &[SmartValue]) -> Option<Value> {
    let entries = values.iter().map(|s| Some((Str::new(s.property.as_str()), num(s.value, "piece").ok()?))).collect::<Option<_>>()?;
    Some(Value::Dict(Dict { entries, layout: Layout::Multiline, ..Default::default() }))
}

fn read_component_orientation(v: &Value) -> Option<Orientation> {
    match v.as_num()? {
        Num::Int(1) => Some(Orientation::Right),
        Num::Int(2) => Some(Orientation::Center),
        _ => None,
    }
}

/// A component's alignment, attachment anchor, smart values, master, lock, orientation, drawing
/// attributes and user data, taken out of its dictionary.
pub(crate) fn take_component_info(d: &mut Dict, version: u32) -> ComponentInfo {
    let mut c = ComponentInfo {
        alignment: take_same(d, "alignment", version, read_alignment, |a| alignment_value(*a).map(int)).unwrap_or_default(),
        anchor: take_text(d, "anchor", version),
        smart_values: take_same(d, "piece", version, read_smart_values, |v| write_smart_values(v)).unwrap_or_default(),
        master_id: take_text(d, "masterId", version),
        locked: take_flag(d, "locked", version),
        orientation: take_same(d, "orientation", version, read_component_orientation, |o| match o {
            Orientation::Right => Some(int(1)),
            Orientation::Center => Some(int(2)),
            Orientation::Left => None,
        })
        .unwrap_or_default(),
        ..Default::default()
    };
    if version >= 4 {
        if let Some((attrs, user)) = take_same(d, "attr", version, |v| read_attrs(v, version, true), |(a, u)| write_attrs(a, u, version)) {
            (c.attrs, c.user_data) = (attrs, user);
        }
    } else {
        if version == 3
            && let Some((attrs, _)) = take_same(d, "attr", version, |v| read_attrs(v, version, false), |(a, u)| write_attrs(a, u, version))
        {
            c.attrs = attrs;
        }
        c.user_data = crate::info::take_user_data(d);
    }
    c
}

/// The component's data as entries.
pub(crate) fn push_component_info(e: &mut Entries, c: &ComponentInfo, version: u32, at: &str) -> Result<()> {
    if let Some(a) = alignment_value(c.alignment) {
        e.push(("alignment".into(), int(a)));
    }
    if let Some(a) = &c.anchor {
        e.push(("anchor".into(), string(a)));
    }
    if !c.smart_values.is_empty() {
        e.push(("piece".into(), write_smart_values(&c.smart_values).ok_or_else(|| bad(at, "a smart-component value is not a finite number"))?));
    }
    if let Some(m) = &c.master_id {
        e.push(("masterId".into(), string(m)));
    }
    if c.locked {
        e.push(("locked".into(), int(1)));
    }
    match c.orientation {
        Orientation::Left => {}
        Orientation::Right => e.push(("orientation".into(), int(1))),
        Orientation::Center => e.push(("orientation".into(), int(2))),
    }
    if version >= 4 {
        if !c.attrs.is_empty() || !c.user_data.is_empty() {
            e.push(("attr".into(), write_attrs(&c.attrs, &c.user_data, version).ok_or_else(|| attrs_error(at))?));
        }
    } else {
        if !c.attrs.is_empty() {
            e.push(("attr".into(), write_attrs(&c.attrs, &UserData::default(), version).ok_or_else(|| attrs_error(at))?));
        }
        crate::info::push_user_data(e, &c.user_data);
    }
    Ok(())
}

fn read_component(mut d: Dict, version: u32) -> Option<Shape> {
    let glyph = take(&mut d, "ref")?.as_str()?.to_string();
    let pair = |d: &mut Dict, k: &str, default: (f64, f64)| -> Option<(f64, f64)> {
        match take(d, k) {
            Some(v) => as_pair(&v, k).ok(),
            None => Some(default),
        }
    };
    let pos = pair(&mut d, "pos", (0.0, 0.0))?;
    let scale = pair(&mut d, "scale", (1.0, 1.0))?;
    let slant = pair(&mut d, "slant", (0.0, 0.0))?;
    let angle = match take(&mut d, "angle") {
        Some(v) => v.as_num()?.as_f64(),
        None => 0.0,
    };
    let info = if d.is_empty() { Rare::default() } else { Rare::new(take_component_info(&mut d, version)) };
    Some(Shape::Component(Component { glyph, transform: Transform::Decomposed { pos, scale, angle, slant }, info, extra: into_extra(d) }))
}

fn write_component(c: &Component, at: &str, version: u32) -> Result<Value> {
    let (pos, scale, angle, slant) = match c.transform {
        Transform::Decomposed { pos, scale, angle, slant } => (pos, scale, angle, slant),
        Transform::Matrix(m) => crate::decompose(m).ok_or_else(|| GlyphsError::Unsupported(format!("{at}: a component transform with shear")))?,
    };
    let mut e = vec![("ref".into(), string(&c.glyph))];
    if pos != (0.0, 0.0) {
        e.push(("pos".into(), pair(pos.0, pos.1, at)?));
    }
    if scale != (1.0, 1.0) {
        e.push(("scale".into(), pair(scale.0, scale.1, at)?));
    }
    if angle != 0.0 {
        e.push(("angle".into(), num(angle, at)?));
    }
    if slant != (0.0, 0.0) {
        e.push(("slant".into(), pair(slant.0, slant.1, at)?));
    }
    push_component_info(&mut e, &c.info, version, at)?;
    Ok(Value::Dict(finish(e, &c.extra)))
}

// ── anchors ────────────────────────────────────────────────────────────────────────────────────

fn read_anchor_orientation(v: &Value) -> Option<Orientation> {
    match v.as_str()? {
        "right" => Some(Orientation::Right),
        "center" => Some(Orientation::Center),
        _ => None,
    }
}

/// An anchor's version 4 `attr` holding only its user data.
fn read_anchor_attr(v: &Value) -> Option<UserData> {
    let d = v.as_dict()?;
    match d.entries.as_slice() {
        [(k, Value::Dict(u))] if k.as_str() == "userData" && !u.is_empty() => Some(UserData(u.clone())),
        _ => None,
    }
}

fn anchor_attr(u: &UserData) -> Value {
    Value::Dict(Dict { entries: vec![("userData".into(), Value::Dict(u.0.clone()))], layout: Layout::Multiline, ..Default::default() })
}

/// An anchor's lock, orientation and user data, taken out of its dictionary.
pub(crate) fn take_anchor_info(d: &mut Dict, version: u32) -> AnchorInfo {
    let mut a = AnchorInfo {
        locked: take_flag(d, "locked", version),
        orientation: take_same(d, "orientation", version, read_anchor_orientation, |o| match o {
            Orientation::Right => Some(string("right")),
            Orientation::Center => Some(string("center")),
            Orientation::Left => None,
        })
        .unwrap_or_default(),
        ..Default::default()
    };
    a.user_data = if version >= 4 {
        take_same(d, "attr", version, read_anchor_attr, |u| Some(anchor_attr(u))).unwrap_or_default()
    } else {
        crate::info::take_user_data(d)
    };
    a
}

pub(crate) fn push_anchor_info(e: &mut Entries, a: &AnchorInfo, version: u32) {
    if a.locked {
        e.push(("locked".into(), int(1)));
    }
    match a.orientation {
        Orientation::Left => {}
        Orientation::Right => e.push(("orientation".into(), string("right"))),
        Orientation::Center => e.push(("orientation".into(), string("center"))),
    }
    if version >= 4 {
        if !a.user_data.is_empty() {
            e.push(("attr".into(), anchor_attr(&a.user_data)));
        }
    } else {
        crate::info::push_user_data(e, &a.user_data);
    }
}

pub(crate) fn read_anchor(v: Value, at: &str, version: u32) -> Result<Anchor> {
    let mut d = crate::as_dict(v, at)?;
    let name = crate::opt_string(&mut d, "name", at)?.unwrap_or_default();
    let (x, y) = match take(&mut d, "pos") {
        Some(p) => as_pair(&p, &format!("{at} anchor `{name}`"))?,
        None => (0.0, 0.0),
    };
    let info = if d.is_empty() { Rare::default() } else { Rare::new(take_anchor_info(&mut d, version)) };
    Ok(Anchor { name, x, y, info, extra: into_extra(d) })
}

pub(crate) fn write_anchor(a: &Anchor, at: &str, version: u32) -> Result<Value> {
    let mut e = vec![("name".into(), string(&a.name))];
    if (a.x, a.y) != (0.0, 0.0) {
        e.push(("pos".into(), pair(a.x, a.y, at)?));
    }
    push_anchor_info(&mut e, &a.info, version);
    Ok(Value::Dict(finish(e, &a.extra)))
}

// ── shapes ─────────────────────────────────────────────────────────────────────────────────────

/// A version 4 image shape: a dictionary with an image path or address and no shape-group id.
fn is_image(d: &Dict) -> bool {
    (d.get("imagePath").is_some() || d.get("imageURL").is_some()) && d.get("groupId").is_none()
}

/// A shape the model can hold, or the shape kept as written.
pub(crate) fn read_shape(v: Value, version: u32) -> Shape {
    let typed = match &v {
        Value::Dict(d) if d.get("ref").is_some() => read_component(d.clone(), version),
        Value::Dict(d) if d.get("nodes").is_some() => read_path(d.clone(), version),
        Value::Dict(d) if version >= 4 && is_image(d) => crate::layer::read_image(&v, version)
            .filter(|i| crate::layer::write_image(i, version).is_some_and(|w| same_text(&w, &v, version)))
            .map(|i| Shape::Image(Box::new(i))),
        _ => None,
    };
    typed.unwrap_or_else(|| raw(v))
}

pub(crate) fn write_shape(s: &Shape, at: &str, version: u32) -> Result<Value> {
    match s {
        Shape::Raw(r) if r.family == Family::Glyphs => Ok(r.value.clone()),
        Shape::Raw(_) => Err(GlyphsError::Unsupported(format!("{at}: a shape read from another format"))),
        Shape::Image(i) if version >= 4 => crate::layer::write_image(i, version).ok_or_else(|| bad(at, "an image value is not a finite number")),
        Shape::Image(_) => Err(GlyphsError::Unsupported(format!("{at}: an image among the shapes before format version 4"))),
        Shape::Component(c) => write_component(c, at, version),
        Shape::Path(p) => write_path(p, at, version),
    }
}

/// Whether the attributes hold nothing but a gradient the version cannot write.
fn only_unwritable_gradient(a: &ShapeAttrs, version: u32) -> bool {
    let rest = ShapeAttrs { gradient: None, extra: fontcraft_model::Extra::new(), ..a.clone() };
    let f = Form { version, gray: &[] };
    rest.is_empty() && a.extra.of(Family::Glyphs).next().is_none() && a.gradient.as_ref().is_some_and(|g| paint::write_gradient(g, f).is_none())
}

/// Whether version 3 cannot hold all of a path's paint attributes.
fn paint_needs_v4(a: &ShapeAttrs) -> bool {
    let colors = a.fill_color.iter().chain(&a.stroke_color).chain(a.shadow.iter().chain(&a.shadow_in).map(|s| &s.color));
    a.gradient.as_ref().is_some_and(paint::gradient_needs_v4) || colors.into_iter().any(paint::color_needs_v4)
}

/// What a format version cannot hold of the shapes' data, one line per kind with the number of
/// objects that have it: the identifiers and anchor colours UFO sources give, tangent joins before
/// version 4 (written as smooth), node locks and orientations and path drawing attributes in
/// version 2, the version 4 paint attributes version 3 lacks, and the nodes' higher-order
/// interpolation data before version 4.
pub(crate) fn losses(glyphs: &[std::sync::Arc<fontcraft_model::Glyph>], version: u32) -> Vec<String> {
    let mut counts = [0usize; 9];
    let shapes =
        glyphs.iter().flat_map(|g| g.layers.iter()).flat_map(|l| l.shapes.iter().chain(l.info.background.iter().flat_map(|b| b.shapes.iter())));
    for s in shapes {
        match s {
            Shape::Path(p) => {
                counts[0] += usize::from(p.info.identifier.is_some()) + p.nodes.iter().filter(|n| n.info.identifier.is_some()).count();
                counts[3] += p.nodes.iter().filter(|n| n.tangent).count();
                counts[4] += p.nodes.iter().filter(|n| n.locked || !n.orientation.is_left()).count();
                counts[5] += usize::from(!p.info.attrs.is_empty());
                counts[6] += usize::from(paint_needs_v4(&p.info.attrs));
                counts[7] += usize::from(p.info.attrs.stroke_pos.as_ref().is_some_and(paint::stroke_pos_unconvertible));
                counts[8] += p.nodes.iter().filter(|n| !n.info.hoi.is_empty()).count();
            }
            Shape::Component(c) => counts[1] += usize::from(c.info.identifier.is_some()),
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    let anchors =
        glyphs.iter().flat_map(|g| g.layers.iter()).flat_map(|l| l.anchors.iter().chain(l.info.background.iter().flat_map(|b| b.anchors.iter())));
    for a in anchors {
        counts[2] += usize::from(a.info.identifier.is_some() || a.info.color.is_some());
    }
    if version >= 4 {
        counts[3] = 0;
        counts[8] = 0;
    }
    if version != 2 {
        counts[4] = 0;
        counts[5] = 0;
    }
    if version != 3 {
        counts[6] = 0;
        counts[7] = 0;
    }
    let what = [
        "the identifiers of paths and nodes",
        "the identifiers of components",
        "the colours and identifiers of anchors",
        "tangent nodes, written as smooth",
        "node locks and orientations",
        "the paths' drawing attributes",
        "the paths' conic gradients, extend modes, gradient radii and colours finer than 8 bits",
        "stroke positions given as expressions, written without conversion to the version 3 convention",
        "the nodes' higher-order interpolation curves",
    ];
    what.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(w, n)| format!("{w} ({n})")).collect()
}

mod paint;
#[cfg(test)]
mod tests;
