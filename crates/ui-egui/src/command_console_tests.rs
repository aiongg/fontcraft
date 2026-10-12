//! The Command Console (`window.macroPanel`): typed lines and run-style lines run and are logged,
//! bad lines change nothing, completion of command ids and glyph names with the parameters doc,
//! the history with Up and Down, module log lines in the pane, the standard-error setting, and
//! the speed of completion.

use egui::{Event, Key, Modifiers, Pos2};
use fontcraft_engine::log::LogKind;
use fontcraft_model::{Font, Glyph};
use serde_json::{Value, json};

use crate::panels::command_console::input_id;
use crate::shell::command_console::{self, MAX_HISTORY, MAX_LINE_BYTES, Recall, SYSTEM_CONSOLE_SETTING, SuggestionKind};
use crate::{FontcraftApp, control};

fn exec(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], true, "{command} {r}");
    r["result"].clone()
}

fn refused(app: &mut FontcraftApp, command: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], false, "{command} {r}");
    r.to_string()
}

fn console_line(app: &mut FontcraftApp, line: &str) -> Value {
    exec(app, "window.macroPanel", json!({ "line": line }))
}

fn glyph_count(app: &FontcraftApp) -> usize {
    app.session.font().map(|f| f.glyphs.len()).unwrap_or(0)
}

fn has_glyph(app: &FontcraftApp, name: &str) -> bool {
    app.session.font().is_ok_and(|f| f.glyph(name).is_some())
}

/// The newest log entry.
fn last_entry(app: &FontcraftApp) -> LogKind {
    let log = &app.session.extensions().log;
    log.get(log.len() - 1).expect("an entry").kind.clone()
}

#[test]
fn window_macro_panel_runs_a_typed_command_and_logs_it() {
    let mut app = FontcraftApp::default();
    let r = exec(&mut app, "window.macroPanel", json!({}));
    assert_eq!(r["open"], true);

    let r = console_line(&mut app, r#"glyph.newGlyph {"name": "x.alt"}"#);
    assert_eq!(r["result"]["name"], "x.alt");
    assert_eq!(r["history"], 1);
    assert!(has_glyph(&app, "x.alt"));
    let LogKind::Command { command, result: Some(result), error: None, .. } = last_entry(&app) else { panic!("a command entry") };
    assert_eq!(command, "glyph.newGlyph");
    assert_eq!(result["name"], "x.alt");
    let pane = command_console::pane_text(&app.state.panels.command_console, &app.session.extensions().log);
    assert!(pane.contains(r#"glyph.newGlyph {"name":"x.alt"} → "#), "{pane}");

    // A window command run from the console is logged by the console.
    console_line(&mut app, r#"window.textPreview {"open": false}"#);
    let LogKind::Command { command, error: None, .. } = last_entry(&app) else { panic!("a command entry") };
    assert_eq!(command, "window.textPreview");

    // Clear empties the pane; the session's log keeps its entries.
    let kept = app.session.extensions().log.len();
    let r = exec(&mut app, "window.macroPanel", json!({"clear": true}));
    assert_eq!(r["shown"], 0);
    assert_eq!(r["open"], true);
    assert_eq!(app.session.extensions().log.len(), kept);
    exec(&mut app, "glyph.newGlyph", json!({"name": "y.alt"}));
    let r = exec(&mut app, "window.macroPanel", json!({"copy": true}));
    assert_eq!(r["shown"], 1, "a command from another surface shows in the pane");
    assert!(app.clipboard_out.as_deref().is_some_and(|t| t.starts_with("glyph.newGlyph {\"name\":\"y.alt\"}")));

    let r = exec(&mut app, "window.macroPanel", json!({"open": false}));
    assert_eq!(r["open"], false);
}

#[test]
fn window_macro_panel_runs_a_run_style_line_as_one_step() {
    let mut app = FontcraftApp::default();
    let line = r#"run --cmd glyph.newGlyph --params '{"name": "y.alt"}' --cmd glyph.newGlyph --params "{\"name\": \"z.alt\"}""#;
    let r = console_line(&mut app, line);
    assert_eq!(r["result"].as_array().map(Vec::len), Some(2), "{r}");
    assert!(has_glyph(&app, "y.alt") && has_glyph(&app, "z.alt"));
    console_line(&mut app, r#"fontcraft-cli run --cmd glyph.newGlyph --params '{"name":"w.alt"}'"#);
    assert!(has_glyph(&app, "w.alt"));

    // One failing command: none of the line's commands takes effect.
    let count = glyph_count(&app);
    let e =
        refused(&mut app, "window.macroPanel", json!({"line": r#"run --cmd glyph.newGlyph --params '{"name":"q.alt"}' --cmd glyph.noSuchCommand"#}));
    assert!(e.contains("noSuchCommand"), "{e}");
    assert_eq!(glyph_count(&app), count);
    assert!(!has_glyph(&app, "q.alt"));

    // File options and window commands in a line of several commands are refused before anything runs.
    let e = refused(&mut app, "window.macroPanel", json!({"line": r#"run --cmd glyph.newGlyph --params '{"name":"v.alt"}' --export out.ttf"#}));
    assert!(e.contains("--export"), "{e}");
    let e = refused(&mut app, "window.macroPanel", json!({"line": "run --cmd glyph.newGlyph --cmd window.textPreview"}));
    assert!(e.contains("window.textPreview"), "{e}");
    assert_eq!(glyph_count(&app), count);
}

#[test]
fn window_macro_panel_refuses_bad_lines_and_logs_why() {
    let mut app = FontcraftApp::default();
    let count = glyph_count(&app);
    for (line, why) in [
        ("glyph.noSuchCommand {}", "glyph.noSuchCommand"),
        (r#"glyph.newGlyph {"name": }"#, "not JSON"),
        ("glyph.newGlyph [1]", "JSON object"),
        (r#"run --cmd glyph.newGlyph --params '{"name": "a"}"#, "quote"),
        ("run --params {}", "after the --cmd"),
        (r#"window.macroPanel {"line": "glyph.newGlyph"}"#, "another console line"),
    ] {
        let e = refused(&mut app, "window.macroPanel", json!({ "line": line }));
        assert!(e.contains(why), "{line}: {e}");
        let LogKind::Command { error: Some(error), .. } = last_entry(&app) else { panic!("{line}: an error entry") };
        assert!(error.contains(why), "{line}: {error}");
    }
    let history = app.state.panels.command_console.history.len();
    let e = refused(&mut app, "window.macroPanel", json!({"line": "x".repeat(MAX_LINE_BYTES + 1)}));
    assert!(e.contains("at most"), "{e}");
    assert_eq!(app.state.panels.command_console.history.len(), history, "an over-long line is not kept");
    assert_eq!(glyph_count(&app), count);
}

#[test]
fn console_completes_command_ids_and_glyph_names_with_the_params_doc() {
    let mut font = Font::new("Test");
    for name in ["alpha", "alpha.sc", "beta", "Alpha"] {
        font.glyphs.push(Glyph::new(name));
    }
    let c = command_console::complete(Some(&font), "glyph.newG");
    assert_eq!(c.start, 0);
    assert!(c.items.iter().any(|s| s.text == "glyph.newGlyph" && s.kind == SuggestionKind::Command));
    let (id, doc) = c.doc.expect("a doc");
    assert_eq!(id, "glyph.newGlyph");
    assert!(doc.starts_with("{name?: string"), "{doc}");
    assert_eq!(command_console::apply("glyph.newG", &c, 0).as_deref(), Some("glyph.newGlyph "));

    // Window commands complete too.
    let c = command_console::complete(None, "window.macroP");
    assert_eq!(c.items.first().map(|s| s.text.as_str()), Some("window.macroPanel"));

    // Glyph names inside a string value, starting matches first, then any case.
    let line = r#"glyph.newGlyph {"name": "al"#;
    let c = command_console::complete(Some(&font), line);
    let names: Vec<&str> = c.items.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(names, ["alpha", "alpha.sc", "Alpha"]);
    assert_eq!(c.doc.map(|d| d.0), Some("glyph.newGlyph"));
    assert_eq!(command_console::apply(line, &c, 1).as_deref(), Some(r#"glyph.newGlyph {"name": "alpha.sc""#));
    let c = command_console::complete(Some(&font), r#"view.editSelectedGlyphs {"glyphs": ["beta", "al"#);
    assert_eq!(c.items.len(), 3);

    // Not inside a key, nor outside a string.
    assert!(command_console::complete(Some(&font), r#"glyph.newGlyph {"al"#).items.is_empty());
    assert!(command_console::complete(Some(&font), r#"glyph.newGlyph {"name": "alpha", "#).items.is_empty());

    // A run-style line: ids after --cmd, glyph names in --params.
    let c = command_console::complete(Some(&font), "run --cmd glyph.newG");
    assert!(c.items.iter().any(|s| s.text == "glyph.newGlyph"));
    let c = command_console::complete(Some(&font), r#"run --cmd glyph.newGlyph --params '{"name": "be"#);
    assert_eq!(c.items.first().map(|s| s.text.as_str()), Some("beta"));
    assert_eq!(c.doc.map(|d| d.0), Some("glyph.newGlyph"));
}

#[test]
fn console_history_walks_with_up_and_down() {
    let mut app = FontcraftApp::default();
    exec(&mut app, "window.macroPanel", json!({}));
    for name in ["a1", "a2", "a3"] {
        console_line(&mut app, &format!(r#"glyph.newGlyph {{"name": "{name}"}}"#));
    }
    let st = &mut app.state.panels.command_console;
    st.input = "draft".into();
    st.recall(Recall::Previous);
    st.recall(Recall::Previous);
    assert_eq!(st.input, r#"glyph.newGlyph {"name": "a2"}"#);
    st.recall(Recall::Next);
    assert_eq!(st.input, r#"glyph.newGlyph {"name": "a3"}"#);
    st.recall(Recall::Next);
    assert_eq!(st.input, "draft");

    // The keys in the window: Up recalls, Return runs.
    let ctx = egui::Context::default();
    let frame = |app: &mut FontcraftApp, events: Vec<Event>| {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    };
    let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    app.state.panels.command_console.input.clear();
    for _ in 0..3 {
        frame(&mut app, Vec::new());
    }
    ctx.memory_mut(|m| m.request_focus(input_id()));
    frame(&mut app, Vec::new());
    frame(&mut app, vec![key(Key::ArrowUp)]);
    assert_eq!(app.state.panels.command_console.input, r#"glyph.newGlyph {"name": "a3"}"#);
    app.state.panels.command_console.input = r#"glyph.newGlyph {"name": "typed"}"#.into();
    frame(&mut app, vec![key(Key::Enter)]);
    assert!(has_glyph(&app, "typed"), "Return runs the line");
    assert!(app.state.panels.command_console.input.is_empty());

    // The history keeps the newest lines.
    let st = &mut app.state.panels.command_console;
    for i in 0..MAX_HISTORY + 5 {
        st.remember(&format!("line {i}"));
    }
    assert_eq!(st.history.len(), MAX_HISTORY);
    assert_eq!(st.history.first().map(String::as_str), Some("line 5"));
}

#[test]
fn console_shows_module_log_lines() {
    let mut app = FontcraftApp::default();
    exec(&mut app, "window.macroPanel", json!({}));
    app.session.extensions_mut().log.push_module_line("com.example.round", "rounded 12 points");
    let pane = command_console::pane_text(&app.state.panels.command_console, &app.session.extensions().log);
    assert!(pane.contains("[com.example.round] rounded 12 points"), "{pane}");
}

#[test]
fn console_output_setting_writes_the_log_to_standard_error() {
    let mut app = FontcraftApp::default();
    let mut err = Vec::new();
    exec(&mut app, "glyph.newGlyph", json!({"name": "quiet"}));
    command_console::echo(&mut app, &mut err);
    assert!(err.is_empty(), "off by default");

    exec(&mut app, "settings.set", json!({"id": SYSTEM_CONSOLE_SETTING, "value": true}));
    exec(&mut app, "glyph.newGlyph", json!({"name": "loud"}));
    app.session.extensions_mut().log.push_module_line("com.example.round", "rounded");
    command_console::echo(&mut app, &mut err);
    let text = String::from_utf8(std::mem::take(&mut err)).unwrap();
    assert!(!text.contains("quiet"), "{text}");
    assert!(text.contains("fontcraft: glyph.newGlyph {\"name\":\"loud\"}"), "{text}");
    assert!(text.contains("fontcraft: [com.example.round] rounded"), "{text}");
    command_console::echo(&mut app, &mut err);
    assert!(err.is_empty(), "each entry is written once");

    exec(&mut app, "settings.set", json!({"id": SYSTEM_CONSOLE_SETTING, "value": false}));
    exec(&mut app, "glyph.newGlyph", json!({"name": "after"}));
    command_console::echo(&mut app, &mut err);
    assert!(err.is_empty());
}

/// `cargo test --release -p fontcraft-ui-egui console_completion -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn console_completion_meets_its_speed_target() {
    let mut font = Font::new("Speed");
    for i in 0..65_000 {
        font.glyphs.push(Glyph::new(format!("uni{i:04X}.alt")));
    }
    let lines = ["glyph.", "run --cmd path.add", r#"view.editSelectedGlyphs {"glyphs": ["uni00"#, r#"glyph.newGlyph {"name": "zzz"#];
    for line in lines {
        let mut best = std::time::Duration::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let c = command_console::complete(Some(&font), line);
            best = best.min(start.elapsed());
            std::hint::black_box(c);
        }
        println!("{line}: {best:?}");
        if !cfg!(debug_assertions) {
            assert!(best.as_micros() < 5_000, "{line} took {best:?}");
        }
    }
}
