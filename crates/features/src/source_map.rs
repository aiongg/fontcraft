//! From positions in the compiled text back to the stored entries (M8.3).
//!
//! Every pass that rewrites text (token expansion, the syntax extensions, M9's variable syntax)
//! returns an [`OffsetMap`] from its output to its input. The maps of one entry compose into
//! one, from the entry's virtual file to its stored code, and [`SourceMap`] holds that map for
//! every file of an assembly, so a compiler diagnostic (file, byte span) maps to the entry and
//! the range of its stored code that caused it (`fontInfo.features.inlineErrors`,
//! `font.features.listErrors`).

use std::ops::Range;

use fontcraft_fea::{Diagnostic, Severity};

/// Which list of `Font::features` an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    Prefix,
    Class,
    Feature,
}

/// One entry of a [`fontcraft_model::Features`]: its list and its index there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryRef {
    pub kind: EntryKind,
    pub index: usize,
}

/// A byte range of an entry's stored code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRange {
    pub entry: EntryRef,
    pub range: Range<usize>,
}

/// A diagnostic mapped back to the stored code. `place` is `None` for a message about no entry
/// (the generated root file, a limit, an included file from disk); `diagnostic` keeps the
/// compiler's own file and span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryDiagnostic {
    pub place: Option<EntryRange>,
    pub diagnostic: Diagnostic,
}

impl EntryDiagnostic {
    pub fn severity(&self) -> Severity {
        self.diagnostic.severity
    }

    pub fn message(&self) -> &str {
        &self.diagnostic.message
    }
}

/// One line for the user about a list of diagnostics: the count and the first message.
pub fn summary(diagnostics: &[EntryDiagnostic]) -> String {
    match diagnostics {
        [] => "the feature code has errors".into(),
        [one] => format!("the feature code has an error: {}", one.message()),
        [first, ..] => format!("the feature code has {} errors; the first: {}", diagnostics.len(), first.message()),
    }
}

/// Offsets of a rewritten text mapped to the text it was made from. Copied stretches map byte
/// for byte; inserted text (a token's value) maps to the range it replaced. An empty map is
/// the identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OffsetMap {
    /// (output range, input range), in output order, not overlapping.
    segments: Vec<(Range<usize>, Range<usize>)>,
}

impl OffsetMap {
    /// The map of a pass that changed nothing.
    pub fn identity() -> Self {
        Self::default()
    }

    /// Records that `output` came from `input`: equal lengths for copied text; any lengths for
    /// replaced text. Calls come in output order.
    pub fn push(&mut self, output: Range<usize>, input: Range<usize>) {
        self.segments.push((output, input));
    }

    pub fn is_identity(&self) -> bool {
        self.segments.is_empty()
    }

    /// The input offset `output` came from.
    pub fn source_offset(&self, output: usize) -> usize {
        // M8.3: binary search over the segments.
        output
    }

    /// The input range `output` came from.
    pub fn source_range(&self, output: Range<usize>) -> Range<usize> {
        self.source_offset(output.start)..self.source_offset(output.end)
    }

    /// The map of `first` followed by `second` (`second` rewrote `first`'s output): from
    /// `second`'s output to `first`'s input.
    pub fn compose(first: &OffsetMap, second: &OffsetMap) -> OffsetMap {
        // M8.3
        if second.is_identity() { first.clone() } else { second.clone() }
    }
}

/// For each virtual file of an assembly that holds an entry: the entry, and the map from the
/// file's text to the entry's stored code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    files: Vec<(String, EntryRef, OffsetMap)>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that virtual file `path` holds `entry`, with `map` from the file's text to the
    /// entry's stored code (including the offset of any wrapper the assembler wrote around it).
    pub fn add(&mut self, path: String, entry: EntryRef, map: OffsetMap) {
        self.files.push((path, entry, map));
    }

    /// The entry and the range of its stored code that `span` of `file` came from; `None` for a
    /// file that holds no entry, or a span inside text the assembler wrote around the code.
    pub fn locate(&self, file: &str, span: Range<usize>) -> Option<EntryRange> {
        // M8.3: spans inside the wrapper map to `None`.
        let (_, entry, map) = self.files.iter().find(|(p, _, _)| p == file)?;
        Some(EntryRange { entry: *entry, range: map.source_range(span) })
    }

    /// [`SourceMap::locate`] for a diagnostic.
    pub fn map(&self, diagnostic: Diagnostic) -> EntryDiagnostic {
        let place = self.locate(&diagnostic.file, diagnostic.span.clone());
        EntryDiagnostic { place, diagnostic }
    }

    /// The virtual file that holds `entry`.
    pub fn path_of(&self, entry: EntryRef) -> Option<&str> {
        self.files.iter().find(|(_, e, _)| *e == entry).map(|(p, _, _)| p.as_str())
    }
}
