//! The charstring specialiser: generic moves, lines and curves rewritten into the shortest Type 2
//! operators (`hlineto`, `vvcurveto`, `rcurveline` …) and combined while the operand stack allows.
//!
//! A port of fontTools `cffLib/specializer.py` (`specializeCommands` with `generalizeFirst=False`
//! and `preserveTopology=False`, and the helpers it uses; MIT, Copyright (c) 2017 Just van Rossum
//! and contributors). The reference compiler specialises through it, so this follows its passes in
//! order, including its choices on ties, rather than searching for a shortest encoding of its own.
//!
//! The passes are generic over the operand ([`SpecArg`]): a plain number for `CFF `, or for
//! `CFF2` a number that may be blended (a default with one delta per region). A blended operand
//! is never zero, two blended operands add delta by delta, and while it is pushed a blended
//! operand takes its default, its deltas and the blend count on the stack, as the reference's
//! `_addArgs` and `_argsStackUse` count them.

/// An operand the specialiser can move, test for zero, add and count on the stack.
pub(crate) trait SpecArg: Clone {
    /// The plain number 0.
    fn zero() -> Self;
    /// A plain 0 (a blended operand never is).
    fn is_zero(&self) -> bool;
    /// The sum of two operands; `None` when they cannot be added (blends over different regions).
    fn add(&self, other: &Self) -> Option<Self>;
    /// Most stack slots the operand takes while it is pushed (it leaves one).
    fn peak(&self) -> usize {
        1
    }
}

impl SpecArg for f64 {
    fn zero() -> Self {
        0.0
    }
    fn is_zero(&self) -> bool {
        *self == 0.0
    }
    fn add(&self, other: &Self) -> Option<Self> {
        Some(self + other)
    }
}

/// Which components of a vector are zero: `Zero` both, `H` only y (a horizontal vector, one
/// argument: dx), `V` only x (vertical, one argument: dy), `R` neither (two arguments).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cat {
    Zero,
    H,
    V,
    R,
}

/// An operator, real or one of the specialiser's intermediate forms. `Curve(first, last)` names a
/// curve by the categories of its first and last control vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Move(Cat),
    Line(Cat),
    Curve(Cat, Cat),
    RLineCurve,
    RCurveLine,
}

/// The generic operators a pen produces.
pub(crate) const RMOVETO: Op = Op::Move(Cat::R);
pub(crate) const RLINETO: Op = Op::Line(Cat::R);
pub(crate) const RRCURVETO: Op = Op::Curve(Cat::R, Cat::R);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cmd<A = f64> {
    pub op: Op,
    pub args: Vec<A>,
}

fn categorize<A: SpecArg>(x: A, y: A) -> (Cat, Vec<A>) {
    if x.is_zero() {
        if y.is_zero() { (Cat::Zero, vec![x]) } else { (Cat::V, vec![y]) }
    } else if y.is_zero() {
        (Cat::H, vec![x])
    } else {
        (Cat::R, vec![x, y])
    }
}

fn merge(a: Cat, b: Cat) -> Option<Cat> {
    if a == Cat::Zero {
        Some(b)
    } else if b == Cat::Zero || a == b {
        Some(a)
    } else {
        None
    }
}

fn negate(a: Cat) -> Cat {
    match a {
        Cat::H => Cat::V,
        Cat::V => Cat::H,
        other => other,
    }
}

fn arg<A: SpecArg>(args: &[A], i: usize) -> A {
    args.get(i).cloned().unwrap_or_else(A::zero)
}

/// Most stack slots `args` take while they are pushed (the reference's `_argsStackUse`).
fn stack_use<A: SpecArg>(args: &[A]) -> usize {
    let mut pushed = 0usize;
    let mut peak = 0usize;
    for a in args {
        peak = peak.max(pushed.saturating_add(a.peak()));
        pushed += 1;
    }
    peak.max(pushed)
}

/// Specialise `commands` (generic `rmoveto`/`rlineto`/`rrcurveto` only) for an operand stack of
/// `max_stack`. `keep_topology` skips pass 3, so every segment stays (the reference's
/// `preserveTopology=True`).
pub(crate) fn specialize<A: SpecArg>(commands: Vec<Cmd<A>>, max_stack: usize, keep_topology: bool) -> Vec<Cmd<A>> {
    let commands = combine_moves(commands);
    let commands: Vec<Cmd<A>> = commands.into_iter().map(specialize_one).collect();
    let mut commands = if keep_topology { commands } else { drop_redundant(commands) };
    peephole(&mut commands);
    let mut commands = combine(commands, max_stack);
    for c in &mut commands {
        resolve(c);
    }
    commands
}

/// Pass 1: successive `rmoveto`s become one. The reference merges from the end, so the sum is
/// formed right to left.
fn combine_moves<A: SpecArg>(commands: Vec<Cmd<A>>) -> Vec<Cmd<A>> {
    let mut out: Vec<Cmd<A>> = Vec::with_capacity(commands.len());
    let mut run: Vec<Cmd<A>> = Vec::new();
    let flush = |run: &mut Vec<Cmd<A>>, out: &mut Vec<Cmd<A>>| {
        let Some(mut acc) = run.pop() else { return };
        // Moves that cannot be added (never in one glyph) stay apart.
        let mut kept_rev: Vec<Cmd<A>> = Vec::new();
        while let Some(prev) = run.pop() {
            let x = arg(&prev.args, 0).add(&arg(&acc.args, 0));
            let y = arg(&prev.args, 1).add(&arg(&acc.args, 1));
            if let (Some(x), Some(y)) = (x, y) {
                acc.args = vec![x, y];
            } else {
                kept_rev.push(acc);
                acc = prev;
            }
        }
        out.push(acc);
        out.extend(kept_rev.into_iter().rev());
    };
    for c in commands {
        if c.op == RMOVETO {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Pass 2: categorise every vector, turning each operator into its zero/horizontal/vertical form.
fn specialize_one<A: SpecArg>(c: Cmd<A>) -> Cmd<A> {
    match c.op {
        Op::Move(Cat::R) | Op::Line(Cat::R) => {
            let (cat, args) = categorize(arg(&c.args, 0), arg(&c.args, 1));
            let op = if c.op == RMOVETO { Op::Move(cat) } else { Op::Line(cat) };
            Cmd { op, args }
        }
        Op::Curve(Cat::R, Cat::R) if c.args.len() == 6 => {
            let (c1, mut args) = categorize(arg(&c.args, 0), arg(&c.args, 1));
            let (c2, last) = categorize(arg(&c.args, 4), arg(&c.args, 5));
            args.push(arg(&c.args, 2));
            args.push(arg(&c.args, 3));
            args.extend(last);
            Cmd { op: Op::Curve(c1, c2), args }
        }
        _ => c,
    }
}

/// Pass 3, walked from the end as the reference walks it: a curve whose first and last vectors
/// are zero becomes a line, a zero-length line is dropped, and runs of `hlineto` (or `vlineto`)
/// are summed into one.
fn drop_redundant<A: SpecArg>(commands: Vec<Cmd<A>>) -> Vec<Cmd<A>> {
    let mut out_rev: Vec<Cmd<A>> = Vec::with_capacity(commands.len());
    let mut rest = commands.into_iter().rev().peekable();
    let mut current = rest.next();
    while let Some(mut c) = current {
        if c.op == Op::Curve(Cat::Zero, Cat::Zero) {
            let (cat, args) = categorize(arg(&c.args, 1), arg(&c.args, 2));
            c = Cmd { op: Op::Line(cat), args };
        }
        if c.op == Op::Line(Cat::Zero) {
            current = rest.next();
            continue;
        }
        if matches!(c.op, Op::Line(Cat::H) | Op::Line(Cat::V))
            && let Some(prev) = rest.peek()
            && prev.op == c.op
            && let Some(sum) = arg(&c.args, 0).add(&arg(&prev.args, 0))
        {
            rest.next();
            current = Some(Cmd { op: c.op, args: vec![sum] });
            continue;
        }
        out_rev.push(c);
        current = rest.next();
    }
    out_rev.reverse();
    out_rev
}

/// Pass 4: an h/v form between two generic ones of its kind goes back to the generic form, which
/// lets the three combine later.
fn peephole<A: SpecArg>(commands: &mut [Cmd<A>]) {
    let len = commands.len();
    for i in 1..len.saturating_sub(1) {
        let (Some(prv), Some(nxt)) = (commands.get(i - 1).map(|c| c.op), commands.get(i + 1).map(|c| c.op)) else {
            continue;
        };
        let Some(c) = commands.get_mut(i) else { continue };
        match c.op {
            Op::Line(Cat::Zero | Cat::H | Cat::V) if prv == RLINETO && nxt == RLINETO => {
                let a = arg(&c.args, 0);
                c.args = if c.op == Op::Line(Cat::V) { vec![A::zero(), a] } else { vec![a, A::zero()] };
                c.op = RLINETO;
            }
            Op::Curve(first, last) if c.args.len() == 5 && prv == RRCURVETO && nxt == RRCURVETO => {
                let pos = if first == Cat::V {
                    0
                } else if first != Cat::R {
                    1
                } else if last == Cat::V {
                    4
                } else {
                    5
                };
                c.args.insert(pos.min(c.args.len()), A::zero());
                c.op = RRCURVETO;
            }
            _ => {}
        }
    }
}

/// What pass 5 does with a pair of neighbours.
enum Pair {
    Merge(Op),
    /// Not combinable.
    Keep,
    /// Not combinable, and the reference also leaves its running stack count untouched.
    Skip,
}

fn pair(op1: Op, op2: Op, len2: usize) -> Pair {
    let generic = |op: Op| op == RLINETO || op == RRCURVETO;
    if generic(op1) && generic(op2) {
        if op1 == op2 {
            return Pair::Merge(op1);
        }
        if op2 == RRCURVETO && len2 == 6 {
            return Pair::Merge(Op::RLineCurve);
        }
        if len2 == 2 {
            return Pair::Merge(Op::RCurveLine);
        }
        return Pair::Keep;
    }
    if (op1 == RLINETO && op2 == Op::RLineCurve) || (op1 == RRCURVETO && op2 == Op::RCurveLine) {
        return Pair::Merge(op2);
    }
    if matches!((op1, op2), (Op::Line(Cat::V), Op::Line(Cat::H)) | (Op::Line(Cat::H), Op::Line(Cat::V))) {
        return Pair::Merge(op1);
    }
    if let (Op::Curve(d0, d1), Op::Curve(d2, d3)) = (op1, op2) {
        if d1 == Cat::R || d2 == Cat::R || (d0 == Cat::R && d3 == Cat::R) {
            return Pair::Skip;
        }
        let Some(d) = merge(d1, d2) else { return Pair::Skip };
        if d0 == Cat::R {
            return match merge(d, d3) {
                Some(d) => Pair::Merge(Op::Curve(Cat::R, d)),
                None => Pair::Skip,
            };
        }
        if d3 == Cat::R {
            return match merge(d0, negate(d)) {
                Some(d0) => Pair::Merge(Op::Curve(d0, Cat::R)),
                None => Pair::Skip,
            };
        }
        return match merge(d0, d3) {
            Some(d0) => Pair::Merge(Op::Curve(d0, d)),
            None => Pair::Skip,
        };
    }
    Pair::Keep
}

/// Pass 5, from the end: neighbours combine into one operator while the combined operands stay
/// below `max_stack` (one slot is kept free, as the reference keeps it for subroutine calls).
fn combine<A: SpecArg>(commands: Vec<Cmd<A>>, max_stack: usize) -> Vec<Cmd<A>> {
    let mut rest = commands.into_iter().rev();
    let Some(mut current) = rest.next() else { return Vec::new() };
    let mut out_rev: Vec<Cmd<A>> = Vec::new();
    let mut used = stack_use(&current.args);
    for prev in rest {
        let args1_use = stack_use(&prev.args);
        match pair(prev.op, current.op, current.args.len()) {
            Pair::Merge(op) => {
                let combined = args1_use.max(prev.args.len().saturating_add(used));
                if combined < max_stack {
                    let mut args = prev.args;
                    args.extend(current.args);
                    current = Cmd { op, args };
                    used = combined;
                } else {
                    out_rev.push(current);
                    current = prev;
                    used = args1_use;
                }
            }
            Pair::Keep => {
                out_rev.push(current);
                current = prev;
                used = args1_use;
            }
            Pair::Skip => {
                out_rev.push(current);
                current = prev;
            }
        }
    }
    out_rev.push(current);
    out_rev.reverse();
    out_rev
}

/// Pass 6: the intermediate forms become real operators, reordering arguments where the real
/// operator expects them in another order.
fn resolve<A>(c: &mut Cmd<A>) {
    match c.op {
        Op::Move(Cat::Zero) => c.op = Op::Move(Cat::H),
        Op::Line(Cat::Zero) => c.op = Op::Line(Cat::H),
        Op::Curve(a, b) => {
            let real = matches!((a, b), (Cat::R, Cat::R) | (Cat::H, Cat::H) | (Cat::V, Cat::V) | (Cat::V, Cat::H) | (Cat::H, Cat::V));
            if real {
                return;
            }
            let l = c.args.len();
            let mut op0 = if a == Cat::Zero { Cat::H } else { a };
            let mut op1 = if b == Cat::Zero { Cat::H } else { b };
            if op0 == Cat::R {
                op0 = op1;
            }
            if op1 == Cat::R {
                op1 = negate(op0);
            }
            if l % 2 == 1 {
                if op0 != op1 {
                    if ((op0 == Cat::H) ^ (l % 8 == 1)) && l >= 2 {
                        c.args.swap(l - 2, l - 1);
                    }
                } else if op0 == Cat::H && l >= 2 {
                    c.args.swap(0, 1);
                }
            }
            c.op = Op::Curve(op0, op1);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(op: Op, args: &[f64]) -> Cmd {
        Cmd { op, args: args.to_vec() }
    }

    fn spec(cmds: &[Cmd]) -> Vec<(Op, Vec<f64>)> {
        specialize(cmds.to_vec(), 48, false).into_iter().map(|c| (c.op, c.args)).collect()
    }

    const HMOVETO: Op = Op::Move(Cat::H);
    const VMOVETO: Op = Op::Move(Cat::V);
    const HLINETO: Op = Op::Line(Cat::H);
    const HVCURVETO: Op = Op::Curve(Cat::H, Cat::V);
    const VHCURVETO: Op = Op::Curve(Cat::V, Cat::H);
    const HHCURVETO: Op = Op::Curve(Cat::H, Cat::H);
    const VVCURVETO: Op = Op::Curve(Cat::V, Cat::V);

    // Expected outputs below are what fontTools' `specializeCommands(…, generalizeFirst=False)`
    // returns for the same input.

    #[test]
    fn moves_specialise_and_combine() {
        let got = spec(&[cmd(RMOVETO, &[10.0, 0.0]), cmd(RMOVETO, &[5.0, 0.0])]);
        assert_eq!(got, vec![(HMOVETO, vec![15.0])]);
        assert_eq!(spec(&[cmd(RMOVETO, &[0.0, 7.0])]), vec![(VMOVETO, vec![7.0])]);
        assert_eq!(spec(&[cmd(RMOVETO, &[0.0, 0.0])]), vec![(HMOVETO, vec![0.0])]);
    }

    #[test]
    fn a_box_alternates_h_and_v_lines() {
        let got = spec(&[cmd(RMOVETO, &[100.0, 0.0]), cmd(RLINETO, &[300.0, 0.0]), cmd(RLINETO, &[0.0, 700.0]), cmd(RLINETO, &[-300.0, 0.0])]);
        assert_eq!(got, vec![(HMOVETO, vec![100.0]), (HLINETO, vec![300.0, 700.0, -300.0])]);
    }

    #[test]
    fn collinear_lines_are_summed_and_zero_lines_dropped() {
        let got = spec(&[
            cmd(RMOVETO, &[0.0, 0.0]),
            cmd(RLINETO, &[10.0, 0.0]),
            cmd(RLINETO, &[20.0, 0.0]),
            cmd(RLINETO, &[0.0, 0.0]),
            cmd(RLINETO, &[5.0, 5.0]),
        ]);
        assert_eq!(got, vec![(HMOVETO, vec![0.0]), (HLINETO, vec![30.0]), (RLINETO, vec![5.0, 5.0])]);
    }

    #[test]
    fn keeping_the_topology_keeps_every_segment() {
        // Expected values from fontTools' specializeCommands with preserveTopology=True.
        let keep = |cmds: &[Cmd]| -> Vec<(Op, Vec<f64>)> { specialize(cmds.to_vec(), 48, true).into_iter().map(|c| (c.op, c.args)).collect() };
        let lines =
            [cmd(RMOVETO, &[0.0, 0.0]), cmd(RLINETO, &[10.0, 0.0]), cmd(RLINETO, &[20.0, 0.0]), cmd(RLINETO, &[0.0, 0.0]), cmd(RLINETO, &[5.0, 5.0])];
        assert_eq!(
            keep(&lines),
            vec![(HMOVETO, vec![0.0]), (HLINETO, vec![10.0]), (HLINETO, vec![20.0]), (HLINETO, vec![0.0]), (RLINETO, vec![5.0, 5.0])]
        );
        let flat = [
            cmd(RMOVETO, &[529.0, 1068.0]),
            cmd(RRCURVETO, &[0.0, 0.0, -1.0, 0.0, 0.0, 0.0]),
            cmd(RLINETO, &[0.0, -253.0]),
            cmd(RRCURVETO, &[0.0, 0.0, 1.0, 0.0, 0.0, 0.0]),
            cmd(RLINETO, &[100.0, 50.0]),
        ];
        assert_eq!(
            keep(&flat),
            vec![
                (RMOVETO, vec![529.0, 1068.0]),
                (HHCURVETO, vec![0.0, -1.0, 0.0, 0.0]),
                (Op::Line(Cat::V), vec![-253.0]),
                (HHCURVETO, vec![0.0, 1.0, 0.0, 0.0]),
                (RLINETO, vec![100.0, 50.0]),
            ]
        );
        assert_eq!(spec(&flat), vec![(RMOVETO, vec![529.0, 1068.0]), (HLINETO, vec![-1.0, -253.0, 1.0]), (RLINETO, vec![100.0, 50.0])]);
    }

    #[test]
    fn a_circle_becomes_alternating_hv_curves() {
        let got = spec(&[
            cmd(RMOVETO, &[0.0, 100.0]),
            cmd(RRCURVETO, &[0.0, 55.0, 45.0, 45.0, 55.0, 0.0]),
            cmd(RRCURVETO, &[55.0, 0.0, 45.0, -45.0, 0.0, -55.0]),
            cmd(RRCURVETO, &[0.0, -55.0, -45.0, -45.0, -55.0, 0.0]),
            cmd(RRCURVETO, &[-55.0, 0.0, -45.0, 45.0, 0.0, 55.0]),
        ]);
        assert_eq!(
            got,
            vec![
                (VMOVETO, vec![100.0]),
                (VHCURVETO, vec![55.0, 45.0, 45.0, 55.0, 55.0, 45.0, -45.0, -55.0, -55.0, -45.0, -45.0, -55.0, -55.0, -45.0, 45.0, 55.0]),
            ]
        );
    }

    #[test]
    fn hh_and_vv_runs_and_the_odd_argument() {
        // A curve starting with an r vector and ending horizontal, then a horizontal one.
        let got = spec(&[
            cmd(RMOVETO, &[1.0, 1.0]),
            cmd(RRCURVETO, &[10.0, 5.0, 20.0, 0.0, 30.0, 0.0]),
            cmd(RRCURVETO, &[10.0, 0.0, 20.0, 0.0, 30.0, 0.0]),
        ]);
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (HHCURVETO, vec![5.0, 10.0, 20.0, 0.0, 30.0, 10.0, 20.0, 0.0, 30.0])]);
        let got = spec(&[cmd(RMOVETO, &[1.0, 1.0]), cmd(RRCURVETO, &[0.0, 10.0, 0.0, 20.0, 0.0, 30.0])]);
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (VVCURVETO, vec![10.0, 0.0, 20.0, 30.0])]);
    }

    #[test]
    fn a_curve_ending_in_r_becomes_hv_with_the_last_pair_swapped() {
        let got = spec(&[cmd(RMOVETO, &[1.0, 1.0]), cmd(RRCURVETO, &[10.0, 0.0, 20.0, 20.0, 5.0, 30.0])]);
        // hrcurveto → hvcurveto with dxa dxb dyb dyc dxc (the reference's swap of the last two).
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (HVCURVETO, vec![10.0, 20.0, 20.0, 30.0, 5.0])]);
    }

    #[test]
    fn lines_and_curves_join_into_rcurveline_and_rlinecurve() {
        let got = spec(&[cmd(RMOVETO, &[1.0, 1.0]), cmd(RRCURVETO, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), cmd(RLINETO, &[7.0, 8.0])]);
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (Op::RCurveLine, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])]);
        let got = spec(&[cmd(RMOVETO, &[1.0, 1.0]), cmd(RLINETO, &[7.0, 8.0]), cmd(RRCURVETO, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]);
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (Op::RLineCurve, vec![7.0, 8.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]);
    }

    #[test]
    fn a_horizontal_line_between_generic_lines_turns_generic() {
        let got = spec(&[cmd(RMOVETO, &[1.0, 1.0]), cmd(RLINETO, &[1.0, 2.0]), cmd(RLINETO, &[5.0, 0.0]), cmd(RLINETO, &[3.0, 4.0])]);
        assert_eq!(got, vec![(RMOVETO, vec![1.0, 1.0]), (RLINETO, vec![1.0, 2.0, 5.0, 0.0, 3.0, 4.0])]);
    }

    #[test]
    fn combining_stops_below_the_stack_limit() {
        let mut cmds = vec![cmd(RMOVETO, &[1.0, 1.0])];
        for i in 0..30 {
            cmds.push(cmd(RLINETO, &[1.0 + i as f64, 2.0]));
        }
        let got = specialize(cmds, 48, false);
        assert!(got.iter().all(|c| c.args.len() < 48));
        // The reference fills the last operator first: 23 pairs (46 arguments), then 7.
        let lens: Vec<usize> = got.iter().map(|c| c.args.len()).collect();
        assert_eq!(lens, vec![2, 14, 46]);
    }
}
