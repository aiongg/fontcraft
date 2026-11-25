//! A checkered fill: the squares of a fixed screen grid that fall inside an outline, filled by the nonzero rule.

use egui::{Color32, Mesh, Pos2, Rect, pos2};

/// Most bands one fill is made of; the rest is left out.
const MAX_BANDS: usize = 2_048;
/// Most work the fills of one frame are: bands, edges crossed and squares. What is beyond it is not filled.
pub const MAX_FRAME_WORK: usize = 100_000;

/// The checkered fill of `contours` (screen points) inside `clip`, every other square of a grid of `side` points. `None`
/// when nothing is filled. `work` is what is left of the frame's [`MAX_FRAME_WORK`].
pub fn checker(contours: &[Vec<Pos2>], clip: Rect, side: f32, color: Color32, work: &mut usize) -> Option<Mesh> {
    if *work == 0 || !(side.is_finite() && side >= 1.0 && clip.is_finite()) {
        return None;
    }
    // A component that is not on the canvas costs no more than its box.
    let mut bounds = Rect::NOTHING;
    contours.iter().flatten().filter(|p| p.is_finite()).for_each(|p| bounds.extend_with(*p));
    if !bounds.intersects(clip) {
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
    edges.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    let mut coming = edges.iter().copied().peekable();
    // The edges a band can cross: those that begin above its middle and have not ended.
    let mut open: Vec<(Pos2, Pos2, i32)> = Vec::new();
    let mut crossing: Vec<(f32, i32)> = Vec::new();
    for k in 0..MAX_BANDS {
        let y0 = ((first + k as f32) * step).max(top);
        let y1 = ((first + k as f32 + 1.0) * step).min(bottom);
        if y0 >= bottom {
            break;
        }
        let mid = (y0 + y1) / 2.0;
        while let Some(edge) = coming.next_if(|(a, _, _)| a.y <= mid) {
            open.push(edge);
        }
        open.retain(|(_, b, _)| b.y > mid);
        crossing.clear();
        crossing.extend(open.iter().map(|(a, b, w)| (a.x + (mid - a.y) * (b.x - a.x) / (b.y - a.y), *w)));
        *work = work.saturating_sub(crossing.len().saturating_add(1));
        if *work == 0 {
            break;
        }
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
                        if *work == 0 {
                            return Some(mesh);
                        }
                        *work = work.saturating_sub(1);
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

    use super::{MAX_FRAME_WORK, checker};

    /// Half of a square's area is filled, all of it inside the outline and the clip.
    #[test]
    fn every_other_square_inside_the_outline_is_filled() {
        let square = vec![pos2(0.0, 0.0), pos2(64.0, 0.0), pos2(64.0, 64.0), pos2(0.0, 64.0)];
        let clip = Rect::from_min_max(pos2(-100.0, -100.0), pos2(100.0, 100.0));
        let mut work = MAX_FRAME_WORK;
        let mesh = checker(std::slice::from_ref(&square), clip, 8.0, Color32::WHITE, &mut work).expect("filled");
        let area: f32 = mesh.vertices.chunks(4).filter_map(|q| Some((q.get(2)?.pos - q.first()?.pos).x * (q.get(2)?.pos - q.first()?.pos).y)).sum();
        assert!((area - 64.0 * 32.0).abs() < 1.0, "{area}");
        let half = Rect::from_min_max(pos2(0.0, 0.0), pos2(64.0, 32.0));
        let clipped = checker(std::slice::from_ref(&square), half, 8.0, Color32::WHITE, &mut work).expect("filled");
        assert!(clipped.vertices.iter().all(|v| half.expand(0.01).contains(v.pos)));
        assert!(checker(&[], clip, 8.0, Color32::WHITE, &mut work).is_none());

        let left = work;
        let away = Rect::from_min_max(pos2(80.0, 0.0), pos2(100.0, 100.0));
        assert!(checker(std::slice::from_ref(&square), away, 8.0, Color32::WHITE, &mut work).is_none() && work == left, "off the canvas");
        let mut little = 100;
        let part = checker(std::slice::from_ref(&square), clip, 8.0, Color32::WHITE, &mut little).expect("filled in part");
        assert!(little == 0 && part.vertices.len() < mesh.vertices.len(), "the frame's work bounds the fills together");
        assert!(checker(&[square], clip, 8.0, Color32::WHITE, &mut little).is_none());
    }
}
