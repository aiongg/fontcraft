//! Power guides, guide thickness and markers (M13.36), drawn over the guides while guides show:
//!
//! - a guide with a thickness gets its second edge, a thinner line at the given distance;
//! - markers are short ticks across the guide;
//! - a power guide has a rhombus at its knob, and a dashed line from each node it holds to the
//!   guide.
//!
//! The data is the guide's ([`fontcraft_model::guide_data`]); colours and widths come from
//! [`crate::theme::Tokens`], in the guide's own colour where it has one.

use egui::{Color32, Pos2, Shape, Stroke};
use fontcraft_geom::{Point, Vec2};
use fontcraft_model::guide_data;
use fontcraft_model::{Guide, Layer, LayerSide};

use super::{Painting, Scene};
use crate::panels::Pending;

/// Length of a marker tick on each side of the guide, in screen points.
const TICK: f32 = 5.0;
/// Dash and gap of a link line, in screen points.
const DASH: f32 = 4.0;
const GAP: f32 = 3.0;
/// Most link lines drawn for one guide.
const MAX_DRAWN_LINKS: usize = 2_000;

/// Draws this overlay for the edited layer.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = (ui, pending);
    if !scene.state.display.guides {
        return;
    }
    let layer = scene.layer;
    let tokens = c.tokens;
    for g in &layer.info.guides {
        let color = guide_color(g).unwrap_or(tokens.guide_local);
        draw_guide(c, layer, g, color, |l| l.glyph.is_none());
    }
    let Some(master) = scene.font.masters.iter().find(|m| m.id == layer.master) else { return };
    let hidden = super::guides::hidden_global_guides(scene.session, scene.font, scene.tab);
    let glyph = scene.tab.edited_glyph().unwrap_or_default();
    let own = scene.tab.side() == LayerSide::Foreground && layer.is_master_layer();
    for (i, g) in master.guides.iter().enumerate() {
        if hidden.contains(&i) {
            continue;
        }
        let color = guide_color(g).unwrap_or(tokens.guide_global);
        draw_guide(c, layer, g, color, |l| own && l.glyph.as_deref() == Some(glyph));
    }
}

/// A guide's own `r,g,b,a` colour.
fn guide_color(g: &Guide) -> Option<Color32> {
    let text = g.color.as_deref()?;
    let parts: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let byte = |v: f64| (v * 255.0).round() as u8;
    match parts.as_slice() {
        [r, g, b, a] => Some(Color32::from_rgba_unmultiplied(byte(*r), byte(*g), byte(*b), byte(*a))),
        _ => None,
    }
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}

fn usable(p: Pos2) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// The guide's unit direction, in font units.
fn direction(g: &Guide) -> Vec2 {
    let a = if g.angle.is_finite() { g.angle.to_radians() } else { 0.0 };
    Vec2::new(a.cos(), a.sin())
}

/// A line through `at` (font units) along `dir`, long enough to cross the canvas.
fn long_line(c: &Painting, at: Point, dir: Vec2) -> Option<[Pos2; 2]> {
    let a = c.view.to_screen(at);
    let b = c.view.to_screen(at + dir);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if !(len.is_finite() && len > 1e-9) {
        return None;
    }
    // From the point of the line nearest the canvas centre, half the canvas diagonal each way.
    let center = c.rect.center();
    let (ux, uy) = (dx / len, dy / len);
    let t = (f64::from(center.x) - a.x) * ux + (f64::from(center.y) - a.y) * uy;
    let reach = f64::from(c.rect.width().hypot(c.rect.height()));
    let mid = Point::new(a.x + ux * t, a.y + uy * t);
    let ends = [pos(Point::new(mid.x - ux * reach, mid.y - uy * reach)), pos(Point::new(mid.x + ux * reach, mid.y + uy * reach))];
    ends.iter().all(|p| usable(*p)).then_some(ends)
}

fn draw_guide(c: &Painting, layer: &Layer, g: &Guide, color: Color32, mine: impl Fn(&guide_data::Link) -> bool) {
    let tokens = c.tokens;
    let knob = Point::new(g.x, g.y);
    let dir = direction(g);
    let normal = Vec2::new(-dir.y, dir.x);
    let faint = color.gamma_multiply(0.5);
    if let Some(t) = guide_data::thickness(g)
        && let Some(edge) = long_line(c, knob + normal * t, dir)
    {
        c.painter.line_segment(edge, Stroke::new(tokens.guide_width * 0.5, color));
        if let Some(centre) = long_line(c, knob + normal * (t / 2.0), dir) {
            c.painter.extend(Shape::dashed_line(&centre, Stroke::new(tokens.guide_width * 0.5, faint), DASH, GAP));
        }
    }
    for d in guide_data::markers(g) {
        let at = c.view.to_screen(knob + dir * d);
        let tip = c.view.to_screen(knob + dir * d + normal);
        let (nx, ny) = (tip.x - at.x, tip.y - at.y);
        let len = nx.hypot(ny);
        if !(len.is_finite() && len > 1e-9) {
            continue;
        }
        let (ux, uy) = ((nx / len) as f32, (ny / len) as f32);
        let p = pos(at);
        if usable(p) {
            let a = Pos2::new(p.x - ux * TICK, p.y - uy * TICK);
            let b = Pos2::new(p.x + ux * TICK, p.y + uy * TICK);
            c.painter.line_segment([a, b], Stroke::new(tokens.guide_width, color));
        }
    }
    let links = guide_data::links(g);
    if links.is_empty() {
        return;
    }
    let k = pos(c.view.to_screen(knob));
    if usable(k) {
        let r = tokens.guide_knob_radius * 1.4;
        let rhombus = vec![Pos2::new(k.x, k.y - r), Pos2::new(k.x + r, k.y), Pos2::new(k.x, k.y + r), Pos2::new(k.x - r, k.y)];
        c.painter.add(Shape::convex_polygon(rhombus, color, Stroke::NONE));
    }
    for link in links.iter().filter(|l| mine(l)).take(MAX_DRAWN_LINKS) {
        let Some(node) = layer.parts(LayerSide::Foreground).path(link.shape).and_then(|p| p.nodes.get(link.node)) else { continue };
        let p = node.point();
        let foot = knob + dir * (p - knob).dot(dir);
        let (a, b) = (pos(c.view.to_screen(p)), pos(c.view.to_screen(foot)));
        if usable(a) && usable(b) {
            c.painter.extend(Shape::dashed_line(&[a, b], Stroke::new(tokens.guide_width, faint), DASH, GAP));
            c.painter.circle_filled(a, tokens.guide_knob_radius * 0.6, faint);
        }
    }
}
