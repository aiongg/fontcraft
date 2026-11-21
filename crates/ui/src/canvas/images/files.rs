//! The decoded files of placed images, shared by the canvas and Font View cells.
//!
//! - Bytes come from the session's asset store (`Session::assets`), never from a file opened here, and are decoded by
//!   `fontcraft_imagery` within its limits ([`imagery::MAX_FILE_BYTES`], [`imagery::MAX_SIDE`],
//!   [`imagery::MAX_PIXELS`]).
//! - A file's header is read first ([`HEADER_BYTES`]): it gives the format and the pixel size, so a file that is too
//!   large, a format kept as a reference only (PSD, SVG, PDF) and a file that cannot be read are known without
//!   decoding anything.
//! - A file of at most [`SYNC_PIXELS`] pixels is decoded when it is first asked for. A larger one is decoded on the
//!   host's jobs (a thread on the desktop, at once on the web and in tests with inline jobs), at most [`MAX_JOBS`] at
//!   a time; until it lands it is [`Loaded::Pending`] and the window is woken when it does.
//! - Decoded pixels are kept shrunk to at most [`MAX_KEPT_SIDE`] on the longer side, by a whole factor. The cache
//!   holds at most [`MAX_FILES`] files and [`MAX_FILE_BYTES`] bytes of pixels; past either it empties.
//! - A file is read again when the window gets the focus back ([`ImageFiles::reread`]), so a changed or restored file
//!   shows; switching documents empties the cache, as the same stored path may name another file.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use craft::host::Host;
use fontcraft_engine::color::AssetStore;
use fontcraft_engine::imagery::{self, ImageFormat, Rgba8};

/// Bytes read from the start of a file for its format and pixel size.
pub const HEADER_BYTES: usize = 1 << 20;
/// Largest image, in pixels, decoded when it is first asked for; larger ones are decoded on a job.
pub const SYNC_PIXELS: u64 = 1 << 22;
/// Most decoding jobs running at once.
pub const MAX_JOBS: usize = 4;
/// Longest side of the pixels kept for drawing.
pub const MAX_KEPT_SIDE: u32 = 4096;
/// Most files kept; past it the cache empties.
pub const MAX_FILES: usize = 1024;
/// Most bytes of decoded pixels kept; past it the cache empties.
pub const MAX_FILE_BYTES: usize = 512 << 20;

/// Decoded pixels shared between the cache and what the canvas draws. Two are equal when they are the same buffer, so
/// comparing view data never compares pixels.
#[derive(Clone)]
pub struct Pixels(pub Arc<Rgba8>);

impl PartialEq for Pixels {
    fn eq(&self, other: &Pixels) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({}×{})", self.0.width, self.0.height)
    }
}

/// What the cache knows of a file.
#[derive(Clone, Debug, PartialEq)]
pub enum Loaded {
    /// Its pixels (possibly shrunk) and its size in pixels as stored.
    Ready { pixels: Pixels, size: [u32; 2] },
    /// Being decoded on a job; the size comes from its header.
    Pending { size: [u32; 2] },
    /// A format kept as a reference but not drawn, with its pixel size when the header gives one.
    Unsupported { format: ImageFormat, size: Option<[u32; 2]> },
    /// The file cannot be read or decoded; the message says why.
    Failed(String),
}

impl Loaded {
    /// The file's size in pixels, when known.
    pub fn size(&self) -> Option<[u32; 2]> {
        match self {
            Loaded::Ready { size, .. } | Loaded::Pending { size } => Some(*size),
            Loaded::Unsupported { size, .. } => *size,
            Loaded::Failed(_) => None,
        }
    }
}

/// A job's answer: the path, the read revision it was started at, and what it found.
type Landing = Arc<Mutex<Vec<(String, u64, Loaded)>>>;

#[derive(Default)]
pub struct ImageFiles {
    doc: Option<u64>,
    /// Raised by [`ImageFiles::reread`]; an entry of an older revision is read again when next asked for.
    revision: u64,
    entries: HashMap<String, (u64, Loaded)>,
    bytes: usize,
    running: HashSet<String>,
    landing: Landing,
    /// Moves whenever an entry is made.
    generation: u64,
}

impl ImageFiles {
    /// Empties the cache when `doc` is another document than the one it was filled for.
    pub fn follow(&mut self, doc: Option<u64>) {
        if self.doc != doc {
            self.doc = doc;
            self.entries.clear();
            self.bytes = 0;
        }
    }

    /// Marks every file to be read again when next asked for: the files on disk may have changed.
    pub fn reread(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Takes in what the jobs decoded. Returns whether anything landed.
    pub fn collect(&mut self) -> bool {
        let landed = std::mem::take(&mut *self.landing.lock().unwrap_or_else(PoisonError::into_inner));
        let any = !landed.is_empty();
        for (path, revision, loaded) in landed {
            self.running.remove(&path);
            self.keep(path, revision, loaded);
        }
        any
    }

    /// Whether a decoding job is running.
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }

    fn keep(&mut self, path: String, revision: u64, loaded: Loaded) {
        let size = match &loaded {
            Loaded::Ready { pixels, .. } => pixels.0.pixels.len(),
            _ => 0,
        };
        if let Some((_, old)) = self.entries.remove(&path)
            && let Loaded::Ready { pixels, .. } = old
        {
            self.bytes = self.bytes.saturating_sub(pixels.0.pixels.len());
        }
        if self.entries.len() >= MAX_FILES || self.bytes.saturating_add(size) > MAX_FILE_BYTES {
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(size);
        self.entries.insert(path, (revision, loaded));
        self.generation = self.generation.wrapping_add(1);
    }

    /// Moves whenever the cache takes in a file, so what was drawn from it can be worked out again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// What the cache holds for `path` when it needs no reading: an entry of the current read revision, or one being
    /// decoded.
    pub fn fresh(&self, path: &str) -> Option<&Loaded> {
        match self.entries.get(path) {
            Some((r, loaded)) if *r == self.revision || self.running.contains(path) => Some(loaded),
            _ => None,
        }
    }

    /// What is known of the file at `path` without reading it.
    pub fn peek(&self, path: &str) -> Option<&Loaded> {
        self.entries.get(path).map(|(_, l)| l)
    }

    /// The file at `path`: read and decoded on first use (or after [`ImageFiles::reread`]), a large one on a job of
    /// `host` that wakes `wake` when it lands. Without a host every file is decoded at once.
    pub fn get(&mut self, assets: &Arc<dyn AssetStore>, path: &str, host: Option<&Host>, wake: Option<&egui::Context>) -> Loaded {
        let revision = self.revision;
        match self.entries.get(path) {
            Some((r, loaded)) if *r == revision => return loaded.clone(),
            Some((_, loaded)) if self.running.contains(path) => return loaded.clone(),
            _ => {}
        }
        let header = match head(assets.as_ref(), path) {
            Ok(h) => h,
            Err(loaded) => {
                self.keep(path.to_string(), revision, loaded.clone());
                return loaded;
            }
        };
        let pixels = u64::from(header.width) * u64::from(header.height);
        let size = [header.width, header.height];
        if pixels <= SYNC_PIXELS || host.is_none() {
            let loaded = decode(assets.as_ref(), path);
            self.keep(path.to_string(), revision, loaded.clone());
            return loaded;
        }
        if self.running.len() >= MAX_JOBS {
            // Asked again on a later pass, once a running job has landed.
            return self.entries.get(path).map_or(Loaded::Pending { size }, |(_, l)| l.clone());
        }
        let Some(host) = host else { return Loaded::Pending { size } };
        self.running.insert(path.to_string());
        let (store, file, out, wake) = (Arc::clone(assets), path.to_string(), Arc::clone(&self.landing), wake.cloned());
        let job = move || {
            let loaded = fontcraft_engine::guard::catch_panic(|| decode(store.as_ref(), &file))
                .unwrap_or_else(|e| Loaded::Failed(format!("the image could not be decoded ({e})")));
            out.lock().unwrap_or_else(PoisonError::into_inner).push((file, revision, loaded));
            if let Some(ctx) = wake {
                ctx.request_repaint();
            }
        };
        host.jobs().spawn("placed image", Box::new(job));
        // Inline jobs (the web, tests) have landed already.
        self.collect();
        match self.entries.get(path) {
            Some((r, loaded)) if *r == revision => loaded.clone(),
            Some((_, loaded)) => loaded.clone(),
            None => Loaded::Pending { size },
        }
    }
}

/// The header of the file at `path`, or what stands in for a file that cannot be drawn.
fn head(assets: &dyn AssetStore, path: &str) -> Result<imagery::ImageInfo, Loaded> {
    let bytes = assets.read_head(path, HEADER_BYTES).map_err(|e| Loaded::Failed(e.to_string()))?;
    let info = imagery::sniff(&bytes).map_err(|e| Loaded::Failed(e.to_string()))?;
    let size = (info.width > 0 && info.height > 0).then_some([info.width, info.height]);
    if !info.format.decodable() {
        return Err(Loaded::Unsupported { format: info.format, size });
    }
    let Some([w, h]) = size else { return Err(Loaded::Failed("the image has no pixels".into())) };
    if w > imagery::MAX_SIDE || h > imagery::MAX_SIDE || u64::from(w) * u64::from(h) > imagery::MAX_PIXELS {
        return Err(Loaded::Failed(format!("the image is {w} × {h} pixels; at most {} pixels are drawn", imagery::MAX_PIXELS)));
    }
    Ok(info)
}

/// Reads and decodes the file at `path`.
pub fn decode(assets: &dyn AssetStore, path: &str) -> Loaded {
    let bytes = match assets.read(path) {
        Ok(b) => b,
        Err(e) => return Loaded::Failed(e.to_string()),
    };
    if bytes.len() > imagery::MAX_FILE_BYTES {
        return Loaded::Failed(format!("the file is larger than {} bytes", imagery::MAX_FILE_BYTES));
    }
    let info = match imagery::sniff(&bytes) {
        Ok(info) => info,
        Err(e) => return Loaded::Failed(e.to_string()),
    };
    if !info.format.decodable() {
        let size = (info.width > 0 && info.height > 0).then_some([info.width, info.height]);
        return Loaded::Unsupported { format: info.format, size };
    }
    match imagery::decode_rgba(&bytes, imagery::MAX_PIXELS) {
        Ok(image) if whole(&image) => {
            let size = [image.width, image.height];
            Loaded::Ready { pixels: Pixels(Arc::new(shrink(image, MAX_KEPT_SIDE))), size }
        }
        Ok(_) => Loaded::Failed("the decoded image has the wrong number of pixels".into()),
        Err(e) => Loaded::Failed(e.to_string()),
    }
}

/// Whether `image` has pixels and exactly `width × height` of them.
fn whole(image: &Rgba8) -> bool {
    let expected = u64::from(image.width) * u64::from(image.height) * 4;
    image.width > 0 && image.height > 0 && u64::try_from(image.pixels.len()).ok() == Some(expected)
}

/// `image` shrunk by a whole factor so that neither side exceeds `max_side`, averaging each block of pixels.
pub fn shrink(image: Rgba8, max_side: u32) -> Rgba8 {
    let (w, h) = (image.width, image.height);
    let longest = w.max(h);
    if max_side == 0 || longest <= max_side || !whole(&image) {
        return image;
    }
    let k = longest.div_ceil(max_side) as usize;
    let (w, h) = (w as usize, h as usize);
    let (nw, nh) = (w.div_ceil(k), h.div_ceil(k));
    let mut pixels = vec![0u8; nw * nh * 4];
    for (i, out) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (ox, oy) = (i % nw, i / nw);
        let mut sum = [0u32; 4];
        let mut n = 0u32;
        for y in oy * k..((oy + 1) * k).min(h) {
            for x in ox * k..((ox + 1) * k).min(w) {
                let at = (y * w + x) * 4;
                if let Some(p) = image.pixels.get(at..at + 4) {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u32::from(*v);
                    }
                    n += 1;
                }
            }
        }
        for (o, s) in out.iter_mut().zip(sum) {
            if let Some(mean) = s.checked_div(n) {
                *o = u8::try_from(mean).unwrap_or(u8::MAX);
            }
        }
    }
    Rgba8 { width: u32::try_from(nw).unwrap_or(u32::MAX), height: u32::try_from(nh).unwrap_or(u32::MAX), pixels }
}
