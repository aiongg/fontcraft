#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use super::*;

/// A fresh folder holding `in` (read root), `out` (write root), `outside` and `in2`.
fn folders(name: &str) -> (PathBuf, Arc<AutomationRoots>) {
    let base = std::env::temp_dir().join(format!("fontcraft-file-access-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in/sub", "out", "outside", "in2"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    std::fs::write(base.join("in/a.glyphs"), "{}").unwrap();
    std::fs::write(base.join("outside/secret.glyphs"), "{}").unwrap();
    let roots = AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap().unwrap();
    (base, roots)
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn paths_inside_the_roots_pass_and_others_dont() {
    let (base, roots) = folders("inside");
    assert_eq!(roots.check(&s(base.join("in/a.glyphs")), Access::Read), Ok(()));
    assert_eq!(roots.check(&s(base.join("in")), Access::Read), Ok(()), "the root itself");
    assert_eq!(roots.check(&s(base.join("in/sub/new.glyphs")), Access::Read), Ok(()), "a missing file is checked by its folder");
    assert_eq!(roots.check(&s(base.join("out/new/deeper/x.ttf")), Access::Write), Ok(()), "and by its nearest folder");
    let e = roots.check(&s(base.join("outside/secret.glyphs")), Access::Read).unwrap_err();
    assert!(e.starts_with("automation path rejected: outside the read root"), "{e}");
    // Read and write access are separate.
    let e = roots.check(&s(base.join("in/a.glyphs")), Access::Write).unwrap_err();
    assert!(e.contains("outside the write root"), "{e}");
    assert!(roots.check(&s(base.join("out/x.ttf")), Access::Read).unwrap_err().contains("outside the read root"));
    // A sibling folder whose name starts with the root's is outside it.
    std::fs::write(base.join("in2/b.glyphs"), "{}").unwrap();
    assert!(roots.check(&s(base.join("in2/b.glyphs")), Access::Read).is_err(), "in2 is not inside in");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn parent_traversal_relative_paths_and_bad_input() {
    let (base, roots) = folders("dotdot");
    let sep = std::path::MAIN_SEPARATOR;
    let up = format!("{}{sep}..{sep}outside{sep}secret.glyphs", s(base.join("in")));
    assert!(roots.check(&up, Access::Read).unwrap_err().contains("outside the read root"), "`..` out of the root");
    let back = format!("{}{sep}..{sep}a.glyphs", s(base.join("in/sub")));
    assert_eq!(roots.check(&back, Access::Read), Ok(()), "`..` that stays inside");
    let missing = format!("{}{sep}nope{sep}..{sep}..{sep}outside{sep}x.ttf", s(base.join("out")));
    let e = roots.check(&missing, Access::Write).unwrap_err();
    // Windows resolves `..` by name first (so does opening the file there): then it's outside.
    assert!(e.contains("parent traversal below a folder that doesn't exist") || e.contains("outside the write root"), "{e}");
    // The tests' working directory is the crate's folder: not inside the roots.
    assert!(roots.check("a.glyphs", Access::Read).unwrap_err().contains("outside the read root"));
    assert!(roots.check("", Access::Read).unwrap_err().contains("path is empty"));
    assert!(roots.check("a\0b", Access::Read).unwrap_err().contains("NUL"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_missing_root_is_an_error_and_none_is_no_confinement() {
    let (base, _) = folders("missing");
    let e = AutomationRoots::new(Some(&base.join("nope")), None).unwrap_err();
    assert!(e.starts_with("cannot open the automation read root"), "{e}");
    let e = AutomationRoots::new(None, Some(&base.join("in/a.glyphs"))).unwrap_err();
    assert!(e.starts_with("cannot open the automation write root"), "a file is not a root: {e}");
    assert!(AutomationRoots::new(None, None).unwrap().is_none());
    // One root alone grants only its own access.
    let read_only = AutomationRoots::new(Some(&base.join("in")), None).unwrap().unwrap();
    let e = read_only.check(&s(base.join("in/x.ttf")), Access::Write).unwrap_err();
    assert_eq!(e, format!("automation filesystem access is not granted: write authority is absent: {}", s(base.join("in/x.ttf"))));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn links_are_followed_before_comparing() {
    let (base, roots) = folders("links");
    #[cfg(unix)]
    let made = |target: &Path, link: &Path| std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    let made = |target: &Path, link: &Path| {
        let made = if target.is_dir() { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) };
        made.is_ok()
    };
    #[cfg(not(any(unix, windows)))]
    let made = |_: &Path, _: &Path| false;
    // Windows needs Developer Mode or elevation to make links: nothing to test without them.
    if !made(&base.join("outside/secret.glyphs"), &base.join("in/leak.glyphs")) {
        eprintln!("skipped: symbolic links can't be made here");
        let _ = std::fs::remove_dir_all(base);
        return;
    }
    assert!(roots.check(&s(base.join("in/leak.glyphs")), Access::Read).unwrap_err().contains("outside the read root"), "a file link out");
    assert!(made(&base.join("outside"), &base.join("out/away")));
    let e = roots.check(&s(base.join("out/away/new.ttf")), Access::Write).unwrap_err();
    assert!(e.contains("outside the write root"), "a folder link out: {e}");
    assert!(made(&base.join("outside/gone.ttf"), &base.join("out/dangling.ttf")));
    let e = roots.check(&s(base.join("out/dangling.ttf")), Access::Write).unwrap_err();
    assert!(e.contains("a link that leads nowhere"), "writing would create the file outside: {e}");
    // A link to a folder inside the root stays inside.
    assert!(made(&base.join("in/sub"), &base.join("in/alias")));
    assert_eq!(roots.check(&s(base.join("in/alias/x.glyphs")), Access::Read), Ok(()));
    // A root given through a link is the folder it leads to.
    assert!(made(&base.join("in"), &base.join("in-link")));
    let linked = AutomationRoots::new(Some(&base.join("in-link")), None).unwrap().unwrap();
    assert_eq!(linked.check(&s(base.join("in/a.glyphs")), Access::Read), Ok(()));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(windows)]
#[test]
fn windows_ignores_case_and_refuses_devices() {
    let (base, roots) = folders("case");
    let upper = s(base.join("IN/A.GLYPHS"));
    assert_eq!(roots.check(&upper, Access::Read), Ok(()), "{upper}");
    let e = roots.check(&s(base.join("out/NUL")), Access::Write).unwrap_err();
    assert!(e.contains("reserved device names"), "{e}");
    assert_eq!(roots.check(&s(base.join("out/console.txt")), Access::Write), Ok(()));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn scopes_nest_and_are_restored() {
    let (base, roots) = folders("scopes");
    let outside = base.join("outside/secret.glyphs");
    assert_eq!(check_read(&outside), Ok(()), "nothing is confined by default");
    confine(Some(&roots), || {
        assert!(check_read(&outside).is_err());
        let o = outside.clone();
        let there = std::thread::spawn(move || check_read(&o).is_ok()).join().unwrap();
        assert!(there, "another thread is not confined");
    });
    assert_eq!(check_read(&outside), Ok(()), "restored after confine");
    let r = std::panic::catch_unwind(|| confine(Some(&roots), || panic!("boom")));
    assert!(r.is_err());
    assert_eq!(check_read(&outside), Ok(()), "restored after a panic");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn device_names_and_verbatim_prefixes() {
    for d in ["CON", "nul", "Com1", "LPT9.txt", "aux.glyphs", "NUL "] {
        assert!(is_device_name(d), "{d}");
    }
    for f in ["console", "com10", "COM0", "lpt", "a.glyphs", "nul-x"] {
        assert!(!is_device_name(f), "{f}");
    }
    assert_eq!(plain(Path::new(r"\\?\C:\work")), PathBuf::from(r"C:\work"));
    assert_eq!(plain(Path::new(r"\\?\UNC\server\share\x")), PathBuf::from(r"\\server\share\x"));
}
