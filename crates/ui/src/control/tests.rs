use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

use craft::host::Host;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::handle;
use crate::app::{APP, FontCraftApp, Msg};
use crate::testing::TestApp;

/// Engine commands, the documents, the menus, refused requests and the methods of later packages, as an agent
/// sees them.
#[test]
fn the_control_protocol_drives_the_app() {
    let mut t = TestApp::new();
    let made = t.ok("engine.execute", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
    assert_eq!(made["name"], "agent");
    assert_eq!(t.ok("document.inspect", json!({}))["documents"][0]["undo"], json!(["New Glyph"]));
    let commands = t.ok("engine.commands", json!({}));
    let shell: usize = crate::shell::commands::MODULES.iter().map(|m| m.commands.len()).sum();
    // `tool.select` is a shell command and the Select tool's command: listed once.
    let tools = fontcraft_tools::tool::TOOLS.iter().filter(|t| crate::shell::commands::find(&format!("tool.{}", t.id)).is_none()).count();
    assert_eq!(tools + 1, fontcraft_tools::tool::TOOLS.len());
    let expected = fontcraft_engine::command_specs().len() + shell + tools;
    assert_eq!(commands["commands"].as_array().map(Vec::len), Some(expected));

    let menus = t.ok("ui.menu.list", json!({}));
    let menus = menus.as_array().cloned().unwrap_or_default();
    assert!(menus.len() >= 4, "{menus:?}");
    assert_eq!(menus[0]["title"], "File");
    let items: Vec<&Value> = menus.iter().flat_map(|m| m["items"].as_array().into_iter().flatten()).collect();
    assert!(items.iter().any(|i| i["command"] == "edit.undo"), "Undo is in a menu");
    assert!(items.iter().all(|i| i["enabled"] == true || i["disabled_reason"].is_string()), "a greyed item says why");

    t.ok("ui.menu.invoke", json!({"command": "edit.undo"}));
    assert!(t.ok("document.inspect", json!({}))["documents"][0]["undo"].as_array().is_some_and(Vec::is_empty), "the menu undid New Glyph");
    assert!(t.error("ui.menu.invoke", json!({"command": "glyph.newGlyph.nope"})).contains("not a menu item"));

    // A tool's command makes the tool current.
    t.ok("engine.execute", json!({"command": "tool.draw"}));
    assert_eq!(t.ok("ui.inspect", json!({}))["tool"], "draw");

    for bad in [
        json!({"method": "nope"}),
        json!({"id": 2}),
        json!({"method": 5}),
        json!({"method": "engine.execute"}),
        json!({"method": "engine.execute", "params": {"command": "no.such"}}),
        json!({"method": "ui.render", "params": [1]}),
        json!([]),
        json!("x"),
    ] {
        let reply = handle(&mut t.app, &bad);
        assert_eq!(reply["ok"], false, "{bad}");
        assert!(reply["error"].as_str().is_some_and(|e| !e.is_empty()), "{bad}: {reply}");
    }
    assert_eq!(handle(&mut t.app, &json!({"id": 9, "method": "nope"}))["id"], 9, "the reply carries the request's id");

    // Methods of packages not built yet say which package brings them.
    for &(method, package) in super::LATER {
        let error = t.error(method, json!({}));
        assert!(error.contains("not built yet") && error.contains(package), "{method}: {error}");
    }
    assert!(t.error("ui.click", json!({"id": "ok"})).contains("no dialog is open"), "a dialog control press needs an open dialog");

    // Every method of the protocol the skeleton answers is in the table.
    let methods = t.ok("control.methods", json!({}));
    for method in [
        "engine.execute",
        "engine.commands",
        "document.inspect",
        "ui.inspect",
        "ui.set",
        "ui.render",
        "ui.resize",
        "ui.focus",
        "ui.click",
        "ui.drag",
        "ui.move",
        "ui.wheel",
        "ui.menu.list",
        "ui.menu.invoke",
    ] {
        assert!(methods["methods"].as_array().is_some_and(|m| m.contains(&json!(method))), "{method}");
    }
}

/// `ui.menu.invoke` plays a click: a filter with a dialog opens it on the shown glyph instead of running, and Close
/// with unsaved work asks about it instead of failing.
#[test]
fn a_menu_invoke_is_a_click() {
    let mut t = TestApp::new();
    t.ok("engine.execute", json!({"command": "path.addRectangle", "params": {"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}}));
    t.app.sent.push(Msg::View(crate::app::ViewMsg::SelectGlyph("A".into())));
    t.frame();
    let undo = t.ok("document.inspect", json!({}))["documents"][0]["undo"].clone();
    assert_eq!(t.ok("ui.menu.invoke", json!({"command": "filter.roundCorners"})), json!({"dialog": "dialog.filterRoundCorners"}));
    assert_eq!(t.ok("ui.dialog.inspect", json!({}))["dialog"], "dialog.filterRoundCorners");
    assert_eq!(t.ok("document.inspect", json!({}))["documents"][0]["undo"], undo, "the filter did not run");
    t.ok("ui.dialog.cancel", json!({}));

    assert!(t.ok("ui.menu.invoke", json!({"command": "file.close"})).is_null());
    assert!(t.app.closing.is_open(), "Close asks about the unsaved square");
    assert_eq!(t.app.session.documents().len(), 1, "the document stays open");
}

/// `ui.render` draws filled glyphs without a window, the same bytes each time; a headless app cannot screenshot.
#[test]
fn ui_render_draws_glyph_pixels_and_a_headless_app_has_no_screenshot() {
    let mut t = TestApp::new();
    t.ok(
        "engine.execute",
        json!({"command": "path.addPath", "params": {"glyph": "A", "nodes": [{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 310, "y": 700}]}}),
    );
    let first = t.ok("ui.render", json!({"glyphs": ["A"], "size": 100}));
    assert_eq!((first["width"].as_u64(), first["height"].as_u64()), (Some(76), Some(116)), "{first}");
    assert_eq!(t.ok("ui.render", json!({"glyphs": ["A"], "size": 100}))["png"], first["png"], "the same bytes twice");
    assert_eq!(t.ok("ui.render", json!({"text": "AB", "scale": 0.1}))["glyphs"], json!(["A", "B"]));
    t.app.sent.push(Msg::View(crate::app::ViewMsg::SelectGlyph("A".into())));
    t.frame();
    assert_eq!(t.ok("ui.render", json!({}))["glyphs"], json!(["A"]), "nothing named: the shown glyph");
    for bad in [
        json!({"glyphs": ["nope"]}),
        json!({"text": "\u{1F600}"}),
        json!({"size": -1}),
        json!({"scale": 1e9}),
        json!({"glyphs": 5}),
        json!({"path": 5}),
        json!({"master": "nope"}),
        json!({"glyphs": ["A", "B"], "layer": "x"}),
    ] {
        t.error("ui.render", bad);
    }
    assert!(t.error("ui.screenshot", json!({})).starts_with("ui.screenshot:"));
}

/// What the user runs may export outside the destination; a control request runs with that off and leaves the
/// app's setting as it was.
#[test]
fn control_requests_export_inside_the_destination_and_keep_the_setting() {
    let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    assert!(app.session.export_outside());
    let _ = handle(&mut app, &json!({"id": 1, "method": "engine.execute", "params": {"command": "glyph.getInfo", "params": {}}}));
    assert!(app.session.export_outside());
}

/// Requests read and write only inside the automation roots; the person at the keyboard is not confined.
#[test]
fn control_requests_honour_the_automation_roots() {
    let base = std::env::temp_dir().join(format!("fontcraft-ui-control-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let at = |p: &str| base.join(p).to_str().unwrap().to_string();
    let mut t = TestApp::new();
    t.app.control.roots = fontcraft_engine::file_access::AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap();
    let save = |path: String| json!({"command": "file.saveAs", "params": {"path": path}});

    assert!(t.error("engine.execute", save(at("outside/x.fontcraft"))).contains("outside the write root"));
    assert!(!base.join("outside/x.fontcraft").exists());
    t.ok("engine.execute", save(at("out/x.fontcraft")));
    assert!(base.join("out/x.fontcraft").exists());
    t.ok(
        "engine.execute",
        json!({"command": "path.addPath", "params": {"glyph": "A", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 50, "y": 100}]}}),
    );
    assert!(t.error("ui.render", json!({"path": at("outside/r.png"), "glyphs": ["A"]})).contains("outside the write root"));
    t.ok("ui.render", json!({"path": at("out/r.png"), "glyphs": ["A"]}));
    assert!(base.join("out/r.png").exists());
    assert!(t.error("ui.screenshot", json!({"path": at("in/s.png")})).contains("outside the write root"));
    // The frame is written a frame later, outside the roots: to the path that was checked.
    let checked = std::fs::canonicalize(base.join("out")).unwrap().join("s.png");
    let roots = t.app.control.roots.clone();
    let path = fontcraft_engine::file_access::confine(roots.as_ref(), || super::screenshot::path(&json!({"path": at("in/../out/s.png")})));
    assert_eq!(path.unwrap().as_deref(), checked.to_str());
    assert!(t.error("engine.execute", json!({"command": "script.run", "params": {"macro": "m"}})).contains("automation roots"));
    // The person at the keyboard is not confined.
    t.app.run_now(Msg::run("file.saveAs", json!({"path": at("outside/y.fontcraft")})), false).unwrap();
    assert!(base.join("outside/y.fontcraft").exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// `ui.set` shows and hides panels with their command; a bad key changes nothing.
#[test]
fn ui_set_turns_panels_on_and_off() {
    let mut t = TestApp::new();
    let shown = |t: &mut TestApp| t.ok("ui.inspect", json!({}))["panels"].clone();
    assert!(shown(&mut t).as_array().is_some_and(|p| p.contains(&json!("layers")) && !p.contains(&json!("textPreview"))));
    assert_eq!(t.ok("ui.set", json!({"layers": false, "textPreview": true})), json!({"layers": false, "textPreview": true}));
    let panels = shown(&mut t);
    assert!(panels.as_array().is_some_and(|p| !p.contains(&json!("layers")) && p.contains(&json!("textPreview"))), "{panels}");
    assert!(t.error("ui.set", json!({"layers": true, "nope": true})).contains("unknown key `nope`"));
    assert!(t.error("ui.set", json!({"layers": "yes"})).contains("true or false"));
    assert_eq!(shown(&mut t), panels, "a refused request moved nothing");
    t.error("ui.set", json!({}));
    t.error("ui.set", json!([1]));
}

/// Clicks, drags, moves and wheel turns reach the frames at window points; a bad request queues nothing.
#[test]
fn window_input_reaches_the_frames() {
    let mut t = TestApp::new();
    assert_eq!(t.ok("ui.move", json!({"x": 300, "y": 200})), json!({"queued": 1}));
    assert_eq!(t.pointer(), Some(egui::pos2(300.0, 200.0)));
    t.ok("ui.drag", json!({"x": 400, "y": 300, "toX": 500, "toY": 350, "steps": 4, "mods": ["shift"]}));
    assert_eq!(t.pointer(), Some(egui::pos2(500.0, 350.0)));
    t.ok("ui.click", json!({"x": 600, "y": 410, "button": "right", "count": 2, "alt": true}));
    assert_eq!(t.pointer(), Some(egui::pos2(600.0, 410.0)));
    t.ok("ui.wheel", json!({"x": 640, "y": 400, "dy": -3}));
    assert_eq!(t.pointer(), Some(egui::pos2(640.0, 400.0)));
    assert_eq!(t.ok("ui.inspect", json!({}))["pendingInput"], false);
    for (method, bad) in [
        ("ui.click", json!({"x": 1e9, "y": 0})),
        ("ui.click", json!({"x": 1, "y": 1, "count": 17})),
        ("ui.click", json!({"x": 1, "y": 1, "button": "up"})),
        ("ui.click", json!({"x": 1, "y": 1, "mods": ["hyper"]})),
        ("ui.click", json!({"x": 1})),
        ("ui.drag", json!({"x": 1, "y": 1, "toX": 2, "toY": 2, "steps": 257})),
        ("ui.drag", json!({"x": 1, "y": 1})),
        ("ui.wheel", json!({"x": 1, "y": 1, "dy": 101})),
        ("ui.wheel", json!({"x": 1, "y": 1, "unit": "page"})),
        ("ui.move", json!({"x": "a", "y": 1})),
        ("ui.resize", json!({"width": 100, "height": 400})),
    ] {
        t.error(method, bad.clone());
        assert_eq!(t.pointer(), Some(egui::pos2(640.0, 400.0)), "{method} {bad} moved the pointer");
    }
    let mut windowless = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
    let reply = handle(&mut windowless, &json!({"method": "ui.click", "params": {"x": 1, "y": 1}}));
    assert!(reply["error"].as_str().is_some_and(|e| e.contains("no window")), "{reply}");
}

/// Over TCP: replies carry the request's id, window input is replied to once the frames carried it, and a line
/// that is no request closes the connection.
#[test]
fn the_listener_answers_between_frames() {
    let mut t = TestApp::new();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let ctx = t.frames.ctx.clone();
    let addr = t.app.control.listen(listener, &ctx).unwrap();
    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut replies = Vec::new();
        for line in [
            r#"{"id": 7, "method": "engine.execute", "params": {"command": "glyph.newGlyph", "params": {"name": "wire"}}}"#,
            r#"{"id": "m", "method": "ui.move", "params": {"x": 111, "y": 222}}"#,
            "[1, 2]",
        ] {
            writeln!(writer, "{line}").unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            replies.push(serde_json::from_str::<Value>(&reply).unwrap());
        }
        let mut rest = String::new();
        let closed = reader.read_line(&mut rest).map(|n| n == 0).unwrap_or(true);
        (replies, closed)
    });
    for _ in 0..2_000 {
        if client.is_finished() {
            break;
        }
        t.frame();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let (replies, closed) = client.join().unwrap();
    assert_eq!((&replies[0]["id"], &replies[0]["ok"], &replies[0]["result"]["name"]), (&json!(7), &json!(true), &json!("wire")), "{}", replies[0]);
    assert_eq!(replies[1], json!({"id": "m", "ok": true, "result": {"queued": 1}}));
    assert_eq!(t.pointer(), Some(egui::pos2(111.0, 222.0)), "the reply came after the move ran");
    assert_eq!(replies[2]["ok"], false);
    assert!(closed, "a line that is no request closes the connection");
}

/// `tool.select` makes the named tool current and is still the Select tool's own command; `tool.list` names the tools.
#[test]
fn tool_select_and_tool_list_choose_and_name_the_tools() {
    let mut t = TestApp::new();
    let chosen = t.ok("engine.execute", json!({"command": "tool.select", "params": {"tool": "zoom"}}));
    assert_eq!(chosen, json!({"tool": "zoom", "previous": "select"}));
    let listed = t.ok("engine.execute", json!({"command": "tool.list"}));
    assert_eq!(listed["active"], "zoom");
    assert!(listed["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| tool["id"] == "draw")), "{listed}");

    // Without `tool`, as its toolbar button and key fire it, it is the Select tool's command.
    assert_eq!(t.ok("engine.execute", json!({"command": "tool.select"})), json!({"tool": "select", "previous": "zoom"}));

    // A `tool` naming no tool is refused and changes nothing.
    t.ok("engine.execute", json!({"command": "tool.draw"}));
    for bad in [json!({"tool": "nope"}), json!({"tool": 5})] {
        let error = t.error("engine.execute", json!({"command": "tool.select", "params": bad}));
        assert!(error.contains("tool"), "{error}");
        assert_eq!(t.ok("ui.inspect", json!({}))["tool"], "draw");
    }

    // The catalog takes `tool.select` once: as the Select tool's definition.
    assert!(t.app.catalog.error.is_none(), "{:?}", t.app.catalog.error);
}

/// The events ring records a command, the document it opened and a failure; `since` reads only what came after, and
/// `events.clear` empties the ring.
#[test]
fn events_record_commands_and_documents() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-events-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("events.glyphs").to_string_lossy().into_owned();
    let mut saved = Session::with_document();
    saved.execute("file.saveAs", &json!({"path": path})).unwrap();

    let mut t = TestApp::new();
    let start = t.ok("events.list", json!({}))["last"].clone();
    t.ok("engine.execute", json!({"command": "file.open", "params": {"path": path}}));
    let after = t.ok("events.list", json!({"since": start}));
    let events = after["events"].as_array().cloned().unwrap_or_default();
    assert!(events.iter().any(|e| e["kind"] == "command" && e["command"] == "file.open" && e["ok"] == true), "{after}");
    let opened = |e: &Value| e["kind"] == "document" && e["action"] == "opened" && e["path"].as_str().is_some_and(|p| p.ends_with("events.glyphs"));
    assert!(events.iter().any(opened), "{after}");
    assert_eq!(after["dropped"], false);

    let missing = dir.join("missing.glyphs").to_string_lossy().into_owned();
    t.error("engine.execute", json!({"command": "file.open", "params": {"path": missing}}));
    let failed = t.ok("events.list", json!({"since": after["last"].clone()}));
    let failed = failed["events"].as_array().cloned().unwrap_or_default();
    assert!(failed.iter().any(|e| e["command"] == "file.open" && e["ok"] == false && e["error"].is_string()), "{failed:?}");
    assert!(!failed.iter().any(opened), "only what came after `since`");

    assert!(t.ok("events.clear", json!({}))["cleared"].as_u64().is_some_and(|n| n > 0));
    assert_eq!(t.ok("events.list", json!({}))["events"], json!([]));
    assert!(t.error("events.list", json!({"since": -1})).contains("since"));
    assert!(t.error("ui.wait", json!({})).starts_with("ui.wait:"), "a host without frames refuses a wait");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `ui.wait` over TCP: after the request that opened a dialog, the idle wait returns with the dialog showing and its
/// event recorded; a frame count waits for the frames; a timeout above the limit is refused at once.
#[test]
fn ui_wait_returns_once_the_window_is_idle() {
    let mut t = TestApp::new();
    t.ok("engine.execute", json!({"command": "path.addRectangle", "params": {"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200}}));
    t.app.sent.push(Msg::View(crate::app::ViewMsg::SelectGlyph("A".into())));
    t.frame();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let ctx = t.frames.ctx.clone();
    let addr = t.app.control.listen(listener, &ctx).unwrap();
    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut replies = Vec::new();
        for line in [
            r#"{"id": 1, "method": "ui.menu.invoke", "params": {"command": "filter.roundCorners"}}"#,
            r#"{"id": 2, "method": "ui.wait", "params": {"timeoutMs": 10000}}"#,
            r#"{"id": 3, "method": "events.list"}"#,
            r#"{"id": 4, "method": "ui.wait", "params": {"frames": 3}}"#,
            r#"{"id": 5, "method": "ui.wait", "params": {"timeoutMs": 30001}}"#,
        ] {
            writeln!(writer, "{line}").unwrap();
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            replies.push(serde_json::from_str::<Value>(&reply).unwrap());
        }
        replies
    });
    for _ in 0..2_000 {
        if client.is_finished() {
            break;
        }
        t.frame();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let replies = client.join().unwrap();
    assert_eq!(replies[0]["result"], json!({"dialog": "dialog.filterRoundCorners"}), "{}", replies[0]);
    let waited = &replies[1]["result"];
    assert_eq!((&waited["idle"], &waited["timedOut"]), (&json!(true), &json!(false)), "{}", replies[1]);
    assert!(waited["frames"].as_u64().is_some_and(|n| n >= super::wait::IDLE_FRAMES), "{waited}");
    let events = replies[2]["result"]["events"].as_array().cloned().unwrap_or_default();
    assert!(events.iter().any(|e| e["kind"] == "dialog" && e["action"] == "opened" && e["dialog"] == "dialog.filterRoundCorners"), "{events:?}");
    assert_eq!(replies[3]["result"]["frames"], 3, "{}", replies[3]);
    assert_eq!(replies[4]["ok"], false);
    assert!(replies[4]["error"].as_str().is_some_and(|e| e.contains("timeoutMs")), "{}", replies[4]);
    assert!(t.app.dialogs.open.is_some(), "the dialog is still open");
}
