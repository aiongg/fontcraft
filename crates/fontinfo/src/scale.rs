//! A font scaled to a new units-per-em: every value measured in font units, as the
//! `font.info.scaleToUnitsPerEm` row's guess lists them, multiplied by `new / old` and rounded to
//! the font's grid (no rounding when the grid step is 0).
//!
//! | Scaled | Kept |
//! |---|---|
//! | nodes of every path, in every layer and background | component scales, rotations and slants |
//! | component offsets | metrics keys, italic angles, smart-component values |
//! | anchors, layer and master guides (their positions) | guide angles |
//! | hint positions and widths (`place`) | hint node references |
//! | annotation positions and widths | number values (`Font::numbers`) |
//! | placed images (position and size) | image crops (in image pixels) |
//! | advance widths, vertical widths and origins | raw shapes the model keeps as stored |
//! | stroke widths and heights | percentages and ratios |
//! | master metric positions and overshoots, zones, stem widths | |
//! | kerning in every direction, contextual kerning | |
//! | parameters whose catalogue entry has [`crate::ParamSpec::units`] (font, masters, exports) | |
//!
//! A scaling that would move any value past [`crate::MAX_SCALED_COORDINATE`] is refused, and the
//! font is unchanged. The factor is `units_per_em / font.units_per_em`, always finite.
//!
//! Rows: `font.info.scaleToUnitsPerEm`, `fontInfo.unitsPerEm.resizeTrick` (M7.4); the `Scale to
//! UPM` parameter at export (`engine/src/export_steps.rs`, M7.10).

use std::sync::Arc;

use fontcraft_geom::{grid_step, round_to_grid};
use fontcraft_model::{
    Anchor, Annotation, Dimension, Font, Glyph, Guide, Hint, Image, KernTable, Layer, Params, Shape, ShapeAttrs, Transform, metric,
};
use fontcraft_plist::{Number, Str, Value};

use crate::error::ScaleError;

/// `font` with units per em `units_per_em` and every value in font units scaled to match.
pub fn scale_font(font: &Font, units_per_em: u16) -> Result<Font, ScaleError> {
    if !crate::UPM_RANGE.contains(&units_per_em) {
        return Err(ScaleError::UnitsPerEm(u32::from(units_per_em)));
    }
    if !crate::UPM_RANGE.contains(&font.units_per_em) {
        return Err(ScaleError::UnitsPerEm(u32::from(font.units_per_em)));
    }
    if units_per_em == font.units_per_em {
        return Ok(font.clone());
    }
    let settings = &font.info.settings;
    let scaler = Scaler {
        factor: f64::from(units_per_em) / f64::from(font.units_per_em),
        grid: grid_step(settings.grid_length.unwrap_or(1.0), settings.grid_sub_division.unwrap_or(1.0)),
    };
    let mut out = font.clone();
    out.units_per_em = units_per_em;

    for i in 0..out.glyphs.len() {
        let Some(mut guard) = out.glyphs.get_mut_at(i) else { continue };
        let glyph: &mut Glyph = &mut guard;
        scaler.glyph(glyph)?;
    }

    let defs = Arc::clone(&font.metrics);
    for m in Arc::make_mut(&mut out.masters).iter_mut() {
        let what = format!("master `{}`", m.name);
        for (i, v) in m.metric_values.iter_mut().enumerate() {
            if defs.get(i).is_some_and(|d| d.kind == metric::ITALIC_ANGLE) {
                continue;
            }
            v.pos = scaler.len(v.pos, &what)?;
            v.over = scaler.len(v.over, &what)?;
        }
        for z in &mut m.zones {
            z.pos = scaler.len(z.pos, &what)?;
            z.size = scaler.len(z.size, &what)?;
        }
        for s in &mut m.stem_values {
            *s = scaler.len(*s, &what)?;
        }
        scaler.guides(&mut m.guides, &what)?;
        scaler.params(&mut m.params, &what)?;
    }

    if !out.instances.is_empty() {
        for (i, inst) in Arc::make_mut(&mut out.instances).iter_mut().enumerate() {
            let what = if inst.name.is_empty() { format!("export #{i}") } else { format!("export `{}`", inst.name) };
            scaler.params(&mut inst.params, &what)?;
        }
    }

    if !out.info.params.is_empty() {
        scaler.params(&mut Arc::make_mut(&mut out.info).params, "the font")?;
    }

    if !out.kerning.is_empty() {
        let k = Arc::make_mut(&mut out.kerning);
        for table in [&mut k.ltr, &mut k.rtl, &mut k.vertical] {
            scaler.kern_table(table)?;
        }
        for c in &mut k.context {
            let what = format!("contextual kerning `{}`", c.context);
            for (_, v) in &mut c.values {
                *v = scaler.len(*v, &what)?;
            }
        }
    }
    Ok(out)
}

/// The factor and the grid every value is scaled and rounded with.
struct Scaler {
    factor: f64,
    /// The grid step; 0 for no rounding.
    grid: f64,
}

impl Scaler {
    /// A length or coordinate scaled and rounded; `what` names its place in the refusal.
    fn len(&self, v: f64, what: &str) -> Result<f64, ScaleError> {
        let s = round_to_grid(v * self.factor, self.grid);
        if !s.is_finite() || s.abs() > crate::MAX_SCALED_COORDINATE {
            return Err(ScaleError::OutOfRange { what: what.to_string() });
        }
        // Rounding a small negative value gives -0, which writers would spell `-0`.
        Ok(if s == 0.0 { 0.0 } else { s })
    }

    /// A ratio (an image's scale) multiplied by the factor, not rounded.
    fn ratio(&self, v: f64, what: &str) -> Result<f64, ScaleError> {
        let s = v * self.factor;
        if s.is_finite() { Ok(s) } else { Err(ScaleError::OutOfRange { what: what.to_string() }) }
    }

    fn glyph(&self, g: &mut Glyph) -> Result<(), ScaleError> {
        for l in &mut g.layers {
            let what = format!("glyph `{}`, layer `{}`", g.name, l.id);
            self.layer(l, &what)?;
        }
        Ok(())
    }

    fn layer(&self, l: &mut Layer, what: &str) -> Result<(), ScaleError> {
        l.width = self.len(l.width, what)?;
        self.shapes(&mut l.shapes, what)?;
        self.anchors(&mut l.anchors, what)?;
        // Writing through an empty box would allocate one; most layers have no extra data.
        if l.info.is_default() {
            return Ok(());
        }
        let info = &mut *l.info;
        if let Some(v) = info.vert_width.as_mut() {
            *v = self.len(*v, what)?;
        }
        if let Some(v) = info.vert_origin.as_mut() {
            *v = self.len(*v, what)?;
        }
        self.guides(&mut info.guides, what)?;
        self.hints(&mut info.hints, what)?;
        self.annotations(&mut info.annotations, what)?;
        if let Some(image) = info.image.as_mut() {
            self.image(image, what)?;
        }
        if let Some(bg) = info.background.as_mut() {
            let what = format!("{what} (background)");
            self.shapes(&mut bg.shapes, &what)?;
            self.anchors(&mut bg.anchors, &what)?;
            self.guides(&mut bg.guides, &what)?;
            self.hints(&mut bg.hints, &what)?;
            self.annotations(&mut bg.annotations, &what)?;
            if let Some(image) = bg.image.as_mut() {
                self.image(image, &what)?;
            }
        }
        Ok(())
    }

    fn shapes(&self, shapes: &mut [Shape], what: &str) -> Result<(), ScaleError> {
        for shape in shapes {
            match shape {
                Shape::Path(p) => {
                    for n in &mut p.nodes {
                        n.x = self.len(n.x, what)?;
                        n.y = self.len(n.y, what)?;
                    }
                    if !p.info.is_default() {
                        self.attrs(&mut p.info.attrs, what)?;
                    }
                }
                Shape::Component(c) => {
                    self.offset(&mut c.transform, what)?;
                    if !c.info.is_default() {
                        self.attrs(&mut c.info.attrs, what)?;
                    }
                }
                Shape::Image(image) => self.image(image, what)?,
                // Shape groups and paths with node kinds the model cannot read are kept as stored.
                Shape::Raw(_) => {}
            }
        }
        Ok(())
    }

    fn attrs(&self, a: &mut ShapeAttrs, what: &str) -> Result<(), ScaleError> {
        for d in [&mut a.stroke_width, &mut a.stroke_height] {
            if let Some(Dimension::Number(v)) = d {
                *v = self.len(*v, what)?;
            }
        }
        Ok(())
    }

    /// A component's offset; its scale, rotation and slant are kept.
    fn offset(&self, t: &mut Transform, what: &str) -> Result<(), ScaleError> {
        match t {
            Transform::Decomposed { pos, .. } => {
                pos.0 = self.len(pos.0, what)?;
                pos.1 = self.len(pos.1, what)?;
            }
            Transform::Matrix([_, _, _, _, dx, dy]) => {
                *dx = self.len(*dx, what)?;
                *dy = self.len(*dy, what)?;
            }
        }
        Ok(())
    }

    /// A placed image keeps its place over the scaled outline: its position is scaled and its
    /// own scale multiplied by the factor.
    fn image(&self, image: &mut Image, what: &str) -> Result<(), ScaleError> {
        match &mut image.transform {
            Transform::Decomposed { pos, scale, .. } => {
                pos.0 = self.len(pos.0, what)?;
                pos.1 = self.len(pos.1, what)?;
                scale.0 = self.ratio(scale.0, what)?;
                scale.1 = self.ratio(scale.1, what)?;
            }
            Transform::Matrix([xx, yx, xy, yy, dx, dy]) => {
                for v in [xx, yx, xy, yy] {
                    *v = self.ratio(*v, what)?;
                }
                *dx = self.len(*dx, what)?;
                *dy = self.len(*dy, what)?;
            }
        }
        Ok(())
    }

    fn anchors(&self, anchors: &mut [Anchor], what: &str) -> Result<(), ScaleError> {
        for a in anchors {
            a.x = self.len(a.x, what)?;
            a.y = self.len(a.y, what)?;
        }
        Ok(())
    }

    fn guides(&self, guides: &mut [Guide], what: &str) -> Result<(), ScaleError> {
        for g in guides {
            g.x = self.len(g.x, what)?;
            g.y = self.len(g.y, what)?;
        }
        Ok(())
    }

    fn annotations(&self, annotations: &mut [Annotation], what: &str) -> Result<(), ScaleError> {
        for a in annotations {
            a.x = self.len(a.x, what)?;
            a.y = self.len(a.y, what)?;
            a.width = self.len(a.width, what)?;
        }
        Ok(())
    }

    /// A hint's `place` (position and width, for a hint not attached to nodes), stored as two
    /// numbers or as format 2's brace text `{pos, width}`. Node references are kept.
    fn hints(&self, hints: &mut [Hint], what: &str) -> Result<(), ScaleError> {
        for h in hints {
            let Value::Dict(d) = &mut h.value else { continue };
            let Some(place) = d.get_mut("place") else { continue };
            match place {
                Value::Array(a) => {
                    for item in &mut a.items {
                        self.number(item, what, 1)?;
                    }
                }
                Value::String(s) => {
                    if let Some(text) = self.brace_text(s.as_str(), what)? {
                        *s = Str::new(text);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `{a, b}` with both numbers scaled, or `None` when the text is not of that form.
    fn brace_text(&self, text: &str, what: &str) -> Result<Option<String>, ScaleError> {
        let Some(inner) = text.trim().strip_prefix('{').and_then(|t| t.strip_suffix('}')) else { return Ok(None) };
        let mut parts = Vec::with_capacity(2);
        for part in inner.split(',') {
            let Some(v) = part.trim().parse::<f64>().ok().filter(|v| v.is_finite()) else { return Ok(None) };
            if parts.len() >= 2 {
                return Ok(None);
            }
            parts.push(number_text(self.len(v, what)?));
        }
        Ok(Some(format!("{{{}}}", parts.join(", "))))
    }

    fn kern_table(&self, table: &mut KernTable) -> Result<(), ScaleError> {
        for mk in &mut table.0 {
            let what = format!("the kerning of master `{}`", mk.master);
            for f in &mut mk.firsts {
                for (_, v) in &mut f.seconds {
                    *v = self.len(*v, &what)?;
                }
            }
        }
        Ok(())
    }

    /// The parameters measured in font units, by the catalogue; disabled ones too, so that
    /// enabling one later gives a value of the right size.
    fn params(&self, params: &mut Params, owner: &str) -> Result<(), ScaleError> {
        for p in &mut params.0 {
            if !crate::catalog::lookup(&p.name).is_some_and(|spec| spec.units) {
                continue;
            }
            let what = format!("parameter `{}` of {owner}", p.name);
            self.number(&mut p.value, &what, 0)?;
        }
        Ok(())
    }

    /// A parameter value's numbers scaled: a number, a text that reads as one, and the items of
    /// a list (to [`crate::MAX_VALUE_DEPTH`]). Other values are kept.
    fn number(&self, v: &mut Value, what: &str, depth: usize) -> Result<(), ScaleError> {
        match v {
            Value::Number(n) => *v = number_value(self.len(n.value().as_f64(), what)?),
            Value::String(s) => {
                if let Some(x) = s.as_str().trim().parse::<f64>().ok().filter(|x| x.is_finite()) {
                    *s = Str::new(number_text(self.len(x, what)?));
                }
            }
            Value::Array(a) if depth < crate::MAX_VALUE_DEPTH => {
                for item in &mut a.items {
                    self.number(item, what, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Whole numbers beyond this are written as floats (exact integers in an `f64` end at 2^53).
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// A finite number as a property-list number: an integer when it is whole.
fn number_value(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < MAX_EXACT_INTEGER {
        return Value::Number(Number::int(v as i64));
    }
    Value::Number(Number::float(v).unwrap_or_else(|| Number::int(0)))
}

/// A finite number as text, without a decimal point when it is whole.
fn number_text(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < MAX_EXACT_INTEGER { format!("{}", v as i64) } else { format!("{v}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_model::{AlignmentZone, Background, Component, KernFirst, LayerInfo, MasterKerning, Param, Path, Rare, StemDef};

    fn font() -> Font {
        let mut font = Font::new("Scale");
        {
            let mut g = font.glyph_mut("A").unwrap();
            let l = g.layer_mut("m01").unwrap();
            l.shapes.push(Shape::Path(Path::rect(15.0, 0.0, 333.0, 701.0)));
            l.anchors.push(Anchor::new("top", 301.0, 701.0));
            l.info = Rare::new(LayerInfo {
                guides: vec![Guide { x: 11.0, y: -7.0, angle: 90.0, ..Default::default() }],
                background: Some(Background { shapes: vec![Shape::Path(Path::rect(1.0, 3.0, 5.0, 7.0))], ..Default::default() }),
                ..Default::default()
            });
        }
        {
            let mut g = font.glyph_mut("B").unwrap();
            let l = g.layer_mut("m01").unwrap();
            let mut c = Component::new("A", 13.0, -9.0);
            if let Transform::Decomposed { scale, .. } = &mut c.transform {
                *scale = (0.5, 0.5);
            }
            l.shapes.push(Shape::Component(c));
        }
        font.stems = Arc::new(vec![StemDef { name: "Stem".into(), ..Default::default() }]);
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.stem_values = vec![83.0];
        m.zones = vec![AlignmentZone { pos: 550.0, size: 13.0 }];
        font.kerning = Arc::new(fontcraft_model::Kerning {
            ltr: KernTable(vec![MasterKerning {
                master: "m01".into(),
                firsts: vec![KernFirst { first: "A".into(), seconds: vec![("B".into(), -37.0)] }],
            }]),
            ..Default::default()
        });
        font
    }

    #[test]
    fn scale_font_doubles_outlines_and_metrics_and_keeps_component_scales() {
        let font = font();
        let big = scale_font(&font, 2000).unwrap();
        assert_eq!(big.units_per_em, 2000);
        let a = big.glyph("A").unwrap().layer("m01").unwrap();
        assert_eq!(a.width, 1200.0);
        assert_eq!(a.anchors[0].x, 602.0);
        assert_eq!(a.info.guides[0].x, 22.0);
        assert_eq!(a.info.guides[0].angle, 90.0);
        let Shape::Component(c) = &big.glyph("B").unwrap().layer("m01").unwrap().shapes[0] else { panic!("component") };
        let Transform::Decomposed { pos, scale, .. } = c.transform else { panic!("decomposed") };
        assert_eq!((pos, scale), ((26.0, -18.0), (0.5, 0.5)));
        assert_eq!(big.masters[0].stem_values, vec![166.0]);
        assert_eq!(big.masters[0].metric_values[0].pos, 1600.0);
        assert_eq!(big.kerning.ltr.get("m01", "A", "B"), Some(-74.0));
    }

    #[test]
    fn scale_font_round_trip_restores_whole_values() {
        let font = font();
        let back = scale_font(&scale_font(&font, 2048).unwrap(), 1000).unwrap();
        assert!(back == font, "1000 → 2048 → 1000 changed the font");
    }

    #[test]
    fn scale_font_refuses_coordinates_past_the_limit_and_bad_targets() {
        let mut font = font();
        font.glyph_mut("C").unwrap().layer_mut("m01").unwrap().anchors.push(Anchor::new("far", 5000.0, 0.0));
        assert!(matches!(scale_font(&font, 16384), Err(ScaleError::OutOfRange { .. })));
        assert!(matches!(scale_font(&font, 8), Err(ScaleError::UnitsPerEm(8))));
    }

    #[test]
    fn scale_font_scales_units_parameters_and_hint_places() {
        let mut font = font();
        let info = Arc::make_mut(&mut font.info);
        info.params.0.push(Param::new("typoAscender", Value::Number(Number::int(100))));
        info.params.0.push(Param::new("Scale to UPM", Value::Number(Number::int(2048))));
        let mut place = fontcraft_plist::Dict::new();
        place.insert("place", Value::String(Str::new("{10, 21}")));
        let hint = Hint { family: fontcraft_model::Family::Glyphs, value: Value::Dict(place) };
        font.glyph_mut("A").unwrap().layer_mut("m01").unwrap().info.hints.push(hint);
        let big = scale_font(&font, 2000).unwrap();
        assert_eq!(big.info.params.get("typoAscender"), Some(&Value::Number(Number::int(200))));
        assert_eq!(big.info.params.get("Scale to UPM"), Some(&Value::Number(Number::int(2048))));
        let h = &big.glyph("A").unwrap().layer("m01").unwrap().info.hints[0];
        assert_eq!(h.value.as_dict().unwrap().get("place").and_then(Value::as_str), Some("{20, 42}"));
    }
}
