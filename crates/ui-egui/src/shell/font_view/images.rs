//! Cell outlines. egui fills only convex shapes, so each cell's glyph is rasterised by the
//! `render` crate into a small white mask texture, tinted when drawn. A texture is made once per
//! glyph, master and pixel size, and kept across edits of other glyphs; a frame makes at most
//! [`MAX_PER_FRAME`] of them and asks for another frame for the rest, so scrolling a large font
//! never stalls.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};
use fontcraft_geom::Affine;
use fontcraft_model::{Font, Glyph, Shape, metric};
use fontcraft_render::{Style, render_path};

/// Most textures kept; past this the cache starts again empty.
const MAX_ENTRIES: usize = 4_096;
/// Most textures made in one frame.
pub const MAX_PER_FRAME: usize = 400;
/// How deep components are followed to find the glyphs a cell depends on, and how many.
const MAX_DEPTH: usize = 8;
const MAX_DEPS: usize = 64;

/// How a glyph is placed in its cell: the vertical extent shown and the font's size.
#[derive(Clone, Copy, PartialEq)]
pub struct Frame {
    pub ascender: f64,
    pub descender: f64,
}

impl Frame {
    pub fn of(font: &Font, master: &str) -> Frame {
        let upm = f64::from(font.units_per_em);
        match font.master(master) {
            Ok(m) => {
                Frame { ascender: font.metric_pos(m, metric::ASCENDER, upm * 0.8), descender: font.metric_pos(m, metric::DESCENDER, -upm * 0.2) }
            }
            Err(_) => Frame { ascender: upm * 0.8, descender: -upm * 0.2 },
        }
    }
}

struct Entry {
    doc: (u64, u64),
    glyph: Arc<Glyph>,
    /// The glyphs its components draw, at any depth; `None` when there are too many to follow.
    deps: Option<Vec<Arc<Glyph>>>,
    frame: Frame,
    side: u32,
    /// `None`: the glyph draws nothing (no closed contour).
    texture: Option<TextureHandle>,
}

/// What [`CellImages::get`] found.
pub enum Cell {
    /// The texture, or `None` when the glyph draws nothing.
    Ready(Option<egui::TextureId>),
    /// This frame has made its share of textures already: draw it next frame.
    Later,
}

#[derive(Default)]
pub struct CellImages {
    entries: HashMap<(String, String), Entry>,
    made: usize,
}

impl CellImages {
    /// Call once per frame before drawing cells.
    pub fn start_frame(&mut self) {
        self.made = 0;
    }

    /// Textures made in this frame (tests read it).
    pub fn made(&self) -> usize {
        self.made
    }

    /// The texture of `glyph` in `master` for a square of `side` pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn get(&mut self, ctx: &egui::Context, doc: (u64, u64), font: &Font, glyph: &Arc<Glyph>, master: &str, frame: Frame, side: u32) -> Cell {
        let key = (glyph.name.clone(), master.to_string());
        // An edit elsewhere in the font keeps the texture when neither the glyph nor any glyph
        // its components draw changed.
        let fresh = self.entries.get(&key).is_some_and(|e| {
            e.side == side
                && e.frame == frame
                && (e.doc == doc
                    || (e.doc.0 == doc.0
                        && Arc::ptr_eq(&e.glyph, glyph)
                        && e.deps.as_ref().is_some_and(|deps| deps.iter().all(|d| font.glyphs.get(&d.name).is_some_and(|now| Arc::ptr_eq(now, d))))))
        });
        if !fresh {
            if self.made >= MAX_PER_FRAME {
                return Cell::Later;
            }
            self.made += 1;
            let texture = rasterise(ctx, font, glyph, master, frame, side);
            if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
                self.entries.clear();
            }
            let deps = dependencies(font, glyph, master);
            self.entries.insert(key.clone(), Entry { doc, glyph: glyph.clone(), deps, frame, side, texture });
        }
        let Some(entry) = self.entries.get_mut(&key) else { return Cell::Later };
        entry.doc = doc;
        entry.glyph = glyph.clone();
        Cell::Ready(entry.texture.as_ref().map(TextureHandle::id))
    }
}

/// The glyphs the components of `glyph`'s master layer draw, nested ones included. `None` past
/// [`MAX_DEPTH`] or [`MAX_DEPS`].
fn dependencies(font: &Font, glyph: &Glyph, master: &str) -> Option<Vec<Arc<Glyph>>> {
    let mut out: Vec<Arc<Glyph>> = Vec::new();
    let mut todo: Vec<(&Glyph, usize)> = vec![(glyph, 0)];
    while let Some((g, depth)) = todo.pop() {
        let Some(layer) = g.layer(master) else { continue };
        for shape in &layer.shapes {
            let Shape::Component(c) = shape else { continue };
            if depth >= MAX_DEPTH || out.len() >= MAX_DEPS {
                return None;
            }
            let Some(base) = font.glyphs.get(&c.glyph) else { continue };
            if out.iter().any(|d| Arc::ptr_eq(d, base)) {
                continue;
            }
            out.push(base.clone());
            todo.push((base, depth + 1));
        }
    }
    Some(out)
}

/// The glyph's outline filled white on a transparent square of `side` pixels: the vertical
/// extent from descender to ascender fills the height, the advance width is centred.
fn rasterise(ctx: &egui::Context, font: &Font, glyph: &Glyph, master: &str, frame: Frame, side: u32) -> Option<TextureHandle> {
    let outline = font.outline_of(glyph, master).ok()?;
    if outline.elements().is_empty() || side == 0 {
        return None;
    }
    let height = (frame.ascender - frame.descender).max(1.0);
    let scale = f64::from(side) / height;
    let width = glyph.layer(master).map_or(0.0, |l| l.width);
    let left = f64::from(side) / 2.0 - width * scale / 2.0;
    let transform = Affine::new([scale, 0.0, 0.0, -scale, left, frame.ascender * scale]);
    let style = Style { ink: [255, 255, 255, 255], background: [0, 0, 0, 0], anti_alias: true };
    let image = render_path(&outline, transform, side, side, &style).ok()?;
    let size = [usize::try_from(image.width).ok()?, usize::try_from(image.height).ok()?];
    let color = ColorImage::from_rgba_premultiplied(size, &image.pixels);
    Some(ctx.load_texture(format!("fontView.cell.{}.{master}", glyph.name), color, TextureOptions::LINEAR))
}
