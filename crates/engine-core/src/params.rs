//! Reading command parameters. Parameters are untrusted (they come from agents, scripts and
//! files), so every accessor returns an error that names the command and the key.
//!
//! [`Targets`] reads the target object of decision 0004 (what an editing command acts on inside
//! one layer) and resolves it against a font: one resolver for every command.

use std::collections::HashSet;
use std::hash::Hash;

use fontcraft_model::{Font, Glyph, Layer, LayerParts, LayerSide, MAX_GLYPHS, Master};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{EngineError, Result};

/// The parameters of one command call. `null` and a missing value count as `{}`.
pub struct Params<'a> {
    command: &'static str,
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Params<'a> {
    pub fn new(command: &'static str, value: &'a Value) -> Result<Self> {
        match value {
            Value::Null => Ok(Self { command, map: None }),
            Value::Object(map) => Ok(Self { command, map: Some(map) }),
            other => Err(EngineError::BadParams { command: command.into(), message: format!("params must be a JSON object, got {}", kind(other)) }),
        }
    }

    /// The command these parameters belong to.
    pub fn command(&self) -> &'static str {
        self.command
    }

    pub fn bad(&self, message: impl Into<String>) -> EngineError {
        EngineError::BadParams { command: self.command.into(), message: message.into() }
    }

    pub fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.and_then(|m| m.get(key)).filter(|v| !v.is_null())
    }

    fn need(&self, key: &str) -> Result<&'a Value> {
        self.get(key).ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    pub fn opt_str(&self, key: &str) -> Result<Option<&'a str>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(other) => Err(self.bad(format!("`{key}` must be a string, got {}", kind(other)))),
        }
    }

    pub fn str(&self, key: &str) -> Result<&'a str> {
        self.need(key)?;
        self.opt_str(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    /// A finite number.
    pub fn opt_f64(&self, key: &str) -> Result<Option<f64>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => match v.as_f64().filter(|n| n.is_finite()) {
                Some(n) => Ok(Some(n)),
                None => Err(self.bad(format!("`{key}` must be a finite number, got {}", kind(v)))),
            },
        }
    }

    pub fn f64(&self, key: &str) -> Result<f64> {
        self.need(key)?;
        self.opt_f64(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    /// A whole number between `min` and `max`.
    pub fn opt_int(&self, key: &str, min: i64, max: i64) -> Result<Option<i64>> {
        let Some(n) = self.opt_f64(key)? else { return Ok(None) };
        if n.fract() != 0.0 || n < min as f64 || n > max as f64 {
            return Err(self.bad(format!("`{key}` must be a whole number from {min} to {max}, got {n}")));
        }
        Ok(Some(n as i64))
    }

    pub fn bool_or(&self, key: &str, default: bool) -> Result<bool> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(self.bad(format!("`{key}` must be true or false, got {}", kind(other)))),
        }
    }

    pub fn opt_array(&self, key: &str) -> Result<Option<&'a Vec<Value>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(a)) => Ok(Some(a)),
            Some(other) => Err(self.bad(format!("`{key}` must be a list, got {}", kind(other)))),
        }
    }

    pub fn array(&self, key: &str) -> Result<&'a Vec<Value>> {
        self.opt_array(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    pub fn opt_object(&self, key: &str) -> Result<Option<&'a Map<String, Value>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Object(m)) => Ok(Some(m)),
            Some(other) => Err(self.bad(format!("`{key}` must be an object, got {}", kind(other)))),
        }
    }

    /// A list of strings: `key: ["a", "b"]`, or one string as a list of one.
    pub fn opt_strings(&self, key: &str) -> Result<Option<Vec<&'a str>>> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(vec![s.as_str()])),
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| v.as_str().ok_or_else(|| self.bad(format!("`{key}` must be a list of strings, found {}", kind(v)))))
                .collect::<Result<Vec<_>>>()
                .map(Some),
            Some(other) => Err(self.bad(format!("`{key}` must be a list of strings, got {}", kind(other)))),
        }
    }

    pub fn strings(&self, key: &str) -> Result<Vec<&'a str>> {
        self.opt_strings(key)?.ok_or_else(|| self.bad(format!("`{key}` is required")))
    }

    /// Glyph names that must all exist in `font`: at most [`MAX_GLYPHS`], repeats dropped, in the
    /// order given.
    pub fn opt_glyphs(&self, key: &str, font: &Font) -> Result<Option<Vec<String>>> {
        if let Some(Value::Array(a)) = self.get(key)
            && a.len() > MAX_GLYPHS
        {
            return Err(self.bad(format!("`{key}` holds {} names; at most {MAX_GLYPHS} are allowed", a.len())));
        }
        let Some(names) = self.opt_strings(key)? else { return Ok(None) };
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if font.glyph(name).is_none() {
                return Err(self.bad(format!("`{key}`: there is no glyph `{name}`")));
            }
            if seen.insert(name) {
                out.push(name.to_string());
            }
        }
        Ok(Some(out))
    }
}

// ── Targets ─────────────────────────────────────────────────────────────────────────────────────

/// Most entries one target list may hold (like `path.addPath`'s nodes).
pub const MAX_TARGETS: usize = 100_000;

/// What kind of target a command takes. The shell fills these parameters from its selection
/// (`CommandSpec::targets`); each name is also the parameter's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    /// `glyphs`: the same layer of several glyphs.
    Glyphs,
    /// `glyph`, `layer?`, `side?`: one layer.
    Layer,
    /// `points`: nodes and handles.
    Points,
    /// `segments`: segments by the node that ends them.
    Segments,
    /// `shapes`: paths and components.
    Shapes,
    Anchors,
    Guides,
    Annotations,
}

/// Whether a guide belongs to the layer or to its master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuideScope {
    Local,
    Global,
}

impl GuideScope {
    pub fn as_str(self) -> &'static str {
        match self {
            GuideScope::Local => "local",
            GuideScope::Global => "global",
        }
    }
}

/// A guide address: global guides are indexed in the layer's master.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GuideRef {
    pub scope: GuideScope,
    pub index: usize,
}

/// The target object (decision 0004): one layer of one glyph, and the lists that name things
/// in it. Indices are positions in the layer's lists, valid for the revision they were read
/// from; [`Targets::resolve`] checks every one. Repeated entries are dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct Targets {
    command: &'static str,
    pub glyph: String,
    /// The layer id; `None` is the first master's layer.
    pub layer: Option<String>,
    pub side: LayerSide,
    pub points: Vec<(usize, usize)>,
    pub segments: Vec<(usize, usize)>,
    pub shapes: Vec<usize>,
    pub anchors: Vec<String>,
    pub guides: Vec<GuideRef>,
    pub annotations: Vec<usize>,
}

/// A target object checked against a font.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedTargets<'f> {
    pub glyph: &'f Glyph,
    pub layer: &'f Layer,
    pub master: &'f Master,
    /// The addressed side of the layer.
    pub parts: LayerParts<'f>,
}

impl Targets {
    /// A target object with no lists, for building a command's `selection` result.
    pub fn new(command: &'static str, glyph: impl Into<String>, layer: Option<String>) -> Self {
        Self {
            command,
            glyph: glyph.into(),
            layer,
            side: LayerSide::Foreground,
            points: Vec::new(),
            segments: Vec::new(),
            shapes: Vec::new(),
            anchors: Vec::new(),
            guides: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// Reads `glyph`, `layer?`, `side?` and the target lists. A list longer than
    /// [`MAX_TARGETS`], or an entry of the wrong form, is an error.
    pub fn parse(p: &Params) -> Result<Self> {
        let mut t = Self::new(p.command(), p.str("glyph")?, p.opt_str("layer")?.map(str::to_string));
        if let Some(side) = p.opt_str("side")? {
            t.side = LayerSide::parse(side).ok_or_else(|| p.bad(format!("`side` must be \"foreground\" or \"background\", got \"{side}\"")))?;
        }
        t.points = list(p, "points", pair)?;
        t.segments = list(p, "segments", pair)?;
        if let Some(v) = p.get("segment") {
            let one = pair(p, "segment", v)?;
            if !t.segments.contains(&one) {
                t.segments.push(one);
            }
        }
        t.shapes = list(p, "shapes", index)?;
        t.anchors = list(p, "anchors", |p, key, v| match v {
            Value::String(s) => Ok(s.clone()),
            other => Err(p.bad(format!("`{key}` must hold anchor names, found {}", kind(other)))),
        })?;
        t.guides = list(p, "guides", guide)?;
        t.annotations = list(p, "annotations", index)?;
        if p.get("glyphs").is_some() && t.has_lists() {
            return Err(p.bad("target lists name things in one glyph: give `glyph`, not `glyphs`"));
        }
        Ok(t)
    }

    /// Whether any target list is given (without one, a command acts on what its row says,
    /// usually the whole layer).
    pub fn has_lists(&self) -> bool {
        !(self.points.is_empty()
            && self.segments.is_empty()
            && self.shapes.is_empty()
            && self.anchors.is_empty()
            && self.guides.is_empty()
            && self.annotations.is_empty())
    }

    pub fn bad(&self, message: impl Into<String>) -> EngineError {
        EngineError::BadParams { command: self.command.into(), message: message.into() }
    }

    /// The id of the addressed layer: the one given, else the first master's.
    pub fn layer_id(&self, font: &Font) -> Result<String> {
        match &self.layer {
            Some(id) => Ok(id.clone()),
            None => Ok(font.default_master()?.id.clone()),
        }
    }

    /// Checks the glyph, the layer and every address against `font`. An address that does not
    /// resolve is an error naming it.
    pub fn resolve<'f>(&self, font: &'f Font) -> Result<ResolvedTargets<'f>> {
        let glyph = font.glyph(&self.glyph).ok_or_else(|| self.bad(format!("there is no glyph `{}`", self.glyph)))?;
        let id = self.layer_id(font)?;
        let layer = glyph.layers.iter().find(|l| l.id == id).ok_or_else(|| self.bad(format!("glyph `{}` has no layer `{id}`", self.glyph)))?;
        let master = font.master(&layer.master)?;
        let parts = layer.parts(self.side);
        self.check(parts, master)?;
        Ok(ResolvedTargets { glyph, layer, master, parts })
    }

    fn check(&self, parts: LayerParts, master: &Master) -> Result<()> {
        let shapes = parts.shapes.len();
        let path_at = |key: &str, s: usize, n: usize| {
            let what = format!("`{key}`: [{s}, {n}] does not resolve");
            if s >= shapes {
                return Err(self.bad(format!("{what}: the layer has {shapes} shapes")));
            }
            let path = parts.path(s).ok_or_else(|| self.bad(format!("{what}: shape {s} is not a path")))?;
            if n >= path.nodes.len() {
                return Err(self.bad(format!("{what}: path {s} has {} nodes", path.nodes.len())));
            }
            Ok(path)
        };
        for &(s, n) in &self.points {
            path_at("points", s, n)?;
        }
        for &(s, n) in &self.segments {
            if !path_at("segments", s, n)?.ends_segment(n) {
                return Err(self.bad(format!("`segments`: node {n} of path {s} does not end a segment")));
            }
        }
        if let Some(s) = self.shapes.iter().find(|s| **s >= shapes) {
            return Err(self.bad(format!("`shapes`: {s} does not resolve: the layer has {shapes} shapes")));
        }
        if let Some(name) = self.anchors.iter().find(|a| parts.anchor(a).is_none()) {
            return Err(self.bad(format!("`anchors`: the layer has no anchor `{name}`")));
        }
        for g in &self.guides {
            let count = match g.scope {
                GuideScope::Local => parts.guides.len(),
                GuideScope::Global => master.guides.len(),
            };
            if g.index >= count {
                return Err(self.bad(format!("`guides`: {} guide {} does not resolve: there are {count}", g.scope.as_str(), g.index)));
            }
        }
        if let Some(i) = self.annotations.iter().find(|i| **i >= parts.annotations.len()) {
            return Err(self.bad(format!("`annotations`: {i} does not resolve: the layer has {}", parts.annotations.len())));
        }
        Ok(())
    }

    /// Runs `f` on the addressed layer of `font` for editing, after checking that the glyph is
    /// not locked and that every address resolves. Editing commands call this inside
    /// the engine's `Session::edit`, so a refusal changes nothing.
    pub fn edit_layer<T>(&self, font: &mut Font, f: impl FnOnce(&mut Layer) -> Result<T>) -> Result<T> {
        let id = {
            let r = self.resolve(font)?;
            crate::check_unlocked(r.glyph)?;
            r.layer.id.clone()
        };
        let mut glyph = font.glyph_mut(&self.glyph)?;
        let layer = glyph.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| self.bad(format!("glyph `{}` has no layer `{id}`", self.glyph)))?;
        f(layer)
    }

    /// The target object as JSON: `glyph`, `layer`, `side` when it is the background, and each
    /// list that is not empty. Commands that change structure return this as `selection`.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("glyph".into(), json!(self.glyph));
        if let Some(l) = &self.layer {
            m.insert("layer".into(), json!(l));
        }
        if self.side == LayerSide::Background {
            m.insert("side".into(), json!(self.side.as_str()));
        }
        let pairs = |v: &[(usize, usize)]| Value::Array(v.iter().map(|(s, n)| json!([s, n])).collect());
        if !self.points.is_empty() {
            m.insert("points".into(), pairs(&self.points));
        }
        if !self.segments.is_empty() {
            m.insert("segments".into(), pairs(&self.segments));
        }
        if !self.shapes.is_empty() {
            m.insert("shapes".into(), json!(self.shapes));
        }
        if !self.anchors.is_empty() {
            m.insert("anchors".into(), json!(self.anchors));
        }
        if !self.guides.is_empty() {
            m.insert("guides".into(), Value::Array(self.guides.iter().map(|g| json!({"scope": g.scope.as_str(), "index": g.index})).collect()));
        }
        if !self.annotations.is_empty() {
            m.insert("annotations".into(), json!(self.annotations));
        }
        Value::Object(m)
    }
}

/// A target list: at most [`MAX_TARGETS`] entries, each read by `entry`, repeats dropped.
fn list<T: Clone + Eq + Hash>(p: &Params, key: &str, entry: impl Fn(&Params, &str, &Value) -> Result<T>) -> Result<Vec<T>> {
    let Some(items) = p.opt_array(key)? else { return Ok(Vec::new()) };
    if items.len() > MAX_TARGETS {
        return Err(p.bad(format!("`{key}` holds {} entries; at most {MAX_TARGETS} are allowed", items.len())));
    }
    let mut seen = HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for v in items {
        let e = entry(p, key, v)?;
        if seen.insert(e.clone()) {
            out.push(e);
        }
    }
    Ok(out)
}

fn index(p: &Params, key: &str, v: &Value) -> Result<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| p.bad(format!("`{key}` must hold whole numbers from 0, found {v}")))
}

/// A `[shape, node]` address.
fn pair(p: &Params, key: &str, v: &Value) -> Result<(usize, usize)> {
    if let Value::Array(a) = v
        && let [s, n] = a.as_slice()
    {
        return Ok((index(p, key, s)?, index(p, key, n)?));
    }
    Err(p.bad(format!("`{key}` must hold [shape, node] pairs of whole numbers, found {v}")))
}

/// A `{scope, index}` guide address.
fn guide(p: &Params, key: &str, v: &Value) -> Result<GuideRef> {
    let Value::Object(o) = v else {
        return Err(p.bad(format!("`{key}` must hold {{scope: \"local\" | \"global\", index}} objects, found {}", kind(v))));
    };
    let scope = match o.get("scope").and_then(Value::as_str) {
        Some("local") => GuideScope::Local,
        Some("global") => GuideScope::Global,
        _ => return Err(p.bad(format!("`{key}`: a guide's `scope` must be \"local\" or \"global\""))),
    };
    let index = index(p, key, o.get("index").unwrap_or(&Value::Null))?;
    Ok(GuideRef { scope, index })
}

/// What kind of JSON value this is, for error messages.
pub fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// A code point given as a number (65), or as hex text (`"0041"`, `"U+0041"`, `"0x41"`).
pub fn codepoint(v: &Value) -> Option<u32> {
    let cp = match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok())?,
        Value::String(s) => {
            let hex = s.trim();
            let hex = hex.strip_prefix("U+").or_else(|| hex.strip_prefix("u+")).or_else(|| hex.strip_prefix("0x")).unwrap_or(hex);
            u32::from_str_radix(hex, 16).ok()?
        }
        _ => return None,
    };
    char::from_u32(cp).map(|_| cp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codepoints_parse_from_numbers_and_hex() {
        assert_eq!(codepoint(&json!(65)), Some(0x41));
        assert_eq!(codepoint(&json!("0041")), Some(0x41));
        assert_eq!(codepoint(&json!("U+1F600")), Some(0x1F600));
        assert_eq!(codepoint(&json!("0x41")), Some(0x41));
        for bad in [json!("xyz"), json!(-1), json!(1.5), json!("D800"), json!(0x110000), json!(null), json!([65])] {
            assert_eq!(codepoint(&bad), None, "{bad}");
        }
    }

    #[test]
    fn accessors_report_the_key_and_the_kind() {
        let v = json!({"name": 5, "n": "x", "list": {}});
        let p = Params::new("test.cmd", &v).unwrap();
        assert!(p.str("name").unwrap_err().to_string().contains("`name` must be a string, got a number"));
        assert!(p.f64("n").unwrap_err().to_string().contains("`n` must be a finite number"));
        assert!(p.array("list").unwrap_err().to_string().contains("must be a list"));
        assert!(p.str("missing").unwrap_err().to_string().contains("`missing` is required"));
        assert!(Params::new("test.cmd", &json!([1])).is_err());
        assert!(Params::new("test.cmd", &Value::Null).unwrap().get("x").is_none());
        assert_eq!(Params::new("t", &json!({"a": "one"})).unwrap().strings("a").unwrap(), ["one"]);
        assert!(Params::new("t", &json!({"a": 2.5})).unwrap().opt_int("a", 0, 10).is_err());
    }
}
