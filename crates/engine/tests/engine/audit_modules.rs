//! Check modules in the audit (M13.5, row `plugin.kind.check`): their findings join `audit.run`
//! under `check.plugin.<module>.<name>`, reach the results the canvas markers read, and go away
//! with the module.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Installs a check module `id` that reports `output` on every layer.
fn install_check(s: &mut Session, id: &str, output: Value) {
    let manifest = json!({"id": id, "name": format!("Test {id}"), "version": "1", "kind": "check", "params": {}}).to_string();
    s.extensions_mut().registry.install_bytes(&fontcraft_plugins::fixtures::returning(&manifest, &output.to_string()), None).unwrap();
}

/// A document whose `A` has one square, with the id of `A`'s master layer.
fn square_doc() -> (Session, String) {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0), "closed": true}));
    let layer = s.font().unwrap().glyph("A").unwrap().layers[0].id.clone();
    (s, layer)
}

fn width_findings() -> Value {
    json!({"findings": [
        {"name": "wide", "severity": "warning", "message": "Too wide", "position": [10, 20]},
        {"name": "tiny", "severity": "error", "message": "Too small"},
    ]})
}

fn result<'a>(doc: &'a Value, check: &str) -> Option<&'a Value> {
    doc["results"].as_array().unwrap().iter().find(|r| r["check"] == check)
}

fn module_results(doc: &Value) -> usize {
    doc["results"].as_array().unwrap().iter().filter(|r| r["check"].as_str().unwrap().starts_with("check.plugin.")).count()
}

fn check_entry<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    doc["checks"].as_array().unwrap().iter().find(|c| c["id"] == id)
}

const WIDE: &str = "check.plugin.org.test.width.wide";
const TINY: &str = "check.plugin.org.test.width.tiny";

#[test]
fn plugin_kind_check_findings_join_audit_run_and_the_markers() {
    let (mut s, layer) = square_doc();
    install_check(&mut s, "org.test.width", width_findings());

    let doc = run(&mut s, "audit.run", json!({"checks": ["check.plugin.*"], "glyphs": ["A"]}));
    let wide = result(&doc, WIDE).unwrap();
    assert_eq!(wide["severity"], "warn");
    assert_eq!(wide["message"], "Too wide");
    assert_eq!(wide["location"]["glyph"], "A");
    assert_eq!(wide["location"]["layer"], layer.as_str());
    assert_eq!(wide["location"]["point"], json!([10.0, 20.0]));
    assert_eq!(wide["fix"]["available"], false);
    assert_eq!(result(&doc, TINY).unwrap()["severity"], "fail");
    assert_eq!(check_entry(&doc, WIDE).unwrap()["status"], "warn");
    assert_eq!(check_entry(&doc, TINY).unwrap()["status"], "fail");
    assert_eq!((doc["summary"]["fail"].clone(), doc["summary"]["warn"].clone()), (json!(1), json!(1)));

    // What the canvas markers read: live audit's call, then the kept run for the edited glyph.
    let live = run(&mut s, "audit.run", json!({"profile": "outlines", "glyphs": ["A"], "layers": [layer]}));
    assert!(result(&live, WIDE).is_some(), "{live:#}");
    let kept = run(&mut s, "audit.results", json!({"glyphs": ["A"]}));
    let wide = result(&kept, WIDE).unwrap();
    assert_eq!((wide["location"]["layer"].clone(), wide["stale"].clone()), (json!(layer), json!(false)));

    // The filters of audit.run apply to module checks like the others.
    let doc = run(&mut s, "audit.run", json!({"checks": ["check.plugin.*"], "exclude": [WIDE], "glyphs": ["A"]}));
    assert!(result(&doc, WIDE).is_none() && result(&doc, TINY).is_some());
    let doc = run(&mut s, "audit.run", json!({"checks": [TINY], "glyphs": ["A"]}));
    assert_eq!(module_results(&doc), 1);
    let doc = run(&mut s, "audit.run", json!({"profile": "outlines", "groups": ["plugin"], "glyphs": ["A"]}));
    assert_eq!(doc["results"].as_array().unwrap().len(), 2);
    let doc = run(&mut s, "audit.run", json!({"profile": "outlines", "groups": ["outlines"], "glyphs": ["A"]}));
    assert_eq!(module_results(&doc), 0);
    let doc = run(&mut s, "audit.run", json!({"checks": ["check.plugin.*"], "minSeverity": "fail", "glyphs": ["A"]}));
    assert!(result(&doc, WIDE).is_none() && result(&doc, TINY).is_some());
}

#[test]
fn plugin_kind_check_removing_the_module_removes_its_checks() {
    let (mut s, _) = square_doc();
    install_check(&mut s, "org.test.width", width_findings());
    let audit = |s: &mut Session| run(s, "audit.run", json!({"checks": ["check.plugin.*"], "glyphs": ["A"], "includePassed": true}));
    assert_eq!(module_results(&audit(&mut s)), 2);

    s.extensions_mut().registry.set_enabled("org.test.width", false).unwrap();
    assert_eq!(module_results(&audit(&mut s)), 0);
    s.extensions_mut().registry.set_enabled("org.test.width", true).unwrap();

    s.extensions_mut().safe_mode = true;
    assert_eq!(module_results(&audit(&mut s)), 0);
    s.extensions_mut().safe_mode = false;
    assert_eq!(module_results(&audit(&mut s)), 2);

    assert!(s.extensions_mut().registry.remove("org.test.width"));
    let doc = audit(&mut s);
    assert_eq!(module_results(&doc), 0);
    assert!(check_entry(&doc, WIDE).is_none());
    assert_eq!(module_results(&run(&mut s, "audit.results", json!({}))), 0);
}

#[test]
fn plugin_kind_check_failing_module_is_an_error_and_changes_nothing() {
    let (mut s, _) = square_doc();
    install_check(&mut s, "org.test.broken", json!({"error": "no metrics"}));
    install_check(&mut s, "org.test.far", json!({"findings": [{"name": "x", "severity": "info", "message": "far", "position": [1e9, 0]}]}));
    install_check(&mut s, "org.test.quiet", json!({"findings": []}));
    let before = serde_json::to_value(s.font().unwrap()).unwrap();

    let doc = run(&mut s, "audit.run", json!({"checks": ["check.plugin.*"], "includePassed": true}));
    let broken = check_entry(&doc, "check.plugin.org.test.broken").unwrap();
    assert_eq!(broken["status"], "error");
    assert!(broken["message"].as_str().unwrap().contains("no metrics"), "{broken}");
    assert_eq!(check_entry(&doc, "check.plugin.org.test.far").unwrap()["status"], "error");
    assert_eq!(check_entry(&doc, "check.plugin.org.test.quiet").unwrap()["status"], "pass");
    assert_eq!(doc["summary"]["errors"], 2);
    assert_eq!(module_results(&doc), 0);
    assert_eq!(serde_json::to_value(s.font().unwrap()).unwrap(), before);
}
