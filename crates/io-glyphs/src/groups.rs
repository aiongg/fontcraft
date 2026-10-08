//! Groups that are not kerning groups (a UFO's other `groups.plist` entries), as glyphsLib 6.15.0
//! keeps them in a `.glyphs` file (`builder/groups.py`, Apache-2.0; see `NOTICE`): classes after
//! the feature classes, each with its members joined by single spaces, named in order by the font
//! user data key `com.schriftgestaltung.Glyphs.groupsNotInFeature`.
//!
//! Reading takes the named classes back out as [`Group`]s only when they are the last classes, in
//! the order the key lists them, with nothing but a name and code, and when writing them back
//! gives the same text (the key at its sorted place in the user data). Otherwise the classes and
//! the key stay as they are.

use std::borrow::Cow;

use fontcraft_model::{Font, Group, Snippet, UserData};
use fontcraft_plist::{Str, Value};

use crate::design::same_text;
use crate::{list, string};

/// The font user data key that names the classes that are groups.
pub(crate) const GROUPS_KEY: &str = "com.schriftgestaltung.Glyphs.groupsNotInFeature";

/// A class's code as group members: split at single spaces, as glyphsLib splits it.
fn members_of(code: &str) -> Vec<String> {
    if code.is_empty() { Vec::new() } else { code.split(' ').map(str::to_string).collect() }
}

/// A group as the class glyphsLib writes for it.
fn as_class(g: &Group) -> Snippet {
    Snippet { name: g.name.clone(), code: g.members.join(" "), ..Default::default() }
}

/// The user data with the key naming `groups`, at its sorted place (or in place of the one there).
fn with_key(user: &UserData, groups: &[Group]) -> UserData {
    let mut out = user.clone();
    let value = list(groups.iter().map(|g| string(&g.name)).collect());
    let entries = &mut out.0.entries;
    match entries.iter().position(|(k, _)| k.as_str() == GROUPS_KEY) {
        Some(i) => {
            if let Some(slot) = entries.get_mut(i) {
                slot.1 = value;
            }
        }
        None => {
            let at = entries.iter().position(|(k, _)| k.as_str().as_bytes() > GROUPS_KEY.as_bytes()).unwrap_or(entries.len());
            entries.insert(at, (Str::new(GROUPS_KEY), value));
        }
    }
    out
}

/// Takes the groups out of the classes and the font user data, when the file names them.
pub(crate) fn take_groups(classes: &mut Vec<Snippet>, user: &mut UserData) -> Vec<Group> {
    let Some(Value::Array(a)) = user.get(GROUPS_KEY) else { return Vec::new() };
    let Some(names) = a.items.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<Vec<String>>>() else { return Vec::new() };
    if names.is_empty() || names.len() > classes.len() {
        return Vec::new();
    }
    let start = classes.len() - names.len();
    let Some(tail) = classes.get(start..) else { return Vec::new() };
    let plain = |c: &Snippet| !c.automatic && !c.disabled && c.notes.is_empty() && c.labels.is_empty() && c.file_name.is_none() && c.extra.is_empty();
    if !tail.iter().zip(&names).all(|(c, n)| c.name == *n && plain(c)) {
        return Vec::new();
    }
    let groups: Vec<Group> = tail.iter().map(|c| Group { name: c.name.clone(), members: members_of(&c.code) }).collect();
    let mut rest = user.clone();
    rest.0.entries.retain(|(k, _)| k.as_str() != GROUPS_KEY);
    if !same_text(&Value::Dict(with_key(&rest, &groups).0), &Value::Dict(user.0.clone())) {
        return Vec::new();
    }
    classes.truncate(start);
    *user = rest;
    groups
}

/// The font user data as written: with the key naming the groups when the font has any.
pub(crate) fn user_data(font: &Font) -> Cow<'_, UserData> {
    if font.groups.is_empty() { Cow::Borrowed(&font.info.user_data) } else { Cow::Owned(with_key(&font.info.user_data, &font.groups)) }
}

/// The classes as written: the feature classes, then a class for each group.
pub(crate) fn classes(font: &Font) -> Cow<'_, [Snippet]> {
    if font.groups.is_empty() {
        Cow::Borrowed(&font.features.classes)
    } else {
        Cow::Owned(font.features.classes.iter().cloned().chain(font.groups.iter().map(as_class)).collect())
    }
}
