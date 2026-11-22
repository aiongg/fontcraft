//! The nonzero fill of flattened contours, as one mesh of horizontal bands, so counters stay open.

use egui::{Color32, Mesh, Pos2, Rect, pos2};

/// Most bands one glyph is filled in.
const MAX_BANDS: f32 = 512.0;

/// An edge in screen points, top end first, with its winding.
#[derive(Clone, Copy)]
struct Edge {
    top: Pos2,
    bottom: Pos2,
    winding: i32,
}

impl Edge {
    fn x_at(&self, y: f32) -> f32 {
        let dy = self.bottom.y - self.top.y;
        if dy <= f32::EPSILON {
            self.top.x
        } else {
            self.top.x + (y.clamp(self.top.y, self.bottom.y) - self.top.y) * (self.bottom.x - self.top.x) / dy
        }
    }
}

/// The nonzero fill of `contours` (screen points) inside `clip`, as one mesh of horizontal bands; `None` when
/// nothing is filled.
pub fn nonzero(contours: &[Vec<Pos2>], clip: Rect, colour: Color32) -> Option<Mesh> {
    let mut edges = Vec::new();
    for contour in contours {
        for pair in contour.windows(2) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.get(1)) else { continue };
            if (a.y - b.y).abs() <= f32::EPSILON {
                continue;
            }
            edges.push(if a.y < b.y { Edge { top: a, bottom: b, winding: 1 } } else { Edge { top: b, bottom: a, winding: -1 } });
        }
    }
    let top = edges.iter().map(|e| e.top.y).fold(f32::INFINITY, f32::min).max(clip.top());
    let bottom = edges.iter().map(|e| e.bottom.y).fold(f32::NEG_INFINITY, f32::max).min(clip.bottom());
    if !(top.is_finite() && bottom.is_finite() && bottom > top) {
        return None;
    }
    let step = ((bottom - top) / MAX_BANDS).max(0.5);
    let mut mesh = Mesh::default();
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut y = top;
    while y < bottom {
        let next = (y + step).min(bottom);
        let mid = (y + next) / 2.0;
        crossings.clear();
        crossings.extend(edges.iter().filter(|e| e.top.y <= mid && mid < e.bottom.y).map(|e| (e.x_at(mid), e.winding)));
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut from = 0.0;
        for &(x, w) in &crossings {
            let was = winding;
            winding += w;
            if was == 0 && winding != 0 {
                from = x;
            } else if was != 0 && winding == 0 && x > from {
                mesh.add_colored_rect(Rect::from_min_max(pos2(from, y), pos2(x, next)), colour);
            }
        }
        y = next;
    }
    (!mesh.is_empty()).then_some(mesh)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use egui::{Color32, Pos2, Rect, pos2};

    use super::nonzero;

    /// The band fill leaves a counter open: a square inside a square, wound the other way, is not filled.
    #[test]
    fn fill_keeps_counters_open() {
        let outer = vec![pos2(0.0, 0.0), pos2(30.0, 0.0), pos2(30.0, 30.0), pos2(0.0, 30.0), pos2(0.0, 0.0)];
        let inner = vec![pos2(10.0, 10.0), pos2(10.0, 20.0), pos2(20.0, 20.0), pos2(20.0, 10.0), pos2(10.0, 10.0)];
        let mesh = nonzero(&[outer, inner], Rect::from_min_size(Pos2::ZERO, egui::Vec2::splat(100.0)), Color32::BLACK).unwrap();
        let covers = |p: Pos2| {
            mesh.indices.chunks(6).any(|quad| {
                let xs: Vec<Pos2> = quad.iter().map(|&i| mesh.vertices[i as usize].pos).collect();
                let r = Rect::from_points(&xs);
                r.contains(p)
            })
        };
        assert!(covers(pos2(5.0, 15.0)));
        assert!(!covers(pos2(15.0, 15.0)));
    }
}
