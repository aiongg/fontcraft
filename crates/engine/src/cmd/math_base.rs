//! MATH and BASE data (M13.47): `font.setMathData` and `font.setBaselines`.
//!
//! The math data lives where glyphsLib and ufo2ft read it (`fontcraft_compile::math` has the
//! table of keys), so `.glyphs` and UFO sources keep it and both FontCraft and the reference
//! compilers write the same `MATH` table:
//!
//! | Data | `.glyphs`, native | UFO |
//! |---|---|---|
//! | constants | the master's user data | the font lib |
//! | extended shape | the glyph's user data | a list in the font lib |
//! | variants and assemblies | the glyph's user data | the glyph's lib |
//! | italic correction, top accent, math kerning | anchors `math.ic`, `math.ta`, `math.tr1`… on the master layer | the same anchors |
//!
//! The baselines are a `table BASE { … } BASE;` block in the feature code, which every source
//! format keeps and every compiler reads. The command rewrites the first entry that holds such a
//! block, or adds a prefix named [`BASE_ENTRY`]. A block that holds statements this module does
//! not read (feature-specific extents) is refused rather than rewritten.

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_compile::math::{
    CONSTANTS_KEY, EXTENDED_SHAPE_KEY, GlyphVariants, ITALIC_ANCHOR, KERN_SIDES, KernSide, MAX_KERN_POINTS, MAX_MATH_VALUE, MAX_PARTS, MAX_VARIANTS,
    Part, TOP_ACCENT_ANCHOR, VARIANTS_KEY, is_constant, is_extended_shape, kern_side, number_value, stored_constants, stored_variants,
};
use fontcraft_model::plist::{Array, Dict, Number, Str, Value as PValue};
use fontcraft_model::{Anchor, Font, MAX_GLYPHS, Snippet, is_ufo_family};
use serde_json::{Map, Value, json};

use super::*;
use crate::params::{Params, kind};

/// The math data types the dialogs and agents read.
pub use fontcraft_compile::math::{GlyphVariants as MathVariants, KernSide as MathKernSide, Part as AssemblyPart};

/// The prefix `font.setBaselines` adds when no entry holds a `BASE` block.
pub const BASE_ENTRY: &str = "Baselines";
/// Most baseline tags of one axis.
pub const MAX_BASE_TAGS: usize = 32;
/// Most scripts of one axis.
pub const MAX_BASE_SCRIPTS: usize = 256;
/// Most extent overrides of one axis.
pub const MAX_BASE_EXTENTS: usize = 1_024;
/// Most glyphs one `font.setMathData` call changes.
pub const MAX_MATH_GLYPHS: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.setMathData",
            "Set Math Data…",
            ["Font"],
            None,
            "{master?: id (default: the first master), constants?: {name: number | null} (the MATH specification's constant names, MinConnectorOverlap among them; null removes one), glyphs?: {name: {italicCorrection?: number | null (from the advance width), topAccent?: number | null (from the origin), extendedShape?: bool, kerning?: {topRight?|topLeft?|bottomRight?|bottomLeft?: [[kern, height]] (replaces that corner's points; [] removes them)}, hVariants?: [glyph], vVariants?: [glyph] (the glyph itself may be the first), hAssembly?: [[glyph, flags (1 = extender), startConnector, endConnector]], vAssembly?: … ([] removes a list)}}} sets the data the compiler writes to MATH; keys left out keep their values, and a master without constants gets an empty set so the font has a MATH table → {master, constants: {name: value}, glyphs: {name: {italicCorrection, topAccent, extendedShape, kerning, hVariants, vVariants, hAssembly, vAssembly}}}",
            has_doc,
            set_math
        ),
        cmd!(
            "font.setBaselines",
            "Set Baselines…",
            ["Font"],
            None,
            "{horizontal?: axis | null, vertical?: axis | null (null removes the axis; left out keeps it)} where axis = {tags: [baseline tag, e.g. romn, ideo, hang], scripts: [{script, baseline: its default baseline tag, values: [one coordinate per tag]}], extents?: [{script, language?: tag (default dflt), min, max}]}; writes the feature code's table BASE block, in the first entry that holds one or a new prefix named Baselines → {entry, horizontal, vertical}",
            has_doc,
            set_baselines
        ),
    ]
}

// ---------------------------------------------------------------------------------------------
// MATH
// ---------------------------------------------------------------------------------------------

/// One glyph's math data on one master.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphMath {
    pub italic_correction: Option<f64>,
    pub top_accent: Option<f64>,
    pub extended_shape: bool,
    /// Each corner's points as (kern, height), sorted by height.
    pub kerning: BTreeMap<KernSide, Vec<(f64, f64)>>,
    pub variants: GlyphVariants,
}

impl GlyphMath {
    pub fn to_json(&self) -> Value {
        let parts = |parts: &[Part]| parts.iter().map(|p| json!([p.glyph, p.flags, p.start, p.end])).collect::<Vec<_>>();
        let kerning: Map<String, Value> = self
            .kerning
            .iter()
            .map(|(side, points)| (side.key().to_string(), json!(points.iter().map(|(k, h)| [*k, *h]).collect::<Vec<_>>())))
            .collect();
        json!({
            "italicCorrection": self.italic_correction,
            "topAccent": self.top_accent,
            "extendedShape": self.extended_shape,
            "kerning": kerning,
            "hVariants": self.variants.h_variants,
            "vVariants": self.variants.v_variants,
            "hAssembly": parts(&self.variants.h_assembly),
            "vAssembly": parts(&self.variants.v_assembly),
        })
    }
}

/// `glyph`'s math data on `master`; `None` when the glyph or its master layer is missing.
pub fn glyph_math(font: &Font, master: &str, glyph: &str) -> Option<GlyphMath> {
    let g = font.glyph(glyph)?;
    let layer = g.layer(master)?;
    let mut out =
        GlyphMath { extended_shape: is_extended_shape(font, master, glyph), variants: stored_variants(font, master, glyph), ..Default::default() };
    for a in &layer.anchors {
        if a.name == ITALIC_ANCHOR {
            out.italic_correction = Some(a.x - layer.width);
        } else if a.name == TOP_ACCENT_ANCHOR {
            out.top_accent = Some(a.x);
        } else if let Some(side) = kern_side(&a.name) {
            out.kerning.entry(side).or_default().push((side.kern_from_anchor(a.x, layer.width), a.y));
        }
    }
    for points in out.kerning.values_mut() {
        points.sort_by(|a, b| a.1.total_cmp(&b.1));
    }
    Some(out)
}

/// The constants `master` stores, by name; empty when it stores none.
pub fn math_constants(font: &Font, master: &str) -> Vec<(String, f64)> {
    font.master(master).ok().and_then(|m| stored_constants(&m.user_data)).unwrap_or_default()
}

/// The id of the master `p` names, else the first.
fn master_id(p: &Params, font: &Font) -> Result<String> {
    Ok(match p.opt_str("master")? {
        Some(m) => font.master(m)?.id.clone(),
        None => font.default_master()?.id.clone(),
    })
}

/// A math value: finite and within the table's 16-bit range.
fn math_number(p: &Params, what: &str, v: &Value) -> Result<f64> {
    v.as_f64()
        .filter(|n| n.is_finite() && n.abs() <= MAX_MATH_VALUE)
        .ok_or_else(|| p.bad(format!("{what} must be a number within ±{MAX_MATH_VALUE}, got {}", kind(v))))
}

fn non_negative(p: &Params, what: &str, v: &Value) -> Result<f64> {
    let n = math_number(p, what, v)?;
    if n < 0.0 {
        return Err(p.bad(format!("{what} must not be negative, got {n}")));
    }
    Ok(n)
}

/// What one call changes on one glyph; `None` fields stay as they are.
#[derive(Debug, Default)]
struct GlyphEdit {
    italic: Option<Option<f64>>,
    accent: Option<Option<f64>>,
    extended: Option<bool>,
    kerning: Vec<(KernSide, Vec<(f64, f64)>)>,
    h_variants: Option<Vec<String>>,
    v_variants: Option<Vec<String>>,
    h_assembly: Option<Vec<Part>>,
    v_assembly: Option<Vec<Part>>,
}

fn glyph_names(p: &Params, font: &Font, what: &str, v: &Value) -> Result<Vec<String>> {
    let items = v.as_array().ok_or_else(|| p.bad(format!("{what} must be a list of glyph names, got {}", kind(v))))?;
    if items.len() > MAX_VARIANTS {
        return Err(p.bad(format!("{what} holds {} glyphs; at most {MAX_VARIANTS} are allowed", items.len())));
    }
    items
        .iter()
        .map(|item| {
            let name = item.as_str().ok_or_else(|| p.bad(format!("{what} must hold glyph names, got {}", kind(item))))?;
            if font.glyph(name).is_none() {
                return Err(p.bad(format!("{what}: there is no glyph named `{name}`")));
            }
            Ok(name.to_string())
        })
        .collect()
}

fn assembly(p: &Params, font: &Font, what: &str, v: &Value) -> Result<Vec<Part>> {
    let items = v.as_array().ok_or_else(|| p.bad(format!("{what} must be a list of parts, got {}", kind(v))))?;
    if items.len() > MAX_PARTS {
        return Err(p.bad(format!("{what} holds {} parts; at most {MAX_PARTS} are allowed", items.len())));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = format!("{what}[{i}]");
        let Some([glyph, flags, start, end]) = item.as_array().map(Vec::as_slice) else {
            return Err(p.bad(format!("{at} must be [glyph, flags, startConnector, endConnector]")));
        };
        let glyph = glyph.as_str().ok_or_else(|| p.bad(format!("{at}: the glyph must be a name")))?;
        if font.glyph(glyph).is_none() {
            return Err(p.bad(format!("{at}: there is no glyph named `{glyph}`")));
        }
        let flags = match flags {
            Value::Bool(b) => u16::from(*b),
            other => {
                other.as_u64().filter(|f| *f <= 1).map(|f| f as u16).ok_or_else(|| p.bad(format!("{at}: the flags must be 0 or 1 (1 = extender)")))?
            }
        };
        out.push(Part {
            glyph: glyph.to_string(),
            flags,
            start: non_negative(p, &format!("{at}: the start connector"), start)?,
            end: non_negative(p, &format!("{at}: the end connector"), end)?,
        });
    }
    Ok(out)
}

fn optional_number(p: &Params, what: &str, v: &Value) -> Result<Option<f64>> {
    if v.is_null() { Ok(None) } else { math_number(p, what, v).map(Some) }
}

fn glyph_edit(p: &Params, font: &Font, master: &str, name: &str, v: &Value) -> Result<GlyphEdit> {
    let g = font.glyph(name).ok_or_else(|| p.bad(format!("`glyphs`: there is no glyph named `{name}`")))?;
    if g.layer(master).is_none() {
        return Err(p.bad(format!("the glyph `{name}` has no layer for master `{master}`")));
    }
    let obj = v.as_object().ok_or_else(|| p.bad(format!("`glyphs.{name}` must be an object, got {}", kind(v))))?;
    let mut e = GlyphEdit::default();
    for (key, value) in obj {
        let what = format!("`glyphs.{name}.{key}`");
        match key.as_str() {
            "italicCorrection" => e.italic = Some(optional_number(p, &what, value)?),
            "topAccent" => e.accent = Some(optional_number(p, &what, value)?),
            "extendedShape" => e.extended = Some(value.as_bool().ok_or_else(|| p.bad(format!("{what} must be true or false")))?),
            "hVariants" => e.h_variants = Some(glyph_names(p, font, &what, value)?),
            "vVariants" => e.v_variants = Some(glyph_names(p, font, &what, value)?),
            "hAssembly" => e.h_assembly = Some(assembly(p, font, &what, value)?),
            "vAssembly" => e.v_assembly = Some(assembly(p, font, &what, value)?),
            "kerning" => {
                let sides = value.as_object().ok_or_else(|| p.bad(format!("{what} must be an object by corner")))?;
                for (side_key, points) in sides {
                    let side =
                        KERN_SIDES.iter().map(|(_, s)| *s).find(|s| s.key() == side_key).ok_or_else(|| {
                            p.bad(format!("{what}: `{side_key}` is not a corner; use topRight, topLeft, bottomRight or bottomLeft"))
                        })?;
                    let list = points.as_array().ok_or_else(|| p.bad(format!("{what}.{side_key} must be a list of [kern, height]")))?;
                    if list.len() > MAX_KERN_POINTS {
                        return Err(p.bad(format!("{what}.{side_key} holds {} points; at most {MAX_KERN_POINTS} are allowed", list.len())));
                    }
                    let mut out = Vec::with_capacity(list.len());
                    for point in list {
                        let Some([k, h]) = point.as_array().map(Vec::as_slice) else {
                            return Err(p.bad(format!("{what}.{side_key} must be a list of [kern, height]")));
                        };
                        out.push((math_number(p, &what, k)?, math_number(p, &what, h)?));
                    }
                    e.kerning.push((side, out));
                }
            }
            other => {
                return Err(p.bad(format!(
                    "{what}: `{other}` is not math data; use italicCorrection, topAccent, extendedShape, kerning, hVariants, vVariants, hAssembly or vAssembly"
                )));
            }
        }
    }
    Ok(e)
}

fn remove_key(d: &mut Dict, key: &str) {
    if let Some(i) = d.entries.iter().position(|(k, _)| k.as_str() == key) {
        d.remove_at(i);
    }
}

/// Puts the anchor `name` at `x` (keeping its height, else at 0), or removes it.
fn set_anchor(anchors: &mut Vec<Anchor>, name: &str, x: Option<f64>) {
    let y = anchors.iter().find(|a| a.name == name).map_or(0.0, |a| a.y);
    anchors.retain(|a| a.name != name);
    if let Some(x) = x {
        anchors.push(Anchor::new(name, x, y));
    }
}

fn apply_glyph(p: &Params, font: &mut Font, master: &str, name: &str, e: &GlyphEdit) -> Result<()> {
    let ufo = is_ufo_family(font.source.format);
    let mut variants = stored_variants(font, master, name);
    let variants_changed = e.h_variants.is_some() || e.v_variants.is_some() || e.h_assembly.is_some() || e.v_assembly.is_some();
    if let Some(v) = &e.h_variants {
        variants.h_variants.clone_from(v);
    }
    if let Some(v) = &e.v_variants {
        variants.v_variants.clone_from(v);
    }
    if let Some(v) = &e.h_assembly {
        variants.h_assembly.clone_from(v);
    }
    if let Some(v) = &e.v_assembly {
        variants.v_assembly.clone_from(v);
    }
    // A UFO lists its extended shapes in the font lib (the master's user data).
    if ufo && let Some(on) = e.extended {
        let masters = Arc::make_mut(&mut font.masters);
        let m = masters.iter_mut().find(|m| m.id == master).ok_or_else(|| p.bad(format!("there is no master `{master}`")))?;
        let mut list: Vec<String> = m
            .user_data
            .get(EXTENDED_SHAPE_KEY)
            .and_then(PValue::as_array)
            .map(|a| a.items.iter().take(MAX_GLYPHS).filter_map(PValue::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        list.retain(|n| n != name);
        if on {
            list.push(name.to_string());
        }
        if list.is_empty() {
            remove_key(&mut m.user_data.0, EXTENDED_SHAPE_KEY);
        } else {
            let items = list.iter().map(|n| PValue::String(Str::new(n.as_str()))).collect();
            m.user_data.0.insert(EXTENDED_SHAPE_KEY, PValue::Array(Array { items, ..Default::default() }));
        }
    }
    let mut g = font.glyph_mut(name)?;
    if let Some(on) = e.extended {
        remove_key(&mut g.info.user_data.0, EXTENDED_SHAPE_KEY);
        if on && !ufo {
            // A number: the `.glyphs` text format has no booleans; glyphsLib only asks for the key.
            g.info.user_data.0.insert(EXTENDED_SHAPE_KEY, PValue::Number(Number::int(1)));
        }
    }
    if variants_changed {
        remove_key(&mut g.info.user_data.0, VARIANTS_KEY);
        if !ufo && !variants.is_empty() {
            g.info.user_data.0.insert(VARIANTS_KEY, variants.to_plist());
        }
    }
    g.info.tidy();
    let layer = g.layer_mut(master).ok_or_else(|| p.bad(format!("the glyph `{name}` has no layer for master `{master}`")))?;
    let width = layer.width;
    if let Some(v) = e.italic {
        set_anchor(&mut layer.anchors, ITALIC_ANCHOR, v.map(|v| v + width));
    }
    if let Some(v) = e.accent {
        set_anchor(&mut layer.anchors, TOP_ACCENT_ANCHOR, v);
    }
    for (side, points) in &e.kerning {
        layer.anchors.retain(|a| kern_side(&a.name) != Some(*side));
        for (i, (kern, height)) in points.iter().enumerate() {
            layer.anchors.push(Anchor::new(format!("{}{}", side.prefix(), i + 1), side.anchor_from_kern(*kern, width), *height));
        }
    }
    if variants_changed {
        remove_key(&mut layer.info.user_data.0, VARIANTS_KEY);
        if ufo && !variants.is_empty() {
            layer.info.user_data.0.insert(VARIANTS_KEY, variants.to_plist());
        }
        layer.info.tidy();
    }
    Ok(())
}

fn set_math(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.setMathData", v)?;
    let font = s.font()?;
    let master = master_id(&p, font)?;
    let mut constants: Vec<(String, Option<f64>)> = Vec::new();
    if let Some(obj) = p.opt_object("constants")? {
        for (name, value) in obj {
            if !is_constant(name) {
                return Err(p.bad(format!("`constants`: `{name}` is not a MATH constant; names are as the specification writes them, such as AxisHeight or MinConnectorOverlap")));
            }
            let what = format!("`constants.{name}`");
            let unsigned = matches!(name.as_str(), "DelimitedSubFormulaMinHeight" | "DisplayOperatorMinHeight" | "MinConnectorOverlap");
            let n = if value.is_null() {
                None
            } else if unsigned {
                Some(non_negative(&p, &what, value)?)
            } else {
                Some(math_number(&p, &what, value)?)
            };
            constants.push((name.clone(), n));
        }
    }
    let mut edits: Vec<(String, GlyphEdit)> = Vec::new();
    if let Some(obj) = p.opt_object("glyphs")? {
        if obj.len() > MAX_MATH_GLYPHS {
            return Err(p.bad(format!("`glyphs` names {} glyphs; at most {MAX_MATH_GLYPHS} are allowed", obj.len())));
        }
        for (name, value) in obj {
            edits.push((name.clone(), glyph_edit(&p, font, &master, name, value)?));
        }
    }
    // Nothing to change: no undo step, and a font without math data gains none.
    let changes = !constants.is_empty() || !edits.is_empty();
    if changes {
        s.edit("Set Math Data", |font| {
            {
                let masters = Arc::make_mut(&mut font.masters);
                let m = masters.iter_mut().find(|m| m.id == master).ok_or_else(|| p.bad(format!("there is no master `{master}`")))?;
                let mut stored = m.user_data.get(CONSTANTS_KEY).and_then(PValue::as_dict).cloned().unwrap_or_default();
                for (name, value) in &constants {
                    match value {
                        Some(v) => stored.insert(name.as_str(), number_value(*v)),
                        None => remove_key(&mut stored, name),
                    }
                }
                m.user_data.0.insert(CONSTANTS_KEY, PValue::Dict(stored));
            }
            for (name, e) in &edits {
                apply_glyph(&p, font, &master, name, e)?;
            }
            Ok(())
        })?;
    }
    let font = s.font()?;
    let constants: Map<String, Value> = math_constants(font, &master).into_iter().map(|(k, v)| (k, json!(v))).collect();
    let glyphs: Map<String, Value> = edits.iter().filter_map(|(name, _)| Some((name.clone(), glyph_math(font, &master, name)?.to_json()))).collect();
    Ok(json!({"master": master, "constants": constants, "glyphs": glyphs}))
}

// ---------------------------------------------------------------------------------------------
// BASE
// ---------------------------------------------------------------------------------------------

/// One script's row of a `BASE` axis.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseScript {
    pub script: String,
    /// The default baseline's tag.
    pub baseline: String,
    /// One coordinate per tag of the axis, in tag order.
    pub values: Vec<i32>,
}

/// An extent override: the line's lowest and highest coordinates for a script and language.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseExtent {
    pub script: String,
    pub language: String,
    pub min: i32,
    pub max: i32,
}

/// One direction of the `BASE` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BaseAxis {
    pub tags: Vec<String>,
    pub scripts: Vec<BaseScript>,
    pub extents: Vec<BaseExtent>,
}

/// The `BASE` table as the feature code states it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baselines {
    pub horizontal: Option<BaseAxis>,
    pub vertical: Option<BaseAxis>,
}

impl BaseAxis {
    pub fn to_json(&self) -> Value {
        json!({
            "tags": self.tags,
            "scripts": self.scripts.iter().map(|s| json!({"script": s.script, "baseline": s.baseline, "values": s.values})).collect::<Vec<_>>(),
            "extents": self.extents.iter().map(|e| json!({"script": e.script, "language": e.language, "min": e.min, "max": e.max})).collect::<Vec<_>>(),
        })
    }

    /// The statements of this axis; `prefix` is `HorizAxis` or `VertAxis`.
    fn write(&self, prefix: &str, out: &mut String) {
        out.push_str(&format!("  {prefix}.BaseTagList {};\n", self.tags.join(" ")));
        if !self.scripts.is_empty() {
            let records: Vec<String> = self
                .scripts
                .iter()
                .map(|s| {
                    let values: Vec<String> = s.values.iter().map(i32::to_string).collect();
                    format!("{} {} {}", s.script, s.baseline, values.join(" "))
                })
                .collect();
            out.push_str(&format!("  {prefix}.BaseScriptList {};\n", records.join(", ")));
        }
        for e in &self.extents {
            out.push_str(&format!("  {prefix}.MinMax {} {} {}, {};\n", e.script, e.language, e.min, e.max));
        }
    }
}

impl Baselines {
    pub fn is_empty(&self) -> bool {
        self.horizontal.is_none() && self.vertical.is_none()
    }

    /// The `table BASE` block, ending in a line break; empty when there is no axis.
    pub fn to_fea(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        let mut out = String::from("table BASE {\n");
        if let Some(h) = &self.horizontal {
            h.write("HorizAxis", &mut out);
        }
        if let Some(v) = &self.vertical {
            v.write("VertAxis", &mut out);
        }
        out.push_str("} BASE;\n");
        out
    }
}

/// A token of feature code and where it starts and ends.
struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

/// Words and the punctuation `{ } ; ,`, without comments.
fn tokens(code: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut chars = code.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '#' {
            while chars.peek().is_some_and(|(_, c)| *c != '\n') {
                chars.next();
            }
            continue;
        }
        if matches!(c, '{' | '}' | ';' | ',') {
            out.push(Token { text: code.get(i..i + 1).unwrap_or(""), start: i, end: i + 1 });
            continue;
        }
        let mut end = i + c.len_utf8();
        while let Some(&(j, d)) = chars.peek() {
            if d.is_whitespace() || matches!(d, '{' | '}' | ';' | ',' | '#') {
                break;
            }
            end = j + d.len_utf8();
            chars.next();
        }
        out.push(Token { text: code.get(i..end).unwrap_or(""), start: i, end });
    }
    out
}

/// A tag as the feature code writes it: one to four printable ASCII characters.
fn valid_tag(t: &str) -> bool {
    (1..=4).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic() && !b"{};,#\"".contains(&b))
}

fn parse_axis_statement(axis: &mut BaseAxis, statement: &str, words: &[&str]) -> std::result::Result<(), String> {
    match statement {
        "BaseTagList" => {
            if words.is_empty() || words.len() > MAX_BASE_TAGS || !words.iter().all(|t| valid_tag(t)) {
                return Err("a BaseTagList holds one to 32 tags".into());
            }
            axis.tags = words.iter().map(|t| t.to_string()).collect();
        }
        "BaseScriptList" => {
            for record in words.split(|w| *w == ",") {
                let [script, baseline, values @ ..] = record else {
                    return Err("a BaseScriptList record is a script, a baseline tag and coordinates".into());
                };
                let values: Vec<i32> = values
                    .iter()
                    .map(|v| v.parse::<i32>())
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|_| "a BaseScriptList coordinate is not a whole number")?;
                if axis.scripts.len() >= MAX_BASE_SCRIPTS {
                    return Err(format!("more than {MAX_BASE_SCRIPTS} scripts"));
                }
                axis.scripts.push(BaseScript { script: script.to_string(), baseline: baseline.to_string(), values });
            }
        }
        "MinMax" => {
            let [script, language, min, ",", max] = words else {
                return Err("only MinMax statements of the form `script language min, max` can be edited here".into());
            };
            let (Ok(min), Ok(max)) = (min.parse::<i32>(), max.parse::<i32>()) else { return Err("a MinMax value is not a whole number".into()) };
            if axis.extents.len() >= MAX_BASE_EXTENTS {
                return Err(format!("more than {MAX_BASE_EXTENTS} extents"));
            }
            axis.extents.push(BaseExtent { script: script.to_string(), language: language.to_string(), min, max });
        }
        other => return Err(format!("`{other}` is not a BASE statement")),
    }
    Ok(())
}

/// The first `table BASE` block of `code`: its byte range (from `table` to the closing `;`) and
/// what it states, or why it cannot be read.
pub fn find_base(code: &str) -> Option<(std::ops::Range<usize>, std::result::Result<Baselines, String>)> {
    let toks = tokens(code);
    let at = toks.windows(3).position(|w| matches!(w, [a, b, c] if a.text == "table" && b.text == "BASE" && c.text == "{"))?;
    let body = toks.get(at + 3..)?;
    let close = body.iter().position(|t| t.text == "}")?;
    let start = toks.get(at)?.start;
    let end = match (body.get(close + 1), body.get(close + 2)) {
        (Some(tag), Some(semi)) if tag.text == "BASE" && semi.text == ";" => semi.end,
        (Some(tag), _) if tag.text == "BASE" => tag.end,
        _ => body.get(close)?.end,
    };
    let mut base = Baselines::default();
    let parsed = (|| {
        for statement in body.get(..close).unwrap_or_default().split(|t| t.text == ";") {
            let Some((head, rest)) = statement.split_first() else { continue };
            let (axis_name, what) = head.text.split_once('.').ok_or_else(|| format!("`{}` is not a BASE statement", head.text))?;
            let axis = match axis_name {
                "HorizAxis" => base.horizontal.get_or_insert_with(BaseAxis::default),
                "VertAxis" => base.vertical.get_or_insert_with(BaseAxis::default),
                other => return Err(format!("`{other}` is not a BASE axis")),
            };
            let words: Vec<&str> = rest.iter().map(|t| t.text).collect();
            parse_axis_statement(axis, what, &words)?;
        }
        Ok(())
    })();
    Some((start..end, parsed.map(|()| base)))
}

/// The baselines the font's feature code states: from the first entry with a `BASE` block.
/// `Ok(None)` when no entry has one.
pub fn baselines(font: &Font) -> std::result::Result<Option<(String, Baselines)>, String> {
    let f = &font.features;
    for e in f.prefixes.iter().chain(&f.classes).chain(&f.features) {
        if let Some((_, parsed)) = find_base(&e.code) {
            return parsed.map(|b| Some((e.name.clone(), b)));
        }
    }
    Ok(None)
}

fn tag_param(p: &Params, what: &str, v: Option<&Value>) -> Result<String> {
    let t = v.and_then(Value::as_str).map(str::trim).ok_or_else(|| p.bad(format!("{what} must be a tag")))?;
    if !valid_tag(t) {
        return Err(p.bad(format!("{what} must be one to four letters or digits, got \"{t}\"")));
    }
    Ok(t.to_string())
}

fn coordinate(p: &Params, what: &str, v: Option<&Value>) -> Result<i32> {
    let n = v.and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= MAX_MATH_VALUE);
    n.map(|n| n.round() as i32).ok_or_else(|| p.bad(format!("{what} must be a number within ±{MAX_MATH_VALUE}")))
}

fn axis_param(p: &Params, key: &str, v: &Value) -> Result<BaseAxis> {
    let obj = v.as_object().ok_or_else(|| p.bad(format!("`{key}` must be an object or null, got {}", kind(v))))?;
    let tags_raw = obj.get("tags").and_then(Value::as_array).ok_or_else(|| p.bad(format!("`{key}.tags` must be a list of baseline tags")))?;
    if tags_raw.is_empty() || tags_raw.len() > MAX_BASE_TAGS {
        return Err(p.bad(format!("`{key}.tags` must hold one to {MAX_BASE_TAGS} tags")));
    }
    let given: Vec<String> = tags_raw.iter().enumerate().map(|(i, t)| tag_param(p, &format!("`{key}.tags[{i}]`"), Some(t))).collect::<Result<_>>()?;
    // The table lists the tags in order; the values follow them.
    let mut order: Vec<usize> = (0..given.len()).collect();
    order.sort_by(|a, b| given.get(*a).cmp(&given.get(*b)));
    let tags: Vec<String> = order.iter().filter_map(|i| given.get(*i).cloned()).collect();
    if tags.windows(2).any(|w| matches!(w, [a, b] if a == b)) {
        return Err(p.bad(format!("`{key}.tags` names a tag twice")));
    }
    let mut scripts = Vec::new();
    let scripts_raw = obj.get("scripts").map(|s| s.as_array().ok_or_else(|| p.bad(format!("`{key}.scripts` must be a list")))).transpose()?;
    for (i, s) in scripts_raw.into_iter().flatten().enumerate() {
        let at = format!("`{key}.scripts[{i}]`");
        if i >= MAX_BASE_SCRIPTS {
            return Err(p.bad(format!("`{key}.scripts` holds more than {MAX_BASE_SCRIPTS} scripts")));
        }
        let script = tag_param(p, &format!("{at}.script"), s.get("script"))?;
        let baseline = tag_param(p, &format!("{at}.baseline"), s.get("baseline"))?;
        if !tags.contains(&baseline) {
            return Err(p.bad(format!("{at}.baseline: `{baseline}` is not one of the axis's tags ({})", tags.join(", "))));
        }
        let values = s.get("values").and_then(Value::as_array).ok_or_else(|| p.bad(format!("{at}.values must be a list of coordinates")))?;
        if values.len() != tags.len() {
            return Err(p.bad(format!("{at}.values holds {} coordinates; the axis has {} tags", values.len(), tags.len())));
        }
        let given_values: Vec<i32> = values.iter().map(|v| coordinate(p, &format!("{at}.values"), Some(v))).collect::<Result<_>>()?;
        let values = order.iter().filter_map(|i| given_values.get(*i).copied()).collect();
        if scripts.iter().any(|x: &BaseScript| x.script == script) {
            return Err(p.bad(format!("{at}: the script `{script}` is listed twice")));
        }
        scripts.push(BaseScript { script, baseline, values });
    }
    scripts.sort_by(|a, b| a.script.cmp(&b.script));
    let mut extents = Vec::new();
    let extents_raw = obj.get("extents").map(|s| s.as_array().ok_or_else(|| p.bad(format!("`{key}.extents` must be a list")))).transpose()?;
    for (i, e) in extents_raw.into_iter().flatten().enumerate() {
        let at = format!("`{key}.extents[{i}]`");
        if i >= MAX_BASE_EXTENTS {
            return Err(p.bad(format!("`{key}.extents` holds more than {MAX_BASE_EXTENTS} entries")));
        }
        let script = tag_param(p, &format!("{at}.script"), e.get("script"))?;
        let language = match e.get("language") {
            None | Some(Value::Null) => "dflt".to_string(),
            some => tag_param(p, &format!("{at}.language"), some)?,
        };
        let (min, max) = (coordinate(p, &format!("{at}.min"), e.get("min"))?, coordinate(p, &format!("{at}.max"), e.get("max"))?);
        if min > max {
            return Err(p.bad(format!("{at}: min ({min}) is above max ({max})")));
        }
        extents.push(BaseExtent { script, language, min, max });
    }
    Ok(BaseAxis { tags, scripts, extents })
}

fn set_baselines(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.setBaselines", v)?;
    let font = s.font()?;
    let current = baselines(font).map_err(|e| p.bad(format!("the feature code's BASE block cannot be edited here: {e}")))?;
    let mut base = current.as_ref().map(|(_, b)| b.clone()).unwrap_or_default();
    for (key, slot) in [("horizontal", &mut base.horizontal), ("vertical", &mut base.vertical)] {
        match p.get(key) {
            None => {}
            Some(Value::Null) => *slot = None,
            Some(value) => *slot = Some(axis_param(&p, key, value)?),
        }
    }
    let block = base.to_fea();
    let entry = current.as_ref().map_or(BASE_ENTRY.to_string(), |(name, _)| name.clone());
    let had_block = current.is_some();
    let unchanged = current.as_ref().map(|(_, b)| b) == Some(&base) || (!had_block && base.is_empty());
    let axis_json = |a: &Option<BaseAxis>| a.as_ref().map_or(Value::Null, BaseAxis::to_json);
    if unchanged {
        let entry = if had_block { json!(entry) } else { Value::Null };
        return Ok(json!({"entry": entry, "horizontal": axis_json(&base.horizontal), "vertical": axis_json(&base.vertical)}));
    }
    s.edit("Set Baselines", |font| {
        let f = Arc::make_mut(&mut font.features);
        if had_block {
            for list in [&mut f.prefixes, &mut f.classes, &mut f.features] {
                let Some(e) = list.iter_mut().find(|e| find_base(&e.code).is_some()) else { continue };
                if let Some((range, _)) = find_base(&e.code) {
                    let before = e.code.get(..range.start).unwrap_or_default();
                    let after = e.code.get(range.end..).unwrap_or_default();
                    // The block ends in a line break of its own, and a removed one leaves no blank line.
                    let after = after.strip_prefix('\n').unwrap_or(after);
                    e.code = format!("{before}{block}{after}");
                }
                break;
            }
            // A prefix left empty by removing its block goes.
            f.prefixes.retain(|e| !(e.name == entry && e.code.trim().is_empty()));
        } else if !block.is_empty() {
            f.prefixes.push(Snippet { name: BASE_ENTRY.into(), code: block.clone(), ..Default::default() });
        }
        Ok(())
    })?;
    Ok(
        json!({"entry": if block.is_empty() && !had_block { Value::Null } else { json!(entry) }, "horizontal": axis_json(&base.horizontal), "vertical": axis_json(&base.vertical)}),
    )
}
