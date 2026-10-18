//! The `SVG ` table (M11.9 from paints, M11.10 from svg layers and bitmaps): one uncompressed
//! document per glyph, built by `fontcraft_color::svg` and `fontcraft_color::svg_import`.
//!
//! Rows: `SVG `, `color.export.exportSvgTable`, `color.export.fullColorToSvg`,
//! `color.export.sbixToSvg`, `color.svg.threeSources`.
//!
//! A glyph whose document cannot be made is left out of the table with a warning; its master
//! layer outline stays the fallback that renderers without SVG support draw. Only a table
//! beyond its size limit fails the compile.

use std::cell::RefCell;
use std::collections::HashSet;

use fontcraft_color::export::{SvgPlan, SvgSource};
use fontcraft_color::paint::{Paint, ShapeRef, StrokeSpec};
use fontcraft_color::svg::{BitmapPlacement, bitmap_document, bitmap_size, document, has_sweep, is_native_stroke, table};
use fontcraft_color::svg_import::import;
use fontcraft_color::{AssetStore, MAX_SVG_FILE_BYTES, Rgba};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_imagery::{ImageFormat, MAX_PIXELS, decode_rgba, sniff, to_png};
use fontcraft_model::{Font, Glyph, Image, MAX_OUTLINE_ELEMENTS, Shape};
use fontcraft_pathops::{Cap, Join, StrokeStyle, stroke_outline};

use super::GlyphOrder;
use crate::CompileError;
use crate::drawing::shapes_outline;

/// The table; `None` when no glyph has a document, which leaves the font without one.
/// `font` gives the outlines of the paints' shapes and the svg layers' placements.
pub fn build(
    font: &Font,
    plan: &SvgPlan,
    order: &GlyphOrder,
    assets: &dyn AssetStore,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<u8>>, CompileError> {
    let mut docs: Vec<(u16, u16, Vec<u8>)> = Vec::with_capacity(plan.documents.len());
    let mut done = HashSet::new();
    for doc in &plan.documents {
        // A glyph that is not exported has no glyph id and so no document.
        let Some(gid) = order.gid(&doc.glyph) else { continue };
        if !done.insert(gid) {
            warnings.push(format!("glyph `{}`: it has more than one SVG document; the first is kept", doc.glyph));
            continue;
        }
        let made = match &doc.source {
            SvgSource::Paint(paint) => paint_document(font, &doc.glyph, paint, gid, &plan.palette, warnings),
            SvgSource::SvgLayer { layer, path } => svg_layer_document(font, &doc.glyph, layer, path, gid, assets),
            SvgSource::Bitmap { layer, ppem } => bitmap_layer_document(font, &doc.glyph, layer, *ppem, gid, assets),
        };
        match made {
            Ok(text) => docs.push((gid, gid, text.into_bytes())),
            Err(message) => warnings.push(format!("glyph `{}`: {message}; it has no SVG document", doc.glyph)),
        }
    }
    if docs.is_empty() {
        if !plan.documents.is_empty() {
            warnings.push("no glyph has an SVG document, so the font has no SVG table".to_string());
        }
        return Ok(None);
    }
    table(&docs).map(Some).map_err(|e| CompileError::Table { table: "SVG ", message: e.to_string() })
}

/// The document of `name` drawing `paint`, or why it cannot be made.
fn paint_document(font: &Font, name: &str, paint: &Paint, gid: u16, palette: &[Rgba], warnings: &mut Vec<String>) -> Result<String, String> {
    let glyph = font.glyph(name).ok_or_else(|| format!("there is no glyph called `{name}`"))?;
    let failure: RefCell<Option<String>> = RefCell::new(None);
    let draw = |shape: &ShapeRef| match shape_outline(font, glyph, shape) {
        Ok(path) => path,
        Err(e) => {
            failure.borrow_mut().get_or_insert(e);
            BezPath::new()
        }
    };
    let text = document(paint, &draw, gid, palette).map_err(|e| e.to_string())?;
    if let Some(e) = failure.into_inner() {
        return Err(e);
    }
    if has_sweep(paint) {
        warnings.push(format!("glyph `{name}`: SVG has no conic gradient, so its conic gradient is drawn in its first colour"));
    }
    Ok(text)
}

/// The document of `name` drawing the SVG file of its svg layer `layer`, placed as the layer
/// places it, or why it cannot be made.
fn svg_layer_document(font: &Font, name: &str, layer: &str, path: &str, gid: u16, assets: &dyn AssetStore) -> Result<String, String> {
    let image = layer_image(font, name, layer)?;
    let transform = image.map_or(Affine::IDENTITY, |i| i.transform.affine());
    // Reading one byte past the limit is enough to refuse a larger file without reading it all.
    let bytes = assets.read_head(path, MAX_SVG_FILE_BYTES + 1).map_err(|e| e.to_string())?;
    import(&bytes, transform, gid).map_err(|e| e.to_string())
}

/// The document of `name` showing the image of its iColor layer `layer` (SBIX to SVG at `ppem`
/// pixels per em), placed as the layer places it, or why it cannot be made.
fn bitmap_layer_document(font: &Font, name: &str, layer: &str, ppem: u16, gid: u16, assets: &dyn AssetStore) -> Result<String, String> {
    let image = layer_image(font, name, layer)?.ok_or_else(|| format!("its iColor layer of {ppem} pixels per em has no image"))?;
    let path = image.path.as_deref().ok_or_else(|| format!("its iColor layer of {ppem} pixels per em has no image"))?;
    let mut bytes = assets.read(path).map_err(|e| e.to_string())?;
    // SVG renderers draw PNG and JPEG only, so a TIFF goes in as PNG.
    if sniff(&bytes).is_ok_and(|info| matches!(info.format, ImageFormat::Tiff)) {
        let png = decode_rgba(&bytes, MAX_PIXELS)
            .and_then(|image| to_png(&image))
            .map_err(|e| format!("the TIFF image `{path}` cannot be converted to PNG: {e}"))?;
        bytes = png.into();
    }
    let (width, height) =
        bitmap_size(&bytes).ok_or_else(|| format!("the image `{path}` is not PNG or JPEG data, which is all SBIX to SVG writes"))?;
    let placement = BitmapPlacement { transform: image.transform.affine(), width, height };
    bitmap_document(&bytes, &placement, gid).map_err(|e| e.to_string())
}

/// The placed image of `layer` in glyph `name`.
fn layer_image<'f>(font: &'f Font, name: &str, layer: &str) -> Result<Option<&'f Image>, String> {
    let glyph = font.glyph(name).ok_or_else(|| format!("there is no glyph called `{name}`"))?;
    let layer = glyph.layers.iter().find(|l| l.id == layer).ok_or_else(|| format!("there is no layer `{layer}`"))?;
    Ok(layer.info.image.as_ref())
}

/// The outline of `shape` in font units, as the SVG writer draws it: the shapes' contours for a
/// fill, components resolved on the layer's master; for a stroke the paths' centre lines when
/// [`is_native_stroke`] says SVG strokes it, else the stroke expanded to an outline.
pub fn shape_outline(font: &Font, glyph: &Glyph, shape: &ShapeRef) -> Result<BezPath, String> {
    let layer = glyph.layers.iter().find(|l| l.id == shape.layer).ok_or_else(|| format!("there is no layer `{}`", shape.layer))?;
    let mut pieces: Vec<BezPath> = Vec::new();
    let mut components: Vec<Shape> = Vec::new();
    for &i in &shape.shapes {
        let s = layer.shapes.get(i).ok_or_else(|| format!("layer `{}` has no shape {i}", shape.layer))?;
        match (s, &shape.stroke) {
            (Shape::Path(p), None) => pieces.push(p.to_bez()?),
            (Shape::Path(p), Some(spec)) => {
                let path = p.to_bez()?;
                if is_native_stroke(spec) {
                    pieces.push(path);
                } else {
                    pieces.extend(stroke_outline(&path, &stroke_style(spec)).map_err(|e| e.to_string())?);
                }
            }
            (Shape::Component(_), None) => components.push(s.clone()),
            // Components have no stroke of their own; images and unknown shapes draw nothing.
            (Shape::Component(_), Some(_)) | (Shape::Image(_), _) | (Shape::Raw(_), _) => {}
        }
    }
    if !components.is_empty() {
        pieces.extend(shapes_outline(font, &components, &layer.master)?);
    }
    let mut out = BezPath::new();
    let mut budget = MAX_OUTLINE_ELEMENTS;
    for piece in pieces {
        budget = budget.checked_sub(piece.elements().len()).ok_or_else(|| format!("the outline has more than {MAX_OUTLINE_ELEMENTS} elements"))?;
        for el in piece.elements() {
            out.push(*el);
        }
    }
    Ok(out)
}

/// The pen [`stroke_outline`] expands `spec` with.
fn stroke_style(spec: &StrokeSpec) -> StrokeStyle {
    StrokeStyle {
        width: spec.width,
        height: spec.height,
        position: spec.position,
        join: Join::from_code(spec.join).unwrap_or_default(),
        cap_start: Cap::from_code(spec.cap_start).unwrap_or_default(),
        cap_end: Cap::from_code(spec.cap_end).unwrap_or_default(),
        ..StrokeStyle::new(spec.width)
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_color::export::{SvgDocument, SvgPlan, SvgSource};
    use fontcraft_color::paint::{ColorRef, Paint, ShapeRef, StrokeSpec};
    use fontcraft_color::{MemoryAssets, Rgba};
    use fontcraft_model::{Font, Node, NodeKind, Path, ResolvedGlyph, Shape};
    use skrifa::GlyphId;
    use skrifa::raw::tables::svg::Svg;
    use skrifa::raw::{FontData, FontRead};

    use super::build;
    use crate::color::GlyphOrder;

    #[test]
    fn svg_table_from_paints_reads_back_with_one_document_per_glyph() {
        let mut font = Font::new("Svg Sans");
        font.add_glyph("heart", vec![0x2665], 600.0).unwrap();
        let master = font.masters[0].id.clone();
        {
            let mut g = font.glyph_mut("heart").unwrap();
            let layer = g.layers.iter_mut().find(|l| l.id == master).unwrap();
            layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
            let n = |x, y| Node::new(x, y, NodeKind::Line);
            layer.shapes.push(Shape::Path(Path::new(false, vec![n(0.0, 200.0), n(300.0, 200.0)])));
        }
        let fill = ShapeRef { layer: master.clone(), shapes: vec![0], stroke: None };
        // An outside stroke has no SVG form: it is expanded and filled.
        let outside = StrokeSpec { width: 20.0, height: 20.0, position: 1.0, cap_start: 0, cap_end: 0, join: 0 };
        let stroke = ShapeRef { layer: master.clone(), shapes: vec![1], stroke: Some(outside) };
        let glyph = |shape, color| Paint::Glyph { shape, paint: Box::new(Paint::Solid(color)) };
        let paint = Paint::Layers(vec![glyph(fill, ColorRef::Palette { index: 0, alpha: 1.0 }), glyph(stroke, ColorRef::Rgba([0.0, 0.0, 1.0, 1.0]))]);
        let plan = SvgPlan {
            palette: vec![Rgba::new(255, 0, 0, 255)],
            documents: vec![
                SvgDocument { glyph: "heart".into(), source: SvgSource::Paint(paint) },
                SvgDocument { glyph: "space".into(), source: SvgSource::SvgLayer { layer: master.clone(), path: "space.svg".into() } },
                SvgDocument { glyph: "unexported".into(), source: SvgSource::Paint(Paint::Layers(Vec::new())) },
            ],
        };
        let glyphs: Vec<ResolvedGlyph> = [".notdef", "space", "heart"].into_iter().map(|n| ResolvedGlyph::new(n, 600.0)).collect();
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let mut warnings = Vec::new();
        let bytes = build(&font, &plan, &order, &MemoryAssets::new(), &mut warnings).unwrap().unwrap();

        let svg = Svg::read(FontData::new(&bytes)).unwrap();
        assert_eq!(svg.svg_document_list().unwrap().num_entries(), 1);
        assert!(svg.glyph_data(GlyphId::new(1)).is_none());
        let doc = std::str::from_utf8(svg.glyph_data(GlyphId::new(2)).unwrap()).unwrap();
        assert!(doc.contains(r#"<g id="glyph2" transform="matrix(1 0 0 -1 0 0)">"#), "{doc}");
        assert!(doc.contains(r##"<path d="M0 0L100 0L100 100L0 100Z" fill="#FF0000"/>"##), "{doc}");
        assert!(doc.contains(r##"fill="#0000FF"/>"##) && !doc.contains("stroke="), "{doc}");
        assert!(warnings.iter().any(|w| w.starts_with("glyph `space`")), "{warnings:?}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        // No document can be made: no table, rather than an empty one.
        let failing = SvgPlan { palette: Vec::new(), documents: plan.documents.into_iter().filter(|d| d.glyph == "space").collect() };
        let mut warnings = Vec::new();
        assert_eq!(build(&font, &failing, &order, &MemoryAssets::new(), &mut warnings).unwrap(), None);
        assert!(warnings.iter().any(|w| w.contains("no SVG table")), "{warnings:?}");
    }

    #[test]
    fn svg_table_from_svg_layers_and_icolor_bitmaps() {
        use fontcraft_model::{Image, Layer, Transform};

        let mut font = Font::new("Svg Layers");
        let master = font.masters[0].id.clone();
        let mut add = |name: &str, layer_id: &str, path: &str, transform: Transform| {
            font.add_glyph(name, Vec::new(), 600.0).unwrap();
            let mut g = font.glyph_mut(name).unwrap();
            let mut layer = Layer::new(master.clone(), 600.0);
            layer.id = layer_id.into();
            layer.info.image = Some(Image { path: Some(path.into()), transform, ..Image::default() });
            g.layers.push(layer);
        };
        add("smile", "svg1", "smile.svg", Transform::Matrix([2.0, 0.0, 0.0, 2.0, 100.0, -50.0]));
        add("rainbow", "ic128", "rainbow.png", Transform::Matrix([7.8125, 0.0, 0.0, 7.8125, 0.0, -100.0]));
        add("frown", "ic64", "frown.png", Transform::default());
        add("tiff", "ic32", "tiff.tif", Transform::default());

        let mut assets = MemoryAssets::new();
        assets.insert(
            "smile.svg",
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4" fill="#fc0"/></svg>"##.to_vec(),
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 3, 8, 6, 0, 0, 0]);
        assets.insert("rainbow.png", png);
        assets.insert("tiff.tif", super::super::sbix::tests::tiff(1, 1, &[255, 0, 0, 255]));

        let bitmap = |glyph: &str, layer: &str, ppem| SvgDocument { glyph: glyph.into(), source: SvgSource::Bitmap { layer: layer.into(), ppem } };
        let plan = SvgPlan {
            palette: Vec::new(),
            documents: vec![
                SvgDocument { glyph: "smile".into(), source: SvgSource::SvgLayer { layer: "svg1".into(), path: "smile.svg".into() } },
                bitmap("rainbow", "ic128", 128),
                bitmap("frown", "ic64", 64),
                bitmap("tiff", "ic32", 32),
            ],
        };
        let glyphs: Vec<ResolvedGlyph> = [".notdef", "smile", "rainbow", "frown", "tiff"].into_iter().map(|n| ResolvedGlyph::new(n, 600.0)).collect();
        let order = GlyphOrder::new(&glyphs, &[]).unwrap();
        let mut warnings = Vec::new();
        let bytes = build(&font, &plan, &order, &assets, &mut warnings).unwrap().unwrap();

        let svg = Svg::read(FontData::new(&bytes)).unwrap();
        assert_eq!(svg.svg_document_list().unwrap().num_entries(), 3);
        let doc = |gid| std::str::from_utf8(svg.glyph_data(GlyphId::new(gid)).unwrap()).unwrap().to_string();
        let smile = doc(1);
        assert!(smile.contains(r#"<g id="glyph1" transform="matrix(1 0 0 -1 0 0)"><g transform="matrix(2 0 0 -2 100 -30)">"#), "{smile}");
        let rainbow = doc(2);
        assert!(rainbow.contains(r#"<image width="2" height="3" transform="matrix(7.8125 0 0 -7.8125 0 -76.5625)""#), "{rainbow}");
        assert!(rainbow.contains("data:image/png;base64,"), "{rainbow}");
        // A TIFF goes in as PNG; a missing file leaves its glyph out, with a warning.
        let tiff = doc(4);
        assert!(tiff.contains(r#"<image width="1" height="1""#) && tiff.contains("data:image/png;base64,"), "{tiff}");
        assert!(svg.glyph_data(GlyphId::new(3)).is_none());
        assert!(warnings.iter().any(|w| w.starts_with("glyph `frown`") && w.contains("frown.png")), "{warnings:?}");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
