//! The URL scheme: `fontcraft://show/?path=…&glyph=…&production=…&layer=…` (M13.13 fills it). Rows:
//! `url.open`, `url.show`, `url.show.*`, `url.safety`. Only the `show` path is accepted, so a web
//! page cannot run commands; the result lists the tabs the shell adds.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.13.
    Vec::new()
}
