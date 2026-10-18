//! Line diffs for [`super::diff`]: an edit script found by Myers' algorithm within a work limit,
//! unified hunks with context lines, and the line-multiset similarity ratio.
//!
//! The hunk grouping and the `@@ -a,b +c,d @@` ranges follow Python's `difflib.unified_diff`, and
//! the ratio is `difflib.SequenceMatcher.quick_ratio`, which fontc's `ttx_diff` reports per table.

use std::collections::HashMap;

/// Most comparisons one edit script may cost. Past it, the part not yet matched is compared
/// position by position instead.
pub(crate) const MAX_DIFF_WORK: usize = 50_000_000;
/// Most edit steps Myers' search takes; the trace it keeps for backtracking grows with the
/// square of this number (about 32 MB at the limit).
pub(crate) const MAX_EDIT_DISTANCE: usize = 2_000;

/// One run of an edit script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Items equal on both sides.
    Equal,
    /// Items of the first side only.
    Delete,
    /// Items of the second side only.
    Insert,
}

/// The runs that turn `n` items of the first side into `m` items of the second; `eq(i, j)` says
/// whether item `i` of the first equals item `j` of the second. The script is the shortest one
/// unless it would cost more than `max_work` comparisons or [`MAX_EDIT_DISTANCE`] edits.
pub(crate) fn script(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool, max_work: usize) -> Vec<(Op, usize)> {
    let mut prefix = 0;
    while prefix < n && prefix < m && eq(prefix, prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < n - prefix && suffix < m - prefix && eq(n - 1 - suffix, m - 1 - suffix) {
        suffix += 1;
    }
    let (n2, m2) = (n - prefix - suffix, m - prefix - suffix);
    let mut runs = Vec::new();
    push(&mut runs, Op::Equal, prefix);
    let middle = myers(n2, m2, |i, j| eq(prefix + i, prefix + j), max_work).unwrap_or_else(|| positional(n2, m2, |i, j| eq(prefix + i, prefix + j)));
    for (op, len) in middle {
        push(&mut runs, op, len);
    }
    push(&mut runs, Op::Equal, suffix);
    runs
}

fn push(runs: &mut Vec<(Op, usize)>, op: Op, len: usize) {
    if len == 0 {
        return;
    }
    match runs.last_mut() {
        Some((last, n)) if *last == op => *n += len,
        _ => runs.push((op, len)),
    }
}

/// The fallback script: items compared at the same position, the longer side's rest added or
/// removed. Right for tables changed in place, which is what a byte-level comparison mostly sees.
fn positional(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool) -> Vec<(Op, usize)> {
    let mut runs = Vec::new();
    for i in 0..n.min(m) {
        if eq(i, i) {
            push(&mut runs, Op::Equal, 1);
        } else {
            push(&mut runs, Op::Delete, 1);
            push(&mut runs, Op::Insert, 1);
        }
    }
    push(&mut runs, Op::Delete, n.saturating_sub(m));
    push(&mut runs, Op::Insert, m.saturating_sub(n));
    runs
}

/// Myers' O(ND) search with a trace for backtracking; `None` when the limits are reached.
fn myers(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool, max_work: usize) -> Option<Vec<(Op, usize)>> {
    if n == 0 || m == 0 {
        let mut runs = Vec::new();
        push(&mut runs, Op::Delete, n);
        push(&mut runs, Op::Insert, m);
        return Some(runs);
    }
    let max_d = n.saturating_add(m).min(MAX_EDIT_DISTANCE);
    let n_i = i64::try_from(n).ok()?;
    let m_i = i64::try_from(m).ok()?;
    let max_d_i = i64::try_from(max_d).ok()?;
    let offset = max_d_i + 1;
    let width = usize::try_from(2 * max_d_i + 3).ok()?;
    let mut v = vec![0i64; width];
    let at = |k: i64| usize::try_from(k + offset).unwrap_or(0);
    // trace[d] holds v before round d, for k in -(d+1)..=d+1.
    let mut trace: Vec<Vec<i64>> = Vec::new();
    let mut work = 0usize;
    for d in 0..=max_d_i {
        let lo = at(-d - 1);
        let hi = at(d + 1);
        trace.push(v.get(lo..=hi)?.to_vec());
        let mut k = -d;
        while k <= d {
            let down = k == -d || (k != d && v.get(at(k - 1))? < v.get(at(k + 1))?);
            let mut x = if down { *v.get(at(k + 1))? } else { v.get(at(k - 1))? + 1 };
            let mut y = x - k;
            while x < n_i && y < m_i && eq(usize::try_from(x).ok()?, usize::try_from(y).ok()?) {
                x += 1;
                y += 1;
                work += 1;
            }
            work += 1;
            if work > max_work {
                return None;
            }
            *v.get_mut(at(k))? = x;
            if x >= n_i && y >= m_i {
                return backtrack(&trace, n_i, m_i, d);
            }
            k += 2;
        }
    }
    None
}

fn backtrack(trace: &[Vec<i64>], n: i64, m: i64, last: i64) -> Option<Vec<(Op, usize)>> {
    let mut rev: Vec<(Op, usize)> = Vec::new();
    let mut add = |op: Op| match rev.last_mut() {
        Some((o, len)) if *o == op => *len += 1,
        _ => rev.push((op, 1)),
    };
    let (mut x, mut y) = (n, m);
    for d in (0..=last).rev() {
        let v = trace.get(usize::try_from(d).ok()?)?;
        // The trace for round d starts at k = -(d+1).
        let get = |k: i64| -> Option<i64> { v.get(usize::try_from(k + d + 1).ok()?).copied() };
        let k = x - y;
        let prev_k = if k == -d || (k != d && get(k - 1)? < get(k + 1)?) { k + 1 } else { k - 1 };
        let prev_x = get(prev_k)?;
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            add(Op::Equal);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            add(if x == prev_x { Op::Insert } else { Op::Delete });
            x = prev_x;
            y = prev_y;
        }
    }
    rev.reverse();
    Some(rev)
}

/// One opcode of a script: what it is and its ranges on each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Code {
    equal: bool,
    a: (usize, usize),
    b: (usize, usize),
}

/// The script as opcodes: equal runs, and each stretch of removals and additions as one change.
fn opcodes(runs: &[(Op, usize)]) -> Vec<Code> {
    let mut codes: Vec<Code> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    for &(op, len) in runs {
        let (di, dj) = match op {
            Op::Equal => (len, len),
            Op::Delete => (len, 0),
            Op::Insert => (0, len),
        };
        let equal = op == Op::Equal;
        match codes.last_mut() {
            Some(c) if !c.equal && !equal => {
                c.a.1 += di;
                c.b.1 += dj;
            }
            _ => codes.push(Code { equal, a: (i, i + di), b: (j, j + dj) }),
        }
        i += di;
        j += dj;
    }
    codes
}

/// The opcodes grouped into hunks with up to `context` equal items around each change, as
/// `difflib.SequenceMatcher.get_grouped_opcodes` groups them.
fn grouped(mut codes: Vec<Code>, context: usize) -> Vec<Vec<Code>> {
    if let Some(first) = codes.first_mut()
        && first.equal
    {
        first.a.0 = first.a.0.max(first.a.1.saturating_sub(context));
        first.b.0 = first.b.0.max(first.b.1.saturating_sub(context));
    }
    if let Some(last) = codes.last_mut()
        && last.equal
    {
        last.a.1 = last.a.1.min(last.a.0.saturating_add(context));
        last.b.1 = last.b.1.min(last.b.0.saturating_add(context));
    }
    let both = context.saturating_mul(2);
    let mut groups = Vec::new();
    let mut group = Vec::new();
    for mut c in codes {
        if c.equal && c.a.1 - c.a.0 > both {
            group.push(Code {
                equal: true,
                a: (c.a.0, c.a.1.min(c.a.0.saturating_add(context))),
                b: (c.b.0, c.b.1.min(c.b.0.saturating_add(context))),
            });
            groups.push(std::mem::take(&mut group));
            c.a.0 = c.a.0.max(c.a.1.saturating_sub(context));
            c.b.0 = c.b.0.max(c.b.1.saturating_sub(context));
        }
        group.push(c);
    }
    if !(group.is_empty() || (group.len() == 1 && group.first().is_some_and(|c| c.equal))) {
        groups.push(group);
    }
    groups.retain(|g| g.iter().any(|c| !c.equal));
    groups
}

/// `difflib`'s unified range: `start,length`, `start` alone for one line, `start-1,0` for none.
fn range(start: usize, end: usize) -> String {
    let len = end - start;
    match len {
        0 => format!("{start},0"),
        1 => format!("{}", start + 1),
        _ => format!("{},{len}", start + 1),
    }
}

/// Unified-diff lines: a `@@ -a,b +c,d @@` header per hunk, then ` `, `-` and `+` lines from
/// `a_line` and `b_line`. At most `limit` removed or added lines are written; the result's second
/// item is how many were left out.
pub(crate) fn unified(
    runs: &[(Op, usize)],
    context: usize,
    limit: usize,
    a_line: impl Fn(usize) -> String,
    b_line: impl Fn(usize) -> String,
) -> (Vec<String>, usize) {
    let codes = opcodes(runs);
    let changed: usize = codes.iter().filter(|c| !c.equal).map(|c| (c.a.1 - c.a.0) + (c.b.1 - c.b.0)).sum();
    let mut out = Vec::new();
    let mut written = 0usize;
    'groups: for group in grouped(codes, context) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else { continue };
        out.push(format!("@@ -{} +{} @@", range(first.a.0, last.a.1), range(first.b.0, last.b.1)));
        for c in &group {
            if c.equal {
                for i in c.a.0..c.a.1 {
                    out.push(format!(" {}", a_line(i)));
                }
                continue;
            }
            for i in c.a.0..c.a.1 {
                if written >= limit {
                    break 'groups;
                }
                out.push(format!("-{}", a_line(i)));
                written += 1;
            }
            for j in c.b.0..c.b.1 {
                if written >= limit {
                    break 'groups;
                }
                out.push(format!("+{}", b_line(j)));
                written += 1;
            }
        }
    }
    (out, changed - written)
}

/// The share of lines the two texts have in common, counted as multisets:
/// `2 × matches / (len a + len b)`, 1 for two empty texts (`difflib`'s `quick_ratio`).
pub(crate) fn quick_ratio(a: &[&str], b: &[&str]) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    let mut avail: HashMap<&str, usize> = HashMap::new();
    for line in b {
        *avail.entry(line).or_insert(0) += 1;
    }
    let mut matches = 0usize;
    for line in a {
        if let Some(n) = avail.get_mut(line)
            && *n > 0
        {
            *n -= 1;
            matches += 1;
        }
    }
    2.0 * matches as f64 / total as f64
}
