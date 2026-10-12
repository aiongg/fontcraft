//! Integration tests of the property-list reader and writer.
//!
//! One test binary for the crate: each area is a module here, so the crate links
//! once. Run one module with `cargo test -p fontcraft-plist --test plist <module>::`.

mod corpus;
mod corpus_key_order;
mod corpus_key_paths;
mod corpus_keys;
mod corpus_spellings;
mod corpus_xml;
