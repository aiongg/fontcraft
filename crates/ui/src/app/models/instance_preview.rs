//! The preview of the first selected static instance on the Exports tab: the glyphs its Instance Preview parameter
//! lists, else those of `Aang126`, in the instance's own outlines, side by side without kerning. The outlines are the
//! engine's kept preview of the instance (`SessionInterpExt::instance_view`), made again only when the font changes.

use craft::ui::contract::PlainData;
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::interp_cache::SessionInterpExt as _;
use fontcraft_engine::model::{Font, metric};
use fontcraft_geom::{Affine, BezPath};

use super::ProjectCx;

/// The glyphs shown without an Instance Preview parameter, as characters.
const DEFAULT_PREVIEW: &str = "Aang126";
const INSTANCE_PREVIEW: &str = "Instance Preview";
/// Most glyphs shown.
const MAX_GLYPHS: usize = 32;
/// Most elements the preview's outline has; the glyphs past it are left out.
const MAX_ELEMENTS: usize = 50_000;

/// The preview: one outline in font units, its advance width and the heights it lies between.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct InstancePreview {
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    pub width: f64,
    pub top: f64,
    pub bottom: f64,
}

/// The glyphs the preview of the instance at `index` shows: those of its parameter the font has, else of
/// [`DEFAULT_PREVIEW`].
pub fn preview_glyphs(font: &Font, index: usize) -> Vec<String> {
    let listed = font.instances.get(index).and_then(|i| i.params.get(INSTANCE_PREVIEW)).and_then(|v| {
        let lines = fv::lines(v)?;
        let names = lines.iter().flat_map(|l| l.split([',', ' '])).map(str::trim).filter(|n| !n.is_empty());
        Some(names.filter(|n| font.glyph(n).is_some()).take(MAX_GLYPHS).map(str::to_string).collect::<Vec<_>>())
    });
    if let Some(listed) = listed.filter(|l| !l.is_empty()) {
        return listed;
    }
    let of = |c: char| font.glyphs.iter().find(|g| g.unicodes.contains(&u32::from(c))).map(|g| g.name.clone());
    DEFAULT_PREVIEW.chars().filter_map(of).collect()
}

/// The preview of the static instance at `index`; `None` for a variable font setting, an instance that cannot be
/// interpolated, or a font without the glyphs.
pub fn project(cx: &mut ProjectCx<'_>, index: usize) -> Option<InstancePreview> {
    let font = cx.session.font().ok()?;
    if font.instances.get(index)?.kind.is_some() {
        return None;
    }
    let glyphs = preview_glyphs(font, index);
    let upm = f64::from(font.units_per_em);
    if glyphs.is_empty() {
        return None;
    }
    let view = cx.session.instance_view(index, &glyphs).ok()?;
    let master = view.masters.first()?;
    let top = view.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let bottom = view.metric_pos(master, metric::DESCENDER, upm * -0.2);
    let mut outline = BezPath::new();
    let mut x = 0.0;
    for name in &glyphs {
        let Some(layer) = view.glyph(name).and_then(|g| g.layer(&master.id)) else { continue };
        if let Ok(path) = view.outline(name, &master.id)
            && outline.elements().len().saturating_add(path.elements().len()) <= MAX_ELEMENTS
        {
            outline.extend((Affine::translate((x, 0.0)) * path).iter());
        }
        if layer.width.is_finite() {
            x += layer.width.max(0.0);
        }
    }
    let sound = outline.elements().len() > 1 && x > 0.0 && top > bottom && [x, top, bottom].iter().all(|v| v.is_finite());
    sound.then_some(InstancePreview { outline, width: x, top, bottom })
}
