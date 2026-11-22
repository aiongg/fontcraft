//! FontCraft test helpers.
//!
//! - [`Fixture`]: sessions in known states (no document, a new font, a font with drawn glyphs).
//! - [`junk_values`] / [`junk_params`]: hostile parameter values for sweeping the command registry.
//! - [`check_session`]: the invariants every command must leave true.
//! - [`catch_quiet`]: run something that may panic without the panic message in the test output.
//! - [`temp_dir`]: a per-process scratch folder.
//!
//! This crate may only be used as a dev-dependency (`cargo xtask layers` enforces it).
// Test support: a failed setup must panic, like `assert!`, so the shipped-code ban on panicking
// (AGENTS.md › Never crash) does not apply here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![forbid(unsafe_code)]

use std::path::PathBuf;

pub use fontcraft_engine::Session;
pub use serde_json::{Value, json};

/// A per-process temporary directory for test output (created on first use).
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-testkit-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Session states tests start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    /// No document open.
    Empty,
    /// One new, untitled font.
    NewFont,
    /// A font with outlines in `A` and `O`, a component in `B`, and one undo and one redo step.
    Drawn,
}

impl Fixture {
    pub const ALL: [Fixture; 3] = [Fixture::Empty, Fixture::NewFont, Fixture::Drawn];

    pub fn session(self) -> Session {
        match self {
            Fixture::Empty => Session::new(),
            Fixture::NewFont => Session::with_document(),
            Fixture::Drawn => {
                let mut s = Session::with_document();
                let run = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("fixture: {id}: {e}"));
                run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
                run(&mut s, "spacing.setWidth", json!({"glyphs": ["A"], "value": 620}));
                run(&mut s, "path.addPath", json!({"glyph": "O", "nodes": oval(350.0, 350.0, 300.0, 360.0)}));
                run(&mut s, "glyph.newGlyph", json!({"name": "extra", "unicodes": ["E000"]}));
                run(&mut s, "glyph.newGlyph", json!({"name": "undone"}));
                run(&mut s, "edit.undo", json!({}));
                s
            }
        }
    }
}

/// The nodes of a rectangle, for `path.addPath`.
pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

/// The nodes of a four-curve oval, for `path.addPath`.
pub fn oval(cx: f64, cy: f64, rx: f64, ry: f64) -> Value {
    let k = 0.5523;
    let on = |x: f64, y: f64| json!({"x": x, "y": y, "type": "curve", "smooth": true});
    let off = |x: f64, y: f64| json!({"x": x, "y": y, "type": "offcurve"});
    json!([
        on(cx + rx, cy),
        off(cx + rx, cy + ry * k),
        off(cx + rx * k, cy + ry),
        on(cx, cy + ry),
        off(cx - rx * k, cy + ry),
        off(cx - rx, cy + ry * k),
        on(cx - rx, cy),
        off(cx - rx, cy - ry * k),
        off(cx - rx * k, cy - ry),
        on(cx, cy - ry),
        off(cx + rx * k, cy - ry),
        off(cx + rx, cy - ry * k),
    ])
}

/// Values no command expects: wrong types, extremes, empties, deep nesting, hostile strings.
pub fn junk_values() -> Vec<Value> {
    let mut deep = json!(1);
    for _ in 0..64 {
        deep = json!([deep]);
    }
    vec![
        Value::Null,
        json!(true),
        json!(0),
        json!(-1),
        json!(1),
        json!(0.5),
        json!(65535),
        json!(65536),
        json!(1e9),
        json!(-1e18),
        json!(1e300),
        json!(i64::MIN),
        json!(u64::MAX),
        json!(""),
        json!(" "),
        json!("A"),
        json!(".notdef"),
        json!("nope"),
        json!("m01"),
        json!("0041"),
        json!("truetype"),
        json!("cff"),
        json!("../../etc/passwd"),
        json!("\u{0}\u{FFFF}\u{1F600}"),
        json!("x".repeat(10_000)),
        json!([]),
        json!(["A"]),
        json!(["A", "A", "nope"]),
        json!([1, "two", null, {}]),
        json!([{"x": 0, "y": 0}]),
        json!([{"x": "a"}]),
        json!([{"x": 1e308, "y": -1e308, "type": "curve"}]),
        json!([{"x": 0, "y": 0, "type": "offcurve"}, {"x": 1, "y": 1, "type": "offcurve"}]),
        json!({}),
        json!({"familyName": ""}),
        json!({"unitsPerEm": 0}),
        json!({"unitsPerEm": 2048, "familyName": "Junk"}),
        json!({"a": {"b": {"c": []}}}),
        deep,
    ]
}

/// The parameter names in a command's params doc: the words before `:` or `?:` in its first
/// `{…}` group. `"{glyphs: [names], layer?: id}"` → `["glyphs", "layer"]`.
pub fn param_keys(doc: &str) -> Vec<String> {
    let head = doc.split('→').next().unwrap_or("");
    let mut keys = Vec::new();
    let mut word = String::new();
    let mut depth = 0i32;
    for c in head.chars() {
        match c {
            '{' | '[' | '(' => {
                depth += 1;
                word.clear();
            }
            '}' | ']' | ')' => {
                depth -= 1;
                word.clear();
            }
            ':' if depth == 1 => {
                let key = word.trim_end_matches('?').to_string();
                if !key.is_empty() && !keys.contains(&key) {
                    keys.push(key);
                }
                word.clear();
            }
            c if c.is_alphanumeric() || c == '?' || c == '_' => word.push(c),
            _ => word.clear(),
        }
    }
    keys
}

/// Parameter objects to throw at a command: non-objects, every key set to every junk value (one at
/// a time, the others absent), and every key set at once. Keys that name files (`path`,
/// `destination`, and the command's declared file parameters `files`) also get a path inside `scratch`, so a command
/// that writes does so there.
pub fn junk_params(doc: &str, files: &[&str], scratch: &std::path::Path) -> Vec<Value> {
    let keys = param_keys(doc);
    let is_file_key = |key: &str| is_file_key(key) || files.contains(&key);
    let junk = junk_values();
    let mut out: Vec<Value> = vec![Value::Null, json!({}), json!([]), json!("x"), json!(7), json!({"unknownKey": 1})];
    for key in &keys {
        for v in &junk {
            // Never point a file parameter at a junk location outside the scratch folder.
            if is_file_key(key) && v.is_string() {
                continue;
            }
            out.push(json!({ key.as_str(): v }));
        }
        if is_file_key(key) {
            out.push(json!({ key.as_str(): scratch.join("junk-out.bin").to_string_lossy() }));
            out.push(json!({ key.as_str(): scratch.join("no-such-folder").join("x").to_string_lossy() }));
            out.push(json!({ key.as_str(): scratch.to_string_lossy() }));
        }
    }
    for v in &junk {
        let mut all = serde_json::Map::new();
        for key in &keys {
            let value = if is_file_key(key) { json!(scratch.join("junk-all.bin").to_string_lossy()) } else { v.clone() };
            all.insert(key.clone(), value);
        }
        out.push(Value::Object(all));
    }
    out
}

fn is_file_key(key: &str) -> bool {
    matches!(key, "path" | "destination" | "paths" | "folder" | "file")
}

/// Runs `f`; a panic becomes `Err(message)` without printing the panic to the test output.
pub fn catch_quiet<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    use std::sync::Mutex;
    // The hook is process-wide: take it for one call at a time.
    static HOOK: Mutex<()> = Mutex::new(());
    let _guard = HOOK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(previous);
    result.map_err(|payload| fontcraft_engine::guard::panic_message(payload.as_ref()))
}

/// The invariants every command must leave true. `Err` describes the first one broken.
pub fn check_session(s: &Session) -> Result<(), String> {
    for (i, d) in s.documents().iter().enumerate() {
        d.font.check().map_err(|e| format!("document {i}: {e}"))?;
        for h in d.history.undo.iter().chain(&d.history.redo) {
            h.font.check().map_err(|e| format!("document {i}, history step `{}`: {e}", h.label))?;
        }
        if d.title.is_empty() {
            return Err(format!("document {i} has no title"));
        }
    }
    let inspect = s.inspect();
    match (inspect.get("active").and_then(Value::as_u64), s.documents().len()) {
        (None, 0) => Ok(()),
        (Some(a), n) if (a as usize) < n => Ok(()),
        (a, n) => Err(format!("active document {a:?} with {n} documents open")),
    }
}

/// Everything about a session a failed command must leave unchanged.
pub fn snapshot(s: &Session) -> Value {
    let fonts: Vec<Value> = s.documents().iter().map(|d| serde_json::to_value(d.font.as_ref()).expect("a font serializes")).collect();
    json!({ "inspect": s.inspect(), "fonts": fonts, "journal": s.journal().len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_valid() {
        for fx in Fixture::ALL {
            check_session(&fx.session()).unwrap();
        }
        let s = Fixture::Drawn.session();
        let doc = s.doc().unwrap();
        assert_eq!((doc.history.undo.len(), doc.history.redo.len()), (4, 1));
    }

    #[test]
    fn param_keys_come_from_the_first_group() {
        assert_eq!(
            param_keys("{glyphs: [names], layer?: master id (default: x), value: number} sets it → {glyphs: [names]}"),
            ["glyphs", "layer", "value"]
        );
        assert_eq!(param_keys("{} nothing → {a: b}"), Vec::<String>::new());
        assert_eq!(param_keys("{values: {familyName?: string}}"), ["values"]);
        assert_eq!(param_keys("{nodes: [{x, y, type?: \"line\"}], closed?: bool = true}"), ["nodes", "closed"]);
    }

    #[test]
    fn catch_quiet_reports_panics() {
        assert_eq!(catch_quiet(|| 1), Ok(1));
        assert_eq!(catch_quiet(|| -> u8 { panic!("no") }), Err("no".to_string()));
    }
}
