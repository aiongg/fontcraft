//! Interpolation of untouched points (IUP): the deltas a `gvar` tuple leaves out, inferred from
//! the points around them. A port of fontTools' `varLib.iup.iup_delta` (MIT, Copyright (c) 2016
//! Google Inc. and the fontTools authors).

/// A point or delta: x, y.
pub type Pair = (f64, f64);

/// Every point's delta, given the explicit ones (`None` where left out), the glyph's default
/// coordinates (with its four phantom points last) and the index of each contour's last point.
/// Each phantom point is a contour of its own, as is each component of a composite glyph (give
/// `ends` = `0..n`). A contour with no explicit delta does not move. `deltas` and `coords` have
/// the same length; extra entries of either are ignored.
pub fn iup_delta(deltas: &[Option<Pair>], coords: &[Pair], ends: &[usize]) -> Vec<Pair> {
    let n = deltas.len().min(coords.len());
    let mut bounds: Vec<usize> = ends.iter().copied().filter(|e| *e + 4 < n).collect();
    bounds.sort_unstable();
    bounds.dedup();
    for k in (1..=4).rev() {
        if let Some(e) = n.checked_sub(k) {
            bounds.push(e);
        }
    }
    let mut out = Vec::with_capacity(n);
    let mut start = 0usize;
    for end in bounds {
        let end = end + 1;
        if end <= start || end > n {
            continue;
        }
        let (Some(d), Some(c)) = (deltas.get(start..end), coords.get(start..end)) else { continue };
        out.extend(iup_contour(d, c));
        start = end;
    }
    // Points after the last contour end (malformed `ends`) do not move.
    out.resize(n, (0.0, 0.0));
    out
}

/// One contour's deltas.
fn iup_contour(deltas: &[Option<Pair>], coords: &[Pair]) -> Vec<Pair> {
    let n = deltas.len();
    let indices: Vec<usize> = deltas.iter().enumerate().filter_map(|(i, d)| d.map(|_| i)).collect();
    let (Some(&first), Some(&last)) = (indices.first(), indices.last()) else {
        return vec![(0.0, 0.0); n];
    };
    if indices.len() == n {
        return deltas.iter().map(|d| d.unwrap_or((0.0, 0.0))).collect();
    }
    let at = |i: usize| (coords.get(i).copied().unwrap_or((0.0, 0.0)), deltas.get(i).copied().flatten().unwrap_or((0.0, 0.0)));
    let mut out = Vec::with_capacity(n);
    if first != 0 {
        let (rc1, rd1) = at(first);
        let (rc2, rd2) = at(last);
        out.extend(segment(coords.get(..first).unwrap_or_default(), rc1, rd1, rc2, rd2));
    }
    out.push(at(first).1);
    let mut start = first;
    for &end in indices.iter().skip(1) {
        if end - start > 1 {
            let (rc1, rd1) = at(start);
            let (rc2, rd2) = at(end);
            out.extend(segment(coords.get(start + 1..end).unwrap_or_default(), rc1, rd1, rc2, rd2));
        }
        out.push(at(end).1);
        start = end;
    }
    if start != n - 1 {
        let (rc1, rd1) = at(start);
        let (rc2, rd2) = at(first);
        out.extend(segment(coords.get(start + 1..).unwrap_or_default(), rc1, rd1, rc2, rd2));
    }
    out
}

/// The deltas of `coords`, which lie between two reference points, per axis.
fn segment(coords: &[Pair], rc1: Pair, rd1: Pair, rc2: Pair, rd2: Pair) -> Vec<Pair> {
    let axis = |pick: fn(Pair) -> f64| -> Vec<f64> {
        let (mut x1, mut x2, mut d1, mut d2) = (pick(rc1), pick(rc2), pick(rd1), pick(rd2));
        if x1 == x2 {
            let d = if d1 == d2 { d1 } else { 0.0 };
            return vec![d; coords.len()];
        }
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut d1, &mut d2);
        }
        let scale = (d2 - d1) / (x2 - x1);
        coords
            .iter()
            .map(|p| {
                let x = pick(*p);
                if x <= x1 {
                    d1
                } else if x >= x2 {
                    d2
                } else {
                    d1 + (x - x1) * scale
                }
            })
            .collect()
    };
    let xs = axis(|p| p.0);
    let ys = axis(|p| p.1);
    xs.into_iter().zip(ys).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iup_interpolates_between_references_and_leaves_untouched_contours() {
        // A square contour with deltas on two opposite corners, then four phantom points of
        // which only the right one moves.
        let coords = [(0.0, 0.0), (0.0, 100.0), (100.0, 100.0), (100.0, 0.0), (50.0, 50.0), (0.0, 0.0), (500.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let deltas = [Some((10.0, 0.0)), None, Some((30.0, 20.0)), None, None, None, Some((40.0, 0.0)), None, None];
        let out = iup_delta(&deltas, &coords, &[4]);
        assert_eq!(out.len(), 9);
        assert_eq!(out[0], (10.0, 0.0));
        // (0, 100): x equals the first reference's x → its delta; y beyond both → nearest.
        assert_eq!(out[1], (10.0, 20.0));
        assert_eq!(out[2], (30.0, 20.0));
        // (100, 0): x at the second reference; y at the first.
        assert_eq!(out[3], (30.0, 0.0));
        // (50, 50) wraps round to point 0: halfway in both axes.
        assert_eq!(out[4], (20.0, 10.0));
        assert_eq!(out[5], (0.0, 0.0), "a phantom point without a delta does not move");
        assert_eq!(out[6], (40.0, 0.0));
    }
}
