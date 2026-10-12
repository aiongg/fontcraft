//! Audit profiles, settings, the ignore list and the result cache (M12.59): user and project
//! profiles with includes and overrides, defaults, threshold and project values, ignore entries
//! that survive a save and reopen in each source format, and refusals that change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use fontcraft_engine::clock::fixed_clock;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn font_json(s: &Session) -> Value {
    serde_json::to_value(s.font().unwrap()).unwrap()
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Runs `id`, expects it to fail, and checks that neither the font, its undo history nor the
/// user settings changed.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let (font, steps, settings) = (font_json(s), undo_steps(s), s.settings().clone());
    let err = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should be refused")).to_string();
    assert_eq!(font_json(s), font, "{id} {p} changed the font");
    assert_eq!(undo_steps(s), steps, "{id} {p} made an undo step");
    assert!(*s.settings() == settings, "{id} {p} changed the user settings");
    err
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-profiles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn outline_check_count(s: &mut Session) -> usize {
    run(s, "audit.profile.show", json!({"profile": "outlines"}))["resolved"]["count"].as_u64().unwrap() as usize
}

#[test]
fn profiles_include_override_and_delete_in_both_stores() {
    let mut s = Session::with_document();
    let outlines = outline_check_count(&mut s);

    let saved = run(
        &mut s,
        "audit.profile.save",
        json!({
            "profile": "house",
            "label": "House rules",
            "include": ["outlines"],
            "exclude": ["check.outlines.tinyContour"],
            "overrides": [{"check": "check.outlines.overlap", "severity": "fail"}],
        }),
    );
    assert_eq!(saved["created"], true);
    assert_eq!(saved["checks"], outlines - 1);
    assert_eq!(saved["profile"]["store"], "user");
    // A user profile changes the settings, not the font.
    assert_eq!(undo_steps(&s), 0);

    let shown = run(&mut s, "audit.profile.show", json!({"profile": "house"}));
    assert_eq!(shown["label"], "House rules");
    assert_eq!(shown["overrides"][0]["severity"], "fail");
    let ids = shown["resolved"]["checks"].as_array().unwrap();
    assert!(!ids.iter().any(|c| c == "check.outlines.tinyContour"));
    assert!(ids.iter().any(|c| c == "check.outlines.overlap"));

    // A project profile builds on built-in and project profiles, and is one undo step.
    run(&mut s, "audit.profile.save", json!({"profile": "repo", "include": ["outlines"], "store": "project"}));
    assert_eq!(undo_steps(&s), 1);
    let top = run(
        &mut s,
        "audit.profile.save",
        json!({"profile": "repo-strict", "include": ["repo"], "checks": ["check.outlines.*"], "exclude": ["check.outlines.overlap"], "store": "project"}),
    );
    assert_eq!(top["profile"]["store"], "project");
    let shown = run(&mut s, "audit.profile.show", json!({"profile": "repo-strict"}));
    assert!(!shown["resolved"]["checks"].as_array().unwrap().iter().any(|c| c == "check.outlines.overlap"));

    // Refusals change nothing.
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "strict", "include": ["universal"]})).contains("built-in"));
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "x", "include": ["house"], "store": "project"})).contains("project profile"));
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "repo", "include": ["outlines"]})).contains("project profile"));
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "x", "checks": ["check.nothing"]})).contains("no check matches"));
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "x", "include": ["missing"]})).contains("missing"));
    assert!(
        refused(
            &mut s,
            "audit.profile.save",
            json!({"profile": "x", "include": ["universal"], "config": {"check.outlines.tinyContour": {"minArea": -5}}})
        )
        .contains("minArea")
    );
    // A cycle: `repo` would include `repo-strict`, which includes `repo`.
    assert!(refused(&mut s, "audit.profile.save", json!({"profile": "repo", "include": ["repo-strict"], "store": "project"})).contains("itself"));
    assert!(refused(&mut s, "audit.profile.delete", json!({"profile": "repo"})).contains("repo-strict"));
    assert!(refused(&mut s, "audit.profile.delete", json!({"profile": "universal"})).contains("built-in"));
    assert!(refused(&mut s, "audit.profile.show", json!({"profile": "nope"})).contains("no profile"));

    // Deleting the default clears it.
    run(&mut s, "audit.profile.setDefault", json!({"profile": "house", "target": "application"}));
    let deleted = run(&mut s, "audit.profile.delete", json!({"profile": "house"}));
    assert_eq!(deleted, json!({"deleted": "house", "store": "user", "defaultCleared": true}));
    run(&mut s, "audit.profile.delete", json!({"profile": "repo-strict"}));
    run(&mut s, "audit.profile.delete", json!({"profile": "repo"}));
    assert!(refused(&mut s, "audit.profile.show", json!({"profile": "repo"})).contains("no profile"));
}

#[test]
fn defaults_and_settings_take_document_over_profile_over_built_in() {
    let mut s = Session::with_document();
    let get = |s: &mut Session| run(s, "audit.config.get", json!({"check": "check.outlines.tinyContour"}));
    let min_area = |v: &Value| v["checks"]["check.outlines.tinyContour"]["minArea"].clone();

    let base = get(&mut s);
    assert_eq!(base["profile"], "universal");
    assert_eq!(base["profileFrom"], "builtIn");
    assert_eq!(min_area(&base)["from"], "default");
    assert_eq!(min_area(&base)["value"], 25.0);
    assert_eq!(base["values"]["vendorId"]["value"], Value::Null);

    run(&mut s, "audit.profile.save", json!({"profile": "fine", "include": ["outlines"], "config": {"check.outlines.tinyContour": {"minArea": 10}}}));
    run(&mut s, "audit.profile.setDefault", json!({"profile": "fine", "target": "application"}));
    let app = get(&mut s);
    assert_eq!(app["profileFrom"], "application");
    assert_eq!(min_area(&app)["from"], "profile");
    assert_eq!(min_area(&app)["value"], 10.0);

    let set = run(&mut s, "audit.profile.setDefault", json!({"profile": "outlines"}));
    assert_eq!(set["target"], "document");
    assert_eq!(set["effective"], "outlines");
    assert_eq!(get(&mut s)["profileFrom"], "document");
    assert_eq!(min_area(&get(&mut s))["from"], "default");

    run(&mut s, "audit.config.set", json!({"check": "check.outlines.tinyContour", "values": {"minArea": 40}}));
    run(&mut s, "audit.config.set", json!({"values": {"vendorId": "ABCD", "keepMacNames": true, "fsType": 8}}));
    let doc = run(&mut s, "audit.config.get", json!({"check": "check.outlines.tinyContour", "profile": "fine"}));
    assert_eq!(doc["profileFrom"], "call");
    assert_eq!(min_area(&doc)["from"], "document");
    assert_eq!(min_area(&doc)["value"], 40.0);
    assert_eq!(doc["values"]["vendorId"]["value"], "ABCD");
    assert_eq!(doc["values"]["keepMacNames"]["value"], true);
    assert_eq!(doc["values"]["fsType"]["value"], 8);

    // null removes an override.
    run(&mut s, "audit.config.set", json!({"check": "check.outlines.tinyContour", "values": {"minArea": null}}));
    run(&mut s, "audit.config.set", json!({"values": {"vendorId": null}}));
    let back = get(&mut s);
    assert_eq!(min_area(&back)["from"], "default");
    assert_eq!(back["values"]["vendorId"]["value"], Value::Null);

    for (params, says) in [
        (json!({"check": "check.outlines.tinyContour", "values": {"nope": 1}}), "no threshold"),
        (json!({"check": "check.outlines.tinyContour", "values": {"minArea": -1}}), "minArea"),
        (json!({"check": "check.outlines.nope", "values": {"minArea": 1}}), "no check"),
        (json!({"values": {"vendorId": "TOO LONG"}}), "vendorId"),
        (json!({"values": {"fsType": 70000}}), "fsType"),
        (json!({"values": {"unknown": 1}}), "no project value"),
        // One bad name refuses the whole call.
        (json!({"values": {"warnSize": 1000, "failSize": -1}}), "failSize"),
        (json!({"values": {}}), "nothing"),
    ] {
        let err = refused(&mut s, "audit.config.set", params.clone());
        assert!(err.contains(says), "{params}: {err}");
    }
    assert!(refused(&mut s, "audit.profile.setDefault", json!({"profile": "missing"})).contains("no profile"));
    run(&mut s, "audit.profile.save", json!({"profile": "here", "include": ["outlines"], "store": "project"}));
    assert!(refused(&mut s, "audit.profile.setDefault", json!({"profile": "here", "target": "application"})).contains("project profile"));

    // A document default that no longer exists falls back to universal.
    run(&mut s, "audit.profile.setDefault", json!({"profile": "here"}));
    run(&mut s, "audit.profile.delete", json!({"profile": "here"}));
    let after = get(&mut s);
    assert_eq!(after["profile"], "fine", "the deleted document default is cleared, so the application default applies");
    run(&mut s, "audit.profile.setDefault", json!({"profile": null}));
    assert_eq!(get(&mut s)["profileFrom"], "application");
}

#[test]
fn ignore_entries_and_project_settings_survive_save_and_reopen_in_each_format() {
    let dir = temp("formats");
    for ext in ["fontcraft", "glyphs", "ufo"] {
        let mut s = Session::with_document();
        s.set_clock(Some(fixed_clock(1_760_000_000)));
        let ignored = run(
            &mut s,
            "audit.ignoreIssue",
            json!({"check": "check.outlines.overlap", "glyph": "A", "master": "m01", "reason": "drawn that way on purpose"}),
        );
        assert_eq!(ignored["added"], true);
        assert_eq!(ignored["entry"]["date"], "2025-10-09 08:53:20 +0000");
        run(
            &mut s,
            "audit.ignoreIssue",
            json!({"check": "check.outlines.tinyContour", "location": {"pair": ["A", "V"], "nameId": 1, "table": "OS/2"}}),
        );
        run(&mut s, "audit.config.set", json!({"check": "check.outlines.tinyContour", "values": {"minArea": 12.5}}));
        run(
            &mut s,
            "audit.config.set",
            json!({"values": {"keepMacNames": true, "allowedTables": ["glyf", "OS/2"], "defaultUrl": "https://example.com/"}}),
        );
        run(
            &mut s,
            "audit.profile.save",
            json!({"profile": "repo", "include": ["outlines"], "overrides": [{"check": "check.outlines.overlap", "code": "crossing", "severity": "info"}], "store": "project"}),
        );
        run(&mut s, "audit.profile.setDefault", json!({"profile": "repo"}));
        let list = run(&mut s, "audit.listIgnored", json!({}));
        let config = run(&mut s, "audit.config.get", json!({}));
        let profile = run(&mut s, "audit.profile.show", json!({"profile": "repo"}));

        let path = dir.join(format!("Test.{ext}"));
        run(&mut s, "file.saveAs", json!({"path": path}));
        // A new session: nothing comes from the first one's settings.
        let mut t = Session::new();
        run(&mut t, "file.open", json!({"path": path}));
        assert_eq!(run(&mut t, "audit.listIgnored", json!({})), list, "{ext}");
        assert_eq!(run(&mut t, "audit.config.get", json!({})), config, "{ext}");
        assert_eq!(run(&mut t, "audit.profile.show", json!({"profile": "repo"})), profile, "{ext}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ignoring_a_result_hides_it_and_listing_tells_whether_it_still_occurs() {
    let mut s = Session::with_document();
    for (glyph, shapes) in [("A", vec![rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 150.0, 150.0)]), ("B", vec![rect(0.0, 0.0, 100.0, 100.0)])] {
        for nodes in shapes {
            run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": nodes, "closed": true}));
        }
    }
    // Before any run nothing can be said.
    run(&mut s, "audit.ignoreIssue", json!({"check": "check.outlines.overlap", "glyph": "B"}));
    assert_eq!(run(&mut s, "audit.listIgnored", json!({}))["entries"][0]["occurs"], Value::Null);
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"result": "r0"})).contains("audit.run"));

    let report = run(&mut s, "audit.run", json!({"checks": ["check.outlines.overlap"]}));
    let result = report["results"].as_array().unwrap().iter().find(|r| r["location"]["glyph"] == "A").unwrap()["id"].as_str().unwrap().to_string();
    let added = run(&mut s, "audit.ignoreIssue", json!({"result": result, "reason": "a test"}));
    assert_eq!(added["entry"]["check"], "check.outlines.overlap");
    assert_eq!(added["entry"]["glyph"], "A");
    assert_eq!(added["entries"], 2);
    // The same issue again replaces the entry.
    let again = run(&mut s, "audit.ignoreIssue", json!({"result": result, "reason": "still a test"}));
    assert_eq!(again["added"], false);
    assert_eq!(again["entries"], 2);
    run(&mut s, "audit.ignoreIssue", json!({"check": "check.outlines.tinyContour"}));

    let listed = run(&mut s, "audit.listIgnored", json!({}));
    assert_eq!(listed["checkedAgainstRun"], true);
    let entries = listed["entries"].as_array().unwrap();
    let occurs = |glyph: &str| entries.iter().find(|e| e["glyph"] == glyph).unwrap()["occurs"].clone();
    assert_eq!(occurs("A"), true);
    assert_eq!(occurs("B"), false);
    assert_eq!(
        entries.iter().find(|e| e["check"] == "check.outlines.tinyContour").unwrap()["occurs"],
        Value::Null,
        "the run did not include the check"
    );
    assert_eq!(entries.iter().find(|e| e["glyph"] == "A").unwrap()["reason"], "still a test");

    // Refusals change nothing.
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"result": result, "glyph": "A"})).contains("not both"));
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"result": "r999999"})).contains("no result"));
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"check": "check.outlines.nope"})).contains("no check"));
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"check": "check.outlines.overlap", "glyph": "nope"})).contains("no glyph"));
    assert!(refused(&mut s, "audit.ignoreIssue", json!({"check": "check.outlines.overlap", "master": "nope"})).contains("no master"));
    assert!(refused(&mut s, "audit.ignoreIssue", json!({})).contains("result"));
    assert!(refused(&mut s, "audit.unignoreIssue", json!({"check": "check.outlines.overlap", "glyph": "Z"})).contains("matches"));

    // A key left out matches any entry; each change is one undo step.
    let steps = undo_steps(&s);
    let removed = run(&mut s, "audit.unignoreIssue", json!({"check": "check.outlines.overlap"}));
    assert_eq!(removed, json!({"removed": 2, "entries": 1}));
    assert_eq!(undo_steps(&s), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "audit.listIgnored", json!({}))["entries"].as_array().unwrap().len(), 3);
}

#[test]
fn clearing_the_cache_drops_the_kept_run() {
    let mut s = Session::with_document();
    run(&mut s, "audit.run", json!({}));
    assert!(s.execute("audit.results", &json!({})).is_ok());
    assert_eq!(run(&mut s, "audit.clearCache", json!({})), json!({"cleared": true}));
    assert!(s.execute("audit.results", &json!({})).is_err());
    assert_eq!(run(&mut s, "audit.clearCache", json!({"target": "document"})), json!({"cleared": false}));
    assert_eq!(run(&mut s, "audit.clearCache", json!({"target": "Some.ttf"})), json!({"cleared": false}));
}

#[test]
fn a_user_store_with_an_unreadable_profile_is_not_written_back() {
    use fontcraft_engine::settings::Reserved;
    let mut s = Session::with_document();
    let stored = json!({"default": "kept", "profiles": [{"id": "kept", "include": ["outlines"]}, {"id": "not an id"}]});
    s.settings_mut().set_reserved(Reserved::AuditProfiles, &stored).unwrap();
    let shown = run(&mut s, "audit.profile.show", json!({"profile": "kept"}));
    assert!(shown["problems"][0].as_str().unwrap().contains("not an id"), "{shown}");

    for (id, p) in [
        ("audit.profile.save", json!({"profile": "house", "include": ["outlines"]})),
        ("audit.profile.delete", json!({"profile": "kept"})),
        ("audit.profile.setDefault", json!({"profile": null, "target": "application"})),
    ] {
        let err = refused(&mut s, id, p);
        assert!(err.contains("could not be read in full") && err.contains("not an id"), "{id}: {err}");
    }
    let after: Option<Value> = s.settings().reserved(Reserved::AuditProfiles).unwrap();
    assert_eq!(after, Some(stored));
    // The document's own store reads cleanly, so project profiles can still be saved.
    run(&mut s, "audit.profile.save", json!({"profile": "repo", "include": ["outlines"], "store": "project"}));
}
