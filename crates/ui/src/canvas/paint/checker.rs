//! A checkered fill: the squares of a fixed screen grid that fall inside an outline, filled by the nonzero rule.

use egui::{Color32, Mesh, Pos2, Rect, pos2};

/// Most bands and most squares one fill is made of; the rest is left out.
const MAX_BANDS: usize = 2_048;
const MAX_SQUARES: usize = 20_000;

/// The checkered fill of `contours` (screen points) inside `clip`, every other square of a grid of `side` points. `None`
/// when nothing is filled.
pub fn checker(contours: &[Vec<Pos2>], clip: Rect, side: f32, color: Color32) -> Option<Mesh> {
    if !(side.is_finite() && side >= 1.0 && clip.is_finite()) {
        return None;
    }
    // Each edge top end first, with its winding: +1 when the contour runs down it.
    let mut edges: Vec<(Pos2, Pos2, i32)> = Vec::new();
    for contour in contours {
        for (i, &a) in contour.iter().enumerate() {
            let b = contour.get(i + 1).or(contour.first()).copied().unwrap_or(a);
            if !(a.is_finite() && b.is_finite()) || (a.y - b.y).abs() <= f32::EPSILON || a.y.max(b.y) < clip.top() || a.y.min(b.y) > clip.bottom() {
                continue;
            }
            edges.push(if a.y < b.y { (a, b, 1) } else { (b, a, -1) });
        }
    }
    let top = edges.iter().map(|e| e.0.y).fold(f32::INFINITY, f32::min).max(clip.top());
    let bottom = edges.iter().map(|e| e.1.y).fold(f32::NEG_INFINITY, f32::max).min(clip.bottom());
    if !(top.is_finite() && bottom.is_finite() && bottom > top) {
        return None;
    }
    // Four bands to a square follow a slanted or curved edge closely enough.
    let step = side / 4.0;
    let first = (top / step).floor();
    let mut mesh = Mesh::default();
    let mut squares = 0usize;
    let mut crossing: Vec<(f32, i32)> = Vec::new();
    for k in 0..MAX_BANDS {
        let y0 = ((first + k as f32) * step).max(top);
        let y1 = ((first + k as f32 + 1.0) * step).min(bottom);
        if y0 >= bottom {
            break;
        }
        let mid = (y0 + y1) / 2.0;
        crossing.clear();
        crossing
            .extend(edges.iter().filter(|(a, b, _)| a.y <= mid && b.y > mid).map(|(a, b, w)| (a.x + (mid - a.y) * (b.x - a.x) / (b.y - a.y), *w)));
        crossing.sort_by(|a, b| a.0.total_cmp(&b.0));
        let row = (mid / side).floor();
        let mut winding = 0;
        let mut start = None;
        for &(x, w) in &crossing {
            let was = winding;
            winding += w;
            if was == 0 && winding != 0 {
                start = Some(x);
            } else if was != 0
                && winding == 0
                && let Some(from) = start.take()
            {
                let (from, to) = (f32::max(from, clip.left()), x.min(clip.right()));
                let mut column = (from / side).floor();
                while column * side < to {
                    if (column + row).rem_euclid(2.0) < 0.5 {
                        if squares >= MAX_SQUARES {
                            return Some(mesh);
                        }
                        squares += 1;
                        quad(&mut mesh, Rect::from_min_max(pos2((column * side).max(from), y0), pos2(((column + 1.0) * side).min(to), y1)), color);
                    }
                    let next = column + 1.0;
                    if next <= column {
                        break;
                    }
                    column = next;
                }
            }
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

fn quad(mesh: &mut Mesh, rect: Rect, color: Color32) {
    let Ok(base) = u32::try_from(mesh.vertices.len()) else { return };
    for p in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()] {
        mesh.colored_vertex(p, color);
    }
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Rect, pos2};

    use super::checker;

    /// Half of a square's area is filled, all of it inside the outline and the clip.
    #[test]
    fn every_other_square_inside_the_outline_is_filled() {
        let square = vec![pos2(0.0, 0.0), pos2(64.0, 0.0), pos2(64.0, 64.0), pos2(0.0, 64.0)];
        let clip = Rect::from_min_max(pos2(-100.0, -100.0), pos2(100.0, 100.0));
        let mesh = checker(std::slice::from_ref(&square), clip, 8.0, Color32::WHITE).expect("filled");
        let area: f32 = mesh.vertices.chunks(4).filter_map(|q| Some((q.get(2)?.pos - q.first()?.pos).x * (q.get(2)?.pos - q.first()?.pos).y)).sum();
        assert!((area - 64.0 * 32.0).abs() < 1.0, "{area}");
        let half = Rect::from_min_max(pos2(0.0, 0.0), pos2(64.0, 32.0));
        let clipped = checker(&[square], half, 8.0, Color32::WHITE).expect("filled");
        assert!(clipped.vertices.iter().all(|v| half.expand(0.01).contains(v.pos)));
        assert!(checker(&[], clip, 8.0, Color32::WHITE).is_none());
    }
}
