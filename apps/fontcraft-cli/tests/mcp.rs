//! `fontcraft-cli mcp --automation-read-root DIR --automation-write-root DIR`: the roots are
//! checked before the server starts, and the server confines what its commands read and write.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};

fn mcp(args: &[&str], input: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli"))
        .arg("mcp")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn roots_are_checked_at_start_and_confine_the_server() {
    let base = std::env::temp_dir().join(format!("fontcraft-cli-mcp-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("in")).unwrap();
    std::fs::create_dir_all(base.join("out")).unwrap();
    let dir = |d: &str| base.join(d).to_str().unwrap().to_string();

    let (code, _, err) = mcp(&["--automation-read-root", &dir("missing")], "");
    assert_ne!(code, 0);
    assert!(err.contains("cannot open the automation read root"), "{err}");
    let (code, _, err) = mcp(&["--automation-read-root", &dir("in"), "--automation-read-root", &dir("out")], "");
    assert_eq!(code, 2);
    assert!(err.contains("given twice"), "{err}");
    let (code, _, err) = mcp(&["--connect", "127.0.0.1:1", "--automation-write-root", &dir("out")], "");
    assert_eq!(code, 2);
    assert!(err.contains("a running app confines itself"), "{err}");

    let save = |path: String| {
        let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "run_command", "arguments": {"command": "file.saveAs", "params": {"path": path}}}});
        format!("{call}\n")
    };
    let write_root = format!("--automation-write-root={}", dir("out"));
    let (code, out, err) = mcp(&["--automation-read-root", &dir("in"), &write_root], &save(format!("{}/x.fontcraft", dir("in"))));
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("files are confined"), "{err}");
    assert!(out.contains("outside the write root") && out.contains("\"isError\":true"), "{out}");
    let (code, out, _) = mcp(&["--automation-read-root", &dir("in"), &write_root], &save(format!("{}/x.fontcraft", dir("out"))));
    assert_eq!(code, 0);
    assert!(out.contains("\"isError\":false") && base.join("out/x.fontcraft").exists(), "{out}");
    let _ = std::fs::remove_dir_all(&base);
}
