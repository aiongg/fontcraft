//! The command tables: their order (menus and listings show commands in it) and their coverage.

use std::collections::HashSet;
use std::path::PathBuf;

use fontcraft_engine::cmd::{COMMAND_TABLES, Install, registry};

/// Every command id in registry order, one per line. After adding or moving commands on
/// purpose, rewrite it with `FONTCRAFT_BLESS=1 cargo test -p fontcraft-engine --test engine
/// registry::`.
const EXPECTED: &str = include_str!("command_ids.txt");

#[test]
fn registry_order_matches_the_golden_list() {
    let actual: String = fontcraft_engine::command_specs().iter().map(|c| format!("{}\n", c.id)).collect();
    if std::env::var_os("FONTCRAFT_BLESS").is_some() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/engine/command_ids.txt");
        std::fs::write(path, &actual).unwrap();
        return;
    }
    let (want, got): (Vec<&str>, Vec<&str>) = (EXPECTED.lines().collect(), actual.lines().collect());
    if let Some(i) = (0..want.len().max(got.len())).find(|&i| want.get(i) != got.get(i)) {
        panic!(
            "the registry order differs from tests/engine/command_ids.txt at line {}: expected {:?}, got {:?} \
             (set FONTCRAFT_BLESS=1 to rewrite the file after a deliberate change)",
            i + 1,
            want.get(i),
            got.get(i)
        );
    }
}

#[test]
fn registry_tables_hold_every_command_once() {
    for (i, a) in COMMAND_TABLES.iter().enumerate() {
        let later = COMMAND_TABLES.get(i + 1..).unwrap_or_default();
        assert!(!later.iter().any(|b| std::ptr::fn_addr_eq(*a, *b)), "table {i} of COMMAND_TABLES is listed twice");
    }
    let ids: Vec<&str> = COMMAND_TABLES.iter().flat_map(|table| table()).map(|c| c.id).collect();
    let mut seen = HashSet::new();
    let twice: Vec<&str> = ids.iter().copied().filter(|id| !seen.insert(*id)).collect();
    assert!(twice.is_empty(), "registered more than once: {twice:?}");
    let installed: Vec<&str> = fontcraft_engine::command_specs().iter().map(|c| c.id).collect();
    assert_eq!(ids, installed);

    // Installing again: the same tables are a no-op, other tables are refused.
    fontcraft_engine::cmd::install().unwrap();
    let other = Install { commands: COMMAND_TABLES.iter().take(1).copied().collect(), ..Install::default() };
    assert!(registry::install(other).is_err());
    assert_eq!(fontcraft_engine::command_specs().len(), ids.len());
}
