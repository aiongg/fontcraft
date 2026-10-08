//! The compiler's view of a font: one master or instance, resolved.
//!
//! [`Font::resolve`] gives what every table builder of a compiler reads, and nothing else: the
//! glyphs in the font's order with the drawing of each at one location (contours as Bézier paths,
//! components as glyph name and transform), the metrics and names that apply there, and the font
//! information with the master's and instance's own properties and parameters in place of the
//! font's. It reads no [`crate::Extra`], keeps no node types, layers, guides or other editing data,
//! and is one-way: nothing is written back from it (`docs/spec/30-source-formats.md` › Compiler
//! view).
//!
//! Which drawing a glyph has at the location:
//!
//! | Target | Layer |
//! |---|---|
//! | a master | the glyph's master layer for it |
//! | an instance at a master's location | that master's layer |
//! | an instance between masters | none: an error until interpolation lands (M9) |
//!
//! Alternate layers (the ones with axis rules) and intermediate layers are not used: at a
//! master's location the master layer is the drawing, and alternate layers become separate
//! glyphs with feature variations, which compiling does not do yet. The result's notes say when
//! a font has any.

use fontcraft_geom::{Affine, BezPath, Point};

use crate::{Font, Master, MetricValue, ModelError, Params, Properties, Shape, metric};

/// What to resolve.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The master with this id.
    Master(String),
    /// The instance at this index of [`Font::instances`]. It must sit exactly at a master.
    Instance(usize),
}

/// One master or instance of a font, as a compiler reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub units_per_em: u16,
    pub family_name: String,
    /// The master's name, or the instance's.
    pub style_name: String,
    pub version_major: u16,
    pub version_minor: u16,
    /// The id of the master whose layers are drawn.
    pub master: String,
    /// The design location, axis by axis in the font's axis order.
    pub location: Vec<f64>,
    pub metrics: Metrics,
    /// The font's properties, with the master's and then the instance's of the same key in their
    /// place.
    pub properties: Properties,
    /// The font's enabled parameters, with the master's and then the instance's of the same name
    /// in their place.
    pub params: Params,
    /// The font's creation date, as the source states it.
    pub date: Option<String>,
    /// Style linking of the instance (none for a master).
    pub style: Option<StyleLinking>,
    /// Every glyph of the font, in the font's order, exported or not.
    pub glyphs: Vec<ResolvedGlyph>,
    /// What resolving left out or chose, for the compile report.
    pub notes: Vec<String>,
}

/// The metrics the font defines at the location. `None` where the font has no definition of
/// that metric; the compiler's defaults then apply.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub ascender: Option<f64>,
    pub descender: Option<f64>,
    pub x_height: Option<f64>,
    pub cap_height: Option<f64>,
    pub italic_angle: Option<f64>,
}

/// An instance's style-linking settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleLinking {
    pub is_bold: bool,
    pub is_italic: bool,
    pub link_style: Option<String>,
}

/// One glyph at the location.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGlyph {
    pub name: String,
    pub unicodes: Vec<u32>,
    /// Written to compiled fonts?
    pub export: bool,
    /// The production name the glyph states, if any.
    pub production_name: Option<String>,
    pub width: f64,
    /// The contours in shape order, each a closed or open Bézier path in font units. A closed
    /// contour that starts with a line ends with an explicit line back to its start
    /// ([`crate::Path::to_contour`]).
    pub contours: Vec<BezPath>,
    /// The components in shape order.
    pub components: Vec<ResolvedComponent>,
    pub anchors: Vec<(String, Point)>,
}

/// A reference to another glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedComponent {
    pub base: String,
    pub transform: Affine,
}

impl ResolvedGlyph {
    /// A glyph with no drawing.
    pub fn new(name: impl Into<String>, width: f64) -> Self {
        Self {
            name: name.into(),
            unicodes: Vec::new(),
            export: true,
            production_name: None,
            width,
            contours: Vec::new(),
            components: Vec::new(),
            anchors: Vec::new(),
        }
    }
}

/// How close two design coordinates must be to count as the same location.
const SAME_LOCATION: f64 = 1e-9;

impl Font {
    /// The master that a static build of this font uses when nothing else is asked for: the
    /// first one.
    pub fn default_target(&self) -> Result<Target, ModelError> {
        Ok(Target::Master(self.default_master()?.id.clone()))
    }

    /// The font as a compiler reads it at `target` (see the module notes).
    ///
    /// The font is assumed valid ([`Font::check`]); a path that is not is still an error, not a
    /// panic.
    pub fn resolve(&self, target: &Target) -> Result<Resolved, ModelError> {
        let mut notes = Vec::new();
        let (master, instance) = match target {
            Target::Master(id) => (self.master(id)?, None),
            Target::Instance(i) => {
                let inst = self.instances.get(*i).ok_or(ModelError::NoSuchInstance(*i))?;
                let label = if inst.name.is_empty() { format!("#{i}") } else { inst.name.clone() };
                if inst.kind.is_some() {
                    return Err(ModelError::NotStatic(label));
                }
                let at = |m: &&Master| {
                    (0..self.axes.len().max(m.location.len()).max(inst.location.len()))
                        .all(|a| (axis_value(&m.location, a) - axis_value(&inst.location, a)).abs() <= SAME_LOCATION)
                };
                let master = self.masters.iter().find(at).ok_or(ModelError::BetweenMasters(label))?;
                (master, Some(inst))
            }
        };

        let mut properties = self.info.properties.clone();
        let mut params = Params(self.info.params.0.iter().filter(|p| !p.disabled).cloned().collect());
        overlay(&mut properties, &mut params, &master.properties, &master.params);
        if let Some(inst) = instance {
            overlay(&mut properties, &mut params, &inst.properties, &inst.params);
        }
        let style_name = match instance {
            Some(inst) if !inst.name.is_empty() => inst.name.clone(),
            _ => master.name.clone(),
        };
        let pos = |kind: &str| self.metric(master, kind).map(|v: MetricValue| v.pos);
        let metrics = Metrics {
            ascender: pos(metric::ASCENDER),
            descender: pos(metric::DESCENDER),
            x_height: pos(metric::X_HEIGHT),
            cap_height: pos(metric::CAP_HEIGHT),
            italic_angle: pos(metric::ITALIC_ANGLE),
        };

        let mut glyphs = Vec::with_capacity(self.glyphs.len());
        let (mut alternates, mut undrawable, mut missing) = (0usize, 0usize, Vec::new());
        for g in self.glyphs.iter() {
            let bad = |message: String| ModelError::BadOutline { glyph: g.name.clone(), message };
            let mut out = ResolvedGlyph::new(g.name.clone(), 0.0);
            out.unicodes.clone_from(&g.unicodes);
            out.export = g.export;
            out.production_name.clone_from(&g.info.production);
            if g.layers.iter().any(|l| l.master == master.id && !l.is_master_layer() && !l.info.attrs.axis_rules.is_empty()) {
                alternates += 1;
            }
            let Some(layer) = g.layer(&master.id) else {
                if g.export {
                    missing.push(g.name.clone());
                }
                glyphs.push(out);
                continue;
            };
            out.width = layer.width;
            for shape in &layer.shapes {
                match shape {
                    Shape::Path(p) => {
                        let contour = p.to_contour().map_err(bad)?;
                        if !contour.elements().is_empty() {
                            out.contours.push(contour);
                        }
                    }
                    Shape::Component(c) => {
                        if c.transform.numbers().iter().any(|v| !v.is_finite()) {
                            return Err(bad("a component transform is not a number".into()));
                        }
                        let transform = c.affine();
                        if !transform.as_coeffs().iter().all(|v| v.is_finite()) {
                            return Err(bad("a component transform is not a number".into()));
                        }
                        out.components.push(ResolvedComponent { base: c.glyph.clone(), transform });
                    }
                    Shape::Image(_) => {}
                    Shape::Raw(_) => undrawable += 1,
                }
            }
            out.anchors = layer.anchors.iter().map(|a| (a.name.clone(), Point::new(a.x, a.y))).collect();
            glyphs.push(out);
        }
        if alternates > 0 {
            notes.push(format!(
                "{alternates} glyphs have alternate layers for master `{}`; their master layers were used (alternate layers become glyphs of their own with feature variations, which compiling does not do yet)",
                master.name
            ));
        }
        if undrawable > 0 {
            notes.push(format!("{undrawable} shapes that only their source format can describe (shape groups, unusual node types) were left out"));
        }
        if !missing.is_empty() {
            let shown: Vec<&str> = missing.iter().take(5).map(String::as_str).collect();
            notes.push(format!(
                "{} glyphs have no layer for master `{}` and were written empty: {}{}",
                missing.len(),
                master.name,
                shown.join(", "),
                if missing.len() > shown.len() { ", …" } else { "" }
            ));
        }

        Ok(Resolved {
            units_per_em: self.units_per_em,
            family_name: self.family_name.clone(),
            style_name,
            version_major: self.version_major,
            version_minor: self.version_minor,
            master: master.id.clone(),
            location: instance.map_or_else(|| master.location.clone(), |i| i.location.clone()),
            metrics,
            properties,
            params,
            date: self.info.date.clone(),
            style: instance.map(|i| StyleLinking { is_bold: i.is_bold, is_italic: i.is_italic, link_style: i.link_style.clone() }),
            glyphs,
            notes,
        })
    }
}

/// A location's value on axis `i`: the reference's defaults where the location is short.
fn axis_value(location: &[f64], i: usize) -> f64 {
    crate::axes::design_value(location, i)
}

/// Puts `own` properties and enabled parameters in place of those of the same key or name.
fn overlay(properties: &mut Properties, params: &mut Params, own_properties: &Properties, own_params: &Params) {
    for p in &own_properties.0 {
        match properties.0.iter_mut().find(|q| q.key == p.key) {
            Some(q) => *q = p.clone(),
            None => properties.0.push(p.clone()),
        }
    }
    let own: Vec<&crate::Param> = own_params.0.iter().filter(|p| !p.disabled).collect();
    params.0.retain(|p| !own.iter().any(|o| o.name == p.name));
    params.0.extend(own.into_iter().cloned());
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_geom::PathEl;
    use fontcraft_plist::Value;

    use super::*;
    use crate::{AxisRule, Component, Instance, Layer, Param, Path};

    fn int(i: i64) -> Value {
        Value::Number(fontcraft_plist::Number::int(i))
    }

    fn font() -> Font {
        let mut f = Font::new("Resolve");
        f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 20.0)));
        f
    }

    #[test]
    fn a_master_resolves_to_its_layers_metrics_and_names() {
        let f = font();
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert_eq!((r.style_name.as_str(), r.master.as_str(), r.units_per_em), ("Regular", "m01", 1000));
        assert_eq!(r.metrics.ascender, Some(800.0));
        assert_eq!(r.metrics.italic_angle, None, "no definition: the compiler's default applies");
        assert_eq!(r.glyphs.len(), f.glyphs.len());
        let a = r.glyphs.iter().find(|g| g.name == "A").unwrap();
        assert_eq!((a.width, a.unicodes.as_slice(), a.contours.len()), (600.0, [0x41].as_slice(), 1));
        // The closing line is explicit: four lines, the last back to the start.
        let lines = a.contours[0].elements().iter().filter(|e| matches!(e, PathEl::LineTo(_))).count();
        assert_eq!(lines, 4);
        let b = r.glyphs.iter().find(|g| g.name == "B").unwrap();
        assert_eq!(b.components, [ResolvedComponent { base: "A".into(), transform: Affine::translate((10.0, 20.0)) }]);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert!(matches!(f.resolve(&Target::Master("nope".into())), Err(ModelError::NoSuchMaster(_))));
    }

    #[test]
    fn an_instance_takes_its_masters_layers_and_overrides_the_information() {
        let mut f = font();
        Arc::make_mut(&mut f.info).properties.set("designer", "Font", false);
        Arc::make_mut(&mut f.info).properties.set("copyright", "Font", false);
        Arc::make_mut(&mut f.info).params.0.push(Param::new("underlinePosition", int(-100)));
        Arc::make_mut(&mut f.info).params.0.push(Param::new("fsType", int(8)));
        let mut off = Param::new("winAscent", int(1));
        off.disabled = true;
        Arc::make_mut(&mut f.info).params.0.push(off);
        let master = &mut Arc::make_mut(&mut f.masters)[0];
        master.properties.set("designer", "Master", false);
        master.params.0.push(Param::new("underlinePosition", int(-150)));
        let mut inst = Instance { name: "Display".into(), is_bold: true, ..Default::default() };
        inst.properties.set("copyright", "Instance", false);
        inst.params.0.push(Param::new("fsType", int(0)));
        Arc::make_mut(&mut f.instances).push(inst);
        let r = f.resolve(&Target::Instance(0)).unwrap();
        assert_eq!(r.style_name, "Display");
        assert_eq!((r.properties.value("designer"), r.properties.value("copyright")), (Some("Master"), Some("Instance")));
        assert_eq!(r.params.get("underlinePosition"), Some(&int(-150)));
        assert_eq!(r.params.get("fsType"), Some(&int(0)));
        assert_eq!(r.params.get("winAscent"), None, "disabled parameters are not compiled");
        assert_eq!(r.style.as_ref().map(|s| s.is_bold), Some(true));

        // A variable font's settings are no static target.
        Arc::make_mut(&mut f.instances).push(Instance { name: "VF".into(), kind: Some("variable".into()), ..Default::default() });
        assert!(matches!(f.resolve(&Target::Instance(1)), Err(ModelError::NotStatic(n)) if n == "VF"));
    }

    #[test]
    fn layers_that_are_not_compiled_are_noted() {
        let mut f = font();
        let mut g = f.glyph_mut("A").unwrap();
        let mut alt = Layer::new("m01", 600.0);
        alt.id = "alt".into();
        alt.info.attrs.axis_rules = vec![AxisRule { min: Some(500.0), max: None }];
        g.layers.push(alt);
        drop(g);
        f.glyph_mut("C").unwrap().layers.clear();
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert!(r.notes.iter().any(|n| n.contains("alternate layers")), "{:?}", r.notes);
        assert!(r.notes.iter().any(|n| n.contains("no layer") && n.contains('C')), "{:?}", r.notes);
        let a = r.glyphs.iter().find(|g| g.name == "A").unwrap();
        assert_eq!(a.contours.len(), 1, "the master layer, not the alternate");
    }
}
