//! Annotations: notes and correction marks stored on one glyph layer (`annotation.*`). They are
//! design notes only; nothing reads them when compiling. The Annotation tool and its sub-tools
//! call these commands.

use fontcraft_model::{Annotation, Extra, MAX_ANNOTATIONS, MAX_COORDINATE};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::params::{Params, TargetKind, Targets};

/// A new text note's box width, in font units, when the caller gives none.
pub const DEFAULT_TEXT_WIDTH: f64 = 200.0;
/// A new circle's diameter, in font units, when the caller gives none.
pub const DEFAULT_CIRCLE_DIAMETER: f64 = 100.0;
/// A new arrow's angle, in degrees, when the caller gives none.
pub const DEFAULT_ARROW_ANGLE: f64 = 45.0;
/// Longest note text, in bytes.
pub const MAX_TEXT_LEN: usize = 65_536;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "annotation.add",
            "Add Annotation",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", type: \"text\" | \"arrow\" | \"circle\" | \"plus\" | \"minus\", position: [x, y], text?: string (text only), width?: number (text only; default 200), angle?: degrees (arrow only; default 45), diameter?: number (circle only; default 100)} add one annotation to the layer; positions within ±1,000,000 units, not rounded to the grid; at most 1,024 annotations per layer and 65,536 bytes of text. A locked glyph refuses it → {glyph, layer, annotation: index, selection: {glyph, layer, annotations: [index]}}",
            has_doc,
            add
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "annotation.update",
            "Edit Annotation",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", annotation: index, position?: [x, y], text?: string (text only; \"\" clears it), width?: number (text only), angle?: degrees (arrow only), diameter?: number (circle only)} change one annotation; give at least one value. An annotation of a type FontCraft does not know takes only `position` → {glyph, layer, annotation: index, selection: {glyph, layer, annotations: [index]}}",
            has_doc,
            update
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "annotation.move",
            "Move Annotations",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", annotations: [index], dx: number, dy: number} offset the listed annotations; the arrow keys use 1, 10 and 100 units → {glyph, layer, moved: count, selection: {glyph, layer, annotations: [index]}}",
            has_doc,
            move_annotations
        )
        .targets(&[TargetKind::Annotations]),
        cmd!(
            "annotation.delete",
            "Delete Annotations",
            [],
            Some("Delete"),
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", annotations: [index]} remove the listed annotations; the ones after them move up in the list → {glyph, layer, removed: count, selection: {glyph, layer}}",
            has_doc,
            delete
        )
        .targets(&[TargetKind::Annotations])
        .when("tool:annotation"),
        cmd!(
            query "annotation.list",
            "List Annotations",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} the layer's annotations in list order, with the values each type uses (width 0 or diameter 0: none stored) → {glyph, layer, side, annotations: [{index, type, x, y, text?, width?, angle?, diameter?}]}",
            has_doc,
            list
        )
        .targets(&[TargetKind::Layer]),
    ]
}

/// The annotation types. Files store them capitalised (`Text`); parameters and results use the
/// lower-case form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Text,
    Arrow,
    Circle,
    Plus,
    Minus,
}

impl Kind {
    const ALL: [Kind; 5] = [Kind::Text, Kind::Arrow, Kind::Circle, Kind::Plus, Kind::Minus];

    fn stored(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::Arrow => "Arrow",
            Kind::Circle => "Circle",
            Kind::Plus => "Plus",
            Kind::Minus => "Minus",
        }
    }

    fn param(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Arrow => "arrow",
            Kind::Circle => "circle",
            Kind::Plus => "plus",
            Kind::Minus => "minus",
        }
    }

    /// The type an annotation read from a file has, if FontCraft knows it.
    fn of(a: &Annotation) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.stored().eq_ignore_ascii_case(&a.kind))
    }

    fn parse(p: &Params, s: &str) -> Result<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.param().eq_ignore_ascii_case(s))
            .ok_or_else(|| p.bad(format!("`type` must be text, arrow, circle, plus or minus, got \"{s}\"")))
    }
}

/// The type-specific values a caller may give.
struct Values<'a> {
    position: Option<(f64, f64)>,
    text: Option<&'a str>,
    width: Option<f64>,
    angle: Option<f64>,
    diameter: Option<f64>,
}

impl<'a> Values<'a> {
    fn read(p: &Params<'a>) -> Result<Self> {
        let position = match p.get("position") {
            None => None,
            Some(v) => Some(read_position(p, v)?),
        };
        let text = p.opt_str("text")?;
        if let Some(t) = text
            && t.len() > MAX_TEXT_LEN
        {
            return Err(p.bad(format!("`text` holds {} bytes; at most {MAX_TEXT_LEN} are allowed", t.len())));
        }
        let size = |key: &str| -> Result<Option<f64>> {
            match p.opt_f64(key)? {
                Some(n) if n <= 0.0 || n > MAX_COORDINATE => Err(p.bad(format!("`{key}` must be more than 0 and at most {MAX_COORDINATE}, got {n}"))),
                other => Ok(other),
            }
        };
        let width = size("width")?;
        let diameter = size("diameter")?;
        // `+ 0.0` turns -0 into 0.
        let angle = p.opt_f64("angle")?.map(|a| a.rem_euclid(360.0) + 0.0);
        Ok(Self { position, text, width, angle, diameter })
    }

    fn is_empty(&self) -> bool {
        self.position.is_none() && self.text.is_none() && self.width.is_none() && self.angle.is_none() && self.diameter.is_none()
    }

    /// Refuses a value the annotation's type does not use.
    fn check_for(&self, p: &Params, kind: Option<Kind>) -> Result<()> {
        let name = kind.map_or("this type", Kind::param);
        let wrong = |key: &str, owner: &str| -> Result<()> { Err(p.bad(format!("`{key}` applies to {owner} annotations only, not to {name}"))) };
        if self.text.is_some() && kind != Some(Kind::Text) {
            return wrong("text", "text");
        }
        if self.width.is_some() && kind != Some(Kind::Text) {
            return wrong("width", "text");
        }
        if self.angle.is_some() && kind != Some(Kind::Arrow) {
            return wrong("angle", "arrow");
        }
        if self.diameter.is_some() && kind != Some(Kind::Circle) {
            return wrong("diameter", "circle");
        }
        Ok(())
    }

    /// Writes the given values into `a`. A circle's diameter is stored as its width.
    fn apply(&self, a: &mut Annotation) {
        if let Some((x, y)) = self.position {
            (a.x, a.y) = (x, y);
        }
        if let Some(t) = self.text {
            a.text = (!t.is_empty()).then(|| t.to_string());
        }
        if let Some(w) = self.width.or(self.diameter) {
            a.width = w;
        }
        if let Some(angle) = self.angle {
            a.angle = angle;
        }
    }
}

/// An `[x, y]` position within the coordinate limit.
fn read_position(p: &Params, v: &Value) -> Result<(f64, f64)> {
    if let Value::Array(a) = v
        && let [x, y] = a.as_slice()
        && let (Some(x), Some(y)) = (x.as_f64(), y.as_f64())
        && in_range(x)
        && in_range(y)
    {
        return Ok((x, y));
    }
    Err(p.bad(format!("`position` must be [x, y], two numbers within ±{MAX_COORDINATE}, got {v}")))
}

/// Refuses target lists other than `annotations` (and `annotations` too unless `allowed`).
fn check_lists(p: &Params, t: &Targets, allowed: bool, why: &str) -> Result<()> {
    let others = !(t.points.is_empty() && t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty());
    if others || (!allowed && !t.annotations.is_empty()) {
        return Err(p.bad(why.to_string()));
    }
    Ok(())
}

/// The selection a command returns: the layer, and the annotations given.
fn selection(t: &Targets, annotations: Vec<usize>) -> Value {
    let mut out = t.clone();
    out.annotations = annotations;
    out.to_json()
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("annotation.add", p)?;
    let kind = Kind::parse(&p, p.str("type")?)?;
    let values = Values::read(&p)?;
    let (x, y) = values.position.ok_or_else(|| p.bad("`position` is required"))?;
    values.check_for(&p, Some(kind))?;
    let mut t = Targets::parse(&p)?;
    check_lists(&p, &t, false, "takes no target lists: it adds an annotation to the layer")?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let mut note = Annotation { kind: kind.stored().into(), x, y, angle: 0.0, text: None, width: 0.0, extra: Extra::new() };
    match kind {
        Kind::Text => note.width = DEFAULT_TEXT_WIDTH,
        Kind::Arrow => note.angle = DEFAULT_ARROW_ANGLE,
        Kind::Circle => note.width = DEFAULT_CIRCLE_DIAMETER,
        Kind::Plus | Kind::Minus => {}
    }
    values.apply(&mut note);
    let index = s.edit("Add Annotation", |font| {
        t.edit_layer(font, |l| {
            let list = l.parts_mut(t.side).annotations;
            if list.len() >= MAX_ANNOTATIONS {
                return Err(p.bad(format!("the layer already holds {MAX_ANNOTATIONS} annotations, the most allowed")));
            }
            list.push(note);
            Ok(list.len() - 1)
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer, "annotation": index, "selection": selection(&t, vec![index]) }))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("annotation.update", p)?;
    let index = p.opt_int("annotation", 0, MAX_ANNOTATIONS as i64)?.ok_or_else(|| p.bad("`annotation` is required"))?;
    let index = usize::try_from(index).map_err(|_| p.bad("`annotation` must be a whole number from 0"))?;
    let values = Values::read(&p)?;
    if values.is_empty() {
        return Err(p.bad("give at least one of `position`, `text`, `width`, `angle` or `diameter`"));
    }
    let mut t = Targets::parse(&p)?;
    check_lists(&p, &t, false, "names one annotation with `annotation`; it takes no target lists")?;
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    t.annotations = vec![index];
    s.edit("Edit Annotation", |font| {
        t.edit_layer(font, |l| {
            let a = l.parts_mut(t.side).annotations.get_mut(index).ok_or_else(|| p.bad(format!("`annotation`: {index} does not resolve")))?;
            values.check_for(&p, Kind::of(a))?;
            values.apply(a);
            Ok(())
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer, "annotation": index, "selection": selection(&t, vec![index]) }))
}

fn move_annotations(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("annotation.move", p)?;
    let (dx, dy) = (p.f64("dx")?, p.f64("dy")?);
    let mut t = Targets::parse(&p)?;
    check_lists(&p, &t, true, "moves annotations only: give `annotations`")?;
    if t.annotations.is_empty() {
        return Err(p.bad("`annotations` is required and must not be empty"));
    }
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    s.edit("Move Annotations", |font| {
        t.edit_layer(font, |l| {
            let list = l.parts_mut(t.side).annotations;
            for &i in &t.annotations {
                let a = list.get_mut(i).ok_or_else(|| p.bad(format!("`annotations`: {i} does not resolve")))?;
                let (x, y) = (a.x + dx, a.y + dy);
                if !(in_range(x) && in_range(y)) {
                    return Err(p.bad(format!("annotation {i} would move to ({x}, {y}), beyond ±{MAX_COORDINATE}")));
                }
                (a.x, a.y) = (x, y);
            }
            Ok(())
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer, "moved": t.annotations.len(), "selection": t.to_json() }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("annotation.delete", p)?;
    let mut t = Targets::parse(&p)?;
    check_lists(&p, &t, true, "removes annotations only: give `annotations`")?;
    if t.annotations.is_empty() {
        return Err(p.bad("`annotations` is required and must not be empty"));
    }
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let mut doomed = t.annotations.clone();
    doomed.sort_unstable();
    s.edit("Delete Annotations", |font| {
        t.edit_layer(font, |l| {
            let list = l.parts_mut(t.side).annotations;
            // Highest index first, so the earlier indices stay valid.
            for &i in doomed.iter().rev() {
                if i >= list.len() {
                    return Err(p.bad(format!("`annotations`: {i} does not resolve")));
                }
                list.remove(i);
            }
            Ok(())
        })
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": layer, "removed": doomed.len(), "selection": selection(&t, Vec::new()) }))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("annotation.list", p)?;
    let t = Targets::parse(&p)?;
    check_lists(&p, &t, false, "takes no target lists: it lists every annotation of the layer")?;
    let r = t.resolve(s.font()?)?;
    let notes: Vec<Value> = r.parts.annotations.iter().enumerate().map(|(index, a)| describe(index, a)).collect();
    Ok(json!({ "glyph": r.glyph.name, "layer": r.layer.id, "side": t.side.as_str(), "annotations": notes }))
}

/// The type name commands report: lower-case for the types FontCraft knows, as stored otherwise.
pub(crate) fn type_name(a: &Annotation) -> &str {
    Kind::of(a).map_or(a.kind.as_str(), |k| k.param())
}

/// One annotation with the values its type uses. A type FontCraft does not know is given as
/// stored, with every value the file holds.
fn describe(index: usize, a: &Annotation) -> Value {
    let kind = Kind::of(a);
    let mut m = Map::new();
    m.insert("index".into(), json!(index));
    m.insert("type".into(), json!(type_name(a)));
    m.insert("x".into(), json!(a.x));
    m.insert("y".into(), json!(a.y));
    match kind {
        Some(Kind::Text) => {
            if let Some(text) = &a.text {
                m.insert("text".into(), json!(text));
            }
            m.insert("width".into(), json!(a.width));
        }
        Some(Kind::Arrow) => {
            m.insert("angle".into(), json!(a.angle));
        }
        Some(Kind::Circle) => {
            m.insert("diameter".into(), json!(a.width));
        }
        Some(Kind::Plus | Kind::Minus) => {}
        None => {
            if let Some(text) = &a.text {
                m.insert("text".into(), json!(text));
            }
            m.insert("width".into(), json!(a.width));
            m.insert("angle".into(), json!(a.angle));
        }
    }
    Value::Object(m)
}
