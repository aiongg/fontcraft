//! The caps one compile works under (`docs/plans/M8.md` § 3, Limits). fea-rs can panic on some
//! invalid input and allocate without bound on some valid input, so every count the work grows
//! with is checked before fea-rs sees the text.

/// Most glyphs a font holds: glyph ids are 16-bit.
pub const MAX_GLYPHS: usize = 65_535;
/// Most bytes of a file path in [`crate::VirtualFiles`] or an include statement.
pub const MAX_PATH_BYTES: usize = 1_024;
/// Most files one compile reads, the root file and included files from disk counted.
pub const MAX_INCLUDE_FILES: usize = 4_096;
/// Deepest chain of include statements, as the feature-file specification allows.
pub const MAX_INCLUDE_DEPTH: usize = 50;
/// Deepest nesting of braces and brackets, as the feature splitter of the UFO reader allows.
pub const MAX_NESTING_DEPTH: usize = 64;
/// Most diagnostics one compile keeps (fea-rs's `max_error_messages`).
pub const MAX_DIAGNOSTICS: usize = 1_000;
/// The stack of the thread fea-rs runs on, on native targets: its parser and compiler recurse.
pub const STACK_SIZE: usize = 64 << 20;
/// Most lookups in [`crate::GeneratedLookups`].
pub const MAX_GENERATED_LOOKUPS: usize = 65_535;
/// Most rules of all generated lookups together: glyph pairs, class pairs, marks, bases,
/// ligatures, cursive glyphs and feature-variation substitutions, each counted once.
pub const MAX_GENERATED_RULES: usize = 2_000_000;
/// Most feature variations (condition sets) in [`crate::GeneratedLookups`].
pub const MAX_FEATURE_VARIATIONS: usize = 65_535;
/// Most deltas of one [`crate::Metric`], and most axes in one [`crate::Region`].
pub const MAX_DELTAS: usize = 4_096;
/// Most axes [`crate::FeaVariations`] may report.
pub const MAX_AXES: usize = 256;
/// Most master locations [`crate::FeaVariations`] may report, and most per-location values
/// handed to [`crate::FeaVariations::resolve`].
pub const MAX_LOCATIONS: usize = 4_096;

/// The caps of one compile. [`Limits::default`] is [`Limits::NATIVE`] on native targets and
/// [`Limits::WASM`] in the browser, where a panic aborts and memory is smaller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most bytes of all files together, included files counted.
    pub text_bytes: usize,
    /// Most files read, the root counted.
    pub include_files: usize,
    /// Deepest include chain.
    pub include_depth: usize,
    /// Deepest brace and bracket nesting.
    pub nesting_depth: usize,
    /// Most glyphs all glyph classes expand to together, counted with saturation over the
    /// parse tree before fea-rs expands them.
    pub class_members: usize,
    /// Most glyphs in one ligature's component sequence.
    pub ligature_components: usize,
    /// Most sequences one ligature or contextual rule expands to (the product of its class
    /// sizes).
    pub rule_sequences: usize,
    /// Most rules after expansion, all lookups together.
    pub rules: usize,
    /// Most diagnostics kept.
    pub diagnostics: usize,
}

impl Limits {
    pub const NATIVE: Limits = Limits {
        text_bytes: 16 << 20,
        include_files: MAX_INCLUDE_FILES,
        include_depth: MAX_INCLUDE_DEPTH,
        nesting_depth: MAX_NESTING_DEPTH,
        class_members: 4_000_000,
        ligature_components: 64,
        rule_sequences: 100_000,
        rules: 2_000_000,
        diagnostics: MAX_DIAGNOSTICS,
    };

    pub const WASM: Limits = Limits { text_bytes: 4 << 20, class_members: 500_000, rules: 200_000, ..Limits::NATIVE };
}

impl Default for Limits {
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") { Limits::WASM } else { Limits::NATIVE }
    }
}
