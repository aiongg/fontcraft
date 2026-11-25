//! What a check cannot compute at L2 (`docs/plans/M12.md` § 3, "How L2 checks reach L3
//! services"). The engine implements [`AuditServices`] (`cmd/audit_services.rs`); every method
//! has a default that answers [`Unavailable`], and a check whose service is unavailable records
//! itself as skipped with the reason.

use std::collections::BTreeMap;

use fontcraft_geom::Point;
use fontcraft_model::{Font, Glyph, Layer};
use serde::Serialize;

use crate::Severity;
use crate::context::SourceMap;

/// Most compatibility issues one report holds.
pub const MAX_COMPAT_ISSUES: usize = 100_000;
/// Most glyphs one shaping request returns.
pub const MAX_SHAPED_GLYPHS: usize = 65_535;
/// Largest pixels-per-em a raster request may ask for.
pub const MAX_RASTER_PPEM: u16 = 4_096;
/// Most messages one run of an external tool passes on; the rest are dropped.
pub const MAX_EXTERNAL_MESSAGES: usize = 10_000;
/// Most problems one validation of a source's files reports; the rest are dropped.
pub const MAX_SOURCE_FILE_ISSUES: usize = 10_000;

/// A service that could not answer, and why (the check's skip reason).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    pub reason: String,
}

impl Unavailable {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

/// One instance of a source compiled in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledInstance {
    /// The instance name, or the variable font's family name.
    pub name: String,
    /// `ttf` or `otf`.
    pub format: &'static str,
    pub variable: bool,
    pub bytes: Vec<u8>,
    /// How the compiled font maps back to the source.
    pub map: SourceMap,
}

/// One table after a TTX dump and read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundTripTable {
    /// Tag text (`OS/2`).
    pub tag: String,
    /// The table read back has the same bytes.
    pub identical: bool,
}

/// The result of a TTX round trip of a compiled font.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoundTrip {
    pub tables: Vec<RoundTripTable>,
    /// Why the dump or the read failed, when it did.
    pub error: Option<String>,
}

/// What to shape.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeRequest {
    pub text: String,
    /// ISO 15924 script code; `None` guesses from the text.
    pub script: Option<String>,
    /// BCP 47 language tag.
    pub language: Option<String>,
    /// Feature tag → on or off.
    pub features: Vec<(String, bool)>,
    /// The location of a variable font, axis tag → user value.
    pub location: Vec<(String, f64)>,
}

/// One shaped glyph, in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// What to rasterise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterRequest {
    pub glyph_id: u32,
    /// 1 to [`MAX_RASTER_PPEM`].
    pub ppem: u16,
    /// Apply the font's hints or instructions.
    pub hinted: bool,
}

/// A rasterised glyph: one coverage byte per pixel, rows top to bottom.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Pixel offset of the bitmap's left edge from the origin.
    pub left: i32,
    /// Pixel offset of the bitmap's top edge above the baseline.
    pub top: i32,
    pub coverage: Vec<u8>,
}

/// The kind of a structural incompatibility between two masters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatKind {
    /// The glyph has no layer for one master.
    MissingGlyph,
    /// The masters have different numbers of paths.
    PathCount,
    /// A path has different numbers of nodes.
    NodeCount,
    /// A node is on-curve in one master and off-curve in the other.
    NodeType,
    /// The components differ (count, order or base glyph).
    ComponentMismatch,
    /// The anchors differ (names or count).
    AnchorMismatch,
}

/// One structural incompatibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatIssue {
    pub glyph: String,
    /// The two master ids compared: the default master first.
    pub masters: [String; 2],
    pub kind: CompatKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<usize>,
    /// The values that differ, for the message (`3 paths against 4`).
    pub detail: String,
}

/// The master-compatibility report: M9's routine behind `interpolation.checkCompatibility`, or
/// the engine's plain structural comparison until M9 lands. Both the audit and that command read
/// this one type, so they cannot disagree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub default_master: String,
    /// At most [`MAX_COMPAT_ISSUES`].
    pub issues: Vec<CompatIssue>,
    /// Issues left out by the limit.
    pub truncated: usize,
    /// Per glyph, whether every master is compatible; glyphs left out had no issue.
    pub compatible: BTreeMap<String, bool>,
}

/// An external program a check may hand a compiled font to. Each runs only when it is installed
/// and the run asks for it, and stops after 60 seconds (`docs/plans/M12.md` § 3, limits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExternalTool {
    /// A browser-style table sanitiser (`check.files.sanitiser`).
    Sanitiser,
    /// The validator the project value `validatorPath` names (`check.files.externalValidator`).
    Validator,
}

/// One message of an external tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalMessage {
    /// The tool's error is [`Severity::Fail`], its warning [`Severity::Warn`], anything milder
    /// [`Severity::Info`].
    pub severity: Severity,
    /// The tool's own message id, when it has one.
    pub id: Option<String>,
    pub message: String,
    /// The table the message is about (tag text).
    pub table: Option<String>,
    /// The glyph the message is about.
    pub glyph: Option<String>,
}

/// What is wrong with the files a source was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceFileProblem {
    /// The designspace document cannot be parsed.
    DesignspaceUnreadable,
    /// The designspace document lists no sources.
    NoSources,
    /// A source the designspace document lists cannot be opened.
    SourceUnreadable,
    /// A UFO fails structural validation: a missing or malformed property list, layer contents
    /// that do not match the files on disk, or an invalid glyph file.
    UfoStructure,
}

/// One problem found when the files a source was read from are read again, strictly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFileIssue {
    pub problem: SourceFileProblem,
    /// The file, relative to the directory of the opened source (`Example-Bold.ufo/glyphs/a.glif`).
    pub file: String,
    pub message: String,
}

/// A component attached through its anchors to a glyph before it on the layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attachment {
    /// Index in the layer's shape list.
    pub shape: usize,
    /// Where the component is.
    pub stored: Point,
    /// Where its anchor pair puts it.
    pub wanted: Point,
    /// The component is aligned horizontally only.
    pub horizontal: bool,
}

/// Where automatic alignment puts what is on one layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerAlignment {
    /// The attached components automatic alignment applies to.
    pub attachments: Vec<Attachment>,
    /// The components aligned now, in shape order.
    pub aligned: Vec<usize>,
    /// The width the aligned components give the layer, when it follows them.
    pub width: Option<f64>,
}

/// The L3 services checks may ask for. Every method answers [`Unavailable`] unless the
/// implementer provides it.
pub trait AuditServices {
    /// Compiles each exporting instance of `font` (and its variable font, when it defines one) in
    /// memory.
    fn compile_instances(&self, font: &Font) -> Result<Vec<CompiledInstance>, Unavailable> {
        let _ = font;
        Err(Unavailable::new("compiling in memory is not available here"))
    }

    /// Dumps the compiled font to TTX and reads it back.
    fn ttx_round_trip(&self, bytes: &[u8]) -> Result<RoundTrip, Unavailable> {
        let _ = bytes;
        Err(Unavailable::new("the TTX round trip is not available here"))
    }

    /// Shapes text with the compiled font (at most [`MAX_SHAPED_GLYPHS`] glyphs).
    fn shape(&self, bytes: &[u8], request: &ShapeRequest) -> Result<Vec<ShapedGlyph>, Unavailable> {
        let _ = (bytes, request);
        Err(Unavailable::new("shaping is not available here"))
    }

    /// The master-compatibility report of `font`.
    fn compatibility(&self, font: &Font) -> Result<CompatibilityReport, Unavailable> {
        let _ = font;
        Err(Unavailable::new("the master-compatibility report is not available here"))
    }

    /// Rasterises one glyph of the compiled font.
    fn rasterise(&self, bytes: &[u8], request: &RasterRequest) -> Result<Raster, Unavailable> {
        let _ = (bytes, request);
        Err(Unavailable::new("rasterising is not available here"))
    }

    /// Runs an external tool on the compiled font and returns its messages (at most
    /// [`MAX_EXTERNAL_MESSAGES`]). Unavailable when the tool is not installed or the run did not
    /// ask for it.
    fn run_external(&self, tool: ExternalTool, bytes: &[u8]) -> Result<Vec<ExternalMessage>, Unavailable> {
        let _ = bytes;
        Err(Unavailable::new(match tool {
            ExternalTool::Sanitiser => "no table sanitiser is installed or asked for",
            ExternalTool::Validator => "no external validator is installed or asked for",
        }))
    }

    /// Where automatic alignment puts the components of `layer`, a layer of `glyph`.
    fn alignment(&self, font: &Font, glyph: &Glyph, layer: &Layer) -> Result<LayerAlignment, Unavailable> {
        let _ = (font, glyph, layer);
        Err(Unavailable::new("automatic alignment is not available here"))
    }

    /// The anchors the glyph data gives `glyph` (those Set Anchors adds): names with a leading
    /// underscore on a mark that attaches to other glyphs.
    fn default_anchors(&self, font: &Font, glyph: &str) -> Result<Vec<String>, Unavailable> {
        let _ = (font, glyph);
        Err(Unavailable::new("the default anchors are not available here"))
    }

    /// The names of the anchors `glyph` has on the layer `layer`, with those it takes over from
    /// its components.
    fn effective_anchors(&self, font: &Font, glyph: &str, layer: &str) -> Result<Vec<String>, Unavailable> {
        let _ = (font, glyph, layer);
        Err(Unavailable::new("the anchors a glyph takes over from its components are not available here"))
    }

    /// Reads the files `font` was opened from again, strictly, and lists what is wrong with them
    /// (at most [`MAX_SOURCE_FILE_ISSUES`]). Unavailable when the source has no files on disk.
    fn check_source_files(&self, font: &Font) -> Result<Vec<SourceFileIssue>, Unavailable> {
        let _ = font;
        Err(Unavailable::new("the source files cannot be read again here"))
    }
}

/// No services at all: every check that needs one skips. For tests and for runs without an
/// engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoServices;

impl AuditServices for NoServices {}
