//! The TrueType execution model in the editor: which points are touched, per direction, after
//! the first n instructions of a layer. Commands validate selections against it (Snap needs
//! untouched points, Shift a touched origin…).
//!
//! Rows: `hinting.tt.hintOrder`, `hinting.tt.touchedPoints`, `hinting.tt.iup` (the model).
//!
//! Instructions run in list order. Each one touches points in its own direction only:
//!
//! | Type | Needs before it runs | Touches |
//! |---|---|---|
//! | Snap | an untouched target | the target |
//! | Stem | an origin; origin and target not both touched | origin and target |
//! | Shift | a touched origin, an untouched target | the target |
//! | Interpolate | two touched references, an untouched target | the target |
//! | Delta | nothing | nothing |
//!
//! A Delta moves its point only at the sizes it lists, so at every other size the point stays as
//! it was; the model therefore leaves it untouched (a guess on `hinting.tt.touchedPoints`). The
//! untouched points that remain after the last instruction of a direction are the ones the
//! final interpolation (IUP) moves. The work is linear in the instructions replayed, which
//! [`crate::tt_instructions`] caps at [`crate::MAX_LAYER_HINTS`], each naming at most three points.

use std::collections::BTreeSet;

use fontcraft_model::Layer;

use crate::{Direction, HintingError, TtInstruction, TtKind, TtPoint, tt_instructions};

/// The touched points in each direction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchedState {
    /// Touched by instructions in the horizontal direction (moved along x).
    pub horizontal: BTreeSet<TtPoint>,
    /// Touched by instructions in the vertical direction (moved along y).
    pub vertical: BTreeSet<TtPoint>,
}

impl TouchedState {
    /// The state after the first `upto` instructions (all of them when `upto` is larger).
    pub fn replay(instructions: &[TtInstruction], upto: usize) -> Self {
        let mut state = Self::default();
        for t in instructions.iter().take(upto) {
            state.run(t);
        }
        state
    }

    /// The state after the first `upto` instructions of `layer` (all of them when `upto` is
    /// larger).
    pub fn of_layer(layer: &Layer, upto: usize) -> Result<Self, HintingError> {
        let list: Vec<TtInstruction> = tt_instructions(layer)?.into_iter().map(|(_, t)| t).collect();
        Ok(Self::replay(&list, upto))
    }

    pub fn is_touched(&self, point: &TtPoint, direction: Direction) -> bool {
        self.set(direction).contains(point)
    }

    /// Runs one instruction: marks the points it touches, whether or not its needs are met.
    pub fn run(&mut self, t: &TtInstruction) {
        let touched: Vec<TtPoint> = match t.kind {
            TtKind::Snap { .. } | TtKind::Shift | TtKind::Interpolate => vec![t.target],
            TtKind::Stem { .. } => t.origin.iter().copied().chain(std::iter::once(t.target)).collect(),
            TtKind::Delta { .. } => Vec::new(),
        };
        let set = match t.direction {
            Direction::Horizontal => &mut self.horizontal,
            Direction::Vertical => &mut self.vertical,
        };
        set.extend(touched);
    }

    /// Whether `t` may run in this state; the error says which need is not met.
    pub fn check(&self, t: &TtInstruction) -> Result<(), String> {
        let d = t.direction;
        let touched = |p: &TtPoint| self.is_touched(p, d);
        let distinct = t.points().iter().collect::<BTreeSet<_>>().len() == t.points().len();
        if !distinct {
            return Err("an instruction names each of its points once".into());
        }
        match t.kind {
            TtKind::Snap { .. } if touched(&t.target) => Err(format!("{} is touched already: Snap needs an untouched point", describe(&t.target))),
            TtKind::Stem { .. } => match t.origin {
                None => Err("a Stem instruction needs an origin".into()),
                Some(o) if touched(&o) && touched(&t.target) => Err(format!(
                    "{} and {} are both touched: a Stem instruction needs two untouched points, or one touched and one untouched",
                    describe(&o),
                    describe(&t.target)
                )),
                Some(_) => Ok(()),
            },
            TtKind::Shift => match t.origin {
                Some(o) if !touched(&o) => Err(format!("{} is not touched: a Shift instruction needs a touched origin", describe(&o))),
                None => Err("a Shift instruction needs an origin".into()),
                Some(_) if touched(&t.target) => Err(format!("{} is touched already: Shift needs an untouched target", describe(&t.target))),
                Some(_) => Ok(()),
            },
            TtKind::Interpolate => match (t.origin, t.other) {
                (Some(a), Some(b)) => match [a, b].into_iter().find(|p| !touched(p)) {
                    Some(p) => Err(format!("{} is not touched: Interpolate needs two touched references", describe(&p))),
                    None if touched(&t.target) => Err(format!("{} is touched already: Interpolate needs an untouched target", describe(&t.target))),
                    None => Ok(()),
                },
                _ => Err("an Interpolate instruction needs two reference points".into()),
            },
            _ => Ok(()),
        }
    }

    fn set(&self, direction: Direction) -> &BTreeSet<TtPoint> {
        match direction {
            Direction::Horizontal => &self.horizontal,
            Direction::Vertical => &self.vertical,
        }
    }
}

/// Every instruction whose needs are not met by the ones before it, with the reason, in list
/// order. A list the commands built has none; one read from a file, or left by deleting an
/// instruction a later one builds on, may have some.
pub fn unmet_needs(instructions: &[TtInstruction]) -> Vec<(usize, String)> {
    let mut state = TouchedState::default();
    let mut out = Vec::new();
    for (i, t) in instructions.iter().enumerate() {
        if let Err(e) = state.check(t) {
            out.push((i, e));
        }
        state.run(t);
    }
    out
}

/// A point as messages name it: `[path, node]` in the model's order, `lsb` or `rsb`.
pub fn describe(p: &TtPoint) -> String {
    match p {
        TtPoint::Node(r) => format!("point [{}, {}]", r.path, r.node),
        TtPoint::Intersection { a, b } => format!("the intersection [{}, {}, {}, {}]", a.path, a.node, b.path, b.node),
        TtPoint::Lsb => "lsb".into(),
        TtPoint::Rsb => "rsb".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeRef, StemRef, ZoneRef};

    fn p(node: usize) -> TtPoint {
        TtPoint::Node(NodeRef::new(0, node))
    }

    fn snap(node: usize) -> TtInstruction {
        TtInstruction::new(TtKind::Snap { zone: ZoneRef::Auto }, Direction::Vertical, p(node))
    }

    fn stem(origin: usize, target: usize) -> TtInstruction {
        let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Auto, triple: None }, Direction::Vertical, p(target));
        t.origin = Some(p(origin));
        t
    }

    fn shift(origin: usize, target: usize) -> TtInstruction {
        let mut t = TtInstruction::new(TtKind::Shift, Direction::Vertical, p(target));
        t.origin = Some(p(origin));
        t
    }

    #[test]
    fn replay_follows_list_order_and_direction() {
        let mut h = snap(5);
        h.direction = Direction::Horizontal;
        let list = vec![snap(0), stem(0, 1), shift(1, 2), h];
        let s1 = TouchedState::replay(&list, 1);
        assert!(s1.is_touched(&p(0), Direction::Vertical) && !s1.is_touched(&p(1), Direction::Vertical));
        let all = TouchedState::replay(&list, usize::MAX);
        assert_eq!(all.vertical, [p(0), p(1), p(2)].into_iter().collect());
        assert_eq!(all.horizontal, [p(5)].into_iter().collect());
        assert!(!all.is_touched(&p(5), Direction::Vertical));
    }

    #[test]
    fn needs_are_checked_against_earlier_instructions() {
        assert!(unmet_needs(&[snap(0), stem(0, 1), shift(1, 2)]).is_empty());
        // A Shift whose origin nothing touched, a Snap on a touched point, a Stem on two touched points.
        let bad = unmet_needs(&[shift(3, 4), snap(4), snap(0), stem(0, 4)]);
        assert_eq!(bad.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 3]);
        let delta = TtInstruction::new(TtKind::Delta { cells: Default::default() }, Direction::Vertical, p(7));
        assert!(!TouchedState::replay(&[delta], 1).is_touched(&p(7), Direction::Vertical));
    }
}
