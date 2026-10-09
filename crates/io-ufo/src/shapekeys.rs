//! glyphsLib's lib keys for the data of shapes that a GLIF file has no attribute for: the order of
//! paths and components, the components' alignment, attachment anchor, lock and smart-component
//! values, node user data, and anchor user data.
//!
//! Ported from glyphsLib 6.15.0 (Apache-2.0, see `NOTICE`): `builder/glyph.py` (`shapeOrder`),
//! `builder/components.py`, `builder/paths.py`, `builder/user_data.py` (node user data),
//! `builder/anchors.py` and `builder/constants.py`.
//!
//! | Model | GLIF lib key |
//! |---|---|
//! | the order of paths and components, in a layer with both | `com.schriftgestaltung.Glyphs.shapeOrder` (`P` and `C`); the contours are written first |
//! | a component's alignment and attachment anchor | `com.schriftgestaltung.Glyphs.ComponentInfo`: `{name, index, alignment, anchor}` for each component that has one |
//! | the components' locks and smart-component values | `com.schriftgestaltung.componentsLocked`, `…componentsSmartComponentValues`: one entry per component |
//! | a node's user data and other keys of its attribute dictionary | `com.schriftgestaltung.Glyphs.nodeUserData.<path>.<node>` (indices as the `.glyphs` format counts them: paths only, a closed path's start node last) |
//! | an anchor's user data | `public.objectLibs`, under the anchor's identifier |
//!
//! glyphsLib writes `shapeOrder` whenever a layer has both kinds of shape, so it follows the
//! defaults rule of [`crate::libkeys`]. A smart component stays a component, with its values in
//! `componentsSmartComponentValues` (glyphsLib draws it as outlines; the engine does that first,
//! with `fontcraft_model::smart`, when asked). An anchor with user data and
//! no identifier gets one derived from the glyph's name and the anchor's name and position in the
//! layer, so that saving twice writes the same file (glyphsLib makes a random one); reading takes
//! that identifier off again.

use fontcraft_model::{Alignment, Anchor, Family, Node, NodeInfo, Rare, Shape, SmartValue, UserData};
use fontcraft_plist::{Dict, Num, Value};

use crate::glyphkeys::{Glif, Written, array, coordinate, dict, int, number, string};
use crate::libkeys::sorted_keys;

pub(crate) const SHAPE_ORDER: &str = "com.schriftgestaltung.Glyphs.shapeOrder";
const COMPONENT_INFO: &str = "com.schriftgestaltung.Glyphs.ComponentInfo";
const COMPONENTS_LOCKED: &str = "com.schriftgestaltung.componentsLocked";
const COMPONENTS_SMART: &str = "com.schriftgestaltung.componentsSmartComponentValues";
const NODE_USER_DATA: &str = "com.schriftgestaltung.Glyphs.nodeUserData.";
pub(crate) const OBJECT_LIBS: &str = "public.objectLibs";
/// The keys the GLIF lib writer takes from these entries even where the user data has the key:
/// the user data keeps only the object libs of objects other than anchors.
pub(crate) const MERGED: [&str; 1] = [OBJECT_LIBS];

/// A key of a background's lib this module types.
pub(crate) fn is_background_key(k: &str) -> bool {
    k.starts_with(NODE_USER_DATA) || k == OBJECT_LIBS
}

fn alignment_number(a: Alignment) -> Option<i64> {
    match a {
        Alignment::Default => None,
        Alignment::Disabled => Some(-1),
        Alignment::Forced => Some(1),
        Alignment::Horizontal => Some(3),
    }
}

fn alignment_of(n: Num) -> Option<Alignment> {
    match n {
        Num::Int(0) => Some(Alignment::Default),
        Num::Int(-1) => Some(Alignment::Disabled),
        Num::Int(1) => Some(Alignment::Forced),
        Num::Int(3) => Some(Alignment::Horizontal),
        _ => None,
    }
}

/// Whether a shape is written as a GLIF `<contour>`, and so counts in node user data's path
/// index.
fn is_contour(s: &Shape) -> bool {
    match s {
        Shape::Path(_) => true,
        Shape::Raw(r) => r.family == Family::Ufo && r.value.as_str().is_some_and(|t| t.trim_start().starts_with("<contour")),
        _ => false,
    }
}

/// The index the `.glyphs` format gives node `j` of a path of `n` nodes: a closed path's start
/// node is its last.
fn glyphs_index(closed: bool, n: usize, j: usize) -> usize {
    if closed && n > 0 { (j + n - 1) % n } else { j }
}

fn model_index(closed: bool, n: usize, gi: usize) -> usize {
    if closed && n > 0 { (gi + 1) % n } else { gi }
}

/// A node's dictionary in glyphsLib's form: the node's attribute dictionary without its name, so
/// `userData` and the keys the model keeps as written.
fn node_dict(n: &Node) -> Option<Value> {
    let mut d = Dict::new();
    if !n.info.user_data.is_empty() {
        d.insert("userData", sorted_keys(Value::Dict(n.info.user_data.0.clone()).without_spellings()));
    }
    // A node with a name or user data keeps its other keys under `attr`; one the reader kept whole
    // has them there with its name.
    if let Some(attr) = n.info.extra.get(Family::Glyphs, "attr").and_then(Value::as_dict) {
        for (k, v) in attr.entries.iter().filter(|(k, _)| !["name", "userData"].contains(&k.as_str())) {
            d.insert(k.as_str(), v.without_spellings());
        }
    }
    (!d.is_empty()).then(|| sorted_keys(Value::Dict(d)))
}

/// The identifier an anchor with user data and none of its own is written with: a hash of the
/// glyph's name and the anchor's position and name (FNV-1a, 128 bits), spelled as a UUID.
pub(crate) fn derived_id(glyph: &str, index: usize, name: &str) -> String {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut h = OFFSET;
    let index = index.to_string();
    for b in glyph.bytes().chain([0]).chain(index.bytes()).chain([0]).chain(name.bytes()) {
        h ^= u128::from(b);
        h = h.wrapping_mul(PRIME);
    }
    let hex = format!("{h:032X}");
    let part = |a: usize, b: usize| hex.get(a..b).unwrap_or_default();
    format!("{}-{}-{}-{}-{}", part(0, 8), part(8, 12), part(12, 16), part(16, 20), part(20, 32))
}

// ── writing ────────────────────────────────────────────────────────────────────────────────────

/// Adds the shape keys of one GLIF to `w`.
pub(crate) fn entries(c: &Glif<'_>, w: &mut Written) {
    let l = c.layer;
    let comps: Vec<&fontcraft_model::Component> = l.shapes.iter().filter_map(|s| if let Shape::Component(x) = s { Some(x) } else { None }).collect();
    if !c.background {
        let paths = l.shapes.iter().filter(|s| matches!(s, Shape::Path(_))).count();
        let raw = l.shapes.iter().any(|s| matches!(s, Shape::Raw(_)));
        if paths > 0 && !comps.is_empty() && !raw && !c.user(SHAPE_ORDER) && c.defaults(SHAPE_ORDER) {
            let order: String = l
                .shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Path(_) => Some('P'),
                    Shape::Component(_) => Some('C'),
                    _ => None,
                })
                .collect();
            w.paths_first = true;
            w.put(SHAPE_ORDER, string(&order));
        }
        let info: Vec<Value> = comps
            .iter()
            .enumerate()
            .filter(|(_, x)| x.info.anchor.is_some() || x.info.alignment != Alignment::Default)
            .map(|(i, x)| {
                let mut e: Vec<(&str, Value)> = vec![("index", int(i64::try_from(i).unwrap_or(i64::MAX))), ("name", string(&x.glyph))];
                if let Some(a) = alignment_number(x.info.alignment) {
                    e.push(("alignment", int(a)));
                }
                if let Some(a) = &x.info.anchor {
                    e.push(("anchor", string(a)));
                }
                dict(e)
            })
            .collect();
        if !info.is_empty() {
            w.put(COMPONENT_INFO, array(info));
        }
        if comps.iter().any(|x| x.info.locked) {
            w.put(COMPONENTS_LOCKED, array(comps.iter().map(|x| Value::Bool(x.info.locked)).collect()));
        }
        if comps.iter().any(|x| !x.info.smart_values.is_empty()) {
            let values = comps.iter().map(|x| dict(x.info.smart_values.iter().map(|s| (s.property.as_str(), number(s.value))).collect())).collect();
            w.put(COMPONENTS_SMART, array(values));
        }
    }
    let mut pi = 0usize;
    for s in &l.shapes {
        if let Shape::Path(p) = s {
            for (j, n) in p.nodes.iter().enumerate() {
                if let Some(v) = node_dict(n) {
                    w.put(&format!("{NODE_USER_DATA}{pi}.{}", glyphs_index(p.closed, p.nodes.len(), j)), v);
                }
            }
        }
        if is_contour(s) {
            pi += 1;
        }
    }
    // Anchor user data under the anchors' identifiers, with the object libs of other objects.
    let own = l.info.user_data.get(OBJECT_LIBS);
    if c.format >= 2 && l.anchors.iter().any(|a| !a.info.user_data.is_empty()) && own.is_none_or(|v| v.as_dict().is_some()) {
        let mut libs = own.and_then(Value::as_dict).map(|d| Value::Dict(d.clone()).without_spellings()).unwrap_or(Value::Dict(Dict::new()));
        if let Value::Dict(d) = &mut libs {
            for (k, a) in l.anchors.iter().enumerate().filter(|(_, a)| !a.info.user_data.is_empty()) {
                let id = match &a.info.identifier {
                    Some(id) => id.clone(),
                    None => {
                        let id = derived_id(&c.glyph.name, k, &a.name);
                        w.anchor_ids.push((k, id.clone()));
                        id
                    }
                };
                d.entries.retain(|(x, _)| x.as_str() != id);
                d.insert(id.as_str(), sorted_keys(Value::Dict(a.info.user_data.0.clone()).without_spellings()));
            }
        }
        w.put(OBJECT_LIBS, sorted_keys(libs));
    }
}

// ── reading ────────────────────────────────────────────────────────────────────────────────────

/// What typing a layer's or background's keys gave: keys no longer user data, and keys typed
/// that stay user data with what is left (the object libs of objects other than anchors).
#[derive(Default)]
pub(crate) struct Typed {
    pub consumed: Vec<String>,
    pub kept: Vec<String>,
}

/// Types the shape keys of a layer's user data, but those `skip` names.
pub(crate) fn apply(glyph: &str, l: &mut fontcraft_model::Layer, skip: &dyn Fn(&str) -> bool) -> Typed {
    let mut out = Typed::default();
    let data = l.info.user_data.0.clone();
    for (k, v) in &data.entries {
        let k = k.as_str();
        if skip(k) {
            continue;
        }
        let ok = match k {
            SHAPE_ORDER => shape_order(&mut l.shapes, v),
            COMPONENT_INFO => component_info(&mut l.shapes, v),
            COMPONENTS_LOCKED => components_locked(&mut l.shapes, v),
            COMPONENTS_SMART => components_smart(&mut l.shapes, v),
            OBJECT_LIBS => match anchor_libs(glyph, &mut l.anchors, v) {
                Some(rest) if rest.is_empty() => true,
                Some(rest) => {
                    if let Some(slot) = l.info.user_data.0.get_mut(OBJECT_LIBS) {
                        *slot = Value::Dict(rest);
                    }
                    out.kept.push(k.to_string());
                    false
                }
                None => false,
            },
            _ if k.starts_with(NODE_USER_DATA) => node_user_data(&mut l.shapes, k, v),
            _ => false,
        };
        if ok {
            out.consumed.push(k.to_string());
        }
    }
    out
}

/// Types the node and anchor keys of a background's lib, but those `skip` names. A background's
/// keys are typed all or none ([`crate::glyphkeys`]), so object libs with entries for other
/// objects stay as they are.
pub(crate) fn apply_background(glyph: &str, b: &mut fontcraft_model::Background, lib: &Dict, skip: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in &lib.entries {
        let k = k.as_str();
        if skip(k) {
            continue;
        }
        let ok = if k == OBJECT_LIBS {
            let mut anchors = b.anchors.clone();
            match anchor_libs(glyph, &mut anchors, v) {
                Some(rest) if rest.is_empty() => {
                    b.anchors = anchors;
                    true
                }
                _ => false,
            }
        } else if k.starts_with(NODE_USER_DATA) {
            node_user_data(&mut b.shapes, k, v)
        } else {
            false
        };
        if ok {
            out.push(k.to_string());
        }
    }
    out
}

/// Reorders the shapes by `shapeOrder`, when the file has its contours first (as glyphsLib writes
/// them) and the string counts its paths and components.
fn shape_order(shapes: &mut Vec<Shape>, v: &Value) -> bool {
    let Some(order) = v.as_str() else { return false };
    if shapes.iter().any(|s| matches!(s, Shape::Raw(_) | Shape::Image(_))) {
        return false;
    }
    let first_component = shapes.iter().position(|s| matches!(s, Shape::Component(_))).unwrap_or(shapes.len());
    if shapes.iter().skip(first_component).any(|s| matches!(s, Shape::Path(_))) {
        return false;
    }
    let (paths, comps) = (first_component, shapes.len() - first_component);
    if order.len() != shapes.len() || order.chars().filter(|c| *c == 'P').count() != paths || order.chars().filter(|c| *c == 'C').count() != comps {
        return false;
    }
    let mut ps: std::collections::VecDeque<Shape> = std::collections::VecDeque::new();
    let mut cs: std::collections::VecDeque<Shape> = std::collections::VecDeque::new();
    for s in std::mem::take(shapes) {
        if matches!(s, Shape::Path(_)) {
            ps.push_back(s);
        } else {
            cs.push_back(s);
        }
    }
    for c in order.chars() {
        let next = if c == 'P' { ps.pop_front() } else { cs.pop_front() };
        if let Some(s) = next {
            shapes.push(s);
        }
    }
    true
}

fn components_mut(shapes: &mut [Shape]) -> Vec<&mut fontcraft_model::Component> {
    shapes.iter_mut().filter_map(|s| if let Shape::Component(c) = s { Some(c) } else { None }).collect()
}

fn component_info(shapes: &mut [Shape], v: &Value) -> bool {
    let Some(items) = v.as_array() else { return false };
    let mut comps = components_mut(shapes);
    let mut changes: Vec<(usize, Option<Alignment>, Option<String>)> = Vec::new();
    for e in &items.items {
        let Some(d) = e.as_dict() else { return false };
        if d.entries.iter().any(|(k, _)| !["alignment", "anchor", "index", "name"].contains(&k.as_str())) {
            return false;
        }
        let index = d.get("index").and_then(Value::as_num).and_then(|n| match n {
            Num::Int(i) => usize::try_from(i).ok(),
            Num::Float(_) => None,
        });
        let (Some(i), Some(name)) = (index, d.get("name").and_then(Value::as_str)) else { return false };
        if comps.get(i).is_none_or(|c| c.glyph != name) {
            return false;
        }
        let alignment = match d.get("alignment") {
            Some(a) => match a.as_num().and_then(alignment_of) {
                Some(a) => Some(a),
                None => return false,
            },
            None => None,
        };
        let anchor = match d.get("anchor") {
            Some(a) => match a.as_str() {
                Some(a) => Some(a.to_string()),
                None => return false,
            },
            None => None,
        };
        changes.push((i, alignment, anchor));
    }
    for (i, alignment, anchor) in changes {
        if let Some(c) = comps.get_mut(i) {
            if let Some(a) = alignment {
                c.info.alignment = a;
            }
            if anchor.is_some() {
                c.info.anchor = anchor;
            }
            c.info.tidy();
        }
    }
    true
}

fn components_locked(shapes: &mut [Shape], v: &Value) -> bool {
    let mut comps = components_mut(shapes);
    let Some(flags) = v.as_array().and_then(|a| a.items.iter().map(Value::as_bool).collect::<Option<Vec<bool>>>()) else { return false };
    if flags.len() != comps.len() {
        return false;
    }
    for (c, f) in comps.iter_mut().zip(flags) {
        c.info.locked = f;
        c.info.tidy();
    }
    true
}

fn components_smart(shapes: &mut [Shape], v: &Value) -> bool {
    let mut comps = components_mut(shapes);
    let Some(items) = v.as_array() else { return false };
    if items.items.len() != comps.len() {
        return false;
    }
    let mut all: Vec<Vec<SmartValue>> = Vec::new();
    for e in &items.items {
        let Some(d) = e.as_dict() else { return false };
        if d.len() > fontcraft_model::MAX_PART_PROPERTIES {
            return false;
        }
        let values: Option<Vec<SmartValue>> =
            d.entries.iter().map(|(k, v)| coordinate(v).map(|value| SmartValue { property: k.as_str().to_string(), value })).collect();
        let Some(values) = values else { return false };
        all.push(values);
    }
    for (c, values) in comps.iter_mut().zip(all) {
        c.info.smart_values = values;
        c.info.tidy();
    }
    true
}

/// A `nodeUserData.<path>.<node>` key into the node's user data and the other keys of its
/// attribute dictionary.
fn node_user_data(shapes: &mut [Shape], key: &str, v: &Value) -> bool {
    let Some((p, n)) = key.strip_prefix(NODE_USER_DATA).and_then(|r| r.split_once('.')) else { return false };
    let (Ok(pi), Ok(gi)) = (p.parse::<usize>(), n.parse::<usize>()) else { return false };
    let Some(d) = v.as_dict() else { return false };
    if d.is_empty() || d.get("name").is_some() || d.get("UFO.identifier").is_some() {
        return false;
    }
    let user = match d.get("userData") {
        Some(Value::Dict(u)) => UserData(u.clone()),
        Some(_) => return false,
        None => UserData::default(),
    };
    let mut rest = Dict::new();
    for (k, x) in d.entries.iter().filter(|(k, _)| k.as_str() != "userData") {
        rest.insert(k.as_str(), x.clone());
    }
    let Some(Shape::Path(path)) = shapes.iter_mut().filter(|s| is_contour(s)).nth(pi) else { return false };
    let (closed, len) = (path.closed, path.nodes.len());
    let Some(node) = path.nodes.get_mut(model_index(closed, len, gi)).filter(|_| gi < len) else { return false };
    if !node.info.user_data.is_empty() || node.info.extra.get(Family::Glyphs, "attr").is_some() {
        return false;
    }
    let mut info: NodeInfo = (*node.info).clone();
    info.user_data = user;
    if !rest.is_empty() {
        info.extra.push(Family::Glyphs, "attr", Value::Dict(rest));
    }
    node.info = Rare::new(info);
    node.info.tidy();
    true
}

/// Gives the anchors their user data from `public.objectLibs`; returns the entries left for other
/// objects, or `None` when no anchor has an entry. An identifier derived as the writer derives it
/// is taken off.
fn anchor_libs(glyph: &str, anchors: &mut [Anchor], v: &Value) -> Option<Dict> {
    let mut rest = v.as_dict()?.clone();
    let mut any = false;
    for (k, a) in anchors.iter_mut().enumerate() {
        let Some(id) = a.info.identifier.clone() else { continue };
        let Some(Value::Dict(data)) = rest.get(&id).cloned() else { continue };
        if data.is_empty() || !a.info.user_data.is_empty() {
            continue;
        }
        a.info.user_data = UserData(data);
        if id == derived_id(glyph, k, &a.name) {
            a.info.identifier = None;
        }
        a.info.tidy();
        rest.entries.retain(|(x, _)| x.as_str() != id);
        any = true;
    }
    any.then_some(rest)
}

/// Whether a layer's anchors have user data its GLIF cannot keep: in a UFO 2 (GLIF format 1) an
/// anchor has no identifier to keep it under, and a lib whose `public.objectLibs` is not a
/// dictionary has no place for it.
pub(crate) fn anchor_data_lost(l: &fontcraft_model::Layer, ufo2: bool) -> bool {
    ufo2 && l.anchors.iter().any(|a| !a.info.user_data.is_empty())
        || l.info.user_data.get(OBJECT_LIBS).is_some_and(|v| v.as_dict().is_none()) && l.anchors.iter().any(|a| !a.info.user_data.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ufo;
    use fontcraft_model::{Background, Component, ComponentInfo, Font, Layer, Path};

    fn text(files: &crate::Files, path: &str) -> String {
        String::from_utf8(files[path].clone()).unwrap()
    }

    fn user(entries: &[(&str, Value)]) -> UserData {
        let mut d = Dict::new();
        for (k, v) in entries {
            d.insert(*k, v.clone());
        }
        UserData(d)
    }

    /// A layer with a component, a path and another component; node and anchor user data.
    fn shaped_font(glyphs_version: Option<u32>) -> Font {
        let mut font = Font::new("Shapes");
        font.source.glyphs_version = glyphs_version;
        let mut a = font.glyph_mut("A").unwrap();
        let l: &mut Layer = a.layers.get_mut(0).unwrap();
        // As a UFO holds a placement: a matrix.
        let matrix = |dx: f64| fontcraft_model::Transform::Matrix([1.0, 0.0, 0.0, 1.0, dx, 0.0]);
        let mut first = Component::new("B", 0.0, 0.0);
        first.transform = matrix(0.0);
        first.info = Rare::new(ComponentInfo { alignment: Alignment::Disabled, anchor: Some("top".into()), locked: true, ..Default::default() });
        let mut path = Path::rect(0.0, 0.0, 100.0, 100.0);
        let node = path.nodes.get_mut(0).unwrap();
        let mut info = NodeInfo { user_data: user(&[("k", string("v"))]), ..Default::default() };
        let mut attr = Dict::new();
        attr.insert("test", string("Hallo"));
        info.extra.push(Family::Glyphs, "attr", Value::Dict(attr));
        node.info = Rare::new(info);
        let mut second = Component::new("C", 10.0, 0.0);
        second.transform = matrix(10.0);
        second.info = Rare::new(ComponentInfo { smart_values: vec![SmartValue { property: "Width".into(), value: 29.5 }], ..Default::default() });
        l.shapes = vec![Shape::Component(first), Shape::Path(path), Shape::Component(second)];
        let mut top = Anchor::new("top", 50.0, 700.0);
        top.info.user_data = user(&[("Some Key", string("Some Value"))]);
        let mut named = Anchor::new("bottom", 50.0, 0.0);
        named.info.identifier = Some("ABC".into());
        named.info.user_data = user(&[("x", int(1))]);
        l.anchors = vec![Anchor::new("center", 1.0, 1.0), top, named];
        l.info.background = Some(Background { shapes: vec![Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0))], ..Default::default() });
        drop(a);
        font
    }

    #[test]
    fn shape_data_goes_to_glyphslib_keys_and_comes_back() {
        let font = shaped_font(Some(3));
        let files = ufo::write(&font).unwrap();
        let a = text(&files, "glyphs/A_.glif");
        let id = derived_id("A", 1, "top");
        for expected in [
            "<key>com.schriftgestaltung.Glyphs.shapeOrder</key>\n      <string>CPC</string>",
            "<key>com.schriftgestaltung.Glyphs.ComponentInfo</key>\n      <array>\n        <dict>\n          <key>alignment</key>\n          <integer>-1</integer>\n          <key>anchor</key>\n          <string>top</string>\n          <key>index</key>\n          <integer>0</integer>\n          <key>name</key>\n          <string>B</string>",
            "<key>com.schriftgestaltung.componentsLocked</key>\n      <array>\n        <true/>\n        <false/>",
            "<key>com.schriftgestaltung.componentsSmartComponentValues</key>\n      <array>\n        <dict/>\n        <dict>\n          <key>Width</key>\n          <real>29.5</real>",
            // The path's first node is the `.glyphs` format's last.
            "<key>com.schriftgestaltung.Glyphs.nodeUserData.0.3</key>\n      <dict>\n        <key>test</key>\n        <string>Hallo</string>\n        <key>userData</key>",
            "<key>public.objectLibs</key>\n      <dict>\n        <key>",
        ] {
            assert!(a.contains(expected), "{expected} in {a}");
        }
        assert!(a.contains(&format!("identifier=\"{id}\"")) && a.contains(&format!("<key>{id}</key>")), "{a}");
        // The contours go first.
        let contour = a.find("<contour").unwrap();
        assert!(a.find("<component base=\"B\"").unwrap() > contour, "{a}");
        let back = ufo::read(&files).unwrap();
        let (l, was) = (&back.glyph("A").unwrap().layers[0], &font.glyph("A").unwrap().layers[0]);
        assert_eq!(l.shapes, was.shapes, "the shapes, in their order and with their data");
        assert_eq!(l.anchors, was.anchors, "the derived identifier is taken off again");
        assert!(l.info.user_data.is_empty(), "{:?}", l.info.user_data);
        assert_eq!(ufo::write(&back).unwrap(), files, "it comes back unchanged");
        assert_eq!(derived_id("A", 1, "top"), id, "the identifier does not change between saves");
    }

    #[test]
    fn a_ufo_of_another_document_keeps_its_shape_order() {
        let files = ufo::write(&shaped_font(None)).unwrap();
        let a = text(&files, "glyphs/A_.glif");
        assert!(!a.contains("shapeOrder") && a.find("<component base=\"B\"").unwrap() < a.find("<contour").unwrap(), "{a}");
    }

    #[test]
    fn keys_that_do_not_fit_the_shapes_stay_user_data() {
        let mut font = shaped_font(None);
        {
            let mut a = font.glyph_mut("A").unwrap();
            let l = a.layers.get_mut(0).unwrap();
            l.info.user_data = user(&[
                (SHAPE_ORDER, string("PPC")),
                (COMPONENT_INFO, array(vec![dict(vec![("index", int(0)), ("name", string("Z"))])])),
                (COMPONENTS_LOCKED, array(vec![Value::Bool(true)])),
                ("com.schriftgestaltung.Glyphs.nodeUserData.0.1", dict(vec![("name", string("x"))])),
                ("com.schriftgestaltung.Glyphs.nodeUserData.9.0", dict(vec![("k", int(1))])),
            ]);
            for s in &mut l.shapes {
                match s {
                    Shape::Component(c) => c.info = Rare::default(),
                    Shape::Path(p) => p.nodes.iter_mut().for_each(|n| n.info = Rare::default()),
                    _ => {}
                }
            }
            l.anchors.iter_mut().for_each(|a| a.info = Rare::default());
        }
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        let l = &back.glyph("A").unwrap().layers[0];
        assert_eq!(l.info.user_data.0.len(), 5, "{:?}", l.info.user_data);
        assert_eq!(l.shapes, font.glyph("A").unwrap().layers[0].shapes);
        assert_eq!(ufo::write(&back).unwrap(), files);
    }

    #[test]
    fn a_random_identifier_stays_and_other_object_libs_stay_user_data() {
        let mut font = Font::new("Libs");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let l = a.layers.get_mut(0).unwrap();
            let mut anchor = Anchor::new("top", 1.0, 2.0);
            anchor.info.identifier = Some("F2D78097-D81D-4CA6-974E-2632096BE367".into());
            l.anchors = vec![anchor];
            let mut libs = Dict::new();
            libs.insert("F2D78097-D81D-4CA6-974E-2632096BE367", dict(vec![("Some Key", string("Some Value"))]));
            libs.insert("guide-1", dict(vec![("x", int(1))]));
            l.info.user_data = user(&[(OBJECT_LIBS, Value::Dict(libs))]);
        }
        let files = ufo::write(&font).unwrap();
        let back = ufo::read(&files).unwrap();
        let l = &back.glyph("A").unwrap().layers[0];
        let a = &l.anchors[0];
        assert_eq!(a.info.identifier.as_deref(), Some("F2D78097-D81D-4CA6-974E-2632096BE367"));
        assert_eq!(a.info.user_data.get("Some Key"), Some(&string("Some Value")));
        let rest = l.info.user_data.get(OBJECT_LIBS).and_then(Value::as_dict).unwrap();
        assert_eq!(rest.len(), 1, "the guideline's lib stays");
        assert_eq!(ufo::write(&back).unwrap(), files);
    }

    #[test]
    fn a_background_is_typed_whole_or_not_at_all() {
        let mut font = Font::new("Backgrounds");
        {
            let mut a = font.glyph_mut("A").unwrap();
            let l = a.layers.get_mut(0).unwrap();
            let mut path = Path::rect(0.0, 0.0, 10.0, 10.0);
            path.nodes.get_mut(1).unwrap().info = Rare::new(NodeInfo { user_data: user(&[("k", int(1))]), ..Default::default() });
            l.info.background = Some(Background { shapes: vec![Shape::Path(path)], ..Default::default() });
        }
        let files = ufo::write(&font).unwrap();
        let bg = text(&files, "glyphs.public.background/A_.glif");
        assert!(bg.contains("<key>com.schriftgestaltung.Glyphs.nodeUserData.0.0</key>"), "{bg}");
        let back = ufo::read(&files).unwrap();
        assert_eq!(
            back.glyph("A").unwrap().layers[0].info.background.as_ref().unwrap().shapes,
            font.glyph("A").unwrap().layers[0].info.background.as_ref().unwrap().shapes
        );
        assert_eq!(ufo::write(&back).unwrap(), files);
        // A key of the background that does not read keeps the others as user data too.
        let mut odd = files.clone();
        let glif = bg.replace("<key>com.schriftgestaltung.Glyphs.nodeUserData.0.0</key>", "<key>com.schriftgestaltung.Glyphs.nodeUserData.7.0</key>");
        odd.insert("glyphs.public.background/A_.glif".into(), glif.into_bytes());
        let back = ufo::read(&odd).unwrap();
        let b = back.glyph("A").unwrap().layers[0].info.background.as_ref().unwrap();
        assert!(matches!(&b.shapes[0], Shape::Path(p) if p.nodes.iter().all(|n| n.info.user_data.is_empty())));
        assert_eq!(ufo::write(&back).unwrap(), odd);
    }
}
