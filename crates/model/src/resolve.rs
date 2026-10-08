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

use crate::{Font, Master, ModelError, Params, Properties, Shape, SourceFormat, metric};

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
    /// The format the font was read from. The two families of formats derive some table fields
    /// differently (the names, the style flags), as the reference compilers do.
    pub format: SourceFormat,
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
    /// The font's axes, with the location in user coordinates.
    pub axes: Vec<ResolvedAxis>,
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

/// The metrics at the location. Where the font has no value for one, the value its format means
/// takes its place ([`Metrics::defaults`]); the model itself keeps the value absent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub ascender: f64,
    pub descender: f64,
    pub x_height: f64,
    pub cap_height: f64,
    /// Clockwise, in degrees, as the reference's format states it (OpenType's is the negative).
    pub italic_angle: f64,
}

impl Metrics {
    /// What a font of `format` means when a master has no value for a metric. The `.glyphs`
    /// format (and the native one, shaped like it) means fixed values: ascender 800, descender
    /// −200, x-height 500, cap height 700 (glyphsLib's master defaults, which fontc's reader
    /// follows). A UFO means fractions of the em (ufo2ft's `fontInfoData`): 0.8, −0.2, 0.5 and
    /// 0.7. The italic angle is 0 in both.
    pub fn defaults(format: SourceFormat, units_per_em: u16) -> Self {
        let upm = f64::from(units_per_em);
        if is_ufo_family(format) {
            Self { ascender: 0.8 * upm, descender: -0.2 * upm, x_height: 0.5 * upm, cap_height: 0.7 * upm, italic_angle: 0.0 }
        } else {
            Self { ascender: 800.0, descender: -200.0, x_height: 500.0, cap_height: 700.0, italic_angle: 0.0 }
        }
    }
}

/// Whether `format` is a UFO or a designspace (whose masters are UFOs).
pub fn is_ufo_family(format: SourceFormat) -> bool {
    matches!(format, SourceFormat::Ufo | SourceFormat::Ufoz | SourceFormat::Designspace)
}

/// An axis of the font at the location.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAxis {
    /// The OpenType tag (the first four characters of a longer one).
    pub tag: String,
    /// The location on this axis, in user coordinates.
    pub user: f64,
    /// Whether the axis has a range (a variable font varies along it), not a single value.
    pub variable: bool,
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
    /// The category and subcategory the glyph states (`Mark`, `Nonspacing`…), if any. Those the
    /// glyph database would give are not looked up here.
    pub category: Option<String>,
    pub sub_category: Option<String>,
    pub width: f64,
    /// The vertical advance, when the layer states one.
    pub vert_width: Option<f64>,
    /// The vertical origin, measured down from the ascender, when the layer states one.
    pub vert_origin: Option<f64>,
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
            category: None,
            sub_category: None,
            width,
            vert_width: None,
            vert_origin: None,
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
        // A master's value list may be shorter than the definitions: a value it does not have is
        // absent, and the format's default applies (the editor shows it as zero).
        let defaults = Metrics::defaults(self.source.format, self.units_per_em);
        let pos = |kind: &str, default: f64| {
            self.metrics.iter().position(|d| d.is_plain(kind)).and_then(|i| master.metric_values.get(i)).map_or(default, |v| v.pos)
        };
        let metrics = Metrics {
            ascender: pos(metric::ASCENDER, defaults.ascender),
            descender: pos(metric::DESCENDER, defaults.descender),
            x_height: pos(metric::X_HEIGHT, defaults.x_height),
            cap_height: pos(metric::CAP_HEIGHT, defaults.cap_height),
            italic_angle: pos(metric::ITALIC_ANGLE, defaults.italic_angle),
        };
        let axes = (0..self.axes.len())
            .filter_map(|i| {
                let axis = self.axes.get(i)?;
                let location = instance.map_or(&master.location, |inst| &inst.location);
                let user = crate::axes::mapping(self, i).to_user(axis_value(location, i));
                let range = crate::axes::range(self, i);
                Some(ResolvedAxis { tag: axis.tag.chars().take(4).collect(), user, variable: range.minimum < range.maximum })
            })
            .collect();

        let mut glyphs = Vec::with_capacity(self.glyphs.len());
        let (mut alternates, mut undrawable, mut missing) = (0usize, 0usize, Vec::new());
        for g in self.glyphs.iter() {
            let bad = |message: String| ModelError::BadOutline { glyph: g.name.clone(), message };
            let mut out = ResolvedGlyph::new(g.name.clone(), 0.0);
            out.unicodes.clone_from(&g.unicodes);
            out.export = g.export;
            out.production_name.clone_from(&g.info.production);
            out.category.clone_from(&g.info.category);
            out.sub_category.clone_from(&g.info.sub_category);
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
            out.vert_width = layer.info.vert_width;
            out.vert_origin = layer.info.vert_origin;
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
            if !is_ufo_family(self.source.format) {
                apply_origin_anchor(&mut out);
            }
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
            format: self.source.format,
            units_per_em: self.units_per_em,
            family_name: self.family_name.clone(),
            style_name,
            version_major: self.version_major,
            version_minor: self.version_minor,
            master: master.id.clone(),
            location: instance.map_or_else(|| master.location.clone(), |i| i.location.clone()),
            axes,
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

/// The anchor that moves a layer's drawing (the reference's format; glyphsLib and fontc apply
/// it the same way).
pub const ORIGIN_ANCHOR: &str = "*origin";

/// Moves the drawing and the other anchors so that the `*origin` anchor's position becomes
/// (0, 0), and drops that anchor. The advance stays.
fn apply_origin_anchor(g: &mut ResolvedGlyph) {
    let Some(origin) = g.anchors.iter().find(|(n, _)| n == ORIGIN_ANCHOR).map(|(_, p)| *p) else { return };
    if !(origin.x.is_finite() && origin.y.is_finite()) {
        return;
    }
    let shift = Affine::translate((-origin.x, -origin.y));
    for c in &mut g.contours {
        c.apply_affine(shift);
    }
    for c in &mut g.components {
        c.transform = shift * c.transform;
    }
    g.anchors.retain(|(n, _)| n != ORIGIN_ANCHOR);
    for (_, p) in &mut g.anchors {
        *p = shift * *p;
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
        assert_eq!(r.metrics.ascender, 800.0);
        assert_eq!(r.metrics.italic_angle, 0.0, "no definition: the format's default");
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
    fn a_metric_the_master_has_no_value_for_is_the_formats_default() {
        let mut f = font();
        f.units_per_em = 2000;
        // The definitions stay; the master's list stops after the ascender.
        let i = f.metrics.iter().position(|d| d.kind == metric::ASCENDER).unwrap();
        Arc::make_mut(&mut f.masters)[0].metric_values.truncate(i + 1);
        Arc::make_mut(&mut f.masters)[0].metric_values[i].pos = 1500.0;
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert_eq!((r.metrics.ascender, r.metrics.descender, r.metrics.x_height, r.metrics.cap_height), (1500.0, -200.0, 500.0, 700.0));
        // A UFO means fractions of the em instead.
        f.source.format = SourceFormat::Ufo;
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert_eq!((r.metrics.ascender, r.metrics.descender, r.metrics.x_height, r.metrics.cap_height), (1500.0, -400.0, 1000.0, 1400.0));
        // A value of zero that the master states stays zero.
        Arc::make_mut(&mut f.masters)[0].metric_values[i].pos = 0.0;
        assert_eq!(f.resolve(&f.default_target().unwrap()).unwrap().metrics.ascender, 0.0);
    }

    #[test]
    fn an_origin_anchor_moves_the_drawing() {
        let mut f = font();
        let mut g = f.glyph_mut("B").unwrap();
        g.layers[0].anchors.push(crate::Anchor::new("*origin", -20.0, 5.0));
        g.layers[0].anchors.push(crate::Anchor::new("top", 100.0, 700.0));
        g.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
        drop(g);
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        let b = r.glyphs.iter().find(|g| g.name == "B").unwrap();
        assert_eq!(b.components[0].transform, Affine::translate((30.0, 15.0)));
        let first = b.contours[0].elements().first().and_then(|e| e.end_point()).unwrap();
        assert_eq!(first, Point::new(20.0, -5.0), "contours move by the anchor too");
        assert_eq!(b.anchors, [("top".to_string(), Point::new(120.0, 695.0))], "the origin anchor itself is gone");
        // A UFO has no such anchor: one of that name is just an anchor.
        f.source.format = SourceFormat::Ufo;
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert_eq!(r.glyphs.iter().find(|g| g.name == "B").unwrap().anchors.len(), 2);
    }

    #[test]
    fn axes_resolve_to_the_targets_user_location() {
        let mut f = font();
        Arc::make_mut(&mut f.axes).push(crate::Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        Arc::make_mut(&mut f.masters)[0].location = vec![700.0];
        let r = f.resolve(&f.default_target().unwrap()).unwrap();
        assert_eq!(r.axes, [ResolvedAxis { tag: "wght".into(), user: 700.0, variable: false }]);
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
