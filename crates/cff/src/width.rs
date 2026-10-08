//! Choosing `defaultWidthX` and `nominalWidthX`.
//!
//! A charstring leaves its width out when it equals `defaultWidthX` and otherwise stores the
//! difference from `nominalWidthX`, in one, two or five bytes. This is a port of fontTools
//! `cffLib/width.py` (`optimizeWidths`, `byteCost`; MIT, Copyright (c) 2017 Just van Rossum and
//! contributors), the method the reference compiler uses: cumulative counts over the width range
//! give the cost of every nominal width, the best default for it, and the cheapest pair. It runs in
//! time linear in the width range plus the glyph count, and ties go to the smallest width, as the
//! reference's do.

use std::collections::BTreeMap;

/// The best `(defaultWidthX, nominalWidthX)` for these widths (already rounded to integers).
/// `None` for an empty list.
pub(crate) fn optimize(widths: &[i64]) -> Option<(i64, i64)> {
    let mut freq: BTreeMap<i64, i64> = BTreeMap::new();
    for &w in widths {
        *freq.entry(w).or_insert(0) += 1;
    }
    let (&min_w, _) = freq.iter().next()?;
    let (&max_w, _) = freq.iter().next_back()?;
    let span = usize::try_from(max_w - min_w).ok()?.checked_add(1)?;
    let at = |x: i64| freq.get(&x).copied().unwrap_or(0);

    // Cumulative sums and maxima, upwards (over widths ≤ x) and downwards (over widths ≥ x).
    let mut frq_up = Vec::with_capacity(span);
    let mut max_up = Vec::with_capacity(span);
    let (mut s, mut m) = (0i64, 0i64);
    for x in min_w..=max_w {
        s += at(x);
        m = m.max(at(x));
        frq_up.push(s);
        max_up.push(m);
    }
    let (total, overall_max) = (s, m);
    let mut frq_down = vec![0i64; span];
    let mut max_down = vec![0i64; span];
    let (mut s, mut m) = (0i64, 0i64);
    for i in (0..span).rev() {
        let x = min_w + i as i64;
        s += at(x);
        m = m.max(at(x));
        if let (Some(f), Some(mx)) = (frq_down.get_mut(i), max_down.get_mut(i)) {
            *f = s;
            *mx = m;
        }
    }
    let slot = |x: i64| usize::try_from(x - min_w).ok();
    // Outside the width range, an upward sum is 0 below it and the total above it; downward sums
    // the other way round.
    let cum_up = |table: &Vec<i64>, x: i64, above: i64| {
        if x < min_w {
            0
        } else if x > max_w {
            above
        } else {
            slot(x).and_then(|i| table.get(i)).copied().unwrap_or(0)
        }
    };
    let cum_down = |table: &Vec<i64>, x: i64, below: i64| {
        if x > max_w {
            0
        } else if x < min_w {
            below
        } else {
            slot(x).and_then(|i| table.get(i)).copied().unwrap_or(0)
        }
    };
    let frq_u = |x: i64| cum_up(&frq_up, x, total);
    let max_u = |x: i64| cum_up(&max_up, x, overall_max);
    let frq_d = |x: i64| cum_down(&frq_down, x, total);
    let max_d = |x: i64| cum_down(&max_down, x, overall_max);

    let nomn_cost = |x: i64| {
        let up = frq_u(x) + frq_u(x - 108) + frq_u(x - 1132) * 3;
        let down = frq_d(x) + frq_d(x + 108) + frq_d(x + 1132) * 3;
        up + down - at(x)
    };
    let dflt_cost_u = |x: i64| max_u(x).max(max_u(x - 108) * 2).max(max_u(x - 1132) * 5);
    let dflt_cost_d = |x: i64| max_d(x).max(max_d(x + 108) * 2).max(max_d(x + 1132) * 5);
    let best_cost = |x: i64| nomn_cost(x) - dflt_cost_u(x).max(dflt_cost_d(x));

    // The first width with the lowest cost.
    let mut nominal = min_w;
    let mut lowest = best_cost(min_w);
    for x in min_w + 1..=max_w {
        let c = best_cost(x);
        if c < lowest {
            lowest = c;
            nominal = x;
        }
    }

    // Work back to the default that achieves it.
    let dflt_c = nomn_cost(nominal) - best_cost(nominal);
    let mut ends = Vec::with_capacity(3);
    if dflt_c == dflt_cost_u(nominal) {
        for start in [nominal, nominal - 108, nominal - 1132] {
            let mut x = start;
            while max_u(x) != 0 && max_u(x) == max_u(x - 1) {
                x -= 1;
            }
            ends.push(x);
        }
    } else {
        for start in [nominal, nominal + 108, nominal + 1132] {
            let mut x = start;
            while max_d(x) != 0 && max_d(x) == max_d(x + 1) {
                x += 1;
            }
            ends.push(x);
        }
    }
    let mut default = *ends.first()?;
    let mut default_cost = byte_cost(&freq, default, nominal);
    for &e in ends.iter().skip(1) {
        let c = byte_cost(&freq, e, nominal);
        if c < default_cost {
            default_cost = c;
            default = e;
        }
    }
    Some((default, nominal))
}

/// Bytes the widths take with this default and nominal width.
pub(crate) fn byte_cost(freq: &BTreeMap<i64, i64>, default: i64, nominal: i64) -> i64 {
    freq.iter()
        .filter(|&(&w, _)| w != default)
        .map(|(&w, &n)| {
            let diff = (w - nominal).abs();
            n * if diff <= 107 {
                1
            } else if diff <= 1131 {
                2
            } else {
                5
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn freq(widths: &[i64]) -> BTreeMap<i64, i64> {
        let mut f = BTreeMap::new();
        for &w in widths {
            *f.entry(w).or_insert(0) += 1;
        }
        f
    }

    /// The reference's brute-force search over every pair, for checking the fast one.
    fn brute(widths: &[i64]) -> i64 {
        let f = freq(widths);
        let lo = *widths.iter().min().unwrap();
        let hi = *widths.iter().max().unwrap();
        let mut best = i64::MAX;
        for n in lo..=hi {
            for d in lo..=hi {
                best = best.min(byte_cost(&f, d, n));
            }
        }
        best
    }

    #[test]
    fn one_width_is_the_default() {
        assert_eq!(optimize(&[500, 500, 500]), Some((500, 500)));
        assert_eq!(optimize(&[]), None);
    }

    #[test]
    fn values_match_the_reference() {
        // Expected pairs are fontTools' `optimizeWidths` results for the same lists.
        assert_eq!(optimize(&[0, 500, 500, 600, 1000]), Some((500, 0)));
        assert_eq!(optimize(&[250, 250, 600, 600, 600, 1200, 3000]), Some((600, 250)));
        assert_eq!(optimize(&[-100, 0, 0, 5000]), Some((5000, -100)));
        assert_eq!(optimize(&[100, 200, 300, 400, 400, 400, 2000, 2100]), Some((400, 969)));
        assert_eq!(optimize(&[0, 0, 0, 1131, 1132, 1300]), Some((0, 1193)));
    }

    #[test]
    fn the_result_costs_what_the_brute_force_optimum_costs() {
        let lists: [&[i64]; 4] = [&[0, 500, 500, 600, 1000], &[100, 200, 300, 400, 400, 400, 2000, 2100], &[0, 0, 0, 1131, 1132, 1300], &[600; 7]];
        for widths in lists {
            let (d, n) = optimize(widths).unwrap();
            assert_eq!(byte_cost(&freq(widths), d, n), brute(widths), "{widths:?}");
        }
    }
}
