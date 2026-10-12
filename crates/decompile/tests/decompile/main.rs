//! Integration tests of the font decompilers.
//!
//! One test binary for the crate: each area is a module here, so the crate links
//! once. Run one module with `cargo test -p fontcraft-decompile --test decompile <module>::`.

mod corpus;
mod gpos_kern;
mod ps_fonts;
