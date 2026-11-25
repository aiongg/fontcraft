//! The switches of `fontcraft_format::output`, in a process of their own.
#![allow(clippy::unwrap_used)]

use fontcraft_format::output::*;
use fontcraft_format::write_atomic;

/// The switches are the process's, so one test goes through all of them.
#[test]
fn a_dry_run_records_and_a_held_place_keeps() {
    let dir = std::env::temp_dir().join(format!("fontcraft-output-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.ttf");
    std::fs::write(&old, b"old").unwrap();

    let place = dir.join("held");
    hold(&place);
    write_atomic(&place.join("a"), b"one").unwrap();
    assert_eq!(held(&place.join("a")).as_deref(), Some(b"one".as_slice()));
    assert!(!place.exists());
    release(&place);
    assert!(held(&place.join("a")).is_none());

    set_no_clobber(true);
    assert!(write_atomic(&old, b"new").unwrap_err().to_string().contains("exists"));
    let mine = dir.join("mine.ttf");
    write_atomic(&mine, b"1").unwrap();
    write_atomic(&mine, b"2").unwrap();
    set_no_clobber(false);

    set_dry_run(true);
    let new = dir.join("sub").join("new.ttf");
    assert!(write_atomic(&new, b"x").is_err(), "the folder is missing");
    create_dir_all(&dir.join("sub")).unwrap();
    write_atomic(&new, b"xy").unwrap();
    write_atomic(&old, b"new").unwrap();
    remove_file(&mine).unwrap();
    assert!(remove_dir(&dir.join("sub")).is_err(), "a file would be in it");
    let (plan, more) = planned();
    let names: Vec<(Action, String, Option<usize>)> =
        plan.iter().map(|p| (p.action, p.path.file_name().unwrap().to_string_lossy().into_owned(), p.bytes)).collect();
    assert_eq!(
        names,
        [
            (Action::CreateFolder, "sub".to_string(), None),
            (Action::Write, "new.ttf".to_string(), Some(2)),
            (Action::Write, "old.ttf".to_string(), Some(3)),
            (Action::Remove, "mine.ttf".to_string(), None),
        ]
    );
    assert_eq!(more, 0);
    remove_file(&new).unwrap();
    remove_dir(&dir.join("sub")).unwrap();
    assert_eq!(planned().0.len(), 2, "what was taken back is not listed");
    set_dry_run(false);
    assert_eq!(std::fs::read(&old).unwrap(), b"old");
    assert!(mine.exists() && !dir.join("sub").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
