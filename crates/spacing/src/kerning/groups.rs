//! Kerning groups: the name check and the members of a group on one side.

use fontcraft_model::{Font, MAX_NAME_LEN};

use super::sides::GroupSide;

/// Longest kerning group name, in bytes (the model's name limit).
pub const MAX_GROUP_NAME_LEN: usize = MAX_NAME_LEN;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum GroupNameError {
    #[error("a kerning group name may not be empty")]
    Empty,
    #[error("a kerning group name may be at most {MAX_GROUP_NAME_LEN} bytes long")]
    TooLong,
    /// `position` counts characters from 1.
    #[error("a kerning group name may hold only A–Z, a–z, 0–9, `_`, `.` and `-`, not `{ch}` (character {position})")]
    Character { ch: char, position: usize },
}

/// Checks a group name, without the `@`.
pub fn check_group_name(name: &str) -> Result<(), GroupNameError> {
    if name.is_empty() {
        return Err(GroupNameError::Empty);
    }
    if name.len() > MAX_GROUP_NAME_LEN {
        return Err(GroupNameError::TooLong);
    }
    match name.chars().enumerate().find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))) {
        Some((i, ch)) => Err(GroupNameError::Character { ch, position: i + 1 }),
        None => Ok(()),
    }
}

/// The glyphs whose group on `side` is `group`, in glyph order.
pub fn members<'a>(font: &'a Font, side: GroupSide, group: &str) -> Vec<&'a str> {
    font.glyphs.iter().filter(|g| side.of(&g.kern_groups) == Some(group)).map(|g| g.name.as_str()).collect()
}

/// Every group on `side` with its members, groups in the order of their first member, members in
/// glyph order.
pub fn group_list(font: &Font, side: GroupSide) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut at = std::collections::HashMap::new();
    for g in font.glyphs.iter() {
        let Some(group) = side.of(&g.kern_groups) else { continue };
        let i = *at.entry(group).or_insert_with(|| {
            out.push((group, Vec::new()));
            out.len() - 1
        });
        if let Some((_, list)) = out.get_mut(i) {
            list.push(g.name.as_str());
        }
    }
    out
}
