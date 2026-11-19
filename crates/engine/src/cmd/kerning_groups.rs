//! Kerning shortcuts and kerning groups: the eight Text tool shortcuts that change the pair the
//! locks name, the four group setters and the group list. Behaviour notes:
//! `docs/spec/08-spacing-kerning.md` § 3.4 to 3.6.

use std::sync::Arc;

use fontcraft_model::{Font, KernGroups};
use fontcraft_spacing::kerning::{GroupSide, Lock, Position, check_group_name, group_list, locks, shortcut_edit, stored_pair};
use serde_json::{Value, json};

use super::shared::{MAX_KERN_VALUE, direction, editable_master};
use super::*;
use crate::params::{Params, TargetKind};
use crate::prelude::*;

/// Largest step one shortcut call may take, in units.
const MAX_SHORTCUT_STEP: i64 = 1_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "kerning.increaseLeft",
            "Increase Kerning on Left Side",
            [],
            Some("Ctrl+Alt+Left"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr), amount?: whole number 1 to 1000 (default: 1)} add `amount` to the kerning that applies to the pair `left` `right` (logical order) and store the result on the sides the locks choose: a closed side is the glyph's group, an open or ungrouped side the glyph; the pair is created when missing. In text the key acts on the glyph before the cursor and the glyph after it. A master that takes its kerning from another master is refused → {left, right, master, direction, pair: {left, right} (command spelling), value, created: bool, locks: {left, right}: \"closed\"|\"open\"|\"ungrouped\"}",
            has_doc,
            increase_left
        )
        .when("textMode"),
        cmd!(
            "kerning.increaseLeftBy10",
            "Increase Kerning on Left Side by 10",
            [],
            Some("Ctrl+Alt+Shift+Left"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr)} as kerning.increaseLeft with a step of 10 → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            increase_left_10
        )
        .when("textMode"),
        cmd!(
            "kerning.decreaseLeft",
            "Decrease Kerning on Left Side",
            [],
            Some("Ctrl+Alt+Right"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr), amount?: whole number 1 to 1000 (default: 1)} subtract `amount` from the kerning that applies to the pair `left` `right` and store the result on the sides the locks choose, as kerning.increaseLeft does → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            decrease_left
        )
        .when("textMode"),
        cmd!(
            "kerning.decreaseLeftBy10",
            "Decrease Kerning on Left Side by 10",
            [],
            Some("Ctrl+Alt+Shift+Right"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr)} as kerning.decreaseLeft with a step of 10 → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            decrease_left_10
        )
        .when("textMode"),
        cmd!(
            "kerning.increaseRight",
            "Increase Kerning on Right Side",
            [],
            Some("Cmd+Alt+Right"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr), amount?: whole number 1 to 1000 (default: 1)} add `amount` to the kerning that applies to the pair `left` `right`, as kerning.increaseLeft does. In text the key acts on the glyph after the cursor and the one after that → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            increase_right
        )
        .when("textMode"),
        cmd!(
            "kerning.increaseRightBy10",
            "Increase Kerning on Right Side by 10",
            [],
            Some("Cmd+Alt+Shift+Right"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr)} as kerning.increaseRight with a step of 10 → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            increase_right_10
        )
        .when("textMode"),
        cmd!(
            "kerning.decreaseRight",
            "Decrease Kerning on Right Side",
            [],
            Some("Cmd+Alt+Left"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr), amount?: whole number 1 to 1000 (default: 1)} subtract `amount` from the kerning that applies to the pair `left` `right`, as kerning.decreaseLeft does → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            decrease_right
        )
        .when("textMode"),
        cmd!(
            "kerning.decreaseRightBy10",
            "Decrease Kerning on Right Side by 10",
            [],
            Some("Cmd+Alt+Shift+Left"),
            "{left: glyph name, right: glyph name, master?: id (default: the first master), direction?: \"ltr\"|\"rtl\"|\"vertical\" (default: ltr)} as kerning.decreaseRight with a step of 10 → {left, right, master, direction, pair: {left, right}, value, created, locks}",
            has_doc,
            decrease_right_10
        )
        .when("textMode"),
        cmd!(
            "kerning.setLeftGroup",
            "Set Left Kerning Group",
            [],
            None,
            "{glyphs: [names], group: string|null} set the left kerning group of each glyph (used when the glyph is second in a left-to-right pair); null or \"\" clears it. A name may hold only A–Z, a–z, 0–9, `_`, `.` and `-`, without `@`. All masters and directions share the group → {glyphs: [names], side: \"left\", group: string|null}",
            has_doc,
            set_left
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "kerning.setRightGroup",
            "Set Right Kerning Group",
            [],
            None,
            "{glyphs: [names], group: string|null} set the right kerning group of each glyph (used when the glyph is first in a left-to-right pair); null or \"\" clears it. Names as for kerning.setLeftGroup → {glyphs: [names], side: \"right\", group: string|null}",
            has_doc,
            set_right
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "kerning.setTopGroup",
            "Set Top Kerning Group",
            [],
            None,
            "{glyphs: [names], group: string|null} set the top kerning group of each glyph (used when the glyph is second in a vertical pair); null or \"\" clears it. Names as for kerning.setLeftGroup → {glyphs: [names], side: \"top\", group: string|null}",
            has_doc,
            set_top
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            "kerning.setBottomGroup",
            "Set Bottom Kerning Group",
            [],
            None,
            "{glyphs: [names], group: string|null} set the bottom kerning group of each glyph (used when the glyph is first in a vertical pair); null or \"\" clears it. Names as for kerning.setLeftGroup → {glyphs: [names], side: \"bottom\", group: string|null}",
            has_doc,
            set_bottom
        )
        .targets(&[TargetKind::Glyphs]),
        cmd!(
            query "kerning.listGroups",
            "List Kerning Groups",
            [],
            None,
            "{side?: \"left\"|\"right\"|\"top\"|\"bottom\" (default: every side), group?: string (default: every group)} the kerning groups of each side with their members: sides in the order left, right, top, bottom; groups in the order of their first member; members in glyph order → {groups: [{side, name, members: [names]}]}",
            has_doc,
            list_groups
        ),
    ]
}

// ── Shortcuts ───────────────────────────────────────────────────────────────────────────────────

fn increase_left(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.increaseLeft", "Increase Kerning on Left Side", 1.0, Some(1))
}
fn increase_left_10(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.increaseLeftBy10", "Increase Kerning on Left Side by 10", 1.0, None)
}
fn decrease_left(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.decreaseLeft", "Decrease Kerning on Left Side", -1.0, Some(1))
}
fn decrease_left_10(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.decreaseLeftBy10", "Decrease Kerning on Left Side by 10", -1.0, None)
}
fn increase_right(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.increaseRight", "Increase Kerning on Right Side", 1.0, Some(1))
}
fn increase_right_10(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.increaseRightBy10", "Increase Kerning on Right Side by 10", 1.0, None)
}
fn decrease_right(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.decreaseRight", "Decrease Kerning on Right Side", -1.0, Some(1))
}
fn decrease_right_10(s: &mut Session, p: &Value) -> Result<Value> {
    shortcut(s, p, "kerning.decreaseRightBy10", "Decrease Kerning on Right Side by 10", -1.0, None)
}

fn lock_name(lock: Lock) -> &'static str {
    match lock {
        Lock::Closed => "closed",
        Lock::Open => "open",
        Lock::Ungrouped => "ungrouped",
    }
}

/// A glyph side of the pair: a name the font has.
fn pair_glyph<'a>(p: &Params<'a>, key: &str, font: &Font) -> Result<&'a str> {
    let name = p.str(key)?;
    if font.glyph(name).is_none() {
        return Err(p.bad(format!("`{key}`: there is no glyph `{name}`")));
    }
    Ok(name)
}

/// One kerning shortcut: `sign` gives increase (+1) or decrease (−1); `step` is the default of
/// `amount`, or `None` for the 10-unit commands, which take no `amount`.
fn shortcut(s: &mut Session, p: &Value, id: &'static str, label: &str, sign: f64, step: Option<i64>) -> Result<Value> {
    let p = Params::new(id, p)?;
    let amount = match step {
        Some(default) => p.opt_int("amount", 1, MAX_SHORTCUT_STEP)?.unwrap_or(default),
        None => 10,
    };
    let direction = direction(&p)?;
    let index = s.kern_index()?;
    let font = s.font()?;
    let master = editable_master(&p, font)?;
    let left = pair_glyph(&p, "left", font)?.to_string();
    let right = pair_glyph(&p, "right", font)?.to_string();
    let locks = locks(&index, direction, &master, &left, &right).map_err(|e| p.bad(e.to_string()))?;
    let edit = shortcut_edit(&index, direction, &master, &left, &right, sign * amount as f64).map_err(|e| p.bad(e.to_string()))?;
    drop(index);
    if edit.value.abs() > MAX_KERN_VALUE {
        return Err(p.bad(format!("the kerning value would be {}, outside −{MAX_KERN_VALUE} to {MAX_KERN_VALUE}", edit.value)));
    }
    let created = s.edit(label, |font| {
        let existing = stored_pair(font, direction, &master, &edit.first, &edit.second).map(|sp| (sp.first.to_string(), sp.second.to_string()));
        let created = existing.is_none();
        let (first, second) =
            existing.unwrap_or_else(|| (edit.first.stored(direction, Position::First), edit.second.stored(direction, Position::Second)));
        direction.table_mut(Arc::make_mut(&mut font.kerning)).set(&master, &first, &second, edit.value);
        Ok(created)
    })?;
    Ok(json!({
        "left": left,
        "right": right,
        "master": master,
        "direction": direction.name(),
        "pair": {"left": edit.first.command(), "right": edit.second.command()},
        "value": edit.value,
        "created": created,
        "locks": {"left": lock_name(locks.first), "right": lock_name(locks.second)},
    }))
}

// ── Groups ──────────────────────────────────────────────────────────────────────────────────────

fn set_left(s: &mut Session, p: &Value) -> Result<Value> {
    set_group(s, p, "kerning.setLeftGroup", "Set Left Kerning Group", GroupSide::Left)
}
fn set_right(s: &mut Session, p: &Value) -> Result<Value> {
    set_group(s, p, "kerning.setRightGroup", "Set Right Kerning Group", GroupSide::Right)
}
fn set_top(s: &mut Session, p: &Value) -> Result<Value> {
    set_group(s, p, "kerning.setTopGroup", "Set Top Kerning Group", GroupSide::Top)
}
fn set_bottom(s: &mut Session, p: &Value) -> Result<Value> {
    set_group(s, p, "kerning.setBottomGroup", "Set Bottom Kerning Group", GroupSide::Bottom)
}

fn slot(groups: &mut KernGroups, side: GroupSide) -> &mut Option<String> {
    match side {
        GroupSide::Left => &mut groups.left,
        GroupSide::Right => &mut groups.right,
        GroupSide::Top => &mut groups.top,
        GroupSide::Bottom => &mut groups.bottom,
    }
}

fn set_group(s: &mut Session, raw: &Value, id: &'static str, label: &str, side: GroupSide) -> Result<Value> {
    let p = Params::new(id, raw)?;
    // `group` must be given; null (unlike a missing key) clears.
    let group = match raw.get("group") {
        None => return Err(p.bad("`group` is required (null clears the group)")),
        Some(Value::Null) => None,
        Some(Value::String(g)) if g.is_empty() => None,
        Some(Value::String(g)) => {
            check_group_name(g).map_err(|e| p.bad(format!("`group`: {e}")))?;
            Some(g.clone())
        }
        Some(other) => return Err(p.bad(format!("`group` must be a string or null, got {}", crate::params::kind(other)))),
    };
    let names = p.opt_glyphs("glyphs", s.font()?)?.ok_or_else(|| p.bad("`glyphs` is required"))?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    s.edit(label, |font| {
        for name in &names {
            let mut g = font.glyph_mut(name)?;
            if side.of(&g.kern_groups) != group.as_deref() {
                *slot(&mut g.kern_groups, side) = group.clone();
            }
        }
        Ok(())
    })?;
    Ok(json!({ "glyphs": names, "side": side.name(), "group": group }))
}

fn list_groups(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("kerning.listGroups", p)?;
    let sides: Vec<GroupSide> = match p.opt_str("side")? {
        None => GroupSide::ALL.to_vec(),
        Some(name) => vec![
            GroupSide::from_name(name).ok_or_else(|| p.bad(format!("`side` must be \"left\", \"right\", \"top\" or \"bottom\", got \"{name}\"")))?,
        ],
    };
    let only = p.opt_str("group")?;
    let font = s.font()?;
    let mut groups = Vec::new();
    for side in sides {
        for (name, members) in group_list(font, side) {
            if only.is_none_or(|g| g == name) {
                groups.push(json!({ "side": side.name(), "name": name, "members": members }));
            }
        }
    }
    Ok(json!({ "groups": groups }))
}
