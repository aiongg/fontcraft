//! Dependency layering rules (docs/architecture.md › Layers).
//!
//! A crate may depend only on crates in lower layers (or earlier in an explicit same-layer chain).
//! Nothing below the UI layer may use a UI toolkit. The test kit is a dev-dependency only. The rule
//! engine works on a small model so it can be unit-tested; [`from_metadata`] builds that model.

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Layer(u8),
    /// Layer 0, and it may depend on no workspace crate at all (publishable on its own).
    Standalone,
    /// Test tooling: may depend on anything up to the engine; others use it as a dev-dependency only.
    Testkit,
    /// Binaries and build tooling: exempt.
    Exempt,
}

/// The layering table. Names are package names without the `fontcraft-` prefix. **Register every
/// new crate here** (append; `cargo xtask layers` fails on an unknown crate). Planned crates and
/// their layers are listed in docs/architecture.md.
pub const TABLE: &[(&str, Class)] = &[
    ("geom", Class::Layer(0)),
    ("plist", Class::Standalone),
    ("cff", Class::Standalone),
    ("container", Class::Standalone),
    ("glyphdata", Class::Standalone),
    ("ttasm", Class::Standalone),
    ("imagery", Class::Standalone),
    ("model", Class::Layer(1)),
    ("autohint", Class::Layer(1)),
    ("pathops", Class::Layer(2)),
    ("audit", Class::Layer(2)),
    ("glyphquery", Class::Layer(2)),
    ("spacing", Class::Layer(2)),
    ("textlayout", Class::Layer(2)),
    ("features", Class::Layer(2)),
    ("components", Class::Layer(2)),
    ("fontinfo", Class::Layer(2)),
    ("fea", Class::Standalone),
    ("shaping", Class::Layer(2)),
    ("interp", Class::Layer(2)),
    ("hinting", Class::Layer(2)),
    ("color", Class::Layer(2)),
    ("plugins", Class::Layer(2)),
    ("format", Class::Layer(3)),
    ("compile", Class::Layer(3)),
    ("io-glyphs", Class::Layer(3)),
    ("io-ufo", Class::Layer(3)),
    ("io-svg", Class::Layer(3)),
    ("io-sfd", Class::Layer(3)),
    ("io-pdf", Class::Layer(3)),
    ("io-bitmap", Class::Layer(3)),
    ("decompile", Class::Layer(3)),
    ("oracle", Class::Layer(3)),
    ("render", Class::Layer(3)),
    ("tables", Class::Layer(3)),
    ("proof", Class::Layer(3)),
    ("tools", Class::Layer(4)),
    ("engine", Class::Layer(5)),
    ("ui-egui", Class::Layer(6)),
    ("mcp", Class::Layer(6)),
    ("testkit", Class::Testkit),
    // L7: apps and tooling
    ("fontcraft", Class::Exempt),
    ("cli", Class::Exempt),
    ("web", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Explicit orderings within a layer: a crate may use the ones before it in its chain.
/// `textlayout` and `features` read kerning through `spacing`; `audit` runs the metrics-key
/// scan of `spacing`; `textlayout` shapes through
/// `shaping`; `features` evaluates glyph predicates through `glyphquery` and overlays condition
/// regions through `interp`; `hinting` reads TT zone
/// and stem filters through `glyphquery`. `tables` uses the glyph encoders of `compile` and the
/// decoders of `decompile`; `proof` draws through `render` and reads `tables`; `oracle` compares
/// layout tables through the decompiler of `tables`.
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[
    &["spacing", "textlayout"],
    &["spacing", "features"],
    &["spacing", "audit"],
    &["shaping", "textlayout"],
    &["glyphquery", "features"],
    &["interp", "features"],
    &["glyphquery", "hinting"],
    &["compile", "tables", "proof"],
    &["decompile", "tables", "proof"],
    &["tables", "oracle"],
    &["render", "proof"],
];

/// External crates that are a UI toolkit, windowing or GPU dependency. A trailing `*` is a prefix.
pub const UI_CRATES: &[&str] =
    &["egui*", "eframe", "epaint*", "emath", "ecolor", "winit", "rfd", "wgpu*", "glow", "glutin*", "accesskit*", "arboard"];
/// First layer allowed to use UI crates.
pub const UI_MIN_LAYER: u8 = 6;
/// The layer the test kit counts as when checking what it depends on.
const TESTKIT_LAYER: u8 = 6;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("fontcraft-").unwrap_or(pkg)
}

pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => "L0 alone".into(),
        Some(Class::Testkit) => "testkit".into(),
        Some(Class::Exempt) => "L7 exempt".into(),
        None => "UNKNOWN".into(),
    }
}

fn layer(class: Class) -> Option<u8> {
    match class {
        Class::Layer(l) => Some(l),
        Class::Standalone => Some(0),
        Class::Testkit => Some(TESTKIT_LAYER),
        Class::Exempt => None,
    }
}

fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER
        .iter()
        .any(|chain| matches!((chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)), (Some(f), Some(t)) if t < f))
}

fn is_ui_crate(name: &str) -> bool {
    UI_CRATES.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

impl DepKind {
    pub fn suffix(self) -> &'static str {
        match self {
            DepKind::Normal => "",
            DepKind::Dev => " (dev)",
            DepKind::Build => " (build)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    UiBelowUiLayer { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE (docs/architecture.md › Layers)")
            }
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) depends on {dep} (L{to}){}: a crate may only use lower layers", kind.suffix())
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => write!(f, "{krate} is a standalone crate but depends on {dep}"),
            Violation::TestkitAsNormalDep { krate } => write!(f, "{krate} uses the test kit as a normal dependency; it must be a dev-dependency"),
            Violation::UiBelowUiLayer { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on the UI crate {dep}; nothing below L{UI_MIN_LAYER} may")
            }
        }
    }
}

pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        let Some(from) = layer(class) else { continue };
        for d in &c.deps {
            if !d.workspace {
                // The test kit reaches every lower crate's tests, so it may not bring a UI toolkit.
                let below_ui = from < UI_MIN_LAYER || class == Class::Testkit;
                if d.kind != DepKind::Dev && below_ui && is_ui_crate(&d.name) {
                    out.push(Violation::UiBelowUiLayer { krate: c.name.clone(), dep: d.name.clone(), layer: from });
                }
                continue;
            }
            let Some(dep_class) = classify(&d.name) else { continue };
            if dep_class == Class::Testkit {
                if d.kind != DepKind::Dev {
                    out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() });
                }
                continue;
            }
            if class == Class::Standalone && d.kind != DepKind::Dev {
                out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                continue;
            }
            // Tests may reach across (a format crate's tests may compile a font); shipped code may not.
            if d.kind == DepKind::Dev {
                continue;
            }
            let Some(to) = layer(dep_class) else {
                out.push(Violation::Upward { krate: c.name.clone(), dep: d.name.clone(), from, to: 7, kind: d.kind });
                continue;
            };
            let allowed = to < from
                || (to == from && class != Class::Testkit && intra_layer_allowed(&c.name, &d.name))
                || (class == Class::Testkit && to < TESTKIT_LAYER);
            if !allowed {
                out.push(Violation::Upward { krate: c.name.clone(), dep: d.name.clone(), from, to, kind: d.kind });
            }
        }
    }
    out
}

pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let packages = meta.get("packages").and_then(Value::as_array).ok_or("cargo metadata: no packages")?;
    let names: Vec<&str> = packages.iter().filter_map(|p| p.get("name").and_then(Value::as_str)).collect();
    let mut crates = Vec::new();
    for p in packages {
        let name = p.get("name").and_then(Value::as_str).ok_or("cargo metadata: a package without a name")?;
        let deps = p
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|deps| {
                deps.iter()
                    .filter_map(|d| {
                        let dep_name = d.get("name").and_then(Value::as_str)?;
                        let kind = match d.get("kind").and_then(Value::as_str) {
                            Some("dev") => DepKind::Dev,
                            Some("build") => DepKind::Build,
                            _ => DepKind::Normal,
                        };
                        Some(Dep { name: dep_name.to_string(), kind, workspace: names.contains(&dep_name) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        crates.push(Crate { name: name.to_string(), deps });
    }
    crates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(crates)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn downward_dependencies_pass() {
        let crates = [
            krate("fontcraft-geom", &[("kurbo", DepKind::Normal, false)]),
            krate("fontcraft-model", &[("fontcraft-geom", DepKind::Normal, true)]),
            krate("fontcraft-engine", &[("fontcraft-model", DepKind::Normal, true), ("fontcraft-testkit", DepKind::Dev, true)]),
            krate("fontcraft-testkit", &[("fontcraft-engine", DepKind::Normal, true)]),
            krate("fontcraft-ui-egui", &[("fontcraft-engine", DepKind::Normal, true), ("egui", DepKind::Normal, false)]),
            krate("fontcraft", &[("fontcraft-ui-egui", DepKind::Normal, true), ("eframe", DepKind::Normal, false)]),
        ];
        assert_eq!(check(&crates), vec![]);
    }

    #[test]
    fn violations_are_found() {
        let crates = [
            krate("fontcraft-model", &[("fontcraft-engine", DepKind::Normal, true)]),
            krate("fontcraft-compile", &[("fontcraft-format", DepKind::Normal, true)]),
            krate("fontcraft-engine", &[("egui", DepKind::Normal, false), ("fontcraft-testkit", DepKind::Normal, true)]),
            krate("fontcraft-new", &[]),
            krate("fontcraft-mcp", &[("fontcraft-ui-egui", DepKind::Normal, true)]),
            krate("fontcraft-format", &[("fontcraft-cli", DepKind::Normal, true)]),
            krate(
                "fontcraft-compile",
                &[("egui-winit", DepKind::Normal, false), ("epaint", DepKind::Build, false), ("accesskit_winit", DepKind::Normal, false)],
            ),
            krate("fontcraft-testkit", &[("egui", DepKind::Normal, false)]),
        ];
        let v = check(&crates);
        assert!(v.contains(&Violation::Upward {
            krate: "fontcraft-model".into(),
            dep: "fontcraft-engine".into(),
            from: 1,
            to: 5,
            kind: DepKind::Normal
        }));
        assert!(v.contains(&Violation::Upward {
            krate: "fontcraft-compile".into(),
            dep: "fontcraft-format".into(),
            from: 3,
            to: 3,
            kind: DepKind::Normal
        }));
        assert!(v.contains(&Violation::UiBelowUiLayer { krate: "fontcraft-engine".into(), dep: "egui".into(), layer: 5 }));
        assert!(v.contains(&Violation::TestkitAsNormalDep { krate: "fontcraft-engine".into() }));
        assert!(v.contains(&Violation::Unregistered { krate: "fontcraft-new".into() }));
        assert!(v.contains(&Violation::Upward {
            krate: "fontcraft-mcp".into(),
            dep: "fontcraft-ui-egui".into(),
            from: 6,
            to: 6,
            kind: DepKind::Normal
        }));
        assert!(v.iter().any(|x| matches!(x, Violation::Upward { krate, to: 7, .. } if krate == "fontcraft-format")));
        for dep in ["egui-winit", "epaint", "accesskit_winit"] {
            assert!(v.contains(&Violation::UiBelowUiLayer { krate: "fontcraft-compile".into(), dep: dep.into(), layer: 3 }), "{dep}");
        }
        assert!(v.iter().any(|x| matches!(x, Violation::UiBelowUiLayer { krate, .. } if krate == "fontcraft-testkit")));
        assert_eq!(v.len(), 11);
    }
}
