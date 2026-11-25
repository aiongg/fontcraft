//! Reading the file within the limits, and finding the first page.
//!
//! The parser decodes cross-reference and object streams while it loads the file. Before it
//! runs, the number of streams sets the bound on each of those (the file's total budget shared
//! out, between [`MIN_PDF_STRUCTURE_STREAM_BYTES`] and [`MAX_PDF_STREAM_BYTES`]). Object
//! streams are decoded earlier still, by [`admit`], which the parser calls for each object
//! before it decodes one: there each gets the same allowance as a content stream and is charged
//! to the file's total, and the first one over its allowance stops the load.

use std::cell::RefCell;

use lopdf::{DecompressError, Dictionary, Document, LoadOptions, Object, ObjectId, Stream};

use crate::content::{Interp, Matrix, PageDrawing};
use crate::{
    MAX_PDF_BYTES, MAX_PDF_DECOMPRESSED, MAX_PDF_DEPTH, MAX_PDF_EXPANSION, MAX_PDF_FILTERS, MAX_PDF_NUMBER, MAX_PDF_PAGE_NODES, MAX_PDF_STREAM_BYTES,
    MAX_PDF_STREAMS, MIN_PDF_STREAM_ALLOWANCE, MIN_PDF_STRUCTURE_STREAM_BYTES, PdfError, looks_like_pdf,
};

/// What the file may still decode to.
pub(crate) struct Budget {
    left: usize,
}

impl Budget {
    /// `stream` decoded, within its allowance and what is left of the file's total.
    pub(crate) fn decode(&mut self, stream: &Stream) -> Result<Vec<u8>, PdfError> {
        let data = decode_bounded(stream, allowance(stream.content.len(), self.left))?;
        self.left = self.left.saturating_sub(data.len());
        Ok(data)
    }
}

fn allowance(stored: usize, left: usize) -> usize {
    stored.saturating_mul(MAX_PDF_EXPANSION).max(MIN_PDF_STREAM_ALLOWANCE).min(left)
}

fn decode_bounded(stream: &Stream, limit: usize) -> Result<Vec<u8>, PdfError> {
    if stream.filters().is_ok_and(|f| f.len() > MAX_PDF_FILTERS) {
        return Err(PdfError::Read(format!("a stream has more than {MAX_PDF_FILTERS} filters")));
    }
    stream.get_plain_content_with_limit(limit).map_err(lopdf_error)
}

fn lopdf_error(e: lopdf::Error) -> PdfError {
    match e {
        lopdf::Error::Decompress(DecompressError::MemoryLimitExceeded { .. }) => PdfError::StreamTooLarge,
        other => PdfError::Read(other.to_string()),
    }
}

// ───────────────────────────── Loading ─────────────────────────────

/// The state [`admit`] works with during one load. The parser takes a plain function, so the
/// state lives in a thread-local slot that [`LoadGuard`] fills and empties. The parser is built
/// without its parallel feature, so it calls `admit` on the loading thread; were that to change,
/// `admit` would find no state and refuse every object stream, which fails safe.
struct LoadState {
    left: usize,
    per_stream: usize,
    refused: Option<PdfError>,
}

thread_local! {
    static LOADING: RefCell<Option<LoadState>> = const { RefCell::new(None) };
}

struct LoadGuard;

impl LoadGuard {
    fn install(per_stream: usize) -> LoadGuard {
        LOADING.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut() {
                *slot = Some(LoadState { left: MAX_PDF_DECOMPRESSED, per_stream, refused: None });
            }
        });
        LoadGuard
    }

    fn take(&self) -> Option<LoadState> {
        LOADING.with(|slot| slot.try_borrow_mut().ok().and_then(|mut s| s.take()))
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        let _ = self.take();
    }
}

/// Called by the parser for each object of the file, before it decodes an object stream. Object
/// streams are decoded here within their allowance and handed back plain; a refusal is recorded
/// and the object discarded. Objects held in object streams pass through this function too, and
/// the parser keeps the value returned for them (they are never streams).
fn admit(id: ObjectId, object: &mut Object) -> Option<(ObjectId, Object)> {
    let Object::Stream(stream) = object else {
        return Some((id, object.clone()));
    };
    if !stream.dict.has_type(b"ObjStm") || !stream.is_compressed() {
        return Some((id, Object::Null));
    }
    let admitted = LOADING.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else { return false };
        let Some(state) = slot.as_mut() else { return false };
        if state.refused.is_some() {
            return false;
        }
        let limit = allowance(stream.content.len(), state.left).min(state.per_stream);
        match decode_bounded(stream, limit) {
            Ok(data) => {
                state.left = state.left.saturating_sub(data.len());
                stream.set_plain_content(data);
                true
            }
            Err(e) => {
                state.refused = Some(e);
                false
            }
        }
    });
    // The parser keeps its own copy of a top-level object; the value returned is not used.
    admitted.then_some((id, Object::Null))
}

/// How many streams the file holds, counted by their `stream` keywords (a keyword cannot be
/// escaped, unlike a name); `endstream` is not counted.
fn count_streams(data: &[u8]) -> usize {
    let mut count = 0usize;
    let mut i = 0usize;
    while let Some(window) = data.get(i..i.saturating_add(6)) {
        if window == b"stream" {
            let after_end = i >= 3 && data.get(i - 3..i) == Some(b"end".as_slice());
            if !after_end {
                count = count.saturating_add(1);
            }
            i = i.saturating_add(6);
        } else {
            i = i.saturating_add(1);
        }
    }
    count
}

/// The parsed file and what it may still decode to.
fn load(data: &[u8]) -> Result<(Document, Budget), PdfError> {
    if data.len() > MAX_PDF_BYTES {
        return Err(PdfError::TooLarge { size: data.len() });
    }
    if !looks_like_pdf(data) {
        return Err(PdfError::NotPdf);
    }
    let streams = count_streams(data);
    if streams > MAX_PDF_STREAMS {
        return Err(PdfError::TooManyStreams);
    }
    let per_stream = (MAX_PDF_DECOMPRESSED / streams.max(1)).clamp(MIN_PDF_STRUCTURE_STREAM_BYTES, MAX_PDF_STREAM_BYTES);
    let guard = LoadGuard::install(per_stream);
    let options = LoadOptions { filter: Some(admit), max_decompressed_size: Some(per_stream), ..LoadOptions::default() };
    let loaded = Document::load_mem_with_options(data, options);
    let state = guard.take();
    if let Some(refused) = state.as_ref().and_then(|s| s.refused.clone()) {
        return Err(refused);
    }
    let doc = loaded.map_err(lopdf_error)?;
    // The parser removes `/Encrypt` once the empty password opens the file.
    if doc.trailer.get(b"Encrypt").is_ok() {
        return Err(PdfError::Encrypted);
    }
    let left = state.map_or(MAX_PDF_DECOMPRESSED, |s| s.left);
    Ok((doc, Budget { left }))
}

// ───────────────────────────── The first page ─────────────────────────────

/// The object `obj` refers to, or `obj` itself.
pub(crate) fn deref<'a>(doc: &'a Document, obj: &'a Object) -> Result<&'a Object, PdfError> {
    doc.dereference(obj).map(|(_, o)| o).map_err(lopdf_error)
}

pub(crate) fn dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match deref(doc, obj).ok()? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

pub(crate) fn number(doc: &Document, obj: &Object) -> Option<f64> {
    let v = match deref(doc, obj).ok()? {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => f64::from(*r),
        _ => return None,
    };
    (v.is_finite() && v.abs() <= MAX_PDF_NUMBER).then_some(v)
}

/// A rectangle `[x1 y1 x2 y2]` as `[llx, lly, urx, ury]`, with a positive width and height.
fn rect(doc: &Document, obj: &Object) -> Option<[f64; 4]> {
    let Object::Array(items) = deref(doc, obj).ok()? else { return None };
    let [a, b, c, d] = items.as_slice() else { return None };
    let (x1, y1, x2, y2) = (number(doc, a)?, number(doc, b)?, number(doc, c)?, number(doc, d)?);
    let r = [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// What a page takes from its ancestors in the page tree when it does not set it itself.
#[derive(Clone, Default)]
struct Inherited<'a> {
    resources: Option<&'a Dictionary>,
    media_box: Option<[f64; 4]>,
    crop_box: Option<[f64; 4]>,
    rotate: i64,
}

impl<'a> Inherited<'a> {
    fn update(&mut self, doc: &'a Document, node: &'a Dictionary) {
        if let Some(res) = node.get(b"Resources").ok().and_then(|r| dict(doc, r)) {
            self.resources = Some(res);
        }
        if let Some(b) = node.get(b"MediaBox").ok().and_then(|o| rect(doc, o)) {
            self.media_box = Some(b);
        }
        if let Some(b) = node.get(b"CropBox").ok().and_then(|o| rect(doc, o)) {
            self.crop_box = Some(b);
        }
        if let Some(r) = node.get(b"Rotate").ok().and_then(|o| deref(doc, o).ok()).and_then(|o| o.as_i64().ok()) {
            self.rotate = r;
        }
    }
}

struct PageSearch {
    visited: usize,
    /// Pages to pass before the one wanted.
    skip: usize,
}

impl PageSearch {
    /// The wanted page when it is under `node`, with what it inherits, in document order.
    fn page<'a>(
        &mut self,
        doc: &'a Document,
        node: &'a Dictionary,
        mut inherited: Inherited<'a>,
        depth: usize,
    ) -> Result<Option<(&'a Dictionary, Inherited<'a>)>, PdfError> {
        if depth > MAX_PDF_DEPTH {
            return Err(PdfError::TooDeep);
        }
        self.visited = self.visited.saturating_add(1);
        if self.visited > MAX_PDF_PAGE_NODES {
            return Err(PdfError::TooMuchWork);
        }
        inherited.update(doc, node);
        let kids = node.get(b"Kids").ok().and_then(|k| deref(doc, k).ok()).and_then(|k| k.as_array().ok());
        let is_page = node.has_type(b"Page") || (kids.is_none() && node.get(b"Contents").is_ok());
        if is_page {
            if self.skip == 0 {
                return Ok(Some((node, inherited)));
            }
            self.skip -= 1;
            return Ok(None);
        }
        for kid in kids.into_iter().flatten() {
            if let Some(kid) = dict(doc, kid)
                && let Some(found) = self.page(doc, kid, inherited.clone(), depth + 1)?
            {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}

/// The page's rotation as a matrix from page space to the page as shown, and the frame in the
/// shown space.
fn rotation(page_box: [f64; 4], rotate: i64) -> (Matrix, [f64; 4]) {
    let [llx, lly, urx, ury] = page_box;
    let (w, h) = (urx - llx, ury - lly);
    match rotate.rem_euclid(360) {
        90 => ([0.0, -1.0, 1.0, 0.0, -lly, urx], [0.0, 0.0, h, w]),
        180 => ([-1.0, 0.0, 0.0, -1.0, urx, ury], [0.0, 0.0, w, h]),
        270 => ([0.0, 1.0, -1.0, 0.0, ury, -llx], [0.0, 0.0, h, w]),
        _ => (crate::content::IDENTITY, page_box),
    }
}

/// The painted paths of the first page, in the page's shown space (its rotation applied).
pub(crate) fn first_page_drawing(data: &[u8]) -> Result<PageDrawing, PdfError> {
    page_drawing(data, 0)
}

/// The painted paths of page `index` (from 0), in the page's shown space (its rotation applied).
pub(crate) fn page_drawing(data: &[u8], index: usize) -> Result<PageDrawing, PdfError> {
    let (doc, budget) = load(data)?;
    let catalog = doc.trailer.get(b"Root").ok().and_then(|r| dict(&doc, r)).ok_or(PdfError::NoPage)?;
    let root = catalog.get(b"Pages").ok().and_then(|p| dict(&doc, p)).ok_or(PdfError::NoPage)?;
    let mut search = PageSearch { visited: 0, skip: index };
    let (page, inherited) = search.page(&doc, root, Inherited::default(), 0)?.ok_or(PdfError::NoPage)?;
    let page_box = inherited.crop_box.or(inherited.media_box);
    let (ctm, frame) = match page_box {
        Some(b) => {
            let (m, f) = rotation(b, inherited.rotate);
            (m, Some(f))
        }
        None => (crate::content::IDENTITY, None),
    };

    let mut interp = Interp::new(&doc, budget, ctm);
    let contents = match page.get(b"Contents").ok().map(|c| deref(&doc, c)).transpose()? {
        Some(Object::Array(parts)) => parts.iter().collect(),
        Some(single) => vec![single],
        None => Vec::new(),
    };
    // The parts of an array of content streams form one stream, split anywhere between tokens.
    let mut bytes = Vec::new();
    for part in contents {
        if let Object::Stream(s) = deref(&doc, part)? {
            let decoded = interp.decode(s)?;
            bytes.extend_from_slice(&decoded);
            bytes.push(b'\n');
        }
    }
    interp.run_page(&bytes, inherited.resources)?;
    let (painted, dropped) = interp.finish();
    Ok(PageDrawing { painted, frame, page_box, dropped })
}
