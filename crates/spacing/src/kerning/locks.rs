//! Kerning locks: derived from the pair that applies, never stored.

use super::lookup::{Applied, KernError, KernIndex};
use super::sides::{Side, facing};
use super::{Direction, Position};

/// The lock of one side of a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// Edits go to the glyph's group on the side facing the pair.
    Closed,
    /// Edits go to the glyph itself: the pair that applies names the glyph although it has a
    /// group on that side.
    Open,
    /// The glyph has no group on that side, so edits go to the glyph itself.
    Ungrouped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locks {
    pub first: Lock,
    pub second: Lock,
}

/// Which locks of a pair a command acts on (`left`, `right`, `both`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    First,
    Second,
    Both,
}

impl Which {
    /// The names used in command parameters: `left`, `right`, `both`.
    pub fn from_name(name: &str) -> Option<Which> {
        match name {
            "left" => Some(Which::First),
            "right" => Some(Which::Second),
            "both" => Some(Which::Both),
            _ => None,
        }
    }

    fn has(self, position: Position) -> bool {
        matches!((self, position), (Which::Both, _) | (Which::First, Position::First) | (Which::Second, Position::Second))
    }
}

/// A pair to store, with its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub first: Side,
    pub second: Side,
    pub value: f64,
}

fn lock_of(index: &KernIndex, direction: Direction, glyph: &str, position: Position, applied: Option<&Applied<'_>>) -> Lock {
    if index.group(glyph, facing(direction, position)).is_none() {
        Lock::Ungrouped
    } else if applied.is_some_and(|a| a.step.names_glyph(position)) {
        Lock::Open
    } else {
        Lock::Closed
    }
}

fn derive<'a>(
    index: &'a KernIndex,
    direction: Direction,
    master: &str,
    first: &str,
    second: &str,
) -> Result<(Option<Applied<'a>>, Locks), KernError> {
    let applied = index.lookup(direction, master, first, second)?;
    let locks = Locks {
        first: lock_of(index, direction, first, Position::First, applied.as_ref()),
        second: lock_of(index, direction, second, Position::Second, applied.as_ref()),
    };
    Ok((applied, locks))
}

/// The lock states of the pair of glyphs `first`, `second` in `master` and `direction`.
pub fn locks(index: &KernIndex, direction: Direction, master: &str, first: &str, second: &str) -> Result<Locks, KernError> {
    derive(index, direction, master, first, second).map(|(_, l)| l)
}

/// The pair a lock state names: a closed side is the glyph's group on the side facing the pair,
/// any other side the glyph itself.
pub fn locked_pair(index: &KernIndex, direction: Direction, first: &str, second: &str, locks: Locks) -> (Side, Side) {
    let side = |glyph: &str, position: Position, lock: Lock| match (lock, index.group(glyph, facing(direction, position))) {
        (Lock::Closed, Some(g)) => Side::Group(g.to_string()),
        _ => Side::Glyph(glyph.to_string()),
    };
    (side(first, Position::First, locks.first), side(second, Position::Second, locks.second))
}

/// What a kerning shortcut writes: the pair the current locks name, at the value that applies
/// plus `amount`. With no pair applying, that is the most general pair the groups allow.
pub fn shortcut_edit(index: &KernIndex, direction: Direction, master: &str, first: &str, second: &str, amount: f64) -> Result<Edit, KernError> {
    if !amount.is_finite() {
        return Err(KernError::NotFinite);
    }
    let (applied, locks) = derive(index, direction, master, first, second)?;
    let value = applied.map_or(0.0, |a| a.value) + amount;
    if !value.is_finite() {
        return Err(KernError::NotFinite);
    }
    let (first, second) = locked_pair(index, direction, first, second, locks);
    Ok(Edit { first, second, value })
}

/// The exception that opening the `which` locks stores: the pair the opened locks name, at the
/// value that applies, so the kerning does not change. `None` when none of those locks is closed.
pub fn open_locks(index: &KernIndex, direction: Direction, master: &str, first: &str, second: &str, which: Which) -> Result<Option<Edit>, KernError> {
    let (applied, locks) = derive(index, direction, master, first, second)?;
    let open = |lock: Lock, position: Position| if lock == Lock::Closed && which.has(position) { Lock::Open } else { lock };
    let opened = Locks { first: open(locks.first, Position::First), second: open(locks.second, Position::Second) };
    if opened == locks {
        return Ok(None);
    }
    let (first, second) = locked_pair(index, direction, first, second, opened);
    Ok(Some(Edit { first, second, value: applied.map_or(0.0, |a| a.value) }))
}

/// The exception that closing the `which` locks deletes: the pair that applies, when one of those
/// locks is open. Deleting it lets the group kerning apply again, so the value can change.
pub fn close_locks<'a>(
    index: &'a KernIndex,
    direction: Direction,
    master: &str,
    first: &str,
    second: &str,
    which: Which,
) -> Result<Option<Applied<'a>>, KernError> {
    let (applied, locks) = derive(index, direction, master, first, second)?;
    let open = (which.has(Position::First) && locks.first == Lock::Open) || (which.has(Position::Second) && locks.second == Lock::Open);
    Ok(applied.filter(|_| open))
}
