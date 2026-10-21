//! Projects: export instances linked to a font file and kept in a project file of their own. The
//! font is read from disk (it need not be open) and never written.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fails(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, returned {v}"),
        Err(e) => e.to_string(),
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-projects-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

/// Saves a one-master font with a drawn `A` and the instances Regular and Bold to
/// `dir/Family.fontcraft`, closes it, and returns its path.
fn saved_font(dir: &Path) -> PathBuf {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "font.instance.add", json!({"styleName": "Regular"}));
    run(&mut s, "font.instance.add", json!({"styleName": "Bold", "weightClass": 700}));
    let path = dir.join("Family.fontcraft");
    run(&mut s, "file.saveAs", json!({"path": path.to_string_lossy()}));
    run(&mut s, "file.close", json!({}));
    assert!(s.documents().is_empty());
    path
}

fn names(project: &Value) -> Vec<String> {
    project["instances"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
}

fn active(s: &mut Session) -> Value {
    let list = run(s, "project.list", json!({}));
    let id = list["active"].as_u64().unwrap();
    list["projects"].as_array().unwrap().iter().find(|p| p["id"].as_u64() == Some(id)).unwrap().clone()
}

#[test]
fn a_project_is_linked_edited_saved_reopened_reverted_and_exported_with_its_font_closed() {
    let dir = temp("flow");
    let font_path = saved_font(&dir);
    let font_bytes = std::fs::read(&font_path).unwrap();
    let mut s = Session::new();

    let created = run(&mut s, "file.newProject", json!({}));
    assert_eq!(created["project"]["title"], "Untitled Project");
    assert!(names(&created["project"]).is_empty());

    // Linking reads the closed font and starts the list from its instances.
    let linked = run(&mut s, "project.linkFont", json!({"font": font_path.to_string_lossy()}));
    assert_eq!(linked["copied"], 2);
    assert_eq!(names(&linked["project"]), ["Regular", "Bold"]);
    assert!(s.documents().is_empty(), "linking opens no font document");

    run(&mut s, "project.removeInstances", json!({"instances": ["Bold"]}));
    let dup = run(&mut s, "project.duplicateInstances", json!({"instances": [0]}));
    assert_eq!(dup["indexes"], json!([1]));
    let edited = run(
        &mut s,
        "project.editInstance",
        json!({"instance": 1, "changes": {"styleName": "Web", "customParameters": [{"action": "add", "name": "fileName", "value": "Web-Out"}]}}),
    );
    assert_eq!(names(&edited["project"]), ["Regular", "Web"]);
    run(&mut s, "project.addInstance", json!({"styleName": "Extra", "at": 0}));
    run(
        &mut s,
        "project.editInstance",
        json!({"instance": "Extra", "changes": {"customParameters": [{"action": "add", "name": "Export Folder", "value": "extra"}, {"action": "add", "name": "fileName", "value": "Extra-Out"}]}}),
    );
    let moved = run(&mut s, "project.moveInstances", json!({"instances": ["Extra"], "to": 2}));
    assert_eq!(names(&moved["project"]), ["Regular", "Web", "Extra"]);
    let out = dir.join("out");
    run(&mut s, "project.setExportDestination", json!({"folder": out.to_string_lossy()}));

    // Export needs a saved project.
    let err = fails(&mut s, "project.export", json!({"timestamp": 0}));
    assert!(err.contains("save"), "{err}");

    let project_path = dir.join("web.fontcraftproject");
    let saved = run(&mut s, "project.save", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(saved["project"]["title"], "web");
    assert_eq!(saved["project"]["modified"], false);
    let text = std::fs::read_to_string(&project_path).unwrap();
    assert!(text.contains("\"format\": \"fontcraft-project\""), "{text}");
    assert!(text.contains("\"font\": \"Family.fontcraft\""), "the link is stored relative: {text}");
    assert!(text.contains("\"destination\": \"out\""), "{text}");

    let exported = run(&mut s, "project.export", json!({"timestamp": 0}));
    assert_eq!(exported["written"], 3, "{exported}");
    assert!(out.join("Web-Out.ttf").is_file());
    assert!(out.join("extra").join("Extra-Out.ttf").is_file(), "the Export Folder parameter places the file");
    assert!(s.documents().is_empty(), "export opens no font document");

    // Reopen: the project reads back as saved; revert takes the font's instances again.
    run(&mut s, "project.close", json!({}));
    assert!(s.projects().is_empty());
    let opened = run(&mut s, "project.open", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(names(&opened["project"]), ["Regular", "Web", "Extra"]);
    assert_eq!(opened["project"]["fontRead"], true);
    assert_eq!(opened["project"]["font"], json!(font_path.to_string_lossy()));
    let again = run(&mut s, "project.open", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(again["alreadyOpen"], true);
    assert_eq!(s.projects().len(), 1);
    let reverted = run(&mut s, "project.revertInstances", json!({}));
    assert_eq!(names(&reverted["project"]), ["Regular", "Bold"]);
    assert_eq!(reverted["project"]["modified"], true);
    let err = fails(&mut s, "project.close", json!({}));
    assert!(err.contains("unsaved"), "{err}");
    run(&mut s, "project.close", json!({"discard": true}));

    assert_eq!(std::fs::read(&font_path).unwrap(), font_bytes, "the font file is never written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_broken_link_is_refused_at_export_and_reported_on_open() {
    let dir = temp("broken");
    let font_path = saved_font(&dir);
    let mut s = Session::new();
    run(&mut s, "file.newProject", json!({"font": font_path.to_string_lossy()}));
    run(&mut s, "project.setExportDestination", json!({"folder": dir.join("out").to_string_lossy()}));
    let project_path = dir.join("p.fontcraftproject");
    run(&mut s, "project.save", json!({"path": project_path.to_string_lossy()}));

    std::fs::rename(&font_path, dir.join("Renamed.fontcraft")).unwrap();
    let err = fails(&mut s, "project.export", json!({"timestamp": 0}));
    assert!(err.contains("choose the font again"), "{err}");
    assert!(!dir.join("out").exists(), "a failed export writes nothing");
    let err = fails(&mut s, "project.revertInstances", json!({}));
    assert!(err.contains("choose the font again"), "{err}");

    run(&mut s, "project.close", json!({}));
    let opened = run(&mut s, "project.open", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(opened["project"]["fontRead"], false);
    assert!(!opened["warnings"].as_array().unwrap().is_empty());
    assert_eq!(names(&opened["project"]), ["Regular", "Bold"], "the instances stay");

    // Choosing the font again mends the link and keeps the project's instances.
    let relinked = run(&mut s, "project.linkFont", json!({"font": dir.join("Renamed.fontcraft").to_string_lossy()}));
    assert_eq!(relinked["copied"], 0);
    assert_eq!(relinked["project"]["fontRead"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_project_command_changes_nothing() {
    let dir = temp("refused");
    let font_path = saved_font(&dir);
    let mut s = Session::new();
    let err = fails(&mut s, "project.addInstance", json!({}));
    assert!(err.contains("no project open"), "{err}");
    run(&mut s, "file.newProject", json!({"font": font_path.to_string_lossy()}));
    let before = active(&mut s);

    let refused = [
        ("project.moveInstances", json!({"instances": [0], "to": 5})),
        ("project.removeInstances", json!({"instances": ["Black"]})),
        ("project.editInstance", json!({"instance": 0, "changes": {"weightClass": 5000}})),
        ("project.editInstance", json!({"instance": 0, "changes": {"styleName": "X", "colour": 1}})),
        (
            "project.editInstance",
            json!({"instance": 0, "changes": {"styleName": "X", "customParameters": [{"action": "set", "name": "fileName", "value": "A"}]}}),
        ),
        ("project.addInstance", json!({"kind": "variableFontSetting"})),
        ("project.linkFont", json!({"font": dir.join("missing.glyphs").to_string_lossy()})),
        ("project.save", json!({"path": dir.join("p.glyphsproject").to_string_lossy()})),
        ("project.save", json!({"path": dir.join("p.txt").to_string_lossy()})),
        ("project.save", json!({})),
        ("project.duplicateInstances", json!({"instances": [], "to": 0})),
    ];
    for (id, params) in refused {
        fails(&mut s, id, params.clone());
        assert_eq!(active(&mut s), before, "{id} {params} changed the project");
    }

    // A hostile project file is refused with a message and opens nothing.
    let junk = dir.join("junk.fontcraftproject");
    std::fs::write(&junk, br#"{"format": "fontcraft-project", "instances": [{"name": "A", "location": [1e308]}]}"#).unwrap();
    fails(&mut s, "project.open", json!({"path": junk.to_string_lossy()}));
    assert_eq!(s.projects().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project linked to a compiled font in its own destination folder refuses an export that
/// would write over that font.
#[test]
fn an_export_that_would_replace_the_linked_font_is_refused() {
    let dir = temp("replace");
    let source = saved_font(&dir);
    let out = dir.join("out");
    let mut s = Session::new();
    run(&mut s, "file.newProject", json!({}));
    run(&mut s, "project.linkFont", json!({"font": source.to_string_lossy()}));
    run(&mut s, "project.removeInstances", json!({"instances": ["Bold"]}));
    let file_name = json!({"customParameters": [{"action": "add", "name": "fileName", "value": "Shared"}]});
    run(&mut s, "project.editInstance", json!({"instance": "Regular", "changes": file_name}));
    run(&mut s, "project.setExportDestination", json!({"folder": out.to_string_lossy()}));
    run(&mut s, "project.save", json!({"path": dir.join("first.fontcraftproject").to_string_lossy()}));
    run(&mut s, "project.export", json!({"timestamp": 0}));
    let compiled = out.join("Shared.ttf");
    let compiled_bytes = std::fs::read(&compiled).unwrap();
    run(&mut s, "project.close", json!({}));

    run(&mut s, "file.newProject", json!({}));
    let copied = run(&mut s, "project.linkFont", json!({"font": compiled.to_string_lossy()}))["copied"].as_u64().unwrap();
    run(&mut s, "project.addInstance", json!({"styleName": "Again"}));
    run(&mut s, "project.editInstance", json!({"instance": copied, "changes": file_name}));
    run(&mut s, "project.setExportDestination", json!({"folder": out.to_string_lossy()}));
    run(&mut s, "project.save", json!({"path": dir.join("second.fontcraftproject").to_string_lossy()}));
    let err = fails(&mut s, "project.export", json!({"timestamp": 0}));
    assert!(err.contains("replace the linked font"), "{err}");
    assert_eq!(std::fs::read(&compiled).unwrap(), compiled_bytes, "the linked font is unchanged");
    let left: Vec<_> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, [std::ffi::OsString::from("Shared.ttf")], "no staged file is left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_open_opens_a_project_file_as_a_project_and_activates_one_already_open() {
    let dir = temp("file-open");
    let font_path = saved_font(&dir);
    let mut s = Session::new();
    run(&mut s, "file.newProject", json!({}));
    run(&mut s, "project.linkFont", json!({"font": font_path.to_string_lossy()}));
    // Any case of the extension names a project file.
    let project_path = dir.join("web.FontCraftProject");
    run(&mut s, "project.save", json!({"path": project_path.to_string_lossy()}));
    run(&mut s, "project.close", json!({}));

    let opened = run(&mut s, "file.open", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(opened["alreadyOpen"], false);
    assert_eq!(names(&opened["project"]), ["Regular", "Bold"]);
    assert!(s.documents().is_empty(), "a project opens no font document");
    let again = run(&mut s, "file.open", json!({"path": project_path.to_string_lossy()}));
    assert_eq!(again["alreadyOpen"], true);
    assert_eq!(s.projects().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
