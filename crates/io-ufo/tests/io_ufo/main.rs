//! Integration tests of the UFO and designspace reader and writer.
//!
//! One test binary for the crate: each area is a module here, so the crate links
//! once. Run one module with `cargo test -p fontcraft-io-ufo --test io_ufo <module>::`.

mod corpus_designspace;
mod corpus_glif;
mod corpus_ufo;
mod norad;
