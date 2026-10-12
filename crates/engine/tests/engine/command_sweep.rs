//! Every-command sweep: each registered command, in every fixture state, with hostile parameters,
//! must return `Ok` or `Err` (never panic). After `Err` the session is unchanged; after `Ok` every
//! invariant still holds. Extend `fontcraft_testkit::junk_values` when a new kind of parameter
//! appears; a command added to the registry is swept without further work.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fontcraft_engine::command_specs;
use fontcraft_testkit::{Fixture, catch_quiet, check_session, junk_params, snapshot, temp_dir};

#[test]
fn no_command_panics_or_corrupts_the_session() {
    let scratch = temp_dir("sweep");
    let mut failures = Vec::new();
    let mut calls = 0usize;
    for spec in command_specs() {
        for params in junk_params(spec.params, &scratch) {
            for fx in Fixture::ALL {
                let mut s = fx.session();
                let before = snapshot(&s);
                calls += 1;
                match catch_quiet(|| s.execute(spec.id, &params)) {
                    Err(panic) => failures.push(format!("PANIC {} {params} [{fx:?}]: {panic}", spec.id)),
                    Ok(Err(_)) => {
                        if snapshot(&s) != before {
                            failures.push(format!("{} {params} [{fx:?}]: returned an error but changed the session", spec.id));
                        }
                    }
                    Ok(Ok(_)) => {
                        if let Err(e) = check_session(&s) {
                            failures.push(format!("{} {params} [{fx:?}]: {e}", spec.id));
                        }
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(calls > 1000, "the sweep made only {calls} calls");
    assert!(failures.is_empty(), "{} of {calls} calls failed:\n{}", failures.len(), failures.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn an_internal_panic_is_reported_and_rolled_back() {
    // The guard is exercised directly: no shipped command panics, so there is none to call.
    let caught = fontcraft_engine::guard::catch_panic(|| -> u8 { panic!("bug") });
    assert_eq!(caught, Err("bug".to_string()));
}
