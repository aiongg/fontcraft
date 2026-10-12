//! Kerning shortcuts and kerning groups: the eight shortcuts, the group setters and the group list.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_model::Font;
use fontcraft_model::plist::Value as Plist;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused, with a message holding `needle`, and leave the font as it
/// was: the very same pointer.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn ltr(font: &Font, first: &str, second: &str) -> Option<f64> {
    font.kerning.ltr.get("m01", first, second)
}

/// Groups: T right group `T`; V right `V`; A right `A`; o and e left `o`; x none.
/// Pairs: @T–@o −50, @T–e −20 (exception for e), V–@o −30, A–o −40.
fn kerned() -> Session {
    let mut s = Session::with_document();
    for (id, glyphs, group) in [
        ("kerning.setRightGroup", json!(["T"]), "T"),
        ("kerning.setRightGroup", json!(["V"]), "V"),
        ("kerning.setRightGroup", json!(["A"]), "A"),
        ("kerning.setLeftGroup", json!(["o", "e"]), "o"),
    ] {
        run(&mut s, id, json!({"glyphs": glyphs, "group": group}));
    }
    s.edit("pairs", |font| {
        let t = &mut Arc::make_mut(&mut font.kerning).ltr;
        t.set("m01", "@MMK_L_T", "@MMK_R_o", -50.0);
        t.set("m01", "@MMK_L_T", "e", -20.0);
        t.set("m01", "V", "@MMK_R_o", -30.0);
        t.set("m01", "A", "o", -40.0);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn kerning_shortcuts_write_the_pair_each_lock_combination_names() {
    let mut s = kerned();
    // closed–closed: T o is kerned by @T–@o, so the group pair changes.
    let r = run(&mut s, "kerning.increaseLeft", json!({"left": "T", "right": "o"}));
    assert_eq!(r["pair"], json!({"left": "@T", "right": "@o"}));
    assert_eq!(r["locks"], json!({"left": "closed", "right": "closed"}));
    assert_eq!((r["value"].as_f64(), r["created"].as_bool()), (Some(-49.0), Some(false)));
    assert_eq!(ltr(s.font().unwrap(), "@MMK_L_T", "@MMK_R_o"), Some(-49.0));
    // closed–open: T e is the exception @T–e.
    let r = run(&mut s, "kerning.decreaseRight", json!({"left": "T", "right": "e"}));
    assert_eq!(r["pair"], json!({"left": "@T", "right": "e"}));
    assert_eq!(r["locks"], json!({"left": "closed", "right": "open"}));
    assert_eq!(ltr(s.font().unwrap(), "@MMK_L_T", "e"), Some(-21.0));
    // open–closed: V o is V–@o.
    let r = run(&mut s, "kerning.increaseRightBy10", json!({"left": "V", "right": "o"}));
    assert_eq!(r["pair"], json!({"left": "V", "right": "@o"}));
    assert_eq!(r["locks"], json!({"left": "open", "right": "closed"}));
    assert_eq!(ltr(s.font().unwrap(), "V", "@MMK_R_o"), Some(-20.0));
    // open–open: A o is A–o.
    let r = run(&mut s, "kerning.decreaseLeftBy10", json!({"left": "A", "right": "o"}));
    assert_eq!(r["pair"], json!({"left": "A", "right": "o"}));
    assert_eq!(r["locks"], json!({"left": "open", "right": "open"}));
    assert_eq!(ltr(s.font().unwrap(), "A", "o"), Some(-50.0));
    // An ungrouped glyph is always itself: x has no right group, so x e creates x–@o.
    let r = run(&mut s, "kerning.increaseLeft", json!({"left": "x", "right": "e"}));
    assert_eq!(r["pair"], json!({"left": "x", "right": "@o"}));
    assert_eq!(r["locks"], json!({"left": "ungrouped", "right": "closed"}));
    assert_eq!(ltr(s.font().unwrap(), "x", "@MMK_R_o"), Some(1.0));
    // The other members of the groups kept their kerning.
    assert_eq!(ltr(s.font().unwrap(), "@MMK_L_T", "@MMK_R_o"), Some(-49.0));
}

#[test]
fn each_kerning_shortcut_moves_the_value_by_its_step_and_undo_restores_the_font() {
    let mut s = kerned();
    let start = Arc::clone(&s.doc().unwrap().font);
    let pair = json!({"left": "T", "right": "o"});
    let mut value = -50.0;
    for (id, step) in [
        ("kerning.increaseLeft", 1.0),
        ("kerning.increaseLeftBy10", 10.0),
        ("kerning.decreaseLeft", -1.0),
        ("kerning.decreaseLeftBy10", -10.0),
        ("kerning.increaseRight", 1.0),
        ("kerning.increaseRightBy10", 10.0),
        ("kerning.decreaseRight", -1.0),
        ("kerning.decreaseRightBy10", -10.0),
    ] {
        value += step;
        let r = run(&mut s, id, pair.clone());
        assert_eq!(r["value"].as_f64(), Some(value), "{id}");
        assert_eq!(ltr(s.font().unwrap(), "@MMK_L_T", "@MMK_R_o"), Some(value), "{id}");
    }
    let r = run(&mut s, "kerning.decreaseRight", json!({"left": "T", "right": "o", "amount": 25}));
    assert_eq!(r["value"].as_f64(), Some(-75.0));
    for _ in 0..9 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(*s.doc().unwrap().font, *start);
    // A fractional value from a file is kept and moved by the step.
    s.edit("fraction", |font| {
        Arc::make_mut(&mut font.kerning).ltr.set("m01", "A", "o", -40.5);
        Ok(())
    })
    .unwrap();
    assert_eq!(run(&mut s, "kerning.increaseLeft", json!({"left": "A", "right": "o"}))["value"].as_f64(), Some(-39.5));
}

#[test]
fn kerning_shortcuts_need_text_tool_create_the_missing_pair_and_refuse_bad_targets() {
    let mut s = Session::with_document();
    // No pair and no groups: the first keypress creates the glyph pair at the step.
    let r = run(&mut s, "kerning.decreaseLeft", json!({"left": "L", "right": "T"}));
    assert_eq!((r["created"].as_bool(), r["value"].as_f64()), (Some(true), Some(-1.0)));
    assert_eq!(ltr(s.font().unwrap(), "L", "T"), Some(-1.0));
    // Pressed again, the same pair changes.
    let r = run(&mut s, "kerning.decreaseLeft", json!({"left": "L", "right": "T"}));
    assert_eq!((r["created"].as_bool(), r["value"].as_f64()), (Some(false), Some(-2.0)));

    // Right-to-left pairs are in logical order and use the facing groups.
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["a"], "group": "a"}));
    run(&mut s, "kerning.setRightGroup", json!({"glyphs": ["b"], "group": "b"}));
    let r = run(&mut s, "kerning.increaseLeft", json!({"left": "a", "right": "b", "direction": "rtl"}));
    assert_eq!(r["pair"], json!({"left": "@a", "right": "@b"}));
    assert_eq!(s.font().unwrap().kerning.rtl.get("m01", "@MMK_R_a", "@MMK_L_b"), Some(1.0));
    assert_eq!(ltr(s.font().unwrap(), "@MMK_L_b", "@MMK_R_a"), None);

    let id = "kerning.increaseLeft";
    refused(&mut s, id, json!({"left": "L"}), "`right` is required");
    refused(&mut s, id, json!({"left": "L", "right": "nosuch"}), "no glyph `nosuch`");
    refused(&mut s, id, json!({"left": "L", "right": "T", "master": "m09"}), "no master `m09`");
    refused(&mut s, id, json!({"left": "L", "right": "T", "direction": "up"}), "`direction`");
    refused(&mut s, id, json!({"left": "L", "right": "T", "amount": 0}), "`amount`");
    refused(&mut s, id, json!({"left": "L", "right": "T", "amount": 1.5}), "`amount`");
    s.edit("big", |font| {
        Arc::make_mut(&mut font.kerning).ltr.set("m01", "L", "T", 32_767.0);
        Ok(())
    })
    .unwrap();
    refused(&mut s, id, json!({"left": "L", "right": "T"}), "outside");

    // A master that follows another master's kerning is refused.
    s.edit("linked", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        m.params.set("Link Metrics With First Master", Plist::Bool(true));
        Arc::make_mut(&mut font.masters).push(m);
        Ok(())
    })
    .unwrap();
    refused(&mut s, id, json!({"left": "L", "right": "T", "master": "m02"}), "takes its metrics and kerning from master `m01`");
}

#[test]
fn kerning_groups_edited_for_many_glyphs_in_one_step() {
    let mut s = Session::with_document();
    let start = Arc::clone(&s.doc().unwrap().font);
    let r = run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["O", "Q", "C"], "group": "O"}));
    assert_eq!(r, json!({"glyphs": ["O", "Q", "C"], "side": "left", "group": "O"}));
    run(&mut s, "kerning.setRightGroup", json!({"glyphs": ["O", "Q"], "group": "O"}));
    run(&mut s, "kerning.setTopGroup", json!({"glyphs": ["a"], "group": "top.a"}));
    run(&mut s, "kerning.setBottomGroup", json!({"glyphs": ["a"], "group": "bottom-a"}));
    let font = s.font().unwrap();
    for name in ["O", "Q", "C"] {
        assert_eq!(font.glyph(name).unwrap().kern_groups.left.as_deref(), Some("O"), "{name}");
    }
    let a = &font.glyph("a").unwrap().kern_groups;
    assert_eq!((a.top.as_deref(), a.bottom.as_deref()), (Some("top.a"), Some("bottom-a")));

    let all = run(&mut s, "kerning.listGroups", json!({}));
    assert_eq!(
        all["groups"],
        json!([
            {"side": "left", "name": "O", "members": ["C", "O", "Q"]},
            {"side": "right", "name": "O", "members": ["O", "Q"]},
            {"side": "top", "name": "top.a", "members": ["a"]},
            {"side": "bottom", "name": "bottom-a", "members": ["a"]},
        ])
    );
    let right = run(&mut s, "kerning.listGroups", json!({"side": "right", "group": "O"}));
    assert_eq!(right["groups"], json!([{"side": "right", "name": "O", "members": ["O", "Q"]}]));

    // null and "" clear.
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["C"], "group": null}));
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["Q"], "group": ""}));
    let left = run(&mut s, "kerning.listGroups", json!({"side": "left"}));
    assert_eq!(left["groups"], json!([{"side": "left", "name": "O", "members": ["O"]}]));

    // A bad name names the character; the font does not change.
    let id = "kerning.setRightGroup";
    refused(&mut s, id, json!({"glyphs": ["O"], "group": "O/slash"}), "not `/` (character 2)");
    refused(&mut s, id, json!({"glyphs": ["O"], "group": "@O"}), "not `@` (character 1)");
    refused(&mut s, id, json!({"glyphs": ["O"], "group": "Ö"}), "not `Ö` (character 1)");
    refused(&mut s, id, json!({"glyphs": ["O"]}), "`group` is required");
    refused(&mut s, id, json!({"glyphs": [], "group": "O"}), "`glyphs` is empty");
    refused(&mut s, id, json!({"glyphs": ["nosuch"], "group": "O"}), "no glyph `nosuch`");
    refused(&mut s, id, json!({"glyphs": ["O"], "group": 3}), "string or null");
    refused(&mut s, "kerning.listGroups", json!({"side": "middle"}), "`side`");

    // Each setter call is one undo step.
    for _ in 0..6 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(*s.doc().unwrap().font, *start);
}
