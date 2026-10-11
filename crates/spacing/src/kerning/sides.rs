//! Kerning sides: the command spelling (`A`, `@X`) and the stored spelling (`A`, `@MMK_L_X`).

use fontcraft_model::{KernGroups, MAX_NAME_LEN, side_group};

use super::groups::{GroupNameError, check_group_name};
use super::{Direction, Position};

/// A glyph's side, as far as kerning groups go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl GroupSide {
    pub const ALL: [GroupSide; 4] = [GroupSide::Left, GroupSide::Right, GroupSide::Top, GroupSide::Bottom];

    /// `left`, `right`, `top`, `bottom`.
    pub fn name(self) -> &'static str {
        match self {
            GroupSide::Left => "left",
            GroupSide::Right => "right",
            GroupSide::Top => "top",
            GroupSide::Bottom => "bottom",
        }
    }

    pub fn from_name(name: &str) -> Option<GroupSide> {
        GroupSide::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The glyph's group on this side.
    pub fn of(self, groups: &KernGroups) -> Option<&str> {
        match self {
            GroupSide::Left => groups.left.as_deref(),
            GroupSide::Right => groups.right.as_deref(),
            GroupSide::Top => groups.top.as_deref(),
            GroupSide::Bottom => groups.bottom.as_deref(),
        }
    }

    pub(crate) fn slot(self) -> usize {
        match self {
            GroupSide::Left => 0,
            GroupSide::Right => 1,
            GroupSide::Top => 2,
            GroupSide::Bottom => 3,
        }
    }
}

/// The side of a glyph that faces the pair when the glyph is at `position`. Right-to-left pairs
/// are in logical order, so their first glyph stands on the right and faces the pair with its
/// left side.
pub fn facing(direction: Direction, position: Position) -> GroupSide {
    match (direction, position) {
        (Direction::Ltr, Position::First) | (Direction::Rtl, Position::Second) => GroupSide::Right,
        (Direction::Ltr, Position::Second) | (Direction::Rtl, Position::First) => GroupSide::Left,
        (Direction::Vertical, Position::First) => GroupSide::Bottom,
        (Direction::Vertical, Position::Second) => GroupSide::Top,
    }
}

/// The stored prefix of a group side at `position`: it names where the side stands in the pair
/// as drawn (`@MMK_L_` on the left, `@MMK_R_` on the right, `@MMK_T_` on top, `@MMK_B_` below).
pub fn prefix(direction: Direction, position: Position) -> &'static str {
    match (direction, position) {
        (Direction::Ltr, Position::First) | (Direction::Rtl, Position::Second) => "@MMK_L_",
        (Direction::Ltr, Position::Second) | (Direction::Rtl, Position::First) => "@MMK_R_",
        (Direction::Vertical, Position::First) => "@MMK_T_",
        (Direction::Vertical, Position::Second) => "@MMK_B_",
    }
}

/// One side of a pair: a glyph or a group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Glyph(String),
    Group(String),
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SideError {
    #[error("a kerning side may not be empty")]
    Empty,
    #[error("a glyph name may be at most {MAX_NAME_LEN} bytes long")]
    TooLong,
    #[error("a glyph name may not hold a control character (byte {0})")]
    Control(usize),
    #[error(transparent)]
    Group(#[from] GroupNameError),
}

impl Side {
    /// Reads a command side: `@X` is the group X, anything else a glyph name. The group name is
    /// checked, and a glyph name may not hold control characters; whether the glyph exists is for
    /// the caller to decide.
    pub fn parse(text: &str) -> Result<Side, SideError> {
        if let Some(group) = text.strip_prefix('@') {
            check_group_name(group)?;
            return Ok(Side::Group(group.to_string()));
        }
        if text.is_empty() {
            return Err(SideError::Empty);
        }
        if text.len() > MAX_NAME_LEN {
            return Err(SideError::TooLong);
        }
        // Tabs and line breaks would split a kerning-text line.
        if let Some((at, _)) = text.char_indices().find(|(_, c)| c.is_control()) {
            return Err(SideError::Control(at));
        }
        Ok(Side::Glyph(text.to_string()))
    }

    /// The command spelling: `A` or `@X`.
    pub fn command(&self) -> String {
        match self {
            Side::Glyph(g) => g.clone(),
            Side::Group(g) => format!("@{g}"),
        }
    }

    /// The stored spelling at `position` in `direction`.
    pub fn stored(&self, direction: Direction, position: Position) -> String {
        match self {
            Side::Glyph(g) => g.clone(),
            Side::Group(g) => format!("{}{g}", prefix(direction, position)),
        }
    }

    /// Reads a stored side in any spelling the model keeps (`@MMK_…`, `public.kern1.…`). The
    /// group is taken by its place in the pair, whatever its prefix says.
    pub fn from_stored(stored: &str) -> Side {
        match side_group(stored) {
            Some(g) => Side::Group(g.to_string()),
            None => Side::Glyph(stored.to_string()),
        }
    }

    /// The glyph or group name, without `@`.
    pub fn name(&self) -> &str {
        match self {
            Side::Glyph(n) | Side::Group(n) => n,
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Side::Group(_))
    }
}
