use super::*;

#[test]
fn limits_table_and_an_empty_registry() {
    assert!(LIMITS_TABLE.iter().any(|(name, v)| *name == "maxModuleBytes" && *v == error::MAX_MODULE_BYTES as u64));
    assert_eq!(LIMITS_TABLE.len(), 19);
    let r = Registry::new();
    assert!(r.is_empty() && r.list().is_empty() && r.enabled_of_kind(Kind::Filter).next().is_none());
}
