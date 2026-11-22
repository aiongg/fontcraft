//! The TrueType Instructor's context menu over a TCP connection to the control channel: each item gives the font
//! what its command gives.

use std::io::{BufRead, BufReader, Write};

use serde_json::{Value, json};

use crate::app::{Msg, ViewMsg};
use crate::testing::TestApp;

/// Sends `requests` over one TCP connection to `t`'s control listener, a line each, and runs frames until every
/// reply is in. The replies, in order.
pub(crate) fn wire(t: &mut TestApp, requests: Vec<Value>) -> Vec<Value> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let ctx = t.frames.ctx.clone();
    let addr = t.app.control.listen(listener, &ctx).unwrap();
    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut replies = Vec::new();
        for (id, mut request) in requests.into_iter().enumerate() {
            request["id"] = json!(id);
            writeln!(writer, "{request}").unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            replies.push(serde_json::from_str::<Value>(&reply).unwrap());
        }
        replies
    });
    for _ in 0..5_000 {
        if client.is_finished() {
            break;
        }
        t.frame();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    client.join().unwrap()
}

/// A request line.
pub(crate) fn request(method: &str, params: Value) -> Value {
    json!({"method": method, "params": params})
}

pub(crate) fn execute(command: &str, params: Value) -> Value {
    request("engine.execute", json!({"command": command, "params": params}))
}

/// Glyph A, shown, with three squares (paths 0, 1, 2).
pub(crate) fn squares() -> TestApp {
    let mut t = TestApp::new();
    for x in [100, 250, 400] {
        t.ok("engine.execute", json!({"command": "path.addRectangle", "params": {"glyph": "A", "x": x, "y": 100, "width": 100, "height": 100}}));
    }
    t.app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
    t.frame();
    t
}

/// Over the wire, with the Instructor active: points selected, the menu opened on empty canvas, an item clicked; the
/// instruction list after each item equals the one its command, run with explicit targets, leaves.
#[test]
fn instructor_menu_items_over_the_wire_equal_their_commands() {
    let list = || execute("hint.tt.listInstructions", json!({"glyph": "A"}));
    // Empty canvas far from the squares and their badges, in font units.
    let open = || request("ui.contextMenu.list", json!({"x": 900, "y": 700}));
    let pick = |id: &str| request("ui.contextMenu.invoke", json!({ "command": id }));
    let select = |points: Value| execute("selection.set", json!({ "points": points }));
    let master = {
        let t = squares();
        t.app.session.font().unwrap().masters[0].id.clone()
    };

    let mut app = squares();
    let replies = wire(
        &mut app,
        vec![
            execute("tool.trueTypeInstructor", json!({})),
            select(json!([[0, 0]])),
            open(),
            pick("hint.tt.snapPoint"),
            list(),
            select(json!([[0, 0]])),
            open(),
            select(json!([[0, 0], [0, 1]])),
            open(),
            pick("hint.tt.addStemHint"),
            list(),
            select(json!([[0, 2]])),
            open(),
            pick("hint.tt.addDelta"),
            list(),
            open(),
            pick("hint.tt.autohintGlyph"),
            list(),
            open(),
            pick("hint.tt.removeHints"),
            list(),
        ],
    );
    for (i, reply) in replies.iter().enumerate() {
        assert_eq!(reply["ok"], true, "request {i}: {reply}");
    }
    // The menu on the snapped point greys Snap out with the command's reason.
    let items: Vec<&Value> = replies[6]["result"]["sections"].as_array().unwrap().iter().flat_map(|s| s["items"].as_array().unwrap()).collect();
    let snap = items.iter().find(|i| i["command"] == "hint.tt.snapPoint").unwrap();
    assert_eq!(snap["enabled"], false);
    assert!(snap["disabled_reason"].as_str().is_some_and(|r| r.contains("touched already")), "{snap}");

    let mut reference = squares();
    let mut lists = Vec::new();
    for (command, params) in [
        ("hint.tt.snapPoint", json!({"glyph": "A", "nodes": [[0, 0]], "direction": "vertical"})),
        ("hint.tt.addStemHint", json!({"glyph": "A", "nodes": [[0, 0], [0, 1]], "direction": "vertical"})),
        ("hint.tt.addDelta", json!({"glyph": "A", "nodes": [[0, 2]], "direction": "vertical"})),
        ("hint.tt.autohintGlyph", json!({"glyphs": ["A"], "layer": master})),
        ("hint.tt.removeHints", json!({"glyphs": ["A"]})),
    ] {
        reference.ok("engine.execute", json!({"command": command, "params": params}));
        lists.push(reference.ok("engine.execute", json!({"command": "hint.tt.listInstructions", "params": {"glyph": "A"}})));
    }
    let wired: Vec<&Value> = [4, 10, 14, 17, 20].iter().map(|&i| &replies[i]["result"]).collect();
    for (step, (got, want)) in wired.iter().zip(&lists).enumerate() {
        assert_eq!(*got, want, "step {step}");
    }
}
