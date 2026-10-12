//! Integration tests of the Glyphs reader and writer.
//!
//! One test binary for the crate: each area is a module here, so the crate links
//! once. Run one module with `cargo test -p fontcraft-io-glyphs --test io_glyphs <module>::`.

mod corpus;
mod kerning_lookup;
mod metrics_keys;
mod timing;
