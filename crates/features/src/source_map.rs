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

    /// The input offset `output` came from. A position inside replaced text maps to the start
    /// of what it replaced; a position no segment covers maps to itself.
    pub fn source_offset(&self, output: usize) -> usize {
        self.map_start(output).unwrap_or(output)
    }

    /// The input range `output` came from: a range ending inside replaced text ends where the
    /// replaced text ends.
    pub fn source_range(&self, output: Range<usize>) -> Range<usize> {
        self.try_source_range(output.clone()).unwrap_or(output)
    }

    /// [`OffsetMap::source_range`], or `None` when the start of `output` is in text no segment
    /// covers (text a pass or the assembler wrote around the code).
    pub fn try_source_range(&self, output: Range<usize>) -> Option<Range<usize>> {
        let start = self.map_start(output.start)?;
        let end = self.map_end(output.end.max(output.start)).unwrap_or(start).max(start);
        Some(start..end)
    }

    /// The input offset a range starting at `output` starts at.
    fn map_start(&self, output: usize) -> Option<usize> {
        if self.is_identity() {
            return Some(output);
        }
        let i = self.segments.partition_point(|(out, _)| out.end <= output);
        if let Some((out, input)) = self.segments.get(i)
            && out.start <= output
        {
            return Some(if is_copy(out, input) { input.start.saturating_add(output - out.start) } else { input.start });
        }
        // At the very end of a segment that nothing follows.
        let (out, input) = self.segments.get(i.checked_sub(1)?)?;
        (out.end == output).then_some(input.end)
    }

    /// The input offset a range ending at `output` ends at.
    fn map_end(&self, output: usize) -> Option<usize> {
        if self.is_identity() {
            return Some(output);
        }
        let i = self.segments.partition_point(|(out, _)| out.end < output);
        let (out, input) = self.segments.get(i)?;
        if out.start < output {
            Some(if is_copy(out, input) { input.start.saturating_add(output - out.start) } else { input.end })
        } else if out.start == output {
            Some(input.start)
        } else {
            None
        }
    }

    /// The map of `first` followed by `second` (`second` rewrote `first`'s output): from
    /// `second`'s output to `first`'s input.
    pub fn compose(first: &OffsetMap, second: &OffsetMap) -> OffsetMap {
        if second.is_identity() {
            return first.clone();
        }
        if first.is_identity() {
            return second.clone();
        }
        let mut out = OffsetMap::default();
        for (out2, mid) in &second.segments {
            if !is_copy(out2, mid) {
                // Replaced text maps to everything its source came from.
                if let Some(start) = first.map_start(mid.start) {
                    let end = first.map_end(mid.end).unwrap_or(start).max(start);
                    out.segments.push((out2.clone(), start..end));
                }
                continue;
            }
            // Copied text follows `first`'s segments piece by piece.
            let from = first.segments.partition_point(|(o, _)| o.end <= mid.start);
            for (out1, in1) in first.segments.iter().skip(from) {
                if out1.start >= mid.end {
                    break;
                }
                let a = out1.start.max(mid.start);
                let b = out1.end.min(mid.end);
                if a >= b {
                    continue;
                }
                let piece = out2.start + (a - mid.start)..out2.start + (b - mid.start);
                let source = if is_copy(out1, in1) { in1.start + (a - out1.start)..in1.start + (b - out1.start) } else { in1.clone() };
                out.segments.push((piece, source));
            }
        }
        // A second map whose every segment fell in unmapped text maps nothing; keep that from
        // reading as the identity.
        if out.segments.is_empty() {
            out.segments.push((0..0, 0..0));
        }
        out
    }
}

/// Copied text: the same length in and out.
fn is_copy(output: &Range<usize>, input: &Range<usize>) -> bool {
    output.len() == input.len()
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
        let (_, entry, map) = self.files.iter().find(|(p, _, _)| p == file)?;
        Some(EntryRange { entry: *entry, range: map.try_source_range(span)? })
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
